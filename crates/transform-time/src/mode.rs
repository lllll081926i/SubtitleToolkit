//! 模式与参数校验（10 §1 三件事必须分开；05 §4 用户必须显式选择）。

use core_model::error::Domain;
use core_model::{Fps, LossReport, Rational, StkError, Time};

use crate::codes;
use crate::ops::{self, Cue};

/// 时间变换模式。四种语义互斥：调用方选哪个就只做哪个，缺参数报错，不会退回更弱的模式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeMode {
    /// 全部时间加减同一 offset：时长不变。
    Offset,
    /// 只把 cue 边界对齐目标帧网格：时长不变（10 §1.2）。
    FrameSnap,
    /// `t' = t * source/target`：影片总时长改变（10 §1.1/§3）。
    SpeedScale,
    /// `t' = a*t + b`，由 4 个控制点拟合（10 §4）。
    TwoPoint,
}

impl TimeMode {
    /// 02 §3 第 3 步的四个 UI 选项名。
    pub fn label(self) -> &'static str {
        match self {
            TimeMode::Offset => "Offset only",
            TimeMode::FrameSnap => "Snap to frame only",
            TimeMode::SpeedScale => "Speed/Duration scale",
            TimeMode::TwoPoint => "Two-point linear fit",
        }
    }

    /// 是否会改变影片总时长语义。Offset/FrameSnap 恒为 false，UI 的 `?` tooltip 直接用。
    pub fn changes_duration(self) -> bool {
        matches!(self, TimeMode::SpeedScale | TimeMode::TwoPoint)
    }

    pub fn requires_source_fps(self) -> bool {
        matches!(self, TimeMode::SpeedScale)
    }

    pub fn requires_target_fps(self) -> bool {
        matches!(
            self,
            TimeMode::FrameSnap | TimeMode::SpeedScale | TimeMode::TwoPoint
        )
    }
}

/// 两点拟合的 4 个控制点：`source1 -> target1`、`source2 -> target2`。
///
/// 不放进 `TimeConfig`，因为它是 4 个 `Time` 而非数值参数；`TimeConfig::mode == TwoPoint` 时必须由调用方另行提供。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TwoPointSpec {
    pub source1: Time,
    pub target1: Time,
    pub source2: Time,
    pub target2: Time,
}

impl TwoPointSpec {
    pub fn fit(&self) -> Result<(Rational, Rational), StkError> {
        ops::two_point_fit(self.source1, self.target1, self.source2, self.target2)
    }
}

/// 时间页的参数集（05 §4）。
#[derive(Clone, Debug, PartialEq)]
pub struct TimeConfig {
    pub mode: TimeMode,
    pub source_fps: Option<Fps>,
    pub target_fps: Option<Fps>,
    pub offset_ms: i64,
}

impl TimeConfig {
    pub fn offset(offset_ms: i64) -> TimeConfig {
        TimeConfig {
            mode: TimeMode::Offset,
            source_fps: None,
            target_fps: None,
            offset_ms,
        }
    }

    pub fn frame_snap(target_fps: Fps) -> TimeConfig {
        TimeConfig {
            mode: TimeMode::FrameSnap,
            source_fps: None,
            target_fps: Some(target_fps),
            offset_ms: 0,
        }
    }

    pub fn speed_scale(source_fps: Fps, target_fps: Fps) -> TimeConfig {
        TimeConfig {
            mode: TimeMode::SpeedScale,
            source_fps: Some(source_fps),
            target_fps: Some(target_fps),
            offset_ms: 0,
        }
    }

    /// 控制点通过 `TwoPointSpec` 传入；source FPS 可选（映射已由控制点定义）。
    pub fn two_point(target_fps: Fps) -> TimeConfig {
        TimeConfig {
            mode: TimeMode::TwoPoint,
            source_fps: None,
            target_fps: Some(target_fps),
            offset_ms: 0,
        }
    }

    /// 只校验 `TimeConfig` 自身。`TwoPoint` 模式需要控制点，因此这里会显式失败，避免 UI 以为参数已完备。
    pub fn validate(&self) -> Result<(), StkError> {
        self.validate_with(None)
    }

    pub fn validate_with(&self, spec: Option<&TwoPointSpec>) -> Result<(), StkError> {
        if self.mode.requires_source_fps() && self.source_fps.is_none() {
            return Err(missing_fps("source", self.mode));
        }
        if self.mode.requires_target_fps() && self.target_fps.is_none() {
            return Err(missing_fps("target", self.mode));
        }
        if self.mode == TimeMode::TwoPoint {
            let spec = spec.ok_or_else(|| {
                StkError::simple(
                    Domain::Transform,
                    codes::NEED_CONTROL_POINTS,
                    "Two-point 模式缺少 4 个控制点",
                )
                .with_action("在 TwoPointSpec 里填写 source1/target1/source2/target2")
                .with_detail(format!("mode = {}", self.mode.label()))
            })?;
            spec.fit()?;
        }
        Ok(())
    }
}

/// 按模式严格执行变换。**唯一**的调度入口，内部不做任何模式推断（05 §4）。
pub fn apply(
    cues: &[Cue],
    cfg: &TimeConfig,
    spec: Option<&TwoPointSpec>,
) -> Result<(Vec<Cue>, LossReport), StkError> {
    cfg.validate_with(spec)?;
    match cfg.mode {
        TimeMode::Offset => ops::apply_offset(cues, cfg.offset_ms),
        TimeMode::FrameSnap => ops::try_apply_frame_snap(cues, need_target(cfg)?),
        TimeMode::SpeedScale => {
            ops::try_apply_speed_scale(cues, need_source(cfg)?, need_target(cfg)?)
        }
        TimeMode::TwoPoint => {
            let (a, b) = spec.ok_or_else(|| missing_control_points(cfg))?.fit()?;
            ops::try_apply_two_point(cues, a, b)
        }
    }
}

fn missing_fps(which: &str, mode: TimeMode) -> StkError {
    StkError::simple(
        Domain::Transform,
        codes::MISSING_FPS,
        &format!("{} 模式缺少 {which} FPS", mode.label()),
    )
    .with_action("显式填写 FPS；模式不会自动推断（05 §4）")
    .with_detail(format!("mode = {}", mode.label()))
}

fn missing_control_points(cfg: &TimeConfig) -> StkError {
    StkError::simple(
        Domain::Transform,
        codes::NEED_CONTROL_POINTS,
        "Two-point 模式缺少 4 个控制点",
    )
    .with_action("在 TwoPointSpec 里填写 source1/target1/source2/target2")
    .with_detail(format!("mode = {}", cfg.mode.label()))
}

fn need_target(cfg: &TimeConfig) -> Result<Fps, StkError> {
    cfg.target_fps
        .ok_or_else(|| missing_fps("target", cfg.mode))
}

fn need_source(cfg: &TimeConfig) -> Result<Fps, StkError> {
    cfg.source_fps
        .ok_or_else(|| missing_fps("source", cfg.mode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cue, ms};

    #[test]
    fn offset_mode_needs_no_fps() {
        assert!(TimeConfig::offset(-1234).validate().is_ok());
        assert!(!TimeMode::Offset.changes_duration());
    }

    #[test]
    fn frame_snap_needs_only_target_fps() {
        let cfg = TimeConfig {
            mode: TimeMode::FrameSnap,
            source_fps: None,
            target_fps: None,
            offset_ms: 0,
        };
        let e = cfg.validate().expect_err("缺 target FPS 必须报错");
        assert_eq!(e.domain, Domain::Transform);
        assert!(e.to_string().contains("Snap to frame only"));
        assert!(TimeConfig::frame_snap(Fps::PAL_50).validate().is_ok());
    }

    #[test]
    fn speed_scale_needs_both_fps() {
        let cfg = TimeConfig {
            mode: TimeMode::SpeedScale,
            source_fps: Some(Fps::NTSC_23976),
            target_fps: None,
            offset_ms: 0,
        };
        assert!(cfg.validate().is_err());
        assert!(TimeConfig::speed_scale(Fps::NTSC_23976, Fps::PAL_25)
            .validate()
            .is_ok());
        assert!(TimeMode::SpeedScale.changes_duration());
    }

    #[test]
    fn two_point_needs_a_spec_separate_from_config() {
        let cfg = TimeConfig::two_point(Fps::PAL_25);
        // TimeConfig 自身装不下 4 个控制点，所以 validate() 必须失败而不是假装参数完备。
        assert!(cfg.validate().is_err());
        let spec = TwoPointSpec {
            source1: Time::ZERO,
            target1: ms(500),
            source2: Time::from_secs(10),
            target2: ms(10500),
        };
        assert!(cfg.validate_with(Some(&spec)).is_ok());
    }

    #[test]
    fn dispatcher_is_strict_about_missing_fps() {
        let cues = [cue(0, 1000)];
        let cfg = TimeConfig {
            mode: TimeMode::SpeedScale,
            source_fps: None,
            target_fps: Some(Fps::PAL_25),
            offset_ms: 0,
        };
        assert!(apply(&cues, &cfg, None).is_err());
    }

    #[test]
    fn dispatcher_routes_two_point_through_spec() {
        let cues = [cue(0, 10000)];
        let cfg = TimeConfig::two_point(Fps::PAL_25);
        let spec = TwoPointSpec {
            source1: Time::ZERO,
            target1: ms(500),
            source2: Time::from_secs(10),
            target2: ms(10500),
        };
        let (out, _loss) = apply(&cues, &cfg, Some(&spec)).expect("合法两点拟合");
        assert_eq!(out, vec![cue(500, 10500)]);
    }

    #[test]
    fn offset_mode_ignores_fps_it_must_not_scale() {
        // 05 §4：即使填了 23.976→25，mode=Offset 也只平移，绝不"顺手"变成 speed scale。
        let cues = [cue(0, 1000), cue(2000, 3000)];
        let cfg = TimeConfig {
            mode: TimeMode::Offset,
            source_fps: Some(Fps::NTSC_23976),
            target_fps: Some(Fps::PAL_25),
            offset_ms: 1000,
        };
        let (out, loss) = apply(&cues, &cfg, None).expect("offset 不需要 FPS，也不拒绝多余 FPS");
        assert_eq!(out, vec![cue(1000, 2000), cue(3000, 4000)]);
        assert!(loss.is_lossless());
    }

    #[test]
    fn frame_snap_mode_ignores_offset_ms() {
        let cues = [cue(40, 2000)];
        let mut cfg = TimeConfig::frame_snap(Fps::PAL_50);
        cfg.offset_ms = 5000;
        let (out, loss) = apply(&cues, &cfg, None).expect("合法配置");
        assert_eq!(out, vec![cue(40, 2000)]);
        assert!(loss.approximated.is_empty());
    }

    #[test]
    fn speed_scale_mode_ignores_offset_ms() {
        let cues = [cue(0, 10_800_000)];
        let mut cfg = TimeConfig::speed_scale(Fps::PAL_25, Fps::PAL_50);
        cfg.offset_ms = 700;
        let (out, _loss) = apply(&cues, &cfg, None).expect("合法配置");
        assert_eq!(out, vec![cue(0, 5_400_000)]);
    }

    #[test]
    fn mode_labels_match_workflow_wording() {
        assert_eq!(TimeMode::Offset.label(), "Offset only");
        assert_eq!(TimeMode::FrameSnap.label(), "Snap to frame only");
        assert_eq!(TimeMode::SpeedScale.label(), "Speed/Duration scale");
        assert_eq!(TimeMode::TwoPoint.label(), "Two-point linear fit");
        assert!(TimeMode::TwoPoint.requires_target_fps());
        assert!(!TimeMode::TwoPoint.requires_source_fps());
    }
}
