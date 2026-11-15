//! Two chores for a whole stretch of the timeline that Iris can be asked for: swapping layers'
//! places in time ("reverse their order") and cutting a composition down to its work area.
//! Staggering and sequencing are in `edit::stagger_layers`; moving and trimming single layers
//! is in `instructions`.

use crate::edit::CompositionSettings;
use crate::model::{Composition, Id, Project};

/// Swaps `layers`' places in time, so the one that played last plays first. Layers that
/// played back to back still do, from where the first of them started; otherwise each takes
/// the start of the one opposite it. Locked layers stay where they are and take no part. A
/// layer that would now overlap another clip on its track moves to a track of its own.
/// Returns whether anything moved.
pub fn reverse_order(composition: &mut Composition, layers: &[Id]) -> bool {
    // Start, end and layer, in the order they play. Layers that start together keep the order given.
    let mut playing: Vec<(i64, i64, Id)> = layers.iter().filter_map(|id| composition.layer(*id)).filter(|layer| !layer.switches.locked).map(|layer| (layer.in_frame, layer.out_frame, layer.id)).collect();
    playing.sort_by_key(|(start, _, _)| *start);
    if playing.len() < 2 {
        return false;
    }
    let back_to_back = playing.windows(2).all(|pair| pair[0].1 == pair[1].0);
    let mut next = playing[0].0;
    // Every layer is moved before any is settled on its track, so two that swap places on one
    // track aren't taken to overlap halfway through.
    let mut moved = Vec::new();
    for (index, (_, _, id)) in playing.iter().rev().enumerate() {
        let Some(layer) = composition.layer_mut(*id) else { continue };
        let start = if back_to_back { next } else { playing[index].0 };
        next = start + (layer.out_frame - layer.in_frame);
        if start != layer.in_frame {
            layer.shift_in_time(start - layer.in_frame);
            moved.push(*id);
        }
    }
    for id in &moved {
        if let Some((track, start)) = composition.layer(*id).map(|layer| (layer.track, layer.in_frame)) {
            crate::tracks::place_clip(composition, *id, track, start);
        }
    }
    !moved.is_empty()
}

/// What cutting a composition down did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trimmed {
    /// The composition's new length, in frames.
    pub frames: i64,
    /// Layers that ended before the part kept, and were removed.
    pub removed: usize,
    /// Layers that start after the part kept: still there, past the new end.
    pub left_after: usize,
}

/// Cuts composition `comp` down to the frames from `start` up to (not including) `end`, as
/// After Effects' Trim Comp to Work Area does: everything moves earlier by `start`, so what
/// played there now plays first, and the composition becomes `end - start` frames long.
/// Layers that ended before `start` can never show again and are removed; a layer that was
/// playing at `start` now begins at the first frame; markers outside the part kept go too.
/// Layers past the new end are left as they are, as when a composition is shortened in its
/// settings. `None` when the range is already the whole composition, or isn't inside it.
pub fn trim_composition(project: &mut Project, comp: Id, start: i64, end: i64) -> Option<Trimmed> {
    let composition = project.composition_mut(comp)?;
    let duration = composition.duration_frames;
    if start < 0 || end > duration || end <= start || (start == 0 && end == duration) {
        return None;
    }
    let frames = end - start;
    let gone: Vec<Id> = composition.layers.iter().filter(|layer| layer.out_frame <= start && !layer.switches.locked).map(|layer| layer.id).collect();
    crate::edit::delete_layers(composition, &gone);
    for layer in &mut composition.layers {
        layer.shift_in_time(-start);
    }
    composition.markers.retain(|marker| (start..end).contains(&marker.frame));
    for marker in &mut composition.markers {
        marker.frame -= start;
    }
    let cut_short: Vec<Id> = composition.layers.iter().filter(|layer| layer.in_frame < 0 && layer.out_frame > 0).map(|layer| layer.id).collect();
    let left_after = composition.layers.iter().filter(|layer| layer.in_frame >= frames).count();
    let settings = CompositionSettings { name: composition.name.clone(), width: composition.width, height: composition.height, frame_rate: composition.frame_rate, duration_frames: frames, background: composition.background };
    for layer in cut_short {
        crate::trim::trim_in(project, comp, layer, 0);
    }
    crate::edit::apply_composition_settings(project, comp, &settings).ok()?;
    Some(Trimmed { frames, removed: gone.len(), left_after })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit;
    use crate::model::{Marker, Rgba, ShapeKind, ShapeStyle};
    use crate::time::FrameRate;

    /// A 10-second composition with three small shapes spanning the given frames, each on a
    /// track of its own. Returns them in the order given.
    fn scene(spans: [(i64, i64); 3]) -> (Project, Id, [Id; 3]) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = CompositionSettings { name: "Promo".into(), width: 1920, height: 1080, frame_rate: FrameRate::new(30, 1), duration_frames: 300, background: Rgba::hex(0x000000) };
        let comp = edit::new_composition(&mut project, &settings).unwrap();
        let ids = spans.map(|(start, end)| {
            let id = edit::add_shape_layer(&mut project, comp, ShapeStyle::new(ShapeKind::Rectangle), Vec2::new(100.0, 100.0), Vec2::new(960.0, 540.0), start).unwrap();
            project.composition_mut(comp).unwrap().layer_mut(id).unwrap().out_frame = end;
            id
        });
        (project, comp, ids)
    }

    fn spans(project: &Project, comp: Id, ids: [Id; 3]) -> [(i64, i64); 3] {
        ids.map(|id| project.composition(comp).unwrap().layer(id).map_or((-1, -1), |layer| (layer.in_frame, layer.out_frame)))
    }

    #[test]
    fn clips_playing_back_to_back_still_do_in_the_opposite_order() {
        let (mut project, comp, ids) = scene([(30, 60), (60, 150), (150, 170)]);
        assert!(reverse_order(project.composition_mut(comp).unwrap(), &ids));
        assert_eq!(spans(&project, comp, ids), [(140, 170), (50, 140), (30, 50)]);
        // Doing it again puts them back.
        assert!(reverse_order(project.composition_mut(comp).unwrap(), &ids));
        assert_eq!(spans(&project, comp, ids), [(30, 60), (60, 150), (150, 170)]);
    }

    #[test]
    fn staggered_layers_swap_their_starts_and_locked_ones_stay() {
        let (mut project, comp, ids) = scene([(0, 300), (10, 300), (20, 300)]);
        assert!(reverse_order(project.composition_mut(comp).unwrap(), &ids));
        assert_eq!(spans(&project, comp, ids).map(|(start, _)| start), [20, 10, 0]);
        // With the middle one locked, the outer two swap around it.
        project.composition_mut(comp).unwrap().layer_mut(ids[1]).unwrap().switches.locked = true;
        assert!(reverse_order(project.composition_mut(comp).unwrap(), &ids));
        assert_eq!(spans(&project, comp, ids).map(|(start, _)| start), [0, 10, 20]);
        assert!(!reverse_order(project.composition_mut(comp).unwrap(), &ids[..1]), "one layer has no order");
    }

    #[test]
    fn a_composition_is_cut_down_to_a_range() {
        let (mut project, comp, ids) = scene([(0, 40), (30, 200), (250, 300)]);
        project.composition_mut(comp).unwrap().markers = vec![Marker { frame: 10, label: "Early".into() }, Marker { frame: 90, label: "Kept".into() }];
        let trimmed = trim_composition(&mut project, comp, 60, 180).unwrap();
        assert_eq!(trimmed, Trimmed { frames: 120, removed: 1, left_after: 1 });
        let composition = project.composition(comp).unwrap();
        assert_eq!(composition.duration_frames, 120);
        assert!(composition.layer(ids[0]).is_none(), "it ended before the part kept");
        assert_eq!(spans(&project, comp, ids)[1..], [(0, 140), (190, 240)], "the one playing at the cut now starts at the first frame");
        assert_eq!(composition.markers, vec![Marker { frame: 30, label: "Kept".into() }]);
        // The whole composition, or a range outside it, is nothing to trim to.
        assert_eq!(trim_composition(&mut project, comp, 0, 120), None);
        assert_eq!(trim_composition(&mut project, comp, 50, 500), None);
    }
}
