//! 结构与改写路径：动态字段表、原文保留、定点改写。

use codec_ass::ast::{AssVariant, EntryKind, FormatSource, SectionKind};
use codec_ass::AssDocument;

const SIMPLE: &str = include_str!("fixtures/simple.ass");
const FULL22: &str = include_str!("fixtures/full22.ass");
const COMMENTS: &str = include_str!("fixtures/comments_blanks.ass");
const FONTS: &str = include_str!("fixtures/fonts.ass");
const BROKEN: &str = include_str!("fixtures/broken_lines.ass");
const LEGACY: &str = include_str!("fixtures/legacy_no_format.ass");

#[test]
fn section_kinds_and_headers_keep_original_spelling() {
    let doc = AssDocument::parse(FULL22);
    let kinds: Vec<SectionKind> = doc.sections.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        vec![
            SectionKind::ScriptInfo,
            SectionKind::Styles,
            SectionKind::Events,
            SectionKind::FileList,
            SectionKind::Unknown
        ]
    );
    assert_eq!(doc.sections[1].header.as_deref(), Some("[V4+ Styles]"));
    assert_eq!(doc.sections[1].variant, AssVariant::V4Plus);
    assert_eq!(doc.sections[3].header.as_deref(), Some("[Attach FileList]"));
    // 大小写与后缀写法保留在 header，归一化文本另存 header_key
    let lower = AssDocument::parse("[v4++ styles]\n[Events (info)]\n");
    assert_eq!(lower.sections[0].header.as_deref(), Some("[v4++ styles]"));
    assert_eq!(lower.sections[0].variant, AssVariant::V4PlusPlus);
    assert_eq!(lower.sections[0].header_key, "v4++ styles");
    assert_eq!(lower.sections[1].kind, SectionKind::Events);
}

#[test]
fn styles_format_is_dynamic() {
    let doc = AssDocument::parse(FULL22);
    let styles = &doc.sections[1];
    assert_eq!(styles.format_source, FormatSource::FormatLine);
    assert_eq!(styles.format.len(), 23);
    assert_eq!(styles.index_of("Fontsize"), Some(2));
    assert_eq!(styles.index_of("encoding"), Some(22));
    assert_eq!(styles.index_of("Fontsize"), styles.index_of("FONTSIZE"));
    assert!(styles.index_of("NoSuchField").is_none());
    assert_eq!(styles.style_entries().len(), 4);
}

#[test]
fn format_line_without_leading_style_token_and_with_spaces() {
    let doc = AssDocument::parse(
        "[V4 Styles]\nFormat: Style,Name,Fontname,Fontsize\nStyle: X,Arial,12\n",
    );
    let s = doc.section(0).unwrap();
    assert_eq!(
        s.format,
        vec![
            "Name".to_owned(),
            "Fontname".to_owned(),
            "Fontsize".to_owned()
        ]
    );
    assert_eq!(s.entry(1).unwrap().field_slice(None, 0), Some("X"));
    assert_eq!(s.entry(1).unwrap().field_slice(None, 2), Some("12"));
}

#[test]
fn missing_format_line_falls_back_to_default_table() {
    let doc = AssDocument::parse(LEGACY);
    let v4 = doc
        .sections
        .iter()
        .find(|s| s.header.as_deref() == Some("[V4 Styles]"))
        .expect("V4 Styles 段");
    assert_eq!(v4.format_source, FormatSource::RepeatedHeader);
    assert_eq!(v4.format.len(), 17);
    let v4pp = doc
        .sections
        .iter()
        .find(|s| s.header.as_deref() == Some("[V4++ Styles]"))
        .expect("V4++ 段");
    assert_eq!(v4pp.format.len(), 23);
    let ev = doc
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Events)
        .expect("Events 段");
    assert_eq!(ev.format.len(), 10);
    assert_eq!(ev.format.last().map(String::as_str), Some("Text"));
    assert_eq!(ev.dialogue_entries().len(), 3);
}

#[test]
fn entry_kinds_cover_comments_remarks_blanks_and_opaque() {
    let doc = AssDocument::parse(COMMENTS);
    let info = &doc.sections[0];
    let kinds: Vec<EntryKind> = info.entries.iter().map(|e| e.kind).collect();
    assert_eq!(kinds[0], EntryKind::Remark);
    assert_eq!(kinds[1], EntryKind::Remark);
    assert_eq!(kinds[2], EntryKind::Remark);
    assert!(kinds.contains(&EntryKind::KeyVal));
    assert!(kinds.contains(&EntryKind::Blank));
    // 值里的尾随空格属于原文
    let blank_after = info
        .entries
        .iter()
        .find(|e| e.key() == Some("BlankAfter"))
        .expect("BlankAfter");
    assert_eq!(blank_after.value(), Some(""));
    assert_eq!(blank_after.raw, "BlankAfter: ");

    let ev = &doc.sections[2];
    assert!(ev.entries.iter().any(|e| e.kind == EntryKind::Comment));
    assert!(ev.entries.iter().any(|e| e.kind == EntryKind::Remark));
    assert!(ev.entries.iter().any(|e| e.kind == EntryKind::Blank));
    assert!(ev
        .entries
        .iter()
        .any(|e| e.kind == EntryKind::KeyVal && e.key() == Some("Attachment")));
    // trailing 空行/注释属于该段，而不是被丢掉
    assert_eq!(
        ev.entries.last().map(|e| e.raw.as_str()),
        Some("Attachment: data.bin")
    );
}

#[test]
fn unparsable_lines_are_kept_verbatim_as_opaque() {
    let doc = AssDocument::parse(BROKEN);
    let ev = doc
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Events)
        .expect("Events");
    let opaque: Vec<&str> = ev
        .entries
        .iter()
        .filter(|e| e.kind == EntryKind::Opaque)
        .map(|e| e.raw.as_str())
        .collect();
    assert_eq!(
        opaque,
        vec!["Dialogue too short no colon", ": starts with colon"]
    );
    // 数字开头的 key 沿用上一行语义，仍然可读出时间轴
    let digit_line = ev
        .entries
        .iter()
        .find(|e| e.raw.starts_with("0,0:00:05.00"))
        .expect("无 key 前缀行");
    assert_eq!(digit_line.kind, EntryKind::Dialogue);
    assert_eq!(doc.sections[0].kind, SectionKind::ScriptInfo);
    assert_eq!(doc.sections[0].header.as_deref(), Some("[Script Info"));
}

#[test]
fn duplicate_keys_are_all_retained_in_order() {
    let doc = AssDocument::parse(FULL22);
    let xs: Vec<&str> = doc.sections[0]
        .entries
        .iter()
        .filter(|e| e.key() == Some("PlayResX"))
        .map(|e| e.value_trimmed().unwrap_or(""))
        .collect();
    assert_eq!(xs, vec!["1920", "1920"]);
    assert_eq!(
        doc.sections[0]
            .entries
            .iter()
            .filter(|e| e.key() == Some("PlayResX"))
            .count(),
        2
    );
    assert_eq!(doc.script_info("PlayResX"), Some("1920"));
    assert_eq!(doc.play_res(), Some((1920, 1080)));
    assert_eq!(doc.script_info("ycbcr matrix"), Some("TV.709"));
    assert_eq!(doc.script_info("NoSuchKey"), None);
}

#[test]
fn out_of_order_events_keep_document_order() {
    let doc = AssDocument::parse(FULL22);
    let ds = doc.dialogues();
    let starts: Vec<String> = ds
        .iter()
        .map(|d| {
            d.start()
                .map(|t| t.to_ass_string().unwrap_or_default())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        starts,
        vec![
            "0:00:01.00",
            "0:00:09.00",
            "0:00:05.00",
            "0:02:03.45",
            "0:00:12.00"
        ]
    );
    // 排序视角给出的下标不改变原文顺序
    let order = doc.sorted_index_order();
    assert_eq!(order, vec![0, 2, 1, 4, 3]);
    assert_eq!(
        doc.dialogues()[1].start().unwrap().to_ass_string().unwrap(),
        "0:00:09.00"
    );
    assert_eq!(doc.sections[2].dialogue_entries().len(), 5);
    assert_eq!(doc.to_string(), FULL22);
}

#[test]
fn dialogue_text_with_commas_is_the_last_field_as_a_whole() {
    let doc = AssDocument::parse(FULL22);
    let with_commas = doc
        .dialogues()
        .into_iter()
        .find(|d| d.text().starts_with("Text with"))
        .expect("含逗号文本");
    assert_eq!(with_commas.text(), "Text with, commas, inside");
    assert_eq!(with_commas.style_name(), Some("OutlineOnly"));
    assert_eq!(with_commas.layer(), Some("0"));
    let line = with_commas.entry.raw.clone();
    assert_eq!(line.matches(',').count(), 11);
}

#[test]
fn set_field_only_touches_the_target_field() {
    let mut doc = AssDocument::parse(FULL22);
    let si = doc.sections_of(SectionKind::Styles)[0];
    let ei = doc.section(si).unwrap().style_entries()[0];
    let before = doc.section(si).unwrap().entry(ei).unwrap().raw.clone();
    assert_eq!(before, "Style: Default,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,3,1,2,25,25,25,1");
    doc.section_mut(si)
        .unwrap()
        .set_field(ei, "Fontsize", "40")
        .unwrap();
    let after = doc.section(si).unwrap().entry(ei).unwrap().raw.clone();
    assert_eq!(after, "Style: Default,Arial,40,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,3,1,2,25,25,25,1");
    assert_eq!(before.matches(',').count(), after.matches(',').count());
    // 整篇文档只有这一行变化
    let rendered = doc.to_string();
    let diff: Vec<&str> = rendered
        .lines()
        .zip(FULL22.lines())
        .filter(|(a, b)| a != b)
        .map(|(a, _)| a)
        .collect();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0], "Style: Default,Arial,40,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,3,1,2,25,25,25,1");
}

#[test]
fn set_field_preserves_colon_space_and_comma_styles() {
    let src = "[V4+ Styles]\nFormat: Name,Fontname, Fontsize ,PrimaryColour\nStyle: A,B, 20 ,&H00FFFFFF\n";
    let mut doc = AssDocument::parse(src);
    let si = doc.sections_of(SectionKind::Styles)[0];
    let ei = doc.section(si).unwrap().style_entries()[0];
    doc.section_mut(si)
        .unwrap()
        .set_field(ei, "Fontsize", "40")
        .unwrap();
    // 字段前的空格属于前一个分隔符，字段内的空格属于值本身：都保持原样
    assert_eq!(
        doc.section(si).unwrap().entry(ei).unwrap().raw,
        "Style: A,B, 40 ,&H00FFFFFF"
    );
    assert_eq!(doc.to_string(), src.replace(" 20 ", " 40 "));

    let mut spaced = AssDocument::parse("[V4+ Styles]\nFormat: Name, Fontname\nStyle: A, B, C\n");
    let si = spaced.sections_of(SectionKind::Styles)[0];
    spaced
        .section_mut(si)
        .unwrap()
        .set_field(1, "Fontname", "Z")
        .unwrap();
    assert_eq!(
        spaced.sections[0].entry(1).expect("style").raw,
        "Style: A, Z, C"
    );
    assert_eq!(
        spaced.to_string(),
        "[V4+ Styles]\nFormat: Name, Fontname\nStyle: A, Z, C\n"
    );
}

#[test]
fn set_field_errors_are_reported_not_panics() {
    let mut doc = AssDocument::parse(SIMPLE);
    let si = doc.sections_of(SectionKind::Styles)[0];
    let ei = doc.section(si).unwrap().style_entries()[0];
    assert!(doc
        .section_mut(si)
        .unwrap()
        .set_field(ei, "NoSuchField", "1")
        .is_err());
    assert!(doc
        .section_mut(si)
        .unwrap()
        .set_field(999, "Fontsize", "1")
        .is_err());
    // 注释行没有可改写结构
    assert!(doc
        .section_mut(0)
        .unwrap()
        .set_field(0, "Title", "x")
        .is_err());
    assert_eq!(doc.section(0).unwrap().entry(0).expect("remark").raw, "; b");
    // 字段比 Format 少时，越界的字段报错但原文不动
    let mut broken =
        AssDocument::parse("[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Short,Arial\n");
    let si = broken.sections_of(SectionKind::Styles)[0];
    let before = broken.to_string();
    assert!(broken
        .section_mut(si)
        .unwrap()
        .set_field(1, "Fontsize", "20")
        .is_err());
    assert_eq!(broken.to_string(), before);
}

#[test]
fn set_field_on_event_text_and_time_keeps_the_rest() {
    let mut doc = AssDocument::parse(SIMPLE);
    let ev = doc.sections_of(SectionKind::Events)[0];
    let first = doc.section(ev).unwrap().dialogue_entries()[0];
    doc.section_mut(ev)
        .unwrap()
        .set_field(first, "Text", "新的文本, 带逗号")
        .unwrap();
    doc.section_mut(ev)
        .unwrap()
        .set_field(first, "Start", "0:01:02.03")
        .unwrap();
    let line = doc
        .section(ev)
        .unwrap()
        .entry(first)
        .expect("dialogue")
        .raw
        .clone();
    assert_eq!(
        line,
        "Dialogue: 0,0:01:02.03,0:00:04.00,Default,,0,0,0,,新的文本, 带逗号"
    );
    assert_eq!(line.matches(',').count(), 10);
    let d = doc.dialogues();
    assert_eq!(d[0].start().unwrap().to_ass_string().unwrap(), "0:01:02.03");
    assert_eq!(d[0].text(), "新的文本, 带逗号");
    assert_eq!(d[1].text(), "Simple line two");
}

#[test]
fn set_field_on_script_info_keyval_preserves_colon_spacing() {
    let src = "[Script Info]\nPlayResX:1280\nPlayResY:  720\n";
    let mut doc = AssDocument::parse(src);
    doc.section_mut(0)
        .unwrap()
        .set_field(0, "PlayResX", "2560")
        .unwrap();
    doc.section_mut(0)
        .unwrap()
        .set_field(1, "PlayResY", "1440")
        .unwrap();
    // `Key:1280` 与 `Key:  1440` 两种写法各自保持，其它行不动
    assert_eq!(
        doc.to_string(),
        "[Script Info]\nPlayResX:2560\nPlayResY:  1440\n"
    );
    assert!(doc
        .section_mut(0)
        .unwrap()
        .set_field(1, "PlayResX", "1")
        .is_err());
    assert!(doc
        .section_mut(0)
        .unwrap()
        .set_field(0, "PlayResY", "1")
        .is_err());
    assert_eq!(doc.play_res(), Some((2560, 1440)));
}

#[test]
fn attachments_and_fonts_are_preserved_line_by_line() {
    let doc = AssDocument::parse(FONTS);
    let fonts = doc
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::Fonts)
        .expect("[V4 Fonts]");
    assert_eq!(fonts.entries.len(), 5);
    assert!(fonts
        .entries
        .iter()
        .filter(|e| e.kind != EntryKind::Blank)
        .all(|e| e.kind == EntryKind::Raw));
    let files = doc
        .sections
        .iter()
        .find(|s| s.kind == SectionKind::FileList)
        .expect("[FileList]");
    assert_eq!(
        files
            .entries
            .iter()
            .map(|e| e.raw.as_str())
            .collect::<Vec<_>>(),
        vec!["FooSans.ttf", "/path/with space/other.ttf", ""]
    );
    // 冒号出现在 base64 行里也不会被当成 key/value
    assert!(fonts
        .entries
        .iter()
        .any(|e| e.raw.starts_with("!not base64 at all:")));
    assert_eq!(doc.to_string(), FONTS);
}

#[test]
fn script_info_order_is_preserved_not_regrouped() {
    let src = "[Script Info]\nZ: 1\nA: 2\nM: 3\nA: 4\n";
    let doc = AssDocument::parse(src);
    let keys: Vec<String> = doc.sections[0]
        .entries
        .iter()
        .filter_map(|e| e.key().map(str::to_owned))
        .collect();
    assert_eq!(keys, vec!["Z", "A", "M", "A"]);
    assert_eq!(doc.to_string(), src);
    assert_eq!(doc.script_info("A"), Some("4"));
}

#[test]
fn newline_style_is_detected_per_entry() {
    let doc = AssDocument::parse(FULL22);
    assert_eq!(doc.newline, "\n");
    let crlf = FULL22.replace('\n', "\r\n");
    let doc2 = AssDocument::parse(&crlf);
    assert_eq!(doc2.newline, "\r\n");
    let mixed = include_str!("fixtures/mixed_eol.ass");
    let doc3 = AssDocument::parse(mixed);
    assert!(doc3.bom);
    assert!(doc3.sections[0]
        .entries
        .iter()
        .any(|e| format!("{:?}", e.eol).contains("CrLf")));
    assert!(doc3.sections[0]
        .entries
        .iter()
        .any(|e| format!("{:?}", e.eol).contains("Lf")));
}
