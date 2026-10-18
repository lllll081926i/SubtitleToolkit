//! 像素格式转换：CSRI 的 BGRA / BGR_ → `image-core` 用的 RGBA8。
//!
//! ## 为什么转换点必须只有一处
//!
//! 12 §1 的三条链给出的像素格式不同：libass 那条给 **RGBA8**，CSRI 给
//! **BGRA** 或 **BGR_（RGB32，alpha 未定义）**。两者都是"32 位色"，
//! 所以"直接 memcpy"看起来是对的 —— 而它会把红蓝通道互换。
//!
//! 那类错误**不会崩、不会报、图也出得来**，只是颜色不对。所以：
//!
//! - 转换只写在这一处，不混进 `image-core` 的合成逻辑；
//! - 每种格式各有一个用例，逐**通道**断言，而不是断言"长度对"。

use crate::codes;
use core_model::error::{Result, StkError};

use crate::ffi::{CsriFrame, PixFmt};

/// 一次渲染得到的 RGBA8 帧。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsriRgba {
    pub width: u32,
    pub height: u32,
    /// 行优先、每像素 4 字节的 RGBA8，`len == width * height * 4`。
    pub pixels: Vec<u8>,
}

impl CsriRgba {
    /// 全透明的一帧。用于"该时刻没有字幕"。
    pub fn transparent(width: u32, height: u32) -> CsriRgba {
        CsriRgba {
            width,
            height,
            pixels: vec![0u8; (width as usize) * (height as usize) * 4],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pixels.iter().all(|b| *b == 0)
    }
}

/// 把一个 CSRI 帧**拷进**已分配好的 RGBA 缓冲。
///
/// `dst` 的长度必须是 `width * height * 4`。函数只读 `frame`，
/// 不做任何分配 —— 缓冲由调用方持有，因为它的半页生命期比 `frame` 长
/// （`frame` 只在 `csri_render` 期间有效）。
///
/// ## `stride` 为什么可以是负的
///
/// 头文件写"topmost line always comes first"，但**应用**自己决定
/// 把 `planes[0]` 指到哪一端。x264 的调用示例用的是
/// `planes[0] = canvas + (height-1)*width*4; strides[0] = -(width*4);`
/// —— 即从**最后一行**开始、逐行向上。所以 `stride` 是 `isize`，
/// 而行地址要按它**带符号**地算。
///
/// 把它当无符号会让 `-7680` 变成一个巨大的正数，于是读到进程内存之外。
pub fn copy_frame_into(frame: &CsriFrame, width: u32, height: u32, dst: &mut [u8]) -> Result<()> {
    let need = (width as usize) * (height as usize) * 4;
    if dst.len() != need {
        return Err(StkError::simple(
            codes::BUFFER_SIZE.0,
            codes::BUFFER_SIZE.1,
            "CSRI 目标缓冲长度与画布不符",
        )
        .with_detail(format!("期望 {need}，收到 {}", dst.len())));
    }
    let Some(fmt) = PixFmt::from_raw(frame.pixfmt) else {
        return Err(StkError::simple(
            codes::UNKNOWN_PIXFMT.0,
            codes::UNKNOWN_PIXFMT.1,
            "CSRI provider 报了一个我们不认识的像素格式",
        )
        .with_detail(format!("pixfmt = 0x{:x}", frame.pixfmt)));
    };
    let Some(bpp) = fmt.bytes_per_pixel() else {
        return Err(StkError::simple(
            codes::PLANAR_PIXFMT.0,
            codes::PLANAR_PIXFMT.1,
            "CSRI provider 给的是 planar 格式，这条链没实现",
        )
        .with_detail(format!(
            "{}：本实现只处理 packed 的 BGRA / BGR_",
            fmt.label()
        )));
    };
    let base = frame.planes[0];
    if base.is_null() {
        // 空指针**不是**"这一帧没内容"：那是 provider 没按约定填，
        // 而当成功处理会让用户拿到一帧全黑而不报错。
        return Err(StkError::simple(
            codes::NULL_PLANE.0,
            codes::NULL_PLANE.1,
            "CSRI provider 没有给出像素缓冲",
        )
        .with_detail(format!("{} 的 planes[0] 为空", fmt.label())));
    }
    let stride = frame.strides[0];
    // 两道核对的是**不同**的事，别把两者混成一次比较：
    //
    // 1. **一行装得下吗**：比的是 `|stride|` 与行宽。
    //    写成比 `|stride * height|` 是错的 —— 那个乘积在 height>1 时
    //    即使 stride 小得装不下一行也可能超过行宽，于是把越界读放了过去。
    //    （这个错是我写完第一版后用自己的用例抳出来的：stride=4、height=2、
    //    行宽 8 时 `4*2+4=12 >= 8` 让张错误过了关。）
    // 2. **整块装得下吗**：`stride * height` 不能溢出。
    let row_bytes = (width as usize) * bpp;
    if stride.unsigned_abs() < row_bytes {
        return Err(StkError::simple(
            codes::BAD_STRIDE.0,
            codes::BAD_STRIDE.1,
            "CSRI 的 stride 装不下一整行",
        )
        .with_detail(format!("stride={stride}，一行需要 {row_bytes} 字节")));
    }
    // 溢出只用 checked 运算探测，结果不再参与判断 —— 上面那条已经够了。
    stride
        .checked_mul(height as isize)
        .and_then(|s| s.checked_add(bpp as isize))
        .ok_or_else(|| {
            StkError::simple(
                codes::BAD_STRIDE.0,
                codes::BAD_STRIDE.1,
                "CSRI 的 stride 组合溢出",
            )
            .with_detail(format!("stride={stride} height={height}"))
        })?;

    for y in 0..height as usize {
        // SAFETY：调用方保证 `base` 指向至少 `|stride| * height` 字节的可读区域
        // （这是 CSRI 的调用契约：应用分配缓冲并把它交给渲染器）。
        // 偏移按 `isize` 算，所以负 stride 会正确地向"更低地址"走。
        let row = unsafe { base.offset(stride * y as isize) };
        let out = &mut dst[y * (width as usize) * 4..(y + 1) * (width as usize) * 4];
        // SAFETY：上面核过 stride 至少装得下一行。
        let src = unsafe { std::slice::from_raw_parts(row, (width as usize) * bpp) };
        convert_row(src, out, fmt);
    }
    Ok(())
}

/// 逐行转换。抽出来是为了让"通道顺序"这件事只有一个地方定义。
fn convert_row(src: &[u8], dst: &mut [u8], fmt: PixFmt) {
    match fmt {
        // BGRA：内存里是 B,G,R,A —— 交换 0 与 2 就是 RGBA。
        PixFmt::Bgra => {
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[2]; // R
                d[1] = s[1]; // G
                d[2] = s[0]; // B
                d[3] = s[3]; // A
            }
        }
        // BGR_（Windows "RGB32"）：B,G,R,X —— 第 4 字节是填充，
        // **不是 alpha**。当 alpha 用会让整帧按那个字节的透明度混，
        // 而它常常是 0。所以显式写 255。
        PixFmt::BgrUnderscore => {
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        }
        // BGR（Windows "RGB24"）：3 字节一像素，同样补不透明 alpha。
        PixFmt::Bgr => {
            for (s, d) in src.chunks_exact(3).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[2];
                d[1] = s[1];
                d[2] = s[0];
                d[3] = 255;
            }
        }
        // RGBA / ARGB / ABGR：本链当前不预期 provider 给这些，
        // 但列出来并**明确处理**，免得将来多一条静默走错的分支。
        PixFmt::Rgba => dst.copy_from_slice(src),
        PixFmt::Argb => {
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[1];
                d[1] = s[2];
                d[2] = s[3];
                d[3] = s[0];
            }
        }
        PixFmt::Abgr => {
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[3];
                d[1] = s[2];
                d[2] = s[1];
                d[3] = s[0];
            }
        }
        // planar / packed-YUV：`copy_frame_into` 已经把它们拦在前面了（
        // `bytes_per_pixel` 返回 None）。这里列全是为了**不留下静默分支** ——
        // 加一个 `_ =>` 会让“新格式被默默当成某种旧格式”成为可能。
        PixFmt::Yv12
        | PixFmt::Yv12a
        | PixFmt::Ayuv
        | PixFmt::Yuva
        | PixFmt::Yvua
        | PixFmt::Yuy2 => debug_assert!(
            false,
            "planar/packed-YUV 不该走到这里：copy_frame_into 已按 bytes_per_pixel 拦截"
        ),
        // 另外几个 32 位变体（_RGB / RGB_ / _BGR）当前 provider 没报过，
        // 但它们是合法输入。**逐字节序处理**而不是“当成某个像的”——
        // 通道顺序写错不会崩、不会报，只是颜色不对。
        PixFmt::RgbUnderscore => {
            // R,G,B,X → 第 4 字节是填充
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[0];
                d[1] = s[1];
                d[2] = s[2];
                d[3] = 255;
            }
        }
        PixFmt::UnderscoreRgb => {
            // X,R,G,B
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[1];
                d[1] = s[2];
                d[2] = s[3];
                d[3] = 255;
            }
        }
        PixFmt::UnderscoreBgr => {
            // X,B,G,R
            for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[3];
                d[1] = s[2];
                d[2] = s[1];
                d[3] = 255;
            }
        }
        PixFmt::Rgb => {
            // 24 位 RGB：3 字节一像素，补不透明 alpha
            for (s, d) in src.chunks_exact(3).zip(dst.chunks_exact_mut(4)) {
                d[0] = s[0];
                d[1] = s[1];
                d[2] = s[2];
                d[3] = 255;
            }
        }
    }
}

/// 造一个指向 `buf` 顶部（第一行）的帧 —— 大多数调用方要的是这个。
///
/// # Safety
///
/// `buf` 必须至少 `width * height * bpp` 字节，且在这次 `csri_render`
/// 调用期间保持有效（provider 会往它里面写）。
pub unsafe fn frame_over_buffer(
    buf: &mut [u8],
    pixfmt: PixFmt,
    width: u32,
    height: u32,
) -> Result<CsriFrame> {
    let bpp = pixfmt.bytes_per_pixel().ok_or_else(|| {
        StkError::simple(
            codes::PLANAR_PIXFMT.0,
            codes::PLANAR_PIXFMT.1,
            "planar 格式不能这样建帧",
        )
    })?;
    let need = (width as usize) * (height as usize) * bpp;
    if buf.len() < need {
        return Err(StkError::simple(
            codes::BUFFER_SIZE.0,
            codes::BUFFER_SIZE.1,
            "像素缓冲小于画布需要的大小",
        )
        .with_detail(format!("需要 {need}，只有 {}", buf.len())));
    }
    let mut frame = CsriFrame {
        pixfmt: pixfmt as i32,
        planes: [std::ptr::null_mut(); 4],
        strides: [0; 4],
    };
    frame.planes[0] = buf.as_mut_ptr();
    frame.strides[0] = ((width as usize) * bpp) as isize;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_from(buf: &mut [u8], fmt: PixFmt, w: u32, h: u32) -> CsriFrame {
        unsafe { frame_over_buffer(buf, fmt, w, h).unwrap() }
    }

    /// BGRA → RGBA：逐**通道**核，不只核长度。
    ///
    /// 只核长度的用例对"红蓝互换"完全无感 —— 而那正是这里最容易犯的错。
    #[test]
    fn bgra_to_rgba_swaps_red_and_blue_not_green_and_alpha() {
        // 一像素：B=0x11 G=0x22 R=0x33 A=0x44
        let mut src = vec![0x11u8, 0x22, 0x33, 0x44];
        let frame = frame_from(&mut src, PixFmt::Bgra, 1, 1);
        let mut dst = vec![0u8; 4];
        copy_frame_into(&frame, 1, 1, &mut dst).unwrap();
        assert_eq!(dst, vec![0x33, 0x22, 0x11, 0x44], "R/B 要换、G/A 不动");
    }

    /// `BGR_` 的第 4 字节是**填充**，不是 alpha。
    ///
    /// 当 alpha 用会让整帧按那个字节混 —— 而它常常是 0（全透明），
    /// 于是"字幕不见了"，且不报任何错。
    #[test]
    fn bgr_underscore_treats_the_fourth_byte_as_padding_not_alpha() {
        let mut src = vec![0x11u8, 0x22, 0x33, 0x00]; // X=0x00
        let frame = frame_from(&mut src, PixFmt::BgrUnderscore, 1, 1);
        let mut dst = vec![0u8; 4];
        copy_frame_into(&frame, 1, 1, &mut dst).unwrap();
        assert_eq!(dst, vec![0x33, 0x22, 0x11, 255], "第 4 字节必须是 255");
    }

    /// `BGR` 是 3 字节一像素，源行比目标行短。
    #[test]
    fn bgr_24bit_expands_to_four_bytes_with_opaque_alpha() {
        let mut src = vec![0x11u8, 0x22, 0x33];
        let frame = frame_from(&mut src, PixFmt::Bgr, 1, 1);
        let mut dst = vec![0u8; 4];
        copy_frame_into(&frame, 1, 1, &mut dst).unwrap();
        assert_eq!(dst, vec![0x33, 0x22, 0x11, 255]);
    }

    /// **负 stride**：`planes[0]` 指向内存里**最后一行**，然后逐行向上取。
    ///
    /// 这是 x264 调用示例用的形态（`planes[0] = canvas + (height-1)*w*4;
    /// strides[0] = -(w*4)`），也是最容易写错的一处：
    /// 把 stride 当无符号会让 `-8` 变成一个巨大的正数。
    ///
    /// 地址算是 `base + stride * y`，所以 y 增大时地址**变小**。
    /// 下面把缓冲按“屏幕顺序”放（内存里的第 0 行就是屏幕的第 0 行），
    /// 然后让 `planes[0]` 指向内存的最后一行 —— 于是：
    ///
    /// - 屏幕第 0 行 = `base + (-8)*0` = 内存最后一行；
    /// - 屏幕第 1 行 = `base + (-8)*1` = 内存第一行。
    ///
    /// 这个对应关系看着反直觉，**但它就是负 stride 的定义**：
    /// 应用把 `planes[0]` 指向自己缓冲的底部，而渲染器逐行向上写。
    /// 本用例钉住的是“我们按带符号的 stride 算”，不是“我们按谁的直觉算”。
    #[test]
    fn a_negative_stride_walks_upward_from_the_last_row() {
        // 屏幕第 0 行像素值 40/41，屏幕第 1 行像素值 10/11
        let mut buf = vec![
            40, 50, 60,
            255, /* 内存第 0 行第 1 像素 → 会作为**屏幕第 1 行** */
            41, 51, 61, 255, /* 内存第 0 行第 2 像素 */
            10, 20, 30,
            255, /* 内存第 1 行第 1 像素 → 会作为**屏幕第 0 行** */
            11, 21, 31, 255, /* 内存第 1 行第 2 像素 */
        ];
        let mut frame = CsriFrame {
            pixfmt: PixFmt::Bgra as i32,
            planes: [std::ptr::null_mut(); 4],
            strides: [0; 4],
        };
        // 指向内存的**最后一行**，stride = -8（一行 2 像素×4 字节）
        frame.planes[0] = unsafe { buf.as_mut_ptr().add(8) };
        frame.strides[0] = -8;

        let mut dst = vec![0u8; 16];
        copy_frame_into(&frame, 2, 2, &mut dst).unwrap();
        // 屏幕第 0 行 = planes[0] 处 = 内存第 1 行（B=10 G=20 R=30）→ RGBA [30,20,10,255]
        assert_eq!(&dst[0..4], &[30, 20, 10, 255], "屏幕第 0 行取错了");
        assert_eq!(&dst[4..8], &[31, 21, 11, 255]);
        // 屏幕第 1 行 = 向上走一行 = 内存第 0 行
        assert_eq!(&dst[8..12], &[60, 50, 40, 255], "屏幕第 1 行取错了");
        assert_eq!(&dst[12..16], &[61, 51, 41, 255]);
    }

    /// 与上一条配对：**正 stride** 时 `planes[0]` 就是屏幕第 0 行。
    ///
    /// 两条合起来才说明“我们是按 stride 的符号走”，而不是只会一种。
    #[test]
    fn a_positive_stride_walks_downward_from_the_first_row() {
        let mut buf = vec![
            10, 20, 30, 255, /* 内存第 0 行 → 屏幕第 0 行 */
            11, 21, 31, 255, //
            40, 50, 60, 255, /* 内存第 1 行 → 屏幕第 1 行 */
            41, 51, 61, 255, //
        ];
        let frame = frame_from(&mut buf, PixFmt::Bgra, 2, 2);
        let mut dst = vec![0u8; 16];
        copy_frame_into(&frame, 2, 2, &mut dst).unwrap();
        assert_eq!(&dst[0..4], &[30, 20, 10, 255]);
        assert_eq!(&dst[8..12], &[60, 50, 40, 255]);
    }

    #[test]
    fn a_null_plane_is_an_error_not_a_black_frame() {
        let frame = CsriFrame {
            pixfmt: PixFmt::Bgra as i32,
            planes: [std::ptr::null_mut(); 4],
            strides: [0; 4],
        };
        let mut dst = vec![0u8; 4];
        let e = copy_frame_into(&frame, 1, 1, &mut dst).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0077");
    }

    #[test]
    fn a_wrong_sized_destination_is_an_error() {
        let mut src = vec![0u8; 16];
        let frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        let mut too_small = vec![0u8; 8];
        let e = copy_frame_into(&frame, 2, 2, &mut too_small).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0074");
    }

    #[test]
    fn an_unknown_pixel_format_is_named_not_guessed() {
        let mut src = vec![0u8; 16];
        let mut frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        frame.pixfmt = 0x7777;
        let mut dst = vec![0u8; 16];
        let e = copy_frame_into(&frame, 2, 2, &mut dst).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0075");
        assert!(e.details.join(" ").contains("0x7777"), "{:?}", e.details);
    }

    #[test]
    fn a_planar_format_is_refused_explicitly() {
        let mut src = vec![0u8; 16];
        let mut frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        frame.pixfmt = PixFmt::Yv12 as i32;
        let mut dst = vec![0u8; 16];
        let e = copy_frame_into(&frame, 2, 2, &mut dst).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0076");
    }

    /// stride 装不下一整行时要拒绝 —— 那是越界读。
    ///
    /// 回归用例：第一版把 `|stride|` 与**行宽**的比较写成了
    /// `|stride * height|` 与行宽的比。两者在 height>1 时不等价 ——
    /// stride=4、height=2、行宽 8 时 `4*2+4=12 >= 8`，于是一条
    /// 根本装不下一行的 stride 被放行，而后面会按 8 字节读一个
    /// 只有 4 字节步长的缓冲。
    #[test]
    fn a_stride_narrower_than_one_row_is_refused() {
        let mut src = vec![0u8; 16];
        let mut frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        frame.strides[0] = 4; // 一行要 8 字节 —— 4 太小
        let mut dst = vec![0u8; 16];
        let e = copy_frame_into(&frame, 2, 2, &mut dst).unwrap_err();
        assert_eq!(e.code_string(), "STK-RND-0078");
        // 报错要说清两边的数字，否则用户无从判断差多少
        let d = e.details.join(" ");
        assert!(d.contains("stride=4"), "{d}");
        assert!(d.contains("8 字节"), "{d}");
    }

    /// 与上一条配对：**恰好等于行宽的 stride 是合法的**。
    ///
    /// 只断言"太小的被拒"会漏掉"把合法的也拒了"这种同样错得离谱的实现。
    #[test]
    fn a_stride_exactly_one_row_wide_is_accepted() {
        let mut src = vec![1u8; 16];
        let mut frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        frame.strides[0] = 8; // 恰好 2 像素 × 4 字节
        let mut dst = vec![0u8; 16];
        copy_frame_into(&frame, 2, 2, &mut dst).expect("恰好一行的 stride 应当合法");
    }

    #[test]
    fn rgba_passes_through_unchanged() {
        let mut src = vec![1u8, 2, 3, 4];
        let frame = frame_from(&mut src, PixFmt::Rgba, 1, 1);
        let mut dst = vec![0u8; 4];
        copy_frame_into(&frame, 1, 1, &mut dst).unwrap();
        assert_eq!(dst, vec![1, 2, 3, 4]);
    }

    #[test]
    fn transparent_frames_are_all_zero_and_recognised_as_empty() {
        let f = CsriRgba::transparent(3, 2);
        assert_eq!(f.pixels.len(), 24);
        assert!(f.is_empty());
    }

    /// 多行、非 1 像素的常规路径也要对（防"只会转 1x1"）。
    #[test]
    fn a_multi_row_frame_converts_every_row() {
        let mut src = vec![0u8; 2 * 2 * 4];
        for y in 0..2usize {
            for x in 0..2usize {
                let i = (y * 2 + x) * 4;
                src[i] = (10 * y + x) as u8; // B
                src[i + 1] = 0x80; // G
                src[i + 2] = (100 + 10 * y + x) as u8; // R
                src[i + 3] = 0xFF; // A
            }
        }
        let frame = frame_from(&mut src, PixFmt::Bgra, 2, 2);
        let mut dst = vec![0u8; 16];
        copy_frame_into(&frame, 2, 2, &mut dst).unwrap();
        for y in 0..2usize {
            for x in 0..2usize {
                let i = (y * 2 + x) * 4;
                assert_eq!(dst[i], 100 + 10 * y as u8 + x as u8, "R 在 ({x},{y})");
                assert_eq!(dst[i + 1], 0x80);
                assert_eq!(dst[i + 2], 10 * y as u8 + x as u8, "B 在 ({x},{y})");
                assert_eq!(dst[i + 3], 0xFF);
            }
        }
    }
}
