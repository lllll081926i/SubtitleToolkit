use gui::state::preview::FrameSource;
use gui::state::profile::{CanvasMode, OutputProfile};
use gui::{Message, SubtitleApp};

#[test]
fn stale_preview_cannot_replace_the_current_selection() {
    let mut app = SubtitleApp {
        preview_generation: 2,
        ..Default::default()
    };
    let _ = app.update(Message::PreviewResolved(
        1,
        Box::new(Message::FrameRendered {
            rgba: vec![255; 4],
            width: 1,
            height: 1,
            source: FrameSource::Renderer {
                provider: "old".into(),
            },
        }),
    ));
    assert!(app.preview.rendered_rgba.is_none());
    let _ = app.update(Message::PreviewResolved(
        2,
        Box::new(Message::FrameRendered {
            rgba: vec![255; 4],
            width: 1,
            height: 1,
            source: FrameSource::Renderer {
                provider: "current".into(),
            },
        }),
    ));
    assert!(app.preview.source.is_trustworthy());
}

#[test]
fn cancellation_does_not_unlock_the_running_batch() {
    let mut app = SubtitleApp::default();
    app.task_drawer.is_running = true;
    let token = app.cancel.clone();
    let _ = app.update(Message::CancelQueue);
    assert!(app.task_drawer.is_running);
    assert!(token.load(std::sync::atomic::Ordering::Relaxed));
    let _ = app.update(Message::StartQueue);
    let _ = app.update(Message::RetryFailed);
    assert!(std::sync::Arc::ptr_eq(&token, &app.cancel));
}

#[test]
fn custom_canvas_obeys_the_same_bluray_limit_as_presets() {
    let mut p = OutputProfile::default_bdn(1);
    p.canvas = CanvasMode::Custom(3840, 2160);
    assert!(p.problem().is_some());
    p.canvas = CanvasMode::Custom(0, 1080);
    assert!(p.problem().is_some());
    p.canvas = CanvasMode::Custom(1920, 1080);
    assert!(p.problem().is_none());
}

#[test]
fn video_suggestion_uses_measured_dimensions_and_preserves_cinema_canvas() {
    let mut app = SubtitleApp::default();
    app.inputs.video_assist = Some(gui::state::inputs::VideoAssist {
        path: "reference.mp4".into(),
        fps: None,
        resolution: Some((1280, 720)),
        is_hdr: false,
        summary: String::new(),
    });
    let _ = app.update(Message::ApplyVideoAssist);
    assert_eq!(
        app.profiles.active_profile().unwrap().canvas,
        CanvasMode::Custom(1280, 720)
    );
    app.inputs.video_assist.as_mut().unwrap().resolution = Some((1920, 800));
    let _ = app.update(Message::ApplyVideoAssist);
    assert_eq!(
        app.profiles.active_profile().unwrap().canvas,
        CanvasMode::Custom(1280, 720)
    );
}

#[test]
fn subtitle_metadata_drives_the_real_timeline() {
    let path = std::env::temp_dir().join(format!("stk-timeline-{}.srt", std::process::id()));
    std::fs::write(&path, "1\n00:00:03,000 --> 00:02:05,000\n字幕\n").unwrap();
    let item = gui::state::inputs::InputItem::from_path(0, path.clone());
    assert_eq!(item.duration_ms, Some(125000));
    assert_eq!(item.event_starts, vec![3000]);
    assert_eq!(item.event_count, 1);
    std::fs::remove_file(path).unwrap();
}
