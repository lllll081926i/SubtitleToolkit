//! round-trip：parse → write → parse 语义等价（时间用 `Time` 比对，不比对空白/缩进）。

use codec_bdn::{BdnDocument, BdnMeta};
use core_model::time::{Fps, Rational};

mod common;
use common::{meta, png, sample_xml, sample_xml_dialect_attrs};

const FPS: Fps = Fps::NTSC_23976;

/// 语义等价的比较面：时间 + 几何 + 字符串字段；不含 XML 空白。
fn semantic(doc: &BdnDocument) -> Vec<String> {
    let mut v = vec![format!(
        "meta|{}|{}|{}|{}|{}|{}|{}x{}",
        doc.meta.version,
        doc.meta.name,
        doc.meta.language,
        doc.meta.video_format,
        doc.meta.frame_rate,
        doc.meta.drop_frame,
        doc.meta.width,
        doc.meta.height
    )];
    for e in &doc.events {
        v.push(format!(
            "ev|{}|{}|{}|{}|{}|{}|{}|{}",
            e.in_tc,
            e.out_tc,
            e.start.as_rational(),
            e.end.as_rational(),
            e.graphic.file_name,
            e.graphic.width,
            e.graphic.height,
            e.graphic.x
        ));
    }
    v
}

#[test]
fn parse_write_parse_is_semantically_equal() {
    let once = codec_bdn::parse(&sample_xml()).unwrap();
    let xml = codec_bdn::write(&once).unwrap();
    let twice = codec_bdn::parse(&xml).unwrap();
    assert_eq!(semantic(&once), semantic(&twice));
    assert_eq!(once.events.len(), 2);
}

#[test]
fn write_output_is_byte_stable_on_reroundtrip() {
    let d = codec_bdn::parse(&sample_xml()).unwrap();
    let a = codec_bdn::write(&d).unwrap();
    let b = codec_bdn::write(&codec_bdn::parse(&a).unwrap()).unwrap();
    assert_eq!(a, b, "写出必须是自身不动点，否则往返会引入漂移");
}

#[test]
fn frame_rate_label_returns_exact_rational_not_decimal() {
    let d = codec_bdn::parse(&sample_xml()).unwrap();
    assert_eq!(
        d.meta.frame_rate,
        Rational::new(24000, 1001).unwrap(),
        "\"23.976\" 必须回到 24000/1001"
    );
    assert_ne!(d.meta.frame_rate, Rational::parse("23.976").unwrap());
}

#[test]
fn framerate_25_is_integer_rational() {
    let xml = sample_xml().replace("<FrameRate>23.976</FrameRate>", "<FrameRate>25</FrameRate>");
    let d = codec_bdn::parse(&xml).unwrap();
    assert_eq!(d.meta.frame_rate, Rational::from_int(25));
    // 25 fps 下 3.5 s 落在帧 12/25 之外，时间码必须是 :12 或 :13 之一（合法 FF）。
    let out = codec_bdn::write(&d).unwrap();
    assert!(out.contains("FrameRate>25<"), "{out}");
}

#[test]
fn arbitrary_version_and_language_are_preserved() {
    let xml = sample_xml()
        .replace("Version=\"0.93\"", "Version=\"3_r14\"")
        .replace("Xml:lang=\"chi\"", "Xml:lang=\"zho-Hans\"");
    let d = codec_bdn::parse(&xml).unwrap();
    assert_eq!(d.meta.version, "3_r14");
    assert_eq!(d.meta.language, "zho-Hans");
    let written = codec_bdn::write(&d).unwrap();
    assert!(written.contains("Version=\"3_r14\""));
    assert!(written.contains("Xml:lang=\"zho-Hans\""));
    assert_eq!(
        codec_bdn::parse(&written).unwrap().meta.language,
        "zho-Hans"
    );
}

#[test]
fn root_and_attribute_case_and_whitespace_are_tolerated() {
    let d = codec_bdn::parse(&sample_xml_dialect_attrs()).unwrap();
    assert_eq!(d.events.len(), 1);
    assert_eq!(d.events[0].graphic.file_name, "a b.png");
    assert_eq!(d.events[0].graphic.width, 800);
    assert_eq!(d.events[0].graphic.height, 120);
    assert_eq!(d.events[0].in_tc.to_string(), "00:00:01:00");
    // 缺 BDNFormat 时用默认 1080p 画布，但不会伪造 FrameRate 之外的信息。
    assert_eq!((d.meta.width, d.meta.height), (1920, 1080));
    assert_eq!(d.meta.version, "0_r22");
}

#[test]
fn graphic_child_element_dialect_and_bare_text_dialect_both_parse() {
    let elem = r#"<BDN Version="0.93"><BDNSubtitleData>
<Event InTC="00:00:01:00" OutTC="00:00:02:00"><Graphic><FileName>x.png</FileName>
<Width>640</Width><Height>80</Height><X>640</X><Y>900</Y></Graphic></Event>
</BDNSubtitleData></BDN>"#;
    let text = r#"<BDN Version="0.93"><BDNSubtitleData>
<Event InTC="00:00:01:00" OutTC="00:00:02:00"><Graphic Width="640" Height="80" X="640" Y="900">x.png</Graphic></Event>
</BDNSubtitleData></BDN>"#;
    let a = codec_bdn::parse(elem).unwrap();
    let b = codec_bdn::parse(text).unwrap();
    assert_eq!(a.events[0].graphic, b.events[0].graphic);
}

#[test]
fn horizontal_offset_dialect_fills_x_when_absent() {
    let xml = r#"<BDN Version="0.93"><BDNSubtitleData>
<Event InTC="00:00:01:00" OutTC="00:00:02:00"><Graphic>
<FileName>h.png</FileName><Width>100</Width><Height>50</Height>
<HorizontalOffset>300</HorizontalOffset><VerticalOffset>700</VerticalOffset>
</Graphic></Event></BDNSubtitleData></BDN>"#;
    let d = codec_bdn::parse(xml).unwrap();
    assert_eq!((d.events[0].graphic.x, d.events[0].graphic.y), (300, 700));
}

#[test]
fn escaped_special_characters_survive_roundtrip() {
    let meta = meta(FPS);
    let g = codec_bdn::BdnGraphic::new("a & <b> \"c\".png", 800, 120, 560, 880).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        core_model::time::Time::from_millis(1000).unwrap(),
        core_model::time::Time::from_millis(2000).unwrap(),
        g,
        FPS,
    )
    .unwrap();
    let d = BdnDocument::new(
        BdnMeta {
            name: "R&D <字幕>".to_string(),
            ..meta
        },
        vec![ev],
    );
    let xml = codec_bdn::write(&d).unwrap();
    assert!(xml.contains("a &amp; &lt;b&gt; &quot;c&quot;.png"), "{xml}");
    assert!(xml.contains("R&amp;D &lt;字幕&gt;"));
    let back = codec_bdn::parse(&xml).unwrap();
    assert_eq!(back.events[0].graphic.file_name, "a & <b> \"c\".png");
    assert_eq!(back.meta.name, "R&D <字幕>");
}

#[test]
fn declared_png_size_is_written_verbatim() {
    // 缺陷二反例（写出侧）：声明尺寸必须逐字等于 graphic 的字段（也就是 PNG 实际尺寸）。
    let xml = codec_bdn::write(&common::doc(FPS, &[1000])).unwrap();
    let d = codec_bdn::parse(&xml).unwrap();
    assert_eq!(
        (d.events[0].graphic.width, d.events[0].graphic.height),
        (800, 120)
    );
    let mut m = meta(FPS);
    m.width = 1920;
    m.height = 1080;
    let g = codec_bdn::BdnGraphic::new("tiny.png", 1, 1, 0, 0).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        core_model::time::Time::from_millis(0).unwrap(),
        core_model::time::Time::from_millis(500).unwrap(),
        g,
        FPS,
    )
    .unwrap();
    let out = codec_bdn::write(&BdnDocument::new(m, vec![ev])).unwrap();
    assert!(out.contains("<Width>1</Width><Height>1</Height>"), "{out}");
    assert!(!out.contains("<Width>320</Width>"));
}

#[test]
fn serde_json_roundtrip_of_document() {
    let d = codec_bdn::parse(&sample_xml()).unwrap();
    let json = serde_json::to_string(&d).unwrap();
    let back: BdnDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(d, back);
}

#[test]
fn png_helper_itself_is_consistent() {
    let bytes = png(123, 45);
    assert!(codec_bdn::check_png_size(&bytes, 123, 45).is_ok());
}
