//! Importing files in the background, with a popup that shows how far along it is. Reading
//! media (FFmpeg probes each file) and After Effects or Lottie files can take a while, so it
//! happens off the main thread, one file at a time, and the popup counts them and offers
//! Cancel. Everything read is added to the project at the end in one step (one undo); a
//! cancelled import adds nothing.

use std::path::PathBuf;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{Disableable as _, Sizable as _};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::{AnyElement, Context, InteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt as _, Window, black, deferred, div, px};
use studio_core::model::Id;

use crate::state::{ReadFile, ReadProject};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// What to do with the new items once they are in the project (place them on the timeline, say).
pub type Then = Box<dyn FnOnce(&mut Workspace, Vec<Id>, &mut Window, &mut Context<Workspace>)>;

/// An import in progress.
pub struct ImportJob {
    /// Files read so far, and how many there are (`None` while folders are still being listed).
    done: usize,
    total: Option<usize>,
    /// The name of the file being read now.
    current: String,
    cancelled: bool,
}

/// A project file to read, and the folder its embedded images are written into.
type ProjectFile = (PathBuf, PathBuf);

/// Imports `paths` (files and folders) in the background, then runs `then` with the new items.
/// After Effects projects and Lottie animations come in as compositions of their own.
pub fn start(workspace: &mut Workspace, paths: Vec<PathBuf>, then: Option<Then>, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.import_job.is_some() {
        window.push_notification("An import is already running. Wait for it, or cancel it.", cx);
        return;
    }
    let (projects, media): (Vec<_>, Vec<_>) = paths.into_iter().partition(|path| ["aep", "aepx", "json"].iter().any(|extension| crate::state::is_extension(path, extension)));
    // XML projects can't be read; say so straight away rather than after the rest.
    let (xml, projects): (Vec<_>, Vec<_>) = projects.into_iter().partition(|path| crate::state::is_extension(path, "aepx"));
    for path in xml {
        let file = file_name(&path);
        window.push_notification(format!("“{file}” is an XML project. In After Effects, save it as .aep, then import that.").as_str(), cx);
    }
    if projects.is_empty() && media.is_empty() {
        return;
    }
    let projects: Vec<ProjectFile> = projects.into_iter().map(|path| {
        let assets = workspace.imported_assets_folder(&path);
        (path, assets)
    }).collect();
    workspace.ui.import_job = Some(ImportJob { done: 0, total: None, current: String::new(), cancelled: false });
    cx.notify();

    cx.spawn_in(window, async move |this, cx| {
        let background = cx.background_executor().clone();
        // Folders are listed first, so the popup can count the files.
        let files = background.spawn(async move { crate::state::expand_folders(media) }).await;
        let total = projects.len() + files.len();
        let still_going = |this: &gpui_kit::WeakEntity<Workspace>, cx: &mut gpui_kit::AsyncWindowContext, name: Option<String>, read: bool| {
            this.update(cx, |this, cx| {
                let Some(job) = this.ui.import_job.as_mut() else { return false };
                job.total = Some(total);
                job.done += usize::from(read);
                if let Some(name) = name {
                    job.current = name;
                }
                cx.notify();
                !job.cancelled
            })
            .unwrap_or(false)
        };

        let mut read_projects = Vec::new();
        for (path, assets) in projects {
            if !still_going(&this, cx, Some(file_name(&path)), false) {
                return finish(&this, cx, None);
            }
            let reading = path.clone();
            let read = background.spawn(async move { crate::state::read_project_file(&reading, &assets) }).await;
            read_projects.push((path, read));
            still_going(&this, cx, None, true);
        }
        let mut read_files = Vec::new();
        for path in files {
            if !still_going(&this, cx, Some(file_name(&path)), false) {
                return finish(&this, cx, None);
            }
            read_files.push(background.spawn(async move { crate::state::read_media_file(path) }).await);
            still_going(&this, cx, None, true);
        }
        finish(&this, cx, Some((read_projects, read_files, then)));
    })
    .detach();
}

/// What was read: project files (each read or failed), then media files.
type Read = (Vec<(PathBuf, Result<ReadProject, String>)>, Vec<ReadFile>, Option<Then>);

/// Closes the popup and, unless the import was cancelled (`read` is `None`), adds what was read.
fn finish(this: &gpui_kit::WeakEntity<Workspace>, cx: &mut gpui_kit::AsyncWindowContext, read: Option<Read>) {
    this.update_in(cx, |this, window, cx| {
        let cancelled = this.ui.import_job.take().is_none_or(|job| job.cancelled);
        if let (false, Some((projects, files, then))) = (cancelled, read) {
            this.state.settle_drags();
            let items = this.add_imported(projects, files, window, cx);
            if let Some(then) = then {
                then(this, items, window, cx);
            }
        }
        cx.notify();
    })
    .ok();
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The popup over the editor while an import runs: what is being read, how far along it is,
/// and Cancel. It covers the window, so nothing is edited halfway through an import.
pub fn popup(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let job = workspace.ui.import_job.as_ref()?;
    let heading = match job.total {
        Some(total) if total > 1 => format!("Importing {} of {total}", (job.done + 1).min(total)),
        _ => "Importing".to_owned(),
    };
    let percent = job.total.filter(|&total| total > 0).map_or(0.0, |total| job.done as f32 / total as f32 * 100.0);
    let card = div()
        .id("import-popup")
        .w(px(340.0))
        .p_4()
        .flex()
        .flex_col()
        .gap_3()
        .rounded(px(12.0))
        .bg(hsla(color::POPOVER))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .shadow_lg()
        .text_size(px(size::TEXT))
        .text_color(hsla(color::TEXT))
        .child(div().flex().items_center().gap_2().child(Spinner::new().small()).child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(heading)))
        .child(div().text_color(hsla(color::MUTED)).truncate().child(if job.current.is_empty() { "Looking for files…".to_owned() } else { job.current.clone() }))
        .child(Progress::new("import-progress").value(percent))
        .child(
            div().flex().justify_end().child(
                Button::new("import-cancel").outline().small().label(if job.cancelled { "Cancelling…" } else { "Cancel" }).disabled(job.cancelled).on_click(cx.listener(|this, _, _, cx| {
                    if let Some(job) = this.ui.import_job.as_mut() {
                        job.cancelled = true;
                    }
                    cx.notify();
                })),
            ),
        )
        .test_support();
    // A dimmed backdrop that takes every click, with the card in the middle.
    let backdrop = div()
        .id("import-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(black().opacity(0.4))
        .occlude()
        .child(card);
    Some(deferred(backdrop).with_priority(4).into_any_element())
}
