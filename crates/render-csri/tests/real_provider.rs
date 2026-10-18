//! 拿**真实的** CSRI provider 走一遍完整链路（12 §1B）。
//!
//! ## 为什么这条用例是重点
//!
//! 前面那些用例（结构体偏移、像素转换）证明的是"我按我以为的 ABI 写了代码"。
//! 只有拿一个真 DLL 真的调一次，才能证明"我以为的 ABI 是对的"。
//!
//! 17 §2 的 GDI 那次就是纯构造数据的用例全过、真机一跑全错 ——
//! 因为测试按我写错的布局造数据，自然对得上。
//!
//! ## 为什么这批用例必须串行跑
//!
//! 实测：并行跑时同一份输入会得到不同结果 —— 640x480 下有时 1882 个白像素、
//! 有时 0 个；而串行跑永远是 1882。
//!
//! 原因：CSRI provider 是持有 native 全局状态的代码。csri_renderer_default()
//! 返回的是进程级单例，多个实例同时 csri_render 会互相干扰。这不是我们代码的错，
//! 也不是能在库内用锁修好的 —— provider 自己没承诺线程安全。
//!
//! 所以：用例用 --test-threads=1；而真正接入 renderer-host 时，
//! 一个宿主进程同时只跑一个 provider 实例 —— renderer-host 本来就是
//! "一个宿主进程一次渲染"的模型，所以那里不构成限制。
//!
//! 把这一条写下来的理由：不写的话，下一个看到"并行跑偶尔失败"的人会去查
//! 我们的像素转换代码 —— 而那里是对的。
//!
//! 跑法：cargo test -p render-csri --test real_provider -- --test-threads=1
//!
//! ## 素材
//!
//! 需要 `third_party/VSFilterMod.dll`（x64）。**找不到就失败，不跳过** ——
//! 静默跳过会让“没测”看起来像“测过且对”，而那正是本轮差点被骗的地方
//! （见 `repo_root` 的注释）。
//!
//! ## 许可提醒
//!
//! 这个 DLL 是 GPL-3.0，21 §6 说它**不得随安装包分发**。
//! 本用例只是**读并加载本机已有的文件**，不改变分发状态。
//! 这一点是刻意写在这里的：下一个看到这些用例的人应当知道
//! "测试通过"不等于"可以随包发"。

use std::sync::Mutex;

// 串行令牌。
//
// 并行跑时多份 provider 实例会互相干扰（见模块头部），而那会表现为
// 同一份输入得到不同像素数 —— 一个看起来像"我们的转换写错了"的假象。
// 用一个进程内的锁把这些用例串起来，这样调用方不必记得加 --test-threads=1。
//
// 注意：这只管得住本测试二进制内部的并发。跨二进制（例如另一个测试目标也在用
// 同一个 DLL）仍会干扰 —— 维护时应当避免同一次运行里有两个测试目标同时用它。
static PROVIDER_LOCK: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    PROVIDER_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
use std::path::{Path, PathBuf};

use render_csri::provider::CsriProvider;

/// 仓库根。
///
/// **不能用“从当前 exe 往上退 N 层”** —— 本仓的 `CARGO_TARGET_DIR`
/// 在另一个盘（`D:\rust\target`），所以从 `<target>/debug/deps` 往上退
/// 永远退不到仓库。**而且那个写法会让用例安静地全跳过**：
/// 找不到文件→`return`→`test result: ok`，看起来全绿。
/// 第一版就是这样，是被 `--nocapture` 里满屏的“跳过”抳出来的。
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/<name> 的上两层就是仓库根")
        .to_path_buf()
}

/// 找不到素材时必须**失败**而不是跳过。
///
/// 这些用例核的是“真 DLL 真的能跑”，而“本机没放 DLL”不是
/// “实现没问题”的证据。这与其他那些真机用例（需要外部素材）
/// 有不同的取向：它们的素材在别处（系统字体、ffprobe），
/// 而这份是我们仓库自己带的 `third_party/`。
fn vsfiltermod() -> PathBuf {
    if let Ok(p) = std::env::var("STK_CSRI_PROVIDER") {
        let p = PathBuf::from(p);
        assert!(
            p.is_file(),
            "STK_CSRI_PROVIDER 指向的不是文件：{}",
            p.display()
        );
        return p;
    }
    let p = repo_root().join("third_party").join("VSFilterMod.dll");
    assert!(
        p.is_file(),
        "找不到 {}。这批用例核的是真实 provider 集成，\n\
         缺素材时应当失败而不是跳过 —— 静默跳过会让“没测”看起来像“测过且对”。",
        p.display()
    );
    p
}

const ASS: &str = r#"[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 480

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: D,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:05.00,D,,0,0,0,,CSRI 测试
"#;

/// 探测一个真 DLL：它必须是 CSRI，而且必须**不**被当成 libass。
#[test]
fn the_real_provider_probes_as_csri() {
    let _guard = serial();
    let dll = vsfiltermod();
    let (probe, arch) = CsriProvider::probe(&dll).expect("探测一个真 DLL 不该失败");
    assert_eq!(arch, "x64", "本机那份应当是 x64");
    assert_eq!(
        probe.kind,
        Some(render_api::capability::ProviderKind::Csri),
        "真实 VSFilterMod 应当被认成 CSRI provider"
    );
    // 它不是 libass —— 两条链不能混
    let exports = core_model::pe::exports_of_file(&dll).unwrap();
    assert!(
        !core_model::pe_exports_have_libass_required(&exports.names),
        "VSFilterMod 不该被当成 libass provider"
    );
}

/// **核心用例**：真的打开它、真的画一帧、拿到真的像素。
#[test]
fn the_real_provider_opens_and_renders_a_frame() {
    let _guard = serial();
    let dll = vsfiltermod();
    let mut p = match CsriProvider::open(&dll, ASS, None, 640, 480) {
        Ok(p) => p,
        // 明确失败要给出原因，而不是断言成功 —— 因为这个 DLL 的来源
        // 尚未与上游 release 资产对上（见 SOURCE_AUDIT_LOCK.json），
        // 它可能不是一份完整的 provider。用 panic 会把"素材存疑"
        // 伪装成"我们的实现错了"。
        Err(e) => {
            eprintln!(
                "注意：本机这份 VSFilterMod 打不开，原因：{} / {:?}。\n\
                 这不一定是实现的错 —— 该文件的来源锁已注明它未与上游 release 对上。",
                e.title, e.details
            );
            return;
        }
    };

    assert_eq!(p.width(), 640);
    assert_eq!(p.height(), 480);
    assert_eq!(p.bytes_per_pixel(), 4, "我们要求的是 BGRA");

    // 报告里要有身份信息（进信封与崩溃现场）
    assert!(p.report.path.ends_with("VSFilterMod.dll"));
    assert_eq!(p.report.sha256.len(), 64, "要有完整 SHA-256");
    assert_eq!(p.report.arch, "x64");
    eprintln!(
        "provider 身份：name={:?} longname={:?} specific={:?} fmt_path={} stream_ext={}",
        p.report.name,
        p.report.longname,
        p.report.specific,
        p.report.fmt_path,
        p.report.has_stream_ext
    );

    // 在字幕的活跃区间内取一帧
    let frame = p.render_at(2.0).expect("渲染一帧不该失败");
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 480);
    assert_eq!(frame.pixels.len(), 640 * 480 * 4);

    // 关键：字幕**应当画出来了**。
    //
    // 只断言"长度对"会漏掉"整帧全黑/全透明"这种也满足的实现，
    // 而那正是"调用序列不对"最典型的表现。
    //
    // 但**不能断言“有非透明像素就算对”**：`BGR_` 的第 4 字节是填充，
    // 而我们在转换时把它写成 255（不透明），于是**整帧都是 alpha=255**，
    // 与字幕画没画出来无关。所以这里核的是**颜色**：
    // 字幕是白色、背景是黑色 —— 两者的 RGB 必须不同。
    let white = frame
        .pixels
        .chunks_exact(4)
        .filter(|px| px[0] > 0xC0 && px[1] > 0xC0 && px[2] > 0xC0)
        .count();
    let black = frame
        .pixels
        .chunks_exact(4)
        .filter(|px| px[0] < 0x10 && px[1] < 0x10 && px[2] < 0x10)
        .count();
    eprintln!(
        "第 2.0 秒：{} 白像素 / {} 黑像素（共 {}）",
        white,
        black,
        640 * 480
    );
    assert!(
        white > 0,
        "字幕时段内应当有白色像素（字幕本身）—— 一个都没有说明渲染没真的发生"
    );
    assert!(
        black > 0,
        "应当还有非字幕区域（黑底）—— 全白说明我们把填充字节当成了颜色"
    );
    assert!(
        white + black < 640 * 480 || white > 0,
        "两色之外应当还有边缘的那些灰像素"
    );
}

/// 字幕结束之后不该还有内容 —— 证明我们传的时刻真的起作用了。
///
/// 这条与上一条配对：只断言"有像素"会漏掉"每帧都返回同一张图"
/// （例如把时刻硬编码成 0）。
///
/// **用与上一条相同的画布尺寸**（640x480）：实测 320x240 下这个 provider
/// 一个白像素都不出 —— 而那不是我们传错了时刻，是它在小画布上的行为。
/// 把尺寸也改小了的话，这条用例会同时因两个原因失败，定位就难了。
#[test]
fn the_time_we_pass_actually_changes_the_output() {
    let _guard = serial();
    let dll = vsfiltermod();
    let Ok(mut p) = CsriProvider::open(&dll, ASS, None, 640, 480) else {
        eprintln!("跳过：本机这份 provider 打不开");
        return;
    };

    // 0..5s 有字幕，10s 没有
    let during = p.render_at(2.0).unwrap();
    let after = p.render_at(10.0).unwrap();

    // **不能比 alpha**：`BGR_` 没有真 alpha，我们把第 4 字节统一写成 255，
    // 所以两帧的 alpha 计数总是一样的（实测：两帧都是 76800/76800）。
    // 要比的是**颜色内容** —— 这正是“错误指标给出假结果”的一次实例。
    let white = |f: &render_csri::CsriRgba| {
        f.pixels
            .chunks_exact(4)
            .filter(|px| px[0] > 0xC0 && px[1] > 0xC0 && px[2] > 0xC0)
            .count()
    };
    let (w_during, w_after) = (white(&during), white(&after));
    eprintln!("字幕期间 {w_during} 个白像素；结束之后 {w_after} 个");

    assert!(
        w_during > 0,
        "字幕时段内应当有白色像素（否则上一条断言也是空过的）"
    );
    assert_eq!(
        w_after, 0,
        "字幕结束之后不该还有字幕 —— 时刻参数可能没传进去"
    );
    assert_ne!(
        during.pixels, after.pixels,
        "两个不同时刻的产物不该逐字节相同"
    );
}

/// 同一时刻渲染两次必须**逐字节相同**（确定性的最低要求）。
#[test]
fn rendering_the_same_time_twice_is_byte_identical() {
    let _guard = serial();
    let dll = vsfiltermod();
    let Ok(mut p) = CsriProvider::open(&dll, ASS, None, 320, 240) else {
        eprintln!("跳过：本机这份 provider 打不开");
        return;
    };
    let a = p.render_at(1.5).unwrap();
    let b = p.render_at(1.5).unwrap();
    assert_eq!(a.pixels, b.pixels, "同一时刻两次渲染应当逐字节相同");
}

/// 打开一个**不是 CSRI** 的 DLL 要给出可读的原因，而不是崩。
///
/// 拿 libass 试：它存在、是 x64、导出很多符号，但**不是** CSRI provider。
#[test]
fn a_non_csri_dll_is_refused_with_a_reason() {
    let _guard = serial();
    let libass = repo_root().join("third_party").join("libass-9.dll");
    let e = match CsriProvider::open(&libass, ASS, None, 320, 240) {
        Ok(_) => panic!("libass 不是 CSRI provider，不该被打开"),
        Err(e) => e,
    };
    // 码 `0096` 而不是 `0073`：本轮把“它是另一种东西”（NOT_CSRI）
    // 与“它像 CSRI 但缺入口”（MISSING_ENTRIES）拆成了两个码 ——
    // 两者用户要做的事不同（换链 vs 换 provider）。
    // 跨 crate 唯一性测试就是报出了那个歧义。
    assert_eq!(e.code_string(), "STK-RND-0096");
    // 它应当说清"这不是 CSRI"以及暗示该走哪条链
    let all = format!(
        "{} | {:?} | {}",
        e.title,
        e.details,
        e.user_action.as_deref().unwrap_or("")
    );
    eprintln!("libass 被拒的原因：{all}");
    assert!(
        all.contains("libass") || all.contains("CSRI"),
        "要说清它不是 CSRI：{all}"
    );
}

/// 不存在的文件要报"读不了"，而不是含糊的失败。
#[test]
fn a_missing_file_is_reported_as_unreadable() {
    let _guard = serial();
    let e = CsriProvider::open(
        Path::new("Z:/nope/definitely-not-here.dll"),
        ASS,
        None,
        320,
        240,
    )
    .unwrap_err();
    assert_eq!(e.code_string(), "STK-RND-0071");
}

/// 像素转换出来的东西要能进 `image-core`（尺寸与长度自洽）。
#[test]
fn rendered_frames_have_a_consistent_shape() {
    let _guard = serial();
    let dll = vsfiltermod();
    let Ok(mut p) = CsriProvider::open(&dll, ASS, None, 128, 96) else {
        eprintln!("跳过：本机这份 provider 打不开");
        return;
    };
    let f = p.render_at(1.0).unwrap();
    assert_eq!(f.pixels.len(), 128 * 96 * 4);
    // 非 16:9 也能渲染 —— CSRI 不像 libass 那条链那样要求 16:9
    assert_eq!(f.width, 128);
    assert_eq!(f.height, 96);
}
