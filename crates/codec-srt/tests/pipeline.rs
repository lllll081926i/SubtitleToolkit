//! 走生产 pipeline 的验收测试（31 §13 反例：不允许只测内部小函数）。
//! 全部只用 `codec_srt` 的公开 API：`parse` / `write` / `TextSubtitle::*`。

use core_model::error::Domain;
use core_model::loss::LossReport;
use core_model::time::{Rational, Time};

use codec_srt::{parse, write, DetectedEncoding, Newline, TextCue, TextSubtitle};

fn ms(v: i64) -> Time {
    Time::from_millis(v).expect("测试内合法毫秒")
}

fn utf16le(text: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE];
    for u in text.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v
}

fn utf16be(text: &str) -> Vec<u8> {
    let mut v = vec![0xFE, 0xFF];
    for u in text.encode_utf16() {
        v.extend_from_slice(&u.to_be_bytes());
    }
    v
}

fn has(list: &[String], needle: &str) -> bool {
    list.iter().any(|v| v.contains(needle))
}

fn ok(bytes: &[u8]) -> (TextSubtitle, LossReport) {
    parse(bytes).unwrap_or_else(|e| panic!("parse 应成功，实得 {e}"))
}

const CLEAN_LF: &str =
    "1\n00:00:01,000 --> 00:00:02,000\nHello\n\n2\n00:00:03,500 --> 00:00:04,000\n多行\n第二行\n\n";
const CLEAN_CRLF: &str = "1\r\n00:00:01,000 --> 00:00:02,000\r\nHello\r\n\r\n2\r\n00:00:03,500 --> 00:00:04,000\r\n多行\r\n第二行\r\n\r\n";

#[test]
fn t01_minimal_single_cue() {
    let (sub, loss) = ok(b"1\n00:00:01,000 --> 00:00:02,000\nHello\n");
    assert_eq!(sub.cues.len(), 1);
    let c = &sub.cues[0];
    assert_eq!(
        (c.index, c.start, c.end, c.text.as_str()),
        (Some(1), ms(1000), ms(2000), "Hello")
    );
    assert_eq!(
        (sub.encoding, sub.bom, sub.newline),
        (DetectedEncoding::Utf8, false, Newline::Lf)
    );
    assert!(
        loss.is_lossless(),
        "干净输入不该有损: {:?}",
        loss.nonstandard
    );
}

#[test]
fn t02_three_cues_with_multiline_text() {
    let src = "1\n00:00:01,000 --> 00:00:02,000\nA1\nA2\n\n2\n00:00:03,000 --> 00:00:04,000\nB1\nB2\nB3\n\n3\n00:00:05,000 --> 00:00:06,000\nC\n\n";
    let (sub, _) = ok(src.as_bytes());
    assert_eq!(sub.cues.len(), 3);
    assert_eq!(sub.cues[1].text, "B1\nB2\nB3");
    assert_eq!(sub.cues[0].text, "A1\nA2");
    assert_eq!(sub.cues[2].text, "C");
    assert_eq!(
        sub.cues.iter().filter_map(|c| c.index).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn t03_crlf_style_is_kept() {
    let (sub, loss) = ok(CLEAN_CRLF.as_bytes());
    assert_eq!(sub.newline, Newline::CrLf);
    assert!(!has(&loss.nonstandard, "混用"));
    assert_eq!(sub.cues.len(), 2);
}

#[test]
fn t04_lf_style_is_default_and_kept() {
    let (sub, _) = ok(CLEAN_LF.as_bytes());
    assert_eq!(sub.newline, Newline::Lf);
    assert_eq!(sub.cues[0].text, "Hello");
}

#[test]
fn t05_cr_only_parses_and_normalizes_to_lf() {
    let (sub, loss) = ok(b"1\r00:00:01,000 --> 00:00:02,000\rHello\r");
    assert_eq!(sub.cues.len(), 1);
    assert_eq!(sub.cues[0].text, "Hello");
    assert_eq!(
        sub.newline,
        Newline::Lf,
        "Newline 枚举没有 CR 变体，必须显式归一"
    );
    assert!(has(&loss.nonstandard, "仅 CR"));
}

#[test]
fn t06_mixed_newlines_are_tolerated_and_reported() {
    let (sub, loss) = ok(
        b"1\r\n00:00:01,000 --> 00:00:02,000\nA\r\n\r\n2\r\n00:00:05,000 --> 00:00:06,000\r\nB\n",
    );
    assert_eq!(sub.newline, Newline::CrLf, "首次检测到的风格胜出");
    assert_eq!(sub.cues.len(), 2);
    assert!(has(&loss.nonstandard, "混用"));
}

#[test]
fn t07_utf16le_with_bom() {
    let (sub, loss) = ok(&utf16le(CLEAN_LF));
    assert_eq!(sub.encoding, DetectedEncoding::Utf16Le);
    assert!(sub.bom);
    assert_eq!(sub.cues.len(), 2);
    assert_eq!(sub.cues[0].text, "Hello");
    assert_eq!(sub.cues[1].text, "多行\n第二行");
    assert!(loss.is_lossless());
}

#[test]
fn t08_utf16be_with_bom_and_surrogate_pair() {
    let (sub, _) = ok(&utf16be("1\n00:00:01,000 --> 00:00:02,000\n😀 中文 ok\n\n"));
    assert_eq!(sub.encoding, DetectedEncoding::Utf16Be);
    assert_eq!(sub.cues[0].text, "😀 中文 ok", "手写解码必须正确处理代理对");
}

#[test]
fn t09_utf8_bom_is_detected_and_kept() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"1\n00:00:01,000 --> 00:00:02,000\nHi\n");
    let (sub, loss) = ok(&bytes);
    assert_eq!(sub.encoding, DetectedEncoding::Utf8Bom);
    assert!(sub.bom);
    assert_eq!(sub.cues[0].text, "Hi", "BOM 不能混进文本");
    assert!(loss.is_lossless());
}

#[test]
fn t10_missing_index_yields_none() {
    let (sub, loss) =
        ok(b"00:00:01,000 --> 00:00:02,000\nHello\n\n00:00:03,000 --> 00:00:04,000\nWorld\n");
    assert_eq!(sub.cues.len(), 2);
    assert!(sub.cues.iter().all(|c| c.index.is_none()));
    assert!(has(&loss.nonstandard, "缺少序号"));
}

#[test]
fn t11_non_contiguous_index_is_preserved_and_reported() {
    let (sub, loss) =
        ok(b"1\n00:00:01,000 --> 00:00:02,000\nA\n\n7\n00:00:03,000 --> 00:00:04,000\nB\n");
    assert_eq!(
        sub.cues.iter().filter_map(|c| c.index).collect::<Vec<_>>(),
        vec![1, 7]
    );
    assert!(has(&loss.nonstandard, "序号不连续"));
}

#[test]
fn t12_dot_millisecond_separator_is_tolerated() {
    let (sub, loss) = ok(b"1\n00:00:01.500 --> 00:00:02.250\nA\n");
    assert_eq!((sub.cues[0].start, sub.cues[0].end), (ms(1500), ms(2250)));
    assert!(has(&loss.nonstandard, "毫秒用 . 分隔"));
}

#[test]
fn t13_surplus_whitespace_in_timing_line() {
    let (sub, loss) = ok(b"  1  \n   00:00:01,000   -->   00:00:02,000   \nA\n");
    assert_eq!(sub.cues[0].index, Some(1));
    assert_eq!((sub.cues[0].start, sub.cues[0].end), (ms(1000), ms(2000)));
    assert!(loss.is_lossless());
}

#[test]
fn t14_milliseconds_are_optional_and_read_as_integers() {
    let (sub, loss) = ok(b"1\n00:00:01 --> 00:00:02,05\nA\n");
    assert_eq!((sub.cues[0].start, sub.cues[0].end), (ms(1000), ms(2005)));
    assert!(has(&loss.nonstandard, "缺少毫秒"));
    assert!(has(&loss.nonstandard, "位数不是 3"));
}

#[test]
fn t15_out_of_range_time_fields_keep_block_verbatim() {
    let (sub, loss) = ok(b"1\n00:99:01,000 --> 00:00:02,000\nA\n");
    assert!(sub.cues.is_empty());
    assert_eq!(sub.unrecognized.len(), 1);
    assert_eq!(
        sub.unrecognized[0].raw,
        "1\n00:99:01,000 --> 00:00:02,000\nA"
    );
    assert!(has(&loss.user_attention, "时间字段越界"));
}

#[test]
fn t16_end_not_after_start_records_attention() {
    let (sub, loss) =
        ok(b"1\n00:00:05,000 --> 00:00:02,000\nA\n\n2\n00:00:06,000 --> 00:00:06,000\nB\n");
    assert_eq!(sub.cues.len(), 2, "宽容解析：留内容，不拒绝");
    assert_eq!(sub.cues[0].start, ms(5000));
    assert!(has(&loss.user_attention, "cue 结束不晚于开始"));
    let e = sub.validate().unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Validate, 142));
}

#[test]
fn t17_illegal_utf8_is_byte_preserved() {
    // GBK "中文" = D6 D0 CE C4（首字节不是任何 BOM，走字节保真分支）
    let mut src = b"1\r\n00:00:01,000 --> 00:00:02,000\r\n".to_vec();
    src.extend_from_slice(&[0xD6, 0xD0, 0xCE, 0xC4]);
    src.extend_from_slice(b"\r\n\r\n");
    let (sub, loss) = ok(&src);
    assert_eq!(sub.encoding, DetectedEncoding::Ansi);
    assert!(!sub.bom);
    assert_eq!(
        sub.cues[0].text.chars().map(u32::from).collect::<Vec<_>>(),
        vec![0xD6, 0xD0, 0xCE, 0xC4]
    );
    assert!(has(&loss.user_attention, "需外部编码转换"));
    assert_eq!(
        write(&sub).expect("写出应成功"),
        src,
        "非法 UTF-8 必须逐字节还原"
    );
}

#[test]
fn t18_illegal_utf8_never_injects_replacement_char() {
    let (sub, _) = ok(b"1\n00:00:01,000 --> 00:00:02,000\n\xFF\xFE\x27\n");
    assert!(
        !sub.cues[0].text.contains('\u{FFFD}'),
        "不许用替换字符静默改掉内容"
    );
    let out = write(&sub).expect("写出应成功");
    assert!(
        !out.windows(3).any(|w| w == [0xEF, 0xBF, 0xBD]),
        "输出里不该出现 U+FFFD 的 UTF-8 序列"
    );
    assert!(out.contains(&0xFF));
}

#[test]
fn t19_odd_length_utf16_is_error_not_panic() {
    let e = parse(&[0xFF, 0xFE, b'1', 0x00, b'\n']).expect_err("奇数字节必须报错");
    assert_eq!((e.domain, e.code), (Domain::Parse, 121));
    assert!(e.user_action.is_some(), "错误要带可执行动作");
}

#[test]
fn t20_unpaired_surrogate_is_error_not_panic() {
    let e = parse(&[0xFF, 0xFE, 0x00, 0xD8]).expect_err("未配对高位代理必须报错");
    assert_eq!((e.domain, e.code), (Domain::Parse, 122));
    let e2 = parse(&[0xFE, 0xFF, 0xDC, 0x00]).expect_err("未配对低位代理同样要报错");
    assert_eq!((e2.domain, e2.code), (Domain::Parse, 122));
}

#[test]
fn t21_utf8_bom_followed_by_invalid_bytes_is_error() {
    let e = parse(&[0xEF, 0xBB, 0xBF, 0x31, 0x0A, 0xFF, 0xFF])
        .expect_err("BOM 声明了 UTF-8，非法序列不能降级成 Ansi");
    assert_eq!((e.domain, e.code), (Domain::Parse, 120));
}

#[test]
fn t22_block_without_arrow_is_kept_verbatim() {
    let (mut sub, loss) = ok("1\n00:00:01,000 --> 00:00:02,000\nHello\n\n完全不是字幕的一行\n第二行\n\n3\n00:00:09,000 --> 00:00:10,000\nWorld\n".as_bytes());
    assert_eq!(sub.cues.len(), 2);
    assert_eq!(sub.unrecognized.len(), 1);
    assert_eq!(sub.unrecognized[0].raw, "完全不是字幕的一行\n第二行");
    assert_eq!(sub.unrecognized[0].after_cues, 1);
    assert!(has(&loss.user_attention, "未识别块"));
    sub.renumber();
    let out = write(&sub).expect("写出应成功");
    let (again, _) = ok(&out);
    assert_eq!(
        again.unrecognized, sub.unrecognized,
        "未识别块必须能原样读回"
    );
    assert_eq!(again.cues, sub.cues);
    assert_eq!(write(&again).expect("再次写出应稳定"), out);
}

#[test]
fn t23_junk_before_timing_line_is_kept_verbatim() {
    let (sub, _) = ok(
        "1\n00:00:01,000 --> 00:00:02,000\nA\n\n标题行\n2\n00:00:03,000 --> 00:00:04,000\nB\n"
            .as_bytes(),
    );
    assert_eq!(sub.cues.len(), 1);
    assert_eq!(
        sub.unrecognized[0].raw,
        "标题行\n2\n00:00:03,000 --> 00:00:04,000\nB"
    );
    assert_eq!(sub.unrecognized[0].after_cues, 1);
}

#[test]
fn t24_second_arrow_line_stays_as_text() {
    let (sub, loss) = ok(b"1\n00:00:01,000 --> 00:00:02,000\na --> b\n");
    assert_eq!(sub.cues.len(), 1);
    assert_eq!(sub.cues[0].text, "a --> b");
    assert!(has(&loss.nonstandard, "再次出现 -->"));
}

#[test]
fn t25_empty_input_is_not_an_error() {
    let (sub, loss) = ok(&[]);
    assert!(sub.cues.is_empty());
    assert_eq!(
        (sub.encoding, sub.bom, sub.newline),
        (DetectedEncoding::Utf8, false, Newline::Lf)
    );
    assert!(has(&loss.user_attention, "未解析到任何有效 cue"));
    assert_eq!(write(&sub).expect("空字幕也能写出"), Vec::<u8>::new());
}

#[test]
fn t26_whitespace_only_input() {
    let (sub, loss) = ok(b"\n  \n\t\n\n");
    assert!(sub.cues.is_empty() && sub.unrecognized.is_empty());
    assert!(has(&loss.nonstandard, "只含空白字符的行"));
    assert!(sub.validate().is_err());
}

#[test]
fn t27_extra_blank_lines_and_missing_trailing_newline() {
    let (sub, _) =
        ok(b"\n\n1\n00:00:01,000 --> 00:00:02,000\nA\n\n\n\n2\n00:00:03,000 --> 00:00:04,000\nB");
    assert_eq!(sub.cues.len(), 2);
    assert_eq!(sub.cues[1].text, "B");
}

#[test]
fn t28_clean_input_roundtrips_byte_identical() {
    for src in [CLEAN_LF, CLEAN_CRLF] {
        let (sub, _) = ok(src.as_bytes());
        let once = write(&sub).expect("写出应成功");
        assert_eq!(once, src.as_bytes(), "规范输入必须字节级等价");
        let (sub2, _) = ok(&once);
        assert_eq!(
            write(&sub2).expect("再写出"),
            once,
            "write∘parse 必须是不动点"
        );
        assert_eq!(sub2, sub);
    }
}

#[test]
fn t29_write_renumbers_and_still_reads_back() {
    let (mut sub, _) =
        ok(b"7\n00:00:01,000 --> 00:00:02,000\nA\n\n\n00:00:03,000 --> 00:00:04,000\nB\n");
    sub.renumber();
    assert_eq!(
        sub.cues.iter().filter_map(|c| c.index).collect::<Vec<_>>(),
        vec![1, 2]
    );
    let out = write(&sub).expect("写出应成功");
    assert!(
        out.starts_with(b"1\n00:00:01,000 --> 00:00:02,000\nA\n\n2\n"),
        "{}",
        String::from_utf8_lossy(&out)
    );
    let (back, _) = ok(&out);
    assert_eq!(back.cues[1].index, Some(2));
    assert!(back.validate().is_ok());
}

#[test]
fn t30_write_emits_utf8_bom_only_when_declared() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"1\n00:00:01,000 --> 00:00:02,000\nHi\n");
    let (sub, _) = ok(&bytes);
    let out = write(&sub).expect("写出应成功");
    assert!(out.starts_with(&[0xEF, 0xBB, 0xBF]));
    let (back, _) = ok(&out);
    assert_eq!(back.encoding, DetectedEncoding::Utf8Bom);
    assert_eq!(back.cues, sub.cues);
    let plain = TextSubtitle {
        bom: false,
        ..sub.clone()
    };
    assert!(!write(&plain)
        .expect("无 BOM 写出")
        .starts_with(&[0xEF, 0xBB, 0xBF]));
}

#[test]
fn t31_utf16_input_writes_back_as_utf8_semantically_equal() {
    let (sub, _) = ok(&utf16le(CLEAN_LF));
    let out = write(&sub).expect("写出应成功");
    assert!(
        out.starts_with(&[0xEF, 0xBB, 0xBF]),
        "bom=true 时补 UTF-8 BOM"
    );
    let (back, _) = ok(&out);
    assert_eq!(
        back.encoding,
        DetectedEncoding::Utf8Bom,
        "writer 只产 UTF-8（已登记为规范疑点）"
    );
    assert_eq!(back.cues, sub.cues);
    assert_eq!(back.newline, Newline::Lf);
}

#[test]
fn t32_ansi_edit_outside_latin1_is_refused_not_mangled() {
    let (mut sub, _) = ok(b"1\n00:00:01,000 --> 00:00:02,000\n\xC4\xE5\n");
    sub.cues[0].text.push('中');
    let e = write(&sub).expect_err("字节保真不成立时必须报错");
    assert_eq!((e.domain, e.code), (Domain::Output, 132));
    assert!(e.user_action.is_some());
}

#[test]
fn t33_write_refuses_whitespace_only_line_and_negative_time() {
    let mut sub = TextSubtitle::default();
    sub.cues
        .push(TextCue::new(Some(1), ms(1000), ms(2000), "a\n \nb"));
    let e = write(&sub).expect_err("空白行会被重解析成块分隔符");
    assert_eq!((e.domain, e.code), (Domain::Output, 131));

    sub.cues[0].text = "ok".into();
    sub.cues[0].start = ms(-5);
    let e = write(&sub).expect_err("负时间必须被拒绝");
    assert_eq!((e.domain, e.code), (Domain::Output, 3));
    assert_eq!(e.code_string(), "STK-OUT-0003");
}

#[test]
fn t34_write_is_refused_for_ansi_plus_bom() {
    let mut sub = TextSubtitle {
        encoding: DetectedEncoding::Ansi,
        bom: true,
        ..Default::default()
    };
    sub.cues.push(TextCue::new(None, ms(0), ms(10), "x"));
    assert_eq!(write(&sub).expect_err("矛盾配置要报错").code, 133);
    assert_eq!(sub.validate().expect_err("validate 也要挡住").code, 147);
}

#[test]
fn t35_validate_rules_on_equal_and_touching_times() {
    let mut sub = TextSubtitle::default();
    sub.cues
        .push(TextCue::new(Some(1), ms(1000), ms(2000), "A"));
    sub.cues
        .push(TextCue::new(Some(2), ms(2000), ms(3000), "B"));
    assert!(sub.validate().is_ok(), "首尾相接合法");
    sub.cues[1].start = ms(1000);
    sub.cues[1].end = ms(1500);
    assert!(sub.validate().is_ok(), "同一起点（forced + 常规并存）允许");
    sub.cues[0].start = ms(2000);
    sub.cues[0].end = ms(3000);
    sub.cues[1].start = ms(1000);
    sub.cues[1].end = ms(1500);
    assert_eq!(sub.validate().expect_err("真正的倒序要报错").code, 143);
    sub.cues[1].start = ms(3000);
    assert_eq!(sub.validate().expect_err("零时长要报错").code, 142);
}

#[test]
fn t36_validate_rejects_empty_text_and_bad_indices() {
    let mut sub = TextSubtitle::default();
    sub.cues.push(TextCue::new(Some(0), ms(0), ms(10), "   "));
    assert_eq!(sub.validate().expect_err("空白文本要报错").code, 144);
    sub.cues[0].text = "x".into();
    assert_eq!(sub.validate().expect_err("序号从 1 开始").code, 145);
    sub.cues[0].index = Some(3);
    sub.cues.push(TextCue::new(Some(3), ms(20), ms(30), "y"));
    assert_eq!(sub.validate().expect_err("序号必须严格递增").code, 146);
    let empty = TextSubtitle::default();
    assert_eq!(empty.validate().expect_err("空字幕非法").code, 140);
}

#[test]
fn t37_zero_based_file_is_fixed_by_renumber() {
    let (mut sub, _) =
        ok(b"0\n00:00:01,000 --> 00:00:02,000\nA\n\n1\n00:00:03,000 --> 00:00:04,000\nB\n");
    assert_eq!(sub.cues[0].index, Some(0));
    assert_eq!(sub.validate().expect_err("0 序号不合法").code, 145);
    sub.renumber();
    assert!(sub.validate().is_ok());
}

#[test]
fn t38_shift_moves_every_boundary_and_roundtrips() {
    let (mut sub, _) = ok(CLEAN_LF.as_bytes());
    sub.shift(1000).expect("正偏移应成功");
    assert_eq!((sub.cues[0].start, sub.cues[1].end), (ms(2000), ms(5000)));
    sub.shift(-500).expect("负偏移仍有非负时间");
    assert_eq!(sub.cues[0].start, ms(1500));
    assert!(sub.validate().is_ok());
    let out = write(&sub).expect("写出应成功");
    let (back, _) = ok(&out);
    assert_eq!(back.cues, sub.cues);
}

#[test]
fn t39_shift_is_all_or_nothing_and_bounds_checked() {
    let (mut sub, _) = ok(CLEAN_LF.as_bytes());
    let before = sub.clone();
    let e = sub.shift(-1500).expect_err("会把第一条推到负数");
    assert_eq!(e.domain, Domain::Transform);
    assert_eq!(sub, before, "不许留下改了一半的 IR");
    assert_eq!(
        sub.shift(i64::MIN).expect_err("极值不得 panic").domain,
        Domain::Input
    );
    assert!(sub.shift(i64::MAX).is_err(), "溢出必须报错而不是回绕");
    sub.shift(-1000).expect("刚好把首条推到 0");
    assert_eq!(sub.cues[0].start, Time::ZERO);
}

#[test]
fn t40_times_stay_exact_rationals() {
    let (sub, _) = ok(b"1\n01:02:03,456 --> 01:02:04,000\nA\n");
    let c = &sub.cues[0];
    assert_eq!(
        c.start,
        Time::from_hms_millis(1, 2, 3, 456).expect("合法时间")
    );
    assert_eq!(
        c.start.as_rational(),
        Rational::new(3_723_456, 1000).expect("可约分")
    );
    assert_eq!(
        c.end.sub(&c.start).expect("差值"),
        Time::from_millis(544).expect("544ms")
    );
    assert_eq!(c.start.to_srt_string().expect("格式化"), "01:02:03,456");
}

#[test]
fn t41_full_pipeline_shift_then_verify_then_shift_back() {
    let (orig, loss) = ok(CLEAN_CRLF.as_bytes());
    assert!(loss.is_lossless());
    let mut sub = orig.clone();
    sub.shift(250).expect("偏移");
    sub.validate().expect("合法性");
    let bytes = write(&sub).expect("写出");
    let (back, _) = ok(&bytes);
    assert_eq!(back.newline, Newline::CrLf);
    assert_eq!(back.cues, sub.cues, "写出→读回必须完全一致");
    assert_eq!(back.cues[0].start, ms(1250));
    let mut undone = back.clone();
    undone.shift(-250).expect("反向偏移");
    assert_eq!(undone.cues, orig.cues, "有理数偏移必须可逆，不许有浮点残差");
}

#[test]
fn t42_blank_line_inside_text_is_refused_by_writer() {
    let (sub, _) = ok(b"1\n00:00:01,000 --> 00:00:02,000\nA\n");
    let mut edited = sub.clone();
    edited.cues[0].text = String::new();
    assert_eq!(write(&edited).expect_err("空文本写出去就消失").code, 131);
    assert_eq!(edited.validate().expect_err("空文本非法").code, 144);
    assert!(sub.validate().is_ok());
}

#[test]
fn t43_multibyte_text_and_emoji_survive_utf16_roundtrip() {
    let src = "1\n00:00:01,000 --> 00:00:02,000\n字幕 😀 😀 测试\n\n";
    for bytes in [utf16le(src), utf16be(src)] {
        let (sub, _) = ok(&bytes);
        assert_eq!(sub.cues[0].text.chars().count(), 9, "代理对不能按字节计数");
        let out = write(&sub).expect("写出");
        let (back, _) = ok(&out);
        assert_eq!(back.cues, sub.cues);
        assert_eq!(back.cues[0].text, "字幕 😀 😀 测试");
    }
}

fn xorshift(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

#[test]
fn t44_deterministic_fuzz_never_panics_and_write_is_a_fixed_point() {
    // 24 §7 把 SRT parser 列入 fuzz 名单；这里先落一个零依赖、可复现的最小版本。
    // 字母表刻意包含 BOM 字节、CR/LF/CRLF、`-->` 片段、非 ASCII 字节与 NUL。
    let alphabet: &[u8] = b"019:,.-\0> \n\raZ\t\xFF\xFE\xEF\xBB\xBF\xD6\xE4";
    let mut state: u64 = 0x243F_6A88_85A3_08D3;
    let alpha = u64::try_from(alphabet.len()).expect("字母表长度为正");
    for case in 0..4000u64 {
        state = xorshift(state);
        let len = (state % 96) as usize;
        let mut bytes: Vec<u8> = Vec::with_capacity(len);
        for _ in 0..len {
            state = xorshift(state);
            bytes.push(alphabet[(state % alpha) as usize]);
        }
        let sub = match parse(&bytes) {
            Ok((sub, _)) => sub,
            Err(e) => {
                assert_eq!(
                    e.domain,
                    Domain::Parse,
                    "只有解码层错误允许出现（case {case}）: {e}"
                );
                continue;
            }
        };
        let out = write(&sub).unwrap_or_else(|e| {
            panic!(
                "case {case} 解析成功后写出失败: {e}｜{:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
        let (back, _) = ok(&out);
        assert_eq!(
            back.cues,
            sub.cues,
            "case {case}: cue 在读回后变了｜{:?}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(back.unrecognized, sub.unrecognized, "case {case}");
        assert_eq!(
            write(&back).expect("再写出"),
            out,
            "case {case}: write∘parse 不是不动点"
        );
    }
}

#[test]
fn t45_body_starting_with_a_bom_signature_is_escaped_not_misread() {
    // 前导空行 + 以 FF FE 开头的字节保真内容：writer 若把空行丢掉，输出首字节就变成 UTF-16LE BOM，
    // 重读会把整份文件判成另一种编码。必须补一个无语义空行消歧（t44 fuzz 查出的真实缺陷）。
    let mut src = vec![b'\n'];
    src.extend_from_slice(&[0xFF, 0xFE, b'x', b'\n', b'\n']);
    let (sub, _) = ok(&src);
    assert_eq!(sub.encoding, DetectedEncoding::Ansi);
    assert_eq!(sub.unrecognized[0].raw, "\u{FF}\u{FE}x");
    let out = write(&sub).expect("写出");
    assert_eq!(out, src, "字节保真：连消歧用的前导空行也该回来");
    let (back, _) = ok(&out);
    assert_eq!(back.unrecognized, sub.unrecognized);
    assert_eq!(write(&back).expect("再写出"), out);
}
