//! 运行时加载一个 native DLL，并按名字取符号。
//!
//! ## 为什么这一份住在 core-model
//!
//! 现有的两条 provider 链（libass / CSRI）都要"运行时加载、绝不编译期链接"，
//! 而加载策略里每条都踩过坑（见下）。**两份实现对同一个 DLL 会给出不同答案** ——
//! 一边放行一边拒绝，而用户看到的是一条自相矛盾的报错。
//! 这与 [`crate::pe`] 的位数判断是同一条理由，所以放在同一层。
//!
//! ## 三条不是风格的细节
//!
//! **一、不搜当前目录。** 用 `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
//! LOAD_LIBRARY_SEARCH_DEFAULT_DIRS`。默认的 `LoadLibrary` 会先搜当前目录，
//! 于是"安装目录旁边放一个同名 DLL"就成了劫持面（23 §安全）。
//!
//! **二、必须按完整路径加载。** `LOAD_LIBRARY_SEARCH_*` 只对完整路径生效；
//! 相对路径即便文件真的存在也会以"找不到文件"收场（门阶里用 `../tools/...`
//! 实测踩过，ISSUES Z4）。所以这里按当前目录补全。
//!
//! **三、分隔符要规范化。** WinAPI 的 DLL 加载**不认正斜杠** ——
//! `C:/tools/.../ass-9.dll` 直接失败，而环境变量里传来的 Windows 路径
//! 恰恰几乎都是正斜杠（bash、CMake、Python、vcpkg 自己都是这种风格）。
//! 走 `Path::components()` 让两种分隔符都被当作分隔符，
//! 同时保留 `\\?\` / UNC 前缀。
//!
//! ## Drop 只释放句柄
//!
//! `FreeLibrary` 减少引用计数。注意：**它不保证卸载** ——
//! 如果还有人持有这个模块里的函数指针，模块会留在地址空间里。
//! 所以调用方必须先把自己持有的资源（如 provider 实例）释放掉，
//! 再让它 drop（各 provider 的 `Drop` 里就是这么排的）。

use std::ffi::{c_void, CString};
use std::path::{Path, PathBuf};

use crate::error::{Domain, Result, StkError};

mod sys {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const i8) -> *mut c_void;
        pub fn FreeLibrary(module: *mut c_void) -> i32;
        pub fn GetLastError() -> u32;
    }
}

/// 已加载的 DLL 句柄。
///
/// **不实现 `Send`**（与项目里 native 句柄的既有取向一致）：
/// 句柄本身可以跨线程用，但持有它的 provider 是 native 代码，
/// 跨线程移动要它自己保证线程安全，而那些库不提供这样的承诺。
/// 保守地不实现，比"看起来能编译但偶发崩"好。
pub struct LibHandle {
    module: *mut c_void,
    path: PathBuf,
}

impl LibHandle {
    /// 按**完整路径**加载。见模块头部三条细节。
    pub fn open(path: &Path) -> Result<LibHandle> {
        // `LOAD_LIBRARY_SEARCH_*` 只按完整路径工作：相对路径即便文件真的存在，
        // 加载也会以"找不到文件"收场（ISSUES Z4）。所以先补全。
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
        let module = unsafe { sys::LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), FLAGS) };
        if module.is_null() {
            // 必须把 Win32 错误码原样带出来（22 §诊断）：只看"加载失败"根本分不开
            // "文件不在"、"位数不符"、"缺依赖 DLL"。
            // SAFETY: GetLastError 无参数、无副作用。
            let code = unsafe { sys::GetLastError() };
            return Err(StkError::simple(Domain::Render, 97, "无法加载 DLL")
                .with_detail(format!("{}：Win32 错误 {code}", absolute.display()))
                .with_action("检查路径、位数与依赖 DLL 是否齐全"));
        }
        Ok(LibHandle {
            module,
            path: absolute,
        })
    }

    /// 取一个导出符号。取不到返回 `None`（调用方决定这是不是致命的）。
    pub fn symbol(&self, name: &str) -> Option<*mut c_void> {
        let c = CString::new(name).ok()?;
        // SAFETY: module 非空（构造时检查过），c 是有效的 NUL 结尾字符串
        let addr = unsafe { sys::GetProcAddress(self.module, c.as_ptr()) };
        (!addr.is_null()).then_some(addr)
    }

    /// 这个句柄是从哪个文件加载的（已规范化为绝对路径）。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LibHandle {
    fn drop(&mut self) {
        // SAFETY: module 由成功的 LoadLibraryExW 产出，且只在这里释放一次。
        unsafe {
            sys::FreeLibrary(self.module);
        }
    }
}

impl std::fmt::Debug for LibHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 不打印 module 裸指针：它对读日志的人没有意义，而会让人以为
        // 那个数字可以拿去核对什么。
        f.debug_struct("LibHandle")
            .field("path", &self.path.display().to_string())
            .finish()
    }
}

/// 把路径转成 `LoadLibraryExW` 能接受的 UTF-16 缓冲。
///
/// 必须走 `components()` 而不是直接 `encode_wide()`：WinAPI 的 DLL 加载
/// **不认正斜杠**，而环境变量里传来的 Windows 路径恰恰几乎都是正斜杠。
pub fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_reported_with_the_path() {
        let e = LibHandle::open(Path::new("Z:/nope/definitely-not-here.dll")).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0097");
        assert!(
            e.details.join(" ").contains("definitely-not-here.dll"),
            "{:?}",
            e.details
        );
        // 必须有 Win32 错误码：只说"加载失败"分不开原因
        assert!(
            e.details.join(" ").contains("Win32 错误"),
            "{:?}",
            e.details
        );
    }

    /// 正斜杠路径也要能加载 —— 环境变量里到处都是这种写法。
    #[test]
    fn forward_slashes_are_normalised_and_do_work() {
        let system = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let forward = format!("{}/System32/kernel32.dll", system.replace('\\', "/"));
        let h = LibHandle::open(Path::new(&forward)).expect("正斜杠路径应当能加载");
        assert!(
            h.symbol("GetProcAddress").is_some(),
            "加载成功就必须能取到符号"
        );
    }

    /// 相对路径要先补成绝对路径再加载（`LOAD_LIBRARY_SEARCH_*` 的要求）。
    #[test]
    fn a_relative_path_is_made_absolute_before_loading() {
        let h = LibHandle::open(Path::new("../..")) // 故意的：不是文件
            .err()
            .map(|e| e.to_string());
        // 不管成不成，都不该 panic；报错里应当是绝对路径
        if let Some(s) = h {
            assert!(
                !s.contains("../..") || s.contains(":\\") || s.contains(":/"),
                "报错里应当已经补成绝对路径：{s}"
            );
        }
    }

    #[test]
    fn a_real_symbol_is_found_and_a_fake_one_is_not() {
        let system = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let k32 = PathBuf::from(&system).join("System32").join("kernel32.dll");
        let Ok(h) = LibHandle::open(&k32) else {
            eprintln!("跳过：加载不到 kernel32");
            return;
        };
        assert!(h.symbol("LoadLibraryExW").is_some());
        assert!(h.symbol("this_symbol_does_not_exist_anywhere").is_none());
        // 空名字不该 panic
        assert!(h.symbol("").is_none());
        // 记录了路径
        assert!(h
            .path()
            .to_string_lossy()
            .to_lowercase()
            .contains("kernel32"));
    }

    #[test]
    fn wide_path_ends_with_nul_and_normalises_separators() {
        let w = wide_path(Path::new("C:/a/b/c.dll"));
        assert_eq!(*w.last().unwrap(), 0u16);
        let s: String = String::from_utf16_lossy(&w[..w.len() - 1]);
        assert!(!s.contains('/'), "正斜杠应当被规范化成反斜杠：{s}");
    }
}
