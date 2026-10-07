//! 真实 libass provider（12 §1A、09 §2 §3、F7 决策：内存输入，不走文件路径）。
//!
//! 三条不可商量的规矩：
//! 1. **运行时加载**（`LibHandle`），不编译期链接：native 依赖缺失时是一条可报告的错误；
//! 2. **字幕走内存**：`ass_read_memory` + UTF-8 字节，不经过窄字符串文件路径（Windows 长路径/中文）；
//! 3. **两个尺寸都要设**：`ass_set_storage_size`（脚本 PlayRes 空间）与 `ass_set_frame_size`
//!    （输出画布）分别对应 09 §2 的四层几何；只设一个会让 libass 按错误的比例缩放。
//!
//! 字体：`ass_set_fonts` 必须在任何一帧之前调用（头文件明写"font lookup must be configured
//! before an ASS_Renderer can be used"）。这里用系统的默认字体探测（DirectWrite/fontconfig 由
//! 构建决定），并把回退族名传进去。
//!
//! 4. **字体注入是可选能力，不是 provider 的可用条件**（17 §7）：`add_font` 把**我们读到的字节**
//!    交给 `ass_add_font`，libass 的 fontselect 会优先用内存字体，于是"渲染缓存 key 里钉住的那些
//!    哈希"与"真正决定像素的那组字节"变成同一件东西 —— 这正是 AI4 那条边界要收的口。
//!    代价说清楚：注入改变了字体选择的来源，所以它只能显式开启（CLI 的 `--inject-fonts`），
//!    并且在报告里逐条列出注入了哪个名字、多少字节、内容哈希是多少。

use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_longlong, c_void};
use std::path::Path;

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use core_model::time::{Rational, Time};

use crate::bitmap::{composite, ImageDesc, ImageKind};
use crate::ffi::{probe, AssImage, LibHandle};
use image_core::Frame;

/// `ASS_FONTPROVIDER_AUTODETECT`（头文件：0=NONE，1=AUTODETECT）。
const FONT_PROVIDER_AUTODETECT: c_int = 1;

type FnLibraryInit = unsafe extern "C" fn() -> *mut c_void;
type FnLibraryDone = unsafe extern "C" fn(*mut c_void);
type FnLibraryVersion = unsafe extern "C" fn() -> c_int;
type FnRendererInit = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type FnRendererDone = unsafe extern "C" fn(*mut c_void);
type FnSetFrameSize = unsafe extern "C" fn(*mut c_void, c_int, c_int);
type FnSetStorageSize = unsafe extern "C" fn(*mut c_void, c_int, c_int);
type FnSetFonts =
    unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, c_int, *const c_char, c_int);
type FnFreeTrack = unsafe extern "C" fn(*mut c_void);
type FnReadMemory =
    unsafe extern "C" fn(*mut c_void, *mut c_char, usize, *const c_char) -> *mut c_void;
type FnRenderFrame =
    unsafe extern "C" fn(*mut c_void, *mut c_void, c_longlong, *mut c_int) -> *mut AssImage;
/// `void ass_add_font(ASS_Library*, const char *name, const char *data, size_t data_size)`。
///
/// **返回 void**（头文件 813 行）。这不是可以省心的地方：第一版按 `int` 声明并检查返回值，
/// 于是每次调用都"失败"——读到的其实是上一次调用留在 EAX 里的指针低位（1894312096 这种数），
/// 看起来像 libass 拒绝了字体，实际是我们自己把返回值类型写错了（12 §探测的同一条：
/// 签名照头文件抄，不照常识猜）。既然没有返回码，成功与否只能靠**效果**证明：
/// `tests/render_real.rs` 用注入前后的像素差来证明这批字节真的参与了渲染。
/// 另外**第四个参数是 `int`，不是 `size_t`**：头文件写 `int data_size`。
/// 按 `usize` 声明在 x64 上"碰巧能用"（被调用方只读低 32 位），但那是运气不是 ABI 正确；
/// 这里照头文件声明成 `c_int`，并把超过 `i32::MAX` 的输入明确拒掉。
/// libass 自己复制数据（内存字体存进 fontselect），所以不要求调用方保活 `data`。
type FnAddFont = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, c_int);

struct Symbols {
    library_init: FnLibraryInit,
    library_done: FnLibraryDone,
    library_version: FnLibraryVersion,
    renderer_init: FnRendererInit,
    renderer_done: FnRendererDone,
    set_frame_size: FnSetFrameSize,
    set_storage_size: FnSetStorageSize,
    set_fonts: FnSetFonts,
    free_track: FnFreeTrack,
    read_memory: FnReadMemory,
    render_frame: FnRenderFrame,
    /// 可选能力（17 §7）：缺了不影响渲染，只有 `add_font` 会点名报错。
    /// 用 Option 而不是要求符号：把可选能力做成硬条件，会把本来能用的 DLL 判成不可用。
    /// 没有 `ass_clear_fonts`：头文件规定它只能在 renderer/track 都释放之后调用，
    /// 而 Provider 活着就意味着 renderer 活着 —— 见 `ffi::LIBASS_FONT_SYMBOLS`。
    add_font: Option<FnAddFont>,
}

impl Symbols {
    fn load(h: &LibHandle) -> Result<Symbols> {
        // 12 §探测：符号一个一个取，取不到就点名，绝不"看起来能用"
        macro_rules! sym {
            ($name:expr) => {{
                let ptr = h.symbol($name).ok_or_else(|| missing($name))?;
                unsafe { std::mem::transmute_copy::<*mut c_void, _>(&ptr) }
            }};
        }
        Ok(Symbols {
            library_init: sym!("ass_library_init"),
            library_done: sym!("ass_library_done"),
            library_version: sym!("ass_library_version"),
            renderer_init: sym!("ass_renderer_init"),
            renderer_done: sym!("ass_renderer_done"),
            set_frame_size: sym!("ass_set_frame_size"),
            set_storage_size: sym!("ass_set_storage_size"),
            set_fonts: sym!("ass_set_fonts"),
            free_track: sym!("ass_free_track"),
            read_memory: sym!("ass_read_memory"),
            render_frame: sym!("ass_render_frame"),
            add_font: opt(h, "ass_add_font"),
        })
    }
}

/// 可选符号：取不到就留 None，由使用点给出能翻译的错误，而不是让 provider 打不开。
fn opt<T>(h: &LibHandle, name: &str) -> Option<T> {
    let ptr = h.symbol(name)?;
    // SAFETY: 调用方按符号名把它转成对应的函数指针类型
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, _>(&ptr) })
}

fn missing(name: &str) -> StkError {
    StkError::simple(Domain::Render, 80, "libass DLL 缺少必需导出符号").with_detail(name)
}

fn bad(title: &str, detail: &str) -> StkError {
    StkError::simple(Domain::Render, 81, title).with_detail(detail.to_string())
}

/// 一条"已经交给 libass 的内存字体"。哈希是在这里算的，不是调用方报上来的：
/// 报告要能自证"渲染用的就是这几份字节"。
#[derive(Clone, Debug)]
pub struct InjectedFont {
    /// 注册给 libass 的名字（通常是解析到的家族名）
    pub name: String,
    pub bytes: usize,
    pub sha256: String,
}

/// 一份脚本的一次渲染结果。
pub struct Rendered {
    pub frame: Frame,
    /// 1=位置可能变了，2=内容可能变了（头文件的 detect_change 语义）
    pub detect_change: i32,
    pub images: usize,
    pub loss: LossReport,
}

/// 一个脚本的句柄。libass 的 renderer 不是线程安全的，
/// 一个 libass 实例（library + renderer + 已配置的尺寸与字体）。
pub struct Provider {
    _handle: LibHandle,
    symbols: Symbols,
    library: *mut c_void,
    renderer: *mut c_void,
    storage: Canvas,
    output: Canvas,
    version: i32,
    injected: Vec<InjectedFont>,
    can_inject: bool,
}

/// 只报身份与尺寸：句柄地址没有诊断价值，也不该被抄进日志。
impl core::fmt::Debug for Provider {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Provider")
            .field("libass_version", &self.version)
            .field("storage", &self.storage.label())
            .field("output", &self.output.label())
            .field("fonts_injected", &self.injected.len())
            .field("can_inject_fonts", &self.can_inject)
            .finish()
    }
}

impl Provider {
    /// 加载并按 09 §2 配好尺寸。`storage` 是脚本 PlayRes 空间，`output` 是输出画布。
    pub fn open(
        dll: &Path,
        storage: Canvas,
        output: Canvas,
        default_family: &str,
    ) -> Result<Provider> {
        // probe() 已经按导出符号判定"能不能当 libass provider"，不看文件名（12 §探测）
        let (handle, _report) = probe(dll)?;
        let symbols = Symbols::load(&handle)?;
        let library = unsafe { (symbols.library_init)() };
        if library.is_null() {
            return Err(bad("ass_library_init 返回空指针", "native 依赖或内存问题"));
        }
        let renderer = unsafe { (symbols.renderer_init)(library) };
        if renderer.is_null() {
            unsafe { (symbols.library_done)(library) };
            return Err(bad(
                "ass_renderer_init 返回空指针",
                "检查 ass-9.dll 及其依赖是否齐全",
            ));
        }
        let version = unsafe { (symbols.library_version)() };
        let can_inject = symbols.add_font.is_some();
        // 顺序：先尺寸后字体（头文件要求字体在渲染前配好）
        unsafe {
            (symbols.set_storage_size)(renderer, storage.width as c_int, storage.height as c_int);
            (symbols.set_frame_size)(renderer, output.width as c_int, output.height as c_int);
        }
        let family =
            CString::new(default_family).map_err(|e| bad("默认字体族名含 NUL", &e.to_string()))?;
        unsafe {
            (symbols.set_fonts)(
                renderer,
                std::ptr::null(),
                family.as_ptr(),
                FONT_PROVIDER_AUTODETECT,
                std::ptr::null(),
                0,
            );
        }
        Ok(Provider {
            _handle: handle,
            symbols,
            library,
            renderer,
            storage,
            output,
            version,
            injected: Vec::new(),
            can_inject,
        })
    }

    /// 这份 DLL 能不能注入字体（`ass_add_font` + `ass_clear_fonts` 都在）。
    /// 报告里要说清"这次是按我们指定的字节渲染，还是按系统字体选择渲染"。
    pub fn can_inject_fonts(&self) -> bool {
        self.can_inject
    }

    /// 交给渲染器的内存字体清单（名字、字节数、内容哈希）。
    pub fn injected(&self) -> &[InjectedFont] {
        &self.injected
    }

    /// 17 §7：把**我们读到的**字体字节交给 libass 作为内存字体。
    ///
    /// 之后 libass 的字体选择会优先命中这些内存字体，于是"缓存 key 里钉住的哈希"
    /// 与"决定像素的那组字节"是同一件事（AI4 的收口）。libass 自己复制数据，
    /// 所以 `data` 不需要在这次调用之后继续存活。
    ///
    /// 失败语义说清楚：符号缺失是**明确报错**（85），不静默回退到"按系统字体渲染"——
    /// 那样报告里的身份就说了假话。`ass_add_font` 本身返回 void，没有码可查，
    /// 所以"这批字节真的参与了渲染"由测试用像素差来证明，而不是由这里承诺。
    ///
    /// 注入之后想换一批字体，只能重新开一个 Provider：`ass_clear_fonts` 要求先释放
    /// renderer 与 track（头文件 815-822 行），而 Provider 活着就意味着 renderer 活着，
    /// 所以我们不提供"清一下"的快捷键 —— 那是个在我们的生命周期里不安全的调用。
    pub fn add_font(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let Some(add) = self.symbols.add_font else {
            return Err(StkError::simple(
                Domain::Render,
                85,
                "这份 libass 没有 ass_add_font，不能按指定字节注入字体",
            )
            .with_detail(name.to_string())
            .with_action("换带字体注入能力的 libass 构建，或去掉 --inject-fonts 走系统字体选择"));
        };
        if data.is_empty() {
            return Err(bad("注入的字体字节为空", name));
        }
        // 头文件的长度参数是 int：超过 i32::MAX 的"字体"不是我们要悄悄截断的东西
        let Ok(len) = c_int::try_from(data.len()) else {
            return Err(bad(
                "注入的字体字节超过 libass 的 int 长度参数能表达的范围",
                &format!("{name}：{} 字节", data.len()),
            ));
        };
        let cname = CString::new(name).map_err(|e| bad("字体名含 NUL", &e.to_string()))?;
        // 返回 void：没有码可查，所以这里的"成功"由调用之后的像素测试负责证明（见上注）。
        unsafe {
            (add)(
                self.library,
                cname.as_ptr(),
                data.as_ptr() as *const c_char,
                len,
            )
        };
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(data);
        let sha256: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        self.injected.push(InjectedFont {
            name: name.to_string(),
            bytes: data.len(),
            sha256,
        });
        Ok(())
    }

    /// `ass_library_version()`：整数版本号（如 0x001705 之类），进 provider 身份记录。
    pub fn library_version(&self) -> i32 {
        self.version
    }

    pub fn sizes(&self) -> (Canvas, Canvas) {
        (self.storage, self.output)
    }

    /// 载入脚本（内存，UTF-8）。
    pub fn load_script(&self, ass: &[u8]) -> Result<Script<'_>> {
        let mut buffer: Vec<c_char> = ass.iter().map(|b| *b as c_char).collect();
        buffer.push(0);
        let name = CString::new("subtitle").map_err(|e| bad("track 名含 NUL", &e.to_string()))?;
        let track = unsafe {
            (self.symbols.read_memory)(self.library, buffer.as_mut_ptr(), ass.len(), name.as_ptr())
        };
        if track.is_null() {
            return Err(bad(
                "ass_read_memory 失败",
                &format!("输入 {} 字节；libass 认为脚本不可用", ass.len()),
            ));
        }
        Ok(Script {
            renderer: self.renderer,
            symbols: &self.symbols,
            track,
            _buffer: buffer,
        })
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        unsafe {
            (self.symbols.renderer_done)(self.renderer);
            (self.symbols.library_done)(self.library);
        }
    }
}

/// 一份已载入的脚本 + 它的 renderer 绑定。
pub struct Script<'a> {
    renderer: *mut c_void,
    symbols: &'a Symbols,
    track: *mut c_void,
    /// 与 track 同生命周期：libass 解析期间引用它
    _buffer: Vec<c_char>,
}

impl Drop for Script<'_> {
    fn drop(&mut self) {
        unsafe { (self.symbols.free_track)(self.track) }
    }
}

/// 把 `now` 精确换成 libass 要的毫秒整数；不是整毫秒就报错而不是悄悄舍入。
fn whole_millis(t: Time) -> Result<i64> {
    let r = t.as_rational();
    let scaled = r
        .mul(&Rational::from_int(1000))
        .ok_or_else(|| bad("时间换算溢出", "毫秒换算溢出"))?;
    if scaled.den() != 1 {
        return Err(
            StkError::simple(Domain::Render, 83, "渲染时间点不在整毫秒网格上")
                .with_detail(format!("t = {r:?} -> {} 毫秒不是整数", scaled.to_f64())),
        );
    }
    Ok(scaled.num())
}

/// 渲染一帧：`now` 是输出时间轴上的毫秒，走 `ass_render_frame` + 合成。
pub fn render(script: &Script<'_>, now: Time, output: Canvas) -> Result<Rendered> {
    let mut detect: c_int = 0;
    let ms = whole_millis(now)?;
    let head = unsafe {
        (script.symbols.render_frame)(
            script.renderer,
            script.track,
            ms as c_longlong,
            &mut detect as *mut c_int,
        )
    };
    let chain = unsafe { crate::ffi::walk_chain(head as *const AssImage)? };
    let mut descs: Vec<ImageDesc<'_>> = Vec::with_capacity(chain.len());
    for img in &chain {
        if img.w <= 0 || img.h <= 0 {
            continue;
        }
        let stride = if img.stride > 0 {
            img.stride as usize
        } else {
            return Err(StkError::simple(Domain::Render, 84, "libass 给出负 stride")
                .with_detail(format!("stride={}", img.stride)));
        };
        let need = stride * (img.h as usize - 1) + img.w as usize;
        if img.bitmap.is_null() {
            return Err(bad("libass 位图指针为空", "结构非法，停止合成"));
        }
        // SAFETY: 头文件保证分配至少 stride*(h-1)+w 字节
        let bitmap = unsafe { std::slice::from_raw_parts(img.bitmap as *const u8, need) };
        let kind = match img.kind {
            1 => ImageKind::Outline,
            2 => ImageKind::Shadow,
            _ => ImageKind::Character,
        };
        if std::env::var("STK_RENDER_DEBUG").is_ok() {
            eprintln!(
                "img w={} h={} stride={} color={:08X} dst=({}, {}) kind={} 前两字节={} {}",
                img.w,
                img.h,
                img.stride,
                img.color,
                img.dst_x,
                img.dst_y,
                img.kind,
                bitmap[0],
                bitmap.get(1).copied().unwrap_or(0)
            );
        }
        descs.push(ImageDesc {
            width: img.w,
            height: img.h,
            stride: img.stride,
            bitmap,
            color: img.color,
            dst_x: img.dst_x,
            dst_y: img.dst_y,
            kind,
        });
    }
    let mut loss = LossReport::new();
    loss.preserved("libass 位图按链表顺序合成（ass.h:56 要求）");
    let frame = composite(output, &descs, &mut loss)?;
    Ok(Rendered {
        frame,
        detect_change: detect as i32,
        images: descs.len(),
        loss,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millisecond_conversion_is_exact_or_refused() {
        assert_eq!(
            whole_millis(Time::from_millis(1234).unwrap()).unwrap(),
            1234
        );
        // 1/3 秒 = 333.33…ms：不在整毫秒网格上，必须拒绝而不是悄悄舍入
        let awkward = Time::from_rational(Rational::new(1, 3).unwrap());
        let e = whole_millis(awkward).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0083");
    }

    #[test]
    fn missing_dll_is_reported_not_panicked() {
        let e = Provider::open(
            Path::new("Z:/nope/ass-9.dll"),
            Canvas::HD720,
            Canvas::HD720,
            "Arial",
        )
        .unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0071");
    }
}
