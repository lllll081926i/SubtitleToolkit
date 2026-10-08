//! `subtitletool-render-host.exe`：唯一在这里，第三方 DLL 才进进程（12 §2）。
//!
//! host 的责任边界刻意很窄：握手 → 载脚本 → 按请求出帧，仅此而已。
//! 业务判断（变化检测、编码、校验）全部留在主进程侧，这样 host 崩溃不会带走任何
//! 已经算好的结果，重试也只是重做像素。
//!
//! `Poison` 是给验收测试用的自伤开关（AT-RENDER-002 要的是"第三方 provider 崩了怎么办"，
//! 而本机没有可下载的 VSFilterMod 二进制，所以由我们自己把 host 打死）。

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
        },
    )
}

fn serve() -> Result<(), String> {
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

    // 到这里才真的 LoadLibrary（render-libass 里那套 LOAD_LIBRARY_SEARCH_*，不搜当前目录）
    let mut provider =
        render_libass::Provider::open(&dll_path, storage_canvas, output_canvas, &default_font)
            .map_err(|e| {
                let _ = reply_err(&mut writer, &e.code_string(), &e.title);
                format!("provider 打不开：{}", e.title)
            })?;

    let name = dll_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("provider")
        .to_string();
    let version = format!("0x{:08X}", provider.library_version());
    reply(
        &mut writer,
        &Response::Hello(Handshake {
            provider: "libass".to_string(),
            name,
            version,
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
            Request::Poison { how } => poison(&how),
            Request::Bye => {
                reply(&mut writer, &Response::ByeOk).map_err(|e| e.to_string())?;
                writer.flush().map_err(|e| e.to_string())?;
                return Ok(());
            }
        }
    };

    for (family, font) in pending_fonts {
        if let Err(e) = provider.add_font(&family, &font) {
            reply_err(&mut writer, &e.code_string(), &e.title).map_err(|reply_error| {
                format!("字体注入失败：{}；发送错误响应失败：{reply_error}", e.title)
            })?;
            return Ok(());
        }
    }
    let script = match provider.load_script(&ass) {
        Ok(script) => script,
        Err(e) => {
            reply_err(&mut writer, &e.code_string(), &e.title).map_err(|reply_error| {
                format!("脚本加载失败：{}；发送错误响应失败：{reply_error}", e.title)
            })?;
            return Ok(());
        }
    };
    reply(&mut writer, &Response::ScriptLoaded { bytes: ass.len() }).map_err(|e| e.to_string())?;
    serve_render_requests(&mut reader, &mut writer, &script, output_canvas)
}

fn serve_render_requests(
    reader: &mut impl std::io::Read,
    writer: &mut impl Write,
    script: &render_libass::Script<'_>,
    output: Canvas,
) -> Result<(), String> {
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
            Request::Render { pts_ms } => match render_at(Some(script), pts_ms, output) {
                Ok((frame, digest, detect_change, images, loss)) => {
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
            },
            Request::Poison { how } => poison(&how),
            Request::Bye => {
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
    script: Option<&render_libass::Script<'_>>,
    pts_ms: i64,
    output: Canvas,
) -> Result<RenderAtOutput, RenderAtError> {
    let script = script.ok_or((
        "STK-RND-0205".to_string(),
        "还没加载脚本就要求出帧".to_string(),
    ))?;
    let t = Time::from_millis(pts_ms).map_err(|e| (e.code_string(), e.title))?;
    let r = render_libass::render(script, t, output).map_err(|e| (e.code_string(), e.title))?;
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

/// 测试用的自伤开关：每种都对应一类"第三方 provider 会怎么死"。
fn poison(how: &str) -> ! {
    match how {
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
            eprintln!("[renderer-host] 未知的 poison 种类：{other}");
            std::process::exit(78)
        }
    }
}
