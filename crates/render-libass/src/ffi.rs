//! 运行时绑定（12 §1A、§进程边界）。
//!
//! 两条刻意的决定：
//! 1. **不在编译期链接 libass**：用 `LoadLibraryExW` + `GetProcAddress` 在运行时取符号。
//!    native 依赖缺失时工程照样能构建、能测试，"缺依赖"因此是一条可报告的状态，
//!    而不是整棵依赖树编译失败；DLL 的选择也留给 provider 探测。
//! 2. **不靠文件名推断能力**（12 §探测）：必须逐个符号名取到地址，取不到就报缺哪几个。
//!
//! 结构体镜像取自 `libass/ass.h:62-86`（libass 0.17.5，SHA-256 见 `SOURCE_AUDIT_LOCK.json`；
//! 注意 `ASS_Image` 定义在 `ass.h` 而不是 `ass_types.h`）。
//! 头文件说明"新字段只会追加在末尾"，所以我们只读到自己认识的那个字段为止。

use core::ffi::{c_int, c_uchar, c_void};
use std::ffi::CString;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use core_model::error::{Domain, Result, StkError};

/// 一个可用的 libass provider 必须导出的符号。
///
/// 规则：这张表**恰好等于** `provider::Symbols` 真正会调用的函数。
/// 列多了会把能正常工作的 DLL 判成不可用（本机 ass-9.dll 就不导出某些内部/新增入口），
/// 列少了则会在真正调用时才炸 —— 两种都比"按实际使用面检查"差。
pub const LIBASS_REQUIRED_SYMBOLS: &[&str] = &[
    "ass_library_init",
    "ass_library_done",
    "ass_library_version",
    "ass_renderer_init",
    "ass_renderer_done",
    "ass_set_frame_size",
    "ass_set_storage_size",
    "ass_set_fonts",
    "ass_read_memory",
    "ass_free_track",
    "ass_render_frame",
];

/// 字体注入用的可选能力符号（17 §7）。
///
/// 这里只列 `ass_add_font`，而且是有意的两件事叠在一起：
/// * **不列进 `LIBASS_REQUIRED_SYMBOLS`**：核心渲染路径不调用它，把可选能力做成硬要求，
///   会把本来能正常工作的 DLL 判成"不可用 provider"；
/// * **不要求 `ass_clear_fonts`**：头文件明写"只有在与该 library 句柄关联的所有
///   ASS_Track 与 ASS_Renderer 都已释放之后才能安全调用"。我们的 Provider 活着的时候
///   renderer 就活着，所以这个函数在我们的生命周期里根本不能调用 —— 把一个不能用的符号
///   写进能力清单，等于宣称了一个做不到的事。内存字体随 `ass_library_done` 一起释放。
pub const LIBASS_FONT_SYMBOLS: &[&str] = &["ass_add_font"];

/// `ASS_Image` 的 Rust 镜像；字段顺序与类型必须与头文件一致（测试逐字段验偏移）。
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AssImage {
    pub w: c_int,
    pub h: c_int,
    pub stride: c_int,
    pub bitmap: *mut c_uchar,
    pub color: u32,
    pub dst_x: c_int,
    pub dst_y: c_int,
    pub next: *mut AssImage,
    pub kind: u32,
}

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
    fn FreeLibrary(module: *mut c_void) -> i32;
}

/// DLL 句柄。Drop 时释放；`Send` 与否由 Windows 语义决定，这里保守地不实现 Send。
pub struct LibHandle {
    module: *mut c_void,
}

/// 把路径转成 `LoadLibraryExW` 能接受的 UTF-16 缓冲。
///
/// 必须走 `components()` 而不是直接 `encode_wide()`：WinAPI 的 DLL 加载**不认正斜杠**，
/// `C:/tools/vcpkg/.../ass-9.dll` 会直接失败，而环境变量里传来的 Windows 路径
/// 恰恰几乎都是正斜杠（bash、CMake、Python、vcpkg 自己都是这种风格）。
/// `Path::components()` 在 Windows 上把两种分隔符都当作分隔符，重组时统一成反斜杠，
/// 同时保留 `\\?\` / UNC 前缀，也不破坏非 UTF-8 的 OsStr。
fn wide_path(path: &Path) -> Vec<u16> {
    let mut normalized = PathBuf::new();
    for c in path.components() {
        normalized.push(c.as_os_str());
    }
    normalized
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

impl LibHandle {
    /// 加载策略（23 §安全）：不搜当前目录、不搜系统目录，避免把安装目录旁边的
    /// 同名 DLL 变成劫持面。
    pub fn open(path: &Path) -> Result<LibHandle> {
        // LOAD_LIBRARY_SEARCH_* 只按"完整路径"工作：相对路径即便文件真的存在，
        // 加载也会以"找不到文件"收场（门阶里用 `../tools/...` 实测踩过，ISSUES Z4）。
        // 所以这里按当前目录补全，再把分隔符规范化。
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let wide = wide_path(&absolute);
        const LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR: u32 = 0x100;
        const LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: u32 = 0x1000;
        const FLAGS: u32 = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS;
        // SAFETY: wide 是以 0 结尾的 UTF-16 缓冲，生命周期覆盖这次调用
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), FLAGS) };
        if module.is_null() {
            // 必须把 Win32 错误码原样带出来（22 §诊断）：只看"加载失败"根本分不开
            // "路径不存在"（2/3）、"缺依赖 DLL"（126）和"架构不对"（193）这三种病因，
            // 而这三者的修法完全不同。把原因写死成"native 依赖未落地"就是在骗人。
            let os = std::io::Error::last_os_error();
            return Err(StkError::simple(Domain::Render, 71, "无法加载 libass DLL")
                .with_detail(format!(
                    "{absolute:?}：{os}（错误码 {}）",
                    os.raw_os_error().unwrap_or(-1)
                ))
                .with_action(
                    "依次核对：路径是否绝对且存在、该 DLL 的同目录依赖是否齐备、位数是否为 x64",
                ));
        }
        Ok(LibHandle { module })
    }

    pub fn symbol(&self, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        // SAFETY: module 非空（构造时检查过），c 是有效的 NUL 结尾字符串
        let addr = unsafe { GetProcAddress(self.module, c.as_ptr()) };
        (!addr.is_null()).then_some(addr)
    }
}

impl Drop for LibHandle {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.module);
        }
    }
}

impl core::fmt::Debug for LibHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // 只报句柄地址：模块名可能含用户路径，诊断里不该反复抄
        f.debug_struct("LibHandle")
            .field("module", &format_args!("{:p}", self.module))
            .finish()
    }
}

#[derive(Debug)]
pub struct ProbeReport {
    pub path: String,
    pub found: Vec<&'static str>,
    pub missing: Vec<&'static str>,
    /// 这份 DLL 能不能做字体注入（17 §7）。缺哪个符号名记在 `missing_font_symbols` 里。
    pub can_inject_fonts: bool,
    pub missing_font_symbols: Vec<&'static str>,
}

impl ProbeReport {
    pub fn is_usable(&self) -> bool {
        self.missing.is_empty()
    }
}

/// 探测一个候选 DLL：加载 → 逐个符号名检查 → 缺哪个报哪个。
pub fn probe(path: &Path) -> Result<(LibHandle, ProbeReport)> {
    let handle = LibHandle::open(path)?;
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for name in LIBASS_REQUIRED_SYMBOLS {
        if handle.symbol(name).is_some() {
            found.push(*name);
        } else {
            missing.push(*name);
        }
    }
    if !missing.is_empty() {
        return Err(
            StkError::simple(Domain::Render, 72, "候选 DLL 不是可用的 libass provider")
                .with_detail(format!(
                    "{path:?} 缺 {} 个必需符号，例如 {:?}",
                    missing.len(),
                    &missing[..missing.len().min(3)]
                ))
                .with_action("按导出符号判定能力，不看文件名（12 §探测）"),
        );
    }
    // 能力探测与"是不是 provider"分开算：这里只记录，不参与可用性判定
    let mut font_have = Vec::new();
    let mut font_missing = Vec::new();
    for name in LIBASS_FONT_SYMBOLS {
        if handle.symbol(name).is_some() {
            font_have.push(*name);
        } else {
            font_missing.push(*name);
        }
    }
    Ok((
        handle,
        ProbeReport {
            path: path.display().to_string(),
            found,
            missing,
            can_inject_fonts: font_missing.is_empty(),
            missing_font_symbols: font_missing,
        },
    ))
}

/// 链表遍历上限：损坏的 `next`（自环）不该让我们转无穷圈。
pub const MAX_IMAGE_CHAIN: usize = 4096;

/// 沿 `next` 收集成拥有所有权的 Vec。
///
/// # Safety
/// `head` 必须指向一段在这段调用期内稳定有效的 `ASS_Image` 链（libass 自己分配的，
/// 或测试里自己构造的合法节点）。
pub unsafe fn walk_chain(head: *const AssImage) -> Result<Vec<AssImage>> {
    let mut out = Vec::new();
    let mut cur = head;
    while !cur.is_null() {
        if out.len() >= MAX_IMAGE_CHAIN {
            return Err(
                StkError::simple(Domain::Render, 70, "ASS_Image 链表超过遍历上限")
                    .with_detail(format!("超过 {MAX_IMAGE_CHAIN} 个节点：疑似 next 指针成环"))
                    .with_action("停止合成这一帧并上报 renderer 异常，不要继续读未知内存"),
            );
        }
        // SAFETY: 已判空；节点是 Copy 的 POD 镜像
        out.push(unsafe { *cur });
        cur = unsafe { (*cur).next }.cast_const();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NULL_IMAGE: AssImage = AssImage {
        w: 0,
        h: 0,
        stride: 0,
        bitmap: std::ptr::null_mut(),
        color: 0,
        dst_x: 0,
        dst_y: 0,
        next: std::ptr::null_mut(),
        kind: 0,
    };

    #[test]
    fn missing_library_is_a_reported_error_not_a_panic() {
        let e = LibHandle::open(Path::new("Z:/definitely/not/here/ass.dll")).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0071");
        // 诊断必须带上真实的 Win32 错误码：三种病因（路径不存在 / 缺依赖 / 位数不对）
        // 的修法完全不同，只说"加载失败"等于没诊断（22 §诊断）
        assert!(e.details.iter().any(|d| d.contains("错误码")), "{e:?}");
        assert!(
            e.user_action
                .as_deref()
                .unwrap_or_default()
                .contains("位数"),
            "{e:?}"
        );
    }

    #[test]
    fn forward_slash_paths_are_normalized_for_the_loader() {
        // LoadLibraryExW 不认正斜杠，而环境变量传来的 Windows 路径基本都是正斜杠。
        // 这一步走错，报出来的就是"无法加载 DLL"这种查不出原因的假故障（ISSUES Z2）。
        let wide = wide_path(Path::new("C:/tools/vcpkg/x64-windows/bin/ass-9.dll"));
        let s = String::from_utf16(&wide[..wide.len() - 1]).unwrap();
        assert_eq!(s, r"C:\tools\vcpkg\x64-windows\bin\ass-9.dll");
        assert!(wide.last() == Some(&0), "必须是 NUL 结尾的 UTF-16");

        // 反斜杠路径不能被"规范化"坏，也不能重复加分隔符
        let back = wide_path(Path::new(r"C:\a\b\ass-9.dll"));
        assert_eq!(
            String::from_utf16(&back[..back.len() - 1]).unwrap(),
            r"C:\a\b\ass-9.dll"
        );

        // 相对路径保持相对（不假装绝对，避免指错目录）
        let rel = wide_path(Path::new("bin/ass-9.dll"));
        assert_eq!(
            String::from_utf16(&rel[..rel.len() - 1]).unwrap(),
            r"bin\ass-9.dll"
        );
    }

    #[test]
    fn capability_is_decided_by_symbols_not_by_the_file_name() {
        // 拿自己的测试可执行文件冒充 provider：必须被符号检查拦下
        let exe = std::env::current_exe().unwrap();
        let e = probe(&exe).unwrap_err();
        assert!(
            e.code_string() == "STK-RND-0071" || e.code_string() == "STK-RND-0072",
            "{e:?}"
        );
        if e.code_string() == "STK-RND-0072" {
            assert!(
                e.details.iter().any(|d| d.contains("符号")),
                "{:?}",
                e.details
            );
        }
    }

    #[test]
    fn chain_walk_is_ordered_terminates_and_bounds_cycles() {
        let mut n3 = Box::new(AssImage {
            color: 3,
            ..NULL_IMAGE
        });
        let mut n2 = Box::new(AssImage {
            color: 2,
            ..NULL_IMAGE
        });
        let mut n1 = Box::new(AssImage {
            color: 1,
            ..NULL_IMAGE
        });
        n2.next = Box::as_mut_ptr(&mut n3);
        n1.next = Box::as_mut_ptr(&mut n2);

        let got = unsafe { walk_chain(Box::as_ptr(&n1)) }.unwrap();
        assert_eq!(got.len(), 3);
        assert_eq!(
            got.iter().map(|i| i.color).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "顺序必须就是 next 链表的顺序（ass.h:56 要求按序合成）"
        );
        assert!(unsafe { walk_chain(std::ptr::null()) }.unwrap().is_empty());

        // 两节点环：必须被上限拦下，而不是转到天荒地老
        let mut a = Box::new(AssImage {
            color: 1,
            ..NULL_IMAGE
        });
        let mut b = Box::new(AssImage {
            color: 2,
            ..NULL_IMAGE
        });
        let pa: *mut AssImage = Box::as_mut_ptr(&mut a);
        let pb: *mut AssImage = Box::as_mut_ptr(&mut b);
        unsafe {
            (*pa).next = pb;
            (*pb).next = pa;
        }
        let e = unsafe { walk_chain(Box::as_ptr(&a)) }.unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0070");
    }

    #[test]
    fn mirrored_layout_matches_the_c_header() {
        // 只测 64 位（本工程唯一 target：x86_64-pc-windows-msvc）
        if !cfg!(target_pointer_width = "64") {
            return;
        }
        let img = NULL_IMAGE;
        let p = std::ptr::addr_of!(img);
        let at = |field: *const u8| (field as usize) - (p as usize);
        assert_eq!(at(std::ptr::addr_of!(img.w).cast::<u8>()), 0);
        assert_eq!(at(std::ptr::addr_of!(img.h).cast::<u8>()), 4);
        assert_eq!(at(std::ptr::addr_of!(img.stride).cast::<u8>()), 8);
        assert_eq!(at(std::ptr::addr_of!(img.bitmap).cast::<u8>()), 16);
        assert_eq!(at(std::ptr::addr_of!(img.color).cast::<u8>()), 24);
        assert_eq!(at(std::ptr::addr_of!(img.dst_x).cast::<u8>()), 28);
        assert_eq!(at(std::ptr::addr_of!(img.dst_y).cast::<u8>()), 32);
        assert_eq!(at(std::ptr::addr_of!(img.next).cast::<u8>()), 40);
        assert_eq!(at(std::ptr::addr_of!(img.kind).cast::<u8>()), 48);
        assert_eq!(core::mem::size_of::<AssImage>(), 56);
    }
}
