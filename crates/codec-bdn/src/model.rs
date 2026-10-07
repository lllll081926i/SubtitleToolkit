//! BDN IR（08 §1「BDN IR：metadata；format/frame rate；event InTC/OutTC；graphic
//! path/width/height/x/y」）。
//!
//! 两条不变量由构造函数保证，而不是靠调用方自觉：
//! - `BdnEvent::in_tc`/`out_tc` 只能来自 `Time::to_bdn_timecode(fps)`（单一时间源，10 §7）。
//!   写出路径用 `BdnEvent::new`（内部现算）；解析路径用 `BdnEvent::from_timecodes`（把文件里的
//!   时间码用同一个 fps 反算成 Time）。`BdnDocument::timecodes_are_derived` 是这条不变量的守卫。
//! - `BdnGraphic::width`/`height` 必须等于 PNG 的真实像素尺寸（13 §7 / AT-BDN-001）。

use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::time::{BdnTimecode, Fps, Rational, Time};
use serde::{Deserialize, Serialize};

/// BDN 文档头：`<BDN Version>` + `<BDNFormat>`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnMeta {
    /// `Version` 属性原样保留（0.93 之外的值也接受，只做往返）。
    pub version: String,
    /// 导出名（`<Name>` / 根属性 `Name`；缺失时 `"BDN"`）。
    pub name: String,
    /// 语言码（根属性 `Xml:lang` / `<LanguageCode>`；缺失时 `"eng"`）。
    pub language: String,
    /// `<VideoFormat>`：`720p` / `1080p` / `2160p` / `custom`。
    pub video_format: String,
    /// `<FrameRate>` 反解出的**精确有理数**：`"23.976"` 必须是 24000/1001，不是 2997/125。
    pub frame_rate: Rational,
    /// `<DropFrame>`；BDN 的 timecode 没有 NTSC 丢帧语义，保留只为如实往返。
    pub drop_frame: bool,
    /// `<DisplayResolution>`：完整 16:9 画布（09 §1），不是 PNG 尺寸。
    pub width: u32,
    pub height: u32,
}

impl Default for BdnMeta {
    fn default() -> Self {
        BdnMeta {
            version: "0.93".to_string(),
            name: "BDN".to_string(),
            language: "eng".to_string(),
            video_format: "1080p".to_string(),
            frame_rate: Fps::NTSC_23976.as_rational(),
            drop_frame: false,
            width: 1920,
            height: 1080,
        }
    }
}

impl BdnMeta {
    /// 时间码的唯一帧率来源：任何 `to_bdn_timecode` / `to_time` 都必须走这里。
    pub fn fps(&self) -> Result<Fps> {
        Fps::new(self.frame_rate).map_err(|e| {
            StkError::simple(Domain::Bdn, 66, "BDN FrameRate 无法作为帧率使用")
                .with_detail(format!("{}", self.frame_rate))
                .with_detail(format!("{e}"))
        })
    }

    /// 完整 16:9 画布（09 §1）。读进来的 SD/非 16:9 文件允许存在，但不会被写成输出。
    pub fn canvas(&self) -> Result<Canvas> {
        let c = Canvas::new(self.width, self.height).map_err(|_| {
            StkError::simple(Domain::Bdn, 64, "BDN DisplayResolution 非法")
                .with_detail(format!("{}x{}", self.width, self.height))
        })?;
        if !c.is_16_9() {
            return Err(StkError::simple(Domain::Bdn, 65, "BDN 画布必须是完整 16:9")
                .with_detail(c.label())
                .with_action("09 §1：特效字幕输出永远是完整 16:9，不缩到有效画面高度"));
        }
        Ok(c)
    }
}

/// 一个 BDN 图形对象（13 §7：尺寸 = PNG 真实尺寸，x/y 属于完整 16:9 画布）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnGraphic {
    /// PNG 文件名（相对 XML 所在目录；也允许绝对路径，见 [`crate::validate_dir`]）。
    pub file_name: String,
    /// 必须等于 PNG IHDR 的实际宽度；见 [`crate::check_png_size`]。
    pub width: u32,
    pub height: u32,
    /// 相对完整 16:9 画布左上角的位置。
    pub x: i32,
    pub y: i32,
}

impl BdnGraphic {
    /// 只接受正的 PNG 实际尺寸：0 边会写出"XML 声明 0 而 PNG 不可能是 0"的必然不一致。
    pub fn new(
        file_name: impl Into<String>,
        png_width: u32,
        png_height: u32,
        x: i32,
        y: i32,
    ) -> Result<BdnGraphic> {
        let file_name = file_name.into();
        if png_width == 0 || png_height == 0 {
            return Err(StkError::simple(Domain::Bdn, 64, "PNG 实际尺寸含 0 边")
                .with_detail(format!("{png_width}x{png_height}"))
                .with_detail(file_name));
        }
        Ok(BdnGraphic {
            file_name,
            width: png_width,
            height: png_height,
            x,
            y,
        })
    }

    /// 放置盒必须整体落在完整 16:9 画布内（09 §3 的 bbox 规则）。全部 checked_*。
    pub fn fits_in(&self, canvas: Canvas) -> Result<()> {
        let left = i64::from(self.x);
        let top = i64::from(self.y);
        let right = left
            .checked_add(i64::from(self.width))
            .ok_or_else(|| self.overflow("右边界", canvas))?;
        let bottom = top
            .checked_add(i64::from(self.height))
            .ok_or_else(|| self.overflow("下边界", canvas))?;
        let (cw, ch) = (i64::from(canvas.width), i64::from(canvas.height));
        if left < 0 || top < 0 || right > cw || bottom > ch {
            return Err(
                StkError::simple(Domain::Bdn, 63, "graphic 越过完整 16:9 画布边界")
                    .with_detail(format!(
                        "{}: box ({left},{top})-({right},{bottom}) 不在 0x0-{cw}x{ch} 内",
                        self.file_name
                    ))
                    .with_action("调整 offset_x/offset_y，或改选更大的 16:9 输出画布"),
            );
        }
        Ok(())
    }

    fn overflow(&self, axis: &str, canvas: Canvas) -> StkError {
        StkError::simple(Domain::Bdn, 63, format!("graphic {axis}计算溢出").as_str())
            .with_detail(self.file_name.clone())
            .with_detail(canvas.label())
    }
}

/// 时间码以 `HH:MM:SS:FF` 字符串进出 JSON，避免给 core-model 的 `BdnTimecode` 加 serde。
mod tc_string {
    use core_model::time::BdnTimecode;
    use serde::{de, Deserialize, Deserializer};

    pub fn serialize<S: serde::Serializer>(tc: &BdnTimecode, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&tc.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BdnTimecode, D::Error> {
        let raw = String::deserialize(d)?;
        BdnTimecode::parse(&raw).map_err(de::Error::custom)
    }
}

/// 一个 BDN 事件 = 一个显示时段 + 一个 graphic。
///
/// `in_tc`/`out_tc` 是 `start`/`end` 在同一 `fps` 下的派生视图（单一时间源），不是第二份真相：
/// [`crate::write`] 永远用 `start`/`end` 现算，绝不复用这两个字段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnEvent {
    #[serde(with = "tc_string")]
    pub in_tc: BdnTimecode,
    #[serde(with = "tc_string")]
    pub out_tc: BdnTimecode,
    pub start: Time,
    pub end: Time,
    pub graphic: BdnGraphic,
}

impl BdnEvent {
    /// 写出/渲染路径：时间码由 `start`/`end` 经同一 `fps` 生成，调用方无法塞入第二套舍入。
    pub fn new(start: Time, end: Time, graphic: BdnGraphic, fps: Fps) -> Result<BdnEvent> {
        if end <= start {
            return Err(StkError::simple(Domain::Bdn, 59, "BDN 事件结束不晚于开始")
                .with_detail(format!("graphic {}", graphic.file_name))
                .with_action("零时长/倒序事件在 BDN 里会写成相同的 InTC/OutTC，播放器行为未定义"));
        }
        Ok(BdnEvent {
            in_tc: start.to_bdn_timecode(fps)?,
            out_tc: end.to_bdn_timecode(fps)?,
            start,
            end,
            graphic,
        })
    }

    /// 解析路径：文件里的时间码用 `to_time(fps)` 反算成 `start`/`end`。
    ///
    /// 在同一 fps 下 `to_bdn_timecode` 与 `to_time` 互为逆运算（`FF < nominal` 时），
    /// 所以保留原时间码与反算出的 Time 必然自洽。
    pub(crate) fn from_timecodes(
        in_tc: BdnTimecode,
        out_tc: BdnTimecode,
        graphic: BdnGraphic,
        fps: Fps,
    ) -> Result<BdnEvent> {
        let start = in_tc.to_time(fps)?;
        let end = out_tc.to_time(fps)?;
        if end <= start {
            return Err(StkError::simple(Domain::Bdn, 59, "BDN 事件结束不晚于开始")
                .with_detail(format!("{in_tc} -> {out_tc}")));
        }
        Ok(BdnEvent {
            in_tc,
            out_tc,
            start,
            end,
            graphic,
        })
    }
}

/// `build_document` 的输入：渲染器交出的一帧 PNG。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnFrame {
    pub start: Time,
    pub end: Time,
    pub png_file_name: String,
    /// **PNG 的真实像素尺寸**，不是目标画布尺寸，也不是任何下限值。
    pub png_width: u32,
    pub png_height: u32,
    /// 相对完整 16:9 画布的放置位置。
    pub x: i32,
    pub y: i32,
}

/// `build_document` 的选项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnOptions {
    pub fps: Fps,
    pub language: String,
    pub name: String,
    /// 小于该边长的对象被**丢弃**（13 §7 的小对象/重复帧清理）。
    ///
    /// 决定（40 §审计的旧工程缺陷反例）：`min_axis` 是过滤阈值，**不是**尺寸钳制。
    /// 旧实现按 `max(min_axis, w)` 写出，XML 声明的 Width/Height 于是与 PNG 实际尺寸脱钩，
    /// 正是 AT-BDN-001 要禁止的缺陷。这里 Width/Height 永远原样等于 `png_width`/`png_height`，
    /// `min_axis` 只决定这一帧进不进文档。
    pub min_axis: u32,
    pub offset_x: i32,
    pub offset_y: i32,
}

/// 顶层文档。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BdnDocument {
    pub meta: BdnMeta,
    pub events: Vec<BdnEvent>,
}

impl BdnDocument {
    pub fn new(meta: BdnMeta, events: Vec<BdnEvent>) -> BdnDocument {
        BdnDocument { meta, events }
    }

    /// 第一个不满足「start 严格递增」的下标；[`crate::write`] 用它拒绝未排序输入。
    pub fn first_disorder(&self) -> Option<usize> {
        (1..self.events.len()).find(|&i| self.events[i].start <= self.events[i - 1].start)
    }

    /// 单一时间源守卫：时间码必须等于 `start`/`end` 在同一 fps 下的现算值。
    /// 旧工程"秒字段与帧字段用不同舍入源"的缺陷在这一步就会暴露。
    pub fn timecodes_are_derived(&self) -> Result<()> {
        let fps = self.meta.fps()?;
        for (i, ev) in self.events.iter().enumerate() {
            let expect_in = ev.start.to_bdn_timecode(fps)?;
            let expect_out = ev.end.to_bdn_timecode(fps)?;
            if ev.in_tc != expect_in || ev.out_tc != expect_out {
                return Err(
                    StkError::simple(Domain::Bdn, 61, "BDN 时间码与 start/end 不同源")
                        .with_detail(format!("events[{i}] {}", ev.graphic.file_name))
                        .with_detail(format!(
                            "声明 {} {}，现算 {} {}",
                            ev.in_tc, ev.out_tc, expect_in, expect_out
                        ))
                        .with_action("用 BdnEvent::new 重建事件：单一时间源是本 crate 的硬约束"),
                );
            }
        }
        Ok(())
    }
}

/// 控制字符会写坏 XML（属性里的裸 CR/LF 会被归一化成空格，破坏往返），一律拒绝。
pub(crate) fn reject_control_chars(what: &str, value: &str) -> Result<()> {
    if let Some(c) = value.chars().find(|c| c.is_control()) {
        let title = format!("BDN 字段 {what} 含控制字符");
        return Err(StkError::simple(Domain::Bdn, 68, &title)
            .with_detail(format!("U+{:04X}", c as u32))
            .with_action("BDN 的字符串字段只有语言码/名字/文件名；文本内容都在 PNG 里"));
    }
    Ok(())
}

/// `FileName` 必须非空且不含控制字符（长度上限在 XML 侧已管）。
pub(crate) fn check_file_name(value: &str) -> Result<()> {
    reject_control_chars("FileName", value)?;
    if value.is_empty() {
        return Err(StkError::simple(Domain::Bdn, 55, "BDN Graphic 缺 FileName"));
    }
    Ok(())
}
