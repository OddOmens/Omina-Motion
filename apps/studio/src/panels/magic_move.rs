//! The Magic Move dialog (Composition ▸ Magic Move…): picks the composition the open one turns
//! into, how long the change takes and whether it eases, then builds the joined composition
//! (`studio_core::magic_move`) and opens it.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::{AppContext as _, Context, InteractiveElement as _, ParentElement, Styled, TestSupportExt as _, Window, div, px};
use studio_core::animation::Easing;
use studio_core::model::Id;
use studio_core::morph::default_easing;

use super::morph::parse_seconds;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// How long the change takes unless another length is typed, in seconds.
const DEFAULT_SECONDS: f64 = 1.0;

/// Opens the dialog for the open composition, or says why there is nothing to move into.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let targets: Vec<(Id, String)> = studio_core::magic_move::targets(&state.project, state.active_comp)
        .into_iter()
        .filter_map(|id| state.project.composition(id).map(|composition| (id, composition.name.clone())))
        .collect();
    let Some(&(first, _)) = targets.first() else {
        window.push_notification("Magic Move needs another composition of the same size and frame rate to move into.", cx);
        return;
    };
    let from_name = state.composition().name.clone();
    let chosen = Rc::new(Cell::new(first));
    let eased = Rc::new(Cell::new(true));
    let input = cx.new(|cx| InputState::new(window, cx).default_value(format!("{DEFAULT_SECONDS}")));
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let chosen_name = targets.iter().find(|(id, _)| *id == chosen.get()).map_or_else(String::new, |(_, name)| name.clone());
        let picker = {
            let (targets, chosen) = (targets.clone(), chosen.clone());
            crate::panels::chrome::dropdown("magic-move-target", chosen_name.clone()).w_full().dropdown_menu(move |menu, _, _| {
                targets.iter().fold(menu, |menu, (id, name)| {
                    let (id, chosen) = (*id, chosen.clone());
                    menu.item(PopupMenuItem::new(name.clone()).checked(chosen.get() == id).on_click(move |_, window, _| {
                        chosen.set(id);
                        window.refresh();
                    }))
                })
            })
        };
        let ease_box = {
            let eased = eased.clone();
            Checkbox::new("magic-move-ease").checked(eased.get()).label("Ease in and out").on_click(move |checked: &bool, window, _| {
                eased.set(*checked);
                window.refresh();
            })
        };
        let (input, chosen, eased, target) = (input.clone(), chosen.clone(), eased.clone(), target.clone());
        dialog
            .title("Magic Move")
            .w(px(500.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(size::LARGE_TEXT))
                    .child(div().flex().items_center().gap_3().child(div().flex_none().child(format!("Move “{from_name}” into:"))).child(div().flex_1().child(picker)))
                    .child(div().flex().items_center().gap_3().child(div().flex_none().child("Length (seconds):")).child(div().id("magic-move-length").w(px(100.0)).child(Input::new(&input)).test_support()))
                    .child(ease_box)
                    .child(div().text_color(hsla(color::MUTED)).child(format!(
                        "Makes a new composition that plays “{from_name}”, changes it into “{chosen_name}”, then plays that. Layers with the same name move into place (shapes morph); the others fade out or in."
                    ))),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("magic-move-ok").primary().label("Make"))),
            )
            .on_ok(move |_, window, cx| {
                let Some(seconds) = parse_seconds(&input.read(cx).value()) else {
                    window.push_notification("Enter a length in seconds, such as 1 or 0.5.", cx);
                    return false;
                };
                let easing = if eased.get() { default_easing() } else { Easing::Linear };
                target
                    .update(cx, |this, cx| {
                        if let Err(error) = this.state.magic_move_to(chosen.get(), seconds, easing) {
                            window.push_notification(error.to_string().as_str(), cx);
                        }
                        cx.notify();
                    })
                    .ok();
                true
            })
    });
}
