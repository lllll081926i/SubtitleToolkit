use gui::SubtitleApp;

fn main() -> iced::Result {
    let level = std::env::var("STK_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            core_model::SettingsStore::current()
                .load()
                .settings
                .log_level
        });
    core_model::init_logging(&level);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), log_level = %level, "GUI 启动");

    let result = iced::application(SubtitleApp::title, SubtitleApp::update, SubtitleApp::view)
        .theme(SubtitleApp::theme)
        .subscription(SubtitleApp::subscription)
        // 默认 SansSerif 在部分 Windows 安装上没有 CJK 覆盖；明确选用 Windows
        // 自带的中文 UI 字体，避免界面中文与 Unicode 图标显示成空框。
        .default_font(iced::Font::with_name("Microsoft YaHei UI"))
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            min_size: Some(iced::Size::new(1100.0, 720.0)),
            ..Default::default()
        })
        .run_with(SubtitleApp::new);
    match &result {
        Ok(()) => tracing::info!("GUI 正常退出"),
        Err(error) => tracing::error!(error = %error, "GUI 运行失败"),
    }
    result
}
