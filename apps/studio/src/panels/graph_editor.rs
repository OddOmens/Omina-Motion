//! The graph editor: a popover from the timeline's Easings button, as in motion tools. On
//! the left, the easing list: Linear, Hold, the classic families (Quad to Back) and Elastic and
//! Bounce, each with In / Out / Both. On the right, the selection's curve, whose two handles
//! drag to make a custom curve. Choices apply at once to the segments leaving each selected
//! keyframe (Flow semantics: between consecutive selected keyframes, see `apply_easing`).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, PathBuilder, Pixels, Point, StatefulInteractiveElement, Styled, TestSupportExt as _, anchored,
    canvas, deferred, div, point, prelude::FluentBuilder as _, px, relative,
};
use studio_core::animation::{CubicBezier, EASING_FAMILIES, EaseMode, Easing};

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

/// Draws the grid, the linear reference, the handle arms (for curves) and the easing itself.
fn paint_easing(easing: Easing, bounds: Bounds<Pixels>, window: &mut gpui_kit::Window) {
    let at = |x: f32, y: f32| {
        let (fx, fy) = to_fraction(x, y);
        point(bounds.origin.x + bounds.size.width * fx, bounds.origin.y + bounds.size.height * fy)
    };
    let line = |from: Point<Pixels>, to: Point<Pixels>, width: f32, color: u32, window: &mut gpui_kit::Window| {
        let mut path = PathBuilder::stroke(px(width));
        path.move_to(from);
        path.line_to(to);
        if let Ok(path) = path.build() {
            window.paint_path(path, hsla(color));
        }
    };
    // A grid over the unit square, and the straight linear curve.
    for step in 0..=8 {
        let v = step as f32 / 8.0;
        line(at(v, 0.0), at(v, 1.0), 1.0, color::HAIRLINE, window);
        line(at(0.0, v), at(1.0, v), 1.0, color::HAIRLINE, window);
    }
    line(at(0.0, 0.0), at(1.0, 1.0), 1.0, color::HAIRLINE, window);
    if let Easing::Curve(curve) = easing {
        line(at(0.0, 0.0), at(curve.x1, curve.y1), 1.0, color::MUTED, window);
        line(at(1.0, 1.0), at(curve.x2, curve.y2), 1.0, color::MUTED, window);
    }
    // The easing, sampled finely enough for Elastic's wiggles and Hold's step.
    let mut path = PathBuilder::stroke(px(2.0));
    path.move_to(at(0.0, easing.apply(0.0)));
    for step in 1..=160 {
        let t = step as f32 / 160.0;
        // Hold jumps at the very end of its segment.
        let y = if easing == Easing::Hold && step == 160 { 1.0 } else { easing.apply(t) };
        path.line_to(at(t, y.clamp(Y_MIN, Y_MAX)));
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, hsla(color::ACCENT));
    }
}

fn handle(index: usize, x: f32, y: f32) -> impl IntoElement + use<> {
    let (fx, fy) = to_fraction(x, y);
    div()
        .id(("graph-handle", index))
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
        .test_support()
}

/// Applies `easing` to the selected keyframes and remembers curves for the editor.
fn apply(workspace: &mut Workspace, easing: Easing) {
    if let Easing::Curve(curve) = easing {
        workspace.ui.flow_curve = curve;
    }
    workspace.state.apply_easing(easing);
}

/// The easing the editor shows: the selection's own, if its segments share one, otherwise the
/// last curve used.
fn shown_easing(workspace: &Workspace) -> Easing {
    workspace.state.selection_easing().unwrap_or(Easing::Curve(workspace.ui.flow_curve))
}

/// The timeline's Easings button: opens the graph editor below it.
pub fn button(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let open = workspace.ui.graph_editor.is_some();
    div()
        .id("easings")
        .size(px(26.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .when(open, |button| button.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.ui.graph_editor = match this.ui.graph_editor {
                Some(_) => None,
                // Open just below the button.
                None => Some(point(event.position.x - px(12.0), event.position.y + px(18.0))),
            };
            cx.notify();
        }))
        .child(Icon::new(IconName::Spline).size(px(15.0)).text_color(hsla(if open { color::TEXT } else { color::MUTED })))
        .test_support()
}

/// One easing row in the list: its name, and In / Out / Both for the families that have them.
/// The row matching the selection is checked, with its form highlighted.
fn row(name: &'static str, id: usize, forms: Option<[Easing; 3]>, single: Option<Easing>, shown: Easing, cx: &mut Context<Workspace>) -> AnyElement {
    let current = forms.and_then(|forms| forms.iter().position(|form| *form == shown));
    let checked = current.is_some() || single == Some(shown);
    let primary = single.or(forms.map(|forms| forms[2]));
    let mut row = div()
        .id(("easing-row", id))
        .h(px(26.0))
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(px(5.0))
        .when(checked, |row| row.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .child(div().w(px(12.0)).children(checked.then(|| Icon::new(IconName::Check).size(px(12.0)).text_color(hsla(color::TEXT)))))
        .child(div().flex_1().text_color(hsla(color::TEXT)).child(name));
    // Clicking the name applies the family's in-and-out form (or the easing itself).
    if let Some(easing) = primary {
        row = row.on_click(cx.listener(move |this, _, _, cx| {
            apply(this, easing);
            cx.notify();
        }));
    }
    if let Some(forms) = forms {
        for (index, (mode, form)) in EaseMode::ALL.into_iter().zip(forms).enumerate() {
            let on = current == Some(index);
            row = row.child(
                div()
                    .id(ElementId::Name(format!("easing:{name}:{}", mode.label()).into()))
                    .px_1()
                    .rounded(px(3.0))
                    .text_size(px(size::SMALL_TEXT))
                    .text_color(hsla(if on { color::TEXT } else { color::MUTED }))
                    .when(on, |chip| chip.bg(hsla(color::ACCENT)))
                    .hover(|style| style.text_color(hsla(color::TEXT)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        apply(this, form);
                        cx.notify();
                    }))
                    .child(mode.label())
                    .test_support(),
            );
        }
    }
    row.test_support().into_any_element()
}

fn rule() -> gpui_kit::Div {
    div().h(px(1.0)).my_1().mx_2().bg(hsla(color::HAIRLINE))
}

/// The popover, while open.
pub fn popover(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let at = workspace.ui.graph_editor?;
    let shown = shown_easing(workspace);
    let selected = workspace.state.selected_keyframes.len();

    let mut list = div().w(px(240.0)).flex_none().flex().flex_col().p_1().gap_0p5().border_r_1().border_color(hsla(color::HAIRLINE));
    // "Graph Editor": a custom curve, made by dragging the handles.
    let custom = matches!(shown, Easing::Curve(curve) if !EASING_FAMILIES.iter().any(|(_, forms)| forms.contains(&curve)));
    list = list
        .child(
            div()
                .h(px(28.0))
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .rounded(px(5.0))
                .border_1()
                .border_color(hsla(if custom { color::ACCENT } else { color::HAIRLINE }))
                .child(Icon::new(IconName::Spline).size(px(13.0)).text_color(hsla(color::MUTED)))
                .child("Graph Editor"),
        )
        .child(rule())
        .child(row("Linear", 0, None, Some(Easing::Linear), shown, cx))
        .child(row("Hold", 1, None, Some(Easing::Hold), shown, cx))
        .child(rule());
    for (index, (name, forms)) in EASING_FAMILIES.iter().enumerate() {
        list = list.child(row(name, 2 + index, Some(forms.map(Easing::Curve)), None, shown, cx));
    }
    list = list
        .child(rule())
        .child(row("Elastic", 20, Some(EaseMode::ALL.map(Easing::Elastic)), None, shown, cx))
        .child(row("Bounce", 21, Some(EaseMode::ALL.map(Easing::Bounce)), None, shown, cx));

    let curve = match shown {
        Easing::Curve(curve) => Some(curve),
        _ => None,
    };
    let editor = div()
        .id("graph-editor")
        .relative()
        .size(px(280.0))
        .rounded(px(6.0))
        .bg(hsla(color::INSET))
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_easing(shown, bounds, window)).absolute().size_full())
        .children(curve.map(|curve| handle(0, curve.x1, curve.y1)))
        .children(curve.map(|curve| handle(1, curve.x2, curve.y2)))
        // Dragging a handle makes the selection's curve a custom one.
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<HandleDrag>, _, cx| {
            let bounds = event.bounds;
            let fx = f32::from(event.event.position.x - bounds.origin.x) / f32::from(bounds.size.width).max(1.0);
            let fy = f32::from(event.event.position.y - bounds.origin.y) / f32::from(bounds.size.height).max(1.0);
            let (x, y) = from_fraction(fx, fy);
            let mut updated: CubicBezier = match shown_easing(this) {
                Easing::Curve(curve) => curve,
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
    let status = if selected == 0 { "Select keyframes to ease them.".to_owned() } else { format!("{selected} keyframes selected") };
    let numbers = curve.map(|c| format!("{:.2}, {:.2}, {:.2}, {:.2}", c.x1, c.y1, c.x2, c.y2));
    let right = div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .child(editor)
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(px(size::SMALL_TEXT))
                .child(div().text_color(hsla(color::MUTED)).child(status))
                .children(numbers.map(|text| div().font_family("Menlo").text_color(hsla(color::VALUE)).child(text))),
        );

    let body = div()
        .id("graph-editor-popover")
        .flex()
        .rounded(px(10.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
        .text_size(px(size::TEXT))
        // Clicking anywhere else closes it.
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.ui.graph_editor = None;
            cx.notify();
        }))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(list)
        .child(right)
        .test_support();
    Some(deferred(anchored().position(at).snap_to_window().child(body)).with_priority(2))
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
