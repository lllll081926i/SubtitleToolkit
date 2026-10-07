//! pgs-author 的规划与端到端证据（13 §4 必须项、09 §3 位置不变、13 §6 自检）。

use codec_pgs::display_set::group_display_sets;
use codec_pgs::encoder::EncodeOptions;
use codec_pgs::segment::parse_stream;
use core_model::canvas::Canvas;
use core_model::profile::ValidationLevel;
use core_model::time::{Fps, Time};
use image_core::Frame;
use pgs_author::{plan, AuthorOptions, EventKind, Sample};
use validator::ProfileWant;

const C: Canvas = Canvas::HD720;

/// 在完整画布上画一个实心矩形（模拟一行字幕）。
fn band(x0: u32, y0: u32, w: u32, h: u32) -> Frame {
    let mut f = Frame::empty(C).unwrap();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let off = ((y * C.width + x) * 4) as usize;
            f.rgba[off..off + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    f
}

/// 空白但 RGB 有脏值的帧：alpha 全 0，按 09 §3 不算内容。
fn dirty_blank() -> Frame {
    let mut f = Frame::empty(C).unwrap();
    for chunk in f.rgba.chunks_exact_mut(4) {
        chunk[0] = 99;
        chunk[1] = 101;
        chunk[2] = 103;
        chunk[3] = 0;
    }
    f
}

fn t(ms: i64) -> Time {
    Time::from_millis(ms).unwrap()
}

fn opts(ms: i64) -> AuthorOptions {
    AuthorOptions { stream_end: t(ms) }
}

#[test]
fn identical_samples_collapse_into_one_event() {
    let f = band(100, 300, 400, 30);
    let samples: Vec<Sample> = (0..5)
        .map(|i| Sample::new(t(1000 + i * 400), f.clone()))
        .collect();
    let p = plan(&samples, &opts(4000)).unwrap();
    assert_eq!(p.events.len(), 1, "5 个相同采样必须并成 1 个显示集事件");
    assert_eq!(p.stats.eliminated, 4);
    assert_eq!(p.events[0].merged_samples, 5);
    assert_eq!(p.events[0].start, t(1000));
    assert_eq!(p.events[0].end, t(4000), "最后一个事件必须用到 stream_end");
    assert!(
        p.loss
            .dropped
            .iter()
            .any(|t| t.contains("重复视觉事件消除")),
        "合并必须进 LossReport：{:?}",
        p.loss.dropped
    );
}

#[test]
fn invisible_rgb_changes_do_not_split_events() {
    // 旧工程式缺陷的反例：只比较 RGBA 全部字节会把"看不见的变化"当成新事件
    let mut a = band(100, 300, 400, 30);
    let b = a.clone();
    // 在 bbox 之外的透明区域涂脏 RGB
    for y in 0..50u32 {
        for x in 0..50u32 {
            let off = ((y * C.width + x) * 4) as usize;
            a.rgba[off] = 7;
            a.rgba[off + 1] = 7;
            a.rgba[off + 2] = 7;
        }
    }
    let samples = vec![Sample::new(t(1000), a), Sample::new(t(1400), b)];
    let p = plan(&samples, &opts(5000)).unwrap();
    assert_eq!(p.events.len(), 1, "透明区域的 RGB 变化不得产生新事件");
}

#[test]
fn moving_the_same_bitmap_is_a_new_event() {
    let samples = vec![
        Sample::new(t(1000), band(100, 300, 400, 30)),
        Sample::new(t(2000), band(110, 300, 400, 30)),
    ];
    let p = plan(&samples, &opts(5000)).unwrap();
    assert_eq!(p.events.len(), 2, "位置就是内容的一部分（09 §3 画布坐标）");
    assert_ne!(p.events[0].digest, p.events[1].digest);
}

#[test]
fn leading_blank_is_dropped_and_interior_blank_becomes_erase() {
    let samples = vec![
        Sample::new(t(500), dirty_blank()),
        Sample::new(t(1000), band(100, 300, 400, 30)),
        Sample::new(t(2000), dirty_blank()),
        Sample::new(t(3000), band(600, 300, 200, 30)),
    ];
    let p = plan(&samples, &opts(6000)).unwrap();
    assert_eq!(p.stats.leading_erase_dropped, 1, "开头空白没有可擦内容");
    assert_eq!(
        p.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec![EventKind::Image, EventKind::Erase, EventKind::Image]
    );
    assert_eq!(p.events[0].end, t(2000), "事件结束于下一个内容开始处");
    assert_eq!(p.events[1].patch, None);
    assert!(
        p.loss.dropped.iter().any(|t| t.contains("erase")),
        "丢弃开头空白必须记账：{:?}",
        p.loss.dropped
    );
}

#[test]
fn consecutive_blanks_produce_only_one_erase_set() {
    let samples = vec![
        Sample::new(t(1000), band(100, 300, 400, 30)),
        Sample::new(t(2000), dirty_blank()),
        Sample::new(t(2500), dirty_blank()),
        Sample::new(t(3000), dirty_blank()),
    ];
    let p = plan(&samples, &opts(9000)).unwrap();
    assert_eq!(p.stats.erase_events, 1, "连着擦三次是多余的");
    assert_eq!(p.events.len(), 2);
}

#[test]
fn plan_rejects_bad_input() {
    let f = band(0, 0, 8, 8);
    let bad_order = vec![
        Sample::new(t(2000), f.clone()),
        Sample::new(t(1000), f.clone()),
    ];
    let e = plan(&bad_order, &opts(9000)).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0111");

    let short_end = vec![Sample::new(t(2000), f.clone())];
    let e = plan(&short_end, &opts(2000)).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0112");

    let e = plan(&[], &opts(9000)).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0110");
}

#[test]
fn authored_stream_is_valid_and_keeps_canvas_positions() {
    let samples = vec![
        Sample::new(t(1000), band(100, 300, 400, 30)),
        Sample::new(t(2000), band(100, 300, 400, 30)), // 重复：应被消除
        Sample::new(t(3000), dirty_blank()),
    ];
    let eopt = EncodeOptions {
        canvas_width: C.width as u16,
        canvas_height: C.height as u16,
        ..Default::default()
    };
    let want = ProfileWant {
        level: ValidationLevel::Basic,
        claim_compliant: false,
    };
    let a = pgs_author::author(
        &samples,
        &opts(6000),
        &eopt,
        &want,
        Some(Fps::NTSC_23976),
        None,
    )
    .expect("author 必须成功");

    // 结构自检：13 §6 第 1-4 层不能有 error/fatal
    a.ensure_valid().unwrap();
    assert!(
        !a.validation.strict_badge_allowed(ValidationLevel::Basic),
        "Basic 报告不能倒推 Strict 徽章"
    );
    let strict = validator::pgs(
        &a.segments,
        &ProfileWant {
            level: ValidationLevel::Strict,
            claim_compliant: true,
        },
        Some(Fps::NTSC_23976),
        None,
    );
    assert!(
        !strict.strict_badge_allowed(ValidationLevel::Strict),
        "第 5/7 层未覆盖，按 Strict 跑也必须被拦下"
    );
    assert!(strict.has_check("PGS.DEC.01") && strict.has_check("PGS.DEV.01"));

    // 采样消除确实发生在流里：3 个采样 → 2 个显示集（1 图 + 1 erase）
    assert_eq!(a.plan.stats.events, 2);
    let segs = parse_stream(&a.bytes).unwrap();
    let sets = group_display_sets(&segs).unwrap();
    assert_eq!(sets.len(), 2, "实际段流里的显示集数必须等于计划事件数");
    assert!(!sets[0].is_erase);
    assert!(
        sets[1].is_erase,
        "第二个显示集必须是 erase，而不是一张全透明图"
    );
    assert_eq!(sets[0].pts, 90_000, "事件起点 1000ms -> 90 kHz");
    assert_eq!(sets[1].pts, 270_000, "erase 事件起点 3000ms");
    assert!(sets[0].dts < sets[0].pts, "decode lead 必须让 DTS 早于 PTS");

    let pcs0 = sets[0].pcs.as_ref().unwrap();
    assert_eq!(
        (pcs0.width, pcs0.height),
        (C.width as u16, C.height as u16),
        "PCS canvas 永远是完整 16:9 画布（13 §4）"
    );
    let patch = a.plan.events[0].patch.as_ref().unwrap();
    let obj = pcs0
        .objects
        .iter()
        .find(|o| sets[0].object_ids.first() == Some(&o.object_id))
        .expect("PCS 必须有那个 composition object");
    assert_eq!(
        (obj.x, obj.y),
        (patch.x as u16, patch.y as u16),
        "PCS placement 必须是完整画布坐标，不是相对 bbox（09 §3）"
    );
    assert!(a.summary().contains("显示集"));
}
