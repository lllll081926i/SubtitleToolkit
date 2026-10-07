//! 探测结果 → 建议（16 §3-§4）。核心约束：建议只是候选值，
//! 永远不因为"用户选了一个 25fps 视频"就自动改时间轴（16 §3、AT-PROBE-001）。

use core_model::canvas::Canvas;
use core_model::profile::ValueOrigin;
use core_model::time::Fps;

use crate::fields::VideoStream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    /// Low 默认不勾选（16 §4）。
    pub fn checked_by_default(self) -> bool {
        !matches!(self, Confidence::Low)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Candidate {
    Fps(Fps),
    Canvas(Canvas),
    /// PGS 画布建议（严格蓝光时会被压回 2K）
    PgsCanvas(Canvas),
    HdrPreset {
        reference_nits: u32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub candidate: Candidate,
    pub origin: ValueOrigin,
    pub confidence: Confidence,
    /// 必须能回答"为什么建议这个？"（35 §7）
    pub evidence: String,
}

impl Suggestion {
    pub fn auto_checked(&self) -> bool {
        self.confidence.checked_by_default()
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SubtitleContext {
    /// 字幕当前逻辑画布（ASS PlayRes）
    pub canvas: Option<Canvas>,
    pub fps: Option<Fps>,
    /// 目标输出是否为严格蓝光/ UHD-BD PGS
    pub strict_bluray_pgs: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SuggestionSet {
    pub items: Vec<Suggestion>,
    /// 恒为 false：帧率差异不能自动推断成 speed conversion（10 §8）
    pub recommends_speed_conversion: bool,
    pub notes: Vec<String>,
}

impl SuggestionSet {
    pub fn fps_candidates(&self) -> Vec<Fps> {
        self.items
            .iter()
            .filter_map(|s| match s.candidate {
                Candidate::Fps(f) => Some(f),
                _ => None,
            })
            .collect()
    }
    pub fn hdr(&self) -> Option<&Suggestion> {
        self.items
            .iter()
            .find(|s| matches!(s.candidate, Candidate::HdrPreset { .. }))
    }
    pub fn pgs_canvas(&self) -> Option<Canvas> {
        self.items.iter().find_map(|s| {
            if let Candidate::PgsCanvas(c) = s.candidate {
                Some(c)
            } else {
                None
            }
        })
    }
    pub fn find(&self, want: &Candidate) -> Option<&Suggestion> {
        self.items.iter().find(|s| &s.candidate == want)
    }
}

pub fn suggest_for(stream: &VideoStream, ctx: &SubtitleContext) -> SuggestionSet {
    let mut items: Vec<Suggestion> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let target = Canvas {
        width: stream.width,
        height: stream.height,
    };

    // 规则 1：3840×2160 + PQ/HLG → HDR preset（16 §3）
    if stream.is_hdr() {
        let conf = if stream.width >= 3840 {
            Confidence::High
        } else {
            Confidence::Medium
        };
        items.push(Suggestion {
            candidate: Candidate::HdrPreset {
                reference_nits: 203,
            },
            origin: ValueOrigin::VideoProbe,
            confidence: conf,
            evidence: format!(
                "transfer={:?} primaries={:?} size={}x{}",
                stream.color.transfer.as_deref(),
                stream.color.primaries.as_deref(),
                stream.width,
                stream.height
            ),
        });
    }

    // 规则 2：填候选 FPS，但明确不执行 speed transform
    for f in stream.fps_candidates() {
        let conf = match (stream.r_frame_rate, stream.avg_frame_rate) {
            (Some(r), Some(a)) if r == a => Confidence::High,
            (Some(_), Some(_)) => Confidence::Medium,
            _ => Confidence::Low,
        };
        items.push(Suggestion {
            candidate: Candidate::Fps(f),
            origin: ValueOrigin::VideoProbe,
            confidence: conf,
            evidence: format!(
                "r_frame_rate={:?} avg_frame_rate={:?}",
                stream.r_frame_rate, stream.avg_frame_rate
            ),
        });
    }
    if let Some(sub_fps) = ctx.fps {
        if stream.avg_frame_rate.is_some() && !stream.fps_candidates().contains(&sub_fps) {
            notes.push(format!("视频帧率与字幕 FPS({sub_fps}) 不同，仅可能表示不同片源版本；未自动选择 speed conversion"));
        }
    }

    // 规则 3：字幕画布小于视频 → 建议加一档 ASS 输出
    if let Some(sc) = ctx.canvas {
        if target.is_16_9()
            && sc.is_16_9()
            && (target.width, target.height) != (sc.width, sc.height)
        {
            let bigger = u64::from(target.width) * u64::from(target.height)
                > u64::from(sc.width) * u64::from(sc.height);
            items.push(Suggestion {
                candidate: Candidate::Canvas(target),
                origin: ValueOrigin::VideoProbe,
                confidence: if bigger {
                    Confidence::High
                } else {
                    Confidence::Low
                },
                evidence: format!("字幕 PlayRes={sc} 视频={target}"),
            });
        }
    }

    // 规则 4：严格蓝光/ UHD-BD PGS 仍建议 2K 画布（09 §6、16 §3）
    if ctx.strict_bluray_pgs {
        let c = if target.within_bluray() && target.is_16_9() {
            target
        } else {
            Canvas::FHD1080
        };
        items.push(Suggestion {
            candidate: Candidate::PgsCanvas(c),
            origin: ValueOrigin::VideoProbe,
            confidence: Confidence::High,
            evidence: format!(
                "视频 {target}；PGS 规范上限 {}x{}，2160p PGS 不标记为蓝光合规",
                core_model::canvas::BLURAY_MAX.width,
                core_model::canvas::BLURAY_MAX.height
            ),
        });
        if !target.within_bluray() {
            notes.push(format!(
                "目标 {target} 超出蓝光 PGS 上限，严格 profile 已改建议 {}",
                Canvas::FHD1080
            ));
        }
    }

    SuggestionSet {
        items,
        recommends_speed_conversion: false,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::ColorMeta;
    use core_model::time::Rational;

    fn stream(w: u32, h: u32, r: &str, a: &str, transfer: Option<&str>) -> VideoStream {
        VideoStream {
            index: 0,
            width: w,
            height: h,
            r_frame_rate: Some(Rational::parse(r).unwrap()),
            avg_frame_rate: Some(Rational::parse(a).unwrap()),
            duration: None,
            color: ColorMeta {
                primaries: Some("bt709".into()),
                transfer: transfer.map(str::to_string),
                matrix: None,
                range: None,
                pix_fmt: None,
            },
            hdr_signaled: false,
        }
    }

    #[test]
    fn uhd_hdr_yields_high_confidence_hdr_preset() {
        let s = stream(3840, 2160, "24000/1001", "24000/1001", Some("smpte2084"));
        let set = suggest_for(&s, &SubtitleContext::default());
        let hdr = set.hdr().expect("应有 HDR 建议");
        assert_eq!(hdr.confidence, Confidence::High);
        assert!(hdr.evidence.contains("smpte2084"));
        assert_eq!(hdr.origin, ValueOrigin::VideoProbe);
        assert!(hdr.auto_checked());
    }

    #[test]
    fn sdr_1080p_makes_no_hdr_suggestion() {
        let s = stream(1920, 1080, "25/1", "25/1", Some("bt709"));
        let set = suggest_for(&s, &SubtitleContext::default());
        assert!(set.hdr().is_none());
        assert_eq!(set.fps_candidates(), vec![Fps::PAL_25]);
    }

    #[test]
    fn probe_never_recommends_speed_conversion() {
        // AT-PROBE-001 的硬断言：即使视频 FPS 与字幕 FPS 不同也不自动建议 speed
        let s = stream(1920, 1080, "25/1", "25/1", None);
        let ctx = SubtitleContext {
            canvas: None,
            fps: Some(Fps::NTSC_23976),
            strict_bluray_pgs: false,
        };
        let set = suggest_for(&s, &ctx);
        assert!(!set.recommends_speed_conversion);
        assert!(
            set.notes
                .iter()
                .any(|n| n.contains("未自动选择 speed conversion")),
            "{:?}",
            set.notes
        );
    }

    #[test]
    fn mismatched_r_and_avg_lowers_fps_confidence() {
        let s = stream(1920, 1080, "24000/1001", "23/1", None);
        let set = suggest_for(&s, &SubtitleContext::default());
        assert_eq!(
            set.items
                .iter()
                .filter(|i| matches!(i.candidate, Candidate::Fps(_)))
                .count(),
            2
        );
        for i in set
            .items
            .iter()
            .filter(|i| matches!(i.candidate, Candidate::Fps(_)))
        {
            assert_eq!(i.confidence, Confidence::Medium);
        }
    }

    #[test]
    fn upscaling_subtitle_is_high_confidence_downscale_is_low() {
        let s = stream(3840, 2160, "24/1", "24/1", None);
        let up = suggest_for(
            &s,
            &SubtitleContext {
                canvas: Some(Canvas::FHD1080),
                ..Default::default()
            },
        );
        assert_eq!(
            up.find(&Candidate::Canvas(Canvas::UHD2160))
                .unwrap()
                .confidence,
            Confidence::High
        );
        let down = suggest_for(
            &stream(1920, 1080, "24/1", "24/1", None),
            &SubtitleContext {
                canvas: Some(Canvas::UHD2160),
                ..Default::default()
            },
        );
        let d = down.find(&Candidate::Canvas(Canvas::FHD1080)).unwrap();
        assert_eq!(d.confidence, Confidence::Low);
        assert!(!d.auto_checked(), "Low 默认不勾选（16 §4）");
    }

    #[test]
    fn strict_bluray_caps_pgs_canvas_to_2k() {
        // 16 §3 第 4 条 + 09 §6：4K 视频 + 严格蓝光 → 仍建议 1920x1080 PGS
        let s = stream(3840, 2160, "23.976", "23.976", Some("smpte2084"));
        let set = suggest_for(
            &s,
            &SubtitleContext {
                strict_bluray_pgs: true,
                ..Default::default()
            },
        );
        assert_eq!(set.pgs_canvas(), Some(Canvas::FHD1080));
        assert!(
            set.notes.iter().any(|n| n.contains("超出蓝光 PGS 上限")),
            "{:?}",
            set.notes
        );
        // 1080p 视频时保持 1080p
        let s2 = stream(1920, 1080, "25/1", "25/1", None);
        assert_eq!(
            suggest_for(
                &s2,
                &SubtitleContext {
                    strict_bluray_pgs: true,
                    ..Default::default()
                }
            )
            .pgs_canvas(),
            Some(Canvas::FHD1080)
        );
    }

    #[test]
    fn non_16_9_video_does_not_suggest_canvas() {
        // AT-CANVAS-002：2.39:1 的不得被建议成逻辑画布
        let s = stream(1920, 800, "24/1", "24/1", None);
        let set = suggest_for(
            &s,
            &SubtitleContext {
                canvas: Some(Canvas::FHD1080),
                ..Default::default()
            },
        );
        assert!(set.items.iter().all(|i| i.candidate
            != Candidate::Canvas(Canvas {
                width: 1920,
                height: 800
            })));
    }
}
