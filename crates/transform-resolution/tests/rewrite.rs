//! 09 §9 的验收硬规则在"真实感脚本"上的集成测试：
//! Preserve 必须逐字节不变；Rewrite 只改该改的，其余字节（BOM、混合换行、附件段、
//! 未知 tag、karaoke 毫秒）必须原样。

use codec_ass::AssDocument;
use core_model::canvas::Canvas;
use core_model::geometry::{
    Geometry, LayoutRes, LayoutResPolicy, ScriptResolutionPolicy, ScriptSize,
};
use core_model::loss::LossReport;
use core_model::time::Rational;
use transform_resolution::{apply, effective_sbs, resolve_script};

/// 一个"脏"脚本：BOM + CRLF/LF 混用 + 最后一行无换行 + 未知 VSFilterMod tag
/// + LayoutRes + drawing + karaoke + Comment + [Fonts] 附件 + 未知 Script Info key。
const MESSY: &str = concat!(
    "\u{feff}[Script Info]\r\n",
    "Title: 测试/🎬\r\n",
    "ScriptType: v4.00+\r\n",
    "PlayResX: 1920\r\n",
    "PlayResY: 1080\r\n",
    "LayoutResX: 1920\r\n",
    "LayoutResY: 1080\r\n",
    "X-Unknown-Key: keep me\n",
    "ScaledBorderAndShadow: yes\r\n",
    "\r\n",
    "[V4+ Styles]\r\n",
    "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\r\n",
    "Style: Default,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,1,0,1,3,2,2,60,60,50,1\r\n",
    "\r\n",
    "[Events]\r\n",
    "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\n",
    "Comment: 0,0:00:01.00,0:00:02.00,Default,,10,10,10,,{\\pos(100,100)}注释不动\r\n",
    "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,20,20,20,,",
    "{\\fscx120\\fscy120\\pos(960,540)\\org(900,500)\\move(10,20,30,40,0,500,1000)",
    "\\bord2\\shad3\\blur8\\be1\\frz45\\fsp2\\fs36}{\\vsfmod-private 7}",
    "{\\p1}m 0 0 l 100 50{\\p0}{\\k50}你{\\kf100}好\n",
    "Dialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\clip(\\p2 m 10 20 l 30 40\\p0)x}",
);

fn rewrite(canvas: Canvas, script: (u32, u32)) -> Geometry {
    Geometry {
        canvas,
        script: Some(ScriptSize {
            x: script.0,
            y: script.1,
        }),
        storage: None,
        script_policy: ScriptResolutionPolicy::Rewrite,
        ..Geometry::default()
    }
}

#[test]
fn preserve_is_byte_identical_on_a_dirty_script() {
    let mut doc = AssDocument::parse(MESSY);
    let before = doc.render();
    let mut loss = LossReport::new();
    let a = apply(
        &mut doc,
        &Geometry {
            canvas: Canvas::UHD2160,
            script: Some(ScriptSize { x: 1920, y: 1080 }),
            script_policy: ScriptResolutionPolicy::Preserve,
            ..Geometry::default()
        },
        &mut loss,
    )
    .unwrap();
    assert_eq!(a.total(), 0);
    assert_eq!(doc.render(), before, "Preserve 改了一个字节");
    assert_eq!(doc.render(), MESSY);
}

#[test]
fn rewrite_twice_only_touches_the_script_coordinate_space() {
    let mut doc = AssDocument::parse(MESSY);
    let mut loss = LossReport::new();
    let a = apply(&mut doc, &rewrite(Canvas::UHD2160, (1920, 1080)), &mut loss).unwrap();
    let out = doc.render();

    // BOM / 换行风格 / 未知 key / 注释 / 附件语义都还在
    assert!(out.starts_with('\u{feff}'));
    assert!(
        out.contains("X-Unknown-Key: keep me\n"),
        "未知 Script Info key 丢失"
    );
    assert!(out.contains("Title: 测试/🎬\r\n"));
    assert!(out.ends_with("x}"), "最后一行无换行的写法必须保持");
    assert!(out
        .contains("Comment: 0,0:00:01.00,0:00:02.00,Default,,10,10,10,,{\\pos(100,100)}注释不动"));

    // 脚本坐标空间
    // AT-CANVAS-001：1080→2160 保持完整 16:9（PlayRes 缩放），且关键定位 ×2。
    // 定义与阈值见 docs/specs/25_ACCEPTANCE_TESTS.md。
    assert!(out.contains("PlayResX: 3840\r\nPlayResY: 2160\r\n"));
    assert!(out.contains("Style: Default,Arial,96,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,2,0,1,6,4,2,120,120,100,1"));
    assert!(out.contains("Default,,40,40,40,,"), "逐行边距要跟着缩放");
    assert!(out.contains("\\pos(1920,1080)\\org(1800,1000)"));
    assert!(
        out.contains("\\move(20,40,60,80,0,500,1000)"),
        "毫秒不能被缩放"
    );
    assert!(
        out.contains("{\\p1}m 0 0 l 200 100{\\p0}"),
        "drawing 坐标缩放、\\p 不动"
    );
    assert!(out.contains("{\\clip(\\p2 m 20 40 l 60 80\\p0)x}"));
    assert!(out.contains("{\\vsfmod-private 7}"), "未知 tag 必须原样");
    assert!(out.contains("{\\k50}你{\\kf100}好"), "karaoke 毫秒不动");

    // 与分辨率无关 / libass 里非 screen_scale 的量
    assert!(out.contains("\\fscx120\\fscy120"));
    assert!(out.contains("\\frz45"));
    assert!(out.contains("\\blur8\\be1"));
    assert!(
        out.contains("\\bord4\\shad6"),
        "SBS: yes -> 边框阴影属脚本空间"
    );
    assert!(out.contains("\\fsp4\\fs72"));

    // LayoutRes 默认 KeepAsIs
    assert!(out.contains("LayoutResX: 1920\r\nLayoutResY: 1080\r\n"));
    assert_eq!(a.layout_scaled, 0);
    assert_eq!(a.layout_dropped, 0);
    assert_eq!(a.comments_skipped, 1);
    assert_eq!(a.unknown_tags, 1);
    assert_eq!(a.blur_kept, 2);
    assert!(a.total() > 12);
    assert_eq!(effective_sbs(&doc), Some(true));
}

#[test]
fn rewritten_document_still_roundtrips_and_reparses_cleanly() {
    let mut doc = AssDocument::parse(MESSY);
    let mut loss = LossReport::new();
    apply(&mut doc, &rewrite(Canvas::UHD2160, (1920, 1080)), &mut loss).unwrap();
    let text = doc.render();
    assert_eq!(
        AssDocument::parse(&text).render(),
        text,
        "重写后不再字节保真"
    );
    // 再按新脚本尺寸 Rewrite 到同一画布：必须是零改动
    let mut again = AssDocument::parse(&text);
    let mut loss2 = LossReport::new();
    let a2 = apply(
        &mut again,
        &rewrite(Canvas::UHD2160, (3840, 2160)),
        &mut loss2,
    )
    .unwrap();
    assert_eq!(a2.total(), 0, "同尺寸重写应当无改动");
    assert_eq!(again.render(), text);
}

#[test]
fn three_halves_factor_keeps_exact_decimals() {
    // 640x360 -> 960x540（系数 1.5）：坐标允许 .5，PlayRes 取整
    let src = "[Script Info]\r\nPlayResX: 640\r\nPlayResY: 360\r\nScaledBorderAndShadow: yes\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\pos(31,17)}x\r\n";
    let mut doc = AssDocument::parse(src);
    let mut loss = LossReport::new();
    apply(
        &mut doc,
        &rewrite(
            Canvas {
                width: 960,
                height: 540,
            },
            (640, 360),
        ),
        &mut loss,
    )
    .unwrap();
    let out = doc.render();
    assert!(out.contains("\\pos(46.5,25.5)"), "{out}");
    assert!(out.contains("PlayResX: 960\r\nPlayResY: 540\r\n"));
    assert!(loss.approximated.is_empty(), "PlayRes 与坐标都精确可表达");
}

#[test]
fn drop_policy_removes_only_layout_lines_and_scales_style_space() {
    let mut doc = AssDocument::parse(MESSY);
    let g = Geometry {
        layout: Some(LayoutRes { x: 1920, y: 1080 }),
        layout_policy: LayoutResPolicy::Drop,
        ..rewrite(Canvas::UHD2160, (1920, 1080))
    };
    let mut loss = LossReport::new();
    let a = apply(&mut doc, &g, &mut loss).unwrap();
    let out = doc.render();
    assert_eq!(a.layout_dropped, 2);
    assert!(!out.contains("LayoutRes"));
    assert!(out.contains("PlayResX: 3840"));
    assert!(out.contains("X-Unknown-Key: keep me\n"));
    assert!(out.contains("ScaledBorderAndShadow: yes"));
}

#[test]
fn unknown_script_playres_is_adopted_before_rewriting() {
    // 任务引擎的正确顺序：先用文件的 PlayRes 填几何，再重写
    let doc = AssDocument::parse(MESSY);
    let g = resolve_script(
        &Geometry {
            canvas: Canvas::UHD2160,
            script: None,
            script_policy: ScriptResolutionPolicy::Rewrite,
            ..Geometry::default()
        },
        &doc,
    );
    assert_eq!(g.script_scale_factor().unwrap(), Rational::from_int(2));
    let mut doc = AssDocument::parse(MESSY);
    let mut loss = LossReport::new();
    let a = apply(&mut doc, &g, &mut loss).unwrap();
    assert!(a.total() > 12);
}
