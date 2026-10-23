//! 并发执行器（15 §3）、多输出共享渲染（15 §4）与崩溃预算接线（38 §3）。
//!
//! 设计取舍，写明白免得后来的人误以为这里"缺了更聪明的实现"：
//!
//! - **分组而不是共享可变缓存**。多输出共享渲染如果做成"一个 Mutex<HashMap> 现取现算"，
//!   就得处理"两个线程同时 miss 同一个 key 到底谁算"（要么重复算，要么 in-flight 标记 +
//!   条件变量）。这里改成**先按缓存 key 分组，每组各算一次**：不同组并行、组内复用同一份结果，
//!   于是"同一帧只渲染一次"是结构性事实，可以直接数出来（AT-MULTI-002），
//!   不依赖运行时锁的正确性。
//! - **每阶段一个计数信号量**（15 §3：render 按 provider 限额、disk bounded、外部进程单独限），
//!   由调用方在真正做那件事的时候 `acquire`。执行器不猜工作类型，因此不会把 CPU 活计算成渲染。
//! - **panic 当作崩溃而不是忽略**。std 线程里的 panic 会被 join 悄悄吞掉，那样 38 §3 的
//!   重启预算形同虚设；这里显式 `catch_unwind` 并当场喂给预算。
//! - **报告顺序与线程调度无关**：结果按 profile 下标落位，输出按名字排序。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use core_model::error::{Domain, Result, StkError};

use crate::job::{
    CancelToken, CrashAction, CrashBudget, Job, JobOutcome, OutputProfile, ProfileOutcome,
};

/// 15 §3 的工作类别，每个类别一份独立的并发额度。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Cpu,
    Render,
    Quant,
    Author,
    Disk,
    External,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Cpu => "cpu",
            Stage::Render => "render",
            Stage::Quant => "quant",
            Stage::Author => "author",
            Stage::Disk => "disk",
            Stage::External => "external",
        }
    }
}

struct SemState {
    free: usize,
    /// 观察到"满额时排队"的次数：限额是否真生效，靠它证明而不是靠相信
    waits: usize,
    /// 同时在运行的数量峰值
    peak: usize,
    live: usize,
}

struct SemInner {
    state: Mutex<SemState>,
    cv: Condvar,
}

struct Sem(Arc<SemInner>);

/// 持有期间占一个额度；Drop 时归还。**不能**把 MutexGuard 带出去：
/// 那样锁在整个工作期间都被占着，第二个线程会卡在 lock() 上而不是条件变量上，
/// 限额看起来"生效"（peak 一直是 1）其实是串行死锁，排队计数也永远是 0。
/// RAII 额度令牌：字段私有，调用方只能持有/丢弃它。
pub struct Permit(Option<Arc<SemInner>>);

impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(inner) = self.0.take() {
            let mut g = inner.state.lock().unwrap_or_else(|e| e.into_inner());
            g.live -= 1;
            g.free += 1;
            inner.cv.notify_one();
        }
    }
}

impl Sem {
    fn new(free: usize) -> Sem {
        Sem(Arc::new(SemInner {
            state: Mutex::new(SemState {
                free,
                waits: 0,
                peak: 0,
                live: 0,
            }),
            cv: Condvar::new(),
        }))
    }

    fn acquire(&self) -> Permit {
        let inner = self.0.clone();
        let mut g = inner.state.lock().unwrap_or_else(|e| e.into_inner());
        while g.free == 0 {
            g.waits += 1;
            let woke = inner
                .cv
                .wait_timeout(g, std::time::Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner());
            g = woke.0;
        }
        g.free -= 1;
        g.live += 1;
        g.peak = g.peak.max(g.live);
        drop(g);
        Permit(Some(inner))
    }

    fn stats(&self) -> (usize, usize) {
        let g = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        (g.waits, g.peak)
    }
}

/// 各类别的并发额度（15 §3）。克隆它很便宜：额度是共享的同一本账。
#[derive(Clone)]
pub struct Pools(Arc<InnerPools>);

impl core::fmt::Debug for Pools {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 诊断里要能看出"现在有几个槽空着、峰值到过多少"（22 §诊断），
        // 只打印一个地址等于没打印
        let mut s = f.debug_struct("Pools");
        for stage in [
            Stage::Cpu,
            Stage::Render,
            Stage::Quant,
            Stage::Author,
            Stage::Disk,
            Stage::External,
        ] {
            let (waits, peak) = self.stats_of(stage);
            s.field(stage.label(), &(waits, peak));
        }
        s.finish()
    }
}

struct InnerPools {
    cpu: Sem,
    render: Sem,
    quant: Sem,
    author: Sem,
    disk: Sem,
    external: Sem,
}

impl Pools {
    /// `quant`/`author` 是 CPU 密集型，复用 cpu 额度：各给一份计数会把"并发 4"变成
    /// 实际 12 路，把机器压成抖动。
    pub fn new(cpu: usize, render: usize, disk: usize, external: usize) -> Result<Pools> {
        for (name, v) in [
            ("cpu", cpu),
            ("render", render),
            ("disk", disk),
            ("external", external),
        ] {
            if v == 0 {
                return Err(StkError::simple(
                    Domain::Input,
                    87,
                    &format!("{name} 并发额度不能为 0"),
                )
                .with_detail("0 意味着这一阶段永远无法开始，任务是卡死而不是排队"));
            }
        }
        Ok(Pools(Arc::new(InnerPools {
            cpu: Sem::new(cpu),
            quant: Sem::new(cpu),
            author: Sem::new(cpu),
            render: Sem::new(render),
            disk: Sem::new(disk),
            external: Sem::new(external),
        })))
    }

    /// 默认档位：渲染额度 1（libass 不是指望它高并发的对象，而且崩溃必须能归因到
    /// 具体 provider），磁盘 2，外部进程 1。
    pub fn default_desktop(cpu: usize) -> Result<Pools> {
        Pools::new(cpu.max(1), 1, 2, 1)
    }

    pub fn acquire(&self, stage: Stage) -> Permit {
        let p = &self.0;
        match stage {
            Stage::Cpu => p.cpu.acquire(),
            Stage::Render => p.render.acquire(),
            Stage::Quant => p.quant.acquire(),
            Stage::Author => p.author.acquire(),
            Stage::Disk => p.disk.acquire(),
            Stage::External => p.external.acquire(),
        }
    }

    fn stats_of(&self, stage: Stage) -> (usize, usize) {
        let p = &self.0;
        match stage {
            Stage::Cpu => p.cpu.stats(),
            Stage::Render => p.render.stats(),
            Stage::Quant => p.quant.stats(),
            Stage::Author => p.author.stats(),
            Stage::Disk => p.disk.stats(),
            Stage::External => p.external.stats(),
        }
    }

    /// 某个类别是否曾经因为额度不足而排队。
    pub fn had_contention(&self, stage: Stage) -> bool {
        self.stats_of(stage).0 > 0
    }

    /// 同时运行的渲染数峰值——用它证明"render 限额真的被压住了"。
    pub fn render_peak(&self) -> usize {
        self.stats_of(Stage::Render).1
    }
}

pub struct Executed {
    /// 按 profile 名字排序：报告与 golden 都不能依赖线程调度（15 §8）
    pub outcomes: Vec<(String, ProfileOutcome)>,
    pub job: JobOutcome,
    pub crashes: usize,
}

/// 并行跑完一个 Job 的所有 profile。
///
/// `f` 收到 profile 与并发额度，自己决定在哪一步 `acquire` 哪个 `Stage`；
/// 执行器负责调度、取消、崩溃归因与结果落位。崩溃预算用 `Mutex` 传进来：
/// 预算必须在崩溃那一刻被消费（38 §3"预算耗尽就停手"），事后统计等于允许
/// 预算用完之后继续撞同一个坑。
pub fn run_profiles<F>(
    job: &Job,
    pools: &Pools,
    cancel: &CancelToken,
    budget: &Mutex<CrashBudget>,
    f: &F,
) -> Executed
where
    F: Fn(&Pools, &OutputProfile) -> ProfileOutcome + Sync,
{
    run_profiles_observed(job, pools, cancel, budget, f, &|_, _| {})
}

/// 每项任务结束后立即通知观察者，包括已捕获的崩溃结果。
pub fn run_profiles_observed<F, O>(
    job: &Job,
    pools: &Pools,
    cancel: &CancelToken,
    budget: &Mutex<CrashBudget>,
    f: &F,
    observe: &O,
) -> Executed
where
    F: Fn(&Pools, &OutputProfile) -> ProfileOutcome + Sync,
    O: Fn(&OutputProfile, &ProfileOutcome) + Sync,
{
    let n = job.profiles.len();
    let slots: Mutex<Vec<Option<ProfileOutcome>>> = Mutex::new((0..n).map(|_| None).collect());
    let next = Arc::new(AtomicUsize::new(0));
    let crashes = Arc::new(AtomicUsize::new(0));
    let workers = job.wants_concurrency().clamp(1, 8);

    {
        let profiles = &job.profiles;
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let next = next.clone();
                let crashes = crashes.clone();
                let slots = &slots;
                scope.spawn(move || loop {
                    if cancel.cancelled() {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    if i >= profiles.len() {
                        return;
                    }
                    let profile = &profiles[i];
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        f(pools, profile)
                    }))
                    .unwrap_or_else(|_| {
                        crashes.fetch_add(1, Ordering::SeqCst);
                        let action = {
                            let mut g = budget.lock().unwrap_or_else(|e| e.into_inner());
                            g.on_crash()
                        };
                        if matches!(action, CrashAction::StopProfile) {
                            // 预算用完：立取消标志，剩下的 profile 不再开工
                            cancel.cancel();
                        }
                        crash_outcome(profile, i, profiles.len(), action)
                    });
                    observe(profile, &outcome);
                    slots.lock().unwrap_or_else(|e| e.into_inner())[i] = Some(outcome);
                });
            }
        });
    }

    let mut filled = slots.into_inner().unwrap_or_else(|e| e.into_inner());
    let mut labeled: Vec<(String, ProfileOutcome)> = Vec::with_capacity(n);
    for (i, slot) in filled.iter_mut().enumerate() {
        let name = job.profiles[i].name.clone();
        labeled.push((
            name,
            slot.take().unwrap_or_else(|| ProfileOutcome::Cancelled {
                stage: "not-started".to_string(),
            }),
        ));
    }
    labeled.sort_by(|a, b| a.0.cmp(&b.0));
    let any_success = labeled.iter().any(|(_, o)| o.is_success());
    let crashes_now = crashes.load(Ordering::SeqCst);
    let job_outcome = if crashes_now > 0 {
        // `JobOutcome::classify` 的规则是"有任何 Cancelled 就算任务被取消"（用户主动停手
        // 优先于统计）。这里不能直接套：崩溃预算耗尽时是我们自己立的取消标志，
        // 于是"剩下几个 profile 没开工"取决于线程调度，同一份输入会时而 Failed、
        // 时而 Cancelled —— soak 门阶实测 20 轮里红过 2 轮。
        // 崩溃驱动的停手必须按"有没有成功产出"定级；"取消"这个标签只留给用户主动停手。
        let ok = labeled.iter().filter(|(_, o)| o.is_success()).count();
        if ok == 0 {
            JobOutcome::Failed
        } else if ok < labeled.len() {
            JobOutcome::PartialSuccess
        } else {
            JobOutcome::Success
        }
    } else if cancel.cancelled() && !any_success {
        JobOutcome::Cancelled
    } else {
        JobOutcome::classify(&labeled.iter().map(|(_, o)| o.clone()).collect::<Vec<_>>())
    };
    Executed {
        outcomes: labeled,
        job: job_outcome,
        crashes: crashes_now,
    }
}

fn crash_outcome(
    profile: &OutputProfile,
    i: usize,
    total: usize,
    action: CrashAction,
) -> ProfileOutcome {
    let what = match action {
        CrashAction::RestartOnce => "崩溃预算内允许重启 host 重试一次（38 §3）",
        CrashAction::StopProfile => {
            "崩溃预算耗尽，只停该 output，任务其余部分不再重启动它（38 §3）"
        }
    };
    ProfileOutcome::Failed {
        stage: "crash".to_string(),
        message: format!(
            "profile {}（第 {}/{} 个）在工作线程里 panic：{what}",
            profile.name,
            i + 1,
            total
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::job::InputSet;
    use core_model::canvas::Canvas;
    use core_model::profile::FormatKind;
    use core_model::time::Fps;

    fn profile(name: &str, w: u32) -> OutputProfile {
        OutputProfile {
            name: name.to_string(),
            format: FormatKind::Ass,
            canvas: Canvas::new(w, w * 9 / 16).unwrap(),
            fps: Fps::PAL_25,
        }
    }

    fn job_of(profiles: Vec<OutputProfile>) -> Job {
        Job {
            id: "t".to_string(),
            input: InputSet {
                files: vec!["a.ass".to_string()],
                primary_sha256: "ab".repeat(32),
            },
            profiles,
        }
    }

    fn ok(prof: &OutputProfile) -> ProfileOutcome {
        ProfileOutcome::Verified {
            path: format!("{}.out", prof.name),
            sha256: "00".repeat(32),
        }
    }

    #[test]
    fn progress_arrives_before_all_profiles_finish() {
        let job = job_of(vec![profile("first", 1280), profile("second", 1280)]);
        let pools = Pools::new(2, 1, 1, 1).unwrap();
        let cancel = CancelToken::new();
        let budget = Mutex::new(CrashBudget::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let rx = Mutex::new(rx);
        let observed = AtomicUsize::new(0);
        let result = run_profiles_observed(
            &job,
            &pools,
            &cancel,
            &budget,
            &|_, p| {
                if p.name == "second" {
                    rx.lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .expect("前一项的进度必须在慢任务结束前到达");
                }
                ok(p)
            },
            &|p, _| {
                observed.fetch_add(1, Ordering::SeqCst);
                if p.name == "first" {
                    tx.send(()).unwrap();
                }
            },
        );
        assert_eq!(observed.load(Ordering::SeqCst), 2);
        assert!(result.outcomes.iter().all(|(_, o)| o.is_success()));
    }

    #[test]
    fn zero_permits_are_refused_because_they_hang_instead_of_queueing() {
        for (cpu, render, disk, external) in
            [(0, 1, 1, 1), (1, 0, 1, 1), (1, 1, 0, 1), (1, 1, 1, 0)]
        {
            assert_eq!(
                Pools::new(cpu, render, disk, external)
                    .unwrap_err()
                    .code_string(),
                "STK-INP-0087"
            );
        }
        Pools::new(2, 1, 2, 1).unwrap();
    }

    #[test]
    fn render_semaphore_holds_the_concurrency_down() {
        let pools = Pools::new(4, 1, 2, 1).unwrap();
        let live = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        std::thread::scope(|scope| {
            for _ in 0..6 {
                let pools = pools.clone();
                let live = live.clone();
                let peak = peak.clone();
                scope.spawn(move || {
                    let _p = pools.acquire(Stage::Render);
                    let now = live.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    live.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "render 额度是 1，峰值却超过就是假限额"
        );
        assert!(
            pools.had_contention(Stage::Render),
            "6 个任务挤 1 个额度必然排队"
        );
    }

    #[test]
    fn results_are_ordered_by_profile_name_not_by_completion_order() {
        let job = job_of(vec![
            profile("zeta", 1920),
            profile("alpha", 1280),
            profile("mid", 1920),
        ]);
        let pools = Pools::default_desktop(2).unwrap();
        let cancel = CancelToken::new();
        let budget = Mutex::new(CrashBudget::new(true));
        let out = run_profiles(&job, &pools, &cancel, &budget, &|_p, prof| ok(prof));
        let names: Vec<&str> = out.outcomes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
        assert_eq!(out.job, JobOutcome::Success);
        assert_eq!(out.crashes, 0);
    }

    #[test]
    fn a_panicking_profile_becomes_a_crash_not_a_silent_hang_or_fake_success() {
        let job = job_of(vec![profile("boom", 1920), profile("fine", 1920)]);
        let pools = Pools::default_desktop(1).unwrap();
        let cancel = CancelToken::new();
        let budget = Mutex::new(CrashBudget::new(true));
        let out = run_profiles(&job, &pools, &cancel, &budget, &|_p, prof| {
            if prof.name == "boom" {
                panic!("renderer 里炸了");
            }
            ok(prof)
        });
        assert_eq!(out.crashes, 1, "panic 必须被数到，否则 38 §3 的预算是摆设");
        assert_eq!(out.job, JobOutcome::PartialSuccess);
        assert_eq!(out.outcomes[0].0, "boom");
        match &out.outcomes[0].1 {
            ProfileOutcome::Failed { stage, message } => {
                assert_eq!(stage, "crash");
                assert!(message.contains("崩溃预算"), "{message}");
            }
            other => panic!("崩溃要落成 Failed：{other:?}"),
        }
        // 崩掉的那个绝不能伪装成成功，另一个必须真的产出
        assert!(out.outcomes[1].1.is_success());
    }

    #[test]
    fn exhausted_crash_budget_stops_the_rest_of_the_job() {
        // safe_to_restart=false：第一次崩溃就没有重试额度了。
        // 这里能保证的是"取消被立起来 + 没有任何 output 假装成功 + 停手被写进消息"，
        // **不是**"剩下的 profile 一定还没开始"：worker 数按 profile 数推导，
        // 三个 profile 三个 worker 时，取消落地前活可能早就派出去了。
        // 把这条写成"必然 Cancelled"是假断言（本机实测在 cargo test 下侥幸绿、nextest 下红）。
        let job = job_of(vec![
            profile("a", 1920),
            profile("b", 1920),
            profile("c", 1920),
        ]);
        let pools = Pools::default_desktop(1).unwrap();
        let cancel = CancelToken::new();
        let budget = Mutex::new(CrashBudget::new(false));
        let out = run_profiles(&job, &pools, &cancel, &budget, &|_p, _prof| {
            panic!("每个 profile 都炸");
        });
        assert!(out.crashes >= 1, "崩溃必须被计数：{}", out.crashes);
        assert!(cancel.cancelled(), "预算耗尽必须把取消标志立起来（38 §3）");
        assert!(
            !out.outcomes.iter().any(|(_, o)| o.is_success()),
            "预算用完之后还报成功就是假绿：{:?}",
            out.outcomes
        );
        assert!(
            out.outcomes.iter().any(|(_, o)| matches!(
                o,
                ProfileOutcome::Failed { stage, .. } if stage == "crash"
            )),
            "至少有一个落成崩溃：{:?}",
            out.outcomes
        );
        assert!(
            out.outcomes.iter().any(|(_, o)| {
                match o {
                    ProfileOutcome::Failed { message, .. } => message.contains("预算耗尽"),
                    _ => false,
                }
            }),
            "预算耗尽要在消息里看得见：{:?}",
            out.outcomes
        );
        assert_eq!(
            out.job,
            JobOutcome::Failed,
            "没有一个 output 成功，任务就不是部分成功"
        );
    }

    #[test]
    fn crash_driven_stop_classifies_the_same_way_every_time() {
        // 定级不能依赖线程调度。这条在测试里自己重复 20 轮：门阶的 concurrency-soak
        // 是外层保险，这里保证任何人单跑测试也躲不掉——第一版断言写成"剩下的 profile
        // 必然没开工"，而那是个不成立的假设，于是同一份代码在 cargo test 下绿、
        // nextest 下偶发红（ISSUES AB2）。
        for round in 0..20 {
            let job = job_of(vec![
                profile("a", 1920),
                profile("b", 1920),
                profile("c", 1920),
                profile("d", 1920),
            ]);
            let pools = Pools::default_desktop(2).unwrap();
            let cancel = CancelToken::new();
            let budget = Mutex::new(CrashBudget::new(false));
            let out = run_profiles(&job, &pools, &cancel, &budget, &|_p, _prof| {
                panic!("第 {round} 轮也炸");
            });
            assert!(out.crashes >= 1, "第 {round} 轮没数到崩溃");
            assert!(cancel.cancelled(), "第 {round} 轮预算耗尽却没停手");
            assert_eq!(
                out.job,
                JobOutcome::Failed,
                "第 {round} 轮定级漂了：{:?}",
                out.outcomes
            );
        }
    }

    #[test]
    fn cancelled_job_does_not_start_more_work() {
        let job = job_of(vec![
            profile("a", 1920),
            profile("b", 1920),
            profile("c", 1920),
        ]);
        let pools = Pools::default_desktop(1).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let ran = Arc::new(AtomicUsize::new(0));
        let budget = Mutex::new(CrashBudget::new(true));
        let ran2 = ran.clone();
        let out = run_profiles(&job, &pools, &cancel, &budget, &|_p, prof| {
            ran2.fetch_add(1, Ordering::SeqCst);
            ok(prof)
        });
        assert_eq!(ran.load(Ordering::SeqCst), 0, "取消后一个都不该跑");
        assert_eq!(out.job, JobOutcome::Cancelled);
        assert!(out
            .outcomes
            .iter()
            .all(|(_, o)| matches!(o, ProfileOutcome::Cancelled { .. })));
    }
}
