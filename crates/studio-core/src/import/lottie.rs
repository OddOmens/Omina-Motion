//! Reading Lottie animations (`.json`), the format After Effects' Bodymovin plugin exports and
//! most animation libraries share (see lottiefiles.github.io/lottie-docs). It is documented, so
//! it reads more completely than `.aep`:
//!
//! - the main animation and its precompositions (`assets` with `layers`), with their size,
//!   frame rate and length,
//! - images (embedded ones are written out to `asset_folder` first),
//! - layers: precompositions, solids, images, nulls, text (font, size, line height, tracking,
//!   fill and outline) and shapes (rectangles, ellipses and paths, inside groups with their
//!   transforms, filled and stroked with colors or gradients; the paths of one group are one
//!   compound shape, so an outline inside another cuts a hole),
//! - a shape layer of one shape becomes that one shape layer, with the layer's own transform,
//! - the Drop Shadow layer effect,
//! - timing, parents and transforms with keyframes; Lottie's easing is already a cubic bezier.
//!
//! Trim paths, repeaters, masks, mattes, other effects and expressions aren't read; the report
//! says so.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Affine, Geometry, Scene, SceneComp, SceneContent, SceneEffect, SceneLayer, SceneShape, SceneSource, SceneText, SceneTransform, SourceKind, Track};
use crate::animation::{CubicBezier, Easing, Vec2};
use crate::gradient::{Gradient, GradientStop};
use crate::model::{BlendMode, Paint, Rgba, Stroke, TextAlign};
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
    let mut reader = Reader { scene: Scene::default(), fps, assets: HashMap::new(), keys: 1, file: file.to_owned(), asset_folder: asset_folder.to_owned(), unsupported: Vec::new(), time_offset: 0.0, fonts: font_list(&root) };
    let length = |value: &Value| (value.get("op").and_then(Value::as_f64).unwrap_or(fps * 10.0) - value.get("ip").and_then(Value::as_f64).unwrap_or(0.0)).max(1.0) / fps;
    // Precompositions have no size of their own in most files: the layers showing them carry it.
    let sizes = precomp_sizes(&root);

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
        let (used_width, used_height) = sizes.get(&id).copied().unwrap_or((width, height));
        let size = |name: &str, fallback: f64| asset.get(name).and_then(Value::as_f64).unwrap_or(fallback).clamp(1.0, 30_000.0) as u32;
        // Long enough for every layer inside, which may run past the main animation's end when
        // the precomposition starts early (a negative `st` on the layer showing it).
        let last_out = layers.iter().map(|layer| layer.out_point).fold(0.0, f64::max);
        reader.scene.comps.push(SceneComp {
            // Lottie players blend colors as browsers do, in display colors.
            linear_blending: false,
            key,
            name: asset.get("nm").and_then(Value::as_str).map_or_else(|| format!("Precomp {id}"), str::to_owned),
            folder: None,
            width: size("w", used_width),
            height: size("h", used_height),
            fps,
            seconds: length(&root).max(last_out),
            background: Rgba::TRANSPARENT,
            layers,
        });
    }
    // The main animation runs from its `ip`, which becomes the composition's first frame.
    reader.time_offset = root.get("ip").and_then(Value::as_f64).unwrap_or(0.0) / fps;
    let layers = reader.layers(root.get("layers"));
    let main = reader.next_key();
    let name = root.get("nm").and_then(Value::as_str).filter(|name| !name.trim().is_empty()).map_or_else(|| file.file_stem().map_or("Lottie".into(), |stem| stem.to_string_lossy().into_owned()), str::to_owned);
    // The main animation first, so it opens.
    reader.scene.comps.insert(0, SceneComp { linear_blending: false, key: main, name, folder: None, width: width as u32, height: height as u32, fps, seconds: length(&root), background: Rgba::TRANSPARENT, layers });
    reader.unsupported.sort();
    reader.unsupported.dedup();
    if !reader.unsupported.is_empty() {
        reader.scene.notes.push(format!("Not brought in: {}.", reader.unsupported.join(", ")));
    }
    Ok(reader.scene)
}

/// The size each precomposition is shown at, by asset id, from the precomposition layers
/// (`ty` 0) that show it, in the main animation or in other precompositions.
fn precomp_sizes(root: &Value) -> HashMap<String, (f64, f64)> {
    let mut sizes = HashMap::new();
    let assets = root.get("assets").and_then(Value::as_array).into_iter().flatten();
    let layer_lists = std::iter::once(root.get("layers")).chain(assets.map(|asset| asset.get("layers")));
    for layer in layer_lists.flatten().filter_map(Value::as_array).flatten() {
        if layer.get("ty").and_then(Value::as_i64) != Some(0) {
            continue;
        }
        let id = layer.get("refId").and_then(Value::as_str);
        let size = layer.get("w").and_then(Value::as_f64).zip(layer.get("h").and_then(Value::as_f64));
        if let (Some(id), Some(size)) = (id, size) {
            sizes.entry(id.to_owned()).or_insert(size);
        }
    }
    sizes
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
    /// Seconds taken off every layer's timing: the main animation's start, while its layers are read.
    time_offset: f64,
    /// The file's fonts (`fonts.list`): family and weight by the name text documents use.
    fonts: HashMap<String, (String, u16)>,
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
        let start = number("st").unwrap_or(0.0) / fps - self.time_offset;
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
            5 => SceneContent::Text(text(layer, &self.fonts)?),
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
        if layer.get("bm").and_then(Value::as_u64).is_some_and(|number| BlendMode::from_lottie(number).is_none()) {
            self.unsupported.push("some blend modes (shown as Normal)");
        }
        let mut effects = Vec::new();
        for effect in layer.get("ef").and_then(Value::as_array).into_iter().flatten() {
            match drop_shadow(effect) {
                Some((shadow, black)) => {
                    effects.push(shadow);
                    if !black {
                        self.unsupported.push("drop shadow colors (shadows are black)");
                    }
                }
                None => self.unsupported.push("effects"),
            }
        }
        let transform = layer.get("ks").map(|ks| transform(ks, start, fps)).unwrap_or_default();
        Some(SceneLayer {
            key: layer.get("ind").and_then(Value::as_u64).unwrap_or_else(|| self.next_key() + 1_000_000),
            name,
            parent: layer.get("parent").and_then(Value::as_u64),
            start,
            in_point: number("ip").unwrap_or(0.0) / fps - self.time_offset,
            out_point: number("op").unwrap_or(fps * 10.0) / fps - self.time_offset,
            visible: layer.get("hd").and_then(Value::as_bool) != Some(true),
            // Modes Studio doesn't have come in as Normal (and are noted above).
            blend: layer.get("bm").and_then(Value::as_u64).and_then(BlendMode::from_lottie).unwrap_or(BlendMode::Normal),
            content,
            transform,
            effects,
        })
    }

    /// Shape items, as `aep::contents` does for After Effects: fills and strokes apply to the
    /// shapes in their group, and group transforms fold in. A gradient wins over a plain color
    /// in the same group (a file may give both, the color for players without gradients). The
    /// paths of one group are one compound shape, filled together as Lottie players do.
    fn contents(&mut self, items: &[Value], to_layer: Affine, fill: Option<Paint>, stroke: Option<Stroke>, out: &mut Vec<SceneShape>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut transform = Affine::IDENTITY;
        for item in items {
            if item.get("ty").and_then(Value::as_str) == Some("tr") {
                transform = group_transform(item);
            }
        }
        let to_layer = transform.then_outer(to_layer);
        // This group's own paints, by kind, with gradients mapped into the layer's pixels.
        let (mut color_fill, mut gradient_fill, mut color_stroke, mut gradient_stroke) = (None, None, None, None);
        let mut even_odd = false;
        for item in items {
            let width = || fixed(item.get("w")).and_then(|w| w.first().copied()).unwrap_or(1.0) as f32;
            match item.get("ty").and_then(Value::as_str) {
                Some("fl") => color_fill = paint(item).map(Paint::Solid),
                Some("gf") => gradient_fill = gradient(item, to_layer),
                Some("st") => color_stroke = paint(item).map(|color| Stroke { paint: Paint::Solid(color), width: width() }),
                Some("gs") => gradient_stroke = gradient(item, to_layer).map(|paint| Stroke { paint, width: width() }),
                Some("tm") => self.unsupported.push("trim paths"),
                Some("rp") => self.unsupported.push("repeaters"),
                _ => {}
            }
            // A fill's rule `r`: 1 non-zero, 2 even-odd.
            if matches!(item.get("ty").and_then(Value::as_str), Some("fl" | "gf")) {
                even_odd = item.get("r").and_then(Value::as_i64) == Some(2);
            }
        }
        let fill = gradient_fill.or(color_fill).or(fill);
        let stroke = gradient_stroke.or(color_stroke).or(stroke);
        let painted = fill.is_some() || stroke.is_some();

        // The group's paths, gathered into one shape placed where its first path is listed.
        let mut paths = items.iter().filter(|item| item.get("ty").and_then(Value::as_str) == Some("sh")).filter_map(|item| item.get("ks").and_then(first_value).and_then(bezier_path));
        let mut compound = paths.next().map(|mut first| {
            first.compound = paths.take(crate::path::MAX_COMPOUND_OUTLINES).collect();
            first.even_odd = even_odd && !first.compound.is_empty();
            first
        });
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
                Some("sh") => match compound.take() {
                    Some(path) => Geometry::Path(path),
                    None => continue,
                },
                _ => continue,
            };
            if painted {
                out.push(SceneShape { geometry: to_layer.geometry(geometry), fill, stroke });
            }
        }
    }
}

/// A file's fonts (`fonts.list`) by the name text documents refer to them by (`fName`): each
/// one's family (`fFamily`), and its weight (`fWeight`, or else read from its style name).
fn font_list(root: &Value) -> HashMap<String, (String, u16)> {
    let list = root.get("fonts").and_then(|fonts| fonts.get("list")).and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|font| {
            let name = font.get("fName").and_then(Value::as_str)?;
            let family = font.get("fFamily").and_then(Value::as_str).map(str::trim).filter(|family| !family.is_empty())?;
            // The weight is a number or a string of one ("700"); without it, the style name ("Bold") says.
            let stated = font.get("fWeight").and_then(|weight| weight.as_u64().or_else(|| weight.as_str()?.trim().parse().ok()));
            let from_style = || font.get("fStyle").and_then(Value::as_str).map(|style| super::font_family(&format!("-{style}")).1);
            let weight = stated.filter(|weight| (1..=1000).contains(weight)).map(|weight| weight as u16).or_else(from_style).unwrap_or(400);
            Some((name.to_owned(), (family.to_owned(), weight)))
        })
        .collect()
}

/// A gradient fill or stroke (`gf`, `gs`): linear (`t` 1) or radial (`t` 2) from its start `s`
/// to its end `e` (in the group's pixels, mapped into the layer's by `to_layer`), with `g.p`
/// color stops `[offset, r, g, b, …]` in `g.k`, any alpha stops `[offset, alpha, …]` after
/// them, and an opacity `o`. A gradient of one color is that color.
fn gradient(item: &Value, to_layer: Affine) -> Option<Paint> {
    let colors = item.get("g")?;
    let count = colors.get("p").and_then(Value::as_u64).filter(|count| *count > 0)? as usize;
    let values = fixed(colors.get("k"))?;
    let opacity = fixed(item.get("o")).and_then(|o| o.first().copied()).unwrap_or(100.0).clamp(0.0, 100.0) / 100.0;
    let color_values = values.get(..count.checked_mul(4)?)?;
    let alphas: Vec<(f64, f64)> = values[color_values.len()..].as_chunks::<2>().0.iter().map(|&[at, alpha]| (at, alpha)).collect();
    let stops: Vec<GradientStop> = color_values
        .as_chunks::<4>()
        .0
        .iter()
        .take(crate::gradient::MAX_STOPS)
        .map(|&[at, r, g, b]| {
            let channel = |value: f64| value.clamp(0.0, 1.0) as f32;
            let alpha = (alpha_at(&alphas, at) * opacity).clamp(0.0, 1.0) as f32;
            GradientStop { at: at as f32, color: Rgba { r: channel(r), g: channel(g), b: channel(b), a: alpha, link: None } }
        })
        .collect();
    if let [only] = stops.as_slice() {
        return Some(Paint::Solid(only.color));
    }
    let from = to_layer.point(fixed_pair(item.get("s")).unwrap_or_default());
    let to = to_layer.point(fixed_pair(item.get("e")).unwrap_or(Vec2::new(100.0, 0.0)));
    let gradient = Gradient::new(&stops, from, to);
    match item.get("t").and_then(Value::as_i64) {
        Some(2) => Some(Paint::Radial(gradient)),
        _ => Some(Paint::Linear(gradient)),
    }
}

/// The alpha a gradient's alpha stops give at `offset`, blending between the stops on either
/// side; fully opaque when there are none.
fn alpha_at(stops: &[(f64, f64)], offset: f64) -> f64 {
    let Some(&(first_at, first)) = stops.first() else { return 1.0 };
    if offset <= first_at {
        return first;
    }
    for pair in stops.windows(2) {
        let ((from_at, from), (to_at, to)) = (pair[0], pair[1]);
        if offset <= to_at {
            let span = to_at - from_at;
            return if span > 1e-9 { from + (to - from) * (offset - from_at) / span } else { to };
        }
    }
    stops.last().map_or(1.0, |&(_, alpha)| alpha)
}

/// Lottie's Drop Shadow layer effect (`ty` 25) as Studio's. Its values are, in order, the
/// shadow's color, opacity (0–255), direction (degrees clockwise from up), distance and
/// softness. Studio's shadows are black, so this also says whether the file's was (near enough).
fn drop_shadow(effect: &Value) -> Option<(SceneEffect, bool)> {
    if effect.get("ty").and_then(Value::as_i64) != Some(25) {
        return None;
    }
    let values: Vec<Vec<f64>> = effect.get("ef")?.as_array()?.iter().map(|value| fixed(value.get("v")).unwrap_or_default()).collect();
    let number = |index: usize, fallback: f64| values.get(index).and_then(|value| value.first().copied()).unwrap_or(fallback);
    let color = values.first().cloned().unwrap_or_default();
    let black = color.iter().take(3).all(|channel| *channel < 0.1);
    let color_alpha = color.get(3).copied().unwrap_or(1.0).clamp(0.0, 1.0);
    let opacity = number(1, 127.5) / 255.0 * 100.0 * color_alpha;
    let values = [opacity, number(2, 135.0), number(3, 5.0), number(4, 0.0)].map(|value| value as f32).to_vec();
    Some((SceneEffect { name: "Drop Shadow", values }, black))
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
    Some(ShapePath { points, closed: value.get("c").and_then(Value::as_bool).unwrap_or(false), ..Default::default() })
}

/// A layer's transform (`ks`).
fn transform(ks: &Value, start: f64, fps: f64) -> SceneTransform {
    let vec = |name: &str| ks.get(name).and_then(|property| track(property, start, fps, |values| Some(Vec2::new(*values.first()? as f32, *values.get(1).unwrap_or(&values[0]) as f32))));
    let number = |name: &str| ks.get(name).and_then(|property| track(property, start, fps, |values| values.first().map(|&v| v as f32)));
    // Position split into x and y ("s": true).
    let position = match ks.get("p") {
        Some(p) if p.get("s").and_then(Value::as_bool) == Some(true) => {
            let x = p.get("x").and_then(|x| track(x, start, fps, |v| v.first().map(|&v| v as f32)));
            let y = p.get("y").and_then(|y| track(y, start, fps, |v| v.first().map(|&v| v as f32)));
            x.zip(y).map(|(x, y)| super::combine_axes(&x, &y))
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

/// A text layer's first text document: its words, font (`f`, looked up in the file's font
/// list), size `s`, line height `lh`, tracking `tr`, fill `fc`, outline `sc` and `sw`, and
/// alignment `j`.
fn text(layer: &Value, fonts: &HashMap<String, (String, u16)>) -> Option<SceneText> {
    let document = layer.get("t")?.get("d")?.get("k")?.as_array()?.first()?.get("s")?;
    let words = document.get("t").and_then(Value::as_str)?.replace('\r', "\n");
    let number = |name: &str| document.get(name).and_then(Value::as_f64).filter(|value| value.is_finite()).map(|value| value as f32);
    let color = |name: &str| document.get(name).and_then(numbers).map(|c| Rgba { r: c[0].clamp(0.0, 1.0) as f32, g: c.get(1).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32, b: c.get(2).copied().unwrap_or(0.0).clamp(0.0, 1.0) as f32, a: 1.0, link: None });
    let align = match document.get("j").and_then(Value::as_i64) {
        Some(1) => TextAlign::Right,
        Some(2) => TextAlign::Center,
        _ => TextAlign::Left,
    };
    let font = document.get("f").and_then(Value::as_str).map(str::to_owned);
    Some(SceneText {
        family: font.as_ref().and_then(|font| fonts.get(font).cloned()),
        font,
        size: number("s").unwrap_or(72.0),
        line_height: number("lh").filter(|height| *height > 0.0),
        tracking: number("tr"),
        fill: color("fc"),
        stroke: color("sc").zip(number("sw")).filter(|(_, width)| *width > 0.0),
        align,
        text: words,
    })
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
    fn blend_modes_come_in_and_ones_studio_lacks_are_noted() {
        let text = r##"{"v":"5.7.4","fr":30,"ip":0,"op":30,"w":100,"h":100,"layers":[
            {"ty":1,"nm":"Shade","ind":1,"sc":"#000000","sw":100,"sh":100,"ip":0,"op":30,"st":0,"bm":1,"ks":{}},
            {"ty":1,"nm":"Dodge","ind":2,"sc":"#ffffff","sw":100,"sh":100,"ip":0,"op":30,"st":0,"bm":6,"ks":{}}]}"##;
        let scene = read(text, Path::new("/a/b.json"), Path::new("/tmp")).unwrap();
        let blends: Vec<BlendMode> = scene.comps[0].layers.iter().map(|layer| layer.blend).collect();
        assert_eq!(blends, [BlendMode::Multiply, BlendMode::Normal]);
        assert!(scene.notes.iter().any(|note| note.contains("blend modes")), "{:?}", scene.notes);
    }

    #[test]
    fn a_lottie_file_becomes_a_scene() {
        let scene = read(SAMPLE, Path::new("/tmp/demo.json"), Path::new("/tmp/assets")).unwrap();
        let comp = &scene.comps[0];
        assert_eq!((comp.name.as_str(), comp.width, comp.height, comp.fps, comp.seconds), ("Demo", 800, 600, 30.0, 2.0));
        assert_eq!(comp.layers.len(), 3);
        let SceneContent::Shapes(shapes) = &comp.layers[0].content else { panic!() };
        let Geometry::Rect { center, size, radius, .. } = shapes[0].geometry else { panic!() };
        assert_eq!((center, size, radius), (Vec2::new(10.0, 5.0), Vec2::new(80.0, 40.0), 8.0), "the group moved it");
        assert_eq!(shapes[0].fill, Some(Paint::Solid(Rgba::hex(0xff0000))));
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
        // The box layer is its one rectangle, following the null; its own pixels start at the
        // rectangle's corner, (10, 5) less half its size, so the anchor moves by as much.
        let box_layer = comp.layers.iter().find(|layer| layer.name == "Box").unwrap();
        let mover = comp.layers.iter().find(|layer| layer.name == "Mover").unwrap();
        assert_eq!(box_layer.parent, Some(mover.id));
        assert!(matches!(box_layer.content, crate::model::LayerContent::Shape { .. }));
        assert!(!comp.layers.iter().any(|layer| layer.parent == Some(box_layer.id)), "no hidden holder with a child");
        assert_eq!(box_layer.transform.as_ref().unwrap().anchor_point.value_at(0.0), Vec2::new(30.0, 15.0));
        let position = &box_layer.transform.as_ref().unwrap().position;
        assert_eq!(position.keyframe_frames(), vec![0, 30]);
    }

    /// A vector as Canvas sends it: one group of two outlines (a square with a square hole),
    /// a plain red fill then a red-to-blue gradient, a stroke, an identity group transform, and
    /// a drop shadow; the layer centered at (400, 300), turned 30° and scaled 150%.
    const CANVAS_VECTOR: &str = r##"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":800,"h":600,"layers":[
        {"ty":4,"nm":"Badge","ind":1,"ip":0,"op":60,"st":0,
         "ks":{"a":{"a":0,"k":[0,0]},"p":{"a":0,"k":[400,300]},"r":{"a":0,"k":30},"s":{"a":0,"k":[150,150]},"o":{"a":0,"k":100}},
         "ef":[{"ty":25,"nm":"Drop Shadow","ef":[
             {"ty":2,"v":{"a":0,"k":[0,0,0,1]}},{"ty":0,"v":{"a":0,"k":127.5}},{"ty":0,"v":{"a":0,"k":135}},
             {"ty":0,"v":{"a":0,"k":10}},{"ty":0,"v":{"a":0,"k":8}}]}],
         "shapes":[{"ty":"gr","it":[
             {"ty":"sh","ks":{"a":0,"k":{"v":[[-100,-100],[100,-100],[100,100],[-100,100]],"i":[[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0]],"c":true}}},
             {"ty":"sh","ks":{"a":0,"k":{"v":[[-50,-50],[-50,50],[50,50],[50,-50]],"i":[[0,0],[0,0],[0,0],[0,0]],"o":[[0,0],[0,0],[0,0],[0,0]],"c":true}}},
             {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100},"r":1},
             {"ty":"gf","t":1,"s":{"a":0,"k":[-100,0]},"e":{"a":0,"k":[100,0]},"o":{"a":0,"k":100},"r":1,
              "g":{"p":2,"k":{"a":0,"k":[0,1,0,0,1,0,0,1]}}},
             {"ty":"st","c":{"a":0,"k":[0,0,0,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":4}},
             {"ty":"tr","p":{"a":0,"k":[0,0]},"a":{"a":0,"k":[0,0]},"s":{"a":0,"k":[100,100]},"r":{"a":0,"k":0},"o":{"a":0,"k":100}}
         ]}]}]}"##;

    #[test]
    fn a_canvas_vector_arrives_as_one_editable_shape_layer() {
        use crate::model::LayerContent;
        let scene = read(CANVAS_VECTOR, Path::new("/tmp/badge.json"), Path::new("/tmp/assets")).unwrap();
        assert!(scene.notes.is_empty(), "the shadow and the gradient both came in: {:?}", scene.notes);
        let mut project = crate::sample::sample_project();
        let imported = super::super::build(&mut project, &scene, "badge", &|_| None).unwrap();
        let comp = project.composition(imported.compositions[0]).unwrap();
        assert_eq!(comp.layers.len(), 1, "one layer, no hidden holder");
        let layer = &comp.layers[0];
        let LayerContent::Shape { style, path: Some(path) } = &layer.content else { panic!("a shape layer: {:?}", layer.content) };
        assert_eq!(path.compound.len(), 1, "the hole is part of the same shape");
        // The gradient wins over the plain fill, running across the box left to right.
        let Some(crate::model::Paint::Linear(gradient)) = style.fill else { panic!("a linear gradient: {:?}", style.fill) };
        assert!((gradient.from - Vec2::new(0.0, 0.5)).length() < 1e-4 && (gradient.to - Vec2::new(1.0, 0.5)).length() < 1e-4, "{gradient:?}");
        assert_eq!(gradient.stops()[1].color, Rgba::hex(0x0000ff));
        assert_eq!(style.stroke.map(|stroke| stroke.width), Some(4.0));
        // Its own transform: turned and scaled about the shape's middle, which sits at (400, 300).
        let transform = layer.transform.as_ref().unwrap();
        assert_eq!((transform.rotation.value_at(0.0), transform.scale.value_at(0.0)), (30.0, Vec2::new(150.0, 150.0)));
        let corners = crate::transform::layer_corners(comp, layer.id, 0.0).unwrap();
        let middle = corners.iter().fold(Vec2::default(), |sum, corner| sum + *corner) * 0.25;
        assert!((middle - Vec2::new(400.0, 300.0)).length() < 0.01, "{middle:?}");
        assert!(((corners[1] - corners[0]).length() - 300.0).abs() < 0.01, "200 pixels at 150%");
        // Lottie's opacity of 127.5 out of 255 is Studio's 50%.
        let [shadow] = layer.effects.as_slice() else { panic!("one effect") };
        let values: Vec<f32> = shadow.properties.iter().map(|property| property.value.value_at(0.0)).collect();
        assert_eq!((shadow.name.as_str(), values), ("Drop Shadow", vec![50.0, 135.0, 10.0, 8.0]));
    }

    #[test]
    fn text_keeps_its_leading_tracking_outline_and_listed_font() {
        let file = r##"{"fr":30,"ip":0,"op":30,"w":800,"h":600,
            "fonts":{"list":[{"fName":"AvenirNext-Bold","fFamily":"Avenir Next","fStyle":"Bold","fWeight":"700"}]},
            "layers":[{"ty":5,"nm":"Title","ind":1,"ip":0,"op":30,"st":0,"ks":{},
            "t":{"d":{"k":[{"t":0,"s":{"t":"Hollow","s":60,"f":"AvenirNext-Bold","lh":90,"tr":50,"sc":[1,0,0],"sw":3,"j":0}}]}}}]}"##;
        let scene = read(file, Path::new("/tmp/title.json"), Path::new("/tmp/assets")).unwrap();
        let mut project = crate::sample::sample_project();
        let imported = super::super::build(&mut project, &scene, "title", &|_| None).unwrap();
        let comp = project.composition(imported.compositions[0]).unwrap();
        let crate::model::LayerContent::Text { style, .. } = &comp.layers[0].content else { panic!() };
        assert_eq!((style.font_family.as_str(), style.font_weight), ("Avenir Next", 700));
        assert_eq!((style.font_size, style.line_height, style.tracking), (60.0, 90.0, 50.0));
        assert_eq!(style.fill, None, "no fill color: outlined text");
        assert_eq!(style.stroke, Some((Rgba::hex(0xff0000), 3.0)));
    }

    #[test]
    fn gradient_alpha_stops_and_single_colors() {
        // Two color stops, fading from opaque to half see-through.
        let fading = serde_json::json!({"t": 2, "s": {"k": [0, 0]}, "e": {"k": [50, 0]}, "g": {"p": 2, "k": {"k": [0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 0.5]}}});
        let Some(Paint::Radial(radial)) = gradient(&fading, Affine::IDENTITY) else { panic!("a radial gradient") };
        assert_eq!((radial.stops()[0].color.a, radial.stops()[1].color.a), (1.0, 0.5));
        assert_eq!(radial.to, Vec2::new(50.0, 0.0));
        let one = serde_json::json!({"t": 1, "g": {"p": 1, "k": {"k": [0, 0, 1, 0]}}});
        assert_eq!(gradient(&one, Affine::IDENTITY), Some(Paint::Solid(Rgba::hex(0x00ff00))));
    }

    #[test]
    fn base64_and_unsafe_paths() {
        assert_eq!(decode_base64("aGk=").unwrap(), b"hi");
        assert_eq!(decode_base64("aGVsbG8").unwrap(), b"hello");
        let mut reader = Reader { scene: Scene::default(), fps: 30.0, assets: HashMap::new(), keys: 0, file: "/a/b.json".into(), asset_folder: "/tmp".into(), unsupported: Vec::new(), time_offset: 0.0, fonts: HashMap::new() };
        let climbing = serde_json::json!({"p": "secret.png", "u": "../../etc/"});
        assert!(reader.image(&climbing, 1).is_none(), "paths can't climb out of the animation's folder");
        let beside = serde_json::json!({"p": "img_0.png", "u": "images/"});
        let SourceKind::File { path, .. } = reader.image(&beside, 2).unwrap().kind else { panic!() };
        assert_eq!(path, PathBuf::from("/a/images/img_0.png"));
    }

    #[test]
    fn a_position_animated_only_up_and_down_keeps_its_motion() {
        let file = r#"{"fr": 30, "ip": 0, "op": 60, "w": 800, "h": 600, "layers": [
            {"ty": 3, "nm": "Bounce", "ip": 0, "op": 60, "st": 0,
             "ks": {"p": {"s": true, "x": {"a": 0, "k": 100}, "y": {"a": 1, "k": [{"t": 0, "s": [0]}, {"t": 30, "s": [500]}]}}}}
        ]}"#;
        let scene = read(file, Path::new("/tmp/bounce.json"), Path::new("/tmp/assets")).unwrap();
        let position = scene.comps[0].layers[0].transform.position.as_ref().unwrap();
        let keys: Vec<Vec2> = position.keys.iter().map(|(_, value, _)| *value).collect();
        assert_eq!(keys, vec![Vec2::new(100.0, 0.0), Vec2::new(100.0, 500.0)]);
    }

    #[test]
    fn an_animation_that_starts_late_starts_at_frame_zero() {
        let file = r#"{"fr": 30, "ip": 30, "op": 90, "w": 800, "h": 600, "layers": [
            {"ty": 3, "nm": "Early", "ip": 30, "op": 90, "st": 30, "ks": {"o": {"a": 1, "k": [{"t": 0, "s": [0]}, {"t": 30, "s": [100]}]}}},
            {"ty": 3, "nm": "Late", "ip": 70, "op": 90, "st": 0, "ks": {}}
        ]}"#;
        let scene = read(file, Path::new("/tmp/late.json"), Path::new("/tmp/assets")).unwrap();
        let comp = &scene.comps[0];
        assert_eq!(comp.seconds, 2.0);
        let timing: Vec<(f64, f64, f64)> = comp.layers.iter().map(|layer| (layer.start, layer.in_point, layer.out_point)).collect();
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(timing[0].1, 0.0) && close(timing[0].2, 2.0), "{timing:?}");
        assert!(close(timing[1].1, 40.0 / 30.0) && close(timing[1].2, 2.0), "{timing:?}");
        // Keyframes move with their layer: the fade still begins on the layer's first frame.
        let opacity = comp.layers[0].transform.opacity.as_ref().unwrap();
        assert!(close(opacity.keys[0].0, 0.0), "{:?}", opacity.keys);
    }

    #[test]
    fn a_precomp_takes_its_size_from_its_layer_and_keeps_late_content() {
        let file = r#"{"fr": 30, "ip": 0, "op": 60, "w": 800, "h": 600,
            "assets": [{"id": "inner", "layers": [{"ty": 3, "nm": "Late", "ip": 60, "op": 120, "st": 0, "ks": {}}]}],
            "layers": [{"ty": 0, "nm": "Holder", "refId": "inner", "w": 400, "h": 300, "ip": 0, "op": 60, "st": -60, "ks": {}}]
        }"#;
        let scene = read(file, Path::new("/tmp/pre.json"), Path::new("/tmp/assets")).unwrap();
        let inner = scene.comps.iter().find(|comp| comp.layers.iter().any(|layer| layer.name == "Late")).unwrap();
        assert_eq!((inner.width, inner.height), (400, 300));
        assert!(inner.seconds >= 4.0, "long enough for its layer: {}", inner.seconds);
        let mut project = crate::sample::sample_project();
        let imported = super::super::build(&mut project, &scene, "pre", &|_| None).unwrap();
        let built = imported.compositions.iter().filter_map(|id| project.composition(*id)).find(|comp| comp.width == 400).unwrap();
        assert_eq!(built.layers.len(), 1, "the late layer is kept");
    }

    #[test]
    fn other_files_are_refused() {
        assert!(read("not json", Path::new("x.json"), Path::new("/tmp")).is_err());
        assert!(read("{\"hello\": 1}", Path::new("x.json"), Path::new("/tmp")).is_err());
    }
}
