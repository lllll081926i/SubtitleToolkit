//! XML 安全（23 §XML）：禁止外部实体/网络解析、长度与深度上限必须生效且不 OOM、
//! 结构错误带位置、任何畸形输入都不 panic（fuzz 前提）。

use codec_bdn::{
    Limits, DEFAULT_LIMITS, DOC_BYTES_HARD_MAX, EVENT_COUNT_HARD_MAX, NAME_LEN_HARD_MAX,
    TEXT_LEN_HARD_MAX,
};

mod common;
use common::sample_xml;

fn code_of(xml: &str) -> String {
    codec_bdn::parse(xml)
        .err()
        .unwrap_or_else(|| panic!("期望错误，但解析成功：{xml}"))
        .code_string()
}

/// 上限常量本身要合理：太小会拒绝真实 BDN，太大会失去意义。
#[test]
fn default_limits_are_bounded_constants() {
    assert_eq!(DEFAULT_LIMITS.doc_bytes, DOC_BYTES_HARD_MAX);
    assert_eq!(DEFAULT_LIMITS.text_bytes, TEXT_LEN_HARD_MAX);
    assert_eq!(DEFAULT_LIMITS.name_bytes, NAME_LEN_HARD_MAX);
    assert_eq!(DEFAULT_LIMITS.events, EVENT_COUNT_HARD_MAX);
    // 这些下限是"安全上限不能挡掉真实文件"的合同，用编译期断言，跑不跑测试都要成立。
    const _: () = assert!(DOC_BYTES_HARD_MAX >= 1024 * 1024); // 真实 BDN 可达数 MB
    const _: () = assert!(TEXT_LEN_HARD_MAX >= 1024); // 容得下 260+ 字符的中文路径
    const _: () = assert!(DEFAULT_LIMITS.depth > 0 && DEFAULT_LIMITS.depth <= 128);
}

#[test]
fn external_entity_doctype_is_rejected_not_resolved() {
    let xml = r#"<!DOCTYPE BDN [<!ENTITY x SYSTEM "file:///C:/Windows/win.ini">]>
<BDN Version="0.93"><BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>&x;</FileName><Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic>
</Event></BDNSubtitleData></BDN>"#;
    assert_eq!(code_of(xml), "STK-BDN-0051");
}

/// XXE 底线：DTD/外部实体一律拒绝。quick-xml 本就不解外部实体、不触网，这里只需保证
/// 解析器不会因为 DOCTYPE 而改变行为。注意：这是 XXE 加固，不是 SSRF 防御 ——
/// 本工具单机运行、无出站网络栈，不需要按“云元数据窃取”威胁模型做额外处理。
/// 用内网元数据 URL 做输入，只是为了拿一个典型的危险 SYSTEM 值。
#[test]
fn doctype_with_external_system_uri_is_rejected() {
    let xml = r#"<!DOCTYPE BDN SYSTEM "http://169.254.169.254/latest/meta-data">
<BDN Version="0.93"/></!DOCTYPE>"#;
    let e = codec_bdn::parse(xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0051");
}

#[test]
fn undefined_entity_in_text_is_rejected() {
    let xml = sample_xml().replace("<Name>中文字幕 &amp; 测试</Name>", "<Name>&xxe;</Name>");
    assert_eq!(code_of(&xml), "STK-BDN-0051");
}

#[test]
fn undefined_entity_in_attribute_is_rejected() {
    let xml = sample_xml().replace("Version=\"0.93\"", "Version=\"&evil;\"");
    let e = codec_bdn::parse(&xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0056");
}

#[test]
fn unescaped_ampersand_is_error_not_panic() {
    let xml = sample_xml().replace("中文字幕 &amp; 测试", "中文字幕 & 测试");
    let e = codec_bdn::parse(&xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0052");
    assert!(
        e.details
            .iter()
            .any(|d| d.contains("元素路径") || d.contains("偏移")),
        "语法错误也要给位置：{:?}",
        e.details
    );
}

/// 未闭合的 `<Event>` 有两种表现，都必须报错且给位置：
/// - 后一个 Event 先来 → 被判成"事件嵌套事件"（0054）；
/// - 父标签先来收掉它 → 结束标签不匹配（0052，XML 良构性）。
#[test]
fn unclosed_tag_is_error_with_position() {
    let nested = sample_xml().replace("</Event>", "");
    let e = codec_bdn::parse(&nested).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0054");
    assert!(!e.details.is_empty(), "结构错误也要给位置");

    let mismatched = r#"<BDN Version="0.93"><BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>a.png</FileName><Width>100</Width><Height>50</Height><X>0</X><Y>0</Y></Graphic>
</BDNSubtitleData></BDN>"#;
    let e = codec_bdn::parse(mismatched).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0052");
    assert!(
        e.details.iter().any(|d| d.contains("</BDNSubtitleData>")),
        "{:?}",
        e.details
    );
}

#[test]
fn stray_end_tag_and_multi_root_are_rejected() {
    assert_eq!(
        code_of("<BDN Version=\"0.93\"></BDN></BDN>"),
        "STK-BDN-0052"
    );
    assert_eq!(code_of("<SRT></SRT>"), "STK-BDN-0053");
    assert_eq!(code_of(""), "STK-BDN-0053");
    assert_eq!(code_of("<BDN Version=\"0.93\">"), "STK-BDN-0052");
}

#[test]
fn non_utf8_input_is_rejected_not_panicking() {
    let mut bytes = sample_xml().into_bytes();
    // 在 Name 文本里插一个非法 UTF-8 序列。
    let pos = bytes.len() / 2;
    bytes.splice(pos..pos, [0xFF, 0xFE, 0x80]);
    let e = codec_bdn::parse_bytes(&bytes).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0058");
}

#[test]
fn oversized_document_reports_limit_instead_of_oom() {
    let big = format!("<!--{}-->", "x".repeat(4096));
    let xml = format!("{big}{}", sample_xml());
    let lim = Limits {
        doc_bytes: 2048,
        ..Default::default()
    };
    let e = codec_bdn::parse_with(&xml, &lim).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0050");
    assert!(e.details.join(" ").contains("文档字节数"));
}

#[test]
fn oversized_attribute_value_reports_limit() {
    let long = "y".repeat(300);
    let xml = format!(
        "<BDN Version=\"0.93\" Name=\"{long}\"><BDNSubtitleData><Event InTC=\"00:00:01:00\" OutTC=\"00:00:02:00\">\
         <Graphic><FileName>a.png</FileName><Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic></Event>\
         </BDNSubtitleData></BDN>"
    );
    let lim = Limits {
        text_bytes: 128,
        ..Default::default()
    };
    let e = codec_bdn::parse_with(&xml, &lim).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0050");
    // 默认额度也必须容得下真实长路径。
    assert!(codec_bdn::parse(&xml).is_ok());
}

#[test]
fn oversized_text_node_reports_limit() {
    let xml = format!(
        "<BDN Version=\"0.93\"><BDNSubtitleData><Event InTC=\"00:00:01:00\" OutTC=\"00:00:02:00\">\
         <Graphic><FileName>{}.png</FileName><Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic></Event>\
         </BDNSubtitleData></BDN>",
        "z".repeat(200)
    );
    let lim = Limits {
        text_bytes: 64,
        ..Default::default()
    };
    let e = codec_bdn::parse_with(&xml, &lim).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0050");
    // 实体分段的超长文本也要被累计拦下（单段都不超限，累计才超）。
    let chunked = format!(
        "<BDN Version=\"0.93\"><BDNSubtitleData><Event InTC=\"00:00:01:00\" OutTC=\"00:00:02:00\">\
         <Graphic><FileName>{}a{}b{}c{}d</FileName><Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic></Event>\
         </BDNSubtitleData></BDN>",
        "z".repeat(30),
        "&amp;",
        "z".repeat(30),
        "&amp;"
    );
    assert_eq!(
        codec_bdn::parse_with(&chunked, &lim)
            .unwrap_err()
            .code_string(),
        "STK-BDN-0050"
    );
}

#[test]
fn oversized_element_name_reports_limit() {
    let xml = format!(
        "<BDN Version=\"0.93\"><{}>x</{}></BDN>",
        "q".repeat(80),
        "q".repeat(80)
    );
    let lim = Limits {
        name_bytes: 32,
        ..Default::default()
    };
    assert_eq!(
        codec_bdn::parse_with(&xml, &lim).unwrap_err().code_string(),
        "STK-BDN-0050"
    );
}

#[test]
fn event_count_limit_is_enforced_during_streaming() {
    let one =
        "<Event InTC=\"00:00:01:00\" OutTC=\"00:00:02:00\"><Graphic><FileName>a.png</FileName>\
               <Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic></Event>";
    let xml =
        format!("<BDN Version=\"0.93\"><BDNSubtitleData>{one}{one}{one}</BDNSubtitleData></BDN>");
    let lim = Limits {
        events: 2,
        ..Default::default()
    };
    assert_eq!(
        codec_bdn::parse_with(&xml, &lim).unwrap_err().code_string(),
        "STK-BDN-0050"
    );
    assert_eq!(codec_bdn::parse(&xml).unwrap().events.len(), 3);
}

#[test]
fn deep_nesting_is_rejected_before_stack_overflow() {
    let depth = 200;
    let mut xml = String::from("<BDN Version=\"0.93\">");
    for _ in 0..depth {
        xml.push_str("<n>");
    }
    for _ in 0..depth {
        xml.push_str("</n>");
    }
    xml.push_str("</BDN>");
    let e = codec_bdn::parse(&xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0050");
    assert!(e.details.join(" ").contains("嵌套深度"));
}

#[test]
fn nested_event_or_misplaced_element_is_rejected_with_path() {
    let xml = r#"<BDN Version="0.93"><BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Event InTC="00:00:03:00" OutTC="00:00:04:00"/></Event></BDNSubtitleData></BDN>"#;
    let e = codec_bdn::parse(xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0054");
    assert!(e.details.join(" ").contains("BDNSubtitleData/Event"));
}

#[test]
fn two_graphics_in_one_event_are_rejected_not_silently_merged() {
    // 静默合并/最后一个赢会让"声明尺寸"与某个 PNG 不再一一对应（缺陷二的变体）。
    let xml = r#"<BDN Version="0.93"><BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>a.png</FileName><Width>100</Width><Height>50</Height><X>0</X><Y>0</Y></Graphic>
<Graphic><FileName>b.png</FileName><Width>200</Width><Height>60</Height><X>0</X><Y>0</Y></Graphic>
</Event></BDNSubtitleData></BDN>"#;
    let e = codec_bdn::parse(xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0057");
}

#[test]
fn missing_required_fields_report_which_field_and_where() {
    let xml = r#"<BDN Version="0.93"><BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>a.png</FileName><Width>100</Width></Graphic></Event></BDNSubtitleData></BDN>"#;
    let e = codec_bdn::parse(xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0055");
    // 22 §错误模型：用户要能看到"缺哪个字段 + 在哪"。标题给字段名，detail 给元素路径与偏移。
    let mut joined = e.title.clone();
    joined.push(' ');
    joined.push_str(&e.details.join(" "));
    assert!(joined.contains("Height"), "{joined}");
    assert!(joined.contains("BDNSubtitleData/Event"), "{joined}");
}

#[test]
fn graphic_outside_canvas_is_rejected_on_read() {
    let xml = r#"<BDN Version="0.93">
<BDNFormat><DisplayResolution><Width>1920</Width><Height>1080</Height></DisplayResolution></BDNFormat>
<BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>a.png</FileName><Width>1900</Width><Height>50</Height><X>100</X><Y>1060</Y></Graphic>
</Event></BDNSubtitleData></BDN>"#;
    let e = codec_bdn::parse(xml).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0063");
}

/// fuzz 前提：任意字节/任意残缺输入只能返回错误，绝不能 panic。
#[test]
fn mutated_and_truncated_inputs_never_panic() {
    let base = sample_xml();
    let mut ok = 0usize;
    let mut err = 0usize;
    for cut in (0..base.len()).step_by(37) {
        let s: String = base.chars().take(cut / 3).collect();
        match codec_bdn::parse(&s) {
            Ok(_) => ok += 1,
            Err(e) => {
                err += 1;
                assert!(e.code_string().starts_with("STK-BDN-00"));
            }
        }
    }
    // 单字节破坏：只关心"不 panic"。
    let bytes = base.as_bytes();
    let mut mutations = 0usize;
    for i in (0..bytes.len()).step_by(29) {
        for mask in [0xFFu8, 0x00, 0x41, 0x80] {
            let mut v = bytes.to_vec();
            v[i] = mask;
            mutations += 1;
            let _ = codec_bdn::parse_bytes(&v);
        }
    }
    assert!(
        err > 0 && mutations > 20,
        "ok={ok} err={err} mutations={mutations}"
    );
}

/// 默认 4096 字节的文本额度不能误杀真实长路径：超长中文 UNC 路径也在额度内。
/// （审查怀疑这个上限会挑真实文件，这里把它锁成一个可执行合同。）
#[test]
fn default_text_limit_admits_a_very_long_path() {
    let deep = format!("{}\\subtitle.png", "\\长目录名".repeat(90));
    assert!(
        deep.len() > 1024,
        "前提：构造出的路径确实很长：{}",
        deep.len()
    );
    assert!(deep.len() < TEXT_LEN_HARD_MAX, "构造成本超限，测试无意义");
    let xml = format!(
        "<BDN Version=\"0.93\"><BDNSubtitleData><Event InTC=\"00:00:01:00\" OutTC=\"00:00:02:00\">\
         <Graphic><FileName>{deep}</FileName><Width>10</Width><Height>10</Height><X>0</X><Y>0</Y></Graphic></Event>\
         </BDNSubtitleData></BDN>"
    );
    let doc = codec_bdn::parse(&xml).expect("默认额度必须容得下真实长路径");
    assert_eq!(doc.events.len(), 1);
}

/// 控制字符不能进 XML 字符串字段（写出后会破坏往返）。
#[test]
fn control_characters_in_strings_are_rejected_on_write() {
    use core_model::time::{Fps, Time};
    let fps = Fps::PAL_25;
    let g = codec_bdn::BdnGraphic::new("a\u{7}b.png", 100, 50, 0, 0).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        Time::from_millis(0).unwrap(),
        Time::from_millis(500).unwrap(),
        g,
        fps,
    )
    .unwrap();
    let mut doc = common::doc(fps, &[0]);
    doc.events = vec![ev];
    let e = codec_bdn::write(&doc).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0068");

    let mut d = common::doc(fps, &[0]);
    d.meta.language = "ch\u{0}i".to_string();
    assert_eq!(
        codec_bdn::write(&d).unwrap_err().code_string(),
        "STK-BDN-0068"
    );

    let mut d2 = common::doc(fps, &[0]);
    d2.meta.name = "line1\nline2".to_string();
    assert_eq!(
        codec_bdn::write(&d2).unwrap_err().code_string(),
        "STK-BDN-0068"
    );
}
