//! Editing helpers that look at a clip's sound or picture first, running in the background
//! with progress and Cancel in the top bar:
//!
//! - **Remove Silences:** cuts the pauses out of talking footage (jump cuts).
//! - **Mark Beats:** puts a marker on every beat of a song, for cutting to the music.
//! - **Track Motion:** pins a layer to a point that moves in a video.
//! - **Split at Scene Cuts / Mark Scene Cuts:** finds the hard cuts in a video clip (see
//!   `studio_media::find_scene_cuts`) and splits the clip into its shots, or marks them.
//! - **Stabilize:** steadies shaky footage by moving the layer against the camera's motion.
//! - **Auto Reframe:** makes a version of a clip in another shape (9:16 from 16:9, say) that
//!   follows the subject, using the background-removal model to find it.
//!
//! Each runs `work` on a thread of its own, then applies its result to the project in one step
//! (one undo).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use studio_core::animation::{Animated, Easing, Keyframe, Vec2};
use studio_core::model::{Id, LayerContent};
use studio_core::templates::Format;

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// A job running in the background.
pub struct AnalysisJob {
    label: &'static str,
    /// How far along, 0–1, as the bits of an `f32` (shared with the thread).
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
}

/// How often the editor checks on the job.
const POLL_INTERVAL: Duration = Duration::from_millis(150);

type Outcome<T> = Arc<Mutex<Option<Result<T, String>>>>;

/// Runs `work` on a thread, then `done` with its result. One job runs at a time.
fn run<T: Send + 'static>(
    workspace: &mut Workspace,
    label: &'static str,
    work: impl FnOnce(&dyn Fn(f32), &dyn Fn() -> bool) -> Result<T, String> + Send + 'static,
    done: impl FnOnce(&mut Workspace, T, &mut Window, &mut Context<Workspace>) + 'static,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if workspace.ui.analysis_job.is_some() {
        window.push_notification("Another analysis is running. Wait for it, or cancel it.", cx);
        return;
    }
    let (progress, cancel) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicBool::new(false)));
    workspace.ui.analysis_job = Some(AnalysisJob { label, progress: progress.clone(), cancel: cancel.clone() });
    cx.notify();
    let outcome: Outcome<T> = Arc::new(Mutex::new(None));
    let (thread_outcome, thread_progress, thread_cancel) = (outcome.clone(), progress, cancel.clone());
    let spawned = std::thread::Builder::new().name(label.into()).spawn(move || {
        let report = |fraction: f32| thread_progress.store(fraction.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        let result = work(&report, &|| thread_cancel.load(Ordering::Relaxed));
        *thread_outcome.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(result);
    });
    if let Err(error) = spawned {
        workspace.ui.analysis_job = None;
        window.push_notification(format!("{label} could not start: {error}").as_str(), cx);
        return;
    }
    cx.spawn_in(window, async move |this, cx| {
        let mut done = Some(done);
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let finished = outcome.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            let going = this.update_in(cx, |this, window, cx| {
                let Some(result) = finished else {
                    cx.notify(); // Progress moved.
                    return true;
                };
                this.ui.analysis_job = None;
                match result {
                    _ if cancel.load(Ordering::Relaxed) => {}
                    Ok(value) => {
                        if let Some(done) = done.take() {
                            this.state.settle_drags();
                            done(this, value, window, cx);
                        }
                    }
                    Err(message) => window.push_notification(format!("{label}: {message}").as_str(), cx),
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
    let job = workspace.ui.analysis_job.as_ref()?;
    let percent = f32::from_bits(job.progress.load(Ordering::Relaxed)) * 100.0;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("{} · {}%", job.label, percent as i64)))
            .child(div().w(px(100.0)).child(Progress::new("analysis-progress").value(percent)))
            .child(Button::new("analysis-cancel").ghost().small().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                if let Some(job) = this.ui.analysis_job.take() {
                    job.cancel.store(true, Ordering::Relaxed);
                }
                cx.notify();
            }))),
    )
}

/// The media file behind a footage or audio layer.
fn media_path(workspace: &Workspace, layer: Id) -> Option<PathBuf> {
    let found = workspace.state.composition().layer(layer)?;
    let (LayerContent::Footage { item } | LayerContent::Audio { item }) = found.content else { return None };
    workspace.state.project.item(item)?.path.clone()
}

/// Cuts the pauses out of a clip with sound, closing up what follows.
pub fn remove_silences(workspace: &mut Workspace, layer: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(path) = media_path(workspace, layer) else { return };
    let comp = workspace.state.active_comp;
    run(
        workspace,
        "Finding pauses",
        move |progress, _| {
            progress(0.2);
            studio_media::audio_peaks(&path, 100).map_err(|error| error.to_string())
        },
        move |this, peaks, window, cx| {
            let spans = studio_core::silence::quiet_spans(&peaks.values, peaks.per_second, Default::default());
            let cut = studio_core::silence::remove_spans(&mut this.state.project, comp, layer, &spans);
            let message = match cut {
                0 => "No pauses long enough to cut were found.".to_owned(),
                1 => "Cut 1 pause.".to_owned(),
                n => format!("Cut {n} pauses."),
            };
            window.push_notification(message.as_str(), cx);
        },
        window,
        cx,
    );
}

/// How finely sound is compared when lining clips up (buckets a second: 10 ms each, well
/// under a frame), and how far apart two recordings may have started and still be matched.
const SYNC_BUCKETS_PER_SECOND: u32 = 100;
const SYNC_MAX_SECONDS: usize = 120;

/// Lines the selected clips up by their sound, for multicam: each is moved so that what it
/// recorded plays at the same moment as in the first one selected, which stays where it is.
/// Works on clips at normal speed that have sound; a clip whose sound doesn't match is left
/// where it is and named.
pub fn sync_by_sound(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let composition = workspace.state.composition();
    let clips: Vec<(Id, String, PathBuf)> = workspace
        .state
        .selected_layers
        .iter()
        .filter_map(|&id| composition.layer(id))
        .filter(|layer| !layer.switches.locked && layer.time.is_normal() && workspace.state.has_sound(layer.id))
        .filter_map(|layer| media_path(workspace, layer.id).map(|path| (layer.id, layer.name.clone(), path)))
        .collect();
    if clips.len() < 2 {
        window.push_notification("Select two or more clips with sound (at normal speed) to line them up.", cx);
        return;
    }
    run(
        workspace,
        "Lining up by sound",
        move |progress, cancelled| {
            // Each clip's loudness over time, then how far each is from the first.
            let mut loudness = Vec::with_capacity(clips.len());
            for (index, (_, name, path)) in clips.iter().enumerate() {
                if cancelled() {
                    return Err("cancelled".to_owned());
                }
                let peaks = studio_media::audio_peaks(path, SYNC_BUCKETS_PER_SECOND).map_err(|error| format!("{name}: {error}"))?;
                loudness.push(peaks.values);
                progress((index + 1) as f32 / clips.len() as f32 * 0.6);
            }
            let max_lag = SYNC_MAX_SECONDS * SYNC_BUCKETS_PER_SECOND as usize;
            let mut found = Vec::new();
            for (index, other) in loudness.iter().enumerate().skip(1) {
                if cancelled() {
                    return Err("cancelled".to_owned());
                }
                let lag = studio_core::multicam::sync_lag(&loudness[0], other, max_lag).map(|(lag, _)| lag as f64 / f64::from(SYNC_BUCKETS_PER_SECOND));
                found.push((clips[index].0, clips[index].1.clone(), lag));
                progress(0.6 + index as f32 / clips.len() as f32 * 0.4);
            }
            Ok((clips[0].0, found))
        },
        move |this, (reference, found): (Id, Vec<(Id, String, Option<f64>)>), window, cx| {
            let mut unmatched = Vec::new();
            let mut moved = 0;
            for (clip, name, lag) in found {
                match lag {
                    Some(seconds) if this.state.line_up_clip(reference, clip, seconds) => moved += 1,
                    _ => unmatched.push(name),
                }
            }
            let message = match (moved, unmatched.is_empty()) {
                (0, _) => "None of the clips' sound matched the first one's, so nothing was moved.".to_owned(),
                (_, true) => format!("Lined up {moved} with the first clip."),
                (_, false) => format!("Lined up {moved} with the first clip. No match was found for: {}.", unmatched.join(", ")),
            };
            window.push_notification(message.as_str(), cx);
        },
        window,
        cx,
    );
}

/// Marks every beat of a clip's sound on the composition's timeline (replacing earlier beat
/// markers), within the clip.
pub fn mark_beats(workspace: &mut Workspace, layer: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(path) = media_path(workspace, layer) else { return };
    let comp = workspace.state.active_comp;
    run(
        workspace,
        "Finding beats",
        move |progress, cancelled| {
            progress(0.3);
            studio_media::detect_beats(&path, cancelled).map_err(|error| error.to_string())
        },
        move |this, beats, window, cx| {
            let Some(composition) = this.state.project.composition_mut(comp) else { return };
            let fps = composition.frame_rate.fps();
            let Some(found) = composition.layer(layer) else { return };
            let (start, in_frame, out_frame) = (found.start_frame, found.in_frame, found.out_frame);
            let frames: Vec<i64> = beats.iter().map(|seconds| start + (seconds * fps).round() as i64).filter(|frame| (in_frame..out_frame).contains(frame)).collect();
            let added = studio_core::edit::replace_markers(composition, "Beat", &frames);
            window.push_notification(format!("Marked {added} beats. Moves and cuts snap to them.").as_str(), cx);
        },
        window,
        cx,
    );
}

/// What to do at each scene cut found in a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneCutAction {
    /// Split the clip there, so each shot is a clip of its own.
    Split,
    /// Put a timeline marker there.
    Mark,
}

/// The shortest shot looked for, in seconds: cuts closer together than this are taken to be
/// one (a flash or a very quick dissolve), keeping the clearest.
const MIN_SHOT_SECONDS: f64 = 0.25;

/// Finds the hard cuts in video clip `layer` in the background, then splits it at each (one
/// undo step) or marks each on the timeline, as `action` says. Only the part of the video the
/// clip shows is looked at.
pub fn scene_cuts(workspace: &mut Workspace, layer: Id, action: SceneCutAction, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let composition = state.composition();
    let Some(found) = composition.layer(layer) else { return };
    let LayerContent::Footage { item } = found.content else { return };
    let Some((path, rate)) = state.project.item(item).and_then(|item| item.path.clone().zip(item.frame_rate)) else { return };
    let comp_rate = composition.frame_rate;
    // Each source frame the clip shows, with the first composition frame showing it. A frame
    // shown twice in a row (a slower source) is looked at once.
    let mut shown: Vec<(i64, i64)> = Vec::new();
    for frame in found.in_frame..found.out_frame {
        let source = found.source_frame(frame, comp_rate, rate).max(0);
        if shown.last().is_none_or(|&(_, last)| last != source) {
            shown.push((frame, source));
        }
    }
    let sources: Vec<i64> = shown.iter().map(|&(_, source)| source).collect();
    let min_shot = ((rate.fps().min(comp_rate.fps()) * MIN_SHOT_SECONDS).round() as usize).max(1);
    let comp = state.active_comp;
    // Where the clip was, to make sure it is still there when the search ends.
    let timing = (found.in_frame, found.out_frame, found.start_frame);
    let label = match action {
        SceneCutAction::Split => "Finding scene cuts",
        SceneCutAction::Mark => "Marking scene cuts",
    };
    run(
        workspace,
        label,
        move |progress, cancelled| studio_media::find_scene_cuts(&path, &sources, min_shot, progress, cancelled).map_err(|error| error.to_string()),
        move |this, cuts, window, cx| {
            let frames: Vec<i64> = cuts.iter().filter_map(|&position| shown.get(position)).map(|&(frame, _)| frame).collect();
            let unchanged = this.state.project.composition(comp).and_then(|composition| composition.layer(layer)).is_some_and(|found| (found.in_frame, found.out_frame, found.start_frame) == timing);
            let message = if !unchanged {
                "The clip was moved, trimmed or deleted while its scene cuts were found. Try again.".to_owned()
            } else if frames.is_empty() {
                "No scene cuts were found in this clip.".to_owned()
            } else {
                match action {
                    SceneCutAction::Split => {
                        let made = this.state.split_layer_at_frames(comp, layer, &frames);
                        if made == 0 {
                            // `cut::split_layers` leaves locked clips, and clips whose track matte doesn't span the cut, alone.
                            "The clip couldn't be split: it is locked, or its track matte doesn't span the cuts.".to_owned()
                        } else {
                            format!("Split the clip at {made} scene cut{} into {} shots.", if made == 1 { "" } else { "s" }, made + 1)
                        }
                    }
                    SceneCutAction::Mark => {
                        let Some(composition) = this.state.project.composition_mut(comp) else { return };
                        let added = studio_core::edit::replace_markers(composition, "Scene", &frames);
                        format!("Marked {added} scene cut{}. Moves and cuts snap to them.", if added == 1 { "" } else { "s" })
                    }
                }
            };
            window.push_notification(message.as_str(), cx);
        },
        window,
        cx,
    );
}

/// Pins `target` to the point `at` (composition pixels) of the footage layer `footage`: the
/// point is followed through the frames the two share, and the target's position keyframed to
/// move with it, from where it is now.
pub fn track_motion(workspace: &mut Workspace, target: Id, footage: Id, at: Vec2, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let composition = state.composition();
    let (Some(target_layer), Some(footage_layer)) = (composition.layer(target), composition.layer(footage)) else { return };
    let LayerContent::Footage { item } = footage_layer.content else { return };
    let Some((path, rate)) = state.project.item(item).and_then(|item| item.path.clone().zip(item.frame_rate)) else { return };
    let frame = state.current_frame;
    let Some(world) = studio_core::transform::world_transform(composition, footage, frame as f64) else { return };
    let Some(local) = world.inverse().map(|inverse| inverse.apply(at)) else { return };
    let size = footage_layer.size;
    let start = Vec2::new(local.x / size.x.max(1.0), local.y / size.y.max(1.0));
    if !(0.0..=1.0).contains(&start.x) || !(0.0..=1.0).contains(&start.y) {
        return;
    }
    // The frames both layers are present for.
    let (first, end) = (target_layer.in_frame.max(footage_layer.in_frame), target_layer.out_frame.min(footage_layer.out_frame));
    if end <= first {
        window.push_notification("The layer and the video are never on screen together.", cx);
        return;
    }
    let comp_rate = composition.frame_rate;
    let source = |frame: i64| footage_layer.source_frame(frame, comp_rate, rate).max(0);
    let (from, back_to, to) = (source(frame.clamp(first, end - 1)), source(first), source(end - 1) + 1);
    let comp = state.active_comp;
    run(
        workspace,
        "Tracking motion",
        move |progress, cancelled| studio_media::track_point(&path, start, from, back_to, to, progress, cancelled).map_err(|error| error.to_string()),
        move |this, tracked, window, cx| {
            let Some(composition) = this.state.project.composition(comp) else { return };
            let Some(footage_layer) = composition.layer(footage) else { return };
            let Some(target_layer) = composition.layer(target) else { return };
            let Some(base) = target_layer.transform.as_ref().map(|transform| transform.position.value_at(frame as f64)) else { return };
            let points: std::collections::HashMap<i64, Vec2> = tracked.points.iter().copied().collect();
            // Where the point is on screen at a composition frame, if it was found in the source
            // frame shown then.
            let point_on_screen = |comp_frame: i64| -> Option<Vec2> {
                let fraction = points.get(&footage_layer.source_frame(comp_frame, comp_rate, rate))?;
                let world = studio_core::transform::world_transform(composition, footage, comp_frame as f64)?;
                Some(world.apply(Vec2::new(fraction.x * size.x, fraction.y * size.y)))
            };
            let Some(anchor) = point_on_screen(frame.clamp(first, end - 1)) else { return };
            // The target moves by as much as the point does on screen, in its parent's space,
            // on the frames the point was found; where it was lost, the layer stays put.
            let keys: Vec<Keyframe<Vec2>> = (first..end)
                .filter_map(|comp_frame| {
                    let moved = point_on_screen(comp_frame)? - anchor;
                    let to_parent = studio_core::transform::parent_space(composition, target, comp_frame as f64);
                    Some(Keyframe { frame: comp_frame, value: base + to_parent.apply_vector(moved), easing: Easing::Linear })
                })
                .collect();
            if keys.len() < 2 {
                window.push_notification("The point was lost straight away. Try a spot with more contrast that stays in view.", cx);
                return;
            }
            let count = keys.len();
            if let Some(transform) = this.state.project.composition_mut(comp).and_then(|composition| composition.layer_mut(target)).and_then(|layer| layer.transform.as_mut()) {
                transform.position = Animated::keyframed(keys);
            }
            let message = if tracked.lost_before.is_some() || tracked.lost_after.is_some() {
                format!("Tracked {count} frames. The point was lost after that (hidden or out of frame), so the layer only follows it while it was found.")
            } else {
                format!("Tracked {count} frames. The layer now follows the point.")
            };
            window.push_notification(message.as_str(), cx);
        },
        window,
        cx,
    );
}

/// Stabilize: measures how the camera moves through footage layer `layer` in the background,
/// then steadies the layer against it (see `studio_core::stabilize`) in one undo step.
pub fn stabilize(workspace: &mut Workspace, layer: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let composition = state.composition();
    let Some(found) = composition.layer(layer) else { return };
    let LayerContent::Footage { item } = found.content else { return };
    let Some((path, rate)) = state.project.item(item).and_then(|item| item.path.clone().zip(item.frame_rate)) else {
        window.push_notification("Stabilize needs the clip's video file, and it can't be found.", cx);
        return;
    };
    // The source frames the layer shows (at any speed or direction), first to last.
    let comp_rate = composition.frame_rate;
    let shown: Vec<i64> = (found.in_frame..found.out_frame).map(|frame| found.source_frame(frame, comp_rate, rate).max(0)).collect();
    let (Some(&from), Some(&to)) = (shown.iter().min(), shown.iter().max()) else { return };
    if to <= from {
        window.push_notification("The clip needs at least two frames to steady.", cx);
        return;
    }
    let (comp, width) = (state.active_comp, found.size.x);
    run(
        workspace,
        "Stabilizing",
        move |progress, cancelled| studio_media::measure_motion(&path, from, to + 1, progress, cancelled).map_err(|error| error.to_string()),
        move |this, analysis, window, cx| {
            // Measured on smaller frames: scaled up to the layer's own pixels. Frames whose
            // motion couldn't be measured (a cut, a blank frame) count as holding still.
            let scale = width / analysis.width.max(1) as f32;
            let unmeasured = analysis.steps.iter().filter(|step| step.is_none()).count();
            let steps = analysis
                .steps
                .iter()
                .map(|step| step.map_or_else(Default::default, |motion| studio_core::stabilize::CameraStep { shift: motion.shift * scale, degrees: motion.rotation.to_degrees() }))
                .collect();
            let motion = studio_core::stabilize::MeasuredMotion { first_frame: analysis.first_frame, steps };
            let message = match this.state.stabilize_layer(comp, layer, motion, studio_core::stabilize::DEFAULT_SMOOTHNESS) {
                Ok(fit) => stabilized_message(fit, unmeasured),
                Err(error) => error.to_string(),
            };
            window.push_notification(message.as_str(), cx);
        },
        window,
        cx,
    );
}

/// What to tell the user after stabilizing: how much the clip was enlarged, whether any edges
/// still show, and how many frames couldn't be measured.
pub fn stabilized_message(fit: studio_core::stabilize::Stabilized, unmeasured: usize) -> String {
    let mut message = if fit.zoom > 1.0 {
        format!("Stabilized. The clip is enlarged to {:.0}% to keep its edges out of view.", fit.zoom * 100.0)
    } else {
        "Stabilized.".to_owned()
    };
    if fit.edges_show {
        message.push_str(" The shake is too strong to hide every edge; try a lower smoothness.");
    }
    if unmeasured > 0 {
        message.push_str(&format!(" {unmeasured} frames couldn't be measured and are held still."));
    }
    message
}

/// Samples taken each second when following the subject for Auto Reframe.
const REFRAME_SAMPLES_PER_SECOND: f64 = 4.0;

/// Makes a new composition in `format` holding a copy of footage layer `layer`, scaled to fill
/// the frame and moving to keep the subject in it. The subject is what the layer's background-
/// removal clicks pick, or else what is in the middle of the picture.
pub fn auto_reframe(workspace: &mut Workspace, layer: Id, format: Format, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let composition = state.composition();
    let Some(found) = composition.layer(layer) else { return };
    let LayerContent::Footage { item } = found.content else { return };
    let Some((path, rate)) = state.project.item(item).and_then(|item| item.path.clone().zip(item.frame_rate)) else { return };
    let comp_rate = composition.frame_rate;
    // Source frames to look at, a few a second across the clip.
    let step = ((comp_rate.fps() / REFRAME_SAMPLES_PER_SECOND).round() as i64).max(1);
    let samples: Vec<(i64, i64)> = (found.in_frame..found.out_frame).step_by(step as usize).map(|frame| (frame, found.source_frame(frame, comp_rate, rate).max(0))).collect();
    // An object matte's strokes on its first painted frame say what the subject is (each
    // stroke's points as clicks); otherwise the subject is taken to be in the middle.
    let painted: Vec<studio_cutout::Click> = found
        .cutout
        .as_ref()
        .and_then(|cut| cut.base_frames().first().map(|&frame| cut.strokes_on(frame).flat_map(|stroke| stroke.points.iter().map(|at| studio_cutout::Click { x: at.x, y: at.y, keep: stroke.add })).take(32).collect::<Vec<_>>()))
        .unwrap_or_default();
    let clicks = if painted.is_empty() { vec![studio_cutout::Click { x: 0.5, y: 0.45, keep: true }] } else { painted };
    let model = super::cutout::model_folder();
    let comp = state.active_comp;
    let start_reframe = move |this: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
        run(
            this,
            "Reframing",
            move |progress, cancelled| {
                if !studio_cutout::model_present(&model) {
                    super::cutout::fetch_model(&model, cancelled)?;
                }
                follow_subject(&model, &path, &samples, &clicks, progress, cancelled)
            },
            move |this, centers, window, cx| match reframed_composition(&mut this.state.project, comp, layer, format, &centers) {
                Some(new_comp) => {
                    this.state.open_composition(new_comp);
                    this.state.selected_item = Some(new_comp);
                }
                None => window.push_notification("The clip could not be reframed.", cx),
            },
            window,
            cx,
        );
    };
    if studio_cutout::model_present(&super::cutout::model_folder()) {
        start_reframe(workspace, window, cx);
        return;
    }
    let target = cx.weak_entity();
    let start_reframe = std::rc::Rc::new(std::cell::RefCell::new(Some(start_reframe)));
    crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| {
        let (target, start_reframe) = (target.clone(), start_reframe.clone());
        dialog
            .title("Download the subject-finding model?")
            .description(format!("Auto Reframe finds the subject with the MobileSAM model, the one background removal uses. It is downloaded once from huggingface.co (about {} MB) and kept in Omina Motion's settings folder. Your footage is never uploaded.", studio_cutout::MODEL_BYTES / 1_000_000))
            .ok_text("Download and Reframe")
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                if let Some(start) = start_reframe.borrow_mut().take() {
                    target.update(cx, |this, cx| start(this, window, cx)).ok();
                }
                true
            })
    });
}

/// Finds the subject's center (fractions of the picture) at each sample (composition frame,
/// source frame) of `path`, starting from `clicks` and following it from sample to sample.
fn follow_subject(model: &std::path::Path, path: &std::path::Path, samples: &[(i64, i64)], clicks: &[studio_cutout::Click], progress: &dyn Fn(f32), cancelled: &dyn Fn() -> bool) -> Result<Vec<(i64, Vec2)>, String> {
    let segmenter = studio_cutout::Segmenter::load(model).map_err(|error| error.to_string())?;
    let mut decoder = studio_media::VideoDecoder::open(path, studio_cutout::WORK_SIZE, true).map_err(|error| error.to_string())?;
    let mut centers = Vec::new();
    let mut previous: Option<studio_cutout::Mask> = None;
    for (index, &(frame, source)) in samples.iter().enumerate() {
        if cancelled() {
            return Ok(Vec::new());
        }
        let picture = decoder.frame(source).map_err(|error| error.to_string())?;
        let prepared = segmenter.prepare(&picture.rgba, picture.width, picture.height).map_err(|error| error.to_string())?;
        let mask = match &previous {
            Some(previous) => segmenter.follow(&prepared, previous, &[]),
            None => segmenter.segment(&prepared, clicks),
        }
        .map_err(|error| error.to_string())?;
        // Where the subject is, if it is anywhere; otherwise where it was last seen.
        if let Some(center) = mask_center(&mask) {
            centers.push((frame, center));
        } else if let Some(&(_, last)) = centers.last() {
            centers.push((frame, last));
        }
        previous = Some(mask);
        progress((index + 1) as f32 / samples.len() as f32);
    }
    Ok(centers)
}

/// The middle of what a mask keeps (fractions of the picture), or `None` if it keeps nothing.
fn mask_center(mask: &studio_cutout::Mask) -> Option<Vec2> {
    let (mut sum_x, mut sum_y, mut count) = (0.0f64, 0.0f64, 0.0f64);
    for (index, &alpha) in mask.alpha.iter().enumerate() {
        if alpha >= 128 {
            sum_x += (index as u32 % mask.width) as f64;
            sum_y += (index as u32 / mask.width) as f64;
            count += 1.0;
        }
    }
    (count > 0.0).then(|| Vec2::new((sum_x / count / f64::from(mask.width)) as f32, (sum_y / count / f64::from(mask.height)) as f32))
}

/// Builds the reframed composition: the clip, filling the new frame, keyframed so the subject
/// (whose center is given at some frames) stays as near the middle as the picture allows.
fn reframed_composition(project: &mut studio_core::model::Project, comp: Id, layer: Id, format: Format, centers: &[(i64, Vec2)]) -> Option<Id> {
    let source = project.composition(comp)?;
    let original = source.layer(layer)?.clone();
    let (width, height) = format.size();
    let settings = studio_core::edit::CompositionSettings {
        name: format!("{} {}", original.name, format.label()),
        width,
        height,
        frame_rate: source.frame_rate,
        duration_frames: source.duration_frames,
        background: studio_core::model::Rgba::hex(0x000000),
    };
    let new_comp = studio_core::edit::new_composition(project, &settings).ok()?;
    let frame = Vec2::new(width as f32, height as f32);
    let size = original.size;
    // Scaled to cover the whole frame.
    let scale = (frame.x / size.x).max(frame.y / size.y);
    let covered = size * scale;
    let anchor = size * 0.5;
    // A gentle average over neighboring samples, so the frame glides instead of jittering.
    let smoothed: Vec<(i64, Vec2)> = centers
        .iter()
        .enumerate()
        .map(|(index, &(at, _))| {
            let around = &centers[index.saturating_sub(2)..(index + 3).min(centers.len())];
            let sum = around.iter().fold(Vec2::default(), |sum, (_, point)| sum + *point);
            (at, sum * (1.0 / around.len() as f32))
        })
        .collect();
    let keep_covered = |position: Vec2| {
        let (half_x, half_y) = (covered.x * 0.5, covered.y * 0.5);
        Vec2::new(position.x.clamp(frame.x - half_x, half_x), position.y.clamp(frame.y - half_y, half_y))
    };
    let position_for = |center: Vec2| keep_covered(frame * 0.5 - Vec2::new((center.x * size.x - anchor.x) * scale, (center.y * size.y - anchor.y) * scale));
    let mut copy = original;
    copy.id = project.allocate_id();
    // Fresh ids for its effects and Merge nodes, with its node graph pointed at them.
    studio_core::effect_graph::reassign_ids(&mut copy, || project.allocate_id());
    copy.parent = None;
    copy.mask = None;
    copy.track_matte = studio_core::model::TrackMatte::None;
    copy.track = 0;
    if let Some(transform) = copy.transform.as_mut() {
        transform.anchor_point = Animated::constant(anchor);
        transform.scale = Animated::constant(Vec2::new(scale * 100.0, scale * 100.0));
        transform.rotation = Animated::constant(0.0);
        transform.position = match smoothed.as_slice() {
            [] => Animated::constant(position_for(Vec2::new(0.5, 0.5))),
            [(_, only)] => Animated::constant(position_for(*only)),
            many => Animated::keyframed(many.iter().map(|&(at, center)| Keyframe { frame: at, value: position_for(center), easing: Easing::Curve(studio_core::animation::CubicBezier::new(0.33, 0.0, 0.67, 1.0)) }).collect()),
        };
    }
    project.composition_mut(new_comp)?.layers.push(copy);
    Some(new_comp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::sample::{ids, sample_project};

    #[test]
    fn a_reframed_clip_fills_the_new_shape_and_follows_the_subject_without_showing_an_edge() {
        let mut project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let clip = composition.layers.iter().find(|layer| matches!(layer.content, LayerContent::Footage { .. })).unwrap().clone();
        // The subject starts on the left and walks to the far right edge.
        let centers: Vec<(i64, Vec2)> = (0..10).map(|step| (clip.in_frame + step * 6, Vec2::new(0.2 + step as f32 * 0.09, 0.5))).collect();
        let reframed = reframed_composition(&mut project, ids::MAIN_COMP, clip.id, Format::Vertical, &centers).unwrap();
        let composition = project.composition(reframed).unwrap();
        assert_eq!((composition.width, composition.height), (1080, 1920));
        let layer = &composition.layers[0];
        let transform = layer.transform.as_ref().unwrap();
        // Scaled to cover the frame's height.
        let scale = transform.scale.value.x / 100.0;
        assert!((layer.size.y * scale - 1920.0).abs() < 1.0);
        let at = |frame: i64| transform.position.value_at(frame as f64);
        let (start, end) = (at(clip.in_frame), at(clip.in_frame + 54));
        assert!(start.x > end.x, "moves left as the subject goes right: {start:?} → {end:?}");
        // Never so far that the frame's edge shows past the picture.
        let half = layer.size.x * scale * 0.5;
        for frame in clip.in_frame..clip.in_frame + 60 {
            let x = at(frame).x;
            assert!(x - half <= 0.5 && x + half >= 1079.5, "frame {frame}: an edge shows at {x}");
        }
    }

    /// Follows the orange disc across the clip made for background removal, with the real
    /// model: `CUTOUT_MODEL=<model folder> CUTOUT_CLIP=<clip> cargo test -p studio --release -- --ignored follows_a_real_subject`.
    #[test]
    #[ignore = "needs the downloaded model and a clip; set CUTOUT_MODEL and CUTOUT_CLIP"]
    fn follows_a_real_subject() {
        let (model, clip) = (std::env::var("CUTOUT_MODEL").unwrap(), std::env::var("CUTOUT_CLIP").unwrap());
        let samples: Vec<(i64, i64)> = (0..24).step_by(4).map(|frame| (frame, frame)).collect();
        // Clicked on the disc where it starts (x = 150 of 640).
        let clicks = [studio_cutout::Click { x: 150.0 / 640.0, y: 0.5, keep: true }];
        let centers = follow_subject(std::path::Path::new(&model), std::path::Path::new(&clip), &samples, &clicks, &|_| {}, &|| false).unwrap();
        assert_eq!(centers.len(), samples.len());
        for (frame, center) in centers {
            let expected = (150.0 + frame as f32 / 12.0 * 150.0) / 640.0;
            assert!((center.x - expected).abs() < 0.04, "frame {frame}: {} vs {expected}", center.x);
        }
    }
}
