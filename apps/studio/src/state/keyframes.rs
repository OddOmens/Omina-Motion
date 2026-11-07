//! Keyframe editing: the stopwatch, adding and removing keyframes, selecting and
//! dragging them in the timeline, and applying easing curves (the Flow panel).

use studio_core::animation::Easing;
use studio_core::model::Id;
use studio_core::property::PropertyPath;

use super::EditorState;

/// One keyframe, identified by its layer, property and frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyframeRef {
    pub layer: Id,
    pub path: PropertyPath,
    pub frame: i64,
}

/// Selected keyframes while they are being dragged: where each started and where it is now.
#[derive(Clone, Debug)]
pub(super) struct KeyframeDrag {
    keys: Vec<(KeyframeRef, i64)>,
}

impl EditorState {
    /// Turns animation on (keyframe at the playhead) or off (keeping the current value).
    pub fn toggle_stopwatch(&mut self, layer: Id, path: PropertyPath) {
        let frame = self.current_frame;
        // Speed's stopwatch works like a ramp: turning it on remembers the steady speed, and
        // turning it off goes back to it, so the clip's length and timing stay consistent.
        if path == PropertyPath::Speed {
            let Some(time) = self.composition().layer(layer).map(|layer| layer.time.clone()) else { return };
            if time.speed.is_animated() {
                self.remove_speed_ramp(layer);
                self.selected_keyframes.retain(|key| !(key.layer == layer && key.path == path));
                return;
            }
            if let Some(clip) = self.composition_mut().layer_mut(layer).filter(|clip| !clip.switches.locked) {
                clip.time.speed_before_ramp = Some(time.speed.value);
            }
        }
        if let Some(animated) = self.composition_mut().layer_mut(layer).and_then(|l| l.animated_mut(path)) {
            animated.toggle_stopwatch(frame);
        }
        self.selected_keyframes.retain(|key| !(key.layer == layer && key.path == path));
    }

    /// Adds a keyframe at the playhead, or removes the one already there.
    pub fn toggle_keyframe_at_playhead(&mut self, layer: Id, path: PropertyPath) {
        let frame = self.current_frame;
        if let Some(animated) = self.composition_mut().layer_mut(layer).and_then(|l| l.animated_mut(path)) {
            animated.toggle_keyframe(frame);
        }
        self.selected_keyframes.retain(|key| *key != KeyframeRef { layer, path, frame });
    }

    /// Moves the playhead to the previous (`forward` false) or next keyframe of a property.
    pub fn jump_to_keyframe(&mut self, layer: Id, path: PropertyPath, forward: bool) {
        let frame = self.current_frame;
        let Some(snapshot) = self.composition().layer(layer).and_then(|l| l.property(path, frame)) else { return };
        let target = if forward {
            snapshot.keyframes.iter().copied().find(|&key| key > frame)
        } else {
            snapshot.keyframes.iter().rev().copied().find(|&key| key < frame)
        };
        if let Some(target) = target {
            self.set_current_frame(target);
        }
    }

    pub fn is_keyframe_selected(&self, key: &KeyframeRef) -> bool {
        self.selected_keyframes.contains(key)
    }

    /// Selects a keyframe. With `additive`, toggles it in the current selection.
    pub fn select_keyframe(&mut self, key: KeyframeRef, additive: bool) {
        if !self.selected_layers.contains(&key.layer) {
            self.selected_layers = vec![key.layer];
        }
        if !additive {
            self.selected_keyframes = vec![key];
        } else if let Some(index) = self.selected_keyframes.iter().position(|selected| *selected == key) {
            self.selected_keyframes.remove(index);
        } else {
            self.selected_keyframes.push(key);
        }
    }

    pub(super) fn delete_selected_keyframes(&mut self) -> usize {
        let keys = std::mem::take(&mut self.selected_keyframes);
        for key in &keys {
            if let Some(animated) = self.composition_mut().layer_mut(key.layer).and_then(|l| l.animated_mut(key.path)) {
                animated.remove_keyframe(key.frame);
            }
        }
        keys.len()
    }

    /// Starts dragging the selected keyframes in time.
    pub fn begin_keyframe_drag(&mut self) {
        self.keyframe_drag = Some(KeyframeDrag { keys: self.selected_keyframes.iter().map(|key| (*key, key.frame)).collect() });
    }

    /// Moves the dragged keyframes `delta` frames from where they started. They move as a
    /// group: at the composition's start or end the whole group stops, keeping its spacing.
    /// A keyframe that would land on another (unselected) keyframe stays where it is.
    pub fn drag_keyframes(&mut self, delta: i64) {
        let Some(origins) = self.keyframe_drag.as_ref().map(|drag| drag.keys.iter().map(|(origin, _)| origin.frame).collect::<Vec<_>>()) else { return };
        let (Some(&first), Some(&last)) = (origins.iter().min(), origins.iter().max()) else { return };
        let exclude = super::snap::SnapExclusions { selected_keyframes: true, ..Default::default() };
        // The group may already lie partly outside the composition (after it was shortened, or a
        // layer was moved in time). It can always move back in, but never further out.
        let earliest = (-first).min(0);
        let latest = (self.duration() - 1 - last).max(0);
        let delta = self.snapped_delta(&origins, delta, exclude).clamp(earliest, latest);
        let Some(mut drag) = self.keyframe_drag.take() else { return };
        // Each keyframe heads for its own target. Move those going right from the rightmost
        // first, then those going left from the leftmost first, so keys moving together never
        // land on one another.
        let target = |index: usize| drag.keys[index].0.frame + delta;
        let mut right: Vec<usize> = (0..drag.keys.len()).filter(|&i| target(i) > drag.keys[i].1).collect();
        let mut left: Vec<usize> = (0..drag.keys.len()).filter(|&i| target(i) < drag.keys[i].1).collect();
        right.sort_by_key(|&i| std::cmp::Reverse(drag.keys[i].1));
        left.sort_by_key(|&i| drag.keys[i].1);
        for index in right.into_iter().chain(left) {
            let (origin, current) = drag.keys[index];
            let to = origin.frame + delta;
            let moved = self
                .composition_mut()
                .layer_mut(origin.layer)
                .and_then(|layer| layer.animated_mut(origin.path))
                .is_some_and(|animated| animated.move_keyframe(current, to));
            if moved {
                drag.keys[index].1 = to;
            }
        }
        self.selected_keyframes = drag.keys.iter().map(|(origin, current)| KeyframeRef { frame: *current, ..*origin }).collect();
        self.keyframe_drag = Some(drag);
    }

    pub fn end_keyframe_drag(&mut self) {
        self.keyframe_drag = None;
        self.clear_snap_line();
    }

    /// The keyframes whose outgoing segment an easing change affects, as in Flow: on a property with
    /// several selected keyframes, the segments between consecutive selected keyframes; on a
    /// property with one selected keyframe, the segment leaving it.
    pub fn eased_segments(&self) -> Vec<KeyframeRef> {
        let comp = self.composition();
        let mut segments = Vec::new();
        let mut groups: Vec<(Id, PropertyPath)> = self.selected_keyframes.iter().map(|key| (key.layer, key.path)).collect();
        groups.sort_by_key(|(layer, path)| (*layer, format!("{path:?}")));
        groups.dedup();
        for (layer_id, path) in groups {
            let Some(layer) = comp.layer(layer_id) else { continue };
            let frames: Vec<i64> = layer.keyframe_easings(path).iter().map(|(frame, _)| *frame).collect();
            let selected: Vec<i64> = self
                .selected_keyframes
                .iter()
                .filter(|key| key.layer == layer_id && key.path == path)
                .map(|key| key.frame)
                .collect();
            for (index, frame) in frames.iter().enumerate() {
                let next_selected = frames.get(index + 1).is_some_and(|next| selected.contains(next));
                if selected.contains(frame) && (selected.len() == 1 || next_selected) {
                    segments.push(KeyframeRef { layer: layer_id, path, frame: *frame });
                }
            }
        }
        segments
    }

    /// Applies an easing curve to the selected segments (see `eased_segments`). The graph
    /// editor eases only the segment it is focused on (`apply_easing_to`); tests ease them all.
    #[cfg(test)]
    pub fn apply_easing(&mut self, easing: Easing) {
        self.apply_easing_to(&self.eased_segments(), easing);
    }

    /// Applies an easing curve to the segments leaving the keyframes `segments`.
    pub fn apply_easing_to(&mut self, segments: &[KeyframeRef], easing: Easing) {
        for &key in segments {
            if let Some(animated) = self.composition_mut().layer_mut(key.layer).and_then(|l| l.animated_mut(key.path)) {
                animated.set_easing(key.frame, easing);
            }
        }
    }

    /// Easy Ease (F9): eases both into and out of every selected keyframe.
    pub fn easy_ease_selected(&mut self) {
        let easing = Easing::Curve(studio_core::animation::EASY_EASE);
        for key in self.selected_keyframes.clone() {
            let Some(layer) = self.composition_mut().layer_mut(key.layer) else { continue };
            let previous = layer.keyframe_easings(key.path).iter().map(|(frame, _)| *frame).rfind(|frame| *frame < key.frame);
            for frame in std::iter::once(key.frame).chain(previous) {
                if let Some(animated) = layer.animated_mut(key.path) {
                    animated.set_easing(frame, easing);
                }
            }
        }
    }

    /// Replaces the keyframe selection, for example after a box selection.
    pub fn set_keyframe_selection(&mut self, keys: Vec<KeyframeRef>) {
        for key in &keys {
            if !self.selected_layers.contains(&key.layer) {
                self.selected_layers.push(key.layer);
            }
        }
        self.selected_keyframes = keys;
    }

    /// Selects every keyframe of one property, as clicking a property's name does in After Effects.
    pub fn select_property_keyframes(&mut self, layer: Id, path: PropertyPath, additive: bool) {
        let frames: Vec<i64> = self.composition().layer(layer).map(|l| l.keyframe_easings(path).iter().map(|(f, _)| *f).collect()).unwrap_or_default();
        let keys = frames.into_iter().map(|frame| KeyframeRef { layer, path, frame });
        if additive {
            let mut selection = self.selected_keyframes.clone();
            selection.extend(keys.filter(|key| !self.selected_keyframes.contains(key)));
            self.set_keyframe_selection(selection);
        } else {
            self.selected_layers = vec![layer];
            self.set_keyframe_selection(keys.collect());
        }
    }

    /// The easing shared by every selected keyframe, if they all agree.
    #[cfg(test)]
    pub fn selection_easing(&self) -> Option<Easing> {
        self.segments_easing(&self.eased_segments())
    }

    /// Where the segment leaving keyframe `key` ends: the property's next keyframe.
    pub fn segment_end(&self, key: KeyframeRef) -> Option<i64> {
        let layer = self.composition().layer(key.layer)?;
        layer.keyframe_easings(key.path).into_iter().map(|(frame, _)| frame).find(|frame| *frame > key.frame)
    }

    /// Whether the value changes over the segment leaving `key`; easing a segment that holds
    /// one value (between a fade in and a fade out, say) makes no visible difference.
    pub fn segment_changes(&self, key: KeyframeRef) -> bool {
        let Some(end) = self.segment_end(key) else { return false };
        let Some(layer) = self.composition().layer(key.layer) else { return false };
        let value = |frame| layer.property(key.path, frame).map(|snapshot| snapshot.value);
        value(key.frame) != value(end)
    }

    /// The easing shared by the segments leaving `segments`, if they all agree.
    pub fn segments_easing(&self, segments: &[KeyframeRef]) -> Option<Easing> {
        let comp = self.composition();
        let mut easings = segments.iter().filter_map(|key| {
            let layer = comp.layer(key.layer)?;
            easing_of(layer, key.path, key.frame)
        });
        let first = easings.next()?;
        easings.all(|easing| easing == first).then_some(first)
    }
}

/// The easing leaving the keyframe at `frame` of a property.
fn easing_of(layer: &studio_core::model::Layer, path: PropertyPath, frame: i64) -> Option<Easing> {
    layer.keyframe_easings(path).into_iter().find(|(key, _)| *key == frame).map(|(_, easing)| easing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::animation::EASING_PRESETS;
    use studio_core::property::TransformProperty;

    fn opacity() -> PropertyPath {
        PropertyPath::Transform(TransformProperty::Opacity)
    }

    fn rotation() -> PropertyPath {
        PropertyPath::Transform(TransformProperty::Rotation)
    }

    fn title_keys(state: &EditorState) -> Vec<i64> {
        let title = state.composition().layers[0].id;
        state.composition().layer(title).unwrap().property(opacity(), 0).unwrap().keyframes
    }

    #[test]
    fn stopwatch_and_keyframe_buttons() {
        let mut state = EditorState::new();
        let solid = state.composition().layers[5].id;
        let rotation = PropertyPath::Transform(TransformProperty::Rotation);
        state.set_current_frame(30);
        state.toggle_stopwatch(solid, rotation);
        state.set_current_frame(60);
        state.toggle_keyframe_at_playhead(solid, rotation);
        let keys = state.composition().layer(solid).unwrap().property(rotation, 0).unwrap().keyframes;
        assert_eq!(keys, vec![30, 60]);
        state.jump_to_keyframe(solid, rotation, false);
        assert_eq!(state.current_frame, 30);
        state.jump_to_keyframe(solid, rotation, true);
        assert_eq!(state.current_frame, 60);
    }

    #[test]
    fn dragging_moves_selected_keyframes_together() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 24 }, false);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 36 }, true);
        state.begin_keyframe_drag();
        state.drag_keyframes(5);
        state.drag_keyframes(10);
        state.end_keyframe_drag();
        assert_eq!(title_keys(&state), vec![34, 46, 156, 168]);
        assert!(state.selected_keyframes.iter().all(|key| [34, 46].contains(&key.frame)));
    }

    #[test]
    fn keyframes_stop_at_neighbors() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 24 }, false);
        state.begin_keyframe_drag();
        state.drag_keyframes(12); // would land on 36
        assert_eq!(title_keys(&state), vec![24, 36, 156, 168]);
    }

    #[test]
    fn a_keyframe_group_stops_together_at_the_composition_edges() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        for frame in [24, 36] {
            state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame }, true);
        }
        state.begin_keyframe_drag();
        for step in 1..=40 {
            state.drag_keyframes(-step);
        }
        assert_eq!(title_keys(&state), vec![0, 12, 156, 168], "the spacing survives hitting frame 0");
    }

    #[test]
    fn keyframes_past_a_shortened_composition_can_still_be_dragged() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        // The title fades at 24, 36, 156 and 168; the composition now ends at frame 100.
        state.composition_mut().duration_frames = 100;
        for frame in [24, 36, 156, 168] {
            state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame }, true);
        }
        state.begin_keyframe_drag();
        state.drag_keyframes(30);
        assert_eq!(title_keys(&state), vec![24, 36, 156, 168], "never further out");
        state.drag_keyframes(-10);
        assert_eq!(title_keys(&state), vec![14, 26, 146, 158], "but back in, as far as frame 0 allows");
        state.drag_keyframes(-40);
        assert_eq!(title_keys(&state), vec![0, 12, 132, 144]);
    }

    #[test]
    fn a_stuck_keyframe_does_not_scramble_the_rest_of_the_group() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let last = state.composition().duration_frames - 1;
        // A keyframe on the last frame, plus the pair at 24 and 36.
        state.toggle_stopwatch(title, rotation());
        state.composition_mut().layer_mut(title).unwrap().animated_mut(rotation()).unwrap().move_keyframe(0, last);
        let keys = [(rotation(), last), (opacity(), 24), (opacity(), 36)];
        for (path, frame) in keys {
            state.select_keyframe(KeyframeRef { layer: title, path, frame }, true);
        }
        state.begin_keyframe_drag();
        state.drag_keyframes(24); // held back: the last-frame key can't go further
        state.drag_keyframes(-12);
        assert_eq!(title_keys(&state), vec![12, 24, 156, 168], "both opacity keys moved together");
    }

    #[test]
    fn flow_eases_only_between_selected_keyframes() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let curve = EASING_PRESETS[2].1;
        // Select the first two opacity keyframes (24 and 36); only the 24→36 segment changes.
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 24 }, false);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 36 }, true);
        state.apply_easing(Easing::Curve(curve));
        let easings = state.composition().layer(title).unwrap().keyframe_easings(opacity());
        assert_eq!(easings[0].1, Easing::Curve(curve));
        assert_eq!(easings[1].1, Easing::Linear, "the segment after the last selected keyframe is untouched");
    }

    #[test]
    fn easy_ease_eases_both_sides() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 36 }, false);
        state.easy_ease_selected();
        let easings = state.composition().layer(title).unwrap().keyframe_easings(opacity());
        let ease = Easing::Curve(studio_core::animation::EASY_EASE);
        assert_eq!((easings[0].1, easings[1].1, easings[2].1), (ease, ease, Easing::Linear));
    }

    #[test]
    fn clicking_a_property_selects_all_its_keyframes() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.select_property_keyframes(title, opacity(), false);
        assert_eq!(state.selected_keyframes.len(), 4);
        let position = PropertyPath::Transform(TransformProperty::Position);
        state.select_property_keyframes(title, position, true);
        assert_eq!(state.selected_keyframes.len(), 6, "shift adds the other property's keyframes");
    }

    #[test]
    fn easing_applies_to_selected_keyframes() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        let curve = EASING_PRESETS[2].1;
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 24 }, false);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 36 }, true);
        state.apply_easing(Easing::Curve(curve));
        assert_eq!(state.selection_easing(), Some(Easing::Curve(curve)));
        // Adding the 156→168 segment, still linear, makes the selection mixed.
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 156 }, true);
        state.select_keyframe(KeyframeRef { layer: title, path: opacity(), frame: 168 }, true);
        assert_eq!(state.selection_easing(), None, "mixed easings have no single value");
    }
}
