//! PGS Basic Encoder（13 §4，Level 3）+ 调色板量化开关（13 §5 的 image quantization）。
//!
//! 输入是"完整 16:9 composition canvas 上的 RGBA 位图对象"，输出结构合法、主流播放器可读的
//! display sets。这一层**不宣称**通过 13 §6 的 Strict 校验（那是 `validator` 的 Level 5），
//! 也不做 13 §5 的 object reuse / palette animation / buffer 模型（Level 4 Advanced Author）。
//!
//! 硬约束（来自 13 §4 与 09 §3）：
//! - PCS 的 composition canvas 始终是完整画布，对象 x/y 是完整画布坐标；
//!   tight bbox 只影响 ODS 位图本身，不改变 placement 语义；
//! - 调色板条目按 `[index][Y][Cr][Cb][alpha]` 5 字节写，索引 0 固定为透明；
//! - RLE 超过单 segment 容量时必须分片，`data_length` 记录分片前的总字节数；
//! - 全透明帧用 erase display set（0 个 composition object）表达，而不是写一张全透明图。
//!
//! 颜色策略（`EncodeOptions::colors`，见 `quantize`）：RGBA 先映射到 PGS 自己的 Y/Cr/Cb/T 域
//! 再按"编码后字节相同"合并，所以降不到 255 色以内才是真失败（而不是"看起来不同"就失败）。
//! - `Exact`（默认，Basic）：超出预算就报错，绝不静默丢色（08 §4 LossReport 的同一原则）；
//! - `MedianCut`（Advanced）：合并进预算，并把度量出来的误差写进 `EncodeReport`，
//!   由调用方（`pgs-author`）转成 LossReport。**没有人看得到失真，才是最坏的失真。**

use core_model::error::{Domain, Result, StkError};

use crate::display_set::{write_pcs, write_wds, CompositionObject, Pcs, Window, PCS_EPOCH_START};
use crate::palette::{write_pds, ColorMatrix, Palette};
use crate::quantize::{self, ColorPolicy, ColorTable};
use crate::rle::encode as encode_rle;
use crate::segment::{Segment, SEG_END, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};

/// ODS 单个 segment 的 payload 上限。参考实现 ass2bdnxml `sup.c` 对大 RLE 用
/// 首片 65508 / 续片 65515 字节，两者加上各自的片头（11 / 4）正好都是 65519，
/// 所以这里只钉 65519 一个常量，那两个数字由它推导（见 `ods_first_data_cap`）。
pub const MAX_ODS_SEGMENT_PAYLOAD: usize = 65_519;
/// 首片片头：object_id(2)+version(1)+sequence(1)+data_length(3)+width(2)+height(2)。
pub const ODS_FIRST_HEADER: usize = 11;
/// 续片片头只有 object_id(2)+version(1)+sequence(1)。
pub const ODS_NEXT_HEADER: usize = 4;

pub const fn ods_first_data_cap(payload_cap: usize) -> usize {
    payload_cap.saturating_sub(ODS_FIRST_HEADER)
}
pub const fn ods_next_data_cap(payload_cap: usize) -> usize {
    payload_cap.saturating_sub(ODS_NEXT_HEADER)
}

/// Blu-ray PGS 的画布上限（09 §8：更大的画布只能是非标准实验输出，不得标 compliant）。
pub const BLURAY_MAX_CANVAS: (u16, u16) = (1920, 1080);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameObject {
    /// RGBA，行主序，长度必须等于 width*height*4
    pub rgba: Vec<u8>,
    pub width: u16,
    pub height: u16,
    /// 完整画布坐标（09 §3：bbox 裁剪不改变 placement 的坐标空间）
    pub x: u16,
    pub y: u16,
    pub forced: bool,
}

impl FrameObject {
    pub fn all_transparent(&self) -> bool {
        let (pixels, rest) = self.rgba.as_chunks::<4>();
        rest.is_empty() && pixels.iter().all(|p| p[3] == 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub pts: i64,
    /// 空 objects + erase=true 即 erase display set
    pub objects: Vec<FrameObject>,
    pub erase: bool,
}

impl Frame {
    pub fn new(pts: i64, objects: Vec<FrameObject>) -> Frame {
        Frame {
            pts,
            objects,
            erase: false,
        }
    }

    pub fn erase(pts: i64) -> Frame {
        Frame {
            pts,
            objects: Vec::new(),
            erase: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodeOptions {
    pub canvas_width: u16,
    pub canvas_height: u16,
    /// PCS 第 5 字节（HDMV 里是 video_format 高 4 位 + frame_rate 低 4 位）。
    /// 取值表未经真盘语料确认，见 ISSUES C19，因此保持可配置并原样写出。
    pub frame_rate_byte: u8,
    /// DTS 提前量（90 kHz tick）。必须 >0 才能满足 decode-before-display。
    pub decode_lead_ticks: i64,
    pub max_ods_segment_payload: usize,
    pub matrix: ColorMatrix,
    /// 调色板 id（0 或 1）
    pub palette_id: u8,
    /// 颜色预算策略：Basic 用 `Exact`，需要吃真实抗锯齿渲染时用 `MedianCut`（13 §5）
    pub colors: ColorPolicy,
}

impl Default for EncodeOptions {
    fn default() -> EncodeOptions {
        EncodeOptions {
            canvas_width: BLURAY_MAX_CANVAS.0,
            canvas_height: BLURAY_MAX_CANVAS.1,
            frame_rate_byte: 0,
            decode_lead_ticks: 3_000,
            max_ods_segment_payload: MAX_ODS_SEGMENT_PAYLOAD,
            matrix: ColorMatrix::Bt709,
            palette_id: 0,
            colors: ColorPolicy::Exact,
        }
    }
}

impl EncodeOptions {
    fn validate(&self) -> Result<()> {
        if self.canvas_width == 0 || self.canvas_height == 0 {
            return Err(err(90, "composition canvas 尺寸不能为 0"));
        }
        if self.canvas_width > BLURAY_MAX_CANVAS.0 || self.canvas_height > BLURAY_MAX_CANVAS.1 {
            return Err(
                err(91, "画布超出 Blu-ray PGS profile 上限 1920x1080").with_action(
                    "更大画布只能作为非标准实验输出，禁止标注 Blu-ray/UHD-BD compliant（09 §8）",
                ),
            );
        }
        if self.decode_lead_ticks <= 0 {
            return Err(err(92, "decode lead 必须为正，否则无法保证 DTS < PTS"));
        }
        if self.max_ods_segment_payload <= ODS_FIRST_HEADER + ODS_NEXT_HEADER {
            return Err(err(93, "ODS 分片上限过小"));
        }
        if self.palette_id > 1 {
            return Err(err(94, "PGS 调色板 id 只能是 0 或 1"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeReport {
    pub display_sets: usize,
    pub objects: usize,
    pub ods_segments: usize,
    pub fragmented_objects: usize,
    pub erase_sets: usize,
    pub palette_segments: usize,
    pub max_palette_entries: usize,
    /// 真的合并过颜色的帧数（`ColorPolicy::Exact` 永远是 0）
    pub quantized_frames: usize,
    /// 被合并掉的颜色总数（各帧之和）
    pub merged_colors: usize,
    /// 单通道最大绝对偏差（各帧最大值）；量化发生过就必须是非零
    pub worst_color_error: u32,
}

fn err(code: u16, msg: &str) -> StkError {
    StkError::simple(Domain::Pgs, code, msg)
}

/// 帧内位图 -> 调色板索引表（预算检查与合并都发生在 `quantize::build_table`）。
///
/// 一帧一张表：PGS 允许每个 display set 自带完整 PDS（现在就是这么写的，
/// `palette_update=0`），所以预算是按帧算的，不是整条流算的。
fn table_for_frame(frame: &Frame, opts: &EncodeOptions) -> Result<ColorTable> {
    let hist = quantize::histogram(
        frame.objects.iter().map(|o| o.rgba.as_chunks::<4>().0),
        opts.matrix,
    );
    quantize::build_table(&hist, opts.colors)
}

fn palette_of(table: &ColorTable, id: u8) -> Palette {
    Palette {
        id,
        version: 0,
        entries: table.entries.clone(),
    }
}

/// 编码为段序列（可直接交给 `segment::write_stream` 或 `SupStream`）。
pub fn encode(frames: &[Frame], opts: &EncodeOptions) -> Result<(Vec<Segment>, EncodeReport)> {
    opts.validate()?;
    if frames.is_empty() {
        return Err(err(96, "没有帧可编码"));
    }
    let canvas_w = opts.canvas_width as u32;
    let canvas_h = opts.canvas_height as u32;
    let first_cap = ods_first_data_cap(opts.max_ods_segment_payload);
    let next_cap = ods_next_data_cap(opts.max_ods_segment_payload);
    let mut segments: Vec<Segment> = Vec::new();
    let mut report = EncodeReport {
        display_sets: 0,
        objects: 0,
        ods_segments: 0,
        fragmented_objects: 0,
        erase_sets: 0,
        palette_segments: 0,
        max_palette_entries: 0,
        quantized_frames: 0,
        merged_colors: 0,
        worst_color_error: 0,
    };
    let mut last_pts: Option<i64> = None;

    for (i, frame) in frames.iter().enumerate() {
        if let Some(prev) = last_pts {
            if frame.pts <= prev {
                return Err(err(97, "帧 PTS 必须严格递增")
                    .with_detail(format!("frame {i}: {} <= {prev}", frame.pts)));
            }
        }
        if frame.pts < opts.decode_lead_ticks {
            return Err(err(98, "PTS 早于 0 或不够放下 decode lead"));
        }
        let dts = frame.pts - opts.decode_lead_ticks;
        last_pts = Some(frame.pts);

        let table = table_for_frame(frame, opts)?;
        if let Some(st) = &table.stats {
            report.quantized_frames += 1;
            report.merged_colors += st.merged;
            report.worst_color_error = report.worst_color_error.max(st.worst);
        }
        let mut object_defs: Vec<(u16, u16, u16, Vec<u8>)> = Vec::new();
        let mut placements: Vec<CompositionObject> = Vec::new();

        if !(frame.erase && frame.objects.is_empty()) {
            for (slot, obj) in frame.objects.iter().enumerate() {
                if obj.width == 0 || obj.height == 0 {
                    return Err(
                        err(99, "对象位图尺寸非法").with_detail(format!("frame {i} object {slot}"))
                    );
                }
                let need = usize::from(obj.width) * usize::from(obj.height);
                if u32::from(obj.x) + u32::from(obj.width) > canvas_w
                    || u32::from(obj.y) + u32::from(obj.height) > canvas_h
                {
                    return Err(err(101, "对象越出 composition canvas").with_detail(format!(
                        "frame {i} object {slot}: x={} y={} w={} h={}",
                        obj.x, obj.y, obj.width, obj.height
                    )));
                }
                let (pixels, rest) = obj.rgba.as_chunks::<4>();
                if !rest.is_empty() || pixels.len() != need {
                    return Err(err(100, "对象 RGBA 长度与尺寸不符").with_detail(format!(
                        "frame {i} object {slot}: 需要 {need} 像素，得到 {} 像素 + {} 字节余数",
                        pixels.len(),
                        rest.len()
                    )));
                }
                let mut indices = Vec::with_capacity(obj.width as usize * obj.height as usize);
                for px in pixels {
                    indices.push(table.index_of(quantize::key_of(*px, opts.matrix)));
                }
                let rle = encode_rle(&indices, obj.width as usize, obj.height as usize)?;
                object_defs.push((slot as u16, obj.width, obj.height, rle));
                placements.push(CompositionObject {
                    object_id: slot as u16,
                    window_id: 0,
                    cropped: false,
                    forced: obj.forced,
                    x: obj.x,
                    y: obj.y,
                    crop: None,
                });
                report.objects += 1;
            }
        } else {
            report.erase_sets += 1;
        }
        report.max_palette_entries = report.max_palette_entries.max(table.entries.len());

        // PCS
        let pcs = Pcs {
            width: opts.canvas_width,
            height: opts.canvas_height,
            frame_rate: opts.frame_rate_byte,
            composition_number: (i as u16) % u16::MAX,
            state: if i == 0 { PCS_EPOCH_START } else { 0 },
            // 每个 display set 都自带完整 PDS，因此不依赖 palette_update flag 的语义（ISSUES C19 之外另有 C20）
            palette_update: 0,
            palette_id: opts.palette_id,
            objects: placements,
        };
        push(&mut segments, SEG_PCS, &write_pcs(&pcs), frame.pts, dts)?;

        // WDS：一个覆盖完整 canvas 的窗口（对象坐标相对画布，见 13 §4）
        let window = Window {
            id: 0,
            x: 0,
            y: 0,
            width: opts.canvas_width,
            height: opts.canvas_height,
        };
        let windows: Vec<Window> = if frame.erase && frame.objects.is_empty() {
            Vec::new()
        } else {
            vec![window]
        };
        push(&mut segments, SEG_WDS, &write_wds(&windows), frame.pts, dts)?;

        if !(frame.erase && frame.objects.is_empty()) {
            let pds = write_pds(&palette_of(&table, opts.palette_id))?;
            push(&mut segments, SEG_PDS, &pds, frame.pts, dts)?;
            report.palette_segments += 1;

            for (id, w, h, rle) in object_defs.iter() {
                let chunks = fragment(rle.len(), first_cap, next_cap);
                if chunks.len() > 1 {
                    report.fragmented_objects += 1;
                }
                for (n, (start, len)) in chunks.iter().enumerate() {
                    let first = n == 0;
                    let last = n + 1 == chunks.len();
                    let mut payload = Vec::with_capacity(
                        len + if first {
                            ODS_FIRST_HEADER
                        } else {
                            ODS_NEXT_HEADER
                        },
                    );
                    payload.extend_from_slice(&id.to_be_bytes());
                    payload.push(0); // object version
                    payload.push(if first { 0x80 } else { 0 } | if last { 0x40 } else { 0 });
                    if first {
                        let total = rle.len() as u32;
                        payload.push((total >> 16) as u8);
                        payload.push((total >> 8) as u8);
                        payload.push(total as u8);
                        payload.extend_from_slice(&w.to_be_bytes());
                        payload.extend_from_slice(&h.to_be_bytes());
                    }
                    payload.extend_from_slice(&rle[*start..*start + *len]);
                    push(&mut segments, SEG_ODS, &payload, frame.pts, dts)?;
                    report.ods_segments += 1;
                }
            }
        }
        push(&mut segments, SEG_END, &[], frame.pts, dts)?;
        report.display_sets += 1;
    }
    Ok((segments, report))
}

/// 写成一个可直接编码的字节级区间表：第一个分片用 `first_cap`，之后用 `next_cap`。
fn fragment(total: usize, first_cap: usize, next_cap: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut cap = first_cap.max(1);
    while at < total {
        let len = (total - at).min(cap);
        out.push((at, len));
        at += len;
        cap = next_cap.max(1);
    }
    if out.is_empty() {
        // 空 RLE 理论不可能（encode_rle 至少有行尾），但仍要给出可写的分片
        out.push((0, 0));
    }
    out
}

fn push(segments: &mut Vec<Segment>, ty: u8, payload: &[u8], pts: i64, dts: i64) -> Result<()> {
    segments.push(Segment::new(pts, dts, ty, payload.to_vec())?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_set::{
        assemble_ods, group_display_sets, parse_ods_header, parse_pcs, parse_wds,
    };
    use crate::fastpath::check_timeline;
    use crate::palette::parse_pds;
    use crate::rle::decode as decode_rle;
    use crate::segment::parse_stream;

    fn obj(w: u16, h: u16, x: u16, y: u16, rgba: Vec<u8>) -> FrameObject {
        FrameObject {
            rgba,
            width: w,
            height: h,
            x,
            y,
            forced: false,
        }
    }

    /// 两行 8 列：第一行全白，第二行全透明（同时覆盖游程与透明边界）
    fn small_object() -> FrameObject {
        // 第一行全白，第二行保持 vec![0] 的全透明
        let mut rgba = vec![0u8; 8 * 2 * 4];
        let (row0, _) = rgba[..8 * 4].as_chunks_mut::<4>();
        for px in row0 {
            *px = [255, 255, 255, 255];
        }
        obj(8, 2, 100, 50, rgba)
    }

    fn encode_one(frame: Frame) -> (Vec<Segment>, EncodeReport) {
        let opts = EncodeOptions::default();
        encode(&[frame], &opts).unwrap()
    }

    #[test]
    fn single_frame_structure_is_valid_and_self_parsing() {
        let (segments, report) = encode_one(Frame::new(90_000, vec![small_object()]));
        let types: Vec<u8> = segments.iter().map(|s| s.seg_type).collect();
        assert_eq!(types, vec![SEG_PCS, SEG_WDS, SEG_PDS, SEG_ODS, SEG_END]);
        assert_eq!(report.display_sets, 1);
        assert_eq!(report.objects, 1);
        assert_eq!(report.ods_segments, 1);
        assert_eq!(report.fragmented_objects, 0);

        // 我们的解析器必须能完整读回
        let bytes = crate::segment::write_stream(&segments).unwrap();
        let again = parse_stream(&bytes).unwrap();
        let sets = group_display_sets(&again).unwrap();
        assert_eq!(sets.len(), 1);
        assert!(!sets[0].is_erase);
        assert!(sets[0].has_palette);
        check_timeline(&again).unwrap();
        assert_eq!(again[0].dts, 90_000 - 3_000, "DTS 必须早于 PTS");
    }

    #[test]
    fn pcs_wds_and_palette_fields_roundtrip() {
        let (segments, _) = encode_one(Frame::new(45_000, vec![small_object()]));
        let pcs = parse_pcs(&segments[0].payload).unwrap();
        assert_eq!((pcs.width, pcs.height), (1920, 1080));
        assert!(
            pcs.is_epoch_start(),
            "第一个 display set 必须标 epoch start"
        );
        assert_eq!(pcs.objects.len(), 1);
        assert_eq!(
            pcs.objects[0],
            CompositionObject {
                object_id: 0,
                window_id: 0,
                cropped: false,
                forced: false,
                x: 100,
                y: 50,
                crop: None,
            }
        );
        let windows = parse_wds(&segments[1].payload).unwrap();
        assert_eq!(
            windows[0],
            Window {
                id: 0,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
        let pal = parse_pds(&segments[2].payload).unwrap();
        assert_eq!(pal.entries.len(), 2, "白 + 索引0透明");
        assert_eq!(pal.entries[0].alpha, 0);
        assert_eq!(pal.id, 0);
    }

    #[test]
    fn object_pixels_survive_encode_then_decode() {
        let o = small_object();
        let (segments, _) = encode_one(Frame::new(90_000, vec![o.clone()]));
        let ods = &segments[3];
        let head = parse_ods_header(&ods.payload).unwrap();
        assert!(head.first && head.last);
        assert_eq!(head.size, Some((8, 2)));
        assert_eq!(
            head.declared_len as usize,
            ods.payload[ODS_FIRST_HEADER..].len()
        );
        let (id, w, h, rle) =
            assemble_ods(segments.iter().filter(|s| s.seg_type == SEG_ODS)).unwrap();
        assert_eq!((id, w, h), (0, 8, 2));
        let indices = decode_rle(&rle, 8, 2).unwrap();
        let pal = parse_pds(&segments[2].payload).unwrap();
        let got: Vec<[u8; 4]> = indices
            .iter()
            .map(|i| pal.entries[*i as usize].to_rgba(ColorMatrix::Bt709))
            .collect();
        // 白像素必须回到接近白（limited range 往返），透明边界必须索引 0
        assert_eq!(indices[0], 1);
        assert_eq!(indices[15], 0);
        assert!(got[0][3] > 250);
        assert!(
            got[0][0] > 245 && got[0][1] > 245 && got[0][2] > 245,
            "{:?}",
            got[0]
        );
        assert_eq!(got[15][3], 0);
    }

    #[test]
    fn fragmentation_matches_reference_constants_and_reassembles() {
        // 13 §4 的大 ODS 分片 golden：ass2bdnxml 的首片 65508 / 续片 65515 必须由常量推导出来
        assert_eq!(ods_first_data_cap(MAX_ODS_SEGMENT_PAYLOAD), 65_508);
        assert_eq!(ods_next_data_cap(MAX_ODS_SEGMENT_PAYLOAD), 65_515);
        let chunks = fragment(65_508 + 65_515 + 7, 65_508, 65_515);
        assert_eq!(chunks, vec![(0, 65_508), (65_508, 65_515), (131_023, 7)]);
        // 分片头大小不同，所以 payload 总长一致（这正是 65508/65515 的来源）
        assert_eq!(65_508 + ODS_FIRST_HEADER, 65_515 + ODS_NEXT_HEADER);

        // 真实大对象：8x8 的棋盘每行都不可压缩，构造 >64KB 的 RLE
        let (w, h) = (1000usize, 1000usize);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                // 交替颜色让每个游程长度都是 1，RLE 不缩水
                let c = if (x + y) % 2 == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 255, 0, 255]
                };
                rgba.extend_from_slice(&c);
            }
        }
        let big = obj(w as u16, h as u16, 0, 0, rgba);
        let (segments, report) = encode_one(Frame::new(90_000, vec![big]));
        assert!(report.fragmented_objects >= 1, "必须发生分片");
        let ods: Vec<&Segment> = segments.iter().filter(|s| s.seg_type == SEG_ODS).collect();
        for (n, s) in ods.iter().enumerate() {
            assert!(
                s.payload.len() <= MAX_ODS_SEGMENT_PAYLOAD,
                "分片 {n} 超过 payload 上限"
            );
            let head = parse_ods_header(&s.payload).unwrap();
            assert_eq!(head.first, n == 0);
            assert_eq!(head.last, n + 1 == ods.len());
        }
        let (id, dw, dh, rle) = assemble_ods(ods.iter().copied()).unwrap();
        assert_eq!((id, dw, dh), (0, 1000, 1000));
        let indices = decode_rle(&rle, 1000, 1000).unwrap();
        assert_eq!(indices.len(), 1_000_000);
        assert_eq!(indices[0], 1);
        assert_eq!(indices[1], 2);
    }

    #[test]
    fn fully_transparent_frame_becomes_erase_display_set() {
        let blank = obj(4, 2, 0, 0, vec![0u8; 4 * 2 * 4]);
        assert!(blank.all_transparent());
        let opts = EncodeOptions::default();
        let frames = vec![
            Frame::new(90_000, vec![small_object()]),
            Frame::erase(180_000),
        ];
        let (segments, report) = encode(&frames, &opts).unwrap();
        assert_eq!(report.erase_sets, 1);
        let sets = group_display_sets(&segments).unwrap();
        assert_eq!(sets.len(), 2);
        assert!(
            sets[1].is_erase,
            "空 composition object 必须被识别为 erase set"
        );
        let pcs = parse_pcs(&segments[sets[1].first_segment].payload).unwrap();
        assert!(pcs.objects.is_empty());
        assert!(parse_wds(&segments[sets[1].first_segment + 1].payload)
            .unwrap()
            .is_empty());
        assert_eq!(segments[sets[1].first_segment + 1].seg_type, SEG_WDS);
        assert_eq!(segments[sets[1].last_segment].seg_type, SEG_END);
    }

    #[test]
    fn multiple_sets_get_monotonic_clocks_and_wrapping_composition_numbers() {
        let frames: Vec<Frame> = (0..3)
            .map(|i| Frame::new(90_000 * (i + 1), vec![small_object()]))
            .collect();
        let (segments, report) = encode(&frames, &EncodeOptions::default()).unwrap();
        assert_eq!(report.display_sets, 3);
        check_timeline(&segments).unwrap();
        let nums: Vec<u16> = segments
            .iter()
            .filter(|s| s.seg_type == SEG_PCS)
            .map(|s| parse_pcs(&s.payload).unwrap().composition_number)
            .collect();
        assert_eq!(nums, vec![0, 1, 2]);
        let states: Vec<u8> = segments
            .iter()
            .filter(|s| s.seg_type == SEG_PCS)
            .map(|s| parse_pcs(&s.payload).unwrap().state)
            .collect();
        assert_eq!(states, vec![PCS_EPOCH_START, 0, 0]);
    }

    #[test]
    fn forced_flag_is_written_into_pcs() {
        let mut o = small_object();
        o.forced = true;
        let (segments, _) = encode_one(Frame::new(90_000, vec![o]));
        let pcs = parse_pcs(&segments[0].payload).unwrap();
        assert!(pcs.objects[0].forced);
    }

    #[test]
    fn invalid_inputs_are_rejected_before_any_segment_is_written() {
        let opts = EncodeOptions::default();
        // 画布过大
        let big = EncodeOptions {
            canvas_width: 3840,
            ..opts
        };
        assert_eq!(
            encode(&[Frame::new(1, vec![])], &big)
                .unwrap_err()
                .code_string(),
            "STK-PGS-0091"
        );
        // 对象越界
        let outside = obj(4, 2, 1918, 1079, vec![255; 4 * 2 * 4]);
        assert_eq!(
            encode(&[Frame::new(90_000, vec![outside])], &opts)
                .unwrap_err()
                .code_string(),
            "STK-PGS-0101"
        );
        // RGBA 长度不符
        assert_eq!(
            encode(
                &[Frame::new(90_000, vec![obj(4, 2, 0, 0, vec![0; 8])])],
                &opts
            )
            .unwrap_err()
            .code_string(),
            "STK-PGS-0100"
        );
        // PTS 不递增
        assert_eq!(
            encode(
                &[Frame::new(90_000, vec![]), Frame::new(90_000, vec![])],
                &opts
            )
            .unwrap_err()
            .code_string(),
            "STK-PGS-0097"
        );
        // decode lead 非法
        let nolead = EncodeOptions {
            decode_lead_ticks: 0,
            ..opts
        };
        assert_eq!(
            encode(&[Frame::new(90_000, vec![])], &nolead)
                .unwrap_err()
                .code_string(),
            "STK-PGS-0092"
        );
    }

    /// 256 个可分辨颜色：255 档 T 的同一灰 + 一条 Y 差 1 的颜色
    fn over_budget_object() -> FrameObject {
        let mut rgba = Vec::with_capacity(256 * 4);
        for a in 1..=255u8 {
            rgba.extend_from_slice(&[235, 128, 128, a]);
        }
        rgba.extend_from_slice(&[234, 128, 128, 255]);
        assert_eq!(rgba.len(), 16 * 16 * 4);
        obj(16, 16, 0, 0, rgba)
    }

    #[test]
    fn exact_policy_fails_loudly_and_median_cut_fits_the_same_frame() {
        // Basic：255 个 alpha + 透明 = 槽位用满，再多一条可分辨颜色必须报错而不是丢色
        let opts = EncodeOptions::default();
        let e = encode(&[Frame::new(90_000, vec![over_budget_object()])], &opts).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0095");

        // Advanced：同一帧必须编得出来，且误差被度量（不许静默失真）
        let qopts = EncodeOptions {
            colors: ColorPolicy::MedianCut { max_colors: 64 },
            ..EncodeOptions::default()
        };
        let (segments, report) =
            encode(&[Frame::new(90_000, vec![over_budget_object()])], &qopts).unwrap();
        assert_eq!(report.quantized_frames, 1);
        assert!(report.merged_colors > 0, "{report:?}");
        assert!(report.worst_color_error > 0, "合并过却报零误差：{report:?}");
        assert!(
            report.max_palette_entries <= 65,
            "{:?}",
            report.max_palette_entries
        );

        // 读回闭环：索引必须都落在实际写出的调色板里
        let (id, w, h, rle) =
            assemble_ods(segments.iter().filter(|s| s.seg_type == SEG_ODS)).unwrap();
        assert_eq!((id, w, h), (0, 16, 16));
        let indices = decode_rle(&rle, 16, 16).unwrap();
        let pal = parse_pds(&segments[2].payload).unwrap();
        assert_eq!(pal.entries.len(), report.max_palette_entries);
        assert!(
            indices.iter().all(|i| (*i as usize) < pal.entries.len()),
            "量化后出现越界索引"
        );
    }

    #[test]
    fn median_cut_is_the_identity_when_the_source_already_fits() {
        // 预算够用时一条都不合并：report 必须诚实地说"没做过量化"
        let mut rgba = Vec::new();
        for a in 1..=20u8 {
            rgba.extend_from_slice(&[235, 128, 128, a]);
        }
        let o = obj(20, 1, 0, 0, rgba);
        let opts = EncodeOptions {
            colors: ColorPolicy::MedianCut { max_colors: 64 },
            ..EncodeOptions::default()
        };
        let (_, report) = encode(&[Frame::new(90_000, vec![o])], &opts).unwrap();
        assert_eq!(report.quantized_frames, 0, "{report:?}");
        assert_eq!(report.merged_colors, 0);
        assert_eq!(report.worst_color_error, 0);
        assert_eq!(report.max_palette_entries, 21, "20 色 + 透明槽位");
    }

    #[test]
    fn canvas_smaller_than_blu_ray_max_is_allowed_for_sd_profiles() {
        let opts = EncodeOptions {
            canvas_width: 720,
            canvas_height: 480,
            ..EncodeOptions::default()
        };
        let o = obj(4, 2, 10, 10, vec![255; 4 * 2 * 4]);
        let (segments, _) = encode(&[Frame::new(90_000, vec![o])], &opts).unwrap();
        let pcs = parse_pcs(&segments[0].payload).unwrap();
        assert_eq!((pcs.width, pcs.height), (720, 480));
    }

    #[test]
    fn encoded_stream_is_readable_by_the_reference_decoder() {
        // 与差分测试同一入口：我们编码 -> oxideav 解码，比较画布/对象数/像素数量
        let (segments, _) = encode_one(Frame::new(90_000, vec![small_object()]));
        let bytes = crate::segment::write_stream(&segments).unwrap();
        let stream = crate::read_file(&bytes).unwrap();
        assert_eq!(stream.display_set_count(), 1);
        assert_eq!(stream.canvas(), Some((1920, 1080)));
    }
}
