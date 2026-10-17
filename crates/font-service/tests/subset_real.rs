//! 子集真实字体测试：用系统字体验证"真的更小 + 等价检查过 + 复杂脚本回退"。
//!
//! 不依赖本项目的构建产物之外的素材；找不到系统字体就明确跳过。

use std::collections::BTreeSet;
use std::path::PathBuf;

use font_service::subset::{subset_bytes, FallbackReason, Outcome};

fn system_font() -> Option<PathBuf> {
    for dir in ["C:\\Windows\\Fonts", "D:\\Windows\\Fonts"] {
        let d = PathBuf::from(dir);
        if !d.is_dir() {
            continue;
        }
        // 优先找到一份含 CJK 的与一份纯拉丁的
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&d)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.eq_ignore_ascii_case("ttf") || s.eq_ignore_ascii_case("otf"))
                    .unwrap_or(false)
            })
            .collect();
        entries.sort();
        if let Some(p) = entries.first() {
            return Some(p.clone());
        }
    }
    None
}

fn chars(s: &str) -> BTreeSet<char> {
    s.chars().collect()
}

#[test]
fn real_font_subset_is_smaller_and_equivalent() {
    let Some(path) = system_font() else {
        eprintln!("跳过：找不到系统字体");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    // 只要少量拉丁字符：子集应当明显更小
    let want = chars("Hello World 123");
    let o = subset_bytes(&bytes, 0, &want).unwrap();

    println!(
        "字体 {}：{} -> {} 字节（省 {}）outcome={:?} fallback={:?} probes={}",
        path.display(),
        o.source_bytes,
        o.bytes.len(),
        o.saved_bytes(),
        o.outcome,
        o.fallback,
        o.equivalence_probes
    );

    assert_eq!(
        o.outcome,
        Outcome::Subset,
        "少量拉丁字符的子集应当成功（回退原因：{:?}）",
        o.fallback
    );
    assert!(o.fallback.is_none());
    assert!(
        o.bytes.len() < o.source_bytes,
        "子集必须更小：{} vs {}",
        o.bytes.len(),
        o.source_bytes
    );
    assert_eq!(o.source_sha256.len(), 64);
    assert_eq!(o.output_sha256.len(), 64);
    assert_ne!(o.source_sha256, o.output_sha256, "子集内容必然与原字体不同");
    assert!(
        o.equivalence_probes > 0,
        "等价检查必须真的跑过，不能是 0 次比对"
    );

    // 子集必须仍是一个能被解析器认出的字体（否则等于交付了一个坏文件）
    assert!(
        font_service::is_parseable_font(&o.bytes),
        "子集产物必须仍能被解析"
    );
}

#[test]
fn subset_path_output_still_parses_and_covers_requested_chars() {
    let Some(path) = system_font() else {
        eprintln!("跳过：找不到系统字体");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let want = chars("ABCDEFabcdef");
    let o = subset_bytes(&bytes, 0, &want).unwrap();
    if o.outcome != Outcome::Subset {
        eprintln!("跳过覆盖检查：字体回退了（{:?}）", o.fallback);
        return;
    }
    // 子集里请求的字符必须仍能解析出字形（覆盖没丢）
    let scope = allsorts::binary::read::ReadScope::new(&o.bytes);
    let data = scope.read::<allsorts::font_data::FontData>().unwrap();
    let provider = data.table_provider(0).unwrap();
    let mut font = allsorts::Font::new(provider).unwrap();
    let mut missing = Vec::new();
    for c in &want {
        let (gid, _) =
            font.lookup_glyph_index(*c, allsorts::font::MatchingPresentation::NotRequired, None);
        if gid == 0 {
            missing.push(*c);
        }
    }
    assert!(
        missing.is_empty(),
        "子集丢了请求的字符：{missing:?}（outcome={:?}）",
        o.outcome
    );
}

#[test]
fn complex_script_request_falls_back_to_the_whole_font() {
    let Some(path) = system_font() else {
        eprintln!("跳过：找不到系统字体");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let o = subset_bytes(&bytes, 0, &chars("مرحبا")).unwrap();
    assert_eq!(o.outcome, Outcome::FullFontFallback);
    assert_eq!(o.fallback, Some(FallbackReason::ComplexScript));
    assert_eq!(o.bytes, bytes, "回退必须原样给回整份字体");
    assert_eq!(o.source_sha256, o.output_sha256);
}

#[test]
fn missing_chars_fall_back_rather_than_producing_an_empty_font() {
    // 用一个不可能存在的私用区字符：cmap 里没有对应字形
    let Some(path) = system_font() else {
        eprintln!("跳过：找不到系统字体");
        return;
    };
    let bytes = std::fs::read(&path).unwrap();
    let o = subset_bytes(&bytes, 0, &chars("\u{10FFFD}\u{10FFFE}")).unwrap();
    assert_eq!(
        o.outcome,
        Outcome::FullFontFallback,
        "没有对应字形时不能交付一个空字体"
    );
    assert!(
        matches!(
            o.fallback,
            Some(FallbackReason::UnsupportedFont(_)) | Some(FallbackReason::NoRequestedGlyphs)
        ),
        "{:?}",
        o.fallback
    );
    assert_eq!(o.bytes, bytes);
}
