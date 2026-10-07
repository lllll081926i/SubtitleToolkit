//! ffprobe 进程适配（23 §外部进程：绝对路径、参数数组、超时、不拼 shell 字符串）。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

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
        let mut child: Child = Command::new(&self.exe)
            .args(default_args(target))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| ProbeError::NotFound)?;
        // 立刻入 job：孙进程继承的管道句柄由 job 关闭来释放
        let job = job::attach(child.id());

        let mut stdout = child.stdout.take().ok_or(ProbeError::NotFound)?;
        // stderr 也必须**一边跑一边排空**。原先是 wait() 之后才读，而管道缓冲只有几 KB：
        // 子进程写满之后就阻塞，于是永远不会退出，wait() 也就永远不返回 —— 我们等的是
        // "对方先把话说完"，而对方等的是"我们先把嘴张开"（ISSUES D10）。
        // ffprobe 恰恰是个话多的进程：坏包、反复 PS 探测、每条流一行警告。
        let mut stderr = child.stderr.take().ok_or(ProbeError::NotFound)?;
        let (etx, erx) = mpsc::channel::<String>();
        let err_reader = std::thread::spawn(move || {
            let mut raw = Vec::new();
            let _ = stderr.read_to_end(&mut raw);
            let _ = etx.send(String::from_utf8_lossy(&raw).into_owned());
        });
        let (tx, rx) = mpsc::channel::<String>();
        let reader = std::thread::spawn(move || {
            let mut buf = String::new();
            // 读侧只做字节到字符串的尽力转换，非 UTF-8 走 lossy（JSON 本应 UTF-8）
            let mut raw = Vec::new();
            let _ = stdout.read_to_end(&mut raw);
            buf.push_str(&String::from_utf8_lossy(&raw));
            let _ = tx.send(buf);
        });

        let text = match rx.recv_timeout(self.timeout) {
            Ok(t) => t,
            Err(_) => {
                let _ = child.kill();
                // 约束住（Job 附着成功）：关闭 job → 整棵进程树终止 → 管道必然 EOF，
                // join 有上界，读线程不会被留在进程里。
                // 没约束住（附着失败）：孙进程可能还握着管道，join 会一直阻塞到它自己结束
                // （实测 30 秒）。这时只给一个很短的宽限期，等不到就把线程留下，
                // 并把"没约束住"如实报出去（ISSUES Z6）。
                let contained = job.is_ok();
                let job_err = job.as_ref().err().copied().unwrap_or(0);
                drop(job);
                let _ = child.wait();
                let reader_done = contained || rx.recv_timeout(Duration::from_millis(200)).is_ok();
                if reader_done {
                    let _ = reader.join();
                    let _ = err_reader.join();
                }
                return Err(if contained {
                    ProbeError::Timeout
                } else {
                    ProbeError::TimeoutUncontained(job_err)
                });
            }
        };
        let status = child.wait().map_err(|_| {
            drop(job);
            ProbeError::Timeout
        })?;
        if !status.success() {
            // stderr 由那条线程收全（不是从 child 上再读一次——句柄已经 take 走了）
            let err = erx.recv().unwrap_or_default();
            let _ = reader.join();
            let _ = err_reader.join();
            return Err(ProbeError::NonZero(status.code().unwrap_or(-1), err));
        }
        let _ = reader.join();
        let _ = err_reader.join();
        Ok(text)
    }
}

impl ProbeRunner for SystemProbe {
    fn probe(&self, target: &Path) -> Result<String> {
        self.spawn_and_read(target).map_err(|e| e.to_stk(&self.exe))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
