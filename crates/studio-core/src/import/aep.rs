//! Reading After Effects projects (`.aep`). Adobe doesn't publish the format; what is read here
//! was worked out from real project files and matches the open-source parsers (for example
//! `aep-parser` and `aftereffects-aep-parser`). It brings in:
//!
//! - folders, compositions (size, frame rate, length) and footage (by file path; solids by
//!   size and color),
//! - layers: footage, solids, nested compositions, nulls, adjustment layers, text (the words,
//!   font, size, color and alignment) and shape layers (rectangles, ellipses and paths with
//!   their fill and stroke),
//! - each layer's timing, visibility, parent, and transform (anchor point, position, scale,
//!   rotation, opacity) with its keyframes and their easing.
//!
//! Cameras, lights, 3D models, effects, masks, expressions and blend modes aren't read; the
//! report says which layers had them.
//!
//! The file is RIFX (see `riff`): a `Fold` list holds the project's items, each an `Item` list
//! with an `idta` header (type, id) and a `Utf8` name. Compositions hold a `cdta` header and one
//! `Layr` list per layer, whose `ldta` header holds its timing, source and flags; a layer's
//! properties are a tree of `tdgp` groups and `tdbs` properties, each named by the `tdmn`
//! chunk before it (After Effects' match names, such as "ADBE Position").

use std::collections::HashMap;
use std::path::PathBuf;

use super::riff::{Chunk, Chunks, f32_at, f64_at, i32_at, u16_at, u32_at};
use super::{Affine, Geometry, Scene, SceneComp, SceneContent, SceneFolder, SceneLayer, SceneShape, SceneSource, SceneText, SceneTransform, SourceKind, Track, bezier_from_speeds};
use crate::animation::{Easing, Vec2};
use crate::model::{Rgba, TextAlign};
use crate::path::{PathPoint, ShapePath};

/// Item types in an `idta` header.
const FOLDER: u16 = 1;
const COMPOSITION: u16 = 4;
const FOOTAGE: u16 = 7;
/// Layer types in an `ldta` header.
const LAYER_LIGHT: u8 = 1;
const LAYER_CAMERA: u8 = 2;
const LAYER_TEXT: u8 = 3;
const LAYER_SHAPE: u8 = 4;
const LAYER_MODEL: u8 = 7;
/// Keyframe interpolation types.
const BEZIER: u8 = 2;
const HOLD: u8 = 3;
/// The most keyframes and path points read from one property, against hostile files.
const MAX_KEYS: usize = 100_000;
/// How deep shape groups may nest.
const MAX_DEPTH: usize = 32;

/// Reads an After Effects project into a scene.
pub fn read(bytes: &[u8]) -> Result<Scene, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFX" || &bytes[8..12] != b"Egg!" {
        return Err("This isn't an After Effects project (.aep). Projects saved as XML (.aepx) aren't read yet.".into());
    }
    let top = Chunks { data: &bytes[12..] };
    let fold = top.into_iter().find(|chunk| chunk.is_list(b"Fold")).ok_or("The project holds no items.")?;
    let mut reader = Reader { scene: Scene::default(), kinds: HashMap::new(), names: HashMap::new(), skipped: Skipped::default() };
    reader.collect_kinds(fold, 0);
    reader.items(fold, None, 0);
    reader.skipped.report(&mut reader.scene.notes);
    Ok(reader.scene)
}

/// What wasn't brought in, counted for the report.
#[derive(Default)]
struct Skipped {
    cameras: usize,
    lights: usize,
    models: usize,
    effects: Vec<String>,
    masks: Vec<String>,
}

impl Skipped {
    fn report(&self, notes: &mut Vec<String>) {
        let list = |names: &[String]| {
            let shown: Vec<&str> = names.iter().take(5).map(String::as_str).collect();
            let more = names.len().saturating_sub(5);
            if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") }
        };
        if self.cameras + self.lights + self.models > 0 {
            notes.push(format!("Cameras ({}), lights ({}) and 3D models ({}) aren't supported yet and were left out.", self.cameras, self.lights, self.models));
        }
        if !self.effects.is_empty() {
            notes.push(format!("Effects weren't brought in, on {}.", list(&self.effects)));
        }
        if !self.masks.is_empty() {
            notes.push(format!("Masks weren't brought in, on {}.", list(&self.masks)));
        }
    }
}

struct Reader {
    scene: Scene,
    /// Every item's type and name, by id, so layers can tell what they show.
    kinds: HashMap<u32, u16>,
    names: HashMap<u32, String>,
    skipped: Skipped,
}

/// An item's type, id and name. After Effects saves footage that goes by its file's name with
/// no name of its own, so such items are named after the file.
fn item_header(item: Chunk<'_>) -> Option<(u16, u32, String)> {
    let header = item.find(b"idta")?;
    let kind = u16_at(header, 0)?;
    let mut name = item.find(b"Utf8").map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default();
    if name.is_empty() && kind == FOOTAGE {
        name = item.list(b"Pin ").and_then(footage_file).and_then(|path| path.file_name().map(|file| file.to_string_lossy().into_owned())).unwrap_or_default();
    }
    Some((kind, u32_at(header, 16)?, name))
}

/// The items directly inside a folder: the project's own list, or a folder's `Sfdr`.
fn folder_items(folder: Chunk<'_>) -> impl Iterator<Item = Chunk<'_>> {
    let nested = folder.list(b"Sfdr").map(|list| list.children());
    folder.children().chain(nested.into_iter().flatten()).filter(|chunk| chunk.is_list(b"Item"))
}

impl Reader {
    fn collect_kinds(&mut self, folder: Chunk<'_>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for item in folder_items(folder) {
            let Some((kind, id, name)) = item_header(item) else { continue };
            self.kinds.insert(id, kind);
            self.names.insert(id, name);
            if kind == FOLDER {
                self.collect_kinds(item, depth + 1);
            }
        }
    }

    fn items(&mut self, folder: Chunk<'_>, parent: Option<u64>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for item in folder_items(folder) {
            let Some((kind, id, name)) = item_header(item) else { continue };
            let key = u64::from(id);
            match kind {
                FOLDER => {
                    self.scene.folders.push(SceneFolder { key, name, folder: parent });
                    self.items(item, Some(key), depth + 1);
                }
                COMPOSITION => {
                    if let Some(comp) = self.composition(item, key, name, parent) {
                        self.scene.comps.push(comp);
                    }
                }
                FOOTAGE => {
                    if let Some(source) = footage(item, key, name, parent) {
                        self.scene.sources.push(source);
                    }
                }
                _ => {}
            }
        }
    }

    fn composition(&mut self, item: Chunk<'_>, key: u64, name: String, folder: Option<u64>) -> Option<SceneComp> {
        let header = item.find(b"cdta")?;
        let ratio = |top: usize, bottom: usize| -> Option<f64> {
            let below = f64::from(u32_at(header, bottom)?);
            (below > 0.0).then(|| f64::from(u32_at(header, top).unwrap_or(0)) / below)
        };
        // Frames per second, and the units per second keyframe times are counted in.
        let fps = ratio(8, 4).filter(|fps| (1.0..=999.0).contains(fps)).unwrap_or(30.0);
        let time_scale = f64::from(u32_at(header, 8).filter(|&scale| scale > 0).unwrap_or(30_720));
        let seconds = ratio(44, 48).filter(|seconds| *seconds > 0.0).unwrap_or(10.0);
        let (width, height) = (u32::from(u16_at(header, 140)?), u32::from(u16_at(header, 142)?));
        let mut layers = Vec::new();
        for chunk in item.children().filter(|chunk| chunk.is_list(b"Layr")) {
            if let Some(layer) = self.layer(chunk, time_scale) {
                layers.push(layer);
            }
        }
        Some(SceneComp { key, name, folder, width, height, fps, seconds, background: Rgba::hex(0x000000), linear_blending: true, layers })
    }

    fn layer(&mut self, layer: Chunk<'_>, time_scale: f64) -> Option<SceneLayer> {
        let header = layer.find(b"ldta")?;
        let time = |top: usize, bottom: usize| -> f64 {
            let below = f64::from(u32_at(header, bottom).unwrap_or(0));
            if below > 0.0 { f64::from(i32_at(header, top).unwrap_or(0)) / below } else { 0.0 }
        };
        let start = time(12, 16);
        let (in_point, out_point) = (start + time(20, 24), start + time(28, 32));
        let source = u32_at(header, 40).unwrap_or(0);
        let (flags, switches, kind) = (*header.get(38)?, *header.get(39)?, *header.get(131)?);
        let parent = u32_at(header, 132).filter(|&parent| parent != 0).map(u64::from);
        let mut name = layer.find(b"Utf8").map(|bytes| String::from_utf8_lossy(bytes).into_owned()).unwrap_or_default();
        if name.is_empty() {
            name = self.names.get(&source).cloned().unwrap_or_default();
        }
        let properties = layer.list(b"tdgp");

        let content = match kind {
            LAYER_CAMERA => {
                self.skipped.cameras += 1;
                return None;
            }
            LAYER_LIGHT => {
                self.skipped.lights += 1;
                return None;
            }
            LAYER_MODEL => {
                self.skipped.models += 1;
                return None;
            }
            LAYER_TEXT => SceneContent::Text(properties.and_then(text).unwrap_or(SceneText::plain(name.clone(), None, 72.0, Some(Rgba::hex(0xffffff)), TextAlign::Left))),
            LAYER_SHAPE => {
                let mut shapes = Vec::new();
                if let Some(vectors) = properties.and_then(|group| member(group, "ADBE Root Vectors Group")) {
                    contents(vectors, Affine::IDENTITY, None, None, &mut shapes, 0);
                }
                SceneContent::Shapes(shapes)
            }
            _ if flags & 0x80 != 0 => SceneContent::Null,
            _ if flags & 0x02 != 0 => SceneContent::Adjustment,
            _ => match self.kinds.get(&source) {
                Some(&COMPOSITION) => SceneContent::Comp(u64::from(source)),
                Some(&FOOTAGE) => SceneContent::Source(u64::from(source)),
                _ => return None,
            },
        };

        let mut transform = SceneTransform::default();
        if let Some(group) = properties {
            if member(group, "ADBE Effect Parade").is_some_and(|effects| effects.children().any(|chunk| chunk.is_list(b"sspc") || chunk.is_list(b"tdgp"))) {
                self.skipped.effects.push(name.clone());
            }
            if member(group, "ADBE Mask Parade").is_some_and(|masks| masks.children().any(|chunk| &chunk.id == b"LIST")) {
                self.skipped.masks.push(name.clone());
            }
            if let Some(xform) = member(group, "ADBE Transform Group") {
                transform = layer_transform(xform, start, time_scale);
            }
        }
        Some(SceneLayer { key: u64::from(u32_at(header, 0)?), name, parent, start, in_point, out_point, visible: switches & 0x01 != 0, blend: crate::model::BlendMode::Normal, content, transform, effects: Vec::new() })
    }
}

/// A footage item: a solid, or a file.
fn footage(item: Chunk<'_>, key: u64, name: String, folder: Option<u64>) -> Option<SceneSource> {
    let pin = item.list(b"Pin ")?;
    let spec = pin.find(b"sspc")?;
    let size = Vec2::new(u32_at(spec, 30).unwrap_or(1920) as f32, u32_at(spec, 34).unwrap_or(1080) as f32);
    if spec.get(22..26) == Some(b"Soli") {
        let options = pin.find(b"opti").unwrap_or(&[]);
        let channel = |at: usize| f32_at(options, at).unwrap_or(1.0).clamp(0.0, 1.0);
        let color = Rgba { r: channel(14), g: channel(18), b: channel(22), a: channel(10), link: None };
        return Some(SceneSource { key, name, folder, kind: SourceKind::Solid { size, color } });
    }
    let path = footage_file(pin)?;
    Some(SceneSource { key, name, folder, kind: SourceKind::File { path, size } })
}

/// Where a footage item's file is: a small JSON block in its `Pin ` list holds the full path.
fn footage_file(pin: Chunk<'_>) -> Option<PathBuf> {
    let alias = pin.list(b"Als2")?.find(b"alas")?;
    let json: serde_json::Value = serde_json::from_slice(alias).ok()?;
    Some(PathBuf::from(json.get("fullpath")?.as_str()?))
}

/// The element named `name` among a group's members (each named by the `tdmn` before it).
fn member<'a>(group: Chunk<'a>, name: &str) -> Option<Chunk<'a>> {
    members(group).find(|(member, _)| member == name).map(|(_, chunk)| chunk)
}

/// A group's members: each `tdmn` name with the list after it.
fn members(group: Chunk<'_>) -> impl Iterator<Item = (String, Chunk<'_>)> {
    let mut name: Option<String> = None;
    group.children().filter_map(move |chunk| {
        if &chunk.id == b"tdmn" {
            let end = chunk.body.iter().position(|&byte| byte == 0).unwrap_or(chunk.body.len());
            name = Some(String::from_utf8_lossy(&chunk.body[..end]).into_owned());
            None
        } else if &chunk.id == b"LIST" {
            name.take().map(|name| (name, chunk))
        } else {
            None
        }
    })
}

/// One keyframe of a property, as stored: time (in the layer's own time units), interpolation
/// on each side, value, and each side's speed and influence (for the dimension that matters).
struct Key {
    time: f64,
    into: u8,
    out: u8,
    value: Vec<f64>,
    speed: (f64, f64),
    influence: (f64, f64),
}

/// A property's value (one number per dimension) and keyframes. `spatial` properties (anchor
/// point, position) store keyframes in their own layout.
fn property(property: Chunk<'_>, spatial: bool) -> Option<(Vec<f64>, Vec<Key>)> {
    if !property.is_list(b"tdbs") {
        return None;
    }
    let dimensions = property.find(b"tdb4").and_then(|header| u16_at(header, 2)).map_or(1, usize::from).clamp(1, 4);
    let value: Vec<f64> = property.find(b"cdat").map(|data| (0..dimensions).filter_map(|i| f64_at(data, i * 8)).collect()).unwrap_or_default();
    let mut keys = Vec::new();
    if let Some(list) = property.list(b"list")
        && let (Some(header), Some(data)) = (list.find(b"lhd3"), list.find(b"ldat"))
    {
        let count = (u32_at(header, 8).unwrap_or(0) as usize).min(MAX_KEYS);
        let size = u32_at(header, 16).unwrap_or(0) as usize;
        for index in 0..count {
            let Some(key) = data.get(index * size..(index + 1) * size).filter(|_| size >= 8) else { break };
            let at = |offset: usize| f64_at(key, offset).unwrap_or(0.0);
            let (time, into, out) = (f64::from(i32_at(key, 0).unwrap_or(0)), key[4], key[5]);
            let parsed = if spatial {
                let value = (0..dimensions).map(|i| at(56 + 8 * i)).collect();
                Key { time, into, out, value, speed: (at(16), at(24)), influence: (at(32), at(48)) }
            } else {
                let n = dimensions;
                let value: Vec<f64> = (0..n).map(|i| at(8 + 8 * i)).collect();
                // Speeds and influences of the first dimension (they are usually the same for all).
                Key { time, into, out, value, speed: (at(8 + 8 * n), at(8 + 24 * n)), influence: (at(8 + 16 * n), at(8 + 32 * n)) }
            };
            keys.push(parsed);
        }
    }
    let value = if value.is_empty() { keys.first().map(|key| key.value.clone())? } else { value };
    Some((value, keys))
}

/// A property as a track in composition seconds, with `make` turning stored numbers into a value.
fn track<T: Copy>(element: Chunk<'_>, spatial: bool, start: f64, time_scale: f64, make: impl Fn(&[f64]) -> Option<T>) -> Option<Track<T>> {
    let (value, keys) = property(element, spatial)?;
    let value = make(&value)?;
    let mut out = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let Some(made) = make(&key.value) else { continue };
        let easing = match keys.get(index + 1) {
            _ if key.out == HOLD => Easing::Hold,
            None => Easing::Linear,
            Some(next) => {
                let seconds = (next.time - key.time) / time_scale;
                let change = if spatial {
                    key.value.iter().zip(&next.value).map(|(a, b)| (b - a).powi(2)).sum::<f64>().sqrt()
                } else {
                    next.value.first().unwrap_or(&0.0) - key.value.first().unwrap_or(&0.0)
                };
                let out = (key.out == BEZIER).then_some((key.speed.1, key.influence.1));
                let into = (next.into == BEZIER).then_some((next.speed.0, next.influence.0));
                bezier_from_speeds(out, into, seconds, change)
            }
        };
        out.push((start + key.time / time_scale, made, easing));
    }
    Some(Track { value, keys: out })
}

fn pair(values: &[f64]) -> Option<Vec2> {
    Some(Vec2::new(*values.first()? as f32, *values.get(1)? as f32))
}

fn layer_transform(group: Chunk<'_>, start: f64, scale: f64) -> SceneTransform {
    let get = |name: &str, spatial: bool| member(group, name).map(|element| (element, spatial));
    let vec = |name: &str, spatial: bool, factor: f32| get(name, spatial).and_then(|(element, spatial)| track(element, spatial, start, scale, |values| pair(values).map(|v| v * factor)));
    let number = |name: &str, factor: f32| get(name, false).and_then(|(element, _)| track(element, false, start, scale, |values| values.first().map(|&v| v as f32 * factor)));
    let mut position = vec("ADBE Position", true, 1.0);
    // Separated dimensions: X and Y keyed on their own.
    if position.is_none() {
        let (x, y) = (number("ADBE Position_0", 1.0), number("ADBE Position_1", 1.0));
        if let (Some(x), Some(y)) = (x, y)
            && (!x.keys.is_empty() || !y.keys.is_empty())
        {
            position = Some(super::combine_axes(&x, &y));
        }
    }
    SceneTransform {
        anchor: vec("ADBE Anchor Point", true, 1.0),
        position,
        scale: vec("ADBE Scale", false, 100.0),
        rotation: number("ADBE Rotate Z", 1.0),
        opacity: number("ADBE Opacity", 100.0),
    }
}

/// A property's value now (its first keyframe, if it is animated).
fn fixed(group: Chunk<'_>, name: &str) -> Option<Vec<f64>> {
    member(group, name).and_then(|element| property(element, false)).map(|(value, _)| value)
}

/// A color property stored as alpha, red, green, blue from 0 to 255.
fn vector_color(group: Chunk<'_>, name: &str) -> Option<Rgba> {
    let values = fixed(group, name)?;
    let channel = |i: usize| (values.get(i).copied().unwrap_or(255.0) / 255.0).clamp(0.0, 1.0) as f32;
    Some(Rgba { r: channel(1), g: channel(2), b: channel(3), a: channel(0), link: None })
}

/// A shape group's contents: shapes, with the fill and stroke in this group (or the nearest
/// group around it) applied, and the group transforms folded in.
fn contents(group: Chunk<'_>, to_layer: Affine, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>, out: &mut Vec<SceneShape>, depth: usize) {
    if depth > MAX_DEPTH || out.len() > MAX_KEYS {
        return;
    }
    let mut fill = fill;
    let mut stroke = stroke;
    for (name, element) in members(group) {
        match name.as_str() {
            "ADBE Vector Graphic - Fill" => fill = vector_color(element, "ADBE Vector Fill Color").or(Some(Rgba::hex(0xff0000))),
            "ADBE Vector Graphic - Stroke" => {
                let width = fixed(element, "ADBE Vector Stroke Width").and_then(|values| values.first().copied()).unwrap_or(2.0) as f32;
                stroke = vector_color(element, "ADBE Vector Stroke Color").map(|color| (color, width));
            }
            _ => {}
        }
    }
    for (name, element) in members(group) {
        let geometry = match name.as_str() {
            "ADBE Vector Group" => {
                let transform = member(element, "ADBE Vector Transform Group").map_or(Affine::IDENTITY, group_transform);
                if let Some(inner) = member(element, "ADBE Vectors Group") {
                    contents(inner, transform.then_outer(to_layer), fill, stroke, out, depth + 1);
                }
                continue;
            }
            "ADBE Vector Shape - Rect" => {
                let size = fixed(element, "ADBE Vector Rect Size").and_then(|v| pair(&v)).unwrap_or(Vec2::new(100.0, 100.0));
                let center = fixed(element, "ADBE Vector Rect Position").and_then(|v| pair(&v)).unwrap_or_default();
                let radius = fixed(element, "ADBE Vector Rect Roundness").and_then(|v| v.first().copied()).unwrap_or(0.0) as f32;
                Geometry::Rect { center, size, radius, rotation: 0.0 }
            }
            "ADBE Vector Shape - Ellipse" => {
                let size = fixed(element, "ADBE Vector Ellipse Size").and_then(|v| pair(&v)).unwrap_or(Vec2::new(100.0, 100.0));
                let center = fixed(element, "ADBE Vector Ellipse Position").and_then(|v| pair(&v)).unwrap_or_default();
                Geometry::Ellipse { center, size, rotation: 0.0 }
            }
            "ADBE Vector Shape - Group" => match member(element, "ADBE Vector Shape").and_then(path) {
                Some(path) => Geometry::Path(path),
                None => continue,
            },
            _ => continue,
        };
        if fill.is_some() || stroke.is_some() {
            out.push(SceneShape::solid(to_layer.geometry(geometry), fill, stroke));
        }
    }
}

/// A shape group's own transform.
fn group_transform(group: Chunk<'_>) -> Affine {
    let vec = |name: &str, default: Vec2| fixed(group, name).and_then(|v| pair(&v)).unwrap_or(default);
    let rotation = fixed(group, "ADBE Vector Rotation").and_then(|v| v.first().copied()).unwrap_or(0.0) as f32;
    let scale = fixed(group, "ADBE Vector Scale").and_then(|v| pair(&v)).map_or(Vec2::new(100.0, 100.0), |v| v * 100.0);
    Affine::group(vec("ADBE Vector Anchor", Vec2::default()), vec("ADBE Vector Position", Vec2::default()), scale, rotation)
}

/// A bezier path (its first keyframe, if it is animated): a bounding box in `shph`, then
/// points as fractions of it, in threes (point, its outgoing handle, the next point's incoming
/// handle), handles as positions rather than offsets.
fn path(shape: Chunk<'_>) -> Option<ShapePath> {
    let keyed = shape.list(b"omks")?;
    let data = keyed.list(b"shap")?;
    let header = data.find(b"shph")?;
    let (left, top, right, bottom) = (f32_at(header, 4)?, f32_at(header, 8)?, f32_at(header, 12)?, f32_at(header, 16)?);
    // Not confirmed on an open path yet; every path in the sample files reads as closed.
    let closed = header.get(3).is_none_or(|flags| flags & 0x08 == 0);
    let list = data.list(b"list")?;
    let count = (u32_at(list.find(b"lhd3")?, 8)? as usize).min(crate::path::MAX_PATH_POINTS * 3);
    let points = list.find(b"ldat")?;
    let at = |index: usize| -> Option<Vec2> {
        let (x, y) = (f32_at(points, index * 8)?, f32_at(points, index * 8 + 4)?);
        Some(Vec2::new(left + x * (right - left), top + y * (bottom - top)))
    };
    let corners = count / 3;
    if corners < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(corners);
    for corner in 0..corners {
        let point = at(corner * 3)?;
        let handle_out = at(corner * 3 + 1)? - point;
        let previous_in = if corner == 0 { at(corners * 3 - 1) } else { at(corner * 3 - 1) };
        let handle_in = previous_in.map_or(Vec2::default(), |handle| handle - point);
        out.push(PathPoint { at: point, handle_in: if corner == 0 && !closed { Vec2::default() } else { handle_in }, handle_out });
    }
    Some(ShapePath { points: out, closed, ..Default::default() })
}

/// A text layer's words and look, from its text document (After Effects' text engine data: a
/// dictionary with numbered keys and strings in parentheses).
fn text(properties: Chunk<'_>) -> Option<SceneText> {
    let document = member(member(properties, "ADBE Text Properties")?, "ADBE Text Document")?;
    let data = document.children().find(|chunk| chunk.is_list(b"btds"))?.children().find(|chunk| chunk.is_list(b"btdk"))?.body;
    let fonts: Vec<String> = find_all(data, b"/99 /CoolTypeFont /0 << /0 (").filter_map(|at| literal(data, at - 1).map(|(text, _)| text)).collect();
    let start = find(data, b"/1 [ << /0 << /0 (", 0)?;
    let (mut words, after) = literal(data, start + b"/1 [ << /0 << /0 (".len() - 1)?;
    words = words.trim_end_matches(['\r', '\n']).replace('\r', "\n");
    let style_at = find(data, b"/6 << /0 ", after);
    let (font, size) = style_at.map_or((None, 72.0), |at| {
        let (font_index, next) = number(data, at + b"/6 << /0 ".len());
        let size = find(data, b"/1 ", next).filter(|&p| p == next + 1 || p == next).map_or(72.0, |p| number(data, p + 3).0);
        (fonts.get(font_index as usize).cloned(), size as f32)
    });
    let fill = style_at.and_then(|at| find(data, b"/53 << /99 /SimplePaint /0 << /0 1 /1 [ ", at)).map(|at| {
        let mut position = at + b"/53 << /99 /SimplePaint /0 << /0 1 /1 [ ".len();
        let mut channels = [1.0f32; 4];
        for channel in &mut channels {
            let (value, next) = number(data, position);
            *channel = (value as f32).clamp(0.0, 1.0);
            position = next;
        }
        Rgba { r: channels[1], g: channels[2], b: channels[3], a: channels[0], link: None }
    });
    // Paragraph alignment: 0 left, 1 right, 2 centered.
    let align = find(data, b" /5 << /0 ", after).filter(|&at| style_at.is_none_or(|style| at < style)).map_or(TextAlign::Left, |at| match number(data, at + b" /5 << /0 ".len()).0 as i64 {
        1 => TextAlign::Right,
        2 => TextAlign::Center,
        _ => TextAlign::Left,
    });
    Some(SceneText::plain(words, font, size, fill, align))
}

fn find(data: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?.windows(needle.len()).position(|window| window == needle).map(|at| at + from)
}

/// Where each `needle` ends, all through `data`.
fn find_all<'a>(data: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    let mut from = 0;
    std::iter::from_fn(move || {
        let at = find(data, needle, from)?;
        from = at + needle.len();
        Some(from)
    })
}

/// A number written at `at` (skipping spaces), and where it ends.
fn number(data: &[u8], at: usize) -> (f64, usize) {
    let mut start = at;
    while data.get(start) == Some(&b' ') {
        start += 1;
    }
    let mut end = start;
    while data.get(end).is_some_and(|c| c.is_ascii_digit() || matches!(c, b'.' | b'-' | b'+' | b'e' | b'E')) {
        end += 1;
    }
    let value = std::str::from_utf8(&data[start..end]).ok().and_then(|text| text.parse().ok()).filter(|v: &f64| v.is_finite()).unwrap_or(0.0);
    (value, end)
}

/// A string in parentheses starting at `open` (a backslash escapes the next byte), as text:
/// UTF-16 if it starts with a byte-order mark, else Latin-1. Returns it and where it ends.
fn literal(data: &[u8], open: usize) -> Option<(String, usize)> {
    if data.get(open) != Some(&b'(') {
        return None;
    }
    let mut bytes = Vec::new();
    let mut index = open + 1;
    while let Some(&byte) = data.get(index) {
        match byte {
            b'\\' => {
                bytes.extend(data.get(index + 1));
                index += 2;
            }
            b')' => break,
            _ => {
                bytes.push(byte);
                index += 1;
            }
        }
        if bytes.len() > 1 << 20 {
            return None;
        }
    }
    let text = if bytes.starts_with(&[0xfe, 0xff]) {
        let units: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|&pair| u16::from_be_bytes(pair)).collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| char::from(b)).collect()
    };
    Some((text, index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_numbers_and_escapes_read_from_text_data() {
        let data = b"/0 (\xfe\xff\x00H\x00i\x00\\)) /1 12.5";
        assert_eq!(literal(data, 3).unwrap().0, "Hi)");
        let after = literal(data, 3).unwrap().1;
        assert_eq!(number(data, after + " /1".len()).0, 12.5);
    }

    #[test]
    fn other_files_are_refused() {
        assert!(read(b"RIFF....WAVE").is_err());
        assert!(read(b"").is_err());
    }

    /// Reads the project that ships inside After Effects, if it is installed:
    /// `AEP_FILE=…/secret.aep cargo test -p studio-core secret -- --ignored --nocapture`
    #[test]
    #[ignore = "needs an After Effects project; set AEP_FILE"]
    fn reads_a_real_project() {
        let bytes = std::fs::read(std::env::var("AEP_FILE").unwrap()).unwrap();
        let scene = read(&bytes).unwrap();
        println!("{} folders, {} sources, {} comps", scene.folders.len(), scene.sources.len(), scene.comps.len());
        for comp in scene.comps.iter().take(6) {
            println!("{} {}x{} {:.3} fps {:.2} s, {} layers", comp.name, comp.width, comp.height, comp.fps, comp.seconds, comp.layers.len());
        }
        for note in &scene.notes {
            println!("note: {note}");
        }
        assert!(!scene.comps.is_empty());
        for source in &scene.sources {
            println!("source: {:?}", source.name);
        }
        assert!(scene.sources.iter().all(|source| !source.name.is_empty()), "every source has a name");
        let mut project = crate::sample::sample_project();
        let imported = super::super::build(&mut project, &scene, "secret", &|_| None).unwrap();
        println!("made {} compositions; main is {:?}", imported.compositions.len(), project.composition(imported.compositions[0]).map(|c| &c.name));
    }
}
