//! Morphing in the editor: the Morph To dialog (a layer's right-click menu, Layer ▸ Morph To…
//! and the Motion tab) and the Morph card in the Motion tab, which keyframes a shape's outline
//! with a Path stopwatch as After Effects does. See `studio_core::morph`.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{AppContext as _, Context, Div, ElementId, InteractiveElement as _, IntoElement as _, ParentElement, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px};
use studio_core::model::{Layer, LayerContent, ShapeKind};
use studio_core::morph::{DEFAULT_MORPH_SECONDS, MorphError, default_easing};
use studio_core::property::PropertyPath;

use super::fields::{RowControl, grid_row, row_control};
use super::properties::{indented, status_line};
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
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
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

/// A row for a shape property that is keyframed but has no single number to show (its
/// outline, or its fill and stroke), laid out like a transform property's row: a stopwatch that
/// starts or stops it, how many keyframes there are, and buttons to step between them and to
/// add or remove one at the playhead. `key` names the row in its controls' ids.
fn keyframes_row(key: &str, name: &'static str, path: PropertyPath, start_tooltip: &'static str, stop_tooltip: &'static str, layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let frame = workspace.state.current_frame;
    let locked = layer.switches.locked;
    let keys = layer.property(path, frame).map(|property| property.keyframes).unwrap_or_default();
    let animated = !keys.is_empty();
    let on_keyframe = keys.contains(&frame);
    // One of the row's controls with its tooltip. A locked layer's stopwatch and keyframe
    // button do nothing, so they get no click handler.
    let control = |name: &str, kind: RowControl, lit: bool, tooltip: &'static str| {
        row_control(ElementId::Name(format!("{key}-{name}").into()), kind, lit).tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
    };
    let stopwatch = control("stopwatch", RowControl::Stopwatch, animated, if animated { stop_tooltip } else { start_tooltip })
        .when(!locked, |stopwatch| {
            stopwatch.on_click(cx.listener(move |this, _, _, cx| {
                this.state.toggle_stopwatch(id, path);
                cx.notify();
            }))
        })
        .test_support();
    let keys_label = match keys.len() {
        0 => "Not keyframed".to_owned(),
        1 => "1 keyframe".to_owned(),
        count => format!("{count} keyframes"),
    };
    let jump = |forward: bool, cx: &mut Context<Workspace>| {
        let (name, kind, tooltip) = if forward { ("next-keyframe", RowControl::NextKeyframe, "Next Keyframe") } else { ("previous-keyframe", RowControl::PreviousKeyframe, "Previous Keyframe") };
        control(name, kind, false, tooltip)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.state.jump_to_keyframe(id, path, forward);
                cx.notify();
            }))
            .test_support()
    };
    let keyframe = control("keyframe", RowControl::Keyframe, on_keyframe, if on_keyframe { "Remove Keyframe" } else { "Add Keyframe" })
        .when(!locked, |keyframe| {
            keyframe.on_click(cx.listener(move |this, _, _, cx| {
                this.state.toggle_keyframe_at_playhead(id, path);
                cx.notify();
            }))
        })
        .test_support();
    grid_row(Some(stopwatch.into_any_element()), name, None, div().truncate().text_color(hsla(color::MUTED)).child(keys_label))
        .when(animated, |row| row.child(jump(false, cx)).child(keyframe).child(jump(true, cx)))
}

/// The row that animates a shape's outline. `prefix` keeps the ids apart where the row is
/// shown in two cards. The same keyframes animate the shape's size (see `morph::set_morph_size`).
pub fn path_row(prefix: &str, layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    keyframes_row(&format!("{prefix}-path"), "Path", PropertyPath::ShapePath, "Animate the Path and Size", "Stop Animating the Path and Size", layer, workspace, cx)
}

/// The row that animates a shape's fill, stroke and stroke width together: with its stopwatch
/// on, changing any of them keyframes the change at the playhead.
pub fn style_row(prefix: &str, layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    keyframes_row(&format!("{prefix}-style"), "Fill & Stroke", PropertyPath::ShapeStyle, "Animate the Fill, Stroke and Stroke Width", "Stop Animating the Fill and Stroke", layer, workspace, cx)
}

/// The Morph card's body for one shape layer: the Path stopwatch with its keyframe buttons, the
/// fill and stroke morph when there is one, and a way to turn a drawn shape into a path.
pub fn section(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let id = layer.id;
    let locked = layer.switches.locked;
    let animated = layer.is_morphing();
    let style_morphs = layer.morph.as_ref().is_some_and(|morph| morph.style.is_animated());
    let is_box_shape = matches!(&layer.content, LayerContent::Shape { style, .. } if style.kind != ShapeKind::Path);
    let path_row = path_row("morph", layer, workspace, cx);

    // While the fill and stroke morph too, their keyframes get a row; its stopwatch stops them.
    let style_row = style_morphs.then(|| style_row("morph", layer, workspace, cx));
    let convert = (is_box_shape && !animated).then(|| {
        Button::new("morph-convert").outline().small().label("Convert to Path").tooltip("Turn the shape into a pen path with the same outline, to edit its points").disabled(locked).on_click(cx.listener(move |this, _, _, cx| {
            this.state.convert_to_path(id);
            cx.notify();
        }))
    });
    let note = if animated {
        "Move the playhead, then double-click the shape in the viewer and drag its points to keyframe the outline there."
    } else {
        "Select a second shape too, then Morph To, to turn this one into it."
    };
    div().flex().flex_col().gap_1().child(path_row).children(style_row).children(convert.map(|button| indented(button).pt_1())).child(status_line(note))
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
