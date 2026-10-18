//! `subtitletool-render-host.exe`：唯一在这里，第三方 DLL 才进进程（12 §2）。
//!
//! host 的责任边界刻意很窄：握手 → 载脚本 → 按请求出帧，仅此而已。
//! 业务判断（变化检测、编码、校验）全部留在主进程侧，这样 host 崩溃不会带走任何
//! 已经算好的结果，重试也只是重做像素。
//!
//! 崩溃注入（AT-RENDER-002 的"第三方 provider 崩了怎么办"）只由宿主进程自己的
//! 环境变量 `STK_HOST_CRASH` 触发。控制面协议不携带自毁指令，所以发布二进制没有
//! 任何可被客户端控制的 abort/hang 入口。

use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use core_model::canvas::Canvas;
use core_model::time::Time;
use sha2::{Digest as _, Sha256};

use renderer_host::pe::{self, PeArch};
use renderer_host::protocol::{self, Handshake, Message, Request, Response, API_VERSION};

fn main() -> ExitCode {
    match serve() {
        Ok(()) => ExitCode::SUCCESS,
        // 任何非零退出码在主进程侧都会被当成"host 没了"，所以这里必须是干净的失败
        Err(reason) => {
            eprintln!("[renderer-host] {reason}");
            ExitCode::from(4)
        }
    }
}

fn arch_label(a: PeArch) -> String {
    match a {
        PeArch::Other(v) => format!("未知架构 0x{v:04x}"),
        other => other.label().to_string(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn reply(w: &mut impl Write, resp: &Response) -> io::Result<()> {
    protocol::write_message(w, &protocol::to_json(resp)?)
}

fn reply_err(w: &mut impl Write, code: &str, title: &str) -> io::Result<()> {
    eprintln!("[renderer-host] {code}: {title}");
    reply(
        w,
        &Response::Error {
            code: code.to_string(),
            title: title.to_string(),
            // 空：调用方若需要带上 details，用下面的 `reply_stk_err`。
            // 保留这个重载是为了“只有一句话”的那些场合（如协议错）。
            details: Vec::new(),
        },
    )
}

/// 把 `StkError` 的**全部**信息回给主进程。
///
/// 为什么不能只传 `title`：host 是**唯一**真的看过这个 DLL 的地方。
/// 它知道“这是一个 CSRI provider，不是 libass”或“缺哪几个符号”，
/// 而主进程只知道“host 拒绝了”。只传一句概括的话，那份诊断就在进程边界上丢了 ——
/// 而 12 §1 把“说清它是什么”当成能力探测的产出之一。
fn reply_stk_err(w: &mut impl Write, e: &core_model::StkError) -> io::Result<()> {
    eprintln!("[renderer-host] {}: {}", e.code_string(), e.title);
    for d in &e.details {
        eprintln!("[renderer-host]   {d}");
    }
    reply(
        w,
        &Response::Error {
            code: e.code_string(),
            title: e.title.clone(),
            details: e.details.clone(),
        },
    )
}

fn serve() -> Result<(), String> {
    // 22 §6：崩溃现场追踪器。先建它，这样连“握手之前就死了”也有记录。
    let tracker = renderer_host::crash::Tracker::from_env();
    tracker.enter(renderer_host::crash::Phase::Startup, None);

    // 在读取任何控制面消息之前就处理崩溃注入：环境变量只有宿主进程自己的启动环境能设，
    // 管道对端无论发什么都无法在运行时自毁宿主。
    maybe_inject_crash_from_env(&tracker);

    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    // BufWriter 在这里不是优化而是正确性的一部分：帧负载有 8 MiB，
    // 逐字节 write 会让 host 在主进程已经放弃等待之后还在往断掉的管道里写。
    let mut writer = BufWriter::new(stdout.lock());

    let first = protocol::read_message(&mut reader)
        .map_err(|e| e.to_string())?
        .ok_or("控制面在 Hello 之前就关闭了")?;
    let hello: Request = protocol::from_json(&first).map_err(|e| e.to_string())?;
    let Request::Hello {
        dll,
        storage,
        output,
        default_font,
    } = hello
    else {
        return Err("第一条消息必须是 Hello".to_string());
    };

    let dll_path = PathBuf::from(&dll);
    // 12 §3 的禁令在两侧都要自查：主进程查"我给对了 host 吗"，host 查"我到底能不能加载它"。
    let provider_arch = pe::arch_of_file(&dll_path)
        .map_err(|e| format!("{e}"))
        .and_then(|a| match a {
            PeArch::Other(v) => Err(format!("拒绝加载未知架构的 provider（0x{v:04x}）")),
            other => Ok(other),
        })?;
    let me = std::env::current_exe().map_err(|e| format!("拿不到自己的路径：{e}"))?;
    let host_arch = pe::arch_of_file(&me).map_err(|e| format!("{e}"))?;
    if provider_arch != host_arch {
        let title = format!(
            "provider {} 与 host {} 位数不符，拒绝加载（12 §3）",
            arch_label(provider_arch),
            arch_label(host_arch)
        );
        reply_err(&mut writer, "STK-RND-0201", &title).map_err(|e| e.to_string())?;
        return Err(title);
    }

    let storage_canvas = Canvas::new(storage[0], storage[1])
        .map_err(|e| format!("storage 画布非法：{}", e.title))?;
    let output_canvas =
        Canvas::new(output[0], output[1]).map_err(|e| format!("output 画布非法：{}", e.title))?;

    let dll_bytes = std::fs::read(&dll_path).map_err(|e| format!("读不到 provider：{e}"))?;
    let mut sh = Sha256::new();
    sh.update(&dll_bytes);
    let provider_sha = hex(&sh.finalize());
    // 22 §6：provider 身份是“哪个 DLL 死的”这个问题的答案。
    tracker.record_provider(&dll_path, &provider_sha, &arch_label(provider_arch));
    tracker.record_canvas(
        (storage_canvas.width, storage_canvas.height),
        (output_canvas.width, output_canvas.height),
    );

    // 到这里才真的 LoadLibrary（`core_model::dllload` 那套 LOAD_LIBRARY_SEARCH_*，
    // 不搜当前目录）。
    //
    // ## 为什么握手时只探测、不打开
    //
    // 两条链的**打开时机不同**：libass 先 open 再 load_script（两步），
    // 而 CSRI 的 `csri_open_mem` 在 open 时就要脚本（一步）。
    //
    // 主机侧只知道了档位（storage/output 画布、默认字体），还不知道脚本 ——
    // 那要等 `Request::LoadScript` 才来。所以这里：
    //
    // 1. **先按导出表分流**（12 §1，不加载）；
    // 2. 把类别写进握手（主进程能看到自己用的是哪条链）；
    // 3. 等脚本到了再真的 open。
    //
    // 为什么不在这里就用一个占位脚本提前 open：那会把一份**我们没校验过**
    // 的文本递给第三方 provider，而 17 §7 的字体环境与缓存 key 是以
    // “我们解析过的那份字节”为准的。
    let backend_kind = renderer_host::backend::classify(&dll_path).map_err(|e| {
        let _ = reply_stk_err(&mut writer, &e);
        format!("provider 分派失败：{}；{}", e.title, e.details.join("；"))
    })?;

    let name = dll_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("provider")
        .to_string();
    let provider_label = match backend_kind {
        render_api::capability::ProviderKind::LibassBuiltin => "libass".to_string(),
        render_api::capability::ProviderKind::Csri => "csri".to_string(),
        render_api::capability::ProviderKind::LegacyVapourSynth => "vapoursynth".to_string(),
        render_api::capability::ProviderKind::LegacyAviSynth => "avisynth".to_string(),
        render_api::capability::ProviderKind::LegacyOther => "legacy-other".to_string(),
    };

    // 版本：两条链拿它的时机不同。
    //
    // - **libass**：`ass_library_version()` 只要 DLL 在就行，不必等脚本。
    //   所以这里直接开一次 libass 只为了问版本 —— 但那又会加载两次 DLL。
    //   更简单且不重复加载的做法：**从导出表读不了版本**，
    //   那就把 libass 的版本推到真打开之后（与 CSRI 一致）。
    //
    // - **CSRI**：只能从 `csri_renderer_info()` 拿，而那需要 renderer 句柄，
    //   同样要等真打开。
    //
    // 两者一致地“握手时不报版本”是**诚实的**：那一刻确实还没有 provider 实例。
    // 而客户端（`verify_handshake`）不依赖版本字段，只依赖 provider 名字。
    reply(
        &mut writer,
        &Response::Hello(Handshake {
            provider: provider_label.clone(),
            name: name.clone(),
            // 空 = “还没打开”。客户端的握手校验不依赖它；
            // 真正的版本在 `provider probe` 与报告里给。
            version: String::new(),
            api_version: API_VERSION,
            host_arch: arch_label(host_arch),
            provider_arch: arch_label(provider_arch),
            pixel_format: "RGBA8".to_string(),
            provider_binary_sha256: provider_sha,
        }),
    )
    .map_err(|e| e.to_string())?;

    let mut pending_fonts: Vec<(String, Vec<u8>)> = Vec::new();
    let ass = loop {
        let Some(msg) = protocol::read_message(&mut reader).map_err(|e| e.to_string())? else {
            return Ok(());
        };
        let req: Request = protocol::from_json(&msg).map_err(|e| e.to_string())?;
        match req {
            Request::Hello { .. } => {
                reply_err(&mut writer, "STK-RND-0205", "已经握手，重复的 Hello 被拒绝")
                    .map_err(|e| e.to_string())?;
            }
            Request::LoadScript => {
                tracker.enter(renderer_host::crash::Phase::LoadScript, None);
                let blob = protocol::read_message(&mut reader)
                    .map_err(|e| e.to_string())?
                    .ok_or("LoadScript 之后没有负载")?;
                let Message::Blob(ass) = blob else {
                    reply_err(
                        &mut writer,
                        "STK-RND-0205",
                        "LoadScript 的负载必须是原始字节",
                    )
                    .map_err(|e| e.to_string())?;
                    continue;
                };
                break ass;
            }
            Request::AddFont { family } => {
                let blob = protocol::read_message(&mut reader)
                    .map_err(|e| e.to_string())?
                    .ok_or("AddFont 之后没有负载")?;
                let Message::Blob(font) = blob else {
                    reply_err(&mut writer, "STK-RND-0205", "AddFont 的负载必须是原始字节")
                        .map_err(|e| e.to_string())?;
                    continue;
                };
                let bytes = font.len();
                if family.len() > 1024 || pending_fonts.len() >= 4096 {
                    reply_err(
                        &mut writer,
                        "STK-RND-0205",
                        "字体名称或字体条目数超过会话上限",
                    )
                    .map_err(|e| e.to_string())?;
                    continue;
                }
                let pending_bytes = pending_fonts
                    .iter()
                    .try_fold(bytes, |total, (_, item)| total.checked_add(item.len()));
                if pending_bytes.is_none_or(|total| total > 256 * 1024 * 1024) {
                    reply_err(&mut writer, "STK-RND-0205", "会话字体总量超过 256 MiB 上限")
                        .map_err(|e| e.to_string())?;
                    continue;
                }
                pending_fonts.push((family, font));
                reply(&mut writer, &Response::FontAdded { bytes }).map_err(|e| e.to_string())?;
            }
            Request::Render { .. } => {
                reply_err(&mut writer, "STK-RND-0205", "LoadScript 之前不能渲染")
                    .map_err(|e| e.to_string())?;
            }
            Request::Bye => {
                reply(&mut writer, &Response::ByeOk).map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
    };

    // 开 provider。
    //
    // 注意**先做字体能力检查、再注入**：`add_font` 对不支持注入的链
    // 返回 `Ok(0)`（"没做"而不是"失败"）—— 那条链是 CSRI，它的头文件里
    // 没有加字体的入口。
    //
    // 而"没做"必须**报出去**：用户以为字形来自他给的那份字体、
    // 而实际用的是系统字体，结果是渲染出来的东西与预期不同却不报警。
    // 所以这里发一条告警（22 §：不能说悄悄地少做一件事）。
    let ass_text = String::from_utf8_lossy(&ass).to_string();
    let mut backend = match renderer_host::backend::Backend::open(
        &dll_path,
        &ass_text,
        storage_canvas,
        output_canvas,
        &default_font,
    ) {
        Ok(b) => b,
        Err(e) => {
            // 把 details 一起回给主进程（见 `reply_stk_err` 的注释）。
            let _ = reply_stk_err(&mut writer, &e);
            return Err(format!(
                "provider 打不开：{}；{}",
                e.title,
                e.details.join("；")
            ));
        }
    };

    // 字体：能力不同的链走不同的路。
    //
    // 两条链在这里的差别是**语义差异**而不是实现细节：libass 有
    // `ass_add_font`，而 CSRI 头文件里**没有**加字体的入口。
    // 所以 CSRI 链上这件事做不到 —— 而那必须**说出来**：
    // 用户以为字形来自他给的那份字体、实际用的是系统字体，
    // 结果是渲染出来的东西与预期不同却不报警。
    let can_inject = backend.supports_font_injection();
    if !pending_fonts.is_empty() && !can_inject {
        eprintln!(
            "[renderer-host] 警告：{} 链不支持内存字体注入，{} 份字体被跳过（17 §7）",
            backend.name(),
            pending_fonts.len()
        );
    }
    let mut injected = 0usize;
    for (family, font) in pending_fonts {
        match backend.add_font(&family, &font) {
            Ok(()) => injected += 1,
            Err(e) => {
                reply_err(&mut writer, &e.code_string(), &e.title).map_err(|reply_error| {
                    format!("字体注入失败：{}；发送错误响应失败：{reply_error}", e.title)
                })?;
                return Ok(());
            }
        }
    }
    // 支持注入而一份都没进：那是上游自己的事（它会报），这里不重复告警。
    let _ = injected;

    // libass 链要把脚本单独加载（两步）；CSRI 链在 `open` 时已经交过了。
    //
    // ## 为什么 `Script` 不能放进 `Backend`
    //
    // `render_libass::Script<'a>` **借用了** `Provider`（它持有 renderer 与
    // track 的裸指针，而那两个的生命期由 Provider 保证）。
    // 把它放进 `Backend::Libass` 会变成一个自引用结构 —— 而自引用在
    // 安全 Rust 里写不出来（这正是 `self_cell` 那类 crate 存在的理由）。
    //
    // 所以脚本句柄留在**调用方**：`Backend` 只管“用什么链”，
    // 脚本生命期由它外面那层持有。CSRI 那份不需要这个句柄
    // （脚本在 open 时就交进去了），所以那里是 `None`。
    reply(&mut writer, &Response::ScriptLoaded { bytes: ass.len() }).map_err(|e| e.to_string())?;

    // libass 链要把脚本单独加载（两步）；CSRI 链在 `open` 时已经交过了。
    //
    // ## `Script` 为什么留在外面
    //
    // 它原来是 `Script<'a>`，**借用** `Provider` 的函数表 —— 那会让
    // 调用方在持有脚本时无法可变借用 provider，而 `Backend::render_at`
    // 恰恰需要那个 `&mut`。
    //
    // 已改成 `Script` **拷贝**一份函数表（`Symbols: Copy`，几十字节），
    // 于是它不再借用任何东西。放在外面仍是对的：`Backend` 只管“用什么链”，
    // 脚本句柄的生命期由调用方持有更直白。
    let is_libass = matches!(backend, renderer_host::backend::Backend::Libass(_));
    let script = if let renderer_host::backend::Backend::Libass(p) = &backend {
        match p.load_script(&ass) {
            Ok(s) => Some(s),
            Err(e) => {
                reply_err(&mut writer, &e.code_string(), &e.title).map_err(|reply_error| {
                    format!("脚本加载失败：{}；发送错误响应失败：{reply_error}", e.title)
                })?;
                return Ok(());
            }
        }
    } else {
        None
    };
    debug_assert_eq!(script.is_some(), is_libass);

    let mut frames = FrameSink {
        backend: &mut backend,
        libass_script: script.as_ref(),
    };
    serve_render_requests(
        &mut reader,
        &mut writer,
        &mut frames,
        output_canvas,
        &tracker,
    )
}

/// 渲染循环需要的东西：provider + （libass 才有的）脚本句柄。
///
/// 把它包成一个结构而不是两个参数，是因为 `render_at` 里要**同时**
/// 用 `&mut backend` 与 `&Script` —— 拆成两个参数会让调用点
/// 无法向借用检查器证明“这两个借用不重叠”。
struct FrameSink<'a> {
    backend: &'a mut renderer_host::backend::Backend,
    libass_script: Option<&'a render_libass::Script>,
}

fn serve_render_requests(
    reader: &mut impl std::io::Read,
    writer: &mut impl Write,
    sink: &mut FrameSink<'_>,
    output: Canvas,
    tracker: &renderer_host::crash::Tracker,
) -> Result<(), String> {
    // 22 §6：进入烧帧阶段。之后的每次崩溃注入都能在记录里看到
    // “死在出第几帧”，那正是调查的第一个问题。
    tracker.enter(renderer_host::crash::Phase::Render, Some(0));
    let mut frames: u64 = 0;
    while let Some(msg) = protocol::read_message(reader).map_err(|e| e.to_string())? {
        let req: Request = protocol::from_json(&msg).map_err(|e| e.to_string())?;
        match req {
            Request::Hello { .. } => {
                reply_err(writer, "STK-RND-0205", "已经握手，重复的 Hello 被拒绝")
                    .map_err(|e| e.to_string())?;
            }
            Request::LoadScript => {
                let _ = protocol::read_message(reader)
                    .map_err(|e| e.to_string())?
                    .ok_or("重复 LoadScript 之后没有负载")?;
                reply_err(writer, "STK-RND-0205", "一个 host 会话只能加载一个脚本")
                    .map_err(|e| e.to_string())?;
            }
            Request::AddFont { .. } => {
                let _ = protocol::read_message(reader)
                    .map_err(|e| e.to_string())?
                    .ok_or("AddFont 之后没有负载")?;
                reply_err(writer, "STK-RND-0205", "字体必须在 LoadScript 之前注入")
                    .map_err(|e| e.to_string())?;
            }
            Request::Render { pts_ms } => {
                frames += 1;
                tracker.enter(renderer_host::crash::Phase::Render, Some(frames));
                match render_at(sink, pts_ms, output) {
                    Ok((frame, digest, detect_change, images, loss)) => {
                        tracker.note_ok("render");
                        let bytes = frame.rgba.len();
                        reply(
                            writer,
                            &Response::Frame {
                                width: frame.canvas.width,
                                height: frame.canvas.height,
                                bytes,
                                digest,
                                detect_change,
                                images,
                                loss,
                            },
                        )
                        .map_err(|e| e.to_string())?;
                        for chunk in frame.rgba.chunks(protocol::FRAME_CHUNK_BYTES) {
                            protocol::write_message(writer, &Message::Blob(chunk.to_vec()))
                                .map_err(|e| e.to_string())?;
                        }
                        writer.flush().map_err(|e| e.to_string())?;
                    }
                    Err((code, title)) => {
                        reply_err(writer, &code, &title).map_err(|e| e.to_string())?
                    }
                }
            }
            Request::Bye => {
                tracker.enter(renderer_host::crash::Phase::Shutdown, None);
                reply(writer, &Response::ByeOk).map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
    }
    Ok(())
}

/// 出一帧。返回（帧本身, RGBA 的 SHA-256, detect_change, 图层数）。
///
/// 摘要在 host 侧算，是为了让主进程能核"管道里这段字节就是我说的那一帧"；
/// 它同时也是 12 §8 变化检测的输入单位（RGBA hash），不是凭文件名或时间戳猜。
type RenderAtOutput = (
    image_core::Frame,
    String,
    i32,
    usize,
    core_model::loss::LossReport,
);
type RenderAtError = (String, String);

fn render_at(
    sink: &mut FrameSink<'_>,
    pts_ms: i64,
    output: Canvas,
) -> Result<RenderAtOutput, RenderAtError> {
    let t = Time::from_millis(pts_ms).map_err(|e| (e.code_string(), e.title))?;
    // `Backend::render_at` 自己会按链选路；libass 那份需要脚本句柄，
    // CSRI 那份不用（脚本已在 open 时交进去）。
    let r = sink
        .backend
        .render_at(t, output, sink.libass_script)
        .map_err(|e| (e.code_string(), e.title))?;
    let mut h = Sha256::new();
    h.update(&r.frame.rgba);
    Ok((
        r.frame,
        hex(&h.finalize()),
        r.detect_change,
        r.images,
        r.loss,
    ))
}

/// 崩溃注入：每种都对应一类"第三方 provider 会怎么死"。
///
/// 只有宿主进程自己的环境变量 `STK_HOST_CRASH` 能走到这里。它在 `serve()` 的最早阶段
/// 被消费，所以发布二进制里没有任何来自控制面的自杀路径（2026-10-09 审查）。
/// 未知取值直接退出：静默忽略会让测试以为崩溃注入生效了，实际却在正常跑。
fn maybe_inject_crash_from_env(tracker: &renderer_host::crash::Tracker) {
    let Ok(how) = std::env::var(protocol::CRASH_ENV) else {
        return;
    };
    // 22 §6：注入之前先把现场写下来。这就是“真崩溃时也有一份记录”的代理证据 ——
    // 记录本身写不了的地方（访问违例、栈溢出），真实崩溃在那里也一样写不了。
    // 写失败**不算错**：诊断文件的写失败不该把一次注入变成另一种结果。
    let _ = tracker.write(&format!("注入 {how}"));
    match how.as_str() {
        // 3 号：像 assert/abort 那样死掉，没有 Rust panic 信息
        "abort" => std::process::abort(),
        // 77 号：像 provider 自己 exit 的非零码
        "exit" => std::process::exit(77),
        // panic：在 release（panic=abort）下是 abort，在 debug 下是 101——两种都是"host 没了"
        "panic" => panic!("故意的 host panic（AT-RENDER-002）"),
        // 不回复：主进程必须靠超时放手，而不是永远等
        "hang" => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
        other => {
            eprintln!("[renderer-host] 未知的崩溃注入种类：{other}");
            std::process::exit(78)
        }
    }
}
