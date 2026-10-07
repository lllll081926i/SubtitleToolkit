//! 格式探测（08 §3）：扩展名只是 hint，判定依据必须是内容签名。

use std::io::Read;
use std::path::Path;

use core_model::error::{Domain, Result, StkError};

/// 签名都在文件头部；限制读取量，避免拖拽大批量文件时把整个文件读进内存（23 §文件）。
const PROBE_WINDOW: u64 = 64 * 1024;

/// PGS 段头 13 字节：`PG`(2) + PTS(4) + DTS(4) + segment_type(1) + segment_length(2)，
/// segment_length 只覆盖第 13 字节之后的 payload。
const SUP_PREFIX: usize = 13;
/// 再往后必须还能看到段类型(1) + stream_id(2)，否则无法确认结构自洽。
const SUP_MIN_HEADER: usize = 13;

/// HDMV PGS 核心段类型（libbluray hdmv_pgs / SUPer 口径）：PCS/WDS/PDS/ODS/END。
const SUP_CORE_TYPES: [u8; 5] = [0x14, 0x15, 0x16, 0x17, 0x19];
/// 13 §2 要求未知/扩展段不得破坏流，所以厂商段也放行，只用于"结构自洽"判定。
const SUP_TYPES: [u8; 9] = [0x14, 0x15, 0x16, 0x17, 0x19, 0x80, 0x81, 0x03, 0xff];

/// Matroska/WebM 容器：字幕在轨内，单个文件级别探测给不出轨道格式。
const EBML_MAGIC: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubtitleFormat {
    Ass,
    Ssa,
    Srt,
    #[serde(rename = "webvtt")]
    WebVtt,
    Pgs,
    #[serde(rename = "bdn-xml")]
    BdnXml,
    #[serde(rename = "vobsub-idx")]
    VobSubIdx,
    Unknown,
}

impl SubtitleFormat {
    fn from_ext(ext: &str) -> Option<SubtitleFormat> {
        Some(match ext {
            "ass" => SubtitleFormat::Ass,
            "ssa" => SubtitleFormat::Ssa,
            "srt" => SubtitleFormat::Srt,
            "vtt" => SubtitleFormat::WebVtt,
            "sup" | "pgs" => SubtitleFormat::Pgs,
            "xml" => SubtitleFormat::BdnXml,
            "idx" | "sub" => SubtitleFormat::VobSubIdx,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            SubtitleFormat::Ass => "ASS",
            SubtitleFormat::Ssa => "SSA",
            SubtitleFormat::Srt => "SRT",
            SubtitleFormat::WebVtt => "WebVTT",
            SubtitleFormat::Pgs => "PGS/SUP",
            SubtitleFormat::BdnXml => "BDN XML",
            SubtitleFormat::VobSubIdx => "VobSub IDX",
            SubtitleFormat::Unknown => "未知格式",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum EncodingHint {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    AnsiLikely,
    Binary,
}

/// `reasons` 面向 UI 的"建议来源"展示，必须是人能读懂的一句话。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Detection {
    pub format: SubtitleFormat,
    pub confidence: Confidence,
    pub encoding_hint: EncodingHint,
    pub reasons: Vec<String>,
}

pub fn probe_bytes(bytes: &[u8], filename_hint: Option<&str>) -> Detection {
    let ext = filename_hint.and_then(extension_of);
    let hint = ext.as_deref().and_then(SubtitleFormat::from_ext);
    let mut why: Vec<String> = Vec::new();

    let (format, confidence, from_content, encoding_hint) = if bytes.is_empty() {
        why.push("输入为空，没有任何可用于判定的字节".to_string());
        (
            SubtitleFormat::Unknown,
            Confidence::Low,
            false,
            EncodingHint::Utf8,
        )
    } else if bytes.starts_with(b"PG") {
        (
            SubtitleFormat::Pgs,
            probe_sup(bytes, &mut why),
            true,
            EncodingHint::Binary,
        )
    } else if bytes.starts_with(&EBML_MAGIC) {
        why.push(
            "发现 Matroska/EBML 容器魔数：字幕位于轨道内，需先提取轨道（当前枚举不表示容器）"
                .to_string(),
        );
        (
            SubtitleFormat::Unknown,
            Confidence::Low,
            false,
            EncodingHint::Binary,
        )
    } else {
        let (enc, text) = decode_text(bytes, &mut why);
        let (f, c, content) = probe_text(&text, hint, ext.as_deref(), &mut why);
        (f, c, content, enc)
    };

    if from_content {
        match (hint, ext.as_deref()) {
            (Some(h), Some(e)) if h == format => why.push(format!("扩展名 .{e} 与内容签名一致")),
            (Some(h), Some(e)) => why.push(format!(
                "扩展名 .{e} 指向 {}，但内容签名判定为 {}；以内容为准（08 §3：扩展名只是 hint）",
                h.label(),
                format.label()
            )),
            _ => {}
        }
    }

    Detection {
        format,
        confidence,
        encoding_hint,
        reasons: why,
    }
}

pub fn probe_file(path: &Path) -> Result<Detection> {
    let mut file = std::fs::File::open(path).map_err(|e| {
        StkError::simple(Domain::Input, 2, "无法打开字幕文件")
            .with_detail(path.display().to_string())
            .with_detail(e.to_string())
            .with_action("确认文件存在、未被其他程序独占，且当前用户有读取权限")
    })?;
    let mut buf: Vec<u8> = Vec::new();
    file.by_ref()
        .take(PROBE_WINDOW)
        .read_to_end(&mut buf)
        .map_err(|e| {
            StkError::simple(Domain::Input, 3, "读取字幕文件失败")
                .with_detail(path.display().to_string())
                .with_detail(e.to_string())
                .with_action("重试读取；若为网络盘或光盘请复制到本地后再探测")
        })?;

    let hint = path.file_name().and_then(|n| n.to_str());
    let mut detection = probe_bytes(&buf, hint);
    if (buf.len() as u64) == PROBE_WINDOW {
        detection.reasons.push(format!(
            "仅读取前 {} KiB 做探测，超出部分未参与判定",
            PROBE_WINDOW / 1024
        ));
    }
    Ok(detection)
}

fn extension_of(hint: &str) -> Option<String> {
    Path::new(hint)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn probe_sup(bytes: &[u8], why: &mut Vec<String>) -> Confidence {
    why.push("发现 SUP/PGS 的 PG 段魔数".to_string());
    let mut off = 0usize;
    let mut segments = 0usize;
    let mut core = 0usize;
    let walk = loop {
        let remaining = bytes.len().saturating_sub(off);
        if remaining < SUP_MIN_HEADER {
            break if segments == 0 {
                Walk::Broken
            } else {
                Walk::Cut
            };
        }
        match sup_record(bytes, off) {
            Ok(record) => {
                segments = segments.saturating_add(1);
                if record.core {
                    core = core.saturating_add(1);
                }
                off = record.next;
                if off == bytes.len() {
                    break Walk::Closed;
                }
            }
            Err(msg) => {
                why.push(msg);
                break Walk::Broken;
            }
        }
    };

    match (walk, segments, core) {
        (Walk::Closed, seg, core_seen) if core_seen > 0 => {
            why.push(format!("{seg} 个段头按 13 字节头 + segment_length 恰好走完整个流，其中 {core_seen} 个为核心段类型"));
            Confidence::High
        }
        (Walk::Closed, seg, _) => {
            why.push(format!(
                "{seg} 个段走完但没有任何核心段类型（PCS/WDS/PDS/ODS/END），结构可疑"
            ));
            Confidence::Medium
        }
        (Walk::Cut, seg, core_seen) if seg >= 2 && core_seen > 0 => {
            why.push(format!(
                "前 {seg} 个段自洽，最后一个段头在本切片末尾被截断（读到的内容不是完整流）"
            ));
            Confidence::High
        }
        (Walk::Cut, seg, _) => {
            why.push(format!(
                "只有 {seg} 个完整段，随后在本切片末尾被截断，无法确认整流自洽"
            ));
            Confidence::Medium
        }
        (Walk::Broken, 0, _) => {
            why.push("只有 PG 魔数，首个段头即不自洽".to_string());
            Confidence::Medium
        }
        (Walk::Broken, seg, _) => {
            why.push(format!("前 {seg} 个段自洽，之后结构断裂"));
            Confidence::Medium
        }
    }
}

struct SupRecord {
    next: usize,
    core: bool,
}

fn sup_record(bytes: &[u8], off: usize) -> core::result::Result<SupRecord, String> {
    let head = off
        .checked_add(SUP_MIN_HEADER)
        .and_then(|need| bytes.get(off..need))
        .ok_or_else(|| format!("偏移 {off} 之后不足 {SUP_MIN_HEADER} 字节段头"))?;
    if !head.starts_with(b"PG") {
        return Err(format!("偏移 {off} 处缺少 PG 魔数，段流在此失去自洽"));
    }
    let seg_type = head[10];
    let seg_len = u16::from_be_bytes([head[11], head[12]]) as usize;
    // END(0x80)/Padding(0xFF) 允许空 payload，五个核心段不允许
    if seg_len == 0 && SUP_CORE_TYPES.contains(&seg_type) {
        return Err(format!(
            "偏移 {off} 的核心段 0x{seg_type:02X} payload 长度为 0，不像真实 PGS 流"
        ));
    }
    if !SUP_TYPES.contains(&seg_type) {
        return Err(format!(
            "偏移 {off} 的段类型 0x{seg_type:02X} 不属于 HDMV PGS 段类型集合"
        ));
    }
    let next = off
        .checked_add(SUP_PREFIX)
        .and_then(|x| x.checked_add(seg_len));
    match next {
        Some(n) if n <= bytes.len() => Ok(SupRecord {
            next: n,
            core: SUP_CORE_TYPES.contains(&seg_type),
        }),
        _ => Err(format!(
            "偏移 {off} 声明 segment_length={seg_len}，超出流结尾（越界或截断）"
        )),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Walk {
    Closed,
    Cut,
    Broken,
}

/// 返回 (编码提示, 用于签名匹配的文本)；BOM 与非法字节的行为都写进 reasons。
fn decode_text(bytes: &[u8], why: &mut Vec<String>) -> (EncodingHint, String) {
    if let Some(body) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return match std::str::from_utf8(body) {
            Ok(s) => {
                why.push("发现 UTF-8 BOM".to_string());
                (EncodingHint::Utf8Bom, s.to_string())
            }
            Err(_) => {
                why.push("有 UTF-8 BOM 但正文不是合法 UTF-8，提示后续按 ANSI 系解码".to_string());
                (
                    EncodingHint::AnsiLikely,
                    String::from_utf8_lossy(body).into_owned(),
                )
            }
        };
    }

    for (bom, big, name) in [
        ([0xffu8, 0xfe], false, "UTF-16LE"),
        ([0xfeu8, 0xff], true, "UTF-16BE"),
    ] {
        if let Some(body) = bytes.strip_prefix(&bom) {
            if bytes.starts_with(&[0xff, 0xfe, 0x00, 0x00]) {
                // UTF-32LE 的 BOM 与 UTF-16LE BOM + NUL 无法区分；字幕生态里没有 UTF-32 常规用法。
                why.push(
                    "前 4 字节为 FF FE 00 00，也可能是 UTF-32LE BOM，此处按 UTF-16LE 处理"
                        .to_string(),
                );
            }
            why.push(format!("发现 {name} BOM"));
            let (text, enc) = decode_utf16(body, big, why);
            return (enc, text);
        }
    }

    if let Ok(s) = std::str::from_utf8(bytes) {
        if looks_binary(bytes) {
            why.push("合法 UTF-8 但含 NUL/高比例控制字节，按二进制处理".to_string());
            return (EncodingHint::Binary, s.to_string());
        }
        return (EncodingHint::Utf8, s.to_string());
    }

    let lossy = String::from_utf8_lossy(bytes).into_owned();
    if looks_binary(bytes) {
        why.push("字节流含 NUL 或高比例控制字符，按二进制处理".to_string());
        return (EncodingHint::Binary, lossy);
    }
    why.push("正文不是合法 UTF-8，提示后续按 ANSI（GB18030/CP1252）解码".to_string());
    (EncodingHint::AnsiLikely, lossy)
}

fn decode_utf16(body: &[u8], big: bool, why: &mut Vec<String>) -> (String, EncodingHint) {
    let (units, dangling) = utf16_units(body, big);
    if let Some(byte) = dangling {
        why.push(format!(
            "UTF-16 正文长度为奇数（{} 字节），丢弃末尾半个码元 0x{byte:02X}",
            body.len()
        ));
    }
    let mut text = String::with_capacity(units.len());
    let mut lone = 0usize;
    for unit in char::decode_utf16(units.iter().copied()) {
        match unit {
            Ok(ch) => text.push(ch),
            Err(_) => {
                lone = lone.saturating_add(1);
                text.push(char::REPLACEMENT_CHARACTER);
            }
        }
    }
    if lone > 0 {
        why.push(format!(
            "UTF-16 中有 {lone} 个孤立代理项，逐个替换为 U+FFFD"
        ));
    }
    (
        text,
        if big {
            EncodingHint::Utf16Be
        } else {
            EncodingHint::Utf16Le
        },
    )
}

/// 手写 UTF-16 码元切分，避免为探测引入编码库；返回可能的半个码元。
fn utf16_units(body: &[u8], big: bool) -> (Vec<u16>, Option<u8>) {
    let mut units = Vec::with_capacity(body.len() / 2);
    let mut i = 0usize;
    while let Some(next) = i.checked_add(2) {
        let Some(chunk) = body.get(i..next) else {
            break;
        };
        let Ok(pair) = <[u8; 2]>::try_from(chunk) else {
            break;
        };
        units.push(if big {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        });
        i = next;
    }
    (units, body.get(i).copied())
}

fn looks_binary(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return true;
    }
    let control = bytes
        .iter()
        .copied()
        .filter(|b| *b < 0x20 && !matches!(*b, 0x09 | 0x0a | 0x0c | 0x0d))
        .count();
    control.checked_mul(10).is_none_or(|c| c > bytes.len())
}

/// 返回 (格式, 置信度, 是否由内容签名得出)；false 表示仅由扩展名支持。
fn probe_text(
    text: &str,
    hint: Option<SubtitleFormat>,
    ext: Option<&str>,
    why: &mut Vec<String>,
) -> (SubtitleFormat, Confidence, bool) {
    if has_bdn_root(text) {
        why.push("发现 <BDN 根元素（XML 内容签名）".to_string());
        return (SubtitleFormat::BdnXml, Confidence::High, true);
    }
    if webvtt_header(text) {
        why.push("首行为 WEBVTT 头".to_string());
        return (SubtitleFormat::WebVtt, Confidence::High, true);
    }
    let compact = compact(text);
    if let Some((f, c)) = probe_ass(text, &compact, hint, why) {
        return (f, c, true);
    }
    if let Some((f, c)) = probe_idx(text, why) {
        return (f, c, true);
    }
    let cues = srt_cue_count(text);
    if cues > 0 {
        why.push(format!(
            "发现 {cues} 组「序号行 + 含 --> 的时间行」，时间戳逗号与点分写法都接受"
        ));
        return (SubtitleFormat::Srt, Confidence::High, true);
    }
    match (hint, ext) {
        (Some(f), Some(e)) => {
            if f == SubtitleFormat::VobSubIdx && e == "sub" {
                why.push(".sub 位图流本身没有文本签名，需与 .idx 配对判定".to_string());
            }
            why.push(format!("内容未出现任何已知字幕签名，判定仅来自扩展名 .{e}"));
            (f, Confidence::Low, false)
        }
        _ => {
            why.push(
                "内容未匹配 PG 魔数、<BDN、WEBVTT、样式段、IDX 索引行或 SRT 时间行".to_string(),
            );
            (SubtitleFormat::Unknown, Confidence::Low, false)
        }
    }
}

/// `<BDN` 是 BDN XML 的唯一必需签名；空白与大小写都容忍，因为导出工具写法不一。
fn has_bdn_root(text: &str) -> bool {
    text.split('<').skip(1).any(|rest| {
        let head = rest.trim_start_matches(|c: char| c.is_ascii_whitespace());
        head.get(..3).is_some_and(|h| h.eq_ignore_ascii_case("BDN"))
    })
}

fn webvtt_header(text: &str) -> bool {
    for line in text.lines() {
        let head = line.trim_start();
        if head.is_empty() {
            continue;
        }
        let b = head.as_bytes();
        return b.len() >= 6
            && b[..6] == *b"WEBVTT"
            && (b.len() == 6 || matches!(b[6], b' ' | b'\t'));
    }
    false
}

fn probe_ass(
    text: &str,
    compact: &str,
    hint: Option<SubtitleFormat>,
    why: &mut Vec<String>,
) -> Option<(SubtitleFormat, Confidence)> {
    let v4plus = compact.contains("[v4+styles]");
    let v4styles = compact.contains("[v4styles]");
    let script_info = compact.contains("[scriptinfo]");
    let fmt_plus = compact.contains("format:v4.00+");
    let fmt_v4 = !fmt_plus && compact.contains("format:v4.00");
    let dialogue = field_lines(text, "dialogue:");
    let style = field_lines(text, "style:");

    if !(v4plus || v4styles || script_info || fmt_plus || fmt_v4 || dialogue > 0 || style > 0) {
        return None;
    }
    if script_info {
        why.push("发现 [Script Info] 段".to_string());
    }
    let strong = dialogue > 0 && style > 0;

    let out = if v4plus {
        why.push("发现 [V4+ Styles] 段（ASS 独有标记）".to_string());
        (SubtitleFormat::Ass, Confidence::High)
    } else if v4styles {
        why.push("发现 [V4 Styles] 段且无 [V4+ Styles]，判定 SSA".to_string());
        (SubtitleFormat::Ssa, Confidence::High)
    } else if fmt_plus {
        why.push("发现 Format: v4.00+ 但没有样式段头".to_string());
        (SubtitleFormat::Ass, Confidence::Medium)
    } else if fmt_v4 {
        why.push("发现 Format: v4.00（无 +）但没有样式段头".to_string());
        (SubtitleFormat::Ssa, Confidence::Medium)
    } else {
        let conf = if strong {
            Confidence::Medium
        } else {
            Confidence::Low
        };
        match hint {
            Some(SubtitleFormat::Ass) | Some(SubtitleFormat::Ssa) => {
                let f = if hint == Some(SubtitleFormat::Ssa) {
                    SubtitleFormat::Ssa
                } else {
                    SubtitleFormat::Ass
                };
                why.push(format!(
                    "缺少 [V4+/V4 Styles] 段头，版本由扩展名提示判定为 {}（Dialogue {dialogue} 条 / Style {style} 条）",
                    f.label()
                ));
                (f, conf)
            }
            Some(other) => {
                why.push(format!(
                    "只有 Dialogue:/Style: 行且无版本段头；扩展名提示的 {} 与 ASS 家族无关，默认按 ASS 处理",
                    other.label()
                ));
                (SubtitleFormat::Ass, Confidence::Low)
            }
            None => {
                why.push(format!(
                    "缺少 [V4+/V4 Styles] 段头且无扩展名线索，按 Dialogue/Style 行判定 ASS（{dialogue} 条 Dialogue / {style} 条 Style）"
                ));
                (SubtitleFormat::Ass, conf)
            }
        }
    };
    Some(out)
}

fn probe_idx(text: &str, why: &mut Vec<String>) -> Option<(SubtitleFormat, Confidence)> {
    let (mut has_format, mut has_size, mut has_index) = (false, false, false);
    for line in text.lines() {
        let key: String = line
            .trim_start()
            .chars()
            .take(16)
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect();
        if key.starts_with("#format") {
            has_format = true;
        } else if key.starts_with("size:") {
            has_size = true;
        } else if key.starts_with("index:") {
            has_index = true;
        }
    }
    if has_format || (has_size && has_index) {
        if has_format {
            why.push("发现 IDX 的 `# format` 声明行".to_string());
        }
        if has_size && has_index {
            why.push("发现 IDX 的 size: 与 index: 行".to_string());
        }
        return Some((SubtitleFormat::VobSubIdx, Confidence::High));
    }
    None
}

fn srt_cue_count(text: &str) -> usize {
    let mut cues = 0usize;
    let mut prev_index = false;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            prev_index = false;
            continue;
        }
        if prev_index && is_timing_line(l) {
            cues = cues.saturating_add(1);
        }
        prev_index = l.bytes().all(|b| b.is_ascii_digit());
    }
    cues
}

fn is_timing_line(line: &str) -> bool {
    let Some((lhs, rhs)) = line.split_once("-->") else {
        return false;
    };
    let start = lhs.split_whitespace().next_back().unwrap_or("");
    let end = rhs.split_whitespace().next().unwrap_or("");
    is_srt_time(start) && is_srt_time(end)
}

/// SRT 官方用逗号、许多导出工具用点，两者都是合法签名（08 §3）。
fn is_srt_time(tok: &str) -> bool {
    let Some(idx) = tok.find([',', '.']) else {
        return false;
    };
    let Some(next) = idx.checked_add(1) else {
        return false;
    };
    let (Some(hms), Some(frac)) = (tok.get(..idx), tok.get(next..)) else {
        return false;
    };
    let fields: Vec<&str> = hms.split(':').collect();
    fields.len() == 3
        && digits(fields[0], 1, 3)
        && digits(fields[1], 2, 2)
        && digits(fields[2], 2, 2)
        && digits(frac, 1, 9)
}

fn digits(s: &str, min: usize, max: usize) -> bool {
    s.len() >= min && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit())
}

fn field_lines(text: &str, key: &str) -> usize {
    text.lines()
        .filter(|line| {
            let t = line.trim_start();
            t.len() >= key.len() && t.as_bytes()[..key.len()].eq_ignore_ascii_case(key.as_bytes())
        })
        .count()
}

fn compact(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ass_text() -> String {
        "[Script Info]\nTitle: demo\nScriptType: v4.00+\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial,20\n\n[Events]\nFormat: Layer, Start, End, Text\nDialogue: 0,0:00:01.00,0:00:02.00,你好\n".to_string()
    }

    fn utf16le_with_bom(s: &str) -> Vec<u8> {
        let mut v = vec![0xff, 0xfe];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    fn utf16be_with_bom(s: &str) -> Vec<u8> {
        let mut v = vec![0xfe, 0xff];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }

    fn sup_record_bytes(pts: u32, seg_type: u8, payload_len: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"PG");
        v.extend_from_slice(&pts.to_be_bytes());
        v.extend_from_slice(&pts.to_be_bytes()); // DTS 与 PTS 同值，真实流常见
        v.push(seg_type);
        v.extend_from_slice(&(payload_len as u16).to_be_bytes());
        v.resize(v.len() + payload_len, 0xab);
        v
    }

    fn reason_text(d: &Detection) -> String {
        d.reasons.join(" | ")
    }

    #[test]
    fn ass_v4plus_section_is_high() {
        let d = probe_bytes(ass_text().as_bytes(), None);
        assert_eq!(d.format, SubtitleFormat::Ass);
        assert_eq!(d.confidence, Confidence::High);
        assert_eq!(d.encoding_hint, EncodingHint::Utf8);
        assert!(reason_text(&d).contains("[V4+ Styles]"), "{d:?}");
    }

    #[test]
    fn ass_crlf_still_high() {
        let d = probe_bytes(ass_text().replace('\n', "\r\n").as_bytes(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Ass, Confidence::High)
        );
    }

    #[test]
    fn ssa_v4styles_section_is_high() {
        let src = "[Script Info]\nTitle: old\n[V4 Styles]\nFormat: Font, Charset\nStyle: Default,Arial,1\n[Events]\nDialogue: 0:00:01.00,hi\n";
        let d = probe_bytes(src.as_bytes(), None);
        assert_eq!(d.format, SubtitleFormat::Ssa);
        assert_eq!(d.confidence, Confidence::High);
        assert!(reason_text(&d).contains("[V4 Styles]"));
    }

    #[test]
    fn script_info_plus_v4styles_is_ssa() {
        let src = "[Script Info]\nScriptType: v4.00\n[V4 Styles]\nFormat: Font\nStyle: A\n";
        let d = probe_bytes(src.as_bytes(), Some("movie.ssa"));
        assert_eq!(d.format, SubtitleFormat::Ssa);
        assert!(reason_text(&d).contains("[Script Info]"));
    }

    #[test]
    fn v4plus_beats_ssa_hint() {
        let d = probe_bytes(ass_text().as_bytes(), Some("movie.ssa"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Ass, Confidence::High)
        );
        assert!(
            reason_text(&d).contains("以内容为准"),
            "{}",
            reason_text(&d)
        );
    }

    #[test]
    fn dialogue_and_style_without_section_is_ass_medium() {
        let src = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,text\nStyle: Default,Arial,20\n";
        let d = probe_bytes(src.as_bytes(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Ass, Confidence::Medium)
        );
    }

    #[test]
    fn dialogue_and_style_follow_ssa_hint() {
        let src = "Dialogue: 0,0:00:01.00,hi\nStyle: Default,Arial\n";
        let d = probe_bytes(src.as_bytes(), Some("old.ssa"));
        assert_eq!(d.format, SubtitleFormat::Ssa);
        assert_eq!(d.confidence, Confidence::Medium);
        assert!(reason_text(&d).contains("扩展名"));
    }

    #[test]
    fn format_v400plus_line_without_section_is_medium() {
        let src = "Format: v4.00+\nDialogue: 0,x\n";
        let d = probe_bytes(src.as_bytes(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Ass, Confidence::Medium)
        );
    }

    #[test]
    fn utf16le_bom_ass_is_high() {
        let bytes = utf16le_with_bom(&ass_text());
        let d = probe_bytes(&bytes, Some("cn.ass"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Ass, Confidence::High)
        );
        assert_eq!(d.encoding_hint, EncodingHint::Utf16Le);
        assert!(reason_text(&d).contains("UTF-16LE BOM"));
    }

    #[test]
    fn utf16be_bom_srt_is_high() {
        let bytes = utf16be_with_bom("1\n00:00:01,000 --> 00:00:02,000\n你好\n");
        let d = probe_bytes(&bytes, None);
        assert_eq!(
            (d.format, d.confidence, d.encoding_hint),
            (SubtitleFormat::Srt, Confidence::High, EncodingHint::Utf16Be)
        );
    }

    #[test]
    fn utf8_bom_srt_is_high() {
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice("1\r\n00:00:01,000 --> 00:00:04,500\r\nHello\r\n".as_bytes());
        let d = probe_bytes(&bytes, None);
        assert_eq!(
            (d.format, d.confidence, d.encoding_hint),
            (SubtitleFormat::Srt, Confidence::High, EncodingHint::Utf8Bom)
        );
    }

    #[test]
    fn srt_dot_delimiter_and_crlf_accepted() {
        let d = probe_bytes(
            "1\r\n00:00:01.000 --> 00:00:02.500\r\nhi\r\n".as_bytes(),
            None,
        );
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Srt, Confidence::High)
        );
        let d2 = probe_bytes("12\r\n1:02:03,4 --> 1:02:05,000\r\nhi\r\n".as_bytes(), None);
        assert_eq!(d2.format, SubtitleFormat::Srt);
    }

    #[test]
    fn timing_line_needs_a_digit_index_line() {
        let d = probe_bytes(
            "some prose\n00:00:01,000 --> 00:00:02,000\nno index\n".as_bytes(),
            None,
        );
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Unknown, Confidence::Low)
        );
    }

    #[test]
    fn webvtt_header_is_high() {
        let src = "WEBVTT - demo\n\n00:00:00.000 --> 00:00:02.000\n- hi\n";
        let d = probe_bytes(src.as_bytes(), Some("s.vtt"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::WebVtt, Confidence::High)
        );
        assert!(reason_text(&d).contains("WEBVTT"));
    }

    #[test]
    fn bdn_xml_root_is_high() {
        let src = "<?xml version=\"1.0\"?>\n<BDN Version=\"0.93alpha\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n<Events>\n</BDN>\n";
        let d = probe_bytes(src.as_bytes(), Some("bdn.xml"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::BdnXml, Confidence::High)
        );
    }

    #[test]
    fn bdn_root_tolerates_case_and_whitespace() {
        let d = probe_bytes("<  bdn Version=\"1\">\n".as_bytes(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::BdnXml, Confidence::High)
        );
    }

    #[test]
    fn vobsub_idx_format_line_is_high() {
        let src = "# VobSub index file, v7 (do not modify this line!)\n# format: utf8,0x,00\nsize: 720,480\nindex: 0, 00:00:00.000, 00:00:05.000, 0x1234\n";
        let d = probe_bytes(src.as_bytes(), Some("movie.idx"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::VobSubIdx, Confidence::High)
        );
        assert!(reason_text(&d).contains("# format"));
    }

    #[test]
    fn vobsub_idx_size_and_index_lines_are_high() {
        let src = "alt: en\nsize: 720,480\norg: 720, 480\nindex: 1, 00:00:06.000, 0x900\n";
        let d = probe_bytes(src.as_bytes(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::VobSubIdx, Confidence::High)
        );
    }

    #[test]
    fn extension_hint_is_overridden_by_content() {
        let d = probe_bytes(
            "1\n00:00:01,000 --> 00:00:02,000\nHello\n".as_bytes(),
            Some("trick.ass"),
        );
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Srt, Confidence::High)
        );
        assert!(
            reason_text(&d).contains("扩展名只是 hint") || reason_text(&d).contains("以内容为准"),
            "{}",
            reason_text(&d)
        );
    }

    #[test]
    fn extension_only_never_high() {
        let d = probe_bytes(
            b"\x00\x01\x02 garbage that is not a subtitle \x1b\x1a".as_slice(),
            Some("fake.ass"),
        );
        assert_eq!(d.format, SubtitleFormat::Ass);
        assert_eq!(d.confidence, Confidence::Low);
        assert_eq!(d.encoding_hint, EncodingHint::Binary);
        assert!(reason_text(&d).contains("仅来自扩展名"));
    }

    #[test]
    fn garbage_bytes_are_unknown() {
        let d = probe_bytes(b"\x11\x22\x33\x44\xff\xee\x01\x01zz".as_slice(), None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Unknown, Confidence::Low)
        );
        assert!(!d.reasons.is_empty());
    }

    #[test]
    fn empty_input_is_unknown_and_not_panic() {
        let d = probe_bytes(b"", Some("a.srt"));
        assert_eq!(
            (d.format, d.confidence, d.encoding_hint),
            (SubtitleFormat::Unknown, Confidence::Low, EncodingHint::Utf8)
        );
    }

    #[test]
    fn ansi_srt_is_high_with_ansi_hint() {
        let mut bytes = b"1\r\n00:00:01,000 --> 00:00:02,000\r\n".to_vec();
        bytes.extend_from_slice(&[0xc4, 0xe3, 0xbf, 0xec]); // GBK "你好"
        bytes.extend_from_slice(b"\r\n");
        let d = probe_bytes(&bytes, None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Srt, Confidence::High)
        );
        assert_eq!(d.encoding_hint, EncodingHint::AnsiLikely);
    }

    #[test]
    fn sup_with_two_walking_segments_is_high() {
        let mut bytes = sup_record_bytes(90_000, 0x14, 5);
        bytes.extend_from_slice(&sup_record_bytes(180_000, 0x17, 20));
        bytes.extend_from_slice(&sup_record_bytes(270_000, 0x80, 0));
        let d = probe_bytes(&bytes, Some("movie.sup"));
        assert_eq!(
            (d.format, d.confidence, d.encoding_hint),
            (SubtitleFormat::Pgs, Confidence::High, EncodingHint::Binary)
        );
        assert!(reason_text(&d).contains("PGS"), "{}", reason_text(&d));
    }

    /// 手写 13 字节头的越界用例：声明的 payload 长度大于剩余字节。
    fn sup_header_over_long(pts: u32, seg_type: u8, declared: u16, filler: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"PG");
        v.extend_from_slice(&pts.to_be_bytes());
        v.extend_from_slice(&pts.to_be_bytes());
        v.push(seg_type);
        v.extend_from_slice(&declared.to_be_bytes());
        v.resize(v.len() + filler, 0x01);
        v
    }

    #[test]
    fn sup_with_out_of_range_length_is_medium() {
        let mut bytes = sup_record_bytes(90_000, 0x14, 5);
        bytes.extend_from_slice(&sup_header_over_long(180_000, 0x17, 60_000, 4));
        let d = probe_bytes(&bytes, None);
        assert_eq!(d.format, SubtitleFormat::Pgs);
        assert_eq!(d.confidence, Confidence::Medium);
        assert!(
            reason_text(&d).contains("超出流结尾"),
            "{}",
            reason_text(&d)
        );
    }

    /// 真实 display set 布局（13 字节头 + PCS19/WDS10/PDS/ODS/END0），必须判 High。
    /// 这条是防"自创段头布局"的回归：早前一版实现把 length 当 4 字节读，真流会被误判。
    #[test]
    fn sup_real_layout_display_set_is_high() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&sup_record_bytes(90_000, 0x16, 19)); // PCS
        bytes.extend_from_slice(&sup_record_bytes(90_000, 0x17, 10)); // WDS
        bytes.extend_from_slice(&sup_record_bytes(90_000, 0x14, 2 + 5 * 8)); // PDS
        bytes.extend_from_slice(&sup_record_bytes(90_000, 0x15, 1024)); // ODS
        bytes.extend_from_slice(&sup_record_bytes(90_000, 0x80, 0)); // END
        let d = probe_bytes(&bytes, Some("real.sup"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Pgs, Confidence::High)
        );
        assert!(reason_text(&d).contains("核心段"), "{}", reason_text(&d));
        assert_eq!(bytes.len(), 13 * 5 + 19 + 10 + 42 + 1024);
    }

    #[test]
    fn sup_magic_only_is_medium() {
        let d = probe_bytes(b"PGxyz", None);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Pgs, Confidence::Medium)
        );
        let d2 = probe_bytes(
            b"PG\x00\x00\x00\x01\x00\x00\x00\x02\x22\x00\x00\x99\x99",
            None,
        );
        assert_eq!(
            (d2.format, d2.confidence),
            (SubtitleFormat::Pgs, Confidence::Medium)
        );
        assert!(reason_text(&d2).contains("段类型"), "{}", reason_text(&d2));
    }

    #[test]
    fn odd_length_utf16le_does_not_panic() {
        let mut bytes = utf16le_with_bom(&ass_text());
        bytes.push(0x41);
        let d = probe_bytes(&bytes, None);
        assert_eq!(d.encoding_hint, EncodingHint::Utf16Le);
        assert_eq!(d.format, SubtitleFormat::Ass);
        assert!(reason_text(&d).contains("奇数"), "{}", reason_text(&d));
    }

    #[test]
    fn lone_surrogate_utf16le_is_replaced_not_panicked() {
        let mut bytes = vec![0xff, 0xfe];
        for u in [0x0053u16, 0xd800, 0x0074, 0xdc00, 0x0079] {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let d = probe_bytes(&bytes, None);
        assert_eq!(d.encoding_hint, EncodingHint::Utf16Le);
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Unknown, Confidence::Low)
        );
        assert!(
            reason_text(&d).contains("孤立代理项"),
            "{}",
            reason_text(&d)
        );
    }

    #[test]
    fn matroska_container_is_unknown_with_reason() {
        let mut bytes = vec![0x1a, 0x45, 0xdf, 0xa3, 0x01, 0x00, 0x00, 0x00, 0x00, 0x1f];
        bytes.extend_from_slice(b"\x80\x80\x80garbage");
        let d = probe_bytes(&bytes, Some("film.mkv"));
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Unknown, Confidence::Low)
        );
        assert!(reason_text(&d).contains("Matroska"));
    }

    #[test]
    fn sub_bitmap_pair_hint_is_low_vobsub() {
        let d = probe_bytes(
            b"\x20\x00\x07\x86\x80\x80\x02\x80\x02\x20\x00",
            Some("movie.sub"),
        );
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::VobSubIdx, Confidence::Low)
        );
        assert!(reason_text(&d).contains(".idx"));
    }

    #[test]
    fn unknown_extension_gives_unknown_format() {
        let d = probe_bytes(
            "just some plain prose, no cues at all\n".as_bytes(),
            Some("notes.txt"),
        );
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Unknown, Confidence::Low)
        );
    }

    #[test]
    fn serde_impls_exist_for_public_types() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<SubtitleFormat>();
        assert_serde::<Confidence>();
        assert_serde::<EncodingHint>();
        assert_serde::<Detection>();
    }

    #[test]
    fn probe_file_reads_head_of_large_file() {
        let path = temp_path("big-window.sup");
        let mut bytes = sup_record_bytes(90_000, 0x14, 5);
        bytes.extend_from_slice(&sup_header_over_long(180_000, 0x17, u16::MAX, 3));
        bytes.resize(70 * 1024, 0x5a);
        std::fs::write(&path, &bytes).expect("write fixture");
        let d = probe_file(&path).expect("probe ok");
        assert_eq!(d.format, SubtitleFormat::Pgs);
        assert_eq!(d.confidence, Confidence::Medium);
        assert!(
            reason_text(&d).contains("仅读取前 64 KiB"),
            "{}",
            reason_text(&d)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn probe_file_detects_text_and_uses_name_as_hint() {
        let path = temp_path("misleading.ass");
        std::fs::write(&path, "1\n00:00:01,000 --> 00:00:02,000\nHello\n").expect("write fixture");
        let d = probe_file(&path).expect("probe ok");
        assert_eq!(
            (d.format, d.confidence),
            (SubtitleFormat::Srt, Confidence::High)
        );
        assert!(
            reason_text(&d).contains("misleading") || reason_text(&d).contains("扩展名"),
            "{}",
            reason_text(&d)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn probe_file_missing_returns_input_error() {
        let path = std::env::temp_dir().join("stk-format-detect-definitely-missing-7f3a9c.bin");
        let e = probe_file(&path).expect_err("must fail");
        assert_eq!(e.domain, Domain::Input);
        assert_eq!(e.code_string(), "STK-INP-0002");
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("stk-format-detect-{}-{}", std::process::id(), name))
    }
}
