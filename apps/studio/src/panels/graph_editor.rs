//! The graph editor, in the Motion tab of the right panel. Its Keyframe Easing section eases
//! the keyframes selected in the timeline: the curve on top, whose two handles drag to make a
//! custom curve, and the easing list under it: Linear, Hold, the classic families (Quad to
//! Back) and Elastic and Bounce, each with In / Out / Both. The selection's segments are the
//! stretches between consecutive selected keyframes (see `eased_segments`). When it has more
//! than one, say a fade in and a fade out, a row of small curves picks which one to edit (the
//! first that changes value, to start) or All of them; choices apply only to that. The same
//! curve editor, smaller, eases a layer's In and Out animations in the Animate section
//! (`CurveTarget::Motion`).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, Div, DragMoveEvent, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, PathBuilder, Pixels, Point, StatefulInteractiveElement, Styled, TestSupportExt as _, canvas, div, point,
    prelude::FluentBuilder as _, px, relative,
};
use studio_core::animation::{CubicBezier, EASING_FAMILIES, EaseMode, Easing};
use studio_core::presets::Phase;

use super::chrome::EmptyDragView;
use crate::state::KeyframeRef;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Which of the selection's segments the editor works on, when it has several.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentFocus {
    /// Every segment at once.
    All,
    /// Only the segment leaving this keyframe.
    One(KeyframeRef),
}

/// The segments the editor shows and eases: all of the selection's when it has one or All is
/// picked; otherwise the one picked, or to start, the first whose value changes.
pub fn focused_segments(workspace: &Workspace) -> Vec<KeyframeRef> {
    let segments = workspace.state.eased_segments();
    if segments.len() <= 1 {
        return segments;
    }
    // A pick lasts as long as the selection it was made for.
    let picked = workspace.ui.segment_focus.as_ref().filter(|(selection, _)| *selection == workspace.state.selected_keyframes).map(|(_, focus)| *focus);
    match picked {
        Some(SegmentFocus::All) => segments,
        Some(SegmentFocus::One(key)) if segments.contains(&key) => vec![key],
        _ => {
            let first = segments.iter().copied().find(|key| workspace.state.segment_changes(*key)).unwrap_or(segments[0]);
            vec![first]
        }
    }
}

/// Whether the timeline's line for the segment leaving `key` is the one being eased, to mark
/// it while the graph editor is in view and the selection has more than one segment.
pub fn is_focused_segment(workspace: &Workspace, key: KeyframeRef) -> bool {
    showing(workspace) && workspace.state.eased_segments().len() > 1 && focused_segments(workspace).contains(&key)
}

/// What a curve editor eases: the keyframes selected in the timeline, or the selected layer's
/// In or Out animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveTarget {
    Keyframes,
    Motion(Phase),
}

/// Vertical room above and below the unit square, so overshooting curves stay visible.
const Y_MIN: f32 = -0.4;
const Y_MAX: f32 = 1.4;

/// Drag payload for a curve handle: which editor it belongs to, and 0 for the first control
/// point or 1 for the second.
#[derive(Clone, Copy, Debug)]
pub struct HandleDrag {
    target: CurveTarget,
    index: usize,
}

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

/// A curve handle: a dot to drag. The keyframe editor's are `graph-handle` 0 and 1; an In or
/// Out editor's are named after its end.
fn handle(target: CurveTarget, index: usize, x: f32, y: f32) -> impl IntoElement + use<> {
    let (fx, fy) = to_fraction(x, y);
    let id = match target {
        CurveTarget::Keyframes => ElementId::from(("graph-handle", index)),
        CurveTarget::Motion(phase) => ElementId::Name(format!("motion-{}-handle-{index}", phase_key(phase)).into()),
    };
    div()
        .id(id)
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
        .on_drag(HandleDrag { target, index }, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .test_support()
}

/// "in" or "out", for naming an end's controls.
pub(super) fn phase_key(phase: Phase) -> &'static str {
    match phase {
        Phase::In => "in",
        Phase::Out => "out",
    }
}

/// A curve editor `height` tall filling its width: the grid, the easing it shows and, for a
/// curve, its two handles. Dragging a handle turns `target`'s easing into a custom curve.
pub(super) fn curve_editor(target: CurveTarget, shown: Easing, height: f32, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let curve = match shown {
        Easing::Curve(curve) => Some(curve),
        _ => None,
    };
    let id = match target {
        CurveTarget::Keyframes => ElementId::from("graph-editor"),
        CurveTarget::Motion(phase) => ElementId::Name(format!("motion-{}-curve", phase_key(phase)).into()),
    };
    div()
        .id(id)
        .relative()
        .w_full()
        .h(px(height))
        .rounded(px(6.0))
        .bg(hsla(color::INSET))
        .child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_easing(shown, bounds, window)).absolute().size_full())
        .children(curve.map(|curve| handle(target, 0, curve.x1, curve.y1)))
        .children(curve.map(|curve| handle(target, 1, curve.x2, curve.y2)))
        .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<HandleDrag>, _, cx| {
            // Every editor hears every handle drag; each answers only its own.
            let drag = *event.drag(cx);
            if drag.target != target {
                return;
            }
            let bounds = event.bounds;
            let fx = f32::from(event.event.position.x - bounds.origin.x) / f32::from(bounds.size.width).max(1.0);
            let fy = f32::from(event.event.position.y - bounds.origin.y) / f32::from(bounds.size.height).max(1.0);
            let (x, y) = from_fraction(fx, fy);
            let mut updated = match eased(this, target) {
                Some(Easing::Curve(curve)) => curve,
                _ => starting_curve(this, target),
            };
            if drag.index == 0 {
                (updated.x1, updated.y1) = (x, y);
            } else {
                (updated.x2, updated.y2) = (x, y);
            }
            set_easing(this, target, Easing::Curve(updated));
            cx.notify();
        }))
}

/// The one layer selected, if exactly one is.
fn single_layer(workspace: &Workspace) -> Option<studio_core::model::Id> {
    match workspace.state.selected_layers.as_slice() {
        [id] => Some(*id),
        _ => None,
    }
}

/// The easing `target` has now: the keyframe selection's (or the last curve used), or the
/// selected layer's In or Out motion's.
fn eased(workspace: &Workspace, target: CurveTarget) -> Option<Easing> {
    match target {
        CurveTarget::Keyframes => Some(shown_easing(workspace)),
        CurveTarget::Motion(phase) => {
            let layer = workspace.state.composition().layer(single_layer(workspace)?)?;
            studio_core::presets::motion_easing(&layer.animation, phase)
        }
    }
}

/// The curve a handle drag starts from when the easing isn't a curve (Linear, a spring and so
/// on): the last one used for keyframes, or a quick settle (In) or speed-away (Out) for motions.
fn starting_curve(workspace: &Workspace, target: CurveTarget) -> CubicBezier {
    let expo = EASING_FAMILIES.iter().find(|(name, _)| *name == "Expo").map(|(_, forms)| *forms);
    match (target, expo) {
        (CurveTarget::Motion(Phase::In), Some(forms)) => forms[1],
        (CurveTarget::Motion(Phase::Out), Some(forms)) => forms[0],
        _ => workspace.ui.flow_curve,
    }
}

/// Gives `target` the easing `easing`.
pub(super) fn set_easing(workspace: &mut Workspace, target: CurveTarget, easing: Easing) {
    match target {
        CurveTarget::Keyframes => apply(workspace, easing),
        CurveTarget::Motion(phase) => {
            if let Some(layer) = single_layer(workspace) {
                workspace.state.set_motion_easing(layer, phase, Some(easing));
            }
        }
    }
}

/// Applies `easing` to the segments being edited and remembers curves for the editor.
fn apply(workspace: &mut Workspace, easing: Easing) {
    if let Easing::Curve(curve) = easing {
        workspace.ui.flow_curve = curve;
    }
    let segments = focused_segments(workspace);
    workspace.state.apply_easing_to(&segments, easing);
}

/// The easing the editor shows: the edited segments' own, if they share one, otherwise the
/// last curve used.
fn shown_easing(workspace: &Workspace) -> Easing {
    workspace.state.segments_easing(&focused_segments(workspace)).unwrap_or(Easing::Curve(workspace.ui.flow_curve))
}

/// Shows the graph editor: the Motion tab, unfolded, with its Keyframe Easing section open.
pub fn show(workspace: &mut Workspace) {
    if !workspace.has(crate::features::Feature::GraphEditor) {
        return;
    }
    workspace.ui.tabs.insert(PanelId::Properties, super::motion::TAB);
    workspace.ui.collapsed_panels.remove(&PanelId::Properties);
    workspace.ui.open_section("easing");
}

/// Whether the graph editor is in view: the Motion tab showing, and its panel unfolded.
pub fn showing(workspace: &Workspace) -> bool {
    workspace.ui.tab(PanelId::Properties) == super::motion::TAB && !workspace.ui.collapsed_panels.contains(&PanelId::Properties)
}

/// The timeline's Easings button: shows the graph editor in the Motion tab.
pub fn button(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let open = showing(workspace);
    div()
        .id("easings")
        .size(px(26.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .when(open, |button| button.bg(hsla(color::ROW_SELECTED)))
        .hover(|style| style.bg(hsla(color::HOVER)))
        .tooltip(|window, cx| gpui_kit::component::tooltip::Tooltip::new("Graph Editor (in the Motion tab)").build(window, cx))
        .on_click(cx.listener(|this, _, _, cx| {
            show(this);
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

/// The Keyframe Easing section: the selected keyframes' curve and the easing list, or a note
/// on how to pick keyframes when none are.
pub fn keyframes_section(workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let selected = workspace.state.selected_keyframes.len();
    if selected == 0 {
        return div().child(super::properties::status_line("Select keyframes in the timeline to ease them: click the line between two, or double-click one."));
    }
    let shown = shown_easing(workspace);
    let picker = segment_picker(workspace, cx);
    let custom = matches!(shown, Easing::Curve(curve) if !EASING_FAMILIES.iter().any(|(_, forms)| forms.contains(&curve)));
    let editing = focused_segments(workspace).len();
    let segments = workspace.state.eased_segments().len();
    let scope = if segments > 1 && editing == 1 { "editing 1 segment".to_owned() } else { format!("{selected} keyframe{} selected", if selected == 1 { "" } else { "s" }) };
    let status = format!("{scope}{}", if custom { " · custom curve" } else { "" });
    let numbers = match shown {
        Easing::Curve(c) => Some(format!("{:.2}, {:.2}, {:.2}, {:.2}", c.x1, c.y1, c.x2, c.y2)),
        _ => None,
    };
    // The easing list, two to a line so it fits beside nothing in a narrow panel.
    let mut entries: Vec<AnyElement> = vec![row("Linear", 0, None, Some(Easing::Linear), shown, cx), row("Hold", 1, None, Some(Easing::Hold), shown, cx)];
    for (index, (name, forms)) in EASING_FAMILIES.iter().enumerate() {
        entries.push(row(name, 2 + index, Some(forms.map(Easing::Curve)), None, shown, cx));
    }
    entries.push(row("Elastic", 20, Some(EaseMode::ALL.map(Easing::Elastic)), None, shown, cx));
    entries.push(row("Bounce", 21, Some(EaseMode::ALL.map(Easing::Bounce)), None, shown, cx));
    let list = div().flex().flex_wrap().children(entries.into_iter().map(|entry| div().w(relative(0.5)).child(entry)));
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(picker)
        .child(curve_editor(CurveTarget::Keyframes, shown, 220.0, cx))
        .child(
            div()
                .flex()
                .justify_between()
                .gap_2()
                .text_size(px(size::SMALL_TEXT))
                .child(div().text_color(hsla(color::MUTED)).child(status))
                .children(numbers.map(|text| div().font_family("Menlo").text_color(hsla(color::VALUE)).child(text))),
        )
        .child(list)
}

/// The row of segments to pick from when the selection has more than one: All, then each
/// segment as a small curve with its property and when it runs. Segments that hold one value
/// are dimmed, since easing them changes nothing. `None` with one segment or none.
fn segment_picker(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<Div> {
    let segments = workspace.state.eased_segments();
    if segments.len() <= 1 {
        return None;
    }
    let focused = focused_segments(workspace);
    let all = focused.len() > 1;
    let composition = workspace.state.composition();
    let fps = composition.frame_rate.fps();
    let seconds = |frame: i64| format!("{:.1}", frame as f64 / fps);
    let pick = move |focus: SegmentFocus| {
        move |this: &mut Workspace, _: &gpui_kit::ClickEvent, _: &mut gpui_kit::Window, cx: &mut Context<Workspace>| {
            this.ui.segment_focus = Some((this.state.selected_keyframes.clone(), focus));
            cx.notify();
        }
    };
    let tile = |id: ElementId, on: bool| {
        div()
            .id(id)
            .w(px(96.0))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .rounded(px(6.0))
            .border_1()
            .border_color(hsla(if on { color::ACCENT } else { color::HAIRLINE }))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .cursor_pointer()
    };
    let mut tiles = vec![
        tile(ElementId::Name("easing-segment-all".into()), all)
            .justify_center()
            .items_center()
            .min_h(px(68.0))
            .child(div().text_color(hsla(color::TEXT)).child("All"))
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} segments", segments.len())))
            .on_click(cx.listener(pick(SegmentFocus::All)))
            .test_support()
            .into_any_element(),
    ];
    for (index, key) in segments.iter().copied().enumerate() {
        let Some(layer) = composition.layer(key.layer) else { continue };
        let name = layer.property(key.path, key.frame).map_or_else(String::new, |snapshot| snapshot.name);
        let end = workspace.state.segment_end(key).unwrap_or(key.frame);
        let changes = workspace.state.segment_changes(key);
        let easing = workspace.state.segments_easing(&[key]).unwrap_or(Easing::Linear);
        let tip = format!("{} · {name}{}", layer.name, if changes { "" } else { " · holds one value, so easing it changes nothing" });
        tiles.push(
            tile(ElementId::from(("easing-segment", index)), !all && focused.contains(&key))
                .when(!changes, |tile| tile.opacity(0.55))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .child(div().h(px(36.0)).rounded(px(4.0)).bg(hsla(color::INSET)).child(canvas(|_, _, _| (), move |bounds, _, window, _| paint_easing(easing, bounds, window)).size_full()))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).truncate().child(name))
                .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{}–{} s", seconds(key.frame), seconds(end))))
                .on_click(cx.listener(pick(SegmentFocus::One(key))))
                .test_support()
                .into_any_element(),
        );
    }
    Some(div().flex().flex_wrap().gap_1().children(tiles))
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
