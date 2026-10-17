//! 22 §6 的崩溃现场记录：host 死之前把自己知道的东西写下来。
//!
//! ## 规格要什么
//!
//! 22 §6：「单独 stdout/stderr capture；超时、exit code、SEH crash/minidump 分离记录。」
//!
//! 这三样里，`stdout/stderr` 捕获、超时与 exit code 已经实现了（见 `client.rs` 的
//! `HostFailure` 变体）。缺的是**崩溃现场的结构化记录** ——
//! 现在主进程拿到的只有一句「退出码 3」，而查这种问题需要知道的是
//! 「在做什么的时候死的、当时加载的是哪个 provider」。
//!
//! ## 为什么不是真 minidump
//!
//! 完整的 minidump 要 `DbgHelp` 的 `MiniDumpWriteDump`，而它的用法有两条硬前提：
//!
//! 1. 必须在一个**异常处理器**里调用，并且那个处理器本身要满足一堆"异步信号安全"的
//!    约束（不能分配内存、不能拿锁、不能调大部分 Win32 API）。写一个**正确**的
//!    异常处理器比自己写一个不安全的更危险：它会掩盖真正的崩溃原因，
//!    或者在最需要它的时候二次崩溃。
//! 2. dump 里含进程内存镜像。这个产品处理的是**用户的字幕与内嵌字体**，
//!    而诊断包默认要能直接外发（22 §5）。一个含内存镜像的 dump 与那条承诺直接冲突。
//!
//! 所以这里做的是**崩溃现场记录**：host 在自己能控制的边界上（每个请求前后）
//! 把「我在做什么、用的是什么」写成一个 json 文件，崩溃后主进程把它收走。
//! 它不包含内存内容，但能回答"哪一步死的"—— 而那正是绝大多数崩溃调查的第一个问题。
//!
//! 真 minidump 仍然是有价值的东西，所以本模块**明确**把这件事记成未做（见
//! `docs/specs/22` 的实现状态），而不是用一个残缺的异常处理器假装做到了。
//!
//! ## 写盘纪律
//!
//! 记录文件必须**先写临时文件再改名**：半截的 json 会让主进程在解析时失败，
//! 而那时用户已经丢了崩溃信息。与其它产物同一条规矩（15 §6）。
//!
//! 而且**写不进去不能影响渲染**：崩溃记录的用途是"事后来看"，
//! 让一个诊断文件的写失败把正常渲染搞坏，是拿主功能去换附属功能。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 记录文件相对于工作目录的名字。主进程按同一个规则去找它。
pub const FILE_NAME: &str = "host-crash.json";

/// 请求种类的粗分类。刻意不用 `Request` 枚举：崩溃记录要**稳定**，
/// 协议加字段不该让旧记录读不出来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// 启动、读握手、加载 provider
    Startup,
    /// 交给脚本
    LoadScript,
    /// 出一帧
    Render,
    /// 收尾
    Shutdown,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Startup => "startup",
            Phase::LoadScript => "load-script",
            Phase::Render => "render",
            Phase::Shutdown => "shutdown",
        }
    }
}

/// 崩溃现场。能被序列化成一个 json 文件，供主进程收集。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CrashNote {
    /// 本文件格式的版本。加字段不改它；改语义才加。
    pub schema: u32,
    /// 死在哪个阶段（22 §6 要的"分离记录"：至少要知道是哪一类动作）。
    pub phase: String,
    /// 该阶段内的序号（第几帧）。`None` = 这个阶段没有序号概念。
    pub step: Option<u64>,
    /// 当时的时间戳（ms，从 host 启动算起）。
    ///
    /// 用**单调时钟的差值**而不是墙上时间：崩溃记录要回答"跑了多久之后死的"，
    /// 而墙上时间在时钟被调整时会撒谎。
    pub uptime_ms: u64,
    /// provider 的身份。这是"哪个 DLL 死的"这个问题的答案。
    pub provider_path: Option<String>,
    pub provider_sha256: Option<String>,
    pub provider_arch: Option<String>,
    /// 画布（有些崩溃只在特定尺寸下出现）。
    ///
    /// 用 `u32` 而不是 `u16`：`Canvas` 的宽高就是 `u32`（09 §），
    /// 在记录里窄化一次只会多一处需要解释的转换。
    pub storage: Option<(u32, u32)>,
    pub output: Option<(u32, u32)>,
    /// 注入的字体数（字形缓存相关的崩溃与它有关）。
    pub injected_fonts: usize,
    /// 最后一个成功的动作，给调查一个起点。
    pub last_ok: Option<String>,
    /// 为什么写这份记录（人工注入 / Rust panic / 未知）。
    pub reason: String,
}

impl CrashNote {
    pub fn new(phase: Phase, step: Option<u64>, reason: &str) -> CrashNote {
        CrashNote {
            schema: SCHEMA,
            phase: phase.label().to_string(),
            step,
            uptime_ms: 0,
            provider_path: None,
            provider_sha256: None,
            provider_arch: None,
            storage: None,
            output: None,
            injected_fonts: 0,
            last_ok: None,
            reason: reason.to_string(),
        }
    }
}

/// 记录格式版本。
pub const SCHEMA: u32 = 1;

/// host 侧的现场追踪器。
///
/// 它在每个阶段边界被更新（**不**在每个字节上），所以开销是常数级的。
/// `std::sync::Mutex` 而不是原子：这里没有争用（host 是单线程的渲染循环），
/// 而互斥锁让"读-改-写"整块状态不会被拆开。
pub struct Tracker {
    inner: Mutex<TrackerState>,
    dir: Option<PathBuf>,
}

struct TrackerState {
    started: std::time::Instant,
    phase: Phase,
    step: Option<u64>,
    last_ok: Option<String>,
    provider_path: Option<String>,
    provider_sha256: Option<String>,
    provider_arch: Option<String>,
    storage: Option<(u32, u32)>,
    output: Option<(u32, u32)>,
    injected_fonts: usize,
}

impl Tracker {
    /// `dir` 为 `None` 时追踪器仍然工作（内存里），只是不落盘。
    /// 这是有意的：拿不到目录不该让 host 起不来。
    pub fn new(dir: Option<PathBuf>) -> Tracker {
        Tracker {
            inner: Mutex::new(TrackerState {
                started: std::time::Instant::now(),
                phase: Phase::Startup,
                step: None,
                last_ok: None,
                provider_path: None,
                provider_sha256: None,
                provider_arch: None,
                storage: None,
                output: None,
                injected_fonts: 0,
            }),
            dir,
        }
    }

    /// 从 `STK_HOST_CRASH_DIR` 读目录；没设就不落盘。
    pub fn from_env() -> Tracker {
        let dir = std::env::var(crate::protocol::CRASH_DIR_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        Tracker::new(dir)
    }

    pub fn record_provider(&self, path: &Path, sha256: &str, arch: &str) {
        let mut s = self.lock();
        s.provider_path = Some(path.display().to_string());
        s.provider_sha256 = Some(sha256.to_string());
        s.provider_arch = Some(arch.to_string());
    }

    pub fn record_canvas(&self, storage: (u32, u32), output: (u32, u32)) {
        let mut s = self.lock();
        s.storage = Some(storage);
        s.output = Some(output);
    }

    pub fn set_injected_fonts(&self, n: usize) {
        self.lock().injected_fonts = n;
    }

    /// 进入一个阶段。`step` 是阶段内序号（例如第几帧）。
    pub fn enter(&self, phase: Phase, step: Option<u64>) {
        let mut s = self.lock();
        s.phase = phase;
        s.step = step;
    }

    /// 记下"这一步成了"。它的用途是给调查一个起点：
    /// 崩溃发生在上一个成功动作与当前阶段之间。
    pub fn note_ok(&self, what: &str) {
        self.lock().last_ok = Some(what.to_string());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TrackerState> {
        // 中毒不该让崩溃路径变成新错误：这里的状态是一组快照值，
        // 拿到半更新的快照也比在崩溃前 panic 好。
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 当前现场（不写盘）。测试与报告都用它。
    pub fn snapshot(&self, reason: &str) -> CrashNote {
        let s = self.lock();
        CrashNote {
            schema: SCHEMA,
            phase: s.phase.label().to_string(),
            step: s.step,
            uptime_ms: s.started.elapsed().as_millis() as u64,
            provider_path: s.provider_path.clone(),
            provider_sha256: s.provider_sha256.clone(),
            provider_arch: s.provider_arch.clone(),
            storage: s.storage,
            output: s.output,
            injected_fonts: s.injected_fonts,
            last_ok: s.last_ok.clone(),
            reason: reason.to_string(),
        }
    }

    /// 把现场写到磁盘。返回写出的路径。
    ///
    /// **先写临时文件再改名**：半截 json 会让主进程解析失败。
    /// 写失败返回 `Err`，而调用方**不应该**因此改变渲染行为（见模块注释）。
    pub fn write(&self, reason: &str) -> std::io::Result<Option<PathBuf>> {
        let Some(dir) = &self.dir else {
            return Ok(None);
        };
        let note = self.snapshot(reason);
        std::fs::create_dir_all(dir)?;
        let target = dir.join(FILE_NAME);
        let tmp = dir.join(format!("{FILE_NAME}.tmp"));
        let text = serde_json::to_string_pretty(&note)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, &target)?;
        Ok(Some(target))
    }
}

/// 读一份崩溃记录（主进程侧用）。
///
/// 读失败**不报错**而是返回 `None`：没有记录是常态（大多数运行不会崩），
/// 而"没有记录"与"记录坏了"在报告里都是"没有可用的现场信息"。
/// 两者区分开来只会多一个用户看不懂的状态。
pub fn read(dir: &Path) -> Option<CrashNote> {
    let text = std::fs::read_to_string(dir.join(FILE_NAME)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 把一份记录收成一行人能读的摘要，供错误卡的 details 用。
pub fn summarize(note: &CrashNote) -> String {
    let mut parts = vec![format!("阶段={}", note.phase)];
    if let Some(step) = note.step {
        parts.push(format!("序号={step}"));
    }
    parts.push(format!("已运行={}ms", note.uptime_ms));
    if let Some(p) = &note.provider_arch {
        parts.push(format!("provider={p}"));
    }
    if let Some(s) = &note.provider_sha256 {
        let short: String = s.chars().take(12).collect();
        parts.push(format!("provider sha256={short}…"));
    }
    if let Some(l) = &note.last_ok {
        parts.push(format!("上一个成功动作={l}"));
    }
    parts.push(format!("原因={}", note.reason));
    parts.join("；")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("stk-crash-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 记录必须能写出来再读回来，字段一个不少。
    #[test]
    fn a_note_survives_a_write_read_roundtrip() {
        let d = tmp("rt");
        let t = Tracker::new(Some(d.clone()));
        t.record_provider(Path::new("C:/x/ass.dll"), "abc123", "x64");
        t.record_canvas((1920, 1080), (1920, 1080));
        t.set_injected_fonts(2);
        t.enter(Phase::Render, Some(417));
        t.note_ok("load_script");

        let path = t.write("人工注入 abort").unwrap().expect("该写出文件");
        assert!(path.is_file(), "{path:?}");
        assert_eq!(path.file_name().unwrap(), FILE_NAME);

        let back = read(&d).expect("该能读回来");
        assert_eq!(back.schema, SCHEMA);
        assert_eq!(back.phase, "render");
        assert_eq!(back.step, Some(417));
        assert_eq!(back.provider_path.as_deref(), Some("C:/x/ass.dll"));
        assert_eq!(back.provider_sha256.as_deref(), Some("abc123"));
        assert_eq!(back.provider_arch.as_deref(), Some("x64"));
        assert_eq!(back.storage, Some((1920, 1080)));
        assert_eq!(back.injected_fonts, 2);
        assert_eq!(back.last_ok.as_deref(), Some("load_script"));
        assert_eq!(back.reason, "人工注入 abort");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 后写覆盖先写 —— 要的是**最后**那个现场。
    /// 保留历史会让主进程读到过期的现场，而它只会按名字找一份。
    #[test]
    fn a_second_write_replaces_the_first() {
        let d = tmp("replace");
        let t = Tracker::new(Some(d.clone()));
        t.enter(Phase::Startup, None);
        t.write("第一次").unwrap();
        t.enter(Phase::Render, Some(9));
        t.write("第二次").unwrap();

        let back = read(&d).unwrap();
        assert_eq!(back.phase, "render");
        assert_eq!(back.step, Some(9));
        assert_eq!(back.reason, "第二次");
        // 目录里只该有一个记录文件（+ 无残留 tmp）
        let names: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec![FILE_NAME.to_string()], "{names:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 没有目录时**不落盘也不报错**：拿不到目录是常态（例如没设环境变量），
    /// 它不该让 host 起不来。
    #[test]
    fn without_a_directory_nothing_is_written_and_nothing_fails() {
        let t = Tracker::new(None);
        t.enter(Phase::Render, Some(1));
        assert_eq!(t.write("x").unwrap(), None);
        // 内存里的现场仍然可用（报告可以只用它）
        let snap = t.snapshot("x");
        assert_eq!(snap.phase, "render");
        assert_eq!(snap.step, Some(1));
    }

    /// 目录不可写时返回 `Err`，而调用方**不该**因此改变渲染行为。
    /// 这里只断言那个 `Err` 确实出现（调用点的纪律在 main.rs 里用 `let _ =` 表达）。
    #[test]
    fn an_unwritable_directory_reports_an_error_instead_of_panicking() {
        let d = tmp("unwritable");
        // 把"目录"这个名字占成一个文件
        let blocked = d.join("blocked");
        std::fs::write(&blocked, "x").unwrap();
        let t = Tracker::new(Some(blocked.join("sub")));
        assert!(t.write("x").is_err(), "写不进去该返回 Err");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 读不存在的记录、读坏记录都返回 `None`，不报错。
    #[test]
    fn a_missing_or_corrupt_note_reads_as_none() {
        let d = tmp("corrupt");
        assert!(read(&d).is_none(), "没有文件");
        std::fs::write(d.join(FILE_NAME), "{ not json").unwrap();
        assert!(read(&d).is_none(), "坏 json");
        std::fs::write(d.join(FILE_NAME), "{}").unwrap();
        // `{}` 缺字段，反序列化该失败
        assert!(read(&d).is_none(), "缺字段");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 摘要要带上调查需要的要素，而不是一句"崩了"。
    #[test]
    fn the_summary_names_the_phase_and_the_provider() {
        let t = Tracker::new(None);
        t.record_provider(Path::new("C:/x/ass.dll"), &"a".repeat(64), "x64");
        t.enter(Phase::Render, Some(12));
        t.note_ok("load_script");
        let s = summarize(&t.snapshot("abort"));

        assert!(s.contains("阶段=render"), "{s}");
        assert!(s.contains("序号=12"), "{s}");
        assert!(s.contains("provider=x64"), "{s}");
        assert!(s.contains("aaaaaaaaaaaa…"), "sha 该截断显示：{s}");
        assert!(s.contains("load_script"), "{s}");
        assert!(s.contains("原因=abort"), "{s}");
        // 不该把完整哈希倒出来（details 是给人看的）
        assert!(!s.contains(&"a".repeat(64)), "{s}");
    }

    /// 阶段标签是稳定字符串，不是 `Debug` 输出。
    /// 它们进 json 文件，改名就等于让旧记录读不出来。
    #[test]
    fn phase_labels_are_stable_strings() {
        assert_eq!(Phase::Startup.label(), "startup");
        assert_eq!(Phase::LoadScript.label(), "load-script");
        assert_eq!(Phase::Render.label(), "render");
        assert_eq!(Phase::Shutdown.label(), "shutdown");
        // 序列化后就是那个标签
        let note = CrashNote::new(Phase::LoadScript, None, "x");
        let text = serde_json::to_string(&note).unwrap();
        assert!(text.contains("\"load-script\""), "{text}");
    }

    /// 没有 `step` 的阶段（启动/收尾）也要能表达。
    #[test]
    fn a_phase_without_a_step_is_representable() {
        let t = Tracker::new(None);
        t.enter(Phase::Startup, None);
        let n = t.snapshot("x");
        assert_eq!(n.step, None);
        let s = summarize(&n);
        assert!(!s.contains("序号="), "没有序号时不该编一个：{s}");
    }
}
