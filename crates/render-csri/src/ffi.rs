//! CSRI（common subtitle renderer interface）的 C ABI 与运行时装订（12 §1B）。
//!
//! ## 为什么这是一个**独立**的链，而不是 libass 的变体
//!
//! 12 §1 把 renderer 分成三条**互相独立**的能力链。CSRI 是其中的 B 条：
//! 它是一个**独立的 provider ABI**，不是"另一种 libass"。所以：
//!
//! - 导出名不同（`csri_*` vs `ass_*`），[`super::probe`] 按导出表分流；
//! - 像素格式不同（CSRI 给 BGRA/BGR_，libass 那条链给 RGBA8），
//!   转换点写在 [`crate::frame`] 里，不混进合成层；
//! - 生命周期不同（`csri_open_*` → `csri_request_fmt` → `csri_render` → `csri_close`）。
//!
//! ## 结构体布局**不是猜的**
//!
//! 下面每个结构体的字段序与偏移都对着上游 `csri.h`（Aegisub vendored 版本）
//! 逐行读出来的，并且用 `offset_of!` 做了**编译期断言**。
//!
//! 为什么必须机器核对而不是写注释：17 §2 的 GDI 那次，`TEXTMETRICW` 里
//! `tmFirstChar`…`tmBreakChar` 是 `WCHAR`(u16) 而我写成了 `u8`，
//! 于是 `tmPitchAndFamily` 的偏移差了 4 字节、恒读 0，
//! **所有字体都被当成非 TrueType**。而那一处**纯构造数据的单元测试看不见** ——
//! 测试按我写的（错的）布局造数据，自然对得上。所以偏移必须是编译期断言，
//! 一处写错就编不过。
//!
//! ## 主进程绝不加载 provider（ADR-005 / 12 §2）
//!
//! 本模块只被 `renderer-host` 用。第三方渲染 DLL 永远不进 CLI/GUI 进程 ——
//! 这是 12 §2 的进程隔离要求，也是"provider 崩了主程序还能活"的前提。

use std::ffi::c_void;

use core_model::error::{Domain, Result, StkError};

// ---------------------------------------------------------------------------
// 像素格式（csri.h 的 enum csri_pixfmt）
// ---------------------------------------------------------------------------

/// CSRI 的像素格式枚举。**值必须与头文件逐字一致** —— 它会被交给 provider。
///
/// ## 第一版把两个值写错了（实测抳出）
///
/// 我最初把 `CSRI_F_BGR_` 写成 `0x101`。实际上头文件是：
///
/// ```c
/// CSRI_F_RGB_ = 0x100,
/// CSRI_F__RGB,      // 0x101
/// CSRI_F_BGR_,      // 0x102  ← Windows "RGB32"
/// CSRI_F__BGR,      // 0x103
/// ```
///
/// 它们从锚点**逐项递增**，而中间的几个没有显式写值 ——
/// 我把“相邻”当成了“同一个数”。后果是把 `CSRI_F__RGB`(0x101)
/// 当成 `CSRI_F_BGR_` 交出去。
///
/// 这个错**不会崩也不会报**：provider 会照 0x101 解释内存，
/// 而 0x101 也是合法格式，于是图出得来、只是通道或步长不对。
/// 抳出它的是一次**逐个试真 provider 接受哪种格式**的探针：
/// 本机 VSFilterMod 对 0x102 返回 0（接受）、对 0x101 返回 -1（拒绝）。
///
/// 教训与 `CSRI_REQUIRED_EXPORTS` 那次一样：**枚举的数值不能从
/// 拼出来的文本里抄** —— 要对着头文件的完整原文读，或者拿真 provider 试。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum PixFmt {
    Rgba = 0,
    Argb = 1,
    Bgra = 2,
    Abgr = 3,
    /// `CSRI_F_RGB_` = 0x100
    RgbUnderscore = 0x100,
    /// `CSRI_F__RGB` = 0x101
    UnderscoreRgb = 0x101,
    /// `CSRI_F_BGR_` = 0x102 —— Windows "RGB32"
    BgrUnderscore = 0x102,
    /// `CSRI_F__BGR` = 0x103
    UnderscoreBgr = 0x103,
    /// `CSRI_F_RGB` = 0x200
    Rgb = 0x200,
    /// `CSRI_F_BGR` = 0x201 —— Windows "RGB24"
    Bgr = 0x201,
    Ayuv = 0x1000,
    Yuva = 0x1001,
    Yvua = 0x1002,
    Yuy2 = 0x1100,
    /// planar YUV 2x2 + alpha plane
    Yv12a = 0x2011,
    /// planar YUV 2x2
    Yv12 = 0x2111,
}

impl PixFmt {
    pub fn label(self) -> &'static str {
        match self {
            PixFmt::Rgba => "RGBA",
            PixFmt::Argb => "ARGB",
            PixFmt::Bgra => "BGRA",
            PixFmt::Abgr => "ABGR",
            PixFmt::RgbUnderscore => "RGB_ (RGB32)",
            PixFmt::UnderscoreRgb => "_RGB",
            PixFmt::BgrUnderscore => "BGR_ (RGB32)",
            PixFmt::UnderscoreBgr => "_BGR",
            PixFmt::Rgb => "RGB (RGB24)",
            PixFmt::Bgr => "BGR (RGB24)",
            PixFmt::Ayuv => "AYUV",
            PixFmt::Yuva => "YUVA",
            PixFmt::Yvua => "YVUA",
            PixFmt::Yuy2 => "YUY2",
            PixFmt::Yv12a => "YV12A",
            PixFmt::Yv12 => "YV12",
        }
    }

    /// 每像素字节数；planar / packed-YUV 格式返回 `None`
    /// （不能用"每像素 N 字节"描述）。
    pub fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            PixFmt::Rgba
            | PixFmt::Argb
            | PixFmt::Bgra
            | PixFmt::Abgr
            | PixFmt::RgbUnderscore
            | PixFmt::UnderscoreRgb
            | PixFmt::BgrUnderscore
            | PixFmt::UnderscoreBgr => Some(4),
            PixFmt::Rgb | PixFmt::Bgr => Some(3),
            _ => None,
        }
    }

    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            PixFmt::Rgba
                | PixFmt::Argb
                | PixFmt::Bgra
                | PixFmt::Abgr
                | PixFmt::Ayuv
                | PixFmt::Yuva
                | PixFmt::Yvua
                | PixFmt::Yv12a
        )
    }

    /// 把 provider 报来的原始值转成我们认识的格式。
    ///
    /// **公开**：`frame.rs` 要拿它核对，而不是自己再写一份映射 ——
    /// 两份映射会对同一个数字给出不同答案。
    pub fn from_raw(v: i32) -> Option<PixFmt> {
        Some(match v {
            0 => PixFmt::Rgba,
            1 => PixFmt::Argb,
            2 => PixFmt::Bgra,
            3 => PixFmt::Abgr,
            0x100 => PixFmt::RgbUnderscore,
            0x101 => PixFmt::UnderscoreRgb,
            0x102 => PixFmt::BgrUnderscore,
            0x103 => PixFmt::UnderscoreBgr,
            0x200 => PixFmt::Rgb,
            0x201 => PixFmt::Bgr,
            0x1000 => PixFmt::Ayuv,
            0x1001 => PixFmt::Yuva,
            0x1002 => PixFmt::Yvua,
            0x1100 => PixFmt::Yuy2,
            0x2011 => PixFmt::Yv12a,
            0x2111 => PixFmt::Yv12,
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// 结构体（与 csri.h 逐字段对应）
// ---------------------------------------------------------------------------

/// `struct csri_fmt`：要交给渲染器的格式与画布。
///
/// 上游声明（逐字）：
/// ```c
/// struct csri_fmt {
///     enum csri_pixfmt pixfmt;   // c_int
///     unsigned width;            // u32
///     unsigned height;           // u32
/// };
/// ```
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CsriFmt {
    pub pixfmt: i32,
    pub width: u32,
    pub height: u32,
}

impl CsriFmt {
    pub fn new(pixfmt: PixFmt, width: u32, height: u32) -> CsriFmt {
        CsriFmt {
            pixfmt: pixfmt as i32,
            width,
            height,
        }
    }
}

/// `struct csri_frame`：一帧的像素缓冲。
///
/// 上游声明（逐字）：
/// ```c
/// struct csri_frame {
///     enum csri_pixfmt pixfmt;   // c_int
///     unsigned char *planes[4];  // 注意是 **4**，不是 3
///     ptrdiff_t strides[4];
/// };
/// ```
///
/// 两个容易写错的点，都是从头文件读出来的而不是推的：
///
/// 1. **数组长度是 4**。写 3 会让 `strides` 整体前移一个指针宽度，
///    而 provider 会往错的地方写 stride —— 那是越界写。
/// 2. **`strides` 可以为负**。头文件写明"topmost line always comes first"，
///    但应用**自己**决定怎么放：x264 的调用示例用的是
///    `planes[0] = canvas + (height-1)*width*4; strides[0] = -(width*4);`
///    即**从下往上**存。所以它是 `isize` 而不是 usize。
#[repr(C)]
pub struct CsriFrame {
    pub pixfmt: i32,
    pub planes: [*mut u8; 4],
    pub strides: [isize; 4],
}

/// x86/x64 下的期望布局。**编译期断言** —— 见模块头部的说明。
const _: () = {
    let pointer_size = std::mem::size_of::<usize>();
    let planes_offset = 4_usize.div_ceil(pointer_size) * pointer_size;
    assert!(std::mem::offset_of!(CsriFrame, pixfmt) == 0);
    assert!(std::mem::offset_of!(CsriFrame, planes) == planes_offset);
    assert!(std::mem::offset_of!(CsriFrame, strides) == planes_offset + 4 * pointer_size);
    assert!(std::mem::size_of::<CsriFrame>() == planes_offset + 8 * pointer_size);
    // csri_fmt 没有填充问题，但一样钉住
    assert!(std::mem::offset_of!(CsriFmt, pixfmt) == 0);
    assert!(std::mem::offset_of!(CsriFmt, width) == 4);
    assert!(std::mem::offset_of!(CsriFmt, height) == 8);
    assert!(std::mem::size_of::<CsriFmt>() == 12);
};

/// `struct csri_openflag`：可选的开脚本标志链表。
///
/// 我们不构造它（一律传 `null`），但要把类型定义对，因为函数签名里出现它。
/// 上游允许"渲染器必须忽略不认识的 flag"，而我们一个都不传。
#[repr(C)]
pub struct CsriOpenFlag {
    pub name: *const i8,
    pub data: CsriVarData,
    pub next: *mut CsriOpenFlag,
}

/// `union csri_vardata` —— 大小取最大的成员。
///
/// 上游的联合体含 `void *fp` / `int i` / `const void *p` 等；
/// 在 x64 上最大成员是指针，所以大小是 8。
#[repr(C)]
#[derive(Clone, Copy)]
pub union CsriVarData {
    pub p: *const c_void,
    pub fp: *mut c_void,
    pub i: i32,
    pub _pad: [u8; 8],
}

// ---------------------------------------------------------------------------
// 函数指针类型
// ---------------------------------------------------------------------------

/// `csri_rend` / `csri_inst` 都是不透明句柄（头文件里 `typedef void`）。
pub type CsriRend = *mut c_void;
pub type CsriInst = *mut c_void;

pub type FnOpenFile = unsafe extern "C" fn(
    renderer: CsriRend,
    filename: *const i8,
    flags: *mut CsriOpenFlag,
) -> CsriInst;
pub type FnOpenMem = unsafe extern "C" fn(
    renderer: CsriRend,
    data: *const c_void,
    length: usize,
    flags: *mut CsriOpenFlag,
) -> CsriInst;
pub type FnClose = unsafe extern "C" fn(inst: CsriInst);
pub type FnRequestFmt = unsafe extern "C" fn(inst: CsriInst, fmt: *const CsriFmt) -> i32;
pub type FnRender = unsafe extern "C" fn(inst: CsriInst, frame: *mut CsriFrame, time: f64);
pub type FnQueryExt = unsafe extern "C" fn(rend: CsriRend, extname: *const i8) -> *mut c_void;
pub type FnRendererDefault = unsafe extern "C" fn() -> CsriRend;
pub type FnRendererByname = unsafe extern "C" fn(name: *const i8, specific: *const i8) -> CsriRend;
pub type FnRendererInfo = unsafe extern "C" fn(rend: CsriRend) -> *mut CsriInfo;

/// `struct csri_info` 的前几个字段。
///
/// 我们只读 `name`/`longname`/`specific` 三个 `const char *` 用来做报告 ——
/// **不完整定义**它，因为后面的 `version` 等字段要按上游逐字节对齐，
/// 而按需读前三个字段足够且更不容易错。
///
/// 「按需读前缀」是一个刻意的选择：整套结构体的偏移都对上了才能安全读它，
/// 而我们对报告只需要名字。
#[repr(C)]
pub struct CsriInfoPrefix {
    pub name: *const i8,
    pub longname: *const i8,
    pub specific: *const i8,
}

/// `csri_info` 的别名（我们只用前缀，见上）。
pub type CsriInfo = CsriInfoPrefix;

// ---------------------------------------------------------------------------
// 装订（binding）：加载 DLL 并取符号
// ---------------------------------------------------------------------------

/// 必需符号。与 [`render_api::capability::CSRI_REQUIRED_EXPORTS`] **同源**。
///
/// 两份列表会分叉，而分叉的后果是"探测说能用、加载时找不到" —— 那时
/// 用户看到的是一条自相矛盾的报错。有一条用例直接比这两个常量。
const REQUIRED: [&str; render_api::capability::CSRI_REQUIRED_EXPORTS.len()] =
    render_api::capability::CSRI_REQUIRED_EXPORTS;
/// open 入口二选一。
const OPEN_VARIANTS: [&str; render_api::capability::CSRI_OPEN_VARIANTS.len()] =
    render_api::capability::CSRI_OPEN_VARIANTS;

/// 已解析的函数表。
///
/// 每个字段要么是**必需**（构造时缺了就失败），要么是 `Option`（可降级）。
pub struct CsriApi {
    pub renderer_default: Option<FnRendererDefault>,
    pub renderer_byname: Option<FnRendererByname>,
    pub renderer_info: Option<FnRendererInfo>,
    pub open_file: Option<FnOpenFile>,
    pub open_mem: Option<FnOpenMem>,
    pub request_fmt: Option<FnRequestFmt>,
    pub render: FnRender,
    pub close: FnClose,
    pub query_ext: Option<FnQueryExt>,
}

impl CsriApi {
    /// 从一组「名字 → 地址」解析。
    ///
    /// 抽成独立函数是为了**可测试**：真正读 DLL 的那部分（[`super::Provider::open`]）
    /// 需要一个真 DLL，而这里的判定逻辑不需要。
    ///
    /// # Safety
    ///
    /// `lookup` 返回的地址必须真的对应那个符号名的正确签名。
    /// 类型对不上时调用它就是 UB —— 所以调用方必须先用
    /// [`render_api::capability::probe`] 按导出名确认过这是 CSRI provider。
    pub unsafe fn from_lookup(
        mut lookup: impl FnMut(&str) -> Option<*mut c_void>,
    ) -> Result<CsriApi> {
        // 先查必需项，缺谁就报谁 —— 一次报全，不要让用户改一轮跑一轮。
        let missing: Vec<&str> = REQUIRED
            .iter()
            .copied()
            .filter(|n| lookup(n).is_none())
            .collect();
        let has_open = OPEN_VARIANTS.iter().any(|n| lookup(n).is_some());
        if !missing.is_empty() || !has_open {
            let mut all = missing;
            if !has_open {
                all.push("csri_open_file（或 csri_open_mem）");
            }
            return Err(
                StkError::simple(Domain::Render, 73, "CSRI provider 缺少必需入口")
                    .with_detail(format!("缺：{}", all.join("、")))
                    .with_action(
                        "换一个 CSRI provider，或用 `provider probe <dll>` 看它到底是什么",
                    ),
            );
        }

        // SAFETY：调用方保证地址与签名匹配；未知架构下不走到这里
        // （`Provider::open` 已经做过位数与能力两道检查）。
        Ok(CsriApi {
            // `csri_renderer_default` 在 REQUIRED 里，所以这里是 Some。
            renderer_default: lookup("csri_renderer_default")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnRendererDefault>(p) }),
            renderer_byname: lookup("csri_renderer_byname")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnRendererByname>(p) }),
            renderer_info: lookup("csri_renderer_info")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnRendererInfo>(p) }),
            open_file: lookup("csri_open_file")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnOpenFile>(p) }),
            open_mem: lookup("csri_open_mem")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnOpenMem>(p) }),
            // `csri_request_fmt` 是**可选**的：走 `csri_stream_ext` 的 provider
            // 不导出它（实测 VSFilterMod 就没有）。所以它不是必需项。
            request_fmt: lookup("csri_request_fmt")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnRequestFmt>(p) }),
            render: unsafe {
                std::mem::transmute::<*mut c_void, FnRender>(lookup("csri_render").unwrap())
            },
            close: unsafe {
                std::mem::transmute::<*mut c_void, FnClose>(lookup("csri_close").unwrap())
            },
            query_ext: lookup("csri_query_ext")
                .map(|p| unsafe { std::mem::transmute::<*mut c_void, FnQueryExt>(p) }),
        })
    }

    /// 这个 provider 能走哪条设格式的路。
    ///
    /// 12 §1B 说"只有确实暴露标准 CSRI / `csri_stream_ext` 的 provider 才走这条链"。
    /// 两条路在 **`csri_request_fmt`** 这一点上分开：
    ///
    /// - 有 `csri_request_fmt` → 旧式：设格式后 `csri_render` 画帧。
    /// - 没有它但有 `csri_query_ext` → 新式：通过扩展接口拿 stream 能力。
    ///
    /// 两者都没有时我们**不做猜测**：那意味着我们不知道该怎么驱动它。
    pub fn fmt_path(&self) -> FmtPath {
        if self.request_fmt.is_some() {
            FmtPath::RequestFmt
        } else if self.query_ext.is_some() {
            FmtPath::StreamExtOnly
        } else {
            FmtPath::Unknown
        }
    }
}

/// 设格式的两条路径（见 [`CsriApi::fmt_path`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FmtPath {
    /// `csri_request_fmt` 可用（旧式，也是标准路径）
    RequestFmt,
    /// 靠 `csri_query_ext` 的扩展接口（新式）
    StreamExtOnly,
    /// 两条都没有：不知道怎么驱动
    Unknown,
}

/// CSRI 扩展里我们关心的那个：12 §1B 点名的 stream 接口。
pub const CSRI_EXT_STREAM: &[u8] = b"csri/stream\0";

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: usize) -> *mut c_void {
        // 非空且互不相同的假地址：只为走通 `from_lookup` 的路径，
        // **不会被调用**（调用才是 UB）。
        (n + 0x1000) as *mut c_void
    }

    /// 用一组导出名构造一个只回答"有没有"的 lookup。
    fn lookup_of<'a>(names: &'a [&'a str]) -> impl FnMut(&str) -> Option<*mut c_void> + 'a {
        move |n: &str| names.iter().position(|x| *x == n).map(addr)
    }

    #[test]
    fn struct_offsets_match_the_upstream_header() {
        // 这几条与模块顶部的编译期断言重复，但**故意重复**：
        // 编译期断言给出"编不过"，运行时断言给出"哪个字段错"。
        // 排查时后者更有用，而前者保证它不可能被绕过。
        assert_eq!(std::mem::offset_of!(CsriFrame, pixfmt), 0);
        let pointer_size = std::mem::size_of::<usize>();
        let planes_offset = 4_usize.div_ceil(pointer_size) * pointer_size;
        assert_eq!(std::mem::offset_of!(CsriFrame, planes), planes_offset);
        assert_eq!(
            std::mem::offset_of!(CsriFrame, strides),
            planes_offset + 4 * pointer_size
        );
        assert_eq!(
            std::mem::size_of::<CsriFrame>(),
            planes_offset + 8 * pointer_size
        );

        // planes/strides 是**4** 个元素。写 3 会让 strides 整体前移一个指针宽。
        let f = CsriFrame {
            pixfmt: 0,
            planes: [std::ptr::null_mut(); 4],
            strides: [0; 4],
        };
        assert_eq!(f.planes.len(), 4);
        assert_eq!(f.strides.len(), 4);
    }

    #[test]
    fn csri_fmt_layout() {
        assert_eq!(std::mem::size_of::<CsriFmt>(), 12);
        let f = CsriFmt::new(PixFmt::Bgra, 1920, 1080);
        assert_eq!(f.pixfmt, 2);
        assert_eq!(f.width, 1920);
        assert_eq!(f.height, 1080);
    }

    /// 像素格式的**数值**必须与头文件一致。
    ///
    /// 这条不是在测"枚举有几个值"：数值错了会被交给 provider，
    /// 而它会照着错的数字解释内存 —— 表现为通道互换或越界。
    ///
    /// 而且它**真错过一次**：我最初把 `CSRI_F_BGR_` 写成 0x101，
    /// 而头文件里 0x101 是 `CSRI_F__RGB`、0x102 才是 `CSRI_F_BGR_`。
    /// 下面是逐个抄头文件原文核对的。
    #[test]
    fn pixel_format_values_match_the_header() {
        assert_eq!(PixFmt::Rgba as i32, 0);
        assert_eq!(PixFmt::Argb as i32, 1);
        assert_eq!(PixFmt::Bgra as i32, 2);
        assert_eq!(PixFmt::Abgr as i32, 3);
        // 这一组从 0x100 起**逐项递增**，中间几个没有显式写值
        assert_eq!(PixFmt::RgbUnderscore as i32, 0x100);
        assert_eq!(PixFmt::UnderscoreRgb as i32, 0x101);
        assert_eq!(PixFmt::BgrUnderscore as i32, 0x102);
        assert_eq!(PixFmt::UnderscoreBgr as i32, 0x103);
        assert_eq!(PixFmt::Rgb as i32, 0x200);
        assert_eq!(PixFmt::Bgr as i32, 0x201);
        // 回归：这两个不能相等 —— 相等就意味着我又把“相邻”当成了“同一个”
        assert_ne!(PixFmt::UnderscoreRgb as i32, PixFmt::BgrUnderscore as i32);
        assert_eq!(PixFmt::Ayuv as i32, 0x1000);
        assert_eq!(PixFmt::Yuy2 as i32, 0x1100);
        assert_eq!(PixFmt::Bgra.bytes_per_pixel(), Some(4));
        assert_eq!(PixFmt::BgrUnderscore.bytes_per_pixel(), Some(4));
        assert_eq!(PixFmt::Bgr.bytes_per_pixel(), Some(3));
        assert_eq!(PixFmt::Yv12.bytes_per_pixel(), None);
        assert_eq!(PixFmt::Yuy2.bytes_per_pixel(), None);
        assert!(PixFmt::Bgra.has_alpha());
        assert!(!PixFmt::Bgr.has_alpha());
        // `from_raw` 要与枚举值自洽（两份映射会对同一数字给出不同答案）
        for f in [
            PixFmt::Bgra,
            PixFmt::BgrUnderscore,
            PixFmt::UnderscoreRgb,
            PixFmt::Bgr,
            PixFmt::Yv12,
        ] {
            assert_eq!(PixFmt::from_raw(f as i32), Some(f), "{f:?} 不能往返");
        }
        assert_eq!(PixFmt::from_raw(0x7777), None, "未知值不能猜");
    }

    #[test]
    fn a_complete_provider_yields_the_old_style_path() {
        let api = unsafe {
            CsriApi::from_lookup(lookup_of(&[
                "csri_renderer_default",
                "csri_open_file",
                "csri_open_mem",
                "csri_request_fmt",
                "csri_render",
                "csri_close",
                "csri_query_ext",
            ]))
            .unwrap()
        };
        assert_eq!(api.fmt_path(), FmtPath::RequestFmt);
        assert!(api.open_file.is_some());
        assert!(api.open_mem.is_some());
        assert!(api.query_ext.is_some());
    }

    /// **真实 VSFilterMod 的导出集**：没有 `csri_request_fmt`。
    ///
    /// 它走 `csri_query_ext` / `csri_stream_ext` 那条路。
    /// 如果实现要求 `csri_request_fmt`，这条用例会失败 ——
    /// 而那正是"把规格点名的 provider 排除掉"。
    #[test]
    fn the_real_vsfiltermod_export_set_loads_as_stream_ext() {
        let api = unsafe {
            CsriApi::from_lookup(lookup_of(&[
                "csri_close",
                "csri_open_file",
                "csri_open_mem",
                "csri_query_ext",
                "csri_render",
                "csri_renderer_byname",
                "csri_renderer_default",
                "csri_renderer_info",
            ]))
            .unwrap()
        };
        assert_eq!(api.fmt_path(), FmtPath::StreamExtOnly);
        assert!(api.request_fmt.is_none());
        assert!(api.query_ext.is_some());
    }

    /// 内存入口的 provider 也算可用（open 变体二选一）。
    #[test]
    fn a_memory_only_provider_loads() {
        let api = unsafe {
            CsriApi::from_lookup(lookup_of(&[
                "csri_renderer_default",
                "csri_open_mem",
                "csri_render",
                "csri_close",
            ]))
            .unwrap()
        };
        assert!(api.open_mem.is_some());
        assert!(api.open_file.is_none());
        // 没有 request_fmt 也没有 query_ext：我们不知道怎么驱动它
        assert_eq!(api.fmt_path(), FmtPath::Unknown);
    }

    /// 缺必需项时**一次报全**，不是缺一个报一个。
    #[test]
    fn missing_required_symbols_are_all_reported_at_once() {
        // 不用 `unwrap_err()`：`CsriApi` 是一堆函数指针，没有有意义的 Debug，
        // 而为测试给它加 Debug 会掩盖"它本来就不该被打印"这件事。
        let e = match unsafe { CsriApi::from_lookup(lookup_of(&["csri_renderer_default"])) } {
            Ok(_) => panic!("只导一个符号的东西不该被当成可用 provider"),
            Err(e) => e,
        };
        assert_eq!(e.code_string(), "STK-RND-0073");
        let d = e.details.join(" ");
        assert!(d.contains("csri_render"), "{d}");
        assert!(d.contains("csri_close"), "{d}");
        // 两个 open 变体都没，应当合成一条
        assert!(d.contains("csri_open_file"), "{d}");
    }

    /// 必需列表必须与能力探测共用同一份。
    ///
    /// 分叉的后果是"探测说能用、加载时找不到"，而用户看到的是一条
    /// 自相矛盾的报错。这里直接比常量。
    #[test]
    fn the_required_list_is_the_same_one_the_capability_probe_uses() {
        assert_eq!(REQUIRED, render_api::capability::CSRI_REQUIRED_EXPORTS);
        assert_eq!(OPEN_VARIANTS, render_api::capability::CSRI_OPEN_VARIANTS);
    }
}
