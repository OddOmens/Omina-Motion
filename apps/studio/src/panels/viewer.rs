//! The Composition panel: the frame rendered by `studio-render` on the GPU, with the
//! selection outline, handles and anchor point drawn on top, and the transport bar.
//! Paused frames render at full display resolution; playback renders at half
//! resolution on high-density screens to keep up (After Effects' "Auto" resolution).

use gpui_kit::assets::IconName;
use gpui_kit::{
    AnyElement, AppContext as _, Context, CursorStyle, DragMoveEvent, ElementId, ImageSource, InteractiveElement, RenderImage, img,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _,
    Window, div,
    PathBuilder, canvas, point, prelude::FluentBuilder as _, px, relative, rgba,
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
}

/// Footage is decoded at most this wide for the viewer.
const PREVIEW_DECODE_WIDTH: u32 = 1920;

impl ViewerRender {
    pub fn new() -> Self {
        // Headless tests keep their proxies apart, so clearing the cache in a test never touches real ones.
        let folder = if cfg!(test) { std::env::temp_dir().join(format!("studio-proxies-test-{}", std::process::id())) } else { studio_media::default_cache_folder() };
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
        }
    }
}

/// Renders the frame for the viewer at `width` × `height` device pixels and swaps it on screen.
fn render_frame(workspace: &mut Workspace, width: u32, height: u32, window: &mut Window, cx: &mut Context<Workspace>) {
    let Ok(renderer) = workspace.render.renderer.as_mut() else { return };
    let state = &workspace.state;
    // While playing, decoders work ahead of the playhead.
    workspace.render.media.set_playing(state.is_playing());
    // Let go of footage the open composition no longer uses (deleted layers, other
    // compositions), so its decoder threads stop and its frames are freed.
    let footage: std::collections::HashSet<&std::path::Path> = state
        .composition()
        .layers
        .iter()
        .filter_map(|layer| match layer.content {
            studio_core::model::LayerContent::Footage { item } => state.project.item(item)?.path.as_deref(),
            _ => None,
        })
        .collect();
    workspace.render.media.release_unused(|path| footage.contains(path));
    let started = std::time::Instant::now();
    let rendered = renderer.render(&state.project, state.active_comp, state.current_frame, width, height, &workspace.render.media);
    workspace.render.last_render_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
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
        .child(format!("{:.0}%", scale * 100.0))
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
        .child(icon_button("first-frame", IconName::SkipBack, "First Frame (Home)", false, cx.listener(|this, _, _, cx| {
            let start = this.state.work_area.start as i64;
            this.go_to(start, cx);
        })))
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
            let end = this.state.work_area.end as i64 - 1;
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
}

/// Screen pixels per composition pixel at the current Fit magnification.
fn viewer_scale(workspace: &Workspace) -> f32 {
    let composition = workspace.state.composition();
    let bounds = workspace.ui.viewer_bounds.get();
    ((bounds.width - FIT_PADDING * 2.0) / composition.width as f32)
        .min((bounds.height - FIT_PADDING * 2.0) / composition.height as f32)
        .max(0.0)
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
    if workspace.ui.tool == Tool::Shape {
        let kind = workspace.ui.shape_kind;
        if workspace.state.begin_shape_draw(kind, point).is_some() {
            workspace.ui.tabs.insert(PanelId::Properties, 0);
            cx.notify();
        }
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
    let density = if workspace.state.is_playing() { window.scale_factor().min(1.0) } else { window.scale_factor() };
    let (pixels_wide, pixels_high) = ((comp_width * scale * density).round() as u32, (comp_height * scale * density).round() as u32);
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

    let comp_frame = div()
        .relative()
        .w(px(composition.width as f32 * scale))
        .h(px(composition.height as f32 * scale))
        .bg(rgba(composition.background.to_rgba_u32()))
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
        .children(selection_overlay(workspace, cx))
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
        .when(workspace.ui.tool == Tool::Shape, |viewer| viewer.cursor(gpui_kit::CursorStyle::Crosshair))
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
                this.state.add_effect(layer, &drag.name);
                this.ui.tabs.insert(PanelId::Properties, 1);
            }
            cx.notify();
        }))
        .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ViewerDrag>, _, cx| {
            let Some((x, y)) = this.ui.viewer_anchor else { return };
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
