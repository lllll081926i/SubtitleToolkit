//! 22 §1 日志落盘的**端到端**行为（含 22 §5 的“logs 收进诊断包”）。
//!
//! 单元测试（`core-model/src/logging.rs`）验滚动与日期换算；这里验用户看得见的行为：
//! 日志真的写进了数据目录、失败真被记成带 code 的一条、且日志会进诊断包。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("logs-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 跑一条命令，返回 stdout（每次都是新进程，所以"新进程也写同一份日志"是可验的）。
fn run(dir: &Path, args: &[&str]) -> String {
    let o = Command::new(cli())
        .args(args)
        .env("STK_DATA_DIR", dir)
        .env_remove("STK_SETTINGS")
        .env_remove("STK_LOG")
        .output()
        .expect("跑 CLI");
    String::from_utf8_lossy(&o.stdout).to_string()
}

/// 数据目录 `logs/` 下唯一那个日志文件的全文。
fn log_text(dir: &Path) -> String {
    let logs = dir.join("logs");
    let mut all = String::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&logs)
        .unwrap_or_else(|e| panic!("日志目录不存在 {}：{e}", logs.display()))
        .flatten()
        .map(|e| e.path())
        .collect();
    names.sort();
    for p in names {
        all.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
    }
    all
}

/// 成功与失败都要落盘，且失败那条带 22 §4 的错误码。
#[test]
fn both_success_and_failure_are_written_with_the_error_code() {
    let d = scratch("basic");

    run(&d, &["doctor", "--json"]);
    let t = log_text(&d);
    assert!(t.contains("command=\"doctor\""), "成功该留一条：{t}");
    assert!(t.contains("ok=true"), "{t}");

    // 这一步必然失败（输入不存在），错误码是 STK-INP-0110
    run(&d, &["probe", "D:\\definitely-missing-xyz.srt", "--json"]);
    let t = log_text(&d);
    assert!(t.contains("ERROR"), "失败该留 ERROR：{t}");
    assert!(
        t.contains("STK-INP-0110"),
        "失败那条要带与报告同一个错误码（22 §4）：{t}"
    );
    assert!(t.contains("command=\"probe\""), "{t}");
}

/// 日志文件里不能有 ANSI 转义：它要被塞进诊断包，转义码在那里只是噪声。
#[test]
fn the_log_file_has_no_ansi_escapes() {
    let d = scratch("ansi");
    run(&d, &["doctor", "--json"]);
    let raw = std::fs::read(log_text_path(&d)).unwrap();
    assert!(
        !raw.contains(&0x1b),
        "日志文件里有 ESC 字节（终端那份才该带颜色）"
    );
    assert!(!raw.is_empty(), "日志该有内容");
}

fn log_text_path(dir: &Path) -> PathBuf {
    let logs = dir.join("logs");
    std::fs::read_dir(&logs)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|x| x.to_str()) == Some("log"))
        .expect("该有一个 .log 文件")
}

/// 36 §1 的 `log_level` 要真的生效：`off` 之后文件里不该再有新内容。
#[test]
fn the_settings_log_level_actually_filters() {
    let d = scratch("level");
    // 先把设置写成 off（这一条命令自己在 off 之前跑，所以会留下一个文件）
    let o = Command::new(cli())
        .args(["settings", "set", "log_level", "off", "--json"])
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_SETTINGS")
        .env_remove("STK_LOG")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("\"ok\":true"),
        "设置该写成"
    );

    run(&d, &["doctor", "--json"]);
    let t = log_text(&d);
    assert!(
        !t.contains("command=\"doctor\""),
        "log_level=off 之后不该再有新记录：{t}"
    );
}

/// `STK_LOG` 压过设置：排查“为什么日志没出来”需要一条不依赖设置文件的路。
#[test]
fn the_environment_override_beats_the_setting() {
    let d = scratch("override");
    Command::new(cli())
        .args(["settings", "set", "log_level", "off", "--json"])
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_LOG")
        .output()
        .unwrap();
    let o = Command::new(cli())
        .args(["doctor", "--json"])
        .env("STK_DATA_DIR", &d)
        .env("STK_LOG", "debug")
        .output()
        .unwrap();
    let t = log_text(&d);
    assert!(
        t.contains("command=\"doctor\""),
        "STK_LOG=debug 该压过设置里的 off：{t}"
    );
    assert!(!o.stdout.is_empty());
}

/// 22 §5：日志要进诊断包，且**不算**用户内容（它们是本程序自己的输出）。
#[test]
fn the_diagnostic_bundle_carries_the_logs_and_does_not_call_them_user_content() {
    let d = scratch("bundle");
    run(&d, &["doctor", "--json"]);
    let out = d.join("diag.zip");
    let o = Command::new(cli())
        .args(["diagnose", "--json", "--out"])
        .arg(&out)
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_SETTINGS")
        .env_remove("STK_LOG")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    let names: Vec<String> = v["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("logs/")),
        "22 §5 要求 logs 进包：{names:?}"
    );
    assert_eq!(
        v["data"]["contains_user_content"], false,
        "日志不是用户内容：{v}"
    );
    assert!(
        v["data"]["user_content_entries"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{v}"
    );

    // 包里的日志内容要真的是日志（不是空文件占位）
    let zip = std::fs::read(&out).unwrap();
    let t = String::from_utf8_lossy(&zip);
    assert!(t.contains("command="), "包里的日志该有真内容");
}
