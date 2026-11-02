//! Reading Lottie animations (`.json`), the format After Effects' Bodymovin plugin exports and
//! most animation libraries share (see lottiefiles.github.io/lottie-docs). It is documented, so
//! it reads more completely than `.aep`:
//!
//! - the main animation and its precompositions (`assets` with `layers`), with their size,
//!   frame rate and length,
//! - images (embedded ones are written out to `asset_folder` first),
//! - layers: precompositions, solids, images, nulls, text and shapes (rectangles, ellipses and
//!   paths, inside groups with their transforms, filled and stroked),
//! - timing, parents and transforms with keyframes; Lottie's easing is already a cubic bezier.
//!
//! Gradients, trim paths, repeaters, masks, mattes, effects and expressions aren't read; the
//! report says so.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Affine, Geometry, Scene, SceneComp, SceneContent, SceneLayer, SceneShape, SceneSource, SceneText, SceneTransform, SourceKind, Track};
use crate::animation::{CubicBezier, Easing, Vec2};
use crate::model::{Rgba, TextAlign};
use crate::path::{PathPoint, ShapePath};

/// How deep groups may nest, and how many points a path may have, against hostile files.
const MAX_DEPTH: usize = 32;

/// Reads a Lottie file into a scene. `file` is where it is (images may be beside it);
/// embedded images are written into `asset_folder`.
pub fn read(text: &str, file: &Path, asset_folder: &Path) -> Result<Scene, String> {
    let root: Value = serde_json::from_str(text).map_err(|_| "This isn't a Lottie animation: it isn't valid JSON.".to_owned())?;
    let (Some(fps), Some(width), Some(height)) = (root.get("fr").and_then(Value::as_f64), root.get("w").and_then(Value::as_f64), root.get("h").and_then(Value::as_f64)) else {
        return Err("This isn't a Lottie animation: it has no frame rate or size.".into());
    };
    if !(fps > 0.0 && fps <= 999.0) || !(1.0..=30_000.0).contains(&width) || !(1.0..=30_000.0).contains(&height) {
        return Err("This Lottie animation has an impossible size or frame rate.".into());
    }
    let mut reader = Reader { scene: Scene::default(), fps, assets: HashMap::new(), keys: 1, file: file.to_owned(), asset_folder: asset_folder.to_owned(), unsupported: Vec::new() };
    let length = |value: &Value| (value.get("op").and_then(Value::as_f64).unwrap_or(fps * 10.0) - value.get("ip").and_then(Value::as_f64).unwrap_or(0.0)).max(1.0) / fps;

    // Assets: images, and precompositions (which need their keys before any layer uses them).
    let assets = root.get("assets").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut precomps = Vec::new();
    for asset in &assets {
        let Some(id) = asset.get("id").and_then(|id| id.as_str().map(str::to_owned).or_else(|| id.as_i64().map(|n| n.to_string()))) else { continue };
        let key = reader.next_key();
        if asset.get("layers").is_some() {
            reader.assets.insert(id.clone(), Asset::Comp(key));
            precomps.push((key, id, asset));
        } else if let Some(source) = reader.image(asset, key) {
            reader.assets.insert(id, Asset::Image(key));
            reader.scene.sources.push(source);
        }
    }
    for (key, id, asset) in precomps {
        let layers = reader.layers(asset.get("layers"));
        let size = |name: &str, fallback: f64| asset.get(name).and_then(Value::as_f64).unwrap_or(fallback).clamp(1.0, 30_000.0) as u32;
        reader.scene.comps.push(SceneComp {
            key,
            name: asset.get("nm").and_then(Value::as_str).map_or_else(|| format!("Precomp {id}"), str::to_owned),
            folder: None,
            width: size("w", width),
            height: size("h", height),
            fps,
            seconds: length(&root),
            background: Rgba::TRANSPARENT,
            layers,
        });
    }
    let layers = reader.layers(root.get("layers"));
    let main = reader.next_key();
    let name = root.get("nm").and_then(Value::as_str).filter(|name| !name.trim().is_empty()).map_or_else(|| file.file_stem().map_or("Lottie".into(), |stem| stem.to_string_lossy().into_owned()), str::to_owned);
    // The main animation first, so it opens.
    reader.scene.comps.insert(0, SceneComp { key: main, name, folder: None, width: width as u32, height: height as u32, fps, seconds: length(&root), background: Rgba::TRANSPARENT, layers });
    reader.unsupported.sort();
    reader.unsupported.dedup();
    if !reader.unsupported.is_empty() {
        reader.scene.notes.push(format!("Not brought in: {}.", reader.unsupported.join(", ")));
    }
    Ok(reader.scene)
}

enum Asset {
    Comp(u64),
    Image(u64),
}

struct Reader {
    scene: Scene,
    fps: f64,
    assets: HashMap<String, Asset>,
    keys: u64,
    file: PathBuf,
    asset_folder: PathBuf,
    unsupported: Vec<&'static str>,
}

impl Reader {
    fn next_key(&mut self) -> u64 {
        self.keys += 1;
        self.keys
    }

    /// An image asset: a file beside the animation, or embedded data written out first.
    fn image(&mut self, asset: &Value, key: u64) -> Option<SceneSource> {
        let name = asset.get("p").and_then(Value::as_str)?;
        let size = Vec2::new(asset.get("w").and_then(Value::as_f64).unwrap_or(100.0) as f32, asset.get("h").and_then(Value::as_f64).unwrap_or(100.0) as f32);
        let path = if let Some(data) = name.strip_prefix("data:") {
            let (kind, encoded) = data.split_once(";base64,")?;
            let extension = match kind {
                "image/png" => "png",
                "image/jpeg" | "image/jpg" => "jpg",
                "image/webp" => "webp",
                _ => return None,
            };
            let bytes = decode_base64(encoded)?;
            std::fs::create_dir_all(&self.asset_folder).ok()?;
            let path = self.asset_folder.join(format!("image-{key}.{extension}"));
            std::fs::write(&path, bytes).ok()?;
            path
        } else {
            let folder = asset.get("u").and_then(Value::as_str).unwrap_or("");
            // Only names inside the animation's folder: never a path that climbs out of it.
            let relative = Path::new(folder).join(name);
            if relative.components().any(|part| !matches!(part, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
                return None;
            }
            self.file.parent().unwrap_or(Path::new(".")).join(relative)
        };
        let label = asset.get("nm").and_then(Value::as_str).unwrap_or(name.get(..name.len().min(40)).unwrap_or("Image")).to_owned();
        Some(SceneSource { key, name: label, folder: None, kind: SourceKind::File { path, size } })
    }

    fn layers(&mut self, layers: Option<&Value>) -> Vec<SceneLayer> {
        let Some(layers) = layers.and_then(Value::as_array) else { return Vec::new() };
        layers.iter().filter_map(|layer| self.layer(layer)).collect()
    }

    fn layer(&mut self, layer: &Value) -> Option<SceneLayer> {
        let fps = self.fps;
        let number = |name: &str| layer.get(name).and_then(Value::as_f64);
        let name = layer.get("nm").and_then(Value::as_str).unwrap_or("Layer").to_owned();
        let start = number("st").unwrap_or(0.0) / fps;
        let content = match layer.get("ty").and_then(Value::as_i64)? {
            0 => match self.assets.get(layer.get("refId")?.as_str()?) {
                Some(Asset::Comp(key)) => SceneContent::Comp(*key),
                _ => return None,
            },
            1 => {
                let key = self.next_key();
                let color = layer.get("sc").and_then(Value::as_str).and_then(hex_color).unwrap_or(Rgba::hex(0x000000));
                let size = Vec2::new(number("sw").unwrap_or(100.0) as f32, number("sh").unwrap_or(100.0) as f32);
                self.scene.sources.push(SceneSource { key, name: name.clone(), folder: None, kind: SourceKind::Solid { size, color } });
                SceneContent::Source(key)
            }
            2 => match self.assets.get(layer.get("refId")?.as_str()?) {
                Some(Asset::Image(key)) => SceneContent::Source(*key),
                _ => return None,
            },
            3 => SceneContent::Null,
            4 => {
                let mut shapes = Vec::new();
                if let Some(items) = layer.get("shapes").and_then(Value::as_array) {
                    self.contents(items, Affine::IDENTITY, None, None, &mut shapes, 0);
                }
                SceneContent::Shapes(shapes)
            }
            5 => SceneContent::Text(text(layer)?),
            _ => {
                self.unsupported.push("audio, camera and other special layers");
                return None;
            }
        };
        if layer.get("masksProperties").and_then(Value::as_array).is_some_and(|masks| !masks.is_empty()) {
            self.unsupported.push("masks");
        }
        if layer.get("tt").is_some() {
            self.unsupported.push("track mattes");
        }
        if layer.get("ef").and_then(Value::as_array).is_some_and(|effects| !effects.is_empty()) {
            self.unsupported.push("effects");
        }
        let transform = layer.get("ks").map(|ks| transform(ks, start, fps)).unwrap_or_default();
        Some(SceneLayer {
            key: layer.get("ind").and_then(Value::as_u64).unwrap_or_else(|| self.next_key() + 1_000_000),
            name,
            parent: layer.get("parent").and_then(Value::as_u64),
            start,
            in_point: number("ip").unwrap_or(0.0) / fps,
            out_point: number("op").unwrap_or(fps * 10.0) / fps,
            visible: layer.get("hd").and_then(Value::as_bool) != Some(true),
            content,
            transform,
        })
    }

    /// Shape items, as `aep::contents` does for After Effects: fills and strokes apply to the
    /// shapes in their group, and group transforms fold in.
    fn contents(&mut self, items: &[Value], to_layer: Affine, fill: Option<Rgba>, stroke: Option<(Rgba, f32)>, out: &mut Vec<SceneShape>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let (mut fill, mut stroke) = (fill, stroke);
        let mut transform = Affine::IDENTITY;
        for item in items {
            match item.get("ty").and_then(Value::as_str) {
                Some("fl") => fill = paint(item),
                Some("st") => stroke = paint(item).map(|color| (color, fixed(item.get("w")).and_then(|w| w.first().copied()).unwrap_or(1.0) as f32)),
                Some("tr") => transform = group_transform(item),
                Some("gf" | "gs") => self.unsupported.push("gradients"),
                Some("tm") => self.unsupported.push("trim paths"),
                Some("rp") => self.unsupported.push("repeaters"),
                _ => {}
            }
        }
        let to_layer = transform.then_outer(to_layer);
        for item in items {
            let geometry = match item.get("ty").and_then(Value::as_str) {
                Some("gr") => {
                    if let Some(inner) = item.get("it").and_then(Value::as_array) {
                        self.contents(inner, to_layer, fill, stroke, out, depth + 1);
                    }
                    continue;
                }
                Some("rc") => Geometry::Rect {
                    center: fixed_pair(item.get("p")).unwrap_or_default(),
                    size: fixed_pair(item.get("s")).unwrap_or(Vec2::new(100.0, 100.0)),
                    radius: fixed(item.get("r")).and_then(|r| r.first().copied()).unwrap_or(0.0) as f32,
                    rotation: 0.0,
                },
                Some("el") => Geometry::Ellipse { center: fixed_pair(item.get("p")).unwrap_or_default(), size: fixed_pair(item.get("s")).unwrap_or(Vec2::new(100.0, 100.0)), rotation: 0.0 },
                Some("sh") => match item.get("ks").and_then(first_value).and_then(bezier_path) {
                    Some(path) => Geometry::Path(path),
                    None => continue,
                },
                _ => continue,
            };
            if fill.is_some() || stroke.is_some() {
                out.push(SceneShape { geometry: to_layer.geometry(geometry), fill, stroke });
            }
        }
    }
}

/// The first value of a property: `{"a":0,"k":value}`, or the first keyframe's `s`.
fn first_value(property: &Value) -> Option<&Value> {
    let k = property.get("k")?;
    let first_key = k.as_array().and_then(|keys| keys.first()).filter(|key| key.get("t").is_some());
    let Some(key) = first_key else { return Some(k) };
    let value = key.get("s")?;
    // Path keyframes wrap their path in a one-item list.
    match value.as_array().map(Vec::as_slice) {
        Some([only]) if only.is_object() => Some(only),
        _ => Some(value),
    }
}

/// A property's first value as numbers.
fn fixed(property: Option<&Value>) -> Option<Vec<f64>> {
    numbers(first_value(property?)?)
}

fn fixed_pair(property: Option<&Value>) -> Option<Vec2> {
    let values = fixed(property)?;
    Some(Vec2::new(*values.first()? as f32, *values.get(1).unwrap_or(values.first()?) as f32))
}

fn numbers(value: &Value) -> Option<Vec<f64>> {
    match value {
        Value::Number(number) => number.as_f64().filter(|n| n.is_finite()).map(|n| vec![n]),
        Value::Array(values) => {
            let out: Vec<f64> = values.iter().filter_map(Value::as_f64).filter(|n| n.is_finite()).collect();
            (!out.is_empty()).then_some(out)
        }
        _ => None,
    }
}

/// A fill or stroke color (0–1 channels), with its opacity (0–100).
fn paint(item: &Value) -> Option<Rgba> {
    let color = fixed(item.get("c"))?;
    let opacity = fixed(item.get("o")).and_then(|o| o.first().copied()).unwrap_or(100.0) / 100.0;
    let channel = |i: usize| color.get(i).copied().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    Some(Rgba { r: channel(0), g: channel(1), b: channel(2), a: (color.get(3).copied().unwrap_or(1.0) * opacity).clamp(0.0, 1.0) as f32, link: None })
}

/// "#rrggbb" as a color.
fn hex_color(text: &str) -> Option<Rgba> {
    let digits = text.trim_start_matches('#');
    (digits.len() == 6).then(|| u32::from_str_radix(digits, 16).ok().map(Rgba::hex)).flatten()
}

fn group_transform(item: &Value) -> Affine {
    let anchor = fixed_pair(item.get("a")).unwrap_or_default();
    let position = fixed_pair(item.get("p")).unwrap_or_default();
    let scale = fixed_pair(item.get("s")).unwrap_or(Vec2::new(100.0, 100.0));
    let rotation = fixed(item.get("r")).and_then(|r| r.first().copied()).unwrap_or(0.0) as f32;
    Affine::group(anchor, position, scale, rotation)
}

/// A Lottie bezier: vertices `v`, and handles `i` and `o` as offsets from them.
fn bezier_path(value: &Value) -> Option<ShapePath> {
    let list = |name: &str| -> Vec<Vec2> {
        value.get(name).and_then(Value::as_array).map(|points| points.iter().filter_map(|point| numbers(point).filter(|p| p.len() >= 2).map(|p| Vec2::new(p[0] as f32, p[1] as f32))).collect()).unwrap_or_default()
    };
    let (vertices, ins, outs) = (list("v"), list("i"), list("o"));
    if vertices.len() < 2 || vertices.len() > crate::path::MAX_PATH_POINTS {
        return None;
    }
    let points = vertices.iter().enumerate().map(|(i, &at)| PathPoint { at, handle_in: ins.get(i).copied().unwrap_or_default(), handle_out: outs.get(i).copied().unwrap_or_default() }).collect();
    Some(ShapePath { points, closed: value.get("c").and_then(Value::as_bool).unwrap_or(false) })
}

/// A layer's transform (`ks`).
fn transform(ks: &Value, start: f64, fps: f64) -> SceneTransform {
    let vec = |name: &str| ks.get(name).and_then(|property| track(property, start, fps, |values| Some(Vec2::new(*values.first()? as f32, *values.get(1).unwrap_or(&values[0]) as f32))));
    let number = |name: &str| ks.get(name).and_then(|property| track(property, start, fps, |values| values.first().map(|&v| v as f32)));
    // Position split into x and y ("s": true): combined at the x keys.
    let position = match ks.get("p") {
        Some(p) if p.get("s").and_then(Value::as_bool) == Some(true) => {
            let x = p.get("x").and_then(|x| track(x, start, fps, |v| v.first().map(|&v| v as f32)));
            let y = p.get("y").and_then(|y| track(y, start, fps, |v| v.first().map(|&v| v as f32)));
            match (x, y) {
                (Some(x), Some(y)) => {
                    let y_at = |seconds: f64| y.keys.iter().rev().find(|(at, _, _)| *at <= seconds).map_or(y.value, |(_, value, _)| *value);
                    Some(Track { value: Vec2::new(x.value, y.value), keys: x.keys.iter().map(|&(at, value, easing)| (at, Vec2::new(value, y_at(at)), easing)).collect() })
                }
                _ => None,
            }
        }
        _ => vec("p"),
    };
    SceneTransform { anchor: vec("a"), position, scale: vec("s"), rotation: number("r").or_else(|| number("rz")), opacity: number("o") }
}

/// A property as a track: `{"a":0,"k":value}` or `{"a":1,"k":[keyframes]}`, each keyframe with
/// its time `t` (frames), value `s`, and handles `o` (out of it) and `i` (into the next), or
/// `h: 1` to hold.
fn track<T: Copy>(property: &Value, start: f64, fps: f64, make: impl Fn(&[f64]) -> Option<T>) -> Option<Track<T>> {
    let k = property.get("k")?;
    let keys = k.as_array().filter(|keys| keys.first().is_some_and(|key| key.get("t").is_some()));
    let Some(keys) = keys else { return Some(Track::fixed(make(&numbers(k)?)?)) };
    let mut out = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let Some(time) = key.get("t").and_then(Value::as_f64) else { continue };
        // Older files give the end value as `e` instead of the next key's `s`.
        let value = key.get("s").and_then(numbers).or_else(|| index.checked_sub(1).and_then(|i| keys[i].get("e")).and_then(numbers));
        let Some(value) = value.and_then(|value| make(&value)) else { continue };
        let handle = |side: &str, axis: &str| key.get(side).and_then(|handle| handle.get(axis)).and_then(numbers).and_then(|values| values.first().copied());
        let easing = if key.get("h").and_then(Value::as_i64) == Some(1) {
            Easing::Hold
        } else {
            match (handle("o", "x"), handle("o", "y"), handle("i", "x"), handle("i", "y")) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => Easing::Curve(CubicBezier::new(x1.clamp(0.0, 1.0) as f32, y1.clamp(-2.0, 3.0) as f32, x2.clamp(0.0, 1.0) as f32, y2.clamp(-2.0, 3.0) as f32)),
                _ => Easing::Linear,
            }
        };
        out.push((start + time / fps, value, easing));
    }
    let value = out.first().map(|(_, value, _)| *value)?;
    Some(Track { value, keys: out })
}

/// A text layer's first text document.
fn text(layer: &Value) -> Option<SceneText> {
    let document = layer.get("t")?.get("d")?.get("k")?.as_array()?.first()?.get("s")?;
    let words = document.get("t").and_then(Value::as_str)?.replace('\r', "\n");
    let fill = document.get("fc").and_then(numbers).map(|c| Rgba { r: c[0].clamp(0.0, 1.0) as f32, g: c.get(1).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32, b: c.get(2).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32, a: 1.0, link: None });
    let align = match document.get("j").and_then(Value::as_i64) {
        Some(1) => TextAlign::Right,
        Some(2) => TextAlign::Center,
        _ => TextAlign::Left,
    };
    Some(SceneText { text: words, font: document.get("f").and_then(Value::as_str).map(str::to_owned), size: document.get("s").and_then(Value::as_f64).unwrap_or(72.0) as f32, fill, align })
}

/// Standard base64 (with or without padding), or `None` if it isn't.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    };
    let clean: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut bits = 0u32;
        for (index, &c) in chunk.iter().enumerate() {
            bits |= value(c)? << (18 - 6 * index);
        }
        let bytes = bits.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small animation: a red rounded rectangle inside a moved group, sliding right with an
    /// ease, a null it is parented to, and a line of text.
    const SAMPLE: &str = r##"{
        "v": "5.7.0", "fr": 30, "ip": 0, "op": 60, "w": 800, "h": 600, "nm": "Demo",
        "assets": [],
        "layers": [
            {"ty": 4, "nm": "Box", "ind": 1, "parent": 2, "ip": 0, "op": 60, "st": 0,
             "ks": {"a": {"a": 0, "k": [0, 0]},
                    "p": {"a": 1, "k": [{"t": 0, "s": [100, 300], "o": {"x": [0.33], "y": [0]}, "i": {"x": [0.67], "y": [1]}}, {"t": 30, "s": [700, 300]}]},
                    "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}, "o": {"a": 0, "k": 100}},
             "shapes": [{"ty": "gr", "it": [
                 {"ty": "rc", "s": {"a": 0, "k": [80, 40]}, "p": {"a": 0, "k": [0, 0]}, "r": {"a": 0, "k": 8}},
                 {"ty": "fl", "c": {"a": 0, "k": [1, 0, 0, 1]}, "o": {"a": 0, "k": 100}},
                 {"ty": "tr", "p": {"a": 0, "k": [10, 5]}, "a": {"a": 0, "k": [0, 0]}, "s": {"a": 0, "k": [100, 100]}, "r": {"a": 0, "k": 0}}
             ]}]},
            {"ty": 3, "nm": "Mover", "ind": 2, "ip": 0, "op": 60, "st": 0, "ks": {}},
            {"ty": 5, "nm": "Title", "ind": 3, "ip": 0, "op": 60, "st": 0, "ks": {},
             "t": {"d": {"k": [{"t": 0, "s": {"t": "Hello", "s": 48, "f": "Inter-Bold", "fc": [0, 0, 1], "j": 2}}]}}}
        ]
    }"##;

    #[test]
    fn a_lottie_file_becomes_a_scene() {
        let scene = read(SAMPLE, Path::new("/tmp/demo.json"), Path::new("/tmp/assets")).unwrap();
        let comp = &scene.comps[0];
        assert_eq!((comp.name.as_str(), comp.width, comp.height, comp.fps, comp.seconds), ("Demo", 800, 600, 30.0, 2.0));
        assert_eq!(comp.layers.len(), 3);
        let SceneContent::Shapes(shapes) = &comp.layers[0].content else { panic!() };
        let Geometry::Rect { center, size, radius, .. } = shapes[0].geometry else { panic!() };
        assert_eq!((center, size, radius), (Vec2::new(10.0, 5.0), Vec2::new(80.0, 40.0), 8.0), "the group moved it");
        assert_eq!(shapes[0].fill, Some(Rgba::hex(0xff0000)));
        let position = comp.layers[0].transform.position.as_ref().unwrap();
        assert_eq!(position.keys.len(), 2);
        assert_eq!(position.keys[0].2, Easing::Curve(CubicBezier::new(0.33, 0.0, 0.67, 1.0)));
        assert_eq!(comp.layers[0].parent, Some(2));
        let SceneContent::Text(text) = &comp.layers[2].content else { panic!() };
        assert_eq!((text.text.as_str(), text.size, text.align), ("Hello", 48.0, TextAlign::Center));
    }

    #[test]
    fn a_lottie_scene_builds_into_the_project() {
        let scene = read(SAMPLE, Path::new("/tmp/demo.json"), Path::new("/tmp/assets")).unwrap();
        let mut project = crate::sample::sample_project();
        let imported = super::super::build(&mut project, &scene, "demo", &|_| None).unwrap();
        let comp = project.composition(imported.compositions[0]).unwrap();
        assert_eq!(comp.name, "Demo");
        // The box layer holds its rectangle as a child; the holder follows the null.
        let box_layer = comp.layers.iter().find(|layer| layer.name == "Box").unwrap();
        let mover = comp.layers.iter().find(|layer| layer.name == "Mover").unwrap();
        assert_eq!(box_layer.parent, Some(mover.id));
        assert!(comp.layers.iter().any(|layer| layer.parent == Some(box_layer.id) && matches!(layer.content, crate::model::LayerContent::Shape { .. })));
        let position = &box_layer.transform.as_ref().unwrap().position;
        assert_eq!(position.keyframe_frames(), vec![0, 30]);
    }

    #[test]
    fn base64_and_unsafe_paths() {
        assert_eq!(decode_base64("aGk=").unwrap(), b"hi");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        let mut reader = Reader { scene: Scene::default(), fps: 30.0, assets: HashMap::new(), keys: 0, file: "/a/b.json".into(), asset_folder: "/tmp".into(), unsupported: Vec::new() };
        let climbing = serde_json::json!({"p": "secret.png", "u": "../../etc/"});
        assert!(reader.image(&climbing, 1).is_none(), "paths can't climb out of the animation's folder");
        let beside = serde_json::json!({"p": "img_0.png", "u": "images/"});
        let SourceKind::File { path, .. } = reader.image(&beside, 2).unwrap().kind else { panic!() };
        assert_eq!(path, PathBuf::from("/a/images/img_0.png"));
    }

    #[test]
    fn other_files_are_refused() {
        assert!(read("not json", Path::new("x.json"), Path::new("/tmp")).is_err());
        assert!(read("{\"hello\": 1}", Path::new("x.json"), Path::new("/tmp")).is_err());
    }
}
