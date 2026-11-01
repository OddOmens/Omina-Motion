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
