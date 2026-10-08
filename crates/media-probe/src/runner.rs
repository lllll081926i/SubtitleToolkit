//! ffprobe 进程适配（23 §外部进程：绝对路径、参数数组、超时、不拼 shell 字符串）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use core_model::error::{Domain, Result, StkError};

/// 固定参数序列（16 §2 需要的字段都在 `-show_streams -show_format` 输出里）。
pub fn default_args(target: &Path) -> Vec<String> {
    vec![
        "-v".to_string(),
        "quiet".to_string(),
        "-print_format".to_string(),
        "json".to_string(),
        "-show_format".to_string(),
        "-show_streams".to_string(),
        target.to_string_lossy().into_owned(),
    ]
}

/// 返回 argv（不含解释器拼接）。第一个参数是 `-v`，保证没有 shell 注入面。
pub fn build_command(exe: &Path, target: &Path) -> (PathBuf, Vec<String>) {
    (exe.to_path_buf(), default_args(target))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FfprobeIdentity {
    pub path: PathBuf,
    pub size: u64,
    /// 二进制内容的 SHA-256，进 JobSnapshot 用于 cache 失效（38 §6）。
    /// 以前是 fnv1a-64：那是个"能检测变化但不是密码学摘要"的东西，
    /// 而 21 §11 与 15 §4 要的是一份可以对外声明"这就是当时那个二进制"的记录（D11）。
    pub digest: String,
}

impl FfprobeIdentity {
    pub fn of(exe: &Path) -> Result<FfprobeIdentity> {
        let bytes = std::fs::read(exe)
            .map_err(|e| StkError::from(e).with_detail(exe.display().to_string()))?;
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(&bytes);
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        Ok(FfprobeIdentity {
            path: exe.to_path_buf(),
            size: bytes.len() as u64,
            digest: format!("sha256-{hex}"),
        })
    }
}

pub trait ProbeRunner {
    /// 返回 ffprobe 的 JSON 文本。
    fn probe(&self, target: &Path) -> Result<String>;
}

pub enum ProbeError {
    NotFound,
    Timeout,
    /// 超时，而且子进程树没被 Job Object 约束住（字段是附着失败的 Win32 错误码，0 = 本平台无此机制）
    TimeoutUncontained(u32),
    NonZero(i32, String),
    OutputTooLarge(&'static str, usize),
}

impl ProbeError {
    pub fn to_stk(self, exe: &Path) -> StkError {
        match self {
            ProbeError::NotFound => {
                StkError::simple(Domain::Dependency, 20, "未找到 ffprobe，视频辅助功能不可用")
                    .with_detail(exe.display().to_string())
                    .with_action("手动指定 ffprobe.exe 路径，或跳过视频探测继续手动填参")
            }
            ProbeError::Timeout => StkError::simple(Domain::Dependency, 21, "ffprobe 超时")
                .with_detail(exe.display().to_string()),
            ProbeError::TimeoutUncontained(win32) => StkError::simple(
                Domain::Dependency,
                23,
                "ffprobe 超时，且子进程树未能被 Job Object 约束",
            )
            .with_detail(format!(
                "{}；Job Object 附着失败（Win32 错误 {win32}）。只终止了直接子进程，\
                 它派生的进程可能仍在运行并占着管道",
                exe.display()
            ))
            .with_action(
                "本进程已在一个外层 Job 里（CI/测试包装器常见）。生产环境请避免在受 Job 包装的宿主内跑探测，或手动结束残留的 ffprobe 子进程",
            ),
            ProbeError::NonZero(code, stderr) => {
                StkError::simple(Domain::Dependency, 22, "ffprobe 非零退出")
                    .with_detail(format!("exit={code} {stderr}"))
            }
            ProbeError::OutputTooLarge(stream, limit) => StkError::simple(
                Domain::Dependency,
                24,
                "ffprobe 输出超过安全上限",
            )
            .with_detail(format!("{stream} 超过 {limit} 字节；{}", exe.display()))
            .with_action("核对媒体文件与 ffprobe 版本，避免处理异常或损坏输入"),
        }
    }
}

pub struct SystemProbe {
    pub exe: PathBuf,
    pub timeout: Duration,
}

/// 子进程树约束用共享的 `win_job`（23 §外部进程）：一份实现，media-probe 与 renderer-host 都用它
/// （一个语义只该有一处实现，ISSUES AD9）。
use win_job as job;

impl SystemProbe {
    pub fn new(exe: impl AsRef<Path>, timeout: Duration) -> SystemProbe {
        SystemProbe {
            exe: exe.as_ref().to_path_buf(),
            timeout,
        }
    }

    fn spawn_and_read(&self, target: &Path) -> std::result::Result<String, ProbeError> {
        let child: Child = Command::new(&self.exe)
            .args(default_args(target))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| ProbeError::NotFound)?;
        self.read_child(child)
    }

    fn read_child(&self, mut child: Child) -> std::result::Result<String, ProbeError> {
        let started = Instant::now();
        // 立刻入 job：孙进程继承的管道句柄由 job 关闭来释放
        let job = job::attach(child.id());

        let stdout = child.stdout.take().ok_or(ProbeError::NotFound)?;
        // stderr 也必须**一边跑一边排空**。原先是 wait() 之后才读，而管道缓冲只有几 KB：
        // 子进程写满之后就阻塞，于是永远不会退出，wait() 也就永远不返回 —— 我们等的是
        // "对方先把话说完"，而对方等的是"我们先把嘴张开"（ISSUES D10）。
        // ffprobe 恰恰是个话多的进程：坏包、反复 PS 探测、每条流一行警告。
        let stderr = child.stderr.take().ok_or(ProbeError::NotFound)?;
        let (etx, erx) = mpsc::channel::<(String, bool)>();
        let err_reader = std::thread::spawn(move || {
            let (text, exceeded) = read_bounded(stderr, 64 * 1024, true);
            let _ = etx.send((text, exceeded));
        });
        let (tx, rx) = mpsc::channel::<(String, bool)>();
        let reader = std::thread::spawn(move || {
            let result = read_bounded(stdout, 32 * 1024 * 1024, false);
            let _ = tx.send(result);
        });

        let mut stdout_result = None;
        let mut stderr_result = None;
        let mut status = None;
        loop {
            if stdout_result.is_none() {
                stdout_result = rx.try_recv().ok();
            }
            if stderr_result.is_none() {
                stderr_result = erx.try_recv().ok();
            }
            if status.is_none() {
                status = child.try_wait().ok().flatten();
            }
            if stdout_result.is_some() && stderr_result.is_some() && status.is_some() {
                break;
            }
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let contained = job.is_ok();
                let job_err = job.as_ref().err().copied().unwrap_or(0);
                drop(job);
                // 两条管道分别等待；stdout EOF 不能证明 stderr 已被释放。
                if stdout_result.is_some() || rx.recv_timeout(Duration::from_millis(200)).is_ok() {
                    let _ = reader.join();
                }
                if stderr_result.is_some() || erx.recv_timeout(Duration::from_millis(200)).is_ok() {
                    let _ = err_reader.join();
                }
                let _ = child.try_wait();
                return Err(if contained {
                    ProbeError::Timeout
                } else {
                    ProbeError::TimeoutUncontained(job_err)
                });
            }
            std::thread::sleep(
                Duration::from_millis(10).min(self.timeout.saturating_sub(started.elapsed())),
            );
        }
        let (text, stdout_exceeded) = stdout_result.expect("循环已确认 stdout 读取完成");
        let (mut err, stderr_exceeded) = stderr_result.expect("循环已确认 stderr 读取完成");
        let status = status.expect("循环已确认子进程退出");
        let _ = reader.join();
        let _ = err_reader.join();
        if stdout_exceeded {
            return Err(ProbeError::OutputTooLarge("stdout", 32 * 1024 * 1024));
        }
        if !status.success() {
            if stderr_exceeded {
                err.insert_str(0, "[stderr 前段已截断；保留末尾 64 KiB] ");
            }
            return Err(ProbeError::NonZero(status.code().unwrap_or(-1), err));
        }
        Ok(text)
    }
}

fn read_bounded(mut reader: impl Read, limit: usize, keep_tail: bool) -> (String, bool) {
    let mut raw = Vec::with_capacity(limit.min(16 * 1024));
    let mut buf = [0u8; 8192];
    let mut exceeded = false;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if keep_tail {
            if n >= limit {
                raw.clear();
                raw.extend_from_slice(&buf[n - limit..n]);
                exceeded = true;
                continue;
            }
            if raw.len() + n > limit {
                let excess = raw.len() + n - limit;
                raw.drain(..excess);
                exceeded = true;
            }
            raw.extend_from_slice(&buf[..n]);
        } else {
            let remaining = limit.saturating_sub(raw.len());
            let kept = remaining.min(n);
            raw.extend_from_slice(&buf[..kept]);
            exceeded |= kept < n;
        }
    }
    (String::from_utf8_lossy(&raw).into_owned(), exceeded)
}

impl ProbeRunner for SystemProbe {
    fn probe(&self, target: &Path) -> Result<String> {
        self.spawn_and_read(target).map_err(|e| e.to_stk(&self.exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn timeout_after_closing_output(test_name: &str, scenario: &str) {
        const CHILD_FLAG: &str = "STK_PROBE_CLOSED_OUTPUT_CHILD";
        if let Ok(mode) = std::env::var(CHILD_FLAG) {
            if mode == "sleep" {
                std::thread::sleep(Duration::from_secs(5));
                std::process::exit(0);
            }
            if scenario == "stderr-descendant" {
                #[allow(
                    clippy::zombie_processes,
                    reason = "测试需要父进程先退出并留下 stderr 持有者；探测器负责清理进程树"
                )]
                let _descendant = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", test_name, "--nocapture"])
                    .env(CHILD_FLAG, "sleep")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::inherit())
                    .spawn()
                    .unwrap();
            }
            use std::os::windows::io::RawHandle;
            extern "system" {
                fn GetStdHandle(kind: u32) -> RawHandle;
                fn CloseHandle(handle: RawHandle) -> i32;
            }
            unsafe {
                assert_ne!(CloseHandle(GetStdHandle(-11i32 as u32)), 0);
                if scenario != "stdout" {
                    assert_ne!(CloseHandle(GetStdHandle(-12i32 as u32)), 0);
                }
            }
            if scenario == "stderr-descendant" {
                std::thread::sleep(Duration::from_millis(100));
                std::process::exit(0);
            }
            std::thread::sleep(Duration::from_secs(5));
            std::process::exit(0);
        }

        let exe = std::env::current_exe().unwrap();
        let child = Command::new(&exe)
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD_FLAG, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let probe = SystemProbe::new(&exe, Duration::from_secs(1));
        let result = probe.read_child(child);
        assert!(matches!(
            result,
            Err(ProbeError::Timeout | ProbeError::TimeoutUncontained(_))
        ));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "关闭输出管道后仍须执行超时"
        );
    }

    #[cfg(windows)]
    #[test]
    fn timeout_covers_exit_after_stdout_is_closed() {
        timeout_after_closing_output(
            "runner::tests::timeout_covers_exit_after_stdout_is_closed",
            "stdout",
        );
    }

    #[cfg(windows)]
    #[test]
    fn timeout_covers_exit_after_both_output_pipes_are_closed() {
        timeout_after_closing_output(
            "runner::tests::timeout_covers_exit_after_both_output_pipes_are_closed",
            "both",
        );
    }

    #[cfg(windows)]
    #[test]
    fn timeout_covers_stderr_held_by_a_descendant_after_the_child_exits() {
        timeout_after_closing_output(
            "runner::tests::timeout_covers_stderr_held_by_a_descendant_after_the_child_exits",
            "stderr-descendant",
        );
    }

    #[test]
    fn subprocess_output_is_bounded_and_stderr_keeps_its_tail() {
        let (stdout, stdout_exceeded) = read_bounded(&b"abcdefgh"[..], 4, false);
        assert_eq!(stdout, "abcd");
        assert!(stdout_exceeded);

        let (stderr, stderr_exceeded) = read_bounded(&b"abcdefgh"[..], 4, true);
        assert_eq!(stderr, "efgh");
        assert!(stderr_exceeded);
    }

    #[test]
    fn args_are_a_fixed_array_ending_with_target() {
        let (exe, args) = build_command(
            Path::new("C:\\tools\\ffprobe.exe"),
            Path::new("D:\\movie.mkv"),
        );
        assert_eq!(exe, PathBuf::from("C:\\tools\\ffprobe.exe"));
        assert_eq!(args.first().map(String::as_str), Some("-v"));
        assert_eq!(args.last().map(String::as_str), Some("D:\\movie.mkv"));
        assert!(args.contains(&"-show_streams".to_string()));
        assert!(
            args.iter().all(|a| !a.contains('&') && !a.contains('|')),
            "参数不得含 shell 连接符"
        );
    }

    #[test]
    fn missing_binary_yields_dependency_error_not_panic() {
        // 16 §5：ffprobe 不存在时错误可诊断，且不阻塞主流程
        let p = SystemProbe::new(
            "C:\\definitely\\not\\here\\ffprobe.exe",
            Duration::from_millis(500),
        );
        let e = p.probe(Path::new("X:\\nothing.mkv")).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0020");
        assert!(e.user_action.is_some());
    }

    /// 真·超时用例：造一个会挂 30 秒的批处理，必须在超时后被 kill 并快速返回。
    #[test]
    fn timeout_kills_child() {
        if std::env::var_os("STK_SKIP_SLOW_TESTS").is_some() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("stk-probe-to-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bat = dir.join("slowprobe.bat");
        std::fs::write(&bat, b"@echo off\r\nping -n 30 127.0.0.1 >nul\r\n").unwrap();
        let p = SystemProbe::new(&bat, Duration::from_millis(400));
        let started = std::time::Instant::now();
        let e = p.probe(Path::new("X:\\nothing.mkv"));
        let elapsed = started.elapsed();
        // 0021 = 超时且进程树被 Job Object 约束住；0023 = 超时但约束失败
        // （宿主已把我们放进一个外层 Job，比如 nextest）。两者都是"快速拿到超时"，
        // 但只有 0021 能承诺没留活物，所以这个测试必须区分它们而不是随便接受一个。
        assert!(
            matches!(e, Err(ref s) if s.code_string() == "STK-DEP-0021"
                || s.code_string() == "STK-DEP-0023"),
            "应为超时错误: {e:?}"
        );
        if e.as_ref().unwrap_err().code_string() == "STK-DEP-0023" {
            let d = e.unwrap_err().details.join(" ");
            assert!(d.contains("Job Object"), "约束失败却不说明原因：{d}");
        }
        assert!(
            elapsed < Duration::from_secs(5),
            "超时后必须尽快返回，实际 {elapsed:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn nonzero_exit_is_reported_with_stderr() {
        // 不存在的 ffprobe 参数组合：验证非零退出分支确实带 exit code，而不是静默成功
        let dir = std::env::temp_dir().join(format!("stk-probe-nz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bat = dir.join("failprobe.bat");
        std::fs::write(&bat, b"@echo off\r\necho bad stream & exit /b 3\r\n").unwrap();
        let p = SystemProbe::new(&bat, Duration::from_secs(20));
        let e = p.probe(Path::new("X:\\nothing.mkv")).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0022");
        assert!(
            e.details.iter().any(|d| d.contains("exit=3")),
            "{:?}",
            e.details
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 子进程把 stderr 写满之后，不能把我们自己也一起卡住（ISSUES D10）。
    ///
    /// 匿名管道的缓冲只有几 KB：老代码是 `child.wait()` **之后**才读 stderr，于是子进程写满
    /// 就阻塞、不退出，而我们的 wait 也不返回 —— 那条路径上根本没有超时（timeout 只管 stdout
    /// 那条线程）。ffprobe 恰是话多的进程：坏包、反复 PS 探测、每条流一行警告。
    ///
    /// 判据是**时间上界**：假 probe 吐 200 KB stderr，必须在 25 秒内交出结果。
    /// 结果拿在线程外面收，是为了让"又退回老写法"这种情况报 FAIL 而不是把门阶挂死。
    #[test]
    fn chatty_stderr_does_not_deadlock_the_wait() {
        let dir = std::env::temp_dir().join(format!("stk-probe-noise-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let noise = "x".repeat(200 * 1024);
        let noise_file = dir.join("noise.txt");
        std::fs::write(&noise_file, noise.as_bytes()).unwrap();
        let bat = dir.join("noiseprobe.bat");
        std::fs::write(
            &bat,
            format!(
                "@echo off\r\necho {{\"programs\":[]}}\r\ntype \"{}\" 1>&2\r\n",
                noise_file.display()
            ),
        )
        .unwrap();

        let (tx, rx) = mpsc::channel::<std::result::Result<usize, String>>();
        let bat_for_thread = bat.clone();
        std::thread::spawn(move || {
            let p = SystemProbe::new(&bat_for_thread, Duration::from_secs(30));
            let r = p.probe(Path::new("X:\\nothing.mkv")).map(|s| s.len());
            let _ = tx.send(r.map_err(|e| format!("{e:?}")));
        });
        let got = match rx.recv_timeout(Duration::from_secs(25)) {
            Ok(v) => v,
            Err(_) => panic!("25 秒没交出结果：又是\"先等退出、后读管道\"的死锁（D10）"),
        };
        assert!(got.is_ok(), "话多的子进程也该拿到 stdout：{:?}", got.err());
        assert!(got.unwrap() > 0, "拿到的正文是空的");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn identity_digest_is_stable_and_detects_change() {
        let dir = std::env::temp_dir().join(format!("stk-probe-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ffprobe.exe");
        std::fs::write(&f, b"AAA").unwrap();
        let a = FfprobeIdentity::of(&f).unwrap();
        let b = FfprobeIdentity::of(&f).unwrap();
        assert_eq!(a.digest, b.digest);
        assert_eq!(a.size, 3);
        // 外部锚点：SHA-256("AAA")。哈希实现对错不能靠"它自己两次一致"来证明——
        // 那连"哈希的是路径而不是内容"这种错都抓不出来。
        assert_eq!(
            a.digest, "sha256-cb1ad2119d8fafb69566510ee712661f9f14b83385006ef92aec47f523a38358",
            "{a:?}"
        );
        std::fs::write(&f, b"BBB").unwrap();
        assert_ne!(
            a.digest,
            FfprobeIdentity::of(&f).unwrap().digest,
            "内容变化必须让 cache 失效"
        );
        assert!(FfprobeIdentity::of(&dir.join("missing.exe")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
