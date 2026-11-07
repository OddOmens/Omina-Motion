//! Morphing from the editor: the Path stopwatch and keyframes (Motion tab), Morph To (a layer's
//! right-click menu, the Layer menu and the Motion tab), and Magic Move between compositions
//! (the Composition menu). See `studio_core::morph` and `studio_core::magic_move`.

use studio_core::animation::Easing;
use studio_core::model::Id;
use studio_core::magic_move::{self, MagicMove, MagicMoveError};
use studio_core::morph::{self, MorphError};

use super::EditorState;

/// The two layers a Morph To would use: the first selected shape morphs into the second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MorphPair {
    pub from: Id,
    pub to: Id,
}

impl EditorState {
    /// The selection as a Morph To pair: exactly two layers, in the order they were selected.
    /// Errors say why they can't morph (not shapes, the first one locked).
    pub fn morph_pair(&self) -> Result<MorphPair, MorphError> {
        let [from, to] = self.selected_layers[..] else { return Err(MorphError::NotShapes) };
        morph::can_morph(self.composition(), from, to)?;
        Ok(MorphPair { from, to })
    }

    /// Morph To: keyframes `pair.from` to turn into `pair.to` over `seconds` from the playhead
    /// (see `morph::morph_to`), then selects the morphing layer. Returns the frame it ends on.
    pub fn morph_layers(&mut self, pair: MorphPair, seconds: f64, easing: Easing) -> Result<i64, MorphError> {
        let fps = self.composition().frame_rate.fps();
        let frames = (seconds * fps).round().max(1.0) as i64;
        let start = self.current_frame;
        let end = morph::morph_to(self.composition_mut(), pair.from, pair.to, start, frames, easing)?;
        self.selected_layers = vec![pair.from];
        Ok(end)
    }

    /// The Path stopwatch on `layer` (see `morph::toggle_path_animation`).
    pub fn toggle_path_animation(&mut self, layer: Id) -> bool {
        let frame = self.current_frame;
        morph::toggle_path_animation(self.composition_mut(), layer, frame)
    }

    /// Adds or removes an outline keyframe on `layer` at the playhead.
    pub fn toggle_path_keyframe(&mut self, layer: Id) -> bool {
        let frame = self.current_frame;
        morph::toggle_path_keyframe(self.composition_mut(), layer, frame)
    }

    /// Moves the playhead to `layer`'s previous or next morph keyframe, if there is one.
    pub fn jump_to_morph_keyframe(&mut self, layer: Id, forward: bool) {
        let Some(frames) = self.composition().layer(layer).and_then(|layer| layer.morph.as_ref()).map(|morph| morph.keyframe_frames()) else { return };
        let current = self.current_frame;
        let target = if forward { frames.into_iter().find(|&frame| frame > current) } else { frames.into_iter().rev().find(|&frame| frame < current) };
        if let Some(frame) = target {
            self.set_current_frame(frame);
        }
    }

    /// Stops `layer`'s fill and stroke from morphing, keeping the look they have at the playhead.
    pub fn stop_style_morph(&mut self, layer: Id) -> bool {
        let frame = self.current_frame;
        morph::stop_style_morph(self.composition_mut(), layer, frame)
    }

    /// Turns `layer`, a drawn shape, into a pen path with the same outline.
    pub fn convert_to_path(&mut self, layer: Id) -> bool {
        morph::convert_to_path(self.composition_mut(), layer)
    }

    /// Magic Move: a new composition that plays the open one, turns it into `to` over `seconds`,
    /// then plays `to` (see `magic_move::magic_move`). The new composition opens.
    pub fn magic_move_to(&mut self, to: Id, seconds: f64, easing: Easing) -> Result<MagicMove, MagicMoveError> {
        let fps = self.composition().frame_rate.fps();
        let frames = (seconds * fps).round().max(1.0) as i64;
        let made = magic_move::magic_move(&mut self.project, self.active_comp, to, frames, easing)?;
        self.open_composition(made.composition);
        Ok(made)
    }
}
