//! CSRI provider 链（12 §1B）。
//!
//! 本 crate 只做三件事：
//!
//! 1. **装订** C ABI（[`ffi`]）—— 结构体布局用 `offset_of!` 编译期核对；
//! 2. **加载**用户提供的 CSRI provider（[`provider`]）—— 运行时装订，
//!    不在编译期链接；
//! 3. **把 CSRI 的像素交给 `image-core`**（[`frame`]）—— BGRA/BGR_ → RGBA8
//!    的转换点只在这里，不混进合成层。
//!
//! ## 许可边界（重要，不是脚注）
//!
//! 12 §1B 允许这条链，21 §6 也明说 VSFilterMod **"默认设计为用户提供/可选外部
//! renderer provider，通过 renderer-host/CSRI 调用"**。但两件事必须分清：
//!
//! - **写一个 loader 是允许的** —— 它不包含任何 GPL 代码，与现有 libass 路径同构
//!   （运行时 `LoadLibraryExW`，DLL 由用户提供）。
//! - **把 GPL 的 DLL 打进发行物是另一件事** —— 21 §1 说"进程边界不自动解决
//!   分发许可义务"，所以"我们在另一个进程里跑它"**不构成**许可上的免责。
//!
//! 因此本 crate 既不携带也不查找任何具体 DLL：路径必须由用户显式给出。
//!
//! ## 主进程不加载 provider（ADR-005 / 12 §2）
//!
//! 这个 crate 只被 `renderer-host` 依赖。第三方渲染 DLL 永远不进 CLI/GUI 进程。
//!
//! ## 待办（已登记，不是遗漏）
//!
//! `render-libass` 里还留着一份自己的 `LibHandle`（那边先用着，因为它是
//! 已验证的代码，不为了去重而在同一轮里动它）。两份实现最终应当并到
//! [`core_model::dllload`]。在那之前，**改动加载策略要同时改两处** ——
//! 这一点写在这里，免得下次只改一半。

pub mod codes;
pub mod ffi;
pub mod frame;
pub mod provider;

pub use frame::CsriRgba;
pub use provider::{CsriProvider, CsriReport};
