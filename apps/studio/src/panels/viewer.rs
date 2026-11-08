//! The Composition panel: the frame rendered by `studio-render` on the GPU, with the
//! selection outline, handles and anchor point drawn on top, and a bar of view settings.
//! Paused frames render at full display resolution; playback renders at half
//! resolution on high-density screens to keep up (After Effects' "Auto" resolution).

use gpui_kit::assets::IconName;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::menu::ContextMenuExt as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, CursorStyle, DragMoveEvent, ElementId, ImageSource, InteractiveElement, RenderImage, img,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _,
    Window, div,
    PathBuilder, canvas, point, prelude::FluentBuilder as _, px, relative,
};
use std::sync::Arc;

use studio_core::animation::Vec2;
use studio_core::model::Id;
use studio_core::transform::layer_corners;
use studio_media::{FrameServer, ProxyManager};
use studio_render::Renderer;

use super::chrome::{EmptyDragView, icon_button, panel};
use crate::state::ScaleHandle;
use crate::theme::{color, hsla, size};
use crate::workspace::{EditTarget, EffectDrag, PanelId, ProjectItemDrag, Tool, Workspace};

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
    /// Frames rendered ahead of the playhead in the background (see `preview_cache`).
    pub preview: crate::preview_cache::PreviewCache,
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
pub(crate) fn transparency_grid() -> Arc<RenderImage> {
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
            preview: crate::preview_cache::PreviewCache::new(),
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

    /// Where carets and selections go in `text` drawn with `style`, for typing in the viewer.
    pub fn text_geometry(&mut self, text: &str, style: &studio_core::model::TextStyle) -> Option<studio_render::text::TextGeometry> {
        Some(self.renderer.as_mut().ok()?.text_geometry(text, style))
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
    /// rendered `seconds` in, `width` × `height` pixels.
    pub fn template_picture(&mut self, template: &studio_core::templates::Template, format: studio_core::templates::Format, seconds: f64, width: u32, height: u32) -> Option<Arc<RenderImage>> {
        let (mut project, comp) = scratch_template(template, format)?;
        self.template_frame(&mut project, comp, (seconds * 30.0) as i64, width, height)
    }

    /// Frame `frame` of a template built with `scratch_template`, `width` × `height` pixels over
    /// a dark gray (so see-through templates still read). Text boxes are fitted to their measured
    /// words as they come into view, drawing a frame a second time only when one changed.
    pub fn template_frame(&mut self, project: &mut studio_core::model::Project, comp: studio_core::model::Id, frame: i64, width: u32, height: u32) -> Option<Arc<RenderImage>> {
        let renderer = self.renderer.as_mut().ok()?;
        let mut picture = renderer.render(project, comp, frame, width, height, &studio_core::media::NoVideo).ok()?;
        let composition = project.composition_mut(comp)?;
        let mut refitted = false;
        for (layer, size) in std::mem::take(&mut picture.text_sizes) {
            if composition.layer(layer).is_some_and(|layer| layer.size != size) {
                studio_core::edit::set_text_box(composition, layer, size);
                refitted = true;
            }
        }
        if refitted {
            picture = renderer.render(project, comp, frame, width, height, &studio_core::media::NoVideo).ok()?;
        }
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

/// `template` in `format`, built into a scratch project of its own (not the user's), for
/// drawing it on a Templates card. Returns the project and the template's composition.
pub fn scratch_template(template: &studio_core::templates::Template, format: studio_core::templates::Format) -> Option<(studio_core::model::Project, studio_core::model::Id)> {
    let mut project = studio_core::model::Project::new(Vec::new(), Vec::new());
    let comp = studio_core::templates::build(&mut project, template, format)?;
    Some((project, comp))
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

/// The project as the viewer shows it while a Keyer's key color is being picked: a copy with
/// that Keyer turned off, so the click lands on the screen's real color rather than on the
/// already keyed-out picture. `None` when no key color is being picked.
fn key_pick_preview(workspace: &Workspace) -> Option<studio_core::model::Project> {
    let (layer, effect) = workspace.ui.key_color_pick?;
    let mut project = workspace.state.project.clone();
    let keyer = project.composition_mut(workspace.state.active_comp)?.layer_mut(layer)?.effects.iter_mut().find(|candidate| candidate.id == effect)?;
    keyer.enabled = false;
    Some(project)
}

/// The project as the viewer shows it while the Selection or Matte view of a keyed layer is
/// chosen (see `studio_core::key_view`). `None` while the plain result is shown.
fn key_view_preview(workspace: &Workspace) -> Option<studio_core::model::Project> {
    let view = workspace.ui.key_view?;
    let layer = keyed_layer(workspace)?;
    studio_core::key_view::preview(&workspace.state.project, workspace.state.active_comp, layer, view)
}

/// What is shown while text is being typed into a text layer: `base` (the project, or a LUT
/// preview of it) with that layer's text swapped for what is in the field so far, so the viewer
/// draws it exactly as it will look, in the layer's own font, style and effects. Nothing is
/// written to the project (or its undo history) until the field is committed.
fn with_typed_text(workspace: &Workspace, base: Option<studio_core::model::Project>, cx: &gpui_kit::App) -> Option<studio_core::model::Project> {
    let Some(editing) = workspace.ui.editing.as_ref() else { return base };
    let EditTarget::SourceText(layer) = editing.target else { return base };
    let typed = editing.input.value(cx);
    if workspace.state.text_of(layer).is_some_and(|(text, _)| text == typed.as_ref()) {
        return base;
    }
    let mut project = base.unwrap_or_else(|| workspace.state.project.clone());
    if let Some(composition) = project.composition_mut(workspace.state.active_comp) {
        studio_core::edit::edit_text(composition, layer, |text, _| *text = typed.to_string());
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
    let previewed = with_typed_text(workspace, lut_preview(workspace).or_else(|| key_pick_preview(workspace)).or_else(|| key_view_preview(workspace)), cx);
    let state = &workspace.state;
    let shown = previewed.as_ref().unwrap_or(&state.project);
    // Rendering ahead: while paused, the background renderer is told what is on screen; while
    // playing, a frame it already made is shown instead of rendering one. (Not while a cut-out
    // is being made: its masks arrive from disk, which the project doesn't record.)
    let budget = if workspace.ui.cutout_job.is_none() { u64::from(workspace.ui.settings.render_ahead_mb) << 20 } else { 0 };
    if state.is_playing() {
        workspace.render.preview.follow(state.current_frame);
        if let Some(ahead) = workspace.render.preview.frame_for(shown, state.active_comp, state.current_frame) {
            // Its texture is let go of as the next frame replaces it, like any other.
            if let Some(previous) = workspace.render.image.replace(ahead.image.clone()) {
                cx.drop_image(previous, Some(window));
            }
            workspace.render.last_render_ms = Some(0.0);
            workspace.render.showing_proxy = false;
            // What is on screen came from the cache; a paused frame is drawn afresh.
            workspace.render.drawn = None;
            return;
        }
    } else {
        workspace.render.preview.show(shown, state.active_comp, width, height, state.current_frame, state.work_area, budget);
    }
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

/// The bar under the viewer (the panel's footer): playback quality, magnification, frame size,
/// snapping and guides. Playing and stepping live in the timeline's bar, so there is one set.
fn transport(workspace: &Workspace, scale: f32, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let composition = workspace.state.composition();
    // Drawn in the panel's footer, so it lines up with the Project panel's footer beside it.
    div()
        .size_full()
        .flex()
        .items_center()
        .gap_2()
        .px_1()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(quality_menu(workspace, cx))
        .child(zoom_menu(workspace, scale, cx))
        .child(format!("{} × {}", composition.width, composition.height))
        .child(icon_button("viewer-snapping", IconName::Magnet, "Snap anchor points to corners, edges and center", workspace.state.viewer_snapping, cx.listener(|this, _, _, cx| {
            this.state.viewer_snapping = !this.state.viewer_snapping;
            cx.notify();
        })))
        .when(workspace.has(crate::features::Feature::Guides), |bar| bar.child(super::guides::guides_menu(workspace, cx)))
        .children(key_view_switch(workspace, cx))
}

/// Viewer drags: moving the selection, or scaling from a corner handle.
#[derive(Clone, Copy, Debug)]
enum ViewerDrag {
    Move,
    Scale,
    Anchor,
    PathPoint,
    GradientPoint,
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
    if let Some(next) = next {
        zoom_to(workspace, at, next);
    }
}

/// Sets the magnification to `next` (kept within the zoom levels' range), keeping the
/// composition point `at` where it is on screen.
fn zoom_to(workspace: &mut Workspace, at: Vec2, next: f32) {
    let current = viewer_scale(workspace);
    let next = next.clamp(ZOOM_LEVELS[0], ZOOM_LEVELS[ZOOM_LEVELS.len() - 1]);
    if !next.is_finite() || (next - current).abs() < f32::EPSILON {
        return;
    }
    // The point stays put on screen: the frame's corner moves by how much farther (or nearer)
    // the point now is from it, less the half of the growth that centering already gives.
    let composition = workspace.state.composition();
    let half = Vec2::new(composition.width as f32 * 0.5, composition.height as f32 * 0.5);
    let shift = (at - half) * (current - next);
    let (x, y) = workspace.ui.viewer_pan;
    workspace.ui.viewer_pan = (x + shift.x, y + shift.y);
    workspace.ui.viewer_zoom = Some(next);
}

/// The topmost layer under a window position in the viewer, if any (for its right-click menu).
pub fn layer_under(workspace: &Workspace, position: gpui_kit::Point<gpui_kit::Pixels>) -> Option<Id> {
    let point = to_comp(workspace, f32::from(position.x), f32::from(position.y));
    workspace.state.layer_at(point)
}

/// Zooms the viewer one step (Cmd+= and Cmd+-), around the middle of what it shows.
pub fn zoom_step(workspace: &mut Workspace, zoom_in: bool) {
    let bounds = workspace.ui.viewer_bounds.get();
    let middle = to_comp(workspace, bounds.left + bounds.width * 0.5, bounds.top + bounds.height * 0.5);
    zoom_at(workspace, middle, zoom_in);
}

/// How strongly the scroll wheel zooms the viewer, per point scrolled.
const WHEEL_ZOOM_SENSITIVITY: f32 = 0.004;

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

/// The layer the viewer's key views are about: the one whose subject is being picked, or else
/// the only selected layer. Either way it must be keyed (have an Object Matte or a key).
fn keyed_layer(workspace: &Workspace) -> Option<Id> {
    let single = match workspace.state.selected_layers.as_slice() {
        [layer] => Some(*layer),
        _ => None,
    };
    let composition = workspace.state.composition();
    picking_layer(workspace).or(single).filter(|layer| composition.layer(*layer).is_some_and(studio_core::key_view::is_keyed))
}

/// A brush stroke being painted on an object matte: the layer, whether it adds to the object,
/// and the points so far (composition pixels).
#[derive(Clone, Debug)]
pub struct MattePaint {
    layer: Id,
    add: bool,
    points: Vec<Vec2>,
}

/// The brush radius the object matte starts with, and its limits, in screen points.
pub const DEFAULT_BRUSH: f32 = 18.0;
const BRUSH_RANGE: (f32, f32) = (3.0, 200.0);

/// Starts a stroke on the painted layer at `point` (Option-drag takes away from the object).
/// A stroke left over from a press released outside the viewer is finished first.
fn start_matte_stroke(workspace: &mut Workspace, layer: Id, point: Vec2, add: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    finish_matte_stroke(workspace, window, cx);
    workspace.ui.matte_paint = Some(MattePaint { layer, add, points: vec![point] });
}

/// Adds the stroke being painted to the layer's matte, and remakes the matte for this frame.
pub(crate) fn finish_matte_stroke(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(paint) = workspace.ui.matte_paint.take() else { return };
    let radius = workspace.ui.matte_brush / viewer_scale(workspace).max(f32::EPSILON);
    if workspace.state.add_matte_stroke(paint.layer, &paint.points, radius, paint.add) {
        super::cutout::make_masks(workspace, paint.layer, false, window, cx);
    }
}

/// Makes the brush bigger (`grow`) or smaller by one step.
fn resize_brush(workspace: &mut Workspace, grow: bool) {
    let factor = if grow { 1.25 } else { 0.8 };
    workspace.ui.matte_brush = (workspace.ui.matte_brush * factor).clamp(BRUSH_RANGE.0, BRUSH_RANGE.1).round();
}

/// The strokes painted on this frame (green on the object, red where it isn't), the one being
/// painted, and the brush outline under the pointer, drawn over the picture.
fn matte_strokes_overlay(workspace: &Workspace) -> Option<AnyElement> {
    let layer = picking_layer(workspace)?;
    let composition = workspace.state.composition();
    let comp_size = Vec2::new(composition.width as f32, composition.height as f32);
    let scale = viewer_scale(workspace).max(f32::EPSILON);
    let mut strokes = workspace.state.matte_strokes_on_screen(layer);
    let brush = workspace.ui.matte_brush / scale;
    if let Some(paint) = workspace.ui.matte_paint.as_ref().filter(|paint| paint.layer == layer) {
        strokes.push((paint.points.clone(), brush, paint.add));
    }
    let pointer = workspace.ui.matte_pointer.map(|at| (at, brush));
    Some(
        gpui_kit::canvas(|_, _, _| (), move |bounds, _, window, _| {
            let place = |at: Vec2| {
                gpui_kit::point(
                    bounds.origin.x + bounds.size.width * (at.x / comp_size.x),
                    bounds.origin.y + bounds.size.height * (at.y / comp_size.y),
                )
            };
            let to_screen = f32::from(bounds.size.width) / comp_size.x;
            for (points, radius, add) in &strokes {
                // Each stroke is filled as round dabs along its path, all in one shape, so the
                // translucent tint stays even where the dabs overlap.
                let r = (radius * to_screen).max(1.0);
                let tint = gpui_kit::rgba(if *add { (KEEP_COLOR << 8) | 0x80 } else { (LEAVE_OUT_COLOR << 8) | 0x80 });
                let mut shape = gpui_kit::PathBuilder::fill();
                let mut dab = |center: gpui_kit::Point<gpui_kit::Pixels>| {
                    let r = px(r);
                    shape.move_to(gpui_kit::point(center.x + r, center.y));
                    shape.arc_to(gpui_kit::point(r, r), px(0.0), false, true, gpui_kit::point(center.x - r, center.y));
                    shape.arc_to(gpui_kit::point(r, r), px(0.0), false, true, gpui_kit::point(center.x + r, center.y));
                    shape.close();
                };
                let mut previous = place(points[0]);
                dab(previous);
                for at in &points[1..] {
                    let next = place(*at);
                    let (dx, dy) = (f32::from(next.x - previous.x), f32::from(next.y - previous.y));
                    let steps = ((dx * dx + dy * dy).sqrt() / (r * 0.5)).ceil().max(1.0) as usize;
                    for step in 1..=steps {
                        let t = step as f32 / steps as f32;
                        dab(gpui_kit::point(previous.x + px(dx * t), previous.y + px(dy * t)));
                    }
                    previous = next;
                }
                if let Ok(shape) = shape.build() {
                    window.paint_path(shape, tint);
                }
            }
            if let Some((at, radius)) = pointer {
                let center = place(at);
                let r = px(radius * to_screen);
                let mut ring = gpui_kit::PathBuilder::stroke(px(1.5));
                ring.move_to(gpui_kit::point(center.x + r, center.y));
                ring.arc_to(gpui_kit::point(r, r), px(0.0), false, true, gpui_kit::point(center.x - r, center.y));
                ring.arc_to(gpui_kit::point(r, r), px(0.0), false, true, gpui_kit::point(center.x + r, center.y));
                if let Ok(ring) = ring.build() {
                    window.paint_path(ring, gpui_kit::rgb(0xffffff));
                }
            }
        })
        .absolute()
        .size_full()
        .into_any_element(),
    )
}

/// Green for strokes on the object, red for strokes on what isn't part of it.
const KEEP_COLOR: u32 = 0x22c55e;
const LEAVE_OUT_COLOR: u32 = 0xef4444;

/// A small dark label floating over the picture.
fn viewer_chip() -> gpui_kit::Div {
    div()
        .absolute()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(px(4.0))
        .bg(gpui_kit::rgba(0x000000b8))
        .text_size(px(size::SMALL_TEXT))
        .text_color(gpui_kit::rgb(0xffffff))
}

/// While an object matte is being painted: how to paint, the brush size, how many strokes this
/// frame has and whether the matte is being made, with a Done button that stops painting.
fn picking_hint(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    use gpui_kit::component::Sizable as _;
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let layer = picking_layer(workspace)?;
    let strokes = workspace.state.matte_strokes_on_screen(layer).len();
    let marker = |add: bool| div().size(px(10.0)).flex_none().rounded_full().bg(gpui_kit::rgb(if add { KEEP_COLOR } else { LEAVE_OUT_COLOR }));
    let finding = workspace.ui.cutout_job.as_ref().is_some_and(|job| job.layer == layer);
    let progress = if finding {
        "Making the matte…".to_owned()
    } else {
        match strokes {
            0 => "No strokes on this frame".to_owned(),
            1 => "1 stroke here".to_owned(),
            count => format!("{count} strokes here"),
        }
    };
    let brush_button = |id: &'static str, label: &'static str, grow: bool, cx: &mut Context<Workspace>| {
        Button::new(id).ghost().xsmall().label(label).tooltip(if grow { "Bigger brush" } else { "Smaller brush" }).on_click(cx.listener(move |this, _, _, cx| {
            resize_brush(this, grow);
            cx.notify();
        }))
    };
    Some(
        viewer_chip()
            .id("viewer-picking-hint")
            .test_support()
            .bottom(px(10.0))
            .left_0()
            .right_0()
            .mx_auto()
            .w(px(560.0))
            .child(marker(true))
            .child("Paint the object")
            .child(marker(false))
            .child("Option-paint to take away")
            .child(div().flex_1())
            .child(brush_button("viewer-brush-smaller", "\u{2212}", false, cx))
            .child(div().text_color(gpui_kit::rgba(0xffffffaa)).child(format!("Brush {:.0}", workspace.ui.matte_brush)))
            .child(brush_button("viewer-brush-bigger", "+", true, cx))
            .child(div().text_color(gpui_kit::rgba(0xffffffaa)).child(progress))
            .child(Button::new("viewer-picking-done").primary().xsmall().label("Done").on_click(cx.listener(move |this, _, _, cx| {
                super::cutout::toggle_picking(this, layer);
                cx.notify();
            })))
            .into_any_element(),
    )
}

/// Says what the picture shows while a key view is on, so gray or black areas aren't mistaken
/// for the real result.
fn key_view_legend(workspace: &Workspace) -> Option<AnyElement> {
    use studio_core::key_view::KeyView;
    // A LUT or key color preview replaces the key view, so its legend would be wrong then.
    if workspace.ui.lut_hover.is_some() || workspace.ui.key_color_pick.is_some() {
        return None;
    }
    let view = workspace.ui.key_view?;
    keyed_layer(workspace)?;
    let swatch = |rgb: u32| div().size(px(10.0)).flex_none().rounded(px(2.0)).border_1().border_color(gpui_kit::rgba(0xffffff88)).bg(gpui_kit::rgb(rgb));
    let (kept, removed): ((u32, &str), (u32, &str)) = match view {
        KeyView::Selection => ((KEEP_COLOR, "Full color is kept"), (0x3a3a3a, "gray is removed")),
        KeyView::Matte => ((0xffffff, "White is kept"), (0x000000, "black is removed")),
    };
    Some(
        viewer_chip()
            .id("viewer-key-legend")
            .test_support()
            .top(px(6.0))
            .left(px(6.0))
            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(view.name()))
            .child(swatch(kept.0))
            .child(kept.1)
            .child(swatch(removed.0))
            .child(removed.1)
            .into_any_element(),
    )
}

/// The footer's choice of how to show a keyed layer: its result, what the key selects, or its
/// matte. Shown only while a keyed layer is selected.
fn key_view_switch(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    use gpui_kit::component::{Selectable as _, Sizable as _};
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use studio_core::key_view::KeyView;
    keyed_layer(workspace)?;
    let current = workspace.ui.key_view;
    let choices: [(&str, &str, Option<KeyView>, &str); 3] = [
        ("viewer-key-result", "Result", None, "Show the finished picture"),
        ("viewer-key-selection", "Selection", Some(KeyView::Selection), "Show what the key keeps in color and what it removes in gray"),
        ("viewer-key-matte", "Matte", Some(KeyView::Matte), "Show the key in black and white: white is kept, black is removed"),
    ];
    Some(choices.into_iter().fold(div().flex().items_center().gap_0p5().pl_1().border_l_1().border_color(hsla(color::BORDER)), |row, (id, label, view, tip)| {
        row.child(Button::new(id).ghost().xsmall().label(label).tooltip(tip).selected(current == view).on_click(cx.listener(move |this, _, _, cx| {
            this.ui.key_view = view;
            cx.notify();
        })))
    }))
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

/// A selected shape's gradient: a line from where it starts to where it ends, with a handle
/// on each end to drag (a ring at the start, a dot at the end, as in design tools).
fn gradient_overlay(workspace: &Workspace, layer: Id, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    use crate::state::GradientEnd;
    let Some(shown) = workspace.state.gradient_on_screen(layer) else { return Vec::new() };
    let composition = workspace.state.composition();
    let size = Vec2::new(composition.width as f32, composition.height as f32);
    let frac = |v: Vec2| (v.x / size.x, v.y / size.y);
    let mut elements = vec![curve_canvas(vec![[shown.from, shown.from, shown.to, shown.to]], size, hsla(color::ACCENT))];
    for (end, at, name) in [(GradientEnd::From, shown.from, "from"), (GradientEnd::To, shown.to, "to")] {
        let at = frac(at);
        let dot = div()
            .size(px(12.0))
            .rounded_full()
            .border_2()
            .border_color(hsla(color::ACCENT))
            .when(end == GradientEnd::To, |dot| dot.bg(hsla(color::ACCENT)))
            .when(end == GradientEnd::From, |dot| dot.bg(hsla(color::PANEL)));
        elements.push(
            div()
                .id(ElementId::Name(format!("gradient-{name}:{}", layer.0).into()))
                .absolute()
                .left(relative(at.0))
                .top(relative(at.1))
                .ml(px(-8.0))
                .mt(px(-8.0))
                .size(px(16.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor(CursorStyle::Crosshair)
                .child(dot)
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.ui.viewer_anchor = Some((f32::from(event.position.x), f32::from(event.position.y)));
                    this.state.begin_gradient_drag(layer, end);
                    cx.notify();
                }))
                .on_drag(ViewerDrag::GradientPoint, |_, _, _, cx| cx.new(|_| EmptyDragView))
                .test_support()
                .into_any_element(),
        );
    }
    elements
}

/// Converts a window position to composition pixels, using where the frame was drawn last frame.
fn to_comp(workspace: &Workspace, x: f32, y: f32) -> Vec2 {
    let frame = workspace.ui.frame_bounds.get();
    let scale = viewer_scale(workspace).max(f32::EPSILON);
    Vec2::new((x - frame.left) / scale, (y - frame.top) / scale)
}

/// Where a drop on the viewer lands, in composition pixels.
fn drop_point(workspace: &Workspace, window: &Window) -> Vec2 {
    let position = window.mouse_position();
    to_comp(workspace, f32::from(position.x), f32::from(position.y))
}

/// Press in the viewer: the Type tool places a text layer; the Shape tool starts drawing a
/// shape; the Selection tool picks the layer under the pointer (Shift adds to the selection)
/// and gets ready to move it.
fn press_viewer(workspace: &mut Workspace, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Workspace>) {
    let (x, y) = (f32::from(event.position.x), f32::from(event.position.y));
    let point = to_comp(workspace, x, y);
    workspace.ui.viewer_anchor = Some((x, y));
    workspace.ui.text_select = None;
    // While typing into a text layer here, a press on that text moves the caret (a double-click
    // selects a word) and a drag selects, as in any text box; a press anywhere else finishes.
    if let Some(layer) = typing_layer(workspace) {
        match typed_caret_at(workspace, layer, point, true, cx) {
            Some(offset) => {
                let Some(editing) = workspace.ui.editing.as_ref() else { return };
                let range = if event.click_count >= 2 { word_around(&editing.input.value(cx), offset) } else { offset..offset };
                editing.input.set_selected_range(range, cx);
                editing.input.focus(window, cx);
                workspace.ui.text_select = (event.click_count < 2).then_some((layer, offset));
                cx.notify();
                return;
            }
            None => workspace.commit_edit(window, cx),
        }
    }
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
    // Picking a Keyer's key color: the color clicked (in the picture before the key) is what
    // it removes.
    if let Some((layer, effect)) = workspace.ui.key_color_pick.take() {
        match color_at(workspace, point).filter(|color| color.a > 0.0) {
            Some(color) => workspace.state.set_key_color(layer, effect, color),
            None => window.push_notification("Click on the green or blue screen in the picture.", cx),
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
    // Painting an object matte: a press starts a stroke on what belongs to the object (Option:
    // on what doesn't); it is added when the button comes up.
    if let Some(layer) = picking_layer(workspace) {
        start_matte_stroke(workspace, layer, point, !event.modifiers.alt, window, cx);
        cx.notify();
        return;
    }
    if workspace.ui.tool == Tool::Type {
        // On a text layer, the Type tool edits it, with the caret where it was clicked.
        if let Some((layer, caret)) = text_hit(workspace, point) {
            let text = workspace.state.text_of(layer).map(|(text, _)| text.to_owned()).unwrap_or_default();
            workspace.state.select_layer(layer, false);
            cx.defer_in(window, move |this, window, cx| {
                this.begin_edit(EditTarget::SourceText(layer), text, window, cx);
                if let Some(editing) = this.ui.editing.as_mut() {
                    editing.in_viewer = true;
                    editing.input.set_selected_range(caret..caret, cx);
                }
                cx.notify();
            });
            return;
        }
        if let Some(layer) = workspace.state.add_text_layer("", point) {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
            // Open the text field once this click finishes; focusing it during mouse-down would
            // lose focus to the window as the click completes, closing the field straight away.
            // It opens right where the text goes, so typing never depends on the Properties
            // panel's Text card being unfolded.
            cx.defer_in(window, move |this, window, cx| {
                this.begin_edit(EditTarget::SourceText(layer), String::new(), window, cx);
                if let Some(editing) = this.ui.editing.as_mut() {
                    editing.in_viewer = true;
                }
                cx.notify();
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
    let corners = layer_corners(workspace.state.composition(), layer, workspace.state.current_frame as f64)?;
    Some(drop_outline(workspace, corners))
}

/// A bright outline around the media slot a photo or video being dragged would fill.
fn slot_drop_highlight(workspace: &Workspace) -> Option<AnyElement> {
    workspace.ui.slot_drop.map(|corners| drop_outline(workspace, corners))
}

/// Outlines `corners` (composition pixels) of a drop target, drawn over the picture.
fn drop_outline(workspace: &Workspace, corners: [Vec2; 4]) -> AnyElement {
    let composition = workspace.state.composition();
    let (width, height) = (composition.width as f32, composition.height as f32);
    let outline = corners.map(|v| (v.x / width, v.y / height));
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
    .into_any_element()
}

/// Outlines the media slot under `position` (window coordinates) while something that could
/// fill it is dragged over the viewer (`over`), and clears the outline otherwise.
fn show_slot_under(workspace: &mut Workspace, over: bool, position: gpui_kit::Point<gpui_kit::Pixels>, cx: &mut Context<Workspace>) {
    let outline = over
        .then(|| workspace.state.slot_at(to_comp(workspace, f32::from(position.x), f32::from(position.y))))
        .flatten()
        .map(|slot| slot.outline);
    if workspace.ui.slot_drop != outline {
        workspace.ui.slot_drop = outline;
        cx.notify();
    }
}

/// The layer an effect dropped at `position` (window coordinates) would go on: the topmost one
/// under the pointer that the effect suits.
fn effect_target(workspace: &Workspace, effect: &str, position: gpui_kit::Point<gpui_kit::Pixels>) -> Option<Id> {
    let point = to_comp(workspace, f32::from(position.x), f32::from(position.y));
    let layer = workspace.state.layer_at(point)?;
    let content = &workspace.state.composition().layer(layer)?.content;
    studio_core::effects::suits_layer(effect, content).then_some(layer)
}

/// The text layer being typed into in the viewer, if any.
fn typing_layer(workspace: &Workspace) -> Option<Id> {
    match workspace.ui.editing.as_ref().filter(|editing| editing.in_viewer)?.target {
        EditTarget::SourceText(layer) => Some(layer),
        _ => None,
    }
}

/// Where in the text being typed into `layer` composition point `point` falls, as a byte offset.
/// With `inside`, only a point on the text's box (or just around it) counts.
fn typed_caret_at(workspace: &mut Workspace, layer: Id, point: Vec2, inside: bool, cx: &gpui_kit::App) -> Option<usize> {
    /// How far outside the text's box, in its own pixels, a press still counts as on it.
    const MARGIN: f32 = 12.0;
    let typed = workspace.ui.editing.as_ref()?.input.value(cx).to_string();
    let style = workspace.state.text_of(layer)?.1.clone();
    let composition = workspace.state.composition();
    let size = composition.layer(layer)?.size;
    let local = studio_core::transform::world_transform(composition, layer, workspace.state.current_frame as f64)?.inverse()?.apply(point);
    let on_box = (-MARGIN..=size.x + MARGIN).contains(&local.x) && (-MARGIN..=size.y + MARGIN).contains(&local.y);
    if inside && !on_box {
        return None;
    }
    Some(workspace.render.text_geometry(&typed, &style)?.hit(local.x, local.y))
}

/// The word in `text` around byte `offset`: the run of letters (or of spaces) it touches.
fn word_around(text: &str, offset: usize) -> std::ops::Range<usize> {
    let offset = offset.min(text.len());
    let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '_';
    // The kind of character at the offset (or just before it, at the end of a word).
    let kind = text[offset..].chars().next().filter(|&c| is_word(c)).or_else(|| text[..offset].chars().next_back()).is_some_and(is_word);
    let start = text[..offset].char_indices().rev().take_while(|&(_, c)| is_word(c) == kind && c != '\n').last().map_or(offset, |(at, _)| at);
    let end = text[offset..].char_indices().find(|&(_, c)| is_word(c) != kind || c == '\n').map_or(text.len(), |(at, _)| offset + at);
    start..end
}

/// The unlocked text layer under composition point `point`, and the place in its text nearest
/// that point (a byte offset, for the caret).
fn text_hit(workspace: &mut Workspace, point: Vec2) -> Option<(Id, usize)> {
    let layer = workspace.state.layer_at(point)?;
    let composition = workspace.state.composition();
    if composition.layer(layer)?.switches.locked {
        return None;
    }
    let (text, style) = workspace.state.text_of(layer).map(|(text, style)| (text.to_owned(), style.clone()))?;
    let local = studio_core::transform::world_transform(composition, layer, workspace.state.current_frame as f64)?.inverse()?.apply(point);
    let geometry = workspace.render.text_geometry(&text, &style)?;
    Some((layer, geometry.hit(local.x, local.y)))
}

/// Typing into a text layer in the viewer (with the Type tool, or after a double-click), as in
/// other design apps: the picture shows the text exactly as it will look (see
/// `with_typed_text`), with the caret and selection drawn on it, following the layer's
/// position, scale and rotation. The field that takes the keystrokes is there but unseen.
/// Enter or clicking elsewhere commits it.
fn text_editor_overlay(workspace: &mut Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    /// How wide the caret is drawn, in screen pixels.
    const CARET_WIDTH: f32 = 2.0;
    let editing = workspace.ui.editing.as_ref().filter(|editing| editing.in_viewer)?;
    let EditTarget::SourceText(layer) = editing.target else { return None };
    let (typed, selected) = (editing.input.value(cx).to_string(), editing.input.selected_range(cx));
    let field = editing.input.element();
    let style = workspace.state.text_of(layer)?.1.clone();
    let geometry = workspace.render.text_geometry(&typed, &style)?;
    let composition = workspace.state.composition();
    let world = studio_core::transform::world_transform(composition, layer, workspace.state.current_frame as f64)?;
    let (width, height) = (composition.width as f32, composition.height as f32);
    // A point in the layer's own pixels, as a fraction of the frame.
    let frac = move |x: f32, y: f32| {
        let at = world.apply(Vec2::new(x, y));
        (at.x / width, at.y / height)
    };
    let selection: Vec<[(f32, f32); 4]> =
        geometry.highlight(selected.clone()).into_iter().map(|(x, y, w, h)| [frac(x, y), frac(x + w, y), frac(x + w, y + h), frac(x, y + h)]).collect();
    let caret_line = {
        let (x, top, line) = geometry.caret(selected.end);
        (frac(x, top), frac(x, top + line))
    };
    // The caret blinks once typing pauses (see `UiState::caret_epoch`).
    let caret_on = (workspace.ui.caret_epoch.elapsed().as_millis() / crate::workspace::CARET_BLINK.as_millis()).is_multiple_of(2);
    let caret = (selected.is_empty() && caret_on).then_some(caret_line);
    let anchor = caret_line.0;
    let marks = canvas(|_, _, _| (), move |bounds, _, window, _| {
        let at = |(fx, fy): (f32, f32)| point(bounds.origin.x + bounds.size.width * fx, bounds.origin.y + bounds.size.height * fy);
        for quad in &selection {
            let mut path = PathBuilder::fill();
            path.move_to(at(quad[0]));
            for corner in &quad[1..] {
                path.line_to(at(*corner));
            }
            path.close();
            if let Ok(path) = path.build() {
                window.paint_path(path, hsla(color::ACCENT).opacity(0.4));
            }
        }
        if let Some((top, bottom)) = caret {
            let mut path = PathBuilder::stroke(px(CARET_WIDTH));
            path.move_to(at(top));
            path.line_to(at(bottom));
            if let Ok(path) = path.build() {
                window.paint_path(path, hsla(color::ACCENT));
            }
        }
    })
    .absolute()
    .size_full();
    // The field itself: unseen, and too small to catch clicks, at the caret (where the system
    // shows its accent and emoji pickers). Escape finishes, as in After Effects.
    let field = div()
        .absolute()
        .left(relative(anchor.0.clamp(0.0, 1.0)))
        .top(relative(anchor.1.clamp(0.0, 1.0)))
        .size(px(1.0))
        .overflow_hidden()
        .opacity(0.0)
        .on_key_down(cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
            if event.keystroke.key == "escape" {
                this.commit_edit(window, cx);
            }
        }))
        .child(field);
    Some(div().absolute().size_full().child(marks).child(field).into_any_element())
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

        // While its subject or a key color is being picked, clicks belong to the picture: no
        // handles to catch them.
        if picking_layer(workspace) == Some(id) || workspace.ui.key_color_pick.is_some() {
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
        // A gradient's line and its two ends, to drag where it runs.
        elements.extend(gradient_overlay(workspace, id, cx));

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

    // Drawn after the frame, which fits the text layer's box to what is being typed.
    let typing = text_editor_overlay(workspace, cx);
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
        // A faint line just outside the frame, so its edge shows even when the picture is as
        // dark as the space around it (an empty or black composition).
        .child(div().absolute().top(px(-1.0)).left(px(-1.0)).right(px(-1.0)).bottom(px(-1.0)).border_1().border_color(hsla(color::MUTED).opacity(0.35)))
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
        .children((workspace.ui.guides && workspace.has(crate::features::Feature::Guides)).then(|| super::guides::guides_overlay(workspace)))
        .children(selection_overlay(workspace, cx))
        .children(pen_draft_overlay(workspace))
        .children(matte_strokes_overlay(workspace))
        .children(key_view_legend(workspace))
        .children(picking_hint(workspace, cx))
        .children(typing)
        .children(effect_drop_highlight(workspace))
        .children(slot_drop_highlight(workspace));

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
        .when(workspace.ui.tool == Tool::Eyedropper || workspace.ui.white_balance_pick.is_some() || workspace.ui.key_color_pick.is_some(), |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Zoom, |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .when(workspace.ui.tool == Tool::Rotate, |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
        .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, window, cx| press_viewer(this, event, window, cx)))
        // Holding the middle mouse button (the scroll wheel) pans, as the Hand tool does.
        .when(workspace.ui.middle_pan.is_some(), |viewer| viewer.cursor(gpui_kit::CursorStyle::ClosedHand))
        .on_mouse_down(MouseButton::Middle, cx.listener(|this, event: &MouseDownEvent, _, cx| {
            this.ui.middle_pan = Some(((f32::from(event.position.x), f32::from(event.position.y)), this.ui.viewer_pan));
            cx.notify();
        }))
        // The brush outline follows the pointer while an object matte is being painted.
        .when(picking_layer(workspace).is_some(), |viewer| {
            viewer.cursor(gpui_kit::CursorStyle::Crosshair).on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                this.ui.matte_pointer = Some(to_comp(this, f32::from(event.position.x), f32::from(event.position.y)));
                cx.notify();
            }))
        })
        // A stroke ends when the button comes up (a press without a drag paints a dot).
        .on_mouse_up(MouseButton::Left, cx.listener(|this, _, window, cx| {
            if this.ui.matte_paint.is_some() {
                finish_matte_stroke(this, window, cx);
                cx.notify();
            }
        }))
        .on_drop(cx.listener(|this, _: &ViewerDrag, window, cx| {
            if this.ui.matte_paint.is_some() {
                finish_matte_stroke(this, window, cx);
                cx.notify();
            }
        }))
        .on_mouse_move(cx.listener(|this, event: &gpui_kit::MouseMoveEvent, _, cx| {
            let Some(((x, y), (pan_x, pan_y))) = this.ui.middle_pan else { return };
            if event.pressed_button != Some(MouseButton::Middle) {
                // Released outside the viewer: the pan is over.
                this.ui.middle_pan = None;
                cx.notify();
                return;
            }
            this.ui.viewer_pan = (pan_x + f32::from(event.position.x) - x, pan_y + f32::from(event.position.y) - y);
            cx.notify();
        }))
        .on_mouse_up(MouseButton::Middle, cx.listener(|this, _, _, cx| {
            this.ui.middle_pan = None;
            cx.notify();
        }))
        // The scroll wheel (or two fingers on a trackpad) zooms smoothly around the pointer.
        .on_scroll(cx.listener(|this, event: &gpui_kit::ScrollEvent, _, cx| {
            let delta = f32::from(event.delta.pixel_delta(px(16.0)).y);
            if delta == 0.0 {
                return;
            }
            let at = to_comp(this, f32::from(event.position.x), f32::from(event.position.y));
            let next = viewer_scale(this) * (delta * WHEEL_ZOOM_SENSITIVITY).exp();
            zoom_to(this, at, next);
            cx.stop_propagation();
            cx.notify();
        }))
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
        // A photo or video dragged over a media slot (a "Drop your … here" placeholder, or a
        // nested composition showing one photo or video) outlines the slot it would fill.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ProjectItemDrag>, _, cx| {
            let fills = studio_core::slots::can_fill_with(&this.state.project, event.drag(cx).item);
            show_slot_under(this, event.bounds.contains(&event.event.position) && fills, event.event.position, cx);
        }))
        // Files from the Finder aren't known to be pictures until they're imported, so any file
        // outlines the slot; a file that isn't a photo or video is added as a layer as usual.
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<gpui_kit::ExternalPaths>, _, cx| {
            show_slot_under(this, event.bounds.contains(&event.event.position), event.event.position, cx);
        }))
        // A Media item dropped on the picture fills the slot under it, or else becomes a layer
        // from the playhead, centered where it was let go (see `EditorState::drop_items_at`).
        .drag_over::<ProjectItemDrag>(|style, _, _, _| style.bg(hsla(color::HOVER)))
        .on_drop(cx.listener(|this, drag: &ProjectItemDrag, window, cx| {
            this.ui.slot_drop = None;
            let at = drop_point(this, window);
            this.state.drop_items_at(&[drag.item], at);
            cx.notify();
        }))
        // Files from the Finder are imported first, then dropped the same way, one after another.
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
        .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
            this.ui.slot_drop = None;
            let at = drop_point(this, window);
            let place: super::importing::Then = Box::new(move |this, items, _, cx| {
                this.state.drop_items_at(&items, at);
                cx.notify();
            });
            this.import_paths(paths.paths().to_vec(), Some(place), window, cx);
            cx.notify();
        }))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ViewerDrag>, _, cx| {
            // Painting an object matte: the stroke follows the pointer.
            if this.ui.matte_paint.is_some() {
                let point = to_comp(this, f32::from(event.event.position.x), f32::from(event.event.position.y));
                this.ui.matte_pointer = Some(point);
                // Points closer than a screen point add nothing to the stroke.
                let spacing = 1.0 / viewer_scale(this).max(f32::EPSILON);
                if let Some(paint) = this.ui.matte_paint.as_mut()
                    && paint.points.last().is_none_or(|last| (*last - point).length() >= spacing)
                {
                    paint.points.push(point);
                }
                cx.notify();
                return;
            }
            // Dragging across text being typed selects it.
            if let Some((layer, anchor)) = this.ui.text_select {
                let point = to_comp(this, f32::from(event.event.position.x), f32::from(event.event.position.y));
                if let (Some(offset), Some(editing)) = (typed_caret_at(this, layer, point, false, cx), this.ui.editing.as_ref()) {
                    editing.input.set_selected_range(anchor.min(offset)..anchor.max(offset), cx);
                }
                return;
            }
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
        .test_support()
        // Right-clicking a layer opens its menu (see `layer_menu`).
        .context_menu(super::layer_menu::build(None, cx.weak_entity()));

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

#[cfg(test)]
mod tests {
    use super::word_around;

    #[test]
    fn a_double_click_selects_the_word_it_lands_on() {
        let text = "Big sale\ntoday";
        assert_eq!(word_around(text, 1), 0..3, "inside a word");
        assert_eq!(word_around(text, 3), 0..3, "at its end");
        assert_eq!(word_around(text, 4), 4..8, "at the next one's start");
        assert_eq!(word_around(text, 10), 9..14, "words stop at line breaks");
        assert_eq!(word_around("a   b", 2), 1..4, "between words, the spaces");
        assert_eq!(word_around("", 0), 0..0);
    }
}
