//! What Studio remembers between runs, apart from projects themselves: the last project
//! (reopened on launch), the window's size and position, and a recovery copy of unsaved
//! work (offered back if Studio closed without saving it).
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// The project open when Studio last quit (or was last saved or opened).
    pub last_project: Option<PathBuf>,
    pub window: Option<WindowPlacement>,
    /// For the recovery copy: the project file it is unsaved work on, if it had one.
    pub recovery_of: Option<PathBuf>,
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

/// The saved settings, or the defaults if there are none (or they can't be read).
pub fn load() -> AppSettings {
    std::fs::read_to_string(settings_path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
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

/// Writes through a temporary file and a rename, so a crash never leaves half a file.
pub fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned());
    let temporary = path.with_file_name(format!(".{name}.saving"));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

/// Keeps a recovery copy of unsaved work, noting which project file it belongs to.
pub fn write_recovery(text: &str, of: Option<&Path>) {
    let written = std::fs::create_dir_all(folder()).and_then(|()| write_atomically(&recovery_path(), text));
    match written {
        Ok(()) => update(|settings| settings.recovery_of = of.map(Path::to_path_buf)),
        Err(error) => eprintln!("could not keep a recovery copy: {error}"),
    }
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
    fn settings_and_recovery_round_trip() {
        let placement = WindowPlacement { x: 10.0, y: 20.0, width: 1400.0, height: 900.0, maximized: false };
        update(|settings| {
            settings.last_project = Some("/projects/Promo.studio".into());
            settings.window = Some(placement);
        });
        let loaded = load();
        assert_eq!(loaded.last_project.as_deref(), Some(Path::new("/projects/Promo.studio")));
        assert_eq!(loaded.window, Some(placement));

        write_recovery("{\"unsaved\": true}", Some(Path::new("/projects/Promo.studio")));
        assert!(recovery_path().exists());
        assert_eq!(load().recovery_of.as_deref(), Some(Path::new("/projects/Promo.studio")));
        clear_recovery();
        assert!(!recovery_path().exists() && load().recovery_of.is_none());
        let _ = std::fs::remove_dir_all(folder());
    }
}
