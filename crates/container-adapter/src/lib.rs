//! MKVToolNix 外部适配（18_CONTAINER_HELPER §1/§4）。
//!
//! 三条纪律决定了这个文件的形状：
//!
//! 1. **不重造 Matroska muxer**（18 §1）。identify 读 `mkvmerge --identify -J` 的 JSON，
//!    提取用 `mkvextract`，重封装用 `mkvmerge --output`。本模块不含任何 EBML 解析。
//! 2. **绝不原地修改源文件**（18 §4）。所有写操作先落到临时文件，成功后再原子 rename；
//!    源路径在整条链路上只以只读身份出现。输出路径等于源路径时直接拒绝。
//! 3. **元数据必须保持**（AUDIT-A023 的验收）。重封装后重新 identify 一次，逐轨核对
//!    type/codec/language/name/default/forced、附件清单与章节数；对不上不算成功。

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use core_model::error::{Domain, Result, StkError};
use serde::{Deserialize, Serialize};

use win_job as job;

/// 外部工具集：三个 exe 都必须来自同一个安装目录（版本一致性由调用方核对）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tools {
    pub mkvmerge: PathBuf,
    pub mkvextract: PathBuf,
    pub mkvinfo: Option<PathBuf>,
    pub timeout: Duration,
}

impl Tools {
    pub fn new(mkvmerge: impl Into<PathBuf>, mkvextract: impl Into<PathBuf>) -> Tools {
        Tools {
            mkvmerge: mkvmerge.into(),
            mkvextract: mkvextract.into(),
            mkvinfo: None,
            timeout: Duration::from_secs(300),
        }
    }

    /// 按 21 §9 记录工具身份：路径 + 内容 SHA-256。报告里必须能说清"是哪个二进制干的活"。
    pub fn identity(&self) -> Result<ToolIdentity> {
        Ok(ToolIdentity {
            mkvmerge: binary_identity(&self.mkvmerge)?,
            mkvextract: binary_identity(&self.mkvextract)?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryIdentity {
    pub path: String,
    pub size: u64,
    pub digest_sha256: String,
}

impl BinaryIdentity {
    pub fn display(&self) -> String {
        format!(
            "{}（{} 字节，sha256:{}…）",
            self.path,
            self.size,
            &self.digest_sha256[..16.min(self.digest_sha256.len())]
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolIdentity {
    pub mkvmerge: BinaryIdentity,
    pub mkvextract: BinaryIdentity,
}

fn binary_identity(exe: &Path) -> Result<BinaryIdentity> {
    let bytes =
        std::fs::read(exe).map_err(|e| StkError::from(e).with_detail(exe.display().to_string()))?;
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(BinaryIdentity {
        path: exe.display().to_string(),
        size: bytes.len() as u64,
        digest_sha256: hex,
    })
}

// ── identify ─────────────────────────────────────────────────────────────────

/// `mkvmerge --identify -J` 的子集（18 §2 需要的字段都在这几个里）。
///
/// 只声明用得到的字段：MKVToolNix 的 JSON 还在加字段，把整份结构镜像下来
/// 会让"上游加个键"变成我们这边的解析失败。
///
/// 真实输出的开关（mkvmerge v102.0 实测）：
/// - `chapters` 是一组章节对象，**不是数字**；空文件是 `[]`。章节数要自己数。
/// - 顶层还有 `errors`/`warnings`/`global_tags`/`track_tags`/`identification_format_version`。
/// - 每个 track 只有 `codec`/`id`/`properties`/`type` 四个键。
#[derive(Clone, Debug, Deserialize)]
pub struct Identify {
    #[serde(default)]
    pub container: Container,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// 章节列表；`.len()` 就是章节数
    #[serde(default)]
    pub chapters: Vec<serde_json::Value>,
    #[serde(default)]
    pub file_name: String,
    /// mkvmerge 自己报的错误（如"某轨解码器缺失"）；它不影响识别结果，但必须暴露出去
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Container {
    #[serde(default)]
    pub recognized: bool,
    #[serde(default)]
    pub supported: bool,
    /// "Matroska" / "MPEG-4" 等
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub properties: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Track {
    /// mkvmerge 的轨道 id（1 起，重封装时用它选轨）
    pub id: u64,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub properties: TrackProperties,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct TrackProperties {
    /// IETF BCP 47（如 "zh-Hans"）；旧文件可能只有 ISO 639-2
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub language_ietf: Option<String>,
    #[serde(default)]
    pub track_name: Option<String>,
    #[serde(default)]
    pub default_track: bool,
    #[serde(default)]
    pub forced_track: bool,
    #[serde(default)]
    pub number: Option<u64>,
    #[serde(default)]
    pub codec_id: Option<String>,
    #[serde(default)]
    pub enabled_track: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Attachment {
    pub id: u64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size: u64,
}

impl Identify {
    /// 字幕轨（ASS/SRT/PGS 都算），18 §2 的 "select subtitle track"。
    pub fn subtitle_tracks(&self) -> Vec<&Track> {
        self.tracks
            .iter()
            .filter(|t| t.kind == "subtitles")
            .collect()
    }

    /// 按 mkvmerge id 找轨道。
    pub fn track(&self, id: u64) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// 附件里看起来是字体的那些（18 §2 的 "attach subset fonts" 要能识别既有附件）。
    pub fn font_attachments(&self) -> Vec<&Attachment> {
        self.attachments
            .iter()
            .filter(|a| is_font_name(&a.file_name))
            .collect()
    }

    /// 章节数。真实 JSON 里 `chapters` 是对象列表，没有单独的计数字段。
    pub fn chapter_count(&self) -> usize {
        self.chapters.len()
    }
}

/// 字体附件的后缀集合（17 §2 的 FONT_EXTS 在 container 侧的最小镜像）。
fn is_font_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    const EXTS: [&str; 6] = [".ttf", ".otf", ".ttc", ".otc", ".woff", ".woff2"];
    EXTS.iter().any(|e| lower.ends_with(e))
}

// ── 进程调用 ─────────────────────────────────────────────────────────────────

/// 子进程失败的分类（与 media-probe 同构：超时 / 未约束 / 非零 / 输出超限）。
#[derive(Debug)]
pub enum RunError {
    NotFound(PathBuf),
    Timeout { contained: bool, job_error: u32 },
    NonZero { code: i32, stderr: String },
    OutputTooLarge { limit: usize },
}

impl RunError {
    fn to_stk(&self, what: &str) -> StkError {
        self.to_stk_with_timeout(what, None)
    }

    fn to_stk_with_timeout(&self, what: &str, timeout: Option<Duration>) -> StkError {
        match self {
            RunError::NotFound(exe) => StkError::simple(
                Domain::Dependency,
                30,
                &format!("未找到 {what}，MKV 助手不可用"),
            )
            .with_detail(exe.display().to_string())
            .with_action("安装 MKVToolNix 并用 --mkvmerge/--mkvextract 指定路径，或设 STK_MKVMERGE/STK_MKVEXTRACT"),
            RunError::Timeout { contained, job_error } => {
                let mut e = StkError::simple(Domain::Dependency, 31, &format!("{what} 超时"));
                if let Some(t) = timeout {
                    e = e.with_detail(format!("超过 {t:?} 未退出"));
                }
                if !contained {
                    e = e.with_detail(format!(
                        "子进程树未能被 Job Object 约束（Win32 错误 {job_error}：{}）；只终止了直接子进程",
                        job::error_name(*job_error)
                    ));
                }
                e.with_action("核对输入文件与磁盘状态；大文件可提高超时或分批处理")
            }
            RunError::NonZero { code, stderr } => {
                let mut e = StkError::simple(Domain::Dependency, 32, &format!("{what} 非零退出"));
                if !stderr.is_empty() {
                    e = e.with_detail(stderr.clone());
                }
                e.with_detail(format!("exit={code}"))
            }
            RunError::OutputTooLarge { limit } => StkError::simple(
                Domain::Dependency,
                33,
                &format!("{what} 输出超过安全上限"),
            )
            .with_detail(format!("上限 {limit} 字节")),
        }
    }
}

/// 跑一次外部工具并收集 stdout（参数是数组，永不拼 shell 字符串，23 §外部进程）。
fn run_capture(
    exe: &Path,
    args: &[String],
    timeout: Duration,
    stdout_limit: usize,
) -> std::result::Result<String, RunError> {
    let child: Child = Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| RunError::NotFound(exe.to_path_buf()))?;
    collect(child, timeout, stdout_limit)
}

/// 等子进程、并发排空两条管道、超时则连整棵树一起收掉。
///
/// 为什么 stdout/stderr 必须**一边跑一边读**：匿名管道缓冲只有几 KB，
/// 子进程写满就阻塞不退出，而我们若先 wait 再读就永远等不到（ISSUES D10）。
/// mkvinfo/mkvmerge 都是话多的进程。
fn collect(
    mut child: Child,
    timeout: Duration,
    stdout_limit: usize,
) -> std::result::Result<String, RunError> {
    let started = Instant::now();
    let job = job::attach(child.id());
    let stdout = child.stdout.take().ok_or(RunError::OutputTooLarge {
        limit: stdout_limit,
    })?;
    let stderr = child.stderr.take().ok_or(RunError::OutputTooLarge {
        limit: stdout_limit,
    })?;
    let (etx, erx) = mpsc::channel::<(String, bool)>();
    let err_thread = std::thread::spawn(move || {
        let _ = etx.send(read_bounded(stderr, 64 * 1024, true));
    });
    let (tx, rx) = mpsc::channel::<(String, bool)>();
    let out_thread = std::thread::spawn(move || {
        let _ = tx.send(read_bounded(stdout, stdout_limit, false));
    });

    let mut out_result = None;
    let mut err_result = None;
    let mut status = None;
    loop {
        if out_result.is_none() {
            out_result = rx.try_recv().ok();
        }
        if err_result.is_none() {
            err_result = erx.try_recv().ok();
        }
        if status.is_none() {
            status = child.try_wait().ok().flatten();
        }
        if out_result.is_some() && err_result.is_some() && status.is_some() {
            break;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let contained = job.is_ok();
            let job_error = job.as_ref().err().copied().unwrap_or(0);
            drop(job);
            if out_result.is_some() || rx.recv_timeout(Duration::from_millis(200)).is_ok() {
                let _ = out_thread.join();
            }
            if err_result.is_some() || erx.recv_timeout(Duration::from_millis(200)).is_ok() {
                let _ = err_thread.join();
            }
            return Err(RunError::Timeout {
                contained,
                job_error,
            });
        }
        std::thread::sleep(
            Duration::from_millis(10).min(timeout.saturating_sub(started.elapsed())),
        );
    }
    let (text, out_exceeded) = out_result.expect("循环已确认 stdout 读取完成");
    let (mut err, err_exceeded) = err_result.expect("循环已确认 stderr 读取完成");
    let status = status.expect("循环已确认子进程退出");
    let _ = out_thread.join();
    let _ = err_thread.join();
    if out_exceeded {
        return Err(RunError::OutputTooLarge {
            limit: stdout_limit,
        });
    }
    if !status.success() {
        if err_exceeded {
            err.insert_str(0, "[stderr 前段已截断；保留末尾 64 KiB] ");
        }
        // mkvmerge 把主要错误写到 **stdout**（stderr 常常是空的），所以失败时
        // 两份输出都要留。只取 stderr 会得到"exit=2"加一片空白（本机实测）。
        let mut detail = String::new();
        if !err.trim().is_empty() {
            detail.push_str(err.trim());
            detail.push_str(" | ");
        }
        let tail: String = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        if !tail.is_empty() {
            detail.push_str(&tail);
        }
        return Err(RunError::NonZero {
            code: status.code().unwrap_or(-1),
            stderr: detail,
        });
    }
    Ok(text)
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

/// 重封装用的固定参数（AUDIT-A023 要求把 argv 固定下来，不是随手拼）。
///
/// 逐个交代为什么是这个写法：
/// - `--no-date`：写进文件的 "Date" 段会让同一份输入的两次运行产生不同字节，
///   破坏"确定性产物"这条本仓库反复强调的性质；
/// - `--disable-track-statistics-tags`：老版本 mkvmerge 会在重封装时加统计标签，
///   它不是源文件的一部分，属于"除目标变化外元数据保持"之外的意外改动；
/// - 轨道选择用 `--audio-tracks`/`--video-tracks` 等**显式 id 列表**，
///   而不是 `!` 排除法：显式列表在源文件有意外轨道时更不容易漏掉新东西；
/// - **空集合用 `--no-subtitles` 这类显式开关**，不写 `*`。
///   mkvmerge v102.0 把 `*` 当 IETF 语言标签解析，`--subtitle-tracks *`
///   直接报"不是有效的 BCP 47 标签"并以 exit=2 失败（本机实测）。
pub fn remux_args(
    source: &Path,
    output: &Path,
    keep_subtitle_tracks: &[u64],
    keep_video_tracks: &[u64],
    keep_audio_tracks: &[u64],
    extra: &[String],
) -> Vec<String> {
    let mut args = vec![
        "--output".to_string(),
        output.display().to_string(),
        "--no-date".to_string(),
        "--disable-track-statistics-tags".to_string(),
    ];
    push_track_selection(&mut args, "subtitle", keep_subtitle_tracks);
    push_track_selection(&mut args, "video", keep_video_tracks);
    push_track_selection(&mut args, "audio", keep_audio_tracks);
    args.extend(extra.iter().cloned());
    args.push(source.display().to_string());
    args
}

/// 一个轨道类别：非空就给显式 id 列表，空就给 `--no-<kind>` 开关。
///
/// **选项名与取值是两个 argv 元素**，不能拼成 `"--subtitle-tracks 2,3"` 一个串：
/// `Command::args` 不做空格分词，mkvmerge 会把整串当成一个未知选项而报错。
///
/// 注意拼写：列表选项是 `--subtitle-tracks`（单数 subtitle），
/// 但"全丢"开关是 `--no-subtitles`（复数）。拿 kind 拼字符串会送出
/// `--subtitles-tracks` 这种不存在的选项，而 mkvmerge 会把它当成一个待打开的文件名
/// ——错误信息完全指不到真正的毛病（本机实测）。所以两种拼法都明确写出来。
fn push_track_selection(args: &mut Vec<String>, kind: &str, ids: &[u64]) {
    let (list_flag, none_flag) = match kind {
        "subtitle" => ("--subtitle-tracks", "--no-subtitles"),
        "video" => ("--video-tracks", "--no-video"),
        "audio" => ("--audio-tracks", "--no-audio"),
        other => unreachable!("未知轨道类别：{other}"),
    };
    if ids.is_empty() {
        args.push(none_flag.to_string());
    } else {
        args.push(list_flag.to_string());
        args.push(id_list(ids));
    }
}

fn id_list(ids: &[u64]) -> String {
    ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

// ── 对外操作 ─────────────────────────────────────────────────────────────────

/// identify 一个 MKV（18 §2 第一条）。
pub fn identify(tools: &Tools, mkv: &Path) -> Result<Identify> {
    let args = vec![
        "--identify".to_string(),
        "-J".to_string(),
        mkv.display().to_string(),
    ];
    let text = run_capture(&tools.mkvmerge, &args, tools.timeout, 32 * 1024 * 1024)
        .map_err(|e| e.to_stk("mkvmerge --identify"))?;
    parse_identify(&text).map_err(|e| {
        e.with_detail(format!("输入：{}", mkv.display()))
            .with_action("确认输入是 MKVToolNix 能识别的容器文件")
    })
}

/// 解析 identify JSON。单独抽出来是为了能对固定样本做测试（不依赖本机 exe）。
pub fn parse_identify(text: &str) -> Result<Identify> {
    let id: Identify = serde_json::from_str(text).map_err(|e| {
        StkError::simple(
            Domain::Dependency,
            34,
            "mkvmerge identify 输出不是预期 JSON",
        )
        .with_detail(e.to_string())
    })?;
    if !id.container.recognized {
        return Err(
            StkError::simple(Domain::Dependency, 35, "mkvmerge 不认识的容器")
                .with_detail(format!("container.type={:?}", id.container.kind))
                .with_action("确认输入是 Matroska/MKV 文件；其它容器请先转换"),
        );
    }
    Ok(id)
}

/// 提取轨道到目标路径（`mkvextract tracks <src> <id>:<out>`）。
pub fn extract_track(tools: &Tools, mkv: &Path, track_id: u64, out: &Path) -> Result<()> {
    guard_output_not_source(mkv, out)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| StkError::from(e).with_detail(parent.display().to_string()))?;
    }
    let args = vec![
        "tracks".to_string(),
        mkv.display().to_string(),
        format!("{}:{}", track_id, out.display()),
    ];
    run_capture(&tools.mkvextract, &args, tools.timeout, 8 * 1024 * 1024)
        .map_err(|e| e.to_stk("mkvextract tracks"))?;
    Ok(())
}

/// 提取附件（`mkvextract attachments <src> <id>:<out>`）。18 §2 的 "extract fonts"。
pub fn extract_attachment(tools: &Tools, mkv: &Path, att_id: u64, out: &Path) -> Result<()> {
    guard_output_not_source(mkv, out)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| StkError::from(e).with_detail(parent.display().to_string()))?;
    }
    let args = vec![
        "attachments".to_string(),
        mkv.display().to_string(),
        format!("{}:{}", att_id, out.display()),
    ];
    run_capture(&tools.mkvextract, &args, tools.timeout, 8 * 1024 * 1024)
        .map_err(|e| e.to_stk("mkvextract attachments"))?;
    Ok(())
}

/// 重封装成**新文件**，并在写好后重新 identify 核对元数据保持（18 §4、AUDIT-A023）。
///
/// 返回新文件的 identify 结果，供调用方/报告使用。
pub fn remux(
    tools: &Tools,
    source: &Path,
    output: &Path,
    keep_subtitle_tracks: &[u64],
    keep_video_tracks: &[u64],
    keep_audio_tracks: &[u64],
    attach_fonts: &[PathBuf],
) -> Result<RemuxOutcome> {
    remux_with(
        tools,
        source,
        output,
        keep_subtitle_tracks,
        keep_video_tracks,
        keep_audio_tracks,
        attach_fonts,
        &[],
    )
}

/// `remux` 的完整签名：`extra` 是调用方追加的 mkvmerge 选项（轨道名称/语言/default/forced）。
#[allow(clippy::too_many_arguments)]
pub fn remux_with(
    tools: &Tools,
    source: &Path,
    output: &Path,
    keep_subtitle_tracks: &[u64],
    keep_video_tracks: &[u64],
    keep_audio_tracks: &[u64],
    attach_fonts: &[PathBuf],
    extra_track_options: &[String],
) -> Result<RemuxOutcome> {
    guard_output_not_source(source, output)?;

    // 先记下源文件的元数据事实，写完再比
    let before = identify(tools, source)?;

    let mut extra: Vec<String> = extra_track_options.to_vec();
    // 附件去重：源文件里已有的同名同内容附件直接复用，不再 --attach-file 一遍。
    // 不去重的话 mkvmerge 会忠实地把同名附件再加一份（本机实测：源里 1 份 deck.ttf，
    // 再附一次就变成 2 份），播放器里出现两个同名字体，而这几乎不会是用户想要的。
    let mut attach_now: Vec<&PathBuf> = Vec::new();
    for font in attach_fonts {
        if !font.is_file() {
            return Err(
                StkError::simple(Domain::Dependency, 36, "要附加的字体文件不存在")
                    .with_detail(font.display().to_string()),
            );
        }
        if !already_attached(&before, font) {
            attach_now.push(font);
        }
    }
    for font in &attach_now {
        extra.push("--attach-file".to_string());
        extra.push(font.display().to_string());
    }

    // 写临时文件再改名：半成品不会以最终名字存在（与 task-engine 的输出事务同一条纪律）
    let staging = staging_path(output);
    let staged_args = remux_args(
        source,
        &staging,
        keep_subtitle_tracks,
        keep_video_tracks,
        keep_audio_tracks,
        &extra,
    );

    let run = run_capture(
        &tools.mkvmerge,
        &staged_args,
        tools.timeout,
        32 * 1024 * 1024,
    );
    if let Err(e) = run {
        let _ = std::fs::remove_file(&staging);
        return Err(e.to_stk_with_timeout("mkvmerge 重封装", Some(tools.timeout)));
    }

    // 交付前验证：新文件必须能被 identify 且确实是 MKV
    let after = match identify(tools, &staging) {
        Ok(a) => a,
        Err(e) => {
            let _ = std::fs::remove_file(&staging);
            return Err(e.with_action("重封装产物无法被识别；已删除半成品，源文件未被改动"));
        }
    };

    // 元数据保持核对（AUDIT-A023 的验收门）。
    // 调用方主动要求改的那几项不算漂移（TrackEdits 从 extra 选项里解出来）。
    let requested = TrackEdits::from_mkvmerge_options(extra_track_options);
    let drift = compare_metadata(
        &before,
        &after,
        keep_subtitle_tracks,
        attach_now.len(),
        &requested,
    );
    if !drift.is_empty() {
        let _ = std::fs::remove_file(&staging);
        return Err(
            StkError::simple(Domain::Dependency, 37, "重封装后元数据与源文件不一致")
                .with_detail(drift.join("; "))
                .with_action("这属于适配器缺陷；已删除半成品，源文件未被改动"),
        );
    }

    std::fs::rename(&staging, output).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        StkError::from(e)
            .with_detail(output.display().to_string())
            .with_action("目标路径可能被占用或跨卷；换一个输出位置重试")
    })?;

    // 各类型的保留/丢弃计数：报告要说清"我到底拿走了什么"，而不是只给一个总数
    let ids_of = |id: &Identify, kind: &str| -> Vec<u64> {
        id.tracks
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.id)
            .collect()
    };
    let kept_video = ids_of(&after, "video");
    let kept_audio = ids_of(&after, "audio");
    let kept_subtitles = ids_of(&after, "subtitles");
    let dropped = |all: Vec<u64>, kept: &[u64]| -> Vec<u64> {
        all.into_iter().filter(|id| !kept.contains(id)).collect()
    };
    let dropped_video = dropped(ids_of(&before, "video"), &kept_video);
    let dropped_audio = dropped(ids_of(&before, "audio"), &kept_audio);
    let dropped_subtitles = dropped(ids_of(&before, "subtitles"), &kept_subtitles);

    Ok(RemuxOutcome {
        output: output.to_path_buf(),
        before,
        after,
        attached_fonts: attach_now.len(),
        skipped_fonts: attach_fonts
            .iter()
            .filter(|f| !attach_now.contains(f))
            .map(|f| f.display().to_string())
            .collect(),
        kept_video,
        kept_audio,
        kept_subtitles,
        dropped_video,
        dropped_audio,
        dropped_subtitles,
    })
}

#[derive(Clone, Debug)]
pub struct RemuxOutcome {
    pub output: PathBuf,
    /// 源文件事实（重封装前）
    pub before: Identify,
    /// 产物事实（重封装后，已逐轨核对过）
    pub after: Identify,
    pub attached_fonts: usize,
    /// 因为"源文件里已经有同名同大小附件"而被跳过的字体（不报告就等于静默少做一件事）
    pub skipped_fonts: Vec<String>,
    /// 产物里保留的轨道 id（按类型分开，报告要用）
    pub kept_video: Vec<u64>,
    pub kept_audio: Vec<u64>,
    pub kept_subtitles: Vec<u64>,
    /// 源文件里被丢弃的轨道 id
    pub dropped_video: Vec<u64>,
    pub dropped_audio: Vec<u64>,
    pub dropped_subtitles: Vec<u64>,
}

/// 半成品路径：与输出同目录（同卷才能原子改名），带 `.stk-tmp` 后缀。
fn staging_path(output: &Path) -> PathBuf {
    let mut name = output
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output.mkv".to_string());
    name.push_str(".stk-tmp.mkv");
    output.with_file_name(name)
}

/// 输出路径与源路径同一文件就拒绝（18 §4：绝不原地修改）。
///
/// 用 canonicalize 比字符串更可靠：`D:\a.mkv` 与 `D:\dir\..\a.mkv` 是同一个文件。
fn guard_output_not_source(source: &Path, output: &Path) -> Result<()> {
    let same = match (source.canonicalize(), output.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        // 输出还不存在时，比较规范化后的父目录 + 文件名
        _ => {
            let sp = source
                .canonicalize()
                .unwrap_or_else(|_| source.to_path_buf());
            let op = match (output.parent(), output.file_name()) {
                (Some(p), Some(f)) => p
                    .canonicalize()
                    .map(|c| c.join(f))
                    .unwrap_or_else(|_| output.to_path_buf()),
                _ => output.to_path_buf(),
            };
            sp == op
        }
    };
    if same {
        return Err(
            StkError::simple(Domain::Dependency, 38, "输出路径与源文件是同一个文件")
                .with_detail(output.display().to_string())
                .with_action("18 §4 要求绝不原地修改 MKV；请给一个不同的输出路径"),
        );
    }
    Ok(())
}

/// 逐轨核对：除被明确排除或**被明确要求修改**的轨道外，
/// 类型/编码/语言/名称/default/forced 必须一致；
/// 附件数量必须等于「源附件 + 新附加的字体」；章节数必须相等。
///
/// `requested` 是调用方主动施加的轨道修改（18 §2 的 language/name/default/forced）。
/// 不把它排掉的话，"按要求改了名字"会被当成"元数据漂移"——
/// 那会让完全正确的操作失败（AT-MKV-001 实测就撞在这里）。
fn compare_metadata(
    before: &Identify,
    after: &Identify,
    kept_subtitle: &[u64],
    actually_attached: usize,
    requested: &TrackEdits,
) -> Vec<String> {
    let mut drift = Vec::new();

    for t in &before.tracks {
        let should_keep = if t.kind == "subtitles" {
            kept_subtitle.contains(&t.id)
        } else {
            true
        };
        if !should_keep {
            continue;
        }
        match after.track(t.id) {
            None => drift.push(format!("轨道 {} 在产物里丢失", t.id)),
            Some(a) => {
                if t.kind != a.kind {
                    drift.push(format!("轨道 {} 类型 {} -> {}", t.id, t.kind, a.kind));
                }
                if t.codec != a.codec {
                    drift.push(format!("轨道 {} 编码 {} -> {}", t.id, t.codec, a.codec));
                }
                // 被请求改过的那一项不比对：它不是"漂移"，是"按要求改了"。
                // language 与 language_ietf 是同一件事的两种写法：
                // 只要轨道在 `requested.language` 里，两项都不算漂移。
                if !requested.language.contains(&t.id) {
                    if t.properties.language != a.properties.language {
                        drift.push(format!(
                            "轨道 {} language {:?} -> {:?}",
                            t.id, t.properties.language, a.properties.language
                        ));
                    }
                    if t.properties.language_ietf != a.properties.language_ietf {
                        drift.push(format!(
                            "轨道 {} language_ietf {:?} -> {:?}",
                            t.id, t.properties.language_ietf, a.properties.language_ietf
                        ));
                    }
                }
                if !requested.name.contains(&t.id)
                    && t.properties.track_name != a.properties.track_name
                {
                    drift.push(format!(
                        "轨道 {} track_name {:?} -> {:?}",
                        t.id, t.properties.track_name, a.properties.track_name
                    ));
                }
                if !requested.default.contains(&t.id)
                    && t.properties.default_track != a.properties.default_track
                {
                    drift.push(format!(
                        "轨道 {} default {} -> {}",
                        t.id, t.properties.default_track, a.properties.default_track
                    ));
                }
                if !requested.forced.contains(&t.id)
                    && t.properties.forced_track != a.properties.forced_track
                {
                    drift.push(format!(
                        "轨道 {} forced {} -> {}",
                        t.id, t.properties.forced_track, a.properties.forced_track
                    ));
                }
            }
        }
    }

    // 不该多出来的轨道：产物里的轨道必须在源里存在
    for t in &after.tracks {
        if before.track(t.id).is_none() {
            drift.push(format!("产物多出轨道 {}（源里没有）", t.id));
        }
    }

    let expected_attachments = before.attachments.len() + actually_attached;
    if after.attachments.len() != expected_attachments {
        drift.push(format!(
            "附件数 {} -> {}（期望 {}）",
            before.attachments.len(),
            after.attachments.len(),
            expected_attachments
        ));
    }
    if before.chapters.len() != after.chapters.len() {
        drift.push(format!(
            "章节数 {} -> {}",
            before.chapters.len(),
            after.chapters.len()
        ));
    }

    drift
}

/// 调用方主动施加的轨道修改（用于把"按要求改的"从"漂移"里排除）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackEdits {
    pub name: Vec<u64>,
    pub language: Vec<u64>,
    pub default: Vec<u64>,
    pub forced: Vec<u64>,
}

impl TrackEdits {
    /// 从 mkvmerge 的 `extra` 选项列表里解出被改动的轨道 id。
    ///
    /// 这个解析故意做得窄：只认我们知道会发的四种选项。
    /// 宽泛解析会把"恰好长得像"的取值当成轨道 id，
    /// 于是真漂移被静默放行（比误报严重得多）。
    pub fn from_mkvmerge_options(options: &[String]) -> TrackEdits {
        let mut e = TrackEdits::default();
        let mut i = 0usize;
        while i + 1 < options.len() {
            let flag = options[i].as_str();
            let value = options[i + 1].as_str();
            let id = value.split(':').next().and_then(|s| s.parse::<u64>().ok());
            if let Some(id) = id {
                match flag {
                    "--track-name" => e.name.push(id),
                    "--language" => e.language.push(id),
                    "--default-track" => e.default.push(id),
                    "--forced-track" => e.forced.push(id),
                    _ => {}
                }
            }
            i += 2;
        }
        e
    }

    /// 一个修改都没有。测试用它断言"不认识的选项不得被当成轨道修改"。
    pub fn is_empty_edits(&self) -> bool {
        self.name.is_empty()
            && self.language.is_empty()
            && self.default.is_empty()
            && self.forced.is_empty()
    }
}

/// 源文件里是否已经挂着同名同大小的附件。
///
/// 判据是「文件名 + 字节数」：mkvmerge 的 identify 只给这两个可用字段。
/// 不比内容哈希——那会要求在 identify 阶段就把每个附件抽出来读一遍，
/// 而容器适配器的工作原则是"调用 MKVToolNix，不自己拆文件"（18 §1）。
/// 同名同长已经足以让"本来就是同一份字体"不再重复附加；
/// 真要判同内容，调用方应该先自己比哈希再决定传不传 --attach-font。
fn already_attached(before: &Identify, font: &Path) -> bool {
    let name = font
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = std::fs::metadata(font).map(|m| m.len()).unwrap_or(u64::MAX);
    before
        .attachments
        .iter()
        .any(|a| a.file_name == name && a.size == size)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "container": { "recognized": true, "supported": true, "type": "Matroska",
        "properties": { "title": "Test", "duration": "PT1M0S" } },
      "tracks": [
        { "id": 0, "type": "video", "codec": "AVC/H.264/MPEG-4p10",
          "properties": { "default_track": true, "language": "und" } },
        { "id": 1, "type": "audio", "codec": "AAC",
          "properties": { "default_track": true, "language": "jpn" } },
        { "id": 2, "type": "subtitles", "codec": "SubStationAlpha",
          "properties": { "default_track": false, "forced_track": false,
                          "language": "zh", "language_ietf": "zh-Hans",
                          "track_name": "简中特效" } },
        { "id": 3, "type": "subtitles", "codec": "HDMV PGS",
          "properties": { "default_track": false, "forced_track": true,
                          "language": "eng", "track_name": "English" } }
      ],
      "attachments": [
        { "id": 1, "file_name": "SourceHanSans.ttf", "description": "",
          "content_type": "application/x-truetype-font", "size": 1234 }
      ],
      "chapters": [{}, {}, {}],
      "file_name": "movie.mkv"
    }"#;

    /// 真实 mkvmerge v102.0 的 identify 输出：`chapters` 是列表不是数字，
    /// 顶层有 `errors`/`warnings`。这一段是从实际二进制粘回来的，
    /// 之前用虚构样本时这里的字段类型全串了（本机实测才发现）。
    const REAL_V102: &str = r#"{
      "attachments": [{ "content_type": "font/ttf", "description": "",
        "file_name": "deck.ttf", "id": 1,
        "properties": { "uid": 10855217905146017453 }, "size": 1045720 }],
      "chapters": [],
      "container": { "properties": { "container_type": 17,
        "duration": 9166167000000, "timestamp_scale": 1000000,
        "writing_application": "mkvmerge v102.0 ('Little Houses') 64-bit" },
        "recognized": true, "supported": true, "type": "Matroska" },
      "errors": [], "file_name": "source.mkv",
      "global_tags": [], "identification_format_version": 20,
      "track_tags": [],
      "tracks": [
        { "codec": "AVC/H.264/MPEG-4p10", "id": 0,
          "properties": { "codec_id": "V_MPEG4/ISO/AVC", "default_track": true,
            "language": "eng", "language_ietf": "en", "track_name": "Main Video" },
          "type": "video" },
        { "codec": "AAC", "id": 1,
          "properties": { "codec_id": "A_AAC", "default_track": true,
            "language": "jpn", "language_ietf": "ja", "track_name": "Japanese Audio" },
          "type": "audio" },
        { "codec": "SubRip/SRT", "id": 2,
          "properties": { "codec_id": "S_TEXT/UTF8", "default_track": true,
            "forced_track": false, "language": "eng", "language_ietf": "en",
            "track_name": "English Subs", "text_subtitles": true },
          "type": "subtitles" },
        { "codec": "HDMV PGS", "id": 3,
          "properties": { "codec_id": "S_HDMV/PGS", "default_track": false,
            "forced_track": false, "language": "zho", "language_ietf": "zh",
            "track_name": "Chinese PGS" },
          "type": "subtitles" }
      ],
      "warnings": []
    }"#;

    #[test]
    fn real_mkvmerge_v102_identify_output_parses() {
        let id = parse_identify(REAL_V102).unwrap();
        assert!(id.container.recognized);
        assert_eq!(id.container.kind, "Matroska");
        assert_eq!(id.tracks.len(), 4);
        assert_eq!(id.subtitle_tracks().len(), 2);
        assert_eq!(
            id.subtitle_tracks()
                .iter()
                .map(|t| t.id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(id.chapter_count(), 0, "空章节是 [] 不是 0");
        assert_eq!(id.attachments.len(), 1);
        assert_eq!(id.font_attachments().len(), 1, "ttf 必须被认成字体附件");
        assert!(id.errors.is_empty() && id.warnings.is_empty());
        let pgs = id.track(3).unwrap();
        assert_eq!(pgs.codec, "HDMV PGS");
        assert_eq!(pgs.properties.language.as_deref(), Some("zho"));
        assert_eq!(pgs.properties.track_name.as_deref(), Some("Chinese PGS"));
    }

    #[test]
    fn identify_json_is_parsed_with_tracks_and_attachments() {
        let id = parse_identify(SAMPLE).unwrap();
        assert!(id.container.recognized);
        assert_eq!(id.container.kind, "Matroska");
        assert_eq!(id.tracks.len(), 4);
        assert_eq!(id.subtitle_tracks().len(), 2);
        assert_eq!(id.subtitle_tracks()[0].id, 2);
        assert_eq!(
            id.track(2).unwrap().properties.language_ietf.as_deref(),
            Some("zh-Hans")
        );
        assert!(id.track(3).unwrap().properties.forced_track);
        assert_eq!(id.attachments.len(), 1);
        assert_eq!(id.font_attachments().len(), 1);
        assert_eq!(id.font_attachments()[0].file_name, "SourceHanSans.ttf");
        assert_eq!(id.chapter_count(), 3, "章节数是列表长度，不是单独字段");
    }

    #[test]
    fn unrecognized_container_is_rejected_with_action() {
        let e = parse_identify(
            r#"{"container":{"recognized":false,"supported":false,"type":"MPEG-4"}}"#,
        )
        .unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0035");
        assert!(e.user_action.is_some(), "拒绝要给下一步：{e:?}");
    }

    #[test]
    fn malformed_json_reports_dependency_error_not_panic() {
        let e = parse_identify("{not json").unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0034");
    }

    #[test]
    fn remux_args_are_a_fixed_array_with_determinism_flags() {
        let args = remux_args(
            Path::new("D:\\in.mkv"),
            Path::new("D:\\out.mkv"),
            &[2],
            &[0],
            &[1],
            &["--attach-file".to_string(), "f.ttf".to_string()],
        );
        // 源必须是最后一个参数；前面是选项
        assert_eq!(args.last().map(String::as_str), Some("D:\\in.mkv"));
        assert!(args.iter().any(|a| a == "--no-date"));
        assert!(args.iter().any(|a| a == "--disable-track-statistics-tags"));
        // 选项名与取值是两个元素：拼成 "--subtitle-tracks 2" 一个串时
        // Command::args 不会分词，mkvmerge 直接 exit=2（本机实测）
        let idx = args
            .iter()
            .position(|a| a == "--subtitle-tracks")
            .expect("--subtitle-tracks 必须是独立元素");
        assert_eq!(args[idx + 1], "2");
        assert!(args.iter().any(|a| a == "--video-tracks"));
        assert!(args.iter().any(|a| a == "--audio-tracks"));
        assert!(args.iter().any(|a| a == "--attach-file"));
        // 任何一个 argv 元素都不该带空格：带空格就说明又把选项和取值拼起来了
        assert!(
            args.iter().all(|a| !a.contains(' ')),
            "argv 元素不得含空格分隔的选项+取值：{args:?}"
        );
        // 单/复数不能搞混：--subtitles-tracks 不存在，mkvmerge 会当文件名
        assert!(
            args.iter().all(|a| !a.starts_with("--subtitles-tracks")),
            "{args:?}"
        );
        // 不得出现 shell 连接符
        assert!(args.iter().all(|a| !a.contains('|') && !a.contains('&')));
    }

    #[test]
    fn empty_track_selection_uses_the_explicit_no_flag_not_a_star() {
        let args = remux_args(Path::new("i.mkv"), Path::new("o.mkv"), &[], &[0], &[1], &[]);
        assert!(
            args.iter().any(|a| a == "--no-subtitles"),
            "空字幕集合要用 --no-subtitles：{args:?}"
        );
        // mkvmerge v102.0 把 `*` 当 IETF 语言标签，`--subtitle-tracks *` 会 exit=2
        assert!(
            !args.iter().any(|a| a.contains('*')),
            "不得出现 `*`：mkvmerge v102.0 会把它当语言标签：{args:?}"
        );
        assert!(args.iter().all(|a| a != "--subtitle-tracks"));
        // 非空集合仍然是显式 id 列表
        let with = remux_args(
            Path::new("i.mkv"),
            Path::new("o.mkv"),
            &[2, 3],
            &[0],
            &[1],
            &[],
        );
        let idx = with
            .iter()
            .position(|a| a == "--subtitle-tracks")
            .expect("非空集合用列表选项");
        assert_eq!(with[idx + 1], "2,3");
        assert!(!with.iter().any(|a| a == "--no-subtitles"));
        // 单数选项名才对
        assert!(
            with.iter().all(|a| !a.contains("--subtitles-tracks")),
            "{with:?}"
        );
    }

    #[test]
    fn writing_output_over_the_source_is_refused() {
        let dir = std::env::temp_dir().join(format!("stk-mkv-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("a.mkv");
        std::fs::write(&src, b"x").unwrap();
        // 同一个路径
        let e = guard_output_not_source(&src, &src).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0038");
        assert!(e.user_action.is_some());
        // 经过 .. 的等价路径
        let sneaky = dir.join("sub").join("..").join("a.mkv");
        assert!(
            guard_output_not_source(&src, &sneaky).is_err(),
            "规范化后是同一个文件，必须拒"
        );
        // 不同路径放行
        assert!(guard_output_not_source(&src, &dir.join("b.mkv")).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn metadata_drift_is_detected() {
        let before = parse_identify(SAMPLE).unwrap();
        let none = TrackEdits::default();
        // 把字幕轨的 language 改掉：这就是"元数据没保持"
        let mut drifted: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        drifted["tracks"][2]["properties"]["language"] =
            serde_json::Value::String("ja".to_string());
        let after = parse_identify(&drifted.to_string()).unwrap();
        let drift = compare_metadata(&before, &after, &[2, 3], 0, &none);
        assert!(drift.iter().any(|d| d.contains("language")), "{drift:?}");

        // 完全一致时不得报漂移
        assert!(compare_metadata(&before, &before, &[2, 3], 0, &none).is_empty());
        // 新附加一份字体：附件数应从 1 变 2 才算对
        let same = parse_identify(SAMPLE).unwrap();
        let drift2 = compare_metadata(&before, &same, &[2, 3], 1, &none);
        assert!(
            drift2.iter().any(|d| d.contains("附件数")),
            "附加字体后附件数没涨就该报：{drift2:?}"
        );
    }

    /// 主动改语言/名字/forced 是 18 §2 的正事，不能报成"漂移"。
    /// 本机实测（AT-MKV-001）就撞在这里：子集操作完全成功，却被自己的校验判失败。
    #[test]
    fn requested_track_edits_are_not_reported_as_drift() {
        let before = parse_identify(SAMPLE).unwrap();
        let mut edited: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        edited["tracks"][2]["properties"]["language"] =
            serde_json::Value::String("chi".to_string());
        edited["tracks"][2]["properties"]["language_ietf"] =
            serde_json::Value::String("zh-Hans".to_string());
        edited["tracks"][2]["properties"]["track_name"] =
            serde_json::Value::String("Processed ASS".to_string());
        edited["tracks"][3]["properties"]["forced_track"] = serde_json::Value::Bool(false);
        let after = parse_identify(&edited.to_string()).unwrap();

        // 没有请求过 -> 全部算漂移。
        // 注意：SAMPLE 里轨道 2 原本就是 language_ietf="zh-Hans"，
        // 这里只把 language 从 "zh" 改成 "chi"，所以 IETF 形式没变，
        // 共三项：language + track_name + forced。
        let none = TrackEdits::default();
        let unrequested = compare_metadata(&before, &after, &[2, 3], 0, &none);
        assert_eq!(unrequested.len(), 3, "{unrequested:?}");

        // 请求过 -> 不报
        let requested = TrackEdits {
            name: vec![2],
            language: vec![2],
            default: vec![],
            forced: vec![3],
        };
        assert!(
            compare_metadata(&before, &after, &[2, 3], 0, &requested).is_empty(),
            "被明确要求的修改不得报成漂移：{:?}",
            compare_metadata(&before, &after, &[2, 3], 0, &requested)
        );
    }

    /// 从 mkvmerge 选项里解轨道 id：只认我们真会发的那四个，解析错就等于放行漂移。
    #[test]
    fn track_edits_are_recovered_from_the_option_list() {
        let opts = vec![
            "--track-name".to_string(),
            "2:Processed ASS".to_string(),
            "--language".to_string(),
            "2:zho-Hans".to_string(),
            "--default-track".to_string(),
            "3:yes".to_string(),
            "--forced-track".to_string(),
            "3:no".to_string(),
            "--attach-file".to_string(),
            "f.ttf".to_string(),
        ];
        let e = TrackEdits::from_mkvmerge_options(&opts);
        assert_eq!(e.name, vec![2]);
        assert_eq!(e.language, vec![2]);
        assert_eq!(e.default, vec![3]);
        assert_eq!(e.forced, vec![3]);
        // 不认识的选项不得被当成轨道修改
        assert!(TrackEdits::from_mkvmerge_options(&[
            "--attach-file".to_string(),
            "12:x.ttf".to_string()
        ])
        .is_empty_edits());
    }

    #[test]
    fn dropped_subtitle_track_is_reported_as_loss() {
        let before = parse_identify(SAMPLE).unwrap();
        // 产物里只剩轨道 0/1（字幕被丢了）
        let mut dropped: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        dropped["tracks"].as_array_mut().unwrap().truncate(2);
        let after = parse_identify(&dropped.to_string()).unwrap();
        let drift = compare_metadata(&before, &after, &[2, 3], 0, &TrackEdits::default());
        assert!(
            drift.iter().any(|d| d.contains("丢失")),
            "要保留的字幕轨丢了必须报：{drift:?}"
        );
    }

    #[test]
    fn staging_path_sits_next_to_output_and_is_distinct() {
        let out = Path::new("D:\\dir\\movie.new.mkv");
        let s = staging_path(out);
        assert_eq!(s, PathBuf::from("D:\\dir\\movie.new.mkv.stk-tmp.mkv"));
        assert_ne!(s, out);
        assert_eq!(s.parent(), out.parent(), "必须同目录才能原子改名");
    }

    #[test]
    fn missing_binary_is_a_dependency_error_not_a_panic() {
        let tools = Tools::new("C:\\nope\\mkvmerge.exe", "C:\\nope\\mkvextract.exe");
        let e = identify(&tools, Path::new("X:\\nothing.mkv")).unwrap_err();
        assert_eq!(e.code_string(), "STK-DEP-0030");
        assert!(e.user_action.is_some());
    }
}
