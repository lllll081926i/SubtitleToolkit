//! 排序守卫（13 §7「event 必须排序」/ 40 §审计的缺陷三：旧工程未按 start 排序就写出）。
//!
//! 新实现的策略是"默认拒绝而不是偷偷排序"：排序属于 task-engine 的时间线规划，
//! writer 只做单一时间源的边界转换。

use codec_bdn::{build_document, BdnFrame, BdnGraphic, BdnOptions};
use core_model::error::Domain;
use core_model::time::Fps;

mod common;
use common::{doc, meta};

const FPS: Fps = Fps::NTSC_23976;

#[test]
fn write_rejects_descending_events_and_reports_first_disorder() {
    let d = doc(FPS, &[5000, 1000, 3000]);
    let e = codec_bdn::write(&d).unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 60));
    let msg = e.to_string() + &e.details.join("\n");
    assert!(
        msg.contains("events[1]"),
        "必须给第一个逆序位置，得到 {msg}"
    );
}

#[test]
fn write_rejects_equal_starts_because_order_is_strict() {
    let d = doc(FPS, &[1000, 2000, 2000]);
    let e = codec_bdn::write(&d).unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 60));
    assert!(e.details.join("\n").contains("events[2]"));
}

#[test]
fn write_rejects_first_event_being_the_only_disorder_at_index_one() {
    let d = doc(FPS, &[10_000, 1_000]);
    let e = codec_bdn::write(&d).unwrap_err();
    assert!(e.details.join("\n").contains("events[1]"));
}

#[test]
fn sorted_events_write_fine_and_do_not_get_reordered() {
    let d = doc(FPS, &[1000, 2000, 3000]);
    let xml = codec_bdn::write(&d).unwrap();
    let back = codec_bdn::parse(&xml).unwrap();
    let files: Vec<&str> = back
        .events
        .iter()
        .map(|e| e.graphic.file_name.as_str())
        .collect();
    assert_eq!(files, vec!["00000.png", "00001.png", "00002.png"]);
}

#[test]
fn parse_keeps_file_order_even_when_unsorted() {
    // 读侧不重排：parse→(未排序)→write 必须失败，而不是悄悄变成有序。
    let xml = r#"<BDN Version="0.93">
<BDNFormat><VideoFormat>1080p</VideoFormat><FrameRate>25</FrameRate>
<DisplayResolution><Width>1920</Width><Height>1080</Height></DisplayResolution></BDNFormat>
<BDNSubtitleData>
<Event InTC="00:00:02:00" OutTC="00:00:03:00"><Graphic><FileName>b.png</FileName>
<Width>100</Width><Height>50</Height><X>0</X><Y>0</Y></Graphic></Event>
<Event InTC="00:00:01:00" OutTC="00:00:01:12"><Graphic><FileName>a.png</FileName>
<Width>100</Width><Height>50</Height><X>0</X><Y>0</Y></Graphic></Event>
</BDNSubtitleData></BDN>"#;
    let d = codec_bdn::parse(xml).unwrap();
    assert_eq!(d.events[0].graphic.file_name, "b.png");
    assert_eq!(d.events[1].graphic.file_name, "a.png");
    assert_eq!(d.first_disorder(), Some(1));
    assert_eq!(codec_bdn::write(&d).unwrap_err().code, 60);
}

#[test]
fn build_document_rejects_unsorted_frames_with_position() {
    let frames = vec![frame(0, 1000, 2000), frame(1, 500, 900)];
    let e = build_document(&frames, &opts(0)).unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 60));
    assert!(e.details.join("\n").contains("kept[1]"), "{:?}", e.details);
}

#[test]
fn zero_duration_event_is_rejected() {
    let f = BdnFrame {
        start: core_model::time::Time::from_millis(1000).unwrap(),
        end: core_model::time::Time::from_millis(1000).unwrap(),
        png_file_name: "x.png".to_string(),
        png_width: 100,
        png_height: 50,
        x: 0,
        y: 0,
    };
    // 构造函数这一层就拒绝（公开 API 造不出零时长事件）。
    let e = build_document(&[f], &opts(0)).unwrap_err();
    assert_eq!((e.domain, e.code), (Domain::Bdn, 59));

    // 绕过构造函数也骗不过 write：直接填字段造一个零时长事件。
    use codec_bdn::{BdnDocument, BdnEvent};
    use core_model::time::Time;
    let t = Time::from_millis(1000).unwrap();
    let ev = BdnEvent {
        in_tc: t.to_bdn_timecode(FPS).unwrap(),
        out_tc: t.to_bdn_timecode(FPS).unwrap(),
        start: t,
        end: t,
        graphic: BdnGraphic::new("x.png", 100, 50, 0, 0).unwrap(),
    };
    let d = BdnDocument::new(meta(FPS), vec![ev]);
    assert_eq!(codec_bdn::write(&d).unwrap_err().code, 59);
}

fn frame(i: usize, start_ms: i64, end_ms: i64) -> BdnFrame {
    BdnFrame {
        start: core_model::time::Time::from_millis(start_ms).unwrap(),
        end: core_model::time::Time::from_millis(end_ms).unwrap(),
        png_file_name: format!("{i:05}.png"),
        png_width: 800,
        png_height: 120,
        x: 560,
        y: 880,
    }
}

pub fn opts(min_axis: u32) -> BdnOptions {
    BdnOptions {
        fps: FPS,
        language: "chi".to_string(),
        name: "测试".to_string(),
        min_axis,
        offset_x: 0,
        offset_y: 0,
    }
}
