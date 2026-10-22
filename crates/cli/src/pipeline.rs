//! 输入/输出与跨 crate 流水线胶水（14_CONVERSION_PIPELINES）。
//!
//! GUI 与 CLI 必须共用同一条流水线（36 §5），所以这里只放无 UI 依赖的部分：
//! 读文本、编码判定、ASS↔SRT、ASS 时间/HDR/分辨率改写、原子写出。

use std::io::Read;
use std::path::{Path, PathBuf};

use codec_ass::{parse_overrides, AssDocument, EntryKind, OverrideToken, SectionKind};
use codec_srt::{Newline, TextCue, TextSubtitle};
use core_model::error::{Domain, Result, StkError};
use core_model::geometry::{Geometry, LayoutResPolicy, ScriptResolutionPolicy};
use core_model::loss::LossReport;
use core_model::time::Fps;
use core_model::Canvas;
use task_engine::{AtomicOutput, OverwritePolicy};
use transform_time::{
    apply_full as apply_time, Cue, PiecewiseSpec, TimeConfig, TimeMode, TwoPointSpec,
};

/// CLI 层的错误码从 110 起，避开各 crate 自己的编号空间（22 §错误模型）。
fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

#[derive(Clone, Debug)]
pub struct Loaded {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub text: String,
    /// 这次实际按什么读进来的。"猜了但不说"和"含糊结论"是同一件事，所以它是数据不是注释。
    pub encoding: &'static str,
}

/// UTF-16 判定的三种结局。BOM 是**声明**，没 BOM 时只能靠结构判定，判不动就承认判不动。
enum Utf16 {
    Not,
    /// (解出的文本, 编码名)。带 BOM 时文本以 U+FEFF 开头 —— BOM 留给 codec 记账（AT-ASS-001、J9）。
    Ok(String, &'static str),
    /// 有 BOM 声明却解不开：(原因, 编码名)。这是"声明了 UTF-16 而内容不是"，不是"没这条能力"。
    Bad(String, &'static str),
}

/// 无 BOM 的 UTF-16 只能这样判：正文里每隔一字节是 0x00。
/// 返回 `Some(true)` = 小端、`Some(false)` = 大端、`None` = 判不出来。
///
/// **这套判据的边界要写清楚**：只对 ASCII 为主的正文有效。全 CJK 的 UTF-16 两种字节都不为 0，
/// 与一段任意二进制在这条规则下不可区分 —— 那种文件必须带 BOM，否则我们承认读不了，
/// 而不是猜一个编码把中文解成乱码（J10 剩下的那半就是这个道理）。
fn utf16_shape_without_bom(b: &[u8]) -> Option<bool> {
    if b.len() < 8 || b.len() % 2 != 0 {
        return None;
    }
    let n = b.len().min(64) / 2;
    let mut le = 0;
    let mut be = 0;
    for i in 0..n {
        if b[2 * i + 1] == 0 {
            le += 1;
        }
        if b[2 * i] == 0 {
            be += 1;
        }
    }
    if le == n {
        Some(true)
    } else if be == n {
        Some(false)
    } else {
        None
    }
}

fn decode_units(raw: &[u8], little: bool) -> std::result::Result<String, String> {
    let units: Vec<u16> = if little {
        raw.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    } else {
        raw.chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect()
    };
    String::from_utf16(&units).map_err(|e| format!("UTF-16 码元不合法：{e}"))
}

fn detect_utf16(bytes: &[u8]) -> Utf16 {
    let little = if bytes.starts_with(&[0xFF, 0xFE]) {
        Some(true)
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        Some(false)
    } else {
        utf16_shape_without_bom(bytes)
    };
    let Some(little) = little else {
        return Utf16::Not;
    };
    let declared = bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
    let raw = if declared { &bytes[2..] } else { bytes };
    let name = match (little, declared) {
        (true, true) => "utf-16le-bom",
        (false, true) => "utf-16be-bom",
        (true, false) => "utf-16le",
        (false, false) => "utf-16be",
    };
    if !declared && raw.len() % 2 != 0 {
        return Utf16::Bad(
            "没有 BOM 且字节数是奇数，与 UTF-16 不吻合".to_string(),
            name,
        );
    }
    match decode_units(raw, little) {
        // BOM 是正文之外的东西，但 codec 要拿它记账、写回时要还原，所以解出来以后原样放回开头
        Ok(s) => {
            let text = if declared { format!("\u{feff}{s}") } else { s };
            Utf16::Ok(text, name)
        }
        Err(why) => Utf16::Bad(why, name),
    }
}

/// 字幕文本的输入上限：1 GiB。
///
/// 依据（23 §文件“大文件…不相关格式拒绝”）：本工具处理的是**字幕**，
/// 不是视频。一部 3 小时的 5000 行 ASS 不到 5 MiB，带大量特效与 `[Fonts]`
/// 内嵌字体的也就几百 MiB。1 GiB 比真字幕高两个数量级，而它拦住的
/// 正是“把 20 GB 的 ts 误当成字幕摍进来”这类误操作。
///
/// 为什么必须**在读取之前**先查字节数：`fs::read` 会先把整份读进内存，
/// 而 `String::from_utf8` 还会再拷一份。一个不该处理的文件
/// 应当以一条“太大了”的消息被拒，而不是把机器的内存吃完再失败
/// —— 后者在 32 位环境或内存本来就紧张时有真实的伤害。
pub const MAX_TEXT_BYTES: u64 = 1024 * 1024 * 1024;

/// 二进制字幕（PGS/SUP）的输入上限：4 GiB。
///
/// 比文本大一档：真实商业 SUP 可以很大（本机那份 1080p 双语特效 SUP 是 98.97 MB，
/// 4K 长片版的 SUP 到 GB 级不奇怪）。4 GiB 仍然挡住了“把几百 GB 的镜像摍进来”这种。
///
/// 不能只看段级别上限（`codec-pgs` 已对每个段的长度做了 checked 校验）：
/// 那些上限防的是“单段声明自己不存在的长度”，而这里是“整个文件就不该被读”。
pub const MAX_BINARY_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// 按上限读文件。**先 stat 再读**，不把超限的文件先读进内存。
///
/// 错误码 113 单独立一个，而不是并进 110（无法读取）：
/// “文件太大”与“文件读不了”的用户动作完全不同 —— 前者要换个输入，
/// 后者要修权限或路径。
pub(crate) fn read_bounded(path: &Path, limit: u64, what: &str) -> Result<Vec<u8>> {
    let md = std::fs::metadata(path)
        .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{:?}：{e}", path)))?;
    if md.len() > limit {
        return Err(cli_err(113, &format!("{what}输入超过本工具的处理上限"))
            .with_detail(format!(
                "{}：{:} 字节，上限 {} 字节",
                path.display(),
                md.len(),
                limit
            ))
            .with_action("确认选对了文件；本工具处理字幕，不处理视频/音频容器"));
    }
    // metadata 只是快速拒绝的第一道检查；文件可能在 stat 后增长。最多读取 limit+1，
    // 确保并发替换/追加不会绕过上限并把超大文件整份装入内存。
    let file = std::fs::File::open(path)
        .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{:?}：{e}", path)))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{:?}：{e}", path)))?;
    if bytes.len() as u64 > limit {
        return Err(cli_err(113, &format!("{what}输入超过本工具的处理上限"))
            .with_detail(format!(
                "{}：至少 {} 字节，上限 {} 字节",
                path.display(),
                bytes.len(),
                limit
            ))
            .with_action("确认选对了文件；本工具处理字幕，不处理视频/音频容器"));
    }
    Ok(bytes)
}

/// 读入并解码。UTF-8（含 BOM）与 UTF-16（BOM 声明，或无 BOM 但结构可判）都接受；
/// Windows ANSI/CP936 这类**没有声明又无法判定**的编码仍然明确拒绝（J10 剩余那半）。
pub fn load_text(path: &Path) -> Result<Loaded> {
    let bytes = read_bounded(path, MAX_TEXT_BYTES, "字幕文本")?;
    let hint = format_detect::probe_bytes(&bytes, None).encoding_hint;
    match detect_utf16(&bytes) {
        Utf16::Ok(text, encoding) => Ok(Loaded {
            path: path.to_path_buf(),
            bytes,
            text,
            encoding,
        }),
        Utf16::Bad(why, encoding) => Err(cli_err(111, "看着是 UTF-16，但解不开")
            .with_detail(why)
            .with_detail(format!("判定编码：{encoding}"))
            .with_detail(format!("其他编码线索：{hint:?}"))
            .with_action("另存为 UTF-8，或至少保留 UTF-16 的 BOM")),
        Utf16::Not => {
            // UTF-8 BOM 不剥离：codec-ass/codec-srt 自己记账并在写回时还原（AT-ASS-001、J9）
            let encoding = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                "utf-8-bom"
            } else {
                "utf-8"
            };
            let text = String::from_utf8(bytes.clone()).map_err(|e| {
                cli_err(112, "输入既不是 UTF-8（含 BOM）也不是可判定的 UTF-16")
                    .with_detail(format!("编码线索：{hint:?}"))
                    .with_detail(e.utf8_error().to_string())
                    .with_action(
                        "Windows 记事本存的 UTF-16 请保留 BOM；ANSI/GBK 这类无声明编码本层不接受，\
                         请先转成 UTF-8（CLI 不猜代码页，猜错会把中文解成看起来正常的乱码）",
                    )
            })?;
            Ok(Loaded {
                path: path.to_path_buf(),
                bytes,
                text,
                encoding,
            })
        }
    }
}

/// 23 §文件 的“极端 cue 数”检查：ASS/SSA 侧的入口。
///
/// `codec_ass::AssDocument::parse` 是不可失败的（它必须尽量多地把字节保下来，
/// 这是 AT-ASS-001 的前提）。所以条目数上限只能在**解析之后**查，
/// 而这个检查必须每个生产调用点都过——所以做成一个函数而不是散在各处的 `if`。
///
/// 为什么在 ASS 侧只查 Dialogue 而不查全部 Entry：`[Script Info]`、样式、
/// 特效字体等条目数不会因恶意输入而爆炸（它们受字段数与行长度约束）。
/// Dialogue 才是“每个数据都要建一个 stage”的那一类。
pub fn check_ass_dialogue_count(doc: &AssDocument) -> Result<()> {
    let n = doc.dialogue_count();
    if let Some(limit) = core_model::limits::cue_count_exceeded(n) {
        return Err(cli_err(114, "事件条数超过本工具的处理上限")
            .with_detail(format!(
                "Dialogue 有 {} 条，上限 {} 条",
                n,
                core_model::limits::limit_label(limit)
            ))
            .with_action("确认选对了文件；逐词时间轴这类超密字幕需要先切分"));
    }
    Ok(())
}

/// CLI 层的写出结果（`task_engine::Commit` 只有 Skipped/Written，dry-run 语义属于 CLI 层）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Emitted {
    Skipped { path: PathBuf },
    WouldWrite { path: PathBuf, bytes: u64 },
    Written { path: PathBuf, bytes: u64 },
}

impl Emitted {
    pub fn describe(&self) -> WriteNote {
        match self {
            Emitted::Skipped { path } => WriteNote {
                path: path.display().to_string(),
                bytes: 0,
                state: "skipped".to_string(),
            },
            Emitted::WouldWrite { path, bytes } => WriteNote {
                path: path.display().to_string(),
                bytes: *bytes,
                state: "would-write".to_string(),
            },
            Emitted::Written { path, bytes } => WriteNote {
                path: path.display().to_string(),
                bytes: *bytes,
                state: "written".to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteNote {
    pub path: String,
    pub bytes: u64,
    pub state: String,
}

/// 原子写出（15/23：tmp + fsync + 回读校验 + rename），dry-run 只报告不落盘。
pub fn emit(
    out: Option<&Path>,
    default_path: &Path,
    bytes: &[u8],
    verify_reparse: impl FnOnce(&[u8]) -> Result<()>,
    dry_run: bool,
    overwrite: OverwritePolicy,
) -> Result<Emitted> {
    let target = out
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| default_path.to_path_buf());
    let len = bytes.len() as u64;
    if dry_run {
        // dry-run 也要跑校验：否则"能 dry 不能写"的任务会一路绿到真正执行才失败
        verify_reparse(bytes)?;
        return Ok(Emitted::WouldWrite {
            path: target,
            bytes: len,
        });
    }
    let ao = AtomicOutput::new(&target, overwrite)?;
    match ao.commit(bytes, verify_reparse)? {
        task_engine::Commit::Skipped => Ok(Emitted::Skipped { path: target }),
        task_engine::Commit::Written(p) => Ok(Emitted::Written {
            path: p,
            bytes: len,
        }),
    }
}

/// ASS 的 Text → 纯文本（SRT/VTT 用）。丢弃的内联标记必须进 loss 报告。
pub fn ass_text_to_plain(text: &str, loss: &mut LossReport) -> String {
    let mut out = String::new();
    for token in parse_overrides(text) {
        match token {
            OverrideToken::Text { raw } => out.push_str(&raw),
            OverrideToken::Drawing { .. } => {
                loss.bump("dropped", "矢量 drawing 在纯文本格式里没有对应物");
            }
            OverrideToken::Block(block) => {
                for tag in &block.tags {
                    match tag.kind {
                        codec_ass::TagKind::Karaoke => {
                            loss.bump("dropped", "卡拉OK 计时标记");
                        }
                        codec_ass::TagKind::Position
                        | codec_ass::TagKind::Movement
                        | codec_ass::TagKind::Origin
                        | codec_ass::TagKind::Clip { .. }
                        | codec_ass::TagKind::Border
                        | codec_ass::TagKind::Shadow
                        | codec_ass::TagKind::Scale
                        | codec_ass::TagKind::Rotation => {
                            loss.bump("dropped", "定位/特效 tag");
                        }
                        codec_ass::TagKind::Colour { .. } | codec_ass::TagKind::Alpha { .. } => {
                            loss.bump("dropped", "颜色/透明度 tag");
                        }
                        codec_ass::TagKind::Unknown => {
                            loss.nonstandard(&format!("未识别 tag 被丢弃：{}", tag.name));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    // ASS 的换行与硬空格
    let mut cleaned = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            cleaned.push(c);
            continue;
        }
        match chars.peek() {
            Some('N') | Some('n') => {
                chars.next();
                cleaned.push('\n');
            }
            Some('h') => {
                chars.next();
                cleaned.push(' ');
            }
            Some('[') => {
                // \[ 是字面方括号
                chars.next();
                cleaned.push('[');
            }
            _ => cleaned.push(c),
        }
    }
    cleaned
}

/// ASS/SSA → SRT。样式/特效信息一律进 loss 报告，不做静默丢弃（08 §4）。
pub fn ass_to_srt(doc: &AssDocument) -> Result<(TextSubtitle, LossReport)> {
    let mut loss = LossReport::new();
    let mut cues: Vec<TextCue> = Vec::new();
    let mut comment_count = 0usize;
    for section in &doc.sections {
        if section.kind != SectionKind::Events {
            continue;
        }
        for (ei, e) in section.entries.iter().enumerate() {
            if e.kind == EntryKind::Comment {
                comment_count += 1;
                continue;
            }
            if e.kind != EntryKind::Dialogue {
                continue;
            }
            let view = match section.dialogue_view(ei) {
                Some(v) => v,
                None => continue,
            };
            let start = view.start()?;
            let end = view.end()?;
            let plain = ass_text_to_plain(view.text(), &mut loss);
            if plain.trim().is_empty() {
                loss.bump("dropped", "只有特效没有可见文本的事件");
                continue;
            }
            cues.push(TextCue {
                index: Some(cues.len() as u32 + 1),
                start,
                end,
                text: plain,
            });
        }
    }
    if comment_count > 0 {
        loss.dropped(&format!("{comment_count} 条 Comment 行"));
    }
    if doc.styles().len() > 1 {
        loss.user_attention("源脚本有多个样式；SRT 只能呈现一种默认外观");
    }
    loss.preserved("事件顺序与文本内容");
    Ok((
        TextSubtitle {
            cues,
            bom: doc.bom,
            newline: if doc.newline == "\r\n" {
                Newline::CrLf
            } else {
                Newline::Lf
            },
            encoding: codec_srt::DetectedEncoding::Utf8,
            unrecognized: Vec::new(),
        },
        loss,
    ))
}

/// SRT → ASS。生成的是合规的 ASS v4.00+ 骨架（08 §1）；画布由调用方给（默认 1920x1080）。
pub fn srt_to_ass(
    sub: &TextSubtitle,
    canvas: Canvas,
    title: &str,
    loss: &mut LossReport,
) -> String {
    // ASS 的时间精度是百分秒；SRT 的毫秒若不在 10ms 网格上就必须上报舍入（08 §4）
    let coarse = sub.cues.iter().any(|c| {
        [c.start, c.end]
            .iter()
            .any(|t| t.to_pts_ticks().unwrap_or(0) % 900 != 0)
    });
    if coarse {
        loss.approximated("SRT 毫秒精度写入 ASS 时舍入到百分秒（1 百分秒 = 900 tick）");
    }
    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str(&format!("Title: {title}\n"));
    out.push_str("ScriptType: v4.00+\n");
    out.push_str(&format!("PlayResX: {}\n", canvas.width));
    out.push_str(&format!("PlayResY: {}\n", canvas.height));
    out.push_str("WrapStyle: 0\n");
    out.push_str("ScaledBorderAndShadow: yes\n");
    out.push_str("\n[V4+ Styles]\n");
    out.push_str("Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n");
    out.push_str("Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,50,50,50,1\n");
    out.push_str("\n[Events]\n");
    out.push_str(
        "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for cue in &sub.cues {
        let start = cue
            .start
            .to_ass_string()
            .unwrap_or_else(|_| "0:00:00.00".to_string());
        let end = cue
            .end
            .to_ass_string()
            .unwrap_or_else(|_| "0:00:00.00".to_string());
        let text = cue.text.replace('\n', "\\N");
        out.push_str(&format!(
            "Dialogue: 0,{start},{end},Default,,0,0,0,,{text}\n"
        ));
    }
    out
}

/// 对 SRT 或 ASS 施加同一种时间变换（10 §1 的三件事之一）。
/// ASS 的写回只改 Start/End 两个字段，其余字节原样（AT-ASS-001）。
pub fn time_srt(
    sub: &TextSubtitle,
    cfg: &TimeConfig,
    spec: Option<&TwoPointSpec>,
    piecewise: &PiecewiseSpec,
) -> Result<(TextSubtitle, LossReport)> {
    let cues: Vec<Cue> = sub.cues.iter().map(|c| (c.start, c.end)).collect();
    let (moved, loss) = apply_time(&cues, cfg, spec, Some(piecewise))?;
    if moved.len() != sub.cues.len() {
        return Err(cli_err(113, "时间变换返回的 cue 数与输入不符"));
    }
    let mut out = sub.clone();
    for (cue, (start, end)) in out.cues.iter_mut().zip(moved) {
        cue.start = start;
        cue.end = end;
    }
    Ok((out, loss))
}

pub fn time_ass(
    doc: &mut AssDocument,
    cfg: &TimeConfig,
    spec: Option<&TwoPointSpec>,
    piecewise: &PiecewiseSpec,
) -> Result<(usize, LossReport)> {
    let positions = doc.sorted_dialogue_positions();
    let mut cues: Vec<Cue> = Vec::with_capacity(positions.len());
    for (si, ei, _) in &positions {
        let view = doc
            .section(*si)
            .and_then(|s| s.dialogue_view(*ei))
            .ok_or_else(|| cli_err(114, "事件条目消失"))?;
        cues.push((view.start()?, view.end()?));
    }
    let (moved, loss) = apply_time(&cues, cfg, spec, Some(piecewise))?;
    for ((si, ei, _), (start, end)) in positions.iter().zip(moved) {
        let s = doc
            .section_mut(*si)
            .ok_or_else(|| cli_err(114, "Events 段消失"))?;
        s.set_field(*ei, "Start", &start.to_ass_string()?)?;
        s.set_field(*ei, "End", &end.to_ass_string()?)?;
    }
    Ok((positions.len(), loss))
}

/// 从 FPS 文本构造 TimeConfig。
pub fn time_config(
    mode: TimeMode,
    source_fps: Option<Fps>,
    target_fps: Option<Fps>,
    offset_ms: i64,
) -> TimeConfig {
    TimeConfig {
        mode,
        source_fps,
        target_fps,
        offset_ms,
    }
}

/// 组装 Rewrite/Preserve 的几何：脚本尺寸来自文件本身（09 §4 B 的"以文件 PlayRes 为准"）。
pub fn geometry_for(
    doc: &AssDocument,
    canvas: Canvas,
    script_policy: ScriptResolutionPolicy,
    layout_policy: LayoutResPolicy,
) -> Result<Geometry> {
    let script = match doc.play_res() {
        Some((x, y)) => {
            let w = u32::try_from(x).map_err(|_| cli_err(115, "PlayResX 超出可用范围"))?;
            let h = u32::try_from(y).map_err(|_| cli_err(115, "PlayResY 超出可用范围"))?;
            Some(core_model::geometry::ScriptSize { x: w, y: h })
        }
        None => None,
    };
    let layout = match (doc.script_info("LayoutResX"), doc.script_info("LayoutResY")) {
        (Some(x), Some(y)) => match (x.parse::<u32>(), y.parse::<u32>()) {
            (Ok(x), Ok(y)) => Some(core_model::geometry::LayoutRes { x, y }),
            _ => None,
        },
        _ => None,
    };
    // 没有显式视频信息时，按输出画布假定同尺寸方形像素存储；渲染层必须再核对（40 §2.1）
    let storage = Some(core_model::geometry::StorageSize::new(
        canvas.width,
        canvas.height,
    )?);
    let g = Geometry {
        canvas,
        canvas_is_custom: !canvas.is_16_9(),
        script,
        layout,
        storage,
        par: core_model::geometry::PixelAspect::square(),
        script_policy,
        layout_policy,
    };
    g.validate()?;
    Ok(g)
}

/// 文件/字节内容的 SHA-256 十六进制。CLI 里凡是要"记录一份字节"的地方都用这一个实现
/// （字体交付、批量缓存 key、注入前核对）——之前有三份各写各的，改动时最容易漏掉一处。
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 测试用的窄接口：直接验边界（`>` vs `>=`）而不跑整个命令。
///
/// 存在的理由与 `read_bounded` 的注释同一条：集成用例已经证明了
/// “超限会被真 CLI 拒”，所以边界那一个比较运算符可以在这一层用小文件钉住，
/// 不必为了测它去读 1 GiB。暴露给测试而不是把它变成 `pub` API。
#[doc(hidden)]
pub fn pipeline_probe_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    read_bounded(path, limit, "测试")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::time::{Rational, Time};

    const ASS: &str = "[Script Info]\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize\r\nStyle: Default,Arial,48\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,{\\pos(10,20)\\blur4}第一行\\N第二行\r\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,{\\k50}唱\r\nComment: 0,0:00:07.00,0:00:08.00,Default,,0,0,0,,注释\r\n";

    fn doc() -> AssDocument {
        AssDocument::parse(ASS)
    }

    #[test]
    fn ass_to_srt_strips_tags_and_reports_loss() {
        let (sub, loss) = ass_to_srt(&doc()).unwrap();
        assert_eq!(sub.cues.len(), 2);
        assert_eq!(sub.cues[0].text, "第一行\n第二行");
        assert_eq!(sub.cues[1].text, "唱");
        assert_eq!(
            sub.cues[0].end,
            Time::from_hms_millis(0, 0, 3, 500).unwrap()
        );
        assert!(loss.count_for("dropped", "卡拉OK 计时标记") >= 1);
        assert!(loss.count_for("dropped", "定位/特效 tag") >= 1);
        assert_eq!(loss.count_for("dropped", "1 条 Comment 行"), 1);
    }

    #[test]
    fn srt_to_ass_roundtrips_back_to_srt_text() {
        let (sub, _) = ass_to_srt(&doc()).unwrap();
        let ass = srt_to_ass(&sub, Canvas::FHD1080, "x.ass", &mut LossReport::new());
        let again = AssDocument::parse(&ass);
        let (sub2, _) = ass_to_srt(&again).unwrap();
        assert_eq!(sub2.cues.len(), sub.cues.len());
        assert_eq!(sub2.cues[0].text, sub.cues[0].text);
        assert_eq!(sub2.cues[0].start, sub.cues[0].start);
        assert!(ass.contains("PlayResX: 1920"));
        assert!(ass.contains("第一行\\N第二行"));
    }

    #[test]
    fn time_on_ass_only_touches_start_end_fields() {
        let mut d = doc();
        let before = d.render();
        let cfg = TimeConfig::offset(1_000);
        let (n, _loss) = time_ass(&mut d, &cfg, None, &PiecewiseSpec::new(Vec::new())).unwrap();
        assert_eq!(n, 2);
        let after = d.render();
        assert_ne!(after, before);
        // 逐行对比：只有 Dialogue 的时间字段变了，Comment/样式/附件行必须一致
        let b: Vec<&str> = before.lines().collect();
        let a: Vec<&str> = after.lines().collect();
        assert_eq!(b.len(), a.len());
        for (x, y) in b.iter().zip(a.iter()) {
            if x.starts_with("Comment:") || x.starts_with("Style:") || x.starts_with("Format:") {
                assert_eq!(x, y, "不该改的行被改了：{x}");
            }
        }
        assert!(after.contains("Dialogue: 0,0:00:02.00,0:00:04.50"));
        assert!(after.contains("Comment: 0,0:00:07.00"));
        assert!(after.contains("{\\pos(10,20)\\blur4}"));
    }

    #[test]
    fn geometry_comes_from_the_file_not_from_a_guess() {
        let g = geometry_for(
            &doc(),
            Canvas::UHD2160,
            ScriptResolutionPolicy::Rewrite,
            LayoutResPolicy::KeepAsIs,
        )
        .unwrap();
        assert_eq!(
            g.script.unwrap(),
            core_model::geometry::ScriptSize { x: 1920, y: 1080 }
        );
        assert_eq!(g.script_scale_factor().unwrap(), Rational::from_int(2));
        assert!(g.ready_for_render().is_ok());
    }

    #[test]
    fn geometry_marks_custom_canvas_explicitly() {
        let g = geometry_for(
            &doc(),
            Canvas::new(1920, 800).unwrap(),
            ScriptResolutionPolicy::Preserve,
            LayoutResPolicy::KeepAsIs,
        )
        .unwrap();
        assert!(g.canvas_is_custom, "非 16:9 必须自动打上自定义标记");
    }

    fn utf16_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("stk-cli-io-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    /// J10 的这一半：UTF-16 不再一律拒绝。BOM 是声明，按声明解；
    /// 而且 BOM 解出来后要原样放回正文开头，codec 才能继续按 AT-ASS-001 记账。
    #[test]
    fn utf16_with_bom_is_decoded_and_the_bom_stays_accountable() {
        for (bom, le) in [(vec![0xFFu8, 0xFE], true), (vec![0xFEu8, 0xFF], false)] {
            let body: Vec<u8> = if le {
                "[1]\n"
                    .encode_utf16()
                    .flat_map(|c| c.to_le_bytes())
                    .collect()
            } else {
                "[1]\n"
                    .encode_utf16()
                    .flat_map(|c| c.to_be_bytes())
                    .collect()
            };
            let mut bytes = bom.clone();
            bytes.extend_from_slice(&body);
            let p = utf16_file("bom.srt", &bytes);
            let loaded = load_text(&p).unwrap();
            assert_eq!(loaded.text, "\u{feff}[1]\n", "{bytes:?}");
            assert!(
                loaded.encoding.ends_with("-bom") && loaded.encoding.starts_with("utf-16"),
                "报告得说清按什么读的：{}",
                loaded.encoding
            );
            // 原始字节仍然带着 BOM 事实：写回侧要能还原它
            assert_eq!(loaded.bytes, bytes);
            std::fs::remove_file(&p).ok();
        }
    }

    /// 没有 BOM 时只接受**结构判得动**的那一种（每隔一字节为 0）。这是判据的边界，
    /// 不是"还没做完"：全 CJK 的无 BOM UTF-16 与任意二进制在这里不可区分，猜就一定会猜错。
    #[test]
    fn utf16_without_bom_is_accepted_only_when_the_shape_proves_it() {
        let le: Vec<u8> = "1\nA\n"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let p = utf16_file("shape.srt", &le);
        let loaded = load_text(&p).unwrap();
        assert_eq!(loaded.text, "1\nA\n");
        assert_eq!(loaded.encoding, "utf-16le", "无 BOM 也要说清按哪头解的");
        std::fs::remove_file(&p).ok();

        // 全 CJK 的无 BOM UTF-16：两种字节都不为 0，判不动 → 明确失败并给可执行出路
        let cjk: Vec<u8> = "你好世界测试内容"
            .encode_utf16()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        // 这条断言是**测试自己的前提**：那段字节必须既"UTF-16 结构判不动"又"不是合法 UTF-8"，
        // 否则失败的根本不是同一种东西，换个内容就是换一道题（不写出来的话，将来只会看到红色却不知道为何）。
        assert!(
            std::str::from_utf8(&cjk).is_err(),
            "前提没了：那段字节现在能被当成 UTF-8，cjk 内容要换一个"
        );
        let p = utf16_file("cjk.srt", &cjk);
        let e = load_text(&p).unwrap_err();
        assert_eq!(e.code_string(), "STK-INP-0112");
        // 出路属于 user_action（22 §错误模型），不是 details：details 放"查到的事实"
        assert!(
            e.user_action.as_deref().unwrap_or_default().contains("BOM"),
            "出路要写在报错里：{:?}",
            e
        );
        std::fs::remove_file(&p).ok();
    }

    /// "声明是 UTF-16 而内容解不开"与"没这个能力"是两种结论，不能共用一句话。
    #[test]
    fn a_bom_that_lies_about_its_payload_is_named() {
        // UTF-16LE BOM + 落单的代理项（前导代理没有跟随的低代理）
        let mut bytes = vec![0xFFu8, 0xFE];
        bytes.extend_from_slice(&[0x00, 0xD8, 0x41, 0x00]);
        let p = utf16_file("lie.srt", &bytes);
        let e = load_text(&p).unwrap_err();
        assert_eq!(e.code_string(), "STK-INP-0111");
        assert!(
            e.details.iter().any(|d| d.contains("utf-16le-bom")),
            "{:?}",
            e.details
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn utf8_still_loads_and_says_so() {
        let p = utf16_file("plain.srt", b"1\n00:00:01,000 --> 00:00:02,000\nhi\n");
        let loaded = load_text(&p).unwrap();
        assert!(loaded.text.contains("hi"));
        assert_eq!(loaded.encoding, "utf-8");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn utf8_bom_is_left_for_the_codec_to_account() {
        let dir = std::env::temp_dir().join(format!("stk-cli-bom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.ass");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(ASS.as_bytes());
        std::fs::write(&p, &bytes).unwrap();
        let loaded = load_text(&p).unwrap();
        assert!(
            loaded.text.starts_with('\u{feff}'),
            "BOM 必须原样留给 codec-ass 记账，否则 round-trip 会丢字节"
        );
        let doc = codec_ass::AssDocument::parse(&loaded.text);
        assert!(doc.bom);
        assert!(
            loaded.bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            "原始字节要保留 BOM 事实"
        );
        std::fs::remove_file(&p).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn emit_dry_run_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("stk-cli-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("out.srt");
        let commit = emit(
            Some(&target),
            &target,
            b"1\n00:00:01,000 --> 00:00:02,000\nhi\n",
            |_| Ok(()),
            true,
            OverwritePolicy::Skip,
        )
        .unwrap();
        assert_eq!(
            commit,
            Emitted::WouldWrite {
                path: target.clone(),
                bytes: 35
            }
        );
        assert_eq!(commit.describe().state, "would-write");
        assert!(!target.exists());
        std::fs::remove_dir(&dir).ok();
    }
}
