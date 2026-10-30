//! 工作台共用的颜色、圆角与交互状态，避免各页面自行漂移。
use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow, Theme, Vector};
pub const INK: Color = Color::from_rgb(0.12, 0.17, 0.25);
pub const MUTED: Color = Color::from_rgb(0.39, 0.45, 0.54);
pub const ACCENT: Color = Color::from_rgb(0.27, 0.34, 0.88);
pub fn theme() -> Theme {
    Theme::custom(
        "Studio".into(),
        iced::theme::Palette {
            background: Color::from_rgb(0.95, 0.96, 0.98),
            text: INK,
            primary: ACCENT,
            success: Color::from_rgb(0.12, 0.57, 0.43),
            danger: Color::from_rgb(0.78, 0.20, 0.28),
        },
    )
}
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::WHITE.into()),
        text_color: Some(INK),
        border: Border {
            color: Color::from_rgb(0.87, 0.89, 0.93),
            width: 1.0,
            radius: 16.0.into(),
        },
        ..Default::default()
    }
}
pub fn selected(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb(0.94, 0.95, 1.0).into()),
        border: Border {
            color: ACCENT,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..card(theme)
    }
}
pub fn stage(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgb(0.08, 0.11, 0.17).into()),
        text_color: Some(Color::from_rgb(0.72, 0.77, 0.86)),
        border: Border {
            radius: 14.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}
pub fn scrim(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Color::from_rgba(0.04, 0.07, 0.12, 0.55).into()),
        ..Default::default()
    }
}
pub fn secondary(theme: &Theme, status: button::Status) -> button::Style {
    let mut s = button::secondary(theme, status);
    s.border.radius = 9.0.into();
    s.text_color = INK;
    s.background = Some(Background::Color(match status {
        button::Status::Hovered => Color::from_rgb(0.87, 0.90, 0.98),
        button::Status::Pressed => Color::from_rgb(0.81, 0.85, 0.96),
        _ => Color::from_rgb(0.94, 0.95, 0.98),
    }));
    if matches!(status, button::Status::Disabled) {
        s.text_color = MUTED;
    }
    s
}
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let mut s = button::primary(theme, status);
    s.border.radius = 10.0.into();
    if matches!(status, button::Status::Hovered) {
        s.shadow = Shadow {
            color: Color::from_rgba(0.27, 0.34, 0.88, 0.2),
            offset: Vector::new(0.0, 3.0),
            blur_radius: 10.0,
        };
    }
    s
}
