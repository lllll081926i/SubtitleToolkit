//! ASS 颜色：`&HAABBGGRR`。HDR 变换需要能拆到 RGB 通道并回到 ASS 文本。

use crate::error::{Domain, StkError};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct AssColor {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl AssColor {
    pub const fn rgb(r: u8, g: u8, b: u8) -> AssColor {
        AssColor { a: 0, r, g, b }
    }

    /// 解析 `&HBBGGRR`、`&HAABBGGRR`、`HBBGGRR`、`BBGGRR`（大小写与前后缀均容忍）。
    pub fn parse_ass(s: &str) -> Result<AssColor, StkError> {
        let t = s.trim();
        let body = t
            .trim_start_matches('&')
            .trim_start_matches('H')
            .trim_start_matches('h');
        if body.is_empty() || !body.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(
                StkError::simple(Domain::Parse, 40, "ASS 颜色格式非法").with_detail(s.to_string())
            );
        }
        let v = u32::from_str_radix(body, 16).map_err(|_| {
            StkError::simple(Domain::Parse, 40, "ASS 颜色数值非法").with_detail(s.to_string())
        })?;
        let color = match body.len() {
            6 => AssColor {
                a: 0,
                b: (v >> 16) as u8,
                g: (v >> 8) as u8,
                r: v as u8,
            },
            8 => AssColor {
                a: (v >> 24) as u8,
                b: (v >> 16) as u8,
                g: (v >> 8) as u8,
                r: v as u8,
            },
            _ => {
                return Err(StkError::simple(
                    Domain::Parse,
                    41,
                    "ASS 颜色长度必须是 6 或 8 位十六进制",
                )
                .with_detail(s.to_string()))
            }
        };
        Ok(color)
    }

    /// 输出 8 位 `&HAABBGGRR`。
    pub fn to_ass_string(self) -> String {
        format!("&H{:02X}{:02X}{:02X}{:02X}", self.a, self.b, self.g, self.r)
    }

    /// 输出 6 位 `&HBBGGRR`（保留原 alpha 为 0 时的常见写法）。
    pub fn to_ass_string_6(self) -> String {
        format!("&H{:02X}{:02X}{:02X}", self.b, self.g, self.r)
    }

    /// 按输入串长度回写同宽度，避免 round-trip 改变原文（11_HDR_COLOR：只改目标字段）。
    pub fn to_ass_like(self, original: &str) -> String {
        let body = original
            .trim_start_matches('&')
            .trim_start_matches(['H', 'h']);
        if body.len() == 6 {
            self.to_ass_string_6()
        } else {
            self.to_ass_string()
        }
    }

    pub fn argb_u32(self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | self.b as u32
    }
    pub fn from_argb_u32(v: u32) -> AssColor {
        AssColor {
            a: (v >> 24) as u8,
            r: (v >> 16) as u8,
            g: (v >> 8) as u8,
            b: v as u8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_widths() {
        assert_eq!(
            AssColor::parse_ass("&H00FFFFFF").unwrap(),
            AssColor::rgb(255, 255, 255)
        );
        assert_eq!(
            AssColor::parse_ass("&H80000000").unwrap(),
            AssColor {
                a: 0x80,
                r: 0,
                g: 0,
                b: 0
            }
        );
        assert_eq!(
            AssColor::parse_ass("H123456").unwrap(),
            AssColor {
                a: 0,
                r: 0x56,
                g: 0x34,
                b: 0x12
            }
        );
        assert_eq!(
            AssColor::parse_ass("&h0000ff00").unwrap(),
            AssColor::rgb(0, 255, 0)
        );
        assert!(AssColor::parse_ass("&H12345").is_err());
        assert!(AssColor::parse_ass("&HZZZZZZ").is_err());
    }

    #[test]
    fn roundtrip_preserves_width() {
        let c = AssColor::parse_ass("&HFFFFFF").unwrap();
        assert_eq!(c.to_ass_like("&HFFFFFF"), "&HFFFFFF");
        let d = AssColor::parse_ass("&H00FFFFFF").unwrap();
        assert_eq!(d.to_ass_like("&H00FFFFFF"), "&H00FFFFFF");
        assert_eq!(AssColor::from_argb_u32(c.argb_u32()), c);
    }
}
