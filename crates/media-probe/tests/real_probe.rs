//! 16 §2-§3 与 AT-PROBE-001 的**真工具**验收：真的 ffprobe + 真的视频文件。
//!
//! 为什么必须有这个文件：`fields.rs` 的色彩解析此前按我想象中的嵌套 schema 写，
//! 单测全绿、真二进制完全失效（ISSUES AF1）。单元级 fixture 只能证明"我的代码自洽"，
//! 证明不了"我读对了别人的输出"。
//!
//! 需要外部工具时按同一套纪律办：`STK_FFPROBE` 指到二进制，门阶用
//! `STK_REQUIRE_FFPROBE=1` 把"没跑"变成失败，而不是"因为没跑所以通过"。
//!
//! 本文件也承担**容器层**的同一类验收（G4）：我们写的 .sup 必须被**别人的**解析器
//! 读出来并且时钟一致。自家 writer + 自家 reader 的往返只能证明自洽，证明不了布局对
//! —— 这正是 13 §2 反复踩过的那一格（ISSUES D9：子代理写的段头步进把 DTS 当成了长度）。
//! 放在这里而不是 codec-pgs，是因为 ffprobe 的找法与"没跑即失败"的纪律已经在这文件里
//! 立好了（Z4：两份实现各写一遍，就会有一份忘了看那个环境变量）。

use std::path::{Path, PathBuf};
use std::process::Command;

use core_model::time::Fps;
use media_probe::{
    parse_video_streams, suggest_for, Candidate, ProbeRunner, SubtitleContext, SystemProbe,
};

fn ffprobe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_FFPROBE") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    // 二进制在仓库**旁边**：crates/media-probe/../.. 是仓库根，还要再上一层才到 out/tools/bin。
    // 这一层曾经少写（与 CLI 侧的 AF2 同一族），后果是"不设 STK_FFPROBE 就必 panic"，
    // 而门阶恰好设了它，所以这条回落从来没被跑到过。
    let guess = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("tools")
        .join("bin")
        .join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
    guess.is_file().then_some(guess)
}

macro_rules! need_ffprobe {
    () => {
        match ffprobe() {
            Some(p) => p,
            None => {
                if std::env::var("STK_REQUIRE_FFPROBE").is_ok() {
                    panic!("STK_REQUIRE_FFPROBE=1，但找不到 ffprobe（设 STK_FFPROBE 指到它）");
                }
                eprintln!("SKIP：本机没有 ffprobe");
                return;
            }
        }
    };
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn probe(exe: &Path, file: &str) -> Vec<media_probe::VideoStream> {
    let runner = SystemProbe::new(exe, std::time::Duration::from_secs(60));
    // runner.probe 已经把 ProbeError 映射成带码的 StkError，这里只负责把它说清楚
    let json = runner
        .probe(&fixture(file))
        .unwrap_or_else(|e| panic!("{file} 探测失败：{e}"));
    parse_video_streams(&json).unwrap_or_else(|e| panic!("{file} 解析失败：{e}"))
}

/// 4K PQ 素材：尺寸/帧率/色彩三项都要从**真输出**里读出来，并且 HDR 建议是 High 置信。
#[test]
fn real_ffprobe_reads_uhd_hdr_and_suggests_the_hdr_preset() {
    let exe = need_ffprobe!();
    let v = probe(&exe, "uhd23976-hdr.mp4");
    assert_eq!(v.len(), 1, "只该有一条视频流：{v:?}");
    assert_eq!((v[0].width, v[0].height), (3840, 2160));
    assert_eq!(
        v[0].r_frame_rate.map(|r| (r.num(), r.den())),
        Some((24000, 1001))
    );
    assert_eq!(
        v[0].color.transfer.as_deref(),
        Some("smpte2084"),
        "{:?}",
        v[0].color
    );
    assert_eq!(v[0].color.primaries.as_deref(), Some("bt2020"));
    assert!(v[0].is_hdr(), "PQ 素材必须被判 HDR（16 §2）");

    let set = suggest_for(&v[0], &SubtitleContext::default());
    assert!(
        set.items.iter().any(|s| {
            matches!(&s.candidate, Candidate::HdrPreset { reference_nits } if *reference_nits == 203)
        }),
        "4K+PQ 要给 HDR preset 候选（16 §3 规则 1）：{:?}",
        set.items
    );
    assert!(!set.recommends_speed_conversion, "永不自动降速（10 §8）");
    assert!(
        set.items.iter().all(|s| !s.evidence.is_empty()),
        "每条候选都必须带证据：{:?}",
        set.items
    );
}

/// 25fps 的 1080p SDR：帧率必须是精确的 25/1（不是 24.999…），且不产生 HDR 建议。
#[test]
fn real_ffprobe_reads_exact_25fps_sdr_without_hdr_advice() {
    let exe = need_ffprobe!();
    let v = probe(&exe, "fhd25.mp4");
    assert_eq!(v[0].fps_candidates(), vec![Fps::PAL_25]);
    assert_eq!(v[0].color.transfer.as_deref(), Some("bt709"));
    assert!(!v[0].is_hdr());
    let set = suggest_for(&v[0], &SubtitleContext::default());
    assert!(
        !set.items
            .iter()
            .any(|s| matches!(s.candidate, Candidate::HdrPreset { .. })),
        "SDR 素材不该冒出 HDR：{:?}",
        set.items
    );
}

/// 4:3 素材：16 §3 的规则是"非 16:9 不建议画布"，这条在真文件上也要成立。
#[test]
fn real_ffprobe_does_not_suggest_a_canvas_for_a_4_3_source() {
    let exe = need_ffprobe!();
    let v = probe(&exe, "sd43-30.mp4");
    assert_eq!((v[0].width, v[0].height), (640, 480));
    let set = suggest_for(&v[0], &SubtitleContext::default());
    assert!(
        !set.items
            .iter()
            .any(|s| matches!(s.candidate, Candidate::Canvas(_) | Candidate::PgsCanvas(_))),
        "4:3 视频被拿来当画布候选了：{:?}",
        set.items
    );
}

/// 二进制身份（16 §5/12 §4 的同一纪律）：路径与内容哈希一起报；哈希变化必须能被发现。
#[test]
fn ffprobe_identity_is_reported_by_content_not_by_name() {
    let exe = need_ffprobe!();
    let id = media_probe::FfprobeIdentity::of(&exe).expect("身份必须读得出来");
    assert!(id.size > 1_000_000, "ffprobe 是个大二进制：{id:?}");
    assert!(id.digest.starts_with("sha256-"), "{id:?}");
    assert_eq!(id.digest.len(), 7 + 64, "SHA-256 是 64 位十六进制");
    let again = media_probe::FfprobeIdentity::of(&exe).unwrap();
    assert_eq!(id.digest, again.digest, "同一个文件的身份必须稳定");
}

/// 探测一个不存在的文件：必须是可报告的依赖错误，而不是 panic 或挂住。
#[test]
fn probing_a_missing_file_reports_instead_of_hanging() {
    let exe = need_ffprobe!();
    let runner = SystemProbe::new(&exe, std::time::Duration::from_secs(30));
    let stk = runner
        .probe(Path::new("Z:/no-such-video-9f3a.mp4"))
        .expect_err("不存在的文件必须失败");
    assert!(stk.code_string().starts_with("STK-DEP-"), "{stk:?}");
    assert!(!stk.title.is_empty(), "{stk:?}");
}

/// 一个最小的可见对象：8x4 全不透明，落在 1920x1080 画布内。
fn one_object_frame(pts: i64) -> codec_pgs::encoder::Frame {
    codec_pgs::encoder::Frame::new(
        pts,
        vec![codec_pgs::encoder::FrameObject {
            rgba: vec![255u8; 8 * 4 * 4],
            width: 8,
            height: 4,
            x: 100,
            y: 100,
            forced: false,
        }],
    )
}

fn two_display_sets() -> Vec<codec_pgs::segment::Segment> {
    let frames = vec![one_object_frame(90_000), one_object_frame(406_800)];
    let (segments, _report) =
        codec_pgs::encoder::encode(&frames, &codec_pgs::encoder::EncodeOptions::default())
            .expect("两个显示集要能编码");
    segments
}

fn write_scratch_copy(bytes: &[u8], tag: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join(format!("interop-{}-{}.sup", std::process::id(), tag));
    std::fs::write(&path, bytes).expect("临时 .sup 写不下去");
    path
}

/// 让 ffprobe 把这份文件当 .sup 读，返回它看到的 (pts, pts_time) 序列。
/// 读不出来就 panic 并交代 stderr —— "读不出来"从来不是通过。
fn ffprobe_packets(exe: &Path, file: &Path) -> Vec<(i64, String)> {
    let out = Command::new(exe)
        .args(["-hide_banner", "-loglevel", "error", "-f", "sup", "-i"])
        .arg(file)
        .args(["-show_packets", "-of", "json"])
        .output()
        .expect("ffprobe 起不来");
    assert!(
        out.status.success(),
        "ffprobe 读我们的 .sup 失败（exit {:?}）：{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("ffprobe 输出的 JSON 解析不了");
    let packets = v
        .get("packets")
        .and_then(|p| p.as_array())
        .unwrap_or_else(|| panic!("输出里没有 packets 数组：{v}"));
    packets
        .iter()
        .map(|p| {
            (
                p.get("pts")
                    .and_then(|x| x.as_i64())
                    .unwrap_or_else(|| panic!("这一段没有 pts 字段：{p}")),
                p.get("pts_time")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .collect()
}

/// 容器层的外部裁判（13 §2、G4）：我们写的段头布局 `PG(2)+PTS(4)+DTS(4)+type(1)+len(2)`
/// 必须被**另一个独立实现**读出同样的段数与同样的 90 kHz 时钟。自家 writer + 自家 reader
/// 的往返只证明自洽，证明不了布局；而 13 §2 的历史恰恰是自洽地错着（ISSUES D9）。
#[test]
fn ffprobe_reads_our_pgs_container_with_the_same_clocks() {
    let exe = need_ffprobe!();
    let segments = two_display_sets();
    let bytes = codec_pgs::segment::write_stream(&segments).expect("段流写不出来");
    let file = write_scratch_copy(&bytes, "full");

    let seen = ffprobe_packets(&exe, &file);
    assert_eq!(
        seen.len(),
        segments.len(),
        "ffprobe 看到 {} 段，我们写了 {} 段",
        seen.len(),
        segments.len()
    );
    for (i, (pts, pts_time)) in seen.iter().enumerate() {
        assert_eq!(*pts, segments[i].pts, "第 {i} 段的 PTS 与我们的不一致");
        // 时钟基数也顺手钉住：90000 ticks 必须被读成 1 秒
        if *pts == 90_000 {
            assert!(pts_time.starts_with("1.000"), "pts_time={pts_time}");
        }
    }

    // 反向对照：截断到中段，ffprobe 不许还报出同样的段数与同样的时钟
    // （没有这一步，上面的比较可能是"任何输出都算过"）
    let cut = write_scratch_copy(&bytes[..bytes.len() / 2], "truncated");
    let partial = ffprobe_packets(&exe, &cut);
    assert!(
        partial.len() < segments.len()
            || partial.iter().zip(&segments).any(|((p, _), s)| *p != s.pts),
        "截断一半之后 ffprobe 仍报出完整的一串（{} 段）—— 这条对照没有牙",
        partial.len()
    );
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_file(&cut);
}
