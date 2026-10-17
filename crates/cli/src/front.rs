//! 14 §转换管线里 ASS→BDN 与 ASS→SUP **共用的前半链**：
//! 读脚本 → 采样网格 → 真渲染 → 变化检测（tight 裁剪后的事件）。
//!
//! 规格在 14 §ASS→SUP 写得很直接："若同时输出 BDN+SUP，共享 render/cache"。
//! 共享点选在 [`pgs_author::Plan`] 这一层，理由：
//!
//! - 再往下（编码成 PGS / 写成 PNG）两边就分岔了，共享不了；
//! - 再往上（逐帧 RGBA 全画布）共享等于把几十万帧的画布压在内存里；
//! - Plan 里的补丁已经裁剪到包围盒，既是要交付的东西，也是两种输出唯一都需要的判断结果。
//!
//! 另有一条纪律：`exact_colors`、`level`、`min_axis` 这些都是**后半链**的参数，
//! 不进缓存 key；而输入内容、画布、帧率、默认字体、renderer 二进制哈希每一项都会改变
//! 前半链的输出，所以都在 key 里。少一项就是假复用。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use codec_ass::AssDocument;
use core_model::canvas::Canvas;
use core_model::error::{Domain, Result, StkError};
use core_model::loss::LossReport;
use core_model::time::{Fps, Time};
use image_core::Frame;

use crate::run;

/// 一次前半链的输入取值。
#[derive(Clone, Copy)]
pub(crate) struct RenderJob<'a> {
    pub input: &'a Path,
    pub fps: Fps,
    pub canvas: Canvas,
    pub dll: Option<&'a Path>,
    pub default_font: &'a str,
    /// `Some(list)` = 把这份列表交给 libass（17 §7）；`None` = 按系统字体选择渲染。
    pub inject: Option<&'a [InjectFace]>,
}

/// 前半链的产物：两种渲染输出都从这里往下走。
/// 一条"要交给渲染器的字体"（17 §7）。
///
/// 三个字段全部来自同一次字体扫描。`expect_sha` 是扫描时记下的内容哈希：
/// 注入前必须重读文件核对 —— 否则"缓存 key 里钉住的字节"和"真画出来的字节"
/// 就不是同一个东西，而这条链路的意义正是让它们是同一个东西。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InjectOrigin {
    /// 从本机字体库里解析到的那条 face（按路径读文件）
    Library,
    /// 脚本 `[Fonts]` 里自带的内嵌字体（字节已经在解析时拿到）
    Embedded,
}

#[derive(Clone, Debug)]
pub(crate) struct InjectFace {
    /// 注册名，同时也是 face 表里的家族名（libass 按字体内部名匹配内存字体）
    pub family: String,
    /// 库来源时要去读的文件；内嵌来源时只是脚本里写的那个文件名（不读盘）
    pub file: std::path::PathBuf,
    pub expect_sha: String,
    /// 内嵌字体的原件字节；`None` 表示这批字节要从 `file` 读
    pub data: Option<Vec<u8>>,
    pub origin: InjectOrigin,
}

#[derive(Clone)]
pub(crate) struct Rendered {
    /// 变化检测 + tight 裁剪的结果
    pub plan: pgs_author::Plan,
    /// 渲染层自己的记账（采样网格取整、颜色量化、位图裁剪）；调用方并进 LossReport
    pub loss: LossReport,
    /// 采样点个数（报告用，也用来解释"为什么这么慢"）
    pub samples: usize,
    pub grid: run::Grid,
    /// 真正交给 libass 的内存字体清单（`--inject-fonts` 关掉时是空的）
    pub injected: Vec<render_libass::InjectedFont>,
}

/// 真渲染 + 变化检测。单命令路径与批量路径都只调这一个函数，
/// 于是"批量重放"与"单发重放"必然是同一份语义（20 §Repro）。
pub(crate) fn render_to_plan(job: &RenderJob<'_>) -> Result<Rendered> {
    let loaded = crate::pipeline::load_text(job.input)?;
    let doc = AssDocument::parse(&loaded.text);
    let grid = run::sample_grid(job.fps, run::last_end_ms_of(&doc)?)?;

    let mut loss = LossReport::new();
    if grid.rounded {
        loss.approximated(&format!(
            "采样网格量化到整数毫秒：{} 的帧间隔不是整毫秒，帧号→ms 半进位取整（渲染器只接受整数 ms）",
            job.fps.to_label()
        ));
    }

    let dll_path = match job.dll {
        Some(p) => p.to_path_buf(),
        None => run::libass_dll(None)?,
    };
    let host_exe = run::renderer_host_exe().ok_or_else(|| {
        StkError::simple(Domain::Render, 89, "找不到 renderer-host")
            .with_detail("实际渲染必须通过 subtitletool-render-host.exe 隔离运行")
            .with_action("构建 renderer-host 并设置 STK_RENDERER_HOST，或将它放在 CLI 同目录")
    })?;
    let spec = renderer_host::client::HostSpec {
        host_exe: &host_exe,
        dll: &dll_path,
        storage: job.canvas,
        output: job.canvas,
        default_font: job.default_font,
        timeout: std::time::Duration::from_secs(30),
        crash_injection: None,
    };
    let mut host = renderer_host::client::Host::start(&spec).map_err(render_host_error)?;
    // 17 §7：把"我们解析到并读过的那份字节"直接交给 libass。
    // 顺序有讲究：注入必须在 load_script 之前，轨道建立时才会把内存字体算进选择结果。
    let mut injected = Vec::new();
    if let Some(list) = job.inject {
        for f in list {
            // 内嵌字体用脚本里那份字节；库里的 face 才去读盘（两条路都汇到同一个核对）
            let read;
            let bytes: &[u8] = match &f.data {
                Some(b) => b.as_slice(),
                None => {
                    read = std::fs::read(&f.file).map_err(|e| {
                        StkError::simple(Domain::Render, 87, "读不到要注入的字体文件")
                            .with_detail(format!("{}：{e}", f.file.display()))
                    })?;
                    read.as_slice()
                }
            };
            let now = crate::pipeline::sha256_hex(bytes);
            if now != f.expect_sha {
                return Err(StkError::simple(
                    Domain::Render,
                    88,
                    "要注入的字体在扫描之后被换掉了，拒绝按旧身份渲染",
                )
                .with_detail(format!(
                    "{}：扫描时 {}，现在 {}",
                    f.file.display(),
                    f.expect_sha,
                    now
                ))
                .with_action("重跑 font scan 取得新身份，或去掉 --inject-fonts"));
            }
            let count = host.add_font(&f.family, bytes).map_err(render_host_error)?;
            if count != bytes.len() {
                return Err(StkError::simple(
                    Domain::Render,
                    90,
                    "renderer-host 注入字体字节数不符",
                )
                .with_detail(format!(
                    "{}：发送 {} 字节，host 接收 {count} 字节",
                    f.family,
                    bytes.len()
                )));
            }
            injected.push(render_libass::InjectedFont {
                name: f.family.clone(),
                bytes: count,
                sha256: now,
            });
        }
    }
    host.load_script(loaded.text.as_bytes())
        .map_err(render_host_error)?;

    let mut builder = pgs_author::PlanBuilder::new(pgs_author::AuthorOptions {
        stream_end: Time::from_millis(grid.stream_end_ms)?,
    });
    let mut sample_count = 0usize;
    for ms in grid.render_ms() {
        let t = Time::from_millis(*ms)?;
        let rendered = host.render(*ms).map_err(render_host_error)?;
        // 渲染层的量化/裁剪记账必须并进来，否则 CLI 看到的只是后半截
        loss.merge(&rendered.loss);
        let frame = Frame::new(job.canvas, rendered.rgba)?;
        builder.push(pgs_author::Sample::new(t, frame))?;
        sample_count += 1;
    }
    builder.push(pgs_author::Sample::new(
        Time::from_millis(grid.blank_ms)?,
        Frame::empty(job.canvas)?,
    ))?;
    sample_count += 1;
    host.shutdown().map_err(render_host_error)?;
    let plan = builder.finish()?;
    Ok(Rendered {
        plan,
        loss,
        samples: sample_count,
        grid,
        injected,
    })
}

fn render_host_error(error: renderer_host::client::HostFailure) -> StkError {
    let title = error.title();
    StkError::simple(Domain::Render, 91, &title).with_detail(format!("{}：{error:?}", error.code()))
}

/// 一次批量任务内的渲染记忆化。
///
/// key 由调用方（batch）给出，覆盖前半链的全部分量；同 key 的多个 profile 里
/// 只有一个真渲染，其余等它。锁按 slot 分，不同 key 之间不互相阻塞。
#[derive(Default)]
pub(crate) struct RenderMemo {
    slots: Mutex<BTreeMap<String, Arc<Mutex<Option<Rendered>>>>>,
    renders: AtomicUsize,
}

impl RenderMemo {
    /// 失败的渲染不进缓存：那会把一次性的错误变成"这个输入永远失败"。
    pub(crate) fn render(&self, key: &str, job: &RenderJob<'_>) -> Result<Rendered> {
        let slot = {
            let mut g = self.slots();
            g.entry(key.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(None)))
                .clone()
        };
        let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = held.as_ref() {
            return Ok(done.clone());
        }
        // 这里 fetch_add 的语义是"真跑了一次 native 渲染"，报告用它自证共享成立
        self.renders.fetch_add(1, Ordering::Relaxed);
        let out = render_to_plan(job)?;
        *held = Some(out.clone());
        Ok(out)
    }

    /// 真正渲染过几次（共享命中时小于渲染型 profile 的个数）。
    pub(crate) fn renders(&self) -> usize {
        self.renders.load(Ordering::Relaxed)
    }

    /// 用过的 key 个数：与 `renders` 相等就说明每个 key 各渲染了一次，没有互相复用。
    pub(crate) fn keys(&self) -> usize {
        self.slots().len()
    }

    /// 中毒在这里不该演变成新错误：slot 的内容要么是要么不是 `Rendered`，
    /// 渲染线程 panic 后留下的 `None` 交给下一个 profile 重新渲染即可
    /// （panic 归因由 task-engine 的崩溃预算负责，38 §3）。
    fn slots(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<Mutex<Option<Rendered>>>>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// "这次运行处在批量上下文里，去用共享缓存"。
#[derive(Clone)]
pub(crate) struct SharedRender<'a> {
    pub memo: &'a RenderMemo,
    pub key: String,
}

impl<'a> SharedRender<'a> {
    pub(crate) fn obtain(&self, job: &RenderJob<'_>) -> Result<Rendered> {
        self.memo.render(&self.key, job)
    }
}

/// 前半链的取法：批量走共享缓存，单命令自己跑。
pub(crate) fn resolve(shared: Option<&SharedRender<'_>>, job: &RenderJob<'_>) -> Result<Rendered> {
    match shared {
        Some(s) => s.obtain(job),
        None => render_to_plan(job),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_model::time::Fps;

    fn dll_path() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("STK_LIBASS_DLL") {
            let p = std::path::PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
            return None;
        }
        let guess = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("..")
            .join("tools")
            .join("vcpkg")
            .join("installed")
            .join("x64-windows")
            .join("bin")
            .join("ass-9.dll");
        guess.is_file().then_some(guess)
    }

    const ASS: &str = "[Script Info]\nScriptType: v4.00+\nPlayResX: 1280\nPlayResY: 720\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: D,Arial,72,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,3,1,2,20,20,40,1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:02.00,D,,0,0,0,,Hi\n";

    /// 注入前的那道核对是唯一保证"缓存 key 里的字节 == 画出来的字节"的东西：
    /// 文件在扫描之后被换掉（覆盖安装、并行改字体）时必须停下来报错，
    /// 而不是拿着旧身份渲染出新像素。
    #[test]
    fn a_font_replaced_after_the_scan_refuses_to_be_injected() {
        let Some(dll) = dll_path() else {
            if std::env::var("STK_REQUIRE_LIBASS").is_ok() {
                panic!("STK_REQUIRE_LIBASS=1 但找不到 ass-9.dll");
            }
            eprintln!("SKIP 注入核对：本机没有 libass DLL");
            return;
        };
        let dir = std::env::temp_dir().join(format!("stk-inject-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ass = dir.join("a.ass");
        std::fs::write(&ass, ASS).unwrap();
        // 拿一份真字体当"被注入的文件"，然后把 expect_sha 写成旧的（全 0）
        let sys = std::path::PathBuf::from(std::env::var("WINDIR").unwrap())
            .join("Fonts")
            .join("arial.ttf");
        let copy = dir.join("arial.ttf");
        std::fs::copy(&sys, &copy).expect("本机该有 arial.ttf");
        let plan = [InjectFace {
            family: "Arial".to_string(),
            file: copy.clone(),
            expect_sha: "0".repeat(64),
            data: None,
            origin: InjectOrigin::Library,
        }];
        let job = RenderJob {
            input: &ass,
            fps: Fps::from_label("25").unwrap(),
            canvas: core_model::canvas::Canvas::HD720,
            dll: Some(dll.as_path()),
            default_font: "Arial",
            inject: Some(&plan),
        };
        let Err(e) = render_to_plan(&job) else {
            panic!("身份与文件现状不符时必须拒绝注入，而不是照着旧身份渲染");
        };
        assert_eq!(e.code_string(), "STK-RND-0088", "{e:?}");
        assert!(
            e.user_action
                .as_deref()
                .unwrap_or_default()
                .contains("font scan"),
            "要给出能执行的补法：{e:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
