//! File ▸ Package…, like After Effects' Collect Files or Premiere's Project Manager: the project
//! and everything it links to, copied into one zip. The dialog names the package, picks the
//! folder it goes in, and says what it will copy and what is missing. Then a background thread
//! writes the zip (`package_writer`), with progress and Cancel in the top bar, as exports do.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::{App, AppContext as _, Context, IntoElement, ParentElement, PromptButton, PromptLevel, Styled, Window, div, px};
use studio_core::package::{self, PackagePlan};
use studio_core::project_file::EditorView;

use crate::package_writer::{self, Outcome, PackageRequest};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// A package being written in the background.
pub struct PackageJob {
    pub file_name: String,
    pub done: u64,
    pub total: u64,
    cancel: Arc<AtomicBool>,
}

/// How often the editor checks on the package: often enough for a smooth progress bar.
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How many missing files the dialog names before saying how many more there are.
const MISSING_NAMED: usize = 3;

/// What the packaging thread reports: how far it has got, and how it ended once it has.
#[derive(Default)]
struct Report {
    done: u64,
    total: u64,
    outcome: Option<Result<Outcome, String>>,
}

/// Where packages go unless another folder is chosen: beside the saved project, else the
/// Desktop, else the home folder.
fn default_folder(project_file: Option<&Path>) -> PathBuf {
    if let Some(folder) = project_file.and_then(Path::parent) {
        return folder.to_path_buf();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let desktop = home.as_ref().map(|home| home.join("Desktop")).filter(|desktop| desktop.is_dir());
    desktop.or(home).unwrap_or_else(|| PathBuf::from("."))
}

/// A file's size, or `None` if there is no file there.
fn size_on_disk(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().filter(|metadata| metadata.is_file()).map(|metadata| metadata.len())
}

/// "1 file" or "3 files".
fn files(count: usize) -> String {
    if count == 1 { "1 file".to_owned() } else { format!("{count} files") }
}

/// What the dialog says the package will hold: what it copies, then what is missing.
fn summary(plan: &PackagePlan) -> Vec<String> {
    let mut lines = Vec::new();
    if plan.files.is_empty() {
        lines.push("The project links no media files, so the package holds just the project.".to_owned());
    } else {
        let total = plan.total_bytes();
        let size = if total < 1_000_000 { "under 1 MB".to_owned() } else { super::settings::format_bytes(total) };
        lines.push(format!("Copies {} ({size}) into the package, with the project.", files(plan.files.len())));
    }
    if !plan.missing.is_empty() {
        let mut names: Vec<String> = plan.missing.iter().take(MISSING_NAMED).map(|path| path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())).collect();
        if plan.missing.len() > MISSING_NAMED {
            names.push(format!("and {} more", plan.missing.len() - MISSING_NAMED));
        }
        let verb = if plan.missing.len() == 1 { "is" } else { "are" };
        lines.push(format!(
            "{} linked by the project {verb} missing and will be left out: {}. The package lists them in “{}”.",
            files(plan.missing.len()),
            names.join(", "),
            package::MISSING_NOTE
        ));
    }
    lines
}

/// Opens the Package dialog. While a package is being made, File ▸ Package… is greyed out and
/// the top bar shows its progress; this says so if it is reached some other way.
pub fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.package_job.is_some() {
        window.push_notification("A package is already being made. Cancel it or wait for it to finish.", cx);
        return;
    }
    let state = &workspace.state;
    let project_folder = state.file().and_then(Path::parent).map(Path::to_path_buf);
    let lines = summary(&package::plan(&state.project, project_folder.as_deref(), size_on_disk));
    let input = cx.new(|cx| InputState::new(window, cx).default_value(state.document_name()));
    let folder = Rc::new(RefCell::new(default_folder(state.file())));
    let target = cx.weak_entity();
    crate::panels::chrome::open_dialog(window, cx, move |dialog, _, _| {
        let choose = {
            let folder = folder.clone();
            Button::new("package-choose").outline().small().label("Choose…").on_click(move |_, window, cx| choose_folder(folder.clone(), window, cx))
        };
        let shown_folder = folder.borrow().display().to_string();
        let (input, folder, target) = (input.clone(), folder.clone(), target.clone());
        let row = |label: &'static str| div().flex().items_center().gap_3().child(div().flex_none().w(px(96.0)).child(label));
        dialog
            .title("Package Project")
            .w(px(560.0))
            .centered()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_size(px(size::TEXT + 1.0))
                    .child(row("Project name:").child(div().flex_1().child(Input::new(&input))))
                    .child(
                        row("Save to:")
                            .child(div().flex_1().min_w_0().truncate().text_color(hsla(color::MUTED)).child(shown_folder))
                            .child(choose),
                    )
                    .children(lines.iter().map(|line| div().text_color(hsla(color::MUTED)).child(line.clone()))),
            )
            .footer(
                DialogFooter::new()
                    .child(DialogClose::new().trigger(|button| button.outline().label("Cancel")))
                    .child(DialogAction::new().child(Button::new("package-ok").primary().label("Package"))),
            )
            .on_ok(move |_, window, cx| {
                let name = package::safe_file_name(&input.read(cx).value(), "");
                if name.is_empty() {
                    window.push_notification("Give the package a name first.", cx);
                    return false;
                }
                let folder = folder.borrow().clone();
                target.update(cx, |this, cx| start(this, name, folder, window, cx)).ok();
                true
            })
    });
}

/// Asks for the folder to save the package in, and shows it in the dialog.
fn choose_folder(folder: Rc<RefCell<PathBuf>>, window: &mut Window, cx: &mut App) {
    let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some("Choose".into()) });
    window
        .spawn(cx, async move |cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let Some(chosen) = paths.into_iter().next() else { return };
            *folder.borrow_mut() = chosen;
            cx.update(|window, _| window.refresh()).ok();
        })
        .detach();
}

/// Packages the project as `<folder>/<name>.zip`, asking first if that file is already there.
fn start(workspace: &mut Workspace, name: String, folder: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    if !folder.is_dir() {
        let message = format!("“{}” isn't a folder that can be saved in. Choose another folder.", folder.display());
        crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title("The package couldn't be made").description(message.clone()));
        return;
    }
    let target = folder.join(format!("{name}.zip"));
    if !target.exists() {
        run(workspace, name, target, window, cx);
        return;
    }
    let message = format!("“{name}.zip” already exists in “{}”.", folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned()));
    let buttons = [PromptButton::ok("Replace"), PromptButton::cancel("Cancel")];
    let answer = window.prompt(PromptLevel::Warning, &message, Some("Replacing it overwrites what is in it now."), &buttons, cx);
    cx.spawn_in(window, async move |this, cx| {
        if answer.await != Ok(0) {
            return;
        }
        this.update_in(cx, |this, window, cx| run(this, name, target, window, cx)).ok();
    })
    .detach();
}

/// Writes the package on a background thread from a snapshot of the project, showing progress
/// in the top bar, then says how it went.
fn run(workspace: &mut Workspace, name: String, target: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) {
    let state = &workspace.state;
    let request = PackageRequest {
        name,
        target: target.clone(),
        project: state.project.clone(),
        project_folder: state.file().and_then(Path::parent).map(Path::to_path_buf),
        view: EditorView { active_composition: state.active_comp, current_frame: state.current_frame, work_area: state.work_area },
        settings: workspace.editor_settings(),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let file_name = target.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    workspace.ui.package_job = Some(PackageJob { file_name: file_name.clone(), done: 0, total: 0, cancel: cancel.clone() });
    cx.notify();

    // Copying is long, steady work on its own thread; the editor checks on it regularly. The
    // zip is remembered in the settings until it is done, so one left half-written by a crash
    // is removed at the next launch.
    let report = Arc::new(Mutex::new(Report::default()));
    let thread_report = report.clone();
    crate::app_settings::update(|settings| settings.package_in_progress = Some(target.clone()));
    let spawned = std::thread::Builder::new().name("package".into()).spawn(move || {
        lock(&thread_report).total = package_writer::total_bytes(&request);
        let outcome = package_writer::write(&request, &mut |done| lock(&thread_report).done = done, &|| cancel.load(Ordering::Relaxed));
        lock(&thread_report).outcome = Some(outcome);
    });
    if let Err(error) = spawned {
        crate::app_settings::update(|settings| settings.package_in_progress = None);
        workspace.ui.package_job = None;
        window.push_notification(format!("Packaging couldn't start: {error}").as_str(), cx);
        return;
    }

    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            let (done, total, outcome) = {
                let mut report = lock(&report);
                (report.done, report.total, report.outcome.take())
            };
            let keep_polling = this.update_in(cx, |this, window, cx| {
                let Some(outcome) = outcome else {
                    if let Some(job) = this.ui.package_job.as_mut()
                        && (job.done, job.total) != (done, total)
                    {
                        (job.done, job.total) = (done, total);
                        cx.notify();
                    }
                    return true;
                };
                this.ui.package_job = None;
                crate::app_settings::update(|settings| settings.package_in_progress = None);
                cx.notify();
                match outcome {
                    Ok(Outcome::Finished { missing }) => announce(&target, &missing, window, cx),
                    Ok(Outcome::Cancelled) => window.push_notification("Packaging cancelled.", cx),
                    Err(error) => crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title("The package couldn't be made").description(error.clone())),
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

/// Says the package is made, and what it is missing, with a button to show it in the Finder.
fn announce(zip: &Path, missing: &[PathBuf], window: &mut Window, cx: &mut App) {
    let zip_name = zip.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let note = if missing.is_empty() {
        Notification::success(format!("Packaged “{zip_name}”."))
    } else {
        let verb = if missing.len() == 1 { "was" } else { "were" };
        Notification::warning(format!("Packaged “{zip_name}”. {} {verb} missing and left out; “{}” in the package lists them.", files(missing.len()), package::MISSING_NOTE))
    };
    let zip = zip.to_path_buf();
    let note = note.action(move |_, _, _| {
        let zip = zip.clone();
        Button::new("package-reveal").ghost().small().label("Show in Finder").on_click(move |_, _, cx| cx.reveal_path(&zip))
    });
    window.push_notification(note, cx);
}

/// The report, even if the packaging thread panicked while holding it.
fn lock(report: &Mutex<Report>) -> std::sync::MutexGuard<'_, Report> {
    report.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Stops the package being made. It removes its partial zip as it stops.
pub fn cancel(workspace: &mut Workspace) {
    if let Some(job) = &workspace.ui.package_job {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// Removes the partial zip of a package Studio didn't live to finish (it quit or crashed
/// mid-package). Called at launch.
pub fn remove_unfinished_package() {
    let Some(target) = crate::app_settings::load().package_in_progress else { return };
    let _ = std::fs::remove_file(package_writer::temporary_path(&target));
    crate::app_settings::update(|settings| settings.package_in_progress = None);
}

/// The top bar's packaging status: zip name, progress bar and Cancel, while a package is made.
pub fn status(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<impl IntoElement + use<>> {
    let job = workspace.ui.package_job.as_ref()?;
    let percent = (job.done as f64 * 100.0 / job.total.max(1) as f64).min(100.0) as f32;
    Some(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(hsla(color::MUTED)).child(format!("Packaging {} · {}%", job.file_name, percent as i64)))
            .child(div().w(px(120.0)).child(Progress::new("package-progress").value(percent)))
            .child(Button::new("package-cancel").ghost().xsmall().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                cancel(this);
                cx.notify();
            }))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_says_what_is_copied_and_names_what_is_missing() {
        let file = |name: &str, bytes| package::PackagedFile { source: PathBuf::from(format!("/media/{name}")), destination: format!("Media/{name}"), bytes };
        let plan = PackagePlan { files: vec![file("a.mov", 1_500_000_000), file("b.png", 200_000)], missing: ["/x/1.mov", "/x/2.mov", "/x/3.mov", "/x/4.mov"].map(PathBuf::from).to_vec() };
        let lines = summary(&plan);
        assert_eq!(lines[0], "Copies 2 files (1.5 GB) into the package, with the project.");
        assert!(lines[1].starts_with("4 files linked by the project are missing and will be left out: 1.mov, 2.mov, 3.mov, and 1 more."), "{}", lines[1]);
        let empty = summary(&PackagePlan::default());
        assert_eq!(empty, ["The project links no media files, so the package holds just the project."]);
        let small = summary(&PackagePlan { files: vec![file("a.cube", 900)], missing: vec![PathBuf::from("/x/1.mov")] });
        assert_eq!(small[0], "Copies 1 file (under 1 MB) into the package, with the project.");
        assert!(small[1].starts_with("1 file linked by the project is missing"), "{}", small[1]);
    }

    #[test]
    fn a_package_studio_didnt_finish_is_cleaned_up_at_the_next_launch() {
        let folder = std::env::temp_dir().join(format!("studio-unfinished-package-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let target = folder.join("Promo.zip");
        let partial = package_writer::temporary_path(&target);
        std::fs::write(&partial, b"half a zip").unwrap();
        crate::app_settings::update(|settings| settings.package_in_progress = Some(target.clone()));
        remove_unfinished_package();
        assert!(!partial.exists(), "the hidden partial file is gone");
        assert_eq!(crate::app_settings::load().package_in_progress, None);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
