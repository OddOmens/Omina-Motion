//! Where libraries live on this computer: `libraries.json` in Studio's settings folder
//! (macOS: `~/Library/Application Support/Studio`), outside every project, so deleting or
//! moving a project never touches them.
//!
//! Writes go through a temporary file and a rename, so a crash can't leave half a file. Before
//! a change replaces the file, the old one is copied into `Library Backups` (at most one copy
//! every few minutes, and always before something is deleted); the newest `KEEP_BACKUPS` copies
//! are kept. If the file is ever damaged, the newest good backup is loaded instead and the
//! damaged file is set aside rather than overwritten.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use studio_core::library::Libraries;

use crate::app_settings::{folder, write_atomically};

/// How many backups are kept.
const KEEP_BACKUPS: usize = 30;
/// Ordinary changes are backed up at most this often (deletions always are).
const BACKUP_INTERVAL: Duration = Duration::from_secs(5 * 60);

fn library_path() -> PathBuf {
    folder().join("libraries.json")
}

fn backup_folder() -> PathBuf {
    folder().join("Library Backups")
}

fn now_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |time| time.as_millis())
}

/// Backups, newest first. Their names start with the time they were made, so names sort by age.
fn backups() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(backup_folder())
        .map(|entries| entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).filter(|path| path.extension().is_some_and(|ext| ext == "json")).collect())
        .unwrap_or_default();
    found.sort();
    found.reverse();
    found
}

fn read(path: &Path) -> Result<Libraries, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str::<Libraries>(&text).map_err(|error| error.to_string())?.validated()
}

/// What loading found: the libraries, and a message for the user if something had to be repaired.
pub struct Loaded {
    pub libraries: Libraries,
    pub notice: Option<String>,
}

/// Loads the libraries. With no file yet there are none. A damaged file is set aside and the
/// newest good backup used instead.
pub fn load() -> Loaded {
    let path = library_path();
    if !path.exists() {
        return Loaded { libraries: Libraries::default(), notice: None };
    }
    let problem = match read(&path) {
        Ok(libraries) => return Loaded { libraries, notice: None },
        Err(problem) => problem,
    };
    // Keep the damaged file, so nothing is lost even if no backup is good.
    let kept = path.with_file_name(format!("libraries.damaged-{}.json", now_millis()));
    let _ = std::fs::rename(&path, &kept);
    for backup in backups() {
        if let Ok(libraries) = read(&backup) {
            let _ = write_atomically(&path, &serde_json::to_string_pretty(&libraries).unwrap_or_default());
            let notice = format!("Your libraries file was damaged ({problem}), so the latest backup was restored. The damaged file was kept as {}.", kept.display());
            return Loaded { libraries, notice: Some(notice) };
        }
    }
    Loaded { libraries: Libraries::default(), notice: Some(format!("Your libraries file was damaged ({problem}) and no backup could be read. It was kept as {}.", kept.display())) }
}

/// Saves the libraries, backing up the previous file first when it is due (or `always_back_up`,
/// before a deletion). Returns a message if saving failed.
pub fn save(libraries: &Libraries, always_back_up: bool) -> Result<(), String> {
    let path = library_path();
    std::fs::create_dir_all(folder()).map_err(|error| error.to_string())?;
    if path.exists() {
        let latest = backups().first().and_then(|backup| backup.metadata().ok()?.modified().ok());
        let due = latest.is_none_or(|time| time.elapsed().is_ok_and(|age| age >= BACKUP_INTERVAL));
        if always_back_up || due {
            std::fs::create_dir_all(backup_folder()).map_err(|error| error.to_string())?;
            std::fs::copy(&path, backup_folder().join(format!("{}.json", now_millis()))).map_err(|error| format!("could not back up the libraries: {error}"))?;
            for old in backups().into_iter().skip(KEEP_BACKUPS) {
                let _ = std::fs::remove_file(old);
            }
        }
    }
    let text = serde_json::to_string_pretty(libraries).map_err(|error| error.to_string())?;
    write_atomically(&path, &text).map_err(|error| format!("could not save the libraries: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::model::Rgba;

    #[test]
    fn libraries_save_load_and_back_up_before_deletions() {
        let _ = std::fs::remove_dir_all(folder());
        let mut libraries = Libraries::default();
        let brand = libraries.add_brand("Brand A");
        libraries.add_color(brand, "Primary", Rgba::hex(0xff0000)).unwrap();
        save(&libraries, false).unwrap();
        assert_eq!(load().libraries, libraries);
        assert!(backups().is_empty(), "nothing to back up the first time");

        libraries.delete(brand);
        save(&libraries, true).unwrap();
        assert_eq!(backups().len(), 1, "the brand was backed up before it went");
        assert_eq!(read(&backups()[0]).unwrap().brands.len(), 1);
        let _ = std::fs::remove_dir_all(folder());
    }

    #[test]
    fn a_damaged_file_is_kept_and_the_latest_backup_restored() {
        let _ = std::fs::remove_dir_all(folder());
        let mut libraries = Libraries::default();
        libraries.add_brand("Brand A");
        save(&libraries, false).unwrap();
        save(&libraries, true).unwrap();
        std::fs::write(library_path(), "{ not json").unwrap();
        let loaded = load();
        assert_eq!(loaded.libraries, libraries, "the backup came back");
        assert!(loaded.notice.is_some_and(|notice| notice.contains("damaged")));
        let kept = std::fs::read_dir(folder()).unwrap().filter_map(Result::ok).any(|entry| entry.file_name().to_string_lossy().starts_with("libraries.damaged-"));
        assert!(kept, "the damaged file was set aside, not deleted");
        let _ = std::fs::remove_dir_all(folder());
    }
}
