//! Editing clips on tracks, for the timeline's Tracks view (see `studio_core::tracks`).
//! Dragging a clip moves it in time (snapping as usual) and, on release, onto the track under
//! the pointer; a clip that would overlap another there gets a new track of its own. With the
//! magnetic main track on, the main track's clips are kept packed after every edit.

use studio_core::animation::Vec2;
use studio_core::model::{Composition, Id};
use studio_core::property::{PropertyPath, PropertyValue, TransformProperty};
use studio_core::{slots, tracks};

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
    /// overlap a clip there). A track it leaves with nothing on stays as an empty row.
    pub fn end_clip_drag(&mut self) {
        let Some(drag) = self.clip_drag.take() else { return };
        self.clear_snap_line();
        let Some(in_frame) = self.composition().layer(drag.layer).map(|layer| layer.in_frame) else { return };
        tracks::move_clip_keeping_track(self.composition_mut(), drag.layer, drag.target_track, in_frame);
    }

    /// Removes an empty track kept on the timeline (the × on its header).
    pub fn remove_empty_track(&mut self, track: i32) {
        tracks::remove_empty_track(self.composition_mut(), track);
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

    /// Adds project items from a drop on the viewer: from the playhead, one after another on a
    /// new track at the top, each centered on `at` (composition pixels). A picture that covers
    /// the whole frame stays centered, so full-frame video isn't knocked off by a slightly
    /// off-center drop. Selects the last one added and returns whether anything was added.
    pub fn add_items_at(&mut self, items: &[Id], at: Vec2) -> bool {
        let comp = self.active_comp;
        let mut frame = self.current_frame;
        let mut track = None;
        let mut added = false;
        for &item in items {
            // Items that can't go in (a composition into itself) are skipped.
            let Some(layer) = studio_core::edit::add_layer_from_item(&mut self.project, comp, item, 0, frame) else { continue };
            // The first starts a new track on top; the rest follow it on that track.
            if let Some(track) = track {
                tracks::place_clip(self.composition_mut(), layer, track, frame);
            }
            center_on(self.composition_mut(), layer, at);
            let Some(placed) = self.composition().layer(layer) else { continue };
            track = Some(placed.track);
            frame = placed.out_frame;
            self.select_layer(layer, false);
            added = true;
        }
        added
    }

    /// The media slot under `at` (pixels of the open composition) at the playhead, if any: a
    /// placeholder or a nested composition showing one photo or video (see `studio_core::slots`).
    pub fn slot_at(&self, at: Vec2) -> Option<slots::Slot> {
        slots::slot_at(&self.project, self.active_comp, at, self.current_frame)
    }

    /// Project items dropped on the viewer at `at`. When they land on a media slot, the first
    /// photo or video fills it and the layer it was found under is selected; everything else is
    /// added as layers, as `add_items_at` does. Returns whether anything changed.
    pub fn drop_items_at(&mut self, items: &[Id], at: Vec2) -> bool {
        let filling = self.slot_at(at).zip(items.iter().position(|&item| slots::can_fill_with(&self.project, item)));
        let Some((slot, index)) = filling else { return self.add_items_at(items, at) };
        if slots::fill(&mut self.project, &slot, items[index]).is_none() {
            return self.add_items_at(items, at);
        }
        self.select_layer(slot.root, false);
        let rest: Vec<Id> = items.iter().enumerate().filter(|&(position, _)| position != index).map(|(_, &item)| item).collect();
        self.add_items_at(&rest, at);
        true
    }

    /// Keeps the main track packed if it is magnetic. Run after edits, outside drags.
    pub(super) fn apply_magnetism(&mut self) {
        if self.magnetic_main_track {
            tracks::compact_main_track(self.composition_mut());
        }
    }
}

/// Moves a just-added layer so its picture is centered on `at`, unless it already fills the
/// frame. Sound-only layers have no picture and are left alone.
fn center_on(composition: &mut Composition, layer: Id, at: Vec2) {
    let Some(start) = composition.layer(layer).map(|layer| layer.in_frame) else { return };
    let Some(bounds) = studio_core::align::layer_bounds(composition, layer, start) else { return };
    let (width, height) = (composition.width as f32, composition.height as f32);
    let fills_frame = bounds.min.x <= 0.5 && bounds.min.y <= 0.5 && bounds.max.x >= width - 0.5 && bounds.max.y >= height - 0.5;
    if fills_frame || !(at.x.is_finite() && at.y.is_finite()) {
        return;
    }
    let shift = at - (bounds.min + bounds.max) * 0.5;
    let Some(layer) = composition.layer_mut(layer) else { return };
    let Some(position) = layer.transform.as_ref().map(|transform| transform.position.value_at(start as f64)) else { return };
    layer.set_property(PropertyPath::Transform(TransformProperty::Position), start, PropertyValue::Pair(position + shift));
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
    fn items_dropped_on_the_viewer_start_at_the_playhead_where_they_were_let_go() {
        use studio_core::sample::ids;
        let mut state = EditorState::new();
        state.set_current_frame(30);
        let at = Vec2::new(400.0, 300.0);
        assert!(state.add_items_at(&[ids::LOGO, ids::B_ROLL], at));
        let logo = state.composition().layers.iter().find(|layer| matches!(layer.content, studio_core::model::LayerContent::Image { item } if item == ids::LOGO) && layer.in_frame == 30).unwrap();
        let bounds = studio_core::align::layer_bounds(state.composition(), logo.id, 30).unwrap();
        let center = (bounds.min + bounds.max) * 0.5;
        assert!((center.x - at.x).abs() < 0.5 && (center.y - at.y).abs() < 0.5, "the logo is centered on the drop: {center:?}");
        // The full-frame B-roll follows the logo and stays centered on the frame.
        let b_roll = state.composition().layer(state.selected_layers[0]).unwrap();
        assert_eq!(b_roll.in_frame, logo.out_frame, "one after another");
        assert_eq!(b_roll.track, logo.track, "on one track");
        let bounds = studio_core::align::layer_bounds(state.composition(), b_roll.id, b_roll.in_frame).unwrap();
        assert!(bounds.min.x.abs() < 0.5 && bounds.min.y.abs() < 0.5, "full-frame video isn't moved: {bounds:?}");
    }

    #[test]
    fn a_composition_dropped_into_itself_adds_nothing() {
        let mut state = EditorState::new();
        let before = state.composition().layers.len();
        assert!(!state.add_items_at(&[state.active_comp], Vec2::new(10.0, 10.0)));
        assert_eq!(state.composition().layers.len(), before);
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
