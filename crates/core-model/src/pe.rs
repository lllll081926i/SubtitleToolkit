//! PE 头里的 Machine 字段（12 §3 / 23 §DLL）：**不看文件名，也不看目录名**。
//!
//! 12 §3 的禁令是"不得跨位数 `LoadLibrary`"。要在调用 `LoadLibrary` 之前就知道位数，
//! 唯一的依据就是文件本身，所以这里只读 PE 头，不加载、不执行。
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
}
