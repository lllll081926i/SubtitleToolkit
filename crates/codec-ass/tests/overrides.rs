//! override tokenizer：字节还原 + 已知 tag 结构 + 未知 tag 不丢（AT-ASS-001）。

use codec_ass::overrides::TagKind;
use codec_ass::{parse_overrides, tokens_to_string, OverrideToken, Tag};

fn roundtrip(text: &str) {
    let tokens = parse_overrides(text);
    let back = tokens_to_string(&tokens);
    assert_eq!(back, text, "tokenize 后必须逐字节还原：{text:?}");
}

fn tags_of(text: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    for token in parse_overrides(text) {
        if let OverrideToken::Block(b) = token {
            out.extend(b.tags.iter().cloned());
        }
    }
    out
}

#[test]
fn plain_text_without_braces_is_one_span() {
    let tokens = parse_overrides("hello world");
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens.first(),
        Some(&OverrideToken::Text {
            raw: "hello world".to_owned()
        })
    );
    roundtrip("hello world");
    roundtrip("");
}

#[test]
fn pos_tokenizes_and_restores() {
    let text = "{\\pos(320,480)}字幕";
    let tags = tags_of(text);
    assert_eq!(tags.len(), 1);
    let tag = tags.first().expect("\\pos");
    assert_eq!(tag.kind, TagKind::Position);
    assert_eq!(tag.name, "\\pos");
    assert_eq!(tag.args, "(320,480)");
    assert_eq!(tag.coords(), vec!["320".to_owned(), "480".to_owned()]);
    roundtrip(text);
    roundtrip("{ \\pos ( 320 , 480 ) } spaces");
}

#[test]
fn colour_tags_are_recognised_in_every_spelling() {
    for text in [
        "{\\c&H00FFFFFF&}",
        "{\\1c&H00FFFFFF&H}",
        "{\\2c&H123456&\\3c&H12345678&H}",
        "{\\4c&H00FF00&}",
        "{\\c&h00ff00&}",
        "{\\cFFFFFF}",
        "{\\c&H00FFFF&H&HFF00FF&H}",
    ] {
        roundtrip(text);
        let tags = tags_of(text);
        assert!(matches!(
            tags.first().expect("colour tag").kind,
            TagKind::Colour { .. }
        ));
        let slots: Vec<u8> = tags
            .iter()
            .filter_map(|t| match t.kind {
                TagKind::Colour { slot } => Some(slot),
                _ => None,
            })
            .collect();
        assert!(!slots.is_empty());
    }
    let slots: Vec<u8> = tags_of("{\\1c&H0&\\2c&H0&\\3c&H0&\\4c&H0&}")
        .iter()
        .filter_map(|t| match t.kind {
            TagKind::Colour { slot } => Some(slot),
            _ => None,
        })
        .collect();
    assert_eq!(slots, vec![1, 2, 3, 4]);
}

#[test]
fn alpha_tags_are_recognised_and_never_treated_as_colour() {
    let text = "{\\alpha&H80&\\1a&HFF&\\4a&H00&}";
    let tags = tags_of(text);
    assert!(
        tags.iter().all(|t| matches!(t.kind, TagKind::Alpha { .. })),
        "{:?}",
        tags
    );
    roundtrip(text);
    // `\anm` 不是 `\a`+`nm`
    let weird = tags_of("{\\anm2}");
    assert_eq!(weird.len(), 1);
    assert_eq!(weird.first().expect("anm").kind, TagKind::Unknown);
}

#[test]
fn geometry_tags_are_recognised() {
    let text = "{\\move(10,20,30,40,100,900)\\org(640,360)\\clip(0,0,640,360)\\iclip(1,2,3,4)}";
    let kinds: Vec<TagKind> = tags_of(text).iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            TagKind::Movement,
            TagKind::Origin,
            TagKind::Clip { inverted: false },
            TagKind::Clip { inverted: true },
        ]
    );
    roundtrip(text);
    assert_eq!(tags_of(text)[0].coords().len(), 6);
}

#[test]
fn bord_shad_scale_rotation_font_tags() {
    let text = "{\\bord4\\xbord3\\ybord5\\shad2\\xshad1\\yshad6\\fscx150\\fscy90\\frz12\\frx3\\fry7\\fnConsolas}";
    let tags = tags_of(text);
    let kinds: Vec<TagKind> = tags.iter().map(|t| t.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            TagKind::Border,
            TagKind::Border,
            TagKind::Border,
            TagKind::Shadow,
            TagKind::Shadow,
            TagKind::Shadow,
            TagKind::Scale,
            TagKind::Scale,
            TagKind::Rotation,
            TagKind::Rotation,
            TagKind::Rotation,
            TagKind::Font,
        ]
    );
    assert_eq!(tags.last().expect("fn").args, "Consolas");
    roundtrip(text);
    roundtrip("{\\fn Arial Black\\bord 3.5 }");
}

#[test]
fn drawing_mode_keeps_inner_spaces_and_marks_the_span() {
    let text = "{\\p1}m 4 12 l 30 40 l 90 12 l 90 100{\\p0} with spaces";
    let tokens = parse_overrides(text);
    assert_eq!(tokens.len(), 4);
    assert_eq!(
        tokens[1],
        OverrideToken::Drawing {
            raw: "m 4 12 l 30 40 l 90 12 l 90 100".to_owned()
        }
    );
    assert_eq!(
        tokens[3],
        OverrideToken::Text {
            raw: " with spaces".to_owned()
        }
    );
    roundtrip(text);
    // clip 内的矢量参数不能被空格切开
    let nested = "{\\iclip(\\p1 m 0 0 l 100 0 l 100 100\\p0)}x";
    let tags = tags_of(nested);
    assert_eq!(tags.len(), 1);
    assert_eq!(
        tags.first().expect("iclip").args,
        "(\\p1 m 0 0 l 100 0 l 100 100\\p0)"
    );
    roundtrip(nested);
}

#[test]
fn nested_animation_is_tokenized_recursively() {
    let text = "{\\t(0,700,30,\\c&H0000FF&H)}";
    let tags = tags_of(text);
    assert_eq!(tags.len(), 1);
    let anim = &tags.first().expect("\\t").kind;
    match anim {
        TagKind::Animation { head, nested, tail } => {
            assert_eq!(head, "(0,700,30,");
            assert_eq!(tail, ")");
            assert_eq!(nested.len(), 1);
            assert_eq!(
                nested.first().expect("inner colour").kind,
                TagKind::Colour { slot: 1 }
            );
        }
        other => panic!("期望 Animation，实际 {other:?}"),
    }
    roundtrip(text);
    // 双层嵌套 + 内部多 tag
    let deep = "{\\t(0,1000,\\t(0,500,\\c&HFF&H)\\bord2)}";
    roundtrip(deep);
    let outer = tags_of(deep);
    if let TagKind::Animation { nested, .. } = &outer.first().expect("outer").kind {
        if let TagKind::Animation { nested: inner, .. } = &nested.first().expect("inner \\t").kind {
            assert_eq!(
                inner.first().expect("\\c").kind,
                TagKind::Colour { slot: 1 }
            );
        } else {
            panic!("内层不是 Animation");
        }
    } else {
        panic!("外层不是 Animation");
    }
    roundtrip("{\\t(50,100,30,\\fscx115\\fscy115\\c&H3200FF&H)\\t(100,150,\\fscx100\\fscy100)}尾");
    roundtrip("{\\t(0,1000)}没有内层 tag");
    roundtrip("{\\t(0,1000,\\anm5)}未知 tag 也进 nested");
}

#[test]
fn unknown_tags_keep_name_args_and_bytes() {
    let tags = tags_of("{\\anm fade-in\\be 12\\zzz weird}x");
    let unknown: Vec<&Tag> = tags.iter().filter(|t| t.is_unknown()).collect();
    assert_eq!(unknown.len(), 2, "{tags:?}");
    assert_eq!(unknown.first().expect("anm").name, "\\anm");
    assert_eq!(unknown.first().expect("anm").args, " fade-in");
    assert_eq!(unknown.last().expect("zzz").name, "\\zzz");
    assert_eq!(unknown.last().expect("zzz").args, " weird");
    roundtrip("{\\anm fade-in\\be 12\\zzz weird}x");
    // 完全没见过的写法也必须能原样吐回去
    for text in [
        "{\\}",
        "{\\}",
        "{\\ )}",
        "{\\*x1}",
        "{\\123456}",
        "{\\fn}",
        "{\\iclip}",
        "{\\xanm2\\9z}",
    ] {
        roundtrip(text);
    }
}

#[test]
fn malformed_braces_round_trip() {
    for text in [
        "{\\c&HFFFFFF& unterminated",
        "} stray close {\\c&H0&}",
        "{{\\c&H0&}}",
        "{\\clip(0,0,100}",
        "a{\\b1}b{\\i1}c",
        "{}",
        "{ }",
        "{\\t(0,1}\\c&H00&)",
        "\\",
        "\\",
        "\\N换行不在 block 内",
        "换行 \\an5 反斜杠在文本里",
    ] {
        roundtrip(text);
    }
}

#[test]
fn karaoke_and_misc_tags_round_trip() {
    roundtrip("{\\k37}ka{\\kf24}ra{\\ko11}o{\\kt9\\an5}ke{\\q2\\h\\fe0}x{\\r}{\\i1\\b-1\\s1\\u1\\an7\\fs20}");
    let tags = tags_of("{\\k37}");
    assert_eq!(tags.first().expect("k").kind, TagKind::Karaoke);
    assert_eq!(tags.first().expect("k").args, "37");
}

#[test]
fn every_fixture_line_round_trips_through_the_tokenizer() {
    let src = include_str!("fixtures/unknown_tags.ass");
    let doc = codec_ass::AssDocument::parse(src);
    let mut seen = 0usize;
    for d in doc.dialogues() {
        roundtrip(d.text());
        seen += 1;
    }
    assert!(seen >= 10, "corpus 覆盖不足: {seen}");
    for style in doc.styles() {
        let _ = style.raw();
    }
}
