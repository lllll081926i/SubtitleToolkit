//! 每个 override tag 的缩放归属（09 §4 B）。
//!
//! 判据不是凭感觉，而是 libass master 的 `init_font_scale()`（libass/ass_render.c:1036-1070）：
//!
//! - `screen_scale = frame_content / PlayRes` → 脚本坐标空间。重写 PlayRes 会改变它，
//!   所以这一类字面量必须乘缩放系数，否则位置/字号会变。
//! - `blur_scale = frame_content / layout_res`（layout_res 取值顺序见
//!   libass/ass_render.c:1008-1020：`LayoutRes*` 头 → storage size → PlayRes）→
//!   它不含 PlayRes，重写脚本分辨率不会改变它，所以这一类字面量必须保持原值。
//!   `ScaledBorderAndShadow: no` 时边框/阴影也走 `blur_scale`（ass_render.c:1053-1059）。
//! - 百分比（`\\fscx`）、角度（`\\fr*`）、毫秒（karaoke、`\\move` 时间参数）、
//!   `\\p` 精度因子都不是尺寸，永远不缩放。`\\pN` 在 libass 里是 2 的指数
//!   （`scale_base = 1 << (N-1)`，`libass/ass_render.c:712-715`、`1188-1191`），
//!   所以"坐标数字 ×S、`\\p` 不变"在 Rewrite 下几何严格等价。
//!
//! 因此对 `\\blur/\\be`，"Rewrite 时不缩放"在 libass 语义下是可证明的正确默认，
//! 而不只是待验证的保守选择；与 VSFilter/CSRI provider 是否等价仍要靠 differential corpus。

use codec_ass::{OverrideBlock, OverrideToken, Tag, TagKind};
use core_model::time::Rational;

use crate::numbers::{scale_literals, LiteralMode};

/// 边框/阴影的缩放归属取决于文档的 ScaledBorderAndShadow。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagContext {
    pub scaled_border_and_shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagAction {
    /// 全部参数里的数值都是脚本坐标/尺寸
    Scale,
    /// 前 4 个数值是坐标，其余是毫秒（`\\move`）
    ScaleFirstFour,
    /// drawing 文本：数值是坐标，但内部 `\\pN` 是精度因子
    ScaleDrawing,
    /// 与分辨率无关
    Keep,
    /// libass 里按 `blur_scale`（layout/storage 相对）生效，Rewrite 时保持原值
    KeepBlur,
    /// 非 SBS 文档的边框/阴影：同上，保持原值
    KeepLegacyBorder,
}

/// 只按 libass 源码可证的语义缩放；`xbl/ybl` 是 VSFilterMod 的分轴模糊写法，
/// 与 `blur` 同类，保持原值并计入待 corpus 审计。
fn by_other_name(name: &str) -> TagAction {
    match name {
        "fs" | "fsp" => TagAction::Scale,
        "blur" | "be" | "xbl" | "ybl" => TagAction::KeepBlur,
        _ => TagAction::Keep,
    }
}

pub fn action_for(tag: &Tag, ctx: &TagContext) -> TagAction {
    match &tag.kind {
        TagKind::Position | TagKind::Origin => TagAction::Scale,
        TagKind::Movement => TagAction::ScaleFirstFour,
        TagKind::Clip { .. } => TagAction::ScaleDrawing,
        TagKind::Border | TagKind::Shadow => {
            if ctx.scaled_border_and_shadow {
                TagAction::Scale
            } else {
                TagAction::KeepLegacyBorder
            }
        }
        TagKind::DrawingMode
        | TagKind::Scale
        | TagKind::Rotation
        | TagKind::Font
        | TagKind::Karaoke
        | TagKind::Colour { .. }
        | TagKind::Alpha { .. } => TagAction::Keep,
        TagKind::Animation { .. } => TagAction::Keep,
        TagKind::Other => by_other_name(&tag.plain_name().to_lowercase()),
        TagKind::Unknown => TagAction::Keep,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagStats {
    pub scaled: usize,
    pub approximated: usize,
    pub unparsable: usize,
    pub blur_kept: usize,
    pub legacy_border_kept: usize,
    /// 未识别 tag 名（可能是 VSFilterMod 私有 tag），原文保留
    pub unknown_names: Vec<String>,
}

fn add(
    stat: &mut TagStats,
    src: &str,
    factor: &Rational,
    mode: LiteralMode,
    limit: Option<usize>,
) -> String {
    let got = scale_literals(src, factor, mode, limit);
    stat.scaled += got.stat.scaled;
    stat.approximated += got.stat.approximated;
    stat.unparsable += got.stat.unparsable;
    got.text
}

fn scale_tag(tag: &mut Tag, factor: &Rational, ctx: &TagContext, stat: &mut TagStats) {
    if let TagKind::Animation { nested, .. } = &mut tag.kind {
        // `\t(...)` 的 head 只有毫秒与加速度，不能碰；内层 tag 按同样规则处理
        for inner in nested.iter_mut() {
            scale_tag(inner, factor, ctx, stat);
        }
        return;
    }
    if tag.is_unknown() {
        let name = tag.name.to_lowercase();
        if !stat.unknown_names.contains(&name) {
            stat.unknown_names.push(name);
        }
        return;
    }
    match action_for(tag, ctx) {
        TagAction::Scale => {
            tag.args = add(stat, &tag.args, factor, LiteralMode::Plain, None);
        }
        TagAction::ScaleFirstFour => {
            tag.args = add(stat, &tag.args, factor, LiteralMode::Plain, Some(4));
        }
        TagAction::ScaleDrawing => {
            tag.args = add(stat, &tag.args, factor, LiteralMode::Drawing, None);
        }
        TagAction::KeepBlur => stat.blur_kept += 1,
        TagAction::KeepLegacyBorder => stat.legacy_border_kept += 1,
        TagAction::Keep => {}
    }
}

pub fn scale_block(
    block: &mut OverrideBlock,
    factor: &Rational,
    ctx: &TagContext,
    stat: &mut TagStats,
) {
    for tag in &mut block.tags {
        scale_tag(tag, factor, ctx, stat);
    }
}

/// 缩放一段 Text（含 override block、`\\p` 模式下的 drawing 正文）。
pub fn scale_text(text: &str, factor: &Rational, ctx: &TagContext, stat: &mut TagStats) -> String {
    let mut tokens = codec_ass::parse_overrides(text);
    for token in &mut tokens {
        match token {
            OverrideToken::Block(block) => scale_block(block, factor, ctx, stat),
            OverrideToken::Drawing { raw } => {
                *raw = add(stat, raw, factor, LiteralMode::Drawing, None);
            }
            OverrideToken::Text { .. } => {}
        }
    }
    codec_ass::tokens_to_string(&tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_ass::parse_overrides;

    const SBS: TagContext = TagContext {
        scaled_border_and_shadow: true,
    };
    const NO_SBS: TagContext = TagContext {
        scaled_border_and_shadow: false,
    };
    const TWO: Rational = Rational::from_const(2, 1);

    fn roundtrip(text: &str, factor: &Rational, ctx: &TagContext) -> (String, TagStats) {
        let mut stat = TagStats::default();
        let out = scale_text(text, factor, ctx, &mut stat);
        (out, stat)
    }

    #[test]
    fn coordinates_scale_borders_scale_when_sbs() {
        let (out, stat) = roundtrip("{\\pos(100,200)}x{\\bord1.5}{\\shad3}", &TWO, &SBS);
        assert_eq!(out, "{\\pos(200,400)}x{\\bord3}{\\shad6}");
        assert_eq!(stat.scaled, 4);
        assert_eq!(stat.blur_kept, 0);
        // 非 SBS：边框/阴影走 blur_scale，Rewrite 不改
        let (out2, stat2) = roundtrip("{\\pos(100,200)}{\\bord1.5}", &TWO, &NO_SBS);
        assert_eq!(out2, "{\\pos(200,400)}{\\bord1.5}");
        assert_eq!(stat2.legacy_border_kept, 1);
    }

    #[test]
    fn move_times_are_not_scaled() {
        let (out, _) = roundtrip("{\\move(10,20,30,40,0,500,1000)}", &TWO, &SBS);
        assert_eq!(out, "{\\move(20,40,60,80,0,500,1000)}");
    }

    #[test]
    fn blur_and_scale_and_karaoke_are_kept() {
        let (out, stat) = roundtrip(
            "{\\blur8}{\\be2}{\\fscx150\\fscy150}{\\k50}\\p3",
            &TWO,
            &SBS,
        );
        assert_eq!(out, "{\\blur8}{\\be2}{\\fscx150\\fscy150}{\\k50}\\p3");
        assert_eq!(stat.blur_kept, 2);
        assert_eq!(stat.scaled, 0);
    }

    #[test]
    fn clip_drawing_keeps_inner_precision_factor() {
        let (out, stat) = roundtrip("{\\clip(\\p1 m 0 0 l 100 100\\p0)}", &TWO, &SBS);
        assert_eq!(out, "{\\clip(\\p1 m 0 0 l 200 200\\p0)}");
        assert_eq!(stat.scaled, 2);
        let (rect, _) = roundtrip("{\\iclip(10,10,200,300)}", &TWO, &SBS);
        assert_eq!(rect, "{\\iclip(20,20,400,600)}");
    }

    #[test]
    fn animation_head_survives_and_nested_scales() {
        let (out, _) = roundtrip("{\\t(0,1000,3,\\fscx50\\pos(10,20))}", &TWO, &SBS);
        assert_eq!(out, "{\\t(0,1000,3,\\fscx50\\pos(20,40))}");
        let (out2, _) = roundtrip("{\\t(0,700,\\bord1\\blur4)}", &TWO, &SBS);
        assert_eq!(out2, "{\\t(0,700,\\bord2\\blur4)}");
    }

    #[test]
    fn drawing_body_between_p1_and_p0_scales() {
        let (out, stat) = roundtrip("{\\p1}m 0 0 l 50 50{\\p0}", &TWO, &SBS);
        assert_eq!(out, "{\\p1}m 0 0 l 100 100{\\p0}");
        assert_eq!(stat.scaled, 2);
    }

    #[test]
    fn fs_and_fsp_scale_but_font_name_does_not() {
        let (out, _) = roundtrip("{\\fs20\\fsp1.5\\fnArial Black}", &TWO, &SBS);
        assert_eq!(out, "{\\fs40\\fsp3\\fnArial Black}");
    }

    #[test]
    fn unknown_tag_is_preserved_verbatim_and_reported() {
        let (out, stat) = roundtrip("{\\vsfmodtag(1,2)}{\\pos(5,5)}", &TWO, &SBS);
        assert_eq!(out, "{\\vsfmodtag(1,2)}{\\pos(10,10)}");
        assert_eq!(stat.unknown_names, vec!["\\vsfmodtag".to_owned()]);
    }

    #[test]
    fn identity_factor_touches_nothing() {
        for text in [
            "{\\pos(100,200)\\move(1,2,3,4,5,6,7)\\clip(\\p1 m 0 0 l 9 9\\p0)}",
            "{\\t(0,500,\\bord1.25\\blur3)}\\p1m 3 4{\\p0}",
            "{\\fscx120\\fspd2\\an8\\frz30\\fnCJK\\k30}",
        ] {
            let (out, stat) = roundtrip(text, &Rational::ONE, &SBS);
            assert_eq!(out, text);
            assert_eq!(stat.scaled, 0, "恒等系数不得改写任何字面量: {text}");
        }
    }

    #[test]
    fn whitespace_and_unterminated_blocks_are_kept() {
        let (out, _) = roundtrip("{ \\pos( 10 , 20 ) ", &TWO, &SBS);
        assert_eq!(out, "{ \\pos( 20 , 40 ) ");
        let (out2, _) = roundtrip("{\\pos10}", &TWO, &SBS);
        assert_eq!(out2, "{\\pos20}");
    }

    #[test]
    fn parse_then_scale_is_stable_for_nested_blocks() {
        let text = "{\\pos(1,2)}a{\\clip(\\p2 m 2 2 l 4 4)}b";
        let first = scale_text(text, &TWO, &SBS, &mut TagStats::default());
        let mut stat = TagStats::default();
        let second = scale_text(&first, &Rational::ONE, &SBS, &mut stat);
        assert_eq!(second, first);
        assert_eq!(stat.scaled, 0);
        let _ = parse_overrides(text);
    }
}
