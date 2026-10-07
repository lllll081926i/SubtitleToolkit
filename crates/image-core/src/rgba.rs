//! RGBA 位图层（09 §3）。
//!
//! 规则：**bbox 裁剪只产生局部位图，绝不改变它在完整 16:9 画布上的位置**。
//! 因此 [`Frame::crop`] 返回的 [`Patch`] 一定带 `x/y` 画布坐标，
//! 而 [`Patch::place_back`] 必须能把裁剪结果原样合回完整画布（测试逐像素验证）。

use core_model::error::{Domain, Result, StkError};
use core_model::Canvas;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub canvas: Canvas,
    /// 行主序 RGBA，长度 = w*h*4
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundingBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Patch {
    pub width: u32,
    pub height: u32,
    /// 完整画布坐标（09 §3：不是相对 bbox 的坐标）
    pub x: u32,
    pub y: u32,
    pub rgba: Vec<u8>,
}

impl Frame {
    pub fn new(canvas: Canvas, rgba: Vec<u8>) -> Result<Frame> {
        let need = pixel_len(canvas.width, canvas.height)?
            .checked_mul(4)
            .ok_or_else(|| StkError::simple(Domain::Image, 1, "画布像素数溢出"))?;
        if rgba.len() != need {
            return Err(StkError::simple(Domain::Image, 2, "RGBA 长度与画布不符")
                .with_detail(format!("需要 {need}，得到 {}", rgba.len())));
        }
        Ok(Frame { canvas, rgba })
    }

    pub fn empty(canvas: Canvas) -> Result<Frame> {
        let n = pixel_len(canvas.width, canvas.height)?
            .checked_mul(4)
            .ok_or_else(|| StkError::simple(Domain::Image, 1, "画布像素数溢出"))?;
        Frame::new(canvas, vec![0u8; n])
    }

    pub fn is_empty(&self) -> bool {
        self.rgba.chunks_exact(4).all(|p| p[3] == 0)
    }

    /// 不透明像素的最小外接矩形；全透明返回 `None`（调用方必须改成 erase，而不是写全透明图）。
    pub fn bbox(&self) -> Option<BoundingBox> {
        let (w, h) = (self.canvas.width, self.canvas.height);
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for y in 0..h {
            for x in 0..w {
                if self.pixel_alpha(x, y) != 0 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }
        if min_x > max_x || min_y > max_y {
            return None;
        }
        Some(BoundingBox {
            x: min_x,
            y: min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        })
    }

    pub fn pixel_alpha(&self, x: u32, y: u32) -> u8 {
        let off = (y * self.canvas.width + x) * 4;
        self.rgba.get(off as usize + 3).copied().unwrap_or(0)
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let off = ((y * self.canvas.width + x) * 4) as usize;
        let mut p = [0u8; 4];
        p.copy_from_slice(&self.rgba[off..off + 4]);
        p
    }

    /// 按 bbox 裁出局部位图，位置保持画布坐标。
    pub fn crop(&self) -> Result<Option<Patch>> {
        let Some(b) = self.bbox() else {
            return Ok(None);
        };
        Ok(Some(self.crop_to(&b)?))
    }

    pub fn crop_to(&self, b: &BoundingBox) -> Result<Patch> {
        if b.width == 0 || b.height == 0 {
            return Err(StkError::simple(Domain::Image, 3, "bbox 尺寸不能为 0"));
        }
        if b.x
            .checked_add(b.width)
            .is_none_or(|e| e > self.canvas.width)
            || b.y
                .checked_add(b.height)
                .is_none_or(|e| e > self.canvas.height)
        {
            return Err(StkError::simple(Domain::Image, 4, "bbox 越出画布")
                .with_detail(format!("{b:?} 画布 {}", self.canvas.label())));
        }
        let n = pixel_len(b.width, b.height)?
            .checked_mul(4)
            .ok_or_else(|| StkError::simple(Domain::Image, 1, "bbox 像素数溢出"))?;
        let mut rgba = vec![0u8; n];
        for row in 0..b.height {
            let src = ((b.y + row) * self.canvas.width + b.x) * 4;
            let dst = row * b.width * 4;
            let len = (b.width * 4) as usize;
            rgba[dst as usize..dst as usize + len]
                .copy_from_slice(&self.rgba[src as usize..src as usize + len]);
        }
        Ok(Patch {
            width: b.width,
            height: b.height,
            x: b.x,
            y: b.y,
            rgba,
        })
    }
}

impl Patch {
    pub fn bounding_box(&self) -> BoundingBox {
        BoundingBox {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }

    /// 合回完整画布（alpha 不混合，直接覆盖）：09 §9 要求裁剪前后 composite 等价。
    pub fn place_back(&self, canvas: Canvas) -> Result<Frame> {
        let mut frame = Frame::empty(canvas)?;
        self.blend_into(&mut frame)?;
        Ok(frame)
    }

    pub fn blend_into(&self, frame: &mut Frame) -> Result<()> {
        if self
            .x
            .checked_add(self.width)
            .is_none_or(|e| e > frame.canvas.width)
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|e| e > frame.canvas.height)
        {
            return Err(
                StkError::simple(Domain::Image, 4, "局部位图越出目标画布").with_detail(format!(
                    "x={} w={} 画布 {}",
                    self.x,
                    self.width,
                    frame.canvas.label()
                )),
            );
        }
        for row in 0..self.height {
            for col in 0..self.width {
                let s = usize::try_from((row * self.width + col) * 4)
                    .map_err(|_| StkError::simple(Domain::Image, 1, "局部位图偏移溢出"))?;
                let d = usize::try_from(((self.y + row) * frame.canvas.width + self.x + col) * 4)
                    .map_err(|_| StkError::simple(Domain::Image, 1, "目标画布偏移溢出"))?;
                // alpha=0 的像素必须整颗清零：留着旧 RGB 会让下一次 bbox 把已擦掉的区域当内容
                if self.rgba[s + 3] == 0 {
                    frame.rgba[d..d + 4].copy_from_slice(&[0, 0, 0, 0]);
                } else {
                    frame.rgba[d..d + 4].copy_from_slice(&self.rgba[s..s + 4]);
                }
            }
        }
        Ok(())
    }

    pub fn pixel_len(&self) -> Result<usize> {
        pixel_len(self.width, self.height)
    }
}

pub fn pixel_len(w: u32, h: u32) -> Result<usize> {
    usize::try_from(w)
        .ok()
        .and_then(|ww| usize::try_from(h).ok().map(|hh| ww.checked_mul(hh)))
        .flatten()
        .ok_or_else(|| StkError::simple(Domain::Image, 1, "尺寸乘法溢出"))
}

/// "覆盖"语义下的透明像素：RGBA 全 0。
pub fn is_transparent(px: &[u8]) -> bool {
    px.len() >= 4 && px[3] == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::canvas::Canvas;

    fn filled(canvas: Canvas, x0: u32, y0: u32, w: u32, h: u32) -> Frame {
        let mut f = Frame::empty(canvas).unwrap();
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let off = ((y * canvas.width + x) * 4) as usize;
                f.rgba[off] = 255;
                f.rgba[off + 1] = 128;
                f.rgba[off + 2] = 10;
                f.rgba[off + 3] = 255;
            }
        }
        f
    }

    #[test]
    fn length_and_oversize_guards() {
        assert!(Frame::new(Canvas::FHD1080, vec![0u8; 16]).is_err());
        let bad = Frame::empty(Canvas::FHD1080).unwrap();
        assert!(matches!(
            bad.crop_to(&BoundingBox {
                x: 1919,
                y: 1079,
                width: 8,
                height: 8
            }),
            Err(e) if e.code_string() == "STK-IMG-0004"
        ));
        assert_eq!(pixel_len(2, 3).unwrap(), 6, "行优先像素数就是 w*h");
    }

    #[test]
    fn bbox_uses_full_canvas_coordinates() {
        let c = Canvas::FHD1080;
        let f = filled(c, 300, 700, 20, 10);
        let b = f.bbox().unwrap();
        assert_eq!(
            (b.x, b.y, b.width, b.height),
            (300, 700, 20, 10),
            "bbox 必须是画布坐标，不是相对内容"
        );
        let p = f.crop().unwrap().unwrap();
        assert_eq!((p.x, p.y, p.width, p.height), (300, 700, 20, 10));
        assert_eq!(p.rgba.len(), 20 * 10 * 4);
    }

    #[test]
    fn crop_then_place_back_is_pixel_identical() {
        // 09 §9：bbox 裁剪前后在完整画布上 composite 的结果必须逐像素相等
        let c = Canvas::FHD1080;
        let f = filled(c, 100, 50, 800, 400);
        let p = f.crop().unwrap().unwrap();
        let back = p.place_back(c).unwrap();
        assert_eq!(back.rgba, f.rgba);
    }

    #[test]
    fn transparent_frame_has_no_bbox_and_must_become_erase() {
        let f = Frame::empty(Canvas::HD720).unwrap();
        assert!(f.is_empty());
        assert!(f.bbox().is_none());
        assert!(f.crop().unwrap().is_none(), "全透明不能写成全透明图");
    }

    #[test]
    fn alpha_zero_pixels_are_not_copied_as_content() {
        let c = Canvas::new(4, 2).unwrap();
        let mut f = Frame::empty(c).unwrap();
        // (1,0) 是 alpha=0 但有非零 RGB 的"脏"像素（行宽 4 像素 → 起点字节 4）
        let off = 4usize;
        f.rgba[off] = 200;
        f.rgba[off + 1] = 200;
        f.rgba[off + 2] = 200;
        f.rgba[off + 3] = 0;
        assert!(f.bbox().is_none(), "alpha=0 不算内容");
        let p = Patch {
            width: 1,
            height: 1,
            x: 0,
            y: 0,
            rgba: vec![9, 9, 9, 0],
        };
        let mut g = filled(c, 0, 0, 1, 1);
        p.blend_into(&mut g).unwrap();
        assert_eq!(g.pixel(0, 0), [0, 0, 0, 0], "透明覆盖像素必须清掉内容");
        assert!(is_transparent(&g.rgba[0..4]));
    }

    #[test]
    fn non_transparent_blend_is_opaque_overwrite() {
        let c = Canvas::new(2, 1).unwrap();
        let mut f = filled(c, 0, 0, 2, 1);
        let p = Patch {
            width: 1,
            height: 1,
            x: 1,
            y: 0,
            rgba: vec![1, 2, 3, 255],
        };
        p.blend_into(&mut f).unwrap();
        assert_eq!(f.pixel(1, 0), [1, 2, 3, 255]);
        assert_eq!(f.pixel(0, 0), [255, 128, 10, 255]);
        assert!(matches!(
            p.place_back(Canvas::new(1, 1).unwrap()),
            Err(e) if e.code_string() == "STK-IMG-0004"
        ));
    }
}
