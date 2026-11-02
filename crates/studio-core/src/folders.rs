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
}
