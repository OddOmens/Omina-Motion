//! The Pre-compose dialog (Shift+Cmd+C, or a layer's right-click menu), as in After Effects:
//! a name for the new composition, whether it is only as long as the selected layers span, and
//! whether to open it afterwards. OK moves the selected layers into it (`edit::precompose`).

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{AppContext as _, Context, ParentElement, Styled, Window, div, px};

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Opens the dialog for the selected layers. With nothing selected, says so instead.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.state.selected_layers.is_empty() {
        window.push_notification("Select the layers to pre-compose first.", cx);
        return;
    }
    let count = workspace.state.selected_layers.len();
    let name = workspace.state.precompose_name();
    let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
    let trim = Rc::new(Cell::new(false));
    let open_after = Rc::new(Cell::new(false));
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let trim_box = {
            let trim = trim.clone();
            Checkbox::new("precompose-trim").checked(trim.get()).label("Adjust composition duration to the time span of the selected layers").on_click(move |checked: &bool, window, _| {
                trim.set(*checked);
                window.refresh();
            })
        };
        let open_box = {
            let open_after = open_after.clone();
            Checkbox::new("precompose-open").checked(open_after.get()).label("Open New Composition").on_click(move |checked: &bool, window, _| {
                open_after.set(*checked);
                window.refresh();
            })
        };
        let what = if count == 1 { "the selected layer".to_owned() } else { format!("the {count} selected layers") };
        let (input, trim, open_after, target) = (input.clone(), trim.clone(), open_after.clone(), target.clone());
        dialog
            .title("Pre-compose")
            .w(px(520.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(size::TEXT + 1.0))
                    .child(div().flex().items_center().gap_3().child(div().flex_none().child("New composition name:")).child(div().flex_1().child(Input::new(&input))))
                    .child(div().text_color(hsla(color::MUTED)).child(format!("Moves {what} into a new composition, with all their attributes, and puts it in their place.")))
                    .child(trim_box)
                    .child(open_box),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("precompose-ok").primary().label("OK"))),
            )
            .on_ok(move |_, window, cx| {
                let name = input.read(cx).value().to_string();
                let (trim, open_after) = (trim.get(), open_after.get());
                target
                    .update(cx, |this, cx| {
                        match this.state.precompose_selection(&name, trim) {
                            Some(layer) => {
                                if open_after && let Some(inner) = this.state.nested_composition(layer) {
                                    this.state.open_composition(inner);
                                }
                            }
                            None => window.push_notification("Those layers couldn't be pre-composed.", cx),
                        }
                        cx.notify();
                    })
                    .ok();
                true
            })
    });
}
