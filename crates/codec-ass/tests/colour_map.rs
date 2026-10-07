//! 11_HDR_COLOR 的入口：`map_color_tokens` 只能碰到颜色 token。

use core_model::color::AssColor;
use core_model::error::{Domain, StkError};

use codec_ass::{map_color_tokens, parse_overrides, OverrideToken, TagKind};

fn halved(c: AssColor) -> AssColor {
    AssColor {
        a: c.a,
        r: c.r / 2,
        g: c.g / 2,
        b: c.b / 2,
    }
}

fn identity(c: AssColor) -> Result<AssColor, StkError> {
    Ok(c)
}

fn halve(c: AssColor) -> Result<AssColor, StkError> {
    Ok(halved(c))
}

#[test]
fn identity_map_changes_nothing() {
    for text in [
        "{\\c&H00FFFFFF&}白{\\3c&H80000000&H}边",
        "{\\alpha&H80&\\an5\\pos(1,2)}x",
        "{\\t(0,1000,\\c&H123456&)}内层",
        "没有颜色的文本",
        "{\\c&H00FFFF&H&HFF00FF&H}gradient",
        "{\\cFFFFFF}裸十六进制",
        "",
        "{\\unknown&H000000&}",
    ] {
        assert_eq!(map_color_tokens(text, &identity).unwrap(), text);
    }
}

#[test]
fn text_without_colour_is_untouched() {
    let text = "纯文本 {\\an8\\pos(960,120)\\bord3\\fnArial Black\\p1}m 0 0 l 9 9{\\p0}\\N换行";
    assert_eq!(map_color_tokens(text, &halve).unwrap(), text);
    assert_eq!(
        map_color_tokens("hello, world", &halve).unwrap(),
        "hello, world"
    );
}

#[test]
fn only_colour_tokens_change_and_width_is_preserved() {
    let text = "{\\c&HFFFFFF&\\2c&H01020304&\\alpha&H80&\\3a&HFF&}x{\\bord2}";
    let out = map_color_tokens(text, &halve).unwrap();
    // 6 位仍是 6 位，8 位仍是 8 位，alpha/边框/文本全部原样
    assert_eq!(
        out,
        "{\\c&H7F7F7F&\\2c&H01010102&\\alpha&H80&\\3a&HFF&}x{\\bord2}"
    );
    assert!(text.matches("&H").count() == out.matches("&H").count());
    // 逐 token 对照：非颜色 token 的原文完全一致
    let before = parse_overrides(text);
    let after = parse_overrides(&out);
    let tag_text = |tokens: &[OverrideToken]| -> Vec<String> {
        tokens
            .iter()
            .map(|t| match t {
                OverrideToken::Block(b) => b
                    .tags
                    .iter()
                    .map(|tag| format!("{}{:?}", tag.name, tag.kind))
                    .collect::<String>(),
                OverrideToken::Text { raw } | OverrideToken::Drawing { raw } => raw.clone(),
            })
            .collect()
    };
    assert_eq!(tag_text(&before).len(), tag_text(&after).len());
    for (a, b) in before.iter().zip(after.iter()) {
        match (a, b) {
            (OverrideToken::Block(x), OverrideToken::Block(y)) => {
                assert_eq!(x.tags.len(), y.tags.len());
                for (t1, t2) in x.tags.iter().zip(y.tags.iter()) {
                    assert_eq!(t1.name, t2.name);
                    assert_eq!(t1.lead, t2.lead);
                    assert_eq!(t1.kind, t2.kind);
                    if !matches!(t1.kind, TagKind::Colour { .. }) {
                        assert_eq!(t1.args, t2.args, "{:?} 不该被改动", t1.name);
                    }
                }
                assert_eq!(x.tail, y.tail);
            }
            _ => assert_eq!(a, b),
        }
    }
}

#[test]
fn gradient_and_bare_hex_and_lowercase_spellings() {
    let out = map_color_tokens("{\\c&H00FFFF&H&HFF00FF&H}", &halve).unwrap();
    assert_eq!(out, "{\\c&H007F7F&H&H7F007F&H}");
    let bare = map_color_tokens("{\\cFFFFFF}", &halve).unwrap();
    assert_eq!(bare, "{\\c7F7F7F}");
    // &h 前缀与十六进制字母大小写：前缀写法保留，十六进制由 core-model 规范化
    let lower = map_color_tokens("{\\c&h00ff00&}", &halve).unwrap();
    assert_eq!(lower, "{\\c&h007F00&}");
    // 未被改动的 token 保持原大小写
    assert_eq!(
        map_color_tokens("{\\c&hAAbBcC&}", &identity).unwrap(),
        "{\\c&hAAbBcC&}"
    );
}

#[test]
fn eight_digit_alpha_channel_is_not_touched_by_default() {
    let text = "{\\1c&HAA112233&}";
    let out = map_color_tokens(text, &halve).unwrap();
    let c = AssColor::parse_ass("&HAA112233").unwrap();
    let expect = AssColor {
        a: c.a,
        r: c.r / 2,
        g: c.g / 2,
        b: c.b / 2,
    };
    assert_eq!(
        out,
        format!("{{\\1c&H{}&}}", expect.to_ass_string().replace("&H", ""))
    );
    assert_eq!(out.matches('&').count(), text.matches('&').count());
}

#[test]
fn colour_inside_nested_animation_is_mapped_and_bytes_elsewhere_hold() {
    let text = "前{\\t(0,700,30,\\c&H0000FF&H)}中{\\anm2\\be3}后";
    let out = map_color_tokens(text, &halve).unwrap();
    assert_eq!(out, "前{\\t(0,700,30,\\c&H00007F&H)}中{\\anm2\\be3}后");
}

#[test]
fn error_from_callback_is_propagated() {
    let boom = |_c: AssColor| -> Result<AssColor, StkError> {
        Err(StkError::simple(Domain::Transform, 9, "映射失败"))
    };
    assert!(map_color_tokens("{\\c&HFFFFFF&}x", &boom).is_err());
    // 没有颜色 token 时不会调用回调，也不会报错
    assert_eq!(map_color_tokens("纯文本", &boom).unwrap(), "纯文本");
    assert!(map_color_tokens("{\\t(0,10,\\c&HFFFFFF&)}", &boom).is_err());
}

#[test]
fn to_ass_like_keeps_original_digit_width() {
    // 复用 core-model 的同宽度写法；codec 侧只按原 digits 长度选择宽度
    let c6 = AssColor::parse_ass("&HFFFFFF").unwrap();
    assert_eq!(c6.to_ass_like("&HFFFFFF"), "&HFFFFFF");
    assert_eq!(c6.to_ass_like("&H00FFFFFF"), "&H00FFFFFF");
    let out = map_color_tokens("{\\c&H00FFFFFF&}", &|c| {
        Ok(AssColor {
            a: c.a,
            r: 1,
            g: 2,
            b: 3,
        })
    })
    .unwrap();
    assert_eq!(out, "{\\c&H00030201&}");
    let out6 = map_color_tokens("{\\c&HFFFFFF&}", &|c| {
        Ok(AssColor {
            a: c.a,
            r: 1,
            g: 2,
            b: 3,
        })
    })
    .unwrap();
    assert_eq!(out6, "{\\c&H030201&}");
}
