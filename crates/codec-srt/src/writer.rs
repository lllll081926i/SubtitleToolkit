//! 严格写出：**只负责"写出去还能原样读回来"**（AT-OUT reopen verify 的前提）。
//! 语义合法性（时间单调、文本非空、序号合法）由调用方先跑 `TextSubtitle::validate()` 决定，
//! 这里只拒绝物理上无法表达的东西：负时间、会被当成块分隔符的空白行、Ansi 模式下无法映射的字符。

use core_model::error::{Domain, StkError};

use crate::encoding::DetectedEncoding;
use crate::ir::TextSubtitle;

pub fn write(sub: &TextSubtitle) -> Result<Vec<u8>, StkError> {
    let nl = sub.newline.as_str();
    let mut body = String::new();
    let mut rest: &[_] = &sub.unrecognized;
    for (i, cue) in sub.cues.iter().enumerate() {
        // 先落回位置不晚于当前 cue 的未识别块原文（不补序号行，避免改动内容）。
        while let Some((head, tail)) = rest.split_first() {
            if head.after_cues > i {
                break;
            }
            let lines: Vec<&str> = head.raw.split('\n').collect();
            push_block(&mut body, nl, &lines)?;
            rest = tail;
        }
        let seq = i
            .checked_add(1)
            .ok_or_else(|| StkError::simple(Domain::Output, 134, "cue 数量超出可计数范围"))?;
        // 负时间由 core-model 报 STK-OUT-0003，本 crate 不复写第二套时间换算。
        let timing = format!(
            "{} --> {}",
            cue.start.to_srt_string()?,
            cue.end.to_srt_string()?
        );
        let number = seq.to_string();
        let extra = cue.text.matches('\n').count();
        let mut lines: Vec<&str> = Vec::with_capacity(2usize.checked_add(extra).unwrap_or(2));
        lines.push(number.as_str());
        lines.push(timing.as_str());
        lines.extend(cue.text.split('\n'));
        push_block(&mut body, nl, &lines)?;
    }
    for block in rest {
        let lines: Vec<&str> = block.raw.split('\n').collect();
        push_block(&mut body, nl, &lines)?;
    }
    escape_leading_bom(sub, nl, encode(sub, body)?)
}

/// 首字节恰好拼出 BOM 签名、但 `bom=false` 时，重读会把整份文件判成另一种编码，内容从第一个字符
/// 就偏掉（块机丢掉了原本起消歧作用的前导空行）。补一个在 SRT 里无语义的空行消歧：
/// 其它字节一律不动，且因为判定只看 body，`write∘parse` 仍是不动点。
fn escape_leading_bom(sub: &TextSubtitle, nl: &str, out: Vec<u8>) -> Result<Vec<u8>, StkError> {
    if sub.bom || !starts_with_bom_sig(&out) {
        return Ok(out);
    }
    let mut v = Vec::with_capacity(
        out.len()
            .checked_add(nl.len())
            .ok_or_else(|| StkError::simple(Domain::Output, 135, "输出长度溢出"))?,
    );
    v.extend_from_slice(nl.as_bytes());
    v.extend_from_slice(&out);
    Ok(v)
}

fn starts_with_bom_sig(b: &[u8]) -> bool {
    b.starts_with(&[0xEF, 0xBB, 0xBF])
        || b.starts_with(&[0xFF, 0xFE])
        || b.starts_with(&[0xFE, 0xFF])
}

/// 块 = 若干行 + 结尾空行，所以整份文件形如 `块1\n\n块2\n\n`；重解析与再次写出都是不动点。
fn push_block(out: &mut String, nl: &str, lines: &[&str]) -> Result<(), StkError> {
    let gap = nl.len().checked_mul(2);
    let hint = gap.and_then(|g| {
        lines.iter().try_fold(0usize, |acc, l| {
            acc.checked_add(l.len()).and_then(|v| v.checked_add(g))
        })
    });
    out.reserve(hint.unwrap_or_default());
    for l in lines {
        if l.chars().all(|c| matches!(c, ' ' | '\t')) {
            return Err(StkError::simple(
                Domain::Output,
                131,
                "块内含空白行，写出后会被当成块分隔符而丢内容",
            )
            .with_detail(format!("{l:?}"))
            .with_action("删掉该空行，或为其补上文本内容"));
        }
        out.push_str(l);
        out.push_str(nl);
    }
    out.push_str(nl);
    Ok(())
}

fn encode(sub: &TextSubtitle, body: String) -> Result<Vec<u8>, StkError> {
    if sub.bom && sub.encoding == DetectedEncoding::Ansi {
        return Err(
            StkError::simple(Domain::Output, 133, "Ansi 字节保真模式不能附加 UTF-8 BOM")
                .with_action("改为 Utf8/Utf8Bom 输出，或先做外部编码转换"),
        );
    }
    if sub.encoding == DetectedEncoding::Ansi {
        return encode_latin1(&body);
    }
    if !sub.bom {
        return Ok(body.into_bytes());
    }
    let mut v = Vec::with_capacity(body.len().checked_add(3).unwrap_or(body.len()));
    v.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    v.extend_from_slice(body.as_bytes());
    Ok(v)
}

/// `Ansi` 输入的文本用 1 字符 = 1 字节承载（见 `encoding`），回写时逐字节还原以保持字节保真。
fn encode_latin1(body: &str) -> Result<Vec<u8>, StkError> {
    let mut v: Vec<u8> = Vec::with_capacity(body.len());
    for ch in body.chars() {
        let b = u8::try_from(u32::from(ch)).map_err(|_| {
            StkError::simple(
                Domain::Output,
                132,
                "Ansi 源字幕含无法逐字节回写的字符，字节保真已不成立",
            )
            .with_detail(format!("{ch}"))
            .with_action("先做编码转换（GBK/CP1252…）再写出，或改为 UTF-8 输出")
        })?;
        v.push(b);
    }
    Ok(v)
}
