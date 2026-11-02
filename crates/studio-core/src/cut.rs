//! Cutting layers in time: splitting at a frame (After Effects' Split Layer, Premiere's
//! Add Edit) and ripple deleting (removing layers and closing the gap they leave).

use std::collections::BTreeSet;

use crate::model::{Composition, Id, Layer, Project, TrackMatte};

/// What a split did, for telling the user.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SplitReport {
    /// The new second halves, one per layer split, in stacking order.
    pub created: Vec<Id>,
    /// Layers left alone because they are locked.
    pub locked: usize,
    /// Layers left alone because they, or a layer they share a track matte with, do not span
    /// the split frame, so splitting them alone would break the matte.
    pub matte_not_spanning: usize,
}

/// The stack positions of the layers that must be split together with the one at `index`:
/// a run of layers where each one below the first uses the layer above it as a track matte.
/// Splitting all of them, and stacking the second halves in the same order, keeps every
/// half's matte directly above it.
fn matte_group(layers: &[Layer], index: usize) -> std::ops::RangeInclusive<usize> {
    let uses_matte = |i: usize| layers[i].track_matte != TrackMatte::None;
    let mut top = index;
    while top > 0 && uses_matte(top) {
        top -= 1;
    }
    let mut bottom = index;
    while bottom + 1 < layers.len() && uses_matte(bottom + 1) {
        bottom += 1;
    }
    top..=bottom
}

/// The ids of `layer` and the layers it shares a track matte with (see `matte_group`).
pub(crate) fn matte_group_ids(composition: &Composition, layer: Id) -> Vec<Id> {
    let Some(index) = composition.layers.iter().position(|found| found.id == layer) else { return Vec::new() };
    composition.layers[matte_group(&composition.layers, index)].iter().map(|member| member.id).collect()
}

/// A copy of `layer` with fresh ids for itself and its effects.
fn duplicate(project: &mut Project, layer: &Layer) -> Layer {
    let mut copy = layer.clone();
    copy.id = project.allocate_id();
    for effect in &mut copy.effects {
        effect.id = project.allocate_id();
    }
    copy
}

/// Splits layers at `frame`: each keeps everything before it, and a new layer directly above
/// holds the rest (same source timing, keyframes and effects). `layers` are the layers to
/// split; with none given, every layer present at `frame` is split, as in After Effects.
/// Layers that do not span the frame are ignored.
pub fn split_layers(project: &mut Project, comp_id: Id, layers: &[Id], frame: i64) -> SplitReport {
    let mut report = SplitReport::default();
    let Some(composition) = project.composition(comp_id) else { return report };
    let spans = |layer: &Layer| layer.in_frame < frame && frame < layer.out_frame;
    let wanted: Vec<usize> = composition
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| if layers.is_empty() { spans(layer) } else { layers.contains(&layer.id) && spans(layer) })
        .map(|(index, _)| index)
        .collect();

    // Group the layers to split with their matte partners, each group once.
    let groups: BTreeSet<(usize, usize)> = wanted
        .iter()
        .map(|&index| {
            let group = matte_group(&composition.layers, index);
            (*group.start(), *group.end())
        })
        .collect();

    // Work from the bottom of the stack up, so inserting above a group never moves a group
    // still to be split.
    for &(top, bottom) in groups.iter().rev() {
        let composition = project.composition(comp_id).expect("checked above");
        let members = &composition.layers[top..=bottom];
        if members.iter().any(|layer| layer.switches.locked) {
            report.locked += members.iter().filter(|layer| wanted.contains(&composition_index(composition, layer.id))).count();
            continue;
        }
        if !members.iter().all(spans) {
            report.matte_not_spanning += members.iter().filter(|layer| wanted.contains(&composition_index(composition, layer.id))).count();
            continue;
        }
        let originals: Vec<Layer> = members.to_vec();
        let mut halves: Vec<Layer> = originals.iter().map(|layer| duplicate(project, layer)).collect();
        for half in &mut halves {
            half.in_frame = frame;
        }
        let composition = project.composition_mut(comp_id).expect("checked above");
        for layer in &mut composition.layers[top..=bottom] {
            layer.out_frame = frame;
        }
        report.created.splice(0..0, halves.iter().map(|half| half.id));
        composition.layers.splice(top..top, halves);
    }
    // Halves share their originals' tracks (they don't overlap); keep the stack in track order.
    if let Some(composition) = project.composition_mut(comp_id) {
        crate::tracks::normalize(composition);
    }
    report
}

fn composition_index(composition: &Composition, id: Id) -> usize {
    composition.layers.iter().position(|layer| layer.id == id).unwrap_or(usize::MAX)
}

/// Deletes `layers` and closes the gaps they leave, as Premiere's Ripple Delete does: on each
/// deleted layer's track, every remaining clip that starts at or after its out point moves
/// earlier by its length (keyframes included). Other tracks are left alone. Where deleted
/// layers on one track overlap, their combined span is closed once. Locked layers are neither
/// deleted nor moved. Returns how many were deleted.
pub fn ripple_delete(project: &mut Project, comp_id: Id, layers: &[Id]) -> usize {
    let Some(composition) = project.composition_mut(comp_id) else { return 0 };
    let doomed: Vec<Id> = composition.layers.iter().filter(|l| layers.contains(&l.id) && !l.switches.locked).map(|l| l.id).collect();
    // The time spans being removed, merged where they overlap.
    let mut spans: Vec<(i32, i64, i64)> = composition.layers.iter().filter(|l| doomed.contains(&l.id)).map(|l| (l.track, l.in_frame, l.out_frame)).collect();
    spans.sort_unstable();
    let mut merged: Vec<(i32, i64, i64)> = Vec::new();
    for (track, start, end) in spans {
        match merged.last_mut() {
            Some(last) if last.0 == track && start <= last.2 => last.2 = last.2.max(end),
            _ => merged.push((track, start, end)),
        }
    }
    // Close the gaps before deleting, so emptied tracks being renumbered can't mix them up.
    for layer in composition.layers.iter_mut().filter(|layer| !layer.switches.locked && !doomed.contains(&layer.id)) {
        // Every removed span on this layer's track that ends by its start pulls it earlier.
        let shift: i64 = merged.iter().filter(|(track, _, end)| *track == layer.track && *end <= layer.in_frame).map(|(_, start, end)| end - start).sum();
        if shift > 0 {
            layer.shift_in_time(-shift);
        }
    }
    crate::edit::delete_layers(composition, &doomed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LayerContent, Rgba};
    use crate::sample::{ids, sample_project};

    fn comp(project: &Project) -> &Composition {
        project.composition(ids::MAIN_COMP).unwrap()
    }

    fn span(project: &Project, id: Id) -> (i64, i64) {
        let layer = comp(project).layer(id).unwrap();
        (layer.in_frame, layer.out_frame)
    }

    /// A composition of plain solids, top first, each with the given (in, out) frames.
    fn solids(spans: &[(i64, i64)]) -> (Project, Vec<Id>) {
        let mut project = sample_project();
        let template = comp(&project).layers.iter().find(|l| matches!(l.content, LayerContent::Solid { .. })).unwrap().clone();
        let mut ids_out = Vec::new();
        let mut layers = Vec::new();
        for &(in_frame, out_frame) in spans {
            let id = project.allocate_id();
            ids_out.push(id);
            layers.push(Layer { id, in_frame, out_frame, start_frame: in_frame, content: LayerContent::Solid { color: Rgba::hex(0x808080) }, ..template.clone() });
        }
        project.composition_mut(ids::MAIN_COMP).unwrap().layers = layers;
        (project, ids_out)
    }

    #[test]
    fn splitting_keeps_both_halves_in_time_and_stacks_the_second_above() {
        let mut project = sample_project();
        let title = Id(100);
        let original = comp(&project).layer(title).unwrap().clone();
        let report = split_layers(&mut project, ids::MAIN_COMP, &[title], 100);
        let [second] = report.created[..] else { panic!("one new layer: {report:?}") };
        assert_eq!(span(&project, title), (original.in_frame, 100));
        assert_eq!(span(&project, second), (100, original.out_frame));
        assert_eq!(comp(&project).layers[0].id, second, "the second half sits directly above");
        assert_eq!(comp(&project).layers[1].id, title);
        let copy = comp(&project).layer(second).unwrap();
        assert_eq!(copy.start_frame, original.start_frame, "footage keeps playing from the same place");
        assert_eq!(copy.transform, original.transform, "keyframes carry over");
        assert!(copy.effects.iter().zip(&original.effects).all(|(a, b)| a.id != b.id), "effects get their own ids");
    }

    #[test]
    fn with_nothing_selected_every_layer_at_the_frame_splits() {
        let (mut project, layers) = solids(&[(0, 50), (10, 100), (60, 90)]);
        let report = split_layers(&mut project, ids::MAIN_COMP, &[], 40);
        assert_eq!(report.created.len(), 2);
        assert_eq!(span(&project, layers[2]), (60, 90), "the layer not present at frame 40 is untouched");
    }

    #[test]
    fn layers_not_spanning_the_frame_are_ignored() {
        let (mut project, layers) = solids(&[(0, 50)]);
        assert!(split_layers(&mut project, ids::MAIN_COMP, &layers, 0).created.is_empty(), "at the in point there is nothing before");
        assert!(split_layers(&mut project, ids::MAIN_COMP, &layers, 50).created.is_empty());
    }

    #[test]
    fn a_track_matte_splits_with_its_layer() {
        let (mut project, layers) = solids(&[(0, 100), (0, 100), (0, 100)]);
        // The middle layer uses the top one as its matte; the bottom one is unrelated.
        project.composition_mut(ids::MAIN_COMP).unwrap().layers[1].track_matte = TrackMatte::Alpha;
        let report = split_layers(&mut project, ids::MAIN_COMP, &[layers[1]], 40);
        assert_eq!(report.created.len(), 2, "the matte is split too");
        let stack: Vec<_> = comp(&project).layers.iter().map(|l| (l.in_frame, l.out_frame, l.track_matte != TrackMatte::None)).collect();
        // Second halves (matte, then matted layer) above the first halves, so each keeps its matte above it.
        assert_eq!(stack, vec![(40, 100, false), (40, 100, true), (0, 40, false), (0, 40, true), (0, 100, false)]);
    }

    #[test]
    fn a_matte_that_ends_earlier_blocks_the_split_and_says_so() {
        let (mut project, layers) = solids(&[(0, 30), (0, 100)]);
        project.composition_mut(ids::MAIN_COMP).unwrap().layers[1].track_matte = TrackMatte::Alpha;
        let report = split_layers(&mut project, ids::MAIN_COMP, &[layers[1]], 40);
        assert!(report.created.is_empty());
        assert_eq!(report.matte_not_spanning, 1);
        assert_eq!(span(&project, layers[1]), (0, 100));
    }

    #[test]
    fn locked_layers_are_not_split() {
        let (mut project, layers) = solids(&[(0, 100)]);
        project.composition_mut(ids::MAIN_COMP).unwrap().layers[0].switches.locked = true;
        let report = split_layers(&mut project, ids::MAIN_COMP, &layers, 40);
        assert_eq!((report.created.len(), report.locked), (0, 1));
    }

    #[test]
    fn ripple_delete_closes_the_gap() {
        // Three clips one after another, and one overlapping the first.
        let (mut project, layers) = solids(&[(0, 24), (24, 48), (48, 96), (10, 30)]);
        assert_eq!(ripple_delete(&mut project, ids::MAIN_COMP, &[layers[1]]), 1);
        assert_eq!(span(&project, layers[2]), (24, 72), "the clip after moves up by 24 frames");
        assert_eq!(span(&project, layers[0]), (0, 24), "earlier clips stay");
        assert_eq!(span(&project, layers[3]), (10, 30), "a clip starting inside the removed span stays");
    }

    #[test]
    fn ripple_deleting_several_closes_each_gap_once() {
        let (mut project, layers) = solids(&[(0, 10), (10, 20), (15, 30), (40, 50), (60, 70)]);
        // Delete the overlapping pair (10–30 combined) and 40–50: the last clip moves up 30.
        ripple_delete(&mut project, ids::MAIN_COMP, &[layers[1], layers[2], layers[3]]);
        assert_eq!(span(&project, layers[4]), (30, 40));
        assert_eq!(span(&project, layers[0]), (0, 10));
    }

    #[test]
    fn ripple_delete_closes_the_gap_only_on_its_own_track() {
        let (mut project, layers) = solids(&[(0, 24), (24, 48), (24, 48)]);
        // The third clip is on another track: deleting the first must not move it.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        for (layer, track) in comp.layers.iter_mut().zip([0, 0, 1]) {
            layer.track = track;
        }
        ripple_delete(&mut project, ids::MAIN_COMP, &[layers[0]]);
        assert_eq!(span(&project, layers[1]), (0, 24), "the next clip on the same track moves up");
        assert_eq!(span(&project, layers[2]), (24, 48), "a clip on another track stays");
    }

    #[test]
    fn ripple_delete_leaves_locked_layers_alone() {
        let (mut project, layers) = solids(&[(0, 24), (24, 48)]);
        project.composition_mut(ids::MAIN_COMP).unwrap().layers[1].switches.locked = true;
        ripple_delete(&mut project, ids::MAIN_COMP, &[layers[0]]);
        assert_eq!(span(&project, layers[1]), (24, 48), "locked layers do not move");
    }
}
