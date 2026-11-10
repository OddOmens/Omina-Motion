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
            // An even average of the pixels within the radius, repeated Iterations times; more
            // iterations round the square blur into a smooth one.
            EffectDefinition {
                name: "Box Blur",
                parameters: &[(param("Blur Radius", "px", 0.2, 0.0, 500.0, 1), 10.0), (param("Iterations", "", 0.05, 1.0, 5.0, 0), 3.0)],
            },
            // Sharpens by adding back the difference between the picture and a blurred copy of
            // it. Threshold (0-255 display levels) leaves smooth areas, where that difference is
            // small, untouched so grain and noise aren't sharpened.
            EffectDefinition {
                name: "Unsharp Mask",
                parameters: &[
                    (param("Amount", "%", 0.5, 0.0, 500.0, 0), 80.0),
                    (param("Radius", "px", 0.05, 0.1, 250.0, 1), 3.0),
                    (param("Threshold", "", 0.5, 0.0, 255.0, 0), 0.0),
                ],
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
            // Moves, scales, turns and fades the picture, like After Effects' Transform effect:
            // each setting keyframes, and in a node graph it moves just one branch. It turns and
            // scales about the middle of the frame.
            EffectDefinition {
                name: "Transform",
                parameters: &[
                    (param("Position X", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Position Y", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Scale", "%", 0.2, 0.0, 1000.0, 1), 100.0),
                    (param("Rotation", "°", 0.5, -3600.0, 3600.0, 1), 0.0),
                    (param("Opacity", "%", 0.2, PERCENT.0, PERCENT.1, 1), 100.0),
                ],
            },
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
            // Builds each output channel from a mix of the input's red, green and blue (in %).
            // The defaults leave the picture as it is; putting 100% of red into green, say,
            // copies the red channel into green.
            EffectDefinition {
                name: "Channel Mixer",
                parameters: &[
                    (param("Red-Red", "%", 0.5, -200.0, 200.0, 0), 100.0),
                    (param("Red-Green", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Red-Blue", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Green-Red", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Green-Green", "%", 0.5, -200.0, 200.0, 0), 100.0),
                    (param("Green-Blue", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Blue-Red", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Blue-Green", "%", 0.5, -200.0, 200.0, 0), 0.0),
                    (param("Blue-Blue", "%", 0.5, -200.0, 200.0, 0), 100.0),
                ],
            },
            // Brings out detail in dark areas and pulls back bright ones, leaving black, white and
            // the colors' hues where they are.
            EffectDefinition {
                name: "Shadow/Highlight",
                parameters: &[(param("Shadow Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 35.0), (param("Highlight Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0)],
            },
            // Maps the picture's brightness onto three colors (0-255 display values): shadows,
            // midtones and highlights. Blend With Original brings the picture back in.
            EffectDefinition {
                name: "Tritone",
                parameters: &[
                    (param("Highlights Red", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Highlights Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Highlights Blue", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Midtones Red", "", 0.5, 0.0, 255.0, 0), 127.0),
                    (param("Midtones Green", "", 0.5, 0.0, 255.0, 0), 100.0),
                    (param("Midtones Blue", "", 0.5, 0.0, 255.0, 0), 70.0),
                    (param("Shadows Red", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Shadows Green", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Shadows Blue", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Blend With Original", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0),
                ],
            },
            // Changes the saturation and lightness of one family of colors at a time (reds,
            // yellows, greens, cyans, blues or magentas), leaving the others and grays alone.
            EffectDefinition {
                name: "Selective Color",
                parameters: &[
                    (param("Reds Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Reds Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Yellows Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Yellows Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Greens Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Greens Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Cyans Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Cyans Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Blues Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Blues Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Magentas Saturation", "", 0.5, -100.0, 100.0, 0), 0.0),
                    (param("Magentas Lightness", "", 0.5, -100.0, 100.0, 0), 0.0),
                ],
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
            // Moves each corner of the frame by its offset (in pixels) and stretches the picture
            // onto the new four-sided shape in perspective. Outside the shape is clear.
            EffectDefinition {
                name: "Corner Pin",
                parameters: &[
                    (param("Top Left X", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Top Left Y", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Top Right X", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Top Right Y", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Bottom Left X", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Bottom Left Y", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Bottom Right X", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                    (param("Bottom Right Y", "px", 0.5, -10000.0, 10000.0, 1), 0.0),
                ],
            },
            // A camera lens's bending: positive curvature bows straight lines outward (barrel),
            // negative pinches them in (pincushion).
            EffectDefinition { name: "Lens Distortion", parameters: &[(param("Curvature", "%", 0.5, -100.0, 100.0, 0), 30.0)] },
            // Wraps the middle of the picture around a ball: positive amounts swell it toward
            // you, negative ones push it away. Radius is a percentage of half the frame's shorter side.
            EffectDefinition { name: "Spherize", parameters: &[(param("Amount", "%", 0.5, -100.0, 100.0, 0), 50.0), (param("Radius", "%", 0.2, PERCENT.0, PERCENT.1, 1), 50.0)] },
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
            // Lines of one color (0-255 display values) every Size pixels across and down,
            // centered on the frame and drawn over the layer.
            EffectDefinition {
                name: "Grid",
                parameters: &[
                    (param("Size", "px", 0.5, 2.0, 2000.0, 0), 40.0),
                    (param("Line Width", "px", 0.1, 0.0, 500.0, 1), 2.0),
                    (param("Color Red", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Color Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Color Blue", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0),
                ],
            },
            // Squares of Size pixels, centered on the frame, every other one filled with the color
            // (0-255 display values) and the rest showing the layer.
            EffectDefinition {
                name: "Checkerboard",
                parameters: &[
                    (param("Size", "px", 0.5, 2.0, 2000.0, 0), 40.0),
                    (param("Color Red", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Color Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Color Blue", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Opacity", "%", 0.5, PERCENT.0, PERCENT.1, 0), 100.0),
                ],
            },
            // A smooth blend between four colors (0-255 display values), one in each corner.
            EffectDefinition {
                name: "4-Color Gradient",
                parameters: &[
                    (param("Top Left Red", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Top Left Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Top Left Blue", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Top Right Red", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Top Right Green", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Top Right Blue", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Bottom Left Red", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Bottom Left Green", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Bottom Left Blue", "", 0.5, 0.0, 255.0, 0), 255.0),
                    (param("Bottom Right Red", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Bottom Right Green", "", 0.5, 0.0, 255.0, 0), 0.0),
                    (param("Bottom Right Blue", "", 0.5, 0.0, 255.0, 0), 255.0),
                ],
            },
            // A band of light across the layer. Position runs from one side of the frame (0%) to
            // the other (100%), so keyframing it sweeps the shine across; Angle tilts the band.
            EffectDefinition {
                name: "Light Sweep",
                parameters: &[
                    (param("Position", "%", 0.2, -100.0, 200.0, 1), 50.0),
                    (param("Angle", "°", 0.5, -3600.0, 3600.0, 1), 30.0),
                    (param("Width", "px", 0.5, 1.0, 4000.0, 0), 60.0),
                    (param("Softness", "%", 0.5, PERCENT.0, PERCENT.1, 0), 60.0),
                    (param("Intensity", "%", 0.5, 0.0, 500.0, 0), 80.0),
                ],
            },
            // The glare of a bright light in a camera lens: a glow, a streak and a ring at the
            // center (as % of the frame), and faint discs on the line through the frame's middle.
            EffectDefinition {
                name: "Lens Flare",
                parameters: &[
                    (param("Flare Center X", "%", 0.2, -50.0, 150.0, 1), 30.0),
                    (param("Flare Center Y", "%", 0.2, -50.0, 150.0, 1), 30.0),
                    (param("Flare Brightness", "%", 0.5, 0.0, 300.0, 0), 100.0),
                ],
            },
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
            // Shrinks the matte's edge inward by Choke Matte pixels, or grows it outward when
            // negative.
            EffectDefinition { name: "Simple Choker", parameters: &[(param("Choke Matte", "px", 0.05, -100.0, 100.0, 1), 1.0)] },
            // Removes areas by brightness: those within Tolerance of the Threshold brightness turn
            // clear, fading over Softness beyond that. Threshold 0 removes black, 100 white.
            // Invert (1) keeps only that brightness instead.
            EffectDefinition {
                name: "Luma Key",
                parameters: &[
                    (param("Threshold", "%", 0.5, PERCENT.0, PERCENT.1, 0), 0.0),
                    (param("Tolerance", "%", 0.5, PERCENT.0, PERCENT.1, 0), 10.0),
                    (param("Softness", "%", 0.5, PERCENT.0, PERCENT.1, 0), 10.0),
                    (param("Invert", "", 0.05, 0.0, 1.0, 0), 0.0),
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
    // Effects on a clip's sound, run as the mix plays and exports (see `studio_dsp`, whose
    // names and setting order must match these). They don't change the picture.
    EffectCategory {
        name: AUDIO_CATEGORY,
        effects: &[
            // A low cut and a high cut, shelves at 120 Hz and 8 kHz, and one band to move.
            EffectDefinition {
                name: "EQ",
                parameters: &[
                    (param("Low Cut", "Hz", 1.0, 20.0, 1000.0, 0), 20.0),
                    (param("Low Gain", "dB", 0.1, -24.0, 24.0, 1), 0.0),
                    (param("Mid Frequency", "Hz", 5.0, 20.0, 20000.0, 0), 1000.0),
                    (param("Mid Gain", "dB", 0.1, -24.0, 24.0, 1), 0.0),
                    (param("High Gain", "dB", 0.1, -24.0, 24.0, 1), 0.0),
                    (param("High Cut", "Hz", 10.0, 1000.0, 20000.0, 0), 20000.0),
                ],
            },
            // Evens out loud and quiet moments.
            EffectDefinition {
                name: "Compressor",
                parameters: &[
                    (param("Threshold", "dB", 0.1, -60.0, 0.0, 1), -18.0),
                    (param("Ratio", ":1", 0.05, 1.0, 20.0, 1), 4.0),
                    (param("Attack", "ms", 0.1, 0.1, 200.0, 1), 10.0),
                    (param("Release", "ms", 1.0, 10.0, 2000.0, 0), 150.0),
                    (param("Makeup Gain", "dB", 0.1, 0.0, 24.0, 1), 0.0),
                ],
            },
            // Keeps peaks under a ceiling.
            EffectDefinition { name: "Limiter", parameters: &[(param("Ceiling", "dB", 0.1, -24.0, 0.0, 1), -1.0), (param("Release", "ms", 1.0, 10.0, 1000.0, 0), 80.0)] },
            // Tames harsh "s" sounds.
            EffectDefinition { name: "De-esser", parameters: &[(param("Amount", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0)] },
            // Removes mains hum (50 or 60 Hz) and its harmonics.
            EffectDefinition { name: "Hum Remover", parameters: &[(param("Frequency", "Hz", 1.0, 50.0, 60.0, 0), 60.0), (param("Harmonics", "", 0.05, 1.0, 8.0, 0), 4.0)] },
            EffectDefinition { name: "Gain", parameters: &[(param("Gain", "dB", 0.1, -48.0, 24.0, 1), 0.0)] },
            // -100 is all left, 100 all right.
            EffectDefinition { name: "Pan", parameters: &[(param("Pan", "", 0.5, -100.0, 100.0, 0), 0.0)] },
            EffectDefinition {
                name: "Echo",
                parameters: &[
                    (param("Time", "ms", 1.0, 1.0, 2000.0, 0), 250.0),
                    (param("Feedback", "%", 0.5, 0.0, 95.0, 0), 30.0),
                    (param("Mix", "%", 0.5, PERCENT.0, PERCENT.1, 0), 30.0),
                ],
            },
            // The sound of a room: Room Size sets how long the reflections ring on, Damping how
            // quickly their treble fades, and Mix how much of the room is heard with the original.
            EffectDefinition {
                name: "Reverb",
                parameters: &[
                    (param("Room Size", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0),
                    (param("Damping", "%", 0.5, PERCENT.0, PERCENT.1, 0), 50.0),
                    (param("Mix", "%", 0.5, PERCENT.0, PERCENT.1, 0), 25.0),
                ],
            },
        ],
    },
];

/// Where an effect's on-screen point is measured from, so the viewer can draw a handle for it
/// and turn a drag back into the effect's settings. Points are in composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointOrigin {
    /// Pixels from one of the layer's own corners (0 top left, 1 top right, 2 bottom right,
    /// 3 bottom left), where it sits in the frame.
    Corner(usize),
    /// Percentages of the frame's width and height.
    Percent,
    /// Pixels from the middle of the frame.
    Center,
}

/// A point an effect's settings describe, shown as a handle in the viewer: its name, the
/// settings holding its x and y, and what they are measured from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectPoint {
    pub label: &'static str,
    pub x: usize,
    pub y: usize,
    pub origin: PointOrigin,
}

/// The on-screen points of the effect called `name` (none for most effects).
pub fn points(name: &str) -> &'static [EffectPoint] {
    const CORNER_PIN: [EffectPoint; 4] = [
        EffectPoint { label: "Top Left", x: 0, y: 1, origin: PointOrigin::Corner(0) },
        EffectPoint { label: "Top Right", x: 2, y: 3, origin: PointOrigin::Corner(1) },
        EffectPoint { label: "Bottom Left", x: 4, y: 5, origin: PointOrigin::Corner(3) },
        EffectPoint { label: "Bottom Right", x: 6, y: 7, origin: PointOrigin::Corner(2) },
    ];
    const LENS_FLARE: [EffectPoint; 1] = [EffectPoint { label: "Flare Center", x: 0, y: 1, origin: PointOrigin::Percent }];
    const TRANSFORM: [EffectPoint; 1] = [EffectPoint { label: "Position", x: 0, y: 1, origin: PointOrigin::Center }];
    match name {
        "Corner Pin" => &CORNER_PIN,
        "Lens Flare" => &LENS_FLARE,
        "Transform" => &TRANSFORM,
        _ => &[],
    }
}

/// Where an effect's points are measured from: the frame's size, and the layer's own corners in
/// the frame (clockwise from the top left), all in composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointSpace {
    pub frame: crate::animation::Vec2,
    pub layer_corners: [crate::animation::Vec2; 4],
}

impl EffectPoint {
    /// Where the point is in `space`, given its two settings' values.
    pub fn position(&self, values: (f32, f32), space: PointSpace) -> crate::animation::Vec2 {
        use crate::animation::Vec2;
        let frame = space.frame;
        match self.origin {
            PointOrigin::Corner(corner) => space.layer_corners[corner.min(3)] + Vec2::new(values.0, values.1),
            PointOrigin::Percent => Vec2::new(values.0 / 100.0 * frame.x, values.1 / 100.0 * frame.y),
            PointOrigin::Center => Vec2::new(frame.x * 0.5 + values.0, frame.y * 0.5 + values.1),
        }
    }

    /// The two settings' values that put the point at `at` in `space`.
    pub fn values(&self, at: crate::animation::Vec2, space: PointSpace) -> (f32, f32) {
        let frame = space.frame;
        match self.origin {
            PointOrigin::Corner(corner) => {
                let from = space.layer_corners[corner.min(3)];
                (at.x - from.x, at.y - from.y)
            }
            PointOrigin::Percent => (at.x / frame.x.max(1.0) * 100.0, at.y / frame.y.max(1.0) * 100.0),
            PointOrigin::Center => (at.x - frame.x * 0.5, at.y - frame.y * 0.5),
        }
    }
}

/// The name of the category of effects on sound.
pub const AUDIO_CATEGORY: &str = "Audio";

/// Whether the effect called `name` works on sound rather than on the picture.
pub fn is_audio(name: &str) -> bool {
    category_of(name).is_some_and(|category| category.name == AUDIO_CATEGORY)
}

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
    // Sound effects suit what can have sound: audio, footage and nested compositions.
    if category.name == AUDIO_CATEGORY {
        return matches!(content, LayerContent::Audio { .. } | LayerContent::Footage { .. } | LayerContent::Composition { .. });
    }
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

/// Fills in an effect's display and range information from the catalog (it isn't saved with
/// the effect). Parameters the catalog doesn't know keep an unlimited range.
pub fn restore_info(effect: &mut Effect) {
    let definition = find(&effect.name);
    for (index, property) in effect.properties.iter_mut().enumerate() {
        let known = definition.and_then(|definition| definition.parameters.get(index)).map(|(info, _)| *info);
        property.info = known.filter(|info| info.name == property.name).unwrap_or(crate::property::PropertyInfo::unknown());
    }
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
mod points_tests {
    use super::*;
    use crate::animation::Vec2;

    #[test]
    fn on_screen_points_round_trip_through_their_settings() {
        let frame = Vec2::new(1920.0, 1080.0);
        // A 400 × 300 layer at (100, 50).
        let layer_corners = [Vec2::new(100.0, 50.0), Vec2::new(500.0, 50.0), Vec2::new(500.0, 350.0), Vec2::new(100.0, 350.0)];
        let space = PointSpace { frame, layer_corners };
        for name in ["Corner Pin", "Lens Flare", "Transform"] {
            let definition = find(name).unwrap();
            for point in points(name) {
                assert!(point.x < definition.parameters.len() && point.y < definition.parameters.len(), "{name}");
                let at = Vec2::new(300.0, 200.0);
                let (x, y) = point.values(at, space);
                assert!((point.position((x, y), space) - at).length() < 1e-3, "{name} {}", point.label);
            }
        }
        // Corner Pin's corners with no offset sit on the layer's own corners.
        assert_eq!(points("Corner Pin")[3].position((0.0, 0.0), space), Vec2::new(500.0, 350.0));
        assert_eq!(points("Corner Pin")[2].position((10.0, 0.0), space), Vec2::new(110.0, 350.0));
        assert_eq!(points("Lens Flare")[0].position((50.0, 50.0), space), Vec2::new(960.0, 540.0));
        assert!(points("Gaussian Blur").is_empty());
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
