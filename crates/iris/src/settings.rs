//! Iris's settings that every app shares: her name and whether her face blinks. They live in
//! one small file in a folder of their own (`Application Support/Iris` on a Mac), so a change
//! made in one app is what the others read the next time they load it. Each app keeps its own
//! copy too, and treats this file as the one that wins.

use std::path::{Path, PathBuf};

/// What Iris is called until someone gives her another name.
pub const DEFAULT_NAME: &str = "Iris";
/// The longest name she can be given, in characters, so the name fits beside her face.
pub const NAME_LIMIT: usize = 16;
/// The file the shared settings are kept in, inside the shared folder.
const FILE_NAME: &str = "settings.json";

/// The settings every app shares.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Shared {
    /// What she is called. Empty means `DEFAULT_NAME`.
    pub name: String,
    /// Whether her face blinks and glances about now and then.
    pub blinks: bool,
}

impl Default for Shared {
    fn default() -> Self {
        Self { name: DEFAULT_NAME.to_owned(), blinks: true }
    }
}

impl Shared {
    /// The name to show: the one given, tidied, or `DEFAULT_NAME` when none is.
    pub fn shown_name(&self) -> String {
        let given = tidy_name(&self.name);
        if given.is_empty() { DEFAULT_NAME.to_owned() } else { given }
    }
}

/// A typed name made fit to keep: no spaces around it, and no longer than `NAME_LIMIT`.
pub fn tidy_name(typed: &str) -> String {
    typed.trim().chars().take(NAME_LIMIT).collect::<String>().trim_end().to_owned()
}

/// The folder the shared settings live in: `~/Library/Application Support/Iris` on a Mac,
/// `$XDG_CONFIG_HOME/iris` (or `~/.config/iris`) elsewhere. `None` without a home folder.
pub fn default_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Application Support/Iris"));
    }
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home.map(|home| home.join(".config"))).map(|config| config.join("iris"))
}

/// The shared settings in `folder`. `None` when there are none yet (no app has written them),
/// so the caller can start them from its own. A damaged file reads as `None` too, and is
/// replaced by the next save.
pub fn load(folder: &Path) -> Option<Shared> {
    let text = std::fs::read_to_string(folder.join(FILE_NAME)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Writes the shared settings into `folder`, through a temporary file so another app reading
/// at the same moment never sees half a file.
pub fn save(folder: &Path, shared: &Shared) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let text = serde_json::to_string_pretty(shared).map_err(std::io::Error::other)?;
    let partial = folder.join(format!("{FILE_NAME}.{}.partial", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, folder.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("iris-settings-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn settings_written_by_one_app_are_read_by_another() {
        let folder = scratch_folder("shared");
        assert_eq!(load(&folder), None, "nothing written yet");
        let renamed = Shared { name: "Pip".to_owned(), blinks: false };
        save(&folder, &renamed).unwrap();
        assert_eq!(load(&folder), Some(renamed));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_damaged_file_reads_as_none_and_missing_fields_take_defaults() {
        let folder = scratch_folder("damaged");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(FILE_NAME), "{ not json").unwrap();
        assert_eq!(load(&folder), None);
        std::fs::write(folder.join(FILE_NAME), r#"{"name": "Pip"}"#).unwrap();
        assert_eq!(load(&folder), Some(Shared { name: "Pip".to_owned(), blinks: true }));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn names_are_tidied_and_blank_ones_fall_back() {
        assert_eq!(tidy_name("  A very long assistant name  "), "A very long assi");
        assert_eq!(Shared { name: "   ".to_owned(), blinks: true }.shown_name(), DEFAULT_NAME);
    }
}
