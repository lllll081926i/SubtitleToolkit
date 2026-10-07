//! 14 §ASS→BDN：渲染 → 变化检测 → tight 裁剪 → PNG → BDN XML → 目录校验。
//!
//! 三条与"交付一份能用的 BDN"直接相关的决定：
//!
//! 1. **变化检测不重写**。BDN 与 SUP 是同一条前半链（14 §ASS→SUP 写的是"同 BDN 前半"），
//!    所以事件切分复用 `pgs_author::plan`。两边各写一份"什么时候算换帧"，
//!    迟早会出现同一输入两种时间轴。
//! 2. **XML 声明的尺寸就是 PNG 的真实尺寸**（AT-BDN-001 / 13 §7）。这正是旧工程
//!    按 `max(min_axis, w)` 钳制尺寸犯下的错，`codec-bdn` 已经把 `min_axis` 定成
//!    纯过滤阈值；这里额外把"写完再读一遍目录"当门：声明与实际不符就不算交付。
//! 3. **PNG 先写、XML 后写**。XML 引用 PNG 文件名；顺序反了的话，中途失败会留下
//!    一份引用不存在文件的 XML——那是比"没有输出"更糟的状态（AT-OUT-001 的精神）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codec_bdn::{
    build_document_on_canvas, png_size, validate_dir, write as write_bdn_xml, BdnFrame, BdnOptions,
};
use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use core_model::time::Fps;
use image_core::Level;

use crate::args::{Command, Invocation};
use crate::output::{Envelope, WriteRecord};
use crate::pipeline;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

/// 事件文件名 → PNG 字节。
type PngMap = BTreeMap<String, Vec<u8>>;

/// 一条 `render` 命令的取值：派发处从 `Command::Render` 里解出来整体交进来，
/// 省掉八个位置参数，也就少了"某个参数没传到底"的空间。
pub struct RenderArgs<'a> {
    pub input: &'a Path,
    pub fps: Fps,
    pub canvas: Canvas,
    pub dll: Option<&'a Path>,
    pub default_font: &'a str,
    pub min_axis: u32,
}

/// 一次 render 的中间产物：XML 字节 + 每个**存活事件**的 PNG 字节。
/// PNG 与 XML 一起算出来，才谈得上"声明尺寸 == 实际尺寸"。
struct Built {
    /// 17 §7：这次真的交给 libass 的内存字体（`--inject-fonts` 关掉时是空的）
    injected: Vec<render_libass::InjectedFont>,
    xml: Vec<u8>,
    pngs: PngMap,
    events: usize,
    /// 过滤前的渲染帧数：events==0 时要能说出"清掉了多少"。
    rendered: usize,
    samples: usize,
    /// `codec-bdn` 构造文档时的记账（含 min_axis 丢弃）；必须并进信封的 loss。
    loss: LossReport,
}

/// 帧列表 →（XML 字节、按文档顺序连续编号的 PNG）。
///
/// `min_axis` 的过滤规则只有 `codec-bdn` 一份。第一遍带着用户阈值构造，拿到"哪些文件
/// 活了下来"；如果确实丢了东西，就把存活帧重新连续编号后再走一遍同一个构造器（阈值置 0，
/// 因为筛选已经发生）。这样盘上不会出现 XML 没引用的孤儿 PNG，文件名也不会有空洞，
/// 而"什么算小对象"仍然只有一个定义。
fn assemble(
    frames: Vec<BdnFrame>,
    mut pngs: PngMap,
    opt: &BdnOptions,
    canvas: Canvas,
) -> Result<(Vec<u8>, PngMap, usize, LossReport)> {
    let (doc, loss) = build_document_on_canvas(&frames, opt, canvas)?;
    let kept: Vec<String> = doc
        .events
        .iter()
        .map(|e| e.graphic.file_name.clone())
        .collect();
    if kept.len() == frames.len() {
        let xml = write_bdn_xml(&doc)?.into_bytes();
        return Ok((xml, pngs, kept.len(), loss));
    }

    let mut renamed: Vec<BdnFrame> = Vec::with_capacity(kept.len());
    let mut pruned: PngMap = BTreeMap::new();
    for (i, name) in kept.iter().enumerate() {
        let src = frames
            .iter()
            .find(|f| &f.png_file_name == name)
            .ok_or_else(|| cli_err(174, "BDN 文档引用了一个构造时不存在的帧"))?;
        let bytes = pngs
            .remove(name)
            .ok_or_else(|| cli_err(175, "BDN 文档引用的 PNG 不在内存产物里"))?;
        let mut frame = src.clone();
        frame.png_file_name = format!("{:04}.png", i);
        pruned.insert(frame.png_file_name.clone(), bytes);
        renamed.push(frame);
    }
    let (doc2, _) = build_document_on_canvas(
        &renamed,
        &BdnOptions {
            min_axis: 0,
            ..opt.clone()
        },
        canvas,
    )?;
    if doc2.events.len() != kept.len() {
        return Err(
            cli_err(177, "重新编号后的第二遍构造与第一遍存活事件数不一致").with_detail(format!(
                "first={} second={}",
                kept.len(),
                doc2.events.len()
            )),
        );
    }
    let xml = write_bdn_xml(&doc2)?.into_bytes();
    Ok((xml, pruned, doc2.events.len(), loss))
}

fn build(
    args: &RenderArgs,
    shared: Option<&crate::front::SharedRender<'_>>,
    inject: Option<&[crate::front::InjectFace]>,
) -> Result<Built> {
    let RenderArgs {
        input,
        fps,
        canvas,
        dll,
        default_font,
        min_axis,
    } = *args;
    // 前半链（采样 → 变化检测 → tight 裁剪）与 ASS→SUP 共用同一份实现：
    // 14 §ASS→SUP 写的是"同 BDN 前半"，反过来 BDN 也不该另写一套"什么时候算换帧"。
    let job = crate::front::RenderJob {
        input,
        fps,
        canvas,
        dll,
        default_font,
        inject,
    };
    let rendered = crate::front::resolve(shared, &job)?;
    let plan = &rendered.plan;
    let injected = rendered.injected.clone();

    let mut frames = Vec::new();
    let mut pngs: PngMap = BTreeMap::new();
    for ev in plan.events.iter().filter(|e| e.patch.is_some()) {
        let patch = ev.patch.as_ref().expect("刚筛过 Some");
        let name = format!("{:04}.png", pngs.len());
        let bytes = patch.to_png(Level::Balanced)?;
        // 自己先验一次：PNG 的 IHDR 必须就是位图块尺寸
        let got = png_size(&bytes)?;
        if (got.width, got.height) != (patch.width, patch.height) {
            return Err(
                cli_err(170, "PNG 实际尺寸与位图块尺寸不一致，拒绝写 BDN").with_detail(format!(
                    "{name}: png {}x{} vs patch {}x{}",
                    got.width, got.height, patch.width, patch.height
                )),
            );
        }
        frames.push(BdnFrame {
            start: ev.start,
            end: ev.end,
            png_file_name: name.clone(),
            png_width: patch.width,
            png_height: patch.height,
            x: patch.x as i32,
            y: patch.y as i32,
        });
        pngs.insert(name, bytes);
    }
    let opt = BdnOptions {
        fps,
        language: "und".to_string(),
        name: input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("subtitles")
            .to_string(),
        min_axis,
        offset_x: 0,
        offset_y: 0,
    };
    let kept_frames = frames.len();
    let (xml, pngs, events, loss) = assemble(frames, pngs, &opt, canvas)?;
    Ok(Built {
        injected,
        xml,
        events,
        rendered: kept_frames,
        samples: rendered.samples,
        pngs,
        loss,
    })
}

pub(crate) fn run_render(
    inv: &Invocation,
    args: &RenderArgs,
    cmd: &str,
    shared: Option<&crate::front::SharedRender<'_>>,
) -> Result<Envelope> {
    let RenderArgs {
        input,
        fps,
        canvas,
        min_axis,
        ..
    } = *args;
    // 字体环境与注入计划都在渲染之前算好（17 §7）：
    // 身份要进报告与缓存 key，注入要在 libass 读轨道之前把字节交出去。
    // 顺序不是风格问题 —— 渲染已经发生之后再算身份，那份身份只是事后追认。
    let text = pipeline::load_text(input)?.text;
    let mut scanner = crate::fontenv::Scanner::with_cache(crate::fontenv::cache_mode(inv));
    let (font_env, env_ms) = scanner.env_for(&text)?;
    let plan = if inv.inject_fonts {
        Some(scanner.inject_plan(&text)?)
    } else {
        None
    };
    let built = build(args, shared, plan.as_deref())?;
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    // 默认输出：<stem>_BDN/<stem>.xml，PNG 与它同目录；--out 直接给 XML 路径
    let xml_default = {
        let mut d = input
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{stem}_BDN"));
        d.push(format!("{stem}.xml"));
        d
    };
    let xml_path = inv.out.clone().unwrap_or(xml_default);
    let dir = xml_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut loss = LossReport::new();
    loss.preserved("14 §ASS→BDN：事件切分与 SUP 共用同一份变化检测（pgs_author::plan）");
    loss.preserved("BDN 声明尺寸原样取 PNG 的 IHDR，min_axis 只过滤不钳制（13 §7）");
    // codec-bdn 的记账并进信封：min_axis 丢了对象、时间码取整，都在这儿如实报告
    loss.merge(&built.loss);
    if min_axis == 0 {
        loss.user_attention("min_axis=0：没有小对象过滤，异常小的帧会原样进 BDN（13 §7）");
    }
    if built.events == 0 {
        // 空 BDN 是一份"看起来写成功了"的废文件，必须说清楚是哪一步把事件清光的。
        loss.user_attention(&format!(
            "过滤后没有任何事件，BDN 是空的（min_axis={min_axis}，输入有 {} 个渲染帧）",
            built.rendered
        ));
    }

    let mut writes: Vec<WriteRecord> = Vec::new();
    // PNG 先落盘（dry-run 时只报告不落盘），XML 后落盘
    for (name, bytes) in &built.pngs {
        let target = dir.join(name);
        let verify_bytes = bytes.clone();
        let emitted = pipeline::emit(
            None,
            &target,
            bytes,
            move |b| {
                if b != verify_bytes.as_slice() {
                    return Err(cli_err(171, "PNG 回读字节与内存内容不一致"));
                }
                png_size(b)?;
                Ok(())
            },
            inv.dry_run,
            inv.overwrite,
        )?;
        let note = emitted.describe();
        writes.push(WriteRecord {
            path: note.path,
            bytes: note.bytes,
            state: note.state,
        });
    }
    let xml = built.xml.clone();
    let png_count = built.pngs.len();
    let verify_xml = {
        let dir = dir.clone();
        let dry = inv.dry_run;
        move |b: &[u8]| -> Result<()> {
            let doc = codec_bdn::parse_bytes(b)?;
            if doc.events.len() != png_count {
                return Err(cli_err(172, "BDN XML 事件数与 PNG 数不一致")
                    .with_detail(format!("xml={} pngs={png_count}", doc.events.len())));
            }
            if !dry {
                // 真正的 AT-BDN-001：读盘上的 PNG，声明尺寸必须与实际一致
                let report = validate_dir(&doc, &dir)?;
                if !report.is_clean() {
                    return Err(cli_err(173, "BDN 目录校验未通过").with_detail(report.summary()));
                }
            }
            Ok(())
        }
    };
    let emitted = pipeline::emit(
        inv.out.as_deref(),
        &xml_path,
        &xml,
        verify_xml,
        inv.dry_run,
        inv.overwrite,
    )?;
    let note = emitted.describe();
    writes.push(WriteRecord {
        path: note.path,
        bytes: note.bytes,
        state: note.state,
    });

    let data = serde_json::json!({
        "font_env": crate::fontenv::env_json(&font_env, env_ms),
        "font_injection": crate::fontenv::injection_json(&built.injected, plan.as_deref().unwrap_or(&[]), inv.inject_fonts),
        "from": "ass",
        "to": "bdn",
        "canvas": canvas.label(),
        "fps": fps.to_label(),
        "events": built.events,
        "rendered_frames": built.rendered,
        "pngs": png_count,
        "samples": built.samples,
        "min_axis": min_axis,
        "xml": xml_path.display().to_string(),
    });
    let mut env = Envelope::ok(cmd, data).with_loss(&loss);
    env.writes = writes;
    if let Some(map) = env.data.as_mut().and_then(|v| v.as_object_mut()) {
        map.insert(
            "lossless".to_string(),
            serde_json::json!(loss.is_lossless()),
        );
    }
    Ok(env)
}

/// batch 的 `bdn` profile 用：与单命令 `render` 造出同一个 Command，
/// 这样"批量重放"与"单发重放"是同一份语义（20 §Repro）。
pub fn command_for(
    input: &Path,
    fps: Fps,
    canvas: Canvas,
    dll: Option<&str>,
    font: &str,
    min_axis: u32,
) -> Command {
    Command::Render {
        input: input.to_path_buf(),
        fps,
        canvas,
        dll: dll.map(PathBuf::from),
        default_font: font.to_string(),
        min_axis,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::time::Time;

    fn frame(name: &str, w: u32, h: u32, start_ms: i64) -> BdnFrame {
        BdnFrame {
            start: Time::from_millis(start_ms).unwrap(),
            end: Time::from_millis(start_ms + 1000).unwrap(),
            png_file_name: name.to_string(),
            png_width: w,
            png_height: h,
            x: 0,
            y: 0,
        }
    }

    fn opts(min_axis: u32) -> BdnOptions {
        BdnOptions {
            fps: Fps::PAL_25,
            language: "und".to_string(),
            name: "t".to_string(),
            min_axis,
            offset_x: 0,
            offset_y: 0,
        }
    }

    fn pngs_of(frames: &[BdnFrame]) -> PngMap {
        frames
            .iter()
            .enumerate()
            .map(|(i, f)| (f.png_file_name.clone(), vec![i as u8; 8]))
            .collect()
    }

    #[test]
    fn min_axis_filtering_leaves_no_orphan_png_and_no_numbering_gaps() {
        let frames = vec![
            frame("0000.png", 1000, 60, 0),
            frame("0001.png", 40, 40, 1000),
            frame("0002.png", 900, 50, 2000),
        ];
        let before = pngs_of(&frames);
        let (xml, pngs, events, loss) =
            assemble(frames, before.clone(), &opts(200), Canvas::FHD1080).unwrap();
        let text = String::from_utf8(xml.clone()).unwrap();
        assert_eq!(events, 2, "{text}");
        // 盘上要正好是连续的 0000/0001，且 0001 携带的是原来 0002 的字节
        let keys: Vec<&str> = pngs.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["0000.png", "0001.png"]);
        assert_eq!(pngs["0001.png"], before["0002.png"]);
        assert_eq!(pngs["0000.png"], before["0000.png"]);
        assert!(text.contains("<FileName>0001.png</FileName>"), "{text}");
        assert!(
            !text.contains("0002.png"),
            "XML 还引用了一个不会被写出的文件"
        );
        assert!(
            loss.dropped.iter().any(|t| t.contains("min_axis")),
            "{loss:?}"
        );
    }

    #[test]
    fn without_filtering_the_first_pass_is_the_answer() {
        let frames = vec![
            frame("0000.png", 1000, 60, 0),
            frame("0001.png", 40, 40, 1000),
        ];
        let before = pngs_of(&frames);
        let (_, pngs, events, _) =
            assemble(frames, before.clone(), &opts(0), Canvas::FHD1080).unwrap();
        assert_eq!(events, 2);
        assert_eq!(pngs, before, "没过滤就不该动任何文件名");
    }
}
