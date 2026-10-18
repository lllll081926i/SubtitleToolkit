//! PE 头里的 Machine 字段（12 §3 / 23 §DLL）：**不看文件名，也不看目录名**。
//!
//! 12 §3 的禁令是"不得跨位数 `LoadLibrary`"。要在调用 `LoadLibrary` 之前就知道位数，
//! 唯一的依据就是文件本身，所以这里只读 PE 头，不加载、不执行。
//!
//! 本模块还负责 **导出表**（12 §1 的能力探测需要"这个 DLL 导出了哪些符号"）。
//! 两者放在一起的理由与位数一样：**只有一份实现**，主程序与 host 才不会对同一个
//! DLL 给出不同答案。
//!
//! ## 为什么必须自己读导出表
//!
//! 12 §1 的三条能力链（libass / CSRI / legacy）要靠**导出符号**区分，
//! 而不是靠文件名。而 `LoadLibrary` + `GetProcAddress` 是**加载**，
//! 恰好就是 12 §3 禁止跨位数做的事 —— 先加载再问“你是谁”，
//! 在位数不符时已经晚了。所以只能静态读导出表。
//!
//! ## 为什么在 core-model 而不是 renderer-host
//!
//! 主程序（在加载 DLL 之前先自检）与 host（真的加载之前再查一次）都要这个判断。
//! 两份实现对**同一个 DLL 会给出不同答案** —— 那意味着一边放行一边拒绝，
//! 而用户看到的是一条自相矛盾的报错。所以只有一份，放在两边都依赖的那一层。

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// 我们关心的机器类型；未知值保留原始数字，让诊断说得出话。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeArch {
    X86,
    X64,
    Arm,
    Arm64,
    Other(u16),
}

impl PeArch {
    pub fn label(self) -> &'static str {
        match self {
            PeArch::X86 => "x86",
            PeArch::X64 => "x64",
            PeArch::Arm => "arm",
            PeArch::Arm64 => "arm64",
            PeArch::Other(_) => "other",
        }
    }

    /// 主程序与 host 都是 x64（12 §3：主程序 x64）。
    pub fn is_x64(self) -> bool {
        matches!(self, PeArch::X64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeError {
    /// 读文件失败（不存在、被占用……带上 os error 更有用）
    Unreadable(String),
    /// 不是 PE（没有 MZ、没有 PE\0\0、头太小）
    NotPe(String),
}

impl PeError {
    /// 稳定的错误码片段，供上层包成 `StkError`。
    ///
    /// 分开"读不了"与"不是 PE"两件事：前者是环境问题（文件不在、被占用），
    /// 后者是拿错了东西（改名、截断、根本不是 DLL）。用户要做的事完全不同。
    pub fn kind_label(&self) -> &'static str {
        match self {
            PeError::Unreadable(_) => "unreadable",
            PeError::NotPe(_) => "not-pe",
        }
    }
}

impl std::fmt::Display for PeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PeError::Unreadable(s) => write!(f, "无法读取 PE 文件：{s}"),
            PeError::NotPe(s) => write!(f, "不是有效的 PE 文件：{s}"),
        }
    }
}

const MZ: [u8; 2] = [0x4d, 0x5a];
const PE_LEAN: [u8; 4] = [0x50, 0x45, 0x00, 0x00];

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// PE 头里的 Machine 值 -> 我们的架构分类。**两个读取路径共用这一份表**，
/// 否则"头在前 4 KiB"与"头在后面"两条分支会对同一个 DLL 给出不同答案。
///
/// 0x8664 是 AMD64（x64）；0x014c 是 i386（x86）；0x0200 是 Itanium，
/// 我们不当成 x64——把它归进 Other，让 12 §3 的"其他架构拒绝并给诊断"生效。
fn from_machine(machine: u16) -> PeArch {
    match machine {
        0x014c => PeArch::X86,
        0x8664 => PeArch::X64,
        0x01c0 => PeArch::Arm,
        0x01c4 => PeArch::Arm,
        0xaa64 => PeArch::Arm64,
        other => PeArch::Other(other),
    }
}

/// 从字节里读机器类型（`None` 表示字节不足以判断——用于流式场景）。
pub fn arch_of_bytes(bytes: &[u8]) -> Result<PeArch, PeError> {
    if bytes.len() < 0x40 {
        return Err(PeError::NotPe(
            "少于 64 字节，连 DOS 头都不完整".to_string(),
        ));
    }
    if bytes[0..2] != MZ {
        return Err(PeError::NotPe("缺少 MZ 签名".to_string()));
    }
    let lfanew = le32(bytes, 0x3c) as usize;
    if lfanew + 6 > bytes.len() {
        return Err(PeError::NotPe(format!(
            "e_lfanew={lfanew} 落在文件之外（文件只有 {} 字节）",
            bytes.len()
        )));
    }
    if bytes[lfanew..lfanew + 4] != PE_LEAN {
        return Err(PeError::NotPe(format!(
            "e_lfanew={lfanew} 处没有 PE\\0\\0 签名"
        )));
    }
    Ok(from_machine(le16(bytes, lfanew + 4)))
}

/// 只读头部：先取 4 KiB（绝大多数 PE 的头就在这里），不够再 seek 到 `e_lfanew`。
/// 这个函数**不加载 DLL**，所以它是"跨位数禁令"唯一能依靠的前置检查。
pub fn arch_of_file(path: &Path) -> Result<PeArch, PeError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
    let mut head = vec![0u8; 4096];
    let mut got = 0usize;
    while got < head.len() {
        let n = file
            .read(&mut head[got..])
            .map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
        if n == 0 {
            break;
        }
        got += n;
    }
    head.truncate(got);
    if head.len() >= 0x40 && head[0..2] == MZ {
        let lfanew = le32(&head, 0x3c) as usize;
        if lfanew + 6 > head.len() && lfanew + 6 <= 1 << 20 {
            // 少数链接器把头放得很靠后：只补读需要的那 6 字节
            let mut tail = [0u8; 6];
            file.seek(SeekFrom::Start(lfanew as u64))
                .map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
            let mut read = 0usize;
            while read < tail.len() {
                let n = file
                    .read(&mut tail[read..])
                    .map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
                if n == 0 {
                    break;
                }
                read += n;
            }
            if read == tail.len() && tail[0..4] == PE_LEAN {
                let machine = u16::from_le_bytes([tail[4], tail[5]]);
                return Ok(from_machine(machine));
            }
        }
    }
    arch_of_bytes(&head)
}

/// 一个 PE 文件的**导出表**（12 §1 的能力探测输入）。
///
/// # 为什么返回结构体而不只返一个名字列表
///
/// 因为 `probe` 需要区分“导出表存在但为空”与“根本没有导出表”——
/// 前者是“这个 DLL 不导出任何东西”（比如资源 DLL），
/// 后者是“这不是一个可探测的 API provider”。
/// 把两者都压成一个空 `Vec` 会让诊断说不出话。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exports {
    /// 按导出表顺序（含序号-only 的条目用空串占位）。
    pub names: Vec<String>,
    /// 导出目录里声明的名字数（**可能大于** `names.len()`，见 [`Exports::note`]）。
    pub declared: u32,
    /// 读不全时的原因（如 RVA 越界、名字在文件外）。
    ///
    /// 有值时调用方应当**当它不完整**，而不是当成“没有那些符号”。
    /// 这一点很重要：把一个读不到的导出表当成“没有 CSRI 符号”，
    /// 会导致对一个其实可用的 provider 报“不是 provider”。
    pub note: Option<String>,
}

impl Exports {
    pub fn is_empty(&self) -> bool {
        self.names.iter().all(|n| n.is_empty())
    }

    /// 是否导出了该名字（**区分大小写** —— PE 导出名是区分大小写的）。
    pub fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }
}

/// 节表里的一节：我们只需要它在**文件里**的位置与它在内存里的 RVA。
#[derive(Clone, Copy, Debug)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_ptr: u32,
    raw_size: u32,
}

/// 把 RVA 落到文件偏移。
///
/// 用节表而不是 RVA 本身：PE 的文件布局与内存布局不同，
/// `RVA + 节偏移` 是错的（差值往往不等于 `SizeOfHeaders`）。
/// 这里逐节查区间 —— 慢一点（节数很少），但不会因为一个巧合的布局而读错地方。
fn rva_to_offset(sections: &[Section], rva: u32) -> Option<usize> {
    for s in sections {
        // 节在内存里的跨度用 `max(virtual_size, raw_size)`：
        // 有些链接器把 `virtual_size` 写得比 `raw_size` 小（不该写 0），
        // 只信其中一个会把本来在文件里的字节算成“不在”。
        let span = s.virtual_size.max(s.raw_size);
        if rva >= s.virtual_address && rva < s.virtual_address.saturating_add(span) {
            let delta = rva - s.virtual_address;
            if delta >= s.raw_size {
                // 落在节的“内存尾巴”上（未初始化数据），文件里没有对应字节
                return None;
            }
            return Some((s.raw_ptr + delta) as usize);
        }
    }
    // 头区（很多小 DLL 把数据目录指向头里）
    if rva < 0x1000 {
        return Some(rva as usize);
    }
    None
}

fn cstr_at(bytes: &[u8], off: usize, limit: usize) -> Option<String> {
    if off >= bytes.len() {
        return None;
    }
    let end = bytes[off..]
        .iter()
        .take(limit)
        .position(|b| *b == 0)
        .map(|n| off + n)?;
    String::from_utf8(bytes[off..end].to_vec()).ok()
}

/// 读一个 PE 文件的导出表。**不加载 DLL，不执行任何代码。**
///
/// 输入是整个文件的字节：导出表可以落在文件任何位置，流式读不够。
/// 调用方（CLI/host）已因为位数检查需要读文件，复用那份字节。
pub fn exports_of_bytes(bytes: &[u8]) -> Result<Exports, PeError> {
    if bytes.len() < 0x40 {
        return Err(PeError::NotPe(
            "少于 64 字节，连 DOS 头都不完整".to_string(),
        ));
    }
    if bytes[0..2] != MZ {
        return Err(PeError::NotPe("缺少 MZ 签名".to_string()));
    }
    let lfanew = le32(bytes, 0x3c) as usize;
    if lfanew + 24 > bytes.len() {
        return Err(PeError::NotPe(format!(
            "e_lfanew={lfanew} 落在文件之外（文件只有 {} 字节）",
            bytes.len()
        )));
    }
    if bytes[lfanew..lfanew + 4] != PE_LEAN {
        return Err(PeError::NotPe(format!(
            "e_lfanew={lfanew} 处没有 PE\\0\\0 签名"
        )));
    }
    let nsec = le16(bytes, lfanew + 6) as usize;
    let opt_size = le16(bytes, lfanew + 20) as usize;
    let opt_at = lfanew + 24;
    if opt_at + opt_size > bytes.len() {
        return Err(PeError::NotPe("可选头超出文件末尾".to_string()));
    }
    // magic: 0x10b = PE32，0x20b = PE32+
    let magic = le16(bytes, opt_at);
    // 数据目录的位置：PE32 在可选头偏移 96，PE32+ 在 112。
    // 导出表是第 0 项。
    let dd_at = match magic {
        0x10b => opt_at + 96,
        0x20b => opt_at + 112,
        other => {
            return Err(PeError::NotPe(format!(
                "可选头 magic=0x{other:04x} 既不是 PE32 也不是 PE32+"
            )))
        }
    };
    if dd_at + 8 > bytes.len() {
        return Err(PeError::NotPe("数据目录超出文件末尾".to_string()));
    }
    let export_rva = le32(bytes, dd_at);
    let export_size = le32(bytes, dd_at + 4);

    let mut sections: Vec<Section> = Vec::with_capacity(nsec);
    let sec_at = opt_at + opt_size;
    for i in 0..nsec {
        let at = sec_at + i * 40;
        if at + 40 > bytes.len() {
            return Err(PeError::NotPe(format!("节表第 {i} 项超出文件末尾")));
        }
        sections.push(Section {
            virtual_size: le32(bytes, at + 8),
            virtual_address: le32(bytes, at + 12),
            raw_size: le32(bytes, at + 16),
            raw_ptr: le32(bytes, at + 20),
        });
    }

    // 没有导出表不是错误：很多 DLL（资源库、纯数据库）本来就不导出任何东西。
    if export_rva == 0 || export_size == 0 {
        return Ok(Exports {
            names: Vec::new(),
            declared: 0,
            note: None,
        });
    }

    let dir_off = rva_to_offset(&sections, export_rva)
        .ok_or_else(|| PeError::NotPe(format!("导出目录 RVA=0x{export_rva:x} 不在任何节里")))?;
    // IMAGE_EXPORT_DIRECTORY：
    // 0  Characteristics, 4  TimeDateStamp, 8  Major,10 Minor,
    // 12 Name RVA, 16 Base, 20 NumberOfFunctions, 24 NumberOfNames,
    // 28 AddressOfFunctions RVA, 32 AddressOfNames RVA, 36 AddressOfNameOrdinals RVA
    let need = dir_off + 40;
    if need > bytes.len() {
        return Err(PeError::NotPe("导出目录头超出文件末尾".to_string()));
    }
    let declared = le32(bytes, dir_off + 24);
    let names_rva = le32(bytes, dir_off + 32);

    // 没有名字（只有序号导出）：不是错误，只是没有可用来判能力的符号。
    if declared == 0 || names_rva == 0 {
        return Ok(Exports {
            names: Vec::new(),
            declared,
            note: Some(format!(
                "导出目录声明 {declared} 个名字，但没有 AddressOfNames 表"
            )),
        });
    }

    let Some(names_off) = rva_to_offset(&sections, names_rva) else {
        return Ok(Exports {
            names: Vec::new(),
            declared,
            note: Some(format!("AddressOfNames RVA=0x{names_rva:x} 不在任何节里")),
        });
    };

    // 上限保护：一个被构造/损坏的导出表可以声明几亿个名字，
    // 而在读到第一个失败之前就先把内存吃光。
    const MAX_NAMES: u32 = 200_000;
    const MAX_NAME_LEN: usize = 4096;
    let mut names: Vec<String> = Vec::new();
    let mut note: Option<String> = None;
    let to_read = declared.min(MAX_NAMES);
    if declared > MAX_NAMES {
        note = Some(format!(
            "导出表声明 {declared} 个名字，超过上限 {MAX_NAMES}；只读了前 {MAX_NAMES} 个"
        ));
    }
    for i in 0..to_read as usize {
        let ent = names_off + i * 4;
        if ent + 4 > bytes.len() {
            note = Some(format!("第 {i} 个名字 RVA 表项超出文件末尾"));
            break;
        }
        let name_rva = le32(bytes, ent);
        match rva_to_offset(&sections, name_rva).and_then(|o| cstr_at(bytes, o, MAX_NAME_LEN)) {
            Some(n) => names.push(n),
            None => {
                // 单个名字读不到就记一笔并继续：我们宁可少几个名字，
                // 而 note 会让调用方知道“这份列表不完整”。
                note.get_or_insert_with(|| format!("第 {i} 个导出名（RVA=0x{name_rva:x}）读不到"));
                names.push(String::new());
            }
        }
    }
    Ok(Exports {
        names,
        declared,
        note,
    })
}

/// 用一份**导出名集**判定“这是不是一个 libass provider”。
///
/// 为什不在 `render-libass` 里做：本轮把“先看导出表、再决定加不加载”设成了一条
/// 跨层的规矩（12 §3），而**判定依据本身**只该有一份。
/// 两份列表会对同一个 DLL 给出不同结论，而用户看到的是一条自相矛盾的报错。
///
/// 这里只列**核心渲染路径真的会调用**的符号。详情与理由见
/// `render-libass` 的 `LIBASS_REQUIRED_SYMBOLS` 注释（那边是权威列表）——
/// 这里保留了同名前缀的检查，是因为它要答的问题更窄：
/// “这个名字集看着像不像 libass”，而不是“能不能拿来渲染”。
///
/// 两者分工：**本函数用于给出“它是什么”的诊断**（在加载之前），
/// 而真正决定能否使用的是 `render-libass::ffi::probe` 的完整清单。
pub fn pe_exports_have_libass_required(names: &[String]) -> bool {
    // 只取几个最有鉴别力的：libass 的公开 API 都是 `ass_` 前缀，
    // 而其它 renderer（CSRI / VSFilterMod / AviSynth）没有这些名字。
    const SIGNATURE: [&str; 3] = ["ass_library_init", "ass_renderer_init", "ass_render_frame"];
    SIGNATURE.iter().all(|n| names.iter().any(|e| e == n))
}

/// 从文件读导出表。
///
/// 整个文件读进内存：导出表在任意位置，没法流式处理；
/// 第三方 renderer DLL 量级（几 MiB）可以接受。
/// 超过 [`MAX_EXPORT_SCAN_BYTES`] 时**明确拒绝**，而不是试着读到一半 ——
/// 一个几百 MiB 的文件应当先让人看它是什么，而不是被我们整个吞下去。
pub fn exports_of_file(path: &Path) -> Result<Exports, PeError> {
    const MAX_EXPORT_SCAN_BYTES: u64 = 256 * 1024 * 1024;
    let meta = std::fs::metadata(path)
        .map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
    if meta.len() > MAX_EXPORT_SCAN_BYTES {
        return Err(PeError::Unreadable(format!(
            "{}：{} 字节，超过扫导出表的上限 {} 字节",
            path.display(),
            meta.len(),
            MAX_EXPORT_SCAN_BYTES
        )));
    }
    let bytes =
        std::fs::read(path).map_err(|e| PeError::Unreadable(format!("{}：{e}", path.display())))?;
    exports_of_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(machine: u16, lfanew: usize) -> Vec<u8> {
        let mut v = vec![0u8; lfanew + 6];
        v[0] = b'M';
        v[1] = b'Z';
        v[0x3c..0x40].copy_from_slice(&(lfanew as u32).to_le_bytes());
        v[lfanew..lfanew + 4].copy_from_slice(&PE_LEAN);
        v[lfanew + 4..lfanew + 6].copy_from_slice(&machine.to_le_bytes());
        v
    }

    #[test]
    fn machine_values_map_to_the_documented_arches() {
        assert_eq!(arch_of_bytes(&fake(0x8664, 0x100)).unwrap(), PeArch::X64);
        assert_eq!(
            arch_of_bytes(&fake(0x0200, 0x100)).unwrap(),
            PeArch::Other(0x0200),
            "Itanium 不是 x64"
        );
        assert_eq!(arch_of_bytes(&fake(0x014c, 0x100)).unwrap(), PeArch::X86);
        assert_eq!(arch_of_bytes(&fake(0xaa64, 0x100)).unwrap(), PeArch::Arm64);
        assert_eq!(
            arch_of_bytes(&fake(0x1234, 0x100)).unwrap(),
            PeArch::Other(0x1234)
        );
    }

    #[test]
    fn non_pe_and_truncated_input_are_named_not_panic() {
        assert!(matches!(
            arch_of_bytes(b"not a pe file at all, really not"),
            Err(PeError::NotPe(_))
        ));
        assert!(matches!(arch_of_bytes(&[0u8; 8]), Err(PeError::NotPe(_))));
        let mut v = fake(0x8664, 0x100);
        v.truncate(0x102);
        assert!(matches!(arch_of_bytes(&v), Err(PeError::NotPe(_))));
    }

    #[test]
    fn reading_a_real_file_reports_the_arch_or_an_explained_error() {
        // 本机的系统 DLL 一定是 x64（SysWOW64 里的才是 x86），这里只做"能读且读对"的最小验证
        let system = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let candidate = std::path::Path::new(&system)
            .join("System32")
            .join("kernel32.dll");
        match arch_of_file(&candidate) {
            Ok(a) => assert_eq!(a, PeArch::X64, "System32 下的 kernel32 必然是 x64"),
            // 非本机环境（或没有该文件）时也必须给出可读的原因，而不是含糊的失败
            Err(e) => assert!(!e.to_string().is_empty()),
        }
        assert!(matches!(
            arch_of_file(std::path::Path::new("Z:/definitely-not-here.pe")),
            Err(PeError::Unreadable(_))
        ));
    }

    // ---- 导出表 ----------------------------------------------------------

    /// 构造一个最小但**合法**的 PE32+（x64）文件，带一个导出表。
    ///
    /// 布局：
    ///   0x98   e_lfanew 指向的 PE 签名
    ///   0xB0   可选头（magic 0x20b = PE32+），数据目录[0] = 导出表
    ///   0x200  节表（1 节，RVA 0x1000 → 文件偏移 0x400）
    ///   0x400  节数据：导出目录 40 字节 → 名字 RVA 表 → 序数表 → 名字串
    fn pe_with_exports(names: &[&str]) -> Vec<u8> {
        const OPT_AT: usize = 0x98;
        const OPT_SIZE: usize = 0xf0;
        const SEC_AT: usize = OPT_AT + 24 + OPT_SIZE;
        const SEC_RAW: usize = 0x400;
        const EXP_RVA: u32 = 0x1000;

        let mut v = vec![0u8; SEC_RAW + 0x1000];
        v[0] = b'M';
        v[1] = b'Z';
        v[0x3c..0x40].copy_from_slice(&(OPT_AT as u32).to_le_bytes());
        v[OPT_AT..OPT_AT + 4].copy_from_slice(&PE_LEAN);
        v[OPT_AT + 4..OPT_AT + 6].copy_from_slice(&0x8664u16.to_le_bytes());
        v[OPT_AT + 6..OPT_AT + 8].copy_from_slice(&1u16.to_le_bytes());
        v[OPT_AT + 20..OPT_AT + 22].copy_from_slice(&(OPT_SIZE as u16).to_le_bytes());
        v[OPT_AT + 24..OPT_AT + 26].copy_from_slice(&0x20bu16.to_le_bytes());
        let dd = OPT_AT + 24 + 112;
        v[dd..dd + 4].copy_from_slice(&EXP_RVA.to_le_bytes());
        v[dd + 4..dd + 8].copy_from_slice(&0x200u32.to_le_bytes());
        v[SEC_AT + 8..SEC_AT + 12].copy_from_slice(&0x1000u32.to_le_bytes());
        v[SEC_AT + 12..SEC_AT + 16].copy_from_slice(&EXP_RVA.to_le_bytes());
        v[SEC_AT + 16..SEC_AT + 20].copy_from_slice(&0x1000u32.to_le_bytes());
        v[SEC_AT + 20..SEC_AT + 24].copy_from_slice(&(SEC_RAW as u32).to_le_bytes());

        let exp = SEC_RAW;
        let n = names.len() as u32;
        let names_tbl = exp + 40;
        let ord_tbl = names_tbl + (n as usize) * 4;
        let strs = ord_tbl + (n as usize) * 2;
        v[exp + 24..exp + 28].copy_from_slice(&n.to_le_bytes());
        v[exp + 32..exp + 36].copy_from_slice(&(EXP_RVA + 40).to_le_bytes());
        v[exp + 36..exp + 40].copy_from_slice(&(EXP_RVA + 40 + n * 4).to_le_bytes());
        let mut cur = strs;
        for (i, name) in names.iter().enumerate() {
            let rva = EXP_RVA + (cur - SEC_RAW) as u32;
            v[names_tbl + i * 4..names_tbl + i * 4 + 4].copy_from_slice(&rva.to_le_bytes());
            v[ord_tbl + i * 2..ord_tbl + i * 2 + 2].copy_from_slice(&(i as u16).to_le_bytes());
            v[cur..cur + name.len()].copy_from_slice(name.as_bytes());
            cur += name.len() + 1;
        }
        v
    }

    #[test]
    fn exports_are_read_from_a_synthetic_pe() {
        let pe = pe_with_exports(&["csri_open", "csri_close", "other"]);
        let e = exports_of_bytes(&pe).unwrap();
        assert_eq!(e.names, vec!["csri_open", "csri_close", "other"]);
        assert_eq!(e.declared, 3);
        assert!(e.note.is_none(), "完整读到就不该有 note：{:?}", e.note);
        assert!(e.has("csri_open"));
        assert!(!e.has("csri_render"));
        assert!(!e.is_empty());
    }

    /// 名字匹配必须**区分大小写**：PE 导出名是区分大小写的，
    /// 而大小写不敏感会把一个 `Csri_Open` 判成 CSRI provider。
    #[test]
    fn export_name_matching_is_case_sensitive() {
        let pe = pe_with_exports(&["CSRI_OPEN"]);
        let e = exports_of_bytes(&pe).unwrap();
        assert!(e.has("CSRI_OPEN"));
        assert!(!e.has("csri_open"), "不该大小写不敏感地命中");
    }

    /// 没有导出表不是错误：很多 DLL（资源库）本来就不导出任何东西。
    #[test]
    fn a_pe_without_exports_is_empty_not_an_error() {
        let mut pe = pe_with_exports(&[]);
        let dd = 0x98 + 24 + 112;
        pe[dd..dd + 8].copy_from_slice(&[0u8; 8]);
        let e = exports_of_bytes(&pe).unwrap();
        assert!(e.is_empty());
        assert_eq!(e.declared, 0);
        assert!(e.note.is_none(), "没有导出表是正常情形，不该报 note");
    }

    /// 非 PE / 截断输入要报错而不是 panic，也不能诈称读到了符号。
    #[test]
    fn export_reading_on_garbage_is_an_error_not_a_panic() {
        assert!(matches!(
            exports_of_bytes(b"definitely not a PE file"),
            Err(PeError::NotPe(_))
        ));
        assert!(matches!(exports_of_bytes(&[]), Err(PeError::NotPe(_))));
        let mut pe = pe_with_exports(&["a"]);
        pe.truncate(0x200);
        match exports_of_bytes(&pe) {
            Err(PeError::NotPe(_)) => {}
            Ok(e) => assert!(
                e.is_empty() || e.note.is_some(),
                "截断的输入不该给出完整列表"
            ),
            Err(other) => panic!("应当是 NotPe，收到 {other:?}"),
        }
    }

    /// 声明了巨量名字的表不能把内存吃光。
    ///
    /// 两道防护都得在，而**哪一道先触发取决于构造**：
    /// 名字 RVA 表很快就超出文件末（这道先拦），而计数上限拦那些
    /// “声称很多、而文件真的够大”的情形。两者都是必要的，
    /// 所以这里断言的是**结果**：没有把 4 亿条分配下去，且说了原因。
    #[test]
    fn a_lying_export_count_is_capped_not_allocated() {
        let mut pe = pe_with_exports(&["x"]);
        pe[SEC_RAW_ONLY + 24..SEC_RAW_ONLY + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        let e = exports_of_bytes(&pe).unwrap();
        assert!(
            e.note.is_some(),
            "声称 4 亿个名字却完整读到了，说明没有任何上限"
        );
        // 关键：没有按声明值分配。读到多少不重要，没爆内存才重要。
        assert!(
            e.names.len() < 10_000,
            "按声明值分配了 {} 条，上限没起作用",
            e.names.len()
        );
        assert_eq!(e.declared, u32::MAX, "声明的值要如实报出来");
    }

    const SEC_RAW_ONLY: usize = 0x400;

    /// **真 DLL 用例**：从本机的 kernel32 读出它确实导出那些符号。
    ///
    /// 这条是"实现真的对"的证据 —— 纯构造数据的用例只能证明
    /// 我按自己以为的布局写了代码，而布局可能整体偏移。
    #[test]
    fn a_real_dll_exports_its_real_symbols() {
        let system = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
        let k32 = std::path::Path::new(&system)
            .join("System32")
            .join("kernel32.dll");
        let Ok(e) = exports_of_file(&k32) else {
            eprintln!("跳过：读不到 {}", k32.display());
            return;
        };
        assert!(
            e.declared > 100,
            "kernel32 导出的名字应当有很多，实际 {}",
            e.declared
        );
        for name in ["CreateFileW", "LoadLibraryExW", "GetProcAddress"] {
            assert!(
                e.has(name),
                "kernel32 应当导出 {name}；前几个读到的是 {:?}",
                e.names.iter().take(8).collect::<Vec<_>>()
            );
        }
    }

    /// 真 DLL 用例：libass 的导出表要能让能力探测工作。
    ///
    /// 同时钉住"两条能力链不混"：libass **不该**导出 CSRI 符号。
    #[test]
    fn a_real_libass_exports_the_symbols_the_capability_probe_needs() {
        let dll = Path::new("D:/Code/SubtitleToolkit/third_party/libass-9.dll");
        let Ok(e) = exports_of_file(dll) else {
            eprintln!("跳过：找不到 {}", dll.display());
            return;
        };
        for name in ["ass_library_init", "ass_renderer_init", "ass_render_frame"] {
            assert!(
                e.has(name),
                "libass 应当导出 {name}；实际导出 {} 个名字",
                e.declared
            );
        }
        assert!(
            !e.has("csri_open"),
            "libass 不是 CSRI provider，却读到了 csri_open"
        );
    }

    /// 真 DLL 用例：VSFilterMod 候选文件**到底是不是** CSRI provider。
    ///
    /// 这条把 12 §1B 的一个悬而未决的事实变成可核对的结论。
    /// 不管结论是哪一边，都要能说出**为什么**。
    #[test]
    fn the_vsfiltermod_candidate_is_probed_by_exports_not_by_name() {
        let dll = Path::new("D:/Code/SubtitleToolkit/third_party/VSFilterMod.dll");
        let Ok(e) = exports_of_file(dll) else {
            eprintln!("跳过：找不到 {}", dll.display());
            return;
        };
        assert!(e.declared > 0, "VSFilterMod 候选应当有导出");
        // 只报告事实，不在这里断言它"必须是"哪一种：
        // 断言会随上游构建而变，而这条用例的价值是**把事实钉在报告里**。
        let csri = e.has("csri_open")
            && e.has("csri_close")
            && e.has("csri_render")
            && e.has("csri_get_renderer");
        eprintln!(
            "VSFilterMod.dll：导出 {} 个名字，CSRI 符号齐全 = {csri}",
            e.declared
        );
        let vs: Vec<&String> = e
            .names
            .iter()
            .filter(|n| n.starts_with("vsfm") || n.contains("VapourSynth"))
            .take(8)
            .collect();
        eprintln!("  含 vsfm/VapourSynth 的导出（前 8）：{vs:?}");
        let csri_like: Vec<&String> = e
            .names
            .iter()
            .filter(|n| n.starts_with("csri"))
            .take(8)
            .collect();
        eprintln!("  含 csri 的导出（前 8）：{csri_like:?}");
    }
}
