//! loss 报告（08_FORMATS_AND_IR.md §4）。AST 本身是无损的，这里报告的是「读不懂但保留了」的部分。

use core_model::loss::LossReport;

use crate::ast::{AssDocument, EntryKind, FormatSource, Section, SectionKind};
use crate::overrides;

/// Script Info 里被工具认识的 key；其余照样保留，但要记 nonstandard。
const KNOWN_SCRIPT_KEYS: &[&str] = &[
    "title",
    "original script",
    "type",
    "scripttype",
    "playresx",
    "playresy",
    "wrapstyle",
    "scaledborderandshadow",
    "video4by3",
    "videowidth",
    "videoheight",
    "ycbcr matrix",
    "layoutresx",
    "layoutresy",
    "active duration",
    "audio lock",
    "genfile",
    "gentime",
    "gentitle",
    "genprogram",
    "gensettings",
    "mixer master vector",
];

/// Events 段里不是事件条目但合法的 key（mkvmerge 附件写法等）。
const KNOWN_EVENT_KEYS: &[&str] = &["attachment", "comment", "dialogue", "format"];

fn known_key(section: &Section, key: &str) -> bool {
    let lc = key.to_lowercase();
    match section.kind {
        SectionKind::ScriptInfo => KNOWN_SCRIPT_KEYS.contains(&lc.as_str()),
        SectionKind::Events => KNOWN_EVENT_KEYS.contains(&lc.as_str()),
        SectionKind::Styles => lc == "format" || section.index_of(&lc).is_some(),
        _ => true,
    }
}

/// 末字段（Text）的下标。
fn text_index(tail: Option<usize>) -> usize {
    tail.unwrap_or(1).saturating_sub(1)
}

pub fn loss_of(doc: &AssDocument) -> LossReport {
    let mut report = LossReport::new();
    report.preserved("行原文顺序/换行风格/BOM");
    let mut unknown_tags = 0usize;
    let mut colour_tokens = 0usize;
    let mut dialogues = 0usize;
    let mut styles = 0usize;
    let mut attachment_lines = 0usize;

    for section in &doc.sections {
        if section.kind.needs_format() && section.format_source != FormatSource::FormatLine {
            report.nonstandard(&format!("缺少 Format 行:{}", header_of(section)));
        }
        if section.kind == SectionKind::Events
            && section.format_source == FormatSource::FormatLine
            && section.index_of("Start").is_none()
            && section.index_of("End").is_none()
        {
            // Aegisub 8b/10b 事故写法：Events 段的 Format 其实是样式表
            report.user_attention(&format!(
                "Events 段 Format 缺少时间字段:{}",
                header_of(section)
            ));
        }
        if section.kind.is_attachment() {
            attachment_lines += section.entries.len();
        }
        for entry in &section.entries {
            match entry.kind {
                EntryKind::Opaque => {
                    report.nonstandard(&format!("无法解析的行(原文保留):{}", sample(&entry.raw)));
                }
                EntryKind::KeyVal => {
                    if let Some(key) = entry.key() {
                        if !known_key(section, key) {
                            report.nonstandard(&format!("未知 key:{key}"));
                        }
                    }
                }
                EntryKind::Style => {
                    styles += 1;
                    let want = section.format.len();
                    let got = entry.field_count(section.tail_of(EntryKind::Style));
                    if want > 0 && got != want {
                        report.nonstandard(&format!("样式字段数不符:{got}!={want}"));
                    }
                }
                EntryKind::Dialogue | EntryKind::Comment => {
                    if entry.kind == EntryKind::Dialogue {
                        dialogues += 1;
                    }
                    let tail = section.event_tail();
                    if let Some(k) = tail {
                        let got = entry.field_count(tail);
                        if got < k {
                            report.nonstandard(&format!("事件字段数不足:{got}<{k}"));
                        }
                    }
                    if let Some(text) = entry.field_slice(tail, text_index(tail)) {
                        let mut names = Vec::new();
                        overrides::unknown_tags(text, &mut names);
                        for name in &names {
                            unknown_tags += 1;
                            report.nonstandard(&format!("未知 tag:{name}"));
                        }
                        colour_tokens += overrides::colour_token_count(text);
                    }
                }
                _ => {}
            }
        }
    }

    if attachment_lines > 0 {
        report.preserved(&format!("附件/字体行 {attachment_lines} 条原文保留"));
    }
    if dialogues > 0 {
        report.preserved(&format!("事件 {dialogues} 条原文保留"));
    }
    if styles > 0 {
        report.preserved(&format!("样式 {styles} 条原文保留"));
    }
    if colour_tokens > 0 {
        report.preserved(&format!("颜色 token {colour_tokens} 个可定位"));
    }
    if unknown_tags > 0 {
        report.user_attention(&format!("未识别 tag 共 {unknown_tags} 个，已原样保留"));
    }
    report
}

fn header_of(section: &Section) -> String {
    section
        .header
        .clone()
        .unwrap_or_else(|| "(无段头)".to_owned())
}

fn sample(line: &str) -> String {
    let t = line.trim();
    if t.len() <= 40 {
        return t.to_owned();
    }
    match t.get(..40) {
        Some(prefix) => format!("{prefix}…"),
        None => t.to_owned(),
    }
}
