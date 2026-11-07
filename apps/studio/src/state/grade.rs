//! Color grading and the eyedropper's colors.
//!
//! Grading works on the one selected clip. To grade the whole video, select an adjustment layer
//! over it and grade that: its corrections change everything under it, and as a layer like any
//! other it can be hidden, trimmed, masked or have its effects keyframed.
//!
//! The eyedropper keeps a current color, as Photoshop keeps a foreground color. Its recent picks
//! and the saved swatches are shared by every project and kept in Settings.

use studio_core::edit;
use studio_core::model::{Id, LayerContent, LutSource, Rgba};
use studio_core::property::PropertyPath;

use super::EditorState;

/// The most recent eyedropper picks kept.
pub const RECENT_COLORS: usize = 10;
/// The most saved swatches.
pub const MAX_SWATCHES: usize = 48;

/// Whether color correction changes how `content` looks (see `effects::suits`).
fn takes_grading(content: &LayerContent) -> bool {
    studio_core::effects::category_of("Basic Correction").is_some_and(|category| studio_core::effects::suits(category, content))
}

impl EditorState {
    /// Adds an adjustment layer over everything, from the playhead to the end, and selects it.
    pub fn add_adjustment_layer(&mut self) -> Option<Id> {
        let id = edit::add_adjustment_layer(&mut self.project, self.active_comp, self.current_frame)?;
        self.selected_layers = vec![id];
        self.selected_keyframes.clear();
        Some(id)
    }

    /// The layer the Color panel grades: the selected clip, when exactly one is selected and
    /// color correction suits it (an adjustment layer counts, and grades what is under it).
    pub fn grade_layer(&self) -> Option<Id> {
        let [id] = self.selected_layers.as_slice() else { return None };
        self.composition().layer(*id).filter(|layer| takes_grading(&layer.content)).map(|layer| layer.id)
    }

    /// The first effect called `name` on `layer`.
    pub fn effect_named(&self, layer: Id, name: &str) -> Option<Id> {
        self.composition().layer(layer)?.effects.iter().find(|effect| effect.name == name).map(|effect| effect.id)
    }

    /// Adds the catalog effect `name` to the graded clip (see `grade_layer`). Returns the layer
    /// and the new effect.
    pub fn add_grade_effect(&mut self, name: &str) -> Option<(Id, Id)> {
        let layer = self.grade_layer()?;
        if self.composition().layer(layer).is_some_and(|layer| layer.switches.locked) {
            return None;
        }
        let effect = edit::add_effect(&mut self.project, self.active_comp, layer, name)?;
        Some((layer, effect))
    }

    /// Adds an Apply LUT effect using `lut` to `layer`.
    pub fn add_lut(&mut self, layer: Id, lut: LutSource) -> Option<Id> {
        if self.composition().layer(layer).is_some_and(|layer| layer.switches.locked) {
            return None;
        }
        let effect = edit::add_lut(&mut self.project, self.active_comp, layer, lut)?;
        if !self.selected_layers.contains(&layer) {
            self.selected_layers = vec![layer];
        }
        Some(effect)
    }

    /// Chooses the lookup table an Apply LUT effect uses.
    pub fn set_effect_lut(&mut self, layer: Id, effect: Id, lut: Option<LutSource>) {
        if self.composition().layer(layer).is_some_and(|layer| !layer.switches.locked) {
            edit::set_effect_lut(self.composition_mut(), layer, effect, lut);
        }
    }

    /// The lookup table an effect uses, if it has one.
    pub fn effect_lut(&self, layer: Id, effect: Id) -> Option<&LutSource> {
        self.composition().layer(layer)?.effects.iter().find(|candidate| candidate.id == effect)?.lut.as_ref()
    }

    /// Changes a Basic Correction's Temperature and Tint so that `seen`, a color picked from the
    /// viewer as it shows now (straight sRGB), becomes neutral gray. The color already carries
    /// the current white balance, so the change is added to it.
    pub fn neutralize(&mut self, layer: Id, effect: Id, seen: Rgba) {
        let linear = [seen.r, seen.g, seen.b].map(srgb_to_linear);
        let (warmer, pinker) = studio_core::grade::neutralize(linear);
        let frame = self.current_frame;
        let value = |state: &Self, parameter| {
            state.composition().layer(layer).and_then(|layer| layer.property(PropertyPath::Effect { effect, parameter }, frame)).map(|snapshot| snapshot.value.component(0))
        };
        let (Some(temperature), Some(tint)) = (value(self, 0), value(self, 1)) else { return };
        self.set_property_component(layer, PropertyPath::Effect { effect, parameter: 0 }, 0, (temperature + warmer).clamp(-100.0, 100.0));
        self.set_property_component(layer, PropertyPath::Effect { effect, parameter: 1 }, 0, (tint + pinker).clamp(-100.0, 100.0));
    }

    /// Sets a Keyer's key color (its first three parameters, 0–255) to `seen`, a color picked
    /// from the viewer (straight sRGB). Animated channels get a keyframe at the playhead.
    pub fn set_key_color(&mut self, layer: Id, effect: Id, seen: Rgba) {
        for (parameter, channel) in [seen.r, seen.g, seen.b].into_iter().enumerate() {
            let value = (channel.clamp(0.0, 1.0) * 255.0).round();
            self.set_property_component(layer, PropertyPath::Effect { effect, parameter }, 0, value);
        }
    }

    /// A Keyer's key color at the playhead, as a display color for its swatch.
    pub fn key_color(&self, layer: Id, effect: Id) -> Option<Rgba> {
        let layer = self.composition().layer(layer)?;
        let channel = |parameter| layer.property(PropertyPath::Effect { effect, parameter }, self.current_frame).map(|snapshot| snapshot.value.component(0) / 255.0);
        Some(Rgba { r: channel(0)?, g: channel(1)?, b: channel(2)?, ..Rgba::hex(0) })
    }

    // The eyedropper's colors -------------------------------------------------

    /// Makes `color` the current color and puts it first among the recent picks.
    pub fn pick_color(&mut self, color: Rgba) {
        let color = color.linked(None);
        self.colors.current = color;
        self.colors.recent.retain(|old| !old.same_look(color));
        self.colors.recent.insert(0, color);
        self.colors.recent.truncate(RECENT_COLORS);
        self.save_colors();
    }

    /// Saves `color` as a swatch, unless an identical one is saved already. Returns whether it
    /// was added.
    pub fn save_swatch(&mut self, color: Rgba) -> bool {
        let color = color.linked(None);
        if self.colors.swatches.iter().any(|old| old.same_look(color)) || self.colors.swatches.len() >= MAX_SWATCHES {
            return false;
        }
        self.colors.swatches.push(color);
        self.save_colors();
        true
    }

    pub fn remove_swatch(&mut self, index: usize) {
        if index < self.colors.swatches.len() {
            self.colors.swatches.remove(index);
            self.save_colors();
        }
    }

    /// Keeps the colors for next time. A failure only loses the colors, and `app_settings`
    /// reports it.
    fn save_colors(&self) {
        let colors = self.colors.clone();
        crate::app_settings::update(|settings| settings.colors = colors);
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.04045 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::ColorTarget;

    /// Layers of the sample project: its title text and its first video.
    const TITLE: Id = Id(100);
    const A_CAM: Id = Id(104);

    fn effect_value(state: &EditorState, layer: Id, effect: Id, parameter: usize) -> f32 {
        state.composition().layer(layer).unwrap().property(PropertyPath::Effect { effect, parameter }, state.current_frame).unwrap().value.component(0)
    }

    #[test]
    fn an_adjustment_layer_is_graded_like_a_clip() {
        let mut state = EditorState::new();
        let adjustment = state.add_adjustment_layer().unwrap();
        assert!(matches!(state.composition().layer(adjustment).unwrap().content, LayerContent::Adjustment));
        assert_eq!(state.grade_layer(), Some(adjustment), "it was selected when made");
        let (layer, effect) = state.add_grade_effect("Basic Correction").unwrap();
        assert_eq!(layer, adjustment);
        assert_eq!(state.effect_named(layer, "Basic Correction"), Some(effect));
    }

    #[test]
    fn a_clip_is_graded_only_when_one_picture_clip_is_selected() {
        let mut state = EditorState::new();
        assert_eq!(state.grade_layer(), None, "nothing selected");
        state.selected_layers = vec![TITLE];
        assert_eq!(state.grade_layer(), None, "text is colored directly, not graded");
        state.selected_layers = vec![A_CAM];
        assert_eq!(state.grade_layer(), Some(A_CAM));
        let (_, effect) = state.add_grade_effect("Basic Correction").unwrap();
        assert!(state.composition().layer(A_CAM).unwrap().effects.iter().any(|e| e.id == effect));
    }

    #[test]
    fn luts_are_added_and_changed() {
        let mut state = EditorState::new();
        let layer = A_CAM;
        let effect = state.add_lut(layer, LutSource::Look("Vivid".into())).unwrap();
        assert_eq!(state.effect_lut(layer, effect), Some(&LutSource::Look("Vivid".into())));
        state.set_effect_lut(layer, effect, Some(LutSource::File("/luts/Film.cube".into())));
        assert_eq!(state.effect_lut(layer, effect), Some(&LutSource::File("/luts/Film.cube".into())));
        assert_eq!(state.selected_layers, vec![layer]);
    }

    #[test]
    fn picking_a_gray_card_neutralizes_the_white_balance() {
        let mut state = EditorState::new();
        let layer = A_CAM;
        let (_, effect) = {
            state.selected_layers = vec![layer];
            state.add_grade_effect("Basic Correction").unwrap()
        };
        // A bluish gray needs warming.
        state.neutralize(layer, effect, Rgba::hex(0x8090b0));
        assert!(effect_value(&state, layer, effect, 0) > 5.0);
        let first = (effect_value(&state, layer, effect, 0), effect_value(&state, layer, effect, 1));
        // Picking a gray that is already neutral changes nothing more.
        state.neutralize(layer, effect, Rgba::hex(0x808080));
        let second = (effect_value(&state, layer, effect, 0), effect_value(&state, layer, effect, 1));
        assert!((first.0 - second.0).abs() < 0.01 && (first.1 - second.1).abs() < 0.01);
    }

    #[test]
    fn picked_colors_are_remembered_newest_first_without_repeats() {
        let mut state = EditorState::new();
        let (red, blue) = (Rgba::hex(0xff0000), Rgba::hex(0x0000ff));
        state.pick_color(red);
        state.pick_color(blue);
        state.pick_color(red);
        assert_eq!(state.colors.current, red);
        assert_eq!(state.colors.recent, vec![red, blue]);
        for value in 0..20 {
            state.pick_color(Rgba::hex(value));
        }
        assert_eq!(state.colors.recent.len(), RECENT_COLORS);
        // They are kept in Settings for next time.
        assert_eq!(crate::app_settings::load().colors, state.colors);
    }

    #[test]
    fn swatches_are_saved_once_and_can_be_removed() {
        let mut state = EditorState::new();
        let green = Rgba::hex(0x30a46c);
        assert!(state.save_swatch(green));
        assert!(!state.save_swatch(green), "already saved");
        assert_eq!(state.colors.swatches, vec![green]);
        state.remove_swatch(5);
        state.remove_swatch(0);
        assert!(state.colors.swatches.is_empty());
    }

    #[test]
    fn the_current_color_goes_on_shapes_text_and_solids() {
        let mut state = EditorState::new();
        let pink = Rgba::hex(0xe93d82);
        state.set_color(ColorTarget::Current, pink);
        assert_eq!(state.color_of(ColorTarget::Current), Some(pink));
        state.selected_layers = vec![TITLE, A_CAM];
        assert!(state.selection_takes_color());
        assert_eq!(state.apply_color_to_selection(state.colors.current), 1, "the video takes no color");
        assert_eq!(state.text_style_of(TITLE).unwrap().fill, Some(pink));
        state.selected_layers = vec![A_CAM];
        assert!(!state.selection_takes_color());
    }

    #[test]
    fn adjustment_layers_start_at_the_playhead_and_cover_the_frame() {
        let mut state = EditorState::new();
        state.set_current_frame(12);
        let id = state.add_adjustment_layer().unwrap();
        let comp = state.composition();
        let layer = comp.layer(id).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame, layer.size), (12, comp.duration_frames, comp.size()));
        assert_eq!(state.selected_layers, vec![id]);
    }
}
