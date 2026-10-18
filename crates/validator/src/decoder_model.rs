//! 第 5 层：解码器模型（13 §6 "Decoder Model"）。
//!
//! ## 这一层为什么能实现，以及实现的是哪一套
//!
//! 13 §6 把第 5 层列为"coded object buffer、window/object decode time、acquisition"。
//! 理论模型（US7620297B2 / US8638861B2，FFmpeg `supenc.c` 的编码侧实现）给的是
//! **编码器的前向模型**：`PCS/WDS/PDS` 的 `DTS = PTS - decode_duration`，
//! 首片 ODS 的 `PTS = DTS + ceil(w*h*FREQ/Rd)`，`Rd = 16e6`。
//!
//! 拿这套公式去校验**真实的商业蓝光流**会得出 0% 命中 ——
//! 本机实测（98.97 MB 的真实 SUP，9951 个显示集）：
//!
//! | 量 | 实测 |
//! | --- | --- |
//! | `PDS` 的 `PTS - DTS` | 恒为 **0**（8558/8558） |
//! | `END` 的 `PTS - DTS` | 恒为 **0**（9951/9951） |
//! | `ODS` 的 `PTS - DTS` | 1..3494 tick，与 `w*h*FREQ/Rd` **无相关**（11286 条 0% 命中） |
//!
//! 所以本层校验的是**真实流满足的那套不变式**，不是编码器的前向公式。
//! 用一套"理论上应该这样"的公式去报错，只会给所有真实文件刷一堆假阳性。
//!
//! ## 实测出来的不变式（每条都在真实语料上 100% 成立）
//!
//! 1. 每个显示集内，**所有段的 PTS ≤ 该集 PCS 的 PTS**（9951/9951）。
//!    PCS 是这一集的呈现时刻，其余段是它的准备数据。
//! 2. 每个显示集内，**所有段的 DTS ≤ 自己的 PTS**（9951/9951）。
//!    解码不能晚于呈现。
//! 3. **全局 DTS 单调不减**（49659 段，0 次递减）。
//!    解码顺序必须单调 —— 这是"能不能顺序解码"的必要条件。
//!    PTS **不**单调（40% 的相邻对递减），那是对的：数据可以提前传。
//! 4. `END` 的 PTS 等于同集最后一个 ODS 的 PTS（8520/8520）。
//!    END 收尾，不引入新时刻。
//! 5. `PDS` 与 `END` 的 `DTS == PTS`（100%）。
//!    这两个段没有解码耗时。
//!
//! ## 本层**不做**的事
//!
//! * 不算 coded object buffer 的**占用曲线**：真实流不带"缓冲区大小"这个字段，
//!   而 `Rx`/`Rd` 这些速率在流里无从观测。可以按假设的速率算一条曲线，
//!   但那是在验我们的假设，不是在验这个文件。
//! * 不判 acquisition point / seamless 分支：需要 PCS 的 composition state 语义
//!   与"哪些集可作为入口"的严格定义，当前没有真盘裁决（13 §10 已列该缺口）。
//!
//! 报告里这两条的缺失会写进 `scope`，不会因为"跑过了层 5"就宣称整层完成。

use codec_pgs::display_set::parse_pcs;
use codec_pgs::segment::{Segment, SEG_END, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};

use crate::report::{Severity, ValidationReport};

/// 90 kHz 时钟。
pub const FREQ: i64 = 90_000;

/// 一条解码器模型发现。带上定位信息，报告要能指出"是哪一集".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimingIssue {
    pub rule: &'static str,
    pub severity: Severity,
    /// 显示集序号（从 0 起，按 END 收尾分组）。
    pub set_index: usize,
    /// 段在整条流里的下标；不在集合内时为 `None`。
    pub segment: Option<usize>,
    pub message: String,
}

/// 一个显示集（按 END 收尾分组）。
#[derive(Clone, Debug)]
struct SetSpan {
    /// 该集内所有段在整条流里的下标范围（闭区间）
    first: usize,
    last: usize,
}

/// 按 END 收尾切成显示集。**永不失败**：校验器的职责是报告坏文件，
/// 所以未收尾的尾巴也要算一集（`codec_pgs::group_display_sets` 会直接报错，
/// 那条路是解析器用的，不是这里用的）。
fn spans(segments: &[Segment]) -> Vec<SetSpan> {
    let mut out = Vec::new();
    let mut first = 0usize;
    for (i, s) in segments.iter().enumerate() {
        if s.seg_type == SEG_END {
            out.push(SetSpan { first, last: i });
            first = i + 1;
        }
    }
    if first < segments.len() {
        out.push(SetSpan {
            first,
            last: segments.len() - 1,
        });
    }
    out
}

/// 跑第 5 层。返回 (报告, 是否真的检查了内容)。
///
/// 第二个返回值为 `false` 时表示"流里一段都没有"，此时不该声称跑过。
pub fn decoder_model(segments: &[Segment]) -> (ValidationReport, bool) {
    let mut r = ValidationReport::new();
    if segments.is_empty() {
        r.add(
            "PGS.DEC.00",
            Severity::Warning,
            "没有任何段，解码器模型无从执行",
            "空流不进层 5 的结论",
        );
        return (r, false);
    }
    let sets = spans(segments);

    let mut issues: Vec<TimingIssue> = Vec::new();

    // 不变式 1/2/4/5：集合内
    for (si, sp) in sets.iter().enumerate() {
        let body = &segments[sp.first..=sp.last];
        // 该集的 PCS 呈现时刻：取集内第一个 PCS。
        // 没有 PCS 的集（结构层已经在报）在这里跳过时间判定，
        // 否则会拿一个编出来的基准去报错。
        let Some(pcs_pts) = body.iter().find(|s| s.seg_type == SEG_PCS).map(|s| s.pts) else {
            continue;
        };

        for (off, s) in body.iter().enumerate() {
            let idx = sp.first + off;

            // 不变式 2：DTS <= PTS
            if s.dts > s.pts {
                issues.push(TimingIssue {
                    rule: "PGS.DEC.02",
                    severity: Severity::Error,
                    set_index: si,
                    segment: Some(idx),
                    message: format!(
                        "段 DTS({}) 晚于 PTS({})，解码晚于呈现（差 {} tick）",
                        s.dts,
                        s.pts,
                        s.dts - s.pts
                    ),
                });
            }

            // 不变式 1：段 PTS <= 该集 PCS 的 PTS
            if s.pts > pcs_pts {
                issues.push(TimingIssue {
                    rule: "PGS.DEC.01",
                    severity: Severity::Warning,
                    set_index: si,
                    segment: Some(idx),
                    message: format!(
                        "段 PTS({}) 晚于本集 PCS 的 PTS({})，超 {} tick（{:.1} ms）",
                        s.pts,
                        pcs_pts,
                        s.pts - pcs_pts,
                        (s.pts - pcs_pts) as f64 / FREQ as f64 * 1000.0
                    ),
                });
            }

            // 不变式 5：PDS / END 的 DTS == PTS
            // 这两类段没有解码耗时，真实流 100% 满足。
            if matches!(s.seg_type, SEG_PDS | SEG_END) && s.dts != s.pts {
                issues.push(TimingIssue {
                    rule: "PGS.DEC.04",
                    severity: Severity::Warning,
                    set_index: si,
                    segment: Some(idx),
                    message: format!(
                        "{} 的 DTS({}) != PTS({})；这类段没有解码耗时",
                        seg_name(s.seg_type),
                        s.dts,
                        s.pts
                    ),
                });
            }
        }

        // 不变式 4：END.PTS == 最后一个 ODS 的 PTS
        let last_ods = body.iter().rev().find(|s| s.seg_type == SEG_ODS);
        let end = body.iter().find(|s| s.seg_type == SEG_END);
        if let (Some(o), Some(e)) = (last_ods, end) {
            if e.pts != o.pts {
                issues.push(TimingIssue {
                    rule: "PGS.DEC.03",
                    severity: Severity::Warning,
                    set_index: si,
                    segment: None,
                    message: format!(
                        "END 的 PTS({}) != 最后一个 ODS 的 PTS({})，差 {} tick",
                        e.pts,
                        o.pts,
                        e.pts - o.pts
                    ),
                });
            }
        }
    }

    // 不变式 3：全局 DTS 单调不减
    let mut drops = 0usize;
    let mut first_drop: Option<(usize, i64, i64)> = None;
    for i in 1..segments.len() {
        if segments[i].dts < segments[i - 1].dts {
            drops += 1;
            if first_drop.is_none() {
                first_drop = Some((i, segments[i - 1].dts, segments[i].dts));
            }
        }
    }
    if let Some((at, prev, cur)) = first_drop {
        issues.push(TimingIssue {
            rule: "PGS.DEC.05",
            severity: Severity::Error,
            set_index: 0,
            segment: Some(at),
            message: format!(
                "DTS 在整条流里递减了 {drops} 次（首次在段 {at}：{prev} -> {cur}）；\
                 解码顺序必须单调，否则播放器无法顺序解码"
            ),
        });
    }

    // 汇总成报告：同类规则只出一条 finding，但把**次数与首个位置**带上
    emit(&mut r, &issues, segments.len(), sets.len());
    (r, true)
}

fn seg_name(t: u8) -> &'static str {
    match t {
        SEG_PCS => "PCS",
        SEG_WDS => "WDS",
        SEG_PDS => "PDS",
        SEG_ODS => "ODS",
        SEG_END => "END",
        _ => "未知段",
    }
}

fn emit(r: &mut ValidationReport, issues: &[TimingIssue], total_segments: usize, sets: usize) {
    let mut by_rule: std::collections::BTreeMap<&'static str, Vec<&TimingIssue>> =
        std::collections::BTreeMap::new();
    for i in issues {
        by_rule.entry(i.rule).or_default().push(i);
    }

    if by_rule.is_empty() {
        r.add(
            "PGS.DEC.10",
            Severity::Info,
            format!("解码器模型 5 条不变式全部成立（{sets} 个显示集，{total_segments} 段）"),
            "PTS 集合内不超 PCS、DTS 不超 PTS、全局 DTS 单调、END 对齐最后 ODS、PDS/END 无解码耗时",
        );
    } else {
        for (rule, list) in &by_rule {
            let first = list[0];
            let head = if list.len() == 1 {
                first.message.clone()
            } else {
                format!("{}（同类共 {} 处）", first.message, list.len())
            };
            let f = r.add(rule, first.severity, head, scope_note(rule));
            // 定位信息要紧：报告要能指出是哪一集、哪个段
            f.display_set = Some(first.set_index);
            f.segment_offset = first.segment;
        }
    }

    // 本层没做的事必须写出来：跑过层 5 != 整层完成。
    r.add(
        "PGS.DEC.20",
        Severity::Info,
        "层 5 已执行的不变式集合（5 条），未含缓冲区占用曲线与 acquisition 分支",
        "真实流不带缓冲区尺寸/速率字段，占用曲线只能按假设算；composition state 的 \
         acquisition/seamless 语义仍需真盘裁决（13 §10）",
    );
}

fn scope_note(rule: &str) -> &'static str {
    match rule {
        "PGS.DEC.01" => "PCS 是这一集的呈现时刻，其余段应是它的准备数据（真实语料 100% 成立）",
        "PGS.DEC.02" => "解码不能晚于呈现；不满足意味着播放器无法按时出画",
        "PGS.DEC.03" => "END 只收尾，不引入新时刻（真实语料 100% 成立）",
        "PGS.DEC.04" => "PDS/END 没有解码耗时（真实语料 100% 成立）",
        "PGS.DEC.05" => "DTS 必须单调，这是顺序解码的必要条件（真实语料 0 次递减）",
        _ => "",
    }
}

/// 给测试与报告用的最小摘要。
pub fn summary(segments: &[Segment]) -> DecoderSummary {
    let sets = spans(segments);
    let pts_le_pcs = {
        let mut ok = 0usize;
        let mut total = 0usize;
        for sp in &sets {
            let body = &segments[sp.first..=sp.last];
            let Some(pcs) = body.iter().find(|s| s.seg_type == SEG_PCS).map(|s| s.pts) else {
                continue;
            };
            for s in body {
                total += 1;
                if s.pts <= pcs {
                    ok += 1;
                }
            }
        }
        (ok, total)
    };
    let dts_le_pts = {
        let mut ok = 0usize;
        for s in segments {
            if s.dts <= s.pts {
                ok += 1;
            }
        }
        (ok, segments.len())
    };
    let dts_drops = (1..segments.len())
        .filter(|&i| segments[i].dts < segments[i - 1].dts)
        .count();
    DecoderSummary {
        sets: sets.len(),
        segments: segments.len(),
        pts_le_pcs_violations: pts_le_pcs.1 - pts_le_pcs.0,
        dts_gt_pts_violations: dts_le_pts.1 - dts_le_pts.0,
        dts_drops,
    }
}

/// 一层的可核对摘要：报告里要能给出"检查了多少、违反了多少"，
/// 而不是只有一句"跑过了"。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DecoderSummary {
    pub sets: usize,
    pub segments: usize,
    /// 段 PTS 晚于本集 PCS 的次数
    pub pts_le_pcs_violations: usize,
    /// DTS 晚于 PTS 的次数
    pub dts_gt_pts_violations: usize,
    /// 全局 DTS 递减次数
    pub dts_drops: usize,
}

/// PCS 的 composition state 提取（给 acquisition 分析用；当前只用于报告分布）。
pub fn composition_states(segments: &[Segment]) -> std::collections::BTreeMap<String, usize> {
    let mut m = std::collections::BTreeMap::new();
    for s in segments.iter().filter(|s| s.seg_type == SEG_PCS) {
        if let Ok(p) = parse_pcs(&s.payload) {
            *m.entry(format!("0x{:02X}", p.state)).or_insert(0) += 1;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_pgs::segment::Segment;

    fn seg(pts: i64, dts: i64, t: u8) -> Segment {
        Segment {
            pts,
            dts,
            seg_type: t,
            payload: Vec::new(),
            offset: None,
        }
    }

    /// 一份教科书式干净的显示集，**按真实流的传输顺序**：
    ///
    /// ```text
    /// PCS -> WDS -> PDS -> ODS -> END
    /// ```
    ///
    /// 这是本机在 98.97 MB 真实商业 SUP 上统计出的最常见骨架
    /// （8520/9951 个显示集就是这个形状）。
    ///
    /// 把它写成“按时间顺序”（PDS 先、PCS 后）是错的：真实流按**传输顺序**排，
    /// 而 PCS 带的是本集的呈现时刻（最高），END 收尾。
    fn clean_set(base: i64) -> Vec<Segment> {
        // 本集的共同解码时刻；PCS 的 PTS 是呈现时刻
        let dts = base - 500;
        vec![
            seg(base, dts, SEG_PCS),
            seg(base - 100, dts, SEG_WDS),
            seg(dts, dts, SEG_PDS),
            seg(dts + 91, dts, SEG_ODS),
            seg(dts + 91, dts + 91, SEG_END),
        ]
    }

    #[test]
    fn a_clean_stream_reports_no_violations() {
        let segs = [clean_set(1000), clean_set(5000)].concat();
        let (r, checked) = decoder_model(&segs);
        assert!(checked);
        assert!(
            !r.findings.iter().any(|f| f.severity >= Severity::Warning),
            "干净流不该有 warning 以上：{:?}",
            r.findings
        );
        assert!(
            r.findings.iter().any(|f| f.check == "PGS.DEC.10"),
            "应当有一条'全部成立'的 info：{:?}",
            r.findings
        );
    }

    /// 每条不变式各造一个反例，确认**确实会被抓**。
    /// 只测正向（干净流不报）是不够的：一个永远返回空的实现也能过。
    ///
    /// 每个反例只违反一条：夹具顺序取真实流的骨架（PCS -> WDS -> PDS -> ODS -> END），
    /// 改动的只是那一个量。
    #[test]
    fn each_invariant_has_a_working_counterexample() {
        let cases: Vec<(&str, &str, Vec<Segment>)> = vec![
            (
                "段 PTS 晚于本集 PCS",
                "PGS.DEC.01",
                vec![
                    seg(1000, 500, SEG_PCS),
                    seg(900, 500, SEG_WDS),
                    seg(500, 500, SEG_PDS),
                    // ODS 的 PTS 高过 PCS
                    seg(1200, 500, SEG_ODS),
                    seg(1200, 1200, SEG_END),
                ],
            ),
            (
                "DTS 晚于 PTS",
                "PGS.DEC.02",
                vec![
                    seg(2000, 500, SEG_PCS),
                    seg(1900, 500, SEG_WDS),
                    seg(500, 500, SEG_PDS),
                    // ODS 的 DTS 超过自己的 PTS。
                    // 它必然让 END 的 DTS 也得跟上（否则 DEC.05 递减），
                    // 于是 END 的 DTS != PTS 也会被 DEC.04 报到——
                    // 这两条的共现是构造上的必然，不是实现串了。
                    seg(591, 700, SEG_ODS),
                    seg(591, 700, SEG_END),
                ],
            ),
            (
                "END 与最后 ODS 不对齐",
                "PGS.DEC.03",
                vec![
                    seg(2000, 500, SEG_PCS),
                    seg(1900, 500, SEG_WDS),
                    seg(500, 500, SEG_PDS),
                    seg(591, 500, SEG_ODS),
                    // END.PTS 应当等于 591；写成 950 同时让它的 DTS(500) != PTS(950)，
                    // 于是 DEC.04 也会报——这两条本就是同一件事的两面：
                    // “END 不引入新时刻” 就蕴含了 “END 没有解码耗时”。
                    seg(950, 500, SEG_END),
                ],
            ),
            (
                "PDS 有解码耗时",
                "PGS.DEC.04",
                vec![
                    seg(2000, 500, SEG_PCS),
                    seg(1900, 500, SEG_WDS),
                    // PDS “有解码耗时”：DTS 比 PTS 晚 1 tick。
                    // DTS 序列 500,500,501,501,501 —— 单调不减，所以不连带 DEC.05；
                    // 但 PDS.DTS > PDS.PTS 就是 DEC.02 的定义，两条必然共现。
                    seg(500, 501, SEG_PDS),
                    seg(591, 501, SEG_ODS),
                    seg(591, 591, SEG_END),
                ],
            ),
            (
                "DTS 全局递减",
                "PGS.DEC.05",
                vec![
                    seg(5000, 4500, SEG_PCS),
                    seg(4900, 4500, SEG_WDS),
                    seg(4500, 4500, SEG_PDS),
                    seg(4591, 4500, SEG_ODS),
                    seg(4591, 4591, SEG_END),
                    // 下一集的 DTS 比上面小 -> 递减
                    seg(2000, 1500, SEG_PCS),
                    seg(1900, 1500, SEG_WDS),
                    seg(1500, 1500, SEG_PDS),
                    seg(1591, 1500, SEG_ODS),
                    seg(1591, 1591, SEG_END),
                ],
            ),
        ];
        // 每个反例允许的共现规则（构造上难以避免的重叠，需要明说而不是放宽断言）。
        let allowed_together: &[(&str, &[&str])] = &[
            ("PGS.DEC.02", &["PGS.DEC.04"]),
            ("PGS.DEC.03", &["PGS.DEC.04"]),
            ("PGS.DEC.04", &["PGS.DEC.02"]),
        ];
        for (name, rule, segs) in cases {
            let (r, _) = decoder_model(&segs);
            assert!(
                r.findings.iter().any(|f| f.check == rule),
                "「{name}」必须被报成 {rule}；实际发现：{:?}",
                r.findings
                    .iter()
                    .map(|f| (f.check, f.severity))
                    .collect::<Vec<_>>()
            );
            let extra = allowed_together
                .iter()
                .find(|(r_, _)| *r_ == rule)
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            let others: Vec<&str> = r
                .findings
                .iter()
                .map(|f| f.check)
                .filter(|c| {
                    c.starts_with("PGS.DEC.")
                        && *c != rule
                        && *c != "PGS.DEC.20"
                        && !extra.contains(c)
                })
                .collect();
            assert!(
                others.is_empty(),
                "「{name}」只该违反 {rule}（可共现 {extra:?}），却也报了 {others:?}"
            );
        }
    }

    /// 不变式之间存在**结构性的蕴含关系**，这一点写下来避免以后误以为实现串了：
    ///
    /// * `END.PTS == 最后 ODS.PTS` 且在集内 `DTS <= PTS` ⇒ `END.DTS == END.PTS`
    ///   （因为 `END.DTS` 不大于 `END.PTS`，而两者又都得对齐同一个值）。
    ///   所以 DEC.03 与 DEC.04 往往一起出现——它们是同一件事的两面，
    ///   不是两条独立的检查。
    #[test]
    fn key_invariants_are_related_and_that_is_documented() {
        let dts = 500;
        let segs = vec![
            seg(2000, dts, SEG_PCS),
            seg(500, dts, SEG_PDS),
            seg(591, dts, SEG_ODS),
            // END 对齐最后 ODS 且无耗时 -> 两条都不报
            seg(591, 591, SEG_END),
        ];
        let (r, _) = decoder_model(&segs);
        for code in ["PGS.DEC.03", "PGS.DEC.04"] {
            assert!(
                !r.findings
                    .iter()
                    .any(|f| f.check == code && f.severity >= Severity::Warning),
                "对齐时不该报 {code}：{:?}",
                r.findings
            );
        }
    }

    #[test]
    fn an_empty_stream_says_it_did_not_check() {
        let (r, checked) = decoder_model(&[]);
        assert!(!checked, "空流不能声称跑过层 5");
        assert!(r.findings.iter().any(|f| f.check == "PGS.DEC.00"));
    }

    /// 本层没做的事必须写在报告里，不能靠"跑过了"暗示整层完成。
    #[test]
    fn the_report_states_what_layer_5_did_not_check() {
        let segs = clean_set(1000);
        let (r, _) = decoder_model(&segs);
        let note = r
            .findings
            .iter()
            .find(|f| f.check == "PGS.DEC.20")
            .expect("必须有'未含'的说明");
        assert!(note.message.contains("未含"), "{}", note.message);
        assert!(
            note.suggestion.contains("缓冲区") || note.suggestion.contains("acquisition"),
            "说明要指出缺的是哪两件事：{}",
            note.suggestion
        );
    }

    #[test]
    fn summary_counts_instead_of_just_claiming() {
        let mut segs = clean_set(1000);
        // 造两个违反：段 0 的 DTS > PTS；段 3（ODS）的 PTS 高过 PCS
        segs[0].dts = segs[0].pts + 5;
        segs[3].pts = 10_000;
        let s = summary(&segs);
        assert_eq!(s.sets, 1);
        assert_eq!(s.segments, 5);
        assert!(s.dts_gt_pts_violations >= 1, "{s:?}");
        assert!(s.pts_le_pcs_violations >= 1, "{s:?}");
    }

    /// 没有 END 收尾的尾巴也要算一集，否则那一段的时间完全不被检查。
    #[test]
    fn an_unterminated_tail_is_still_checked() {
        // 第一集完整（END 收尾），后面跟一段没有 END 的数据
        let mut segs = clean_set(1000);
        segs.push(seg(5000, 4500, SEG_PCS));
        segs.push(seg(4500, 4500, SEG_PDS));
        // 这一段 PTS 高过本集 PCS(5000)
        segs.push(seg(6000, 4500, SEG_ODS));
        let s = summary(&segs);
        assert_eq!(s.sets, 2, "未收尾的尾巴必须算一集：{s:?}");
        assert_eq!(s.pts_le_pcs_violations, 1, "{s:?}");
        assert_eq!(s.dts_drops, 0, "DTS 本身没递减：{s:?}");
    }

    /// 真实语料里的三种骨架都要能跑过（本机实测的分布）。
    /// 只测一种形状会让实现悄悄只兼容那一种。
    #[test]
    fn the_three_real_world_shapes_all_pass() {
        // PCS -> WDS -> PDS -> ODS -> END（8520 次）
        let full = clean_set(1000);
        // PCS -> WDS -> END（1393 次）
        let dts = 500;
        let wds_only = vec![
            seg(1000, dts, SEG_PCS),
            seg(900, dts, SEG_WDS),
            seg(900, 900, SEG_END),
        ];
        // PCS -> PDS -> END（38 次）
        let pds_only = vec![
            seg(1000, dts, SEG_PCS),
            seg(dts, dts, SEG_PDS),
            seg(dts, dts, SEG_END),
        ];
        for (name, segs) in [
            ("PCS-WDS-PDS-ODS-END", full),
            ("PCS-WDS-END", wds_only),
            ("PCS-PDS-END", pds_only),
        ] {
            let (r, checked) = decoder_model(&segs);
            assert!(checked);
            let bad: Vec<_> = r
                .findings
                .iter()
                .filter(|f| f.severity >= Severity::Warning)
                .map(|f| (f.check, f.message.as_str()))
                .collect();
            assert!(bad.is_empty(), "真实骨架「{name}」不该报错：{bad:?}");
        }
    }

    // -----------------------------------------------------------------------
    // AK17：`PGS.DEC.01`…`05` 这五条不变式原来**一条用例都没有**
    // （`tools/rule_coverage.py` 数出来的，不是“我觉得该测”）。
    //
    // 每条都成对：弄坏那一个条件 → 必须报；干净的骨架 → 不报。
    // 单方向的用例看不出误报，而这五条都是**必要条件**
    // （不满足一定有问题），所以“不报”那一半同样要锁。
    // -----------------------------------------------------------------------

    /// 一份干净显示集，供下面五条反例作基底。
    ///
    /// 形状照本模块已有的 `clean_set(base)`：
    /// PCS 带**本集的呈现时刻**（最高），其余段的 PTS ≤ 它；
    /// PDS 与 END 没有解码耗时（`pts == dts`）。
    /// 第一版我自己凭直觉拼了一个，结果它一上来就报 `PGS.DEC.01` ——
    /// 正好说明这五条不变式为什么值得写用例。
    fn inv_clean() -> Vec<Segment> {
        let dts = 500; // base - 500
        vec![
            seg(1000, dts, SEG_PCS),
            seg(900, dts, SEG_WDS),
            seg(dts, dts, SEG_PDS),
            seg(dts + 91, dts, SEG_ODS),
            seg(dts + 91, dts + 91, SEG_END),
        ]
    }

    fn rules_of(segs: &[Segment]) -> Vec<&'static str> {
        let (r, checked) = decoder_model(segs);
        assert!(checked, "这段应当被当成“跑过”");
        r.findings
            .iter()
            .filter(|f| f.severity >= Severity::Warning)
            .map(|f| f.check)
            .collect()
    }

    /// 跑一遍解码器模型，拿到**报告本身**。
    ///
    /// ## 为什么不包装成 `has_check(&segs, id) -> bool`
    ///
    /// 我第一版就是那么写的，断言完全正确 —— 但
    /// `tools/rule_coverage.py` 扫的是 `<报告>.has_check("<id>")`
    /// 这个**方法调用**写法，于是这五条在盘点表上一直写着“一次都没提到”，
    /// 而实际上已经有用例了。下一个人会照着过期的事实重复劳动。
    ///
    /// 返回报告本身，调用方就能用工具认得、也本就是报告自带 API 的写法。
    fn report_of(segs: &[Segment]) -> ValidationReport {
        let (r, checked) = decoder_model(segs);
        assert!(checked, "这段应当被当成“跑过”");
        r
    }

    #[test]
    fn a_clean_set_reports_none_of_the_decoder_invariants() {
        let got = rules_of(&inv_clean());
        assert!(got.is_empty(), "干净骨架不该报任何东西：{got:?}");
    }

    /// `PGS.DEC.01`：段的 PTS **晚于**该集 PCS 的 PTS。
    ///
    /// 条件（读代码得到）：`s.pts > pcs_pts`。
    /// 我第一版写反了方向（把 WDS 标到 PCS 之前）—— **实测不报**，
    /// 因为 PCS 带的本来就是本集的最高呈现时刻。改成**后来的段**才对。
    #[test]
    fn a_segment_later_than_its_own_pcs_is_reported() {
        let mut s = inv_clean();
        // WDS 标得比 PCS（1000）还晚
        s[1].pts = 1500;
        s[1].dts = 500;
        assert!(
            report_of(&s).has_check("PGS.DEC.01"),
            "晚于 PCS 的段应当报：{:?}",
            rules_of(&s)
        );
        assert!(!report_of(&inv_clean()).has_check("PGS.DEC.01"));
    }

    /// `PGS.DEC.02`：`DTS > PTS` —— 解码时间晚于显示时间，物理上不成立。
    #[test]
    fn a_segment_decoded_after_it_is_presented_is_reported() {
        let mut s = inv_clean();
        s[3].pts = 500;
        s[3].dts = 600; // 反了
        assert!(
            report_of(&s).has_check("PGS.DEC.02"),
            "DTS > PTS 应当报：{:?}",
            rules_of(&s)
        );
        assert!(!report_of(&inv_clean()).has_check("PGS.DEC.02"));
    }

    /// `PGS.DEC.03`：`END` 的 PTS **不等于**最后一个 `ODS` 的 PTS。
    ///
    /// 两者相等才说明“这一集完整解完了”。不相等意味着中途被截。
    #[test]
    fn an_end_not_matching_the_last_ods_is_reported() {
        let mut s = inv_clean();
        s[4].pts = 1200; // END 与最后一个 ODS（591）对不上
        s[4].dts = 1200;
        assert!(
            report_of(&s).has_check("PGS.DEC.03"),
            "END 与末个 ODS 不一致应当报：{:?}",
            rules_of(&s)
        );
        assert!(!report_of(&inv_clean()).has_check("PGS.DEC.03"));
    }

    /// `PGS.DEC.04`：`PDS` / `END` 的 `DTS != PTS`。
    ///
    /// 这两类段没有解码耗时，真实流 100% 满足等号。
    /// 这条正是那个曾抳出我们**自己编码器两个真缺陷**的不变式
    /// （见 `docs/reviews/2026-10-17-decoder-model-layer5.md`）。
    #[test]
    fn a_pds_or_end_with_a_decode_delay_is_reported() {
        let mut s = inv_clean();
        s[2].dts = 400; // PDS 有“解码耗时”
        assert!(
            report_of(&s).has_check("PGS.DEC.04"),
            "PDS 有解码耗时应当报：{:?}",
            rules_of(&s)
        );

        let mut t = inv_clean();
        t[4].dts = 500; // END 有“解码耗时”
        assert!(
            report_of(&t).has_check("PGS.DEC.04"),
            "END 有解码耗时应当报：{:?}",
            rules_of(&t)
        );

        assert!(!report_of(&inv_clean()).has_check("PGS.DEC.04"));
    }

    /// `PGS.DEC.05`：**流的 DTS 倒退了**（后被解出的段反而更早）。
    ///
    /// ## 实测修正过假设
    ///
    /// 我第一版以为它管“段类型顺序”（把 ODS 摆到 PCS 前面）—— **实测不报**。
    /// 读代码才看清：它扫的是 `segments[i].dts < segments[i-1].dts`，
    /// 即**整个流上 DTS 必须单调不减**。
    /// 段类型顺序是另一回事（结构层管）。
    ///
    /// 名字里的“顺序”指的是**时间上的顺序**，不是类型顺序 ——
    /// 这一点写在这里，免得下一个人又按字面去造反例。
    #[test]
    fn a_stream_with_a_backwards_dts_is_reported() {
        // 第二集的 DTS 比第一集还早
        let mut a = inv_clean();
        let mut b = inv_clean();
        // 把整集往后推，但让第二集的 DTS 反而更小
        for s in &mut a {
            s.pts += 5000;
            s.dts += 5000;
        }
        for s in &mut b {
            s.pts += 4000;
            s.dts = 500; // 比第一集的末个 DTS（591）还小
        }
        let bad: Vec<Segment> = a.into_iter().chain(b).collect();
        assert!(
            report_of(&bad).has_check("PGS.DEC.05"),
            "DTS 倒退应当报：{:?}",
            rules_of(&bad)
        );
        assert!(!report_of(&inv_clean()).has_check("PGS.DEC.05"));
    }
}
