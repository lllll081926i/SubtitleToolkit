//! Styles/Events 读取视角与时间/颜色便捷读取。

use codec_ass::views::parse_ass_time;
use codec_ass::AssDocument;

const FULL22: &str = include_str!("fixtures/full22.ass");
const SIMPLE: &str = include_str!("fixtures/simple.ass");
const LEGACY: &str = include_str!("fixtures/legacy_no_format.ass");

#[test]
fn style_view_reads_fields_by_name() {
    let doc = AssDocument::parse(FULL22);
    let styles = doc.styles();
    assert_eq!(styles.len(), 4);
    let first = styles.first().expect("Default");
    assert_eq!(first.name(), Some("Default"));
    assert_eq!(first.fontname(), Some("Arial"));
    assert_eq!(first.field("Fontsize"), Some("48"));
    assert_eq!(first.field("FontSize"), Some("48"));
    assert_eq!(first.field("marginv"), Some("25"));
    assert_eq!(first.field("NoSuchField"), None);
    assert_eq!(first.field_at(0), Some("Default"));
    assert_eq!(first.outline(), Some("3"));
    assert_eq!(first.shadow(), Some("1"));
    assert_eq!(first.index, doc.sections[1].style_entries()[0]);
    // 带空格的写法：值本身不被 trim 掉排版空格以外的部分
    assert_eq!(styles[3].field("BackColour"), Some("&H80000000"));
}

#[test]
fn style_colour_fields_are_locatable_for_hdr() {
    let doc = AssDocument::parse(FULL22);
    let s = doc.styles().remove(0);
    assert_eq!(
        s.primary_colour().map(|c| c.to_ass_string()),
        Some("&H00FFFFFF".to_owned())
    );
    assert_eq!(
        s.secondary_colour().map(|c| c.to_ass_string()),
        Some("&H000000FF".to_owned())
    );
    assert_eq!(
        s.outline_colour().map(|c| c.to_ass_string()),
        Some("&H00000000".to_owned())
    );
    assert_eq!(
        s.back_colour().map(|c| c.to_ass_string()),
        Some("&H80000000".to_owned())
    );
    // 中文样式名 + 全宽字段
    let zh = doc.styles().remove(1);
    assert_eq!(zh.name(), Some("中文"));
    assert_eq!(zh.fontname(), Some("Microsoft YaHei UI"));
    assert_eq!(zh.field("PrimaryColour"), Some("&H0000FFFF"));
}

#[test]
fn dialogue_view_time_and_text() {
    let doc = AssDocument::parse(SIMPLE);
    let ds = doc.dialogues();
    assert_eq!(ds.len(), 2);
    let d = ds.first().expect("first");
    assert_eq!(d.start().unwrap().to_ass_string().unwrap(), "0:00:01.00");
    assert_eq!(d.end().unwrap().to_ass_string().unwrap(), "0:00:04.00");
    assert_eq!(d.text(), "Simple line one");
    assert_eq!(d.style_name(), Some("Default"));
    assert_eq!(d.actor(), Some(""));
    assert_eq!(d.layer(), Some("0"));
    assert_eq!(d.margin_l(), Some("0"));
    assert_eq!(d.effect(), Some(""));
    assert_eq!(doc.dialogue_count(), 2);
}

#[test]
fn ass_time_parsing_uses_core_model_and_reports_errors() {
    let t = parse_ass_time("1:02:03.45").unwrap();
    assert_eq!(t.to_ass_string().unwrap(), "1:02:03.45");
    assert_eq!(t.as_rational().num(), 74_469);
    assert_eq!(t.as_rational().den(), 20);
    assert_eq!(
        parse_ass_time("0:00:01.00").unwrap(),
        codec_ass::Time::ZERO
            .add(&codec_ass::Time::from_secs(1))
            .unwrap()
    );
    assert_eq!(
        parse_ass_time(" 0:00:10.50 ")
            .unwrap()
            .to_ass_string()
            .unwrap(),
        "0:00:10.50"
    );
    // 秒位缺省小数按 0 处理（现实文件里确有 `0:00:01`），不猜毫秒
    assert_eq!(
        parse_ass_time("0:00:01").unwrap().to_ass_string().unwrap(),
        "0:00:01.00"
    );
    for bad in [
        "",
        "99:00:00.00:00",
        "a:00:00.00",
        "0:60:00.00",
        "0:00:60.00",
        "0:00:00.100",
        "-1:00:00.00",
        "0:00:01.x",
        "0:00:.00",
        "::",
    ] {
        let err = parse_ass_time(bad);
        assert!(err.is_err(), "{bad:?} 必须报错而不是 panic");
    }
}

#[test]
fn dialogue_time_accessor_propagates_parse_failure() {
    let doc = AssDocument::parse("[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,broken,0:00:02.00,Default,,0,0,0,,x\n");
    let d = doc.dialogues().remove(0);
    assert!(d.start().is_err());
    assert!(d.end().is_ok());
    assert_eq!(d.text(), "x");
    assert_eq!(doc.sorted_index_order(), vec![0]);
}

#[test]
fn out_of_order_events_and_sorting_view() {
    let doc = AssDocument::parse(FULL22);
    let order = doc.sorted_index_order();
    let times: Vec<String> = order
        .iter()
        .map(|i| {
            doc.dialogues()[*i]
                .start()
                .map(|t| t.to_ass_string().unwrap_or_default())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        times,
        vec![
            "0:00:01.00",
            "0:00:05.00",
            "0:00:09.00",
            "0:00:12.00",
            "0:02:03.45"
        ]
    );
    // 排序不改动文档：原文仍然逐字节相等
    assert_eq!(doc.to_string(), FULL22);
    let positions = doc.sorted_dialogue_positions();
    assert_eq!(positions.len(), 5);
    assert!(positions.iter().all(|(_, _, flat)| *flat < 5));
}

#[test]
fn legacy_events_without_format_line_still_expose_time_and_text() {
    let doc = AssDocument::parse(LEGACY);
    let ds = doc.dialogues();
    assert_eq!(ds.len(), 3);
    // Marked=0 开头的 SSA 行
    assert_eq!(
        ds[0].start().unwrap().to_ass_string().unwrap(),
        "0:00:01.00"
    );
    assert_eq!(ds[0].layer(), Some("Marked=0"));
    assert_eq!(ds[0].actor(), Some("NTP"));
    assert_eq!(ds[0].text(), "{\\anm2}legacy marked line");
    // 8b/10b 写法（整行少一个字段）自动左移一格
    assert_eq!(
        ds[1].start().unwrap().to_ass_string().unwrap(),
        "0:00:03.00"
    );
    assert_eq!(ds[1].end().unwrap().to_ass_string().unwrap(), "0:00:04.00");
    assert_eq!(ds[1].style_name(), Some("Default"));
    assert_eq!(ds[1].text(), "aegisub 8b/10b style format");
    assert_eq!(ds[2].text(), "second such line");
    assert_eq!(doc.to_string(), LEGACY);
}

#[test]
fn comments_are_exposed_as_views_but_not_as_dialogues() {
    let doc = AssDocument::parse(FULL22);
    assert_eq!(doc.comments().len(), 1);
    assert_eq!(
        doc.comments().first().expect("comment").text(),
        "note kept verbatim"
    );
    assert_eq!(doc.dialogue_count(), 5);
    assert_eq!(
        doc.dialogues()
            .iter()
            .filter(|d| d.text().contains("note"))
            .count(),
        0
    );
}

#[test]
fn unicode_smp_and_cjk_survive_views() {
    let doc = AssDocument::parse(FULL22);
    let d = doc
        .dialogues()
        .into_iter()
        .find(|d| d.text().contains("中文"))
        .expect("CJK 行");
    assert_eq!(d.text(), "中文 + Emoji \u{1F3AC} subtitle");
    assert_eq!(d.style_name(), Some("中文"));
    assert_eq!(d.overrides().len(), 1);
}

#[test]
fn inline_colour_tokens_are_locatable_from_a_dialogue() {
    let src = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\1c&H0000FF&}A{\\3c&H00FF00&}B\n";
    let doc = AssDocument::parse(src);
    let d = doc.dialogues().remove(0);
    assert_eq!(codec_ass::overrides::colour_token_count(d.text()), 2);
    let mut slots = Vec::new();
    for token in d.overrides() {
        if let codec_ass::OverrideToken::Block(b) = token {
            for tag in b.tags {
                if let codec_ass::TagKind::Colour { slot } = tag.kind {
                    slots.push((slot, tag.args.clone()));
                }
            }
        }
    }
    assert_eq!(
        slots,
        vec![(1, "&H0000FF&".to_owned()), (3, "&H00FF00&".to_owned())]
    );
}

#[test]
fn styles_from_a_section_view_are_addressable_for_mutation() {
    let mut doc = AssDocument::parse(FULL22);
    let si = doc.sections_of(codec_ass::SectionKind::Styles)[0];
    let indices = doc.section(si).unwrap().style_entries();
    assert_eq!(indices.len(), 4);
    // 逐条把 Fontsize 翻倍写出后仍能读回
    for i in indices.clone() {
        let current = doc
            .section(si)
            .unwrap()
            .style_view(i)
            .expect("view")
            .fontsize()
            .unwrap_or("0")
            .to_owned();
        let doubled: i64 = current.parse::<i64>().unwrap_or(0) * 2;
        doc.section_mut(si)
            .unwrap()
            .set_field(i, "Fontsize", &doubled.to_string())
            .unwrap();
    }
    let sizes: Vec<String> = doc
        .styles()
        .into_iter()
        .map(|s| s.fontsize().unwrap_or("").to_owned())
        .collect();
    assert_eq!(sizes, vec!["96", "108", "120", "72"]);
    // 除 Fontsize 外没有别的字节变化
    let rendered = doc.to_string();
    let changed: Vec<(usize, &str, &str)> = rendered
        .lines()
        .zip(FULL22.lines())
        .enumerate()
        .map(|(n, (a, b))| (n, a, b))
        .filter(|(_, a, b)| a != b)
        .collect();
    assert_eq!(changed.len(), 4);
    assert!(changed.iter().all(|(_, a, _)| a.starts_with("Style: ")));
}
