//! PGS fast path 的时间戳平移（10 §6）：90k ticks 精确加减，越界显式报错，绝不回绕。

use core_model::error::Domain;
use core_model::time::PTS_MAX_TICKS;
use core_model::StkError;

use crate::codes;

/// 1 ms = 90 ticks（整系数，无浮点）。
const TICKS_PER_MS: i64 = 90;

/// SUP/PGS offset 快速通道：只做 `ticks + delay_ms * 90`，segment bytes 其余部分不动。
///
/// 负结果与超过 33 bit 的结果都必须报错——MPEG PTS 是模 2^33 的回绕计数器，
/// 静默截断会让字幕跳到片头，因此这里禁止任何回绕（10 §6 / AT-PGS-001 的 +1000ms 语义）。
pub fn shift_pts_ticks(ticks: i64, delay_ms: i64) -> Result<i64, StkError> {
    check_range(ticks)?;
    let delta = delay_ms.checked_mul(TICKS_PER_MS).ok_or_else(bad_delay)?;
    let shifted = ticks.checked_add(delta).ok_or_else(bad_delay)?;
    check_range(shifted)
        .map_err(|e| e.with_detail(format!("原始 ticks = {ticks}，delay = {delay_ms} ms")))
}

fn check_range(ticks: i64) -> Result<i64, StkError> {
    if ticks < 0 {
        return Err(
            StkError::simple(Domain::Pgs, codes::PTS_NEGATIVE, "PTS 为负")
                .with_action("减小负向 offset；fast path 不允许回绕或钳位")
                .with_detail(format!("偏移后 PTS ticks = {ticks}")),
        );
    }
    if ticks >= PTS_MAX_TICKS {
        return Err(
            StkError::simple(Domain::Pgs, codes::PTS_OVERFLOW, "PTS 超过 33bit 上限")
                .with_action("减小正向 offset")
                .with_detail(format!(
                    "偏移后 PTS ticks = {ticks}，上限 = {PTS_MAX_TICKS}"
                )),
        );
    }
    Ok(ticks)
}

fn bad_delay() -> StkError {
    StkError::simple(
        Domain::Pgs,
        codes::PTS_TICKS_OVERFLOW,
        "PGS offset 数值过大，90 kHz ticks 运算溢出",
    )
    .with_action("把 offset 限制在合理范围内")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_one_thousand_ms_is_ninety_thousand_ticks() {
        // AT-PGS-001 的时间层：+1000ms -> +90000 ticks。
        assert_eq!(shift_pts_ticks(0, 1000), Ok(90_000));
        assert_eq!(shift_pts_ticks(123_456, 1000), Ok(213_456));
    }

    #[test]
    fn zero_delay_is_identity() {
        assert_eq!(shift_pts_ticks(45_000, 0), Ok(45_000));
    }

    #[test]
    fn shifting_below_zero_is_rejected_not_wrapped() {
        let e = shift_pts_ticks(89, -1).expect_err("必须拒绝 -1 tick");
        assert_eq!(e.domain, Domain::Pgs);
        assert_eq!(e.code, codes::PTS_NEGATIVE);
        assert!(e.details.iter().any(|d| d.contains("-1")));
        assert_eq!(shift_pts_ticks(90, -1), Ok(0));
    }

    #[test]
    fn negative_input_ticks_is_rejected() {
        assert_eq!(
            shift_pts_ticks(-1, 0).expect_err("输入本身为负").code,
            codes::PTS_NEGATIVE
        );
    }

    #[test]
    fn shifting_past_33_bits_is_rejected_not_truncated() {
        let e = shift_pts_ticks(PTS_MAX_TICKS - 90, 1).expect_err("必须拒绝越界");
        assert_eq!(e.code, codes::PTS_OVERFLOW);
        assert!(e.details.iter().any(|d| d.contains("上限")));
        assert_eq!(
            shift_pts_ticks(PTS_MAX_TICKS - 90, 0),
            Ok(PTS_MAX_TICKS - 90)
        );
    }

    #[test]
    fn input_already_over_the_limit_is_rejected() {
        assert_eq!(
            shift_pts_ticks(PTS_MAX_TICKS, 0)
                .expect_err("输入越界")
                .code,
            codes::PTS_OVERFLOW
        );
    }

    #[test]
    fn absurd_delay_errors_instead_of_panicking() {
        assert!(shift_pts_ticks(0, i64::MAX).is_err());
        assert!(shift_pts_ticks(0, i64::MIN).is_err());
    }
}
