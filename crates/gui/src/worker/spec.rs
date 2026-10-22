//! 把 GUI 的输入集与 Profile 矩阵编译成**真实的 batch spec**（15 §1）。
//!
//! ## 这个模块存在的唯一理由
//!
//! 20 §Repro 要求"GUI 复制的 CLI 命令可以直接重放"。要让那句话**结构上成立**，
//! 就不能让 GUI 自己走一条并行的执行路径：它必须产出与 `subtitletool-cli batch`
//! 完全相同的 spec，再由**同一份** `cli::batch::run_batch` 执行。
//!
//! 于是"GUI 跑出来的结果"与"把 spec 存下来用 CLI 跑"是同一件事，而不是两件
//! 看起来像的事。这也让 GUI 的行为可以被 CLI 的测试覆盖到——不需要一套 GUI 专用的
//! 验收门槛。
//!
//! ## 为什么用 `cli::batch::Spec` 而不是自己再声明一份
//!
//! 自己声明一份就有两个形状：CLI 加了字段 GUI 不知道，或者 GUI 写了 CLI 不认的 key
//! （spec 是 `deny_unknown_fields`，会直接报错）。直接复用类型，编译器替我们守住一致性。

use std::path::{Path, PathBuf};

use cli::batch::{Spec, SpecOutput, SpecProfile};

use crate::state::profile::{CanvasMode, HdrMode, OutputProfile, TargetFormat, TimeAdjustMode};

/// 画布模式 → spec 的 `WxH` 字符串。
///
/// 使用明确的 original sentinel：batch 会逐输入读取真实 PlayRes。
pub const KEEP_ORIGINAL_CANVAS: &str = "original";

fn canvas_string(c: &CanvasMode) -> String {
    match c {
        CanvasMode::KeepOriginal => KEEP_ORIGINAL_CANVAS.to_string(),
        CanvasMode::Fhd1080 => "1920x1080".to_string(),
        CanvasMode::Uhd2160 => "3840x2160".to_string(),
        CanvasMode::Hd720 => "1280x720".to_string(),
        CanvasMode::Custom(w, h) => format!("{w}x{h}"),
    }
}

/// 目标格式 → spec 的 `format` 取值（`ass|ssa|srt|pgs|bdn`）。
///
/// `FormatChanged` 目前只会在 BDN/SUP/ASS/SRT 之间切换（见 `state::profile`），
/// 这里的映射与 `cli::batch::resolve_profiles` 认的拼写**逐字对应**；
/// 对不上的话 CLI 会报 `STK-INP-0156`，而那是"配置写错了"，不该由 GUI 产生。
pub fn format_string(f: &TargetFormat) -> &'static str {
    match f {
        TargetFormat::BdnXml => "bdn",
        TargetFormat::Pgs => "pgs",
        TargetFormat::Ass => "ass",
        TargetFormat::Srt => "srt",
    }
}

/// 一个 profile 的编译结果：spec 里的那一项 + 给用户看的说明。
pub struct CompiledProfile {
    pub spec: SpecProfile,
    /// 这个 profile 在当前参数下**做不到**的事，必须显示出来。
    ///
    /// `text` 而非 `Option` 的取舍：把"做不到"当错误会让整个任务起不来，
    /// 而当静默忽略会让用户以为参数生效了。折中是编译出来、随任务一起报。
    pub notes: Vec<String>,
}

/// 把单个 profile 编译成 spec 项。
pub fn compile_profile(p: &OutputProfile) -> CompiledProfile {
    compile_profile_with_fps(p, crate::worker::task_worker::default_fps())
}

pub fn compile_profile_with_fps(p: &OutputProfile, default_fps: &str) -> CompiledProfile {
    let mut notes = Vec::new();

    // 时间与 HDR 作为可重放字段写入 batch spec，由 CLI 在渲染前真实应用。
    let (offset_ms, speed_from, speed_to) = match p.time_adjust {
        TimeAdjustMode::None => (None, None, None),
        TimeAdjustMode::OffsetMs(ms) => (Some(ms), None, None),
        TimeAdjustMode::SpeedScale { from, to } =>
            (None, Some(from.to_string()), Some(to.to_string())),
    };
    let (hdr_percent, hdr_reference_nits) = match p.hdr {
        HdrMode::Sdr => (None, None),
        HdrMode::Percentage(pct) => (Some(pct), None),
        HdrMode::ReferenceWhite(nits) => (None, Some(nits)),
    };
    CompiledProfile {
        spec: SpecProfile {
            name: p.name.clone(),
            format: format_string(&p.target_format).to_string(),
            canvas: canvas_string(&p.canvas),
            // 帧率：GUI 目前没有逐 profile 的帧率控件，用 spec 的默认值。
            // 这里的字符串必须与 cli::batch::default_fps 一致，否则"GUI 与 CLI 默认不同"
            // 又是一处静默分歧。
            fps: default_fps.to_string(),
            // pgs：GUI 没有暴露"严格 255 色"开关，保持与单命令默认一致（量化 + 记账）
            exact_colors: false,
            // bdn：与 CLI 单命令默认一致（0 = 不过滤）
            min_axis: 0,
            default_font: None,
            offset_ms,
            speed_from,
            speed_to,
            hdr_percent,
            hdr_reference_nits,
        },
        notes,
    }
}

/// 编译整份 batch spec。
///
/// `roots` 是选中的输入文件；`output_dir` 落在 `spec.output.dir`（绝对路径），
/// `libass_dll` 只在需要渲染的 profile 存在时才有意义，但传进去无害：
/// CLI 只在真需要 renderer 时才会去读它。
pub fn compile_spec(
    job_id: &str,
    roots: &[PathBuf],
    profiles: &[&OutputProfile],
    output_dir: &Path,
    libass_dll: Option<PathBuf>,
    template: &str,
) -> (Spec, Vec<String>) {
    compile_spec_with_fps(
        job_id,
        roots,
        profiles,
        output_dir,
        libass_dll,
        template,
        crate::worker::task_worker::default_fps(),
    )
}

pub fn compile_spec_with_fps(
    job_id: &str,
    roots: &[PathBuf],
    profiles: &[&OutputProfile],
    output_dir: &Path,
    libass_dll: Option<PathBuf>,
    template: &str,
    default_fps: &str,
) -> (Spec, Vec<String>) {
    let mut notes: Vec<String> = Vec::new();
    let mut compiled = Vec::new();
    for p in profiles {
        let c = compile_profile_with_fps(p, default_fps);
        for n in &c.notes {
            notes.push(format!("[{}] {}", p.name, n));
        }
        compiled.push(c.spec);
    }

    let spec = Spec {
        id: job_id.to_string(),
        // 直接把选中的文件当 roots：15 §2 的 roots 可以是文件也可以是目录，
        // 这样"用户勾了哪几个"就精确对应"跑了哪几个"，不需要再靠 include/exclude 表达。
        roots: roots
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
        // 文件型 roots 下 recursive 没有作用；留着 true 是为了目录型 roots 未来能用
        recursive: true,
        include: default_includes(),
        exclude: default_excludes(),
        profiles: compiled,
        only_pairs: Vec::new(),
        output: SpecOutput {
            dir: Some(output_dir.display().to_string()),
            template: template.to_string(),
            // 与 CLI 单命令默认一致（15 §6）：目标已存在就换个名字，绝不截断原文件
            overwrite: None,
        },
        // 38 §3：GUI 无法保证"重跑不会伤到用户数据"（用户可能改过产物），
        // 所以不给崩溃预算，与 CLI 的默认一致。
        runtime: None,
        libass_dll: libass_dll.map(|p| p.display().to_string()),
        video_association: false,
    };
    (spec, notes)
}

/// 输入发现的白名单。GUI 把每个选中文件直接列进 roots，所以这里只需保证
/// 这些扩展名不会被 exclude 掉。
fn default_includes() -> Vec<String> {
    ["ass", "ssa", "srt", "sup", "xml"]
        .iter()
        .map(|s| format!("*.{s}"))
        .collect()
}

/// 不排除任何东西：用户已经逐个勾选了，再按扩展名排除就等于无视他的选择。
fn default_excludes() -> Vec<String> {
    Vec::new()
}

/// 把 spec 序列化成可以存盘、也可以直接喂给 `subtitletool-cli batch --spec` 的 JSON。
///
/// 这份文本就是"复制到 CLI 重放"的那个东西：它必须能通过 `cli::batch::load_spec`
/// 的校验（含 `deny_unknown_fields`），由测试守着。
pub fn spec_to_json(spec: &Spec) -> String {
    serde_json::to_string_pretty(spec).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::profile::ProfilesState;
    use core_model::time::Rational;

    fn dir() -> PathBuf {
        PathBuf::from(r"D:\out")
    }

    #[test]
    fn compiled_spec_passes_the_real_cli_validator() {
        // 这条是"GUI 与 CLI 不是两套东西"的直接判据：
        // 把编译结果交给 CLI 自己的解析器，它必须接受。
        let profiles = ProfilesState::new();
        let enabled: Vec<&OutputProfile> = profiles.enabled_profiles();
        let (spec, _notes) = compile_spec(
            "gui-job",
            &[PathBuf::from(r"D:\in\a.ass")],
            &enabled,
            &dir(),
            None,
            "{stem}.{profile}.{ext}",
        );
        let text = spec_to_json(&spec);
        let back: Spec =
            serde_json::from_str(&text).expect("CLI 的 Spec 必须能解回自己序列化的东西");

        // 再用 CLI 的**公开入口**校验一遍：文件落盘后 load_spec 必须通过。
        // 只比 serde 往返不够——load_spec 还会查 id 非空、roots 非空、profiles 非空。
        let tmp = std::env::temp_dir().join(format!("stk-gui-spec-{}.json", std::process::id()));
        std::fs::write(&tmp, &text).unwrap();
        let loaded = cli::batch::load_spec(&tmp).expect("GUI 编译的 spec 必须能过 CLI 的校验");
        let _ = std::fs::remove_file(&tmp);

        assert_eq!(loaded.id, "gui-job");
        assert_eq!(loaded.roots.len(), 1);
        assert_eq!(loaded.profiles.len(), back.profiles.len());
        assert!(!loaded.profiles.is_empty(), "默认至少有一个启用的 profile");
    }

    #[test]
    fn every_gui_format_maps_to_a_spelling_the_cli_accepts() {
        // 拼错一个字母，CLI 会报 STK-INP-0156，而那是"配置写错了"——
        // 不该由我们的枚举产生。
        for (fmt, want) in [
            (TargetFormat::BdnXml, "bdn"),
            (TargetFormat::Pgs, "pgs"),
            (TargetFormat::Ass, "ass"),
            (TargetFormat::Srt, "srt"),
        ] {
            assert_eq!(format_string(&fmt), want);
        }
    }

    #[test]
    fn canvas_modes_map_to_real_dimensions() {
        assert_eq!(canvas_string(&CanvasMode::Fhd1080), "1920x1080");
        assert_eq!(canvas_string(&CanvasMode::Uhd2160), "3840x2160");
        assert_eq!(canvas_string(&CanvasMode::Hd720), "1280x720");
        assert_eq!(canvas_string(&CanvasMode::Custom(2560, 1440)), "2560x1440");
        // KeepOriginal 没有对应取值，必须落到一个具体数字且**说出来**
        assert_eq!(
            canvas_string(&CanvasMode::KeepOriginal),
            KEEP_ORIGINAL_CANVAS
        );
        let p = OutputProfile {
            id: 1,
            enabled: true,
            name: "x".into(),
            target_format: TargetFormat::Ass,
            canvas: CanvasMode::KeepOriginal,
            time_adjust: TimeAdjustMode::None,
            hdr: HdrMode::Sdr,
            custom_suffix: String::new(),
        };
        let c = compile_profile(&p);
        assert!(
            c.notes.iter().any(|n| n.contains("保持脚本原始尺寸")),
            "换个画布就必须说清楚：{:?}",
            c.notes
        );
    }

    /// 时间与 HDR 参数必须写入可重放的 batch spec。
    #[test]
    fn unsupported_profile_settings_are_reported_not_silently_dropped() {
        let mut p = OutputProfile::default_bdn(1);
        p.time_adjust = TimeAdjustMode::OffsetMs(1000);
        p.hdr = HdrMode::ReferenceWhite(100);
        let c = compile_profile(&p);
        assert_eq!(c.spec.offset_ms, Some(1000));
        assert_eq!(c.spec.hdr_reference_nits, Some(100));
        assert!(c.notes.is_empty(), "已实现的功能不应再报未应用: {:?}", c.notes);

        let mut q = OutputProfile::default_bdn(2);
        q.time_adjust = TimeAdjustMode::SpeedScale {
            from: Rational::new(24000, 1001).unwrap(),
            to: Rational::new(25, 1).unwrap(),
        };
        let cq = compile_profile(&q);
        assert_eq!(cq.spec.speed_from.as_deref(), Some("24000/1001"));
        assert_eq!(cq.spec.speed_to.as_deref(), Some("25"));
    }

    #[test]
    fn notes_are_attributed_to_the_profile_that_caused_them() {
        let profiles = ProfilesState::new();
        let mut a = profiles.profiles[0].clone();
        a.name = "A".into();
        a.time_adjust = TimeAdjustMode::OffsetMs(5);
        // 拿一个**真的干净**的 profile 当对照组：默认的 2160p SUP 带着
        // `HdrMode::ReferenceWhite(203)`，它本身就会产生一条说明——
        // 拿它当“没问题的那个”会让这条断言变假。
        let mut b = profiles.profiles[0].clone();
        b.name = "B".into();
        b.time_adjust = TimeAdjustMode::None;
        b.hdr = HdrMode::Sdr;
        b.canvas = CanvasMode::Fhd1080;

        let (_, notes) = compile_spec(
            "j",
            &[PathBuf::from("x.ass")],
            &[&a, &b],
            &dir(),
            None,
            "{stem}.{ext}",
        );
        assert!(
            notes.iter().any(|n| n.starts_with("[A]")),
            "说明必须带上是谁引起的：{notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.starts_with("[B]")),
            "参数都支持的那个不该产生说明：{notes:?}"
        );
    }

    #[test]
    fn output_dir_and_template_land_in_the_spec() {
        let p = OutputProfile::default_bdn(1);
        let (spec, _) = compile_spec(
            "j",
            &[PathBuf::from(r"D:\in\a.ass")],
            &[&p],
            &PathBuf::from(r"D:\deliver"),
            Some(PathBuf::from(r"D:\deps\ass.dll")),
            "{stem}_{profile}.{ext}",
        );
        assert_eq!(spec.output.dir.as_deref(), Some(r"D:\deliver"));
        assert_eq!(spec.output.template, "{stem}_{profile}.{ext}");
        assert_eq!(spec.libass_dll.as_deref(), Some(r"D:\deps\ass.dll"));
        assert_eq!(spec.roots, vec![r"D:\in\a.ass".to_string()]);
        // 不排除任何东西：用户已经逐个勾选了
        assert!(spec.exclude.is_empty());
        assert!(!spec.include.is_empty(), "白名单不能是空——空等于放行一切");
    }

    #[test]
    fn configured_fps_is_frozen_into_the_gui_batch_snapshot() {
        let profile = ProfilesState::new().profiles.remove(0);
        let (spec, _) = compile_spec_with_fps(
            "j",
            &[PathBuf::from("a.ass")],
            &[&profile],
            &PathBuf::from("out"),
            None,
            "{stem}.{ext}",
            "24000/1001",
        );
        assert_eq!(spec.profiles[0].fps, "24000/1001");
    }
}
