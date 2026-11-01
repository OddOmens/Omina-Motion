//! Maps catalog effects to GPU passes in `shaders/effect.wgsl`, converting their
//! parameters from composition pixels to output pixels. Effects that are not
//! implemented on the GPU yet return `None`, and the UI leaves them out.

use studio_core::model::Effect;

/// One step of an effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pass {
    /// A single shader pass: effect id and up to eight parameters.
    Shader { id: u32, params: [f32; 8] },
    /// A two-pass Gaussian blur with this sigma, in output pixels.
    Blur { sigma: f32 },
    /// Bright pass, blur, then add back: threshold 0–1, sigma in output pixels, intensity.
    Glow { threshold: f32, sigma: f32, intensity: f32 },
    /// The layer's alpha offset, blurred and placed underneath. Offset and sigma in output pixels.
    Shadow { offset: [f32; 2], opacity: f32, sigma: f32 },
}

/// Effects whose controls exist in the catalog but which the GPU does not draw yet.
const NOT_RENDERED: &[&str] = &["Stroke", "Linear Color Key", "Key Cleaner", "Spill Suppressor", "Median"];

/// Whether the renderer draws this catalog effect.
pub fn is_rendered(name: &str) -> bool {
    studio_core::effects::find(name).is_some() && !NOT_RENDERED.contains(&name)
}

fn shader(id: u32, values: &[f32]) -> Pass {
    let mut params = [0.0; 8];
    params[..values.len()].copy_from_slice(values);
    Pass::Shader { id, params }
}

/// The passes for `effect` at `frame`. `scale` is output pixels per composition pixel.
/// Returns `None` for effects the GPU does not draw yet.
pub fn passes(effect: &Effect, frame: f64, scale: f32) -> Option<Vec<Pass>> {
    let v = |index: usize| effect.properties.get(index).map_or(0.0, |p| p.value.value_at(frame));
    let px = |index: usize| v(index) * scale;
    let pass = match effect.name.as_str() {
        // AE's Blurriness is roughly twice the Gaussian sigma.
        "Gaussian Blur" => Pass::Blur { sigma: px(0) * 0.5 },
        "Directional Blur" => shader(2, &[v(0), px(1)]),
        "Radial Blur" => shader(3, &[v(0)]),
        "Sharpen" => shader(4, &[v(0)]),
        "Brightness & Contrast" => shader(5, &[v(0), v(1)]),
        "Color Balance" => shader(6, &[v(0), v(1), v(2)]),
        "Exposure" => shader(7, &[v(0)]),
        "Hue/Saturation" => shader(8, &[v(0), v(1), v(2)]),
        "Tint" => shader(9, &[v(0)]),
        "Vibrance" => shader(10, &[v(0), v(1)]),
        "Bulge" => shader(11, &[v(0), px(1)]),
        "Wave Warp" => shader(12, &[px(0), px(1), v(2)]),
        "Turbulent Displace" => shader(13, &[px(0), px(1)]),
        "Fill" => shader(14, &[v(0)]),
        "Gradient Ramp" => shader(15, &[v(0)]),
        "Fractal Noise" => shader(16, &[v(0), v(1), v(2)]),
        "Add Grain" => shader(17, &[v(0), px(1)]),
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

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::effects::{CATEGORIES, find, instantiate};
    use studio_core::model::Id;

    #[test]
    fn every_catalog_effect_is_either_rendered_or_listed_as_not_rendered() {
        for definition in CATEGORIES.iter().flat_map(|category| category.effects) {
            let effect = instantiate(definition, Id(1));
            let has_passes = passes(&effect, 0.0, 1.0).is_some();
            assert_eq!(has_passes, is_rendered(definition.name), "{} is inconsistent", definition.name);
        }
        for name in NOT_RENDERED {
            assert!(find(name).is_some(), "{name} is not in the catalog");
        }
    }

    #[test]
    fn lengths_scale_with_the_output() {
        let blur = instantiate(find("Gaussian Blur").unwrap(), Id(1));
        assert_eq!(passes(&blur, 0.0, 0.5).unwrap(), vec![Pass::Blur { sigma: 2.5 }]);
    }
}
