//! 12 §2-§3 的验收（AT-RENDER-001/002/003）。
//!
//! 这三条的共同点：它们考的都是**失败会怎样**，不是"能不能出图"。
//! 崩溃、挂住、位数不符都必须在主进程之外解决，所以这里的断言全部围绕
//! "host 死了，主进程拿到了什么证据，还能不能重试"。

use std::path::{Path, PathBuf};
use std::time::Duration;

use core_model::canvas::Canvas;
use core_model::time::Time;
use renderer_host::client::{Host, HostFailure, HostSpec};
use renderer_host::pe::{self, PeArch};
use renderer_host::protocol::API_VERSION;

const ASS: &str = "[Script Info]\r\nScriptType: v4.00+\r\nPlayResX: 1920\r\nPlayResY: 1080\r\n\r\n[V4+ Styles]\r\nFormat: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, Outline, Shadow, Alignment, MarginL, MarginR, MarginV\r\nStyle: Default,Arial,64,&H00FFFFFF,&H00000000,2,0,2,20,20,60\r\n\r\n[Events]\r\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\r\nDialogue: 0,0:00:01.00,0:00:03.00,Default,,0,0,0,,host isolation\r\n";

fn host_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_subtitletool-render-host"))
}

/// 真 libass：`STK_LIBASS_DLL` → 仓库旁边的 vcpkg 目录；`STK_REQUIRE_LIBASS=1` 时不许 SKIP
/// （"因为没跑所以通过"是最坏的一种绿，ISSUES Z4）。
fn libass() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let guess = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(|p| p.parent())
        .map(|root| {
            root.join("tools")
                .join("vcpkg")
                .join("installed")
                .join("x64-windows")
                .join("bin")
                .join("ass-9.dll")
        })?;
    guess.is_file().then_some(guess)
}

macro_rules! need_libass {
    () => {
        match libass() {
            Some(p) => p,
            None => {
                if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
                    panic!("STK_REQUIRE_LIBASS=1，但本机没有可用的 ass-9.dll（设 STK_LIBASS_DLL）");
                }
                eprintln!("SKIP：本机没有 libass DLL");
                return;
            }
        }
    };
}

/// 起一个 host。`crash` 非空时，宿主进程会在启动环境里收到 `STK_HOST_CRASH`，
/// 在读取任何控制面消息之前自我了断（AT-RENDER-002）。
/// exe 必须活到 `Host::start` 返回之后，所以整个构造放在函数里，而不是返回一个借用它的 `HostSpec`。
fn start_with_crash(
    dll: &Path,
    timeout: Duration,
    crash: Option<&str>,
) -> Result<Host, HostFailure> {
    let exe = host_exe();
    Host::start(&HostSpec {
        host_exe: &exe,
        dll,
        storage: Canvas::FHD1080,
        output: Canvas::FHD1080,
        default_font: "Arial",
        timeout,
        crash_injection: crash,
        lie_about_script_len: false,
        crash_dir: None,
    })
}

fn start(dll: &Path, timeout: Duration) -> Result<Host, HostFailure> {
    start_with_crash(dll, timeout, None)
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("stk-host-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// AT-RENDER-001：libass 在 host 里跑，主进程不加载任何 DLL，
/// 而且交回的像素必须与进程内渲染**逐字节相同**——否则"隔离"就变成了"换了个渲染器"。
#[test]
fn at_render_001_the_host_hands_back_the_same_pixels_as_an_in_process_render() {
    let dll = need_libass!();
    let mut host = start(&dll, Duration::from_secs(60)).expect("host 必须起得来");

    let hs = host.handshake().clone();
    assert_eq!(hs.provider, "libass", "{hs:?}");
    assert_eq!(hs.api_version, API_VERSION, "{hs:?}");
    assert_eq!(hs.pixel_format, "RGBA8", "{hs:?}");
    assert_eq!(hs.host_arch, "x64", "{hs:?}");
    assert_eq!(hs.provider_arch, "x64", "{hs:?}");
    // 版本**不在握手时报**：两条链都要真的打开 provider 才知道版本
    // （libass 要 `ass_library_version`，CSRI 要 `csri_renderer_info`），
    // 而握手发生在脚本到达之前 —— 那时还没有 provider 实例。
    //
    // 原来这里断言 `version` 以 `0x` 开头，而 host 那时确实**先开了**
    // libass 只为了问版本。改为按链分派后那一步没了（两条链不可能
    // 都用同一个“提前打开”），所以这个断言跟着改成“空即未报”。
    //
    // 真实的 provider 身份靠 `provider_binary_sha256`（下面那条）与
    // `provider probe` 命令，不靠这个字段。
    assert!(
        hs.version.is_empty(),
        "握手时不该报版本（还没打开 provider）：{hs:?}"
    );
    assert_eq!(hs.provider_binary_sha256.len(), 64, "{hs:?}");
    // 12 §4：身份是哈希。两侧各算一次，对不上就是看的不是同一个文件
    assert!(host.provider_sha_matches(&dll), "{hs:?}");

    host.load_script(ASS.as_bytes()).expect("脚本必须能加载");
    let frame = host.render(1500).expect("1.5s 处必须有帧");
    assert_eq!((frame.width, frame.height), (1920, 1080));
    assert_eq!(frame.rgba.len(), 1920 * 1080 * 4);
    assert!(frame.images > 0, "字幕画面上应该有图层：{frame:?}");
    // "崩-free"的一半证据：正常路径没有崩溃、没有超时
    let painted = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] != 0)
        .count();
    assert!(
        painted > 1000,
        "画面上几乎什么都没有：{painted} 个非透明像素"
    );

    // 另一半证据：与进程内渲染逐字节一致
    let provider = render_libass::Provider::open(&dll, Canvas::FHD1080, Canvas::FHD1080, "Arial")
        .expect("进程内 provider 必须起得来");
    let script = provider.load_script(ASS.as_bytes()).unwrap();
    let direct =
        render_libass::render(&script, Time::from_millis(1500).unwrap(), Canvas::FHD1080).unwrap();
    assert_eq!(
        direct.frame.rgba, frame.rgba,
        "host 与进程内渲染出的像素不同：隔离不该改变结果"
    );
    assert_eq!(direct.frame.canvas, Canvas::FHD1080);

    host.shutdown().expect("正常收尾");
}

/// AT-RENDER-002：provider 崩溃只能终止 host。主进程必须拿到退出码与 provider 日志，
/// 并且**能重试**——重试是新的一个 host，不是"假设刚才那个还能用"。
///
/// 崩溃由启动环境注入（`STK_HOST_CRASH`），不是控制面消息：正文二进制里没有可远程触发的自杀入口。
#[test]
fn at_render_002_a_crashing_host_is_reported_and_a_retry_works() {
    let dll = need_libass!();
    // 先证正常会话是好的，再起一个必崩的 host
    let mut good = start(&dll, Duration::from_secs(60)).expect("host 必须起得来");
    good.load_script(ASS.as_bytes()).unwrap();
    assert!(good.render(1500).is_ok(), "崩溃之前就该出帧");
    good.shutdown().expect("正常收尾");

    // 注入崩溃的宿主连握手都完成不了，主进程必须拿到 Crashed 而不是永远等
    let err = match start_with_crash(&dll, Duration::from_secs(60), Some("abort")) {
        Ok(_) => panic!("带崩溃注入的 host 不应该握手成功"),
        Err(e) => e,
    };
    let HostFailure::Crashed {
        exit_code,
        contained,
        job_error,
        ..
    } = &err
    else {
        panic!("崩溃必须报成 Crashed，收到 {err:?}");
    };
    assert!(exit_code.is_some(), "崩溃必须带退出码：{err:?}");
    let _ = (contained, job_error);

    // 主进程此刻还活着——测试继续跑本身就是证据。重试：新起一个 host
    let mut again = start(&dll, Duration::from_secs(60)).expect("重试必须起得来");
    again
        .load_script(ASS.as_bytes())
        .expect("重试后脚本必须能加载");
    let frame = again.render(1500).expect("重试必须能出帧");
    assert_eq!(frame.rgba.len(), 1920 * 1080 * 4);
    again.shutdown().expect("收尾");
}

/// 挂住不等于永远等：超时必须返回，并且交代"孙进程有没有被约束住"（12 §2 的 timeout）。
#[test]
fn at_render_002_a_hanging_host_times_out_and_can_be_abandoned() {
    let dll = need_libass!();
    // 宿主在启动阶段就挂死，所以 `Host::start` 的握手等待必须超时返回
    let started = std::time::Instant::now();
    let err = match start_with_crash(&dll, Duration::from_millis(700), Some("hang")) {
        Ok(_) => panic!("挂住的 host 不应该握手成功"),
        Err(e) => e,
    };
    let waited = started.elapsed();
    let HostFailure::Timeout { waited_ms, .. } = &err else {
        panic!("挂住必须报成 Timeout，收到 {err:?}");
    };
    assert_eq!(*waited_ms, 700, "{err:?}");
    assert!(
        waited < Duration::from_secs(20),
        "等了 {waited:?}，超时没有生效"
    );
}

/// 超时后放弃一个活着的 host：`abandon()` 必须交代"直接子进程有没有被杀掉、
/// Job 约束有没有生效"，不允许静默退化（Z6）。
#[test]
fn abandoning_a_live_host_reports_kill_and_containment() {
    let dll = need_libass!();
    let mut host = start(&dll, Duration::from_secs(60)).expect("host 必须起得来");
    let abandoned = host.abandon();
    assert!(abandoned.killed, "直接子进程没被杀掉：{abandoned:?}");
    assert!(
        abandoned.contained || abandoned.job_error != 0,
        "既没约束住又没给出错误码，等于静默退化：{abandoned:?}"
    );
}

/// AT-RENDER-003：位数不符的 DLL 必须在进程边界上被拒，绝不能走到 `LoadLibrary`。
/// 本机有真的 x86 PE（SysWOW64），所以这条能实测，而不是只测一个假头。
#[test]
fn at_render_003_an_x86_provider_is_never_handed_to_the_x64_host() {
    let wow64 = Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string()))
        .join("SysWOW64")
        .join("kernel32.dll");
    if !wow64.is_file() {
        // 没有 SysWOW64 就不是 64 位 Windows，这条测试的前提不成立；
        // 但仍然要能读出"这不是我能加载的东西"，所以下面的非 PE 断言照跑。
        eprintln!("SKIP：本机没有 {}", wow64.display());
    } else {
        let arch = pe::arch_of_file(&wow64).expect("SysWOW64 的 DLL 必须能读出 PE 头");
        assert_eq!(arch, PeArch::X86, "{wow64:?} 应该是 x86");

        let exe = host_exe();
        assert_eq!(
            pe::arch_of_file(&exe).unwrap(),
            PeArch::X64,
            "本测试用的 host 必须是 x64：{}",
            exe.display()
        );
        let err = Host::start(&HostSpec {
            host_exe: &exe,
            dll: &wow64,
            storage: Canvas::FHD1080,
            output: Canvas::FHD1080,
            default_font: "Arial",
            timeout: Duration::from_secs(5),
            crash_injection: None,
            lie_about_script_len: false,
            crash_dir: None,
        })
        .expect_err("x86 provider 交给 x64 host 必须被拒绝");
        assert_eq!(err.code(), "STK-RND-0201", "{err:?}");
        match err {
            HostFailure::Arch { provider, host } => {
                assert_eq!(provider, "x86");
                assert_eq!(host, "x64");
            }
            other => panic!("位数不符必须报成 Arch，收到 {other:?}"),
        }
        // 关键：拒绝发生在**起进程之前**，所以没有新进程需要清理
    }

    // 非 PE 输入也要给出可读诊断，而不是崩在头解析里
    let dir = temp_dir("notpe");
    let fake = dir.join("Cargo.toml");
    std::fs::write(&fake, "[package]\nname = \"definitely-not-a-dll\"\n").unwrap();
    let err = Host::start(&HostSpec {
        host_exe: &host_exe(),
        dll: &fake,
        storage: Canvas::FHD1080,
        output: Canvas::FHD1080,
        default_font: "Arial",
        timeout: Duration::from_secs(5),
        crash_injection: None,
        lie_about_script_len: false,
        crash_dir: None,
    })
    .expect_err("非 PE 文件必须被拒");
    let HostFailure::Io(msg) = &err else {
        panic!("读不出 PE 头属于 Io/诊断，收到 {err:?}");
    };
    assert!(msg.contains("PE"), "{msg}");
    std::fs::remove_dir_all(&dir).ok();
}

/// host 侧也要自查位数（纵深防御）：直接给它一个 x86 DLL，它不加载、报 0201、退非零。
#[test]
fn the_host_refuses_a_mismatched_provider_before_loading_it() {
    let wow64 = Path::new(&std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string()))
        .join("SysWOW64")
        .join("kernel32.dll");
    if pe::arch_of_file(&wow64) != Ok(PeArch::X86) {
        eprintln!("SKIP：本机没有 x86 系统 DLL 可用来验证 host 的自查");
        return;
    }
    let exe = host_exe();
    let out = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("host 必须能启动");
    reap_and_check(out, &wow64);
}

fn reap_and_check(mut child: std::process::Child, dll: &Path) {
    use renderer_host::protocol::{self, Message, Request};
    use std::io::Write;

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let hello = Request::Hello {
        dll: dll.display().to_string(),
        storage: [1920, 1080],
        output: [1920, 1080],
        default_font: "Arial".to_string(),
    };
    protocol::write_message(&mut stdin, &protocol::to_json(&hello).unwrap()).unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let msg = protocol::read_message(&mut stdout)
        .unwrap()
        .expect("host 必须回一条消息");
    let Message::Text(text) = msg else {
        panic!("期望 JSON 回应，收到原始字节");
    };
    let resp: renderer_host::protocol::Response = serde_json::from_str(&text).unwrap();
    match resp {
        renderer_host::protocol::Response::Error {
            code,
            title,
            details: _,
        } => {
            assert_eq!(code, "STK-RND-0201", "{title}");
            assert!(title.contains("位数不符"), "{title}");
        }
        other => panic!("host 应该拒绝加载并报错，收到 {other:?}"),
    }
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "拒绝加载之后 host 必须以非零码退出，不能装作干完了"
    );
}
