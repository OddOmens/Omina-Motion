//! Built-in effects and their parameters. Adding an effect to a layer creates
//! its parameters from this catalog, so every effect has real, editable,
//! keyframeable controls. Rendering the effects is the render engine's job.

use crate::animation::Animated;
use crate::model::{Effect, EffectProperty, Id, LayerContent};
use crate::property::PropertyInfo;

pub struct EffectDefinition {
    pub name: &'static str,
    pub parameters: &'static [(PropertyInfo, f32)],
}

pub struct EffectCategory {
    pub name: &'static str,
    pub effects: &'static [EffectDefinition],
}

/// A parameter's display and range information. `step` is the change per pixel dragged.
const fn param(name: &'static str, unit: &'static str, step: f32, min: f32, max: f32, decimals: usize) -> PropertyInfo {
    PropertyInfo { name, unit, step, min, max, decimals }
}

const PERCENT: (f32, f32) = (0.0, 100.0);

pub const CATEGORIES: &[EffectCategory] = &[
    EffectCategory {
        name: "Blur & Sharpen",
        effects: &[
            EffectDefinition { name: "Gaussian Blur", parameters: &[(param("Blurriness", "", 0.5, 0.0, 1000.0, 1), 10.0)] },
            EffectDefinition {
                name: "Directional Blur",
                parameters: &[(param("Direction", "°", 0.5, -3600.0, 3600.0, 1), 0.0), (param("Blur Length", "", 0.5, 0.0, 1000.0, 1), 10.0)],
            },
            EffectDefinition {
                name: "Radial Blur",
                parameters: &[(param("Amount", "", 0.5, 0.0, 1000.0, 1), 10.0)],
            },
            EffectDefinition {
                name: "Sharpen",
                parameters: &[(param("Sharpen Amount", "", 0.5, 0.0, 500.0, 0), 20.0)],
            },
        ],
    },
    EffectCategory {
        name: "Color Correction",
        effects: &[
            EffectDefinition {
                name: "Brightness & Contrast",
                parameters: &[(param("Brightness", "", 0.5, -150.0, 150.0, 0), 0.0), (param("Contrast", "", 0.5, -100.0, 100.0, 0), 0.0)],
            },
            EffectDefinition {
                name: "Color Balance",
                parameters: &[
                    (param("Shadows", "", 0.5, -100.0, 100.0, 1), 0.0),
                    (param("Midtones", "", 0.5, -100.0, 100.0, 1), 0.0),
                    (param("Highlights", "", 0.5, -100.0, 100.0, 1), 0.0),
                ],
            },
            EffectDefinition { name: "Exposure", parameters: &[(param("Exposure", "", 0.02, -20.0, 20.0, 2), 0.0)] },
            EffectDefinition {
                name: "Hue/Saturation",
                parameters: &[
                    (param("Master Hue", "°", 0.5, -180.0, 180.0, 1), 0.0),
                    (param("Master Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Master Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                ],
            },
            EffectDefinition { name: "Tint", parameters: &[(param("Amount to Tint", "%", 0.5, PERCENT.0, PERCENT.1, 1), 100.0)] },
            EffectDefinition { name: "Black & White", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0), (param("Contrast", "%", 0.5, -100.0, 100.0, 0), 20.0)] },
            EffectDefinition {
                name: "Vibrance",
                parameters: &[(param("Vibrance", "", 0.5, -100.0, 100.0, 0), 0.0), (param("Saturation", "", 0.5, -100.0, 100.0, 0), 0.0)],
            },
        ],
    },
    EffectCategory {
        name: "Distort",
        effects: &[
            EffectDefinition { name: "Bulge", parameters: &[(param("Bulge Height", "", 0.02, -4.0, 4.0, 2), 1.0), (param("Horizontal Radius", "", 1.0, 0.0, 4000.0, 0), 200.0)] },
            EffectDefinition {
                name: "Wave Warp",
                parameters: &[
                    (param("Wave Height", "", 0.5, -10000.0, 10000.0, 0), 10.0),
                    (param("Wave Width", "", 0.5, 1.0, 10000.0, 0), 40.0),
                    (param("Wave Speed", "", 0.05, -100.0, 100.0, 1), 1.0),
                ],
            },
            EffectDefinition { name: "Turbulent Displace", parameters: &[(param("Amount", "", 0.5, -10000.0, 10000.0, 1), 50.0), (param("Size", "", 0.5, 2.0, 10000.0, 1), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Generate",
        effects: &[
            EffectDefinition { name: "Fill", parameters: &[(param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 1), 100.0)] },
            EffectDefinition { name: "Gradient Ramp", parameters: &[(param("Ramp Scatter", "", 0.5, 0.0, 256.0, 1), 0.0)] },
            EffectDefinition { name: "Fractal Noise", parameters: &[(param("Contrast", "", 0.5, 0.0, 10000.0, 1), 100.0), (param("Brightness", "", 0.5, -10000.0, 10000.0, 1), 0.0), (param("Evolution", "°", 0.5, -100000.0, 100000.0, 1), 0.0)] },
            EffectDefinition { name: "Stroke", parameters: &[(param("Brush Size", "", 0.1, 0.0, 1000.0, 1), 2.0), (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 1), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Keying",
        effects: &[
            EffectDefinition { name: "Linear Color Key", parameters: &[(param("Matching Tolerance", "%", 0.5, PERCENT.0, PERCENT.1, 1), 10.0), (param("Matching Softness", "%", 0.5, PERCENT.0, PERCENT.1, 1), 0.0)] },
            EffectDefinition { name: "Key Cleaner", parameters: &[(param("Additional Edge Radius", "", 0.05, 0.0, 100.0, 1), 1.0)] },
            EffectDefinition { name: "Spill Suppressor", parameters: &[(param("Suppression", "", 0.5, 0.0, 200.0, 0), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Noise & Grain",
        effects: &[
            EffectDefinition { name: "Add Grain", parameters: &[(param("Intensity", "", 0.01, 0.0, 10.0, 2), 1.0), (param("Size", "", 0.01, 0.0, 10.0, 2), 1.0)] },
            EffectDefinition { name: "Median", parameters: &[(param("Radius", "", 0.1, 0.0, 255.0, 0), 1.0)] },
        ],
    },
    EffectCategory {
        name: "Stylize",
        effects: &[
            EffectDefinition { name: "Glow", parameters: &[(param("Glow Threshold", "%", 0.5, PERCENT.0, PERCENT.1, 1), 60.0), (param("Glow Radius", "", 0.5, 0.0, 1000.0, 1), 10.0), (param("Glow Intensity", "", 0.02, 0.0, 255.0, 2), 1.0)] },
            EffectDefinition { name: "Drop Shadow", parameters: &[(param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0), (param("Direction", "°", 0.5, -3600.0, 3600.0, 1), 135.0), (param("Distance", "", 0.5, 0.0, 4000.0, 1), 5.0), (param("Softness", "", 0.5, 0.0, 250.0, 1), 0.0)] },
            EffectDefinition { name: "Mosaic", parameters: &[(param("Horizontal Blocks", "", 0.2, 1.0, 4000.0, 0), 10.0), (param("Vertical Blocks", "", 0.2, 1.0, 4000.0, 0), 10.0)] },
            EffectDefinition { name: "Posterize", parameters: &[(param("Level", "", 0.1, 2.0, 255.0, 0), 6.0)] },
            EffectDefinition { name: "Invert", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
            EffectDefinition { name: "Vignette", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 60.0), (param("Size", "%", 0.5, PERCENT.0, PERCENT.1, 0), 60.0), (param("Softness", "%", 0.5, 1.0, PERCENT.1, 0), 50.0)] },
            EffectDefinition { name: "Letterbox", parameters: &[(param("Aspect Ratio", ":1", 0.01, 0.5, 4.0, 2), 2.39), (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
            EffectDefinition { name: "Light Leak", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 60.0), (param("Hue", "°", 0.5, 0.0, 360.0, 0), 25.0), (param("Speed", "", 0.01, 0.0, 10.0, 2), 0.3)] },
            EffectDefinition { name: "Strobe", parameters: &[(param("Flashes per Second", "", 0.05, 0.1, 30.0, 1), 4.0), (param("Flash Length", "%", 0.5, 1.0, PERCENT.1, 0), 30.0), (param("Brightness", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Motion",
        effects: &[
            EffectDefinition { name: "Wiggle", parameters: &[(param("Amount", "px", 0.5, 0.0, 2000.0, 1), 20.0), (param("Wiggles per Second", "", 0.05, 0.0, 60.0, 1), 3.0), (param("Seed", "", 0.1, 0.0, 1000.0, 0), 1.0)] },
            EffectDefinition { name: "Camera Shake", parameters: &[(param("Amount", "px", 0.5, 0.0, 2000.0, 1), 12.0), (param("Shakes per Second", "", 0.05, 0.0, 60.0, 1), 8.0), (param("Rotation", "°", 0.05, 0.0, 45.0, 1), 1.5), (param("Zoom", "%", 0.1, 0.0, 50.0, 1), 4.0)] },
        ],
    },
    EffectCategory {
        name: "Retro & Glitch",
        effects: &[
            EffectDefinition { name: "Glitch", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0), (param("Block Size", "px", 0.5, 2.0, 500.0, 0), 40.0), (param("Changes per Second", "", 0.05, 0.0, 60.0, 1), 10.0)] },
            EffectDefinition { name: "RGB Split", parameters: &[(param("Distance", "px", 0.1, 0.0, 500.0, 1), 8.0), (param("Angle", "°", 0.5, -3600.0, 3600.0, 1), 0.0)] },
            EffectDefinition { name: "VHS", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 60.0), (param("Noise", "%", 0.5, PERCENT.0, PERCENT.1, 0), 30.0)] },
            EffectDefinition { name: "Scanlines", parameters: &[(param("Line Spacing", "px", 0.1, 1.0, 100.0, 1), 4.0), (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 40.0), (param("Scroll Speed", "", 0.05, -100.0, 100.0, 1), 0.0)] },
        ],
    },
    EffectCategory {
        name: "Transition",
        effects: &[
            EffectDefinition { name: "Linear Wipe", parameters: &[(param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0), (param("Wipe Angle", "°", 0.5, -3600.0, 3600.0, 1), 90.0), (param("Feather", "", 0.5, 0.0, 10000.0, 1), 0.0)] },
            EffectDefinition { name: "Radial Wipe", parameters: &[(param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0), (param("Start Angle", "°", 0.5, -3600.0, 3600.0, 1), 0.0), (param("Feather", "", 0.5, 0.0, 10000.0, 1), 0.0)] },
            EffectDefinition { name: "Block Dissolve", parameters: &[(param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0), (param("Block Width", "", 0.5, 1.0, 10000.0, 1), 10.0)] },
        ],
    },
];

pub fn find(name: &str) -> Option<&'static EffectDefinition> {
    CATEGORIES.iter().flat_map(|category| category.effects).find(|effect| effect.name == name)
}

/// The category an effect belongs to.
pub fn category_of(name: &str) -> Option<&'static EffectCategory> {
    CATEGORIES.iter().find(|category| category.effects.iter().any(|effect| effect.name == name))
}

/// Whether effects in `category` are useful on a layer showing `content`. Audio has no picture
/// at all. Text, shapes and solids are flat colours you set directly, so colour correction is
/// pointless on them, and they have no background to key out.
pub fn suits(category: &EffectCategory, content: &LayerContent) -> bool {
    match content {
        LayerContent::Audio { .. } => false,
        LayerContent::Text { .. } | LayerContent::Shape { .. } | LayerContent::Solid { .. } => !matches!(category.name, "Color Correction" | "Keying"),
        LayerContent::Footage { .. } | LayerContent::Image { .. } | LayerContent::Composition { .. } | LayerContent::Adjustment => true,
    }
}

/// Whether the effect called `name` is useful on a layer showing `content` (see `suits`).
pub fn suits_layer(name: &str, content: &LayerContent) -> bool {
    category_of(name).is_some_and(|category| suits(category, content))
}

/// A new effect instance with every parameter at its default value.
pub fn instantiate(definition: &EffectDefinition, id: Id) -> Effect {
    Effect {
        id,
        name: definition.name.to_owned(),
        enabled: true,
        properties: definition
            .parameters
            .iter()
            .map(|&(info, default)| EffectProperty { name: info.name.to_owned(), info, value: Animated::constant(default) })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_effect_has_valid_parameters() {
        let mut names = std::collections::HashSet::new();
        for effect in CATEGORIES.iter().flat_map(|category| category.effects) {
            assert!(names.insert(effect.name), "duplicate effect {}", effect.name);
            assert!(!effect.parameters.is_empty(), "{} has no parameters", effect.name);
            for (info, default) in effect.parameters {
                assert!(info.min <= *default && *default <= info.max, "{} default out of range", info.name);
                assert!(info.step > 0.0);
            }
        }
    }

    #[test]
    fn effects_are_offered_only_where_they_change_something() {
        let text = LayerContent::Text { text: "Hi".into(), style: Default::default() };
        assert!(!suits_layer("Exposure", &text), "colour correction on text");
        assert!(suits_layer("Gaussian Blur", &text));
        assert!(!suits_layer("Gaussian Blur", &LayerContent::Audio { item: Id(1) }));
        assert!(suits_layer("Exposure", &LayerContent::Footage { item: Id(1) }));
        assert!(!suits_layer("No Such Effect", &LayerContent::Adjustment));
    }

    #[test]
    fn instances_start_at_defaults() {
        let blur = instantiate(find("Gaussian Blur").unwrap(), Id(9));
        assert_eq!(blur.properties[0].value.value, 10.0);
    }
}
