//! 22 §1 的结构化日志：`tracing` + 落在数据目录 `logs/` 下的滚动文件。
//!
//! ## 规格要求与本文件的对应
//!
//! | 22 §1 原文 | 这里的落法 |
//! |---|---|
//! | `tracing` structured logging | `tracing` 的 event 字段，不拼字符串 |
//! | 默认 `%LOCALAPPDATA%/…/logs` | 走 [`DataDir`]，于是便携包内自动落在包内 `data/logs/` |
//! | 滚动 7 天 | 按日期文件名 `stk-YYYY-MM-DD.log`，启动时删除 7 天前的 |
//! | 容量限制 | 启动时把总量压到 [`MAX_TOTAL_BYTES`] 以下，从最旧的开始删 |
//!
//! ## 为什么同时保留 stderr 与文件
//!
//! 22 §1 只说日志落盘，但 CLI 是**管道里的程序**：把日志完全藏进文件，
//! 用户在终端看不到任何东西，而"为什么这次跑得这么慢"这类问题需要现场看。
//! 所以两份都写：文件供事后诊断（也进诊断包），stderr 供当场看。二者是同一个
//! event 的两处输出，不是两套日志。
//!
//! ## 为什么不用 `tracing_appender` 的 rolling
//!
//! 那要再引一条依赖（含 `time` 的本地时区链），而这里需要的滚动规则只要
//! "按日切文件 + 启动时清理"。用自己的两段小逻辑代替，能把"哪些文件会被删"
//! 写成可被测试断言的事实 —— 删文件是破坏性动作，不该藏在一个黑盒 appender 里。
//!
//! ## 时间戳格式
//!
//! 文件按**本地**日期命名（用户找日志时想的是"今天"，不是 UTC 日期）。
//! 本地时区换算用 Windows 的 `GetLocalTime`（`windows-sys` 已在依赖树里，
//! 见 `win-job`），不引时区数据库。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::datadir::DataDir;

/// 保留天数（22 §1 的「滚动 7 天」）。
pub const KEEP_DAYS: u64 = 7;

/// 日志总量上限。10 MiB 的依据：诊断包要发出去，而日志是包里唯一会持续变大的东西；
/// 7 天 × 多进程的普通使用量远低于这个数，而异常刷屏时这一条能兜住。
pub const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;

/// 单个日志文件超过这个大小就轮转成 `.1`、`.2`…（同一天内跑很多次时用得上）。
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// 已初始化的日志目标，供诊断包与报告查询"日志在哪"。
static LOG_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 初始化日志。**只应该被调一次**；重复调用是安全的（直接返回）。
///
/// `level` 取自 36 §1 的 AppSettings `log_level`：`off`/`error`/`warn`/`info`/`debug`/`trace`。
/// 解析不了就按 `info` 处理并**记一条警告**——用户拼错的级别名不该让程序起不来，
/// 但也不该被静默忽略。
pub fn init(level: &str) {
    if LOG_DIR.get().is_some() {
        return;
    }
    let dir = DataDir::resolve_current_exe().file("logs");
    if let Some(d) = &dir {
        // 目录建不出来不是致命错误：stderr 那份仍然可用。
        // 但要把失败记下来 —— 否则用户会以为"日志开了"而其实是空的。
        if let Err(e) = std::fs::create_dir_all(d) {
            let _ = LOG_DIR.set(None);
            eprintln!(
                "[stk] 日志目录创建失败（{e}）：{}；本次只写 stderr",
                d.display()
            );
            init_stderr_only(level);
            return;
        }
        prune(d);
        let file = d.join(format!("stk-{}.log", today_stamp()));
        let _ = LOG_DIR.set(Some(d.clone()));
        init_file_and_stderr(level, &file);
        return;
    }
    let _ = LOG_DIR.set(None);
    init_stderr_only(level);
}

/// 日志目录（`None` = 没有落盘位置，只写 stderr）。诊断包用它定位要收什么。
pub fn log_dir() -> Option<&'static Path> {
    LOG_DIR.get().and_then(|o| o.as_deref())
}

/// 当前会写入的日志文件名（报告用）。
pub fn current_log_file() -> Option<PathBuf> {
    log_dir().map(|d| d.join(format!("stk-{}.log", today_stamp())))
}

/// 把 `level` 解析成 `tracing` 的过滤指令。
///
/// 只接受整层级名，不支持 `crate=level` 这种语法：36 §1 的设置项是一个**级别**，
/// 不是一个过滤器表达式；允许完整语法会把"用户能不能看懂这个值"变成另一个问题。
fn directive(level: &str) -> (String, Option<String>) {
    const KNOWN: &[&str] = &["off", "error", "warn", "info", "debug", "trace"];
    let l = level.trim().to_ascii_lowercase();
    if KNOWN.contains(&l.as_str()) {
        if l == "off" {
            // tracing 没有 "off" 这个 LevelFilter 名；用不存在的 target 关掉全部。
            return ("stk=off".to_string(), None);
        }
        return (format!("stk={l},{l}"), None);
    }
    (
        "stk=info,info".to_string(),
        Some(format!(
            "log_level 取值 {level:?} 不是已知级别（{}），按 info 处理",
            KNOWN.join("/")
        )),
    )
}

/// 无 ANSI 的字段格式化器：给文件那份日志用。
///
/// 为什么不直接用 `DefaultFields`：它的 ANSI 开关不跟 `layer.with_ansi(false)` 走，
/// 于是 span 的 `corr_id=` 这类字段名会在文件里留下 `ESC[3m` 序列 ——
/// 而那份文件要进诊断包（22 §5）。
///
/// 实现很小，因为只需要“名字=值 + 空格”这个形状；
/// `FormatFields` 这个 trait 就是 `tracing` 为这种定制准备的。
struct NoAnsiFields;

impl<'writer> tracing_subscriber::fmt::format::FormatFields<'writer> for NoAnsiFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: tracing_subscriber::fmt::format::Writer<'writer>,
        fields: R,
    ) -> core::fmt::Result {
        let mut buf = String::new();
        let mut v = PlainFields(&mut buf);
        fields.record(&mut v);
        let mut writer = writer;
        writer.write_str(buf.trim_end())
    }
}

/// 逐字段写成 `name=value`。
struct PlainFields<'a>(&'a mut String);

impl tracing_subscriber::field::Visit for PlainFields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn core::fmt::Debug) {
        use core::fmt::Write as _;
        // 值走 `{:?}`：字符串会带引号，与终端那份的写法一致。
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

fn init_file_and_stderr(level: &str, file: &Path) {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let (dir, warn) = directive(level);
    let filter = match tracing_subscriber::EnvFilter::try_new(&dir) {
        Ok(f) => f,
        Err(_) => tracing_subscriber::EnvFilter::new("stk=info,info"),
    };
    // 文件那份用无颜色的纯文本：它会被塞进诊断包，ANSI 转义码在那里只是噪声。
    //
    // 为什么 `with_ansi(false)` 不够：它只关掉**事件字段**的着色，
    // 而 span 的字段名（`stk{corr_id=…}` 里的 `corr_id`）走的是另一个开关。
    // 两个都关才能得到真无转义字节的文件 —— 而“文件里没有 ESC 字节”
    // 是一条断言过的事实（见 cli/tests/logging.rs）。
    let file_layer = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .ok()
        .map(|f| {
            // 三处都要关：`layer.with_ansi`（事件）、`format.with_ansi`（时间/级别/目标）、
            // 以及 `DefaultFields`（**span 的字段名**）。第三处是最容易漏的那个 ——
            // span 的 `corr_id=` 这类字段名走它自己的开关。
            let fmt = tracing_subscriber::fmt::format()
                .with_ansi(false)
                .with_target(true)
                .with_level(true);
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .event_format(fmt)
                .fmt_fields(NoAnsiFields)
                .with_writer(std::sync::Mutex::new(f))
        });
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_ansi(true)
        .with_writer(std::io::stderr);

    let reg = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer);
    // 已经装过 subscriber 的进程（例如测试里跑多次）不该 panic
    let _ = reg.try_init();
    if let Some(w) = warn {
        tracing::warn!(log_level = %level, "{w}");
    }
    roll_if_too_big(file);
}

fn init_stderr_only(level: &str) {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let (dir, warn) = directive(level);
    let filter = match tracing_subscriber::EnvFilter::try_new(&dir) {
        Ok(f) => f,
        Err(_) => tracing_subscriber::EnvFilter::new("stk=info,info"),
    };
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
    if let Some(w) = warn {
        tracing::warn!(log_level = %level, "{w}");
    }
}

/// 同一天内文件超限就把它挪成 `.1`（再超就 `.2`）。只挪一次，不递归。
fn roll_if_too_big(file: &Path) {
    let Ok(md) = std::fs::metadata(file) else {
        return;
    };
    if md.len() < MAX_FILE_BYTES {
        return;
    }
    let n = next_rotation_index(file);
    let _ = std::fs::rename(file, file.with_extension(format!("log.{n}")));
}

fn next_rotation_index(file: &Path) -> u32 {
    for i in 1..1000u32 {
        if !file.with_extension(format!("log.{i}")).exists() {
            return i;
        }
    }
    999
}

/// 清理：先按天数删，再按总量删（都从最旧的开始）。
///
/// **只碰本模块自己生成的文件名**（`stk-YYYY-MM-DD.log` 与 `stk-YYYY-MM-DD.log.N`）。
/// 日志目录有可能被用户或别的工具共用，而"清理一个目录"这种动作的范围必须
/// 是能逐条说清楚的 —— 这是不硬删别人资产的纪律在日志上的落法。
fn prune(dir: &Path) {
    let now = now_days();
    let mut files: Vec<(PathBuf, u64, u64)> = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let Some(stamp) = stamp_of(&p) else {
            continue;
        };
        let Ok(md) = ent.metadata() else { continue };
        files.push((p, stamp, md.len()));
    }
    // 1) 超过保留天数
    for (p, stamp, _) in &files {
        if now.saturating_sub(*stamp) > KEEP_DAYS {
            let _ = std::fs::remove_file(p);
        }
    }
    // 2) 总量超限：从最旧的开始删，直到降到上限以内
    let mut rest: Vec<(PathBuf, u64, u64)> =
        files.into_iter().filter(|(p, _, _)| p.exists()).collect();
    rest.sort_by_key(|(_, stamp, _)| *stamp);
    let mut total: u64 = rest.iter().map(|(_, _, len)| *len).sum();
    for (p, _, len) in rest {
        if total <= MAX_TOTAL_BYTES {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// 从 `stk-YYYY-MM-DD.log`（或 `.log.N`）里取出日期戳，用于排序与天数判断。
/// 名字不符合就返回 `None`——**不认识的文件一律不动**。
fn stamp_of(p: &Path) -> Option<u64> {
    let name = p.file_name()?.to_str()?;
    if !name.starts_with("stk-") {
        return None;
    }
    let rest = &name[4..];
    let date = rest.split(".log").next()?;
    let mut it = date.split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// 今天的日期戳（本地时间）。
fn today_stamp() -> String {
    let (y, m, d) = local_ymd();
    format!("{y:04}-{m:02}-{d:02}")
}

/// 今天距 UNIX 纪元的天数（本地日期）。
fn now_days() -> u64 {
    let (y, m, d) = local_ymd();
    days_from_civil(y as i64, m as u32, d as u32)
}

/// 本地年月日。用 Windows 的 `GetLocalTime`：文件按本地日期命名，
/// 而用户找日志时想的是"今天"，不是 UTC 日期。
///
/// 非 Windows 上退回 UNIX 纪元的天数（本项目的正式 target 只有 Windows，
/// 这条分支存在只是为了不让测试环境编译失败）。
#[cfg(windows)]
fn local_ymd() -> (u16, u16, u16) {
    #[repr(C)]
    #[derive(Default)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    extern "system" {
        fn GetLocalTime(t: *mut SystemTime);
    }
    let mut t = SystemTime::default();
    unsafe { GetLocalTime(&mut t) };
    if t.year == 0 {
        // 理论上不会发生；发生了也不能让日志模块崩
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|x| x.as_secs() / 86_400)
            .unwrap_or(0);
        let (y, m, dd) = civil_from_days(d as i64);
        return (y as u16, m as u16, dd as u16);
    }
    (t.year, t.month, t.day)
}

#[cfg(not(windows))]
fn local_ymd() -> (u16, u16, u16) {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|x| x.as_secs() / 86_400)
        .unwrap_or(0);
    let (y, m, dd) = civil_from_days(d as i64);
    (y as u16, m as u16, dd as u16)
}

/// Howard Hinnant 的 `days_from_civil`：公历年月日 → 距 1970-01-01 的天数。
/// 用它而不是自己判闰年：这一族算法有一处经典的两月边界错误，
/// 用一份被广泛验证过的实现比自写更省事。
fn days_from_civil(y: i64, m: u32, d: u32) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = ((m + 9) % 12) as u64;
    let doy = (153 * mp + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    days.max(0) as u64
}

/// 反向：天数 → 公历年月日（只用于"拿不到本地时间"的兜底）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 日期换算必须与已知值对上：这里错一天，7 天滚动就会多删或少删一整天的日志。
    #[test]
    fn civil_date_conversions_match_known_values() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), 0, "纪元前被夹到 0");
        // 2000-03-01 是闰年之后那一天，是经典的两月边界
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        // 这几个期望值用 Python 的 datetime 独立核对过（不是手算）：
        // (dt.date(2026,10,17) - dt.date(1970,1,1)).days == 20743
        assert_eq!(days_from_civil(2026, 10, 17), 20743);
        // 往返
        for d in [0u64, 1, 11017, 20743, 20758] {
            let (y, m, dd) = civil_from_days(d as i64);
            assert_eq!(days_from_civil(y, m, dd), d, "{y}-{m}-{dd}");
        }
    }

    #[test]
    fn only_our_own_file_names_are_recognized_for_pruning() {
        assert_eq!(
            stamp_of(Path::new("C:/x/stk-2026-10-17.log")),
            Some(days_from_civil(2026, 10, 17))
        );
        // 同日轮转出来的也要认得，否则它们永远不被清理
        assert_eq!(
            stamp_of(Path::new("C:/x/stk-2026-10-17.log.1")),
            Some(days_from_civil(2026, 10, 17))
        );
        // 别人的文件一律不认 —— 清理动作的范围必须能逐条说清楚
        for other in [
            "app.log",
            "stk-.log",
            "stk-2026-13-01.log",
            "stk-2026-10-32.log",
            "notes.txt",
            "stk-2026-10.log",
            "stk-2026-10-17-extra.log",
        ] {
            assert_eq!(
                stamp_of(Path::new(other)),
                None,
                "{other} 不该被当成我们的日志"
            );
        }
    }

    /// 天数与总量两条规则都要真的删文件，且**只删**自己的。
    #[test]
    fn pruning_removes_only_old_or_excess_own_files() {
        let d = std::env::temp_dir().join(format!("stk-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let today = days_from_civil(
            super::local_ymd().0 as i64,
            super::local_ymd().1 as u32,
            super::local_ymd().2 as u32,
        );
        let (y, m, dd) = civil_from_days(today as i64 - 30);
        let old = d.join(format!("stk-{y:04}-{m:02}-{dd:02}.log"));
        std::fs::write(&old, "x".repeat(100)).unwrap();
        let foreign = d.join("someone-else.log");
        std::fs::write(&foreign, "keep me").unwrap();

        let this = d.join(format!("stk-{}.log", today_stamp()));
        std::fs::write(&this, "current").unwrap();

        prune(&d);
        assert!(!old.exists(), "30 天前的该被删");
        assert!(foreign.exists(), "别人的文件不该被动");
        assert!(this.exists(), "今天的该留着");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 总量超限时从最旧的开始删。
    #[test]
    fn pruning_enforces_the_total_size_budget_from_the_oldest() {
        let d = std::env::temp_dir().join(format!("stk-logs-b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();

        let today = days_from_civil(
            super::local_ymd().0 as i64,
            super::local_ymd().1 as u32,
            super::local_ymd().2 as u32,
        );
        // 8 个文件 × 2 MiB = 16 MiB > 10 MiB 上限；都在保留期内
        let mut names = Vec::new();
        for i in 0..8u64 {
            let (y, m, dd) = civil_from_days(today as i64 - i as i64);
            let p = d.join(format!("stk-{y:04}-{m:02}-{dd:02}.log"));
            std::fs::write(&p, vec![b'x'; 2 * 1024 * 1024]).unwrap();
            names.push(p);
        }
        let newest = names[0].clone(); // 今天那份是最新的
        prune(&d);
        let left: u64 = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(left <= MAX_TOTAL_BYTES, "总量没被压到上限内：{left}");
        assert!(newest.exists(), "最新的那份不该被删");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unknown_level_falls_back_to_info_and_is_reported() {
        for good in ["off", "error", "warn", "info", "debug", "trace", "INFO"] {
            let (dir, warn) = directive(good);
            assert!(warn.is_none(), "{good} 不该报警");
            assert!(!dir.is_empty());
        }
        let (dir, warn) = directive("loud");
        assert_eq!(dir, "stk=info,info");
        assert!(warn.unwrap().contains("loud"), "要说出是哪个值不认识");
    }
}
