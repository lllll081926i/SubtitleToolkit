//! 四种时间变换的原子实现（10 §1/§3/§4）+ PGS fast path 时长策略。

use core_model::error::Domain;
use core_model::{Fps, LossReport, Rational, StkError, Time};

use crate::codes;

/// 一条 cue 的起止时间。
pub type Cue = (Time, Time);

/// `LossReport::count_for("user_attention", ...)` 的计数键，供 UI/其它 crate 取条数。
pub const EXTEND_KEY: &str = "结束时间延长到最小显示时长";
pub const ALIGN_KEY: &str = "cue 边界对齐到目标帧";

pub(crate) fn overflow_err() -> StkError {
    StkError::simple(Domain::Transform, codes::OVERFLOW, "时间数值溢出")
}

fn to_ms(r: Rational) -> Result<Rational, StkError> {
    r.mul(&Rational::from_int(1000)).ok_or_else(overflow_err)
}

pub(crate) fn ms_round(r: Rational) -> Result<i64, StkError> {
    to_ms(r)?.round_i64().ok_or_else(overflow_err)
}

/// 向上取整：UI 显示的"最坏漂移"不允许低估。
pub(crate) fn ceil_i64(r: Rational) -> Result<i64, StkError> {
    let neg = r.neg().ok_or_else(overflow_err)?;
    neg.floor_i64().checked_neg().ok_or_else(overflow_err)
}

pub(crate) fn abs_rat(r: Rational) -> Result<Rational, StkError> {
    if r < Rational::ZERO {
        r.neg().ok_or_else(overflow_err)
    } else {
        Ok(r)
    }
}

/// 10 §1.3 Offset：整体平移。负 offset 合法；一旦产生负时间即显式报错，**绝不静默钳到 0**。
pub fn apply_offset(cues: &[Cue], offset_ms: i64) -> Result<(Vec<Cue>, LossReport), StkError> {
    let delta = Time::from_rational(Rational::new(offset_ms, 1000).map_err(|_| overflow_err())?);
    let mut out = Vec::with_capacity(cues.len());
    for c in cues {
        out.push((c.0.add(&delta)?, c.1.add(&delta)?));
    }
    let mut loss = LossReport::new();
    loss.preserved("每条 cue 的时长（Offset 只平移时间轴）");
    note_inverted_inputs(cues, &mut loss);
    require_non_negative(&out, Some(offset_ms))?;
    Ok((out, loss))
}

/// 10 §1.2 Frame snap：只对齐边界，不缩放（AT-TIME-002）。
pub fn apply_frame_snap(cues: &[Cue], target_fps: Fps) -> (Vec<Cue>, LossReport) {
    infallible(cues, try_apply_frame_snap(cues, target_fps))
}

/// 10 §3 Speed conversion：`t' = t * source/target`，倍率是精确有理数（AT-TIME-001）。
pub fn apply_speed_scale(cues: &[Cue], source_fps: Fps, target_fps: Fps) -> (Vec<Cue>, LossReport) {
    infallible(cues, try_apply_speed_scale(cues, source_fps, target_fps))
}

/// 10 §4 两点拟合结果的应用形式。
pub fn apply_two_point(cues: &[Cue], a: Rational, b: Rational) -> (Vec<Cue>, LossReport) {
    infallible(cues, try_apply_two_point(cues, a, b))
}

/// BDSup2Sub 的 minimum duration 语义：只延长 `end` 到 `start + min`，绝不缩短、不动 `start`。
pub fn min_duration_policy(cues: &[Cue], min_ms: i64) -> (Vec<Cue>, LossReport) {
    infallible(cues, try_min_duration_policy(cues, min_ms))
}

pub(crate) fn try_apply_frame_snap(
    cues: &[Cue],
    target_fps: Fps,
) -> Result<(Vec<Cue>, LossReport), StkError> {
    let mut out = Vec::with_capacity(cues.len());
    let mut aligned = 0usize;
    for c in cues {
        let start = c.0.snap_to_frame(target_fps)?;
        let end = c.1.snap_to_frame(target_fps)?;
        if start != c.0 || end != c.1 {
            aligned += 1;
        }
        out.push((start, end));
    }
    let mut loss = LossReport::new();
    loss.preserved("影片总时长与每条 cue 时长（Frame snap 不缩放时间）");
    if aligned > 0 {
        loss.approximated(&format!(
            "{aligned} 条 cue 边界对齐到目标帧（{} fps = {}），未缩放时间轴",
            target_fps.to_label(),
            target_fps.as_rational()
        ));
        for _ in 0..aligned {
            loss.bump("user_attention", ALIGN_KEY);
        }
    }
    note_inverted_inputs(cues, &mut loss);
    require_non_negative(&out, None)?;
    Ok((out, loss))
}

pub(crate) fn try_apply_speed_scale(
    cues: &[Cue],
    source_fps: Fps,
    target_fps: Fps,
) -> Result<(Vec<Cue>, LossReport), StkError> {
    let ratio = source_fps.speed_ratio(target_fps)?;
    let mut out = Vec::with_capacity(cues.len());
    for c in cues {
        out.push((c.0.scale(&ratio)?, c.1.scale(&ratio)?));
    }
    let mut loss = LossReport::new();
    loss.approximated(&format!(
        "时间轴按倍率 x {ratio} 缩放（{} → {}，影片总时长改变）",
        source_fps.to_label(),
        target_fps.to_label()
    ));
    loss.preserved("倍率为精确有理数，缩放后可精确逆还原（无浮点累积）");
    note_inverted_inputs(cues, &mut loss);
    require_non_negative(&out, None)?;
    Ok((out, loss))
}

pub(crate) fn try_apply_two_point(
    cues: &[Cue],
    a: Rational,
    b: Rational,
) -> Result<(Vec<Cue>, LossReport), StkError> {
    let b_time = Time::from_rational(b);
    let mut out = Vec::with_capacity(cues.len());
    for c in cues {
        out.push((c.0.scale(&a)?.add(&b_time)?, c.1.scale(&a)?.add(&b_time)?));
    }
    let mut loss = LossReport::new();
    loss.approximated(&format!("两点线性拟合 t' = {a} * t + {b}"));
    loss.preserved("两个控制点精确命中；其余时间点按同一直线映射");
    note_inverted_inputs(cues, &mut loss);
    require_non_negative(&out, None)?;
    Ok((out, loss))
}

pub(crate) fn try_min_duration_policy(
    cues: &[Cue],
    min_ms: i64,
) -> Result<(Vec<Cue>, LossReport), StkError> {
    let mut loss = LossReport::new();
    loss.preserved("每条 cue 的起始时间；只延长过短的结束时间");
    if min_ms <= 0 {
        loss.user_attention("min_ms <= 0：minimum duration 策略未做任何修改");
        return Ok((cues.to_vec(), loss));
    }
    let floor = Rational::new(min_ms, 1000).map_err(|_| overflow_err())?;
    let mut out = Vec::with_capacity(cues.len());
    let mut extended = 0usize;
    for c in cues {
        let earliest_end = c.0.add(&Time::from_rational(floor))?;
        if c.1 < earliest_end {
            out.push((c.0, earliest_end));
            extended += 1;
        } else {
            out.push(*c);
        }
    }
    if extended > 0 {
        loss.approximated(&format!(
            "{extended} 条 cue 的结束时间被延长到最小显示时长 {min_ms} ms（只延长不缩短）"
        ));
        for _ in 0..extended {
            loss.bump("user_attention", EXTEND_KEY);
        }
        loss.user_attention("延长后的结束时间可能与下一条 cue 重叠，需 validator 复查");
    }
    note_inverted_inputs(cues, &mut loss);
    require_non_negative(&out, None)?;
    Ok((out, loss))
}

/// 10 §4：由两个对应点解 `t' = a*t + b`（纯有理数运算）。
pub fn two_point_fit(
    src1: Time,
    dst1: Time,
    src2: Time,
    dst2: Time,
) -> Result<(Rational, Rational), StkError> {
    let d_src = src2.sub(&src1)?;
    if d_src.as_rational().is_zero() {
        return Err(StkError::simple(
            Domain::Transform,
            codes::COINCIDENT_POINTS,
            "两个 source 控制点重合，无法确定直线",
        )
        .with_action("让两个 source 时间点不同（间隔至少 1 帧）"));
    }
    let d_dst = dst2.sub(&dst1)?;
    let a = d_dst
        .as_rational()
        .div(&d_src.as_rational())
        .ok_or_else(overflow_err)?;
    if a <= Rational::ZERO {
        return Err(StkError::simple(
            Domain::Transform,
            codes::BAD_MAPPING,
            "拟合斜率非正，会让字幕时间倒流或全部重合",
        )
        .with_action("检查控制点顺序：target 必须随 source 单调递增")
        .with_detail(format!("a = {a}")));
    }
    let b = dst1.sub(&src1.scale(&a)?)?;
    Ok((a, b.as_rational()))
}

/// 规格 4/5/6/9 给出的是非 Result 签名：溢出/越界时原样保留 cue 并把它记进报告，
/// 既不 panic、也不静默改时间。`mode::apply` 走 Result 版本，不经过这里。
fn infallible(cues: &[Cue], r: Result<(Vec<Cue>, LossReport), StkError>) -> (Vec<Cue>, LossReport) {
    match r {
        Ok(v) => v,
        Err(e) => {
            let mut loss = LossReport::new();
            loss.user_attention(&format!("时间运算越界，cue 原样保留：{}", e.code_string()));
            (cues.to_vec(), loss)
        }
    }
}

/// 供其它模块（piecewise）复用的包装。
/// 这两个收尾工作（倒序计数、负时间拒绝）必须与其它模式完全一致——
/// 两处各写一份就会在某一刻分叉。
pub(crate) fn note_inverted_inputs_pub(cues: &[Cue], loss: &mut LossReport) {
    note_inverted_inputs(cues, loss)
}

/// 同上：变换后不得出现负时间（无 offset 场景，不给"安全下限"提示）。
pub(crate) fn require_non_negative_pub(cues: &[Cue]) -> Result<(), StkError> {
    require_non_negative(cues, None)
}

fn note_inverted_inputs(cues: &[Cue], loss: &mut LossReport) {
    let inverted = cues.iter().filter(|c| c.1 < c.0).count();
    if inverted > 0 {
        loss.nonstandard(&format!("{inverted} 条输入 cue 的结束时间早于开始时间"));
    }
}

/// 负时间显式报错（可诊断要求）：给出变换后的最小起始时间，以及 Offset 场景下的安全 offset 下限。
fn require_non_negative(cues: &[Cue], offset_ms: Option<i64>) -> Result<(), StkError> {
    let min_start = cues.iter().map(|c| c.0).min().unwrap_or(Time::ZERO);
    let min_value = cues
        .iter()
        .flat_map(|c| [c.0, c.1])
        .min()
        .unwrap_or(Time::ZERO);
    if !min_value.is_negative() {
        return Ok(());
    }
    let min_start_ms = to_ms(min_start.as_rational())?;
    let min_value_ms = to_ms(min_value.as_rational())?;
    let mut err = StkError::simple(
        Domain::Transform,
        codes::NEGATIVE_TIME,
        "变换后出现负时间，已拒绝（不会自动钳到 0）",
    )
    .with_action("调整 offset 或控制点，让最早的 cue 从 0 或正时间开始");
    err = err.with_detail(format!("变换后最小起始时间 = {min_start_ms} ms"));
    err = err.with_detail(format!("变换后最小时间值 = {min_value_ms} ms"));
    if let Some(base) = offset_ms {
        let lift = Rational::ZERO.sub(&min_value_ms).ok_or_else(overflow_err)?;
        if let Some(safe) = base.checked_add(ceil_i64(lift)?) {
            err = err.with_detail(format!("安全 offset 下限 = {safe} ms"));
        }
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cues, ms};
    use core_model::Time;

    fn rat(n: i64, d: i64) -> Rational {
        Rational::new(n, d).expect("测试有理数合法")
    }

    #[test]
    fn offset_positive_shifts_every_boundary() {
        let src = cues(&[(0, 1000), (2000, 3000)]);
        let (out, loss) = apply_offset(&src, 1500).expect("正 offset 合法");
        assert_eq!(out, cues(&[(1500, 2500), (3500, 4500)]));
        assert!(loss.is_lossless());
    }

    #[test]
    fn offset_negative_is_allowed_while_times_stay_non_negative() {
        let src = cues(&[(2000, 3000)]);
        let (out, _loss) = apply_offset(&src, -2000).expect("offset 到 0 仍合法");
        assert_eq!(out, cues(&[(0, 1000)]));
    }

    #[test]
    fn offset_zero_is_identity_and_preserves_durations() {
        let src = cues(&[(40, 2000), (2040, 4000)]);
        let (out, loss) = apply_offset(&src, 0).expect("零 offset 合法");
        assert_eq!(out, src);
        assert_eq!(loss.preserved.len(), 1);
    }

    #[test]
    fn offset_producing_negative_start_errors_with_a_suggested_floor() {
        let src = cues(&[(500, 1000), (2000, 2500)]);
        let e = apply_offset(&src, -1000).expect_err("必须拒绝负起始时间");
        assert_eq!(e.domain, Domain::Transform);
        assert_eq!(e.code, codes::NEGATIVE_TIME);
        let joined = e.details.join(" | ");
        assert!(joined.contains("变换后最小起始时间 = -500 ms"), "{joined}");
        assert!(joined.contains("安全 offset 下限 = -500 ms"), "{joined}");
        assert!(e.user_action.is_some());
    }

    #[test]
    fn offset_suggestion_rounds_up_for_fractional_minimum() {
        let start = Time::from_rational(rat(1, 4_000)); // 0.25 ms
        let src = [(start, ms(1000))];
        let e = apply_offset(&src, -1).expect_err("3/4 ms 的负时间也必须拒绝");
        let joined = e.details.join(" | ");
        assert!(joined.contains("变换后最小起始时间 = -3/4 ms"), "{joined}");
        assert!(joined.contains("安全 offset 下限 = 0 ms"), "{joined}");
    }

    #[test]
    fn frame_snap_25_to_50_keeps_total_span() {
        // AT-TIME-002 正例：输入已在 25fps 网格（40ms 步长），snap 到 50fps 不改变任何边界。
        let src = cues(&[(40, 2000), (2040, 4000), (7200, 1080000)]);
        let (out, loss) = apply_frame_snap(&src, Fps::PAL_50);
        assert_eq!(out, src);
        let span_in = src[2].1.sub(&src[0].0).expect("时长可减");
        let span_out = out[2].1.sub(&out[0].0).expect("时长可减");
        assert_eq!(span_in, span_out);
        assert!(loss.approximated.is_empty());
    }

    #[test]
    fn frame_snap_is_idempotent() {
        let src = cues(&[(1234, 5678), (9999, 10001)]);
        let (once, _l) = apply_frame_snap(&src, Fps::NTSC_2997);
        let (twice, _l2) = apply_frame_snap(&once, Fps::NTSC_2997);
        assert_eq!(once, twice);
    }

    #[test]
    fn frame_snap_records_aligned_cues() {
        let src = cues(&[(1234, 5678), (9999, 10001)]);
        let (out, loss) = apply_frame_snap(&src, Fps::NTSC_2997);
        assert_eq!(out.len(), 2);
        assert_eq!(loss.count_for("user_attention", ALIGN_KEY), 2);
        assert_eq!(loss.approximated.len(), 1);
        assert!(
            loss.approximated[0].contains("对齐到目标帧"),
            "{}",
            loss.approximated[0]
        );
        for (s, e) in &out {
            assert!(!s.is_negative() && !e.is_negative());
        }
    }

    #[test]
    fn frame_snap_never_scales_durations() {
        let src = cues(&[(1, 1001), (2000, 4096)]);
        let fps = Fps::NTSC_23976;
        let frame = Rational::ONE.div(&fps.as_rational()).expect("帧长可算");
        let (out, _loss) = apply_frame_snap(&src, fps);
        for (a, b) in src.iter().zip(out.iter()) {
            let d_in = a.1.sub(&a.0).expect("时长可减");
            let d_out = b.1.sub(&b.0).expect("时长可减");
            let diff = d_out
                .as_rational()
                .sub(&d_in.as_rational())
                .expect("差值可减");
            // 单条时长变化不超过一帧（纯对齐误差上界），绝不是整体缩放。
            assert!(abs_rat(diff).expect("绝对值可算") <= frame, "{diff}");
        }
    }

    #[test]
    fn speed_scale_3h_end_is_the_exact_rational_result() {
        // AT-TIME-001：23.976 -> 25，3 小时 cue 的末尾时间必须等于 t*24000/(1001*25) 的精确有理数。
        let src = cues(&[(0, 10_800_000)]);
        let (out, loss) = apply_speed_scale(&src, Fps::NTSC_23976, Fps::PAL_25);
        let expected_ratio = rat(24000, 1001)
            .div(&Rational::from_int(25))
            .expect("期望倍率");
        let expected = Time::from_secs(3 * 3600)
            .scale(&expected_ratio)
            .expect("期望值可算");
        assert_eq!(out[0].1.as_rational(), expected.as_rational());
        assert_eq!(out[0].1.as_rational(), rat(10_368_000, 1001));
        assert!(
            loss.approximated[0].contains("x 960/1001"),
            "{}",
            loss.approximated[0]
        );
        assert!(
            loss.approximated[0].contains("23.976 → 25"),
            "{}",
            loss.approximated[0]
        );
    }

    #[test]
    fn speed_scale_has_no_accumulated_error_over_a_long_run() {
        let src = cues(&(0..1000).map(|i| (i * 41, i * 41 + 40)).collect::<Vec<_>>());
        let (out, _loss) = apply_speed_scale(&src, Fps::NTSC_23976, Fps::PAL_25);
        let ratio = Fps::NTSC_23976.speed_ratio(Fps::PAL_25).expect("倍率可算");
        for (c, o) in src.iter().zip(out.iter()) {
            let want_start = c.0.scale(&ratio).expect("起点可缩放");
            let want_end = c.1.scale(&ratio).expect("终点可缩放");
            assert_eq!(o.0, want_start);
            assert_eq!(o.1, want_end);
        }
        // 第 999 条与闭式解一致：证明不是逐条累加出来的近似值。
        let last = Time::from_rational(rat(999 * 41 + 40, 1000))
            .scale(&ratio)
            .expect("闭式解可算");
        assert_eq!(out[999].1, last);
    }

    #[test]
    fn speed_scale_inverse_roundtrip_is_exact() {
        let src = cues(&[(0, 10_800_000), (12345, 54321)]);
        let (scaled, _l) = apply_speed_scale(&src, Fps::NTSC_23976, Fps::PAL_25);
        let (back, _l2) = apply_speed_scale(&scaled, Fps::PAL_25, Fps::NTSC_23976);
        assert_eq!(back, src);
    }

    #[test]
    fn speed_scale_25_to_50_halves_every_duration() {
        // 与 frame snap 形成对照：同样 25 -> 50，SpeedScale 会改变时长。
        let src = cues(&[(0, 4000)]);
        let (out, _loss) = apply_speed_scale(&src, Fps::PAL_25, Fps::PAL_50);
        assert_eq!(out, cues(&[(0, 2000)]));
    }

    #[test]
    fn two_point_fit_recovers_unit_slope_and_offset() {
        let (a, b) =
            two_point_fit(Time::ZERO, ms(500), Time::from_secs(10), ms(10_500)).expect("拟合成功");
        assert_eq!(a, Rational::ONE);
        assert_eq!(b, rat(1, 2));
    }

    #[test]
    fn two_point_fit_recovers_scale_and_offset() {
        let (a, b) = two_point_fit(ms(1000), ms(2000), ms(3000), ms(4500)).expect("拟合成功");
        assert_eq!(a, rat(5, 4));
        assert_eq!(b, rat(3, 4));
        let (out, _loss) = apply_two_point(&cues(&[(1000, 3000)]), a, b);
        assert_eq!(out, cues(&[(2000, 4500)]));
    }

    #[test]
    fn two_point_coincident_source_is_an_error() {
        let e = two_point_fit(ms(1000), ms(2000), ms(1000), ms(9000)).expect_err("重合点不可拟合");
        assert_eq!(e.code, codes::COINCIDENT_POINTS);
        assert!(e.user_action.is_some());
    }

    #[test]
    fn two_point_non_positive_slope_is_an_error() {
        assert_eq!(
            two_point_fit(Time::ZERO, ms(9000), Time::from_secs(10), ms(2000))
                .expect_err("倒序控制点")
                .code,
            codes::BAD_MAPPING
        );
        assert_eq!(
            two_point_fit(Time::ZERO, ms(1000), Time::from_secs(10), ms(1000))
                .expect_err("零斜率")
                .code,
            codes::BAD_MAPPING
        );
    }

    #[test]
    fn two_point_hits_both_control_points_exactly() {
        let (a, b) =
            two_point_fit(ms(500), ms(500), Time::from_secs(10), ms(10_500)).expect("拟合成功");
        let (out, _loss) = apply_two_point(&cues(&[(500, 10_000)]), a, b);
        assert_eq!(out[0].0, ms(500));
        assert_eq!(out[0].1, ms(10_500));
    }

    #[test]
    fn min_duration_extends_short_cues_only() {
        let src = cues(&[(0, 100), (1000, 5000), (6000, 6100)]);
        let (out, loss) = min_duration_policy(&src, 250);
        assert_eq!(out, cues(&[(0, 250), (1000, 5000), (6000, 6250)]));
        assert_eq!(loss.count_for("user_attention", EXTEND_KEY), 2);
        assert!(
            loss.approximated[0].starts_with("2 条"),
            "{}",
            loss.approximated[0]
        );
    }

    #[test]
    fn min_duration_never_shortens_or_moves_start() {
        let src = cues(&[(1000, 5000)]);
        let (out, loss) = min_duration_policy(&src, 200);
        assert_eq!(out, src);
        assert!(loss.approximated.is_empty());
        assert_eq!(loss.count_for("user_attention", EXTEND_KEY), 0);
    }

    #[test]
    fn min_duration_with_non_positive_floor_is_a_reported_noop() {
        let src = cues(&[(0, 10)]);
        let (out, loss) = min_duration_policy(&src, 0);
        assert_eq!(out, src);
        assert_eq!(loss.user_attention.len(), 1);
        let (out2, loss2) = min_duration_policy(&src, -5);
        assert_eq!(out2, src);
        assert_eq!(loss2.user_attention.len(), 1);
    }

    #[test]
    fn inverted_input_cues_are_reported_not_panic() {
        let src = cues(&[(5000, 1000)]);
        let (_out, loss) = apply_frame_snap(&src, Fps::PAL_25);
        assert_eq!(loss.nonstandard.len(), 1);
        assert!(
            loss.nonstandard[0].contains("1 条"),
            "{}",
            loss.nonstandard[0]
        );
    }

    #[test]
    fn negative_input_after_speed_scale_is_rejected() {
        let src = [(ms(-1000), ms(1000))];
        let (_out, loss) = apply_speed_scale(&src, Fps::NTSC_23976, Fps::PAL_25);
        assert!(
            loss.user_attention[0].contains("STK-TRF-0073"),
            "{:?}",
            loss.user_attention
        );
    }

    #[test]
    fn mode_is_never_inferred_from_25_to_50() {
        // 05 §4：25 -> 50 且用户选 FrameSnap 时，输出总时长与输入完全一致，证明没有偷偷 speed scale。
        let src = cues(&[(40, 2000), (2040, 60_000)]);
        let (snapped, _l) = apply_frame_snap(&src, Fps::PAL_50);
        assert_eq!(snapped, src);
        assert_eq!(
            snapped[1].1.sub(&snapped[0].0).expect("跨度"),
            src[1].1.sub(&src[0].0).expect("跨度")
        );
        let (scaled, _l2) = apply_speed_scale(&src, Fps::PAL_25, Fps::PAL_50);
        assert_ne!(scaled, src);
    }
}
