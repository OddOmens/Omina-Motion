//! File ▸ Package…, like After Effects' Collect Files: working out what a packaged copy of a
//! project holds. Every file the project links to (the files of media items, and the `.cube`
//! files of Apply LUT effects) gets a place inside the package folder that mirrors the Project
//! panel's folders, and the packaged project points at those copies by paths relative to its own
//! file. This module only plans; the app copies the files and writes the zip.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::model::{Id, LutSource, Project};

/// Where media at the top of the Project panel (in no folder) goes inside the package.
pub const ROOT_MEDIA_FOLDER: &str = "Media";
/// Where the `.cube` files of Apply LUT effects go inside the package.
pub const LUT_FOLDER: &str = "LUTs";
/// The note inside the package listing linked files that weren't found.
pub const MISSING_NOTE: &str = "Missing files.txt";

/// One file to copy into the package.
#[derive(Clone, Debug, PartialEq)]
pub struct PackagedFile {
    /// Where the file is now.
    pub source: PathBuf,
    /// Where its copy goes, relative to the package folder, with `/` between folders.
    pub destination: String,
    pub bytes: u64,
}

/// What a package will hold: the files to copy, and the linked files that weren't found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackagePlan {
    pub files: Vec<PackagedFile>,
    /// Each missing file once, in the order the project links them.
    pub missing: Vec<PathBuf>,
}

impl PackagePlan {
    /// How many bytes the copies add up to.
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

/// Works out the package for `project`. `project_folder` is the saved project's folder, which
/// any linked path still relative to it is read against. `size_of` gives a file's size, or
/// `None` when there is no file there (it is then listed as missing).
///
/// A file linked more than once (an item duplicated in the Project panel, or a LUT used on
/// several layers) is copied once, into the first place it is needed. Names that clash inside a
/// folder get " 2", " 3" and so on, ignoring case, since macOS folders do.
pub fn plan(project: &Project, project_folder: Option<&Path>, size_of: impl Fn(&Path) -> Option<u64>) -> PackagePlan {
    let mut plan = PackagePlan::default();
    let mut copied: HashSet<PathBuf> = HashSet::new();
    let mut taken: HashSet<String> = HashSet::new();
    for (linked, folder) in linked_files(project) {
        let source = resolve(linked, project_folder);
        if copied.contains(&source) || plan.missing.contains(&source) {
            continue;
        }
        let Some(bytes) = size_of(&source) else {
            plan.missing.push(source);
            continue;
        };
        let name = source.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let destination = free_destination(&folder, &safe_file_name(&name, "File"), &mut taken);
        copied.insert(source.clone());
        plan.files.push(PackagedFile { source, destination, bytes });
    }
    plan
}

/// The project as it is saved inside the package: every linked file that was copied points at
/// its copy, by a path relative to the packaged project file. Files that weren't copied (missing
/// ones) keep their full path, so the project still finds them if they turn up there again.
pub fn packaged_project(project: &Project, project_folder: Option<&Path>, copied: &[PackagedFile]) -> Project {
    let copies: HashMap<&Path, &str> = copied.iter().map(|file| (file.source.as_path(), file.destination.as_str())).collect();
    let mut packaged = project.clone();
    for_each_linked_path_mut(&mut packaged, |path| {
        let source = resolve(path, project_folder);
        *path = match copies.get(source.as_path()) {
            Some(destination) => PathBuf::from(destination),
            None => source,
        };
    });
    packaged
}

/// The text of the note listing the files a package is missing.
pub fn missing_note(project_name: &str, missing: &[PathBuf]) -> String {
    let mut note = format!("These files are linked from “{project_name}” but weren't found when it was packaged, so they aren't in this package:\n\n");
    for path in missing {
        note.push_str(&path.display().to_string());
        note.push('\n');
    }
    note
}

/// `name` made safe to use as a file or folder name: characters the Finder can't show in names
/// (`/`, `:`, `\` and control characters) become dashes, and leading dots (which would hide it)
/// and spaces at either end are dropped. A name left empty becomes `fallback`.
pub fn safe_file_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name.chars().map(|c| if matches!(c, '/' | ':' | '\\') || c.is_control() { '-' } else { c }).collect();
    // Dots and spaces are dropped in turn until none lead, so ". .." can't become "..".
    let cleaned = cleaned.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end();
    if cleaned.is_empty() { fallback.to_owned() } else { cleaned.to_owned() }
}

/// Every file `project` links to, with the package folder it belongs in: a media item's file
/// in the folder mirroring its Project panel folder, a LUT file in `LUTs`.
fn linked_files(project: &Project) -> Vec<(&Path, String)> {
    let items = project.items.iter().filter_map(|item| Some((item.path.as_deref()?, package_folder(project, item.parent))));
    let effects = project.compositions.iter().flat_map(|comp| &comp.layers).flat_map(|layer| &layer.effects);
    let luts = effects.filter_map(|effect| match &effect.lut {
        Some(LutSource::File(path)) => Some((path.as_path(), LUT_FOLDER.to_owned())),
        _ => None,
    });
    items.chain(luts).collect()
}

/// Calls `visit` on every linked path in `project`, in the same order as `linked_files`.
fn for_each_linked_path_mut(project: &mut Project, mut visit: impl FnMut(&mut PathBuf)) {
    project.items.iter_mut().filter_map(|item| item.path.as_mut()).for_each(&mut visit);
    let effects = project.compositions.iter_mut().flat_map(|comp| &mut comp.layers).flat_map(|layer| &mut layer.effects);
    for effect in effects {
        if let Some(LutSource::File(path)) = &mut effect.lut {
            visit(path);
        }
    }
}

/// The folder inside the package for media in Project panel folder `parent`: the names of the
/// folders from the top down, joined by `/`, or `Media` for media at the top level.
fn package_folder(project: &Project, parent: Option<Id>) -> String {
    let mut names = Vec::new();
    let mut current = parent;
    // Bounded, so a damaged project whose folders loop can't hang packaging.
    for _ in 0..=project.items.len() {
        let Some(folder) = current.and_then(|id| project.item(id)) else { break };
        names.push(safe_file_name(&folder.name, "Folder"));
        current = folder.parent;
    }
    if names.is_empty() {
        return ROOT_MEDIA_FOLDER.to_owned();
    }
    names.reverse();
    names.join("/")
}

/// A linked path as a full path: one relative to the project file is read against its folder.
fn resolve(path: &Path, project_folder: Option<&Path>) -> PathBuf {
    match project_folder {
        Some(folder) if path.is_relative() => folder.join(path),
        _ => path.to_path_buf(),
    }
}

/// `folder/name`, or `folder/name 2`, `folder/name 3` and so on (before the extension) when an
/// earlier file took it. Records the result, and the folders it is in, as taken.
fn free_destination(folder: &str, name: &str, taken: &mut HashSet<String>) -> String {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let destination = (1..)
        .map(|n| if n == 1 { format!("{folder}/{name}") } else { format!("{folder}/{stem} {n}{extension}") })
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("some numbered name is free");
    // The folders count as taken too, so no file is given a folder's name.
    let mut prefix = String::new();
    for part in folder.split('/') {
        prefix = if prefix.is_empty() { part.to_owned() } else { format!("{prefix}/{part}") };
        taken.insert(prefix.to_lowercase());
    }
    taken.insert(destination.to_lowercase());
    destination
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit;
    use crate::model::ProjectItem;
    use crate::sample::{ids, sample_project};

    /// Links sample item `id` to the file at `path`.
    fn link(project: &mut Project, id: Id, path: &str) {
        project.items.iter_mut().find(|item| item.id == id).unwrap().path = Some(path.into());
    }

    fn item_path(project: &Project, id: Id) -> PathBuf {
        project.item(id).and_then(|item: &ProjectItem| item.path.clone()).unwrap()
    }

    /// Every file exists, 10 bytes long, except those under `/gone`.
    fn sizes(path: &Path) -> Option<u64> {
        (!path.starts_with("/gone")).then_some(10)
    }

    #[test]
    fn copies_mirror_the_project_panel_folders() {
        let mut project = sample_project();
        let b_roll = crate::folders::add_folder(&mut project, "B-Roll", Some(ids::FOOTAGE_FOLDER));
        link(&mut project, ids::A_CAM, "/shoot/day1/A-cam_01.mov");
        link(&mut project, ids::B_ROLL, "/shoot/day2/B-roll_02.mov");
        crate::folders::move_to(&mut project, ids::B_ROLL, Some(b_roll));
        // Moved to the top level: it goes in Media.
        link(&mut project, ids::LOGO, "/brand/logo.png");
        crate::folders::move_to(&mut project, ids::LOGO, None);
        let plan = plan(&project, None, sizes);
        let destinations: Vec<_> = plan.files.iter().map(|file| file.destination.as_str()).collect();
        assert_eq!(destinations, ["Footage/A-cam_01.mov", "Footage/B-Roll/B-roll_02.mov", "Media/logo.png"]);
        assert_eq!(plan.total_bytes(), 30);
        assert!(plan.missing.is_empty());
    }

    #[test]
    fn clashing_names_are_numbered_and_shared_files_copied_once() {
        let mut project = sample_project();
        link(&mut project, ids::A_CAM, "/card1/clip.mov");
        link(&mut project, ids::B_ROLL, "/card2/Clip.MOV");
        link(&mut project, ids::LOGO, "/card3/clip.mov");
        // The same file as A-cam, linked again (a duplicated item).
        link(&mut project, ids::MUSIC, "/card1/clip.mov");
        let plan = plan(&project, None, sizes);
        let destinations: Vec<_> = plan.files.iter().map(|file| file.destination.as_str()).collect();
        assert_eq!(destinations, ["Footage/clip.mov", "Footage/Clip 2.MOV", "Footage/clip 3.mov"], "names differing only in case clash too");
        // Names without an extension, or starting with a dot, are numbered sensibly as well.
        let mut taken = HashSet::new();
        assert_eq!(free_destination("Media", "README", &mut taken), "Media/README");
        assert_eq!(free_destination("Media", "README", &mut taken), "Media/README 2");
        assert_eq!(free_destination("Media", ".env", &mut taken), "Media/.env");
        assert_eq!(free_destination("Media", ".env", &mut taken), "Media/.env 2");
        // A file can't take the name of a folder already in the package.
        assert_eq!(free_destination("Footage/B-Roll", "a.mov", &mut taken), "Footage/B-Roll/a.mov");
        assert_eq!(free_destination("Footage", "B-Roll", &mut taken), "Footage/B-Roll 2");
    }

    #[test]
    fn missing_files_are_listed_once_and_not_copied() {
        let mut project = sample_project();
        link(&mut project, ids::A_CAM, "/gone/a.mov");
        link(&mut project, ids::B_ROLL, "/gone/a.mov");
        link(&mut project, ids::LOGO, "/here/logo.png");
        let plan = plan(&project, None, sizes);
        assert_eq!(plan.missing, [PathBuf::from("/gone/a.mov")]);
        assert_eq!(plan.files.len(), 1);
        let note = missing_note("Promo", &plan.missing);
        assert!(note.contains("“Promo”") && note.contains("/gone/a.mov"), "{note}");
    }

    #[test]
    fn the_packaged_project_points_at_the_copies_relative_to_itself() {
        let mut project = sample_project();
        link(&mut project, ids::A_CAM, "/shoot/A-cam_01.mov");
        link(&mut project, ids::LOGO, "/gone/logo.png");
        // A path already relative to the saved project is read against its folder.
        link(&mut project, ids::MUSIC, "Audio/score.wav");
        let grade = edit::ensure_color_grade_layer(&mut project, ids::MAIN_COMP).unwrap();
        edit::add_lut(&mut project, ids::MAIN_COMP, grade, LutSource::File("/looks/Film.cube".into())).unwrap();
        let folder = Path::new("/projects/promo");
        let plan = plan(&project, Some(folder), sizes);
        let packaged = packaged_project(&project, Some(folder), &plan.files);

        assert_eq!(item_path(&packaged, ids::A_CAM), PathBuf::from("Footage/A-cam_01.mov"));
        assert_eq!(item_path(&packaged, ids::MUSIC), PathBuf::from("Footage/score.wav"));
        assert_eq!(item_path(&packaged, ids::LOGO), PathBuf::from("/gone/logo.png"), "a missing file keeps its full path");
        let luts: Vec<_> = packaged.compositions.iter().flat_map(|comp| &comp.layers).flat_map(|layer| &layer.effects).filter_map(|effect| effect.lut.clone()).collect();
        assert_eq!(luts, [LutSource::File("LUTs/Film.cube".into())]);
        assert_eq!(plan.files.iter().find(|file| file.destination == "Footage/score.wav").unwrap().source, PathBuf::from("/projects/promo/Audio/score.wav"));
        // The original project is untouched.
        assert_eq!(item_path(&project, ids::A_CAM), PathBuf::from("/shoot/A-cam_01.mov"));

        // Opened from wherever the package is unzipped, the copies are found there.
        let view = crate::project_file::EditorView { active_composition: ids::MAIN_COMP, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, 10.0) };
        let text = crate::project_file::to_text(&packaged, view).unwrap();
        let (mut opened, _) = crate::project_file::from_text(&text).unwrap();
        let unzipped = Path::new("/elsewhere/Promo");
        let copies: Vec<PathBuf> = plan.files.iter().map(|file| unzipped.join(&file.destination)).collect();
        let exists = |path: &Path| copies.iter().any(|copy| copy == path);
        let missing = crate::project_file::relink_missing_media(&mut opened, unzipped, exists);
        assert_eq!(missing, [PathBuf::from("/gone/logo.png")]);
        assert_eq!(item_path(&opened, ids::A_CAM), unzipped.join("Footage/A-cam_01.mov"));
    }

    #[test]
    fn folder_and_file_names_are_made_safe() {
        assert_eq!(safe_file_name("B-Roll/Day 1: Wide", "Folder"), "B-Roll-Day 1- Wide");
        assert_eq!(safe_file_name("  ..hidden ", "Folder"), "hidden");
        assert_eq!(safe_file_name(" / ", "Untitled"), "-");
        assert_eq!(safe_file_name("..", "Untitled"), "Untitled");
        assert_eq!(safe_file_name(". ..", "Folder"), "Folder", "never a name that climbs out of the package");
        let mut project = sample_project();
        let odd = crate::folders::add_folder(&mut project, "Day 1/2", Some(ids::FOOTAGE_FOLDER));
        link(&mut project, ids::A_CAM, "/shoot/a.mov");
        crate::folders::move_to(&mut project, ids::A_CAM, Some(odd));
        assert_eq!(plan(&project, None, sizes).files[0].destination, "Footage/Day 1-2/a.mov");
    }
}
