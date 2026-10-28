use super::style;
use crate::icons;
use crate::message::Message;
use crate::state::InputsState;
use iced::widget::{button, checkbox, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

pub fn view<'a>(state: &'a InputsState) -> Element<'a, Message> {
    let header = row![
        text("素材库").size(18),
        Space::with_width(Length::Fill),
        button(
            row![icons::plus(12.0), text("添加文件").size(12)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .style(style::secondary)
        .on_press(Message::AddFilesClicked)
        .padding([4, 8]),
    ]
    .align_y(Alignment::Center);

    let batch_controls = row![
        button(text("全选").size(12))
            .style(style::secondary)
            .on_press(Message::SelectAllInputs(true))
            .padding([2, 6]),
        button(text("全不选").size(12))
            .style(style::secondary)
            .on_press(Message::SelectAllInputs(false))
            .padding([2, 6]),
        Space::with_width(Length::Fill),
        text(format!("共 {} 个文件", state.items.len())).size(12),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut list_col = column![].spacing(6);

    if state.items.is_empty() {
        list_col = list_col.push(
            container(
                column![
                    text("添加第一份字幕").size(18),
                    text("拖入文件，或使用上方添加按钮。\n支持 ASS / SSA / SRT / SUP / XML")
                        .size(12)
                        .color(style::MUTED),
                ]
                .spacing(4)
                .align_x(Alignment::Center),
            )
            .padding([32, 8])
            .center_x(Length::Fill),
        );
    } else {
        for item in &state.items {
            let check =
                checkbox("", item.selected).on_toggle(move |_| Message::InputToggled(item.id));

            let name_btn = button(
                column![
                    text(&item.file_name).size(13),
                    text(format!(
                        "{:?} · {}",
                        item.format,
                        item.play_res
                            .map(|(w, h)| format!("{w} × {h}"))
                            .unwrap_or_else(|| "尺寸待探测".into())
                    ))
                    .size(12),
                ]
                .spacing(2),
            )
            .style(style::secondary)
            .on_press(Message::InputSelected(item.id))
            .width(Length::Fill)
            .padding(4);

            let remove_btn = button(icons::close(12.0))
                .style(style::secondary)
                .on_press(Message::RemoveInput(item.id))
                .padding([4, 6]);

            let item_row = row![check, name_btn, remove_btn]
                .spacing(6)
                .align_y(Alignment::Center);

            let boxed = container(item_row).padding(8).width(Length::Fill).style(
                if state.active_preview_id == Some(item.id) {
                    style::selected
                } else {
                    style::card
                },
            );

            list_col = list_col.push(boxed);
        }
    }

    let mut main_col = column![
        header,
        batch_controls,
        scrollable(list_col).height(Length::Fill)
    ]
    .spacing(10);

    // 智能建议横幅 (Action Banner)
    if let Some(ref suggestion) = state.banner_suggestion {
        let banner = container(
            row![
                text(suggestion).size(12).width(Length::Fill),
                button(text("应用").size(12))
                    .style(style::secondary)
                    .on_press(Message::ApplyVideoAssist)
                    .padding([2, 6]),
                button(icons::close(10.0))
                    .style(style::secondary)
                    .on_press(Message::DismissVideoBanner)
                    .padding([2, 4]),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding(8);
        main_col = main_col.push(banner);
    }

    container(main_col)
        .width(Length::Fixed(272.0))
        .height(Length::Fill)
        .padding(16)
        .style(style::card)
        .into()
}
