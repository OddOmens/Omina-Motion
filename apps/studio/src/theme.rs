//! Studio's palettes and element sizes. The layout follows After Effects; the surfaces are
//! neutral greys and there is exactly one color besides them, the accent: a vivid lime by
//! default. Selection, values, the playhead's handle, the timecode, the snap guide and the
//! Export button all use it, so nothing else on screen competes with the clips.
//!
//! Panels name colors by the constants in `color` (the Obsidian theme's values) and turn them
//! into GPUI colors with `hsla`, which swaps in the active theme's color for each one. So a
//! theme (Midnight, Obsidian, Winter or Mist) and an accent change every panel at once,
//! without each panel knowing about themes.

use std::cell::RefCell;

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};
use serde::{Deserialize, Serialize};

/// Palette as 0xRRGGBB values.
pub mod color {
    /// The window behind the panels, and the gaps between them.
    pub const BACKGROUND: u32 = 0x0e0e0f;
    pub const PANEL: u32 = 0x1a1a1c;
    /// Wells inside a panel (fields, lists, the timeline's rows).
    pub const INSET: u32 = 0x141415;
    /// Around the composition frame in the viewer.
    pub const PASTEBOARD: u32 = 0x111112;
    pub const BORDER: u32 = 0x09090a;
    pub const HAIRLINE: u32 = 0x2a2a2d;
    pub const HOVER: u32 = 0x26262a;
    pub const ROW_SELECTED: u32 = 0x251f3a;
    pub const TEXT: u32 = 0xededef;
    pub const MUTED: u32 = 0x8b8b92;
    /// The one color besides the greys: selection, focus, active controls and everything
    /// that marks a moment in time. A vivid lime unless another accent is picked.
    pub const ACCENT: u32 = 0xd4f25c;
    pub const VALUE: u32 = 0xa78bfa;
    /// The guide line showing where a timeline drag snapped. The accent, so there is only
    /// one color on screen.
    pub const SNAP: u32 = ACCENT;
    /// The one call to action on screen (Export). The accent as well.
    pub const HIGHLIGHT: u32 = ACCENT;
    pub const WORK_AREA: u32 = 0x3a3150;
    pub const NAVIGATOR_THUMB: u32 = 0x3a3a40;
    /// Menus, dialogs and other surfaces that float above the panels.
    pub const POPOVER: u32 = 0x202023;
    /// Buttons and other raised controls at rest.
    pub const RAISED: u32 = 0x232326;
    /// The playhead's line: white, so it reads over any clip color. (One step off pure white,
    /// so the light themes can tell it apart from white used for other things.)
    pub const PLAYHEAD: u32 = 0xfffffe;
}

/// Fixed element sizes in pixels.
pub mod size {
    pub const TOOLBAR_HEIGHT: f32 = 44.0;
    pub const PANEL_HEADER_HEIGHT: f32 = 30.0;
    pub const PANEL_FOOTER_HEIGHT: f32 = 28.0;
    pub const ROW_HEIGHT: f32 = 22.0;
    // The type scale. Every piece of text in all three apps uses one of these sizes, so
    // labels, values and menu items of the same kind come out the same size everywhere.
    /// Tiny notes: sizes under thumbnails, badges, tick labels.
    pub const CAPTION_TEXT: f32 = 10.0;
    /// Secondary text: property names, hints, details in a list.
    pub const SMALL_TEXT: f32 = 11.0;
    /// The standard size: values, buttons, dropdowns, menu items and text fields.
    pub const TEXT: f32 = 12.0;
    /// Setting names and the body of dialogs, one step up so they read as the main line.
    pub const LARGE_TEXT: f32 = 13.0;
    /// Small headings, such as a category over a row of cards.
    pub const HEADING_TEXT: f32 = 14.0;
    /// The app's name beside its icon, and other short titles.
    pub const TITLE_TEXT: f32 = 16.0;
    /// Page and dialog titles: a settings section, a start screen heading.
    pub const PAGE_TITLE_TEXT: f32 = 20.0;
    /// The big title of a welcome step.
    pub const DISPLAY_TEXT: f32 = 26.0;
    /// The start screen's headline.
    pub const HERO_TEXT: f32 = 52.0;
    /// The component library (buttons, dropdowns, menus, text fields) sizes its text and
    /// controls in rems, its text mostly at seven eighths of a rem. At 14 px a rem, that text
    /// is 12.25 px, which matches `TEXT`, and the controls land on whole pixels: a small
    /// button or dropdown is 21 px tall, a compact one 17.5 px.
    pub const REM: f32 = 14.0;
    pub const GUTTER: f32 = 6.0;
    /// Corner radius of panels: barely rounded, enough to soften each panel's corners
    /// without making it look like a card.
    pub const PANEL_RADIUS: f32 = 3.0;
    /// Corner radius of everything that floats over the editor (dialogs, the quick search,
    /// progress popups): a little rounder than a panel, so it reads as lifted off the layout
    /// while still belonging to it.
    pub const POPUP_RADIUS: f32 = 6.0;
}

/// The fixed-width font for timecodes, frame counts and property values, so digits keep their
/// place while a value changes.
pub const MONO_FONT: &str = "Menlo";

/// The app's look: how light the surfaces are. (The aliases are the names these themes were
/// saved under before they were renamed.)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeName {
    /// Pure black surfaces, for dark rooms and OLED screens.
    #[serde(alias = "Black")]
    Midnight,
    /// Dark grey surfaces, not black.
    #[default]
    #[serde(alias = "Dark")]
    Obsidian,
    /// White panels.
    #[serde(alias = "Light")]
    Winter,
    /// Light grey panels, softer than Winter in a bright room.
    Mist,
}

impl ThemeName {
    pub const ALL: [ThemeName; 4] = [Self::Midnight, Self::Obsidian, Self::Winter, Self::Mist];

    pub fn name(self) -> &'static str {
        match self {
            Self::Midnight => "Midnight",
            Self::Obsidian => "Obsidian",
            Self::Winter => "Winter",
            Self::Mist => "Mist",
        }
    }

    /// Whether the theme has light surfaces with dark text.
    pub fn is_light(self) -> bool {
        matches!(self, Self::Winter | Self::Mist)
    }

    /// The theme's window background, panel and text colors, for drawing a small picture
    /// of it where themes are picked.
    pub fn preview(self) -> (u32, u32, u32) {
        let Surfaces([background, panel, _, _, _, _, _, text, ..]) = surfaces(self);
        (background, panel, text)
    }
}

/// The one color used besides the greys: selection, focus, values, active controls and the
/// time markers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Lime,
    Violet,
    Blue,
    Teal,
    Green,
    Orange,
    Pink,
    Red,
}

impl Accent {
    pub const ALL: [Accent; 8] = [Self::Lime, Self::Violet, Self::Blue, Self::Teal, Self::Green, Self::Orange, Self::Pink, Self::Red];

    pub fn name(self) -> &'static str {
        match self {
            Self::Lime => "Lime",
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
            Self::Lime => color::ACCENT,
            Self::Violet => 0x8b5cf6,
            Self::Blue => 0x3b82f6,
            Self::Teal => 0x14b8a6,
            Self::Green => 0x22c55e,
            Self::Orange => 0xf97316,
            Self::Pink => 0xec4899,
            Self::Red => 0xef4444,
        }
    }

    /// The accent as drawn on a theme. Lime is too pale to read on light surfaces, so the
    /// light themes use a deeper green of the same hue; the other accents are the same on all.
    pub fn on_theme(self, theme: ThemeName) -> u32 {
        if self == Self::Lime && theme.is_light() { 0x5a9a0a } else { self.value() }
    }
}

/// The palette colors a theme sets, in this order.
const KEYS: [u32; 17] = [
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
];

/// A theme's surfaces and text: background, panel, inset, pasteboard, border, hairline, hover,
/// text, muted, navigator thumb, popover, raised, playhead.
struct Surfaces([u32; 13]);

fn surfaces(theme: ThemeName) -> Surfaces {
    use color::*;
    Surfaces(match theme {
        ThemeName::Obsidian => [BACKGROUND, PANEL, INSET, PASTEBOARD, BORDER, HAIRLINE, HOVER, TEXT, MUTED, NAVIGATOR_THUMB, POPOVER, RAISED, PLAYHEAD],
        ThemeName::Midnight => [0x000000, 0x0a0a0a, 0x050505, 0x030303, 0x000000, 0x1a1a1a, 0x151515, 0xf2f2f2, 0x808086, 0x262626, 0x111111, 0x141414, 0xffffff],
        ThemeName::Winter => [0xeeeef1, 0xffffff, 0xf5f5f7, 0xe4e4e8, 0xd0d0d6, 0xe2e2e6, 0xededf0, 0x18181b, 0x66666e, 0xc8c8ce, 0xffffff, 0xf3f3f5, 0x18181b],
        ThemeName::Mist => [0xc9c9ce, 0xe4e4e7, 0xdadade, 0xc0c0c6, 0xb2b2b9, 0xcacacf, 0xd6d6da, 0x18181b, 0x5c5c64, 0xb0b0b7, 0xeeeef0, 0xdcdce0, 0x18181b],
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
fn palette(theme: ThemeName, accent: Accent) -> [u32; 17] {
    let Surfaces([background, panel, inset, pasteboard, border, hairline, hover, text, muted, thumb, popover, raised, playhead]) = surfaces(theme);
    let accent = accent.on_theme(theme);
    let light = theme.is_light();
    // Shades of the accent: a tint for selected rows and the work area, and a readable tone
    // for values (lighter on dark surfaces, darker on light ones).
    let selected = mix(panel, accent, if light { 0.18 } else { 0.16 });
    let value = if light { mix(accent, 0x000000, 0.2) } else { mix(accent, 0xffffff, 0.25) };
    let work_area = mix(panel, accent, if light { 0.35 } else { 0.28 });
    [background, panel, inset, pasteboard, border, hairline, hover, selected, text, muted, accent, value, work_area, thumb, popover, raised, playhead]
}

thread_local! {
    /// The active theme's colors for `KEYS`. The UI runs on one thread, so this is per thread.
    static ACTIVE: RefCell<[u32; 17]> = const { RefCell::new(KEYS) };
}

/// A palette color (one of the `color` constants, or any 0xRRGGBB) as a GPUI color, in the
/// active theme.
pub fn hsla(value: u32) -> Hsla {
    let themed = KEYS.iter().position(|&key| key == value).map_or(value, |index| ACTIVE.with(|active| active.borrow()[index]));
    rgb(themed).into()
}

/// The color for text and icons drawn on top of the accent: near-black on a bright accent
/// (lime, say) and white on a deep one, so it always reads.
pub fn on_accent() -> Hsla {
    let accent = themed(color::ACCENT);
    let (red, green, blue) = ((accent >> 16) & 0xff, (accent >> 8) & 0xff, accent & 0xff);
    // Perceived brightness, 0–255, weighting green most as the eye does.
    let brightness = (red * 299 + green * 587 + blue * 114) / 1000;
    rgb(if brightness > 150 { 0x10130a } else { 0xffffff }).into()
}

/// The active theme's value for a palette color, as 0xRRGGBB.
pub fn themed(value: u32) -> u32 {
    KEYS.iter().position(|&key| key == value).map_or(value, |index| ACTIVE.with(|active| active.borrow()[index]))
}

/// Makes `theme` and `accent` active: Studio's own panels, and the component library under
/// them (menus, dialogs, inputs and buttons). Call `cx.refresh_windows()` after a change.
pub fn apply(theme: ThemeName, accent: Accent, cx: &mut App) {
    ACTIVE.with(|active| *active.borrow_mut() = palette(theme, accent));
    let mode = if theme.is_light() { ThemeMode::Light } else { ThemeMode::Dark };
    Theme::change(mode, None, cx);
    let theme = Theme::global_mut(cx);
    theme.colors.background = hsla(color::BACKGROUND);
    theme.colors.foreground = hsla(color::TEXT);
    theme.colors.border = hsla(color::HAIRLINE);
    theme.colors.muted_foreground = hsla(color::MUTED);
    theme.colors.primary = hsla(color::ACCENT);
    theme.colors.primary_foreground = on_accent();
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
    // Ghost buttons that are switched on (snapping, loop, compact tracks…) sit on a tint of the
    // accent, so on and off are easy to tell apart.
    theme.colors.secondary_active = hsla(color::ROW_SELECTED);
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
    theme.font_size = px(size::REM);
    theme.radius_lg = px(size::POPUP_RADIUS);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_with_lime_keeps_the_named_colors_and_others_swap_them() {
        let obsidian = palette(ThemeName::Obsidian, Accent::Lime);
        assert_eq!(obsidian[0], color::BACKGROUND);
        assert_eq!(obsidian[10], color::ACCENT);
        let winter = palette(ThemeName::Winter, Accent::Blue);
        assert_eq!(winter[10], 0x3b82f6);
        assert_ne!(winter[1], color::PANEL, "light panels");
        // Text on panels stays readable: dark on light.
        assert!(winter[8] < 0x404040 && obsidian[8] > 0xc0c0c0);
        // Lime is deepened on the light themes, where the pale lime would not show.
        assert_ne!(palette(ThemeName::Mist, Accent::Lime)[10], color::ACCENT);
    }

    #[test]
    fn themes_saved_under_their_old_names_still_load() {
        let load = |name: &str| serde_json::from_str::<ThemeName>(&format!("\"{name}\"")).unwrap();
        assert_eq!((load("Dark"), load("Black"), load("Light")), (ThemeName::Obsidian, ThemeName::Midnight, ThemeName::Winter));
    }

    #[test]
    fn every_key_is_distinct_so_lookups_are_exact() {
        let mut keys = KEYS.to_vec();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KEYS.len());
    }
}
