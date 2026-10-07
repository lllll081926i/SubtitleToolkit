//! PGS RLE（13 §4）。真实码流的游程编码带标志位：
//! 透明游程 `[00][len]` / `[00][40|hi][lo]`（不写颜色字节）；
//! 非透明单像素直接写颜色字节；非透明游程 `[00][80|len][color]` / `[00][C0|hi][lo][color]`。
//! 行尾 `00 00`，图末 `00 00 FF FF`。
//! 解码严格受像素预算约束：超出 w*h 立即报错，绝不先分配再收缩（23 §PGS / AT-SEC-001）。

use core_model::error::{Domain, Result, StkError};

const MAX_RUN: usize = 0x3FFF;

pub fn decode(payload: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    if w == 0 || h == 0 {
        return Err(StkError::simple(Domain::Pgs, 20, "RLE 目标尺寸非法"));
    }
    if w > 8192 || h > 8192 {
        return Err(StkError::simple(Domain::Pgs, 21, "RLE 尺寸超出支持上限"));
    }
    let budget = w
        .checked_mul(h)
        .ok_or_else(|| StkError::simple(Domain::Pgs, 22, "尺寸乘法溢出"))?;
    // 每个像素最多 1 字节；同时限制"极短 payload 声明极大画布"的分配攻击
    if payload.len() < 2 || budget > payload.len().saturating_mul(4096) {
        return Err(
            StkError::simple(Domain::Pgs, 23, "声明尺寸远大于压缩数据，拒绝分配")
                .with_detail(format!("{w}x{h} with {} payload bytes", payload.len())),
        );
    }
    let mut out: Vec<u8> = Vec::with_capacity(budget.min(1 << 20));
    let mut rows = 0usize;
    let mut i = 0usize;
    while i < payload.len() {
        let b = payload[i];
        if b != 0 {
            push(&mut out, b, 1, budget)?;
            i += 1;
            continue;
        }
        let next = *payload
            .get(i + 1)
            .ok_or_else(|| StkError::simple(Domain::Pgs, 24, "RLE 在行结束标记处被截断"))?;
        if next == 0 {
            i += 2;
            rows += 1;
            // 图末标记：末行的 00 00 之后紧跟 FF FF。部分编码器不写 FF FF，靠行数收敛，也要接受。
            if let Some(tail) = payload.get(i..) {
                if tail.len() >= 2 && tail[0] == 0xFF && tail[1] == 0xFF {
                    break;
                }
            }
            if !out.len().is_multiple_of(w) {
                return Err(StkError::simple(Domain::Pgs, 25, "行结束标记出现在行中间")
                    .with_detail(format!("row {rows} at pixel {}", out.len())));
            }
            continue;
        }
        let (color, used, run) = if next < 0x40 {
            (0u8, 2usize, usize::from(next))
        } else if next < 0x80 {
            let lo = *payload
                .get(i + 2)
                .ok_or_else(|| StkError::simple(Domain::Pgs, 24, "RLE 截断"))?;
            (0u8, 3, ((usize::from(next) & 0x3F) << 8) | usize::from(lo))
        } else if next < 0xC0 {
            let c = *payload
                .get(i + 2)
                .ok_or_else(|| StkError::simple(Domain::Pgs, 24, "RLE 截断"))?;
            (c, 3, usize::from(next) & 0x3F)
        } else {
            let lo = *payload
                .get(i + 2)
                .ok_or_else(|| StkError::simple(Domain::Pgs, 24, "RLE 截断"))?;
            let c = *payload
                .get(i + 3)
                .ok_or_else(|| StkError::simple(Domain::Pgs, 24, "RLE 截断"))?;
            (c, 4, ((usize::from(next) & 0x3F) << 8) | usize::from(lo))
        };
        if run == 0 {
            return Err(StkError::simple(Domain::Pgs, 26, "RLE 游程长度为 0"));
        }
        push(&mut out, color, run, budget)?;
        i += used;
    }
    if rows != h {
        return Err(
            StkError::simple(Domain::Pgs, 27, "RLE 行数与声明高度不一致")
                .with_detail(format!("decoded {rows} rows, declared {h}")),
        );
    }
    if out.len() != budget {
        return Err(
            StkError::simple(Domain::Pgs, 28, "RLE 像素总数与声明尺寸不一致")
                .with_detail(format!("decoded {}, expected {budget}", out.len())),
        );
    }
    Ok(out)
}

fn push(out: &mut Vec<u8>, color: u8, run: usize, budget: usize) -> Result<()> {
    if run > MAX_RUN {
        return Err(
            StkError::simple(Domain::Pgs, 29, "RLE 游程长度超过 14bit 上限")
                .with_detail(format!("{run}")),
        );
    }
    if out
        .len()
        .checked_add(run)
        .ok_or_else(|| StkError::simple(Domain::Pgs, 22, "像素数溢出"))?
        > budget
    {
        return Err(StkError::simple(
            Domain::Pgs,
            30,
            "RLE 解码超出像素预算（分配攻击或数据损坏）",
        )
        .with_detail(format!("have {}, run {run}, budget {budget}", out.len())));
    }
    out.resize(out.len() + run, color);
    Ok(())
}

/// 编码：行内游程 + 每行 `00 00`（末行合并为 `00 00 FF FF`）。
pub fn encode(indices: &[u8], w: usize, h: usize) -> Result<Vec<u8>> {
    if w == 0 || h == 0 {
        return Err(StkError::simple(Domain::Pgs, 20, "RLE 目标尺寸非法"));
    }
    let need = w
        .checked_mul(h)
        .ok_or_else(|| StkError::simple(Domain::Pgs, 22, "尺寸乘法溢出"))?;
    if indices.len() != need {
        return Err(StkError::simple(Domain::Pgs, 31, "索引图长度与尺寸不符")
            .with_detail(format!("{} != {need}", indices.len())));
    }
    let mut out = Vec::with_capacity(need / 4 + h * 2 + 4);
    for y in 0..h {
        let row = &indices[y * w..(y + 1) * w];
        let mut x = 0usize;
        while x < w {
            let color = row[x];
            let mut run = 1usize;
            while x + run < w && row[x + run] == color && run < MAX_RUN {
                run += 1;
            }
            match (color, run) {
                (0, r) if r < 0x40 => {
                    out.push(0);
                    out.push(r as u8);
                }
                (0, r) => {
                    out.push(0);
                    out.push(0x40 | (r >> 8) as u8);
                    out.push((r & 0xFF) as u8);
                }
                (_, 1) => out.push(color),
                (c, r) if r < 0x40 => {
                    out.push(0);
                    out.push(0x80 | r as u8);
                    out.push(c);
                }
                (c, r) => {
                    out.push(0);
                    out.push(0xC0 | (r >> 8) as u8);
                    out.push((r & 0xFF) as u8);
                    out.push(c);
                }
            }
            x += run;
        }
        out.push(0);
        out.push(0);
        if y + 1 == h {
            out.push(0xFF);
            out.push(0xFF);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_format_is_pinned_per_variant() {
        // 四种码型各一条，字节级钉死，防止将来"自我一致"地改错
        assert_eq!(
            encode(&[1], 1, 1).unwrap(),
            vec![0x01, 0x00, 0x00, 0xFF, 0xFF]
        );
        assert_eq!(
            encode(&[0, 0, 0], 3, 1).unwrap(),
            vec![0x00, 0x03, 0x00, 0x00, 0xFF, 0xFF]
        );
        assert_eq!(
            encode(&[2, 2], 2, 1).unwrap(),
            vec![0x00, 0x82, 0x02, 0x00, 0x00, 0xFF, 0xFF]
        );
        let long = vec![0u8; 100];
        let e = encode(&long, 100, 1).unwrap();
        assert_eq!(
            &e[0..3],
            &[0x00, 0x40, 100],
            "长透明游程用 40|hi + lo，不写颜色"
        );
        let longc = vec![7u8; 100];
        let e2 = encode(&longc, 100, 1).unwrap();
        assert_eq!(
            &e2[0..4],
            &[0x00, 0xC0, 100, 7],
            "长非透明游程用 C0|hi + lo + color"
        );
    }

    #[test]
    fn decode_accepts_each_variant() {
        assert_eq!(decode(&[0x03, 0x00, 0x00], 1, 1).unwrap(), vec![3]);
        assert_eq!(
            decode(&[0x00, 0x03, 0x00, 0x00], 3, 1).unwrap(),
            vec![0, 0, 0]
        );
        assert_eq!(
            decode(&[0x00, 0x82, 0x05, 0x00, 0x00], 2, 1).unwrap(),
            vec![5, 5]
        );
        assert_eq!(
            decode(&[0x00, 0xC0, 0x64, 0x09, 0x00, 0x00], 100, 1).unwrap(),
            vec![9u8; 100]
        );
    }

    #[test]
    fn roundtrip_without_terminator_is_also_accepted() {
        // 部分编码器只写行尾 00 00，不写 FF FF
        let px: Vec<u8> = (0..4 * 3).map(|i| (i % 3) as u8).collect();
        let mut stream = encode(&px, 4, 3).unwrap();
        assert!(stream.ends_with(&[0x00, 0x00, 0xFF, 0xFF]));
        stream.truncate(stream.len() - 2);
        assert_eq!(decode(&stream, 4, 3).unwrap(), px);
        assert_eq!(decode(&encode(&px, 4, 3).unwrap(), 4, 3).unwrap(), px);
    }

    #[test]
    fn encode_decode_roundtrip_for_every_shape() {
        let cases: Vec<(Vec<u8>, usize, usize)> = vec![
            (vec![0, 0, 0, 0, 1, 1, 1, 2], 4, 2),
            (vec![7u8; 8], 4, 2),
            (vec![0u8; 16], 8, 2),
            (vec![1u8; 4000], 100, 40),
            ((0..255u8).collect::<Vec<u8>>(), 15, 17),
            (vec![0u8; 8000], 1600, 5),
        ];
        for (px, w, h) in cases {
            let rle = encode(&px, w, h).unwrap();
            assert_eq!(decode(&rle, w, h).unwrap(), px, "{w}x{h}");
        }
    }

    #[test]
    fn decode_rejects_pixel_budget_attack() {
        // 声明 4000x3000 但 payload 只有 6 字节：必须拒绝而不是先分配 12M
        let e = decode(&[0x00, 0x00, 0x00, 0xFF], 4000, 3000).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0023");
    }

    #[test]
    fn decode_rejects_row_count_mismatch() {
        // 只给了 2 行，却声明 3 行
        let v = vec![1u8, 1, 1, 1, 0x00, 0x00, 2, 2, 2, 2, 0x00, 0x00];
        assert_eq!(decode(&v, 4, 3).unwrap_err().code_string(), "STK-PGS-0027");
        // 反过来：行数对得上时不可能像素不足，所以 STK-PGS-0028 是防御性分支；
        // 这里验证真正可达的那条：末行未走完就结束 → 0025
        let v2 = vec![1u8, 1, 1, 0x00, 0x00];
        assert_eq!(decode(&v2, 4, 1).unwrap_err().code_string(), "STK-PGS-0025");
    }

    #[test]
    fn decode_rejects_run_overflowing_budget() {
        // 声明每行 4 像素，但非透明游程长度 10
        let v = vec![0x00, 0x8A, 0x7F, 0x00, 0x00, 0xFF, 0xFF];
        assert_eq!(decode(&v, 4, 1).unwrap_err().code_string(), "STK-PGS-0030");
    }

    #[test]
    fn truncated_payload_errors_instead_of_panicking() {
        assert!(decode(&[0x00], 2, 1).is_err());
        assert!(decode(&[0x00, 0x05], 2, 1).is_err());
        assert!(decode(&[0x00, 0x40], 4, 1).is_err());
        assert!(decode(&[0x00, 0x82], 4, 1).is_err());
        assert!(decode(&[0x00, 0xC0, 0x05], 6, 1).is_err());
        assert!(decode(&[1u8; 8], 2, 2).is_err(), "没有行结束标记");
        assert!(decode(&[], 2, 2).is_err());
        assert!(decode(&[0, 0, 0, 0xFF], 0, 1).is_err());
    }

    #[test]
    fn encode_rejects_bad_input() {
        assert!(encode(&[0, 0], 2, 2).is_err());
        assert!(encode(&[0u8; 4], 0, 1).is_err());
        let e = encode(&[0u8; 3], 2, 2).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0031");
    }

    #[test]
    fn transparent_index_zero_is_the_default_color() {
        // PGS 约定索引 0 为背景/透明；整幅透明必须压缩到极小
        let px = vec![0u8; 1920 * 4];
        let rle = encode(&px, 1920, 4).unwrap();
        assert_eq!(decode(&rle, 1920, 4).unwrap(), px);
        assert!(
            rle.len() < 64,
            "全透明 1920x4 应该只有几十字节，实际 {}",
            rle.len()
        );
    }

    #[test]
    fn mid_row_marker_is_rejected() {
        // 行中间出现 00 00（前面只落了 3 个像素，行宽 4）
        let v = vec![1u8, 2, 3, 0x00, 0x00, 0x00, 0xFF];
        assert_eq!(decode(&v, 4, 2).unwrap_err().code_string(), "STK-PGS-0025");
    }
}
