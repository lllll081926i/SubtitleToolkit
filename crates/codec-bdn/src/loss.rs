//! BDN 的 Loss Model（08 §4）与 Profile 标签（09 §8、13 §7）。
//!
//! BDN 只是交换格式，不是最终 PGS strict，所以这里只报告"这份文档本身说明了什么"：画布档位、
//! 时间码量化、非预设帧率、Version 方言。合规判定属于 validator（Level 5），codec 层不伪造
//! "Blu-ray compliant"。

use core_model::canvas::{Canvas, BLURAY_MAX};
use core_model::loss::LossReport;
use core_model::time::{Fps, Time};

use crate::model::BdnDocument;

/// 文档级 loss 报告（永不失败：无法判定的项按 user_attention 报告，不 panic 也不吞掉）。
pub fn loss_of(doc: &BdnDocument) -> LossReport {
    let mut loss = LossReport::new();
    loss.preserved("事件时间轴（start/end 是精确有理数，不落 float）");
    loss.preserved("Graphic 尺寸语义：XML 的 Width/Height 必须等于 PNG 实际尺寸");
    loss.approximated("文本/样式信息全部固化进 PNG（BDN 是图像交换格式，13 §7）");

    let canvas = Canvas {
        width: doc.meta.width,
        height: doc.meta.height,
    };
    if !canvas.is_16_9() {
        loss.nonstandard(&format!("输出画布 {} 不是完整 16:9", canvas));
        loss.user_attention("09 §1：特效字幕输出永远是完整 16:9 画布");
    }
    if !canvas.within_bluray() {
        loss.nonstandard(&format!(
            "画布 {} 超出 Blu-ray 允许的最大 {}，只能标 custom/exotic",
            canvas, BLURAY_MAX
        ));
        loss.user_attention("禁止把 2160p BDN/PGS 显示为 Blu-ray/UHD-BD 合规（09 §8、13 §1）");
    }
    if doc.meta.version != "0.93" {
        loss.nonstandard(&format!(
            "BDN Version=\"{}\" 非常见 0.93 方言",
            doc.meta.version
        ));
    }
    if doc.meta.drop_frame {
        loss.nonstandard("DropFrame=TRUE：BDN timecode 没有丢帧语义，仅作原样往返");
        loss.user_attention("下游 PGS 会忽略 DropFrame");
    }

    let Ok(fps) = Fps::new(doc.meta.frame_rate) else {
        loss.user_attention("FrameRate 无法作为帧率使用，时间码不可解释");
        return loss;
    };
    if Fps::PRESETS
        .iter()
        .any(|(_, f)| f.as_rational() == fps.as_rational())
    {
        loss.preserved("帧率是标准预设（精确 rational，如 23.976 = 24000/1001）");
    } else {
        loss.nonstandard(&format!("帧率 {} 不是任何标准预设", fps));
    }

    // 单一时间源的代价：BDN timecode 只有 1/nominal 秒的分辨率，不落在帧网格上的时间会被
    // 量化 —— 这是"近似"，必须显式报告而不是假装无损（13 §7：BDN 以视频 frame boundary 为主）。
    let mut quantized = 0usize;
    for ev in &doc.events {
        if !timecode_is_exact(ev.start, fps) || !timecode_is_exact(ev.end, fps) {
            quantized = quantized.saturating_add(1);
        }
    }
    if quantized > 0 {
        loss.approximated("InTC/OutTC 量化到 1/nominal 帧网格（BDN 固有精度）");
        for _ in 0..quantized {
            loss.bump("user_attention", "事件时间不落在帧网格上");
        }
    }

    // 单个 graphic 超过 2K 也已在 PGS 之外（13 §7：BDN 是中间表示，不是合规声明）。
    for ev in &doc.events {
        let g = &ev.graphic;
        if g.width > BLURAY_MAX.width || g.height > BLURAY_MAX.height {
            loss.bump(
                "nonstandard",
                &format!(
                    "graphic {} 尺寸 {}x{} 超 {}",
                    g.file_name, g.width, g.height, BLURAY_MAX
                ),
            );
        }
    }
    loss
}

/// 时间是否恰好落在 BDN timecode 能表达的网格上。
fn timecode_is_exact(t: Time, fps: Fps) -> bool {
    let Ok(tc) = t.to_bdn_timecode(fps) else {
        return false;
    };
    match tc.to_time(fps) {
        Ok(back) => back == t,
        Err(_) => false,
    }
}
