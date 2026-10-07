//! 注册字体枚举：17 §2 说的"字体注册"那一半边。
//!
//! 为什么不是 DirectWrite：17 §2 的权威确实是 DirectWrite（AG1 记着这条边界），但
//! "某个家族名指向哪个文件"这件事，登记处就是注册表
//! `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts` 与同名 `HKCU` 键——
//! 只扫 `%WINDIR%\Fonts` 会漏掉：
//! * 装在其它目录却被注册进来的字体（老程序、Office/Adobe 自带）；
//! * 用户从"设置→字体"注册的、文件名与家族名完全不同的字体。
//!
//! 这里读的是那张登记表，仍然**不是** DirectWrite 的 fallback 链：那需要 COM
//! `IDWriteFactory::CreateSystemFontCollection`，没做（AG1）。
//!
//! 每条注册项都要有交代：解析不到文件的必须留下原因，而不是静悄悄少一个字体
//! （与 `db::FontDb` 里 `ScanNote` 同一纪律）。

use std::path::{Path, PathBuf};

/// 注册表里的一条字体登记。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredFont {
    /// "HKLM" 或 "HKCU"
    pub hive: String,
    /// 值名，形如 `Arial (TrueType)`
    pub display_name: String,
    /// 值原文（可能是文件名、绝对路径，也可能带 `%VAR%`）
    pub data: String,
    /// 解析出来的实际文件；`None` 时 `note` 必须说明为什么
    pub path: Option<PathBuf>,
    pub note: Option<String>,
}

/// 一次枚举的完整结果：条目 + 打开键本身的失败（后者是环境缺失，不是"没有字体"）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryReport {
    pub entries: Vec<RegisteredFont>,
    /// `(hive, 说明)`：键打不开时必须说清，不能让"没读到"伪装成"库里没有"
    pub open_errors: Vec<(String, String)>,
    /// 某个键下的**单条**值读不了：数量与原因都要报，不能让"少了几条"无声发生
    pub field_errors: Vec<(String, String)>,
    /// 本平台没有注册表这套机制（与"有但读不到"区分开）
    pub unsupported: bool,
}

impl RegistryReport {
    /// 解析成功、且带字体后缀的文件集合——喂给 `FontDb` 与目录扫到的合并去重。
    pub fn font_files(&self) -> Vec<PathBuf> {
        use crate::db::FONT_EXTS;
        self.entries
            .iter()
            .filter_map(|e| e.path.clone())
            .filter(|p| {
                p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| FONT_EXTS.contains(&s.to_ascii_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .collect()
    }

    /// 报告用的聚合：登记了多少、解析到多少、失败原因分布。
    pub fn summary(&self) -> Vec<(String, usize)> {
        use std::collections::BTreeMap;
        let mut m: BTreeMap<String, usize> = BTreeMap::new();
        for e in &self.entries {
            let key = match (&e.path, &e.note) {
                (Some(_), _) => "解析到文件".to_string(),
                (None, Some(n)) => n.clone(),
                (None, None) => "没有原因（这是 bug）".to_string(),
            };
            *m.entry(key).or_insert(0) += 1;
        }
        m.into_iter().collect()
    }
}

/// 把注册表值原文解析成一个可能存在的文件路径。
///
/// `windir_fonts` 是 `%WINDIR%\Fonts`，`user_fonts` 是
/// `%LOCALAPPDATA%\Microsoft\Windows\Fonts`（1703 起用户自装字体在这里）。
/// 查找顺序是**先用户目录再系统目录**：同名时用户注册的覆盖系统那份，
/// 这与 Windows 自己的解析次序一致，也是"这台机器会给什么"的真实答案。
///
/// `file_exists` 作为参数传进来，是为了让这套规则可以在任何机器上被测（不必真有注册表）。
pub fn resolve_value(
    data: &str,
    windir_fonts: &Path,
    user_fonts: &Path,
    file_exists: &dyn Fn(&Path) -> bool,
) -> (Option<PathBuf>, Option<String>) {
    let raw = data.trim();
    if raw.is_empty() {
        return (None, Some("注册值为空".to_string()));
    }
    let expanded = expand_percent_vars(raw);
    let candidate = match expanded.as_ref() {
        Ok(path) => PathBuf::from(path),
        Err(var) => {
            return (
                None,
                Some(format!("注册值里的环境变量 %{var}% 没有定义，无法定位文件")),
            );
        }
    };
    // 注册表里常见的相对写法：`\ARIAL.TTF`、`.\Fonts\x.ttf` 一律按文件名处理
    let name = candidate
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return (None, Some("注册值指向的不是文件".to_string()));
    }
    if candidate.is_absolute() && file_exists(&candidate) {
        return (Some(candidate), None);
    }
    for dir in [user_fonts, windir_fonts] {
        let p = dir.join(&name);
        if file_exists(&p) {
            return (Some(p), None);
        }
    }
    if candidate.is_absolute() {
        return (
            None,
            Some(format!("注册指向不存在的文件：{}", candidate.display())),
        );
    }
    (
        None,
        Some(format!("注册的文件名在用户与系统字体目录都不存在：{name}")),
    )
}

/// 展开 `%NAME%`。未定义的变量返回 `Err(名字)`——展开不了就必须报告，不能猜一个路径。
fn expand_percent_vars(s: &str) -> Result<String, String> {
    if !s.contains('%') {
        return Ok(s.to_string());
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        match tail.find('%') {
            Some(end) => {
                let var = &tail[..end];
                if var.is_empty() {
                    // `%%` 在注册表里就是字面量一个百分号
                    out.push('%');
                    rest = &tail[end + 1..];
                    continue;
                }
                match std::env::var(var) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => return Err(var.to_string()),
                }
                rest = &tail[end + 1..];
            }
            None => {
                // 落单的百分号：按字面量保留
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// 枚举 HKLM 与 HKCU 的 Fonts 键。非 Windows 平台返回 `unsupported = true`，
/// 这是"本平台没有这套机制"，与"有机制但读不到"（`open_errors`）严格区分。
pub fn enumerate() -> RegistryReport {
    let mut report = RegistryReport::default();
    if !cfg!(windows) {
        report.unsupported = true;
        return report;
    }
    #[cfg(windows)]
    {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let windir_fonts = Path::new(&windir).join("Fonts");
        let user_fonts = std::env::var("LOCALAPPDATA")
            .map(|a| {
                Path::new(&a)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Fonts")
            })
            .unwrap_or_else(|_| windir_fonts.clone());
        for (hive, hive_key, sub) in [
            (
                "HKLM",
                win::HKLM,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts",
            ),
            (
                "HKCU",
                win::HKCU,
                r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts",
            ),
        ] {
            match win::enum_values(hive_key, sub) {
                Ok((rows, row_failures)) => {
                    for why in row_failures {
                        report.field_errors.push((hive.to_string(), why));
                    }
                    for (name, kind, data) in rows {
                        let (path, note) = match kind {
                            // 只有字符串型才是"指向文件"；其它（如 REG_BINARY 的遗留项）如实说明
                            win::REG_SZ | win::REG_EXPAND_SZ => {
                                resolve_value(&data, &windir_fonts, &user_fonts, &|p| p.is_file())
                            }
                            other => (
                                None,
                                Some(format!("注册值类型不是字符串（type={other}），跳过")),
                            ),
                        };
                        report.entries.push(RegisteredFont {
                            hive: hive.to_string(),
                            display_name: name,
                            data,
                            path,
                            note,
                        });
                    }
                }
                Err(code) => report
                    .open_errors
                    .push((hive.to_string(), format!("{sub}：{code}"))),
            }
        }
        // 顺序必须稳定：两次枚举给出同一份数据库，否则"字体环境变了"与"枚举顺序变了"分不开
        report
            .entries
            .sort_by(|a, b| (&a.hive, &a.display_name).cmp(&(&b.hive, &b.display_name)));
    }
    report
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub const REG_SZ: u32 = 1;
    pub const REG_EXPAND_SZ: u32 = 2;

    pub const HKLM: usize = 0x8000_0002;
    pub const HKCU: usize = 0x8000_0001;

    const KEY_READ: u32 = 0x20019;
    // 64 位进程读 64 位视图；不带这张表的话在 WOW64 下会被重定向到 32 位视图
    const KEY_WOW64_64KEY: u32 = 0x0100;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_NO_MORE_ITEMS: i32 = 259;
    const ERROR_MORE_DATA: i32 = 234;

    type HKey = *mut c_void;
    type Long = i32;
    type UChar = u16;
    /// 一条登记：值名、注册表类型、字符串内容
    type Row = (String, u32, String);
    /// 读到的行 + 逐条读取失败的原因
    type Rows = (Vec<Row>, Vec<String>);

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(
            hkey: HKey,
            sub_key: *const UChar,
            options: u32,
            sam: u32,
            result: *mut HKey,
        ) -> Long;
        fn RegCloseKey(hkey: HKey) -> Long;
        fn RegEnumValueW(
            hkey: HKey,
            index: u32,
            name: *mut UChar,
            name_len: *mut u32,
            // lpReserved 是**指针**（必须为 NULL）。按 u32 传会让后面几个栈参数整体错位，
            // 表现是每条都返回 ERROR_INVALID_PARAMETER(87)（ISSUES AH1）
            reserved: *mut u32,
            kind: *mut u32,
            data: *mut u8,
            data_len: *mut u32,
        ) -> Long;
    }

    /// Win32 错误码翻译：报告里必须说清是"键不存在"还是"读不了"，
    /// 不能只给一个数字，也不能把两者混成"没有注册字体"。
    fn error_text(code: i32) -> String {
        let name = match code {
            ERROR_SUCCESS => "成功",
            ERROR_FILE_NOT_FOUND => "键不存在（这台机器没有登记过字体）",
            5 => "拒绝访问（当前身份读不了这个键）",
            other => return format!("Win32 错误 {other}"),
        };
        format!("Win32 错误 {code}：{name}")
    }

    fn wide(s: &str) -> Vec<UChar> {
        let mut v: Vec<UChar> = s.encode_utf16().collect();
        v.push(0);
        v
    }

    fn from_wide(ptr: *const UChar, units: usize) -> String {
        if ptr.is_null() {
            return String::new();
        }
        let slice = unsafe { std::slice::from_raw_parts(ptr, units) };
        String::from_utf16_lossy(slice)
    }

    /// 指定 hive 下一个键的所有值。
    ///
    /// 返回读到的行 + **逐条读取失败的原因**（后者绝不能吞掉：一个空列表配上空错误列表，
    /// 是这套枚举最危险的形状）。
    ///
    /// 这里**不用** `RegQueryInfoKeyW` 预取长度：第一轮那么写，参数次序写错，把子键计数
    /// 当成了值的个数，于是"0 条登记项 + 0 个错误"（ISSUES AH1）。改成 `RegEnumValueW`
    /// 走到 `ERROR_NO_MORE_ITEMS` 为止，只需要一个 API，条数由循环自己数。
    pub fn enum_values(hive: usize, sub_key: &str) -> Result<Rows, String> {
        let mut h: HKey = std::ptr::null_mut();
        let name = wide(sub_key);
        let status = unsafe {
            RegOpenKeyExW(
                hive as HKey,
                name.as_ptr(),
                0,
                KEY_READ | KEY_WOW64_64KEY,
                &mut h,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(error_text(status));
        }
        let mut name_buf = vec![0u16; 1024];
        let mut data_buf = vec![0u16; 2048];
        let mut out: Vec<Row> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        let mut index = 0u32;
        let done = loop {
            if index >= 8192 {
                failures.push(format!("枚举在 {index} 条处被上限截断"));
                break true;
            }
            let mut grew = 0usize;
            let row = loop {
                let mut n_units = name_buf.len() as u32;
                let mut d_bytes = (data_buf.len() * 2) as u32;
                let mut k = 0u32;
                let st = unsafe {
                    RegEnumValueW(
                        h,
                        index,
                        name_buf.as_mut_ptr(),
                        &mut n_units,
                        std::ptr::null_mut(),
                        &mut k,
                        data_buf.as_mut_ptr() as *mut u8,
                        &mut d_bytes,
                    )
                };
                match st {
                    ERROR_SUCCESS => {
                        let vname =
                            from_wide(name_buf.as_ptr(), (n_units as usize).saturating_sub(1));
                        let units = (d_bytes as usize / 2).min(data_buf.len());
                        let raw = from_wide(data_buf.as_ptr() as *const UChar, units)
                            .trim_end_matches('\0')
                            .to_string();
                        break Ok((vname, k, raw));
                    }
                    ERROR_NO_MORE_ITEMS => break Err(None),
                    ERROR_MORE_DATA if grew < 4 => {
                        // 扩容后重试同一条：字体登记字符串很短，但边界不靠猜
                        grew += 1;
                        let need_bytes = d_bytes as usize;
                        if need_bytes > data_buf.len() * 2 {
                            data_buf = vec![0u16; need_bytes / 2 + 64];
                        } else {
                            name_buf = vec![0u16; name_buf.len() * 2];
                        }
                    }
                    other => break Err(Some(error_text(other))),
                }
            };
            match row {
                Ok(r) => {
                    out.push(r);
                    index += 1;
                }
                Err(None) => break true,
                Err(Some(why)) => {
                    failures.push(format!("第 {index} 条读不了：{why}"));
                    index += 1;
                }
            }
        };
        let _ = done;
        unsafe {
            RegCloseKey(h);
        }
        Ok((out, failures))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn exists(set: Vec<PathBuf>) -> impl Fn(&Path) -> bool {
        move |p| set.iter().any(|q| q == p)
    }

    /// 三种真实写法：光文件名、用户目录同名覆盖系统、绝对路径。
    #[test]
    fn resolution_covers_the_shapes_the_registry_actually_uses() {
        let sys = PathBuf::from(r"C:\Windows\Fonts");
        let user = PathBuf::from(r"C:\Users\a\AppData\Local\Microsoft\Windows\Fonts");
        let hits = vec![
            sys.join("arial.ttf"),
            user.join("MyFont.ttf"),
            PathBuf::from(r"D:\fonts\Office.ttf"),
        ];
        let f = exists(hits);
        let (p, note) = resolve_value("arial.ttf", &sys, &user, &f);
        assert_eq!(p, Some(sys.join("arial.ttf")), "{note:?}");
        let (p, _) = resolve_value("MyFont.ttf", &sys, &user, &f);
        assert_eq!(
            p,
            Some(user.join("MyFont.ttf")),
            "用户注册的必须盖过系统目录那份"
        );
        let (p, _) = resolve_value(r"D:\fonts\Office.ttf", &sys, &user, &f);
        assert_eq!(p, Some(PathBuf::from(r"D:\fonts\Office.ttf")));
        // 相对写法（`\x.ttf`、`.\Fonts\x.ttf`）按文件名落到两个目录
        let (p, _) = resolve_value(r"\arial.ttf", &sys, &user, &f);
        assert_eq!(p, Some(sys.join("arial.ttf")), "反斜杠前缀是相对写法");
    }

    /// 解析不到必须给原因：注册表里留着一条死链接是常事（卸载残留、手动删文件）。
    #[test]
    fn unresolvable_values_get_a_reason_not_silence() {
        let sys = PathBuf::from(r"C:\Windows\Fonts");
        let user = PathBuf::from(r"C:\Users\a\AppData\Local\Microsoft\Windows\Fonts");
        let f = exists(vec![]);
        let (p, note) = resolve_value("gone.ttf", &sys, &user, &f);
        assert!(p.is_none());
        assert!(note.unwrap().contains("不存在"), "原因必须说清是哪类失败");
        let (p, note) = resolve_value("   ", &sys, &user, &f);
        assert!(p.is_none() && note.unwrap().contains("为空"));
    }

    /// `%VAR%` 展开：未定义就报错，绝不猜一个路径出来。
    #[test]
    fn percent_vars_are_expanded_or_reported() {
        std::env::set_var("STK_TEST_FONTS_DIR", r"D:\registered");
        let sys = PathBuf::from(r"C:\Windows\Fonts");
        let user = PathBuf::from(r"C:\Users\a\AppData\Local");
        let f = exists(vec![PathBuf::from(r"D:\registered\x.ttf")]);
        let (p, note) = resolve_value("%STK_TEST_FONTS_DIR%\\x.ttf", &sys, user.as_path(), &f);
        assert_eq!(p, Some(PathBuf::from(r"D:\registered\x.ttf")), "{note:?}");
        let (p, note) = resolve_value("%STK_NO_SUCH_VAR_ZZ%\\y.ttf", &sys, &user, &f);
        assert!(p.is_none(), "展开不了不能给路径");
        assert!(
            note.unwrap().contains("STK_NO_SUCH_VAR_ZZ"),
            "必须点名是哪个变量"
        );
    }

    /// 非 Windows 平台是"没有这套机制"，不是"读不到"——两者不能混成一个结论。
    #[test]
    fn enumeration_distinguishes_unsupported_from_failure() {
        let r = enumerate();
        if cfg!(windows) {
            // 这台机器上 HKLM 的 Fonts 键一定有登记项；空表 + 空错误就是"读法错"的样子
            // （第一轮就是因为 RegQueryInfoKeyW 的参数次序错，把子键计数当成了值个数）
            assert!(
                !r.entries.is_empty(),
                "Windows 上枚举到 0 条登记项，而打开键也没失败：这不可能同时成立，说明读法错了：{r:?}"
            );
            assert!(
                r.open_errors.is_empty(),
                "标准键打不开要当环境异常看：{:?}",
                r.open_errors
            );
            assert!(
                r.entries.iter().any(|e| e.path.is_some()),
                "一条都解析不到文件，等于这项能力没生效：{r:?}",
            );
            for e in &r.entries {
                if e.path.is_none() {
                    assert!(e.note.is_some(), "{e:?} 解析失败却没给原因");
                }
            }
            let again = enumerate();
            assert_eq!(
                again.entries, r.entries,
                "两次枚举必须给同一份顺序，否则数据库无法比对"
            );
        } else {
            assert!(
                r.unsupported,
                "本平台应报 unsupported，而不是空表冒充\"没有字体\""
            );
        }
    }
}
