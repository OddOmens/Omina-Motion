//! The Settings dialog (Cmd+, or the gear at the bottom of the rail): the app's look (theme
//! and accent color) and its playback cache. Every change applies at once and is remembered.
//!
//! About the cache: Studio makes small ProRes proxy copies of footage on disk, so scrubbing
//! and playback stay smooth whatever the source codec. That folder is what the limit and the
//! Clear button manage. In memory, playback keeps only a few decoded frames around the playhead
//! for each clip, which needs no setting.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogClose, DialogFooter};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};

use crate::app_settings;
use crate::theme::{Accent, ThemeName, color, hsla, size};
use crate::workspace::Workspace;

/// Disk limits offered for proxies, in gigabytes (`None` for no limit).
const CACHE_LIMITS: &[Option<u32>] = &[Some(5), Some(10), Some(25), Some(50), Some(100), None];

fn limit_label(limit: Option<u32>) -> String {
    limit.map_or_else(|| "No limit".to_owned(), |gb| format!("{gb} GB"))
}

/// A byte count for people: "820 MB", "1.4 GB".
pub fn format_bytes(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_000_000.0;
    if megabytes < 1000.0 { format!("{megabytes:.0} MB") } else { format!("{:.1} GB", megabytes / 1000.0) }
}

/// What the dialog shows; kept outside the dialog so its buttons can change it.
struct Shown {
    theme: ThemeName,
    accent: Accent,
    limit: Option<u32>,
    cache_bytes: u64,
    note: Option<String>,
}

/// Applies a theme and accent everywhere and remembers them.
fn set_look(workspace: &gpui_kit::WeakEntity<Workspace>, theme: ThemeName, accent: Accent, cx: &mut gpui_kit::App) {
    crate::theme::apply(theme, accent, cx);
    app_settings::update(|settings| (settings.theme, settings.accent) = (theme, accent));
    // Every panel redraws when the workspace does.
    workspace.update(cx, |_, cx| cx.notify()).ok();
    cx.refresh_windows();
}

fn heading(text: &'static str) -> gpui_kit::Div {
    div().pt_1().text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text)
}

fn row(label: &'static str, control: impl IntoElement) -> gpui_kit::Div {
    div().min_h(px(34.0)).flex().items_center().gap_3().child(div().w(px(110.0)).flex_none().text_color(hsla(color::MUTED)).child(label)).child(div().flex_1().min_w_0().child(control))
}

pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let settings = app_settings::load();
    let folder = workspace.render.proxies.cache_folder().to_path_buf();
    let shown = Rc::new(RefCell::new(Shown {
        theme: settings.theme,
        accent: settings.accent,
        limit: settings.proxy_cache_limit_gb,
        cache_bytes: studio_media::cache_size(&folder),
        note: None,
    }));
    let target = cx.weak_entity();
    let proxies = workspace.render.proxies.clone();

    window.open_dialog(cx, move |dialog, _, _| {
        let now = shown.borrow();
        let themes = ThemeName::ALL.into_iter().map(|theme| {
            let (shown, target) = (shown.clone(), target.clone());
            let selected = now.theme == theme;
            // A little preview of the theme's panel and text colors.
            let (panel, text) = match theme {
                ThemeName::Dark => (0x111112, 0xededef),
                ThemeName::Black => (0x000000, 0xf2f2f2),
                ThemeName::Light => (0xf6f6f7, 0x18181b),
            };
            div()
                .id(("theme", theme as usize))
                .flex_1()
                .flex()
                .flex_col()
                .gap_1p5()
                .p_1p5()
                .rounded(px(8.0))
                .border_2()
                .border_color(hsla(if selected { color::ACCENT } else { color::HAIRLINE }))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    let accent = shown.borrow().accent;
                    shown.borrow_mut().theme = theme;
                    set_look(&target, theme, accent, cx);
                    window.refresh();
                })
                .child(div().h(px(36.0)).rounded(px(5.0)).bg(rgb(panel)).border_1().border_color(hsla(color::HAIRLINE)).p_1p5().child(div().text_color(rgb(text)).text_size(px(size::SMALL_TEXT)).child("Aa")))
                .child(div().text_size(px(size::SMALL_TEXT)).child(theme.name()))
                .test_support()
        });
        let accents = Accent::ALL.into_iter().map(|accent| {
            let (shown, target) = (shown.clone(), target.clone());
            let selected = now.accent == accent;
            div()
                .id(("accent", accent as usize))
                .size(px(26.0))
                .rounded_full()
                .border_2()
                .border_color(hsla(if selected { color::TEXT } else { color::PANEL }))
                .p(px(2.0))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| {
                    let theme = shown.borrow().theme;
                    shown.borrow_mut().accent = accent;
                    set_look(&target, theme, accent, cx);
                    window.refresh();
                })
                .child(div().size_full().rounded_full().bg(rgb(accent.value())))
                .test_support()
        });

        let limit_menu = {
            let (shown, proxies) = (shown.clone(), proxies.clone());
            Button::new("cache-limit").outline().small().label(limit_label(now.limit)).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
                CACHE_LIMITS.iter().fold(menu, |menu, &limit| {
                    let (shown, proxies) = (shown.clone(), proxies.clone());
                    let checked = shown.borrow().limit == limit;
                    menu.item(PopupMenuItem::new(limit_label(limit)).checked(checked).on_click(move |_, window, _| {
                        proxies.set_cache_limit(limit.map(|gb| u64::from(gb) << 30));
                        app_settings::update(|settings| settings.proxy_cache_limit_gb = limit);
                        let mut shown = shown.borrow_mut();
                        shown.limit = limit;
                        shown.cache_bytes = studio_media::cache_size(proxies.cache_folder());
                        window.refresh();
                    }))
                })
            })
        };
        let clear = {
            let (shown, proxies) = (shown.clone(), proxies.clone());
            Button::new("cache-clear").outline().small().label("Clear Cache").on_click(move |_, window, _| {
                let freed = proxies.clear_cache();
                let mut shown = shown.borrow_mut();
                shown.cache_bytes = studio_media::cache_size(proxies.cache_folder());
                shown.note = Some(format!("Freed {}. Proxies are made again when a project needs them.", format_bytes(freed)));
                window.refresh();
            })
        };
        let reveal = {
            let folder = proxies.cache_folder().to_path_buf();
            Button::new("cache-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| cx.reveal_path(&folder))
        };

        dialog
            .title("Settings")
            .w(px(520.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_size(px(size::TEXT + 1.0))
                    .child(heading("Appearance"))
                    .child(row("Theme", div().flex().gap_2().children(themes)))
                    .child(row("Accent color", div().flex().flex_wrap().items_center().gap_1p5().children(accents).child(div().pl_1().text_color(hsla(color::MUTED)).child(now.accent.name()))))
                    .child(div().h(px(1.0)).my_2().bg(hsla(color::HAIRLINE)))
                    .child(heading("Playback cache"))
                    .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(
                        "Studio makes small ProRes proxies of your footage so scrubbing and playback stay smooth. They are kept on disk and can be rebuilt at any time. In memory, only a few frames around the playhead are kept for each clip.",
                    ))
                    .child(row("Proxies on disk", div().flex().items_center().gap_2().child(div().flex_1().child(format_bytes(now.cache_bytes))).child(reveal)))
                    .child(row("Size limit", div().flex().items_center().gap_2().child(limit_menu).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("oldest go first"))))
                    .child(row("", clear))
                    .when_some(now.note.clone(), |column, note| column.child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::VALUE)).child(note))),
            )
            .footer(DialogFooter::new().child(DialogClose::new().trigger(|button| button.primary().label("Done"))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(format_bytes(820_000_000), "820 MB");
        assert_eq!(format_bytes(1_400_000_000), "1.4 GB");
        assert_eq!(limit_label(None), "No limit");
    }
}
