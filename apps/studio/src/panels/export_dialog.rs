//! The Export dialog (File ▸ Export…, ⌘M). On the left, a preview of the frames that will be
//! exported, with a scrubber across exactly that range and a summary of the file (size, frame
//! rate, length, sound). On the right, the settings, grouped: the format (and for H.264 and
//! H.265 their quality and encoding speed), the picture (size and frame rate), which frames,
//! and the sound. Every choice is kept in `ExportChoice` between exports; Export… asks where
//! to save, then `export::start` runs it in the background. Add to Queue… saves the export to
//! run later with others (see `export_queue`). At the top, a Preset menu puts a saved set of
//! choices in place, and Save as Preset keeps the current ones under a name (see
//! `export_presets`).

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{Disableable as _, Icon, Sizable as _, WindowExt as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, DragMoveEvent, Entity, ImageSource, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, RenderImage, StatefulInteractiveElement, Styled, TestSupportExt as _, WeakEntity, Window, div, img, prelude::FluentBuilder as _, px,
    relative,
};
use studio_core::audio::composition_audio;
use studio_core::model::Id;
use studio_core::time::{FrameRate, format_timecode as timecode};
use studio_export::{ExportFormat, ExportRequest};
use studio_media::{EncodeSpeed, MovieFormat, VideoQuality};

use super::chrome::{EmptyDragView, MeasuredBounds};
use super::export::{ExportChoice, chosen_frames};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The dialog's width, and the box the preview picture is fitted into.
const DIALOG_WIDTH: f32 = 900.0;
const PREVIEW_WIDTH: f32 = 500.0;
const PREVIEW_HEIGHT: f32 = 282.0;
/// The width of the setting names in the right column.
const LABEL_WIDTH: f32 = 84.0;

/// Output sizes offered, as fractions of the composition.
const SCALES: [(f32, &str); 4] = [(1.0, "Full size"), (0.75, "Three quarters"), (0.5, "Half"), (0.25, "Quarter")];
/// Fixed bitrates offered for H.264 and H.265, in kilobits per second, with what they suit.
const BITRATES: [(u32, &str); 4] = [(8_000, "social 1080p"), (16_000, "YouTube 1080p"), (35_000, "YouTube 4K"), (50_000, "broadcast")];
/// Sound bitrates offered, in kilobits per second.
const AUDIO_BITRATES: [u32; 4] = [128, 192, 256, 320];
/// The highest sound bitrate WebM's Opus encoder takes; a higher choice is encoded at this.
const OPUS_MAX_KBPS: u32 = 256;

/// The sound bitrate the export really uses: the one chosen, or Opus's highest for WebM.
fn sound_kbps(choice: &ExportChoice) -> u32 {
    if choice.format == ExportFormat::Movie(MovieFormat::WebM) { choice.audio_kbps.min(OPUS_MAX_KBPS) } else { choice.audio_kbps }
}
/// Frame rates offered besides the composition's own.
const FRAME_RATES: [FrameRate; 10] = [
    FrameRate::new(12, 1),
    FrameRate::new(15, 1),
    FrameRate::new(24000, 1001),
    FrameRate::new(24, 1),
    FrameRate::new(25, 1),
    FrameRate::new(30000, 1001),
    FrameRate::new(30, 1),
    FrameRate::new(50, 1),
    FrameRate::new(60000, 1001),
    FrameRate::new(60, 1),
];

/// The preview's state while the dialog is open: the frame picked with the scrubber, and the
/// picture last drawn for it (drawn again only when the frame, size or decoded footage change).
#[derive(Default)]
pub struct ExportPreview {
    /// The frame picked with the scrubber, or `None` for the playhead's frame.
    pub frame: Option<i64>,
    picture: Option<PreviewPicture>,
    bar: MeasuredBounds,
}

impl ExportPreview {
    /// The frame the preview picture on screen was drawn from, if one was.
    #[cfg(test)]
    pub fn drawn_frame(&self) -> Option<i64> {
        self.picture.as_ref().map(|picture| picture.key.frame)
    }
}

struct PreviewPicture {
    key: PreviewKey,
    /// The frame server's count of arrived frames when this was drawn.
    media: u64,
    /// Whether every video frame in it was the one asked for (one still decoding is stood in
    /// for by a nearby frame until it arrives).
    exact: bool,
    image: Arc<RenderImage>,
}

/// What a preview picture shows. The project itself can't change while the dialog is open.
#[derive(Clone, Copy, PartialEq)]
struct PreviewKey {
    comp: Id,
    frame: i64,
    width: u32,
    height: u32,
}

/// Dragging the preview's scrubber.
#[derive(Clone, Copy)]
struct ScrubDrag;

/// One line of a settings dropdown: a choice, a heading over a group of choices, or a divider.
enum MenuEntry<T> {
    Choice { label: String, checked: bool, enabled: bool, value: T },
    Heading(&'static str),
    Separator,
}

impl<T> MenuEntry<T> {
    fn choice(label: impl Into<String>, checked: bool, value: T) -> Self {
        Self::Choice { label: label.into(), checked, enabled: true, value }
    }
}

/// A dropdown in the dialog whose choices set the workspace's export choice.
fn choice_menu<T: Clone + 'static>(id: &'static str, label: String, entries: Vec<MenuEntry<T>>, set: fn(&mut ExportChoice, T), target: WeakEntity<Workspace>) -> AnyElement {
    let entries = std::rc::Rc::new(entries);
    crate::panels::chrome::dropdown(id, label)
        .dropdown_menu(move |menu, _, _| {
            entries.iter().fold(menu, |menu: PopupMenu, entry| match entry {
                MenuEntry::Heading(title) => menu.label(*title),
                MenuEntry::Separator => menu.separator(),
                MenuEntry::Choice { label, checked, enabled, value } => {
                    let (target, value) = (target.clone(), value.clone());
                    menu.item(PopupMenuItem::new(label.clone()).checked(*checked).disabled(!enabled).on_click(move |_, _, cx| {
                        target
                            .update(cx, |this, cx| {
                                set(&mut this.ui.export_choice, value.clone());
                                cx.notify();
                            })
                            .ok();
                    }))
                }
            })
        })
        .into_any_element()
}

/// Opens the Export dialog for the open composition.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut gpui_kit::Context<Workspace>) {
    if workspace.ui.export_job.is_some() {
        window.push_notification("An export is already running. Cancel it or wait for it to finish.", cx);
        return;
    }
    // Each opening starts at the playhead's frame.
    forget_preview(workspace, window, cx);
    if workspace.ui.export_preset_name.is_none() {
        workspace.ui.export_preset_name = Some(cx.new(|cx| gpui_kit::component::input::InputState::new(window, cx).placeholder("Preset name")));
    }
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, window, cx| {
        let Some(workspace) = target.upgrade() else { return dialog };
        let start = target.clone();
        let close = target.clone();
        let queue_on = workspace.read(cx).has(crate::features::Feature::ExportQueue);
        dialog
            .title(dialog_title(&workspace, cx))
            .w(px(DIALOG_WIDTH))
            .child(body(&workspace, &target, window, cx))
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel").w(px(100.0))))
                    .when(queue_on, |footer| footer.child({
                        // Saves these settings as a queued export, to run later with others.
                        let queue = target.clone();
                        Button::new("export-queue-add").outline().label("Add to Queue…").on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            queue.update(cx, |this, cx| super::export::choose_file_then(this, true, window, cx)).ok();
                        })
                    }))
                    .child(DialogAction::new().child(Button::new("export-ok").primary().icon(IconName::Download).label("Export…").w(px(132.0)))),
            )
            .on_ok(move |_, window, cx| {
                start
                    .update(cx, |this, cx| {
                        forget_preview(this, window, cx);
                        super::export::choose_file(this, window, cx);
                    })
                    .ok();
                true
            })
            .on_close(move |_, window, cx| {
                close.update(cx, |this, cx| forget_preview(this, window, cx)).ok();
            })
    });
}

/// Frees the preview picture and goes back to showing the playhead's frame.
fn forget_preview(workspace: &mut Workspace, window: &mut Window, cx: &mut App) {
    workspace.ui.export_preview.frame = None;
    if let Some(picture) = workspace.ui.export_preview.picture.take() {
        cx.drop_image(picture.image, Some(window));
    }
}

fn dialog_title(workspace: &Entity<Workspace>, cx: &App) -> String {
    format!("Export “{}”", workspace.read(cx).state.composition().name)
}

/// The two columns: preview and summary on the left, settings on the right.
fn body(workspace: &Entity<Workspace>, target: &WeakEntity<Workspace>, window: &mut Window, cx: &mut App) -> AnyElement {
    // The preview is drawn first: it needs the editor's renderer, so it updates the workspace.
    let (frame, range) = {
        let ws = workspace.read(cx);
        let range = chosen_frames(&ws.state, &ws.ui.export_choice);
        (preview_frame(ws, &range), range)
    };
    let fitted = {
        let comp = workspace.read(cx).state.composition();
        fit(comp.width, comp.height)
    };
    let scale = window.scale_factor();
    let pixels = (((fitted.0 * scale).round() as u32).max(2), ((fitted.1 * scale).round() as u32).max(2));
    let picture = workspace.update(cx, |ws, cx| preview_picture(ws, frame, pixels, window, cx));

    let ws = workspace.read(cx);
    div()
        .flex()
        .gap_5()
        .text_size(px(size::LARGE_TEXT))
        .child(
            div()
                .w(px(PREVIEW_WIDTH))
                .flex_none()
                .flex()
                .flex_col()
                .gap_2()
                .child(preview_box(ws, fitted, picture))
                .child(scrubber(ws, frame, &range, target))
                .child(summary(ws, &range)),
        )
        .child(settings(ws, target))
        .into_any_element()
}

/// The frame the preview shows: the one picked with the scrubber, else the playhead's, kept
/// inside the frames being exported.
fn preview_frame(workspace: &Workspace, range: &Range<i64>) -> i64 {
    let wanted = workspace.ui.export_preview.frame.unwrap_or(workspace.state.current_frame);
    wanted.clamp(range.start, (range.end - 1).max(range.start))
}

/// The preview picture's size on screen: the composition fitted into the preview box.
fn fit(width: u32, height: u32) -> (f32, f32) {
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (PREVIEW_WIDTH / width).min(PREVIEW_HEIGHT / height);
    ((width * scale).max(1.0), (height * scale).max(1.0))
}

/// The preview of `frame`, drawn `width` × `height` pixels with the viewer's renderer and
/// footage, or the picture already drawn when nothing it shows has changed.
fn preview_picture(workspace: &mut Workspace, frame: i64, (width, height): (u32, u32), window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    let key = PreviewKey { comp: workspace.state.active_comp, frame, width, height };
    let media = workspace.render.media.generation();
    if let Some(picture) = &workspace.ui.export_preview.picture
        && picture.key == key
        && (picture.exact || picture.media == media)
    {
        return Some(picture.image.clone());
    }
    let (rendered, exact) = workspace.render.preview_frame(&workspace.state.project, key.comp, frame, width, height)?;
    let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(rendered.width, rendered.height, rendered.bgra)?;
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
    let fresh = PreviewPicture { key, media, exact, image: image.clone() };
    // Free the previous picture's texture, as the viewer does with its frames.
    if let Some(previous) = workspace.ui.export_preview.picture.replace(fresh) {
        cx.drop_image(previous.image, Some(window));
    }
    Some(image)
}

/// The picture in its box: over a checkerboard when the format keeps transparency (so it shows
/// what stays see-through), else over black, as the file will have it.
fn preview_box(workspace: &Workspace, (width, height): (f32, f32), picture: Option<Arc<RenderImage>>) -> impl IntoElement {
    let format = workspace.ui.export_choice.format;
    let backdrop = if format.has_alpha() {
        div().absolute().size_full().overflow_hidden().child(gpui_kit::StyledImage::object_fit(img(super::viewer::transparency_grid()).size_full(), gpui_kit::ObjectFit::Cover))
    } else {
        div().absolute().size_full().bg(gpui_kit::black())
    };
    let shown = div()
        .id("export-preview")
        .relative()
        .w(px(width))
        .h(px(height))
        .overflow_hidden()
        .child(backdrop)
        .children(picture.map(|picture| img(ImageSource::Render(picture)).absolute().size_full()))
        .test_support();
    let badge = |text: String| {
        div().px_1p5().py_0p5().rounded(px(4.0)).bg(gpui_kit::hsla(0.0, 0.0, 0.0, 0.6)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(text)
    };
    let comp = workspace.state.composition();
    let request = ExportRequest { scale: workspace.ui.export_choice.scale, ..ExportRequest::new(comp.id, 0..1, format, PathBuf::new()) };
    let (out_width, out_height) = request.output_size(comp.width, comp.height);
    div()
        .relative()
        .w(px(PREVIEW_WIDTH))
        .h(px(PREVIEW_HEIGHT))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .overflow_hidden()
        .bg(hsla(color::PASTEBOARD))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(shown)
        .child(div().absolute().top_2().left_2().flex().gap_1().child(badge(format.label().to_owned())).child(badge(format!("{out_width} × {out_height}"))))
}

/// A bar across exactly the frames being exported: click or drag along it to preview any of
/// them, or step a frame at a time with the arrows. The timecodes say where it is and where
/// the range ends.
fn scrubber(workspace: &Workspace, frame: i64, range: &Range<i64>, target: &WeakEntity<Workspace>) -> impl IntoElement {
    let rate = workspace.state.composition().frame_rate;
    let span = (range.end - range.start - 1).max(1);
    let fraction = ((frame - range.start) as f32 / span as f32).clamp(0.0, 1.0);
    let bar = workspace.ui.export_preview.bar.clone();
    let (first, last) = (range.start, (range.end - 1).max(range.start));
    // A pointer `x` (window pixels) across the bar `left`..`left + width`, as a frame.
    let pick = move |target: &WeakEntity<Workspace>, x: f32, left: f32, width: f32, cx: &mut App| {
        let at = ((x - left) / width.max(1.0)).clamp(0.0, 1.0);
        let frame = first + (at * (last - first) as f32).round() as i64;
        target
            .update(cx, |this, cx| {
                this.ui.export_preview.frame = Some(frame);
                cx.notify();
            })
            .ok();
    };
    let step = |id: &'static str, icon: IconName, by: i64, target: WeakEntity<Workspace>| {
        Button::new(id).ghost().xsmall().icon(icon).on_click(move |_, _, cx| {
            target
                .update(cx, |this, cx| {
                    let range = chosen_frames(&this.state, &this.ui.export_choice);
                    let frame = preview_frame(this, &range) + by;
                    this.ui.export_preview.frame = Some(frame.clamp(range.start, (range.end - 1).max(range.start)));
                    cx.notify();
                })
                .ok();
        })
    };
    let (down, moved) = (target.clone(), target.clone());
    let track = div()
        .id("export-scrub")
        .relative()
        .flex_1()
        .h(px(18.0))
        .cursor_pointer()
        .child(bar.probe())
        .child(div().absolute().left_0().right_0().top(px(7.0)).h(px(4.0)).rounded(px(2.0)).bg(hsla(color::HAIRLINE)))
        .child(div().absolute().left_0().top(px(7.0)).h(px(4.0)).w(relative(fraction)).rounded(px(2.0)).bg(hsla(color::ACCENT)))
        .child(div().absolute().left(relative(fraction)).ml(px(-6.0)).top(px(3.0)).size(px(12.0)).rounded_full().bg(hsla(color::TEXT)).border_2().border_color(hsla(color::ACCENT)))
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            let area = bar.get();
            pick(&down, f32::from(event.position.x), area.left, area.width, cx);
        })
        .on_drag(ScrubDrag, |_, _, _, cx| cx.new(|_| EmptyDragView))
        .on_drag_move(move |event: &DragMoveEvent<ScrubDrag>, _, cx| {
            pick(&moved, f32::from(event.event.position.x), f32::from(event.bounds.origin.x), f32::from(event.bounds.size.width), cx);
        })
        .test_support();
    let muted = |text: String| div().flex_none().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text);
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(step("export-preview-back", IconName::ChevronLeft, -1, target.clone()))
        .child(div().flex_none().w(px(76.0)).text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(timecode(frame, rate)))
        .child(track)
        .child(muted(timecode(last, rate)))
        .child(step("export-preview-forward", IconName::ChevronRight, 1, target.clone()))
}

/// What the file will be, at a glance: picture size, frame rate, length, sound, transparency,
/// and its size when that can be worked out (a fixed bitrate).
fn summary(workspace: &Workspace, range: &Range<i64>) -> impl IntoElement {
    let choice = workspace.ui.export_choice;
    let comp = workspace.state.composition();
    let request = request_for(workspace, range.clone());
    let (width, height) = request.output_size(comp.width, comp.height);
    let rate = request.output_rate(comp.frame_rate);
    let frames = request.output_frame_count(comp.frame_rate);
    let seconds = (range.end - range.start) as f64 / comp.frame_rate.fps();
    let sound = has_audio(workspace) && choice.include_audio && choice.format.has_sound();
    let mut chips = vec![
        chip(IconName::Scan, format!("{width} × {height}")),
        chip(IconName::Gauge, rate.label()),
        chip(IconName::Timer, format!("{} · {frames} frames", seconds_label(seconds))),
        match (sound, choice.format.has_sound_bitrate()) {
            (false, _) => chip(IconName::VolumeX, "No sound".into()),
            (true, true) => {
                let codec = if choice.format == ExportFormat::Movie(MovieFormat::WebM) { "Opus" } else { "AAC" };
                chip(IconName::Volume2, format!("{codec} {} kbps", sound_kbps(&choice)))
            }
            (true, false) => chip(IconName::Volume2, "Uncompressed sound".into()),
        },
    ];
    if choice.format.has_alpha() {
        chips.push(chip(IconName::Grid3x3, "Keeps transparency".into()));
    }
    if let (VideoQuality::Bitrate(kbps), true) = (choice.quality, choice.format.has_quality()) {
        let total_kbps = f64::from(kbps) + if sound { f64::from(sound_kbps(&choice)) } else { 0.0 };
        chips.push(chip(IconName::Download, format!("About {}", megabytes(total_kbps * seconds / 8.0 / 1000.0))));
    }
    div().flex().flex_wrap().gap_1p5().children(chips)
}

fn chip(icon: IconName, text: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded(px(5.0))
        .bg(hsla(color::RAISED))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::TEXT))
        .child(Icon::new(icon).size(px(12.0)).text_color(hsla(color::MUTED)))
        .child(text)
        .into_any_element()
}

/// "4 s", "2.5 s", "1 min 12 s".
fn seconds_label(seconds: f64) -> String {
    let tenths = (seconds * 10.0).round() / 10.0;
    if tenths < 60.0 {
        return if tenths.fract() == 0.0 { format!("{tenths} s") } else { format!("{tenths:.1} s") };
    }
    let whole = tenths.round() as i64;
    format!("{} min {} s", whole / 60, whole % 60)
}

/// "38 MB", "1.2 GB".
fn megabytes(megabytes: f64) -> String {
    if megabytes >= 1000.0 { format!("{:.1} GB", megabytes / 1000.0) } else { format!("{} MB", megabytes.round().max(1.0)) }
}

/// The export the dialog's choices describe, for working out sizes and frame counts.
fn request_for(workspace: &Workspace, frames: Range<i64>) -> ExportRequest {
    let choice = workspace.ui.export_choice;
    ExportRequest { scale: choice.scale, frame_rate: choice.frame_rate, quality: choice.quality, ..ExportRequest::new(workspace.state.active_comp, frames, choice.format, PathBuf::new()) }
}

fn has_audio(workspace: &Workspace) -> bool {
    !composition_audio(&workspace.state.project, workspace.state.composition()).is_empty()
}

/// What each format is for, under the format menu.
fn format_note(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Movie(MovieFormat::H264) => "Plays everywhere. The usual choice for sharing and uploading.",
        ExportFormat::Movie(MovieFormat::H265) => "About half the size of H.264 at the same quality. Plays on recent phones and computers.",
        ExportFormat::Movie(MovieFormat::ProRes4444) => "Keeps transparency, for overlays and graphics used in other editing apps.",
        ExportFormat::Movie(MovieFormat::WebM) => "Small files for websites and web apps, keeping transparency. Plays in every browser; Chrome, Edge and Firefox show the transparency.",
        ExportFormat::Movie(MovieFormat::ProRes422Hq) => "Near-lossless, for finishing and further editing. Large files.",
        ExportFormat::Movie(MovieFormat::ProRes422) => "High quality for editing, a little smaller than HQ.",
        ExportFormat::Movie(MovieFormat::ProResLt) => "Lighter editing copies, about two-thirds the size of ProRes 422.",
        ExportFormat::Movie(MovieFormat::ProResProxy) => "Small, rough editing copies for offline editing.",
        ExportFormat::Gif => "Loops silently in chats and on web pages. 256 colors and large files, so keep it short and small.",
        ExportFormat::PngSequence => "One picture per frame, with transparency, in a new folder.",
        ExportFormat::Lottie => "Shapes, text and keyframes for Lottie players on the web and in apps. Video, sound and most effects are left out; you’ll see a list after exporting.",
        ExportFormat::DotLottie => "The Lottie animation zipped into one smaller file, for LottieFiles and dotLottie players.",
        ExportFormat::AnimatedSvg => "An SVG that animates itself with CSS, for web pages. Video, sound and most effects are left out; you’ll see a list after exporting.",
    }
}

/// The right column: one group each for the format, the picture, the frames and the sound.
/// The Preset group: a menu of the saved presets and the ones Motion comes with, which puts a
/// preset's choices in place, and a name field with Save as Preset, which keeps the choices
/// made now. A preset made for another frame shape says so, since the window can't reshape the
/// composition (Iris can: "export for Instagram Reel" makes a copy at that size first).
fn presets(workspace: &Workspace, target: &WeakEntity<Workspace>) -> impl IntoElement {
    use super::export_presets::{self as presets, PresetSettings};
    let choice = workspace.ui.export_choice;
    let comp = workspace.state.composition();
    let saved = presets::saved();
    let own = presets::built_in();
    let listed: Vec<(iris::export_presets::Preset, bool)> = saved.iter().cloned().map(|preset| (preset, false)).chain(own.iter().filter(|preset| !saved.iter().any(|kept| kept.name.eq_ignore_ascii_case(&preset.name))).cloned().map(|preset| (preset, true))).collect();
    let current = listed.iter().find(|(preset, _)| PresetSettings::of(preset).choice() == choice).map(|(preset, _)| preset.clone());
    let mut entries = Vec::new();
    if listed.iter().any(|(_, own)| !own) {
        entries.push(MenuEntry::Heading("Saved"));
    }
    for (index, (preset, is_own)) in listed.iter().enumerate() {
        if *is_own && (index == 0 || !listed[index - 1].1) {
            if index > 0 {
                entries.push(MenuEntry::Separator);
            }
            entries.push(MenuEntry::Heading("Comes with Omina Motion"));
        }
        let picked = PresetSettings::of(preset).choice();
        entries.push(MenuEntry::choice(preset.name.clone(), picked == choice, picked));
    }
    let label = current.as_ref().map_or_else(|| "Custom".to_owned(), |preset| preset.name.clone());
    let menu = choice_menu("export-preset", label, entries, |c, picked| *c = picked, target.clone());
    let reshape = current.as_ref().and_then(|preset| PresetSettings::of(preset).frame).filter(|frame| *frame != (comp.width, comp.height)).map(|(width, height)| {
        let name = current.as_ref().map_or_else(String::new, |preset| preset.name.clone());
        div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("Made for {width} × {height}; this composition is {} × {}. Ask Iris “export for {name}” to export a copy at that size.", comp.width, comp.height))
    });
    let name_field = workspace.ui.export_preset_name.clone();
    let save = {
        let (target, field) = (target.clone(), name_field.clone());
        Button::new("export-preset-save").outline().small().label("Save as Preset").on_click(move |_, window, cx| {
            let Some(field) = field.as_ref() else { return };
            let name = field.read(cx).value().trim().to_owned();
            if name.is_empty() {
                window.push_notification("Type a name for the preset first.", cx);
                return;
            }
            let Some(choice) = target.upgrade().map(|workspace| workspace.read(cx).ui.export_choice) else { return };
            let said = presets::save_current(&choice, &name).unwrap_or_else(|why| why);
            field.update(cx, |field, cx| field.set_value("", window, cx));
            window.push_notification(said.as_str(), cx);
            target.update(cx, |_, cx| cx.notify()).ok();
        })
    };
    let field = name_field.map(|field| div().id("export-preset-name").w(px(140.0)).child(gpui_kit::component::input::Input::new(&field).small()).test_support());
    group("Preset").child(div().flex().items_center().gap_2().child(menu).child(div().flex_1()).children(field).child(save)).children(reshape)
}

fn settings(workspace: &Workspace, target: &WeakEntity<Workspace>) -> impl IntoElement {
    let choice = workspace.ui.export_choice;
    let comp = workspace.state.composition();
    let rate = comp.frame_rate;

    // Format, grouped by what each is for.
    let format_entry = |format: ExportFormat| MenuEntry::choice(format.label(), choice.format == format, format);
    let mut formats = vec![MenuEntry::Heading("Share")];
    formats.extend([MovieFormat::H264, MovieFormat::H265].map(|format| format_entry(format.into())));
    formats.extend([MenuEntry::Separator, MenuEntry::Heading("Edit and finish")]);
    formats.extend(ExportFormat::ALL.iter().filter(|format| format.movie().is_some_and(MovieFormat::is_prores)).map(|&format| format_entry(format)));
    // WebM needs encoders an FFmpeg may be built without; then it is shown greyed out, saying so.
    let webm_format = ExportFormat::from(MovieFormat::WebM);
    let webm_ok = MovieFormat::WebM.can_encode();
    let webm_label = if webm_ok { webm_format.label().to_owned() } else { format!("{} — not in this build", webm_format.label()) };
    let webm = MenuEntry::Choice { label: webm_label, checked: choice.format == webm_format, enabled: webm_ok, value: webm_format };
    formats.extend([MenuEntry::Separator, MenuEntry::Heading("Web and pictures"), webm, format_entry(ExportFormat::Gif), format_entry(ExportFormat::PngSequence)]);
    formats.extend([MenuEntry::Separator, MenuEntry::Heading("Vector animation")]);
    formats.extend(ExportFormat::ALL.iter().filter(|format| format.is_vector()).map(|&format| format_entry(format)));
    let format_menu = choice_menu("export-format", choice.format.label().to_owned(), formats, |c, format| c.format = format, target.clone());

    let mut qualities: Vec<MenuEntry<VideoQuality>> = VideoQuality::PRESETS.iter().map(|&quality| MenuEntry::choice(quality.label(), choice.quality == quality, quality)).collect();
    qualities.push(MenuEntry::Separator);
    qualities.extend(BITRATES.iter().map(|&(kbps, suits)| MenuEntry::choice(format!("{} ({suits})", VideoQuality::Bitrate(kbps).label()), choice.quality == VideoQuality::Bitrate(kbps), VideoQuality::Bitrate(kbps))));
    let quality_menu = choice_menu("export-quality", choice.quality.label(), qualities, |c, quality| c.quality = quality, target.clone());
    let speeds = EncodeSpeed::ALL.iter().map(|&speed| MenuEntry::choice(speed.label(), choice.speed == speed, speed)).collect();
    let speed_menu = choice_menu("export-speed", choice.speed.label().to_owned(), speeds, |c, speed| c.speed = speed, target.clone());

    // Picture: size and frame rate.
    let size_label = |scale: f32, name: &str| {
        let request = ExportRequest { scale, ..ExportRequest::new(comp.id, 0..1, choice.format, PathBuf::new()) };
        let (width, height) = request.output_size(comp.width, comp.height);
        format!("{name} ({width} × {height})")
    };
    let current_size = SCALES.iter().find(|(scale, _)| *scale == choice.scale).map_or("Full size", |(_, name)| name);
    let sizes = SCALES.iter().map(|&(scale, name)| MenuEntry::choice(size_label(scale, name), choice.scale == scale, scale)).collect();
    let size_menu = choice_menu("export-size", size_label(choice.scale, current_size), sizes, |c, scale| c.scale = scale, target.clone());
    // Vector files draw sharply at any size, so there is no size to choose.
    let size_control = if choice.format.is_vector() { note("Any size: shapes stay sharp when scaled").into_any_element() } else { size_menu };
    let rate_menu = frame_rate_menu(choice, comp.id, rate, target);

    // Frames: the work area or the whole composition.
    let work_area = (workspace.state.work_area.start as i64, workspace.state.work_area.end as i64);
    let span = if choice.work_area_only { work_area } else { (0, comp.duration_frames) };
    let range_switch = div()
        .flex()
        .gap_0p5()
        .p_0p5()
        .rounded(px(7.0))
        .bg(hsla(color::INSET))
        .child(range_choice("export-work-area", "Work Area", true, choice.work_area_only, target))
        .child(range_choice("export-whole", "Entire Composition", false, !choice.work_area_only, target));
    let range_line = div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} – {}", timecode(span.0, rate), timecode(span.1, rate)));

    // Sound.
    let audible = has_audio(workspace);
    let can_sound = audible && choice.format.has_sound();
    let audio_box = {
        let target = target.clone();
        Checkbox::new("export-audio").label("Include sound").checked(can_sound && choice.include_audio).disabled(!can_sound).on_click(move |checked: &bool, _, cx| {
            let checked = *checked;
            target
                .update(cx, |this, cx| {
                    this.ui.export_choice.include_audio = checked;
                    cx.notify();
                })
                .ok();
        })
    };
    let sound_note = match (choice.format, audible) {
        (format, _) if format.is_vector() => Some("Vector animations can't carry sound."),
        (ExportFormat::Gif, _) => Some("GIFs can't carry sound."),
        (ExportFormat::PngSequence, _) => Some("Pictures can't carry sound."),
        (_, false) => Some("This composition has no audible layers."),
        _ if !choice.format.has_sound_bitrate() && choice.include_audio => Some("ProRes keeps the sound uncompressed."),
        _ => None,
    };
    // Only the bitrates the format's encoder takes, with the one it will really use checked.
    let used_kbps = sound_kbps(&choice);
    let audio_rates = AUDIO_BITRATES.iter().filter(|&&kbps| kbps == sound_kbps(&ExportChoice { audio_kbps: kbps, ..choice })).map(|&kbps| MenuEntry::choice(format!("{kbps} kbps"), used_kbps == kbps, kbps)).collect();
    let show_bitrate = can_sound && choice.include_audio && choice.format.has_sound_bitrate();
    let audio_menu = show_bitrate.then(|| choice_menu("export-audio-bitrate", format!("{used_kbps} kbps"), audio_rates, |c, kbps| c.audio_kbps = kbps, target.clone()));

    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_4()
        .child(presets(workspace, target))
        .child(
            group("Format")
                .child(format_menu)
                .child(note(format_note(choice.format)))
                .when(choice.format.has_quality(), |group| group.child(row("Quality", quality_menu)).child(row("Speed", speed_menu))),
        )
        .child(group("Picture").child(row("Size", size_control)).child(row("Frame rate", rate_menu)))
        .child(group("Frames").child(range_switch).child(range_line))
        .child(group("Sound").child(div().flex().items_center().gap_3().child(audio_box).child(div().flex_1()).children(audio_menu)).children(sound_note.map(note)))
}

/// The frame rate dropdown: the composition's own, or a fixed rate. Rates the format can't play
/// are offered but disabled (GIFs stop at 50).
fn frame_rate_menu(choice: ExportChoice, comp: Id, composition: FrameRate, target: &WeakEntity<Workspace>) -> AnyElement {
    let effective = ExportRequest { frame_rate: choice.frame_rate, ..ExportRequest::new(comp, 0..1, choice.format, PathBuf::new()) }.output_rate(composition);
    let label = match choice.frame_rate {
        _ if !choice.format.allows_rate(choice.frame_rate.unwrap_or(composition)) => format!("{} (GIF limit)", effective.label()),
        None => format!("Composition ({})", composition.label()),
        Some(rate) => rate.label(),
    };
    let mut rates = vec![MenuEntry::Choice {
        label: format!("Composition ({})", composition.label()),
        checked: choice.frame_rate.is_none(),
        enabled: choice.format.allows_rate(composition),
        value: None,
    }];
    rates.push(MenuEntry::Separator);
    rates.extend(FRAME_RATES.iter().map(|&rate| MenuEntry::Choice { label: rate.label(), checked: choice.frame_rate == Some(rate), enabled: choice.format.allows_rate(rate), value: Some(rate) }));
    choice_menu("export-frame-rate", label, rates, |c, rate| c.frame_rate = rate, target.clone())
}

/// One half of the Frames switch.
fn range_choice(id: &'static str, label: &'static str, work_area: bool, selected: bool, target: &WeakEntity<Workspace>) -> impl IntoElement {
    let target = target.clone();
    Button::new(id).small().label(label).flex_1().map(|button| if selected { button.primary() } else { button.ghost() }).on_click(move |_, _, cx| {
        target
            .update(cx, |this, cx| {
                this.ui.export_choice.work_area_only = work_area;
                cx.notify();
            })
            .ok();
    })
}

/// A titled group of settings.
fn group(title: &'static str) -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(title.to_uppercase()))
}

/// A setting's name and its control, lined up with the others.
fn row(label: &'static str, control: AnyElement) -> impl IntoElement {
    div().flex().items_center().gap_3().child(div().w(px(LABEL_WIDTH)).flex_none().text_color(hsla(color::MUTED)).child(label)).child(control)
}

fn note(text: &'static str) -> impl IntoElement {
    div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_sizes_read_naturally() {
        assert_eq!(seconds_label(4.0), "4 s");
        assert_eq!(seconds_label(2.5), "2.5 s");
        assert_eq!(seconds_label(72.0), "1 min 12 s");
        assert_eq!(megabytes(37.6), "38 MB");
        assert_eq!(megabytes(0.2), "1 MB");
        assert_eq!(megabytes(1234.0), "1.2 GB");
    }

    #[test]
    fn the_preview_keeps_the_composition_shape() {
        let (width, height) = fit(1920, 1080);
        assert!((width - PREVIEW_WIDTH).abs() < 0.01 && (height - PREVIEW_WIDTH * 9.0 / 16.0).abs() < 0.01);
        let (width, height) = fit(1080, 1920);
        assert!((height - PREVIEW_HEIGHT).abs() < 0.01 && width < height, "tall compositions fit the height");
    }
}
