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
        return Err("This file is too large to be an Omina Motion project.".into());
    }
    std::fs::read_to_string(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidData => "This is not an Omina Motion project file.".to_owned(),
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

/// Opens the projects the system hands over once Motion is running. Motion shows one project
/// at a time, so of several, the first is opened (after offering to save unsaved changes).
pub fn open_handed_over(workspace: &mut Workspace, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(path) = paths.into_iter().next() {
        crate::panels::start_screen::open_recent(workspace, path, window, cx);
    }
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
        // Which single property a clip was opened to isn't saved: clips reopen to all of them.
        self.ui.timeline.only_property.clear();
        // Dismissed suggestions were about the project that was open before.
        self.ui.assistant.dismissed.clear();
        // An answer about the project that was open would be wrong about the new one.
        self.ui.assistant.pending = None;
        // So would "too much" or "it" about what Iris last changed there.
        self.ui.assistant.last_change = None;
        self.ui.assistant.last_layers.clear();
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
            let message = format!("Omina Motion closed with unsaved changes to “{name}”.");
            let buttons = [PromptButton::ok("Restore"), PromptButton::new("Discard")];
            let answer = window.prompt(PromptLevel::Warning, &message, Some("Restore them to keep working where you left off."), &buttons, cx);
            self.ui.recovery_offered = true;
            cx.spawn_in(window, async move |this, cx| {
                let restore = answer.await == Ok(0);
                this.update_in(cx, |this, window, cx| {
                    this.ui.recovery_offered = false;
                    if restore {
                        this.restore_recovery(&text, settings.recovery_of, window, cx);
                        // A project double-clicked to launch Motion opens next, offering to
                        // save the restored changes first.
                        let handed_over = crate::opened_files::take_at_launch();
                        open_handed_over(this, handed_over, window, cx);
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

    /// Opens a project the system handed over (double-clicked, or named on the command line),
    /// else the last project if Settings asks for that and it is still there; otherwise shows
    /// the start screen.
    /// The very first time, the welcome setup shows over it.
    fn start(&mut self, settings: &app_settings::AppSettings, window: &mut Window, cx: &mut Context<Self>) {
        let handed_over = crate::opened_files::take_at_launch().into_iter().next();
        let last = settings.last_project.clone().filter(|path| path.exists());
        match (handed_over, last) {
            (Some(path), _) => self.open_file(path, window, cx),
            (None, Some(last)) if settings.start_with == app_settings::StartWith::LastProject => self.open_file(last, window, cx),
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

    /// Writes an auto-save copy beside the project (see `auto_save`) when Settings has it on, the
    /// project has been saved somewhere, it has changed since, and the last save or auto-save is
    /// older than Settings asks for. Says once if copies can't be written (a read-only folder).
    fn auto_save_if_due(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.ui.settings.auto_save;
        let Some(file) = self.state.file().map(Path::to_path_buf) else { return };
        let due = self.ui.last_auto_save_at.is_none_or(|at| at.elapsed().as_secs() >= u64::from(settings.every_minutes.max(1)) * 60);
        if !settings.on || !due || !self.state.has_unsaved_changes() {
            return;
        }
        let written = self
            .state
            .project_text_with(&self.editor_settings())
            .and_then(|text| crate::auto_save::write(&file, &text, settings.how_many as usize, chrono::Local::now()).map_err(|error| error.to_string()));
        self.ui.last_auto_save_at = Some(std::time::Instant::now());
        match written {
            Ok(_) => self.ui.auto_save_failing = false,
            Err(error) if !self.ui.auto_save_failing => {
                self.ui.auto_save_failing = true;
                window.push_notification(format!("Auto-save couldn't write a copy beside the project: {error}").as_str(), cx);
            }
            Err(_) => {}
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
        // The work as it is now is kept even when Settings doesn't keep versions, since the
        // restore replaces it; if it can't be kept, nothing is restored.
        let kept = self.state.project_text_with(&self.editor_settings()).and_then(|now| crate::versions::keep(&now, file.as_deref(), self.ui.settings.versions.how_many as usize).map_err(|error| error.to_string()));
        if let Err(error) = kept {
            return show_error(window, cx, "That version wasn't restored", format!("Your work as it is now couldn't be kept as a version first, so it was left as it is. {error}"));
        }
        self.ui.last_version_at = Some(std::time::Instant::now());
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

    /// Runs every `RECOVERY_INTERVAL`: keeps a timed version when one is due, keeps the recovery
    /// copy (unless Settings says never, or the last one is still being offered back), saying once if it can't be kept (a full or read-only
    /// disk), and tries again to save libraries that failed to save.
    pub fn keep_work_safe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keep_timed_version();
        self.auto_save_if_due(window, cx);
        let recovery = if self.ui.settings.recovery_seconds == 0 || self.ui.recovery_offered { Ok(()) } else { self.keep_recovery_copy() };
        match recovery {
            Ok(()) => self.ui.recovery_failing = false,
            Err(error) if !self.ui.recovery_failing => {
                self.ui.recovery_failing = true;
                window.push_notification(format!("Unsaved work can't be kept safe in case Omina Motion closes: {error}. Save your project to keep it.").as_str(), cx);
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

    /// Keeps the picture in the viewer as the saved project's thumbnail for the start screen
    /// (see `project_thumbnails`). It is scaled and written on a background thread; a picture
    /// that can't be saved only leaves the project without one, so it is reported on the
    /// terminal rather than interrupting the save.
    fn save_thumbnail(&self, path: &Path) {
        let Some(shown) = self.render.shown_image().cloned() else { return };
        let (path, now) = (path.to_path_buf(), app_settings::now_seconds());
        let spawned = std::thread::Builder::new().name("project thumbnail".into()).spawn(move || {
            let size = shown.size(0);
            let Some(bgra) = shown.as_bytes(0) else { return };
            if let Err(error) = crate::project_thumbnails::write(&path, bgra, size.width.0.max(0) as u32, size.height.0.max(0) as u32, now) {
                eprintln!("could not save the project's thumbnail: {error}");
            }
        });
        if let Err(error) = spawned {
            eprintln!("could not start saving the project's thumbnail: {error}");
        }
    }

    /// Writes the project to `path`. Shows what went wrong and returns false if it could not.
    fn write_project(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Iris's recipes set to run before each save run first, so what they change is saved.
        crate::panels::assistant_automation::at(self, iris::recipes::Trigger::BeforeSave, window, cx);
        let text = self.state.project_text_with(&self.editor_settings());
        let written = text.as_ref().map_err(Clone::clone).and_then(|text| write_atomically(path, text).map_err(|error| error.to_string()));
        match written {
            Ok(()) => {
                // Each save is also a version to come back to.
                if let Ok(text) = &text {
                    self.keep_version(text, Some(path));
                }
                self.state.mark_saved(path.to_path_buf(), cx.has_active_drag());
                // The next auto-save is a full interval after this save.
                self.ui.last_auto_save_at = Some(std::time::Instant::now());
                // Saved: nothing left to recover, and this is the project to reopen next time.
                app_settings::clear_recovery();
                app_settings::note_project_used(path);
                self.save_thumbnail(path);
                // A brief notice, so a save (⌘S especially) is seen to have worked.
                let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                window.push_notification(format!("Saved “{name}”").as_str(), cx);
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
                // Iris's recipes set to run whenever something is opened.
                crate::panels::assistant_automation::at(self, iris::recipes::Trigger::OnOpen, window, cx);
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
            // A recovery copy still being offered back is left for the next launch.
            if !self.ui.recovery_offered {
                clear_stale_recovery();
            }
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
    crate::panels::chrome::open_alert_dialog(window, cx, move |dialog, _, _| dialog.title(title).description(error.clone()));
}
