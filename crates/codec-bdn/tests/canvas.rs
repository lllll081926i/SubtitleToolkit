//! 画布与坐标（09 §1/§3、13 §7）：x/y 属于完整 16:9 画布；2160p 只能 custom/exotic。

use codec_bdn::{
    build_document, build_document_on_canvas, build_document_with_loss, loss_of, BdnFrame,
    BdnGraphic, BdnMeta, BdnOptions,
};
use core_model::canvas::{Canvas, BLURAY_MAX};
use core_model::time::{Fps, Time};

mod common;
use common::meta;

const FPS: Fps = Fps::NTSC_23976;

fn frame(name: &str, w: u32, h: u32, x: i32, y: i32, ms: i64) -> BdnFrame {
    BdnFrame {
        start: Time::from_millis(ms).unwrap(),
        end: Time::from_millis(ms + 500).unwrap(),
        png_file_name: name.to_string(),
        png_width: w,
        png_height: h,
        x,
        y,
    }
}

fn opt(offset_x: i32, offset_y: i32) -> BdnOptions {
    BdnOptions {
        fps: FPS,
        language: "chi".to_string(),
        name: "n".to_string(),
        min_axis: 0,
        offset_x,
        offset_y,
    }
}

#[test]
fn offset_is_applied_and_must_stay_inside_full_canvas() {
    let f = frame("a.png", 800, 120, 560, 880, 1000);
    let d = build_document(std::slice::from_ref(&f), &opt(0, 0)).unwrap();
    assert_eq!((d.events[0].graphic.x, d.events[0].graphic.y), (560, 880));
    let d2 = build_document(&[f], &opt(100, 50)).unwrap();
    assert_eq!((d2.events[0].graphic.x, d2.events[0].graphic.y), (660, 930));
    assert_eq!((d2.meta.width, d2.meta.height), (1920, 1080));
}

#[test]
fn offset_pushing_object_off_canvas_is_rejected() {
    let f = frame("a.png", 800, 120, 560, 880, 1000);
    // 右边越界
    let e = build_document(std::slice::from_ref(&f), &opt(600, 0)).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0063");
    // 下边越界
    assert_eq!(
        build_document(std::slice::from_ref(&f), &opt(0, 90))
            .unwrap_err()
            .code_string(),
        "STK-BDN-0063"
    );
    // 负偏移越出左上角
    assert_eq!(
        build_document(&[f], &opt(-600, 0))
            .unwrap_err()
            .code_string(),
        "STK-BDN-0063"
    );
}

#[test]
fn offset_overflow_is_checked_not_wrapped() {
    let f = frame("a.png", 100, 50, i32::MAX, 0, 1000);
    let e = build_document(&[f], &opt(i32::MAX, 0)).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0063");
    let g = BdnGraphic::new("a.png", 100, 50, i32::MAX, i32::MAX).unwrap();
    assert_eq!(
        g.fits_in(Canvas::FHD1080).unwrap_err().code_string(),
        "STK-BDN-0063"
    );
}

#[test]
fn explicit_2160_canvas_is_flagged_custom_exotic() {
    // 2160p 项目必须显式给画布（不靠猜）；给了就必须标 custom/exotic。
    let f = frame("uhd.png", 2000, 400, 900, 1600, 1000);
    let (doc, loss) = build_document_on_canvas(&[f], &opt(0, 0), Canvas::UHD2160).unwrap();
    assert_eq!((doc.meta.width, doc.meta.height), (3840, 2160));
    assert_eq!(doc.meta.video_format, "2160p");
    assert!(
        loss.nonstandard.iter().any(|n| n.contains("custom/exotic")),
        "{:?}",
        loss.nonstandard
    );

    let l = loss_of(&doc);
    assert!(
        l.nonstandard
            .iter()
            .any(|n| n.contains("2160") || n.contains("custom/exotic")),
        "{:?}",
        l.nonstandard
    );
    assert!(
        l.user_attention.iter().any(|u| u.contains("Blu-ray")),
        "{:?}",
        l.user_attention
    );
    assert!(!l.is_lossless());
}

#[test]
fn object_wider_than_the_canvas_is_rejected_instead_of_rescaled() {
    let f = frame("huge.png", 6000, 400, 0, 0, 1000);
    assert_eq!(
        build_document(std::slice::from_ref(&f), &opt(0, 0))
            .unwrap_err()
            .code_string(),
        "STK-BDN-0063"
    );
    // 升到 2160p 也放不下，仍然是错误（绝不静默缩放坐标或尺寸）。
    assert_eq!(
        build_document_on_canvas(&[f], &opt(0, 0), Canvas::UHD2160)
            .unwrap_err()
            .code_string(),
        "STK-BDN-0063"
    );
}

#[test]
fn build_document_defaults_to_bluray_max_canvas() {
    let f = frame("a.png", 800, 120, 560, 880, 1000);
    let (doc, loss) = build_document_with_loss(&[f], &opt(0, 0)).unwrap();
    assert_eq!((doc.meta.width, doc.meta.height), (1920, 1080));
    assert!(loss.nonstandard.is_empty(), "{:?}", loss.nonstandard);
    // 720p 也必须显式声明；非 16:9 直接拒绝。
    assert!(build_document_on_canvas(&[], &opt(0, 0), Canvas::HD720).is_ok());
    assert_eq!(
        build_document_on_canvas(
            &[],
            &opt(0, 0),
            Canvas {
                width: 1920,
                height: 800
            }
        )
        .unwrap_err()
        .code_string(),
        "STK-INP-0031"
    );
}

#[test]
fn blueray_max_is_the_reference_for_1080p_compliance() {
    assert_eq!(BLURAY_MAX, Canvas::FHD1080);
    let mut m = meta(FPS);
    m.width = 3840;
    m.height = 2160;
    m.video_format = "2160p".to_string();
    let doc = codec_bdn::BdnDocument::new(m, Vec::new());
    let l = loss_of(&doc);
    assert!(!l.is_lossless(), "2160p BDN 永远不该被报告为无损合规输出");
    // 文档本身仍可写出（16:9），只是带 custom/exotic 标签。
    assert!(codec_bdn::write(&doc).is_ok());
}

#[test]
fn sd_non_16_9_document_can_be_read_but_is_flagged_and_not_written() {
    // 09 §1：输出永远是完整 16:9；SD(4:3) 的 BDN 允许读入做分析，但拒绝写出。
    let xml = r#"<BDN Version="0.93"><BDNFormat><VideoFormat>480p</VideoFormat><FrameRate>25</FrameRate>
<DisplayResolution><Width>720</Width><Height>480</Height></DisplayResolution></BDNFormat>
<BDNSubtitleData><Event InTC="00:00:01:00" OutTC="00:00:02:00">
<Graphic><FileName>a.png</FileName><Width>600</Width><Height>80</Height><X>60</X><Y>380</Y></Graphic>
</Event></BDNSubtitleData></BDN>"#;
    let doc = codec_bdn::parse(xml).unwrap();
    assert_eq!((doc.meta.width, doc.meta.height), (720, 480));
    let l = loss_of(&doc);
    assert!(
        l.nonstandard.iter().any(|n| n.contains("16:9")),
        "{:?}",
        l.nonstandard
    );
    assert_eq!(
        codec_bdn::write(&doc).unwrap_err().code_string(),
        "STK-BDN-0065"
    );
}

#[test]
fn loss_of_reports_timecode_quantization_as_approximated() {
    // 1234 ms 不在 23.976 的 1/24 s 时间码网格上 → 必须报告"近似"，而不是假装无损。
    let mut m: BdnMeta = meta(FPS);
    m.language = "chi".to_string();
    let g = BdnGraphic::new("q.png", 100, 50, 0, 0).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        Time::from_millis(1234).unwrap(),
        Time::from_millis(2000).unwrap(),
        g,
        FPS,
    )
    .unwrap();
    let doc = codec_bdn::BdnDocument::new(m, vec![ev]);
    let l = loss_of(&doc);
    assert!(
        l.approximated.iter().any(|a| a.contains("帧网格")),
        "{:?}",
        l.approximated
    );
    assert_eq!(l.count_for("user_attention", "事件时间不落在帧网格上"), 1);
}

#[test]
fn loss_of_1080p_grid_aligned_document_keeps_png_semantics_preserved() {
    let doc = common::doc(FPS, &[1000, 2000, 3000]);
    let l = loss_of(&doc);
    assert!(
        l.preserved.iter().any(|p| p.contains("Width/Height")),
        "{:?}",
        l.preserved
    );
    assert!(
        !l.nonstandard.iter().any(|n| n.contains("16:9")),
        "{:?}",
        l.nonstandard
    );
}
