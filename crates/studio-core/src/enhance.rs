//! Edits to a layer's Enhance Speech settings (see `model::SpeechEnhance`). The cleaned copy
//! itself is made by the app with `studio-enhance`; these edits keep the settings and the record
//! of which copy is current in step, the way `cutout` does for masks.

use crate::model::{Composition, Id, LayerContent, SpeechEnhance};

/// Whether a layer has sound that can be enhanced: footage or audio with sound.
pub fn can_enhance(composition: &Composition, layer: Id) -> bool {
    composition.layer(layer).is_some_and(|layer| layer.has_audio && matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Audio { .. }))
}

/// Sets (or with `None`, removes) a layer's Enhance Speech. A cleaned copy already made is kept
/// when the new settings ask for the same sound (turning it off and on again, say); otherwise
/// it is out of date and dropped. Returns whether anything changed.
pub fn set_enhance(composition: &mut Composition, layer_id: Id, enhance: Option<SpeechEnhance>) -> bool {
    if !can_enhance(composition, layer_id) {
        return false;
    }
    let Some(layer) = composition.layer_mut(layer_id).filter(|layer| !layer.switches.locked) else { return false };
    let enhance = enhance.map(|mut enhance| {
        let kept = layer.audio.enhance.as_ref().filter(|old| old.same_sound(&enhance)).and_then(|old| old.file.clone());
        enhance.file = kept;
        enhance
    });
    if layer.audio.enhance == enhance {
        return false;
    }
    layer.audio.enhance = enhance;
    true
}

/// Records `file` as the cleaned copy made for `made_for`, if the layer still asks for that
/// sound (settings changed while it was being made make it stale). Returns whether it was used.
pub fn record_cleaned(composition: &mut Composition, layer_id: Id, made_for: &SpeechEnhance, file: std::path::PathBuf) -> bool {
    let Some(enhance) = composition.layer_mut(layer_id).and_then(|layer| layer.audio.enhance.as_mut()) else { return false };
    if !enhance.same_sound(made_for) {
        return false;
    }
    enhance.file = Some(file);
    true
}

/// The layers in `composition` that are set to be enhanced but have no cleaned copy yet.
pub fn waiting(composition: &Composition) -> Vec<(Id, SpeechEnhance)> {
    composition
        .layers
        .iter()
        .filter_map(|layer| layer.audio.enhance.as_ref().filter(|enhance| enhance.on && enhance.file.is_none()).map(|enhance| (layer.id, enhance.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EnhanceModel;
    use crate::sample::{ids, sample_project};

    /// A layer with sound added to the sample's main composition.
    fn with_sound() -> (crate::model::Project, Id) {
        let mut project = sample_project();
        let info = crate::edit::FootageInfo { size: crate::animation::Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: crate::time::FrameRate::new(24, 1), has_audio: true };
        let item = crate::edit::import_footage(&mut project, "/media/interview.mp4".into(), &info);
        let layer = crate::edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
        (project, layer)
    }

    #[test]
    fn the_cleaned_copy_follows_the_settings_and_plays_in_place_of_the_original() {
        let (mut project, layer) = with_sound();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let settings = SpeechEnhance::default();
        assert!(set_enhance(comp, layer, Some(settings.clone())));
        assert_eq!(waiting(comp), vec![(layer, settings.clone())]);
        assert!(record_cleaned(comp, layer, &settings, "/cache/clean.wav".into()));
        assert!(waiting(comp).is_empty());
        let heard = |project: &crate::model::Project| {
            let comp = project.composition(ids::MAIN_COMP).unwrap();
            crate::audio::composition_audio(project, comp).into_iter().find(|clip| clip.layer == layer).unwrap().path
        };
        assert_eq!(heard(&project), std::path::PathBuf::from("/cache/clean.wav"));

        // Off plays the original, and back on keeps the copy.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        set_enhance(comp, layer, Some(SpeechEnhance { on: false, ..settings.clone() }));
        assert_eq!(heard(&project), std::path::PathBuf::from("/media/interview.mp4"));
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        set_enhance(comp, layer, Some(settings.clone()));
        assert_eq!(heard(&project), std::path::PathBuf::from("/cache/clean.wav"));

        // Other settings need a new copy; one made for the old settings is not used.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        set_enhance(comp, layer, Some(SpeechEnhance { model: EnhanceModel::Light, ..settings.clone() }));
        assert_eq!(waiting(comp).len(), 1);
        assert!(!record_cleaned(comp, layer, &settings, "/cache/stale.wav".into()));
        assert_eq!(heard(&project), std::path::PathBuf::from("/media/interview.mp4"), "the original plays until the new copy is ready");
    }

    #[test]
    fn layers_without_sound_cannot_be_enhanced() {
        let mut project = sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let title = comp.layers.iter().find(|layer| layer.name == "Title").unwrap().id;
        assert!(!set_enhance(comp, title, Some(SpeechEnhance::default())));
    }
}
