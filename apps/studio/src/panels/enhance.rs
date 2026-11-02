//! Enhance Speech jobs: making the cleaned copy of a clip's sound in the background (see
//! `studio_enhance`). A layer whose settings ask for a copy that doesn't exist yet is picked up
//! by `keep_going`, one layer at a time; copies are kept in Studio's cache under `Enhanced`,
//! named after the source file and the settings, so the same settings never clean twice.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use studio_core::model::{EnhanceModel, Id, SpeechEnhance};

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// The cleaned copy being made.
pub struct EnhanceJob {
    pub layer: Id,
    pub fraction: f32,
    pub stage: &'static str,
    settings: SpeechEnhance,
    cancel: Arc<AtomicBool>,
}

/// What the background thread reports.
#[derive(Default)]
struct Report {
    stage: &'static str,
    fraction: f32,
    outcome: Option<Result<(), String>>,
}

/// How often the editor checks on the job.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Where cleaned copies are kept: `Enhanced`, beside the proxies in Studio's cache.
pub fn enhanced_folder(workspace: &Workspace) -> PathBuf {
    let proxies = workspace.render.proxies.cache_folder();
    proxies.parent().map_or_else(|| proxies.join("Enhanced"), |root| root.join("Enhanced"))
}

/// The file a cleaned copy of `source` with `settings` goes in: named after the source's path,
/// size and change time (so a changed file is cleaned again) and the settings.
fn copy_path(workspace: &Workspace, source: &Path, settings: &SpeechEnhance) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    if let Ok(metadata) = std::fs::metadata(source) {
        metadata.len().hash(&mut hasher);
        metadata.modified().ok().hash(&mut hasher);
    }
    format!("{:?}{}{}{}{}{}", settings.model, settings.strength, settings.remove_hum, settings.voice_eq, settings.de_ess, settings.even_out).hash(&mut hasher);
    enhanced_folder(workspace).join(format!("{:016x}.wav", hasher.finish()))
}

/// The cleanup steps a layer's settings ask for.
fn steps(settings: &SpeechEnhance) -> studio_enhance::Settings {
    studio_enhance::Settings {
        denoiser: match settings.model {
            EnhanceModel::Light => Some(studio_enhance::Denoiser::Light),
            EnhanceModel::Studio => Some(studio_enhance::Denoiser::Studio),
            EnhanceModel::Off => None,
        },
        strength: (settings.strength / 100.0).clamp(0.0, 1.0),
        remove_hum: settings.remove_hum,
        voice_eq: settings.voice_eq,
        de_ess: settings.de_ess,
        even_out: settings.even_out,
    }
}

/// Starts on the next layer waiting for a cleaned copy, unless one is being made. Copies that
/// already exist (the same settings again, or after an undo) are used straight away.
pub fn keep_going(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.enhance_job.is_some() {
        return;
    }
    if workspace.state.forget_missing_cleaned() {
        cx.notify();
    }
    let comp = workspace.state.active_comp;
    for (layer, settings) in workspace.state.waiting_for_enhance() {
        if workspace.ui.enhance_skipped.contains(&(layer, settings.clone())) {
            continue;
        }
        let Some(source) = workspace.state.sound_source(layer) else { continue };
        let path = copy_path(workspace, &source, &settings);
        if path.is_file() {
            workspace.state.record_cleaned(comp, layer, &settings, path);
            cx.notify();
            continue;
        }
        start(workspace, comp, layer, settings, source, path, window, cx);
        return;
    }
}

/// Stops the running job; the layer keeps playing its original sound, and isn't tried again
/// until its settings change.
pub fn cancel(workspace: &mut Workspace) {
    if let Some(job) = workspace.ui.enhance_job.take() {
        job.cancel.store(true, Ordering::Relaxed);
        workspace.ui.enhance_skipped.push((job.layer, job.settings));
    }
}

fn lock(report: &Mutex<Report>) -> std::sync::MutexGuard<'_, Report> {
    report.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Reads all of `source`'s sound at the cleanup's sample rate, as one list per channel.
fn read_sound(source: &Path, cancelled: &dyn Fn() -> bool) -> Result<Option<Vec<Vec<f32>>>, String> {
    let channels = studio_media::CHANNELS;
    let mut decoder = studio_media::AudioDecoder::open(source, studio_enhance::RATE).map_err(|error| error.to_string())?;
    let mut split = vec![Vec::new(); channels];
    let mut block = vec![0.0f32; 48_000 * channels];
    loop {
        if cancelled() {
            return Ok(None);
        }
        let frames = decoder.read(&mut block).map_err(|error| error.to_string())?;
        for frame in block[..frames * channels].chunks_exact(channels) {
            for (channel, &sample) in split.iter_mut().zip(frame) {
                channel.push(sample);
            }
        }
        if frames * channels < block.len() {
            return Ok(Some(split));
        }
    }
}

/// The background work: read the sound, clean it, save it.
fn run(source: &Path, destination: &Path, settings: studio_enhance::Settings, report: &Mutex<Report>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<(), String> {
    lock(report).stage = "Reading the sound";
    let Some(sound) = read_sound(source, cancelled)? else { return Err("cancelled".into()) };
    lock(report).stage = "Cleaning up";
    let progress = |done: f32| lock(report).fraction = done;
    let Some(cleaned) = studio_enhance::enhance(&sound, settings, &progress, cancelled).map_err(|error| error.0)? else { return Err("cancelled".into()) };
    lock(report).stage = "Saving";
    if let Some(folder) = destination.parent() {
        std::fs::create_dir_all(folder).map_err(|error| format!("could not make the cache folder: {error}"))?;
    }
    studio_enhance::write_wav(destination, &cleaned, studio_enhance::RATE).map_err(|error| format!("could not save the cleaned sound: {error}"))
}

#[allow(clippy::too_many_arguments)]
fn start(workspace: &mut Workspace, comp: Id, layer: Id, settings: SpeechEnhance, source: PathBuf, path: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    let cancel = Arc::new(AtomicBool::new(false));
    workspace.ui.enhance_job = Some(EnhanceJob { layer, fraction: 0.0, stage: "Starting", settings: settings.clone(), cancel: cancel.clone() });
    cx.notify();
    let report = Arc::new(Mutex::new(Report::default()));
    let (thread_report, thread_cancel, thread_path, steps) = (report.clone(), cancel.clone(), path.clone(), steps(&settings));
    let spawned = std::thread::Builder::new().name("enhance".into()).spawn(move || {
        let outcome = run(&source, &thread_path, steps, &thread_report, &|| thread_cancel.load(Ordering::Relaxed));
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
        workspace.ui.enhance_job = None;
        window.push_notification(format!("Enhance Speech could not start: {error}").as_str(), cx);
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let (stage, fraction, outcome) = {
                let mut report = lock(&report);
                (report.stage, report.fraction, report.outcome.take())
            };
            let going = this.update_in(cx, |this, window, cx| {
                let ours = this.ui.enhance_job.as_ref().is_some_and(|job| Arc::ptr_eq(&job.cancel, &cancel));
                let Some(outcome) = outcome else {
                    if let Some(job) = this.ui.enhance_job.as_mut().filter(|_| ours) {
                        (job.stage, job.fraction) = (stage, fraction);
                        cx.notify();
                    }
                    return true;
                };
                if ours {
                    this.ui.enhance_job = None;
                }
                match outcome {
                    Ok(()) => {
                        // Settings changed meanwhile make it stale; the next job picks up the new ones.
                        this.state.record_cleaned(comp, layer, &settings, path.clone());
                    }
                    Err(message) if !cancel.load(Ordering::Relaxed) => {
                        this.ui.enhance_skipped.push((layer, settings.clone()));
                        window.push_notification(format!("The sound could not be enhanced: {message}").as_str(), cx);
                    }
                    Err(_) => {}
                }
                keep_going(this, window, cx);
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
    let job = workspace.ui.enhance_job.as_ref()?;
    let name = workspace.state.composition().layer(job.layer).map_or_else(String::new, |layer| layer.name.clone());
    let percent = job.fraction * 100.0;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("Enhancing {name}: {} · {}%", job.stage, percent as i64)))
            .child(div().w(px(100.0)).child(Progress::new("enhance-progress").value(percent)))
            .child(Button::new("enhance-cancel").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                cancel(this);
                cx.notify();
            }))),
    )
}
