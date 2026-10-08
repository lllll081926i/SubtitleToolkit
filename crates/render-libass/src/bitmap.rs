//! `ASS_Image` 链表 → 完整 16:9 RGBA 画布（12 §1A、09 §3）。
//!
//! 取证来源：`third_party/sources/libass-master.tar.gz`（libass 0.17.5，SHA-256 见
//! `SOURCE_AUDIT_LOCK.json`）。本模块的行为必须与下面四条一致：
//! - `libass/ass.h:53-56`："These images have to be rendered **in-order** for the correct
//!   screen composition"；"w/h can be zero, in this case the bitmap should not be
//!   rendered at all"；"**The last bitmap row is not guaranteed to be padded up to
//!   stride size**, e.g. in the worst case a bitmap has the size `stride * (h - 1) + w`"；
//! - `libass/ass.h:71`：`color` 是 RGBA 打包的 uint32；
//! - `libass/ass_render.c:3411-3417`：注释写着 "concat lists, removing fully transparent
//!   bitmaps"，被剔除的条件是 `_a(cur->color) == 0xFF`；
//! - `compare/compare.c:48-53`（libass 自带的渲染比对工具，即官方参考消费者）：
//!   `uint8_t a = img->color >> 0;` 之后立刻 `int32_t mul = 129 * (255 - a);`。
//!
//! 后两条合起来只有一种读法：**alpha 字节存的是 ASS 风格的"透明度"，不是直通 alpha**
//! （0x00 = 完全不透明，0xFF = 完全透明）。这是真实渲染才暴露的坑（ISSUES Z1）：
//! 按直通 alpha 解释时，一条正常字幕会画出全透明的空画布，而自家单元测试因为
//! 只喂自己构造的位图，永远发现不了。
//!
//! 这一层**不链接 libass**：它只处理已经拿到位图描述的数据。native 调用在 `ffi`
//! 里用运行时加载（12 §进程边界），所以即使 native 依赖还没落地，这里的合成语义也能测。

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use image_core::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    /// 字形本体
    Character,
    /// 描边
    Outline,
    /// 阴影
    Shadow,
}

/// 一张 libass 位图的**值拷贝描述**（不含裸指针），便于安全测试与复用。
#[derive(Clone, Copy, Debug)]
pub struct ImageDesc<'a> {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub bitmap: &'a [u8],
    /// RGBA 打包（ass.h:71）：r = color>>24, g = >>16, b = >>8；
    /// 最低字节是**透明度**，取值要再取反一次才是 alpha
    pub color: u32,
    /// 完整画布坐标（09 §3：不是相对 bbox）
    pub dst_x: i32,
    pub dst_y: i32,
    pub kind: ImageKind,
}

impl ImageDesc<'_> {
    /// 按头文件给的保证长度校验输入；不合规直接拒绝，不"尽力读"。
    pub fn check(&self) -> Result<()> {
        if self.width < 0 || self.height < 0 {
            return Err(err(
                60,
                "ASS_Image 尺寸不能为负",
                &format!("{}x{}", self.width, self.height),
            ));
        }
        if self.width == 0 || self.height == 0 {
            return Ok(()); // 头文件：这种位图直接不画
        }
        if self.stride < self.width {
            return Err(err(
                61,
                "ASS_Image stride 小于宽度",
                &format!("stride={} w={}", self.stride, self.width),
            )
            .with_action("按 stride 逐行读取会越界；这不该被容忍，而该上报 renderer 异常"));
        }
        let guaranteed = (self.stride as usize) * (self.height as usize - 1) + self.width as usize;
        if self.bitmap.len() < guaranteed {
            return Err(err(
                62,
                "ASS_Image 位图短于头文件保证的长度",
                &format!("得到 {}，需要 {guaranteed}", self.bitmap.len()),
            )
            .with_action("最后一行允许不补齐到 stride，但不能少于 stride*(h-1)+w"));
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

fn err(code: u16, title: &str, detail: &str) -> StkError {
    StkError::simple(Domain::Render, code, title).with_detail(detail)
}

/// RGBA 打包字拆包（libass 的 `color` 语义）。
///
/// 最低字节是"透明度"而不是 alpha，所以这里必须 `255 -`：`0x…00` 是全不透明，
/// `0x…FF` 是全透明（`ass_render.c:3416` 正是用 `_a(color)==0xFF` 判定"完全透明"
/// 并直接丢弃位图，`compare/compare.c:53` 用 `255 - a` 作为混合系数）。
pub fn unpack_color(color: u32) -> [u8; 4] {
    [
        (color >> 24) as u8,
        (color >> 16) as u8,
        (color >> 8) as u8,
        255 - (color as u8),
    ]
}

fn round_div(num: u64, den: u64) -> u64 {
    // 半进位：8 位 alpha 乘法不可避免要量化，调用方必须把这件事报出去
    (num + den / 2) / den
}

/// 把一组位图按链表顺序合成到完整画布。
///
/// `images` 的顺序就是 `ASS_Image.next` 的顺序：**颠倒顺序会得到不同的画面**
/// （阴影压在字上面），所以这里不排序、不并发，只按序叠加。
pub fn composite(canvas: Canvas, images: &[ImageDesc<'_>], loss: &mut LossReport) -> Result<Frame> {
    let mut frame = Frame::empty(canvas)?;
    let mut clipped = 0u64;
    let mut quantized = 0u64;

    for (i, img) in images.iter().enumerate() {
        img.check()?;
        if img.is_empty() {
            continue;
        }
        let [cr, cg, cb, ca] = unpack_color(img.color);
        let stride = img.stride as usize;
        for row in 0..img.height as usize {
            let y = img.dst_y as i64 + row as i64;
            if y < 0 || y >= canvas.height as i64 {
                clipped += img.width.max(0) as u64;
                continue;
            }
            // 最后一行允许不补齐：只取到该行真实可用的字节
            let start = row * stride;
            let avail = (img.bitmap.len() - start).min(img.width as usize);
            if avail == 0 {
                return Err(err(
                    63,
                    "ASS_Image 行起点越界",
                    &format!(
                        "images[{i}] row {row} start {start} len {}",
                        img.bitmap.len()
                    ),
                ));
            }
            for col in 0..avail {
                let x = img.dst_x as i64 + col as i64;
                if x < 0 || x >= canvas.width as i64 {
                    clipped += 1;
                    continue;
                }
                let coverage = img.bitmap[start + col];
                if coverage == 0 || ca == 0 {
                    continue;
                }
                // src alpha = 覆盖率 × 颜色 alpha，8 位量化（半进位）
                let sa = round_div(u64::from(coverage) * u64::from(ca), 255) as u32;
                if sa == 0 {
                    quantized += 1;
                    continue;
                }
                let d = ((y as u32 * canvas.width + x as u32) * 4) as usize;
                let da = frame.rgba[d + 3] as u32;
                let out_a = sa + da * (255 - sa) / 255;
                if out_a == 0 {
                    frame.rgba[d..d + 4].copy_from_slice(&[0, 0, 0, 0]);
                    continue;
                }
                for c in 0..3usize {
                    let src = match c {
                        0 => cr as u64,
                        1 => cg as u64,
                        _ => cb as u64,
                    };
                    let dst = frame.rgba[d + c] as u64;
                    let num =
                        src * u64::from(sa) * 255 + dst * u64::from(da) * (255 - u64::from(sa));
                    let den = u64::from(out_a) * 255;
                    frame.rgba[d + c] = round_div(num, den).min(255) as u8;
                }
                frame.rgba[d + 3] = out_a.min(255) as u8;
            }
        }
    }

    if quantized > 0 {
        loss.approximated(&format!(
            "libass 覆盖图与颜色 alpha 相乘后按 8 位量化，{quantized} 个像素被量化为全透明"
        ));
    }
    if clipped > 0 {
        loss.dropped(&format!(
            "{clipped} 个位图像素落在输出画布之外被裁掉（libass 本来会按 frame size 裁剪，这里再兜一层）"
        ));
    }
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::canvas::Canvas;

    fn small_canvas() -> Canvas {
        Canvas::new(8, 4).unwrap()
    }

    #[test]
    fn color_unpack_is_rgba_with_inverted_alpha() {
        // 头文件（ass.h:71）：color 是 "Bitmap color and alpha, RGBA"；
        // 但最低字节按 ASS 惯例是"透明度"，所以 alpha = 255 - 最低字节
        // （compare/compare.c:53 的 `129 * (255 - a)`、ass_render.c:3416 的 `_a==0xFF` 剔除）。
        assert_eq!(unpack_color(0xFF80_4020), [0xFF, 0x80, 0x40, 0xDF]);
        assert_eq!(
            unpack_color(0x0000_00FF),
            [0, 0, 0, 0],
            "透明度 0xFF = 全透明"
        );
        assert_eq!(
            unpack_color(0x0000_0000),
            [0, 0, 0, 255],
            "透明度 0x00 = 全不透明"
        );
    }

    #[test]
    fn zero_sized_bitmap_is_skipped_not_error() {
        let img = ImageDesc {
            width: 0,
            height: 5,
            stride: 0,
            bitmap: &[],
            // 不透明白：这样"什么都没画"只能归因于 w==0，而不是颜色透明
            color: 0xFF_FF_FF_00,
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Character,
        };
        img.check().unwrap();
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        assert!(f.is_empty(), "w/h 为 0 的位图不该画任何东西");
    }

    #[test]
    fn coverage_becomes_alpha_and_color_is_kept() {
        // 2x1 半覆盖（128）的纯白、完全不透明
        let bmp = [128u8, 255];
        let img = ImageDesc {
            width: 2,
            height: 1,
            stride: 2,
            bitmap: &bmp,
            color: 0xFF_FF_FF_00,
            dst_x: 1,
            dst_y: 2,
            kind: ImageKind::Character,
        };
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        assert_eq!(f.pixel(1, 2)[3], 128, "覆盖率 128 -> alpha 128（半进位）");
        assert_eq!(f.pixel(2, 2), [255, 255, 255, 255]);
        assert_eq!(f.pixel(0, 2), [0, 0, 0, 0], "画布其余保持透明");
    }

    #[test]
    fn color_alpha_multiplies_coverage() {
        let bmp = [200u8];
        let img = ImageDesc {
            width: 1,
            height: 1,
            stride: 1,
            bitmap: &bmp,
            color: 0xFF_00_00_80, // 红色；透明度 0x80 -> alpha=127
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Shadow,
        };
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        // round_div(200*127, 255) = 25527/255 = 100
        assert_eq!(f.pixel(0, 0), [255, 0, 0, 100]);
    }

    #[test]
    fn order_matters_shadow_under_glyph_vs_above() {
        // 同一位置两张图：先画的被压在下面
        let cov = [255u8];
        let shadow = ImageDesc {
            width: 1,
            height: 1,
            stride: 1,
            bitmap: &cov,
            color: 0x00_00_FF_80,
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Shadow,
        };
        let glyph = ImageDesc {
            width: 1,
            height: 1,
            stride: 1,
            bitmap: &cov,
            color: 0xFF_00_00_00,
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Character,
        };
        let mut loss = LossReport::new();
        let a = composite(small_canvas(), &[shadow, glyph], &mut loss).unwrap();
        let b = composite(small_canvas(), &[glyph, shadow], &mut loss).unwrap();
        assert_eq!(a.pixel(0, 0), [255, 0, 0, 255], "不透明红字压住蓝阴影");
        assert_ne!(
            a.pixel(0, 0),
            b.pixel(0, 0),
            "顺序颠倒必须改变结果（ass.h:56 要求按链表顺序合成）"
        );
    }

    #[test]
    fn last_row_may_be_short_but_stride_smaller_than_width_is_error() {
        // h=2 stride=4 w=4：最后一行只给了 4 字节 -> 保证长度 4*1+4=8，给 7 就不合法
        let short = [0u8; 7];
        let img = ImageDesc {
            width: 4,
            height: 2,
            stride: 4,
            bitmap: &short,
            color: 0xFF_FF_FF_FF,
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Character,
        };
        let e = img.check().unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0062");

        let bad_stride = ImageDesc {
            stride: 3,
            bitmap: &[0u8; 8],
            ..img
        };
        assert_eq!(
            bad_stride.check().unwrap_err().code_string(),
            "STK-RND-0061",
            "stride < w 时逐行读取必然越界"
        );

        // 恰好满足保证长度（stride*(h-1)+w）就行
        let ok = ImageDesc {
            bitmap: &[0u8; 8],
            ..img
        };
        ok.check().unwrap();
    }

    #[test]
    fn negative_dest_and_overrun_are_clipped_not_wrapped() {
        let cov = [255u8; 4];
        let img = ImageDesc {
            width: 4,
            height: 1,
            stride: 4,
            bitmap: &cov,
            color: 0x11_22_33_00,
            dst_x: -2,
            dst_y: 0,
            kind: ImageKind::Character,
        };
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        // dst_x=-2、宽 4：只有第 3、4 个像素落在画布上（x=0、x=1）
        assert_eq!(f.pixel(0, 0), [0x11, 0x22, 0x33, 255]);
        assert_eq!(f.pixel(1, 0), [0x11, 0x22, 0x33, 255]);
        assert_eq!(f.pixel(2, 0), [0, 0, 0, 0], "越界部分不许环绕到行首");
        assert_eq!(f.pixel(7, 0), [0, 0, 0, 0], "更不许被当成最后一列");
        assert!(
            loss.dropped.iter().any(|t| t.contains("画布之外")),
            "裁剪必须记账：{:?}",
            loss.dropped
        );

        let off = ImageDesc { dst_y: 99, ..img };
        let mut loss2 = LossReport::new();
        let g = composite(small_canvas(), &[off], &mut loss2).unwrap();
        assert!(g.is_empty(), "整行在画布外就不能画出任何东西");
        assert!(loss2.dropped.iter().any(|t| t.contains("画布之外")));
    }

    #[test]
    fn fully_transparent_color_alpha_draws_nothing() {
        let cov = [255u8; 2];
        let img = ImageDesc {
            width: 2,
            height: 1,
            stride: 2,
            bitmap: &cov,
            // 透明度 0xFF：libass 自己在 ass_render.c:3416 就把这种位图整个丢掉，
            // 我们保留位图但必须画不出东西
            color: 0xFF_FF_FF_FF,
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Outline,
        };
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        assert!(f.is_empty(), "透明度字节 0xFF 等于不画");
    }

    #[test]
    fn quantization_to_zero_is_reported_not_silent() {
        // 覆盖率 1 × alpha 1 -> round(1/255) = 0：这类像素被量化掉，必须进 LossReport
        let cov = [1u8];
        let img = ImageDesc {
            width: 1,
            height: 1,
            stride: 1,
            bitmap: &cov,
            color: 0xFF_FF_FF_FE, // 透明度 0xFE -> alpha=1
            dst_x: 0,
            dst_y: 0,
            kind: ImageKind::Shadow,
        };
        let mut loss = LossReport::new();
        let f = composite(small_canvas(), &[img], &mut loss).unwrap();
        assert!(f.is_empty());
        assert!(
            loss.approximated.iter().any(|t| t.contains("量化")),
            "{:?}",
            loss.approximated
        );
    }
}
