//! Saving and opening project files from the editor window: the file dialogs, writing files
//! safely, and asking about unsaved changes before they would be lost (opening another
//! project, quitting, closing the window).

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, PromptButton, PromptLevel, Window};
use studio_core::project_file::EXTENSION;

use crate::workspace::Workspace;

/// What to do once the project is safely saved (or the user chose not to save).
type Then = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Project files larger than this are refused rather than read into memory. Real projects
/// are a few hundred kilobytes; this only guards against opening the wrong file.
const MAX_PROJECT_BYTES: u64 = 256 * 1024 * 1024;

/// Writes `text` to `path` without ever leaving a half-written project behind: it goes to a
/// temporary file in the same folder first, which then replaces the old file in one step.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().map_or_else(|| "project".into(), |name| name.to_string_lossy().into_owned());
    let temporary = path.with_file_name(format!(".{name}.saving"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// Reads a project file, refusing anything implausibly large.
fn read_project(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|error| error.to_string())?.len();
    if size > MAX_PROJECT_BYTES {
        return Err("This file is too large to be a Studio project.".into());
    }
    std::fs::read_to_string(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => "This is not a Studio project file.".to_owned(),
        _ => error.to_string(),
    })
}

/// Where save dialogs start: next to the current project, else the Documents folder.
fn starting_folder(current: Option<&Path>) -> PathBuf {
    current
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."))
}

impl Workspace {
    /// Saves to the current file, or asks where to save the first time.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_then(window, cx, None);
    }

    /// Always asks where to save, then keeps working on the new file.
    pub fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_as_then(window, cx, None);
    }

    fn save_then(&mut self, window: &mut Window, cx: &mut Context<Self>, then: Option<Then>) {
        match self.state.file().map(Path::to_path_buf) {
            Some(path) => {
                if self.write_project(&path, window, cx)
                    && let Some(then) = then
                {
                    then(self, window, cx);
                }
            }
            None => self.save_as_then(window, cx, then),
        }
    }

    fn save_as_then(&mut self, window: &mut Window, cx: &mut Context<Self>, then: Option<Then>) {
        let folder = starting_folder(self.state.file());
        let suggested = format!("{}.{EXTENSION}", self.state.document_name());
        let chosen = cx.prompt_for_new_path(&folder, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut path))) = chosen.await else { return };
            if path.extension().is_none() {
                path.set_extension(EXTENSION);
            }
            this.update_in(cx, |this, window, cx| {
                if this.write_project(&path, window, cx)
                    && let Some(then) = then
                {
                    then(this, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Writes the project to `path`. Shows what went wrong and returns false if it could not.
    fn write_project(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let written = self.state.project_text().and_then(|text| write_atomically(path, &text).map_err(|error| error.to_string()));
        match written {
            Ok(()) => {
                self.state.mark_saved(path.to_path_buf());
                cx.notify();
                true
            }
            Err(error) => {
                show_error(window, cx, "The project could not be saved", error);
                false
            }
        }
    }

    /// Asks for a project file and opens it, after offering to save unsaved changes.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unless_cancelled(window, cx, Box::new(|_, window, cx| {
            let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open".into()),
            });
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = chosen.await else { return };
                let Some(path) = paths.into_iter().next() else { return };
                this.update_in(cx, |this, window, cx| this.open_file(path, window, cx)).ok();
            })
            .detach();
        }));
    }

    fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match read_project(&path).and_then(|text| self.state.open_project(&text, path)) {
            Ok(missing) => {
                self.request_proxies();
                if !missing.is_empty() {
                    let names: Vec<String> = missing.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                    let message = format!("Media not found: {}. Put the files next to the project to relink them.", names.join(", "));
                    window.push_notification(message.as_str(), cx);
                }
                cx.notify();
            }
            Err(error) => show_error(window, cx, "The project could not be opened", error),
        }
    }

    /// Quits, after offering to save unsaved changes. A running export is stopped first
    /// (after asking), so it can remove its partial file.
    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unless_cancelled(window, cx, Box::new(|this, window, cx| {
            if this.ui.export_job.is_none() {
                cx.quit();
                return;
            }
            let buttons = [PromptButton::ok("Stop Export and Quit"), PromptButton::cancel("Cancel")];
            let answer = window.prompt(PromptLevel::Warning, "An export is still running.", Some("Quitting now stops it."), &buttons, cx);
            cx.spawn_in(window, async move |this, cx| {
                if answer.await != Ok(0) {
                    return;
                }
                // The export's completion quits the app once its partial file is gone.
                this.update(cx, |this, _| {
                    this.ui.quit_after_export = true;
                    crate::panels::export::cancel(this);
                })
                .ok();
            })
            .detach();
        }));
    }

    /// Runs `then` right away if nothing is unsaved. Otherwise asks first: save and continue,
    /// continue without saving, or cancel (and do nothing).
    pub fn unless_cancelled(&mut self, window: &mut Window, cx: &mut Context<Self>, then: Then) {
        if !self.state.has_unsaved_changes() {
            then(self, window, cx);
            return;
        }
        let message = format!("Save changes to “{}”?", self.state.document_name());
        let buttons = [PromptButton::ok("Save"), PromptButton::new("Don’t Save"), PromptButton::cancel("Cancel")];
        let answer = window.prompt(PromptLevel::Warning, &message, Some("Your changes will be lost if you don’t save them."), &buttons, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(choice) = answer.await else { return };
            this.update_in(cx, |this, window, cx| match choice {
                0 => this.save_then(window, cx, Some(then)),
                1 => then(this, window, cx),
                _ => {}
            })
            .ok();
        })
        .detach();
    }
}

fn show_error(window: &mut Window, cx: &mut Context<Workspace>, title: &'static str, error: String) {
    window.open_alert_dialog(cx, move |dialog, _, _| dialog.title(title).description(error.clone()));
}
