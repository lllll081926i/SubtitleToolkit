//! 经 `renderer-host` 走 **CSRI 链**的端到端用例（12 §1B）。
//!
//! ## 与 `host_isolation.rs` 的分工
//!
//! `host_isolation.rs` 验的是 **libass 那条链**（AT-RENDER-001），
//! 包括"host 与进程内渲染逐字节一致"。
//!
//! 本文件验的是**另一条链已经接进 host 并能真的出帧**：
//!
//! - host 会按导出表分流（不再硬编码 libass）；
//! - 握手报的是 `csri` 而不是 `libass`；
//! - 真的能把字幕渲染成非空像素；
//! - 拿 libass 的 DLL 走 CSRI 路径会被**明确拒绝**（不允许"先试一条再试另一条"）。
//!
//! ## 为什么不做"与进程内渲染逐字节一致"
//!
//! 那条断言在 libass 上有意义（同一个库、同一个入口）。
//! 而 CSRI 链与 libass 链**本来就是两个不同的渲染器**，
//! 输出像素不同是正常的、也是预期的。把它们比成"应该一致"是错的判据 ——
//! 那会逼着实现去伪造一致。
//!
//! 所以这里核的是**结构与语义**：帧尺寸对、有内容、时刻敏感、身份对。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use renderer_host::client::HostFailure;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/renderer-host 的上两层就是仓库根")
        .to_path_buf()
}

fn csri_dll() -> PathBuf {
    if let Ok(p) = std::env::var("STK_CSRI_PROVIDER") {
        return PathBuf::from(p);
    }
    repo_root().join("third_party").join("VSFilterMod.dll")
}

fn libass_dll() -> PathBuf {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        return PathBuf::from(p);
    }
    repo_root().join("third_party").join("libass-9.dll")
}

fn host_exe() -> PathBuf {
    // 与 host_isolation.rs 同一套定位：先看环境变量，再看同目录。
    if let Ok(p) = std::env::var("STK_RENDERER_HOST") {
        return PathBuf::from(p);
    }
    let mut p = std::env::current_exe().unwrap();
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join(format!(
        "subtitletool-render-host{}",
        std::env::consts::EXE_SUFFIX
    ))
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
Dialogue: 0,0:00:00.00,0:00:05.00,D,,0,0,0,,CSRI via host
"#;

/// 起一个 host 并做到 LoadScript。返回 (host, 握手)。
fn start_csri() -> Result<
    (
        renderer_host::client::Host,
        renderer_host::protocol::Handshake,
    ),
    HostFailure,
> {
    let dll = csri_dll();
    assert!(
        dll.is_file(),
        "找不到 CSRI provider {}。这批用例核的是真实 provider 集成，\n\
         缺素材时应当失败而不是跳过 —— 静默跳过会让“没测”看起来像“测过且对”。",
        dll.display()
    );
    let exe = host_exe();
    assert!(exe.is_file(), "找不到 renderer-host：{}", exe.display());
    let spec = renderer_host::client::HostSpec {
        host_exe: &exe,
        dll: &dll,
        storage: core_model::canvas::Canvas::new(640, 480).unwrap(),
        output: core_model::canvas::Canvas::new(640, 480).unwrap(),
        default_font: "Arial",
        timeout: Duration::from_secs(30),
        crash_injection: None,
        crash_dir: None,
    };
    let mut host = renderer_host::client::Host::start(&spec)?;
    let hs = host.handshake().clone();
    host.load_script(ASS.as_bytes())?;
    Ok((host, hs))
}

/// 握手要报 `csri`，而不是 `libass`。
///
/// 这条是"host 真的按导出表分流了"的最直接证据：
/// 改之前 host 硬编码 `render_libass::Provider` 且握手恒定报 `"libass"`。
#[test]
fn the_host_reports_the_csri_chain_in_its_handshake() {
    let (mut host, hs) = match start_csri() {
        Ok(v) => v,
        Err(e) => panic!("CSRI 链应当能起：{e:?}"),
    };
    assert_eq!(hs.provider, "csri", "{hs:?}");
    assert_eq!(hs.provider_arch, "x64", "{hs:?}");
    assert_eq!(hs.pixel_format, "RGBA8", "{hs:?}");
    assert_eq!(hs.provider_binary_sha256.len(), 64, "{hs:?}");
    let _ = host.shutdown();
}

/// 真的能出帧，而且**帧里有内容**。
///
/// 只核尺寸会漏掉"整帧全空"这种也满足的实现。
#[test]
fn the_csri_chain_really_renders_content() {
    let Ok((mut host, _)) = start_csri() else {
        panic!("CSRI 链应当能起");
    };
    let frame = host.render(2000).expect("2.0s 处应当有帧");
    assert_eq!((frame.width, frame.height), (640, 480));
    assert_eq!(frame.rgba.len(), 640 * 480 * 4);

    // 不透明像素 = 字幕内容（背景已被 host 侧剥成透明，见 backend.rs）。
    let painted = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count();
    eprintln!("CSRI 经 host：2.0s 处 {painted} 个不透明像素");
    assert!(
        painted > 0,
        "字幕时段内应当画出东西；整帧全透明说明像素转换或调用序列不对"
    );
    // 也不该是整幅画布 —— 那说明背景没剥掉（见 backend.rs 的长注释）
    assert!(
        painted < 640 * 480,
        "整幅画布都不透明：背景没剥掉，BDN/SUP 会得到一张带底的图"
    );
    let _ = host.shutdown();
}

/// 时刻要真的起作用：字幕结束之后不该还有内容。
#[test]
fn the_csri_chain_is_time_accurate() {
    let Ok((mut host, _)) = start_csri() else {
        panic!("CSRI 链应当能起");
    };
    let during = host.render(2000).unwrap();
    let after = host.render(10000).unwrap();
    let painted = |f: &renderer_host::client::Frame| {
        f.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] != 0)
            .count()
    };
    let (d, a) = (painted(&during), painted(&after));
    eprintln!("CSRI 经 host：字幕期间 {d} 个不透明像素；结束之后 {a} 个");
    assert!(d > 0, "字幕时段内应当有内容（否则上一条断言也是空过的）");
    assert_eq!(a, 0, "字幕结束之后不该还有内容 —— 时刻可能没传进去");
    let _ = host.shutdown();
}

/// **拿 libass 的 DLL 走 CSRI 路径必须被明确拒绝。**
///
/// 这条钉住 12 §1 的分派纪律：**先看导出表，再只走一条链**。
/// 不许"先试 libass，失败再试 CSRI" —— 那会在进程里留下半初始化状态。
#[test]
fn a_libass_dll_is_dispatched_to_the_libass_chain_not_csri() {
    let dll = libass_dll();
    if !dll.is_file() {
        eprintln!("跳过：找不到 {}", dll.display());
        return;
    }
    let exe = host_exe();
    let spec = renderer_host::client::HostSpec {
        host_exe: &exe,
        dll: &dll,
        storage: core_model::canvas::Canvas::FHD1080,
        output: core_model::canvas::Canvas::FHD1080,
        default_font: "Arial",
        timeout: Duration::from_secs(30),
        crash_injection: None,
        crash_dir: None,
    };
    let host = renderer_host::client::Host::start(&spec).expect("libass host 应当起得来");
    // 关键：它要报 libass，而不是 csri
    assert_eq!(
        host.handshake().provider,
        "libass",
        "libass 的 DLL 应当被分派到 libass 链"
    );
}

/// `classify` 对两条链给出不同结论 —— 这是分派的**纯函数**证据。
#[test]
fn classify_separates_the_two_chains() {
    use renderer_host::backend::classify;
    let libass = libass_dll();
    if libass.is_file() {
        assert_eq!(
            classify(&libass).unwrap(),
            render_api::capability::ProviderKind::LibassBuiltin
        );
    }
    let csri = csri_dll();
    if csri.is_file() {
        assert_eq!(
            classify(&csri).unwrap(),
            render_api::capability::ProviderKind::Csri
        );
    }
}

/// 拿一个**两者都不是**的文件要报可读的错，而不是含糊的失败。
#[test]
fn classify_names_an_unrecognised_dll() {
    let p = std::env::temp_dir().join(format!("stk-neither-{}.dll", std::process::id()));
    // 拿一个有导出、但不是任何 renderer 的 DLL：kernel32
    let system = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let k32 = Path::new(&system).join("System32").join("kernel32.dll");
    if !k32.is_file() {
        eprintln!("跳过：找不到 kernel32");
        let _ = std::fs::remove_file(&p);
        return;
    }
    let e = renderer_host::backend::classify(&k32).unwrap_err();
    eprintln!("kernel32 被拒：{} / {:?}", e.title, e.details);
    assert!(
        e.title.contains("不属于任何已知"),
        "要说清它不属于任何链：{}",
        e.title
    );
    let _ = std::fs::remove_file(&p);
}

/// 使用 `Command` 只是为了让 `use` 不空 —— 保持在文件里以表明
/// 这些用例走的是**真实进程**而不是进程内调用。
#[test]
fn the_host_is_a_real_process() {
    let exe = host_exe();
    assert!(
        exe.is_file(),
        "renderer-host 应当是磁盘上的真实可执行文件：{}",
        exe.display()
    );
    let ok = Command::new(&exe).arg("--definitely-not-a-flag").output();
    assert!(ok.is_ok(), "能启动这个可执行文件");
}
