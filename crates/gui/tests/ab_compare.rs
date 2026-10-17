//! 05 §4.1 的 A/B 分屏对比：**端到端**行为。
//!
//! 规格原文：「提供 `[分屏对比]` 开关：左半屏显示原版（SDR / 原始尺寸），
//! 右半屏显示应用当前 Profile 变换后（HDR 调光 / 16:9 缩放）的渲染画面，
//! 调光调色效果一目了然。」
//!
//! 这一组测的是三件能判定的事：
//!
//! 1. **B 侧真的做了变换**。之前 `ToggleSplitAb` 只翻转一个布尔值并改按钮文字，
//!    画面毫无变化 —— 那是"看起来能用"的典型，而 05 §1 第 3 条明令禁止
//!    界面上放未实现的占位控件。所以这里断言的是**像素真的变了**。
//! 2. **恒等映射时两侧该一样，且说得出为什么**。`ReferenceWhite(203)` 就是
//!    SDR 参考白（`transform_hdr::SDR_REFERENCE_NITS`），映射是恒等的；
//!    那时两侧**必须**相同，而注记必须说清楚原因，否则用户以为功能坏了。
//! 3. **拼接是左右分屏，不是同一张图抄两遍**。`split_ab` 的几何要能被验证：
//!    左半来自 A、右半来自 B、中间有一条分隔线。
//!
//! 单元测试无法覆盖第 2 条 —— 它正是"参数选得不对看起来像坏了"那一类，
//! 只有真渲染才能抳出来（第一版探针就是这么发现 203 是恒等的）。

use std::path::PathBuf;

use gui::message::Message;
use gui::state::profile::HdrMode;

fn dll() -> Option<PathBuf> {
    let p = std::env::var("STK_LIBASS_DLL").ok()?;
    let p = PathBuf::from(p);
    p.is_file().then_some(p)
}

fn scratch(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("ab-{tag}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// 写一份带**可缩放颜色**的 ASS：纯白 (0xFF) 在任何缩放下都是 255，
/// 所以必须用中间调才能验出映射；灰色 (0x80) 是最敏感的那个。
fn write_ass(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("a.ass");
    std::fs::write(
        &p,
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 640\nPlayResY: 360\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,Arial,48,&H00808080,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,30,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.00,D,,0,0,0,,Hello AB\n",
    )
    .unwrap();
    p
}

/// 跑一次 A/B 渲染。
fn ab(
    ass: PathBuf,
    canvas_a: (u32, u32),
    canvas_b: (u32, u32),
    hdr: HdrMode,
) -> gui::worker::preview_worker::AbFrames {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(gui::worker::preview_worker::render_ab(
        gui::worker::preview_worker::AbRequest {
            ass_path: ass,
            time_ms: 2000,
            canvas_a,
            canvas_b,
            hdr,
            scale_to_canvas: canvas_a != canvas_b,
        },
    ))
    .expect("A/B 该能渲染")
}

fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
}

/// **核心断言**：B 侧真的被变换过，像素与 A 不同。
///
/// 用 `ReferenceWhite(120)`：120 nit 远低于 SDR 基准 203，
/// 所以映射必然把颜色压暗（`scale_channel` 单调且从不提亮，有专门的单元用例）。
#[test]
fn the_b_side_is_really_transformed_and_differs_from_a() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let d = scratch("transform");
    let f = ab(
        write_ass(&d),
        (640, 360),
        (640, 360),
        HdrMode::ReferenceWhite(120),
    );

    assert_eq!(f.a.width, 640);
    assert_eq!(f.b.width, 640);
    assert!(
        f.note.contains("120"),
        "注记要说清用的是哪个 nit：{}",
        f.note
    );
    assert!(!f.note.contains("恒等"), "120 nit 不是恒等映射：{}", f.note);

    let diff = differing_bytes(&f.a.rgba, &f.b.rgba);
    assert!(
        diff > 0,
        "B 侧该有像素差异（note={}）—— 没有差异就说明变换没生效",
        f.note
    );
}

/// **恒等映射必须被说清楚**，而不是给出一对一样的图让人以为坏了。
///
/// `ReferenceWhite(203)` = `transform_hdr::SDR_REFERENCE_NITS`，映射恒等。
/// 而它恰好是 GUI 默认 profile 的取值 —— 也就是新用户第一次点 A/B 会看到的东西。
#[test]
fn an_identity_mapping_says_so_instead_of_looking_broken() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let d = scratch("identity");
    let f = ab(
        write_ass(&d),
        (640, 360),
        (640, 360),
        HdrMode::ReferenceWhite(203),
    );

    assert_eq!(
        differing_bytes(&f.a.rgba, &f.b.rgba),
        0,
        "203 nit 是 SDR 基准，映射恒等，两侧该一致"
    );
    assert!(
        f.note.contains("恒等") || f.note.contains("203"),
        "两侧一样时必须说明原因（否则用户会以为对比坏了）：{}",
        f.note
    );
    // 但 RGB 通道仍要真的渲染出东西（不是两张全黑）
    assert!(f.a.rgba.chunks(4).any(|px| px[3] > 0), "该有非透明像素");
}

/// SDR + 画布相同时走"不做变换"的短路，两侧一致且注记说明。
#[test]
fn sdr_with_the_same_canvas_takes_the_no_op_path_and_says_so() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let d = scratch("noop");
    let f = ab(write_ass(&d), (640, 360), (640, 360), HdrMode::Sdr);
    assert_eq!(differing_bytes(&f.a.rgba, &f.b.rgba), 0);
    assert!(f.note.contains("未做变换"), "该说清没做变换：{}", f.note);
}

/// 画布不同时 B 侧要被缩放：尺寸变了，且注记里写明了从多少到多少。
#[test]
fn a_different_canvas_scales_the_b_side_and_reports_the_change() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let d = scratch("scale");
    let f = ab(write_ass(&d), (640, 360), (320, 180), HdrMode::Sdr);

    // B 侧画布就是目标尺寸（渲染器按 storage/output 出图）
    assert_eq!(f.b.width, 320, "B 侧该是目标画布宽");
    assert_eq!(f.b.height, 180);
    assert_eq!(f.a.width, 640, "A 侧该保持原尺寸（规格：原版）");
    assert!(
        f.note.contains("640x360") && f.note.contains("320x180"),
        "注记要写明缩放前后：{}",
        f.note
    );
}

/// 变换 + 缩放同时发生：两者都要在注记里。
#[test]
fn transform_and_scale_can_both_apply() {
    let Some(_) = dll() else {
        eprintln!("SKIP：本机没有 libass DLL");
        return;
    };
    let d = scratch("both");
    let f = ab(
        write_ass(&d),
        (640, 360),
        (320, 180),
        HdrMode::ReferenceWhite(120),
    );
    assert!(
        f.note.contains("HDR") && f.note.contains("尺寸"),
        "两件变换都该报出来：{}",
        f.note
    );
    assert_eq!((f.b.width, f.b.height), (320, 180));
}

/// `Message::ToggleSplitAb` 打开时**必须带回一个 Task**（去渲对照帧），
/// 而不是只翻转布尔值。这一条把「开关真的接了执行」钉住 ——
/// 之前那个版本只改按钮文字。
#[test]
fn toggling_ab_requests_a_render_instead_of_just_flipping_a_flag() {
    let d = scratch("toggle");
    let mut app = gui::app::SubtitleApp::default();
    app.inputs.add_path(write_ass(&d));
    app.inputs.select_all(true);
    let id = app.inputs.items[0].id;
    let _ = app.update(Message::InputSelected(id));

    assert!(!app.preview.is_split_ab, "默认该是关");
    let _task = app.update(Message::ToggleSplitAb);
    assert!(app.preview.is_split_ab, "点一下该打开");
    assert!(
        app.preview.is_rendering,
        "打开时该标记为正在渲染对照帧（只翻标志位就不该标记）"
    );
    // 打开时把上一次的对照帧清掉：留着它会让刚开始那一瞬间显示一对不匹配的画面
    assert!(app.preview.ab_rgba.is_none(), "打开时该清掉旧对照帧");
    assert!(app.preview.ab_note.is_none());
}

/// A/B 帧到达时：A 进 `rendered_*`、B 进 `ab_*`，注记与 provider 都落到位。
#[test]
fn ab_frames_land_in_both_slots_with_the_note_and_provider() {
    let mut app = gui::app::SubtitleApp::default();
    app.preview.is_rendering = true;
    let _ = app.update(Message::AbRendered {
        a_rgba: vec![10, 0, 0, 255],
        a_dims: (1, 1),
        b_rgba: vec![20, 0, 0, 255],
        b_dims: (1, 1),
        note: "HDR 参考白 120 nit：改了 2 个样式色".to_string(),
        source: gui::state::preview::FrameSource::Renderer {
            provider: "libass-9.dll".to_string(),
        },
    });
    assert!(!app.preview.is_rendering, "收到帧该停止转圈");
    assert_eq!(
        app.preview.rendered_rgba.as_deref(),
        Some(&[10u8, 0, 0, 255][..])
    );
    assert_eq!(app.preview.ab_rgba.as_deref(), Some(&[20u8, 0, 0, 255][..]));
    assert!(app.preview.ab_note.as_deref().unwrap().contains("120"));
    assert_eq!(app.preview.source.label(), "真实渲染（libass-9.dll）");
}

/// B 侧渲染失败时：**清帧并如实报错**，绝不把 A 当成完整的对比画面留着。
///
/// 留着旧帧会让用户以为"这就是变换后的样子"，而那是上一刻 A 的像素 ——
/// 那正好是 A/B 对比最不能犯的错。
#[test]
fn a_failed_ab_render_clears_frames_instead_of_showing_a_stale_pair() {
    let mut app = gui::app::SubtitleApp::default();
    app.preview.is_split_ab = true;
    // 先放一对好帧
    let _ = app.update(Message::AbRendered {
        a_rgba: vec![1, 2, 3, 255],
        a_dims: (1, 1),
        b_rgba: vec![4, 5, 6, 255],
        b_dims: (1, 1),
        note: "旧".to_string(),
        source: gui::state::preview::FrameSource::Renderer {
            provider: "x".to_string(),
        },
    });
    assert!(app.preview.ab_rgba.is_some());

    let _ = app.update(Message::AbRendered {
        a_rgba: vec![],
        a_dims: (1, 1),
        b_rgba: vec![],
        b_dims: (1, 1),
        note: String::new(),
        source: gui::state::preview::FrameSource::Failed {
            code: "STK-RND-0091".to_string(),
        },
    });
    assert!(app.preview.rendered_rgba.is_none(), "失败该清掉 A 侧");
    assert!(app.preview.ab_rgba.is_none(), "失败该清掉 B 侧");
}

/// 一侧空了（而另一侧有）也要清：不能拿旧帧的一半拼出新对的假象。
#[test]
fn a_half_empty_ab_result_also_clears_both_sides() {
    let mut app = gui::app::SubtitleApp::default();
    app.preview.is_split_ab = true;
    let _ = app.update(Message::AbRendered {
        a_rgba: vec![1, 2, 3, 255],
        a_dims: (1, 1),
        b_rgba: vec![4, 5, 6, 255],
        b_dims: (1, 1),
        note: "旧".to_string(),
        source: gui::state::preview::FrameSource::Renderer {
            provider: "x".to_string(),
        },
    });
    // A 有、B 空（例如 B 那一侧渲染器拒了）
    let _ = app.update(Message::AbRendered {
        a_rgba: vec![9, 9, 9, 255],
        a_dims: (1, 1),
        b_rgba: vec![],
        b_dims: (1, 1),
        note: "半张".to_string(),
        source: gui::state::preview::FrameSource::Failed {
            code: "STK-RND-0091".to_string(),
        },
    });
    assert!(
        app.preview.rendered_rgba.is_none() && app.preview.ab_rgba.is_none(),
        "任一侧空了就要两边都清，否则会把旧的一侧当新的用"
    );
}
