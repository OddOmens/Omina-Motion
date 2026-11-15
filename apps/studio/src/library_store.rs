//! Where libraries live on this computer: `libraries.json` in Studio's settings folder
//! (macOS: `~/Library/Application Support/Studio`), outside every project, so deleting or
//! moving a project never touches them.
//!
//! Writes go through a temporary file and a rename, so a crash can't leave half a file. Before
//! a change replaces the file, the old one is copied into `Library Backups` (at most one copy
//! every few minutes, and always before something is deleted); the newest `KEEP_BACKUPS` copies
//! are kept. If the file is ever damaged, the newest good backup is loaded instead and the
//! damaged file is set aside rather than overwritten.
//!
//! The file is shared with Canvas, Studio's image and vector editor, which reads and writes it
//! too. So Studio remembers what the file held when it last read or wrote it (`BASE`): a save
//! merges Studio's changes into the file as it is now (`merge`), keeping what Canvas changed
//! meanwhile, and `outside_changes` notices when Canvas has changed it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use studio_core::library::{Brand, Libraries};
use studio_core::model::LinkId;

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

thread_local! {
    /// The libraries as the file held them when Studio last read or wrote it, with the file's
    /// modified time then, and a merged copy a save made that the editor hasn't taken yet.
    /// (The editor reads and writes the file on its one UI thread; tests each have their own.)
    static BASE: RefCell<Base> = RefCell::new(Base::default());
}

#[derive(Default)]
struct Base {
    libraries: Option<Libraries>,
    modified: Option<SystemTime>,
    merged: Option<Libraries>,
}

fn modified() -> Option<SystemTime> {
    std::fs::metadata(library_path()).and_then(|metadata| metadata.modified()).ok()
}

/// Remembers `libraries` as what the file now holds.
fn remember(libraries: &Libraries) {
    BASE.with(|base| {
        let mut base = base.borrow_mut();
        base.libraries = Some(libraries.clone());
        base.modified = modified();
    });
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
        Ok(libraries) => {
            remember(&libraries);
            return Loaded { libraries, notice: None };
        }
        Err(problem) => problem,
    };
    // Keep the damaged file, so nothing is lost even if no backup is good.
    let kept = path.with_file_name(format!("libraries.damaged-{}.json", now_millis()));
    let _ = std::fs::rename(&path, &kept);
    for backup in backups() {
        if let Ok(libraries) = read(&backup) {
            let _ = write_atomically(&path, &serde_json::to_string_pretty(&libraries).unwrap_or_default());
            remember(&libraries);
            let notice = format!("Your libraries file was damaged ({problem}), so the latest backup was restored. The damaged file was kept as {}.", kept.display());
            return Loaded { libraries, notice: Some(notice) };
        }
    }
    Loaded { libraries: Libraries::default(), notice: Some(format!("Your libraries file was damaged ({problem}) and no backup could be read. It was kept as {}.", kept.display())) }
}

/// Saves the libraries, backing up the previous file first when it is due (or `always_back_up`,
/// before a deletion). Returns a message if saving failed.
///
/// If Canvas changed the file since Studio last read or wrote it, Studio's changes are merged
/// into it rather than written over it; the editor picks up the merged libraries through
/// `outside_changes`.
pub fn save(libraries: &Libraries, always_back_up: bool) -> Result<(), String> {
    let path = library_path();
    std::fs::create_dir_all(folder()).map_err(|error| error.to_string())?;
    let mut libraries = std::borrow::Cow::Borrowed(libraries);
    let base = BASE.with(|base| base.borrow().libraries.clone());
    if let Some(base) = base
        && path.exists()
        && let Ok(theirs) = read(&path)
        && theirs != base
    {
        let merged = merge(&base, &libraries, &theirs)?;
        BASE.with(|base| base.borrow_mut().merged = Some(merged.clone()));
        libraries = std::borrow::Cow::Owned(merged);
    }
    let libraries = libraries.as_ref();
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
    write_atomically(&path, &text).map_err(|error| format!("could not save the libraries: {error}"))?;
    remember(libraries);
    Ok(())
}

/// Libraries changed outside Studio (by Canvas) since Studio last read or wrote the file,
/// merged with Studio's own `current` ones (which may hold changes not saved yet), or `None`
/// when nothing changed. Also hands over the result of a save that had to merge.
pub fn outside_changes(current: &Libraries) -> Option<Libraries> {
    if let Some(merged) = BASE.with(|base| base.borrow_mut().merged.take()) {
        return Some(merged);
    }
    let (base, seen) = BASE.with(|base| {
        let base = base.borrow();
        (base.libraries.clone(), base.modified)
    });
    let now = modified();
    if now == seen {
        return None;
    }
    // A file that can't be read now (half written, say) is tried again next time.
    let theirs = read(&library_path()).ok()?;
    let merged = match base {
        Some(base) => merge(&base, current, &theirs).ok()?,
        None => theirs.clone(),
    };
    BASE.with(|cell| {
        let mut cell = cell.borrow_mut();
        cell.libraries = Some(theirs);
        cell.modified = now;
    });
    (merged != *current).then_some(merged)
}

/// Combines two sets of changes to the libraries made from the same `base`: Studio's (`mine`)
/// and Canvas's (`theirs`, what the file holds now). Item by item (brands, their colors and
/// text styles, and presets, matched by id): what only one side added is kept, what either
/// side deleted goes, and an item both kept takes the side that changed it (Studio's, if
/// both did).
pub fn merge(base: &Libraries, mine: &Libraries, theirs: &Libraries) -> Result<Libraries, String> {
    fn pick<T: Clone + PartialEq>(base: &T, mine: &T, theirs: &T) -> T {
        if mine != base { mine.clone() } else { theirs.clone() }
    }
    fn merge_brand(base: &Brand, mine: &Brand, theirs: &Brand) -> Brand {
        let mut brand = pick(base, mine, theirs);
        brand.name = pick(&base.name, &mine.name, &theirs.name);
        brand.colors = merge_items(&base.colors, &mine.colors, &theirs.colors, |item| item.id, pick);
        brand.text_styles = merge_items(&base.text_styles, &mine.text_styles, &theirs.text_styles, |item| item.id, pick);
        brand
    }
    let mut merged = mine.clone();
    merged.brands = merge_items(&base.brands, &mine.brands, &theirs.brands, |brand| brand.id, merge_brand);
    merged.presets = merge_items(&base.presets, &mine.presets, &theirs.presets, |preset| preset.id, pick);
    // Brings the next id past every id either side handed out.
    merged.validated()
}

/// Merges one list of items, matched by `id`, in `theirs`' order with Studio's new ones after.
fn merge_items<T: Clone>(base: &[T], mine: &[T], theirs: &[T], id: impl Fn(&T) -> LinkId, both: impl Fn(&T, &T, &T) -> T) -> Vec<T> {
    let find = |list: &[T], wanted: LinkId| list.iter().find(|item| id(item) == wanted).cloned();
    let mut merged = Vec::new();
    for item in theirs {
        let key = id(item);
        match (find(base, key), find(mine, key)) {
            // Deleted in Studio.
            (Some(_), None) => {}
            (Some(original), Some(own)) => merged.push(both(&original, &own, item)),
            // New in Canvas (or made on both sides under one id: Studio's wins).
            (None, own) => merged.push(own.unwrap_or_else(|| item.clone())),
        }
    }
    // New in Studio. (Kept in Studio but deleted in Canvas: gone.)
    merged.extend(mine.iter().filter(|item| find(base, id(item)).is_none() && find(theirs, id(item)).is_none()).cloned());
    merged
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
    fn studios_and_canvass_changes_to_the_libraries_merge() {
        let mut base = Libraries::default();
        let brand = base.add_brand("Brand A");
        let red = base.add_color(brand, "Red", Rgba::hex(0xff0000)).unwrap();
        let green = base.add_color(brand, "Green", Rgba::hex(0x00ff00)).unwrap();
        // Studio renames the brand, recolors red and adds a brand of its own.
        let mut mine = base.clone();
        mine.rename(brand, "Brand B");
        mine.set_color(red, Rgba::hex(0x880000));
        let studio_brand = mine.add_brand("Studio's");
        // Canvas (with ids far from Studio's) adds a color and deletes green.
        let mut theirs: serde_json::Value = serde_json::to_value(&base).unwrap();
        let colors = theirs["brands"][0]["colors"].as_array_mut().unwrap();
        colors.retain(|color| color["id"] != green.0);
        colors.push(serde_json::json!({ "id": 1_700_000_000_000u64, "name": "Blue", "color": { "r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0 } }));
        let theirs: Libraries = serde_json::from_value(theirs).unwrap();

        let merged = merge(&base, &mine, &theirs).unwrap();
        let first = merged.brand(brand).unwrap();
        assert_eq!(first.name, "Brand B");
        assert_eq!(first.colors.iter().map(|color| color.name.as_str()).collect::<Vec<_>>(), ["Red", "Blue"]);
        assert_eq!(merged.color(red).unwrap().1.color, Rgba::hex(0x880000));
        assert!(merged.brand(studio_brand).is_some());
        // Studio's next id moves past Canvas's, so the two never hand out the same one.
        let mut next = merged.clone();
        assert!(next.add_brand("Later").0 > 1_700_000_000_000);
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
