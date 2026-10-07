//! image-core（07 §组件 / 09 §2 §3 / 13 §7）：RGBA 位图层 + PNG 读写。
//!
//! 这一层的存在理由：
//! - **bbox 裁剪不能移动内容**（09 §3）：[`rgba::Patch`] 始终携带完整画布坐标，
//!   [`rgba::Patch::place_back`] 必须还原出逐像素相同的合成结果。
//! - **PNG 必须是真压缩**（I001/B001）：[`png::encode`] 走 flate2 deflate，
//!   并逐行挑 filter；写 stored（未压缩）块会让 BDN 交换目录体积失控。
//! - **声明尺寸 = PNG 实际尺寸**（13 §7 / AT-BDN-001）：[`png::header`] 只读 IHDR，
//!   供 `codec-bdn` 与 XML 的 Width/Height 对齐。

mod png;
mod rgba;

pub use png::decode;
pub use png::encode;
pub use png::header;
pub use png::Header as PngHeader;
pub use png::Image as PngImage;
pub use png::Level;
pub use png::MAX_DECOMPRESSED_BYTES;
pub use png::SIGNATURE;
pub use rgba::is_transparent;
pub use rgba::pixel_len;
pub use rgba::BoundingBox;
pub use rgba::Frame;
pub use rgba::Patch;
