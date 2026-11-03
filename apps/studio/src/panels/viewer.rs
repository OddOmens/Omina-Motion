//! The Composition panel: the frame rendered by `studio-render` on the GPU, with the
//! selection outline, handles and anchor point drawn on top, and the transport bar.
//! Paused frames render at full display resolution; playback renders at half
//! resolution on high-density screens to keep up (After Effects' "Auto" resolution).

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, CursorStyle, DragMoveEvent, ElementId, ImageSource, InteractiveElement, RenderImage, img,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _,
    Window, div,
    PathBuilder, canvas, point, prelude::FluentBuilder as _, px, relative,
};
use std::sync::Arc;

use studio_core::animation::Vec2;
use studio_core::model::Id;
use studio_core::time::format_timecode;
use studio_core::transform::layer_corners;
use studio_media::{FrameServer, ProxyManager};
use studio_render::Renderer;

use super::chrome::{EmptyDragView, icon_button, panel};
use crate::state::ScaleHandle;
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, EffectDrag, PanelId, Tool, Workspace};

/// Space kept between the composition frame and the viewer edge.
const FIT_PADDING: f32 = 24.0;

/// Rendering state for the viewer: the GPU renderer, or why it could not start, and the frame on screen.
pub struct ViewerRender {
    renderer: Result<Renderer, String>,
    /// Builds proxies of imported footage in the background.
    pub proxies: Arc<ProxyManager>,
    /// Decodes footage on background threads; the renderer asks it for frames.
    pub media: FrameServer,
    image: Option<Arc<RenderImage>>,
    last_error: Option<String>,
    /// Whether the frame on screen shows footage from a proxy (it is replaced as full quality arrives).
    showing_proxy: bool,
    /// How long the last viewer frame took to render, shown under the timeline.
    pub last_render_ms: Option<f64>,
    /// What the frame on screen was drawn from, so an unchanged frame is not drawn again.
    drawn: Option<DrawnFrame>,
    /// How many viewer frames have been drawn, so tests can check that unchanged ones aren't.
    #[cfg(test)]
    pub frames_drawn: usize,
}

/// Everything a viewer frame depends on. The cheap parts are checked first, so a changed
/// frame is told apart before the projects are compared.
struct DrawnFrame {
    frame: i64,
    comp: Id,
    width: u32,
    height: u32,
    /// The frame server's count of arrived frames (see `FrameServer::generation`).
    media: u64,
    /// The project as drawn: the editor's, or the copy previewing a LUT.
    project: studio_core::model::Project,
}


/// A gray checkerboard, made once, shown under transparent parts of the composition.
fn transparency_grid() -> Arc<RenderImage> {
    static GRID: std::sync::OnceLock<Arc<RenderImage>> = std::sync::OnceLock::new();
    GRID.get_or_init(|| {
        let (size, cell) = (1024u32, 16u32);
        let mut bgra = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let shade = if (x / cell + y / cell) % 2 == 0 { 0x2a } else { 0x1e };
                bgra.extend([shade, shade, shade, 255]);
            }
        }
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(size, size, bgra).expect("the grid holds size × size pixels");
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    })
    .clone()
}

/// Footage is decoded at most this wide for the viewer.
const PREVIEW_DECODE_WIDTH: u32 = 1920;

impl ViewerRender {
    pub fn new() -> Self {
        // Headless tests keep their proxies apart, so clearing the cache in a test never touches real ones.
        // Headless tests get a cache folder of their own (see `cache_root`).
        let folder = crate::app_settings::cache_root().join("Proxies");
        let proxies = Arc::new(ProxyManager::new(folder));
        #[cfg(not(test))]
        proxies.set_cache_limit(crate::app_settings::load().proxy_cache_limit_gb.map(|gb| u64::from(gb) << 30));
        Self {
            renderer: Renderer::new().map_err(|error| error.to_string()),
            media: FrameServer::new(PREVIEW_DECODE_WIDTH, Some(proxies.clone())),
            proxies,
            image: None,
            last_error: None,
            showing_proxy: false,
            last_render_ms: None,
            drawn: None,
            #[cfg(test)]
            frames_drawn: 0,
        }
    }
}

impl ViewerRender {
    /// A small picture of an element (see `studio_core::elements`) on a plain square, `size`
    /// pixels a side, for the Elements panel.
    pub fn element_picture(&mut self, index: usize, size: u32) -> Option<Arc<RenderImage>> {
        let renderer = self.renderer.as_mut().ok()?;
        let mut project = studio_core::model::Project::new(Vec::new(), Vec::new());
        let settings = studio_core::edit::CompositionSettings { name: "Element".into(), width: 640, height: 640, frame_rate: studio_core::time::FrameRate::new(30, 1), duration_frames: 1, background: studio_core::model::Rgba::TRANSPARENT };
        let comp = studio_core::edit::new_composition(&mut project, &settings).ok()?;
        studio_core::elements::add(&mut project, comp, index, Vec2::new(320.0, 320.0), 0);
        // Fit what was added into the square: elements are sized for a 1080-pixel frame.
        let measured = renderer.render(&project, comp, 0, size, size, &studio_core::media::NoVideo).ok()?;
        for (layer, text_size) in measured.text_sizes {
            studio_core::edit::set_text_box(project.composition_mut(comp)?, layer, text_size);
        }
        let picture = renderer.render(&project, comp, 0, size, size, &studio_core::media::NoVideo).ok()?;
        let bgra: Vec<u8> = picture
            .bgra
            .chunks(4)
            .flat_map(|pixel| {
                let a = f32::from(pixel[3]) / 255.0;
                let over = |c: u8| (f32::from(c) * a + 46.0 * (1.0 - a)) as u8;
                [over(pixel[0]), over(pixel[1]), over(pixel[2]), 255]
            })
            .collect();
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(size, size, bgra)?;
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    }

    /// The composition's frame drawn `width` × `height` for the LUT previews, from the
    /// viewer's own decoded footage. The flag says whether every video frame in it was the one
    /// asked for; one still decoding is stood in for by a nearby frame, or left out.
    pub fn preview_frame(&mut self, project: &studio_core::model::Project, comp: studio_core::model::Id, frame: i64, width: u32, height: u32) -> Option<(studio_render::RenderedFrame, bool)> {
        let renderer = self.renderer.as_mut().ok()?;
        let watched = WatchedVideo { inner: &self.media, exact: std::cell::Cell::new(true) };
        let picture = renderer.render(project, comp, frame, width, height, &watched).ok()?;
        Some((picture, watched.exact.get()))
    }

    /// A small picture of a device mockup, for the Mockups panel: built into a scratch project
    /// and drawn once, fitted into a `side` × `side` square keeping its shape, over a dark
    /// gray where it is see-through.
    pub fn mockup_picture(&mut self, request: &studio_core::mockups::MockupRequest, side: u32) -> Option<Arc<RenderImage>> {
        let renderer = self.renderer.as_mut().ok()?;
        let mut project = studio_core::model::Project::new(Vec::new(), Vec::new());
        let made = studio_core::mockups::build(&mut project, request)?;
        let composition = project.composition(made.composition)?;
        let fit = side as f32 / composition.width.max(composition.height) as f32;
        let (width, height) = (((composition.width as f32 * fit).round() as u32).max(1), ((composition.height as f32 * fit).round() as u32).max(1));
        let picture = renderer.render(&project, made.composition, 0, width, height, &studio_core::media::NoVideo).ok()?;
        let bgra: Vec<u8> = picture
            .bgra
            .chunks(4)
            .flat_map(|pixel| {
                let a = f32::from(pixel[3]) / 255.0;
                let over = |c: u8| (f32::from(c) * a + 12.0 * (1.0 - a)) as u8;
                [over(pixel[0]), over(pixel[1]), over(pixel[2]), 255]
            })
            .collect();
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(picture.width, picture.height, bgra)?;
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    }

    /// A small picture of a template, for the Templates panel: built into a scratch project and
    /// rendered `seconds` in, `width` × `height` pixels. Drawn twice, as the app does, so text
    /// boxes fit their measured words.
    pub fn template_picture(&mut self, template: &studio_core::templates::Template, format: studio_core::templates::Format, seconds: f64, width: u32, height: u32) -> Option<Arc<RenderImage>> {
        let renderer = self.renderer.as_mut().ok()?;
        let mut project = studio_core::model::Project::new(Vec::new(), Vec::new());
        let comp = studio_core::templates::build(&mut project, template, format)?;
        let frame = (seconds * 30.0) as i64;
        let measured = renderer.render(&project, comp, frame, width, height, &studio_core::media::NoVideo).ok()?;
        for (layer, size) in measured.text_sizes {
            studio_core::edit::set_text_box(project.composition_mut(comp)?, layer, size);
        }
        let picture = renderer.render(&project, comp, frame, width, height, &studio_core::media::NoVideo).ok()?;
        // Straight-alpha pixels over a dark gray, so see-through templates still read.
        let bgra: Vec<u8> = picture
            .bgra
            .chunks(4)
            .flat_map(|pixel| {
                let a = f32::from(pixel[3]) / 255.0;
                let over = |c: u8| (f32::from(c) * a + 52.0 * (1.0 - a)) as u8;
                [over(pixel[0]), over(pixel[1]), over(pixel[2]), 255]
            })
            .collect();
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, bgra)?;
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
    }
}

/// Passes video frames through from the frame server, noting whether any wasn't the exact
/// frame asked for (still on its way).
struct WatchedVideo<'a> {
    inner: &'a FrameServer,
    exact: std::cell::Cell<bool>,
}

impl studio_core::media::VideoSource for WatchedVideo<'_> {
    fn frame(&self, path: &std::path::Path, index: i64) -> Option<Arc<studio_core::media::VideoFrame>> {
        let frame = studio_core::media::VideoSource::frame(self.inner, path, index);
        if frame.as_ref().is_none_or(|frame| frame.index != index) {
            self.exact.set(false);
        }
        frame
    }
}

/// The project as the viewer shows it while a LUT in the Effects panel is under the pointer: a
/// copy with that LUT added to the selected clips that take grading, or, with none selected,
/// on an adjustment layer over the whole picture. `None` when nothing is being previewed.
pub(crate) fn lut_preview(workspace: &Workspace) -> Option<studio_core::model::Project> {
    let lut = workspace.ui.lut_hover.clone()?;
    let state = &workspace.state;
    let comp = state.active_comp;
    let mut project = state.project.clone();
    let graded: Vec<Id> = state
        .selected_layers
        .iter()
        .copied()
        .filter(|id| state.composition().layer(*id).is_some_and(|layer| studio_core::effects::suits_layer("Apply LUT", &layer.content)))
        .collect();
    let targets = if graded.is_empty() { vec![studio_core::edit::add_adjustment_layer(&mut project, comp, 0)?] } else { graded };
    for layer in targets {
        studio_core::edit::add_lut(&mut project, comp, layer, lut.clone());
    }
    Some(project)
}

/// Renders the frame for the viewer at `width` × `height` device pixels and swaps it on screen.
///
/// The viewer is redrawn far more often than its picture changes: on every edit anywhere, and
/// on every pointer move during any drag (the UI library redraws the whole window then). So a
/// frame is only drawn again when something it depends on changed. While a drag is resizing
/// the viewer, the picture on screen is stretched to the new size instead, and drawn sharp at
/// the new size once the drag ends.
fn render_frame(workspace: &mut Workspace, width: u32, height: u32, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.render.renderer.is_err() {
        return;
    }
    let state = &workspace.state;
    // While playing (the composition, or a preview in the Project panel), decoders work ahead.
    workspace.render.media.set_playing(state.is_playing() || workspace.ui.preview.playing.is_some());
    // Let go of footage the open composition no longer uses (deleted layers, other
    // compositions), so its decoder threads stop and its frames are freed.
    // The footage previewed in the Project panel is kept too.
    let previewed = workspace.ui.preview.item.and_then(|item| state.project.item(item)).and_then(|item| item.path.as_deref());
    let footage: std::collections::HashSet<&std::path::Path> = state
        .composition()
        .layers
        .iter()
        .filter_map(|layer| match layer.content {
            studio_core::model::LayerContent::Footage { item } => state.project.item(item)?.path.as_deref(),
            _ => None,
        })
        .chain(previewed)
        .collect();
    workspace.render.media.release_unused(|path| footage.contains(path));
    let previewed = lut_preview(workspace);
    let state = &workspace.state;
    let shown = previewed.as_ref().unwrap_or(&state.project);
    // Cut-outs load their masks from disk as they are made, which the project doesn't record,
    // so frames are always drawn afresh while one is being made.
    let reusable = workspace.ui.cutout_job.is_none() && workspace.render.image.is_some();
    if let Some(drawn) = workspace.render.drawn.as_ref().filter(|_| reusable) {
        let fresh = drawn.frame == state.current_frame && drawn.comp == state.active_comp && drawn.media == workspace.render.media.generation() && drawn.project == *shown;
        if fresh && ((drawn.width, drawn.height) == (width, height) || cx.has_active_drag()) {
            return;
        }
    }
    let started = std::time::Instant::now();
    // Read before drawing: a frame arriving while this one is drawn leaves it out of date.
    let media = workspace.render.media.generation();
    let Ok(renderer) = workspace.render.renderer.as_mut() else { return };
    let rendered = renderer.render(shown, state.active_comp, state.current_frame, width, height, &workspace.render.media);
    workspace.render.last_render_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
    #[cfg(test)]
    {
        workspace.render.frames_drawn += 1;
    }
    workspace.render.drawn = rendered.is_ok().then(|| DrawnFrame { frame: state.current_frame, comp: state.active_comp, width, height, media, project: shown.clone() });
    match rendered {
        Ok(frame) => {
            workspace.render.last_error = None;
            workspace.render.showing_proxy = frame.used_proxy;
            // Keep text layer bounds matching the measured text.
            workspace.state.apply_text_sizes(&frame.text_sizes);
            let Some(buffer) = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(frame.width, frame.height, frame.bgra) else { return };
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
            // Free the previous frame's texture; otherwise every frame would stay in GPU memory.
            if let Some(previous) = workspace.render.image.replace(image) {
                cx.drop_image(previous, Some(window));
            }
        }
        Err(error) => workspace.render.last_error = Some(error.to_string()),
    }
}

/// The magnification menu: Fit, or a fixed zoom. It shows the zoom in use.
fn zoom_menu(workspace: &Workspace, scale: f32, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let current = workspace.ui.viewer_zoom;
    let label = match current {
        None => format!("Fit ({}%)", (scale * 100.0).round()),
        Some(_) => zoom_label(current),
    };
    let target = cx.weak_entity();
    Button::new("viewer-zoom").ghost().xsmall().label(label).dropdown_caret(true).dropdown_menu(move |menu, _, _| {
        std::iter::once(None).chain(ZOOM_LEVELS.iter().copied().filter(|&level| level >= 0.125).map(Some)).fold(menu, |menu, zoom| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(zoom_label(zoom)).checked(zoom == current).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        set_zoom(this, zoom);
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
}

/// The playback quality menu (also in Settings > Playback): how sharp the viewer is while
/// playing. Paused frames are always full quality.
fn quality_menu(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    use crate::app_settings::PlaybackQuality;
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let current = workspace.ui.settings.playback_quality;
    let target = cx.weak_entity();
    Button::new("viewer-quality").ghost().xsmall().label(current.name()).dropdown_caret(true).tooltip("Quality while playing").dropdown_menu(move |menu, _, _| {
        PlaybackQuality::ALL.into_iter().fold(menu, |menu, quality| {
            let target = target.clone();
            menu.item(PopupMenuItem::new(quality.name()).checked(quality == current).on_click(move |_, _, cx| {
                target
                    .update(cx, |this, cx| {
                        set_playback_quality(this, quality);
                        cx.notify();
                    })
                    .ok();
            }))
        })
    })
}

/// Uses `quality` while playing from now on, and remembers it.
pub fn set_playback_quality(workspace: &mut Workspace, quality: crate::app_settings::PlaybackQuality) {
    workspace.ui.settings.playback_quality = quality;
    crate::app_settings::update(|settings| settings.playback_quality = quality);
}

/// The bar under the viewer (the panel's footer), in one row: magnification and frame size on the left, the play
/// and frame-stepping controls in the middle, and the current timecode on the right.
fn transport(workspace: &Workspace, scale: f32, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let playing = workspace.state.is_playing();
    let looping = workspace.state.loop_playback;
    let composition = workspace.state.composition();
    // Equal-width sides keep the controls centred under the picture.
    let info = div()
        .flex_1()
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(quality_menu(workspace, cx))
        .child(zoom_menu(workspace, scale, cx))
        .child(format!("{} × {}", composition.width, composition.height))
        .child(icon_button("viewer-snapping", IconName::Magnet, "Snap anchor points to corners, edges and center", workspace.state.viewer_snapping, cx.listener(|this, _, _, cx| {
            this.state.viewer_snapping = !this.state.viewer_snapping;
            cx.notify();
        })));
    let timecode = div().flex_1().flex().justify_end().child(
        div()
            .px_2()
            .rounded(px(2.0))
            .bg(hsla(color::INSET))
            .text_size(px(size::SMALL_TEXT))
            .text_color(hsla(color::VALUE))
            .font_family("Menlo")
            .child(format_timecode(workspace.state.current_frame, composition.frame_rate)),
    );
    let controls = div()
        .flex()
        .items_center()
        .gap_1()
        // The same as Home and End: the composition's first and last frames.
        .child(icon_button("first-frame", IconName::SkipBack, "First Frame (Home)", false, cx.listener(|this, _, _, cx| this.go_to(0, cx))))
        .child(icon_button("previous-frame", IconName::StepBack, "Previous Frame (Page Up)", false, cx.listener(
            |this, _, _, cx| {
                let frame = this.state.current_frame - 1;
                this.go_to(frame, cx);
            },
        )))
        .child(icon_button(
            "play",
            if playing { IconName::Pause } else { IconName::Play },
            if playing { "Pause (Space)" } else { "Play (Space)" },
            playing,
            cx.listener(|this, _, _, cx| this.toggle_playback(cx)),
        ))
        .child(icon_button("next-frame", IconName::StepForward, "Next Frame (Page Down)", false, cx.listener(
            |this, _, _, cx| {
                let frame = this.state.current_frame + 1;
                this.go_to(frame, cx);
            },
        )))
        .child(icon_button("last-frame", IconName::SkipForward, "Last Frame (End)", false, cx.listener(|this, _, _, cx| {
            let end = this.state.composition().duration_frames - 1;
            this.go_to(end, cx);
        })))
        .child(div().w(px(8.0)))
        .child(icon_button("loop", IconName::Repeat, "Loop Playback", looping, cx.listener(|this, _, _, cx| {
            this.state.loop_playback = !this.state.loop_playback;
            cx.notify();
        })));
    // Drawn in the panel's footer, so it lines up with the Project panel's footer beside it.
    div()
        .size_full()
        .flex()
        .items_center()
        .px_1()
        .child(info)
        .child(controls)
        .child(timecode)
}

/// Viewer drags: moving the selection, or scaling from a corner handle.
#[derive(Clone, Copy, Debug)]
enum ViewerDrag {
    Move,
    Scale,
    Anchor,
    PathPoint,
}

/// Screen points per composition pixel that fit the whole frame in the viewer.
fn fit_scale(workspace: &Workspace) -> f32 {
    let composition = workspace.state.composition();
    let bounds = workspace.ui.viewer_bounds.get();
    ((bounds.width - FIT_PADDING * 2.0) / composition.width as f32)
        .min((bounds.height - FIT_PADDING * 2.0) / composition.height as f32)
        .max(0.0)
}

/// Screen points per composition pixel: the chosen magnification, or Fit.
fn viewer_scale(workspace: &Workspace) -> f32 {
    workspace.ui.viewer_zoom.unwrap_or_else(|| fit_scale(workspace))
}

/// The magnifications the Zoom tool steps through and the menu offers.
const ZOOM_LEVELS: [f32; 10] = [0.0625, 0.125, 0.25, 0.33, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

/// Zooms the viewer in (or out) one step, keeping the composition point `at` under the pointer.
fn zoom_at(workspace: &mut Workspace, at: Vec2, zoom_in: bool) {
    let current = viewer_scale(workspace);
    let next = if zoom_in {
        ZOOM_LEVELS.iter().copied().find(|&level| level > current * 1.01)
    } else {
        ZOOM_LEVELS.iter().rev().copied().find(|&level| level < current * 0.99)
    };
    let Some(next) = next else { return };
    // The point stays put on screen: the frame's corner moves by how much farther (or nearer)
    // the point now is from it, less the half of the growth that centering already gives.
    let composition = workspace.state.composition();
    let half = Vec2::new(composition.width as f32 * 0.5, composition.height as f32 * 0.5);
    let shift = (at - half) * (current - next);
    let (x, y) = workspace.ui.viewer_pan;
    workspace.ui.viewer_pan = (x + shift.x, y + shift.y);
    workspace.ui.viewer_zoom = Some(next);
}

/// Sets the magnification from the menu (`None` for Fit), centering the frame.
fn set_zoom(workspace: &mut Workspace, zoom: Option<f32>) {
    workspace.ui.viewer_zoom = zoom;
    workspace.ui.viewer_pan = (0.0, 0.0);
}

fn zoom_label(zoom: Option<f32>) -> String {
    zoom.map_or_else(|| "Fit".to_owned(), |zoom| format!("{}%", (zoom * 100.0).round()))
}

/// The layer whose subject is being picked for background removal, while it is still the one
/// selected (picking ends when the selection moves on).
fn picking_layer(workspace: &Workspace) -> Option<Id> {
    workspace.ui.cutout_pick.filter(|layer| workspace.state.selected_layers == [*layer])
}

/// The clicks picking a cut-out's subject: green dots on what to keep, red on what to leave out.
fn cutout_clicks_overlay(workspace: &Workspace) -> Vec<AnyElement> {
    let Some(layer) = picking_layer(workspace) else { return Vec::new() };
    let composition = workspace.state.composition();
    let (width, height) = (composition.width as f32, composition.height as f32);
    workspace
        .state
        .cutout_clicks_on_screen(layer)
        .into_iter()
        .map(|(at, keep)| {
            div()
                .absolute()
                .left(relative(at.x / width))
                .top(relative(at.y / height))
                .ml(px(-6.0))
                .mt(px(-6.0))
                .size(px(12.0))
                .rounded_full()
                .border_2()
                .border_color(gpui_kit::rgb(0xffffff))
                .bg(gpui_kit::rgb(if keep { 0x22c55e } else { 0xef4444 }))
                .into_any_element()
        })
        .collect()
}

/// The color of the rendered frame at composition point `point`, as shown (transparent areas
/// keep their transparency).
fn color_at(workspace: &Workspace, point: Vec2) -> Option<studio_core::model::Rgba> {
    let image = workspace.render.image.as_ref()?;
    let size = image.size(0);
    let (width, height) = (size.width.0.max(0) as usize, size.height.0.max(0) as usize);
    let composition = workspace.state.composition();
    let (fx, fy) = (point.x / composition.width as f32, point.y / composition.height as f32);
    if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
        return None;
    }
    let (x, y) = ((fx * width as f32) as usize, (fy * height as f32) as usize);
    let bytes = image.as_bytes(0)?;
    let pixel = bytes.get((y * width + x) * 4..(y * width + x) * 4 + 4)?;
    // The frame is stored blue, green, red, alpha.
    let rgb = (u32::from(pixel[2]) << 16) | (u32::from(pixel[1]) << 8) | u32::from(pixel[0]);
    Some(studio_core::model::Rgba::hex(rgb).with_alpha(f32::from(pixel[3]) / 255.0))
}

/// How near (in screen points) a Pen click must be to the path's first point to close it.
const CLOSE_PATH_REACH: f32 = 8.0;

/// Draws curves given in composition pixels as a thin line over the frame.
fn curve_canvas(segments: Vec<[Vec2; 4]>, composition_size: Vec2, line: gpui_kit::Hsla) -> AnyElement {
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let at = |v: Vec2| point(bounds.origin.x + bounds.size.width * (v.x / composition_size.x), bounds.origin.y + bounds.size.height * (v.y / composition_size.y));
        let mut path = PathBuilder::stroke(px(1.5));
        for [start, control_a, control_b, end] in &segments {
            path.move_to(at(*start));
            path.cubic_bezier_to(at(*end), at(*control_a), at(*control_b));
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, line);
        }
    })
    .absolute()
    .size_full()
    .into_any_element()
}

/// A small square marking a path point (filled for the first point of a path being drawn, which
/// closes it when clicked).
fn point_mark(at: (f32, f32), filled: bool) -> gpui_kit::Div {
    div()
        .absolute()
        .left(relative(at.0))
        .top(relative(at.1))
        .ml(px(-4.0))
        .mt(px(-4.0))
        .size(px(8.0))
        .border_1()
        .border_color(hsla(color::ACCENT))
        .bg(if filled { hsla(color::ACCENT) } else { hsla(color::PANEL) })
}

/// The pen path being drawn: its curves so far and its points.
fn pen_draft_overlay(workspace: &Workspace) -> Vec<AnyElement> {
    let Some(pen) = workspace.state.pen.as_ref() else { return Vec::new() };
    let composition = workspace.state.composition();
    let size = Vec2::new(composition.width as f32, composition.height as f32);
    let frac = |v: Vec2| (v.x / size.x, v.y / size.y);
    let mut elements = vec![curve_canvas(pen.segments(), size, hsla(color::ACCENT))];
    for (index, point) in pen.points.iter().enumerate() {
        // The handles of a smooth point, as in every pen tool.
        if point.handle_out.length() > 0.0 {
            let handles = vec![[point.at + point.handle_in, point.at + point.handle_in, point.at + point.handle_out, point.at + point.handle_out]];
            elements.push(curve_canvas(handles, size, hsla(color::MUTED)));
        }
        elements.push(point_mark(frac(point.at), index == 0).into_any_element());
    }
    elements
}

/// The points and handles of a selected pen path layer, each draggable to reshape it.
fn path_points_overlay(workspace: &Workspace, layer: Id, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    use studio_core::edit::PathPart;
    let composition = workspace.state.composition();
    let size = Vec2::new(composition.width as f32, composition.height as f32);
    let frac = |v: Vec2| (v.x / size.x, v.y / size.y);
    let points = workspace.state.path_points_on_screen(layer);
    let mut elements = Vec::new();
    let grab = |id: String, at: (f32, f32), part: PathPart, index: usize, look: gpui_kit::Div, cx: &mut Context<Workspace>| {
        div()
            .id(ElementId::Name(id.into()))
            .absolute()
            .left(relative(at.0))
            .top(relative(at.1))
            .ml(px(-7.0))
            .mt(px(-7.0))
            .size(px(14.0))
            .cursor(CursorStyle::Crosshair)
            .child(look)
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.ui.viewer_anchor = Some((f32::from(event.position.x), f32::from(event.position.y)));
                this.state.begin_path_point_drag(layer, index, part);
                cx.notify();
            }))
            .on_drag(ViewerDrag::PathPoint, |_, _, _, cx| cx.new(|_| EmptyDragView))
            .test_support()
            .into_any_element()
    };
    for (index, point) in points.iter().enumerate() {
        for (part, handle) in [(PathPart::HandleIn, point.handle_in), (PathPart::HandleOut, point.handle_out)] {
            if (handle - point.at).length() < 0.5 {
                continue;
            }
            elements.push(curve_canvas(vec![[point.at, point.at, handle, handle]], size, hsla(color::MUTED)));
            let dot = div().absolute().left(px(3.5)).top(px(3.5)).size(px(7.0)).rounded_full().bg(hsla(color::ACCENT));
            elements.push(grab(format!("path-handle:{}:{index}:{part:?}", layer.0), frac(handle), part, index, dot, cx));
        }
        let square = point_mark((0.5, 0.5), false);
        elements.push(grab(format!("path-point:{}:{index}", layer.0), frac(point.at), PathPart::Point, index, square, cx));
    }
    elements
}

/// Converts a window position to composition pixels, using where the frame was drawn last frame.
fn to_comp(workspace: &Workspace, x: f32, y: f32) -> Vec2 {
    let frame = workspace.ui.frame_bounds.get();
    let scale = viewer_scale(workspace).max(f32::EPSILON);
    Vec2::new((x - frame.left) / scale, (y - frame.top) / scale)
}

/// Press in the viewer: the Type tool places a text layer; the Shape tool starts drawing a
/// shape; the Selection tool picks the layer under the pointer (Shift adds to the selection)
/// and gets ready to move it.
fn press_viewer(workspace: &mut Workspace, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Workspace>) {
    let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
    let point = to_comp(workspace, x, y);
    workspace.ui.viewer_anchor = Some((x, y));
    // The color picker's eyedropper takes the color under the pointer.
    if workspace.ui.color_picker.as_ref().is_some_and(|picker| picker.sampling) {
        if let Some(color) = color_at(workspace, point) {
            super::color_picker::sampled(workspace, color, window, cx);
        }
        cx.notify();
        return;
    }
    // Picking a white balance: the color clicked becomes neutral gray.
    if let Some((layer, effect)) = workspace.ui.white_balance_pick.take() {
        match color_at(workspace, point).filter(|color| color.a > 0.0) {
            Some(color) => workspace.state.neutralize(layer, effect, color),
            None => window.push_notification("Click on something in the picture that should be gray or white.", cx),
        }
        cx.notify();
        return;
    }
    // The Eyedropper tool takes the color under the pointer as the current color; with Shift
    // it also goes on the selected shapes, text and solids.
    if workspace.ui.tool == Tool::Eyedropper {
        match color_at(workspace, point).filter(|color| color.a > 0.0) {
            Some(color) => {
                let color = color.with_alpha(1.0);
                workspace.state.pick_color(color);
                if event.modifiers.shift {
                    workspace.state.apply_color_to_selection(color);
                }
            }
            None => window.push_notification("There is no picture there to take a color from.", cx),
        }
        cx.notify();
        return;
    }
    // Track Motion: the click picks the point of the video to follow.
    if let Some(target) = workspace.ui.track_pick.take() {
        let frame = workspace.state.current_frame;
        let composition = workspace.state.composition();
        let footage = composition
            .layers
            .iter()
            .filter(|layer| layer.id != target && layer.switches.visible && layer.is_active_at(frame) && matches!(layer.content, studio_core::model::LayerContent::Footage { .. }))
            .find(|layer| studio_core::transform::layer_contains(composition, layer.id, point, frame as f64))
            .map(|layer| layer.id);
        match footage {
            Some(footage) => super::analysis::track_motion(workspace, target, footage, point, window, cx),
            None => window.push_notification("Click a point on a video to follow.", cx),
        }
        cx.notify();
        return;
    }
    // Picking a subject to cut out: clicks mark what to keep (Option: what to leave out), and
    // each one updates the mask.
    if let Some(layer) = picking_layer(workspace) {
        if workspace.state.add_cutout_click(layer, point, !event.modifiers.alt) {
            super::cutout::make_masks(workspace, layer, false, window, cx);
        }
        cx.notify();
        return;
    }
    if workspace.ui.tool == Tool::Type {
        if let Some(layer) = workspace.state.add_text_layer("", point) {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
            // Open the text field once this click finishes; focusing it during mouse-down would
            // lose focus to the window as the click completes, closing the field straight away.
            cx.defer_in(window, move |this, window, cx| {
                this.begin_edit(EditTarget::SourceText(layer), String::new(), window, cx);
            });
            cx.notify();
        }
        return;
    }
    match workspace.ui.tool {
        // The Hand drags the picture around; the Zoom tool zooms in where clicked (Option: out).
        Tool::Hand => {
            workspace.ui.pan_start = workspace.ui.viewer_pan;
            cx.notify();
            return;
        }
        Tool::Zoom => {
            zoom_at(workspace, point, !event.modifiers.alt);
            cx.notify();
            return;
        }
        // The Rotate tool turns the layer under the pointer (or the selection, if it is part of it).
        Tool::Rotate => {
            match workspace.state.layer_at(point) {
                Some(layer) if !workspace.state.selected_layers.contains(&layer) => workspace.state.select_layer(layer, event.modifiers.shift),
                None if workspace.state.selected_layers.is_empty() => return,
                _ => {}
            }
            workspace.state.begin_rotate(point);
            cx.notify();
            return;
        }
        _ => {}
    }
    if workspace.ui.tool == Tool::Pen {
        // A double-click ends the path as an open line; a click on its first point closes it.
        let finished = if event.click_count >= 2 {
            workspace.state.finish_pen(false)
        } else {
            let reach = CLOSE_PATH_REACH / viewer_scale(workspace).max(f32::EPSILON);
            workspace.state.pen_press(point, reach)
        };
        if finished.is_some() {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
        }
        cx.notify();
        return;
    }
    if workspace.ui.tool == Tool::Shape {
        let kind = workspace.ui.shape_kind;
        if workspace.state.begin_shape_draw(kind, point).is_some() {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
            cx.notify();
        }
        return;
    }
    // Double-clicking a nested composition opens it, as in After Effects.
    if event.click_count >= 2
        && let Some(inner) = workspace.state.layer_at(point).and_then(|layer| workspace.state.nested_composition(layer))
    {
        workspace.state.open_composition(inner);
        cx.notify();
        return;
    }
    // Double-clicking a text layer edits its text right there, over the layer.
    if event.click_count >= 2
        && let Some(layer) = workspace.state.layer_at(point).filter(|&layer| workspace.state.text_of(layer).is_some())
    {
        let text = workspace.state.text_of(layer).map(|(text, _)| text.to_owned()).unwrap_or_default();
        workspace.state.select_layer(layer, false);
        cx.defer_in(window, move |this, window, cx| {
            this.begin_edit(EditTarget::SourceText(layer), text, window, cx);
            if let Some(editing) = this.ui.editing.as_mut() {
                editing.in_viewer = true;
            }
            cx.notify();
        });
        return;
    }
    let additive = event.modifiers.shift || event.modifiers.platform;
    match workspace.state.layer_at(point) {
        Some(layer) if !workspace.state.selected_layers.contains(&layer) || additive => workspace.state.select_layer(layer, additive),
        Some(_) => {}
        None if !additive => workspace.state.clear_selection(),
        None => {}
    }
    workspace.state.begin_move();
    cx.notify();
}

/// A bright outline around the layer an effect being dragged over the viewer would go on.
fn effect_drop_highlight(workspace: &Workspace) -> Option<AnyElement> {
    let layer = workspace.ui.effect_drop_layer?;
    let composition = workspace.state.composition();
    let corners = layer_corners(composition, layer, workspace.state.current_frame as f64)?;
    let (width, height) = (composition.width as f32, composition.height as f32);
    let outline = corners.map(|v| (v.x / width, v.y / height));
    Some(
        canvas(|_, _, _| (), move |bounds, _, window, _| {
            let at = |(fx, fy): (f32, f32)| point(bounds.origin.x + bounds.size.width * fx, bounds.origin.y + bounds.size.height * fy);
            let mut path = PathBuilder::stroke(px(3.0));
            path.move_to(at(outline[0]));
            for corner in &outline[1..] {
                path.line_to(at(*corner));
            }
            path.line_to(at(outline[0]));
            if let Ok(path) = path.build() {
                window.paint_path(path, hsla(color::HIGHLIGHT));
            }
        })
        .absolute()
        .size_full()
        .into_any_element(),
    )
}

/// The layer an effect dropped at `position` (window coordinates) would go on: the topmost one
/// under the pointer that the effect suits.
fn effect_target(workspace: &Workspace, effect: &str, position: gpui_kit::Point<gpui_kit::Pixels>) -> Option<Id> {
    let point = to_comp(workspace, f32::from(position.x), f32::from(position.y));
    let layer = workspace.state.layer_at(point)?;
    let content = &workspace.state.composition().layer(layer)?.content;
    studio_core::effects::suits_layer(effect, content).then_some(layer)
}

/// The text field for a text layer being edited in the viewer (after a double-click), laid over
/// the layer's bounds. Enter or clicking elsewhere commits it.
fn text_editor_overlay(workspace: &Workspace) -> Option<AnyElement> {
    let editing = workspace.ui.editing.as_ref().filter(|editing| editing.in_viewer)?;
    let EditTarget::SourceText(layer) = editing.target else { return None };
    let composition = workspace.state.composition();
    let bounds = studio_core::align::layer_bounds(composition, layer, workspace.state.current_frame)?;
    let (width, height) = (composition.width as f32, composition.height as f32);
    let (left, top) = (bounds.min.x / width, (bounds.min.y + bounds.max.y) * 0.5 / height);
    let span = ((bounds.max.x - bounds.min.x) / width).max(0.0);
    Some(
        div()
            .absolute()
            .left(relative(left.clamp(0.0, 0.9)))
            .top(relative(top.clamp(0.0, 0.95)))
            .mt(px(-16.0))
            .w(relative(span.clamp(0.1, 1.0)))
            .min_w(px(160.0))
            .child(gpui_kit::component::input::Input::new(&editing.input))
            .into_any_element(),
    )
}

/// The selection outline (the layer's real, possibly rotated rectangle), anchor point and
/// corner handles for each selected layer at the playhead.
fn selection_overlay(workspace: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let composition = workspace.state.composition();
    let frame = workspace.state.current_frame;
    let (width, height) = (composition.width as f32, composition.height as f32);
    let frac = |v: Vec2| (v.x / width, v.y / height);
    let mut elements = Vec::new();
    for &id in &workspace.state.selected_layers {
        let Some(layer) = composition.layer(id).filter(|layer| layer.is_active_at(frame)) else { continue };
        let Some(corners) = layer_corners(composition, id, frame as f64) else { continue };

        let outline = corners.map(frac);
        elements.push(
            canvas(|_, _, _| (), move |bounds, _, window, _| {
                let at = |(fx, fy): (f32, f32)| point(bounds.origin.x + bounds.size.width * fx, bounds.origin.y + bounds.size.height * fy);
                let mut path = PathBuilder::stroke(px(1.0));
                path.move_to(at(outline[0]));
                for corner in &outline[1..] {
                    path.line_to(at(*corner));
                }
                path.line_to(at(outline[0]));
                if let Ok(path) = path.build() {
                    window.paint_path(path, hsla(color::ACCENT));
                }
            })
            .absolute()
            .size_full()
            .into_any_element(),
        );

        // While its subject is being picked, clicks belong to the picture: no handles to catch them.
        if picking_layer(workspace) == Some(id) {
            continue;
        }
        let handles = [ScaleHandle::TopLeft, ScaleHandle::TopRight, ScaleHandle::BottomRight, ScaleHandle::BottomLeft];
        for (handle, corner) in handles.into_iter().zip(outline) {
            let (hx, hy) = corner;
            elements.push(
                div()
                    .id(ElementId::Name(format!("scale-handle:{}:{handle:?}", id.0).into()))
                    .absolute()
                    .left(relative(hx))
                    .top(relative(hy))
                    .ml(px(-8.0))
                    .mt(px(-8.0))
                    .size(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    // A 16 pt grab area around the visible 8 pt square.
                    .child(div().size(px(8.0)).bg(hsla(color::ACCENT)))
                    .cursor(if matches!(handle, ScaleHandle::TopLeft | ScaleHandle::BottomRight) { CursorStyle::ResizeUpLeftDownRight } else { CursorStyle::ResizeUpRightDownLeft })
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.ui.viewer_anchor = Some((f32::from(event.position.x), f32::from(event.position.y)));
                        this.state.begin_scale(id, handle);
                        cx.notify();
                    }))
                    .on_drag(ViewerDrag::Scale, |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .test_support()
                    .into_any_element(),
            );
        }

        // A pen path's own points, over the scale handles (a point often sits on a corner).
        elements.extend(path_points_overlay(workspace, id, cx));

        // The anchor point, where the layer scales and rotates around, drawn over the scale
        // handles so it can be dragged off a corner. Drag it to move it; the layer stays put.
        // While dragging, the points it snaps to are shown.
        if let (Some(transform), Some(world)) = (layer.transform.as_ref(), studio_core::transform::world_transform(composition, id, frame as f64)) {
            if workspace.state.anchor_drag_layer() == Some(id) && workspace.state.viewer_snapping {
                for (fx, fy) in crate::state::ANCHOR_POINTS {
                    let (sx, sy) = frac(world.apply(Vec2::new(layer.size.x * fx, layer.size.y * fy)));
                    elements.push(div().absolute().left(relative(sx)).top(relative(sy)).ml(px(-2.5)).mt(px(-2.5)).size(px(5.0)).rounded_full().bg(hsla(color::HIGHLIGHT)).into_any_element());
                }
            }
            let (ax, ay) = frac(world.apply(transform.anchor_point.value_at(frame as f64)));
            let line = |horizontal: bool| {
                let bar = div().absolute().bg(hsla(color::ACCENT));
                if horizontal { bar.left(px(-4.0)).right(px(-4.0)).top(px(5.5)).h(px(1.0)) } else { bar.top(px(-4.0)).bottom(px(-4.0)).left(px(5.5)).w(px(1.0)) }
            };
            elements.push(
                div()
                    .id(ElementId::Name(format!("anchor-handle:{}", id.0).into()))
                    .absolute()
                    .left(relative(ax))
                    .top(relative(ay))
                    .ml(px(-6.0))
                    .mt(px(-6.0))
                    .size(px(12.0))
                    .cursor(CursorStyle::Crosshair)
                    .child(line(true))
                    .child(line(false))
                    .child(div().absolute().left(px(1.0)).top(px(1.0)).size(px(10.0)).rounded_full().border_1().border_color(hsla(color::ACCENT)).bg(gpui_kit::rgba(0x00000066)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.ui.viewer_anchor = Some((f32::from(event.position.x), f32::from(event.position.y)));
                        // Snap within about 8 screen points of a corner, edge middle or center.
                        let reach = 8.0 / viewer_scale(this).max(f32::EPSILON);
                        this.state.begin_anchor_drag(id, reach);
                        cx.notify();
                    }))
                    .on_drag(ViewerDrag::Anchor, |_, _, _, cx| cx.new(|_| EmptyDragView))
                    .test_support()
                    .into_any_element(),
            );
        }

    }
    elements
}

pub fn render(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    // Render at display resolution when paused; at half resolution on high-density screens while playing.
    let scale = viewer_scale(workspace);
    let (comp_width, comp_height) = (workspace.state.composition().width as f32, workspace.state.composition().height as f32);
    let density = if workspace.state.is_playing() { workspace.ui.settings.playback_quality.density(window.scale_factor()) } else { window.scale_factor() };
    // Zoomed in past 100%, the frame is rendered at full size and shown larger, as in After
    // Effects: rendering more pixels than the composition has would add nothing but time.
    let render_scale = scale.min(1.0);
    let (pixels_wide, pixels_high) = ((comp_width * render_scale * density).round() as u32, (comp_height * render_scale * density).round() as u32);
    if pixels_wide > 0 && pixels_high > 0 {
        render_frame(workspace, pixels_wide, pixels_high, window, cx);
    }

    let composition = workspace.state.composition();
    let picture = workspace.render.image.clone().map(|image| img(ImageSource::Render(image)).absolute().size_full());
    let problem = match (&workspace.render.renderer, &workspace.render.last_error) {
        (Err(error), _) | (Ok(_), Some(error)) => Some(
            div().absolute().size_full().flex().items_center().justify_center().p_4().text_color(hsla(color::MUTED)).child(format!("Rendering is unavailable: {error}")),
        ),
        _ => None,
    };

    // A checkerboard under the picture shows where the composition is transparent.
    let (pan_x, pan_y) = workspace.ui.viewer_pan;
    let comp_frame = div()
        .relative()
        .flex_none()
        .left(px(pan_x))
        .top(px(pan_y))
        .w(px(composition.width as f32 * scale))
        .h(px(composition.height as f32 * scale))
        // Only the grid is clipped to the frame: selection handles may reach past its edges.
        .child(match workspace.ui.settings.transparency {
            crate::app_settings::Transparency::Checkerboard => div().absolute().size_full().overflow_hidden().child(gpui_kit::StyledImage::object_fit(img(transparency_grid()).size_full(), gpui_kit::ObjectFit::Cover)),
            solid => div().absolute().size_full().bg(gpui_kit::rgb(match solid {
                crate::app_settings::Transparency::Black => 0x000000,
                crate::app_settings::Transparency::White => 0xffffff,
                _ => 0x808080,
            })),
        })
        .child(workspace.ui.frame_bounds.probe())
        .children(picture)
        .children(problem)
        .children(workspace.render.showing_proxy.then(|| {
            div()
                .absolute()
                .bottom(px(6.0))
                .left(px(6.0))
                .px_1p5()
                .rounded(px(3.0))
                .bg(gpui_kit::rgba(0x000000aa))
                .text_size(px(size::SMALL_TEXT))
                .text_color(hsla(color::TEXT))
                .child("Proxy")
        }))
        // Says what the picture shows while a LUT is previewed from the Effects panel.
        .children(workspace.ui.lut_hover.as_ref().map(|lut| {
            div()
                .absolute()
                .top(px(6.0))
                .left(px(6.0))
                .px_1p5()
                .rounded(px(3.0))
                .bg(gpui_kit::rgba(0x000000aa))
                .text_size(px(size::SMALL_TEXT))
                .text_color(hsla(color::TEXT))
                .child(format!("Preview: {}", lut.name()))
        }))
        .children(selection_overlay(workspace, cx))
        .children(pen_draft_overlay(workspace))
        .children(cutout_clicks_overlay(workspace))
        .children(text_editor_overlay(workspace))
        .children(effect_drop_highlight(workspace));

    let viewer = div()
        .id("viewer")
        .flex_1()
        .min_h_0()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .bg(hsla(color::PASTEBOARD))
        .when(workspace.ui.tool == Tool::Type, |viewer| viewer.cursor_text())
        .when(matches!(workspace.ui.tool, Tool::Shape | Tool::Pen) || picking_layer(workspace).is_some(), |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Hand, |viewer| viewer.cursor_grab())
        .when(workspace.ui.track_pick.is_some(), |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.color_picker.as_ref().is_some_and(|picker| picker.sampling), |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Eyedropper || workspace.ui.white_balance_pick.is_some(), |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Zoom, |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Rotate, |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| press_viewer(this, event, window, cx)))
        .on_drag(ViewerDrag::Move, |_, _, _, cx| cx.new(|_| EmptyDragView))
        // An effect dragged over the picture lights up the layer it would go on (one it suits).
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<EffectDrag>, _, cx| {
            let position = event.event.position;
            let target = event.bounds.contains(&position).then(|| effect_target(this, &event.drag(cx).name, position)).flatten();
            if this.ui.effect_drop_layer != target {
                this.ui.effect_drop_layer = target;
                cx.notify();
            }
        }))
        // The target is found again where the mouse is released: the highlight is cleared by
        // the window's mouse-up handler, which runs before this.
        .on_drop(cx.listener(|this, drag: &EffectDrag, window, cx| {
            this.ui.effect_drop_layer = None;
            if let Some(layer) = effect_target(this, &drag.name, window.mouse_position()) {
                drag.apply(&mut this.state, layer);
                this.ui.tabs.insert(PanelId::Properties, 1);
            }
            cx.notify();
        }))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ViewerDrag>, _, cx| {
            let Some((x, y)) = this.ui.viewer_anchor else { return };
            if this.ui.tool == Tool::Hand {
                let position = event.event.position;
                let (start_x, start_y) = this.ui.pan_start;
                this.ui.viewer_pan = (start_x + f32::from(position.x) - x, start_y + f32::from(position.y) - y);
                cx.notify();
                return;
            }
            let scale = viewer_scale(this).max(f32::EPSILON);
            let position = event.event.position;
            let delta = Vec2::new((f32::from(position.x) - x) / scale, (f32::from(position.y) - y) / scale);
            this.state.drag_in_viewer(delta, event.event.modifiers.shift);
            cx.notify();
        }))
        .child(workspace.ui.viewer_bounds.probe())
        .child(comp_frame)
        .test_support();

    let title = format!("Composition: {}", composition.name);
    panel(
        PanelId::Composition,
        &[title.as_str()],
        Some(transport(workspace, scale, cx).into_any_element()),
        div().size_full().flex().flex_col().child(viewer),
        workspace,
        cx,
    )
}
