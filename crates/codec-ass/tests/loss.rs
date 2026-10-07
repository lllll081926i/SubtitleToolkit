//! loss 报告：读不懂但保留了什么（08 §4）。

use codec_ass::{loss_of, AssDocument};

const SIMPLE: &str = include_str!("fixtures/simple.ass");
const FULL22: &str = include_str!("fixtures/full22.ass");
const UNKNOWN_TAGS: &str = include_str!("fixtures/unknown_tags.ass");
const COMMENTS: &str = include_str!("fixtures/comments_blanks.ass");
const BROKEN: &str = include_str!("fixtures/broken_lines.ass");
const FONTS: &str = include_str!("fixtures/fonts.ass");
const LEGACY: &str = include_str!("fixtures/legacy_no_format.ass");

#[test]
fn clean_file_is_reported_lossless() {
    let report = loss_of(&AssDocument::parse(SIMPLE));
    assert!(report.is_lossless(), "{}", report.summary());
    assert!(report.preserved.iter().any(|p| p.contains("事件 2 条")));
    assert!(report.counts.is_empty() || report.counts.iter().all(|(_, n)| *n >= 1));
}

#[test]
fn unknown_keys_are_counted_per_key() {
    let doc = AssDocument::parse(FULL22);
    let report = loss_of(&doc);
    assert_eq!(
        report.count_for("nonstandard", "未知 key:Aegisub ColorSpace"),
        1
    );
    assert_eq!(report.nonstandard.len(), 1, "{:?}", report.nonstandard);
    // 未知 key 只是 non-standard，不是 dropped：内容仍在原文里
    assert!(report.dropped.is_empty());
    assert!(report.count_for("dropped", "未知 key:Aegisub ColorSpace") == 0);
}

#[test]
fn unknown_tags_are_counted_and_reported_as_preserved() {
    let report = loss_of(&AssDocument::parse(UNKNOWN_TAGS));
    assert_eq!(report.count_for("nonstandard", r"未知 tag:\anm"), 1);
    assert_eq!(report.count_for("nonstandard", r"未知 tag:\zzz"), 1);
    // \be / \blur / \fr 属于已知 tag，不进未知计数
    assert_eq!(report.count_for("nonstandard", r"未知 tag:\be"), 0);
    assert!(report
        .user_attention
        .iter()
        .any(|u| u.contains("未识别 tag 共 2 个")));
    assert!(report.preserved.iter().any(|p| p.contains("颜色 token")));
}

#[test]
fn missing_format_line_is_reported() {
    let report = loss_of(&AssDocument::parse(LEGACY));
    assert_eq!(
        report.count_for("nonstandard", "缺少 Format 行:[V4 Styles]"),
        1
    );
    assert_eq!(
        report.count_for("nonstandard", "缺少 Format 行:[V4++ Styles]"),
        1
    );
    assert_eq!(
        report.count_for("nonstandard", "缺少 Format 行:[Events]"),
        1
    );
    assert!(report.preserved.iter().any(|p| p.contains("事件 3 条")));
    let clean = loss_of(&AssDocument::parse(FULL22));
    assert_eq!(clean.count_for("nonstandard", "缺少 Format 行:[Events]"), 0);
}

#[test]
fn unparsable_lines_and_field_counts_are_reported() {
    let report = loss_of(&AssDocument::parse(BROKEN));
    assert_eq!(
        report.count_for(
            "nonstandard",
            "无法解析的行(原文保留):Dialogue too short no colon"
        ),
        1
    );
    assert_eq!(
        report.count_for("nonstandard", "无法解析的行(原文保留):: starts with colon"),
        1
    );
    assert_eq!(
        report.count_for("nonstandard", "未知 key:Totally Unknown Key"),
        1
    );
    assert_eq!(report.count_for("nonstandard", "事件字段数不足:9<10"), 1);
    assert_eq!(report.count_for("nonstandard", "样式字段数不符:6!=3"), 1);
    assert_eq!(report.count_for("nonstandard", "样式字段数不符:2!=3"), 2);
    // 报告本身不影响原文
    let doc = AssDocument::parse(BROKEN);
    assert_eq!(doc.to_string(), BROKEN);
}

#[test]
fn attachments_are_reported_as_preserved_not_dropped() {
    let report = loss_of(&AssDocument::parse(FONTS));
    assert!(report.dropped.is_empty());
    assert!(report
        .preserved
        .iter()
        .any(|p| p.contains("附件/字体行 8 条")));
    // [Fonts]/[FileList] 里的冒号不会被误报为未知 key
    assert!(
        report
            .nonstandard
            .iter()
            .all(|n| !n.starts_with("未知 key:")),
        "{:?}",
        report.nonstandard
    );
}

#[test]
fn remark_and_blank_lines_produce_no_loss_entries() {
    let report = loss_of(&AssDocument::parse(COMMENTS));
    assert_eq!(report.count_for("nonstandard", "未知 key:BlankAfter"), 1);
    assert_eq!(report.count_for("nonstandard", "未知 key:Attachment"), 0);
    assert!(report.preserved.iter().any(|p| p.contains("样式 1 条")));
}

#[test]
fn sample_cap_and_summary_work_on_repetitive_input() {
    // 同一类问题只留样本 + 计数（LossReport 语义）
    let mut src = String::from("[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n");
    for i in 0..30 {
        src.push_str(&format!(
            "Dialogue: 0,0:00:0{i:0>1}.00,0:00:09.00,Default,,0,0,0,,{{\\zzz{i}}}\n"
        ));
    }
    let report = loss_of(&AssDocument::parse(&src));
    assert_eq!(
        report
            .nonstandard
            .iter()
            .filter(|n| n.starts_with(r"未知 tag:\zzz"))
            .count(),
        8
    );
    assert_eq!(report.count_for("nonstandard", r"未知 tag:\zzz0"), 1);
    assert!(report.user_attention.iter().any(|u| u.contains("共 30 个")));
    assert!(!report.summary().is_empty());
}

#[test]
fn loss_of_does_not_mutate_the_document() {
    let doc = AssDocument::parse(UNKNOWN_TAGS);
    let before = doc.to_string();
    let report = loss_of(&doc);
    assert_eq!(doc.to_string(), before);
    assert_eq!(before, UNKNOWN_TAGS);
    assert!(!report.preserved.is_empty());
}
