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
//! 4. **spec 是 JSON 不是 TOML**：Preset 是可移植的目标配置，batch spec 是冻结的
//!    执行输入；两者职责不同。`serde_json` 已在依赖表里，字段一律
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
    /// 每个输出 Profile 独立的时间与 HDR 变换；没有设置时保持输入原样。
    #[serde(default)]
    pub offset_ms: Option<i64>,
    #[serde(default)]
    pub speed_from: Option<String>,
    #[serde(default)]
    pub speed_to: Option<String>,
    #[serde(default)]
    pub hdr_percent: Option<u32>,
    #[serde(default)]
    pub hdr_reference_nits: Option<u32>,
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

/// 默认命名模板。
///
/// 与 15 §5 逐字一致：`{stem}.{profile}.{width}x{height}.{fps}.{hdr}.{ext}`。
///
/// ## 为什么不能只写 `{stem}.{profile}.{ext}`
///
/// 那是第一版的写法，实用上有一个坑：**只要用户把 profile 名起得与格式同形**
/// （叫 `srt`、`sup`、`ass`——这是最自然的起法），产物就会变成
/// `demo.srt.srt`、`demo.sup.sup`。实测过：给一个名为 `srt` 的 profile，
/// 产物就是 `demo.srt.srt`。
///
/// 名字里带上尺寸/帧率/HDR 之后，即使 profile 名为 `SRT`，产物也是
/// `demo.SRT.1920x1080.25.sdr.srt` —— 仍是多余的，但**信息量是完整的**，
/// 而且与规格写的一致。
///
/// 注意这**不是**在替用户防呆：用户显式给了 `template` 就完全按他的来。
/// 这里只管默认值。
fn default_template() -> String {
    "{stem}.{profile}.{width}x{height}.{fps}.{hdr}.{ext}".to_string()
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

/// 精确重试的一个工作单元。空列表表示常规全矩阵批处理。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BatchPair {
    pub input: String,
    pub profile: String,
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
    pub only_pairs: Vec<BatchPair>,
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
    options: SpecProfile,
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
    if spec.only_pairs.iter().any(|p| p.input.trim().is_empty() || p.profile.trim().is_empty()) {
        return Err(cli_err(218, "only_pairs 不能包含空的输入路径或 Profile 名"));
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
    // 实际画布在每个输入上解析；此处的临时值不能拿来执行。
    if s.eq_ignore_ascii_case("original") {
        return Ok(Canvas::FHD1080);
    }
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
            options: p.clone(),
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
        "exact={}|min_axis={}|tool={}|inject={}|transform={}",
        r.exact_colors, r.min_axis, tool, inject, transform_fingerprint(&r.options)
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
        ("transform", &transform_fingerprint(&r.options)),
    ])
}

fn transform_fingerprint(p: &SpecProfile) -> String {
    format!(
        "offset={:?};speed={:?}/{:?};hdr={:?}/{:?}",
        p.offset_ms, p.speed_from, p.speed_to, p.hdr_percent, p.hdr_reference_nits
    )
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

/// 读取每个输入的原始逻辑画布；拒绝“保持原样”却实际偷偷改成 1080p。
fn original_canvas(input: &Path, target: FormatKind) -> Result<Canvas> {
    // 纯文本 SRT 没有画布，且导出 SRT 时画布不参与输出。
    if matches!(target, FormatKind::Srt) {
        return Ok(Canvas::FHD1080);
    }
    let loaded = pipeline::load_text(input)?;
    let document = codec_ass::AssDocument::parse(&loaded.text);
    let (width, height) = document.play_res().ok_or_else(|| {
        cli_err(220, "输入没有 PlayResX/PlayResY，无法保持原始画布")
            .with_detail(input.display().to_string())
    })?;
    let w = u32::try_from(width).map_err(|_| cli_err(220, "原始画布宽度非法"))?;
    let h = u32::try_from(height).map_err(|_| cli_err(220, "原始画布高度非法"))?;
    Canvas::new(w, h)
}

/// 一次 Profile 的临时输入文件；即使渲染失败也通过 Drop 清理。
struct PreparedInput {
    path: PathBuf,
}
impl Drop for PreparedInput {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
}

fn prepare_input(input: &Path, resolved: &Resolved) -> Result<Option<PreparedInput>> {
    use core_model::loss::LossReport;
    use core_model::profile::HdrSettings;
    use transform_time::{PiecewiseSpec, TimeConfig};
    let p = &resolved.options;
    if p.offset_ms.is_none() && p.speed_from.is_none() && p.speed_to.is_none()
        && p.hdr_percent.is_none() && p.hdr_reference_nits.is_none() {
        return Ok(None);
    }
    if p.offset_ms.is_some() && (p.speed_from.is_some() || p.speed_to.is_some()) {
        return Err(cli_err(221, "不能同时指定时间偏移和变速"));
    }
    if p.hdr_percent.is_some() && p.hdr_reference_nits.is_some() {
        return Err(cli_err(221, "不能同时指定两种 HDR 映射"));
    }
    let loaded = pipeline::load_text(input)?;
    let input_kind = format_detect::probe_file(input)?.format;
    let time = if let Some(ms) = p.offset_ms {
        Some(TimeConfig::offset(ms))
    } else if let (Some(from), Some(to)) = (&p.speed_from, &p.speed_to) {
        let from = Fps::from_label(from)?;
        let to = Fps::from_label(to)?;
        Some(TimeConfig::speed_scale(from, to))
    } else if p.speed_from.is_some() || p.speed_to.is_some() {
        return Err(cli_err(221, "变速必须同时指定源与目标帧率"));
    } else { None };
    let settings = if let Some(percent) = p.hdr_percent {
        Some(HdrSettings::percentage(percent.checked_mul(100).ok_or_else(|| cli_err(221, "HDR 百分比溢出"))?)?)
    } else if let Some(nits) = p.hdr_reference_nits {
        Some(HdrSettings::reference_white(nits)?)
    } else { None };
    let (content, extension) = match input_kind {
        format_detect::SubtitleFormat::Ass | format_detect::SubtitleFormat::Ssa => {
            let mut document = codec_ass::AssDocument::parse(&loaded.text);
            pipeline::check_ass_dialogue_count(&document)?;
            if let Some(cfg) = time {
                pipeline::time_ass(&mut document, &cfg, None, &PiecewiseSpec::new(Vec::new()))?;
            }
            if let Some(settings) = settings {
                transform_hdr::apply(&mut document, &settings, &mut LossReport::new())?;
            }
            (document.render().into_bytes(), "ass")
        }
        format_detect::SubtitleFormat::Srt => {
            if settings.is_some() {
                return Err(cli_err(222, "SRT 不包含 HDR 颜色属性，不能执行 HDR 映射"));
            }
            let (subtitle, _) = codec_srt::parse(&loaded.bytes)?;
            let subtitle = if let Some(cfg) = time {
                pipeline::time_srt(&subtitle, &cfg, None, &PiecewiseSpec::new(Vec::new()))?.0
            } else { subtitle };
            (codec_srt::write(&subtitle)?, "srt")
        }
        _ => return Err(cli_err(222, "此输入格式不支持 batch 时间/HDR 变换")),
    };
    // 不能修改原文件；temp 文件在输出目录外，且带进程内唯一序号。
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join("subtitletool-profile-stage");
    std::fs::create_dir_all(&dir).map_err(StkError::from)?;
    let path = dir.join(format!("profile-{}-{seq}.{extension}", std::process::id()));
    std::fs::write(&path, content).map_err(StkError::from)?;
    Ok(Some(PreparedInput { path }))
}

/// 把 profile 派发到与单命令完全相同的执行路径。
fn command_for(r: &Resolved, input: &Path, dll: Option<&str>) -> Command {
    match r.target {
        FormatKind::Pgs => Command::PgsEncode {
            input: input.to_path_buf(),
            // 批量 spec 里的值是**已解析的确定值**，所以一律 `Some`：
            // spec 自己就是一份完整的参数快照（36 §1 的 JobSnapshot 层），
            // 不应被用户的 ToolDefaults 影响 —— 否则同一份 spec
            // 在不同机器上会跑出不同的结果，而“可重放”正是它的用途。
            fps: Some(r.profile.fps),
            canvas: Some(r.profile.canvas),
            // 与单命令默认一致（spec 没有这个字段）。
            // 也不查 ToolDefaults：同上，spec 是冻结的参数。
            level: Some(ValidationLevel::Standard),
            dll: dll.map(PathBuf::from),
            default_font: Some(r.font.clone()),
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
    tracing::info!(
        job_id = %spec.id,
        inputs = found.files.len(),
        profiles = resolved.len(),
        render_profiles = resolved.iter().filter(|profile| profile.renders).count(),
        dry_run = inv.dry_run,
        "批处理已解析"
    );

    // 只有真的需要渲染时才要 renderer：纯文本批量任务不该被"没装 libass"卡住
    let renderer: Option<RenderIdentity> = if resolved.iter().any(|r| r.renders) {
        let dll = run::libass_dll(spec.libass_dll.as_deref().map(Path::new))?;
        let bytes = std::fs::read(&dll).map_err(|e| {
            cli_err(162, "无法读取 renderer 二进制").with_detail(format!("{dll:?}：{e}"))
        })?;
        // 绝不在 CLI 主进程里打开 provider：它可能崩溃，且 x86 provider 不能由 x64
        // 进程加载。DLL 版本必须由隔离 host 提供；当前协议未返回版本时明确记未知。
        let version = "unknown (not loaded in CLI process)".to_string();
        Some(RenderIdentity {
            provider: "libass".to_string(),
            version,
            binary_sha256: sha256_hex(&bytes),
            // 23 §DLL：架构从**二进制的 PE 头**读，不写死。
            // 写死一个 "x86_64" 会让 JobSnapshot 里那句“当时用了哪个二进制”变成谎话
            // ——而 JobSnapshot 的全部用途就是回答这个问题（20 §Repro）。
            // 走到这里时 `libass_dll` 已校验过位数，所以读失败只可能是文件在
            // 两次读之间被替掉；如实记未知而不编一个值。
            arch: core_model::pe::arch_of_file(&dll)
                .map(|a| a.label().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
        })
    } else {
        None
    };
    let renderer_sha = if let Some(identity) = &renderer {
        let dll = run::libass_dll(spec.libass_dll.as_deref().map(Path::new))?;
        let host = run::renderer_host_exe_for_dll(&dll).ok_or_else(|| {
            cli_err(163, "找不到与渲染库架构匹配的 renderer-host")
                .with_detail(dll.display().to_string())
        })?;
        let host_bytes = std::fs::read(&host).map_err(|e| {
            cli_err(164, "无法读取 renderer-host").with_detail(format!("{}：{e}", host.display()))
        })?;
        // 同一 provider 由不同位数/版本的 host 执行时不能共用渲染缓存。
        Some(sha256_hex(
            format!("{}:{}", identity.binary_sha256, sha256_hex(&host_bytes)).as_bytes(),
        ))
    } else {
        None
    };
    let dll_str = spec.libass_dll.clone();
    let tool = env!("CARGO_PKG_VERSION").to_string();
    let tool_versions = vec![("subtitletool-cli".to_string(), tool.clone())];
    let state_path = out_dir.join(".jobstate.json");
    // 损坏的历史恢复文件必须留证据；不能静默当成“从没跑过”。
    let mut recovery_warning = None;
    let mut state = if inv.dry_run || !state_path.exists() {
        JobState::new(spec.id.clone(), tool_versions.clone())
    } else {
        match JobState::read_from(&state_path) {
            Ok(saved) => saved,
            Err(error) => {
                let quarantine = state_path.with_extension("jobstate.corrupt");
                if quarantine.exists() {
                    return Err(cli_err(219, "旧恢复状态损坏，诊断副本已存在")
                        .with_detail(quarantine.display().to_string())
                        .with_action("请先备份并检查损坏的恢复文件，再重新执行"));
                }
                std::fs::rename(&state_path, &quarantine).map_err(|e| {
                    cli_err(219, "无法保存损坏的恢复状态")
                        .with_detail(format!("{}: {e}", state_path.display()))
                })?;
                recovery_warning = Some(format!(
                    "旧恢复状态无法解析（{error}），已移至 {}，将重新计算输出",
                    quarantine.display()
                ));
                JobState::new(spec.id.clone(), tool_versions.clone())
            }
        }
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
    if let Some(warning) = recovery_warning {
        warnings.push(warning);
    }
    // 同名字幕在不同目录里很常见：只在碰撞时加路径指纹，不改非碰撞文件的现有命名。
    let mut stem_counts = std::collections::HashMap::<String, usize>::new();
    for path in &found.files {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
        *stem_counts.entry(stem.to_string()).or_default() += 1;
    }
    // 跨输入累计"因为 skip 策略而没动"的 output，好让报告能解释它
    let skipped: Mutex<Vec<String>> = Mutex::new(Vec::new());

    for input in &found.files {
        // 精确重试只能执行原来失败的 (input, profile) 配对；不能把两组
        // 唯一值再做笛卡尔积，否则会重新写入上一轮已成功的产物。
        let selected: Vec<Resolved> = resolved.iter()
            .filter(|r| spec.only_pairs.is_empty() || spec.only_pairs.iter().any(|pair| {
                let requested = resolve(&pair.input);
                requested == *input && pair.profile == r.profile.name
            }))
            .cloned()
            .map(|mut r| {
                if r.options.canvas.eq_ignore_ascii_case("original") {
                    r.profile.canvas = original_canvas(input, r.target)?;
                }
                Ok(r)
            })
            .collect::<Result<Vec<_>>>()?;
        if selected.is_empty() {
            continue;
        }
        let resolved = &selected;
        let raw = crate::pipeline::read_bounded(input, crate::pipeline::MAX_BINARY_BYTES, "输入")?;
        let input_sha = sha256_hex(&raw);
        let original_stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let stem = if stem_counts.get(original_stem).copied().unwrap_or(0) > 1 {
            let canonical = input.canonicalize().unwrap_or_else(|_| input.clone());
            let fingerprint = sha256_hex(canonical.to_string_lossy().as_bytes());
            format!("{original_stem}-{}", &fingerprint[..12])
        } else {
            original_stem.to_string()
        };
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
                // 先对输入的私有副本应用这个 Profile 的时间/HDR，再走原单命令链。
                // 变换身份已加入 job/render 缓存键；不会把别的 Profile 的像素拿来复用。
                let transformed = match prepare_input(input, r) {
                    Ok(stage) => stage,
                    Err(error) => return ProfileOutcome::Failed {
                        stage: error.code_string(),
                        message: error.title,
                    },
                };
                let command_input = transformed.as_ref().map(|stage| stage.path.as_path()).unwrap_or(input);
                let sub = Invocation {
                    command: command_for(r, command_input, dll_str.as_deref()),
                    json: false,
                    out: Some(path.clone()),
                    dry_run: dry,
                    overwrite,
                    no_font_cache: inv.no_font_cache,
                    inject_fonts: inv.inject_fonts,
                    // 子调用继承父调用：批量里的「关掉渲染缓存」必须真的关到每个 profile，
                    // 否则批量路径与单命令路径的输出会出现不可解释的差异。
                    render_cache: inv.render_cache,
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
            match o {
                ProfileOutcome::Failed { stage, message } => tracing::error!(
                    job_id = %spec.id,
                    input = %stem,
                    profile = %name,
                    stage = %stage,
                    error = %message,
                    "批处理 profile 失败"
                ),
                ProfileOutcome::Cancelled { stage } => tracing::warn!(
                    job_id = %spec.id,
                    input = %stem,
                    profile = %name,
                    stage = %stage,
                    "批处理 profile 已取消"
                ),
                _ => tracing::debug!(
                    job_id = %spec.id,
                    input = %stem,
                    profile = %name,
                    status = %status,
                    "批处理 profile 完成"
                ),
            }
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
        state.write_to(&state_path).map_err(|e| {
            cli_err(164, "无法原子提交 .jobstate.json").with_detail(format!("{state_path:?}：{e}"))
        })?;
    }

    // classify 的规则是"没成功就是失败"，而 dry-run 什么都没写：
    // 直接把 WouldWrite 交给它，一次演练就会被判成 failed——那等于没人敢用 --dry-run 检查任务。
    let outcome = judge(&all, inv.dry_run);
    tracing::info!(
        job_id = %spec.id,
        outcome = outcome.label(),
        inputs = found.files.len(),
        profiles = resolved.len(),
        renders = memo.renders(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "批处理结束"
    );
    // 22 §2 的三层 id。
    //
    // `job_id` 是**用户给的** spec id（“我这次任务叫什么”），而 `corr_id` 是
    // **本次执行**的 id（“这一次跑”）—— 两者不能合并：同一个 spec 会跑很多次，
    // 而用户要能定位其中**那一次**。
    //
    // `output_ids` 与 `renderer_id` 都继承 job 的共享段，于是列表里按共享段
    // 分组就能把一次执行的全部输出聚在一起（见 `core_model::corrid`）。
    let corr = core_model::CorrId::new_job();
    let output_ids: Vec<serde_json::Value> = resolved
        .iter()
        .map(|r| {
            let o = corr.child(core_model::CorrScope::Output);
            serde_json::json!({
                "profile": r.profile.name,
                "corr_id": o.as_string(),
            })
        })
        .collect();
    let renderer_id = renderer
        .as_ref()
        .map(|_| corr.child(core_model::CorrScope::Renderer).as_string());
    let report = serde_json::json!({
        "job_id": spec.id,
        "corr_id": corr.as_string(),
        "output_ids": output_ids,
        "renderer_id": renderer_id,
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
        // 22 §2：同一个 corr_id 也进顶层信封（`Envelope::with_corr_id` 会覆盖），
        // 于是 `data` 里的这一份与信封上那一份、日志里那一份三处一致。
        "corr_id": corr.as_string(),
        "output_ids": output_ids,
        "renderer_id": renderer_id,
        "outcome": outcome.label(),
        "inputs": found.files.len(),
        "profiles": resolved.len(),
        "selected_pairs": if spec.only_pairs.is_empty() { serde_json::Value::Null } else { serde_json::json!(&spec.only_pairs) },
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
    // 22 §2：批量**自己**定 corr_id —— 它的子 id（`output_ids`）已经按这个
    // 共享段生成好了，所以不能用 `with_corr_id`（那只是换个显示值），
    // 而要用 `claim_corr_id` 声明“这个 id 由我定”。
    let mut env = Envelope::ok(cmd, data).claim_corr_id(&corr.as_string());
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
            options: SpecProfile {
                name: "bdn".to_string(), format: "bdn".to_string(),
                canvas: "1280x720".to_string(), fps: "25".to_string(),
                exact_colors: false, min_axis: 8, default_font: None,
                offset_ms: None, speed_from: None, speed_to: None,
                hdr_percent: None, hdr_reference_nits: None,
            },
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
