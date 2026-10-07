//! 字体服务（17_FONT_MANAGEMENT.md）：字幕**可复现性**用的字体数据库。
//!
//! 定位与边界（照文档，不照方便）：
//!
//! * 做的是"找对 face + 确认覆盖"，**不是**字体编辑器；
//! * 17 §5 的 subset 不在这个 crate 里（要 `allsorts` 与 corpus 才敢说安全，
//!   见 ISSUES AG2）；17 §7 的 libass 字体注入同理未做；
//! * 17 §2 说权威是 DirectWrite。这里给的是"读表"视角：`name`/`OS2`/`cmap`/`fvar`，
//!   覆盖注册字体与 DirectWrite fallback 链的部分未做（AG1）。
//!
//! 三个模块按数据的流向排：[`db`] 建库 → [`usage`] 从 ASS 抽出真实用量 →
//! [`report`] 把两者对上并给出评级与缺字清单。

pub mod attachments;
pub mod cache;
pub mod db;
pub mod env;
pub mod registry;
pub mod report;
pub mod usage;

pub use attachments::{
    decode_base64, parse as parse_font_attachments, Attachment, MAX_ATTACHMENT_BYTES,
    MAX_TOTAL_BYTES,
};
pub use db::{
    family_of_bytes, AxisInfo, CacheMode, CacheState, FaceRecord, FontDb, NameMatch, ScanNote,
    FONT_CHANGED_AFTER_SCAN, FONT_EXTS,
};
pub use env::{identify, FontEnv, POLICY_VERSION};
pub use registry::{RegisteredFont, RegistryReport};
pub use report::{evaluate, evaluate_or_rescan, FaceSummary, FontMatch, Verdict};
pub use usage::{scan_document, scan_text, FaceRequest};

/// 这份字节是不是一个可解析的字体/字体集合？
///
/// 交付层复制字体之后要用它自检（"拷过去的东西必须还能读"），
/// 而判据必须来自解析器本身，不能是"文件大小一样"。
pub fn is_parseable_font(bytes: &[u8]) -> bool {
    read_fonts::FileRef::new(bytes).is_ok()
}
