//! Exporting the open composition (File ▸ Export…, ⌘M). The dialog (`export_dialog`) picks
//! the format, picture, frames and sound; then a background thread renders and encodes a
//! snapshot of the project, so editing can carry on meanwhile. Progress and a Cancel button
//! show in the toolbar.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use studio_core::model::Id;
use studio_core::time::FrameRate;
use studio_export::{ExportFormat, ExportRequest, Outcome};
use studio_media::{EncodeSpeed, MovieFormat, VideoQuality};

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// The choices in the Export dialog. Kept between exports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExportChoice {
    pub format: ExportFormat,
    /// The output's frame rate, or `None` for the composition's.
    pub frame_rate: Option<FrameRate>,
    /// Export only the work area, rather than the whole composition.
    pub work_area_only: bool,
    pub include_audio: bool,
    /// The movie's size as a fraction of the composition's.
    pub scale: f32,
    pub quality: VideoQuality,
    pub speed: EncodeSpeed,
    pub audio_kbps: u32,
}

impl Default for ExportChoice {
    fn default() -> Self {
        Self { format: ExportFormat::Movie(MovieFormat::H264), frame_rate: None, work_area_only: true, include_audio: true, scale: 1.0, quality: VideoQuality::High, speed: EncodeSpeed::Balanced, audio_kbps: 320 }
    }
}

/// An export running in the background.
pub struct ExportJob {
    pub file_name: String,
    pub done: i64,
    pub total: i64,
    cancel: Arc<AtomicBool>,
}

/// How often the editor checks on the export: often enough for a smooth progress bar.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What the export thread reports: how far it has got, and how it ended once it has.
#[derive(Default)]
struct Report {
    done: i64,
    outcome: Option<Result<Outcome, String>>,
}

/// Where the save dialog starts: next to the project if it has been saved, else Movies.
pub(crate) fn starting_folder(project_file: Option<&Path>) -> PathBuf {
    project_file
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(if cfg!(target_os = "macos") { "Movies" } else { "Videos" })))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Asks where to save the export, then starts it. A PNG sequence is saved as a folder of
/// that name.
pub fn choose_file(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let format = workspace.ui.export_choice.format;
    let name = &workspace.state.composition().name;
    let suggested = match format.extension() {
        Some(extension) => format!("{name}.{extension}"),
        None => name.clone(),
    };
    let chosen = cx.prompt_for_new_path(&starting_folder(workspace.state.file()), Some(&suggested));
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(mut path))) = chosen.await else { return };
        // The file's extension must match what it holds, or players may refuse it.
        if let Some(extension) = format.extension()
            && path.extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case(extension))
        {
            path.set_extension(extension);
        }
        this.update_in(cx, |this, window, cx| start(this, path, window, cx)).ok();
    })
    .detach();
}

/// What to do once an export finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterExport {
    /// Just say it is done.
    Nothing,
    /// Say where the file went (a quick export, which asks no questions).
    Announce,
    /// Import the movie into the project's Exports folder and put it in place of `layer`.
    Replace { comp: Id, layer: Id },
}

/// Where quick exports go: an Exports folder beside the saved project, or in Movies (Videos)
/// for one not saved yet.
pub fn exports_folder(project_file: Option<&Path>) -> PathBuf {
    match project_file.and_then(Path::parent) {
        Some(folder) => folder.join(studio_core::folders::EXPORTS_FOLDER),
        // Tests never write into the person's Movies folder.
        None if cfg!(test) => std::env::temp_dir().join(format!("studio-test-exports-{}", std::process::id())),
        None => starting_folder(None).join("Studio Exports"),
    }
}

/// A file name in `folder` that is not taken yet: `name.extension`, else `name 2.extension`,
/// and so on. Characters the Finder can't show in names are replaced.
fn free_path(folder: &Path, name: &str, extension: &str) -> PathBuf {
    let clean: String = name.chars().map(|c| if matches!(c, '/' | ':' | '\\') || c.is_control() { '-' } else { c }).collect();
    let clean = if clean.trim().is_empty() { "Layer".to_owned() } else { clean.trim().to_owned() };
    (1..)
        .map(|n| folder.join(if n == 1 { format!("{clean}.{extension}") } else { format!("{clean} {n}.{extension}") }))
        .find(|path| !path.exists())
        .expect("some numbered name is free")
}

/// Renders just `layer` of the open composition, with its transparency, to a ProRes 4444 movie
/// in the Exports folder (see `exports_folder`). With `replace`, the movie is then imported and
/// takes the layer's place, so playback reads one file instead of working the layer out.
pub fn quick_export_layer(workspace: &mut Workspace, layer: Id, replace: bool, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.export_job.is_some() {
        window.push_notification("An export is already running. Cancel it or wait for it to finish.", cx);
        return;
    }
    let comp = workspace.state.active_comp;
    if replace && studio_core::bake::is_depended_on(&workspace.state.project, comp, layer) {
        window.push_notification("Other layers use this one (as a parent, mask or text path), so it can't be replaced.", cx);
        return;
    }
    let Some((project, frames)) = studio_core::bake::solo_copy(&workspace.state.project, comp, layer) else {
        window.push_notification("This layer isn't on screen inside the composition, so there is nothing to export.", cx);
        return;
    };
    let folder = exports_folder(workspace.state.file());
    if let Err(error) = std::fs::create_dir_all(&folder) {
        window.push_notification(format!("Could not make the Exports folder: {error}").as_str(), cx);
        return;
    }
    let composition = workspace.state.composition();
    let layer_name = composition.layer(layer).map_or_else(String::new, |layer| layer.name.clone());
    let format = MovieFormat::ProRes4444;
    let path = free_path(&folder, &format!("{} - {layer_name}", composition.name), format.extension());
    let request = ExportRequest::new(comp, frames, format, path);
    let after = if replace { AfterExport::Replace { comp, layer } } else { AfterExport::Announce };
    run(workspace, project, request, after, window, cx);
}

/// The composition frames `choice` exports: the work area, or the whole composition.
pub fn chosen_frames(state: &crate::state::EditorState, choice: &ExportChoice) -> std::ops::Range<i64> {
    if choice.work_area_only {
        state.work_area.start as i64..state.work_area.end as i64
    } else {
        0..state.composition().duration_frames
    }
}

/// Starts exporting a snapshot of the project to `path` on a background thread.
pub fn start(workspace: &mut Workspace, path: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    let choice = workspace.ui.export_choice;
    let state = &workspace.state;
    let frames = chosen_frames(state, &choice);
    let request = ExportRequest {
        frame_rate: choice.frame_rate,
        include_audio: choice.include_audio,
        scale: choice.scale,
        quality: choice.quality,
        speed: choice.speed,
        audio_kbps: choice.audio_kbps,
        ..ExportRequest::new(state.active_comp, frames, choice.format, path)
    };
    let project = state.project.clone();
    run(workspace, project, request, AfterExport::Nothing, window, cx);
}

/// Removes the unfinished file of an export that Studio didn't live to finish (it quit or
/// crashed mid-export). Called at launch.
pub fn remove_unfinished_export() {
    let Some(target) = crate::app_settings::load().export_in_progress else { return };
    let unfinished = studio_export::temporary_path(&target);
    if unfinished.exists() {
        studio_export::discard(&unfinished);
    }
    crate::app_settings::update(|settings| settings.export_in_progress = None);
}

/// Exports `project` as `request` says on a background thread, showing progress in the
/// toolbar, then does what `after` says.
fn run(workspace: &mut Workspace, project: studio_core::model::Project, request: ExportRequest, after: AfterExport, window: &mut Window, cx: &mut Context<Workspace>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let file_name = request.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let path = request.path.clone();
    let total = project.composition(request.composition).map_or(request.frames.end - request.frames.start, |comp| request.output_frame_count(comp.frame_rate));
    workspace.ui.export_job = Some(ExportJob { file_name: file_name.clone(), done: 0, total, cancel: cancel.clone() });
    cx.notify();

    // The export runs on its own thread (it is long, steady work); the editor checks on it
    // regularly rather than being woken by it.
    let report = Arc::new(Mutex::new(Report::default()));
    let thread_report = report.clone();
    crate::app_settings::update(|settings| settings.export_in_progress = Some(path.clone()));
    let spawned = std::thread::Builder::new().name("export".into()).spawn(move || {
        let outcome = studio_export::export(&project, &request, &mut |progress| lock(&thread_report).done = progress.done, &|| {
            cancel.load(Ordering::Relaxed)
        });
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
        crate::app_settings::update(|settings| settings.export_in_progress = None);
        workspace.ui.export_job = None;
        window.push_notification(format!("The export could not start: {error}").as_str(), cx);
        return;
    }

    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let (done, outcome) = {
                let mut report = lock(&report);
                (report.done, report.outcome.take())
            };
            let keep_polling = this.update_in(cx, |this, window, cx| {
                let Some(outcome) = outcome else {
                    if let Some(job) = this.ui.export_job.as_mut()
                        && job.done != done
                    {
                        job.done = done;
                        cx.notify();
                    }
                    return true;
                };
                this.ui.export_job = None;
                crate::app_settings::update(|settings| settings.export_in_progress = None);
                cx.notify();
                if this.ui.quit_after_export {
                    cx.quit();
                    return false;
                }
                match outcome {
                    // What the format couldn't hold gets a notice of its own, listing it.
                    Ok(Outcome::Finished { left_out }) if !left_out.is_empty() && after == AfterExport::Nothing => {
                        let title = format!("Exported “{file_name}”, with changes");
                        let list = left_out.iter().map(|line| format!("• {line}")).collect::<Vec<_>>().join("\n");
                        window.open_alert_dialog(cx, move |dialog, _, _| dialog.title(title.clone()).description(list.clone()));
                    }
                    Ok(Outcome::Finished { .. }) => match after {
                        AfterExport::Nothing => window.push_notification(format!("Exported “{file_name}”.").as_str(), cx),
                        AfterExport::Announce => {
                            let folder = path.parent().map_or_else(String::new, |folder| folder.display().to_string());
                            window.push_notification(format!("Exported “{file_name}” to {folder}.").as_str(), cx);
                        }
                        AfterExport::Replace { comp, layer } => replace_with_movie(this, &path, comp, layer, window, cx),
                    },
                    Ok(Outcome::Cancelled) => window.push_notification("Export cancelled.", cx),
                    Err(error) => {
                        window.open_alert_dialog(cx, move |dialog, _, _| dialog.title("The export failed").description(error.clone()));
                    }
                }
                false
            });
            if !keep_polling.unwrap_or(false) {
                break;
            }
        }
    })
    .detach();
}

/// Imports the finished movie into the project's Exports folder and puts it in place of
/// `layer`. If the layer has gone meanwhile, the movie is still imported.
fn replace_with_movie(workspace: &mut Workspace, path: &Path, comp: Id, layer: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    let replace: crate::panels::importing::Then = Box::new(move |workspace, items, window, cx| {
        if let Some(&item) = items.first() {
            put_in_place(workspace, item, comp, layer, window, cx);
        }
    });
    workspace.import_paths(vec![path.to_path_buf()], Some(replace), window, cx);
}

/// The second half of `replace_with_movie`, once the movie is imported as `item`.
fn put_in_place(workspace: &mut Workspace, item: Id, comp: Id, layer: Id, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.state.file_into_exports(item);
    workspace.ui.reveal_item(&workspace.state.project, item);
    if workspace.state.replace_with_render(comp, layer, item).is_none() {
        window.push_notification("The render was imported into Exports, but the layer it was made from has changed, so it was left in place.", cx);
    }
    cx.notify();
}

/// The report, even if the export thread panicked while holding it.
fn lock(report: &Mutex<Report>) -> std::sync::MutexGuard<'_, Report> {
    report.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Stops the running export. It finishes its current frame and removes the partial file.
pub fn cancel(workspace: &mut Workspace) {
    if let Some(job) = &workspace.ui.export_job {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// The toolbar's export status: file name, progress bar and Cancel, while an export runs.
pub fn status(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let job = workspace.ui.export_job.as_ref()?;
    let percent = job.done as f32 * 100.0 / job.total.max(1) as f32;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("Exporting {} · {}%", job.file_name, percent as i64)))
            .child(div().w(px(120.0)).child(Progress::new("export-progress").value(percent)))
            .child(Button::new("export-cancel").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                cancel(this);
                cx.notify();
            }))),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn an_export_studio_didnt_finish_is_cleaned_up_at_the_next_launch() {
        let folder = std::env::temp_dir().join(format!("studio-unfinished-export-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("Promo.mp4");
        let unfinished = studio_export::temporary_path(&target);
        std::fs::write(&unfinished, b"half a movie").unwrap();
        crate::app_settings::update(|settings| settings.export_in_progress = Some(target.clone()));
        super::remove_unfinished_export();
        assert!(!unfinished.exists(), "the hidden partial file is gone");
        assert_eq!(crate::app_settings::load().export_in_progress, None);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
