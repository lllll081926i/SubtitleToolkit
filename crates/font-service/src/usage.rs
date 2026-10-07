//! ASS 字体用量扫描（17 §3）：字幕**真正引用**了哪些 face、每个 face 要画哪些字符。
//!
//! 三条来自规格的要求，逐条落实在这里：
//!
//! 1. **按动态 `Format:` 字段解析 Style**，不依赖固定列号 —— 走 `codec-ass` 的 `StyleView`。
//! 2. **state tracking**：`\fn`、`\b`、`\i`、`\r`/`\rStyle` reset 会改变"接下来的字用哪个 face"，
//!    所以必须按顺序走一遍，而不是把整条文本里的 tag 收集成一堆。
//! 3. **按 dialogue 收集实际字符**：drawing（`\p`）内容不是字符，卡拉OK 的 `\k` 参数不是字符，
//!    换行/硬空格这些转义也不是字符。
//!
//! 一个已知不精确处：`\t` 内层 tag 会把动画中途改的字体当成"这个 face 也要画后面这些字"。
//! 宁可多报一个 face，也不要漏报一个 —— 漏报会让用户在缺字体的机器上看到意外字形。

use std::collections::BTreeSet;

use codec_ass::{parse_overrides, AssDocument, OverrideToken, StyleView, Tag};
use core_model::error::{Domain, Result, StkError};

/// 一次"某个 face 被用来画这些字符"。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRequest {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub chars: BTreeSet<char>,
    /// 带重复的字次数：`chars` 去重后看不出"这字体用了 5000 次"
    pub pen: usize,
    /// 是谁引入的：`style:Default`、`line 4 \fn`、`line 4 \t 内层 \fn`
    pub sources: Vec<String>,
}

impl FaceRequest {
    fn new(family: &str, bold: bool, italic: bool) -> Self {
        FaceRequest {
            family: family.to_string(),
            bold,
            italic,
            chars: BTreeSet::new(),
            pen: 0,
            sources: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct State {
    /// style 决定的基准（`\r` 回到这里）
    base_family: String,
    base_bold: bool,
    base_italic: bool,
    family: String,
    bold: bool,
    italic: bool,
}

/// 扫描一份 ASS 文档。SSA（v4.00）与 ASS 都用同一套 tag 语义处理。
pub fn scan_document(doc: &AssDocument) -> Result<Vec<FaceRequest>> {
    let styles = doc.styles();
    let mut by_name: Vec<(String, StyleView<'_>)> = Vec::new();
    for s in styles {
        if let Some(name) = s.name() {
            by_name.push((name.to_string(), s));
        }
    }
    let fallback = if by_name.is_empty() {
        None
    } else {
        Some(by_name[0].1)
    };

    let mut out: Vec<FaceRequest> = Vec::new();
    for (row, d) in doc.dialogues().iter().enumerate() {
        let style_name = d.style_name().unwrap_or("Default").to_string();
        let style = lookup(&by_name, &style_name).or(fallback);
        let mut st = match style {
            Some(s) => State {
                base_family: s.fontname().unwrap_or("Arial").to_string(),
                base_bold: truthy(s.field("Bold")),
                base_italic: truthy(s.field("Italic")),
                family: String::new(),
                bold: false,
                italic: false,
            },
            None => State {
                base_family: "Arial".to_string(),
                base_bold: false,
                base_italic: false,
                family: String::new(),
                bold: false,
                italic: false,
            },
        };
        st.family = st.base_family.clone();
        st.bold = st.base_bold;
        st.italic = st.base_italic;
        // 基准 style 本身也算一次引用：没有 inline tag 的字幕常用它
        touch(&mut out, &st, &format!("style:{style_name}"));

        let text = d.text();
        for token in parse_overrides(text) {
            match token {
                OverrideToken::Text { raw } => {
                    for c in visible_chars(&raw) {
                        let idx = index_for(&mut out, &st);
                        out[idx].chars.insert(c);
                        out[idx].pen += 1;
                    }
                }
                // drawing 模式的命令串不是"字符"，不进入覆盖检查（17 §3）
                OverrideToken::Drawing { .. } => {}
                OverrideToken::Block(block) => {
                    for tag in walk_tags(&block.tags) {
                        if apply_tag(&mut st, &by_name, tag) {
                            touch(&mut out, &st, &format!("line {} {}", row + 1, tag.name));
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        (a.family.as_str(), a.bold, a.italic)
            .partial_cmp(&(b.family.as_str(), b.bold, b.italic))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

/// 展开 `\t` 的内层 tag：动画中途改的字体也要被算进用量。
fn walk_tags(tags: &[Tag]) -> Vec<&Tag> {
    let mut out = Vec::new();
    for t in tags {
        out.push(t);
        if let codec_ass::TagKind::Animation { nested, .. } = &t.kind {
            out.extend(walk_tags(nested));
        }
    }
    out
}

/// 应用一个 tag。返回 true 表示"face 变了"（需要记录一次引用来源）。
fn apply_tag(st: &mut State, styles: &[(String, StyleView<'_>)], tag: &Tag) -> bool {
    let arg = tag.args.trim();
    match tag.name.as_str() {
        "\\fn" => {
            // `\fn` 不带参数 = 回到 style 的字体（VSFilter 语义）
            let next = if arg.is_empty() {
                st.base_family.clone()
            } else {
                arg.to_string()
            };
            let changed = next != st.family;
            st.family = next;
            changed
        }
        "\\b" | "\\bf" => {
            let next = arg != "0";
            let changed = next != st.bold;
            st.bold = next;
            changed
        }
        "\\i" => {
            let next = arg != "0";
            let changed = next != st.italic;
            st.italic = next;
            changed
        }
        "\\r" => {
            // `\r` 回 style；`\rStyleName` 回另一个 style 的基准
            if arg.is_empty() {
                st.family = st.base_family.clone();
                st.bold = st.base_bold;
                st.italic = st.base_italic;
            } else if let Some(s) = lookup(styles, arg) {
                st.family = s.fontname().unwrap_or("Arial").to_string();
                st.bold = truthy(s.field("Bold"));
                st.italic = truthy(s.field("Italic"));
            }
            true
        }
        _ => false,
    }
}

fn lookup<'a>(styles: &'a [(String, StyleView<'a>)], name: &str) -> Option<StyleView<'a>> {
    styles
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, s)| *s)
}

fn truthy(v: Option<&str>) -> bool {
    matches!(
        v.map(str::trim),
        Some("-1") | Some("1") | Some("yes") | Some("true") | Some("Yes") | Some("TRUE")
    )
}

/// 文本里真正会被画出来的字符：跳过 `\N`/`\n`/`\h` 这类转义与 `{}` 之外的花括号。
fn visible_chars(raw: &str) -> Vec<char> {
    let mut out = Vec::new();
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            // 转义：吃掉紧随其后的一个字符（\N \n \h 都是两字符；\ 罕见，同样处理）
            it.next();
            continue;
        }
        if c == '{' || c == '}' {
            continue;
        }
        out.push(c);
    }
    out
}

fn index_for(list: &mut Vec<FaceRequest>, st: &State) -> usize {
    if let Some(i) = list
        .iter()
        .position(|r| r.family == st.family && r.bold == st.bold && r.italic == st.italic)
    {
        return i;
    }
    list.push(FaceRequest::new(&st.family, st.bold, st.italic));
    list.len() - 1
}

fn touch(list: &mut Vec<FaceRequest>, st: &State, source: &str) {
    let idx = index_for(list, st);
    let item = &mut list[idx];
    if !item.sources.iter().any(|s| s == source) {
        item.sources.push(source.to_string());
    }
}

/// 用于测试与 CLI 的便捷入口：从文本直接得到用量表。
pub fn scan_text(ass: &str) -> Result<Vec<FaceRequest>> {
    let doc = AssDocument::parse(ass);
    if doc.sections_of(codec_ass::SectionKind::Events).is_empty() {
        return Err(StkError::simple(
            Domain::Input,
            230,
            "这份文件没有 [Events] 段，谈不上字体用量",
        ));
    }
    scan_document(&doc)
}
