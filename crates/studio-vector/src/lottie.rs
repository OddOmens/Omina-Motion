//! Writes the scene as Lottie: the Bodymovin JSON format that lottie-web, the iOS and Android
//! Lottie libraries and LottieFiles play. Also as dotLottie, the same JSON zipped with a small
//! manifest.
//!
//! Layer types used: precomposition (0), solid (1), image (2), null (3), shape (4) and text (5,
//! only when no font outlines were available). Properties are written the usual way: `{"a": 0,
//! "k": value}` when constant, or `{"a": 1, "k": [keyframes]}` with each keyframe's easing as
//! its out tangent `o` and the next keyframe's in tangent `i` (the two control points of the
//! cubic Bézier), or `"h": 1` for a hold.

use serde_json::{Map, Value, json};
use studio_core::animation::Vec2;
use studio_core::model::{BlendMode, Id, Paint, Project, Rgba, StrokeCap, StrokeJoin, StrokePosition, TextAlign};

use crate::encode::{data_url, zip};
use crate::geometry::Geometry;
use crate::scene::{self, Comp, Content, Ease, Group, Key, LiveText, Matte, PaintOp, Scene, SceneLayer, Track, Transform};
use crate::{Exported, Resources, VectorOptions};

/// The Bodymovin version the files say they follow.
const VERSION: &str = "5.7.4";

/// Exports `options` as Lottie JSON.
pub fn lottie(project: &Project, options: &VectorOptions, resources: &mut dyn Resources) -> Result<Exported<String>, String> {
    let scene = scene::build(project, options, resources)?;
    let mut writer = Writer::default();
    let document = writer.document(&scene);
    let mut notes = scene.notes;
    for (topic, name) in writer.notes {
        if name.is_empty() {
            notes.add_general(topic);
        } else {
            notes.add(topic, crate::notes::quoted(&name));
        }
    }
    Ok(Exported { file: document.to_string(), left_out: notes.into_lines() })
}

/// Exports `options` as a dotLottie file: a zip holding `manifest.json` and the animation as
/// `animations/<id>.json` (version 1 of the dotLottie layout, which every dotLottie player reads).
pub fn dotlottie(project: &Project, options: &VectorOptions, resources: &mut dyn Resources) -> Result<Exported<Vec<u8>>, String> {
    let name = project.composition(options.composition).map_or_else(String::new, |comp| comp.name.clone());
    let Exported { file, left_out } = lottie(project, options, resources)?;
    let id = animation_id(&name);
    let manifest = json!({
        "version": "1",
        "generator": "Studio",
        "author": "",
        "revision": 1,
        "animations": [{ "id": id, "speed": 1, "loop": true, "direction": 1, "playMode": "normal", "autoplay": true }],
        "activeAnimationId": id,
    });
    let manifest = manifest.to_string();
    let path = format!("animations/{id}.json");
    let archive = zip(&[("manifest.json", manifest.as_bytes()), (path.as_str(), file.as_bytes())]);
    Ok(Exported { file: archive, left_out })
}

/// An animation id for the dotLottie manifest: the composition's name in lower case, with
/// anything but letters and digits turned into dashes.
fn animation_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id = id.trim_end_matches('-').to_owned();
    if id.is_empty() { "animation".into() } else { id }
}

/// A number rounded to a thousandth, which keeps files small and is far finer than a pixel.
fn num(value: f64) -> Value {
    if !value.is_finite() {
        return json!(0);
    }
    let rounded = (value * 1000.0).round() / 1000.0;
    // No "-0" in the file.
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    if rounded.fract() == 0.0 && rounded.abs() < 1e15 { json!(rounded as i64) } else { json!(rounded) }
}

fn nums(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|&value| num(value)).collect())
}

fn point(v: Vec2) -> Value {
    nums(&[f64::from(v.x), f64::from(v.y)])
}

/// A constant property.
/// Lottie's number for a stroke's ends: 1 flat, 2 round, 3 square.
fn line_cap(cap: StrokeCap) -> u8 {
    match cap {
        StrokeCap::Butt => 1,
        StrokeCap::Round => 2,
        StrokeCap::Square => 3,
    }
}

/// Lottie's number for a stroke's corners: 1 sharp (miter), 2 round, 3 beveled.
fn line_join(join: StrokeJoin) -> u8 {
    match join {
        StrokeJoin::Miter => 1,
        StrokeJoin::Round => 2,
        StrokeJoin::Bevel => 3,
    }
}

fn fixed(value: Value) -> Value {
    json!({ "a": 0, "k": value })
}

/// An animatable property from a track. One-number values are written as plain numbers when
/// constant, and as one-item arrays in keyframes, as Bodymovin does.
fn property(track: &Track) -> Value {
    match track {
        Track::Static(value) if value.len() == 1 => fixed(num(value[0])),
        Track::Static(value) => fixed(nums(value)),
        Track::Keys(keys) => json!({ "a": 1, "k": keys.iter().enumerate().map(|(index, key)| keyframe(key, index + 1 == keys.len())).collect::<Vec<_>>() }),
    }
}

fn keyframe(key: &Key, last: bool) -> Value {
    let mut object = Map::new();
    object.insert("t".into(), num(key.time));
    object.insert("s".into(), nums(&key.value));
    if !last {
        let dimensions = key.value.len();
        let tangent = |x: f32, y: f32| json!({ "x": vec![num(f64::from(x)); dimensions], "y": vec![num(f64::from(y)); dimensions] });
        match key.ease {
            Ease::Hold => {
                object.insert("h".into(), json!(1));
            }
            Ease::Linear => {
                object.insert("o".into(), tangent(0.0, 0.0));
                object.insert("i".into(), tangent(1.0, 1.0));
            }
            Ease::Curve(curve) => {
                object.insert("o".into(), tangent(curve.x1, curve.y1));
                object.insert("i".into(), tangent(curve.x2, curve.y2));
            }
        }
    }
    Value::Object(object)
}

fn transform_properties(transform: &Transform) -> Map<String, Value> {
    let mut object = Map::new();
    object.insert("a".into(), property(&transform.anchor));
    object.insert("p".into(), property(&transform.position));
    object.insert("s".into(), property(&transform.scale));
    object.insert("r".into(), property(&transform.rotation));
    object.insert("o".into(), property(&transform.opacity));
    object
}

/// A color as Lottie's 0–1 RGBA list.
fn color(rgba: Rgba) -> Value {
    nums(&[f64::from(rgba.r), f64::from(rgba.g), f64::from(rgba.b), 1.0])
}

fn blend(mode: BlendMode) -> i64 {
    mode.lottie_number() as i64
}

fn image_id(item: Id) -> String {
    format!("image_{}", item.0)
}

fn comp_id(comp: Id) -> String {
    format!("comp_{}", comp.0)
}

/// Builds the document, remembering the fonts live text uses.
#[derive(Default)]
struct Writer {
    fonts: Vec<(String, String, u16)>,
    notes: Vec<(&'static str, String)>,
}

impl Writer {
    fn document(&mut self, scene: &Scene) -> Value {
        let mut assets: Vec<Value> = scene
            .images
            .iter()
            .map(|(item, image)| json!({ "id": image_id(*item), "w": image.width, "h": image.height, "u": "", "p": data_url(image.mime, &image.bytes), "e": 1 }))
            .collect();
        for (id, comp) in &scene.precomps {
            let layers = self.layers(comp);
            assets.push(json!({ "id": comp_id(*id), "nm": comp.name, "layers": layers }));
        }
        let layers = self.layers(&scene.root);
        let markers: Vec<Value> = scene.markers.iter().map(|(time, label)| json!({ "tm": num(*time), "cm": label, "dr": 0 })).collect();
        let mut document = json!({
            "v": VERSION,
            "fr": num(scene.fps),
            "ip": 0,
            "op": num(scene.root.length),
            "w": scene.root.size.x.round() as i64,
            "h": scene.root.size.y.round() as i64,
            "nm": scene.root.name,
            "ddd": 0,
            "assets": assets,
            "layers": layers,
            "markers": markers,
            "meta": { "g": "Studio" },
        });
        if !self.fonts.is_empty() {
            let list: Vec<Value> = self.fonts.iter().map(|(name, family, weight)| json!({ "fName": name, "fFamily": family, "fStyle": weight_name(*weight), "ascent": 75 })).collect();
            document["fonts"] = json!({ "list": list });
        }
        document
    }

    fn layers(&mut self, comp: &Comp) -> Vec<Value> {
        comp.layers.iter().enumerate().map(|(index, layer)| self.layer(index, layer)).collect()
    }

    fn layer(&mut self, index: usize, layer: &SceneLayer) -> Value {
        let mut object = Map::new();
        object.insert("ddd".into(), json!(0));
        object.insert("ind".into(), json!(index + 1));
        object.insert("nm".into(), json!(layer.name));
        object.insert("sr".into(), json!(1));
        object.insert("ao".into(), json!(0));
        object.insert("ip".into(), num(layer.span.0));
        object.insert("op".into(), num(layer.span.1));
        object.insert("st".into(), json!(0));
        object.insert("bm".into(), json!(blend(layer.blend)));
        if let Some(parent) = layer.parent {
            object.insert("parent".into(), json!(parent + 1));
        }
        if layer.is_matte {
            object.insert("td".into(), json!(1));
        }
        if let Some(matte) = layer.matte {
            if matte == Matte::LumaInverted {
                self.notes.push(("Luma inverted mattes are standard Lottie, but some players (lottie-web among them) don’t draw them", layer.name.clone()));
            }
            let kind = match matte {
                Matte::Alpha => 1,
                Matte::AlphaInverted => 2,
                Matte::Luma => 3,
                Matte::LumaInverted => 4,
            };
            object.insert("tt".into(), json!(kind));
        }
        let mut transform = layer.transform.clone();
        let kind = match &layer.content {
            Content::Null => 3,
            Content::Solid { color, size } if color.a >= 0.999 => {
                object.insert("sc".into(), json!(color.to_hex().to_lowercase()));
                object.insert("sw".into(), json!(size.x.round() as i64));
                object.insert("sh".into(), json!(size.y.round() as i64));
                1
            }
            Content::Solid { color, size } => {
                // A see-through solid: a rectangle filled with the color at its opacity.
                let group = Group {
                    name: layer.name.clone(),
                    transform: None,
                    shapes: vec![Geometry::Rect { center: *size * 0.5, size: *size, radius: 0.0 }],
                    paints: vec![PaintOp::Fill { paint: Paint::Solid(*color), area: *size }],
                    children: Vec::new(),
                };
                object.insert("shapes".into(), json!([self.group(&group)]));
                4
            }
            Content::Shapes(groups) => {
                let shapes: Vec<Value> = groups.iter().map(|group| self.group(group)).collect();
                object.insert("shapes".into(), Value::Array(shapes));
                4
            }
            Content::Image { item, size, native } => {
                object.insert("refId".into(), json!(image_id(*item)));
                transform = stretched(&transform, *size, *native);
                2
            }
            Content::Precomp { comp, size, native, start } => {
                transform = stretched(&transform, *size, *native);
                object.insert("refId".into(), json!(comp_id(*comp)));
                object.insert("w".into(), json!(native.x.round() as i64));
                object.insert("h".into(), json!(native.y.round() as i64));
                object.insert("st".into(), num(*start));
                0
            }
            Content::Text(text) => {
                let (data, baseline) = self.live_text(text);
                object.insert("t".into(), data);
                // Lottie places text from its first baseline; Studio from the box's top left.
                transform.anchor = transform.anchor.map(|anchor| vec![anchor[0] - f64::from(baseline.x), anchor[1] - f64::from(baseline.y)]);
                5
            }
        };
        object.insert("ty".into(), json!(kind));
        object.insert("ks".into(), Value::Object(transform_properties(&transform)));
        Value::Object(object)
    }

    /// A group of shapes (`gr`): outlines, then paints top first (Lottie draws earlier items over
    /// later ones), then the group's transform, which must come last.
    fn group(&mut self, group: &Group) -> Value {
        let mut items: Vec<Value> = Vec::new();
        if !group.children.is_empty() {
            items.extend(group.children.iter().map(|child| self.group(child)));
        } else if group.paints.iter().any(needs_unit_space) {
            // A radial gradient in a box that isn't square stretches with the box in Studio.
            // Lottie's are round, so that fill is drawn in a unit square stretched to the box.
            for paint in group.paints.iter().rev() {
                let part = if needs_unit_space(paint) {
                    let area = paint_area(paint);
                    let unit = Vec2::new(1.0 / area.x.max(1e-3), 1.0 / area.y.max(1e-3));
                    let mut items: Vec<Value> = group.shapes.iter().map(|shape| geometry(&shape.scaled(unit))).collect();
                    items.push(self.paint(paint, Vec2::new(1.0, 1.0)));
                    items.push(group_transform(&Transform::scaling(area)));
                    items
                } else {
                    let mut items: Vec<Value> = group.shapes.iter().map(geometry).collect();
                    items.push(self.paint(paint, paint_area(paint)));
                    items.push(group_transform(&Transform::identity()));
                    items
                };
                let count = part.len();
                items.push(json!({ "ty": "gr", "nm": group.name, "np": count, "it": part }));
            }
        } else {
            items.extend(group.shapes.iter().map(geometry));
            items.extend(group.paints.iter().rev().map(|paint| self.paint(paint, paint_area(paint))));
        }
        items.push(group_transform(group.transform.as_ref().unwrap_or(&Transform::identity())));
        json!({ "ty": "gr", "nm": group.name, "np": items.len(), "it": items })
    }

    /// A fill or stroke item, with gradient points in a box of `area`.
    fn paint(&mut self, paint: &PaintOp, area: Vec2) -> Value {
        match paint {
            PaintOp::Fill { paint, .. } => match paint {
                Paint::Solid(rgba) => json!({ "ty": "fl", "c": fixed(color(*rgba)), "o": fixed(num(f64::from(rgba.a) * 100.0)), "r": 1 }),
                _ => {
                    let mut object = gradient(*paint, area);
                    object.insert("ty".into(), json!("gf"));
                    object.insert("r".into(), json!(1));
                    Value::Object(object)
                }
            },
            PaintOp::Stroke { paint, width, line, .. } => {
                let mut object = match paint {
                    Paint::Solid(rgba) => {
                        let mut object = Map::new();
                        object.insert("ty".into(), json!("st"));
                        object.insert("c".into(), fixed(color(*rgba)));
                        object.insert("o".into(), fixed(num(f64::from(rgba.a) * 100.0)));
                        object
                    }
                    _ => {
                        if matches!(paint, Paint::Radial(_)) && (area.x - area.y).abs() > 0.5 {
                            self.notes.push(("Radial gradients on outlines in boxes that aren’t square are drawn round", String::new()));
                        }
                        let mut object = gradient(*paint, area);
                        object.insert("ty".into(), json!("gs"));
                        object
                    }
                };
                object.insert("w".into(), fixed(num(f64::from(*width))));
                object.insert("lc".into(), json!(line_cap(line.cap)));
                object.insert("lj".into(), json!(line_join(line.join)));
                object.insert("ml".into(), json!(num(f64::from(line.miter_limit))));
                if let Some((dash, gap)) = line.dash {
                    let length = |name: &str, value: f32| json!({ "n": name, "nm": name, "v": fixed(num(f64::from(value))) });
                    object.insert("d".into(), json!([length("d", dash), length("g", gap), length("o", 0.0)]));
                }
                Value::Object(object)
            }
        }
    }

    /// Live text (`ty` 5): the text, its font and colors, and where its first baseline sits in
    /// the layer's box (an estimate, since it depends on the player's font).
    fn live_text(&mut self, text: &LiveText) -> (Value, Vec2) {
        let style = &text.style;
        let family = style.font_family.clone();
        let name = format!("{}-{}", family.replace(' ', ""), weight_name(style.font_weight).replace(' ', ""));
        if !self.fonts.iter().any(|(known, _, _)| *known == name) {
            self.fonts.push((name.clone(), family, style.font_weight));
        }
        let justify = match style.align {
            TextAlign::Left => 0,
            TextAlign::Right => 1,
            TextAlign::Center => 2,
        };
        let stroke = style.stroke.filter(|(_, width)| *width > 0.0);
        let mut document = json!({
            "s": num(f64::from(style.font_size)),
            "f": name,
            // Lottie separates lines with carriage returns.
            "t": text.text.replace('\n', "\r"),
            "j": justify,
            "tr": num(f64::from(style.tracking)),
            "lh": num(f64::from(style.line_height)),
            "ls": 0,
        });
        // Without a fill color the letters are left unfilled: hollow when outlined, as Studio
        // draws them. The outline is doubled under a fill, which covers its inner half.
        if let Some(fill) = style.fill {
            document["fc"] = nums(&[f64::from(fill.r), f64::from(fill.g), f64::from(fill.b)]);
        }
        if let Some((rgba, width)) = stroke {
            // As `scene::text_paints` writes outlined text: an outside stroke under a fill is
            // doubled under it; others are centered, over the fill when set so.
            let options = style.stroke_options;
            let outside_under_fill = style.fill.is_some() && options.position == StrokePosition::Outside;
            document["sc"] = nums(&[f64::from(rgba.r), f64::from(rgba.g), f64::from(rgba.b)]);
            document["sw"] = num(f64::from(width) * if outside_under_fill { 2.0 } else { 1.0 });
            document["of"] = json!(style.fill.is_none() || (options.over_fill && !outside_under_fill));
        }
        let x = match style.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => text.size.x * 0.5,
            TextAlign::Right => text.size.x,
        };
        // The baseline sits about a third of the font size below the middle of the first line.
        let baseline = Vec2::new(x, style.line_height * 0.5 + style.font_size * 0.32);
        let data = json!({ "d": { "k": [{ "s": document, "t": 0 }] }, "p": {}, "m": { "g": 1, "a": fixed(json!([0, 0])) }, "a": [] });
        (data, baseline)
    }
}

/// A layer's transform for content Lottie draws at its own `native` size (a picture or nested
/// composition) where Studio stretches it to the layer's box of `size`. Layers parented to it
/// follow its stand-in null, which keeps the plain transform, so they aren't stretched too.
fn stretched(transform: &Transform, size: Vec2, native: Vec2) -> Transform {
    let factor = Vec2::new(size.x / native.x.max(1e-3), size.y / native.y.max(1e-3));
    if (factor.x - 1.0).abs() < 1e-4 && (factor.y - 1.0).abs() < 1e-4 {
        return transform.clone();
    }
    transform.for_content_scaled(factor)
}

fn paint_area(paint: &PaintOp) -> Vec2 {
    match paint {
        PaintOp::Fill { area, .. } | PaintOp::Stroke { area, .. } => *area,
    }
}

/// Whether a fill is a radial gradient in a box that isn't square (see `Writer::group`).
fn needs_unit_space(paint: &PaintOp) -> bool {
    matches!(paint, PaintOp::Fill { paint: Paint::Radial(_), area } if (area.x - area.y).abs() > 0.5)
}

/// A group's transform item (`tr`).
fn group_transform(transform: &Transform) -> Value {
    let mut object = transform_properties(transform);
    object.insert("ty".into(), json!("tr"));
    object.insert("nm".into(), json!("Transform"));
    object.insert("sk".into(), fixed(json!(0)));
    object.insert("sa".into(), fixed(json!(0)));
    Value::Object(object)
}

/// An outline item: a rectangle (`rc`), an ellipse (`el`) or a path (`sh`).
fn geometry(shape: &Geometry) -> Value {
    match shape {
        Geometry::Rect { center, size, radius } => json!({ "ty": "rc", "d": 1, "p": fixed(point(*center)), "s": fixed(point(*size)), "r": fixed(num(f64::from(*radius))) }),
        Geometry::Ellipse { center, size } => json!({ "ty": "el", "d": 1, "p": fixed(point(*center)), "s": fixed(point(*size)) }),
        Geometry::Path(path) => {
            let vertices: Vec<Value> = path.points.iter().map(|p| point(p.at)).collect();
            let ins: Vec<Value> = path.points.iter().map(|p| point(p.handle_in)).collect();
            let outs: Vec<Value> = path.points.iter().map(|p| point(p.handle_out)).collect();
            json!({ "ty": "sh", "d": 1, "ks": fixed(json!({ "c": path.closed, "v": vertices, "i": ins, "o": outs })) })
        }
    }
}

/// The gradient half of a gradient fill or stroke: its kind, ends and stops. Stop opacities
/// follow the colors, as Lottie lists them, when any stop is see-through.
fn gradient(paint: Paint, area: Vec2) -> Map<String, Value> {
    let gradient = paint.gradient().expect("only gradients come here").sanitized();
    let stops = gradient.stops();
    let mut values: Vec<f64> = stops.iter().flat_map(|stop| [f64::from(stop.at), f64::from(stop.color.r), f64::from(stop.color.g), f64::from(stop.color.b)]).collect();
    if stops.iter().any(|stop| stop.color.a < 0.999) {
        values.extend(stops.iter().flat_map(|stop| [f64::from(stop.at), f64::from(stop.color.a)]));
    }
    let at = |v: Vec2| Vec2::new(v.x * area.x, v.y * area.y);
    let mut object = Map::new();
    object.insert("o".into(), fixed(json!(100)));
    object.insert("t".into(), json!(if matches!(paint, Paint::Radial(_)) { 2 } else { 1 }));
    object.insert("s".into(), fixed(point(at(gradient.from))));
    object.insert("e".into(), fixed(point(at(gradient.to))));
    object.insert("g".into(), json!({ "p": stops.len(), "k": fixed(nums(&values)) }));
    if matches!(paint, Paint::Radial(_)) {
        object.insert("h".into(), fixed(json!(0)));
        object.insert("a".into(), fixed(json!(0)));
    }
    object
}

/// The usual name of a font weight, for the font list.
fn weight_name(weight: u16) -> &'static str {
    match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NoResources;
    use studio_core::sample::{ids, sample_project};

    #[test]
    fn a_dotlottie_holds_a_manifest_and_the_same_animation() {
        let project = sample_project();
        let options = VectorOptions { composition: ids::MAIN_COMP, frames: 0..48, frame_rate: None };
        let archive = dotlottie(&project, &options, &mut NoResources).unwrap().file;
        let files = crate::encode::unzip(&archive);
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["manifest.json", "animations/main-comp.json"]);
        let manifest: Value = serde_json::from_slice(&files[0].1).unwrap();
        assert_eq!(manifest["animations"][0]["id"], "main-comp");
        assert_eq!(manifest["activeAnimationId"], "main-comp");
        let plain = lottie(&project, &options, &mut NoResources).unwrap().file;
        assert_eq!(files[1].1, plain.into_bytes(), "the same JSON a Lottie export writes");
    }

    #[test]
    fn animation_ids_are_plain_lowercase_words() {
        assert_eq!(animation_id("Launch Day — Wide 16:9"), "launch-day-wide-16-9");
        assert_eq!(animation_id("…"), "animation");
    }
}
