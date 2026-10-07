//! render-libass（12 §1A）：把 libass 交出的位图链变成完整 16:9 RGBA 画布。
//!
//! 分工：
//! - [`bitmap`]：纯 Rust 的合成语义（顺序、覆盖率、alpha、裁剪、量化记账）。
//!   这一层不依赖 native，能现在就测；
//! - [`ffi`]：结构体镜像 + 运行时符号探测。native 依赖缺失时它给出可报告的错误，
//!   而不是让整棵工程编译不过（12 §进程边界、23 §安全）。
//!
//! `provider` 是真正的 `ass_render_frame` 封装（运行时加载 + 内存输入 + 双尺寸配置）；
//! 它只在 native 依赖落地后可用（本机：vcpkg 的 `ass-9.dll`），因此相关测试带明确的 SKIP 条件。

pub mod bitmap;
pub mod ffi;
pub mod provider;

pub use bitmap::composite;
pub use bitmap::unpack_color;
pub use bitmap::ImageDesc;
pub use bitmap::ImageKind;
pub use ffi::probe;
pub use ffi::walk_chain;
pub use ffi::AssImage;
pub use ffi::LibHandle;
pub use ffi::ProbeReport;
pub use ffi::LIBASS_FONT_SYMBOLS;
pub use ffi::LIBASS_REQUIRED_SYMBOLS;
pub use ffi::MAX_IMAGE_CHAIN;
pub use provider::render;
pub use provider::InjectedFont;
pub use provider::Provider;
pub use provider::Rendered;
pub use provider::Script;
