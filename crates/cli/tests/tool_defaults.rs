//! 36 §1 第二层（ToolDefaults）的 CLI 集成用例。
//!
//! 核心核的是**求值**而不是**存储**：用户问的是"我设的生效了吗"，
//! 而"文件里有这一条"不回答那个问题。所以每条用例都走真实二进制，
//! 从 `show` 的输出里读 `effective`。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!("subtitletool-cli{}", std::env::consts::EXE_SUFFIX))
}

/// 每次用具自己的目录：避免用例之间通过设置文件互相影响。
fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-tdc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(cli())
        .args(args)
        .env("STK_TOOL_DEFAULTS", dir.join("td.json"))
        .output()
        .expect("跑不动 subtitletool-cli");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or(serde_json::Value::Null)
}

/// 从 `show`/`set` 的输出里取某工具的实际生效取值。
fn effective(v: &serde_json::Value, tool: &str) -> serde_json::Value {
    v.pointer(&format!("/data/tools/{tool}/effective"))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[test]
fn a_fresh_install_has_no_effective_values_and_offers_the_whitelist() {
    let d = tmp("fresh");
    let (code, out) = run(&d, &["tool-defaults", "show", "--json"]);
    assert_eq!(code, 0);
    let v = json(&out);
    // 没设过就不该有生效值 —— 补上 CLI 的默认值会造出第二份默认值定义。
    assert_eq!(
        effective(&v, "pgs encode"),
        serde_json::json!({}),
        "空设置不该凭空生出取值"
    );
    // 但白名单要给出来，用户不必翻文档就知道能设什么。
    let allowed = v.pointer("/data/tools/pgs encode/allowed_keys");
    assert!(allowed.is_some_and(|a| a.get("level").is_some()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_set_value_shows_up_as_effective() {
    let d = tmp("set");
    let (code, _) = run(
        &d,
        &[
            "tool-defaults",
            "set",
            "pgs encode",
            "level",
            "standard",
            "--json",
        ],
    );
    assert_eq!(code, 0);
    let (_, out) = run(&d, &["tool-defaults", "show", "--json"]);
    assert_eq!(
        effective(&json(&out), "pgs encode"),
        serde_json::json!({"level": "standard"})
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// 一个工具的设置不能漏到另一个工具上。
#[test]
fn settings_do_not_leak_between_tools() {
    let d = tmp("leak");
    run(
        &d,
        &["tool-defaults", "set", "render", "min_axis", "8", "--json"],
    );
    let (_, out) = run(&d, &["tool-defaults", "show", "--json"]);
    let v = json(&out);
    assert_eq!(
        effective(&v, "render"),
        serde_json::json!({"min_axis": "8"})
    );
    assert_eq!(
        effective(&v, "pgs encode"),
        serde_json::json!({}),
        "render 的设置漏到了 pgs encode"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// 拼错的键必须**在写入前**被拒。
///
/// 允许写进去的后果：它存得下来但不生效，用户看到"设置成功了"却看不到变化 ——
/// 那是最难查的一类问题。
#[test]
fn a_typo_key_is_refused_before_it_is_written() {
    let d = tmp("typo");
    let (code, out) = run(
        &d,
        &["tool-defaults", "set", "pgs encode", "levle", "x", "--json"],
    );
    assert_ne!(code, 0, "拼错的键必须被拒");
    assert!(!out.contains("\"ok\":true"), "不该报成功: {out}");
    // 而且真的没写进去
    let (_, show) = run(&d, &["tool-defaults", "show", "--json"]);
    assert_eq!(effective(&json(&show), "pgs encode"), serde_json::json!({}));
    // 报错里要给出正确的键名
    assert!(out.contains("level"), "报错要给出可用的键: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_unknown_tool_is_refused_at_parse_time() {
    let d = tmp("notool");
    let (code, out) = run(
        &d,
        &["tool-defaults", "set", "nope", "level", "x", "--json"],
    );
    assert_eq!(code, 2, "工具名错是用法错");
    assert!(!out.contains("\"ok\":true"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn init_refuses_to_overwrite_an_existing_file() {
    let d = tmp("init");
    assert_eq!(run(&d, &["tool-defaults", "init", "--json"]).0, 0);
    let (code, out) = run(&d, &["tool-defaults", "init", "--json"]);
    assert_ne!(code, 0, "init 不该覆盖已有文件");
    assert!(out.contains("已存在"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// 值里存的是 JSON 数字/布尔，不是一律字符串。
#[test]
fn numeric_and_boolean_values_keep_their_type_on_disk() {
    let d = tmp("types");
    run(
        &d,
        &["tool-defaults", "set", "render", "min_axis", "8", "--json"],
    );
    let raw = std::fs::read_to_string(d.join("td.json")).unwrap();
    assert!(raw.contains("\"min_axis\": 8"), "数字被存成了字符串: {raw}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_broken_file_does_not_stop_the_tool_and_is_reported() {
    let d = tmp("broken");
    std::fs::write(d.join("td.json"), "{ not json").unwrap();
    let (code, out) = run(&d, &["tool-defaults", "show", "--json"]);
    assert_eq!(code, 0, "坏设置文件不该让人用不了程序");
    let v = json(&out);
    let problems = v.pointer("/data/problems").and_then(|p| p.as_array());
    assert!(
        problems.is_some_and(|p| !p.is_empty()),
        "坏文件必须说出来: {out}"
    );
    assert!(
        v.pointer("/data/quarantined").is_some_and(|q| !q.is_null()),
        "坏文件要留证据"
    );
    let _ = std::fs::remove_dir_all(&d);
}
