//! ASS HDR 颜色预调整（11_HDR_COLOR.md）。
//! 语义：对已有 ASS 的颜色值做映射，让它在 HDR 播放链里达到期望主观亮度；
//! **不是**给 ASS 写 HDR metadata。Alpha 通道独立，默认完全不改。
//!
//! 算法版本进报告，因为"同一 nit 不同实现结果不同"必须可追溯。

use std::cell::Cell;

use core_model::color::AssColor;
use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use core_model::profile::{HdrMode, HdrSettings};
use core_model::time::Rational;

/// 改动映射时升版本。
pub const ALGORITHM_VERSION: u32 = 1;
/// SDR 参考白（nit），ReferenceWhite 模式的映射基准。
pub const SDR_REFERENCE_NITS: u32 = 203;
/// Compatibility 模式：RGB 各通道 × 1/2（社区工具"降一半"的可预期行为）。
pub const COMPATIBILITY_FACTOR: Rational = Rational::from_const(1, 2);
/// 样式的四个颜色字段。
pub const STYLE_COLOR_FIELDS: [&str; 4] = [
    "PrimaryColour",
    "SecondaryColour",
    "OutlineColour",
    "BackColour",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct HdrApplied {
    pub style_colors: usize,
    pub inline_colors: usize,
    pub alpha_touched: usize,
    pub unparsable_colors: usize,
    pub unknown_color_tags: usize,
}

impl HdrApplied {
    pub fn total(&self) -> usize {
        self.style_colors + self.inline_colors
    }
}

/// 当前设置的 RGB 缩放系数（有理数，0<r<=1）。
pub fn factor(settings: &HdrSettings) -> Result<Rational> {
    match settings.mode {
        HdrMode::Off => Ok(Rational::ONE),
        HdrMode::Compatibility => Ok(COMPATIBILITY_FACTOR),
        HdrMode::Percentage => {
            let bp = settings.percent_basis_points;
            if bp == 0 || bp > 10_000 {
                return Err(bad("HDR 百分比必须在 0.01%..100%", 60));
            }
            Rational::new(bp as i64, 10_000).map_err(|_| bad("HDR 百分比非法", 60))
        }
        HdrMode::ReferenceWhite => {
            let nits = settings.reference_nits;
            if nits == 0 {
                return Err(bad("reference white nit 不能为 0", 61));
            }
            let f = Rational::new(nits as i64, i64::from(SDR_REFERENCE_NITS))
                .map_err(|_| bad("reference white 比例非法", 61))?;
            // 只做预压暗，不提亮；>1 钳到 1 由调用方在报告里体现为"无变化"
            Ok(if f > Rational::ONE { Rational::ONE } else { f })
        }
    }
}

fn bad(msg: &str, code: u16) -> StkError {
    StkError::simple(Domain::Transform, code, msg)
}

/// 逐通道缩放。`linear=true` 走 sRGB 线性化域（Percentage / ReferenceWhite），
/// 否则按通道整数缩放（Compatibility 的"可预期"行为）。
pub fn scale_channel(v: u8, f: &Rational, linear: bool) -> u8 {
    if !linear {
        let n = i64::from(v).checked_mul(f.num()).unwrap_or(0).max(0);
        let d = (n + f.den() / 2) / f.den().max(1);
        return d.clamp(0, 255) as u8;
    }
    linear_to_srgb(srgb_to_linear(v) * f.to_f64())
}

/// 只用于颜色；时间/坐标禁止走浮点（ADR-007）。
fn srgb_to_linear(v: u8) -> f64 {
    let c = f64::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(v: f64) -> u8 {
    let c = v.clamp(0.0, 1.0);
    let enc = if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (enc * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

fn scale_color(c: AssColor, f: &Rational, linear: bool, touch_alpha: bool) -> AssColor {
    AssColor {
        r: scale_channel(c.r, f, linear),
        g: scale_channel(c.g, f, linear),
        b: scale_channel(c.b, f, linear),
        a: if touch_alpha {
            scale_channel(c.a, f, linear)
        } else {
            c.a
        },
    }
}

/// 就地变换文档。
pub fn apply(
    doc: &mut codec_ass::AssDocument,
    settings: &HdrSettings,
    report: &mut LossReport,
) -> Result<HdrApplied> {
    if settings.mode == HdrMode::Off {
        report.preserved("HDR 未启用，文档保持原样");
        return Ok(HdrApplied::default());
    }
    let f = factor(settings)?;
    let linear = settings.mode == HdrMode::Percentage || settings.mode == HdrMode::ReferenceWhite;
    let mut a = HdrApplied::default();

    if settings.touch_styles {
        for si in doc.sections_of(codec_ass::SectionKind::Styles) {
            let idxs = doc
                .section(si)
                .map(|s| s.style_entries())
                .unwrap_or_default();
            for field in STYLE_COLOR_FIELDS {
                let idx = match doc.section(si).and_then(|s| s.index_of(field)) {
                    Some(i) => i,
                    None => continue,
                };
                for ei in &idxs {
                    let current = match doc
                        .section(si)
                        .and_then(|s| s.entry(*ei))
                        .and_then(|e| e.field_slice(None, idx))
                    {
                        Some(v) => v.to_string(),
                        None => continue,
                    };
                    let parsed = match AssColor::parse_ass(&current) {
                        Ok(c) => c,
                        Err(_) => {
                            a.unparsable_colors += 1;
                            report.user_attention(&format!(
                                "样式颜色无法解析，已原样保留：{current}"
                            ));
                            continue;
                        }
                    };
                    let scaled = scale_color(parsed, &f, linear, settings.touch_alpha);
                    if scaled == parsed {
                        continue;
                    }
                    if scaled.a != parsed.a {
                        a.alpha_touched += 1;
                    }
                    let out = scaled.to_ass_like(&current);
                    doc.section_mut(si)
                        .ok_or_else(|| bad("样式段消失", 63))?
                        .set_field(*ei, field, &out)?;
                    a.style_colors += 1;
                }
            }
        }
    }

    if settings.touch_inline {
        for si in doc.sections_of(codec_ass::SectionKind::Events) {
            let (idxs, text_idx) = match doc.section(si) {
                Some(s) => (s.dialogue_entries(), s.index_of("Text")),
                None => continue,
            };
            let tail = doc.section(si).and_then(|s| s.event_tail());
            let ti = match text_idx {
                Some(i) => i,
                None => continue,
            };
            for ei in &idxs {
                let text = match doc
                    .section(si)
                    .and_then(|s| s.entry(*ei))
                    .and_then(|e| e.field_slice(tail, ti))
                {
                    Some(v) => v.to_string(),
                    None => continue,
                };
                if !text.contains('\\') {
                    continue;
                }
                let hits = Cell::new(0usize);
                let alpha = Cell::new(0usize);
                let mapped =
                    codec_ass::map_color_tokens(&text, &|c: AssColor| -> Result<AssColor> {
                        hits.set(hits.get() + 1);
                        let s = scale_color(c, &f, linear, settings.touch_alpha);
                        if s.a != c.a {
                            alpha.set(alpha.get() + 1);
                        }
                        Ok(s)
                    })?;
                a.inline_colors += hits.get();
                a.alpha_touched += alpha.get();
                if hits.get() == 0 {
                    if let Some(u) = count_unknown_color_tags(&text) {
                        a.unknown_color_tags += u;
                    }
                }
                if mapped != text {
                    doc.section_mut(si)
                        .ok_or_else(|| bad("事件段消失", 63))?
                        .set_field(*ei, "Text", &mapped)?;
                }
            }
        }
    }

    if a.total() == 0 {
        report.preserved("没有命中任何可调整的颜色字段");
        return Ok(a);
    }
    report.approximated(&format!(
        "HDR 颜色预调整 factor={f}（算法版本 {ALGORITHM_VERSION}）"
    ));
    if a.alpha_touched > 0 {
        report.user_attention("本次调整触及 alpha 通道（默认应关闭）");
    }
    if a.unparsable_colors > 0 {
        report.user_attention(&format!(
            "{} 处颜色值无法解析，已原样保留",
            a.unparsable_colors
        ));
    }
    if a.unknown_color_tags > 0 {
        report.nonstandard(&format!(
            "{} 处未识别的颜色类扩展 tag，已原样保留",
            a.unknown_color_tags
        ));
    }
    Ok(a)
}

/// 未识别但形似颜色的 tag（VSFilterMod 渐变等）：只统计，不改写（11 §2 要求显式声明处理范围）。
fn count_unknown_color_tags(text: &str) -> Option<usize> {
    let mut n = 0usize;
    for t in codec_ass::parse_overrides(text) {
        if let codec_ass::OverrideToken::Block(b) = t {
            for tg in &b.tags {
                if tg.is_unknown() && tg.plain_name().to_ascii_lowercase().contains('c') {
                    n += 1;
                }
            }
        }
    }
    if n > 0 {
        Some(n)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_ass::AssDocument;

    const SAMPLE: &str = "[Script Info]\nTitle: t\nPlayResX: 1920\nPlayResY: 1080\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,3,0,2,10,10,10,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\c&H00FFFF&H\\pos(960,900)}白字\n\
Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,无颜色文本\n";

    fn doc() -> AssDocument {
        AssDocument::parse(SAMPLE)
    }

    #[test]
    fn off_mode_is_byte_noop() {
        let mut d = doc();
        let before = d.render();
        let a = apply(&mut d, &HdrSettings::off(), &mut LossReport::new()).unwrap();
        assert_eq!(a.total(), 0);
        assert_eq!(d.render(), before);
    }

    #[test]
    fn compatibility_halves_rgb_by_integers() {
        let f = factor(&HdrSettings::compatibility()).unwrap();
        assert_eq!(f, COMPATIBILITY_FACTOR);
        assert_eq!(scale_channel(255, &f, false), 128);
        assert_eq!(scale_channel(235, &f, false), 118);
        assert_eq!(scale_channel(0, &f, false), 0);
    }

    #[test]
    fn percentage_and_reference_white_use_linear_light() {
        let p = HdrSettings::percentage(5000).unwrap();
        let f = factor(&p).unwrap();
        assert_eq!(f, Rational::new(1, 2).unwrap());
        let w = scale_channel(255, &f, true);
        let m = scale_channel(128, &f, true);
        assert!(w < 255 && w > 180, "50% 线性光下白色应仍偏亮：{w}");
        assert!(m < 128 && m > 40, "中灰：{m}");
        assert!(factor(&HdrSettings::reference_white(100).unwrap()).unwrap() < Rational::ONE);
        assert_eq!(
            factor(&HdrSettings::reference_white(300).unwrap()).unwrap(),
            Rational::ONE,
            "不许提亮"
        );
        assert!(HdrSettings::reference_white(0).is_err());
    }

    #[test]
    fn alpha_is_untouched_by_default() {
        let mut d = doc();
        let a = apply(
            &mut d,
            &HdrSettings::compatibility(),
            &mut LossReport::new(),
        )
        .unwrap();
        assert_eq!(a.alpha_touched, 0);
        let s = d.section(1).unwrap();
        let back = s
            .entry(s.style_entries()[0])
            .unwrap()
            .field_slice(None, s.index_of("BackColour").unwrap())
            .unwrap();
        assert!(
            back.to_uppercase().contains("80000000"),
            "alpha 0x80 必须保留，实际 {back}"
        );
    }

    #[test]
    fn only_color_lines_change() {
        // AT-ASS-002：时间/文本/几何 hash 不变
        let original = doc().render();
        let mut d = doc();
        let a = apply(
            &mut d,
            &HdrSettings::percentage(4000).unwrap(),
            &mut LossReport::new(),
        )
        .unwrap();
        assert!(a.total() > 0, "必须真的改到颜色，否则测试是空的");
        let after = d.render();
        let b: Vec<&str> = original.lines().collect();
        let c: Vec<&str> = after.lines().collect();
        assert_eq!(b.len(), c.len());
        let mut changed = 0;
        for (x, y) in b.iter().zip(&c) {
            if x != y {
                changed += 1;
                let color_bearing =
                    (x.starts_with("Style:") && x.contains("&H")) || x.contains("\\c&H");
                assert!(color_bearing, "不该变的行变了：{x}");
            }
        }
        assert_eq!(
            changed, 2,
            "1 样式行 + 1 含 inline 颜色的事件行，实际 {changed}"
        );
        assert!(after.contains("0,0:00:01.00,0:00:02.00"));
        assert!(after.contains("\\pos(960,900)"));
        assert!(after.contains("无颜色文本"));
    }

    #[test]
    fn dirty_and_unknown_content_survives() {
        let src = "[V4+ Styles]\nFormat: Name, PrimaryColour\nStyle: A,&HZZZZZZ\n\n[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:02.00,A,{\\fad(200,200)}x\n";
        let mut d = AssDocument::parse(src);
        let mut rep = LossReport::new();
        let a = apply(&mut d, &HdrSettings::compatibility(), &mut rep).unwrap();
        assert_eq!(a.unparsable_colors, 1, "脏颜色应被记数");
        assert!(d.render().contains("&HZZZZZZ"));
        assert!(d.render().contains("\\fad(200,200)"));
        assert!(
            rep.user_attention.iter().any(|s| s.contains("无法解析")),
            "{:?}",
            rep.user_attention
        );
    }

    #[test]
    fn nested_animation_color_is_scaled() {
        let src = "[Events]\nFormat: Layer, Start, End, Style, Text\nDialogue: 0,0:00:01.00,0:00:02.00,D,{\\t(0,1000,\\c&HFFFFFF&)}x\n";
        let mut d = AssDocument::parse(src);
        let a = apply(
            &mut d,
            &HdrSettings::compatibility(),
            &mut LossReport::new(),
        )
        .unwrap();
        assert_eq!(a.inline_colors, 1);
        assert!(d.render().contains("&H808080&"), "实际 {}", d.render());
    }

    #[test]
    fn scopes_off_is_noop() {
        let mut s = HdrSettings::compatibility();
        s.touch_styles = false;
        s.touch_inline = false;
        let mut d = doc();
        let before = d.render();
        let a = apply(&mut d, &s, &mut LossReport::new()).unwrap();
        assert_eq!(a.total(), 0);
        assert_eq!(d.render(), before);
    }

    #[test]
    fn factor_errors_are_typed() {
        let mut p = HdrSettings::compatibility();
        p.mode = HdrMode::Percentage;
        p.percent_basis_points = 0;
        assert_eq!(factor(&p).unwrap_err().code_string(), "STK-TRF-0060");
        let mut r = HdrSettings::compatibility();
        r.mode = HdrMode::ReferenceWhite;
        r.reference_nits = 0;
        assert_eq!(factor(&r).unwrap_err().code_string(), "STK-TRF-0061");
    }

    #[test]
    fn scaling_is_monotonic_and_never_brightens() {
        let f = Rational::new(1, 2).unwrap();
        for v in 0..255u8 {
            assert!(
                scale_channel(v + 1, &f, false) >= scale_channel(v, &f, false),
                "整数域 {v}"
            );
            assert!(scale_channel(v, &f, true) <= v, "线性域不得提亮 {v}");
            assert!(
                scale_channel(v + 1, &f, true) >= scale_channel(v, &f, true),
                "线性域单调 {v}"
            );
        }
    }

    #[test]
    fn repeated_application_is_deterministic() {
        let s = HdrSettings::percentage(6000).unwrap();
        let mut d1 = doc();
        let mut d2 = doc();
        apply(&mut d1, &s, &mut LossReport::new()).unwrap();
        apply(&mut d2, &s, &mut LossReport::new()).unwrap();
        assert_eq!(
            d1.render(),
            d2.render(),
            "同一输入必须得到同一输出（无浮点不确定性的表现差异）"
        );
    }
}
