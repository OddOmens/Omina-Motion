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
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, Div, DragMoveEvent, ElementId, ImageSource, InteractiveElement, IntoElement, MouseButton,
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
use crate::workspace::{EditTarget, EffectDrag, ProjectItemDrag, Tool, Workspace};

/// Width of the track headers left of the rows: the track's number and its eye and mute buttons.
pub const HEADER_WIDTH: f32 = 84.0;

/// Row heights: the main track is tallest, as in CapCut; sound tracks are shorter.
const MAIN_HEIGHT: f32 = 58.0;
const PICTURE_HEIGHT: f32 = 48.0;
const SOUND_HEIGHT: f32 = 40.0;
/// Every row's height with compact tracks (Settings > Timeline & Viewer): room for a clip's
/// name strip only.
const COMPACT_HEIGHT: f32 = 28.0;
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
    pub(crate) fn image(&self, frame: &Arc<VideoFrame>) -> Arc<RenderImage> {
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

fn base_height(track: i32, compact: bool) -> f32 {
    match track {
        _ if compact => COMPACT_HEIGHT,
        MAIN_TRACK => MAIN_HEIGHT,
        track if track < MAIN_TRACK => SOUND_HEIGHT,
        _ => PICTURE_HEIGHT,
    }
}

/// The properties shown under an expanded clip: the animated ones, in panel order.
pub(super) fn animated_properties(layer: &Layer) -> Vec<(PropertyPath, String, Vec<i64>)> {
    layer
        .property_paths()
        .into_iter()
        .filter_map(|path| layer.property(path, 0).filter(|property| !property.keyframes.is_empty()).map(|property| (path, property.name, property.keyframes)))
        .collect()
}

/// The keyframe rows an expanded clip shows: every animated property, or just the one picked
/// with the P, R or S key (no rows when that one isn't animated).
fn shown_properties(workspace: &Workspace, layer: &Layer) -> Vec<(PropertyPath, String, Vec<i64>)> {
    let mut properties = animated_properties(layer);
    if let Some(only) = workspace.ui.timeline.only_property.get(&layer.id) {
        properties.retain(|(path, ..)| path == only);
    }
    properties
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
                .map(|layer| shown_properties(workspace, layer).len().max(1))
                .max()
                .unwrap_or(0);
            (track, base_height(track, workspace.ui.settings.compact_timeline) + rows as f32 * PROPERTY_ROW)
        })
        .collect()
}

/// Height of the top track's row.
pub(super) fn first_row_height(workspace: &Workspace) -> f32 {
    layout(workspace).first().map_or(base_height(0, workspace.ui.settings.compact_timeline), |(_, height)| *height)
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
        .when(workspace.has(crate::features::Feature::GraphEditor), |bar| bar.child(crate::panels::graph_editor::button(workspace, cx)));
    let middle = div()
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        // The editor's one set of playback controls (the viewer's bar keeps to view settings).
        .child(icon_button("tracks-start", IconName::SkipBack, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Go to Start", "first-frame"), false, cx.listener(|this, _, _, cx| this.go_to(0, cx))))
        .child(icon_button("previous-frame", IconName::StepBack, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Previous Frame", "previous-frame"), false, cx.listener(|this, _, _, cx| {
            let frame = this.state.current_frame - 1;
            this.go_to(frame, cx);
        })))
        .child(icon_button("tracks-play", if playing { IconName::Pause } else { IconName::Play }, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Play / Pause", "toggle-playback"), playing, cx.listener(|this, _, _, cx| this.toggle_playback(cx))))
        .child(icon_button("next-frame", IconName::StepForward, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Next Frame", "next-frame"), false, cx.listener(|this, _, _, cx| {
            let frame = this.state.current_frame + 1;
            this.go_to(frame, cx);
        })))
        .child(icon_button("last-frame", IconName::SkipForward, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Go to End", "last-frame"), false, cx.listener(|this, _, _, cx| {
            let end = this.state.composition().duration_frames - 1;
            this.go_to(end, cx);
        })))
        .child(div().w(px(6.0)))
        .child(icon_button("tracks-loop", IconName::Repeat, "Loop Playback", state.loop_playback, cx.listener(|this, _, _, cx| {
            this.state.loop_playback = !this.state.loop_playback;
            cx.notify();
        })))
        .when(workspace.has(crate::features::Feature::RenderAndPlay), |bar| bar.child(icon_button("tracks-render-play", IconName::Film, crate::keymap::with_key(&workspace.ui.settings.shortcuts, "Render & Play: render the work area, then play it smoothly", "render-and-play"), workspace.ui.render_then_play, cx.listener(|this, _, window, cx| this.render_and_play(window, cx)))))
        .child(div().pl_2().font_family(crate::theme::MONO_FONT).text_size(px(crate::theme::size::LARGE_TEXT)).text_color(hsla(color::ACCENT)).child(format_timecode(state.current_frame, rate)))
        .children(render_progress(workspace, cx));
    let right = side()
        .justify_end()
        .child(icon_button("tracks-compact", IconName::Rows3, "Compact Tracks", workspace.ui.settings.compact_timeline, cx.listener(|this, _, _, cx| {
            let compact = !this.ui.settings.compact_timeline;
            this.ui.settings.compact_timeline = compact;
            crate::app_settings::update(|settings| settings.compact_timeline = compact);
            cx.notify();
        })))
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

/// While Render & Play renders: how far it has got, and Cancel.
fn render_progress(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let (done, wanted) = workspace.render_then_play_progress()?;
    let percent = (done * 100).checked_div(wanted.max(1)).unwrap_or(0).min(100);
    Some(
        div()
            .flex()
            .items_center()
            .gap_1()
            .pl_2()
            .text_size(px(crate::theme::size::TEXT))
            .text_color(hsla(color::MUTED))
            .child(div().id("render-play-progress").child(format!("Rendering {percent}%")).test_support())
            .child(icon_button("render-play-cancel", IconName::X, "Cancel", false, cx.listener(|this, _, _, cx| {
                this.ui.render_then_play = false;
                cx.notify();
            }))),
    )
}

/// The track rows with their clips.
pub fn rows(workspace: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    workspace.ui.timeline.media.free_retired(window, cx);
    let state = &workspace.state;
    let composition = state.composition();
    let existing = tracks::tracks(composition);
    let dragging = state.clip_drag_target();
    let hovered = workspace.ui.timeline.track_drop.filter(|_| cx.has_active_drag()).map(|(track, _)| track);

    // Only rows near what can be seen are drawn (a screenful either side, so scrolling never
    // shows a gap); the rest are left empty at their full height, so nothing moves. Clips wholly
    // outside the time shown aren't drawn either. Big projects stay quick this way.
    let viewport = workspace.ui.timeline.viewport.get();
    let measured = viewport.height > 0.0;
    let (near_top, near_bottom) = (viewport.top - viewport.height, viewport.top + viewport.height * 2.0);
    let mut row_top = workspace.ui.timeline.rows.get().top;
    let view = state.view_range;
    let rows: Vec<AnyElement> = layout(workspace)
        .into_iter()
        .map(|(track, height)| {
            let top = row_top;
            row_top += height;
            let near = !measured || (top + height >= near_top && top <= near_bottom);
            // A clip being dragged is drawn in the row it is headed for.
            let clips: Vec<AnyElement> = composition
                .layers
                .iter()
                .filter(|_| near)
                .filter(|layer| match dragging {
                    Some((id, target)) if id == layer.id && existing.contains(&target) => target == track,
                    _ => layer.track == track,
                })
                .filter(|layer| layer.out_frame as f64 >= view.start && layer.in_frame as f64 <= view.end)
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
            let place: crate::panels::importing::Then = Box::new(move |this, items, _, cx| {
                for item in items {
                    this.state.add_clip_on_track(item, track, frame);
                    if let Some(added) = this.state.selected_layers.first().and_then(|id| this.state.composition().layer(*id)) {
                        frame = added.out_frame;
                    }
                }
                cx.notify();
            });
            this.import_paths(paths.paths().to_vec(), Some(place), window, cx);
            cx.notify();
        }))
        .children(rows)
        // Under the lines below, so the playhead is never hidden behind the hint.
        .children(super::empty_hint(workspace))
        .child(work_line(work_start))
        .child(work_line(work_end))
        .child(div().absolute().top_0().bottom_0().left(relative(playhead)).w(px(1.0)).bg(hsla(color::PLAYHEAD)))
        .children(snap)
        .children(drop_line)
}

/// The headers beside the track rows, top to bottom: each track's number (01 at the top), an
/// eye that hides or shows its clips, and a speaker that mutes or unmutes them. A button is
/// greyed out on a track where it would do nothing (no sound on a title track, say).
pub fn headers(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let compact = workspace.ui.settings.compact_timeline;
    let rows = layout(workspace).into_iter().enumerate().map(|(index, (track, height))| {
        div()
            .id(ElementId::Name(format!("track-header:{track}").into()))
            .h(px(height))
            .flex_none()
            .border_b_1()
            .border_color(hsla(color::BORDER))
            .bg(hsla(if track == MAIN_TRACK { color::PANEL } else { color::INSET }))
            .child(
                // Lined up with the track's own row, above any keyframe rows of opened clips.
                div()
                    .h(px(base_height(track, compact)))
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .px_1p5()
                    .child(
                        div()
                            .id(ElementId::Name(format!("track-number:{track}").into()))
                            .w(px(20.0))
                            .flex_none()
                            .font_family(crate::theme::MONO_FONT)
                            .text_size(px(crate::theme::size::SMALL_TEXT))
                            .text_color(hsla(color::MUTED))
                            .child(track_number(index))
                            .test_support(),
                    )
                    .child(track_toggle(state, Some(track), Switch::Visible, cx))
                    .child(track_toggle(state, Some(track), Switch::Audio, cx)),
            )
            .test_support()
            .into_any_element()
    });
    div()
        .id("track-headers")
        .w(px(HEADER_WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .border_r_1()
        .border_color(hsla(color::HAIRLINE))
        // A press here isn't a press on the tracks (which would move the playhead).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(rows.collect::<Vec<_>>())
}

/// The eye (Visible) or speaker (Audio) button for one track, or for every track with `None`.
/// It shows the crossed-out icon while the switch is off for all of them, and is greyed out
/// where it would do nothing.
fn track_toggle(state: &crate::state::EditorState, track: Option<i32>, switch: Switch, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let on = state.track_switch(track, switch);
    let off = on == Some(false);
    let (icon, which, verb) = match (switch, off) {
        (Switch::Audio, false) => (IconName::Volume2, "track-mute", "Mute"),
        (Switch::Audio, true) => (IconName::VolumeX, "track-mute", "Unmute"),
        (_, false) => (IconName::Eye, "track-eye", "Hide"),
        (_, true) => (IconName::EyeOff, "track-eye", "Show"),
    };
    let (id, tip) = match track {
        Some(track) => (format!("{which}:{track}"), format!("{verb} this track")),
        None => (format!("{which}:all"), format!("{verb} every track")),
    };
    icon_button(ElementId::Name(id.into()), icon, tip, off, cx.listener(move |this, _, _, cx| {
        let turn_on = this.state.track_switch(track, switch) == Some(false);
        this.state.set_track_switch(track, switch, turn_on);
        cx.notify();
    }))
    .disabled(on.is_none())
}

/// The top of the track header column, beside the navigator and ruler: one block whose bottom
/// row, level with the ruler, holds the eye and mute buttons for every track at once.
pub fn headers_top(workspace: &Workspace, height: f32, bottom_row: f32, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    div()
        .id("track-headers-top")
        .w(px(HEADER_WIDTH))
        .h(px(height))
        .flex_none()
        .flex()
        .flex_col()
        .justify_end()
        .border_r_1()
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(
            div()
                .h(px(bottom_row))
                .flex()
                .items_center()
                .gap_0p5()
                .px_1p5()
                .child(div().w(px(20.0)).flex_none().text_size(px(crate::theme::size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).child("All"))
                .child(track_toggle(state, None, Switch::Visible, cx))
                .child(track_toggle(state, None, Switch::Audio, cx)),
        )
}

/// The number shown on the `index`th track from the top: 01, 02, and so on.
fn track_number(index: usize) -> String {
    format!("{:02}", index + 1)
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
/// A clip's name in its header, or a text field while it is being renamed.
fn clip_name(layer: &Layer, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    if let Some(input) = workspace.ui.editing.as_ref().filter(|editing| editing.target == EditTarget::LayerName(layer.id)).and_then(|editing| editing.input.line()) {
        return div()
            .flex_1()
            .min_w_0()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(crate::panels::chrome::name_input(input, CLIP_NAME_TEXT))
            .into_any_element();
    }
    // Double-clicking the name renames the clip (double-clicking the rest of a nested
    // composition's clip opens it).
    let (id, name) = (layer.id, layer.name.clone());
    div()
        .id(("clip-name", id.0 as usize))
        .flex_1()
        .min_w_0()
        .truncate()
        .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, window, cx| {
            if event.click_count >= 2 {
                cx.stop_propagation();
                this.begin_edit(EditTarget::LayerName(id), name.clone(), window, cx);
            }
        }))
        .child(layer.name.clone())
        .test_support()
        .into_any_element()
}

/// The size of a clip's name in its header, and of the field it turns into when renamed.
const CLIP_NAME_TEXT: f32 = 11.0;

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
    let compact = workspace.ui.settings.compact_timeline;
    let height = base_height(track, compact) - 8.0;
    let cutting = workspace.ui.tool == Tool::Cut;
    let body_tool = matches!(workspace.ui.tool, Tool::Slip | Tool::Slide | Tool::TrackSelect);
    let held = state.clip_drag_target().is_some_and(|(layer, _)| layer == id) || state.slipping_layer() == Some(id);
    // The frame under the pointer, for the Cut tool.
    let pointer_frame = |this: &Workspace, x: gpui_kit::Pixels| frame_at(f32::from(x), this.ui.timeline.graph.get(), this.state.view_range).round() as i64;

    let mut inside: Vec<AnyElement> = Vec::new();
    let has_strip = matches!(layer.content, LayerContent::Footage { .. }) && layer.has_audio;
    let media_height = height - CLIP_HEADER - if has_strip { CLIP_SOUND_STRIP } else { 0.0 };
    // Compact clips are just their name strip.
    match &layer.content {
        _ if compact => {}
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
            inside.push(div().px_1p5().truncate().text_size(px(crate::theme::size::SMALL_TEXT)).text_color(rgba(0xffffffcc)).child(text.lines().next().unwrap_or_default().to_owned()).into_any_element());
        }
        _ => {}
    }

    let stretchable = studio_core::speed::has_speed(&layer.content);
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
                    // Rate Stretch changes footage's speed; anything else just trims.
                    Tool::RateStretch if stretchable => LayerTimeEdit::Stretch(end),
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
    let has_sound = studio_core::audio::layer_has_sound(&workspace.state.project, layer);
    let fade = |seconds: f32, from_left: bool| {
        (has_sound && seconds > 0.0).then(|| {
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
        .text_size(px(CLIP_NAME_TEXT))
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
                    // The chevron always opens to every animated property.
                    this.ui.timeline.only_property.remove(&id);
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
        .child(clip_name(layer, workspace, cx))
        .when(!layer.effects.is_empty(), |header| header.child(div().px_1().rounded(px(3.0)).bg(rgba(0x00000026)).child(format!("fx {}", layer.effects.len()))))
        .when(studio_core::audio::layer_has_sound(&workspace.state.project, layer), |header| header.child(clip_toggle(id, "clip-mute", switches.audio, (IconName::Volume2, IconName::VolumeX), Switch::Audio, dark_ink, cx)))
        // Shown only while motion blur is on (it is turned on in the Properties panel), so the
        // clips that blur stand out and can be switched back off here.
        .when(switches.motion_blur, |header| header.child(clip_toggle(id, "clip-motion-blur", true, (IconName::Wind, IconName::Wind), Switch::MotionBlur, dark_ink, cx)))
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
                drag.apply(&mut this.state, id);
                cx.notify();
            }))
            // A transition dropped on a clip goes on the cut nearer the pointer: its start or end.
            .drag_over::<crate::workspace::TransitionDrag>(|style, _, _, _| style.border_color(hsla(0xffffff)))
            .on_drop(cx.listener(move |this, drag: &crate::workspace::TransitionDrag, window, cx| {
                let frame = pointer_frame(this, window.mouse_position().x);
                this.state.add_transition(id, frame * 2 < in_frame + out_frame, drag.transition, 0.6);
                cx.notify();
            }))
            // The right-click menu covers the clip's header and body; the trim grips and fade
            // handles sit on top of it.
            .child(
                div()
                    .id(("clip-body", id.0 as usize))
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(header)
                    .children(inside)
                    .test_support()
                    .context_menu(crate::panels::layer_menu::build(Some(layer.id), cx.weak_entity())),
            )
            .children(fades.into_iter().flatten())
            .children(cut_line)
            // Trim grips would catch clicks near the ends, so the Cut tool goes without them.
            // So do the body-dragging tools (Slip, Slide, Track Select).
            .when(!cutting && !body_tool, |clip| clip.child(grip(LayerTimeEdit::TrimIn, cx).left_0()).child(grip(LayerTimeEdit::TrimOut, cx).right_0()))
            .test_support()
            .into_any_element(),
    ];

    if expanded {
        elements.push(property_rows(layer, left, width, base_height(track, compact), head, workspace, cx).into_any_element());
    }
    elements
}

/// The rows under an expanded clip: one per animated property, with its name above a line
/// across its keyframes. Pressing a row selects all its keyframes; keyframes select (Shift
/// adds) and drag in time; clicking the line between two keyframes selects that segment and
/// opens the graph editor to ease it, as does double-clicking a keyframe.
fn property_rows(layer: &Layer, left: f32, width: f32, top: f32, head: u32, workspace: &Workspace, cx: &mut Context<Workspace>) -> Div {
    let state = &workspace.state;
    let view = state.view_range;
    let id = layer.id;
    let properties = shown_properties(workspace, layer);
    // What to say when there is nothing to show: the one property asked for has no keyframes,
    // or nothing on the layer is animated.
    let nothing = match workspace.ui.timeline.only_property.get(&id).and_then(|&path| layer.property(path, 0)) {
        Some(property) => format!("No {} keyframes yet: animate it in Properties", property.name),
        None => "No keyframes yet: animate a property in Properties".to_owned(),
    };
    // Keyframe positions are fractions of the visible range; inside this box they are measured
    // from its left edge, so convert.
    let local = move |frame: i64| (fraction(frame as f64, view) - left) / width.max(1e-6);
    let tint = |alpha: u32| -> gpui_kit::Hsla { rgba((head << 8) | alpha).into() };
    let rows: Vec<AnyElement> = if properties.is_empty() {
        vec![div().h(px(PROPERTY_ROW)).flex().items_center().px_2().text_size(px(crate::theme::size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(nothing).into_any_element()]
    } else {
        properties
            .into_iter()
            .map(|(path, name, frames)| {
                let all_selected = frames.iter().all(|&frame| state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame }));
                // The line between each pair of keyframes: click it to ease that segment.
                let segments = frames.windows(2).map(|pair| {
                    let (from, to) = (pair[0], pair[1]);
                    let selected = state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame: from }) && state.is_keyframe_selected(&KeyframeRef { layer: id, path, frame: to });
                    // The segment the graph editor is easing, when the selection has several.
                    let focused = crate::panels::graph_editor::is_focused_segment(workspace, KeyframeRef { layer: id, path, frame: from });
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
                        .child(div().w_full().h(px(if selected { 2.0 } else { 1.0 })).bg(if focused { hsla(color::ACCENT) } else if selected { hsla(0xffffff) } else { tint(0x99) }))
                        .on_mouse_down(MouseButton::Left, cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.state.set_keyframe_selection(vec![KeyframeRef { layer: id, path, frame: from }, KeyframeRef { layer: id, path, frame: to }]);
                            crate::panels::graph_editor::show(this);
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
                                crate::panels::graph_editor::show(this);
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
                            .text_size(px(crate::theme::size::CAPTION_TEXT))
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
    // Settings can turn the pictures off.
    let Some(path) = source.path.as_ref().filter(|_| workspace.ui.settings.clip_thumbnails) else { return div().h(px(height)).into_any_element() };
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
    // Settings can turn waveforms off.
    let Some(peaks) = peaks.filter(|_| workspace.ui.settings.clip_waveforms) else { return div().h(px(height)).into_any_element() };
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


#[cfg(test)]
mod tests {
    use super::track_number;

    #[test]
    fn tracks_are_numbered_from_01_with_two_digits() {
        assert_eq!(track_number(0), "01");
        assert_eq!(track_number(8), "09");
        assert_eq!(track_number(11), "12");
        assert_eq!(track_number(99), "100");
    }
}
