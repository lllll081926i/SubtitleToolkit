//! SRT 编解码：`bytes → TextSubtitle IR → bytes`（08 §1 TextSubtitle IR、§4 Loss Model）。
//!
//! 三条硬口径：
//! - **宽容解析**：能读就继续读，可疑处写进 `LossReport`；无法安全归属的块整体保留原文，
//!   所以绝不 panic、绝不静默改内容（旧闭源工具的反例，见 31 §13）。
//! - **严格写出**：`write` 只保证"写出去还能读回来"，语义合法性由调用方先跑 `validate()`。
//! - **无 float**：时间只用 `core-model` 的 `Rational`/`Time`（ADR-007），毫秒解析走
//!   `Time::from_hms_millis`，格式化走 `Time::to_srt_string`。

mod encoding;
mod ir;
mod parser;
mod writer;

pub use crate::writer::write;
pub use encoding::{DetectedEncoding, Newline};
pub use ir::{TextCue, TextSubtitle, UnrecognizedBlock};

use core_model::error::StkError;
use core_model::loss::LossReport;

/// `bytes → IR`：SRT 读取的唯一入口（对应 14 §SRT→ASS 的 `Probe → Text IR` 两段）。
pub fn parse(bytes: &[u8]) -> Result<(TextSubtitle, LossReport), StkError> {
    let decoded = encoding::decode(bytes)?;
    let mut loss = LossReport::new();
    if decoded.encoding == DetectedEncoding::Ansi {
        loss.user_attention(
            "非 UTF-8 字节已按原样保留（1 字节=1 字符），需外部编码转换才能正确显示",
        );
    }
    let mut sub = TextSubtitle {
        bom: decoded.bom,
        encoding: decoded.encoding,
        ..Default::default()
    };
    if let Some(e) = parser::collect(parser::split_lines(&decoded.text), &mut sub, &mut loss) {
        return Err(e);
    }
    loss.preserved("cue 时间与文本（毫秒精度、字节保真）");
    if sub.cues.is_empty() {
        loss.user_attention("未解析到任何有效 cue");
    }
    Ok((sub, loss))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_ser<T: serde::Serialize + serde::de::DeserializeOwned>() {}

    /// IR 要能进 JobSnapshot / bug report 的 JSON（07 §4、22），这里做编译期约束检查。
    #[test]
    fn ir_types_are_serializable() {
        assert_ser::<TextCue>();
        assert_ser::<TextSubtitle>();
        assert_ser::<UnrecognizedBlock>();
        assert_ser::<DetectedEncoding>();
        assert_ser::<Newline>();
    }

    #[test]
    fn only_blank_or_ascii_whitespace_lines_split_blocks() {
        assert!(parser::is_separator("  \t"));
        // 非 ASCII 空白（NBSP）是内容，不能被当成空行吃掉（AT-PATH 中文语料）。
        assert!(!parser::is_separator("\u{a0}"));
    }

    #[test]
    fn line_splitter_reports_first_style_and_variety() {
        let cr = parser::split_lines("a\rb\r\nc");
        assert_eq!(
            (cr.lines.as_slice(), cr.newline, cr.mixed, cr.cr_seen),
            (["a", "b", "c"].as_slice(), Newline::Lf, true, true)
        );
        let lf = parser::split_lines("a\nb");
        assert_eq!(
            (lf.lines.as_slice(), lf.newline, lf.mixed, lf.cr_seen),
            (["a", "b"].as_slice(), Newline::Lf, false, false)
        );
    }
}
