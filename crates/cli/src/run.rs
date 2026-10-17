//! 命令执行（20 §Commands + 14 流水线）。每个命令都做三件事：
//! 变换 → 用对方的解析器回读校验 → 原子写出。校验失败就不动旧文件（AT-OUT-001）。

use std::path::{Path, PathBuf};

use codec_ass::AssDocument;
use codec_pgs::fastpath::{apply as apply_pgs, check_timeline, ods_invariant};
use codec_pgs::segment::{parse_stream, write_stream};
use codec_srt::{parse as parse_srt, write as write_srt};
use core_model::error::{Domain, Result, StkError};
use core_model::geometry::{LayoutResPolicy, ScriptResolutionPolicy};
use core_model::loss::LossReport;
use core_model::profile::FormatKind;
use core_model::time::Fps;
use core_model::Canvas;
use media_probe::{parse_video_streams, FfprobeIdentity, ProbeRunner, SystemProbe};
use transform_hdr::HdrApplied;
use transform_resolution::ResApplied;
use transform_time::TimeConfig;
use validator::BitrateArgs;

use crate::args::{Command, Invocation};
use crate::output::{Envelope, WriteRecord};
use crate::pipeline;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

pub fn execute(inv: &Invocation) -> Result<Envelope> {
    execute_ctx(inv, None)
}

/// `execute` 的批量版本：渲染型命令可以拿到一份"同一输入已经渲染过"的共享结果
/// （14 §ASS→SUP 写的"若同时输出 BDN+SUP，共享 render/cache"，ISSUES AC6）。
/// 传 `None` 就是单命令语义：前半链自己跑一次，别的行为完全一致。
pub(crate) fn execute_ctx(
    inv: &Invocation,
    shared: Option<&crate::front::SharedRender<'_>>,
) -> Result<Envelope> {
    let cmd = inv.command.name();
    // 37 §2 的磁盘缓存只在**渲染型命令**上有意义。这里统一构造一次，
    // 不放到渲染函数里：那个函数在批量路径下会被调很多次，每次
    // `DataDir::resolve_current_exe` 都会去 stat 一遍 exe 旁的 portable.flag。
    // 关掉它时显式给 None，而不是给一个「会返回 None 的对象」——后者在
    // 报告里说不清楚「没缓存」到底是被关了还是拿不到目录。
    let cache = if inv.render_cache {
        crate::rendercache::DiskCache::from_env()
    } else {
        None
    };
    let outcome = match &inv.command {
        Command::Probe { input } => probe(input, cmd),
        Command::Convert {
            input,
            to,
            canvas,
            script_policy,
            layout_policy,
        } => convert(
            inv,
            input,
            *to,
            *canvas,
            *script_policy,
            *layout_policy,
            cmd,
        ),
        Command::Time {
            input,
            mode,
            source_fps,
            target_fps,
            offset_ms,
            two_point,
            piecewise,
        } => {
            let cfg = TimeConfig {
                mode: *mode,
                source_fps: *source_fps,
                target_fps: *target_fps,
                offset_ms: *offset_ms,
            };
            cfg.validate_with(two_point.as_ref())?;
            time(inv, input, &cfg, two_point.as_ref(), piecewise, cmd)
        }
        Command::Hdr { input, settings } => hdr(inv, input, settings, cmd),
        Command::PgsAdjust { input, ops } => pgs_adjust(inv, input, ops, cmd),
        Command::PgsValidate {
            input,
            level,
            peak_bps,
        } => pgs_validate(input, *level, *peak_bps, cmd),
        Command::PgsEncode {
            input,
            fps,
            canvas,
            level,
            dll,
            default_font,
            exact_colors,
            snap_pts,
        } => pgs_encode(
            inv,
            input,
            *fps,
            *canvas,
            *level,
            dll.as_deref(),
            default_font,
            *exact_colors,
            *snap_pts,
            cmd,
            shared,
            &cache,
        ),
        Command::Doctor => doctor(cmd),
        Command::Cache { action } => cache_cmd(*action, cmd),
        Command::Settings { action, key, value } => {
            settings_cmd(*action, key.as_deref(), value.as_deref(), cmd)
        }
        Command::Diagnose {
            include_input,
            job_state,
        } => diagnose_cmd(inv, include_input, job_state.as_deref(), cmd),
        Command::Render {
            input,
            fps,
            canvas,
            dll,
            default_font,
            min_axis,
        } => crate::render::run_render(
            inv,
            &crate::render::RenderArgs {
                input,
                fps: *fps,
                canvas: *canvas,
                dll: dll.as_deref(),
                default_font,
                min_axis: *min_axis,
            },
            cmd,
            shared,
            &cache,
        ),
        Command::Batch { spec } => crate::batch::run_batch(inv, spec, cmd),
        Command::FontScan { .. } => crate::font::run_font(inv),
        Command::FontSubset { .. } => crate::subset::run_subset(inv),
        Command::Mkv { .. } => crate::mkv::run_mkv(inv),
        Command::NotBuilt { name, phase } => Err(StkError::simple(
            Domain::Input,
            60,
            &format!("`{name}` 还没有实现（{phase}）"),
        )
        .with_action("不要期待占位命令产生输出；已实现命令见 --help")),
    };
    match outcome {
        Ok(e) => {
            tracing::info!(command = cmd, ok = true, "执行完成");
            Ok(e)
        }
        Err(e) => {
            // 错误也要进日志：这是“事后诊断”最需要的那一条。
            // 带完整 code_string，与用户看到的报告是同一个标识（22 §4）。
            tracing::error!(
                command = cmd,
                code = %e.code_string(),
                title = %e.title,
                "执行失败"
            );
            Err(e)
        }
    }
}

/// 已加载的输入 + 探测结果。
struct Input {
    loaded: pipeline::Loaded,
    format: FormatKind,
    detection: serde_json::Value,
}

fn load_input(path: &Path) -> Result<Input> {
    let detection = format_detect::probe_file(path)?;
    let json = serde_json::to_value(&detection)
        .map_err(|e| cli_err(121, "探测结果序列化失败").with_detail(e.to_string()))?;
    let format = match detection.format {
        format_detect::SubtitleFormat::Ass | format_detect::SubtitleFormat::Ssa => FormatKind::Ass,
        format_detect::SubtitleFormat::Srt => FormatKind::Srt,
        format_detect::SubtitleFormat::Pgs => FormatKind::Pgs,
        format_detect::SubtitleFormat::BdnXml => FormatKind::Bdn,
        other => {
            return Err(cli_err(
                120,
                &format!("该输入不是本 CLI 当前能处理的字幕格式：{other:?}"),
            )
            .with_detail(detection.reasons.join("；")));
        }
    };
    let loaded = pipeline::load_text(path)?;
    Ok(Input {
        loaded,
        format,
        detection: json,
    })
}

/// 一条建议的可读序列化。候选值必须带着"它是什么、从哪来、多有把握、为什么"一起出门
/// （16 §4、35 §7），所以这里把四样都写成字段，而不是只写一个字符串。
fn suggestion_row(it: &media_probe::Suggestion) -> serde_json::Value {
    let (kind, value) = match &it.candidate {
        media_probe::Candidate::Fps(f) => ("fps", f.to_label()),
        media_probe::Candidate::Canvas(c) => ("canvas", c.label()),
        media_probe::Candidate::PgsCanvas(c) => ("pgs-canvas", c.label()),
        media_probe::Candidate::HdrPreset { reference_nits } => (
            "hdr-preset",
            format!("reference-white@{reference_nits}nits"),
        ),
    };
    serde_json::json!({
        "kind": kind,
        "value": value,
        "origin": format!("{:?}", it.origin),
        "confidence": format!("{:?}", it.confidence),
        "auto_checked": it.auto_checked(),
        "evidence": it.evidence,
    })
}

fn probe(path: &Path, cmd: &str) -> Result<Envelope> {
    let bytes = std::fs::read(path)
        .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{path:?}：{e}")))?;
    let hint_name = if is_video(path) { None } else { Some("") };
    let detection = format_detect::probe_bytes(&bytes, hint_name);
    let mut data = serde_json::json!({
        "path": path.display().to_string(),
        "bytes": bytes.len(),
        "detection": serde_json::to_value(&detection).map_err(|e| cli_err(121, "探测结果序列化失败").with_detail(e.to_string()))?,
    });
    if is_video(path) {
        match find_ffprobe() {
            Some(exe) => {
                let runner = SystemProbe::new(&exe, std::time::Duration::from_secs(30));
                let json = runner.probe(path)?;
                let streams = parse_video_streams(&json)?;
                let rows: Vec<serde_json::Value> = streams
                    .iter()
                    .map(|s| {
                        let rat = |r: Option<core_model::time::Rational>| {
                            r.map(|v| format!("{}/{}", v.num(), v.den()))
                        };
                        serde_json::json!({
                            "index": s.index,
                            "width": s.width,
                            "height": s.height,
                            "r_frame_rate": rat(s.r_frame_rate),
                            "avg_frame_rate": rat(s.avg_frame_rate),
                            "duration_seconds": s.duration.map(|v| format!("{}/{}", v.num(), v.den())),
                            "hdr_signaled": s.hdr_signaled,
                            // 16 §2 的 HDR 判定不只看 side data：transfer 才是主要依据，
                            // 所以两个都报出来，用户能自己看出结论是怎么来的
                            "is_hdr": s.is_hdr(),
                            "color": {
                                "primaries": s.color.primaries,
                                "transfer": s.color.transfer,
                                "matrix": s.color.matrix,
                                "range": s.color.range,
                                "pix_fmt": s.color.pix_fmt,
                            },
                            "fps_candidates": s
                                .fps_candidates()
                                .iter()
                                .map(|f| f.to_label())
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                data["video_streams"] = serde_json::Value::Array(rows);
                // 16 §3：候选值 + 证据，永不自动改时间轴。没有字幕上下文时
                // canvas/fps 传 None，建议就只由视频本身决定（这是 CLI 单文件探测的语义）。
                if let Some(first) = streams.first() {
                    let set =
                        media_probe::suggest_for(first, &media_probe::SubtitleContext::default());
                    data["suggestions"] = serde_json::json!({
                        "items": set.items.iter().map(suggestion_row).collect::<Vec<_>>(),
                        // 恒为 false 的这条承诺必须出现在输出里，否则"永不自动降速"只是代码里的注释
                        "recommends_speed_conversion": set.recommends_speed_conversion,
                        "notes": set.notes,
                    });
                }
                data["ffprobe"] = match FfprobeIdentity::of(&exe) {
                    Ok(id) => serde_json::json!({
                        "path": id.path.display().to_string(),
                        "size": id.size,
                        "digest_sha256": id.digest,
                    }),
                    // 身份读不出来也要继续探测：报路径，不假装哈希存在
                    Err(_) => serde_json::json!({ "path": exe.display().to_string() }),
                };
            }
            None => {
                data["note"] = serde_json::json!(
                    "未找到 ffprobe.exe：视频信息未读取（doctor 会给出搜索路径）"
                );
            }
        }
    }
    Ok(Envelope::ok(cmd, data))
}

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some(
            "mkv" | "mp4" | "m4v" | "ts" | "m2ts" | "avi" | "mov" | "webm" | "flv" | "mpg" | "mpeg"
        )
    )
}

/// PATH + 项目内 tools/bin 里找 ffprobe.exe。
pub fn find_ffprobe() -> Option<PathBuf> {
    // 显式覆盖优先：与 STK_LIBASS_DLL 同一套纪律，门阶不需要把二进制装进 PATH
    if let Ok(p) = std::env::var("STK_FFPROBE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    if let Ok(path) = std::env::var("PATH") {
        for d in std::env::split_paths(&path) {
            let cand = d.join(exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    // 开发期回落：仓库旁边的 tools/bin（与 libass 的 tools/vcpkg 同一位置约定）。
    // 早先这里写的是 `crates/cli/../tools/bin`，也就是 crates/tools/bin —— 那个目录从来不存在，
    // 所以这条回落在真机上从来没生效过（ISSUES AF2）。
    let local = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("tools")
        .join("bin")
        .join(exe_name);
    if local.is_file() {
        return Some(local);
    }
    None
}

/// doctor 要能回答"我为什么没找到 ffprobe"：把搜索过的地方如实列出来。
pub fn ffprobe_search_paths() -> Vec<String> {
    let exe_name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    let mut out = vec!["$STK_FFPROBE（未设置则跳过）".to_string()];
    out.push(format!("$PATH 里的 {exe_name}"));
    out.push(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("tools")
            .join("bin")
            .join(exe_name)
            .display()
            .to_string(),
    );
    out
}

/// 一次写出的全部上下文（参数太多会被 clippy 拦，说明这里本该是一个结构）。
struct Finish<'a> {
    inv: &'a Invocation,
    input: &'a Path,
    fallback_ext: &'a str,
    cmd: &'a str,
    loss: LossReport,
    data: serde_json::Value,
}

fn finish_text(
    f: Finish<'_>,
    bytes: &[u8],
    verify: impl FnOnce(&[u8]) -> Result<()>,
) -> Result<Envelope> {
    let out_default = sibling(f.input, f.fallback_ext);
    let emitted = pipeline::emit(
        f.inv.out.as_deref(),
        &out_default,
        bytes,
        verify,
        f.inv.dry_run,
        // 必须用调用方传来的策略：写死 Rename 会让 `--overwrite skip|replace`
        // 与 batch spec 里的 output.overwrite 变成装饰（实测：skip 测试拿到 verified）
        f.inv.overwrite,
    )?;
    let note = emitted.describe();
    let mut env = Envelope::ok(f.cmd, f.data)
        .with_loss(&f.loss)
        .with_writes(vec![WriteRecord {
            path: note.path,
            bytes: note.bytes,
            state: note.state,
        }]);
    if let Some(map) = env.data.as_mut().and_then(|v| v.as_object_mut()) {
        map.insert(
            "lossless".to_string(),
            serde_json::json!(f.loss.is_lossless()),
        );
    }
    Ok(env)
}

fn sibling(input: &Path, ext: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let mut p = input
        .parent()
        .unwrap_or(Path::new("."))
        .join(format!("{stem}.stk-{ext}"));
    p.set_extension(ext);
    p
}

fn verify_ass_bytes(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)
        .map_err(|e| cli_err(122, "输出不是合法 UTF-8").with_detail(e.to_string()))?;
    let doc = AssDocument::parse(text);
    if doc.render() != text {
        return Err(cli_err(123, "输出无法按自身字节重新解析为同一文档"));
    }
    Ok(())
}

fn verify_srt_bytes(bytes: &[u8]) -> Result<()> {
    let (sub, _) = parse_srt(bytes)?;
    let again = write_srt(&sub)?;
    if again != bytes {
        return Err(cli_err(124, "SRT 输出二次写出不稳定"));
    }
    Ok(())
}

fn convert(
    inv: &Invocation,
    input: &Path,
    to: FormatKind,
    canvas: Option<Canvas>,
    script_policy: ScriptResolutionPolicy,
    layout_policy: LayoutResPolicy,
    cmd: &str,
) -> Result<Envelope> {
    let src = load_input(input)?;
    let canvas = canvas.unwrap_or(Canvas::FHD1080);
    match (src.format, to) {
        (FormatKind::Srt, FormatKind::Srt) => Err(cli_err(125, "输入已经是 SRT")),
        (FormatKind::Srt, FormatKind::Ass) | (FormatKind::Srt, FormatKind::Ssa) => {
            let (sub, mut loss) = parse_srt(&src.loaded.bytes)?;
            let text = pipeline::srt_to_ass(&sub, canvas, &file_name(input), &mut loss);
            if script_policy == ScriptResolutionPolicy::Rewrite {
                loss.user_attention("SRT→ASS 直接生成目标画布的 PlayRes，不存在脚本重写问题");
            }
            let bytes = text.into_bytes();
            let data = serde_json::json!({
                "from": "srt", "to": kind_ext(to), "cues": sub.cues.len(), "canvas": canvas.label(),
                "detected": src.detection.clone(), "input_encoding": src.loaded.encoding
            });
            finish_text(
                Finish {
                    inv,
                    input,
                    fallback_ext: "ass",
                    cmd,
                    loss,
                    data,
                },
                &bytes,
                verify_ass_bytes,
            )
        }
        (FormatKind::Ass, FormatKind::Srt) => {
            let doc = AssDocument::parse(&src.loaded.text);
            let (sub, loss) = pipeline::ass_to_srt(&doc)?;
            let bytes = write_srt(&sub)?;
            let data = serde_json::json!({
                "from": "ass", "to": "srt", "cues": sub.cues.len(),
                "detected": src.detection.clone(), "input_encoding": src.loaded.encoding,
                "note": "样式/特效/卡拉OK 信息按 loss 报告丢弃"
            });
            finish_text(
                Finish {
                    inv,
                    input,
                    fallback_ext: "srt",
                    cmd,
                    loss,
                    data,
                },
                &bytes,
                verify_srt_bytes,
            )
        }
        (FormatKind::Ass, FormatKind::Ass) | (FormatKind::Ass, FormatKind::Ssa) => {
            let mut doc = AssDocument::parse(&src.loaded.text);
            let mut loss = LossReport::new();
            let g = pipeline::geometry_for(&doc, canvas, script_policy, layout_policy)?;
            let applied: ResApplied = transform_resolution::apply(&mut doc, &g, &mut loss)?;
            let bytes = doc.render().into_bytes();
            let data = serde_json::json!({
                "from": "ass", "to": kind_ext(to),
                "script_policy": format!("{script_policy:?}"),
                "layout_policy": format!("{layout_policy:?}"),
                "canvas": canvas.label(),
                "dialogues": doc.dialogue_count(),
                "detected": src.detection.clone(), "input_encoding": src.loaded.encoding,
                "changed": applied.total(),
                // 字节没被近似不等于"可声明等效"：Rewrite 的等效性只能由 differential corpus 判定（09 §9）
                "corpus_required": applied.total() > 0,
                "detail": serde_json::to_value(applied).unwrap_or(serde_json::Value::Null),
            });
            finish_text(
                Finish {
                    inv,
                    input,
                    fallback_ext: "ass",
                    cmd,
                    loss,
                    data,
                },
                &bytes,
                verify_ass_bytes,
            )
        }
        (from, to_kind) => Err(cli_err(
            126,
            &format!("暂不支持 {from:?} → {to_kind:?} 的转换"),
        )),
    }
}

/// 输出用的扩展名，同时作为 JSON 里的目标格式名。
fn kind_ext(k: FormatKind) -> &'static str {
    match k {
        FormatKind::Ass | FormatKind::Ssa => "ass",
        FormatKind::Srt => "srt",
        FormatKind::Vtt => "vtt",
        FormatKind::Pgs => "sup",
        FormatKind::Bdn => "xml",
        FormatKind::Text => "txt",
    }
}

fn file_name(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subtitle")
        .to_string()
}

fn time(
    inv: &Invocation,
    input: &Path,
    cfg: &TimeConfig,
    spec: Option<&transform_time::TwoPointSpec>,
    piecewise: &transform_time::PiecewiseSpec,
    cmd: &str,
) -> Result<Envelope> {
    let src = load_input(input)?;
    match src.format {
        FormatKind::Ass => {
            let mut doc = AssDocument::parse(&src.loaded.text);
            let (n, loss) = pipeline::time_ass(&mut doc, cfg, spec, piecewise)?;
            let bytes = doc.render().into_bytes();
            let data =
                serde_json::json!({"format": "ass", "mode": cfg.mode.label(), "dialogues": n});
            finish_text(
                Finish {
                    inv,
                    input,
                    fallback_ext: "ass",
                    cmd,
                    loss,
                    data,
                },
                &bytes,
                verify_ass_bytes,
            )
        }
        FormatKind::Srt => {
            let (sub, loss) = parse_srt(&src.loaded.bytes)?;
            let (moved, tl_loss) = pipeline::time_srt(&sub, cfg, spec, piecewise)?;
            let mut loss = loss;
            loss.merge(&tl_loss);
            let bytes = write_srt(&moved)?;
            let data = serde_json::json!({"format": "srt", "mode": cfg.mode.label(), "cues": moved.cues.len()});
            finish_text(
                Finish {
                    inv,
                    input,
                    fallback_ext: "srt",
                    cmd,
                    loss,
                    data,
                },
                &bytes,
                verify_srt_bytes,
            )
        }
        other => Err(cli_err(127, &format!("time 不支持的输入格式：{other:?}"))),
    }
}

fn hdr(
    inv: &Invocation,
    input: &Path,
    settings: &core_model::profile::HdrSettings,
    cmd: &str,
) -> Result<Envelope> {
    let src = load_input(input)?;
    if src.format != FormatKind::Ass {
        return Err(cli_err(128, "hdr 只作用于 ASS/SSA 的 &H… 颜色"));
    }
    let mut doc = AssDocument::parse(&src.loaded.text);
    let mut loss = LossReport::new();
    let applied: HdrApplied = transform_hdr::apply(&mut doc, settings, &mut loss)?;
    let bytes = doc.render().into_bytes();
    let data = serde_json::json!({
        "format": "ass",
        "mode": format!("{:?}", settings.mode),
        "algorithm_version": transform_hdr::ALGORITHM_VERSION,
        "factor": format!("{:?}", transform_hdr::factor(settings)?),
        "changed": applied.total(),
        "detail": serde_json::to_value(applied).unwrap_or(serde_json::Value::Null),
    });
    finish_text(
        Finish {
            inv,
            input,
            fallback_ext: "ass",
            cmd,
            loss,
            data,
        },
        &bytes,
        verify_ass_bytes,
    )
}

fn pgs_adjust(
    inv: &Invocation,
    input: &Path,
    ops: &[codec_pgs::fastpath::FastOp],
    cmd: &str,
) -> Result<Envelope> {
    let stream =
        codec_pgs::read_file(&std::fs::read(input).map_err(|e| {
            cli_err(110, "无法读取输入文件").with_detail(format!("{input:?}：{e}"))
        })?)?;
    let before = stream.segments.clone();
    let after = apply_pgs(&before, ops)?;
    check_timeline(&after)?;
    // 只改时间/位置/调色板的 Fast Path 不允许动 ODS 载荷（13 §3.1）
    let invariant = ods_invariant(&before, &after)?;
    // 这里原先还有一句 `if !touches_objects && !invariant.equal { 报错 }` —— 那是**永不可达**的分支：
    // `ods_invariant` 在发现任何一个 ODS payload 变了的时候就直接失败（STK-PGS-0088），
    // Ok 出来的 `equal` 只能是 true。所以真正的把关在 88 那里，与这次是哪种 op 无关
    // （MoveBy 也不能夹带一次重编码，因为检查只看字节，不看操作类型）。
    // 留一个永远为真的布尔值当"证据"，比不证据更糟：它会让人以为检查过而其实没检查。
    let bytes = write_stream(&after)?;
    let verify = |b: &[u8]| -> Result<()> {
        let again = parse_stream(b)?;
        if again.len() != after.len() {
            return Err(cli_err(130, "SUP 输出段数不一致"));
        }
        Ok(())
    };
    let loss = LossReport::new();
    let data = serde_json::json!({
        "segments": after.len(),
        "display_sets": stream.display_set_count(),
        "canvas": stream.canvas().map(|(w, h)| format!("{w}x{h}")),
        "ods_invariant": {
            "objects": invariant.objects,
            // 0 个 ODS 时没有"通过"可言，所以不折摘要：空集合上的全称命题是真的，但结论什么也没说
            "aggregate_sha256": if invariant.objects > 0 { Some(invariant.aggregate.clone()) } else { None },
            "per_object": invariant.per_object.iter().map(|(id, hex)| serde_json::json!({
                "object": id, "sha256": hex,
            })).collect::<Vec<_>>(),
            "meaning": "13 §3.1：每个 ODS payload 的 SHA-256。时间轴被动过之后这些哈希必须逐个不变，\
                        变了就不是 Fast Path 而是一次重编码（报 STK-PGS-0088 并拒绝输出）。\
                        aggregate_sha256 是这份清单折成的单一值，存下来即可跨版本、跨机器复核。",
        },
        "ops": ops.len(),
    });
    finish_text(
        Finish {
            inv,
            input,
            fallback_ext: "sup",
            cmd,
            loss,
            data,
        },
        &bytes,
        verify,
    )
}

/// libass 只从命令参数或明确的环境变量读取路径，不在磁盘上搜索 DLL（23 §安全）。
pub(crate) fn libass_dll(explicit: Option<&Path>) -> Result<PathBuf> {
    let path = explicit
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("STK_LIBASS_DLL").map(PathBuf::from));
    let Some(path) = path else {
        return Err(cli_err(141, "渲染任务需要 libass，但没有给出 DLL 路径")
            .with_detail("传入 --libass-dll PATH，或设置 STK_LIBASS_DLL")
            .with_action("构建或安装本机 libass DLL 后重试"));
    };
    if !path.is_file() {
        return Err(cli_err(141, "指定的 libass DLL 不存在或不是文件")
            .with_detail(format!(
                "{}；传入 --libass-dll PATH 或设置 STK_LIBASS_DLL",
                path.display()
            ))
            .with_action("检查 DLL 路径后重试"));
    }
    // 23 §DLL：**PE arch 检测**。不先查位数，`LoadLibrary` 只会返回一句含糊的
    // “不是有效的 Win32 应用程序”，用户看到的信息就只剩这一句。
    // 已有实现（12 §3）在 core-model::pe —— 复用它而不是另写一份：
    // 两份实现对同一个 DLL 会给出不同答案。
    let arch = core_model::pe::arch_of_file(&path).map_err(|e| {
        cli_err(142, "指定的渲染库不是有效的 PE 文件")
            .with_detail(format!("{}：{e}", path.display()))
            .with_action("确认这是一个真的 Windows DLL，而不是改名或截断的文件")
    })?;
    // 主程序是 x64（12 §3）。位数不符时现在就报，而不是交给 LoadLibrary 报一个更远的原因。
    if !arch.is_x64() {
        return Err(cli_err(143, "渲染库的位数与本程序不匹配，加载必然失败")
            .with_detail(format!(
                "{}：本程序是 x64，该库是 {}",
                path.display(),
                arch.label()
            ))
            .with_action("换成 x64 的那一份；x86 host 与 x64 host 各自的库不能互换"));
    }
    Ok(path)
}

/// 最后一个 Dialogue 事件的结束时刻（向上取整到 ms）。
/// 向上取整是为了"结束那一帧仍然画得出来"；向下取整会悄悄截掉尾巴。
pub(crate) fn last_end_ms_of(doc: &AssDocument) -> Result<i64> {
    let dialogues = doc.dialogues();
    if dialogues.is_empty() {
        return Err(cli_err(140, "没有 Dialogue 事件，产不出字幕流"));
    }
    let mut last = 0i64;
    for d in &dialogues {
        let r = d.end()?.as_rational();
        let ms = if r.num() <= 0 {
            0
        } else {
            (r.num() * 1000 + r.den() - 1) / r.den()
        };
        last = last.max(ms);
    }
    Ok(last)
}

/// 14 §ASS→BDN 与 §ASS→SUP 共用的采样网格。两条流水线必须落在同一组时刻上，
/// 否则"同一输入、两种输出"的时间轴会各自漂移。
///
/// 三个时刻要分清：`steps_ms` 是要真渲染的采样点，`blank_ms` 是末尾那个空白点
/// （erase / 事件收尾靠它），`stream_end_ms` 必须**晚于**最后一个采样点
/// （否则 author 按定义拒绝：STK-PGS-0112）。
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    pub steps_ms: Vec<i64>,
    pub blank_ms: i64,
    pub stream_end_ms: i64,
    /// 分数帧率下帧号→ms 取过整；调用方要把这件事写进 LossReport
    pub rounded: bool,
}

impl Grid {
    /// 需要出图的采样点（不含末尾空白点）。
    pub fn render_ms(&self) -> &[i64] {
        &self.steps_ms
    }
}

pub(crate) fn sample_grid(fps: Fps, last_end_ms: i64) -> Result<Grid> {
    let rate = fps.as_rational();
    if rate.num() <= 0 || rate.den() <= 0 {
        return Err(cli_err(142, "帧率必须为正").with_detail(fps.to_label()));
    }
    // libass 的 ass_render_frame 只接受整数 ms，whole_millis 会拒绝非整毫秒
    // 而不是偷偷舍入（STK-RND-0083），所以网格本身就落在整数 ms 上。
    let ms_at = |frame: i64| -> i64 { (frame * 1000 * rate.den() + rate.num() / 2) / rate.num() };
    let step_ms = ms_at(1).max(1);
    let blank_ms = last_end_ms + step_ms;
    let mut steps_ms = Vec::new();
    let mut frame = 0i64;
    while ms_at(frame) < blank_ms {
        steps_ms.push(ms_at(frame));
        frame += 1;
    }
    Ok(Grid {
        steps_ms,
        blank_ms,
        stream_end_ms: blank_ms + step_ms,
        rounded: rate.num() % rate.den() != 0,
    })
}

/// 14 §ASS→SUP：真 libass 渲染 → 帧采样 → pgs-author → 回读校验 → 原子写出。
///
/// 采样网格落在**整数毫秒**上：libass 的 `ass_render_frame` 只接受整数 ms，
/// `whole_millis` 会拒绝非整毫秒而不是偷偷舍入（STK-RND-0083）。所以 23.976 这类
/// 分数帧率下帧号→ms 按半进位取整，并且**把这件事写进 LossReport**——
/// 网格被量化过是事实，不写出来就是让用户猜。
#[allow(clippy::too_many_arguments)]
fn pgs_encode(
    inv: &Invocation,
    input: &Path,
    fps: Fps,
    canvas: Canvas,
    level: core_model::profile::ValidationLevel,
    dll: Option<&Path>,
    default_font: &str,
    exact_colors: bool,
    snap_pts: bool,
    cmd: &str,
    shared: Option<&crate::front::SharedRender<'_>>,
    cache: &Option<crate::rendercache::DiskCache>,
) -> Result<Envelope> {
    let no_render_cache = !inv.render_cache;
    let mut loss_disk_hit = false;
    // 前半链（采样 → 变化检测）与 ASS→BDN 共用同一份实现；批量里同一输入的
    // 两种渲染输出通过 `shared` 只跑一次（14 §Multi-output）。
    // 先读输入并确认渲染器可用。缺 DLL 时无需扫描整套系统字体库。
    let text = crate::pipeline::load_text(input)?.text;
    let dll_path = libass_dll(dll)?;
    // 字体环境与注入计划在渲染之前算（17 §7）：算晚了，报告里的身份就只是事后追认
    let mut scanner = crate::fontenv::Scanner::with_cache(crate::fontenv::cache_mode(inv));
    let (font_env, env_ms) = scanner.env_for(&text)?;
    let plan = if inv.inject_fonts {
        Some(scanner.inject_plan(&text)?)
    } else {
        None
    };
    let job = crate::front::RenderJob {
        input,
        fps,
        canvas,
        dll: Some(&dll_path),
        default_font,
        inject: plan.as_deref(),
    };
    let rendered = match crate::front::resolve_cached(
        shared,
        if no_render_cache {
            None
        } else {
            cache.as_ref()
        },
        &job,
    ) {
        Ok((r, from_disk)) => {
            if from_disk {
                // 命中磁盘缓存是**报告要写出来的事实**：它意味着这次没跑 native
                // 渲染，凡是从渲染结果反推的结论（时间、像素）都是上次那份。
                // 不写出来，同一命令两次运行的耗时差异就没有解释。
                loss_disk_hit = true;
            }
            r
        }
        Err(e) => return Err(e),
    };
    let grid = rendered.grid.clone();
    let mut loss = rendered.loss.clone();
    if loss_disk_hit {
        loss.preserved("37 §2：前半链命中了磁盘渲染缓存，未重跑 native 渲染");
    }

    let eopt = codec_pgs::encoder::EncodeOptions {
        canvas_width: canvas.width as u16,
        canvas_height: canvas.height as u16,
        matrix: codec_pgs::ColorMatrix::for_canvas(canvas.height),
        // 默认走 Advanced 量化档：真实抗锯齿渲染的颜色就是会超过 255（ISSUES Z3），
        // 但合并必须被度量并被用户看见；--exact-colors 把档位退回 Basic 的"超色即失败"
        colors: if exact_colors {
            codec_pgs::ColorPolicy::Exact
        } else {
            codec_pgs::ColorPolicy::MedianCut { max_colors: 255 }
        },
        ..Default::default()
    };
    let want = validator::ProfileWant {
        level,
        claim_compliant: false,
    };
    // AA5：可选把显示集起点吸到 --fps 的帧网格上。规格只要求"严格兼容声明"才必须上格
    // （09 §6、13 §2），普通 SUP 允许非帧边界，所以默认关 —— 开着会改动输出字节，必须显式请求。
    let mut authored_plan = rendered.plan;
    if snap_pts {
        let moved = authored_plan.snap_starts_to_grid(fps)?;
        loss.preserved(&format!(
            "AA5：{} 个显示集起点里 {moved} 个被吸到 {fps} 的帧网格（validator 的 PGS.TML.05 应随之消失）",
            authored_plan.events.len()
        ));
    }
    let a = pgs_author::author_from_plan(authored_plan, &eopt, &want, Some(fps), None)?;
    // author 的账（重复消除、空白转 erase、量化合并的颜色）要并进 CLI 的 loss 再交付。
    // 这一步最初漏了：信封里 quantized_frames=1、merged_colors=364，loss 却是空的、
    // lossless=true —— 失真记在了没人看的对象里，正是本仓库反复犯的那类错（ISSUES Z3 的余波）。
    loss.merge(&a.plan.loss);
    // 17 §7：这份像素是在哪一套字体上渲出来的必须成为身份的一部分。批量走 batch 里的
    // 同一套算法（库只建一次），单命令在这里算一次。
    let data = serde_json::json!({
        "font_env": crate::fontenv::env_json(&font_env, env_ms),
        "font_injection": crate::fontenv::injection_json(
            &rendered.injected,
            plan.as_deref().unwrap_or(&[]),
            inv.inject_fonts,
        ),
        "canvas": canvas.label(),
        "fps": fps.to_label(),
        "samples": rendered.samples,
        "stream_end_ms": grid.stream_end_ms,
        // 37 §2：缓存命不命中是**可核对的事实**，不是内部细节。
        // 同一命令两次运行耗时不同、或与 --no-render-cache 结果不同，
        // 读报告的人必须能一眼看出是那一层在起作用。
        "render_cache": {
            "enabled": cache.is_some(),
            "hit": loss_disk_hit,
            "counters": cache.as_ref().map(|c| {
                let s = c.stats();
                serde_json::json!({
                    "hits": s.hits,
                    "misses": s.misses,
                    "corrupt": s.corrupt,
                    "write_failures": s.write_failures,
                })
            }),
        },
        "events": a.plan.stats.events,
        "erase_events": a.plan.stats.erase_events,
        "eliminated": a.plan.stats.eliminated,
        "display_sets": a.encode_report.display_sets,
        "objects": a.encode_report.objects,
        "fragmented_objects": a.encode_report.fragmented_objects,
        "palette_entries": a.encode_report.max_palette_entries,
        "quantized_frames": a.encode_report.quantized_frames,
        "merged_colors": a.encode_report.merged_colors,
        "worst_color_error": a.encode_report.worst_color_error,
        "bytes": a.bytes.len(),
        "level": format!("{level:?}"),
        "findings": a.validation.text_lines(),
        "summary": a.validation.summary(),
        "no_blocking_findings": a.validation.no_blocking_findings(),
        "strict_badge_allowed": a.validation.strict_badge_allowed(level),
    });
    // 校验没过也先把全部发现交出去再退非零，不动旧文件（N3 / AT-OUT-001）
    if let Err(e) = a.ensure_valid() {
        return Ok(Envelope::fail_with(cmd, &e, data));
    }

    let expect_sets = a.encode_report.display_sets;
    let verify = move |b: &[u8]| -> Result<()> {
        let again = parse_stream(b)?;
        let sets = codec_pgs::display_set::group_display_sets(&again)?;
        if sets.len() != expect_sets {
            return Err(cli_err(143, "SUP 输出回读的显示集数与编码报告不一致")
                .with_detail(format!("报告 {expect_sets}，回读 {}", sets.len())));
        }
        Ok(())
    };
    finish_text(
        Finish {
            inv,
            input,
            fallback_ext: "sup",
            cmd,
            loss,
            data,
        },
        &a.bytes,
        verify,
    )
}

fn pgs_validate(
    input: &Path,
    level: core_model::profile::ValidationLevel,
    peak_bps: Option<i64>,
    cmd: &str,
) -> Result<Envelope> {
    let bytes = std::fs::read(input)
        .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{input:?}：{e}")))?;
    // 故意用 parse_stream 而不是 read_file：损坏文件也要能拿到全部发现（校验器的职责）
    let segments = parse_stream(&bytes)?;
    let want = validator::ProfileWant {
        level,
        claim_compliant: level == core_model::profile::ValidationLevel::Strict,
    };
    let bitrate_args = peak_bps.map(|bps| validator::BitrateArgs {
        limit_bps: bps,
        ..BitrateArgs::default()
    });
    let report = validator::pgs(&segments, &want, None, bitrate_args.as_ref());
    let measurement = bitrate_args
        .as_ref()
        .map(|a| validator::peak_bitrate(&segments, a));
    let canvas = segments.iter().find_map(|s| {
        codec_pgs::display_set::parse_pcs(&s.payload)
            .ok()
            .map(|p| format!("{}x{}", p.width, p.height))
    });
    let data = serde_json::json!({
        "segments": segments.len(),
        "canvas": canvas,
        "level": format!("{level:?}"),
        "findings": report.text_lines(),
        "summary": report.summary(),
        "no_blocking_findings": report.no_blocking_findings(),
        // 只有按 Strict 跑过才可能给徽章；Basic/Standard 的报告不能倒推（13 §6）
        "strict_badge_allowed": report.strict_badge_allowed(level),
        // 被拒的原因必须能看见：只说"不给徽章"等于把用户困在里面。
        // `coverage_gap` 列出真的**没跑**的层（当前只剩第 7 层设备语料）。
        "coverage_gap": report
            .findings
            .iter()
            .filter(|f| validator::report::ValidationReport::COVERAGE_GAP_CHECKS.contains(&f.check))
            .map(|f| f.check)
            .collect::<Vec<_>>(),
        "peak_bitrate": measurement,
        "layers": "1 结构 / 2 时间线 / 3 生命周期 / 4 profile / 5 解码器模型 / 6 码率测量；第 7 层设备语料未覆盖时会显式报告",
    });
    // 有 error/fatal：完整报告照旧交出去，用 ok=false 与非零退出码阻断流水线
    match validator::to_error(&report, "SUP") {
        Ok(()) => Ok(Envelope::ok(cmd, data)),
        Err(e) => Ok(Envelope::fail_with(cmd, &e, data)),
    }
}

fn doctor(cmd: &str) -> Result<Envelope> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let writable = test_writable(&cwd);
    let data_dir = core_model::DataDir::resolve_current_exe();
    let ffprobe = find_ffprobe();
    let identity = ffprobe
        .as_ref()
        .and_then(|p| FfprobeIdentity::of(p).ok())
        .map(|i| {
            serde_json::json!({
                "path": i.path.display().to_string(),
                "size": i.size,
                "digest_sha256": i.digest,
            })
        });
    let data = serde_json::json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "exe_extension": std::env::consts::EXE_SUFFIX,
        "pid": std::process::id(),
        "cwd": cwd.display().to_string(),
        "cwd_writable": writable,
        // 28 §4：数据目录落在哪、为什么落在那里。“便携没生效”这类反馈必须能一眼定位，
        // 所以 kind 与理由都要报出来，而不是只给一个路径。
        "data_dir": {
            "path": data_dir.path.as_ref().map(|p| p.display().to_string()),
            "kind": data_dir.kind.label(),
            "why": data_dir.why,
            "portable_flag": core_model::DataDir::PORTABLE_FLAG,
        },
        "font_cache": crate::fontenv::cache_path_report(),
        "ffprobe": identity,
        "ffprobe_search_paths": ffprobe_search_paths(),
        "render_host": render_host_report(),
        "long_path_support": "输出事务使用绝对路径 + tmp/rename（15/23）",
        "not_built": crate::args::not_built_commands(),
    });
    Ok(Envelope::ok(cmd, data))
}

/// 37 §2：磁盘渲染缓存的账目与清理。
///
/// 两件事必须说清楚，否则这条命令就没什么用：
/// **配额上限是多少**（用户要知道它会不会把盘吃满）、
/// **缓存目录在哪**（用户要知道删的是哪里、怎么手动清）。
/// `cache` 环境变量关掉时也要明说，而不是报一个看起来正常但其实是"没开"的空目录。
fn cache_cmd(action: crate::args::CacheAction, cmd: &str) -> Result<Envelope> {
    use crate::args::CacheAction;
    let Some(c) = crate::rendercache::DiskCache::from_env() else {
        return Ok(Envelope::ok(
            cmd,
            serde_json::json!({
                "enabled": false,
                "reason": "磁盘渲染缓存已关闭（37 §2）",
                "how_to_disable": "环境变量 STK_RENDERCACHE=0 或命令行 --no-render-cache",
            }),
        ));
    };
    let (bytes, count) = c.usage();
    match action {
        CacheAction::Stat => Ok(Envelope::ok(
            cmd,
            serde_json::json!({
                "enabled": true,
                "dir": c.dir().display().to_string(),
                "entries": count,
                "bytes": bytes,
                "budget_bytes": c.max_bytes(),
                // 环境变量与命令行都能改这两个值，把生效来源一并报出来
                "dir_env": "STK_RENDERCACHE_DIR",
                "budget_env": "STK_RENDERCACHE_MAX_BYTES",
            }),
        )),
        CacheAction::Clear => {
            let removed = c.clear().map_err(|e| {
                StkError::simple(Domain::Cache, 20, "清理磁盘渲染缓存失败")
                    .with_detail(format!("{}：{e}", c.dir().display()))
            })?;
            let (left_bytes, left_count) = c.usage();
            Ok(Envelope::ok(
                cmd,
                serde_json::json!({
                    "enabled": true,
                    "dir": c.dir().display().to_string(),
                    "removed_entries": removed,
                    "bytes_before": bytes,
                    "entries_after": left_count,
                    "bytes_after": left_bytes,
                }),
            ))
        }
    }
}

/// 36 §1 的 AppSettings：看、初始化、改一个字段。
///
/// 三件事必须报出来，否则这条命令跟没有一样：**生效的值**、
/// **值从哪来**（文件还是内置默认）、以及**文件在哪**（用户要知道改哪个文件）。
/// 28 §4 把“settings 落盘”列为未实现项，这里把它补上；同一个数据目录解析
/// （`DataDir`）让便携包里的设置也落在包内。
fn settings_cmd(
    action: crate::args::SettingsAction,
    key: Option<&str>,
    value: Option<&str>,
    cmd: &str,
) -> Result<Envelope> {
    use crate::args::SettingsAction;
    // `STK_SETTINGS` 优先：测试与“把设置放在别处”都要一条不依赖 exe 位置的路。
    let store = match std::env::var("STK_SETTINGS") {
        Ok(p) if !p.trim().is_empty() => core_model::SettingsStore::at(PathBuf::from(p)),
        _ => core_model::SettingsStore::current(),
    };
    match action {
        SettingsAction::Show => {
            let got = store.load();
            Ok(Envelope::ok(
                cmd,
                serde_json::json!({
                    "path": store.path().map(|p| p.display().to_string()),
                    "source": got.source.label(),
                    "settings": serde_json::to_value(settings_json(&got.settings))
                        .unwrap_or(serde_json::Value::Null),
                    // 非空 = 有些东西没被采纳。空列表和“没查”必须区分开，
                    // 所以这里总是给一个列表而不是省略键。
                    "problems": got.problems,
                    "quarantined": got.quarantined.map(|p| p.display().to_string()),
                }),
            ))
        }
        SettingsAction::Init => {
            // 先读一次：已有文件就把它的值当基准写回（未知字段也保留），
            // 而不是拿默认值把用户现有的设置盖掉。
            let got = store.load();
            if got.source == core_model::SettingsSource::File {
                return Err(StkError::simple(
                    Domain::Input,
                    320,
                    "设置文件已存在，init 不会覆盖它",
                )
                .with_detail(format!(
                    "{}（来源 {}）",
                    store
                        .path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    got.source.label()
                ))
                .with_action("直接编辑该文件，或用 `settings set <字段> <值>` 修改"));
            }
            let path = store.save(&got.settings)?;
            Ok(Envelope::ok(
                cmd,
                serde_json::json!({
                    "path": path.display().to_string(),
                    "source": "defaults",
                    "written": true,
                    "problems": got.problems,
                }),
            ))
        }
        SettingsAction::Set => {
            let key = key.unwrap_or_default();
            let value = value.unwrap_or_default();
            let got = store.load();
            // schema 比当前高就不写：新版写的设置里可能有旧版无法表达的语义，
            // 强行写回等于降级破坏（36 §3）。
            if got.settings.schema > core_model::settings::CURRENT_SCHEMA {
                return Err(StkError::simple(
                    Domain::Input,
                    321,
                    "设置文件的 schema 高于本程序支持的版本，拒绝写回",
                )
                .with_detail(format!(
                    "文件 schema={}，本程序支持到 {}",
                    got.settings.schema,
                    core_model::settings::CURRENT_SCHEMA
                ))
                .with_action("用较新版本的程序修改，或手工编辑该文件"));
            }
            let mut s = got.settings.clone();
            apply_setting(&mut s, key, value)?;
            let path = store.save(&s)?;
            Ok(Envelope::ok(
                cmd,
                serde_json::json!({
                    "path": path.display().to_string(),
                    "changed": key,
                    "settings": serde_json::to_value(settings_json(&s))
                        .unwrap_or(serde_json::Value::Null),
                    // 之前读时的警告（例如别的字段格式不对）也要一并给出，
                    // 否则用户会以为整份文件都好了。
                    "problems": got.problems,
                }),
            ))
        }
    }
}

/// 受管字段的白名单。不在表里的字段一律拒绝，理由：
/// 静默接受一个拼错的字段名，用户会以为设置生效了 —— 而它其实写进了一个没人读的地方。
fn apply_setting(s: &mut core_model::AppSettings, key: &str, value: &str) -> Result<()> {
    let bad = |why: String| {
        StkError::simple(Domain::Input, 322, "设置取值不合法")
            .with_detail(why)
            .with_action("settings show 可以看到当前值与字段名")
    };
    match key {
        "log_level" => {
            // 与 tracing 的过滤语法对齐：只收已知级别名，避免把拼错的值写进文件。
            const LEVELS: &[&str] = &["off", "error", "warn", "info", "debug", "trace"];
            if !LEVELS.contains(&value) {
                return Err(bad(format!(
                    "log_level 只能是 {}，收到 {value:?}",
                    LEVELS.join("/")
                )));
            }
            s.log_level = value.to_string();
        }
        "ui_language" => {
            // 空串 = 清掉这个偏好（跟随系统），与写显式 null 同义。
            s.ui_language = (!value.is_empty()).then(|| value.to_string());
        }
        "default_fps" => {
            core_model::Fps::from_label(value).map_err(|_| {
                bad(format!(
                    "default_fps 取值 {value:?} 不是合法帧率（整数、a/b 分数或 23.976 这类标签）"
                ))
            })?;
            s.default_fps = value.to_string();
        }
        "default_canvas" => {
            let ok = value
                .split_once(['x', 'X'])
                .and_then(|(w, h)| {
                    Some((w.trim().parse::<u32>().ok()?, h.trim().parse::<u32>().ok()?))
                })
                .is_some_and(|(w, h)| w > 0 && h > 0);
            if !ok {
                return Err(bad(format!(
                    "default_canvas 要写成 WxH 形式（例：1920x1080），收到 {value:?}"
                )));
            }
            s.default_canvas = value.to_string();
        }
        "experimental" => {
            return Err(bad(
                "experimental 是一个开关表，不能整表赋值；直接编辑设置文件里的 experimental 对象"
                    .to_string(),
            ));
        }
        "schema" => {
            return Err(bad(
                "schema 由程序维护，不应手工改；它决定迁移行为".to_string()
            ));
        }
        other => {
            return Err(bad(format!(
                "没有名为 {other:?} 的设置字段（受管字段：log_level/ui_language/default_fps/default_canvas）"
            )));
        }
    }
    Ok(())
}

/// 设置的 JSON 表示。手写而不是给 `AppSettings` 派生 `Serialize`：
/// 派生会把 `extra` 的原始 JSON 也当成“我们管的东西”输出，而它必须只在
/// `serialize` 里按 36 §3 合并 —— 两处表达同一件事就会分叉。
fn settings_json(s: &core_model::AppSettings) -> serde_json::Value {
    serde_json::json!({
        "schema": s.schema,
        "ui_language": s.ui_language,
        "log_level": s.log_level,
        "default_fps": s.default_fps,
        "default_canvas": s.default_canvas,
        "experimental": s.experimental,
        "preserved_unknown_fields": s.extra.keys().collect::<Vec<_>>(),
    })
}

/// 22 §5 的诊断包。
///
/// 规格里最要紧的一句是「**默认不包含原字幕/视频**；用户显式选择才附样本」。
/// 所以这里的分工是：
///
/// * 环境、版本、依赖身份、设置（脱敏）、缓存账目 —— 总是收，都不来自用户内容；
/// * 输入文件 —— 只有 `--include-input` 给了才收，且在报告里单独列出来。
///
/// 报告必须回答两个问题，否则这个功能不可用：**包写在哪**、以及**里面有没有用户内容**。
fn diagnose_cmd(
    inv: &Invocation,
    include_input: &[std::path::PathBuf],
    job_state: Option<&Path>,
    cmd: &str,
) -> Result<Envelope> {
    let mut b = crate::diagnostic::Bundle::new();
    b.text(
        "environment.json",
        serde_json::to_string_pretty(&crate::diagnostic::environment(env!("CARGO_PKG_VERSION")))
            .unwrap_or_else(|_| "{}".to_string()),
    );

    // 设置：脱敏后收（22 §5 的 settings-redacted）。
    // 脱敏后仍然保留“这个键存在”这个事实，而不是整行删掉。
    let store = core_model::SettingsStore::current();
    let got = store.load();
    b.text(
        "settings.json",
        serde_json::to_string_pretty(&crate::diagnostic::redact_settings(&serde_json::json!({
            "settings": {
                "schema": got.settings.schema,
                "log_level": got.settings.log_level,
                "default_fps": got.settings.default_fps,
                "default_canvas": got.settings.default_canvas,
                "ui_language": got.settings.ui_language,
            },
            "source": got.source.label(),
            // 路径本身就是敏感信息（含用户名），所以它经脱敏逻辑之后才写进去
            "path": store.path().map(|p| p.display().to_string()),
            "problems": got.problems,
        })))
        .unwrap_or_else(|_| "{}".to_string()),
    );

    // 依赖身份：22 §5 明确要求。ffprobe/renderer host 的身份决定能不能复现。
    let mut deps = serde_json::Map::new();
    deps.insert(
        "ffprobe".into(),
        find_ffprobe()
            .map(|p| serde_json::json!(p.display().to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    deps.insert(
        "render_host".into(),
        serde_json::to_value(render_host_report()).unwrap_or(serde_json::Value::Null),
    );
    deps.insert(
        "libass_dll".into(),
        libass_dll(None)
            .map(|p| serde_json::json!(p.display().to_string()))
            .unwrap_or(serde_json::Value::Null),
    );
    b.text(
        "dependencies.json",
        serde_json::to_string_pretty(&serde_json::Value::Object(deps))
            .unwrap_or_else(|_| "{}".to_string()),
    );

    // 缓存账目：它影响“同一命令两次跑为什么不一样”。
    let cache = crate::rendercache::DiskCache::from_env();
    let (bytes, count) = cache.as_ref().map(|c| c.usage()).unwrap_or((0, 0));
    b.text(
        "render_cache.json",
        serde_json::to_string_pretty(&serde_json::json!({
            "enabled": cache.is_some(),
            "dir": cache.as_ref().map(|c| c.dir().display().to_string()),
            "entries": count,
            "bytes": bytes,
            "budget_bytes": cache.as_ref().map(|c| c.max_bytes()),
        }))
        .unwrap_or_else(|_| "{}".to_string()),
    );

    // 诊断包自己也把自己的红线写进包里：收包的人不必去猜。
    b.text(
        "README.txt",
        "SubtitleToolkit 诊断包（22 §5）\n\n\
         默认只包含环境、版本、依赖身份、脱敏后的设置与**本程序自己的日志**，\n\
         不含用户的字幕或视频内容。日志里只记命令名与结果，不记文件路径。\n\
         `input/` 下的条目是用户用 --include-input 显式附上的，请在对外发送前自行确认。\n\
         settings.json 里与路径/凭据相关的键已替换为 ⟨已脱敏⟩。\n",
    );

    if let Some(js) = job_state {
        b.file(Some("job-state.json"), js, false);
    }
    // 22 §5 明确要求把 logs 收进诊断包。收**所有**现存的日志文件：
    // 一次故障现场的“上一次跑过什么”往往就在前一天那个文件里，
    // 只收今天那份等于把上下文切一半。
    if let Some(d) = core_model::log_dir() {
        if let Ok(rd) = std::fs::read_dir(d) {
            let mut logs: Vec<std::path::PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("stk-") && n.contains(".log"))
                })
                .collect();
            logs.sort();
            for p in logs {
                let name = p
                    .file_name()
                    .map(|n| format!("logs/{}", n.to_string_lossy()))
                    .unwrap_or_else(|| "logs/unnamed.log".to_string());
                // 日志**不是**用户内容：它们是本程序自己的输出，且不含字幕文本
                // （只记命令名与结果）。标成用户内容会让报告把“可以外发”的东西
                // 说成需要审查。
                b.file(Some(&name), &p, false);
            }
        }
    }
    for f in include_input {
        // 放进 input/ 子目录：一眼能看出"这些是用户内容"
        let name = f
            .file_name()
            .map(|n| format!("input/{}", n.to_string_lossy()))
            .unwrap_or_else(|| "input/unnamed".to_string());
        b.file(Some(&name), f, true);
    }

    let user_content: Vec<String> = b
        .user_content_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect();
    let names: Vec<String> = b.names().into_iter().map(|s| s.to_string()).collect();

    // 输出路径：默认 <数据目录>/diagnostics/stk-diag-<pid>.zip。
    // 用 pid 而不是时间戳：它让同一次故障里跑几次也能并存，且不需要时区处理。
    let out = match &inv.out {
        Some(p) => p.clone(),
        None => {
            let dir = core_model::DataDir::resolve_current_exe();
            let base = dir
                .file("diagnostics")
                .unwrap_or_else(|| std::env::temp_dir().join("subtitletool-diagnostics"));
            base.join(format!("stk-diag-{}.zip", std::process::id()))
        }
    };
    let zip = b.to_zip()?;
    // 回读校验：把写出的字节能读回一个合法 zip（EOCD 存在、条目数与声明一致）。
    // 与其它产物同一条纪律：能写不能读的产物不算写成功（15 §6、23）。
    crate::pipeline::emit(
        inv.out.as_deref(),
        &out,
        &zip,
        crate::diagnostic::verify_zip,
        inv.dry_run,
        inv.overwrite,
    )?;

    Ok(Envelope::ok(
        cmd,
        serde_json::json!({
            "path": out.display().to_string(),
            "entries": names,
            "contains_user_content": !user_content.is_empty(),
            "user_content_entries": user_content,
            "note": "默认不含原字幕/视频；user_content_entries 里的条目是显式附上的（22 §5）",
        }),
    ))
}

/// host 可执行文件在哪：`STK_RENDERER_HOST` → 与 CLI 同目录的 `subtitletool-render-host.exe`。
/// 找不到不是错误，是诊断信息（28 §2 把它列为独立产物，没打包就是没有）。
pub fn renderer_host_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_RENDERER_HOST") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let name = format!("subtitletool-render-host{}", std::env::consts::EXE_SUFFIX);
    let sibling = dir.join(&name);
    if sibling.is_file() {
        return Some(sibling);
    }
    // `cargo test` places the test executable under `target/<profile>/deps/`.
    if dir.file_name().is_some_and(|part| part == "deps") {
        let workspace_bin = dir.parent()?.join(name);
        if workspace_bin.is_file() {
            return Some(workspace_bin);
        }
    }
    None
}

/// 12 §2 的可用性问题："能不能把第三方渲染 DLL 关在另一个进程里"。
/// 诊断要回答三件事：host 在哪、位数配不配、握手成没成——而不是只说"有/没有"。
fn render_host_report() -> serde_json::Value {
    let Some(exe) = renderer_host_exe() else {
        return serde_json::json!({
            "available": false,
            "reason": "找不到 subtitletool-render-host.exe（设 STK_RENDERER_HOST 指到它）",
        });
    };
    let Ok(dll) = libass_dll(None) else {
        return serde_json::json!({
            "available": false,
            "host": exe.display().to_string(),
            "reason": "没有可用的 libass：给 --libass-dll 或设 STK_LIBASS_DLL",
        });
    };
    let spec = renderer_host::client::HostSpec {
        host_exe: &exe,
        dll: &dll,
        storage: Canvas::FHD1080,
        output: Canvas::FHD1080,
        default_font: "Arial",
        timeout: std::time::Duration::from_secs(10),
        crash_injection: None,
    };
    match renderer_host::client::Host::start(&spec) {
        Ok(mut host) => {
            let hs = host.handshake().clone();
            let contained = host.contained();
            let job_error = host.job_error();
            let _ = host.shutdown();
            serde_json::json!({
                "available": true,
                "host": exe.display().to_string(),
                "handshake": hs,
                // 孙进程约束状态必须报出来：附着失败意味着"崩溃时只能杀直接子进程"（Z6）
                "tree_contained": contained,
                "job_error": job_error,
                "job_error_name": win_job::error_name(job_error),
            })
        }
        Err(e) => serde_json::json!({
            "available": false,
            "host": exe.display().to_string(),
            "code": e.code(),
            "reason": e.title(),
        }),
    }
}

fn test_writable(dir: &Path) -> bool {
    let probe = dir.join(format!("stk-doctor-{}.tmp", std::process::id()));
    let ok = std::fs::write(&probe, b"ok").is_ok();
    if ok {
        std::fs::remove_file(&probe).ok();
    }
    ok
}

impl Envelope {
    fn with_writes(mut self, w: Vec<WriteRecord>) -> Envelope {
        self.writes = w;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::Invocation;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-cli-run-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn inv(command: Command, out: Option<PathBuf>, dry: bool) -> Invocation {
        Invocation {
            command,
            json: true,
            out,
            dry_run: dry,
            // 单命令测试走默认策略：覆盖策略本身的行为在 task-engine 的事务测试里锁
            overwrite: task_engine::OverwritePolicy::Rename,
            inject_fonts: false,
            no_font_cache: false,
            render_cache: true,
        }
    }

    #[test]
    fn an_explicit_missing_libass_path_is_rejected_before_render_setup() {
        let path =
            std::env::temp_dir().join(format!("stk-missing-libass-{}.dll", std::process::id()));
        assert!(!path.exists(), "测试路径必须不存在：{}", path.display());
        let err = libass_dll(Some(&path)).expect_err("不存在的 DLL 必须拒绝");
        assert_eq!(err.code_string(), "STK-INP-0141");
        assert!(err.title.contains("libass"), "{err}");
        assert!(
            err.details
                .iter()
                .any(|detail| detail.contains(&path.display().to_string())),
            "错误必须指出无效路径：{err}"
        );
    }

    const ASS: &str = "[Script Info]\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, Outline, Shadow, MarginV\r\nStyle: Default,Arial,48,3,2,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(960,540)\\blur4}你好\r\n";
    const SRT: &str = "1\r\n00:00:01,000 --> 00:00:02,500\r\n你好\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n世界\r\n\r\n";

    #[test]
    fn probe_reports_format_and_confidence() {
        let d = dir("probe");
        let p = d.join("x.ass");
        std::fs::write(&p, ASS).unwrap();
        let env = execute(&inv(Command::Probe { input: p.clone() }, None, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert!(env.ok);
        assert_eq!(v["data"]["detection"]["format"], "ass");
        std::fs::remove_file(&p).ok();
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn convert_ass_to_srt_writes_and_reports_loss() {
        let d = dir("a2s");
        let p = d.join("x.ass");
        std::fs::write(&p, ASS).unwrap();
        let out = d.join("x.srt");
        let env = execute(&inv(
            Command::Convert {
                input: p.clone(),
                to: FormatKind::Srt,
                canvas: None,
                script_policy: ScriptResolutionPolicy::Preserve,
                layout_policy: LayoutResPolicy::KeepAsIs,
            },
            Some(out.clone()),
            false,
        ))
        .unwrap();
        assert!(env.ok);
        assert_eq!(env.writes[0].state, "written");
        let srt = std::fs::read_to_string(&out).unwrap();
        assert!(srt.contains("你好"));
        assert!(srt.contains("00:00:01,000 --> 00:00:02,000"));
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&out).ok();
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn convert_ass_rewrite_scales_script() {
        let d = dir("rewrite");
        let p = d.join("x.ass");
        std::fs::write(&p, ASS).unwrap();
        let out = d.join("x.2160.ass");
        let env = execute(&inv(
            Command::Convert {
                input: p.clone(),
                to: FormatKind::Ass,
                canvas: Some(Canvas::UHD2160),
                script_policy: ScriptResolutionPolicy::Rewrite,
                layout_policy: LayoutResPolicy::KeepAsIs,
            },
            Some(out.clone()),
            false,
        ))
        .unwrap();
        let text = std::fs::read_to_string(&out).unwrap();
        assert!(text.contains("PlayResX: 3840"));
        assert!(text.contains("\\pos(1920,1080)"));
        assert!(text.contains("Style: Default,Arial,96,6,4,100"));
        assert!(text.contains("\\blur4"), "blur 不随分辨率缩放（H2）");
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert!(v["data"]["changed"].as_u64().unwrap() > 0);
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&out).ok();
        std::fs::remove_dir(&d).ok();
    }

    /// J10 的端到端半边：UTF-16 输入不再被拒，而且**报告要说清这次按什么读的**。
    /// 只测 `load_text` 不够 —— 真路径是命令走 `load_input`，编码事实要出得了门。
    #[test]
    fn convert_reads_a_utf16le_ass_file_and_says_so() {
        let d = dir("a2s-utf16");
        let p = d.join("x.ass");
        let mut bytes = vec![0xFFu8, 0xFE];
        bytes.extend_from_slice(
            &ASS.encode_utf16()
                .flat_map(|c| c.to_le_bytes())
                .collect::<Vec<u8>>(),
        );
        std::fs::write(&p, &bytes).unwrap();
        let out = d.join("x.srt");
        let env = execute(&inv(
            Command::Convert {
                input: p.clone(),
                to: FormatKind::Srt,
                canvas: None,
                script_policy: ScriptResolutionPolicy::Preserve,
                layout_policy: LayoutResPolicy::KeepAsIs,
            },
            Some(out.clone()),
            false,
        ))
        .unwrap();
        assert!(env.ok, "{:?}", env.errors);
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert_eq!(v["data"]["input_encoding"], "utf-16le-bom", "{v}");
        let srt = std::fs::read_to_string(&out).unwrap();
        assert!(srt.contains("你好"), "解错了就不会有中文：{srt}");
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(&out).ok();
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn time_offset_shifts_both_formats() {
        let d = dir("time");
        let a = d.join("x.ass");
        let s = d.join("y.srt");
        std::fs::write(&a, ASS).unwrap();
        std::fs::write(&s, SRT).unwrap();
        let mk = |input: PathBuf| {
            inv(
                Command::Time {
                    input,
                    mode: transform_time::TimeMode::Offset,
                    source_fps: None,
                    target_fps: None,
                    offset_ms: 1_500,
                    two_point: None,
                    piecewise: transform_time::PiecewiseSpec::new(Vec::new()),
                },
                None,
                true,
            )
        };
        let env = execute(&mk(a.clone())).unwrap();
        assert_eq!(env.writes[0].state, "would-write");
        let env2 = execute(&mk(s.clone())).unwrap();
        assert!(env2.ok);
        for p in [&a, &s] {
            std::fs::remove_file(p).ok();
        }
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn hdr_changes_only_colours_and_is_reversible_by_parse() {
        let d = dir("hdr");
        let src = ASS
            .replace("Style: Default,Arial", "Style: Default,&H00FFFFFF,Arial")
            .replace(
                "Format: Name, Fontname,",
                "Format: Name, PrimaryColour, Fontname,",
            );
        let p = d.join("c.ass");
        std::fs::write(&p, &src).unwrap();
        let settings = core_model::profile::HdrSettings {
            mode: core_model::profile::HdrMode::Compatibility,
            ..core_model::profile::HdrSettings::default()
        };
        let env = execute(&inv(
            Command::Hdr {
                input: p.clone(),
                settings,
            },
            Some(d.join("out.ass")),
            false,
        ))
        .unwrap();
        assert!(env.ok, "{:?}", env.errors);
        std::fs::remove_file(&p).ok();
        std::fs::remove_file(d.join("out.ass")).ok();
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn pgs_validate_and_adjust_round_trip() {
        // 用 encoder 造一个合法 SUP，再走 validate + adjust（时间平移）
        let d = dir("pgs");
        let mut rgba = vec![0u8; 8 * 2 * 4];
        for px in rgba[..32].chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        let obj = codec_pgs::encoder::FrameObject {
            rgba,
            width: 8,
            height: 2,
            x: 100,
            y: 50,
            forced: false,
        };
        let frames = vec![
            codec_pgs::encoder::Frame::new(90_000, vec![obj]),
            codec_pgs::encoder::Frame::erase(180_000),
        ];
        let (segs, _) =
            codec_pgs::encoder::encode(&frames, &codec_pgs::encoder::EncodeOptions::default())
                .unwrap();
        let sup = d.join("t.sup");
        std::fs::write(&sup, write_stream(&segs).unwrap()).unwrap();

        let env = execute(&inv(
            Command::PgsValidate {
                input: sup.clone(),
                level: core_model::profile::ValidationLevel::Standard,
                peak_bps: None,
            },
            None,
            false,
        ))
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert_eq!(v["data"]["canvas"], "1920x1080");
        // Standard 级的报告不能倒推出 Strict 徽章（13 §6）
        assert_eq!(v["data"]["no_blocking_findings"], true);
        assert_eq!(v["data"]["strict_badge_allowed"], false);
        assert!(
            v["data"]["findings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| !s.as_str().unwrap_or("").starts_with("[error]")),
            "{:?}",
            v["data"]["findings"]
        );

        let out = d.join("shifted.sup");
        let env = execute(&inv(
            Command::PgsAdjust {
                input: sup.clone(),
                ops: vec![codec_pgs::fastpath::FastOp::OffsetMs(1_000)],
            },
            Some(out.clone()),
            false,
        ))
        .unwrap();
        assert!(env.ok);
        let moved = std::fs::read(&out).unwrap();
        let after = codec_pgs::read_file(&moved).unwrap();
        assert_eq!(after.segments[0].pts, 90_000 + 90_000);
        assert_eq!(
            after.segments[0].dts,
            90_000 - 3_000 + 90_000,
            "DTS 一起平移，保持 decode lead"
        );
        // F4（13 §3.1）：报告交出的是**可存档的哈希**，不是一个永远为真的布尔值。
        // 空集合不折摘要，所以这里 objects>0 时 aggregate_sha256 必须是 64 位十六进制。
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        let ods = &v["data"]["ods_invariant"];
        let objects = ods["objects"].as_u64().unwrap();
        assert!(
            objects > 0,
            "这份 SUP 里没有 ODS，那条不变量就成了空谈：{v}"
        );
        let agg = ods["aggregate_sha256"].as_str().unwrap_or_default();
        assert_eq!(agg.len(), 64, "要是一个 SHA-256：{ods}");
        assert!(agg.chars().all(|c| c.is_ascii_hexdigit()), "{agg}");
        assert_eq!(
            ods["per_object"].as_array().unwrap().len(),
            objects as usize,
            "逐个哈希的条数必须与 objects 一致"
        );

        // 换一个平移量重跑：时间轴动了，但 ODS 字节的摘要必须还是同一个值
        let out2 = d.join("shifted2.sup");
        let env2 = execute(&inv(
            Command::PgsAdjust {
                input: sup.clone(),
                ops: vec![codec_pgs::fastpath::FastOp::OffsetMs(21_000)],
            },
            Some(out2.clone()),
            false,
        ))
        .unwrap();
        assert!(env2.ok);
        let v2: serde_json::Value = serde_json::from_str(&env2.to_json_line()).unwrap();
        assert_eq!(
            v2["data"]["ods_invariant"]["aggregate_sha256"]
                .as_str()
                .unwrap_or_default(),
            agg,
            "同一份输入的 ODS 摘要随平移量变了 —— 那不是无损路径"
        );
        assert_ne!(
            codec_pgs::read_file(&std::fs::read(&out2).unwrap())
                .unwrap()
                .segments[0]
                .pts,
            after.segments[0].pts,
            "两次平移的 PTS 必须真的不同，否则上面那句相等是白比"
        );
        std::fs::remove_file(&out2).ok();
        std::fs::remove_file(&sup).ok();
        std::fs::remove_file(&out).ok();
        std::fs::remove_dir(&d).ok();
    }

    #[test]
    fn not_built_commands_fail_loudly() {
        let e = execute(&inv(
            Command::NotBuilt {
                name: "render".to_string(),
                phase: "Phase 5".to_string(),
            },
            None,
            false,
        ))
        .unwrap_err();
        assert_eq!(e.code_string(), "STK-INP-0060");
    }

    #[test]
    fn doctor_runs_without_external_tools() {
        let env = execute(&inv(Command::Doctor, None, false)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&env.to_json_line()).unwrap();
        assert_eq!(v["data"]["os"], std::env::consts::OS);
        assert!(v["data"]["not_built"].is_array());
    }
}
