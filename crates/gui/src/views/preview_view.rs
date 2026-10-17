use crate::icons;
use crate::message::Message;
use crate::state::PreviewState;
use iced::widget::{button, column, container, image, row, slider, text, Space};
use iced::{Alignment, Element, Length};

pub fn view<'a>(state: &'a PreviewState) -> Element<'a, Message> {
    let time_sec = state.current_time_ms as f64 / 1000.0;
    let dur_sec = state.duration_ms as f64 / 1000.0;

    let time_code = format!(
        "{:02}:{:02}:{:02}.{:03} / {:02}:{:02}:{:02}",
        (time_sec / 3600.0) as u32,
        ((time_sec % 3600.0) / 60.0) as u32,
        (time_sec % 60.0) as u32,
        (state.current_time_ms % 1000) as u32,
        (dur_sec / 3600.0) as u32,
        ((dur_sec % 3600.0) / 60.0) as u32,
        (dur_sec % 60.0) as u32,
    );

    let top_bar = row![
        text("实时单帧渲染预览 (Live Preview)").size(15),
        Space::with_width(Length::Fill),
        // 像素来源必须一直可见：占位色块与真渲染在画面上分辨不出来，
        // 而它们的含义完全不同（前者是我们编的颜色，后者是可交付的根据）。
        text(state.source.label()).size(12),
        Space::with_width(Length::Fixed(12.0)),
        text(time_code).size(13),
        Space::with_width(Length::Fixed(12.0)),
        button(text(if state.is_split_ab {
            "A/B对比: 开"
        } else {
            "A/B对比: 关"
        }))
        .on_press(Message::ToggleSplitAb)
        .padding([4, 10]),
    ]
    .align_y(Alignment::Center);

    // 画面展示区域
    let canvas_box: Element<'a, Message> = if let Some(ref rgba) = state.rendered_rgba {
        let (w, h) = state.rendered_dims;
        let handle = image::Handle::from_rgba(w, h, rgba.clone());
        container(image(handle).width(Length::Fill).height(Length::Fill))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    } else {
        container(
            column![
                text(if state.is_rendering {
                    "正在通过 renderer-host 渲染单帧..."
                } else {
                    "暂无预览画面（选择左侧输入文件并定位时间轴）"
                })
                .size(13),
                text("16:9 逻辑画布保障 | 毫秒级单帧返回").size(11),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    };

    // 时间轴与对白控制
    let time_slider = slider(
        0.0..=(state.duration_ms.max(1) as f32),
        state.current_time_ms as f32,
        |v| Message::PreviewTimeChanged(v as i64),
    );

    let controls = row![
        button(
            row![icons::skip_back(14.0), text("上一句").size(12)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .on_press(Message::PreviewJumpEvent(false))
        .padding([4, 10]),
        Space::with_width(Length::Fixed(8.0)),
        time_slider,
        Space::with_width(Length::Fixed(8.0)),
        button(
            row![text("下一句").size(12), icons::skip_forward(14.0)]
                .spacing(4)
                .align_y(Alignment::Center),
        )
        .on_press(Message::PreviewJumpEvent(true))
        .padding([4, 10]),
    ]
    .align_y(Alignment::Center);

    // 底部字体诊断卡片
    let font_card = container(
        row![
            icons::activity(12.0),
            text("字体诊断:").size(11),
            text(if state.font_health_summary.is_empty() {
                "等待载入字体检查..."
            } else {
                "系统字体完整就绪，无缺字"
            })
            .size(11),
            Space::with_width(Length::Fill),
            text("DirectWrite 真实字符覆盖核准").size(10),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(6);

    let content = column![
        top_bar,
        container(canvas_box)
            .width(Length::Fill)
            .height(Length::Fill),
        controls,
        font_card,
    ]
    .spacing(10);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12)
        .into()
}
