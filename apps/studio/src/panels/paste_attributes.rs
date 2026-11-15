//! The Paste Attributes dialog (Alt+Cmd+V), as in Premiere Pro: copy a clip, select others,
//! then choose which of the copied clip's settings to put on them (motion, opacity, effects,
//! speed, blend mode, audio levels). Keyframes line up with each clip's start. See
//! `studio_core::copy::paste_attributes`.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::{Context, ParentElement, Styled, Window, div, px};
use studio_core::copy::Attributes;

use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Opens the dialog, or says what's missing: something copied, and something selected.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(source) = workspace.state.attributes_source() else {
        window.push_notification("Copy a clip first (⌘C), then select the clips to paste its attributes onto.", cx);
        return;
    };
    let source_name = source.name.clone();
    let targets = workspace.state.selected_layers.iter().filter(|id| **id != source.id).count();
    if targets == 0 {
        window.push_notification("Select the clips to paste the attributes onto.", cx);
        return;
    }
    let chosen = Rc::new(Cell::new(Attributes::default()));
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
        // One checkbox per attribute, each flipping its own field of the shared choice.
        let option = |id: &'static str, label: &'static str, get: fn(&Attributes) -> bool, set: fn(&mut Attributes, bool)| {
            let chosen = chosen.clone();
            Checkbox::new(id).label(label).checked(get(&chosen.get())).on_click(move |checked: &bool, window, _| {
                let mut attributes = chosen.get();
                set(&mut attributes, *checked);
                chosen.set(attributes);
                window.refresh();
            })
        };
        let onto = if targets == 1 { "the selected clip".to_owned() } else { format!("the {targets} selected clips") };
        let (chosen, target) = (chosen.clone(), target.clone());
        dialog
            .title("Paste Attributes")
            .w(px(420.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_size(px(size::TEXT + 1.0))
                    .child(div().text_color(hsla(color::MUTED)).child(format!("From “{source_name}” onto {onto}:")))
                    .child(option("paste-attr-motion", "Motion (position, scale, rotation, anchor)", |a| a.motion, |a, on| a.motion = on))
                    .child(option("paste-attr-opacity", "Opacity", |a| a.opacity, |a, on| a.opacity = on))
                    .child(option("paste-attr-effects", "Effects (added after their own)", |a| a.effects, |a, on| a.effects = on))
                    .child(option("paste-attr-speed", "Speed", |a| a.speed, |a, on| a.speed = on))
                    .child(option("paste-attr-blend", "Blend mode", |a| a.blend_mode, |a, on| a.blend_mode = on))
                    .child(option("paste-attr-audio", "Volume and fades", |a| a.audio, |a, on| a.audio = on)),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("paste-attributes-ok").primary().label("Paste"))),
            )
            .on_ok(move |_, window, cx| {
                let attributes = chosen.get();
                target
                    .update(cx, |this, cx| {
                        if this.state.paste_attributes(attributes) == 0 {
                            window.push_notification("Nothing changed: those attributes don't apply to the selected clips, or they're locked.", cx);
                        }
                        cx.notify();
                    })
                    .ok();
                true
            })
    });
}
