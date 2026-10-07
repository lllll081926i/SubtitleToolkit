//! 文本字幕 IR（08_FORMATS_AND_IR.md §1）：SRT/WebVTT 这类弱样式字幕共用同一份结构，
//! 后续 SRT→ASS 流水线（14 §SRT→ASS）也从这里取输入，所以字段必须"写得出去也读得回来"。

use core_model::error::{Domain, StkError};
use core_model::time::Time;
use serde::{Deserialize, Serialize};

use crate::encoding::{DetectedEncoding, Newline};

/// 单条 cue。`text` 内换行统一为 `\n`，由 writer 还原成 `TextSubtitle::newline`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextCue {
    /// 文件里的原始序号；缺序号行时为 `None`。写出时按位置连续重排，所以它不承载语义。
    pub index: Option<u32>,
    pub start: Time,
    pub end: Time,
    pub text: String,
}

impl TextCue {
    pub fn new(index: Option<u32>, start: Time, end: Time, text: impl Into<String>) -> TextCue {
        TextCue {
            index,
            start,
            end,
            text: text.into(),
        }
    }
}

/// 解析期归约不出 cue 的块（与 08 §1 的 ASS AST 同源思路：只改目标字段，其余原样 round-trip）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnrecognizedBlock {
    /// 该块位于前 `after_cues` 条 cue 之后，用于写出时回到原始相对位置。
    pub after_cues: usize,
    /// 块内各行以 `\n` 连接，逐字符保留。
    pub raw: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSubtitle {
    pub cues: Vec<TextCue>,
    /// 输入是否带 BOM；写出时按此决定是否补 UTF-8 BOM。
    pub bom: bool,
    pub newline: Newline,
    pub encoding: DetectedEncoding,
    /// 未识别块：内容不丢（23 §健壮性），写出时按 `after_cues` 原位还原。
    pub unrecognized: Vec<UnrecognizedBlock>,
}

impl TextSubtitle {
    /// 批量/CLI 最小能力之一：序号按当前顺序连续重排（`write` 本来也这么写）。
    pub fn renumber(&mut self) {
        for (i, cue) in self.cues.iter_mut().enumerate() {
            // cue 数超过 u32 需要每条 ≥30 字节的数十 GB 输入，实际不可达；这里宁可保留旧值也不静默截断。
            if let Ok(v) = u32::try_from(i.saturating_add(1)) {
                cue.index = Some(v);
            }
        }
    }

    /// 整体偏移（10 §4 Offset：加减固定量，不缩放）。任何一条会变成负时间就整体拒绝，
    /// 让批处理不会留下"改了一半"的 IR（38 §失败恢复）。
    pub fn shift(&mut self, offset_ms: i64) -> Result<(), StkError> {
        // core-model 的 `Rational::new` 对 i64::MIN 取相反数会溢出（已登记问题清单），
        // 偏移量来自 CLI 的任意 i64，这里先挡住，保证任何输入都不 panic。
        // 编号 200：`Domain::Input` 的 110-185 是 CLI 层对外的契约，别的 crate 要报输入问题
        // 就让自己的号不踩进那段（ISSUES H8）。
        if offset_ms == i64::MIN {
            return Err(StkError::simple(Domain::Input, 200, "偏移量超出可处理范围")
                .with_action("使用绝对值更小的毫秒偏移"));
        }
        let delta = Time::from_millis(offset_ms)?;
        let mut moved: Vec<(Time, Time)> = Vec::with_capacity(self.cues.len());
        for cue in &self.cues {
            let start = cue.start.add(&delta)?;
            let end = cue.end.add(&delta)?;
            if start.is_negative() || end.is_negative() {
                return Err(StkError::simple(Domain::Transform, 150, "偏移后出现负时间")
                    .with_detail(format!(
                        "第 {} 条 start={start:?}",
                        moved.len().saturating_add(1)
                    ))
                    .with_action("减小负向偏移量，或先删掉片头被推到 0 之前的 cue"));
            }
            moved.push((start, end));
        }
        for (cue, (start, end)) in self.cues.iter_mut().zip(moved) {
            cue.start = start;
            cue.end = end;
        }
        Ok(())
    }

    /// 严格写出前的合法性检查。
    ///
    /// 规则与理由：
    /// 1. 时长必须为正（拒绝 `end <= start`）。零时长 cue 在多数 SRT 播放器里被整条忽略，
    ///    转成 ASS 后也是空事件——属于"需要用户确认才能落盘"的语义，不是可静默归一的语法。
    /// 2. 起点必须非递减：**允许** `start[n] == end[n-1]`（首尾相接是正常分句），也**允许**
    ///    两条 cue 共享同一起点（forced 行与常规行并存的常见写法），只拒绝真正的倒序。
    /// 3. 序号可选；出现时必须 ≥1 且严格递增（`write` 会整体重排，此检查只防手改出错）。
    /// 4. 文本必须含非空白字符；时间不得为负（`Time` 允许负值只为预览 offset 服务）。
    pub fn validate(&self) -> Result<(), StkError> {
        if self.cues.is_empty() {
            return Err(
                StkError::simple(Domain::Validate, 140, "字幕中没有任何 cue")
                    .with_action("检查输入文件是否被误判编码，或该文件本来只含空白"),
            );
        }
        if self.bom && self.encoding == DetectedEncoding::Ansi {
            return Err(
                StkError::simple(Domain::Validate, 147, "Ansi 字节保真模式不能声明 BOM")
                    .with_action("把 encoding 改为 Utf8/Utf8Bom 并显式做编码转换"),
            );
        }
        let mut prev_start: Option<Time> = None;
        let mut prev_index: Option<u32> = None;
        for (n, cue) in self.cues.iter().enumerate() {
            let pos = n + 1;
            if cue.start.is_negative() || cue.end.is_negative() {
                return Err(StkError::simple(
                    Domain::Validate,
                    141,
                    "cue 时间为负，无法按 SRT 落盘",
                )
                .with_detail(format!("第 {pos} 条"))
                .with_action("用更小的负偏移，或删掉被推到 0 之前的 cue"));
            }
            if cue.end <= cue.start {
                return Err(
                    StkError::simple(Domain::Validate, 142, "cue 结束不晚于开始")
                        .with_detail(format!("第 {pos} 条"))
                        .with_action("修正 end，或删掉这条零时长/倒序 cue"),
                );
            }
            if cue
                .text
                .trim_matches(|c: char| c == ' ' || c == '\t')
                .is_empty()
            {
                return Err(StkError::simple(Domain::Validate, 144, "cue 文本为空")
                    .with_detail(format!("第 {pos} 条"))
                    .with_action("补上文本，或删掉这条空 cue"));
            }
            if let Some(p) = prev_start {
                if cue.start < p {
                    return Err(
                        StkError::simple(Domain::Validate, 143, "cue 起点不是非递减")
                            .with_detail(format!("第 {pos} 条早于前一条"))
                            .with_action("按时间排序后再写出"),
                    );
                }
            }
            if let Some(i) = cue.index {
                if i == 0 {
                    return Err(
                        StkError::simple(Domain::Validate, 145, "cue 序号必须从 1 开始")
                            .with_detail(format!("第 {pos} 条")),
                    );
                }
                if let Some(p) = prev_index {
                    if i <= p {
                        return Err(StkError::simple(
                            Domain::Validate,
                            146,
                            "cue 序号必须严格递增",
                        )
                        .with_detail(format!("第 {pos} 条 index={i} 前值={p}"))
                        .with_action("调用 renumber() 连续重排"));
                    }
                }
                prev_index = Some(i);
            }
            prev_start = Some(cue.start);
        }
        Ok(())
    }
}
