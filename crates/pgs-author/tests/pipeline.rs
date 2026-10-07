//! 渲染位图 → author → PGS 段的整链路证明（12 §1A、09 §3、13 §4）。
//!
//! 这里用一个**合成 provider**：直接给出 `render_libass` 认识的 `ImageDesc`
//! （字形 + 半透明阴影），跑过
//! `composite → image_core::Frame → pgs_author::plan/author → validator → 读回段流`。
//! 合成而不是调 DLL，是为了让这条链路在没有 native 依赖的机器上也能跑；
//! 真实 libass 的语义由 `crates/render-libass/tests/render_real.rs` 负责（ISSUES R/Z）。
//! 每一环都是已实现代码；串起来测一次比分开测更容易暴露接口不匹配
//! （alpha 语义、坐标空间、颜色打包顺序这类"两边各自自洽"的约定）——
//! 这句话不是空话：ASS 的 alpha 字节是"透明度"，本文件的 `color` 必须按那个方向写，
//! 写成直通 alpha 会得到一张全透明的画布（ISSUES Z1）。

use codec_pgs::display_set::group_display_sets;
use codec_pgs::encoder::EncodeOptions;
use codec_pgs::segment::parse_stream;
use core_model::canvas::Canvas;
use core_model::loss::LossReport;
use core_model::profile::ValidationLevel;
use core_model::time::{Fps, Time};
use image_core::Frame;
use pgs_author::{AuthorOptions, Sample};
use render_libass::{composite, ImageDesc, ImageKind};
use validator::ProfileWant;

const C: Canvas = Canvas::HD720;

fn t(ms: i64) -> Time {
    Time::from_millis(ms).unwrap()
}

/// 按 libass 的 `ASS_Image.color` 打包方向构造颜色（ISSUES Z1）。
///
/// `alpha` 这里是**期望的不透明度**（255=不透明），而 color 的最低字节存的是
/// "透明度"，所以打包时要取反。写这个 helper 的原因是：直接把 `0xFF_FF_FF_FF`
/// 当成"不透明白"是这个测试曾经犯过的错，而它当时"全绿"。
fn ass_color(r: u8, g: u8, b: u8, alpha: u8) -> u32 {
    (u32::from(r) << 24) | (u32::from(g) << 16) | (u32::from(b) << 8) | u32::from(255 - alpha)
}

/// 一帧"渲染输出"：一条白字 + 右下偏移 1px 的半透明黑阴影。
fn rendered_line(y_offset: i32) -> Frame {
    let mut glyph = [0u8; 120 * 12];
    for (i, b) in glyph.iter_mut().enumerate() {
        *b = if i % 7 == 0 { 200 } else { 255 };
    }
    let shadow = [90u8; 120 * 12];
    let images = [
        ImageDesc {
            width: 120,
            height: 12,
            stride: 120,
            bitmap: &shadow,
            color: ass_color(0, 0, 0, 128),
            dst_x: 301,
            dst_y: 601 + y_offset,
            kind: ImageKind::Shadow,
        },
        ImageDesc {
            width: 120,
            height: 12,
            stride: 120,
            bitmap: &glyph,
            color: ass_color(255, 255, 255, 255),
            dst_x: 300,
            dst_y: 600 + y_offset,
            kind: ImageKind::Character,
        },
    ];
    let mut loss = LossReport::new();
    composite(C, &images, &mut loss).expect("合成必须成功")
}

#[test]
fn color_packing_agrees_with_the_renderer_that_reads_libass() {
    // 这个测试文件自己打包颜色，render-libass 自己解释颜色；两边都由我写，
    // 于是"自洽的错误"（直通 alpha）曾同时存在于两侧。用对方的拆包函数把
    // 自己的打包钉住，任何一个方向被改动都会在这里撞车。
    assert_eq!(
        render_libass::unpack_color(ass_color(0x11, 0x22, 0x33, 200)),
        [0x11, 0x22, 0x33, 200]
    );
    assert_eq!(
        render_libass::unpack_color(ass_color(255, 255, 255, 255)),
        [255, 255, 255, 255],
        "ASS 的不透明是最低字节 0x00，不是 0xFF"
    );
    assert_eq!(
        render_libass::unpack_color(ass_color(0, 0, 0, 0)),
        [0, 0, 0, 0],
        "alpha=0 必须打包成完全透明（libass 会直接丢掉这种位图）"
    );
}

#[test]
fn semi_transparent_pixels_survive_crop_and_bbox() {
    let frame = rendered_line(0);
    let patch = frame
        .crop()
        .unwrap()
        .expect("有半透明内容就必须有 bbox，不能被判成全透明");
    // 字形 300..=419 / 阴影 301..=420 -> bbox x=300 w=121；行 600..=612 -> y=600 h=13
    assert_eq!((patch.x, patch.y), (300, 600), "bbox 必须是完整画布坐标");
    assert_eq!(
        (patch.width, patch.height),
        (121, 13),
        "bbox 必须盖住阴影下沿"
    );
    let alphas: Vec<u8> = patch.rgba.chunks_exact(4).map(|p| p[3]).collect();
    assert!(alphas.contains(&255), "字形本体应当是不透明");
    assert!(
        alphas.iter().any(|a| *a > 0 && *a < 255),
        "阴影的半透明 alpha 不能被二值化成 0 或 255（21 §5：量化必须保 alpha）"
    );
}

#[test]
fn render_output_flows_into_a_valid_pgs_stream() {
    let samples = vec![
        Sample::new(t(1000), rendered_line(0)),
        // 内容完全相同的一帧：author 必须把它消掉，而不是多产出一个显示集
        Sample::new(t(2000), rendered_line(0)),
        Sample::new(t(3000), rendered_line(40)),
        // 空白：必须是 erase display set
        Sample::new(t(4000), Frame::empty(C).unwrap()),
    ];
    let aopt = AuthorOptions {
        stream_end: t(6000),
    };
    let eopt = EncodeOptions {
        canvas_width: C.width as u16,
        canvas_height: C.height as u16,
        ..Default::default()
    };
    let want = ProfileWant {
        level: ValidationLevel::Basic,
        claim_compliant: false,
    };
    let a = pgs_author::author(&samples, &aopt, &eopt, &want, Some(Fps::PAL_25), None)
        .expect("author 整链路必须成功");

    assert_eq!(a.plan.stats.events, 3, "计划：{}", a.summary());
    assert_eq!(a.plan.stats.erase_events, 1);
    assert_eq!(a.plan.stats.eliminated, 1, "重复采样必须被消除");
    a.ensure_valid().expect("Basic 层不能有 error/fatal");

    let segs = parse_stream(&a.bytes).unwrap();
    let sets = group_display_sets(&segs).unwrap();
    assert_eq!(sets.len(), 3, "实际段流里的显示集数");
    assert!(sets[2].is_erase, "最后一帧是全透明，必须写成 erase");
    assert_eq!(sets[0].pts, 90_000, "1000ms -> 90kHz");
    assert_eq!(sets[1].pts, 270_000, "3000ms（2000ms 那帧被合并掉了）");

    let pcs = sets[0].pcs.as_ref().unwrap();
    assert_eq!(
        (pcs.width, pcs.height),
        (1280, 720),
        "PCS canvas 是完整画布"
    );
    let obj = &pcs.objects[0];
    assert_eq!(
        (obj.x, obj.y),
        (300, 600),
        "PCS placement 必须是合成时的完整画布坐标，不是相对 bbox（09 §3）"
    );
    // Basic Encoder 用一个覆盖整画布的窗口（对象坐标是画布坐标，13 §4）
    let w = &sets[0].windows[0];
    assert_eq!((w.x, w.y, w.width, w.height), (0, 0, 1280, 720));
    assert!(
        obj.x as u32 + 121 <= w.x as u32 + w.width as u32
            && obj.y as u32 + 13 <= w.y as u32 + w.height as u32,
        "对象（含局部位图尺寸）必须整个落在它引用的窗口内：obj=({}, {}) window=({}, {} {}x{})",
        obj.x,
        obj.y,
        w.x,
        w.y,
        w.width,
        w.height
    );

    // 局部位图的真实尺寸进的是 ODS 首片的 width/height（13 §4：tight bbox 只影响位图本身）
    let ods = a
        .segments
        .iter()
        .find(|s| s.seg_type == codec_pgs::segment::SEG_ODS)
        .expect("第一个显示集必须有 ODS");
    let hdr = codec_pgs::display_set::parse_ods_header(&ods.payload).unwrap();
    assert!(hdr.first, "单片对象也标 first");
    assert_eq!(
        hdr.size,
        Some((121, 13)),
        "ODS 尺寸应当等于裁剪后的局部位图"
    );
}

#[test]
fn repeated_render_output_does_not_bloat_the_stream() {
    // 25fps 采 50 帧但内容只有两次变化：显示集必须是 2 个，不是 50 个
    let mut samples = Vec::new();
    for i in 0..50 {
        let frame = if i < 25 {
            rendered_line(0)
        } else {
            rendered_line(40)
        };
        samples.push(Sample::new(t(1000 + i * 40), frame));
    }
    let aopt = AuthorOptions {
        stream_end: t(4000),
    };
    let eopt = EncodeOptions {
        canvas_width: C.width as u16,
        canvas_height: C.height as u16,
        ..Default::default()
    };
    let want = ProfileWant {
        level: ValidationLevel::Standard,
        claim_compliant: false,
    };
    let a = pgs_author::author(&samples, &aopt, &eopt, &want, Some(Fps::PAL_25), None).unwrap();
    assert_eq!(a.plan.stats.events, 2, "50 帧同内容必须塌成 2 个事件");
    assert_eq!(a.plan.stats.eliminated, 48);
    assert_eq!(a.encode_report.display_sets, 2);
    assert!(
        a.bytes.len() < 200_000,
        "两个显示集的 SUP 不该膨胀：{}",
        a.bytes.len()
    );
}

/// 真实 libass 的输出喂进同一条 author 链路（12 §1A 与 13 §4/§5 的闭环）。
///
/// 上面的合成 provider 证明的是"链路各环接口对得上"；这条证明的是"链路对上的是
/// **libass 真的会给出的那种数据**"。只有真渲染才能发现 alpha 方向搞反（ISSUES Z1），
/// 也只有把真渲染结果继续走到段流，才发现得了尺寸/坐标层面的不匹配。
/// 需要 native 依赖：`STK_LIBASS_DLL` → 本机 vcpkg 目录；默认 SKIP，
/// 门阶用 `STK_REQUIRE_LIBASS=1` 强制——"因为没跑所以通过"是我最不想见到的绿。
#[test]
fn real_libass_output_flows_into_a_valid_pgs_stream() {
    let Some(frame) = real_frame() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!(
                "STK_REQUIRE_LIBASS=1 但没有可用的 libass DLL（设 STK_LIBASS_DLL 指到 ass-9.dll）"
            );
        }
        eprintln!("SKIP 真实渲染闭环：本机没有 libass DLL");
        return;
    };
    let lit = frame.rgba.chunks_exact(4).filter(|p| p[3] != 0).count();
    assert!(lit > 100, "真实渲染的是字幕，不该只有 {lit} 个可见像素");

    let samples = vec![
        Sample::new(t(1000), frame),
        Sample::new(t(4000), Frame::empty(C).unwrap()),
    ];
    let aopt = AuthorOptions {
        stream_end: t(6000),
    };
    let eopt = codec_pgs::encoder::EncodeOptions {
        canvas_width: C.width as u16,
        canvas_height: C.height as u16,
        // 真渲染的抗锯齿颜色超过 255 条预算，Basic 档必然编不出来（ISSUES Z3），
        // 所以这条闭环只能站在 Advanced 的量化档上
        colors: codec_pgs::ColorPolicy::MedianCut { max_colors: 255 },
        ..Default::default()
    };
    let want = ProfileWant {
        level: ValidationLevel::Standard,
        claim_compliant: false,
    };
    let a = pgs_author::author(&samples, &aopt, &eopt, &want, Some(Fps::PAL_25), None)
        .expect("真实渲染结果必须能 author");
    a.ensure_valid().expect("Standard 层不能有 error/fatal");
    assert!(
        a.plan.loss.approximated.iter().any(|t| t.contains("量化")),
        "合并过颜色却没进 LossReport：{:?}",
        a.plan.loss.approximated
    );

    let segs = parse_stream(&a.bytes).unwrap();
    let sets = group_display_sets(&segs).unwrap();
    assert_eq!(sets.len(), 2, "一帧字幕 + 一次清除：{}", a.summary());
    assert!(sets[1].is_erase);

    let pcs = sets[0].pcs.as_ref().unwrap();
    assert_eq!((pcs.width, pcs.height), (1280, 720));
    let obj = &pcs.objects[0];
    // 对象尺寸不在 PCS 里，而在它自己的 ODS 首片（13 §4）
    let ods = a
        .segments
        .iter()
        .find(|s| s.seg_type == codec_pgs::segment::SEG_ODS)
        .expect("有内容的那个显示集必须有 ODS");
    let size = codec_pgs::display_set::parse_ods_header(&ods.payload)
        .unwrap()
        .size
        .expect("ODS 首片必须带宽高");
    assert!(size.0 > 0 && size.1 > 0, "对象必须带非零位图尺寸：{size:?}");
    assert!(
        u32::from(obj.x) + u32::from(size.0) <= 1280 && u32::from(obj.y) + u32::from(size.1) <= 720,
        "libass 的真实摆放必须整块落在画布内：obj=({}, {}) size={size:?} canvas=1280x720",
        obj.x,
        obj.y
    );
}

/// M10（13 §5）的负基线：**真渲染像素**过一遍量化，alpha 阶梯不许被压成二值。
///
/// 为什么这条要在 corpus 上跑、而不是只留 `quantize.rs` 里的合成用例：抗锯齿的边缘是 libass
/// 给的，合成 ramp 只能证明"代码符合我对它的假设"，真像素才证明那个假设对得上现实
/// （Z1 的 alpha 方向、AK1 的字体选择都是这样才被推翻的）。
/// 三件事分开断言：①前提（真渲染确实有多级半透明）；②合并必须被**度量**出来
/// （静默失真比失败更糟）；③量化后不透明度仍是多级 —— 压成"要么透明要么不透明"就是把抗锯齿抹掉。
#[test]
fn real_pixels_keep_their_alpha_ramp_through_quantization() {
    use codec_pgs::palette::ColorMatrix;
    use codec_pgs::quantize::{build_table, histogram, key_of, ColorPolicy};

    let Some(frame) = real_frame() else {
        if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
            panic!(
                "STK_REQUIRE_LIBASS=1 但没有可用的 libass DLL（设 STK_LIBASS_DLL 指到 ass-9.dll）"
            );
        }
        eprintln!("SKIP 量化负基线：本机没有 libass DLL");
        return;
    };
    let px: Vec<[u8; 4]> = frame
        .rgba
        .chunks_exact(4)
        .map(|c| [c[0], c[1], c[2], c[3]])
        .collect();
    let matrix = ColorMatrix::for_canvas(frame.canvas.height);
    // ①前提：这份渲染里有"既不全透明也不全不透明"的像素，而且不止两档
    let ramp: std::collections::BTreeSet<u8> = px
        .iter()
        .filter(|p| p[3] != 0 && p[3] != 255)
        .map(|p| p[3])
        .collect();
    assert!(
        ramp.len() > 2,
        "真渲染只给出 {} 级半透明，这条基线失去意义（渲染或字体变了，先看现场再说结论）",
        ramp.len()
    );

    let hist = histogram(std::iter::once(px.as_slice()), matrix);
    // Exact 档（Basic Encoder）在真渲染上必须**拒绝**，不是近似合并。
    // 我原本在这里断言"真字幕的颜色数应在 255 预算内" —— 那是一厢情愿：这份渲染有
    // 442 个可分辨颜色，正是 Z3 当年记下的现实（libass 的抗锯齿经常超过预算）。
    // 所以这条断言反过来更有价值：既证明 Basic 不偷偷降级，也证明 13 §5 的量化不是纸面理由。
    assert!(
        hist.len() > 255,
        "前提：这份真渲染要真的超过预算，实测 {} 色",
        hist.len()
    );
    let e = build_table(&hist, ColorPolicy::Exact).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0095", "{e:?}");

    // ②压到 8 条：一定发生合并，且误差被度量出来
    let q = build_table(&hist, ColorPolicy::MedianCut { max_colors: 8 }).unwrap();
    let stats = q.stats.expect("压到 8 条必然合并过；没报告就是静默失真");
    assert!(stats.merged > 0, "合并没发生：{stats:?}");
    assert!(stats.worst > 0, "合并了却报告无损：{stats:?}");
    assert!(
        q.entries.len() <= 9,
        "透明槽位 + 最多 8 条：实际 {}",
        q.entries.len()
    );

    // ③量化之后可见像素的不透明度仍是多级
    let after: std::collections::BTreeSet<u8> = px
        .iter()
        .map(|p| key_of(*p, matrix))
        .map(|k| q.entries[usize::from(q.index_of(k))].alpha)
        .filter(|a| *a != 0)
        .collect();
    assert!(
        after.len() > 2,
        "量化把 alpha 阶梯压成了 {:?}：字幕边缘的抗锯齿会被抹掉",
        after
    );
    // 阶梯"该保住多少级"我给不出依据，所以不编阈值；能讲的是**单调性**：
    // 预算放大，保住的阶梯不会变少。这条如果反过来，说明切分把 T 通道当成了次要维度。
    let levels = |budget: u8| -> usize {
        let t = build_table(&hist, ColorPolicy::MedianCut { max_colors: budget }).unwrap();
        px.iter()
            .map(|p| key_of(*p, matrix))
            .map(|k| t.entries[usize::from(t.index_of(k))].alpha)
            .filter(|a| *a != 0)
            .collect::<std::collections::BTreeSet<u8>>()
            .len()
    };
    let (l8, l32, l128) = (levels(8), levels(32), levels(128));
    assert!(
        l8 <= l32 && l32 <= l128,
        "预算放大反而保住更少阶梯：8→{} 32→{} 128→{}",
        l8,
        l32,
        l128
    );
    assert!(
        l128 > l8,
        "128 色预算没有比 8 色保住更多阶梯（都是 {}）：T 通道没有参与切分？",
        l128
    );
}

fn real_frame() -> Option<image_core::Frame> {
    let dll = match std::env::var("STK_LIBASS_DLL") {
        Ok(p) => std::path::PathBuf::from(p),
        // 开发期默认：仓库同级 tools/vcpkg/installed（28 §1：native 依赖锁版本，不入库）
        Err(_) => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("tools")
            .join("vcpkg")
            .join("installed")
            .join("x64-windows")
            .join("bin")
            .join("ass-9.dll"),
    };
    if !dll.is_file() {
        return None;
    }
    let provider = render_libass::Provider::open(&dll, C, C, "Arial")
        .unwrap_or_else(|e| panic!("打不开 {dll:?}：{e}｜细节 {e:?}"));
    let script = provider
        .load_script(REAL_ASS.as_bytes())
        .expect("脚本必须能载入");
    let r = render_libass::render(&script, t(2000), C).expect("渲染必须成功");
    assert!(r.images > 0, "1~4 秒的事件应当给出位图");
    Some(r.frame)
}

const REAL_ASS: &str = "[Script Info]\n\
ScriptType: v4.00+\n\
PlayResX: 1280\n\
PlayResY: 720\n\
ScaledBorderAndShadow: yes\n\
\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,72,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,20,20,40,1\n\
\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,Hello 世界\n\
";

// ── AA5：显示集起点的帧网格吸附 ─────────────────────────────────────────────
// 规格口径（09 §6、13 §2、validator 的 PGS.TML.05）：普通 SUP 允许非帧边界，
// 但那样就**拿不到严格兼容**那条声明。吸附因此是可选动作，且只在网格真的是整数 tick
// 时才允许 —— 23.976 的一帧是 3753.75 tick，取整吸附是在造一条假网格。

/// 起点都不在 25fps（3600 tick）网格上：90450 / 271350 / 361800。
fn off_grid_samples() -> Vec<Sample> {
    vec![
        Sample::new(t(1005), rendered_line(0)),
        Sample::new(t(2010), rendered_line(0)), // 同内容：并进上一个事件
        Sample::new(t(3015), rendered_line(40)), // 变了：新事件
        Sample::new(t(4020), Frame::empty(C).unwrap()), // 空白：erase 事件
    ]
}

fn plan_of(samples: &[Sample]) -> pgs_author::Plan {
    pgs_author::plan(
        samples,
        &AuthorOptions {
            stream_end: t(60_000),
        },
    )
    .expect("规划必须成功")
}

fn pts_of(plan: &pgs_author::Plan) -> Vec<i64> {
    plan.codec_frames()
        .expect("codec_frames 必须成功")
        .iter()
        .map(|f| f.pts)
        .collect()
}

#[test]
fn aa5_snapping_puts_every_display_set_on_the_frame_grid() {
    let mut plan = plan_of(&off_grid_samples());
    let before = pts_of(&plan);
    assert!(
        before.iter().any(|p| p % 3600 != 0),
        "夹具本身就该是 off-grid，否则这条测试什么都没验：{before:?}"
    );
    let moved = plan.snap_starts_to_grid(Fps::PAL_25).unwrap();
    assert!(moved >= 1, "一个起点都没动：{moved}");
    let after = pts_of(&plan);
    assert_eq!(after.len(), before.len(), "吸附不许改变显示集数量");
    for p in &after {
        assert_eq!(*p % 3600, 0, "PTS {p} 不在 25fps（3600 tick）网格上");
    }
}

#[test]
fn aa5_snapping_refuses_a_grid_that_is_not_whole_ticks() {
    // 23.976 的一帧 = 3753.75 tick。这里必须拒绝，而不是 `round` 一下继续吸。
    let mut plan = plan_of(&off_grid_samples());
    let e = plan.snap_starts_to_grid(Fps::NTSC_23976).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0119", "{e:?}");
    assert!(e.user_action.is_some(), "拒绝必须给出路：{e:?}");
    assert_eq!(
        pts_of(&plan),
        pts_of(&plan_of(&off_grid_samples())),
        "被拒绝的吸附不许顺手改动计划"
    );
}

#[test]
fn aa5_snapping_never_collapses_two_display_sets_onto_one_tick() {
    // 两个事件只差 45ms（< 一帧 40ms 的量级），floor 之后会同格。
    let samples = vec![
        Sample::new(t(1005), rendered_line(0)),
        Sample::new(t(1050), rendered_line(40)),
        Sample::new(t(3000), rendered_line(0)),
    ];
    let mut plan = plan_of(&samples);
    let n = plan.events.len();
    plan.snap_starts_to_grid(Fps::PAL_25).unwrap();
    assert_eq!(plan.events.len(), n, "吸附不许吞掉显示集");
    let pts = pts_of(&plan);
    for w in pts.windows(2) {
        assert!(
            w[1] > w[0],
            "PTS 不再严格递增（会被 STK-PGS-0090 拦）：{pts:?}"
        );
        assert_eq!(w[1] % 3600, 0, "{pts:?}");
    }
}

#[test]
fn aa5_snapping_is_opt_in_and_the_default_path_stays_off_grid() {
    // 反例式对照：把"吸过"与"没吸"放在一起比。原来草拟的是 plan==plan 那种恒真比较，
    // 正是 F4 批过的"永远为真的断言"。
    let plain = plan_of(&off_grid_samples());
    let mut snapped = plan_of(&off_grid_samples());
    snapped.snap_starts_to_grid(Fps::PAL_25).unwrap();
    let a = pts_of(&plain);
    let b = pts_of(&snapped);
    assert!(a != b, "吸了网格却没改变 PTS：{a:?} vs {b:?}");
    assert!(
        a.iter().any(|p| p % 3600 != 0),
        "默认路径应当仍是 off-grid：{a:?}"
    );
    assert!(b.iter().all(|p| p % 3600 == 0), "吸附后应当全部上格：{b:?}");
}

#[test]
fn aa5_a_snapped_origin_still_hits_the_decode_lead_floor() {
    // 网格原点是 0：起点在第一格内的事件会被吸到 PTS 0，那放不下 decode lead。
    // 这条测试钉的是"响亮地拒绝"，不是偷偷挪网格原点 —— 用户要能看见是谁的要求冲突了。
    let samples = vec![
        Sample::new(t(20), rendered_line(0)),
        Sample::new(t(3000), rendered_line(40)),
    ];
    let mut plan = plan_of(&samples);
    plan.snap_starts_to_grid(Fps::PAL_25).unwrap();
    let frames = plan.codec_frames().unwrap();
    assert_eq!(frames[0].pts, 0, "第一个显示集应被吸到网格原点");
    let e = codec_pgs::encoder::encode(&frames, &EncodeOptions::default()).unwrap_err();
    assert_eq!(e.code_string(), "STK-PGS-0098", "{e:?}");
}
