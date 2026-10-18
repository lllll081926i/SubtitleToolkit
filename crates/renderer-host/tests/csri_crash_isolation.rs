//! AT-RENDER-002：**第三方 provider 崩溃只终止 renderer-host，主进程可以重试**。
//!
//! ## 规格怎么说的
//!
//! > 取得可用 VSFilterMod provider 后，证明其崩溃只终止 renderer-host，GUI 可重试；
//! > 自伤 `Poison` 测试不能代替该 provider 测试。
//!
//! ## 本文件做到了哪一步，以及哪一步没做到
//!
//! **做到了**：用**真实 CSRI provider**（VSFilterMod.dll）跑完整握手与出帧，
//! 在**真的要调 provider 的那一刻**（`crash_during_render_if_asked`）
//! 把 host 弄死，然后证明：
//!
//! - 主进程**没有跟着死**（它拿到了错误而不是崩）；
//! - 错误里带得出**崩溃现场**（22 §6 的记录说明死在哪个阶段、第几帧）；
//! - **重试能成** —— 换成不注入的 host 重跑同一份输入，出帧正常。
//!
//! **没做到**：让**第三方 DLL 自己**真的崩。那需要它的代码里有一个可触发的缺陷
//! 或一个我们能控制的崩溃入口 —— 两者我们都没有，而"故意喂坏数据让它崩"
//! 不是可复现的测试，是碰运气。
//!
//! 所以这里的注入点**在 provider 调用的前一刻**：它复现的是**同一个时序**
//! （控制面在等回复、而对面已经死了），而隔离机制要对付的正是那个时序。
//! **但这不是规格要求的那条证据**，所以 AT-RENDER-002 **仍记为未验收**。

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
    if let Ok(p) = std::env::var("STK_CSRI_PROVIDER") {
        return PathBuf::from(p);
    }
    repo_root().join("third_party").join("VSFilterMod.dll")
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
        "stk-csri-crash-{tag}-{}-{}",
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
    crash_injection: Option<&'a str>,
    crash_dir: Option<&'a std::path::Path>,
) -> HostSpec<'a> {
    HostSpec {
        host_exe: exe,
        dll,
        storage: core_model::canvas::Canvas::new(640, 480).unwrap(),
        output: core_model::canvas::Canvas::new(640, 480).unwrap(),
        default_font: "Arial",
        timeout: Duration::from_secs(20),
        crash_injection,
        crash_dir,
    }
}

fn need_provider() -> (PathBuf, PathBuf) {
    let dll = csri_dll();
    assert!(
        dll.is_file(),
        "找不到 CSRI provider {}。AT-RENDER-002 核的就是第三方 provider 的崩溃隔离，\n\
         缺素材时应当失败而不是跳过。",
        dll.display()
    );
    let exe = host_exe();
    assert!(exe.is_file(), "找不到 renderer-host：{}", exe.display());
    (dll, exe)
}

/// **核心用例**：CSRI provider 出帧途中 host 死了，主进程活着并拿到错误。
#[test]
fn a_csri_host_dying_mid_render_does_not_kill_the_caller() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("midrender");
    let mut host =
        renderer_host::client::Host::start(&spec(&dll, &exe, Some("render-abort"), Some(&dir)))
            .expect("拉起一个会被注入崩溃的 host");
    host.load_script(ASS.as_bytes())
        .expect("脚本要能加载（崩溃发生在出帧时，不是加载时）");

    // 这一调用会让 host 在里面 abort。
    // **本进程必须活着** —— 如果隔离没做成，这里就是整个测试进程崩掉。
    let err = host
        .render(2000)
        .expect_err("出帧必须失败（host 死在那一帧里）");
    eprintln!("主进程活着，拿到：{err:?}");

    match &err {
        HostFailure::Crashed {
            exit_code,
            crash_note,
            ..
        } => {
            eprintln!(
                "退出码={exit_code:?}  现场={:?}",
                crash_note
                    .as_ref()
                    .map(|n| renderer_host::crash::summarize(n))
            );
            assert!(exit_code.is_some(), "崩溃必须带退出码");
            // 22 §6：崩溃现场要能回答“哪一步死的”。
            let note = crash_note
                .as_ref()
                .expect("注入前会写一条记录，所以这里应当有现场");
            let s = renderer_host::crash::summarize(note);
            assert!(
                s.contains("render") || s.contains("出帧"),
                "现场要说清死在哪个阶段：{s}"
            );
        }
        // 管道被提前关掉也算"host 没了"，但那不是我们想验的路径：
        // 不许静默放过。
        other => panic!("应当是 Crashed，收到 {other:?}"),
    }

    let _ = host.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// **重试必须能成**：同一份输入，换成不注入的 host，出帧正常。
///
/// 这条是"GUI 可重试"那半句的对应物。只验崩溃不验重试，
/// 等于只证明了一半 —— 用户真正要的是"崩了还能接着干"。
#[test]
fn retrying_after_a_csri_crash_succeeds() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("retry");

    // 第一次：崩
    {
        let mut h =
            renderer_host::client::Host::start(&spec(&dll, &exe, Some("render-abort"), Some(&dir)))
                .unwrap();
        h.load_script(ASS.as_bytes()).unwrap();
        let e = h.render(2000).expect_err("第一次必须失败");
        assert!(matches!(e, HostFailure::Crashed { .. }), "{e:?}");
        let _ = h.shutdown();
    }

    // 第二次：同一份输入，不注入 —— 必须能出帧
    let mut h = renderer_host::client::Host::start(&spec(&dll, &exe, None, Some(&dir)))
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

/// 挂住的 host 要被**超时**放手，而不是让主进程永远等。
///
/// 这条与"崩溃"是两类故障：崩溃会关管道，挂住不会。
/// 只测其中一个会漏掉另一种 —— 而后者会把 GUI 冻在那儿。
#[test]
fn a_csri_host_hanging_mid_render_times_out() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("hangmid");
    let mut s = spec(&dll, &exe, Some("render-hang"), Some(&dir));
    // 这条用例故意把超时调短：它只验"会不会放手"，不验等多久。
    s.timeout = Duration::from_secs(2);
    let mut host = renderer_host::client::Host::start(&s).expect("起 host");
    host.load_script(ASS.as_bytes()).expect("脚本要能加载");
    let err = host.render(2000).expect_err("挂住的 host 必须超时");
    eprintln!("超时报告：{err:?}");
    assert!(
        matches!(err, HostFailure::Timeout { .. }),
        "应当是超时：{err:?}"
    );
    let _ = host.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 崩溃现场**要落盘**（22 §6），而不只是活在内存里。
///
/// 理由：真崩溃之后进程没了，内存里那份也随它没了 ——
/// 只有落盘的那份能进错误卡供人看。
#[test]
fn the_crash_note_is_written_to_the_crash_dir() {
    let (dll, exe) = need_provider();
    let dir = tmp_dir("note");
    {
        let mut h =
            renderer_host::client::Host::start(&spec(&dll, &exe, Some("render-abort"), Some(&dir)))
                .unwrap();
        h.load_script(ASS.as_bytes()).unwrap();
        let _ = h.render(2000).expect_err("必须失败");
        let _ = h.shutdown();
    }

    // 记录文件应当出现在 crash_dir 里
    let mut found: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            found.push(e.file_name().to_string_lossy().to_string());
        }
    }
    eprintln!("崩溃目录内容：{found:?}");
    assert!(
        !found.is_empty(),
        "崩溃现场必须落盘，否则进程死后什么都拿不到"
    );
    // 而且内容要能读、要说得出阶段
    let note_path = dir.join(renderer_host::crash::FILE_NAME);
    let text = std::fs::read_to_string(&note_path)
        .unwrap_or_else(|e| panic!("读不到 {}：{e}", note_path.display()));
    assert!(text.contains("render"), "现场记录要说清阶段：{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
