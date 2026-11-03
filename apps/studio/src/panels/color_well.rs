//! The Eyedropper's color menu, opened by holding or right-clicking the top bar's Eyedropper button.
//! Like Photoshop's foreground color, the eyedropper keeps a current color: this menu shows it
//! (click it to edit it in the color picker), puts it on the selected shapes, text and solids,
//! saves it as a swatch or into a library brand, and offers the recent picks and saved swatches
//! to make current again.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled,
    TestSupportExt as _, deferred, div, prelude::FluentBuilder as _, px, rgba,
};
use studio_core::model::Rgba;

use crate::state::ColorTarget;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// A small muted heading over a row of colors.
fn heading(text: &'static str) -> gpui_kit::Div {
    div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text)
}

/// One color to click: it becomes the current color. Swatches can be removed with a right-click.
fn chip(key: &'static str, index: usize, color: Rgba, removable: bool, cx: &mut Context<Workspace>) -> AnyElement {
    div()
        .id((key, index))
        .size(px(22.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(rgba(color.to_rgba_u32()))
        .cursor_pointer()
        .hover(|style| style.border_color(hsla(color::TEXT)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.colors.current = color;
            cx.notify();
        }))
        .when(removable, |chip| {
            chip.on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.state.remove_swatch(index);
                cx.notify();
            }))
        })
        .test_support()
        .into_any_element()
}

/// "Save to library": adds the current color to a brand's colors.
fn save_to_library(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let brands: Vec<_> = workspace.state.libraries.brands.iter().map(|brand| (brand.id, brand.name.clone())).collect();
    if brands.is_empty() {
        return None;
    }
    let weak = cx.weak_entity();
    Some(
        Button::new("well-save-library")
            .outline()
            .small()
            .label("Save to Library")
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                brands.iter().fold(menu, |menu, (brand, name)| {
                    let (weak, brand) = (weak.clone(), *brand);
                    menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        weak.update(cx, |this, cx| {
                            let color = this.state.colors.current;
                            this.state.add_library_color(brand, color);
                            cx.notify();
                        })
                        .ok();
                    }))
                })
            })
            .into_any_element(),
    )
}

/// The menu beside the Eyedropper button, while it is open.
pub fn menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let kept = workspace.state.colors.clone();
    let current = kept.current;
    let takes_color = workspace.state.selection_takes_color();

    let current_row = div()
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .id("well-current")
                .size(px(40.0))
                .flex_none()
                .rounded(px(8.0))
                .border_1()
                .border_color(hsla(color::HAIRLINE))
                .bg(rgba(current.to_rgba_u32()))
                .cursor_pointer()
                .hover(|style| style.border_color(hsla(color::TEXT)))
                .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                    this.ui.color_well = false;
                    super::color_picker::open(this, ColorTarget::Current, event.position(), window, cx);
                    cx.notify();
                }))
                .test_support(),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_color(hsla(color::TEXT)).child(current.to_hex()))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Click to edit · I picks from the viewer")),
        );

    let apply = Button::new("well-apply")
        .primary()
        .small()
        .label("Apply to Selection")
        .disabled(!takes_color)
        .tooltip(if takes_color { "Fill the selected shapes, text and solids with this color" } else { "Select a shape, text or solid first" })
        .on_click(cx.listener(|this, _, window, cx| {
            let count = this.state.apply_color_to_selection(this.state.colors.current);
            if count == 0 {
                window.push_notification("Select a shape, text or solid to color.", cx);
            }
            cx.notify();
        }));
    let save = Button::new("well-save-swatch").outline().small().label("Save Swatch").on_click(cx.listener(|this, _, window, cx| {
        let color = this.state.colors.current;
        if !this.state.save_swatch(color) {
            window.push_notification("That color is already a swatch (or the swatches are full).", cx);
        }
        cx.notify();
    }));

    let recent = if kept.recent.is_empty() {
        div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Pick with the Eyedropper (I) and click the picture.")
    } else {
        let chips: Vec<_> = kept.recent.iter().enumerate().map(|(index, color)| chip("well-recent", index, *color, false, cx)).collect();
        div().flex().flex_wrap().gap_1p5().children(chips)
    };
    let swatches = if kept.swatches.is_empty() {
        div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("No swatches yet. Save Swatch keeps the current color here.")
    } else {
        let chips: Vec<_> = kept.swatches.iter().enumerate().map(|(index, color)| chip("well-swatch", index, *color, true, cx)).collect();
        div().flex().flex_wrap().gap_1p5().children(chips)
    };

    deferred(
        div()
            .id("color-well")
            .absolute()
            .left_0()
            .top(px(34.0))
            .w(px(260.0))
            .p_3()
            .flex()
            .flex_col()
            .gap_2p5()
            .rounded(px(10.0))
            .bg(hsla(color::POPOVER))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .shadow_lg()
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.ui.color_well = false;
                cx.notify();
            }))
            // Clicks in the menu stay in it, rather than reaching the panel underneath (which
            // would clear the selection the color is meant for).
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(current_row)
            .child(div().flex().flex_wrap().gap_1p5().child(apply).child(save).children(save_to_library(workspace, cx)))
            .child(heading("Recent"))
            .child(recent)
            .child(div().flex().child(heading("Swatches")).child(div().flex_1()).when(!kept.swatches.is_empty(), |row| row.child(heading("Right-click to remove"))))
            .child(swatches)
            .test_support(),
    )
    .with_priority(2)
}
