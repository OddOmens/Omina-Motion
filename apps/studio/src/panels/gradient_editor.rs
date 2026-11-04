//! The gradient editor under a shape's fill or stroke in Properties, like Premiere's: a bar
//! showing the gradient with a marker for each color stop. Click the bar to add a stop there,
//! drag a marker to move its stop, click a marker to pick its color. The picked stop's
//! location can be typed, and it can be deleted (a gradient keeps at least two stops).
//! Where the gradient runs is set by dragging its two ends in the viewer.

use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseUpEvent,
    ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
    relative, rgba,
};
use studio_core::gradient::{Gradient, MAX_STOPS};
use studio_core::model::{Id, Rgba};

use super::chrome::{EmptyDragView, heading_button};
use super::fields::{FieldTarget, value_field};
use crate::state::{ColorTarget, PaintSlot};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The gradient bar's height, and how far below it the stop markers sit.
const BAR_HEIGHT: f32 = 18.0;
const MARKER: f32 = 12.0;

/// Dragging a stop marker along the bar.
#[derive(Clone, Copy, Debug)]
pub struct GradientStopDrag {
    layer: Id,
    slot: PaintSlot,
}

fn slot_index(slot: PaintSlot) -> usize {
    match slot {
        PaintSlot::Fill => 0,
        PaintSlot::Stroke => 1,
    }
}

fn slot_key(slot: PaintSlot) -> &'static str {
    match slot {
        PaintSlot::Fill => "fill",
        PaintSlot::Stroke => "stroke",
    }
}

fn gpui(color: Rgba) -> gpui_kit::Hsla {
    rgba(color.to_rgba_u32()).into()
}

/// The picked stop of this gradient, if one is picked (and still exists).
fn picked(workspace: &Workspace, layer: Id, slot: PaintSlot, gradient: &Gradient) -> Option<usize> {
    workspace.ui.gradient_stop.filter(|&(l, s, index)| l == layer && s == slot && index < gradient.stops().len()).map(|(_, _, index)| index)
}

/// The bar: each stretch between two stops as a two-color gradient, and flat color before the
/// first stop and after the last.
fn bar(gradient: &Gradient) -> impl IntoElement {
    let stops = gradient.stops();
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    let mut bar = div().absolute().top_0().left_0().right_0().h(px(BAR_HEIGHT)).flex().rounded(px(4.0)).overflow_hidden().border_1().border_color(hsla(color::HAIRLINE));
    bar = bar.child(div().h_full().flex_none().w(relative(first.at)).bg(gpui(first.color)));
    for pair in stops.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        bar = bar.child(div().h_full().flex_none().w(relative(b.at - a.at)).bg(linear_gradient(90.0, linear_color_stop(gpui(a.color), 0.0), linear_color_stop(gpui(b.color), 1.0))));
    }
    bar.child(div().h_full().flex_1().bg(gpui(last.color)))
}

/// One stop's marker under the bar: press to pick it, drag to move it, release without
/// dragging to choose its color.
fn marker(layer: Id, slot: PaintSlot, index: usize, at: f32, stop_color: Rgba, is_picked: bool, cx: &mut Context<Workspace>) -> AnyElement {
    div()
        .id(ElementId::Name(format!("gradient-stop:{}:{index}", slot_key(slot)).into()))
        .absolute()
        .top(px(BAR_HEIGHT + 3.0))
        .left(relative(at))
        .ml(px(-MARKER / 2.0))
        .size(px(MARKER))
        .rounded(px(3.0))
        .border_2()
        .border_color(hsla(if is_picked { color::TEXT } else { color::MUTED }))
        .bg(gpui(stop_color))
        .cursor(gpui_kit::CursorStyle::ResizeLeftRight)
        // The press isn't stopped here: the element's own drag start needs to see it too. The
        // bar's click handler ignores presses below the bar, where markers sit.
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            this.ui.gradient_stop = Some((layer, slot, index));
            this.ui.gradient_stop_dragged = false;
            cx.notify();
        }))
        .on_drag(GradientStopDrag { layer, slot }, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_mouse_up(MouseButton::Left, cx.listener(move |this, event: &MouseUpEvent, window, cx| {
            if this.ui.gradient_stop_dragged {
                this.ui.gradient_stop_dragged = false;
                return;
            }
            // The stop may have moved to another index while it was dragged.
            let index = this.ui.gradient_stop.map_or(index, |(_, _, picked)| picked);
            super::color_picker::open(this, ColorTarget::Shape { layer, slot, stop: index }, event.position, window, cx);
        }))
        .test_support()
        .into_any_element()
}

/// The whole editor for one shape paint's gradient.
pub fn editor(layer: Id, slot: PaintSlot, gradient: Gradient, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let bounds = workspace.ui.gradient_bars[slot_index(slot)].clone();
    let picked_stop = picked(workspace, layer, slot, &gradient);
    let full = gradient.stops().len() >= MAX_STOPS;
    let click_bounds = bounds.clone();
    let mut area = div()
        .id(ElementId::Name(format!("gradient-bar:{}", slot_key(slot)).into()))
        .relative()
        .w_full()
        .h(px(BAR_HEIGHT + MARKER + 5.0))
        .child(bounds.probe())
        .child(bar(&gradient))
        // A press on the bar adds a stop there, in the color already at that spot.
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            let area = click_bounds.get();
            if f32::from(event.position.y) > area.top + BAR_HEIGHT {
                return;
            }
            let at = (f32::from(event.position.x) - area.left) / area.width.max(1.0);
            if let Some(index) = this.state.add_gradient_stop(layer, slot, at) {
                this.ui.gradient_stop = Some((layer, slot, index));
            }
            cx.notify();
        }))
        // A marker dragged along the bar moves its stop; passing another stop reorders them.
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<GradientStopDrag>, _, cx| {
            let drag = *event.drag(cx);
            if drag.layer != layer || drag.slot != slot {
                return;
            }
            let Some((_, _, index)) = this.ui.gradient_stop.filter(|&(l, s, _)| l == layer && s == slot) else { return };
            let left = f32::from(event.bounds.origin.x);
            let width = f32::from(event.bounds.size.width).max(1.0);
            let at = (f32::from(event.event.position.x) - left) / width;
            let index = this.state.move_gradient_stop(layer, slot, index, at);
            this.ui.gradient_stop = Some((layer, slot, index));
            this.ui.gradient_stop_dragged = true;
            cx.notify();
        }))
        .when(!full, |area| area.cursor(gpui_kit::CursorStyle::Crosshair))
        .test_support();
    for (index, stop) in gradient.stops().iter().enumerate() {
        area = area.child(marker(layer, slot, index, stop.at, stop.color, picked_stop == Some(index), cx));
    }

    // Under the bar: the picked stop's location and Delete, then Reverse.
    let mut row = div().flex().items_center().gap_1p5().text_size(px(size::SMALL_TEXT));
    match picked_stop {
        Some(index) => {
            let can_delete = gradient.stops().len() > 2;
            row = row
                .child(div().text_color(hsla(color::MUTED)).child("Location"))
                .child(value_field(FieldTarget::GradientStop { layer, slot, stop: index }, workspace, cx))
                .child(
                    heading_button(ElementId::Name(format!("gradient-delete:{}", slot_key(slot)).into()), IconName::Trash, "Delete Stop", false, cx.listener(move |this, _, _, cx| {
                        if this.state.remove_gradient_stop(layer, slot, index) {
                            this.ui.gradient_stop = None;
                        }
                        cx.notify();
                    }))
                    .disabled(!can_delete),
                );
        }
        None => {
            row = row.child(div().text_color(hsla(color::MUTED)).child(if full { "Click a stop to edit it." } else { "Click the bar to add a stop." }));
        }
    }
    row = row.child(div().flex_1()).child(heading_button(
        ElementId::Name(format!("gradient-reverse:{}", slot_key(slot)).into()),
        IconName::ArrowLeftRight,
        "Reverse Gradient",
        false,
        cx.listener(move |this, _, _, cx| {
            this.state.reverse_gradient(layer, slot);
            this.ui.gradient_stop = None;
            cx.notify();
        }),
    ));
    div().w_full().flex().flex_col().gap_1().child(area).child(row).into_any_element()
}
