//! renderer-host：把第三方渲染 DLL 关在另一个进程里（12 §2-§3）。
//!
//! 规格的理由很直白：**"第三方 DLL 永远不进入 GUI 主进程"**。一个 provider 崩溃必须
//! 只终止 host，主进程拿到崩溃码 + provider 日志，然后可以重试（AT-RENDER-001/002）；
//! 而位数不符的 DLL 绝对不能被 `LoadLibrary`（AT-RENDER-003），所以要先读 PE 头再决定路由。
//!
//! 这个 crate 同时提供两侧：
//!
//! * [`client`] —— 主进程侧：起进程、握手、要帧、超时/崩溃取证（`subtitletool-cli`、GUI、CI 用）；
//! * `src/main.rs`（`subtitletool-render-host.exe`）—— host 侧：在这里才真的加载 DLL。
//!
//! 分帧协议见 [`protocol`]：`1 字节标签 + u32 长度 + 负载`，标签 1 是 JSON、2 是原始字节。

pub mod client;
// 12 §3 / 23 §DLL 的 PE 头读取在 `core-model`：主程序与 host 都要在加载 DLL
// 之前判定位数，而**两份实现对同一个 DLL 会给出不同答案**。
// 这里只重新导出，保持既有调用点（`crate::pe::…`）不变。
pub use core_model::pe;
pub mod protocol;

pub use client::{Abandoned, Frame, Host, HostFailure, HostSpec};
pub use pe::{arch_of_bytes, arch_of_file, PeArch};
pub use protocol::{Handshake, Request, Response, API_VERSION};
