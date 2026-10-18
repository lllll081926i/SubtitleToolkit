//! AT-RENDER-002：**真实第三方 provider 的崩溃只终止 renderer-host，主进程可重试**。
//!
//! ## 规格要求
//!
//! > 取得可用 VSFilterMod provider 后，证明其崩溃只终止 renderer-host，GUI 可重试；
//! > 自伤 `Poison` 测试不能代替该 provider 测试。
//!
//! ## 这条用例怎么造出**真的** VSFilterMod 崩溃
//!
//! 不是模拟、不是注入、不是 poison —— 是**真的让 DLL 自己访问违例**。
//!
//! 探针（`docs/reviews/2026-10-17-at-render-002-real-crash.md` 有原始记录）逐个试了
//! 18 种违反 CSRI 契约的输入，只有一种让它崩：
//!
//! ```text
//! open-mem-bad-len   exit=-1073741819   ← 0xC0000005 访问违例
//! ```
//!
//! `csri_open_mem(rdr, data, length, ...)` 的 `length` 若**大于真实缓冲**，
//! VSFilterMod 不做上界检查，顺着读下去直到越界。
//!
//! 所以本用例走一条**只有 host 会走**的路径：把脚本交进去时不告诉它真实长度。
//! 为此 host 侧加了一个**只在测试下发**的开关 [`LIE_ABOUT_LENGTH_ENV`]。
//!
//! ## 为什么开关放在 host 侧而不是"让 host 读一个畸形文件"
//!
//! 因为 host 的**正常**路径已经先 `read_bounded` 校验过长度了（23 §输入上限）。
//! 要复现这个崩溃就必须**绕过那道校验**，而那正是 provider 自己缺失的那道。
//! 用一个显式开关，比"让 host 偶尔不校验"要诚实得多 ——
//! 开关的名字本身说明了它在造什么。
//!
//! ## 为什么关掉 Windows 错误报告
//!
//! 真访问违例会触发 WER 弹窗，那会卡住 CI、也需要人去点。
//! 本用例在 host 进程里设 `SEM_NOGPFAULTERRORBOX`（通过 `SetErrorMode` 的父进程继承
//! 不够可靠，所以在 host 启动时自己设）。**关掉的是对话框，不是崩溃** ——
//! 进程照样死、退出码照样是 0xC0000005。

use std::path::PathBuf;
use std::time::Duration;

use renderer_host::client::{HostFailure, HostSpec};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/renderer-host 的上两层就是仓库根")
        .to_path_buf()
}

fn csri_dll() -> PathBuf {
    std::env::var("STK_CSRI_PROVIDER")
        .map(PathBuf::from)
        .unwrap_or_else(|_| repo_root().join("third_party").join("VSFilterMod.dll"))
}

fn host_exe() -> PathBuf {
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

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "stk-at002-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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
Dialogue: 0,0:00:00.00,0:00:05.00,D,,0,0,0,,AT-RENDER-002
"#;

fn spec<'a>(
    dll: &'a std::path::Path,
    exe: &'a std::path::Path,
    crash_dir: Option<&'a std::path::Path>,
    lie: bool,
) -> HostSpec<'a> {
    HostSpec {
        host_exe: exe,
        dll,
        storage: core_model::canvas::Canvas::new(640, 480).unwrap(),
        output: core_model::canvas::Canvas::new(640, 480).unwrap(),
        default_font: "Arial",
        // 真访问违例不需要等很久：它在 open 那一刻就死
        timeout: Duration::from_secs(20),
        crash_injection: None,
        crash_dir,
        lie_about_script_len: lie,
    }
}

fn need_provider() -> (PathBuf, PathBuf) {
    let dll = csri_dll();
    assert!(
        dll.is_file(),
        "找不到 CSRI provider {}。AT-RENDER-002 核的就是真实第三方 provider 的崩溃隔离，\n\
         缺素材时应当**失败**而不是跳过 —— 静默跳过会让“没测”看起来像“测过且对”。",
        dll.display()
    );
    let exe = host_exe();
    assert!(exe.is_file(), "找不到 renderer-host：{}", exe.display());
    (dll, exe)
}

/// **核心用例**：真实 VSFilterMod 访问违例，主进程活着、拿到崩溃现场、能重试。
///
/// 这一个用例同时回答规格里的三句话：
///   - "其崩溃"           → host 真的死在 provider 的代码里（0xC0000005）
///   - "只终止 renderer-host" → 本进程没跟着死
///   - "GUI 可重试"        → 换一个 host 重跑同一份输入能出帧
#[test]
fn a_real_vsfiltermod_access_violation_only_kills_the_host() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("realcrash");

    // ---- 第一次：让 provider 真的崩 ----
    let s = spec(&dll, &exe, Some(&dir), true);
    let mut host = renderer_host::client::Host::start(&s).expect("起 host");

    // 脚本交进去的那一刻 provider 越界读 —— 这一调用会失败。
    // **本进程必须活着** —— 隔离没做成的话，这里整个测试进程就没了。
    let err = host
        .load_script(ASS.as_bytes())
        .expect_err("provider 必须在这里崩掉（这就是要测的东西）");
    eprintln!("主进程活着，拿到：{err:?}");

    match &err {
        HostFailure::Crashed {
            exit_code,
            crash_note,
            contained,
            ..
        } => {
            assert!(
                *contained,
                "崩溃必须被判定为“已隔离”（它在 host 里，不在我们这儿）"
            );
            let code = exit_code.expect("崩溃必须带退出码");
            eprintln!("退出码 = {code} (0x{code:08X})");
            // 0xC0000005 = STATUS_ACCESS_VIOLATION
            assert_eq!(
                code, 0xC000_0005,
                "应当是访问违例 —— 别的退出码说明崩的不是 provider 的越界读"
            );
            let note = crash_note
                .as_ref()
                .expect("22 §6：注入前会写现场，所以这里应当有");
            eprintln!("崩溃现场：{}", renderer_host::crash::summarize(note));
            assert!(
                note.provider_sha256.is_some(),
                "现场要说清是哪个 provider 崩的"
            );
        }
        other => panic!("应当是 Crashed(0xC0000005)，收到 {other:?}"),
    }
    let _ = host.shutdown();

    // ---- 第二次：同一份输入，正常路径 —— 必须能出帧 ----
    let mut h = renderer_host::client::Host::start(&spec(&dll, &exe, Some(&dir), false))
        .expect("重试要能起得来（上一个 host 的死不该影响下一个）");
    h.load_script(ASS.as_bytes()).expect("重试时脚本要能加载");
    let frame = h.render(2000).expect("重试必须能出帧");
    assert_eq!((frame.width, frame.height), (640, 480));
    let painted = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count();
    eprintln!("重试成功：{painted} 个不透明像素");
    assert!(painted > 0, "重试拿到的帧里应当有字幕内容");
    let _ = h.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 崩溃现场**要落盘**：进程死后只有落盘的那份能进错误卡。
#[test]
fn the_real_crash_note_is_written_to_disk() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("realnote");
    {
        let s = spec(&dll, &exe, Some(&dir), true);
        let mut h = renderer_host::client::Host::start(&s).unwrap();
        let _ = h.load_script(ASS.as_bytes()).expect_err("必须崩");
        let _ = h.shutdown();
    }

    let note_path = dir.join(renderer_host::crash::FILE_NAME);
    let text = std::fs::read_to_string(&note_path)
        .unwrap_or_else(|e| panic!("崩溃现场必须落盘（读不到 {}：{e}）", note_path.display()));
    eprintln!("现场文件：{text}");
    assert!(
        !text.is_empty(),
        "现场文件不该是空的 —— 空的现场等于没有现场"
    );
    assert!(
        text.contains("VSFilterMod") || text.contains("provider_sha256"),
        "现场要说清是哪个 provider：{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
