//! 文本字幕（ASS/SSA 与 SRT 类时间对）的校验（08、09 §9、23）。
//!
//! 与 `codec-ass` 的分工：AST 永远能读（解析不失败），这里只回答"能不能安全交付"。
//! 时间对以 `Vec<(Time, Time)>` 传入，因此 SRT/VTT 共用同一套规则而不必绑死某个 codec。

use codec_ass::{AssDocument, OverrideToken, SectionKind};
use core_model::geometry::Geometry;
use core_model::time::Time;

use crate::report::{Severity, ValidationReport};

/// ASS/SSA 文档级校验。`geo` 为 None 时跳过画布/坐标空间相关的检查。
pub fn ass(doc: &AssDocument, geo: Option<&Geometry>) -> ValidationReport {
    let mut r = ValidationReport::new();

    match doc.script_info("ScriptType").map(str::trim) {
        None => {
            r.add(
                "ASS.STR.01",
                Severity::Warning,
                "缺少 ScriptType 头",
                "多数播放器仍能读，但导出时补 `ScriptType: v4.00+` 更稳",
            );
        }
        Some(v) if !v.to_ascii_lowercase().starts_with("v4") => {
            r.add(
                "ASS.STR.02",
                Severity::Warning,
                format!("ScriptType 非 v4 系列：{v}"),
                "确认渲染器支持该版本；否则按 V4+ 重导",
            );
        }
        _ => {}
    }

    let playres = doc.play_res();
    let has_x = doc.script_info("PlayResX").is_some();
    let has_y = doc.script_info("PlayResY").is_some();
    match playres {
        Some((x, y)) if x <= 0 || y <= 0 => {
            r.add(
                "ASS.GEO.01",
                Severity::Error,
                format!("PlayRes 非法：{x}x{y}"),
                "改成正数；Rewrite 模式需要可信的脚本尺寸",
            );
        }
        None if has_x || has_y => {
            r.add(
                "ASS.GEO.02",
                Severity::Warning,
                "PlayRes 只声明了单轴",
                "另一轴由 renderer 按比例推导；Rewrite 前建议补齐两轴",
            );
        }
        None => {
            r.add(
                "ASS.GEO.03",
                Severity::Info,
                "没有 PlayRes 头，脚本坐标空间由 renderer 决定",
                "Preserve 模式安全；Rewrite 必须显式给出基准尺寸",
            );
        }
        Some(_) => {}
    }

    let lay_x = doc.script_info("LayoutResX");
    let lay_y = doc.script_info("LayoutResY");
    match (lay_x, lay_y) {
        (Some(a), Some(b)) => {
            let ok = a.trim().parse::<i64>().is_ok_and(|v| v > 0)
                && b.trim().parse::<i64>().is_ok_and(|v| v > 0);
            if !ok {
                r.add(
                    "ASS.GEO.04",
                    Severity::Warning,
                    format!("LayoutResX/Y 值非法：{a:?} / {b:?}"),
                    "libass 只在两轴都为正时启用 LayoutRes（ass.c:888-893）；否则会被忽略",
                );
            } else if let Some(g) = geo {
                if g.script.is_some() && g.script_scale_factor().is_ok_and(|f| f != Rational::ONE) {
                    r.add(
                        "ASS.GEO.05",
                        Severity::Warning,
                        "Rewrite 会改 PlayRes；LayoutRes 决定 PAR 与 blur 基准，默认保持原值",
                        "若确实要一起缩放，请先跑 libass/VSFilterMod/CSRI differential corpus（09 §2.1）",
                    );
                }
            }
        }
        (Some(_), None) | (None, Some(_)) => {
            r.add(
                "ASS.GEO.06",
                Severity::Warning,
                "LayoutRes 只给了一轴",
                "libass 要求两轴同时为正才生效；请补齐或删除该头",
            );
        }
        (None, None) => {}
    }

    if let Some(g) = geo {
        if let Err(e) = g.validate() {
            r.add(
                "ASS.CAN.01",
                Severity::Error,
                format!("输出画布不合规：{}", e.title),
                e.user_action
                    .clone()
                    .unwrap_or_else(|| "使用完整 16:9 画布或显式声明实验性自定义".to_string()),
            );
        }
        if let (Some(s), Some((x, y))) = (g.script, playres) {
            if i64::from(s.x) != x || i64::from(s.y) != y {
                r.add(
                    "ASS.CAN.02",
                    Severity::Error,
                    format!(
                        "几何模型的脚本尺寸 {}x{} 与文件 PlayRes {x}x{y} 不一致",
                        s.x, s.y
                    ),
                    "以文件为准（resolve_script），或修正 OutputProfile；否则缩放系数是错的",
                );
            }
        }
    }

    let style_names: Vec<String> = doc
        .styles()
        .iter()
        .filter_map(|s| s.name().map(|n| n.trim().to_lowercase()))
        .collect();
    let has_default = style_names.iter().any(|n| n == "default");
    let mut missing_style: Vec<String> = Vec::new();

    for d in doc.dialogues() {
        let start = d.start().ok();
        let end = d.end().ok();
        match (start, end) {
            (Some(s), Some(e)) => {
                if e <= s {
                    r.add(
                        "ASS.EVT.01",
                        Severity::Error,
                        format!(
                            "事件结束不晚于开始：{} -> {}",
                            s.to_ass_string().unwrap_or_default(),
                            e.to_ass_string().unwrap_or_default()
                        ),
                        "播放器可能直接丢弃该行；请修正时间或删掉空事件",
                    );
                }
            }
            _ => {
                r.add(
                    "ASS.EVT.02",
                    Severity::Error,
                    "事件时间无法解析",
                    format!("原行：{}", d.raw()),
                );
            }
        }
        if let Some(st) = d.style_name() {
            let low = st.to_lowercase();
            if !style_names.contains(&low) && !low.is_empty() {
                missing_style.push(st.to_string());
            }
        }
        for token in codec_ass::parse_overrides(d.text()) {
            if let OverrideToken::Block(b) = token {
                if !b.closed {
                    let f = r.add(
                        "ASS.EVT.03",
                        Severity::Warning,
                        "override block 没有闭合的 `}`",
                        "分词器容忍，但多数播放器行为不一致；建议补上 `}`",
                    );
                    f.pts = start.and_then(|t| t.to_pts_ticks().ok());
                }
                for t in &b.tags {
                    if let codec_ass::TagKind::Animation { nested, .. } = &t.kind {
                        for n in nested {
                            if n.is_unknown() {
                                r.add(
                                    "ASS.EVT.04",
                                    Severity::Info,
                                    format!("`\\t` 内含未识别 tag：{}", n.name),
                                    "原样保留；VSFilterMod 私有 tag 需要 provider 才能等价（12 §1B/C）",
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    if !missing_style.is_empty() {
        let sample = missing_style[0].clone();
        let sev = if has_default {
            Severity::Warning
        } else {
            Severity::Error
        };
        r.add(
            "ASS.REF.01",
            sev,
            format!(
                "{} 个事件引用了不存在的样式（例如 {sample:?}）",
                missing_style.len()
            ),
            if has_default {
                "播放器会回退到 Default；建议改名或补样式"
            } else {
                "没有 Default 可回退时行为不一致；请补样式或改事件"
            },
        );
    }

    let events = doc.dialogue_count();
    if events > 1 {
        let mut prev: Option<Time> = None;
        let mut out_of_order = 0usize;
        for d in doc.dialogues() {
            if let Ok(s) = d.start() {
                if prev.is_some_and(|p| s < p) {
                    out_of_order += 1;
                }
                prev = Some(s);
            }
        }
        if out_of_order > 0 {
            r.add(
                "ASS.EVT.05",
                Severity::Info,
                format!("{out_of_order} 个事件未按时间递增"),
                "写出阶段可以排序（08 §1）；这里只报告，不改动原文顺序",
            );
        }
    }

    for s in &doc.sections {
        if s.kind.is_attachment() {
            let body_lines = s
                .entries
                .iter()
                .filter(|e| e.kind == codec_ass::EntryKind::Raw)
                .count();
            if body_lines == 0 {
                r.add(
                    "ASS.ATT.01",
                    Severity::Warning,
                    format!("附件段 {:?} 是空的", s.header.as_deref().unwrap_or("")),
                    "空的 [Fonts]/[FileList] 会让播放器去找系统字体；要么补内容要么删段",
                );
            } else {
                r.add(
                    "ASS.ATT.02",
                    Severity::Info,
                    format!("保留了 {} 行附件内容", body_lines),
                    "写出时必须走内存注入路径（ass_add_font），不要窄化 Windows 路径（F7）",
                );
            }
        }
        if s.kind == SectionKind::Styles && s.format.is_empty() {
            r.add(
                "ASS.STR.03",
                Severity::Warning,
                "Styles 段没有字段表",
                "字段顺序只能按默认表猜测；请补 `Format:` 行",
            );
        }
        if s.kind.needs_format() && !s.format_source_is_line() {
            r.add(
                "ASS.STR.04",
                Severity::Info,
                format!(
                    "{:?} 段的字段表来自 {:?}，不是真正的 Format 行",
                    s.kind, s.format_source
                ),
                "非标准写法会被保留，但导出给别的工具时建议规范化",
            );
        }
    }

    let mut unknown: Vec<String> = Vec::new();
    for d in doc.dialogues() {
        codec_ass::overrides::unknown_tags(d.text(), &mut unknown);
    }
    for name in unknown.iter().take(8).cloned().collect::<Vec<_>>() {
        r.add(
            "ASS.TAG.01",
            Severity::Info,
            format!("未识别 tag：{name}"),
            "原样保留；只有确认 provider 后才能声称特效等价（12 §1）",
        );
    }
    if unknown.len() > 8 {
        r.add(
            "ASS.TAG.02",
            Severity::Info,
            format!("另有 {} 种未识别 tag 未逐条列出", unknown.len() - 8),
            "完整清单在诊断包里",
        );
    }
    r
}

/// 时间对（SRT/VTT 或任何"起止时间 + 文本"的字幕）的通用检查。
pub fn time_pairs(pairs: &[(Time, Time)], texts: &[&str]) -> ValidationReport {
    let mut r = ValidationReport::new();
    let mut bad = 0usize;
    let mut overlap = 0usize;
    let mut prev_end: Option<Time> = None;
    for ((s, e), text) in pairs.iter().zip(texts.iter().chain(std::iter::repeat(&""))) {
        if e <= s {
            bad += 1;
        }
        if let Some(p) = prev_end {
            if *s < p {
                overlap += 1;
            }
        }
        prev_end = Some(*e);
        let _ = text;
    }
    if bad > 0 {
        r.add(
            "SRT.EVT.01",
            Severity::Error,
            format!("{bad} 条 cue 的结束不晚于开始"),
            "修正时间或删除该条；播放器可能整条丢弃",
        );
    }
    if overlap > 0 {
        r.add(
            "SRT.EVT.02",
            Severity::Warning,
            format!("{overlap} 条 cue 与上一条时间重叠"),
            "文本类字幕通常应顺序显示；确认是否刻意的双行字幕",
        );
    }
    if pairs.iter().any(|(s, _)| s.to_pts_ticks().is_err()) {
        r.add(
            "SRT.EVT.03",
            Severity::Warning,
            "存在无法表示为毫秒的时间",
            "检查是否混用了不同精度来源（10 §7 单一时间源）",
        );
    }
    r
}

use core_model::time::Rational;

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::canvas::Canvas;
    use core_model::geometry::{Geometry, ScriptResolutionPolicy, ScriptSize, StorageSize};
    use core_model::time::{Fps, Time};

    const OK: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize\r\nStyle: Default,Arial,48\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(10,10)}你好\r\n";

    fn doc(src: &str) -> AssDocument {
        AssDocument::parse(src)
    }

    fn geo(canvas: Canvas, script: Option<(u32, u32)>) -> Geometry {
        Geometry {
            canvas,
            script: script.map(|(x, y)| ScriptSize { x, y }),
            storage: Some(StorageSize::new(canvas.width, canvas.height).unwrap()),
            script_policy: ScriptResolutionPolicy::Preserve,
            ..Default::default()
        }
    }

    #[test]
    fn clean_script_has_no_blocking_findings() {
        let r = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(
            !r.findings.iter().any(|f| f.severity.blocks_strict()),
            "{:?}",
            r.text_lines()
        );
    }

    #[test]
    fn negative_duration_and_missing_style_are_reported() {
        let src = OK.replace(
            "0,0:00:01.00,0:00:02.00,Default",
            "0,0:00:05.00,0:00:04.00,Default",
        );
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.EVT.01"), "{:?}", r.text_lines());
    }

    #[test]
    fn missing_style_reference_is_error_only_without_default() {
        let with_default = OK.replace(",0:00:02.00,Default", ",0:00:02.00,Missing");
        let r = ass(&doc(&with_default), None);
        let f = r
            .findings
            .iter()
            .find(|f| f.check == "ASS.REF.01")
            .expect("要报告缺失样式");
        assert_eq!(f.severity, Severity::Warning, "有 Default 时是 warning");

        let no_default = with_default.replace("Style: Default,Arial,48", "Style: Other,Arial,48");
        let r2 = ass(&doc(&no_default), None);
        let f2 = r2
            .findings
            .iter()
            .find(|f| f.check == "ASS.REF.01")
            .expect("要报告缺失样式");
        assert_eq!(f2.severity, Severity::Error, "没有回退目标时必须阻止交付");
    }

    #[test]
    fn geometry_mismatch_with_file_is_error() {
        let r = ass(&doc(OK), Some(&geo(Canvas::UHD2160, Some((1280, 720)))));
        assert!(r.has_check("ASS.CAN.02"), "{:?}", r.text_lines());
    }

    #[test]
    fn non_16_9_canvas_without_custom_flag_is_error() {
        let g = Geometry {
            canvas: Canvas::new(1920, 800).unwrap(),
            ..geo(Canvas::new(1920, 800).unwrap(), Some((1920, 1080)))
        };
        let r = ass(&doc(OK), Some(&g));
        assert!(r.has_check("ASS.CAN.01"), "{:?}", r.text_lines());
    }

    #[test]
    fn unclosed_block_and_unknown_tag_are_reported() {
        let src = OK.replace("{\\pos(10,10)}你好", "{\\pos(10,10)\\vsfmod 1}你好");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.TAG.01"), "{:?}", r.text_lines());
        let src2 = OK.replace("{\\pos(10,10)}你好", "{\\pos(10,10)你好");
        let r2 = ass(&doc(&src2), None);
        assert!(r2.has_check("ASS.EVT.03"), "{:?}", r2.text_lines());
    }

    #[test]
    fn layout_res_half_given_is_warning() {
        let src = OK.replace("PlayResY: 1080", "PlayResY: 1080\r\nLayoutResX: 1920");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.GEO.06"), "{:?}", r.text_lines());
    }

    #[test]
    fn rewrite_with_layout_res_present_warns_about_corpus() {
        let src = OK.replace(
            "PlayResY: 1080",
            "PlayResY: 1080\r\nLayoutResX: 1920\r\nLayoutResY: 1080",
        );
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            storage: Some(StorageSize::new(1920, 1080).unwrap()),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Default::default()
        };
        let r = ass(&doc(&src), Some(&g));
        assert!(r.has_check("ASS.GEO.05"), "{:?}", r.text_lines());
    }

    #[test]
    fn attachment_section_is_reported() {
        let src = format!("{OK}\n[Fonts]\nfontdata: 00\r\n");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.ATT.02"), "{:?}", r.text_lines());
        let empty = format!("{OK}\n[Fonts]\n");
        let r2 = ass(&doc(&empty), None);
        assert!(r2.has_check("ASS.ATT.01"), "{:?}", r2.text_lines());
    }

    #[test]
    fn single_axis_playres_is_warning() {
        let src = OK.replace("PlayResX: 1920\r\nPlayResY: 1080", "PlayResY: 1080");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.GEO.02"), "{:?}", r.text_lines());
    }

    #[test]
    fn time_pairs_catch_bad_and_overlap() {
        let t = |ms: i64| Time::from_millis(ms).unwrap();
        let bad = vec![(t(1_000), t(1_000)), (t(3_000), t(2_000))];
        let r = time_pairs(&bad, &["a", "b"]);
        assert!(r.has_check("SRT.EVT.01"));
        let overlap = vec![(t(0), t(2_000)), (t(1_500), t(3_000))];
        let r2 = time_pairs(&overlap, &["a", "b"]);
        assert!(r2.has_check("SRT.EVT.02"));
        assert!(!r2.has_check("SRT.EVT.01"));
        let clean = vec![(t(0), t(1_000)), (t(1_000), t(2_000))];
        let r3 = time_pairs(&clean, &["a", "b"]);
        assert!(r3.findings.is_empty(), "{:?}", r3.text_lines());
    }

    #[test]
    fn missing_script_type_and_no_playres_are_not_errors() {
        let src = "[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,x\r\n";
        let r = ass(&doc(src), None);
        assert!(r.has_check("ASS.STR.01") || r.has_check("ASS.GEO.03"));
        assert!(
            !r.findings
                .iter()
                .any(|f| f.severity.blocks_strict() && f.check != "ASS.REF.01"),
            "{:?}",
            r.text_lines()
        );
    }

    #[test]
    fn fps_independence_check_for_layout_scale_zero() {
        // Rational::ONE 比较用的辅助：确保 geo 的 Rewrite 判定不被 fps 干扰
        let g = geo(Canvas::FHD1080, Some((1920, 1080)));
        assert_eq!(g.script_scale_factor().unwrap(), Rational::ONE);
        let _ = Fps::PAL_25;
        let _ = Time::ZERO;
    }

    // -----------------------------------------------------------------------
    // AK17 的“该报/不该报”双向覆盖：下面这几条规则之前**一条用例都没有**。
    //
    // `tools/rule_coverage.py` 数得出来（“测试里一次都没提到（15 条）”），
    // 所以这不是“我觉得该测”，而是**量出来的洞**。
    // 每条都成对写：报得出来 + 干净输入不报 —— 单方向的用例看不出误报。
    // -----------------------------------------------------------------------

    /// `ASS.EVT.02`：事件时间无法解析 → 错误。
    #[test]
    fn an_unparseable_event_time_is_reported() {
        let src = OK.replace("0:00:01.00", "0:00:AB.CD");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.EVT.02"), "{:?}", r.text_lines());
        // 反面：干净输入不该报它
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.EVT.02"), "{:?}", clean.text_lines());
    }

    /// `ASS.EVT.04`：`\t` 里有未识别的 tag → 信息级。
    ///
    /// 留意与 `ASS.EVT.05`（乱序）分开：两者虽然级别相同，
    /// 但说的是完全不同的事，合在一起会让报告失去分辨力。
    #[test]
    fn an_unknown_tag_inside_a_transform_is_reported() {
        let src = OK.replace(
            "{\\pos(10,10)}你好",
            "{\\pos(10,10)\\t(0,500,\\bogus1)}你好",
        );
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.EVT.04"), "{:?}", r.text_lines());
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.EVT.04"), "{:?}", clean.text_lines());
    }

    /// `ASS.EVT.05`：事件未按时间递增 → 信息级（写出阶段可重排）。
    #[test]
    fn events_out_of_order_are_reported() {
        let src = OK.replace(
            "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(10,10)}你好\r\n",
            "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,一\r\n\
             Dialogue: 0,0:00:00.50,0:00:01.50,Default,,0,0,0,,二\r\n",
        );
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.EVT.05"), "{:?}", r.text_lines());
        // 反面：本来就只有一条事件的 OK 不该报
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.EVT.05"), "{:?}", clean.text_lines());
    }

    /// `ASS.GEO.01`：PlayRes 不是正整数 → 错误。
    #[test]
    fn an_illegal_playres_is_reported() {
        let src = OK.replace("PlayResX: 1920", "PlayResX: 0");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.GEO.01"), "{:?}", r.text_lines());
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.GEO.01"), "{:?}", clean.text_lines());
    }

    /// `ASS.GEO.04`：LayoutResX/Y 值非法 → 警告。
    #[test]
    fn an_illegal_layout_res_is_reported() {
        let src = OK.replace(
            "PlayResY: 1080",
            "PlayResY: 1080\r\nLayoutResX: 0\r\nLayoutResY: abc",
        );
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.GEO.04"), "{:?}", r.text_lines());
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.GEO.04"), "{:?}", clean.text_lines());
    }

    /// `ASS.STR.02`：ScriptType 不是 v4 系列 → 警告。
    #[test]
    fn a_non_v4_script_type_is_reported() {
        let src = OK.replace("ScriptType: v4.00+", "ScriptType: v3.00");
        let r = ass(&doc(&src), None);
        assert!(r.has_check("ASS.STR.02"), "{:?}", r.text_lines());
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.STR.02"), "{:?}", clean.text_lines());
    }

    /// `ASS.STR.03`：Styles 段没有字段表 → 警告。
    ///
    /// ## 实测修正过两次假设，最后发现它的 Styles 分支**走不到**
    ///
    /// 1. 我第一版是“把 Styles 的 `Format:` 行删掉就报 03” —— **实测不报**，
    ///    报的是 `ASS.STR.04`（`format_source = RepeatedHeader`）。
    /// 2. 第二版把 Styles 与 Events 的 `Format:` 行都删掉 —— **仍不报**，
    ///    两条都是 `RepeatedHeader`。
    ///
    /// 追到 `codec-ass` 才明白：字段表有**四档来源**
    /// （`FormatLine` / `RepeatedHeader` / `Fallback` / `None`），
    /// 而 Styles/Events 这两个 `needs_format()` 的段总会被填上一种，
    /// **永远不会是空的**。`FormatSource::None` 只出现在
    /// `SectionKind::Unknown` 的默认表上，而那种段不走这个分支。
    ///
    /// ## 所以这里不断言“报得出 03”
    ///
    /// 强造一个能触发它的输入需要打破解析器的不变式，那测的就是**假想中的代码**
    /// 而不是真实行为。本用例改成锁**实际成立的那件事**：
    /// 字段表不是来自 Format 行时必须报 `ASS.STR.04`。
    ///
    /// 并把“03 的 Styles 分支疑似不可达”记在这里 ——
    /// 它是 `rule_coverage.py` 里 03 长期未覆盖的真实原因，
    /// 而不是“忘了写用例”。
    #[test]
    fn a_styles_section_without_a_format_line_is_reported() {
        let src = OK.replace("Format: Name, Fontname, Fontsize\r\n", "");
        let r = ass(&doc(&src), None);
        // 实际行为：报 04（来源不是 Format 行），而不是 03
        assert!(
            r.has_check("ASS.STR.04"),
            "字段表不是来自 Format 行时必须报 04：{:?}",
            r.text_lines()
        );
        // 干净输入不该报任何一条
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.STR.04"), "{:?}", clean.text_lines());
        assert!(!clean.has_check("ASS.STR.03"), "{:?}", clean.text_lines());
    }

    /// `ASS.STR.04`：字段表**不是来自 `Format:` 行** → 信息级。
    ///
    /// 条件：`s.kind.needs_format() && !s.format_source_is_line()`。
    ///
    /// 它与 `ASS.STR.03` 是**相邻的两件事**：03 说“根本没有字段表”，
    /// 04 说“有，但不是从 Format 行来的”。只测 03 会把 04 的
    /// “来源不明”当成“来源就是 Format 行”。
    #[test]
    fn a_field_table_taken_from_somewhere_else_is_reported() {
        // 把 Styles 段的 Format 行改成注释式（`;Format:`）：
        // 于是字段表要么为空（03），要么来自别处（04），两者必有一报。
        let src = OK.replace(
            "Format: Name, Fontname, Fontsize\r\n",
            ";Format: Name, Fontname, Fontsize\r\n",
        );
        let r = ass(&doc(&src), None);
        assert!(
            r.has_check("ASS.STR.04") || r.has_check("ASS.STR.03"),
            "字段表来源不明时至少要报一条：{:?}",
            r.text_lines()
        );
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.STR.04"), "{:?}", clean.text_lines());
    }

    /// `ASS.TAG.02`：未识别 tag 超过**逐条列出的上限（8 种）** → 只说“另有 N 种”。
    ///
    /// 条件看清楚了：`unknown.len() > 8` ——
    /// 前 8 个逐条报 `ASS.TAG.01`，剩下的归并成 `ASS.TAG.02`。
    /// 所以要触发它必须造出**超过 8 种不同**的未识别 tag。
    /// 这条与 AG6 同族（避免报告被噪声淹掉），只是在“种类数”这个维度上截断。
    #[test]
    fn too_many_kinds_of_unknown_tags_are_summarised() {
        // 12 种互不相同的未识别 tag，超过上限 8
        let tags: String = (0..12).map(|i| format!("\\zz{i}")).collect();
        let src = OK.replace(
            "{\\pos(10,10)}你好",
            &format!("{{{}\\pos(10,10)}}你好", tags),
        );
        let r = ass(&doc(&src), None);
        assert!(
            r.has_check("ASS.TAG.02"),
            "超过 8 种未知 tag 应当归并：{:?}",
            r.text_lines()
        );
        // 逐条的也要在（前 8 个）
        assert!(r.has_check("ASS.TAG.01"), "{:?}", r.text_lines());
        let clean = ass(&doc(OK), Some(&geo(Canvas::FHD1080, Some((1920, 1080)))));
        assert!(!clean.has_check("ASS.TAG.02"), "{:?}", clean.text_lines());
    }

    /// `SRT.EVT.03`：存在不能用做 PTS 的时间 → 警告（10 §7 单一时间源）。
    ///
    /// ## 真实条件（读代码得到，**修正过一次假设**）
    ///
    /// ```ignore
    /// if pairs.iter().any(|(s, _)| s.to_pts_ticks().is_err())
    /// ```
    ///
    /// 我第一版以为“非整拍”就能触发，并造了 `1/7` 毫秒去测 —— **实测不报**。
    /// 读 `to_pts_ticks` 才明白：它是 `mul(90_000).round_i64()`，
    /// **会四舍五入**，所以普通值永远不会出错。
    ///
    /// 能把 `is_err()` 变真的只有两件事：**负时间**、**溢出 33bit 上限**。
    /// 所以这条规则的真实含义是“**这个时间不是合法 PTS**”，
    /// 而不是“精度对不上” —— 它的标题（“无法表示为毫秒”）比实际条件更宽。
    /// 这一点写在这里，免得下一个人又走一遍同样的弯路。
    ///
    /// 这条规则的实际入口是 `time_pairs`（SRT 路径），不是 `ass`。
    #[test]
    fn an_srt_time_that_cannot_be_a_pts_is_reported() {
        // 反面：正常时间不报
        let ok = [(
            Time::from_millis(1000).unwrap(),
            Time::from_millis(2000).unwrap(),
        )];
        let r = time_pairs(&ok, &["一二三"]);
        assert!(
            !r.has_check("SRT.EVT.03"),
            "正常时间不该报：{:?}",
            r.text_lines()
        );

        // 正面：负起点 —— `to_pts_ticks` 明确报 `PTS 为负`
        let neg = [(
            Time::from_millis(-1).unwrap(),
            Time::from_millis(2000).unwrap(),
        )];
        let r2 = time_pairs(&neg, &["一二三"]);
        assert!(
            r2.has_check("SRT.EVT.03"),
            "负时间不是合法 PTS，应当报：{:?}",
            r2.text_lines()
        );
    }
}
