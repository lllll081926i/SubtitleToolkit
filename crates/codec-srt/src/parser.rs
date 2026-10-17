//! 宽容的 SRT 块状态机。
//!
//! 规范实况（24/25 语料与 AT-PATH/AT-OUT）：扩展名与"官方语法"都不可信，所以这里对
//! 序号缺失、毫秒分隔符 `.`、毫秒位数不为 3、时间行多余空白、空白字符分隔行、CR 换行
//! 一律容忍并记 non-standard；只有"无法安全归属"的块（无 `-->`、时间行之前还有别的内容、
//! 时间字段非法）整体按原文保留为 `UnrecognizedBlock`——绝不 panic，也绝不丢内容。

use core_model::error::{Domain, StkError};
use core_model::loss::LossReport;
use core_model::time::Time;

use crate::encoding::Newline;
use crate::ir::{TextCue, TextSubtitle, UnrecognizedBlock};

/// 行终止符种类。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Term {
    CrLf,
    Lf,
    Cr,
}

pub(crate) struct Split<'a> {
    pub(crate) lines: Vec<&'a str>,
    /// 首次检测到的风格；`Cr` 归一为 `Lf`（`Newline` 只有两种输出风格）。
    pub(crate) newline: Newline,
    pub(crate) mixed: bool,
    pub(crate) cr_seen: bool,
}

pub(crate) fn split_lines(text: &str) -> Split<'_> {
    // 行数 <= 字符数；用 len()/4 + 1 起步，小输入不过度分配、大输入避免反复扩容。
    let mut lines: Vec<&str> = Vec::with_capacity(text.len() / 4 + 1);
    let mut first: Option<Term> = None;
    let mut seen = 0u8;
    let mut rest = text;
    while let Some(pos) = rest.find(['\r', '\n']) {
        lines.push(&rest[..pos]);
        let cr = matches!(rest.as_bytes().get(pos), Some(b'\r'));
        let lf_after = matches!(rest.as_bytes().get(pos + 1), Some(b'\n'));
        if cr && lf_after {
            seen |= 1;
            if first.is_none() {
                first = Some(Term::CrLf);
            }
            rest = &rest[pos + 2..];
        } else if cr {
            seen |= 4;
            if first.is_none() {
                first = Some(Term::Cr);
            }
            rest = &rest[pos + 1..];
        } else {
            seen |= 2;
            if first.is_none() {
                first = Some(Term::Lf);
            }
            rest = &rest[pos + 1..];
        }
    }
    if !rest.is_empty() {
        lines.push(rest);
    }
    Split {
        lines,
        newline: match first {
            Some(Term::CrLf) => Newline::CrLf,
            _ => Newline::Lf,
        },
        mixed: seen.count_ones() > 1,
        cr_seen: seen & 4 != 0,
    }
}

#[derive(Default)]
struct Flags {
    mixed: bool,
    cr: bool,
    blank_sep: bool,
    missing_index: bool,
    gap_index: bool,
    dot_ms: bool,
    no_ms: bool,
    ms_width: bool,
    second_arrow: bool,
    unrecognized: bool,
}

#[derive(Default)]
struct State {
    cues: Vec<TextCue>,
    unrecognized: Vec<UnrecognizedBlock>,
    flags: Flags,
    last_index: Option<u32>,
}

/// 把已经切好的行灌进 `TextSubtitle`（`newline`/`cues`/`unrecognized` 三字段）。
pub(crate) fn collect(
    split: Split<'_>,
    sub: &mut TextSubtitle,
    loss: &mut LossReport,
) -> Option<StkError> {
    let Split {
        lines,
        newline,
        mixed,
        cr_seen,
    } = split;
    sub.newline = newline;
    let mut st = State::default();
    st.flags.mixed = mixed;
    st.flags.cr = cr_seen;
    let mut block: Vec<&str> = Vec::new();
    for line in lines.iter().copied() {
        if is_separator(line) {
            if !block.is_empty() {
                flush_block(&block, &mut st, loss);
                block.clear();
                // 23 §文件：“极端 cue 数”的防线按**条目数**算，字节数看不住
                // （200 万条空 cue 只有 40 多 MB）。超限一律报错而不是取前 N 条：
                // 静默截断会让用户拿到一份缺了后半段的字幕，而那份文件在别处看起来完全正常。
                if let Some(limit) = core_model::cue_count_exceeded(st.cues.len()) {
                    return Some(cue_limit_error(limit, st.cues.len()));
                }
            }
            if !line.is_empty() {
                st.flags.blank_sep = true;
            }
            continue;
        }
        block.push(line);
    }
    if !block.is_empty() {
        flush_block(&block, &mut st, loss);
        // 收尾块也要查：文件最后一块也可能正好跨过上限。
        if let Some(limit) = core_model::cue_count_exceeded(st.cues.len()) {
            return Some(cue_limit_error(limit, st.cues.len()));
        }
    }
    apply_flags(&st.flags, loss);
    sub.cues = st.cues;
    sub.unrecognized = st.unrecognized;
    None
}

/// 超限错误。单独一个函数是为了让两处（循环内与收尾块）给出同一条消息。
fn cue_limit_error(limit: usize, got: usize) -> StkError {
    StkError::simple(Domain::Parse, 123, "cue 条数超过本工具的处理上限")
        .with_detail(format!(
            "已解析出 {} 条，上限 {} 条",
            got.min(limit + 1),
            core_model::limits::limit_label(limit)
        ))
        .with_action("确认选对了文件；逐词时间轴这类超密字幕需要先切分")
}

/// 只有空行或纯 ASCII 空白行才算块分隔符；非 ASCII 空白视为内容（保持字节保真）。
pub(crate) fn is_separator(line: &str) -> bool {
    line.chars().all(|c| matches!(c, ' ' | '\t'))
}

fn flush_block(lines: &[&str], st: &mut State, loss: &mut LossReport) {
    let Some(ai) = lines.iter().position(|l| l.contains("-->")) else {
        st.push_unrecognized(lines, "块内没有 --> 时间行", loss);
        return;
    };
    let mut pre = 0usize;
    let mut index = None;
    if ai >= 1 {
        if let Some(v) = index_value(lines[0]) {
            index = Some(v);
            pre = 1;
        }
    }
    if ai > pre {
        st.push_unrecognized(lines, "时间行之前存在无法归属为序号的内容", loss);
        return;
    }
    let (start, end) = match parse_timing(lines[ai], &mut st.flags) {
        Ok(t) => t,
        Err(reason) => {
            st.push_unrecognized(lines, reason, loss);
            return;
        }
    };
    if ai == 0 {
        st.flags.missing_index = true;
    }
    if let Some(v) = index {
        if let Some(p) = st.last_index {
            if v != p.saturating_add(1) {
                st.flags.gap_index = true;
            }
        }
        st.last_index = Some(v);
    }
    let text_lines = &lines[ai + 1..];
    if text_lines.is_empty() {
        loss.user_attention("cue 文本为空");
    }
    let mut text = String::new();
    for (i, l) in text_lines.iter().enumerate() {
        if i > 0 {
            text.push('\n');
        }
        text.push_str(l);
        if l.contains("-->") {
            st.flags.second_arrow = true;
        }
    }
    if end <= start {
        loss.user_attention("cue 结束不晚于开始");
    }
    st.cues.push(TextCue {
        index,
        start,
        end,
        text,
    });
}

impl State {
    fn push_unrecognized(&mut self, lines: &[&str], reason: &str, loss: &mut LossReport) {
        let raw = lines.join("\n");
        self.flags.unrecognized = true;
        loss.bump(
            "user_attention",
            &format!("{reason}（原文已保留）：{}", preview(&raw)),
        );
        self.unrecognized.push(UnrecognizedBlock {
            after_cues: self.cues.len(),
            raw,
        });
    }
}

fn preview(raw: &str) -> String {
    raw.chars()
        .take(40)
        .collect::<String>()
        .replace('\n', " ⏎ ")
}

/// 解析 `HH:MM:SS,mmm --> HH:MM:SS,mmm`，容忍多余空白与 `,`/`.` 两种毫秒分隔符。
fn parse_timing(line: &str, f: &mut Flags) -> Result<(Time, Time), &'static str> {
    let (l, r) = line.split_once("-->").ok_or("块内没有 --> 时间行")?;
    Ok((parse_stamp(l.trim(), f)?, parse_stamp(r.trim(), f)?))
}

/// `HH:MM:SS` + 可选毫秒段。毫秒按**整数值**解释（`1,50` = 50ms），不猜位数补齐，
/// 这样"写出去再读回来"与 core-model 的 `from_hms_millis(0..1000)` 契约保持一致。
fn parse_stamp(s: &str, f: &mut Flags) -> Result<Time, &'static str> {
    // 分隔符是 ASCII，`find` 返回的必然是字符边界，按字节切片安全。
    let (hms, ms) = match s.find([',', '.']) {
        Some(i) => {
            if matches!(s.as_bytes().get(i), Some(b'.')) {
                f.dot_ms = true;
            }
            let ms = &s[i + 1..];
            if ms.len() != 3 {
                f.ms_width = true;
            }
            (&s[..i], ms)
        }
        None => {
            f.no_ms = true;
            (s, "")
        }
    };
    let (h, m, sec) = split_hms(hms).ok_or("时间字段结构非法（需要 HH:MM:SS）")?;
    let ms_v = if ms.is_empty() {
        0
    } else if !ms.bytes().all(|b| b.is_ascii_digit()) {
        return Err("毫秒字段含非数字字符");
    } else {
        ms.parse::<i64>().map_err(|_| "毫秒字段数值溢出")?
    };
    Time::from_hms_millis(h, m, sec, ms_v).map_err(|_| "时间字段越界（分/秒需 <60，毫秒需 <1000）")
}

fn split_hms(s: &str) -> Option<(i64, i64, i64)> {
    let (h, rest) = s.split_once(':')?;
    let (m, sec) = rest.split_once(':')?;
    Some((number(h)?, number(m)?, number(sec)?))
}

fn number(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

fn index_value(line: &str) -> Option<u32> {
    let t = line.trim_matches(|c: char| c == ' ' || c == '\t');
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<u32>().ok()
}

fn apply_flags(f: &Flags, loss: &mut LossReport) {
    if f.mixed {
        loss.nonstandard("换行风格混用（写出统一为首次检测到的风格）");
    }
    if f.cr {
        loss.nonstandard("存在仅 CR 的换行（Newline 无 CR 变体，写出归一为 LF）");
    }
    if f.blank_sep {
        loss.nonstandard("用只含空白字符的行充当块分隔空行");
    }
    if f.missing_index {
        loss.nonstandard("存在缺少序号行的块（index=None）");
    }
    if f.gap_index {
        loss.nonstandard("序号不连续（写出时连续重排）");
    }
    if f.dot_ms {
        loss.nonstandard("毫秒用 . 分隔（写出统一为 ,）");
    }
    if f.no_ms {
        loss.nonstandard("时间戳缺少毫秒字段（按 0 毫秒处理）");
    }
    if f.ms_width {
        loss.nonstandard("毫秒位数不是 3（按整数值解释，写出补零）");
    }
    if f.second_arrow {
        loss.nonstandard("文本行中再次出现 -->（按 cue 文本内容保留）");
    }
    if f.unrecognized {
        loss.user_attention("存在未识别块：原文整体保留，写出时按原位置还原");
    }
}
