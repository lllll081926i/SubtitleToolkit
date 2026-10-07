//! ASS/SSA 字节保真 AST（08_FORMATS_AND_IR.md §1、AT-ASS-001）。
//!
//! 不变量：`AssDocument::parse(src).to_string() == src`。写出路径只认每条条目保存的
//! 原文行与行尾终止符，结构化视图（字段表、override token）是读取视角；改写走
//! 「替换目标字段 → 重新拼装该条 raw line」，因此未修改的内容必然原样 round-trip，
//! 未知 key / 未知 tag / 注释 / 附件一律不丢。

pub mod ast;
pub mod line;
pub mod loss;
pub mod overrides;
pub mod views;

pub use ast::{
    AssDocument, AssVariant, Entry, EntryKind, Field, FormatSource, Section, SectionKind,
};
pub use line::Eol;
pub use loss::loss_of;
pub use overrides::{
    map_color_tokens, parse_overrides, tokens_to_string, OverrideBlock, OverrideToken, Tag, TagKind,
};
pub use views::{parse_ass_time, DialogueView, StyleView};

/// 下游 crate（transform-hdr / transform-resolution / validator）只依赖本 crate 就能拿到核心类型。
pub use core_model::color::AssColor;
pub use core_model::error::{Domain, Result, StkError};
pub use core_model::loss::LossReport;
pub use core_model::time::{Fps, Rational, Time};
