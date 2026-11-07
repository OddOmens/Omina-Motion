//! What Studio remembers between runs, apart from projects themselves: the projects opened
//! lately (listed on the start screen), the window's size and position, and a recovery copy of
//! unsaved work (offered back if Studio closed without saving it).
//!
//! It lives in the platform's settings folder (macOS: `~/Library/Application Support/Studio`).
//! Tests use a temporary folder of their own instead, so they never touch real settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The window's placement, in screen points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub maximized: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// The project open when Studio last quit (or was last saved or opened).
    pub last_project: Option<PathBuf>,
    pub window: Option<WindowPlacement>,
    /// For the recovery copy: the project file it is unsaved work on, if it had one.
    pub recovery_of: Option<PathBuf>,
    pub theme: crate::theme::ThemeName,
    pub accent: crate::theme::Accent,
    /// The most disk space proxies may take, in gigabytes; `None` for no limit.
    pub proxy_cache_limit_gb: Option<u32>,
    /// Where Studio keeps what it can rebuild (proxies, background-removal masks); `None` for
    /// the platform's cache folder.
    pub cache_folder: Option<PathBuf>,
    /// Whether imported footage gets a proxy built in the background.
    pub build_proxies: bool,
    /// Which columns the Project panel shows.
    pub project_columns: ProjectColumns,
    /// Folders every new project starts with.
    pub default_folders: Vec<String>,
    /// Which folder imported files go into, by kind (`None` leaves them at the top).
    pub import_folders: ImportFolders,
    /// What Studio shows when it starts: the start screen, or the last project.
    pub start_with: StartWith,
    /// The projects opened or saved lately, newest first, for the start screen and Open Recent.
    pub recent_projects: Vec<RecentProject>,
    /// When Studio last reached its update feed, in seconds since 1970.
    pub last_update_check: u64,
    /// How often unsaved work is copied aside for recovery, in seconds (0: never).
    pub recovery_seconds: u32,
    /// What new compositions (and new projects) start as.
    pub new_composition: NewComposition,
    /// How sharp the viewer is while playing.
    pub playback_quality: PlaybackQuality,
    /// Whether playback starts over at the end of the work area, for new sessions.
    pub loop_playback: bool,
    /// Whether snapping starts on.
    pub snapping: bool,
    /// Whether the viewer's anchor points start snapping to their layer's corners and middle.
    pub anchor_snapping: bool,
    /// What shows behind the transparent parts of a composition in the viewer.
    pub transparency: Transparency,
    /// Pictures and waveforms drawn on timeline clips.
    pub clip_thumbnails: bool,
    pub clip_waveforms: bool,
    /// The frame render time under the timeline.
    pub show_render_time: bool,
    /// Short timeline rows: each clip shows just its name strip, so more tracks fit.
    pub compact_timeline: bool,
    /// Background removal rests the GPU between frames: slower, but cool and quiet.
    pub cooler_background_removal: bool,
    /// Which AI features are shown (Settings > AI Models).
    pub ai: AiFeatures,
    /// Keyboard shortcuts: a starting set and the person's own changes.
    pub shortcuts: crate::keymap::Shortcuts,
    /// Earlier versions of projects, kept to roll back to (see `versions`).
    pub versions: VersionSettings,
    /// Memory set aside for frames rendered ahead of the playhead, in megabytes (0 is off).
    pub render_ahead_mb: u32,
    /// The movie an export was writing when Studio last ran, until it finished: if Studio quit
    /// or crashed first, its unfinished hidden file is removed on the next launch.
    pub export_in_progress: Option<PathBuf>,
    /// The same for File ▸ Package…: the zip being written, until it is finished.
    pub package_in_progress: Option<PathBuf>,
    /// The eyedropper's current color, the colors it picked lately, and the saved swatches.
    pub colors: SavedColors,
}

/// What Studio shows when it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartWith {
    /// The start screen: recent projects, a new or opened project, or a template.
    #[default]
    StartScreen,
    /// The project that was open last, straight away (the start screen if there is none).
    LastProject,
}

/// A project on the recent list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: PathBuf,
    /// When it was last opened or saved, in seconds since 1970.
    pub opened: u64,
}

/// How many projects the recent list keeps.
pub const RECENT_LIMIT: usize = 20;

/// Puts `path` at the top of the recent list (moving it if it was already there), stamped `now`
/// (seconds since 1970), and keeps the list to `RECENT_LIMIT`.
pub fn add_recent(recent: &mut Vec<RecentProject>, path: &Path, now: u64) {
    recent.retain(|project| project.path != path);
    recent.insert(0, RecentProject { path: path.to_path_buf(), opened: now });
    recent.truncate(RECENT_LIMIT);
}

/// Seconds since 1970, for stamping recent projects.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// Notes that the project at `path` was just opened or saved: it is the one to reopen, and goes
/// to the top of the recent list.
pub fn note_project_used(path: &Path) {
    update(|settings| {
        settings.last_project = Some(path.to_path_buf());
        add_recent(&mut settings.recent_projects, path, now_seconds());
    });
}

/// Takes `path` off the recent list.
pub fn forget_recent(path: &Path) {
    update(|settings| settings.recent_projects.retain(|project| project.path != path));
}

/// Colors kept from one session to the next, like Photoshop's foreground color and swatches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedColors {
    pub current: studio_core::model::Rgba,
    /// The latest picks, newest first.
    pub recent: Vec<studio_core::model::Rgba>,
    pub swatches: Vec<studio_core::model::Rgba>,
}

impl Default for SavedColors {
    fn default() -> Self {
        Self { current: studio_core::model::Rgba::hex(0xffffff), recent: Vec::new(), swatches: Vec::new() }
    }
}

/// The features that use AI models, each of which can be hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiFeatures {
    pub captions: bool,
    pub remove_background: bool,
    pub auto_reframe: bool,
    pub enhance_speech: bool,
}

impl Default for AiFeatures {
    fn default() -> Self {
        Self::all(true)
    }
}

impl AiFeatures {
    /// Every feature shown, or every one hidden.
    pub fn all(on: bool) -> Self {
        Self { captions: on, remove_background: on, auto_reframe: on, enhance_speech: on }
    }

    /// Whether any is shown.
    pub fn any(self) -> bool {
        self.captions || self.remove_background || self.auto_reframe || self.enhance_speech
    }
}

/// How earlier versions of projects are kept (File ▸ Browse Versions…).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionSettings {
    pub keep: bool,
    /// Minutes between versions while a project changes (one is also kept on each save).
    pub every_minutes: u32,
    /// How many each project keeps; the oldest go first.
    pub how_many: u32,
}

impl Default for VersionSettings {
    fn default() -> Self {
        Self { keep: true, every_minutes: 10, how_many: 30 }
    }
}

/// What new compositions start as.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewComposition {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub seconds: u32,
}

impl Default for NewComposition {
    fn default() -> Self {
        Self { width: 1920, height: 1080, fps: 30, seconds: 10 }
    }
}

/// How sharp the viewer is while playing; paused frames are always full quality.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackQuality {
    /// Half resolution on high-density screens, full elsewhere.
    #[default]
    Auto,
    Full,
    Half,
    Quarter,
}

impl PlaybackQuality {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Full, Self::Half, Self::Quarter];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Full => "Full",
            Self::Half => "Half",
            Self::Quarter => "Quarter",
        }
    }

    /// Device pixels per screen point to render at while playing, on a screen of `density`.
    pub fn density(self, density: f32) -> f32 {
        match self {
            Self::Auto => density.min(1.0),
            Self::Full => density,
            Self::Half => density * 0.5,
            Self::Quarter => density * 0.25,
        }
    }
}

/// What shows behind transparent parts of a composition in the viewer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transparency {
    #[default]
    Checkerboard,
    Black,
    Gray,
    White,
}

impl Transparency {
    pub const ALL: [Self; 4] = [Self::Checkerboard, Self::Black, Self::Gray, Self::White];

    pub fn name(self) -> &'static str {
        match self {
            Self::Checkerboard => "Checkerboard",
            Self::Black => "Black",
            Self::Gray => "Gray",
            Self::White => "White",
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            last_project: None,
            window: None,
            recovery_of: None,
            theme: Default::default(),
            accent: Default::default(),
            proxy_cache_limit_gb: None,
            cache_folder: None,
            build_proxies: true,
            project_columns: ProjectColumns::default(),
            default_folders: ["Footage", "Images", "Music", "Sound Effects", "Voice Over"].map(String::from).to_vec(),
            import_folders: ImportFolders::default(),
            start_with: StartWith::default(),
            recent_projects: Vec::new(),
            last_update_check: 0,
            recovery_seconds: 30,
            new_composition: NewComposition::default(),
            playback_quality: PlaybackQuality::default(),
            loop_playback: true,
            snapping: true,
            anchor_snapping: true,
            transparency: Transparency::default(),
            clip_thumbnails: true,
            clip_waveforms: true,
            show_render_time: true,
            compact_timeline: false,
            cooler_background_removal: false,
            ai: AiFeatures::default(),
            shortcuts: crate::keymap::Shortcuts::default(),
            versions: VersionSettings::default(),
            // Tests turn it on where they need it, so the viewer stays predictable elsewhere.
            render_ahead_mb: if cfg!(test) { 0 } else { 2048 },
            export_in_progress: None,
            package_in_progress: None,
            colors: SavedColors::default(),
        }
    }
}

/// The Project panel's optional columns (the name is always shown).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectColumns {
    pub label: bool,
    pub kind: bool,
    pub duration: bool,
    pub dimensions: bool,
    pub frame_rate: bool,
}

impl Default for ProjectColumns {
    fn default() -> Self {
        Self { label: true, kind: true, duration: false, dimensions: false, frame_rate: false }
    }
}

/// The folder each kind of imported file is put in, by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportFolders {
    pub video: Option<String>,
    pub image: Option<String>,
    pub audio: Option<String>,
}

impl Default for ImportFolders {
    fn default() -> Self {
        Self { video: Some("Footage".into()), image: Some("Images".into()), audio: Some("Music".into()) }
    }
}

/// The folder Studio keeps its caches in: the one chosen in Settings, or the platform's.
pub fn cache_root() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("studio-cache-test-{}", std::process::id()));
    }
    load().cache_folder.unwrap_or_else(studio_media::default_cache_root)
}

/// Studio's settings folder.
pub(crate) fn folder() -> PathBuf {
    if cfg!(test) {
        // Per test thread, so tests running side by side don't share settings.
        let thread = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
        return std::env::temp_dir().join(format!("studio-settings-test-{}-{thread}", std::process::id()));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|home| home.join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|home| home.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Studio")
}

fn settings_path() -> PathBuf {
    folder().join("settings.json")
}

/// Where unsaved work is kept until it is saved or thrown away.
pub fn recovery_path() -> PathBuf {
    folder().join("recovery.studio")
}

/// The saved settings, or the defaults if there are none. A setting that can't be read (one
/// written by a newer Studio, say) falls back to its default on its own, keeping the rest. A file
/// that can't be read in full is copied aside first (`settings.damaged.json`), since the next
/// save replaces it.
pub fn load() -> AppSettings {
    let Ok(text) = std::fs::read_to_string(settings_path()) else { return AppSettings::default() };
    if let Ok(settings) = serde_json::from_str(&text) {
        return settings;
    }
    if let Err(error) = std::fs::write(folder().join("settings.damaged.json"), &text) {
        eprintln!("could not keep a copy of the unreadable settings: {error}");
    }
    serde_json::from_str::<serde_json::Value>(&text).map_or_else(|_| AppSettings::default(), |saved| readable_part(&saved))
}

/// The settings in `saved` that can be read, each over the default.
fn readable_part(saved: &serde_json::Value) -> AppSettings {
    let Ok(serde_json::Value::Object(mut kept)) = serde_json::to_value(AppSettings::default()) else { return AppSettings::default() };
    for (name, value) in saved.as_object().into_iter().flatten() {
        let mut trial = kept.clone();
        trial.insert(name.clone(), value.clone());
        if serde_json::from_value::<AppSettings>(serde_json::Value::Object(trial)).is_ok() {
            kept.insert(name.clone(), value.clone());
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

/// Saves the settings. Failing to (a read-only disk, say) only loses what is remembered, so it
/// is reported on the terminal rather than interrupting work.
pub fn save(settings: &AppSettings) {
    let written = std::fs::create_dir_all(folder())
        .and_then(|()| serde_json::to_string_pretty(settings).map_err(std::io::Error::other))
        .and_then(|text| write_atomically(&settings_path(), &text));
    if let Err(error) = written {
        eprintln!("could not save Studio's settings: {error}");
    }
}

/// Changes the settings with `change` and saves them.
pub fn update(change: impl FnOnce(&mut AppSettings)) {
    let mut settings = load();
    change(&mut settings);
    save(&settings);
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file. The new
/// text is flushed to the disk before the rename, so a power cut can't leave an empty file in
/// its place. A symbolic link is written through (the file it points to is replaced), so the
/// link stays a link, and the file keeps its permissions (one only its owner may read stays so).
pub fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let path = if path.is_symlink() { std::fs::canonicalize(path)? } else { path.to_path_buf() };
    let name = path.file_name().map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned());
    let temporary = path.with_file_name(format!(".{name}.saving"));
    let written = std::fs::File::create(&temporary).and_then(|mut file| {
        if let Ok(existing) = std::fs::metadata(&path) {
            file.set_permissions(existing.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    written.and_then(|()| std::fs::rename(&temporary, &path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    // The rename itself is recorded in the folder; flush that too where the system allows it.
    #[cfg(unix)]
    if let Some(folder) = path.parent().and_then(|folder| std::fs::File::open(folder).ok()) {
        let _ = folder.sync_all();
    }
    Ok(())
}

/// Keeps a recovery copy of unsaved work, noting which project file it belongs to.
pub fn write_recovery(text: &str, of: Option<&Path>) -> std::io::Result<()> {
    std::fs::create_dir_all(folder()).and_then(|()| write_atomically(&recovery_path(), text))?;
    update(|settings| settings.recovery_of = of.map(Path::to_path_buf));
    Ok(())
}

/// Removes the recovery copy, once the work is saved or deliberately thrown away.
pub fn clear_recovery() {
    let _ = std::fs::remove_file(recovery_path());
    update(|settings| settings.recovery_of = None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn saving_keeps_a_files_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let folder = std::env::temp_dir().join(format!("studio-permissions-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("private.studio");
        std::fs::write(&file, "old").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&file, "new").unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn saving_through_a_link_keeps_the_link() {
        let folder = std::env::temp_dir().join(format!("studio-atomic-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (real, link) = (folder.join("real.studio"), folder.join("link.studio"));
        std::fs::write(&real, "old").unwrap();
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write_atomically(&link, "new").unwrap();
        assert!(link.is_symlink(), "still a link");
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "new", "the file it points to was saved");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_setting_that_cant_be_read_doesnt_lose_the_others() {
        std::fs::create_dir_all(folder()).unwrap();
        // A theme from a newer Studio, next to settings this one knows.
        std::fs::write(settings_path(), r#"{"theme": "Sepia", "loop_playback": true, "recovery_seconds": 90}"#).unwrap();
        let settings = load();
        assert_eq!(settings.theme, AppSettings::default().theme, "the unknown theme falls back");
        assert!(settings.loop_playback);
        assert_eq!(settings.recovery_seconds, 90, "the rest are kept");
        let copy = std::fs::read_to_string(folder().join("settings.damaged.json")).unwrap();
        assert!(copy.contains("Sepia"), "the original is kept aside");
        std::fs::write(settings_path(), "{ not json").unwrap();
        assert_eq!(load(), AppSettings::default());
        assert_eq!(std::fs::read_to_string(folder().join("settings.damaged.json")).unwrap(), "{ not json");
        std::fs::remove_file(settings_path()).unwrap();
    }

    #[test]
    fn the_recent_list_puts_the_latest_first_without_repeats() {
        let mut recent = Vec::new();
        add_recent(&mut recent, Path::new("/a.studio"), 1);
        add_recent(&mut recent, Path::new("/b.studio"), 2);
        add_recent(&mut recent, Path::new("/a.studio"), 3);
        assert_eq!(recent, [RecentProject { path: "/a.studio".into(), opened: 3 }, RecentProject { path: "/b.studio".into(), opened: 2 }]);
        for index in 0..30 {
            add_recent(&mut recent, Path::new(&format!("/{index}.studio")), 10 + index);
        }
        assert_eq!(recent.len(), RECENT_LIMIT);
        assert_eq!(recent[0].path, Path::new("/29.studio"));
    }

    #[test]
    fn settings_and_recovery_round_trip() {
        let placement = WindowPlacement { x: 10.0, y: 20.0, width: 1400.0, height: 900.0, maximized: false };
        update(|settings| {
            settings.last_project = Some("/projects/Promo.studio".into());
            settings.window = Some(placement);
        });
        let loaded = load();
        assert_eq!(loaded.last_project.as_deref(), Some(Path::new("/projects/Promo.studio")));
        assert_eq!(loaded.window, Some(placement));

        write_recovery("{\"unsaved\": true}", Some(Path::new("/projects/Promo.studio"))).unwrap();
        assert!(recovery_path().exists());
        assert_eq!(load().recovery_of.as_deref(), Some(Path::new("/projects/Promo.studio")));
        clear_recovery();
        assert!(!recovery_path().exists() && load().recovery_of.is_none());
        let _ = std::fs::remove_dir_all(folder());
    }
}
