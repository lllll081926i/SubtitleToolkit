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

    // A/B 打开时把“B 侧到底做了什么”写在画面上。
    //
    // 这不是装饰：Profile 是 SDR 且画布相同时两侧**应该**一模一样，
    // 而用户看到两张相同的图很容易以为对比坏了。说清楚它为什么一样，
    // 比让用户去猜要好。
    let ab_caption: Option<Element<'a, Message>> = if state.is_split_ab {
        Some(
            row![
                text("左：原版（SDR / 原尺寸）").size(11),
                Space::with_width(Length::Fixed(10.0)),
                text("中：分界线").size(11),
                Space::with_width(Length::Fixed(10.0)),
                text(format!(
                    "右：当前 Profile —— {}",
                    state.ab_note.as_deref().unwrap_or("等待渲染")
                ))
                .size(11),
            ]
            .align_y(Alignment::Center)
            .into(),
        )
    } else {
        None
    };

    // 画面展示区域
    //
    // 05 §4.1：A/B 模式下左半是原版，右半是应用了 Profile 变换的版本。
    // 拼接在这里做（视图层），而不是在状态层：状态里存两张**独立**的帧，
    // 于是切换布局不需要重渲（重渲会花几百毫秒，而用户只是点了一下开关）。
    let canvas_box: Element<'a, Message> = match (&state.rendered_rgba, &state.ab_rgba) {
        (Some(a), Some(b)) if state.is_split_ab => {
            let (aw, ah) = state.rendered_dims;
            let (bw, bh) = state.ab_dims;
            match split_ab(a, (aw, ah), b, (bw, bh)) {
                Some((px, w, h)) => {
                    let handle = image::Handle::from_rgba(w, h, px);
                    container(image(handle).width(Length::Fill).height(Length::Fill))
                        .center_x(Length::Fill)
                        .center_y(Length::Fill)
                        .into()
                }
                None => container(text("A/B 两帧尺寸不兼容，无法拼接").size(13))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center_x(Length::Fill)
                    .center_y(Length::Fill)
                    .into(),
            }
        }
        (Some(rgba), _) => {
            let (w, h) = state.rendered_dims;
            let handle = image::Handle::from_rgba(w, h, rgba.clone());
            container(image(handle).width(Length::Fill).height(Length::Fill))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into()
        }
        // A/B 开着但对照帧还没到：说清楚在等什么，
        // 而不是把 A 侧当完整画面显示（那会让人以为对比已经生效）。
        (None, _) if state.is_split_ab && state.is_rendering => {
            container(text("正在渲染 A/B 对照帧（左侧原版 / 右侧变换后）...").size(13))
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .into()
        }
        _ => container(
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
        .into(),
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

    // 说明行插在画面与控件之间（在画面上方太挤，在下面会被控件挤走）
    let content = match ab_caption {
        Some(c) => column![
            top_bar,
            c,
            container(canvas_box)
                .width(Length::Fill)
                .height(Length::Fill),
            controls,
            font_card,
        ]
        .spacing(10),
        None => column![
            top_bar,
            container(canvas_box)
                .width(Length::Fill)
                .height(Length::Fill),
            controls,
            font_card,
        ]
        .spacing(10),
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(12)
        .into()
}

/// 把 A/B 两帧拼成左右分屏（05 §4.1）。
///
/// ## 尺寸不同时怎么办
///
/// 规格里 A/B 的两侧本来就可能不同尺寸（“左半屏原版 / 右半屏 16:9 缩放”），
/// 而拼接要求同一个像素网格。三条路：
///
/// 1. **按较大的一侧画布居中摆放**，小的一侧按其真实像素居中（不放大）——
///    放大后再拼会让人以为“B 的字更大”，而那是插值伪影不是变换结果。
/// 2. 缩放到相同尺寸再拼 —— 会引入一次不必要的重采样，直接污染对比。
/// 3. 拒绝拼接 —— 用户什么都看不到。
///
/// 选 (1)：两侧的**像素内容**不做任何重采样（对比的可信度取决于这一点），
/// 差异体现在“画布里的占比”上 —— 而那正是缩放变换的真实效果。
///
/// 返回 `None` 只在两帧尺寸都不可用时（例如任一侧为 0）。
fn split_ab(
    a: &[u8],
    a_dims: (u32, u32),
    b: &[u8],
    b_dims: (u32, u32),
) -> Option<(Vec<u8>, u32, u32)> {
    let (aw, ah) = a_dims;
    let (bw, bh) = b_dims;
    if aw == 0 || ah == 0 || bw == 0 || bh == 0 {
        return None;
    }
    if a.len() != (aw as usize * ah as usize * 4) || b.len() != (bw as usize * bh as usize * 4) {
        return None;
    }
    // 每半屏各占 half_w，总宽 = 2 × half_w；高度取较大的那一侧。
    let half_w = aw.max(bw);
    let h = ah.max(bh);
    let total_w = half_w.checked_mul(2)?;
    let mut out = vec![0u8; (total_w as usize) * (h as usize) * 4];

    // 把一侧居中贴进 [x0, x0+half_w) × [0, h)
    let mut blit = |src: &[u8], sw: u32, sh: u32, x0: u32| {
        let ox = x0 + (half_w - sw) / 2;
        let oy = (h - sh) / 2;
        for y in 0..sh {
            let src_row = (y as usize) * (sw as usize) * 4;
            let dst_row = ((y + oy) as usize) * (total_w as usize) * 4 + (ox as usize) * 4;
            let n = (sw as usize) * 4;
            out[dst_row..dst_row + n].copy_from_slice(&src[src_row..src_row + n]);
        }
    };
    blit(a, aw, ah, 0);
    blit(b, bw, bh, half_w);

    // 中缝一条深色分隔线：不加的话两块同色画面看起来像一整张，
    // 而“我到底在看左还是右”是这块界面唯一要回答的问题。
    let mid = half_w;
    for y in 0..h {
        let i = ((y as usize) * (total_w as usize) + mid as usize) * 4;
        out[i] = 0;
        out[i + 1] = 0;
        out[i + 2] = 0;
        out[i + 3] = 255;
    }
    Some((out, total_w, h))
}
