//! Snapping in the timeline. While a layer, a layer's end, a slipped source, keyframes or
//! the playhead is dragged, it is pulled onto the nearest frame of interest within reach:
//! the playhead, any layer's in or out point, any keyframe, the work area's ends, and the
//! composition's start and end. Snapping is on by default (S turns it off and on).
//!
//! How far "within reach" is depends on zoom, so the timeline sets `snap_threshold` (in
//! frames, for about 8 screen pixels) before each drag step. With it at 0, nothing snaps.

use studio_core::model::Id;
use studio_core::snap::snap;

use super::EditorState;

/// What a drag must not snap to: the things moving with it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SnapExclusions {
    /// A layer whose in and out points move with the drag.
    pub layer_edges: Option<Id>,
    /// A layer whose keyframes move with the drag.
    pub layer_keyframes: Option<Id>,
    /// The selected keyframes, when they are what is being dragged. Their whole properties
    /// are left out: landing on a neighbor there would put two keyframes on one frame.
    pub selected_keyframes: bool,
    /// The playhead, when it is what is being dragged.
    pub playhead: bool,
}

impl EditorState {
    /// Every frame a drag can snap to, sorted, leaving out what moves with the drag.
    fn snap_targets(&self, exclude: SnapExclusions) -> Vec<i64> {
        let composition = self.composition();
        let mut targets = vec![0, composition.duration_frames, self.work_area.start as i64, self.work_area.end as i64];
        // Markers (beats, say), so cuts and moves land on them.
        targets.extend(composition.markers.iter().map(|marker| marker.frame));
        if !exclude.playhead {
            targets.push(self.current_frame);
        }
        for layer in &composition.layers {
            if exclude.layer_edges != Some(layer.id) {
                targets.extend([layer.in_frame, layer.out_frame]);
            }
            if exclude.layer_keyframes == Some(layer.id) {
                continue;
            }
            for path in layer.property_paths() {
                let being_dragged = exclude.selected_keyframes && self.selected_keyframes.iter().any(|key| key.layer == layer.id && key.path == path);
                if being_dragged {
                    continue;
                }
                if let Some(property) = layer.property(path, 0) {
                    targets.extend(property.keyframes);
                }
            }
        }
        targets.sort_unstable();
        targets.dedup();
        targets
    }

    /// Adjusts a drag of `delta` frames so that one of `edges` (their frames before the drag)
    /// lands on a snap target, if one is within reach, and records where it snapped.
    pub(super) fn snapped_delta(&mut self, edges: &[i64], delta: i64, exclude: SnapExclusions) -> i64 {
        self.snap_line = None;
        if !self.snapping || self.snap_threshold <= 0 {
            return delta;
        }
        let moved: Vec<i64> = edges.iter().map(|edge| edge + delta).collect();
        match snap(&moved, &self.snap_targets(exclude), self.snap_threshold) {
            Some(found) => {
                self.snap_line = Some(found.at);
                delta + found.nudge
            }
            None => delta,
        }
    }

    /// Moves the playhead to `frame` while scrubbing, snapping it to layer edges and keyframes.
    pub fn scrub_to(&mut self, frame: i64) {
        let frame = frame + self.snapped_delta(&[frame], 0, SnapExclusions { playhead: true, ..Default::default() });
        self.set_current_frame(frame);
    }

    /// The frame the last drag step snapped to, for drawing a guide line; `None` if it did not snap.
    pub fn snap_line(&self) -> Option<i64> {
        self.snap_line
    }

    /// Hides the guide line once a drag ends.
    pub fn clear_snap_line(&mut self) {
        self.snap_line = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LayerTimeEdit;

    /// A fresh editor with snapping reaching 3 frames.
    fn state() -> EditorState {
        let mut state = EditorState::new();
        state.snap_threshold = 3;
        state
    }

    fn timing(state: &EditorState, layer: Id) -> (i64, i64) {
        let layer = state.composition().layer(layer).unwrap();
        (layer.in_frame, layer.out_frame)
    }

    #[test]
    fn moving_a_layer_snaps_its_edges_to_the_playhead() {
        let mut state = state();
        let title = state.composition().layers[0].id;
        let (in_frame, _) = timing(&state, title);
        state.current_frame = in_frame + 20;
        state.begin_layer_time_drag(title, LayerTimeEdit::Move);
        state.drag_layer_time(18); // 2 short of the playhead
        assert_eq!(timing(&state, title).0, in_frame + 20);
        assert_eq!(state.snap_line(), Some(in_frame + 20));
        state.drag_layer_time(30); // well past it: no snap
        assert_eq!(timing(&state, title).0, in_frame + 30);
        assert_eq!(state.snap_line(), None);
    }

    #[test]
    fn trimming_snaps_to_another_layers_edge() {
        let mut state = state();
        let layers: Vec<(Id, i64, i64)> = state.composition().layers.iter().map(|l| (l.id, l.in_frame, l.out_frame)).collect();
        // The title (24–168 in the sample) and the logo, which starts at 48.
        let (title, _, title_out) = layers[0];
        let logo_in = layers[2].1;
        state.current_frame = 0;
        state.begin_layer_time_drag(title, LayerTimeEdit::TrimOut);
        state.drag_layer_time(logo_in - title_out + 2);
        assert_eq!(timing(&state, title).1, logo_in);
    }

    #[test]
    fn keyframes_snap_but_not_to_themselves() {
        let mut state = state();
        let title = state.composition().layers[0].id;
        let opacity = studio_core::property::PropertyPath::Transform(studio_core::property::TransformProperty::Opacity);
        // The title's opacity keys are at 24, 36, 156 and 168; drag the one at 36 toward the playhead at 60.
        state.current_frame = 60;
        state.select_keyframe(crate::state::KeyframeRef { layer: title, path: opacity, frame: 36 }, false);
        state.begin_keyframe_drag();
        state.drag_keyframes(22);
        assert_eq!(state.selected_keyframes[0].frame, 60);
        // A tiny nudge does not stick to the keyframe's own starting place.
        state.drag_keyframes(1);
        assert_eq!(state.selected_keyframes[0].frame, 37);
    }

    #[test]
    fn the_playhead_snaps_to_layer_edges_while_scrubbing() {
        let mut state = state();
        let title_in = state.composition().layers[0].in_frame;
        state.scrub_to(title_in + 2);
        assert_eq!(state.current_frame, title_in);
        state.snapping = false;
        state.scrub_to(title_in + 2);
        assert_eq!(state.current_frame, title_in + 2, "with snapping off it goes exactly where dragged");
    }
}
