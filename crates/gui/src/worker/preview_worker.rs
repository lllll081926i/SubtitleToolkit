use crate::state::profile::HdrMode;
use core_model::canvas::Canvas;
use core_model::profile::HdrSettings;

/// 一帧的渲染结果 + 它的来源。
///
/// 把来源一并返回是必须的：拿不到 libass 时这里会画一个合成色块，
/// 而没有来源标记的调用方会把它当成真实的渲染结果展示给用户。
pub struct PreviewFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub source: crate::state::preview::FrameSource,
}

/// A/B 对比的渲染请求（05 §4.1）。
///
/// 规格原话：「左半屏显示原版（SDR / 原始尺寸），右半屏显示应用当前 Profile
/// 变换后（HDR 调光 / 16:9 缩放）的渲染画面」。
///
/// 所以 B 侧要**真变换**，而不是把同一张图抄到右边：
/// 实现方式是先把变换应用到脚本文本上，再拿那份文本去渲染。
/// 那与 `hdr` 命令走的是**同一个** `transform_hdr::apply`，不是另写一遍映射。
pub struct AbRequest {
    pub ass_path: std::path::PathBuf,
    pub time_ms: i64,
    /// A 侧画布（原尺寸）
    pub canvas_a: (u32, u32),
    /// B 侧画布（Profile 的目标画布；与 A 相同时只差颜色）
    pub canvas_b: (u32, u32),
    /// Profile 上的 HDR 设置。`Sdr` = 不做颜色变换（两侧像素应当一致）
    pub hdr: HdrMode,
    /// 目标尺寸与脚本 PlayRes 不同时是否等比缩放到 B 画布。
    /// 对应规格里的“16:9 缩放”。
    pub scale_to_canvas: bool,
}

/// 一次 A/B 的结果。两侧都是完整帧，由调用方拼接成左右分屏。
pub struct AbFrames {
    pub a: PreviewFrame,
    pub b: PreviewFrame,
    /// B 侧实际做了什么（进界面，不能默默无闻）。
    pub note: String,
}

/// 异步请求渲染某一时间点的单帧。
pub async fn render_single_frame(
    ass_path: std::path::PathBuf,
    time_ms: i64,
    canvas: (u32, u32),
) -> Result<PreviewFrame, String> {
    render_script(
        move || std::fs::read(&ass_path).map_err(|e| e.to_string()),
        time_ms,
        canvas,
        canvas,
    )
    .await
}

/// A/B 对比：同一时间点渲染两次，B 侧先应用 Profile 的 HDR 变换（05 §4.1）。
///
/// “先变换脚本再渲染”而不是“渲完再改像素”是有意的：
/// 规格里 B 侧要展示的是“应用当前 Profile 变换后的画面”，而 Profile 的变换
/// 在流水线里就是对脚本做的（`transform_hdr::apply`）。
/// 如果在这里改像素，A/B 与真正导出的结果就会是两套颜色 ——
/// 而用户开对比模式的目的正是“预判导出成什么样”。
pub async fn render_ab(req: AbRequest) -> Result<AbFrames, String> {
    let (hdr, canvas_b, scale, path, time, canvas_a) = (
        req.hdr,
        req.canvas_b,
        req.scale_to_canvas,
        req.ass_path,
        req.time_ms,
        req.canvas_a,
    );

    // B 侧：把变换应用到脚本文本上。
    let (b_bytes, note) = if matches!(hdr, HdrMode::Sdr) && !scale {
        // 两侧一样：不去读两遍文件，也不谎称做了什么
        (
            None,
            "未做变换（Profile 是 SDR 且画布相同）：两侧应当一致".to_string(),
        )
    } else {
        let raw = std::fs::read(&path).map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&raw).into_owned();
        let mut doc = codec_ass::AssDocument::parse(&text);
        let mut parts: Vec<String> = Vec::new();

        // 颜色变换
        if !matches!(hdr, HdrMode::Sdr) {
            let settings = hdr_settings(hdr);
            let mut loss = core_model::LossReport::new();
            let applied = transform_hdr::apply(&mut doc, &settings, &mut loss)
                .map_err(|e| format!("{}：{}", e.code_string(), e.title))?;
            // “改了 0 个颜色”是一个**有效结果**，不是什么都没做：
            //
            // `ReferenceWhite(203)` 恰好就是 SDR 参考白（`transform_hdr::SDR_REFERENCE_NITS`），
            // 所以它的映射是**恒等**——与 11 §2 “SDR 参考白即基准”一致。
            // 而 GUI 默认 profile 里就是 203，于是新用户第一次打开 A/B
            // 会看到两张一样的图。不说清楚，他会以为对比功能坏了；
            // 而他真正该做的是把参考白调成实际的目标 nit。
            if applied.style_colors == 0 && applied.inline_colors == 0 {
                parts.push(format!(
                    "HDR {} 的映射是恒等（203 nit 就是 SDR 基准）：颜色不变",
                    hdr_label(hdr)
                ));
            } else {
                parts.push(format!(
                    "HDR {}：改了 {} 个样式色 + {} 个内联色",
                    hdr_label(hdr),
                    applied.style_colors,
                    applied.inline_colors
                ));
            }
        }

        // 尺寸变换：Profile 的目标画布与脚本 PlayRes 不同时等比缩放。
        //
        // 用“改写脚本的 PlayRes + 几何缩放”而不是“缩放渲染出的像素”：
        // 后者会把字也拉变形，而真正导出时改变的是脚本坐标系（14 §）。
        if scale && canvas_b != canvas_a {
            // 脚本没写 PlayRes 时用 A 侧画布当基准：
            // 缺 PlayRes 的 ASS 由渲染器回退到默认尺寸，而那个默认就是存储画布。
            let before = doc
                .play_res()
                .unwrap_or((canvas_a.0 as i64, canvas_a.1 as i64));
            let (bx, by) = (
                u32::try_from(before.0).unwrap_or(canvas_a.0),
                u32::try_from(before.1).unwrap_or(canvas_a.1),
            );
            let g = core_model::geometry::Geometry {
                canvas: Canvas::new(canvas_b.0, canvas_b.1).unwrap_or(Canvas::FHD1080),
                canvas_is_custom: true,
                script: Some(core_model::geometry::ScriptSize { x: bx, y: by }),
                layout: Some(core_model::geometry::LayoutRes { x: bx, y: by }),
                storage: Some(core_model::geometry::StorageSize {
                    width: canvas_b.0,
                    height: canvas_b.1,
                }),
                par: core_model::geometry::PixelAspect::square(),
                // 09 §9：Rewrite 才改脚本文本（B 侧要展示的正是“改完之后”）。
                script_policy: core_model::geometry::ScriptResolutionPolicy::Rewrite,
                layout_policy: core_model::geometry::LayoutResPolicy::ScaleWithScript,
            };
            let _ = transform_resolution::apply(&mut doc, &g, &mut core_model::LossReport::new());
            parts.push(format!(
                "尺寸 {}x{} → {}x{}",
                bx, by, canvas_b.0, canvas_b.1
            ));
        }

        let note = if parts.is_empty() {
            "未做变换".to_string()
        } else {
            parts.join("；")
        };
        (Some(doc.render().into_bytes()), note)
    };

    // A 侧：原版，不变
    let a = render_script(
        move || std::fs::read(&path).map_err(|e| e.to_string()),
        time,
        canvas_a,
        canvas_a,
    )
    .await?;

    let b = match b_bytes {
        Some(bytes) => render_script(move || Ok(bytes), time, canvas_b, canvas_b).await?,
        // 没做变换：B 直接用 A 的像素。**这是诚实的** ——
        // 注记已经说明“未做变换”，而重渲一遍只会得到同样的像素（多花一次 host 启动）。
        None => PreviewFrame {
            rgba: a.rgba.clone(),
            width: a.width,
            height: a.height,
            source: a.source.clone(),
        },
    };

    Ok(AbFrames { a, b, note })
}

fn hdr_label(m: HdrMode) -> String {
    match m {
        HdrMode::Sdr => "关".to_string(),
        HdrMode::Percentage(p) => format!("兼容减半 {p}%"),
        HdrMode::ReferenceWhite(n) => format!("参考白 {n} nit"),
    }
}

/// GUI 的 `HdrMode` → 核心的 `HdrSettings`。
///
/// 只在这里做一次映射：两处各自算一遍百分比/参考白，就会出现
/// “预览与导出颜色不一样”而两边都自认为对的情况。
///
/// **必须用核心的构造函数而不是结构体字面量**：`HdrSettings` 里
/// `touch_styles` / `touch_inline` 默认为 `false`（那是给“只想看报告”
/// 这一类调用留的），而用 `..Default::default()` 拼出来的设置会
/// **一个颜色都不改**，却仍然报告成功。
/// 本函数的第一版就是这么写的，而 A/B 探针当场抳出了它：
/// note 显示“改了 0 个样式色 + 0 个内联色”，两侧像素逐字节相同。
fn hdr_settings(m: HdrMode) -> HdrSettings {
    match m {
        HdrMode::Sdr => HdrSettings::off(),
        // GUI 存的是整数百分比；核心用万分比（11 §2）
        HdrMode::Percentage(p) => {
            HdrSettings::percentage(p.saturating_mul(100)).unwrap_or_else(|_| HdrSettings::off())
        }
        HdrMode::ReferenceWhite(nits) => {
            HdrSettings::reference_white(nits).unwrap_or_else(|_| HdrSettings::off())
        }
    }
}

/// 共用的单帧渲染：拿字节、起 host、出一帧。
///
/// `bytes` 用闭包而不是 `Vec<u8>`：单帧路径不需要先把整个脚本读进内存
/// （它可能带内嵌字体），而 A/B 需要（它要先改再渲）。两条路径共用一个函数
/// 才能保证“预览用的就是那次导出的渲染器与参数”。
async fn render_script(
    bytes: impl FnOnce() -> Result<Vec<u8>, String> + Send + 'static,
    time_ms: i64,
    storage: (u32, u32),
    output: (u32, u32),
) -> Result<PreviewFrame, String> {
    // 复用 CLI 的 host 定位规则（一条规则一处实现）：
    // 自己再找一遍，就会出现"CLI 找得到、GUI 找不到"这种难查的分歧。
    let host_exe = crate::worker::doctor_worker::renderer_host_exe();

    let dll_path = std::env::var("STK_LIBASS_DLL")
        .map(std::path::PathBuf::from)
        .ok()
        .filter(|p| p.is_file())
        .or_else(|| {
            let guess = std::path::PathBuf::from("third_party/libass-9.dll");
            guess.is_file().then_some(guess)
        });

    let (host, dll) =
        match (host_exe, dll_path) {
            (Some(h), Some(d)) => (h, d),
            (None, _) => {
                return Ok(placeholder(
                    output,
                    "找不到 subtitletool-render-host.exe（渲染宿主没打包或不在同目录）",
                ))
            }
            (_, None) => return Ok(placeholder(
                output,
                "找不到 libass DLL：设 STK_LIBASS_DLL 指向它，或把 libass-9.dll 放到 third_party/",
            )),
        };

    let script_bytes = bytes()?;

    // 在后台线程安全启动隔离宿主进行单帧渲染
    tokio::task::spawn_blocking(move || {
        let storage_canvas = Canvas::new(storage.0, storage.1).unwrap_or(Canvas::FHD1080);
        let output_canvas = Canvas::new(output.0, output.1).unwrap_or(Canvas::FHD1080);

        // 22 §6：预览的 host 崩溃也要有现场 —— 预览是用户最常用的一条渲染路径，
        // 而“预览一闪就没了”是最难报告的一类问题。
        let crash = core_model::DataDir::resolve_current_exe().file("host-crashes");
        let spec = renderer_host::client::HostSpec {
            host_exe: &host,
            dll: &dll,
            storage: storage_canvas,
            output: output_canvas,
            default_font: "Arial",
            timeout: std::time::Duration::from_secs(5),
            crash_injection: None,
            crash_dir: crash.as_deref(),
        };

        let mut client = renderer_host::client::Host::start(&spec).map_err(|e| format!("{e:?}"))?;
        client
            .load_script(&script_bytes)
            .map_err(|e| format!("{e:?}"))?;
        let frame = client.render(time_ms).map_err(|e| format!("{e:?}"))?;
        let _ = client.shutdown();
        Ok(PreviewFrame {
            rgba: frame.rgba,
            width: frame.width,
            height: frame.height,
            // 真渲染器出的像素。provider 名字取自 DLL 文件名（它是实际加载的那一个）。
            source: crate::state::preview::FrameSource::Renderer {
                provider: dll
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "libass".to_string()),
            },
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 合成占位帧。**必须携带原因**：不带原因的占位色块会被当成真实预览。
fn placeholder(canvas: (u32, u32), reason: &str) -> PreviewFrame {
    let (w, h) = canvas;
    let mut rgba = vec![30u8; (w * h * 4) as usize];
    // 简单画一个浅色居中色块示意
    for y in (h / 3)..(h * 2 / 3) {
        for x in (w / 3)..(w * 2 / 3) {
            let idx = ((y * w + x) * 4) as usize;
            rgba[idx] = 200;
            rgba[idx + 1] = 200;
            rgba[idx + 2] = 255;
            rgba[idx + 3] = 255;
        }
    }
    PreviewFrame {
        rgba,
        width: w,
        height: h,
        source: crate::state::preview::FrameSource::Placeholder {
            reason: reason.to_string(),
        },
    }
}
