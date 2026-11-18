//! Object matte jobs (the Object Matte section in Properties), like After Effects' Roto Brush.
//! Brush strokes on a footage or image layer mark the object; each stroke remakes the mask for
//! its frame straight away, and Propagate Through Clip follows the object from the painted
//! frames through the rest of the clip, starting again from the strokes at every frame that
//! has some. The segmentation model (MobileSAM, about 40 MB) runs on this computer (see
//! `studio_cutout`); the first time, it is downloaded after asking. Work runs on a background
//! thread, with progress and Cancel in the top bar, and the masks appear in the viewer as they
//! are made.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder as _, px};
use studio_core::model::{Id, MatteEdges, MatteStroke};
use studio_cutout::{EdgeSettings, Guide, Mask, Segmenter, Stroke};

use crate::state::CutoutSource;
use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// An object matte job running in the background.
pub struct CutoutJob {
    pub layer: Id,
    /// Following the object through the clip, rather than making one frame's mask.
    pub tracking: bool,
    stage: String,
    fraction: f32,
    cancel: Arc<AtomicBool>,
}

/// What a job works from: the layer, its picture, the strokes and where to put the masks.
#[derive(Clone)]
struct Request {
    comp: Id,
    layer: Id,
    source: CutoutSource,
    strokes: Vec<MatteStroke>,
    /// The source frame the work starts from: the one just painted, or for propagating, the
    /// first painted frame.
    frame: i64,
    /// Masks still current from before (their folder and source frames, end exclusive, ending
    /// at `frame`), linked into the new folder so they keep showing.
    keep: Option<(PathBuf, (i64, i64))>,
    /// This frame's mask before the stroke, which the new strokes refine.
    prior: Option<PathBuf>,
    edges: MatteEdges,
    folder: PathBuf,
    track: bool,
    /// The folder holding the model.
    model: PathBuf,
    /// Rest the GPU between frames (Settings > AI Models), so the Mac stays cool and quiet.
    cooler: bool,
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
/// In the cooler mode, how long the GPU rests after each frame, as a share of the time the
/// frame took: 1.0 keeps it busy half the time.
const COOL_REST: f64 = 1.0;
/// Frames are compared at this size to spot ones that haven't changed.
const GLANCE: (u32, u32) = (32, 18);

/// The strokes painted on source frame `frame`, as the model takes them.
fn strokes_on(strokes: &[MatteStroke], frame: i64) -> Vec<Stroke> {
    strokes.iter().filter(|stroke| stroke.frame == frame).map(|stroke| Stroke { points: stroke.points.iter().map(|at| (at.x, at.y)).collect(), radius: stroke.radius, add: stroke.add }).collect()
}

/// Refine Detail (a percentage) as the guided filter's radius in mask pixels: 50% is the radius
/// the tool always used before it could be changed.
fn detail_radius(edges: &MatteEdges) -> f32 {
    edges.refine_detail / 100.0 * 12.0
}

/// Following an object through a clip, one frame at a time, from a painted frame outwards.
/// Frames with strokes start again from them; other frames follow the mask before them. The
/// model only looks at frames that are shown and have changed; the rest reuse the mask before
/// them, linked rather than copied, so they cost nothing.
struct Tracker<'a> {
    segmenter: &'a Segmenter,
    request: &'a Request,
    cancelled: &'a dyn Fn() -> bool,
    /// The last mask the model made (before edge refinement), the frame it was for, and a
    /// glance at that frame.
    mask: Mask,
    index: i64,
    glance: Vec<u8>,
}

impl Tracker<'_> {
    fn mask_path(&self, index: i64) -> PathBuf {
        self.request.folder.join(format!("{index}.png"))
    }

    /// Masks frame `index`. `picture` is `None` for a frame the composition doesn't show.
    fn frame(&mut self, index: i64, picture: Option<&Picture>) -> Result<(), String> {
        if (self.cancelled)() {
            return Err("cancelled".into());
        }
        let painted = strokes_on(&self.request.strokes, index);
        let glance = picture.map(glance);
        let unchanged = glance.as_ref().is_none_or(|glance| looks_the_same(glance, &self.glance));
        if unchanged && painted.is_empty() {
            let (from, to) = (self.mask_path(self.index), self.mask_path(index));
            return std::fs::hard_link(&from, &to)
                .or_else(|_| std::fs::copy(&from, &to).map(|_| ()))
                .map_err(|error| format!("could not save the mask: {error}"));
        }
        let Some(picture) = picture else { return Err("a painted frame isn't shown".into()) };
        let started = std::time::Instant::now();
        let prepared = self.segmenter.prepare(&picture.rgba, picture.width, picture.height).map_err(|error| error.to_string())?;
        let mask = if painted.is_empty() {
            self.segmenter.propagate(&prepared, &self.mask)
        } else {
            self.segmenter.segment_strokes(&prepared, &painted, Some(&self.mask))
        }
        .map_err(|error| error.to_string())?;
        // Reduce Chatter calms the soft edge against the frame before.
        let chatter = self.request.edges.reduce_chatter / 100.0;
        let calmed = studio_cutout::reduce_chatter(Some(&self.mask), &mask, None, chatter);
        finish(&calmed, picture, &self.request.edges).save(&self.mask_path(index)).map_err(|error| error.to_string())?;
        (self.mask, self.index, self.glance) = (mask, index, glance.unwrap_or_default());
        if self.request.cooler {
            rest(started.elapsed().mul_f64(COOL_REST), self.cancelled);
        }
        Ok(())
    }
}

/// A mask with Refine Detail applied, ready to save. (Feather, Contrast and Shift Edge are
/// applied when the picture is drawn, so they change at once.)
fn finish(mask: &Mask, picture: &Picture, edges: &MatteEdges) -> Mask {
    let guide = Guide::Rgba { pixels: &picture.rgba, width: picture.width, height: picture.height };
    mask.with_edges(&guide, &EdgeSettings { detail_radius: detail_radius(edges), ..EdgeSettings::default() })
}

/// Reads a saved mask, or `None` if it can't be read.
fn read_mask(path: &Path) -> Option<Mask> {
    let image = image::open(path).ok()?.to_luma8();
    let (width, height) = image.dimensions();
    Some(Mask { width, height, alpha: image.into_raw() })
}

/// A tiny grayscale copy of a picture, for comparing frames.
fn glance(picture: &Picture) -> Vec<u8> {
    let Some(image) = image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba.clone()) else { return Vec::new() };
    image::imageops::grayscale(&image::imageops::thumbnail(&image, GLANCE.0, GLANCE.1)).into_raw()
}

/// Whether two glances show the same picture: frozen frames, held titles, a still screen.
/// Strict, so any real movement is masked afresh.
fn looks_the_same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() || a.is_empty() {
        return false;
    }
    let differences = a.iter().zip(b).map(|(a, b)| a.abs_diff(*b));
    let (total, largest) = differences.fold((0u32, 0u8), |(total, largest), d| (total + u32::from(d), largest.max(d)));
    largest <= 4 && (total as f32 / a.len() as f32) < 0.75
}

/// Waits `time`, waking early if the job is cancelled.
fn rest(time: Duration, cancelled: &dyn Fn() -> bool) {
    let until = std::time::Instant::now() + time;
    while !cancelled() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20).min(until.saturating_duration_since(std::time::Instant::now())));
    }
}

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

/// Downloads the model, checked against the SHA-256 it was published with (see `download`).
pub fn fetch_model(folder: &Path, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("could not make the model folder: {error}"))?;
    crate::download::fetch_checked(studio_cutout::MODEL_URL, &folder.join(studio_cutout::MODEL_FILE), studio_cutout::MODEL_SHA256, "the cut-out model", cancelled)
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

/// The background work: download the model if needed, keep the masks still current, make the
/// starting frame's mask from its strokes, then (when propagating) follow the object forwards
/// to the clip's end and backwards to its start.
fn run(request: &Request, report: &Mutex<Report>, cancelled: &dyn Fn() -> bool) -> Result<(), String> {
    let model = &request.model;
    if !studio_cutout::model_present(model) {
        lock(report).stage = "Downloading the matte model".into();
        fetch_model(model, cancelled)?;
    }
    lock(report).stage = "Loading".into();
    let segmenter = Segmenter::load(model).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&request.folder).map_err(|error| format!("could not make the mask folder: {error}"))?;

    // Masks before the painted frame stay as they were: linked into this job's folder.
    let mut made = (request.frame, request.frame);
    if let Some((old, (start, end))) = &request.keep {
        for index in *start..*end {
            let (from, to) = (old.join(format!("{index}.png")), request.folder.join(format!("{index}.png")));
            std::fs::hard_link(&from, &to).or_else(|_| std::fs::copy(&from, &to).map(|_| ())).map_err(|error| format!("could not keep a mask: {error}"))?;
        }
        made.0 = *start;
    }

    lock(report).stage = "Making the matte".into();
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
    let first_picture = picture_at(request.frame)?;
    let prepared = segmenter.prepare(&first_picture.rgba, first_picture.width, first_picture.height).map_err(|error| error.to_string())?;
    // The mask this frame had refines with the new strokes, if it is the right size.
    let prior = request.prior.as_deref().and_then(read_mask).filter(|mask| (mask.width, mask.height) == (first_picture.width, first_picture.height));
    let first = segmenter.segment_strokes(&prepared, &strokes_on(&request.strokes, request.frame), prior.as_ref()).map_err(|error| error.to_string())?;
    if cancelled() {
        return Err("cancelled".into());
    }
    finish(&first, &first_picture, &request.edges).save(&request.folder.join(format!("{}.png", request.frame))).map_err(|error| error.to_string())?;
    made.1 = request.frame + 1;
    lock(report).made = Some(made);
    let (Some((start, end)), true) = (request.source.frames, request.track) else { return Ok(()) };

    let total = (end - start).max(1) as f32;
    let progress = |made: (i64, i64), stage: &str| {
        let mut report = lock(report);
        report.made = Some(made);
        report.fraction = (made.1 - made.0) as f32 / total;
        report.stage = stage.into();
    };
    let shown = |index: i64| request.source.shown.as_ref().is_none_or(|shown| shown.contains(&index));
    let start_glance = glance(&first_picture);
    let new_tracker = |mask: Mask| Tracker { segmenter: &segmenter, request, cancelled, mask, index: request.frame, glance: start_glance.clone() };
    // Forwards from the starting frame, starting again at each later painted frame.
    let mut tracker = new_tracker(first.clone());
    for index in request.frame + 1..end {
        let painted = !strokes_on(&request.strokes, index).is_empty();
        let picture = if shown(index) || painted { Some(picture_at(index)?) } else { None };
        tracker.frame(index, picture.as_ref())?;
        made.1 = index + 1;
        progress(made, "Propagating forwards");
    }
    // Backwards to the start, a run of frames at a time.
    let mut tracker = new_tracker(first);
    let mut run_end = request.frame;
    while run_end > start {
        let run_start = (run_end - BACKWARD_CHUNK).max(start);
        let pictures: Vec<Option<Picture>> = (run_start..run_end)
            .map(|index| if shown(index) || !strokes_on(&request.strokes, index).is_empty() { picture_at(index).map(Some) } else { Ok(None) })
            .collect::<Result<_, _>>()?;
        for (offset, picture) in pictures.iter().enumerate().rev() {
            let index = run_start + offset as i64;
            tracker.frame(index, picture.as_ref())?;
            made.0 = index;
            progress(made, "Propagating backwards");
        }
        run_end = run_start;
    }
    Ok(())
}

/// Where object matte masks are kept: `Cutouts`, beside the proxies in Studio's cache.
pub fn cutouts_folder(workspace: &Workspace) -> PathBuf {
    let proxies = workspace.render.proxies.cache_folder();
    proxies.parent().map_or_else(|| proxies.join("Cutouts"), |root| root.join("Cutouts"))
}

/// A new folder for one job's masks, under Studio's cache. Each job writes its own folder, so a
/// job that was cancelled (or overtaken by a newer stroke) can never overwrite newer masks.
fn new_mask_folder(workspace: &Workspace, layer: Id) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (layer.0, std::time::SystemTime::now(), std::process::id()).hash(&mut hasher);
    cutouts_folder(workspace).join(format!("{:016x}", hasher.finish()))
}

/// Remakes the mask for the frame at the playhead of `layer` (after a stroke), or with `track`
/// follows the object from the first painted frame through the whole clip. Any job already
/// running is stopped: its strokes are out of date. Asks first if the model has to be
/// downloaded. A matte with no strokes left just keeps the masks it has.
pub fn make_masks(workspace: &mut Workspace, layer: Id, track: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(source) = workspace.state.cutout_source(layer) else { return };
    let Some(cut) = workspace.state.composition().layer(layer).and_then(|target| target.cutout.clone()).filter(|cut| !cut.strokes.is_empty()) else { return };
    let Some(here) = workspace.state.cutout_frame_at_playhead(layer) else { return };
    if let Some(job) = workspace.ui.cutout_job.take() {
        job.cancel.store(true, Ordering::Relaxed);
    }
    // Propagating starts from the first painted frame and remakes everything; a stroke remakes
    // its own frame and keeps the masks just before it.
    let (frame, keep) = if track {
        (cut.base_frames().first().copied().unwrap_or(here), None)
    } else {
        let keep = cut.masks.clone().zip(cut.frames).filter(|(_, (_, end))| *end == here);
        (here, keep)
    };
    // The mask the frame showed until now (if it had one of its own) refines with the strokes.
    let prior = cut.masks.as_ref().filter(|_| !track).map(|folder| folder.join(format!("{frame}.png"))).filter(|path| path.exists());
    let cooler = workspace.ui.settings.cooler_background_removal;
    let request = Request {
        comp: workspace.state.active_comp,
        layer,
        source,
        strokes: cut.strokes,
        frame,
        keep,
        prior,
        edges: cut.edges,
        folder: new_mask_folder(workspace, layer),
        track,
        model: model_folder(),
        cooler,
    };
    if studio_cutout::model_present(&model_folder()) {
        start(workspace, request, window, cx);
        return;
    }
    let target = cx.weak_entity();
    let megabytes = studio_cutout::MODEL_BYTES / 1_000_000;
    crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| {
        let (target, request) = (target.clone(), request.clone());
        dialog
            .title("Download the object matte model?")
            .description(format!("Objects are matted on this computer with the MobileSAM model. It is downloaded once from huggingface.co (about {megabytes} MB) and kept in Omina Motion's settings folder. Your footage is never uploaded."))
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
        window.push_notification(format!("The object matte could not start: {error}").as_str(), cx);
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
                        this.state.record_cutout_masks(request.comp, request.layer, &request.strokes, request.folder.clone(), frames);
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
                    window.push_notification(format!("The object matte could not be made: {message}").as_str(), cx);
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

/// Starts or stops painting `layer`'s object matte in the viewer. Starting shows the Selection
/// view (unless another view was chosen), so each stroke shows at once what it added or took
/// away.
pub fn toggle_picking(workspace: &mut Workspace, layer: Id) {
    if workspace.ui.cutout_pick == Some(layer) {
        workspace.ui.cutout_pick = None;
        return;
    }
    workspace.ui.cutout_pick = Some(layer);
    workspace.ui.key_view.get_or_insert(studio_core::key_view::KeyView::Selection);
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
    let label = if job.tracking { format!("Object matte: {} · {}%", job.stage, percent as i64) } else { format!("Object matte: {}", job.stage) };
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(label))
            .when(job.tracking, |row| row.child(div().w(px(100.0)).child(Progress::new("cutout-progress").value(percent))))
            .child(Button::new("cutout-cancel").ghost().small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
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
        // Painted in the middle of the clip, on the disc (which starts at x = 150 and moves 150 px a second).
        let middle = frames / 2;
        let disc_x = (150.0 + middle as f32 / 12.0 * 150.0) / 640.0;
        let request = Request {
            comp: Id(1),
            layer: Id(2),
            source: CutoutSource { path: clip.into(), frames: Some((0, frames)), shown: None },
            strokes: vec![MatteStroke { frame: middle, points: vec![Vec2::new(disc_x, 0.5)], radius: 0.02, add: true }],
            frame: middle,
            keep: None,
            prior: None,
            edges: MatteEdges::default(),
            folder: folder.clone(),
            track: true,
            model: model.into(),
            cooler: false,
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
