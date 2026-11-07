//! Stabilize from the editor: applying the measured camera motion to a video layer, changing
//! its smoothness, and taking it off again. The motion is measured in the background
//! (`panels::analysis::stabilize`); the steadying itself is `studio_core::stabilize`.

use studio_core::model::{Id, LayerContent};
use studio_core::stabilize::{self, MeasuredMotion, StabilizeError, Stabilized};
use studio_core::time::FrameRate;

use super::EditorState;

impl EditorState {
    /// The frame rate of the video file behind footage layer `layer` of composition `comp`.
    fn footage_rate(&self, comp: Id, layer: Id) -> Option<FrameRate> {
        let found = self.project.composition(comp)?.layer(layer)?;
        let LayerContent::Footage { item } = found.content else { return None };
        self.project.item(item)?.frame_rate
    }

    /// Steadies footage layer `layer` of composition `comp` against `motion` (measured on its
    /// source), with `smoothness` in percent.
    pub fn stabilize_layer(&mut self, comp: Id, layer: Id, motion: MeasuredMotion, smoothness: f32) -> Result<Stabilized, StabilizeError> {
        let rate = self.footage_rate(comp, layer).ok_or(StabilizeError::NotFootage)?;
        let composition = self.project.composition_mut(comp).ok_or(StabilizeError::NotFootage)?;
        stabilize::stabilize(composition, layer, motion, smoothness, rate)
    }

    /// Re-steadies a stabilized layer of the open composition with a new `smoothness`, from the
    /// motion already measured. `Ok(None)` if the layer isn't stabilized.
    pub fn set_stabilize_smoothness(&mut self, layer: Id, smoothness: f32) -> Result<Option<Stabilized>, StabilizeError> {
        let comp = self.active_comp;
        let rate = self.footage_rate(comp, layer).ok_or(StabilizeError::NotFootage)?;
        stabilize::set_smoothness(self.composition_mut(), layer, smoothness, rate)
    }

    /// Takes stabilizing off a layer of the open composition, putting its transform back as it
    /// was. Returns whether it was stabilized.
    pub fn remove_stabilization(&mut self, layer: Id) -> bool {
        stabilize::remove(self.composition_mut(), layer)
    }
}
