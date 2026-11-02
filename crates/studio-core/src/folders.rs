//! Folders in the Project panel: adding them, the folders new projects start with, and sorting
//! imported files into the folder for their kind.

use crate::model::{Id, LabelColor, Project, ProjectItem, ProjectItemKind};

/// Adds a folder named `name` inside `parent` (`None` for the top level). Returns its id.
pub fn add_folder(project: &mut Project, name: &str, parent: Option<Id>) -> Id {
    let id = project.allocate_id();
    project.items.push(ProjectItem {
        id,
        name: name.to_owned(),
        kind: ProjectItemKind::Folder,
        label: LabelColor::None,
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
/// by that name yet. A blank name leaves the item where it is.
pub fn file_into(project: &mut Project, item: Id, folder: &str) {
    let folder = folder.trim();
    if folder.is_empty() {
        return;
    }
    let folder = top_folder(project, folder).unwrap_or_else(|| add_folder(project, folder, None));
    if let Some(item) = project.items.iter_mut().find(|candidate| candidate.id == item) {
        item.parent = Some(folder);
    }
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
/// itself or one of its own folders. Returns whether it moved.
pub fn move_to(project: &mut Project, item: Id, parent: Option<Id>) -> bool {
    if let Some(folder) = parent {
        let is_folder = project.item(folder).is_some_and(|folder| folder.kind == ProjectItemKind::Folder);
        if !is_folder || is_within(project, folder, item) {
            return false;
        }
    }
    match project.items.iter_mut().find(|candidate| candidate.id == item) {
        Some(found) if found.parent != parent => {
            found.parent = parent;
            true
        }
        _ => false,
    }
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
}
