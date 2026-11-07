//! Media slots: places a photo or video can be dropped into from the viewer.
//!
//! A slot is either a placeholder (a shape named "Drop your … here", as templates and mockups
//! make) or a nested composition that shows a single photo or video. Dropping media on a
//! placeholder turns it into a small composition of its own holding the media, cropped to the
//! placeholder's shape, so the layer keeps its animation, effects and timing. Dropping media on
//! a composition that already holds one photo or video swaps it, keeping any pan and zoom, and
//! every place that shows that composition updates with it.
//!
//! Slots are found through any depth of nested compositions, so dropping on a template or
//! mockup placed inside another composition fills the right spot inside it.

use crate::animation::{Animated, Vec2};
use crate::edit::{self, CompositionSettings};
use crate::model::{Composition, Id, Layer, LayerContent, LayerMask, MaskMode, Paint, Project, ProjectItemKind, Rgba, ShapeKind};
use crate::transform::{Affine, world_transform};

/// How the name of every placeholder starts; the rest says what goes there ("photo here").
pub const PLACEHOLDER_PREFIX: &str = "Drop your ";

/// The name a placeholder for `what` (such as "product photo") gets.
pub fn placeholder_name(what: &str) -> String {
    format!("{PLACEHOLDER_PREFIX}{what} here")
}

/// Whether `layer` is a placeholder waiting for media.
pub fn is_placeholder(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Shape { .. }) && layer.name.starts_with(PLACEHOLDER_PREFIX)
}

/// What a drop would fill, found under a point in the viewer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    /// The composition holding the layer to fill (the open one, or one nested inside it).
    pub comp: Id,
    /// The placeholder, or the photo or video layer to swap.
    pub layer: Id,
    pub kind: SlotKind,
    /// The layer of the open composition the slot was found under, to select after filling.
    pub root: Id,
    /// The slot's outline in the open composition's pixels, clockwise from top-left, for
    /// highlighting it while something is dragged over it.
    pub outline: [Vec2; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// A placeholder shape: filling it wraps the media in a composition of the shape's size.
    Placeholder,
    /// The one photo or video in a nested composition: filling it swaps the media.
    Media,
}

/// Whether project item `item` can go in a slot: a photo or a video with a known size.
pub fn can_fill_with(project: &Project, item: Id) -> bool {
    project.item(item).is_some_and(|item| matches!(item.kind, ProjectItemKind::Footage | ProjectItemKind::Image) && item.size.is_some())
}

/// The slot under `point` (pixels of composition `comp`) at `frame`, if any. Only the topmost
/// layer under the point is considered, looking inside nested compositions, so a drop never
/// fills something hidden beneath another layer.
pub fn slot_at(project: &Project, comp: Id, point: Vec2, frame: i64) -> Option<Slot> {
    find(project, comp, point, frame, Affine::IDENTITY, None, 0)
}

/// How deep to look into nested compositions, matching what the renderer draws.
const MAX_DEPTH: u32 = 8;

/// Looks for a slot in composition `comp` under `point` (its own pixels) at its `frame`.
/// `to_open` maps its pixels to the open composition's, and `root` is the layer of the open
/// composition being looked inside (`None` while looking at the open composition itself).
fn find(project: &Project, comp: Id, point: Vec2, frame: i64, to_open: Affine, root: Option<Id>, depth: u32) -> Option<Slot> {
    let composition = project.composition(comp)?;
    // A nested composition holding just one photo or video (or one placeholder) is a slot as
    // a whole, wherever in it the drop lands and even after a short video has ended.
    if let Some(root) = root
        && let Some((layer, kind)) = sole_media(composition)
    {
        let (width, height) = (composition.width as f32, composition.height as f32);
        let corners = [Vec2::new(0.0, 0.0), Vec2::new(width, 0.0), Vec2::new(width, height), Vec2::new(0.0, height)];
        return Some(Slot { comp, layer, kind, root, outline: corners.map(|corner| to_open.apply(corner)) });
    }
    let fframe = frame as f64;
    let top = composition
        .layers
        .iter()
        .filter(|layer| layer.switches.visible && !layer.switches.locked && layer.is_active_at(frame) && layer.content.has_picture())
        .find(|layer| shows_point(composition, layer, point, fframe))?;
    let root_here = root.unwrap_or(top.id);
    match top.content {
        LayerContent::Shape { .. } if is_placeholder(top) => {
            let world = world_transform(composition, top.id, fframe)?;
            let (width, height) = (top.size.x, top.size.y);
            let corners = [Vec2::new(0.0, 0.0), Vec2::new(width, 0.0), Vec2::new(width, height), Vec2::new(0.0, height)];
            let outline = corners.map(|corner| to_open.apply(world.apply(corner)));
            Some(Slot { comp, layer: top.id, kind: SlotKind::Placeholder, root: root_here, outline })
        }
        LayerContent::Composition { composition: inner_id } if depth < MAX_DEPTH => {
            let inner = project.composition(inner_id)?;
            // The nested composition is stretched over the layer's box (as the renderer draws it).
            let stretch = Affine::scale(Vec2::new(top.size.x / (inner.width as f32).max(1.0), top.size.y / (inner.height as f32).max(1.0)));
            let inner_to_comp = stretch.then(world_transform(composition, top.id, fframe)?);
            let inner_point = inner_to_comp.inverse()?.apply(point);
            let inner_frame = nested_frame(composition, top, inner, frame);
            find(project, inner_id, inner_point, inner_frame, inner_to_comp.then(to_open), Some(root_here), depth + 1)
        }
        _ => None,
    }
}

/// The frame of nested composition `inner` that plays at frame `frame` of `outer`, through
/// `layer`: counted from where the layer starts, at the nested composition's own rate.
fn nested_frame(outer: &Composition, layer: &Layer, inner: &Composition, frame: i64) -> i64 {
    let seconds = (frame - layer.start_frame) as f64 / outer.frame_rate.fps();
    (seconds * inner.frame_rate.fps() + 1e-6).floor() as i64
}

/// Whether `layer` draws at `point`: on its box, and inside its mask (or outside, for a mask
/// that hides).
fn shows_point(composition: &Composition, layer: &Layer, point: Vec2, frame: f64) -> bool {
    if !crate::transform::layer_contains(composition, layer.id, point, frame) {
        return false;
    }
    match layer.mask {
        Some(LayerMask { layer: mask, mode }) if composition.layer(mask).is_some() => {
            crate::transform::layer_contains(composition, mask, point, frame) == (mode == MaskMode::Show)
        }
        _ => true,
    }
}

/// The one photo, video or placeholder a composition shows, if that's all it shows. Layers
/// switched off (such as a mask) don't count.
fn sole_media(composition: &Composition) -> Option<(Id, SlotKind)> {
    let mut shown = composition.layers.iter().filter(|layer| layer.switches.visible && layer.content.has_picture());
    let only = shown.next()?;
    if shown.next().is_some() {
        return None;
    }
    match only.content {
        LayerContent::Footage { .. } | LayerContent::Image { .. } => Some((only.id, SlotKind::Media)),
        LayerContent::Shape { .. } if is_placeholder(only) => Some((only.id, SlotKind::Placeholder)),
        _ => None,
    }
}

/// Puts photo or video `item` in `slot`. Returns the layer now showing it (inside the slot's
/// composition), or `None` if the item can't go in a slot or the slot is gone.
pub fn fill(project: &mut Project, slot: &Slot, item: Id) -> Option<Id> {
    if !can_fill_with(project, item) {
        return None;
    }
    match slot.kind {
        SlotKind::Placeholder => fill_placeholder(project, slot.comp, slot.layer, item),
        SlotKind::Media => swap_media(project, slot.comp, slot.layer, item).then_some(slot.layer),
    }
}

/// Turns placeholder `layer` of `comp` into a nested composition of its size showing `item`,
/// scaled to cover it and cropped to the placeholder's shape. The layer keeps its id, place,
/// animation, effects and timing; only what it shows changes. Returns the media layer.
fn fill_placeholder(project: &mut Project, comp: Id, layer_id: Id, item: Id) -> Option<Id> {
    let composition = project.composition(comp)?;
    let placeholder = composition.layer(layer_id)?.clone();
    let LayerContent::Shape { style, path } = &placeholder.content else { return None };
    let what = placeholder.name.trim_start_matches(PLACEHOLDER_PREFIX).trim_end_matches(" here");
    let name = format!("{} slot", capitalized(what));
    let side = |value: f32| (value.round() as u32).clamp(1, CompositionSettings::MAX_SIZE);
    let settings = CompositionSettings {
        name: name.clone(),
        width: side(placeholder.size.x),
        height: side(placeholder.size.y),
        frame_rate: composition.frame_rate,
        // Exactly as long as the placeholder is on screen: the media starts as it appears.
        duration_frames: (placeholder.out_frame - placeholder.in_frame).max(1),
        background: Rgba::TRANSPARENT,
    };
    let folder = project.item(comp).and_then(|item| item.parent);
    let slot = edit::new_composition(project, &settings).ok()?;
    if folder.is_some() {
        crate::folders::move_to(project, slot, folder);
    }
    let size = Vec2::new(settings.width as f32, settings.height as f32);
    let middle = size * 0.5;

    // A plain rectangle is cropped by the composition's own edges; any other shape (rounded
    // corners, a circle, a drawn path) gets a hidden copy of itself as the media's mask.
    let needs_mask = style.kind != ShapeKind::Rectangle || style.corner_radius > 0.0 || path.is_some();
    let mask = if needs_mask {
        let solid = crate::model::ShapeStyle { fill: Some(Paint::Solid(Rgba::hex(0xffffff))), stroke: None, ..*style };
        let mask = edit::add_shape_layer(project, slot, solid, size, middle, 0)?;
        let layer = project.composition_mut(slot)?.layer_mut(mask)?;
        layer.name = "Slot shape".into();
        layer.content = LayerContent::Shape { style: solid, path: path.clone() };
        layer.switches.visible = false;
        Some(mask)
    } else {
        None
    };
    let media = edit::add_layer_from_item(project, slot, item, 0, 0)?;
    let inside = project.composition_mut(slot)?;
    cover(inside.layer_mut(media)?, size);
    if let Some(mask) = mask {
        edit::set_layer_mask(inside, media, Some(LayerMask { layer: mask, mode: MaskMode::Show }));
    }

    // The placeholder becomes the slot's layer, stretched over the same box as before. A
    // shape's start can lie after its in point (it was trimmed earlier, which shapes allow), so
    // the slot's time is set to begin at the in point, or its first frames would show nothing.
    let outer_comp = project.composition_mut(comp)?;
    // Text set along the placeholder's outline can't follow a composition: it goes back to a line.
    for layer in &mut outer_comp.layers {
        if layer.text_path.is_some_and(|on| on.layer == layer_id) {
            layer.text_path = None;
        }
    }
    let outer = outer_comp.layer_mut(layer_id)?;
    outer.start_frame = outer.in_frame;
    outer.content = LayerContent::Composition { composition: slot };
    outer.name = name;
    outer.morph = None;
    // A video's sound plays through the slot, as any nested composition's does.
    outer.has_audio = true;
    outer.switches.audio = true;
    Some(media)
}

/// Swaps the photo or video layer `layer_id` of `comp` for `item`. The new media covers the
/// same area the old one did: its scale changes by however much more or less it takes to cover
/// the composition, so a pan or zoom keeps going. It plays from its start, at normal speed.
fn swap_media(project: &mut Project, comp: Id, layer_id: Id, item_id: Id) -> bool {
    let Some(item) = project.item(item_id).cloned() else { return false };
    let Some(composition) = project.composition_mut(comp) else { return false };
    let frame_size = composition.size();
    let (rate, duration) = (composition.frame_rate, composition.duration_frames);
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let Some(new_size) = item.size else { return false };
    if !matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Image { .. } | LayerContent::Shape { .. }) {
        return false;
    }
    let old_size = layer.size;
    let factor = cover_scale(frame_size, new_size) / cover_scale(frame_size, old_size);
    if let Some(transform) = layer.transform.as_mut() {
        transform.scale.map_values(|scale| scale * factor);
        // The anchor stays at the same spot relative to the picture (normally its middle).
        transform.anchor_point.map_values(|anchor| Vec2::new(anchor.x / old_size.x.max(1.0) * new_size.x, anchor.y / old_size.y.max(1.0) * new_size.y));
    }
    layer.content = match item.kind {
        ProjectItemKind::Image => LayerContent::Image { item: item_id },
        _ => LayerContent::Footage { item: item_id },
    };
    layer.name = item.name.clone();
    layer.size = new_size;
    layer.start_frame = layer.in_frame;
    layer.out_frame = match item.duration_seconds {
        Some(seconds) => layer.in_frame.saturating_add(rate.seconds_to_frames(seconds).clamp(1, crate::model::MAX_FRAME)),
        None => duration.max(layer.in_frame + 1),
    };
    layer.has_audio = item.has_audio;
    layer.switches.audio = item.has_audio;
    layer.time = Default::default();
    // Background removal and cleaned-up sound were made for the old media.
    layer.cutout = None;
    layer.audio.enhance = None;
    true
}

/// Places a just-added media layer in the middle of a `frame` sized area, scaled to cover it.
fn cover(layer: &mut Layer, frame: Vec2) {
    let size = layer.size;
    let Some(transform) = layer.transform.as_mut() else { return };
    let fill = cover_scale(frame, size) * 100.0;
    transform.anchor_point = Animated::constant(size * 0.5);
    transform.position = Animated::constant(frame * 0.5);
    transform.scale = Animated::constant(Vec2::new(fill, fill));
}

/// How much `size` must be scaled by to cover `frame` with no gaps.
fn cover_scale(frame: Vec2, size: Vec2) -> f32 {
    (frame.x / size.x.max(1.0)).max(frame.y / size.y.max(1.0))
}

/// `text` with its first letter capitalized.
fn capitalized(text: &str) -> String {
    let mut letters = text.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => "Media".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::FootageInfo;
    use crate::model::ShapeStyle;
    use crate::time::FrameRate;

    /// A 1000 × 1000 composition with a rounded 400 × 300 placeholder turned 90° at (500, 500),
    /// and a 1600 × 900 photo in the project.
    fn project_with_placeholder() -> (Project, Id, Id, Id) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = CompositionSettings {
            name: "Main".into(),
            width: 1000,
            height: 1000,
            frame_rate: FrameRate::new(30, 1),
            duration_frames: 90,
            background: Rgba::TRANSPARENT,
        };
        let comp = edit::new_composition(&mut project, &settings).unwrap();
        let style = ShapeStyle { corner_radius: 20.0, ..ShapeStyle::new(ShapeKind::Rectangle) };
        let layer = edit::add_shape_layer(&mut project, comp, style, Vec2::new(400.0, 300.0), Vec2::new(500.0, 500.0), 0).unwrap();
        let placeholder = project.composition_mut(comp).unwrap().layer_mut(layer).unwrap();
        placeholder.name = placeholder_name("photo");
        placeholder.transform.as_mut().unwrap().rotation = Animated::constant(90.0);
        let photo = edit::import_image(&mut project, "/media/photo.jpg".into(), Vec2::new(1600.0, 900.0));
        (project, comp, layer, photo)
    }

    #[test]
    fn a_placeholder_under_the_pointer_is_a_slot_and_its_outline_follows_its_turn() {
        let (project, comp, layer, _) = project_with_placeholder();
        let slot = slot_at(&project, comp, Vec2::new(500.0, 650.0), 0).expect("on the turned placeholder");
        assert_eq!((slot.comp, slot.layer, slot.kind, slot.root), (comp, layer, SlotKind::Placeholder, layer));
        // Turned a quarter, the 400-wide box stands 400 tall around (500, 500).
        let top = slot.outline.iter().map(|corner| corner.y).fold(f32::MAX, f32::min);
        assert!((top - 300.0).abs() < 0.01, "top edge at {top}");
        assert!(slot_at(&project, comp, Vec2::new(100.0, 100.0), 0).is_none(), "nothing there");
    }

    #[test]
    fn filling_a_placeholder_keeps_the_layer_and_crops_the_photo_to_its_shape() {
        let (mut project, comp, layer, photo) = project_with_placeholder();
        let slot = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).unwrap();
        let media = fill(&mut project, &slot, photo).expect("filled");
        let outer = project.composition(comp).unwrap().layer(layer).unwrap();
        let LayerContent::Composition { composition: inner } = outer.content else { panic!("now a nested composition") };
        assert_eq!(outer.transform.as_ref().unwrap().rotation.value, 90.0, "keeps its turn");
        assert_eq!(outer.name, "Photo slot");
        let inner = project.composition(inner).unwrap();
        assert_eq!((inner.width, inner.height), (400, 300));
        let shown = inner.layer(media).unwrap();
        // 1600 × 900 covers 400 × 300 at a third of its size (a quarter would leave gaps).
        assert!((shown.transform.as_ref().unwrap().scale.value.x - 100.0 / 3.0).abs() < 0.01);
        let mask = shown.mask.expect("cropped to the rounded corners");
        assert!(!inner.layer(mask.layer).unwrap().switches.visible, "the mask itself isn't drawn");
    }

    #[test]
    fn a_placeholder_trimmed_to_start_earlier_shows_its_media_from_its_first_frame() {
        let (mut project, comp, layer, photo) = project_with_placeholder();
        // Shapes have no source, so their in point can be moved before their start.
        let placeholder = project.composition_mut(comp).unwrap().layer_mut(layer).unwrap();
        (placeholder.in_frame, placeholder.start_frame) = (0, 30);
        let slot = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).unwrap();
        let media = fill(&mut project, &slot, photo).unwrap();
        let outer = project.composition(comp).unwrap().layer(layer).unwrap();
        let LayerContent::Composition { composition: inner } = outer.content else { panic!() };
        assert_eq!(outer.start_frame, outer.in_frame, "the slot's time begins as it appears");
        let inner = project.composition(inner).unwrap();
        assert_eq!(inner.duration_frames, outer.out_frame - outer.in_frame);
        assert!(inner.layer(media).unwrap().is_active_at(0), "the photo shows from the slot's first frame");
    }

    #[test]
    fn text_set_along_a_placeholder_goes_back_to_a_line_when_it_is_filled() {
        let (mut project, comp, layer, photo) = project_with_placeholder();
        let text = edit::add_text_layer(&mut project, comp, "Along", Vec2::new(500.0, 200.0), 0).unwrap();
        // Turned into a pen path (as the Path stopwatch does) so text can follow it.
        let composition = project.composition_mut(comp).unwrap();
        assert!(crate::morph::convert_to_path(composition, layer));
        assert!(edit::set_text_path(composition, text, Some(crate::model::TextPath { layer, start: 0.0 })));
        let slot = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).unwrap();
        fill(&mut project, &slot, photo).unwrap();
        assert_eq!(project.composition(comp).unwrap().layer(text).unwrap().text_path, None);
        assert_eq!(crate::project_file::check(&project), Ok(()), "the project can still be saved");
    }

    #[test]
    fn dropping_again_swaps_the_media_and_keeps_its_zoom() {
        let (mut project, comp, _, photo) = project_with_placeholder();
        let slot = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).unwrap();
        let media = fill(&mut project, &slot, photo).unwrap();
        let inner = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).expect("the filled slot");
        assert_eq!((inner.kind, inner.layer), (SlotKind::Media, media));
        // A slow zoom on the photo, then a square video in its place.
        let layer = project.composition_mut(inner.comp).unwrap().layer_mut(media).unwrap();
        let transform = layer.transform.as_mut().unwrap();
        transform.scale = Animated::keyframed(vec![
            crate::animation::Keyframe { frame: 0, value: Vec2::new(25.0, 25.0), easing: crate::animation::Easing::Linear },
            crate::animation::Keyframe { frame: 60, value: Vec2::new(50.0, 50.0), easing: crate::animation::Easing::Linear },
        ]);
        let info = FootageInfo { size: Vec2::new(200.0, 200.0), duration_seconds: 1.0, frame_rate: FrameRate::new(30, 1), has_audio: false };
        let video = edit::import_footage(&mut project, "/media/clip.mov".into(), &info);
        assert_eq!(fill(&mut project, &inner, video), Some(media));
        let swapped = project.composition(inner.comp).unwrap().layer(media).unwrap();
        assert_eq!(swapped.content, LayerContent::Footage { item: video });
        assert_eq!(swapped.out_frame, 30, "plays the video's own length");
        // 200 × 200 needs 200% to cover 400 × 300 where the photo needed a third: six times more.
        let keys: Vec<f32> = swapped.transform.as_ref().unwrap().scale.keyframes().iter().map(|key| key.value.x).collect();
        assert!((keys[0] - 150.0).abs() < 0.01 && (keys[1] - 300.0).abs() < 0.01, "got {keys:?}");
    }

    #[test]
    fn a_slot_inside_a_nested_composition_is_found_and_outlined_where_it_shows() {
        let (mut project, inner_comp, layer, photo) = project_with_placeholder();
        // A background behind the placeholder, so the composition isn't a slot as a whole.
        edit::add_background(&mut project, inner_comp, Rgba::hex(0x223344)).unwrap();
        // Show the 1000 × 1000 composition at half size in the top-left of another.
        let settings = CompositionSettings { name: "Outer".into(), width: 1000, height: 1000, frame_rate: FrameRate::new(30, 1), duration_frames: 90, background: Rgba::TRANSPARENT };
        let outer = edit::new_composition(&mut project, &settings).unwrap();
        let nested = edit::add_layer_from_item(&mut project, outer, inner_comp, 0, 0).unwrap();
        let transform = project.composition_mut(outer).unwrap().layer_mut(nested).unwrap().transform.as_mut().unwrap();
        transform.scale = Animated::constant(Vec2::new(50.0, 50.0));
        transform.position = Animated::constant(Vec2::new(250.0, 250.0));
        let slot = slot_at(&project, outer, Vec2::new(250.0, 250.0), 0).expect("the placeholder, through the nesting");
        assert_eq!((slot.comp, slot.layer, slot.root), (inner_comp, layer, nested));
        let top = slot.outline.iter().map(|corner| corner.y).fold(f32::MAX, f32::min);
        assert!((top - 150.0).abs() < 0.01, "half of 300, at {top}");
        assert!(fill(&mut project, &slot, photo).is_some());
    }

    #[test]
    fn a_nested_composition_holding_only_a_placeholder_is_a_slot_as_a_whole() {
        let (mut project, inner_comp, layer, _) = project_with_placeholder();
        let settings = CompositionSettings { name: "Outer".into(), width: 1000, height: 1000, frame_rate: FrameRate::new(30, 1), duration_frames: 90, background: Rgba::TRANSPARENT };
        let outer = edit::new_composition(&mut project, &settings).unwrap();
        let nested = edit::add_layer_from_item(&mut project, outer, inner_comp, 0, 0).unwrap();
        // Off the placeholder itself, but on the composition showing it.
        let slot = slot_at(&project, outer, Vec2::new(50.0, 50.0), 0).expect("the whole composition counts");
        assert_eq!((slot.comp, slot.layer, slot.kind, slot.root), (inner_comp, layer, SlotKind::Placeholder, nested));
    }

    #[test]
    fn something_drawn_over_a_placeholder_keeps_drops_off_it() {
        let (mut project, comp, _, _) = project_with_placeholder();
        edit::add_text_layer(&mut project, comp, "On top", Vec2::new(500.0, 500.0), 0).unwrap();
        assert!(slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).is_none());
    }

    #[test]
    fn only_pictures_and_videos_fill_slots() {
        let (mut project, comp, _, _) = project_with_placeholder();
        let slot = slot_at(&project, comp, Vec2::new(500.0, 500.0), 0).unwrap();
        let sound = edit::import_audio(&mut project, "/media/song.mp3".into(), 10.0);
        assert!(!can_fill_with(&project, sound));
        assert!(fill(&mut project, &slot, sound).is_none());
        assert!(fill(&mut project, &slot, comp).is_none(), "not a composition either");
    }
}
