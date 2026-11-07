//! Morphing in the editor: the Morph To dialog (a layer's right-click menu, Layer ▸ Morph To…
//! and the Motion tab) and the Morph card in the Motion tab, which keyframes a shape's outline
//! with a Path stopwatch as After Effects does. See `studio_core::morph`.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _};
use gpui_kit::{AppContext as _, Context, Div, ElementId, InteractiveElement as _, ParentElement, Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px};
use studio_core::model::{Layer, LayerContent, ShapeKind};
use studio_core::morph::{DEFAULT_MORPH_SECONDS, MorphError, default_easing};

use super::properties::{labeled_row, status_line};
use crate::state::MorphPair;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// What the Morph card is for, shown by the info button on its heading.
pub const MORPH_INFO: &str = "Morphs a shape's outline from one keyframe to the next. Start the Path stopwatch, move the playhead, then drag the shape's points in the viewer. To turn one shape into another, select both (the one to change first) and choose Morph To.";

/// Why Morph To can't run on the selection, in words for a notification or tooltip.
pub fn morph_problem(workspace: &Workspace) -> Option<String> {
    if workspace.state.selected_layers.len() != 2 {
        return Some("Select two shape layers: the one to morph first, then the one to morph it into.".to_owned());
    }
    workspace.state.morph_pair().err().map(|error| error.to_string())
}

/// The seconds typed into the Length field: a positive number, with or without a trailing "s".
pub(super) fn parse_seconds(text: &str) -> Option<f64> {
    let seconds: f64 = text.trim().trim_end_matches('s').trim().parse().ok()?;
    (seconds.is_finite() && seconds > 0.0).then_some(seconds)
}

/// Opens the Morph To dialog for the two selected layers, or says why it can't.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let pair = match workspace.state.morph_pair() {
        Ok(pair) => pair,
        Err(_) => {
            let problem = morph_problem(workspace).unwrap_or_else(|| MorphError::NotShapes.to_string());
            window.push_notification(problem.as_str(), cx);
            return;
        }
    };
    let name = |id| workspace.state.composition().layer(id).map(|layer| layer.name.clone()).unwrap_or_default();
    let (from_name, to_name) = (name(pair.from), name(pair.to));
    let input = cx.new(|cx| InputState::new(window, cx).default_value(format!("{DEFAULT_MORPH_SECONDS}")));
    let eased = Rc::new(Cell::new(true));
    let target = cx.weak_entity();
    window.open_dialog(cx, move |dialog, _, _| {
        let ease_box = {
            let eased = eased.clone();
            Checkbox::new("morph-ease").checked(eased.get()).label("Ease in and out").on_click(move |checked: &bool, window, _| {
                eased.set(*checked);
                window.refresh();
            })
        };
        let (input, eased, target) = (input.clone(), eased.clone(), target.clone());
        dialog
            .title("Morph To")
            .w(px(480.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(size::TEXT + 1.0))
                    .child(div().child(format!("“{from_name}” turns into “{to_name}”, starting at the playhead.")))
                    .child(div().flex().items_center().gap_3().child(div().flex_none().child("Length (seconds):")).child(div().id("morph-length").w(px(100.0)).child(Input::new(&input)).test_support()))
                    .child(ease_box)
                    .child(div().text_color(hsla(color::MUTED)).child(format!(
                        "Keyframes the outline, fill, stroke, position, scale, rotation and opacity of “{from_name}”, then hides “{to_name}”."
                    ))),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("morph-ok").primary().label("Morph"))),
            )
            .on_ok(move |_, window, cx| {
                let Some(seconds) = parse_seconds(&input.read(cx).value()) else {
                    window.push_notification("Enter a length in seconds, such as 1 or 0.5.", cx);
                    return false;
                };
                let easing = if eased.get() { default_easing() } else { studio_core::animation::Easing::Linear };
                target.update(cx, |this, cx| run(this, pair, seconds, easing, window, cx)).ok();
                true
            })
    });
}

/// Makes the morph and shows it in the Motion tab, or says why it failed.
pub fn run(workspace: &mut Workspace, pair: MorphPair, seconds: f64, easing: studio_core::animation::Easing, window: &mut Window, cx: &mut Context<Workspace>) {
    match workspace.state.morph_layers(pair, seconds, easing) {
        Ok(_) => {
            workspace.ui.tabs.insert(PanelId::Properties, super::motion::TAB);
        }
        Err(error) => window.push_notification(error.to_string().as_str(), cx),
    }
    cx.notify();
}

/// A small square icon button on a Morph card row.
fn icon_button(id: &str, icon: IconName, tooltip: &'static str, lit: bool) -> Button {
    Button::new(ElementId::Name(id.to_owned().into()))
        .ghost()
        .xsmall()
        .icon(Icon::new(icon).text_color(hsla(if lit { color::ACCENT } else { color::MUTED })))
        .tooltip(tooltip)
}

/// The Morph card's body for one shape layer: the Path stopwatch with its keyframe buttons, the
/// fill and stroke morph when there is one, and a way to turn a drawn shape into a path.
pub fn section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let frame = workspace.state.current_frame;
    let locked = layer.switches.locked;
    let path_keys = layer.morph.as_ref().map(|morph| morph.path.keyframe_frames()).unwrap_or_default();
    let animated = !path_keys.is_empty();
    let on_keyframe = path_keys.contains(&frame);
    let style_morphs = layer.morph.as_ref().is_some_and(|morph| morph.style.is_animated());
    let is_box_shape = matches!(&layer.content, LayerContent::Shape { style, .. } if style.kind != ShapeKind::Path);

    let stopwatch = icon_button("morph-path-stopwatch", IconName::Timer, if animated { "Stop Animating the Path" } else { "Animate the Path" }, animated)
        .disabled(locked)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_path_animation(id);
            cx.notify();
        }));
    let keys_label = match path_keys.len() {
        0 => "Not keyframed".to_owned(),
        1 => "1 keyframe".to_owned(),
        count => format!("{count} keyframes"),
    };
    let jump = |forward: bool, cx: &mut Context<Workspace>| {
        let (key, icon, tooltip) = if forward { ("morph-next-keyframe", IconName::ChevronRight, "Next Keyframe") } else { ("morph-previous-keyframe", IconName::ChevronLeft, "Previous Keyframe") };
        icon_button(key, icon, tooltip, false).on_click(cx.listener(move |this, _, _, cx| {
            this.state.jump_to_morph_keyframe(id, forward);
            cx.notify();
        }))
    };
    let keyframe = icon_button("morph-path-keyframe", IconName::Diamond, if on_keyframe { "Remove Path Keyframe" } else { "Add Path Keyframe" }, on_keyframe)
        .disabled(locked)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_path_keyframe(id);
            cx.notify();
        }));
    let path_row = div()
        .h(px(30.0))
        .flex()
        .items_center()
        .gap_1()
        .child(stopwatch)
        .child(div().w(px(70.0)).flex_none().child("Path"))
        .child(div().flex_1().text_color(hsla(color::MUTED)).child(keys_label))
        .when(animated, |row| row.child(jump(false, cx)).child(keyframe).child(jump(true, cx)));

    let style_row = style_morphs.then(|| {
        let stop = Button::new("morph-stop-style").outline().xsmall().label("Stop").tooltip("Keep the fill and stroke as they are at the playhead").disabled(locked).on_click(cx.listener(move |this, _, _, cx| {
            this.state.stop_style_morph(id);
            cx.notify();
        }));
        labeled_row("Fill & Stroke", div().flex().items_center().gap_2().child(div().flex_1().text_color(hsla(color::MUTED)).child("Morphing")).child(stop))
    });
    let convert = (is_box_shape && !animated).then(|| {
        Button::new("morph-convert").outline().small().label("Convert to Path").tooltip("Turn the shape into a pen path with the same outline, to edit its points").disabled(locked).on_click(cx.listener(move |this, _, _, cx| {
            this.state.convert_to_path(id);
            cx.notify();
        }))
    });
    let note = if animated {
        "Move the playhead and drag the shape's points in the viewer to keyframe the outline there."
    } else {
        "Select a second shape too, then Morph To, to turn this one into it."
    };
    div().flex().flex_col().gap_1().child(path_row).children(style_row).children(convert.map(|button| div().pt_1().child(button))).child(status_line(note))
}

/// The Morph card's body when two layers are selected: what Morph To would do, and its button.
pub fn pair_section(workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let problem = morph_problem(workspace);
    let names: Vec<String> = workspace.state.selected_layers.iter().filter_map(|id| workspace.state.composition().layer(*id)).map(|layer| layer.name.clone()).collect();
    let summary = match (&problem, names.as_slice()) {
        (None, [from, to]) => format!("“{from}” morphs into “{to}”."),
        (Some(problem), _) => problem.clone(),
        _ => String::new(),
    };
    let button = Button::new("morph-to-open")
        .primary()
        .small()
        .label("Morph To…")
        .disabled(problem.is_some())
        .on_click(cx.listener(|this, _, window, cx| open(this, window, cx)));
    div().flex().flex_col().gap_2().child(status_line(summary)).child(div().child(button))
}

#[cfg(test)]
mod tests {
    use super::parse_seconds;

    #[test]
    fn lengths_are_positive_seconds() {
        assert_eq!(parse_seconds("1"), Some(1.0));
        assert_eq!(parse_seconds(" 0.5 s"), Some(0.5));
        assert_eq!(parse_seconds("2s"), Some(2.0));
        for bad in ["", "0", "-1", "soon", "inf", "NaN"] {
            assert_eq!(parse_seconds(bad), None, "{bad:?}");
        }
    }
}
