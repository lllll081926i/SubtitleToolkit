//! 字节 → 文本：只做 BOM/编码判定与逐字节保真，不引入任何字符集表（21 依赖约束）。
//!
//! 关键策略：非法 UTF-8 且无 BOM 的输入按 `DetectedEncoding::Ansi` 处理——**1 字节存成 1 个
//! 字符**（Latin-1 窗口），这样 GBK/CP1252 之类的原始字节既不会被 lossy 替换字符静默改掉
//! （可复现性要求），也不会在本 crate 里被"猜"成错误文本；真正的转码由外部编码转换器负责，
//! `parse` 用 `loss.user_attention` 把这件事交还给调用方。

use core_model::error::{Domain, StkError};
use serde::{Deserialize, Serialize};

/// 探测结果。`Ansi` = "无法确认的字节流"，不是"确认是 Windows-1252"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedEncoding {
    #[default]
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Ansi,
}

/// 文件换行风格。规范只定义两种输出风格；只含 CR 的输入归一为 `Lf` 并记 non-standard。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Newline {
    #[default]
    Lf,
    CrLf,
}

impl Newline {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Newline::Lf => "\n",
            Newline::CrLf => "\r\n",
        }
    }
}

pub(crate) struct Decoded {
    pub(crate) text: String,
    pub(crate) encoding: DetectedEncoding,
    pub(crate) bom: bool,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const UTF16_LE_BOM: [u8; 2] = [0xFF, 0xFE];
const UTF16_BE_BOM: [u8; 2] = [0xFE, 0xFF];

pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, StkError> {
    if let Some(rest) = bytes.strip_prefix(&UTF8_BOM) {
        // 有 UTF-8 BOM 就是明确声明 UTF-8，非法序列属于真错误，不能降级成 Ansi。
        let text = core::str::from_utf8(rest).map_err(|e| {
            StkError::simple(Domain::Parse, 120, "带 UTF-8 BOM 的文件内含非法 UTF-8 序列")
                .with_detail(format!("BOM 之后第 {} 字节起", e.valid_up_to()))
                .with_action("去掉 BOM 让字节保真流程接管，或先修复文件编码")
        })?;
        return Ok(Decoded {
            text: text.to_string(),
            encoding: DetectedEncoding::Utf8Bom,
            bom: true,
        });
    }
    if let Some(rest) = bytes.strip_prefix(&UTF16_LE_BOM) {
        return Ok(Decoded {
            text: decode_utf16(rest, true)?,
            encoding: DetectedEncoding::Utf16Le,
            bom: true,
        });
    }
    if let Some(rest) = bytes.strip_prefix(&UTF16_BE_BOM) {
        return Ok(Decoded {
            text: decode_utf16(rest, false)?,
            encoding: DetectedEncoding::Utf16Be,
            bom: true,
        });
    }
    match core::str::from_utf8(bytes) {
        Ok(s) => Ok(Decoded {
            text: s.to_string(),
            encoding: DetectedEncoding::Utf8,
            bom: false,
        }),
        Err(_) => Ok(Decoded {
            text: bytes.iter().map(|b| char::from(*b)).collect(),
            encoding: DetectedEncoding::Ansi,
            bom: false,
        }),
    }
}

/// 手写 UTF-16 解码：奇数字节与未配对代理项都是错误，绝不 panic。
fn decode_utf16(bytes: &[u8], little: bool) -> Result<String, StkError> {
    // 用 `chunks(2)` + 末块长度判奇偶：落单的半字符在此就地报错，不留到后面按下标越界。
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let [a, b] = *pair else {
            return Err(StkError::simple(
                Domain::Parse,
                121,
                "UTF-16 数据长度为奇数字节，末尾缺少一字节",
            )
            .with_detail(format!("BOM 之后 {} 字节", bytes.len()))
            .with_action("文件可能被截断：重新导出该字幕"));
        };
        units.push(if little {
            u16::from_le_bytes([a, b])
        } else {
            u16::from_be_bytes([a, b])
        });
    }
    // UTF-8 上界 = 码元数 * 3 字节（BMP 内 1 码元最多 3 字节），仍是输入字节数的 3/2，无分配放大面。
    let cap = bytes
        .len()
        .checked_div(2)
        .and_then(|n| n.checked_mul(3))
        .unwrap_or_default();
    let mut text = String::with_capacity(cap);
    for (i, r) in core::char::decode_utf16(units).enumerate() {
        match r {
            Ok(c) => text.push(c),
            Err(e) => {
                return Err(
                    StkError::simple(Domain::Parse, 122, "UTF-16 存在未配对的代理项")
                        .with_detail(format!("第 {i} 个码元 U+{:04X}", e.unpaired_surrogate()))
                        .with_action("用其它工具重存该 UTF-16 字幕"),
                );
            }
        }
    }
    Ok(text)
}
