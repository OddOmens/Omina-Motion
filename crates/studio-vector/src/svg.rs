//! Writes the scene as an animated SVG that plays in any modern browser, on its own or in an
//! `<img>` tag. Movement is CSS keyframe animation: each animated part of a transform (position,
//! rotation, scale, anchor point) gets a nested group of its own with its own keyframes, so
//! each keeps its own timing, and each keyframe's easing becomes that segment's
//! `animation-timing-function` (`cubic-bezier(…)`, `linear`, or `steps(1, end)` for a hold).
//! The whole animation loops, as Lottie players do.
//!
//! Layers are drawn bottom first. Track mattes and mask layers become SVG masks, nested
//! compositions become clipped groups whose animations are shifted to when they start, and
//! pictures are embedded as data URLs.

use std::fmt::Write as _;

use studio_core::animation::Vec2;
use studio_core::model::{BlendMode, Paint, Project, Rgba, StrokeCap, StrokeJoin, StrokePosition, TextAlign};

use crate::encode::data_url;
use crate::geometry::Geometry;
use crate::scene::{self, Comp, Content, Ease, Group, Key, LiveText, Matte, PaintOp, Scene, SceneLayer, Track, Transform};
use crate::{Exported, Resources, VectorOptions};

/// Exports `options` as an animated SVG.
pub fn svg(project: &Project, options: &VectorOptions, resources: &mut dyn Resources) -> Result<Exported<String>, String> {
    let scene = scene::build(project, options, resources)?;
    let mut writer = Writer { scene: &scene, style: String::new(), defs: String::new(), next_id: 0, add_blend: false };
    let body = writer.comp(&scene.root, 0.0);
    let mut notes = scene.notes.clone();
    if writer.add_blend {
        notes.add_general("The Add blend mode was written as plus-lighter, which some SVG viewers don’t support");
    }
    let (width, height) = (scene.root.size.x.round() as i64, scene.root.size.y.round() as i64);
    let mut file = String::new();
    let _ = write!(file, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">");
    let _ = write!(file, "<title>{}</title>", escape(&scene.root.name));
    // SVG transforms turn about the user space's origin; the same is said for the CSS ones.
    let _ = write!(file, "<style>g{{transform-box:view-box;transform-origin:0 0}}{}</style>", writer.style);
    if !writer.defs.is_empty() {
        let _ = write!(file, "<defs>{}</defs>", writer.defs);
    }
    file.push_str(&body);
    file.push_str("</svg>\n");
    Ok(Exported { file, left_out: notes.into_lines() })
}

/// A number with at most three decimals, without trailing zeros ("12", "0.5", "-3.142").
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let text = format!("{:.3}", (value * 1000.0).round() / 1000.0);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" { "0".into() } else { text.to_owned() }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn hex(rgba: Rgba) -> String {
    rgba.to_hex().to_lowercase()
}

/// A CSS timing function for a keyframe's segment.
fn timing(ease: Ease) -> String {
    match ease {
        Ease::Linear => "linear".into(),
        Ease::Hold => "steps(1,end)".into(),
        Ease::Curve(curve) => format!("cubic-bezier({},{},{},{})", num(f64::from(curve.x1)), num(f64::from(curve.y1)), num(f64::from(curve.x2)), num(f64::from(curve.y2))),
    }
}

/// One part of a transform: how it reads as an SVG attribute (constant) or a CSS value
/// (keyframes), and the value at which it does nothing.
#[derive(Clone, Copy)]
enum Part {
    Position,
    Rotation,
    Scale,
    Anchor,
}

impl Part {
    fn attribute(self, v: &[f64]) -> String {
        match self {
            Self::Position => format!("translate({} {})", num(v[0]), num(v[1])),
            Self::Rotation => format!("rotate({})", num(v[0])),
            Self::Scale => format!("scale({} {})", num(v[0] / 100.0), num(v[1] / 100.0)),
            Self::Anchor => format!("translate({} {})", num(-v[0]), num(-v[1])),
        }
    }

    fn css(self, v: &[f64]) -> String {
        match self {
            Self::Position => format!("transform:translate({}px,{}px)", num(v[0]), num(v[1])),
            Self::Rotation => format!("transform:rotate({}deg)", num(v[0])),
            Self::Scale => format!("transform:scale({},{})", num(v[0] / 100.0), num(v[1] / 100.0)),
            Self::Anchor => format!("transform:translate({}px,{}px)", num(-v[0]), num(-v[1])),
        }
    }

    fn does_nothing(self, v: &[f64]) -> bool {
        match self {
            Self::Position | Self::Anchor => v[0].abs() < 1e-6 && v[1].abs() < 1e-6,
            Self::Rotation => v[0].abs() < 1e-6,
            Self::Scale => (v[0] - 100.0).abs() < 1e-6 && (v[1] - 100.0).abs() < 1e-6,
        }
    }
}

struct Writer<'a> {
    scene: &'a Scene,
    style: String,
    defs: String,
    next_id: usize,
    add_blend: bool,
}

impl Writer<'_> {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// The animation's length in output frames, and in seconds for CSS.
    fn length(&self) -> f64 {
        self.scene.root.length
    }

    /// A composition's layers, bottom first. `shift` is when (in output frames) its frame 0
    /// plays: 0 for the exported composition, later for a nested one.
    fn comp(&mut self, comp: &Comp, shift: f64) -> String {
        let mut out = String::new();
        for index in (0..comp.layers.len()).rev() {
            let layer = &comp.layers[index];
            if layer.is_matte || layer.content == Content::Null {
                continue;
            }
            out.push_str(&self.layer(comp, index, shift, true));
        }
        out
    }

    /// One layer, with its time on screen, opacity, blend mode and matte (the layer above,
    /// when `with_matte`), its parents' transforms and its own, around its content.
    fn layer(&mut self, comp: &Comp, index: usize, shift: f64, with_matte: bool) -> String {
        let layer = &comp.layers[index];
        let (start, end) = (layer.span.0 + shift, layer.span.1 + shift);
        let length = self.length();
        if end <= 0.0 || start >= length || end <= start {
            return String::new();
        }
        let content = self.content(layer, shift);
        if content.is_empty() {
            return String::new();
        }
        // Parents' transforms around the layer's own, the topmost parent outermost.
        let mut placed = self.transformed(&layer.transform, shift, &content, false);
        let mut parent = layer.parent;
        let mut guard = 0;
        while let Some(parent_index) = parent {
            let Some(parent_layer) = comp.layers.get(parent_index) else { break };
            placed = self.transformed(&parent_layer.transform, shift, &placed, false);
            parent = parent_layer.parent;
            guard += 1;
            if guard > comp.layers.len() {
                break;
            }
        }
        // The layer's opacity, mask and blend mode, on a group in the composition's own space.
        let mut attributes = String::new();
        let mut animations = Vec::new();
        match layer.transform.opacity.shifted(shift).clipped(0.0, length) {
            Track::Static(value) if value[0] >= 99.999 => {}
            Track::Static(value) => {
                let _ = write!(attributes, " opacity=\"{}\"", num(value[0] / 100.0));
            }
            Track::Keys(keys) => animations.push(self.keyframes(&keys, |v| format!("opacity:{}", num(v[0] / 100.0)))),
        }
        if let Some(blend) = css_blend(layer.blend) {
            self.add_blend |= layer.blend == BlendMode::Add;
            let _ = write!(attributes, " style=\"mix-blend-mode:{blend}\"");
        }
        if let (true, Some(kind), Some(above)) = (with_matte, layer.matte, index.checked_sub(1)) {
            let matte = self.layer(comp, above, shift, false);
            let mask = self.mask(kind, &matte, comp.size);
            let _ = write!(attributes, " mask=\"url(#{mask})\"");
        }
        let class = self.class(&animations);
        let mut out = format!("<g{class}{attributes}>{placed}</g>");
        // The time on screen, on a group of its own so nested layers can't show past it.
        if start > 0.0 || end < length {
            let mut frames = Vec::new();
            let visible = |shown: bool| format!("opacity:{}", if shown { 1 } else { 0 });
            frames.push((0.0, visible(start <= 0.0), Ease::Hold));
            if start > 0.0 {
                frames.push((start, visible(true), Ease::Hold));
            }
            if end < length {
                frames.push((end, visible(false), Ease::Hold));
            }
            let name = self.keyframe_rule(&frames);
            let class = self.class(&[name]);
            out = format!("<g{class}>{out}</g>");
        }
        out
    }

    /// `inner` inside groups that apply `transform`'s position, rotation, scale and anchor
    /// point (outermost first). Runs of constant parts share one group. With `with_opacity`,
    /// the transform's opacity is applied too (for groups inside a layer).
    fn transformed(&mut self, transform: &Transform, shift: f64, inner: &str, with_opacity: bool) -> String {
        let length = self.length();
        let parts = [(Part::Position, &transform.position), (Part::Rotation, &transform.rotation), (Part::Scale, &transform.scale), (Part::Anchor, &transform.anchor)];
        // Built from the inside out.
        let mut out = inner.to_owned();
        if with_opacity {
            match transform.opacity.shifted(shift).clipped(0.0, length) {
                Track::Static(value) if value[0] >= 99.999 => {}
                Track::Static(value) => out = format!("<g opacity=\"{}\">{out}</g>", num(value[0] / 100.0)),
                Track::Keys(keys) => {
                    let name = self.keyframes(&keys, |v| format!("opacity:{}", num(v[0] / 100.0)));
                    let class = self.class(&[name]);
                    out = format!("<g{class}>{out}</g>");
                }
            }
        }
        let mut constant: Vec<String> = Vec::new();
        for (part, track) in parts.iter().rev() {
            match track.shifted(shift).clipped(0.0, length) {
                Track::Static(value) => {
                    if !part.does_nothing(&value) {
                        constant.insert(0, part.attribute(&value));
                    }
                }
                Track::Keys(keys) => {
                    if !constant.is_empty() {
                        out = format!("<g transform=\"{}\">{out}</g>", constant.join(" "));
                        constant.clear();
                    }
                    let part = *part;
                    let name = self.keyframes(&keys, move |v| part.css(v));
                    let class = self.class(&[name]);
                    out = format!("<g{class}>{out}</g>");
                }
            }
        }
        if !constant.is_empty() {
            out = format!("<g transform=\"{}\">{out}</g>", constant.join(" "));
        }
        out
    }

    /// A class running the keyframe rules `names` over the animation's length, looping.
    fn class(&mut self, names: &[String]) -> String {
        if names.is_empty() {
            return String::new();
        }
        let class = self.id("a");
        let seconds = num(self.scene.seconds());
        let runs: Vec<String> = names.iter().map(|name| format!("{name} {seconds}s linear infinite both")).collect();
        let _ = write!(self.style, ".{class}{{animation:{}}}", runs.join(","));
        format!(" class=\"{class}\"")
    }

    /// A keyframe rule for keyframes already clipped to the animation (see `Track::clipped`).
    fn keyframes(&mut self, keys: &[Key], css: impl Fn(&[f64]) -> String) -> String {
        let length = self.length();
        let mut frames = Vec::new();
        if keys[0].time > 0.0 {
            frames.push((0.0, css(&keys[0].value), Ease::Linear));
        }
        for key in keys {
            frames.push((key.time, css(&key.value), key.ease));
        }
        if let Some(last) = keys.last()
            && last.time < length
        {
            frames.push((length, css(&last.value), Ease::Linear));
        }
        self.keyframe_rule(&frames)
    }

    /// An `@keyframes` rule from (output frame, declarations, easing to the next) triples.
    fn keyframe_rule(&mut self, frames: &[(f64, String, Ease)]) -> String {
        let name = self.id("k");
        let length = self.length().max(1e-9);
        let _ = write!(self.style, "@keyframes {name}{{");
        for (time, declarations, ease) in frames {
            let percent = (time / length * 100.0).clamp(0.0, 100.0);
            let percent = format!("{:.4}", percent);
            let percent = percent.trim_end_matches('0').trim_end_matches('.');
            let _ = write!(self.style, "{percent}%{{{declarations};animation-timing-function:{}}}", timing(*ease));
        }
        self.style.push('}');
        name
    }

    /// A mask made from `matte` (a layer's markup in the same space), as `kind` says. Returns
    /// its id. Alpha mattes are made white so only their opacity counts; inverted ones are
    /// flipped with a filter over the whole area.
    fn mask(&mut self, kind: Matte, matte: &str, area: Vec2) -> String {
        let (w, h) = (f64::from(area.x.max(1.0)), f64::from(area.y.max(1.0)));
        let region = format!("x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"", num(-w), num(-h), num(w * 3.0), num(h * 3.0));
        let matrix = match kind {
            Matte::Alpha => Some("<feColorMatrix type=\"matrix\" values=\"0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 1 0\"/>"),
            Matte::AlphaInverted => Some("<feColorMatrix type=\"matrix\" values=\"0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 -1 1\"/>"),
            Matte::Luma => None,
            // Laid over black first, so empty areas count as dark, then inverted.
            Matte::LumaInverted => Some("<feFlood flood-color=\"#000\" result=\"black\"/><feComposite in=\"SourceGraphic\" in2=\"black\" operator=\"over\"/><feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 -1 0 0 1 0 0 -1 0 1 0 0 0 0 1\"/>"),
        };
        let content = match matrix {
            Some(primitives) => {
                let filter = self.id("f");
                let _ = write!(self.defs, "<filter id=\"{filter}\" filterUnits=\"userSpaceOnUse\" {region} color-interpolation-filters=\"sRGB\">{primitives}</filter>");
                format!("<g filter=\"url(#{filter})\">{matte}</g>")
            }
            None => matte.to_owned(),
        };
        let id = self.id("m");
        let _ = write!(self.defs, "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" {region}>{content}</mask>");
        id
    }

    /// What a layer draws, in its own pixels.
    fn content(&mut self, layer: &SceneLayer, shift: f64) -> String {
        match &layer.content {
            Content::Null => String::new(),
            Content::Solid { color, size } => format!("<rect width=\"{}\" height=\"{}\"{}/>", num(f64::from(size.x)), num(f64::from(size.y)), fill_attributes(*color)),
            Content::Shapes(groups) => groups.iter().map(|group| self.group(group, shift)).collect(),
            Content::Image { item, size, native } => {
                let Some(image) = self.scene.image(*item) else { return String::new() };
                let id = format!("i{}", item.0);
                if !self.defs.contains(&format!("id=\"{id}\"")) {
                    let _ = write!(self.defs, "<image id=\"{id}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"{}\"/>", image.width, image.height, data_url(image.mime, &image.bytes));
                }
                let stretch = (f64::from(size.x / native.x.max(1e-3)), f64::from(size.y / native.y.max(1e-3)));
                if (stretch.0 - 1.0).abs() < 1e-6 && (stretch.1 - 1.0).abs() < 1e-6 {
                    format!("<use href=\"#{id}\"/>")
                } else {
                    format!("<use href=\"#{id}\" transform=\"scale({} {})\"/>", num(stretch.0), num(stretch.1))
                }
            }
            Content::Precomp { comp, size, native, start } => {
                let Some(inner) = self.scene.precomp(*comp) else { return String::new() };
                let body = self.comp(inner, shift + start);
                if body.is_empty() {
                    return String::new();
                }
                let clip = self.id("c");
                let _ = write!(self.defs, "<clipPath id=\"{clip}\"><rect width=\"{}\" height=\"{}\"/></clipPath>", num(f64::from(native.x)), num(f64::from(native.y)));
                let clipped = format!("<g clip-path=\"url(#{clip})\">{body}</g>");
                let stretch = (f64::from(size.x / native.x.max(1e-3)), f64::from(size.y / native.y.max(1e-3)));
                if (stretch.0 - 1.0).abs() < 1e-6 && (stretch.1 - 1.0).abs() < 1e-6 { clipped } else { format!("<g transform=\"scale({} {})\">{clipped}</g>", num(stretch.0), num(stretch.1)) }
            }
            Content::Text(text) => live_text(text),
        }
    }

    /// A group of shapes: its outlines as one element with its paints, or its child groups,
    /// inside its own transform.
    fn group(&mut self, group: &Group, shift: f64) -> String {
        let inner = if group.children.is_empty() {
            self.shapes(&group.shapes, &group.paints)
        } else {
            // The first child is drawn on top, as in the scene; SVG draws later elements on top.
            group.children.iter().rev().map(|child| self.group(child, shift)).collect()
        };
        if inner.is_empty() {
            return inner;
        }
        match &group.transform {
            Some(transform) => self.transformed(transform, shift, &inner, true),
            None => inner,
        }
    }

    /// Outlines drawn with paints (bottom first): one element, or one per paint when there are
    /// more than a fill and a stroke.
    fn shapes(&mut self, shapes: &[Geometry], paints: &[PaintOp]) -> String {
        if shapes.is_empty() || paints.is_empty() {
            return String::new();
        }
        let element = |attributes: &str| -> String {
            match shapes {
                [Geometry::Rect { center, size, radius }] => {
                    let corner = if *radius > 0.0 { format!(" rx=\"{}\"", num(f64::from(*radius))) } else { String::new() };
                    format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{corner}{attributes}/>",
                        num(f64::from(center.x - size.x * 0.5)),
                        num(f64::from(center.y - size.y * 0.5)),
                        num(f64::from(size.x)),
                        num(f64::from(size.y))
                    )
                }
                [Geometry::Ellipse { center, size }] => format!(
                    "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{attributes}/>",
                    num(f64::from(center.x)),
                    num(f64::from(center.y)),
                    num(f64::from(size.x * 0.5)),
                    num(f64::from(size.y * 0.5))
                ),
                _ => format!("<path d=\"{}\"{attributes}/>", path_data(shapes)),
            }
        };
        match paints {
            [only] => {
                let attributes = self.paint_attributes(only, true);
                element(&attributes)
            }
            [bottom, top] if matches!(bottom, PaintOp::Fill { .. }) != matches!(top, PaintOp::Fill { .. }) => {
                // One element can hold a fill and a stroke; `paint-order` says which is on top.
                let mut attributes = self.paint_attributes(bottom, false);
                attributes.push_str(&self.paint_attributes(top, false));
                if matches!(bottom, PaintOp::Stroke { .. }) {
                    attributes.push_str(" paint-order=\"stroke\"");
                }
                element(&attributes)
            }
            _ => paints.iter().map(|paint| element(&self.paint_attributes(paint, true))).collect(),
        }
    }

    /// The attributes for one paint. With `alone` (the element has no other paint), the other
    /// kind is turned off, since SVG fills shapes black by default.
    fn paint_attributes(&mut self, paint: &PaintOp, alone: bool) -> String {
        match paint {
            PaintOp::Fill { paint, area } => {
                let mut attributes = match paint {
                    Paint::Solid(rgba) => fill_attributes(*rgba),
                    _ => format!(" fill=\"url(#{})\"", self.gradient(*paint, *area)),
                };
                if alone {
                    attributes.push_str(" stroke=\"none\"");
                }
                attributes
            }
            PaintOp::Stroke { paint, area, width, line } => {
                let mut attributes = match paint {
                    Paint::Solid(rgba) => {
                        let opacity = if rgba.a < 0.999 { format!(" stroke-opacity=\"{}\"", num(f64::from(rgba.a))) } else { String::new() };
                        format!(" stroke=\"{}\"{opacity}", hex(*rgba))
                    }
                    _ => format!(" stroke=\"url(#{})\"", self.gradient(*paint, *area)),
                };
                let _ = write!(
                    attributes,
                    " stroke-width=\"{}\" stroke-linejoin=\"{}\" stroke-linecap=\"{}\" stroke-miterlimit=\"{}\"",
                    num(f64::from(*width)),
                    line_join(line.join),
                    line_cap(line.cap),
                    num(f64::from(line.miter_limit))
                );
                if let Some((dash, gap)) = line.dash {
                    let _ = write!(attributes, " stroke-dasharray=\"{} {}\"", num(f64::from(dash)), num(f64::from(gap)));
                }
                if alone {
                    attributes.insert_str(0, " fill=\"none\"");
                }
                attributes
            }
        }
    }

    /// A gradient definition for `paint` over a box of `area`; returns its id. A radial
    /// gradient is laid out in fractions of the box and stretched with it, as Studio draws it.
    fn gradient(&mut self, paint: Paint, area: Vec2) -> String {
        let gradient = paint.gradient().expect("only gradients come here").sanitized();
        let id = self.id("g");
        let stops: String = gradient
            .stops()
            .iter()
            .map(|stop| {
                let opacity = if stop.color.a < 0.999 { format!(" stop-opacity=\"{}\"", num(f64::from(stop.color.a))) } else { String::new() };
                format!("<stop offset=\"{}\" stop-color=\"{}\"{opacity}/>", num(f64::from(stop.at)), hex(stop.color))
            })
            .collect();
        let (from, to) = (gradient.from, gradient.to);
        if matches!(paint, Paint::Radial(_)) {
            let radius = f64::from((to.x - from.x).hypot(to.y - from.y)).max(1e-4);
            let _ = write!(
                self.defs,
                "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\" gradientTransform=\"scale({} {})\">{stops}</radialGradient>",
                num(f64::from(from.x)),
                num(f64::from(from.y)),
                num(radius),
                num(f64::from(area.x)),
                num(f64::from(area.y))
            );
        } else {
            let _ = write!(
                self.defs,
                "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">{stops}</linearGradient>",
                num(f64::from(from.x * area.x)),
                num(f64::from(from.y * area.y)),
                num(f64::from(to.x * area.x)),
                num(f64::from(to.y * area.y))
            );
        }
        id
    }
}

/// A solid fill as attributes.
fn fill_attributes(rgba: Rgba) -> String {
    let opacity = if rgba.a < 0.999 { format!(" fill-opacity=\"{}\"", num(f64::from(rgba.a))) } else { String::new() };
    format!(" fill=\"{}\"{opacity}", hex(rgba))
}

/// The CSS blend mode for a layer's, or `None` for normal.
fn css_blend(mode: BlendMode) -> Option<&'static str> {
    match mode {
        BlendMode::Normal => None,
        BlendMode::Add => Some("plus-lighter"),
        BlendMode::Multiply => Some("multiply"),
        BlendMode::Screen => Some("screen"),
        BlendMode::Overlay => Some("overlay"),
        BlendMode::SoftLight => Some("soft-light"),
        BlendMode::HardLight => Some("hard-light"),
        BlendMode::Difference => Some("difference"),
    }
}

/// Path data (`d`) for outlines, all in one.
fn path_data(shapes: &[Geometry]) -> String {
    let mut d = String::new();
    let p = |v: Vec2| format!("{} {}", num(f64::from(v.x)), num(f64::from(v.y)));
    for shape in shapes {
        match shape {
            Geometry::Rect { center, size, radius } => {
                let (left, top) = (center.x - size.x * 0.5, center.y - size.y * 0.5);
                let r = radius.min(size.x * 0.5).min(size.y * 0.5);
                let _ = write!(d, "M{}H{}", p(Vec2::new(left + r, top)), num(f64::from(left + size.x - r)));
                if r > 0.0 {
                    let _ = write!(d, "A{r} {r} 0 0 1 {}", p(Vec2::new(left + size.x, top + r)), r = num(f64::from(r)));
                }
                let _ = write!(d, "V{}", num(f64::from(top + size.y - r)));
                if r > 0.0 {
                    let _ = write!(d, "A{r} {r} 0 0 1 {}", p(Vec2::new(left + size.x - r, top + size.y)), r = num(f64::from(r)));
                }
                let _ = write!(d, "H{}", num(f64::from(left + r)));
                if r > 0.0 {
                    let _ = write!(d, "A{r} {r} 0 0 1 {}", p(Vec2::new(left, top + size.y - r)), r = num(f64::from(r)));
                }
                let _ = write!(d, "V{}", num(f64::from(top + r)));
                if r > 0.0 {
                    let _ = write!(d, "A{r} {r} 0 0 1 {}", p(Vec2::new(left + r, top)), r = num(f64::from(r)));
                }
                d.push('Z');
            }
            Geometry::Ellipse { center, size } => {
                let (rx, ry) = (size.x * 0.5, size.y * 0.5);
                let _ = write!(
                    d,
                    "M{}A{rx} {ry} 0 1 1 {}A{rx} {ry} 0 1 1 {}Z",
                    p(Vec2::new(center.x - rx, center.y)),
                    p(Vec2::new(center.x + rx, center.y)),
                    p(Vec2::new(center.x - rx, center.y)),
                    rx = num(f64::from(rx)),
                    ry = num(f64::from(ry))
                );
            }
            Geometry::Path(path) => {
                let Some(first) = path.points.first() else { continue };
                let _ = write!(d, "M{}", p(first.at));
                for [_, control_a, control_b, end] in path.segments() {
                    let _ = write!(d, "C{} {} {}", p(control_a), p(control_b), p(end));
                }
                if path.closed {
                    d.push('Z');
                }
            }
        }
    }
    d
}

/// SVG's name for a stroke's corners.
fn line_join(join: StrokeJoin) -> &'static str {
    match join {
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
        StrokeJoin::Bevel => "bevel",
    }
}

/// SVG's name for a stroke's ends.
fn line_cap(cap: StrokeCap) -> &'static str {
    match cap {
        StrokeCap::Butt => "butt",
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
    }
}

/// Live text, when no outlines were available: lines as `<tspan>`s, placed from an estimated
/// baseline, in the player's copy of the font.
fn live_text(text: &LiveText) -> String {
    let style = &text.style;
    let (anchor, x) = match style.align {
        TextAlign::Left => ("start", 0.0),
        TextAlign::Center => ("middle", text.size.x * 0.5),
        TextAlign::Right => ("end", text.size.x),
    };
    let stroke = style.stroke.filter(|(_, width)| *width > 0.0);
    if style.fill.is_none() && stroke.is_none() {
        return String::new();
    }
    // Without a fill the letters are hollow, as Studio draws them. As `scene::text_paints`
    // writes outlined text: an outside stroke under a fill is doubled under it; others are
    // centered, over the fill when set so.
    let mut attributes = style.fill.map_or_else(|| " fill=\"none\"".to_owned(), fill_attributes);
    if let Some((rgba, width)) = stroke {
        let options = style.stroke_options;
        let outside_under_fill = style.fill.is_some() && options.position == StrokePosition::Outside;
        let width = f64::from(width) * if outside_under_fill { 2.0 } else { 1.0 };
        let _ = write!(
            attributes,
            " stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"{}\" stroke-linecap=\"{}\" stroke-miterlimit=\"{}\"",
            hex(rgba),
            num(width),
            line_join(options.join),
            line_cap(options.cap),
            num(f64::from(options.miter_limit))
        );
        if options.dashed() {
            let _ = write!(attributes, " stroke-dasharray=\"{} {}\"", num(f64::from(options.dash)), num(f64::from(options.gap)));
        }
        if !(options.over_fill && !outside_under_fill) {
            attributes.push_str(" paint-order=\"stroke\"");
        }
    }
    let lines: String = text
        .text
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            let baseline = style.line_height * (index as f32 + 0.5) + style.font_size * 0.32;
            format!("<tspan x=\"{}\" y=\"{}\">{}</tspan>", num(f64::from(x)), num(f64::from(baseline)), escape(line))
        })
        .collect();
    format!(
        "<text xml:space=\"preserve\" text-anchor=\"{anchor}\" font-family=\"{}\" font-weight=\"{}\" font-size=\"{}\" letter-spacing=\"{}\"{attributes}>{lines}</text>",
        escape(&style.font_family),
        style.font_weight,
        num(f64::from(style.font_size)),
        num(f64::from(style.tracking / 1000.0 * style.font_size))
    )
}
