//! ASS 脚本分辨率策略（09_CANVAS_RESOLUTION_MULTI_OUTPUT.md §2/§4/§9）。
//!
//! 两条路径必须泾渭分明：
//!
//! - **Preserve（默认）**：脚本坐标空间不动，升采样发生在 renderer。09 §9 的硬规则是
//!   "Preserve 在 1080→2160 时不得修改源 ASS 文本"，所以本 crate 在 Preserve 下
//!   一定一个字节都不写。
//! - **Rewrite**：把脚本真正改写成目标画布的坐标空间（PlayRes × 系数）。哪些量该乘系数
//!   不由直觉决定，而由 libass 的两套缩放比决定（见 `tags` 模块文档与源码行号）：
//!   `screen_scale = frame/PlayRes` 参与的量要缩放；`blur_scale = frame/layout_res`
//!   参与的量（模糊、非 SBS 的边框阴影、Effect 像素参数）不缩放；百分比/角度/毫秒/`\\p`
//!   精度因子不是尺寸，永远不缩放。
//!
//! LayoutRes 是 libass 真实存在的头（`libass/ass.c:888-893`，
//! `ASS_Track.LayoutResX` 注释："overrides values from ass_set_storage_size and also
//! takes precedence over ass_set_pixel_aspect"）。它不参与脚本→画面的几何缩放，
//! 而是决定 PAR 与 blur 基准，所以默认 `KeepAsIs`（09 §4 B 的"不得无脑 ×2"）。

pub mod numbers;
pub mod tags;

use core_model::error::{Domain, Result, StkError};
use core_model::geometry::{Geometry, LayoutResPolicy, ScriptResolutionPolicy, ScriptSize};
use core_model::loss::LossReport;
use core_model::time::Rational;

use codec_ass::{AssDocument, Entry, EntryKind, Eol, SectionKind};

pub use numbers::{
    scale_literals, scale_to_integer, LiteralMode, Rewrite, RewriteStat, Rewritten, MAX_DECIMALS,
};
pub use tags::{action_for, TagAction, TagContext, TagStats};

/// 改动映射规则时升版本（同一输入必须能追溯到同一算法）。
pub const ALGORITHM_VERSION: u32 = 1;

/// Rewrite 时属于脚本坐标空间的样式字段（含 SSA 老命名）。
/// `Outline`/`Shadow` 只在 ScaledBorderAndShadow: yes 时属于该空间，见 `rewrite_styles`。
pub const SCALED_STYLE_FIELDS: [&str; 10] = [
    "Fontsize",
    "Spacing",
    "MarginL",
    "MarginR",
    "MarginV",
    "LeftMargin",
    "RightMargin",
    "VerticalMargin",
    "Outline",
    "Shadow",
];

/// 逐行覆盖样式的边距字段。
pub const SCALED_EVENT_FIELDS: [&str; 3] = ["MarginL", "MarginR", "MarginV"];

const PLAYRES_KEYS: [&str; 2] = ["PlayResX", "PlayResY"];
const LAYOUT_KEYS: [&str; 2] = ["LayoutResX", "LayoutResY"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ResApplied {
    pub playres_changed: usize,
    pub playres_inserted: usize,
    pub layout_scaled: usize,
    pub layout_dropped: usize,
    pub style_fields: usize,
    pub event_fields: usize,
    pub tag_numbers: usize,
    pub approximated: usize,
    pub unparsable: usize,
    /// `\\blur/\\be` 等按 layout/storage 相对缩放、Rewrite 时保持原值的 tag 数
    pub blur_kept: usize,
    /// 非 SBS 文档里保持原值的边框/阴影数
    pub legacy_border_kept: usize,
    pub unknown_tags: usize,
    pub comments_skipped: usize,
    pub effects_kept: usize,
}

impl ResApplied {
    /// 实际写入的改动总数；为 0 意味着输出与输入逐字节相同。
    pub fn total(&self) -> usize {
        self.playres_changed
            + self.playres_inserted
            + self.layout_scaled
            + self.layout_dropped
            + self.style_fields
            + self.event_fields
            + self.tag_numbers
    }
}

/// 文档声明的 ScaledBorderAndShadow；`None` 表示没有该头（由 renderer 假设）。
pub fn effective_sbs(doc: &AssDocument) -> Option<bool> {
    let v = doc
        .script_info("ScaledBorderAndShadow")?
        .trim()
        .to_lowercase();
    match v.as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// 用文件自身的 PlayRes 补全 `Geometry.script`（已有值时原样返回）。
/// 任务引擎在解析脚本后调用，避免"几何模型说 640×360、文件写 1280×720"这类歧义。
pub fn resolve_script(geo: &Geometry, doc: &AssDocument) -> Geometry {
    if geo.script.is_some() {
        return *geo;
    }
    match doc.play_res() {
        Some((x, y)) => match (u32::try_from(x), u32::try_from(y)) {
            (Ok(x), Ok(y)) => Geometry {
                script: Some(ScriptSize { x, y }),
                ..*geo
            },
            _ => *geo,
        },
        None => *geo,
    }
}

/// 就地重写文档。返回改动统计；统计为 0 即输出必然与输入逐字节相同。
pub fn apply(doc: &mut AssDocument, geo: &Geometry, report: &mut LossReport) -> Result<ResApplied> {
    if geo.script_policy == ScriptResolutionPolicy::Preserve {
        // 09 §9：Preserve 不得修改源 ASS 文本，这一条优先于任何 LayoutRes 策略
        if geo.layout_policy != LayoutResPolicy::KeepAsIs {
            report.user_attention("Preserve 模式不改脚本文本，LayoutRes 策略已忽略");
        }
        report.preserved("Preserve Script Resolution：脚本文本零改动，升采样由 renderer 完成");
        return Ok(ResApplied::default());
    }
    geo.validate()?;
    let factor = geo.script_scale_factor()?;
    let layout_factor = geo.layout_scale_factor()?;
    check_playres_agreement(doc, geo)?;

    if factor == Rational::ONE
        && layout_factor == Rational::ONE
        && geo.layout_policy != LayoutResPolicy::Drop
    {
        report.preserved("Rewrite 缩放系数为 1（脚本尺寸已等于输出画布），文本零改动");
        return Ok(ResApplied::default());
    }

    let sbs = effective_sbs(doc);
    let ctx = TagContext {
        scaled_border_and_shadow: sbs.unwrap_or(true),
    };
    let mut a = ResApplied::default();

    rewrite_playres(doc, geo, factor, &mut a, report)?;
    rewrite_layoutres(doc, geo, layout_factor, &mut a, report)?;
    rewrite_styles(doc, factor, &ctx, &mut a)?;
    rewrite_events(doc, factor, &ctx, &mut a, report)?;

    match sbs {
        None => report.user_attention(
            "缺少 ScaledBorderAndShadow 头：边框/阴影按现代默认（yes）随脚本缩放",
        ),
        Some(false) => report.user_attention(
            "ScaledBorderAndShadow: no：样式 Outline/Shadow 与 \\bord/\\shad 在 libass 里走 blur_scale（libass/ass_render.c:1053-1059），Rewrite 保持原值",
        ),
        Some(true) => {}
    }
    if a.approximated > 0 {
        report.approximated("整数坐标缩放后按 half-up 取整（PlayRes/样式字段/边距）");
    }
    if a.unparsable > 0 {
        report.user_attention(&format!(
            "有 {} 个数值字段无法解析为有理数，已原样保留",
            a.unparsable
        ));
    }
    if a.blur_kept > 0 {
        report.user_attention(
            "\\blur/\\be/\\xbl/\\ybl 未按分辨率缩放：libass 的 blur_scale 以 layout_res（LayoutRes→storage）为基准而不含 PlayRes（libass/ass_render.c:1050-1052）；与 VSFilter/CSRI 是否等价仍需 differential corpus",
        );
    }
    if a.effects_kept > 0 {
        report.user_attention(
            "Effect（Banner / Scroll up|down）的参数含 storage 相对像素与依赖 layout_res/PlayRes 的延时换算（libass/ass_parse.c:939-1005），Rewrite 保持原值并需 corpus 复核",
        );
    }
    if a.comments_skipped > 0 {
        report.preserved("Comment 行按原文保留，不参与分辨率重写");
    }
    if a.total() > 0 {
        report.user_attention(
            "Rewrite 输出必须经 libass + CSRI/VSFilterMod differential corpus 才能声明视觉等效（09 §2.1、§9）",
        );
    }
    Ok(a)
}

/// 09 §4 B：缩放基准必须与文件真实 PlayRes 一致，否则会产生"按错误系数缩放"的静默错误。
fn check_playres_agreement(doc: &AssDocument, geo: &Geometry) -> Result<()> {
    let (Some(s), Some((x, y))) = (geo.script, doc.play_res()) else {
        return Ok(());
    };
    if i64::from(s.x) == x && i64::from(s.y) == y {
        return Ok(());
    }
    Err(StkError::simple(
        Domain::Transform,
        39,
        "Geometry.script 与文件 PlayRes 不一致，拒绝重写",
    )
    .with_detail(format!("geometry {}x{} / file {}x{}", s.x, s.y, x, y))
    .with_action("用 resolve_script() 以文件 PlayRes 为准，或修正 OutputProfile 的脚本尺寸"))
}

/// 按 key 收集 Script Info 条目下标（大小写不敏感，最后一次出现才生效，所以全部改写）。
fn info_hits(doc: &AssDocument, keys: &[&str]) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for si in doc.sections_of(SectionKind::ScriptInfo) {
        let Some(s) = doc.section(si) else { continue };
        for (ei, e) in s.entries.iter().enumerate() {
            if e.kind != EntryKind::KeyVal {
                continue;
            }
            let Some(k) = e.key() else { continue };
            if keys.iter().any(|want| want.eq_ignore_ascii_case(k)) {
                out.push((si, ei, k.to_owned()));
            }
        }
    }
    out
}

fn scale_info_lines(
    doc: &mut AssDocument,
    keys: &[&str],
    factor: Rational,
    a: &mut ResApplied,
) -> Result<()> {
    for (si, ei, key) in info_hits(doc, keys) {
        let current = doc
            .section(si)
            .and_then(|s| s.entry(ei))
            .and_then(Entry::value_trimmed)
            .unwrap_or_default()
            .to_owned();
        let Some(r) = scale_to_integer(&current, &factor) else {
            a.unparsable += 1;
            continue;
        };
        if r.approximated {
            a.approximated += 1;
        }
        if r.text == current {
            continue;
        }
        doc.section_mut(si)
            .ok_or_else(|| StkError::simple(Domain::Transform, 40, "Script Info 段消失"))?
            .set_field(ei, &key, &r.text)?;
        a.playres_changed += 1;
    }
    Ok(())
}

fn rewrite_playres(
    doc: &mut AssDocument,
    geo: &Geometry,
    factor: Rational,
    a: &mut ResApplied,
    report: &mut LossReport,
) -> Result<()> {
    let hits = info_hits(doc, &PLAYRES_KEYS);
    if hits.is_empty() {
        // 文件没有显式坐标空间：重写后的脚本必须自己说明新空间，否则 renderer 只能猜。
        insert_info_lines(
            doc,
            &[
                ("PlayResX", &geo.canvas.width.to_string()),
                ("PlayResY", &geo.canvas.height.to_string()),
            ],
        )?;
        a.playres_inserted = 2;
        report.user_attention(&format!(
            "源脚本没有 PlayRes 头，Rewrite 已写入 PlayResX/Y = {}x{}（= 输出画布）",
            geo.canvas.width, geo.canvas.height
        ));
        return Ok(());
    }
    scale_info_lines(doc, &PLAYRES_KEYS, factor, a)?;
    if hits.len() == 1 {
        report.user_attention(
            "源脚本只声明了单轴 PlayRes：另一轴继续由 renderer 按比例推导，缩放已保持纵横比一致",
        );
    }
    Ok(())
}

fn rewrite_layoutres(
    doc: &mut AssDocument,
    geo: &Geometry,
    layout_factor: Rational,
    a: &mut ResApplied,
    report: &mut LossReport,
) -> Result<()> {
    let hits = info_hits(doc, &LAYOUT_KEYS);
    if hits.is_empty() {
        return Ok(());
    }
    match geo.layout_policy {
        LayoutResPolicy::KeepAsIs => {
            if factor_is_not_one(geo)? {
                report.user_attention(
                    "LayoutRes 按默认策略保持原值：它决定 libass 的 PAR 与 blur 基准（libass/ass_render.c:1008-1020），不随 PlayRes 一起改",
                );
            }
            Ok(())
        }
        LayoutResPolicy::ScaleWithScript => {
            if geo.warns_double_scaling() {
                report.user_attention(
                    "PlayRes 与 LayoutRes 同时缩放：libass 会用 LayoutRes 覆盖 storage 语义并据此重算 PAR，可能产生双重缩放，必须由 corpus 判定",
                );
            }
            let before = a.playres_changed;
            scale_info_lines(doc, &LAYOUT_KEYS, layout_factor, a)?;
            a.layout_scaled = a.playres_changed.saturating_sub(before);
            a.playres_changed = before;
            Ok(())
        }
        LayoutResPolicy::Drop => {
            let mut removed = 0usize;
            // 倒序删除，避免下标漂移
            for (si, ei, _) in hits.iter().rev() {
                if let Some(s) = doc.section_mut(*si) {
                    if *ei < s.entries.len() {
                        s.entries.remove(*ei);
                        removed += 1;
                    }
                }
            }
            a.layout_dropped = removed;
            report.dropped("LayoutResX/LayoutResY（交由 libass 回退到 storage/PlayRes）");
            Ok(())
        }
    }
}

fn factor_is_not_one(geo: &Geometry) -> Result<bool> {
    Ok(geo.script_scale_factor()? != Rational::ONE)
}

fn rewrite_styles(
    doc: &mut AssDocument,
    factor: Rational,
    ctx: &TagContext,
    a: &mut ResApplied,
) -> Result<()> {
    for si in doc.sections_of(SectionKind::Styles) {
        let idxs = match doc.section(si) {
            Some(s) => s.style_entries(),
            None => continue,
        };
        for field in SCALED_STYLE_FIELDS {
            let Some(fidx) = doc.section(si).and_then(|s| s.index_of(field)) else {
                continue;
            };
            let is_border = matches!(field, "Outline" | "Shadow");
            if is_border && !ctx.scaled_border_and_shadow {
                // 非 SBS：样式 Outline/Shadow 走 blur_scale（libass/ass_render.c:1053-1059），Rewrite 不动
                a.legacy_border_kept += idxs.len();
                continue;
            }
            for ei in &idxs {
                let current = match doc
                    .section(si)
                    .and_then(|s| s.entry(*ei))
                    .and_then(|e| e.field_slice(None, fidx))
                {
                    Some(v) => v.to_owned(),
                    None => continue,
                };
                let Some(r) = scale_to_integer(&current, &factor) else {
                    if !current.is_empty() {
                        a.unparsable += 1;
                    }
                    continue;
                };
                if r.approximated {
                    a.approximated += 1;
                }
                if r.text == current {
                    continue;
                }
                doc.section_mut(si)
                    .ok_or_else(|| StkError::simple(Domain::Transform, 41, "Styles 段消失"))?
                    .set_field(*ei, field, &r.text)?;
                a.style_fields += 1;
            }
        }
    }
    Ok(())
}

fn rewrite_events(
    doc: &mut AssDocument,
    factor: Rational,
    ctx: &TagContext,
    a: &mut ResApplied,
    report: &mut LossReport,
) -> Result<()> {
    for si in doc.sections_of(SectionKind::Events) {
        let (idxs, tail, named_text) = match doc.section(si) {
            Some(s) => (s.dialogue_entries(), s.event_tail(), s.index_of("Text")),
            None => continue,
        };
        let comment_count = match doc.section(si) {
            Some(s) => s
                .entries
                .iter()
                .filter(|e| e.kind == EntryKind::Comment)
                .count(),
            None => 0,
        };
        a.comments_skipped += comment_count;
        let margin_idx: Vec<(&str, usize)> = SCALED_EVENT_FIELDS
            .iter()
            .filter_map(|f| doc.section(si).and_then(|s| s.index_of(f)).map(|i| (*f, i)))
            .collect();
        for ei in &idxs {
            for (field, fidx) in &margin_idx {
                let current = match doc
                    .section(si)
                    .and_then(|s| s.entry(*ei))
                    .and_then(|e| e.field_slice(tail, *fidx))
                {
                    Some(v) => v.to_owned(),
                    None => continue,
                };
                let Some(r) = scale_to_integer(&current, &factor) else {
                    if !current.is_empty() {
                        a.unparsable += 1;
                    }
                    continue;
                };
                if r.approximated {
                    a.approximated += 1;
                }
                if r.text == current {
                    continue;
                }
                doc.section_mut(si)
                    .ok_or_else(|| StkError::simple(Domain::Transform, 42, "Events 段消失"))?
                    .set_field(*ei, field, &r.text)?;
                a.event_fields += 1;
            }
            let effect = doc
                .section(si)
                .and_then(|s| s.dialogue_view(*ei))
                .and_then(|v| v.effect().map(|s| s.trim().to_owned()))
                .unwrap_or_default();
            if !effect.is_empty() {
                a.effects_kept += 1;
            }
            let text_idx = match doc
                .section(si)
                .and_then(|s| s.entry(*ei))
                .map(|e| named_text.unwrap_or_else(|| e.field_count(tail).saturating_sub(1)))
            {
                Some(i) => i,
                None => continue,
            };
            let text = match doc
                .section(si)
                .and_then(|s| s.entry(*ei))
                .and_then(|e| e.field_slice(tail, text_idx))
            {
                Some(v) => v.to_owned(),
                None => continue,
            };
            if !text.contains('\\') {
                continue;
            }
            let mut stat = TagStats::default();
            let scaled = tags::scale_text(&text, &factor, ctx, &mut stat);
            a.tag_numbers += stat.scaled;
            a.approximated += stat.approximated;
            a.unparsable += stat.unparsable;
            a.blur_kept += stat.blur_kept;
            a.legacy_border_kept += stat.legacy_border_kept;
            a.unknown_tags += stat.unknown_names.len();
            for name in stat.unknown_names {
                report.nonstandard(&format!("未识别 tag 已原样保留：{name}"));
            }
            if scaled == text {
                continue;
            }
            doc.section_mut(si)
                .ok_or_else(|| StkError::simple(Domain::Transform, 42, "Events 段消失"))?
                .entry_mut(*ei)
                .ok_or_else(|| StkError::simple(Domain::Transform, 42, "事件条目消失"))?
                .set_field_at(tail, text_idx, &scaled)?;
        }
    }
    Ok(())
}

/// 在第一个 Script Info 段末尾追加 `Key: value` 行。
fn insert_info_lines(doc: &mut AssDocument, lines: &[(&str, &str)]) -> Result<()> {
    let si = doc
        .sections_of(SectionKind::ScriptInfo)
        .into_iter()
        .next()
        .ok_or_else(|| {
            StkError::simple(
                Domain::Transform,
                38,
                "Rewrite 需要 Script Info 段来写入 PlayRes",
            )
            .with_action("改用 Preserve，或先补齐 [Script Info] 与 PlayResX/PlayResY")
        })?;
    let eol = match doc.newline.as_str() {
        "\r\n" => Eol::CrLf,
        "\r" => Eol::Cr,
        _ => Eol::Lf,
    };
    let mut raws: Vec<String> = Vec::new();
    for (k, v) in lines {
        raws.push(format!("{k}: {v}"));
    }
    let section = doc
        .section_mut(si)
        .ok_or_else(|| StkError::simple(Domain::Transform, 40, "Script Info 段消失"))?;
    // 原末行若没有终止符，直接追加会把两行粘成一行
    if let Some(last) = section.entries.last_mut() {
        if last.eol == Eol::Eof {
            last.eol = eol;
        }
    }
    for raw in raws {
        // 我生成的行没有 key 后空白之外的写法：`Key: value`，值起点 = key 长度 + 2
        let at = raw.find(':').map(|c| c + 2).unwrap_or(raw.len());
        section.entries.push(Entry {
            kind: EntryKind::KeyVal,
            raw,
            eol,
            at: Some(at),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::canvas::Canvas;
    use core_model::geometry::LayoutRes;

    const SRC: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, Outline, Shadow, Spacing, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,48,&H00FFFFFF,3,2,1,60,60,50\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(960,540)\\bord2\\blur4}你好\r\n";

    fn doc() -> AssDocument {
        AssDocument::parse(SRC)
    }
    fn geo(canvas: Canvas, policy: ScriptResolutionPolicy) -> Geometry {
        Geometry {
            canvas,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: policy,
            ..Geometry::default()
        }
    }

    #[test]
    fn preserve_is_byte_identical() {
        // 09 §9 硬规则
        let mut d = doc();
        let before = d.render();
        let mut loss = LossReport::new();
        let a = apply(
            &mut d,
            &geo(Canvas::UHD2160, ScriptResolutionPolicy::Preserve),
            &mut loss,
        )
        .unwrap();
        assert_eq!(a.total(), 0);
        assert_eq!(d.render(), before);
        assert!(loss.preserved.iter().any(|p| p.contains("Preserve")));
    }

    #[test]
    fn preserve_ignores_layout_policy_with_a_warning() {
        let mut g = geo(Canvas::UHD2160, ScriptResolutionPolicy::Preserve);
        g.layout = Some(LayoutRes { x: 1920, y: 1080 });
        g.layout_policy = LayoutResPolicy::Drop;
        let mut d = doc();
        let mut loss = LossReport::new();
        apply(&mut d, &g, &mut loss).unwrap();
        assert_eq!(d.render(), SRC, "Preserve 下即使策略是 Drop 也不得改文件");
        assert!(!loss.user_attention.is_empty());
    }

    #[test]
    fn rewrite_scales_script_space_and_keeps_layout_independent_values() {
        let mut d = doc();
        let mut loss = LossReport::new();
        let a = apply(
            &mut d,
            &geo(Canvas::UHD2160, ScriptResolutionPolicy::Rewrite),
            &mut loss,
        )
        .unwrap();
        let out = d.render();
        assert!(out.contains("PlayResX: 3840"));
        assert!(out.contains("PlayResY: 2160"));
        assert!(out.contains("Style: Default,Arial,96,&H00FFFFFF,6,4,2,120,120,100"));
        assert!(out.contains("\\pos(1920,1080)"));
        assert!(
            out.contains("\\bord4"),
            "SBS 默认 yes：\\bord 属脚本坐标空间"
        );
        assert!(
            out.contains("\\blur4"),
            "libass blur_scale 不含 PlayRes：保持原值"
        );
        assert!(a.playres_changed >= 2);
        assert!(a.style_fields >= 6);
        assert!(a.tag_numbers >= 3);
        assert_eq!(a.blur_kept, 1);
        assert!(a.total() > 0);
    }

    #[test]
    fn rewrite_to_same_canvas_changes_nothing() {
        let mut d = doc();
        let before = d.render();
        let mut loss = LossReport::new();
        let a = apply(
            &mut d,
            &geo(Canvas::FHD1080, ScriptResolutionPolicy::Rewrite),
            &mut loss,
        )
        .unwrap();
        assert_eq!(a.total(), 0);
        assert_eq!(d.render(), before);
    }

    #[test]
    fn rewrite_rejects_geometry_that_disagrees_with_the_file() {
        let mut d = doc();
        let mut g = geo(Canvas::UHD2160, ScriptResolutionPolicy::Rewrite);
        g.script = Some(ScriptSize { x: 1280, y: 720 });
        let mut loss = LossReport::new();
        let e = apply(&mut d, &g, &mut loss).unwrap_err();
        assert_eq!(e.code_string(), "STK-TRF-0039");
        assert_eq!(d.render(), SRC, "拒绝时必须一行都不写");
    }

    #[test]
    fn missing_script_is_taken_from_the_file() {
        let d = doc();
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: None,
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let fixed = resolve_script(&g, &d);
        assert_eq!(fixed.script, Some(ScriptSize { x: 1920, y: 1080 }));
        assert_eq!(fixed.script_scale_factor().unwrap(), Rational::from_int(2));
    }

    #[test]
    fn non_sbs_document_keeps_style_borders() {
        let src = SRC.replace(
            "ScriptType: v4.00+\r\n",
            "ScriptType: v4.00+\r\nScaledBorderAndShadow: no\r\n",
        );
        let mut d = AssDocument::parse(&src);
        let mut loss = LossReport::new();
        let a = apply(
            &mut d,
            &geo(Canvas::UHD2160, ScriptResolutionPolicy::Rewrite),
            &mut loss,
        )
        .unwrap();
        let out = d.render();
        assert!(out.contains("Style: Default,Arial,96,&H00FFFFFF,3,2,2,120,120,100"));
        assert!(
            out.contains("\\bord2"),
            "非 SBS：\\bord 走 blur_scale，保持原值"
        );
        assert_eq!(a.legacy_border_kept, 3);
        assert!(loss.user_attention.iter().any(|s| s.contains("no")));
    }

    #[test]
    fn layout_res_default_kept_and_only_warned() {
        let src = SRC.replace(
            "PlayResY: 1080\r\n",
            "PlayResY: 1080\r\nLayoutResX: 1440\r\nLayoutResY: 1080\r\n",
        );
        let mut d = AssDocument::parse(&src);
        let mut g = geo(Canvas::UHD2160, ScriptResolutionPolicy::Rewrite);
        g.layout = Some(LayoutRes { x: 1440, y: 1080 });
        let mut loss = LossReport::new();
        let a = apply(&mut d, &g, &mut loss).unwrap();
        let out = d.render();
        assert!(out.contains("LayoutResX: 1440"), "KeepAsIs 是默认");
        assert_eq!(a.layout_scaled, 0);
        assert_eq!(a.layout_dropped, 0);
        assert!(loss.user_attention.iter().any(|s| s.contains("LayoutRes")));
    }

    #[test]
    fn layout_res_scale_and_drop_policies() {
        let src = SRC.replace(
            "PlayResY: 1080\r\n",
            "PlayResY: 1080\r\nLayoutResX: 1920\r\nLayoutResY: 1080\r\n",
        );
        let mut g = geo(Canvas::UHD2160, ScriptResolutionPolicy::Rewrite);
        g.layout = Some(LayoutRes { x: 1920, y: 1080 });
        g.layout_policy = LayoutResPolicy::ScaleWithScript;
        let mut d = AssDocument::parse(&src);
        let mut loss = LossReport::new();
        let a = apply(&mut d, &g, &mut loss).unwrap();
        assert_eq!(a.layout_scaled, 2);
        assert!(d.render().contains("LayoutResX: 3840"));
        assert!(loss.user_attention.iter().any(|s| s.contains("双重缩放")));

        g.layout_policy = LayoutResPolicy::Drop;
        let mut d2 = AssDocument::parse(&src);
        let mut loss2 = LossReport::new();
        let a2 = apply(&mut d2, &g, &mut loss2).unwrap();
        assert_eq!(a2.layout_dropped, 2);
        let out = d2.render();
        assert!(!out.contains("LayoutResX") && !out.contains("LayoutResY"));
        assert_eq!(out.matches("[Script Info]").count(), 1);
    }

    #[test]
    fn playres_is_inserted_when_absent() {
        let src = "[Script Info]\r\nScriptType: v4.00+\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontsize\r\nStyle: Default,20\r\n";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        // 文件没有 PlayRes 头时，resolve 拿不到基准 -> Rewrite 必须报错而不是猜系数
        let unknown = Geometry { script: None, ..g };
        let mut loss = LossReport::new();
        let g2 = resolve_script(&unknown, &d);
        assert_eq!(g2.script, None);
        let e = apply(&mut d, &g2, &mut loss).unwrap_err();
        assert_eq!(e.code_string(), "STK-TRF-0035");
        // 由用户显式给出基准时：写入等于画布的 PlayRes
        let mut loss2 = LossReport::new();
        let a = apply(&mut d, &g, &mut loss2).unwrap();
        assert_eq!(a.playres_inserted, 2);
        let out = d.render();
        assert!(out.contains("PlayResX: 3840\r\nPlayResY: 2160\r\n"));
        assert!(out.contains("Style: Default,40"));
    }

    #[test]
    fn insertion_fixes_missing_trailing_newline() {
        let src = "[Script Info]\r\nScriptType: v4.00+";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        apply(&mut d, &g, &mut loss).unwrap();
        let out = d.render();
        assert!(out.contains("v4.00+\r\nPlayResX: 3840"), "{out}");
        assert!(!out.contains("v4.00+PlayResX"));
    }

    #[test]
    fn no_script_info_section_is_rejected() {
        let mut d = AssDocument::parse(
            "[Events]\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,x\r\n",
        );
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        assert_eq!(
            apply(&mut d, &g, &mut loss).unwrap_err().code_string(),
            "STK-TRF-0038"
        );
    }

    #[test]
    fn non_16_9_canvas_needs_explicit_custom_flag() {
        let mut g = geo(
            Canvas {
                width: 1920,
                height: 800,
            },
            ScriptResolutionPolicy::Rewrite,
        );
        g.canvas_is_custom = true;
        let mut d = doc();
        let mut loss = LossReport::new();
        // 自定义画布允许，但两轴系数不一致（1920/1920 vs 800/1080）会被几何层拒绝
        assert_eq!(
            apply(&mut d, &g, &mut loss).unwrap_err().code_string(),
            "STK-TRF-0036"
        );
    }

    #[test]
    fn half_up_rounding_is_reported_once() {
        // 640x360 -> 1920x1080 系数 3；1280x720 -> 1920x1080 系数 1.5（奇数会取整）
        let src = "[Script Info]\r\nPlayResX: 1280\r\nPlayResY: 720\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontsize, MarginV\r\nStyle: Default,25,17\r\n";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::FHD1080,
            script: Some(ScriptSize { x: 1280, y: 720 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        let a = apply(&mut d, &g, &mut loss).unwrap();
        let out = d.render();
        assert!(out.contains("Style: Default,38,26"), "{out}");
        assert!(a.approximated >= 2);
        assert_eq!(
            loss.approximated
                .iter()
                .filter(|s| s.contains("half-up"))
                .count(),
            1,
            "同类近似只进报告一次"
        );
    }

    #[test]
    fn effect_and_comment_are_accounted_but_untouched() {
        let src = "[Script Info]\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nComment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(10,10)}留着\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,Banner;20;1,{\\pos(10,10)}x\r\n";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        let a = apply(&mut d, &g, &mut loss).unwrap();
        let out = d.render();
        assert!(out.contains("Comment: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(10,10)}留着"));
        assert!(out.contains("Banner;20;1,{\\pos(20,20)}x"));
        assert_eq!(a.comments_skipped, 1);
        assert_eq!(a.effects_kept, 1);
        assert!(a.event_fields == 0, "事件边距为 0 时不写");
    }

    #[test]
    fn unknown_tags_survive_and_are_reported() {
        let src = "[Script Info]\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\vsfmod 3}{\\pos(10,10)}x\r\n";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        let a = apply(&mut d, &g, &mut loss).unwrap();
        let out = d.render();
        assert!(out.contains("{\\vsfmod 3}{\\pos(20,20)}x"));
        assert_eq!(a.unknown_tags, 1);
        assert_eq!(
            loss.count_for("nonstandard", "未识别 tag 已原样保留：\\vsfmod"),
            1
        );
    }

    #[test]
    fn rational_factor_is_exact_for_three_halves() {
        let src = "[Script Info]\r\nPlayResX: 1280\r\nPlayResY: 720\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(31,17)\\org(5,5)}x\r\n";
        let mut d = AssDocument::parse(src);
        let g = Geometry {
            canvas: Canvas::FHD1080,
            script: Some(ScriptSize { x: 1280, y: 720 }),
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        };
        let mut loss = LossReport::new();
        apply(&mut d, &g, &mut loss).unwrap();
        // 1.5 倍：31 -> 46.5，坐标允许小数（\pos 是浮点参数）
        assert!(d.render().contains("\\pos(46.5,25.5)\\org(7.5,7.5)"));
    }
}
