//! BDN（Blu-ray Disc Native：XML + PNG 交换格式）读写层。
//!
//! 落地 13_PGS_BDN_BLURAY.md §7，并针对 40_SOURCE_CODE_AUDIT.md 记录的旧闭源工具三个缺陷
//! 各设一道闸（都有测试反例）：
//! 1. **单一时间源**：InTC/OutTC 只由 `core_model::time::Time::to_bdn_timecode(fps)` 生成
//!    （10 §7）。本 crate 内不出现任何手写的"秒/帧"拆分，也不允许两处用不同舍入源。
//! 2. **声明尺寸 = PNG 实际尺寸**：`Width`/`Height` 只能是 PNG 的 IHDR 尺寸；`min_axis` 只做
//!    过滤、绝不抬尺寸（旧工程把它当钳制，导致 XML 声明与 PNG 不符）。
//! 3. **写出前必须已排序**：events 未按 `start` 严格递增时 [`write`] 直接拒绝并给位置，
//!    绝不偷偷排序（排序策略属于 task-engine）。
//!
//! BDN 是交换格式，不等于最终 PGS strict（13 §7）；合规信息通过 [`loss_of`] 回到 Loss Model
//! （08 §4），本 crate 不伪造 "Blu-ray compliant"。

mod build;
mod limits;
mod loss;
mod model;
mod parse;
mod png;
mod write;

pub use build::{build_document, build_document_on_canvas, build_document_with_loss};
pub use limits::{
    Limits, DEFAULT_LIMITS, DEPTH_HARD_MAX, DOC_BYTES_HARD_MAX, EVENT_COUNT_HARD_MAX,
    NAME_LEN_HARD_MAX, TEXT_LEN_HARD_MAX,
};
pub use loss::loss_of;
pub use model::{BdnDocument, BdnEvent, BdnFrame, BdnGraphic, BdnMeta, BdnOptions};
pub use parse::{parse, parse_bytes, parse_with};
pub use png::{check_png_size, png_size, validate_dir, PngProblem, PngSize, Report};
pub use write::write;

/// 读 PNG 头部一次取够 IHDR：签名 8 + length 4 + "IHDR" 4 + width 4 + height 4 = 24，
/// 再加 bit depth / color type / compression / filter / interlace 共 9 字节 → 33。
pub const PNG_HEADER_BYTES: usize = 33;
