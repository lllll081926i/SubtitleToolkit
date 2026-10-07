//! 端到端（13 §7 / AT-BDN-001 / 09 §3）：渲染位图 → bbox 裁剪 → PNG → BDN XML →
//! 按目录真实复核。核心断言是"XML 声明尺寸 == PNG IHDR 实际尺寸"，
//! 且 `min_axis` 只过滤、绝不抬尺寸（旧工程缺陷）。

use codec_bdn::{build_document, check_png_size, parse, validate_dir, write, BdnFrame, BdnOptions};
use core_model::canvas::Canvas;
use core_model::time::{Fps, Time};
use image_core::{header, Frame, Level};

const FPS: Fps = Fps::NTSC_23976;

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("stk-img-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn options(min_axis: u32) -> BdnOptions {
    BdnOptions {
        fps: FPS,
        language: "chi".to_string(),
        name: "image-core".to_string(),
        min_axis,
        offset_x: 0,
        offset_y: 0,
    }
}

/// 一帧"字幕"：1920x1080 画布上两条不连续的实心带，逼出真实的 bbox 与局部 PNG。
fn subtitle_frame() -> Frame {
    let c = Canvas::FHD1080;
    let mut f = Frame::empty(c).unwrap();
    for (x0, y0, w, h) in [(300u32, 900u32, 800u32, 40u32), (1400, 900, 60, 40)] {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let off = ((y * c.width + x) * 4) as usize;
                f.rgba[off..off + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
    }
    f
}

#[test]
fn png_sizes_drive_the_xml_and_the_directory_check() {
    let dir = tmpdir("bdn");
    let frame = subtitle_frame();
    let patch = frame.crop().unwrap().unwrap();
    // bbox 必须把两个对象都框进来（1160x40，从 x=300 起）
    assert_eq!((patch.x, patch.y), (300, 900));
    assert_eq!((patch.width, patch.height), (1160, 40));

    let png = patch.to_png(Level::Balanced).unwrap();
    let hdr = header(&png).unwrap();
    assert_eq!((hdr.width, hdr.height), (patch.width, patch.height));
    std::fs::write(dir.join("0001.png"), &png).unwrap();

    let frames = [BdnFrame {
        start: Time::from_millis(1000).unwrap(),
        end: Time::from_millis(3500).unwrap(),
        png_file_name: "0001.png".to_string(),
        png_width: hdr.width,
        png_height: hdr.height,
        x: patch.x as i32,
        y: patch.y as i32,
    }];
    // min_axis 取一个会"抬尺寸"的值：正确语义是过滤，不是钳制
    let doc = build_document(&frames, &options(4000)).unwrap();
    assert!(
        doc.events.is_empty(),
        "最长边 1160 < min_axis 4000，这一帧必须被丢弃而不是被放大"
    );

    let doc = build_document(&frames, &options(2)).unwrap();
    assert_eq!(doc.events.len(), 1);
    assert_eq!(
        (doc.events[0].graphic.width, doc.events[0].graphic.height),
        (1160, 40),
        "Width/Height 必须原样等于 PNG 尺寸"
    );
    assert_eq!(
        (doc.events[0].graphic.x, doc.events[0].graphic.y),
        (300, 900)
    );

    let xml = write(&doc).unwrap();
    let back = parse(&xml).unwrap();
    let rep = validate_dir(&back, &dir).unwrap();
    assert!(rep.is_clean(), "真实目录复核失败：{}", rep.summary());
    assert_eq!(rep.checked, 1);

    // 反例：声明尺寸比 PNG 大 8 像素（旧工程的钳制缺陷）必须被 validate_dir 抓住
    let mut bad = back.clone();
    bad.events[0].graphic.width += 8;
    let rep2 = validate_dir(&bad, &dir).unwrap();
    assert_eq!(rep2.problems.len(), 1);
    assert_eq!(rep2.problems[0].error.code_string(), "STK-BDN-0073");

    // 也直接用 image-core 的 header 复核：两个实现看到的尺寸必须一致
    let bytes = std::fs::read(dir.join("0001.png")).unwrap();
    check_png_size(&bytes, hdr.width, hdr.height).unwrap();
    assert!(check_png_size(&bytes, hdr.width + 1, hdr.height).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blank_frame_has_no_patch_and_must_not_write_a_full_transparent_png() {
    // 09 §3 + 13 §6：全透明帧没有 bbox，作者层必须写 erase display set，
    // 而不是产出一张 1920x1080 的全透明 PNG。
    let f = Frame::empty(Canvas::FHD1080).unwrap();
    assert!(f.crop().unwrap().is_none());
}
