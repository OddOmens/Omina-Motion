//! Copy, paste and duplicate for layers (Cmd+C, Cmd+V, Cmd+D). The clipboard lives in the
//! editor, so copied layers keep every setting, keyframe and effect.

use studio_core::copy;
use studio_core::model::Id;

use super::EditorState;

impl EditorState {
    /// Copies the selected layers. Returns how many were copied (a track matte they use comes along).
    pub fn copy_selection(&mut self) -> usize {
        if self.selected_layers.is_empty() {
            return 0;
        }
        self.clipboard = copy::layers_to_copy(self.composition(), &self.selected_layers);
        self.clipboard.len()
    }

    /// Cut (Cmd+X): copies the selected layers, then removes them. Returns how many were cut.
    pub fn cut_selection(&mut self) -> usize {
        let copied = self.copy_selection();
        if copied > 0 {
            // Selected keyframes would be deleted in place of the layers.
            self.selected_keyframes.clear();
            self.delete_selection();
        }
        copied
    }

    /// Select All (Cmd+A): selects every layer in the open composition.
    pub fn select_all_layers(&mut self) {
        self.selected_layers = self.composition().layers.iter().map(|layer| layer.id).collect();
        self.selected_keyframes.clear();
    }

    /// Pastes the copied layers at the playhead, each on a new track, and selects them.
    pub fn paste(&mut self) -> Vec<Id> {
        let (comp, frame) = (self.active_comp, self.current_frame);
        let pasted = copy::paste_layers(&mut self.project, comp, &self.clipboard, frame);
        self.select_new_layers(&pasted);
        pasted
    }

    /// Duplicates the selected layers just above themselves and selects the copies.
    pub fn duplicate_selection(&mut self) -> Vec<Id> {
        let comp = self.active_comp;
        let selected = self.selected_layers.clone();
        let copies = copy::duplicate_layers(&mut self.project, comp, &selected);
        self.select_new_layers(&copies);
        copies
    }

    /// The name Pre-compose suggests, as After Effects does: the top selected layer's name and
    /// "Comp", numbered past any composition already called that.
    pub fn precompose_name(&self) -> String {
        let composition = self.composition();
        let base = composition.layers.iter().find(|layer| self.selected_layers.contains(&layer.id)).map_or("Pre-comp", |layer| layer.name.as_str());
        let taken = |name: &str| self.project.items.iter().any(|item| item.name == name);
        (1..).map(|number| format!("{base} Comp {number}")).find(|name| !taken(name)).unwrap_or_else(|| format!("{base} Comp"))
    }

    /// Pre-compose: moves the selected layers into a new composition named `name` (with `trim`,
    /// only as long as they span), in their place, and selects the layer that shows it.
    pub fn precompose_selection(&mut self, name: &str, trim: bool) -> Option<Id> {
        if self.selected_layers.is_empty() {
            return None;
        }
        let comp = self.active_comp;
        let name = if name.trim().is_empty() { self.precompose_name() } else { name.trim().to_owned() };
        let selected = self.selected_layers.clone();
        let layer = studio_core::edit::precompose(&mut self.project, comp, &selected, &name, trim)?;
        self.select_new_layers(&[layer]);
        Some(layer)
    }

    /// Ungroups a nested composition's layer: its layers come back into this composition in
    /// its place (the composition stays in the project) and are selected.
    pub fn ungroup_layer(&mut self, layer: Id) -> Result<studio_core::nesting::Ungrouped, studio_core::nesting::UngroupError> {
        let comp = self.active_comp;
        let result = studio_core::nesting::ungroup(&mut self.project, comp, layer)?;
        self.select_new_layers(&result.layers);
        Ok(result)
    }

    /// Mirrors the selected layers left to right (`horizontal`) or top to bottom. Returns
    /// whether any flipped.
    pub fn flip_selection(&mut self, horizontal: bool) -> bool {
        let selected = self.selected_layers.clone();
        let mut flipped = false;
        for layer in selected {
            flipped |= studio_core::edit::flip_layer(self.composition_mut(), layer, horizontal);
        }
        flipped
    }

    /// Resets the transform of every selected layer (see `reset_transform`).
    pub fn reset_selection_transform(&mut self) {
        for layer in self.selected_layers.clone() {
            self.reset_transform(layer);
        }
    }

    /// The composition a layer shows, if it is a nested composition.
    pub fn nested_composition(&self, layer: Id) -> Option<Id> {
        match self.composition().layer(layer)?.content {
            studio_core::model::LayerContent::Composition { composition } => Some(composition),
            _ => None,
        }
    }

    fn select_new_layers(&mut self, layers: &[Id]) {
        if !layers.is_empty() {
            self.selected_layers = layers.to_vec();
            self.selected_keyframes.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_layers_paste_at_the_playhead_and_undo_in_one_step() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        assert_eq!(state.copy_selection(), 1);
        state.current_frame = 50;
        let count = state.composition().layers.len();
        let pasted = state.paste();
        state.record_history(false);
        assert_eq!(state.composition().layers.len(), count + 1);
        assert_eq!(state.composition().layer(pasted[0]).unwrap().in_frame, 50);
        assert_eq!(state.selected_layers, pasted);
        state.undo();
        assert_eq!(state.composition().layers.len(), count);
    }

    #[test]
    fn duplicate_selects_the_copy_and_needs_a_selection() {
        let mut state = EditorState::new();
        assert!(state.duplicate_selection().is_empty(), "nothing selected, nothing duplicated");
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        let copies = state.duplicate_selection();
        assert_eq!(state.selected_layers, copies);
        assert_ne!(copies[0], title);
    }
}
