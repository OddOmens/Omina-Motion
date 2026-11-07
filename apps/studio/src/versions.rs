//! Earlier versions of a project: copies kept every few minutes while it changes, and each time
//! it is saved (Settings ▸ General ▸ Versions), so work can be rolled back after a change that
//! went wrong, well beyond what undo remembers. File ▸ Browse Versions… lists and restores them.
//!
//! Each project's versions are `.studio` files named by the time they were made, in a folder of
//! their own under Studio's settings folder (`Versions/<name> <id>`), so they survive the
//! project being moved or renamed and never clutter the folder it is saved in. Only the newest
//! ones are kept.

use std::path::{Path, PathBuf};

/// One kept version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub path: PathBuf,
    /// When it was kept, in seconds since 1970.
    pub kept_at: u64,
    pub bytes: u64,
}

/// Where a project's versions are kept: one folder per project file (or one for untitled work),
/// named after the project so it can be found by hand too.
pub fn folder_for(project: Option<&Path>) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let root = crate::app_settings::folder().join("Versions");
    let Some(project) = project else { return root.join("Untitled") };
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    project.hash(&mut hasher);
    let name: String = project.file_stem().map_or_else(|| "Project".into(), |stem| stem.to_string_lossy().into_owned()).chars().filter(|c| !matches!(c, '/' | ':' | '\\') && !c.is_control()).take(60).collect();
    root.join(format!("{name} {:08x}", hasher.finish() as u32))
}

/// A project's versions, newest first.
pub fn list(project: Option<&Path>) -> Vec<Version> {
    let Ok(entries) = std::fs::read_dir(folder_for(project)) else { return Vec::new() };
    let mut versions: Vec<Version> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let kept_at = path.extension().filter(|extension| *extension == "studio").and(path.file_stem()).and_then(|stem| stem.to_str()).and_then(|stem| stem.parse().ok())?;
            Some(Version { bytes: entry.metadata().map_or(0, |metadata| metadata.len()), path, kept_at })
        })
        .collect();
    versions.sort_by_key(|version| std::cmp::Reverse(version.kept_at));
    versions
}

/// Keeps `text` as a new version of `project`, unless it is the same as the newest one, then
/// removes all but the newest `how_many`. Returns whether a version was written.
pub fn keep(text: &str, project: Option<&Path>, how_many: usize) -> std::io::Result<bool> {
    let existing = list(project);
    if existing.first().and_then(|newest| std::fs::read_to_string(&newest.path).ok()).is_some_and(|newest| newest == text) {
        return Ok(false);
    }
    let folder = folder_for(project);
    std::fs::create_dir_all(&folder)?;
    // A second version within the same second goes just after the one before it.
    let mut kept_at = crate::app_settings::now_seconds();
    if let Some(newest) = existing.first() {
        kept_at = kept_at.max(newest.kept_at + 1);
    }
    crate::app_settings::write_atomically(&folder.join(format!("{kept_at}.studio")), text)?;
    for old in list(project).into_iter().skip(how_many.max(1)) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_kept_newest_first_without_repeats_and_pruned() {
        let project = std::env::temp_dir().join(format!("versions-test-{}.studio", std::process::id()));
        let _ = std::fs::remove_dir_all(folder_for(Some(&project)));
        assert!(keep("one", Some(&project), 2).unwrap());
        assert!(!keep("one", Some(&project), 2).unwrap(), "the same again isn't kept twice");
        assert!(keep("two", Some(&project), 2).unwrap());
        assert!(keep("three", Some(&project), 2).unwrap());
        let versions = list(Some(&project));
        assert_eq!(versions.len(), 2, "only the newest two");
        assert_eq!(std::fs::read_to_string(&versions[0].path).unwrap(), "three");
        assert_eq!(std::fs::read_to_string(&versions[1].path).unwrap(), "two");
        assert!(versions[0].kept_at > versions[1].kept_at);
        // Other projects keep their own.
        assert!(list(Some(Path::new("/elsewhere/Other.studio"))).is_empty());
        std::fs::remove_dir_all(folder_for(Some(&project))).unwrap();
    }
}
