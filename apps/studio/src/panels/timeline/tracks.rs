//! The timeline's Tracks view, in the style of CapCut and Premiere: one row per track, each
//! holding any number of clips side by side (see `studio_core::tracks`). Video clips show a
//! filmstrip of thumbnails, sound shows its waveform, and other layers their name and color.
//!
//! Clips drag in time and between tracks (a clip that would overlap another gets a new track),
//! their ends trim, Alt-drag slips the source, and project items drop straight onto a track.
//! The ruler, navigator, playhead, snapping and zoom are shared with the Layers view.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gpui_kit::{
    AnyElement, Bounds, ImageSource, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, RenderImage,
    StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, fill, img, point, prelude::FluentBuilder as _, px,
    relative, rgba, size as gpui_size,
};
use studio_core::media::VideoFrame;
use studio_core::model::{Layer, LayerContent};
use studio_core::tracks::{self, MAIN_TRACK};
use studio_media::{PeakCache, Peaks, Thumbnailer};

use super::{TimelineDrag, fraction, frame_at};
use crate::panels::chrome::{EmptyDragView, icon_button};
use crate::state::{LayerTimeEdit, Switch};
use crate::theme::{color, hsla};
use crate::workspace::{ProjectItemDrag, Workspace};
use gpui_kit::assets::IconName;
use gpui_kit::{AppContext as _, Context, DragMoveEvent};

/// Width of the track headers on the left.
pub const HEADER_WIDTH: f32 = 150.0;
/// Row heights: the main track is tallest, as in CapCut; sound tracks are shorter.
const MAIN_HEIGHT: f32 = 56.0;
const PICTURE_HEIGHT: f32 = 40.0;
const SOUND_HEIGHT: f32 = 32.0;
/// Height of filmstrip thumbnails, in pixels.
pub const THUMBNAIL_HEIGHT: u32 = 36;
/// Height of a clip's name strip.
const CLIP_HEADER: f32 = 14.0;
/// Height of the waveform under a video clip that has sound.
const CLIP_SOUND_STRIP: f32 = 12.0;
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

fn track_height(track: i32) -> f32 {
    match track {
        MAIN_TRACK => MAIN_HEIGHT,
        track if track < MAIN_TRACK => SOUND_HEIGHT,
        _ => PICTURE_HEIGHT,
    }
}

/// The track under window y position `y`, given where the rows start. Above the top row is a
/// new track above it; below the bottom row, a new track below it.
pub fn track_at(composition: &studio_core::model::Composition, rows_top: f32, y: f32) -> i32 {
    let rows = tracks::tracks(composition);
    let mut top = rows_top;
    if y < top {
        return rows.first().map_or(MAIN_TRACK, |track| track + 1);
    }
    for &track in &rows {
        top += track_height(track);
        if y < top {
            return track;
        }
    }
    rows.last().map_or(MAIN_TRACK, |track| track - 1)
}

fn track_name(track: i32) -> String {
    match track {
        MAIN_TRACK => "Main".to_owned(),
        track if track > MAIN_TRACK => format!("Video {track}"),
        track => format!("Audio {}", -track),
    }
}

/// The colors a clip is drawn in: body and accent, by what it holds.
fn clip_colors(layer: &Layer) -> (u32, u32) {
    match layer.content {
        LayerContent::Footage { .. } => (0x173d44, 0x2bb3c0),
        LayerContent::Audio { .. } => (0x16304d, 0x4c8fe0),
        LayerContent::Image { .. } => (0x2f2b4a, 0x8a7cf0),
        LayerContent::Text { .. } => (0x4a2438, 0xe0609a),
        LayerContent::Solid { .. } => (0x2c2c32, 0x9a9aa6),
        LayerContent::Adjustment => (0x3d3a1f, 0xd8c24a),
        LayerContent::Composition { .. } => (0x1f3d2b, 0x4cc07a),
    }
}

/// The track headers on the left: name, and toggles that hide, mute or lock the whole track.
pub fn headers(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let rows = tracks::tracks(state.composition()).into_iter().map(|track| {
        let has_sound = state.composition().layers.iter().any(|layer| layer.track == track && layer.has_audio);
        let toggle = |id: &'static str, icon: IconName, tip: &'static str, switch: Switch, on: bool, cx: &mut Context<Workspace>| {
            icon_button((id, track as i64 as usize), icon, tip, on, cx.listener(move |this, _, _, cx| {
                this.state.toggle_track_switch(track, switch);
                cx.notify();
            }))
        };
        let picture = track >= MAIN_TRACK;
        div()
            .h(px(track_height(track)))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(hsla(color::BORDER))
            .child(div().flex_1().truncate().text_color(hsla(if track == MAIN_TRACK { color::TEXT } else { color::MUTED })).child(track_name(track)))
            .when(track == MAIN_TRACK, |row| {
                row.child(icon_button("magnetic", IconName::Link, "Magnetic main track: clips stay packed together", state.magnetic_main_track, cx.listener(|this, _, _, cx| {
                    this.state.magnetic_main_track = !this.state.magnetic_main_track;
                    cx.notify();
                })))
            })
            .when(picture, |row| row.child(toggle("track-hide", IconName::Eye, "Show or hide this track", Switch::Visible, state.track_switch(track, Switch::Visible), cx)))
            .when(has_sound, |row| row.child(toggle("track-mute", IconName::Volume2, "Mute or unmute this track", Switch::Audio, state.track_switch(track, Switch::Audio), cx)))
            .child(toggle("track-lock", IconName::Lock, "Lock this track", Switch::Locked, state.track_switch(track, Switch::Locked), cx))
    });
    div().flex().flex_col().bg(hsla(color::PANEL)).children(rows.collect::<Vec<_>>())
}

/// The track rows with their clips.
pub fn rows(workspace: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    workspace.ui.timeline.media.free_retired(window, cx);
    let state = &workspace.state;
    let composition = state.composition();
    let dragging = state.clip_drag_target();
    let hovered = workspace.ui.timeline.track_drop.filter(|_| cx.has_active_drag()).map(|(track, _)| track);

    let rows: Vec<AnyElement> = tracks::tracks(composition)
        .into_iter()
        .map(|track| {
            // A clip being dragged is drawn in the row it is headed for.
            let clips: Vec<AnyElement> = composition
                .layers
                .iter()
                .filter(|layer| match dragging {
                    Some((id, target)) if id == layer.id && tracks::tracks(composition).contains(&target) => target == track,
                    _ => layer.track == track,
                })
                .map(|layer| clip(layer, track_height(track), workspace, cx))
                .collect();
            div()
                .relative()
                .h(px(track_height(track)))
                .border_b_1()
                .border_color(hsla(color::BORDER))
                .bg(hsla(if hovered == Some(track) { color::HOVER } else if track == MAIN_TRACK { color::INSET } else { color::PANEL }))
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

    div()
        .id("track-rows")
        .relative()
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
                let track = track_at(this.state.composition(), this.ui.timeline.rows.get().top, y);
                (track, frame_at(x, graph, this.state.view_range).round().max(0.0) as i64)
            });
            cx.notify();
        }))
        .on_drop(cx.listener(|this, drag: &ProjectItemDrag, _, cx| {
            if let Some((track, frame)) = this.ui.timeline.track_drop.take() {
                this.state.add_clip_on_track(drag.item, track, frame);
            }
            cx.notify();
        }))
        .children(rows)
        .child(div().absolute().top_0().bottom_0().left(relative(playhead)).w(px(1.0)).bg(hsla(color::TEXT)))
        .children(snap)
        .children(drop_line)
}

/// One clip: a rounded box spanning its time, with what it holds drawn inside.
fn clip(layer: &Layer, row_height: f32, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let state = &workspace.state;
    let view = state.view_range;
    let id = layer.id;
    let left = fraction(layer.in_frame as f64, view);
    let width = fraction(layer.out_frame as f64, view) - left;
    let selected = state.selected_layers.contains(&id);
    let (body, accent) = clip_colors(layer);
    let graph_width = workspace.ui.timeline.graph.get().width.max(1.0);
    let clip_px = width * graph_width;

    let mut inside: Vec<AnyElement> = Vec::new();
    let has_strip = matches!(layer.content, LayerContent::Footage { .. }) && layer.has_audio;
    let media_height = row_height - 8.0 - CLIP_HEADER - if has_strip { CLIP_SOUND_STRIP } else { 0.0 };
    match &layer.content {
        LayerContent::Footage { item } => {
            if let Some(source) = state.project.item(*item) {
                inside.push(filmstrip(layer, source, left, clip_px, media_height, workspace));
                if has_strip && let Some(path) = &source.path {
                    inside.push(waveform(layer, workspace.ui.timeline.media.peaks.peaks(path), CLIP_SOUND_STRIP, accent, workspace));
                }
            }
        }
        LayerContent::Audio { item } => {
            let peaks = state.project.item(*item).and_then(|source| source.path.as_ref()).and_then(|path| workspace.ui.timeline.media.peaks.peaks(path));
            inside.push(waveform(layer, peaks, media_height, accent, workspace));
        }
        LayerContent::Text { text, .. } => {
            inside.push(div().px_1().truncate().text_color(hsla(color::TEXT)).child(text.lines().next().unwrap_or_default().to_owned()).into_any_element());
        }
        LayerContent::Solid { color: fill_color } => {
            inside.push(div().mx_1().h(px(media_height.max(4.0))).rounded(px(2.0)).bg(rgba(fill_color.to_rgba_u32())).into_any_element());
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
                this.state.begin_layer_time_drag(id, edit);
                cx.notify();
            }))
            .on_drag(TimelineDrag::LayerBar, |_, _, _, cx| cx.new(|_| EmptyDragView))
            .test_support()
    };

    let effects = layer.effects.len();
    let header = div()
        .h(px(CLIP_HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .text_size(px(10.0))
        .text_color(hsla(color::TEXT))
        .child(div().truncate().child(layer.name.clone()))
        .when(effects > 0, |header| header.child(div().px_1().rounded(px(2.0)).bg(rgba(0xffffff1f)).child(format!("fx {effects}"))));

    div()
        .id(("clip", id.0 as usize))
        .absolute()
        .top(px(4.0))
        .bottom(px(4.0))
        .left(relative(left))
        .w(relative(width))
        .overflow_hidden()
        .rounded(px(4.0))
        .bg(rgba((body << 8) | 0xff))
        .border_1()
        .border_color(if selected { hsla(0xffffff) } else { rgba((accent << 8) | 0x66).into() })
        .when(!layer.switches.visible, |clip| clip.opacity(0.4))
        .flex()
        .flex_col()
        .cursor_grab()
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            this.state.select_layer(id, event.modifiers.shift || event.modifiers.platform);
            this.anchor_drag(event);
            // Alt-drag slips the source under the clip, as elsewhere in the timeline.
            if event.modifiers.alt && this.state.can_slip(id) {
                this.state.begin_layer_time_drag(id, LayerTimeEdit::Slip);
            } else {
                this.state.begin_clip_drag(id);
            }
            cx.notify();
        }))
        .on_drag(TimelineDrag::Clip, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .child(header)
        .children(inside)
        .child(grip(LayerTimeEdit::TrimIn, cx).left_0())
        .child(grip(LayerTimeEdit::TrimOut, cx).right_0())
        .test_support()
        .into_any_element()
}

/// Thumbnails across a video clip, one per tile, each showing the source frame at its spot.
/// Only tiles that are on screen are asked for.
fn filmstrip(layer: &Layer, source: &studio_core::model::ProjectItem, left: f32, clip_px: f32, height: f32, workspace: &Workspace) -> AnyElement {
    let Some(path) = source.path.as_ref() else { return div().into_any_element() };
    let aspect = source.size.map_or(16.0 / 9.0, |size| size.x / size.y.max(1.0));
    let tile = (height * aspect).max(16.0);
    let graph_width = workspace.ui.timeline.graph.get().width.max(1.0);
    let clip_left_px = left * graph_width;
    let frames = (layer.out_frame - layer.in_frame).max(1) as f32;
    let fps = workspace.state.composition().frame_rate.fps();
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
        let seconds = (frame - layer.start_frame as f64) / fps;
        let picture = media.thumbnails.thumbnail(path, seconds.max(0.0))?;
        let image = media.image(&picture);
        Some(img(ImageSource::Render(image)).absolute().left(px(offset)).top_0().w(px(tile)).h(px(height)).into_any_element())
    });
    div().relative().h(px(height)).w_full().overflow_hidden().children(tiles.collect::<Vec<_>>()).into_any_element()
}

/// A waveform across a clip: one bar per two pixels, from the source's peak levels at that time.
fn waveform(layer: &Layer, peaks: Option<Arc<Peaks>>, height: f32, accent: u32, workspace: &Workspace) -> AnyElement {
    let Some(peaks) = peaks else { return div().h(px(height)).into_any_element() };
    let fps = workspace.state.composition().frame_rate.fps();
    let (in_frame, out_frame, start_frame) = (layer.in_frame, layer.out_frame, layer.start_frame);
    let color = rgba((accent << 8) | 0xcc);
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let width = f32::from(bounds.size.width);
        let tall = f32::from(bounds.size.height);
        let middle = f32::from(bounds.origin.y) + tall * 0.5;
        let span = (out_frame - in_frame).max(1) as f32;
        let mut x = 0.0;
        while x < width {
            let frame = in_frame as f32 + x / width.max(1.0) * span;
            let seconds = (f64::from(frame) - start_frame as f64) / fps;
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
