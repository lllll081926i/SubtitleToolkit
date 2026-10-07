//! 行级原文模型。round-trip 的字节保真来自这里：每行单独记住自己的结束符种类，
//! 因此混合换行（同一文件里既有 CRLF 又有 LF）与缺失的末尾换行都能原样还原。

/// 行结束符种类；`Eof` 表示输入在该行后直接结束（无终止符）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Cr,
    Lf,
    CrLf,
    Eof,
}

impl Eol {
    pub fn str(self) -> &'static str {
        match self {
            Eol::Cr => "\r",
            Eol::Lf => "\n",
            Eol::CrLf => "\r\n",
            Eol::Eof => "",
        }
    }

    /// 终止符自身占用的字节数（`Eof` 为 0）。
    pub fn byte_len(self) -> usize {
        self.str().len()
    }
}

/// 按 CR / LF / CRLF 切分。末尾若以终止符结束则不产生额外的空行条目，
/// 使 `split → concat` 对任意输入都是恒等的。
pub fn split_lines(src: &str) -> Vec<(&str, Eol)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\r' || b[i] == b'\n' {
            let term = i;
            let eol = if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
                Eol::CrLf
            } else if b[i] == b'\r' {
                Eol::Cr
            } else {
                Eol::Lf
            };
            i += eol.byte_len();
            if let Some(line) = src.get(start..term) {
                out.push((line, eol));
            }
            start = i;
        } else {
            i += 1;
        }
    }
    if start < src.len() {
        if let Some(tail) = src.get(start..) {
            out.push((tail, Eol::Eof));
        }
    }
    out
}

/// `Key: value` / `Dialogue: a,b,c` 的分界；没有冒号即不是可解析行。
pub fn colon(line: &str) -> Option<usize> {
    line.find(':')
}

/// 前缀长度：到 ':'（含）以及紧随其后的空格/制表符。
/// 字段内容从第一个非空白字符开始，这样 `fields()` 与 `field_slice()` 的边界一致。
pub fn head_len(line: &str) -> Option<usize> {
    let c = colon(line)?;
    Some(skip_ws_from(line, c + 1))
}

/// 从 `from` 起跳过空格/制表符，返回下一个字段的起始位置。
pub fn skip_ws_from(line: &str, from: usize) -> usize {
    let b = line.as_bytes();
    let mut i = from;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_all_terminator_kinds() {
        assert_eq!(
            split_lines("a\nb\r\nc\rd"),
            vec![
                ("a", Eol::Lf),
                ("b", Eol::CrLf),
                ("c", Eol::Cr),
                ("d", Eol::Eof)
            ]
        );
    }

    #[test]
    fn trailing_terminator_does_not_add_ghost_line() {
        assert_eq!(split_lines("a\n"), vec![("a", Eol::Lf)]);
        assert_eq!(split_lines("a\n\n"), vec![("a", Eol::Lf), ("", Eol::Lf)]);
        assert_eq!(split_lines(""), vec![]);
    }

    #[test]
    fn multibyte_content_is_never_split_mid_char() {
        let s = "字幕🎬\r\n";
        let parts = split_lines(s);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts.first().map(|p| p.0), Some("字幕🎬"));
    }

    #[test]
    fn head_len_eats_spaces_only() {
        assert_eq!(head_len("PlayResX:  1280"), Some("PlayResX:  ".len()));
        assert_eq!(head_len("Style:A,B"), Some("Style:".len()));
        assert_eq!(head_len("no colon here"), None);
    }
}
