//! The Flow panel (second tab beside Effects & Presets), modeled on the Flow
//! plugin for After Effects: a curve editor with two draggable handles, curve
//! presets, and Linear / Hold. Changes apply immediately to the segments that
//! leave each selected keyframe.

use gpui_kit::component::button::Button;
use gpui_kit::component::Sizable as _;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement,
    MouseButton, ParentElement, PathBuilder, Pixels, StatefulInteractiveElement, Styled, canvas, div, point, px,
    relative,
};
use studio_core::animation::{CubicBezier, EASING_PRESETS, Easing};

use super::chrome::EmptyDragView;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// Vertical room above and below the unit square, so overshooting curves stay visible.
const Y_MIN: f32 = -0.4;
const Y_MAX: f32 = 1.4;

/// Drag payload for the curve handles: 0 is the first control point, 1 the second.
#[derive(Clone, Copy, Debug)]
pub struct HandleDrag(usize);

/// Maps a curve point (x 0–1, y in `Y_MIN..Y_MAX`) to fractions of the editor's width and height.
fn to_fraction(x: f32, y: f32) -> (f32, f32) {
    (x, 1.0 - (y - Y_MIN) / (Y_MAX - Y_MIN))
}

/// The inverse of `to_fraction`, clamped so x stays in 0–1 and y inside the editor.
fn from_fraction(fx: f32, fy: f32) -> (f32, f32) {
    (fx.clamp(0.0, 1.0), (Y_MIN + (1.0 - fy) * (Y_MAX - Y_MIN)).clamp(Y_MIN, Y_MAX))
}

/// Draws the grid, the linear reference, the handle arms and the curve itself.
fn paint_curve(curve: CubicBezier, bounds: Bounds<Pixels>, window: &mut gpui_kit::Window) {
    let at = |x: f32, y: f32| {
        let (fx, fy) = to_fraction(x, y);
        point(bounds.origin.x + bounds.size.width * fx, bounds.origin.y + bounds.size.height * fy)
    };
    let line = |from: gpui_kit::Point<Pixels>, to: gpui_kit::Point<Pixels>, width: f32, color: u32, window: &mut gpui_kit::Window| {
        let mut path = PathBuilder::stroke(px(width));
        path.move_to(from);
        path.line_to(to);
        if let Ok(path) = path.build() {
            window.paint_path(path, hsla(color));
        }
    };
    // The unit square's top and bottom edges, and the straight linear curve.
    line(at(0.0, 0.0), at(1.0, 0.0), 1.0, color::HAIRLINE, window);
    line(at(0.0, 1.0), at(1.0, 1.0), 1.0, color::HAIRLINE, window);
    line(at(0.0, 0.0), at(1.0, 1.0), 1.0, color::HAIRLINE, window);
    // Handle arms.
    line(at(0.0, 0.0), at(curve.x1, curve.y1), 1.0, color::MUTED, window);
    line(at(1.0, 1.0), at(curve.x2, curve.y2), 1.0, color::MUTED, window);
    // The curve.
    let mut path = PathBuilder::stroke(px(2.0));
    path.move_to(at(0.0, 0.0));
    path.cubic_bezier_to(at(1.0, 1.0), at(curve.x1, curve.y1), at(curve.x2, curve.y2));
    if let Ok(path) = path.build() {
        window.paint_path(path, hsla(color::ACCENT));
    }
}

fn handle(index: usize, x: f32, y: f32) -> impl IntoElement + use<> {
    let (fx, fy) = to_fraction(x, y);
    div()
        .id(("flow-handle", index))
        .absolute()
        .left(relative(fx))
        .top(relative(fy))
        .ml(px(-6.0))
        .mt(px(-6.0))
        .size(px(12.0))
        .rounded_full()
        .bg(hsla(color::TEXT))
        .border_2()
        .border_color(hsla(color::ACCENT))
        .cursor_grab()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_drag(HandleDrag(index), |_, _, _, cx| cx.new(|_| EmptyDragView))
}

/// Applies `easing` to the selected keyframes and remembers curves for the editor.
fn apply(workspace: &mut Workspace, easing: Easing) {
    if let Easing::Curve(curve) = easing {
        workspace.ui.flow_curve = curve;
    }
    workspace.state.apply_easing(easing);
}

pub fn body(workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let selected = workspace.state.selected_keyframes.len();
    // Show the selection's own curve when its keyframes share one; otherwise the last curve used.
    let shared = workspace.state.selection_easing();
    let curve = match shared {
        Some(Easing::Curve(curve)) => curve,
        _ => workspace.ui.flow_curve,
    };

    let editor = div()
        .id("flow-editor")
        .relative()
        .h(px(170.0))
        .mx_3()
        .my_2()
        .rounded(px(3.0))
        .bg(hsla(color::INSET))
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_curve(curve, bounds, window)).absolute().size_full())
        .child(handle(0, curve.x1, curve.y1))
        .child(handle(1, curve.x2, curve.y2))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<HandleDrag>, _, cx| {
            let bounds = event.bounds;
            let fx = f32::from(event.event.position.x - bounds.origin.x) / f32::from(bounds.size.width).max(1.0);
            let fy = f32::from(event.event.position.y - bounds.origin.y) / f32::from(bounds.size.height).max(1.0);
            let (x, y) = from_fraction(fx, fy);
            let mut updated = match this.state.selection_easing() {
                Some(Easing::Curve(curve)) => curve,
                _ => this.ui.flow_curve,
            };
            if event.drag(cx).0 == 0 {
                (updated.x1, updated.y1) = (x, y);
            } else {
                (updated.x2, updated.y2) = (x, y);
            }
            apply(this, Easing::Curve(updated));
            cx.notify();
        }));

    let presets = EASING_PRESETS.iter().enumerate().map(|(index, (name, preset))| {
        let preset = *preset;
        Button::new(("flow-preset", index))
            .outline()
            .xsmall()
            .label(*name)
            .w_full()
            .on_click(cx.listener(move |this, _, _, cx| {
                apply(this, Easing::Curve(preset));
                cx.notify();
            }))
    });

    let status = match (selected, shared) {
        (0, _) => "Select keyframes in the timeline to apply a curve.".to_owned(),
        (count, Some(Easing::Linear)) => format!("{count} selected · Linear"),
        (count, Some(Easing::Hold)) => format!("{count} selected · Hold"),
        (count, Some(Easing::Curve(_))) => format!("{count} selected"),
        (count, None) => format!("{count} selected · mixed easing"),
    };

    div()
        .id("flow-body")
        .size_full()
        .overflow_y_scroll()
        .child(editor)
        .child(
            div()
                .px_3()
                .flex()
                .justify_between()
                .text_size(px(size::SMALL_TEXT))
                .child(div().text_color(hsla(color::MUTED)).child(status))
                .child(div().font_family("Menlo").text_color(hsla(color::VALUE)).child(format!(
                    "{:.2}, {:.2}, {:.2}, {:.2}",
                    curve.x1, curve.y1, curve.x2, curve.y2
                ))),
        )
        .child(
            div()
                .p_3()
                .grid()
                .grid_cols(3)
                .gap_1p5()
                .children(presets)
                .child(Button::new(ElementId::Name("flow-linear".into())).outline().xsmall().label("Linear").w_full().on_click(cx.listener(|this, _, _, cx| {
                    apply(this, Easing::Linear);
                    cx.notify();
                })))
                .child(Button::new(ElementId::Name("flow-hold".into())).outline().xsmall().label("Hold").w_full().on_click(cx.listener(|this, _, _, cx| {
                    apply(this, Easing::Hold);
                    cx.notify();
                }))),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_round_trip_and_clamp() {
        let (fx, fy) = to_fraction(0.25, 1.0);
        let (x, y) = from_fraction(fx, fy);
        assert!((x - 0.25).abs() < 1e-5 && (y - 1.0).abs() < 1e-5);
        assert_eq!(from_fraction(1.5, -1.0), (1.0, Y_MAX), "handles stay inside the editor");
    }
}
