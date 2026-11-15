//! Folders in the Project panel: adding them, the folders new projects start with, and sorting
//! imported files into the folder for their kind.

use crate::model::{Id, LabelColor, Project, ProjectItem, ProjectItemKind};

/// The label colors folders are given, in the order they are handed out, so each folder
/// (and with it the things kept in it) can be told apart at a glance.
const FOLDER_LABELS: [LabelColor; 11] = [
    LabelColor::Blue,
    LabelColor::Seafoam,
    LabelColor::Orange,
    LabelColor::Pink,
    LabelColor::Aqua,
    LabelColor::Yellow,
    LabelColor::Lavender,
    LabelColor::Peach,
    LabelColor::Green,
    LabelColor::Purple,
    LabelColor::Red,
];

/// The label color for a new folder: the first one no folder has yet, or once every color is
/// taken, the next one in turn.
fn next_folder_label(project: &Project) -> LabelColor {
    let folders = || project.items.iter().filter(|item| item.kind == ProjectItemKind::Folder);
    FOLDER_LABELS.into_iter().find(|label| folders().all(|folder| folder.label != *label)).unwrap_or(FOLDER_LABELS[folders().count() % FOLDER_LABELS.len()])
}

/// The label color of `folder`, which the items put in it take. The top level (`None`) has none.
pub fn folder_label(project: &Project, folder: Option<Id>) -> LabelColor {
    folder.and_then(|folder| project.item(folder)).map_or(LabelColor::None, |folder| folder.label)
}

/// Gives `item` the label color of the folder `to` it is going into (none, at the top level).
/// An item whose color was picked by hand keeps it, and so does a folder, which has a color
/// of its own.
fn take_folder_label(project: &mut Project, item: Id, to: Option<Id>) {
    let entering = folder_label(project, to);
    if let Some(item) = project.items.iter_mut().find(|candidate| candidate.id == item)
        && item.kind != ProjectItemKind::Folder
        && !item.label_picked
    {
        item.label = entering;
    }
}

/// Colors the folders of a project saved before folders had colors, when it is opened: each
/// folder gets a color of its own, and the items directly in it take that color. An item
/// keeps a color that looks picked by hand (one that is neither blank nor the color its kind
/// was imported with). Does nothing once any folder has a color, so a folder later set back
/// to no color stays that way. Returns whether anything changed.
pub fn color_older_project(project: &mut Project) -> bool {
    let folders: Vec<Id> = project.items.iter().filter(|item| item.kind == ProjectItemKind::Folder).map(|item| item.id).collect();
    if folders.is_empty() || folders.iter().any(|folder| folder_label(project, Some(*folder)) != LabelColor::None) {
        return false;
    }
    // The label each kind of item is given on import.
    let imported_with = |kind: ProjectItemKind| match kind {
        ProjectItemKind::Image => LabelColor::Peach,
        ProjectItemKind::Footage => LabelColor::Aqua,
        ProjectItemKind::Audio => LabelColor::Green,
        ProjectItemKind::Folder | ProjectItemKind::Composition | ProjectItemKind::Solid => LabelColor::None,
    };
    for folder in folders {
        let label = next_folder_label(project);
        for item in project.items.iter_mut() {
            let inside = item.parent == Some(folder) && item.kind != ProjectItemKind::Folder;
            let untouched = !item.label_picked && (item.label == LabelColor::None || item.label == imported_with(item.kind));
            if item.id == folder || (inside && untouched) {
                item.label = label;
            }
        }
    }
    true
}

/// Adds a folder named `name` inside `parent` (`None` for the top level), with a label color
/// no other folder has. Returns its id.
pub fn add_folder(project: &mut Project, name: &str, parent: Option<Id>) -> Id {
    let id = project.allocate_id();
    let label = next_folder_label(project);
    project.items.push(ProjectItem {
        id,
        name: name.to_owned(),
        kind: ProjectItemKind::Folder,
        label,
        label_picked: false,
        parent,
        size: None,
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: None,
    });
    id
}

/// The top-level folder named `name` (ignoring case), if there is one.
pub fn top_folder(project: &Project, name: &str) -> Option<Id> {
    project.items.iter().find(|item| item.kind == ProjectItemKind::Folder && item.parent.is_none() && item.name.eq_ignore_ascii_case(name)).map(|item| item.id)
}

/// The folder renders made inside Studio are imported into (Quick Export). Every project has one.
pub const EXPORTS_FOLDER: &str = "Exports";

/// Adds each of `names`, and the Exports folder, as a top-level folder, unless one by that name
/// is already there. Blank names are skipped.
pub fn add_default_folders(project: &mut Project, names: &[String]) {
    let names = names.iter().map(String::as_str).chain(std::iter::once(EXPORTS_FOLDER));
    for name in names.map(str::trim).filter(|name| !name.is_empty()) {
        if top_folder(project, name).is_none() {
            add_folder(project, name, None);
        }
    }
}

/// Puts `item` in the top-level folder named `folder`, making the folder if the project has none
/// by that name yet. A blank name leaves the item where it is. The item takes the folder's
/// label color unless its own was picked by hand.
pub fn file_into(project: &mut Project, item: Id, folder: &str) {
    let folder = folder.trim();
    if folder.is_empty() {
        return;
    }
    let folder = top_folder(project, folder).unwrap_or_else(|| add_folder(project, folder, None));
    if let Some(item) = project.items.iter_mut().find(|candidate| candidate.id == item) {
        item.parent = Some(folder);
    }
    take_folder_label(project, item, Some(folder));
}

/// Whether `item` is `folder` or somewhere inside it.
fn is_within(project: &Project, item: Id, folder: Id) -> bool {
    let mut current = Some(item);
    while let Some(id) = current {
        if id == folder {
            return true;
        }
        current = project.item(id).and_then(|item| item.parent);
    }
    false
}

/// Moves `item` into folder `parent`, or to the top level with `None`. A folder can't go inside
/// itself or one of its own folders. The item takes its new folder's label color unless its
/// own was picked by hand. Returns whether it moved.
pub fn move_to(project: &mut Project, item: Id, parent: Option<Id>) -> bool {
    if let Some(folder) = parent {
        let is_folder = project.item(folder).is_some_and(|folder| folder.kind == ProjectItemKind::Folder);
        if !is_folder || is_within(project, folder, item) {
            return false;
        }
    }
    match project.items.iter_mut().find(|candidate| candidate.id == item) {
        Some(found) if found.parent != parent => found.parent = parent,
        _ => return false,
    }
    take_folder_label(project, item, parent);
    true
}

/// Puts folder `folder` just before (or with `after`, just after) folder `beside`, in the same
/// folder as it. Folders keep the order they are put in; files are always listed by name.
/// Returns whether anything changed.
pub fn place_folder(project: &mut Project, folder: Id, beside: Id, after: bool) -> bool {
    let is_folder = |id: Id| project.item(id).is_some_and(|item| item.kind == ProjectItemKind::Folder);
    if folder == beside || !is_folder(folder) || !is_folder(beside) {
        return false;
    }
    let parent = project.item(beside).and_then(|item| item.parent);
    if parent.is_some_and(|parent| is_within(project, parent, folder)) {
        return false;
    }
    let Some(from) = project.items.iter().position(|item| item.id == folder) else { return false };
    let mut moved = project.items.remove(from);
    moved.parent = parent;
    let at = project.items.iter().position(|item| item.id == beside).map_or(project.items.len(), |index| index + usize::from(after));
    project.items.insert(at, moved);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;

    #[test]
    fn default_folders_are_added_once_and_imports_go_into_theirs() {
        let mut project = crate::sample::sample_project();
        let names = ["Footage", "Music", " ", "music"].map(String::from);
        add_default_folders(&mut project, &names);
        let folders = |project: &Project| project.items.iter().filter(|item| item.kind == ProjectItemKind::Folder && item.parent.is_none()).count();
        let before = folders(&project);
        add_default_folders(&mut project, &names);
        assert_eq!(folders(&project), before, "nothing doubles up");
        assert!(top_folder(&project, "MUSIC").is_some());
        assert!(top_folder(&project, EXPORTS_FOLDER).is_some(), "every project gets an Exports folder");
        // An image filed into "Images", which the project doesn't have yet: it is made.
        let image = crate::edit::import_image(&mut project, "/media/still.png".into(), Vec2::new(10.0, 10.0));
        file_into(&mut project, image, "Images");
        let images = top_folder(&project, "Images").expect("made");
        assert_eq!(project.item(image).unwrap().parent, Some(images));
    }

    #[test]
    fn items_move_between_folders_but_never_into_themselves() {
        let mut project = crate::sample::sample_project();
        let outer = add_folder(&mut project, "Outer", None);
        let inner = add_folder(&mut project, "Inner", None);
        let image = crate::edit::import_image(&mut project, "/media/still.png".into(), Vec2::new(10.0, 10.0));
        assert!(move_to(&mut project, image, Some(inner)));
        assert!(move_to(&mut project, inner, Some(outer)), "folders go in folders");
        assert!(!move_to(&mut project, outer, Some(inner)), "but not inside their own folder");
        assert!(!move_to(&mut project, outer, Some(outer)));
        assert!(!move_to(&mut project, image, Some(image)), "only folders hold things");
        assert!(move_to(&mut project, image, None), "back to the top");
        assert_eq!(project.item(image).unwrap().parent, None);
    }

    #[test]
    fn folders_are_put_where_they_are_dropped() {
        let mut project = crate::sample::sample_project();
        project.items.retain(|item| item.kind != ProjectItemKind::Folder || item.parent.is_some());
        let (a, b, c) = (add_folder(&mut project, "A", None), add_folder(&mut project, "B", None), add_folder(&mut project, "C", None));
        let order = |project: &Project| project.items.iter().filter(|item| item.kind == ProjectItemKind::Folder && item.parent.is_none()).map(|item| item.name.clone()).collect::<Vec<_>>();
        assert!(place_folder(&mut project, c, a, false));
        assert_eq!(order(&project), ["C", "A", "B"]);
        assert!(place_folder(&mut project, c, b, true));
        assert_eq!(order(&project), ["A", "B", "C"]);
        // Beside a folder inside another, it moves in there too.
        let inside = add_folder(&mut project, "Inside", Some(a));
        assert!(place_folder(&mut project, b, inside, false));
        assert_eq!(project.item(b).unwrap().parent, Some(a));
        assert!(!place_folder(&mut project, a, inside, true), "a folder can't go inside itself");
    }

    #[test]
    fn folders_get_colors_of_their_own_and_items_take_the_color_of_the_folder_they_are_in() {
        let mut project = Project::new(Vec::new(), Vec::new());
        add_default_folders(&mut project, &["Footage".to_owned(), "Music".to_owned()]);
        let (footage, music) = (top_folder(&project, "Footage").unwrap(), top_folder(&project, "Music").unwrap());
        let label = |project: &Project, id: Id| project.item(id).unwrap().label;
        let mut labels: Vec<LabelColor> = [footage, music, top_folder(&project, EXPORTS_FOLDER).unwrap()].map(|folder| label(&project, folder)).to_vec();
        assert!(!labels.contains(&LabelColor::None), "every folder has a color");
        labels.dedup();
        assert_eq!(labels.len(), 3, "each a different one");

        // A picture filed into Footage wears Footage's color, and Music's once moved there.
        let picture = crate::edit::import_image(&mut project, "/media/still.png".into(), Vec2::new(64.0, 36.0));
        file_into(&mut project, picture, "Footage");
        assert_eq!(label(&project, picture), label(&project, footage));
        assert!(move_to(&mut project, picture, Some(music)));
        assert_eq!(label(&project, picture), label(&project, music));

        // A color picked by hand stays through a move.
        crate::edit::set_item_label(&mut project, picture, LabelColor::Red);
        assert!(move_to(&mut project, picture, Some(footage)));
        assert_eq!(label(&project, picture), LabelColor::Red);

        // A folder moved into another keeps its own color.
        let music_color = label(&project, music);
        assert!(move_to(&mut project, music, Some(footage)));
        assert_eq!(label(&project, music), music_color);
    }

    #[test]
    fn recoloring_a_folder_recolors_the_items_that_wore_its_color() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let folder = add_folder(&mut project, "Footage", None);
        let follows = crate::edit::import_image(&mut project, "/media/a.png".into(), Vec2::new(64.0, 36.0));
        let picked = crate::edit::import_image(&mut project, "/media/b.png".into(), Vec2::new(64.0, 36.0));
        move_to(&mut project, follows, Some(folder));
        move_to(&mut project, picked, Some(folder));
        crate::edit::set_item_label(&mut project, picked, LabelColor::Red);
        crate::edit::set_item_label(&mut project, folder, LabelColor::Green);
        assert_eq!(project.item(follows).unwrap().label, LabelColor::Green);
        assert_eq!(project.item(picked).unwrap().label, LabelColor::Red, "a color picked by hand stays");
    }

    #[test]
    fn new_compositions_go_into_the_exports_folder_in_its_color() {
        let mut project = Project::new(Vec::new(), Vec::new());
        add_default_folders(&mut project, &[]);
        let exports = top_folder(&project, EXPORTS_FOLDER).unwrap();
        let settings = crate::edit::CompositionSettings {
            name: "Comp".into(),
            width: 1920,
            height: 1080,
            frame_rate: crate::time::FrameRate::new(30, 1),
            duration_frames: 300,
            background: crate::model::Rgba::hex(0x000000),
        };
        let comp = crate::edit::new_composition(&mut project, &settings).unwrap();
        let item = project.item(comp).unwrap();
        assert_eq!((item.parent, item.label), (Some(exports), project.item(exports).unwrap().label));
    }

    #[test]
    fn an_older_project_gets_folder_colors_once_and_keeps_colors_picked_by_hand() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let footage = add_folder(&mut project, "Footage", None);
        let music = add_folder(&mut project, "Music", None);
        let plain = crate::edit::import_image(&mut project, "/media/a.png".into(), Vec2::new(64.0, 36.0));
        let picked = crate::edit::import_image(&mut project, "/media/b.png".into(), Vec2::new(64.0, 36.0));
        // As a file saved before folders had colors: no folder color, items where they were
        // put with the colors they had (one still as imported, one changed by hand).
        for item in project.items.iter_mut() {
            match item.id {
                id if id == footage || id == music => item.label = LabelColor::None,
                id if id == plain => item.parent = Some(footage),
                id if id == picked => (item.parent, item.label) = (Some(footage), LabelColor::Purple),
                _ => {}
            }
        }
        assert!(color_older_project(&mut project));
        let label = |project: &Project, id: Id| project.item(id).unwrap().label;
        assert_ne!(label(&project, footage), LabelColor::None);
        assert_ne!(label(&project, footage), label(&project, music), "each folder its own color");
        assert_eq!(label(&project, plain), label(&project, footage), "an item as imported takes its folder's color");
        assert_eq!(label(&project, picked), LabelColor::Purple, "a changed one keeps its color");

        // Once colored it is left alone, so a folder set back to no color stays that way.
        crate::edit::set_item_label(&mut project, music, LabelColor::None);
        assert!(!color_older_project(&mut project));
        assert_eq!(label(&project, music), LabelColor::None);
    }
}
