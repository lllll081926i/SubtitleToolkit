//! 最重要的验收组：`parse → to_string` 必须与原输入逐字节相等（AT-ASS-001）。

use codec_ass::AssDocument;

const SIMPLE: &str = include_str!("fixtures/simple.ass");
const FULL22: &str = include_str!("fixtures/full22.ass");
const UNKNOWN_TAGS: &str = include_str!("fixtures/unknown_tags.ass");
const COMMENTS: &str = include_str!("fixtures/comments_blanks.ass");
const FONTS: &str = include_str!("fixtures/fonts.ass");
const LEGACY: &str = include_str!("fixtures/legacy_no_format.ass");
const BROKEN: &str = include_str!("fixtures/broken_lines.ass");
const CRLF: &str = include_str!("fixtures/crlf.ass");
const MIXED: &str = include_str!("fixtures/mixed_eol.ass");

/// 失败时报出第一个差异字节偏移，而不是打印两个长字符串。
fn assert_byte_equal(name: &str, src: &str) {
    let back = AssDocument::parse(src).to_string();
    let a = src.as_bytes();
    let b = back.as_bytes();
    for i in 0..a.len().min(b.len()) {
        if a[i] != b[i] {
            panic!(
                "{name}: 第 {i} 字节不一致（输入 {:02X}，输出 {:02X}）｜输入行={}｜输出行={}",
                a[i],
                b[i],
                line_of(src, i),
                line_of(&back, i)
            );
        }
    }
    assert_eq!(
        a.len(),
        b.len(),
        "{name}: 长度不一致 输入={} 输出={}",
        a.len(),
        b.len()
    );
}

fn line_of(src: &str, byte: usize) -> String {
    let mut at = byte.min(src.len());
    while !src.is_char_boundary(at) {
        at -= 1;
    }
    let line_start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = src[line_start..]
        .find(['\n', '\r'])
        .map(|i| line_start + i)
        .unwrap_or(src.len());
    src[line_start..line_end].to_owned()
}

fn corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        ("simple.ass", SIMPLE),
        ("full22.ass", FULL22),
        ("unknown_tags.ass", UNKNOWN_TAGS),
        ("comments_blanks.ass", COMMENTS),
        ("fonts.ass", FONTS),
        ("legacy_no_format.ass", LEGACY),
        ("broken_lines.ass", BROKEN),
        ("crlf.ass", CRLF),
        ("mixed_eol.ass", MIXED),
    ]
}

#[test]
fn every_fixture_round_trips_byte_for_byte() {
    let cases = corpus();
    assert!(cases.len() >= 8, "fixture 数量不足");
    for (name, src) in &cases {
        assert_byte_equal(name, src);
    }
}

#[test]
fn crlf_fixture_really_uses_crlf() {
    // 若 fixture 被工具改成了 LF，这组测试就失去意义，所以先自证输入形态
    let b = CRLF.as_bytes();
    assert!(CRLF.contains("\r\n"));
    // 每个 LF 前面都是 CR，每个 CR 后面都是 LF —— 没有孤立行尾
    assert!(b
        .iter()
        .enumerate()
        .all(|(i, c)| *c != b'\n' || i > 0 && b[i - 1] == b'\r'));
    assert!(b
        .iter()
        .enumerate()
        .all(|(i, c)| *c != b'\r' || i + 1 < b.len() && b[i + 1] == b'\n'));
    assert_eq!(CRLF.matches("\r\n").count(), FULL22.matches('\n').count());
    assert!(MIXED.starts_with('\u{feff}'));
    assert!(MIXED.contains("\r\n") && MIXED.contains("\n") && MIXED.ends_with('\r'));
    assert!(!SIMPLE.contains('\r'));
    assert!(BROKEN.ends_with('\n'));
}

#[test]
fn generated_line_ending_variants_round_trip() {
    let lf_to_crlf = SIMPLE.replace('\n', "\r\n");
    assert_byte_equal("simple→crlf", &lf_to_crlf);
    let lf_to_cr = SIMPLE.replace('\n', "\r");
    assert_byte_equal("simple→cr", &lf_to_cr);
    assert_byte_equal("bom+simple", &format!("\u{feff}{SIMPLE}"));
    let mut no_final_newline = SIMPLE.to_owned();
    no_final_newline.pop();
    assert_byte_equal("no final newline", &no_final_newline);
    assert_byte_equal("empty", "");
    assert_byte_equal("only newline", "\n");
    assert_byte_equal("only crlf", "\r\n");
    assert_byte_equal("lone bracket", "[\n");
    assert_byte_equal(
        "no section header",
        "Dialogue: 0,0:00:00.00,0:00:00.01,,,,,,x\n",
    );
}

#[test]
fn pathological_inputs_round_trip_without_panic() {
    // fuzz 前提（23 §ASS）：任意字节都不得 panic，且必须原文还原
    let cases = [
        "[[[[[\n]]]]]\n",
        ": :: :::\n",
        "Style:\n",
        "Style: ,,,,\n",
        "Format: ,,,,\n",
        "Dialogue: \n",
        "Comment:, , , \n",
        "[Fonts]\nfontname\n\n\n",
        "\u{feff}\u{feff}[Script Info]\n",
        "key\u{0}value: 1\n",
        "🙂: 🙂,🙂\n",
        "A: B, C, D\nA: E\n",
        "[V4+ Styles]\r\rStyle: a,b\r",
        "PlayResX:   1280   \n",
        "Title: trailing space \ttab\t\n",
    ];
    for src in cases {
        assert_byte_equal("pathological", src);
    }
}

#[test]
fn synthesized_corpus_round_trips() {
    // 组合式输入：段头大小写/写法多样、未知段、无 Format、乱序事件混在一起
    let parts = [
        "[script info]\n",
        "[Script Info (info)]\n",
        "[V4++ Styles]\n",
        "[v4.00 Styles]\n",
        "[Events]\n",
        "[DOCUMENT EVENTS]\n",
        "[Fonts]\n",
        "[FileList]\n",
        "[Whatever]\n",
        "[\n",
        "]\n",
        "",
    ];
    let lines = [
        "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,text\n",
        "Style: A,B,1\n",
        "; c\n",
        "\n",
        "Key: value\n",
        "garbage line\n",
        "Attachment: x.bin\n",
    ];
    for (pi, p) in parts.iter().enumerate() {
        for (li, l) in lines.iter().enumerate() {
            let src = format!("{p}{l}{l}");
            assert_byte_equal(&format!("synth {pi}/{li}"), &src);
            let crlf = src.replace('\n', "\r\n");
            assert_byte_equal(&format!("synth crlf {pi}/{li}"), &crlf);
        }
    }
}
