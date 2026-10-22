//! 17 §6 第四/五档的真实验收：内嵌 `[Fonts]` 的 ASS 与 MKV 附件目录。
//!
//! ## 能证明什么、不能证明什么
//!
//! 原本想拿"字体库置空的目录里渲染内嵌产物"当判据。实测发现这条路证明不了
//! 内嵌是否生效：`STK_FONTS_DIR` 只换我们自己的**扫描根**（报告里的 `roots` 可见
//! 它只在扫描侧生效），libass 仍会走它自己的系统字体回退链，
//! 于是"空目录也能渲染成功"——那跟内嵌有没有关系无关。
//! 把这种结果写成"内嵌验证通过"就是拿一个自欺的断言充证据。
//!
//! 因此这一组测的是**真正能被判定的东西**：
//!
//! 1. 内嵌段能被解析器读回，**字节与交付的那份逐字节相同**（SHA-256 相等）；
//! 2. 产物里只有一个 `[Fonts]` 段（旧的必须被替换掉，不能两份并存）；
//! 3. 带内嵌段的 ASS 真能被 libass 渲染成功，且结果与不带它时逐像素一致
//!    （这一条只能证明"加了这段没弄坏渲染"，**不能**证明 libass 真的用了这段）；
//! 4. MKV 附件目录里的字节与报告里的 SHA-256 一致。
//!
//! 真"libass 用没用内嵌字节"需要一条能屏蔽系统字体的注入通路（17 §7），
//! 那由 `--inject-fonts` 的用例回答（见 `subset_render.rs`）。
//!
//! 需要 `STK_LIBASS_DLL`。没有就明确打印跳过理由。

use std::path::{Path, PathBuf};
use std::process::Command;

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,72,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Hello World 123\r\n";

fn libass() -> Option<PathBuf> {
    let p = std::env::var("STK_LIBASS_DLL").ok()?;
    let pb = PathBuf::from(&p);
    pb.is_file().then_some(pb)
}

fn arial() -> Option<PathBuf> {
    let mut candidates = vec![
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\ARIAL.TTF"),
    ];
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local_app_data).join("Microsoft/Windows/Fonts/ARIAL_0.TTF"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-embed-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("没有 JSON 行：{text:?}\nstderr={:?}", out.stderr));
    serde_json::from_str(line).unwrap()
}

fn png_hashes(dir: &Path) -> Vec<String> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(dir)
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
                font_service::attachments::digest_hex(&std::fs::read(&p).unwrap()),
            )
        })
        .collect();
    v.sort();
    v.into_iter().map(|(_, h)| h).collect()
}

/// `font subset --embed-fonts`：内嵌段字节与交付的那份完全一致。
///
/// 交付的是**子集**（字符集来自 ASS 用量），所以比对对象是产物目录里那份子集文件，
/// 不是原始系统字体。比错对象会得出"字体内嵌不对"的错误结论。
#[test]
fn embedded_section_carries_the_exact_delivered_bytes() {
    let (Some(dll), Some(font)) = (libass(), arial()) else {
        eprintln!("跳过：需要 STK_LIBASS_DLL 与一份真实 Arial");
        return;
    };
    let dir = workspace("fonts");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let fonts_dir = dir.join("fonts");
    std::fs::create_dir_all(&fonts_dir).unwrap();
    std::fs::copy(&font, fonts_dir.join("Arial.ttf")).unwrap();

    // 1) 基线：用普通字体库渲染原脚本
    let out_base = dir.join("r_base");
    let st = Command::new(cli())
        .args([
            "render",
            ass.to_str().unwrap(),
            "--fps",
            "24",
            "--canvas",
            "1920x1080",
            "--out",
            out_base.join("bdn.xml").to_str().unwrap(),
            "--json",
        ])
        .env("STK_LIBASS_DLL", &dll)
        .env("STK_FONTS_DIR", &fonts_dir)
        .output()
        .unwrap();
    assert!(st.status.success(), "基线渲染必须成功");
    let base = png_hashes(&out_base);
    assert!(!base.is_empty(), "基线必须产出 PNG");

    // 2) 生成带 [Fonts] 的 ASS
    let embedded = dir.join("embedded.ass");
    let subsets = dir.join("subsets");
    let st = Command::new(cli())
        .args([
            "font",
            "subset",
            ass.to_str().unwrap(),
            "--subset-dir",
            subsets.to_str().unwrap(),
            "--embed-fonts",
            "--out",
            embedded.to_str().unwrap(),
            "--json",
        ])
        .env("STK_LIBASS_DLL", &dll)
        .env("STK_FONTS_DIR", &fonts_dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "--embed-fonts 必须成功：{}",
        String::from_utf8_lossy(&st.stdout)
    );
    let v = json_of(&st);
    let outputs = v["data"]["outputs"].as_array().unwrap();
    let embed = outputs
        .iter()
        .find(|o| o["mode"] == "embed ASS [Fonts]")
        .expect("报告里必须有 embed 档的说明");
    assert_eq!(embed["verified_round_trip"], true, "{embed}");

    // 3) 产物里只有一段 [Fonts]，且字节 = 目录里那份交付产物
    let text = std::fs::read_to_string(&embedded).unwrap();
    assert_eq!(text.matches("[Fonts]").count(), 1, "只能有一个 [Fonts]");
    let items = font_service::parse_font_attachments(&text).unwrap();
    assert_eq!(items.len(), 1);
    let delivered = embed["embedded_fonts"][0]["sha256"].as_str().unwrap();
    assert_eq!(
        items[0].sha256, delivered,
        "内嵌段的哈希必须等于报告里声明交付的那份"
    );
    assert_eq!(
        embed["embedded_fonts"][0]["bytes"].as_u64().unwrap() as usize,
        items[0].data.len(),
        "字节数也要对得上"
    );
    // 交付的确实是子集（这份请求不会触发复杂脚本回退）
    let src_len = std::fs::metadata(&font).unwrap().len() as usize;
    assert!(
        items[0].data.len() < src_len,
        "本用例期望交付子集：内嵌 {} 字节 vs 源 {src_len}",
        items[0].data.len()
    );

    // 4) 带内嵌段的 ASS 真能渲染，且结果与不带它时一致
    //    （只能证明"加这段没弄坏渲染"，**不能**证明 libass 用了它）
    let out_embed = dir.join("r_embed");
    let st = Command::new(cli())
        .args([
            "render",
            embedded.to_str().unwrap(),
            "--fps",
            "24",
            "--canvas",
            "1920x1080",
            "--out",
            out_embed.join("bdn.xml").to_str().unwrap(),
            "--json",
        ])
        .env("STK_LIBASS_DLL", &dll)
        .env("STK_FONTS_DIR", &fonts_dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "带内嵌段的 ASS 必须能渲染：{}",
        String::from_utf8_lossy(&st.stdout)
    );
    let got = png_hashes(&out_embed);
    assert_eq!(base.len(), got.len(), "帧数必须一致");
    assert_eq!(base, got, "内嵌段不能改变渲染结果（逐像素）");
    println!(
        "内嵌段往返字节一致（{} 字节），且 {} 帧渲染不变",
        items[0].data.len(),
        base.len()
    );
}

/// `--mkv-attachments`：附件目录里的字节必须与源字体一致，且报告给出可照抄的命令行。
#[test]
fn mkv_attachments_directory_holds_the_exact_font_bytes() {
    let Some(font) = arial() else {
        eprintln!("跳过：找不到一份真实 Arial");
        return;
    };
    let dir = workspace("mkvatt");
    let ass = dir.join("a.ass");
    std::fs::write(&ass, ASS).unwrap();
    let fonts_dir = dir.join("fonts");
    std::fs::create_dir_all(&fonts_dir).unwrap();
    std::fs::copy(&font, fonts_dir.join("Arial.ttf")).unwrap();

    let subsets = dir.join("out");
    let st = Command::new(cli())
        .args([
            "font",
            "subset",
            ass.to_str().unwrap(),
            "--subset-dir",
            subsets.to_str().unwrap(),
            "--mkv-attachments",
            "--json",
        ])
        .env("STK_FONTS_DIR", &fonts_dir)
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "--mkv-attachments 必须成功：{}",
        String::from_utf8_lossy(&st.stdout)
    );
    let v = json_of(&st);
    let outputs = v["data"]["outputs"].as_array().unwrap();
    let att = outputs
        .iter()
        .find(|o| o["mode"] == "prepare MKV attachments")
        .expect("报告里必须有 MKV 附件档的说明");

    let entries = att["attachments"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{att}");
    let path = PathBuf::from(entries[0]["path"].as_str().unwrap());
    assert!(path.is_file(), "附件必须真的落盘：{}", path.display());
    // 字节必须与报告里声明的 SHA-256 一致。
    // 注意交付的是**子集**（字符集来自 ASS 用量），不是原始系统字体，
    // 所以拿源字体比对会得出错误结论。
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(
        font_service::attachments::digest_hex(&bytes),
        entries[0]["sha256"].as_str().unwrap(),
        "附件字节必须与报告里的哈希一致"
    );
    assert_eq!(
        bytes.len() as u64,
        entries[0]["bytes"].as_u64().unwrap(),
        "字节数也要对得上"
    );
    assert!(
        font_service::is_parseable_font(&bytes),
        "附件必须还是可解析的字体（否则 MKV 里就是一个坏附件）"
    );
    let src_len = std::fs::metadata(&font).unwrap().len() as usize;
    assert!(
        bytes.len() < src_len,
        "本用例期望交付子集：附件 {} 字节 vs 源 {src_len}",
        bytes.len()
    );
    // 报告里的命令行要能直接照抄（并且指向真实产物）
    let hint = att["remux_hint"].as_str().unwrap();
    assert!(hint.contains("mkv remux"), "{hint}");
    assert!(hint.contains("--attach-font"), "{hint}");
    assert!(
        hint.contains(path.to_str().unwrap()),
        "提示必须指向真实产物：{hint}"
    );
}
