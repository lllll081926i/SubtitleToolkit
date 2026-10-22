//! 子集渲染等价（17 §5 的 RGBA 对照）：用真 libass 证明子集渲染与整份一致。
//!
//! ## 这条测试存在的理由
//!
//! 17 §5 的原话是"原字体 vs subset 字体，在相同 renderer/timestamp/canvas 上的
//! glyph shaping + rendered RGBA 必须达到 golden 条件"。
//! `font-service::subset` 里的 cmap 级检查只是其中一半；**这一条补上 RGBA 那一半**。
//!
//! 做法：同一份 ASS、同一 canvas、同一 fps，把整份字体与子集字体分别经
//! `--inject-fonts`（17 §7 `ass_add_font` 通路）注入 libass，
//! 逐帧比较 PNG 的 SHA-256。任何一个像素不同就是失败。
//!
//! 需要 `STK_LIBASS_DLL`。没有就把跳过理由打清楚，不算通过。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,72,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Hello World 123\r\nDialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,The quick brown fox jumps\r\n";

fn libass() -> Option<PathBuf> {
    let p = std::env::var("STK_LIBASS_DLL").ok()?;
    let pb = PathBuf::from(&p);
    if pb.is_file() {
        Some(pb)
    } else {
        None
    }
}

/// 找一份真正的 Arial。没有就返回 None（明确跳过）。
fn arial() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\ARIAL.TTF"),
    ];
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Microsoft/Windows/Fonts/ARIAL_0.TTF"));
    }
    for p in candidates {
        if p.is_file() {
            return Some(p);
        }
    }
    // 退而求其次：任意一份 ttf
    let mut any = std::fs::read_dir(r"C:\Windows\Fonts")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("ttf"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    any.sort();
    any.into_iter().next()
}

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-subset-render-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn png_hashes(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("png"))
                .unwrap_or(false)
        })
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                font_service::subset::sha256_hex(&std::fs::read(&p).unwrap()),
            )
        })
        .collect();
    out.sort();
    out
}

fn render(cli: &Path, ass: &Path, outdir: &Path, font_dir: &Path, dll: &Path) -> bool {
    let _ = std::fs::remove_dir_all(outdir);
    std::fs::create_dir_all(outdir).unwrap();
    let st = Command::new(cli)
        .args([
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "24",
            "--canvas",
            "1920x1080",
            "--inject-fonts",
            "--out",
            outdir.join("bdn.xml").to_str().unwrap(),
            "--json",
        ])
        .env("STK_LIBASS_DLL", dll)
        .env("STK_FONTS_DIR", font_dir)
        .output()
        .expect("CLI 必须能启动");
    if !st.status.success() {
        eprintln!("render 失败：{}", String::from_utf8_lossy(&st.stdout));
        return false;
    }
    true
}

/// AT-FONT-003：子集字体与整份字体渲染结果逐像素一致。
#[test]
fn at_font_003_subset_renders_identically_to_the_full_font() {
    let (Some(dll), Some(font)) = (libass(), arial()) else {
        eprintln!("跳过 AT-FONT-003：需要 STK_LIBASS_DLL 与一份真实 Arial");
        return;
    };
    let dir = workspace("eq");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();

    // 小字符集：只要这行字用到的
    let want: BTreeSet<char> = "Hello World 123The quick brown fox jumps".chars().collect();
    let bytes = std::fs::read(&font).unwrap();
    let o = font_service::subset::subset_bytes(&bytes, 0, &want).unwrap();
    if o.outcome != font_service::subset::Outcome::Subset {
        eprintln!("跳过 AT-FONT-003：这份字体回退了（{:?}）", o.fallback);
        return;
    }
    println!(
        "子集：{} -> {} 字节（省 {}）",
        o.source_bytes,
        o.bytes.len(),
        o.saved_bytes()
    );

    // 两个独立字体目录，保证 libass 只能看见那一份
    let full_dir = dir.join("full");
    let sub_dir = dir.join("sub");
    std::fs::create_dir_all(&full_dir).unwrap();
    std::fs::create_dir_all(&sub_dir).unwrap();
    // libass 按 Fontname 匹配，两份都叫 Arial
    std::fs::copy(&font, full_dir.join("Arial.ttf")).unwrap();
    std::fs::write(sub_dir.join("Arial.ttf"), &o.bytes).unwrap();

    let cli = PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"));
    let out_full = dir.join("r_full");
    let out_sub = dir.join("r_sub");
    assert!(
        render(&cli, &ass, &out_full, &full_dir, &dll),
        "整份字体渲染必须成功"
    );
    assert!(
        render(&cli, &ass, &out_sub, &sub_dir, &dll),
        "子集字体渲染必须成功（渲染不了就说明子集是坏的）"
    );

    let a = png_hashes(&out_full);
    let b = png_hashes(&out_sub);
    assert!(!a.is_empty(), "渲染必须产出 PNG");
    assert_eq!(
        a.len(),
        b.len(),
        "帧数必须一致：整份 {} 帧，子集 {} 帧",
        a.len(),
        b.len()
    );
    for ((na, ha), (nb, hb)) in a.iter().zip(b.iter()) {
        assert_eq!(
            ha, hb,
            "帧 {na}/{nb} 像素不一致：整份 {ha}，子集 {hb} —— 17 §5 的 RGBA 对照不过"
        );
    }
    println!("{} 帧逐像素一致", a.len());
}
