//! Device mockups: iPhone, iPhone Duo, iPad, Mac, Apple Watch and a browser window, drawn as
//! shapes (a body in one color with a thin outline, side buttons, the Dynamic Island or the
//! notch, watch bands, the browser's window buttons and address bar), with their screens at the
//! sizes App Store screenshots use (the browser's at 1920 × 1080, for videos of websites).
//!
//! Making one adds two compositions in a "Mockups" folder:
//!
//! - **the screen**, at the device's own screen size, where footage goes (it starts with a drop
//!   spot, or with the footage asked for, scaled to fill it), and
//! - **the mockup**, see-through and just big enough for the device (and its shadow), which
//!   shows the device with the screen composition inside it, cut to the screen's rounded
//!   corners by a mask layer. Every part hangs off a "Device" null.
//!
//! `add_to` puts the mockup into a composition as one layer, centered and scaled to fit its
//! frame. Edit the screen composition and every mockup using it follows.

use crate::animation::{Animated, Vec2};
use crate::edit::{self, CompositionSettings};
use crate::model::{Id, LayerContent, LayerMask, MaskMode, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke};
use crate::time::FrameRate;

/// The devices mockups come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    IPhone,
    Duo,
    IPad,
    Mac,
    Watch,
    Browser,
}

impl Device {
    pub const ALL: [Self; 6] = [Self::IPhone, Self::Duo, Self::IPad, Self::Mac, Self::Watch, Self::Browser];

    pub fn name(self) -> &'static str {
        match self {
            Self::IPhone => "iPhone",
            Self::Duo => "iPhone Duo",
            Self::IPad => "iPad",
            Self::Mac => "Mac",
            Self::Watch => "Apple Watch",
            Self::Browser => "Browser",
        }
    }

    /// The screen in pixels, upright: the App Store screenshot sizes, and for the browser the
    /// page at 1920 × 1080.
    pub fn screen(self) -> (u32, u32) {
        match self {
            Self::IPhone => (1320, 2868),
            Self::Duo => (2007, 2853),
            Self::IPad => (2064, 2752),
            Self::Mac => (2880, 1800),
            Self::Watch => (416, 496),
            Self::Browser => (1920, 1080),
        }
    }

    /// Whether the device can be turned on its side (a Mac, a watch and a browser window only
    /// stand one way).
    pub fn turns(self) -> bool {
        matches!(self, Self::IPhone | Self::Duo | Self::IPad)
    }

    /// The finishes the device comes in, by name; the first is how its picture is shown. Each
    /// device has its own list, so one can come in colors the others don't.
    pub fn finishes(self) -> Vec<(&'static str, Finish)> {
        let finish = |body: u32, outline: u32, shadow: f32| Finish { body: Rgba::hex(body), outline: Rgba::hex(outline), outline_width: 4.0, shadow };
        match self {
            Self::Watch => vec![
                ("Jet Black", finish(0x1c1c1e, 0x3a3a3c, 0.55)),
                ("Silver", finish(0xdcdde0, 0xa9abb0, 0.4)),
                ("Rose Gold", finish(0xe9cfc6, 0xc29e92, 0.45)),
                ("Natural Titanium", finish(0xc8c2b8, 0x9c968c, 0.45)),
            ],
            Self::Browser => vec![("Light", finish(0xf4f4f5, 0xd4d4d8, 0.45)), ("Dark", finish(0x2a2a2e, 0x46464d, 0.55))],
            _ => Finish::presets().to_vec(),
        }
    }
}

/// Which way up the device stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Orientation {
    #[default]
    Portrait,
    /// Turned a quarter to the left: the top of the device is on the left.
    Landscape,
}

/// How the device looks: body color, outline color and width, and how dark its shadow is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub body: Rgba,
    pub outline: Rgba,
    /// Outline width in pixels on a device about 1,100 pixels wide.
    pub outline_width: f32,
    /// Shadow strength, 0 to 1.
    pub shadow: f32,
}

impl Finish {
    /// Ready-made finishes, by name.
    pub fn presets() -> [(&'static str, Finish); 4] {
        let finish = |body: u32, outline: u32, shadow: f32| Finish { body: Rgba::hex(body), outline: Rgba::hex(outline), outline_width: 4.0, shadow };
        [
            ("Space Black", finish(0x1c1c1e, 0x48484a, 0.55)),
            ("Silver", finish(0xe3e4e6, 0xb4b6b9, 0.4)),
            ("Midnight", finish(0x1c2433, 0xa0b8ce, 0.6)),
            ("Gold", finish(0xf1e3cf, 0xc9ae86, 0.45)),
        ]
    }
}

/// How much of a composition's frame a mockup added to it fills, on its longer side.
const FIT: f32 = 0.9;
/// Room around the device in its mockup composition for the shadow, as a share of its size.
const SHADOW_ROOM: f32 = 0.12;

/// What to make.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MockupRequest {
    pub device: Device,
    pub orientation: Orientation,
    pub finish: Finish,
    /// Footage or a picture to put on the screen (a project item), scaled to fill it.
    pub footage: Option<Id>,
    pub seconds: f64,
}

/// What was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mockup {
    pub composition: Id,
    pub screen: Id,
}

/// A mockup added to a composition: the layer showing it, and its two compositions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub layer: Id,
    pub mockup: Mockup,
}

/// A rectangle of the device, in device pixels from its middle, drawn upright.
#[derive(Clone, Copy, Debug)]
struct Part {
    center: Vec2,
    size: Vec2,
    corner: f32,
}

impl Part {
    fn new(cx: f32, cy: f32, width: f32, height: f32, corner: f32) -> Self {
        Self { center: Vec2::new(cx, cy), size: Vec2::new(width, height), corner }
    }

    /// The same part with the device turned on its side (its top to the left).
    fn turned(self, orientation: Orientation) -> Self {
        match orientation {
            Orientation::Portrait => self,
            Orientation::Landscape => Self { center: Vec2::new(self.center.y, -self.center.x), size: Vec2::new(self.size.y, self.size.x), corner: self.corner },
        }
    }
}

/// What color a detail is drawn in.
#[derive(Clone, Copy, Debug)]
enum Tone {
    /// Always this color (the browser's window buttons).
    Fixed(u32),
    /// The finish's body color moved this far toward its outline color, so it suits every
    /// finish (a watch band, the browser's address bar).
    Shade(f32),
}

/// A colored part of the device other than its body and buttons, with its layer's name.
#[derive(Clone, Copy, Debug)]
struct Detail {
    name: &'static str,
    part: Part,
    tone: Tone,
}

/// A device's drawing, upright, in device pixels around its middle.
struct Drawing {
    body: Part,
    screen: Part,
    /// Buttons on the edges, drawn behind the body.
    buttons: Vec<Part>,
    /// Parts drawn behind the body (watch bands), and on it, around the screen (the browser's
    /// toolbar).
    behind: Vec<Detail>,
    on_body: Vec<Detail>,
    /// Black parts over the screen: the Dynamic Island, a camera, the Mac's notch.
    over_screen: Vec<Part>,
    /// A faint fold down the middle (the Duo).
    fold: Option<Part>,
    /// The Mac's base below the lid.
    base: Option<(Part, Part)>,
}

fn drawing(device: Device) -> Drawing {
    let (w, h) = device.screen();
    let (w, h) = (w as f32, h as f32);
    match device {
        Device::IPhone => {
            let bezel = 48.0;
            let (bw, bh) = (w + bezel * 2.0, h + bezel * 2.0);
            let top = -h * 0.5;
            let left_edge = -bw * 0.5;
            let right_edge = bw * 0.5;
            let button = |x: f32, from: f32, length: f32| Part::new(x, top + from + length * 0.5, 26.0, length, 10.0);
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 185.0 + bezel),
                screen: Part::new(0.0, 0.0, w, h, 185.0),
                buttons: vec![button(left_edge, 520.0, 130.0), button(left_edge, 760.0, 240.0), button(left_edge, 1060.0, 240.0), button(right_edge, 880.0, 400.0)],
                over_screen: vec![Part::new(0.0, top + 95.0, 380.0, 112.0, 56.0)],
                fold: None,
                base: None,
                behind: Vec::new(),
                on_body: Vec::new(),
            }
        }
        Device::Duo => {
            let bezel = 46.0;
            let (bw, bh) = (w + bezel * 2.0, h + bezel * 2.0);
            let top = -h * 0.5;
            let right_edge = bw * 0.5;
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 110.0 + bezel),
                screen: Part::new(0.0, 0.0, w, h, 110.0),
                buttons: vec![Part::new(right_edge, top + 840.0, 26.0, 280.0, 10.0), Part::new(right_edge, top + 1220.0, 26.0, 340.0, 10.0)],
                over_screen: vec![Part::new(w * 0.5 - 130.0, top + 110.0, 64.0, 64.0, 32.0)],
                fold: Some(Part::new(0.0, 0.0, 4.0, h, 0.0)),
                base: None,
                behind: Vec::new(),
                on_body: Vec::new(),
            }
        }
        Device::IPad => {
            let bezel = 110.0;
            let (bw, bh) = (w + bezel * 2.0, h + bezel * 2.0);
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 60.0 + bezel),
                screen: Part::new(0.0, 0.0, w, h, 60.0),
                buttons: vec![Part::new(w * 0.32, -bh * 0.5, 260.0, 26.0, 10.0), Part::new(bw * 0.5, -h * 0.5 + 420.0, 26.0, 200.0, 10.0)],
                // The front camera, on the edge that is the top when the iPad is turned sideways.
                over_screen: vec![Part::new(w * 0.5 + bezel * 0.5, 0.0, 26.0, 26.0, 13.0)],
                fold: None,
                base: None,
                behind: Vec::new(),
                on_body: Vec::new(),
            }
        }
        Device::Mac => {
            let (side, top_bezel, bottom_bezel) = (60.0, 70.0, 110.0);
            let (bw, bh) = (w + side * 2.0, h + top_bezel + bottom_bezel);
            // The lid's middle is off the screen's: the bezel is deeper at the bottom.
            let screen_y = (top_bezel - bottom_bezel) * 0.5;
            let lid_bottom = bh * 0.5;
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 60.0),
                screen: Part::new(0.0, screen_y, w, h, 18.0),
                buttons: Vec::new(),
                over_screen: vec![Part::new(0.0, screen_y - h * 0.5 + 30.0, 400.0, 76.0, 22.0)],
                fold: None,
                base: Some((Part::new(0.0, lid_bottom + 34.0, bw * 1.16, 68.0, 34.0), Part::new(0.0, lid_bottom + 12.0, 520.0, 24.0, 12.0))),
                behind: Vec::new(),
                on_body: Vec::new(),
            }
        }
        Device::Watch => {
            let bezel = 44.0;
            let (bw, bh) = (w + bezel * 2.0, h + bezel * 2.0);
            // The bands run on from the top and bottom of the case, a little narrower than it.
            let (band_width, band_length) = (bw * 0.78, 300.0);
            let band = |direction: f32| Detail {
                name: "Band",
                part: Part::new(0.0, direction * (bh * 0.5 + band_length * 0.5 - 60.0), band_width, band_length, 60.0),
                tone: Tone::Shade(0.4),
            };
            let right_edge = bw * 0.5;
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 96.0 + bezel),
                screen: Part::new(0.0, 0.0, w, h, 96.0),
                // The Digital Crown, then the side button below it.
                buttons: vec![Part::new(right_edge + 6.0, -h * 0.2, 40.0, 112.0, 16.0), Part::new(right_edge, h * 0.18, 22.0, 150.0, 10.0)],
                over_screen: Vec::new(),
                fold: None,
                base: None,
                behind: vec![band(-1.0), band(1.0)],
                on_body: Vec::new(),
            }
        }
        Device::Browser => {
            // A window: a toolbar across the top, then the page edge to edge below it.
            let (border, toolbar) = (2.0, 104.0);
            let (bw, bh) = (w + border * 2.0, h + toolbar + border);
            let toolbar_y = -bh * 0.5 + toolbar * 0.5;
            let screen_y = -bh * 0.5 + toolbar + h * 0.5;
            let left = -bw * 0.5;
            // The close, minimize and zoom buttons, in their own colors.
            let light = |index: usize, name: &'static str, color: u32| Detail { name, part: Part::new(left + 44.0 + index as f32 * 40.0, toolbar_y, 24.0, 24.0, 12.0), tone: Tone::Fixed(color) };
            let address = Part::new(0.0, toolbar_y, w * 0.42, 60.0, 16.0);
            let button = |index: usize| Detail { name: "Toolbar button", part: Part::new(bw * 0.5 - 52.0 - index as f32 * 56.0, toolbar_y, 30.0, 30.0, 8.0), tone: Tone::Shade(0.6) };
            Drawing {
                body: Part::new(0.0, 0.0, bw, bh, 26.0),
                screen: Part::new(0.0, screen_y, w, h, 24.0),
                buttons: Vec::new(),
                over_screen: Vec::new(),
                fold: None,
                base: None,
                behind: Vec::new(),
                on_body: vec![
                    light(0, "Close", 0xff5f57),
                    light(1, "Minimize", 0xfebc2e),
                    light(2, "Zoom", 0x28c840),
                    Detail { name: "Address bar", part: address, tone: Tone::Shade(0.5) },
                    // A short line standing for the page's address.
                    Detail { name: "Address", part: Part::new(0.0, toolbar_y, w * 0.14, 14.0, 7.0), tone: Tone::Shade(1.4) },
                    button(0),
                    button(1),
                ],
            }
        }
    }
}

/// The color a detail is drawn in, for `finish`.
fn tone_color(tone: Tone, finish: Finish) -> Rgba {
    match tone {
        Tone::Fixed(color) => Rgba::hex(color),
        Tone::Shade(amount) => mix(finish.body, finish.outline, amount),
    }
}

/// Halfway between two colors, for the Mac's base and its thumb notch.
fn mix(a: Rgba, b: Rgba, amount: f32) -> Rgba {
    Rgba { r: a.r + (b.r - a.r) * amount, g: a.g + (b.g - a.g) * amount, b: a.b + (b.b - a.b) * amount, a: 1.0, link: None }
}

/// Makes the screen and mockup compositions for `request`. Returns them, or `None` if the
/// footage asked for isn't a picture or video in the project.
pub fn build(project: &mut Project, request: &MockupRequest) -> Option<Mockup> {
    let device = request.device;
    let orientation = if device.turns() { request.orientation } else { Orientation::Portrait };
    let footage = match request.footage {
        Some(item) => Some(project.item(item).filter(|item| item.size.is_some())?.id),
        None => None,
    };
    let rate = FrameRate::new(30, 1);
    let duration = ((request.seconds.clamp(1.0, 3600.0) * rate.fps()).round() as i64).max(1);
    let label = match orientation {
        Orientation::Portrait => device.name().to_owned(),
        Orientation::Landscape => format!("{} Landscape", device.name()),
    };

    // The screen, at the device's own size.
    let (sw, sh) = device.screen();
    let (sw, sh) = if orientation == Orientation::Landscape { (sh, sw) } else { (sw, sh) };
    let settings = CompositionSettings { name: format!("{label} Screen"), width: sw, height: sh, frame_rate: rate, duration_frames: duration, background: Rgba::hex(0x000000) };
    let screen = edit::new_composition(project, &settings).ok()?;
    fill_screen(project, screen, footage);

    // The mockup: see-through, the device's own size (everything, buttons and base included)
    // with room for its shadow, so the screen shows at its full resolution.
    let drawing = drawing(device);
    let turn = |part: Part| part.turned(orientation);
    let mut extent = Vec2::new(turn(drawing.body).size.x * 0.5, turn(drawing.body).size.y * 0.5);
    let behind = drawing.behind.iter().map(|detail| &detail.part);
    for part in drawing.buttons.iter().chain(drawing.base.iter().map(|(base, _)| base)).chain(behind).copied().map(turn) {
        extent.x = extent.x.max(part.center.x.abs() + part.size.x * 0.5);
        extent.y = extent.y.max(part.center.y.abs() + part.size.y * 0.5);
    }
    let even = |length: f32| ((length * (1.0 + SHADOW_ROOM * 2.0)).ceil() as u32 + 1) & !1;
    let (cw, ch) = (even(extent.x * 2.0), even(extent.y * 2.0));
    let settings = CompositionSettings { name: format!("{label} Mockup"), width: cw, height: ch, frame_rate: rate, duration_frames: duration, background: Rgba::TRANSPARENT };
    let comp = edit::new_composition(project, &settings).ok()?;
    let finish = request.finish;
    let scale = 1.0;
    let middle = Vec2::new(cw as f32 * 0.5, ch as f32 * 0.5);
    let place = |part: Part| (middle + turn(part).center * scale, turn(part).size * scale, turn(part).corner * scale);
    let outline = finish.outline_width * drawing.body.size.x.min(drawing.body.size.y) / 1100.0 * scale;

    // Bottom to top: bands, buttons and the base behind the body, then the body and what is on
    // it, the screen and what sits over it.
    let mut parts = Vec::new();
    for (index, detail) in drawing.behind.iter().enumerate() {
        let (at, size, corner) = place(detail.part);
        parts.push(shape(project, comp, detail.name, index, size, at, corner, Some(tone_color(detail.tone, finish)), Some((finish.outline, outline)))?);
    }
    for (index, button) in drawing.buttons.iter().enumerate() {
        let (at, size, corner) = place(*button);
        parts.push(shape(project, comp, "Button", index, size, at, corner, Some(finish.body), Some((finish.outline, outline)))?);
    }
    if let Some((base, notch)) = drawing.base {
        let (at, size, corner) = place(base);
        let metal = mix(finish.body, finish.outline, 0.55);
        parts.push(shape(project, comp, "Base", 0, size, at, corner, Some(metal), Some((finish.outline, outline)))?);
        let (at, size, corner) = place(notch);
        parts.push(shape(project, comp, "Base notch", 0, size, at, corner, Some(mix(metal, Rgba::hex(0x000000), 0.25)), None)?);
    }
    let (at, size, corner) = place(drawing.body);
    let body = shape(project, comp, "Body", 0, size, at, corner, Some(finish.body), Some((finish.outline, outline)))?;
    if finish.shadow > 0.0 {
        let shadow = edit::add_effect(project, comp, body, "Drop Shadow");
        set_effect(project, comp, body, shadow, &[(0, (finish.shadow * 100.0).clamp(0.0, 100.0)), (1, 180.0), (2, 40.0 * scale), (3, (90.0 * scale).min(250.0))]);
    }
    parts.push(body);
    for detail in &drawing.on_body {
        let (at, size, corner) = place(detail.part);
        parts.push(shape(project, comp, detail.name, 0, size, at, corner, Some(tone_color(detail.tone, finish)), None)?);
    }

    // The screen composition, cut to the screen's rounded corners by a mask.
    let (at, size, corner) = place(drawing.screen);
    let mask = shape(project, comp, "Screen mask", 0, size, at, corner, Some(Rgba::hex(0xffffff)), None)?;
    let shown = edit::add_layer_from_item(project, comp, screen, 0, 0)?;
    {
        let composition = project.composition_mut(comp)?;
        if let Some(layer) = composition.layer_mut(mask) {
            layer.switches.visible = false;
        }
        let layer = composition.layer_mut(shown)?;
        layer.name = "Screen".into();
        let transform = layer.transform.as_mut()?;
        transform.position = Animated::constant(at);
        transform.scale = Animated::constant(Vec2::new(size.x / sw as f32 * 100.0, size.y / sh as f32 * 100.0));
        layer.mask = Some(LayerMask { layer: mask, mode: MaskMode::Show });
    }
    parts.extend([mask, shown]);
    if let Some(fold) = drawing.fold {
        let (at, size, corner) = place(fold);
        parts.push(shape(project, comp, "Fold", 0, size, at, corner, Some(Rgba::hex(0xffffff).with_alpha(0.07)), None)?);
    }
    for (index, part) in drawing.over_screen.iter().enumerate() {
        let (at, size, corner) = place(*part);
        let name = match device {
            Device::IPhone => "Dynamic Island",
            Device::Mac => "Notch",
            _ => "Camera",
        };
        parts.push(shape(project, comp, name, index, size, at, corner, Some(Rgba::hex(0x000000)), None)?);
    }

    // A null everything hangs off, so the device moves as one.
    let null = add_null(project, comp, middle)?;
    let composition = project.composition_mut(comp)?;
    for part in parts {
        edit::set_parent(composition, part, Some(null), 0);
    }
    crate::folders::file_into(project, screen, "Mockups");
    crate::folders::file_into(project, comp, "Mockups");
    Some(Mockup { composition: comp, screen })
}

/// Makes a mockup for `request` and adds it to the composition `comp` as one layer at the top,
/// centered and scaled so the device fills most of the frame. Both last as long as `comp`, or
/// as long as the footage on the screen if that is longer.
pub fn add_to(project: &mut Project, comp: Id, request: &MockupRequest) -> Option<Placed> {
    let target = project.composition(comp)?;
    let (width, height, length) = (target.width as f32, target.height as f32, target.duration_frames as f64 / target.frame_rate.fps());
    let mockup = build(project, &MockupRequest { seconds: request.seconds.max(length), ..*request })?;
    let made = project.composition(mockup.composition)?;
    let fit = (width * FIT / made.width as f32).min(height * FIT / made.height as f32) * 100.0;
    let layer = edit::add_layer_from_item(project, comp, mockup.composition, 0, 0)?;
    let added = project.composition_mut(comp)?.layer_mut(layer)?;
    let transform = added.transform.as_mut()?;
    transform.position = Animated::constant(Vec2::new(width * 0.5, height * 0.5));
    transform.scale = Animated::constant(Vec2::new(fit, fit));
    Some(Placed { layer, mockup })
}

/// The screen composition's contents: the footage scaled to fill it, or a drop spot.
fn fill_screen(project: &mut Project, screen: Id, footage: Option<Id>) {
    let Some(composition) = project.composition(screen) else { return };
    let (width, height) = (composition.width as f32, composition.height as f32);
    if let Some(item) = footage
        && let Some(layer) = edit::add_layer_from_item(project, screen, item, 0, 0)
        && let Some(found) = project.composition_mut(screen).and_then(|composition| composition.layer_mut(layer))
        && let Some(transform) = found.transform.as_mut()
    {
        let fill = (width / found.size.x.max(1.0)).max(height / found.size.y.max(1.0)) * 100.0;
        transform.position = Animated::constant(Vec2::new(width * 0.5, height * 0.5));
        transform.scale = Animated::constant(Vec2::new(fill, fill));
        return;
    }
    let fill = Paint::Linear { start: Rgba::hex(0x3f3f46), end: Rgba::hex(0x27272a), angle: 60.0 };
    let style = ShapeStyle { fill: Some(fill), ..ShapeStyle::new(ShapeKind::Rectangle) };
    if let Some(id) = edit::add_shape_layer(project, screen, style, Vec2::new(width, height), Vec2::new(width * 0.5, height * 0.5), 0)
        && let Some(layer) = project.composition_mut(screen).and_then(|composition| composition.layer_mut(id))
    {
        layer.name = "Drop your screen recording here".into();
    }
}

/// A rounded rectangle named `name` (numbered after the first), at the top of the stack.
#[allow(clippy::too_many_arguments)]
fn shape(project: &mut Project, comp: Id, name: &str, index: usize, size: Vec2, at: Vec2, corner: f32, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>) -> Option<Id> {
    let style = ShapeStyle {
        fill: fill.map(Paint::Solid),
        stroke: stroke.filter(|(_, width)| *width > 0.0).map(|(color, width)| Stroke { paint: Paint::Solid(color), width }),
        corner_radius: corner,
        ..ShapeStyle::new(ShapeKind::Rectangle)
    };
    let id = edit::add_shape_layer(project, comp, style, size, at, 0)?;
    let layer = project.composition_mut(comp)?.layer_mut(id)?;
    layer.name = if index == 0 { name.to_owned() } else { format!("{name} {}", index + 1) };
    Some(id)
}

/// A null: a hidden layer that others are parented to, as imported nulls are.
fn add_null(project: &mut Project, comp: Id, at: Vec2) -> Option<Id> {
    let style = ShapeStyle { fill: None, ..ShapeStyle::new(ShapeKind::Rectangle) };
    let id = edit::add_shape_layer(project, comp, style, Vec2::new(100.0, 100.0), at, 0)?;
    let layer = project.composition_mut(comp)?.layer_mut(id)?;
    layer.name = "Device".into();
    layer.content = LayerContent::Solid { color: Rgba::TRANSPARENT };
    layer.switches.visible = false;
    Some(id)
}

/// Sets some of an effect's settings, by position.
fn set_effect(project: &mut Project, comp: Id, layer: Id, effect: Option<Id>, settings: &[(usize, f32)]) {
    let Some(effect) = effect else { return };
    let Some(found) = project.composition_mut(comp).and_then(|composition| composition.layer_mut(layer)).and_then(|layer| layer.effects.iter_mut().find(|candidate| candidate.id == effect)) else { return };
    for &(index, value) in settings {
        if let Some(property) = found.properties.get_mut(index) {
            property.value = Animated::constant(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(device: Device, orientation: Orientation) -> MockupRequest {
        MockupRequest { device, orientation, finish: Finish::presets()[0].1, footage: None, seconds: 10.0 }
    }

    #[test]
    fn every_device_makes_a_screen_and_a_masked_mockup() {
        for device in Device::ALL {
            for orientation in [Orientation::Portrait, Orientation::Landscape] {
                let mut project = Project::new(Vec::new(), Vec::new());
                let made = build(&mut project, &request(device, orientation)).unwrap();
                let screen = project.composition(made.screen).unwrap();
                let (w, h) = device.screen();
                let turned = device.turns() && orientation == Orientation::Landscape;
                assert_eq!((screen.width, screen.height), if turned { (h, w) } else { (w, h) }, "{device:?} {orientation:?}");
                let mockup = project.composition(made.composition).unwrap();
                assert_eq!(mockup.background, Rgba::TRANSPARENT, "see-through, to go over anything");
                assert!(mockup.width > screen.width && mockup.height > screen.height, "the device and its shadow fit around the screen");
                let shown = mockup.layers.iter().find(|layer| layer.content == LayerContent::Composition { composition: made.screen }).unwrap();
                let mask = mockup.layer(shown.mask.unwrap().layer).unwrap();
                assert!(!mask.switches.visible, "the mask only shapes the screen");
                // The screen fits its hole exactly, at its own resolution.
                let scale = shown.transform.as_ref().unwrap().scale.value;
                let shown_size = Vec2::new(shown.size.x * scale.x, shown.size.y * scale.y) * 0.01;
                assert!((shown_size.x - mask.size.x).abs() < 0.5 && (shown_size.y - mask.size.y).abs() < 0.5, "{device:?}: {shown_size:?} vs {:?}", mask.size);
                // Everything follows the Device null.
                let null = mockup.layers.iter().find(|layer| layer.name == "Device").unwrap().id;
                assert!(mockup.layers.iter().filter(|layer| layer.id != null).all(|layer| layer.parent == Some(null)));
                let view = crate::project_file::EditorView { active_composition: made.composition, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, mockup.duration_frames as f64) };
                crate::project_file::from_text(&crate::project_file::to_text(&project, view).unwrap()).unwrap();
            }
        }
    }

    #[test]
    fn an_added_mockup_is_one_layer_that_fits_the_frame() {
        for (width, height) in [(1920, 1080), (1080, 1920), (1080, 1350)] {
            let mut project = Project::new(Vec::new(), Vec::new());
            let settings = CompositionSettings { name: "Main".into(), width, height, frame_rate: FrameRate::new(30, 1), duration_frames: 600, background: Rgba::hex(0) };
            let comp = edit::new_composition(&mut project, &settings).unwrap();
            let placed = add_to(&mut project, comp, &request(Device::IPad, Orientation::Landscape)).unwrap();
            let main = project.composition(comp).unwrap();
            assert_eq!(main.layers.len(), 1, "one layer, not every part of the device");
            let layer = main.layer(placed.layer).unwrap();
            assert_eq!(layer.content, LayerContent::Composition { composition: placed.mockup.composition });
            let transform = layer.transform.as_ref().unwrap();
            assert_eq!(transform.position.value, Vec2::new(width as f32 * 0.5, height as f32 * 0.5));
            // Shown size: the mockup scaled, which fills 90% of the frame on one side and fits on the other.
            let shown = layer.size * (transform.scale.value.x * 0.01);
            let (fx, fy) = (shown.x / width as f32, shown.y / height as f32);
            assert!(fx <= FIT + 1e-3 && fy <= FIT + 1e-3 && ((fx - FIT).abs() < 1e-3 || (fy - FIT).abs() < 1e-3), "{width}×{height}: {fx} {fy}");
            // It lasts as long as the composition.
            assert_eq!(project.composition(placed.mockup.composition).unwrap().duration_frames, 600);
        }
    }

    #[test]
    fn footage_fills_the_screen() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let info = edit::FootageInfo { size: Vec2::new(1920.0, 1080.0), duration_seconds: 5.0, frame_rate: FrameRate::new(30, 1), has_audio: false };
        let clip = edit::import_footage(&mut project, "/media/app.mov".into(), &info);
        let made = build(&mut project, &MockupRequest { footage: Some(clip), ..request(Device::IPhone, Orientation::Portrait) }).unwrap();
        let screen = project.composition(made.screen).unwrap();
        let layer = screen.layers.iter().find(|layer| layer.content == LayerContent::Footage { item: clip }).unwrap();
        let scale = layer.transform.as_ref().unwrap().scale.value;
        // 1920 × 1080 filling 1320 × 2868: scaled to the height.
        assert!((scale.x - 2868.0 / 1080.0 * 100.0).abs() < 0.01);
        assert!(!screen.layers.iter().any(|layer| layer.name.starts_with("Drop your")), "no drop spot when there is footage");
        assert!(build(&mut project, &MockupRequest { footage: Some(Id(999_999)), ..request(Device::IPad, Orientation::Portrait) }).is_none());
    }

    #[test]
    fn the_watch_and_browser_have_their_own_finishes_and_parts() {
        let names = |device: Device| device.finishes().iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert_eq!(names(Device::Watch), ["Jet Black", "Silver", "Rose Gold", "Natural Titanium"]);
        assert_eq!(names(Device::Browser), ["Light", "Dark"]);
        assert_eq!(names(Device::IPhone), ["Space Black", "Silver", "Midnight", "Gold"]);
        let parts = |device: Device| {
            let mut project = Project::new(Vec::new(), Vec::new());
            let finish = device.finishes()[0].1;
            let made = build(&mut project, &MockupRequest { device, orientation: Orientation::Landscape, finish, footage: None, seconds: 10.0 }).unwrap();
            project.composition(made.composition).unwrap().layers.iter().map(|layer| layer.name.clone()).collect::<Vec<_>>()
        };
        let watch = parts(Device::Watch);
        assert_eq!(watch.iter().filter(|name| name.starts_with("Band")).count(), 2, "{watch:?}");
        let browser = parts(Device::Browser);
        for name in ["Close", "Minimize", "Zoom", "Address bar"] {
            assert!(browser.iter().any(|layer| layer == name), "{name} in {browser:?}");
        }
        // Neither turns on its side: asked for sideways, it still stands upright.
        assert!(!Device::Watch.turns() && !Device::Browser.turns());
        let mut project = Project::new(Vec::new(), Vec::new());
        let made = build(&mut project, &MockupRequest { device: Device::Browser, orientation: Orientation::Landscape, finish: Device::Browser.finishes()[1].1, footage: None, seconds: 10.0 }).unwrap();
        assert_eq!((project.composition(made.screen).unwrap().width, project.composition(made.screen).unwrap().height), (1920, 1080));
    }

    #[test]
    fn mockups_go_in_their_own_folder() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let made = build(&mut project, &request(Device::Mac, Orientation::Landscape)).unwrap();
        let folder = crate::folders::top_folder(&project, "Mockups").unwrap();
        assert_eq!(project.item(made.composition).unwrap().parent, Some(folder));
        assert_eq!(project.item(made.screen).unwrap().parent, Some(folder));
        assert_eq!(project.composition(made.screen).unwrap().width, 2880, "a Mac stays upright");
    }
}
