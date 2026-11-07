//! Taking a nested composition apart again (Ungroup), the reverse of Pre-compose
//! (`edit::precompose`). The nested composition's layers are copied into this one in its
//! layer's place, at the moments they played; the nested composition itself stays in the
//! project, in case it is used elsewhere.

use crate::model::{Composition, Id, Layer, LayerContent, Project, Rgba, TrackMatte};
use crate::property::TransformProperty;

/// Why a composition layer couldn't be ungrouped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UngroupError {
    /// The layer isn't a nested composition, or is gone.
    NotNested,
    Locked,
    /// The nested composition runs at another frame rate, so its frames don't line up.
    DifferentFrameRate,
    /// None of its layers play while the composition layer is showing.
    Empty,
}

impl std::fmt::Display for UngroupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotNested => "Only a nested composition's layer can be ungrouped.",
            Self::Locked => "The layer is locked. Unlock it to ungroup it.",
            Self::DifferentFrameRate => "The nested composition has a different frame rate, so its layers can't be moved over frame for frame.",
            Self::Empty => "None of the nested composition's layers play while its layer is showing.",
        })
    }
}

/// What ungrouping did: the layers it added (top first), the null that carries the composition
/// layer's placement (if one was needed), and what the composition layer had that its layers
/// can't take over (for a note to the user).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ungrouped {
    pub layers: Vec<Id>,
    pub null: Option<Id>,
    pub dropped: Vec<&'static str>,
}

/// Replaces nested-composition layer `layer_id` in `comp_id` with copies of the nested
/// composition's layers. They play at the same moments as before, trimmed to the composition
/// layer's in and out points. When the composition layer was moved, scaled, turned or parented,
/// a null layer takes over its transform and the copies are parented to it, so everything stays
/// where it was on screen. The nested composition stays in the project.
pub fn ungroup(project: &mut Project, comp_id: Id, layer_id: Id) -> Result<Ungrouped, UngroupError> {
    let outer = project.composition(comp_id).ok_or(UngroupError::NotNested)?;
    let index = outer.layers.iter().position(|layer| layer.id == layer_id).ok_or(UngroupError::NotNested)?;
    let nested_layer = outer.layers[index].clone();
    let LayerContent::Composition { composition: inner_id } = nested_layer.content else { return Err(UngroupError::NotNested) };
    if nested_layer.switches.locked {
        return Err(UngroupError::Locked);
    }
    let inner = project.composition(inner_id).ok_or(UngroupError::NotNested)?;
    if inner.frame_rate != outer.frame_rate {
        return Err(UngroupError::DifferentFrameRate);
    }
    // Inner frame 0 plays at the composition layer's start frame.
    let delta = nested_layer.start_frame;
    let (from, to) = (nested_layer.in_frame, nested_layer.out_frame);
    let playing: Vec<Layer> = inner.layers.iter().filter(|layer| layer.in_frame + delta < to && layer.out_frame + delta > from).cloned().collect();
    if playing.is_empty() {
        return Err(UngroupError::Empty);
    }
    let dropped = dropped_attributes(&nested_layer);
    let needs_null = !places_one_to_one(&nested_layer, inner);

    // The composition layer goes, and the copies take its place in the stack.
    let outer = project.composition_mut(comp_id).ok_or(UngroupError::NotNested)?;
    outer.layers.remove(index);
    let copies = crate::copy::insert_copies(project, comp_id, &playing, index, delta);
    let null = needs_null.then(|| project.allocate_id());
    let outer = project.composition_mut(comp_id).ok_or(UngroupError::NotNested)?;
    for copy in outer.layers.iter_mut().filter(|layer| copies.contains(&layer.id)) {
        // Trimmed to when the composition layer showed (copies always overlap it).
        copy.in_frame = copy.in_frame.max(from);
        copy.out_frame = copy.out_frame.min(to);
        if copy.parent.is_none() {
            copy.parent = null;
        }
    }
    // The block's top copy loses its track matte (nothing above it was copied); the copies
    // below the block keep theirs, which pointed at the composition layer's place.
    if let Some(null) = null {
        let mut carrier = nested_layer.clone();
        carrier.id = null;
        carrier.name = format!("{} (placement)", nested_layer.name);
        carrier.content = LayerContent::Solid { color: Rgba::TRANSPARENT };
        carrier.effects.clear();
        carrier.mask = None;
        carrier.track_matte = TrackMatte::None;
        carrier.cutout = None;
        carrier.text_path = None;
        carrier.has_audio = false;
        carrier.audio = Default::default();
        carrier.blend_mode = crate::model::BlendMode::Normal;
        if let Some(transform) = carrier.transform.as_mut() {
            transform.opacity = crate::animation::Animated::constant(100.0);
        }
        // Below the copies: a null draws nothing, so where it sits doesn't matter.
        let at = (index + copies.len()).min(outer.layers.len());
        carrier.track = crate::tracks::make_room_at(outer, at, false);
        outer.layers.insert(at, carrier);
    }
    // Layers that pointed at the composition layer (as a parent, say) now point at the null,
    // which is where it was, or at nothing.
    for layer in &mut outer.layers {
        if layer.parent == Some(layer_id) {
            layer.parent = null;
        }
        if layer.mask.is_some_and(|mask| mask.layer == layer_id) {
            layer.mask = None;
        }
        if layer.text_path.is_some_and(|on| on.layer == layer_id) {
            layer.text_path = None;
        }
        if layer.audio.duck_under == Some(layer_id) {
            layer.audio.duck_under = None;
        }
    }
    crate::tracks::normalize(outer);
    Ok(Ungrouped { layers: copies, null, dropped })
}

/// Whether the composition layer shows its composition pixel for pixel where it would be
/// anyway: not moved, scaled, turned, parented or animated, and the same size. Then its layers
/// need no null to stay in place.
fn places_one_to_one(layer: &Layer, inner: &Composition) -> bool {
    let Some(transform) = layer.transform.as_ref() else { return true };
    let still = !transform.anchor_point.is_animated() && !transform.position.is_animated() && !transform.scale.is_animated() && !transform.rotation.is_animated();
    let size = inner.size();
    still
        && layer.parent.is_none()
        && transform.rotation.value == 0.0
        && transform.scale.value == crate::animation::Vec2::new(100.0, 100.0)
        && transform.position.value == transform.anchor_point.value
        && layer.size == size
}

/// What the composition layer has that can't move onto its layers, by name.
fn dropped_attributes(layer: &Layer) -> Vec<&'static str> {
    let mut dropped = Vec::new();
    if !layer.effects.is_empty() {
        dropped.push("effects");
    }
    let opacity = layer.property(crate::property::PropertyPath::Transform(TransformProperty::Opacity), layer.in_frame);
    if opacity.is_some_and(|snapshot| !snapshot.keyframes.is_empty() || snapshot.value.component(0) != 100.0) {
        dropped.push("opacity");
    }
    if layer.blend_mode != crate::model::BlendMode::Normal {
        dropped.push("blend mode");
    }
    if layer.mask.is_some() || layer.track_matte != TrackMatte::None {
        dropped.push("mask");
    }
    // Its volume, fades, ducking and noise gate applied to the sound inside it.
    if layer.audio != crate::model::LayerAudio::default() || !layer.switches.audio {
        dropped.push("sound settings");
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::property::{PropertyPath, PropertyValue};
    use crate::sample::{ids, sample_project};

    fn comp(project: &Project) -> &Composition {
        project.composition(ids::MAIN_COMP).unwrap()
    }

    #[test]
    fn ungrouping_puts_the_layers_back_at_their_moments_and_keeps_the_composition() {
        let mut project = sample_project();
        let (title, second) = (comp(&project).layers[0].clone(), comp(&project).layers[1].clone());
        let count = comp(&project).layers.len();
        // Trimmed, so the nested composition starts where the layers did.
        let nested = crate::edit::precompose(&mut project, ids::MAIN_COMP, &[title.id, second.id], "Card", true).unwrap();
        let LayerContent::Composition { composition: inner } = comp(&project).layer(nested).unwrap().content else { panic!() };
        let result = ungroup(&mut project, ids::MAIN_COMP, nested).unwrap();
        assert_eq!(result.null, None, "it hadn't been moved, so no null is needed");
        assert_eq!(comp(&project).layers.len(), count, "two layers again, in place of one");
        assert!(project.composition(inner).is_some() && project.item(inner).is_some(), "the composition stays in the project");
        let back = comp(&project).layer(result.layers[0]).unwrap();
        assert_eq!(back.name, title.name);
        assert_eq!((back.in_frame, back.out_frame), (title.in_frame, title.out_frame), "at the same moments");
        assert_eq!(comp(&project).layers[0].id, result.layers[0], "on top, where they were");
    }

    #[test]
    fn a_moved_composition_layer_leaves_a_null_carrying_its_placement() {
        let mut project = sample_project();
        let title = comp(&project).layers[0].id;
        let nested = crate::edit::precompose(&mut project, ids::MAIN_COMP, &[title], "Card", false).unwrap();
        let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(nested).unwrap();
        layer.set_property(PropertyPath::Transform(TransformProperty::Scale), 0, PropertyValue::Pair(Vec2::new(50.0, 50.0)));
        layer.set_property(PropertyPath::Transform(TransformProperty::Rotation), 0, PropertyValue::Scalar(30.0));
        // Where the title's corners show on screen while nested: through the composition layer.
        let LayerContent::Composition { composition: inner } = comp(&project).layer(nested).unwrap().content else { panic!() };
        let frame = 40.0;
        let placement = crate::transform::world_transform(comp(&project), nested, frame).unwrap();
        let before: Vec<Vec2> = crate::transform::layer_corners(project.composition(inner).unwrap(), title, frame).unwrap().iter().map(|&corner| placement.apply(corner)).collect();
        let result = ungroup(&mut project, ids::MAIN_COMP, nested).unwrap();
        let null = result.null.expect("a null for the scale and turn");
        let copy = comp(&project).layer(result.layers[0]).unwrap();
        assert_eq!(copy.parent, Some(null), "the layer is parented to it");
        let after = crate::transform::layer_corners(comp(&project), copy.id, frame).unwrap();
        for (a, b) in before.iter().zip(after.iter()) {
            assert!((*a - *b).length() < 0.01, "it stays where it was on screen: {before:?} vs {after:?}");
        }
    }

    #[test]
    fn only_nested_compositions_ungroup() {
        let mut project = sample_project();
        let title = comp(&project).layers[0].id;
        assert_eq!(ungroup(&mut project, ids::MAIN_COMP, title), Err(UngroupError::NotNested));
    }
}
