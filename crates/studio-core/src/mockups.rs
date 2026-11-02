//! Device mockups: iPhone, iPhone Duo, iPad and Mac, drawn as shapes in the style of
//! OpenConnect's device frames (a body in one color with a thin outline, side buttons, the
//! Dynamic Island or the notch), with their screens at the sizes App Store screenshots use.
//!
//! Making one adds two compositions in a "Mockups" folder:
//!
//! - **the screen**, at the device's own screen size, where footage goes (it starts with a drop
//!   spot, or with the footage asked for, scaled to fill it), and
//! - **the mockup**, which shows the device with the screen composition inside it, cut to the
//!   screen's rounded corners by a mask layer. Every part hangs off a "Device" null, so moving,
//!   scaling or turning that moves the whole device.
//!
//! Edit the screen composition and every mockup using it follows.

use crate::animation::{Animated, Vec2};
use crate::edit::{self, CompositionSettings};
use crate::model::{Id, LayerContent, LayerMask, MaskMode, Paint, Project, Rgba, ShapeKind, ShapeStyle, Stroke};
use crate::templates::Format;
use crate::time::FrameRate;

/// The devices mockups come in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    IPhone,
    Duo,
    IPad,
    Mac,
}

impl Device {
    pub const ALL: [Self; 4] = [Self::IPhone, Self::Duo, Self::IPad, Self::Mac];

    pub fn name(self) -> &'static str {
        match self {
            Self::IPhone => "iPhone",
            Self::Duo => "iPhone Duo",
            Self::IPad => "iPad",
            Self::Mac => "Mac",
        }
    }

    /// The screen in pixels, upright: the App Store screenshot sizes OpenConnect designs at.
    pub fn screen(self) -> (u32, u32) {
        match self {
            Self::IPhone => (1320, 2868),
            Self::Duo => (2007, 2853),
            Self::IPad => (2064, 2752),
            Self::Mac => (2880, 1800),
        }
    }

    /// Whether the device can be turned on its side (a Mac only stands one way).
    pub fn turns(self) -> bool {
        self != Self::Mac
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

/// How the device looks: OpenConnect's frame style (body color, outline color and width, and
/// how dark its shadow is), and what goes behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    pub body: Rgba,
    pub outline: Rgba,
    /// Outline width as OpenConnect sets it: pixels on a frame about 1,100 pixels wide.
    pub outline_width: f32,
    /// Shadow strength, 0 to 1.
    pub shadow: f32,
    /// A background behind the device, or `None` to leave the mockup see-through.
    pub background: Option<Rgba>,
}

impl Finish {
    /// Ready-made finishes, by name.
    pub fn presets() -> [(&'static str, Finish); 4] {
        let finish = |body: u32, outline: u32, shadow: f32, background: u32| Finish { body: Rgba::hex(body), outline: Rgba::hex(outline), outline_width: 4.0, shadow, background: Some(Rgba::hex(background)) };
        [
            ("Space Black", finish(0x1c1c1e, 0x48484a, 0.55, 0x101114)),
            ("Silver", finish(0xe3e4e6, 0xb4b6b9, 0.4, 0xf2f2f4)),
            ("Midnight", finish(0x1c2433, 0xa0b8ce, 0.6, 0x16202f)),
            ("Gold", finish(0xf1e3cf, 0xc9ae86, 0.45, 0xfaf4ea)),
        ]
    }
}

/// What to make.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MockupRequest {
    pub device: Device,
    pub orientation: Orientation,
    /// The mockup composition's shape.
    pub format: Format,
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

/// A device's drawing, upright, in device pixels around its middle.
struct Drawing {
    body: Part,
    screen: Part,
    /// Buttons on the edges, drawn behind the body.
    buttons: Vec<Part>,
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
            }
        }
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

    // The mockup.
    let (cw, ch) = request.format.size();
    let settings = CompositionSettings { name: format!("{label} Mockup {}", request.format.label()), width: cw, height: ch, frame_rate: rate, duration_frames: duration, background: Rgba::TRANSPARENT };
    let comp = edit::new_composition(project, &settings).ok()?;
    let finish = request.finish;
    if let Some(color) = finish.background {
        edit::add_background(project, comp, color)?;
    }
    let drawing = drawing(device);
    let turn = |part: Part| part.turned(orientation);
    // Everything, buttons and base included, fits in 82% of the frame.
    let mut extent = Vec2::new(turn(drawing.body).size.x * 0.5, turn(drawing.body).size.y * 0.5);
    for part in drawing.buttons.iter().chain(drawing.base.iter().map(|(base, _)| base)).copied().map(turn) {
        extent.x = extent.x.max(part.center.x.abs() + part.size.x * 0.5);
        extent.y = extent.y.max(part.center.y.abs() + part.size.y * 0.5);
    }
    let scale = (cw as f32 * 0.82 / (extent.x * 2.0)).min(ch as f32 * 0.82 / (extent.y * 2.0));
    let middle = Vec2::new(cw as f32 * 0.5, ch as f32 * 0.5);
    let place = |part: Part| (middle + turn(part).center * scale, turn(part).size * scale, turn(part).corner * scale);
    let outline = finish.outline_width * drawing.body.size.x.min(drawing.body.size.y) / 1100.0 * scale;

    // Bottom to top: buttons and the base behind the body, then the body, the screen and what
    // sits over it.
    let mut parts = Vec::new();
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

    fn request(device: Device, orientation: Orientation, format: Format) -> MockupRequest {
        MockupRequest { device, orientation, format, finish: Finish::presets()[0].1, footage: None, seconds: 10.0 }
    }

    #[test]
    fn every_device_makes_a_screen_and_a_masked_mockup() {
        for device in Device::ALL {
            for orientation in [Orientation::Portrait, Orientation::Landscape] {
                for format in Format::ALL {
                    let mut project = Project::new(Vec::new(), Vec::new());
                    let made = build(&mut project, &request(device, orientation, format)).unwrap();
                    let screen = project.composition(made.screen).unwrap();
                    let (w, h) = device.screen();
                    let turned = device.turns() && orientation == Orientation::Landscape;
                    assert_eq!((screen.width, screen.height), if turned { (h, w) } else { (w, h) }, "{device:?} {orientation:?}");
                    let mockup = project.composition(made.composition).unwrap();
                    assert_eq!((mockup.width, mockup.height), format.size());
                    let shown = mockup.layers.iter().find(|layer| layer.content == LayerContent::Composition { composition: made.screen }).unwrap();
                    let mask = mockup.layer(shown.mask.unwrap().layer).unwrap();
                    assert!(!mask.switches.visible, "the mask only shapes the screen");
                    // The screen fits its hole exactly.
                    let scale = shown.transform.as_ref().unwrap().scale.value;
                    let shown_size = Vec2::new(shown.size.x * scale.x, shown.size.y * scale.y) * 0.01;
                    assert!((shown_size.x - mask.size.x).abs() < 0.5 && (shown_size.y - mask.size.y).abs() < 0.5, "{device:?}: {shown_size:?} vs {:?}", mask.size);
                    // Everything but the background follows the Device null, and it all fits the frame.
                    let null = mockup.layers.iter().find(|layer| layer.name == "Device").unwrap().id;
                    assert!(mockup.layers.iter().filter(|layer| layer.name != "Background" && layer.id != null).all(|layer| layer.parent == Some(null)));
                    let view = crate::project_file::EditorView { active_composition: made.composition, current_frame: 0, work_area: crate::time::FrameRange::new(0.0, mockup.duration_frames as f64) };
                    crate::project_file::from_text(&crate::project_file::to_text(&project, view).unwrap()).unwrap();
                }
            }
        }
    }

    #[test]
    fn footage_fills_the_screen() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let info = edit::FootageInfo { size: Vec2::new(1920.0, 1080.0), duration_seconds: 5.0, frame_rate: FrameRate::new(30, 1), has_audio: false };
        let clip = edit::import_footage(&mut project, "/media/app.mov".into(), &info);
        let made = build(&mut project, &MockupRequest { footage: Some(clip), ..request(Device::IPhone, Orientation::Portrait, Format::Vertical) }).unwrap();
        let screen = project.composition(made.screen).unwrap();
        let layer = screen.layers.iter().find(|layer| layer.content == LayerContent::Footage { item: clip }).unwrap();
        let scale = layer.transform.as_ref().unwrap().scale.value;
        // 1920 × 1080 filling 1320 × 2868: scaled to the height.
        assert!((scale.x - 2868.0 / 1080.0 * 100.0).abs() < 0.01);
        assert!(!screen.layers.iter().any(|layer| layer.name.starts_with("Drop your")), "no drop spot when there is footage");
        assert!(build(&mut project, &MockupRequest { footage: Some(Id(999_999)), ..request(Device::IPad, Orientation::Portrait, Format::Wide) }).is_none());
    }

    #[test]
    fn mockups_go_in_their_own_folder() {
        let mut project = Project::new(Vec::new(), Vec::new());
        let made = build(&mut project, &request(Device::Mac, Orientation::Landscape, Format::Wide)).unwrap();
        let folder = crate::folders::top_folder(&project, "Mockups").unwrap();
        assert_eq!(project.item(made.composition).unwrap().parent, Some(folder));
        assert_eq!(project.item(made.screen).unwrap().parent, Some(folder));
        assert_eq!(project.composition(made.screen).unwrap().width, 2880, "a Mac stays upright");
    }
}
