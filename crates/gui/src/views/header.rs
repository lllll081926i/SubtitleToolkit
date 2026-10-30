use super::style;
use crate::icons;
use crate::message::Message;
use iced::widget::{button, column, container, row, text, Space};
use iced::{Alignment, Element, Length};

pub fn view<'a>() -> Element<'a, Message> {
    let title = column![
        text("SUBTITLE STUDIO").size(12).color(style::ACCENT),
        text("字幕工作台").size(24)
    ]
    .spacing(4);

    let settings_btn = button(
        row![icons::settings(14.0), text("设置").size(12),]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .style(style::secondary)
    .on_press(Message::OpenSettings)
    .padding([5, 12]);
    let doc_btn = button(text("环境自检").size(12))
        .style(style::secondary)
        .on_press(Message::OpenDoctorDialog)
        .padding([5, 12]);

    let content = row![
        title,
        Space::with_width(Length::Fixed(24.0)),
        Space::with_width(Length::Fill),
        settings_btn,
        doc_btn,
    ]
    .padding([8, 4])
    .spacing(10)
    .align_y(Alignment::Center);

    container(content).width(Length::Fill).into()
}
