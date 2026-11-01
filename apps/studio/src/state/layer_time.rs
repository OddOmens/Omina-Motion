//! Changing layers in time from the timeline: dragging a layer bar moves it, dragging either
//! end trims it, and Alt-dragging the bar slips its source (see `studio_core::trim`).
//! Also Alt+[ and Alt+], which trim the selected layers to the playhead as in After Effects,
//! splitting at the playhead (⌘⇧D) and ripple delete (⇧Delete).

use studio_core::cut::{self, SplitReport};
use studio_core::model::Id;
use studio_core::trim;

use super::EditorState;
use super::snap::SnapExclusions;

/// What dragging a layer bar does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerTimeEdit {
    /// The whole layer moves, keyframes included.
    Move,
    /// The left end: the in point.
    TrimIn,
    /// The right end: the out point.
    TrimOut,
    /// The source moves under fixed in and out points.
    Slip,
}

/// A layer bar being dragged: which layer and edit, and its timing when the drag began.
#[derive(Clone, Copy, Debug)]
pub(super) struct LayerTimeDrag {
    layer: Id,
    edit: LayerTimeEdit,
    /// Frames already moved, for `Move` (which shifts keyframes step by step).
    applied: i64,
    start_frame: i64,
    in_frame: i64,
    out_frame: i64,
}

impl EditorState {
    /// Starts dragging a layer bar (or one of its ends) in time.
    pub fn begin_layer_time_drag(&mut self, layer: Id, edit: LayerTimeEdit) {
        let Some(found) = self.composition().layer(layer) else { return };
        self.layer_time_drag = Some(LayerTimeDrag {
            layer,
            edit,
            applied: 0,
            start_frame: found.start_frame,
            in_frame: found.in_frame,
            out_frame: found.out_frame,
        });
    }

    pub fn end_layer_time_drag(&mut self) {
        let finished = self.layer_time_drag.take();
        self.clear_snap_line();
        // A layer moved onto a busy part of its track moves up to a track of its own.
        if let Some(drag) = finished.filter(|drag| drag.edit == LayerTimeEdit::Move) {
            self.settle_on_track(drag.layer);
        }
    }

    /// Applies the drag `delta` frames from where it started, snapping what moves (both ends
    /// when moving, one end when trimming, the source's ends when slipping).
    pub fn drag_layer_time(&mut self, delta: i64) {
        let Some(drag) = self.layer_time_drag else { return };
        let comp = self.active_comp;
        let own = Some(drag.layer);
        let (edges, exclude) = match drag.edit {
            LayerTimeEdit::Move => {
                (vec![drag.in_frame, drag.out_frame], SnapExclusions { layer_edges: own, layer_keyframes: own, ..Default::default() })
            }
            LayerTimeEdit::TrimIn => (vec![drag.in_frame], SnapExclusions { layer_edges: own, ..Default::default() }),
            LayerTimeEdit::TrimOut => (vec![drag.out_frame], SnapExclusions { layer_edges: own, ..Default::default() }),
            // The source's ends may line up with the clip's own in and out points.
            LayerTimeEdit::Slip => {
                let end = self.source_span(drag.layer).map_or(drag.start_frame, |(start, end)| end - start + drag.start_frame);
                (vec![drag.start_frame, end], SnapExclusions::default())
            }
        };
        let delta = self.snapped_delta(&edges, delta, exclude);
        match drag.edit {
            LayerTimeEdit::Move => self.move_dragged_layer(drag, delta),
            LayerTimeEdit::TrimIn => {
                trim::trim_in(&mut self.project, comp, drag.layer, drag.in_frame + delta);
            }
            LayerTimeEdit::TrimOut => {
                trim::trim_out(&mut self.project, comp, drag.layer, drag.out_frame + delta);
            }
            LayerTimeEdit::Slip => {
                trim::slip_to(&mut self.project, comp, drag.layer, drag.start_frame + delta);
            }
        }
    }

    /// Moves the dragged layer (and its keyframes) to `delta` frames from where it started.
    /// Selected keyframes on that layer follow along.
    fn move_dragged_layer(&mut self, drag: LayerTimeDrag, delta: i64) {
        let step = delta - drag.applied;
        if step == 0 {
            return;
        }
        let Some(layer) = self.composition_mut().layer_mut(drag.layer) else { return };
        if layer.switches.locked {
            return;
        }
        layer.shift_in_time(step);
        for key in self.selected_keyframes.iter_mut().filter(|key| key.layer == drag.layer) {
            key.frame += step;
        }
        self.layer_time_drag = Some(LayerTimeDrag { applied: delta, ..drag });
    }

    /// Whether `layer`'s source can be slipped (it has more source than is showing).
    pub fn can_slip(&self, layer: Id) -> bool {
        let composition = self.composition();
        composition.layer(layer).is_some_and(|layer| trim::can_slip(&self.project, composition, layer))
    }

    /// The span of a layer's whole source on the timeline, for layers whose source has a
    /// fixed length: where it starts and the frame after it ends.
    pub fn source_span(&self, layer: Id) -> Option<(i64, i64)> {
        let composition = self.composition();
        let layer = composition.layer(layer)?;
        trim::source_frames(&self.project, composition, layer).map(|length| (layer.start_frame, layer.start_frame + length))
    }

    /// Splits the selected layers at the playhead (every layer there, if none is selected).
    /// The new second halves become the selection, so splitting again cuts further along.
    pub fn split_at_playhead(&mut self) -> SplitReport {
        let (comp, frame) = (self.active_comp, self.current_frame);
        let report = cut::split_layers(&mut self.project, comp, &self.selected_layers, frame);
        if !report.created.is_empty() {
            self.selected_layers = report.created.clone();
            self.selected_keyframes.clear();
        }
        report
    }

    /// The Cut tool: splits one clip at `frame` (pulled to the playhead when snapping and close
    /// to it), and selects the part after the cut. Returns whether it was split.
    pub fn cut_layer_at(&mut self, layer: Id, frame: i64) -> bool {
        let frame = if self.snapping && (frame - self.current_frame).abs() <= self.snap_threshold { self.current_frame } else { frame };
        let comp = self.active_comp;
        let report = cut::split_layers(&mut self.project, comp, &[layer], frame);
        if report.created.is_empty() {
            return false;
        }
        self.selected_layers = report.created;
        self.selected_keyframes.clear();
        true
    }

    /// Deletes the selected layers and closes the gaps they leave. Returns how many were deleted.
    pub fn ripple_delete_selection(&mut self) -> usize {
        let comp = self.active_comp;
        let deleted = cut::ripple_delete(&mut self.project, comp, &self.selected_layers);
        let remaining = self.composition();
        let kept: Vec<Id> = self.selected_layers.iter().copied().filter(|id| remaining.layer(*id).is_some()).collect();
        self.selected_layers = kept;
        self.selected_keyframes.clear();
        deleted
    }

    /// Alt+[: starts the selected layers at the playhead. Returns whether any changed.
    pub fn trim_selected_in_to_playhead(&mut self) -> bool {
        let (comp, frame) = (self.active_comp, self.current_frame);
        let layers = self.selected_layers.clone();
        layers.iter().fold(false, |changed, &layer| trim::trim_in(&mut self.project, comp, layer, frame) | changed)
    }

    /// Alt+]: ends the selected layers at the playhead (the playhead's frame is their last).
    /// Returns whether any changed.
    pub fn trim_selected_out_to_playhead(&mut self) -> bool {
        let (comp, frame) = (self.active_comp, self.current_frame);
        let layers = self.selected_layers.clone();
        layers.iter().fold(false, |changed, &layer| trim::trim_out(&mut self.project, comp, layer, frame + 1) | changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::property::{PropertyPath, TransformProperty};

    fn title_keys(state: &EditorState) -> Vec<i64> {
        let title = state.composition().layers[0].id;
        state.composition().layer(title).unwrap().property(PropertyPath::Transform(TransformProperty::Opacity), 0).unwrap().keyframes
    }

    #[test]
    fn the_cut_tool_splits_one_clip_where_clicked() {
        let mut state = EditorState::new();
        state.snapping = false;
        let title = state.composition().layers[0].id;
        let (in_frame, out_frame) = { let layer = state.composition().layer(title).unwrap(); (layer.in_frame, layer.out_frame) };
        let count = state.composition().layers.len();
        assert!(state.cut_layer_at(title, in_frame + 10));
        assert_eq!(state.composition().layers.len(), count + 1, "only that clip was split");
        assert_eq!(state.composition().layer(title).unwrap().out_frame, in_frame + 10);
        let after = state.composition().layer(state.selected_layers[0]).unwrap();
        assert_eq!((after.in_frame, after.out_frame), (in_frame + 10, out_frame));
        assert!(!state.cut_layer_at(title, in_frame), "cutting at an end does nothing");
    }

    /// The colors clip (2 s, 48 frames at 24 fps) imported and placed on top at frame 0.
    fn with_clip() -> (EditorState, Id) {
        let mut state = EditorState::new();
        let clip = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-media/tests/fixtures/colors.mp4");
        state.import_files(vec![clip]);
        let item = state.selected_item.unwrap();
        state.add_layer_from_item(item, 0, 0);
        let layer = state.composition().layers[0].id;
        (state, layer)
    }

    fn timing(state: &EditorState, layer: Id) -> (i64, i64, i64) {
        let layer = state.composition().layer(layer).unwrap();
        (layer.start_frame, layer.in_frame, layer.out_frame)
    }

    #[test]
    fn layer_bars_drag_in_time() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.begin_layer_time_drag(title, LayerTimeEdit::Move);
        state.drag_layer_time(6);
        state.drag_layer_time(10);
        let layer = state.composition().layer(title).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (34, 178));
        assert_eq!(title_keys(&state), vec![34, 46, 166, 178]);
    }

    #[test]
    fn dragging_the_ends_trims_and_keyframes_stay() {
        let (mut state, clip) = with_clip();
        state.begin_layer_time_drag(clip, LayerTimeEdit::TrimIn);
        state.drag_layer_time(5);
        state.drag_layer_time(12);
        state.end_layer_time_drag();
        state.begin_layer_time_drag(clip, LayerTimeEdit::TrimOut);
        state.drag_layer_time(-12);
        state.end_layer_time_drag();
        assert_eq!(timing(&state, clip), (0, 12, 36));
        // Dragging the out point past the clip's end stops at its last frame.
        state.begin_layer_time_drag(clip, LayerTimeEdit::TrimOut);
        state.drag_layer_time(100);
        assert_eq!(timing(&state, clip), (0, 12, 48));
    }

    #[test]
    fn alt_dragging_slips_the_source() {
        let (mut state, clip) = with_clip();
        assert!(!state.can_slip(clip), "all of the clip is showing");
        state.begin_layer_time_drag(clip, LayerTimeEdit::TrimIn);
        state.drag_layer_time(12);
        state.begin_layer_time_drag(clip, LayerTimeEdit::TrimOut);
        state.drag_layer_time(-12);
        assert!(state.can_slip(clip));
        state.begin_layer_time_drag(clip, LayerTimeEdit::Slip);
        state.drag_layer_time(-8);
        assert_eq!(timing(&state, clip), (-8, 12, 36));
        assert_eq!(state.source_span(clip), Some((-8, 40)));
        state.drag_layer_time(-50);
        assert_eq!(timing(&state, clip), (-12, 12, 36), "the clip must still reach the out point");
    }

    #[test]
    fn alt_brackets_trim_the_selection_to_the_playhead() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_layer(title, false);
        state.set_current_frame(60);
        assert!(state.trim_selected_in_to_playhead());
        state.set_current_frame(90);
        assert!(state.trim_selected_out_to_playhead());
        let layer = state.composition().layer(title).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (60, 91), "frame 90 is the last one shown");
    }
}
