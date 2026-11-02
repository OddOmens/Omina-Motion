//! The Magic Resize dialog (Composition > Magic Resize…): pick one or more sizes and Studio
//! makes a copy of the open composition in each, with its layers rearranged to suit the new
//! shape (see `studio_core::resize`). The original is left as it was; the last copy made opens.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::component::Disableable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::{Context, ParentElement, Styled, Window, div, px};
use studio_core::resize::SIZES;

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The sizes ticked when the dialog opens: the three social shapes, minus the one the
/// composition already is.
const SOCIAL: [usize; 3] = [0, 1, 2];

pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let comp = workspace.state.composition();
    let current = (comp.width, comp.height);
    let chosen: Rc<RefCell<BTreeSet<usize>>> = Rc::new(RefCell::new(SOCIAL.into_iter().filter(|&index| (SIZES[index].1, SIZES[index].2) != current).collect()));
    let source_name = comp.name.clone();
    let target = cx.weak_entity();

    window.open_dialog(cx, move |dialog, _, _| {
        let mut list = div().flex().flex_col().gap_2().text_size(px(size::TEXT + 1.0));
        for (index, &(name, width, height)) in SIZES.iter().enumerate() {
            let same = (width, height) == current;
            let ticked = chosen.borrow().contains(&index);
            let chosen = chosen.clone();
            let label = if same { format!("{name}  ({width}×{height}, the size it is now)") } else { format!("{name}  ({width}×{height})") };
            list = list.child(Checkbox::new(("resize-size", index)).label(label).disabled(same).checked(ticked).on_click(move |checked: &bool, window, _| {
                let mut chosen = chosen.borrow_mut();
                if *checked {
                    chosen.insert(index);
                } else {
                    chosen.remove(&index);
                }
                window.refresh();
            }));
        }
        let count = chosen.borrow().len();
        let (ok_chosen, ok_target) = (chosen.clone(), target.clone());
        dialog
            .title("Magic Resize")
            .w(px(520.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().text_color(hsla(color::MUTED)).child(format!(
                        "Makes a copy of “{source_name}” in each size you tick. Backgrounds are scaled to fill the new frame; titles, logos and graphics keep their place and are resized to fit. “{source_name}” itself is not changed."
                    )))
                    .child(list),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("resize-ok").primary().disabled(count == 0).label(match count {
                        1 => "Make 1 Copy".to_owned(),
                        n => format!("Make {n} Copies"),
                    }))),
            )
            .on_ok(move |_, window, cx| {
                let sizes: Vec<usize> = ok_chosen.borrow().iter().copied().collect();
                if sizes.is_empty() {
                    return false;
                }
                let made = ok_target
                    .update(cx, |this, cx| {
                        let made = sizes.iter().filter(|&&index| this.state.magic_resize(SIZES[index].1, SIZES[index].2, short_label(SIZES[index].0)).is_some()).count();
                        cx.notify();
                        made
                    })
                    .unwrap_or(0);
                if made < sizes.len() {
                    window.push_notification("Some sizes could not be made.", cx);
                }
                true
            })
    });
}

/// The part of a size's name used in the copy's name: "Vertical 9:16 · Reels…" → "9:16".
pub fn short_label(name: &str) -> &str {
    let head = name.split(" · ").next().unwrap_or(name);
    head.rsplit(' ').next().unwrap_or(head)
}
