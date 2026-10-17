//! 分层校验（13 §6 的七层里，这里能跑的四层 + 明确的"未覆盖"声明）。
//!
//! 关键立场：
//! - 一次给出全部发现（不是遇错即停），每条都带严重级别、位置与修复建议（22 §错误模型）；
//! - 允许 Strict/compliant 标签需要三个条件：无 error/fatal、按 Strict 跑过、且没有未覆盖层
//!   （`ValidationReport::strict_badge_allowed`，39 门禁）；
//! - 码率只做**测量**，绝不声称编码会把流压到预算内（13 §5.1 的命名分离）；
//! - 解码器缓冲模型与设备语料不在本 crate 假通过，缺失时以 warning 显式声明。

pub mod ass;
pub mod decoder_model;
pub mod pgs;
pub mod report;

pub use ass::{ass as ass_document, time_pairs};
pub use pgs::{
    lifecycle, peak_bitrate, peak_bitrate_findings, profile, structure, timeline, to_error, worst,
    BitrateArgs, BitrateMeasurement, ProfileWant, BLURAY_PGS_CANVASES,
};
pub use report::{Finding, Severity, ValidationReport};

/// PGS 全流程：1/2/3/4 层 + 可选码率预算。
pub fn pgs(
    segments: &[codec_pgs::segment::Segment],
    want: &ProfileWant,
    grid: Option<core_model::time::Fps>,
    bitrate: Option<&BitrateArgs>,
) -> ValidationReport {
    let mut r = pgs::pgs_all(segments, want, grid);
    if let Some(args) = bitrate {
        r.merge(peak_bitrate_findings(&peak_bitrate(segments, args)));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_pgs::encoder::{encode, EncodeOptions, Frame, FrameObject};
    use core_model::profile::ValidationLevel;

    fn segs() -> Vec<codec_pgs::segment::Segment> {
        let obj = FrameObject {
            rgba: vec![255u8; 8 * 2 * 4],
            width: 8,
            height: 2,
            x: 10,
            y: 10,
            forced: false,
        };
        let (s, _) = encode(
            &[Frame::new(90_000, vec![obj]), Frame::erase(180_000)],
            &EncodeOptions::default(),
        )
        .unwrap();
        let bytes = codec_pgs::segment::write_stream(&s).unwrap();
        codec_pgs::segment::parse_stream(&bytes).unwrap()
    }

    #[test]
    fn top_level_pgs_reports_bitrate_when_asked() {
        let want = ProfileWant {
            level: ValidationLevel::Basic,
            claim_compliant: false,
        };
        let without = pgs(&segs(), &want, None, None);
        assert!(!without.has_check("PGS.BTR.00"));
        let with = pgs(
            &segs(),
            &want,
            None,
            Some(&BitrateArgs {
                limit_bps: 1,
                ..BitrateArgs::default()
            }),
        );
        assert!(with.has_check("PGS.BTR.01"), "{:?}", with.text_lines());
        assert!(!with.no_blocking_findings());
    }
}
