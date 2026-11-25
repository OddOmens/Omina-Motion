//! Auto-save, as in After Effects: while a saved project has unsaved changes, a copy is written
//! every few minutes into an "<project> Auto-Save" folder beside it, named with the time it was
//! made, and only the newest few are kept. The project file itself is never touched, so an
//! auto-save never saves over work the person meant to throw away. Unsaved new projects have no
//! folder to save beside; the recovery copy (see `app_settings::write_recovery`) covers those.

use std::path::{Path, PathBuf};

use studio_core::project_file::EXTENSION;

/// The folder a project's auto-saves go in: "Promo Auto-Save" beside "Promo.omim".
pub fn folder_for(project: &Path) -> PathBuf {
    let name = project_name(project);
    project.parent().unwrap_or(Path::new(".")).join(format!("{name} Auto-Save"))
}

/// The project's name: its file name without the extension.
fn project_name(project: &Path) -> String {
    project.file_stem().map_or_else(|| "Untitled".to_owned(), |stem| stem.to_string_lossy().into_owned())
}

/// Writes `text` as a new auto-save of `project`, stamped with `time` (local time, as
/// "2026-11-19 09.18.04", which also sorts oldest first), then deletes the oldest of the
/// project's auto-saves past `keep`. Returns where the copy went.
pub fn write(project: &Path, text: &str, keep: usize, time: chrono::DateTime<chrono::Local>) -> std::io::Result<PathBuf> {
    let folder = folder_for(project);
    std::fs::create_dir_all(&folder)?;
    let name = project_name(project);
    let path = folder.join(format!("{name} {}.{EXTENSION}", time.format("%Y-%m-%d %H.%M.%S")));
    crate::app_settings::write_atomically(&path, text)?;
    // Newest first: the first `keep` stay, the older ones go.
    for old in saves(project).into_iter().skip(keep.max(1)) {
        // One that can't be removed now is tried again after the next auto-save.
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// The project's auto-saves, newest first. Only files this module named are listed, so nothing
/// else someone keeps in the folder is ever deleted.
pub fn saves(project: &Path) -> Vec<PathBuf> {
    let prefix = format!("{} ", project_name(project));
    let Ok(entries) = std::fs::read_dir(folder_for(project)) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { return false };
            let Some(stamp) = name.strip_prefix(&prefix).and_then(|rest| rest.strip_suffix(&format!(".{EXTENSION}"))) else { return false };
            is_stamp(stamp)
        })
        .collect();
    found.sort();
    found.reverse();
    found
}

/// Whether `text` is a time stamp as `write` makes them: "2026-11-19 09.18.04".
fn is_stamp(text: &str) -> bool {
    let shape = "0000-00-00 00.00.00";
    text.len() == shape.len() && text.chars().zip(shape.chars()).all(|(have, want)| if want == '0' { have.is_ascii_digit() } else { have == want })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("motion-auto-save-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn copies_go_in_a_folder_beside_the_project_and_only_the_newest_are_kept() {
        let folder = scratch("keep");
        let project = folder.join("Promo.omim");
        std::fs::write(&project, "the saved project").unwrap();
        // Something else kept in the folder is never touched.
        std::fs::create_dir_all(folder_for(&project)).unwrap();
        std::fs::write(folder_for(&project).join("notes.txt"), "mine").unwrap();
        for minute in 0..5 {
            let time = chrono::Local.with_ymd_and_hms(2026, 11, 19, 9, minute, 4).unwrap();
            write(&project, &format!("copy {minute}"), 3, time).unwrap();
        }
        assert_eq!(folder_for(&project), folder.join("Promo Auto-Save"));
        let names: Vec<String> = saves(&project).iter().map(|path| path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Promo 2026-11-19 09.04.04.omim", "Promo 2026-11-19 09.03.04.omim", "Promo 2026-11-19 09.02.04.omim"]);
        assert_eq!(std::fs::read_to_string(&saves(&project)[0]).unwrap(), "copy 4");
        assert_eq!(std::fs::read_to_string(&project).unwrap(), "the saved project", "the project itself is left alone");
        assert!(folder_for(&project).join("notes.txt").exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn only_stamps_in_the_expected_shape_count() {
        assert!(is_stamp("2026-11-19 09.18.04"));
        assert!(!is_stamp("2026-11-19 09:18:04") && !is_stamp("draft") && !is_stamp("2026-11-19"));
    }
}
