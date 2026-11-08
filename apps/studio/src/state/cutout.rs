//! The object matte on the selected footage or image layer: painting brush strokes that mark
//! the object (and what isn't part of it), and recording the masks the background job makes
//! from them (see `panels::cutout`).

use std::path::PathBuf;

use studio_core::animation::Vec2;
use studio_core::cutout;
use studio_core::model::{Id, LayerContent, MatteEdges, MatteStroke};
use studio_core::transform::world_transform;

use super::EditorState;

/// What a cut-out is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct CutoutSource {
    pub path: PathBuf,
    /// `None` for an image; for footage, the source frames the layer shows (start, end
    /// exclusive), which tracking covers.
    pub frames: Option<(i64, i64)>,
    /// The source frames the composition actually shows, which are the only ones worth asking
    /// the model about (a 60 fps clip in a 30 fps composition shows every other frame). `None`
    /// means all of them.
    pub shown: Option<std::collections::BTreeSet<i64>>,
}

impl EditorState {
    /// The file a layer's cut-out is made from, and which of its frames are in use.
    pub fn cutout_source(&self, layer: Id) -> Option<CutoutSource> {
        let composition = self.composition();
        let layer = composition.layer(layer)?;
        match layer.content {
            LayerContent::Image { item } => Some(CutoutSource { path: self.project.item(item)?.path.clone()?, frames: None, shown: None }),
            LayerContent::Footage { item } => {
                let item = self.project.item(item)?;
                let rate = item.frame_rate?;
                let first = layer.source_frame(layer.in_frame, composition.frame_rate, rate).max(0);
                let last = layer.source_frame(layer.out_frame - 1, composition.frame_rate, rate).max(first);
                let shown = (layer.in_frame..layer.out_frame).map(|frame| layer.source_frame(frame, composition.frame_rate, rate).clamp(first, last)).collect();
                Some(CutoutSource { path: item.path.clone()?, frames: Some((first, last + 1)), shown: Some(shown) })
            }
            _ => None,
        }
    }

    /// The source frame a cut-out layer shows at the playhead (0 for an image).
    pub fn cutout_frame_at_playhead(&self, layer: Id) -> Option<i64> {
        let composition = self.composition();
        let target = composition.layer(layer)?;
        match target.content {
            LayerContent::Image { .. } => Some(0),
            LayerContent::Footage { item } => {
                let rate = self.project.item(item)?.frame_rate?;
                let (first, end) = self.cutout_source(layer)?.frames?;
                Some(target.source_frame(self.current_frame, composition.frame_rate, rate).clamp(first, end - 1))
            }
            _ => None,
        }
    }

    /// Adds a brush stroke painted along `points` (composition pixels) with a brush `radius`
    /// composition pixels wide, on the layer's picture at the playhead: one marking the object,
    /// or (with `add` false) something that isn't part of it. Returns whether it was added.
    pub fn add_matte_stroke(&mut self, layer: Id, points: &[Vec2], radius: f32, add: bool) -> bool {
        let comp = self.composition();
        let Some(target) = comp.layer(layer).filter(|target| target.is_active_at(self.current_frame)) else { return false };
        let Some(to_local) = world_transform(comp, layer, self.current_frame as f64).and_then(|world| world.inverse()) else { return false };
        let size = Vec2::new(target.size.x.max(1.0), target.size.y.max(1.0));
        let local: Vec<Vec2> = points.iter().map(|point| to_local.apply(*point)).map(|at| Vec2::new(at.x / size.x, at.y / size.y)).collect();
        // A stroke that stays off the picture paints nothing.
        if !local.iter().any(|at| (0.0..=1.0).contains(&at.x) && (0.0..=1.0).contains(&at.y)) {
            return false;
        }
        let radius = to_local.apply_vector(Vec2::new(radius, 0.0)).length() / size.x;
        let Some(frame) = self.cutout_frame_at_playhead(layer) else { return false };
        cutout::add_stroke(self.composition_mut(), layer, MatteStroke { frame, points: local, radius, add })
    }

    /// The strokes painted on the frame at the playhead, on screen: each one's points and brush
    /// radius in composition pixels, and whether it adds to the object.
    pub fn matte_strokes_on_screen(&self, layer: Id) -> Vec<(Vec<Vec2>, f32, bool)> {
        let comp = self.composition();
        let Some(target) = comp.layer(layer) else { return Vec::new() };
        let (Some(cut), Some(frame)) = (target.cutout.as_ref(), self.cutout_frame_at_playhead(layer)) else { return Vec::new() };
        let Some(world) = world_transform(comp, layer, self.current_frame as f64) else { return Vec::new() };
        let place = |at: &Vec2| world.apply(Vec2::new(at.x * target.size.x, at.y * target.size.y));
        cut.strokes_on(frame)
            .map(|stroke| {
                let radius = world.apply_vector(Vec2::new(stroke.radius * target.size.x, 0.0)).length();
                (stroke.points.iter().map(place).collect(), radius, stroke.add)
            })
            .collect()
    }

    /// Removes the strokes on the frame at the playhead. Returns whether there were any.
    pub fn clear_matte_frame(&mut self, layer: Id) -> bool {
        let Some(frame) = self.cutout_frame_at_playhead(layer) else { return false };
        cutout::clear_frame(self.composition_mut(), layer, frame)
    }

    /// Changes how the matte's edge is finished.
    pub fn set_matte_edges(&mut self, layer: Id, edges: MatteEdges) {
        cutout::set_edges(self.composition_mut(), layer, edges);
    }

    pub fn clear_cutout(&mut self, layer: Id) {
        cutout::clear(self.composition_mut(), layer);
    }

    pub fn set_cutout_remove(&mut self, layer: Id, remove: bool) {
        cutout::set_remove(self.composition_mut(), layer, remove);
    }

    /// Records masks the background job made from `strokes`. This follows the user's strokes
    /// rather than being an edit of its own, so it is not an undo step.
    pub fn record_cutout_masks(&mut self, comp: Id, layer: Id, strokes: &[MatteStroke], folder: PathBuf, frames: (i64, i64)) -> bool {
        self.settle_drags();
        let Some(composition) = self.project.composition_mut(comp) else { return false };
        if !cutout::set_masks(composition, layer, strokes, folder.clone(), frames) {
            return false;
        }
        self.history.absorb_everywhere(|project| {
            if let Some(composition) = project.composition_mut(comp) {
                cutout::set_masks(composition, layer, strokes, folder.clone(), frames);
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_land_on_the_layers_picture_where_they_were_painted() {
        let mut state = EditorState::new();
        let item = studio_core::edit::import_image(&mut state.project, "/media/portrait.png".into(), Vec2::new(400.0, 200.0));
        let comp = state.active_comp;
        let layer = studio_core::edit::add_layer_from_item(&mut state.project, comp, item, 0, 0).unwrap();
        // The image is centered in the 1920 × 1080 frame: 760..1160 across, 440..640 down.
        assert!(state.add_matte_stroke(layer, &[Vec2::new(860.0, 490.0), Vec2::new(960.0, 540.0)], 20.0, true));
        assert!(!state.add_matte_stroke(layer, &[Vec2::new(100.0, 100.0)], 20.0, true), "off the picture");
        let cut = state.composition().layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!(cut.strokes.len(), 1);
        assert_eq!(cut.strokes[0].points, vec![Vec2::new(0.25, 0.25), Vec2::new(0.5, 0.5)]);
        assert!((cut.strokes[0].radius - 0.05).abs() < 1e-4, "20 px of a 400 px picture");
        let shown = state.matte_strokes_on_screen(layer);
        assert!((shown[0].0[0] - Vec2::new(860.0, 490.0)).length() < 1e-3);
        assert!((shown[0].1 - 20.0).abs() < 1e-3);
        assert_eq!(state.cutout_source(layer).unwrap().frames, None, "an image has one frame");
        assert!(state.clear_matte_frame(layer));
        assert!(state.matte_strokes_on_screen(layer).is_empty());
    }
}
