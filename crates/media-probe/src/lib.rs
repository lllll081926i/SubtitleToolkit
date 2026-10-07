//! 目标视频辅助（16_MEDIA_PROBE_SMART_ASSIST.md）。
//! 定位：只填候选值 + 给证据，永不自动改时间轴（16 §3、AT-PROBE-001）。

mod fields;
mod runner;
mod suggest;

pub use fields::{parse_video_streams, ColorMeta, VideoStream};
pub use runner::{
    build_command, default_args, FfprobeIdentity, ProbeError, ProbeRunner, SystemProbe,
};
pub use suggest::{suggest_for, Candidate, Confidence, SubtitleContext, Suggestion, SuggestionSet};
