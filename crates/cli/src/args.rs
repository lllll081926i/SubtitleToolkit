//! 参数解析（20 §Commands）。
//!
//! 刻意不引入 CLI 框架：命令面小、需要精确控制错误码与 `--json` 语义，
//! 而且 GUI 与 CLI 必须共用同一份参数 schema（36 §5），手写反而更容易核对。
//!
//! 全局开关（`--json/--dry-run/--out`）可以在命令前后任意位置出现：脚本重放时人们把它们
//! 加在末尾，GUI 生成的命令行习惯放前面，两种都必须能用。

use std::path::PathBuf;

use codec_pgs::fastpath::FastOp;
use core_model::error::StkError;
use core_model::geometry::{LayoutResPolicy, ScriptResolutionPolicy};
use core_model::profile::{FormatKind, HdrMode, HdrSettings, ValidationLevel};
use core_model::time::{Fps, Rational, Time};
use core_model::Canvas;
use task_engine::OverwritePolicy;
use transform_time::{ControlPoint, PiecewiseSpec, TimeMode, TwoPointSpec};

pub const USAGE: &str = "\
usage: subtitletool-cli <command> [options] [--out PATH] [--json] [--dry-run]

commands:
  probe        <file>                探测格式/编码；视频文件读帧率候选（需要 ffprobe）
  convert      <file> --to srt|ass|ssa
                     [--canvas WxH] [--script-policy preserve|rewrite]
                     [--layout-policy keep-as-is|scale-with-script|drop]
  time         <file> --offset-ms N | --speed SRC --to-fps DST | --snap-fps FPS
                     | --two-point 源1,目标1,源2,目标2
                     | --piecewise 源,目标 --piecewise 源,目标 [--piecewise ...]
                     10 §5：--piecewise 可重复给多段（相邻两点连成一段），用于广告/剪辑差异；
                     段外按最近一段的斜率外推，并在报告里报出外推了多少条
  hdr          <file> --hdr-mode compatibility|percentage|reference-white
                     [--nits N] [--bp 万分比] [--touch-alpha]
                     [--touch-styles yes|no] [--touch-inline yes|no]
  pgs adjust   <file.sup> [--offset-ms N] [--scale A/B] [--move DX,DY]
                     [--forced yes|no] [--luma-basis-points BP]
  pgs validate <file.sup>   [--level probe|structure|basic|standard|strict]
                     [--peak-bitrate-bps N]
                     分层校验：1 结构 / 2 时间线 / 3 生命周期 / 4 profile / 6 码率测量
                     （第 5 层缓冲模型与第 7 层设备语料未覆盖，按 strict 跑会给出 warning 并拒绝发徽章）
  render       <file.ass>   [--fps R] [--canvas WxH] [--libass-dll PATH]
                     [--default-font FAMILY] [--min-axis N]
                     14 §ASS→BDN：渲染 → 变化检测 → tight 裁剪 → PNG → BDN XML → 目录校验
                     [--out 是 XML 路径，PNG 写在同目录]；需要 native 渲染器
  pgs encode   <file.ass>   [--fps R] [--canvas WxH] [--level ...] [--snap-pts]
                     [--libass-dll PATH] [--default-font FAMILY] [--exact-colors] [--snap-pts]
                     14 §ASS→SUP：真 libass 渲染 → 采样 → 变化检测 → PGS author → 回读校验
                     需要 native 渲染器（--libass-dll 或环境变量 STK_LIBASS_DLL）
                     默认用 Advanced 量化把颜色压进 255 预算并记账；--exact-colors 改回 Basic（超色即失败）
  batch        --spec FILE      15 §1 批量任务：输入发现 × 多输出 profile × 报告
                     profile.format = ass|ssa|srt|pgs|bdn；pgs/bdn 需要 libass
                     [--out report.json] [--dry-run] [--overwrite skip|rename|replace]
                     spec 里的相对路径相对 spec 自身解析；bdn 是目录型产物，
                     每个输入单独一层目录（{profile}/{stem}/），PNG 与 XML 同放
  font scan    <file.ass> [--fonts-dir DIR]... [--out report.json]
                     17 §2-§4：读字体表建库 → 抽 ASS 真实用量 → 评级 + 缺字清单
                     默认目录 %WINDIR%\\Fonts 与 %LOCALAPPDATA%\\Microsoft\\Windows\\Fonts，
                     并读注册表 HKLM/HKCU\\...\\CurrentVersion\\Fonts 的登记（--no-registry 可关）
                     [--copy-fonts DIR] 17 §6 第二档：把解析到的字体文件连 manifest 一起复制过去
                     不带 --copy-fonts 时只出报告：不写字体、不装字体、不 subset（17 §6 第一档）
  font subset  <file.ass> [--fonts-dir DIR]... [--no-registry]
                     [--subset-dir ABS_DIR] [--fallback-full]
                     [--embed-fonts --out EMBEDDED.ass] [--mkv-attachments]
                     17 §5/§6：把 ASS 真正用到的字符做成字体子集，并做 cmap 级等价检查
                     复杂脚本（Arabic/Indic/combining/vertical/IVS/emoji）一律回退整份
                     默认写到源字体目录下的 .subtitletool-subset/，不覆盖原字体
                     --embed-fonts：把字体写进产物 ASS 的 [Fonts] 段（需 --out）
                     --mkv-attachments：生成可直接嗂给 mkv remux --attach-font 的附件目录
  mkv identify <file.mkv> [--tools-dir DIR]
                     18 §2：列出轨道（语言/名称/default/forced）与附件（含字体附件）
  mkv extract  <file.mkv> --track ID:OUT... [--attachment ID:OUT]... [--tools-dir DIR]
                     18 §2：抽出字幕轨或字体附件；源文件只读
  mkv remux    <file.mkv> --out NEW.mkv [--subtitle-tracks 2,3 | --all-subtitles]
                     [--drop-video] [--drop-audio] [--attach-font FONT]...
                     [--track-name ID:NAME] [--track-language ID:LANG]
                     [--default-track ID] [--forced-track ID] [--tools-dir DIR]
                     18 §2/§4：重封装成新 MKV，绝不原地修改；写完重新 identify 核对元数据
                     需要 MKVToolNix（--tools-dir 或 STK_MKVTOOLNIX_DIR），不做视频/音频转码
  doctor                             环境自检：架构、可写路径、ffprobe 身份、未实现清单
cache [stat|clear]                 37 §2：报账或清理磁盘渲染缓存
settings [show|init|set]           36 §1：查看/初始化/修改全局偏好
   settings set <键> <值>           受管字段：log_level / ui_language / default_fps / default_canvas
diagnose [--include-input F]…      22 §5：打出诊断包 zip（**默认不含**原字幕/视频，附上需点名）

全局开关：--json（单行信封）/ --dry-run（不写盘）/ --out PATH / --overwrite skip|rename|replace
         / --no-font-cache（不使用字体库磁盘缓存，每次全量重扫）
         / --no-render-cache（37 §2：不用磁盘渲染缓存，每次都真渲染）
         / --inject-fonts（17 §7：把解析到的字体字节交给 libass，让渲染用的就是那份字节）
覆盖策略默认 rename：目标已存在就换个名字写，绝不直接截断原文件（15 §6、AT-OUT-001）
尚未实现（返回 STK-INP-0060 并指明所属 Phase，不会假装成功）：
  --preset FILE                        等 TOML 依赖按 21 §1 批准（ISSUES E4）；现在会明确报错
";

/// 20 §Commands 的全部命令名（`pgs` 的三个子命令分开列，因为它们是三条不同的承诺）。
pub const COMMAND_CATALOG: &[&str] = &[
    "probe",
    "convert",
    "time",
    "hdr",
    "render",
    "pgs adjust",
    "pgs validate",
    "pgs encode",
    "batch",
    "font scan",
    "font subset",
    "mkv identify",
    "mkv extract",
    "mkv remux",
    "cache",
    "settings",
    "diagnose",
    "doctor",
];

/// 哪些命令还没有 backing implementation。
///
/// **问解析器，不维护第二份清单**：手写的"未实现列表"曾经把已经落地的
/// `pgs encode` 与 `batch` 继续列在未实现里，而没有任何测试会发现——
/// doctor 报告的恰恰是"这个产品能干什么"，它说错就等于用户对功能面的判断错。
pub fn not_built_commands() -> Vec<&'static str> {
    COMMAND_CATALOG
        .iter()
        .copied()
        .filter(|name| {
            let mut argv: Vec<String> = name.split(' ').map(str::to_string).collect();
            argv.push("x".to_string());
            match parse(&argv) {
                Ok(inv) => matches!(inv.command, Command::NotBuilt { .. }),
                // 参数不对 => 这条命令有自己的解析器，也就是已经实现
                Err(_) => false,
            }
        })
        .collect()
}

/// 无值开关。其余 `--flag` 都必须带取值。
const SWITCHES: &[&str] = &[
    "--touch-alpha",
    "--exact-colors",
    "--no-registry",
    "--snap-pts",
    "--all-subtitles",
    "--drop-video",
    "--drop-audio",
    "--fallback-full",
    "--embed-fonts",
    "--mkv-attachments",
];

/// `cache` 子命令的两个动作（37 §2）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CacheAction {
    /// 只报账：目录、字节数、条数、配额上限。
    #[default]
    Stat,
    /// 删掉所有渲染缓存记录（只删本产品自己的 `.rc` 文件）。
    Clear,
}

/// `settings` 子命令（36 §1）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsAction {
    /// 报出当前生效的设置、来源与文件位置（**默认**）。
    #[default]
    Show,
    /// 把设置写成一份带默认值的文件，便于用户手工编辑。
    Init,
    /// 改一个受管字段：`settings set <键> <值>`。
    Set,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Command {
    Probe {
        input: PathBuf,
    },
    Convert {
        input: PathBuf,
        to: FormatKind,
        canvas: Option<Canvas>,
        script_policy: ScriptResolutionPolicy,
        layout_policy: LayoutResPolicy,
    },
    Time {
        input: PathBuf,
        mode: TimeMode,
        source_fps: Option<Fps>,
        target_fps: Option<Fps>,
        offset_ms: i64,
        two_point: Option<TwoPointSpec>,
        /// 10 §5 分段同步的控制点（可重复的 `--piecewise 源,目标`）
        piecewise: PiecewiseSpec,
    },
    Hdr {
        input: PathBuf,
        settings: HdrSettings,
    },
    PgsAdjust {
        input: PathBuf,
        ops: Vec<FastOp>,
    },
    PgsValidate {
        input: PathBuf,
        level: ValidationLevel,
        peak_bps: Option<i64>,
    },
    /// 14 §ASS→SUP：渲染 → 采样 → author → 校验 → 写出。
    PgsEncode {
        input: PathBuf,
        fps: Fps,
        canvas: Canvas,
        level: ValidationLevel,
        dll: Option<PathBuf>,
        default_font: String,
        exact_colors: bool,
        /// AA5：把每个显示集起点吸附到 --fps 的帧网格（默认关，输出逐字节不变）
        snap_pts: bool,
    },
    /// 14 §ASS→BDN：渲染成 BDN XML + PNG 目录。
    Render {
        input: PathBuf,
        fps: Fps,
        canvas: Canvas,
        dll: Option<PathBuf>,
        default_font: String,
        min_axis: u32,
    },
    #[default]
    Doctor,
    /// 37 §2：查看或清理磁盘渲染缓存的账目。
    ///
    /// 为什么需要这条命令：配额、命中率与损坏计数如果不给用户看，
    /// 缓存就变成一个不可核对的愚盒 —— 而 37 §2 明确要求配额与容量有记录。
    Cache {
        action: CacheAction,
    },
    /// 36 §1 的 AppSettings：查看/修改跨会话保留的全局偏好。
    Settings {
        action: SettingsAction,
        /// `set` 的目标字段名；其余动作是空。
        key: Option<String>,
        /// `set` 的新值；其余动作是空。
        value: Option<String>,
    },
    /// 22 §5 的诊断包：把故障现场打成一个 zip。
    Diagnose {
        /// 要附进去的输入文件（**显式**选择才会包含用户内容）。
        include_input: Vec<PathBuf>,
        /// 把上一次执行的 job state（如果命令给了它）一并收进去。
        job_state: Option<PathBuf>,
    },
    /// 15 §1 的批量任务：输入集合 × 多个输出 profile，spec 是 JSON。
    Batch {
        spec: PathBuf,
    },
    /// 17 §2–§4 的字体用量扫描：ASS 引用的 face × 本机字体库（含注册表登记的字体）。
    FontScan {
        input: PathBuf,
        fonts_dirs: Vec<PathBuf>,
        /// 读不读 `HKLM/HKCU\...\CurrentVersion\Fonts` 那张登记表（默认读）
        registry: bool,
        /// 17 §6 的第二档：把解析到的 face 文件连同 manifest 复制到指定目录
        copy_fonts: Option<PathBuf>,
    },
    /// 17 §5/§6 的子集：把 ASS 真正用到的字符做成字体子集（复杂脚本回退整份）。
    FontSubset {
        input: PathBuf,
        fonts_dirs: Vec<PathBuf>,
        /// 读不读注册表登记的字体（默认读，与 font scan 一致）
        registry: bool,
        /// 子集输出目录；None = 源字体目录下的 `.subtitletool-subset/`
        subset_dir: Option<PathBuf>,
        /// 有任何一份回退就整体失败，而不是混合交付
        fallback_full: bool,
        /// 17 §6 第四档：把字体写进产物 ASS 的 `[Fonts]` 段
        embed_fonts: bool,
        /// 17 §6 第五档：生成可直接喂给 `mkv remux --attach-font` 的附件目录
        mkv_attachments: bool,
    },
    /// 18 §2 的 MKV 助手：identify / extract / remux，全部走外部 MKVToolNix。
    Mkv {
        action: MkvAction,
        /// MKVToolNix 安装目录（mkvmerge/mkvextract 都在里面）；None 走环境变量/默认
        tools_dir: Option<PathBuf>,
    },
    /// 20 §Commands 已规划、但当前 Phase 还没有 backing crate 的命令。
    /// 明确报错而不是"看起来成功了"，这是 39 检查表的要求。
    NotBuilt {
        name: String,
        phase: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MkvAction {
    /// `mkv identify FILE`：列出轨道与附件（18 §2 第一条）。
    Identify { input: PathBuf },
    /// `mkv extract FILE --track ID:OUT [--attachment ID:OUT]...`
    Extract {
        input: PathBuf,
        tracks: Vec<(u64, PathBuf)>,
        attachments: Vec<(u64, PathBuf)>,
    },
    /// `mkv remux FILE [...]`：新的 MKV，绝不原地修改（18 §4）。
    /// 输出路径走全局 `--out`（与其它写产物的命令一致），不是子命令专属开关。
    Remux {
        input: PathBuf,
        /// 保留哪些字幕轨（mkvmerge 轨道 id）。空 = 全部丢弃。
        subtitle_tracks: Vec<u64>,
        /// `--all-subtitles`：保留全部字幕轨（与 `--subtitle-tracks` 互斥）
        all_subtitles: bool,
        /// 保留哪些视频/音频轨；默认全保留（`--drop-video`/`--drop-audio` 显式丢弃）
        keep_video: bool,
        keep_audio: bool,
        /// 要附加的字体文件（18 §2 `attach subset fonts`）
        attach_fonts: Vec<PathBuf>,
        /// 设置轨道元数据：`--track-name ID:NAME`、`--track-language ID:LANG`、
        /// `--default-track ID`、`--forced-track ID`
        track_name: Vec<(u64, String)>,
        track_language: Vec<(u64, String)>,
        default_track: Vec<u64>,
        forced_track: Vec<u64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    pub command: Command,
    pub json: bool,
    pub out: Option<PathBuf>,
    pub dry_run: bool,
    /// 15 §6 的覆盖策略。默认 `Rename`：绝不直接 truncate 原文件。
    pub overwrite: OverwritePolicy,
    /// 把解析到的字体字节交给 libass（17 §7 `ass_add_font`）。
    pub inject_fonts: bool,
    /// 不要用磁盘字体库缓存（17 §2/AI5）。留着它是为了"怀疑缓存时有一条独立可复现的路"，
    /// 不是为了日常提速 —— 默认那条路在使用点复核过被用到的每个 face。
    pub no_font_cache: bool,
    /// 37 §2 的磁盘渲染缓存开关。默认开：它是一个**纯优化**（命中就省一次 native
    /// 渲染，未命中就是没命中），不改变输出字节。
    /// 关掉是为了两件事：对照跑（验证两次运行真的一样）与排查"是不是缓存骗了我"。
    pub render_cache: bool,
}

/// 参数解析阶段的失败。这里**没有**"用法错 / 取值错"的区分：
/// `parse()` 报错就说明命令行本身没拼对，根本没走到执行阶段。
/// 退出码统一 2，把"该改参数"和"命令对了但跑失败"（1）分开，脚本才能靠退出码决策
/// （20 §契约、22 §错误模型）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgError {
    pub message: String,
}

impl ArgError {
    fn usage(msg: impl Into<String>) -> ArgError {
        ArgError {
            message: msg.into(),
        }
    }

    fn value(msg: impl Into<String>) -> ArgError {
        ArgError {
            message: msg.into(),
        }
    }
}

impl From<StkError> for ArgError {
    fn from(e: StkError) -> ArgError {
        ArgError {
            message: e.to_string(),
        }
    }
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Command::Probe { .. } => "probe",
            Command::Convert { .. } => "convert",
            Command::Time { .. } => "time",
            Command::Hdr { .. } => "hdr",
            Command::PgsAdjust { .. } => "pgs adjust",
            Command::PgsValidate { .. } => "pgs validate",
            Command::PgsEncode { .. } => "pgs encode",
            Command::Doctor => "doctor",
            Command::Cache { .. } => "cache",
            Command::Settings { .. } => "settings",
            Command::Diagnose { .. } => "diagnose",
            Command::Render { .. } => "render",
            Command::Batch { .. } => "batch",
            Command::FontScan { .. } => "font scan",
            Command::FontSubset { .. } => "font subset",
            Command::Mkv { action, .. } => match action {
                MkvAction::Identify { .. } => "mkv identify",
                MkvAction::Extract { .. } => "mkv extract",
                MkvAction::Remux { .. } => "mkv remux",
            },
            Command::NotBuilt { .. } => "not-built",
        }
    }

    pub fn time_mode(&self) -> Option<TimeMode> {
        match self {
            Command::Time { mode, .. } => Some(*mode),
            _ => None,
        }
    }
}

fn parse_canvas(s: &str) -> Result<Canvas, ArgError> {
    let (w, h) = s
        .split_once(['x', 'X', ':'])
        .ok_or_else(|| ArgError::value(format!("--canvas 需要 WxH，收到 {s}")))?;
    let w = w
        .trim()
        .parse::<u32>()
        .map_err(|_| ArgError::value(format!("画布宽度非法：{w}")))?;
    let h = h
        .trim()
        .parse::<u32>()
        .map_err(|_| ArgError::value(format!("画布高度非法：{h}")))?;
    Ok(Canvas::new(w, h)?)
}

fn parse_fps(s: &str) -> Result<Fps, ArgError> {
    Ok(Fps::new(Rational::parse(s)?)?)
}

fn parse_time(s: &str) -> Result<Time, ArgError> {
    codec_ass::parse_ass_time(s).map_err(ArgError::from)
}

fn parse_bool(s: &str) -> Result<bool, ArgError> {
    match s.to_lowercase().as_str() {
        "yes" | "true" | "1" | "on" => Ok(true),
        "no" | "false" | "0" | "off" => Ok(false),
        _ => Err(ArgError::value(format!("布尔值非法：{s}"))),
    }
}

fn format_kind(s: &str) -> Result<FormatKind, ArgError> {
    match s.to_lowercase().as_str() {
        "ass" => Ok(FormatKind::Ass),
        "ssa" => Ok(FormatKind::Ssa),
        "srt" => Ok(FormatKind::Srt),
        "vtt" => Ok(FormatKind::Vtt),
        "pgs" | "sup" => Ok(FormatKind::Pgs),
        "bdn" | "xml" => Ok(FormatKind::Bdn),
        _ => Err(ArgError::value(format!("未知目标格式：{s}"))),
    }
}

fn script_policy(s: &str) -> Result<ScriptResolutionPolicy, ArgError> {
    match s.to_lowercase().as_str() {
        "preserve" => Ok(ScriptResolutionPolicy::Preserve),
        "rewrite" => Ok(ScriptResolutionPolicy::Rewrite),
        _ => Err(ArgError::value(
            "--script-policy 只能是 preserve 或 rewrite",
        )),
    }
}

fn parse_level(s: &str) -> Result<ValidationLevel, ArgError> {
    Ok(match s.to_lowercase().as_str() {
        "probe" => ValidationLevel::Probe,
        "structure" => ValidationLevel::Structure,
        "basic" => ValidationLevel::Basic,
        "standard" => ValidationLevel::Standard,
        "strict" => ValidationLevel::Strict,
        _ => {
            return Err(ArgError::value(
                "--level 只能是 probe|structure|basic|standard|strict",
            ))
        }
    })
}

fn layout_policy(s: &str) -> Result<LayoutResPolicy, ArgError> {
    match s.to_lowercase().as_str() {
        "keep-as-is" | "keep" => Ok(LayoutResPolicy::KeepAsIs),
        "scale-with-script" | "scale" => Ok(LayoutResPolicy::ScaleWithScript),
        "drop" => Ok(LayoutResPolicy::Drop),
        _ => Err(ArgError::value("--layout-policy 取值非法")),
    }
}

/// 主解析入口。
pub fn parse(argv: &[String]) -> Result<Invocation, ArgError> {
    let mut inv = Invocation {
        command: Command::Doctor,
        json: false,
        out: None,
        dry_run: false,
        overwrite: OverwritePolicy::Rename,
        inject_fonts: false,
        no_font_cache: false,
        // 默认开：纯优化，命中不改变输出字节（37 §2）。
        render_cache: true,
    };
    // 第一遍：抽走全局开关，剩下的 token 才是命令与其专属参数
    let mut tokens: Vec<String> = Vec::with_capacity(argv.len());
    let mut i = 0usize;
    while i < argv.len() {
        let a = argv[i].as_str();
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (a, None),
        };
        match name {
            "--json" => inv.json = true,
            "--dry-run" => inv.dry_run = true,
            "--no-font-cache" => inv.no_font_cache = true,
            "--no-render-cache" => inv.render_cache = false,
            "--inject-fonts" => inv.inject_fonts = true,
            "--overwrite" => {
                let v = match inline {
                    Some(v) => v,
                    None => {
                        let v = argv
                            .get(i + 1)
                            .ok_or_else(|| ArgError::usage("--overwrite 缺少取值"))?;
                        i += 1;
                        v
                    }
                };
                inv.overwrite = match v.to_ascii_lowercase().as_str() {
                    "skip" => OverwritePolicy::Skip,
                    // 这里的 rename 是"目标被占就换个名字写"，不是 POSIX 的 mv 语义
                    "rename" => OverwritePolicy::Rename,
                    "replace" => OverwritePolicy::ReplaceAfterVerify,
                    other => {
                        return Err(ArgError::value(format!(
                            "--overwrite 只能是 skip|rename|replace，收到 {other}"
                        )))
                    }
                };
            }
            "--help" | "-h" => return Err(ArgError::usage("help")),
            "--out" => match inline {
                Some(v) => inv.out = Some(PathBuf::from(v)),
                None => {
                    let v = argv
                        .get(i + 1)
                        .ok_or_else(|| ArgError::usage("--out 缺少取值"))?;
                    inv.out = Some(PathBuf::from(v));
                    i += 1;
                }
            },
            "--preset" => {
                return Err(ArgError::value(
                    "--preset 需要 TOML 解析依赖（21 §1 的宽松许可原则允许，但按 ISSUES E 段等确认）；\
                     当前请直接在命令行给出参数",
                ));
            }
            _ => tokens.push(a.to_string()),
        }
        i += 1;
    }
    if tokens.is_empty() {
        return Err(ArgError::usage("缺少命令"));
    }
    let cmd = tokens[0].as_str();
    let rest = &tokens[1..];
    inv.command = match cmd {
        "probe" => Command::Probe {
            input: one_input(rest)?,
        },
        "convert" => {
            let mut to: Option<FormatKind> = None;
            let mut canvas: Option<Canvas> = None;
            let mut sp = ScriptResolutionPolicy::Preserve;
            let mut lp = LayoutResPolicy::KeepAsIs;
            let input = scan(rest, &mut |name, value| {
                match name {
                    "--to" => to = Some(format_kind(value)?),
                    "--canvas" => canvas = Some(parse_canvas(value)?),
                    "--script-policy" => sp = script_policy(value)?,
                    "--layout-policy" => lp = layout_policy(value)?,
                    _ => return Err(ArgError::value(format!("convert 不支持开关 {name}"))),
                }
                Ok(())
            })?;
            Command::Convert {
                input,
                to: to.ok_or_else(|| ArgError::value("convert 需要 --to srt|ass|ssa"))?,
                canvas,
                script_policy: sp,
                layout_policy: lp,
            }
        }
        "time" => {
            let mut mode: Option<TimeMode> = None;
            let mut src: Option<Fps> = None;
            let mut dst: Option<Fps> = None;
            let mut off = 0i64;
            let mut tp: Option<TwoPointSpec> = None;
            let mut pw: Vec<ControlPoint> = Vec::new();
            let input = scan(rest, &mut |name, value| {
                match name {
                    "--offset-ms" => {
                        off = value
                            .parse::<i64>()
                            .map_err(|_| ArgError::value(format!("--offset-ms 非法：{value}")))?;
                        mode = Some(TimeMode::Offset);
                    }
                    "--speed" => {
                        src = Some(parse_fps(value)?);
                        mode = Some(TimeMode::SpeedScale);
                    }
                    "--to-fps" => {
                        dst = Some(parse_fps(value)?);
                    }
                    "--snap-fps" => {
                        dst = Some(parse_fps(value)?);
                        mode = Some(TimeMode::FrameSnap);
                    }
                    "--two-point" => {
                        let parts: Vec<&str> = value.split(',').map(str::trim).collect();
                        if parts.len() != 4 {
                            return Err(ArgError::value(
                                "--two-point 需要 4 个时间：源1,目标1,源2,目标2",
                            ));
                        }
                        tp = Some(TwoPointSpec {
                            source1: parse_time(parts[0])?,
                            target1: parse_time(parts[1])?,
                            source2: parse_time(parts[2])?,
                            target2: parse_time(parts[3])?,
                        });
                        mode = Some(TimeMode::TwoPoint);
                    }
                    // 10 §5 分段同步：可重复给 `--piecewise 源,目标`，相邻两点连成一段。
                    // 用可重复的小开关而不是一个长串：控制点数量不定，
                    // 而且用户是一条一条从播放器里量出来的。
                    "--piecewise" => {
                        let parts: Vec<&str> = value.split(',').map(str::trim).collect();
                        if parts.len() != 2 {
                            return Err(ArgError::value(
                                "--piecewise 需要 2 个时间：源,目标（可重复给多段）",
                            ));
                        }
                        pw.push(ControlPoint::new(
                            parse_time(parts[0])?,
                            parse_time(parts[1])?,
                        ));
                        mode = Some(TimeMode::Piecewise);
                    }
                    _ => return Err(ArgError::value(format!("time 不支持开关 {name}"))),
                }
                Ok(())
            })?;
            let mode = mode.ok_or_else(|| {
                ArgError::value(
                    "time 需要 --offset-ms / --speed / --snap-fps / --two-point / --piecewise 之一",
                )
            })?;
            if mode == TimeMode::Piecewise && pw.len() < 2 {
                return Err(ArgError::value(format!(
                    "--piecewise 至少需要 2 个控制点（相邻两点连成一段），收到 {}",
                    pw.len()
                )));
            }
            if mode == TimeMode::SpeedScale && (src.is_none() || dst.is_none()) {
                return Err(ArgError::value(
                    "speed 模式需要 --speed SRC 与 --to-fps DST（两者都是有理数 FPS，ADR-007）",
                ));
            }
            Command::Time {
                input,
                mode,
                source_fps: src,
                target_fps: dst,
                offset_ms: off,
                two_point: tp,
                piecewise: PiecewiseSpec::new(pw),
            }
        }
        "hdr" => {
            let mut settings = HdrSettings::default();
            let mut alpha = false;
            let input = scan(rest, &mut |name, value| {
                match name {
                    "--hdr-mode" => {
                        settings.mode = match value.to_lowercase().as_str() {
                            "compatibility" => HdrMode::Compatibility,
                            "percentage" => HdrMode::Percentage,
                            "reference-white" | "reference_white" => HdrMode::ReferenceWhite,
                            "off" => HdrMode::Off,
                            _ => return Err(ArgError::value(format!("未知 --hdr-mode：{value}"))),
                        };
                    }
                    "--nits" => {
                        settings.reference_nits = value
                            .parse::<u32>()
                            .map_err(|_| ArgError::value("reference nits 必须是整数"))?;
                    }
                    "--percent-basis-points" | "--bp" => {
                        settings.percent_basis_points = value
                            .parse::<u32>()
                            .map_err(|_| ArgError::value("万分比必须是整数"))?;
                    }
                    "--touch-alpha" => alpha = true,
                    "--touch-styles" => settings.touch_styles = parse_bool(value)?,
                    "--touch-inline" => settings.touch_inline = parse_bool(value)?,
                    _ => return Err(ArgError::value(format!("hdr 不支持开关 {name}"))),
                }
                Ok(())
            })?;
            settings.touch_alpha = alpha;
            if settings.mode == HdrMode::Off {
                return Err(ArgError::value("hdr 需要显式 --hdr-mode"));
            }
            Command::Hdr { input, settings }
        }
        "pgs" => {
            let sub = rest.first().map(String::as_str).unwrap_or("");
            let tail = rest.get(1..).unwrap_or(&[]);
            match sub {
                "adjust" => {
                    let mut ops: Vec<FastOp> = Vec::new();
                    let input = scan(tail, &mut |name, value| {
                        match name {
                            "--offset-ms" => ops.push(FastOp::OffsetMs(
                                value
                                    .parse::<i64>()
                                    .map_err(|_| ArgError::value("offset 必须是整数毫秒"))?,
                            )),
                            "--scale" => ops.push(FastOp::ScaleTime(Rational::parse(value)?)),
                            "--move" => {
                                let mut parts = value.split(',');
                                let dx = parts
                                    .next()
                                    .and_then(|s| s.trim().parse::<i32>().ok())
                                    .ok_or_else(|| ArgError::value("--move 需要 DX,DY"))?;
                                let dy = parts
                                    .next()
                                    .and_then(|s| s.trim().parse::<i32>().ok())
                                    .ok_or_else(|| ArgError::value("--move 需要 DX,DY"))?;
                                ops.push(FastOp::MoveBy(dx, dy));
                            }
                            "--forced" => ops.push(FastOp::SetForced(parse_bool(value)?)),
                            "--luma-basis-points" => ops.push(FastOp::PaletteLuma(
                                value
                                    .parse::<i32>()
                                    .map_err(|_| ArgError::value("万分比必须是整数"))?,
                            )),
                            _ => {
                                return Err(ArgError::value(format!(
                                    "pgs adjust 不支持开关 {name}"
                                )))
                            }
                        }
                        Ok(())
                    })?;
                    if ops.is_empty() {
                        return Err(ArgError::value("pgs adjust 至少需要一个变换操作"));
                    }
                    Command::PgsAdjust { input, ops }
                }
                "validate" => {
                    let mut level = ValidationLevel::Standard;
                    let mut peak: Option<i64> = None;
                    let input = scan(tail, &mut |name, value| {
                        match name {
                            "--level" => level = parse_level(value)?,
                            "--peak-bitrate-bps" => {
                                peak = Some(value.parse::<i64>().map_err(|_| {
                                    ArgError::value("--peak-bitrate-bps 必须是整数 bit/s")
                                })?);
                            }
                            _ => {
                                return Err(ArgError::value(format!(
                                    "pgs validate 不支持开关 {name}"
                                )))
                            }
                        }
                        Ok(())
                    })?;
                    Command::PgsValidate {
                        input,
                        level,
                        peak_bps: peak,
                    }
                }
                "encode" => {
                    let mut fps = Fps::NTSC_23976;
                    let mut canvas = Canvas::FHD1080;
                    let mut level = ValidationLevel::Standard;
                    let mut dll: Option<PathBuf> = None;
                    let mut font = "Arial".to_string();
                    let mut exact = false;
                    let mut snap = false;
                    let input = scan(tail, &mut |name, value| {
                        match name {
                            "--fps" => fps = parse_fps(value)?,
                            "--canvas" => canvas = parse_canvas(value)?,
                            "--level" => level = parse_level(value)?,
                            "--libass-dll" => dll = Some(PathBuf::from(value)),
                            "--default-font" => font = value.to_string(),
                            "--exact-colors" => exact = true,
                            "--snap-pts" => snap = true,
                            _ => {
                                return Err(ArgError::value(format!(
                                    "pgs encode 不支持开关 {name}"
                                )))
                            }
                        }
                        Ok(())
                    })?;
                    // 采样网格与输出画布都必须是 16:9 profile（13 §4：composition canvas
                    // 始终是完整画布），非 16:9 在这里就拒绝，而不是让 PGS 段流带病出门
                    if !canvas.is_16_9() {
                        return Err(ArgError::value(format!(
                            "PGS composition canvas 必须是 16:9 profile，收到 {}",
                            canvas.label()
                        )));
                    }
                    Command::PgsEncode {
                        input,
                        fps,
                        canvas,
                        level,
                        dll,
                        default_font: font,
                        exact_colors: exact,
                        snap_pts: snap,
                    }
                }
                other => {
                    return Err(ArgError::usage(format!(
                        "pgs 子命令只能是 adjust|validate|encode，收到 {other}"
                    )));
                }
            }
        }
        "doctor" => {
            if !rest.is_empty() {
                return Err(ArgError::usage("doctor 不接受位置参数"));
            }
            Command::Doctor
        }
        "cache" => {
            let action = match rest.first().map(|s| s.as_str()).unwrap_or("stat") {
                "stat" => CacheAction::Stat,
                "clear" => CacheAction::Clear,
                other => {
                    return Err(ArgError::usage(format!(
                        "cache 的子命令只能是 stat 或 clear，收到 {other}"
                    )));
                }
            };
            if rest.len() > 1 {
                return Err(ArgError::usage("cache 最多接一个子命令"));
            }
            Command::Cache { action }
        }
        "settings" => {
            let action = match rest.first().map(|s| s.as_str()).unwrap_or("show") {
                "show" => SettingsAction::Show,
                "init" => SettingsAction::Init,
                "set" => SettingsAction::Set,
                other => {
                    return Err(ArgError::usage(format!(
                        "settings 的子命令只能是 show、init 或 set，收到 {other}"
                    )));
                }
            };
            let (key, value) = match action {
                SettingsAction::Set => {
                    if rest.len() != 3 {
                        return Err(ArgError::usage(
                            "settings set 需要两个参数：字段名与值（例：settings set log_level debug）",
                        ));
                    }
                    (Some(rest[1].clone()), Some(rest[2].clone()))
                }
                _ => {
                    if rest.len() > 1 {
                        return Err(ArgError::usage("settings show/init 不接受额外参数"));
                    }
                    (None, None)
                }
            };
            Command::Settings { action, key, value }
        }
        "diagnose" => {
            let mut include_input = Vec::new();
            let mut job_state = None;
            let mut it = rest.iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--include-input" => match it.next() {
                        Some(v) => include_input.push(PathBuf::from(v)),
                        None => {
                            return Err(ArgError::usage("--include-input 后面要跟一个文件路径"))
                        }
                    },
                    "--job-state" => match it.next() {
                        Some(v) => job_state = Some(PathBuf::from(v)),
                        None => return Err(ArgError::usage("--job-state 后面要跟一个文件路径")),
                    },
                    other => {
                        return Err(ArgError::usage(format!(
                            "diagnose 不认识参数 {other:?}（可用：--include-input FILE、--job-state FILE）"
                        )))
                    }
                }
            }
            Command::Diagnose {
                include_input,
                job_state,
            }
        }
        "render" => {
            let mut fps = Fps::NTSC_23976;
            let mut canvas = Canvas::FHD1080;
            let mut dll: Option<PathBuf> = None;
            let mut font = "Arial".to_string();
            let mut min_axis = 0u32;
            let input = scan(rest, &mut |name, value| {
                match name {
                    "--fps" => fps = parse_fps(value)?,
                    "--canvas" => canvas = parse_canvas(value)?,
                    "--libass-dll" => dll = Some(PathBuf::from(value)),
                    "--default-font" => font = value.to_string(),
                    "--min-axis" => {
                        min_axis = value
                            .parse::<u32>()
                            .map_err(|_| ArgError::value("--min-axis 需要非负整数"))?
                    }
                    _ => return Err(ArgError::value(format!("render 不支持开关 {name}"))),
                }
                Ok(())
            })?;
            if !canvas.is_16_9() {
                return Err(ArgError::value(format!(
                    "BDN 输出画布必须是完整 16:9，收到 {}",
                    canvas.label()
                )));
            }
            Command::Render {
                input,
                fps,
                canvas,
                dll,
                default_font: font,
                min_axis,
            }
        }
        "batch" => {
            let mut spec: Option<PathBuf> = None;
            let mut i = 0usize;
            while i < rest.len() {
                let a = rest[i].as_str();
                let (name, inline) = match a.split_once('=') {
                    Some((n, v)) if n.starts_with("--") => (n, Some(v)),
                    _ => (a, None),
                };
                match name {
                    // 取值要就地吃掉，否则它会被下一轮当成"位置参数"——
                    // 这条在实测里把 batch 全部挡在了解析阶段（退出码 2）
                    "--spec" => {
                        let v = match inline {
                            Some(v) => v.to_string(),
                            None => {
                                let v = rest
                                    .get(i + 1)
                                    .ok_or_else(|| ArgError::usage("--spec 缺少文件路径"))?;
                                i += 1;
                                v.clone()
                            }
                        };
                        spec = Some(PathBuf::from(v));
                    }
                    other if other.starts_with("--") => {
                        return Err(ArgError::value(format!("batch 不支持开关 {other}")));
                    }
                    other => {
                        // batch 只吃 --spec：给它一个"看起来像输入文件"的东西，
                        // 会显得跑了其实什么都没做
                        return Err(ArgError::usage(format!(
                            "batch 不接受位置参数 {other}，请用 --spec FILE 指向任务描述"
                        )));
                    }
                }
                i += 1;
            }
            Command::Batch {
                spec: spec.ok_or_else(|| {
                    ArgError::usage("batch 需要 --spec FILE（15 §1 的 job 描述）")
                })?,
            }
        }
        "font" => {
            let sub = rest.first().map(String::as_str).unwrap_or("");
            let tail = rest.get(1..).unwrap_or(&[]);
            match sub {
                "scan" => {
                    let mut dirs: Vec<PathBuf> = Vec::new();
                    let mut registry = true;
                    let mut copy_to: Option<PathBuf> = None;
                    let input = scan(tail, &mut |name, value| {
                        match name {
                            "--fonts-dir" => dirs.push(PathBuf::from(value)),
                            "--no-registry" => registry = false,
                            "--copy-fonts" => copy_to = Some(PathBuf::from(value)),
                            _ => {
                                return Err(ArgError::value(format!("font scan 不支持开关 {name}")))
                            }
                        }
                        Ok(())
                    })?;
                    if let Some(p) = &copy_to {
                        if !p.is_absolute() {
                            return Err(ArgError::value(format!(
                                "--copy-fonts 需要绝对路径（输出事务只对绝对路径负责），收到 {}",
                                p.display()
                            )));
                        }
                    }
                    Command::FontScan {
                        input,
                        fonts_dirs: dirs,
                        registry,
                        copy_fonts: copy_to,
                    }
                }
                "subset" => {
                    let mut dirs: Vec<PathBuf> = Vec::new();
                    let mut registry = true;
                    let mut subset_dir: Option<PathBuf> = None;
                    let mut fallback_full = false;
                    let mut embed_fonts = false;
                    let mut mkv_attachments = false;
                    let input = scan(tail, &mut |name, value| {
                        match name {
                            "--fonts-dir" => dirs.push(PathBuf::from(value)),
                            "--no-registry" => registry = false,
                            "--subset-dir" => subset_dir = Some(PathBuf::from(value)),
                            "--fallback-full" => fallback_full = true,
                            "--embed-fonts" => embed_fonts = true,
                            "--mkv-attachments" => mkv_attachments = true,
                            _ => {
                                return Err(ArgError::value(format!(
                                    "font subset 不支持开关 {name}"
                                )))
                            }
                        }
                        Ok(())
                    })?;
                    if let Some(p) = &subset_dir {
                        if !p.is_absolute() {
                            return Err(ArgError::value(format!(
                                "--subset-dir 需要绝对路径，收到 {}",
                                p.display()
                            )));
                        }
                    }
                    Command::FontSubset {
                        input,
                        fonts_dirs: dirs,
                        registry,
                        subset_dir,
                        fallback_full,
                        embed_fonts,
                        mkv_attachments,
                    }
                }
                other => {
                    return Err(ArgError::usage(format!(
                        "font 子命令未知：{other}（可用：scan|subset）"
                    )))
                }
            }
        }
        "mkv" => {
            let sub = rest.first().map(String::as_str).unwrap_or("");
            let tail = rest.get(1..).unwrap_or(&[]);
            let action =
                match sub {
                    "identify" => {
                        let mut tools_dir = None;
                        let input = scan(tail, &mut |name, value| match name {
                            "--tools-dir" => {
                                tools_dir = Some(PathBuf::from(value));
                                Ok(())
                            }
                            _ => Err(ArgError::value(format!("mkv identify 不支持开关 {name}"))),
                        })?;
                        (MkvAction::Identify { input }, tools_dir)
                    }
                    "extract" => {
                        let mut tracks: Vec<(u64, PathBuf)> = Vec::new();
                        let mut attachments: Vec<(u64, PathBuf)> = Vec::new();
                        let mut tools_dir = None;
                        let input = scan(tail, &mut |name, value| match name {
                            "--track" => {
                                tracks.push(parse_id_path(value, "--track")?);
                                Ok(())
                            }
                            "--attachment" => {
                                attachments.push(parse_id_path(value, "--attachment")?);
                                Ok(())
                            }
                            "--tools-dir" => {
                                tools_dir = Some(PathBuf::from(value));
                                Ok(())
                            }
                            _ => Err(ArgError::value(format!("mkv extract 不支持开关 {name}"))),
                        })?;
                        if tracks.is_empty() && attachments.is_empty() {
                            return Err(ArgError::usage(
                                "mkv extract 至少要一个 --track ID:OUT 或 --attachment ID:OUT",
                            ));
                        }
                        (
                            MkvAction::Extract {
                                input,
                                tracks,
                                attachments,
                            },
                            tools_dir,
                        )
                    }
                    "remux" => {
                        let mut subtitle_tracks: Vec<u64> = Vec::new();
                        let mut all_subtitles = false;
                        let mut keep_video = true;
                        let mut keep_audio = true;
                        let mut attach_fonts: Vec<PathBuf> = Vec::new();
                        let mut track_name: Vec<(u64, String)> = Vec::new();
                        let mut track_language: Vec<(u64, String)> = Vec::new();
                        let mut default_track: Vec<u64> = Vec::new();
                        let mut forced_track: Vec<u64> = Vec::new();
                        let mut tools_dir = None;
                        let input =
                            scan(tail, &mut |name, value| {
                                match name {
                                    "--out" => return Err(ArgError::usage(
                                        "--out 是全局开关：写成 mkv remux in.mkv --out o.mkv 即可",
                                    )),
                                    "--subtitle-tracks" => subtitle_tracks
                                        .extend(parse_id_list(value, "--subtitle-tracks")?),
                                    "--all-subtitles" => all_subtitles = true,
                                    "--drop-video" => keep_video = false,
                                    "--drop-audio" => keep_audio = false,
                                    "--attach-font" => attach_fonts.push(PathBuf::from(value)),
                                    "--track-name" => {
                                        track_name.push(parse_u64_str(value, "--track-name")?)
                                    }
                                    "--track-language" => track_language
                                        .push(parse_u64_str(value, "--track-language")?),
                                    "--default-track" => default_track
                                        .push(parse_u64_only(value, "--default-track")?),
                                    "--forced-track" => {
                                        forced_track.push(parse_u64_only(value, "--forced-track")?)
                                    }
                                    "--tools-dir" => tools_dir = Some(PathBuf::from(value)),
                                    _ => {
                                        return Err(ArgError::value(format!(
                                            "mkv remux 不支持开关 {name}"
                                        )))
                                    }
                                }
                                Ok(())
                            })?;
                        if all_subtitles && !subtitle_tracks.is_empty() {
                            return Err(ArgError::value(
                                "--all-subtitles 与 --subtitle-tracks 不能同时给（语义冲突）",
                            ));
                        }
                        (
                            MkvAction::Remux {
                                input,
                                subtitle_tracks,
                                all_subtitles,
                                keep_video,
                                keep_audio,
                                attach_fonts,
                                track_name,
                                track_language,
                                default_track,
                                forced_track,
                            },
                            tools_dir,
                        )
                    }
                    other => {
                        return Err(ArgError::usage(format!(
                            "mkv 子命令未知：{other}（可用：identify|extract|remux）"
                        )))
                    }
                };
            Command::Mkv {
                action: action.0,
                tools_dir: action.1,
            }
        }
        other => return Err(ArgError::usage(format!("未知命令：{other}"))),
    };
    Ok(inv)
}

/// `ID:OUT` —— mkv 的轨道/附件选择。id 必须能解析成数字：一个能写法错的字段
/// 在解析层挡住，比跑到 mkvextract 才因为拼错而拿到二进制垃圾桶要好。
fn parse_id_path(value: &str, flag: &str) -> Result<(u64, PathBuf), ArgError> {
    let (id, path) = value
        .split_once(':')
        .ok_or_else(|| ArgError::value(format!("{flag} 需要 ID:OUT 形式，收到 {value}")))?;
    let id: u64 = id
        .trim()
        .parse()
        .map_err(|_| ArgError::value(format!("{flag} 的 ID 必须是数字，收到 {id}")))?;
    if path.is_empty() {
        return Err(ArgError::value(format!("{flag} 缺少输出路径")));
    }
    Ok((id, PathBuf::from(path)))
}

/// `1,3,5` —— 轨道 id 列表。
fn parse_id_list(value: &str, flag: &str) -> Result<Vec<u64>, ArgError> {
    if value.trim().is_empty() {
        return Err(ArgError::value(format!("{flag} 至少要给一个轨道 id")));
    }
    value
        .split(',')
        .map(|s| {
            s.trim()
                .parse::<u64>()
                .map_err(|_| ArgError::value(format!("{flag} 里的轨道 id 必须是数字：{s}")))
        })
        .collect()
}

/// `ID` 单独一个数字。
fn parse_u64_only(value: &str, flag: &str) -> Result<u64, ArgError> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| ArgError::value(format!("{flag} 需要轨道 id（数字），收到 {value}")))
}

/// `ID:VALUE` —— 轨道 id 加一个字符串（名称/语言），值里允许有冒号（名称常带）。
fn parse_u64_str(value: &str, flag: &str) -> Result<(u64, String), ArgError> {
    let (id, rest) = value
        .split_once(':')
        .ok_or_else(|| ArgError::value(format!("{flag} 需要 ID:VALUE 形式，收到 {value}")))?;
    let id = parse_u64_only(id, flag)?;
    if rest.is_empty() {
        return Err(ArgError::value(format!("{flag} 的值不能为空")));
    }
    Ok((id, rest.to_string()))
}

/// 把一个命令登记为"已规划但未实现"。
///
/// **当前没有任何命令用它**（2026-10-17：`font subset` 与 `mkv` 落地后
/// `doctor` 的 `not_built` 为空）。保留它是因为：
/// * `COMMAND_CATALOG` 上加一条新命令而还没实现时，这是唯一的正确写法；
/// * `not_built_commands()` 靠它推导清单，删掉它就等于删掉那条门禁的入口。
/// * `--preset` 仍旧走解析层的明确拒绝（它不是命令，所以不在这里）。
#[allow(dead_code)]
fn not_built(name: &str, phase: &str) -> Command {
    Command::NotBuilt {
        name: name.to_string(),
        phase: phase.to_string(),
    }
}

/// 命令专属开关扫描：位置参数里第一个就是输入文件。
/// 支持 `--flag value` 与 `--flag=value`，并识别无值开关。
fn scan(
    args: &[String],
    f: &mut dyn FnMut(&str, &str) -> Result<(), ArgError>,
) -> Result<PathBuf, ArgError> {
    let mut input: Option<PathBuf> = None;
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i].as_str();
        if a.starts_with("--") {
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (a, None),
            };
            if SWITCHES.contains(&name) {
                f(name, "")?;
                i += 1;
                continue;
            }
            let value = match inline {
                Some(v) => v,
                None => {
                    let v = args
                        .get(i + 1)
                        .ok_or_else(|| ArgError::usage(format!("{name} 缺少取值")))?;
                    if v.starts_with("--") {
                        return Err(ArgError::usage(format!("{name} 缺少取值")));
                    }
                    v.as_str()
                }
            };
            f(name, value)?;
            i += if inline.is_some() { 1 } else { 2 };
            continue;
        }
        if a.starts_with('-') && a.len() > 1 {
            return Err(ArgError::value(format!("未知开关：{a}")));
        }
        if input.is_some() {
            return Err(ArgError::usage("只接受一个输入文件"));
        }
        input = Some(PathBuf::from(a));
        i += 1;
    }
    input.ok_or_else(|| ArgError::usage("缺少输入文件"))
}

fn one_input(rest: &[String]) -> Result<PathBuf, ArgError> {
    let mut iter = rest.iter().filter(|a| !a.starts_with('-'));
    let first = iter.next().ok_or_else(|| ArgError::usage("缺少输入文件"))?;
    if iter.next().is_some() {
        return Err(ArgError::usage("这个命令只接受一个输入文件"));
    }
    Ok(PathBuf::from(first))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn global_flags_work_before_and_after_the_command() {
        for order in [
            argv(&["probe", "a.srt", "--json", "--dry-run"]),
            argv(&["--json", "probe", "a.srt", "--dry-run"]),
            argv(&["--dry-run", "--json", "probe", "a.srt"]),
        ] {
            let inv = parse(&order).unwrap();
            assert!(inv.json && inv.dry_run, "{order:?}");
            assert_eq!(
                inv.command,
                Command::Probe {
                    input: PathBuf::from("a.srt")
                }
            );
        }
    }

    /// `--inject-fonts` 改变的是像素的来路，所以每条会渲染的命令都要认它；
    /// 默认必须是关 —— 开着它渲染等于换了一份字体来源，不能悄悄发生。
    #[test]
    fn inject_fonts_is_accepted_by_every_rendering_command() {
        for order in [
            argv(&["render", "a.ass", "--inject-fonts"]),
            argv(&["--inject-fonts", "render", "a.ass"]),
            argv(&["pgs", "encode", "a.ass", "--inject-fonts"]),
            argv(&["batch", "--spec", "s.json", "--inject-fonts"]),
        ] {
            let inv = parse(&order).unwrap();
            assert!(inv.inject_fonts, "{order:?}");
        }
        let plain = parse(&argv(&["render", "a.ass"])).unwrap();
        assert!(!plain.inject_fonts, "注入必须显式开启");
    }

    /// "不要缓存"必须每个命令都认得：它是"怀疑缓存时那条独立可复现的路"，
    /// 只在某一个命令生效就等于没有。
    #[test]
    fn no_font_cache_is_a_global_switch() {
        for order in [
            argv(&["--no-font-cache", "font", "scan", "x.ass"]),
            argv(&["font", "scan", "x.ass", "--no-font-cache"]),
            argv(&["--no-font-cache", "probe", "a.srt"]),
            argv(&["--no-font-cache", "batch", "--spec", "s.json"]),
        ] {
            let inv = parse(&order).unwrap();
            assert!(inv.no_font_cache, "{order:?}");
        }
        let plain = parse(&argv(&["font", "scan", "x.ass"])).unwrap();
        assert!(!plain.no_font_cache, "默认该用缓存，不然这个开关没有意义");
    }

    #[test]
    fn out_flag_is_global_and_not_rejected_by_subcommands() {
        let inv = parse(&argv(&[
            "convert", "x.ass", "--to", "srt", "--out", "y.srt", "--json",
        ]))
        .unwrap();
        assert_eq!(inv.out, Some(PathBuf::from("y.srt")));
        assert!(inv.json);
        match inv.command {
            Command::Convert {
                to,
                canvas,
                script_policy,
                layout_policy,
                ..
            } => {
                assert_eq!(to, FormatKind::Srt);
                assert!(canvas.is_none());
                assert_eq!(script_policy, ScriptResolutionPolicy::Preserve);
                assert_eq!(layout_policy, LayoutResPolicy::KeepAsIs);
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn convert_carries_resolution_policy() {
        let inv = parse(&argv(&[
            "convert",
            "x.ass",
            "--to",
            "srt",
            "--canvas",
            "3840x2160",
            "--script-policy",
            "rewrite",
            "--layout-policy",
            "drop",
        ]))
        .unwrap();
        match inv.command {
            Command::Convert {
                canvas,
                script_policy,
                layout_policy,
                ..
            } => {
                assert_eq!(canvas.unwrap().label(), "3840x2160");
                assert_eq!(script_policy, ScriptResolutionPolicy::Rewrite);
                assert_eq!(layout_policy, LayoutResPolicy::Drop);
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn inline_equals_form_is_accepted() {
        let inv = parse(&argv(&["convert", "--to=srt", "a.ass"])).unwrap();
        match inv.command {
            Command::Convert { input, to, .. } => {
                assert_eq!(input, PathBuf::from("a.ass"));
                assert_eq!(to, FormatKind::Srt);
            }
            c => panic!("{c:?}"),
        }
        let inv = parse(&argv(&[
            "convert",
            "a.ass",
            "--canvas=1920x1080",
            "--to=ass",
        ]))
        .unwrap();
        match inv.command {
            Command::Convert { canvas, .. } => assert_eq!(canvas.unwrap().label(), "1920x1080"),
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn time_modes_are_parsed() {
        let inv = parse(&argv(&["time", "x.srt", "--offset-ms", "-500"])).unwrap();
        assert_eq!(inv.command.time_mode(), Some(TimeMode::Offset));
        let inv = parse(&argv(&[
            "time",
            "x.ass",
            "--speed",
            "24000/1001",
            "--to-fps",
            "25/1",
        ]))
        .unwrap();
        match inv.command {
            Command::Time {
                mode,
                source_fps,
                target_fps,
                ..
            } => {
                assert_eq!(mode, TimeMode::SpeedScale);
                assert_eq!(source_fps.unwrap().nominal().unwrap(), 24);
                assert_eq!(target_fps.unwrap().nominal().unwrap(), 25);
            }
            c => panic!("{c:?}"),
        }
        let inv = parse(&argv(&["time", "x.srt", "--snap-fps", "24000/1001"])).unwrap();
        assert_eq!(inv.command.time_mode(), Some(TimeMode::FrameSnap));
        let e = parse(&argv(&["time", "x.ass", "--speed", "24/1"])).unwrap_err();
        assert!(e.message.contains("--to-fps"), "{}", e.message);
    }

    #[test]
    fn two_point_uses_ass_time_not_millis() {
        let inv = parse(&argv(&[
            "time",
            "x.srt",
            "--two-point",
            "0:00:01.00,0:00:02.00,0:00:03.00,0:00:05.00",
        ]))
        .unwrap();
        match inv.command {
            Command::Time {
                two_point, mode, ..
            } => {
                assert_eq!(mode, TimeMode::TwoPoint);
                let tp = two_point.unwrap();
                assert_eq!(tp.source1, Time::from_hms_millis(0, 0, 1, 0).unwrap());
                assert_eq!(tp.target2, Time::from_hms_millis(0, 0, 5, 0).unwrap());
            }
            c => panic!("{c:?}"),
        }
        let e = parse(&argv(&["time", "x.srt", "--two-point", "1,2,3"])).unwrap_err();
        assert!(e.message.contains("4 个时间"));
    }

    #[test]
    fn pgs_adjust_collects_multiple_ops() {
        let inv = parse(&argv(&[
            "pgs",
            "adjust",
            "x.sup",
            "--offset-ms",
            "1000",
            "--scale",
            "25/24",
            "--move",
            "10,-20",
            "--forced",
            "yes",
            "--luma-basis-points",
            "8000",
        ]))
        .unwrap();
        match inv.command {
            Command::PgsAdjust { ops, .. } => assert_eq!(ops.len(), 5),
            c => panic!("{c:?}"),
        }
        let e = parse(&argv(&["pgs", "adjust", "x.sup"])).unwrap_err();
        assert!(e.message.contains("至少需要"));
        assert!(parse(&argv(&["pgs", "bogus", "x.sup"])).is_err());
    }

    #[test]
    fn pgs_validate_takes_input_after_subcommand() {
        let inv = parse(&argv(&["pgs", "validate", "x.sup", "--json"])).unwrap();
        assert_eq!(
            inv.command,
            Command::PgsValidate {
                input: PathBuf::from("x.sup"),
                level: ValidationLevel::Standard,
                peak_bps: None,
            }
        );
        assert!(inv.json);
    }

    /// `--fonts-dir` 可重复：17 §2 明说系统目录不足以覆盖注册字体，
    /// 用户一定要把额外目录一起交进来。
    #[test]
    fn font_scan_collects_repeated_font_dirs_and_rejects_stray_flags() {
        let inv = parse(&argv(&[
            "font",
            "scan",
            "x.ass",
            "--fonts-dir",
            "D:\\fonts",
            "--fonts-dir=C:\\extra",
        ]))
        .unwrap();
        assert_eq!(
            inv.command,
            Command::FontScan {
                input: PathBuf::from("x.ass"),
                fonts_dirs: vec![PathBuf::from("D:\\fonts"), PathBuf::from("C:\\extra")],
                registry: true,
                copy_fonts: None,
            }
        );
        // 注册表那块来源要能明确关掉：别的机器上一次都读不到时，代价只是多花点时间
        let off = parse(&argv(&["font", "scan", "x.ass", "--no-registry"])).unwrap();
        match &off.command {
            Command::FontScan { registry, .. } => assert!(!registry, "默认是开，必须能关：{off:?}"),
            other => panic!("{other:?}"),
        }
        // 复制目标走的是输出事务，绝对路径这条规矩在解析层就挡住，而不是写盘时才失败
        let rel = parse(&argv(&[
            "font",
            "scan",
            "x.ass",
            "--copy-fonts",
            "out/fonts",
        ]))
        .unwrap_err();
        assert!(rel.message.contains("绝对路径"), "{rel:?}");
        let abs = parse(&argv(&[
            "font",
            "scan",
            "x.ass",
            "--copy-fonts",
            "D:\\deliver",
        ]))
        .unwrap();
        match &abs.command {
            Command::FontScan { copy_fonts, .. } => {
                assert_eq!(
                    copy_fonts.as_deref(),
                    Some(std::path::Path::new("D:\\deliver"))
                )
            }
            other => panic!("{other:?}"),
        }
        // 没有 --fonts-dir 也合法：走默认目录（解析器不替执行层决定）
        let bare = parse(&argv(&["font", "scan", "x.ass"])).unwrap();
        match &bare.command {
            Command::FontScan { fonts_dirs, .. } => assert!(fonts_dirs.is_empty()),
            other => panic!("应为 FontScan：{other:?}"),
        }
        assert!(parse(&argv(&["font", "scan", "x.ass", "--fps", "24"])).is_err());
        assert!(parse(&argv(&["font", "bogus", "x.ass"])).is_err());
        // scan 需要一个输入文件，不能空跑
        assert!(parse(&argv(&["font", "scan"])).is_err());
    }

    /// 18 §4 的"绝不原地修改"在解析层就要能表达出来：`mkv remux` 必须带 `--out`，
    /// 否则它只能写回源文件，而那是规格明禁的。
    #[test]
    fn mkv_remux_requires_out_and_rejects_conflicting_subtitle_selectors() {
        let inv = parse(&argv(&[
            "mkv",
            "remux",
            "in.mkv",
            "--out",
            "out.mkv",
            "--subtitle-tracks",
            "2,3",
            "--attach-font",
            "f.ttf",
        ]))
        .unwrap();
        assert_eq!(inv.command.name(), "mkv remux");
        assert_eq!(inv.out.as_deref(), Some(std::path::Path::new("out.mkv")));
        match inv.command {
            Command::Mkv {
                action:
                    MkvAction::Remux {
                        subtitle_tracks,
                        attach_fonts,
                        keep_video,
                        keep_audio,
                        ..
                    },
                ..
            } => {
                assert_eq!(subtitle_tracks, vec![2, 3]);
                assert_eq!(attach_fonts, vec![PathBuf::from("f.ttf")]);
                assert!(keep_video && keep_audio, "默认不该丢视频/音频");
            }
            other => panic!("{other:?}"),
        }
        // 没有 --out 时解析本身合法（--out 是全局开关），但执行层必须拒；
        // 这里只校验解析结果里确实没有输出路径
        let no_out = parse(&argv(&["mkv", "remux", "in.mkv"])).unwrap();
        assert_eq!(no_out.out, None);
        // --all-subtitles 与 --subtitle-tracks 语义冲突
        let e = parse(&argv(&[
            "mkv",
            "remux",
            "in.mkv",
            "--out",
            "o.mkv",
            "--all-subtitles",
            "--subtitle-tracks",
            "2",
        ]))
        .unwrap_err();
        assert!(e.message.contains("不能同时"), "{e:?}");
    }

    #[test]
    fn mkv_extract_needs_ids_and_parses_them_strictly() {
        let inv = parse(&argv(&[
            "mkv",
            "extract",
            "in.mkv",
            "--track",
            "2:sub.ass",
            "--attachment",
            "1:font.ttf",
        ]))
        .unwrap();
        match inv.command {
            Command::Mkv {
                action:
                    MkvAction::Extract {
                        tracks,
                        attachments,
                        ..
                    },
                ..
            } => {
                assert_eq!(tracks, vec![(2, PathBuf::from("sub.ass"))]);
                assert_eq!(attachments, vec![(1, PathBuf::from("font.ttf"))]);
            }
            other => panic!("{other:?}"),
        }
        // 一个也不选就是空跑，必须在解析层报用法错而不是静默成功
        assert!(parse(&argv(&["mkv", "extract", "in.mkv"])).is_err());
        // id 不是数字：挡住，别把拼写错误带到 mkvextract
        assert!(parse(&argv(&["mkv", "extract", "in.mkv", "--track", "x:sub.ass"])).is_err());
        // 缺冒号
        assert!(parse(&argv(&["mkv", "extract", "in.mkv", "--track", "2"])).is_err());
    }

    #[test]
    fn mkv_identify_takes_an_input_and_an_optional_tools_dir() {
        let inv = parse(&argv(&[
            "mkv",
            "identify",
            "in.mkv",
            "--tools-dir",
            "D:\\mkt",
        ]))
        .unwrap();
        assert_eq!(inv.command.name(), "mkv identify");
        match inv.command {
            Command::Mkv {
                action: MkvAction::Identify { input },
                tools_dir,
            } => {
                assert_eq!(input, PathBuf::from("in.mkv"));
                assert_eq!(tools_dir, Some(PathBuf::from("D:\\mkt")));
            }
            other => panic!("{other:?}"),
        }
        assert!(parse(&argv(&["mkv", "bogus", "in.mkv"])).is_err());
        assert!(parse(&argv(&["mkv", "identify"])).is_err());
    }

    /// `font subset` 的解析：它已落地（17 §5），必须走真命令而不是占位。
    #[test]
    fn font_subset_parses_as_a_real_command() {
        let inv = parse(&argv(&[
            "font",
            "subset",
            "x.ass",
            "--subset-dir",
            "D:\\out",
            "--fallback-full",
        ]))
        .unwrap();
        assert_eq!(inv.command.name(), "font subset");
        match inv.command {
            Command::FontSubset {
                subset_dir,
                fallback_full,
                registry,
                ..
            } => {
                assert_eq!(subset_dir, Some(PathBuf::from("D:\\out")));
                assert!(fallback_full);
                assert!(registry, "默认读注册表，与 font scan 一致");
            }
            other => panic!("{other:?}"),
        }
        // 相对路径的子集目录要在解析层挡住（输出事务只对绝对路径负责）
        let e = parse(&argv(&[
            "font",
            "subset",
            "x.ass",
            "--subset-dir",
            "rel/out",
        ]))
        .unwrap_err();
        assert!(e.message.contains("绝对路径"), "{e:?}");
        // 默认不整体拒绝回退
        let bare = parse(&argv(&["font", "subset", "x.ass"])).unwrap();
        match bare.command {
            Command::FontSubset {
                fallback_full,
                subset_dir,
                ..
            } => {
                assert!(!fallback_full);
                assert_eq!(subset_dir, None);
            }
            other => panic!("{other:?}"),
        }
        assert!(parse(&argv(&["font", "subset", "x.ass", "--bogus", "1"])).is_err());
    }

    #[test]
    fn unimplemented_commands_are_not_silently_ok() {
        // 未实现清单问解析器要（`not_built_commands`），不手抄第二份：
        // 手抄的那份在命令落地之后还在宣称它未实现，而且没有任何测试会发现（ISSUES AC4）。
        let pending = not_built_commands();
        // 2026-10-17：catalog 里的每条命令都已实现，未实现清单为空。
        // 这个断言反过来守住"不能悄悄多出一条占位命令"。
        assert!(
            pending.is_empty(),
            "所有已规划命令都已实现，清单必须为空：{pending:?}"
        );
        // 已落地的命令不许继续躺在清单里；pgs encode 与 render 都曾经是占位
        for done in [
            "render",
            "pgs encode",
            "batch",
            "font scan",
            "font subset",
            "mkv identify",
            "mkv extract",
            "mkv remux",
        ] {
            assert!(
                !pending.contains(&done),
                "{done} 已经有实现，却仍被列为未实现：{pending:?}"
            );
        }
        let inv = parse(&argv(&["render", "x.ass", "--min-axis", "8"])).unwrap();
        match inv.command {
            Command::Render {
                fps,
                canvas,
                min_axis,
                default_font,
                dll,
                ..
            } => {
                assert_eq!(min_axis, 8);
                assert_eq!(fps, Fps::NTSC_23976, "默认帧率要有出处");
                assert_eq!(canvas, Canvas::FHD1080);
                assert_eq!(default_font, "Arial");
                assert_eq!(dll, None);
            }
            other => panic!("render 应该解析成 Command::Render，收到 {other:?}"),
        }
        assert!(parse(&argv(&["render", "x.ass", "--bogus", "1"])).is_err());
        // pgs encode 已经落地（14 §ASS→SUP）：它必须解析成真正的命令，不再是占位
        let inv = parse(&argv(&["pgs", "encode", "x.ass"])).unwrap();
        assert_eq!(
            inv.command.name(),
            "pgs encode",
            "{:?} 应该已经是实命令",
            inv.command
        );
        // batch 同理：spec 文件走 --spec，位置参数是用法错误
        let inv = parse(&argv(&["batch", "--spec", "job.json"])).unwrap();
        assert_eq!(inv.command.name(), "batch");
        assert!(
            parse(&argv(&["batch", "job.json"])).is_err(),
            "位置参数必须被拒"
        );
    }

    #[test]
    fn pgs_encode_snap_pts_is_off_by_default_and_on_when_asked() {
        // AA5：吸附会改动输出字节，因此默认必须关；开关也只影响这一个字段。
        let off = parse(&argv(&["pgs", "encode", "x.ass"])).unwrap();
        match off.command {
            Command::PgsEncode { snap_pts, .. } => assert!(!snap_pts, "默认不该吸附"),
            _ => panic!("不是 pgs encode"),
        }
        let on = parse(&argv(&["pgs", "encode", "x.ass", "--snap-pts"])).unwrap();
        match on.command {
            Command::PgsEncode { snap_pts, fps, .. } => {
                assert!(snap_pts, "--snap-pts 没生效");
                assert_eq!(fps.to_label(), "23.976", "开关不该改动别的字段");
            }
            _ => panic!("不是 pgs encode"),
        }
    }

    #[test]
    fn pgs_encode_defaults_are_the_bluray_profile_and_quantized_colors() {
        let inv = parse(&argv(&["pgs", "encode", "x.ass"])).unwrap();
        match inv.command {
            Command::PgsEncode {
                fps,
                canvas,
                level,
                dll,
                default_font,
                exact_colors,
                ..
            } => {
                assert_eq!(fps.to_label(), "23.976");
                assert_eq!((canvas.width, canvas.height), (1920, 1080));
                assert_eq!(level, ValidationLevel::Standard);
                assert_eq!(dll, None, "不猜 DLL 路径，只认显式输入与环境变量");
                assert_eq!(default_font, "Arial");
                assert!(!exact_colors, "默认走量化档：真实渲染就是会超过 255 色");
            }
            other => panic!("不是 PgsEncode：{other:?}"),
        }
        // 非 16:9 的 composition canvas 在参数阶段就拒（13 §4 的 profile 约束）
        assert_eq!(
            parse(&argv(&["pgs", "encode", "x.ass", "--canvas", "1000x800"]))
                .unwrap_err()
                .message,
            "PGS composition canvas 必须是 16:9 profile，收到 1000x800"
        );
        let e = parse(&argv(&["pgs", "encode", "x.ass", "--nosuch=1"])).unwrap_err();
        assert!(
            e.message.contains("pgs encode 不支持开关 --nosuch"),
            "未知开关要指名命令与前缀：{}",
            e.message
        );
        // 空值开关（不在 SWITCHES 里）由 scan 统一报"缺少取值"，各命令不必重复实现
        assert!(parse(&argv(&["pgs", "encode", "x.ass", "--nosuch"]))
            .unwrap_err()
            .message
            .contains("缺少取值"));
    }

    #[test]
    fn hdr_requires_a_mode_and_switch_flags_take_no_value() {
        assert!(parse(&argv(&["hdr", "x.ass"]))
            .unwrap_err()
            .message
            .contains("--hdr-mode"));
        let inv = parse(&argv(&[
            "hdr",
            "x.ass",
            "--hdr-mode",
            "reference-white",
            "--nits",
            "500",
            "--touch-alpha",
            "--out",
            "o.ass",
        ]))
        .unwrap();
        match inv.command {
            Command::Hdr { settings, .. } => {
                assert_eq!(settings.mode, HdrMode::ReferenceWhite);
                assert_eq!(settings.reference_nits, 500);
                assert!(settings.touch_alpha);
            }
            c => panic!("{c:?}"),
        }
        // 无值开关后面紧跟另一个开关时不能被吞
        let inv = parse(&argv(&[
            "hdr",
            "--touch-alpha",
            "--hdr-mode",
            "compatibility",
            "x.ass",
        ]))
        .unwrap();
        match inv.command {
            Command::Hdr { settings, input } => {
                assert_eq!(settings.mode, HdrMode::Compatibility);
                assert!(settings.touch_alpha);
                assert_eq!(input, PathBuf::from("x.ass"));
            }
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn unknown_flag_and_missing_value_are_classified() {
        assert!(parse(&argv(&["nosuch", "a"])).is_err());
        assert!(parse(&argv(&["convert", "a.ass", "--bogus", "1"])).is_err());
        assert!(parse(&argv(&["convert", "a.ass", "--to"])).is_err());
        assert!(parse(&argv(&["convert", "a.ass", "--to", "--json"])).is_err());
        assert!(parse(&argv(&["probe"])).is_err());
        assert!(parse(&argv(&[])).is_err());
        assert_eq!(parse(&argv(&["--help"])).unwrap_err().message, "help");
    }

    #[test]
    fn preset_is_refused_until_the_toml_dependency_is_confirmed() {
        let e = parse(&argv(&[
            "convert", "--preset", "a.toml", "--to", "srt", "x.ass",
        ]))
        .unwrap_err();
        assert!(e.message.contains("TOML"), "{}", e.message);
    }

    #[test]
    fn canvas_value_is_validated_by_core_model() {
        // Canvas::new 允许任意尺寸；16:9 规则在 profile/geometry 层执行（ISSUES F2）
        let inv = parse(&argv(&[
            "convert", "a.ass", "--to", "ass", "--canvas", "1920x800",
        ]))
        .unwrap();
        match inv.command {
            Command::Convert { canvas, .. } => assert_eq!(canvas.unwrap().label(), "1920x800"),
            c => panic!("{c:?}"),
        }
        assert!(parse(&argv(&[
            "convert", "a.ass", "--canvas", "abc", "--to", "ass"
        ]))
        .is_err());
        assert!(parse(&argv(&[
            "convert", "a.ass", "--canvas", "0x10", "--to", "ass"
        ]))
        .is_err());
    }

    #[test]
    fn two_positional_inputs_are_rejected() {
        let e = parse(&argv(&["convert", "a.ass", "b.ass", "--to", "srt"])).unwrap_err();
        assert!(e.message.contains("一个输入文件"), "{e:?}");
    }
}
