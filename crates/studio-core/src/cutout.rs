//! Edits to a layer's object matte (see `model::Cutout`). The masks themselves are made by the
//! app with the `studio-cutout` model; these edits keep the strokes and the record of which
//! masks are current in step.

use crate::animation::Vec2;
use crate::model::{Composition, Cutout, Id, Layer, LayerContent, MatteEdges, MatteStroke, MAX_MATTE_STROKES, MAX_STROKE_POINTS};

/// Whether a layer's picture can have an object matte: footage and images.
pub fn can_cut_out(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Image { .. })
}

/// The brush radius of a stroke made from an old click, as a fraction of the picture's width.
const CLICK_RADIUS: f32 = 0.015;

/// Whether a stroke is usable: on the picture, with a sensible brush and point count.
pub fn stroke_is_valid(stroke: &MatteStroke) -> bool {
    let on_picture = |at: &Vec2| (0.0..=1.0).contains(&at.x) && (0.0..=1.0).contains(&at.y);
    !stroke.points.is_empty() && stroke.points.len() <= MAX_STROKE_POINTS && stroke.points.iter().all(on_picture) && stroke.radius.is_finite() && stroke.radius > 0.0 && stroke.radius <= 0.5
}

/// Keeps a stroke's points within the limit by dropping evenly spaced ones (the first and last
/// stay), so a long scribble still fits.
fn thin_points(points: &mut Vec<Vec2>) {
    if points.len() <= MAX_STROKE_POINTS {
        return;
    }
    let step = points.len() as f32 / (MAX_STROKE_POINTS - 1) as f32;
    let last = *points.last().unwrap_or(&Vec2::new(0.0, 0.0));
    let mut kept: Vec<Vec2> = (0..MAX_STROKE_POINTS - 1).map(|index| points[(index as f32 * step) as usize]).collect();
    kept.push(last);
    *points = kept;
}

/// Adds a brush stroke. Masks of its frame and every later frame are no longer current (they
/// follow from this frame on); earlier ones are kept. Returns whether it was added: the layer
/// must take a matte and not be locked, and the stroke must be on the picture.
pub fn add_stroke(composition: &mut Composition, layer_id: Id, mut stroke: MatteStroke) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| can_cut_out(layer) && !layer.switches.locked) else { return false };
    // Points just past the edge (a stroke that wandered off the picture) are pulled back on.
    for point in &mut stroke.points {
        *point = Vec2::new(point.x.clamp(0.0, 1.0), point.y.clamp(0.0, 1.0));
    }
    thin_points(&mut stroke.points);
    if !stroke_is_valid(&stroke) {
        return false;
    }
    let cutout = layer.cutout.get_or_insert_with(Cutout::default);
    if cutout.strokes.len() >= MAX_MATTE_STROKES {
        return false;
    }
    cutout.frames = keep_before(cutout.frames, stroke.frame);
    cutout.strokes.push(stroke);
    true
}

/// The range of masks still current after frame `frame` changes: only frames before it.
fn keep_before(frames: Option<(i64, i64)>, frame: i64) -> Option<(i64, i64)> {
    frames.and_then(|(start, end)| (frame > start).then_some((start, end.min(frame))))
}

/// Removes the strokes on source frame `frame` (and the masks from it on). Returns whether
/// there were any.
pub fn clear_frame(composition: &mut Composition, layer_id: Id, frame: i64) -> bool {
    let Some(cutout) = composition.layer_mut(layer_id).filter(|layer| !layer.switches.locked).and_then(|layer| layer.cutout.as_mut()) else { return false };
    let before = cutout.strokes.len();
    cutout.strokes.retain(|stroke| stroke.frame != frame);
    if cutout.strokes.len() == before {
        return false;
    }
    cutout.frames = if cutout.strokes.is_empty() { None } else { keep_before(cutout.frames, frame) };
    true
}

/// Removes a layer's object matte, showing its whole picture again.
pub fn clear(composition: &mut Composition, layer_id: Id) {
    if let Some(layer) = composition.layer_mut(layer_id) {
        layer.cutout = None;
    }
}

/// Keeps what was painted (`remove` false) or removes it instead.
pub fn set_remove(composition: &mut Composition, layer_id: Id, remove: bool) {
    if let Some(cutout) = composition.layer_mut(layer_id).and_then(|layer| layer.cutout.as_mut()) {
        cutout.remove = remove;
    }
}

/// Changes how the matte's edge is finished (brought into range).
pub fn set_edges(composition: &mut Composition, layer_id: Id, edges: MatteEdges) {
    if let Some(cutout) = composition.layer_mut(layer_id).filter(|layer| !layer.switches.locked).and_then(|layer| layer.cutout.as_mut()) {
        cutout.edges = edges.clamped();
    }
}

/// Records masks made for source frames `frames` in `folder`, if they were made from the
/// strokes the layer still has (`strokes`): strokes changed while the masks were being made
/// make them stale, and they are not used. Returns whether they were recorded.
pub fn set_masks(composition: &mut Composition, layer_id: Id, strokes: &[MatteStroke], folder: std::path::PathBuf, frames: (i64, i64)) -> bool {
    let Some(cutout) = composition.layer_mut(layer_id).and_then(|layer| layer.cutout.as_mut()) else { return false };
    if cutout.strokes != strokes || frames.1 <= frames.0 {
        return false;
    }
    // Masks made further through the clip add to the ones already made around them.
    let frames = match cutout.frames.filter(|_| cutout.masks.as_ref() == Some(&folder)) {
        Some((start, end)) if frames.0 <= end && start <= frames.1 => (start.min(frames.0), end.max(frames.1)),
        _ => frames,
    };
    cutout.masks = Some(folder);
    cutout.frames = Some(frames);
    true
}

/// Turns the clicks an older file saved into short strokes on their frame, and forgets them.
/// Their masks are kept: they were made from the same picks.
pub fn upgrade_clicks(cutout: &mut Cutout) {
    let frame = cutout.old_click_frame;
    let strokes = cutout.old_clicks.drain(..).map(|click| MatteStroke { frame, points: vec![click.at], radius: CLICK_RADIUS, add: click.keep });
    cutout.strokes.extend(strokes);
    cutout.old_click_frame = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LayerSwitches, Project};
    use crate::sample::{self, ids};

    fn footage_layer(project: &mut Project) -> Id {
        let info = crate::edit::FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: false };
        let item = crate::edit::import_footage(project, "/media/dancer.mp4".into(), &info);
        crate::edit::add_layer_from_item(project, ids::MAIN_COMP, item, 0, 0).unwrap()
    }

    fn stroke(frame: i64, x: f32, add: bool) -> MatteStroke {
        MatteStroke { frame, points: vec![Vec2::new(x, 0.5), Vec2::new(x + 0.1, 0.55)], radius: 0.02, add }
    }

    #[test]
    fn strokes_land_only_on_pictures_that_can_take_a_matte() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(add_stroke(comp, layer, stroke(10, 0.5, true)));
        assert!(add_stroke(comp, layer, stroke(40, 0.2, false)), "strokes on other frames add corrections");
        assert!(add_stroke(comp, layer, MatteStroke { frame: 10, points: vec![Vec2::new(1.2, -0.1)], radius: 0.02, add: true }), "points just off the picture are pulled on");
        assert!(!add_stroke(comp, layer, MatteStroke { frame: 10, points: vec![], radius: 0.02, add: true }), "an empty stroke");
        assert!(!add_stroke(comp, layer, MatteStroke { frame: 10, points: vec![Vec2::new(0.5, 0.5)], radius: f32::NAN, add: true }));
        let cutout = comp.layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!(cutout.base_frames(), vec![10, 40]);
        assert_eq!(cutout.strokes_on(10).count(), 2);
        let solid = comp.layers.iter().find(|layer| matches!(layer.content, LayerContent::Solid { .. })).unwrap().id;
        assert!(!add_stroke(comp, solid, stroke(0, 0.5, true)), "only footage and images");
        comp.layer_mut(layer).unwrap().switches = LayerSwitches { locked: true, ..LayerSwitches::default() };
        assert!(!add_stroke(comp, layer, stroke(10, 0.5, true)), "locked layers stay as they are");
    }

    #[test]
    fn long_strokes_are_thinned_to_the_limit() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let points: Vec<Vec2> = (0..5000).map(|index| Vec2::new(index as f32 / 5000.0, 0.5)).collect();
        assert!(add_stroke(comp, layer, MatteStroke { frame: 0, points, radius: 0.02, add: true }));
        let kept = &comp.layer(layer).unwrap().cutout.as_ref().unwrap().strokes[0].points;
        assert_eq!(kept.len(), MAX_STROKE_POINTS);
        assert_eq!(kept.last().unwrap().x, 4999.0 / 5000.0, "the end of the stroke is kept");
    }

    #[test]
    fn masks_are_kept_for_their_strokes_and_a_correction_keeps_earlier_frames() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        add_stroke(comp, layer, stroke(10, 0.5, true));
        let strokes = comp.layer(layer).unwrap().cutout.as_ref().unwrap().strokes.clone();
        assert!(set_masks(comp, layer, &strokes, "/cache/a".into(), (10, 11)));
        assert!(set_masks(comp, layer, &strokes, "/cache/a".into(), (11, 60)), "following adds to the painted frame");
        let cutout = comp.layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!(cutout.frames, Some((10, 60)));
        assert_eq!(cutout.mask_path(5), Some("/cache/a/10.png".into()), "before the first mask, the first is used");
        assert_eq!(cutout.mask_path(99), Some("/cache/a/59.png".into()));
        // A correction on frame 30: frames before it keep their masks, the rest are remade.
        add_stroke(comp, layer, stroke(30, 0.3, false));
        assert!(!set_masks(comp, layer, &strokes, "/cache/a".into(), (10, 60)), "masks from the old strokes are stale");
        assert_eq!(comp.layer(layer).unwrap().cutout.as_ref().unwrap().frames, Some((10, 30)));
        // Clearing frame 30 again keeps frames before it; clearing the last strokes clears all.
        assert!(clear_frame(comp, layer, 30));
        assert_eq!(comp.layer(layer).unwrap().cutout.as_ref().unwrap().frames, Some((10, 30)));
        assert!(clear_frame(comp, layer, 10));
        assert_eq!(comp.layer(layer).unwrap().cutout.as_ref().unwrap().frames, None);
        assert!(!clear_frame(comp, layer, 10), "nothing left there");
    }

    #[test]
    fn edge_settings_are_kept_in_range_and_old_clicks_become_strokes() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        add_stroke(comp, layer, stroke(0, 0.5, true));
        set_edges(comp, layer, MatteEdges { feather: 900.0, contrast: -5.0, shift_edge: 150.0, reduce_chatter: f32::NAN, refine_detail: 30.0 });
        let edges = comp.layer(layer).unwrap().cutout.as_ref().unwrap().edges;
        assert_eq!(edges, MatteEdges { feather: 200.0, contrast: 0.0, shift_edge: 100.0, reduce_chatter: 0.0, refine_detail: 30.0 });

        let mut old = Cutout { old_clicks: vec![crate::model::CutoutClick { at: Vec2::new(0.25, 0.5), keep: false }], old_click_frame: 12, ..Cutout::default() };
        upgrade_clicks(&mut old);
        assert!(old.old_clicks.is_empty());
        assert_eq!(old.strokes, vec![MatteStroke { frame: 12, points: vec![Vec2::new(0.25, 0.5)], radius: CLICK_RADIUS, add: false }]);
    }
}
