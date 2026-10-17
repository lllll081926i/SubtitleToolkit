//! `batch`（15 §1/§5/§6/§7/§8）：输入集合 × 多个输出 profile，一次跑完并出报告。
//!
//! 四条决定性的设计，都是为了避免"看起来跑了其实没跑"：
//!
//! 1. **每个 profile 走的就是单命令那条路**（`run::execute`），不另写一套生产逻辑。
//!    20 §Repro 要求"GUI 复制 CLI 命令可以直接重放"；批量与单发如果各有一套实现，
//!    重放出来的东西就和批量时不一样。
//! 2. **失败不回滚别的 output**（14 §Multi-output）：任务级判 `PartialSuccess`，
//!    成功产物原样留着，报告逐个说清是谁在哪一步坏的。
//! 3. **恢复状态只记真的落过盘的东西**：dry-run 既不读也不写 `.jobstate.json`，
//!    否则下一次运行会"复用"一个从来没存在过的文件。
//! 4. **spec 是 JSON 不是 TOML**：`--preset` 要用的 TOML 依赖还没批（ISSUES E4），
//!    批量描述不能因此等下去。`serde_json` 已在依赖表里，字段一律
//!    `deny_unknown_fields`——写错 key 直接报错，而不是被静默忽略。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::geometry::{LayoutResPolicy, ScriptResolutionPolicy};
use core_model::profile::{FormatKind, ValidationLevel};
use core_model::time::Fps;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use task_engine::{
    CancelToken, CrashBudget, DiscoverOptions, InputSet, Job, JobOutcome, JobState, NameFields,
    NameTemplate, OutputProfile, OverwritePolicy, Pools, ProfileOutcome, RenderIdentity,
    StageRecord,
};

use crate::args::{Command, Invocation};
use crate::output::{Envelope, ErrorEntry, WriteRecord};
use crate::pipeline;
use crate::run;

fn cli_err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Input, code, msg)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecProfile {
    pub name: String,
    /// ass | ssa | srt | pgs | bdn
    pub format: String,
    #[serde(default = "default_canvas")]
    pub canvas: String,
    #[serde(default = "default_fps")]
    pub fps: String,
    /// pgs：true 时用 Basic 档，超过 255 色直接失败而不是量化（13 §4/§5）
    #[serde(default)]
    pub exact_colors: bool,
    /// bdn：小对象过滤阈值（像素），只过滤不钳制（13 §7）
    #[serde(default)]
    pub min_axis: u32,
    #[serde(default)]
    pub default_font: Option<String>,
}

fn default_canvas() -> String {
    "1920x1080".to_string()
}
fn default_fps() -> String {
    "25".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecOutput {
    /// 默认与 spec 同目录
    #[serde(default)]
    pub dir: Option<String>,
    /// 15 §5 的命名模板
    #[serde(default = "default_template")]
    pub template: String,
    /// 15 §6：skip | rename | replace
    #[serde(default)]
    pub overwrite: Option<String>,
}

fn default_template() -> String {
    "{stem}.{profile}.{ext}".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecRuntime {
    #[serde(default = "default_cpu")]
    pub cpu: usize,
    #[serde(default = "default_one")]
    pub render: usize,
    #[serde(default = "default_two")]
    pub disk: usize,
    #[serde(default = "default_one")]
    pub external: usize,
    /// 38 §3：只有"重跑不会伤到用户数据"才给崩溃预算，所以默认 false
    #[serde(default)]
    pub safe_to_restart: bool,
}

fn default_cpu() -> usize {
    2
}
fn default_one() -> usize {
    1
}
fn default_two() -> usize {
    2
}

impl Default for SpecRuntime {
    fn default() -> SpecRuntime {
        SpecRuntime {
            cpu: 2,
            render: 1,
            disk: 2,
            external: 1,
            safe_to_restart: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub id: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default = "default_true")]
    pub recursive: bool,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    pub profiles: Vec<SpecProfile>,
    #[serde(default)]
    pub output: SpecOutput,
    #[serde(default)]
    pub runtime: Option<SpecRuntime>,
    #[serde(default)]
    pub libass_dll: Option<String>,
    /// 15 §2 的同名视频关联是 optional 的
    #[serde(default = "default_true")]
    pub video_association: bool,
}

fn default_true() -> bool {
    true
}

/// 解析后的 profile：给执行器的部分（`OutputProfile`）+ 派发单命令需要的信息。
#[derive(Clone, Debug)]
struct Resolved {
    profile: OutputProfile,
    target: FormatKind,
    exact_colors: bool,
    /// 14 §ASS→BDN 的小对象过滤阈值：只过滤，不钳制尺寸（13 §7）
    min_axis: u32,
    font: String,
    /// 需要渲染器才能产出的 profile
    renders: bool,
    ext: &'static str,
}

fn ext_for(kind: FormatKind) -> &'static str {
    match kind {
        FormatKind::Ass => "ass",
        FormatKind::Ssa => "ssa",
        FormatKind::Srt => "srt",
        FormatKind::Pgs => "sup",
        FormatKind::Bdn => "xml",
        // 其余格式现在没有批量生产者：宁可在这里拒绝，也不要"看起来出了一个文件"
        _ => "bin",
    }
}

pub fn load_spec(path: &Path) -> Result<Spec> {
    let bytes = std::fs::read(path)
        .map_err(|e| cli_err(150, "无法读取 batch spec").with_detail(format!("{path:?}：{e}")))?;
    let spec: Spec = serde_json::from_slice(&bytes)
        .map_err(|e| cli_err(151, "batch spec 不是合法的 job 描述").with_detail(e.to_string()))?;
    if spec.id.trim().is_empty() {
        return Err(cli_err(152, "batch spec 的 id 不能为空"));
    }
    if spec.roots.is_empty() {
        return Err(cli_err(153, "batch spec 没有任何输入根")
            .with_action("15 §2：roots 至少一项（文件或目录）"));
    }
    if spec.profiles.is_empty() {
        return Err(cli_err(154, "batch spec 没有任何输出 profile"));
    }
    Ok(spec)
}

fn parse_overwrite(v: Option<&str>) -> Result<OverwritePolicy> {
    Ok(match v {
        None | Some("rename") => OverwritePolicy::Rename,
        Some("skip") => OverwritePolicy::Skip,
        Some("replace") => OverwritePolicy::ReplaceAfterVerify,
        Some(other) => {
            return Err(cli_err(155, "output.overwrite 只能是 skip|rename|replace")
                .with_detail(other.to_string()))
        }
    })
}

fn parse_canvas(s: &str) -> Result<Canvas> {
    let (w, h) = s
        .split_once(['x', 'X', ':'])
        .ok_or_else(|| cli_err(159, "profile.canvas 需要 WxH"))?;
    let w = w
        .trim()
        .parse::<u32>()
        .map_err(|_| cli_err(159, "profile.canvas 宽度非法"))?;
    let h = h
        .trim()
        .parse::<u32>()
        .map_err(|_| cli_err(159, "profile.canvas 高度非法"))?;
    Canvas::new(w, h)
}

fn resolve_profiles(spec: &Spec) -> Result<Vec<Resolved>> {
    let mut out = Vec::new();
    for p in &spec.profiles {
        let target = match p.format.to_ascii_lowercase().as_str() {
            "ass" => FormatKind::Ass,
            "ssa" => FormatKind::Ssa,
            "srt" => FormatKind::Srt,
            "pgs" | "sup" => FormatKind::Pgs,
            "bdn" | "xml" => FormatKind::Bdn,
            other => {
                return Err(cli_err(156, "profile.format 不支持")
                    .with_detail(format!("{} = {other}（可用：ass|ssa|srt|pgs|bdn）", p.name)))
            }
        };
        let canvas =
            parse_canvas(&p.canvas).map_err(|e| e.with_detail(format!("profile {}", p.name)))?;
        let fps = Fps::from_label(&p.fps).map_err(|e| {
            cli_err(157, "profile.fps 非法").with_detail(format!("{}：{e}", p.name))
        })?;
        out.push(Resolved {
            profile: OutputProfile {
                name: p.name.clone(),
                format: target,
                canvas,
                fps,
            },
            ext: ext_for(target),
            target,
            exact_colors: p.exact_colors,
            min_axis: p.min_axis,
            font: p
                .default_font
                .clone()
                .unwrap_or_else(|| "Arial".to_string()),
            renders: matches!(target, FormatKind::Pgs | FormatKind::Bdn),
        })
    }
    let mut names: Vec<&str> = out.iter().map(|r| r.profile.name.as_str()).collect();
    names.sort_unstable();
    if names.windows(2).any(|w| w[0] == w[1]) {
        // 重名会让报告与 .jobstate.json 分不清谁是谁（15 §8）
        return Err(cli_err(158, "batch spec 里有重名 profile"));
    }
    Ok(out)
}

fn sha256_hex(bytes: &[u8]) -> String {
    crate::pipeline::sha256_hex(bytes)
}

fn field_hash(parts: &[(&str, &str)]) -> String {
    let mut h = Sha256::new();
    for (name, value) in parts {
        // 名字与值都带长度前缀：拼接歧义在这里被结构性排除（与 CacheKey::compute 同一纪律）
        h.update((name.len() as u32).to_be_bytes());
        h.update(name.as_bytes());
        h.update((value.len() as u32).to_be_bytes());
        h.update(value.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 15 §4 的缓存 key 分量。
///
/// 有渲染的 profile 把 renderer 的**实际二进制哈希**并进分量（12 §4：不按文件名认能力）；
/// 纯文本 profile 没有这一维，显式写 `none` 而不是留空——留空会让"以后加进来一个
/// renderer 相关的 profile"看起来像 key 没变。
fn job_cache_key(
    input_sha: &str,
    r: &Resolved,
    renderer_sha: Option<&str>,
    tool: &str,
    font_env: &str,
    inject: &str,
) -> String {
    let canvas = format!("{}x{}", r.profile.canvas.width, r.profile.canvas.height);
    // min_axis 也必须进 key：改了过滤阈值却复用上一次的 BDN，等于用户改了参数没生效
    // inject 也必须进 key（AD5 的同一条纪律）：注入改变的是"哪批字节画了像素"。
    // 复用上一次的产物却不记下这件事，等于拿未注入的像素当注入的结果交付。
    let options = format!(
        "exact={}|min_axis={}|tool={}|inject={}",
        r.exact_colors, r.min_axis, tool, inject
    );
    field_hash(&[
        ("input", input_sha),
        ("format", r.ext),
        ("canvas", &canvas),
        ("fps", &r.profile.fps.to_label()),
        ("renderer", renderer_sha.unwrap_or("none")),
        ("font", &r.font),
        // 17 §7：字体环境身份。少了它，"升级了字体却拿上一次的像素交付"就是静默发生的
        ("fonts", font_env),
        ("options", &options),
    ])
}

/// 前半链（真渲染 + 变化检测）的缓存 key：**只覆盖会改变渲染结果的分量**——
/// 输入内容、画布、帧率、默认字体、renderer 二进制哈希。
///
/// `min_axis`、`exact_colors`、`level` 都发生在渲染之后，故意不放进来：放进 key 只会
/// 让同一份渲染多跑几遍，而不放进去的那些参数各自有 `job_cache_key` 管产物复用。
/// 反过来，少放一个真正影响渲染的分量就是假复用：同一 key 交出别人的像素。
fn render_cache_key(
    input_sha: &str,
    r: &Resolved,
    renderer_sha: Option<&str>,
    font_env: &str,
    inject: &str,
) -> String {
    let canvas = format!("{}x{}", r.profile.canvas.width, r.profile.canvas.height);
    field_hash(&[
        ("input", input_sha),
        ("canvas", &canvas),
        ("fps", &r.profile.fps.to_label()),
        ("font", &r.font),
        ("renderer", renderer_sha.unwrap_or("none")),
        ("fonts", font_env),
        ("inject", inject),
    ])
}

#[derive(Serialize)]
struct ProfileLine {
    input: String,
    profile: String,
    status: String,
    detail: String,
}

fn describe(o: &ProfileOutcome) -> (String, String) {
    match o {
        ProfileOutcome::WouldWrite { path, bytes } => (
            "would-write".to_string(),
            format!("{path}（dry-run：{bytes} 字节未写盘）"),
        ),
        ProfileOutcome::Verified { path, sha256 } => {
            ("verified".to_string(), format!("{path} sha256={sha256}"))
        }
        ProfileOutcome::FromCache { path, sha256 } => (
            "from-cache".to_string(),
            format!("{path} 复用 sha256={sha256}"),
        ),
        ProfileOutcome::Failed { stage, message } => {
            ("failed".to_string(), format!("{stage}: {message}"))
        }
        ProfileOutcome::Cancelled { stage } => ("cancelled".to_string(), stage.clone()),
    }
}

/// 批量任务的判定。与 `JobOutcome::classify` 同一套优先级，唯一区别：
/// dry-run 里 `WouldWrite` 记作成功（它校验过内容，只是没落盘）。
fn judge(outcomes: &[ProfileOutcome], dry_run: bool) -> JobOutcome {
    let ok = |o: &ProfileOutcome| {
        o.is_success() || (dry_run && matches!(o, ProfileOutcome::WouldWrite { .. }))
    };
    let good = outcomes.iter().filter(|o| ok(o)).count();
    let cancelled = outcomes
        .iter()
        .any(|o| matches!(o, ProfileOutcome::Cancelled { .. }));
    if outcomes.is_empty() {
        JobOutcome::Failed
    } else if good == outcomes.len() {
        JobOutcome::Success
    } else if cancelled && good == 0 {
        JobOutcome::Cancelled
    } else if good == 0 {
        JobOutcome::Failed
    } else {
        JobOutcome::PartialSuccess
    }
}

/// 产物指纹。BDN 是"XML + 同目录一组 PNG"，只哈希 XML 会漏掉 PNG 被删/被换的情况，
/// 所以目录型产物按"文件名 + 每个文件的内容哈希"整体哈希（15 §7 的复用判据必须
/// 覆盖用户实际会拿走的那组文件）。`AtomicOutput` 留下的 `.tmp` 不属于产物，排除。
fn dir_fingerprint(dir: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| !n.ends_with(".tmp"))
        .collect();
    names.sort();
    if names.is_empty() {
        return None;
    }
    let mut h = Sha256::new();
    for n in names {
        let bytes = std::fs::read(dir.join(&n)).ok()?;
        h.update(n.as_bytes());
        h.update([0]);
        h.update(sha256_hex(&bytes).as_bytes());
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn is_dir_artifact(target: FormatKind) -> bool {
    matches!(target, FormatKind::Bdn)
}

/// `None` = 产物不在盘上（复用判据据此否决 from-cache）。
fn artifact_of(r: &Resolved, path: &Path) -> Option<String> {
    if is_dir_artifact(r.target) {
        dir_fingerprint(path.parent()?)
    } else {
        std::fs::read(path).ok().map(|b| sha256_hex(&b))
    }
}

/// 把 profile 派发到与单命令完全相同的执行路径。
fn command_for(r: &Resolved, input: &Path, dll: Option<&str>) -> Command {
    match r.target {
        FormatKind::Pgs => Command::PgsEncode {
            input: input.to_path_buf(),
            fps: r.profile.fps,
            canvas: r.profile.canvas,
            level: ValidationLevel::Standard,
            dll: dll.map(PathBuf::from),
            default_font: r.font.clone(),
            exact_colors: r.exact_colors,
            // AA5 的吸附是"改动输出"的动作，批量 profile 里不暴露：
            // spec 里没有这个字段就保持与单命令默认一致（不许悄悄替用户改时间轴）。
            snap_pts: false,
        },
        FormatKind::Bdn => crate::render::command_for(
            input,
            r.profile.fps,
            r.profile.canvas,
            dll,
            &r.font,
            r.min_axis,
        ),
        to => Command::Convert {
            input: input.to_path_buf(),
            to,
            canvas: Some(r.profile.canvas),
            script_policy: ScriptResolutionPolicy::Preserve,
            layout_policy: LayoutResPolicy::KeepAsIs,
        },
    }
}

pub fn run_batch(inv: &Invocation, spec_path: &Path, cmd: &str) -> Result<Envelope> {
    run_batch_with_cancel(inv, spec_path, cmd, None)
}

/// `run_batch` 的完整签名。
///
/// `external_cancel` 让调用方（GUI 的取消按钮）把取消权握在自己手里。
/// 不传就是单命令语义：取消令牌只在本次调用内存在。
///
/// 为什么必须真共享同一个 flag，而不是“每跑完一格就把外部标志读一次”：
/// 后者只能事后发现，挡不住下一格已经开始写入；
/// 而“取消”这个词的含义就是“别再开始了”。
pub fn run_batch_with_cancel(
    inv: &Invocation,
    spec_path: &Path,
    cmd: &str,
    external_cancel: Option<&CancelToken>,
) -> Result<Envelope> {
    let started = std::time::Instant::now();
    let spec = load_spec(spec_path)?;
    let resolved = resolve_profiles(&spec)?;
    let runtime = spec.runtime.clone().unwrap_or_default();
    let pools = Pools::new(runtime.cpu, runtime.render, runtime.disk, runtime.external)?;

    // spec 里相对路径一律相对 spec 所在目录解析：任务文件挪位置不该改语义
    let base = spec_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let resolve = |p: &str| {
        let q = PathBuf::from(p);
        if q.is_absolute() {
            q
        } else {
            base.join(q)
        }
    };
    let out_dir = match spec.output.dir.as_deref() {
        Some(d) => resolve(d),
        None => base.clone(),
    };
    let template = NameTemplate::new(&spec.output.template)?;
    // 15 §6：命令行上的 --overwrite 优先于 spec——命令行是"这次要这样"，spec 是"通常这样"
    let overwrite = if inv.overwrite != OverwritePolicy::Rename || spec.output.overwrite.is_none() {
        inv.overwrite
    } else {
        parse_overwrite(spec.output.overwrite.as_deref())?
    };
    let found = DiscoverOptions {
        roots: spec.roots.iter().map(|r| resolve(r)).collect(),
        recursive: spec.recursive,
        include: spec.include.clone(),
        exclude: spec.exclude.clone(),
        video_association: spec.video_association,
    }
    .discover()?;
    if found.files.is_empty() {
        return Err(
            cli_err(161, "输入发现没有任何可处理的文件").with_detail(format!(
                "roots={}，include={:?}，exclude={:?}",
                spec.roots.join(","),
                spec.include,
                spec.exclude
            )),
        );
    }

    // 只有真的需要渲染时才要 renderer：纯文本批量任务不该被"没装 libass"卡住
    let renderer: Option<RenderIdentity> = if resolved.iter().any(|r| r.renders) {
        let dll = run::libass_dll(spec.libass_dll.as_deref().map(Path::new))?;
        let bytes = std::fs::read(&dll).map_err(|e| {
            cli_err(162, "无法读取 renderer 二进制").with_detail(format!("{dll:?}：{e}"))
        })?;
        let version =
            match render_libass::Provider::open(&dll, Canvas::HD720, Canvas::HD720, "Arial") {
                Ok(p) => format!("0x{:08X}", p.library_version()),
                // 打开失败也要继续：key 里如实记下"没读到版本号"，而不是编一个
                Err(e) => format!("unreadable:{}", e.code_string()),
            };
        Some(RenderIdentity {
            provider: "libass".to_string(),
            version,
            binary_sha256: sha256_hex(&bytes),
            arch: "x86_64".to_string(),
        })
    } else {
        None
    };
    let renderer_sha = renderer.as_ref().map(|r| r.binary_sha256.clone());
    let dll_str = spec.libass_dll.clone();
    let tool = env!("CARGO_PKG_VERSION").to_string();
    let tool_versions = vec![("subtitletool-cli".to_string(), tool.clone())];
    let state_path = out_dir.join(".jobstate.json");
    let mut state = if inv.dry_run {
        JobState::new(spec.id.clone(), tool_versions.clone())
    } else {
        JobState::read_from(&state_path)
            .unwrap_or_else(|_| JobState::new(spec.id.clone(), tool_versions.clone()))
    };

    // 一次批量任务共享一份前半链缓存：key 含输入哈希，所以跨输入不会互相污染
    let memo = crate::front::RenderMemo::default();
    let render_profiles = resolved.iter().filter(|r| r.renders).count();
    // 字体环境身份（17 §7）：库只建一次，逐输入算"这个脚本实际用了哪些 face"。
    // 不渲染的 job 根本不建库，也不该被字体环境绑住。
    let inject_flag = if inv.inject_fonts { "on" } else { "off" };
    let mut font_scanner = crate::fontenv::Scanner::with_cache(crate::fontenv::cache_mode(inv));
    let mut font_envs: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut font_env_report: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();

    let mut lines: Vec<ProfileLine> = Vec::new();
    let mut writes: Vec<WriteRecord> = Vec::new();
    let mut all: Vec<ProfileOutcome> = Vec::new();
    let spec_name = spec_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    // spec 文件自己会躺在扫描目录里，被 include 挡掉是应该的，不值得当成 warning 吓人
    let mut warnings: Vec<String> = found
        .skipped
        .iter()
        .filter(|s| !s.contains(&spec_name))
        .cloned()
        .collect();
    // 跨输入累计"因为 skip 策略而没动"的 output，好让报告能解释它
    let skipped: Mutex<Vec<String>> = Mutex::new(Vec::new());

    for input in &found.files {
        let raw = std::fs::read(input)
            .map_err(|e| cli_err(110, "无法读取输入文件").with_detail(format!("{input:?}：{e}")))?;
        let input_sha = sha256_hex(&raw);
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let job = Job {
            id: format!("{}:{}", spec.id, stem),
            input: InputSet {
                files: vec![input.display().to_string()],
                primary_sha256: input_sha.clone(),
            },
            profiles: resolved.iter().map(|r| r.profile.clone()).collect(),
        };
        // 记录与复用都要带上是哪个输入：一个 job 里 profile 名字会重复出现 N 次
        let input_key = input.display().to_string();
        let mut font_env_digest = "none".to_string();
        if resolved.iter().any(|r| r.renders) {
            let loaded = crate::pipeline::load_text(input)?;
            let (env, ms) = font_scanner.env_for(&loaded.text)?;
            font_env_digest = env.digest.clone();
            font_envs.insert(input_key.clone(), env.digest.clone());
            font_env_report.insert(input_key.clone(), crate::fontenv::env_json(&env, ms));
        } else {
            font_envs.insert(input_key.clone(), font_env_digest.clone());
        }
        let keys: Vec<String> = resolved
            .iter()
            .map(|r| {
                job_cache_key(
                    &input_sha,
                    r,
                    renderer_sha.as_deref(),
                    &tool,
                    &font_env_digest,
                    inject_flag,
                )
            })
            .collect();
        // 前半链（渲染 + 变化检测）的共享 key：**不含** profile 名，所以同一输入的
        // SUP 与 BDN 会落进同一个 slot——14 §ASS→SUP 那句"共享 render/cache"就是这件事。
        let render_keys: Vec<String> = resolved
            .iter()
            .map(|r| {
                render_cache_key(
                    &input_sha,
                    r,
                    renderer_sha.as_deref(),
                    &font_env_digest,
                    inject_flag,
                )
            })
            .collect();
        let memo = &memo;
        let paths: Vec<PathBuf> = {
            let mut v = Vec::new();
            for r in &resolved {
                let name = template.render(&NameFields {
                    stem: stem.clone(),
                    profile: r.profile.name.clone(),
                    width: r.profile.canvas.width,
                    height: r.profile.canvas.height,
                    fps: r.profile.fps.to_label(),
                    hdr: "sdr".to_string(),
                    ext: r.ext.to_string(),
                })?;
                // 目录型产物（BDN）每个输入独占一层目录：两个输入的 PNG 都从 0000.png
                // 开始编号，落在同一目录里就会互相覆盖，而 14 §Multi-output 要求
                // "输出相互独立"。模板继续决定文件名，只是父目录由 profile + stem 兜住。
                let path = if is_dir_artifact(r.target) {
                    out_dir.join(&r.profile.name).join(&stem).join(&name)
                } else {
                    out_dir.join(&name)
                };
                v.push(path);
            }
            v
        };
        let cancel = match external_cancel {
            Some(t) => CancelToken::from_shared(t.shared()),
            None => CancelToken::new(),
        };
        let budget = Mutex::new(CrashBudget::new(runtime.safe_to_restart));
        let skipped_ref = &skipped;
        let state_ref = &state;
        let dry = inv.dry_run;
        let executed =
            task_engine::run_profiles(&job, &pools, &cancel, &budget, &|_pools, prof| {
                let idx = resolved
                    .iter()
                    .position(|r| r.profile.name == prof.name)
                    .unwrap();
                let r = &resolved[idx];
                let path = &paths[idx];
                let key = &keys[idx];
                if !dry {
                    // 15 §7：只有"产物真在、指纹对得上、分量没变"才复用。
                    // 目录型产物的指纹覆盖整组文件，不只是 XML。
                    let actual = artifact_of(r, path);
                    if state_ref
                        .reuse_for(
                            &input_key,
                            &prof.name,
                            key,
                            actual.as_deref(),
                            &tool_versions,
                        )
                        .may_reuse()
                    {
                        return ProfileOutcome::FromCache {
                            path: path.display().to_string(),
                            sha256: actual.unwrap_or_default(),
                        };
                    }
                }
                let skipped = skipped_ref;
                let sub = Invocation {
                    command: command_for(r, input, dll_str.as_deref()),
                    json: false,
                    out: Some(path.clone()),
                    dry_run: dry,
                    overwrite,
                    no_font_cache: inv.no_font_cache,
                    inject_fonts: inv.inject_fonts,
                };
                // 渲染型 profile 才需要前半链；文本 profile 传 None 就是单命令语义
                let shared = if r.renders {
                    Some(crate::front::SharedRender {
                        memo,
                        key: render_keys[idx].clone(),
                    })
                } else {
                    None
                };
                match run::execute_ctx(&sub, shared.as_ref()) {
                    Ok(env) if env.ok => {
                        // 一次 render 会写多个文件，profile 的"产物"是本次请求的那个路径
                        let wanted = path.display().to_string();
                        let note = env
                            .writes
                            .iter()
                            .find(|w| w.path == wanted)
                            .cloned()
                            .or_else(|| env.writes.first().cloned())
                            .unwrap_or(WriteRecord {
                                path: wanted,
                                bytes: 0,
                                state: "written".to_string(),
                            });
                        match note.state.as_str() {
                            "would-write" => ProfileOutcome::WouldWrite {
                                path: note.path,
                                bytes: note.bytes,
                            },
                            // skip 命中：目标已在，什么都没写。这是**成功**（15 §6 的语义就是
                            // "别动它"），所以落成 FromCache 而不是 Failed；
                            // "为什么没动"由 skipped 列表进报告 warnings，不含糊过去。
                            "skipped" => {
                                if let Ok(mut g) = skipped.lock() {
                                    g.push(format!("{}（{}）", prof.name, note.path));
                                }
                                ProfileOutcome::FromCache {
                                    path: note.path.clone(),
                                    sha256: artifact_of(r, Path::new(&note.path))
                                        .unwrap_or_else(|| "0".repeat(64)),
                                }
                            }
                            _ => ProfileOutcome::Verified {
                                path: note.path.clone(),
                                sha256: artifact_of(r, Path::new(&note.path)).unwrap_or_default(),
                            },
                        }
                    }
                    Ok(env) => ProfileOutcome::Failed {
                        stage: env.command.clone(),
                        message: env
                            .errors
                            .first()
                            .map(|e| format!("{}: {}", e.code, e.title))
                            .unwrap_or_else(|| "未知失败".to_string()),
                    },
                    Err(e) => ProfileOutcome::Failed {
                        stage: e.code_string(),
                        message: e.title.clone(),
                    },
                }
            });
        // 落盘的产物才进恢复状态（15 §7）
        if !dry {
            for (name, o) in &executed.outcomes {
                if let ProfileOutcome::Verified { path, sha256 } = o {
                    if sha256.len() == 64 {
                        let idx = resolved
                            .iter()
                            .position(|r| &r.profile.name == name)
                            .unwrap();
                        state.record(StageRecord {
                            input: input_key.clone(),
                            profile: name.clone(),
                            stage: "write".to_string(),
                            path: path.clone(),
                            artifact_sha256: sha256.clone(),
                            cache_key: keys[idx].clone(),
                        })?;
                    }
                }
            }
        }
        for (name, o) in &executed.outcomes {
            let (status, detail) = describe(o);
            lines.push(ProfileLine {
                input: input.display().to_string(),
                profile: name.clone(),
                status,
                detail,
            });
            if let ProfileOutcome::Verified { path, .. } = o {
                let p = Path::new(path);
                let idx = resolved
                    .iter()
                    .position(|r| &r.profile.name == name)
                    .unwrap();
                if is_dir_artifact(resolved[idx].target) {
                    // BDN 的交付物是一组文件：信封里必须逐个报出来，只报 XML 会让用户
                    // 以为 PNG 没写（也让我们自己的"报了就落盘"检查漏掉一半产物）。
                    let dir = p.parent().unwrap_or(Path::new("."));
                    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
                        .map(|rd| {
                            rd.filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .filter(|e| {
                                    e.file_name()
                                        .and_then(|n| n.to_str())
                                        .is_some_and(|n| !n.ends_with(".tmp"))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    entries.sort();
                    for e in entries {
                        if let Ok(m) = std::fs::metadata(&e) {
                            writes.push(WriteRecord {
                                path: e.display().to_string(),
                                bytes: m.len(),
                                state: "written".to_string(),
                            });
                        }
                    }
                } else if let Ok(m) = std::fs::metadata(p) {
                    writes.push(WriteRecord {
                        path: path.clone(),
                        bytes: m.len(),
                        state: "written".to_string(),
                    });
                }
            } else if let ProfileOutcome::WouldWrite { path, bytes } = o {
                writes.push(WriteRecord {
                    path: path.clone(),
                    bytes: *bytes,
                    state: "would-write".to_string(),
                });
            }
            all.push(o.clone());
        }
        if executed.crashes > 0 {
            warnings.push(format!(
                "{}：{} 个 profile 在工作线程里 panic，已按 38 §3 计入崩溃预算",
                input.display(),
                executed.crashes
            ));
        }
    }

    if let Ok(g) = skipped.lock() {
        for s in g.iter() {
            warnings.push(format!(
                "按 overwrite=skip 策略没有改动已存在的 output：{s}"
            ));
        }
    }
    if !inv.dry_run {
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            cli_err(163, "无法创建输出目录").with_detail(format!("{out_dir:?}：{e}"))
        })?;
        std::fs::write(&state_path, state.to_json()?).map_err(|e| {
            cli_err(164, "无法写 .jobstate.json").with_detail(format!("{state_path:?}：{e}"))
        })?;
    }

    // classify 的规则是"没成功就是失败"，而 dry-run 什么都没写：
    // 直接把 WouldWrite 交给它，一次演练就会被判成 failed——那等于没人敢用 --dry-run 检查任务。
    let outcome = judge(&all, inv.dry_run);
    let report = serde_json::json!({
        "job_id": spec.id,
        "outcome": outcome.label(),
        "inputs": found.files.len(),
        "profiles": resolved.len(),
        "elapsed_ms": started.elapsed().as_millis() as u64,
        // 14 §Multi-output 的可核对证据：渲染次数应当少于"渲染型 profile 执行次数"，
        // 相等就等于没共享。三个数一起报出来，谁都能自己算。
        "render_cache": {
            "executions": render_profiles * found.files.len(),
            "renders": memo.renders(),
            "keys": memo.keys(),
        },
        "renderer": renderer.as_ref().map(|id| serde_json::json!({
            "provider": id.provider,
            "version": id.version,
            "binary_sha256": id.binary_sha256,
            "arch": id.arch,
        })),
        "details": lines,
        "warnings": warnings,
        "jobstate": if inv.dry_run { "dry-run：未读写恢复状态".to_string() } else { state_path.display().to_string() },
    });
    let pretty = serde_json::to_vec_pretty(&report)
        .map_err(|e| cli_err(165, "批量报告序列化失败").with_detail(e.to_string()))?;
    // 报告也走同一套原子输出：坏 JSON 的报告覆盖掉上一份好报告，等于没有报告
    let emitted = pipeline::emit(
        inv.out.as_deref(),
        &out_dir.join("job-report.json"),
        &pretty,
        |b| {
            serde_json::from_slice::<serde_json::Value>(b)
                .map(|_| ())
                .map_err(|e| cli_err(167, "批量报告不是合法 JSON").with_detail(e.to_string()))
        },
        inv.dry_run,
        overwrite,
    )?;
    let note = emitted.describe();
    writes.push(WriteRecord {
        path: note.path,
        bytes: note.bytes,
        state: note.state,
    });
    let data = serde_json::json!({
        "job_id": spec.id,
        "outcome": outcome.label(),
        "inputs": found.files.len(),
        "profiles": resolved.len(),
        "elapsed_ms": started.elapsed().as_millis() as u64,
        "overwrite": format!("{overwrite:?}"),
        "cache_keys": keys_of(
            &resolved,
            &found.files,
            renderer_sha.as_deref(),
            &tool,
            &font_envs,
            inject_flag,
        ),
        "font_env": font_env_report,
        "report": report,
    });
    let mut env = Envelope::ok(cmd, data);
    env.writes = writes;
    if outcome != JobOutcome::Success {
        // 部分成功/失败/取消都要 ok=false 与非零退出码阻断流水线（38 §2、20 §契约），
        // 但报告照旧完整交出：让人知道是哪一个 profile 在哪一步坏的
        env.ok = false;
        env.errors.push(ErrorEntry::of(
            &cli_err(
                166,
                &format!("批量任务判定为 {}", outcome.label()),
            )
            .with_action("看 report.details 里 status 不是 verified 的项；成功的产物不会被回滚（14 §Multi-output）"),
        ));
    }
    Ok(env)
}

/// data 里的缓存 key 摘要：每个输入 × 每个 profile 一条。
/// 批量任务被问"你为什么复用了这个文件"时，这是能拿出来的凭据（01 §任务凭据）。
fn keys_of(
    resolved: &[Resolved],
    files: &[PathBuf],
    renderer_sha: Option<&str>,
    tool: &str,
    font_envs: &std::collections::BTreeMap<String, String>,
    inject: &str,
) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for f in files {
        let sha = std::fs::read(f)
            .ok()
            .map(|b| sha256_hex(&b))
            .unwrap_or_default();
        // 报告里的 key 必须与执行时用的是同一份字体环境；写 "none" 会让报告与真实身份不一致
        let env = font_envs
            .get(&f.display().to_string())
            .cloned()
            .unwrap_or_else(|| "none".to_string());
        let rows: Vec<serde_json::Value> = resolved
            .iter()
            .map(|r| {
                serde_json::json!({
                    "profile": r.profile.name,
                    "key": job_cache_key(&sha, r, renderer_sha, tool, &env, inject),
                })
            })
            .collect();
        m.insert(f.display().to_string(), serde_json::Value::Array(rows));
    }
    serde_json::Value::Object(m)
}

#[cfg(test)]
mod inject_key_tests {
    use super::*;

    fn resolved() -> Resolved {
        Resolved {
            profile: OutputProfile {
                name: "bdn".to_string(),
                format: FormatKind::Bdn,
                canvas: Canvas::HD720,
                fps: Fps::from_label("25").unwrap(),
            },
            target: FormatKind::Bdn,
            exact_colors: false,
            min_axis: 8,
            font: "Arial".to_string(),
            renders: true,
            ext: "xml",
        }
    }

    /// `--inject-fonts` 换的是"哪批字节画了像素"，所以它必须进两个 key。
    /// 少进一个，就会把"未注入时算出的旧像素"判成可复用 —— 与 AD5（min_axis 没进 key）同一族。
    #[test]
    fn the_injection_switch_changes_both_cache_keys() {
        let r = resolved();
        assert_ne!(
            job_cache_key("sha", &r, Some("rs"), "tool", "env", "off"),
            job_cache_key("sha", &r, Some("rs"), "tool", "env", "on"),
            "job key 没记注入开关"
        );
        assert_ne!(
            render_cache_key("sha", &r, Some("rs"), "env", "off"),
            render_cache_key("sha", &r, Some("rs"), "env", "on"),
            "前半链 key 没记注入开关"
        );
        // 反方向也要成立：这次改动不能把 §32 那条（字体环境不同必须是两个 key）冲掉
        assert_ne!(
            render_cache_key("sha", &r, Some("rs"), "env-A", "on"),
            render_cache_key("sha", &r, Some("rs"), "env-B", "on")
        );
    }
}
