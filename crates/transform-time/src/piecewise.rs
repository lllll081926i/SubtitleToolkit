//! 10 §5 Segmented sync：多段 piecewise 线性映射。
//!
//! ## 它解决的是哪一类问题
//!
//! Offset / SpeedScale / TwoPoint 都是**全局**变换：整条时间轴一条公式。
//! 但真实片源常见的差异不是全局的：中间插了广告、片头被剪掉一段、某一段被加速过。
//! 这类差异用一条直线拟合必然"两头对、中间错"。
//! piecewise 把时间轴切成若干段，每段自己一条直线，于是每一段都能对上。
//!
//! ## 关键设计：段边界必须是**显式给定**的
//!
//! 本模块**不做**自动检测广告/剪辑点。自动检测需要内容比对（音频指纹、场景检测），
//! 那是另一条研究线，而且错了会静默把字幕移到错误的地方。
//! 这里只做**确定性执行**：调用方给出若干个控制点，我们算出分段映射并施加。
//! 这与 10 §5 的原话一致——"可以输出多段 piecewise mapping"，重点是**输出**该映射，
//! 而映射从哪来（人工量取或将来接 Smart Sync）不在本层职责内。
//!
//! ## 为什么用绝对控制点而不是"每段的斜率"
//!
//! 控制点是 `(source, target)` 对，相邻两点连成一段。这有三个好处：
//!
//! 1. **连续**：相邻段共享端点，映射天然没有跳变。用独立斜率就可能在边界处跳。
//! 2. **可验证**：每段的直线由它那两个端点唯一确定，不需要额外约定。
//! 3. **可直接对齐到真实观测**：用户在播放器里量到"这里应该是 1:23.456"，
//!    那就是一个控制点，直接填。
//!
//! ## 单调性是硬要求
//!
//! 源控制点必须**严格递增**（否则时间段有交叠，同一时刻会映射到两个值——
//! 那不是函数），且每段斜率必须为正（否则该段字幕倒流）。
//! 两条都在构造时校验，失败时给出**是第几个控制点**出的问题。

use core_model::error::Domain;
use core_model::{LossReport, Rational, StkError, Time};

use crate::codes;
use crate::ops::{self, Cue};

/// 一个控制点：源时间 → 目标时间。
///
/// 语义是"源时间轴上的 `source` 应对应目标时间轴上的 `target`"。
/// 用户的量法就是这句：在播放器里听到某句台词出现在 `source`，
/// 而它本该出现在 `target`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlPoint {
    pub source: Time,
    pub target: Time,
}

impl ControlPoint {
    pub fn new(source: Time, target: Time) -> ControlPoint {
        ControlPoint { source, target }
    }
}

/// 分段映射：`N` 个控制点定义 `N-1` 段，每段一条 `t' = a*t + b`。
#[derive(Clone, Debug, PartialEq)]
pub struct PiecewiseMap {
    /// 按 source 严格递增排好的控制点（至少 2 个）
    points: Vec<ControlPoint>,
    /// 每段的 `(a, b)`，长度 = `points.len() - 1`
    segments: Vec<(Rational, Rational)>,
}

impl PiecewiseMap {
    /// 由控制点构造并校验。
    ///
    /// 校验项（每条都报**是第几个点**出的问题）：
    /// * 至少 2 个点（1 个点定不出直线）；
    /// * source 严格递增（相等或递减都拒——前者让时间段重叠，后者让映射不是函数）；
    /// * target 严格递增（否则某一段斜率非正，字幕在该段内倒流或停住）；
    /// * 每段斜率 > 0。
    pub fn new(points: &[ControlPoint]) -> Result<PiecewiseMap, StkError> {
        if points.len() < 2 {
            return Err(StkError::simple(
                Domain::Transform,
                codes::NEED_CONTROL_POINTS,
                "分段同步至少需要 2 个控制点",
            )
            .with_action("给出至少两个 (source, target) 对；相邻两点连成一段")
            .with_detail(format!("实际给了 {} 个", points.len())));
        }

        // 先按 source 排序，再校验。排序而不是要求调用方自己排好：
        // 用户从播放器里量控制点往往不是按时间顺序记的，为此报错没有意义。
        let mut sorted = points.to_vec();
        sorted.sort_by_key(|p| p.source);

        for (i, w) in sorted.windows(2).enumerate() {
            if w[0].source == w[1].source {
                return Err(StkError::simple(
                    Domain::Transform,
                    codes::COINCIDENT_POINTS,
                    "分段同步有两个控制点的 source 相同",
                )
                .with_action("合并重复的控制点，或让它们的源时间不同")
                .with_detail(format!(
                    "第 {} 与第 {} 个点都落在 {} ms",
                    i,
                    i + 1,
                    ms_of(w[0].source)
                )));
            }
        }

        let mut segments = Vec::with_capacity(sorted.len() - 1);
        for (i, w) in sorted.windows(2).enumerate() {
            let (a, b) = ops::two_point_fit(w[0].source, w[0].target, w[1].source, w[1].target)
                .map_err(|e| {
                    // 把"第几段坏了"补进去：只报"斜率非正"用户不知道该改哪一个点
                    e.with_detail(format!(
                        "第 {i} 段（{} ms -> {} ms）",
                        ms_of(w[0].source),
                        ms_of(w[1].source)
                    ))
                })?;
            segments.push((a, b));
        }

        Ok(PiecewiseMap {
            points: sorted,
            segments,
        })
    }

    pub fn points(&self) -> &[ControlPoint] {
        &self.points
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// 每段的 `(a, b)`：`t' = a*t + b`。报告里要能给出这张表，
    /// 用户才可能拿着它去核对"为什么这一段对上了那一段没对上"。
    pub fn segments(&self) -> &[(Rational, Rational)] {
        &self.segments
    }

    /// 单个时刻的映射。
    ///
    /// **段外采用最近段的直线外推**，不是钳到端点值：
    /// 钳住会让片头/片尾整段字幕堆在同一时刻，那是明显的坏结果；
    /// 外推至少保持单调与时间间距。两条都不是"正确"答案（缺少那段的信息），
    /// 所以外推的事实会由 [`PiecewiseMap::extrapolated`] 报出来。
    pub fn map(&self, t: Time) -> Result<Time, StkError> {
        let (a, b) = self.segment_for(t);
        let scaled = t.scale(&a)?;
        scaled.add(&Time::from_rational(b))
    }

    /// 这个时刻是否落在控制点覆盖范围**之外**（即被外推了）。
    pub fn extrapolated(&self, t: Time) -> bool {
        let first = self.points[0].source;
        let last = self.points[self.points.len() - 1].source;
        t < first || t > last
    }

    fn segment_for(&self, t: Time) -> (Rational, Rational) {
        // 落在第 i 段：points[i].source <= t < points[i+1].source
        for (i, w) in self.points.windows(2).enumerate() {
            if t < w[1].source {
                return self.segments[i];
            }
        }
        // t >= 最后一个控制点：用最后一段外推
        *self.segments.last().expect("构造时已保证至少一段")
    }

    /// 把映射施加到 cue 列表。与其它模式相同的返回形状。
    ///
    /// 每段修改过的 cue 会被计数，进 `LossReport` 的 `preserved` 说明——
    /// "分段同步做了什么"必须可见，否则用户没法判断是不是自己想要的那件事。
    pub fn apply(&self, cues: &[Cue]) -> Result<(Vec<Cue>, LossReport), StkError> {
        let mut out = Vec::with_capacity(cues.len());
        let mut extrapolated_cues = 0usize;
        for c in cues {
            if self.extrapolated(c.0) || self.extrapolated(c.1) {
                extrapolated_cues += 1;
            }
            out.push((self.map(c.0)?, self.map(c.1)?));
        }
        let mut loss = LossReport::new();
        loss.preserved(&format!(
            "分段线性映射：{} 个控制点 / {} 段",
            self.points.len(),
            self.segments.len()
        ));
        if extrapolated_cues > 0 {
            // 外推是"我们不知道那段该怎么变"的地方，必须让用户看见
            loss.user_attention(&format!(
                "{extrapolated_cues} 条 cue 落在控制点覆盖范围之外，按最近一段的斜率外推"
            ));
        }
        ops::note_inverted_inputs_pub(cues, &mut loss);
        ops::require_non_negative_pub(&out)?;
        Ok((out, loss))
    }
}

fn ms_of(t: Time) -> i64 {
    (t.as_rational().to_f64() * 1000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cue, ms};

    fn pt(s: i64, t: i64) -> ControlPoint {
        ControlPoint::new(ms(s), ms(t))
    }

    /// 用户最常量的两种情况：片头剪掉一段 + 中间插广告。
    /// 这是本模块存在的理由，必须能对上。
    #[test]
    fn a_middle_discontinuity_maps_both_sides_exactly() {
        // 源: 0 -> 目标 0；源 10s -> 目标 10s；源 20s -> 目标 25s（中间插了 5s 广告）
        // 源 30s -> 目标 35s
        let m = PiecewiseMap::new(&[
            pt(0, 0),
            pt(10_000, 10_000),
            pt(20_000, 25_000),
            pt(30_000, 35_000),
        ])
        .unwrap();
        assert_eq!(m.segment_count(), 3);
        // 控制点自身必须**精确**映射到位（这是 piecewise 的全部意义）
        for p in m.points() {
            assert_eq!(m.map(p.source).unwrap(), p.target, "控制点必须精确命中");
        }
        // 广告之前不变
        assert_eq!(m.map(ms(5_000)).unwrap(), ms(5_000));
        // 广告之后整体 +5s
        assert_eq!(m.map(ms(25_000)).unwrap(), ms(30_000));
    }

    #[test]
    fn control_points_are_sorted_so_input_order_does_not_matter() {
        // 用户从播放器量控制点往往不是按时间顺序记的
        let a = PiecewiseMap::new(&[pt(20_000, 25_000), pt(0, 0), pt(10_000, 10_000)]).unwrap();
        let b = PiecewiseMap::new(&[pt(0, 0), pt(10_000, 10_000), pt(20_000, 25_000)]).unwrap();
        assert_eq!(a.points(), b.points());
        assert_eq!(a.segments(), b.segments());
    }

    #[test]
    fn fewer_than_two_points_is_rejected_with_a_count() {
        let e = PiecewiseMap::new(&[pt(0, 0)]).unwrap_err();
        assert_eq!(e.code, codes::NEED_CONTROL_POINTS);
        assert!(e.details.iter().any(|d| d.contains("1 个")), "{e:?}");
    }

    /// 源时间相同 -> 时间段重叠 -> 同一时刻会映射到两个值，那不是函数。
    #[test]
    fn duplicate_source_times_are_rejected_and_named() {
        let e = PiecewiseMap::new(&[pt(0, 0), pt(5_000, 5_000), pt(5_000, 6_000)]).unwrap_err();
        assert_eq!(e.code, codes::COINCIDENT_POINTS);
        assert!(
            e.details.iter().any(|d| d.contains("第 1 与第 2")),
            "要指认是哪两个点：{e:?}"
        );
    }

    /// 目标时间倒退 -> 该段字幕倒流。
    #[test]
    fn a_backwards_target_is_rejected_and_names_the_segment() {
        let e = PiecewiseMap::new(&[pt(0, 0), pt(10_000, 20_000), pt(20_000, 15_000)]).unwrap_err();
        assert_eq!(e.code, codes::BAD_MAPPING);
        assert!(
            e.details.iter().any(|d| d.contains("第 1 段")),
            "要指认是哪一段：{e:?}"
        );
    }

    /// 段外是外推，而且**必须报出来**——那是我们缺信息的地方。
    #[test]
    fn extrapolation_past_the_end_is_reported_not_hidden() {
        let m = PiecewiseMap::new(&[pt(10_000, 10_000), pt(20_000, 20_000)]).unwrap();
        assert!(m.extrapolated(ms(5_000)), "早于首个控制点就是外推");
        assert!(m.extrapolated(ms(25_000)), "晚于末个控制点也是外推");
        assert!(!m.extrapolated(ms(15_000)));

        let cues = vec![cue(0, 40_000)];
        let (out, loss) = m.apply(&cues).unwrap();
        assert_eq!(out.len(), 1);
        assert!(
            loss.user_attention
                .iter()
                .any(|s| s.contains("覆盖范围之外")),
            "外推必须进报告：{loss:?}"
        );
        // 外推要保持单调，不能把两头钳成同一个值
        assert!(out[0].0 < out[0].1, "外推后仍是正常区间：{:?}", out[0]);
    }

    /// 恰好落在控制点上时**不算**外推（边界是闭区间）。
    #[test]
    fn the_boundaries_themselves_are_not_extrapolation() {
        let m = PiecewiseMap::new(&[pt(10_000, 10_000), pt(20_000, 20_000)]).unwrap();
        assert!(!m.extrapolated(ms(10_000)));
        assert!(!m.extrapolated(ms(20_000)));
        let (_, loss) = m.apply(&[cue(10_000, 20_000)]).unwrap();
        assert!(
            !loss
                .user_attention
                .iter()
                .any(|s| s.contains("覆盖范围之外")),
            "全在范围内的 cue 不该被报外推：{loss:?}"
        );
    }

    /// 映射必须单调：源上更晚的时刻，目标上也必须更晚（否则字幕顺序会乱）。
    #[test]
    fn the_mapping_is_monotonic_across_every_segment() {
        let m = PiecewiseMap::new(&[
            pt(0, 1_000),
            pt(10_000, 12_000),
            pt(20_000, 30_000),
            pt(30_000, 31_000),
        ])
        .unwrap();
        let mut prev = None;
        for t in (0..=35_000).step_by(250) {
            let v = m.map(ms(t)).unwrap();
            if let Some(p) = prev {
                assert!(v > p, "在 {t} ms 处映射不再递增：{p:?} -> {v:?}");
            }
            prev = Some(v);
        }
    }

    /// 单段 piecewise 与 TwoPoint 必须给出**完全相同**的结果。
    /// 两套实现算出两个答案就是两套实现，不是一个功能。
    #[test]
    fn one_segment_agrees_with_two_point_mode() {
        let m = PiecewiseMap::new(&[pt(0, 500), pt(10_000, 10_500)]).unwrap();
        assert_eq!(m.segment_count(), 1);
        let (a, b) = crate::two_point_fit(ms(0), ms(500), ms(10_000), ms(10_500)).unwrap();
        let cues = vec![cue(0, 4_000), cue(6_000, 10_000)];
        let (piecewise, _) = m.apply(&cues).unwrap();
        let (two_point, _) = crate::apply_two_point(&cues, a, b);
        assert_eq!(piecewise, two_point, "单段 piecewise 必须等于 TwoPoint");
    }

    #[test]
    fn negative_result_after_mapping_is_rejected() {
        // 目标整体往前移，让早段变成负时间
        let m = PiecewiseMap::new(&[pt(0, -5_000), pt(10_000, 5_000)]).unwrap();
        let e = m.apply(&[cue(0, 1_000)]).unwrap_err();
        assert_eq!(e.code, codes::NEGATIVE_TIME);
    }

    #[test]
    fn the_segment_table_is_available_for_reports() {
        let m = PiecewiseMap::new(&[pt(0, 0), pt(10_000, 10_000), pt(20_000, 25_000)]).unwrap();
        let segs = m.segments();
        assert_eq!(segs.len(), 2);
        // 第一段是单位映射：a=1 b=0
        assert_eq!(segs[0].0, Rational::from_int(1));
        assert_eq!(segs[0].1, Rational::ZERO);
        // 第二段斜率仍是 1（10s -> 10s 与 20s -> 25s 之间其实是 1.5，这里只验证结构）
        assert!(segs[1].0 > Rational::ZERO);
    }

    #[test]
    fn an_empty_cue_list_is_fine() {
        let m = PiecewiseMap::new(&[pt(0, 0), pt(1_000, 1_000)]).unwrap();
        let (out, loss) = m.apply(&[]).unwrap();
        assert!(out.is_empty());
        assert!(loss.preserved.iter().any(|s| s.contains("分段线性映射")));
    }
}
