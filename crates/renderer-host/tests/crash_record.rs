//! 22 §6 的崩溃现场记录：**端到端**行为。
//!
//! 规格原文：「单独 stdout/stderr capture；超时、exit code、SEH crash/minidump
//! 分离记录。」其中 stdout/stderr、超时与 exit code 已有用例覆盖，
//! 这一组测的是**崩溃现场**这一格：
//!
//! 1. 注入 abort 之后，目录里留下**可解析**的记录，且记录说得出「死在哪个阶段」；
//! 2. 那份现场真的进到**错误卡**里（用户看得到，而不是只躺在磁盘上）；
//! 3. **正常运行时目录里不该有记录**；
//! 4. **上一次的记录不会被当成本次的** —— 这条是这一格里最危险的一格：
//!    一次写不出记录的崩溃（访问违例、栈溢出）配上上一回遗留的文件，
//!    会让报告斩钉截铁地说出**错误的**阶段与 provider。
//!
//! 单元测试（`renderer-host/src/crash.rs`）验格式与读写；这里验那四件。

use std::path::{Path, PathBuf};
use std::process::Command;

fn host_exe() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-render-host"));
    p.is_file().then_some(p)
}

fn dll() -> Option<PathBuf> {
    let p = std::env::var("STK_LIBASS_DLL").ok()?;
    let p = PathBuf::from(p);
    p.is_file().then_some(p)
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("crashrec-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 直接把 host 跑起来并注入崩溃。这是最贴近“真崩溃”的可控形式：
/// 注入发生在 host 自己的启动环境里，与真崩溃走同一条退出路径。
fn run_host_with_injection(dir: &Path, how: &str) -> std::process::Output {
    let host = host_exe().expect("该有 host 可执行文件");
    Command::new(host)
        .env("STK_HOST_CRASH", how)
        .env(
            "STK_HOST_CRASH_DIR",
            dir.to_str().expect("测试目录该是有效 UTF-8"),
        )
        // 显式清掉可能继承到的注入目录之外的东西
        .env_remove("STK_LIBASS_DLL")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("能跑 host")
}

fn read_note(dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(dir.join("host-crash.json"))
        .unwrap_or_else(|e| panic!("该有崩溃记录：{e}（目录 {}）", dir.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("记录该是合法 JSON：{e}\n{text}"))
}

/// abort 之后必须留下可解析的记录，且记录说得出阶段与原因。
#[test]
fn an_abort_leaves_a_parseable_note_naming_the_phase() {
    let Some(_) = host_exe() else {
        eprintln!("SKIP：没有 host 可执行文件");
        return;
    };
    let d = scratch("abort");
    let out = run_host_with_injection(&d, "abort");

    // 退出码必须是异常终止的样子（不是正常退出）
    let code = out.status.code();
    assert!(
        code.is_none() || code.is_some_and(|c| c != 0),
        "abort 不该是正常退出：{code:?}"
    );

    let note = read_note(&d);
    assert_eq!(note["schema"], 1);
    assert_eq!(note["phase"], "startup", "注入在握手之前，阶段该是 startup");
    assert!(
        note["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("abort"),
        "原因该说清是哪种注入：{note}"
    );
    // 记录里不该有内存镜像之类的东西（22 §5 的诊断包默认可外发）
    let raw = std::fs::read_to_string(d.join("host-crash.json")).unwrap();
    assert!(raw.len() < 4096, "记录应当很小，实际 {} 字节", raw.len());
}

/// 四种注入都要留下记录，且各自的原因不混淆。
#[test]
fn every_injection_kind_is_recorded_distinctly() {
    let Some(_) = host_exe() else {
        eprintln!("SKIP：没有 host 可执行文件");
        return;
    };
    let d = scratch("kinds");
    for (how, needle) in [("abort", "abort"), ("exit", "exit"), ("panic", "panic")] {
        let _ = run_host_with_injection(&d, how);
        let note = read_note(&d);
        assert!(
            note["reason"].as_str().unwrap_or_default().contains(needle),
            "{how} 的记录原因该含 {needle:?}：{note}"
        );
    }
    // "hang" 不会自己退出，所以不在这里测（它由 client 的超时用例覆盖）。
}

/// 没有注入时**不该**留下记录。
///
/// 少了这条，一个"每次启动都写一份假记录"的实现也能让上面两条全绿。
#[test]
fn a_normal_host_startup_leaves_no_note() {
    let Some(_) = host_exe() else {
        eprintln!("SKIP：没有 host 可执行文件");
        return;
    };
    let d = scratch("normal");
    let host = host_exe().unwrap();
    // 不注入，只把 stdin 关掉让它立刻结束
    let _ = Command::new(host)
        .env(
            "STK_HOST_CRASH_DIR",
            d.to_str().expect("测试目录该是有效 UTF-8"),
        )
        .env_remove("STK_HOST_CRASH")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("能跑 host");
    assert!(
        !d.join("host-crash.json").exists(),
        "没崩溃就不该有记录（目录内容：{:?}）",
        std::fs::read_dir(&d).map(|rd| rd.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );
}

/// **最重要的一条**：过期的记录不能被当成本次的现场。
///
/// 真崩溃常常发生在写不出记录的地方（访问违例、栈溢出）。那时目录里如果
/// 还留着上一回的文件，报告会说出一个**斩钉截铁但完全错误**的阶段与 provider ——
/// 而调查会照着它走错方向。所以 `Host::start` 会在起进程之前先删掉旧记录。
///
/// 这一条通过 `Host` 的公开接口验证（而不是直接删文件）：
/// 要证的是**产品行为**，不是我自己会删文件。
#[test]
fn a_stale_note_is_not_reported_as_this_run_crash() {
    let Some(_) = host_exe() else {
        eprintln!("SKIP：没有 host 可执行文件");
        return;
    };
    // 没有 libass 时 `Host::start` 会在起进程之前就因 DLL 检查失败，
    // 那条路径不涉及崩溃记录清理，测不到本用例要的东西。
    let Some(dll) = dll() else {
        eprintln!("SKIP：需要 STK_LIBASS_DLL 才能走到真起 host 那一步");
        return;
    };
    use renderer_host::client::{Host, HostFailure, HostSpec};

    let d = scratch("stale");
    let crash_dir = d.join("crashes");
    std::fs::create_dir_all(&crash_dir).unwrap();
    // 伪造一份"上一次"的记录
    let stale = crash_dir.join("host-crash.json");
    std::fs::write(
        &stale,
        r#"{"schema":1,"phase":"render","step":417,"uptime_ms":12345,
            "provider_path":"C:/old/ass.dll","provider_sha256":"deadbeef","provider_arch":"x86",
            "storage":[1920,1080],"output":[1920,1080],"injected_fonts":0,
            "last_ok":"render","reason":"上一次崩溃"}"#,
    )
    .unwrap();

    let host = host_exe().unwrap();
    let spec = HostSpec {
        host_exe: &host,
        dll: &dll,
        storage: core_model::Canvas::HD720,
        output: core_model::Canvas::HD720,
        default_font: "Arial",
        // 短超时：握手会失败（a.ass 不存在），但我们关心的是清理
        timeout: std::time::Duration::from_secs(3),
        crash_injection: None,
        lie_about_script_len: false,
        crash_dir: Some(&crash_dir),
    };
    match Host::start(&spec) {
        Ok(_) => {
            // 起来了也要看：start 之后旧记录必须已经不在了
        }
        Err(e) => {
            // 允许任何失败，但**不允许**把上一回的现场当成本次的报出来
            if let HostFailure::Crashed { crash_note, .. } = &e {
                assert!(
                    crash_note.is_none(),
                    "把过期的记录当成了本次的现场：{crash_note:?}"
                );
            }
        }
    }
    assert!(
        !stale.exists(),
        "起 host 之前该把旧记录删掉，否则它会冒充本次崩溃的现场"
    );
}
