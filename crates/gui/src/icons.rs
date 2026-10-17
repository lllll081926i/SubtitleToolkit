use iced::widget::svg::{Handle, Svg};
use iced::Length;
use icondata::{
    LuActivity, LuArrowRight, LuCheck, LuChevronDown, LuChevronUp, LuCopy, LuFolder, LuPlay,
    LuPlus, LuSettings, LuSkipBack, LuSkipForward, LuSquare, LuX,
};

/// 将成熟图标库 icondata 的矢量数据转换为 Iced 原生 SVG 控件
pub fn render(data: icondata::Icon, size: f32) -> Svg<'static, iced::Theme> {
    let view_box = data.view_box.unwrap_or("0 0 24 24");
    let stroke = data.stroke.unwrap_or("currentColor");
    let stroke_width = data.stroke_width.unwrap_or("2");
    let fill = data.fill.unwrap_or("none");
    let stroke_linecap = data.stroke_linecap.unwrap_or("round");
    let stroke_linejoin = data.stroke_linejoin.unwrap_or("round");

    let svg_str = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{view_box}" fill="{fill}" stroke="{stroke}" stroke-width="{stroke_width}" stroke-linecap="{stroke_linecap}" stroke-linejoin="{stroke_linejoin}">{}</svg>"#,
        data.data
    );

    iced::widget::svg(Handle::from_memory(svg_str.into_bytes()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
}

// === 成熟的 Lucide 矢量图标语义导出，杜绝任何 Emoji ===

pub fn play(size: f32) -> Svg<'static, iced::Theme> {
    render(LuPlay, size)
}

pub fn stop(size: f32) -> Svg<'static, iced::Theme> {
    render(LuSquare, size)
}

pub fn skip_back(size: f32) -> Svg<'static, iced::Theme> {
    render(LuSkipBack, size)
}

pub fn skip_forward(size: f32) -> Svg<'static, iced::Theme> {
    render(LuSkipForward, size)
}

pub fn plus(size: f32) -> Svg<'static, iced::Theme> {
    render(LuPlus, size)
}

pub fn close(size: f32) -> Svg<'static, iced::Theme> {
    render(LuX, size)
}

pub fn copy(size: f32) -> Svg<'static, iced::Theme> {
    render(LuCopy, size)
}

pub fn check(size: f32) -> Svg<'static, iced::Theme> {
    render(LuCheck, size)
}

pub fn folder(size: f32) -> Svg<'static, iced::Theme> {
    render(LuFolder, size)
}

pub fn settings(size: f32) -> Svg<'static, iced::Theme> {
    render(LuSettings, size)
}

pub fn chevron_up(size: f32) -> Svg<'static, iced::Theme> {
    render(LuChevronUp, size)
}

pub fn chevron_down(size: f32) -> Svg<'static, iced::Theme> {
    render(LuChevronDown, size)
}

pub fn activity(size: f32) -> Svg<'static, iced::Theme> {
    render(LuActivity, size)
}

pub fn arrow_right(size: f32) -> Svg<'static, iced::Theme> {
    render(LuArrowRight, size)
}
