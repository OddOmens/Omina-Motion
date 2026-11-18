//! Direct manipulation in the composition viewer: picking a layer under the
//! pointer, and moving or scaling selected layers by dragging. Changes are made
//! at the playhead, so animated properties are auto-keyframed as in After Effects.

use studio_core::animation::{Animated, Vec2};
use studio_core::edit;
use studio_core::frame_snap::{Snapped, snap_box};
use studio_core::transform::{Affine, layer_contains, layer_corners, parent_space, world_transform};
use studio_core::edit::PathPart;
use studio_core::model::{Id, ShapeKind, ShapeStyle};
use studio_core::path::{PathPoint, ShapePath};
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
    /// composition pixels into the space its position lives in (its parent's), and the box
    /// around them all before the move, as (top left, bottom right) in composition pixels, for
    /// snapping.
    Move { layers: Vec<(Id, Vec2, Affine)>, bounds: Option<(Vec2, Vec2)> },
    /// Scaling one layer: its starting scale, the grabbed corner's offset from the anchor in source
    /// pixels, and the transform from composition pixels into the layer's source pixels.
    Scale { layer: Id, scale: Vec2, reach: Vec2, to_local: Affine },
    /// Drawing a new shape layer with the Shape tool, from the point where the drag began.
    DrawShape { layer: Id, start: Vec2 },
    /// Moving a layer's anchor point: where it started (layer pixels), the transform from
    /// composition pixels into the layer's pixels, and how near (in layer pixels) it snaps.
    Anchor { layer: Id, start: Vec2, to_local: Affine, reach: f32 },
    /// Turning layers with the Rotate tool about the first one's anchor point (composition
    /// pixels): each layer's rotation when the drag began, and where the pointer started.
    Rotate { starts: Vec<(Id, f32)>, pivot: Vec2, start: Vec2 },
    /// Pulling out the handles of the point just placed with the Pen tool, making it smooth.
    PenHandle { index: usize },
    /// Moving a point (or handle) of a pen path layer, from where it started in composition
    /// pixels. The layer's box is fitted again as it moves, so its own pixels shift; working
    /// from composition pixels keeps the drag steady.
    PathPoint { layer: Id, index: usize, part: PathPart, start: Vec2 },
    /// Moving one end of a shape's gradient, from where it started in composition pixels.
    GradientPoint { layer: Id, slot: super::PaintSlot, end: super::GradientEnd, start: Vec2 },
    /// One of an effect's on-screen points (a Corner Pin corner, say), and where it started.
    EffectPoint { layer: Id, effect: Id, point: usize, start: Vec2 },
}

/// A point of a pen path layer as shown in the viewer, in composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPathPoint {
    pub at: Vec2,
    pub handle_in: Vec2,
    pub handle_out: Vec2,
}

/// The nine points of a layer's box an anchor snaps to (corners, edge middles, center), as
/// fractions of its size.
pub const ANCHOR_POINTS: [(f32, f32); 9] = [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (0.0, 0.5), (0.5, 0.5), (1.0, 0.5), (0.0, 1.0), (0.5, 1.0), (1.0, 1.0)];

/// The nearest of a box's nine anchor points to `point`, if it is within `reach`.
fn snap_anchor(point: Vec2, size: Vec2, reach: f32) -> Option<Vec2> {
    ANCHOR_POINTS
        .iter()
        .map(|&(fx, fy)| Vec2::new(size.x * fx, size.y * fy))
        .map(|target| (target, (target - point).length()))
        .filter(|&(_, distance)| distance <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(target, _)| target)
}

/// A shape drawn smaller than this (a click rather than a drag) gets `DEFAULT_SHAPE_SIZE`.
const SMALLEST_DRAWN_SHAPE: f32 = 4.0;
/// The size of a shape placed with a click, in composition pixels.
const DEFAULT_SHAPE_SIZE: f32 = 300.0;

const POSITION: PropertyPath = PropertyPath::Transform(TransformProperty::Position);
const SCALE: PropertyPath = PropertyPath::Transform(TransformProperty::Scale);
const ROTATION: PropertyPath = PropertyPath::Transform(TransformProperty::Rotation);
/// With Shift, the Rotate tool turns in steps this big, in degrees.
const ROTATION_STEP: f32 = 15.0;

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
        let starts: Vec<(Id, Vec2, Affine)> = self
            .selected_layers
            .iter()
            .filter(|&&id| self.composition().layer(id).is_some_and(|layer| !layer.switches.locked))
            .filter_map(|&id| {
                let position = self.pair(id, POSITION)?;
                Some((id, position, parent_space(self.composition(), id, frame)))
            })
            .collect();
        let bounds = self.selection_box(&starts);
        self.viewer_drag = Some(ViewerDrag::Move { layers: starts, bounds });
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

    /// Starts turning the selected layers with the Rotate tool, from `point` (composition
    /// pixels). They turn by the angle the pointer sweeps around the first one's anchor point.
    pub fn begin_rotate(&mut self, point: Vec2) {
        let frame = self.current_frame as f64;
        let comp = self.composition();
        let starts: Vec<(Id, f32)> = self
            .selected_layers
            .iter()
            .filter_map(|&id| comp.layer(id).filter(|layer| !layer.switches.locked).map(|layer| (id, layer)))
            .filter_map(|(id, layer)| layer.transform.as_ref().map(|transform| (id, transform.rotation.value_at(frame))))
            .collect();
        let Some(&(first, _)) = starts.first() else { return };
        let (Some(world), Some(anchor)) = (world_transform(comp, first, frame), comp.layer(first).and_then(|layer| layer.transform.as_ref()).map(|t| t.anchor_point.value_at(frame))) else { return };
        self.viewer_drag = Some(ViewerDrag::Rotate { starts, pivot: world.apply(anchor), start: point });
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

    /// Starts moving `layer`'s anchor point. `snap_reach` is how near (in composition pixels) the
    /// pointer must come to a corner, edge middle or center to snap to it, when snapping is on.
    pub fn begin_anchor_drag(&mut self, layer: Id, snap_reach: f32) {
        let frame = self.current_frame as f64;
        let comp = self.composition();
        let Some(target) = comp.layer(layer).filter(|layer| !layer.switches.locked) else { return };
        let (Some(transform), Some(world)) = (target.transform.as_ref(), world_transform(comp, layer, frame)) else { return };
        let Some(to_local) = world.inverse() else { return };
        // Composition pixels to layer pixels, for the reach: the layer's scale, roughly.
        let reach = to_local.apply_vector(Vec2::new(snap_reach, 0.0)).length().max(to_local.apply_vector(Vec2::new(0.0, snap_reach)).length());
        self.viewer_drag = Some(ViewerDrag::Anchor { layer, start: transform.anchor_point.value_at(frame), to_local, reach });
    }

    /// The layer whose anchor point is being dragged, so the viewer can show where it snaps.
    pub fn anchor_drag_layer(&self) -> Option<Id> {
        match self.viewer_drag {
            Some(ViewerDrag::Anchor { layer, .. }) => Some(layer),
            _ => None,
        }
    }

    /// Puts a layer's anchor point at a fraction of its box (0, 0 is the top left; 0.5, 0.5
    /// the center), keeping the layer where it is.
    pub fn set_anchor_fraction(&mut self, layer: Id, fraction: (f32, f32)) {
        let frame = self.current_frame;
        let Some(size) = self.composition().layer(layer).map(|layer| layer.size) else { return };
        edit::move_anchor(self.composition_mut(), layer, Vec2::new(size.x * fraction.0, size.y * fraction.1), frame);
    }

    /// A press with the Pen tool at `point` (composition pixels). Pressing on the first point of
    /// a path with at least two points closes it and makes the shape layer; anywhere else adds a
    /// corner point, whose handles a drag then pulls out. `close_reach` is how near the first
    /// point a press must be to close the path. Returns the new layer when the path is finished.
    pub fn pen_press(&mut self, point: Vec2, close_reach: f32) -> Option<Id> {
        let pen = self.pen.get_or_insert_with(ShapePath::default);
        if pen.points.len() >= 2 && (pen.points[0].at - point).length() <= close_reach {
            return self.finish_pen(true);
        }
        pen.points.push(PathPoint::corner(point));
        self.viewer_drag = Some(ViewerDrag::PenHandle { index: pen.points.len() - 1 });
        None
    }

    /// Ends the pen path being drawn: with at least two points it becomes a shape layer
    /// (filled if `closed`, a stroked line if not), which is selected. Returns it.
    pub fn finish_pen(&mut self, closed: bool) -> Option<Id> {
        let mut path = self.pen.take()?;
        path.closed = closed;
        let (comp, frame) = (self.active_comp, self.current_frame);
        let layer = edit::add_path_layer(&mut self.project, comp, &path, ShapeStyle::new(ShapeKind::Path), frame)?;
        self.select_layer(layer, false);
        Some(layer)
    }

    /// The points of a pen path layer in composition pixels at the playhead, for the viewer to
    /// draw and let them be dragged. Empty for other layers.
    pub fn path_points_on_screen(&self, layer: Id) -> Vec<ScreenPathPoint> {
        let comp = self.composition();
        let Some(target) = comp.layer(layer) else { return Vec::new() };
        // A morphing outline shows (and is edited) as it is at the playhead.
        let Some(path) = target.shape_path_at(self.current_frame as f64) else { return Vec::new() };
        let Some(world) = world_transform(comp, layer, self.current_frame as f64) else { return Vec::new() };
        let size = target.size;
        let place = |v: Vec2| world.apply(Vec2::new(v.x * size.x, v.y * size.y));
        path.points.iter().map(|point| ScreenPathPoint { at: place(point.at), handle_in: place(point.at + point.handle_in), handle_out: place(point.at + point.handle_out) }).collect()
    }

    /// Starts moving one point (or handle) of a pen path layer.
    pub fn begin_path_point_drag(&mut self, layer: Id, index: usize, part: PathPart) {
        let Some(shown) = self.path_points_on_screen(layer).get(index).copied() else { return };
        if self.composition().layer(layer).is_none_or(|target| target.switches.locked) {
            return;
        }
        let start = match part {
            PathPart::Point => shown.at,
            PathPart::HandleIn => shown.handle_in,
            PathPart::HandleOut => shown.handle_out,
        };
        self.viewer_drag = Some(ViewerDrag::PathPoint { layer, index, part, start });
    }

    /// Applies the current drag. `delta` is how far the pointer moved since the drag began, in
    /// composition pixels. With `constrain` (Shift), moves stay on one axis. Scaling keeps proportions
    /// while Scale is linked in the Properties panel; Shift frees them for that drag (or, when
    /// unlinked, keeps them).
    pub fn drag_in_viewer(&mut self, delta: Vec2, constrain: bool) {
        let frame = self.current_frame;
        let Some(drag) = self.viewer_drag.clone() else { return };
        match drag {
            ViewerDrag::Move { layers, bounds } => {
                let mut delta = if !constrain {
                    delta
                } else if delta.x.abs() >= delta.y.abs() {
                    Vec2::new(delta.x, 0.0)
                } else {
                    Vec2::new(0.0, delta.y)
                };
                self.frame_snapped = Snapped::default();
                if let Some((min, max)) = bounds {
                    let size = Vec2::new(self.composition().width as f32, self.composition().height as f32);
                    let snapped = keep_on_line(snap_box(min + delta, max + delta, size, self.frame_snap, self.frame_snap_reach), delta, constrain);
                    delta = delta + snapped.nudge;
                    self.frame_snapped = snapped;
                }
                for (id, start, to_parent) in layers {
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
                if self.scale_linked != constrain {
                    let uniform = if (x - 1.0).abs() >= (y - 1.0).abs() { x } else { y };
                    (x, y) = (uniform, uniform);
                }
                if let Some(target) = self.composition_mut().layer_mut(layer) {
                    target.set_property(SCALE, frame, PropertyValue::Pair(Vec2::new(scale.x * x, scale.y * y)));
                }
            }
            ViewerDrag::Anchor { layer, start, to_local, reach } => {
                let size = self.composition().layer(layer).map_or(Vec2::default(), |layer| layer.size);
                let mut anchor = start + to_local.apply_vector(delta);
                if self.viewer_snapping
                    && let Some(snapped) = snap_anchor(anchor, size, reach)
                {
                    anchor = snapped;
                }
                edit::move_anchor(self.composition_mut(), layer, anchor, frame);
            }
            ViewerDrag::Rotate { starts, pivot, start } => {
                let angle = |v: Vec2| (v.y - pivot.y).atan2(v.x - pivot.x).to_degrees();
                let mut turned = angle(start + delta) - angle(start);
                // The shorter way round, so crossing the left of the pivot doesn't jump a turn.
                turned = (turned + 540.0).rem_euclid(360.0) - 180.0;
                for (id, rotation) in starts {
                    let mut value = rotation + turned;
                    if constrain {
                        value = (value / ROTATION_STEP).round() * ROTATION_STEP;
                    }
                    if let Some(layer) = self.composition_mut().layer_mut(id) {
                        layer.set_property(ROTATION, frame, PropertyValue::Scalar(value));
                    }
                }
            }
            ViewerDrag::PenHandle { index } => {
                // Dragging pulls out both handles, mirrored, so the curve runs smoothly through.
                if let Some(point) = self.pen.as_mut().and_then(|pen| pen.points.get_mut(index)) {
                    *point = PathPoint::smooth(point.at, delta);
                }
            }
            ViewerDrag::PathPoint { layer, index, part, start } => {
                let comp = self.composition();
                let Some(to_local) = world_transform(comp, layer, frame as f64).and_then(|world| world.inverse()) else { return };
                let target = to_local.apply(start + delta);
                // A morphing outline gets a keyframe at the playhead instead of a new box.
                if self.composition().layer(layer).is_some_and(|target| target.is_morphing()) {
                    studio_core::morph::move_morph_point(self.composition_mut(), layer, frame, index, part, target);
                } else {
                    edit::move_path_point(self.composition_mut(), layer, index, part, target);
                }
            }
            ViewerDrag::GradientPoint { layer, slot, end, start } => self.place_gradient_end(layer, slot, end, start + delta),
            ViewerDrag::EffectPoint { layer, effect, point, start } => self.place_effect_point(layer, effect, point, start + delta),
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
        self.frame_snapped = Snapped::default();
    }

    /// The box around the layers being moved, as (top left, bottom right) in composition
    /// pixels, from each layer's real (rotated, parented) corners at the playhead.
    fn selection_box(&self, layers: &[(Id, Vec2, Affine)]) -> Option<(Vec2, Vec2)> {
        let frame = self.current_frame as f64;
        let corners = layers.iter().filter_map(|(id, _, _)| layer_corners(self.composition(), *id, frame)).flatten();
        corners.fold(None, |found, corner| match found {
            None => Some((corner, corner)),
            Some((min, max)) => Some((Vec2::new(min.x.min(corner.x), min.y.min(corner.y)), Vec2::new(max.x.max(corner.x), max.y.max(corner.y)))),
        })
    }
}

/// With Shift held the move stays on its straight line (across or down), so only the axis it
/// runs along may snap. Without Shift, `snapped` is kept as it is.
fn keep_on_line(snapped: Snapped, delta: Vec2, constrained: bool) -> Snapped {
    if !constrained {
        return snapped;
    }
    let across = delta.y == 0.0;
    Snapped {
        nudge: if across { Vec2::new(snapped.nudge.x, 0.0) } else { Vec2::new(0.0, snapped.nudge.y) },
        vertical: snapped.vertical.filter(|_| across),
        horizontal: snapped.horizontal.filter(|_| !across),
        dead_center: false,
    }
}

/// One of an effect's on-screen points, where the viewer draws a handle for it.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectHandle {
    pub effect: Id,
    /// Its place in the effect's list of points (see `studio_core::effects::points`).
    pub point: usize,
    pub label: &'static str,
    /// Where it is, in composition pixels.
    pub at: Vec2,
}

impl EditorState {
    /// The on-screen points of `layer`'s effects that are switched on, at the playhead.
    pub fn effect_handles(&self, layer: Id) -> Vec<EffectHandle> {
        let comp = self.composition();
        let Some(target) = comp.layer(layer).filter(|target| target.switches.effects_enabled) else { return Vec::new() };
        let Some(space) = self.effect_point_space(layer) else { return Vec::new() };
        let at = self.current_frame as f64;
        let mut handles = Vec::new();
        for effect in target.effects.iter().filter(|effect| effect.enabled) {
            for (index, point) in studio_core::effects::points(&effect.name).iter().enumerate() {
                let value = |parameter: usize| effect.properties.get(parameter).map_or(0.0, |property| property.value.value_at(at));
                handles.push(EffectHandle { effect: effect.id, point: index, label: point.label, at: point.position((value(point.x), value(point.y)), space) });
            }
        }
        handles
    }

    /// What `layer`'s effect points are measured from at the playhead: the frame, and the
    /// layer's own corners in it (so Corner Pin starts on the clip, not the frame).
    fn effect_point_space(&self, layer: Id) -> Option<studio_core::effects::PointSpace> {
        let comp = self.composition();
        let layer_corners = studio_core::transform::layer_corners(comp, layer, self.current_frame as f64)?;
        Some(studio_core::effects::PointSpace { frame: Vec2::new(comp.width as f32, comp.height as f32), layer_corners })
    }

    /// Starts dragging one of an effect's on-screen points in the viewer.
    pub fn begin_effect_point_drag(&mut self, layer: Id, effect: Id, point: usize) {
        if self.composition().layer(layer).is_none_or(|target| target.switches.locked) {
            return;
        }
        let Some(start) = self.effect_handles(layer).into_iter().find(|handle| handle.effect == effect && handle.point == point).map(|handle| handle.at) else { return };
        self.viewer_drag = Some(ViewerDrag::EffectPoint { layer, effect, point, start });
    }

    /// Puts one of an effect's on-screen points at `at` (composition pixels), setting the two
    /// settings it is made of (keyframed where they are animated, like a typed value).
    pub(super) fn place_effect_point(&mut self, layer: Id, effect: Id, point: usize, at: Vec2) {
        let comp = self.composition();
        let Some(space) = self.effect_point_space(layer) else { return };
        let Some(name) = comp.layer(layer).and_then(|target| target.effects.iter().find(|candidate| candidate.id == effect)).map(|found| found.name.clone()) else { return };
        let Some(definition) = studio_core::effects::points(&name).get(point).copied() else { return };
        let (x, y) = definition.values(at, space);
        self.set_property_component(layer, PropertyPath::Effect { effect, parameter: definition.x }, 0, x);
        self.set_property_component(layer, PropertyPath::Effect { effect, parameter: definition.y }, 0, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::align::layer_bounds;
    use studio_core::property::TransformProperty;

    #[test]
    fn the_anchor_drags_anywhere_and_snaps_to_the_box_when_close() {
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Rectangle, Vec2::new(500.0, 500.0)).unwrap();
        state.end_viewer_drag();
        // The 300 px box runs from 350 to 650; its anchor starts at the center.
        let corners_before = studio_core::transform::layer_corners(state.composition(), layer, 0.0).unwrap();
        let anchor = |state: &EditorState| state.composition().layer(layer).unwrap().transform.as_ref().unwrap().anchor_point.value_at(0.0);
        state.begin_anchor_drag(layer, 10.0);
        state.drag_in_viewer(Vec2::new(-146.0, -147.0), false);
        assert_eq!(anchor(&state), Vec2::new(0.0, 0.0), "near the top left corner, it snaps there");
        state.drag_in_viewer(Vec2::new(-100.0, 20.0), false);
        assert_eq!(anchor(&state), Vec2::new(50.0, 170.0), "elsewhere, it goes where it is put");
        state.viewer_snapping = false;
        state.drag_in_viewer(Vec2::new(-146.0, -147.0), false);
        assert_eq!(anchor(&state), Vec2::new(4.0, 3.0), "snapping off, it stays where dragged");
        state.end_viewer_drag();
        let corners_after = studio_core::transform::layer_corners(state.composition(), layer, 0.0).unwrap();
        assert_eq!(corners_before, corners_after, "the shape itself never moved");

        state.set_anchor_fraction(layer, (1.0, 1.0));
        assert_eq!(anchor(&state), Vec2::new(300.0, 300.0));
    }

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
    fn the_pen_draws_corners_and_smooth_points_and_closes_on_the_first_point() {
        let mut state = EditorState::new();
        assert_eq!(state.pen_press(Vec2::new(100.0, 100.0), 8.0), None);
        state.end_viewer_drag();
        state.pen_press(Vec2::new(300.0, 100.0), 8.0);
        // Dragging after the press pulls out mirrored handles.
        state.drag_in_viewer(Vec2::new(0.0, 50.0), false);
        state.end_viewer_drag();
        state.pen_press(Vec2::new(200.0, 250.0), 8.0);
        state.end_viewer_drag();
        let pen = state.pen.clone().unwrap();
        assert_eq!(pen.points[1].handle_out, Vec2::new(0.0, 50.0));
        assert_eq!(pen.points[1].handle_in, Vec2::new(0.0, -50.0));
        // Pressing near the first point closes it into a selected shape layer.
        let layer = state.pen_press(Vec2::new(104.0, 103.0), 8.0).expect("closing makes the layer");
        assert!(state.pen.is_none());
        assert_eq!(state.selected_layers, vec![layer]);
        let shown = state.path_points_on_screen(layer);
        assert_eq!(shown.len(), 3);
        assert!((shown[0].at - Vec2::new(100.0, 100.0)).length() < 1e-3, "drawn where it was clicked: {:?}", shown[0].at);
        // A single point is not enough for a shape.
        state.pen_press(Vec2::new(10.0, 10.0), 8.0);
        assert_eq!(state.finish_pen(false), None);
    }

    #[test]
    fn dragging_a_path_point_moves_only_that_point() {
        let mut state = EditorState::new();
        for point in [(100.0, 100.0), (300.0, 100.0), (200.0, 250.0)] {
            state.pen_press(Vec2::new(point.0, point.1), 8.0);
            state.end_viewer_drag();
        }
        let layer = state.finish_pen(true).unwrap();
        state.begin_path_point_drag(layer, 2, PathPart::Point);
        state.drag_in_viewer(Vec2::new(-150.0, 50.0), false);
        state.drag_in_viewer(Vec2::new(-200.0, 100.0), false);
        state.end_viewer_drag();
        let shown = state.path_points_on_screen(layer);
        let near = |a: Vec2, b: Vec2| (a - b).length() < 1e-2;
        assert!(near(shown[2].at, Vec2::new(0.0, 350.0)), "the point follows the pointer: {:?}", shown[2].at);
        assert!(near(shown[0].at, Vec2::new(100.0, 100.0)) && near(shown[1].at, Vec2::new(300.0, 100.0)), "the others stay: {shown:?}");
    }

    #[test]
    fn the_rotate_tool_turns_layers_about_their_anchor() {
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Rectangle, Vec2::new(500.0, 500.0)).unwrap();
        state.end_viewer_drag();
        // From right of the anchor to straight below it: a quarter turn clockwise.
        state.begin_rotate(Vec2::new(600.0, 500.0));
        state.drag_in_viewer(Vec2::new(-100.0, 100.0), false);
        let rotation = |state: &EditorState| state.composition().layer(layer).unwrap().transform.as_ref().unwrap().rotation.value_at(0.0);
        assert!((rotation(&state) - 90.0).abs() < 1e-3, "{}", rotation(&state));
        // Shift keeps it to 15° steps.
        state.drag_in_viewer(Vec2::new(-5.0, 20.0), true);
        assert_eq!(rotation(&state) % 15.0, 0.0);
        state.end_viewer_drag();
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
    fn moved_layers_snap_to_dead_center_and_shift_keeps_to_one_axis() {
        use studio_core::frame_snap::{SnapOptions, Snapped};
        let mut state = EditorState::new();
        let layer = state.begin_shape_draw(ShapeKind::Rectangle, Vec2::new(500.0, 500.0)).unwrap();
        state.end_viewer_drag();
        state.select_layer(layer, false);
        state.frame_snap = SnapOptions { grid: None, center_lines: true, dead_center: true };
        state.frame_snap_reach = 8.0;
        let middle = |state: &EditorState| {
            let corners = layer_corners(state.composition(), layer, 0.0).unwrap();
            (corners[0] + corners[2]) * 0.5
        };
        let center = Vec2::new(state.composition().width as f32, state.composition().height as f32) * 0.5;
        let start = middle(&state);
        let close = |a: Vec2, b: Vec2| (a - b).length() < 0.01;

        state.begin_move();
        state.drag_in_viewer(center - start + Vec2::new(-3.0, 4.0), false);
        assert!(close(middle(&state), center), "dead center: {:?}", middle(&state));
        assert!(state.frame_snapped.dead_center);

        // With Shift the move stays straight across, so only the vertical center line pulls.
        state.drag_in_viewer(Vec2::new(center.x - start.x - 3.0, 1.0), true);
        assert!(close(middle(&state), Vec2::new(center.x, start.y)), "across only: {:?}", middle(&state));
        assert_eq!((state.frame_snapped.vertical, state.frame_snapped.horizontal), (Some(center.x), None));
        state.end_viewer_drag();
        assert_eq!(state.frame_snapped, Snapped::default(), "the guides go when the drag ends");
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

    #[test]
    fn corner_drags_keep_proportions_while_scale_is_linked() {
        let mut state = EditorState::new();
        let solid = state.composition().layers[5].id;
        let scale_after = |state: &mut EditorState, delta: Vec2, shift: bool| {
            state.begin_scale(solid, ScaleHandle::BottomRight);
            state.drag_in_viewer(delta, shift);
            state.end_viewer_drag();
            let value = state.composition().layer(solid).unwrap().property(SCALE, 0).unwrap().value;
            state.undo();
            value
        };
        // Linked (the default): pulling the corner only sideways still scales both ways.
        assert!(state.scale_linked);
        assert_eq!(scale_after(&mut state, Vec2::new(480.0, 0.0), false), PropertyValue::Pair(Vec2::new(150.0, 150.0)));
        // Shift frees it for one drag.
        assert_eq!(scale_after(&mut state, Vec2::new(480.0, 0.0), true), PropertyValue::Pair(Vec2::new(150.0, 100.0)));
        // Unlinked: free, and Shift keeps proportions.
        state.scale_linked = false;
        assert_eq!(scale_after(&mut state, Vec2::new(480.0, 0.0), false), PropertyValue::Pair(Vec2::new(150.0, 100.0)));
        assert_eq!(scale_after(&mut state, Vec2::new(480.0, 0.0), true), PropertyValue::Pair(Vec2::new(150.0, 150.0)));
    }

    #[test]
    fn dragging_an_effect_point_moves_its_settings() {
        let mut state = EditorState::new();
        let a_cam = Id(104);
        state.add_effect(a_cam, "Corner Pin");
        let pin = state.effect_named(a_cam, "Corner Pin").unwrap();
        let handles = state.effect_handles(a_cam);
        assert_eq!(handles.iter().filter(|handle| handle.effect == pin).count(), 4);
        // Each corner starts on the clip's own corner, wherever the clip sits.
        let corners = layer_bounds_corners(&state, a_cam);
        let at = |point: usize| handles.iter().find(|handle| handle.effect == pin && handle.point == point).unwrap().at;
        assert_eq!((at(0), at(1), at(2), at(3)), (corners[0], corners[1], corners[3], corners[2]));
        // The top left corner dragged 100 right and 50 down.
        state.begin_effect_point_drag(a_cam, pin, 0);
        state.drag_in_viewer(Vec2::new(100.0, 50.0), false);
        state.end_viewer_drag();
        let layer = state.composition().layer(a_cam).unwrap();
        let value = |parameter| layer.property(PropertyPath::Effect { effect: pin, parameter }, 0).unwrap().value.component(0);
        assert_eq!((value(0), value(1)), (100.0, 50.0));
        assert_eq!(state.effect_handles(a_cam).iter().find(|handle| handle.effect == pin && handle.point == 0).unwrap().at, corners[0] + Vec2::new(100.0, 50.0));
    }

    /// `layer`'s corners in the composition at the playhead, clockwise from the top left.
    fn layer_bounds_corners(state: &EditorState, layer: Id) -> [Vec2; 4] {
        studio_core::transform::layer_corners(state.composition(), layer, state.current_frame as f64).unwrap()
    }
}
