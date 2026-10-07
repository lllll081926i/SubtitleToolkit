//! Styles/Events 的读取视角。视角只借用文档，不改原文顺序；
//! 字段名来自各段自己的 `Format:` 行，所以字段乱序/改名也能读到。

use core_model::color::AssColor;
use core_model::error::{Domain, StkError};
use core_model::time::Time;

use crate::ast::{AssDocument, Entry, EntryKind, Section, SectionKind};

fn bad_time(s: &str) -> StkError {
    StkError::simple(Domain::Parse, 36, "ASS 时间格式必须是 H:MM:SS.cc").with_detail(s.to_owned())
}

fn part_to_i64(part: &str) -> Option<i64> {
    part.trim().parse::<i64>().ok()
}

/// `0:00:01.00` / `1:02:03.45` → `Time`。秒与百分秒之间的 `.` 可缺省。
pub fn parse_ass_time(s: &str) -> Result<Time, StkError> {
    let t = s.trim();
    let mut it = t.split(':');
    let h = it.next().and_then(part_to_i64).ok_or_else(|| bad_time(s))?;
    let m = it.next().and_then(part_to_i64).ok_or_else(|| bad_time(s))?;
    let sec_part = it.next().ok_or_else(|| bad_time(s))?;
    if it.next().is_some() {
        return Err(bad_time(s));
    }
    let (sec, cs) = match sec_part.split_once('.') {
        Some((a, b)) => (
            part_to_i64(a).ok_or_else(|| bad_time(s))?,
            part_to_i64(b).ok_or_else(|| bad_time(s))?,
        ),
        None => (part_to_i64(sec_part).ok_or_else(|| bad_time(s))?, 0),
    };
    Time::from_hcs(h, m, sec, cs).map_err(|_| bad_time(s))
}

/// 字段查找：先按 `Format:` 行的名字，找不到再退回默认位置（省略/错写 Format 的行）。
fn named_or_positional<'a>(
    entry: &'a Entry,
    tail: Option<usize>,
    names: &[String],
    name: &str,
    fallback: usize,
    shift: usize,
) -> Option<&'a str> {
    let want = name.to_lowercase();
    let idx = names
        .iter()
        .position(|f| f.to_lowercase() == want)
        .unwrap_or(fallback);
    entry.field_slice(tail, idx.saturating_sub(shift))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleView<'a> {
    /// 条目在该段 `entries` 中的下标（配合 `Section::set_field` 定点改写）
    pub index: usize,
    pub entry: &'a Entry,
    pub names: &'a [String],
}

impl<'a> StyleView<'a> {
    pub fn raw(&self) -> &'a str {
        &self.entry.raw
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        let fallback = default_style_position(name);
        named_or_positional(self.entry, None, self.names, name, fallback, 0)
    }

    pub fn field_at(&self, idx: usize) -> Option<&str> {
        self.entry.field_slice(None, idx)
    }

    pub fn name(&self) -> Option<&str> {
        self.field("Name")
    }

    pub fn fontname(&self) -> Option<&str> {
        self.field("Fontname")
    }

    pub fn fontsize(&self) -> Option<&str> {
        self.field("Fontsize")
    }

    pub fn margin_l(&self) -> Option<&str> {
        self.field("MarginL")
    }

    pub fn margin_r(&self) -> Option<&str> {
        self.field("MarginR")
    }

    pub fn margin_v(&self) -> Option<&str> {
        self.field("MarginV")
    }

    pub fn outline(&self) -> Option<&str> {
        self.field("Outline")
    }

    pub fn shadow(&self) -> Option<&str> {
        self.field("Shadow")
    }

    fn colour(&self, name: &str) -> Option<AssColor> {
        AssColor::parse_ass(self.field(name)?).ok()
    }

    pub fn primary_colour(&self) -> Option<AssColor> {
        self.colour("PrimaryColour")
    }

    pub fn secondary_colour(&self) -> Option<AssColor> {
        self.colour("SecondaryColour")
    }

    /// V4+ 叫 OutlineColour，老 SSA 叫 TertiaryColour。
    pub fn outline_colour(&self) -> Option<AssColor> {
        self.colour("OutlineColour")
            .or_else(|| self.colour("TertiaryColour"))
    }

    pub fn back_colour(&self) -> Option<AssColor> {
        self.colour("BackColour")
    }
}

/// 没有 Format 行（或 Format 写的是别的表的字段名）时的默认位置。
fn default_style_position(name: &str) -> usize {
    match name.to_lowercase().as_str() {
        "name" => 0,
        "fontname" => 1,
        "fontsize" => 2,
        "primarycolour" => 3,
        "secondarycolour" => 4,
        "outlinecolour" | "tertiarycolour" => 5,
        "backcolour" => 6,
        "outline" => 16,
        "shadow" => 17,
        "marginl" | "leftmargin" => 19,
        "marginr" | "rightmargin" => 20,
        "marginv" | "verticalmargin" => 21,
        _ => usize::MAX,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogueView<'a> {
    pub index: usize,
    pub entry: &'a Entry,
    pub names: &'a [String],
}

impl<'a> DialogueView<'a> {
    fn tail(&self) -> Option<usize> {
        if self.names.is_empty() {
            None
        } else {
            Some(self.names.len())
        }
    }

    pub fn raw(&self) -> &'a str {
        &self.entry.raw
    }

    /// Aegisub 8b/10b 事故写法会整行省略 Marked/Layer：首段本身就是时间即成立，全部字段左移一格。
    fn shift(&self) -> usize {
        match self.entry.field_slice(self.tail(), 0) {
            Some(first) if parse_ass_time(first).is_ok() => 1,
            _ => 0,
        }
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        let fallback = default_event_position(name);
        named_or_positional(
            self.entry,
            self.tail(),
            self.names,
            name,
            fallback,
            self.shift(),
        )
    }

    pub fn field_at(&self, idx: usize) -> Option<&str> {
        self.entry.field_slice(self.tail(), idx)
    }

    pub fn start(&self) -> Result<Time, StkError> {
        parse_ass_time(
            self.field("Start")
                .ok_or_else(|| bad_time(&self.entry.raw))?,
        )
    }

    pub fn end(&self) -> Result<Time, StkError> {
        parse_ass_time(self.field("End").ok_or_else(|| bad_time(&self.entry.raw))?)
    }

    pub fn style_name(&self) -> Option<&str> {
        self.field("Style").map(str::trim)
    }

    pub fn actor(&self) -> Option<&str> {
        self.field("Name").map(str::trim)
    }

    pub fn effect(&self) -> Option<&str> {
        self.field("Effect")
    }

    pub fn layer(&self) -> Option<&str> {
        self.field("Layer").or_else(|| self.field("Marked"))
    }

    pub fn margin_l(&self) -> Option<&str> {
        self.field("MarginL")
    }

    pub fn margin_r(&self) -> Option<&str> {
        self.field("MarginR")
    }

    pub fn margin_v(&self) -> Option<&str> {
        self.field("MarginV")
    }

    /// Text 永远是该行的最后一段，含逗号与换行标记；字段偏少的行退化为最后一段。
    pub fn text(&self) -> &str {
        let tail = self.tail();
        let want = self.names.len().saturating_sub(1 + self.shift());
        let have = self.entry.field_count(tail);
        let idx = if have == 0 { want } else { want.min(have - 1) };
        self.entry.field_slice(tail, idx).unwrap_or("")
    }

    pub fn overrides(&self) -> Vec<crate::overrides::OverrideToken> {
        crate::overrides::parse_overrides(self.text())
    }
}

fn default_event_position(name: &str) -> usize {
    match name.to_lowercase().as_str() {
        "marked" | "layer" => 0,
        "start" => 1,
        "end" => 2,
        "style" => 3,
        "name" | "actor" => 4,
        "marginl" => 5,
        "marginr" => 6,
        "marginv" => 7,
        "effect" => 8,
        "text" => usize::MAX,
        _ => usize::MAX,
    }
}

impl Section {
    /// 单条目的事件视角（按段内下标）。
    pub fn dialogue_view(&self, index: usize) -> Option<DialogueView<'_>> {
        let entry = self.entry(index)?;
        Some(DialogueView {
            index,
            entry,
            names: &self.format,
        })
    }

    pub fn style_view(&self, index: usize) -> Option<StyleView<'_>> {
        let entry = self.entry(index)?;
        Some(StyleView {
            index,
            entry,
            names: &self.format,
        })
    }
}

/// `AssDocument` 上的读取视角入口。
impl AssDocument {
    pub fn styles(&self) -> Vec<StyleView<'_>> {
        let mut out = Vec::new();
        for s in &self.sections {
            if s.kind != SectionKind::Styles {
                continue;
            }
            for ei in s.style_entries() {
                if let Some(e) = s.entry(ei) {
                    out.push(StyleView {
                        index: ei,
                        entry: e,
                        names: &s.format,
                    });
                }
            }
        }
        out
    }

    pub fn dialogues(&self) -> Vec<DialogueView<'_>> {
        let mut out = Vec::new();
        for s in &self.sections {
            if s.kind != SectionKind::Events {
                continue;
            }
            for ei in s.dialogue_entries() {
                if let Some(e) = s.entry(ei) {
                    out.push(DialogueView {
                        index: ei,
                        entry: e,
                        names: &s.format,
                    });
                }
            }
        }
        out
    }

    /// Comment 条目也提供视角（08：Comments 必须保留且可读）。
    pub fn comments(&self) -> Vec<DialogueView<'_>> {
        let mut out = Vec::new();
        for s in &self.sections {
            if s.kind != SectionKind::Events {
                continue;
            }
            for (ei, e) in s.entries.iter().enumerate() {
                if e.kind == EntryKind::Comment {
                    out.push(DialogueView {
                        index: ei,
                        entry: e,
                        names: &s.format,
                    });
                }
            }
        }
        out
    }
}
