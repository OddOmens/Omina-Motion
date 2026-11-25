//! The Curves effect's editor, in the Color and Effects tabs: a graph of input (across) to
//! output (up) with the curve over a grid, for brightness (RGB) or one channel. Click the graph
//! to add a point on the curve, drag a point to move it, double-click a point to remove it. The
//! curve runs smoothly through every point (see `grade::curve_table`). Omina Edit's Curves
//! adjustment works the same way.

use gpui_kit::{
    AnyElement, AppContext as _, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, PathBuilder,
    StatefulInteractiveElement, Styled, TestSupportExt as _, canvas, div, point, prelude::FluentBuilder as _, px,
};
use studio_core::grade::curve_table;
use studio_core::model::{CurveChannel, Id, LutSource, ToneCurves};

use super::chrome::{EmptyDragView, MeasuredBounds};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Drag payload: the Curves effect, and which point of its shown curve.
#[derive(Clone, Copy, Debug)]
struct PointDrag {
    effect: Id,
    index: usize,
}

/// The curves of the Curves effect `effect` on `layer`.
fn curves_of(workspace: &Workspace, layer: Id, effect: Id) -> Option<ToneCurves> {
    match workspace.state.effect_lut(layer, effect)? {
        LutSource::Curves(curves) => Some(curves.clone()),
        _ => None,
    }
}

/// Replaces one curve's points on the effect.
fn write_points(workspace: &mut Workspace, layer: Id, effect: Id, channel: CurveChannel, points: &[[f32; 2]]) {
    let Some(mut curves) = curves_of(workspace, layer, effect) else { return };
    curves.set_points(channel, points);
    workspace.state.set_effect_lut(layer, effect, Some(LutSource::Curves(curves)));
}

/// A point's place on the graph (0–1 each way) from a window position over it.
fn place(bounds: &MeasuredBounds, position: gpui_kit::Point<gpui_kit::Pixels>) -> [f32; 2] {
    let area = bounds.get();
    [((f32::from(position.x) - area.left) / area.width.max(1.0)).clamp(0.0, 1.0), (1.0 - (f32::from(position.y) - area.top) / area.height.max(1.0)).clamp(0.0, 1.0)]
}

/// The color a channel's curve is drawn in.
fn line_color(channel: CurveChannel) -> u32 {
    match channel {
        CurveChannel::Master => 0xededef,
        CurveChannel::Red => 0xff5f57,
        CurveChannel::Green => 0x4cd964,
        CurveChannel::Blue => 0x4a9eff,
    }
}

/// The Curves effect's controls: which curve to edit (RGB, Red, Green, Blue; one with points
/// moved off its straight line is marked), the graph, and a button to straighten the shown curve.
pub fn editor(layer: Id, effect: Id, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(curves) = curves_of(workspace, layer, effect) else { return div().into_any_element() };
    let channel = workspace.ui.curve_channel;
    let straight = ToneCurves::default();
    let tabs = div().flex().gap_1().children(CurveChannel::ALL.map(|option| {
        let shaped = curves.points(option) != straight.points(option);
        let selected = option == channel;
        div()
            .id(ElementId::Name(format!("curve-channel:{}:{}", effect.0, option.name()).into()))
            .px_2()
            .py_0p5()
            .rounded(px(4.0))
            .cursor_pointer()
            .text_size(px(size::SMALL_TEXT))
            .text_color(if selected { gpui_kit::Hsla::from(gpui_kit::rgb(line_color(option))) } else { hsla(color::MUTED) })
            .when(selected, |tab| tab.bg(hsla(color::RAISED)))
            .child(if shaped { format!("{} •", option.name()) } else { option.name().to_owned() })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.ui.curve_channel = option;
                cx.notify();
            }))
            .test_support()
    }));
    let reset = div()
        .id(ElementId::Name(format!("curve-straighten:{}", effect.0).into()))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .cursor_pointer()
        .hover(|style| style.text_color(hsla(color::TEXT)))
        .child("Straighten")
        .on_click(cx.listener(move |this, _, _, cx| {
            write_points(this, layer, effect, this.ui.curve_channel, &ToneCurves::default().points(CurveChannel::Master));
            cx.notify();
        }))
        .test_support();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_1()
        .child(div().flex().items_center().justify_between().child(tabs).child(reset))
        .child(graph(layer, effect, channel, &curves, workspace, cx))
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).child("Click to add a point, drag to move it, double-click to remove it."))
        .into_any_element()
}

/// The graph of one curve, with a grid, the other channels' curves faintly behind, and a
/// handle on every point.
fn graph(layer: Id, effect: Id, channel: CurveChannel, curves: &ToneCurves, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let bounds = workspace.ui.curve_bounds.clone();
    let points = curves.points(channel);
    let shown = curve_table(&points);
    // The other curves that have been shaped, drawn faintly so each channel's shape stays in view.
    let others: Vec<([f32; 256], u32)> = CurveChannel::ALL
        .into_iter()
        .filter(|&other| other != channel && curves.points(other) != ToneCurves::default().points(other))
        .map(|other| (curve_table(&curves.points(other)), line_color(other)))
        .collect();
    let line = line_color(channel);
    let drawing = canvas(|_, _, _| (), move |area, _, window, _| {
        let (left, top, width, height) = (area.origin.x, area.origin.y, area.size.width, area.size.height);
        let at = |x: f32, y: f32| point(left + width * x, top + height * (1.0 - y));
        // Quarters, and the straight line from black to white the curve bends away from.
        let mut grid = PathBuilder::stroke(px(1.0));
        for step in 1..4 {
            let t = step as f32 / 4.0;
            grid.move_to(at(t, 0.0));
            grid.line_to(at(t, 1.0));
            grid.move_to(at(0.0, t));
            grid.line_to(at(1.0, t));
        }
        grid.move_to(at(0.0, 0.0));
        grid.line_to(at(1.0, 1.0));
        if let Ok(path) = grid.build() {
            window.paint_path(path, hsla(color::HAIRLINE));
        }
        let draw_curve = |table: &[f32; 256], width: f32| {
            let mut path = PathBuilder::stroke(px(width));
            path.move_to(at(0.0, table[0]));
            for (index, value) in table.iter().enumerate().skip(1) {
                path.line_to(at(index as f32 / 255.0, *value));
            }
            path.build().ok()
        };
        for (table, other_line) in &others {
            if let Some(path) = draw_curve(table, 1.0) {
                window.paint_path(path, gpui_kit::Hsla::from(gpui_kit::rgb(*other_line)).opacity(0.35));
            }
        }
        if let Some(path) = draw_curve(&shown, 1.5) {
            window.paint_path(path, gpui_kit::Hsla::from(gpui_kit::rgb(line)));
        }
    })
    .absolute()
    .size_full();
    let handles: Vec<_> = points
        .iter()
        .enumerate()
        .map(|(index, &[x, y])| {
            div()
                .id(ElementId::Name(format!("curve-point:{}:{index}", effect.0).into()))
                .absolute()
                .left(gpui_kit::relative(x))
                .top(gpui_kit::relative(1.0 - y))
                .ml(px(-5.0))
                .mt(px(-5.0))
                .size(px(10.0))
                .rounded_full()
                .border_1()
                .border_color(gpui_kit::rgb(0x000000))
                .bg(gpui_kit::rgb(0xffffff))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.ui.grade_pressed = true;
                    // A double-click removes the point; the two ends stay.
                    if event.click_count >= 2
                        && let Some(curves) = curves_of(this, layer, effect)
                    {
                        let mut points = curves.points(channel);
                        if points.len() > 2 && index > 0 && index + 1 < points.len() {
                            points.remove(index);
                            write_points(this, layer, effect, channel, &points);
                            cx.notify();
                        }
                    }
                }))
                .on_drag(PointDrag { effect, index }, |_, _, _, cx| cx.new(|_| EmptyDragView))
                .test_support()
        })
        .collect();
    let (add_bounds, move_bounds) = (bounds.clone(), bounds.clone());
    div()
        .id(ElementId::Name(format!("curve-graph:{}", effect.0).into()))
        .relative()
        .w_full()
        .h(px(180.0))
        .rounded(px(4.0))
        .bg(hsla(color::INSET))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(bounds.probe())
        .child(drawing)
        .children(handles)
        // A click on the graph adds a point there, on the curve (so the picture doesn't jump).
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            let [x, _] = place(&add_bounds, event.position);
            let Some(curves) = curves_of(this, layer, effect) else { return };
            let mut points = curves.points(channel);
            let y = curve_table(&points)[((x * 255.0).round() as usize).min(255)];
            points.push([x, y]);
            write_points(this, layer, effect, channel, &points);
            this.ui.grade_pressed = true;
            cx.notify();
        }))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<PointDrag>, _, cx| {
            let PointDrag { effect: dragged, index } = *event.drag(cx);
            if dragged != effect {
                return;
            }
            let Some(curves) = curves_of(this, layer, effect) else { return };
            let mut points = curves.points(channel);
            let [mut x, y] = place(&move_bounds, event.event.position);
            // A point stays between its neighbors; the ends stay the ends.
            let count = points.len();
            if index == 0 {
                x = x.min(points.get(1).map_or(1.0, |next| next[0] - 0.01));
            } else if index + 1 == count {
                x = x.max(points[index - 1][0] + 0.01);
            } else {
                x = x.clamp(points[index - 1][0] + 0.01, points[index + 1][0] - 0.01);
            }
            if let Some(point) = points.get_mut(index) {
                *point = [x, y];
            }
            write_points(this, layer, effect, channel, &points);
            cx.notify();
        }))
        .test_support()
        .into_any_element()
}
