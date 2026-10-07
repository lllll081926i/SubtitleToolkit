//! Profile / JobSnapshot / Plan 与参数来源标记（35、36 两份文档的数据侧）。

use crate::canvas::Canvas;
use crate::error::{Domain, StkError};
use crate::time::Fps;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FormatKind {
    Ass,
    Ssa,
    Srt,
    Vtt,
    Pgs,
    Bdn,
    Text,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RendererChoice {
    /// 文本转换不需要渲染器
    None,
    Libass,
    /// 自动探测 CSRI provider / VSFilterMod
    SpecialAuto,
    Csri,
    VapourSynth,
    AviSynth,
}

/// PGS 成熟度分级（13 §1、ADR-009）：能力与标签必须一一对应，不得越级宣称。
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ValidationLevel {
    Probe,
    Structure,
    Basic,
    Standard,
    Strict,
}

impl ValidationLevel {
    pub fn label(self) -> &'static str {
        match self {
            ValidationLevel::Probe => "probe",
            ValidationLevel::Structure => "structure",
            ValidationLevel::Basic => "basic",
            ValidationLevel::Standard => "standard",
            ValidationLevel::Strict => "blu-ray strict",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HdrMode {
    #[default]
    Off,
    Compatibility,
    Percentage,
    ReferenceWhite,
}

/// HDR 预调整参数。亮度相关量全部用整数，避免 float 进入核心。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct HdrSettings {
    pub mode: HdrMode,
    /// Percentage 模式：万分比（3330 = 33.30%）。
    pub percent_basis_points: u32,
    /// ReferenceWhite 模式：目标 reference white nit。
    pub reference_nits: u32,
    /// 算法版本进 report（11 §3）。
    pub algorithm_version: u32,
    pub touch_styles: bool,
    pub touch_inline: bool,
    pub touch_vsfiltermod_ext: bool,
    /// Alpha 默认完全不改。
    pub touch_alpha: bool,
}

impl HdrSettings {
    pub fn off() -> HdrSettings {
        HdrSettings {
            mode: HdrMode::Off,
            ..Default::default()
        }
    }
    pub fn compatibility() -> HdrSettings {
        HdrSettings {
            mode: HdrMode::Compatibility,
            algorithm_version: 1,
            touch_styles: true,
            touch_inline: true,
            ..Default::default()
        }
    }
    pub fn percentage(basis_points: u32) -> Result<HdrSettings, StkError> {
        if basis_points == 0 || basis_points > 10_000 {
            return Err(StkError::simple(
                Domain::Input,
                40,
                "HDR 百分比必须在 0.01%..100% 之间",
            ));
        }
        Ok(HdrSettings {
            mode: HdrMode::Percentage,
            percent_basis_points: basis_points,
            algorithm_version: 1,
            touch_styles: true,
            touch_inline: true,
            ..Default::default()
        })
    }
    pub fn reference_white(nits: u32) -> Result<HdrSettings, StkError> {
        if nits == 0 || nits > 10_000 {
            return Err(StkError::simple(
                Domain::Input,
                41,
                "reference white nit 非法",
            ));
        }
        Ok(HdrSettings {
            mode: HdrMode::ReferenceWhite,
            reference_nits: nits,
            algorithm_version: 1,
            touch_styles: true,
            touch_inline: true,
            ..Default::default()
        })
    }
    pub fn is_noop(&self) -> bool {
        self.mode == HdrMode::Off
    }
}

/// 参数来源（35 §2）：视频探测永远不能覆盖用户显式值。
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, serde::Serialize, serde::Deserialize,
)]
pub enum ValueOrigin {
    Default,
    AutoRecommendation,
    VideoProbe,
    InputMetadata,
    Preset,
    UserExplicit,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OutputProfile {
    pub name: String,
    pub format: FormatKind,
    /// Output Canvas（09 §2 的第一个概念）
    pub canvas: Option<Canvas>,
    /// 用户显式选择的实验性自定义画布（09 §1 的例外通道）
    pub canvas_is_custom: bool,
    /// 脚本分辨率策略（09 §4：默认 Preserve）
    pub script_policy: crate::geometry::ScriptResolutionPolicy,
    pub layout_policy: crate::geometry::LayoutResPolicy,
    pub fps: Option<Fps>,
    pub hdr: HdrSettings,
    pub renderer: RendererChoice,
    pub pgs_validation: ValidationLevel,
    pub suffix: String,
    pub enabled: bool,
}

impl OutputProfile {
    pub fn new(name: &str, format: FormatKind) -> OutputProfile {
        OutputProfile {
            name: name.to_string(),
            format,
            canvas: None,
            canvas_is_custom: false,
            script_policy: crate::geometry::ScriptResolutionPolicy::Preserve,
            layout_policy: crate::geometry::LayoutResPolicy::KeepAsIs,
            fps: None,
            hdr: HdrSettings::off(),
            renderer: RendererChoice::None,
            pgs_validation: ValidationLevel::Probe,
            suffix: String::new(),
            enabled: true,
        }
    }

    /// 严格 Blu-ray/UHD-BD profile 禁止 2160p PGS（09 §6、AT-BD-001）。
    pub fn validate(&self) -> Result<(), StkError> {
        if self.name.trim().is_empty() {
            return Err(StkError::simple(Domain::Input, 50, "profile 名不能为空"));
        }
        if let Some(c) = self.canvas {
            if !c.is_16_9() && !self.canvas_is_custom {
                return Err(StkError::simple(
                    Domain::Input,
                    31,
                    "输出画布必须是完整 16:9，或显式声明为实验性自定义",
                )
                .with_detail(c.label()));
            }
        }
        if self.format == FormatKind::Pgs && self.pgs_validation >= ValidationLevel::Standard {
            if let Some(c) = self.canvas {
                if !c.within_bluray() {
                    return Err(StkError::simple(
                        Domain::Validate,
                        2,
                        "严格蓝光 profile 的 PGS 画布不得超过 1920x1080",
                    )
                    .with_detail(c.label())
                    .with_action(
                        "改用 1920x1080 PGS 画布；3840x2160 只能作为 non-standard/custom 输出",
                    ));
                }
            }
        }
        Ok(())
    }

    /// UI 卡片标签（05 §1）：Format / Canvas / FPS / HDR / Renderer / Validation。
    pub fn card_labels(&self) -> Vec<(&'static str, String)> {
        let hdr = match self.hdr.mode {
            HdrMode::Off => "SDR".to_string(),
            HdrMode::Compatibility => "compatibility".to_string(),
            HdrMode::Percentage => format!("{}%", self.hdr.percent_basis_points as f64 / 100.0),
            HdrMode::ReferenceWhite => format!("{}nit", self.hdr.reference_nits),
        };
        vec![
            ("Format", format!("{:?}", self.format)),
            (
                "Canvas",
                self.canvas
                    .map(|c| c.label())
                    .unwrap_or_else(|| "text".to_string()),
            ),
            (
                "FPS",
                self.fps
                    .map(|f| f.to_label())
                    .unwrap_or_else(|| "same".to_string()),
            ),
            ("HDR", hdr),
            ("Renderer", format!("{:?}", self.renderer)),
            ("Validation", self.pgs_validation.label().to_string()),
        ]
    }
}

/// 一次执行的完整冻结参数（35 §3：运行中的任务永不读取 Draft）。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct JobSnapshot {
    pub job_id: String,
    pub input_paths: Vec<String>,
    pub input_hash: String,
    pub profiles: Vec<OutputProfile>,
    pub plan: Plan,
    pub taken_at: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum StageKind {
    Decode,
    Transform,
    Render,
    Image,
    Write,
    Validate,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stage {
    pub kind: StageKind,
    pub detail: String,
    /// 共享该 artifact 的 profile 下标（15 §4 多输出复用的表达）
    pub shared_by: Vec<usize>,
}

/// Planner 产出的不可变 DAG，必须能导出 JSON 供 bug report 与 CLI 重放（07 §4、20）。
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Plan {
    pub stages: Vec<Stage>,
}

impl Plan {
    pub fn push(&mut self, kind: StageKind, detail: &str, shared_by: Vec<usize>) {
        self.stages.push(Stage {
            kind,
            detail: detail.to_string(),
            shared_by,
        });
    }
    pub fn find(&self, kind: StageKind) -> Option<&Stage> {
        self.stages.iter().find(|s| s.kind == kind)
    }
    pub fn to_json(&self) -> Result<String, StkError> {
        serde_json::to_string_pretty(self).map_err(|e| {
            StkError::simple(Domain::Output, 60, "Plan 序列化失败").with_detail(e.to_string())
        })
    }
    pub fn from_json(s: &str) -> Result<Plan, StkError> {
        serde_json::from_str(s).map_err(|e| {
            StkError::simple(Domain::Input, 61, "Plan JSON 非法").with_detail(e.to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;

    #[test]
    fn strict_pgs_blocks_2160_canvas() {
        let mut p = OutputProfile::new("UHD-BD-SUP", FormatKind::Pgs);
        p.canvas = Some(Canvas::UHD2160);
        p.pgs_validation = ValidationLevel::Strict;
        let e = p.validate().unwrap_err();
        assert_eq!(e.code_string(), "STK-VAL-0002");
        // 同样的画布作为 custom（Structure 级）是允许的
        p.pgs_validation = ValidationLevel::Structure;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn bluray_canvas_passes() {
        let mut p = OutputProfile::new("BD-SUP", FormatKind::Pgs);
        p.canvas = Some(Canvas::FHD1080);
        p.pgs_validation = ValidationLevel::Strict;
        assert!(p.validate().is_ok());
        assert!(OutputProfile::new("", FormatKind::Pgs).validate().is_err());
    }

    #[test]
    fn non_16_9_profile_needs_custom_flag() {
        // 09 §1：除非用户显式选择实验性自定义
        let mut p = OutputProfile::new("odd", FormatKind::Ass);
        p.canvas = Some(Canvas {
            width: 1920,
            height: 800,
        });
        assert_eq!(p.validate().unwrap_err().code_string(), "STK-INP-0031");
        p.canvas_is_custom = true;
        assert!(p.validate().is_ok());
        assert_eq!(
            p.script_policy,
            crate::geometry::ScriptResolutionPolicy::Preserve
        );
        assert_eq!(p.layout_policy, crate::geometry::LayoutResPolicy::KeepAsIs);
    }

    #[test]
    fn hdr_validation_and_noop() {
        assert!(HdrSettings::off().is_noop());
        assert_eq!(HdrSettings::compatibility().mode, HdrMode::Compatibility);
        assert_eq!(
            HdrSettings::percentage(3330).unwrap().percent_basis_points,
            3330
        );
        assert!(HdrSettings::percentage(0).is_err());
        assert!(HdrSettings::percentage(10001).is_err());
        assert!(HdrSettings::reference_white(203).unwrap().touch_inline);
        assert!(!HdrSettings::reference_white(203).unwrap().touch_alpha);
        assert!(HdrSettings::reference_white(0).is_err());
    }

    #[test]
    fn origin_priority_is_ordered() {
        assert!(ValueOrigin::UserExplicit > ValueOrigin::Preset);
        assert!(ValueOrigin::Preset > ValueOrigin::InputMetadata);
        assert!(ValueOrigin::InputMetadata > ValueOrigin::VideoProbe);
        assert!(ValueOrigin::VideoProbe > ValueOrigin::AutoRecommendation);
        assert!(ValueOrigin::AutoRecommendation > ValueOrigin::Default);
    }

    #[test]
    fn plan_roundtrips_json() {
        let mut plan = Plan::default();
        plan.push(StageKind::Decode, "ass", vec![0, 1, 2]);
        plan.push(StageKind::Render, "libass@1920x1080", vec![1, 2]);
        plan.push(StageKind::Write, "sup", vec![2]);
        let json = plan.to_json().unwrap();
        assert!(json.contains("libass@1920x1080"));
        assert_eq!(Plan::from_json(&json).unwrap(), plan);
        assert_eq!(plan.find(StageKind::Render).unwrap().shared_by, vec![1, 2]);
        assert!(plan.find(StageKind::Validate).is_none());
        assert!(Plan::from_json("{ not json").is_err());
    }

    #[test]
    fn card_labels_cover_six_fields() {
        let mut p = OutputProfile::new("HDR-4K", FormatKind::Ass);
        p.canvas = Some(Canvas::UHD2160);
        p.fps = Some(Fps::NTSC_23976);
        p.hdr = HdrSettings::reference_white(203).unwrap();
        let c = p.card_labels();
        assert_eq!(c.len(), 6);
        assert_eq!(c[3], ("HDR", "203nit".to_string()));
        assert_eq!(c[1], ("Canvas", "3840x2160".to_string()));
        assert_eq!(c[2], ("FPS", "23.976".to_string()));
    }
}
