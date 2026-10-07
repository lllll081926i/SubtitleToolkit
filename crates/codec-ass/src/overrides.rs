//! ASS override block 分词器（给后续 HDR / 分辨率 transform 用，不做渲染）。
//!
//! 保真策略：每个 tag 保存自己的原文片段（`lead`/`name`/`args`，`\t` 另有 `head`/`tail`），
//! `render()` 只做拼接，所以未被改动的 tag 必然逐字节还原；未识别 tag 归入
//! `TagKind::Unknown`，同样原样保留（AT-ASS-001「未知 tag 不丢」）。
//! 括号内的逗号与空格属于参数本身（`\pos(320, 480)`、`\clip(\p1 m 0 0 l 9 9\p0)`），
//! 因此参数扫描按括号深度而不是按空白切分。

use core_model::color::AssColor;
use core_model::error::StkError;

/// 已知 tag 表：`(小写名, 原型, 粘连规则)`；匹配时取最长可接受前缀。
const KNOWN_TAGS: &[(&str, Proto, Glue)] = &[
    ("iclip", Proto::Clip(true), Glue::None),
    ("clip", Proto::Clip(false), Glue::None),
    ("alpha", Proto::Alpha(1), Glue::None),
    ("xbord", Proto::Border, Glue::None),
    ("ybord", Proto::Border, Glue::None),
    ("bord", Proto::Border, Glue::None),
    ("xshad", Proto::Shadow, Glue::None),
    ("yshad", Proto::Shadow, Glue::None),
    ("shad", Proto::Shadow, Glue::None),
    ("fscx", Proto::Scale, Glue::None),
    ("fscy", Proto::Scale, Glue::None),
    ("fspd", Proto::Other, Glue::None),
    ("blur", Proto::Other, Glue::None),
    ("fade", Proto::Other, Glue::None),
    ("fade0", Proto::Other, Glue::None),
    ("fade1", Proto::Other, Glue::None),
    ("frz", Proto::Rotation, Glue::None),
    ("frx", Proto::Rotation, Glue::None),
    ("fry", Proto::Rotation, Glue::None),
    ("move", Proto::Movement, Glue::None),
    ("org", Proto::Origin, Glue::None),
    ("pos", Proto::Position, Glue::None),
    ("fad", Proto::Other, Glue::None),
    ("fax", Proto::Other, Glue::None),
    ("fay", Proto::Other, Glue::None),
    ("ang", Proto::Other, Glue::None),
    ("fsp", Proto::Other, Glue::None),
    ("kt", Proto::Karaoke, Glue::None),
    ("kf", Proto::Karaoke, Glue::None),
    ("ko", Proto::Karaoke, Glue::None),
    ("1c", Proto::Colour(1), Glue::ColourValue),
    ("2c", Proto::Colour(2), Glue::ColourValue),
    ("3c", Proto::Colour(3), Glue::ColourValue),
    ("4c", Proto::Colour(4), Glue::ColourValue),
    ("1a", Proto::Alpha(1), Glue::ColourValue),
    ("2a", Proto::Alpha(2), Glue::ColourValue),
    ("3a", Proto::Alpha(3), Glue::ColourValue),
    ("4a", Proto::Alpha(4), Glue::ColourValue),
    ("fr", Proto::Rotation, Glue::None),
    ("be", Proto::Other, Glue::None),
    ("fn", Proto::Font, Glue::Letters),
    ("fs", Proto::Other, Glue::None),
    ("fe", Proto::Other, Glue::None),
    ("an", Proto::Other, Glue::None),
    ("sp", Proto::Other, Glue::None),
    ("q", Proto::Other, Glue::None),
    ("h", Proto::Other, Glue::None),
    ("p", Proto::Drawing, Glue::None),
    ("t", Proto::Anim, Glue::None),
    ("c", Proto::Colour(1), Glue::ColourValue),
    ("a", Proto::Alpha(1), Glue::ColourValue),
    ("k", Proto::Karaoke, Glue::None),
    ("b", Proto::Other, Glue::None),
    ("i", Proto::Other, Glue::None),
    ("u", Proto::Other, Glue::None),
    ("s", Proto::Other, Glue::None),
    ("r", Proto::Other, Glue::Letters),
    ("v", Proto::Other, Glue::None),
];

/// 名字后紧跟字母时是否仍算匹配。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glue {
    /// 后跟字母即视为另一个 tag 名（`\anm` 不是 `\an`）
    None,
    /// 参数是字体/样式名，可以紧贴（`\fnArial`、`\rDefault`）
    Letters,
    /// 参数是十六进制颜色，可以紧贴（`\cFF0000`）；否则 `\anm` 会被读成 `\a`+`nm`
    ColourValue,
}

fn glue_ok(glue: Glue, rest: &str) -> bool {
    match rest.chars().next() {
        None => true,
        Some(c) => match glue {
            Glue::Letters => true,
            Glue::ColourValue => c.is_ascii_hexdigit() || c == '&',
            Glue::None => !c.is_ascii_alphabetic(),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Proto {
    Colour(u8),
    Alpha(u8),
    Position,
    Movement,
    Origin,
    Clip(bool),
    Drawing,
    Anim,
    Border,
    Shadow,
    Scale,
    Rotation,
    Font,
    Karaoke,
    Other,
    UnknownTag,
}

fn proto_kind(proto: Proto) -> TagKind {
    match proto {
        Proto::Colour(slot) => TagKind::Colour { slot },
        Proto::Alpha(slot) => TagKind::Alpha { slot },
        Proto::Position => TagKind::Position,
        Proto::Movement => TagKind::Movement,
        Proto::Origin => TagKind::Origin,
        Proto::Clip(inverted) => TagKind::Clip { inverted },
        Proto::Drawing => TagKind::DrawingMode,
        Proto::Anim => TagKind::Animation {
            head: String::new(),
            nested: Vec::new(),
            tail: String::new(),
        },
        Proto::Border => TagKind::Border,
        Proto::Shadow => TagKind::Shadow,
        Proto::Scale => TagKind::Scale,
        Proto::Rotation => TagKind::Rotation,
        Proto::Font => TagKind::Font,
        Proto::Karaoke => TagKind::Karaoke,
        Proto::Other => TagKind::Other,
        Proto::UnknownTag => TagKind::Unknown,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagKind {
    /// `\c` 与 `\1c..\4c`
    Colour {
        slot: u8,
    },
    /// `\a`/`\alpha` 与 `\1a..\4a`
    Alpha {
        slot: u8,
    },
    Position,
    Movement,
    Origin,
    Clip {
        inverted: bool,
    },
    /// `\p`：drawing 模式开关
    DrawingMode,
    /// `\t`：内层 override 递归 tokenize
    Animation {
        head: String,
        nested: Vec<Tag>,
        tail: String,
    },
    Border,
    Shadow,
    Scale,
    Rotation,
    Font,
    Karaoke,
    /// 已识别但本 crate 不需要结构语义
    Other,
    /// 未识别（含 VSFilterMod 私有 tag）：原文保留
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// tag 前的空白/杂散原文，属于 block 内部字节流的一部分
    pub lead: String,
    /// 含反斜杠的原文名，如 `\pos`、`\1c`
    pub name: String,
    /// 原文参数（含括号与内部空格）；`Animation` 时为空，内容在 head/nested/tail
    pub args: String,
    pub kind: TagKind,
}

impl Tag {
    /// 参数原文（`\t` 取 head+nested+tail 拼回）。
    pub fn args_full(&self) -> String {
        let mut out = String::new();
        match &self.kind {
            TagKind::Animation { head, nested, tail } => {
                out.push_str(head);
                for t in nested {
                    t.render(&mut out);
                }
                out.push_str(tail);
            }
            _ => out.push_str(&self.args),
        }
        out
    }

    /// 拼接出该 tag 的原文（不含 `lead`）。
    pub fn render(&self, out: &mut String) {
        out.push_str(&self.name);
        out.push_str(&self.args_full());
    }

    pub fn render_full(&self, out: &mut String) {
        out.push_str(&self.lead);
        self.render(out);
    }

    /// 去掉外层括号与空白后的参数文本。
    pub fn body(&self) -> String {
        let raw = self.args_full();
        let trimmed = raw.trim();
        let start = trimmed.find('(').map(|i| i + 1).unwrap_or(0);
        let inner = trimmed.get(start..).unwrap_or("");
        let end = inner.rfind(')').unwrap_or(inner.len());
        inner.get(..end).unwrap_or(inner).to_owned()
    }

    /// `\pos(320, 480)` → `["320", "480"]`（原文数字写法，不做数值转换）。
    pub fn coords(&self) -> Vec<String> {
        self.body()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    pub fn plain_name(&self) -> &str {
        self.name.get(1..).unwrap_or(&self.name)
    }

    pub fn is_unknown(&self) -> bool {
        self.kind == TagKind::Unknown
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideBlock {
    /// 原文是否有 `{`
    pub open: bool,
    pub tags: Vec<Tag>,
    /// 最后一个 tag 之后到 `}` 之前的原文（通常是空白）
    pub tail: String,
    /// 原文是否有闭合 `}`
    pub closed: bool,
}

impl OverrideBlock {
    pub fn render(&self, out: &mut String) {
        if self.open {
            out.push('{');
        }
        for t in &self.tags {
            t.render_full(out);
        }
        out.push_str(&self.tail);
        if self.closed {
            out.push('}');
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverrideToken {
    /// `{...}` 之外的普通文本
    Text {
        raw: String,
    },
    /// `\p1` 与 `\p0` 之间的矢量绘制文本：空格不是分隔符
    Drawing {
        raw: String,
    },
    Block(OverrideBlock),
}

impl OverrideToken {
    pub fn render(&self, out: &mut String) {
        match self {
            OverrideToken::Text { raw } | OverrideToken::Drawing { raw } => out.push_str(raw),
            OverrideToken::Block(b) => b.render(out),
        }
    }

    pub fn raw(&self) -> String {
        let mut s = String::new();
        self.render(&mut s);
        s
    }
}

impl core::fmt::Display for OverrideToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.raw())
    }
}

impl core::fmt::Display for Tag {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut s = String::new();
        self.render(&mut s);
        f.write_str(&s)
    }
}

pub fn tokens_to_string(tokens: &[OverrideToken]) -> String {
    let mut out = String::new();
    for t in tokens {
        t.render(&mut out);
    }
    out
}

/// tokenize 一段 ASS 文本；永不失败也不 panic（23 §ASS：parser fuzz 友好）。
pub fn parse_overrides(text: &str) -> Vec<OverrideToken> {
    let mut out: Vec<OverrideToken> = Vec::new();
    let mut pos = 0usize;
    let mut drawing = false;
    while pos < text.len() {
        if text.get(pos..pos + 1) == Some("{") {
            let inner_start = pos + 1;
            let (inner, next, closed) = match text.get(inner_start..).and_then(|r| r.find('}')) {
                Some(rel) => {
                    let close = inner_start + rel;
                    (text.get(inner_start..close).unwrap_or(""), close + 1, true)
                }
                None => (text.get(inner_start..).unwrap_or(""), text.len(), false),
            };
            let (tags, tail) = parse_tag_list(inner);
            if let Some(state) = last_drawing(&tags) {
                drawing = state;
            }
            out.push(OverrideToken::Block(OverrideBlock {
                open: true,
                tags,
                tail,
                closed,
            }));
            pos = next;
        } else {
            let next = text
                .get(pos..)
                .and_then(|r| r.find('{'))
                .map(|i| pos + i)
                .unwrap_or(text.len());
            let raw = text.get(pos..next).unwrap_or("");
            if !raw.is_empty() {
                let token = if drawing {
                    OverrideToken::Drawing {
                        raw: raw.to_owned(),
                    }
                } else {
                    OverrideToken::Text {
                        raw: raw.to_owned(),
                    }
                };
                out.push(token);
            }
            pos = next;
        }
    }
    out
}

/// block 内部的 tag 序列：以 `\` 为边界，其余字节属于前一个 tag 的参数或 block tail。
fn parse_tag_list(src: &str) -> (Vec<Tag>, String) {
    let mut tags: Vec<Tag> = Vec::new();
    let mut cursor = 0usize;
    let mut lead_start = 0usize;
    while let Some(rel) = src.get(cursor..).and_then(|r| r.find('\\')) {
        let slash = cursor + rel;
        let run_end = scan_name(src, slash + 1);
        let candidate = src.get(slash + 1..run_end).unwrap_or("");
        let (name_len, kind) = match_known(candidate);
        let name_end = slash + 1 + name_len;
        let args_end = scan_args(src, name_end);
        let name = src.get(slash..name_end).unwrap_or("").to_owned();
        let args = src.get(name_end..args_end).unwrap_or("").to_owned();
        let kind = match kind {
            Proto::Anim => {
                let (head, nested, tail) = split_animation(&args);
                TagKind::Animation { head, nested, tail }
            }
            other => proto_kind(other),
        };
        let lead = src.get(lead_start..slash).unwrap_or("").to_owned();
        tags.push(Tag {
            lead,
            name,
            args,
            kind,
        });
        cursor = args_end.max(slash + 1);
        lead_start = cursor;
    }
    (tags, src.get(lead_start..).unwrap_or("").to_owned())
}

fn scan_name(src: &str, from: usize) -> usize {
    let b = src.as_bytes();
    let mut i = from;
    while i < b.len() && b[i].is_ascii_alphanumeric() {
        i += 1;
    }
    i
}

/// 参数扫描：`(` 开头按括号深度吞掉整组（组内空格与逗号都不是分隔符）；否则吃到下一个 `\`。
/// 未闭合的括号吞到 block 文本末尾，绝不 panic。
fn scan_args(src: &str, from: usize) -> usize {
    let b = src.as_bytes();
    let mut i = from;
    while i + 1 < b.len() && (b[i] == b' ' || b[i] == b'\t') && b[i + 1] == b'(' {
        i += 1;
    }
    if b.get(i) == Some(&b'(') {
        let mut depth = 0i32;
        while i < b.len() {
            match b[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        return i;
    }
    while i < b.len() && b[i] != b'\\' {
        i += 1;
    }
    i
}

/// 取最长可接受前缀；都不匹配则整段 alnum 作为未知 tag 名。
fn match_known(candidate: &str) -> (usize, Proto) {
    let lower = candidate.to_lowercase();
    let mut best: Option<(usize, Proto)> = None;
    for (tag, proto, glue) in KNOWN_TAGS {
        if !lower.starts_with(tag) {
            continue;
        }
        let rest = lower.get(tag.len()..).unwrap_or("");
        if !glue_ok(*glue, rest) {
            continue;
        }
        if best.map(|(done, _)| tag.len() > done).unwrap_or(true) {
            best = Some((tag.len(), *proto));
        }
    }
    match best {
        Some((len, proto)) => (len, proto),
        None => (candidate.chars().count(), Proto::UnknownTag),
    }
}

/// `\t(0,1000,\c&HFF&H)` → head=`"(0,1000,"`、nested、tail=`")"`；拼回必定等于原文。
fn split_animation(args: &str) -> (String, Vec<Tag>, String) {
    let passthrough = (args.to_owned(), Vec::new(), String::new());
    let open = match args.find('(') {
        Some(i) => i,
        None => return passthrough,
    };
    let ws = args.get(..open).unwrap_or("");
    let after = args.get(open + 1..).unwrap_or("");
    let bytes = after.as_bytes();
    let mut depth = 1i32;
    let mut close: Option<usize> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let close = match close {
        Some(c) => c,
        None => return passthrough,
    };
    let content = after.get(..close).unwrap_or("");
    let tail = format!("){}", after.get(close + 1..).unwrap_or(""));
    let first_tag = match content.find('\\') {
        Some(k) => k,
        None => return passthrough,
    };
    match content.get(..first_tag).and_then(|head| head.rfind(',')) {
        Some(c) => {
            let head = format!("{ws}({}", content.get(..=c).unwrap_or(""));
            let nested = parse_tag_list(content.get(c + 1..).unwrap_or("")).0;
            (head, nested, tail)
        }
        None => (format!("{ws}("), parse_tag_list(content).0, tail),
    }
}

/// `\p` 的最后一次取值决定其后文本是不是 drawing。
fn last_drawing(tags: &[Tag]) -> Option<bool> {
    let mut state = None;
    for t in tags {
        if t.kind == TagKind::DrawingMode {
            state = Some(t.args.trim().parse::<i64>().unwrap_or(0) != 0);
        }
    }
    state
}

struct ColourLiteral {
    end: usize,
    prefix: String,
    digits: String,
    suffix: String,
    colour: AssColor,
}

/// 在 args 原文中定位一个颜色字面量：`&H` 前缀可选，6/8 位十六进制，`&`/`&H` 后缀可选。
fn try_colour(src: &str, from: usize) -> Option<ColourLiteral> {
    let bytes = src.as_bytes();
    let mut offset = 0usize;
    let mut prefix = String::new();
    if matches!(bytes.get(from..from + 2), Some([b'&', b'H' | b'h'])) {
        prefix = src.get(from..from + 2)?.to_owned();
        offset = 2;
    } else if bytes
        .get(from)
        .copied()
        .is_some_and(|c| c.is_ascii_hexdigit())
        && bytes
            .get(from.wrapping_sub(1))
            .copied()
            .is_some_and(|c| c.is_ascii_hexdigit())
    {
        // 裸十六进制必须是极大hex游走的起点，否则 "&H123456789&" 会被误切成 8 位
        return None;
    }
    let body = src.get(from + offset..)?;
    let mut len = 0usize;
    while body
        .as_bytes()
        .get(len)
        .copied()
        .is_some_and(|c| c.is_ascii_hexdigit())
    {
        len += 1;
    }
    let digits = body.get(..len)?;
    if digits.len() != 6 && digits.len() != 8 {
        return None;
    }
    let colour = AssColor::parse_ass(digits).ok()?;
    let mut end = from + offset + len;
    let mut suffix = String::new();
    if src.get(end..end + 1) == Some("&") {
        suffix.push('&');
        end += 1;
        if src
            .get(end..end + 1)
            .is_some_and(|c| c.eq_ignore_ascii_case("h"))
        {
            suffix.push('H');
            end += 1;
        }
    }
    Some(ColourLiteral {
        end,
        prefix,
        digits: digits.to_owned(),
        suffix,
        colour,
    })
}

/// 只替换颜色字面量，其余字节（`&H` 写法、逗号、空格、字母大小写）原样保留。
fn recolour_args(
    args: &str,
    f: &dyn Fn(AssColor) -> Result<AssColor, StkError>,
) -> Result<String, StkError> {
    let mut out = String::with_capacity(args.len());
    let mut pos = 0usize;
    while pos < args.len() {
        match try_colour(args, pos) {
            Some(lit) => {
                let mapped = f(lit.colour)?;
                out.push_str(&lit.prefix);
                // 映射结果等于原值时连写法一起保留（含十六进制字母大小写），
                // 这样 identity transform 是真正的零改动（AT-ASS-002）
                let body = if mapped == lit.colour {
                    lit.digits.clone()
                } else {
                    // 宽度由原 digits 决定，十六进制文本由 core-model 生成
                    let probe = if lit.digits.len() == 6 {
                        "&H000000"
                    } else {
                        "&H00000000"
                    };
                    let rendered = mapped.to_ass_like(probe);
                    rendered.get(2..).unwrap_or(&rendered).to_owned()
                };
                out.push_str(&body);
                out.push_str(&lit.suffix);
                pos = lit.end;
            }
            None => match args[pos..].chars().next() {
                Some(ch) => {
                    out.push(ch);
                    pos += ch.len_utf8();
                }
                None => break,
            },
        }
    }
    Ok(out)
}

/// 遍历所有颜色 token（含 `\t` 内层与 VSFilterMod 复合颜色）；非颜色 token 不动。
/// 11_HDR_COLOR 的 transform 走这个入口。
pub fn map_color_tokens<F>(text: &str, f: &F) -> Result<String, StkError>
where
    F: Fn(AssColor) -> Result<AssColor, StkError>,
{
    let mut tokens = parse_overrides(text);
    for token in &mut tokens {
        if let OverrideToken::Block(block) = token {
            map_tag_colours(&mut block.tags, f)?;
        }
    }
    Ok(tokens_to_string(&tokens))
}

fn map_tag_colours(
    tags: &mut [Tag],
    f: &dyn Fn(AssColor) -> Result<AssColor, StkError>,
) -> Result<(), StkError> {
    for tag in tags.iter_mut() {
        if matches!(tag.kind, TagKind::Colour { .. }) {
            let next = recolour_args(&tag.args, f)?;
            tag.args = next;
        } else if let TagKind::Animation { nested, .. } = &mut tag.kind {
            map_tag_colours(nested, f)?;
        }
    }
    Ok(())
}

/// 收集未识别 tag 名（小写，含反斜杠），供 loss 报告使用。
pub fn unknown_tags(text: &str, sink: &mut Vec<String>) {
    for token in parse_overrides(text) {
        if let OverrideToken::Block(block) = token {
            collect_unknown(&block.tags, sink);
        }
    }
}

fn collect_unknown(tags: &[Tag], sink: &mut Vec<String>) {
    for tag in tags {
        match &tag.kind {
            TagKind::Unknown => sink.push(tag.name.to_lowercase()),
            TagKind::Animation { nested, .. } => collect_unknown(nested, sink),
            _ => {}
        }
    }
}

/// 文本里可被 HDR transform 定位的颜色 token 数。
pub fn colour_token_count(text: &str) -> usize {
    let mut n = 0usize;
    for token in parse_overrides(text) {
        if let OverrideToken::Block(block) = token {
            count_colours(&block.tags, &mut n);
        }
    }
    n
}

fn count_colours(tags: &[Tag], n: &mut usize) {
    for tag in tags {
        match &tag.kind {
            TagKind::Colour { .. } => *n += 1,
            TagKind::Animation { nested, .. } => count_colours(nested, n),
            _ => {}
        }
    }
}
