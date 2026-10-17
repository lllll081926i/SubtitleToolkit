//! `mkv identify|extract|remux` 的端到端验收（18_CONTAINER_HELPER / AT-MKV-001）。
//!
//! 这些测试需要真实 MKVToolNix。找不到就**明确跳过并说明**，而不是静默通过 ——
//! "跳过"与"通过"是两种结论，混起来就等于虚报（24 §执行记录合同）。
//!
//! 素材用 MKVToolNix 自己造（ffmpeg 若可用则加视频/音频轨），
//! 因为真实商业 MKV 语料没有授权入库（24 §语料授权）。
//! 判据全部交给 MKVToolNix 自己验证，不用本项目 reader/writer 自证往返。

use std::path::{Path, PathBuf};
use std::process::Command;

fn mkv_dir() -> Option<PathBuf> {
    for var in ["STK_MKVTOOLNIX_DIR"] {
        if let Ok(d) = std::env::var(var) {
            let p = PathBuf::from(d);
            if p.join("mkvmerge.exe").is_file() {
                return Some(p);
            }
        }
    }
    for dir in ["D:\\xia\\mkvtoolnix", "C:\\Program Files\\MKVToolNix"] {
        let p = PathBuf::from(dir);
        if p.join("mkvmerge.exe").is_file() {
            return Some(p);
        }
    }
    None
}

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-mkv-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_subtitletool-cli"))
        .args(args)
        .output()
        .expect("CLI 二进制必须能启动")
}

fn json(out: &std::process::Output) -> serde_json::Value {
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("没有 JSON 行：stdout={text:?} stderr={:?}", out.stderr));
    serde_json::from_str(line).expect("--json 输出必须是合法 JSON")
}

/// 造一个只有字幕轨 + 字体附件的 MKV（不需要 ffmpeg）。
fn build_source(dir: &Path, tools: &Path) -> PathBuf {
    let srt = dir.join("en.srt");
    std::fs::write(&srt, "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n\r\n").unwrap();
    // 一份真的 ttf：拿系统字体目录里第一份；找不到就跳过附件断言
    let font = std::fs::read_dir("C:\\Windows\\Fonts")
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .find(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("ttf"))
                .unwrap_or(false)
        })
        .expect("需要一份系统 TTF 才能验证字体附件");
    let font_copy = dir.join("deck.ttf");
    std::fs::copy(&font, &font_copy).unwrap();

    let out = dir.join("source.mkv");
    let st = Command::new(tools.join("mkvmerge.exe"))
        .args([
            "--output",
            out.to_str().unwrap(),
            "--no-date",
            "--language",
            "0:eng",
            "--track-name",
            "0:English Subs",
            srt.to_str().unwrap(),
            "--attach-file",
            font_copy.to_str().unwrap(),
        ])
        .status()
        .expect("mkvmerge 必须能启动");
    assert!(st.success(), "造素材失败");
    out
}

fn sha256(p: &Path) -> String {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(std::fs::read(p).unwrap());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn identify_with_mkvmerge(tools: &Path, mkv: &Path) -> serde_json::Value {
    let out = Command::new(tools.join("mkvmerge.exe"))
        .args(["--identify", "-J", mkv.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "独立 identify 失败");
    serde_json::from_slice(&out.stdout).unwrap()
}

/// AT-MKV-001 的主链路：提取 → 处理 → 新 MKV，原文件不改，字体附件存在。
#[test]
fn at_mkv_001_extract_process_remux_keeps_source_and_attachments() {
    let Some(tools) = mkv_dir() else {
        eprintln!("跳过 AT-MKV-001：未找到 MKVToolNix（设 STK_MKVTOOLNIX_DIR 可指定）");
        return;
    };
    let dir = workspace("full");
    let src = build_source(&dir, &tools);
    let tools_arg = tools.to_str().unwrap();
    let src_hash = sha256(&src);

    // ---- identify ----
    let out = run(&[
        "mkv",
        "identify",
        src.to_str().unwrap(),
        "--tools-dir",
        tools_arg,
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "identify 应成功：{out:?}");
    let v = json(&out);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["subtitle_tracks"], serde_json::json!([0]));
    assert_eq!(
        v["data"]["font_attachments"],
        serde_json::json!(["deck.ttf"]),
        "字体附件必须被认出来"
    );

    // ---- extract 字幕 + 附件 ----
    let srt = dir.join("out.srt");
    let font = dir.join("out.ttf");
    let out = run(&[
        "mkv",
        "extract",
        src.to_str().unwrap(),
        "--tools-dir",
        tools_arg,
        "--track",
        &format!("0:{}", srt.display()),
        "--attachment",
        &format!("1:{}", font.display()),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "extract 应成功：{out:?}");
    assert!(std::fs::read_to_string(&srt).unwrap().contains("Hello"));
    assert_eq!(
        sha256(&font),
        sha256(&dir.join("deck.ttf")),
        "抽出的附件必须与源字节一致"
    );

    // ---- remux：改语言/名称 + 再附一份字体 ----
    let newmkv = dir.join("new.mkv");
    let out = run(&[
        "mkv",
        "remux",
        src.to_str().unwrap(),
        "--tools-dir",
        tools_arg,
        "--out",
        newmkv.to_str().unwrap(),
        "--subtitle-tracks",
        "0",
        "--track-language",
        "0:zho-Hans",
        "--track-name",
        "0:Processed",
        "--attach-font",
        dir.join("deck.ttf").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(0), "remux 应成功：{out:?}");
    let v = json(&out);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["source_untouched"], true);
    assert_eq!(v["data"]["metadata_preserved"], true, "元数据保持是交付门");

    // ---- 原文件不改（最强判据：内容哈希）----
    assert_eq!(sha256(&src), src_hash, "18 §4：源文件绝不能被改动");

    // ---- 独立工具复核产物 ----
    let info = identify_with_mkvmerge(&tools, &newmkv);
    let track = &info["tracks"][0];
    assert_eq!(track["type"], "subtitles");
    assert_eq!(track["properties"]["language"], "chi", "语言应被改成中文");
    assert_eq!(track["properties"]["track_name"], "Processed", "名称应被改");
    assert!(
        !info["attachments"].as_array().unwrap().is_empty(),
        "产物必须保留字体附件"
    );

    // ---- 附件去重：源里已有同名同大小的，不该再附一份 ----
    let names: Vec<&str> = info["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["file_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec!["deck.ttf"],
        "同一份字体不该被附两次（去重依名字+字节数）：{names:?}"
    );
}

/// 18 §4：绝不原地修改。输出与输入相同时必须在解析/执行层拒绝。
#[test]
fn at_mkv_001_in_place_output_is_refused() {
    let Some(tools) = mkv_dir() else {
        eprintln!("跳过原地修改用例：未找到 MKVToolNix");
        return;
    };
    let dir = workspace("inplace");
    let src = build_source(&dir, &tools);
    let before = sha256(&src);
    let out = run(&[
        "mkv",
        "remux",
        src.to_str().unwrap(),
        "--tools-dir",
        tools.to_str().unwrap(),
        "--out",
        src.to_str().unwrap(),
        "--all-subtitles",
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(1), "原地修改必须失败");
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0257", "错误码要指向 18 §4");
    assert!(v["errors"][0]["action"].is_string(), "拒绝要给下一步");
    assert_eq!(sha256(&src), before, "被拒之后源文件也不能变");
}

/// 不存在的轨道 id 必须在跑外部工具之前就被挡住（错误信息要能指路）。
#[test]
fn at_mkv_001_unknown_track_id_is_rejected_with_a_list() {
    let Some(tools) = mkv_dir() else {
        eprintln!("跳过未知轨道用例：未找到 MKVToolNix");
        return;
    };
    let dir = workspace("badtrack");
    let src = build_source(&dir, &tools);
    let out = run(&[
        "mkv",
        "extract",
        src.to_str().unwrap(),
        "--tools-dir",
        tools.to_str().unwrap(),
        "--track",
        &format!("99:{}", dir.join("x.srt").display()),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0253");
    let details = v["errors"][0]["details"].as_array().unwrap();
    assert!(
        details
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("可用轨道")),
        "错误里要列出可用轨道，否则用户只能猜：{details:?}"
    );
}

/// 找不到 MKVToolNix 时必须是可诊断的依赖错误，而不是 panic 或假成功。
#[test]
fn missing_mkvtoolnix_is_a_dependency_error_not_a_panic() {
    let empty = workspace("notools");
    let bogus = empty.join("not-installed");
    std::fs::create_dir_all(&bogus).unwrap();
    let out = run(&[
        "mkv",
        "identify",
        "x.mkv",
        "--tools-dir",
        bogus.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0251");
    assert!(v["errors"][0]["action"].is_string(), "要给安装指引");
}

/// 未实现命令必须仍然明确失败（不能因为 mkv 落地就把占位机制拆了）。
#[test]
fn font_subset_is_still_explicitly_unimplemented() {
    let out = run(&["font", "subset", "x.ass", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["errors"][0]["code"], "STK-INP-0060");
}
