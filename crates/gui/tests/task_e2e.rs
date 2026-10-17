//! GUI 端到端：走 `task_worker::run` 这条**真**通路，验证产物真的落盘。
//!
//! 与 `gui_cli_replay.rs` 的分工：
//! * 那边证"GUI 编译的 spec 与 CLI 手写的一致"（可重放性）；
//! * 这边证"GUI 自己调 run() 时，产物真的出来了"（执行真的接上了）。
//!
//! 两条都要有：只证一致，可能两边都没跑；只证落盘，可能 GUI 走了另一条路。

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use gui::state::profile::{CanvasMode, HdrMode, OutputProfile, TargetFormat, TimeAdjustMode};
use gui::worker::task_worker::Progress;

fn workspace(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-gui-e2e-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,48,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,Hello GUI\r\n";

fn srt(name: &str) -> OutputProfile {
    OutputProfile {
        id: 1,
        enabled: true,
        name: name.to_string(),
        target_format: TargetFormat::Srt,
        canvas: CanvasMode::Fhd1080,
        time_adjust: TimeAdjustMode::None,
        hdr: HdrMode::Sdr,
        custom_suffix: ".srt".into(),
    }
}

/// 走 GUI 的真执行通路，产物必须真的出现。
#[test]
fn the_gui_execution_path_actually_writes_outputs() {
    let dir = workspace("run");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let out_dir = dir.join("out");

    let p = srt("文本导出");
    let (spec, notes) = gui::worker::spec::compile_spec(
        &gui::worker::task_worker::job_id(),
        std::slice::from_ref(&input),
        &[&p],
        &out_dir,
        None,
        "{stem}.{profile}.{ext}",
    );
    assert!(notes.is_empty(), "{notes:?}");

    let mut updates: Vec<Progress> = Vec::new();
    let r = gui::worker::task_worker::run(&spec, &out_dir, Arc::new(AtomicBool::new(false)), |p| {
        updates.push(p)
    });
    assert!(r.is_ok(), "任务必须能跑起来：{r:?}");

    // 1) 必须报过 Started，而且 spec 真的落了盘（"复制到 CLI 重放"靠它）
    let spec_path = updates.iter().find_map(|u| match u {
        Progress::Started { spec_path, .. } => Some(spec_path.clone()),
        _ => None,
    });
    let spec_path = spec_path.expect("必须报 Started");
    assert!(
        spec_path.is_file(),
        "spec 必须落盘：{}",
        spec_path.display()
    );
    let spec_text = std::fs::read_to_string(&spec_path).unwrap();
    assert!(
        spec_text.contains("\"gui-batch\""),
        "spec 内容不对：{spec_text}"
    );

    // 2) 每一行都要有状态，且状态取自 CLI 的词表
    let rows: Vec<(String, String)> = updates
        .iter()
        .filter_map(|u| match u {
            Progress::Row {
                profile, status, ..
            } => Some((profile.clone(), status.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "一份输入×一个 profile 应当只有一行：{rows:?}"
    );
    assert_eq!(rows[0].0, "文本导出");
    assert!(
        matches!(
            rows[0].1.as_str(),
            "verified" | "from-cache" | "would-write"
        ),
        "状态必须是 CLI 的词表里的：{}",
        rows[0].1
    );

    // 3) 整体判定
    let outcome = updates.iter().find_map(|u| match u {
        Progress::Finished { outcome, .. } => Some(outcome.clone()),
        _ => None,
    });
    assert_eq!(outcome.as_deref(), Some("success"));

    // 4) **产物真的在盘上**——这一条才是"接上了"的定义
    let srt_files: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("srt"))
        .collect();
    assert_eq!(srt_files.len(), 1, "应当产出一个 SRT：{srt_files:?}");
    let text = std::fs::read_to_string(&srt_files[0]).unwrap();
    assert!(text.contains("Hello GUI"), "内容必须来自输入：{text}");

    // 5) 报告也落了盘
    assert!(
        out_dir.join("job-report.json").is_file(),
        "报告必须落盘（用户要能看到逐条状态）"
    );
}

/// 起不来的时候**不能**报 Started，也不能留下半成品消息。
#[test]
fn a_task_that_cannot_start_reports_no_progress_rows() {
    let dir = workspace("nostart");
    let p = srt("X");
    let (spec, _) = gui::worker::spec::compile_spec(
        "bad",
        &[dir.join("whatever.ass")],
        &[&p],
        // 把"目录"指向一个普通文件：create_dir_all 必然失败
        &dir.join("a.ass"),
        None,
        "{stem}.{ext}",
    );
    std::fs::write(dir.join("a.ass"), b"x").unwrap();

    let mut updates: Vec<Progress> = Vec::new();
    let r = gui::worker::task_worker::run(
        &spec,
        &dir.join("a.ass"),
        Arc::new(AtomicBool::new(false)),
        |p| updates.push(p),
    );
    assert!(r.is_err(), "必须失败");
    assert!(updates.is_empty(), "起不来就不该报任何进度：{updates:?}");
}

/// 取消后不能再往下跑。
#[test]
fn a_cancelled_task_stops_before_running_profiles() {
    let dir = workspace("cancel");
    let input = dir.join("a.ass");
    std::fs::write(&input, ASS).unwrap();
    let out_dir = dir.join("out");
    let p = srt("X");
    let (spec, _) = gui::worker::spec::compile_spec(
        "c",
        std::slice::from_ref(&input),
        &[&p],
        &out_dir,
        None,
        "{stem}.{ext}",
    );

    let mut updates: Vec<Progress> = Vec::new();
    let r = gui::worker::task_worker::run(
        &spec,
        &out_dir,
        Arc::new(AtomicBool::new(true)), // 一开始就取消
        |p| updates.push(p),
    );
    assert!(r.is_ok(), "取消不是错误");
    let rows: Vec<_> = updates
        .iter()
        .filter(|u| matches!(u, Progress::Row { .. }))
        .collect();
    assert!(rows.is_empty(), "取消后不该有逐行结果：{rows:?}");
    let outcome = updates.iter().find_map(|u| match u {
        Progress::Finished { outcome, .. } => Some(outcome.clone()),
        _ => None,
    });
    assert_eq!(outcome.as_deref(), Some("cancelled"));
    // 取消后不该产出字幕
    let produced = std::fs::read_dir(&out_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("srt"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(produced, 0, "取消后不该有字幕产物");
}

/// AT-UI-001 的实质：**取消令牌真的连到了引擎**。
///
/// 两件事分开证，因为它们的可观测性不同：
///
/// 1. **令牌连上了**：给 `run` 一个已经置位的令牌，它必须停在工作前（下面那个
///    `a_cancelled_task_stops_before_running_profiles` 已经证了）。
/// 2. **中途置位也有效**：引擎在领每一格之前看令牌（`runner.rs` 的
///    `if cancel.cancelled() { return; }`），而 GUI 的令牌就是它的同一份。
///    这一条只能靠“令牌确实是同一份”来证——而那是类型层面的事实：
///    `run_batch_with_cancel` 收的就是 `CancelToken::from_shared(flag)`。
///
/// 之所以不写“跑到一半从回调里置位”那种用例：GUI 的逐行结果是在
/// `run_batch` **返回之后**才从报告里翻译出来的，回调根本没有“中途”这个时机。
/// 写成那样只会得到一条永远绿（或永远红）的假断言。
#[test]
fn the_gui_cancel_flag_is_the_same_flag_the_engine_checks() {
    // 类型层面的证据：`from_shared` 拿到的就是调用方那个 Arc。
    let flag = Arc::new(AtomicBool::new(false));
    let token = task_engine::CancelToken::from_shared(flag.clone());
    assert!(!token.cancelled(), "刚建出来不该是取消状态");

    // 调用方置位 = 引擎看得到
    flag.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        token.cancelled(),
        "外部置位必须被引擎看到，否则取消按钮就是个装饰"
    );

    // 反方向：反向共享也成立（`shared()` 给回同一个 Arc）
    let t2 = task_engine::CancelToken::new();
    let s = t2.shared();
    t2.cancel();
    assert!(
        s.load(std::sync::atomic::Ordering::SeqCst),
        "引擎内部置位也必须对调用方可见"
    );
}
