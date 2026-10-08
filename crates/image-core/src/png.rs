//! PNG（13 §7 BDN 交换 / 09 §2 输出位图）。
//!
//! 写入固定 RGBA8（color type 6），IDAT 走**真 deflate 压缩**：旧工程写 stored（未压缩）块
//! 导致体积失控（TRACEABILITY I001/B001），所以压缩不自己实现，交给 flate2
//! （MIT/Apache-2.0 + miniz_oxide，符合 21 §1）。
//!
//! 读取支持 color type 0/3 的 1/2/4/8 位、color type 2/4/6 的 8 位，加全部 5 种 filter。
//! **16 位、隔行、tRNS 颜色键是策略性拒绝**（位深转换要先进色彩管线，不能默默 8 化），
//! 见 docs/history/2026-10-06/ISSUES.md。

use core_model::error::{Domain, Result, StkError};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{Read, Write};

use crate::rgba::{pixel_len, Frame, Patch};

pub const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// IDAT 解压后的容量上限：2^27 ≈ 134 MB。渲染管线的单张位图远小于此，
/// 超出即按不可信输入拒绝（23 §健壮性），不做"先分配再说"。
pub const MAX_DECOMPRESSED_BYTES: usize = 1 << 27;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Fast,
    Balanced,
    Max,
}

impl Level {
    pub fn compression(self) -> Compression {
        match self {
            Level::Fast => Compression::new(1),
            Level::Balanced => Compression::new(6),
            Level::Max => Compression::new(9),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub color_type: u8,
    pub interlace: u8,
}

/// 解码结果：始终是 8 位 RGBA，行主序。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// 原文件的 color type：非 6 时调用方要知道自己吃进了一个转换过的来源。
    pub source_color_type: u8,
}

fn err(code: u16, title: &str) -> StkError {
    StkError::simple(Domain::Image, code, title)
}

fn io_err(e: std::io::Error, code: u16, title: &str) -> StkError {
    err(code, title).with_detail(e.to_string())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            // 反射 CRC-32：每步只移 1 位（写成 >>8 会得到自洽但错误的校验值，
            // 自家 reader 察觉不到——Pillow 交叉验证就是在这里抓到的）
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn channels(color_type: u8) -> Result<usize> {
    match color_type {
        0 => Ok(1),
        2 => Ok(3),
        3 => Ok(1),
        4 => Ok(2),
        6 => Ok(4),
        other => Err(err(6, "PNG color type 不受支持（读 0/2/3/4/6，写只产 6）")
            .with_detail(format!("color type {other}"))),
    }
}

/// 只放渲染管线真的能无损读回的组合；16 位需要位深转换策略，直接拒绝。
/// 子字节灰度/调色板按位复制到 8 位（0/1/2/4 位是纯打包，不涉及色彩解释）。
fn check_depth(color_type: u8, depth: u8) -> Result<()> {
    let allowed: &[u8] = match color_type {
        0 => &[1, 2, 4, 8],
        3 => &[1, 2, 4, 8],
        2 | 4 | 6 => &[8],
        _ => &[8],
    };
    if depth == 16 {
        return Err(err(
            6,
            "不支持 16 位 PNG：位深转换属于色彩管线职责，未定策略前不静默降位",
        ));
    }
    if !allowed.contains(&depth) {
        return Err(err(6, "PNG bit depth 与 color type 组合不受支持")
            .with_detail(format!("color type {color_type} 允许 {allowed:?}")));
    }
    Ok(())
}

impl Header {
    /// 一行的原始字节数（不含 filter 字节）。
    fn row_bytes(&self) -> Result<usize> {
        let bits = channels(self.color_type)?
            .checked_mul(usize::from(self.bit_depth))
            .ok_or_else(|| err(9, "PNG 通道位宽溢出"))?;
        let w = usize::try_from(self.width).map_err(|_| err(9, "PNG 宽度超出可处理范围"))?;
        let total = w
            .checked_mul(bits)
            .ok_or_else(|| err(9, "PNG 行位宽溢出"))?;
        Ok(total.div_ceil(8))
    }
}

// ---------------------------------------------------------------- 写入

fn paeth(a: i32, b: i32, c: i32) -> i32 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// libpng 同款启发式：过滤残差按有符号看，绝对值之和越小越好压。
fn filter_cost(buf: &[u8]) -> u64 {
    buf.iter()
        .map(|&v| {
            let signed = i32::from(v) - if v > 127 { 256 } else { 0 };
            u64::from(signed.unsigned_abs())
        })
        .sum()
}

fn apply_filter(tag: u8, src: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.push(tag);
    for (i, &v) in src.iter().enumerate() {
        let left = if i >= bpp { src[i - bpp] } else { 0 };
        let up = prev[i];
        let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
        let pred: u8 = match tag {
            0 => 0,
            1 => left,
            2 => up,
            3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
            _ => paeth(i32::from(left), i32::from(up), i32::from(up_left)) as u8,
        };
        out.push(v.wrapping_sub(pred));
    }
}

/// 逐行挑代价最小的 filter，平局取编号小的。返回 `(filter 字节 + 残差)` 的整块。
fn choose_filters(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let bpp = 4usize;
    let row = width * bpp;
    let mut filtered = Vec::with_capacity((row + 1) * height);
    let mut prev = vec![0u8; row];
    let mut cand = Vec::with_capacity(row + 1);
    let mut best = Vec::with_capacity(row + 1);
    for y in 0..height {
        let src = &rgba[y * row..(y + 1) * row];
        let mut best_cost = u64::MAX;
        for tag in 0..5u8 {
            cand.clear();
            apply_filter(tag, src, &prev, bpp, &mut cand);
            let cost = filter_cost(&cand[1..]);
            if cost < best_cost {
                best_cost = cost;
                // 平局取编号小的 filter：严格小于才换，保证输出可复现
                std::mem::swap(&mut best, &mut cand);
            }
        }
        filtered.extend_from_slice(&best);
        prev.copy_from_slice(src);
    }
    filtered
}

fn ihdr_bytes(width: u32, height: u32) -> [u8; 13] {
    let mut b = [0u8; 13];
    b[0..4].copy_from_slice(&width.to_be_bytes());
    b[4..8].copy_from_slice(&height.to_be_bytes());
    b[8] = 8; // bit depth
    b[9] = 6; // color type: RGBA
    b[10] = 0; // compression: deflate
    b[11] = 0; // filter: adaptive
    b[12] = 0; // interlace: none
    b
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) -> Result<()> {
    let len = u32::try_from(data.len()).map_err(|_| err(9, "PNG chunk 超过 4 GiB"))?;
    out.extend_from_slice(&len.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
    Ok(())
}

/// RGBA8 → PNG。`rgba.len()` 必须等于 `width*height*4`。
pub fn encode(rgba: &[u8], width: u32, height: u32, level: Level) -> Result<Vec<u8>> {
    let need = pixel_len(width, height)?
        .checked_mul(4)
        .ok_or_else(|| err(9, "PNG 像素字节数溢出"))?;
    if rgba.len() != need {
        return Err(
            err(2, "RGBA 长度与尺寸不符").with_detail(format!("需要 {need}，得到 {}", rgba.len()))
        );
    }
    let filtered = choose_filters(
        rgba,
        usize::try_from(width).map_err(|_| err(9, "PNG 宽度过大"))?,
        usize::try_from(height).map_err(|_| err(9, "PNG 高度过大"))?,
    );
    let mut enc = ZlibEncoder::new(Vec::new(), level.compression());
    enc.write_all(&filtered)
        .map_err(|e| io_err(e, 10, "PNG IDAT 压缩失败"))?;
    let idat = enc
        .finish()
        .map_err(|e| io_err(e, 10, "PNG IDAT 压缩收尾失败"))?;

    let mut out = Vec::with_capacity(idat.len() + 64);
    out.extend_from_slice(&SIGNATURE);
    write_chunk(&mut out, b"IHDR", &ihdr_bytes(width, height))?;
    write_chunk(&mut out, b"IDAT", &idat)?;
    write_chunk(&mut out, b"IEND", &[])?;
    Ok(out)
}

impl Patch {
    /// 局部位图 → PNG。IHDR 尺寸就是位图尺寸，BDN 的 Graphic Width/Height 只能取它
    /// （13 §7 / AT-BDN-001：声明尺寸必须等于 PNG 实际尺寸）。
    pub fn to_png(&self, level: Level) -> Result<Vec<u8>> {
        encode(&self.rgba, self.width, self.height, level)
    }
}

impl Frame {
    pub fn to_png(&self, level: Level) -> Result<Vec<u8>> {
        encode(&self.rgba, self.canvas.width, self.canvas.height, level)
    }
}

// ---------------------------------------------------------------- 读取

/// 只解析 IHDR（不解压 IDAT），给尺寸一致性检查用。
pub fn header(png: &[u8]) -> Result<Header> {
    if png.len() < 33 || png[..8] != SIGNATURE {
        return Err(
            err(5, "不是 PNG：签名缺失或头部被截断").with_detail(format!("长度 {}", png.len()))
        );
    }
    let len = be_u32(png, 8)?;
    if &png[12..16] != b"IHDR" {
        return Err(err(5, "PNG 首个 chunk 不是 IHDR").with_detail(hex4(&png[12..16])));
    }
    if len != 13 {
        return Err(err(5, "PNG IHDR 长度非法").with_detail(format!("声明 {len}，规范要求 13")));
    }
    Ok(parse_ihdr(&png[16..29]))
}

fn parse_ihdr(d: &[u8]) -> Header {
    Header {
        width: u32::from_be_bytes([d[0], d[1], d[2], d[3]]),
        height: u32::from_be_bytes([d[4], d[5], d[6], d[7]]),
        bit_depth: d[8],
        color_type: d[9],
        interlace: d[12],
    }
}

fn hex4(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn be_u32(b: &[u8], at: usize) -> Result<u32> {
    let end = at.checked_add(4).ok_or_else(|| err(5, "PNG 偏移溢出"))?;
    if b.len() < end {
        return Err(err(5, "PNG 数据在 32 位字段处截断").with_detail(format!("需要 {end} 字节")));
    }
    Ok(u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]))
}

/// 所有 IDAT 串起来是一个 zlib 流；解压容量按 IHDR 精确限定。
fn inflate(idat: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut dec = ZlibDecoder::new(idat);
    let mut out = Vec::with_capacity(expected.min(1 << 16));
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = dec
            .read(&mut buf)
            .map_err(|e| io_err(e, 10, "PNG IDAT 解压失败"))?;
        if n == 0 {
            break;
        }
        if out.len() + n > expected {
            return Err(err(9, "PNG IDAT 解压超过 IHDR 声明容量（压缩炸弹保护）")
                .with_detail(format!("声明容量 {expected}")));
        }
        out.extend_from_slice(&buf[..n]);
    }
    if out.len() != expected {
        return Err(err(7, "PNG IDAT 解压长度与 IHDR 声明不符")
            .with_detail(format!("期望 {expected}，得到 {}", out.len())));
    }
    Ok(out)
}

fn unfilter(raw: &[u8], height: usize, row_bytes: usize, bpp: usize) -> Result<Vec<u8>> {
    if raw.len() != (row_bytes + 1) * height {
        return Err(err(7, "PNG 过滤后数据长度与 IHDR 不符"));
    }
    let mut cur = vec![0u8; row_bytes];
    let mut prev = vec![0u8; row_bytes];
    let mut out = Vec::with_capacity(row_bytes * height);
    for y in 0..height {
        let line = &raw[y * (row_bytes + 1)..(y + 1) * (row_bytes + 1)];
        let tag = line[0];
        if tag > 4 {
            return Err(err(6, "PNG filter type 非法").with_detail(format!("行 {y} tag {tag}")));
        }
        for (i, &v) in line[1..].iter().enumerate() {
            let left = if i >= bpp { cur[i - bpp] } else { 0 };
            let up = prev[i];
            let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
            let pred: u8 = match tag {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                _ => paeth(i32::from(left), i32::from(up), i32::from(up_left)) as u8,
            };
            cur[i] = v.wrapping_add(pred);
        }
        out.extend_from_slice(&cur);
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(out)
}

/// 灰度按字节原样复制到 RGB：不做 gamma 线性化。位图扩展必须逐字节无损，
/// 任何色彩解释的变化都要由调用方显式声明（11 §色彩）。
fn expand_to_rgba(rows: &[u8], hdr: &Header, palette: &[u8], trns: &[u8]) -> Result<Vec<u8>> {
    let w = usize::try_from(hdr.width).map_err(|_| err(9, "PNG 宽度过大"))?;
    let h = usize::try_from(hdr.height).map_err(|_| err(9, "PNG 高度过大"))?;
    let row_bytes = hdr.row_bytes()?;
    let mut rgba = Vec::with_capacity(
        w.checked_mul(h)
            .and_then(|x| x.checked_mul(4))
            .ok_or_else(|| err(9, "PNG 容量溢出"))?,
    );
    for y in 0..h {
        let row = &rows[y * row_bytes..(y + 1) * row_bytes];
        match hdr.color_type {
            6 => rgba.extend_from_slice(row),
            2 => {
                for px in row.chunks_exact(3) {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            4 => {
                for px in row.chunks_exact(2) {
                    rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
            }
            0 => {
                for &s in &unpack_samples(row, hdr.bit_depth, w)? {
                    let g = scale_sub_byte(s, hdr.bit_depth);
                    rgba.extend_from_slice(&[g, g, g, 255]);
                }
            }
            3 => {
                for idx in unpack_samples(row, hdr.bit_depth, w)? {
                    let base = usize::from(idx) * 3;
                    if base + 3 > palette.len() {
                        return Err(err(7, "PNG 调色板项缺失").with_detail(format!("index {idx}")));
                    }
                    let a = if usize::from(idx) < trns.len() {
                        trns[usize::from(idx)]
                    } else {
                        255
                    };
                    rgba.extend_from_slice(&[
                        palette[base],
                        palette[base + 1],
                        palette[base + 2],
                        a,
                    ]);
                }
            }
            _ => unreachable!("color type 已在 channels() 校验"),
        }
    }
    Ok(rgba)
}

/// 子字节样本复制到 8 位：按 PNG 规范做整数放大（等价于位复制），不涉及 gamma。
fn scale_sub_byte(v: u8, depth: u8) -> u8 {
    match depth {
        1 => u8::from(v != 0).saturating_mul(255),
        2 => v * 85,
        4 => v * 17,
        _ => v,
    }
}

fn unpack_samples(row: &[u8], depth: u8, width: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(width);
    match depth {
        8 => out.extend_from_slice(row),
        4 => {
            for b in row {
                out.push(b >> 4);
                out.push(b & 0x0F);
            }
        }
        2 => {
            for b in row {
                out.push(b >> 6);
                out.push((b >> 4) & 3);
                out.push((b >> 2) & 3);
                out.push(b & 3);
            }
        }
        1 => {
            for b in row {
                for s in (0..8).rev() {
                    out.push((b >> s) & 1);
                }
            }
        }
        other => return Err(err(6, "PNG 调色板位深非法").with_detail(format!("{other}"))),
    }
    if out.len() < width {
        return Err(
            err(7, "PNG 索引像素数少于宽度").with_detail(format!("得到 {}，宽 {width}", out.len()))
        );
    }
    out.truncate(width);
    Ok(out)
}

/// 读 PNG → 8 位 RGBA。
pub fn decode(png: &[u8]) -> Result<Image> {
    let hdr = header(png)?;
    if hdr.width == 0 || hdr.height == 0 {
        return Err(
            err(5, "PNG IHDR 尺寸不合理").with_detail(format!("{}x{}", hdr.width, hdr.height))
        );
    }
    if hdr.interlace != 0 {
        return Err(err(
            6,
            "不支持隔行 PNG（Adam7）：局部位图与逐行 filter 假设不成立",
        ));
    }
    check_depth(hdr.color_type, hdr.bit_depth)?;
    // filter 的位移基准按整字节像素；子字节图像是 1
    let bpp = channels(hdr.color_type)?;
    let row_bytes = hdr.row_bytes()?;
    let h = usize::try_from(hdr.height).map_err(|_| err(9, "PNG 高度过大"))?;
    let expected = (row_bytes + 1)
        .checked_mul(h)
        .ok_or_else(|| err(9, "PNG 容量溢出"))?;
    if expected > MAX_DECOMPRESSED_BYTES {
        return Err(err(9, "PNG 解压容量超过保护上限")
            .with_detail(format!("需要 {expected}，上限 {MAX_DECOMPRESSED_BYTES}")));
    }

    let mut pos = 8usize;
    let mut idat: Vec<u8> = Vec::new();
    let mut palette: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut ihdr_count = 0usize;
    let mut done = false;
    while pos + 12 <= png.len() {
        if done {
            return Err(err(5, "PNG IEND 之后还有数据"));
        }
        let len = usize::try_from(be_u32(png, pos)?).map_err(|_| err(9, "PNG chunk 长度过大"))?;
        let kind = [png[pos + 4], png[pos + 5], png[pos + 6], png[pos + 7]];
        let data_start = pos + 8;
        let data_end = data_start
            .checked_add(len)
            .ok_or_else(|| err(9, "PNG chunk 长度溢出"))?;
        if data_end + 4 > png.len() {
            return Err(err(5, "PNG chunk 被截断").with_detail(hex4(&kind)));
        }
        let data = &png[data_start..data_end];
        let want = be_u32(png, data_end)?;
        let got = crc32(&png[pos + 4..data_end]);
        if want != got {
            return Err(err(8, "PNG chunk CRC 校验失败")
                .with_detail(format!("{} 声明 {want:08X}，实算 {got:08X}", hex4(&kind))));
        }
        match &kind {
            b"IHDR" => ihdr_count += 1,
            b"PLTE" => palette.extend_from_slice(data),
            b"tRNS" => trns.extend_from_slice(data),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => done = true,
            // gAMA/sRGB/pHYs/tEXt 等：09 §2 规定不自动应用色彩标签，忽略但保留在文件里
            _ => {}
        }
        pos = data_end + 4;
    }
    if ihdr_count != 1 {
        return Err(err(5, "PNG IHDR 数量非法").with_detail(format!("发现 {ihdr_count} 个")));
    }
    if !done {
        return Err(err(5, "PNG 缺少 IEND"));
    }
    if pos != png.len() {
        return Err(err(5, "PNG IEND 之后还有数据")
            .with_detail(format!("IEND 结束于 {pos}，文件长 {}", png.len())));
    }
    if idat.is_empty() {
        return Err(err(5, "PNG 没有 IDAT"));
    }
    if hdr.color_type == 3 && palette.is_empty() {
        return Err(err(7, "PNG 调色板图像缺少 PLTE"));
    }
    if palette.len() % 3 != 0 {
        return Err(err(7, "PNG PLTE 长度不是 3 的倍数"));
    }
    if !trns.is_empty() && hdr.color_type != 3 {
        return Err(err(6, "不支持 tRNS 颜色键（Gray/RGB）：只支持调色板透明"));
    }
    let raw = inflate(&idat, expected)?;
    let rows = unfilter(&raw, h, row_bytes, bpp)?;
    let rgba = expand_to_rgba(&rows, &hdr, &palette, &trns)?;
    Ok(Image {
        width: hdr.width,
        height: hdr.height,
        source_color_type: hdr.color_type,
        rgba,
    })
}

impl Image {
    pub fn to_patch(&self, x: u32, y: u32) -> Result<Patch> {
        Ok(Patch {
            width: self.width,
            height: self.height,
            x,
            y,
            rgba: self.rgba.clone(),
        })
    }

    pub fn to_frame(&self, canvas: core_model::Canvas) -> Result<Frame> {
        if canvas.width != self.width || canvas.height != self.height {
            return Err(err(2, "PNG 尺寸与目标画布不符").with_detail(format!(
                "PNG {}x{} 画布 {}",
                self.width,
                self.height,
                canvas.label()
            )));
        }
        Frame::new(canvas, self.rgba.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                v.push(((x * 7 + y * 3) % 256) as u8);
                v.push(((y * 5) % 256) as u8);
                v.push(((x + y) % 256) as u8);
                v.push(if (x + y) % 3 == 0 {
                    0
                } else {
                    200 + (x % 55) as u8
                });
            }
        }
        v
    }

    fn synthetic(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                // 模拟字幕位图：大片全透明 + 一条实心带
                let in_band = (40..60).contains(&x) && (10..20).contains(&y);
                if in_band {
                    v.extend_from_slice(&[255, 255, 255, 255]);
                } else {
                    v.extend_from_slice(&[0, 0, 0, 0]);
                }
            }
        }
        v
    }

    #[test]
    fn roundtrip_is_pixel_exact() {
        for (w, h) in [(1, 1), (3, 7), (64, 33), (255, 2)] {
            let src = gradient(w, h);
            let png = encode(&src, w, h, Level::Balanced).unwrap();
            let back = decode(&png).unwrap();
            assert_eq!((back.width, back.height), (w, h));
            assert_eq!(back.rgba, src, "{w}x{h} RGBA 往返必须逐像素相等");
            assert_eq!(back.source_color_type, 6);
        }
    }

    #[test]
    fn idat_is_really_compressed_not_stored() {
        // 旧工程缺陷：stored 块让 PNG 体积≈原始 RGBA（I001/B001）。
        let (w, h) = (640, 360);
        let src = vec![0u8; (w as usize * h as usize) * 4];
        let png = encode(&src, w, h, Level::Max).unwrap();
        assert!(
            png.len() * 100 < src.len(),
            "全透明 {w}x{h} 应压到原始大小的 1% 以下，实际 {} vs {}",
            png.len(),
            src.len()
        );
        let band = synthetic(320, 240);
        let png2 = encode(&band, 320, 240, Level::Balanced).unwrap();
        assert!(
            png2.len() * 10 < band.len(),
            "字幕式位图至少压缩 10 倍，实际 {} vs {}",
            png2.len(),
            band.len()
        );
        assert_eq!(decode(&png2).unwrap().rgba, band);
    }

    #[test]
    fn filter_choice_is_per_row_and_deterministic() {
        // 行 0：全 20 -> Sub 最优（None 成本 8*20=160，Sub 残差 4*20=80）
        // 行 1：与行 0 相同 -> Up 残差全 0，成本 0
        // 行 2：全 0 -> None 成本 0
        let rgba: Vec<u8> = [vec![20u8; 8], vec![20u8; 8], vec![0u8; 8]].concat();
        let filtered = choose_filters(&rgba, 2, 3);
        let tags: Vec<u8> = filtered.iter().step_by(9).copied().collect();
        assert_eq!(
            tags,
            vec![1, 2, 0],
            "逐行 filter 必须按代价选，不平局时写死 None"
        );

        // 常数行第一行不会是 None
        let one = choose_filters(&[33u8; 8], 2, 1);
        assert_eq!(one[0], 1);
    }

    #[test]
    fn rejects_corrupt_crc_and_truncation() {
        let png = encode(&gradient(8, 8), 8, 8, Level::Fast).unwrap();
        let mut bad = png.clone();
        let last = bad.len() - 5;
        bad[last] ^= 0xFF; // IEND 的 CRC 字节
        assert!(matches!(
            decode(&bad),
            Err(e) if e.code_string() == "STK-IMG-0008"
        ));

        assert!(matches!(
            decode(&png[..png.len() - 6]),
            Err(e) if e.code_string() == "STK-IMG-0005"
        ));

        let mut no_sig = png.clone();
        no_sig[1] = b'X';
        assert!(matches!(
            header(&no_sig),
            Err(e) if e.code_string() == "STK-IMG-0005"
        ));

        let mut extra = png.clone();
        extra.extend_from_slice(&[0u8; 4]);
        assert!(matches!(
            decode(&extra),
            Err(e) if e.code_string() == "STK-IMG-0005"
        ));
    }

    #[test]
    fn rejects_interlace_16bit_and_unknown_color_type() {
        let mut png = encode(&gradient(4, 4), 4, 4, Level::Fast).unwrap();
        png[16 + 12] = 1; // interlace
        let e = decode(&png).unwrap_err();
        assert_eq!(e.code_string(), "STK-IMG-0006");
        png[16 + 12] = 0;

        png[16 + 8] = 16; // bit depth
        assert!(matches!(
            decode(&png),
            Err(e) if e.code_string() == "STK-IMG-0006"
        ));
        png[16 + 8] = 8;

        png[16 + 9] = 5; // 未定义的 color type
        assert!(matches!(
            decode(&png),
            Err(e) if e.code_string() == "STK-IMG-0006"
        ));
        // 改过头部就要重新算 CRC，否则先撞 0008；这里只验证判定顺序，不再复用
    }

    #[test]
    fn rejects_decompression_bomb() {
        // IHDR 说 2x2，IDAT 里塞一个更大的合法 zlib 流
        let big = vec![7u8; (2 * 4 + 1) * 2 + 4096];
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::new(1));
        enc.write_all(&big).unwrap();
        let idat = enc.finish().unwrap();
        let mut out = Vec::new();
        out.extend_from_slice(&SIGNATURE);
        write_chunk(&mut out, b"IHDR", &ihdr_bytes(2, 2)).unwrap();
        write_chunk(&mut out, b"IDAT", &idat).unwrap();
        write_chunk(&mut out, b"IEND", &[]).unwrap();
        assert!(matches!(
            decode(&out),
            Err(e) if e.code_string() == "STK-IMG-0009"
        ));
    }

    #[test]
    fn decodes_palette_gray_and_rgb_from_other_writers() {
        // 2x2 调色板 PNG（color type 3, depth 4）+ tRNS
        let mut png = Vec::new();
        png.extend_from_slice(&SIGNATURE);
        let mut ih = [0u8; 13];
        ih[0..4].copy_from_slice(&2u32.to_be_bytes());
        ih[4..8].copy_from_slice(&2u32.to_be_bytes());
        ih[8] = 4;
        ih[9] = 3;
        write_chunk(&mut png, b"IHDR", &ih).unwrap();
        write_chunk(&mut png, b"PLTE", &[255, 0, 0, 0, 255, 0]).unwrap();
        write_chunk(&mut png, b"tRNS", &[0, 255]).unwrap();
        let mut e = ZlibEncoder::new(Vec::new(), Compression::new(6));
        e.write_all(&[0u8, 0x01, 0u8, 0x10]).unwrap();
        let idat = e.finish().unwrap();
        write_chunk(&mut png, b"IDAT", &idat).unwrap();
        write_chunk(&mut png, b"IEND", &[]).unwrap();

        let img = decode(&png).unwrap();
        assert_eq!(img.width, 2);
        assert_eq!(
            img.rgba,
            vec![
                255, 0, 0, 0, // 行 0 高 4 位 = 索引 0，tRNS[0]=0 -> 透明
                0, 255, 0, 255, // 行 0 低 4 位 = 索引 1
                0, 255, 0, 255, // 行 1 高 4 位 = 索引 1
                255, 0, 0, 0, // 行 1 低 4 位 = 索引 0
            ],
            "tRNS 必须作用到调色板索引上"
        );

        // 灰度 color type 0 depth 8
        let mut png2 = Vec::new();
        png2.extend_from_slice(&SIGNATURE);
        let mut ih2 = [0u8; 13];
        ih2[0..4].copy_from_slice(&2u32.to_be_bytes());
        ih2[4..8].copy_from_slice(&2u32.to_be_bytes());
        ih2[8] = 8;
        ih2[9] = 0;
        write_chunk(&mut png2, b"IHDR", &ih2).unwrap();
        let mut e2 = ZlibEncoder::new(Vec::new(), Compression::new(6));
        e2.write_all(&[0u8, 17, 18, 0u8, 19, 20]).unwrap();
        write_chunk(&mut png2, b"IDAT", &e2.finish().unwrap()).unwrap();
        write_chunk(&mut png2, b"IEND", &[]).unwrap();
        let img2 = decode(&png2).unwrap();
        assert_eq!(img2.rgba[0..4], [17, 17, 17, 255]);
        assert_eq!(img2.rgba[4..8], [18, 18, 18, 255]);
        assert_eq!(img2.rgba[12..16], [20, 20, 20, 255]);

        // RGBA 但用了 Paeth / Up filter：手工构造确保解 filter 的两个分支都对
        let mut png3 = Vec::new();
        png3.extend_from_slice(&SIGNATURE);
        write_chunk(&mut png3, b"IHDR", &ihdr_bytes(2, 2)).unwrap();
        let mut e3 = ZlibEncoder::new(Vec::new(), Compression::new(6));
        // 行 0 filter 4(Paeth)：上一行是全 0，首像素预测 0，次像素预测=左像素
        // 行 1 filter 2(Up)：预测值就是上一行解码结果
        e3.write_all(&[
            4, 10, 20, 30, 40, 5, 5, 5, 5, // -> [10,20,30,40] [15,25,35,45]
            2, 1, 1, 1, 1, 2, 2, 2, 2,
        ]) // -> [11,21,31,41] [17,27,37,47]
        .unwrap();
        write_chunk(&mut png3, b"IDAT", &e3.finish().unwrap()).unwrap();
        write_chunk(&mut png3, b"IEND", &[]).unwrap();
        let img3 = decode(&png3).unwrap();
        assert_eq!(img3.rgba[0..4], [10, 20, 30, 40]);
        assert_eq!(img3.rgba[4..8], [15, 25, 35, 45]);
        assert_eq!(img3.rgba[8..12], [11, 21, 31, 41]);
        assert_eq!(img3.rgba[12..16], [17, 27, 37, 47]);
    }

    #[test]
    fn header_reads_size_without_decoding_pixels() {
        let png = encode(&gradient(9, 5), 9, 5, Level::Fast).unwrap();
        let h = header(&png).unwrap();
        assert_eq!(
            (h.width, h.height, h.bit_depth, h.color_type, h.interlace),
            (9, 5, 8, 6, 0)
        );
    }

    #[test]
    fn crc32_matches_official_check_values() {
        // CRC-32/ISO-HDLC 的公开 check 值：跟"自家实现"比是循环论证，比不过外部参考。
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        let mut ih = Vec::from(&b"IHDR"[..]);
        ih.extend_from_slice(&ihdr_bytes(8, 8));
        assert_eq!(crc32(&ih), 0xC40F_BE8B);
    }

    #[test]
    fn encode_rejects_length_mismatch() {
        assert!(matches!(
            encode(&[0u8; 7], 2, 2, Level::Fast),
            Err(e) if e.code_string() == "STK-IMG-0002"
        ));
    }
}
