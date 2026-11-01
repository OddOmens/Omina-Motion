//! Direct manipulation in the composition viewer: picking a layer under the
//! pointer, and moving or scaling selected layers by dragging. Changes are made
//! at the playhead, so animated properties are auto-keyframed as in After Effects.

use studio_core::animation::{Animated, Vec2};
use studio_core::edit;
use studio_core::transform::{Affine, layer_contains, parent_space, world_transform};
use studio_core::model::{Id, ShapeKind, ShapeStyle};
use studio_core::property::{PropertyPath, PropertyValue, TransformProperty};

use super::EditorState;

/// A corner handle of the selection box. Dragging one scales the layer about its anchor point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Debug)]
pub(super) enum ViewerDrag {
    /// Moving the selected layers: each with its starting position and the transform from
    /// composition pixels into the space its position lives in (its parent's).
    Move(Vec<(Id, Vec2, Affine)>),
    /// Scaling one layer: its starting scale, the grabbed corner's offset from the anchor in source
    /// pixels, and the transform from composition pixels into the layer's source pixels.
    Scale { layer: Id, scale: Vec2, reach: Vec2, to_local: Affine },
    /// Drawing a new shape layer with the Shape tool, from the point where the drag began.
    DrawShape { layer: Id, start: Vec2 },
}

/// A shape drawn smaller than this (a click rather than a drag) gets `DEFAULT_SHAPE_SIZE`.
const SMALLEST_DRAWN_SHAPE: f32 = 4.0;
/// The size of a shape placed with a click, in composition pixels.
const DEFAULT_SHAPE_SIZE: f32 = 300.0;

const POSITION: PropertyPath = PropertyPath::Transform(TransformProperty::Position);
const SCALE: PropertyPath = PropertyPath::Transform(TransformProperty::Scale);

impl EditorState {
    /// The topmost visible, unlocked layer under `point` (composition pixels) at the playhead,
    /// tested against the layer's real (rotated, parented) rectangle.
    pub fn layer_at(&self, point: Vec2) -> Option<Id> {
        let frame = self.current_frame;
        let comp = self.composition();
        comp.layers
            .iter()
            .filter(|layer| layer.switches.visible && !layer.switches.locked && layer.is_active_at(frame))
            .find(|layer| layer_contains(comp, layer.id, point, frame as f64))
            .map(|layer| layer.id)
    }

    fn pair(&self, layer: Id, path: PropertyPath) -> Option<Vec2> {
        match self.composition().layer(layer)?.property(path, self.current_frame)?.value {
            PropertyValue::Pair(value) => Some(value),
            PropertyValue::Scalar(_) => None,
        }
    }

    /// Starts moving the selected layers (locked ones stay put).
    pub fn begin_move(&mut self) {
        let frame = self.current_frame as f64;
        let starts = self
            .selected_layers
            .iter()
            .filter(|&&id| self.composition().layer(id).is_some_and(|layer| !layer.switches.locked))
            .filter_map(|&id| {
                let position = self.pair(id, POSITION)?;
                Some((id, position, parent_space(self.composition(), id, frame)))
            })
            .collect();
        self.viewer_drag = Some(ViewerDrag::Move(starts));
    }

    /// Starts scaling `layer` from `handle`.
    pub fn begin_scale(&mut self, layer: Id, handle: ScaleHandle) {
        let frame = self.current_frame;
        let comp = self.composition();
        let (Some(target), Some(scale), Some(world)) =
            (comp.layer(layer), self.pair(layer, SCALE), world_transform(comp, layer, frame as f64))
        else {
            return;
        };
        if target.switches.locked {
            return;
        }
        let Some(anchor) = target.transform.as_ref().map(|t| t.anchor_point.value_at(frame as f64)) else { return };
        let size = target.size;
        let corner = match handle {
            ScaleHandle::TopLeft => Vec2::new(0.0, 0.0),
            ScaleHandle::TopRight => Vec2::new(size.x, 0.0),
            ScaleHandle::BottomLeft => Vec2::new(0.0, size.y),
            ScaleHandle::BottomRight => size,
        };
        let Some(to_local) = world.inverse() else { return };
        self.viewer_drag = Some(ViewerDrag::Scale { layer, scale, reach: corner - anchor, to_local });
    }

    /// Starts drawing a shape layer of `kind` at `point` (composition pixels), at the playhead.
    /// It is selected, and grows as the drag goes on.
    pub fn begin_shape_draw(&mut self, kind: ShapeKind, point: Vec2) -> Option<Id> {
        let (comp, frame) = (self.active_comp, self.current_frame);
        let layer = edit::add_shape_layer(&mut self.project, comp, ShapeStyle::new(kind), Vec2::new(1.0, 1.0), point, frame)?;
        self.select_layer(layer, false);
        self.viewer_drag = Some(ViewerDrag::DrawShape { layer, start: point });
        Some(layer)
    }

    /// Fits a shape layer to the box from `start` to `end`, keeping it centered on its anchor.
    fn set_shape_box(&mut self, layer: Id, start: Vec2, end: Vec2) {
        let (min, max) = (Vec2::new(start.x.min(end.x), start.y.min(end.y)), Vec2::new(start.x.max(end.x), start.y.max(end.y)));
        let size = Vec2::new((max.x - min.x).max(1.0), (max.y - min.y).max(1.0));
        let composition = self.composition_mut();
        if let Some(target) = composition.layer_mut(layer) {
            target.size = size;
            if let Some(transform) = target.transform.as_mut() {
                transform.anchor_point = Animated::constant(size * 0.5);
                transform.position = Animated::constant((min + max) * 0.5);
            }
        }
        // A smaller box may no longer fit the corner radius.
        edit::edit_shape(composition, layer, |_| {});
    }

    /// Applies the current drag. `delta` is how far the pointer moved since the drag began, in
    /// composition pixels. With `constrain` (Shift), moves stay on one axis and scaling keeps proportions.
    pub fn drag_in_viewer(&mut self, delta: Vec2, constrain: bool) {
        let frame = self.current_frame;
        let Some(drag) = self.viewer_drag.clone() else { return };
        match drag {
            ViewerDrag::Move(starts) => {
                let delta = if !constrain {
                    delta
                } else if delta.x.abs() >= delta.y.abs() {
                    Vec2::new(delta.x, 0.0)
                } else {
                    Vec2::new(0.0, delta.y)
                };
                for (id, start, to_parent) in starts {
                    if let Some(layer) = self.composition_mut().layer_mut(id) {
                        layer.set_property(POSITION, frame, PropertyValue::Pair(start + to_parent.apply_vector(delta)));
                    }
                }
            }
            ViewerDrag::Scale { layer, scale, reach, to_local } => {
                // Measure the drag in the layer's own (rotated) source pixels, from the anchor point.
                let moved = to_local.apply_vector(delta);
                let ratio = |reach: f32, moved: f32| if reach.abs() > 0.5 { (reach + moved) / reach } else { 1.0 };
                let (mut x, mut y) = (ratio(reach.x, moved.x), ratio(reach.y, moved.y));
                if constrain {
                    let uniform = if (x - 1.0).abs() >= (y - 1.0).abs() { x } else { y };
                    (x, y) = (uniform, uniform);
                }
                if let Some(target) = self.composition_mut().layer_mut(layer) {
                    target.set_property(SCALE, frame, PropertyValue::Pair(Vec2::new(scale.x * x, scale.y * y)));
                }
            }
            ViewerDrag::DrawShape { layer, start } => {
                // Shift keeps it square (or circular), as in every drawing tool.
                let delta = if constrain {
                    let side = delta.x.abs().max(delta.y.abs());
                    Vec2::new(side.copysign(delta.x), side.copysign(delta.y))
                } else {
                    delta
                };
                self.set_shape_box(layer, start, start + delta);
            }
        }
    }

    pub fn end_viewer_drag(&mut self) {
        // A click without a drag places a shape of the default size there.
        if let Some(ViewerDrag::DrawShape { layer, start }) = self.viewer_drag
            && self.composition().layer(layer).is_some_and(|target| target.size.x.max(target.size.y) < SMALLEST_DRAWN_SHAPE)
        {
            let half = Vec2::new(DEFAULT_SHAPE_SIZE, DEFAULT_SHAPE_SIZE) * 0.5;
            self.set_shape_box(layer, start - half, start + half);
        }
        self.viewer_drag = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::align::layer_bounds;
    use studio_core::property::TransformProperty;

    #[test]
    fn dragging_with_the_shape_tool_draws_the_box_and_a_click_places_a_default_one() {
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Rectangle, Vec2::new(100.0, 100.0)).unwrap();
        state.drag_in_viewer(Vec2::new(200.0, -80.0), false);
        state.end_viewer_drag();
        let drawn = state.composition().layer(layer).unwrap();
        assert_eq!(drawn.size, Vec2::new(200.0, 80.0));
        assert_eq!(drawn.transform.as_ref().unwrap().position.value_at(0.0), Vec2::new(200.0, 60.0), "centered in the box");
        let bounds = layer_bounds(state.composition(), layer, 0).unwrap();
        assert!((bounds.min.x - 100.0).abs() < 0.01 && (bounds.min.y - 20.0).abs() < 0.01, "from the drag start: {bounds:?}");

        let clicked = state.begin_shape_draw(ShapeKind::Star, Vec2::new(500.0, 500.0)).unwrap();
        state.end_viewer_drag();
        assert_eq!(state.composition().layer(clicked).unwrap().size, Vec2::new(DEFAULT_SHAPE_SIZE, DEFAULT_SHAPE_SIZE));
        assert_eq!(state.selected_layers, vec![clicked]);
    }

    #[test]
    fn shift_draws_a_square() {
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Ellipse, Vec2::new(0.0, 0.0)).unwrap();
        state.drag_in_viewer(Vec2::new(120.0, 40.0), true);
        assert_eq!(state.composition().layer(layer).unwrap().size, Vec2::new(120.0, 120.0));
    }

    #[test]
    fn picks_the_topmost_layer_under_the_pointer() {
        let mut state = EditorState::new();
        state.set_current_frame(100);
        let (grade, logo) = (state.composition().layers[1].id, state.composition().layers[2].id);
        // The full-frame Grade adjustment layer sits above the logo, so it is picked first.
        assert_eq!(state.layer_at(Vec2::new(960.0, 300.0)), Some(grade));
        // With Grade hidden, the logo is the topmost layer there, but not outside its bounds.
        state.toggle_switch(grade, super::super::Switch::Visible);
        assert_eq!(state.layer_at(Vec2::new(960.0, 300.0)), Some(logo));
        assert_ne!(state.layer_at(Vec2::new(100.0, 1000.0)), Some(logo));
    }

    #[test]
    fn moving_auto_keyframes_animated_positions() {
        let mut state = EditorState::new();
        let title = state.composition().layers[0].id;
        state.set_current_frame(100);
        state.select_layer(title, false);
        state.begin_move();
        state.drag_in_viewer(Vec2::new(40.0, 10.0), false);
        state.drag_in_viewer(Vec2::new(50.0, 20.0), true);
        state.end_viewer_drag();
        let layer = state.composition().layer(title).unwrap();
        let position = layer.property(POSITION, 100).unwrap();
        assert_eq!(position.value, PropertyValue::Pair(Vec2::new(1010.0, 540.0)), "shift keeps the move horizontal");
        assert!(position.keyframes.contains(&100));
    }

    #[test]
    fn moving_a_parented_layer_follows_the_parents_rotation() {
        let mut state = EditorState::new();
        let (child, parent) = (state.composition().layers[0].id, state.composition().layers[5].id);
        state.set_parent(child, Some(parent));
        let rotation = PropertyPath::Transform(TransformProperty::Rotation);
        state.set_property_component(parent, rotation, 0, 90.0);
        state.set_current_frame(100);
        let before = studio_core::transform::layer_corners(state.composition(), child, 100.0).unwrap()[0];
        state.select_layer(child, false);
        state.begin_move();
        state.drag_in_viewer(Vec2::new(50.0, 0.0), false);
        let after = studio_core::transform::layer_corners(state.composition(), child, 100.0).unwrap()[0];
        // On screen the layer moves exactly as far as the pointer, even though its parent is turned.
        assert!((after.x - before.x - 50.0).abs() < 1e-3 && (after.y - before.y).abs() < 1e-3, "{before:?} → {after:?}");
    }

    #[test]
    fn corner_drags_scale_about_the_anchor() {
        let mut state = EditorState::new();
        let solid = state.composition().layers[5].id;
        state.begin_scale(solid, ScaleHandle::BottomRight);
        // The corner starts 960 × 540 px from the anchor; moving it out by half of that is 150%.
        state.drag_in_viewer(Vec2::new(480.0, 270.0), false);
        state.end_viewer_drag();
        let layer = state.composition().layer(solid).unwrap();
        assert_eq!(layer.property(SCALE, 0).unwrap().value, PropertyValue::Pair(Vec2::new(150.0, 150.0)));
        let bounds = layer_bounds(state.composition(), solid, 0).unwrap();
        assert_eq!(bounds.max, Vec2::new(1920.0 + 480.0, 1080.0 + 270.0));
    }
}
