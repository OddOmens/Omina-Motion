//! Exporting the open composition to a movie (File ▸ Export…, ⌘M). The dialog picks the
//! format, the frames and whether to include sound; then a background thread renders and
//! encodes a snapshot of the project, so editing can carry on meanwhile. Progress and a
//! Cancel button show in the toolbar.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::radio::Radio;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use studio_core::audio::composition_audio;
use studio_core::time::format_timecode as timecode;
use studio_export::{ExportRequest, Outcome};
use studio_media::MovieFormat;

use crate::theme::{color, hsla};
use crate::workspace::Workspace;

/// The choices in the Export dialog. Kept between exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExportChoice {
    pub format: MovieFormat,
    /// Export only the work area, rather than the whole composition.
    pub work_area_only: bool,
    pub include_audio: bool,
}

impl Default for ExportChoice {
    fn default() -> Self {
        Self { format: MovieFormat::H264, work_area_only: true, include_audio: true }
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

/// Opens the Export dialog for the open composition.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.export_job.is_some() {
        window.push_notification("An export is already running. Cancel it or wait for it to finish.", cx);
        return;
    }
    let target = cx.weak_entity();
    window.open_dialog(cx, move |dialog, _, cx| {
        let Some(workspace) = target.upgrade() else { return dialog };
        let ws = workspace.read(cx);
        let choice = ws.ui.export_choice;
        let comp = ws.state.composition();
        let has_audio = !composition_audio(&ws.state.project, comp).is_empty();
        let rate = comp.frame_rate;
        let work_area = format!(
            "Work area ({} – {})",
            timecode(ws.state.work_area.start as i64, rate),
            timecode(ws.state.work_area.end as i64, rate)
        );
        let whole = format!("Entire composition ({})", timecode(comp.duration_frames, rate));

        // Each control writes its choice into the workspace; the dialog redraws from it.
        let set = |change: fn(&mut ExportChoice, bool)| {
            let target = target.clone();
            move |checked: &bool, _: &mut Window, cx: &mut gpui_kit::App| {
                let checked = *checked;
                target.update(cx, |this, cx| {
                    change(&mut this.ui.export_choice, checked);
                    cx.notify();
                }).ok();
            }
        };
        let formats = MovieFormat::ALL.iter().enumerate().map(|(index, &format)| {
            let target = target.clone();
            Radio::new(("export-format", index)).label(format.label()).checked(choice.format == format).on_click(move |_, _, cx| {
                target.update(cx, |this, cx| {
                    this.ui.export_choice.format = format;
                    cx.notify();
                }).ok();
            })
        });
        let start = target.clone();
        dialog
            .title(format!("Export “{}”", comp.name))
            .w(px(440.0))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(section("Format", div().flex().flex_col().gap_1().children(formats)))
                    .child(section(
                        "Frames",
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(Radio::new("export-work-area").label(work_area).checked(choice.work_area_only).on_click(set(|c, _| c.work_area_only = true)))
                            .child(Radio::new("export-whole").label(whole).checked(!choice.work_area_only).on_click(set(|c, _| c.work_area_only = false))),
                    ))
                    .child(section(
                        "Sound",
                        Checkbox::new("export-audio")
                            .label(if has_audio { "Include audio" } else { "Include audio (this composition has no audible layers)" })
                            .checked(has_audio && choice.include_audio)
                            .disabled(!has_audio)
                            .on_click(set(|c, checked| c.include_audio = checked)),
                    )),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("export-ok").primary().label("Export…"))),
            )
            .on_ok(move |_, window, cx| {
                start.update(cx, |this, cx| choose_file(this, window, cx)).ok();
                true
            })
    });
}

fn section(title: &'static str, body: impl IntoElement) -> impl IntoElement {
    div().flex().flex_col().gap_1().child(div().text_color(hsla(color::MUTED)).child(title)).child(body)
}

/// Where the save dialog starts: next to the project if it has been saved, else Movies.
fn starting_folder(project_file: Option<&Path>) -> PathBuf {
    project_file
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(if cfg!(target_os = "macos") { "Movies" } else { "Videos" })))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Asks where to save the movie, then starts the export.
fn choose_file(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let format = workspace.ui.export_choice.format;
    let suggested = format!("{}.{}", workspace.state.composition().name, format.extension());
    let chosen = cx.prompt_for_new_path(&starting_folder(workspace.state.file()), Some(&suggested));
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(mut path))) = chosen.await else { return };
        // The file's extension must match what it holds, or players may refuse it.
        if path.extension().and_then(|e| e.to_str()).is_none_or(|e| !e.eq_ignore_ascii_case(format.extension())) {
            path.set_extension(format.extension());
        }
        this.update_in(cx, |this, window, cx| start(this, path, window, cx)).ok();
    })
    .detach();
}

/// Starts exporting a snapshot of the project to `path` on a background thread.
pub fn start(workspace: &mut Workspace, path: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    let choice = workspace.ui.export_choice;
    let state = &workspace.state;
    let frames = if choice.work_area_only {
        state.work_area.start as i64..state.work_area.end as i64
    } else {
        0..state.composition().duration_frames
    };
    let request = ExportRequest { composition: state.active_comp, frames, format: choice.format, include_audio: choice.include_audio, path };
    let project = state.project.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let file_name = request.path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    workspace.ui.export_job = Some(ExportJob { file_name: file_name.clone(), done: 0, total: request.frames.end - request.frames.start, cancel: cancel.clone() });
    cx.notify();

    // The export runs on its own thread (it is long, steady work); the editor checks on it
    // regularly rather than being woken by it.
    let report = Arc::new(Mutex::new(Report::default()));
    let thread_report = report.clone();
    let spawned = std::thread::Builder::new().name("export".into()).spawn(move || {
        let outcome = studio_export::export(&project, &request, &mut |progress| lock(&thread_report).done = progress.done, &|| {
            cancel.load(Ordering::Relaxed)
        });
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
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
                cx.notify();
                if this.ui.quit_after_export {
                    cx.quit();
                    return false;
                }
                match outcome {
                    Ok(Outcome::Finished) => window.push_notification(format!("Exported “{file_name}”.").as_str(), cx),
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
