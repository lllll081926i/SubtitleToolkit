use core_model::canvas::Canvas;

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

/// 异步请求渲染某一时间点的单帧。
pub async fn render_single_frame(
    ass_path: std::path::PathBuf,
    time_ms: i64,
    canvas: (u32, u32),
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
                    canvas,
                    "找不到 subtitletool-render-host.exe（渲染宿主没打包或不在同目录）",
                ))
            }
            (_, None) => return Ok(placeholder(
                canvas,
                "找不到 libass DLL：设 STK_LIBASS_DLL 指向它，或把 libass-9.dll 放到 third_party/",
            )),
        };

    let script_bytes = std::fs::read(&ass_path).map_err(|e| e.to_string())?;

    // 在后台线程安全启动隔离宿主进行单帧渲染
    tokio::task::spawn_blocking(move || {
        let storage_canvas = Canvas::new(canvas.0, canvas.1).unwrap_or(Canvas::FHD1080);
        let output_canvas = Canvas::new(canvas.0, canvas.1).unwrap_or(Canvas::FHD1080);

        let spec = renderer_host::client::HostSpec {
            host_exe: &host,
            dll: &dll,
            storage: storage_canvas,
            output: output_canvas,
            default_font: "Arial",
            timeout: std::time::Duration::from_secs(5),
            crash_injection: None,
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
