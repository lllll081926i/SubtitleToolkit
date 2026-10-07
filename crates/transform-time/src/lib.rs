//! transform-time：时间/FPS 变换核心（10_TIME_FPS_SYNC.md，P0）。
//!
//! 复用 `core-model` 的 `Rational`/`Time`/`Fps`，不重造时间类型；全部纯函数，无 IO、无 GUI。
//! ADR-007：时间运算全程精确有理数，float 只出现在 `TimePreview::ratio_label` 一处显示格式化。
//! 10 §1 + 05 §4：Offset / FrameSnap / SpeedScale / TwoPoint 四种语义严格分离，绝不自动升级或降级。

#![forbid(unsafe_code)]

mod mode;
mod ops;
mod preview;
mod pts;

pub use mode::{apply, TimeConfig, TimeMode, TwoPointSpec};
pub use ops::{
    apply_frame_snap, apply_offset, apply_speed_scale, apply_two_point, min_duration_policy,
    two_point_fit, Cue, ALIGN_KEY, EXTEND_KEY,
};
pub use preview::{preview, preview_with, TimePreview};
pub use pts::shift_pts_ticks;

/// `STK-TRF-00xx` 错误码（22_LOGGING_DIAGNOSTICS.md）；避开 core-model 已占用的 2/3。
pub(crate) mod codes {
    pub(crate) const MISSING_FPS: u16 = 70;
    pub(crate) const NEED_CONTROL_POINTS: u16 = 71;
    pub(crate) const COINCIDENT_POINTS: u16 = 72;
    pub(crate) const NEGATIVE_TIME: u16 = 73;
    pub(crate) const OVERFLOW: u16 = 74;
    pub(crate) const BAD_MAPPING: u16 = 75;
    /// 与 core-model `Time::to_pts_ticks` 同义，保证 UI 看到同一套 PGS 错误码：
    /// 编号相同的代价是**标题也必须逐字一致**（core-model/tests/error_code_uniqueness.rs 把关），
    /// 变换特有的说法写在 action/detail 里。
    pub(crate) const PTS_NEGATIVE: u16 = 1;
    pub(crate) const PTS_OVERFLOW: u16 = 2;
    /// 「偏移量本身大到 90 kHz 乘法溢出」不是「PTS 超过 33bit」，所以给它自己的号。
    pub(crate) const PTS_TICKS_OVERFLOW: u16 = 12;
}

#[cfg(test)]
pub(crate) mod testutil {
    use core_model::Time;

    pub(crate) fn ms(v: i64) -> Time {
        Time::from_millis(v).expect("测试用毫秒时间必然合法")
    }

    pub(crate) fn cue(s: i64, e: i64) -> (Time, Time) {
        (ms(s), ms(e))
    }

    pub(crate) fn cues(v: &[(i64, i64)]) -> Vec<(Time, Time)> {
        v.iter().map(|&(s, e)| cue(s, e)).collect()
    }
}
