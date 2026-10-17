//! 36 §1 AppSettings 落盘的**端到端**行为（28 §4 的 "settings 的落盘"）。
//!
//! 单元测试覆盖解析与序列化；这一组验的是用户真能做的事：
//! 设置能被新进程读到、未知字段在往返中不丢、非法值被拒而不是静默写入。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("settings-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 跑一条 `settings` 命令；每个调用都是**新进程**，所以"读得到"本身就是持久化证据。
fn run(dir: &Path, args: &[&str]) -> serde_json::Value {
    let o = Command::new(cli())
        .arg("settings")
        .args(args)
        .arg("--json")
        .env("STK_SETTINGS", dir.join("settings.json"))
        .output()
        .expect("跑 CLI");
    let s = String::from_utf8_lossy(&o.stdout);
    serde_json::from_str(s.trim()).unwrap_or_else(|e| {
        panic!(
            "输出不是单个 JSON 信封：{e}\nstdout={s}\nstderr={}",
            String::from_utf8_lossy(&o.stderr)
        )
    })
}

/// 首次运行给默认值且不报问题；`set` 之后新进程读得到。
#[test]
fn a_setting_persists_into_a_later_process() {
    let d = scratch("persist");
    let v = run(&d, &["show"]);
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["data"]["source"], "defaults", "首次该是默认值");
    assert_eq!(v["data"]["settings"]["log_level"], "info");
    assert!(
        v["data"]["problems"].as_array().unwrap().is_empty(),
        "首次运行不该报问题：{v}"
    );
    assert!(!d.join("settings.json").exists(), "show 不该顺手建文件");

    let v = run(&d, &["set", "log_level", "debug"]);
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["data"]["settings"]["log_level"], "debug");
    assert!(d.join("settings.json").is_file(), "set 该真的落盘");

    let v = run(&d, &["show"]);
    assert_eq!(v["data"]["source"], "file", "新进程该从文件读：{v}");
    assert_eq!(v["data"]["settings"]["log_level"], "debug", "{v}");
}

/// 36 §3：未知字段必须往返保留。
///
/// 反例是"旧版程序读新版设置、原样写回"，新版加的字段会静默消失。
#[test]
fn unknown_fields_survive_a_cli_roundtrip() {
    let d = scratch("unknown");
    run(&d, &["set", "log_level", "warn"]);
    let p = d.join("settings.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    v["written_by_a_newer_version"] = serde_json::json!({"keep": [1, 2, 3]});
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();

    // 改一个受管字段 → 再读文件
    let out = run(&d, &["set", "default_fps", "25"]);
    let preserved = out["data"]["settings"]["preserved_unknown_fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(
        preserved.contains(&"written_by_a_newer_version".to_string()),
        "报告该列出被保留的未知字段：{out}"
    );

    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert_eq!(
        after["written_by_a_newer_version"]["keep"][2], 3,
        "未知字段的内容被丢了：{after}"
    );
    assert_eq!(after["default_fps"], "25");
    assert_eq!(after["log_level"], "warn", "之前的设置不该被重置：{after}");
}

/// `init` 拒绝覆盖已有设置。静默覆盖用户偏好是最容易被骂的那类缺陷。
#[test]
fn init_refuses_to_overwrite_an_existing_file() {
    let d = scratch("init");
    run(&d, &["set", "log_level", "trace"]);
    let v = run(&d, &["init"]);
    assert!(!v["ok"].as_bool().unwrap(), "已有文件时 init 该失败：{v}");
    assert_eq!(v["errors"][0]["code"], "STK-INP-0320", "{v}");
    // 原值必须原封不动
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("settings.json")).unwrap()).unwrap();
    assert_eq!(after["log_level"], "trace", "init 动了已有文件：{after}");
}

/// 从零开始（没有文件）时 `init` 写出一份默认值文件。
#[test]
fn init_writes_a_default_file_when_none_exists() {
    let d = scratch("initnew");
    let v = run(&d, &["init"]);
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert!(d.join("settings.json").is_file());
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(d.join("settings.json")).unwrap()).unwrap();
    assert_eq!(after["schema"], 1);
    assert_eq!(after["log_level"], "info");
}

/// 非法值必须被拒，且**不能写进文件**。
/// 静默接受一个拼错的字段名，用户会以为设置生效了 —— 而它其实写进了没人读的地方。
#[test]
fn invalid_values_are_rejected_and_nothing_is_written() {
    let d = scratch("invalid");
    for args in [
        vec!["set", "log_level", "loud"],
        vec!["set", "default_fps", "not-a-rate"],
        vec!["set", "default_canvas", "1920*1080"],
        vec!["set", "no_such_field", "1"],
        vec!["set", "schema", "2"],
    ] {
        let v = run(&d, &args);
        assert!(!v["ok"].as_bool().unwrap(), "{args:?} 该被拒：{v}");
        assert_eq!(v["errors"][0]["code"], "STK-INP-0322", "{args:?} {v}");
        assert!(
            !d.join("settings.json").exists(),
            "{args:?} 被拒后不该留下文件"
        );
    }
}

/// 坏设置文件不能让程序用不了：退回默认值 + 说出原因 + 原文件留证据。
#[test]
fn a_corrupt_file_degrades_instead_of_blocking() {
    let d = scratch("corrupt");
    let p = d.join("settings.json");
    std::fs::write(&p, "{ not json at all").unwrap();
    let v = run(&d, &["show"]);
    assert!(v["ok"].as_bool().unwrap(), "坏设置不该让命令失败：{v}");
    assert_eq!(v["data"]["source"], "fallback-after-error", "{v}");
    assert_eq!(v["data"]["settings"]["log_level"], "info", "该退回默认值");
    let problems = v["data"]["problems"].as_array().unwrap();
    assert!(!problems.is_empty(), "必须说出发生了什么：{v}");
    let q = v["data"]["quarantined"].as_str().expect("该给出证据路径");
    assert!(Path::new(q).is_file(), "证据文件不在：{q}");
    assert!(!p.exists(), "原路径该已让开，否则下次还会读到它");
}

/// 设置文件落在 `DataDir` 里：便携包内就该落在包内 `data/`。
/// 这条用 `STK_DATA_DIR` 验（`STK_SETTINGS` 是给测试直接指定文件用的旁路）。
#[test]
fn settings_land_in_the_resolved_data_directory() {
    let d = scratch("datadir");
    let o = Command::new(cli())
        .args(["settings", "set", "log_level", "debug", "--json"])
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_SETTINGS")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    let path = v["data"]["path"].as_str().unwrap();
    assert!(
        Path::new(path).starts_with(&d),
        "设置该落在 data dir 里：{path} 不在 {} 下",
        d.display()
    );
    assert!(d.join("settings.json").is_file());
}
