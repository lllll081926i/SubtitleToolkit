//! PGS/SUP 编解码（13、08 §1 PGS IR）。当前实现覆盖 Level 0-3：
//! 段级读写、有状态 display set 解析、Fast Path 无损变换、Basic Encoder（`encoder`）。
//! `quantize` 是 Level 4 Advanced Author 的第一项（13 §5 image quantization）；
//! Strict Validator 的其余部分在 `pgs-author` / `validator` 里继续。

pub mod display_set;
pub mod encoder;
pub mod fastpath;
pub mod palette;
pub mod quantize;
pub mod rle;
pub mod segment;

pub use display_set::{
    group_display_sets, parse_ods_header, parse_pcs, parse_wds, unresolved_objects, write_pcs,
    write_wds, CompositionObject, DisplaySet, Pcs, Window,
};
pub use fastpath::{apply, check_timeline, ods_digests, ods_invariant, FastOp, OdsInvariant};
pub use palette::{luma_scalar, parse_pds, write_pds, ColorMatrix, Palette, PaletteEntry};
pub use quantize::{
    build_table, histogram, key_of, ColorPolicy, ColorTable, QuantStats, COLOR_BUDGET,
    TRANSPARENT_INDEX,
};
pub use rle::{decode as decode_rle, encode as encode_rle};
pub use segment::{
    parse_stream, unknown_types, write_stream, Segment, CLOCK_FIELD_MAX, HEADER_LEN, MAX_SEGMENTS,
    SEG_END, SEG_IDS, SEG_ODS, SEG_PADDING, SEG_PCS, SEG_PDS, SEG_SEQ, SEG_WDS,
};

use core_model::error::{Domain, Result, StkError};

/// 一次性读取整个 SUP：段列表 + display set 分组 + 结构检查。
pub fn read_file(bytes: &[u8]) -> Result<SupStream> {
    let segments = segment::parse_stream(bytes)?;
    let sets = group_display_sets(&segments)?;
    check_timeline(&segments)?;
    let unresolved = unresolved_objects(&segments, &sets);
    if !unresolved.is_empty() {
        return Err(
            StkError::simple(Domain::Validate, 1, "存在未定义的 composition object 引用")
                .with_detail(format!("{unresolved:?}"))
                .with_action("该文件不能被判定为结构合法；需要重新 author"),
        );
    }
    let unknown = unknown_types(&segments);
    Ok(SupStream {
        segments,
        sets,
        unknown,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupStream {
    pub segments: Vec<Segment>,
    pub sets: Vec<DisplaySet>,
    /// 保留但可识别的扩展段类型（13 §2 preserve policy）
    pub unknown: Vec<u8>,
}

impl SupStream {
    pub fn write(&self) -> Result<Vec<u8>> {
        write_stream(&self.segments)
    }
    pub fn display_set_count(&self) -> usize {
        self.sets.len()
    }
    pub fn canvas(&self) -> Option<(u16, u16)> {
        self.segments
            .iter()
            .find(|s| s.seg_type == SEG_PCS)
            .and_then(|s| parse_pcs(&s.payload).ok())
            .map(|p| (p.width, p.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_set::{Window, PCS_EPOCH_START};

    fn synth() -> Vec<u8> {
        let pcs = write_pcs(&Pcs {
            width: 1920,
            height: 1080,
            frame_rate: 0x10,
            composition_number: 0,
            state: PCS_EPOCH_START,
            palette_update: 0,
            palette_id: 0,
            objects: vec![CompositionObject {
                object_id: 1,
                window_id: 0,
                cropped: false,
                forced: false,
                x: 120,
                y: 900,
                crop: None,
            }],
        })
        .unwrap();
        let wds = write_wds(&[Window {
            id: 0,
            x: 120,
            y: 900,
            width: 400,
            height: 70,
        }])
        .unwrap();
        let pal = write_pds(&Palette {
            id: 0,
            version: 0,
            entries: vec![
                PaletteEntry::transparent(0),
                PaletteEntry {
                    index: 1,
                    y: 235,
                    cr: 128,
                    cb: 128,
                    alpha: 255,
                },
            ],
        })
        .unwrap();
        let indices: Vec<u8> = (0..400 * 70)
            .map(|i| if i % 5 == 0 { 1 } else { 0 })
            .collect();
        let rle = encode_rle(&indices, 400, 70).unwrap();
        let mut ods = vec![0x00, 0x01, 0x00, 0xC0];
        let dl = (rle.len() + 4) as u32;
        ods.push(((dl >> 16) & 0xff) as u8);
        ods.push(((dl >> 8) & 0xff) as u8);
        ods.push((dl & 0xff) as u8);
        ods.extend_from_slice(&400u16.to_be_bytes());
        ods.extend_from_slice(&70u16.to_be_bytes());
        ods.extend_from_slice(&rle);
        let segs = vec![
            Segment::new(90_000, 90_000, SEG_PCS, pcs).unwrap(),
            Segment::new(90_000, 90_000, SEG_WDS, wds).unwrap(),
            Segment::new(90_000, 90_000, SEG_PDS, pal).unwrap(),
            Segment::new(90_000, 90_000, SEG_ODS, ods).unwrap(),
            Segment::new(90_000, 90_000, SEG_END, vec![]).unwrap(),
        ];
        write_stream(&segs).unwrap()
    }

    #[test]
    fn read_then_write_is_byte_identical() {
        let bytes = synth();
        let s = read_file(&bytes).unwrap();
        assert_eq!(s.write().unwrap(), bytes);
        assert_eq!(s.display_set_count(), 1);
        assert_eq!(s.canvas(), Some((1920, 1080)));
        assert!(s.unknown.is_empty());
    }

    #[test]
    fn object_pixels_reach_original_index_map() {
        let s = read_file(&synth()).unwrap();
        let ods: Vec<&Segment> = s
            .segments
            .iter()
            .filter(|x| x.seg_type == SEG_ODS)
            .collect();
        let (id, w, h, stream) = group_display_sets(&s.segments)
            .map(|_| ())
            .and_then(|()| crate::display_set::assemble_ods(ods.iter().copied()))
            .unwrap();
        let idx = decode_rle(&stream, w as usize, h as usize).unwrap();
        assert_eq!(id, 1);
        assert_eq!(idx.len(), w as usize * h as usize);
        assert_eq!(idx[0], 1, "i=0 满足 i%5==0 → 索引 1");
        assert_eq!(idx[1], 0);
        assert_eq!(idx[5], 1);
    }

    #[test]
    fn fast_path_through_file_level_api_keeps_ods() {
        let bytes = synth();
        let before = read_file(&bytes).unwrap();
        let moved = apply(
            &before.segments,
            &[FastOp::MoveBy(10, -5), FastOp::OffsetMs(500)],
        )
        .unwrap();
        assert!(ods_invariant(&before.segments, &moved).unwrap().equal);
        let out = SupStream {
            segments: moved,
            sets: before.sets.clone(),
            unknown: Vec::new(),
        };
        let rebytes = out.write().unwrap();
        let after = read_file(&rebytes).unwrap();
        assert_eq!(after.segments[0].pts - before.segments[0].pts, 45_000);
        assert_eq!(after.canvas(), Some((1920, 1080)));
    }

    #[test]
    fn dangling_reference_is_rejected_at_read() {
        let bytes = synth();
        let mut s = read_file(&bytes).unwrap();
        // 把 PCS 里的 object id 改成不存在的 9
        let mut pcs = s.segments[0].clone();
        pcs.payload[11] = 0;
        pcs.payload[12] = 9;
        s.segments[0] = pcs;
        let broken = write_stream(&s.segments).unwrap();
        assert_eq!(
            read_file(&broken).unwrap_err().code_string(),
            "STK-VAL-0001"
        );
    }
}
