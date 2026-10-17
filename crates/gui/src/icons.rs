//! 界面图标：返回 `Text` 控件，不依赖 iced 的 `svg` 特性。
//!
//! 为什么不用 `iced::widget::svg`：该特性会引入 `resvg`/`usvg`（强制 MPL-2.0），
//! 与 21 §1「核心只收宽松许可」冲突，也让依赖树多出上百个包。图标在这里只作
//! 小尺寸装饰，用 Unicode 符号即可，不需要完整的 SVG 渲染栈。

use iced::widget::text;
use iced::{Element, Font, Length};

/// 图标以固定尺寸的居中字形呈现，保证与相邻文字基线对齐。
fn glyph(sym: &'static str, size: f32) -> Element<'static, crate::message::Message> {
    text(sym)
        .size(size)
        .width(Length::Fixed(size * 1.3))
        .height(Length::Fixed(size * 1.3))
        .font(Font::DEFAULT)
        .into()
}

pub fn play(size: f32) -> Element<'static, crate::message::Message> {
    glyph("▶", size)
}

pub fn stop(size: f32) -> Element<'static, crate::message::Message> {
    glyph("■", size)
}

pub fn skip_back(size: f32) -> Element<'static, crate::message::Message> {
    glyph("⏮", size)
}

pub fn skip_forward(size: f32) -> Element<'static, crate::message::Message> {
    glyph("⏭", size)
}

pub fn plus(size: f32) -> Element<'static, crate::message::Message> {
    glyph("＋", size)
}

pub fn close(size: f32) -> Element<'static, crate::message::Message> {
    glyph("✕", size)
}

pub fn copy(size: f32) -> Element<'static, crate::message::Message> {
    glyph("⧉", size)
}

pub fn check(size: f32) -> Element<'static, crate::message::Message> {
    glyph("✓", size)
}

pub fn folder(size: f32) -> Element<'static, crate::message::Message> {
    glyph("🗀", size)
}

pub fn settings(size: f32) -> Element<'static, crate::message::Message> {
    glyph("⚙", size)
}

pub fn chevron_up(size: f32) -> Element<'static, crate::message::Message> {
    glyph("▲", size)
}

pub fn chevron_down(size: f32) -> Element<'static, crate::message::Message> {
    glyph("▼", size)
}

pub fn activity(size: f32) -> Element<'static, crate::message::Message> {
    glyph("◉", size)
}

pub fn arrow_right(size: f32) -> Element<'static, crate::message::Message> {
    glyph("→", size)
}
