//! Edits to a project. Every change the UI makes goes through a function here,
//! which keeps the rules in one tested place (and is where undo will hook in).

use crate::animation::Vec2;
use crate::transform::Affine;
use crate::model::{
    BlendMode, Composition, INNER_RADIUS_RANGE, Id, Layer, LayerContent, LayerSwitches, MAX_SHAPE_POINTS, MAX_STROKE_WIDTH, Paint,
    Project, ProjectItem, ProjectItemKind, Rgba, ShapeKind, ShapeStyle, TextStyle, TrackMatte, Transform,
};
use crate::time::FrameRate;

/// Whether `item` can be added as a layer to `composition`.
/// Folders cannot, and a composition cannot be placed inside itself.
pub fn can_add_item(item: &ProjectItem, composition: &Composition) -> bool {
    item.kind != ProjectItemKind::Folder && item.id != composition.id
}

/// Builds a layer for `item` starting at `in_frame`. Time-based media keeps its own length,
/// converted to the composition's frame rate; stills and solids run to the end of the composition.
fn layer_from_item(item: &ProjectItem, composition: &Composition, in_frame: i64, id: Id) -> Option<Layer> {
    if !can_add_item(item, composition) {
        return None;
    }
    let content = match item.kind {
        ProjectItemKind::Folder => return None,
        ProjectItemKind::Footage => LayerContent::Footage { item: item.id },
        ProjectItemKind::Image => LayerContent::Image { item: item.id },
        ProjectItemKind::Audio => LayerContent::Audio { item: item.id },
        ProjectItemKind::Composition => LayerContent::Composition { composition: item.id },
        ProjectItemKind::Solid => LayerContent::Solid { color: item.color.unwrap_or(crate::model::Rgba::hex(0x333333)) },
    };

    // A bogus or enormous media length (from a damaged file or a misreported probe) is kept
    // within the frame limit, so the out point below cannot overflow.
    let duration = match item.duration_seconds {
        Some(seconds) => composition.frame_rate.seconds_to_frames(seconds),
        None => composition.duration_frames - in_frame,
    }
    .clamp(1, crate::model::MAX_FRAME);
    let size = item.size.unwrap_or(composition.size());
    let has_picture = content.has_picture();

    Some(Layer {
        id,
        name: item.name.clone(),
        label: item.label,
        size,
        in_frame,
        out_frame: in_frame.saturating_add(duration),
        start_frame: in_frame,
        track: 0,
        blend_mode: BlendMode::Normal,
        track_matte: TrackMatte::None,
        mask: None,
        cutout: None,
        text_path: None,
        parent: None,
        switches: LayerSwitches { visible: has_picture, audio: item.has_audio, ..LayerSwitches::default() },
        transform: has_picture.then(|| Transform::centered(size, composition.size() * 0.5)),
        effects: Vec::new(),
        has_audio: item.has_audio,
        audio: Default::default(),
        time: Default::default(),
        animation: Default::default(),
        content,
    })
}

/// Adds project item `item_id` to composition `comp_id` as a new layer at `stack_index` (0 = top),
/// starting at `in_frame`. Returns the new layer's id, or `None` if the item cannot become a layer.
pub fn add_layer_from_item(
    project: &mut Project,
    comp_id: Id,
    item_id: Id,
    stack_index: usize,
    in_frame: i64,
) -> Option<Id> {
    // A composition can't end up inside itself, however deeply.
    if contains_composition(project, item_id, comp_id) {
        return None;
    }
    let id = project.allocate_id();
    let item = project.item(item_id)?.clone();
    let composition = project.composition_mut(comp_id)?;
    let start = in_frame.clamp(0, composition.duration_frames - 1);
    let mut layer = layer_from_item(&item, composition, start, id)?;
    let index = keep_matte_pairs(composition, stack_index.min(composition.layers.len()));
    // A track of its own at that height (see `tracks`).
    layer.track = crate::tracks::make_room_at(composition, index, crate::tracks::is_sound_only(&layer));
    composition.layers.insert(index, layer);
    crate::tracks::normalize(composition);
    Some(id)
}

/// Separates a video clip's sound onto a clip of its own, as CapCut and Premiere do: an audio
/// layer on a sound track playing the same file over the same span (trim and slip included),
/// while the video clip itself goes quiet. Returns the new audio layer. Clips with a speed
/// change or reversed are refused: audio layers always play at normal speed.
pub fn separate_audio(project: &mut Project, comp_id: Id, layer_id: Id) -> Option<Id> {
    let id = project.allocate_id();
    let composition = project.composition_mut(comp_id)?;
    let index = composition.layers.iter().position(|layer| layer.id == layer_id)?;
    let video = &composition.layers[index];
    let LayerContent::Footage { item } = video.content else { return None };
    if !video.has_audio || video.switches.locked || !video.time.is_normal() {
        return None;
    }
    let audio = Layer {
        id,
        name: format!("{} (audio)", video.name),
        content: LayerContent::Audio { item },
        transform: None,
        effects: Vec::new(),
        parent: None,
        track_matte: TrackMatte::None,
        mask: None,
        cutout: None,
        text_path: None,
        switches: LayerSwitches { visible: false, audio: video.switches.audio, ..LayerSwitches::default() },
        ..video.clone()
    };
    composition.layers[index].switches.audio = false;
    let track = crate::tracks::make_room_at(composition, composition.layers.len(), true);
    composition.layers.push(Layer { track, ..audio });
    crate::tracks::normalize(composition);
    Some(id)
}

/// Sets a layer's sound settings, kept in range: volume within `LayerAudio::VOLUME_RANGE`, and
/// fades from zero to the layer's length.
pub fn set_layer_audio(composition: &mut Composition, layer_id: Id, edit: impl FnOnce(&mut crate::model::LayerAudio)) {
    let fps = composition.frame_rate.fps();
    let Some(layer) = composition.layer_mut(layer_id) else { return };
    let length = ((layer.out_frame - layer.in_frame) as f64 / fps) as f32;
    edit(&mut layer.audio);
    let audio = &mut layer.audio;
    let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
    audio.volume_db = finite(audio.volume_db).clamp(crate::model::LayerAudio::VOLUME_RANGE.0, crate::model::LayerAudio::VOLUME_RANGE.1);
    audio.fade_in = finite(audio.fade_in).clamp(0.0, length);
    audio.fade_out = finite(audio.fade_out).clamp(0.0, length);
    audio.duck_db = finite(audio.duck_db).clamp(crate::model::LayerAudio::DUCK_RANGE.0, crate::model::LayerAudio::DUCK_RANGE.1);
    audio.noise_gate = finite(audio.noise_gate).clamp(0.0, 100.0);
    if audio.duck_under == Some(layer_id) {
        audio.duck_under = None;
    }
}

/// Whether composition `outer` is, or holds somewhere inside it, composition `inner`.
pub fn contains_composition(project: &Project, outer: Id, inner: Id) -> bool {
    let mut pending = vec![outer];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        if id == inner {
            return true;
        }
        if !seen.insert(id) {
            continue;
        }
        let Some(composition) = project.composition(id) else { continue };
        pending.extend(composition.layers.iter().filter_map(|layer| match layer.content {
            LayerContent::Composition { composition } => Some(composition),
            _ => None,
        }));
    }
    false
}

/// Moves `ids` (with any track mattes they use) into a new composition named `name`, the
/// same size, rate and length as this one, and puts a layer showing it where they were, as
/// After Effects' Pre-compose does. The layers keep their timing. Parenting and masks that would
/// cross between the two compositions are let go. Returns the new composition's layer.
pub fn precompose(project: &mut Project, comp_id: Id, ids: &[Id], name: &str) -> Option<Id> {
    let parent = project.composition(comp_id)?;
    let moving: Vec<Id> = crate::copy::layers_to_copy(parent, ids).iter().map(|layer| layer.id).collect();
    let top = parent.layers.iter().position(|layer| moving.contains(&layer.id))?;
    let settings = CompositionSettings {
        name: name.to_owned(),
        width: parent.width,
        height: parent.height,
        frame_rate: parent.frame_rate,
        duration_frames: parent.duration_frames,
        background: Rgba::TRANSPARENT,
    };
    let inner_id = new_composition(project, &settings).ok()?;
    let parent = project.composition_mut(comp_id)?;
    let (mut moved, kept): (Vec<Layer>, Vec<Layer>) = std::mem::take(&mut parent.layers).into_iter().partition(|layer| moving.contains(&layer.id));
    parent.layers = kept;
    for layer in &mut parent.layers {
        if layer.parent.is_some_and(|p| moving.contains(&p)) {
            layer.parent = None;
        }
        if layer.mask.is_some_and(|mask| moving.contains(&mask.layer)) {
            layer.mask = None;
        }
        if layer.text_path.is_some_and(|on| moving.contains(&on.layer)) {
            layer.text_path = None;
        }
    }
    for layer in &mut moved {
        if layer.parent.is_some_and(|p| !moving.contains(&p)) {
            layer.parent = None;
        }
        if layer.mask.is_some_and(|mask| !moving.contains(&mask.layer)) {
            layer.mask = None;
        }
        if layer.text_path.is_some_and(|on| !moving.contains(&on.layer)) {
            layer.text_path = None;
        }
    }
    let inner = project.composition_mut(inner_id)?;
    inner.layers = moved;
    crate::tracks::normalize(inner);
    // The layer showing it, in the moved layers' place, for the whole composition.
    let layer_id = project.allocate_id();
    let item = project.item(inner_id)?.clone();
    let parent = project.composition_mut(comp_id)?;
    let mut layer = layer_from_item(&item, parent, 0, layer_id)?;
    let index = keep_matte_pairs(parent, top.min(parent.layers.len()));
    layer.track = crate::tracks::make_room_at(parent, index, false);
    parent.layers.insert(index, layer);
    crate::tracks::normalize(parent);
    Some(layer_id)
}

/// Where to insert a layer near `index` without coming between a layer and its track matte
/// (the layer directly above one that uses a matte): above the whole matte group instead.
fn keep_matte_pairs(composition: &Composition, mut index: usize) -> usize {
    while index > 0 && composition.layers.get(index).is_some_and(|below| below.track_matte != crate::model::TrackMatte::None) {
        index -= 1;
    }
    index
}

/// Adds the catalog effect `name`, with all its parameters at their defaults, to a layer.
/// Returns `None` for unknown effects or layers, and for effects that do nothing on that layer.
pub fn add_effect(project: &mut Project, comp_id: Id, layer_id: Id, name: &str) -> Option<Id> {
    let definition = crate::effects::find(name)?;
    // Refuse effects that would do nothing on this kind of layer (see `effects::suits`).
    let content = &project.composition(comp_id)?.layer(layer_id)?.content;
    if !crate::effects::suits_layer(name, content) {
        return None;
    }
    let id = project.allocate_id();
    let layer = project.composition_mut(comp_id)?.layer_mut(layer_id)?;
    layer.effects.push(crate::effects::instantiate(definition, id));
    Some(id)
}

/// Removes an effect from a layer.
pub fn remove_effect(composition: &mut Composition, layer_id: Id, effect_id: Id) {
    if let Some(layer) = composition.layer_mut(layer_id) {
        layer.effects.retain(|effect| effect.id != effect_id);
    }
}

/// Removes layers from a composition. Layers parented to a removed layer are unparented, and
/// layers it masked lose their mask.
pub fn delete_layers(composition: &mut Composition, ids: &[Id]) -> usize {
    let before = composition.layers.len();
    composition.layers.retain(|layer| !ids.contains(&layer.id));
    for layer in &mut composition.layers {
        if layer.parent.is_some_and(|parent| ids.contains(&parent)) {
            layer.parent = None;
        }
        if layer.mask.is_some_and(|mask| ids.contains(&mask.layer)) {
            layer.mask = None;
        }
        if layer.text_path.is_some_and(|on| ids.contains(&on.layer)) {
            layer.text_path = None;
        }
        if layer.audio.duck_under.is_some_and(|under| ids.contains(&under)) {
            layer.audio.duck_under = None;
        }
    }
    // Tracks left empty go away.
    crate::tracks::normalize(composition);
    before - composition.layers.len()
}

/// Everything deleting a set of project items would remove, for the confirmation message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemDeletion {
    /// The items themselves plus everything inside deleted folders.
    pub items: Vec<Id>,
    /// Layers in any composition that use one of those items.
    pub layers: usize,
    /// True if the set includes the composition that is open, which cannot be deleted.
    pub includes_open_composition: bool,
}

/// Works out what deleting `ids` would remove. Nothing is changed.
pub fn plan_item_deletion(project: &Project, ids: &[Id], open_composition: Id) -> ItemDeletion {
    let mut items: Vec<Id> = Vec::new();
    let mut pending: Vec<Id> = ids.to_vec();
    while let Some(id) = pending.pop() {
        if items.contains(&id) || project.item(id).is_none() {
            continue;
        }
        items.push(id);
        pending.extend(project.items.iter().filter(|item| item.parent == Some(id)).map(|item| item.id));
    }
    let layers = project
        .compositions
        .iter()
        .filter(|comp| !items.contains(&comp.id))
        .flat_map(|comp| &comp.layers)
        .filter(|layer| layer_source(layer).is_some_and(|source| items.contains(&source)))
        .count();
    ItemDeletion { includes_open_composition: items.contains(&open_composition), items, layers }
}

/// The project item a layer shows, if any.
fn layer_source(layer: &Layer) -> Option<Id> {
    match layer.content {
        LayerContent::Footage { item } | LayerContent::Image { item } | LayerContent::Audio { item } => Some(item),
        LayerContent::Composition { composition } => Some(composition),
        _ => None,
    }
}

/// Deletes the planned items, their compositions and every layer that used them.
/// Refuses (returns false) if the plan includes the open composition.
pub fn delete_items(project: &mut Project, plan: &ItemDeletion) -> bool {
    if plan.includes_open_composition {
        return false;
    }
    project.items.retain(|item| !plan.items.contains(&item.id));
    project.compositions.retain(|comp| !plan.items.contains(&comp.id));
    for composition in &mut project.compositions {
        let doomed: Vec<Id> = composition
            .layers
            .iter()
            .filter(|layer| layer_source(layer).is_some_and(|source| plan.items.contains(&source)))
            .map(|layer| layer.id)
            .collect();
        delete_layers(composition, &doomed);
    }
    true
}

/// The settings in the Composition Settings dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct CompositionSettings {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    pub duration_frames: i64,
    pub background: Rgba,
}

impl CompositionSettings {
    /// Largest frame size accepted, matching After Effects' 30,000-pixel limit.
    pub const MAX_SIZE: u32 = 30_000;

    /// Checks the settings, returning a message suitable for the dialog if they are invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Give the composition a name.".into());
        }
        if !(1..=Self::MAX_SIZE).contains(&self.width) || !(1..=Self::MAX_SIZE).contains(&self.height) {
            return Err(format!("Width and height must be between 1 and {} pixels.", Self::MAX_SIZE));
        }
        if self.frame_rate.numerator == 0 || self.frame_rate.denominator == 0 || self.frame_rate.fps() > 999.0 {
            return Err("Frame rate must be between 1 and 999 fps.".into());
        }
        if self.duration_frames < 1 {
            return Err("Duration must be at least one frame.".into());
        }
        Ok(())
    }
}

/// Creates a composition and its Project panel item. Returns its id.
pub fn new_composition(project: &mut Project, settings: &CompositionSettings) -> Result<Id, String> {
    settings.validate()?;
    let id = project.allocate_id();
    project.compositions.push(Composition {
        id,
        name: settings.name.trim().to_owned(),
        width: settings.width,
        height: settings.height,
        frame_rate: settings.frame_rate,
        duration_frames: settings.duration_frames,
        background: settings.background,
        layers: Vec::new(),
        markers: Vec::new(),
    });
    project.items.push(ProjectItem {
        id,
        name: settings.name.trim().to_owned(),
        kind: ProjectItemKind::Composition,
        label: crate::model::LabelColor::None,
        parent: None,
        size: Some(Vec2::new(settings.width as f32, settings.height as f32)),
        duration_seconds: Some(settings.duration_frames as f64 / settings.frame_rate.fps()),
        frame_rate: Some(settings.frame_rate),
        has_audio: false,
        color: None,
        path: None,
    });
    Ok(id)
}

/// Applies new settings to an existing composition. Layers keep their positions; the
/// work area and layers are not trimmed when the duration shrinks, as in After Effects.
pub fn apply_composition_settings(project: &mut Project, id: Id, settings: &CompositionSettings) -> Result<(), String> {
    settings.validate()?;
    let composition = project.composition_mut(id).ok_or("The composition no longer exists.")?;
    composition.name = settings.name.trim().to_owned();
    composition.width = settings.width;
    composition.height = settings.height;
    composition.frame_rate = settings.frame_rate;
    composition.duration_frames = settings.duration_frames;
    composition.background = settings.background;
    if let Some(item) = project.items.iter_mut().find(|item| item.id == id) {
        item.name = settings.name.trim().to_owned();
        item.size = Some(Vec2::new(settings.width as f32, settings.height as f32));
        item.duration_seconds = Some(settings.duration_frames as f64 / settings.frame_rate.fps());
        item.frame_rate = Some(settings.frame_rate);
    }
    Ok(())
}

/// Adds a text layer at the top of the stack, centered on `position` and starting at `in_frame`.
pub fn add_text_layer(project: &mut Project, comp_id: Id, text: &str, position: Vec2, in_frame: i64) -> Option<Id> {
    let style = TextStyle::default();
    let size = style.estimate_size(text);
    let name: String = text.lines().next().unwrap_or("Text").chars().take(40).collect();
    add_top_layer(project, comp_id, name, LayerContent::Text { text: text.to_owned(), style }, size, position, in_frame)
}

/// Adds a caption layer at the top of the stack, spanning `frames` of the composition, with
/// `words` timed from `start_frame` (where the sound they came from starts). It is as wide
/// as the composition and sits in its lower third, as captions do.
pub fn add_caption_layer(
    project: &mut Project,
    comp_id: Id,
    words: Vec<crate::captions::CaptionWord>,
    style: crate::captions::CaptionStyle,
    frames: std::ops::Range<i64>,
    start_frame: i64,
) -> Option<Id> {
    let composition = project.composition(comp_id)?;
    let size = Vec2::new(composition.width as f32, style.text.line_height * 2.5);
    let position = Vec2::new(composition.width as f32 * 0.5, composition.height as f32 * 0.8);
    let id = add_top_layer(project, comp_id, "Captions".into(), LayerContent::Captions { words, style }, size, position, frames.start)?;
    let layer = project.composition_mut(comp_id)?.layer_mut(id)?;
    layer.out_frame = frames.end.max(frames.start + 1);
    layer.start_frame = start_frame;
    Some(id)
}

/// Adds a shape layer of `size` at the top of the stack, centered on `position` and starting at
/// `in_frame`. Its name is the shape's, numbered after any shape layers already named that way.
pub fn add_shape_layer(project: &mut Project, comp_id: Id, style: ShapeStyle, size: Vec2, position: Vec2, in_frame: i64) -> Option<Id> {
    let base = style.kind.name();
    let taken = project.composition(comp_id)?.layers.iter().filter(|layer| layer.name.starts_with(base)).count();
    let name = format!("{base} {}", taken + 1);
    let size = Vec2::new(size.x.max(1.0), size.y.max(1.0));
    add_top_layer(project, comp_id, name, LayerContent::Shape { style, path: None }, size, position, in_frame)
}

/// Adds a rectangle the size of the frame, filled with `color`, behind everything else and for
/// the whole composition: a background that can be recolored like any shape. It goes on the
/// bottom picture track, moving the clips there up a track if it has any. Returns the layer.
pub fn add_background(project: &mut Project, comp_id: Id, color: Rgba) -> Option<Id> {
    let composition = project.composition(comp_id)?;
    let size = Vec2::new(composition.width as f32, composition.height as f32);
    let style = ShapeStyle { fill: Some(Paint::Solid(color)), ..ShapeStyle::new(ShapeKind::Rectangle) };
    let id = add_top_layer(project, comp_id, "Background".to_owned(), LayerContent::Shape { style, path: None }, size, size * 0.5, 0)?;
    let composition = project.composition_mut(comp_id)?;
    let main_in_use = composition.layers.iter().any(|layer| layer.id != id && layer.track == crate::tracks::MAIN_TRACK);
    for layer in &mut composition.layers {
        if layer.id == id {
            layer.track = crate::tracks::MAIN_TRACK;
        } else if main_in_use && layer.track >= crate::tracks::MAIN_TRACK {
            layer.track += 1;
        }
    }
    crate::tracks::normalize(composition);
    Some(id)
}

/// Adds a pen path drawn in composition pixels as a shape layer at the top of the stack. The
/// layer's box is fitted around the path and anchored at its center, so the shape appears
/// exactly where it was drawn. An open path gets a stroke and no fill; a closed one is filled.
/// Returns `None` for a path too short or broken to draw.
pub fn add_path_layer(project: &mut Project, comp_id: Id, path: &crate::path::ShapePath, style: ShapeStyle, in_frame: i64) -> Option<Id> {
    if !path.is_sound() {
        return None;
    }
    let (origin, size, fitted) = crate::path::fit_to_box(path)?;
    let mut style = ShapeStyle { kind: ShapeKind::Path, corner_radius: 0.0, ..style };
    if !path.closed {
        let paint = style.fill.unwrap_or(Paint::Solid(Rgba::hex(0xffffff)));
        style.stroke = Some(style.stroke.unwrap_or(crate::model::Stroke { paint, width: 4.0 }));
        style.fill = None;
    }
    let taken = project.composition(comp_id)?.layers.iter().filter(|layer| layer.name.starts_with("Path")).count();
    let center = origin + size * 0.5;
    add_top_layer(project, comp_id, format!("Path {}", taken + 1), LayerContent::Shape { style, path: Some(fitted) }, size, center, in_frame)
}

/// Moves a pen path's point (or one of its handles) to `to`, in the layer's own pixels, then
/// fits the layer's box around the path again without moving anything on screen. `part` picks
/// what moves: the point itself (its handles come with it) or one handle. Returns whether it
/// changed. Locked layers are left alone.
pub fn move_path_point(composition: &mut Composition, layer_id: Id, index: usize, part: PathPart, to: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let size = layer.size;
    let LayerContent::Shape { path: Some(path), .. } = &mut layer.content else { return false };
    if layer.switches.locked || index >= path.points.len() || !(to.x.is_finite() && to.y.is_finite()) {
        return false;
    }
    // Work in layer pixels, then fit the box again.
    let mut pixels = path.mapped(|v| Vec2::new(v.x * size.x, v.y * size.y));
    let point = &mut pixels.points[index];
    match part {
        PathPart::Point => point.at = to,
        PathPart::HandleIn => point.handle_in = to - point.at,
        PathPart::HandleOut => point.handle_out = to - point.at,
    }
    let Some((origin, new_size, fitted)) = crate::path::fit_to_box(&pixels) else { return false };
    *path = fitted;
    layer.size = new_size;
    // The layer's own pixels now start at `origin`: move the anchor point by the same amount,
    // so every point keeps its place on screen.
    if let Some(transform) = layer.transform.as_mut() {
        transform.anchor_point.map_values(|anchor| anchor - origin);
    }
    true
}

/// Opens or closes a pen path. Returns whether the layer is one.
pub fn set_path_closed(composition: &mut Composition, layer_id: Id, closed: bool) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let LayerContent::Shape { path: Some(path), .. } = &mut layer.content else { return false };
    path.closed = closed;
    true
}

/// Whether `path_layer` can carry `text_layer`'s text: a pen path in the same composition, and
/// a text layer to set along it.
pub fn can_set_text_on(composition: &Composition, text_layer: Id, path_layer: Id) -> bool {
    let text = composition.layer(text_layer).is_some_and(|layer| matches!(layer.content, LayerContent::Text { .. }));
    let path = composition.layer(path_layer).is_some_and(|layer| matches!(layer.content, LayerContent::Shape { path: Some(_), .. }));
    text && path && text_layer != path_layer
}

/// Sets a text layer along a pen path (or back on its own line, with `None`). Returns whether
/// it was set.
pub fn set_text_path(composition: &mut Composition, text_layer: Id, text_path: Option<crate::model::TextPath>) -> bool {
    if let Some(on) = text_path
        && (!can_set_text_on(composition, text_layer, on.layer) || !on.start.is_finite())
    {
        return false;
    }
    let Some(layer) = composition.layer_mut(text_layer) else { return false };
    layer.text_path = text_path;
    true
}

/// Replaces the markers labeled `label` with ones at `frames` (inside the composition), keeping
/// any others. Markers stay in frame order. Returns how many were added.
pub fn replace_markers(composition: &mut Composition, label: &str, frames: &[i64]) -> usize {
    composition.markers.retain(|marker| marker.label != label);
    let duration = composition.duration_frames;
    let mut added = 0;
    for &frame in frames.iter().filter(|frame| (0..duration).contains(*frame)) {
        if !composition.markers.iter().any(|marker| marker.frame == frame) {
            composition.markers.push(crate::model::Marker { frame, label: label.to_owned() });
            added += 1;
        }
    }
    composition.markers.sort_by_key(|marker| marker.frame);
    added
}

/// Which part of a path point is being moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPart {
    Point,
    HandleIn,
    HandleOut,
}

/// Sets (or clears) the layer that masks `layer_id`. A layer cannot mask itself, and the mask
/// must be in the same composition. Returns whether it was set.
pub fn set_layer_mask(composition: &mut Composition, layer_id: Id, mask: Option<crate::model::LayerMask>) -> bool {
    if mask.is_some_and(|mask| mask.layer == layer_id || composition.layer(mask.layer).is_none()) {
        return false;
    }
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    layer.mask = mask;
    true
}

/// Adds a layer showing `content` at the top of the stack, on a new track above everything,
/// centered on `position` and running from `in_frame` to the end of the composition.
fn add_top_layer(project: &mut Project, comp_id: Id, name: String, content: LayerContent, size: Vec2, position: Vec2, in_frame: i64) -> Option<Id> {
    let id = project.allocate_id();
    let composition = project.composition_mut(comp_id)?;
    // Anchored at the center of the box, so the layer scales and rotates in place.
    let transform = Transform::centered(size, position);
    let track = crate::tracks::make_room_at(composition, 0, false);
    let in_frame = in_frame.clamp(0, composition.duration_frames - 1);
    composition.layers.insert(
        0,
        Layer {
            id,
            name,
            label: crate::model::LabelColor::Red,
            content,
            size,
            in_frame,
            out_frame: composition.duration_frames,
            start_frame: in_frame,
            track,
            blend_mode: BlendMode::Normal,
            track_matte: TrackMatte::None,
            mask: None,
            cutout: None,
            text_path: None,
            parent: None,
            switches: LayerSwitches::default(),
            transform: Some(transform),
            effects: Vec::new(),
            has_audio: false,
            audio: Default::default(),
            time: Default::default(),
            animation: Default::default(),
        },
    );
    crate::tracks::normalize(composition);
    Some(id)
}

/// Changes a shape layer's style with `edit`, keeping the values in range: corner radius from
/// 0 to half the shorter side, 3 to `MAX_SHAPE_POINTS` sides or points, a star's inner radius
/// within `INNER_RADIUS_RANGE`, and strokes up to `MAX_STROKE_WIDTH`. A pen path stays a path
/// and a drawn shape cannot become one. Returns whether the layer is a shape.
pub fn edit_shape(composition: &mut Composition, layer_id: Id, edit: impl FnOnce(&mut ShapeStyle)) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    let size = layer.size;
    let LayerContent::Shape { style, path } = &mut layer.content else { return false };
    let kind_before = style.kind;
    edit(style);
    // A path shape is drawn from its outline: a box shape cannot become one, nor can a path
    // turn into a box shape and keep a stale outline.
    if (style.kind == ShapeKind::Path) != path.is_some() {
        style.kind = kind_before;
    }
    let finite_or = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
    style.corner_radius = finite_or(style.corner_radius, 0.0).clamp(0.0, ShapeStyle::max_corner_radius(size));
    style.points = style.points.clamp(3, MAX_SHAPE_POINTS);
    style.inner_radius = finite_or(style.inner_radius, 0.5).clamp(INNER_RADIUS_RANGE.0, INNER_RADIUS_RANGE.1);
    if let Some(stroke) = style.stroke.as_mut() {
        stroke.width = finite_or(stroke.width, 1.0).clamp(0.0, MAX_STROKE_WIDTH);
    }
    for paint in style.fill.iter_mut().chain(style.stroke.as_mut().map(|stroke| &mut stroke.paint)) {
        if let Paint::Linear { angle, .. } = paint {
            *angle = finite_or(*angle, 0.0).rem_euclid(360.0);
        }
    }
    true
}

/// Adds an image file to the project as an Image item of `size` pixels. Returns its id.
pub fn import_image(project: &mut Project, path: std::path::PathBuf, size: Vec2) -> Id {
    let id = project.allocate_id();
    let name = path.file_name().map_or_else(|| "Image".to_owned(), |name| name.to_string_lossy().into_owned());
    project.items.push(ProjectItem {
        id,
        name,
        kind: ProjectItemKind::Image,
        label: crate::model::LabelColor::Peach,
        parent: None,
        size: Some(size),
        duration_seconds: None,
        frame_rate: None,
        has_audio: false,
        color: None,
        path: Some(path),
    });
    id
}

/// Facts about a video file needed to add it to a project (read by the media layer).
#[derive(Clone, Debug, PartialEq)]
pub struct FootageInfo {
    pub size: Vec2,
    pub duration_seconds: f64,
    pub frame_rate: FrameRate,
    pub has_audio: bool,
}

/// Adds a video file to the project as a Footage item. Returns its id.
pub fn import_footage(project: &mut Project, path: std::path::PathBuf, info: &FootageInfo) -> Id {
    let id = project.allocate_id();
    let name = path.file_name().map_or_else(|| "Footage".to_owned(), |name| name.to_string_lossy().into_owned());
    project.items.push(ProjectItem {
        id,
        name,
        kind: ProjectItemKind::Footage,
        label: crate::model::LabelColor::Aqua,
        parent: None,
        size: Some(info.size),
        duration_seconds: Some(info.duration_seconds),
        frame_rate: Some(info.frame_rate),
        has_audio: info.has_audio,
        color: None,
        path: Some(path),
    });
    id
}

/// Adds a sound-only file (music, voice-over) to the project as an Audio item. Returns its id.
pub fn import_audio(project: &mut Project, path: std::path::PathBuf, duration_seconds: f64) -> Id {
    let id = project.allocate_id();
    let name = path.file_name().map_or_else(|| "Audio".to_owned(), |name| name.to_string_lossy().into_owned());
    project.items.push(ProjectItem {
        id,
        name,
        kind: ProjectItemKind::Audio,
        label: crate::model::LabelColor::Green,
        parent: None,
        size: None,
        duration_seconds: Some(duration_seconds),
        frame_rate: None,
        has_audio: true,
        color: None,
        path: Some(path),
    });
    id
}

/// Moves a layer's anchor point to `anchor` (in its own pixels) at `frame`, moving its position
/// to match so the layer stays exactly where it is on screen (After Effects' Pan Behind). Only
/// what the layer rotates and scales around changes. Locked layers and layers without a
/// transform are left alone. Returns whether it moved.
pub fn move_anchor(composition: &mut Composition, layer_id: Id, anchor: Vec2, frame: i64) -> bool {
    use crate::property::{PropertyPath, PropertyValue, TransformProperty};
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if layer.switches.locked || !(anchor.x.is_finite() && anchor.y.is_finite()) {
        return false;
    }
    let Some(local) = crate::transform::local_transform(layer, frame as f64) else { return false };
    let Some(transform) = layer.transform.as_ref() else { return false };
    let old_anchor = transform.anchor_point.value_at(frame as f64);
    let old_position = transform.position.value_at(frame as f64);
    // The anchor's step, measured in the parent's space (the layer's scale and rotation applied).
    let shift = local.apply_vector(anchor - old_anchor);
    layer.set_property(PropertyPath::Transform(TransformProperty::AnchorPoint), frame, PropertyValue::Pair(anchor));
    layer.set_property(PropertyPath::Transform(TransformProperty::Position), frame, PropertyValue::Pair(old_position + shift));
    true
}

/// Sets a text layer's box to its measured size, keeping the anchor point in the same relative spot.
/// Returns whether anything changed.
pub fn set_text_box(composition: &mut Composition, layer_id: Id, size: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if !matches!(layer.content, LayerContent::Text { .. }) || (layer.size - size).x.abs() + (layer.size - size).y.abs() < 0.01 {
        return false;
    }
    resize_box(layer, size);
    true
}

/// The largest shape box side, in layer pixels (the same limit as a composition's frame).
pub const MAX_SHAPE_SIZE: f32 = 30_000.0;

/// Sets a shape layer's size, from 1 to `MAX_SHAPE_SIZE` pixels a side. The anchor point keeps
/// its relative spot, so a centered shape grows from its center. Returns whether it is a shape.
pub fn set_shape_size(composition: &mut Composition, layer_id: Id, size: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if !matches!(layer.content, LayerContent::Shape { .. }) {
        return false;
    }
    let side = |value: f32| if value.is_finite() { value.clamp(1.0, MAX_SHAPE_SIZE) } else { 1.0 };
    resize_box(layer, Vec2::new(side(size.x), side(size.y)));
    // A smaller box may no longer fit the corner radius.
    edit_shape(composition, layer_id, |_| {})
}

/// Changes a layer's box to `size`, keeping its anchor point in the same relative spot.
fn resize_box(layer: &mut Layer, size: Vec2) {
    if let Some(transform) = layer.transform.as_mut() {
        let anchor = transform.anchor_point.value;
        let relative = Vec2::new(anchor.x / layer.size.x.max(1.0), anchor.y / layer.size.y.max(1.0));
        transform.anchor_point.value = Vec2::new(relative.x * size.x, relative.y * size.y);
    }
    layer.size = size;
}

/// Keeps a text style's size and spacing within the ranges the editor allows.
fn clamp_text_style(style: &mut TextStyle) {
    use crate::model::{FONT_SIZE_RANGE, LINE_HEIGHT_RANGE, TRACKING_RANGE};
    style.font_size = style.font_size.clamp(FONT_SIZE_RANGE.0, FONT_SIZE_RANGE.1);
    style.line_height = style.line_height.clamp(LINE_HEIGHT_RANGE.0, LINE_HEIGHT_RANGE.1);
    style.tracking = style.tracking.clamp(TRACKING_RANGE.0, TRACKING_RANGE.1);
}

/// Changes a text layer's text and/or style, keeping its size (and so its bounds) in step.
/// Caption layers take the same style edits (font, size, fill, stroke) for all their captions;
/// their words and box stay as they are.
pub fn edit_text(composition: &mut Composition, layer_id: Id, edit: impl FnOnce(&mut String, &mut TextStyle)) {
    let Some(layer) = composition.layer_mut(layer_id) else { return };
    if let LayerContent::Captions { style, .. } = &mut layer.content {
        edit(&mut String::new(), &mut style.text);
        clamp_text_style(&mut style.text);
        return;
    }
    if let LayerContent::Text { text, style } = &mut layer.content {
        edit(text, style);
        clamp_text_style(style);
        let size = style.estimate_size(text);
        // Keep the anchor in the same relative spot (the center, by default) as the box resizes.
        if let Some(transform) = layer.transform.as_mut() {
            let anchor = transform.anchor_point.value;
            let relative = Vec2::new(anchor.x / layer.size.x.max(1.0), anchor.y / layer.size.y.max(1.0));
            transform.anchor_point.value = Vec2::new(relative.x * size.x, relative.y * size.y);
        }
        layer.size = size;
        layer.name = text.lines().next().filter(|line| !line.is_empty()).unwrap_or("<empty text layer>").chars().take(40).collect();
    }
}

/// Whether `parent` can become `layer`'s parent: it exists, and it is neither the layer itself
/// nor one of its descendants (which would make a loop). Stops safely even if the existing
/// parent chain already loops, as a damaged project file could make it.
pub fn can_parent(composition: &Composition, layer: Id, parent: Id) -> bool {
    if composition.layer(parent).is_none() {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    let mut ancestor = Some(parent);
    while let Some(current) = ancestor {
        if current == layer || !seen.insert(current) {
            return false;
        }
        ancestor = composition.layer(current).and_then(|found| found.parent);
    }
    true
}

/// Sets a layer's parent. A layer cannot be parented to itself or to one of its own descendants.
///
/// As in After Effects, the layer stays where it is on screen: its position, rotation and scale are
/// re-expressed in the new parent's space, using the parents' transforms at `frame`.
pub fn set_parent(composition: &mut Composition, layer_id: Id, parent: Option<Id>, frame: i64) -> bool {
    if parent.is_some_and(|parent| !can_parent(composition, layer_id, parent)) {
        return false;
    }
    let at = frame as f64;
    let Some(layer) = composition.layer(layer_id) else { return false };
    let old_space = layer.parent.and_then(|old| crate::transform::world_transform(composition, old, at)).unwrap_or(Affine::IDENTITY);
    let new_space = parent.and_then(|new| crate::transform::world_transform(composition, new, at)).unwrap_or(Affine::IDENTITY);
    // Old parent space → composition → new parent space.
    let change = new_space.inverse().map(|inverse| old_space.then(inverse));

    let layer = composition.layer_mut(layer_id).expect("the layer exists");
    if let (Some(change), Some(transform)) = (change, layer.transform.as_mut()) {
        let turn = change.b.atan2(change.a).to_degrees();
        let stretch = Vec2::new(change.a.hypot(change.b), change.c.hypot(change.d));
        transform.position.map_values(|position| change.apply(position));
        transform.rotation.map_values(|rotation| rotation + turn);
        transform.scale.map_values(|scale| Vec2::new(scale.x * stretch.x, scale.y * stretch.y));
    }
    layer.parent = parent;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, ids};

    #[test]
    fn separating_audio_puts_the_sound_on_its_own_clip_in_step() {
        let mut project = sample::sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = import_footage(&mut project, "/media/interview.mp4".into(), &info);
        let video = add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 24).unwrap();
        let audio = separate_audio(&mut project, ids::MAIN_COMP, video).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let (v, a) = (comp.layer(video).unwrap(), comp.layer(audio).unwrap());
        assert!(matches!(a.content, LayerContent::Audio { item: i } if i == item));
        assert_eq!((a.in_frame, a.out_frame, a.start_frame), (v.in_frame, v.out_frame, v.start_frame), "in step with the video");
        assert!(!v.switches.audio && a.switches.audio, "the sound moved to the new clip");
        assert!(crate::tracks::is_sound_only(a) && a.track < 0, "on a sound track");
        let heard = crate::audio::composition_audio(&project, comp);
        assert_eq!(heard.iter().map(|clip| clip.layer).collect::<Vec<_>>(), vec![audio], "heard once, from the audio clip");
        assert!(separate_audio(&mut project, ids::MAIN_COMP, audio).is_none(), "an audio clip has nothing to separate");
    }

    #[test]
    fn sped_up_clips_keep_their_sound() {
        let mut project = sample::sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = import_footage(&mut project, "/media/interview.mp4".into(), &info);
        let video = add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 24).unwrap();
        assert!(crate::speed::set_speed(project.composition_mut(ids::MAIN_COMP).unwrap(), video, 50.0));
        let layers = project.composition(ids::MAIN_COMP).unwrap().layers.len();
        assert!(separate_audio(&mut project, ids::MAIN_COMP, video).is_none(), "audio clips have no speed, so it would not match");
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layers.len(), layers);
        assert!(comp.layer(video).unwrap().switches.audio, "the clip still plays its own sound");
    }

    #[test]
    fn precomposing_moves_layers_into_a_new_composition_in_their_place() {
        let mut project = sample::sample_project();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let (title, second) = (comp.layers[0].id, comp.layers[1].id);
        let count = comp.layers.len();
        let (in_frame, out_frame) = (comp.layers[0].in_frame, comp.layers[0].out_frame);
        let layer = precompose(&mut project, ids::MAIN_COMP, &[title, second], "Title Card").unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layers.len(), count - 1, "two layers became one");
        let LayerContent::Composition { composition } = comp.layer(layer).unwrap().content else { panic!() };
        let inner = project.composition(composition).unwrap();
        assert_eq!(inner.name, "Title Card");
        assert_eq!(inner.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![title, second], "in the same order");
        let moved = inner.layer(title).unwrap();
        assert_eq!((moved.in_frame, moved.out_frame), (in_frame, out_frame), "with their timing");
        assert!(project.item(composition).is_some(), "it is in the Project panel");
        // And it can't be put inside itself.
        assert!(add_layer_from_item(&mut project, composition, ids::MAIN_COMP, 0, 0).is_none());
        assert!(add_layer_from_item(&mut project, composition, composition, 0, 0).is_none());
    }

    #[test]
    fn moving_the_anchor_keeps_a_rotated_scaled_layer_in_place() {
        use crate::property::{PropertyPath, PropertyValue, TransformProperty};
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let layer = comp.layers[0].id;
        let target = comp.layer_mut(layer).unwrap();
        target.switches.locked = false;
        target.set_property(PropertyPath::Transform(TransformProperty::Rotation), 0, PropertyValue::Scalar(30.0));
        target.set_property(PropertyPath::Transform(TransformProperty::Scale), 0, PropertyValue::Pair(Vec2::new(150.0, 80.0)));
        let before = crate::transform::layer_corners(comp, layer, 0.0).unwrap();
        assert!(move_anchor(comp, layer, Vec2::new(0.0, 0.0), 0), "to the top left corner");
        let after = crate::transform::layer_corners(comp, layer, 0.0).unwrap();
        for (a, b) in before.iter().zip(after) {
            assert!((a.x - b.x).abs() < 0.01 && (a.y - b.y).abs() < 0.01, "the layer did not move: {before:?} vs {after:?}");
        }
        let world = crate::transform::world_transform(comp, layer, 0.0).unwrap();
        assert!((world.apply(Vec2::new(0.0, 0.0)) - before[0]).x.abs() < 0.01, "the anchor now sits on that corner");
    }

    #[test]
    fn footage_keeps_its_length_in_comp_frames() {
        let mut project = sample::sample_project();
        let id = add_layer_from_item(&mut project, ids::MAIN_COMP, ids::A_CAM, 2, 12).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let layer = &comp.layers[2];
        assert_eq!(layer.id, id);
        // 5.5 seconds at 24 fps.
        assert_eq!((layer.in_frame, layer.out_frame), (12, 12 + 132));
        assert!(layer.has_audio && layer.switches.audio);
    }

    #[test]
    fn stills_run_to_the_end_and_audio_has_no_picture() {
        let mut project = sample::sample_project();
        add_layer_from_item(&mut project, ids::MAIN_COMP, ids::LOGO, 0, 40).unwrap();
        add_layer_from_item(&mut project, ids::MAIN_COMP, ids::MUSIC, 99, 0).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layers[0].out_frame, comp.duration_frames);
        let music = comp.layers.last().unwrap();
        assert!(music.transform.is_none() && !music.switches.visible);
    }

    #[test]
    fn an_absurd_media_length_cannot_overflow_the_out_point() {
        let mut project = sample::sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 1.0e30, frame_rate: FrameRate::new(24, 1), has_audio: false };
        let item = import_footage(&mut project, "/media/odd.mp4".into(), &info);
        let layer = add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 10).unwrap();
        let layer = project.composition(ids::MAIN_COMP).unwrap().layer(layer).unwrap();
        assert!(layer.out_frame > layer.in_frame && layer.out_frame <= 10 + crate::model::MAX_FRAME);
    }

    #[test]
    fn parenting_refuses_loops_even_in_damaged_projects() {
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let (a, b, c) = (comp.layers[0].id, comp.layers[1].id, comp.layers[2].id);
        assert!(set_parent(comp, a, Some(b), 0));
        assert!(!can_parent(comp, b, a), "B can't follow its own child");
        assert!(!can_parent(comp, a, a));
        assert!(!can_parent(comp, a, Id(9999)), "a missing layer can't be a parent");
        // A loop already in the file (A → B → A): asking about C must still finish.
        comp.layer_mut(b).unwrap().parent = Some(a);
        assert!(!can_parent(comp, c, a));
        assert!(!set_parent(comp, c, Some(a), 0));
    }

    #[test]
    fn added_layers_get_a_track_of_their_own() {
        let mut project = sample::sample_project();
        // Between the title (track 5) and the grade layer (track 4).
        let added = add_layer_from_item(&mut project, ids::MAIN_COMP, ids::GRAY_SOLID, 1, 0).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        assert_eq!(comp.layers[1].id, added);
        let tracks: Vec<i32> = comp.layers.iter().map(|layer| layer.track).collect();
        assert_eq!(tracks, vec![6, 5, 4, 3, 2, 1, 0]);
        // Sound goes below the main track.
        let music = add_layer_from_item(&mut project, ids::MAIN_COMP, ids::MUSIC, 0, 0).unwrap();
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layer(music).unwrap().track, -1);
    }

    #[test]
    fn new_layers_never_split_a_layer_from_its_track_matte() {
        let mut project = sample::sample_project();
        // B-roll (stack position 3) uses the logo above it as its matte.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.layers[3].track_matte = crate::model::TrackMatte::Alpha;
        let (logo, b_roll) = (comp.layers[2].id, comp.layers[3].id);
        let added = add_layer_from_item(&mut project, ids::MAIN_COMP, ids::GRAY_SOLID, 3, 0).unwrap();
        let order: Vec<Id> = project.composition(ids::MAIN_COMP).unwrap().layers.iter().map(|l| l.id).collect();
        let at = |id: Id| order.iter().position(|x| *x == id).unwrap();
        assert_eq!(at(b_roll), at(logo) + 1, "the matte stays directly above");
        assert_eq!(at(added) + 1, at(logo), "the new layer goes above the pair");
    }

    #[test]
    fn imported_audio_files_become_audible_layers_without_a_picture() {
        let mut project = sample::sample_project();
        let item = import_audio(&mut project, "/media/voice.wav".into(), 3.0);
        let layer = add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 24).unwrap();
        let layer = project.composition(ids::MAIN_COMP).unwrap().layer(layer).unwrap();
        assert!(matches!(layer.content, LayerContent::Audio { .. }) && layer.transform.is_none());
        assert!(layer.has_audio && layer.switches.audio);
        assert_eq!((layer.in_frame, layer.out_frame), (24, 24 + 72));
    }

    #[test]
    fn folders_and_self_nesting_are_rejected() {
        let mut project = sample::sample_project();
        let before = project.composition(ids::MAIN_COMP).unwrap().layers.len();
        assert!(add_layer_from_item(&mut project, ids::MAIN_COMP, ids::FOOTAGE_FOLDER, 0, 0).is_none());
        assert!(add_layer_from_item(&mut project, ids::MAIN_COMP, ids::MAIN_COMP, 0, 0).is_none());
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layers.len(), before);
    }

    #[test]
    fn effects_come_from_the_catalog() {
        let mut project = sample::sample_project();
        let layer = project.composition(ids::MAIN_COMP).unwrap().layers[0].id;
        assert!(add_effect(&mut project, ids::MAIN_COMP, layer, "Gaussian Blur").is_some());
        assert!(add_effect(&mut project, ids::MAIN_COMP, layer, "Not An Effect").is_none());
        let added = &project.composition(ids::MAIN_COMP).unwrap().layers[0].effects[0];
        assert_eq!(added.properties[0].name, "Blurriness");
    }

    #[test]
    fn deleting_layers_unparents_children() {
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let (child, parent) = (comp.layers[0].id, comp.layers[1].id);
        set_parent(comp, child, Some(parent), 0);
        assert_eq!(delete_layers(comp, &[parent]), 1);
        assert_eq!(comp.layer(child).unwrap().parent, None);
    }

    #[test]
    fn deleting_a_folder_removes_its_items_and_their_layers() {
        let mut project = sample::sample_project();
        let plan = plan_item_deletion(&project, &[ids::FOOTAGE_FOLDER], ids::MAIN_COMP);
        // The folder, two clips, the logo and the music.
        assert_eq!(plan.items.len(), 5);
        // A-cam, B-roll and logo layers use those items.
        assert_eq!(plan.layers, 3);
        let before = project.composition(ids::MAIN_COMP).unwrap().layers.len();
        assert!(delete_items(&mut project, &plan));
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layers.len(), before - 3);
        assert!(project.item(ids::A_CAM).is_none());
    }

    #[test]
    fn the_open_composition_cannot_be_deleted() {
        let mut project = sample::sample_project();
        let plan = plan_item_deletion(&project, &[ids::MAIN_COMP], ids::MAIN_COMP);
        assert!(plan.includes_open_composition);
        assert!(!delete_items(&mut project, &plan));
        assert!(project.composition(ids::MAIN_COMP).is_some());
    }

    fn settings() -> CompositionSettings {
        CompositionSettings {
            name: "Vertical".into(),
            width: 1080,
            height: 1920,
            frame_rate: FrameRate::new(30, 1),
            duration_frames: 300,
            background: Rgba::hex(0x000000),
        }
    }

    #[test]
    fn compositions_can_be_created_and_changed() {
        let mut project = sample::sample_project();
        let id = new_composition(&mut project, &settings()).unwrap();
        assert_eq!(project.composition(id).unwrap().size(), Vec2::new(1080.0, 1920.0));
        assert_eq!(project.item(id).unwrap().kind, ProjectItemKind::Composition);
        let wide = CompositionSettings { width: 3840, height: 2160, ..settings() };
        apply_composition_settings(&mut project, id, &wide).unwrap();
        assert_eq!(project.item(id).unwrap().size, Some(Vec2::new(3840.0, 2160.0)));
    }

    #[test]
    fn invalid_settings_are_rejected() {
        let mut project = sample::sample_project();
        assert!(new_composition(&mut project, &CompositionSettings { width: 0, ..settings() }).is_err());
        assert!(new_composition(&mut project, &CompositionSettings { name: "  ".into(), ..settings() }).is_err());
        assert!(new_composition(&mut project, &CompositionSettings { duration_frames: 0, ..settings() }).is_err());
    }

    #[test]
    fn text_layers_resize_with_their_text() {
        let mut project = sample::sample_project();
        let id = add_text_layer(&mut project, ids::MAIN_COMP, "Hi", Vec2::new(960.0, 540.0), 0).unwrap();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let narrow = comp.layer(id).unwrap().size.x;
        edit_text(comp, id, |text, _| text.push_str(" there, everyone"));
        let layer = comp.layer(id).unwrap();
        assert!(layer.size.x > narrow);
        assert_eq!(layer.transform.as_ref().unwrap().anchor_point.value, layer.size * 0.5, "anchor stays centered");
        assert_eq!(layer.name, "Hi there, everyone");
    }

    #[test]
    fn parenting_keeps_the_layer_in_place_on_screen() {
        use crate::animation::Animated;
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let (child, parent) = (comp.layers[2].id, comp.layers[5].id);
        // Turn and shrink the parent first, so its space differs from the composition's.
        let parent_transform = comp.layer_mut(parent).unwrap().transform.as_mut().unwrap();
        parent_transform.rotation = Animated::constant(30.0);
        parent_transform.scale = Animated::constant(Vec2::new(50.0, 50.0));
        let before = crate::transform::layer_corners(comp, child, 100.0).unwrap();
        assert!(set_parent(comp, child, Some(parent), 100));
        let after = crate::transform::layer_corners(comp, child, 100.0).unwrap();
        for (b, a) in before.iter().zip(after) {
            assert!((b.x - a.x).abs() < 0.05 && (b.y - a.y).abs() < 0.05, "{before:?} vs {after:?}");
        }
        // Unparenting keeps it in place too.
        set_parent(comp, child, None, 100);
        let back = crate::transform::layer_corners(comp, child, 100.0).unwrap();
        assert!((back[0].x - before[0].x).abs() < 0.05 && (back[0].y - before[0].y).abs() < 0.05);
    }

    #[test]
    fn parenting_rejects_cycles() {
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let (a, b) = (comp.layers[0].id, comp.layers[1].id);
        assert!(set_parent(comp, a, Some(b), 0));
        assert!(!set_parent(comp, b, Some(a), 0), "b → a → b would be a cycle");
        assert!(!set_parent(comp, a, Some(a), 0));
    }

    /// A closed triangle drawn in composition pixels.
    fn triangle() -> crate::path::ShapePath {
        let corners = [(100.0, 100.0), (300.0, 100.0), (200.0, 260.0)];
        crate::path::ShapePath { points: corners.map(|(x, y)| crate::path::PathPoint::corner(Vec2::new(x, y))).to_vec(), closed: true }
    }

    #[test]
    fn a_pen_path_becomes_a_layer_exactly_where_it_was_drawn() {
        let mut project = sample::sample_project();
        let id = add_path_layer(&mut project, ids::MAIN_COMP, &triangle(), ShapeStyle::new(ShapeKind::Rectangle), 0).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let layer = comp.layer(id).unwrap();
        assert_eq!(layer.size, Vec2::new(200.0, 160.0));
        let corners = crate::transform::layer_corners(comp, id, 0.0).unwrap();
        assert_eq!((corners[0], corners[2]), (Vec2::new(100.0, 100.0), Vec2::new(300.0, 260.0)));
        let LayerContent::Shape { style, path: Some(path) } = &layer.content else { panic!("a path shape") };
        assert_eq!(style.kind, ShapeKind::Path);
        assert!(style.fill.is_some(), "closed paths are filled");
        assert_eq!(path.points[2].at, Vec2::new(0.5, 1.0));
        // An open path is a line: stroked, not filled.
        let open = crate::path::ShapePath { closed: false, ..triangle() };
        let line = add_path_layer(&mut project, ids::MAIN_COMP, &open, ShapeStyle::new(ShapeKind::Rectangle), 0).unwrap();
        let LayerContent::Shape { style, .. } = &project.composition(ids::MAIN_COMP).unwrap().layer(line).unwrap().content else { panic!() };
        assert!(style.fill.is_none() && style.stroke.is_some());
    }

    #[test]
    fn moving_a_path_point_refits_the_box_without_moving_the_rest() {
        let mut project = sample::sample_project();
        let id = add_path_layer(&mut project, ids::MAIN_COMP, &triangle(), ShapeStyle::new(ShapeKind::Rectangle), 0).unwrap();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        // Pull the bottom point further down and left, past the old box (layer pixels).
        assert!(move_path_point(comp, id, 2, PathPart::Point, Vec2::new(-50.0, 240.0)));
        let layer = comp.layer(id).unwrap();
        assert_eq!(layer.size, Vec2::new(250.0, 240.0));
        // The first point is still at (100, 100) in the composition.
        let world = crate::transform::world_transform(comp, id, 0.0).unwrap();
        let LayerContent::Shape { path: Some(path), .. } = &layer.content else { panic!() };
        let first = world.apply(Vec2::new(path.points[0].at.x * layer.size.x, path.points[0].at.y * layer.size.y));
        assert!((first.x - 100.0).abs() < 1e-3 && (first.y - 100.0).abs() < 1e-3, "{first:?}");
        assert!(!move_path_point(comp, id, 9, PathPart::Point, Vec2::default()), "no such point");
    }

    #[test]
    fn masks_must_be_another_layer_and_go_when_it_is_deleted() {
        use crate::model::{LayerMask, MaskMode};
        let mut project = sample::sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        let (layer, mask) = (comp.layers[2].id, comp.layers[0].id);
        assert!(!set_layer_mask(comp, layer, Some(LayerMask { layer, mode: MaskMode::Show })), "not itself");
        assert!(set_layer_mask(comp, layer, Some(LayerMask { layer: mask, mode: MaskMode::Hide })));
        delete_layers(comp, &[mask]);
        assert_eq!(comp.layer(layer).unwrap().mask, None);
    }

    #[test]
    fn a_background_fills_the_frame_behind_every_picture_layer() {
        let mut project = sample::sample_project();
        let background = add_background(&mut project, ids::MAIN_COMP, Rgba::hex(0xffffff)).unwrap();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let bottom = comp.layers.iter().rev().find(|layer| !crate::tracks::is_sound_only(layer)).unwrap();
        assert_eq!(bottom.id, background, "drawn first, under everything");
        let layer = comp.layer(background).unwrap();
        assert_eq!((layer.track, layer.in_frame, layer.out_frame), (crate::tracks::MAIN_TRACK, 0, comp.duration_frames));
        let bounds = crate::align::layer_bounds(comp, background, 0).unwrap();
        assert_eq!((bounds.min, bounds.max), (Vec2::new(0.0, 0.0), Vec2::new(1920.0, 1080.0)));
        // Nothing else shares its track.
        assert_eq!(comp.layers.iter().filter(|other| other.track == crate::tracks::MAIN_TRACK).count(), 1);
    }
}
