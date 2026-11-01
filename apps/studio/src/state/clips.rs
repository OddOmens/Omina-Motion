//! Editing clips on tracks, for the timeline's Tracks view (see `studio_core::tracks`).
//! Dragging a clip moves it in time (snapping as usual) and, on release, onto the track under
//! the pointer; a clip that would overlap another there gets a new track of its own. With the
//! magnetic main track on, the main track's clips are kept packed after every edit.

use studio_core::model::Id;
use studio_core::tracks;

use super::EditorState;
use super::snap::SnapExclusions;

/// A clip being dragged in the Tracks view: where it started, and the track under the pointer.
#[derive(Clone, Copy, Debug)]
pub(super) struct ClipDrag {
    layer: Id,
    in_frame: i64,
    out_frame: i64,
    /// Frames moved so far (keyframes shift step by step).
    applied: i64,
    target_track: i32,
}

impl EditorState {
    /// Starts dragging a clip on its track.
    pub fn begin_clip_drag(&mut self, layer: Id) {
        let Some(found) = self.composition().layer(layer) else { return };
        if found.switches.locked {
            return;
        }
        self.clip_drag = Some(ClipDrag { layer, in_frame: found.in_frame, out_frame: found.out_frame, applied: 0, target_track: found.track });
    }

    /// Moves the dragged clip `delta` frames from where it started (snapping its ends), toward
    /// `track`, the track under the pointer. The track change happens on release.
    pub fn drag_clip(&mut self, delta: i64, track: i32) {
        let Some(drag) = self.clip_drag else { return };
        let exclude = SnapExclusions { layer_edges: Some(drag.layer), layer_keyframes: Some(drag.layer), ..Default::default() };
        // Not before frame 0, where a clip on a track would be partly unreachable.
        let delta = self.snapped_delta(&[drag.in_frame, drag.out_frame], delta, exclude).max(-drag.in_frame);
        let step = delta - drag.applied;
        if let Some(layer) = self.composition_mut().layer_mut(drag.layer)
            && step != 0
        {
            layer.shift_in_time(step);
        }
        for key in self.selected_keyframes.iter_mut().filter(|key| key.layer == drag.layer) {
            key.frame += step;
        }
        self.clip_drag = Some(ClipDrag { applied: delta, target_track: track, ..drag });
    }

    /// Drops the dragged clip onto its target track (or a new track next to it, if it would
    /// overlap a clip there).
    pub fn end_clip_drag(&mut self) {
        let Some(drag) = self.clip_drag.take() else { return };
        self.clear_snap_line();
        let Some(in_frame) = self.composition().layer(drag.layer).map(|layer| layer.in_frame) else { return };
        tracks::place_clip(self.composition_mut(), drag.layer, drag.target_track, in_frame);
    }

    /// The clip being dragged and the track it is headed for, for the view to draw it there.
    pub fn clip_drag_target(&self) -> Option<(Id, i32)> {
        self.clip_drag.map(|drag| (drag.layer, drag.target_track))
    }

    /// After a layer was moved in time without regard to tracks (the Layers view's drag), moves
    /// it to a track of its own if it now overlaps another clip on its track.
    pub(super) fn settle_on_track(&mut self, layer: Id) {
        let Some((track, in_frame)) = self.composition().layer(layer).map(|layer| (layer.track, layer.in_frame)) else { return };
        tracks::place_clip(self.composition_mut(), layer, track, in_frame);
    }

    /// Adds a project item as a clip on `track` at `frame` (a drop in the Tracks view): there
    /// if it fits, otherwise on a new track next to it. Selects it.
    pub fn add_clip_on_track(&mut self, item: Id, track: i32, frame: i64) {
        let comp = self.active_comp;
        let Some(layer) = studio_core::edit::add_layer_from_item(&mut self.project, comp, item, 0, frame) else { return };
        tracks::place_clip(self.composition_mut(), layer, track, frame.max(0));
        self.select_layer(layer, false);
    }

    /// Whether every clip on `track` has `switch` on (false for an empty track).
    pub fn track_switch(&self, track: i32, switch: super::Switch) -> bool {
        let mut clips = self.composition().layers.iter().filter(|layer| layer.track == track).peekable();
        clips.peek().is_some() && clips.all(|layer| switch.get(&layer.switches))
    }

    /// The track header's toggles (hide, mute, lock): turns `switch` off for every clip on
    /// `track` if all have it on, otherwise on for all of them.
    pub fn toggle_track_switch(&mut self, track: i32, switch: super::Switch) {
        let on = !self.track_switch(track, switch);
        for layer in self.composition_mut().layers.iter_mut().filter(|layer| layer.track == track) {
            *switch.slot(&mut layer.switches) = on;
        }
        // Locked clips can't stay selected (see `select_layer`).
        if switch == super::Switch::Locked && on {
            let composition = self.composition();
            let locked: Vec<Id> = composition.layers.iter().filter(|layer| layer.track == track).map(|layer| layer.id).collect();
            self.selected_layers.retain(|id| !locked.contains(id));
            self.selected_keyframes.retain(|key| !locked.contains(&key.layer));
        }
    }

    /// Keeps the main track packed if it is magnetic. Run after edits, outside drags.
    pub(super) fn apply_magnetism(&mut self) {
        if self.magnetic_main_track {
            tracks::compact_main_track(self.composition_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::tracks::MAIN_TRACK;

    fn timing(state: &EditorState, layer: Id) -> (i32, i64, i64) {
        let layer = state.composition().layer(layer).unwrap();
        (layer.track, layer.in_frame, layer.out_frame)
    }

    #[test]
    fn dragging_a_clip_onto_another_track_puts_both_on_one_line() {
        let mut state = EditorState::new();
        state.magnetic_main_track = false;
        // The title (24–168, track 5) dropped onto the main track after the background solid ends.
        let title = state.composition().layers[0].id;
        let solid = state.composition().layers.iter().find(|layer| layer.track == MAIN_TRACK).unwrap().id;
        state.composition_mut().layer_mut(solid).unwrap().out_frame = 20;
        state.begin_clip_drag(title);
        state.drag_clip(-4, MAIN_TRACK);
        state.end_clip_drag();
        assert_eq!(timing(&state, title).0, MAIN_TRACK);
        assert_eq!(timing(&state, title).1, 20);
        assert_eq!(timing(&state, solid).0, MAIN_TRACK, "two clips, one line");
    }

    #[test]
    fn a_clip_dropped_where_it_would_overlap_gets_a_new_track() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let before = studio_core::tracks::tracks(state.composition()).len();
        state.begin_clip_drag(title);
        state.drag_clip(0, MAIN_TRACK); // the background solid fills the main track
        state.end_clip_drag();
        assert_eq!(timing(&state, title).0, 1, "a new track just above the main track");
        assert_eq!(studio_core::tracks::tracks(state.composition()).len(), before, "its old track went away");
    }

    #[test]
    fn the_magnetic_main_track_closes_the_gap_after_a_delete() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let solid = state.composition().layers.iter().find(|layer| layer.track == MAIN_TRACK).unwrap().id;
        // Two clips on the main track: the solid (0–100) then the title (100–244).
        state.composition_mut().layer_mut(solid).unwrap().out_frame = 100;
        state.begin_clip_drag(title);
        state.drag_clip(100 - 24, MAIN_TRACK);
        state.end_clip_drag();
        state.record_history(false);
        assert_eq!(timing(&state, title), (MAIN_TRACK, 100, 244));
        state.select_layer(solid, false);
        state.delete_selection();
        state.record_history(false);
        assert_eq!(timing(&state, title).1, 0, "the title slid up to frame 0");
    }

    #[test]
    fn track_toggles_switch_every_clip_on_the_track() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let track = timing(&state, title).0;
        assert!(state.track_switch(track, super::super::Switch::Visible));
        state.select_layer(title, false);
        state.toggle_track_switch(track, super::super::Switch::Visible);
        assert!(!state.composition().layer(title).unwrap().switches.visible);
        state.toggle_track_switch(track, super::super::Switch::Locked);
        assert!(state.composition().layer(title).unwrap().switches.locked);
        assert!(state.selected_layers.is_empty(), "locking a track lets go of its clips");
    }

    #[test]
    fn dropping_an_item_on_a_track_places_it_there_when_it_fits() {
        let mut state = EditorState::new();
        state.magnetic_main_track = false;
        let solid = state.composition().layers.iter().find(|layer| layer.track == MAIN_TRACK).unwrap().id;
        state.composition_mut().layer_mut(solid).unwrap().out_frame = 50;
        state.add_clip_on_track(studio_core::sample::ids::GRAY_SOLID, MAIN_TRACK, 60);
        let added = state.selected_layers[0];
        assert_eq!(timing(&state, added).0, MAIN_TRACK);
        assert_eq!(timing(&state, added).1, 60);
    }
}
