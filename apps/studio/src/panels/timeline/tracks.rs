//! The timeline's Tracks view, styled after modern motion tools and CapCut: one row per
//! track, each holding any number of clips side by side (see `studio_core::tracks`). Clips are
//! brightly colored by kind, with a header (expand arrow, kind icon, name, and mute / show /
//! lock toggles). Video clips show a filmstrip of thumbnails, sound its waveform. Expanding a
//! clip lists its animated properties under it, with their keyframes.
//!
//! Clips drag in time and between tracks (a clip that would overlap another gets a new track),
//! their ends trim, Alt-drag slips the source, keyframes drag in time, and project items drop
//! straight onto a track. Above them are a transport bar, the navigator and the ruler (see
//! the parent module).

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, Div, DragMoveEvent, ImageSource, InteractiveElement, IntoElement, MouseButton,
    MouseDownEvent, ParentElement, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, fill,
    img, point, prelude::FluentBuilder as _, px, relative, rgba, size as gpui_size,
};
use studio_core::media::VideoFrame;
use studio_core::model::{Id, Layer, LayerContent};
use studio_core::property::PropertyPath;
use studio_core::time::{FrameRange, format_timecode};
use studio_core::trim_tools::End;
use studio_core::tracks::{self, MAIN_TRACK};
use studio_media::{PeakCache, Peaks, Thumbnailer};

use super::{TimelineDrag, fraction, frame_at, zoom_around_playhead};
use crate::panels::chrome::{EmptyDragView, icon_button};
use crate::state::{KeyframeRef, LayerTimeEdit, Switch};
use crate::theme::{color, hsla};
use crate::workspace::{EffectDrag, ProjectItemDrag, Tool, Workspace};

/// Row heights: the main track is tallest, as in CapCut; sound tracks are shorter.
const MAIN_HEIGHT: f32 = 58.0;
const PICTURE_HEIGHT: f32 = 48.0;
const SOUND_HEIGHT: f32 = 40.0;
/// Each keyframe row under an expanded clip: a small name on top, keyframes on a line below it.
const PROPERTY_ROW: f32 = 38.0;
/// Where a property row's keyframe line runs, from the row's top.
const KEY_LINE: f32 = 27.0;
/// The keyframes' clickable size (larger than the diamond drawn, so they are easy to grab).
const KEY_TARGET: f32 = 20.0;
/// Height of filmstrip thumbnails, in pixels.
pub const THUMBNAIL_HEIGHT: u32 = 36;
/// Height of a clip's header strip.
const CLIP_HEADER: f32 = 18.0;
/// Height of the waveform under a video clip that has sound.
const CLIP_SOUND_STRIP: f32 = 10.0;
/// Grips at each end of a clip that trim it.
const TRIM_GRIP: f32 = 6.0;
/// Most thumbnail images kept on the GPU at once.
const IMAGE_CACHE_LIMIT: usize = 600;

/// The media behind the Tracks view: thumbnails and waveforms decoded in the background, and
/// the GPU images made from them.
pub struct TrackMedia {
    pub thumbnails: Thumbnailer,
    pub peaks: PeakCache,
    images: RefCell<ImageCache>,
}

/// GPU images for thumbnails, keyed by the decoded picture they came from. Each entry keeps
/// its picture alive, so a key (the picture's address) can never be reused for another one.
#[derive(Default)]
struct ImageCache {
    images: HashMap<usize, (Arc<VideoFrame>, Arc<RenderImage>)>,
    order: VecDeque<usize>,
    /// Images dropped from the cache, to be freed on the GPU at the next frame.
    retired: Vec<Arc<RenderImage>>,
}

impl Default for TrackMedia {
    fn default() -> Self {
        Self { thumbnails: Thumbnailer::new(THUMBNAIL_HEIGHT), peaks: PeakCache::new(), images: RefCell::default() }
    }
}

impl TrackMedia {
    /// The GPU image for a decoded thumbnail, made once.
    fn image(&self, frame: &Arc<VideoFrame>) -> Arc<RenderImage> {
        let key = Arc::as_ptr(frame) as usize;
        let mut cache = self.images.borrow_mut();
        if let Some((_, image)) = cache.images.get(&key) {
            return image.clone();
        }
        // GPUI's images are BGRA; thumbnails come as RGBA.
        let mut bgra = frame.rgba.clone();
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(frame.width, frame.height, bgra).expect("a thumbnail holds width × height pixels");
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
        cache.images.insert(key, (frame.clone(), image.clone()));
        cache.order.push_back(key);
        while cache.order.len() > IMAGE_CACHE_LIMIT {
            if let Some(old) = cache.order.pop_front()
                && let Some((_, image)) = cache.images.remove(&old)
            {
                cache.retired.push(image);
            }
        }
        image
    }

    /// Frees the GPU copies of thumbnails that left the cache.
    pub fn free_retired(&self, window: &mut Window, cx: &mut gpui_kit::App) {
        for image in self.images.borrow_mut().retired.drain(..) {
            cx.drop_image(image, Some(window));
        }
    }
}

fn base_height(track: i32) -> f32 {
    match track {
        MAIN_TRACK => MAIN_HEIGHT,
        track if track < MAIN_TRACK => SOUND_HEIGHT,
        _ => PICTURE_HEIGHT,
    }
}

/// The properties shown under an expanded clip: the animated ones, in panel order.
fn animated_properties(layer: &Layer) -> Vec<(PropertyPath, String, Vec<i64>)> {
    layer
        .property_paths()
        .into_iter()
        .filter_map(|path| layer.property(path, 0).filter(|property| !property.keyframes.is_empty()).map(|property| (path, property.name, property.keyframes)))
        .collect()
}

/// Every row top to bottom with its height: a track's own height plus room for the keyframe
/// rows of any expanded clip on it.
fn layout(workspace: &Workspace) -> Vec<(i32, f32)> {
    let composition = workspace.state.composition();
    let expanded = &workspace.ui.timeline.expanded_clips;
    tracks::tracks(composition)
        .into_iter()
        .map(|track| {
            let rows = composition
                .layers
                .iter()
                .filter(|layer| layer.track == track && expanded.contains(&layer.id))
                .map(|layer| animated_properties(layer).len().max(1))
                .max()
                .unwrap_or(0);
            (track, base_height(track) + rows as f32 * PROPERTY_ROW)
        })
        .collect()
}

/// The track under window y position `y`. Above the top row is a new track above it; below
/// the bottom row, a new track below it.
pub fn track_at(workspace: &Workspace, y: f32) -> i32 {
    let rows = layout(workspace);
    let mut top = workspace.ui.timeline.rows.get().top;
    if y < top {
        return rows.first().map_or(MAIN_TRACK, |(track, _)| track + 1);
    }
    for &(track, height) in &rows {
        top += height;
        if y < top {
            return track;
        }
    }
    rows.last().map_or(MAIN_TRACK, |(track, _)| track - 1)
}

/// A clip's colors by kind, as in the reference design: a bright header, a darker body, and
/// whether text on the header should be dark (for the light colors).
fn clip_colors(layer: &Layer) -> (u32, u32, bool) {
    match layer.content {
        LayerContent::Footage { .. } => (0x8b5cf6, 0x6d44d9, false),
        LayerContent::Audio { .. } => (0x6ee7a8, 0x1f5c45, true),
        LayerContent::Image { .. } | LayerContent::Solid { .. } => (0xe0b93a, 0x8f7420, true),
        LayerContent::Text { .. } => (0xf472b6, 0x9d3a6f, true),
        LayerContent::Shape { .. } => (0x22d3ee, 0x167a8a, true),
        LayerContent::Captions { .. } => (0xfacc15, 0x8a6d0a, true),
        LayerContent::Adjustment => (0xfb923c, 0x9a531c, true),
        LayerContent::Composition { .. } => (0x60a5fa, 0x2f5f9e, true),
    }
}

/// The icon on a clip: what kind of layer it is, or a mask's icon for a layer that masks
/// another (it is not drawn itself, only used to show or hide that layer).
fn kind_icon(layer: &Layer, is_mask: bool) -> IconName {
    if is_mask {
        return IconName::VenetianMask;
    }
    match &layer.content {
        LayerContent::Shape { path: Some(_), .. } => IconName::PenTool,
        LayerContent::Footage { .. } => IconName::SquarePlay,
        LayerContent::Audio { .. } => IconName::Music,
        LayerContent::Image { .. } => IconName::Image,
        LayerContent::Text { .. } => IconName::Type,
        LayerContent::Shape { .. } => IconName::Shapes,
        LayerContent::Captions { .. } => IconName::Captions,
        LayerContent::Adjustment => IconName::Contrast,
        LayerContent::Composition { .. } => IconName::Clapperboard,
        LayerContent::Solid { .. } => IconName::Square,
    }
}

/// The bar above the ruler: the composition's name and the graph editor on the left, the
/// transport and time in the middle, and snapping and zoom on the right. The two sides share
/// the leftover width equally, so the middle stays centered however long the name is.
pub fn toolbar(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let playing = state.is_playing();
    let rate = state.composition().frame_rate;
    let side = || div().flex_1().flex_basis(px(0.0)).min_w_0().flex().items_center().gap_1();
    let left = side()
        .child(
            div()
                .id("timeline-comp-name")
                .min_w_0()
                .truncate()
                .pl_1()
                .pr_2()
                .text_color(hsla(color::TEXT))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(state.composition().name.clone())
                .test_support(),
        )
        .child(div().w(px(1.0)).h(px(16.0)).mr_1().flex_none().bg(hsla(color::HAIRLINE)))
        .child(crate::panels::graph_editor::button(workspace, cx));
    let middle = div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .child(icon_button("tracks-start", IconName::SkipBack, "Go to Start (Home)", false, cx.listener(|this, _, _, cx| {
            let start = this.state.work_area.start as i64;
            this.go_to(start, cx);
        })))
        .child(icon_button("tracks-play", if playing { IconName::Pause } else { IconName::Play }, "Play / Pause (Space)", playing, cx.listener(|this, _, _, cx| this.toggle_playback(cx))))
        .child(icon_button("tracks-loop", IconName::Repeat, "Loop Playback", state.loop_playback, cx.listener(|this, _, _, cx| {
            this.state.loop_playback = !this.state.loop_playback;
            cx.notify();
        })))
        .child(div().pl_2().font_family("Menlo").text_size(px(13.0)).text_color(hsla(color::TEXT)).child(format_timecode(state.current_frame, rate)));
    let right = side()
        .justify_end()
        .child(icon_button("snapping", IconName::Magnet, "Snapping (S)", state.snapping, cx.listener(|this, _, _, cx| {
            this.state.snapping = !this.state.snapping;
            cx.notify();
        })))
        .child(icon_button("magnetic", IconName::Link, "Magnetic main track: its clips stay packed together", state.magnetic_main_track, cx.listener(|this, _, _, cx| {
            this.state.magnetic_main_track = !this.state.magnetic_main_track;
            cx.notify();
        })))
        .child(icon_button("tracks-zoom-out", IconName::ZoomOut, "Zoom Out (Alt + Scroll)", false, cx.listener(|this, _, _, cx| {
            zoom_around_playhead(this, 1.5);
            cx.notify();
        })))
        .child(icon_button("tracks-zoom-in", IconName::ZoomIn, "Zoom In (Alt + Scroll)", false, cx.listener(|this, _, _, cx| {
            zoom_around_playhead(this, 1.0 / 1.5);
            cx.notify();
        })))
        .child(icon_button("tracks-zoom-fit", IconName::Scan, "Zoom to Fit", false, cx.listener(|this, _, _, cx| {
            let duration = this.state.composition().duration_frames as f64;
            this.state.set_view_range(FrameRange::new(0.0, duration));
            cx.notify();
        })));
    div().size_full().flex().items_center().gap_2().px_2().child(left).child(middle).child(right)
}

/// The track rows with their clips.
pub fn rows(workspace: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    workspace.ui.timeline.media.free_retired(window, cx);
    let state = &workspace.state;
    let composition = state.composition();
    let existing = tracks::tracks(composition);
    let dragging = state.clip_drag_target();
    let hovered = workspace.ui.timeline.track_drop.filter(|_| cx.has_active_drag()).map(|(track, _)| track);

    let rows: Vec<AnyElement> = layout(workspace)
        .into_iter()
        .map(|(track, height)| {
            // A clip being dragged is drawn in the row it is headed for.
            let clips: Vec<AnyElement> = composition
                .layers
                .iter()
                .filter(|layer| match dragging {
                    Some((id, target)) if id == layer.id && existing.contains(&target) => target == track,
                    _ => layer.track == track,
                })
                .flat_map(|layer| clip(layer, track, workspace, cx))
                .collect();
            div()
                .relative()
                .h(px(height))
                .border_b_1()
                .border_color(hsla(color::BORDER))
                .bg(hsla(if hovered == Some(track) { color::HOVER } else if track == MAIN_TRACK { color::PANEL } else { color::INSET }))
                .children(clips)
                .into_any_element()
        })
        .collect();

    let view = state.view_range;
    let playhead = fraction(state.current_frame as f64, view);
    let snap = state.snap_line().map(|frame| div().id("snap-line").absolute().top_0().bottom_0().left(relative(fraction(frame as f64, view))).w(px(1.0)).bg(hsla(color::SNAP)).test_support());
    let drop_line = workspace.ui.timeline.track_drop.filter(|_| cx.has_active_drag()).map(|(_, frame)| {
        div().absolute().top_0().bottom_0().left(relative(fraction(frame as f64, view))).w(px(1.0)).bg(hsla(color::ACCENT))
    });
    // The work area's ends, as faint lines through the rows.
    let (work_start, work_end) = (fraction(state.work_area.start, view), fraction(state.work_area.end, view));
    let work_line = |at: f32| div().absolute().top_0().bottom_0().left(relative(at)).w(px(1.0)).bg(hsla(color::WORK_AREA));

    // While a clip is held, the closed hand stays wherever the pointer goes over the tracks.
    let holding = state.clip_drag_target().is_some() || state.slipping_layer().is_some();
    div()
        .id("track-rows")
        .relative()
        .when(holding, |rows| rows.cursor(gpui_kit::CursorStyle::ClosedHand))
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        // Pressing empty track space moves the playhead (and scrubs while dragging).
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            this.state.clear_selection();
            this.anchor_drag(event);
            this.apply_timeline_drag(TimelineDrag::Scrub, f32::from(event.position.x), f32::from(event.position.y));
            cx.notify();
        }))
        .on_drag(TimelineDrag::Scrub, |_, _, _, cx| cx.new(|_| EmptyDragView))
        // Project items dropped onto a track become clips there.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ProjectItemDrag>, _, cx| {
            let (x, y) = (f32::from(event.event.position.x), f32::from(event.event.position.y));
            let inside = Bounds::contains(&event.bounds, &event.event.position);
            this.ui.timeline.track_drop = inside.then(|| {
                let graph = this.ui.timeline.graph.get();
                (track_at(this, y), frame_at(x, graph, this.state.view_range).round().max(0.0) as i64)
            });
            cx.notify();
        }))
        .on_drop(cx.listener(|this, drag: &ProjectItemDrag, _, cx| {
            if let Some((track, frame)) = this.ui.timeline.track_drop.take() {
                this.state.add_clip_on_track(drag.item, track, frame);
            }
            cx.notify();
        }))
        // Files dragged in from the Finder: imported, then placed one after another on the track.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<gpui_kit::ExternalPaths>, _, cx| {
            let (x, y) = (f32::from(event.event.position.x), f32::from(event.event.position.y));
            let inside = Bounds::contains(&event.bounds, &event.event.position);
            this.ui.timeline.track_drop = inside.then(|| {
                let graph = this.ui.timeline.graph.get();
                (track_at(this, y), frame_at(x, graph, this.state.view_range).round().max(0.0) as i64)
            });
            cx.notify();
        }))
        .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
            let Some((track, mut frame)) = this.ui.timeline.track_drop.take() else { return };
            for item in this.import_paths(paths.paths().to_vec(), window, cx) {
                this.state.add_clip_on_track(item, track, frame);
                if let Some(added) = this.state.selected_layers.first().and_then(|id| this.state.composition().layer(*id)) {
                    frame = added.out_frame;
                }
            }
            cx.notify();
        }))
        .children(rows)
        .child(work_line(work_start))
        .child(work_line(work_end))
        .child(div().absolute().top_0().bottom_0().left(relative(playhead)).w(px(1.0)).bg(hsla(color::PLAYHEAD)))
        .children(snap)
        .children(drop_line)
}

/// A small icon toggle on a clip's header; the icon shows the state (crossed out when off).
fn clip_toggle(layer: Id, which: &'static str, on: bool, icons: (IconName, IconName), switch: Switch, dark: bool, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let ink = if dark { 0x111111 } else { 0xffffff };
    div()
        .id((which, layer.0 as usize))
        .size(px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        // A button on the clip, not part of it to grab: the ordinary arrow.
        .cursor(gpui_kit::CursorStyle::Arrow)
        .hover(|style| style.bg(rgba(0x0000002a)))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(cx.listener(move |this, _, _, cx| {
            this.state.toggle_switch(layer, switch);
            cx.notify();
        }))
        .child(Icon::new(if on { icons.0 } else { icons.1 }).size(px(12.0)).text_color(rgba((ink << 8) | if on { 0xdd } else { 0x88 })))
        .test_support()
}

/// One clip, and (when expanded) its keyframe rows underneath, as positioned elements of its
/// track row.
fn clip(layer: &Layer, track: i32, workspace: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let state = &workspace.state;
    let view = state.view_range;
    let id = layer.id;
    let left = fraction(layer.in_frame as f64, view);
    let width = fraction(layer.out_frame as f64, view) - left;
    let selected = state.selected_layers.contains(&id);
    let expanded = workspace.ui.timeline.expanded_clips.contains(&id);
    let (head, body, dark_ink) = clip_colors(layer);
    let is_mask = state.composition().layers.iter().any(|other| other.mask.is_some_and(|mask| mask.layer == id));
    let ink = if dark_ink { 0x111111 } else { 0xffffff };
    let graph_width = workspace.ui.timeline.graph.get().width.max(1.0);
    let clip_px = width * graph_width;
    let height = base_height(track) - 8.0;
    let cutting = workspace.ui.tool == Tool::Cut;
    let body_tool = matches!(workspace.ui.tool, Tool::Slip | Tool::Slide | Tool::TrackSelect);
    let held = state.clip_drag_target().is_some_and(|(layer, _)| layer == id) || state.slipping_layer() == Some(id);
    // The frame under the pointer, for the Cut tool.
    let pointer_frame = |this: &Workspace, x: gpui_kit::Pixels| frame_at(f32::from(x), this.ui.timeline.graph.get(), this.state.view_range).round() as i64;

    let mut inside: Vec<AnyElement> = Vec::new();
    let has_strip = matches!(layer.content, LayerContent::Footage { .. }) && layer.has_audio;
    let media_height = height - CLIP_HEADER - if has_strip { CLIP_SOUND_STRIP } else { 0.0 };
    match &layer.content {
        LayerContent::Footage { item } => {
            if let Some(source) = state.project.item(*item) {
                inside.push(filmstrip(layer, source, left, clip_px, media_height, false, workspace));
                if has_strip && let Some(path) = &source.path {
                    inside.push(waveform(layer, workspace.ui.timeline.media.peaks.peaks(path), CLIP_SOUND_STRIP, 0xffffff, workspace));
                }
            }
        }
        // A still image: its picture, repeated along the clip.
        LayerContent::Image { item } => {
            if let Some(source) = state.project.item(*item) {
                inside.push(filmstrip(layer, source, left, clip_px, media_height, true, workspace));
            }
        }
        LayerContent::Audio { item } => {
            let peaks = state.project.item(*item).and_then(|source| source.path.as_ref()).and_then(|path| workspace.ui.timeline.media.peaks.peaks(path));
            inside.push(waveform(layer, peaks, media_height, head, workspace));
        }
        LayerContent::Text { text, .. } => {
            inside.push(div().px_1p5().truncate().text_size(px(11.0)).text_color(rgba(0xffffffcc)).child(text.lines().next().unwrap_or_default().to_owned()).into_any_element());
        }
        _ => {}
    }

    let grip = |edit: LayerTimeEdit, cx: &mut Context<Workspace>| {
        let name = if edit == LayerTimeEdit::TrimIn { "clip-trim-in" } else { "clip-trim-out" };
        div()
            .id((name, id.0 as usize))
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(TRIM_GRIP))
            .cursor_ew_resize()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.state.select_layer(id, event.modifiers.shift || event.modifiers.platform);
                this.anchor_drag(event);
                // The Ripple and Roll tools change what dragging an end does.
                let end = if edit == LayerTimeEdit::TrimIn { End::In } else { End::Out };
                let edit = match this.ui.tool {
                    Tool::Ripple => LayerTimeEdit::Ripple(end),
                    Tool::Roll => LayerTimeEdit::Roll(end),
                    _ => edit,
                };
                this.state.begin_layer_time_drag(id, edit);
                cx.notify();
            }))
            .on_drag(TimelineDrag::LayerBar, |_, _, _, cx| cx.new(|_| EmptyDragView))
            .test_support()
    };

    // Where the Cut tool would cut this clip: a line across it under the pointer.
    let (in_frame, out_frame) = (layer.in_frame, layer.out_frame);
    let cut_line = workspace.ui.timeline.cut_hover.filter(|&(hovered, frame)| cutting && hovered == id && in_frame < frame && frame < out_frame).map(|(_, frame)| {
        let along = (frame - in_frame) as f32 / (out_frame - in_frame) as f32;
        div().absolute().top_0().bottom_0().left(relative(along)).w(px(1.5)).bg(hsla(0xffffff))
    });
    // Sound fades shade the clip's ends, darkest where the sound is quietest.
    let fps = workspace.state.composition().frame_rate.fps();
    let length = (layer.out_frame - layer.in_frame).max(1) as f64;
    let fade = |seconds: f32, from_left: bool| {
        (layer.has_audio && seconds > 0.0).then(|| {
            let share = (f64::from(seconds) * fps / length).clamp(0.0, 1.0) as f32;
            let (dark, clear) = (gpui_kit::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.55 }, gpui_kit::Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 });
            let shade = div().absolute().top_0().bottom_0().w(relative(share)).bg(gpui_kit::linear_gradient(
                if from_left { 90.0 } else { 270.0 },
                gpui_kit::linear_color_stop(dark, 0.0),
                gpui_kit::linear_color_stop(clear, 1.0),
            ));
            if from_left { shade.left_0() } else { shade.right_0() }
        })
    };
    let fades = [fade(layer.audio.fade_in, true), fade(layer.audio.fade_out, false)];
    let switches = layer.switches;
    let header = div()
        .h(px(CLIP_HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap_0p5()
        .px_1()
        .bg(rgba((head << 8) | 0xff))
        .text_size(px(11.0))
        .text_color(rgba((ink << 8) | 0xff))
        // The arrow opens the clip's keyframe rows.
        .child(
            div()
                .id(("clip-expand", id.0 as usize))
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor(gpui_kit::CursorStyle::Arrow)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let expanded = &mut this.ui.timeline.expanded_clips;
                    if !expanded.remove(&id) {
                        expanded.insert(id);
                    }
                    cx.notify();
                }))
                .child(Icon::new(if expanded { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)).text_color(rgba((ink << 8) | 0xdd)))
                .test_support(),
        )
        .child(Icon::new(kind_icon(layer, is_mask)).size(px(12.0)).text_color(rgba((ink << 8) | 0xdd)))
        .child(div().flex_1().min_w_0().truncate().child(layer.name.clone()))
        .when(!layer.effects.is_empty(), |header| header.child(div().px_1().rounded(px(3.0)).bg(rgba(0x00000026)).child(format!("fx {}", layer.effects.len()))))
        .when(layer.has_audio, |header| header.child(clip_toggle(id, "clip-mute", switches.audio, (IconName::Volume2, IconName::VolumeX), Switch::Audio, dark_ink, cx)))
        .when(layer.content.has_picture(), |header| header.child(clip_toggle(id, "clip-show", switches.visible, (IconName::Eye, IconName::EyeOff), Switch::Visible, dark_ink, cx)))
        .child(clip_toggle(id, "clip-lock", !switches.locked, (IconName::LockOpen, IconName::Lock), Switch::Locked, dark_ink, cx));

    let mut elements = vec![
        div()
            .id(("clip", id.0 as usize))
            .absolute()
            .top(px(4.0))
            .h(px(height))
            .left(relative(left))
            .w(relative(width))
            .overflow_hidden()
            .rounded(px(5.0))
            .bg(rgba((body << 8) | 0xff))
            .border_1()
            .border_color(if selected { hsla(0xffffff) } else { rgba((head << 8) | 0x55).into() })
            .when(!switches.visible, |clip| clip.opacity(0.45))
            .flex()
            .flex_col()
            // An open hand to say it can be picked up, closed while it is held.
            .map(|clip| if held { clip.cursor(gpui_kit::CursorStyle::ClosedHand) } else { clip.cursor_grab() })
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                // Double-clicking a nested composition's clip opens it.
                if event.click_count >= 2
                    && let Some(inner) = this.state.nested_composition(id)
                {
                    this.state.open_composition(inner);
                    cx.notify();
                    return;
                }
                // The Cut tool splits the clip where it is clicked, and does nothing else.
                if this.ui.tool == Tool::Cut {
                    let frame = pointer_frame(this, event.position.x);
                    this.state.cut_layer_at(id, frame);
                    this.ui.timeline.cut_hover = None;
                    cx.notify();
                    return;
                }
                // Track Select Forward picks this clip and everything after it (Shift: on this
                // track only), and dragging moves them all.
                if this.ui.tool == Tool::TrackSelect {
                    this.anchor_drag(event);
                    this.state.begin_track_select_forward(id, event.modifiers.shift);
                    cx.notify();
                    return;
                }
                this.state.select_layer(id, event.modifiers.shift || event.modifiers.platform);
                this.anchor_drag(event);
                match this.ui.tool {
                    // The Slip and Slide tools drag the clip's body; Ripple and Roll only its ends.
                    Tool::Slip => {
                        if this.state.can_slip(id) {
                            this.state.begin_layer_time_drag(id, LayerTimeEdit::Slip);
                        }
                    }
                    Tool::Slide => this.state.begin_layer_time_drag(id, LayerTimeEdit::Slide),
                    Tool::Ripple | Tool::Roll => {}
                    // Alt-drag slips the source under the clip, as elsewhere in the timeline.
                    _ if event.modifiers.alt && this.state.can_slip(id) => {
                        this.state.begin_layer_time_drag(id, LayerTimeEdit::Slip);
                    }
                    _ => this.state.begin_clip_drag(id),
                }
                cx.notify();
            }))
            .on_drag(TimelineDrag::Clip, |_, _, _, cx| cx.new(|_| EmptyDragView))
            .when(cutting, |clip| {
                clip.cursor(gpui_kit::CursorStyle::Crosshair)
                    .on_mouse_move(cx.listener(move |this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                        let hover = Some((id, pointer_frame(this, event.position.x)));
                        if this.ui.timeline.cut_hover != hover {
                            this.ui.timeline.cut_hover = hover;
                            cx.notify();
                        }
                    }))
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        if !hovered && this.ui.timeline.cut_hover.is_some_and(|(layer, _)| layer == id) {
                            this.ui.timeline.cut_hover = None;
                            cx.notify();
                        }
                    }))
            })
            // Effects dragged from the Effects panel are added to the clip they're dropped on.
            .drag_over::<EffectDrag>(|style, _, _, _| style.border_color(hsla(0xffffff)))
            .on_drop(cx.listener(move |this, drag: &EffectDrag, _, cx| {
                this.state.add_effect(id, &drag.name);
                cx.notify();
            }))
            .child(header)
            .children(inside)
            .children(fades.into_iter().flatten())
            .children(cut_line)
            // Trim grips would catch clicks near the ends, so the Cut tool goes without them.
            // So do the body-dragging tools (Slip, Slide, Track Select).
            .when(!cutting && !body_tool, |clip| clip.child(grip(LayerTimeEdit::TrimIn, cx).left_0()).child(grip(LayerTimeEdit::TrimOut, cx).right_0()))
            .test_support()
            .into_any_element(),
    ];

    if expanded {
        elements.push(property_rows(layer, left, width, base_height(track), head, workspace, cx).into_any_element());
    }
    elements
}

/// Opens the graph editor next to `at` (window coordinates), for the keyframes just selected.
fn open_graph_editor(workspace: &mut Workspace, at: gpui_kit::Point<gpui_kit::Pixels>) {
    workspace.ui.graph_editor = Some(point(at.x - px(120.0), at.y + px(14.0)));
}

/// The rows under an expanded clip: one per animated property, with its name above a line
/// across its keyframes. Pressing a row selects all its keyframes; keyframes select (Shift
/// adds) and drag in time; clicking the line between two keyframes selects that segment and
/// opens the graph editor to ease it, as does double-clicking a keyframe.
fn property_rows(layer: &Layer, left: f32, width: f32, top: f32, head: u32, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let state = &workspace.state;
    let view = state.view_range;
    let id = layer.id;
    let properties = animated_properties(layer);
    // Keyframe positions are fractions of the visible range; inside this box they are measured
    // from its left edge, so convert.
    let local = move |frame: i64| (fraction(frame as f64, view) - left) / width.max(1e-6);
    let tint = |alpha: u32| -> gpui_kit::Hsla { rgba((head << 8) | alpha).into() };
    let rows: Vec<AnyElement> = if properties.is_empty() {
        vec![div().h(px(PROPERTY_ROW)).flex().items_center().px_2().text_size(px(11.0)).text_color(hsla(color::MUTED)).child("No keyframes yet: animate a property in Properties").into_any_element()]
    } else {
        properties
            .into_iter()
            .map(|(path, name, frames)| {
                let all_selected = frames.iter().all(|&frame| state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame }));
                // The line between each pair of keyframes: click it to ease that segment.
                let segments = frames.windows(2).map(|pair| {
                    let (from, to) = (pair[0], pair[1]);
                    let selected = state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame: from }) && state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame: to });
                    div()
                        .id(gpui_kit::ElementId::Name(format!("clip-seg:{}:{path:?}:{from}", id.0).into()))
                        .absolute()
                        .top(px(KEY_LINE - 6.0))
                        .h(px(12.0))
                        .left(relative(local(from)))
                        .w(relative((local(to) - local(from)).max(0.0)))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .child(div().w_full().h(px(if selected { 2.0 } else { 1.0 })).bg(if selected { hsla(0xffffff) } else { tint(0x99) }))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.state.set_keyframe_selection(vec![KeyframeRef { layer: id, path, frame: from }, KeyframeRef { layer: id, path, frame: to }]);
                            open_graph_editor(this, event.position);
                            cx.notify();
                        }))
                        .test_support()
                });
                let keys = frames.iter().map(|&frame| {
                    let key = KeyframeRef { layer: id, path, frame };
                    let selected = state.is_keyframe_selected(&key);
                    div()
                        .id(gpui_kit::ElementId::Name(format!("clip-kf:{}:{path:?}:{frame}", id.0).into()))
                        .absolute()
                        .top(px(KEY_LINE - KEY_TARGET * 0.5))
                        .left(relative(local(frame)))
                        .ml(px(-KEY_TARGET * 0.5))
                        .size(px(KEY_TARGET))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.0))
                        .hover(|style| style.bg(rgba(0xffffff1a)))
                        .cursor_ew_resize()
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let additive = event.modifiers.shift || event.modifiers.platform;
                            if !(this.state.is_keyframe_selected(&key) && !additive) {
                                this.state.select_keyframe(key, additive);
                            }
                            // A double-click opens the graph editor to ease it.
                            if event.click_count >= 2 {
                                open_graph_editor(this, event.position);
                            } else {
                                this.anchor_drag(event);
                                this.state.begin_keyframe_drag();
                            }
                            cx.notify();
                        }))
                        .on_drag(TimelineDrag::Keyframes, |_, _, _, cx| cx.new(|_| EmptyDragView))
                        .child(Icon::new(IconName::Diamond).size(px(if selected { 13.0 } else { 11.0 })).text_color(if selected { hsla(0xffffff) } else { tint(0xff) }))
                        .test_support()
                });
                div()
                    .id(gpui_kit::ElementId::Name(format!("clip-prop:{}:{path:?}", id.0).into()))
                    .relative()
                    .h(px(PROPERTY_ROW))
                    .when(all_selected, |row| row.bg(rgba(0xffffff0d)))
                    // Pressing the row (not a keyframe) selects all of its keyframes.
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.state.select_property_keyframes(id, path, event.modifiers.shift || event.modifiers.platform);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id(gpui_kit::ElementId::Name(format!("clip-prop-name:{}:{path:?}", id.0).into()))
                            .absolute()
                            .left(px(8.0))
                            .top(px(2.0))
                            .h(px(13.0))
                            .text_size(px(10.0))
                            .line_height(px(13.0))
                            .text_color(tint(0xee))
                            .child(name)
                            .test_support(),
                    )
                    .children(segments.collect::<Vec<_>>())
                    .children(keys.collect::<Vec<_>>())
                    .test_support()
                    .into_any_element()
            })
            .collect()
    };
    div()
        .absolute()
        .top(px(top))
        .left(relative(left))
        .w(relative(width))
        .rounded_b(px(5.0))
        .bg(rgba((head << 8) | 0x1c))
        .children(rows)
}

/// Thumbnails across a clip, one per tile: for video, the source frame at each tile's spot; for
/// a still image (`still`), its one picture. Only tiles that are on screen are asked for.
#[allow(clippy::too_many_arguments)]
fn filmstrip(layer: &Layer, source: &studio_core::model::ProjectItem, left: f32, clip_px: f32, height: f32, still: bool, workspace: &Workspace) -> AnyElement {
    let Some(path) = source.path.as_ref() else { return div().into_any_element() };
    let aspect = source.size.map_or(16.0 / 9.0, |size| size.x / size.y.max(1.0));
    let tile = (height * aspect).max(16.0);
    let graph_width = workspace.ui.timeline.graph.get().width.max(1.0);
    let clip_left_px = left * graph_width;
    let frames = (layer.out_frame - layer.in_frame).max(1) as f32;
    let comp_rate = workspace.state.composition().frame_rate;
    let media = &workspace.ui.timeline.media;
    let count = (clip_px / tile).ceil().clamp(1.0, 400.0) as usize;
    let tiles = (0..count).filter_map(|index| {
        let offset = index as f32 * tile;
        let on_screen = clip_left_px + offset + tile >= 0.0 && clip_left_px + offset <= graph_width;
        if !on_screen {
            return None;
        }
        // The source moment at the tile's middle.
        let frame = layer.in_frame as f64 + f64::from((offset + tile * 0.5) / clip_px.max(1.0) * frames);
        // Following the clip's speed, so a slowed or reversed clip shows what plays there.
        let seconds = if still { 0.0 } else { layer.source_seconds(frame.round() as i64, comp_rate) };
        let picture = media.thumbnails.thumbnail(path, seconds.max(0.0))?;
        let image = media.image(&picture);
        Some(img(ImageSource::Render(image)).absolute().left(px(offset)).top_0().w(px(tile)).h(px(height)).into_any_element())
    });
    div().relative().h(px(height)).w_full().overflow_hidden().children(tiles.collect::<Vec<_>>()).into_any_element()
}

/// A waveform across a clip: one bar per two pixels, from the source's peak levels at that time.
fn waveform(layer: &Layer, peaks: Option<Arc<Peaks>>, height: f32, ink: u32, workspace: &Workspace) -> AnyElement {
    let Some(peaks) = peaks else { return div().h(px(height)).into_any_element() };
    let fps = workspace.state.composition().frame_rate.fps();
    let (in_frame, out_frame, start_frame) = (layer.in_frame, layer.out_frame, layer.start_frame);
    // At another steady speed the sound covers more (or less) of the file per frame.
    let speed = layer.time.constant_factor().unwrap_or(1.0);
    let color = rgba((ink << 8) | 0xcc);
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let width = f32::from(bounds.size.width);
        let tall = f32::from(bounds.size.height);
        let middle = f32::from(bounds.origin.y) + tall * 0.5;
        let span = (out_frame - in_frame).max(1) as f32;
        let mut x = 0.0;
        while x < width {
            let frame = in_frame as f32 + x / width.max(1.0) * span;
            let seconds = (f64::from(frame) - start_frame as f64) / fps * speed;
            let bucket = (seconds * f64::from(peaks.per_second)).floor();
            let level = (bucket >= 0.0).then(|| peaks.values.get(bucket as usize).copied()).flatten().unwrap_or(0.0);
            // Quiet sounds still show; full scale fills the strip.
            let bar = (level.sqrt() * tall).max(1.0);
            let origin = point(bounds.origin.x + px(x), px(middle - bar * 0.5));
            window.paint_quad(fill(Bounds::new(origin, gpui_size(px(1.5), px(bar))), color));
            x += 2.0;
        }
    })
    .h(px(height))
    .w_full()
    .into_any_element()
}

