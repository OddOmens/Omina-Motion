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
    // Moving, cutting and repeating the picture without changing its colors.
    EffectCategory {
        name: "Transform",
        effects: &[
            EffectDefinition {
                name: "Crop",
                parameters: &[
                    (param("Left", "%", 0.2, PERCENT.0, PERCENT.1, 1), 0.0),
                    (param("Right", "%", 0.2, PERCENT.0, PERCENT.1, 1), 0.0),
                    (param("Top", "%", 0.2, PERCENT.0, PERCENT.1, 1), 0.0),
                    (param("Bottom", "%", 0.2, PERCENT.0, PERCENT.1, 1), 0.0),
                    (param("Feather", "px", 0.5, 0.0, 1000.0, 1), 0.0),
                ],
            },
            // One side of a line through the center reflected onto the other.
            EffectDefinition { name: "Mirror", parameters: &[(param("Axis Angle", "°", 0.5, -3600.0, 3600.0, 1), 0.0), (param("Center Offset", "%", 0.2, -50.0, 50.0, 1), 0.0)] },
            // Shifts the picture with wraparound, so what leaves one edge comes back at the other.
            EffectDefinition { name: "Offset", parameters: &[(param("Shift X", "%", 0.2, -100.0, 100.0, 1), 0.0), (param("Shift Y", "%", 0.2, -100.0, 100.0, 1), 0.0)] },
            EffectDefinition {
                name: "Motion Tile",
                parameters: &[
                    (param("Tiles Across", "", 0.05, 1.0, 50.0, 1), 3.0),
                    (param("Tiles Down", "", 0.05, 1.0, 50.0, 1), 3.0),
                    (param("Mirror Edges", "", 0.05, 0.0, 1.0, 0), 0.0),
                ],
            },
        ],
    },
    EffectCategory {
        name: "Color Correction",
        effects: &[
            // Lumetri-style basic correction: white balance, then tone, then color.
            EffectDefinition {
                name: "Basic Correction",
                parameters: &[
                    (param("Temperature", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Tint", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Exposure", "", 0.01, -5.0, 5.0, 2), 0.0),
                    (param("Contrast", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Highlights", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Shadows", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Whites", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Blacks", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Saturation", "", 0.5, 0.0, 200.0, 0), 100.0),
                    (param("Vibrance", "", 0.5, -100.0, 100.0, 0), 0.0),
                ],
            },
            // Lift, gamma and gain, each pushed toward a hue and made brighter or darker.
            EffectDefinition {
                name: "Color Wheels",
                parameters: &[
                    (param("Shadows Hue", "°", 0.5, 0.0, 360.0, 0), 0.0),
                    (param("Shadows Amount", "", 0.5, 0.0, 100.0, 0), 0.0),
                    (param("Shadows Brightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Midtones Hue", "°", 0.5, 0.0, 360.0, 0), 0.0),
                    (param("Midtones Amount", "", 0.5, 0.0, 100.0, 0), 0.0),
                    (param("Midtones Brightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Highlights Hue", "°", 0.5, 0.0, 360.0, 0), 0.0),
                    (param("Highlights Amount", "", 0.5, 0.0, 100.0, 0), 0.0),
                    (param("Highlights Brightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                ],
            },
            // A parametric curve, as in Lightroom: each band of brightness lifted or lowered.
            EffectDefinition {
                name: "Tone Curve",
                parameters: &[
                    (param("Highlights", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Lights", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Darks", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Shadows", "", 0.5, -100.0, 100.0, 0), 0.0),
                ],
            },
            EffectDefinition {
                name: "Levels",
                parameters: &[
                    (param("Input Black", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Input White", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Gamma", "", 0.01, 0.1, 10.0, 2), 1.0),
                    (param("Output Black", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Output White", "", 0.5, 0.0, 255.0, 0), 255.0),
                ],
            },
            // A lookup table: a built-in look or a .cube file (`Effect::lut`), mixed in by Intensity.
            EffectDefinition { name: "Apply LUT", parameters: &[(param("Intensity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
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
            // Radius is a percentage of half the frame's shorter side.
            EffectDefinition { name: "Twirl", parameters: &[(param("Angle", "°", 0.5, -3600.0, 3600.0, 1), 90.0), (param("Radius", "%", 0.2, PERCENT.0, PERCENT.1, 1), 50.0)] },
            EffectDefinition {
                name: "Ripple",
                parameters: &[
                    (param("Amplitude", "px", 0.1, 0.0, 1000.0, 1), 10.0),
                    (param("Wavelength", "px", 0.5, 1.0, 4000.0, 0), 40.0),
                    (param("Speed", "", 0.05, -100.0, 100.0, 1), 1.0),
                ],
            },
            EffectDefinition { name: "Kaleidoscope", parameters: &[(param("Segments", "", 0.05, 2.0, 32.0, 0), 6.0), (param("Rotation", "°", 0.5, -3600.0, 3600.0, 1), 0.0)] },
            // Rectangular to polar: the top edge wraps round to the center, the bottom to the rim.
            EffectDefinition { name: "Polar Coordinates", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Generate",
        effects: &[
            EffectDefinition { name: "Fill", parameters: &[(param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 1), 100.0)] },
            EffectDefinition { name: "Gradient Ramp", parameters: &[(param("Ramp Scatter", "", 0.5, 0.0, 256.0, 1), 0.0)] },
            EffectDefinition { name: "Fractal Noise", parameters: &[(param("Contrast", "", 0.5, 0.0, 10000.0, 1), 100.0), (param("Brightness", "", 0.5, -10000.0, 10000.0, 1), 0.0), (param("Evolution", "°", 0.5, -100000.0, 100000.0, 1), 0.0)] },
            // A white outline around the layer's shape, drawn outside its edge.
            EffectDefinition { name: "Stroke", parameters: &[(param("Brush Size", "px", 0.1, 0.0, 200.0, 1), 4.0), (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 1), 100.0)] },
        ],
    },
    EffectCategory {
        name: "Keying",
        effects: &[
            // Removes colors near the key hue (120° is green screen, 240° blue screen).
            EffectDefinition {
                name: "Linear Color Key",
                parameters: &[
                    (param("Key Hue", "°", 0.5, 0.0, 360.0, 0), 120.0),
                    (param("Matching Tolerance", "%", 0.5, PERCENT.0, PERCENT.1, 1), 20.0),
                    (param("Matching Softness", "%", 0.5, PERCENT.0, PERCENT.1, 1), 10.0),
                ],
            },
            // Pulls a key's edge in by the radius and softens it, leaving solid areas alone.
            EffectDefinition { name: "Key Cleaner", parameters: &[(param("Additional Edge Radius", "px", 0.05, 0.0, 100.0, 1), 1.0)] },
            // Takes the screen's color cast out of edges and reflections.
            EffectDefinition { name: "Spill Suppressor", parameters: &[(param("Key Hue", "°", 0.5, 0.0, 360.0, 0), 120.0), (param("Suppression", "", 0.5, 0.0, 200.0, 0), 100.0)] },
            // Green or blue screen in one effect, like Premiere's Ultra Key: the key color (as
            // 0–255 display values, set by the eyedropper in Effect Controls), how close a color
            // must be to vanish, a soft band beyond that, the screen's cast taken out of what
            // stays, and the edge pulled in.
            EffectDefinition {
                name: "Keyer",
                parameters: &[
                    (param("Key Red", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Key Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Key Blue", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Tolerance", "%", 0.5, PERCENT.0, PERCENT.1, 0), 30.0),
                    (param("Softness", "%", 0.5, PERCENT.0, PERCENT.1, 0), 20.0),
                    (param("Spill Suppression", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0),
                    (param("Choke", "px", 0.05, 0.0, 20.0, 1), 0.0),
                ],
            },
        ],
    },
    EffectCategory {
        name: "Noise & Grain",
        effects: &[
            EffectDefinition { name: "Add Grain", parameters: &[(param("Intensity", "", 0.01, 0.0, 10.0, 2), 1.0), (param("Size", "", 0.01, 0.0, 10.0, 2), 1.0)] },
            // The middle value of nine samples spaced by the radius, which removes specks and keeps edges.
            EffectDefinition { name: "Median", parameters: &[(param("Radius", "px", 0.1, 0.0, 100.0, 1), 1.0)] },
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
            // Print-style dots: black on white, larger where the picture is darker.
            EffectDefinition { name: "Halftone", parameters: &[(param("Dot Size", "px", 0.1, 2.0, 500.0, 1), 8.0), (param("Angle", "°", 0.5, -3600.0, 3600.0, 1), 45.0)] },
            EffectDefinition { name: "Find Edges", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0)] },
            // Direction is where the light comes from, clockwise from straight up.
            EffectDefinition { name: "Emboss", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, 500.0, 0), 100.0), (param("Direction", "°", 0.5, -3600.0, 3600.0, 1), 135.0)] },
            // Dark areas take one color and bright areas the other.
            EffectDefinition {
                name: "Duotone",
                parameters: &[
                    (param("Shadow Hue", "°", 0.5, 0.0, 360.0, 0), 275.0),
                    (param("Highlight Hue", "°", 0.5, 0.0, 360.0, 0), 45.0),
                    (param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0),
                ],
            },
        ],
    },
    EffectCategory {
        name: "Motion",
        effects: &[
            EffectDefinition { name: "Wiggle", parameters: &[(param("Amount", "px", 0.5, 0.0, 2000.0, 1), 20.0), (param("Wiggles per Second", "", 0.05, 0.0, 60.0, 1), 3.0), (param("Seed", "", 0.1, 0.0, 1000.0, 0), 1.0)] },
            EffectDefinition { name: "Camera Shake", parameters: &[(param("Amount", "px", 0.5, 0.0, 2000.0, 1), 12.0), (param("Shakes per Second", "", 0.05, 0.0, 60.0, 1), 8.0), (param("Rotation", "°", 0.05, 0.0, 45.0, 1), 1.5), (param("Zoom", "%", 0.1, 0.0, 50.0, 1), 4.0)] },
            // Grows and shrinks smoothly about the center, like a heartbeat.
            EffectDefinition { name: "Pulse", parameters: &[(param("Amount", "%", 0.2, PERCENT.0, PERCENT.1, 1), 10.0), (param("Pulses per Second", "", 0.05, 0.0, 30.0, 2), 1.0)] },
            // Rocks back and forth about the center like a pendulum.
            EffectDefinition { name: "Swing", parameters: &[(param("Angle", "°", 0.1, 0.0, 180.0, 1), 10.0), (param("Swings per Second", "", 0.05, 0.0, 30.0, 2), 0.5)] },
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
            // A circle closing in on the center.
            EffectDefinition { name: "Iris Wipe", parameters: &[(param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0), (param("Feather", "px", 0.5, 0.0, 10000.0, 1), 0.0)] },
            EffectDefinition {
                name: "Venetian Blinds",
                parameters: &[
                    (param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0),
                    (param("Direction", "°", 0.5, -3600.0, 3600.0, 1), 0.0),
                    (param("Width", "px", 0.5, 1.0, 10000.0, 1), 20.0),
                    (param("Feather", "px", 0.5, 0.0, 10000.0, 1), 0.0),
                ],
            },
            // The layer's own brightness sets the order: dark areas go first, bright ones last.
            EffectDefinition { name: "Gradient Wipe", parameters: &[(param("Transition Completion", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0), (param("Softness", "%", 0.5, PERCENT.0, PERCENT.1, 0), 10.0)] },
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
        LayerContent::Text { .. } | LayerContent::Captions { .. } | LayerContent::Shape { .. } | LayerContent::Solid { .. } => !matches!(category.name, "Color Correction" | "Keying"),
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
        lut: None,
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
            // The effect shader takes at most twelve parameters.
            assert!(effect.parameters.len() <= 12, "{} has too many parameters", effect.name);
            let mut parameter_names = std::collections::HashSet::new();
            for (info, default) in effect.parameters {
                assert!(info.min <= *default && *default <= info.max, "{} default out of range", info.name);
                assert!(info.step > 0.0);
                assert!(parameter_names.insert(info.name), "{} has two parameters called {}", effect.name, info.name);
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
    fn grading_effects_suit_footage_and_adjustment_layers_only() {
        for name in ["Basic Correction", "Color Wheels", "Tone Curve", "Levels", "Apply LUT"] {
            assert!(suits_layer(name, &LayerContent::Adjustment), "{name}");
            assert!(suits_layer(name, &LayerContent::Footage { item: Id(1) }), "{name}");
            assert!(!suits_layer(name, &LayerContent::Solid { color: crate::model::Rgba::hex(0) }), "{name}");
        }
        assert!(instantiate(find("Apply LUT").unwrap(), Id(2)).lut.is_none());
    }

    #[test]
    fn instances_start_at_defaults() {
        let blur = instantiate(find("Gaussian Blur").unwrap(), Id(9));
        assert_eq!(blur.properties[0].value.value, 10.0);
    }
}
