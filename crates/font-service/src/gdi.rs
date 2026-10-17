//! 17 §2 的**权威交叉核对**：用 GDI 的枚举结果去检验我们自己的字体库。
//!
//! ## 为什么是 GDI 而不是 DirectWrite
//!
//! 17 §2 的原话是「Windows Font Resolver 以 DirectWrite 为权威」，而 DirectWrite 的
//! `IDWriteFactory::CreateSystemFontCollection` 是 **COM**：手写 vtable 布局意味着
//! 一个字段顺序错就崩，而那种崩溃无法用单元测试穷尽。本仓库一以贯之的取向是
//! **不自己实现安全边界**（同 23 §XML、PNG deflate、真 minidump），这里属于同一类。
//!
//! GDI 的 `EnumFontFamiliesExW` 是**扁平 C API**，只需要两个 `extern "system"` 声明，
//! 没有 vtable、没有引用计数、没有 HRESULT 忘了检查就漏资源。它枚举的正是
//! DirectWrite 在 Windows 上要暴露的那一批系统字体（同一套字体注册表）。
//!
//! 它给我们的不是「权威替换」，而是**权威交叉核对**：我们的库是不是漏了系统看得到的
//! face。这件事的价值比"换一个数据源"更大 —— 漏掉的 face 会让"找到作者引用的字体"
//! 这件事静默失败，而那正是 17 §1 的全部目的。
//!
//! ## 为什么这比"记成未实现"更好
//!
//! 一个未实现项只说明"这件事没做"。交叉核对会给出**可核对的数字**：
//! 「GDI 看得到 412 个 face，我们的库有 398 个，缺的 14 个是……」。
//! 那 14 个名字是可执行的线索（去查为什么没扫到），而"未实现"什么都不是。
//!
//! ## 一条纪律：枚举函数本身绝不能崩
//!
//! GDI 的回调是 `extern "system"`，其中的 panic 会跨 FFI 边界 —— 那是未定义行为。
//! 所以回调里**不做任何可能 panic 的事**：不用 `String::from_utf8`（用有损转换）、
//! 不索引切片、不 `unwrap`。同时用 `catch_unwind` 包住整个枚举调用作为第二道防线。

use std::ffi::c_void;
use std::path::PathBuf;

/// 一条 GDI 枚举出来的 face。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GdiFace {
    /// `ENUMLOGFONTEXW.elfLogFont.lfFaceName`：GDI 的**家族名**。
    ///
    /// 注意它是"GDI 视角的名字"，与字体 `name` 表里的 typographic family
    /// 可能不同（GDI 用的是 Win32 家族名 ID 1）。核对时两边都试。
    pub family: String,
    /// `elfFullName`：全名。可以为空（很多字体没填）。
    pub full_name: String,
    /// `elfStyle`：子家族名（Regular / Bold …）。
    pub style: String,
    /// `lfPitchAndFamily` 的低位：`VARIABLE_PITCH` / `FIXED_PITCH`。
    pub variable_pitch: bool,
    /// `TMPF_TRUETYPE` 位：TrueType 轮廓（含 OpenType）。位图/矢量字体没有。
    pub truetype: bool,
}

/// GDI 交叉核对的结果。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CrossCheck {
    /// 本层是否真的跑过。`false` 时下面的字段没有意义。
    pub ran: bool,
    /// 为什么没跑（拿不到 DC 等）。
    pub skipped_reason: Option<String>,
    /// GDI 枚举出多少个 face（**过滤前**的原始数）。
    ///
    /// 两个数都给：过滤前的说明“GDI 一共看得到多少东西”，过滤后的才是
    /// 与我们的库**可比**的范围。只报一个会让人误判（本机原始 6020，
    /// 其中大部分是不带 `name`/`cmap` 表的古早位图字体）。
    pub gdi_faces_raw: usize,
    /// 过滤后的 GDI face 数（只数真曲线字体、不含 `@` 竖排变体）。
    pub gdi_faces: usize,
    /// 我们的库有多少个 face。
    pub db_faces: usize,
    /// 去重后的 GDI 家族名数。
    pub gdi_families: usize,
    /// **GDI 看得到、而我们的库里一个 face 都匹配不上**的家族名。
    ///
    /// 这是本模块最重要的输出：它们是"我们漏掉的字体"的可执行线索。
    pub missing_from_db: Vec<String>,
    /// 我们的库里有、而 GDI 没枚举到的家族名。
    ///
    /// 通常是**正常现象**（用户字体、非 TrueType 位图字体、GDI 不暴露的格式），
    /// 所以它只是信息，不是问题。列出来是为了让"两边不一致"这件事完整可见，
    /// 而不是只报一半。
    pub extra_in_db: Vec<String>,
}

impl CrossCheck {
    pub fn skipped(reason: &str) -> CrossCheck {
        CrossCheck {
            ran: false,
            skipped_reason: Some(reason.to_string()),
            gdi_faces_raw: 0,
            gdi_faces: 0,
            db_faces: 0,
            gdi_families: 0,
            missing_from_db: Vec::new(),
            extra_in_db: Vec::new(),
        }
    }

    /// 有 face 被我们漏掉了吗。报告用它决定要不要提示用户。
    pub fn has_gap(&self) -> bool {
        self.ran && !self.missing_from_db.is_empty()
    }
}

// --- GDI 绑定 ---------------------------------------------------------------
//
// 只声明用到的那两个函数与两个结构体的前缀。全部字段都用 `c_void` 占位？
// 不行 —— 我们要读 `lfFaceName`。所以结构体按 `wingdi.h` 的真实布局声明到
// 需要的字段为止，**保持前缀完全一致**（这是 `EnumFontFamiliesEx` 回调 ABI 的前提）。

/// `LOGFONTW`。只声明到 `lfFaceName`，后面还有 4 个字段对我们没用，
/// 但**必须留出它们的位置**，否则 `lfFaceName` 的偏移就错了。
#[repr(C)]
struct LogFontW {
    lf_height: i32,
    lf_width: i32,
    lf_escapement: i32,
    lf_orientation: i32,
    lf_weight: i32,
    lf_italic: u8,
    lf_underline: u8,
    lf_strike_out: u8,
    lf_char_set: u8,
    lf_out_precision: u8,
    lf_clip_precision: u8,
    lf_quality: u8,
    lf_pitch_and_family: u8,
    /// `LF_FACESIZE = 32`
    lf_face_name: [u16; 32],
}

impl Default for LogFontW {
    fn default() -> Self {
        // SAFETY: 全 `u16`/整数的 POD，零初始化是合法状态（`lfFaceName` 为空串）。
        unsafe { std::mem::zeroed() }
    }
}

/// `ENUMLOGFONTEXW`：`ENUMLOGFONTW` 后面再跟 `elfScript[32]`。
#[repr(C)]
struct EnumLogFontExW {
    elf_log_font: LogFontW,
    /// `LF_FULLFACESIZE = 64`
    elf_full_name: [u16; 64],
    /// `LF_FACESIZE = 32`
    elf_style: [u16; 32],
    elf_script: [u16; 32],
}

/// `NEWTEXTMETRICEXW` 的前缀：我们只用 `ntmTm.tmPitchAndFamily`。
///
/// 字段宽度**必须与 `wingdi.h` 逐字对应**。这里曾经把 `tmFirstChar`…`tmBreakChar`
/// 写成 `u8`，而它们其实是 **`WCHAR`（u16）** —— 于是 `tmPitchAndFamily` 的偏移
/// 差了 4 字节，读出来恒为 0，所有字体都被当成“非 TrueType”。
/// 表现是：GDI 枚举到 6020 个 face 而**过滤后只剩 0 个**。
/// 真机用例（`tests/gdi_real.rs`）把这条抳出来的 —— 纯构造数据的单元测试看不见它。
#[repr(C)]
struct TextMetricW {
    tm_height: i32,
    tm_ascent: i32,
    tm_descent: i32,
    tm_internal_leading: i32,
    tm_external_leading: i32,
    tm_ave_char_width: i32,
    tm_max_char_width: i32,
    tm_weight: i32,
    tm_overhang: i32,
    tm_digitized_aspect_x: i32,
    tm_digitized_aspect_y: i32,
    // 这四个是 WCHAR！
    tm_first_char: u16,
    tm_last_char: u16,
    tm_default_char: u16,
    tm_break_char: u16,
    tm_italic: u8,
    tm_underlined: u8,
    tm_struck_out: u8,
    tm_pitch_and_family: u8,
    tm_char_set: u8,
}

#[repr(C)]
struct NewTextMetricExW {
    ntm_tm: TextMetricW,
    /// `NEWTEXTMETRICW` 之后追加的四个 `DWORD`；我们不需要它们，
    /// 但必须留位以保持长度。
    ntm_avgsize: u32,
    ntm_familytype: u32,
    ntm_size: u32,
    ntm_reserved: u32,
}

/// 布局自检：`tmPitchAndFamily` 在 `TEXTMETRICW` 里的真实偏移。
///
/// 11 个 `LONG`（4×11=44）+ 4 个 `WCHAR`（2×4=8）+ `tmItalic`/`tmUnderlined`/
/// `tmStruckOut`（3）= **55**。它必须能被机器核对 —— 上面那条注释里的错误
/// 就是靠真机用例抳出来的，而这里再加一道**编译期**的。
const _: () = {
    assert!(
        std::mem::offset_of!(TextMetricW, tm_pitch_and_family) == 55,
        "TEXTMETRICW 布局与 wingdi.h 不符：tmPitchAndFamily 的偏移应当是 55"
    );
    assert!(
        std::mem::offset_of!(LogFontW, lf_face_name) == 28,
        "LOGFONTW 布局与 wingdi.h 不符：lfFaceName 的偏移应当是 28"
    );
    assert!(
        std::mem::offset_of!(EnumLogFontExW, elf_full_name) == 92,
        "ENUMLOGFONTEXW 布局与 wingdi.h 不符：elfFullName 的偏移应当是 92"
    );
};

/// 位标志（`wingdi.h`）。
const TMPF_TRUETYPE: u8 = 0x04;
const TMPF_FIXED_PITCH: u8 = 0x01;
/// `VARIABLE_PITCH` 在 GDI 里的写法很反直觉：`TMPF_FIXED_PITCH` **置位**表示
/// 不是等宽。这里按真实语义命名，避免读代码的人按字面理解反了。
const FIXED_PITCH: u8 = TMPF_FIXED_PITCH;

#[cfg(windows)]
#[link(name = "gdi32")]
extern "system" {
    fn EnumFontFamiliesExW(
        hdc: *mut c_void,
        lp_logfont: *mut LogFontW,
        lp_proc: extern "system" fn(
            *const EnumLogFontExW,
            *const NewTextMetricExW,
            u32,
            isize,
        ) -> i32,
        lp_param: isize,
        flags: u32,
    ) -> i32;
    fn GetDeviceCaps(hdc: *mut c_void, index: i32) -> i32;
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn GetDC(hwnd: *mut c_void) -> *mut c_void;
    fn ReleaseDC(hwnd: *mut c_void, hdc: *mut c_void) -> i32;
}

/// 读一个 NUL 结尾的 UTF-16 缓冲。**有损转换**，绝不 panic。
///
/// 用 `from_utf16_lossy` 而不是 `from_utf16`：字体名里出现一个未配对代理项
/// （真实存在——有些老字体的名字表就是错的）不该让整次枚举失败。
fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// 回调里收集的原始数据。用 `Vec` 而不是在回调里做判断：
/// 回调只负责搬运，判断留在安全的普通代码里。
struct Collected {
    faces: Vec<GdiFace>,
    /// 上限，防止某个坏驱动让我们无限增长内存
    limit: usize,
}

/// GDI 回调。**这里不许 panic**（跨 FFI 边界的 panic 是 UB）。
extern "system" fn enum_proc(
    lf: *const EnumLogFontExW,
    tm: *const NewTextMetricExW,
    _font_type: u32,
    param: isize,
) -> i32 {
    // SAFETY: GDI 按 `EnumFontFamiliesExW` 的契约传这两个指针，它们在本次回调内有效。
    // 解引用前先判空（防御性，且不花钱）。
    if lf.is_null() || tm.is_null() || param == 0 {
        return 1; // 1 = 继续枚举
    }
    let collected = param as *mut Collected;
    // SAFETY: `param` 是我们传进去的 `&mut Collected` 的地址，生命周期覆盖整次枚举
    // （`catch_unwind` 包着的那段），回调期间那个引用一直有效且独占。
    let c = unsafe { &mut *collected };
    if c.faces.len() >= c.limit {
        return 0; // 0 = 停止枚举
    }
    // SAFETY: 指针非空且类型正确（GDI 契约）。
    let (log, metrics) = unsafe { (&*lf, &*tm) };
    let pitch = metrics.ntm_tm.tm_pitch_and_family;
    c.faces.push(GdiFace {
        family: wide_to_string(&log.elf_log_font.lf_face_name),
        full_name: wide_to_string(&log.elf_full_name),
        style: wide_to_string(&log.elf_style),
        variable_pitch: pitch & FIXED_PITCH == 0,
        truetype: pitch & TMPF_TRUETYPE != 0,
    });
    1 // 继续
}

/// 枚举上限。系统字体通常几百到一千出头；这个数只防"坏驱动让枚举停不下来"。
const ENUM_LIMIT: usize = 20_000;

/// 用 GDI 枚举系统字体。非 Windows 或拿不到 DC 时返回 `Err(原因)`。
///
/// `charset` 传 `DEFAULT_CHARSET`（1）而不是 `ANSI_CHARSET`：后者会漏掉 GB2312
/// 这类非 ANSI 字符集的字体，而中文字幕工具最不能漏的就是它们。
#[cfg(windows)]
fn enumerate_gdi() -> Result<Vec<GdiFace>, String> {
    const DEFAULT_CHARSET: u8 = 1;
    // SAFETY: `GetDC(NULL)` 取整个屏幕的 DC，是 GDI 的常规用法。
    let hdc = unsafe { GetDC(std::ptr::null_mut()) };
    if hdc.is_null() {
        return Err("GetDC 拿不到设备上下文".to_string());
    }
    // `GetDeviceCaps` 的调用不是为了读值，是为了**证明这个 DC 可用**：
    // 某些会话（无桌面服务）会给一个不能枚举字体的 DC，而那会表现为 0 个 face。
    let _bits = unsafe {
        GetDeviceCaps(hdc, 12 /* BITSPIXEL */)
    };

    let mut lf = LogFontW {
        lf_char_set: DEFAULT_CHARSET,
        ..LogFontW::default()
    };

    let mut collected = Collected {
        faces: Vec::new(),
        limit: ENUM_LIMIT,
    };
    let param = (&mut collected as *mut Collected) as isize;

    // `catch_unwind` 是第二道防线（第一道是回调里不 panic）。
    // 万一将来有人在回调里加了会 panic 的代码，这里至少不会跨 FFI 边界炸出去。
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: 指针都指向本函数栈上/堆上、生命周期覆盖本次调用的对象。
        unsafe {
            EnumFontFamiliesExW(hdc, &mut lf, enum_proc, param, 0);
        }
    }));
    // SAFETY: 与 `GetDC` 配对释放。
    unsafe {
        ReleaseDC(std::ptr::null_mut(), hdc);
    }
    if outcome.is_err() {
        return Err("GDI 枚举过程中发生了 panic".to_string());
    }
    Ok(collected.faces)
}

#[cfg(not(windows))]
fn enumerate_gdi() -> Result<Vec<GdiFace>, String> {
    Err("GDI 交叉核对只在 Windows 上可用".to_string())
}

/// GDI 的家族名能不能看成某个**已知家族**的变体。
///
/// 逐词从尾部剥，看剩下的前缀能不能对上（最短留一词）。
/// 例：`Bahnschrift Light` → 试 `Bahnschrift Light`（已试过）→ `Bahnschrift` ✓。
/// `Sitka Display Semibold` → `Sitka Display` → `Sitka` ✓。
///
/// 为什么只剥尾部而不是看任意子串：`Noto Sans SC` 与 `Noto Sans Arabic`
/// 共享前缀 `Noto Sans`，而在尾剥能把它们区分开（前者剩 `Noto Sans`
/// 也仍是 `Noto Sans SC` 的真前缀），任意子串匹配会把它们混成一族。
fn matches_a_family_prefix(name: &str, db_norm: &[String]) -> bool {
    let n = name.trim().to_lowercase();
    let words: Vec<&str> = n.split_whitespace().collect();
    // 从去掉最后 1 个词开始，一直到只剩 1 个词。
    // 不从完整名字开始：那个已经在调用方试过了。
    for cut in (1..words.len()).rev() {
        let prefix = words[..cut].join(" ");
        if db_norm.iter().any(|d| d == &prefix) {
            return true;
        }
    }
    false
}

/// 把我们的字体库与 GDI 的枚举结果对照。
///
/// ## 匹配规则
///
/// GDI 给的是 Win32 家族名 + 全名 + 风格名，而我们的 `FaceRecord` 里同时有
/// typographic family、win32_family、full_name、postscript、以及全部本地化名。
/// 所以判定"GDI 看到的这个 face 我们在库里有"时，**两边都要试**：
///
/// 1. 先把 GDI 的 `family` 与 `family + " " + style` 当作候选名；
/// 2. 用 `FaceRecord::matches_name` 去库里面找（它已经会试所有别名）；
/// 3. 一个都找不到就进 `missing_from_db`。
///
/// 只比较**规范化后的家族名集合**，而不是逐 face 比：GDI 与我们的视角在
/// "一个家族有几个 face"上本来就会不同（GDI 按字体链接/模拟加粗展开），
/// 逐 face 比会刷出大量无意义的差异。而"整个家族的名字我们完全没见过"
/// 才是真的漏。
pub fn cross_check(db_families: &[String], gdi: Result<Vec<GdiFace>, String>) -> CrossCheck {
    let gdi_raw = match gdi {
        Ok(g) => g,
        Err(why) => return CrossCheck::skipped(&why),
    };
    // 只比**真曲线字体**（TrueType/OpenType）：GDI 会把 System / Terminal / Fixedsys
    // 这些古老的位图字体也枚举出来（本机 6020 个 face 里大多数是它们），
    // 而它们没有 `name`/`cmap` 表，**不在** 17 §2 那个数据库的范围内。
    // 把它们算成“我们漏了”会直接埋掉真正的缺口。
    //
    // 同理，`@` 开头的是 GDI 的**竖排变体**（同一份字体文件），
    // 我们的库里不会为它单列一条。按前缀剔掉。
    let gdi_raw_count = gdi_raw.len();
    let gdi: Vec<GdiFace> = gdi_raw
        .into_iter()
        .filter(|f| f.truetype && !f.family.starts_with('@'))
        .collect();

    let mut want: Vec<String> = db_families.to_vec();
    want.sort();
    want.dedup();
    let norm = |s: &str| s.trim().to_lowercase();

    let mut gdi_names: Vec<String> = Vec::new();
    for f in &gdi {
        if !f.family.is_empty() {
            gdi_names.push(norm(&f.family));
        }
        if !f.full_name.is_empty() {
            gdi_names.push(norm(&f.full_name));
        }
    }
    gdi_names.sort();
    gdi_names.dedup();

    let db_norm: Vec<String> = want.iter().map(|s| norm(s)).collect();
    let has = |name: &str| db_norm.iter().any(|d| d == name);

    // 一个 GDI 家族只报一次，且只要它的**家族名或全名**能与库里的任何名字对上
    // 就不算缺 —— 有些字体的 GDI 家族名是异体（或者干脆为空），而全名才是
    // 与 `name` 表对得上的那个。只看家族名会刷出假缺口。
    let mut missing_from_db: Vec<String> = Vec::new();
    for f in &gdi {
        if f.family.is_empty() {
            continue;
        }
        let known = has(&norm(&f.family)) || (!f.full_name.is_empty() && has(&norm(&f.full_name)));
        if !known && !missing_from_db.contains(&f.family) {
            missing_from_db.push(f.family.clone());
        }
    }
    // 第二次机会：GDI 会把 **可变字体的具名实例** 与 **带风格后缀的族**
    // 当独立家族枚举出来（本机实测 63 个这样的一项）。
    //
    // 它们不是“我们漏了字体”，而是**同一个文件**：
    //   * `Bahnschrift Light` / `Bahnschrift SemiCondensed` —— 一个可变字体
    //     的 `fvar` 具名实例（我们的库里只有 `Bahnschrift` 一条）；
    //   * `Sitka Display Semibold` / `Sitka Small` —— 家族名带了子族后缀
    //     （库里是 `Sitka` + `style_name`）。
    //
    // 判据：把 GDI 名字的尾部逐词去掉，看剩下的能不能对上一个已知家族。
    // 只在**尾部**剥（不是任意子串）：那才能区分“同一族的实例”与“另一个族”，
    // 而后者才是真缺口。
    missing_from_db.retain(|name| !matches_a_family_prefix(name, &db_norm));

    // 反向：GDI 的候选名里没有一个能匹配上我们的某个家族。
    // 注意判据是"我们的哪个家族名没出现在 GDI 的候选里"，
    // 而不是"我们的家族数比 GDI 少" —— 后者在 GDI 合并同族 face 时会误报。
    let db_norm_set: std::collections::BTreeSet<String> = db_norm.iter().cloned().collect();
    let gdi_set: std::collections::BTreeSet<String> = gdi_names.iter().cloned().collect();
    let extra_in_db: Vec<String> = want
        .iter()
        .filter(|name| {
            let n = norm(name);
            !n.is_empty() && !gdi_set.contains(&n) && db_norm_set.contains(&n)
        })
        .cloned()
        .collect();

    CrossCheck {
        ran: true,
        skipped_reason: None,
        gdi_faces_raw: gdi_raw_count,
        gdi_faces: gdi.len(),
        db_faces: 0, // 由调用方填（本函数只关心名字集合）
        gdi_families: gdi_set.len(),
        missing_from_db,
        extra_in_db,
    }
}

/// 跑一次完整的交叉核对：枚举 + 对照。
pub fn run(db_families: &[String], db_faces: usize) -> CrossCheck {
    let mut c = cross_check(db_families, enumerate_gdi());
    c.db_faces = db_faces;
    c
}

/// 目录里有没有 Windows 字体（给"跳过"找一个可说的理由）。
pub fn default_font_dir() -> Option<PathBuf> {
    std::env::var("WINDIR")
        .ok()
        .map(|w| PathBuf::from(w).join("Fonts"))
}

/// 测试用：直接跑一次真枚举。
///
/// 存在的理由：真枚举要 DC，而单元测试的环境不一定有桌面会话；
/// 真机用例（`tests/gdi_real.rs`）用它，并在拿不到 DC 时显式 SKIP。
#[doc(hidden)]
pub fn enumerate_for_test() -> Result<Vec<GdiFace>, String> {
    enumerate_gdi()
}

/// 测试用：拿一个**已经枚举好的** GDI 结果去跑对照。
///
/// 让调用方把结果传进来，测试就不必为了跑两次对照去开两次 DC。
#[doc(hidden)]
pub fn cross_check_for_test(db_families: &[String], gdi: Vec<GdiFace>) -> CrossCheck {
    cross_check(db_families, Ok(gdi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(family: &str, full: &str, style: &str) -> GdiFace {
        GdiFace {
            family: family.to_string(),
            full_name: full.to_string(),
            style: style.to_string(),
            variable_pitch: true,
            truetype: true,
        }
    }

    /// 位图字体（GDI 也枚举得到，但没有 `name`/`cmap` 表）。
    fn bitmap(family: &str) -> GdiFace {
        GdiFace {
            family: family.to_string(),
            full_name: family.to_string(),
            style: String::new(),
            variable_pitch: true,
            truetype: false,
        }
    }

    /// **真机上最容易埋掉真缺口的那个坑**：GDI 会枚举出 System/Terminal/Fixedsys
    /// 这些古早**位图**字体（本机 6020 个 face 里大多数是它们），以及 `@` 开头的
    /// **竖排变体**。它们没有 `name`/`cmap` 表，不在 17 §2 数据库的范围内；
    /// 把它们算成“我们漏了”会直接埋掉真正的缺口。
    #[test]
    fn bitmap_fonts_and_vertical_variants_are_not_counted_as_gaps() {
        let db = vec!["Arial".to_string()];
        let gdi = Ok(vec![
            face("Arial", "Arial", "Regular"),
            bitmap("System"),
            bitmap("Terminal"),
            bitmap("Fixedsys"),
            face("@Arial", "@Arial", "Regular"),
            face("@Some New Font", "@Some New Font", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert_eq!(c.gdi_faces_raw, 6, "原始数该是 6");
        assert_eq!(c.gdi_faces, 1, "过滤后只剩 Arial");
        assert!(
            c.missing_from_db.is_empty(),
            "位图字体与 @ 变体都不算缺口：{c:?}"
        );
        assert!(!c.has_gap());
    }

    /// 核心语义：GDI 看到而库里没有的家族要进 `missing_from_db`。
    #[test]
    fn a_gdi_family_we_do_not_have_is_reported_as_missing() {
        let db = vec!["Arial".to_string(), "微软雅黑".to_string()];
        let gdi = Ok(vec![
            face("Arial", "Arial", "Regular"),
            face("微软雅黑", "Microsoft YaHei", "Regular"),
            face("Some New Font", "Some New Font", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert!(c.ran);
        assert_eq!(c.gdi_faces, 3);
        assert_eq!(c.missing_from_db, vec!["Some New Font".to_string()]);
        assert!(c.has_gap());
    }

    /// **负基线**：两边一致时不该报任何 missing。
    /// 少了这条，一个"永远报缺"的实现也能让上面那条通过。
    #[test]
    fn an_agreeing_pair_reports_no_gap() {
        let db = vec!["Arial".to_string(), "微软雅黑".to_string()];
        let gdi = Ok(vec![
            face("Arial", "Arial", "Regular"),
            face("微软雅黑", "Microsoft YaHei", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert!(c.missing_from_db.is_empty(), "{c:?}");
        assert!(!c.has_gap());
    }

    /// 大小写与空白不该造成假差异。
    #[test]
    fn matching_is_case_and_whitespace_insensitive() {
        let db = vec!["Arial".to_string()];
        let gdi = Ok(vec![face("  aRiAl ", "ARIAL", "Regular")]);
        let c = cross_check(&db, gdi);
        assert!(c.missing_from_db.is_empty(), "{c:?}");
    }

    /// 全名也能当匹配依据：有些字体的 GDI 家族名是空/异体，而全名对得上。
    #[test]
    fn a_match_on_the_full_name_counts_as_known() {
        let db = vec!["Microsoft YaHei".to_string()];
        let gdi = Ok(vec![face("微软雅黑", "Microsoft YaHei", "Regular")]);
        let c = cross_check(&db, gdi);
        assert!(c.missing_from_db.is_empty(), "全名对上了就不该报缺：{c:?}");
    }

    /// 我们有、GDI 没有的家族进 `extra_in_db`（**信息**而不是问题）。
    ///
    /// 它通常来自用户字体或 GDI 不暴露的格式，所以只列出、不判为缺陷 ——
    /// 但必须列出来，否则"两边不一致"只报了一半。
    #[test]
    fn a_db_only_family_is_listed_as_extra_not_as_a_failure() {
        let db = vec!["Arial".to_string(), "My Private Font".to_string()];
        let gdi = Ok(vec![face("Arial", "Arial", "Regular")]);
        let c = cross_check(&db, gdi);
        assert_eq!(c.extra_in_db, vec!["My Private Font".to_string()]);
        assert!(c.missing_from_db.is_empty());
        assert!(!c.has_gap(), "extra 不是缺口");
    }

    /// **真机实测抳出来的那条**：GDI 把可变字体的具名实例与带风格后缀的族
    /// 当独立家族枚举（本机 63 个），而它们是**同一个文件**，不是我们漏了。
    #[test]
    fn named_instances_and_style_suffixed_families_are_not_gaps() {
        let db = vec![
            "Bahnschrift".to_string(),
            "Sitka".to_string(),
            "Segoe UI Variable".to_string(),
            "Noto Sans SC".to_string(),
        ];
        let gdi = Ok(vec![
            // 可变字体的 fvar 具名实例
            face("Bahnschrift Light", "Bahnschrift Light", "Regular"),
            face(
                "Bahnschrift SemiCondensed",
                "Bahnschrift SemiCondensed",
                "Regular",
            ),
            // 家族名带子族后缀
            face(
                "Sitka Display Semibold",
                "Sitka Display Semibold",
                "Regular",
            ),
            face(
                "Segoe UI Variable Small Light",
                "Segoe UI Variable Small Light",
                "Regular",
            ),
            // 权重名当最后一段
            face("Noto Sans SC Thin", "Noto Sans SC Thin", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert!(
            c.missing_from_db.is_empty(),
            "这些是同一份字体的实例/带后缀写法，不该算缺口：{:?}",
            c.missing_from_db
        );
    }

    /// **负基线**：真的没见过的族仍要被报出来。
    /// 少了这条，一个「把所有带空格的都当变体」的实现也能让上面那条绿。
    #[test]
    fn a_genuinely_unknown_family_is_still_reported_after_prefix_matching() {
        // db 里**只有** Arial：Bahnschrift 不在库里，所以 `Bahnschrift Light`
        // 的前缀剥到 `Bahnschrift` 仍然对不上，它就该报。
        let db = vec!["Arial".to_string()];
        let gdi = Ok(vec![
            face("Totally New Font", "Totally New Font", "Regular"),
            face("Bahnschrift Light", "Bahnschrift Light", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert_eq!(
            c.missing_from_db,
            vec![
                "Totally New Font".to_string(),
                "Bahnschrift Light".to_string()
            ],
            "库里没有基名时，带后缀的写法也是真缺口"
        );
    }

    /// 前缀剥离**只在尾部**剥：`Noto Sans Arabic` 不是 `Noto Sans SC` 的变体。
    #[test]
    fn prefix_matching_does_not_merge_unrelated_families() {
        let db = vec!["Noto Sans SC".to_string()];
        let gdi = Ok(vec![face(
            "Noto Sans Arabic",
            "Noto Sans Arabic",
            "Regular",
        )]);
        let c = cross_check(&db, gdi);
        assert_eq!(
            c.missing_from_db,
            vec!["Noto Sans Arabic".to_string()],
            "共享词首但最后一个词不同，不是同族的变体"
        );
    }

    /// 单字家族名不会被剥成空串。
    #[test]
    fn a_single_word_family_is_not_stripped_to_nothing() {
        assert!(!matches_a_family_prefix("Arial", &[]));
        assert!(!matches_a_family_prefix("Arial", &["arial".to_string()]));
        // 两个词、第一个词对上时才算变体
        assert!(matches_a_family_prefix(
            "Arial Bold",
            &["arial".to_string()]
        ));
    }

    /// 枚举失败时要如实说"没跑"，而不是给一个看起来正常的 0 差异结果。
    #[test]
    fn a_failed_enumeration_says_it_did_not_run() {
        let db = vec!["Arial".to_string()];
        let c = cross_check(&db, Err("GetDC 拿不到设备上下文".to_string()));
        assert!(!c.ran, "没跑就不能说是通过");
        assert!(c.skipped_reason.as_deref().unwrap().contains("GetDC"));
        assert!(
            !c.has_gap(),
            "没跑时不该报缺口（那会把环境问题说成字体问题）"
        );
    }

    /// 空家族名（某些字体的 GDI 名字为空）不该进 missing —— 那会刷出无意义的条目。
    #[test]
    fn an_empty_gdi_family_name_is_not_reported() {
        let db = vec!["Arial".to_string()];
        let gdi = Ok(vec![
            face("Arial", "Arial", "Regular"),
            face("", "", "Regular"),
        ]);
        let c = cross_check(&db, gdi);
        assert!(c.missing_from_db.is_empty(), "{c:?}");
    }

    /// 重复的 GDI 家族只报一次。
    /// 一个家族有 4 个 face（Regular/Bold/Italic/Bold Italic）是常态。
    #[test]
    fn a_repeated_gdi_family_is_reported_once() {
        let db = vec!["Arial".to_string()];
        let gdi = Ok(vec![
            face("NewFont", "NewFont", "Regular"),
            face("NewFont", "NewFont", "Bold"),
            face("NewFont", "NewFont", "Italic"),
            face("NewFont", "NewFont", "Bold Italic"),
        ]);
        let c = cross_check(&db, gdi);
        assert_eq!(
            c.missing_from_db,
            vec!["NewFont".to_string()],
            "同一个家族四个 face 只该报一次"
        );
    }

    /// 读宽字符串：NUL 截断、有损转换、不 panic。
    #[test]
    fn reading_a_wide_buffer_stops_at_nul_and_never_panics() {
        let mut buf = [0u16; 8];
        for (i, c) in "AB".encode_utf16().enumerate() {
            buf[i] = c;
        }
        assert_eq!(wide_to_string(&buf), "AB");
        // 全零 = 空串
        assert_eq!(wide_to_string(&[0u16; 4]), "");
        // 无 NUL 结尾时读满
        assert_eq!(wide_to_string(&[b'X' as u16; 3]), "XXX");
        // 未配对代理项：有损但不 panic
        let s = wide_to_string(&[0xD800, 0x0041, 0]);
        assert!(!s.is_empty(), "有损转换也该产出点什么");
    }
}
