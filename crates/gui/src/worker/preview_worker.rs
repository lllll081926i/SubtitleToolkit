use core_model::canvas::Canvas;

/// 异步请求渲染某一时间点的单帧
pub async fn render_single_frame(
    ass_path: std::path::PathBuf,
    time_ms: i64,
    canvas: (u32, u32),
) -> Result<(Vec<u8>, u32, u32), String> {
    // 检查宿主可执行程序与 libass 运行库
    let host_exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("subtitletool-render-host.exe")));

    let dll_path = std::env::var("STK_LIBASS_DLL")
        .map(std::path::PathBuf::from)
        .ok()
        .or_else(|| {
            let guess = std::path::PathBuf::from("third_party/libass-9.dll");
            guess.is_file().then_some(guess)
        });

    let (Some(host), Some(dll)) = (host_exe, dll_path) else {
        // 环境暂无 libass，返回合成占位帧（测试与无外部依赖降级路径）
        let (w, h) = canvas;
        let mut mock_rgba = vec![30u8; (w * h * 4) as usize];
        // 简单画一个浅色居中色块示意
        for y in (h / 3)..(h * 2 / 3) {
            for x in (w / 3)..(w * 2 / 3) {
                let idx = ((y * w + x) * 4) as usize;
                mock_rgba[idx] = 200;
                mock_rgba[idx + 1] = 200;
                mock_rgba[idx + 2] = 255;
                mock_rgba[idx + 3] = 255;
            }
        }
        return Ok((mock_rgba, w, h));
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
        Ok((frame.rgba, frame.width, frame.height))
    })
    .await
    .map_err(|e| e.to_string())?
}
