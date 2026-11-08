//! Saving and opening project files from the editor window: the file dialogs, writing files
//! safely, and asking about unsaved changes before they would be lost (opening another
//! project, quitting, closing the window). Also the session around them: the start screen (or
//! the last project) on launch, the recent projects list, the window keeping its place, and unsaved work is kept in a recovery
//! copy every half minute, offered back if Studio closed without saving it.

use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, PromptButton, PromptLevel, Window, WindowBounds};
use studio_core::project_file::{EXTENSION, EditorSettings};

use crate::app_settings::{self, WindowPlacement, write_atomically};
use crate::workspace::Workspace;

/// How often unsaved work is copied to the recovery file (in the app; tests call it directly).
#[cfg_attr(test, allow(dead_code))]
pub const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// What to do once the project is safely saved (or the user chose not to save).
type Then = Box<dyn FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Project files larger than this are refused rather than read into memory. Real projects
/// are a few hundred kilobytes; this only guards against opening the wrong file.
const MAX_PROJECT_BYTES: u64 = 256 * 1024 * 1024;

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
    /// How the editor is set up: the state's settings plus the timeline's open clips.
    pub fn editor_settings(&self) -> EditorSettings {
        let mut open_clips: Vec<_> = self.ui.timeline.expanded_clips.iter().copied().collect();
        open_clips.sort();
        EditorSettings { open_clips, ..self.state.editor_settings() }
    }

    /// Restores the window's part of saved settings (the state restores its own on opening).
    fn apply_settings(&mut self, settings: &EditorSettings) {
        self.ui.timeline.expanded_clips = settings.open_clips.iter().copied().collect();
    }

    /// On launch: offers back unsaved work if Studio closed without saving it, otherwise
    /// shows the start screen or reopens the project that was open last, as Settings says.
    pub fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        crate::panels::export::remove_unfinished_export();
        crate::panels::package::remove_unfinished_package();
        let settings = app_settings::load();
        let recovery = app_settings::recovery_path();
        if let Ok(text) = std::fs::read_to_string(&recovery) {
            let name = settings.recovery_of.as_deref().and_then(|path| path.file_stem()).map_or_else(|| "Untitled".to_owned(), |name| name.to_string_lossy().into_owned());
            let message = format!("Studio closed with unsaved changes to “{name}”.");
            let buttons = [PromptButton::ok("Restore"), PromptButton::new("Discard")];
            let answer = window.prompt(PromptLevel::Warning, &message, Some("Restore them to keep working where you left off."), &buttons, cx);
            cx.spawn_in(window, async move |this, cx| {
                let restore = answer.await == Ok(0);
                this.update_in(cx, |this, window, cx| {
                    if restore {
                        this.restore_recovery(&text, settings.recovery_of, window, cx);
                    } else {
                        app_settings::clear_recovery();
                        this.start(&settings, window, cx);
                    }
                })
                .ok();
            })
            .detach();
            return;
        }
        self.start(&settings, window, cx);
    }

    /// Opens the last project if Settings asks for that and it is still there; otherwise shows
    /// the start screen.
    /// The very first time, the welcome setup shows over it.
    fn start(&mut self, settings: &app_settings::AppSettings, window: &mut Window, cx: &mut Context<Self>) {
        let last = settings.last_project.clone().filter(|path| path.exists());
        match last {
            Some(last) if settings.start_with == app_settings::StartWith::LastProject => self.open_file(last, window, cx),
            _ => crate::panels::start_screen::show(self, window, cx),
        }
        if !settings.onboarded {
            crate::panels::onboarding::show(self, cx);
        }
    }

    /// Opens the recovery copy as unsaved work on `of` (its project file, if it had one).
    fn restore_recovery(&mut self, text: &str, of: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let path = of.clone().unwrap_or_else(app_settings::recovery_path);
        match self.state.open_project(text, path) {
            Ok((_, settings)) => {
                self.apply_settings(&settings);
                // It is unsaved work: on the project file it came from, or untitled.
                self.state.mark_restored_unsaved(of);
                self.request_proxies();
                cx.notify();
            }
            Err(error) => show_error(window, cx, "The unsaved changes could not be restored", error),
        }
    }

    /// Copies unsaved work to the recovery file (run every `RECOVERY_INTERVAL`). Returns why it
    /// couldn't, so the user can be told their work isn't being kept safe.
    pub fn keep_recovery_copy(&self) -> Result<(), String> {
        if !self.state.has_unsaved_changes() {
            // Back to how it was saved (by undoing, say): an older copy would offer back work
            // that was taken out on purpose.
            clear_stale_recovery();
            return Ok(());
        }
        let text = self.state.project_text_with(&self.editor_settings())?;
        app_settings::write_recovery(&text, self.state.file()).map_err(|error| error.to_string())
    }

    /// Keeps `text` as a version of the project at `file`, if Settings keeps versions.
    fn keep_version(&mut self, text: &str, file: Option<&Path>) {
        let settings = self.ui.settings.versions;
        if !settings.keep {
            return;
        }
        match crate::versions::keep(text, file, settings.how_many as usize) {
            Ok(_) => self.ui.last_version_at = Some(std::time::Instant::now()),
            Err(error) => eprintln!("could not keep a version of the project: {error}"),
        }
    }

    /// Keeps a version of unsaved work if the last one is older than Settings asks for.
    pub fn keep_timed_version(&mut self) {
        let settings = self.ui.settings.versions;
        let due = self.ui.last_version_at.is_none_or(|at| at.elapsed().as_secs() >= u64::from(settings.every_minutes.max(1)) * 60);
        if !settings.keep || !due || !self.state.has_unsaved_changes() {
            return;
        }
        if let Ok(text) = self.state.project_text_with(&self.editor_settings()) {
            let file = self.state.file().map(Path::to_path_buf);
            self.keep_version(&text, file.as_deref());
        }
    }

    /// Restores version `version` of the project as unsaved changes, after keeping the work as
    /// it is now as a version too (so the restore can be undone by restoring that).
    pub fn restore_version(&mut self, version: &crate::versions::Version, window: &mut Window, cx: &mut Context<Self>) {
        let text = match std::fs::read_to_string(&version.path) {
            Ok(text) => text,
            Err(error) => return show_error(window, cx, "That version couldn't be read", error.to_string()),
        };
        let file = self.state.file().map(Path::to_path_buf);
        if let Ok(now) = self.state.project_text_with(&self.editor_settings()) {
            self.keep_version(&now, file.as_deref());
        }
        let path = file.clone().unwrap_or_else(app_settings::recovery_path);
        match self.state.open_project(&text, path) {
            Ok((_, settings)) => {
                self.apply_settings(&settings);
                self.state.mark_restored_unsaved(file);
                self.request_proxies();
                cx.notify();
            }
            Err(error) => show_error(window, cx, "That version couldn't be restored", error),
        }
    }

    /// Runs every `RECOVERY_INTERVAL`: keeps the recovery copy, saying once if it can't be kept
    /// (a full or read-only disk), and tries again to save libraries that failed to save.
    pub fn keep_work_safe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_timed_version();
        match self.keep_recovery_copy() {
            Ok(()) => self.ui.recovery_failing = false,
            Err(error) if !self.ui.recovery_failing => {
                self.ui.recovery_failing = true;
                window.push_notification(format!("Unsaved work can't be kept safe in case Studio closes: {error}. Save your project to keep it.").as_str(), cx);
            }
            Err(_) => {}
        }
        if let Some(back_up) = self.ui.library_save_failed.take() {
            // Tests never write the real libraries file.
            #[cfg(not(test))]
            if crate::library_store::save(&self.state.libraries, back_up).is_err() {
                self.ui.library_save_failed = Some(back_up);
            }
            #[cfg(test)]
            let _ = back_up;
        }
    }

    /// Remembers the window's size and position and the open project, for the next launch.
    pub fn remember_session(&self, window: &Window) {
        let placement = match window.window_bounds() {
            WindowBounds::Windowed(bounds) => WindowPlacement {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
                maximized: false,
            },
            WindowBounds::Maximized(bounds) | WindowBounds::Fullscreen(bounds) => WindowPlacement {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
                maximized: true,
            },
        };
        let project = self.state.file().map(Path::to_path_buf);
        app_settings::update(|settings| {
            settings.window = Some(placement);
            if project.is_some() {
                settings.last_project = project;
            }
        });
    }

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
        let text = self.state.project_text_with(&self.editor_settings());
        let written = text.as_ref().map_err(Clone::clone).and_then(|text| write_atomically(path, text).map_err(|error| error.to_string()));
        match written {
            Ok(()) => {
                // Each save is also a version to come back to.
                if let Ok(text) = &text {
                    self.keep_version(text, Some(path));
                }
                self.state.mark_saved(path.to_path_buf(), cx.has_active_drag());
                // Saved: nothing left to recover, and this is the project to reopen next time.
                app_settings::clear_recovery();
                app_settings::note_project_used(path);
                crate::workspace::refresh_menus(cx);
                cx.notify();
                true
            }
            Err(error) => {
                show_error(window, cx, "The project could not be saved", error);
                false
            }
        }
    }

    /// Starts a new, empty project, after offering to save unsaved changes.
    pub fn new_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unless_cancelled(window, cx, Box::new(|this, _, cx| {
            this.begin_new_project();
            cx.notify();
        }));
    }

    /// Closes the project (after offering to save unsaved changes) and shows the start screen,
    /// like After Effects' Close Project. An empty project is left behind it.
    pub fn close_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unless_cancelled(window, cx, Box::new(|this, window, cx| {
            this.begin_new_project();
            crate::panels::start_screen::show(this, window, cx);
        }));
    }

    /// Replaces the project with a new, empty one, without asking (callers ask first).
    pub fn begin_new_project(&mut self) {
        self.begin_new_project_sized(None);
    }

    /// The same, with its first composition at `size` rather than Settings' new-composition size.
    pub fn begin_new_project_sized(&mut self, size: Option<app_settings::NewComposition>) {
        let settings = app_settings::load();
        self.state.new_project(&settings.default_folders, size.unwrap_or(settings.new_composition));
        app_settings::update(|app| app.last_project = None);
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

    pub fn open_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        match read_project(&path).and_then(|text| self.state.open_project(&text, path.clone())) {
            Ok((missing, settings)) => {
                self.apply_settings(&settings);
                app_settings::note_project_used(&path);
                crate::workspace::refresh_menus(cx);
                crate::panels::start_screen::hide(self);
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

    /// Quits, after offering to save unsaved changes. A running export or package is stopped
    /// first, after asking. An export removes its partial file before the app quits; a
    /// package's partial file is removed the next time Studio starts.
    pub fn quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unless_cancelled(window, cx, Box::new(|this, window, cx| {
            this.remember_session(window);
            let (title, stop) = match (this.ui.export_job.is_some(), this.ui.package_job.is_some()) {
                (false, false) => {
                    cx.quit();
                    return;
                }
                (true, false) => ("An export is still running.", "Stop Export and Quit"),
                (false, true) => ("A package is still being made.", "Stop Packaging and Quit"),
                (true, true) => ("An export and a package are still running.", "Stop Both and Quit"),
            };
            let buttons = [PromptButton::ok(stop), PromptButton::cancel("Cancel")];
            let answer = window.prompt(PromptLevel::Warning, title, Some("Quitting now stops it."), &buttons, cx);
            cx.spawn_in(window, async move |this, cx| {
                if answer.await != Ok(0) {
                    return;
                }
                this.update(cx, |this, cx| {
                    crate::panels::package::cancel(this);
                    if this.ui.export_job.is_some() {
                        // The export's completion quits the app once its partial file is gone.
                        this.ui.quit_after_export = true;
                        crate::panels::export::cancel(this);
                    } else {
                        cx.quit();
                    }
                })
                .ok();
            })
            .detach();
        }));
    }

    /// Runs `then` right away if nothing is unsaved. Otherwise asks first: save and continue,
    /// continue without saving, or cancel (and do nothing). It doesn't ask again on the start
    /// screen when the person already chose not to save on the way there.
    pub fn unless_cancelled(&mut self, window: &mut Window, cx: &mut Context<Self>, then: Then) {
        if !self.state.has_unsaved_changes() || crate::panels::start_screen::changes_declined(self) {
            clear_stale_recovery();
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
                // Thrown away on purpose: there is nothing to recover.
                1 => {
                    app_settings::clear_recovery();
                    then(this, window, cx)
                }
                _ => {}
            })
            .ok();
        })
        .detach();
    }
}

/// Removes a recovery copy left from before the project was back to how it was saved. Checks
/// first, so the settings file is not rewritten when there is nothing to remove.
fn clear_stale_recovery() {
    if app_settings::recovery_path().exists() {
        app_settings::clear_recovery();
    }
}

fn show_error(window: &mut Window, cx: &mut Context<Workspace>, title: &'static str, error: String) {
    window.open_alert_dialog(cx, move |dialog, _, _| dialog.title(title).description(error.clone()));
}
