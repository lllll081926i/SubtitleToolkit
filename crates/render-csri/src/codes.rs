//! 把 CSRI 链的错误码集中定义在一处。
//!
//! ## 为什么要有这个文件
//!
//! `core-model` 的跨 crate 唯一性测试会核对**同一个码不会被两个地方
//! 用来表达不同的事**。那个检查在建本轮时立刻报出了两处：
//! `Render/80` 同时表示"只支持文件开脚本却没给路径"与"路径不是合法 UTF-8"，
//! `Render/83` 同时表示"只有 stream 扩展"与"两条路都没有"。
//!
//! 那两种情形**用户要做的事不同**，共用一个码会让"看错误码判因"失效。
//! 所以集中列在这里，加一个码就要在这里加一行 ——
//! 这样"我该用几号"不会靠在代码里搜数字来回答。
//!
//! ## 与 `Domain::Render` 既有码的关系
//!
//! 本仓 `Render` 域已用：`3, 10, 11, 70–74, 76–78, 80–84, 87, 89, 91`。
//! 其中 71/72/73/74/76/77/78/80/81/82/83/84 是 libass 那条链与本轮 CSRI
//! 早期版本已经占下的；**本轮把有歧义的那几个拆开**，
//! 新码从 92 起 —— 92–99 在本轮之前是空的。
//!
//! 注意 91 是 `cli` 的 "renderer-host 失败"，它把 host 侧的码包在 details
//! 里往上带；不要拿 91 表示 CSRI 自己发现的错。

use core_model::error::Domain;

/// 读不了 provider 文件（不存在、被占用）。**环境问题**，修路径。
pub const READ_FAILED: (Domain, u16) = (Domain::Render, 71);
/// 不是有效的 PE 文件。**拿错了东西**，换一个。
pub const NOT_PE: (Domain, u16) = (Domain::Render, 72);
/// 位数与本进程不符（12 §3）。换 x64 的那一份。
pub const ARCH_MISMATCH: (Domain, u16) = (Domain::Render, 91);
/// 是 CSRI，但缺必需入口。
pub const MISSING_ENTRIES: (Domain, u16) = (Domain::Render, 73);
/// 探测认为它根本**不是** CSRI（可能是 libass / VapourSynth / AviSynth）。
///
/// 与 [`MISSING_ENTRIES`] 分开：那一个是“它像 CSRI 但缺东西”，
/// 这一个是“它是另一种东西”。用户要做的事不同（换 provider vs 换链）。
pub const NOT_CSRI: (Domain, u16) = (Domain::Render, 96);
/// `csri_renderer_default()` 返回空。
pub const NO_RENDERER: (Domain, u16) = (Domain::Render, 79);
/// 只导出 `csri_open_file`，但没有给文件路径。
pub const NEEDS_FILE_PATH: (Domain, u16) = (Domain::Render, 92);
/// 脚本路径不是合法 UTF-8，无法转成 `csri_open_file` 要的 C 字符串。
pub const BAD_SCRIPT_PATH: (Domain, u16) = (Domain::Render, 93);
/// `csri_open_*` 返回空：打不开这段脚本。
pub const SCRIPT_OPEN_FAILED: (Domain, u16) = (Domain::Render, 81);
/// provider 不接受我们支持的任一像素格式。
pub const PIXFMT_REFUSED: (Domain, u16) = (Domain::Render, 82);
/// 只支持 `csri/stream` 扩展，而这条链尚未实现它。
pub const STREAM_EXT_ONLY: (Domain, u16) = (Domain::Render, 94);
/// 既没有 `csri_request_fmt` 也没有 `csri/stream`：不知道怎么驱动。
pub const NO_FMT_PATH: (Domain, u16) = (Domain::Render, 95);
/// 像素缓冲长度与画布不符。
pub const BUFFER_SIZE: (Domain, u16) = (Domain::Render, 74);
/// provider 报了一个我们不认识的像素格式。
pub const UNKNOWN_PIXFMT: (Domain, u16) = (Domain::Render, 75);
/// provider 给的是 planar / packed-YUV 格式，这条链没实现。
pub const PLANAR_PIXFMT: (Domain, u16) = (Domain::Render, 76);
/// provider 没给出像素缓冲（`planes[0]` 为空）。
pub const NULL_PLANE: (Domain, u16) = (Domain::Render, 77);
/// stride 装不下一整行，或 stride 组合溢出。
pub const BAD_STRIDE: (Domain, u16) = (Domain::Render, 78);
/// 无法加载 DLL（Win32 错误码在 details 里）。
pub const LOAD_FAILED: (Domain, u16) = (Domain::Render, 97);
