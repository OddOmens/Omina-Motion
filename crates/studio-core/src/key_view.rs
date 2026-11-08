//! Ways of looking at what a layer's keying keeps and removes, for the viewer. A layer is
//! "keyed" when it has Remove Background or a keying effect (a green or blue screen key). The
//! viewer can show its result as usual, or one of these views, built here as a copy of the
//! project that the ordinary renderer draws. Nothing here is written to the real project.

use crate::model::{Composition, Id, Layer, Project};

/// How the viewer shows a keyed layer instead of its plain result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyView {
    /// What is kept in full color with a white line around its edge, and what is removed still
    /// visible, but gray and dark, so both the selection and its surroundings can be seen.
    Selection,
    /// The layer alone in black and white: white is kept, black is removed, gray is partly
    /// see-through. Shows holes and rough edges a busy picture hides.
    Matte,
}

impl KeyView {
    pub const ALL: [KeyView; 2] = [KeyView::Selection, KeyView::Matte];

    pub fn name(self) -> &'static str {
        match self {
            KeyView::Selection => "Selection",
            KeyView::Matte => "Matte",
        }
    }
}

/// Keying effects that make parts of the picture see-through. (Spill Suppressor only changes
/// colors, so on its own it removes nothing.)
const REMOVING_EFFECTS: [&str; 3] = ["Linear Color Key", "Keyer", "Key Cleaner"];

/// Whether `name` is a keying effect, whose work the selection views take away to show what it removed.
fn is_keying_effect(name: &str) -> bool {
    crate::effects::category_of(name).is_some_and(|category| category.name == "Keying")
}

/// Whether the layer removes part of its picture: it has Remove Background, or a keying effect
/// that is switched on.
pub fn is_keyed(layer: &Layer) -> bool {
    let effects_on = layer.switches.effects_enabled;
    layer.cutout.is_some() || (effects_on && layer.effects.iter().any(|effect| effect.enabled && REMOVING_EFFECTS.contains(&effect.name.as_str())))
}

/// How dark (in stops) the removed parts are drawn in the Selection view: dim enough to read
/// as "not kept", light enough to still see what is there.
const REMOVED_EXPOSURE: f32 = -2.5;

/// The project as the viewer draws `view` of `layer_id` in composition `comp_id`. The layer
/// gets a copy of itself just underneath, with no keying, so the removed parts can be drawn
/// too. `None` when the layer isn't there or isn't keyed.
pub fn preview(project: &Project, comp_id: Id, layer_id: Id, view: KeyView) -> Option<Project> {
    let original = project.composition(comp_id)?.layer(layer_id)?;
    if !is_keyed(original) {
        return None;
    }
    let mut project = project.clone();
    let mut keyed = original.clone();
    // Effects switched off for the layer stay off, but the view's own effects must still draw.
    if !keyed.switches.effects_enabled {
        keyed.effects.iter_mut().for_each(|effect| effect.enabled = false);
        keyed.switches.effects_enabled = true;
    }
    let mut whole = keyed.clone();
    whole.id = project.allocate_id();
    whole.cutout = None;
    // The copy sits under the layer, where its matte (the layer above) would now be the layer
    // itself, so it uses none.
    whole.track_matte = crate::model::TrackMatte::None;
    for effect in &mut whole.effects {
        effect.id = project.allocate_id();
        if is_keying_effect(&effect.name) {
            effect.enabled = false;
        }
    }
    let line = edge_line_width(project.composition(comp_id)?);
    match view {
        KeyView::Selection => {
            add(&mut project, &mut whole, "Black & White", &[(0, 100.0), (1, 0.0)]);
            add(&mut project, &mut whole, "Exposure", &[(0, REMOVED_EXPOSURE)]);
            add(&mut project, &mut keyed, "Stroke", &[(0, line), (1, 100.0)]);
        }
        KeyView::Matte => {
            // Black wherever the picture is, white where the key keeps it.
            add(&mut project, &mut whole, "Exposure", &[(0, -30.0)]);
            add(&mut project, &mut keyed, "Fill", &[(0, 100.0)]);
            keyed.switches.solo = true;
            whole.switches.solo = true;
        }
    }
    let composition = project.composition_mut(comp_id)?;
    let index = composition.layers.iter().position(|layer| layer.id == layer_id)?;
    if view == KeyView::Matte {
        // Only the layer shows: any other solo is switched off for the view.
        composition.layers.iter_mut().for_each(|layer| layer.switches.solo = false);
    }
    composition.layers[index] = keyed;
    composition.layers.insert(index + 1, whole);
    Some(project)
}

/// The width of the line drawn around what is kept, in composition pixels: about 3 pixels on a
/// 1080p frame, and never under 1.5, so it shows at any size.
fn edge_line_width(composition: &Composition) -> f32 {
    (composition.width.max(composition.height) as f32 / 640.0).max(1.5)
}

/// Appends the catalog effect `name` to `layer`, with the parameters listed as (index, value)
/// and the rest at their defaults.
fn add(project: &mut Project, layer: &mut Layer, name: &str, values: &[(usize, f32)]) {
    let Some(definition) = crate::effects::find(name) else { return };
    let mut effect = crate::effects::instantiate(definition, project.allocate_id());
    for &(index, value) in values {
        if let Some(property) = effect.properties.get_mut(index) {
            property.value = crate::animation::Animated::constant(value);
        }
    }
    let id = effect.id;
    layer.effects.push(effect);
    crate::effect_graph::effect_added(layer, id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::model::{LayerContent, MatteStroke};
    use crate::sample::{self, ids};

    /// A project with a video layer on top of the main composition, and that layer's id.
    fn with_footage() -> (Project, Id) {
        let mut project = sample::sample_project();
        let info = crate::edit::FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: false };
        let item = crate::edit::import_footage(&mut project, "/media/screen.mp4".into(), &info);
        let layer = crate::edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
        (project, layer)
    }

    fn effect_names(layer: &Layer) -> Vec<(&str, bool)> {
        layer.effects.iter().map(|effect| (effect.name.as_str(), effect.enabled)).collect()
    }

    #[test]
    fn only_layers_that_remove_part_of_their_picture_are_keyed() {
        let (mut project, layer) = with_footage();
        let keyed = |project: &Project| is_keyed(project.composition(ids::MAIN_COMP).unwrap().layer(layer).unwrap());
        assert!(!keyed(&project));
        assert!(preview(&project, ids::MAIN_COMP, layer, KeyView::Selection).is_none(), "nothing to show");
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Spill Suppressor").unwrap();
        assert!(!keyed(&project), "spill suppression only changes colors");
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Keyer").unwrap();
        assert!(keyed(&project));
        let switches = &mut project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(layer).unwrap().switches;
        switches.effects_enabled = false;
        assert!(!keyed(&project), "a layer's effects switched off remove nothing");
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        crate::cutout::add_stroke(comp, layer, MatteStroke { frame: 0, points: vec![Vec2::new(0.5, 0.5)], radius: 0.02, add: true });
        assert!(keyed(&project), "Remove Background removes part of it even with effects off");
    }

    #[test]
    fn the_selection_view_draws_the_whole_picture_dimmed_under_the_kept_part_outlined() {
        let (mut project, layer) = with_footage();
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Keyer").unwrap();
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Spill Suppressor").unwrap();
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Gaussian Blur").unwrap();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        crate::cutout::add_stroke(comp, layer, MatteStroke { frame: 0, points: vec![Vec2::new(0.5, 0.5)], radius: 0.02, add: true });
        let count = comp.layers.len();
        let shown = preview(&project, ids::MAIN_COMP, layer, KeyView::Selection).unwrap();
        let comp = shown.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layers.len(), count + 1);
        let index = comp.layers.iter().position(|candidate| candidate.id == layer).unwrap();
        let (keyed, whole) = (&comp.layers[index], &comp.layers[index + 1]);
        assert!(keyed.cutout.is_some());
        assert_eq!(effect_names(keyed), [("Keyer", true), ("Spill Suppressor", true), ("Gaussian Blur", true), ("Stroke", true)]);
        assert_ne!(whole.id, layer);
        assert!(whole.cutout.is_none(), "the copy underneath shows the whole picture");
        assert_eq!(effect_names(whole), [("Keyer", false), ("Spill Suppressor", false), ("Gaussian Blur", true), ("Black & White", true), ("Exposure", true)]);
        // Every id in the copy is new, so the renderer keeps their caches apart.
        let mut ids: Vec<Id> = comp.layers.iter().map(|layer| layer.id).chain(comp.layers.iter().flat_map(|layer| layer.effects.iter().map(|effect| effect.id))).collect();
        let total = ids.len();
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        assert_eq!(ids.len(), total);
        // The real project is left alone.
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layers.len(), count);
    }

    #[test]
    fn the_matte_view_shows_the_layer_alone_in_black_and_white() {
        let (mut project, layer) = with_footage();
        crate::edit::add_effect(&mut project, ids::MAIN_COMP, layer, "Linear Color Key").unwrap();
        let other = project.composition(ids::MAIN_COMP).unwrap().layers.iter().find(|candidate| candidate.id != layer && !matches!(candidate.content, LayerContent::Audio { .. })).unwrap().id;
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(other).unwrap().switches.solo = true;
        let shown = preview(&project, ids::MAIN_COMP, layer, KeyView::Matte).unwrap();
        let comp = shown.composition(ids::MAIN_COMP).unwrap();
        let soloed: Vec<Id> = comp.layers.iter().filter(|candidate| candidate.switches.solo).map(|candidate| candidate.id).collect();
        let index = comp.layers.iter().position(|candidate| candidate.id == layer).unwrap();
        assert_eq!(soloed, [layer, comp.layers[index + 1].id], "only the layer and its copy show");
        assert_eq!(effect_names(&comp.layers[index]), [("Linear Color Key", true), ("Fill", true)]);
        assert_eq!(effect_names(&comp.layers[index + 1]), [("Linear Color Key", false), ("Exposure", true)]);
    }
}
