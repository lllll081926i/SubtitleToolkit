//! 22 §2 的 correlation ID：每个 Job、每个 Output、每个 renderer session 一个 UUID。
//!
//! ## 规格要的是什么
//!
//! 22 §2 原文只有两句：「每个 Job/Output/renderer session 都有 UUID；
//! 用户看到的错误卡可复制 ID。」这两句决定了三件事：
//!
//! 1. **必须可复制**。不是日志内部的 trace id，而是用户能在错误卡上抄下来、
//!    贴进 issue 的东西。所以它必须稳定地出现在**报告**里（`22 §4` 的错误格式那一条）。
//! 2. **必须有层级**。Job → Output → session 是三层，而不是一个全局编号：
//!    一个批量任务里哪个输出出的问题，靠一个扁平 id 是说不清的。
//! 3. **必须能关联起来**。同一个 Job 的所有 Output 应当能从 id 上看出同源，
//!    否则用户在日志里翻到三条记录也不知道它们是同一次执行。
//!
//! ## 格式：为什么是可读前缀 + 随机尾
//!
//! 纯随机 UUID v4 能满足唯一性，但用户抄下来之后没法核对"这是我那次"。
//! 所以这里用 `J-<8hex>-<12hex>` 这种形状：前缀标出层级（`J`ob / `O`utput /
//! `R`enderer），中间一段是**本次 Job 的共享段**（同一个 Job 的所有 Output 都有它，
//! 于是"这三条是一家人"在字面上就成立），最后一段是各自的唯一尾。
//!
//! 不用 `uuid` crate：只需要 122 位的随机性，而 `getrandom` 已经不在这棵依赖树上；
//! 引一条新依赖（21 §11 要审计来源与许可）去生成 16 个随机字节不成比例。
//! 这里的随机源是 [`Random`]：Windows 上走 `BCryptGenRandom`
//! （系统 CSPRNG，与 `rand` 库同一个底层），其他平台退回时间 + 计数器混合。
//! 它**不用于任何安全用途**——只是要"两次运行的 id 几乎不可能撞上"。
//!
//! ## 一条纪律
//!
//! **id 不能携带用户数据**。它会被贴进公开的 issue，所以里面不许有路径、
//! 文件名、用户名。这一条由"只由随机字节与固定前缀组成"在结构上保证。

use std::fmt;

/// 层级。决定 id 的第一个字符，让人一眼看出这是哪一层。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scope {
    /// 一次 Job（单命令执行或一次 batch）。
    Job,
    /// 一个 Output（一个 profile 对一份输入）。
    Output,
    /// 一次 renderer session（一个 host 进程的一生）。
    Renderer,
}

impl Scope {
    pub fn prefix(self) -> char {
        match self {
            Scope::Job => 'J',
            Scope::Output => 'O',
            Scope::Renderer => 'R',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::Job => "job",
            Scope::Output => "output",
            Scope::Renderer => "renderer",
        }
    }
}

/// 一个 correlation id。
///
/// 内部存的是原始字节（`stem` 是同一个 Job 的共享段），
/// 显示时按 `{prefix}-{stem}-{tail}` 拼出来。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CorrId {
    scope: Scope,
    /// 同一个 Job 的所有 Output 共享这一段。
    stem: [u8; 4],
    /// 本层的唯一尾。
    tail: [u8; 6],
}

impl CorrId {
    /// 起一个新 Job，以及它的共享段。
    pub fn new_job() -> CorrId {
        let mut stem = [0u8; 4];
        fill(&mut stem);
        let mut tail = [0u8; 6];
        fill(&mut tail);
        CorrId {
            scope: Scope::Job,
            stem,
            tail,
        }
    }

    /// 在同一个 Job 下起一个 Output 或 renderer session。
    ///
    /// 继承 `stem` 是有意的：22 §2 要的是"能关联"，
    /// 而"同一个 Job 的全部下级共享一段"就是那件事的最小充分实现。
    pub fn child(&self, scope: Scope) -> CorrId {
        let mut tail = [0u8; 6];
        fill(&mut tail);
        CorrId {
            scope,
            stem: self.stem,
            tail,
        }
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// 本 Job 的共享段（列表里按它分组就能把一次执行聚在一起）。
    pub fn stem_hex(&self) -> String {
        hex(&self.stem)
    }

    /// `{prefix}-{stem}-{tail}`。
    pub fn as_string(&self) -> String {
        format!(
            "{}-{}-{}",
            self.scope.prefix(),
            self.stem_hex(),
            hex(&self.tail)
        )
    }

    /// 从字符串还原。**只用于测试与反序列化**：
    /// 生产路径永远是"生成"而不是"解析"。
    pub fn parse(s: &str) -> Option<CorrId> {
        let mut it = s.split('-');
        let p = it.next()?;
        let stem = it.next()?;
        let tail = it.next()?;
        if it.next().is_some() || stem.len() != 8 || tail.len() != 12 {
            return None;
        }
        let scope = match p {
            "J" => Scope::Job,
            "O" => Scope::Output,
            "R" => Scope::Renderer,
            _ => return None,
        };
        Some(CorrId {
            scope,
            stem: unhex::<4>(stem)?,
            tail: unhex::<6>(tail)?,
        })
    }
}

impl fmt::Display for CorrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string())
    }
}

impl fmt::Debug for CorrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CorrId({})", self.as_string())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// 填随机字节。
fn fill(buf: &mut [u8]) {
    if bcrypt(buf) {
        return;
    }
    // 没有系统 CSPRNG 时的退路：时间 + 进程 + 计数器混合。
    // **不是为了安全**，只是让两次运行的 id 不撞上（22 §2 只要求唯一）。
    // 拿不到随机源不是失败条件，所以这里不返回 Result。
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut seed = t ^ pid.rotate_left(17) ^ n.rotate_left(33);
    for slot in buf.iter_mut() {
        // splitmix64 的一步：分布够均匀，且不需要引 rand
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        *slot = z as u8;
    }
}

/// Windows 上的系统 CSPRNG。返回 `false` 表示这个平台/这次调用拿不到。
#[cfg(windows)]
fn bcrypt(buf: &mut [u8]) -> bool {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(
            alg: *mut core::ffi::c_void,
            buffer: *mut u8,
            len: u32,
            flags: u32,
        ) -> i32;
    }
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG：让 BCrypt 自己挑算法，不必先开 provider。
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    if buf.is_empty() {
        return true;
    }
    let Ok(len) = u32::try_from(buf.len()) else {
        return false;
    };
    // SAFETY: 传的是本函数栈上的可变切片指针与它的真实长度；
    // BCryptGenRandom 只写这 len 个字节，且 null alg 与这个 flag 是文档允许的组合。
    let status = unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            buf.as_mut_ptr(),
            len,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    status == 0 // STATUS_SUCCESS
}

#[cfg(not(windows))]
fn bcrypt(_buf: &mut [u8]) -> bool {
    false
}

/// 把一个 id 塞进日志的"上下文"。
///
/// 用法上只要在两处调一次（job 开始、output 开始），中间的每条 `tracing` event
/// 就自动带上它 —— 这是选 `tracing` 而不是 `log` 的直接好处（22 §1 / §2 的联动）。
pub fn span_for(id: CorrId, name: &'static str) -> tracing::Span {
    tracing::info_span!("stk", corr_id = %id, scope = id.scope().label(), name = name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_the_documented_shape() {
        let j = CorrId::new_job();
        let s = j.as_string();
        let parts: Vec<&str> = s.split('-').collect();
        assert_eq!(parts.len(), 3, "{s}");
        assert_eq!(parts[0], "J");
        assert_eq!(parts[1].len(), 8, "{s}");
        assert_eq!(parts[2].len(), 12, "{s}");
        // 字符集：层级前缀（J/O/R）与十六进制，外加分隔的 `-`。
        // 这条断言与 `an_id_carries_no_user_data` 是同一个约束的两面，
        // 而它就在这里先把“形状”钉住。
        assert!(
            s.chars()
                .all(|c| c.is_ascii_hexdigit() || c == '-' || "JOR".contains(c)),
            "{s}"
        );
    }

    /// 同一个 Job 下的子 id 必须共享 `stem`，否则"它们是一家人"就不成立。
    /// 22 §2 要的是"能关联"，这就是那件事的最小充分实现。
    #[test]
    fn children_of_one_job_share_their_stem_and_differ_in_the_tail() {
        let j = CorrId::new_job();
        let a = j.child(Scope::Output);
        let b = j.child(Scope::Output);
        let r = j.child(Scope::Renderer);
        assert_eq!(a.stem_hex(), j.stem_hex(), "子 id 该继承 Job 的共享段");
        assert_eq!(b.stem_hex(), j.stem_hex());
        assert_eq!(r.stem_hex(), j.stem_hex());
        assert_ne!(a.as_string(), b.as_string(), "两个 Output 必须不同");
        assert_ne!(a.as_string(), r.as_string());
        assert_eq!(a.scope(), Scope::Output);
        assert_eq!(r.scope(), Scope::Renderer);
        // 前缀要能区分层级
        assert!(a.as_string().starts_with("O-"));
        assert!(r.as_string().starts_with("R-"));
        // 分组：按 stem 就能把一次执行聚起来
        assert_eq!(a.stem_hex(), j.stem_hex());
    }

    /// 两次独立运行不能撞上。
    ///
    /// 这条看着平凡，但它正是"能不能靠 id 找到那一次执行"的前提。
    /// 拒绝随机源时会退化成时间+计数器，那时的分布也必须够散。
    #[test]
    fn independent_jobs_do_not_collide() {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..2000 {
            let j = CorrId::new_job();
            assert!(seen.insert(j.as_string()), "撞了 id：{j}");
        }
        assert_eq!(seen.len(), 2000);
    }

    #[test]
    fn parse_roundtrips_and_rejects_junk() {
        let j = CorrId::new_job();
        let back = CorrId::parse(&j.as_string()).expect("自己的 id 该能解析");
        assert_eq!(back, j);
        assert_eq!(back.scope(), j.scope());

        for bad in [
            "",
            "J",
            "J-",
            "J-abcd",
            "X-01234567-0123456789ab",  // 坏前缀
            "J-01234567-0123456789a",   // 尾不够长
            "J-0123456-0123456789ab",   // 段不够长
            "J-01234567-0123456789ab-", // 多一段
            "J-zzzzzzzz-0123456789ab",  // 非十六进制
        ] {
            assert!(CorrId::parse(bad).is_none(), "{bad:?} 该被拒");
        }
    }

    /// id 不能携带用户数据：它会被贴进公开的 issue。
    /// 结构上只有固定前缀 + 十六进制，所以这条其实是"字符集"的断言。
    #[test]
    fn an_id_carries_no_user_data() {
        let s = CorrId::new_job().child(Scope::Output).as_string();
        for ch in s.chars() {
            assert!(
                ch.is_ascii_hexdigit() || ch == '-' || "JOR".contains(ch),
                "id 里出现了非固定字符：{ch:?}（{s}）"
            );
        }
        // 不含路径分隔符、空格、盘符
        assert!(!s.contains(['\\', '/', ' ', ':']), "{s}");
    }

    /// 前缀必须与层级一一对应，否则"一眼看出是哪一层"就不成立。
    #[test]
    fn every_scope_has_a_distinct_prefix() {
        let all = [Scope::Job, Scope::Output, Scope::Renderer];
        let prefixes: std::collections::BTreeSet<char> = all.iter().map(|s| s.prefix()).collect();
        assert_eq!(prefixes.len(), all.len(), "前缀撞了：{prefixes:?}");
    }
}
