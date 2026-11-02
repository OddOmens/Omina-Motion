//! The Project panel: a preview of the selected item and the folder tree of
//! footage, images, sound, solids and compositions. Items drag onto the timeline as layers.
//!
//! The preview shows images, plays video and sound (click it, or press its play button), and
//! scrubs through video and sound as the pointer moves across it. Which columns the list shows
//! is chosen in Settings.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::{
    AnyElement, AppContext as _, Bounds, Context, ImageSource, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, RenderImage,
    StatefulInteractiveElement, Styled, TestSupportExt as _, Window, canvas, div, fill, img, point, prelude::FluentBuilder as _, px, relative, rgb,
};
use studio_core::edit::can_add_item;
use studio_core::model::{Id, ProjectItem, ProjectItemKind};

use super::chrome::{icon_button, label_swatch, panel};
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, ProjectItemDrag, Workspace};

/// The preview's state: which item it shows, where in it (seconds), whether it is playing, and
/// the last video frame shown (made into an image once per frame, not once per redraw).
#[derive(Default)]
pub struct Preview {
    pub item: Option<Id>,
    pub seconds: f64,
    /// When playback began, and from where.
    pub playing: Option<(Instant, f64)>,
    picture: RefCell<Option<(PathBuf, i64, Arc<RenderImage>)>>,
}

/// The preview picture's size: a 16:9 box.
const PREVIEW_WIDTH: f32 = 160.0;
const PREVIEW_HEIGHT: f32 = 90.0;
/// Video frames are shrunk to about this wide for the preview.
const PREVIEW_PIXELS: u32 = 320;

/// A footage frame shrunk for the preview, as an image GPUI can draw (nearest pixel: plenty
/// for a small preview, and quick enough for every frame of playback).
fn small_picture(frame: &studio_core::media::VideoFrame) -> Option<Arc<RenderImage>> {
    let step = (frame.width / PREVIEW_PIXELS).max(1);
    let (width, height) = ((frame.width / step).max(1), (frame.height / step).max(1));
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let at = (((y * step) * frame.width + x * step) * 4) as usize;
            let pixel = frame.rgba.get(at..at + 4)?;
            bgra.extend([pixel[2], pixel[1], pixel[0], 255]);
        }
    }
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// "0:07", or "1:02:07" past an hour.
fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).floor() as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// Whether an item's preview can play: footage and sound with a file and a length.
fn playable(item: &ProjectItem) -> bool {
    matches!(item.kind, ProjectItemKind::Footage | ProjectItemKind::Audio) && item.path.is_some() && item.duration_seconds.is_some_and(|seconds| seconds > 0.0)
}

/// The video frame for the preview at its current time, from the viewer's frame server.
fn video_picture(item: &ProjectItem, workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<Arc<RenderImage>> {
    use studio_core::media::VideoSource as _;
    // Headless tests don't decode for the preview: their scheduler refuses wake-ups from the
    // decoder threads.
    if cfg!(test) {
        return None;
    }
    let (path, rate) = (item.path.as_ref()?, item.frame_rate?);
    let index = (workspace.ui.preview.seconds * rate.fps() + 1e-6).floor() as i64;
    let mut cached = workspace.ui.preview.picture.borrow_mut();
    if let Some((cached_path, cached_index, picture)) = cached.as_ref()
        && cached_path == path
        && *cached_index == index
    {
        return Some(picture.clone());
    }
    let frame = workspace.render.media.frame(path, index)?;
    let picture = small_picture(&frame)?;
    // The previous frame's texture is freed, or every frame would stay in GPU memory.
    if let Some((_, _, previous)) = cached.replace((path.clone(), frame.index, picture.clone())) {
        cx.drop_image(previous, None);
    }
    Some(picture)
}

/// A sound file's waveform across the preview, with the played part brighter.
fn waveform(peaks: Option<Arc<studio_media::Peaks>>, length: f64, at: f64) -> AnyElement {
    let Some(peaks) = peaks else { return div().into_any_element() };
    let played = (at / length.max(1e-6)) as f32;
    canvas(|_, _, _| (), move |bounds, _, window, _| {
        let width = f32::from(bounds.size.width);
        let tall = f32::from(bounds.size.height) * 0.8;
        let middle = f32::from(bounds.origin.y) + f32::from(bounds.size.height) * 0.5;
        let mut x = 0.0;
        while x < width {
            let seconds = f64::from(x / width.max(1.0)) * length;
            let level = peaks.values.get((seconds * f64::from(peaks.per_second)) as usize).copied().unwrap_or(0.0);
            let bar = (level.sqrt() * tall).max(1.0);
            let ink = if x / width.max(1.0) <= played { hsla(color::ACCENT) } else { hsla(color::MUTED) };
            window.paint_quad(fill(Bounds::new(point(bounds.origin.x + px(x), px(middle - bar * 0.5)), gpui_kit::size(px(1.5), px(bar))), ink));
            x += 2.5;
        }
    })
    .absolute()
    .size_full()
    .into_any_element()
}

/// The preview box: the picture (or waveform), a play button and a progress line for what plays,
/// and scrubbing as the pointer moves across it.
fn preview_box(item: &ProjectItem, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let preview = &workspace.ui.preview;
    let length = item.duration_seconds.unwrap_or(0.0);
    let at = preview.seconds.min(length);
    let playing = preview.playing.is_some();
    let picture: Option<AnyElement> = match (item.kind, item.path.as_ref()) {
        (ProjectItemKind::Image, Some(path)) => Some(gpui_kit::StyledImage::object_fit(img(path.clone()).size_full(), gpui_kit::ObjectFit::Contain).into_any_element()),
        (ProjectItemKind::Footage, Some(_)) => video_picture(item, workspace, cx).map(|picture| gpui_kit::StyledImage::object_fit(img(ImageSource::Render(picture)).size_full(), gpui_kit::ObjectFit::Contain).into_any_element()),
        (ProjectItemKind::Audio, Some(path)) => Some(waveform(workspace.ui.timeline.media.peaks.peaks(path), length, at)),
        (ProjectItemKind::Solid, _) => item.color.map(|color| div().size_full().bg(rgb(color.to_rgba_u32() >> 8)).into_any_element()),
        _ => None,
    };
    let picture = picture.unwrap_or_else(|| div().size_full().flex().items_center().justify_center().child(Icon::new(kind_icon(item.kind)).size(px(22.0)).text_color(hsla(color::MUTED))).into_any_element());
    let id = item.id;
    let can_play = playable(item);
    div()
        .id("project-preview")
        .relative()
        .w(px(PREVIEW_WIDTH))
        .h(px(PREVIEW_HEIGHT))
        .flex_none()
        .overflow_hidden()
        .rounded(px(2.0))
        .bg(hsla(color::INSET))
        .child(workspace.ui.preview_bounds.probe())
        .child(picture)
        .when(can_play, |frame| {
            frame
                .cursor_pointer()
                // Moving across it scrubs (while not playing); clicking plays or pauses.
                .on_mouse_move(cx.listener(move |this, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if this.ui.preview.playing.is_some() || this.ui.preview.item != Some(id) {
                        return;
                    }
                    let bounds = this.ui.preview_bounds.get();
                    let fraction = ((f32::from(event.position.x) - bounds.left) / bounds.width.max(1.0)).clamp(0.0, 1.0);
                    this.ui.preview.seconds = f64::from(fraction) * length;
                    cx.notify();
                }))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_preview();
                    cx.notify();
                }))
                // Where it is: a thin line along the bottom, and a play or pause mark.
                .child(div().absolute().left_0().bottom_0().h(px(2.0)).w(relative((at / length.max(1e-6)) as f32)).bg(hsla(color::ACCENT)))
                .child(
                    div()
                        .absolute()
                        .left(px(4.0))
                        .bottom(px(6.0))
                        .size(px(20.0))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(gpui_kit::rgba(0x000000aa))
                        .child(Icon::new(if playing { IconName::Pause } else { IconName::Play }).size(px(11.0)).text_color(gpui_kit::rgb(0xffffff))),
                )
        })
        .test_support()
        .into_any_element()
}

fn kind_icon(kind: ProjectItemKind) -> IconName {
    match kind {
        ProjectItemKind::Folder => IconName::Folder,
        ProjectItemKind::Composition => IconName::Clapperboard,
        ProjectItemKind::Footage => IconName::FileVideoCamera,
        ProjectItemKind::Image => IconName::Image,
        ProjectItemKind::Solid => IconName::Square,
        ProjectItemKind::Audio => IconName::Music,
    }
}

/// Flattens the folder tree into visible rows with their nesting depth. Folders sort first.
fn visible_rows<'a>(items: &'a [ProjectItem], is_open: &dyn Fn(Id) -> bool) -> Vec<(&'a ProjectItem, usize)> {
    fn visit<'a>(
        items: &'a [ProjectItem],
        parent: Option<Id>,
        depth: usize,
        is_open: &dyn Fn(Id) -> bool,
        rows: &mut Vec<(&'a ProjectItem, usize)>,
    ) {
        let mut children: Vec<&ProjectItem> = items.iter().filter(|item| item.parent == parent).collect();
        children.sort_by(|a, b| {
            (b.kind == ProjectItemKind::Folder).cmp(&(a.kind == ProjectItemKind::Folder)).then(a.name.cmp(&b.name))
        });
        for item in children {
            rows.push((item, depth));
            if item.kind == ProjectItemKind::Folder && is_open(item.id) {
                visit(items, Some(item.id), depth + 1, is_open, rows);
            }
        }
    }
    let mut rows = Vec::new();
    visit(items, None, 0, is_open, &mut rows);
    rows
}

/// What follows the pointer while a project item is dragged.
pub struct DragPreview {
    label: String,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded(px(3.0))
            .bg(hsla(color::ROW_SELECTED))
            .border_1()
            .border_color(hsla(color::ACCENT))
            .text_size(px(size::TEXT))
            .text_color(hsla(color::TEXT))
            .child(self.label.clone())
    }
}

/// Column widths, in points.
const TYPE_WIDTH: f32 = 96.0;
const DURATION_WIDTH: f32 = 64.0;
const DIMENSIONS_WIDTH: f32 = 90.0;
const RATE_WIDTH: f32 = 70.0;

/// The Type column: the item kind, plus proxy status for footage ("Proxy 42%", "Proxy").
fn type_label(item: &ProjectItem, workspace: &Workspace) -> String {
    let status = item.path.as_ref().filter(|_| item.kind == ProjectItemKind::Footage).and_then(|path| workspace.render.proxies.status(path));
    match status {
        Some(studio_media::ProxyStatus::Queued) => "Proxy queued".to_owned(),
        Some(studio_media::ProxyStatus::Building(progress)) => format!("Proxy {:.0}%", progress * 100.0),
        Some(studio_media::ProxyStatus::Ready(_)) => "Footage · Proxy".to_owned(),
        Some(studio_media::ProxyStatus::Failed(_)) => "Proxy failed".to_owned(),
        None => item.kind.label().to_owned(),
    }
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let project = &workspace.state.project;
    let composition = workspace.state.composition();
    let selected = workspace.state.selected_item.and_then(|id| project.item(id));
    let columns = workspace.ui.project_columns;

    let preview = div().h(px(PREVIEW_HEIGHT + 20.0)).flex_none().flex().gap_3().p_2p5().children(selected.map(|item| {
        let showing = workspace.ui.preview.item == Some(item.id);
        let time = (playable(item) && showing).then(|| format!("{} / {}", clock(workspace.ui.preview.seconds), clock(item.duration_seconds.unwrap_or(0.0))));
        div()
            .flex()
            .gap_3()
            .min_w_0()
            .child(preview_box(item, workspace, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .min_w_0()
                    .child(div().truncate().child(item.name.clone()))
                    .child(div().text_color(hsla(color::MUTED)).text_size(px(size::SMALL_TEXT)).child(item.kind.label()))
                    .child(div().text_color(hsla(color::MUTED)).text_size(px(size::SMALL_TEXT)).child(item.details()))
                    .children(time.map(|time| div().text_color(hsla(color::VALUE)).text_size(px(size::SMALL_TEXT)).font_family("Menlo").child(time))),
            )
    }));

    let column_header = div()
        .h(px(size::ROW_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .border_y_1()
        .border_color(hsla(color::HAIRLINE))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .when(columns.label, |header| header.child(div().w(px(26.0)).flex().justify_center().child(Icon::new(IconName::Tag).size(px(12.0)))))
        .child(div().flex_1().pl_2().child("Name"))
        .when(columns.kind, |header| header.child(div().w(px(TYPE_WIDTH)).pl_2().child("Type")))
        .when(columns.duration, |header| header.child(div().w(px(DURATION_WIDTH)).pl_2().child("Duration")))
        .when(columns.dimensions, |header| header.child(div().w(px(DIMENSIONS_WIDTH)).pl_2().child("Size")))
        .when(columns.frame_rate, |header| header.child(div().w(px(RATE_WIDTH)).pl_2().child("Rate")));

    let rows = visible_rows(&project.items, &|id| workspace.ui.folder_open(id)).into_iter().map(|(item, depth)| {
        let id = item.id;
        let kind = item.kind;
        let is_folder = kind == ProjectItemKind::Folder;
        let is_open = workspace.ui.folder_open(id);
        let is_selected = workspace.state.selected_item == Some(id);
        // A folder's arrow opens and closes it with one click.
        let twirl = if is_folder {
            div()
                .id(("folder-twirl", id.0 as usize))
                .size(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.ui.toggle_folder(id);
                    cx.notify();
                }))
                .child(Icon::new(if is_open { IconName::ChevronDown } else { IconName::ChevronRight }).size(px(12.0)).text_color(hsla(color::MUTED)))
                .test_support()
                .into_any_element()
        } else {
            div().w(px(14.0)).into_any_element()
        };

        let mut row = div()
            .id(("project-item", id.0 as usize))
            .h(px(size::ROW_HEIGHT))
            .flex()
            .items_center()
            .when(is_selected, |row| row.bg(hsla(color::ROW_SELECTED)))
            .hover(|style| style.bg(hsla(color::HOVER)))
            .on_click(cx.listener(move |this, event: &gpui_kit::ClickEvent, _, cx| {
                this.state.selected_item = Some(id);
                // Double-click (or Enter) opens folders and compositions.
                let opens = event.click_count() >= 2 || event.is_keyboard();
                if opens && is_folder {
                    this.ui.toggle_folder(id);
                }
                if opens && kind == ProjectItemKind::Composition {
                    this.state.open_composition(id);
                }
                cx.notify();
            }))
            .when(columns.label, |row| row.child(div().w(px(26.0)).flex().justify_center().child(label_swatch(item.label.color().to_rgba_u32()))))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .pl(px(8.0 + depth as f32 * 14.0))
                    .child(twirl)
                    // An expanded folder shows as open, as in Finder.
                    .child(Icon::new(if is_folder && is_open { IconName::FolderOpen } else { kind_icon(item.kind) }).size(px(14.0)).text_color(hsla(color::MUTED)))
                    .child(div().truncate().child(item.name.clone())),
            )
            .when(columns.kind, |row| row.child(div().w(px(TYPE_WIDTH)).pl_2().truncate().text_color(hsla(color::MUTED)).child(type_label(item, workspace))))
            .when(columns.duration, |row| row.child(div().w(px(DURATION_WIDTH)).pl_2().text_color(hsla(color::MUTED)).child(item.duration_seconds.map(clock).unwrap_or_default())))
            .when(columns.dimensions, |row| {
                let size = item.size.map(|size| format!("{} × {}", size.x, size.y)).unwrap_or_default();
                row.child(div().w(px(DIMENSIONS_WIDTH)).pl_2().truncate().text_color(hsla(color::MUTED)).child(size))
            })
            .when(columns.frame_rate, |row| row.child(div().w(px(RATE_WIDTH)).pl_2().truncate().text_color(hsla(color::MUTED)).child(item.frame_rate.map(|rate| rate.label()).unwrap_or_default())));

        if can_add_item(item, composition) {
            let drag = ProjectItemDrag { item: id, name: item.name.clone() };
            row = row.on_drag(drag, |drag, _offset, _window, cx| cx.new(|_| DragPreview { label: drag.name.clone() }));
        }
        row.test_support()
    });

    let footer = div()
        .flex()
        .items_center()
        .gap_1()
        .w_full()
        .child(icon_button("import", IconName::FileInput, "Import Images and Video (⌘I)", false, cx.listener(|this, _, window, cx| {
            this.import(window, cx);
        })))
        .child(icon_button("new-folder", IconName::FolderPlus, "New Folder", false, cx.listener(|this, _, _, cx| {
            this.state.new_folder();
            cx.notify();
        })))
        .child(icon_button("new-comp", IconName::Clapperboard, "New Composition (⌘N)", false, cx.listener(|this, _, window, cx| {
            super::composition_settings::open(this, true, window, cx);
        })))
        .child(div().flex_1())
        .child(icon_button("delete-item", IconName::Trash, "Delete (⌫)", false, |_, window, cx| {
            window.dispatch_action(Box::new(crate::workspace::Delete), cx);
        }));

    panel(
        PanelId::Project,
        &["Project"],
        Some(footer.into_any_element()),
        div()
            .id("project-drop")
            .size_full()
            .flex()
            .flex_col()
            // Files dragged in from the Finder are imported (linked where they are on disk).
            .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
            .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
                this.import_paths(paths.paths().to_vec(), window, cx);
            }))
            .child(preview)
            .child(column_header)
            .child(div().id("project-items").flex_1().min_h_0().overflow_y_scroll().children(rows).test_support()),
        workspace,
        cx,
    )
}
