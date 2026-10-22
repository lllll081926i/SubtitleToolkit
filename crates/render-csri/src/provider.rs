//! 加载一个用户提供的 CSRI provider，并把它跑起来。
//!
//! ## 与 libass 那条链的关系
//!
//! 两条链**结构同构**（都是运行时加载、都不在编译期链接），
//! 而且现在共用同一份加载器：[`core_model::dllload::LibHandle`]。
//!
//! 共享这份是有意的 —— 加载策略里每条都踩过坑（不搜当前目录、
//! 必须完整路径、正斜杠要规范化），而**两份实现对同一个 DLL 会给出不同答案**：
//! 一边放行一边拒绝，用户看到的是一条自相矛盾的报错。
//!
//! 不共享的是**装订代码**：CSRI 与 libass 的 ABI 完全不同，
//! 抽一个"通用 provider"会把两套不兼容的语义塞进一个抽象里。
//!
//! ## 加载顺序（12 §1 的逐级探测）
//!
//! ```text
//! PE 位数  →  导出表（render-api::capability::probe）  →  取符号  →  调用
//! ```
//!
//! 每一步失败都给出**用户能照做**的说明，而不是一句"加载失败"。
//! 尤其：如果它其实是一个 VapourSynth 插件或 AviSynth 滤镜，
//! 要直接说出来 —— 那比"缺符号"有用得多。

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use core_model::dllload::LibHandle;
use core_model::error::{Result, StkError};
use render_api::capability::{self, ProviderKind};

use crate::codes;
use crate::ffi::{CsriApi, CsriFmt, CsriInst, CsriRend, FmtPath, PixFmt, CSRI_EXT_STREAM};
use crate::frame::CsriRgba;

/// provider 的身份报告（进信封与崩溃现场）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsriReport {
    pub path: String,
    pub sha256: String,
    pub arch: String,
    /// `csri_renderer_info()` 报出的名字（拿不到就空）。
    pub name: String,
    pub longname: String,
    pub specific: String,
    /// 走的哪条设格式路径。
    pub fmt_path: String,
    /// 实际向 provider 申请到的像素格式（**决定内存布局**，必须报出来）。
    pub pixfmt: String,
    /// 是否拿到 `csri/stream` 扩展（12 §1B 点名的那条）。
    pub has_stream_ext: bool,
}

/// 一个已打开的 CSRI provider。
pub struct CsriProvider {
    handle: LibHandle,
    api: CsriApi,
    renderer: CsriRend,
    inst: CsriInst,
    pub report: CsriReport,
    /// 已申请到的格式与画布。
    ///
    /// 存 `CsriFmt` 的**原始值**而不是我们的枚举：`csri_fmt` 是要交给
    /// provider 的东西，而 provider 认的是数字。
    fmt: CsriFmt,
    pixfmt: PixFmt,
    /// 给 provider 的像素缓冲。它的生命期必须覆盖整次渲染 ——
    /// provider 只持指针，不拷贝。
    pixels: Vec<u8>,
    bpp: usize,
}

// 与 libass 的 `LibHandle` 同一条理由：不实现 `Send`。
// provider 是 native 代码，跨线程移动它要它自己保证线程安全，
// 而 CSRI 头文件里没有这样的承诺。
impl std::fmt::Debug for CsriProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsriProvider")
            .field("report", &self.report)
            .finish()
    }
}

/// 我们愿意接受的像素格式，**按优先级**。
///
/// 优先 BGRA（自带 alpha，与 libass 那条链的 RGBA8 语义最近），
/// 退而用 `BGR_`（Windows "RGB32"，第 4 字节是**填充**不是 alpha，
/// 见 `frame.rs` 里那一处的处理）。
///
/// **刻意不接** `BGR`(24 位) 与 YUV 系：前者每像素 3 字节，而行距计算与
/// 拷贝路径是按 4 字节写的；后者要一整套色彩转换，那是独立的一件工作，
/// 不该在这里顺手偷做。
const WANTED_PIXFMTS: &[PixFmt] = &[PixFmt::Bgra, PixFmt::BgrUnderscore];

impl CsriProvider {
    /// 探测一个 DLL 是不是可用的 CSRI provider，并给出**它是什么**。
    ///
    /// **不加载**。与 [`Self::open`] 分开，因为"它是什么"这个问题
    /// 在加载之前就该有答案（12 §3 禁止跨位数加载）。
    pub fn probe(path: &Path) -> Result<(capability::ProbeResult, String)> {
        let exports = read_exports(path)?;
        let arch = core_model::pe::arch_of_file(path)
            .map(|a| a.label().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let r = capability::probe(
            path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            &exports.names,
        );
        Ok((r, arch))
    }

    /// 打开一个 CSRI provider 并载入一段 ASS 脚本。
    ///
    /// 脚本**从内存给**（`csri_open_mem`）优先：宿主已经把脚本读进来并校验过，
    /// 再让 provider 去读一次盘会把"我们校验过的那份"与"它读的那份"
    /// 变成两个可能不同的东西。只有在 provider 不支持内存入口时才回退到文件。
    ///
    /// ## `declared_script_len`：为什么要把一个参数写成这样的名字
    ///
    /// 正常调用传 `None`，长度就是 `script.len()`。
    /// 传 `Some(n)` 会向 provider **谎报**长度 —— 而这个参数存在的唯一理由
    /// 是 AT-RENDER-002：
    ///
    /// > 证明其崩溃只终止 renderer-host
    ///
    /// 那个"其"是**第三方 DLL 自己**，不能用注入/poison 代替。
    /// 而已知的真崩溃点就在这里：`csri_open_mem` 的 `length` 若大于真实缓冲，
    /// VSFilterMod **不做上界检查**，越界读到访问违例（实测 0xC0000005）。
    ///
    /// 把它写成显式参数而不是"从环境变量里读"，是因为这样**看得见**：
    /// 读代码的人在这个签名上就知道存在一条能谎报长度的路径，
    /// 而不会以为 `script.len()` 是唯一的真相。调用方只有 `renderer-host`，
    /// 而它只在 `STK_HOST_LIE_ABOUT_LENGTH` 被显式设上时才传 `Some`。
    pub fn open(
        path: &Path,
        script: &str,
        script_path_for_open_file: Option<&Path>,
        width: u32,
        height: u32,
    ) -> Result<CsriProvider> {
        Self::open_with_declared_len(path, script, script_path_for_open_file, width, height, None)
    }

    /// 见 [`Self::open`] 的 `declared_script_len` 说明。
    /// 谎报长度是一件需要被看见的事，所以它单独有一个入口。
    pub fn open_with_declared_len(
        path: &Path,
        script: &str,
        script_path_for_open_file: Option<&Path>,
        width: u32,
        height: u32,
        declared_script_len: Option<usize>,
    ) -> Result<CsriProvider> {
        // ---- ① 位数（12 §3）----
        let arch = core_model::pe::arch_of_file(path).map_err(|e| {
            StkError::simple(
                codes::READ_FAILED.0,
                codes::READ_FAILED.1,
                "CSRI provider 读取失败",
            )
            .with_detail(format!("{}：{e}", path.display()))
        })?;
        let expected_arch = if cfg!(target_arch = "x86") {
            core_model::pe::PeArch::X86
        } else if cfg!(target_arch = "x86_64") {
            core_model::pe::PeArch::X64
        } else {
            core_model::pe::PeArch::Other(0)
        };
        if arch != expected_arch {
            return Err(StkError::simple(
                codes::ARCH_MISMATCH.0,
                codes::ARCH_MISMATCH.1,
                "CSRI provider 的位数与本进程不匹配",
            )
            .with_detail(format!(
                "{}：本进程目标是 {}，该库是 {}",
                path.display(),
                if cfg!(target_arch = "x86") {
                    "x86"
                } else {
                    "x64"
                },
                arch.label()
            ))
            .with_action("换成与 renderer-host 相同位数的那一份；12 §3 禁止跨位数加载"));
        }

        // ---- ② 能力（12 §1）----
        let (probe, _) = Self::probe(path)?;
        if probe.kind != Some(ProviderKind::Csri) {
            let (what, why) = describe_kind(probe.kind);
            return Err(StkError::simple(
                codes::NOT_CSRI.0,
                codes::NOT_CSRI.1,
                "候选 DLL 不是可用的 CSRI provider",
            )
            .with_detail(format!("{}：{what}", path.display()))
            .with_detail(why)
            .with_action("用 `provider probe <dll>` 看它究竟是什么"));
        }

        // ---- ③ 取符号 ----
        let handle = LibHandle::open(path)?;
        // SAFETY：上一步已按导出名确认这是 CSRI provider；
        // 每个地址都按 CSRI 头文件的签名解释。
        let api = unsafe { CsriApi::from_lookup(|n| handle.symbol(n))? };
        if api.fmt_path() == FmtPath::Unknown {
            return Err(StkError::simple(
                codes::NO_FMT_PATH.0,
                codes::NO_FMT_PATH.1,
                "CSRI provider 既没有 csri_request_fmt 也没有 csri_query_ext",
            )
            .with_detail(format!(
                "{}：我们能认出它是 CSRI，但不知道该怎么让它设格式",
                path.display()
            ))
            .with_action("这是 provider 的问题；换一份，或用 `provider probe` 核对"));
        }

        // ---- ④ 拿 renderer 句柄 ----
        // SAFETY：`renderer_default` 在必需集合里，上一步保证了它在。
        let renderer = unsafe { (api.renderer_default.expect("必需项"))() };
        if renderer.is_null() {
            return Err(StkError::simple(
                codes::NO_RENDERER.0,
                codes::NO_RENDERER.1,
                "CSRI provider 没有给出默认 renderer",
            )
            .with_detail(format!(
                "{}：csri_renderer_default() 返回空",
                path.display()
            ))
            .with_action("这份 provider 可能不完整；换一份"));
        }

        // ---- ⑤ 读它的自我描述（可失败，不致命）----
        let (name, longname, specific) = read_info(&api, renderer);

        // ---- ⑥ 开脚本 ----
        // 先拼一个以 0 结尾的副本：CSRI 头文件要求"渲染器若要保留数据必须
        // 自己拷贝"，所以调用之后我们释放它是安全的。
        let mut script_c = script.as_bytes().to_vec();
        script_c.push(0);
        let inst = if let Some(open_mem) = api.open_mem {
            // 正常路径下 declared 就是真长度；只有 AT-RENDER-002 的用例会谎报，
            // 用来逼出 provider 自己的越界读（它不是我们的输入校验问题，
            // 而是 provider 缺了那道校验）。
            let declared = declared_script_len.unwrap_or(script.len());
            // SAFETY：缓冲以 0 结尾且在这次调用期间有效。
            // `declared` 如果被谎报大了，从这里越界读**是故意的** ——
            // 那种情况下本调用会访问违例，而 host 本来就是为了装住它才存在的。
            unsafe {
                open_mem(
                    renderer,
                    script_c.as_ptr() as *const c_void,
                    declared,
                    std::ptr::null_mut(),
                )
            }
        } else if let Some(open_file) = api.open_file {
            let Some(p) = script_path_for_open_file else {
                return Err(StkError::simple(
                    codes::NEEDS_FILE_PATH.0,
                    codes::NEEDS_FILE_PATH.1,
                    "该 CSRI provider 只支持从文件开脚本，但没有给出路径",
                )
                .with_detail(format!("{}：只导出 csri_open_file", path.display()))
                .with_action("这条链会优先用内存入口；这份 provider 需要文件路径"));
            };
            let mut c = path_to_c_string(p)?;
            c.push(0);
            // SAFETY：以 0 结尾的 C 字符串，调用期间有效。
            unsafe { open_file(renderer, c.as_ptr() as *const i8, std::ptr::null_mut()) }
        } else {
            unreachable!("from_lookup 已经保证至少有一个 open 入口")
        };
        if inst.is_null() {
            return Err(StkError::simple(
                codes::SCRIPT_OPEN_FAILED.0,
                codes::SCRIPT_OPEN_FAILED.1,
                "CSRI provider 打不开这段脚本",
            )
            .with_detail(format!(
                "{}：csri_open_* 返回空（脚本 {} 字节）",
                path.display(),
                script.len()
            ))
            .with_action("确认脚本是 provider 能解析的 ASS/SSA"));
        }

        // ---- ⑦ 设格式 ----
        //
        // 按优先级**逐个试**，而不是"只试一个然后放弃"。
        //
        // 实测抳出来的：本机 `VSFilterMod.dll` 对 **BGRA 返回 -1**，
        // 而对 `BGR_`(0x102) 返回 0。所以只试 BGRA 会让它打不开 ——
        // 那不是 provider 的问题，是我们没按它的能力谈。
        //
        // 抳出这一点的是一次**逐个试真 provider 接受哪种格式**的探针
        // （`examples/probe_fmt.rs`），顺手还抳出我自己把
        // `CSRI_F_BGR_` 的数值写错了（见 `PixFmt` 的注释）。
        let mut chosen: Option<PixFmt> = None;
        let mut tried: Vec<String> = Vec::new();
        if let Some(request_fmt) = api.request_fmt {
            for cand in WANTED_PIXFMTS {
                let f = CsriFmt::new(*cand, width, height);
                // SAFETY：inst 非空、f 是有效的 CsriFmt。
                let rc = unsafe { request_fmt(inst, &f) };
                tried.push(format!("{}={rc}", cand.label()));
                if rc == 0 {
                    chosen = Some(*cand);
                    break;
                }
            }
        }
        let Some(pixfmt) = chosen else {
            // 没谈成。区分三种情形，因为用户要做的事不同。
            let has_stream = if let Some(q) = api.query_ext {
                // SAFETY：renderer 非空；extname 是以 0 结尾的常量。
                !unsafe { q(renderer, CSRI_EXT_STREAM.as_ptr() as *const i8) }.is_null()
            } else {
                false
            };
            // SAFETY：inst 是我们刚拿到的。
            unsafe { (api.close)(inst) };
            return Err(if !tried.is_empty() {
                StkError::simple(
                    codes::PIXFMT_REFUSED.0,
                    codes::PIXFMT_REFUSED.1,
                    "CSRI provider 不接受我们支持的像素格式",
                )
                .with_detail(format!(
                    "{}：{}x{} 下 {}",
                    path.display(),
                    width,
                    height,
                    tried.join("、")
                ))
                .with_action(
                    "这条链只接 BGRA 与 BGR_（都是 32 位，行距与拷贝路径按 4 字节写）；\
                     该 provider 要的是别的格式，接它需要先开发对应的转换",
                )
            } else if has_stream {
                StkError::simple(
                    codes::STREAM_EXT_ONLY.0,
                    codes::STREAM_EXT_ONLY.1,
                    "该 CSRI provider 只支持 csri/stream 扩展，而这条链尚未实现它",
                )
                .with_detail(format!(
                    "{}：有 csri/stream，没有 csri_request_fmt",
                    path.display()
                ))
                .with_action("如实报告：不是 provider 的问题，是这条链还没实现 stream 路径")
            } else {
                StkError::simple(
                    codes::NO_FMT_PATH.0,
                    codes::NO_FMT_PATH.1,
                    "CSRI provider 既没有 csri_request_fmt 也没有 csri/stream 扩展",
                )
                .with_detail(format!("{}：没有可用的设格式入口", path.display()))
                .with_action("这条链需要 provider 支持二者之一")
            });
        };

        let bpp = pixfmt
            .bytes_per_pixel()
            .expect("WANTED_PIXFMTS 里都是 32 位格式");
        let fmt = CsriFmt::new(pixfmt, width, height);
        let pixels = vec![0u8; (width as usize) * (height as usize) * bpp];
        let sha256 = sha256_of_file(path);

        // 这几个值要在 `api` 被移进结构体**之前**算好：
        // 结构体字面量里的字段求值顺序不保证，先读 `api` 会 borrow error。
        let fmt_path_label = match api.fmt_path() {
            FmtPath::RequestFmt => "csri_request_fmt".to_string(),
            FmtPath::StreamExtOnly => "csri_query_ext".to_string(),
            FmtPath::Unknown => "unknown".to_string(),
        };
        let has_stream_ext = if let Some(q) = api.query_ext {
            // SAFETY：renderer 非空（上面核过）；extname 是以 0 结尾的常量。
            !unsafe { q(renderer, CSRI_EXT_STREAM.as_ptr() as *const i8) }.is_null()
        } else {
            false
        };

        Ok(CsriProvider {
            handle,
            api,
            renderer,
            inst,
            report: CsriReport {
                path: path.display().to_string(),
                sha256,
                arch: arch.label().to_string(),
                name,
                longname,
                specific,
                fmt_path: fmt_path_label,
                pixfmt: pixfmt.label().to_string(),
                has_stream_ext,
            },
            fmt,
            pixfmt,
            pixels,
            bpp,
        })
    }

    pub fn width(&self) -> u32 {
        self.fmt.width
    }

    pub fn height(&self) -> u32 {
        self.fmt.height
    }

    /// 实际申请到的像素格式（决定内存布局，不猜）。
    pub fn pixfmt(&self) -> PixFmt {
        self.pixfmt
    }

    /// 画某个时刻的一帧。返回 RGBA8。
    ///
    /// 每帧都重新分配返回缓冲：`pixels` 是给 provider 写的，
    /// 直接把它交出去会让下一次 `render` 覆写调用方还持着的帧。
    pub fn render_at(&mut self, t_seconds: f64) -> Result<CsriRgba> {
        // 每帧清空：provider 可能只画有字幕的部分，残留上一帧会变成"鬼影"。
        self.pixels.iter_mut().for_each(|b| *b = 0);
        let mut frame = unsafe {
            crate::frame::frame_over_buffer(
                &mut self.pixels,
                self.pixfmt,
                self.fmt.width,
                self.fmt.height,
            )?
        };
        // SAFETY：inst 非空且在 `open` 时核过；frame 的 planes[0] 指向
        // `self.pixels`，它在这次调用期间有效且大小正确。
        unsafe { (self.api.render)(self.inst, &mut frame, t_seconds) };
        let mut out = vec![0u8; (self.fmt.width as usize) * (self.fmt.height as usize) * 4];
        crate::frame::copy_frame_into(&frame, self.fmt.width, self.fmt.height, &mut out)?;
        Ok(CsriRgba {
            width: self.fmt.width,
            height: self.fmt.height,
            pixels: out,
        })
    }

    /// 这个 provider 有没有 12 §1B 点名的 stream 扩展。
    pub fn has_stream_ext(&self) -> bool {
        self.report.has_stream_ext
    }

    /// 每像素字节数（BGRA / BGR_ 都是 4）。
    pub fn bytes_per_pixel(&self) -> usize {
        self.bpp
    }
}

impl Drop for CsriProvider {
    fn drop(&mut self) {
        // 先关实例再卸载 DLL：顺序反了会让 provider 在已卸载的代码里跑 close。
        if !self.inst.is_null() {
            // SAFETY：inst 由 `open` 里那次成功调用产出，且只在这里关一次。
            unsafe { (self.api.close)(self.inst) };
            self.inst = std::ptr::null_mut();
        }
        // `handle` 与 `renderer` 由各自的 Drop/值语义收尾；
        // 这里只是让"它们归本次结构体所有"这件事显式。
        let _ = (&self.handle, self.renderer);
    }
}

fn read_exports(path: &Path) -> Result<core_model::pe::Exports> {
    core_model::pe::exports_of_file(path).map_err(|e| match e {
        // "读不了"与"不是那种 DLL"必须分开：前者修路径，后者换文件。
        core_model::pe::PeError::Unreadable(_) => StkError::simple(
            codes::READ_FAILED.0,
            codes::READ_FAILED.1,
            "CSRI provider 读取失败",
        )
        .with_detail(format!("{}：{e}", path.display())),
        core_model::pe::PeError::NotPe(_) => StkError::simple(
            codes::NOT_PE.0,
            codes::NOT_PE.1,
            "候选 DLL 不是有效的 PE 文件",
        )
        .with_detail(format!("{}：{e}", path.display())),
    })
}

/// 把"它到底是什么"翻译成两句给人看的话。
fn describe_kind(kind: Option<ProviderKind>) -> (&'static str, &'static str) {
    match kind {
        Some(ProviderKind::Csri) => (
            "导出表看着像 CSRI，但缺必需入口",
            "调用链需要 csri_renderer_default + csri_open_file/mem + csri_render + csri_close",
        ),
        Some(ProviderKind::LegacyVapourSynth) => (
            "这是一个 VapourSynth 插件，不是 CSRI provider",
            "12 §1C 的 legacy 链尚未实现",
        ),
        Some(ProviderKind::LegacyAviSynth) => (
            "这是一个 AviSynth 滤镜，不是 CSRI provider",
            "12 §1C 的 legacy 链尚未实现",
        ),
        Some(ProviderKind::LibassBuiltin) => (
            "这是 libass provider，应当走 libass 那条链",
            "用 --libass-dll 而不要用 CSRI 路径",
        ),
        _ => (
            "导出表里没有 CSRI 入口",
            "既不是 CSRI，也不是我们认识的 provider",
        ),
    }
}

/// 读 `csri_renderer_info()`。**可失败** —— 拿不到就返回空串，
/// 不让"报不出名字"变成"打不开 provider"。
fn read_info(api: &CsriApi, renderer: CsriRend) -> (String, String, String) {
    let Some(f) = api.renderer_info else {
        return (String::new(), String::new(), String::new());
    };
    // SAFETY：renderer 非空（调用方核过）；返回的是 `struct csri_info *`。
    // 我们只读前三个 `const char *` 字段（见 `CsriInfoPrefix` 的注释）。
    let info = unsafe { f(renderer) };
    if info.is_null() {
        return (String::new(), String::new(), String::new());
    }
    // SAFETY: 非空即指向 `csri_info`；前三个字段是指向静态字符串的指针。
    unsafe {
        let info = &*info;
        (cstr(info.name), cstr(info.longname), cstr(info.specific))
    }
}

/// # Safety
/// `p` 要么为空，要么指向以 0 结尾的 C 字符串。
unsafe fn cstr(p: *const i8) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    // 上限只是防"对方没给结尾 0"时读穿内存；正常名字都很短。
    const MAX: usize = 512;
    while len < MAX && *p.add(len) != 0 {
        len += 1;
    }
    let bytes = std::slice::from_raw_parts(p as *const u8, len);
    String::from_utf8_lossy(bytes).to_string()
}

fn path_to_c_string(p: &Path) -> Result<Vec<u8>> {
    let s = p.to_str().ok_or_else(|| {
        StkError::simple(
            codes::BAD_SCRIPT_PATH.0,
            codes::BAD_SCRIPT_PATH.1,
            "脚本路径不是合法 UTF-8",
        )
        .with_detail(p.display().to_string())
    })?;
    Ok(s.as_bytes().to_vec())
}

fn sha256_of_file(path: &Path) -> String {
    use sha2::Digest as _;
    match std::fs::read(path) {
        Ok(b) => {
            let mut h = sha2::Sha256::new();
            h.update(&b);
            h.finalize().iter().map(|x| format!("{x:02x}")).collect()
        }
        Err(_) => String::new(),
    }
}

/// 找一个候选 provider，**在给定的目录里**。
///
/// 为什么必须由调用方给目录：ADR-005 与 23 §安全都要求
/// **不擅自扫系统目录、不搜当前目录** —— 那会变成 DLL 劫持面。
/// 用户要么显式给路径，要么显式给一个目录。
pub fn candidates_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case("dll"))
                .unwrap_or(false)
        })
        .collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vapoursynth_plugin_is_described_as_such() {
        let (what, why) = describe_kind(Some(ProviderKind::LegacyVapourSynth));
        assert!(what.contains("VapourSynth"), "{what}");
        assert!(why.contains("12 §1C"), "{why}");
    }

    #[test]
    fn a_libass_provider_is_redirected_to_the_libass_chain() {
        let (what, why) = describe_kind(Some(ProviderKind::LibassBuiltin));
        assert!(what.contains("libass"), "{what}");
        assert!(why.contains("--libass-dll"), "{why}");
    }

    #[test]
    fn an_unknown_kind_says_it_is_not_recognised() {
        let (what, _) = describe_kind(None);
        assert!(what.contains("没有 CSRI 入口"), "{what}");
    }

    /// 我们愿意接受的格式只有那两个 32 位的。
    ///
    /// 这条钉住"不悄悄接受 24 位或 YUV"—— 那些需要另一套行距与转换。
    #[test]
    fn only_thirty_two_bit_formats_are_wanted() {
        assert_eq!(WANTED_PIXFMTS.len(), 2);
        for f in WANTED_PIXFMTS {
            assert_eq!(f.bytes_per_pixel(), Some(4), "{} 不是 32 位", f.label());
        }
        // BGRA 优先：它有真 alpha
        assert_eq!(WANTED_PIXFMTS[0], PixFmt::Bgra);
        assert!(WANTED_PIXFMTS[0].has_alpha());
        // 退路 BGR_ 没有真 alpha（第 4 字节是填充）
        assert!(!WANTED_PIXFMTS[1].has_alpha());
    }

    #[test]
    fn candidates_only_lists_dlls_and_sorts_them() {
        let d = std::env::temp_dir().join(format!("stk-csri-cand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("b.dll"), b"x").unwrap();
        std::fs::write(d.join("a.dll"), b"x").unwrap();
        std::fs::write(d.join("readme.txt"), b"x").unwrap();
        let v = candidates_in(&d);
        assert_eq!(v.len(), 2, "只应当列出 DLL：{v:?}");
        assert!(v[0].ends_with("a.dll"));
        assert!(v[1].ends_with("b.dll"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn candidates_of_a_missing_dir_is_empty_not_an_error() {
        assert!(candidates_in(Path::new("Z:/nope")).is_empty());
    }

    /// 非 UTF-8 的路径要报错而不是 panic。
    #[test]
    fn a_non_utf8_script_path_is_an_error() {
        #[cfg(windows)]
        {
            use std::ffi::OsString;
            use std::os::windows::ffi::OsStringExt;
            // 单独代理项：不是合法 UTF-8，也不是合法 UTF-16 序列
            let bad = std::path::PathBuf::from(OsString::from_wide(&[0xD800]));
            assert!(path_to_c_string(&bad).is_err());
        }
    }

    #[test]
    fn cstr_handles_null_empty_and_normal() {
        assert_eq!(unsafe { cstr(std::ptr::null()) }, "");
        let empty = b"\0";
        assert_eq!(
            unsafe { cstr(empty.as_ptr() as *const i8) },
            "",
            "空串不该被当成缺失"
        );
        let hello = b"libass\0";
        assert_eq!(unsafe { cstr(hello.as_ptr() as *const i8) }, "libass");
    }
}
