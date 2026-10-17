//! portable 数据目录的端到端验收（28 §4）。
//!
//! 判据只有一条，但必须用**真 exe** 证明：可执行文件旁边有 `portable.flag` 时，
//! 运行期状态（这里是字体库缓存）落在同级 `data/`，而不是用户 profile。
//!
//! 为什么不用单元测试代替：`DataDir::resolve` 的单元测试证明的是函数逻辑，
//! 而"CLI 真的把缓存写过去了"要靠真进程 + 真命令 + 真文件系统才能证明。

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-portable-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 把当前测试的 CLI 及其 host 一起拷到一个隔离目录（否则 exe 旁边的 flag 落在仓库里）。
fn stage_exe(dir: &Path) -> PathBuf {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-cli"));
    let target = dir.join(exe.file_name().unwrap());
    std::fs::copy(&exe, &target).unwrap();
    // host 是可选的；拷不过去也能跑 doctor/font scan（那些命令不需要渲染）
    let host_src = exe.parent().unwrap().join(format!(
        "subtitletool-render-host{}",
        std::env::consts::EXE_SUFFIX
    ));
    if host_src.is_file() {
        let _ = std::fs::copy(&host_src, dir.join(host_src.file_name().unwrap()));
    }
    target
}

fn doctor(exe: &Path, data_dir: Option<&Path>) -> serde_json::Value {
    let mut cmd = Command::new(exe);
    cmd.args(["doctor", "--json"]);
    // 让子进程看不到父进程的覆盖，否则测的就不是 portable.flag 了
    cmd.env_remove("STK_DATA_DIR");
    if let Some(d) = data_dir {
        cmd.env("STK_DATA_DIR", d);
    }
    let out = cmd.output().expect("CLI 必须能启动");
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("没有 JSON 行：{text:?}"));
    serde_json::from_str(line).unwrap()
}

/// 28 §4：`portable.flag` 让数据目录切到 exe 同级的 `data/`。
#[test]
fn portable_flag_next_to_the_exe_switches_the_data_dir() {
    let dir = workspace("flag");
    let exe = stage_exe(&dir);

    // 1) 没有 flag：不走 portable
    let before = doctor(&exe, None);
    assert_eq!(before["ok"], true);
    assert_ne!(
        before["data"]["data_dir"]["kind"], "portable",
        "没有 portable.flag 就不该走便携：{before}"
    );
    assert!(
        before["data"]["data_dir"]["why"].is_string(),
        "结论必须带理由，否则\"便携没生效\"只能靠猜：{before}"
    );

    // 2) 放上 flag：切到同级 data/
    std::fs::write(dir.join("portable.flag"), b"").unwrap();
    let after = doctor(&exe, None);
    assert_eq!(after["data"]["data_dir"]["kind"], "portable", "{after}");
    let expected = dir.join("data");
    assert_eq!(
        after["data"]["data_dir"]["path"].as_str().unwrap(),
        expected.display().to_string(),
        "{after}"
    );
    assert_eq!(after["data"]["data_dir"]["portable_flag"], "portable.flag");
    // 缓存路径必须跟着走，否则"数据落在包里"只是一句口号
    assert_eq!(
        after["data"]["font_cache"]["path"].as_str().unwrap(),
        expected.join("fontdb-v1.json").display().to_string(),
        "{after}"
    );
    assert_eq!(after["data"]["font_cache"]["source"], "data-dir");
}

/// 3) `STK_DATA_DIR` 的优先级高于 `portable.flag`（门阶需要"我明确指定一个目录"）。
#[test]
fn explicit_data_dir_wins_over_the_portable_flag() {
    let dir = workspace("explicit");
    let exe = stage_exe(&dir);
    std::fs::write(dir.join("portable.flag"), b"").unwrap();
    let chosen = dir.join("chosen");
    let v = doctor(&exe, Some(&chosen));
    assert_eq!(v["data"]["data_dir"]["kind"], "explicit", "{v}");
    assert_eq!(
        v["data"]["data_dir"]["path"].as_str().unwrap(),
        chosen.display().to_string(),
        "{v}"
    );
    assert_eq!(
        v["data"]["font_cache"]["path"].as_str().unwrap(),
        chosen.join("fontdb-v1.json").display().to_string(),
        "{v}"
    );
}

/// 4) 真的写盘：跑一次会用缓存路径的命令，确认状态落在便携目录里。
#[test]
fn runtime_state_lands_inside_the_portable_directory() {
    let dir = workspace("state");
    let exe = stage_exe(&dir);
    std::fs::write(dir.join("portable.flag"), b"").unwrap();

    let ass = dir.join("t.ass");
    std::fs::write(
        &ass,
        "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 640\r\nPlayResY: 480\r\n\r\n\
         [V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, \
         Spacing, MarginL, MarginR, MarginV\r\n\
         Style: Default,Arial,32,&H00FFFFFF,2,1,0,20,20,20\r\n\r\n\
         [Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, \
         Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hi\r\n",
    )
    .unwrap();

    let out = Command::new(&exe)
        .args(["font", "scan", ass.to_str().unwrap(), "--json"])
        .env_remove("STK_DATA_DIR")
        .env_remove("STK_FONTCACHE")
        .output()
        .expect("CLI 必须能启动");
    assert!(
        out.status.success(),
        "font scan 必须成功：{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let cache = dir.join("data").join("fontdb-v1.json");
    assert!(
        cache.is_file(),
        "便携模式下缓存必须落在 {}（而不是用户 profile）",
        cache.display()
    );
    assert!(
        std::fs::metadata(&cache).unwrap().len() > 0,
        "缓存文件不能是空的"
    );
}
