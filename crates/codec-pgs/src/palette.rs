//! PGS 调色板（13 §4、40 §3.3）：条目为 5 字节 `[index][Y][Cr][Cb][T]`，
//! 颜色空间是 limited-range YCbCr，不是 RGB。SD 与 HD 使用不同矩阵。

use core_model::error::{Domain, Result, StkError};

/// 8-bit PDS：header 2 字节 + 每条 5 字节。
pub const PDS_HEADER: usize = 2;
pub const PDS_ENTRY: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ColorMatrix {
    Bt601,
    Bt709,
}

impl ColorMatrix {
    /// 蓝光侧经验：SD/720 一档用 BT.601，1080 及以上用 BT.709（ass2bdnxml 也按 SD/HD 分流）。
    /// 本函数的选择必须用真盘语料验证后才写进 golden（语料缺口 docs/ISSUES.md E3，色彩口径家族见 C16）。
    pub fn for_canvas(height: u32) -> ColorMatrix {
        if height <= 576 {
            ColorMatrix::Bt601
        } else {
            ColorMatrix::Bt709
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteEntry {
    pub index: u8,
    pub y: u8,
    pub cr: u8,
    pub cb: u8,
    pub alpha: u8,
}

impl PaletteEntry {
    pub fn transparent(index: u8) -> PaletteEntry {
        PaletteEntry {
            index,
            y: 0,
            cr: 0x80,
            cb: 0x80,
            alpha: 0,
        }
    }

    pub fn to_rgba(&self, m: ColorMatrix) -> [u8; 4] {
        // limited range 逆矩阵（ITU-R 整数形式）：Y 减 16 后按 256 定标
        let y = (self.y as i32 - 16).max(0);
        let cb = self.cb as i32 - 128;
        let cr = self.cr as i32 - 128;
        let (r, g, b) = match m {
            ColorMatrix::Bt601 => (
                (298 * y + 409 * cr + 128) >> 8,
                (298 * y - 100 * cb - 208 * cr + 128) >> 8,
                (298 * y + 516 * cb + 128) >> 8,
            ),
            ColorMatrix::Bt709 => (
                (298 * y + 459 * cr + 128) >> 8,
                (298 * y - 54 * cb - 136 * cr + 128) >> 8,
                (298 * y + 541 * cb + 128) >> 8,
            ),
        };
        [
            r.clamp(0, 255) as u8,
            g.clamp(0, 255) as u8,
            b.clamp(0, 255) as u8,
            self.alpha,
        ]
    }

    pub fn from_rgba(rgba: [u8; 4], index: u8, m: ColorMatrix) -> PaletteEntry {
        let (r, g, b) = (rgba[0] as i32, rgba[1] as i32, rgba[2] as i32);
        let (y, cb, cr) = match m {
            ColorMatrix::Bt601 => (
                ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16,
                (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128),
                (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128),
            ),
            ColorMatrix::Bt709 => (
                ((47 * r + 157 * g + 16 * b + 128) >> 8) + 16,
                (((-25 * r - 85 * g + 110 * b + 128) >> 8) + 128),
                (((110 * r - 100 * g - 10 * b + 128) >> 8) + 128),
            ),
        };
        PaletteEntry {
            index,
            y: y.clamp(0, 255) as u8,
            cr: cr.clamp(0, 255) as u8,
            cb: cb.clamp(0, 255) as u8,
            alpha: rgba[3],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub id: u8,
    pub version: u8,
    pub entries: Vec<PaletteEntry>,
}

pub fn parse_pds(payload: &[u8]) -> Result<Palette> {
    if payload.len() < PDS_HEADER {
        return Err(StkError::simple(Domain::Pgs, 40, "PDS 少于 2 字节头"));
    }
    let body = &payload[PDS_HEADER..];
    if !body.len().is_multiple_of(PDS_ENTRY) {
        return Err(
            StkError::simple(Domain::Pgs, 41, "PDS 条目长度不是 5 的整数倍")
                .with_detail(format!("body {} bytes", body.len())),
        );
    }
    let mut entries = Vec::with_capacity(body.len() / PDS_ENTRY);
    let mut off = 0usize;
    while off + PDS_ENTRY <= body.len() {
        let e = &body[off..off + PDS_ENTRY];
        off += PDS_ENTRY;
        entries.push(PaletteEntry {
            index: e[0],
            y: e[1],
            cr: e[2],
            cb: e[3],
            alpha: e[4],
        });
    }
    Ok(Palette {
        id: payload[0],
        version: payload[1],
        entries,
    })
}

pub fn write_pds(p: &Palette) -> Result<Vec<u8>> {
    if p.entries.len() > 256 {
        return Err(StkError::simple(Domain::Pgs, 42, "调色板条目超过 256"));
    }
    let mut v = Vec::with_capacity(PDS_HEADER + p.entries.len() * PDS_ENTRY);
    v.push(p.id);
    v.push(p.version);
    for e in &p.entries {
        v.push(e.index);
        v.push(e.y);
        v.push(e.cr);
        v.push(e.cb);
        v.push(e.alpha);
    }
    Ok(v)
}

/// Limited-range Y 标量亮度（13 §3.3 的 "Brightness (Luma only)"）。
/// 只改 Y 字节，chroma 与 alpha 不动 —— 对彩色字幕这是近似，UI 必须如此标注。
pub fn luma_scalar(payload: &[u8], basis_points: i32) -> Result<Vec<u8>> {
    if !(1..=10_000).contains(&basis_points) {
        return Err(StkError::simple(
            Domain::Transform,
            50,
            "亮度系数必须在 1%..100%（万分比）",
        ));
    }
    let mut p = parse_pds(payload)?;
    for e in &mut p.entries {
        let v = (e.y as i32 * basis_points + 5000) / 10_000; // half-up
        e.y = v.clamp(16, 235) as u8;
    }
    write_pds(&p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pds_roundtrip_byte_identical() {
        let p = Palette {
            id: 0,
            version: 0,
            entries: vec![
                PaletteEntry::transparent(0),
                PaletteEntry {
                    index: 1,
                    y: 235,
                    cr: 128,
                    cb: 128,
                    alpha: 255,
                },
                PaletteEntry {
                    index: 2,
                    y: 81,
                    cr: 90,
                    cb: 240,
                    alpha: 200,
                },
            ],
        };
        let bytes = write_pds(&p).unwrap();
        assert_eq!(bytes.len(), 2 + 15);
        assert_eq!(parse_pds(&bytes).unwrap(), p);
    }

    #[test]
    fn pds_rejects_bad_length() {
        assert_eq!(
            parse_pds(&[0, 0, 1, 2, 3]).unwrap_err().code_string(),
            "STK-PGS-0041"
        );
        assert!(parse_pds(&[0]).is_err());
        let many = Palette {
            id: 0,
            version: 0,
            entries: (0..=255u8)
                .map(PaletteEntry::transparent)
                .chain(std::iter::once(PaletteEntry::transparent(0)))
                .collect(),
        };
        assert_eq!(many.entries.len(), 257);
        assert!(write_pds(&many).is_err());
    }

    #[test]
    fn white_and_black_map_through_matrix() {
        // limited range：Y=235 白、Y=16 黑，chroma 128
        let white = PaletteEntry {
            index: 1,
            y: 235,
            cr: 128,
            cb: 128,
            alpha: 255,
        };
        let black = PaletteEntry {
            index: 2,
            y: 16,
            cr: 128,
            cb: 128,
            alpha: 255,
        };
        let w = white.to_rgba(ColorMatrix::Bt709);
        let b = black.to_rgba(ColorMatrix::Bt709);
        assert!(w[0] > 230 && w[1] > 230 && w[2] > 230, "白 → {w:?}");
        assert!(b[0] < 25 && b[1] < 25 && b[2] < 25, "黑 → {b:?}");
        assert_eq!(white.alpha, w[3]);
    }

    #[test]
    fn rgb_ycbcr_roundtrip_stays_close() {
        // 整数矩阵必然带舍入误差；这里的阈值只保证"没有量级错误"。
        // 真正的矩阵/范围选择要用真盘语料与播放器差分确认（docs/ISSUES.md E3；范围问题记在 C16）。
        for m in [ColorMatrix::Bt601, ColorMatrix::Bt709] {
            for (r, g, b) in [
                (255, 255, 255),
                (0, 0, 0),
                (255, 0, 0),
                (0, 255, 0),
                (0, 0, 255),
                (18, 40, 200),
                (128, 128, 128),
            ] {
                let e = PaletteEntry::from_rgba([r, g, b, 255], 7, m);
                let back = e.to_rgba(m);
                let d = (back[0] as i32 - r as i32)
                    .abs()
                    .max((back[1] as i32 - g as i32).abs())
                    .max((back[2] as i32 - b as i32).abs());
                assert!(d <= 6, "{m:?} {r},{g},{b} -> {back:?} 误差 {d}");
            }
        }
    }

    #[test]
    fn limited_range_extremes_map_to_16_and_235() {
        for m in [ColorMatrix::Bt601, ColorMatrix::Bt709] {
            let white = PaletteEntry::from_rgba([255, 255, 255, 255], 1, m);
            let black = PaletteEntry::from_rgba([0, 0, 0, 255], 2, m);
            assert_eq!(white.y, 235, "{m:?} 白色 Y 必须是 limited range 上限");
            assert_eq!(black.y, 16, "{m:?} 黑色 Y 必须是 limited range 下限");
            assert_eq!(white.cr, 128);
            assert_eq!(white.cb, 128);
        }
    }

    #[test]
    fn matrix_choice_follows_canvas_height() {
        assert_eq!(ColorMatrix::for_canvas(480), ColorMatrix::Bt601);
        assert_eq!(ColorMatrix::for_canvas(576), ColorMatrix::Bt601);
        assert_eq!(ColorMatrix::for_canvas(720), ColorMatrix::Bt709);
        assert_eq!(ColorMatrix::for_canvas(1080), ColorMatrix::Bt709);
        assert_eq!(ColorMatrix::for_canvas(2160), ColorMatrix::Bt709);
    }

    #[test]
    fn luma_scalar_only_touches_y_and_clamps() {
        let p = Palette {
            id: 0,
            version: 1,
            entries: vec![
                PaletteEntry {
                    index: 0,
                    y: 235,
                    cr: 100,
                    cb: 200,
                    alpha: 255,
                },
                PaletteEntry {
                    index: 1,
                    y: 20,
                    cr: 90,
                    cb: 120,
                    alpha: 128,
                },
            ],
        };
        let before = write_pds(&p).unwrap();
        let after = luma_scalar(&before, 5000).unwrap();
        let pa = parse_pds(&after).unwrap();
        assert_eq!(pa.entries[0].y, 118); // 235*0.5 = 117.5 → half-up 118
        assert_eq!(pa.entries[1].y, 16, "低于 16 必须钳到 limited range 下限");
        assert_eq!(pa.entries[0].cr, 100);
        assert_eq!(pa.entries[0].cb, 200);
        assert_eq!(pa.entries[1].alpha, 128, "alpha 不动");
        assert_eq!(pa.id, 0);
        assert_eq!(pa.version, 1);
        assert_ne!(before, after);
        // 100% 系数必须等价于原字节
        assert_eq!(luma_scalar(&before, 10_000).unwrap(), before);
        assert!(luma_scalar(&before, 0).is_err());
        assert!(luma_scalar(&before, 10_001).is_err());
    }

    #[test]
    fn transparent_entry_is_canonical() {
        let t = PaletteEntry::transparent(3);
        assert_eq!((t.y, t.cr, t.cb, t.alpha), (0, 0x80, 0x80, 0));
        let rgba = t.to_rgba(ColorMatrix::Bt601);
        assert_eq!(rgba[3], 0, "透明条目 alpha 必须为 0");
    }
}
