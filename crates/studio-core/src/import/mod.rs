//! Importing projects made in other apps: After Effects projects (`.aep`, see `aep`) and Lottie
//! animations (`.json`, see `lottie`).
//!
//! Each reader turns its file into a `Scene`, a plain description of folders, sources,
//! compositions and layers in seconds and pixels, and `build` turns a scene into Studio items
//! and compositions inside an existing project, under a folder named after the file. Anything
//! Studio can't show yet (cameras, lights, effects, expressions) is listed in the report rather
//! than silently dropped.
//!
//! Files are untrusted: the readers check every size and count, and the result is checked like
//! a loaded project file before it is added, so a damaged file can't damage the project.

pub mod aep;
pub mod lottie;
pub mod svg;
mod riff;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::animation::{Animated, CubicBezier, Easing, Keyframe, Vec2};
use crate::edit::{self, CompositionSettings, FootageInfo};
use crate::model::{Id, LabelColor, LayerContent, Paint, Project, ProjectItem, ProjectItemKind, Rgba, ShapeKind, ShapeStyle, Stroke, TextAlign, TextStyle};
use crate::path::{PathPoint, ShapePath};
use crate::time::FrameRate;

/// What a media file turned out to be, from the app's probe (the core can't read media).
#[derive(Clone, Debug, PartialEq)]
pub enum Probed {
    Image(Vec2),
    Footage(FootageInfo),
    Audio(f64),
}

/// A whole imported file, before it becomes Studio items.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub folders: Vec<SceneFolder>,
    pub sources: Vec<SceneSource>,
    pub comps: Vec<SceneComp>,
    /// Things the file holds that weren't brought in, for the report.
    pub notes: Vec<String>,
    /// Put what it holds at the top of the Project panel, not in a folder named after the
    /// file (for a file that is a single composition, such as an SVG).
    pub loose: bool,
}

/// A folder in the Project panel. Keys are the file's own ids, unique among folders, sources
/// and compositions together.
#[derive(Clone, Debug)]
pub struct SceneFolder {
    pub key: u64,
    pub name: String,
    pub folder: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct SceneSource {
    pub key: u64,
    pub name: String,
    pub folder: Option<u64>,
    pub kind: SourceKind,
}

#[derive(Clone, Debug)]
pub enum SourceKind {
    /// A media file. If it can't be found, a gray solid of `size` stands in for it.
    File { path: PathBuf, size: Vec2 },
    Solid { size: Vec2, color: Rgba },
}

#[derive(Clone, Debug)]
pub struct SceneComp {
    pub key: u64,
    pub name: String,
    pub folder: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub seconds: f64,
    pub background: Rgba,
    /// Top layer first.
    pub layers: Vec<SceneLayer>,
}

#[derive(Clone, Debug)]
pub struct SceneLayer {
    /// The file's id for the layer, unique in its composition (for parenting).
    pub key: u64,
    pub name: String,
    pub parent: Option<u64>,
    /// Composition seconds: where the source's start plays, and the visible span.
    pub start: f64,
    pub in_point: f64,
    pub out_point: f64,
    pub visible: bool,
    pub content: SceneContent,
    pub transform: SceneTransform,
}

#[derive(Clone, Debug)]
pub enum SceneContent {
    Source(u64),
    Comp(u64),
    /// Moves its children; isn't drawn.
    Null,
    Adjustment,
    Text(SceneText),
    Shapes(Vec<SceneShape>),
    /// One vector shape as a layer of its own (an SVG shape): its outline in composition
    /// pixels, and how it is filled and stroked.
    Vector(Box<SceneVector>),
}

/// A vector shape that becomes one shape layer (see `SceneContent::Vector`).
#[derive(Clone, Debug)]
pub struct SceneVector {
    pub path: ShapePath,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
}

#[derive(Clone, Debug)]
pub struct SceneText {
    pub text: String,
    pub font: Option<String>,
    pub size: f32,
    pub fill: Option<Rgba>,
    pub align: TextAlign,
}

/// One drawable of a shape layer, in the layer's own pixels (group transforms applied).
#[derive(Clone, Debug)]
pub struct SceneShape {
    pub geometry: Geometry,
    pub fill: Option<Rgba>,
    pub stroke: Option<(Rgba, f32)>,
}

#[derive(Clone, Debug)]
pub enum Geometry {
    Rect { center: Vec2, size: Vec2, radius: f32, rotation: f32 },
    Ellipse { center: Vec2, size: Vec2, rotation: f32 },
    /// Points with their handles as offsets, as in `crate::path`.
    Path(ShapePath),
}

/// A layer's transform; `None` parts keep Studio's default for the layer.
#[derive(Clone, Debug, Default)]
pub struct SceneTransform {
    pub anchor: Option<Track<Vec2>>,
    pub position: Option<Track<Vec2>>,
    /// Percent.
    pub scale: Option<Track<Vec2>>,
    /// Degrees.
    pub rotation: Option<Track<f32>>,
    /// Percent.
    pub opacity: Option<Track<f32>>,
}

/// A value, or keyframes of it at composition seconds. Each key's easing shapes the way to
/// the next one.
#[derive(Clone, Debug)]
pub struct Track<T> {
    pub value: T,
    pub keys: Vec<(f64, T, Easing)>,
}

impl<T: Copy> Track<T> {
    pub fn fixed(value: T) -> Self {
        Self { value, keys: Vec::new() }
    }

    /// The value at `seconds`, holding each key until the next (no easing between them).
    fn held_at(&self, seconds: f64) -> T {
        self.keys.iter().rev().find(|(at, _, _)| *at <= seconds).map_or(self.value, |(_, value, _)| *value)
    }
}

/// A position keyed as separate X and Y tracks, as one track. The axis with more keys drives
/// the timing; the other is sampled at those times (held, not eased), so an animation on
/// either axis alone comes through.
pub fn combine_axes(x: &Track<f32>, y: &Track<f32>) -> Track<Vec2> {
    if x.keys.len() >= y.keys.len() {
        Track { value: Vec2::new(x.value, y.value), keys: x.keys.iter().map(|&(at, value, easing)| (at, Vec2::new(value, y.held_at(at)), easing)).collect() }
    } else {
        Track { value: Vec2::new(x.value, y.value), keys: y.keys.iter().map(|&(at, value, easing)| (at, Vec2::new(x.held_at(at), value), easing)).collect() }
    }
}

/// What came in: the compositions made (the main one first) and what was left out.
#[derive(Clone, Debug, Default)]
pub struct Imported {
    pub folder: Option<Id>,
    pub compositions: Vec<Id>,
    pub notes: Vec<String>,
}

/// The most layers one composition may bring in, so a hostile file can't ask for millions.
const MAX_LAYERS: usize = 5_000;

/// A Studio frame rate close to `fps`: the broadcast rates exactly, others to a thousandth.
pub fn frame_rate(fps: f64) -> FrameRate {
    let fps = if fps.is_finite() { fps.clamp(1.0, 240.0) } else { 30.0 };
    for (rate, numerator) in [(23.976, 24_000), (29.97, 30_000), (47.952, 48_000), (59.94, 60_000), (119.88, 120_000)] {
        if (fps - rate).abs() < 0.005 {
            return FrameRate::new(numerator, 1001);
        }
    }
    if (fps - fps.round()).abs() < 0.001 {
        FrameRate::new(fps.round() as u32, 1)
    } else {
        FrameRate::new((fps * 1000.0).round() as u32, 1000)
    }
}

/// An easing from After Effects-style handles: how long each handle reaches along the segment
/// (influence, 0–1) and how fast the value moves there (`speed`, units per second), for a
/// segment of `seconds` changing by `change`. Linear or hold sides are passed as `None`.
pub fn bezier_from_speeds(out: Option<(f64, f64)>, into: Option<(f64, f64)>, seconds: f64, change: f64) -> Easing {
    if out.is_none() && into.is_none() {
        return Easing::Linear;
    }
    let slope = |speed: f64, influence: f64| if change.abs() > 1e-9 { speed * influence * seconds / change } else { 0.0 };
    let (x1, y1) = out.map_or((1.0 / 3.0, 1.0 / 3.0), |(speed, influence)| (influence, slope(speed, influence)));
    let (x2, y2) = into.map_or((2.0 / 3.0, 2.0 / 3.0), |(speed, influence)| (1.0 - influence, 1.0 - slope(speed, influence)));
    let clean = |v: f64, low: f64, high: f64| if v.is_finite() { v.clamp(low, high) as f32 } else { 0.5 };
    Easing::Curve(CubicBezier::new(clean(x1, 0.0, 1.0), clean(y1, -2.0, 3.0), clean(x2, 0.0, 1.0), clean(y2, -2.0, 3.0)))
}

/// Adds `scene` to `project` under a new folder named `name`. `probe` says what each media
/// file is; `None` means it couldn't be found or read. The project is changed only if
/// everything checks out.
pub fn build(project: &mut Project, scene: &Scene, name: &str, probe: &dyn Fn(&Path) -> Option<Probed>) -> Result<Imported, String> {
    if scene.comps.is_empty() {
        return Err("The file holds no compositions to bring in.".into());
    }
    let mut working = project.clone();
    let mut notes = scene.notes.clone();
    let root = (!scene.loose).then(|| crate::folders::add_folder(&mut working, name.trim(), None));

    // Folders, parents before children (the file's order may not be).
    let mut folders: HashMap<u64, Id> = HashMap::new();
    let mut pending: Vec<&SceneFolder> = scene.folders.iter().collect();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|folder| {
            let parent = match folder.folder {
                None => root,
                Some(key) => folders.get(&key).copied(),
            };
            let Some(parent) = parent else { return true };
            let id = crate::folders::add_folder(&mut working, &folder.name, Some(parent));
            folders.insert(folder.key, id);
            false
        });
        if pending.len() == before {
            // Folders inside folders that don't exist (or loop): put them at the top.
            for folder in pending.drain(..) {
                let id = crate::folders::add_folder(&mut working, &folder.name, root);
                folders.insert(folder.key, id);
            }
        }
    }
    let folder_of = |key: Option<u64>| key.and_then(|key| folders.get(&key).copied()).or(root);

    // Sources.
    let mut sources: HashMap<u64, Id> = HashMap::new();
    for source in &scene.sources {
        let item = match &source.kind {
            SourceKind::Solid { size, color } => solid_item(&mut working, &source.name, *size, *color),
            SourceKind::File { path, size } => match probe(path) {
                Some(Probed::Image(size)) => edit::import_image(&mut working, path.clone(), size),
                Some(Probed::Footage(info)) => edit::import_footage(&mut working, path.clone(), &info),
                Some(Probed::Audio(seconds)) => edit::import_audio(&mut working, path.clone(), seconds),
                None => {
                    notes.push(format!("“{}” wasn't found at {}; a gray solid stands in for it.", source.name, path.display()));
                    solid_item(&mut working, &format!("Missing: {}", source.name), *size, Rgba::hex(0x555555))
                }
            },
        };
        if let Some(found) = working.items.iter_mut().find(|candidate| candidate.id == item) {
            found.name = source.name.clone();
            found.parent = folder_of(source.folder);
        }
        sources.insert(source.key, item);
    }

    // Compositions first (layers may use ones listed later), then their layers.
    let mut comps: HashMap<u64, Id> = HashMap::new();
    for comp in &scene.comps {
        let rate = frame_rate(comp.fps);
        let duration = ((comp.seconds.max(0.0) * rate.fps()).round() as i64).clamp(1, crate::model::MAX_FRAME / 2);
        let settings = CompositionSettings {
            name: if comp.name.trim().is_empty() { "Comp".into() } else { comp.name.clone() },
            width: comp.width.clamp(1, CompositionSettings::MAX_SIZE),
            height: comp.height.clamp(1, CompositionSettings::MAX_SIZE),
            frame_rate: rate,
            duration_frames: duration,
            background: comp.background,
        };
        let id = edit::new_composition(&mut working, &settings).map_err(|error| format!("“{}” couldn't be made: {error}", comp.name))?;
        if let Some(item) = working.items.iter_mut().find(|item| item.id == id) {
            item.parent = folder_of(comp.folder);
        }
        comps.insert(comp.key, id);
    }
    for comp in &scene.comps {
        let id = comps[&comp.key];
        if comp.layers.len() > MAX_LAYERS {
            notes.push(format!("“{}” has more than {MAX_LAYERS} layers; only the top ones came in.", comp.name));
        }
        let mut made: HashMap<u64, Id> = HashMap::new();
        // Bottom layer first: each new layer goes on top.
        for layer in comp.layers.iter().take(MAX_LAYERS).rev() {
            if let Some(ids) = add_layer(&mut working, id, layer, &sources, &comps, &mut notes) {
                made.insert(layer.key, ids);
            }
        }
        // Parents, once every layer exists. Values are taken as they are in the file, which
        // already places children in their parent's space.
        for layer in &comp.layers {
            let (Some(&child), Some(parent)) = (made.get(&layer.key), layer.parent.and_then(|key| made.get(&key).copied())) else { continue };
            let composition = working.composition_mut(id).ok_or("a composition went missing")?;
            if edit::can_parent(composition, child, parent)
                && let Some(found) = composition.layer_mut(child)
            {
                found.parent = Some(parent);
            }
        }
        if let Some(composition) = working.composition_mut(id) {
            crate::tracks::normalize(composition);
        }
    }

    crate::project_file::check(&working).map_err(|error| format!("The imported project didn't check out, so nothing was added: {error}"))?;
    *project = working;
    // The main composition: the one no other uses, with the most layers.
    let used: std::collections::HashSet<u64> = scene.comps.iter().flat_map(|comp| &comp.layers).filter_map(|layer| match layer.content {
        SceneContent::Comp(key) => Some(key),
        _ => None,
    }).collect();
    let mut order: Vec<&SceneComp> = scene.comps.iter().collect();
    order.sort_by_key(|comp| (used.contains(&comp.key), std::cmp::Reverse(comp.layers.len())));
    notes.dedup();
    Ok(Imported { folder: root, compositions: order.iter().map(|comp| comps[&comp.key]).collect(), notes })
}

/// A solid project item.
fn solid_item(project: &mut Project, name: &str, size: Vec2, color: Rgba) -> Id {
    let id = project.allocate_id();
    project.items.push(ProjectItem {
        id,
        name: name.to_owned(),
        kind: ProjectItemKind::Solid,
        label: LabelColor::Red,
        parent: None,
        size: Some(Vec2::new(size.x.clamp(1.0, 30_000.0), size.y.clamp(1.0, 30_000.0))),
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: Some(color),
        path: None,
    });
    id
}

/// Makes one layer (for a shape layer: a holder with one child per shape). Returns the layer
/// other layers parent to.
fn add_layer(project: &mut Project, comp: Id, layer: &SceneLayer, sources: &HashMap<u64, Id>, comps: &HashMap<u64, Id>, notes: &mut Vec<String>) -> Option<Id> {
    let composition = project.composition(comp)?;
    let (fps, duration) = (composition.frame_rate.fps(), composition.duration_frames);
    let center = Vec2::new(composition.width as f32 * 0.5, composition.height as f32 * 0.5);
    let frame = |seconds: f64| (seconds * fps).round() as i64;
    let in_frame = frame(layer.in_point).clamp(0, duration - 1);
    let out_frame = frame(layer.out_point).clamp(in_frame + 1, duration.max(in_frame + 1));
    if frame(layer.out_point) <= 0 || frame(layer.in_point) >= duration {
        return None; // Never on screen in this composition.
    }

    let made = match &layer.content {
        SceneContent::Source(key) => edit::add_layer_from_item(project, comp, *sources.get(key)?, 0, in_frame)?,
        SceneContent::Comp(key) => match edit::add_layer_from_item(project, comp, *comps.get(key)?, 0, in_frame) {
            Some(id) => id,
            None => {
                notes.push(format!("“{}” uses a composition inside itself, so it was left out.", layer.name));
                return None;
            }
        },
        SceneContent::Null | SceneContent::Shapes(_) => {
            let item = solid_item(project, &layer.name, Vec2::new(100.0, 100.0), Rgba::hex(0xffffff));
            let id = edit::add_layer_from_item(project, comp, item, 0, in_frame)?;
            // The solid was only needed to make the layer.
            project.items.retain(|candidate| candidate.id != item);
            let found = project.composition_mut(comp)?.layer_mut(id)?;
            found.content = LayerContent::Solid { color: Rgba::TRANSPARENT };
            found.switches.visible = false;
            id
        }
        SceneContent::Adjustment => {
            let size = Vec2::new(center.x * 2.0, center.y * 2.0);
            let item = solid_item(project, &layer.name, size, Rgba::hex(0xffffff));
            let id = edit::add_layer_from_item(project, comp, item, 0, in_frame)?;
            project.items.retain(|candidate| candidate.id != item);
            let found = project.composition_mut(comp)?.layer_mut(id)?;
            found.content = LayerContent::Adjustment;
            found.switches.adjustment = true;
            id
        }
        SceneContent::Vector(vector) => {
            let style = ShapeStyle { fill: vector.fill, stroke: vector.stroke, ..ShapeStyle::new(ShapeKind::Path) };
            let id = edit::add_path_layer(project, comp, &vector.path, style, in_frame)?;
            // `add_path_layer` gives an open outline a stroke in place of its fill; a vector shape
            // keeps exactly the fill and stroke it was drawn with.
            if let Some(LayerContent::Shape { style, .. }) = project.composition_mut(comp)?.layer_mut(id).map(|layer| &mut layer.content) {
                style.fill = vector.fill;
                style.stroke = vector.stroke;
            }
            id
        }
        SceneContent::Text(text) => {
            let id = edit::add_text_layer(project, comp, &text.text, center, in_frame)?;
            let composition = project.composition_mut(comp)?;
            edit::edit_text(composition, id, |_, style| apply_text_style(style, text));
            id
        }
    };

    let composition = project.composition_mut(comp)?;
    let found = composition.layer_mut(made)?;
    found.name = if layer.name.trim().is_empty() { found.name.clone() } else { layer.name.clone() };
    found.in_frame = in_frame;
    found.out_frame = out_frame;
    found.start_frame = frame(layer.start).clamp(-crate::model::MAX_FRAME / 2, crate::model::MAX_FRAME / 2);
    if !layer.visible {
        found.switches.visible = false;
    }
    // Defaults the file leaves out: After Effects' (anchor at the source's middle, or the
    // origin for text and shapes, in the frame's middle, full size, upright, opaque).
    let size = found.size;
    let is_text = matches!(layer.content, SceneContent::Text(_));
    let default_anchor = match layer.content {
        SceneContent::Shapes(_) | SceneContent::Text(_) => Vec2::default(),
        _ => size * 0.5,
    };
    let text_origin = if is_text { text_origin(found) } else { Vec2::default() };
    if let Some(transform) = found.transform.as_mut() {
        let to_frames = |track: &Track<Vec2>, shift: Vec2| animated(track, fps, |value| value + shift);
        let anchor = layer.transform.anchor.clone().unwrap_or(Track::fixed(default_anchor));
        transform.anchor_point = to_frames(&anchor, text_origin);
        transform.position = to_frames(&layer.transform.position.clone().unwrap_or(Track::fixed(center)), Vec2::default());
        transform.scale = to_frames(&layer.transform.scale.clone().unwrap_or(Track::fixed(Vec2::new(100.0, 100.0))), Vec2::default());
        transform.rotation = animated(&layer.transform.rotation.clone().unwrap_or(Track::fixed(0.0)), fps, |value| value);
        transform.opacity = animated(&layer.transform.opacity.clone().unwrap_or(Track::fixed(100.0)), fps, |value| value.clamp(0.0, 100.0));
    }

    if let SceneContent::Shapes(shapes) = &layer.content {
        for shape in shapes.iter().rev() {
            add_shape(project, comp, made, shape, in_frame, out_frame);
        }
    }
    Some(made)
}

/// Where a text layer's own origin (After Effects': the start of the first line's baseline, or
/// its middle for centered text) sits in Studio's text box.
fn text_origin(layer: &crate::model::Layer) -> Vec2 {
    let LayerContent::Text { style, .. } = &layer.content else { return Vec2::default() };
    let x = match style.align {
        TextAlign::Left => 0.0,
        TextAlign::Center => layer.size.x * 0.5,
        TextAlign::Right => layer.size.x,
    };
    Vec2::new(x, style.font_size * 0.8)
}

fn apply_text_style(style: &mut TextStyle, text: &SceneText) {
    if let Some(font) = &text.font {
        let (family, weight) = font_family(font);
        style.font_family = family;
        style.font_weight = weight;
    }
    if text.size.is_finite() && text.size > 0.0 {
        style.font_size = text.size.clamp(1.0, 2_000.0);
        style.line_height = style.font_size * 1.2;
    }
    style.fill = text.fill;
    style.align = text.align;
}

/// A font's family and weight from its PostScript name ("MyriadPro-Bold" is Myriad Pro at 700).
pub fn font_family(postscript: &str) -> (String, u16) {
    let (family, style) = postscript.split_once('-').unwrap_or((postscript, ""));
    let style = style.to_ascii_lowercase();
    let weight = [("thin", 100), ("extralight", 200), ("light", 300), ("medium", 500), ("semibold", 600), ("demibold", 600), ("extrabold", 800), ("black", 900), ("heavy", 900), ("bold", 700)]
        .iter()
        .find(|(name, _)| style.contains(name))
        .map_or(400, |&(_, weight)| weight);
    // "MyriadPro" reads as "Myriad Pro": a space before each capital that follows a lower-case letter.
    let mut spaced = String::new();
    let mut previous_lower = false;
    for c in family.chars() {
        if c.is_uppercase() && previous_lower {
            spaced.push(' ');
        }
        previous_lower = c.is_lowercase();
        spaced.push(c);
    }
    (spaced, weight)
}

/// A keyframed (or fixed) value in composition frames, with `map` applied to every value.
fn animated<T: Copy + crate::animation::Lerp>(track: &Track<T>, fps: f64, map: impl Fn(T) -> T) -> Animated<T> {
    if track.keys.is_empty() {
        return Animated::constant(map(track.value));
    }
    let mut keys: Vec<Keyframe<T>> = Vec::new();
    for &(seconds, value, easing) in &track.keys {
        let frame = (seconds * fps).round() as i64;
        if !seconds.is_finite() || frame.abs() > crate::model::MAX_FRAME / 2 {
            continue;
        }
        // Keys that land on one frame (sub-frame keys): the later one wins.
        keys.retain(|key| key.frame != frame);
        keys.push(Keyframe { frame, value: map(value), easing });
    }
    keys.sort_by_key(|key| key.frame);
    if keys.is_empty() { Animated::constant(map(track.value)) } else { Animated::keyframed(keys) }
}

/// One shape of a shape layer, as a child of the (hidden) layer holding them.
fn add_shape(project: &mut Project, comp: Id, holder: Id, shape: &SceneShape, in_frame: i64, out_frame: i64) {
    let fill = shape.fill.map(Paint::Solid);
    let stroke = shape.stroke.filter(|(_, width)| width.is_finite() && *width > 0.0).map(|(color, width)| Stroke { paint: Paint::Solid(color), width: width.min(crate::model::MAX_STROKE_WIDTH) });
    let (made, rotation) = match &shape.geometry {
        Geometry::Rect { center, size, radius, rotation } => {
            let style = ShapeStyle { fill, stroke, corner_radius: radius.max(0.0).min(size.x.min(size.y) * 0.5), ..ShapeStyle::new(ShapeKind::Rectangle) };
            (edit::add_shape_layer(project, comp, style, *size, *center, in_frame), *rotation)
        }
        Geometry::Ellipse { center, size, rotation } => {
            let style = ShapeStyle { fill, stroke, ..ShapeStyle::new(ShapeKind::Ellipse) };
            (edit::add_shape_layer(project, comp, style, *size, *center, in_frame), *rotation)
        }
        Geometry::Path(path) => {
            let style = ShapeStyle { fill, stroke, ..ShapeStyle::new(ShapeKind::Path) };
            (edit::add_path_layer(project, comp, path, style, in_frame), 0.0)
        }
    };
    let Some(made) = made else { return };
    let Some(composition) = project.composition_mut(comp) else { return };
    if let Some(layer) = composition.layer_mut(made) {
        layer.parent = Some(holder);
        layer.out_frame = out_frame;
        if let Some(transform) = layer.transform.as_mut()
            && rotation.is_finite()
        {
            transform.rotation = Animated::constant(rotation);
        }
    }
}

/// A 2D affine transform (for shape groups): `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    pub const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    /// A group's transform: moved to `position`, around `anchor`, scaled (percent) and turned (degrees).
    pub fn group(anchor: Vec2, position: Vec2, scale: Vec2, rotation: f32) -> Self {
        let (sin, cos) = rotation.to_radians().sin_cos();
        let (sx, sy) = (scale.x / 100.0, scale.y / 100.0);
        let (a, b, c, d) = (cos * sx, sin * sx, -sin * sy, cos * sy);
        Self { a, b, c, d, e: position.x - (a * anchor.x + c * anchor.y), f: position.y - (b * anchor.x + d * anchor.y) }
    }

    /// `inner` applied first, then `self`.
    pub fn then_outer(self, outer: Self) -> Self {
        Self {
            a: outer.a * self.a + outer.c * self.b,
            b: outer.b * self.a + outer.d * self.b,
            c: outer.a * self.c + outer.c * self.d,
            d: outer.b * self.c + outer.d * self.d,
            e: outer.a * self.e + outer.c * self.f + outer.e,
            f: outer.b * self.e + outer.d * self.f + outer.f,
        }
    }

    pub fn point(self, p: Vec2) -> Vec2 {
        Vec2::new(self.a * p.x + self.c * p.y + self.e, self.b * p.x + self.d * p.y + self.f)
    }

    pub fn vector(self, v: Vec2) -> Vec2 {
        Vec2::new(self.a * v.x + self.c * v.y, self.b * v.x + self.d * v.y)
    }

    /// How much it stretches each way, and how far it turns (degrees).
    pub fn scale_and_rotation(self) -> (Vec2, f32) {
        (Vec2::new(self.a.hypot(self.b), self.c.hypot(self.d)), self.b.atan2(self.a).to_degrees())
    }

    /// A shape `geometry` drawn under this transform.
    pub fn geometry(self, geometry: Geometry) -> Geometry {
        let (stretch, turn) = self.scale_and_rotation();
        match geometry {
            Geometry::Rect { center, size, radius, rotation } => Geometry::Rect { center: self.point(center), size: Vec2::new(size.x * stretch.x, size.y * stretch.y), radius: radius * stretch.x.min(stretch.y), rotation: rotation + turn },
            Geometry::Ellipse { center, size, rotation } => Geometry::Ellipse { center: self.point(center), size: Vec2::new(size.x * stretch.x, size.y * stretch.y), rotation: rotation + turn },
            Geometry::Path(path) => Geometry::Path(ShapePath {
                points: path.points.iter().map(|point| PathPoint { at: self.point(point.at), handle_in: self.vector(point.handle_in), handle_out: self.vector(point.handle_out) }).collect(),
                closed: path.closed,
                ..Default::default()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates_snap_to_broadcast_rates() {
        assert_eq!(frame_rate(23.976_023), FrameRate::new(24_000, 1001));
        assert_eq!(frame_rate(30.0), FrameRate::new(30, 1));
        assert_eq!(frame_rate(12.5), FrameRate::new(12_500, 1000));
    }

    #[test]
    fn handle_speeds_become_curves() {
        // Ease out of the first key and into the second: zero speed at both ends.
        let Easing::Curve(curve) = bezier_from_speeds(Some((0.0, 1.0 / 3.0)), Some((0.0, 1.0 / 3.0)), 1.0, 100.0) else { panic!() };
        assert_eq!((curve.x1, curve.y1, curve.x2, curve.y2), (1.0 / 3.0, 0.0, 2.0 / 3.0, 1.0));
        assert_eq!(bezier_from_speeds(None, None, 1.0, 100.0), Easing::Linear);
    }

    #[test]
    fn postscript_names_become_families_and_weights() {
        assert_eq!(font_family("MyriadPro-Bold"), ("Myriad Pro".to_owned(), 700));
        assert_eq!(font_family("Inter"), ("Inter".to_owned(), 400));
        assert_eq!(font_family("GlutenDefault-Light"), ("Gluten Default".to_owned(), 300));
    }

    #[test]
    fn group_transforms_compose() {
        let inner = Affine::group(Vec2::default(), Vec2::new(10.0, 0.0), Vec2::new(200.0, 200.0), 0.0);
        let outer = Affine::group(Vec2::default(), Vec2::new(0.0, 5.0), Vec2::new(100.0, 100.0), 90.0);
        let p = inner.then_outer(outer).point(Vec2::new(1.0, 0.0));
        assert!((p.x - 0.0).abs() < 1e-4 && (p.y - 17.0).abs() < 1e-4, "{p:?}");
    }
}
