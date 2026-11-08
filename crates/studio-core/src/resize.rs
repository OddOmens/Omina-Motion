//! Magic Resize: a copy of a composition in another size (9:16 from 16:9, say), with its layers
//! rearranged to suit it, like Canva's resize.
//!
//! - Layers that fill the frame (backgrounds, full-frame footage) are scaled to cover the new
//!   frame, centered as they were.
//! - Everything else keeps its place relative to the frame (a title a third of the way down
//!   stays a third of the way down), and is scaled by how the frame's area changed, so it keeps
//!   its weight on screen. Anything that would then be wider than the new frame is made smaller
//!   to fit.
//! - Layers parented to another move with their parent and are otherwise left alone.
//!
//! Keyframed positions and scales are mapped the same way at every keyframe, so animation is
//! kept. The original composition is not changed.

use std::collections::HashMap;

use crate::animation::Vec2;
use crate::edit::CompositionSettings;
use crate::model::{Id, Project};

/// How much of the frame a layer must cover, each way, to count as a background.
const FULL_FRAME: f32 = 0.85;
/// The share of the new frame's width a layer may take before it is made smaller.
const MAX_WIDTH: f32 = 0.92;

/// Makes a copy of composition `comp` sized `width` × `height`, named `name`. Returns the new
/// composition.
pub fn resize_composition(project: &mut Project, comp: Id, width: u32, height: u32, name: &str) -> Option<Id> {
    let source = project.composition(comp)?.clone();
    let settings = CompositionSettings {
        name: name.to_owned(),
        width,
        height,
        frame_rate: source.frame_rate,
        duration_frames: source.duration_frames,
        background: source.background,
    };
    let new_comp = crate::edit::new_composition(project, &settings).ok()?;
    let (old, new) = (Vec2::new(source.width as f32, source.height as f32), Vec2::new(width as f32, height as f32));
    // How the frame's size changed overall, and how much a full-frame layer must grow to cover it.
    let area = ((new.x * new.y) / (old.x * old.y)).sqrt().clamp(0.25, 4.0);
    let cover = (new.x / old.x).max(new.y / old.y);
    let to_new = |point: Vec2| Vec2::new(point.x / old.x * new.x, point.y / old.y * new.y);
    let around_center = |point: Vec2, factor: f32| new * 0.5 + (point - old * 0.5) * factor;

    // Fresh ids, and the old ones mapped to them so parents, masks and the like still connect.
    let ids: HashMap<Id, Id> = source.layers.iter().map(|layer| (layer.id, project.allocate_id())).collect();
    let mut layers = Vec::with_capacity(source.layers.len());
    for layer in &source.layers {
        let mut copy = layer.clone();
        copy.id = ids[&layer.id];
        // Fresh ids for its effects and Merge nodes, with its node graph pointed at them.
        crate::effect_graph::reassign_ids(&mut copy, || project.allocate_id());
        let remap = |id: Id| ids.get(&id).copied();
        copy.parent = copy.parent.and_then(remap);
        copy.mask = copy.mask.and_then(|mask| remap(mask.layer).map(|layer| crate::model::LayerMask { layer, ..mask }));
        copy.text_path = copy.text_path.and_then(|on| remap(on.layer).map(|layer| crate::model::TextPath { layer, ..on }));
        copy.audio.duck_under = copy.audio.duck_under.and_then(remap);
        if layer.parent.is_none()
            && let Some(transform) = copy.transform.as_mut()
        {
            let bounds = crate::align::layer_bounds(&source, layer.id, layer.in_frame.max(0));
            let size = bounds.map_or(Vec2::default(), |bounds| bounds.max - bounds.min);
            let background = size.x >= old.x * FULL_FRAME && size.y >= old.y * FULL_FRAME;
            if background {
                transform.position.map_values(|point| around_center(point, cover));
                transform.scale.map_values(|scale| scale * cover);
            } else {
                // Its weight on screen kept, but never wider than the new frame allows.
                let wide = size.x * area;
                let fit = if wide > new.x * MAX_WIDTH { new.x * MAX_WIDTH / wide } else { 1.0 };
                transform.position.map_values(to_new);
                transform.scale.map_values(|scale| scale * area * fit);
            }
        }
        layers.push(copy);
    }
    let composition = project.composition_mut(new_comp)?;
    composition.layers = layers;
    composition.markers = source.markers.clone();
    Some(new_comp)
}

/// The sizes offered for resizing: name, width, height.
pub const SIZES: [(&str, u32, u32); 6] = [
    ("Vertical 9:16 · Reels, TikTok, Shorts", 1080, 1920),
    ("Portrait 4:5 · Instagram feed", 1080, 1350),
    ("Square 1:1 · Instagram, Facebook", 1080, 1080),
    ("Landscape 16:9 · YouTube", 1920, 1080),
    ("4K 16:9", 3840, 2160),
    ("Pinterest 2:3", 1000, 1500),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    #[test]
    fn a_wide_composition_becomes_a_tall_one_with_its_layers_in_their_places() {
        let mut project = sample_project();
        let tall = resize_composition(&mut project, ids::MAIN_COMP, 1080, 1920, "Main Comp 9:16").unwrap();
        let (source, made) = (project.composition(ids::MAIN_COMP).unwrap(), project.composition(tall).unwrap());
        assert_eq!((made.width, made.height), (1080, 1920));
        assert_eq!(made.layers.len(), source.layers.len());
        // The full-frame background solid now covers the tall frame.
        let background = made.layers.iter().find(|layer| layer.name.contains("Solid")).unwrap();
        let bounds = crate::align::layer_bounds(made, background.id, background.in_frame).unwrap();
        assert!(bounds.min.x <= 0.5 && bounds.min.y <= 0.5 && bounds.max.x >= 1079.5 && bounds.max.y >= 1919.5, "covers the frame: {bounds:?}");
        // The title stays centered and fits across the narrower frame.
        let title = made.layers.iter().find(|layer| layer.name == "Title").unwrap();
        let bounds = crate::align::layer_bounds(made, title.id, 100).unwrap();
        assert!(bounds.max.x - bounds.min.x <= 1080.0 * MAX_WIDTH + 1.0, "{bounds:?}");
        assert!(((bounds.min.x + bounds.max.x) * 0.5 - 540.0).abs() < 2.0);
        // Ids are new, and parenting still connects within the copy.
        assert!(made.layers.iter().all(|layer| source.layer(layer.id).is_none()));
        for layer in &made.layers {
            if let Some(parent) = layer.parent {
                assert!(made.layer(parent).is_some());
            }
        }
    }
}
