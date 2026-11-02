//! Background removal on the selected footage or image layer: placing clicks that pick out the
//! subject, and recording the masks the background job makes from them (see
//! `panels::cutout`).

use std::path::PathBuf;

use studio_core::animation::Vec2;
use studio_core::cutout;
use studio_core::model::{CutoutClick, Id, LayerContent};
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

    /// Adds a click at `point` (composition pixels) on a cut-out layer's picture, at the
    /// playhead: one marking something to keep, or (with `keep` false) to leave out. Returns
    /// whether it landed on the picture.
    pub fn add_cutout_click(&mut self, layer: Id, point: Vec2, keep: bool) -> bool {
        let comp = self.composition();
        let Some(target) = comp.layer(layer).filter(|target| target.is_active_at(self.current_frame)) else { return false };
        let Some(to_local) = world_transform(comp, layer, self.current_frame as f64).and_then(|world| world.inverse()) else { return false };
        let local = to_local.apply(point);
        let at = Vec2::new(local.x / target.size.x.max(1.0), local.y / target.size.y.max(1.0));
        let Some(frame) = self.cutout_frame_at_playhead(layer) else { return false };
        cutout::add_click(self.composition_mut(), layer, CutoutClick { at, keep }, frame)
    }

    /// Where a cut-out's clicks are on screen (composition pixels) and whether each keeps or
    /// leaves out, while the playhead is on the frame they were placed on.
    pub fn cutout_clicks_on_screen(&self, layer: Id) -> Vec<(Vec2, bool)> {
        let comp = self.composition();
        let Some(target) = comp.layer(layer) else { return Vec::new() };
        let Some(cut) = target.cutout.as_ref().filter(|cut| self.cutout_frame_at_playhead(layer) == Some(cut.frame)) else { return Vec::new() };
        let Some(world) = world_transform(comp, layer, self.current_frame as f64) else { return Vec::new() };
        cut.clicks.iter().map(|click| (world.apply(Vec2::new(click.at.x * target.size.x, click.at.y * target.size.y)), click.keep)).collect()
    }

    pub fn clear_cutout(&mut self, layer: Id) {
        cutout::clear(self.composition_mut(), layer);
    }

    pub fn set_cutout_remove(&mut self, layer: Id, remove: bool) {
        cutout::set_remove(self.composition_mut(), layer, remove);
    }

    /// Records masks the background job made from `clicks`. This follows the user's clicks
    /// rather than being an edit of its own, so it is not an undo step.
    pub fn record_cutout_masks(&mut self, comp: Id, layer: Id, clicks: &[CutoutClick], folder: PathBuf, frames: (i64, i64)) -> bool {
        let Some(composition) = self.project.composition_mut(comp) else { return false };
        if !cutout::set_masks(composition, layer, clicks, folder.clone(), frames) {
            return false;
        }
        self.history.absorb(|project| {
            if let Some(composition) = project.composition_mut(comp) {
                cutout::set_masks(composition, layer, clicks, folder, frames);
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_land_on_the_layers_picture_where_they_were_made() {
        let mut state = EditorState::new();
        let item = studio_core::edit::import_image(&mut state.project, "/media/portrait.png".into(), Vec2::new(400.0, 200.0));
        let comp = state.active_comp;
        let layer = studio_core::edit::add_layer_from_item(&mut state.project, comp, item, 0, 0).unwrap();
        // The image is centered in the 1920 × 1080 frame: 760..1160 across, 440..640 down.
        assert!(state.add_cutout_click(layer, Vec2::new(860.0, 490.0), true));
        assert!(!state.add_cutout_click(layer, Vec2::new(100.0, 100.0), true), "off the picture");
        let cut = state.composition().layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!(cut.clicks, vec![CutoutClick { at: Vec2::new(0.25, 0.25), keep: true }]);
        let shown = state.cutout_clicks_on_screen(layer);
        assert!((shown[0].0 - Vec2::new(860.0, 490.0)).length() < 1e-3);
        assert_eq!(state.cutout_source(layer).unwrap().frames, None, "an image has one frame");
    }
}
