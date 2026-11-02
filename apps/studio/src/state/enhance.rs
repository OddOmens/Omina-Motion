//! Enhance Speech on the open composition's layers (see `studio_core::enhance`); the background
//! job that makes the cleaned copies is in `panels/enhance.rs`.

use studio_core::enhance;
use studio_core::model::{Id, SpeechEnhance};

use super::EditorState;

impl EditorState {
    /// A layer's Enhance Speech settings, if it has them.
    pub fn enhance_of(&self, layer: Id) -> Option<&SpeechEnhance> {
        self.composition().layer(layer)?.audio.enhance.as_ref()
    }

    /// Whether a layer has sound that can be enhanced.
    pub fn can_enhance(&self, layer: Id) -> bool {
        enhance::can_enhance(self.composition(), layer)
    }

    /// Sets or removes a layer's Enhance Speech (an undoable edit).
    pub fn set_enhance(&mut self, layer: Id, settings: Option<SpeechEnhance>) -> bool {
        enhance::set_enhance(self.composition_mut(), layer, settings)
    }

    /// Changes one part of a layer's Enhance Speech settings.
    pub fn edit_enhance(&mut self, layer: Id, change: impl FnOnce(&mut SpeechEnhance)) -> bool {
        let Some(mut settings) = self.enhance_of(layer).cloned() else { return false };
        change(&mut settings);
        self.set_enhance(layer, Some(settings))
    }

    /// Records a finished cleaned copy, made for `made_for`, on `layer` of composition `comp`. It
    /// is the app's own work, not an edit, so it is not an undo step.
    pub fn record_cleaned(&mut self, comp: Id, layer: Id, made_for: &SpeechEnhance, file: std::path::PathBuf) -> bool {
        self.settle_drags();
        let Some(composition) = self.project.composition_mut(comp) else { return false };
        if !enhance::record_cleaned(composition, layer, made_for, file.clone()) {
            return false;
        }
        self.history.absorb_everywhere(|project| {
            if let Some(composition) = project.composition_mut(comp) {
                enhance::record_cleaned(composition, layer, made_for, file.clone());
            }
        });
        true
    }

    /// Forgets cleaned copies whose file has gone (the cache was cleared or moved), so they are
    /// made again rather than playing nothing. Not an undo step.
    pub fn forget_missing_cleaned(&mut self) -> bool {
        let comp = self.active_comp;
        let missing: Vec<Id> = self
            .composition()
            .layers
            .iter()
            .filter(|layer| layer.audio.enhance.as_ref().and_then(|enhance| enhance.file.as_ref()).is_some_and(|file| !file.is_file()))
            .map(|layer| layer.id)
            .collect();
        let forget = |project: &mut studio_core::model::Project| {
            for &layer in &missing {
                // Only a copy that really is gone: an undo step may point at another one.
                if let Some(enhance) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer)).and_then(|layer| layer.audio.enhance.as_mut())
                    && enhance.file.as_ref().is_some_and(|file| !file.is_file())
                {
                    enhance.file = None;
                }
            }
        };
        if !missing.is_empty() {
            self.settle_drags();
        }
        forget(&mut self.project);
        self.history.absorb_everywhere(forget);
        !missing.is_empty()
    }

    /// The layers of the open composition waiting for a cleaned copy, and what each asks for.
    pub fn waiting_for_enhance(&self) -> Vec<(Id, SpeechEnhance)> {
        enhance::waiting(self.composition())
    }

    /// The sound file a layer's sound comes from (not a cleaned copy).
    pub fn sound_source(&self, layer: Id) -> Option<std::path::PathBuf> {
        let item = match self.composition().layer(layer)?.content {
            studio_core::model::LayerContent::Footage { item } | studio_core::model::LayerContent::Audio { item } => item,
            _ => return None,
        };
        self.project.item(item)?.path.clone()
    }
}
