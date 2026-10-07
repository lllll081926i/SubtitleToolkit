//! Windows 路径边界（23 §文件、AT-PATH-001）：`validate_dir` 必须支持中文文件名与 >260 字符
//! 长路径，并且绝不把路径强转 UTF-8 去做系统调用。

use std::path::{Path, PathBuf};

use codec_bdn::{build_document, validate_dir, BdnFrame, BdnOptions};
use core_model::time::Fps;

mod common;
use common::png;

const FPS: Fps = Fps::PAL_25;

/// 造一个"中文 + 深层嵌套"的交换目录，并把内部 PNG 的完整路径推到 >260 字符。
fn deep_dir(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("字幕工具箱-BDN-{}-{tag}", std::process::id()));
    for i in 0..8 {
        p.push(format!(
            "第{i}层-很长的中文目录名用于把完整路径推到 MAX_PATH 二百六十以上-继续加长"
        ));
    }
    p
}

/// 目录 + 中文长文件名，完整路径必须显著超过 260 字符（AT-PATH-001 的真实形态）。
fn deep_file(dir: &Path, name: &str) -> PathBuf {
    let full = dir.join(name);
    assert!(
        full.to_string_lossy().chars().count() > 260,
        "测试路径必须真的超过 MAX_PATH：{}",
        full.display()
    );
    full
}

/// Windows 下建目录/写文件也要长路径前缀，否则测试自己先失败。
fn verbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy().to_string();
    if raw.starts_with(r"\\?\") || !path.is_absolute() {
        return path.to_path_buf();
    }
    PathBuf::from(format!(r"\\?\{}", raw.replace('/', "\\")))
}

fn frame(name: &str, ms: i64) -> BdnFrame {
    BdnFrame {
        start: core_model::time::Time::from_millis(ms).unwrap(),
        end: core_model::time::Time::from_millis(ms + 400).unwrap(),
        png_file_name: name.to_string(),
        png_width: 640,
        png_height: 100,
        x: 640,
        y: 900,
    }
}

#[test]
fn chinese_file_names_and_long_paths_are_validated() {
    let dir = deep_dir("ok");
    let real = verbatim(&dir);
    std::fs::create_dir_all(&real).unwrap();
    let names: Vec<String> = [
        "第一句字幕-很长的中文文件名用于把路径进一步加长.png",
        "中文字幕-第二句-很长的中文文件名继续加长.png",
        "ａｂｃ全角-很长的中文文件名再加长一些.png",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    for name in &names {
        std::fs::write(deep_file(&real, name), png(640, 100)).unwrap();
    }
    let frames: Vec<BdnFrame> = names
        .iter()
        .enumerate()
        .map(|(i, n)| frame(n, (i as i64 + 1) * 1000))
        .collect();
    let doc = build_document(
        &frames,
        &BdnOptions {
            fps: FPS,
            language: "chi".to_string(),
            name: "中文".to_string(),
            min_axis: 0,
            offset_x: 0,
            offset_y: 0,
        },
    )
    .unwrap();

    // 传给 validate_dir 的是"未加前缀"的原始路径：内部必须自己处理长路径。
    let report = validate_dir(&doc, &dir).unwrap();
    assert!(report.is_clean(), "{}", report.summary());
    assert_eq!(report.checked, 3);
    let _ = std::fs::remove_dir_all(&real);
}

#[test]
fn long_path_with_one_bad_png_is_located() {
    let dir = deep_dir("bad");
    let real = verbatim(&dir);
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(deep_file(&real, "好.png"), png(640, 100)).unwrap();
    std::fs::write(deep_file(&real, "坏.png"), png(640, 222)).unwrap();
    let doc = build_document(
        &[frame("好.png", 1000), frame("坏.png", 2000)],
        &BdnOptions {
            fps: FPS,
            language: "chi".to_string(),
            name: "n".to_string(),
            min_axis: 0,
            offset_x: 0,
            offset_y: 0,
        },
    )
    .unwrap();
    let r = validate_dir(&doc, &dir).unwrap();
    assert_eq!(r.problems.len(), 1);
    assert_eq!(r.problems[0].file_name, "坏.png");
    assert_eq!(r.problems[0].error.code_string(), "STK-BDN-0073");
    let _ = std::fs::remove_dir_all(&real);
}

#[test]
fn path_traversal_and_control_chars_in_file_names_are_refused() {
    let dir = std::env::temp_dir();
    let mut m = common::meta(FPS);
    m.width = 1920;
    m.height = 1080;
    let g = codec_bdn::BdnGraphic::new("../../windows/win.ini", 100, 50, 0, 0).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        core_model::time::Time::from_millis(0).unwrap(),
        core_model::time::Time::from_millis(400).unwrap(),
        g,
        FPS,
    )
    .unwrap();
    let doc = codec_bdn::BdnDocument::new(m, vec![ev]);
    let r = validate_dir(&doc, &dir).unwrap();
    assert_eq!(r.problems[0].error.code_string(), "STK-BDN-0077");

    // 绝对路径 FileName 是 BDN 交换目录里真实存在的写法：支持，但仍然只读头部比对。
    let abs = verbatim(&dir).join("绝对路径字幕.png");
    std::fs::write(&abs, png(100, 50)).unwrap();
    let g2 = codec_bdn::BdnGraphic::new(abs.to_string_lossy().to_string(), 100, 50, 0, 0).unwrap();
    let ev2 = codec_bdn::BdnEvent::new(
        core_model::time::Time::from_millis(0).unwrap(),
        core_model::time::Time::from_millis(400).unwrap(),
        g2,
        FPS,
    )
    .unwrap();
    let mut m2 = common::meta(FPS);
    m2.width = 1920;
    m2.height = 1080;
    let doc2 = codec_bdn::BdnDocument::new(m2, vec![ev2]);
    let r2 = validate_dir(&doc2, &dir).unwrap();
    assert!(r2.is_clean(), "{}", r2.summary());
    let _ = std::fs::remove_file(&abs);
}

#[test]
fn report_summary_is_informative() {
    let dir = std::env::temp_dir();
    let mut m = common::meta(FPS);
    m.width = 1920;
    m.height = 1080;
    let g = codec_bdn::BdnGraphic::new("不存在.png", 100, 50, 0, 0).unwrap();
    let ev = codec_bdn::BdnEvent::new(
        core_model::time::Time::from_millis(0).unwrap(),
        core_model::time::Time::from_millis(400).unwrap(),
        g,
        FPS,
    )
    .unwrap();
    let doc = codec_bdn::BdnDocument::new(m, vec![ev]);
    let r = validate_dir(&doc, &dir).unwrap();
    let s = r.summary();
    assert!(s.contains("1 个 PNG"), "{s}");
    assert!(s.contains("STK-BDN-0075"), "{s}");
    assert!(!r.is_clean());
}
