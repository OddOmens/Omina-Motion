//! Edits to a layer's background removal (see `model::Cutout`). The masks themselves are made
//! by the app with the `studio-cutout` model; these edits keep the clicks and the record of
//! which masks are current in step.

use crate::model::{Composition, Cutout, CutoutClick, Id, Layer, LayerContent, MAX_CUTOUT_CLICKS};

/// Whether a layer's picture can have its background removed: footage and images.
pub fn can_cut_out(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Image { .. })
}

/// Adds a click on source frame `frame`. Clicks belong to one frame: a click on another frame
/// starts the pick again there. Masks made from the old clicks are no longer current. Returns
/// whether the click was added (it must be on the picture, and there is a limit).
pub fn add_click(composition: &mut Composition, layer_id: Id, click: CutoutClick, frame: i64) -> bool {
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| can_cut_out(layer) && !layer.switches.locked) else { return false };
    let on_picture = (0.0..=1.0).contains(&click.at.x) && (0.0..=1.0).contains(&click.at.y);
    if !on_picture {
        return false;
    }
    let cutout = layer.cutout.get_or_insert_with(|| Cutout { frame, clicks: Vec::new(), remove: false, masks: None, frames: None });
    if cutout.frame != frame {
        cutout.frame = frame;
        cutout.clicks.clear();
    }
    if cutout.clicks.len() >= MAX_CUTOUT_CLICKS {
        return false;
    }
    cutout.clicks.push(click);
    cutout.frames = None;
    true
}

/// Removes a layer's background removal, showing its whole picture again.
pub fn clear(composition: &mut Composition, layer_id: Id) {
    if let Some(layer) = composition.layer_mut(layer_id) {
        layer.cutout = None;
    }
}

/// Keeps what was picked (`remove` false) or removes it instead.
pub fn set_remove(composition: &mut Composition, layer_id: Id, remove: bool) {
    if let Some(cutout) = composition.layer_mut(layer_id).and_then(|layer| layer.cutout.as_mut()) {
        cutout.remove = remove;
    }
}

/// Records masks made for source frames `frames` in `folder`, if they were made from the
/// clicks the layer still has (`clicks`): clicks changed while the masks were being made make
/// them stale, and they are not used. Returns whether they were recorded.
pub fn set_masks(composition: &mut Composition, layer_id: Id, clicks: &[CutoutClick], folder: std::path::PathBuf, frames: (i64, i64)) -> bool {
    let Some(cutout) = composition.layer_mut(layer_id).and_then(|layer| layer.cutout.as_mut()) else { return false };
    if cutout.clicks != clicks || frames.1 <= frames.0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::model::{LayerSwitches, Project};
    use crate::sample::{self, ids};

    fn footage_layer(project: &mut Project) -> Id {
        let info = crate::edit::FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: false };
        let item = crate::edit::import_footage(project, "/media/dancer.mp4".into(), &info);
        crate::edit::add_layer_from_item(project, ids::MAIN_COMP, item, 0, 0).unwrap()
    }

    fn click(x: f32, keep: bool) -> CutoutClick {
        CutoutClick { at: Vec2::new(x, 0.5), keep }
    }

    #[test]
    fn clicks_belong_to_one_frame_and_only_land_on_the_picture() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert!(add_click(comp, layer, click(0.5, true), 10));
        assert!(add_click(comp, layer, click(0.2, false), 10));
        assert!(!add_click(comp, layer, click(1.5, true), 10), "off the picture");
        assert_eq!(comp.layer(layer).unwrap().cutout.as_ref().unwrap().clicks.len(), 2);
        assert!(add_click(comp, layer, click(0.7, true), 40));
        let cutout = comp.layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!((cutout.frame, cutout.clicks.len()), (40, 1), "a click on another frame starts over");
        // Only footage and images can be cut out.
        let solid = comp.layers.iter().find(|layer| matches!(layer.content, LayerContent::Solid { .. })).unwrap().id;
        assert!(!add_click(comp, solid, click(0.5, true), 0));
        comp.layer_mut(layer).unwrap().switches = LayerSwitches { locked: true, ..LayerSwitches::default() };
        assert!(!add_click(comp, layer, click(0.5, true), 40), "locked layers stay as they are");
    }

    #[test]
    fn masks_are_kept_only_for_the_clicks_they_were_made_from_and_grow_together() {
        let mut project = sample::sample_project();
        let layer = footage_layer(&mut project);
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        add_click(comp, layer, click(0.5, true), 10);
        let clicks = comp.layer(layer).unwrap().cutout.as_ref().unwrap().clicks.clone();
        assert!(set_masks(comp, layer, &clicks, "/cache/a".into(), (10, 11)));
        assert!(set_masks(comp, layer, &clicks, "/cache/a".into(), (11, 60)), "tracking adds to the preview frame");
        let cutout = comp.layer(layer).unwrap().cutout.clone().unwrap();
        assert_eq!(cutout.frames, Some((10, 60)));
        assert_eq!(cutout.mask_path(5), Some("/cache/a/10.png".into()), "before the first mask, the first is used");
        assert_eq!(cutout.mask_path(99), Some("/cache/a/59.png".into()));
        // A new click while masks were being made: those masks are stale.
        add_click(comp, layer, click(0.3, false), 10);
        assert!(!set_masks(comp, layer, &clicks, "/cache/a".into(), (10, 60)));
        assert_eq!(comp.layer(layer).unwrap().cutout.as_ref().unwrap().frames, None);
    }
}
