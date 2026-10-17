//! 字体服务（17_FONT_MANAGEMENT.md）：字幕**可复现性**用的字体数据库。
//!
//! 定位与边界（照文档，不照方便）：
//!
//! * 做的是"找对 face + 确认覆盖"，**不是**字体编辑器；
//! * 17 §5 的子集在 [`subset`]：`allsorts` 实现 + shaping 等价检查，
//!   恒定可能回退整份；**RGBA 那一半仍未做**（需要真 renderer 与授权语料）；
//! * 17 §7 的 libass 字体注入不在这里（在 `cli` 的 `fontenv`/渲染路径）。
//!
//! 四个模块按数据的流向排：[`db`] 建库 → [`usage`] 从 ASS 抽出真实用量 →
//! [`report`] 把两者对上并给出评级与缺字清单 → [`subset`] 安全地缩小体积。

pub mod attachments;
pub mod cache;
pub mod db;
pub mod env;
pub mod gdi;
pub mod registry;
pub mod report;
pub mod subset;
pub mod usage;

pub use attachments::{
    decode_base64, encode_base64, encode_fonts_section, insert_fonts_section,
    parse as parse_font_attachments, strip_fonts_section, Attachment, MAX_ATTACHMENT_BYTES,
    MAX_TOTAL_BYTES,
};
pub use core_model::{DataDir, DataDirKind};
pub use db::{
    family_of_bytes, AxisInfo, CacheMode, CacheState, FaceRecord, FontDb, NameMatch, ScanNote,
    FONT_CHANGED_AFTER_SCAN, FONT_EXTS,
};
pub use env::{identify, FontEnv, POLICY_VERSION};
pub use gdi::{run as font_cross_check, CrossCheck, GdiFace};
pub use registry::{RegisteredFont, RegistryReport};
pub use report::{evaluate, evaluate_or_rescan, FaceSummary, FontMatch, Verdict};
pub use subset::{
    needs_complex_fallback, subset_bytes, subset_file, subset_path, FallbackReason, Outcome,
    SubsetOutcome,
};
pub use usage::{scan_document, scan_text, FaceRequest};

/// 这份字节是不是一个可解析的字体/字体集合？
///
/// 交付层复制字体之后要用它自检（"拷过去的东西必须还能读"），
/// 而判据必须来自解析器本身，不能是"文件大小一样"。
pub fn is_parseable_font(bytes: &[u8]) -> bool {
    read_fonts::FileRef::new(bytes).is_ok()
}
