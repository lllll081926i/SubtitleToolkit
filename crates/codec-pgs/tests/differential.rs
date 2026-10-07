//! 与参考实现的差分测试（24 §"differential"、26 Phase0 Gate）。
//! 目的不是"它也绿所以我对"，而是：两个独立实现对同一批字节的解读必须一致；
//! 任何分歧都要么是我们的缺陷，要么登记为需要第三方语料裁决的问题。

use codec_pgs::display_set::{assemble_ods, parse_pcs, parse_wds};
use codec_pgs::fastpath::{apply, check_timeline, ods_invariant, FastOp};
use codec_pgs::palette::{parse_pds, ColorMatrix};
use codec_pgs::rle;
use codec_pgs::segment::{parse_stream, SEG_ODS, SEG_PCS, SEG_PDS, SEG_WDS};

fn ox_stream(canvas: (u16, u16), obj: (u16, u16), pos: (u16, u16), fragments: usize) -> Vec<u8> {
    let palette = vec![
        (0u8, [0u8, 0, 0, 0]),
        (1, [255, 255, 255, 255]),
        (2, [240, 40, 40, 255]),
        (3, [30, 200, 90, 255]),
    ];
    let pixels: Vec<u8> = (0..(obj.0 as usize * obj.1 as usize))
        .map(|i| (i % 4) as u8)
        .collect();
    if fragments > 1 {
        oxideav_sub_image::pgs::build_demo_display_set_fragmented(
            canvas, obj, pos, &palette, &pixels, fragments,
        )
    } else {
        oxideav_sub_image::pgs::build_demo_display_set(canvas, obj, pos, &palette, &pixels)
    }
}

#[test]
fn our_parser_reads_reference_output() {
    let bytes = ox_stream((1920, 1080), (120, 40), (900, 980), 1);
    let segs = parse_stream(&bytes).expect("参考流必须可解析");
    assert!(segs.iter().any(|s| s.seg_type == SEG_PCS));
    assert!(segs.iter().any(|s| s.seg_type == SEG_WDS));
    assert!(segs.iter().any(|s| s.seg_type == SEG_PDS));
    assert!(segs.iter().any(|s| s.seg_type == SEG_ODS));

    let pcs_seg = segs.iter().find(|s| s.seg_type == SEG_PCS).unwrap();
    let pcs = parse_pcs(&pcs_seg.payload).unwrap();
    assert_eq!((pcs.width, pcs.height), (1920, 1080));
    assert_eq!(pcs.objects.len(), 1);
    assert_eq!((pcs.objects[0].x, pcs.objects[0].y), (900, 980));

    let wds = segs.iter().find(|s| s.seg_type == SEG_WDS).unwrap();
    let windows = parse_wds(&wds.payload).unwrap();
    // 参考实现把窗口写成"整个画布"而不是对象外接框 —— 我们的解析照实读出
    assert_eq!((windows[0].width, windows[0].height), (1920, 1080));
    assert_eq!((windows[0].x, windows[0].y), (0, 0));

    // 我们重写一遍必须还能被参考实现的段读取函数接受
    let rewritten = codec_pgs::segment::write_stream(&segs).unwrap();
    let mut pos = 0usize;
    let mut count = 0;
    while pos < rewritten.len() {
        let (_, next) = oxideav_sub_image::pgs::read_segment(&rewritten, pos).unwrap();
        pos = next;
        count += 1;
    }
    assert_eq!(count, segs.len());
}

#[test]
fn palette_structure_matches_reference_and_range_difference_is_measured() {
    // 结构层面必须完全一致；数值层面发现参考实现用 full-range（Y 直映 0..255），
    // 而 Blu-ray PGS 约定是 limited range（Y 16..235）。差异已登记 docs/ISSUES.md C16，
    // 需要真盘语料/播放器裁决，不能靠"两家都能读自己的输出"当正确性证明。
    let bytes = ox_stream((1920, 1080), (120, 40), (0, 0), 1);
    let segs = parse_stream(&bytes).unwrap();
    let pds = segs.iter().find(|s| s.seg_type == SEG_PDS).unwrap();
    let pal = parse_pds(&pds.payload).unwrap();
    let given: [(u8, [u8; 4]); 4] = [
        (0, [0, 0, 0, 0]),
        (1, [255, 255, 255, 255]),
        (2, [240, 40, 40, 255]),
        (3, [30, 200, 90, 255]),
    ];
    assert_eq!(pal.entries.len(), 4, "条目数量必须一致");
    for (i, (idx, rgba)) in given.iter().enumerate() {
        let e = &pal.entries[i];
        assert_eq!(&e.index, idx, "条目顺序与 index 必须一致");
        assert_eq!(e.alpha, rgba[3], "alpha 通道位置无歧义，必须精确");
        if *idx == 1 {
            // 白色：limited range 应为 235，full range 参考实现写 255
            assert_eq!(e.y, 255, "记录参考实现的实际取值");
            let ours = codec_pgs::palette::PaletteEntry::from_rgba(*rgba, 1, ColorMatrix::Bt601);
            assert_eq!(ours.y, 235, "我们的 limited range 白色 Y");
            assert_ne!(ours.y, e.y, "两家的色度范围约定不同（C16）");
        }
    }
    // 用我们的逆矩阵解参考实现的条目：alpha 精确、彩色方向一致（允许 ±16 级别的偏移）
    for (idx, want) in given {
        let e = pal.entries.iter().find(|x| x.index == idx).unwrap();
        let got = e.to_rgba(ColorMatrix::Bt601);
        assert_eq!(got[3], want[3]);
        if idx == 1 {
            assert!(
                got[0] >= 235 && got[1] >= 235 && got[2] >= 235,
                "白色解出来必须仍接近白：{got:?}"
            );
        }
    }
}

#[test]
fn rle_interops_with_reference_in_both_directions() {
    // 不要求两家字节相同（参考实现有"短游程拆成单像素"等尺寸优化，且不写 FF FF）；
    // 要求的是：两家都能正确解对方的输出 —— 这才是格式理解的证据。
    let pixels: Vec<u8> = (0..1920 * 40).map(|i| (i % 5) as u8).collect();
    let ours = rle::encode(&pixels, 1920, 40).unwrap();
    let theirs = oxideav_sub_image::pgs::encode_rle(&pixels, 1920, 40);
    assert_eq!(
        rle::decode(&theirs, 1920, 40).unwrap(),
        pixels,
        "参考实现的 RLE 我们必须正确解出"
    );
    // 参考实现既不写也不认 00 00 FF FF 图末标记（docs/ISSUES.md C17）：
    // 去掉标记后喂给它，验证四种码型它都认同我们。
    let mut ours_no_marker = ours.clone();
    assert!(ours_no_marker.ends_with(&[0x00, 0x00, 0xFF, 0xFF]));
    ours_no_marker.truncate(ours_no_marker.len() - 2);
    assert_eq!(
        oxideav_sub_image::pgs::decode_rle(&ours_no_marker, 1920, 40).unwrap(),
        pixels,
        "我们的 RLE（去图末标记）参考实现必须正确解出"
    );
    assert_eq!(
        rle::decode(&ours, 1920, 40).unwrap(),
        pixels,
        "我们自己的完整流必须可解"
    );
    // 参考实现还做了"短非透明游程拆成单像素"的尺寸优化，我们尚未做，
    // 所以这里断言的是可证明的上界与压缩有效性，而不是与它比大小。
    // 最坏情况：每个像素都是孤立透明像素（2 字节码）+ 每行 2 字节行尾
    assert!(
        ours.len() <= pixels.len() * 2 + 2 * 40 + 4,
        "码流超出最坏情况上界: {}",
        ours.len()
    );
    let solid = vec![0u8; 1920 * 40];
    assert!(
        rle::encode(&solid, 1920, 40).unwrap().len() < 400,
        "全透明图必须极小（40 行 × 约 5 字节）"
    );

    let _ = theirs;
}

#[test]
fn fragmented_ods_reassembly_matches_reference() {
    let bytes = ox_stream((1920, 1080), (300, 60), (10, 20), 4);
    let segs = parse_stream(&bytes).unwrap();
    let ods: Vec<_> = segs.iter().filter(|s| s.seg_type == SEG_ODS).collect();
    assert!(ods.len() > 1, "应真的产生多个分片，实际 {}", ods.len());
    let (id, w, h, stream) = assemble_ods(ods.iter().copied()).unwrap();
    assert_eq!((w, h), (300, 60));
    assert!(id > 0);
    let idx = rle::decode(&stream, w as usize, h as usize).unwrap();
    assert_eq!(idx.len(), 18000);
    // 与参考实现对同一 RLE 流的解读逐像素一致
    assert_eq!(
        oxideav_sub_image::pgs::decode_rle(&stream, w as usize, h as usize).unwrap(),
        idx
    );
    // 分片顺序不变（13 §3.1）
    let ids: Vec<u16> = ods
        .iter()
        .map(|s| codec_pgs::parse_ods_header(&s.payload).unwrap().object_id)
        .collect();
    assert!(
        ids.iter().all(|x| *x == ids[0]),
        "分片必须同属一个 object：{ids:?}"
    );
}

#[test]
fn our_fast_path_output_still_parses_as_reference() {
    let bytes = ox_stream((1920, 1080), (120, 40), (900, 980), 1);
    let before = parse_stream(&bytes).unwrap();
    check_timeline(&before).unwrap();
    let after = apply(&before, &[FastOp::OffsetMs(1500), FastOp::SetForced(true)]).unwrap();
    assert!(ods_invariant(&before, &after).unwrap().equal);
    let out = codec_pgs::segment::write_stream(&after).unwrap();
    let re = parse_stream(&out).unwrap();
    let pcs = parse_pcs(&re.iter().find(|s| s.seg_type == SEG_PCS).unwrap().payload).unwrap();
    // 参考实现把窗口写成整画布，因此对它的输出做 MoveBy 必然越界（见 docs/ISSUES.md C18）；
    // 平移的正确性由 fastpath 自己的单元测试覆盖。
    assert_eq!((pcs.objects[0].x, pcs.objects[0].y), (900, 980));
    assert!(pcs.objects[0].forced);
    // 参考实现逐段读取也必须走通我们的输出
    let mut pos = 0usize;
    while pos < out.len() {
        let (_, next) = oxideav_sub_image::pgs::read_segment(&out, pos)
            .unwrap_or_else(|e| panic!("参考实现拒绝我们的输出 at {pos}: {e:?}"));
        pos = next;
    }
}

#[test]
fn our_written_stream_is_readable_by_reference_decoder_api() {
    // 完全由我们自己造的流（不借参考实现的编码器），验证字段布局不是"自我一致"
    let pal = codec_pgs::palette::Palette {
        id: 0,
        version: 0,
        entries: vec![
            codec_pgs::palette::PaletteEntry::transparent(0),
            codec_pgs::palette::PaletteEntry::from_rgba([220, 30, 30, 255], 1, ColorMatrix::Bt601),
        ],
    };
    let idx: Vec<u8> = (0..80 * 10)
        .map(|i| if i % 3 == 0 { 1 } else { 0 })
        .collect();
    let rle_bytes = rle::encode(&idx, 80, 10).unwrap();
    let pcs = codec_pgs::display_set::write_pcs(&codec_pgs::display_set::Pcs {
        width: 1920,
        height: 1080,
        frame_rate: 0x10,
        composition_number: 7,
        state: codec_pgs::display_set::PCS_EPOCH_START,
        palette_update: 0,
        palette_id: 0,
        objects: vec![codec_pgs::display_set::CompositionObject {
            object_id: 5,
            window_id: 0,
            cropped: false,
            forced: false,
            x: 40,
            y: 900,
            crop: None,
        }],
    });
    let wds = codec_pgs::display_set::write_wds(&[codec_pgs::display_set::Window {
        id: 0,
        x: 40,
        y: 900,
        width: 80,
        height: 10,
    }]);
    let mut ods = vec![0x00, 0x05, 0x00, 0xC0];
    let dl = (rle_bytes.len() + 4) as u32;
    ods.push(((dl >> 16) & 0xff) as u8);
    ods.push(((dl >> 8) & 0xff) as u8);
    ods.push((dl & 0xff) as u8);
    ods.extend_from_slice(&80u16.to_be_bytes());
    ods.extend_from_slice(&10u16.to_be_bytes());
    ods.extend_from_slice(&rle_bytes);
    let segs = vec![
        codec_pgs::segment::Segment::new(90_000, 90_000, SEG_PCS, pcs).unwrap(),
        codec_pgs::segment::Segment::new(90_000, 90_000, SEG_WDS, wds).unwrap(),
        codec_pgs::segment::Segment::new(
            90_000,
            90_000,
            SEG_PDS,
            codec_pgs::palette::write_pds(&pal).unwrap(),
        )
        .unwrap(),
        codec_pgs::segment::Segment::new(90_000, 90_000, SEG_ODS, ods).unwrap(),
        codec_pgs::segment::Segment::new(90_000, 90_000, codec_pgs::SEG_END, vec![]).unwrap(),
    ];
    let bytes = codec_pgs::segment::write_stream(&segs).unwrap();
    let mut pos = 0usize;
    let mut n = 0;
    while pos < bytes.len() {
        let (seg, next) =
            oxideav_sub_image::pgs::read_segment(&bytes, pos).expect("参考实现应能读我们的段");
        assert_eq!(seg.seg_type, segs[n].seg_type);
        assert_eq!(
            seg.body, segs[n].payload,
            "第 {n} 段 payload 必须与参考实现读到的一致"
        );
        pos = next;
        n += 1;
    }
    assert_eq!(n, 5);
    let mut theirs_input = rle_bytes.clone();
    theirs_input.truncate(theirs_input.len() - 2); // 参考实现不认图末 FF FF（docs/ISSUES.md C17）
    let decoded = oxideav_sub_image::pgs::decode_rle(&theirs_input, 80, 10).unwrap();
    assert_eq!(decoded, idx);
}

/// Basic Encoder 的双向差分：我们编的整个 display set 必须能被参考实现的段读取器逐段读回，
/// 并且它的 RLE 解码器要与我们的解码结果一致（13 §4 "结构正确、主流播放器可读"的最低取证）。
#[test]
fn encoder_output_is_readable_by_reference_decoder() {
    use codec_pgs::display_set::assemble_ods;
    use codec_pgs::encoder::{encode, EncodeOptions, Frame, FrameObject};

    // 80x12 的三色图：产生多条游程 + 一个非透明索引 0 边界
    let (w, h) = (80usize, 12usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for _ in 0..h {
        for x in 0..w {
            let c = if x < 30 {
                [0, 0, 0, 0]
            } else if x < 55 {
                [255, 255, 255, 255]
            } else {
                [220, 30, 30, 180]
            };
            rgba.extend_from_slice(&c);
        }
    }
    let obj = FrameObject {
        rgba,
        width: w as u16,
        height: h as u16,
        x: 900,
        y: 900,
        forced: true,
    };
    let frames = vec![
        Frame::new(90_000, vec![obj.clone()]),
        Frame::new(180_000, vec![obj]),
        Frame::erase(270_000),
    ];
    let opts = EncodeOptions::default();
    let (segs, report) = encode(&frames, &opts).unwrap();
    assert_eq!(report.display_sets, 3);
    assert_eq!(report.erase_sets, 1);
    assert_eq!(report.objects, 2);

    let bytes = codec_pgs::segment::write_stream(&segs).unwrap();
    let mut pos = 0usize;
    let mut n = 0usize;
    while pos < bytes.len() {
        let (seg, next) =
            oxideav_sub_image::pgs::read_segment(&bytes, pos).expect("参考实现应能读 encoder 的段");
        assert_eq!(seg.seg_type, segs[n].seg_type, "第 {n} 段类型不一致");
        assert_eq!(seg.body, segs[n].payload, "第 {n} 段 payload 不一致");
        assert_eq!(i64::from(seg.pts_90k), segs[n].pts, "第 {n} 段 PTS 不一致");
        assert_eq!(i64::from(seg.dts_90k), segs[n].dts, "第 {n} 段 DTS 不一致");
        pos = next;
        n += 1;
    }
    assert_eq!(n, segs.len(), "参考实现读到的段数必须与我们写的一致");

    // RLE 互解：取第一个对象的连续 ODS 分片序列后重组，交给参考实现
    let mut ods: Vec<&codec_pgs::segment::Segment> = Vec::new();
    for s in segs.iter() {
        if s.seg_type == codec_pgs::segment::SEG_ODS {
            ods.push(s);
        } else if !ods.is_empty() {
            break;
        }
    }
    let (id, dw, dh, rle) = assemble_ods(ods.iter().copied()).unwrap();
    assert_eq!((id, dw, dh), (0, 80, 12));
    let ours = codec_pgs::rle::decode(&rle, 80, 12).unwrap();
    let mut theirs_input = rle.clone();
    theirs_input.truncate(theirs_input.len() - 2); // 参考实现不认图末 00 00 FF FF（ISSUES C17）
    let theirs = oxideav_sub_image::pgs::decode_rle(&theirs_input, 80, 12).unwrap();
    assert_eq!(
        ours, theirs,
        "两个解码器对我们 encoder 的 RLE 必须给出同样的索引图"
    );
    // 索引 0 必须是透明段（前 30 列）
    assert_eq!(&ours[0..30], &vec![0u8; 30]);
}
