//! 2D affine transforms: a layer's own transform (anchor point, scale, rotation,
//! position) and its world transform, which includes every parent's transform.
//! The renderer draws with these, and hit-testing, bounds and viewer dragging use them.

use crate::animation::Vec2;
use crate::model::{Composition, Id, Layer, LayerContent};

/// A 2D affine matrix mapping (x, y) to (a·x + c·y + tx, b·x + d·y + ty).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, tx: 0.0, ty: 0.0 };

    pub fn translate(offset: Vec2) -> Self {
        Self { tx: offset.x, ty: offset.y, ..Self::IDENTITY }
    }

    pub fn scale(factor: Vec2) -> Self {
        Self { a: factor.x, d: factor.y, ..Self::IDENTITY }
    }

    /// Clockwise rotation in degrees (y points down, as on screen and in After Effects).
    pub fn rotate(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self { a: cos, b: sin, c: -sin, d: cos, tx: 0.0, ty: 0.0 }
    }

    /// `self` applied after `inner`: (self · inner)(p) = self(inner(p)).
    pub fn then(self, outer: Self) -> Self {
        outer.compose(self)
    }

    fn compose(self, inner: Self) -> Self {
        Self {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    pub fn apply(self, point: Vec2) -> Vec2 {
        Vec2::new(self.a * point.x + self.c * point.y + self.tx, self.b * point.x + self.d * point.y + self.ty)
    }

    /// Applies only the linear part, for directions and offsets.
    pub fn apply_vector(self, vector: Vec2) -> Vec2 {
        Vec2::new(self.a * vector.x + self.c * vector.y, self.b * vector.x + self.d * vector.y)
    }

    /// The inverse transform, or `None` if the matrix squashes everything flat (zero scale).
    pub fn inverse(self) -> Option<Self> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Self { a, b, c, d, tx: -(a * self.tx + c * self.ty), ty: -(b * self.tx + d * self.ty) })
    }
}

/// The layer's own transform at `frame`, from its source pixels to its parent's space
/// (the composition, if it has no parent). `None` for layers without a picture.
pub fn local_transform(layer: &Layer, frame: f64) -> Option<Affine> {
    let transform = layer.transform.as_ref()?;
    let anchor = transform.anchor_point.value_at(frame);
    let scale = transform.scale.value_at(frame) * 0.01;
    Some(
        Affine::translate(Vec2::new(-anchor.x, -anchor.y))
            .then(Affine::scale(scale))
            .then(Affine::rotate(transform.rotation.value_at(frame)))
            .then(Affine::translate(transform.position.value_at(frame))),
    )
}

/// The transform from a layer's source pixels to composition pixels at `frame`, including
/// its parents. Parents without a transform (audio) and parenting cycles are ignored.
pub fn world_transform(composition: &Composition, layer_id: Id, frame: f64) -> Option<Affine> {
    let layer = composition.layer(layer_id)?;
    let mut world = local_transform(layer, frame)?;
    let mut visited = vec![layer_id];
    let mut parent = layer.parent;
    while let Some(parent_id) = parent {
        if visited.contains(&parent_id) {
            break;
        }
        visited.push(parent_id);
        let Some(parent_layer) = composition.layer(parent_id) else { break };
        if let Some(parent_transform) = local_transform(parent_layer, frame) {
            world = world.then(parent_transform);
        }
        parent = parent_layer.parent;
    }
    Some(world)
}

/// The transform from composition pixels into the space a layer's position lives in
/// (its parent's space), so drags in the viewer move parented layers correctly.
pub fn parent_space(composition: &Composition, layer_id: Id, frame: f64) -> Affine {
    let parent = composition.layer(layer_id).and_then(|layer| layer.parent);
    parent
        .and_then(|parent| world_transform(composition, parent, frame))
        .and_then(Affine::inverse)
        .unwrap_or(Affine::IDENTITY)
}

/// The rectangle a layer's content covers at `frame`, in the layer's own pixels: its top-left
/// and bottom-right corners. For most layers that is the layer's box. A pen path whose outline
/// is keyframed keeps one box for all its keyframes, and the outline moves inside and beyond
/// it, so for a path the rectangle is fitted around the outline as it is drawn at `frame`.
pub fn layer_box(layer: &Layer, frame: f64) -> (Vec2, Vec2) {
    let whole = (Vec2::new(0.0, 0.0), layer.size);
    if !matches!(layer.content, LayerContent::Shape { path: Some(_), .. }) {
        return whole;
    }
    let size = layer.size;
    let in_pixels = |v: Vec2| Vec2::new(v.x * size.x, v.y * size.y);
    layer.shape_path_at(frame).and_then(|path| path.bounds()).map_or(whole, |(min, max)| (in_pixels(min), in_pixels(max)))
}

/// Whether composition point `point` lies on the layer's rectangle (see `layer_box`) at `frame`.
pub fn layer_contains(composition: &Composition, layer_id: Id, point: Vec2, frame: f64) -> bool {
    let Some(layer) = composition.layer(layer_id) else { return false };
    let Some(local) = world_transform(composition, layer_id, frame).and_then(Affine::inverse).map(|inv| inv.apply(point)) else {
        return false;
    };
    let (min, max) = layer_box(layer, frame);
    (min.x..=max.x).contains(&local.x) && (min.y..=max.y).contains(&local.y)
}

/// The four corners of the layer's rectangle (see `layer_box`) in composition pixels,
/// clockwise from top-left.
pub fn layer_corners(composition: &Composition, layer_id: Id, frame: f64) -> Option<[Vec2; 4]> {
    let layer = composition.layer(layer_id)?;
    let world = world_transform(composition, layer_id, frame)?;
    let (min, max) = layer_box(layer, frame);
    Some([min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y)].map(|corner| world.apply(corner)))
}

/// The path a layer travels across the composition while its position is animated.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionPath {
    /// Where the layer is, in composition pixels, at steps from its first position keyframe to
    /// its last. Joined up, they draw the path, curved wherever easing or a moving parent bends it.
    pub points: Vec<Vec2>,
    /// Where the layer is at each of its position keyframes.
    pub keyframes: Vec<Vec2>,
}

/// The most steps a motion path is sampled at, however long the move lasts.
const MOTION_PATH_STEPS: i64 = 200;

/// The motion path of a layer whose position has at least two keyframes and actually moves
/// between them. The layer's place is taken at its anchor point, parents included. `None` for
/// a layer that stays put.
pub fn motion_path(composition: &Composition, layer_id: Id) -> Option<MotionPath> {
    let transform = composition.layer(layer_id)?.transform.as_ref()?;
    let frames = transform.position.keyframe_frames();
    let (first, last) = (*frames.first()?, *frames.last()?);
    if frames.len() < 2 {
        return None;
    }
    let place = |frame: f64| world_transform(composition, layer_id, frame).map(|world| world.apply(transform.anchor_point.value_at(frame)));
    // Every frame of a short move; a long one in equal steps, ending exactly on the last keyframe.
    let steps = (last - first).min(MOTION_PATH_STEPS);
    let points: Vec<Vec2> = (0..=steps).filter_map(|step| place(first as f64 + (last - first) as f64 * step as f64 / steps as f64)).collect();
    let start = *points.first()?;
    if points.iter().all(|point| (*point - start).length() < 0.5) {
        return None;
    }
    let keyframes = frames.iter().filter_map(|frame| place(*frame as f64)).collect();
    Some(MotionPath { points, keyframes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Animated;
    use crate::edit::set_parent;
    use crate::sample::{ids, sample_project};

    fn close(a: Vec2, b: Vec2) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn matrices_compose_invert_and_rotate_clockwise() {
        let m = Affine::rotate(90.0).then(Affine::translate(Vec2::new(10.0, 0.0)));
        // Rotating (1, 0) clockwise by 90° on a y-down screen points it down.
        assert!(close(m.apply(Vec2::new(1.0, 0.0)), Vec2::new(10.0, 1.0)));
        let back = m.inverse().unwrap().apply(Vec2::new(10.0, 1.0));
        assert!(close(back, Vec2::new(1.0, 0.0)));
        assert!(Affine::scale(Vec2::new(0.0, 1.0)).inverse().is_none());
    }

    #[test]
    fn a_layer_maps_its_anchor_to_its_position() {
        let comp = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        let logo = &comp.layers[2];
        let world = world_transform(&comp, logo.id, 100.0).unwrap();
        let anchor = logo.transform.as_ref().unwrap().anchor_point.value;
        assert!(close(world.apply(anchor), Vec2::new(960.0, 300.0)));
    }

    #[test]
    fn children_follow_their_parent() {
        let mut comp = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        let (child, parent) = (comp.layers[0].id, comp.layers[5].id);
        // Parent to the full-frame solid, then rotate the solid 90° about its center.
        set_parent(&mut comp, child, Some(parent), 24);
        comp.layer_mut(parent).unwrap().transform.as_mut().unwrap().rotation = Animated::constant(90.0);
        // The child's anchor sat at (960, 620) at frame 24; after the parent turns it orbits the center.
        let child_layer = comp.layer(child).unwrap();
        let anchor = child_layer.transform.as_ref().unwrap().anchor_point.value_at(24.0);
        let moved = world_transform(&comp, child, 24.0).unwrap().apply(anchor);
        assert!(close(moved, Vec2::new(960.0 - 80.0, 540.0)), "got {moved:?}");
    }

    #[test]
    fn hit_testing_respects_rotation() {
        let mut comp = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        let logo = comp.layers[2].id;
        comp.layer_mut(logo).unwrap().transform.as_mut().unwrap().rotation = Animated::constant(90.0);
        // At 50% scale the 1200 × 400 logo becomes 200 wide and 600 tall once turned.
        assert!(layer_contains(&comp, logo, Vec2::new(960.0, 300.0 + 280.0), 100.0));
        assert!(!layer_contains(&comp, logo, Vec2::new(960.0 + 280.0, 300.0), 100.0));
    }

    #[test]
    fn cycles_do_not_hang() {
        let mut comp = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        let (a, b) = (comp.layers[0].id, comp.layers[1].id);
        comp.layer_mut(a).unwrap().parent = Some(b);
        comp.layer_mut(b).unwrap().parent = Some(a);
        assert!(world_transform(&comp, a, 0.0).is_some());
    }

    #[test]
    fn a_layer_with_a_moving_position_has_a_motion_path_through_its_keyframes() {
        let project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        // The title moves up from (960, 620) at frame 24 to (960, 540) at frame 48.
        let path = motion_path(composition, Id(100)).expect("the title moves");
        assert_eq!(path.points.len(), 25, "one point for each frame of the move");
        assert!(close(path.keyframes[0], Vec2::new(960.0, 620.0)) && close(path.keyframes[1], Vec2::new(960.0, 540.0)));
        assert!(close(path.points[0], path.keyframes[0]) && close(*path.points.last().unwrap(), path.keyframes[1]));
        // A layer that isn't animated, or whose keyframes all hold one place, has none.
        assert_eq!(motion_path(composition, Id(105)), None);
        let mut still = composition.clone();
        still.layer_mut(Id(100)).unwrap().transform.as_mut().unwrap().position.map_values(|_| Vec2::new(10.0, 10.0));
        assert_eq!(motion_path(&still, Id(100)), None);
    }
}
