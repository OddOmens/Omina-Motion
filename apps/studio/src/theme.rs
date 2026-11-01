//! Studio's dark editor palette and element sizes. The layout follows After Effects; the
//! colors are neutral near-blacks with a violet accent and brightly colored clips, as in
//! modern motion tools, a shade darker so the picture stands out. Panels use these
//! constants instead of literals so the look can be tuned in one place.

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

/// Palette as 0xRRGGBB values.
pub mod color {
    /// The window behind the panels, and the gaps between them.
    pub const BACKGROUND: u32 = 0x070708;
    pub const PANEL: u32 = 0x111112;
    /// Wells inside a panel (fields, lists, the timeline's rows).
    pub const INSET: u32 = 0x0c0c0d;
    /// Around the composition frame in the viewer.
    pub const PASTEBOARD: u32 = 0x0a0a0b;
    pub const BORDER: u32 = 0x040405;
    pub const HAIRLINE: u32 = 0x1e1e21;
    pub const HOVER: u32 = 0x1b1b1e;
    pub const ROW_SELECTED: u32 = 0x251f3a;
    pub const TEXT: u32 = 0xededef;
    pub const MUTED: u32 = 0x85858c;
    /// Selection, focus and active controls: a violet, as in the motion tools this is styled on.
    pub const ACCENT: u32 = 0x8b5cf6;
    pub const VALUE: u32 = 0xa78bfa;
    /// The guide line showing where a timeline drag snapped.
    pub const SNAP: u32 = 0xd4f25c;
    /// The bright lime used for the one call to action on screen (Export).
    pub const HIGHLIGHT: u32 = 0xd4f25c;
    pub const WORK_AREA: u32 = 0x3a3150;
    pub const NAVIGATOR_THUMB: u32 = 0x2f2f35;
    /// Menus, dialogs and other surfaces that float above the panels.
    pub const POPOVER: u32 = 0x161618;
    /// Buttons and other raised controls at rest.
    pub const RAISED: u32 = 0x18181b;
    /// The playhead: white, so it reads over any clip color.
    pub const PLAYHEAD: u32 = 0xffffff;
}

/// Fixed element sizes in pixels.
pub mod size {
    pub const TOOLBAR_HEIGHT: f32 = 40.0;
    pub const PANEL_HEADER_HEIGHT: f32 = 30.0;
    pub const PANEL_FOOTER_HEIGHT: f32 = 28.0;
    pub const ROW_HEIGHT: f32 = 22.0;
    pub const TEXT: f32 = 12.0;
    pub const SMALL_TEXT: f32 = 11.0;
    pub const GUTTER: f32 = 6.0;
    /// Corner radius of panels.
    pub const PANEL_RADIUS: f32 = 10.0;
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
    // A quiet focus ring on text fields, not a bright one.
    theme.colors.ring = hsla(0x3a3a42);
    theme.colors.selection = gpui_kit::rgba(0x8b5cf655).into();
    theme.colors.slider_bar = hsla(color::ACCENT);
    theme.colors.progress_bar = hsla(color::ACCENT);
    theme.colors.accent = hsla(color::HOVER);
    // The component library's own surfaces, so menus, dialogs, inputs and buttons match.
    theme.colors.popover = hsla(color::POPOVER);
    theme.colors.input = hsla(color::HAIRLINE);
    theme.colors.muted = hsla(color::INSET);
    theme.colors.secondary = hsla(color::RAISED);
    theme.colors.secondary_hover = hsla(color::HOVER);
    theme.colors.button = hsla(color::RAISED);
    theme.colors.button_hover = hsla(color::HOVER);
    theme.colors.list = hsla(color::PANEL);
    theme.colors.list_hover = hsla(color::HOVER);
    theme.colors.list_active = hsla(color::ROW_SELECTED);
    theme.colors.tab_bar = hsla(color::PANEL);
    theme.colors.title_bar = hsla(color::BACKGROUND);
    theme.colors.window_border = hsla(color::BORDER);
    theme.colors.scrollbar = hsla(color::PANEL);
    theme.colors.scrollbar_thumb = hsla(color::HAIRLINE);
    theme.font_size = px(size::TEXT);
}
