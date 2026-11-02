//! Rendering one layer on its own (Quick Export Layer), and putting the render in its place
//! (Export and Replace). Baking a heavy layer, such as footage with its background removed or
//! a stack of effects, into a plain clip means playback just reads the file.

use std::ops::Range;

use crate::model::{Id, Project, Rgba, TrackMatte};

/// A copy of the project in which composition `comp` shows only `layer`, over a transparent
/// background, and the frames the layer is on screen. The layer is soloed, so whatever shapes
/// it (its parent, track matte or mask) still applies, while every other layer stays out of
/// the picture and the sound.
pub fn solo_copy(project: &Project, comp: Id, layer: Id) -> Option<(Project, Range<i64>)> {
    let mut copy = project.clone();
    let composition = copy.composition_mut(comp)?;
    let target = composition.layer(layer)?;
    let frames = target.in_frame.max(0)..target.out_frame.min(composition.duration_frames);
    if frames.is_empty() {
        return None;
    }
    composition.background = Rgba::TRANSPARENT;
    // Its own switches still decide whether it is seen and heard, as in the composition.
    for other in &mut composition.layers {
        other.switches.solo = other.id == layer;
    }
    Some((copy, frames))
}

/// Whether another layer leans on `layer` (as its parent, mask, text path or ducking voice), so
/// replacing it with a render would change that layer too.
pub fn is_depended_on(project: &Project, comp: Id, layer: Id) -> bool {
    project.composition(comp).is_some_and(|composition| {
        composition.layers.iter().any(|other| {
            other.parent == Some(layer)
                || other.mask.is_some_and(|mask| mask.layer == layer)
                || other.text_path.is_some_and(|path| path.layer == layer)
                || other.audio.duck_under == Some(layer)
        })
    })
}

/// Replaces `layer` with a clip of the rendered movie `item` (made by `solo_copy`), on the same
/// track at the same time, under the same name. The render already includes its effects,
/// animation, matte and mask, so the clip has none of its own, and a track matte layer it used
/// is turned off. Returns the new layer.
pub fn replace_with_render(project: &mut Project, comp: Id, layer: Id, item: Id) -> Option<Id> {
    if is_depended_on(project, comp, layer) {
        return None;
    }
    let composition = project.composition(comp)?;
    let index = composition.layers.iter().position(|candidate| candidate.id == layer)?;
    let old = composition.layers[index].clone();
    let matte_source = (old.track_matte != TrackMatte::None && index > 0).then(|| composition.layers[index - 1].id);
    let start = old.in_frame.max(0);
    let new = crate::edit::add_layer_from_item(project, comp, item, index, start)?;
    let composition = project.composition_mut(comp)?;
    crate::edit::delete_layers(composition, &[layer]);
    if let Some(clip) = composition.layer_mut(new) {
        clip.name = old.name.clone();
        clip.label = old.label;
        clip.track = old.track;
        clip.out_frame = clip.out_frame.min(old.out_frame);
    }
    if let Some(matte) = matte_source.and_then(|matte| composition.layer_mut(matte)) {
        matte.switches.visible = false;
    }
    crate::tracks::normalize(composition);
    Some(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    #[test]
    fn a_solo_copy_shows_only_the_layer_over_transparency() {
        let project = sample_project();
        let title = project.composition(ids::MAIN_COMP).unwrap().layers.iter().find(|layer| layer.name == "Title").unwrap().clone();
        let (copy, frames) = solo_copy(&project, ids::MAIN_COMP, title.id).unwrap();
        let composition = copy.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(composition.background, Rgba::TRANSPARENT);
        assert_eq!(frames, title.in_frame..title.out_frame.min(composition.duration_frames));
        assert!(composition.layers.iter().all(|layer| layer.switches.solo == (layer.id == title.id)));
        // The original is untouched.
        assert!(project.composition(ids::MAIN_COMP).unwrap().layers.iter().all(|layer| !layer.switches.solo));
    }

    #[test]
    fn the_render_takes_the_layers_place() {
        let mut project = sample_project();
        let title = project.composition(ids::MAIN_COMP).unwrap().layers.iter().find(|layer| layer.name == "Title").unwrap().clone();
        let info = crate::edit::FootageInfo { size: crate::animation::Vec2::new(1920.0, 1080.0), duration_seconds: 10.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: false };
        let item = crate::edit::import_footage(&mut project, "/renders/Title.mov".into(), &info);
        let clip = replace_with_render(&mut project, ids::MAIN_COMP, title.id, item).unwrap();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        assert!(composition.layer(title.id).is_none());
        let clip = composition.layer(clip).unwrap();
        assert_eq!((clip.name.as_str(), clip.in_frame, clip.out_frame), ("Title", title.in_frame, title.out_frame));
        assert_eq!(clip.track, title.track);
    }
}
