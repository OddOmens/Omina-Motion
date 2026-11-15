//! Sample photos for templates: a template's spots for photos and footage can start out filled
//! with pictures, so it looks finished the moment it is made. Each picture sits in its spot the
//! way a dropped one does (see `slots`), so dropping another photo or video on it replaces it.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::animation::Vec2;
use crate::edit;
use crate::model::{Id, Project};
use crate::slots::{self, Slot, SlotKind};

/// What a photo shows, to match it to the spot it suits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhotoKind {
    /// One person, for profile pictures, guests, customers and talking to camera.
    Portrait,
    /// Something for sale.
    Product,
    /// A place or a moment: travel, friends, a table.
    Scene,
    /// An abstract picture, for album covers.
    Art,
}

impl PhotoKind {
    /// The kind of photo that suits a spot, from what the spot asks for (the words between
    /// "Drop your" and "here" in its name). `None` for a spot no stock picture suits: a logo.
    fn for_spot(what: &str) -> Option<Self> {
        let asks_for = |words: &[&str]| words.iter().any(|word| what.contains(word));
        if asks_for(&["logo"]) {
            None
        } else if asks_for(&["profile", "guest", "customer", "talking", "before", "after", "model", "speaker"]) {
            Some(Self::Portrait)
        } else if asks_for(&["product"]) {
            Some(Self::Product)
        } else if asks_for(&["album"]) {
            Some(Self::Art)
        } else {
            Some(Self::Scene)
        }
    }
}

/// A picture file that can fill a template's photo spots.
#[derive(Clone, Debug, PartialEq)]
pub struct SamplePhoto {
    pub kind: PhotoKind,
    pub path: PathBuf,
    /// Its size in pixels.
    pub size: Vec2,
}

/// The project's picture for the file at `path`, imported if it isn't there yet, so a photo used
/// in several spots (or several templates) is one item in the Project panel.
fn item_for(project: &mut Project, photo: &SamplePhoto) -> Id {
    let existing = project.items.iter().find(|item| item.path.as_deref() == Some(photo.path.as_path())).map(|item| item.id);
    existing.unwrap_or_else(|| edit::import_image(project, photo.path.clone(), photo.size))
}

/// Fills every empty photo or footage spot of composition `comp` with one of `photos`: a
/// portrait where a person is wanted, a product where one is, and so on, taking the next
/// photo of that kind each time and starting over when they run out. Which photo comes first
/// follows from the composition's name, so templates don't all open on the same picture, while
/// one template gets the same pictures every time. A spot stays empty when there is no photo
/// of its kind, or when it wants something of the user's own (a logo). Returns how many spots
/// were filled. A before-and-after's after spot repeats the before spot's picture.
pub fn fill_photo_spots(project: &mut Project, comp: Id, photos: &[SamplePhoto]) -> usize {
    let Some(composition) = project.composition(comp) else { return 0 };
    let first = project.item(comp).map_or(0, |item| item.name.bytes().map(usize::from).sum::<usize>());
    // From the bottom layer up, so the pictures land in the order the spots were laid out.
    let spots: Vec<(Id, PhotoKind)> = composition
        .layers
        .iter()
        .rev()
        .filter(|layer| slots::is_placeholder(layer))
        .filter_map(|layer| Some((layer.id, PhotoKind::for_spot(layer.name.trim_start_matches(slots::PLACEHOLDER_PREFIX))?)))
        .collect();
    let mut used: HashMap<PhotoKind, usize> = HashMap::new();
    let mut filled = 0;
    // The photo the before spot got, which the after spot repeats.
    let mut before: Option<&SamplePhoto> = None;
    for (layer, kind) in spots {
        let what = project.composition(comp).and_then(|composition| composition.layer(layer)).map(|layer| layer.name.clone()).unwrap_or_default();
        let photo = match before.filter(|_| what.contains("after")) {
            // A before-and-after compares one subject, so both sides show the same picture.
            Some(same) => same,
            None => {
                let of_kind: Vec<&SamplePhoto> = photos.iter().filter(|photo| photo.kind == kind).collect();
                if of_kind.is_empty() {
                    continue;
                }
                // A before spot always takes the first photo of its kind, so the comparison opens
                // on the same face in every shape; other spots take turns.
                if what.contains("before") {
                    of_kind[0]
                } else {
                    let turn = used.entry(kind).or_insert(first);
                    let photo = of_kind[*turn % of_kind.len()];
                    *turn += 1;
                    photo
                }
            }
        };
        if what.contains("before") {
            before = Some(photo);
        }
        let item = item_for(project, photo);
        let slot = Slot { comp, layer, kind: SlotKind::Placeholder, root: layer, outline: [Vec2::new(0.0, 0.0); 4] };
        if slots::fill(project, &slot, item).is_some() {
            filled += 1;
        }
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LayerContent, ProjectItemKind};
    use crate::templates::{Format, Layout, TEMPLATES, build};

    fn photo(kind: PhotoKind, name: &str) -> SamplePhoto {
        SamplePhoto { kind, path: format!("/photos/{name}.jpg").into(), size: Vec2::new(1200.0, 1800.0) }
    }

    fn template(layout: Layout) -> &'static crate::templates::Template {
        TEMPLATES.iter().find(|template| template.layout == layout).expect("every layout has a template")
    }

    fn images(project: &Project) -> Vec<String> {
        project.items.iter().filter(|item| item.kind == ProjectItemKind::Image).map(|item| item.name.clone()).collect()
    }

    #[test]
    fn spots_ask_for_the_kind_of_photo_they_name() {
        assert_eq!(PhotoKind::for_spot("profile photo here"), Some(PhotoKind::Portrait));
        assert_eq!(PhotoKind::for_spot("talking-head video here"), Some(PhotoKind::Portrait));
        assert_eq!(PhotoKind::for_spot("product photo here"), Some(PhotoKind::Product));
        assert_eq!(PhotoKind::for_spot("album art here"), Some(PhotoKind::Art));
        assert_eq!(PhotoKind::for_spot("photo or video here"), Some(PhotoKind::Scene));
        assert_eq!(PhotoKind::for_spot("logo here"), None, "a logo is the user's own");
    }

    #[test]
    fn a_collage_gets_each_scene_in_turn_and_no_spot_is_left_empty() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template(Layout::Collage), Format::Vertical).unwrap();
        let photos = [photo(PhotoKind::Scene, "beach"), photo(PhotoKind::Scene, "city"), photo(PhotoKind::Portrait, "maya")];
        assert_eq!(fill_photo_spots(&mut project, comp, &photos), 4);
        // Four spots share two scenes; the portrait isn't wanted, so it isn't imported.
        let mut used = images(&project);
        used.sort();
        assert_eq!(used, vec!["beach.jpg", "city.jpg"]);
        let composition = project.composition(comp).unwrap();
        assert!(!composition.layers.iter().any(slots::is_placeholder));
        assert_eq!(composition.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Composition { .. })).count(), 4);
        // Filling again finds nothing left to fill.
        assert_eq!(fill_photo_spots(&mut project, comp, &photos), 0);
    }

    #[test]
    fn before_and_after_show_the_same_photo() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template(Layout::BeforeAfter), Format::Vertical).unwrap();
        let photos = [photo(PhotoKind::Portrait, "maya"), photo(PhotoKind::Portrait, "sam")];
        assert_eq!(fill_photo_spots(&mut project, comp, &photos), 2);
        assert_eq!(images(&project), vec!["maya.jpg"], "the first portrait, used on both sides");
    }

    #[test]
    fn a_spot_with_no_photo_of_its_kind_stays_empty() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let comp = build(&mut project, template(Layout::ProductCard), Format::Wide).unwrap();
        assert_eq!(fill_photo_spots(&mut project, comp, &[photo(PhotoKind::Portrait, "maya")]), 0);
        assert!(project.composition(comp).unwrap().layers.iter().any(slots::is_placeholder));
        assert!(images(&project).is_empty());
    }

    #[test]
    fn every_template_still_passes_the_file_checks_with_photos_in() {
        let photos = [photo(PhotoKind::Portrait, "maya"), photo(PhotoKind::Product, "shoe"), photo(PhotoKind::Scene, "beach"), photo(PhotoKind::Art, "orb")];
        for template in &TEMPLATES {
            let mut project = Project::new(Vec::new(), Vec::new());
            let comp = build(&mut project, template, Format::Vertical).unwrap();
            fill_photo_spots(&mut project, comp, &photos);
            let empty = project.composition(comp).unwrap().layers.iter().filter(|layer| slots::is_placeholder(layer)).count();
            assert_eq!(empty, usize::from(template.layout == Layout::BrandIntro), "{} has an empty spot (only a logo's stays)", template.name);
            let view = crate::project_file::EditorView { active_composition: comp, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, 300.0) };
            let text = crate::project_file::to_text(&project, view).unwrap();
            crate::project_file::from_text(&text).unwrap_or_else(|error| panic!("{}: {error}", template.name));
        }
    }
}
