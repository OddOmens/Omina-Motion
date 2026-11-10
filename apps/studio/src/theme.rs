//! Studio's palettes and element sizes. The layout follows After Effects; the default colors
//! are near-blacks with a slight cool, inky cast, a violet accent for selection, and one lime
//! "time" color for everything that marks a moment (the playhead's handle, the current
//! timecode, the snap guide).
//!
//! Panels name colors by the constants in `color` (the Dark theme's values) and turn them into
//! GPUI colors with `hsla`, which swaps in the active theme's color for each one. So a theme
//! (Dark, Black or Light) and an accent (violet, blue, and so on) change every panel at once,
//! without each panel knowing about themes.

use std::cell::RefCell;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};
use serde::{Deserialize, Serialize};

/// Palette as 0xRRGGBB values.
pub mod color {
    // The Dark theme's surfaces are not pure greys: each leans slightly blue, which gives the
    // app its own cast and lets the lime time color and the clip colors stand out.
    /// The window behind the panels, and the gaps between them.
    pub const BACKGROUND: u32 = 0x07080b;
    pub const PANEL: u32 = 0x111318;
    /// Wells inside a panel (fields, lists, the timeline's rows).
    pub const INSET: u32 = 0x0c0d11;
    /// Around the composition frame in the viewer.
    pub const PASTEBOARD: u32 = 0x0a0b0e;
    pub const BORDER: u32 = 0x040507;
    pub const HAIRLINE: u32 = 0x1f222a;
    pub const HOVER: u32 = 0x1b1e25;
    pub const ROW_SELECTED: u32 = 0x251f3a;
    pub const TEXT: u32 = 0xeceef2;
    pub const MUTED: u32 = 0x838893;
    /// Selection, focus and active controls: a violet, as in the motion tools this is styled on.
    pub const ACCENT: u32 = 0x8b5cf6;
    pub const VALUE: u32 = 0xa78bfa;
    /// Studio's signature lime, used only for things that mark a moment in time: the playhead's
    /// handle, the current timecode and the snap guide. (One step off `HIGHLIGHT`, so a theme
    /// can give it a darker shade where lime would not show.)
    pub const TIME: u32 = 0xd4f25d;
    /// The guide line showing where a timeline drag snapped.
    pub const SNAP: u32 = TIME;
    /// The bright lime used for the one call to action on screen (Export).
    pub const HIGHLIGHT: u32 = 0xd4f25c;
    pub const WORK_AREA: u32 = 0x3a3150;
    pub const NAVIGATOR_THUMB: u32 = 0x2e323c;
    /// Menus, dialogs and other surfaces that float above the panels.
    pub const POPOVER: u32 = 0x16181e;
    /// Buttons and other raised controls at rest.
    pub const RAISED: u32 = 0x181b21;
    /// The playhead: white, so it reads over any clip color. (One step off pure white, so the
    /// Light theme can tell it apart from white used for other things.)
    pub const PLAYHEAD: u32 = 0xfffffe;
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
    /// Corner radius of panels: gently rounded, so each reads as its own slab on the darker
    /// window behind it.
    pub const PANEL_RADIUS: f32 = 6.0;
}

/// The fixed-width font for timecodes, frame counts and property values, so digits keep their
/// place while a value changes.
pub const MONO_FONT: &str = "Menlo";

/// The app's look: how light the surfaces are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeName {
    #[default]
    Dark,
    /// Pure black surfaces, for dark rooms and OLED screens.
    Black,
    Light,
}

impl ThemeName {
    pub const ALL: [ThemeName; 3] = [Self::Dark, Self::Black, Self::Light];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Black => "Black",
            Self::Light => "Light",
        }
    }
}

/// The color used for selection, focus, values and active controls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Violet,
    Blue,
    Teal,
    Green,
    Orange,
    Pink,
    Red,
}

impl Accent {
    pub const ALL: [Accent; 7] = [Self::Violet, Self::Blue, Self::Teal, Self::Green, Self::Orange, Self::Pink, Self::Red];

    pub fn name(self) -> &'static str {
        match self {
            Self::Violet => "Violet",
            Self::Blue => "Blue",
            Self::Teal => "Teal",
            Self::Green => "Green",
            Self::Orange => "Orange",
            Self::Pink => "Pink",
            Self::Red => "Red",
        }
    }

    pub fn value(self) -> u32 {
        match self {
            Self::Violet => color::ACCENT,
            Self::Blue => 0x3b82f6,
            Self::Teal => 0x14b8a6,
            Self::Green => 0x22c55e,
            Self::Orange => 0xf97316,
            Self::Pink => 0xec4899,
            Self::Red => 0xef4444,
        }
    }
}

/// The palette colors a theme sets, in this order.
const KEYS: [u32; 18] = [
    color::BACKGROUND,
    color::PANEL,
    color::INSET,
    color::PASTEBOARD,
    color::BORDER,
    color::HAIRLINE,
    color::HOVER,
    color::ROW_SELECTED,
    color::TEXT,
    color::MUTED,
    color::ACCENT,
    color::VALUE,
    color::WORK_AREA,
    color::NAVIGATOR_THUMB,
    color::POPOVER,
    color::RAISED,
    color::PLAYHEAD,
    color::TIME,
];

/// A theme's surfaces and text: background, panel, inset, pasteboard, border, hairline, hover,
/// text, muted, navigator thumb, popover, raised, playhead.
struct Surfaces([u32; 13]);

fn surfaces(theme: ThemeName) -> Surfaces {
    use color::*;
    Surfaces(match theme {
        ThemeName::Dark => [BACKGROUND, PANEL, INSET, PASTEBOARD, BORDER, HAIRLINE, HOVER, TEXT, MUTED, NAVIGATOR_THUMB, POPOVER, RAISED, PLAYHEAD],
        ThemeName::Black => [0x000000, 0x0a0a0a, 0x050505, 0x030303, 0x000000, 0x1a1a1a, 0x151515, 0xf2f2f2, 0x808086, 0x262626, 0x111111, 0x141414, 0xffffff],
        ThemeName::Light => [0xdcdce0, 0xf6f6f7, 0xececee, 0xcfcfd4, 0xc4c4ca, 0xdadade, 0xe6e6e9, 0x18181b, 0x66666e, 0xc2c2c8, 0xffffff, 0xeeeef0, 0x18181b],
    })
}

/// `a` moved `amount` (0–1) of the way to `b`, channel by channel.
fn mix(a: u32, b: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let (from, to) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        ((from + (to - from) * amount).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

/// The colors standing in for `KEYS` in a theme with an accent.
fn palette(theme: ThemeName, accent: Accent) -> [u32; 18] {
    let Surfaces([background, panel, inset, pasteboard, border, hairline, hover, text, muted, thumb, popover, raised, playhead]) = surfaces(theme);
    let accent = accent.value();
    let light = theme == ThemeName::Light;
    // Shades of the accent: a tint for selected rows and the work area, and a readable tone
    // for values (lighter on dark surfaces, darker on light ones).
    let selected = mix(panel, accent, if light { 0.18 } else { 0.16 });
    let value = if light { mix(accent, 0x000000, 0.2) } else { mix(accent, 0xffffff, 0.25) };
    let work_area = mix(panel, accent, if light { 0.35 } else { 0.28 });
    // Lime disappears on light surfaces, so the Light theme uses a dark olive of the same hue.
    let time = if light { 0x5a7a00 } else { color::TIME };
    [background, panel, inset, pasteboard, border, hairline, hover, selected, text, muted, accent, value, work_area, thumb, popover, raised, playhead, time]
}

thread_local! {
    /// The active theme's colors for `KEYS`. The UI runs on one thread, so this is per thread.
    static ACTIVE: RefCell<[u32; 18]> = const { RefCell::new(KEYS) };
}

/// A palette color (one of the `color` constants, or any 0xRRGGBB) as a GPUI color, in the
/// active theme.
pub fn hsla(value: u32) -> Hsla {
    let themed = KEYS.iter().position(|&key| key == value).map_or(value, |index| ACTIVE.with(|active| active.borrow()[index]));
    rgb(themed).into()
}

/// The active theme's value for a palette color, as 0xRRGGBB.
pub fn themed(value: u32) -> u32 {
    KEYS.iter().position(|&key| key == value).map_or(value, |index| ACTIVE.with(|active| active.borrow()[index]))
}

/// Makes `theme` and `accent` active: Studio's own panels, and the component library under
/// them (menus, dialogs, inputs and buttons). Call `cx.refresh_windows()` after a change.
pub fn apply(theme: ThemeName, accent: Accent, cx: &mut App) {
    ACTIVE.with(|active| *active.borrow_mut() = palette(theme, accent));
    let mode = if theme == ThemeName::Light { ThemeMode::Light } else { ThemeMode::Dark };
    Theme::change(mode, None, cx);
    let theme = Theme::global_mut(cx);
    theme.colors.background = hsla(color::BACKGROUND);
    theme.colors.foreground = hsla(color::TEXT);
    theme.colors.border = hsla(color::HAIRLINE);
    theme.colors.muted_foreground = hsla(color::MUTED);
    theme.colors.primary = hsla(color::ACCENT);
    // A quiet focus ring on text fields, not a bright one.
    theme.colors.ring = hsla(if mode == ThemeMode::Light { 0xb4b4bc } else { 0x3a3a42 });
    theme.colors.selection = gpui_kit::rgba((themed(color::ACCENT) << 8) | 0x55).into();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dark_violet_theme_keeps_the_original_colors_and_others_swap_them() {
        let dark = palette(ThemeName::Dark, Accent::Violet);
        assert_eq!(dark[0], color::BACKGROUND);
        assert_eq!(dark[10], color::ACCENT);
        let light = palette(ThemeName::Light, Accent::Blue);
        assert_eq!(light[10], 0x3b82f6);
        assert_ne!(light[1], color::PANEL, "light panels");
        // The time color is lime on dark surfaces and a darker shade on light ones.
        assert_eq!(dark[17], color::TIME);
        assert!(light[17] < color::TIME, "a darker time color on light panels");
        // Text on panels stays readable: dark on light.
        assert!(light[8] < 0x404040 && dark[8] > 0xc0c0c0);
    }

    #[test]
    fn every_key_is_distinct_so_lookups_are_exact() {
        let mut keys = KEYS.to_vec();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KEYS.len());
    }
}
