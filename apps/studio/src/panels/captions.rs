//! Making captions from a clip's sound (the Generate Captions button in Properties). Speech
//! recognition runs on this computer with Whisper (see `studio_captions`); the first time,
//! its model (about 150 MB) is downloaded, after asking. The work runs on a background thread,
//! with progress and Cancel in the top bar, and ends in a caption layer above the clip.
//!
//! The Captions panel (on the rail) lists the clips with sound in the open composition, each
//! with a button to caption it, and ready-made caption looks to apply to a caption layer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::{Disableable as _, Icon};
use gpui_kit::assets::IconName;
use gpui_kit::{Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px, rgb};
use studio_core::model::{Id, LayerContent};

use crate::state::HeardWord;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

/// A captioning job running in the background.
pub struct CaptionJob {
    stage: String,
    fraction: f32,
    cancel: Arc<AtomicBool>,
}

/// What the background thread reports.
#[derive(Default)]
struct Report {
    stage: String,
    fraction: f32,
    outcome: Option<Result<Vec<HeardWord>, String>>,
}

/// How often the editor checks on the job.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Where the speech model is kept: Studio's settings folder, under `Models/whisper-tiny`.
pub fn model_folder() -> PathBuf {
    crate::app_settings::folder().join("Models").join("whisper-tiny")
}

fn lock(report: &Mutex<Report>) -> std::sync::MutexGuard<'_, Report> {
    report.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Downloads the model's files, each checked against the SHA-256 it was published with (see
/// `download`). Files already there are kept.
fn download_model(folder: &std::path::Path, report: &Mutex<Report>, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("could not make the model folder: {error}"))?;
    let files = studio_captions::model_paths(folder);
    for (index, (url, destination, sha256)) in files.iter().enumerate() {
        if cancelled() {
            return Err("cancelled".into());
        }
        {
            let mut report = lock(report);
            report.stage = "Downloading the captions model".into();
            report.fraction = index as f32 / files.len() as f32;
        }
        if destination.is_file() {
            continue;
        }
        crate::download::fetch_checked(url, destination, sha256, "the captions model", cancelled)?;
    }
    Ok(())
}

/// Downloads the model now (Settings > AI Models), on the calling thread.
pub fn fetch_model() -> Result<(), String> {
    download_model(&model_folder(), &Mutex::new(Report::default()), &|| false)
}

/// The background work: download the model if needed, then listen and transcribe.
fn run(source: (PathBuf, f64, f64), report: &Mutex<Report>, cancelled: &dyn Fn() -> bool) -> Result<Vec<HeardWord>, String> {
    let folder = model_folder();
    if !studio_captions::model_present(&folder) {
        download_model(&folder, report, cancelled)?;
    }
    lock(report).stage = "Listening".into();
    let (path, start, end) = source;
    let speech = studio_captions::read_speech(&path, start, end).map_err(|error| error.to_string())?;
    let mut transcriber = studio_captions::Transcriber::load(&folder).map_err(|error| error.to_string())?;
    lock(report).stage = "Transcribing".into();
    let phrases = transcriber
        .transcribe(&speech, None, &mut |fraction| lock(report).fraction = fraction, cancelled)
        .map_err(|error| error.to_string())?;
    Ok(studio_captions::words(&phrases).into_iter().map(|word| HeardWord { text: word.text, start: word.start, end: word.end }).collect())
}

/// Makes captions for `clip`, asking first if the model has to be downloaded.
pub fn generate(workspace: &mut Workspace, clip: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.caption_job.is_some() {
        window.push_notification("Captions are already being made. Wait for them, or cancel.", cx);
        return;
    }
    let Some(source) = workspace.state.caption_source(clip) else {
        window.push_notification("This layer has no sound to make captions from.", cx);
        return;
    };
    if studio_captions::model_present(&model_folder()) {
        start(workspace, clip, source, window, cx);
        return;
    }
    let target = cx.weak_entity();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let (target, source) = (target.clone(), source.clone());
        dialog
            .title("Download the captions model?")
            .description("Captions are made on this computer with OpenAI's Whisper (tiny) model. It is downloaded once from huggingface.co (about 150 MB) and kept in Studio's settings folder. Your sound is never uploaded.")
            .ok_text("Download and Make Captions")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let source = source.clone();
                target.update(cx, |this, cx| start(this, clip, source, window, cx)).ok();
                true
            })
    });
}

/// Starts the background job and checks on it until it ends.
fn start(workspace: &mut Workspace, clip: Id, source: (PathBuf, f64, f64), window: &mut Window, cx: &mut Context<Workspace>) {
    let cancel = Arc::new(AtomicBool::new(false));
    workspace.ui.caption_job = Some(CaptionJob { stage: "Starting".into(), fraction: 0.0, cancel: cancel.clone() });
    cx.notify();
    let report = Arc::new(Mutex::new(Report::default()));
    let thread_report = report.clone();
    let spawned = std::thread::Builder::new().name("captions".into()).spawn(move || {
        let outcome = run(source, &thread_report, &|| cancel.load(Ordering::Relaxed));
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
        workspace.ui.caption_job = None;
        window.push_notification(format!("Captions could not start: {error}").as_str(), cx);
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let (stage, fraction, outcome) = {
                let mut report = lock(&report);
                (report.stage.clone(), report.fraction, report.outcome.take())
            };
            let going = this.update_in(cx, |this, window, cx| {
                let Some(outcome) = outcome else {
                    if let Some(job) = this.ui.caption_job.as_mut() {
                        (job.stage, job.fraction) = (stage, fraction);
                        cx.notify();
                    }
                    return true;
                };
                this.ui.caption_job = None;
                match outcome {
                    Ok(words) if words.is_empty() => window.push_notification("No speech was heard in this clip.", cx),
                    Ok(words) => {
                        this.state.add_captions(clip, &words);
                    }
                    Err(message) if message == "cancelled" => {}
                    Err(message) => window.push_notification(format!("Captions could not be made: {message}").as_str(), cx),
                }
                cx.notify();
                false
            });
            if !matches!(going, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

/// The job's progress and a Cancel button, for the top bar.
pub fn status(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let job = workspace.ui.caption_job.as_ref()?;
    let percent = job.fraction * 100.0;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("Captions: {} · {}%", job.stage, percent as i64)))
            .child(div().w(px(100.0)).child(Progress::new("captions-progress").value(percent)))
            .child(Button::new("captions-cancel").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                if let Some(job) = this.ui.caption_job.as_ref() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                cx.notify();
            }))),
    )
}

fn heading(text: &'static str) -> gpui_kit::Div {
    div().pt_2().pb_1().text_color(hsla(color::TEXT)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(text)
}

/// The Captions panel: clips to caption, and caption looks.
pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let state = &workspace.state;
    let busy = workspace.ui.caption_job.is_some();
    let clips: Vec<(Id, String)> = state.composition().layers.iter().filter(|layer| state.caption_source(layer.id).is_some()).map(|layer| (layer.id, layer.name.clone())).collect();
    let rows = clips.iter().map(|(id, name)| {
        let id = *id;
        div()
            .h(px(36.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .rounded(px(6.0))
            .bg(hsla(color::INSET))
            .child(Icon::new(IconName::AudioLines).size(px(14.0)).text_color(hsla(color::MUTED)))
            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
            .child(
                Button::new(("caption-clip", id.0 as usize))
                    .small()
                    .primary()
                    .label("Caption")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| generate(this, id, window, cx))),
            )
    });
    // Looks apply to the selected caption layers.
    let selected: Vec<Id> = state.selected_layers.iter().copied().filter(|&layer| matches!(state.composition().layer(layer).map(|layer| &layer.content), Some(LayerContent::Captions { .. }))).collect();
    let can_style = !selected.is_empty();
    let looks = studio_core::captions::presets().into_iter().enumerate().map(|(index, (name, style))| {
        let fill = style.text.fill.map_or(0xffffff, |fill| fill.to_rgba_u32() >> 8);
        let highlight = style.highlight.to_rgba_u32() >> 8;
        let sample = if style.uppercase { "YOUR WORDS" } else { "Your words" };
        let (first, second) = sample.split_at(5);
        let box_color = style.background.map(|color| color.to_rgba_u32() >> 8);
        let (selected, look) = (selected.clone(), style.clone());
        div()
            .id(("caption-look", index))
            .w(px(128.0))
            .flex()
            .flex_col()
            .gap_1()
            .cursor_pointer()
            .when(!can_style, |card| card.opacity(0.5))
            .on_click(cx.listener(move |this, _, _, cx| {
                for &layer in &selected {
                    let look = look.clone();
                    this.state.edit_captions(layer, |current| *current = look);
                }
                cx.notify();
            }))
            .child(
                div()
                    .h(px(64.0))
                    .rounded(px(6.0))
                    .bg(hsla(color::PASTEBOARD))
                    .border_1()
                    .border_color(hsla(color::HAIRLINE))
                    .hover(|card| card.border_color(hsla(color::ACCENT)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .px_1p5()
                            .rounded(px(4.0))
                            .when_some(box_color, |words, color| words.bg(rgb(color)))
                            .flex()
                            .font_family(style.text.font_family.clone())
                            .font_weight(gpui_kit::FontWeight(f32::from(style.text.font_weight)))
                            .text_size(px(15.0))
                            .child(div().text_color(rgb(fill)).child(first.to_owned()))
                            .child(div().text_color(rgb(highlight)).child(second.to_owned())),
                    ),
            )
            .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::TEXT)).child(name))
            .child(div().text_size(px(10.0)).text_color(hsla(color::MUTED)).child(style.animation.name()))
            .test_support()
    });
    let body = div()
        .id("captions-list")
        .size_full()
        .overflow_y_scroll()
        .p_2()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("Captions are made on this computer from the speech in a clip, as a caption layer above it. Each word is timed, so the words can light up as they are spoken."))
        .child(heading("Clips with sound"))
        .when(clips.is_empty(), |list| list.child(super::chrome::hint("Add a clip with speech (video or audio) to the timeline to caption it.")))
        .children(rows)
        .child(heading("Looks"))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(if can_style { "Click a look to apply it to the selected captions." } else { "Select a caption layer, then click a look to restyle it." }))
        .child(div().pt_1().flex().flex_wrap().gap_2().children(looks));
    super::chrome::panel(PanelId::Captions, &["Captions"], None, body, workspace, cx)
}
