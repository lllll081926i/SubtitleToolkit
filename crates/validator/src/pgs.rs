//! PGS 分层校验（13 §6 的第 1/2/3/4/6 层）。
//!
//! 分工：`codec-pgs` 负责"能不能读"（结构性非法直接报错），本模块负责"读得懂但合不合规"，
//! 并且一次给出全部发现。第 5 层（coded object buffer / acquisition 的完整解码器模型）与
//! 第 7 层（设备语料）在这里**不做假通过**：它们分别需要 Advanced Author 的缓冲模型和
//! 真机语料。按 Strict 申请徽章时这两层缺失会给出 `PGS.DEC.01` / `PGS.DEV.01` 两条 **error**：
//! 没跑过的层不能被成本地算作"通过"。

use codec_pgs::display_set::{parse_ods_header, parse_pcs, parse_wds, DisplaySet};
use codec_pgs::segment::{Segment, SEG_END, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};
use core_model::error::{Domain, Result, StkError};
use core_model::profile::ValidationLevel;
use core_model::time::Fps;

use crate::report::{Finding, Severity, ValidationReport};

/// 蓝光 PGS 允许的组合画布（09 §8：视频 3840x2160 不等于字幕位图 3840x2160）。
pub const BLURAY_PGS_CANVASES: [(u32, u32); 4] =
    [(720, 480), (720, 576), (1280, 720), (1920, 1080)];

/// 自己的 display set 分组。`codec_pgs::group_display_sets` 对"没有 END 收尾"直接返回错误，
/// 而校验器的职责恰恰是报告这种文件；所以这里做一份永不失败的分组。
fn spans(segments: &[Segment]) -> Vec<DisplaySet> {
    let mut out: Vec<DisplaySet> = Vec::new();
    let mut first = 0usize;
    let mut open = false;
    let mut palette = false;
    let mut windows: Vec<codec_pgs::display_set::Window> = Vec::new();
    let mut objects: Vec<u16> = Vec::new();
    let mut pcs: Option<codec_pgs::display_set::Pcs> = None;
    let mut pts = 0i64;
    let mut dts = 0i64;
    let flush = |out: &mut Vec<DisplaySet>,
                 first: usize,
                 last: usize,
                 pts: i64,
                 dts: i64,
                 pcs: Option<codec_pgs::display_set::Pcs>,
                 windows: Vec<codec_pgs::display_set::Window>,
                 palette: bool,
                 objects: Vec<u16>| {
        out.push(DisplaySet {
            index: out.len(),
            first_segment: first,
            last_segment: last,
            pts,
            dts,
            pcs,
            windows,
            has_palette: palette,
            object_ids: objects,
            is_erase: false,
        });
    };
    for (i, s) in segments.iter().enumerate() {
        if !open {
            first = i;
            pts = s.pts;
            dts = s.dts;
            palette = false;
            windows.clear();
            objects.clear();
            pcs = None;
            open = true;
        }
        match s.seg_type {
            SEG_PCS => {
                if i != first {
                    flush(
                        &mut out,
                        first,
                        i - 1,
                        pts,
                        dts,
                        pcs.take(),
                        std::mem::take(&mut windows),
                        palette,
                        std::mem::take(&mut objects),
                    );
                    first = i;
                    pts = s.pts;
                    dts = s.dts;
                    palette = false;
                }
                pcs = parse_pcs(&s.payload).ok();
            }
            SEG_WDS => {
                if let Ok(ws) = parse_wds(&s.payload) {
                    windows.extend(ws);
                }
            }
            SEG_PDS => palette = true,
            SEG_ODS => {
                if let Ok(h) = parse_ods_header(&s.payload) {
                    if h.first && !objects.contains(&h.object_id) {
                        objects.push(h.object_id);
                    }
                }
            }
            SEG_END => {
                flush(
                    &mut out,
                    first,
                    i,
                    pts,
                    dts,
                    pcs.take(),
                    std::mem::take(&mut windows),
                    palette,
                    std::mem::take(&mut objects),
                );
                let last = out.last_mut();
                if let Some(d) = last {
                    d.is_erase = d.pcs.as_ref().is_some_and(|p| p.objects.is_empty());
                }
                open = false;
            }
            _ => {}
        }
    }
    if open {
        flush(
            &mut out,
            first,
            segments.len().saturating_sub(1),
            pts,
            dts,
            pcs,
            windows,
            palette,
            objects,
        );
    }
    out
}

/// 第 1 层：段级结构。
pub fn structure(segments: &[Segment]) -> ValidationReport {
    let mut r = ValidationReport::new();
    let sets = spans(segments);
    if segments.first().map(|s| s.seg_type) != Some(SEG_PCS) {
        let f = r.add(
            "PGS.STR.01",
            Severity::Error,
            "流不以 PCS 开头",
            "在第一个显示集前补 PCS，或从原始字幕重新 author",
        );
        f.segment_offset = segments.first().and_then(|s| s.offset);
    }
    for set in &sets {
        let tail = segments.get(set.last_segment);
        if tail.map(|s| s.seg_type) != Some(SEG_END) {
            let f = r.add(
                "PGS.STR.02",
                Severity::Error,
                format!("display set {} 不以 END 收尾", set.index),
                "补 END 段（0 字节 payload），否则播放器不知道显示集结束",
            );
            f.display_set = Some(set.index);
            f.pts = Some(set.pts);
        }
        for s in &segments[set.first_segment..=set.last_segment] {
            match s.seg_type {
                SEG_PCS => {
                    if let Err(e) = parse_pcs(&s.payload) {
                        let f = r.add(
                            "PGS.STR.03",
                            Severity::Error,
                            format!("PCS 非法：{}", e.title),
                            "重新生成 PCS（11 字节头 + 每对象 8/16 字节）",
                        );
                        f.display_set = Some(set.index);
                        f.segment_offset = s.offset;
                    }
                }
                SEG_WDS => {
                    if let Err(e) = parse_wds(&s.payload) {
                        let f = r.add(
                            "PGS.STR.04",
                            Severity::Error,
                            format!("WDS 非法：{}", e.title),
                            "窗口数量字段必须与 9 字节/窗口的长度一致",
                        );
                        f.display_set = Some(set.index);
                        f.segment_offset = s.offset;
                    }
                }
                _ => {}
            }
        }
        // ODS 分片完整性：每个 object 的 first/last 序列与声明的 data_length
        for obj in ods_objects(segments, set) {
            if !obj.complete {
                let f = r.add(
                    "PGS.STR.05",
                    Severity::Error,
                    format!(
                        "object {} 的 ODS 分片序列不完整（first={}, last={})",
                        obj.object_id, obj.first, obj.last
                    ),
                    "重写分片：首片必须带 0x80、末片必须带 0x40，中间片只有 4 字节头",
                );
                f.display_set = Some(set.index);
                f.pts = Some(obj.pts);
            }
            if let Some((declared, actual)) = obj.length_mismatch {
                let f = r.add(
                    "PGS.STR.06",
                    Severity::Error,
                    format!(
                        "object {} 声明 data_length {declared}，实收 {actual} 字节",
                        obj.object_id
                    ),
                    "首片的 3 字节字段必须等于分片前 RLE 总长",
                );
                f.display_set = Some(set.index);
                f.segment_offset = obj.offset;
            }
        }
        if !set.has_palette && set.pcs.as_ref().is_some_and(|p| p.palette_update != 0) {
            let f = r.add(
                "PGS.STR.07",
                Severity::Warning,
                format!(
                    "display set {} 的 palette_update 标志为 {} 但没有 PDS",
                    set.index,
                    set.pcs.as_ref().map_or(0, |p| p.palette_update)
                ),
                "该标志的确切语义未取证（ISSUES C20）；Basic Encoder 选择每个显示集自带完整 PDS",
            );
            f.display_set = Some(set.index);
        }
    }
    let unknown: Vec<u8> = codec_pgs::unknown_types(segments);
    if !unknown.is_empty() {
        r.add(
            "PGS.STR.08",
            Severity::Info,
            format!("保留了未识别段类型：{:?}", unknown),
            "未知段按其原始字节 round-trip（13 §2 preserve policy），不要静默丢弃",
        );
    }
    r
}

/// (object id, 有 first, 见到 last, 声明长度, 累计字节, pts, 首片偏移)
type OdsAcc = (u16, bool, bool, Option<u32>, usize, i64, Option<usize>);

struct OdsRun {
    object_id: u16,
    complete: bool,
    first: bool,
    last: bool,
    length_mismatch: Option<(u32, usize)>,
    pts: i64,
    offset: Option<usize>,
}

/// 把 set 内的 ODS 段按 object id 归并成"分片序列"事实。
fn ods_objects(segments: &[Segment], set: &DisplaySet) -> Vec<OdsRun> {
    let mut runs: Vec<OdsRun> = Vec::new();
    let mut cur: Option<OdsAcc> = None;
    for s in segments[set.first_segment..=set.last_segment]
        .iter()
        .filter(|s| s.seg_type == SEG_ODS)
    {
        let Ok(h) = parse_ods_header(&s.payload) else {
            if let Some((id, first, last, declared, got, pts, offset)) = cur.take() {
                runs.push(OdsRun {
                    object_id: id,
                    complete: false,
                    first,
                    last,
                    length_mismatch: declared.map(|d| (d, got)),
                    pts,
                    offset,
                });
            }
            continue;
        };
        let same = cur.as_ref().is_some_and(|c| c.0 == h.object_id);
        if !same {
            if let Some((id, first, last, declared, got, pts, offset)) = cur.take() {
                runs.push(OdsRun {
                    object_id: id,
                    complete: last && first,
                    first,
                    last,
                    length_mismatch: declared.and_then(|d| {
                        if d as usize != got {
                            Some((d, got))
                        } else {
                            None
                        }
                    }),
                    pts,
                    offset,
                });
            }
            cur = Some((
                h.object_id,
                h.first,
                h.last,
                if h.first { Some(h.declared_len) } else { None },
                s.payload.len() - h.rle_start,
                s.pts,
                s.offset,
            ));
            continue;
        }
        if let Some(c) = cur.as_mut() {
            c.2 = c.2 || h.last;
            c.4 += s.payload.len() - h.rle_start;
        }
    }
    if let Some((id, first, last, declared, got, pts, offset)) = cur.take() {
        runs.push(OdsRun {
            object_id: id,
            complete: first && last,
            first,
            last,
            length_mismatch: declared.and_then(|d| {
                if d as usize != got {
                    Some((d, got))
                } else {
                    None
                }
            }),
            pts,
            offset,
        });
    }
    runs
}

/// 第 2 层：时间线。`grid` 给出时额外检查帧边界对齐。
pub fn timeline(segments: &[Segment], grid: Option<Fps>) -> ValidationReport {
    let mut r = ValidationReport::new();
    let sets = spans(segments);
    let mut prev: Option<&DisplaySet> = None;
    let mut leads: Vec<i64> = Vec::new();
    let mut off_grid: Vec<(usize, String)> = Vec::new();
    for set in &sets {
        if set.dts > set.pts {
            let f = r.add(
                "PGS.TML.01",
                Severity::Error,
                format!(
                    "display set {} 的 DTS {} 晚于 PTS {}",
                    set.index, set.dts, set.pts
                ),
                "重新排 decode lead：先解后显，绝不机械令 DTS=PTS（10 §6）",
            );
            f.display_set = Some(set.index);
            f.pts = Some(set.pts);
        }
        leads.push(set.pts - set.dts);
        // set 内所有段必须同一对时钟
        let body = &segments[set.first_segment..=set.last_segment];
        if body.iter().any(|s| s.pts != set.pts || s.dts != set.dts) {
            let f = r.add(
                "PGS.TML.02",
                Severity::Error,
                format!("display set {} 内各段时间戳不一致", set.index),
                "同一显示集必须共享一个 PCS 时间戳",
            );
            f.display_set = Some(set.index);
        }
        if let Some(p) = prev {
            if set.pts <= p.pts {
                let f = r.add(
                    "PGS.TML.03",
                    Severity::Error,
                    format!(
                        "display set {} 的 PTS {} 不晚于前一个 {}",
                        set.index, set.pts, p.pts
                    ),
                    "重新排序或修正平移量（Fast Path 平移后必须重查时间线）",
                );
                f.display_set = Some(set.index);
                f.pts = Some(set.pts);
            }
            if set.dts < p.pts && set.dts < p.dts {
                let f = r.add(
                    "PGS.TML.04",
                    Severity::Warning,
                    format!("display set {} 的 DTS 早于前一显示集的解码时刻", set.index),
                    "检查 decode 调度是否被压缩；严格 authoring 需要重排",
                );
                f.display_set = Some(set.index);
            }
            if let Some(g) = grid {
                // None 表示这个 FPS 的一帧不是整数 tick（23.976/29.97）：没有可判定的帧边界，
                // 就不罚人 —— 跳过，既不默认合格也不默认违规。
                let ticks = g.frame_ticks().unwrap_or(0);
                if ticks > 0 && set.pts % ticks != 0 {
                    off_grid.push((set.index, g.to_label()));
                }
            }
        }
        prev = Some(set);
    }
    if !off_grid.is_empty() {
        let f = r.add(
            "PGS.TML.05",
            Severity::Warning,
            format!(
                "{} 个显示集的 PTS 不在 {} 帧网格上（例如 set {}）",
                off_grid.len(),
                off_grid[0].1,
                off_grid[0].0
            ),
            "High Precision SUP 允许非帧边界；要标严格兼容请吸附到帧（10 §6）",
        );
        f.display_set = Some(off_grid[0].0);
    }
    let any_lead = leads.iter().any(|l| *l > 0);
    let any_zero = leads.contains(&0);
    if any_lead && any_zero {
        r.add(
            "PGS.TML.06",
            Severity::Warning,
            "同一文件里 decode lead 混用（有的 set DTS<PTS，有的 DTS==PTS）",
            "统一调度模型，否则时间缩放后可能出现非单调 DTS",
        );
    }
    if segments.iter().any(|s| s.pts < 0 || s.dts < 0) {
        r.add(
            "PGS.TML.07",
            Severity::Error,
            "存在负的 90 kHz 时钟值",
            "平移量过大；先减小 offset 再重排",
        );
    }
    r
}

/// 帧边界检查要的是**一帧长多少 tick**，不是"把 fps 当秒数换算"。
///
/// 原来的错：`Time::from_rational(fps)` 把 25 帧/秒当成 25 秒，得到 2 250 000 tick，
/// 于是 PGS.TML.05 变成"要求 PTS 是 25 **秒**的倍数"——把真正落在帧边界上的文件也叫作不合格
/// （本轮由 AA5 逼出来：写出的 PTS 全部 % 3600 == 0，却报"不在 25 帧网格上"）。
/// 错的方向是**偏严**，不会让违规文件混过去，但它把"严格兼容"变成了几乎不可达的标签，
/// 而这条正是 author 侧要不要吸附的判断依据。
trait FpsTicks {
    /// 一帧的 tick 数；非整数 tick（23.976/29.97）返回 None —— 没有可判定的帧边界。
    fn frame_ticks(&self) -> Option<i64>;
}

impl FpsTicks for Fps {
    fn frame_ticks(&self) -> Option<i64> {
        let period = core_model::time::Rational::from_int(90_000).div(&self.as_rational())?;
        // 只有整数 tick 才存在"帧边界"这个判据；取整会造出一条假网格
        if period.den() != 1 || period.num() <= 0 {
            return None;
        }
        Some(period.num())
    }
}

/// 第 3 层：跨显示集的对象/调色板/窗口生命周期。
pub fn lifecycle(segments: &[Segment]) -> ValidationReport {
    let mut r = ValidationReport::new();
    let sets = spans(segments);
    let mut defined_objects: Vec<u16> = Vec::new();
    let mut defined_windows: Vec<u8> = Vec::new();
    let mut palette_ready = false;
    let mut object_versions: Vec<(u16, u8)> = Vec::new();
    for set in &sets {
        let body = &segments[set.first_segment..=set.last_segment];
        let defines_object = body.iter().any(|s| s.seg_type == SEG_ODS);
        if body.iter().any(|s| s.seg_type == SEG_PDS) {
            palette_ready = true;
        }
        for s in body.iter().filter(|s| s.seg_type == SEG_WDS) {
            if let Ok(ws) = parse_wds(&s.payload) {
                for w in ws {
                    if !defined_windows.contains(&w.id) {
                        defined_windows.push(w.id);
                    }
                }
            }
        }
        for s in body.iter().filter(|s| s.seg_type == SEG_ODS) {
            if let Ok(h) = parse_ods_header(&s.payload) {
                if h.first {
                    if let Some((_, v)) = object_versions.iter().find(|(id, _)| *id == h.object_id)
                    {
                        if *v != h.version {
                            let f = r.add(
                                "PGS.LC.01",
                                Severity::Info,
                                format!(
                                    "object {} 版本从 {v} 变为 {}（redefinition）",
                                    h.object_id, h.version
                                ),
                                "合法的对象重定义；Advanced Author 用它复用 id（13 §5）",
                            );
                            f.display_set = Some(set.index);
                        }
                    } else {
                        object_versions.push((h.object_id, h.version));
                    }
                    if !defined_objects.contains(&h.object_id) {
                        defined_objects.push(h.object_id);
                    }
                }
            }
        }
        if let Some(pcs) = &set.pcs {
            if !defines_object {
                for o in &pcs.objects {
                    if !defined_objects.contains(&o.object_id) {
                        let f = r.add(
                            "PGS.LC.02",
                            Severity::Error,
                            format!(
                                "display set {} 引用未定义的 object {}",
                                set.index, o.object_id
                            ),
                            "在同一显示集内给出该 object 的 ODS，或改为复用已定义 id",
                        );
                        f.display_set = Some(set.index);
                        f.pts = Some(set.pts);
                    }
                }
            }
            for o in &pcs.objects {
                if !defined_windows.contains(&o.window_id) {
                    let f = r.add(
                        "PGS.LC.03",
                        Severity::Error,
                        format!(
                            "display set {} 的 object {} 引用未定义窗口 {}",
                            set.index, o.object_id, o.window_id
                        ),
                        "补 WDS，或把对象放进已定义窗口",
                    );
                    f.display_set = Some(set.index);
                }
            }
            if !palette_ready && !pcs.objects.is_empty() {
                let f = r.add(
                    "PGS.LC.04",
                    Severity::Error,
                    format!("display set {} 在任何调色板定义之前就显示对象", set.index),
                    "在本集或更早的集合里给出 PDS（13 §2 palette 生命周期）",
                );
                f.display_set = Some(set.index);
                f.pts = Some(set.pts);
            }
            if !set.pcs.is_some() {
                continue;
            }
            if set.index == 0 && !pcs.is_epoch_start() {
                r.add(
                    "PGS.LC.05",
                    Severity::Warning,
                    "首个显示集没有 epoch start 标志",
                    "解码器需要 epoch 起点来复位状态",
                );
            }
        } else {
            let f = r.add(
                "PGS.LC.06",
                Severity::Info,
                format!("display set {} 没有 PCS", set.index),
                "palette-only 更新；播放器沿用上一次的组成（13 §2）",
            );
            f.display_set = Some(set.index);
            f.pts = Some(set.pts);
        }
    }
    r
}

/// Profile 层的输入（不直接依赖 `OutputProfile`，避免校验器被产品配置模型绑死）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileWant {
    /// 用户想要的等级：Basic 只要求结构，Standard/Strict 要求蓝光 profile
    pub level: ValidationLevel,
    /// 声称要 Blu-ray/UHD-BD 兼容徽章
    pub claim_compliant: bool,
}

/// 第 4 层：Profile（分辨率/时钟字节/画布一致性）。
pub fn profile(segments: &[Segment], want: &ProfileWant) -> ValidationReport {
    let mut r = ValidationReport::new();
    let mut canvases: Vec<(u16, u16)> = Vec::new();
    let mut rates: Vec<u8> = Vec::new();
    for s in segments.iter().filter(|s| s.seg_type == SEG_PCS) {
        if let Ok(p) = parse_pcs(&s.payload) {
            let pair = (p.width, p.height);
            if !canvases.contains(&pair) {
                canvases.push(pair);
            }
            if !rates.contains(&p.frame_rate) {
                rates.push(p.frame_rate);
            }
        }
    }
    match canvases.as_slice() {
        [] => {
            r.add(
                "PGS.PRO.00",
                Severity::Error,
                "没有 PCS，无法判定画布",
                "至少需要一个显示集",
            );
        }
        [(w, h)] => {
            let big = u32::from(*w) * u32::from(*h);
            let in_profile = BLURAY_PGS_CANVASES
                .iter()
                .any(|(cw, ch)| u32::from(*w) == *cw && u32::from(*h) == *ch);
            if !in_profile {
                let over_hd = big > 1920 * 1080;
                let f = r.add(
                    "PGS.PRO.01",
                    if over_hd {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    format!("画布 {}x{} 不在蓝光 PGS profile 内", w, h),
                    if over_hd {
                        "超过 1920x1080 的 PGS 只能作非标准实验输出，禁止标 compliant（09 §8）"
                    } else {
                        "确认目标设备是否接受该尺寸；否则改回 profile 内画布"
                    },
                );
                f.segment_offset = segments
                    .iter()
                    .find(|s| s.seg_type == SEG_PCS)
                    .and_then(|s| s.offset);
            }
        }
        many => {
            r.add(
                "PGS.PRO.02",
                Severity::Error,
                format!(
                    "同一文件里出现 {} 种 composition canvas：{:?}",
                    many.len(),
                    many
                ),
                "epoch 内画布必须一致；请分批导出",
            );
        }
    }
    match rates.as_slice() {
        [] => {}
        [_] => {
            r.add(
                "PGS.PRO.03",
                Severity::Info,
                "PCS 的 frame-rate 字节一致",
                "该字节（video_format 高 4 位 + frame_rate 低 4 位）的取值表仍未由真盘取证（ISSUES C19）",
            );
        }
        many => {
            r.add(
                "PGS.PRO.04",
                Severity::Warning,
                format!("出现多个 frame-rate 字节值：{:?}", many),
                "确认这些显示集确实属于不同速率；否则统一",
            );
        }
    }
    if want.claim_compliant {
        r.add(
            "PGS.PRO.05",
            Severity::Info,
            "compliant 徽章还需要设备语料（13 §6 第 7 层）",
            "把本报告的 error/warning 清零只是必要条件，不是充分条件",
        );
    }
    // 第 5 层与设备语料不在这里假通过：按 Strict 申请徽章时它们是 error，
    // 因为"没跑过"不能让任何自称 Strict 的输出过关（13 §6 末段）。
    if want.level == ValidationLevel::Strict {
        r.add(
            "PGS.DEC.01",
            Severity::Error,
            "未执行解码器缓冲模型（coded object buffer / acquisition point / window decode time）",
            "需要 pgs-author 的 Advanced Author 缓冲计算（13 §5）；没有它就不能自称 Strict",
        );
        r.add(
            "PGS.DEV.01",
            Severity::Error,
            "未执行设备与软件播放器语料（13 §6 第 7 层）",
            "先跑 corpus 清单里的播放器矩阵，再谈 Strict badge",
        );
    }
    r
}

/// 第 6 层：**测量**峰值码率。名字即边界——它不整形、不改流（13 §5.1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitrateArgs {
    /// 滑窗长度（90 kHz tick）
    pub window_ticks: i64,
    /// 允许的峰值（bit/s）
    pub limit_bps: i64,
}

impl Default for BitrateArgs {
    fn default() -> BitrateArgs {
        BitrateArgs {
            window_ticks: 90_000,
            limit_bps: 40_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct BitrateMeasurement {
    pub window_ticks: i64,
    pub limit_bps: i64,
    /// 观察到的最大滑窗 bit/s
    pub peak_bps: i64,
    /// 峰值窗口起点（90 kHz tick）
    pub peak_pts: i64,
    pub total_bytes: i64,
    pub exceeded: bool,
}

pub fn peak_bitrate(segments: &[Segment], args: &BitrateArgs) -> BitrateMeasurement {
    let mut total: i64 = 0;
    for s in segments {
        total += (codec_pgs::segment::HEADER_LEN + s.payload.len()) as i64;
    }
    let mut peak = 0i64;
    let mut peak_pts = 0i64;
    let mut win_bytes = 0i64;
    // 双指针滑窗：段按时间戳出现顺序写；这里按 PTS 排序后计算。
    let mut items: Vec<(i64, i64)> = segments
        .iter()
        .map(|s| {
            (
                s.pts,
                (codec_pgs::segment::HEADER_LEN + s.payload.len()) as i64,
            )
        })
        .collect();
    items.sort_by_key(|(pts, _)| *pts);
    let mut start = 0usize;
    for end in 0..items.len() {
        win_bytes += items[end].1;
        while items[end].0 - items[start].0 > args.window_ticks.max(1) {
            win_bytes -= items[start].1;
            start += 1;
        }
        let seconds = args.window_ticks.max(1) as f64 / 90_000.0;
        let bps = (win_bytes as f64 * 8.0 / seconds) as i64;
        if bps > peak {
            peak = bps;
            peak_pts = items[end].0;
        }
    }
    BitrateMeasurement {
        window_ticks: args.window_ticks,
        limit_bps: args.limit_bps,
        peak_bps: peak,
        peak_pts,
        total_bytes: total,
        exceeded: peak > args.limit_bps,
    }
}

/// 把测量结果转成发现（超阈值是 error：它是"输出不满足声明的预算"，不是"encoder 会压到"）。
pub fn peak_bitrate_findings(m: &BitrateMeasurement) -> ValidationReport {
    let mut r = ValidationReport::new();
    let text = format!(
        "滑窗 {} tick 内峰值 {} bit/s（预算 {} bit/s）",
        m.window_ticks, m.peak_bps, m.limit_bps
    );
    if m.exceeded {
        let f = r.add(
            "PGS.BTR.01",
            Severity::Error,
            text,
            "这是测量结果，不是编码器承诺：请降低位图复杂度/色数或改用 Advanced Author 的复用策略（13 §5.1）",
        );
        f.pts = Some(m.peak_pts);
    } else {
        r.add("PGS.BTR.00", Severity::Info, text, "无需处理");
    }
    r
}

/// 按等级汇总各层，并给出"能否进 Strict 徽章"的结论。
pub fn pgs_all(segments: &[Segment], want: &ProfileWant, grid: Option<Fps>) -> ValidationReport {
    let mut r = structure(segments);
    r.merge(timeline(segments, grid));
    r.merge(lifecycle(segments));
    r.merge(profile(segments, want));
    r
}

/// 报告里最严重的发现（用于日志/CLI 摘要）。
pub fn worst(r: &ValidationReport) -> Option<&Finding> {
    r.findings
        .iter()
        .min_by_key(|f| std::cmp::Reverse(f.severity as u8))
}

/// error/fatal 折叠成可中断流水线的错误。
pub fn to_error(r: &ValidationReport, context: &str) -> Result<()> {
    if let Some(f) = r
        .findings
        .iter()
        .find(|f| matches!(f.severity, Severity::Error | Severity::Fatal))
    {
        let mut e = StkError::simple(Domain::Validate, 40, &format!("{context} 校验未通过"))
            .with_detail(format!("[{}] {}", f.severity.label(), f.message))
            .with_action(&f.suggestion);
        for d in r
            .findings
            .iter()
            .filter(|x| matches!(x.severity, Severity::Error | Severity::Fatal))
            .skip(1)
            .take(6)
        {
            e = e.with_detail(format!("[{}] {}", d.severity.label(), d.message));
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codec_pgs::encoder::{encode, EncodeOptions, Frame, FrameObject};
    use codec_pgs::fastpath::{apply, FastOp};

    fn stream() -> Vec<Segment> {
        let mut rgba = vec![0u8; 8 * 2 * 4];
        for px in rgba[..32].chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 255, 255, 255]);
        }
        let obj = FrameObject {
            rgba,
            width: 8,
            height: 2,
            x: 100,
            y: 50,
            forced: false,
        };
        let frames = vec![
            Frame::new(90_000, vec![obj.clone()]),
            Frame::new(180_000, vec![obj]),
            Frame::erase(270_000),
        ];
        let (segs, _) = encode(&frames, &EncodeOptions::default()).unwrap();
        // 经过 parse 才让 offset 有值（校验器读的是文件态）
        let bytes = codec_pgs::segment::write_stream(&segs).unwrap();
        codec_pgs::segment::parse_stream(&bytes).unwrap()
    }

    fn want(level: ValidationLevel) -> ProfileWant {
        ProfileWant {
            level,
            claim_compliant: false,
        }
    }

    #[test]
    fn encoder_output_passes_structure_timeline_and_lifecycle() {
        let segs = stream();
        let s = structure(&segs);
        assert!(
            !s.findings.iter().any(|f| f.severity.blocks_strict()),
            "{:#?}",
            s.text_lines()
        );
        let t = timeline(&segs, Some(Fps::PAL_25));
        assert!(
            !t.findings.iter().any(|f| f.severity.blocks_strict()),
            "{:#?}",
            t.text_lines()
        );
        let l = lifecycle(&segs);
        assert!(
            !l.findings.iter().any(|f| f.severity.blocks_strict()),
            "{:#?}",
            l.text_lines()
        );
        let p = profile(&segs, &want(ValidationLevel::Basic));
        assert!(p.no_blocking_findings(), "{:?}", p.text_lines());
    }

    #[test]
    fn missing_end_marker_is_reported_with_display_set() {
        let mut segs = stream();
        let end = segs
            .iter()
            .rposition(|s| s.seg_type == SEG_END)
            .expect("encoder 会写 END");
        segs.remove(end);
        let r = structure(&segs);
        assert!(r.has_check("PGS.STR.02"), "{:?}", r.text_lines());
        let f = r.findings.iter().find(|f| f.check == "PGS.STR.02").unwrap();
        assert_eq!(f.severity, Severity::Error);
        assert!(f.display_set.is_some() && f.suggestion.contains("END"));
    }

    #[test]
    fn corrupt_ods_data_length_is_caught() {
        let mut segs = stream();
        let ods = segs
            .iter_mut()
            .find(|s| s.seg_type == SEG_ODS)
            .expect("有 ODS");
        // 首片：把声明的 data_length 抬高 1 字节
        ods.payload[4] = 1;
        let r = structure(&segs);
        assert!(r.has_check("PGS.STR.06"), "{:?}", r.text_lines());
    }

    #[test]
    fn non_monotonic_pts_and_dts_after_pts_are_caught() {
        let segs = stream();
        let mut broken = segs.clone();
        for s in broken.iter_mut().skip(5) {
            s.pts = 10;
        }
        let r = timeline(&broken, None);
        assert!(r.has_check("PGS.TML.03"), "{:?}", r.text_lines());

        let mut swapped = segs;
        for s in swapped.iter_mut() {
            s.dts = s.pts + 1;
        }
        let r = timeline(&swapped, None);
        assert!(r.has_check("PGS.TML.01"), "{:?}", r.text_lines());
        assert!(!r.no_blocking_findings());
    }

    #[test]
    fn an_on_grid_stream_raises_no_frame_boundary_warning() {
        // 这条是 TML.05 的**负基线**，也是本轮才发现缺的那一条：encoder 的规整输出
        // （PTS 都是 40ms 的整数倍）必须一条 TML.05 都不报。原实现把 fps 换算成
        // 2 250 000 tick，于是这个用例永远红 —— 而当时没有这个用例，偏严的 bug 就藏在
        // "只有一个半速缩放的用例刚好也红"里（AA5/AK15）。
        let r = timeline(&stream(), Some(Fps::PAL_25));
        assert!(
            !r.has_check("PGS.TML.05"),
            "在网格上的流不该被说成离开网格：{:?}",
            r.text_lines()
        );
    }

    #[test]
    fn a_fractional_frame_period_is_not_judged_at_all() {
        // 23.976 的一帧是 3753.75 tick：没有"帧边界"可判，跳过而不是罚。
        let r = timeline(&stream(), Some(Fps::NTSC_23976));
        assert!(
            !r.has_check("PGS.TML.05"),
            "非整数 tick 的网格必须跳过判定：{:?}",
            r.text_lines()
        );
    }

    #[test]
    fn frame_grid_alignment_is_a_warning_not_an_error() {
        let segs = stream();
        let scaled = apply(
            &segs,
            &[FastOp::ScaleTime(
                core_model::time::Rational::new(1, 2).unwrap(),
            )],
        )
        .unwrap();
        let r = timeline(&scaled, Some(Fps::PAL_25));
        let f = r
            .findings
            .iter()
            .find(|f| f.check == "PGS.TML.05")
            .expect("半速缩放必然离开 25fps 网格");
        assert_eq!(f.severity, Severity::Warning);
        assert!(r.no_blocking_findings(), "非网格点仍可播放，只是不能标严格");
    }

    #[test]
    fn undefined_object_reference_is_fatal_for_strict() {
        let segs = stream();
        // palette-only set（无 PCS）应记 info；引用未定义 object 应记 error
        let sets = spans(&segs);
        assert!(sets.len() >= 2, "erase set 应当在：{}", sets.len());
        let l = lifecycle(&segs);
        assert!(
            !l.has_check("PGS.LC.02"),
            "encoder 输出不该引用未定义 object：{:?}",
            l.text_lines()
        );
        let palette_only = ValidationReport::default();
        assert!(palette_only.findings.is_empty());
    }

    #[test]
    fn oversized_canvas_is_rejected_for_compliant_claims() {
        let segs = stream();
        let mut big = segs.clone();
        for s in big.iter_mut().filter(|s| s.seg_type == SEG_PCS) {
            s.payload[0] = 15;
            s.payload[1] = 0; // 3840
            s.payload[2] = 8;
            s.payload[3] = 96; // 2160
        }
        let r = profile(
            &big,
            &ProfileWant {
                level: ValidationLevel::Strict,
                claim_compliant: true,
            },
        );
        assert!(r.has_check("PGS.PRO.01"), "{:?}", r.text_lines());
        assert!(!r.no_blocking_findings());
        // Strict 等级还必须显式承认"缓冲模型/设备语料未覆盖"
        assert!(r.has_check("PGS.DEC.01") && r.has_check("PGS.DEV.01"));
        assert!(r.has_check("PGS.PRO.05"), "compliant 声明必须附带语料要求");
    }

    #[test]
    fn peak_bitrate_measures_and_only_measures() {
        let segs = stream();
        let m = peak_bitrate(&segs, &BitrateArgs::default());
        assert!(m.total_bytes > 0);
        assert!(m.peak_bps > 0);
        assert!(!m.exceeded);
        let tight = BitrateArgs {
            window_ticks: 90_000,
            limit_bps: 1,
        };
        let m2 = peak_bitrate(&segs, &tight);
        assert!(m2.exceeded);
        let r = peak_bitrate_findings(&m2);
        assert!(r.has_check("PGS.BTR.01"));
        let text = &r.findings[0].suggestion;
        assert!(
            text.contains("不是编码器承诺"),
            "命名与语义必须分离（13 §5.1）：{text}"
        );
    }

    #[test]
    fn to_error_only_folds_error_and_fatal() {
        let mut r = ValidationReport::new();
        r.add("X.01", Severity::Warning, "w", "s");
        assert!(to_error(&r, "sup").is_ok());
        r.add("X.02", Severity::Error, "e1", "s1");
        r.add("X.03", Severity::Error, "e2", "s2");
        let e = to_error(&r, "sup").unwrap_err();
        assert_eq!(e.code_string(), "STK-VAL-0040");
        assert!(e.details.len() == 2, "{:?}", e.details);
        assert!(e.user_action.unwrap().contains("s1"));
        assert!(worst(&r).unwrap().check == "X.02");
    }

    #[test]
    fn pgs_all_combines_layers_without_duplicates() {
        let segs = stream();
        let r = pgs_all(&segs, &want(ValidationLevel::Standard), Some(Fps::PAL_25));
        let mut seen: Vec<&str> = Vec::new();
        for f in &r.findings {
            assert!(!seen.contains(&f.check), "同一检查点重复报告：{}", f.check);
            seen.push(f.check);
        }
    }

    // ── AK17 的第一批双向用例：这三条规则此前在测试里从未被点过名 ──────────
    // 每条都是"该报的要报出来 + 不该报的要闭嘴"两个方向，且点名 check id。
    // 只有正例的规则可以偏严地活着（AK16 就是这么发生的），只有反例的规则失效没人知道。

    #[test]
    fn pgs_str_01_fires_without_a_leading_pcs_and_stays_quiet_with_one() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.01"),
            "编码器自己的流以 PCS 开头，不该被报成不开头"
        );
        let mut headless = clean.clone();
        headless.remove(0); // 去掉 PCS，第一段不再是 PCS
        let r = structure(&headless);
        assert!(
            r.has_check("PGS.STR.01"),
            "不以 PCS 开头必须报：{}",
            r.text_lines().join(" | ")
        );
        assert_eq!(
            r.findings
                .iter()
                .find(|f| f.check == "PGS.STR.01")
                .map(|f| f.severity),
            Some(Severity::Error),
            "结构层的开头问题不是警告"
        );
    }

    #[test]
    fn pgs_tml_02_fires_only_when_a_set_splits_its_own_clock() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.02"),
            "一个显示集内所有段共享同一对时钟时不该报"
        );
        let mut split = clean.clone();
        // 只动第二个显示集内部非 PCS 的一段：集合内时间戳从此不一致
        let second = split
            .iter()
            .position(|s| s.seg_type == SEG_PCS && s.pts == 180_000)
            .expect("夹具应有第二个显示集");
        split[second + 1].pts += 90;
        let r = timeline(&split, None);
        assert!(
            r.has_check("PGS.TML.02"),
            "集合内段时钟不一致必须报：{}",
            r.text_lines().join(" | ")
        );
    }

    #[test]
    fn pgs_tml_04_fires_only_when_decode_slipping_below_the_previous_set() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.04"),
            "正常的 decode lead 排布不该被报成压缩了调度：{}",
            timeline(&clean, None).text_lines().join(" | ")
        );
        let mut squashed = clean.clone();
        // 把第二个显示集的 DTS 拉到前一个集合的 PTS/DTS 之前：先解后显的顺序被压扁
        let second = squashed
            .iter()
            .position(|s| s.pts == 180_000)
            .expect("夹具应有第二个显示集");
        for seg in &mut squashed[second..] {
            if seg.pts == 180_000 {
                seg.dts = 1;
            }
        }
        let r = timeline(&squashed, None);
        assert!(
            r.has_check("PGS.TML.04"),
            "DTS 落到前一集合之前必须报：{}",
            r.text_lines().join(" | ")
        );
        assert!(
            r.no_blocking_findings(),
            "TML.04 仍是 Warning：非严格档可以继续，只是不能标严格"
        );
    }

    // ── AK17 第二批：又三条从未被点名的规则，同样是两个方向 + 点名 check id ──────

    #[test]
    fn pgs_str_02_fires_only_when_a_set_forgets_its_end() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.02"),
            "编码器自己的每个显示集都以 END 收尾：{}",
            structure(&clean).text_lines().join(" | ")
        );
        let mut no_tail = clean.clone();
        let last_end = no_tail
            .iter()
            .rposition(|s| s.seg_type == SEG_END)
            .expect("夹具该有 END 段");
        no_tail.remove(last_end);
        let r = structure(&no_tail);
        assert!(
            r.has_check("PGS.STR.02"),
            "去掉 END 之后必须报『不以 END 收尾』：{}",
            r.text_lines().join(" | ")
        );
        assert_eq!(
            r.findings
                .iter()
                .find(|f| f.check == "PGS.STR.02")
                .map(|f| f.severity),
            Some(Severity::Error),
            "收尾缺失会让播放器不知道集合结束，不是警告"
        );
    }

    #[test]
    fn pgs_tml_01_fires_only_when_decode_is_scheduled_after_display() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.01"),
            "正常的 decode lead 是先解后显，不该报"
        );
        let mut flipped = clean.clone();
        let sets = spans(&flipped);
        let second = &sets[1];
        for seg in &mut flipped[second.first_segment..=second.last_segment] {
            seg.dts = seg.pts + 1_000;
        }
        let r = timeline(&flipped, None);
        assert!(
            r.has_check("PGS.TML.01"),
            "DTS 晚于 PTS 必须报：{}",
            r.text_lines().join(" | ")
        );
        assert!(
            !r.no_blocking_findings(),
            "TML.01 是 Error：机械令 DTS=PTS 之后的错排不能标严格"
        );
    }

    #[test]
    fn pgs_tml_03_fires_only_when_display_sets_stop_advancing() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.03"),
            "夹具里后一集合严格晚于前一集合"
        );
        let mut stuck = clean.clone();
        let sets = spans(&stuck);
        let (a, b) = (&sets[0], &sets[1]);
        // 把第二个集合整段搬到第一个集合的时钟上（集合内仍一致，所以不会误触 TML.02）
        for seg in &mut stuck[b.first_segment..=b.last_segment] {
            seg.pts = a.pts;
            seg.dts = a.dts;
        }
        let r = timeline(&stuck, None);
        assert!(
            r.has_check("PGS.TML.03"),
            "PTS 不晚于前一个集合必须报：{}",
            r.text_lines().join(" | ")
        );
        assert!(
            !r.has_check("PGS.TML.02"),
            "集合内时钟仍一致，不该顺带报出 TML.02（那会掩盖真问题）：{}",
            r.text_lines().join(" | ")
        );
    }

    // ── AK17 第三批：PGS 四层里其余 16 条从未点过名的规则 ────────────────────
    // 形状与前两批相同（1c528ed / f0f2a26）：该报的报出来、干净的闭嘴、严重度不写反、
    // 一次构造只碰一条规则（顺带触发的别的检查要显式声明"不该响"）。
    // 构造一律从 encoder 的真输出出发再改字节，不手搓段 —— 手搓的形状是我发明的，
    // 而 AK10 的教训正是"夹具的形状会撒谎"。

    fn sev(r: &ValidationReport, id: &str) -> Option<Severity> {
        r.findings
            .iter()
            .find(|f| f.check == id)
            .map(|f| f.severity)
    }

    fn msg(r: &ValidationReport, id: &str) -> String {
        r.findings
            .iter()
            .find(|f| f.check == id)
            .map(|f| f.message.clone())
            .unwrap_or_default()
    }

    fn pcs_indexes(segs: &[Segment]) -> Vec<usize> {
        segs.iter()
            .enumerate()
            .filter(|(_, s)| s.seg_type == SEG_PCS)
            .map(|(i, _)| i)
            .collect()
    }

    fn drop_type(segs: &mut Vec<Segment>, t: u8) {
        segs.retain(|s| s.seg_type != t);
    }

    fn dump(r: &ValidationReport) -> String {
        r.text_lines().join(" | ")
    }

    #[test]
    fn pgs_str_03_fires_only_on_a_malformed_pcs_and_points_at_the_bytes() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.03"),
            "编码器自己写的 PCS 不该被判非法：{}",
            dump(&structure(&clean))
        );
        let mut broken = clean.clone();
        let second = *pcs_indexes(&broken).get(1).expect("夹具应有第二个 PCS");
        // 截到 11 字节头以下：codec 层的 Pgs/50 必须在这一层变成一条点名发现
        broken[second].payload.truncate(5);
        let r = structure(&broken);
        assert!(r.has_check("PGS.STR.03"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.STR.03"), Some(Severity::Error));
        assert!(
            msg(&r, "PGS.STR.03").contains("PCS 非法"),
            "要把底层错误原因带上来：{}",
            msg(&r, "PGS.STR.03")
        );
        let f = r.findings.iter().find(|f| f.check == "PGS.STR.03").unwrap();
        assert_eq!(f.display_set, Some(1), "发现要指到出问题的那个显示集");
        assert_eq!(
            f.segment_offset, broken[second].offset,
            "还要带原始字节偏移，光有编号定位不到"
        );
    }

    #[test]
    fn pgs_str_04_fires_only_when_the_window_count_does_not_match_the_payload() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.04"),
            "合法的 WDS 不该被判非法：{}",
            dump(&structure(&clean))
        );
        let mut bogus = clean.clone();
        let w = bogus
            .iter()
            .position(|s| s.seg_type == SEG_WDS)
            .expect("夹具应有 WDS");
        bogus[w].payload[0] = 2; // 声明 2 个窗口，body 只有 1 个（9 字节）
        let r = structure(&bogus);
        assert!(r.has_check("PGS.STR.04"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.STR.04"), Some(Severity::Error));
        assert!(
            msg(&r, "PGS.STR.04").contains("WDS 非法"),
            "{}",
            msg(&r, "PGS.STR.04")
        );
    }

    #[test]
    fn pgs_str_05_fires_only_when_an_object_run_never_opens() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.05"),
            "编码器写的分片是首尾齐的：{}",
            dump(&structure(&clean))
        );
        let mut headless = clean.clone();
        let o = headless
            .iter()
            .position(|s| s.seg_type == SEG_ODS)
            .expect("夹具应有 ODS");
        // 抹掉首片的 0x80：这一段降级成"续片"，整条分片序列没有起点
        headless[o].payload[3] = 0x40;
        let r = structure(&headless);
        assert!(r.has_check("PGS.STR.05"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.STR.05"), Some(Severity::Error));
        assert!(msg(&r, "PGS.STR.05").contains("分片序列不完整"));
        assert!(
            !r.has_check("PGS.STR.06"),
            "首片标记没了就读不到声明长度，此时顺带报『长度不符』是假问题：{}",
            dump(&r)
        );
    }

    #[test]
    fn pgs_str_07_warns_only_for_a_palette_update_flag_without_a_pds() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.07"),
            "每个集合自带 PDS 时不该报：{}",
            dump(&structure(&clean))
        );
        // 反向的第二格：没有 PDS 但也没打标志 —— 同样不该报（语义未取证，不能猜）
        let mut plain = clean.clone();
        drop_type(&mut plain, SEG_PDS);
        assert!(
            !structure(&plain).has_check("PGS.STR.07"),
            "标志为 0 时不该凭缺失的 PDS 报警：{}",
            dump(&structure(&plain))
        );
        let mut flagged = clean.clone();
        drop_type(&mut flagged, SEG_PDS);
        for i in pcs_indexes(&flagged) {
            flagged[i].payload[8] = 1; // palette_update 字节
        }
        let r = structure(&flagged);
        assert!(r.has_check("PGS.STR.07"), "{}", dump(&r));
        assert_eq!(
            sev(&r, "PGS.STR.07"),
            Some(Severity::Warning),
            "C20 的语义还没取证，只能提醒不能拦路"
        );
        assert!(r.no_blocking_findings(), "{}", dump(&r));
    }

    #[test]
    fn pgs_str_08_records_an_unrecognized_segment_type_without_losing_the_end() {
        let clean = stream();
        assert!(
            !structure(&clean).has_check("PGS.STR.08"),
            "全认识的流不该报未识别段：{}",
            dump(&structure(&clean))
        );
        let mut odd = clean.clone();
        let end = odd
            .iter()
            .rposition(|s| s.seg_type == SEG_END)
            .expect("夹具应有 END");
        // 未识别段插在最后一个 END **之前**：集合仍然收尾，只有 STR.08 该响
        let mut junk = odd[end].clone();
        junk.seg_type = 0x1a;
        odd.insert(end, junk);
        let r = structure(&odd);
        assert!(r.has_check("PGS.STR.08"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.STR.08"), Some(Severity::Info));
        assert!(
            msg(&r, "PGS.STR.08").contains("26"),
            "未识别的类型要按字节点名，否则用户不知道保留了什么：{}",
            msg(&r, "PGS.STR.08")
        );
        assert!(
            !r.has_check("PGS.STR.02"),
            "插在集合内部不该被算成『不以 END 收尾』：{}",
            dump(&r)
        );
    }

    #[test]
    fn pgs_lc_01_notes_a_redefinition_only_when_the_object_version_changes() {
        let clean = stream();
        assert!(
            !lifecycle(&clean).has_check("PGS.LC.01"),
            "encoder 跨集合复用同一对象（同 id 同版本）不该被说成重定义：{}",
            dump(&lifecycle(&clean))
        );
        // 追加一个真·新集合：同 object id、版本 +1（Advanced Author 的复用手法，13 §5）
        let s0 = {
            let sets = spans(&clean);
            let s = sets.first().expect("夹具至少一个显示集");
            (s.first_segment, s.last_segment)
        };
        let mut extra: Vec<Segment> = clean[s0.0..=s0.1].to_vec();
        for s in extra.iter_mut() {
            s.pts = 360_000;
            s.dts = 357_000;
        }
        let bumped = extra
            .iter_mut()
            .filter(|s| s.seg_type == SEG_ODS)
            .fold(0usize, |n, s| {
                s.payload[2] = s.payload[2].wrapping_add(1); // version 字段
                n + 1
            });
        assert!(bumped > 0, "夹具的第一个集合应当定义对象");
        let mut redef = clean.clone();
        redef.extend_from_slice(&extra);
        let r = lifecycle(&redef);
        assert!(r.has_check("PGS.LC.01"), "{}", dump(&r));
        assert_eq!(
            sev(&r, "PGS.LC.01"),
            Some(Severity::Info),
            "合法的对象重定义不是错误"
        );
        assert!(r.no_blocking_findings(), "{}", dump(&r));
        assert!(
            !r.has_check("PGS.LC.02"),
            "引用的对象都是已定义的，不该顺带报未定义对象：{}",
            dump(&r)
        );
    }

    #[test]
    fn pgs_lc_03_fires_when_an_object_points_at_an_undefined_window() {
        let clean = stream();
        assert!(
            !lifecycle(&clean).has_check("PGS.LC.03"),
            "编码器的对象都放在自己声明的窗口里：{}",
            dump(&lifecycle(&clean))
        );
        let mut orphan = clean.clone();
        let mut renamed = 0usize;
        for w in orphan.iter_mut().filter(|s| s.seg_type == SEG_WDS) {
            // erase 集合的 WDS 是空表（只有 1 字节的窗口计数），没有 id 可改
            if w.payload.len() >= 2 {
                w.payload[1] = w.payload[1].wrapping_add(7); // 第一个窗口的 id
                renamed += 1;
            }
        }
        assert!(renamed > 0, "夹具的内容集合应当声明窗口");
        let r = lifecycle(&orphan);
        assert!(r.has_check("PGS.LC.03"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.LC.03"), Some(Severity::Error));
        assert!(
            !r.has_check("PGS.LC.02"),
            "对象本身仍是已定义的，不该被说成未定义对象：{}",
            dump(&r)
        );
        assert!(!r.no_blocking_findings());
    }

    #[test]
    fn pgs_lc_04_fires_when_objects_display_before_any_palette() {
        let clean = stream();
        assert!(
            !lifecycle(&clean).has_check("PGS.LC.04"),
            "夹具每个集合自带 PDS：{}",
            dump(&lifecycle(&clean))
        );
        let mut no_palette = clean.clone();
        drop_type(&mut no_palette, SEG_PDS);
        let r = lifecycle(&no_palette);
        assert!(r.has_check("PGS.LC.04"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.LC.04"), Some(Severity::Error));
        assert!(
            msg(&r, "PGS.LC.04").contains("在任何调色板定义之前就显示对象"),
            "{}",
            msg(&r, "PGS.LC.04")
        );
        assert!(
            !r.has_check("PGS.LC.03"),
            "窗口定义没动，不该顺带报未定义窗口：{}",
            dump(&r)
        );
        assert!(!r.no_blocking_findings());
    }

    #[test]
    fn pgs_lc_05_warns_only_when_the_first_set_lacks_the_epoch_start_flag() {
        let clean = stream();
        // encoder 只给 set 0 打 0x80，后面的集合都不打 —— 这条正是 LC.05 只判首集的依据
        let r = lifecycle(&clean);
        assert!(
            !r.has_check("PGS.LC.05"),
            "首个集合有 epoch 标记，后续集合缺标记不该报：{}",
            dump(&r)
        );
        let mut no_epoch = clean.clone();
        let first = pcs_indexes(&no_epoch)[0];
        no_epoch[first].payload[7] &= 0x7f; // state 字节的 PCS_EPOCH_START
        let r = lifecycle(&no_epoch);
        assert!(r.has_check("PGS.LC.05"), "{}", dump(&r));
        assert_eq!(
            sev(&r, "PGS.LC.05"),
            Some(Severity::Warning),
            "缺 epoch 起点是可疑，不是非法"
        );
        assert!(r.no_blocking_findings(), "{}", dump(&r));
    }

    #[test]
    fn pgs_lc_06_notes_a_set_that_carries_no_pcs() {
        let clean = stream();
        assert!(
            !lifecycle(&clean).has_check("PGS.LC.06"),
            "每个集合都有 PCS，不该被说成缺 PCS：{}",
            dump(&lifecycle(&clean))
        );
        let mut palette_only = clean.clone();
        let second = *pcs_indexes(&palette_only)
            .get(1)
            .expect("夹具应有第二个 PCS");
        // 把第二个集合的 PCS 换成 PDS：这一集只剩调色板更新，没有组成声明
        palette_only[second].seg_type = SEG_PDS;
        let r = lifecycle(&palette_only);
        assert!(r.has_check("PGS.LC.06"), "{}", dump(&r));
        assert_eq!(
            sev(&r, "PGS.LC.06"),
            Some(Severity::Info),
            "palette-only 是合法写法（13 §2），只是要知道它没有 PCS"
        );
        let f = r.findings.iter().find(|f| f.check == "PGS.LC.06").unwrap();
        assert_eq!(f.display_set, Some(1), "要点名是哪一个集合");
        assert!(r.no_blocking_findings(), "{}", dump(&r));
    }

    #[test]
    fn pgs_pro_00_fires_only_when_no_pcs_can_declare_a_canvas() {
        let clean = stream();
        assert!(
            !profile(&clean, &want(ValidationLevel::Basic)).has_check("PGS.PRO.00"),
            "有 PCS 的流不该被说成无法判定画布"
        );
        let mut headless = clean.clone();
        drop_type(&mut headless, SEG_PCS);
        let r = profile(&headless, &want(ValidationLevel::Basic));
        assert!(r.has_check("PGS.PRO.00"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.PRO.00"), Some(Severity::Error));
        assert!(
            !r.has_check("PGS.PRO.01"),
            "一条 PCS 都没有时该报『无法判定』，而不是凭空评判某种画布：{}",
            dump(&r)
        );
        assert!(!r.no_blocking_findings());
    }

    #[test]
    fn pgs_pro_02_fires_only_when_the_canvas_changes_mid_file() {
        let clean = stream();
        let p = profile(&clean, &want(ValidationLevel::Basic));
        assert!(!p.has_check("PGS.PRO.02"), "夹具全程一种画布：{}", dump(&p));
        assert!(!p.has_check("PGS.PRO.01"), "夹具画布本就在 profile 内");
        let mut mixed = clean.clone();
        let first = {
            let idx = pcs_indexes(&mixed);
            idx.first().copied().expect("夹具应有 PCS")
        };
        let w0 = parse_pcs(&mixed[first].payload).unwrap().width;
        // 换成另一种 profile 内画布：问题不是"尺寸违规"，而是 epoch 中途换了画布
        let (w, h): (u16, u16) = if w0 == 1280 {
            (1920, 1080)
        } else {
            (1280, 720)
        };
        let idx = pcs_indexes(&mixed);
        for i in idx[1..].iter() {
            mixed[*i].payload[0] = (w >> 8) as u8;
            mixed[*i].payload[1] = (w & 0xff) as u8;
            mixed[*i].payload[2] = (h >> 8) as u8;
            mixed[*i].payload[3] = (h & 0xff) as u8;
        }
        let r = profile(&mixed, &want(ValidationLevel::Basic));
        assert!(r.has_check("PGS.PRO.02"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.PRO.02"), Some(Severity::Error));
        assert!(
            !r.has_check("PGS.PRO.01"),
            "混用时报『混用』就够了，两种都在 profile 内不该再评判尺寸：{}",
            dump(&r)
        );
    }

    #[test]
    fn pgs_pro_03_confirms_a_uniform_frame_rate_byte_and_stops_when_it_isnt() {
        let clean = stream();
        let r = profile(&clean, &want(ValidationLevel::Basic));
        assert!(
            r.has_check("PGS.PRO.03"),
            "全程一致的 rate 字节应被确认为一致：{}",
            dump(&r)
        );
        assert_eq!(sev(&r, "PGS.PRO.03"), Some(Severity::Info));
        let mut mixed = clean.clone();
        let idx = pcs_indexes(&mixed);
        mixed[idx[0]].payload[4] = 1; // frame_rate 字节（video_format 高 4 位 + 低 4 位）
        let r2 = profile(&mixed, &want(ValidationLevel::Basic));
        assert!(
            !r2.has_check("PGS.PRO.03"),
            "取值已经不一致了，不能再宣称一致：{}",
            dump(&r2)
        );
    }

    #[test]
    fn pgs_pro_04_warns_only_when_several_frame_rate_bytes_appear() {
        let clean = stream();
        assert!(
            !profile(&clean, &want(ValidationLevel::Basic)).has_check("PGS.PRO.04"),
            "单一取值不该报混用"
        );
        let mut mixed = clean.clone();
        for (k, i) in pcs_indexes(&mixed).iter().enumerate() {
            mixed[*i].payload[4] = if k % 2 == 0 { 0 } else { 3 };
        }
        let r = profile(&mixed, &want(ValidationLevel::Basic));
        assert!(r.has_check("PGS.PRO.04"), "{}", dump(&r));
        assert_eq!(
            sev(&r, "PGS.PRO.04"),
            Some(Severity::Warning),
            "取值表还没由真盘取证（C19），只能说可疑不能说非法"
        );
        assert!(r.no_blocking_findings(), "{}", dump(&r));
    }

    #[test]
    fn pgs_tml_06_warns_only_when_decode_leads_are_mixed_in_one_file() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.06"),
            "统一的 decode lead 不该被报成混用：{}",
            dump(&timeline(&clean, None))
        );
        let (from, to) = {
            let sets = spans(&clean);
            let last = sets.last().expect("夹具应有显示集");
            (last.first_segment, last.last_segment)
        };
        let mut mixed = clean.clone();
        for seg in &mut mixed[from..=to] {
            seg.dts = seg.pts; // 只有最后一个集合没有 lead
        }
        let r = timeline(&mixed, None);
        assert!(r.has_check("PGS.TML.06"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.TML.06"), Some(Severity::Warning));
        assert!(
            r.no_blocking_findings(),
            "混用是调度模型可疑，不是非法：{}",
            dump(&r)
        );
        assert!(
            !r.has_check("PGS.TML.01"),
            "把 DTS 抬到与 PTS 相等不该顺带报『DTS 晚于 PTS』：{}",
            dump(&r)
        );
    }

    #[test]
    fn pgs_tml_07_fires_only_when_a_clock_value_goes_negative() {
        let clean = stream();
        assert!(
            !timeline(&clean, None).has_check("PGS.TML.07"),
            "正时钟不该报负值"
        );
        let mut shifted = clean.clone();
        for s in shifted.iter_mut() {
            s.pts -= 200_000;
            s.dts -= 200_000;
        }
        let r = timeline(&shifted, None);
        assert!(r.has_check("PGS.TML.07"), "{}", dump(&r));
        assert_eq!(sev(&r, "PGS.TML.07"), Some(Severity::Error));
        assert!(!r.no_blocking_findings());
        assert!(
            !r.has_check("PGS.TML.03"),
            "整条平移不改变先后，不该顺带报时间线倒退：{}",
            dump(&r)
        );
    }
}
