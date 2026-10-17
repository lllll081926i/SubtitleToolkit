//! 22 §5 诊断包的**端到端**行为。
//!
//! 规格里最要紧的一句是「**默认不包含原字幕/视频**；用户显式选择才附样本」。
//! 所以这一组测试的重心不是"包能不能打开"，而是**默认发出去的东西里有没有用户内容**。
//!
//! 单元测试（`cli/src/diagnostic.rs`）验编码与脱敏规则；这里验用户看得见的行为。

use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"))
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("diag-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 跑 `diagnose`，把包写到 `dir/out.zip`，返回信封与包字节。
///
/// 每次都把上一次的 `out.zip*` 清掉：`--overwrite` 默认是 `Rename`（15 §6，
/// 绝不直接截断已有文件），所以同一个路径跑两次会得到 `out.zip` 与 `out.1.zip`。
/// 测试若不断言覆盖策略，就必须自己保证读的是这一次写的那份。
fn diagnose(dir: &Path, extra: &[&str]) -> (serde_json::Value, Vec<u8>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("zip") {
            let _ = std::fs::remove_file(&p);
        }
    }
    let out = dir.join("out.zip");
    let mut c = Command::new(cli());
    c.arg("diagnose")
        .arg("--json")
        .args(["--out", out.to_str().unwrap()])
        .args(extra)
        // 数据目录指到临时目录：诊断包要能报出"设置在哪"，而那条路径会过脱敏
        .env("STK_DATA_DIR", dir)
        .env_remove("STK_SETTINGS");
    let o = c.output().expect("跑 CLI");
    let s = String::from_utf8_lossy(&o.stdout);
    let v: serde_json::Value = serde_json::from_str(s.trim()).unwrap_or_else(|e| {
        panic!(
            "输出不是单个 JSON 信封：{e}\nstdout={s}\nstderr={}",
            String::from_utf8_lossy(&o.stderr)
        )
    });
    let bytes = std::fs::read(&out).unwrap_or_else(|e| panic!("包没写出来：{e}"));
    // 报告说的路径必须就是刚读的那份。少了这条，一个把产物写到别处的缺陷
    // 会被"反正读到了东西"掩盖。
    assert_eq!(
        v["data"]["path"].as_str().unwrap_or_default(),
        out.display().to_string(),
        "报告里的路径与磁盘上的实际产物不符"
    );
    (v, bytes)
}

/// 用独立解析器确认写出的东西是**结构完整的 zip**。
///
/// 这里刻意不复用生产码的 `verify_zip`：那只能证明"写入与自己的校验一致"。
/// 这个读取器只看 ZIP 的公开布局（EOCD → 中央目录 → 本地头 → 数据），
/// 所以它能揭穿"写入与校验有同一个误解"。
fn zip_entries(zip: &[u8]) -> Vec<(String, Vec<u8>)> {
    let eocd = zip.len() - 22;
    assert_eq!(
        u32::from_le_bytes(zip[eocd..eocd + 4].try_into().unwrap()),
        0x0605_4B50,
        "结尾没有 EOCD"
    );
    let count = u16::from_le_bytes(zip[eocd + 10..eocd + 12].try_into().unwrap()) as usize;
    let cd = u32::from_le_bytes(zip[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let mut at = cd;
    let mut out = Vec::new();
    for _ in 0..count {
        assert_eq!(
            u32::from_le_bytes(zip[at..at + 4].try_into().unwrap()),
            0x0201_4B50,
            "@{at} 不是中央目录项"
        );
        let method = u16::from_le_bytes(zip[at + 10..at + 12].try_into().unwrap());
        let crc = u32::from_le_bytes(zip[at + 16..at + 20].try_into().unwrap());
        let csize = u32::from_le_bytes(zip[at + 20..at + 24].try_into().unwrap()) as usize;
        let nlen = u16::from_le_bytes(zip[at + 28..at + 30].try_into().unwrap()) as usize;
        let elen = u16::from_le_bytes(zip[at + 30..at + 32].try_into().unwrap()) as usize;
        let clen = u16::from_le_bytes(zip[at + 32..at + 34].try_into().unwrap()) as usize;
        let lho = u32::from_le_bytes(zip[at + 42..at + 46].try_into().unwrap()) as usize;
        let name =
            String::from_utf8(zip[at + 46..at + 46 + nlen].to_vec()).expect("名字不是 UTF-8");
        let l_nlen = u16::from_le_bytes(zip[lho + 26..lho + 28].try_into().unwrap()) as usize;
        let l_elen = u16::from_le_bytes(zip[lho + 28..lho + 30].try_into().unwrap()) as usize;
        let data_at = lho + 30 + l_nlen + l_elen;
        let payload = &zip[data_at..data_at + csize];
        let data = match method {
            0 => payload.to_vec(),
            8 => {
                use flate2::read::DeflateDecoder;
                use std::io::Read;
                let mut d = DeflateDecoder::new(payload);
                let mut buf = Vec::new();
                d.read_to_end(&mut buf).expect("deflate 解不开");
                buf
            }
            m => panic!("不支持的方法 {m}"),
        };
        // CRC 必须对：这是"内容没坏"的可判定证据
        let mut h = crc32fast(&data);
        h = !h;
        assert_eq!(h, crc, "CRC 不符（{name}）");
        out.push((name, data));
        at += 46 + nlen + elen + clen;
    }
    out
}

/// CRC-32（与 ZIP 同一条多项式），独立实现，不用被测代码的表。
fn crc32fast(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for b in data {
        c ^= *b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    c
}

/// 默认包：结构完整、含环境与依赖身份、且**不含用户内容**。
#[test]
fn a_default_bundle_is_a_valid_zip_without_any_user_content() {
    let d = scratch("default");
    let (v, zip) = diagnose(&d, &[]);
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert_eq!(v["data"]["contains_user_content"], false, "{v}");
    assert!(
        v["data"]["user_content_entries"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{v}"
    );

    let e = zip_entries(&zip);
    let names: Vec<&str> = e.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"environment.json"), "{names:?}");
    assert!(names.contains(&"dependencies.json"), "{names:?}");
    assert!(names.contains(&"settings.json"), "{names:?}");
    assert!(
        names.iter().all(|n| !n.starts_with("input/")),
        "默认包里不该有任何 input/ 条目：{names:?}"
    );

    // 环境条目要能当 JSON 解，并含平台信息（22 §5 要求）
    let env: serde_json::Value =
        serde_json::from_slice(&e.iter().find(|(n, _)| n == "environment.json").unwrap().1)
            .expect("environment.json 该是合法 JSON");
    assert_eq!(env["os"], std::env::consts::OS);
    assert!(env["app_version"].is_string(), "{env}");
}

/// 设置里的路径必须脱敏：诊断包是要发出去的，路径含用户名。
#[test]
fn settings_are_redacted_so_paths_do_not_leak() {
    let d = scratch("redact");
    let (_, zip) = diagnose(&d, &[]);
    let e = zip_entries(&zip);
    let raw = String::from_utf8_lossy(&e.iter().find(|(n, _)| n == "settings.json").unwrap().1);
    // 数据目录被设成临时目录，它的真实路径绝不能出现
    assert!(
        !raw.contains(&d.display().to_string()),
        "设置里的路径泄漏了：{raw}"
    );
    let st: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(st["path"], "⟨已脱敏⟩", "{raw}");
    // 非敏感项仍要在（否则脱敏就变成了删除信息）
    assert!(st["settings"]["log_level"].is_string(), "{raw}");
}

/// 显式附样本时才含用户内容，且报告要指出来。
#[test]
fn input_is_included_only_when_explicitly_requested_and_is_flagged() {
    let d = scratch("input");
    let ass = d.join("private.ass");
    std::fs::write(&ass, "[Script Info]\nTitle: 用户私有内容\n").unwrap();

    let before = zip_entries(&diagnose(&d, &[]).1);
    assert!(
        before.iter().all(|(n, _)| !n.starts_with("input/")),
        "不点名就不该收"
    );

    let (v, zip) = diagnose(&d, &["--include-input", ass.to_str().unwrap()]);
    assert_eq!(v["data"]["contains_user_content"], true, "{v}");
    assert_eq!(
        v["data"]["user_content_entries"][0], "input/private.ass",
        "{v}"
    );
    let e = zip_entries(&zip);
    let names: Vec<String> = e.iter().map(|(n, _)| n.clone()).collect();
    let got = e
        .iter()
        .find(|(n, _)| n == "input/private.ass")
        .unwrap_or_else(|| panic!("点名的文件该在包里，实际条目={names:?}"));
    assert!(
        String::from_utf8_lossy(&got.1).contains("用户私有内容"),
        "收进来的内容该是原样的"
    );
}

/// `--dry-run` 不落盘（15 §6：与其它产物同一条规矩）。
#[test]
fn dry_run_does_not_write_the_bundle() {
    let d = scratch("dry");
    let out = d.join("dry.zip");
    let o = Command::new(cli())
        .args(["diagnose", "--json", "--dry-run", "--out"])
        .arg(&out)
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_SETTINGS")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    assert!(v["ok"].as_bool().unwrap(), "{v}");
    assert!(!out.exists(), "--dry-run 不该落盘：{}", out.display());
}

/// 诊断包写不进去时要说清楚，而不是默默成功。
#[test]
fn an_unwritable_target_reports_a_real_error() {
    let d = scratch("unwritable");
    // 目标路径的父目录是一个**文件**，创建目录必然失败
    let blocker = d.join("blocker");
    std::fs::write(&blocker, "x").unwrap();
    let out = blocker.join("sub").join("x.zip");
    let o = Command::new(cli())
        .args(["diagnose", "--json", "--out"])
        .arg(&out)
        .env("STK_DATA_DIR", &d)
        .env_remove("STK_SETTINGS")
        .output()
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&o.stdout).trim()).unwrap();
    assert!(!v["ok"].as_bool().unwrap(), "该失败：{v}");
    assert!(!v["errors"].as_array().unwrap().is_empty(), "{v}");
    assert!(!out.exists());
}
