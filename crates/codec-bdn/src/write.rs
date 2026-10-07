//! BDN XML 写出（13 §7）。这是"边界转换"发生的地方（08 §2：timecode 只在 writer 边界出现）。
//!
//! 三条硬约束在本模块的落点：
//! - 时间码：只调用 `Time::to_bdn_timecode(fps)`；`write` 先跑 `BdnDocument::timecodes_are_derived`
//!   证明文档内部只有一套舍入源，再逐事件现算文本，绝不复用文档里的时间码字段；
//! - 排序：`BdnDocument::first_disorder` 命中即报错并给位置，**不**在 writer 里排序；
//! - 尺寸：`Width`/`Height` 直接取 `BdnGraphic` 的字段（其构造语义就是 PNG 实际尺寸）。

use core_model::error::{Domain, Result, StkError};

use crate::model::{check_file_name, reject_control_chars, BdnDocument};

/// 写出完整 XML 文本。
///
/// 拒绝而不是修正：
/// - `STK-BDN-0060`：events 未按 `start` 严格递增（附第一个逆序位置）；
/// - `STK-BDN-0061`：文档里声明的时间码与 `start`/`end` 现算值不一致（不同舍入源的指纹）；
/// - `STK-BDN-0059`：事件 `end` 不晚于 `start`；
/// - `STK-BDN-0068`：`Name` / `Language` / `FileName` 含控制字符；
/// - `STK-BDN-0063`/`0064`/`0065`/`0066`：几何、尺寸、画布或帧率非法。
pub fn write(doc: &BdnDocument) -> Result<String> {
    let fps = doc.meta.fps()?;
    let canvas = doc.meta.canvas()?;
    reject_control_chars("Name", &doc.meta.name)?;
    reject_control_chars("Language", &doc.meta.language)?;
    reject_control_chars("VideoFormat", &doc.meta.video_format)?;
    reject_control_chars("Version", &doc.meta.version)?;

    // 单一时间源的可证明性：先确认文档内部自洽，再写。
    doc.timecodes_are_derived()?;

    if let Some(i) = doc.first_disorder() {
        let prev = &doc.events[i - 1];
        let cur = &doc.events[i];
        return Err(
            StkError::simple(Domain::Bdn, 60, "BDN 事件未按 start 严格递增，拒绝写出")
                .with_detail(format!("第一个逆序位置：events[{i}]"))
                .with_detail(format!(
                    "events[{}].InTC={} 之后出现 events[{i}].InTC={}",
                    i - 1,
                    prev.in_tc,
                    cur.in_tc
                ))
                .with_action("排序策略属于 task-engine（15 §批量任务）；writer 绝不代为重排"),
        );
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<BDN");
    push_attr(&mut out, "Version", &doc.meta.version);
    push_attr(&mut out, "Xml:lang", &doc.meta.language);
    out.push_str(">\n<BDNFormat>\n");
    out.push_str("<VideoFormat>");
    out.push_str(&escape(&doc.meta.video_format));
    out.push_str("</VideoFormat>\n");
    // FrameRate 写回预设标签（"23.976"），保证被 `Fps::from_label` 精确读回 24000/1001。
    out.push_str("<FrameRate>");
    out.push_str(&escape(&fps.to_label()));
    out.push_str("</FrameRate>\n");
    out.push_str(if doc.meta.drop_frame {
        "<DropFrame>TRUE</DropFrame>\n"
    } else {
        "<DropFrame>FALSE</DropFrame>\n"
    });
    out.push_str("<DisplayResolution>");
    out.push_str(&format!(
        "<Width>{}</Width><Height>{}</Height>",
        doc.meta.width, doc.meta.height
    ));
    out.push_str("</DisplayResolution>\n");
    out.push_str("<Name>");
    out.push_str(&escape(&doc.meta.name));
    out.push_str("</Name>\n");
    out.push_str("</BDNFormat>\n<BDNSubtitleData>\n");

    for (idx, ev) in doc.events.iter().enumerate() {
        check_file_name(&ev.graphic.file_name)
            .map_err(|e| e.with_detail(format!("events[{idx}]")))?;
        if ev.end <= ev.start {
            return Err(
                StkError::simple(Domain::Bdn, 59, "BDN 事件结束不晚于开始，拒绝写出")
                    .with_detail(format!("events[{idx}] {}", ev.graphic.file_name)),
            );
        }
        ev.graphic
            .fits_in(canvas)
            .map_err(|e| e.with_detail(format!("events[{idx}]")))?;
        // 单一时间源：全 crate 只有下面两行把 Time 变成 timecode 文本。
        let in_tc = ev.start.to_bdn_timecode(fps)?;
        let out_tc = ev.end.to_bdn_timecode(fps)?;
        out.push_str("<Event forced=\"false\"");
        push_attr(&mut out, "InTC", &in_tc.to_string());
        push_attr(&mut out, "OutTC", &out_tc.to_string());
        out.push_str(">\n<Graphic>\n<FileName>");
        out.push_str(&escape(&ev.graphic.file_name));
        out.push_str("</FileName>\n");
        // 尺寸/位置：原样输出，这里没有任何 min_axis 或钳制逻辑。
        out.push_str(&format!(
            "<Width>{}</Width><Height>{}</Height><X>{}</X><Y>{}</Y>\n",
            ev.graphic.width, ev.graphic.height, ev.graphic.x, ev.graphic.y
        ));
        out.push_str("</Graphic>\n</Event>\n");
    }

    out.push_str("</BDNSubtitleData>\n</BDN>\n");
    Ok(out)
}

fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    out.push_str(&escape(value));
    out.push('"');
}

/// 转义 `& < > "`；先处理 `&`，避免二次转义。
fn escape(value: &str) -> String {
    let mut s = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        match c {
            '&' => s.push_str("&amp;"),
            '<' => s.push_str("&lt;"),
            '>' => s.push_str("&gt;"),
            '"' => s.push_str("&quot;"),
            _ => s.push(c),
        }
    }
    s
}
