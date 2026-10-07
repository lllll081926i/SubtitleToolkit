//! 渲染采样计划（09 §6、13 §8、40 §3.1）。
//! 多个输出 profile 先求采样时间点的并集，只渲染一次，再由各 sink 消费。

use core_model::time::{Rational, Time};

/// 为什么需要这个采样点。
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SampleReason {
    /// 视频帧边界（BDN 主用途）
    FrameBoundary,
    /// ASS 事件精确 start/end（High Precision SUP）
    EventStart,
    EventEnd,
    /// 动态特效需要（\t/\move/karaoke/fade/animated clip）
    Dynamic,
    /// 用户预览请求
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Sink {
    Xml,
    Sup,
    Preview,
}

/// 采样点集合。内部按 90kHz 整数 ticks 排序去重，避免浮点抖动。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleSet {
    pub samples: Vec<RenderSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSample {
    pub pts_90k: i64,
    pub time: Time,
    pub reason: SampleReason,
    pub sinks: u32,
}

impl SampleSet {
    pub fn new() -> SampleSet {
        SampleSet::default()
    }

    /// 加入一个采样点；同 pts 已存在时合并 sink 位并保留更强的 reason。
    pub fn add(
        &mut self,
        time: Time,
        reason: SampleReason,
        sinks: u32,
    ) -> Result<(), core_model::error::StkError> {
        let pts = time.to_pts_ticks()?;
        if let Some(existing) = self.samples.iter_mut().find(|s| s.pts_90k == pts) {
            existing.sinks |= sinks;
            if reason_rank(reason) > reason_rank(existing.reason) {
                existing.reason = reason;
            }
            return Ok(());
        }
        let s = RenderSample {
            pts_90k: pts,
            time,
            reason,
            sinks,
        };
        let pos = self.samples.partition_point(|x| x.pts_90k < pts);
        self.samples.insert(pos, s);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 只取某个 sink 需要的样本序列（sink 各自维护 event segmentation 状态）。
    pub fn for_sink(&self, sink: Sink) -> Vec<RenderSample> {
        let bit = sink_bit(sink);
        self.samples
            .iter()
            .copied()
            .filter(|s| s.sinks & bit != 0)
            .collect()
    }
}

fn reason_rank(r: SampleReason) -> u8 {
    match r {
        SampleReason::FrameBoundary => 1,
        SampleReason::Preview => 2,
        SampleReason::EventStart | SampleReason::EventEnd => 3,
        SampleReason::Dynamic => 4,
    }
}

pub fn sink_bit(s: Sink) -> u32 {
    match s {
        Sink::Xml => 1,
        Sink::Sup => 2,
        Sink::Preview => 4,
    }
}

/// 单个动态事件的采样帧上限（37 §4：禁止无界缓存/无界循环）。
pub const MAX_DYNAMIC_SAMPLES: i64 = 200_000;

/// 计划器输入：一个 profile 的采样需求。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDemand {
    pub name: String,
    pub sinks: u32,
    pub fps: core_model::time::Fps,
    /// 是否把 ASS 事件精确边界也取进来（High Precision SUP）
    pub exact_event_boundaries: bool,
    /// 事件区间 [start, end]
    pub events: Vec<(Time, Time)>,
    /// 含动态特效的事件下标
    pub dynamic_events: Vec<usize>,
}

/// 动态事件在 [start,end] 内按 fps 逐帧采样；静态事件只取首尾。
/// HFR 下采样策略（37 §）留待 image-core 结合视觉阈值决定，这里不擅自丢帧。
pub fn plan(demands: &[ProfileDemand]) -> Result<SampleSet, core_model::error::StkError> {
    let mut set = SampleSet::new();
    for d in demands {
        let fps = d.fps.as_rational();
        for (i, (start, end)) in d.events.iter().enumerate() {
            if d.exact_event_boundaries {
                set.add(*start, SampleReason::EventStart, d.sinks)?;
                set.add(*end, SampleReason::EventEnd, d.sinks)?;
            }
            let is_dynamic = d.dynamic_events.contains(&i);
            if !is_dynamic && !d.exact_event_boundaries {
                // 静态事件且不需要精确边界：仍要有落在事件内的帧边界
                let mid = start.add(end)?.scale(&Rational::new(1, 2)?)?;
                set.add(mid, SampleReason::FrameBoundary, d.sinks)?;
                continue;
            }
            if !is_dynamic {
                continue;
            }
            let mut f = start.snap_to_frame(core_model::time::Fps::new(fps)?)?;
            let end_snapped = end.snap_to_frame(core_model::time::Fps::new(fps)?)?;
            let step = Rational::new(1, 1)?.div(&fps).ok_or_else(bad_ratio_err)?;
            // 事前估算，避免把超范围事件真的插进 Vec（排序插入是 O(n)，事后 guard 会变成 O(n^2) 卡死）
            let span_frames = end_snapped
                .sub(&f)?
                .as_rational()
                .mul(&fps)
                .ok_or_else(bad_ratio_err)?
                .round_i64()
                .ok_or_else(bad_ratio_err)?;
            if span_frames + 1 > MAX_DYNAMIC_SAMPLES {
                return Err(core_model::error::StkError::simple(
                    core_model::Domain::Render,
                    30,
                    "动态采样点数超出安全上限",
                )
                .with_detail(format!(
                    "事件 {}..{} 约需 {span_frames} 帧采样，上限 {MAX_DYNAMIC_SAMPLES}",
                    start.as_rational(),
                    end.as_rational()
                ))
                .with_action("缩小该事件的动态区间，或降低目标帧率"));
            }
            let mut guard = 0u64;
            while f <= end_snapped {
                set.add(f, SampleReason::Dynamic, d.sinks)?;
                f = f.add(&Time::from_rational(step))?;
                guard += 1;
                if guard > 2_000_000 {
                    return Err(core_model::error::StkError::simple(
                        core_model::Domain::Render,
                        30,
                        "动态采样点数超出安全上限",
                    )
                    .with_detail(format!(
                        "事件 {}..{}",
                        start.as_rational(),
                        end.as_rational()
                    )));
                }
            }
        }
    }
    Ok(set)
}

// Transform/2 是 core-model `Rational::new` 的「分母为 0」；采样算术出问题是本 crate 的事，
// 报在自己的 Render 段里（ISSUES H8）。
fn bad_ratio_err() -> core_model::error::StkError {
    core_model::error::StkError::simple(core_model::Domain::Render, 3, "非法比率")
}

/// composited RGBA 的可见帧指纹（40 §3.2：不能只信 libass detect_change）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameFingerprint {
    pub bbox: Option<(u32, u32, u32, u32)>,
    pub hash: u64,
    pub non_zero_pixels: u64,
}

impl FrameFingerprint {
    /// 对最终合成结果计算；空帧（全透明）与有内容帧必须不同。
    pub fn of_rgba(pixels: &[u8], width: usize) -> FrameFingerprint {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut nz = 0u64;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (i, px) in pixels.chunks(4).enumerate() {
            if px.len() != 4 {
                break;
            }
            if px[3] >= 8 {
                nz += 1;
                let x = (i % width.max(1)) as u32;
                let y = (i / width.max(1)) as u32;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            for b in px {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100_0000_01b3);
            }
        }
        FrameFingerprint {
            bbox: if nz == 0 {
                None
            } else {
                Some((x0, y0, x1, y1))
            },
            hash: h,
            non_zero_pixels: nz,
        }
    }

    /// hash 命中后再做一次防碰撞的字节比较（40 §2.4 的可选项）
    pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::time::{Fps, Time};

    fn t(ms: i64) -> Time {
        Time::from_millis(ms).unwrap()
    }

    #[test]
    fn samples_dedupe_by_pts_and_merge_sinks() {
        let mut s = SampleSet::new();
        s.add(t(1000), SampleReason::FrameBoundary, sink_bit(Sink::Xml))
            .unwrap();
        s.add(t(1000), SampleReason::EventStart, sink_bit(Sink::Sup))
            .unwrap();
        assert_eq!(s.len(), 1, "同一 90kHz ticks 必须合并成一个采样点");
        let one = s.samples[0];
        assert_eq!(one.reason, SampleReason::EventStart, "保留更强的 reason");
        assert_eq!(one.sinks, sink_bit(Sink::Xml) | sink_bit(Sink::Sup));
    }

    #[test]
    fn samples_stay_sorted_after_out_of_order_insert() {
        let mut s = SampleSet::new();
        for ms in [5000i64, 1000, 3000, 2000] {
            s.add(t(ms), SampleReason::FrameBoundary, 1).unwrap();
        }
        let pts: Vec<i64> = s.samples.iter().map(|x| x.pts_90k).collect();
        assert_eq!(pts, vec![90_000, 180_000, 270_000, 450_000]);
    }

    #[test]
    fn planner_unifies_two_profiles() {
        // 1080 SUP（帧边界 + 精确边界）与 2160 SUP 同 fps：并集而不是两倍渲染
        let d1 = ProfileDemand {
            name: "BD".into(),
            sinks: sink_bit(Sink::Sup) | sink_bit(Sink::Xml),
            fps: Fps::NTSC_23976,
            exact_event_boundaries: true,
            events: vec![(t(1000), t(2000))],
            dynamic_events: vec![],
        };
        let d2 = ProfileDemand {
            name: "UHD".into(),
            sinks: sink_bit(Sink::Sup),
            fps: Fps::NTSC_23976,
            exact_event_boundaries: true,
            events: vec![(t(1000), t(2000))],
            dynamic_events: vec![],
        };
        let set = plan(&[d1, d2]).unwrap();
        assert_eq!(set.len(), 2, "两个 profile 的相同时间点必须复用");
        assert_eq!(set.for_sink(Sink::Xml).len(), 2);
        assert_eq!(set.for_sink(Sink::Sup).len(), 2);
    }

    #[test]
    fn dynamic_event_samples_every_frame_but_static_does_not() {
        let d = ProfileDemand {
            name: "x".into(),
            sinks: 1,
            fps: Fps::PAL_25,
            exact_event_boundaries: false,
            events: vec![(t(0), t(400)), (t(1000), t(1400))],
            dynamic_events: vec![1],
        };
        let set = plan(&[d]).unwrap();
        // 静态事件：1 个中点；动态事件 400ms@25fps → 11 帧（含首尾）
        assert_eq!(
            set.for_sink(Sink::Xml)
                .iter()
                .filter(|s| s.reason == SampleReason::Dynamic)
                .count(),
            11
        );
        assert_eq!(
            set.for_sink(Sink::Xml)
                .iter()
                .filter(|s| s.reason == SampleReason::FrameBoundary)
                .count(),
            1
        );
    }

    #[test]
    fn runaway_dynamic_range_is_rejected_not_hung() {
        let d = ProfileDemand {
            name: "x".into(),
            sinks: 1,
            fps: Fps::NTSC_23976,
            exact_event_boundaries: false,
            events: vec![(t(0), Time::from_secs(100_000))],
            dynamic_events: vec![0],
        };
        let e = plan(&[d]).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0030");
    }

    #[test]
    fn fingerprint_distinguishes_empty_and_content() {
        let empty = vec![0u8; 4 * 16];
        let mut some = empty.clone();
        some[4 * 6 + 3] = 255; // 第 7 个像素不透明（宽 4 → x=2,y=1）
        let f0 = FrameFingerprint::of_rgba(&empty, 4);
        let f1 = FrameFingerprint::of_rgba(&some, 4);
        assert_eq!(f0.bbox, None);
        assert_eq!(f0.non_zero_pixels, 0);
        assert_eq!(f1.non_zero_pixels, 1);
        assert_eq!(f1.bbox, Some((2, 1, 2, 1)));
        assert_ne!(f0.hash, f1.hash);
        assert!(FrameFingerprint::bytes_equal(&empty, &empty));
        assert!(!FrameFingerprint::bytes_equal(&empty, &some));
    }

    #[test]
    fn low_alpha_pixels_do_not_count_as_visible() {
        let mut px = vec![0u8; 4 * 2];
        px[3] = 7; // alpha 7 < 8 阈值
        let f = FrameFingerprint::of_rgba(&px, 2);
        assert_eq!(f.non_zero_pixels, 0);
        assert_eq!(f.bbox, None);
    }
}
