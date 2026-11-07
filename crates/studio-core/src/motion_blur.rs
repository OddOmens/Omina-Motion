//! Layer motion blur, as in After Effects: a layer with its Motion Blur switch on is drawn as
//! the average of several moments spread across the camera's shutter, so fast moves smear
//! along their path. The composition sets the shutter (how much of each frame it stays open)
//! and how many moments are averaged. Only the layer's transform (and its parents') is
//! sampled; layers that don't move during the shutter are drawn once, as usual.

use crate::animation::Vec2;
use crate::model::{Composition, Id, Layer, LayerContent, Project};
use crate::transform::{Affine, world_transform};

/// A composition's shutter for motion blur.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MotionBlur {
    /// How long the shutter stays open, in degrees of a frame: 180° is half a frame, 360° the
    /// whole frame, as on a film camera. 0 turns motion blur off.
    pub shutter_angle: f32,
    /// How many moments across the shutter are drawn and averaged.
    pub samples: u32,
}

impl MotionBlur {
    /// The widest shutter accepted (two frames), as in After Effects.
    pub const MAX_SHUTTER_ANGLE: f32 = 720.0;
    /// The fewest and most moments averaged. More look smoother on very fast moves and cost
    /// one extra draw each.
    pub const MIN_SAMPLES: u32 = 2;
    pub const MAX_SAMPLES: u32 = 64;

    /// Whether these are the settings a new composition starts with, which project files leave out.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Checks typed or loaded settings, returning a message suitable for the user.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.shutter_angle.is_finite() && (0.0..=Self::MAX_SHUTTER_ANGLE).contains(&self.shutter_angle)) {
            return Err(format!("Shutter angle must be between 0° and {}°.", Self::MAX_SHUTTER_ANGLE));
        }
        if !(Self::MIN_SAMPLES..=Self::MAX_SAMPLES).contains(&self.samples) {
            return Err(format!("Motion blur samples must be between {} and {}.", Self::MIN_SAMPLES, Self::MAX_SAMPLES));
        }
        Ok(())
    }

    /// The moments (in fractional frames) averaged for the frame at `frame`: evenly spaced
    /// across the shutter, which is centered on the frame. Empty when the shutter is closed.
    pub fn sample_frames(&self, frame: f64) -> Vec<f64> {
        let open = f64::from(self.shutter_angle) / 360.0;
        if open <= 0.0 || self.samples == 0 {
            return Vec::new();
        }
        let count = self.samples;
        (0..count).map(|index| frame + (f64::from(index) + 0.5) / f64::from(count) * open - open * 0.5).collect()
    }
}

impl Default for MotionBlur {
    /// After Effects' defaults are a 180° shutter at 16 samples; 12 keeps playback quicker and
    /// still reads as a smooth streak.
    fn default() -> Self {
        Self { shutter_angle: 180.0, samples: 12 }
    }
}

/// How far (in composition pixels) any corner of a layer must move during the shutter for it to
/// blur. Below a hundredth of a pixel, averaging would change nothing visible.
const STILL: f32 = 0.01;

/// Whether `a` and `b` put every corner of a box of `size` (in the layer's own pixels) in the
/// same place, to within `STILL`.
fn same_place(a: Affine, b: Affine, size: Vec2) -> bool {
    let corners = [Vec2::new(0.0, 0.0), Vec2::new(size.x, 0.0), Vec2::new(0.0, size.y), size];
    corners.iter().all(|&corner| {
        let (from, to) = (a.apply(corner), b.apply(corner));
        (from.x - to.x).abs() <= STILL && (from.y - to.y).abs() <= STILL
    })
}

/// Sets a composition's motion blur shutter, after checking it.
pub fn set_motion_blur(project: &mut Project, composition: Id, settings: MotionBlur) -> Result<(), String> {
    settings.validate()?;
    project.composition_mut(composition).ok_or("The composition no longer exists.")?.motion_blur = settings;
    Ok(())
}

/// The world transforms to average for `layer` at `frame`, or `None` when it is drawn once as
/// usual: its switch is off, the shutter is closed, it has nothing to move (adjustment and
/// sound layers), or it doesn't move while the shutter is open.
pub fn shutter_transforms(composition: &Composition, layer: &Layer, frame: f64) -> Option<Vec<Affine>> {
    if !layer.switches.motion_blur || matches!(layer.content, LayerContent::Adjustment | LayerContent::Audio { .. }) {
        return None;
    }
    let frames = composition.motion_blur.sample_frames(frame);
    let worlds: Vec<Affine> = frames.iter().map(|&at| world_transform(composition, layer.id, at)).collect::<Option<_>>()?;
    let first = *worlds.first()?;
    worlds.iter().any(|world| !same_place(*world, first, layer.size)).then_some(worlds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{Animated, Keyframe};
    use crate::sample::{ids, sample_project};

    #[test]
    fn the_shutter_is_centered_on_the_frame() {
        let half = MotionBlur { shutter_angle: 180.0, samples: 4 };
        assert_eq!(half.sample_frames(10.0), vec![9.8125, 9.9375, 10.0625, 10.1875]);
        assert!(MotionBlur { shutter_angle: 0.0, samples: 8 }.sample_frames(10.0).is_empty(), "a closed shutter blurs nothing");
    }

    #[test]
    fn settings_out_of_range_are_refused() {
        assert!(MotionBlur::default().validate().is_ok());
        assert!(MotionBlur { shutter_angle: 721.0, ..MotionBlur::default() }.validate().is_err());
        assert!(MotionBlur { shutter_angle: f32::NAN, ..MotionBlur::default() }.validate().is_err());
        assert!(MotionBlur { samples: 1, ..MotionBlur::default() }.validate().is_err());
        assert!(MotionBlur { samples: 65, ..MotionBlur::default() }.validate().is_err());
    }

    #[test]
    fn only_moving_layers_with_the_switch_on_are_sampled() {
        let mut composition = sample_project().composition(ids::MAIN_COMP).unwrap().clone();
        let layer = composition.layers[0].id;
        let set_up = |composition: &mut Composition, blur: bool, position: Animated<Vec2>| {
            let target = composition.layer_mut(layer).unwrap();
            target.switches.motion_blur = blur;
            let transform = target.transform.as_mut().unwrap();
            transform.position = position;
            transform.rotation = Animated::constant(0.0);
            transform.scale = Animated::constant(Vec2::new(100.0, 100.0));
        };
        let sweep = Animated::keyframed(vec![Keyframe::linear(0, Vec2::new(0.0, 100.0)), Keyframe::linear(10, Vec2::new(1000.0, 100.0))]);

        set_up(&mut composition, false, sweep.clone());
        assert_eq!(shutter_transforms(&composition, composition.layer(layer).unwrap(), 5.0), None, "switch off");
        set_up(&mut composition, true, Animated::constant(Vec2::new(100.0, 100.0)));
        assert_eq!(shutter_transforms(&composition, composition.layer(layer).unwrap(), 5.0), None, "not moving");

        set_up(&mut composition, true, sweep);
        let worlds = shutter_transforms(&composition, composition.layer(layer).unwrap(), 5.0).expect("a moving layer is sampled");
        assert_eq!(worlds.len(), MotionBlur::default().samples as usize);
        assert!(worlds.last().unwrap().tx > worlds.first().unwrap().tx + 40.0, "100 px a frame, so about 46 px across half a frame");
        assert_eq!(shutter_transforms(&composition, composition.layer(layer).unwrap(), 50.0), None, "held after its last keyframe");
        composition.motion_blur.shutter_angle = 0.0;
        assert_eq!(shutter_transforms(&composition, composition.layer(layer).unwrap(), 5.0), None, "a closed shutter");
    }
}
