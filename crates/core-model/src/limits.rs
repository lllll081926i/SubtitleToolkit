//! 23 §文件 的「极端 cue 数」上限：**条目数**的共用常量与判定。
//!
//! ## 为什么字节数上限不够
//!
//! 已经有 1 GiB / 4 GiB 的整文件上限（见 `cli::pipeline::read_bounded`），
//! 但它拦不住"小文件、极多条"。两个真实的形状：
//!
//! 1. **空 cue 洪水**：一个 SRT 里塞 200 万条 `1\n00:00:01,000 --> 00:00:02,000\n`，
//!    只有 40 多 MB，但解析后每 cue 要占一个 `TextCue`（几个 String + 时间）。
//!    内存放大几十倍，而且下游每个 cue 都要建 stage。
//! 2. **解析即倍增**：某些路径会把 cue 展开/切分，条目数比输入行数还多。
//!
//! 23 §文件 把"极端 cue 数"与"大文件"分开列出，就是因为这两件事的防线不同：
//! 前者按**条目数**算，字节数看不住。
//!
//! ## 为什么上限选这个数
//!
//! 100 万条远超任何真实字幕：一部 3 小时、每秒两句的片子也才 2.16 万条，
//! 100 万是它的 46 倍。而有声书/播客那种逐词时间轴（真实存在，且 cue 特别密）
//! 通常也就几万条。
//!
//! 上限取值同时考虑了下游：每条 cue 进批处理会生成一个 stage，
//! 100 万 stage 已经在"能跑完但很慢"的边缘；再往上就不是"用户想要的东西"了。
//!
//! ## 一条纪律：报出来而不是静默截断
//!
//! 超限一律**报错**，绝不"取前 N 条"：静默截断会让用户拿到一份缺了后半段的字幕，
//! 而那份文件在别处看起来完全正常。这是本仓库一贯的取向（同 15 §6 的覆盖策略、
//! 23 §文件 的拒绝语义）。

/// SRT 的 cue 数上限。
pub const CUE_COUNT_LIMIT: usize = 1_000_000;

/// 文本字幕（SRT）的每文件 cue 数上限。
pub const EVENT_COUNT_LIMIT: usize = 1_000_000;

/// 判定结果。返回 `None` 表示在上限之内。
///
/// 单立一个函数而不是内联 `if n > LIMIT`：上限**检查点**会散在几个解析器里，
/// 而它们必须对同一条规则给出同一个答案（尤其是错误消息里的数字）。
pub fn cue_count_exceeded(n: usize) -> Option<usize> {
    (n > CUE_COUNT_LIMIT).then_some(CUE_COUNT_LIMIT)
}

/// 可读上限，供错误消息用（按万为单位更易读，但不四舍五入到失真）。
pub fn limit_label(limit: usize) -> String {
    format!("{limit} 条")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limit_is_inclusive_like_the_byte_limits() {
        assert_eq!(
            cue_count_exceeded(CUE_COUNT_LIMIT),
            None,
            "恰好等于上限要放行"
        );
        assert_eq!(
            cue_count_exceeded(CUE_COUNT_LIMIT + 1),
            Some(CUE_COUNT_LIMIT),
            "多一条就要拒"
        );
        assert_eq!(cue_count_exceeded(0), None);
    }

    /// 上限必须远高于真实语料，否则这个限制会比它防的问题更常出问题。
    /// 3 小时 × 每秒 2 句 = 21600 条；装下它应当是绰绰有余的事。
    #[test]
    fn the_limit_is_far_above_any_realistic_subtitle() {
        let three_hours_at_two_per_second = 3 * 3600 * 2;
        assert!(
            CUE_COUNT_LIMIT > three_hours_at_two_per_second * 40,
            "上限 {CUE_COUNT_LIMIT} 对真实字幕余量不足"
        );
    }

    #[test]
    fn the_label_carries_the_real_number() {
        assert_eq!(limit_label(1_000_000), "1000000 条");
    }
}
