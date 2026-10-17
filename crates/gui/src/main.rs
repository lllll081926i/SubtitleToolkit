use gui::SubtitleApp;

fn main() -> iced::Result {
    iced::application(SubtitleApp::title, SubtitleApp::update, SubtitleApp::view)
        .theme(SubtitleApp::theme)
        .window(iced::window::Settings {
            size: iced::Size::new(1440.0, 900.0),
            min_size: Some(iced::Size::new(1100.0, 720.0)),
            ..Default::default()
        })
        .run_with(SubtitleApp::new)
}
