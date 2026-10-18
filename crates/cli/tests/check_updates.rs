//! 28 §6 更新检查的 CLI 集成用例。
//!
//! 这里核的是**命令行到输出**的整条路：参数解析（`check-updates` 没有位置参数，
//! 走的是自己遍历 `rest` 的那条路）、信封形状、以及"到底做了什么"的声明。
//!
//! 特别核一条**安全属性**：本命令的结果里必须自报 `downloaded: false` /
//! `installed: false`。规格（28 §6）明确说"不要在早期自写高权限 auto-updater"，
//! 而这条断言让"它真的没下载"成为可检查的事实，而不是靠读代码相信。

use std::path::PathBuf;
use std::process::Command;

fn cli() -> PathBuf {
    // 与其它 CLI 集成测试同一套定位方式。
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("subtitletool-cli{}", std::env::consts::EXE_SUFFIX))
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(cli())
        .args(args)
        .output()
        .expect("跑不动 subtitletool-cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-upd-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn json_field(s: &str, key: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    v.get("data")?.get(key).map(|x| {
        if let Some(t) = x.as_str() {
            t.to_string()
        } else {
            x.to_string()
        }
    })
}

/// 没有清单时**不得**静默报“已是最新”。
///
/// 这条是整个功能里最要紧的行为：一个静默返回“已是最新”的更新检查
/// 比没有更新检查更坏 —— 它会让用户确信自己是最新的。
#[test]
fn without_a_manifest_it_reports_unknown_and_never_pretends_to_be_current() {
    let (code, out) = run(&["check-updates", "--json"]);
    assert_eq!(code, 0, "没清单不是错误，只是没有结论");
    assert_eq!(json_field(&out, "verdict").as_deref(), Some("unknown"));
    assert_ne!(
        json_field(&out, "verdict").as_deref(),
        Some("same"),
        "没有清单却说‘已是最新’"
    );
    let detail = json_field(&out, "detail").unwrap_or_default();
    assert!(
        detail.contains("--manifest"),
        "要告诉用户怎么补上: {detail}"
    );
}

/// 本命令必须自报“没有下载、没有安装”。
#[test]
fn it_declares_that_it_downloaded_and_installed_nothing() {
    let (_, out) = run(&["check-updates", "--json"]);
    assert_eq!(json_field(&out, "downloaded").as_deref(), Some("false"));
    assert_eq!(json_field(&out, "installed").as_deref(), Some("false"));
    let meaning = json_field(&out, "meaning").unwrap_or_default();
    assert!(meaning.contains("没有下载"), "说明没说清: {meaning}");
}

#[test]
fn a_newer_manifest_is_reported_as_newer() {
    let d = tmp("newer");
    let m = d.join("m.json");
    std::fs::write(&m, r#"{"version":"99.0.0"}"#).unwrap();
    let (code, out) = run(&["check-updates", "--manifest", m.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    assert_eq!(json_field(&out, "verdict").as_deref(), Some("newer"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_older_manifest_is_not_an_error() {
    let d = tmp("older");
    let m = d.join("m.json");
    // 本地是 0.1.0；0.0.1 必然更旧。
    std::fs::write(&m, r#"{"version":"0.0.1"}"#).unwrap();
    let (code, out) = run(&["check-updates", "--manifest", m.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "清单更旧不是错误");
    assert_eq!(json_field(&out, "verdict").as_deref(), Some("older"));
    let _ = std::fs::remove_dir_all(&d);
}

/// 清单坏了要**报错**，不能退化成“已是最新”。
#[test]
fn a_broken_manifest_fails_instead_of_reporting_current() {
    let d = tmp("broken");
    let m = d.join("m.json");
    std::fs::write(&m, "这不是 JSON").unwrap();
    let (code, out) = run(&["check-updates", "--manifest", m.to_str().unwrap(), "--json"]);
    assert_ne!(code, 0, "坏清单必须报错");
    assert!(!out.contains("\"same\""), "坏清单不该给出任何结论: {out}");

    // 是 JSON 但没有 version
    let m2 = d.join("m2.json");
    std::fs::write(&m2, r#"{"ver":"1"}"#).unwrap();
    let (code2, _) = run(&[
        "check-updates",
        "--manifest",
        m2.to_str().unwrap(),
        "--json",
    ]);
    assert_ne!(code2, 0, "缺 version 必须报错");
    let _ = std::fs::remove_dir_all(&d);
}

/// 版本形状不认识时报 `unknown` 并说清是哪个字符串 —— 不猜。
#[test]
fn an_unrecognised_version_shape_is_unknown_not_a_guess() {
    let d = tmp("weird");
    let m = d.join("m.json");
    std::fs::write(&m, r#"{"version":"v2.0"}"#).unwrap();
    let (code, out) = run(&["check-updates", "--manifest", m.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0);
    assert_eq!(json_field(&out, "verdict").as_deref(), Some("unknown"));
    let detail = json_field(&out, "detail").unwrap_or_default();
    assert!(detail.contains("v2.0"), "要说清是哪个字符串: {detail}");
    let _ = std::fs::remove_dir_all(&d);
}

/// 清单里的 url 覆盖内置默认地址。
#[test]
fn a_manifest_can_point_at_its_own_release_page() {
    let d = tmp("url");
    let m = d.join("m.json");
    std::fs::write(
        &m,
        r#"{"version":"99.0.0","url":"https://example.invalid/r"}"#,
    )
    .unwrap();
    let (_, out) = run(&["check-updates", "--manifest", m.to_str().unwrap(), "--json"]);
    assert_eq!(
        json_field(&out, "release_url").as_deref(),
        Some("https://example.invalid/r")
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `--release-url` 覆盖内置默认值。
#[test]
fn the_release_url_flag_overrides_the_builtin_default() {
    let (_, out) = run(&[
        "check-updates",
        "--release-url",
        "https://mine.invalid/x",
        "--json",
    ]);
    assert_eq!(
        json_field(&out, "release_url").as_deref(),
        Some("https://mine.invalid/x")
    );
}

/// 不认识的开关要报用法错误，而不是被当成输入文件悄悄忽略。
#[test]
fn an_unknown_flag_is_a_usage_error() {
    let (code, out) = run(&["check-updates", "--nope", "x", "--json"]);
    assert_eq!(code, 2, "用法错应当退 2");
    assert!(!out.contains("\"ok\":true"), "不该成功: {out}");
}
