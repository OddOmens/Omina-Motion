//! Stabilize, like a simpler Warp Stabilizer: steadies shaky footage by moving, turning and
//! slightly enlarging the layer against the camera's motion.
//!
//! The camera's motion from one frame to the next is measured elsewhere (`studio_media`'s
//! `measure_motion`) and handed in as `CameraStep`s. Added up, they give the camera's path
//! through the clip; a blurred copy of that path is how a steady camera would have moved.
//! Each frame is then corrected by the difference between the two, written as position (and,
//! when the camera turned, rotation) keyframes on every frame, and the layer is scaled up just
//! enough that the moving edges stay outside the frame. The layer's transform from before is
//! kept, so Remove Stabilization puts it back exactly.

use crate::animation::{Animated, Keyframe, Vec2};
use crate::model::{Composition, Id, LayerContent, Transform};
use crate::time::FrameRate;
use crate::transform::Affine;

/// How the camera moved from one frame to the next, in the layer's source pixels: the picture
/// turned by `degrees` (clockwise) about its center, then shifted by `shift`.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CameraStep {
    pub shift: Vec2,
    pub degrees: f32,
}

/// The camera's measured motion over part of a clip.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeasuredMotion {
    /// The source frame the first step starts from.
    pub first_frame: i64,
    /// Step `i` is the motion from source frame `first_frame + i` to the next.
    pub steps: Vec<CameraStep>,
}

impl MeasuredMotion {
    /// The last source frame the motion covers.
    pub fn last_frame(&self) -> i64 {
        self.first_frame + self.steps.len() as i64
    }
}

/// A stabilized layer's settings, kept on the layer so the smoothness can be changed without
/// measuring again, and so stabilizing can be taken off.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Stabilization {
    /// How smooth the steadied camera is, in percent (see `SMOOTHNESS_CHOICES`).
    pub smoothness: f32,
    pub motion: MeasuredMotion,
    /// How much the layer was scaled up to keep its edges out of the frame (1 is not at all).
    pub zoom: f32,
    /// The layer's transform before it was stabilized.
    pub original: Transform,
}

impl Stabilization {
    /// Whether the settings are usable (for checking project files).
    pub fn is_sound(&self) -> bool {
        smoothness_is_valid(self.smoothness)
            && self.zoom.is_finite()
            && (1.0..=MAX_ZOOM).contains(&self.zoom)
            && self.motion.steps.iter().all(|step| step.shift.x.is_finite() && step.shift.y.is_finite() && step.degrees.is_finite())
    }
}

/// The smoothness choices offered, in percent. 100% evens out the camera's path over about a
/// second either side of each frame; 10% only takes out fast jitter.
pub const SMOOTHNESS_CHOICES: [f32; 5] = [10.0, 25.0, 50.0, 75.0, 100.0];
/// The smoothness a first Stabilize uses.
pub const DEFAULT_SMOOTHNESS: f32 = 50.0;
/// The largest smoothness accepted, in percent.
const MAX_SMOOTHNESS: f32 = 200.0;
/// The most the layer is ever scaled up. Past this the picture would get too soft, so a few
/// edges are allowed to show instead.
pub const MAX_ZOOM: f32 = 1.5;
/// Corrections smaller than this (in degrees) on every frame count as no turn, so a layer
/// filmed level keeps its rotation untouched.
const MIN_ROTATION: f32 = 0.01;

fn smoothness_is_valid(smoothness: f32) -> bool {
    smoothness.is_finite() && smoothness > 0.0 && smoothness <= MAX_SMOOTHNESS
}

/// Why a layer could not be stabilized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StabilizeError {
    /// Only video layers (with a transform) can be stabilized.
    NotFootage,
    /// The smoothness must be above 0% and at most 200%.
    BadSmoothness,
    /// No motion was measured (a clip of a single frame).
    NoMotion,
}

impl std::fmt::Display for StabilizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFootage => "Only video layers can be stabilized.",
            Self::BadSmoothness => "Smoothness must be above 0% and at most 200%.",
            Self::NoMotion => "There was no motion to measure: the clip needs at least two frames.",
        })
    }
}

/// What stabilizing did: how much the layer was scaled up, and whether some edges still show
/// because hiding them all would have needed more than `MAX_ZOOM`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stabilized {
    pub zoom: f32,
    pub edges_show: bool,
}

/// Where the camera is at one frame, relative to the first: turned by `degrees` about the
/// picture's center, then shifted by `offset`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraPose {
    pub degrees: f32,
    pub offset: Vec2,
}

/// Turns `vector` clockwise by `degrees` (y points down).
fn turn(vector: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Vec2::new(cos * vector.x - sin * vector.y, sin * vector.x + cos * vector.y)
}

/// The camera's pose at each frame, from its steps: one more pose than steps, the first at
/// rest. Turns about the same center add up; each step's turn also swings the shift so far.
pub fn camera_path(steps: &[CameraStep]) -> Vec<CameraPose> {
    let mut path = Vec::with_capacity(steps.len() + 1);
    let mut pose = CameraPose::default();
    path.push(pose);
    for step in steps {
        pose = CameraPose { degrees: pose.degrees + step.degrees, offset: turn(pose.offset, step.degrees) + step.shift };
        path.push(pose);
    }
    path
}

/// A smoothed copy of `path`: each pose is a Gaussian-weighted average of the poses within
/// three `sigma` frames of it (fewer near the ends, where the weights are rescaled).
pub fn smooth_path(path: &[CameraPose], sigma: f32) -> Vec<CameraPose> {
    if sigma <= 0.0 || path.len() < 2 {
        return path.to_vec();
    }
    let reach = (sigma * 3.0).ceil() as usize;
    let weights: Vec<f32> = (0..=reach).map(|distance| (-0.5 * (distance as f32 / sigma).powi(2)).exp()).collect();
    (0..path.len())
        .map(|index| {
            let (from, to) = (index.saturating_sub(reach), (index + reach).min(path.len() - 1));
            let (mut total, mut degrees, mut offset) = (0.0, 0.0, Vec2::default());
            for (other, pose) in path.iter().enumerate().take(to + 1).skip(from) {
                let weight = weights[index.abs_diff(other)];
                total += weight;
                degrees += pose.degrees * weight;
                offset = offset + pose.offset * weight;
            }
            CameraPose { degrees: degrees / total, offset: offset * (1.0 / total) }
        })
        .collect()
}

/// The correction for each frame, in the layer's source pixels: what moves the picture from
/// where the shaky camera put it to where the smoothed camera would have. `center` is the
/// picture's center, which the measured turns are about.
fn corrections(path: &[CameraPose], smoothed: &[CameraPose], center: Vec2) -> Vec<Affine> {
    path.iter()
        .zip(smoothed)
        .map(|(actual, steady)| {
            // A point shown at q by the shaky camera belongs at steady(actual⁻¹(q)), which is a
            // turn by the difference in angle about the center plus a shift.
            let degrees = steady.degrees - actual.degrees;
            let shift = steady.offset - turn(actual.offset, degrees);
            Affine::translate(Vec2::new(-center.x, -center.y)).then(Affine::rotate(degrees)).then(Affine::translate(center + shift))
        })
        .collect()
}

/// Scaling by `zoom` about `anchor`.
fn zoom_about(anchor: Vec2, zoom: f32) -> Affine {
    Affine::translate(Vec2::new(-anchor.x, -anchor.y)).then(Affine::scale(Vec2::new(zoom, zoom))).then(Affine::translate(anchor))
}

/// The smallest scale-up (about `anchor`) at which every corrected frame still covers the
/// whole `size` rectangle, so no edge of the picture comes into view; capped at `MAX_ZOOM`.
fn cover_zoom(corrections: &[Affine], size: Vec2, anchor: Vec2) -> Stabilized {
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(size.x, 0.0), Vec2::new(0.0, size.y), size];
    // A little slack, so rounding doesn't call an exact fit a gap.
    let inside = |point: Vec2| (-0.01..=size.x + 0.01).contains(&point.x) && (-0.01..=size.y + 0.01).contains(&point.y);
    let covers = |zoom: f32| {
        corrections.iter().all(|correction| {
            // Which picture point ends up at each corner of the frame; it must exist.
            correction.then(zoom_about(anchor, zoom)).inverse().is_some_and(|back| corners.iter().all(|&corner| inside(back.apply(corner))))
        })
    };
    if covers(1.0) {
        return Stabilized { zoom: 1.0, edges_show: false };
    }
    if !covers(MAX_ZOOM) {
        return Stabilized { zoom: MAX_ZOOM, edges_show: true };
    }
    // Halving the range each time; 20 rounds leave it under a millionth wide.
    let (mut low, mut high) = (1.0, MAX_ZOOM);
    for _ in 0..20 {
        let middle = (low + high) * 0.5;
        if covers(middle) { high = middle } else { low = middle }
    }
    Stabilized { zoom: high, edges_show: false }
}

/// Stabilizes footage layer `layer` from `motion` (measured on its source, which plays at
/// `source_rate`), with `smoothness` in percent. A layer already stabilized starts again from
/// its transform from before. One edit, so one undo step.
pub fn stabilize(composition: &mut Composition, layer: Id, motion: MeasuredMotion, smoothness: f32, source_rate: FrameRate) -> Result<Stabilized, StabilizeError> {
    if !smoothness_is_valid(smoothness) {
        return Err(StabilizeError::BadSmoothness);
    }
    if motion.steps.is_empty() {
        return Err(StabilizeError::NoMotion);
    }
    let comp_rate = composition.frame_rate;
    let found = composition.layer_mut(layer).filter(|found| matches!(found.content, LayerContent::Footage { .. })).ok_or(StabilizeError::NotFootage)?;
    let original = match found.stabilization.take() {
        Some(earlier) => earlier.original,
        None => found.transform.clone().ok_or(StabilizeError::NotFootage)?,
    };
    let size = found.size;
    let center = size * 0.5;
    // The smoothness is a time, so it is counted in source frames: 100% is a second.
    let sigma = smoothness / 100.0 * source_rate.fps() as f32;
    let path = camera_path(&motion.steps);
    let fixes = corrections(&path, &smooth_path(&path, sigma), center);
    // The anchor point is rarely animated; where it is on the first frame is close enough for
    // finding how much to scale up.
    let anchor = original.anchor_point.value_at(found.in_frame as f64);
    let fit = cover_zoom(&fixes, size, anchor);

    // Every composition frame of the layer gets its corrected position and rotation.
    let mut positions = Vec::new();
    let mut rotations = Vec::new();
    let mut turns = false;
    for frame in found.in_frame..found.out_frame {
        let source = found.source_frame(frame, comp_rate, source_rate).clamp(motion.first_frame, motion.last_frame());
        let fix = fixes[(source - motion.first_frame) as usize];
        let at = frame as f64;
        let (anchor, scale, rotation) = (original.anchor_point.value_at(at), original.scale.value_at(at), original.rotation.value_at(at));
        let layer_to_parent = Affine::translate(Vec2::new(-anchor.x, -anchor.y))
            .then(Affine::scale(scale * 0.01))
            .then(Affine::rotate(rotation))
            .then(Affine::translate(original.position.value_at(at)));
        // The correction comes first, in the picture's own pixels, then the scale-up about the
        // anchor, then the layer's own transform. The anchor ends up at the new position, and
        // the correction's turn adds to the rotation (subtracts, if the layer is mirrored; with
        // unequal width and height scales the turn is only approximate).
        let moved = fix.then(zoom_about(anchor, fit.zoom)).then(layer_to_parent);
        let fix_degrees = fix.b.atan2(fix.a).to_degrees();
        let mirrored = scale.x * scale.y < 0.0;
        turns |= fix_degrees.abs() > MIN_ROTATION;
        positions.push(Keyframe::linear(frame, moved.apply(anchor)));
        rotations.push(Keyframe::linear(frame, rotation + if mirrored { -fix_degrees } else { fix_degrees }));
    }
    let mut transform = original.clone();
    if !positions.is_empty() {
        transform.position = Animated::keyframed(positions);
    }
    if turns && !rotations.is_empty() {
        transform.rotation = Animated::keyframed(rotations);
    }
    transform.scale.map_values(|scale| scale * fit.zoom);
    found.transform = Some(transform);
    found.stabilization = Some(Stabilization { smoothness, motion, zoom: fit.zoom, original });
    Ok(fit)
}

/// Stabilizes `layer` again with a new `smoothness`, from the motion already measured.
/// `Ok(None)` if the layer isn't stabilized.
pub fn set_smoothness(composition: &mut Composition, layer: Id, smoothness: f32, source_rate: FrameRate) -> Result<Option<Stabilized>, StabilizeError> {
    let Some(motion) = composition.layer(layer).and_then(|found| found.stabilization.as_ref()).map(|stabilization| stabilization.motion.clone()) else {
        return Ok(None);
    };
    stabilize(composition, layer, motion, smoothness, source_rate).map(Some)
}

/// Takes stabilizing off `layer`, putting its transform back exactly as it was. Returns
/// whether it was stabilized.
pub fn remove(composition: &mut Composition, layer: Id) -> bool {
    let Some(found) = composition.layer_mut(layer) else { return false };
    let Some(stabilization) = found.stabilization.take() else { return false };
    found.transform = Some(stabilization.original);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};
    use crate::transform::world_transform;

    /// Repeatable shake: a made-up random sequence in -1..1.
    fn shake(index: usize) -> f32 {
        let hash = (index as u32).wrapping_mul(2_654_435_761).rotate_left(13).wrapping_mul(2_246_822_519);
        (hash >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
    }

    /// A slow pan right with up to `amount` pixels of shake added to each step.
    fn shaky_pan(frames: usize, amount: f32) -> Vec<CameraStep> {
        (0..frames).map(|index| CameraStep { shift: Vec2::new(2.0 + shake(index) * amount, shake(index + 1000) * amount), degrees: shake(index + 2000) * 0.2 }).collect()
    }

    /// How jumpy a sequence of positions is: the average size of its second differences
    /// (zero for anything moving at a steady speed).
    fn jitter(points: &[Vec2]) -> f32 {
        let jumps: Vec<f32> = points.windows(3).map(|three| (three[2] - three[1] - (three[1] - three[0])).length()).collect();
        jumps.iter().sum::<f32>() / jumps.len() as f32
    }

    /// The sample project's video layer (`A-cam_01.mov`), playing at 24 fps from frame 0, and
    /// the motion of a camera over all of its frames.
    fn shaky_clip() -> (Composition, Id, MeasuredMotion) {
        let project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap().clone();
        let clip = composition.layers.iter().find(|layer| matches!(layer.content, LayerContent::Footage { item } if item == ids::A_CAM)).unwrap();
        let frames = (clip.out_frame - clip.in_frame) as usize;
        let id = clip.id;
        (composition, id, MeasuredMotion { first_frame: 0, steps: shaky_pan(frames, 6.0) })
    }

    const RATE: FrameRate = FrameRate::new(24, 1);

    #[test]
    fn smoothing_takes_out_jitter_but_keeps_the_pan() {
        let path = camera_path(&shaky_pan(120, 5.0));
        let smoothed = smooth_path(&path, 12.0);
        let offsets = |poses: &[CameraPose]| poses.iter().map(|pose| pose.offset).collect::<Vec<_>>();
        let (before, after) = (jitter(&offsets(&path)), jitter(&offsets(&smoothed)));
        assert!(after < before * 0.1, "jitter {before} → {after}");
        // Still panning right at about 2 px a frame through the middle.
        let speed = (smoothed[80].offset.x - smoothed[40].offset.x) / 40.0;
        assert!((speed - 2.0).abs() < 0.3, "{speed}");
    }

    #[test]
    fn a_camera_path_adds_up_turns_and_shifts() {
        let path = camera_path(&[CameraStep { shift: Vec2::new(10.0, 0.0), degrees: 0.0 }, CameraStep { shift: Vec2::new(0.0, 0.0), degrees: 90.0 }]);
        assert_eq!(path.len(), 3);
        assert_eq!(path[1].offset, Vec2::new(10.0, 0.0));
        // A quarter turn clockwise (y down) swings the earlier shift from right to down.
        assert!((path[2].offset - Vec2::new(0.0, 10.0)).length() < 1e-4, "{:?}", path[2].offset);
        assert_eq!(path[2].degrees, 90.0);
    }

    #[test]
    fn a_stabilized_clip_holds_still_on_screen_and_is_scaled_to_hide_the_edges() {
        let (mut composition, clip, motion) = shaky_clip();
        let path = camera_path(&motion.steps);
        let result = stabilize(&mut composition, clip, motion.clone(), 50.0, RATE).unwrap();
        assert!(result.zoom > 1.0 && !result.edges_show, "{result:?}");
        let layer = composition.layer(clip).unwrap();
        // Something fixed in the scene, at the middle of the first frame, sits at a point of the
        // picture that moves with the camera; the screen shows that point where the layer puts it.
        let center = layer.size * 0.5;
        let on_screen = |stabilized: &Composition, frame: i64| {
            // The turns are about the center, so only the shift moves it.
            let in_picture = center + path[frame as usize].offset;
            world_transform(stabilized, clip, frame as f64).unwrap().apply(in_picture)
        };
        let steadied: Vec<Vec2> = (0..composition.layer(clip).unwrap().out_frame).map(|frame| on_screen(&composition, frame)).collect();
        let (original_composition, _, _) = shaky_clip();
        let shaky: Vec<Vec2> = (0..original_composition.layer(clip).unwrap().out_frame).map(|frame| on_screen(&original_composition, frame)).collect();
        let (before, after) = (jitter(&shaky), jitter(&steadied));
        assert!(after < before * 0.15, "jitter on screen {before} → {after}");
        // Keyframes on every frame of the clip, for position and (as the camera turned) rotation.
        let transform = layer.transform.as_ref().unwrap();
        assert_eq!(transform.position.keyframes().len() as i64, layer.out_frame - layer.in_frame);
        assert!(transform.rotation.is_animated());
        assert_eq!(transform.scale.value, Vec2::new(50.0, 50.0) * result.zoom);
        // The frame's corners always show picture: none of the layer's edges come into view.
        let fixes = corrections(&path, &smooth_path(&path, 12.0), center);
        assert!(!cover_zoom(&fixes, layer.size, center).edges_show);
    }

    #[test]
    fn removing_stabilization_puts_the_layer_back_exactly() {
        let (mut composition, clip, motion) = shaky_clip();
        let before = composition.layer(clip).unwrap().clone();
        stabilize(&mut composition, clip, motion, 50.0, RATE).unwrap();
        assert_ne!(composition.layer(clip).unwrap(), &before);
        // Changing the smoothness re-stabilizes from the original, not on top of itself.
        let smoother = set_smoothness(&mut composition, clip, 100.0, RATE).unwrap().unwrap();
        assert_eq!(composition.layer(clip).unwrap().stabilization.as_ref().unwrap().original, before.transform.clone().unwrap());
        assert!(smoother.zoom >= 1.0);
        assert!(remove(&mut composition, clip));
        assert_eq!(composition.layer(clip).unwrap(), &before);
        assert!(!remove(&mut composition, clip), "nothing left to remove");
        assert_eq!(set_smoothness(&mut composition, clip, 50.0, RATE), Ok(None));
    }

    #[test]
    fn a_level_camera_keeps_the_layers_rotation() {
        let (mut composition, clip, mut motion) = shaky_clip();
        for step in &mut motion.steps {
            step.degrees = 0.0;
        }
        stabilize(&mut composition, clip, motion, 50.0, RATE).unwrap();
        assert!(!composition.layer(clip).unwrap().transform.as_ref().unwrap().rotation.is_animated());
    }

    #[test]
    fn only_footage_with_motion_and_a_sensible_smoothness_is_stabilized() {
        let (mut composition, clip, motion) = shaky_clip();
        let solid = composition.layers.iter().find(|layer| matches!(layer.content, LayerContent::Solid { .. })).unwrap().id;
        assert_eq!(stabilize(&mut composition, solid, motion.clone(), 50.0, RATE), Err(StabilizeError::NotFootage));
        for smoothness in [0.0, -5.0, f32::NAN, 500.0] {
            assert_eq!(stabilize(&mut composition, clip, motion.clone(), smoothness, RATE), Err(StabilizeError::BadSmoothness));
        }
        assert_eq!(stabilize(&mut composition, clip, MeasuredMotion::default(), 50.0, RATE), Err(StabilizeError::NoMotion));
        assert!(composition.layer(clip).unwrap().stabilization.is_none(), "a refused stabilize changes nothing");
    }

    #[test]
    fn a_steady_camera_needs_no_scaling() {
        let (mut composition, clip, _) = shaky_clip();
        let still = MeasuredMotion { first_frame: 0, steps: vec![CameraStep::default(); 40] };
        let result = stabilize(&mut composition, clip, still, 50.0, RATE).unwrap();
        assert_eq!(result, Stabilized { zoom: 1.0, edges_show: false });
    }
}
