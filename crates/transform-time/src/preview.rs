//! 时间页预览（02 §3 第 4 步 / 05 §4 "Scale summary: x 0.95904"）。

use core_model::error::Domain;
use core_model::{Rational, StkError};

use crate::codes;
use crate::mode::{apply, TimeConfig, TimeMode, TwoPointSpec};
use crate::ops::{abs_rat, ceil_i64, ms_round, Cue};

/// UI 只读预览：不改变 cue，只回答"倍率多少、片尾差多少、最坏漂移多少"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimePreview {
    /// 形如 `x 0.95904`；Offset/FrameSnap 恒为 `x 1`。
    pub ratio_label: String,
    /// 末条 cue 结束时间的预计变化（ms，half-up）。
    pub end_shift_ms: i64,
    /// 所有边界变化量的最大值（ms，向上取整，绝不低估）。
    pub worst_drift_ms: i64,
}

/// 预览当前配置。`TwoPoint` 模式请改用 [`preview_with`] 传入控制点。
pub fn preview(cues: &[Cue], cfg: &TimeConfig) -> Result<TimePreview, StkError> {
    preview_with(cues, cfg, None)
}

pub fn preview_with(
    cues: &[Cue],
    cfg: &TimeConfig,
    spec: Option<&TwoPointSpec>,
) -> Result<TimePreview, StkError> {
    let (out, _loss) = apply(cues, cfg, spec)?;
    let ratio = mapping_ratio(cfg, spec)?;
    Ok(TimePreview {
        ratio_label: ratio_label(ratio),
        end_shift_ms: ms_round(last_delta(cues, &out)?)?,
        worst_drift_ms: ceil_i64(worst_drift(cues, &out)?)?,
    })
}

fn last_delta(src: &[Cue], out: &[Cue]) -> Result<Rational, StkError> {
    let (Some(a), Some(b)) = (src.last(), out.last()) else {
        return Ok(Rational::ZERO);
    };
    Ok(b.1.sub(&a.1)?.as_rational())
}

fn worst_drift(src: &[Cue], out: &[Cue]) -> Result<Rational, StkError> {
    let mut worst = Rational::ZERO;
    for (a, b) in src.iter().zip(out.iter()) {
        for d in [b.0.sub(&a.0)?.as_rational(), b.1.sub(&a.1)?.as_rational()] {
            let m = abs_rat(d)?;
            if m > worst {
                worst = m;
            }
        }
    }
    rat_ms(worst)
}

fn rat_ms(r: Rational) -> Result<Rational, StkError> {
    r.mul(&Rational::from_int(1000))
        .ok_or_else(|| StkError::simple(Domain::Transform, codes::OVERFLOW, "时间数值溢出"))
}

fn mapping_ratio(cfg: &TimeConfig, spec: Option<&TwoPointSpec>) -> Result<Rational, StkError> {
    match cfg.mode {
        // 10 §1：Offset 与 FrameSnap 都不缩放时间，倍率恒为 1。
        TimeMode::Offset | TimeMode::FrameSnap => Ok(Rational::ONE),
        TimeMode::SpeedScale => {
            let source = cfg.source_fps.ok_or_else(|| {
                StkError::simple(Domain::Transform, codes::MISSING_FPS, "缺少 source FPS")
            })?;
            let target = cfg.target_fps.ok_or_else(|| {
                StkError::simple(Domain::Transform, codes::MISSING_FPS, "缺少 target FPS")
            })?;
            source.speed_ratio(target)
        }
        TimeMode::TwoPoint => spec
            .ok_or_else(|| {
                StkError::simple(
                    Domain::Transform,
                    codes::NEED_CONTROL_POINTS,
                    "Two-point 模式缺少 4 个控制点",
                )
            })?
            .fit()
            .map(|(a, _)| a),
        // 10 §5 分段同步：各段斜率可以不同，所以**没有单个倍率**。
        // 不编一个"代表值"出来：那会让界面上显示一个不存在的数字，
        // 而用户拿着它去核对时对不上。改为报错，让调用方走分段专用的展示路径。
        TimeMode::Piecewise => Err(StkError::simple(
            Domain::Transform,
            codes::NEED_CONTROL_POINTS,
            "分段同步没有单一倍率，不能按倍率预览",
        )
        .with_action("分段同步的映射逐段不同；请按具体的源时间查它对应的目标时间")),
    }
}

/// 全 crate 唯一的 float 出现点：仅供 UI 文本（ADR-007）。
fn ratio_label(r: Rational) -> String {
    let text = format!("{:.5}", r.to_f64());
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("x {text}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cues, ms};
    use core_model::{Fps, Time};

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).expect("测试有理数合法")
    }

    #[test]
    fn preview_speed_scale_label_matches_spec() {
        let src = cues(&[(0, 10_800_000)]);
        let p = preview(&src, &TimeConfig::speed_scale(Fps::NTSC_23976, Fps::PAL_25))
            .expect("合法配置");
        assert_eq!(p.ratio_label, "x 0.95904");
    }

    #[test]
    fn preview_reports_end_shift_in_ms() {
        let src = cues(&[(0, 10_800_000)]);
        let p = preview(&src, &TimeConfig::speed_scale(Fps::NTSC_23976, Fps::PAL_25))
            .expect("合法配置");
        // 10800s * (960/1001) - 10800s = -442800/1001 s
        assert_eq!(p.end_shift_ms, -442_358);
        assert_eq!(p.worst_drift_ms, 442_358);
    }

    #[test]
    fn preview_offset_mode_is_x_one() {
        let src = cues(&[(1000, 2000), (3000, 4000)]);
        let p = preview(&src, &TimeConfig::offset(1500)).expect("offset 不需要 FPS");
        assert_eq!(p.ratio_label, "x 1");
        assert_eq!(p.end_shift_ms, 1500);
        assert_eq!(p.worst_drift_ms, 1500);
    }

    #[test]
    fn preview_offset_negative_is_signed() {
        let src = cues(&[(2000, 3000)]);
        let p = preview(&src, &TimeConfig::offset(-500)).expect("负 offset 合法");
        assert_eq!(p.end_shift_ms, -500);
        assert_eq!(p.worst_drift_ms, 500);
    }

    #[test]
    fn preview_frame_snap_is_neutral_on_grid() {
        let src = cues(&[(40, 2000), (2040, 60_000)]);
        let p = preview(&src, &TimeConfig::frame_snap(Fps::PAL_50)).expect("合法配置");
        assert_eq!(p.ratio_label, "x 1");
        assert_eq!(p.end_shift_ms, 0);
        assert_eq!(p.worst_drift_ms, 0);
    }

    #[test]
    fn preview_two_point_needs_the_spec() {
        let src = cues(&[(0, 10_000)]);
        let cfg = TimeConfig::two_point(Fps::PAL_25);
        assert!(preview(&src, &cfg).is_err());
        let spec = TwoPointSpec {
            source1: Time::ZERO,
            target1: ms(500),
            source2: Time::from_secs(10),
            target2: ms(10_500),
        };
        let p = preview_with(&src, &cfg, Some(&spec)).expect("控制点齐备");
        assert_eq!(p.ratio_label, "x 1");
        assert_eq!(p.end_shift_ms, 500);
    }

    #[test]
    fn preview_rejects_incomplete_config() {
        let src = cues(&[(0, 1000)]);
        let cfg = TimeConfig {
            mode: TimeMode::SpeedScale,
            source_fps: Some(Fps::FILM_24),
            target_fps: None,
            offset_ms: 0,
        };
        let e = preview(&src, &cfg).expect_err("缺 FPS 必须报错");
        assert_eq!(e.code, codes::MISSING_FPS);
    }

    #[test]
    fn preview_of_empty_timeline_is_zero() {
        let p = preview(&[], &TimeConfig::offset(700)).expect("空字幕也可预览");
        assert_eq!(p.end_shift_ms, 0);
        assert_eq!(p.worst_drift_ms, 0);
        assert_eq!(p.ratio_label, "x 1");
    }

    #[test]
    fn preview_worst_drift_rounds_up_not_toward_zero() {
        // 24 -> 25：末条漂移 0.16ms，half-up 后 end_shift 显示 0，worst drift 上取整为 1，UI 不会低估。
        let src = cues(&[(1, 2), (3, 4)]);
        let cfg = TimeConfig::speed_scale(Fps::FILM_24, Fps::PAL_25);
        let p = preview(&src, &cfg).expect("合法倍率");
        assert_eq!(p.ratio_label, "x 0.96");
        assert_eq!(p.end_shift_ms, 0);
        assert_eq!(p.worst_drift_ms, 1);
    }

    #[test]
    fn preview_frame_snap_reports_alignment_drift_without_ratio_change() {
        // 非网格输入：倍率仍是 x 1（没被偷偷 speed scale），但片尾差如实报告。
        let src = cues(&[(1, 1001), (2000, 4096)]);
        let p = preview(&src, &TimeConfig::frame_snap(Fps::NTSC_23976)).expect("合法配置");
        assert_eq!(p.ratio_label, "x 1");
        assert_eq!(p.end_shift_ms, -9);
        assert_eq!(p.worst_drift_ms, 9);
    }

    #[test]
    fn ratio_label_trims_trailing_zeros() {
        assert_eq!(ratio_label(Rational::ONE), "x 1");
        assert_eq!(ratio_label(rat(1, 2)), "x 0.5");
        assert_eq!(ratio_label(rat(24000, 25025)), "x 0.95904");
    }
}
