//! The parts of the Lottie format Studio writes, as typed structs that refuse unknown fields,
//! and the rules between fields that types can't say (a track matte needs a matte layer above
//! it, keyframes need tangents, a group ends with its transform). Reading an export back
//! through these checks it is Lottie a player can read, field by field.

#![allow(dead_code)]

use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Animation {
    pub v: String,
    pub fr: f64,
    pub ip: f64,
    pub op: f64,
    pub w: u32,
    pub h: u32,
    pub nm: String,
    pub ddd: u8,
    pub assets: Vec<Asset>,
    pub layers: Vec<Layer>,
    pub markers: Vec<Marker>,
    pub meta: Meta,
    #[serde(default)]
    pub fonts: Option<Fonts>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub g: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    pub tm: f64,
    pub cm: String,
    pub dr: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    pub list: Vec<Font>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct Font {
    pub fName: String,
    pub fFamily: String,
    pub fStyle: String,
    pub ascent: f64,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Asset {
    Image(ImageAsset),
    Precomp(PrecompAsset),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAsset {
    pub id: String,
    pub w: u32,
    pub h: u32,
    pub u: String,
    pub p: String,
    pub e: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrecompAsset {
    pub id: String,
    pub nm: String,
    pub layers: Vec<Layer>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct Layer {
    pub ddd: u8,
    pub ind: u32,
    pub ty: u8,
    pub nm: String,
    pub sr: f64,
    pub ao: u8,
    pub ip: f64,
    pub op: f64,
    pub st: f64,
    pub bm: u8,
    pub ks: LayerTransform,
    pub parent: Option<u32>,
    pub td: Option<u8>,
    pub tt: Option<u8>,
    pub shapes: Option<Vec<ShapeItem>>,
    pub refId: Option<String>,
    pub sc: Option<String>,
    pub sw: Option<u32>,
    pub sh: Option<u32>,
    pub w: Option<u32>,
    pub h: Option<u32>,
    pub t: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerTransform {
    pub a: Property,
    pub p: Property,
    pub s: Property,
    pub r: Property,
    pub o: Property,
}

/// `{"a": 1, "k": [keyframes]}` or `{"a": 0, "k": value}`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Property {
    Animated { a: u8, k: Vec<Keyframe> },
    Static { a: u8, k: Value },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyframe {
    pub t: f64,
    pub s: Vec<f64>,
    pub i: Option<Tangent>,
    pub o: Option<Tangent>,
    pub h: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tangent {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "ty", deny_unknown_fields)]
pub enum ShapeItem {
    #[serde(rename = "gr")]
    Group { nm: String, np: u32, it: Vec<ShapeItem> },
    #[serde(rename = "rc")]
    Rect { d: u8, p: Property, s: Property, r: Property },
    #[serde(rename = "el")]
    Ellipse { d: u8, p: Property, s: Property },
    #[serde(rename = "sh")]
    Path { d: u8, ks: PathProperty },
    #[serde(rename = "fl")]
    Fill { c: Property, o: Property, r: u8 },
    #[serde(rename = "gf")]
    GradientFill { o: Property, r: u8, t: u8, s: Property, e: Property, g: GradientStops, h: Option<Property>, a: Option<Property> },
    #[serde(rename = "st")]
    Stroke { c: Property, o: Property, w: Property, lc: u8, lj: u8, ml: f64 },
    #[serde(rename = "gs")]
    GradientStroke { o: Property, t: u8, s: Property, e: Property, g: GradientStops, h: Option<Property>, a: Option<Property>, w: Property, lc: u8, lj: u8, ml: f64 },
    #[serde(rename = "tr")]
    Transform { nm: String, a: Property, p: Property, s: Property, r: Property, o: Property, sk: Property, sa: Property },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GradientStops {
    pub p: usize,
    pub k: Property,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathProperty {
    pub a: u8,
    pub k: PathShape,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathShape {
    pub c: bool,
    pub v: Vec<[f64; 2]>,
    pub i: Vec<[f64; 2]>,
    pub o: Vec<[f64; 2]>,
}

/// Reads `json` as Lottie and checks the rules between its fields. Panics with what is wrong.
pub fn check(json: &str) -> Animation {
    let animation: Animation = serde_json::from_str(json).unwrap_or_else(|error| panic!("not Lottie as Studio writes it: {error}"));
    assert!(animation.fr > 0.0 && animation.op > animation.ip, "frame rate and length");
    let images: HashSet<&str> = animation.assets.iter().filter_map(|asset| if let Asset::Image(image) = asset { Some(image.id.as_str()) } else { None }).collect();
    let comps: HashSet<&str> = animation.assets.iter().filter_map(|asset| if let Asset::Precomp(comp) = asset { Some(comp.id.as_str()) } else { None }).collect();
    for asset in &animation.assets {
        match asset {
            Asset::Image(image) => assert!(image.p.starts_with("data:image/") && image.e == 1, "images are embedded"),
            Asset::Precomp(comp) => check_layers(&comp.layers, &images, &comps),
        }
    }
    check_layers(&animation.layers, &images, &comps);
    animation
}

fn check_layers(layers: &[Layer], images: &HashSet<&str>, comps: &HashSet<&str>) {
    let indices: HashSet<u32> = layers.iter().map(|layer| layer.ind).collect();
    assert_eq!(indices.len(), layers.len(), "layer indices are unique");
    for (position, layer) in layers.iter().enumerate() {
        let name = &layer.nm;
        assert!(layer.op >= layer.ip, "{name}: out point after in point");
        if let Some(parent) = layer.parent {
            assert!(indices.contains(&parent) && parent != layer.ind, "{name}: parent {parent} exists");
        }
        if let Some(kind) = layer.tt {
            assert!(position > 0 && layers[position - 1].td == Some(1), "{name}: a track matte needs a matte layer directly above");
            assert!((1..=4).contains(&kind));
        }
        for property in [&layer.ks.a, &layer.ks.p, &layer.ks.s, &layer.ks.r, &layer.ks.o] {
            check_property(property, name);
        }
        match layer.ty {
            0 => assert!(layer.refId.as_deref().is_some_and(|id| comps.contains(id)) && layer.w.is_some() && layer.h.is_some(), "{name}: precomp"),
            1 => assert!(layer.sc.as_deref().is_some_and(|color| color.starts_with('#') && color.len() == 7) && layer.sw.is_some() && layer.sh.is_some(), "{name}: solid"),
            2 => assert!(layer.refId.as_deref().is_some_and(|id| images.contains(id)), "{name}: image"),
            3 => {}
            4 => layer.shapes.as_ref().expect("shape layers have shapes").iter().for_each(|item| check_shape(item, name)),
            5 => assert!(layer.t.is_some(), "{name}: text data"),
            other => panic!("{name}: unexpected layer type {other}"),
        }
    }
}

fn check_property(property: &Property, name: &str) {
    match property {
        Property::Static { a, .. } => assert_eq!(*a, 0, "{name}: a constant property has a = 0"),
        Property::Animated { a, k } => {
            assert_eq!(*a, 1, "{name}: keyframes have a = 1");
            assert!(k.len() >= 2, "{name}: at least two keyframes");
            for pair in k.windows(2) {
                assert!(pair[1].t >= pair[0].t, "{name}: keyframes in time order");
            }
            let dimensions = k[0].s.len();
            for (index, key) in k.iter().enumerate() {
                assert_eq!(key.s.len(), dimensions, "{name}: every keyframe has the same number of values");
                if index + 1 == k.len() {
                    continue;
                }
                if key.h == Some(1) {
                    continue;
                }
                let (i, o) = (key.i.as_ref().expect("in tangent"), key.o.as_ref().expect("out tangent"));
                for tangent in [i, o] {
                    assert_eq!((tangent.x.len(), tangent.y.len()), (dimensions, dimensions), "{name}: a tangent per value");
                    assert!(tangent.x.iter().all(|x| (0.0..=1.0).contains(x)), "{name}: tangent x within 0–1");
                }
            }
        }
    }
}

fn check_shape(item: &ShapeItem, name: &str) {
    match item {
        ShapeItem::Group { it, .. } => {
            assert!(matches!(it.last(), Some(ShapeItem::Transform { .. })), "{name}: a group ends with its transform");
            assert_eq!(it.iter().filter(|item| matches!(item, ShapeItem::Transform { .. })).count(), 1, "{name}: one transform per group");
            it.iter().for_each(|item| check_shape(item, name));
        }
        ShapeItem::Path { ks, .. } => {
            assert_eq!(ks.a, 0);
            assert!(ks.k.v.len() == ks.k.i.len() && ks.k.v.len() == ks.k.o.len(), "{name}: a tangent pair per vertex");
        }
        ShapeItem::GradientFill { g, t, .. } | ShapeItem::GradientStroke { g, t, .. } => {
            assert!(*t == 1 || *t == 2);
            let Property::Static { k, .. } = &g.k else { panic!("{name}: constant gradient") };
            let count = k.as_array().expect("stops").len();
            assert!(count == g.p * 4 || count == g.p * 6, "{name}: {} color stops, with or without opacity stops, in {count} numbers", g.p);
        }
        ShapeItem::Stroke { lc, lj, .. } => assert!((1..=3).contains(lc) && (1..=3).contains(lj)),
        _ => {}
    }
}
