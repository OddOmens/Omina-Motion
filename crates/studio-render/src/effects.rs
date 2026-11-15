//! Maps catalog effects to GPU passes in `shaders/effect.wgsl`, converting their
//! parameters from composition pixels to output pixels. Effects that are not
//! implemented on the GPU yet return `None`, and the UI leaves them out.

use studio_core::animation::Vec2;
use studio_core::model::Effect;

/// One step of an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pass {
    /// A single shader pass: effect id and up to twelve parameters.
    Shader { id: u32, params: [f32; 12] },
    /// A two-pass Gaussian blur with this sigma, in output pixels.
    Blur { sigma: f32 },
    /// Bright pass, blur, then add back: threshold 0–1, sigma in output pixels, intensity.
    Glow { threshold: f32, sigma: f32, intensity: f32 },
    /// The layer's alpha offset, blurred and placed underneath. Offset and sigma in output pixels.
    Shadow { offset: [f32; 2], opacity: f32, sigma: f32 },
    /// Unsharp mask: the picture blurred with this sigma (output pixels), then the difference
    /// between the picture and the blur added back × `amount` (1 = 100%) wherever it is more
    /// than `threshold` (0–1 of display brightness).
    Unsharp { amount: f32, sigma: f32, threshold: f32 },
    /// The effect's lookup table (`Effect::lut`), mixed in at `intensity` (0–1). Draws nothing
    /// while no table is chosen or it can't be read.
    Lut { intensity: f32 },
}

/// Effects whose controls exist in the catalog but which the GPU does not draw yet. Every
/// catalog effect is drawn today; new ones go here until their shader exists.
const NOT_RENDERED: &[&str] = &[];

/// Whether the renderer draws this catalog effect. Effects on sound are run by the mixer
/// instead, so they don't count.
pub fn is_rendered(name: &str) -> bool {
    studio_core::effects::find(name).is_some() && !NOT_RENDERED.contains(&name) && !studio_core::effects::is_audio(name)
}

fn shader(id: u32, values: &[f32]) -> Pass {
    let mut params = [0.0; 12];
    params[..values.len()].copy_from_slice(values);
    Pass::Shader { id, params }
}

/// The passes for `effect` at `frame`. `scale` is output pixels per composition pixel, and
/// `layer_box` is the layer's own rectangle in output pixels (its corners clockwise from the top
/// left), which effects tied to the layer's shape, like Corner Pin, start from.
/// Returns `None` for effects the GPU does not draw yet.
pub fn passes(effect: &Effect, frame: f64, scale: f32, layer_box: [Vec2; 4]) -> Option<Vec<Pass>> {
    let v = |index: usize| effect.properties.get(index).map_or(0.0, |p| p.value.value_at(frame));
    let px = |index: usize| v(index) * scale;
    let pass = match effect.name.as_str() {
        // AE's Blurriness is roughly twice the Gaussian sigma.
        "Gaussian Blur" => Pass::Blur { sigma: px(0) * 0.5 },
        // A horizontal then a vertical box average per iteration.
        "Box Blur" => {
            let iterations = v(1).round().clamp(1.0, 5.0) as usize;
            let across = shader(87, &[px(0), 1.0, 0.0]);
            let down = shader(87, &[px(0), 0.0, 1.0]);
            return Some([across, down].repeat(iterations));
        }
        "Unsharp Mask" => Pass::Unsharp { amount: v(0) / 100.0, sigma: px(1), threshold: v(2) / 255.0 },
        "Corner Pin" => shader(88, &corner_pin_matrix(layer_box, [(px(0), px(1)), (px(2), px(3)), (px(6), px(7)), (px(4), px(5))])),
        "Lens Distortion" => shader(89, &[v(0)]),
        "Spherize" => shader(90, &[v(0), v(1)]),
        "Channel Mixer" => shader(92, &(0..9).map(v).collect::<Vec<_>>()),
        "Shadow/Highlight" => shader(93, &[v(0), v(1)]),
        "Tritone" => shader(94, &(0..10).map(v).collect::<Vec<_>>()),
        "Selective Color" => shader(95, &(0..12).map(v).collect::<Vec<_>>()),
        "Simple Choker" => shader(96, &[px(0)]),
        "Luma Key" => shader(97, &[v(0), v(1), v(2), v(3)]),
        "Grid" => shader(98, &[px(0), px(1), v(2), v(3), v(4), v(5)]),
        "Checkerboard" => shader(99, &[px(0), v(1), v(2), v(3), v(4)]),
        "4-Color Gradient" => shader(100, &(0..12).map(v).collect::<Vec<_>>()),
        "Light Sweep" => shader(101, &[v(0), v(1), px(2), v(3), v(4)]),
        "Lens Flare" => shader(102, &[v(0), v(1), v(2)]),
        "Directional Blur" => shader(2, &[v(0), px(1)]),
        "Radial Blur" => shader(3, &[v(0)]),
        "Sharpen" => shader(4, &[v(0)]),
        "Brightness & Contrast" => shader(5, &[v(0), v(1)]),
        "Color Balance" => shader(6, &[v(0), v(1), v(2)]),
        "Exposure" => shader(7, &[v(0)]),
        "Hue/Saturation" => shader(8, &[v(0), v(1), v(2)]),
        "Tint" => shader(9, &[v(0)]),
        "Vibrance" => shader(10, &[v(0), v(1)]),
        "Basic Correction" => shader(52, &(0..10).map(v).collect::<Vec<_>>()),
        "Color Wheels" => shader(53, &(0..9).map(v).collect::<Vec<_>>()),
        "Tone Curve" => shader(54, &[v(0), v(1), v(2), v(3)]),
        "Levels" => shader(55, &[v(0), v(1), v(2), v(3), v(4)]),
        "Apply LUT" => Pass::Lut { intensity: v(0) / 100.0 },
        "Bulge" => shader(11, &[v(0), px(1)]),
        "Wave Warp" => shader(12, &[px(0), px(1), v(2)]),
        "Turbulent Displace" => shader(13, &[px(0), px(1)]),
        "Fill" => shader(14, &[v(0)]),
        "Gradient Ramp" => shader(15, &[v(0)]),
        "Fractal Noise" => shader(16, &[v(0), v(1), v(2)]),
        "Add Grain" => shader(17, &[v(0), px(1)]),
        "Film Grain" => shader(103, &[v(0), px(1), v(2), v(3)]),
        "Paper Texture" => shader(104, &[v(0), px(1)]),
        "Old Film" => shader(105, &[v(0), v(1), v(2), v(3)]),
        "Mosaic" => shader(18, &[v(0), v(1)]),
        "Posterize" => shader(19, &[v(0)]),
        "Linear Wipe" => shader(20, &[v(0), v(1), px(2)]),
        "Radial Wipe" => shader(21, &[v(0), v(1), v(2)]),
        "Block Dissolve" => shader(22, &[v(0), px(1)]),
        "Black & White" => shader(40, &[v(0), v(1)]),
        "Invert" => shader(41, &[v(0)]),
        "Vignette" => shader(42, &[v(0), v(1), v(2)]),
        "Letterbox" => shader(43, &[v(0), v(1)]),
        "Light Leak" => shader(44, &[v(0), v(1), v(2)]),
        "Strobe" => shader(45, &[v(0), v(1), v(2)]),
        "Wiggle" => shader(46, &[px(0), v(1), v(2)]),
        "Camera Shake" => shader(47, &[px(0), v(1), v(2), v(3)]),
        "Glitch" => shader(48, &[v(0), px(1), v(2)]),
        "RGB Split" => shader(49, &[px(0), v(1)]),
        "VHS" => shader(50, &[v(0), v(1)]),
        "Scanlines" => shader(51, &[px(0), v(1), v(2)]),
        "Linear Color Key" => shader(61, &[v(0), v(1), v(2)]),
        "Spill Suppressor" => shader(62, &[v(0), v(1)]),
        "Key Cleaner" => shader(63, &[px(0)]),
        "Keyer" => shader(83, &[v(0), v(1), v(2), v(3), v(4), v(5), px(6)]),
        "Median" => shader(64, &[px(0)]),
        "Stroke" => shader(65, &[px(0), v(1)]),
        "Crop" => shader(66, &[v(0), v(1), v(2), v(3), px(4)]),
        "Mirror" => shader(67, &[v(0), v(1)]),
        "Offset" => shader(68, &[v(0), v(1)]),
        // Position in composition pixels becomes output pixels; the rest have no size.
        "Transform" => shader(86, &[px(0), px(1), v(2), v(3), v(4)]),
        "Motion Tile" => shader(69, &[v(0), v(1), v(2)]),
        "Twirl" => shader(70, &[v(0), v(1)]),
        "Ripple" => shader(71, &[px(0), px(1), v(2)]),
        "Kaleidoscope" => shader(72, &[v(0), v(1)]),
        "Polar Coordinates" => shader(73, &[v(0)]),
        "Halftone" => shader(74, &[px(0), v(1)]),
        "Find Edges" => shader(75, &[v(0)]),
        "Emboss" => shader(76, &[v(0), v(1)]),
        "Duotone" => shader(77, &[v(0), v(1), v(2)]),
        "Iris Wipe" => shader(78, &[v(0), px(1)]),
        "Venetian Blinds" => shader(79, &[v(0), v(1), px(2), px(3)]),
        "Gradient Wipe" => shader(80, &[v(0), v(1)]),
        "Pulse" => shader(81, &[v(0), v(1)]),
        "Swing" => shader(82, &[v(0), v(1)]),
        "Glow" => Pass::Glow { threshold: v(0) / 100.0, sigma: px(1) * 0.5, intensity: v(2) },
        "Drop Shadow" => {
            // Direction is measured clockwise from straight up, as in After Effects.
            let (sin, cos) = v(1).to_radians().sin_cos();
            Pass::Shadow { offset: [sin * px(2), -cos * px(2)], opacity: v(0) / 100.0, sigma: px(3) * 0.5 }
        }
        _ => return None,
    };
    Some(vec![pass])
}

/// A 3×3 matrix, rows first.
type Matrix3 = [[f64; 3]; 3];

/// The perspective mapping that takes the unit square's corners (0,0), (1,0), (1,1), (0,1) to
/// `quad`'s four corners in order (Heckbert's closed form). `None` when the quad is flat.
fn square_to_quad(quad: [Vec2; 4]) -> Option<Matrix3> {
    let [p0, p1, p2, p3] = quad.map(|point| (point.x as f64, point.y as f64));
    let (d1, d2) = ((p1.0 - p2.0, p1.1 - p2.1), (p3.0 - p2.0, p3.1 - p2.1));
    let d3 = (p0.0 - p1.0 + p2.0 - p3.0, p0.1 - p1.1 + p2.1 - p3.1);
    let den = d1.0 * d2.1 - d2.0 * d1.1;
    if den.abs() < 1e-9 {
        return None;
    }
    let g = (d3.0 * d2.1 - d2.0 * d3.1) / den;
    let h = (d1.0 * d3.1 - d3.0 * d1.1) / den;
    Some([
        [p1.0 - p0.0 + g * p1.0, p3.0 - p0.0 + h * p3.0, p0.0],
        [p1.1 - p0.1 + g * p1.1, p3.1 - p0.1 + h * p3.1, p0.1],
        [g, h, 1.0],
    ])
}

/// The inverse of `m`, or `None` when it has none.
fn invert(m: Matrix3) -> Option<Matrix3> {
    let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adjugate = [
        [cofactor(1, 2, 1, 2), -cofactor(0, 2, 1, 2), cofactor(0, 1, 1, 2)],
        [-cofactor(1, 2, 0, 2), cofactor(0, 2, 0, 2), -cofactor(0, 1, 0, 2)],
        [cofactor(1, 2, 0, 1), -cofactor(0, 2, 0, 1), cofactor(0, 1, 0, 1)],
    ];
    let det = m[0][0] * adjugate[0][0] + m[0][1] * adjugate[1][0] + m[0][2] * adjugate[2][0];
    (det.abs() > 1e-12).then(|| adjugate.map(|row| row.map(|value| value / det)))
}

fn multiply(a: Matrix3, b: Matrix3) -> Matrix3 {
    std::array::from_fn(|row| std::array::from_fn(|column| (0..3).map(|k| a[row][k] * b[k][column]).sum()))
}

/// Corner Pin's shader parameters: the mapping from each output pixel back to the pixel of the
/// unpinned picture it shows, as three rows of three (in p0.xyz, p1.xyz and p2.xyz). The layer's
/// box (`layer_box`, clockwise from the top left) is stretched onto the same corners moved by
/// `offsets` (output pixels, in the same order). All zeros, which the shader draws as clear,
/// when either shape is flat.
fn corner_pin_matrix(layer_box: [Vec2; 4], offsets: [(f32, f32); 4]) -> [f32; 12] {
    let pinned: [Vec2; 4] = std::array::from_fn(|index| layer_box[index] + Vec2::new(offsets[index].0, offsets[index].1));
    let mut params = [0.0; 12];
    let (Some(from_square), Some(to_pinned)) = (square_to_quad(layer_box), square_to_quad(pinned)) else { return params };
    let Some(from_pinned) = invert(to_pinned) else { return params };
    let back = multiply(from_square, from_pinned);
    for (row, values) in back.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            params[row * 4 + column] = *value as f32;
        }
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    use studio_core::effects::{CATEGORIES, find, instantiate};
    use studio_core::model::Id;

    /// A layer box for effects that don't use it.
    const UNIT_BOX: [Vec2; 4] = [Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 1.0, y: 0.0 }, Vec2 { x: 1.0, y: 1.0 }, Vec2 { x: 0.0, y: 1.0 }];

    #[test]
    fn every_catalog_effect_is_either_rendered_or_listed_as_not_rendered() {
        for definition in CATEGORIES.iter().flat_map(|category| category.effects) {
            let effect = instantiate(definition, Id(1));
            let has_passes = passes(&effect, 0.0, 1.0, UNIT_BOX).is_some();
            assert_eq!(has_passes, is_rendered(definition.name), "{} is inconsistent", definition.name);
        }
        for name in NOT_RENDERED {
            assert!(find(name).is_some(), "{name} is not in the catalog");
        }
    }

    #[test]
    fn lengths_scale_with_the_output() {
        let blur = instantiate(find("Gaussian Blur").unwrap(), Id(1));
        assert_eq!(passes(&blur, 0.0, 0.5, UNIT_BOX).unwrap(), vec![Pass::Blur { sigma: 2.5 }]);
    }

    /// Where Corner Pin's parameters send output pixel `at` back to.
    fn pinned_source(params: &[f32; 12], at: Vec2) -> Vec2 {
        let row = |r: usize| params[r * 4] * at.x + params[r * 4 + 1] * at.y + params[r * 4 + 2];
        Vec2::new(row(0) / row(2), row(1) / row(2))
    }

    #[test]
    fn corner_pin_starts_from_the_layers_own_corners() {
        // A 100 × 50 layer sitting at (20, 30).
        let layer_box = [Vec2::new(20.0, 30.0), Vec2::new(120.0, 30.0), Vec2::new(120.0, 80.0), Vec2::new(20.0, 80.0)];
        let mut pin = instantiate(find("Corner Pin").unwrap(), Id(2));
        let params = |pin: &Effect| match passes(pin, 0.0, 1.0, layer_box).unwrap()[0] {
            Pass::Shader { params, .. } => params,
            other => panic!("Corner Pin is one shader pass, not {other:?}"),
        };
        let unpinned = params(&pin);
        assert!((pinned_source(&unpinned, Vec2::new(70.0, 55.0)) - Vec2::new(70.0, 55.0)).length() < 1e-3, "no offsets change nothing");
        // Moving the top right corner 40 px right: that corner now shows the layer's corner.
        pin.properties[2].value = studio_core::animation::Animated::constant(40.0);
        let moved = params(&pin);
        assert!((pinned_source(&moved, Vec2::new(160.0, 30.0)) - Vec2::new(120.0, 30.0)).length() < 1e-3);
        assert!((pinned_source(&moved, Vec2::new(20.0, 80.0)) - Vec2::new(20.0, 80.0)).length() < 1e-3, "the other corners stay");
        // A layer squashed flat has nothing to pin.
        let flat = [Vec2::new(0.0, 0.0); 4];
        assert!(matches!(passes(&pin, 0.0, 1.0, flat).unwrap()[0], Pass::Shader { params, .. } if params == [0.0; 12]));
    }

    #[test]
    fn box_blur_runs_two_passes_per_iteration() {
        let mut blur = instantiate(find("Box Blur").unwrap(), Id(1));
        assert_eq!(passes(&blur, 0.0, 1.0, UNIT_BOX).unwrap().len(), 6, "three iterations by default");
        blur.properties[1].value = studio_core::animation::Animated::constant(1.0);
        assert_eq!(passes(&blur, 0.0, 1.0, UNIT_BOX).unwrap().len(), 2);
    }
}
