//! Background removal jobs (the Remove Background section in Properties). Clicks on a footage or
//! image layer pick out a subject; each click makes a mask for that frame straight away, and
//! Track Through Clip follows the subject through the rest of the clip. The segmentation model
//! (MobileSAM, about 40 MB) runs on this computer (see `studio_cutout`); the first time, it is
//! downloaded after asking. Work runs on a background thread, with progress and Cancel in the
//! top bar, and the masks appear in the viewer as they are made.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder as _, px};
use studio_core::model::{CutoutClick, Id};
use studio_cutout::{Click, Mask, Segmenter};

use crate::state::CutoutSource;
use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// A background-removal job running in the background.
pub struct CutoutJob {
    pub layer: Id,
    /// Following the subject through the clip, rather than making one frame's mask.
    pub tracking: bool,
    stage: String,
    fraction: f32,
    cancel: Arc<AtomicBool>,
}

/// What a job works from: the layer, its picture, the clicks and where to put the masks.
#[derive(Clone)]
struct Request {
    comp: Id,
    layer: Id,
    source: CutoutSource,
    clicks: Vec<CutoutClick>,
    /// The source frame the clicks are on.
    frame: i64,
    folder: PathBuf,
    track: bool,
    /// The folder holding the model.
    model: PathBuf,
}

/// What the background thread reports.
#[derive(Default)]
struct Report {
    stage: String,
    fraction: f32,
    /// The source frames with masks so far (start, end exclusive).
    made: Option<(i64, i64)>,
    outcome: Option<Result<(), String>>,
}

/// How often the editor checks on the job.
const POLL_INTERVAL: Duration = Duration::from_millis(150);
/// Frames decoded together when tracking backwards through a clip: video decodes quickly
/// forwards and slowly backwards, so each run is decoded forwards, then followed in reverse.
const BACKWARD_CHUNK: i64 = 24;

/// Where the segmentation model is kept: Studio's settings folder, under `Models/mobile-sam`.
/// Headless tests look in an empty folder of their own, so they never run the real model.
pub fn model_folder() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("studio-cutout-model-test-{}", std::process::id()));
    }
    crate::app_settings::folder().join("Models").join("mobile-sam")
}

fn lock(report: &Mutex<Report>) -> std::sync::MutexGuard<'_, Report> {
    report.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Downloads the model with the system's `curl` to a temporary name, checks it is the file
/// published (its SHA-256), and only then puts it in place.
fn download_model(folder: &Path, report: &Mutex<Report>) -> Result<(), String> {
    use sha2::Digest as _;
    std::fs::create_dir_all(folder).map_err(|error| format!("could not make the model folder: {error}"))?;
    lock(report).stage = "Downloading the cut-out model".into();
    let destination = folder.join(studio_cutout::MODEL_FILE);
    let partial = destination.with_extension("part");
    let status = std::process::Command::new("curl").args(["-sSfL", "-o"]).arg(&partial).arg(studio_cutout::MODEL_URL).status().map_err(|error| format!("could not start the download: {error}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err("the cut-out model could not be downloaded. Check the internet connection and try again.".into());
    }
    let bytes = std::fs::read(&partial).map_err(|error| format!("could not read the download: {error}"))?;
    let digest: String = sha2::Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    if digest != studio_cutout::MODEL_SHA256 {
        let _ = std::fs::remove_file(&partial);
        return Err("the downloaded cut-out model was not the expected file, so it was not used.".into());
    }
    std::fs::rename(&partial, &destination).map_err(|error| format!("could not save the model: {error}"))
}

/// One source picture as RGBA: an image file, or a decoded video frame.
struct Picture {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

fn read_image(path: &Path) -> Result<Picture, String> {
    let image = image::open(path).map_err(|error| format!("could not read the image: {error}"))?.to_rgba8();
    let (width, height) = image.dimensions();
    Ok(Picture { rgba: image.into_raw(), width, height })
}

/// The background work: download the model if needed, make the clicked frame's mask, then (when
/// tracking) follow the subject forwards to the clip's end and backwards to its start.
fn run(request: &Request, report: &Mutex<Report>, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let model = &request.model;
    if !studio_cutout::model_present(model) {
        download_model(model, report)?;
    }
    lock(report).stage = "Loading".into();
    let segmenter = Segmenter::load(model).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&request.folder).map_err(|error| format!("could not make the mask folder: {error}"))?;
    let save = |mask: &Mask, index: i64| mask.save(&request.folder.join(format!("{index}.png"))).map_err(|error| error.to_string());
    let clicks: Vec<Click> = request.clicks.iter().map(|click| Click { x: click.at.x, y: click.at.y, keep: click.keep }).collect();
    let removes: Vec<Click> = clicks.iter().filter(|click| !click.keep).copied().collect();
    let segment = |picture: &Picture| segmenter.prepare(&picture.rgba, picture.width, picture.height).map_err(|error| error.to_string());

    lock(report).stage = "Finding the subject".into();
    let mut decoder = match request.source.frames {
        Some(_) => Some(studio_media::VideoDecoder::open(&request.source.path, studio_cutout::WORK_SIZE, true).map_err(|error| error.to_string())?),
        None => None,
    };
    let mut picture_at = |index: i64| -> Result<Picture, String> {
        match decoder.as_mut() {
            Some(decoder) => {
                let frame = decoder.frame(index).map_err(|error| error.to_string())?;
                Ok(Picture { rgba: frame.rgba, width: frame.width, height: frame.height })
            }
            None => read_image(&request.source.path),
        }
    };
    let first = segmenter.segment(&segment(&picture_at(request.frame)?)?, &clicks).map_err(|error| error.to_string())?;
    if cancelled() {
        return Err("cancelled".into());
    }
    save(&first, request.frame)?;
    lock(report).made = Some((request.frame, request.frame + 1));
    let (Some((start, end)), true) = (request.source.frames, request.track) else { return Ok(()) };

    let total = (end - start).max(1) as f32;
    let mut made = (request.frame, request.frame + 1);
    let progress = |made: (i64, i64), stage: &str| {
        let mut report = lock(report);
        report.made = Some(made);
        report.fraction = (made.1 - made.0) as f32 / total;
        report.stage = stage.into();
    };
    // Forwards from the clicked frame.
    let mut previous = first.clone();
    for index in request.frame + 1..end {
        if cancelled() {
            return Err("cancelled".into());
        }
        let mask = segmenter.follow(&segment(&picture_at(index)?)?, &previous, &removes).map_err(|error| error.to_string())?;
        save(&mask, index)?;
        made.1 = index + 1;
        progress(made, "Tracking forwards");
        previous = mask;
    }
    // Backwards to the start, a run of frames at a time.
    let mut previous = first;
    let mut run_end = request.frame;
    while run_end > start {
        let run_start = (run_end - BACKWARD_CHUNK).max(start);
        let pictures: Vec<Picture> = (run_start..run_end).map(&mut picture_at).collect::<Result<_, _>>()?;
        for (offset, picture) in pictures.iter().enumerate().rev() {
            if cancelled() {
                return Err("cancelled".into());
            }
            let index = run_start + offset as i64;
            let mask = segmenter.follow(&segment(picture)?, &previous, &removes).map_err(|error| error.to_string())?;
            save(&mask, index)?;
            made.0 = index;
            progress(made, "Tracking backwards");
            previous = mask;
        }
        run_end = run_start;
    }
    Ok(())
}

/// A new folder for one job's masks, under Studio's cache. Each job writes its own folder, so a
/// job that was cancelled (or overtaken by a newer click) can never overwrite newer masks.
fn new_mask_folder(workspace: &Workspace, layer: Id) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (layer.0, std::time::SystemTime::now(), std::process::id()).hash(&mut hasher);
    let cache = workspace.render.proxies.cache_folder();
    cache.parent().unwrap_or(cache).join("Cutouts").join(format!("{:016x}", hasher.finish()))
}

/// Makes the mask for the clicked frame of `layer` (after a click), or follows the subject
/// through the whole clip with `track`. Any job already running is stopped: its clicks are out
/// of date. Asks first if the model has to be downloaded.
pub fn make_masks(workspace: &mut Workspace, layer: Id, track: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(source) = workspace.state.cutout_source(layer) else { return };
    let Some(cut) = workspace.state.composition().layer(layer).and_then(|target| target.cutout.clone()).filter(|cut| !cut.clicks.is_empty()) else { return };
    if let Some(job) = workspace.ui.cutout_job.take() {
        job.cancel.store(true, Ordering::Relaxed);
    }
    let request = Request { comp: workspace.state.active_comp, layer, source, clicks: cut.clicks, frame: cut.frame, folder: new_mask_folder(workspace, layer), track, model: model_folder() };
    if studio_cutout::model_present(&model_folder()) {
        start(workspace, request, window, cx);
        return;
    }
    let target = cx.weak_entity();
    let megabytes = studio_cutout::MODEL_BYTES / 1_000_000;
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let (target, request) = (target.clone(), request.clone());
        dialog
            .title("Download the background removal model?")
            .description(format!("Backgrounds are removed on this computer with the MobileSAM model. It is downloaded once from huggingface.co (about {megabytes} MB) and kept in Studio's settings folder. Your footage is never uploaded."))
            .ok_text("Download and Continue")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let request = request.clone();
                target.update(cx, |this, cx| start(this, request, window, cx)).ok();
                true
            })
    });
}

/// Starts the background job and checks on it until it ends, showing masks as they are made.
fn start(workspace: &mut Workspace, request: Request, window: &mut Window, cx: &mut Context<Workspace>) {
    let cancel = Arc::new(AtomicBool::new(false));
    workspace.ui.cutout_job = Some(CutoutJob { layer: request.layer, tracking: request.track, stage: "Starting".into(), fraction: 0.0, cancel: cancel.clone() });
    cx.notify();
    let report = Arc::new(Mutex::new(Report::default()));
    let (thread_report, thread_request, thread_cancel) = (report.clone(), request.clone(), cancel.clone());
    let spawned = std::thread::Builder::new().name("cutout".into()).spawn(move || {
        let outcome = run(&thread_request, &thread_report, &|| thread_cancel.load(Ordering::Relaxed));
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
        workspace.ui.cutout_job = None;
        window.push_notification(format!("Background removal could not start: {error}").as_str(), cx);
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        let mut recorded = None;
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let (stage, fraction, made, outcome) = {
                let mut report = lock(&report);
                (report.stage.clone(), report.fraction, report.made, report.outcome.take())
            };
            let going = this.update_in(cx, |this, window, cx| {
                // Masks made so far show straight away.
                if made.is_some() && made != recorded {
                    recorded = made;
                    if let Some(frames) = made {
                        this.state.record_cutout_masks(request.comp, request.layer, &request.clicks, request.folder.clone(), frames);
                    }
                    cx.notify();
                }
                let ours = this.ui.cutout_job.as_ref().is_some_and(|job| Arc::ptr_eq(&job.cancel, &cancel));
                let Some(outcome) = outcome else {
                    if let Some(job) = this.ui.cutout_job.as_mut().filter(|_| ours) {
                        (job.stage, job.fraction) = (stage, fraction);
                        cx.notify();
                    }
                    return true;
                };
                // A newer job may have taken over; only this job's own entry is cleared.
                if ours {
                    this.ui.cutout_job = None;
                }
                if let Err(message) = outcome
                    && !cancel.load(Ordering::Relaxed)
                {
                    window.push_notification(format!("The background could not be removed: {message}").as_str(), cx);
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

/// Stops the running job, keeping the masks it already made.
pub fn cancel(workspace: &mut Workspace) {
    if let Some(job) = workspace.ui.cutout_job.take() {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// The job's progress and a Cancel button, for the top bar.
pub fn status(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let job = workspace.ui.cutout_job.as_ref()?;
    let percent = job.fraction * 100.0;
    let label = if job.tracking { format!("Tracking cut-out: {} · {}%", job.stage, percent as i64) } else { format!("Cut-out: {}", job.stage) };
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(label))
            .when(job.tracking, |row| row.child(div().w(px(100.0)).child(Progress::new("cutout-progress").value(percent))))
            .child(Button::new("cutout-cancel").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                cancel(this);
                cx.notify();
            }))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::animation::Vec2;

    /// Tracks a real clip with the real model. It needs the downloaded model and a clip of an
    /// orange disc moving left to right across a dark frame, so it only runs when asked:
    /// `CUTOUT_MODEL=<model folder> CUTOUT_CLIP=<clip> cargo test -p studio --release -- --ignored tracks_a_real_clip`
    #[test]
    #[ignore = "needs the downloaded model and a clip; set CUTOUT_MODEL and CUTOUT_CLIP"]
    fn tracks_a_real_clip() {
        let (model, clip) = (std::env::var("CUTOUT_MODEL").unwrap(), std::env::var("CUTOUT_CLIP").unwrap());
        let folder = std::env::temp_dir().join(format!("studio-cutout-track-{}", std::process::id()));
        let frames = studio_media::VideoDecoder::open(Path::new(&clip), 640, false).unwrap().frame_count();
        // Clicked in the middle of the clip, on the disc (which starts at x = 150 and moves 150 px a second).
        let middle = frames / 2;
        let disc_x = (150.0 + middle as f32 / 12.0 * 150.0) / 640.0;
        let request = Request {
            comp: Id(1),
            layer: Id(2),
            source: CutoutSource { path: clip.into(), frames: Some((0, frames)) },
            clicks: vec![CutoutClick { at: Vec2::new(disc_x, 0.5), keep: true }],
            frame: middle,
            folder: folder.clone(),
            track: true,
            model: model.into(),
        };
        let report = Mutex::new(Report::default());
        let started = std::time::Instant::now();
        run(&request, &report, &|| false).unwrap();
        println!("{frames} frames in {:?}", started.elapsed());
        assert_eq!(lock(&report).made, Some((0, frames)), "every frame has a mask");
        // On every frame, the disc's center is kept and the far corners are not.
        for index in 0..frames {
            let mask = image::open(folder.join(format!("{index}.png"))).unwrap().to_luma8();
            let scale = mask.width() as f32 / 640.0;
            let x = ((150.0 + index as f32 / 12.0 * 150.0) * scale) as u32;
            let y = (180.0 * scale) as u32;
            assert!(mask.get_pixel(x.min(mask.width() - 1), y)[0] > 200, "frame {index}: the disc is kept");
            assert!(mask.get_pixel(5, 5)[0] < 50 && mask.get_pixel(mask.width() - 5, mask.height() - 5)[0] < 50, "frame {index}: the background is not");
        }
        std::fs::remove_dir_all(&folder).ok();
    }
}
