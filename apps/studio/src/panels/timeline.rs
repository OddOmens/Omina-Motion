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
    ScrollEvent, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px, relative,
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
    /// Bounds of the time column (navigator, ruler and tracks share its x range).
    pub(crate) graph: MeasuredBounds,
    drag_anchor: Option<DragAnchor>,
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

    let edge = |id: &'static str, drag: TimelineDrag, cx: &mut Context<Workspace>| {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(8.0))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.anchor_drag(event);
            }))
            .on_drag(drag, |_, _, _, cx| cx.new(|_| EmptyDragView))
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
                    .child(edge("work-start", TimelineDrag::WorkStart, cx).left(px(-4.0)))
                    .child(edge("work-end", TimelineDrag::WorkEnd, cx).right(px(-4.0))),
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
                .children(ticks)
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
        .child(ui.rows.probe())
        // Alt + scroll zooms around the pointer.
        .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| {
            if !event.modifiers.alt {
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
        .child(band(CONTROLS_BAND).child(div().flex_1().min_w_0().child(navigator(workspace, cx))))
        .child(band(RULER_BAND).child(div().flex_1().min_w_0().child(ruler(workspace, cx))))
        .child(div().id("timeline-rows").flex_1().min_h_0().overflow_y_scroll().child(rows_content))
        .children(super::graph_editor::popover(workspace, cx));

    let footer = div()
        .flex()
        .items_center()
        .w_full()
        // Measured around the viewer's last render.
        .child(div().px_1().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(match workspace.render.last_render_ms {
            Some(ms) => format!("Frame Render Time: {ms:.1} ms"),
            None => "Frame Render Time: –".to_owned(),
        }));

    let comp_name = workspace.state.composition().name.clone();
    panel(PanelId::Timeline, &[comp_name.as_str()], Some(footer.into_any_element()), body, workspace, cx)
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
