//! Display set 结构与有状态解码（13 §2）。
//! PCS = 11 字节头 + 每对象 8 字节（裁剪对象再加 8 字节）；WDS = 1 + 每窗口 9 字节。

use core_model::error::{Domain, Result, StkError};

use crate::segment::{Segment, SEG_END, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositionObject {
    pub object_id: u16,
    pub window_id: u8,
    pub cropped: bool,
    pub forced: bool,
    pub x: u16,
    pub y: u16,
    /// cropped 时的对象尺寸与裁剪偏移
    pub crop: Option<(u16, u16, u16, u16)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcs {
    pub width: u16,
    pub height: u16,
    pub frame_rate: u8,
    pub composition_number: u16,
    pub state: u8,
    pub palette_update: u8,
    pub palette_id: u8,
    pub objects: Vec<CompositionObject>,
}

pub const PCS_EPOCH_START: u8 = 0x80;
pub const PCS_ACQUISITION_POINT: u8 = 0x40;

impl Pcs {
    pub fn is_epoch_start(&self) -> bool {
        self.state & PCS_EPOCH_START != 0
    }
    pub fn has_acquisition_point(&self) -> bool {
        self.state & PCS_ACQUISITION_POINT != 0
    }

    pub fn payload_len(&self) -> usize {
        11 + self
            .objects
            .iter()
            .map(|o| if o.cropped { 16 } else { 8 })
            .sum::<usize>()
    }
}

pub fn parse_pcs(payload: &[u8]) -> Result<Pcs> {
    if payload.len() < 11 {
        return Err(StkError::simple(Domain::Pgs, 50, "PCS 少于 11 字节头")
            .with_detail(format!("{}", payload.len())));
    }
    let mut cur = 11usize;
    let n = payload[10] as usize;
    let mut objects = Vec::with_capacity(n);
    for _ in 0..n {
        if payload.len() < cur + 8 {
            return Err(StkError::simple(
                Domain::Pgs,
                51,
                "PCS composition object 被截断",
            ));
        }
        let object_id = u16::from_be_bytes([payload[cur], payload[cur + 1]]);
        let window_id = payload[cur + 2];
        let flags = payload[cur + 3];
        let cropped = flags & 0x80 != 0;
        let forced = flags & 0x40 != 0;
        let x = u16::from_be_bytes([payload[cur + 4], payload[cur + 5]]);
        let y = u16::from_be_bytes([payload[cur + 6], payload[cur + 7]]);
        cur += 8;
        let crop = if cropped {
            if payload.len() < cur + 8 {
                return Err(StkError::simple(Domain::Pgs, 52, "裁剪对象缺少 crop 字段"));
            }
            let c = (
                u16::from_be_bytes([payload[cur], payload[cur + 1]]),
                u16::from_be_bytes([payload[cur + 2], payload[cur + 3]]),
                u16::from_be_bytes([payload[cur + 4], payload[cur + 5]]),
                u16::from_be_bytes([payload[cur + 6], payload[cur + 7]]),
            );
            cur += 8;
            Some(c)
        } else {
            None
        };
        objects.push(CompositionObject {
            object_id,
            window_id,
            cropped,
            forced,
            x,
            y,
            crop,
        });
    }
    if cur != payload.len() {
        return Err(
            StkError::simple(Domain::Pgs, 53, "PCS payload 长度与对象数量不符")
                .with_detail(format!("consumed {cur}, payload {}", payload.len())),
        );
    }
    Ok(Pcs {
        width: u16::from_be_bytes([payload[0], payload[1]]),
        height: u16::from_be_bytes([payload[2], payload[3]]),
        frame_rate: payload[4],
        composition_number: u16::from_be_bytes([payload[5], payload[6]]),
        state: payload[7],
        palette_update: payload[8],
        palette_id: payload[9],
        objects,
    })
}

pub fn write_pcs(p: &Pcs) -> Result<Vec<u8>> {
    if p.objects.len() > u8::MAX as usize {
        return Err(
            StkError::simple(Domain::Pgs, 71, "PCS 对象数量超过 8bit 字段上限")
                .with_detail(format!("{} 个对象，最多 255 个", p.objects.len())),
        );
    }
    let mut v = Vec::with_capacity(p.payload_len());
    v.extend_from_slice(&p.width.to_be_bytes());
    v.extend_from_slice(&p.height.to_be_bytes());
    v.push(p.frame_rate);
    v.extend_from_slice(&p.composition_number.to_be_bytes());
    v.push(p.state);
    v.push(p.palette_update);
    v.push(p.palette_id);
    v.push(p.objects.len() as u8);
    for o in &p.objects {
        v.extend_from_slice(&o.object_id.to_be_bytes());
        v.push(o.window_id);
        v.push(if o.cropped { 0x80 } else { 0 } | if o.forced { 0x40 } else { 0 });
        v.extend_from_slice(&o.x.to_be_bytes());
        v.extend_from_slice(&o.y.to_be_bytes());
        if let Some((cw, ch, cx, cy)) = o.crop {
            v.extend_from_slice(&cw.to_be_bytes());
            v.extend_from_slice(&ch.to_be_bytes());
            v.extend_from_slice(&cx.to_be_bytes());
            v.extend_from_slice(&cy.to_be_bytes());
        }
    }
    Ok(v)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

pub fn parse_wds(payload: &[u8]) -> Result<Vec<Window>> {
    if payload.is_empty() {
        return Err(StkError::simple(Domain::Pgs, 54, "WDS 为空"));
    }
    let n = payload[0] as usize;
    let body = &payload[1..];
    if body.len() != n * 9 {
        return Err(StkError::simple(Domain::Pgs, 55, "WDS 长度与窗口数量不符")
            .with_detail(format!("declared {n} windows, body {} bytes", body.len())));
    }
    let mut out = Vec::with_capacity(n);
    let mut off = 0usize;
    while off + 9 <= body.len() {
        let c = &body[off..off + 9];
        off += 9;
        out.push(Window {
            id: c[0],
            x: u16::from_be_bytes([c[1], c[2]]),
            y: u16::from_be_bytes([c[3], c[4]]),
            width: u16::from_be_bytes([c[5], c[6]]),
            height: u16::from_be_bytes([c[7], c[8]]),
        });
    }
    Ok(out)
}

pub fn write_wds(w: &[Window]) -> Result<Vec<u8>> {
    if w.len() > u8::MAX as usize {
        return Err(
            StkError::simple(Domain::Pgs, 72, "WDS 窗口数量超过 8bit 字段上限")
                .with_detail(format!("{} 个窗口，最多 255 个", w.len())),
        );
    }
    let mut v = vec![w.len() as u8];
    for x in w {
        v.push(x.id);
        v.extend_from_slice(&x.x.to_be_bytes());
        v.extend_from_slice(&x.y.to_be_bytes());
        v.extend_from_slice(&x.width.to_be_bytes());
        v.extend_from_slice(&x.height.to_be_bytes());
    }
    Ok(v)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OdsHeader {
    pub object_id: u16,
    pub version: u8,
    pub first: bool,
    pub last: bool,
    /// 3 字节字段：首片覆盖 `width+height+rle`，后续分片只有 rle 续块
    pub declared_len: u32,
    pub size: Option<(u16, u16)>,
    pub rle_start: usize,
}

pub fn parse_ods_header(payload: &[u8]) -> Result<OdsHeader> {
    if payload.len() < 4 {
        return Err(StkError::simple(Domain::Pgs, 56, "ODS 头少于 4 字节"));
    }
    let object_id = u16::from_be_bytes([payload[0], payload[1]]);
    let version = payload[2];
    let seq = payload[3];
    let first = seq & 0x80 != 0;
    let last = seq & 0x40 != 0;
    // 续片没有 data_length/尺寸字段：头只有 object_id(2)+version(1)+sequence_flag(1)
    let (declared_len, size, rle_start) = if first {
        if payload.len() < 11 {
            return Err(StkError::simple(
                Domain::Pgs,
                57,
                "ODS 首片长度不足 11 字节",
            ));
        }
        let len = ((payload[4] as u32) << 16) | ((payload[5] as u32) << 8) | payload[6] as u32;
        let s = (
            u16::from_be_bytes([payload[7], payload[8]]),
            u16::from_be_bytes([payload[9], payload[10]]),
        );
        (len, Some(s), 11)
    } else {
        (0, None, 4)
    };
    Ok(OdsHeader {
        object_id,
        version,
        first,
        last,
        declared_len,
        size,
        rle_start,
    })
}

/// 把分片重组为完整 RLE 流；要求首片存在且 `last` 之后 id 连续。
pub fn assemble_ods<'a>(
    segments: impl Iterator<Item = &'a Segment>,
) -> Result<(u16, u16, u16, Vec<u8>)> {
    let mut rle = Vec::new();
    let mut id = None;
    let mut dims = None;
    let mut expect_first = true;
    let mut done = false;
    for s in segments {
        let h = parse_ods_header(&s.payload)?;
        if id.is_some() && id != Some(h.object_id) {
            return Err(StkError::simple(
                Domain::Pgs,
                58,
                "ODS 分片序列中的 object id 不一致",
            ));
        }
        id = Some(h.object_id);
        if done {
            return Err(StkError::simple(Domain::Pgs, 59, "ODS 序列结束后仍有分片"));
        }
        if h.first != expect_first {
            return Err(StkError::simple(Domain::Pgs, 60, "ODS 分片标志顺序非法"));
        }
        expect_first = false;
        if let Some((w, hh)) = h.size {
            dims = Some((w, hh));
        }
        rle.extend_from_slice(&s.payload[h.rle_start..]);
        if h.last {
            done = true;
        }
    }
    let (id, (w, h)) = match (id, dims) {
        (Some(i), Some(d)) => (i, d),
        _ => {
            return Err(StkError::simple(
                Domain::Pgs,
                61,
                "ODS 缺少 object id 或尺寸",
            ))
        }
    };
    if !done {
        return Err(StkError::simple(Domain::Pgs, 62, "ODS 序列没有 last 分片"));
    }
    Ok((id, w, h, rle))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplaySet {
    pub index: usize,
    pub first_segment: usize,
    pub last_segment: usize,
    pub pts: i64,
    pub dts: i64,
    pub pcs: Option<Pcs>,
    pub windows: Vec<Window>,
    pub has_palette: bool,
    pub object_ids: Vec<u16>,
    pub is_erase: bool,
}

/// 分组：PCS 开启一个 display set，END 收尾；无 PCS 的段归入当前 set（palette-only 更新合法）。
pub fn group_display_sets(segments: &[Segment]) -> Result<Vec<DisplaySet>> {
    let mut sets: Vec<DisplaySet> = Vec::new();
    let mut cur: Option<DisplaySet> = None;
    for (i, s) in segments.iter().enumerate() {
        match s.seg_type {
            SEG_PCS => {
                if let Some(prev) = cur.take() {
                    if prev.last_segment == usize::MAX {
                        return Err(StkError::simple(Domain::Pgs, 63, "PCS 之前缺少 END 段")
                            .with_detail(format!("set {}", prev.index)));
                    }
                    sets.push(prev);
                }
                cur = Some(DisplaySet {
                    index: sets.len(),
                    first_segment: i,
                    last_segment: usize::MAX,
                    pts: s.pts,
                    dts: s.dts,
                    pcs: Some(parse_pcs(&s.payload)?),
                    windows: Vec::new(),
                    has_palette: false,
                    object_ids: Vec::new(),
                    is_erase: false,
                });
            }
            SEG_WDS => {
                let w = parse_wds(&s.payload)?;
                let c = cur
                    .as_mut()
                    .ok_or_else(|| StkError::simple(Domain::Pgs, 64, "WDS 出现在任何 PCS 之前"))?;
                c.windows.extend(w);
            }
            SEG_PDS => {
                let c = cur
                    .as_mut()
                    .ok_or_else(|| StkError::simple(Domain::Pgs, 65, "PDS 出现在任何 PCS 之前"))?;
                if c.has_palette {
                    return Err(StkError::simple(
                        Domain::Pgs,
                        66,
                        "同一 display set 出现两个 PDS",
                    ));
                }
                c.has_palette = true;
                parse_palette_len(&s.payload)?;
            }
            SEG_ODS => {
                let c = cur
                    .as_mut()
                    .ok_or_else(|| StkError::simple(Domain::Pgs, 67, "ODS 出现在任何 PCS 之前"))?;
                let h = parse_ods_header(&s.payload)?;
                if h.first || c.object_ids.is_empty() {
                    c.object_ids.push(h.object_id);
                }
            }
            SEG_END => {
                let mut c = cur
                    .take()
                    .ok_or_else(|| StkError::simple(Domain::Pgs, 68, "END 出现在任何 PCS 之前"))?;
                c.last_segment = i;
                c.is_erase = c
                    .pcs
                    .as_ref()
                    .map(|p| p.objects.is_empty())
                    .unwrap_or(false);
                sets.push(c);
            }
            // SEQ/IDS/PADDING/未知段：附着在当前 set 上，不破坏结构（13 §2 preserve policy）
            _ => {
                if cur.is_none() {
                    return Err(
                        StkError::simple(Domain::Pgs, 69, "扩展段出现在任何 PCS 之前")
                            .with_detail(format!("type 0x{:02X}", s.seg_type)),
                    );
                }
            }
        }
    }
    if let Some(prev) = cur {
        return Err(
            StkError::simple(Domain::Pgs, 70, "文件以未收尾的 display set 结束")
                .with_detail(format!("set {}", prev.index)),
        );
    }
    Ok(sets)
}

fn parse_palette_len(payload: &[u8]) -> Result<usize> {
    let p = crate::palette::parse_pds(payload)?;
    Ok(p.entries.len())
}

/// 每个对象引用的 ODS 是否存在；Strict validator 的 Lifecycle 前置检查。
pub fn unresolved_objects(segments: &[Segment], sets: &[DisplaySet]) -> Vec<u16> {
    let mut out = Vec::new();
    for s in sets {
        let defined: Vec<u16> = segments[s.first_segment..=s.last_segment]
            .iter()
            .filter(|x| x.seg_type == SEG_ODS)
            .filter_map(|x| parse_ods_header(&x.payload).ok().map(|h| h.object_id))
            .collect();
        if let Some(pcs) = &s.pcs {
            for o in &pcs.objects {
                if !defined.contains(&o.object_id) {
                    out.push(o.object_id);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::segment::SEG_END;

    fn pcs_payload(width: u16, height: u16, objects: &[(u16, bool, bool)]) -> Vec<u8> {
        let p = Pcs {
            width,
            height,
            frame_rate: 0x10,
            composition_number: 1,
            state: PCS_EPOCH_START,
            palette_update: 0,
            palette_id: 0,
            objects: objects
                .iter()
                .map(|(id, crop, forced)| CompositionObject {
                    object_id: *id,
                    window_id: 0,
                    cropped: *crop,
                    forced: *forced,
                    x: 100,
                    y: 800,
                    crop: if *crop { Some((200, 60, 0, 0)) } else { None },
                })
                .collect(),
        };
        write_pcs(&p).unwrap()
    }

    #[test]
    fn pcs_roundtrip_with_and_without_crop() {
        for objs in [
            vec![(1u16, false, false)],
            vec![(2, true, false)],
            vec![(3, false, true)],
            vec![(4, true, true)],
        ] {
            let bytes = pcs_payload(1920, 1080, &objs);
            let p = parse_pcs(&bytes).unwrap();
            assert_eq!(p.width, 1920);
            assert_eq!(p.height, 1080);
            assert_eq!(p.objects.len(), 1);
            assert_eq!(p.payload_len(), bytes.len());
            assert_eq!(write_pcs(&p).unwrap(), bytes, "PCS 重写必须逐字节相同");
        }
        let multi = pcs_payload(1920, 1080, &[(1, false, false), (2, false, false)]);
        assert_eq!(multi.len(), 11 + 16);
        assert!(parse_pcs(&multi).unwrap().is_epoch_start());
    }

    #[test]
    fn pcs_rejects_truncation_and_trailing_bytes() {
        assert_eq!(
            parse_pcs(&[0u8; 5]).unwrap_err().code_string(),
            "STK-PGS-0050"
        );
        let mut truncated = pcs_payload(1920, 1080, &[(1, false, false)]);
        truncated.truncate(15);
        assert_eq!(
            parse_pcs(&truncated).unwrap_err().code_string(),
            "STK-PGS-0051"
        );
        let mut cropped_missing = write_pcs(&Pcs {
            width: 1920,
            height: 1080,
            frame_rate: 0x10,
            composition_number: 1,
            state: 0,
            palette_update: 0,
            palette_id: 0,
            objects: vec![CompositionObject {
                object_id: 1,
                window_id: 0,
                cropped: true,
                forced: false,
                x: 0,
                y: 0,
                crop: Some((1, 1, 0, 0)),
            }],
        })
        .unwrap();
        cropped_missing.truncate(19);
        assert_eq!(
            parse_pcs(&cropped_missing).unwrap_err().code_string(),
            "STK-PGS-0052"
        );
        let mut extra = pcs_payload(1920, 1080, &[(1, false, false)]);
        extra.push(0);
        assert_eq!(parse_pcs(&extra).unwrap_err().code_string(), "STK-PGS-0053");
    }

    #[test]
    fn wds_roundtrip() {
        let w = vec![
            Window {
                id: 0,
                x: 100,
                y: 800,
                width: 300,
                height: 80,
            },
            Window {
                id: 1,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ];
        let bytes = write_wds(&w).unwrap();
        assert_eq!(bytes.len(), 1 + 18);
        assert_eq!(parse_wds(&bytes).unwrap(), w);
        assert_eq!(parse_wds(&[]).unwrap_err().code_string(), "STK-PGS-0054");
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(parse_wds(&bad).unwrap_err().code_string(), "STK-PGS-0055");
    }

    #[test]
    fn serializers_reject_counts_that_do_not_fit_their_wire_fields() {
        let object = CompositionObject {
            object_id: 1,
            window_id: 0,
            cropped: false,
            forced: false,
            x: 0,
            y: 0,
            crop: None,
        };
        let pcs = Pcs {
            width: 1920,
            height: 1080,
            frame_rate: 0x10,
            composition_number: 0,
            state: 0,
            palette_update: 0,
            palette_id: 0,
            objects: vec![object; 256],
        };
        assert_eq!(write_pcs(&pcs).unwrap_err().code_string(), "STK-PGS-0071");

        let window = Window {
            id: 0,
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(
            write_wds(&vec![window; 256]).unwrap_err().code_string(),
            "STK-PGS-0072"
        );
    }

    #[test]
    fn ods_fragments_reassemble() {
        // 首片带尺寸，中间片只有 rle 续块，末片 last
        let mut first = vec![
            0x00, 0x07, 0x00, 0x80, 0x00, 0x00, 0x20, 0x00, 0xC8, 0x00, 0x3C,
        ];
        first.extend_from_slice(&[1, 2, 3, 4]);
        // 续片只有 4 字节头（没有 data_length），与首片不同
        let mut mid = vec![0x00, 0x07, 0x00, 0x00];
        mid.extend_from_slice(&[5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        let mut last = vec![0x00, 0x07, 0x00, 0x40];
        last.extend_from_slice(&[17, 18, 19, 20]);
        let segs = [
            Segment::new(1, 1, SEG_ODS, first).unwrap(),
            Segment::new(1, 1, SEG_ODS, mid).unwrap(),
            Segment::new(1, 1, SEG_ODS, last).unwrap(),
        ];
        let (id, w, h, rle) = assemble_ods(segs.iter()).unwrap();
        assert_eq!((id, w, h), (7, 200, 60));
        assert_eq!(rle, (1..=20).collect::<Vec<u8>>());
    }

    #[test]
    fn ods_sequence_errors_are_caught() {
        let bad_first = Segment::new(
            1,
            1,
            SEG_ODS,
            vec![0, 1, 0, 0x40, 0, 0, 4, 0, 2, 0, 2, 9, 9, 9, 9],
        )
        .unwrap();
        assert_eq!(
            assemble_ods([bad_first].iter()).unwrap_err().code_string(),
            "STK-PGS-0060"
        );
        let no_last = Segment::new(
            1,
            1,
            SEG_ODS,
            vec![0, 1, 0, 0x80, 0, 0, 4, 0, 2, 0, 2, 9, 9, 9, 9],
        )
        .unwrap();
        assert_eq!(
            assemble_ods([no_last].iter()).unwrap_err().code_string(),
            "STK-PGS-0062"
        );
        assert!(parse_ods_header(&[0, 1, 0]).is_err());
    }

    #[test]
    fn display_sets_group_and_detect_erase() {
        let segs = vec![
            Segment::new(
                90_000,
                90_000,
                SEG_PCS,
                pcs_payload(1920, 1080, &[(1, false, false)]),
            )
            .unwrap(),
            Segment::new(
                90_000,
                90_000,
                SEG_WDS,
                write_wds(&[Window {
                    id: 0,
                    x: 100,
                    y: 800,
                    width: 200,
                    height: 50,
                }])
                .unwrap(),
            )
            .unwrap(),
            Segment::new(
                90_000,
                90_000,
                SEG_PDS,
                vec![0, 0, 0, 16, 128, 128, 0, 1, 235, 128, 128, 255],
            )
            .unwrap(),
            Segment::new(
                90_000,
                90_000,
                SEG_ODS,
                vec![0, 1, 0, 0xC0, 0, 0, 6, 0, 4, 0, 2, 1, 2, 3, 4],
            )
            .unwrap(),
            Segment::new(90_000, 90_000, SEG_END, vec![]).unwrap(),
            Segment::new(180_000, 180_000, SEG_PCS, pcs_payload(1920, 1080, &[])).unwrap(),
            Segment::new(180_000, 180_000, SEG_END, vec![]).unwrap(),
        ];
        let sets = group_display_sets(&segs).unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].first_segment, 0);
        assert_eq!(sets[0].last_segment, 4);
        assert!(sets[0].has_palette);
        assert_eq!(sets[0].object_ids, vec![1]);
        assert_eq!(sets[0].windows.len(), 1);
        assert!(!sets[0].is_erase);
        assert!(sets[1].is_erase, "零对象 PCS 就是 erase display set");
        assert!(unresolved_objects(&segs, &sets).is_empty());
    }

    #[test]
    fn dangling_objects_and_missing_end_are_reported() {
        let segs = vec![
            Segment::new(1, 1, SEG_PCS, pcs_payload(1920, 1080, &[(9, false, false)])).unwrap(),
            Segment::new(1, 1, SEG_END, vec![]).unwrap(),
        ];
        let sets = group_display_sets(&segs).unwrap();
        assert_eq!(unresolved_objects(&segs, &sets), vec![9]);
        // 没有 END 收尾
        let open = vec![Segment::new(1, 1, SEG_PCS, pcs_payload(1920, 1080, &[])).unwrap()];
        assert_eq!(
            group_display_sets(&open).unwrap_err().code_string(),
            "STK-PGS-0070"
        );
        // WDS 在 PCS 之前
        let early = vec![Segment::new(
            1,
            1,
            SEG_WDS,
            write_wds(&[Window {
                id: 0,
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            }])
            .unwrap(),
        )
        .unwrap()];
        assert_eq!(
            group_display_sets(&early).unwrap_err().code_string(),
            "STK-PGS-0064"
        );
    }

    #[test]
    fn double_palette_is_rejected() {
        let segs = vec![
            Segment::new(1, 1, SEG_PCS, pcs_payload(1920, 1080, &[])).unwrap(),
            Segment::new(1, 1, SEG_PDS, vec![0, 0, 0, 16, 128, 128, 0]).unwrap(),
            Segment::new(1, 1, SEG_PDS, vec![0, 0, 0, 16, 128, 128, 0]).unwrap(),
            Segment::new(1, 1, SEG_END, vec![]).unwrap(),
        ];
        assert_eq!(
            group_display_sets(&segs).unwrap_err().code_string(),
            "STK-PGS-0066"
        );
    }
}
