//! 时间码：单一 rational frame clock 的验收（10 §7、13 §7、AT-BDN-001）。
//!
//! 本文件是旧工程缺陷一（"时间码秒字段与帧字段用不同舍入源"）的反例集合。

use codec_bdn::{BdnDocument, BdnEvent, BdnGraphic};
use core_model::error::Domain;
use core_model::time::{BdnTimecode, Fps, Rational, Time};

mod common;
use common::{abs, meta};

const FPS23976: Fps = Fps::NTSC_23976;

/// 1/23.976 秒，精确有理数（不是 0.0417…）。
fn frame_len() -> Rational {
    Rational::new(1001, 24000).unwrap()
}

fn t_ms(ms: i64) -> Time {
    Time::from_millis(ms).unwrap()
}

fn tc_of(t: Time) -> String {
    t.to_bdn_timecode(FPS23976).unwrap().to_string()
}

fn graphic(i: usize) -> BdnGraphic {
    BdnGraphic::new(format!("{i:05}.png"), 800, 120, 560, 880).unwrap()
}

#[test]
fn wall_clock_one_second_is_frame_23_at_23976() {
    // BDN 时间码是帧号（10 §7 + ass2bdnxml `mk_timecode`）：trunc(1.000s × 23.976) = 23
    assert_eq!(tc_of(Time::from_secs(1)), "00:00:00:23");
    // 而 "00:00:01:00" 是第 24 帧，也就是 1.001 秒
    assert_eq!(tc_of(t_ms(1001)), "00:00:01:00");
}

#[test]
fn three_and_a_half_seconds_is_frame_11() {
    assert_eq!(tc_of(t_ms(3500)), "00:00:03:11");
}

#[test]
fn frame_172800_is_02_00_00_00_and_roundtrips_exactly() {
    // "02:00:00:00" = 第 172800 帧 = 172800 / 23.976 秒 = 7207.2 秒墙上时间，
    // 不是墙上 2 小时；把它当成墙上 2 小时正是"双舍入源"的表现。
    let tc = BdnTimecode::from_frame_index(172_800, 24).unwrap();
    assert_eq!(tc.to_string(), "02:00:00:00");
    let t = tc.to_time(FPS23976).unwrap();
    assert_eq!(t, t_ms(7_207_200));
    assert_eq!(t.to_bdn_timecode(FPS23976).unwrap(), tc);
    assert_eq!(BdnTimecode::parse("02:00:00:00").unwrap(), tc);
}

#[test]
fn frame_field_is_never_printed_as_nominal() {
    // 量化是 trunc(t × fps)，FF 自然落在 [0, nominal)：不存在"FF==24 需要进位"的第二套舍入。
    assert_eq!(tc_of(t_ms(1990)), "00:00:01:23");
    assert_eq!(tc_of(t_ms(1958)), "00:00:01:22");
    assert_eq!(tc_of(t_ms(2002)), "00:00:02:00");
    assert!(BdnTimecode::parse("00:00:01:24")
        .unwrap()
        .to_time(FPS23976)
        .is_err());
}

/// 缺陷一反例：2 小时逐帧时间码，反算与真实秒数误差必须小于 1 帧（实际要求 = 精确相等）。
#[test]
fn two_hour_frame_clock_has_no_cumulative_drift() {
    let frame = frame_len();
    let nominal = FPS23976.nominal().unwrap();
    let mut worst = Rational::ZERO;
    for i in 0..172_628i64 {
        // 第 i 帧的真实时间 = i * 1001/24000，全程有理数，不经过 float。
        let t = Time::from_rational(frame.mul(&Rational::from_int(i)).unwrap());
        let tc = t.to_bdn_timecode(FPS23976).unwrap();
        assert!(tc.ff < nominal, "FF 必须小于 nominal：{tc}");
        let back = BdnTimecode::parse(&tc.to_string())
            .unwrap()
            .to_time(FPS23976)
            .unwrap();
        let diff = abs(back.sub(&t).unwrap().as_rational());
        assert_eq!(diff, Rational::ZERO, "第 {i} 帧往返必须精确相等，得到 {tc}");
        if diff > worst {
            worst = diff;
        }
    }
    assert!(worst < frame, "最大误差 {worst} 必须小于 1 帧");
}

/// 同一件事再从"真的写出 XML 再读回来"的路径验证一遍（2 小时长片、4000+ 事件）。
#[test]
fn written_and_reparsed_two_hour_document_stays_within_one_frame() {
    let step = frame_len().mul(&Rational::from_int(41)).unwrap(); // ≈1.71 s
    let hold = frame_len().mul(&Rational::from_int(40)).unwrap();
    let mut events = Vec::new();
    let mut t = Time::ZERO;
    let mut i = 0usize;
    while t < Time::from_secs(7200) {
        let end = Time::from_rational(t.as_rational().add(&hold).unwrap());
        events.push(BdnEvent::new(t, end, graphic(i), FPS23976).unwrap());
        t = Time::from_rational(t.as_rational().add(&step).unwrap());
        i += 1;
    }
    assert!(i > 4000, "长片样本量：{i}");
    let doc = BdnDocument::new(meta(FPS23976), events);
    let xml = codec_bdn::write(&doc).unwrap();
    let back = codec_bdn::parse(&xml).unwrap();
    assert_eq!(back.events.len(), doc.events.len());
    for (orig, reparsed) in doc.events.iter().zip(back.events.iter()) {
        let diff = abs(reparsed.start.sub(&orig.start).unwrap().as_rational());
        assert!(diff < frame_len(), "反算误差 {diff} 必须小于 1 帧");
        assert_eq!(reparsed.in_tc, orig.in_tc);
    }
}

/// 缺陷一的直接反证：手搓"秒/帧来自两套舍入"的时间码（旧做法：把总帧数按真实 fps 取模），
/// 必须被守卫拒绝，`write` 也必须整份拒绝。
#[test]
fn mixing_two_rounding_sources_is_rejected() {
    let t = Time::from_secs(7000);
    let mut ev =
        BdnEvent::new(t, t.add(&Time::from_secs(2)).unwrap(), graphic(0), FPS23976).unwrap();
    // 故意破坏单一时间源：写成"墙上秒 + FF=0"的老做法（正是旧闭源工具的缺陷一）。
    // 正确值应是第 167832 帧 -> 01:56:33:00；墙上 7000 秒 -> 01:56:40:00 是另一个舍入源。
    let secs = t.as_rational().floor_i64();
    ev.in_tc = BdnTimecode {
        h: secs / 3600,
        m: (secs % 3600) / 60,
        s: secs % 60,
        ff: 0,
    };
    assert_eq!(ev.in_tc.to_string(), "01:56:40:00");
    assert_ne!(ev.in_tc, t.to_bdn_timecode(FPS23976).unwrap());
    let doc = BdnDocument::new(meta(FPS23976), vec![ev]);

    let e = doc.timecodes_are_derived().unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 61));
    let w = codec_bdn::write(&doc).unwrap_err();
    assert_eq!((w.domain, w.code), (Domain::Bdn, 61));
}

#[test]
fn frame_field_is_always_below_nominal_base() {
    for ms in (0..3_600_000).step_by(7919) {
        let tc = t_ms(ms).to_bdn_timecode(FPS23976).unwrap();
        assert!((0..24).contains(&tc.ff), "{tc}");
        let back = BdnTimecode::parse(&tc.to_string())
            .unwrap()
            .to_time(FPS23976)
            .unwrap();
        let err = abs(back.sub(&t_ms(ms)).unwrap().as_rational());
        // 量化误差的上界就是 1 帧本身；说"小于半帧"会隐含第二次舍入（round），这里刻意不那么声明。
        assert!(err < frame_len(), "ms={ms} 误差 {err} 必须小于 1 帧");
    }
}

#[test]
fn integer_fps_timecodes_are_exact_frame_grid() {
    let fps = Fps::PAL_25;
    for f in 0..3600i64 {
        let t = Time::from_rational(Rational::new(f, 25).unwrap());
        let tc = t.to_bdn_timecode(fps).unwrap();
        assert_eq!(tc.ff, f % 25);
        assert_eq!(
            BdnTimecode::parse(&tc.to_string())
                .unwrap()
                .to_time(fps)
                .unwrap(),
            t
        );
    }
}

/// 负时间不允许进 BDN（写出会给出无法回读的时间码）。
#[test]
fn negative_time_is_rejected_by_timecode_generation() {
    let neg = t_ms(-1);
    let e = BdnEvent::new(neg, Time::from_secs(1), graphic(0), FPS23976).unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 3));
}
