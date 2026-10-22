//! 界面图标：使用 Windows 自带的 Segoe MDL2 Assets 字体。
//!
//! 为什么不用 `iced::widget::svg`：该特性会引入 `resvg`/`usvg`（强制 MPL-2.0），
//! 与 21 §1「核心只收宽松许可」冲突，也让依赖树多出上百个包。图标在这里只作
//! 小尺寸装饰，不需要完整的 SVG 渲染栈。

use iced::widget::text;
use iced::{Element, Font, Length};

// Windows 10/11 自带该字体。PUA 字形必须显式指定图标字体。
const ICON_FONT: Font = Font::with_name("Segoe MDL2 Assets");

/// 图标以固定尺寸的居中字形呈现，保证与相邻文字基线对齐。
fn glyph(sym: &'static str, size: f32) -> Element<'static, crate::message::Message> {
    text(sym)
        .size(size)
        .width(Length::Fixed(size * 1.3))
        .height(Length::Fixed(size * 1.3))
        .font(ICON_FONT)
        .into()
}

pub fn play(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e768}", size)
}

pub fn stop(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e71a}", size)
}

pub fn skip_back(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e72b}", size)
}

pub fn skip_forward(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e72a}", size)
}

pub fn plus(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e710}", size)
}

pub fn close(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e711}", size)
}

pub fn copy(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e8c8}", size)
}

pub fn check(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e73e}", size)
}

pub fn folder(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e8b7}", size)
}

pub fn settings(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e713}", size)
}

pub fn chevron_up(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e70e}", size)
}

pub fn chevron_down(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e70d}", size)
}

pub fn activity(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e9d9}", size)
}

pub fn arrow_right(size: f32) -> Element<'static, crate::message::Message> {
    glyph("\u{e72a}", size)
}
