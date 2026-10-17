use crate::icons;
use crate::message::Message;
use crate::state::profile::{CanvasMode, HdrMode, ProfilesState, TargetFormat};
use iced::widget::{button, checkbox, column, container, pick_list, row, scrollable, text, Space};
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

pub fn view<'a>(state: &'a ProfilesState) -> Element<'a, Message> {
    let header = row![
        text("输出 Profile 矩阵").size(16),
        Space::with_width(Length::Fill),
        button(
            row![icons::plus(12.0), text("新建 Profile").size(12)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
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
                text(profile.target_format.label()).size(11),
            ]
            .align_y(Alignment::Center),
        )
        .on_press(Message::ProfileSelected(id))
        .width(Length::Fill)
        .padding(4);

        let dup_btn = button(icons::copy(12.0))
            .on_press(Message::DuplicateProfile(id))
            .padding([4, 6]);

        let remove_btn = button(icons::close(12.0))
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

            let options_col = column![
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
                    text("合规指示:").size(11).width(Length::Fixed(70.0)),
                    text(if profile.target_format == TargetFormat::Pgs {
                        "Basic PGS: 具备 (自适应量化)"
                    } else {
                        "标准交换格式 (无合规锁死)"
                    })
                    .size(11),
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(6);

            card_content = card_content.push(options_col);
        }

        let boxed_card = container(card_content).padding(10).width(Length::Fill);

        profiles_col = profiles_col.push(boxed_card);
    }

    let main_col = column![header, scrollable(profiles_col).height(Length::Fill)].spacing(10);

    container(main_col)
        .width(Length::Fixed(360.0))
        .height(Length::Fill)
        .padding(12)
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
            Self::Sdr => write!(f, "SDR (原样保持)"),
            Self::ReferenceWhite(n) => write!(f, "参考白 ({} nit 近似)", n),
            Self::Percentage(p) => write!(f, "线性光缩放 ({}%)", p),
        }
    }
}
