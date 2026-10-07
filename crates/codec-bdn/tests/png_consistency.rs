//! PNG 尺寸一致性（13 §7 / AT-BDN-001）——旧工程缺陷二"XML 声明的 Graphic Width/Height 与
//! PNG 实际尺寸不一致"的反例集合，含 `min_axis` 语义决定的直接验证。

use std::path::Path;

use codec_bdn::{
    build_document, build_document_with_loss, check_png_size, png_size, validate_dir, BdnFrame,
    BdnGraphic, BdnOptions,
};
use core_model::time::{Fps, Time};

mod common;
use common::{meta, png};

const FPS: Fps = Fps::NTSC_23976;

fn frame(name: &str, w: u32, h: u32, x: i32, y: i32, start_ms: i64) -> BdnFrame {
    BdnFrame {
        start: Time::from_millis(start_ms).unwrap(),
        end: Time::from_millis(start_ms + 800).unwrap(),
        png_file_name: name.to_string(),
        png_width: w,
        png_height: h,
        x,
        y,
    }
}

fn opts(min_axis: u32) -> BdnOptions {
    BdnOptions {
        fps: FPS,
        language: "chi".to_string(),
        name: "n".to_string(),
        min_axis,
        offset_x: 0,
        offset_y: 0,
    }
}

#[test]
fn check_png_size_accepts_matching_declaration() {
    assert!(check_png_size(&png(100, 50), 100, 50).is_ok());
    let s = png_size(&png(1920, 1080)).unwrap();
    assert_eq!((s.width, s.height), (1920, 1080));
}

#[test]
fn declared_100x50_but_png_is_120x50_is_rejected() {
    let e = check_png_size(&png(120, 50), 100, 50).unwrap_err();
    assert_eq!(e.code_string(), "STK-BDN-0073");
    let d = e.details.join(" ");
    assert!(
        d.contains("声明 100x50") && d.contains("实际 120x50"),
        "{d}"
    );
}

#[test]
fn bad_signature_and_truncated_headers_are_rejected_without_panic() {
    assert_eq!(
        png_size(b"GIF89aAAAAAAAAAAAAAAAA")
            .unwrap_err()
            .code_string(),
        "STK-BDN-0070"
    );
    assert_eq!(png_size(&[]).unwrap_err().code_string(), "STK-BDN-0071");
    assert_eq!(
        png_size(&png(100, 50)[..20]).unwrap_err().code_string(),
        "STK-BDN-0071"
    );
    assert_eq!(
        png_size(&common::png_truncated(100, 50, 14))
            .unwrap_err()
            .code_string(),
        "STK-BDN-0071"
    );
}

#[test]
fn wrong_ihdr_chunk_type_or_length_is_rejected() {
    let mut b = png(100, 50);
    b[12..16].copy_from_slice(b"tEXt");
    assert_eq!(png_size(&b).unwrap_err().code_string(), "STK-BDN-0072");
    let mut b = png(100, 50);
    b[8..12].copy_from_slice(&12u32.to_be_bytes());
    assert_eq!(png_size(&b).unwrap_err().code_string(), "STK-BDN-0072");
}

#[test]
fn absurd_png_dimensions_are_rejected() {
    assert_eq!(
        png_size(&png(0, 50)).unwrap_err().code_string(),
        "STK-BDN-0074"
    );
    assert_eq!(
        png_size(&png(100_000, 50)).unwrap_err().code_string(),
        "STK-BDN-0074"
    );
    // 声明 0 边的 graphic 根本构造不出来。
    assert_eq!(
        BdnGraphic::new("x.png", 0, 50, 0, 0)
            .unwrap_err()
            .code_string(),
        "STK-BDN-0064"
    );
}

/// 非调色板（灰度 / RGB / RGBA / 隔行）都能只读 IHDR 拿到尺寸——不依赖图片库。
#[test]
fn non_palette_color_types_are_supported() {
    for ct in [0u8, 2, 3, 4, 6] {
        let mut b = png(64, 32);
        b[25] = ct; // IHDR data[1] = color type
        assert_eq!(png_size(&b).unwrap().width, 64, "color type {ct}");
        assert_eq!(png_size(&b).unwrap().height, 32, "color type {ct}");
    }
}

/// 阈值语义：宽扁的正常字幕条不得被当成小对象丢弃（只有最长边也小于阈值才丢）。
#[test]
fn thin_subtitle_strip_is_not_dropped_by_min_axis() {
    let strip = frame("strip.png", 900, 40, 510, 980, 1000);
    let doc = build_document(&[strip], &opts(320)).unwrap();
    assert_eq!(doc.events.len(), 1, "900x40 的字幕条不是小对象");
    assert_eq!(
        (doc.events[0].graphic.width, doc.events[0].graphic.height),
        (900, 40)
    );
}

/// 缺陷二反例（核心）：`min_axis` 是过滤阈值，绝不把声明尺寸抬到 min_axis。
#[test]
fn min_axis_is_only_a_filter_and_never_rewrites_declared_png_size() {
    let small = frame("small.png", 100, 50, 10, 20, 1000);
    let big = frame("big.png", 800, 600, 560, 200, 3000);
    // min_axis = 320：short edge 小于它的对象被丢弃；留下的对象尺寸必须原样。
    let doc = build_document(&[small.clone(), big.clone()], &opts(320)).unwrap();
    assert_eq!(doc.events.len(), 1, "只有小对象被丢弃");
    let g = &doc.events[0].graphic;
    assert_eq!((g.width, g.height), (800, 600), "尺寸必须仍是 PNG 实际尺寸");
    assert_ne!(g.width, 320);
    assert_ne!(g.height, 320);

    let xml = codec_bdn::write(&doc).unwrap();
    assert!(
        xml.contains("<Width>800</Width><Height>600</Height>"),
        "{xml}"
    );
    assert!(
        !xml.contains("320"),
        "min_axis 绝不该出现在写出的 XML 里：{xml}"
    );

    // 旧缺陷写法会把 100x50 抬成 320x320；这里必须保持 100x50（min_axis=0 时谁都不丢）。
    let doc2 = build_document(&[small, big], &opts(0)).unwrap();
    assert_eq!(doc2.events.len(), 2);
    assert_eq!(
        (doc2.events[0].graphic.width, doc2.events[0].graphic.height),
        (100, 50),
        "min_axis 不能改写 Width/Height"
    );
    let xml2 = codec_bdn::write(&doc2).unwrap();
    assert!(
        xml2.contains("<Width>100</Width><Height>50</Height>"),
        "{xml2}"
    );
}

/// `min_axis` 丢弃要能在 Loss Model 里看到（不允许静默掉帧）。
#[test]
fn min_axis_drops_are_reported_as_dropped() {
    let small = frame("small.png", 100, 50, 10, 20, 1000);
    let big = frame("big.png", 800, 600, 560, 200, 3000);
    let (doc, loss) = build_document_with_loss(&[small, big], &opts(320)).unwrap();
    assert_eq!(doc.events.len(), 1);
    assert_eq!(loss.count_for("dropped", "小于 min_axis 320 的小对象"), 1);
    assert!(!loss.is_lossless());
    assert!(
        loss.preserved.iter().any(|p| p.contains("PNG 实际尺寸")),
        "{:?}",
        loss.preserved
    );
}

/// 端到端：写出的 XML 声明尺寸与真实落盘的 PNG 一致。
#[test]
fn written_document_matches_real_pngs_on_disk() {
    let dir = temp_dir("real-png");
    std::fs::create_dir_all(&dir).unwrap();
    let mut frames = Vec::new();
    for i in 0..3i64 {
        let name = format!("字幕{i}.png");
        let (w, h) = (
            if i == 1 { 640 } else { 800 },
            if i == 1 { 90 } else { 120 },
        );
        std::fs::write(dir.join(&name), png(w as u32, h as u32)).unwrap();
        frames.push(frame(
            &name,
            w as u32,
            h as u32,
            560,
            880 - 200 * i as i32,
            i * 2000 + 1000,
        ));
    }
    let doc = build_document(&frames, &opts(32)).unwrap();
    let report = validate_dir(&doc, Path::new(&dir)).unwrap();
    assert!(report.is_clean(), "{}", report.summary());
    assert_eq!(report.checked, 3);

    // 把一个 PNG 换成更大尺寸 → 必须报 0073。
    std::fs::write(dir.join("字幕1.png"), png(1024, 90)).unwrap();
    let bad = validate_dir(&doc, Path::new(&dir)).unwrap();
    assert_eq!(bad.problems.len(), 1);
    assert_eq!(bad.problems[0].error.code_string(), "STK-BDN-0073");
    assert_eq!(bad.problems[0].event, 1);
}

/// XML 与 PNG 一起交付才算完整（缺失文件必须报 0075）。
#[test]
fn missing_png_is_reported_not_silently_ok() {
    let dir = temp_dir("missing-png");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.png"), png(100, 50)).unwrap();
    let doc = common::doc(FPS, &[1000, 2000]);
    let r = validate_dir(&doc, Path::new(&dir)).unwrap();
    assert_eq!(r.checked, 2);
    assert_eq!(r.problems.len(), 2);
    for p in &r.problems {
        assert_eq!(p.error.code_string(), "STK-BDN-0075");
    }
}

#[test]
fn meta_helper_is_internally_consistent() {
    let m = meta(FPS);
    assert_eq!(m.fps().unwrap().to_label(), "23.976");
    assert_eq!(m.canvas().unwrap().label(), "1920x1080");
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "stk-bdn-{tag}-{}-{}",
        std::process::id(),
        tag.len()
    ));
    let _ = std::fs::remove_dir_all(&p);
    p
}
