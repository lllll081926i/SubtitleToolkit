//! 文档级 AST。写出只依赖 `Entry::raw` + `Entry::eol`，结构化解析永远是「原文的读取视角」，
//! 所以 `parse(src).to_string() == src` 是结构性成立而不是靠约定（08_FORMATS_AND_IR.md §1）。

use core_model::error::{Domain, StkError};

use crate::line::{head_len, skip_ws_from, split_lines, Eol};

/// UTF-8 BOM；单独记账，避免它污染首行段头识别。
pub const BOM: char = '\u{feff}';

/// V4+ 样式默认字段表：仅在段头自称 V4/V4+ 却没有 `Format:` 行时使用（libass 同策略）。
const STYLE_FIELDS_V4PLUS: &[&str] = &[
    "Name",
    "Fontname",
    "Fontsize",
    "PrimaryColour",
    "SecondaryColour",
    "OutlineColour",
    "BackColour",
    "Bold",
    "Italic",
    "Underline",
    "StrikeOut",
    "ScaleX",
    "ScaleY",
    "Spacing",
    "Angle",
    "BorderStyle",
    "Outline",
    "Shadow",
    "Alignment",
    "MarginL",
    "MarginR",
    "MarginV",
    "Encoding",
];

/// 老 SSA 的 17 字段样式表。
const STYLE_FIELDS_V4: &[&str] = &[
    "Name",
    "Fontname",
    "Fontsize",
    "PrimaryColour",
    "SecondaryColour",
    "OutlineColour",
    "BackColour",
    "Bold",
    "Italic",
    "Underline",
    "StrikeOut",
    "ScaleX",
    "ScaleY",
    "Spacing",
    "Angle",
    "BorderStyle",
    "Outline",
];

/// 事件默认字段表（最后一个字段是吞掉余下内容的 Text）。
const EVENT_FIELDS: &[&str] = &[
    "Marked", "Start", "End", "Style", "Name", "MarginL", "MarginR", "MarginV", "Effect", "Text",
];

/// SSA/VSS 的 `Style:` 值以 `Marked=Yes` 开头的老写法。
const EVENT_FIELDS_MARKED: &[&str] = &[
    "Marked=Yes",
    "Start",
    "End",
    "Style",
    "Name",
    "MarginL",
    "MarginR",
    "MarginV",
    "Effect",
    "Text",
];

/// 段头自称某版本时的字段表；用于「重复段头省略 Format」这一 Aegisub 常见写法。
const HEADER_FIELD_ALIASES: &[(&str, &[&str])] = &[
    ("v4++ styles", STYLE_FIELDS_V4PLUS),
    ("v4+ styles", STYLE_FIELDS_V4PLUS),
    ("v4 styles", STYLE_FIELDS_V4),
    ("v4++ events", EVENT_FIELDS),
    ("v4+ events", EVENT_FIELDS),
    ("v4 events", EVENT_FIELDS),
    ("document events", EVENT_FIELDS_MARKED),
    ("events", EVENT_FIELDS),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    ScriptInfo,
    Styles,
    Events,
    Fonts,
    FileList,
    Unknown,
}

impl SectionKind {
    /// 这些段的内容不是 `Key: value` 结构，而是逐行原文（附件区）。
    pub fn is_attachment(self) -> bool {
        matches!(self, SectionKind::Fonts | SectionKind::FileList)
    }

    /// 需要字段表才能读取语义的段。
    pub fn needs_format(self) -> bool {
        matches!(self, SectionKind::Styles | SectionKind::Events)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssVariant {
    /// ASS v4.00（SSA 派生）
    V4,
    /// ASS v4.00+
    V4Plus,
    /// ASS v4.00++
    V4PlusPlus,
    Unknown,
}

/// 字段表来源；非 `FormatLine` 都要进 loss 报告（08 §4 non-standard）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatSource {
    FormatLine,
    RepeatedHeader,
    Fallback,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Blank,
    /// `;` / `#` 开头的松散注释
    Remark,
    /// Events 段里的 `Comment:` 条目
    Comment,
    Dialogue,
    Style,
    /// 段内的 `Format:` 行本身
    Format,
    KeyVal,
    /// 附件区的逐行原文（预期不是结构化行）
    Raw,
    /// 解析不出结构的行：原文保留 + loss 记录
    Opaque,
}

impl EntryKind {
    /// 有 `key: ...` 结构的行。
    pub fn structured(self) -> bool {
        matches!(
            self,
            EntryKind::Comment
                | EntryKind::Dialogue
                | EntryKind::Style
                | EntryKind::Format
                | EntryKind::KeyVal
        )
    }

    /// 与事件同构（逗号字段 + 末字段吞余下内容）的行。
    pub fn event_like(self) -> bool {
        matches!(self, EntryKind::Comment | EntryKind::Dialogue)
    }

    pub fn field_line(self) -> bool {
        self.event_like() || self == EntryKind::Style
    }
}

/// 一个字段：`value` 是原文内容，`sep` 是紧跟其后的原文分隔符（含逗号与其后空白）。
/// 写回时只替换 value、原样保留 sep，因此 `"A, B"` 与 `"A,B"` 的写法不会被统一。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub value: String,
    pub sep: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    /// 不含行尾终止符的原文行（round-trip 的唯一依据）
    pub raw: String,
    pub eol: Eol,
    /// 字段/值的起始偏移（`Key: ` 之后）。Aegisub 省略 key 的事件行
    /// （`0,0:00:01.00,...`）没有 key 前缀，用首个冒号定位会错位，所以偏移在判型时确定。
    pub at: Option<usize>,
}

impl Entry {
    fn err_bad_line(raw: &str) -> StkError {
        StkError::simple(Domain::Parse, 32, "该行没有可改写的字段结构").with_detail(raw.to_owned())
    }

    pub fn render(&self, out: &mut String) {
        out.push_str(&self.raw);
        out.push_str(self.eol.str());
    }

    /// 冒号前的 key 原文（去掉首尾空白）；省略 key 的事件行没有 key。
    pub fn key(&self) -> Option<&str> {
        if !self.kind.structured() || self.at == Some(0) {
            return None;
        }
        let c = self.raw.find(':')?;
        self.raw.get(..c).map(str::trim)
    }

    /// 字段/值之前的前缀原文。
    pub fn head(&self) -> Option<&str> {
        let h = self.at?;
        self.raw.get(..h)
    }

    /// `KeyVal` / `Format` 行的值原文。
    pub fn value(&self) -> Option<&str> {
        match self.kind {
            EntryKind::KeyVal | EntryKind::Format => self.at.and_then(|h| self.raw.get(h..)),
            _ => None,
        }
    }

    /// KeyVal 行的值（去尾部空白，用于 key 比较/读取数值）。
    pub fn value_trimmed(&self) -> Option<&str> {
        self.value().map(str::trim)
    }

    /// 替换 `Key: value` 的值部分；key 与 `:` 后的空白写法保持原样。
    pub fn set_key_value(&mut self, key: &str, value: &str) -> Result<(), StkError> {
        if self.kind != EntryKind::KeyVal {
            return Err(Entry::err_bad_line(&self.raw));
        }
        match self.key() {
            Some(found) if found.eq_ignore_ascii_case(key.trim()) => {}
            _ => {
                return Err(StkError::simple(Domain::Parse, 37, "key 与该行不匹配")
                    .with_detail(key.to_owned())
                    .with_detail(self.raw.clone()));
            }
        }
        let h = self.at.ok_or_else(|| Entry::err_bad_line(&self.raw))?;
        let head = match self.raw.get(..h) {
            Some(head) => head.to_owned(),
            None => return Err(Entry::err_bad_line(&self.raw)),
        };
        self.raw = format!("{head}{value}");
        Ok(())
    }

    /// 第 `idx` 个字段的原文切片（不分配）。
    /// `tail = Some(k)` 表示字段表长度为 k，第 k-1 个字段吞掉剩余逗号（Events 的 Text）；
    /// `tail = None` 表示全部按逗号切分（Styles 的所有字段）。
    /// 逗号前后的空白算排版而不是值内容，因此读到的值无需再 trim；末字段（Text）保持全文。
    pub fn field_slice(&self, tail: Option<usize>, idx: usize) -> Option<&str> {
        if !self.kind.field_line() {
            return None;
        }
        let limit = tail.unwrap_or(usize::MAX);
        let mut pos = self.at?;
        let mut i = 0usize;
        while i < idx {
            // 已经在 Text 字段里还继续往后取 → 越界
            if i + 1 >= limit {
                return None;
            }
            let rel = self.raw.get(pos..)?.find(',')?;
            pos = skip_ws_from(&self.raw, pos + rel + 1);
            i += 1;
        }
        if i + 1 >= limit {
            return self.raw.get(pos..);
        }
        match self.raw.get(pos..).and_then(|rest| rest.find(',')) {
            Some(rel) => self.raw.get(pos..pos + rel).map(trim_end_ws),
            None => self.raw.get(pos..),
        }
    }

    /// 带原文分隔符的字段表；用于定点改写。
    pub fn fields(&self, tail: Option<usize>) -> Vec<Field> {
        let mut out = Vec::new();
        if !self.kind.field_line() {
            return out;
        }
        let h = match self.at {
            Some(h) => h,
            None => return out,
        };
        let rest = match self.raw.get(h..) {
            Some(r) => r,
            None => return out,
        };
        let limit = tail.unwrap_or(usize::MAX);
        let mut value = String::new();
        let mut idx = 0usize;
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == ',' && idx + 1 < limit {
                // 值末尾的空白连同逗号一起归入分隔符，改写值时不会被吞掉
                let trim_at = value.len() - ws_tail_len(&value);
                let sep: String = match value.get(trim_at..) {
                    Some(tail_ws) => format!("{tail_ws},"),
                    None => String::from(","),
                };
                if let Some(v) = value.get(..trim_at) {
                    out.push(Field {
                        value: v.to_owned(),
                        sep,
                    });
                }
                value.clear();
                while let Some(&next) = chars.peek() {
                    if next == ' ' || next == '\t' {
                        match out.last_mut() {
                            Some(last) => last.sep.push(next),
                            None => value.push(next),
                        }
                        chars.next();
                    } else {
                        break;
                    }
                }
                idx += 1;
            } else {
                value.push(c);
            }
        }
        out.push(Field {
            value,
            sep: String::new(),
        });
        out
    }

    /// 字段个数（不分配）。
    pub fn field_count(&self, tail: Option<usize>) -> usize {
        if !self.kind.field_line() {
            return 0;
        }
        let mut pos = match self.at {
            Some(h) => h,
            None => return 0,
        };
        let limit = tail.unwrap_or(usize::MAX);
        let mut n = 1usize;
        while n < limit {
            match self.raw.get(pos..).and_then(|rest| rest.find(',')) {
                Some(rel) => {
                    pos = skip_ws_from(&self.raw, pos + rel + 1);
                    n += 1;
                }
                None => break,
            }
        }
        n
    }

    /// 只替换第 `idx` 个字段的值：其余字段、逗号数量与字段间空白风格原样保留。
    pub fn set_field_at(
        &mut self,
        tail: Option<usize>,
        idx: usize,
        value: &str,
    ) -> Result<(), StkError> {
        let h = self.at.ok_or_else(|| Entry::err_bad_line(&self.raw))?;
        let mut fs = self.fields(tail);
        if idx >= fs.len() {
            return Err(Entry::err_bad_line(&self.raw)
                .with_detail(format!("字段下标 {idx} 超出 {}", fs.len())));
        }
        fs[idx].value = value.to_owned();
        let mut line = String::with_capacity(self.raw.len() + value.len() + 1);
        if let Some(head) = self.raw.get(..h) {
            line.push_str(head);
        }
        for f in &fs {
            line.push_str(&f.value);
            line.push_str(&f.sep);
        }
        self.raw = line;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub kind: SectionKind,
    pub variant: AssVariant,
    /// 段头原文（含 `[V4+ Styles]` 的大小写与空格写法）；隐式首段为 `None`。
    pub header: Option<String>,
    pub header_eol: Eol,
    /// 段头括号内的归一化文本（小写去空白），用于重复段头别名匹配与 loss 报告。
    pub header_key: String,
    /// 动态字段表：来自本段 `Format:` 行；缺失时按段头版本兜底。
    pub format: Vec<String>,
    pub format_source: FormatSource,
    pub entries: Vec<Entry>,
}

impl Section {
    fn implicit() -> Section {
        Section {
            kind: SectionKind::Unknown,
            variant: AssVariant::Unknown,
            header: None,
            header_eol: Eol::Eof,
            header_key: String::new(),
            format: Vec::new(),
            format_source: FormatSource::None,
            entries: Vec::new(),
        }
    }

    pub fn render(&self, out: &mut String) {
        if let Some(h) = &self.header {
            out.push_str(h);
            out.push_str(self.header_eol.str());
        }
        for e in &self.entries {
            e.render(out);
        }
    }

    /// 事件行的字段数上限（Text 之前的逗号个数由它决定）。
    pub fn event_tail(&self) -> Option<usize> {
        if self.format.is_empty() {
            None
        } else {
            Some(self.format.len())
        }
    }

    /// 某条目的 tail 取值。
    pub fn tail_of(&self, kind: EntryKind) -> Option<usize> {
        match kind {
            EntryKind::Style => None,
            _ => self.event_tail(),
        }
    }

    /// 按字段名取下标；大小写不敏感（`Fontsize`/`FontSize` 等价）。
    pub fn index_of(&self, name: &str) -> Option<usize> {
        let want = name.trim().to_lowercase();
        self.format.iter().position(|f| f.to_lowercase() == want)
    }

    pub fn entry(&self, i: usize) -> Option<&Entry> {
        self.entries.get(i)
    }

    pub fn entry_mut(&mut self, i: usize) -> Option<&mut Entry> {
        self.entries.get_mut(i)
    }

    /// 按字段名改写某条目：只替换目标字段，其余原样。
    /// `KeyVal` 行（Script Info 的 PlayResX/Y 等）按 key 匹配，替换冒号后的值并保留 `Key: ` 写法。
    pub fn set_field(&mut self, i: usize, name: &str, value: &str) -> Result<(), StkError> {
        enum Target {
            KeyValue,
            Field(Option<usize>, usize),
        }
        let missing = |this: &Section| {
            StkError::simple(Domain::Parse, 33, "字段不在该段的 Format 列表中")
                .with_detail(name.to_owned())
                .with_detail(this.header.clone().unwrap_or_default())
        };
        let target = match self.entries.get(i) {
            None => {
                return Err(StkError::simple(Domain::Parse, 35, "条目下标不存在")
                    .with_detail(i.to_string()))
            }
            Some(e) if e.kind == EntryKind::KeyVal => Target::KeyValue,
            Some(e) if e.kind.field_line() => match self.index_of(name) {
                Some(idx) => Target::Field(self.tail_of(e.kind), idx),
                None => return Err(missing(self)),
            },
            Some(e) => {
                return Err(
                    StkError::simple(Domain::Parse, 34, "该条目不是可改写字段行")
                        .with_detail(e.raw.clone()),
                );
            }
        };
        let entry = match self.entries.get_mut(i) {
            Some(e) => e,
            None => {
                return Err(StkError::simple(Domain::Parse, 35, "条目下标不存在")
                    .with_detail(i.to_string()))
            }
        };
        match target {
            Target::KeyValue => entry.set_key_value(name, value),
            Target::Field(tail, idx) => entry.set_field_at(tail, idx, value),
        }
    }

    /// 字段表是否来自真正的 `Format:` 行。
    pub fn format_source_is_line(&self) -> bool {
        self.format_source == FormatSource::FormatLine
    }

    /// 该段的 Style 条目下标。
    pub fn style_entries(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == EntryKind::Style)
            .map(|(i, _)| i)
            .collect()
    }

    /// 该段的 Dialogue 条目下标（不含 Comment）。
    pub fn dialogue_entries(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == EntryKind::Dialogue)
            .map(|(i, _)| i)
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssDocument {
    pub bom: bool,
    /// 输入中占多数的换行符；供后续追加条目使用，已有条目各自记住自己的结束符。
    pub newline: String,
    pub sections: Vec<Section>,
}

/// 段头方括号内的文本；不是段头时返回 `None`。
fn header_inner(line: &str) -> Option<&str> {
    if !line.starts_with('[') {
        return None;
    }
    let body = line.get(1..)?.trim();
    Some(body.trim_end_matches(']').trim())
}

fn variant_of(inner: &str) -> AssVariant {
    let lc = inner.to_lowercase();
    if lc.contains("v4++") || lc.contains("v4 ++") {
        AssVariant::V4PlusPlus
    } else if lc.contains("v4+") || lc.contains("v4 +") {
        AssVariant::V4Plus
    } else if lc.contains("v4") {
        AssVariant::V4
    } else {
        AssVariant::Unknown
    }
}

fn kind_of(inner: &str) -> SectionKind {
    let lc = inner.to_lowercase();
    if lc.contains("style") {
        SectionKind::Styles
    } else if lc.contains("event") {
        SectionKind::Events
    } else if lc.contains("font") {
        SectionKind::Fonts
    } else if lc.contains("filelist") {
        SectionKind::FileList
    } else if lc.contains("script") || lc.contains("info") {
        SectionKind::ScriptInfo
    } else {
        SectionKind::Unknown
    }
}

/// 缺 Format 行时按段头自称的版本取默认字段表。
fn default_format(header_key: &str, kind: SectionKind) -> (Vec<String>, FormatSource) {
    if kind == SectionKind::Styles {
        for (needle, names) in HEADER_FIELD_ALIASES {
            if needle.ends_with("styles") && header_key.contains(needle) {
                return (
                    names.iter().map(|s| (*s).to_owned()).collect(),
                    FormatSource::RepeatedHeader,
                );
            }
        }
        return (
            STYLE_FIELDS_V4PLUS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            FormatSource::Fallback,
        );
    }
    if kind == SectionKind::Events {
        for (needle, names) in HEADER_FIELD_ALIASES {
            if needle.ends_with("events") && header_key.contains(needle) {
                return (
                    names.iter().map(|s| (*s).to_owned()).collect(),
                    FormatSource::RepeatedHeader,
                );
            }
        }
        return (
            EVENT_FIELDS.iter().map(|s| (*s).to_owned()).collect(),
            FormatSource::Fallback,
        );
    }
    (Vec::new(), FormatSource::None)
}

/// `Format: Name, Fontname, ...` 的名字列表；`Format: Style,Name,...` 的前导 token
/// 与 key 重复，不是字段，需剥掉才能对齐后面的值。
fn parse_format_names(raw: &str) -> Vec<String> {
    let mut names: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    if names.len() > 1 {
        let first_is_key = matches!(
            names.first().map(|s| s.to_lowercase()).as_deref(),
            Some("style") | Some("dialogue") | Some("comment")
        );
        if first_is_key {
            names.remove(0);
        }
    }
    names
}

fn apply_format(section: &mut Section, line: &str) {
    let raw = match head_len(line).and_then(|h| line.get(h..)) {
        Some(r) => r,
        None => return,
    };
    let names = parse_format_names(raw);
    if !names.is_empty() {
        section.format = names;
        section.format_source = FormatSource::FormatLine;
    }
}

/// 判定行类型。Aegisub 省略字段表的两种写法（重复段头、`0:00:01.00,...`）在这里降级处理，
/// 而不是把整行当垃圾。
fn classify(
    line: &str,
    section: &mut Section,
    last_kind: Option<EntryKind>,
) -> (EntryKind, Option<usize>) {
    let at = head_len(line);
    if line.trim().is_empty() {
        return (EntryKind::Blank, None);
    }
    let first = line.as_bytes().first().copied();
    if first == Some(b';') || first == Some(b'#') {
        return (EntryKind::Remark, None);
    }
    if section.kind.is_attachment() {
        // 附件区没有 Key: value 语义，整行按原文保留（含带冒号的 base64 行）
        return (EntryKind::Raw, None);
    }
    let colon = match line.find(':') {
        Some(c) => c,
        None => return (EntryKind::Opaque, None),
    };
    let key = line.get(..colon).map(str::trim).unwrap_or("");
    let lc = key.to_lowercase();
    if lc.is_empty() {
        return (EntryKind::Opaque, None);
    }
    match section.kind {
        SectionKind::Styles => match lc.as_str() {
            "style" => (EntryKind::Style, at),
            "format" => {
                apply_format(section, line);
                (EntryKind::Format, at)
            }
            _ => (EntryKind::KeyVal, at),
        },
        SectionKind::Events => match lc.as_str() {
            "dialogue" => (EntryKind::Dialogue, at),
            "comment" => (EntryKind::Comment, at),
            "format" => {
                apply_format(section, line);
                (EntryKind::Format, at)
            }
            // 数字开头 = 省略了 Marked=/Layer=，整行都是字段表
            _ if lc.chars().next().is_some_and(|c| c.is_ascii_digit())
                && matches!(last_kind, Some(prev) if prev.event_like()) =>
            {
                (EntryKind::Dialogue, Some(0))
            }
            // Aegisub 8b/10b 事故：样式字段表落在 Events 段里，样式行也写在 Events 段
            _ if !section.format_source_is_line()
                && is_default_field_name(&lc, &section.format) =>
            {
                (EntryKind::Style, at)
            }
            _ => (EntryKind::KeyVal, at),
        },
        _ => (EntryKind::KeyVal, at),
    }
}

/// 字段表还没被真正的 `Format:` 行确认时，用兜底表判断 key 是否是字段名。
fn is_default_field_name(lc: &str, names: &[String]) -> bool {
    names.iter().any(|n| n.to_lowercase() == lc)
}

fn is_layout_ws(c: u8) -> bool {
    c == 0x20 || c == 0x09
}

/// 值末尾的空白长度（字节数，只可能是空格/制表符）。
fn ws_tail_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = b.len();
    while i > 0 && is_layout_ws(b[i - 1]) {
        i -= 1;
    }
    b.len() - i
}

fn trim_end_ws(s: &str) -> &str {
    let cut = s.len() - ws_tail_len(s);
    s.get(..cut).unwrap_or(s)
}

fn dominant_newline(src: &str) -> String {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut cr = 0usize;
    for (_, eol) in split_lines(src) {
        match eol {
            Eol::CrLf => crlf += 1,
            Eol::Lf => lf += 1,
            Eol::Cr => cr += 1,
            Eol::Eof => {}
        }
    }
    if crlf > lf && crlf > cr {
        "\r\n".to_owned()
    } else if cr > lf {
        "\r".to_owned()
    } else {
        "\n".to_owned()
    }
}

impl AssDocument {
    /// 解析。任何无法理解的输入都降级为原文条目，因此本函数永不失败。
    pub fn parse(src: &str) -> AssDocument {
        let (bom, body) = match src.strip_prefix(BOM) {
            Some(rest) => (true, rest),
            None => (false, src),
        };
        let mut doc = AssDocument {
            bom,
            newline: dominant_newline(body),
            sections: Vec::new(),
        };
        let mut cur = Section::implicit();
        let mut last_kind: Option<EntryKind> = None;
        for (line, eol) in split_lines(body) {
            if line.starts_with('[') {
                let done = std::mem::replace(&mut cur, Section::implicit());
                let inner = header_inner(line).unwrap_or("").to_owned();
                let key = inner.trim().to_lowercase();
                let kind = kind_of(&key);
                let (format, format_source) = default_format(&key, kind);
                cur.kind = kind;
                cur.variant = variant_of(&inner);
                cur.header = Some(line.to_owned());
                cur.header_eol = eol;
                cur.header_key = key;
                cur.format = format;
                cur.format_source = format_source;
                if done.header.is_some() || !done.entries.is_empty() {
                    doc.sections.push(done);
                }
                last_kind = None;
            } else {
                let (kind, at) = classify(line, &mut cur, last_kind);
                if kind.field_line() {
                    last_kind = Some(kind);
                }
                cur.entries.push(Entry {
                    kind,
                    raw: line.to_owned(),
                    eol,
                    at,
                });
            }
        }
        if cur.header.is_some() || !cur.entries.is_empty() {
            doc.sections.push(cur);
        }
        doc
    }

    /// 与输入逐字节相等（含 BOM、混合换行、缺失的末尾换行）。
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(estimate(self));
        if self.bom {
            out.push(BOM);
        }
        for s in &self.sections {
            s.render(&mut out);
        }
        out
    }

    pub fn section(&self, i: usize) -> Option<&Section> {
        self.sections.get(i)
    }

    pub fn section_mut(&mut self, i: usize) -> Option<&mut Section> {
        self.sections.get_mut(i)
    }

    pub fn sections_of(&self, kind: SectionKind) -> Vec<usize> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == kind)
            .map(|(i, _)| i)
            .collect()
    }

    /// Script Info 里某个 key 的最后一次出现值（ASS 的语义是后者生效）。
    pub fn script_info(&self, key: &str) -> Option<&str> {
        let want = key.trim().to_lowercase();
        for s in &self.sections {
            if s.kind != SectionKind::ScriptInfo {
                continue;
            }
            let mut hit = None;
            for e in &s.entries {
                if e.kind == EntryKind::KeyVal
                    && e.key().map(str::to_lowercase).as_deref() == Some(want.as_str())
                {
                    hit = e.value_trimmed();
                }
            }
            if hit.is_some() {
                return hit;
            }
        }
        None
    }

    pub fn play_res(&self) -> Option<(i64, i64)> {
        let x = self.script_info("PlayResX")?.parse::<i64>().ok()?;
        let y = self.script_info("PlayResY")?.parse::<i64>().ok()?;
        Some((x, y))
    }

    pub fn dialogue_count(&self) -> usize {
        self.dialogues().len()
    }

    /// 按起始时间排序后的下标序列，下标指向 `dialogues()` 的返回顺序。
    /// 排序只提供一个读取视角，绝不改动 `sections`/`entries` 的原文顺序
    /// （08 与旧审计都要求事件排序在写出阶段处理）。
    pub fn sorted_index_order(&self) -> Vec<usize> {
        let rows = self.sorted_dialogue_positions();
        rows.into_iter().map(|(_, _, i)| i).collect()
    }

    /// 同上，但返回 `(段下标, 条目下标, 展平序号)`，供写出阶段定位原文行。
    pub fn sorted_dialogue_positions(&self) -> Vec<(usize, usize, usize)> {
        let mut rows: Vec<(usize, usize, usize, Option<core_model::time::Time>)> = Vec::new();
        let mut flat = 0usize;
        for (si, s) in self.sections.iter().enumerate() {
            if s.kind != SectionKind::Events {
                continue;
            }
            for ei in s.dialogue_entries() {
                let start = s.dialogue_view(ei).and_then(|v| v.start().ok());
                rows.push((si, ei, flat, start));
                flat += 1;
            }
        }
        // sort_by 稳定：时间读不出来的条目保持原相对顺序并排在最后
        rows.sort_by(|a, b| match (a.3, b.3) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => core::cmp::Ordering::Less,
            (None, Some(_)) => core::cmp::Ordering::Greater,
            (None, None) => core::cmp::Ordering::Equal,
        });
        rows.into_iter()
            .map(|(si, ei, flat, _)| (si, ei, flat))
            .collect()
    }
}

/// 写出即 `render()`；实现 Display 后 `doc.to_string()` 就是字节还原路径。
impl core::fmt::Display for AssDocument {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

fn estimate(doc: &AssDocument) -> usize {
    doc.sections
        .iter()
        .map(|s| {
            let head = s.header.as_ref().map(String::len).unwrap_or(0) + s.header_eol.byte_len();
            head + s
                .entries
                .iter()
                .map(|e| e.raw.len() + e.eol.byte_len())
                .sum::<usize>()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_and_sections_are_detected() {
        let doc = AssDocument::parse("\u{feff}[Script Info]\r\nTitle: x\r\n[V4+ Styles]\r\n");
        assert!(doc.bom);
        assert_eq!(doc.newline, "\r\n");
        assert_eq!(doc.sections.len(), 2);
        assert_eq!(
            doc.section(0).and_then(|s| s.header.as_deref()),
            Some("[Script Info]")
        );
        assert_eq!(doc.sections_of(SectionKind::Styles), vec![1]);
        assert_eq!(doc.script_info("title"), Some("x"));
        assert_eq!(
            doc.to_string(),
            "\u{feff}[Script Info]\r\nTitle: x\r\n[V4+ Styles]\r\n"
        );
    }

    #[test]
    fn header_inner_handles_variants() {
        assert_eq!(header_inner("[V4+ Styles]"), Some("V4+ Styles"));
        assert_eq!(header_inner("[Script Info ]"), Some("Script Info"));
        assert_eq!(header_inner("[unclosed"), Some("unclosed"));
        assert_eq!(header_inner("Style: a,b"), None);
    }

    #[test]
    fn section_kind_and_variant_matching() {
        assert_eq!(kind_of("v4++ styles"), SectionKind::Styles);
        assert_eq!(kind_of("document events"), SectionKind::Events);
        assert_eq!(kind_of("v4 fonts"), SectionKind::Fonts);
        assert_eq!(kind_of("filelist"), SectionKind::FileList);
        assert_eq!(kind_of("script info"), SectionKind::ScriptInfo);
        assert_eq!(kind_of("timecodes v1 format"), SectionKind::Unknown);
        assert_eq!(variant_of("V4+ Styles"), AssVariant::V4Plus);
        assert_eq!(variant_of("V4++ Styles"), AssVariant::V4PlusPlus);
        assert_eq!(variant_of("V4 Styles"), AssVariant::V4);
    }

    #[test]
    fn format_line_strips_leading_style_token() {
        let names = parse_format_names("Style,Name,Fontname,Fontsize");
        assert_eq!(
            names,
            vec![
                "Name".to_owned(),
                "Fontname".to_owned(),
                "Fontsize".to_owned()
            ]
        );
        let plain = parse_format_names(" Name, Fontname , Fontsize ,");
        assert_eq!(
            plain,
            vec![
                "Name".to_owned(),
                "Fontname".to_owned(),
                "Fontsize".to_owned()
            ]
        );
    }

    #[test]
    fn default_format_by_header_alias() {
        let (v4pp, src) = default_format("v4++ styles", SectionKind::Styles);
        assert_eq!(src, FormatSource::RepeatedHeader);
        assert_eq!(v4pp.len(), 23);
        assert_eq!(v4pp.last().map(String::as_str), Some("Encoding"));
        let (v4, _) = default_format("v4 styles", SectionKind::Styles);
        assert_eq!(v4.len(), 17);
        let (ev, src) = default_format("events", SectionKind::Events);
        assert_eq!(src, FormatSource::RepeatedHeader);
        assert_eq!(ev.len(), 10);
        assert_eq!(ev.last().map(String::as_str), Some("Text"));
        let (none, s) = default_format("anm data", SectionKind::Unknown);
        assert!(none.is_empty());
        assert_eq!(s, FormatSource::None);
    }

    #[test]
    fn field_slice_and_count_agree_with_fields() {
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,a,b,\"c\"";
        let e = Entry {
            kind: EntryKind::Dialogue,
            at: head_len(line),
            raw: line.to_owned(),
            eol: Eol::Lf,
        };
        let fs = e.fields(Some(10));
        assert_eq!(fs.len(), 10);
        assert_eq!(e.field_count(Some(10)), 10);
        for (i, f) in fs.iter().enumerate() {
            assert_eq!(e.field_slice(Some(10), i), Some(f.value.as_str()));
        }
        assert_eq!(e.field_slice(Some(10), 9), Some("a,b,\"c\""));
        assert_eq!(e.field_slice(Some(10), 1), Some("0:00:01.00"));
        assert_eq!(e.field_slice(Some(10), 10), None);
        assert_eq!(e.field_slice(Some(10), 11), None);
    }

    #[test]
    fn style_fields_split_on_every_comma() {
        let raw = "Style: Default,Arial,20,&H00FFFFFF".to_owned();
        let e = Entry {
            kind: EntryKind::Style,
            at: head_len(&raw),
            raw,
            eol: Eol::CrLf,
        };
        assert_eq!(e.field_count(None), 4);
        assert_eq!(e.field_slice(None, 2), Some("20"));
        assert_eq!(
            e.fields(None)
                .iter()
                .map(|f| f.value.clone())
                .collect::<Vec<_>>(),
            vec!["Default", "Arial", "20", "&H00FFFFFF"]
        );
    }

    #[test]
    fn set_field_keeps_separators_and_comma_count() {
        let raw = "Style: Default, Arial , 20,&H00FFFFFF".to_owned();
        let mut e = Entry {
            kind: EntryKind::Style,
            at: head_len(&raw),
            raw,
            eol: Eol::Lf,
        };
        assert!(e.set_field_at(None, 2, "40").is_ok());
        assert_eq!(e.raw, "Style: Default, Arial , 40,&H00FFFFFF");
        let raw = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,x, y".to_owned();
        let mut d = Entry {
            kind: EntryKind::Dialogue,
            at: head_len(&raw),
            raw,
            eol: Eol::Lf,
        };
        assert!(d.set_field_at(Some(10), 9, "z, w").is_ok());
        assert_eq!(
            d.raw,
            "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,z, w"
        );
        assert!(d.set_field_at(Some(10), 12, "nope").is_err());
    }

    #[test]
    fn unterminated_line_renders_without_added_newline() {
        let doc = AssDocument::parse("[Events]\nDialogue: 0");
        assert_eq!(doc.to_string(), "[Events]\nDialogue: 0");
    }
}
