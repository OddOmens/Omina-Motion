//! Studio's dark editor palette and element sizes, matching the After Effects
//! layout the UI is modeled on. Panels use these constants instead of literals
//! so the look can be tuned in one place.

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

/// Palette as 0xRRGGBB values.
pub mod color {
    pub const BACKGROUND: u32 = 0x1b1b1b;
    pub const PANEL: u32 = 0x232323;
    pub const INSET: u32 = 0x1a1a1a;
    pub const PASTEBOARD: u32 = 0x181818;
    pub const BORDER: u32 = 0x111111;
    pub const HAIRLINE: u32 = 0x2c2c2c;
    pub const HOVER: u32 = 0x2b2b2b;
    pub const ROW_SELECTED: u32 = 0x3a3f47;
    pub const TEXT: u32 = 0xd6d6d6;
    pub const MUTED: u32 = 0x8f8f8f;
    pub const ACCENT: u32 = 0x2d8ceb;
    pub const VALUE: u32 = 0x4ea6f5;
    pub const KEYFRAME: u32 = 0xc8c8c8;
    /// The guide line showing where a timeline drag snapped.
    pub const SNAP: u32 = 0xf2c94c;
    pub const WORK_AREA: u32 = 0x5b6270;
    pub const NAVIGATOR_THUMB: u32 = 0x4a4f57;
}

/// Fixed element sizes in pixels.
pub mod size {
    pub const TOOLBAR_HEIGHT: f32 = 40.0;
    pub const PANEL_HEADER_HEIGHT: f32 = 30.0;
    pub const PANEL_FOOTER_HEIGHT: f32 = 28.0;
    pub const ROW_HEIGHT: f32 = 22.0;
    pub const TEXT: f32 = 12.0;
    pub const SMALL_TEXT: f32 = 11.0;
    pub const GUTTER: f32 = 4.0;
}

pub fn hsla(value: u32) -> Hsla {
    rgb(value).into()
}

/// Switches the component library to dark mode and applies Studio's palette over it.
pub fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.colors.background = hsla(color::BACKGROUND);
    theme.colors.foreground = hsla(color::TEXT);
    theme.colors.border = hsla(color::HAIRLINE);
    theme.colors.muted_foreground = hsla(color::MUTED);
    theme.colors.primary = hsla(color::ACCENT);
    theme.colors.accent = hsla(color::HOVER);
    theme.colors.popover = hsla(0x2a2a2a);
    theme.font_size = px(size::TEXT);
}
