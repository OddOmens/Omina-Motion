//! The composition as both vector formats see it, worked out once so the Lottie and SVG
//! writers stay simple.
//!
//! Times here are in output frames (the animation's own frame rate), with the first exported
//! frame at 0. Animated values are `Track`s of keyframes whose easings are ones both formats
//! have (linear, hold and cubic Bézier); Studio's elastic and bounce easings, which no single
//! curve can draw, are sampled into a keyframe per frame. Text animators are worked out the
//! same way, piece by piece. Layers are listed top first, and a layer's matte is always the
//! layer listed directly above it, as both formats want.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use studio_core::animation::{Animated, CubicBezier, Easing, Lerp, Vec2};
use studio_core::model::{BlendMode, Composition, Id, Layer, LayerContent, MaskMode, Paint, Project, Rgba, ShapeKind, TextStyle, TrackMatte};
use studio_core::text_animation::{self, TextMotion, TextUnit, UnitPose};
use studio_core::time::FrameRate;

use crate::geometry::{self, Geometry};
use crate::notes::{Notes, quoted};
use crate::{EmbeddedImage, Resources, TextOutline, VectorOptions};

/// How deeply compositions may nest, as in the renderer.
const MAX_NESTING: usize = 8;

/// How a keyframe's segment moves to the next keyframe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ease {
    Linear,
    /// Keeps this keyframe's value until the next one.
    Hold,
    Curve(CubicBezier),
}

impl Ease {
    /// Eased progress (0 to 1, curves may overshoot) for linear progress `t`.
    pub fn apply(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::Hold => 0.0,
            Self::Curve(curve) => f64::from(curve.ease(t as f32)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Key {
    pub time: f64,
    pub value: Vec<f64>,
    /// The easing of the segment that leaves this keyframe (unused on the last).
    pub ease: Ease,
}

/// One animated (or constant) value: a number, or a few for a point or a scale.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Track {
    Static(Vec<f64>),
    /// Sorted by time, at least two.
    Keys(Vec<Key>),
}

impl Track {
    /// A track from keyframes, constant when they never change.
    pub fn from_keys(mut keys: Vec<Key>) -> Self {
        // Drop keyframes in the middle of a run of the same value: they change nothing.
        let mut index = 1;
        while index + 1 < keys.len() {
            if keys[index].value == keys[index - 1].value && keys[index].value == keys[index + 1].value && keys[index - 1].ease == keys[index].ease {
                keys.remove(index);
            } else {
                index += 1;
            }
        }
        match keys.len() {
            0 => Self::Static(vec![0.0]),
            1 => Self::Static(keys.remove(0).value),
            _ if keys.windows(2).all(|pair| pair[0].value == pair[1].value) => Self::Static(keys.remove(0).value),
            _ => Self::Keys(keys),
        }
    }

    /// The value at `time`: held before the first keyframe and after the last.
    pub fn value_at(&self, time: f64) -> Vec<f64> {
        let keys = match self {
            Self::Static(value) => return value.clone(),
            Self::Keys(keys) => keys,
        };
        if time <= keys[0].time {
            return keys[0].value.clone();
        }
        let Some(pair) = keys.windows(2).find(|pair| time < pair[1].time) else {
            return keys[keys.len() - 1].value.clone();
        };
        let (from, to) = (&pair[0], &pair[1]);
        let progress = from.ease.apply((time - from.time) / (to.time - from.time).max(1e-9));
        from.value.iter().zip(&to.value).map(|(a, b)| a + (b - a) * progress).collect()
    }

    /// The same track with every value mapped by `f`.
    pub fn map(&self, f: impl Fn(&[f64]) -> Vec<f64>) -> Self {
        match self {
            Self::Static(value) => Self::Static(f(value)),
            Self::Keys(keys) => Self::Keys(keys.iter().map(|key| Key { value: f(&key.value), ..key.clone() }).collect()),
        }
    }

    /// The same track `by` frames later.
    pub fn shifted(&self, by: f64) -> Self {
        match self {
            Self::Static(_) => self.clone(),
            Self::Keys(keys) => Self::Keys(keys.iter().map(|key| Key { time: key.time + by, ..key.clone() }).collect()),
        }
    }

    /// Only the part between `start` and `end`, with keyframes added at the cuts so it moves
    /// exactly as before (a curve cut partway is replaced by the matching part of itself).
    pub fn clipped(&self, start: f64, end: f64) -> Self {
        let Self::Keys(keys) = self else { return self.clone() };
        let mut out: Vec<Key> = Vec::new();
        for pair in keys.windows(2) {
            let (from, to) = (&pair[0], &pair[1]);
            let (cut_start, cut_end) = (from.time.max(start), to.time.min(end));
            if cut_end <= cut_start {
                continue;
            }
            let span = (to.time - from.time).max(1e-9);
            let ease = sub_ease(from.ease, (cut_start - from.time) / span, (cut_end - from.time) / span);
            match out.last_mut() {
                Some(last) if (last.time - cut_start).abs() < 1e-9 => last.ease = ease,
                _ => out.push(Key { time: cut_start, value: self.value_at(cut_start), ease }),
            }
            // The value where the segment is cut off: a held segment keeps its first value.
            let value = if from.ease == Ease::Hold && cut_end < to.time { from.value.clone() } else { self.value_at(cut_end) };
            out.push(Key { time: cut_end, value, ease: Ease::Linear });
        }
        if out.len() < 2 {
            return Self::Static(self.value_at(start));
        }
        Self::from_keys(out)
    }
}

/// The part of `ease` between linear progress `from` and `to`, rescaled to run 0 to 1 itself.
fn sub_ease(ease: Ease, from: f64, to: f64) -> Ease {
    let Ease::Curve(curve) = ease else { return ease };
    if from <= 1e-9 && to >= 1.0 - 1e-9 {
        return ease;
    }
    let points = [(0.0, 0.0), (f64::from(curve.x1.clamp(0.0, 1.0)), f64::from(curve.y1)), (f64::from(curve.x2.clamp(0.0, 1.0)), f64::from(curve.y2)), (1.0, 1.0)];
    let (s0, s1) = (parameter_for_x(&points, from), parameter_for_x(&points, to));
    let piece = bezier_piece(&points, s0, s1);
    let (first, last) = (piece[0], piece[3]);
    let (width, height) = (last.0 - first.0, last.1 - first.1);
    if width.abs() < 1e-9 || height.abs() < 1e-9 {
        return Ease::Linear;
    }
    let normal = |(x, y): (f64, f64)| (((x - first.0) / width) as f32, ((y - first.1) / height) as f32);
    let ((x1, y1), (x2, y2)) = (normal(piece[1]), normal(piece[2]));
    Ease::Curve(CubicBezier::new(x1, y1, x2, y2))
}

/// The point at parameter `s` on a cubic Bézier.
fn bezier_at(points: &[(f64, f64); 4], s: f64) -> (f64, f64) {
    let u = 1.0 - s;
    let blend = |index: usize| [u * u * u, 3.0 * u * u * s, 3.0 * u * s * s, s * s * s][index];
    (0..4).fold((0.0, 0.0), |(x, y), index| (x + points[index].0 * blend(index), y + points[index].1 * blend(index)))
}

/// The curve parameter where a timing curve's x reaches `x` (x always rises along it).
fn parameter_for_x(points: &[(f64, f64); 4], x: f64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..60 {
        let middle = (low + high) * 0.5;
        if bezier_at(points, middle).0 < x { low = middle } else { high = middle }
    }
    (low + high) * 0.5
}

/// The control points of the part of a cubic Bézier between parameters `s0` and `s1`.
fn bezier_piece(points: &[(f64, f64); 4], s0: f64, s1: f64) -> [(f64, f64); 4] {
    // The derivative gives the handles: each end's handle is a third of the tangent there,
    // scaled to the length of the piece.
    let derivative = |s: f64| {
        let u = 1.0 - s;
        let d = |a: f64, b: f64, c: f64, e: f64| 3.0 * (u * u * (b - a) + 2.0 * u * s * (c - b) + s * s * (e - c));
        (d(points[0].0, points[1].0, points[2].0, points[3].0), d(points[0].1, points[1].1, points[2].1, points[3].1))
    };
    let (start, end) = (bezier_at(points, s0), bezier_at(points, s1));
    let scale = (s1 - s0) / 3.0;
    let (d0, d1) = (derivative(s0), derivative(s1));
    [start, (start.0 + d0.0 * scale, start.1 + d0.1 * scale), (end.0 - d1.0 * scale, end.1 - d1.1 * scale), end]
}

/// A layer's (or a group's) placement: anchor point, position, scale (percent), rotation
/// (degrees clockwise) and opacity (percent), as in Studio, Lottie and After Effects.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Transform {
    pub anchor: Track,
    pub position: Track,
    pub scale: Track,
    pub rotation: Track,
    pub opacity: Track,
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            anchor: Track::Static(vec![0.0, 0.0]),
            position: Track::Static(vec![0.0, 0.0]),
            scale: Track::Static(vec![100.0, 100.0]),
            rotation: Track::Static(vec![0.0]),
            opacity: Track::Static(vec![100.0]),
        }
    }

    /// A transform that only stretches, by `factor` (1 is full size).
    pub fn scaling(factor: Vec2) -> Self {
        Self { scale: Track::Static(vec![f64::from(factor.x) * 100.0, f64::from(factor.y) * 100.0]), ..Self::identity() }
    }

    /// The same placement for content drawn `factor` times larger than the layer's box (an
    /// image or nested composition whose own size differs from the layer's): the content's
    /// pixels are stretched into the box by scaling, and the anchor moves to match.
    pub fn for_content_scaled(&self, factor: Vec2) -> Self {
        let (fx, fy) = (f64::from(factor.x), f64::from(factor.y));
        Self {
            anchor: self.anchor.map(|v| vec![v[0] / fx.max(1e-9), v[1] / fy.max(1e-9)]),
            scale: self.scale.map(|v| vec![v[0] * fx, v[1] * fy]),
            ..self.clone()
        }
    }
}

/// How a layer is cut out by the layer above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Matte {
    Alpha,
    AlphaInverted,
    Luma,
    LumaInverted,
}

/// A fill or stroke. `area` is the box gradient points are fractions of.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaintOp {
    Fill { paint: Paint, area: Vec2 },
    Stroke { paint: Paint, area: Vec2, width: f32, round_joins: bool, round_caps: bool },
}

/// Outlines drawn with paints, and groups inside it. A group has either outlines and paints
/// or child groups, never both, so a paint never reaches outlines in another group.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Group {
    pub name: String,
    pub transform: Option<Transform>,
    pub shapes: Vec<Geometry>,
    /// Painted in order: the first is at the bottom.
    pub paints: Vec<PaintOp>,
    pub children: Vec<Group>,
}

/// Text kept as text (when no font outlines were available).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LiveText {
    pub text: String,
    pub style: TextStyle,
    pub size: Vec2,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Content {
    /// Draws nothing; carries a transform for layers parented to it.
    Null,
    Solid { color: Rgba, size: Vec2 },
    Shapes(Vec<Group>),
    /// An embedded picture (see `Scene::images`) stretched over a box of `size`.
    Image { item: Id, size: Vec2, native: Vec2 },
    /// A nested composition (see `Scene::precomps`) whose frame 0 plays at `start`.
    Precomp { comp: Id, size: Vec2, native: Vec2, start: f64 },
    Text(LiveText),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneLayer {
    pub name: String,
    /// When it is on screen: from the first frame to the frame after the last.
    pub span: (f64, f64),
    /// The index (in the same composition) of the layer it is parented to.
    pub parent: Option<usize>,
    pub transform: Transform,
    pub blend: BlendMode,
    /// Used as the matte of the layer below it, and not drawn itself.
    pub is_matte: bool,
    /// Cut out by the layer directly above (which has `is_matte`).
    pub matte: Option<Matte>,
    pub content: Content,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Comp {
    pub name: String,
    pub size: Vec2,
    /// How long it runs, in output frames.
    pub length: f64,
    /// Top first.
    pub layers: Vec<SceneLayer>,
}

/// Everything a writer needs.
pub(crate) struct Scene {
    /// Output frames per second.
    pub fps: f64,
    pub root: Comp,
    /// Nested compositions, each built once however often it is used.
    pub precomps: Vec<(Id, Comp)>,
    pub images: Vec<(Id, EmbeddedImage)>,
    /// Timeline markers: when, and their labels.
    pub markers: Vec<(f64, String)>,
    pub notes: Notes,
}

impl Scene {
    pub fn precomp(&self, id: Id) -> Option<&Comp> {
        self.precomps.iter().find(|(known, _)| *known == id).map(|(_, comp)| comp)
    }

    pub fn image(&self, id: Id) -> Option<&EmbeddedImage> {
        self.images.iter().find(|(known, _)| *known == id).map(|(_, image)| image)
    }

    /// The animation's length in seconds.
    pub fn seconds(&self) -> f64 {
        self.root.length / self.fps
    }
}

/// Converts one composition's frames to output frames: `zero` is where its frame 0 falls, and
/// `ratio` is output frames per composition frame.
#[derive(Clone, Copy, Debug)]
struct Clock {
    zero: f64,
    ratio: f64,
    rate: FrameRate,
}

impl Clock {
    fn out(self, frame: f64) -> f64 {
        self.zero + frame * self.ratio
    }

    fn source(self, time: f64) -> f64 {
        (time - self.zero) / self.ratio
    }
}

/// Works out the scene for `options`.
pub(crate) fn build(project: &Project, options: &VectorOptions, resources: &mut dyn Resources) -> Result<Scene, String> {
    let composition = project.composition(options.composition).ok_or("The composition no longer exists.")?;
    let frames = options.frames.start.max(0)..options.frames.end.min(composition.duration_frames);
    if frames.is_empty() {
        return Err("There are no frames to export.".into());
    }
    let fps = options.frame_rate.unwrap_or(composition.frame_rate).fps();
    let comp_fps = composition.frame_rate.fps();
    if !(fps.is_finite() && fps > 0.0 && comp_fps.is_finite() && comp_fps > 0.0) {
        return Err("The frame rate can’t be used.".into());
    }
    let ratio = fps / comp_fps;
    let clock = Clock { zero: -(frames.start as f64) * ratio, ratio, rate: composition.frame_rate };
    // A whole number of output frames, as the other formats have.
    let length = ((frames.end - frames.start) as f64 * ratio).round().max(1.0);
    let mut builder = Builder { project, resources, notes: Notes::default(), fps, images: Vec::new(), unreadable: HashSet::new(), precomps: Vec::new(), nesting: vec![composition.id] };
    let root = builder.composition(composition, clock, length);
    let markers = composition.markers.iter().filter(|marker| frames.contains(&marker.frame)).map(|marker| (clock.out(marker.frame as f64), marker.label.clone())).collect();
    Ok(Scene { fps, root, precomps: builder.precomps, images: builder.images, markers, notes: builder.notes })
}

struct Builder<'a> {
    project: &'a Project,
    resources: &'a mut dyn Resources,
    notes: Notes,
    fps: f64,
    images: Vec<(Id, EmbeddedImage)>,
    /// Image items already found unreadable, so they are asked for once.
    unreadable: HashSet<Id>,
    precomps: Vec<(Id, Comp)>,
    /// The compositions being built, outermost first, to stop a composition nesting itself.
    nesting: Vec<Id>,
}

impl Builder<'_> {
    /// One composition's layers, `length` output frames long.
    fn composition(&mut self, composition: &Composition, clock: Clock, length: f64) -> Comp {
        let span = (0.0, length);
        let any_solo = composition.layers.iter().any(|layer| layer.switches.solo);
        // As in the renderer: a track matte is the layer above, and mask layers only shape
        // the layers they mask. Neither is drawn on its own.
        let matte_sources: HashSet<Id> = composition.layers.windows(2).filter(|pair| pair[1].track_matte != TrackMatte::None).map(|pair| pair[0].id).collect();
        let mask_sources: HashSet<Id> = composition.layers.iter().filter_map(|layer| layer.mask).map(|mask| mask.layer).collect();

        // Every layer something is parented to gets a stand-in that carries only its transform,
        // on screen the whole time, so children follow it even while it is hidden or off screen
        // (as in Studio, where a parent's in and out points don't matter).
        let mut parents: Vec<Id> = Vec::new();
        for layer in &composition.layers {
            if let Some(parent) = self.parent_of(composition, layer)
                && !parents.contains(&parent)
            {
                parents.push(parent);
            }
        }
        let proxies: HashMap<Id, usize> = parents.iter().enumerate().map(|(index, id)| (*id, index)).collect();
        let mut layers: Vec<SceneLayer> = Vec::new();
        for id in &parents {
            let layer = composition.layer(*id).expect("parents exist");
            layers.push(SceneLayer {
                name: format!("{} (parent)", layer.name),
                span,
                parent: self.parent_of(composition, layer).and_then(|parent| proxies.get(&parent).copied()),
                transform: self.transform(layer, clock),
                blend: BlendMode::Normal,
                is_matte: false,
                matte: None,
                content: Content::Null,
            });
        }

        for (index, layer) in composition.layers.iter().enumerate() {
            if matte_sources.contains(&layer.id) || mask_sources.contains(&layer.id) {
                continue;
            }
            if !layer.switches.visible || (any_solo && !layer.switches.solo) {
                continue;
            }
            let Some(mut drawn) = self.layer(composition, layer, clock, &proxies) else { continue };
            let above = index.checked_sub(1).map(|above| &composition.layers[above]);
            // The matte (or mask) goes directly above the layer it cuts out.
            let matte = match (layer.track_matte, above, layer.mask) {
                (TrackMatte::None, _, None) | (_, None, None) => None,
                (TrackMatte::None, _, Some(mask)) | (_, None, Some(mask)) => {
                    let kind = if mask.mode == MaskMode::Hide { Matte::AlphaInverted } else { Matte::Alpha };
                    composition.layer(mask.layer).map(|source| (source, kind))
                }
                (track_matte, Some(source), mask) => {
                    if mask.is_some() {
                        self.notes.add("A layer can have a track matte or a mask layer in these formats, not both, so the mask layer was left out", quoted(&layer.name));
                    }
                    let kind = match track_matte {
                        TrackMatte::Alpha | TrackMatte::None => Matte::Alpha,
                        TrackMatte::AlphaInverted => Matte::AlphaInverted,
                        TrackMatte::Luma => Matte::Luma,
                        TrackMatte::LumaInverted => Matte::LumaInverted,
                    };
                    Some((source, kind))
                }
            };
            if let Some((source, kind)) = matte {
                match self.layer(composition, source, clock, &proxies) {
                    Some(mut matte_layer) => {
                        matte_layer.is_matte = true;
                        matte_layer.blend = BlendMode::Normal;
                        layers.push(matte_layer);
                        drawn.matte = Some(kind);
                    }
                    None => self.notes.add("Mattes and masks made from layers these formats can’t hold were left out", quoted(&layer.name)),
                }
            }
            layers.push(drawn);
        }

        if composition.background.a > 0.001 {
            let size = composition.size();
            let transform = Transform { anchor: Track::Static(vec![0.0, 0.0]), ..Transform::identity() };
            layers.push(SceneLayer { name: "Background".into(), span, parent: None, transform, blend: BlendMode::Normal, is_matte: false, matte: None, content: Content::Solid { color: composition.background, size } });
        }
        Comp { name: composition.name.clone(), size: composition.size(), length, layers }
    }

    /// The layer `layer` is parented to, when that layer exists, has a transform, and the chain
    /// of parents never loops back on itself.
    fn parent_of(&mut self, composition: &Composition, layer: &Layer) -> Option<Id> {
        let parent = composition.layer(layer.parent?).filter(|parent| parent.transform.is_some())?;
        let mut seen = vec![layer.id];
        let mut next = Some(parent.id);
        while let Some(id) = next {
            if seen.contains(&id) {
                self.notes.add("Parenting that loops back on itself was ignored", quoted(&layer.name));
                return None;
            }
            seen.push(id);
            next = composition.layer(id).and_then(|layer| layer.parent);
        }
        Some(parent.id)
    }

    /// One layer as the formats draw it, or `None` (with a note) for one they can't.
    fn layer(&mut self, composition: &Composition, layer: &Layer, clock: Clock, proxies: &HashMap<Id, usize>) -> Option<SceneLayer> {
        layer.transform.as_ref()?;
        let content = self.content(layer, clock)?;
        self.note_losses(layer);
        let mut span = (clock.out(layer.in_frame as f64), clock.out(layer.out_frame as f64));
        let transform = self.transform(layer, clock);
        if let Content::Precomp { comp, start, .. } = &content {
            // A nested composition shows nothing before it starts or after it ends.
            let length = self.precomps.iter().find(|(id, _)| id == comp).map_or(0.0, |(_, inner)| inner.length);
            span = (span.0.max(*start), span.1.min(start + length));
        }
        Some(SceneLayer {
            name: layer.name.clone(),
            span,
            parent: self.parent_of(composition, layer).and_then(|parent| proxies.get(&parent).copied()),
            transform,
            blend: layer.blend_mode,
            is_matte: false,
            matte: None,
            content,
        })
    }

    /// Notes what a drawn layer loses: its effects and motion blur.
    fn note_losses(&mut self, layer: &Layer) {
        if layer.switches.effects_enabled {
            for effect in layer.effects.iter().filter(|effect| effect.enabled) {
                self.notes.add("Effects were left out", format!("{} on {}", effect.name, quoted(&layer.name)));
            }
        }
        if layer.switches.motion_blur {
            self.notes.add("Motion blur was left out", quoted(&layer.name));
        }
    }

    fn content(&mut self, layer: &Layer, clock: Clock) -> Option<Content> {
        let name = quoted(&layer.name);
        match &layer.content {
            LayerContent::Footage { .. } => {
                self.notes.add("Video layers were skipped, as these formats can’t play video", name);
                None
            }
            LayerContent::Audio { .. } => {
                self.notes.add("Sound was left out", name);
                None
            }
            LayerContent::Captions { .. } => {
                self.notes.add("Caption layers were left out", name);
                None
            }
            LayerContent::Adjustment => {
                self.notes.add("Adjustment layers were left out, as their effects can’t be exported", name);
                None
            }
            // A fully see-through solid is how Studio makes a null object: it draws nothing.
            LayerContent::Solid { color } if color.a <= 0.001 => Some(Content::Null),
            LayerContent::Solid { color } => Some(Content::Solid { color: *color, size: layer.size }),
            LayerContent::Shape { style, path } => {
                if layer.morph.as_ref().is_some_and(|morph| morph.is_animated()) {
                    self.notes.add("Shape morphs were left out: the shape keeps its own outline and colors", name.clone());
                }
                let shape = geometry::shape(style, path.as_ref(), layer.size)?;
                let open = matches!(&shape, Geometry::Path(path) if !path.closed);
                let pen = style.kind == ShapeKind::Path;
                let mut paints = Vec::new();
                // Only a closed outline has an inside to fill.
                if let (Some(fill), false) = (style.fill, open) {
                    paints.push(PaintOp::Fill { paint: fill, area: layer.size });
                }
                if let Some(stroke) = style.stroke.filter(|stroke| stroke.width.is_finite() && stroke.width > 0.0) {
                    // Drawn shapes get round outer corners from their distance field; pen paths
                    // are stroked with mitred joins, and round ends when open.
                    paints.push(PaintOp::Stroke { paint: stroke.paint, area: layer.size, width: stroke.width, round_joins: !pen, round_caps: pen && open });
                }
                if paints.is_empty() {
                    return None;
                }
                Some(Content::Shapes(vec![Group { name: layer.name.clone(), transform: None, shapes: vec![shape], paints, children: Vec::new() }]))
            }
            LayerContent::Text { text, style } => self.text(layer, text, style, clock),
            LayerContent::Image { item } => {
                if layer.cutout.is_some() {
                    self.notes.add("Background removal was left out", name.clone());
                }
                let embedded = self.image(*item);
                if embedded.is_none() {
                    self.notes.add("Pictures that couldn’t be read were left out", name);
                }
                let (width, height) = embedded?;
                Some(Content::Image { item: *item, size: layer.size, native: Vec2::new(width as f32, height as f32) })
            }
            LayerContent::Composition { composition } => {
                let inner = self.project.composition(*composition)?;
                if self.nesting.contains(composition) || self.nesting.len() >= MAX_NESTING {
                    self.notes.add("Compositions nested inside themselves or too deeply were left out", name);
                    return None;
                }
                if !self.precomps.iter().any(|(id, _)| id == composition) {
                    let inner_fps = inner.frame_rate.fps();
                    if !(inner_fps.is_finite() && inner_fps > 0.0) {
                        return None;
                    }
                    let ratio = self.fps / inner_fps;
                    self.nesting.push(*composition);
                    let built = self.composition(inner, Clock { zero: 0.0, ratio, rate: inner.frame_rate }, inner.duration_frames as f64 * ratio);
                    self.nesting.pop();
                    self.precomps.push((*composition, built));
                }
                Some(Content::Precomp { comp: *composition, size: layer.size, native: inner.size(), start: clock.out(layer.start_frame as f64) })
            }
        }
    }

    /// The size of image `item`, embedding it the first time it is used.
    fn image(&mut self, item: Id) -> Option<(u32, u32)> {
        if let Some((_, image)) = self.images.iter().find(|(id, _)| *id == item) {
            return Some((image.width, image.height));
        }
        if self.unreadable.contains(&item) {
            return None;
        }
        let embedded = self.project.item(item).and_then(|project_item| self.resources.image(project_item)).filter(|image| image.width > 0 && image.height > 0);
        let Some(image) = embedded else {
            self.unreadable.insert(item);
            return None;
        };
        let size = (image.width, image.height);
        self.images.push((item, image));
        Some(size)
    }

    /// A text layer: its letters as outlines (each piece moving on its own while a text
    /// animator runs), or live text when no outlines are available.
    fn text(&mut self, layer: &Layer, text: &str, style: &TextStyle, clock: Clock) -> Option<Content> {
        let name = quoted(&layer.name);
        if layer.text_path.is_some() {
            self.notes.add("Text set along a path was exported in a straight line", name.clone());
        }
        let animated = layer.animation.text_in.is_some() || layer.animation.text_out.is_some();
        let Some(outline) = self.resources.text(text, style) else {
            if animated {
                self.notes.add("Text animations were left out, as the letters couldn’t be outlined", name.clone());
            }
            self.notes.add("Text was kept as live text, so it shows in the viewer’s own copy of the font", name);
            return Some(Content::Text(LiveText { text: text.to_owned(), style: style.clone(), size: layer.size }));
        };
        // Color glyphs (emoji) are pictures, not outlines, so they have no contours to export.
        // Spaces have none either, and aren't missed.
        let pictured = outline.glyphs.iter().any(|glyph| glyph.contours.is_empty() && text[glyph.start..].chars().next().is_some_and(|letter| !letter.is_whitespace()));
        if pictured {
            self.notes.add("Emoji and other color characters were left out of text", name.clone());
        }
        let paints = text_paints(style, outline.size);
        if outline.glyphs.iter().all(|glyph| glyph.contours.is_empty()) || paints.is_empty() {
            return None;
        }
        let shapes_in = |range: Option<&Range<usize>>| -> Vec<Geometry> {
            outline.glyphs.iter().filter(|glyph| range.is_none_or(|range| range.contains(&glyph.start))).flat_map(|glyph| glyph.contours.iter().cloned().map(Geometry::Path)).collect()
        };
        let children = if !animated {
            vec![Group { name: "Letters".into(), transform: None, shapes: shapes_in(None), paints, children: Vec::new() }]
        } else {
            self.notes.add("Text animations were turned into keyframes for each letter, word or line", name);
            // Pieces move on their own: the animators' unit, or single letters when the In and
            // Out animators move different units.
            let units: Vec<TextUnit> = [layer.animation.text_in, layer.animation.text_out].into_iter().flatten().map(|animator| animator.unit).collect();
            let unit = if units.iter().all(|unit| *unit == units[0]) { units[0] } else { TextUnit::Character };
            let pieces = text_animation::units(text, unit);
            pieces
                .iter()
                .enumerate()
                .filter_map(|(index, range)| {
                    let shapes = shapes_in(Some(range));
                    if shapes.is_empty() {
                        return None;
                    }
                    let transform = piece_transform(layer, text, style, &outline, &pieces, index, clock);
                    Some(Group { name: text[range.clone()].to_owned(), transform: Some(transform), shapes, paints: paints.clone(), children: Vec::new() })
                })
                .collect()
        };
        // Studio stretches the laid-out text to the layer's box.
        let fit = Vec2::new(layer.size.x / outline.size.x.max(1.0), layer.size.y / outline.size.y.max(1.0));
        let stretched = (fit.x - 1.0).abs() > 1e-4 || (fit.y - 1.0).abs() > 1e-4;
        Some(Content::Shapes(vec![Group { name: layer.name.clone(), transform: stretched.then(|| Transform::scaling(fit)), shapes: Vec::new(), paints: Vec::new(), children }]))
    }

    /// A layer's transform as tracks.
    fn transform(&mut self, layer: &Layer, clock: Clock) -> Transform {
        let Some(transform) = layer.transform.as_ref() else { return Transform::identity() };
        let pair = |v: Vec2| vec![f64::from(v.x), f64::from(v.y)];
        Transform {
            anchor: self.track(&transform.anchor_point, clock, pair, &layer.name),
            position: self.track(&transform.position, clock, pair, &layer.name),
            scale: self.track(&transform.scale, clock, pair, &layer.name),
            rotation: self.track(&transform.rotation, clock, |v| vec![f64::from(v)], &layer.name),
            opacity: self.track(&transform.opacity, clock, |v| vec![f64::from(v.clamp(0.0, 100.0))], &layer.name),
        }
    }

    /// An animated value as a track in output frames. Elastic and bounce segments, which no
    /// single curve can draw, get a keyframe on every frame between their ends.
    fn track<T: Lerp>(&mut self, animated: &Animated<T>, clock: Clock, parts: impl Fn(T) -> Vec<f64>, layer: &str) -> Track {
        if !animated.is_animated() {
            return Track::Static(parts(animated.value));
        }
        let keyframes = animated.keyframes();
        let mut keys = Vec::new();
        for (index, keyframe) in keyframes.iter().enumerate() {
            let time = clock.out(keyframe.frame as f64);
            let ease = match keyframe.easing {
                Easing::Linear | Easing::Elastic(_) | Easing::Bounce(_) => Ease::Linear,
                Easing::Hold => Ease::Hold,
                // Studio keeps the curve's x values within 0–1 so time never runs backwards.
                Easing::Curve(curve) => Ease::Curve(CubicBezier::new(curve.x1.clamp(0.0, 1.0), curve.y1, curve.x2.clamp(0.0, 1.0), curve.y2)),
            };
            keys.push(Key { time, value: parts(keyframe.value), ease });
            if let (Some(next), Easing::Elastic(_) | Easing::Bounce(_)) = (keyframes.get(index + 1), keyframe.easing) {
                self.notes.add("Elastic and bounce easing was turned into a keyframe on every frame", quoted(layer));
                let end = clock.out(next.frame as f64);
                let mut sample = time.floor() + 1.0;
                while sample < end - 1e-6 {
                    keys.push(Key { time: sample, value: parts(animated.value_at(clock.source(sample))), ease: Ease::Linear });
                    sample += 1.0;
                }
            }
        }
        Track::from_keys(keys)
    }
}

/// A text style's paints, bottom first: the outline under the fill. Studio draws the outline
/// outside the letters only. Under a fill that is matched by a stroke twice as wide, whose
/// inner half the fill covers. Text with an outline and no fill is hollow, as Studio draws it:
/// neither format can stroke only the outside, so the outline is centred on the letters' edges
/// at Studio's width.
fn text_paints(style: &TextStyle, area: Vec2) -> Vec<PaintOp> {
    let stroke = style.stroke.filter(|(_, width)| width.is_finite() && *width > 0.0);
    let mut paints = Vec::new();
    if let Some((color, width)) = stroke {
        let width = if style.fill.is_some() { width * 2.0 } else { width };
        paints.push(PaintOp::Stroke { paint: Paint::Solid(color), area, width, round_joins: true, round_caps: true });
    }
    if let Some(color) = style.fill {
        paints.push(PaintOp::Fill { paint: Paint::Solid(color), area });
    }
    paints
}

/// The middle of piece `index` of `pieces` (byte ranges of the text), as the renderer turns and
/// scales pieces about it: from its first letter to its last, widened to the box's edges at the
/// ends of a line, and its line's height (to the box's top and bottom on the first and last).
fn piece_middle(outline: &TextOutline, pieces: &[Range<usize>], index: usize) -> Option<Vec2> {
    let range = pieces.get(index)?;
    let glyphs: Vec<_> = outline.glyphs.iter().filter(|glyph| range.contains(&glyph.start)).collect();
    let first = glyphs.first()?;
    let line = first.line;
    let mut left = glyphs.iter().map(|glyph| glyph.left).fold(f32::MAX, f32::min);
    let mut right = glyphs.iter().map(|glyph| glyph.left + glyph.width).fold(f32::MIN, f32::max);
    let line_of = |start: usize| outline.glyphs.iter().find(|glyph| glyph.start >= start).map_or(line, |glyph| glyph.line);
    let on_line: Vec<&Range<usize>> = pieces.iter().filter(|other| line_of(other.start) == line).collect();
    if on_line.iter().all(|other| other.start >= range.start) {
        left = 0.0;
    }
    if on_line.iter().all(|other| other.end <= range.end) {
        right = outline.size.x;
    }
    let (line_top, line_height) = outline.lines.get(line).copied().unwrap_or((0.0, outline.size.y));
    let last_line = outline.lines.len().saturating_sub(1);
    let top = if line == 0 { 0.0 } else { line_top };
    let bottom = if line >= last_line { outline.size.y } else { line_top + line_height };
    Some(Vec2::new((left + right) * 0.5, (top + bottom) * 0.5))
}

/// How piece `index` of a text layer moves while its text animators run, sampled at every
/// output frame of each animator (and at its ends), as a transform about the piece's middle.
fn piece_transform(layer: &Layer, text: &str, style: &TextStyle, outline: &TextOutline, pieces: &[Range<usize>], index: usize, clock: Clock) -> Transform {
    let home_middle = piece_middle(outline, pieces, index).unwrap_or_default();
    let range = &pieces[index];
    let fps = clock.rate.fps();
    let length = |seconds: f32| f64::from(seconds.clamp(text_animation::MIN_SECONDS, text_animation::MAX_SECONDS)) * fps;
    let mut windows = Vec::new();
    if let Some(animator) = layer.animation.text_in {
        windows.push((layer.in_frame as f64, layer.in_frame as f64 + length(animator.seconds)));
    }
    if let Some(animator) = layer.animation.text_out {
        windows.push((layer.out_frame as f64 - length(animator.seconds), layer.out_frame as f64));
    }
    let mut times: Vec<f64> = Vec::new();
    for (start, end) in windows {
        let (from, to) = (clock.out(start), clock.out(end));
        times.push(from);
        let mut sample = from.ceil();
        while sample < to {
            times.push(sample);
            sample += 1.0;
        }
        times.push(to);
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

    let mut tracks: [Vec<Key>; 5] = Default::default();
    for time in times {
        let frame = clock.source(time);
        let poses = text_animation::layer_poses(layer, text, style.font_size, frame, clock.rate);
        // The piece moves with the running animator's unit that holds it, about that unit's middle.
        let (pose, middle) = poses
            .as_ref()
            .and_then(|poses| {
                let units: Vec<Range<usize>> = poses.iter().map(|(unit, _)| unit.clone()).collect();
                let holder = units.iter().position(|unit| unit.contains(&range.start))?;
                Some((poses[holder].1, piece_middle(outline, &units, holder).unwrap_or(home_middle)))
            })
            .unwrap_or((UnitPose::HOME, home_middle));
        let typing = text_animation::running(layer, frame, clock.rate).is_some_and(|(animator, _, _)| animator.motion == TextMotion::Typewriter);
        let ease = if typing { Ease::Hold } else { Ease::Linear };
        let (mx, my) = (f64::from(middle.x), f64::from(middle.y));
        let values = [
            vec![mx, my],
            vec![mx + f64::from(pose.offset.x), my + f64::from(pose.offset.y)],
            vec![f64::from(pose.scale) * 100.0, f64::from(pose.scale) * 100.0],
            vec![f64::from(pose.rotation)],
            vec![f64::from(pose.opacity) * 100.0],
        ];
        for (track, value) in tracks.iter_mut().zip(values) {
            track.push(Key { time, value, ease });
        }
    }
    let [anchor, position, scale, rotation, opacity] = tracks.map(Track::from_keys);
    Transform { anchor, position, scale, rotation, opacity }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f64, value: f64, ease: Ease) -> Key {
        Key { time, value: vec![value], ease }
    }

    #[test]
    fn tracks_hold_outside_their_keyframes_and_ease_between() {
        let track = Track::from_keys(vec![key(10.0, 0.0, Ease::Linear), key(20.0, 100.0, Ease::Linear)]);
        assert_eq!(track.value_at(0.0), [0.0]);
        assert_eq!(track.value_at(15.0), [50.0]);
        assert_eq!(track.value_at(99.0), [100.0]);
        let held = Track::from_keys(vec![key(0.0, 0.0, Ease::Hold), key(10.0, 1.0, Ease::Linear)]);
        assert_eq!(held.value_at(9.9), [0.0]);
    }

    #[test]
    fn keyframes_that_change_nothing_are_dropped() {
        let track = Track::from_keys(vec![key(0.0, 5.0, Ease::Linear), key(5.0, 5.0, Ease::Linear), key(10.0, 5.0, Ease::Linear)]);
        assert_eq!(track, Track::Static(vec![5.0]));
        let track = Track::from_keys(vec![key(0.0, 0.0, Ease::Linear), key(5.0, 5.0, Ease::Linear), key(6.0, 5.0, Ease::Linear), key(7.0, 5.0, Ease::Linear), key(10.0, 0.0, Ease::Linear)]);
        let Track::Keys(keys) = track else { panic!() };
        assert_eq!(keys.len(), 4, "the middle of the flat run is gone");
    }

    #[test]
    fn cutting_a_curve_keeps_its_shape() {
        let curve = Ease::Curve(CubicBezier::new(0.34, 1.56, 0.64, 1.0));
        let track = Track::from_keys(vec![key(0.0, 0.0, curve), key(40.0, 100.0, Ease::Linear)]);
        let cut = track.clipped(10.0, 30.0);
        for time in [10.0, 14.0, 20.0, 26.0, 30.0] {
            let (before, after) = (track.value_at(time)[0], cut.value_at(time)[0]);
            assert!((before - after).abs() < 0.2, "at {time}: {before} vs {after}");
        }
        let Track::Keys(keys) = &cut else { panic!() };
        assert_eq!((keys[0].time, keys.last().unwrap().time), (10.0, 30.0));
        // Wholly before or after the cut, a track is constant.
        assert_eq!(track.clipped(50.0, 60.0), Track::Static(vec![100.0]));
    }

    #[test]
    fn a_held_segment_cut_partway_keeps_its_value() {
        let track = Track::from_keys(vec![key(0.0, 0.0, Ease::Hold), key(10.0, 1.0, Ease::Hold), key(20.0, 2.0, Ease::Linear)]);
        let cut = track.clipped(5.0, 15.0);
        assert_eq!(cut.value_at(5.0), [0.0]);
        assert_eq!(cut.value_at(12.0), [1.0]);
        assert_eq!(cut.value_at(15.0), [1.0]);
    }
}
