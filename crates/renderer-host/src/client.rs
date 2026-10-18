//! 主进程这一侧：起 host、握手、要帧、处理崩溃与超时（12 §2）。
//!
//! 三条纪律决定了这个文件的形状：
//!
//! 1. **位数检查在加载之前**（12 §3）。`pe::arch_of_file` 只读 PE 头，不加载 DLL；
//!    provider 与 host 的位数不一致就直接拒绝，绝不允许"试一下 `LoadLibrary` 看看行不行"。
//! 2. **失败必须带证据**（38 §1）。崩溃返回退出码 + host 留下的 stderr 尾部 +
//!    "这次有没有 Job 约束住整棵进程树"；超时返回等了多久、以及约束状态。
//!    静默退化是本仓库反复犯的那类错（ISSUES Z6）。
//! 3. **帧不直接信**。长度必须等于 `w*h*4`，摘要必须等于 host 报的 SHA-256
//!    （12 §8 的去重纪律反过来用就是"传输完整性"）。

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use core_model::canvas::Canvas;
use core_model::loss::LossReport;
use sha2::{Digest as _, Sha256};

use crate::crash;
use crate::pe::{self, PeArch};
use crate::protocol::{self, Handshake, Message, Request, Response, API_VERSION};

/// 一次 host 会话的请求参数。
pub struct HostSpec<'a> {
    /// `renderer-host-x64.exe` / `renderer-host-x86.exe`（28 §2 的产物）
    pub host_exe: &'a Path,
    pub dll: &'a Path,
    pub storage: Canvas,
    pub output: Canvas,
    pub default_font: &'a str,
    /// 单次请求（含握手）的等待上限
    pub timeout: Duration,
    /// 测试专用：让宿主进程以指定方式自我了断（AT-RENDER-002）。
    /// 它是**启动环境**的一部分，不是控制面指令，所以发布二进制没有可远程触发的自杀路径。
    /// 传 `None` 就是正常宿主，占用该进程的 `STK_HOST_CRASH` 环境变量。
    pub crash_injection: Option<&'a str>,
    /// 22 §6：host 把崩溃现场写到哪里。
    ///
    /// `None` = 不记录（host 仍然正常工作，只是崩溃后只能看到退出码）。
    /// 由调用方指定而不是硬编码：部署形态不同（便携包、安装版、测试）
    /// 应该落在不同的目录里，而那是调用方知道的事。
    pub crash_dir: Option<&'a Path>,
}

/// 把 host 回传的 details 拼成一句可读的尾巴。
///
/// 为空时返回空串（调用方直接拼接，不会多出一个分隔符）。
fn join_details(details: &[String]) -> String {
    if details.is_empty() {
        return String::new();
    }
    format!("（{}）", details.join("；"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostFailure {
    /// 位数不符或架构未知：主进程据此换 host 或明确放弃（12 §3）
    Arch {
        provider: String,
        host: String,
    },
    Spawn(String),
    /// 等不到回应：调用方要知道"我等了多久、树有没有被约束住"
    Timeout {
        waited_ms: u64,
        contained: bool,
        job_error: u32,
    },
    /// host 死了（崩溃、abort、被系统干掉）
    Crashed {
        exit_code: Option<u32>,
        log_tail: String,
        contained: bool,
        job_error: u32,
        /// 22 §6：host 死之前写下的现场（阶段、第几步、provider 身份）。
        /// `None` = 没有记录（大多数崩溃发生在写不了记录的地方，或没设目录）。
        /// 有这个字段与没有它是两回事：前者能回答“哪一步死的”。
        crash_note: Option<Box<crash::CrashNote>>,
    },
    Protocol(String),
    Io(String),
}

impl HostFailure {
    /// 22 §6：这次崩溃的现场摘要（如果有记录）。
    ///
    /// 提成一个方法而不是让调用方去 match 变体：
    /// “崩溃时给出一行现场”是一个契约，不该由每个调用点各自记得怎么写。
    pub fn crash_summary(&self) -> Option<String> {
        match self {
            HostFailure::Crashed {
                crash_note: Some(n),
                ..
            } => Some(crate::crash::summarize(n)),
            _ => None,
        }
    }

    /// 给人看的错误码（20 §错误模型：每一类失败都要有能对上的码）。
    pub fn code(&self) -> &'static str {
        match self {
            HostFailure::Arch { .. } => "STK-RND-0201",
            HostFailure::Spawn(_) => "STK-RND-0202",
            HostFailure::Timeout { .. } => "STK-RND-0203",
            HostFailure::Crashed { .. } => "STK-RND-0204",
            HostFailure::Protocol(_) => "STK-RND-0205",
            HostFailure::Io(_) => "STK-RND-0206",
        }
    }

    pub fn title(&self) -> String {
        match self {
            HostFailure::Arch { provider, host } => {
                format!("provider 与 host 位数不符（provider {provider}，host {host}）")
            }
            HostFailure::Spawn(s) => format!("无法启动 renderer-host：{s}"),
            HostFailure::Timeout { waited_ms, .. } => {
                format!("renderer-host 在 {waited_ms} ms 内没有回应")
            }
            HostFailure::Crashed { exit_code, .. } => format!(
                "renderer-host 终止（退出码 {}）",
                exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "无".to_string())
            ),
            HostFailure::Protocol(s) => format!("renderer-host 协议不符：{s}"),
            HostFailure::Io(s) => format!("与 renderer-host 的管道出错：{s}"),
        }
    }
}

impl From<io::Error> for HostFailure {
    fn from(e: io::Error) -> Self {
        HostFailure::Io(e.to_string())
    }
}

/// 一帧输出像素。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// libass 的 `detect_change` 原值，只当 fast hint 用（12 §8）
    pub detect_change: i32,
    pub images: usize,
    pub loss: LossReport,
}

/// 一个活着的 host 会话。
pub struct Host {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<io::Result<Message>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// `None` 意味着孙进程没被约束住（例如本进程已经属于另一个 Job —— nextest 就是这样）
    job: Option<win_job::Job>,
    job_error: u32,
    handshake: Handshake,
    timeout: Duration,
    exe: PathBuf,
    /// 22 §6：崩溃现场的目录。`None` = host 不记录（拿不到目录）。
    crash_dir: Option<PathBuf>,
}

impl Host {
    /// 起 host。**这里不做任何 `LoadLibrary`**：位数不符在进程边界上就拦掉。
    pub fn start(spec: &HostSpec<'_>) -> Result<Host, HostFailure> {
        let provider_arch =
            pe::arch_of_file(spec.dll).map_err(|e| HostFailure::Io(e.to_string()))?;
        let host_arch =
            pe::arch_of_file(spec.host_exe).map_err(|e| HostFailure::Io(e.to_string()))?;
        if !same_bitness(provider_arch, host_arch) {
            return Err(HostFailure::Arch {
                provider: arch_text(provider_arch),
                host: arch_text(host_arch),
            });
        }

        let mut command = Command::new(spec.host_exe);
        if let Some(how) = spec.crash_injection {
            command.env(protocol::CRASH_ENV, how);
        }
        // 22 §6：崩溃现场写到哪儿。与 `STK_HOST_CRASH` 同一类：
        // 启动环境而不是控制面，客户端无法在运行时改它。
        if let Some(dir) = spec.crash_dir {
            command.env(protocol::CRASH_DIR_ENV, dir);
            // 22 §6：**先删掉上一次的记录**再起进程。
            //
            // 不删的后果很隐蔽：这次崩溃发生在写不出记录的地方（访问违例、
            // 栈溢出），而目录里还留着上一回的现场 —— 于是报告会把上一次崩溃的
            // “阶段/第几帧/provider”当成这一次的。**一份错的现场比没有现场坏得多**，
            // 因为它会让调查直接走错方向。删失败不算错（没有旧文件就是常态）。
            let _ = std::fs::remove_file(dir.join(crate::crash::FILE_NAME));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| HostFailure::Spawn(format!("{}：{e}", spec.host_exe.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| HostFailure::Spawn("拿不到 host 的 stdin".to_string()))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| HostFailure::Spawn("拿不到 host 的 stdout".to_string()))?;
        let mut stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| HostFailure::Spawn("拿不到 host 的 stderr".to_string()))?;

        // 读线程 + 通道：这样"等回应"可以有超时，而不是把调用方挂死在 read 上。
        // 读线程同时把 EOF 变成一个明确的错误消息，让"host 崩了"与"host 还在想"区分得开。
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || receive_messages(&mut stdout, tx));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&stderr);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut g) = sink.lock() {
                            const STDERR_TAIL_LIMIT: usize = 64 * 1024;
                            if n >= STDERR_TAIL_LIMIT {
                                g.clear();
                                g.extend_from_slice(&buf[n - STDERR_TAIL_LIMIT..n]);
                                continue;
                            }
                            if g.len() + n > STDERR_TAIL_LIMIT {
                                let excess = g.len() + n - STDERR_TAIL_LIMIT;
                                g.drain(..excess);
                            }
                            g.extend_from_slice(&buf[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // 12 §2：Job Object 限制 child tree。附着失败不能静默——它意味着只能杀直接子进程，
        // 孙进程可能继续跑（Z6 实测过的正是这个场景）。
        let (job, job_error) = match win_job::attach(child.id()) {
            Ok(j) => (Some(j), 0u32),
            Err(code) => (None, code),
        };

        let mut host = Host {
            child,
            stdin,
            rx,
            stderr,
            job,
            job_error,
            handshake: empty_handshake(),
            timeout: spec.timeout,
            exe: spec.host_exe.to_path_buf(),
            crash_dir: spec.crash_dir.map(|p| p.to_path_buf()),
        };

        let hello = Request::Hello {
            dll: spec.dll.display().to_string(),
            storage: [spec.storage.width, spec.storage.height],
            output: [spec.output.width, spec.output.height],
            default_font: spec.default_font.to_string(),
        };
        protocol::write_message(&mut host.stdin, &protocol::to_json(&hello)?)?;
        match host.recv() {
            Ok(Response::Hello(hs)) => {
                host.handshake = hs;
                host.verify_handshake()?;
            }
            Ok(Response::Error {
                code,
                title,
                details,
            }) => {
                return Err(HostFailure::Protocol(format!(
                    "host 拒绝握手：{code} {title}{}",
                    join_details(&details)
                )))
            }
            Ok(other) => return Err(HostFailure::Protocol(format!("握手收到意外消息 {other:?}"))),
            Err(e) => return Err(e),
        }
        Ok(host)
    }

    fn verify_handshake(&self) -> Result<(), HostFailure> {
        let h = &self.handshake;
        if h.api_version != API_VERSION {
            return Err(HostFailure::Protocol(format!(
                "协议版本应为 {API_VERSION}，host 报 {}",
                h.api_version
            )));
        }
        if h.pixel_format != "RGBA8" {
            return Err(HostFailure::Protocol(format!(
                "不支持的像素格式 {}",
                h.pixel_format
            )));
        }
        if h.provider != "libass" {
            return Err(HostFailure::Protocol(format!(
                "这个 host 只会讲 libass 协议，收到 provider {}",
                h.provider
            )));
        }
        Ok(())
    }

    pub fn handshake(&self) -> &Handshake {
        &self.handshake
    }

    /// 12 §4：provider 的身份是哈希。两侧各自算一次，对不上就是"看的不是同一个文件"。
    pub fn provider_sha_matches(&self, dll: &Path) -> bool {
        let Ok(bytes) = std::fs::read(dll) else {
            return false;
        };
        let mut h = Sha256::new();
        h.update(bytes);
        let mine: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        mine == self.handshake.provider_binary_sha256
    }

    pub fn load_script(&mut self, ass: &[u8]) -> Result<usize, HostFailure> {
        protocol::write_message(&mut self.stdin, &protocol::to_json(&Request::LoadScript)?)?;
        protocol::write_message(&mut self.stdin, &Message::Blob(ass.to_vec()))?;
        match self.recv()? {
            Response::ScriptLoaded { bytes } => Ok(bytes),
            Response::Error {
                code,
                title,
                details,
            } => Err(HostFailure::Protocol(format!(
                "host 拒绝加载脚本 {code}：{title}{}",
                join_details(&details)
            ))),
            other => Err(HostFailure::Protocol(format!(
                "LoadScript 收到意外消息 {other:?}"
            ))),
        }
    }

    pub fn add_font(&mut self, family: &str, bytes: &[u8]) -> Result<usize, HostFailure> {
        protocol::write_message(
            &mut self.stdin,
            &protocol::to_json(&Request::AddFont {
                family: family.to_string(),
            })?,
        )?;
        protocol::write_message(&mut self.stdin, &Message::Blob(bytes.to_vec()))?;
        match self.recv()? {
            Response::FontAdded { bytes } => Ok(bytes),
            Response::Error {
                code,
                title,
                details,
            } => Err(HostFailure::Protocol(format!(
                "host 拒绝注入字体 {code}：{title}{}",
                join_details(&details)
            ))),
            other => Err(HostFailure::Protocol(format!(
                "AddFont 收到意外消息 {other:?}"
            ))),
        }
    }

    pub fn render(&mut self, pts_ms: i64) -> Result<Frame, HostFailure> {
        protocol::write_message(
            &mut self.stdin,
            &protocol::to_json(&Request::Render { pts_ms })?,
        )?;
        let (w, h, frame_bytes, digest, detect_change, images, loss) = match self.recv()? {
            Response::Frame {
                width,
                height,
                bytes,
                digest,
                detect_change,
                images,
                loss,
            } => (width, height, bytes, digest, detect_change, images, loss),
            Response::Error {
                code,
                title,
                details,
            } => {
                return Err(HostFailure::Protocol(format!(
                    "host 渲染失败 {code}：{title}{}",
                    join_details(&details)
                )))
            }
            other => {
                return Err(HostFailure::Protocol(format!(
                    "Render 收到意外消息 {other:?}"
                )))
            }
        };
        let expected = (w as usize)
            .checked_mul(h as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| HostFailure::Protocol("host 帧尺寸乘法溢出".to_string()))?;
        if frame_bytes != expected {
            return Err(HostFailure::Protocol(format!(
                "帧长度 {frame_bytes} 与声明的 {w}x{h} RGBA 需要的 {expected} 不符"
            )));
        }
        let mut body = Vec::new();
        body.try_reserve_exact(expected)
            .map_err(|e| HostFailure::Protocol(format!("无法分配 {expected} 字节帧：{e}")))?;
        while body.len() < expected {
            let chunk = match self.recv_msg()? {
                Message::Blob(b) => b,
                Message::Text(t) => {
                    return Err(HostFailure::Protocol(format!(
                        "期望 RGBA 字节块，收到文本 {t}"
                    )))
                }
            };
            let remaining = expected - body.len();
            if chunk.is_empty() || chunk.len() > remaining {
                return Err(HostFailure::Protocol(format!(
                    "RGBA 字节块长度非法：{}，剩余 {remaining}",
                    chunk.len()
                )));
            }
            body.extend_from_slice(&chunk);
        }
        let mut hasher = Sha256::new();
        hasher.update(&body);
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if got != digest {
            return Err(HostFailure::Protocol(format!(
                "帧摘要不符：host 报 {digest}，实际 {got}"
            )));
        }
        Ok(Frame {
            width: w,
            height: h,
            rgba: body,
            detect_change,
            images,
            loss,
        })
    }

    pub fn shutdown(&mut self) -> Result<(), HostFailure> {
        protocol::write_message(&mut self.stdin, &protocol::to_json(&Request::Bye)?)?;
        match self.recv() {
            Ok(Response::ByeOk) => Ok(()),
            // Bye 之后 host 直接退出、管道先断，也是正常收尾
            Err(HostFailure::Crashed { .. }) | Err(HostFailure::Timeout { .. }) => Ok(()),
            Ok(other) => Err(HostFailure::Protocol(format!("Bye 收到意外消息 {other:?}"))),
            Err(e) => Err(e),
        }
    }

    /// 这次调用不要了：杀 host，并让 Job 带走整棵树。
    /// 返回"孙进程到底有没有被约束住"，调用方必须把它说清楚而不是假装干净。
    pub fn abandon(&mut self) -> Abandoned {
        let killed = self.child.kill().is_ok();
        let _ = self.child.wait();
        let contained = self.job.take().is_some();
        Abandoned {
            killed,
            contained,
            job_error: self.job_error,
        }
    }

    /// host 自己的可执行文件路径（诊断用）。
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// 22 §6：把 host 死前写下的现场读回来。
    ///
    /// 读不到就返回 `None`：没有记录是**常态**（大多数运行不崩，而真崩溃常常
    /// 发生在写不了记录的地方 —— 访问违例、栈溢出）。把“没记录”当成错误
    /// 只会给用户一个他无法处理的噪声。
    ///
    /// 这个记录属于**本次**会话：旧记录在 `start` 里就被删掉了（见那里的注释）。
    fn crash_note(&self) -> Option<Box<crate::crash::CrashNote>> {
        let dir = self.crash_dir.as_ref()?;
        crate::crash::read(dir).map(Box::new)
    }

    /// provider log 尾部（12 §2："crash code / provider log 返回主进程"）。
    pub fn log_tail(&self, bytes: usize) -> String {
        let Ok(g) = self.stderr.lock() else {
            return String::new();
        };
        let start = g.len().saturating_sub(bytes);
        String::from_utf8_lossy(&g[start..]).into_owned()
    }

    pub fn job_error(&self) -> u32 {
        self.job_error
    }

    pub fn contained(&self) -> bool {
        self.job.is_some()
    }

    fn recv_msg(&mut self) -> Result<Message, HostFailure> {
        let outcome = self.rx.recv_timeout(self.timeout);
        match outcome {
            Ok(Ok(msg)) => Ok(msg),
            Ok(Err(e)) => Err(self.disconnected(e)),
            Err(e) => Err(self.wait_failure(e)),
        }
    }

    fn recv(&mut self) -> Result<Response, HostFailure> {
        let msg = self.recv_msg()?;
        protocol::from_json(&msg).map_err(|e| HostFailure::Io(e.to_string()))
    }

    /// 读线程报来的管道错误：如果是"host 关掉了控制面"，那就是崩溃/退出，
    /// 要把退出码和日志一起拿回来。
    fn disconnected(&mut self, e: io::Error) -> HostFailure {
        if e.kind() != io::ErrorKind::UnexpectedEof && e.kind() != io::ErrorKind::BrokenPipe {
            return HostFailure::Io(e.to_string());
        }
        let mut exit_code = None;
        let deadline = Instant::now() + Duration::from_millis(2_000);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                exit_code = status.code().map(|c| c as u32);
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        HostFailure::Crashed {
            exit_code,
            log_tail: self.log_tail(4096),
            crash_note: self.crash_note(),
            contained: self.job.is_some(),
            job_error: self.job_error,
        }
    }

    fn wait_failure(&self, e: RecvTimeoutError) -> HostFailure {
        match e {
            RecvTimeoutError::Timeout => HostFailure::Timeout {
                waited_ms: self.timeout.as_millis() as u64,
                contained: self.job.is_some(),
                job_error: self.job_error,
            },
            RecvTimeoutError::Disconnected => HostFailure::Crashed {
                exit_code: None,
                log_tail: self.log_tail(4096),
                crash_note: self.crash_note(),
                contained: self.job.is_some(),
                job_error: self.job_error,
            },
        }
    }
}

fn receive_messages(reader: &mut impl Read, tx: mpsc::SyncSender<io::Result<Message>>) {
    loop {
        let outcome = match protocol::read_message(reader) {
            Ok(None) => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "host 关闭了控制面",
            )),
            Ok(Some(msg)) => Ok(msg),
            Err(e) => Err(e),
        };
        let terminal = outcome.is_err();
        if tx.send(outcome).is_err() || terminal {
            break;
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        // 顺序要紧：先让直接子进程退出，再关 Job 句柄带走可能的孙进程
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(job) = self.job.take() {
            drop(job);
        }
    }
}

impl std::fmt::Debug for Host {
    /// 只报会改变判断的信息：是谁、在哪、约束住了没有。
    /// （`Child`/管道句柄本身没有可用的 Debug，硬凑出来只会误导。）
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("exe", &self.exe)
            .field("pid", &self.child.id())
            .field("handshake", &self.handshake)
            .field("contained", &self.job.is_some())
            .field("job_error", &self.job_error)
            .field("timeout_ms", &self.timeout.as_millis())
            .finish()
    }
}

/// `abandon()` 的结果：调用方必须能把"约束是否生效"讲清楚。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Abandoned {
    pub killed: bool,
    pub contained: bool,
    pub job_error: u32,
}

fn empty_handshake() -> Handshake {
    Handshake {
        provider: String::new(),
        name: String::new(),
        version: String::new(),
        api_version: 0,
        host_arch: String::new(),
        provider_arch: String::new(),
        pixel_format: String::new(),
        provider_binary_sha256: String::new(),
    }
}

fn same_bitness(a: PeArch, b: PeArch) -> bool {
    use PeArch::*;
    match (a, b) {
        (X64, X64) | (X86, X86) | (Arm64, Arm64) | (Arm, Arm) => true,
        // 未知架构一律不当成"能配对"，哪怕两边数字一样（12 §3：其他架构拒绝并给诊断）
        _ => false,
    }
}

fn arch_text(a: PeArch) -> String {
    match a {
        PeArch::Other(v) => format!("未知架构 0x{v:04x}"),
        other => other.label().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn closed_pipe_reports_eof_once_and_stops_the_reader() {
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut child =
                Command::new(std::env::var_os("COMSPEC").unwrap_or_else(|| "cmd.exe".into()))
                    .args(["/D", "/C", "exit 0"])
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
            receive_messages(&mut child.stdout.take().unwrap(), tx);
            assert!(child.wait().unwrap().success());
        });
        let error = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)),
            Err(RecvTimeoutError::Disconnected)
        ));
        reader.join().unwrap();
    }

    #[test]
    fn truncated_payload_reports_one_error_and_does_not_retry_the_stream() {
        let (tx, rx) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut bytes = std::io::Cursor::new([protocol::TAG_BLOB, 2, 0, 0, 0, 7]);
            receive_messages(&mut bytes, tx);
        });
        let error = rx
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(10)),
            Err(RecvTimeoutError::Disconnected)
        ));
        reader.join().unwrap();
    }

    #[test]
    fn bitness_gate_accepts_only_matching_known_pairs() {
        assert!(same_bitness(PeArch::X64, PeArch::X64));
        assert!(same_bitness(PeArch::X86, PeArch::X86));
        assert!(!same_bitness(PeArch::X86, PeArch::X64));
        assert!(
            !same_bitness(PeArch::Other(0x1234), PeArch::Other(0x1234)),
            "未知架构即使两边相同也不能放行"
        );
        assert_eq!(arch_text(PeArch::Other(0x1234)), "未知架构 0x1234");
    }

    #[test]
    fn every_host_failure_has_its_own_code() {
        let cases = [
            HostFailure::Arch {
                provider: "x86".to_string(),
                host: "x64".to_string(),
            },
            HostFailure::Spawn("nope".to_string()),
            HostFailure::Timeout {
                waited_ms: 1,
                contained: false,
                job_error: 5,
            },
            HostFailure::Crashed {
                exit_code: Some(3),
                log_tail: String::new(),
                contained: true,
                job_error: 0,
                crash_note: None,
            },
            HostFailure::Protocol("x".to_string()),
            HostFailure::Io("x".to_string()),
        ];
        let mut codes: Vec<&str> = cases.iter().map(|c| c.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            cases.len(),
            "每类失败必须有独立错误码（12 §5）"
        );
        for c in &cases {
            assert!(!c.title().is_empty(), "{c:?} 的标题不能是空的");
        }
    }
}
