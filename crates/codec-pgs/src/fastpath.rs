//! PGS Fast Path（13 §3）。只动白名单字段，绝不重编码位图，
//! 并且必须能证明 ODS payload 一个字节都没变（13 §3.1）。

use core_model::error::{Domain, Result, StkError};
use core_model::time::Rational;
use sha2::{Digest, Sha256};

use crate::display_set::{self, write_pcs, write_wds};
use crate::segment::{Segment, SEG_END, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FastOp {
    /// 展示与解码时间一起平移（毫秒）
    OffsetMs(i64),
    /// 时间轴缩放（倍率，有理数）；PTS 与 DTS 分别缩放以保留 decode lead
    ScaleTime(Rational),
    /// 位置平移：PCS 对象与 WDS 窗口一起移动
    MoveBy(i32, i32),
    /// 强制显示标记
    SetForced(bool),
    /// 仅亮度（limited-range Y 标量，万分比）
    PaletteLuma(i32),
}

pub fn apply(segments: &[Segment], ops: &[FastOp]) -> Result<Vec<Segment>> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    let canvas = canvas_of(segments);
    for s in segments {
        let mut s = s.clone();
        s.offset = None;
        for op in ops {
            match op {
                FastOp::OffsetMs(ms) => {
                    let delta = ms.checked_mul(90).ok_or_else(clock_overflow)?;
                    s.pts = shift_clock(s.pts, delta)?;
                    s.dts = shift_clock(s.dts, delta)?;
                }
                FastOp::ScaleTime(r) => {
                    s.pts = scale_clock(s.pts, r)?;
                    s.dts = scale_clock(s.dts, r)?;
                }
                FastOp::MoveBy(dx, dy) => {
                    if s.seg_type == SEG_PCS {
                        s.payload = move_pcs(&s.payload, *dx, *dy)?;
                    } else if s.seg_type == SEG_WDS {
                        s.payload = move_wds(&s.payload, *dx, *dy, canvas)?;
                    }
                }
                FastOp::SetForced(f) => {
                    if s.seg_type == SEG_PCS {
                        s.payload = set_forced(&s.payload, *f)?;
                    }
                }
                FastOp::PaletteLuma(bp) => {
                    if s.seg_type == SEG_PDS {
                        s.payload = crate::palette::luma_scalar(&s.payload, *bp)?;
                    }
                }
            }
        }
        if s.dts > s.pts {
            return Err(StkError::simple(
                Domain::Pgs,
                80,
                "变换后 DTS 晚于 PTS，违反先解码后显示顺序",
            )
            .with_detail(format!("dts={} pts={}", s.dts, s.pts))
            .with_action("该输出不能作为 Strict；需要重新 author 或调整 decode lead"));
        }
        out.push(s);
    }
    Ok(out)
}

fn canvas_of(segments: &[Segment]) -> Option<(u32, u32)> {
    segments
        .iter()
        .find(|s| s.seg_type == SEG_PCS)
        .and_then(|s| display_set::parse_pcs(&s.payload).ok())
        .map(|p| (p.width as u32, p.height as u32))
}

fn clock_overflow() -> StkError {
    StkError::simple(Domain::Pgs, 81, "时间戳计算溢出")
}

fn shift_clock(v: i64, delta: i64) -> Result<i64> {
    let n = v.checked_add(delta).ok_or_else(clock_overflow)?;
    if n < 0 {
        return Err(StkError::simple(
            Domain::Pgs,
            82,
            "平移后时间戳为负（需要更大偏移或先裁剪前面内容）",
        )
        .with_detail(format!("{v} + {delta}")));
    }
    if n >= crate::segment::CLOCK_FIELD_MAX {
        return Err(StkError::simple(
            Domain::Pgs,
            10,
            "时间戳超过 4 字节字段可表示范围",
        ));
    }
    Ok(n)
}

fn scale_clock(v: i64, r: &Rational) -> Result<i64> {
    let n = Rational::from_int(v)
        .mul(r)
        .ok_or_else(clock_overflow)?
        .round_i64()
        .ok_or_else(clock_overflow)?;
    if n < 0 {
        return Err(StkError::simple(Domain::Pgs, 82, "缩放后时间戳为负"));
    }
    if n >= crate::segment::CLOCK_FIELD_MAX {
        return Err(StkError::simple(
            Domain::Pgs,
            10,
            "时间戳超过 4 字节字段可表示范围",
        ));
    }
    Ok(n)
}

fn move_pcs(payload: &[u8], dx: i32, dy: i32) -> Result<Vec<u8>> {
    let mut p = display_set::parse_pcs(payload)?;
    for o in &mut p.objects {
        o.x = move_coord(o.x as i64, dx)? as u16;
        o.y = move_coord(o.y as i64, dy)? as u16;
    }
    write_pcs(&p)
}

fn move_wds(payload: &[u8], dx: i32, dy: i32, canvas: Option<(u32, u32)>) -> Result<Vec<u8>> {
    let mut w = display_set::parse_wds(payload)?;
    for win in &mut w {
        let nx = move_coord(win.x as i64, dx)?;
        let ny = move_coord(win.y as i64, dy)?;
        if let Some((cw, ch)) = canvas {
            if nx > i64::from(cw) || ny > i64::from(ch) {
                return Err(StkError::simple(
                    Domain::Pgs,
                    83,
                    "移动后窗口原点超出 composition canvas",
                )
                .with_detail(format!("({nx},{ny}) vs canvas {cw}x{ch}")));
            }
            if nx + i64::from(win.width) > i64::from(cw)
                || ny + i64::from(win.height) > i64::from(ch)
            {
                return Err(
                    StkError::simple(Domain::Pgs, 84, "移动后窗口超出完整 16:9 画布边界")
                        .with_detail(format!(
                            "window {}x{} at ({nx},{ny})",
                            win.width, win.height
                        )),
                );
            }
        }
        win.x = nx as u16;
        win.y = ny as u16;
    }
    write_wds(&w)
}

fn move_coord(v: i64, d: i32) -> Result<i64> {
    let n = v + i64::from(d);
    if n < 0 {
        return Err(
            StkError::simple(Domain::Pgs, 85, "移动后坐标为负，超出画布")
                .with_detail(format!("{v} + {d}")),
        );
    }
    if n > i64::from(u16::MAX) {
        return Err(StkError::simple(Domain::Pgs, 86, "移动后坐标超出 16bit"));
    }
    Ok(n)
}

fn set_forced(payload: &[u8], forced: bool) -> Result<Vec<u8>> {
    let mut p = display_set::parse_pcs(payload)?;
    for o in &mut p.objects {
        o.forced = forced;
    }
    write_pcs(&p)
}

/// 13 §3.1：任何自称无损快速路径的输出，ODS 数量、顺序与 payload 摘要必须完全一致。
pub fn ods_invariant(before: &[Segment], after: &[Segment]) -> Result<OdsInvariant> {
    let b: Vec<(u16, String)> = ods_digests(before)?;
    let a: Vec<(u16, String)> = ods_digests(after)?;
    if b.len() != a.len() {
        return Err(
            StkError::simple(Domain::Pgs, 87, "Fast Path 改变了 ODS 数量").with_detail(format!(
                "before {} after {}",
                b.len(),
                a.len()
            )),
        );
    }
    for (x, y) in b.iter().zip(&a) {
        if x != y {
            return Err(StkError::simple(
                Domain::Pgs,
                88,
                "Fast Path 修改了 ODS payload，这不是无损路径",
            )
            .with_detail(format!(
                "object {} before={} after={}",
                x.0,
                &x.1[..12.min(x.1.len())],
                &y.1[..12.min(y.1.len())]
            )));
        }
    }
    let aggregate = aggregate_digest(&b);
    Ok(OdsInvariant {
        objects: b.len(),
        // Ok 就意味着逐个比过且相等；不等的结局是 Err(88)，不会走到这里。
        // 真正能被外部复核的证据是下面那两个摘要，不是这个布尔值。
        equal: true,
        per_object: b,
        aggregate,
    })
}

/// 把逐 object 的 SHA-256 折成一个可存档的值：跨版本、跨机器比"这批 ODS 字节没变"，
/// 只需要比这一个十六进制串，不必带着一整份清单（13 §3.1、F4）。
/// 折的方式是"把 id 和它自己的摘要成对拼起来再求 SHA-256"，所以对象序号也包含在内。
fn aggregate_digest(list: &[(u16, String)]) -> String {
    let mut joined = String::new();
    for (id, hex) in list {
        joined.push_str(&format!("{id}:{hex};"));
    }
    let mut d = Sha256::new();
    d.update(joined.as_bytes());
    format!("{:x}", d.finalize())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OdsInvariant {
    pub objects: usize,
    pub equal: bool,
    /// 每个 object 的 ODS payload 的 SHA-256（按出现顺序）。空列表是"这份 SUP 里没有 ODS"，
    /// 不是"检查通过"，所以 `objects` 要一起看。
    pub per_object: Vec<(u16, String)>,
    /// `per_object` 折叠出来的单一摘要。
    pub aggregate: String,
}
pub fn ods_digests(segments: &[Segment]) -> Result<Vec<(u16, String)>> {
    let mut out = Vec::new();
    let mut expected_first_id: Option<u16> = None;
    for s in segments.iter().filter(|s| s.seg_type == SEG_ODS) {
        let h = display_set::parse_ods_header(&s.payload)?;
        if expected_first_id.is_some() && expected_first_id != Some(h.object_id) {
            return Err(StkError::simple(Domain::Pgs, 89, "ODS 分片序列被打断"));
        }
        let mut d = Sha256::new();
        d.update(&s.payload);
        out.push((h.object_id, format!("{:x}", d.finalize())));
        if !h.last {
            expected_first_id = Some(h.object_id);
        } else {
            expected_first_id = None;
        }
    }
    Ok(out)
}

/// 结构自检：display set 级时钟单调 + 每个 set 以 END 收尾 + 段内 DTS 不晚于 PTS
/// （13 §6 Timeline 的最小版本）。
///
/// **单调性只看 PCS**，不看每一段。段各有 PTS/DTS（FFmpeg supenc 按解码角色逐段推导：
/// PCS/WDS/PDS 用 base PTS，首个 ODS 的 PTS 再加对象解码时间，末段沿用上个 ODS 的时钟），
/// 所以真实文件里同一集合内完全可以出现“后一段的 PTS 比前一段小”
/// （实测：PCS 303750 → WDS 303702 → PDS 297870）。把逐段单调当成不变量会误杀全部真文件。
pub fn check_timeline(segments: &[Segment]) -> Result<()> {
    let mut open = false;
    let mut prev_pcs_pts: Option<i64> = None;
    for s in segments {
        match s.seg_type {
            SEG_PCS => {
                if open {
                    return Err(StkError::simple(
                        Domain::Pgs,
                        91,
                        "连续两个 PCS 之间没有 END",
                    ));
                }
                if let Some(p) = prev_pcs_pts {
                    if s.pts <= p {
                        return Err(
                            StkError::simple(Domain::Pgs, 90, "display set 的 PTS 非递增")
                                .with_detail(format!(
                                    "offset={:?} 前一个 PCS {p} -> 本个 PCS {}",
                                    s.offset, s.pts
                                )),
                        );
                    }
                }
                prev_pcs_pts = Some(s.pts);
                open = true;
            }
            SEG_END => {
                if !open {
                    return Err(StkError::simple(Domain::Pgs, 92, "END 没有对应的 PCS"));
                }
                open = false;
            }
            _ => {}
        }
        if s.dts > s.pts {
            return Err(StkError::simple(Domain::Pgs, 80, "段内 DTS 晚于 PTS")
                .with_detail(format!("offset={:?}", s.offset)));
        }
    }
    if open {
        return Err(StkError::simple(
            Domain::Pgs,
            70,
            "文件以未收尾的 display set 结束",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_set::{CompositionObject, Pcs, Window, PCS_EPOCH_START};
    use crate::palette::{write_pds, Palette, PaletteEntry};

    fn pcs(objs: Vec<CompositionObject>) -> Segment {
        let p = Pcs {
            width: 1920,
            height: 1080,
            frame_rate: 0x10,
            composition_number: 3,
            state: PCS_EPOCH_START,
            palette_update: 0,
            palette_id: 0,
            objects: objs,
        };
        Segment::new(900_000, 900_000, SEG_PCS, write_pcs(&p).unwrap()).unwrap()
    }

    fn obj(id: u16, x: u16, y: u16, forced: bool) -> CompositionObject {
        CompositionObject {
            object_id: id,
            window_id: 0,
            cropped: false,
            forced,
            x,
            y,
            crop: None,
        }
    }

    fn window(x: u16, y: u16, w: u16, h: u16) -> Segment {
        Segment::new(
            900_000,
            900_000,
            SEG_WDS,
            write_wds(&[Window {
                id: 0,
                x,
                y,
                width: w,
                height: h,
            }])
            .unwrap(),
        )
        .unwrap()
    }

    fn ods(id: u16) -> Segment {
        let mut p = vec![
            (id >> 8) as u8,
            (id & 0xff) as u8,
            0,
            0xC0,
            0,
            0,
            8,
            0,
            4,
            0,
            2,
        ];
        p.extend_from_slice(&[1, 2, 3, 4]);
        Segment::new(900_000, 900_000, SEG_ODS, p).unwrap()
    }

    fn palette_seg() -> Segment {
        let p = Palette {
            id: 0,
            version: 0,
            entries: vec![
                PaletteEntry::transparent(0),
                PaletteEntry {
                    index: 1,
                    y: 200,
                    cr: 128,
                    cb: 128,
                    alpha: 255,
                },
            ],
        };
        Segment::new(900_000, 900_000, SEG_PDS, write_pds(&p).unwrap()).unwrap()
    }

    fn stream() -> Vec<Segment> {
        vec![
            pcs(vec![obj(1, 100, 800, false)]),
            window(100, 800, 300, 80),
            palette_seg(),
            ods(1),
            Segment::new(900_000, 900_000, SEG_END, vec![]).unwrap(),
        ]
    }

    #[test]
    fn offset_shifts_both_clocks_and_keeps_ods_identical() {
        let before = stream();
        let after = apply(&before, &[FastOp::OffsetMs(1000)]).unwrap();
        assert_eq!(after[0].pts - before[0].pts, 90_000);
        assert_eq!(after[4].dts - before[4].dts, 90_000);
        let inv = ods_invariant(&before, &after).unwrap();
        assert_eq!(inv.objects, 1);
        assert!(inv.equal);
        // 未涉及的段必须逐字节相同
        assert_eq!(after[3].payload, before[3].payload);
    }

    #[test]
    fn negative_offset_that_would_wrap_is_rejected() {
        let before = stream();
        let e = apply(&before, &[FastOp::OffsetMs(-20_000)]).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0082");
    }

    #[test]
    fn scale_preserves_decode_lead() {
        // 输入本身带 decode lead：dts 比 pts 早 9000 tick
        let mut s = stream();
        s[0].dts = s[0].pts - 9_000;
        let out = apply(&s, &[FastOp::ScaleTime(Rational::new(25, 24).unwrap())]).unwrap();
        assert!(out[0].dts < out[0].pts, "缩放后仍需先解码后显示");
        assert_eq!(
            out[0].pts,
            Rational::from_int(900_000)
                .mul(&Rational::new(25, 24).unwrap())
                .unwrap()
                .round_i64()
                .unwrap()
        );
        assert_eq!(ods_invariant(&s, &out).unwrap().objects, 1);
    }

    #[test]
    fn scale_that_violates_decode_order_is_rejected() {
        let mut s = stream();
        s[0].dts = s[0].pts - 9_000;
        // 用不同的 DTS 关系制造违例：把 DTS 推到 PTS 之后
        let e = apply(&s, &[FastOp::OffsetMs(0), FastOp::SetForced(true)]);
        assert!(e.is_ok());
        let mut bad = stream();
        bad[0].dts = bad[0].pts + 1;
        assert_eq!(
            apply(&bad, &[FastOp::OffsetMs(0)])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0080"
        );
    }

    #[test]
    fn move_by_shifts_pcs_and_wds_together() {
        let before = stream();
        let out = apply(&before, &[FastOp::MoveBy(50, -20)]).unwrap();
        let p = display_set::parse_pcs(&out[0].payload).unwrap();
        assert_eq!((p.objects[0].x, p.objects[0].y), (150, 780));
        let w = display_set::parse_wds(&out[1].payload).unwrap();
        assert_eq!((w[0].x, w[0].y), (150, 780));
        assert!(ods_invariant(&before, &out).unwrap().equal);
    }

    #[test]
    fn move_outside_full_canvas_is_rejected() {
        assert_eq!(
            apply(&stream(), &[FastOp::MoveBy(-150, 0)])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0085"
        );
        // 原点仍在画布内、但窗口尾部越界
        assert_eq!(
            apply(&stream(), &[FastOp::MoveBy(0, 201)])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0084"
        );
        // 原点本身就出界
        assert_eq!(
            apply(&stream(), &[FastOp::MoveBy(0, 300)])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0083"
        );
        // 窗口超出画布
        let s = vec![
            pcs(vec![obj(1, 100, 800, false)]),
            window(100, 800, 1800, 80),
            Segment::new(900_000, 900_000, SEG_END, vec![]).unwrap(),
        ];
        assert_eq!(
            apply(&s, &[FastOp::MoveBy(200, 0)])
                .unwrap_err()
                .code_string(),
            "STK-PGS-0084"
        );
    }

    /// 13 §3.1 / F4：这些哈希必须是**payload 字节的 SHA-256**。
    /// 期望值是从外部算好的（openssl/python 对同一批字节求 SHA-256），不是拿实现自己再算一遍 ——
    /// 否则"哈希的是段号而不是字节""折进去的是空集合"这类错都自洽地过。
    #[test]
    fn ods_digests_are_sha256_of_the_payload_bytes() {
        let s = stream();
        let got = ods_digests(&s).unwrap();
        assert_eq!(got.len(), 1, "夹具里就是一个对象：{got:?}");
        assert_eq!(got[0].0, 1, "记的是 ODS 头里的 object id");
        assert_eq!(
            got[0].1, "205d942aea3127613b535dcacc94d0fc893b18d518c42920e883cf99a5f4b2c4",
            "与外部锚点不符：那不是 payload 的 SHA-256"
        );
        // 折成的单一值同样锚定：把 "1:<每个摘要>;" 再求一次 SHA-256
        let inv = ods_invariant(&s, &s).unwrap();
        assert_eq!(inv.objects, 1);
        assert_eq!(
            inv.aggregate, "3cb7f005a37a113869fd68f6e29d4b266c5952110d5e2d948097c75798b7697c",
            "aggregate 的折法变了要有人发现：存档里的值就作废了"
        );
    }

    /// F4 的正证：任何时间轴/标志位的 Fast Path 都不该改动 ODS 字节 —— 判据是可存档的摘要，
    /// 不是"equal 为 true"那种永远成立的布尔。
    #[test]
    fn timing_rewrites_keep_the_ods_digests_identical() {
        let s = stream();
        let base = ods_invariant(&s, &s).unwrap().aggregate;
        for op in [
            FastOp::OffsetMs(1_000),
            FastOp::OffsetMs(-500),
            FastOp::SetForced(true),
        ] {
            let out = apply(&s, &[op]).unwrap();
            let inv = ods_invariant(&s, &out).unwrap();
            assert_eq!(inv.aggregate, base, "{op:?} 之后 ODS 字节变了");
            assert_eq!(inv.objects, 1, "{op:?}");
        }
    }

    #[test]
    fn forced_flag_only_touches_pcs() {
        let before = stream();
        let out = apply(&before, &[FastOp::SetForced(true)]).unwrap();
        let p = display_set::parse_pcs(&out[0].payload).unwrap();
        assert!(p.objects[0].forced);
        assert_eq!(out[1].payload, before[1].payload);
        assert_eq!(out[3].payload, before[3].payload);
        assert!(ods_invariant(&before, &out).unwrap().equal);
        let back = apply(&out, &[FastOp::SetForced(false)]).unwrap();
        assert!(!display_set::parse_pcs(&back[0].payload).unwrap().objects[0].forced);
    }

    #[test]
    fn palette_luma_leaves_ods_and_alpha_alone() {
        let before = stream();
        let out = apply(&before, &[FastOp::PaletteLuma(8000)]).unwrap();
        let p = crate::palette::parse_pds(&out[2].payload).unwrap();
        assert_eq!(p.entries[1].y, 160);
        assert_eq!(p.entries[1].alpha, 255);
        assert_eq!(p.entries[0].cr, 0x80, "chroma 不动");
        assert!(ods_invariant(&before, &out).unwrap().equal);
    }

    #[test]
    fn invariant_detects_a_tampered_object() {
        let before = stream();
        let mut after = before.clone();
        after[3].payload[11] ^= 0xFF;
        let e = ods_invariant(&before, &after).unwrap_err();
        assert_eq!(e.code_string(), "STK-PGS-0088");
        // 少一个对象也要报错
        let mut shorter = before.clone();
        shorter.remove(3);
        assert_eq!(
            ods_invariant(&before, &shorter).unwrap_err().code_string(),
            "STK-PGS-0087"
        );
    }

    #[test]
    fn timeline_checks() {
        assert!(check_timeline(&stream()).is_ok());
        // 逐段 PTS 回退（真实文件里合法）不得报错
        let mut per_segment = stream();
        per_segment[4].pts = 1;
        per_segment[4].dts = 1;
        assert!(
            check_timeline(&per_segment).is_ok(),
            "段间时钟不同是正常的（FFmpeg/真实字幕都这样）"
        );
        // PCS 级 PTS 不递增必须报：它是真正的 display set 时钟
        let mut backwards = stream();
        // 再拼一个完整的 display set（PCS..END），PTS 与第一组相同 → 不递增
        let mut second_set: Vec<Segment> = stream();
        for s in &mut second_set {
            s.offset = None;
        }
        backwards.extend(second_set);
        assert!(
            check_timeline(&backwards).is_err(),
            "两组 PCS 同 PTS 应被报成不递增"
        );
        // PCS 级 PTS 递增时不应报（逐段 PTS 回退是合法的）
        let mut forward = stream();
        let mut second_set: Vec<Segment> = stream();
        for s in &mut second_set {
            s.pts += 90_000;
            s.dts += 90_000;
            s.offset = None;
        }
        forward.extend(second_set);
        assert!(
            check_timeline(&forward).is_ok(),
            "第二组 PCS 晚于第一组就该过：{:?}",
            check_timeline(&forward).unwrap_err()
        );
        let unclosed = vec![pcs(vec![obj(1, 0, 0, false)])];
        assert_eq!(
            check_timeline(&unclosed).unwrap_err().code_string(),
            "STK-PGS-0070"
        );
        let orphan_end = vec![Segment::new(1, 1, SEG_END, vec![]).unwrap()];
        assert_eq!(
            check_timeline(&orphan_end).unwrap_err().code_string(),
            "STK-PGS-0092"
        );
    }

    #[test]
    fn offsets_are_dropped_on_output_segments() {
        let with_offset = {
            let mut s = stream();
            s[0].offset = Some(0);
            s
        };
        let out = apply(&with_offset, &[FastOp::OffsetMs(10)]).unwrap();
        assert_eq!(out[0].offset, None, "变换后原偏移不再有效");
    }
}
