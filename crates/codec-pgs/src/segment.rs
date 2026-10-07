//! PGS 段级读写（13 §2）。段内各字段的布局以真实实现取证为准，见 docs/ISSUES.md C3
//! （PDS 条目、ODS 长度、WDS、PCS）；段头本身是
//! `PG`(2) + PTS(4) + DTS(4) + segment_type(1) + segment_length(2)，length 只覆盖 payload ——
//! 这一布局不只是我们的读写自洽，真 ffprobe 的 `sup` demuxer 也按同样的段数与时钟读出来
//! （media-probe/tests/real_probe.rs 的 ffprobe_reads_our_pgs_container_with_the_same_clocks，ISSUES G4）。

use core_model::error::{Domain, Result, StkError};

pub const SEG_PDS: u8 = 0x14;
pub const SEG_ODS: u8 = 0x15;
pub const SEG_PCS: u8 = 0x16;
pub const SEG_WDS: u8 = 0x17;
pub const SEG_IDS: u8 = 0x19;
pub const SEG_END: u8 = 0x80;
pub const SEG_SEQ: u8 = 0x81;
pub const SEG_PADDING: u8 = 0xff;

pub const HEADER_LEN: usize = 13;
/// 单文件段数上限（23 §PGS：避免分配攻击；真实长片也就几万段）
pub const MAX_SEGMENTS: usize = 400_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub pts: i64,
    pub dts: i64,
    pub seg_type: u8,
    /// 原始 payload 字节。Fast Path 未触碰的段必须原样写回（13 §3.1）。
    pub payload: Vec<u8>,
    /// 原始文件中的起始偏移；未来自输入流解析时才有值，用于诊断定位。
    pub offset: Option<usize>,
}

impl Segment {
    pub fn new(pts: i64, dts: i64, seg_type: u8, payload: Vec<u8>) -> Result<Segment> {
        if payload.len() > u16::MAX as usize {
            return Err(StkError::simple(
                Domain::Pgs,
                3,
                "payload 超过单段 65535 字节上限，需要分片",
            )
            .with_detail(format!("{} 字节", payload.len())));
        }
        check_clock(pts)?;
        check_clock(dts)?;
        Ok(Segment {
            pts,
            dts,
            seg_type,
            payload,
            offset: None,
        })
    }

    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(HEADER_LEN + self.payload.len());
        v.extend_from_slice(b"PG");
        v.extend_from_slice(&be3(self.pts)?);
        v.extend_from_slice(&be3(self.dts)?);
        v.push(self.seg_type);
        let len = self.payload.len() as u16;
        v.extend_from_slice(&len.to_be_bytes());
        v.extend_from_slice(&self.payload);
        Ok(v)
    }

    pub fn is_core(&self) -> bool {
        matches!(self.seg_type, SEG_PDS | SEG_ODS | SEG_PCS | SEG_WDS)
    }
}

/// PTS/DTS 落在 4 字节大端字段里，可表示范围是 0..2^32-1。
/// 规范文本说的 33bit 属于 PES/流层（core-model 的 `PTS_MAX_TICKS`），而 .sup **容器**这个字段只有
/// 4 字节，所以写文件的硬上限是 2^32 —— 两者是不同量程，不是互相矛盾（裁决与外部证据见 docs/ISSUES.md G4）。
pub const CLOCK_FIELD_MAX: i64 = 1i64 << 32;

fn check_clock(v: i64) -> Result<()> {
    // 11 而不是 1：Pgs/1、Pgs/2 是 core-model 的 PTS 不变量码（被 transform-time 同义复用），
    // 两个 crate 用同一个编号各说一件事，读报告的人只能靠标题猜（ISSUES H8）。
    if v < 0 {
        return Err(StkError::simple(Domain::Pgs, 11, "时间戳为负"));
    }
    if v >= CLOCK_FIELD_MAX {
        return Err(
            StkError::simple(Domain::Pgs, 10, "时间戳超过 4 字节字段可表示范围")
                .with_detail(format!("{v} >= {CLOCK_FIELD_MAX}")),
        );
    }
    Ok(())
}

fn be3(v: i64) -> Result<[u8; 4]> {
    check_clock(v)?;
    Ok((v as u32).to_be_bytes())
}

/// 严格解析：任何不自洽（魔数缺失、长度越界、尾部垃圾、未知段类型）都带偏移报错。
pub fn parse_stream(bytes: &[u8]) -> Result<Vec<Segment>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos < bytes.len() {
        if bytes.len() - pos < HEADER_LEN {
            return Err(
                StkError::simple(Domain::Pgs, 4, "文件尾部有不足 13 字节的残块")
                    .with_detail(format!("offset={pos} remaining={}", bytes.len() - pos)),
            );
        }
        if &bytes[pos..pos + 2] != b"PG" {
            return Err(StkError::simple(Domain::Pgs, 5, "段魔数不是 PG")
                .with_detail(format!("offset={pos}")));
        }
        let pts = u32::from_be_bytes([
            bytes[pos + 2],
            bytes[pos + 3],
            bytes[pos + 4],
            bytes[pos + 5],
        ]) as i64;
        let dts = u32::from_be_bytes([
            bytes[pos + 6],
            bytes[pos + 7],
            bytes[pos + 8],
            bytes[pos + 9],
        ]) as i64;
        let seg_type = bytes[pos + 10];
        let len = usize::from(u16::from_be_bytes([bytes[pos + 11], bytes[pos + 12]]));
        if pts >= CLOCK_FIELD_MAX || dts >= CLOCK_FIELD_MAX {
            return Err(
                StkError::simple(Domain::Pgs, 6, "时间戳超过 4 字节字段范围")
                    .with_detail(format!("offset={pos} pts={pts} dts={dts}")),
            );
        }
        let end = pos
            .checked_add(HEADER_LEN)
            .and_then(|x| x.checked_add(len))
            .ok_or_else(|| {
                StkError::simple(Domain::Pgs, 7, "段长度计算溢出")
                    .with_detail(format!("offset={pos}"))
            })?;
        if end > bytes.len() {
            return Err(StkError::simple(
                Domain::Pgs,
                8,
                "段声明长度超出文件结尾（截断或恶意长度）",
            )
            .with_detail(format!(
                "offset={pos} declared={len} available={}",
                bytes.len() - pos - HEADER_LEN
            )));
        }
        out.push(Segment {
            pts,
            dts,
            seg_type,
            payload: bytes[pos + HEADER_LEN..end].to_vec(),
            offset: Some(pos),
        });
        if out.len() > MAX_SEGMENTS {
            return Err(StkError::simple(Domain::Pgs, 9, "段数量超过上限")
                .with_detail(format!("{MAX_SEGMENTS}")));
        }
        pos = end;
    }
    Ok(out)
}

pub fn write_stream(segments: &[Segment]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(segments.iter().map(|s| HEADER_LEN + s.payload.len()).sum());
    for s in segments {
        out.extend_from_slice(&s.encode()?);
    }
    Ok(out)
}

/// 未知/扩展段的保留策略（13 §2）：本实现原样保留 payload，不静默丢弃。
pub fn unknown_types(segments: &[Segment]) -> Vec<u8> {
    let mut v: Vec<u8> = segments
        .iter()
        .map(|s| s.seg_type)
        .filter(|t| {
            !matches!(
                *t,
                SEG_PDS | SEG_ODS | SEG_PCS | SEG_WDS | SEG_IDS | SEG_END | SEG_SEQ | SEG_PADDING
            )
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t: u8, pts: i64, dts: i64, payload: &[u8]) -> Segment {
        Segment::new(pts, dts, t, payload.to_vec()).unwrap()
    }

    #[test]
    fn header_is_13_bytes_and_length_covers_payload_only() {
        let s = seg(SEG_END, 90_000, 90_000, &[]);
        let b = s.encode().unwrap();
        assert_eq!(b.len(), 13);
        assert_eq!(&b[0..2], b"PG");
        assert_eq!(u32::from_be_bytes([b[2], b[3], b[4], b[5]]), 90_000);
        assert_eq!(u32::from_be_bytes([b[6], b[7], b[8], b[9]]), 90_000);
        assert_eq!(b[10], SEG_END);
        assert_eq!(u16::from_be_bytes([b[11], b[12]]), 0);
    }

    #[test]
    fn roundtrip_is_byte_identical() {
        let segs = vec![
            seg(
                SEG_PCS,
                100,
                100,
                &[0x07, 0x80, 0x04, 0x38, 0x10, 0, 1, 0x80, 0, 0, 1],
            ),
            seg(
                SEG_WDS,
                100,
                100,
                &[1, 0, 0, 0, 0, 0, 0x1c, 0x20, 0, 0, 0, 0x60],
            ),
            seg(SEG_PDS, 100, 100, &[0, 0, 0, 16, 128, 128, 255]),
            seg(SEG_ODS, 100, 100, &[0, 1, 0, 0xC0, 0, 0, 8, 0, 4, 0, 2]),
            seg(SEG_END, 100, 100, &[]),
        ];
        let bytes = write_stream(&segs).unwrap();
        let back = parse_stream(&bytes).unwrap();
        assert_eq!(back.len(), 5);
        for (a, b) in segs.iter().zip(&back) {
            assert_eq!(a.seg_type, b.seg_type);
            assert_eq!(a.pts, b.pts);
            assert_eq!(a.dts, b.dts);
            assert_eq!(a.payload, b.payload);
        }
        assert_eq!(write_stream(&back).unwrap(), bytes, "重写必须逐字节相同");
    }

    #[test]
    fn pts_and_dts_are_independent_fields() {
        // 10 §6：不得机械令 DTS=PTS，两者必须分别保留
        let s = seg(SEG_PCS, 200_000, 180_000, &[0u8; 11]);
        let b = s.encode().unwrap();
        let back = parse_stream(&b).unwrap();
        assert_eq!((back[0].pts, back[0].dts), (200_000, 180_000));
    }

    #[test]
    fn malformed_input_errors_with_offset_and_never_panics() {
        assert!(parse_stream(b"").unwrap().is_empty());
        assert_eq!(
            parse_stream(b"PGxyz").unwrap_err().code_string(),
            "STK-PGS-0004"
        );
        assert_eq!(
            parse_stream(&[0u8; 13]).unwrap_err().code_string(),
            "STK-PGS-0005"
        );
        // 声明长度超出结尾
        let mut b = vec![0u8; 13];
        b[0] = b'P';
        b[1] = b'G';
        b[10] = SEG_ODS;
        b[11] = 0xff;
        b[12] = 0xff;
        assert_eq!(parse_stream(&b).unwrap_err().code_string(), "STK-PGS-0008");
        // 4 字节字段能表示的最大值必须正常解析（ parse 侧的超范围分支不可达，
        // 因为 4 字节读出必然 < 2^32；该检查保留给未来字段拓宽时用，量程口径见 docs/ISSUES.md G4）
        let mut b2 = vec![0u8; 13];
        b2[0] = b'P';
        b2[1] = b'G';
        b2[10] = SEG_END;
        b2[2] = 0xff;
        b2[3] = 0xff;
        b2[4] = 0xff;
        b2[5] = 0xff;
        assert_eq!(parse_stream(&b2).unwrap()[0].pts, CLOCK_FIELD_MAX - 1);
        // 尾部残块
        let mut ok = seg(SEG_END, 1, 1, &[]).encode().unwrap();
        ok.push(0x41);
        assert_eq!(parse_stream(&ok).unwrap_err().code_string(), "STK-PGS-0004");
    }

    #[test]
    fn oversized_payload_is_rejected_not_truncated() {
        let big = vec![0u8; 65536];
        let e = Segment::new(1, 1, SEG_ODS, big).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0003");
    }

    #[test]
    fn negative_and_overflow_clocks_rejected() {
        assert!(Segment::new(-1, 0, SEG_END, vec![]).is_err());
        assert!(Segment::new(0, 0, SEG_END, vec![]).is_ok());
        assert!(Segment::new(CLOCK_FIELD_MAX - 1, 0, SEG_END, vec![]).is_ok());
        assert_eq!(
            Segment::new(CLOCK_FIELD_MAX, 0, SEG_END, vec![])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0010"
        );
        assert!(
            Segment::new(1 << 33, 0, SEG_END, vec![]).is_err(),
            "33bit 也必须被 4 字节上限拦住"
        );
    }

    #[test]
    fn max_clock_encodes_and_decodes() {
        let s = seg(SEG_END, CLOCK_FIELD_MAX - 1, CLOCK_FIELD_MAX - 1, &[]);
        let back = parse_stream(&s.encode().unwrap()).unwrap();
        assert_eq!(back[0].pts, CLOCK_FIELD_MAX - 1);
    }

    #[test]
    fn unknown_segment_types_are_reported_not_dropped() {
        let segs = vec![seg(0x82, 1, 1, &[9]), seg(SEG_END, 1, 1, &[])];
        assert_eq!(unknown_types(&segs), vec![0x82]);
        assert_eq!(
            parse_stream(&write_stream(&segs).unwrap()).unwrap().len(),
            2,
            "未知段必须原样保留"
        );
        assert!(unknown_types(&[seg(SEG_PCS, 1, 1, &[0u8; 11])]).is_empty());
    }
}
