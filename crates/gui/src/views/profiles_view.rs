use super::style;
use crate::icons;
use crate::message::Message;
use crate::state::profile::{CanvasMode, HdrMode, ProfilesState, TargetFormat, TimeAdjustMode};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, scrollable, text, text_input, Space,
};
use iced::{Alignment, Element, Length};

const FORMATS: &[TargetFormat] = &[
    TargetFormat::BdnXml,
    TargetFormat::Pgs,
    TargetFormat::Ass,
    TargetFormat::Srt,
];

const CANVASES: &[CanvasMode] = &[
    CanvasMode::KeepOriginal,
    CanvasMode::Fhd1080,
    CanvasMode::Uhd2160,
    CanvasMode::Hd720,
];

const HDRS: &[HdrMode] = &[
    HdrMode::Sdr,
    HdrMode::ReferenceWhite(203),
    HdrMode::Percentage(50),
];

pub fn view<'a>(state: &'a ProfilesState, notice: &'a str) -> Element<'a, Message> {
    let header = row![
        text("输出方案").size(22),
        button(text("导入").size(12))
            .style(style::secondary)
            .on_press(Message::ImportPreset)
            .padding([4, 7]),
        button(text("导出").size(12))
            .style(style::secondary)
            .on_press(Message::ExportPreset)
            .padding([4, 7]),
        Space::with_width(Length::Fill),
        button(
            row![icons::plus(12.0), text("新建方案").size(12)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .style(style::secondary)
        .on_press(Message::AddProfile)
        .padding([4, 8]),
    ]
    .align_y(Alignment::Center);

    let mut profiles_col = column![].spacing(10);

    for profile in &state.profiles {
        let is_editing = state.active_edit_id == Some(profile.id);
        let id = profile.id;

        let check = checkbox("", profile.enabled).on_toggle(move |_| Message::ProfileToggled(id));

        let title_btn = button(
            row![
                text(&profile.name).size(13),
                Space::with_width(Length::Fill),
                text(profile.target_format.label()).size(12),
            ]
            .align_y(Alignment::Center),
        )
        .style(style::secondary)
        .on_press(Message::ProfileSelected(id))
        .width(Length::Fill)
        .padding(4);

        let dup_btn = button(icons::copy(12.0))
            .style(style::secondary)
            .on_press(Message::DuplicateProfile(id))
            .padding([4, 6]);

        let remove_btn = button(icons::close(12.0))
            .style(style::secondary)
            .on_press(Message::RemoveProfile(id))
            .padding([4, 6]);

        let top_row = row![check, title_btn, dup_btn, remove_btn]
            .spacing(6)
            .align_y(Alignment::Center);

        let mut card_content = column![top_row].spacing(8);

        // 如果处于展开编辑状态，显示参数链路选项
        if is_editing {
            let format_pick = pick_list(FORMATS, Some(profile.target_format), move |fmt| {
                Message::FormatChanged(id, fmt)
            });

            let canvas_pick = pick_list(CANVASES, Some(profile.canvas), move |c| {
                Message::CanvasChanged(id, c)
            });

            let hdr_pick = pick_list(HDRS, Some(profile.hdr), move |h| Message::HdrChanged(id, h));

            let time_options = [
                TimeAdjustMode::None,
                TimeAdjustMode::OffsetMs(-1000),
                TimeAdjustMode::OffsetMs(-500),
                TimeAdjustMode::OffsetMs(500),
                TimeAdjustMode::OffsetMs(1000),
            ];
            let options_col = column![
                text_input("方案名称（不可为空）", &profile.name)
                    .on_input(move |name| Message::ProfileRenamed(id, name))
                    .padding(10),
                row![
                    text("时间调整").width(Length::Fixed(90.0)),
                    pick_list(time_options, Some(profile.time_adjust), move |v| {
                        Message::TimeAdjustChanged(id, v)
                    })
                ]
                .align_y(Alignment::Center),
                row![
                    text("目标格式:").size(12).width(Length::Fixed(70.0)),
                    format_pick
                ]
                .align_y(Alignment::Center),
                row![
                    text("逻辑画布:").size(12).width(Length::Fixed(70.0)),
                    canvas_pick
                ]
                .align_y(Alignment::Center),
                row![
                    text("HDR映射:").size(12).width(Length::Fixed(70.0)),
                    hdr_pick
                ]
                .align_y(Alignment::Center),
                row![
                    text("状态:").size(12).width(Length::Fixed(70.0)),
                    text(if profile.problem().is_none() {
                        "可用"
                    } else {
                        "请检查参数"
                    })
                    .size(12),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(12);

            card_content = card_content.push(options_col);
        }

        if let Some(why) = profile.problem() {
            let warn = column![
                text("此配置无法运行").size(12),
                text(why.lines().next().unwrap_or("画布超出蓝光上限").to_string()).size(12),
            ]
            .spacing(4);

            card_content = card_content.push(
                container(warn)
                    .padding(8)
                    .width(Length::Fill)
                    .style(container::bordered_box),
            );
        }

        let boxed_card = container(card_content)
            .padding(20)
            .width(Length::Fill)
            .style(if is_editing {
                style::selected
            } else {
                style::card
            });

        profiles_col = profiles_col.push(boxed_card);
    }

    let mut main_col = column![
        header,
        text("勾选需要导出的方案；点击方案展开参数。每份输入可同时生成多种输出。")
            .size(13)
            .color(style::MUTED)
    ]
    .spacing(16);
    if !notice.is_empty() {
        main_col = main_col.push(text(notice).size(12));
    }
    let main_col = main_col.push(scrollable(profiles_col).height(Length::Fill));

    container(main_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(20)
        .style(style::card)
        .into()
}

impl std::fmt::Display for TargetFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

impl std::fmt::Display for CanvasMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

impl std::fmt::Display for HdrMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sdr => write!(f, "SDR"),
            Self::ReferenceWhite(n) => write!(f, "参考白 {} nit", n),
            Self::Percentage(p) => write!(f, "线性缩放 {}%", p),
        }
    }
}

impl std::fmt::Display for TimeAdjustMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "保持原时间轴"),
            Self::OffsetMs(ms) => write!(f, "偏移 {ms:+} ms"),
            Self::SpeedScale { from, to } => write!(f, "变速 {from} → {to}"),
        }
    }
}
