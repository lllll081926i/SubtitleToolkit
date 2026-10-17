//! 22 §2 correlation ID 的**端到端**行为。
//!
//! 规格原文只有两句：「每个 Job/Output/renderer session 都有 UUID；
//! 用户看到的错误卡可复制 ID。」这两句各自对应可判定的行为：
//!
//! 1. **可复制**：人读输出里有一行能抄下来的 id，且它与 JSON 里的值是同一个；
//! 2. **能对上**：日志里那条记录的 id 与用户报告里的 id 必须**是同一个值** ——
//!    "看起来像 UUID"不够，分隔两处就毫无用处；
//! 3. **有层级**：批量里每个输出各有一个 id，且它们共享同一个 Job 段。
//!
//! 单元测试（`core-model/src/corrid.rs`）验格式与唯一性；这里验那三件用户看得见的事。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("corr-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 跑一条命令，返回（信封, 原始 stdout）。
fn run(dir: &Path, args: &[&str]) -> (serde_json::Value, String) {
    let o = Command::new(cli())
        .args(args)
        .env("STK_DATA_DIR", dir)
        .env_remove("STK_SETTINGS")
        .env_remove("STK_LOG")
        .output()
        .expect("跑 CLI");
    let s = String::from_utf8_lossy(&o.stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(s.trim())
        .unwrap_or_else(|e| panic!("不是单个 JSON 信封：{e}\nstdout={s}"));
    (v, s)
}

fn log_text(dir: &Path) -> String {
    let logs = dir.join("logs");
    let mut all = String::new();
    let Ok(rd) = std::fs::read_dir(&logs) else {
        return all;
    };
    for e in rd.flatten() {
        all.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
    }
    all
}

/// 成功与失败的信封都要带 corr_id，且形状是 `J-8hex-12hex`。
#[test]
fn every_envelope_carries_a_well_formed_correlation_id() {
    let d = scratch("shape");
    let (v, _) = run(&d, &["doctor", "--json"]);
    let id = v["corr_id"].as_str().expect("成功信封该有 corr_id");
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 3, "{id}");
    assert_eq!(parts[0], "J");
    assert_eq!(parts[1].len(), 8, "{id}");
    assert_eq!(parts[2].len(), 12, "{id}");

    let (v, _) = run(&d, &["probe", "D:\\definitely-missing-xyz.srt", "--json"]);
    assert!(!v["ok"].as_bool().unwrap(), "{v}");
    assert!(
        v["corr_id"].as_str().is_some_and(|s| s.starts_with("J-")),
        "失败信封也要有：{v}"
    );
}

/// **这条是本文件最重要的断言**：日志里那个 id 与报告里的 id 必须是同一个值。
///
/// 分开生成过一版，结果是"两边都像 UUID 但对不上"——用户贴出来的 id
/// 在日志里搜不到。correlation id 的全部用途就是让这两处是同一个。
#[test]
fn the_id_in_the_log_is_the_same_value_as_the_one_in_the_report() {
    let d = scratch("match");
    let (v, _) = run(&d, &["probe", "D:\\definitely-missing-xyz.srt", "--json"]);
    let reported = v["corr_id"].as_str().unwrap().to_string();
    let logs = log_text(&d);
    assert!(!logs.is_empty(), "该有日志");
    assert!(
        logs.contains(&reported),
        "报告里的 id {reported} 在日志里找不到。\n日志：\n{logs}"
    );
    // 失败那条日志也带上了正确的错误码（22 §4 的联动）
    assert!(logs.contains("STK-INP-0110"), "{logs}");
}

/// 两次独立执行的 id 必须不同 —— 否则"定位到那一次"不成立。
#[test]
fn two_runs_get_different_ids() {
    let d = scratch("distinct");
    let (a, _) = run(&d, &["doctor", "--json"]);
    let (b, _) = run(&d, &["doctor", "--json"]);
    let (ia, ib) = (
        a["corr_id"].as_str().unwrap(),
        b["corr_id"].as_str().unwrap(),
    );
    assert_ne!(ia, ib, "两次跑了同一个 id：{ia}");
}

/// 22 §2 的「错误卡可复制 ID」：人读输出里有一行能直接抄的 id。
#[test]
fn the_text_output_has_a_copyable_id_line() {
    let d = scratch("text");
    // 失败时也要有（那才是用户最需要抄下来的时候）
    let o = Command::new(cli())
        .args(["probe", "D:\\definitely-missing-xyz.srt"])
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_LOG")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    let line = out
        .lines()
        .find(|l| l.starts_with("corr-id: "))
        .unwrap_or_else(|| panic!("人读输出里没有 corr-id 行：\n{out}"));
    let id = line.trim_start_matches("corr-id: ").trim();
    // 那一行本身必须能直接用：不含空格、路径、引号
    assert!(!id.contains(' '), "{id:?}");
    assert!(!id.contains(['\\', '/', ':']), "{id:?}");
    assert!(id.starts_with("J-"), "{id:?}");

    // 且它必须与日志里的一致
    let logs = log_text(&d);
    assert!(logs.contains(id), "文本输出的 id {id} 在日志里找不到");
}

/// 批量：信封上的 id 必须与 `data.corr_id` 是**同一个**，且每个输出各有 id。
///
/// 这两条曾经都不成立：信封由 `dispatch` 生成、`data` 由批量路径生成，
/// 于是报告里是两个不同的值，而输出 id 的共享段对不上其中任何一个。
#[test]
fn a_batch_gives_one_job_id_and_one_id_per_output() {
    let d = scratch("batch");
    // 两个**纯文本** profile（不涉渲染）：本用例关心的是 id 而不是产物，
    // 而渲染 profile 在没有 libass 的机器上会让批量在早期就失败，
    // 那时根本走不到“生成 output_ids”那一步。
    // 两份输入是因为 srt→srt 会被正当拒绝，需要另一份来产生两个输出。
    std::fs::write(d.join("a.srt"), "1\n00:00:01,000 --> 00:00:02,000\nHi\n").unwrap();
    let spec = d.join("s.json");
    std::fs::write(
        &spec,
        r#"{"id":"my-task","roots":["."],"include":["*.srt"],
 "profiles":[{"name":"p1","format":"ass"},{"name":"p2","format":"ass","canvas":"1920x1080"}],
 "output":{"dir":"out"},"video_association":false}"#,
    )
    .unwrap();
    let (v, _) = run(&d, &["batch", "--spec", spec.to_str().unwrap(), "--json"]);
    assert!(v["data"]["output_ids"].is_array(), "{v}");

    let env_id = v["corr_id"].as_str().expect("信封该有 corr_id");
    let data_id = v["data"]["corr_id"].as_str().expect("data 该有 corr_id");
    assert_eq!(
        env_id, data_id,
        "信封与报告的 corr_id 不一致（这正是要防的那个缺陷）"
    );
    // `job_id` 是用户给的名字，与本次执行的 id 不是一回事
    assert_eq!(v["data"]["job_id"], "my-task");
    assert_ne!(env_id, "my-task");

    let stem = env_id.split('-').nth(1).unwrap();
    let outs = v["data"]["output_ids"].as_array().expect("该有 output_ids");
    assert_eq!(outs.len(), 2, "{v}");
    assert_eq!(outs[0]["profile"], "p1");
    assert_eq!(outs[1]["profile"], "p2");
    for o in outs {
        let id = o["corr_id"].as_str().unwrap();
        assert!(id.starts_with("O-"), "{id}");
        assert_eq!(
            id.split('-').nth(1).unwrap(),
            stem,
            "输出 id 的共享段该与 Job 一致：{id} vs {env_id}"
        );
    }
    // 两个输出之间必须不同
    assert_ne!(
        outs[0]["corr_id"].as_str().unwrap(),
        outs[1]["corr_id"].as_str().unwrap()
    );
}

/// 22 §2 的字符集约束：id 会被贴进公开的 issue，不能携带用户数据。
#[test]
fn an_id_never_carries_user_data() {
    let d = scratch("privacy");
    // 目录名刻意带中文与空格：如果 id 里混进路径，这条就会失败
    let dir = d.join("用户 的 目录");
    std::fs::create_dir_all(&dir).unwrap();
    let (v, _) = run(&dir, &["doctor", "--json"]);
    let id = v["corr_id"].as_str().unwrap();
    for ch in id.chars() {
        assert!(
            ch.is_ascii_hexdigit() || ch == '-' || "JOR".contains(ch),
            "id 里有可疑字符 {ch:?}（{id}）"
        );
    }
    assert!(!id.contains("用户"), "{id}");
}
