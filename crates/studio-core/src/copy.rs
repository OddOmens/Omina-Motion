//! Copying layers: Duplicate (Cmd+D), Paste (Cmd+V) and Paste Attributes (Alt+Cmd+V, which
//! copies chosen settings of one layer onto others without adding layers). Copies get fresh ids for themselves and
//! their effects, and each goes on a new track of its own, so nothing already in the timeline
//! moves in time or is covered differently.

use std::collections::HashMap;

use crate::model::{Composition, Id, Layer, LayerContent, LayerMask, MAX_FRAME, Project, TrackMatte};

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
        LayerContent::Composition { composition } => project.composition(composition).is_some() && !crate::edit::contains_composition(project, composition, comp_id),
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
/// composition. Masks and text paths follow the same rule. The block's top layer has nothing above it to use as a matte, so loses its own.
pub(crate) fn insert_copies(project: &mut Project, comp_id: Id, originals: &[Layer], at: usize, delta: i64) -> Vec<Id> {
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
        // The same for a mask: the copied mask if it came too, else the original if it is here.
        copy.mask = copy.mask.and_then(|mask| match new_ids.get(&mask.layer) {
            Some(&copied) => Some(LayerMask { layer: copied, ..mask }),
            None => composition.layer(mask.layer).map(|_| mask),
        });
        copy.text_path = copy.text_path.and_then(|on| match new_ids.get(&on.layer) {
            Some(&copied) => Some(crate::model::TextPath { layer: copied, ..on }),
            None => composition.layer(on.layer).map(|_| on),
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

/// Duplicates a Project panel item (Cmd+D there): a copy named "<name> copy" (or "copy 2", and
/// so on) right after it, in the same folder. A composition's copy is a new composition with the
/// same settings and copies of all its layers, with fresh ids, and parents, masks, text paths
/// and ducking pointed at the copies. Folders aren't duplicated. Returns the copy's id.
pub fn duplicate_item(project: &mut Project, id: Id) -> Option<Id> {
    let index = project.items.iter().position(|item| item.id == id)?;
    let original = project.items[index].clone();
    if original.kind == crate::model::ProjectItemKind::Folder {
        return None;
    }
    let copy_id = project.allocate_id();
    if original.kind == crate::model::ProjectItemKind::Composition {
        let composition = copy_composition(project, id, copy_id)?;
        project.compositions.push(composition);
    }
    let name = copy_name(project, &original.name);
    if let Some(composition) = project.composition_mut(copy_id) {
        composition.name = name.clone();
    }
    project.items.insert(index + 1, crate::model::ProjectItem { id: copy_id, name, ..original });
    Some(copy_id)
}

/// "<name> copy", or "<name> copy 2" and up when that name is taken.
fn copy_name(project: &Project, name: &str) -> String {
    let taken = |candidate: &str| project.items.iter().any(|item| item.name == candidate);
    let first = format!("{name} copy");
    if !taken(&first) {
        return first;
    }
    (2..).map(|number| format!("{name} copy {number}")).find(|candidate| !taken(candidate)).unwrap_or(first)
}

/// A copy of composition `id` with id `copy_id`, its layers and effects given fresh ids and the
/// layers' references to each other moved over to the copies.
fn copy_composition(project: &mut Project, id: Id, copy_id: Id) -> Option<Composition> {
    let mut composition = project.composition(id)?.clone();
    composition.id = copy_id;
    let mut new_ids: HashMap<Id, Id> = HashMap::new();
    for layer in &mut composition.layers {
        let fresh = project.allocate_id();
        new_ids.insert(layer.id, fresh);
        layer.id = fresh;
        for effect in &mut layer.effects {
            effect.id = project.allocate_id();
        }
    }
    let moved = |old: Id| new_ids.get(&old).copied();
    for layer in &mut composition.layers {
        layer.parent = layer.parent.and_then(moved);
        layer.mask = layer.mask.and_then(|mask| moved(mask.layer).map(|layer| LayerMask { layer, ..mask }));
        layer.text_path = layer.text_path.and_then(|on| moved(on.layer).map(|layer| crate::model::TextPath { layer, ..on }));
        layer.audio.duck_under = layer.audio.duck_under.and_then(moved);
    }
    Some(composition)
}

/// Which parts of a layer Paste Attributes copies, as in Premiere Pro.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attributes {
    /// Position, scale, rotation and anchor point.
    pub motion: bool,
    pub opacity: bool,
    /// The copied layer's effects, added after the target's own.
    pub effects: bool,
    /// Speed, ramps and playing backwards (footage only).
    pub speed: bool,
    pub blend_mode: bool,
    /// Volume and fades (layers with sound only).
    pub audio: bool,
}

impl Default for Attributes {
    fn default() -> Self {
        Self { motion: true, opacity: true, effects: true, speed: false, blend_mode: false, audio: false }
    }
}

/// A copy of `animated` with its keyframes `delta` frames later.
fn shifted<T: crate::animation::Blend>(delta: i64, animated: &crate::animation::Animated<T>) -> crate::animation::Animated<T> {
    let mut copy = animated.clone();
    copy.shift(delta);
    copy
}

/// Copies the chosen `attributes` of `source` onto each of `targets`. Keyframes keep their
/// timing relative to the clip's start, so an animation that begins as `source` appears
/// begins as each target appears. Locked layers, and `source` itself, are left alone. Returns
/// how many layers changed.
pub fn paste_attributes(project: &mut Project, comp_id: Id, source: &Layer, targets: &[Id], attributes: Attributes) -> usize {
    let mut changed = 0;
    for &target in targets {
        if target == source.id {
            continue;
        }
        // Fresh ids for the copied effects, taken before the composition is borrowed.
        let effect_ids: Vec<Id> = if attributes.effects { source.effects.iter().map(|_| project.allocate_id()).collect() } else { Vec::new() };
        let Some(composition) = project.composition_mut(comp_id) else { return changed };
        let Some(layer) = composition.layer_mut(target).filter(|layer| !layer.switches.locked) else { continue };
        let before = layer.clone();
        let delta = layer.in_frame - source.in_frame;
        if let (Some(from), Some(to)) = (source.transform.as_ref(), layer.transform.as_mut()) {
            if attributes.motion {
                to.anchor_point = shifted(delta, &from.anchor_point);
                to.position = shifted(delta, &from.position);
                to.scale = shifted(delta, &from.scale);
                to.rotation = shifted(delta, &from.rotation);
            }
            if attributes.opacity {
                to.opacity = shifted(delta, &from.opacity);
            }
        }
        if attributes.effects {
            for (effect, id) in source.effects.iter().zip(effect_ids) {
                let mut copy = effect.clone();
                copy.id = id;
                for property in &mut copy.properties {
                    property.value.shift(delta);
                }
                layer.effects.push(copy);
            }
        }
        // Speed only means something between two footage clips that play (not freeze frames).
        let plays = |layer: &Layer| crate::speed::has_speed(&layer.content) && layer.time.hold.is_none();
        if attributes.speed && plays(source) && plays(layer) {
            match source.time.constant_factor() {
                Some(factor) => {
                    crate::speed::set_speed(composition, target, (factor * 100.0) as f32);
                }
                None => {
                    // A ramp keeps the target's length, so its keyframes are simply carried over.
                    let layer = composition.layer_mut(target).expect("found above");
                    layer.time.speed = shifted(delta, &source.time.speed);
                    layer.time.speed_before_ramp = before.time.constant_factor().map(|factor| (factor * 100.0) as f32);
                }
            }
            crate::speed::set_reverse(composition, target, source.time.reverse);
        }
        let layer = composition.layer_mut(target).expect("found above");
        if attributes.blend_mode {
            layer.blend_mode = source.blend_mode;
        }
        if attributes.audio && source.has_audio && layer.has_audio {
            layer.audio.volume_db = source.audio.volume_db;
            layer.audio.fade_in = source.audio.fade_in;
            layer.audio.fade_out = source.audio.fade_out;
        }
        if *layer != before {
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    fn comp(project: &Project) -> &Composition {
        project.composition(ids::MAIN_COMP).unwrap()
    }

    #[test]
    fn duplicating_a_composition_item_copies_its_layers_with_fresh_ids() {
        let mut project = sample_project();
        let original = comp(&project).clone();
        let copy = duplicate_item(&mut project, ids::MAIN_COMP).unwrap();
        let item = project.item(copy).unwrap();
        assert_eq!(item.name, format!("{} copy", project.item(ids::MAIN_COMP).unwrap().name));
        let position = |id| project.items.iter().position(|item| item.id == id).unwrap();
        assert_eq!(position(copy), position(ids::MAIN_COMP) + 1, "right after the original");
        let duplicate = project.composition(copy).unwrap();
        assert_eq!(duplicate.layers.len(), original.layers.len());
        assert_eq!(duplicate.name, item.name);
        for (copied, layer) in duplicate.layers.iter().zip(&original.layers) {
            assert_ne!(copied.id, layer.id, "fresh ids");
            assert_eq!(copied.name, layer.name);
            // Parents point inside the copy, never back into the original.
            if let Some(parent) = copied.parent {
                assert!(duplicate.layer(parent).is_some());
            }
        }
        // A second copy gets a numbered name.
        let again = duplicate_item(&mut project, ids::MAIN_COMP).unwrap();
        assert!(project.item(again).unwrap().name.ends_with("copy 2"));
    }

    #[test]
    fn media_items_duplicate_but_folders_do_not() {
        let mut project = sample_project();
        let copy = duplicate_item(&mut project, ids::LOGO).unwrap();
        assert_eq!(project.item(copy).unwrap().path, project.item(ids::LOGO).unwrap().path);
        assert_eq!(duplicate_item(&mut project, ids::FOOTAGE_FOLDER), None);
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

    #[test]
    fn paste_attributes_copies_motion_and_effects_lined_up_with_each_clip() {
        use crate::animation::{Animated, Keyframe, Easing, Vec2};
        let mut project = sample_project();
        let (source_id, target_id) = (comp(&project).layers[0].id, comp(&project).layers[1].id);
        // The source fades in over its first 10 frames and has one effect.
        let blur = project.allocate_id();
        let composition = project.composition_mut(ids::MAIN_COMP).unwrap();
        let source = composition.layer_mut(source_id).unwrap();
        let start = source.in_frame;
        source.transform.as_mut().unwrap().opacity = Animated::keyframed(vec![
            Keyframe { frame: start, value: 0.0, easing: Easing::Linear },
            Keyframe { frame: start + 10, value: 100.0, easing: Easing::Linear },
        ]);
        source.transform.as_mut().unwrap().position = Animated::constant(Vec2::new(5.0, 6.0));
        source.effects = vec![crate::model::Effect { id: blur, name: "Blur".into(), enabled: true, properties: Vec::new(), lut: None }];
        let source = source.clone();
        let target = composition.layer_mut(target_id).unwrap();
        target.in_frame = start + 40;
        target.out_frame = start + 80;
        let effects_before = target.effects.len();

        let only_opacity = Attributes { motion: false, opacity: true, effects: true, speed: false, blend_mode: false, audio: false };
        assert_eq!(paste_attributes(&mut project, ids::MAIN_COMP, &source, &[target_id, source_id], only_opacity), 1, "the source itself is skipped");
        let target = comp(&project).layer(target_id).unwrap();
        let opacity = &target.transform.as_ref().unwrap().opacity;
        assert_eq!(opacity.keyframes().iter().map(|key| key.frame).collect::<Vec<_>>(), vec![start + 40, start + 50], "the fade starts with the target");
        assert_ne!(target.transform.as_ref().unwrap().position.value, Vec2::new(5.0, 6.0), "motion wasn't chosen");
        assert_eq!(target.effects.len(), effects_before + 1);
        assert_ne!(target.effects.last().unwrap().id, blur, "the copied effect has its own id");
    }

    #[test]
    fn paste_attributes_leaves_locked_layers_alone() {
        let mut project = sample_project();
        let source = comp(&project).layers[0].clone();
        let target = comp(&project).layers[1].id;
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(target).unwrap().switches.locked = true;
        let before = comp(&project).layer(target).unwrap().clone();
        let everything = Attributes { motion: true, opacity: true, effects: true, speed: true, blend_mode: true, audio: true };
        assert_eq!(paste_attributes(&mut project, ids::MAIN_COMP, &source, &[target], everything), 0);
        assert_eq!(comp(&project).layer(target).unwrap(), &before);
    }
}
