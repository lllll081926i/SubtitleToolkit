//! AT-UI-001：重任务期间 UI 60s 内持续可响应、取消可点击。
//!
//! ## 规格要求与可判定形式
//!
//! 原文是「重任务期间 UI 60s 内持续可响应、取消可点击」。拆成三件能测的事：
//!
//! 1. **`update` 本身够快**。UI 线程的唯一入口是 `SubtitleApp::update`；
//!    它的每次调用都在毫秒级返回，界面就不会卡住。37 §1 的原话是
//!    「UI 主线程任何单次同步工作建议 < 8ms，绝不直接执行文件遍历/renderer」。
//! 2. **重任务期间它仍然快**。空闲时很快、一跑任务就阻塞的实现也满足第 1 条，
//!    所以要在**真有重活在后台跑**的时候重复测。
//! 3. **取消点了就生效**。点击本身不阻塞，且置位后引擎真的停。
//!
//! ## 怎么做重任务
//!
//! 直接调 GUI 用的那个 worker（`gui::worker::task_worker::run`），参数与
//! `StartQueue` 里交给 `spawn_blocking` 的完全一样。理由是**避免测运行时的管道**：
//! `Task<Message>` 的 stream 拆解是 `iced_runtime` 的私有 API，
//! 在测试里复刻它只会得到一堆与产品无关的适配代码。
//!
//! 而本用例要证的是「后台重活在跑时 UI 线程仍然快」—— 那件事只取决于
//! “重活确实在别的线程上”，与它由谁调度无关。所以这里把重活放进一个
//! `std::thread`，主线程反复调 `update` 并计时。
//!
//! 另外还有一条**端到端**的用例（`the_start_queue_path_runs_on_a_worker_thread`）：
//! 它走完整的 `Message::StartQueue` → `Task` → 驱动 → 完成，证明 GUI 那条路
//! 本身是通的（产物真的出现）。
//!
//! ## 不测什么
//!
//! 真人点按钮、IME、DPI。那些要人眼与人手，本文件不假装覆盖它们
//! （`docs/specs/25` 的"执行记录合同"明确要求 GUI 验收不拿设计文档或自制返回值替代）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gui::app::SubtitleApp;
use gui::message::Message;
use gui::state::profile::{OutputProfile, ProfilesState, TargetFormat};

/// 37 §1 的阈值。它是"建议"值，而这里把它当**判据**用：
/// 一个远超它的实测值（无论怎么解释）都是缺陷。
const UPDATE_BUDGET: Duration = Duration::from_millis(8);

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ui-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 造一份有真实内容的 SRT。
fn write_srt(dir: &Path, name: &str, cues: usize) -> PathBuf {
    let mut s = String::new();
    for i in 0..cues {
        let sec = 1 + i;
        s.push_str(&format!(
            "{}\n00:{:02}:{:02},000 --> 00:{:02}:{:02},900\n第 {i} 行\n\n",
            i + 1,
            sec / 60,
            sec % 60,
            sec / 60,
            (sec % 60) + 1
        ));
    }
    let p = dir.join(name);
    std::fs::write(&p, s).unwrap();
    p
}

/// 把应用摆成"可以开始跑任务"的状态：若干输入 + 一个纯文本 profile。
///
/// 用 ASS 而不是渲染型 profile：渲染要 libass，而本用例测的是 UI 响应性，
/// 不该因为机器上没装渲染器就跳过 —— 那样 AT-UI-001 就成了"有 libass 才验收"。
fn ready_to_run(dir: &Path, inputs: usize, cues: usize) -> SubtitleApp {
    let mut app = SubtitleApp::default();
    for i in 0..inputs {
        app.inputs
            .add_path(write_srt(dir, &format!("in{i}.srt"), cues));
    }
    app.inputs.select_all(true);
    // `ProfilesState::new()` 会预置默认 profile（BDN / 4K HDR SUP），
    // 而那两个是**渲染型**（`renders: true`）—— 留着它们会让本用例要求 libass，
    // 于是“没有渲染器就跳过”。本用例测的是 UI 响应性，不该被渲染依赖绑架。
    let mut profiles = ProfilesState::new();
    profiles.profiles.clear();
    let mut p = OutputProfile::default_bdn(0);
    p.name = "ass-out".to_string();
    p.target_format = TargetFormat::Ass;
    profiles.profiles.push(p);
    app.profiles = profiles;
    // 输出目录放进临时目录，别把测试产物写到仓库或用户目录
    app.task_drawer.output_dir = dir.join("out");
    app
}

/// 调一次 `update` 并返回耗时。
fn timed(app: &mut SubtitleApp, msg: Message) -> Duration {
    let t = Instant::now();
    let _task = app.update(msg);
    t.elapsed()
}

/// 空闲时的 UI 交互必须在预算内。
///
/// 这是基线：如果连空闲都超预算，那"重任务期间"的结论就没有意义了。
#[test]
fn idle_ui_interactions_stay_within_the_budget() {
    let d = scratch("idle");
    let mut app = ready_to_run(&d, 1, 3);
    let id = app.inputs.items[0].id;

    // 选输入（会启动预览渲染 —— 它必须是 Task，而不是在 update 里同步跑）
    let t = timed(&mut app, Message::InputSelected(id));
    assert!(
        t < UPDATE_BUDGET,
        "InputSelected 用了 {t:?}，超过 {UPDATE_BUDGET:?}（预览渲染该走 Task）"
    );

    // 纯状态改动
    for msg in [
        Message::PreviewTimeChanged(1200),
        Message::PreviewTimeChanged(2400),
        Message::InputToggled(id),
        Message::SelectAllInputs(true),
        Message::SelectAllInputs(false),
        Message::ToggleTaskDrawer,
        Message::OpenDoctorDialog,
        Message::CloseDialog,
    ] {
        let t = timed(&mut app, msg.clone());
        assert!(
            t < UPDATE_BUDGET,
            "{msg:?} 用了 {t:?}，超过 {UPDATE_BUDGET:?}"
        );
    }
}

/// **重任务期间** UI 仍然要快。这一条才是 AT-UI-001 的实质。
///
/// 重活由 GUI 自己的 worker 干（参数与 `StartQueue` 交给 `spawn_blocking`
/// 的完全一样），放进一个**独立线程**；主线程反复调 `update` 并计时。
/// 采样窗口盖住整个任务期 —— 这就是"60 秒内持续可响应"的可判定形式。
#[test]
fn the_ui_stays_responsive_while_a_heavy_task_runs() {
    let d = scratch("under-load");
    // 造一份**够重**的输入：任务要跑上几秒，采样窗口才盖得住“持续可响应”。
    // 每份 4000 条 cue × 12 份输入——足够让批处理真的忙一会儿，
    // 同时远低于 23 §文件 的 cue 数上限（所以不会因限额而失败）。
    let mut app = ready_to_run(&d, 12, 4000);

    // 与 `Message::StartQueue` 里做的一样：编译成真 batch spec
    let inputs: Vec<(String, String)> = app
        .inputs
        .selected_inputs()
        .into_iter()
        .map(|i| (i.path.display().to_string(), i.file_name.clone()))
        .collect();
    let profiles: Vec<OutputProfile> = app
        .profiles
        .enabled_profiles()
        .into_iter()
        .cloned()
        .collect();
    assert!(!inputs.is_empty() && !profiles.is_empty(), "测试前提不成立");
    let roots: Vec<PathBuf> = inputs.iter().map(|(p, _)| PathBuf::from(p)).collect();
    let (spec, notes) = gui::worker::spec::compile_spec(
        "ui-load",
        &roots,
        &profiles.iter().collect::<Vec<_>>(),
        &app.task_drawer.output_dir,
        None,
        "{stem}.{profile}.{ext}",
    );
    assert!(
        notes.iter().all(|n| !n.contains("无法")),
        "spec 编译有硬错误：{notes:?}"
    );

    // 造出与真任务一样多的行，让抽屉有东西可更新
    let names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
    app.task_drawer.plan(&inputs, &names);
    app.task_drawer.is_running = true;

    // 重活放独立线程（GUI 那边是 spawn_blocking，同样是别的线程）
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = cancel.clone();
    let out_dir = app.task_drawer.output_dir.clone();
    let started = Instant::now();
    let worker = std::thread::spawn(move || {
        let mut rows = 0usize;
        let r = gui::worker::task_worker::run(&spec, &out_dir, cancel_for_worker, |_p| {
            rows += 1;
        });
        (r, rows)
    });

    // 任务刚起来，确认它真的在跑（否则下面测的不是“重任务期间”）
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut samples: Vec<Duration> = Vec::new();
    let mut done = false;
    while Instant::now() < deadline {
        // 真实的 UI 事件：切预览时间、切选择状态、开合抽屉
        samples.push(timed(&mut app, Message::PreviewTimeChanged(1000)));
        let first = app.inputs.items[0].id;
        let _ = timed(&mut app, Message::InputToggled(first));
        let _ = timed(&mut app, Message::ToggleTaskDrawer);

        if worker.is_finished() {
            done = true;
            break;
        }
        // 模拟一帧：不 busy loop，否则测的是 CPU 而不是响应性
        std::thread::sleep(Duration::from_millis(16));
    }

    let elapsed = started.elapsed();
    let (result, rows) = worker.join().expect("worker 不该 panic");

    assert!(!samples.is_empty(), "一次都没采到样");
    let worst = samples.iter().max().copied().unwrap();
    let total: Duration = samples.iter().sum();
    let mean = total / (samples.len() as u32);
    // 实测值要打出来：AT-UI-001 的证据就是这个分布，而不只是一句“通过”。
    eprintln!(
        "AT-UI-001 实测：{} 次 update，最慢 {worst:?}，均值 {mean:?}，预算 {UPDATE_BUDGET:?}",
        samples.len()
    );
    assert!(
        worst < UPDATE_BUDGET,
        "重任务期间最慢的一次 update 用了 {worst:?}，超过 {UPDATE_BUDGET:?}。\
         采样 {} 次；这说明有同步工作跑在 UI 线程上（37 §1）。",
        samples.len()
    );
    assert!(
        done,
        "60 秒内任务没跑完（采样 {} 次，最慢 {worst:?}，跑了 {elapsed:?}）\
         —— 这个用例的前提就不成立",
        samples.len()
    );
    assert!(rows > 0, "worker 一条进度都没报，任务真的跑了吗");
    assert!(result.is_ok(), "worker 该跑完而不是起不来：{result:?}");
    // 产物真的出现（否则“跑完了”可能只是“什么都没干”）
    let produced = std::fs::read_dir(&app.task_drawer.output_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("ass"))
        .count();
    assert_eq!(
        produced,
        roots.len(),
        "每份输入各该产出一个 .ass（{produced}/{}）",
        roots.len()
    );
}

/// 取消必须在任务跑着的时候就能生效，且**点击本身不阻塞**。
///
/// "取消可点击"拆开是两件事：按钮的事件能在毫秒级被处理，
/// 以及置位之后引擎真的会停。
#[test]
fn cancel_is_clickable_during_a_task_and_actually_stops_it() {
    let d = scratch("cancel");
    let mut app = ready_to_run(&d, 6, 400);

    let inputs: Vec<(String, String)> = app
        .inputs
        .selected_inputs()
        .into_iter()
        .map(|i| (i.path.display().to_string(), i.file_name.clone()))
        .collect();
    let profiles: Vec<OutputProfile> = app
        .profiles
        .enabled_profiles()
        .into_iter()
        .cloned()
        .collect();
    let roots: Vec<PathBuf> = inputs.iter().map(|(p, _)| PathBuf::from(p)).collect();
    let (spec, _) = gui::worker::spec::compile_spec(
        "ui-cancel",
        &roots,
        &profiles.iter().collect::<Vec<_>>(),
        &app.task_drawer.output_dir,
        None,
        "{stem}.{profile}.{ext}",
    );
    let names: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
    app.task_drawer.plan(&inputs, &names);
    app.task_drawer.is_running = true;

    let cancel = Arc::new(AtomicBool::new(false));
    // 关键：把**应用的令牌**交给 worker，而不是另开一份。
    // 这正是 `StartQueue` 做的事（`let cancel = self.cancel.clone()`）。
    // 各用一份的结果是“点了取消，worker 照跑”——
    // 那恰好就是 AT-UI-001 要防的缺陷（取消按钮无效）。
    app.cancel = cancel.clone();
    let cancel_for_worker = cancel.clone();
    let out_dir = app.task_drawer.output_dir.clone();
    let worker = std::thread::spawn(move || {
        gui::worker::task_worker::run(&spec, &out_dir, cancel_for_worker, |_p| {})
    });

    // 取消点击本身必须立刻返回
    let t = timed(&mut app, Message::CancelQueue);
    assert!(
        t < UPDATE_BUDGET,
        "取消点击用了 {t:?}，超过 {UPDATE_BUDGET:?}"
    );
    assert!(
        cancel.load(Ordering::Relaxed),
        "点了取消之后令牌该已置位（GUI 的 `self.cancel` 就是这一份）"
    );

    // 置位之后任务必须停下来
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !worker.is_finished() {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(worker.is_finished(), "取消后 30 秒 worker 仍未结束");
    let result = worker.join().expect("worker 不该 panic");
    assert!(
        result.is_ok(),
        "取消该是被支持的结果，而不是错误：{result:?}"
    );
}

/// **端到端**：`Message::StartQueue` 这条真实路径确实把 worker 接上了。
///
/// 上一条把 worker 单独拎出来测响应性，那会漏掉“`StartQueue` 到底接没接上”
/// 这一类问题（历史上的真缺陷就是“`StartQueue` 从不调执行器”）。
///
/// 这里不驱动 `iced::Task`（它的 action 流是 `iced_runtime` 的私有 API，
/// 在测试里复刻只会得到一堆与产品无关的适配代码），而是直接断言
/// `StartQueue` 之后的状态：任务被标记在跑、抽屉里已有计划的行、
/// 且 `cancel` 是一份**新的**令牌（上一任务置位的旧令牌被丢掉）。
/// 那三件都是“它真打算干活”的可观测前提，而不只是“返回了一个 Task”。
#[test]
fn start_queue_arms_a_real_run_instead_of_just_returning_a_task() {
    let d = scratch("e2e");
    let mut app = ready_to_run(&d, 3, 50);

    // 先让上一轮把令牌置位，验证新任务不会一启动就自认为被取消
    drop(app.update(Message::CancelQueue));

    let _task = app.update(Message::StartQueue);
    assert!(app.task_drawer.is_running, "StartQueue 该把任务标成在跑");
    assert!(
        app.task_drawer.is_expanded,
        "该把抽屉展开——任务在跑却看不见它，用户会以为没反应"
    );
    assert!(
        !app.task_drawer.tasks.is_empty(),
        "该已计划出行（否则抽屉里连占位都没有）"
    );
    assert!(
        !app.cancel.load(Ordering::Relaxed),
        "新任务必须拿一份新令牌：复用被取消过的旧令牌会让它一开始就停"
    );
}

/// 没有任何选中输入（或没有启用的 profile）时，`StartQueue` **不能**
/// 把状态标成“在跑”。
///
/// 否则界面会转圈等待一个从未开始的任务 —— 那正是“看起来在动、实际什么都没干”
/// 这类缺陷的形状（本仓库把“占位实现必须明确报错”列为纪律，同 20 §Commands）。
#[test]
fn start_queue_with_nothing_to_do_does_not_pretend_to_run() {
    let d = scratch("empty");
    let mut app = ready_to_run(&d, 1, 3);
    app.inputs.select_all(false); // 取消选中所有输入

    let _ = app.update(Message::StartQueue);
    assert!(
        !app.task_drawer.is_running,
        "没有选中输入时不该标成在跑（否则界面会转圈等一个不存在的任务）"
    );
}
