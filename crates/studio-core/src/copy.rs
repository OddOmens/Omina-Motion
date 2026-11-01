//! Copying layers: Duplicate (Cmd+D) and Paste (Cmd+V). Copies get fresh ids for themselves and
//! their effects, and each goes on a new track of its own, so nothing already in the timeline
//! moves in time or is covered differently.

use std::collections::HashMap;

use crate::model::{Composition, Id, Layer, LayerContent, MAX_FRAME, Project, TrackMatte};

/// The layers to copy from `composition`, in stacking order (top first), so a pasted block keeps
/// the order it had. A layer that uses a track matte brings its matte (the layer directly above)
/// along, as it would look wrong without it.
pub fn layers_to_copy(composition: &Composition, ids: &[Id]) -> Vec<Layer> {
    let layers = &composition.layers;
    let mut wanted: Vec<bool> = layers.iter().map(|layer| ids.contains(&layer.id)).collect();
    for index in (1..layers.len()).rev() {
        if wanted[index] && layers[index].track_matte != TrackMatte::None {
            wanted[index - 1] = true;
        }
    }
    layers.iter().zip(wanted).filter(|(_, wanted)| *wanted).map(|(layer, _)| layer.clone()).collect()
}

/// Duplicates `ids` in place: the copies sit directly above the topmost of them, at the same
/// times. Returns the copies' ids, top first.
pub fn duplicate_layers(project: &mut Project, comp_id: Id, ids: &[Id]) -> Vec<Id> {
    let Some(composition) = project.composition(comp_id) else { return Vec::new() };
    let originals = layers_to_copy(composition, ids);
    let Some(top) = composition.layers.iter().position(|layer| ids.contains(&layer.id)) else { return Vec::new() };
    // Above the whole matte group the top layer belongs to, so no matte pair is split apart.
    let mut at = top;
    while at > 0 && composition.layers[at].track_matte != TrackMatte::None {
        at -= 1;
    }
    insert_copies(project, comp_id, &originals, at, 0)
}

/// Pastes copies of `layers` (top first) at the top of the stack, moved in time so the earliest
/// starts at `frame`. Layers that cannot go here are left out: a composition inside itself, or
/// media no longer in the project (copied before another project was opened, say). Returns the
/// pasted layers' ids, top first.
pub fn paste_layers(project: &mut Project, comp_id: Id, layers: &[Layer], frame: i64) -> Vec<Id> {
    let fits = |layer: &&Layer| match layer.content {
        LayerContent::Composition { composition } => composition != comp_id && project.composition(composition).is_some(),
        LayerContent::Footage { item } | LayerContent::Image { item } | LayerContent::Audio { item } => project.item(item).is_some(),
        _ => true,
    };
    let layers: Vec<Layer> = layers.iter().filter(fits).cloned().collect();
    let Some(earliest) = layers.iter().map(|layer| layer.in_frame).min() else { return Vec::new() };
    let delta = frame.clamp(0, MAX_FRAME) - earliest;
    insert_copies(project, comp_id, &layers, 0, delta)
}

/// Inserts copies of `originals` (top first, as `layers_to_copy` gives them) as a block at stack
/// index `at`, each on a new track, `delta` frames later than the original. A parent that was
/// copied too becomes the parent's copy; one outside the block is kept if it is in this
/// composition. The block's top layer has nothing above it to use as a matte, so loses its own.
fn insert_copies(project: &mut Project, comp_id: Id, originals: &[Layer], at: usize, delta: i64) -> Vec<Id> {
    let mut new_ids: HashMap<Id, Id> = HashMap::new();
    let mut copies = Vec::with_capacity(originals.len());
    for original in originals {
        let mut copy = original.clone();
        copy.id = project.allocate_id();
        new_ids.insert(original.id, copy.id);
        for effect in &mut copy.effects {
            effect.id = project.allocate_id();
        }
        copy.shift_in_time(delta);
        copies.push(copy);
    }
    let Some(composition) = project.composition_mut(comp_id) else { return Vec::new() };
    for (index, copy) in copies.iter_mut().enumerate() {
        copy.parent = copy.parent.and_then(|parent| match new_ids.get(&parent) {
            Some(&copied) => Some(copied),
            None => composition.layer(parent).map(|_| parent),
        });
        if index == 0 {
            copy.track_matte = TrackMatte::None;
        }
    }
    // Bottom first, each at the same index, so the block ends up in its original order.
    for mut copy in copies.iter().rev().cloned() {
        copy.track = crate::tracks::make_room_at(composition, at, crate::tracks::is_sound_only(&copy));
        composition.layers.insert(at.min(composition.layers.len()), copy);
    }
    crate::tracks::normalize(composition);
    copies.iter().map(|copy| copy.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    fn comp(project: &Project) -> &Composition {
        project.composition(ids::MAIN_COMP).unwrap()
    }

    #[test]
    fn duplicates_sit_above_with_new_ids_at_the_same_time() {
        let mut project = sample_project();
        let title = comp(&project).layers[0].clone();
        let count = comp(&project).layers.len();
        let copies = duplicate_layers(&mut project, ids::MAIN_COMP, &[title.id]);
        assert_eq!(copies.len(), 1);
        let copy = comp(&project).layer(copies[0]).unwrap();
        assert_ne!(copy.id, title.id);
        assert_eq!((copy.in_frame, copy.out_frame), (title.in_frame, title.out_frame));
        assert_eq!(comp(&project).layers.len(), count + 1);
        assert_eq!(comp(&project).layers[0].id, copy.id, "the copy is on top");
        assert!(copy.track > comp(&project).layer(title.id).unwrap().track, "on its own track above");
    }

    #[test]
    fn paste_moves_the_block_to_the_frame() {
        let mut project = sample_project();
        let title = comp(&project).layers[0].clone();
        let copied = layers_to_copy(comp(&project), &[title.id]);
        let pasted = paste_layers(&mut project, ids::MAIN_COMP, &copied, 100);
        let first = comp(&project).layer(pasted[0]).unwrap();
        assert_eq!((first.in_frame, first.out_frame), (100, 100 + title.out_frame - title.in_frame));
    }

    #[test]
    fn copying_a_matted_layer_brings_its_matte_along() {
        let mut project = sample_project();
        // The second layer uses the title above it as an alpha matte.
        project.composition_mut(ids::MAIN_COMP).unwrap().layers[1].track_matte = TrackMatte::Alpha;
        let composition = comp(&project);
        let index = 1;
        let matted = composition.layers[index].clone();
        let copied = layers_to_copy(composition, &[matted.id]);
        assert_eq!(copied.len(), 2, "the matte above came too");
        let pasted = paste_layers(&mut project, ids::MAIN_COMP, &copied, 0);
        let layers = &comp(&project).layers;
        let at = layers.iter().position(|layer| layer.id == pasted[1]).unwrap();
        assert_eq!(layers[at].track_matte, matted.track_matte, "the copy keeps its matte");
        assert_eq!(layers[at - 1].id, pasted[0], "with the copied matte directly above it");
    }

    #[test]
    fn layers_whose_media_is_gone_are_not_pasted() {
        let mut project = sample_project();
        let mut layer = comp(&project).layers[0].clone();
        layer.content = LayerContent::Footage { item: Id(999_999) };
        assert!(paste_layers(&mut project, ids::MAIN_COMP, &[layer], 0).is_empty());
    }

    #[test]
    fn a_composition_is_never_pasted_into_itself() {
        let mut project = sample_project();
        let mut layer = comp(&project).layers[0].clone();
        layer.content = LayerContent::Composition { composition: ids::MAIN_COMP };
        assert!(paste_layers(&mut project, ids::MAIN_COMP, &[layer], 0).is_empty());
    }
}
