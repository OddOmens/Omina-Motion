//! The Timeline panel: tracks, as in CapCut and Premiere (see `tracks`), under a transport
//! bar, a navigator (the whole composition, with the visible span) and a ruler (the work area
//! and the playhead). Each track row holds several clips side by side.
//!
//! Positions in the time graph are fractions of the visible frame range, so the graph needs
//! no pixel bookkeeping. Pointer math (scrubbing, dropping) uses the time column's bounds from
//! the previous frame.

mod tracks;

use gpui_kit::{
    AppContext as _, Context, DragMoveEvent, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    ScrollEvent, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px, relative,
};
use studio_core::model::Id;
use studio_core::time::{FrameRange, ruler_tick_step};

use super::chrome::{EmptyDragView, MeasuredBounds, Rect, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// Heights of the bars above the tracks.
const CONTROLS_BAND: f32 = 40.0;
const RULER_BAND: f32 = 28.0;
/// How close, in screen pixels, a drag has to come to something before it snaps there.
const SNAP_PIXELS: f64 = 8.0;
/// Space kept clear at the right of the time column, so handles at the composition's end sit
/// clear of the panel's resize strip (which would otherwise catch the pointer) and stay easy to grab.
const RIGHT_GUTTER: f32 = 8.0;
/// How fast Alt + scroll zooms.
const WHEEL_ZOOM_SENSITIVITY: f64 = 0.004;

/// Values captured when a drag starts, so moves can be applied relative to them.
#[derive(Clone, Debug)]
struct DragAnchor {
    pointer_x: f32,
    view: FrameRange,
    work: FrameRange,
}

/// Timeline UI state kept between frames.
#[derive(Default)]
pub struct TimelineUi {
    /// Thumbnails and waveforms for the clips.
    pub(crate) media: tracks::TrackMedia,
    /// While something is dragged over the tracks: the track and frame it would land at.
    track_drop: Option<(i32, i64)>,
    /// Clips opened to show their keyframe rows.
    pub(crate) expanded_clips: std::collections::HashSet<Id>,
    /// Bounds of the rows content (it scrolls, so its top moves with the scroll position).
    rows: MeasuredBounds,
    /// Bounds of the part of the rows that can be seen: rows far outside it aren't drawn.
    pub(crate) viewport: MeasuredBounds,
    /// Bounds of the time column (navigator, ruler and tracks share its x range).
    pub(crate) graph: MeasuredBounds,
    drag_anchor: Option<DragAnchor>,
    /// With the Cut tool, the clip under the pointer and the frame a click would cut it at.
    pub(crate) cut_hover: Option<(Id, i64)>,
}

/// Drag kinds that move something inside the timeline rather than carrying data.
#[derive(Clone, Copy, Debug)]
enum TimelineDrag {
    Scrub,
    ViewStart,
    ViewEnd,
    ViewBody,
    WorkStart,
    WorkEnd,
    /// Moving the selected keyframes in time.
    Keyframes,
    /// Trimming a clip's end, or slipping its source.
    LayerBar,
    /// Moving a clip, in time and between tracks.
    Clip,
}

/// Fraction of the visible range at which `frame` sits (0 = left edge, 1 = right edge).
fn fraction(frame: f64, view: FrameRange) -> f32 {
    ((frame - view.start) / view.len().max(1.0)) as f32
}

/// The frame under window x position `x`, given the time column's bounds.
fn frame_at(x: f32, graph: Rect, view: FrameRange) -> f64 {
    view.start + f64::from((x - graph.left) / graph.width.max(1.0)) * view.len()
}

impl Workspace {
    fn anchor_drag(&mut self, event: &MouseDownEvent) {
        self.ui.timeline.drag_anchor =
            Some(DragAnchor { pointer_x: f32::from(event.position.x), view: self.state.view_range, work: self.state.work_area });
    }

    /// Applies a timeline drag at window x position `x`.
    fn apply_timeline_drag(&mut self, drag: TimelineDrag, x: f32, y: f32) {
        let graph = self.ui.timeline.graph.get();
        let Some(anchor) = self.ui.timeline.drag_anchor.clone() else { return };
        let full = FrameRange::new(0.0, self.state.composition().duration_frames as f64);
        // Frames per pixel in the navigator (whole composition) and in the ruler (visible range).
        let navigator_delta = f64::from(x - anchor.pointer_x) / f64::from(graph.width.max(1.0)) * full.len();
        let ruler_delta = f64::from(x - anchor.pointer_x) / f64::from(graph.width.max(1.0)) * anchor.view.len();
        // Snapping reaches the same distance on screen at any zoom.
        let frames_per_pixel = self.state.view_range.len() / f64::from(graph.width.max(1.0));
        self.state.snap_threshold = (SNAP_PIXELS * frames_per_pixel).round() as i64;
        match drag {
            TimelineDrag::Scrub => {
                let frame = frame_at(x, graph, self.state.view_range).round() as i64;
                self.state.scrub_to(frame);
            }
            TimelineDrag::ViewStart => {
                self.state.set_view_range(FrameRange::new(anchor.view.start + navigator_delta, anchor.view.end));
            }
            TimelineDrag::ViewEnd => {
                self.state.set_view_range(FrameRange::new(anchor.view.start, anchor.view.end + navigator_delta));
            }
            TimelineDrag::ViewBody => self.state.set_view_range(FrameRange::new(
                anchor.view.start + navigator_delta,
                anchor.view.end + navigator_delta,
            )),
            TimelineDrag::WorkStart => {
                self.state.set_work_area(FrameRange::new(anchor.work.start + ruler_delta, anchor.work.end));
            }
            TimelineDrag::WorkEnd => {
                self.state.set_work_area(FrameRange::new(anchor.work.start, anchor.work.end + ruler_delta));
            }
            TimelineDrag::Keyframes => self.state.drag_keyframes(ruler_delta.round() as i64),
            TimelineDrag::LayerBar => self.state.drag_layer_time(ruler_delta.round() as i64),
            // A clip press starts either a move or (with Alt) a slip; only that one is active,
            // and the other call does nothing.
            TimelineDrag::Clip => {
                let track = tracks::track_at(self, y);
                self.state.drag_clip(ruler_delta.round() as i64, track);
                self.state.drag_layer_time(ruler_delta.round() as i64);
            }
        }
    }

}

/// The names of the keyframe rows an expanded clip shows, for tests.
#[cfg(test)]
pub(crate) fn animated_row_names(layer: &studio_core::model::Layer) -> Vec<String> {
    tracks::animated_properties(layer).into_iter().map(|(_, name, _)| name).collect()
}

fn tick_label(frame: i64, base: i64) -> String {
    if frame % base == 0 { format!("{:02}s", frame / base) } else { format!("{}f", frame % base) }
}

/// The work-area bar and the ruler with its ticks and the playhead handle.
fn ruler(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let view = state.view_range;
    let rate = state.composition().frame_rate;
    let graph = workspace.ui.timeline.graph.get();
    let pixels_per_frame = f64::from(graph.width) / view.len().max(1.0);
    let label_step = ruler_tick_step(pixels_per_frame, rate, 70.0);
    let subdivisions = [5, 4, 2].into_iter().find(|count| label_step % count == 0).unwrap_or(1);
    let minor_step = label_step / subdivisions;
    let first = (view.start / minor_step as f64).floor() as i64 * minor_step;
    let base = rate.timecode_base();

    let ticks = (0..)
        .map(move |index| first + index * minor_step)
        .take_while(move |frame| (*frame as f64) <= view.end)
        .map(move |frame| {
            let labeled = frame % label_step == 0;
            div()
                .absolute()
                .bottom_0()
                .left(relative(fraction(frame as f64, view)))
                .child(div().w(px(1.0)).h(px(if labeled { 8.0 } else { 4.0 })).bg(hsla(color::MUTED)))
                .when(labeled, |tick| {
                    tick.child(
                        div()
                            .absolute()
                            .bottom(px(8.0))
                            .left(px(3.0))
                            .text_size(px(10.0))
                            .text_color(hsla(color::MUTED))
                            .whitespace_nowrap()
                            .child(tick_label(frame, base)),
                    )
                })
        });

    let work = state.work_area;
    let (work_left, work_right) = (fraction(work.start, view), fraction(work.end, view));
    let playhead = fraction(state.current_frame as f64, view);
    let markers: Vec<_> = state
        .composition()
        .markers
        .iter()
        .map(|marker| fraction(marker.frame as f64, view))
        .filter(|at| (0.0..=1.0).contains(at))
        .map(|at| div().absolute().top(px(1.0)).left(relative(at)).ml(px(-3.0)).size(px(6.0)).rounded_full().bg(hsla(color::SNAP)))
        .collect();

    let edge = |id: &'static str, drag: TimelineDrag, cx: &mut Context<Workspace>| {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(6.0))
            .rounded(px(2.0))
            .bg(hsla(color::ACCENT))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.anchor_drag(event);
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
            .test_support()
    };

    div()
        .relative()
        .size_full()
        .overflow_hidden()
        .child(workspace.ui.timeline.graph.probe())
        // Work area: the frames a preview or render covers.
        .child(
            div().absolute().top_0().left_0().right_0().h(px(8.0)).bg(hsla(color::INSET)).child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(work_left))
                    .w(relative(work_right - work_left))
                    .bg(hsla(color::WORK_AREA))
                    // Its ends: bright handles that drag where playback starts and stops.
                    .child(edge("work-start", TimelineDrag::WorkStart, cx).left_0())
                    .child(edge("work-end", TimelineDrag::WorkEnd, cx).right_0()),
            ),
        )
        // Ticks; pressing or dragging here moves the playhead.
        .child(
            div()
                .id("ruler-scrub")
                .absolute()
                .top(px(8.0))
                .bottom_0()
                .left_0()
                .right_0()
                .cursor_col_resize()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.anchor_drag(event);
                    this.apply_timeline_drag(TimelineDrag::Scrub, f32::from(event.position.x), f32::from(event.position.y));
                    cx.notify();
                }))
                .on_drag(TimelineDrag::Scrub, |_, _, _, cx| cx.new(|_| EmptyDragView))
                // Frames already rendered ahead, drawn first so the numbers sit on top.
                .children(rendered_ahead(workspace))
                .children(ticks)
                // Markers (beats, say): small accent marks along the top of the ruler.
                .children(markers)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(relative(playhead))
                        .ml(px(-7.0))
                        .w(px(14.0))
                        // A white pill head on a thin white line.
                        .child(div().absolute().top_0().bottom_0().left(px(6.5)).w(px(1.0)).bg(hsla(color::PLAYHEAD)))
                        .child(div().absolute().bottom(px(2.0)).left_0().w(px(14.0)).h(px(18.0)).rounded(px(7.0)).bg(hsla(color::PLAYHEAD))),
                ),
        )
}

/// The bar above the ruler showing the whole composition; its bright span is the visible range.
fn navigator(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let duration = workspace.state.composition().duration_frames as f64;
    let full = FrameRange::new(0.0, duration);
    let view = workspace.state.view_range;
    let (left, right) = (fraction(view.start, full), fraction(view.end, full));

    let handle = |id: &'static str, drag: TimelineDrag, cx: &mut Context<Workspace>| {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(8.0))
            .bg(hsla(color::ACCENT))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.anchor_drag(event);
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
    };

    div().relative().size_full().child(
        div().absolute().left_0().right_0().top(px(14.0)).h(px(12.0)).rounded(px(2.0)).bg(hsla(color::INSET)).child(
            div()
                .id("navigator-thumb")
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(left))
                .w(relative(right - left))
                .rounded(px(2.0))
                .bg(hsla(color::NAVIGATOR_THUMB))
                .cursor_grab()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, _| this.anchor_drag(event)))
                .on_drag(TimelineDrag::ViewBody, |_, _, _, cx| cx.new(|_| EmptyDragView))
                .child(handle("view-start", TimelineDrag::ViewStart, cx).left_0().rounded_l(px(2.0)))
                .child(handle("view-end", TimelineDrag::ViewEnd, cx).right_0().rounded_r(px(2.0))),
        ),
    )
}

pub fn render(workspace: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ui = &workspace.ui.timeline;
    let band = |height: f32| div().h(px(height)).flex_none().flex().border_b_1().border_color(hsla(color::HAIRLINE));

    let rows_content = div()
        .id("timeline-rows-content")
        .relative()
        .min_h_full()
        .flex()
        .pr(px(RIGHT_GUTTER))
        .child(ui.rows.probe())
        // Alt + scroll zooms around the pointer.
        .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
            if !event.modifiers.alt {
                // Plain scrolling: rows coming into view are drawn (only those near the view
                // are), once now and again when the new position has been measured.
                this.notify_timeline(cx);
                cx.on_next_frame(window, |this, _, cx| this.notify_timeline(cx));
                return;
            }
            let graph = this.ui.timeline.graph.get();
            let delta = f64::from(f32::from(event.delta.pixel_delta(px(16.0)).y));
            let view = this.state.view_range;
            let anchor = frame_at(f32::from(event.position.x), graph, view);
            let factor = (delta * WHEEL_ZOOM_SENSITIVITY).exp();
            this.state.set_view_range(FrameRange::new(anchor - (anchor - view.start) * factor, anchor + (view.end - anchor) * factor));
            cx.stop_propagation();
            cx.notify();
        }))
        .child(tracks::rows(workspace, window, cx));

    let body = div()
        .id("timeline-body")
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<TimelineDrag>, _, cx| {
            let drag = *event.drag(cx);
            this.apply_timeline_drag(drag, f32::from(event.event.position.x), f32::from(event.event.position.y));
            cx.notify();
        }))
        .child(band(CONTROLS_BAND).child(div().flex_1().min_w_0().child(tracks::toolbar(workspace, cx))))
        .child(band(CONTROLS_BAND).child(div().flex_1().min_w_0().mr(px(RIGHT_GUTTER)).child(navigator(workspace, cx))))
        .child(band(RULER_BAND).child(div().flex_1().min_w_0().mr(px(RIGHT_GUTTER)).child(ruler(workspace, cx))))
        .child(div().relative().flex_1().min_h_0().child(ui.viewport.probe()).child(div().id("timeline-rows").size_full().overflow_y_scroll().child(rows_content)))
        .children(empty_hint(workspace));

    let footer = div()
        .flex()
        .items_center()
        .w_full()
        // Measured around the viewer's last render.
        .when(workspace.ui.settings.show_render_time, |footer| {
            footer.child(div().px_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(match workspace.render.last_render_ms {
                Some(ms) => format!("Frame Render Time: {ms:.1} ms"),
                None => "Frame Render Time: –".to_owned(),
            }))
        });

    // The composition's name is in the toolbar row, beside the transport.
    panel(PanelId::Timeline, &[], Some(footer.into_any_element()), body, workspace, cx)
}

/// Green lines along the ruler, behind its numbers, where frames have been rendered ahead (as
/// in After Effects). They sit in the ruler rather than the work-area bar so the bar never hides them.
fn rendered_ahead(workspace: &Workspace) -> Vec<gpui_kit::Div> {
    let view = workspace.state.view_range;
    workspace
        .render
        .preview
        .ranges()
        .into_iter()
        .map(|(start, end)| (fraction(start as f64, view), fraction(end as f64, view)))
        .filter(|(left, right)| *right > 0.0 && *left < 1.0)
        .map(|(left, right)| div().absolute().top(px(4.0)).h(px(2.0)).left(relative(left.max(0.0))).w(relative(right.min(1.0) - left.max(0.0))).bg(gpui_kit::rgb(0x3ccf6e)))
        .collect()
}

/// What to do in an empty composition, over its empty tracks (the tracks still take drops).
fn empty_hint(workspace: &Workspace) -> Option<impl IntoElement + use<>> {
    if !workspace.state.composition().layers.is_empty() {
        return None;
    }
    let keys = &workspace.ui.settings.shortcuts;
    let key = |id: &str| crate::keymap::key_for(keys, id).map(|key| format!(" ({key})")).unwrap_or_default();
    Some(
        div()
            .id("timeline-empty-hint")
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .top(px(CONTROLS_BAND * 2.0 + RULER_BAND))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(hsla(color::MUTED))
            .child(div().text_color(hsla(color::TEXT)).child("This composition is empty"))
            .child(format!("Drag footage, pictures or sound here from the Project panel or the Finder, add text{} or a shape{}, or start from a template.", key("type-tool"), key("shape-tool")))
            .test_support(),
    )
}

/// Zooms the visible range by `factor`, centered on the playhead.
fn zoom_around_playhead(workspace: &mut Workspace, factor: f64) {
    let view = workspace.state.view_range;
    let center = workspace.state.current_frame as f64;
    let half = view.len() * factor / 2.0;
    workspace.state.set_view_range(FrameRange::new(center - half, center + half));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_and_frames_round_trip() {
        let view = FrameRange::new(48.0, 96.0);
        assert_eq!(fraction(72.0, view), 0.5);
        let graph = Rect { left: 100.0, top: 0.0, width: 480.0, height: 20.0 };
        assert_eq!(frame_at(340.0, graph, view), 72.0);
    }

    #[test]
    fn tick_labels_show_seconds_and_frames() {
        assert_eq!(tick_label(48, 24), "02s");
        assert_eq!(tick_label(54, 24), "6f");
    }
}
