//! 从渲染帧构造 BDN 文档（13 §7 + 09 §3）。
//!
//! 冻结三条决定：
//! 1. **`min_axis` 只过滤，绝不改写尺寸。** 旧工程按 `max(min_axis, w)` 钳制，XML 声明的
//!    Width/Height 于是与 PNG 的 IHDR 尺寸脱钩 —— 正是 AT-BDN-001 要禁止的缺陷。`min_axis`
//!    的正确语义（13 §7 的小对象清理）是"小于该阈值的对象被丢弃/合并"，所以这里把
//!    `png_width`/`png_height` **原样**写进 [`BdnGraphic`]，`min_axis` 只决定这一帧进不进文档。
//! 2. **时间码不在这里生成**：一律走 `BdnEvent::new` → `Time::to_bdn_timecode(fps)`（10 §7）。
//! 3. **画布不靠猜。** 规格给的 `BdnOptions` 不含画布字段，因此默认画布固定为 Blu-ray 允许的
//!    最大 16:9 画布（`BLURAY_MAX` = 1920x1080）；越界就报错，**不**通过"把整份文档升到
//!    3840x2160"来静默掩盖错误的 offset（09 §2：四层坐标不得互相顶替）。需要 2160p/exotic
//!    时用 [`build_document_on_canvas`] 显式给画布，它同样要求完整 16:9，并在 LossReport 里
//!    记 custom/exotic（09 §8）。已登记待办：上游 profile 应把输出画布传进来。

use core_model::canvas::{Canvas, BLURAY_MAX};
use core_model::error::{Domain, Result, StkError};
use core_model::geometry::Geometry;
use core_model::loss::LossReport;
use core_model::time::{Fps, Time};

use crate::model::{
    check_file_name, reject_control_chars, BdnDocument, BdnEvent, BdnFrame, BdnGraphic, BdnMeta,
    BdnOptions,
};

/// 由帧序列构造文档（丢掉 LossReport 的便捷入口）。画布 = 1920x1080。
pub fn build_document(frames: &[BdnFrame], opt: &BdnOptions) -> Result<BdnDocument> {
    build_document_with_loss(frames, opt).map(|(doc, _)| doc)
}

/// 构造文档 + Loss Model（08 §4）。画布 = `BLURAY_MAX`（1920x1080）。
pub fn build_document_with_loss(
    frames: &[BdnFrame],
    opt: &BdnOptions,
) -> Result<(BdnDocument, LossReport)> {
    build_document_on_canvas(frames, opt, BLURAY_MAX)
}

/// 在指定输出画布上构造文档（2160p / 720p / exotic 走这里）。
pub fn build_document_on_canvas(
    frames: &[BdnFrame],
    opt: &BdnOptions,
    canvas: Canvas,
) -> Result<(BdnDocument, LossReport)> {
    let mut loss = LossReport::new();
    reject_control_chars("Name", &opt.name)?;
    reject_control_chars("Language", &opt.language)?;
    let fps = Fps::new(opt.fps.as_rational())?;
    // 09 §1：输出画布永远是完整 16:9。
    Geometry::for_canvas(canvas).validate()?;

    // 先按 min_axis 过滤（丢弃），再生成事件：被丢弃的对象不该出现在 XML 里。
    let mut kept: Vec<&BdnFrame> = Vec::with_capacity(frames.len());
    for (i, f) in frames.iter().enumerate() {
        if f.png_width == 0 || f.png_height == 0 {
            return Err(StkError::simple(Domain::Bdn, 64, "PNG 实际尺寸含 0 边")
                .with_detail(format!("frames[{i}] {}", f.png_file_name)));
        }
        if f.png_width.max(f.png_height) < opt.min_axis {
            // 用"最长边"衡量小对象：字幕条天生又宽又扁（例如 800x40），按最短边判断会
            // 把正常字幕行当成垃圾丢掉。13 §7 说的"小对象"指整体都很小的碎块。
            loss.bump(
                "dropped",
                &format!("小于 min_axis {} 的小对象", opt.min_axis),
            );
            continue;
        }
        check_file_name(&f.png_file_name).map_err(|e| e.with_detail(format!("frames[{i}]")))?;
        kept.push(f);
    }

    let mut events: Vec<BdnEvent> = Vec::with_capacity(kept.len());
    let mut prev_start: Option<Time> = None;
    for (i, f) in kept.iter().enumerate() {
        let x = placed(f.x, opt.offset_x, i, "X")?;
        let y = placed(f.y, opt.offset_y, i, "Y")?;
        // 尺寸原样 = PNG 实际尺寸（决定 1）。
        let graphic = BdnGraphic::new(f.png_file_name.clone(), f.png_width, f.png_height, x, y)
            .map_err(|e| e.with_detail(format!("kept[{i}] {}", f.png_file_name)))?;
        graphic
            .fits_in(canvas)
            .map_err(|e| e.with_detail(format!("kept[{i}] {}", f.png_file_name)))?;
        let event = BdnEvent::new(f.start, f.end, graphic, fps)
            .map_err(|e| e.with_detail(format!("kept[{i}] {}", f.png_file_name)))?;
        if let Some(p) = prev_start {
            if event.start <= p {
                let cur = event.start.as_rational().to_f64();
                let before = p.as_rational().to_f64();
                return Err(StkError::simple(
                    Domain::Bdn,
                    60,
                    "BDN 帧未按 start 严格递增，拒绝构造",
                )
                .with_detail(format!(
                    "kept[{i}] start={cur:.6} 不晚于 kept[{}] start={before:.6}",
                    i - 1
                ))
                .with_detail(format!("文件 {}", event.graphic.file_name))
                .with_action("排序/合并属于 task-engine 的时间线规划（15 §批量任务）"));
            }
        }
        prev_start = Some(event.start);
        events.push(event);
    }

    let meta = BdnMeta {
        version: "0.93".to_string(),
        name: opt.name.clone(),
        language: opt.language.clone(),
        video_format: video_format_label(canvas).to_string(),
        frame_rate: fps.as_rational(),
        drop_frame: false,
        width: canvas.width,
        height: canvas.height,
    };
    loss.preserved("PNG 实际尺寸（Width/Height 不受 min_axis 影响）");
    loss.preserved("InTC/OutTC 由统一 rational frame clock 生成");
    if !canvas.within_bluray() {
        loss.nonstandard(&format!(
            "画布 {} 超出 Blu-ray 允许的最大 {}，只能标 custom/exotic",
            canvas, BLURAY_MAX
        ));
        loss.user_attention("2160p BDN 绝不标 Blu-ray/UHD-BD 合规（09 §8）");
    }
    Ok((BdnDocument::new(meta, events), loss))
}

/// 坐标叠加 offset：全程 checked_*；是否越界由 `BdnGraphic::fits_in` 统一判定。
fn placed(v: i32, offset: i32, index: usize, axis: &str) -> Result<i32> {
    v.checked_add(offset).ok_or_else(|| {
        StkError::simple(Domain::Bdn, 63, "BDN 位置叠加 offset 后溢出")
            .with_detail(format!("kept[{index}] {axis}={v} + {offset}"))
    })
}

fn video_format_label(canvas: Canvas) -> &'static str {
    match (canvas.width, canvas.height) {
        (1280, 720) => "720p",
        (3840, 2160) => "2160p",
        (1920, 1080) => "1080p",
        _ => "custom",
    }
}
