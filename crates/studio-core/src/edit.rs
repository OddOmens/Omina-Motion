//! Edits to a project. Every change the UI makes goes through a function here,
//! which keeps the rules in one tested place (and is where undo will hook in).

use crate::animation::Vec2;
use crate::transform::Affine;
use crate::model::{
    BlendMode, Composition, Id, Layer, LayerContent, LayerSwitches, Project, ProjectItem, ProjectItemKind, Rgba,
    TextStyle, TrackMatte, Transform,
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

    let duration = match item.duration_seconds {
        Some(seconds) => composition.frame_rate.seconds_to_frames(seconds),
        None => composition.duration_frames - in_frame,
    }
    .max(1);
    let size = item.size.unwrap_or(composition.size());
    let has_picture = content.has_picture();

    Some(Layer {
        id,
        name: item.name.clone(),
        label: item.label,
        size,
        in_frame,
        out_frame: in_frame + duration,
        start_frame: in_frame,
        blend_mode: BlendMode::Normal,
        track_matte: TrackMatte::None,
        parent: None,
        switches: LayerSwitches { visible: has_picture, audio: item.has_audio, ..LayerSwitches::default() },
        transform: has_picture.then(|| Transform::centered(size, composition.size() * 0.5)),
        effects: Vec::new(),
        has_audio: item.has_audio,
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
    let id = project.allocate_id();
    let item = project.item(item_id)?.clone();
    let composition = project.composition_mut(comp_id)?;
    let start = in_frame.clamp(0, composition.duration_frames - 1);
    let layer = layer_from_item(&item, composition, start, id)?;
    let index = stack_index.min(composition.layers.len());
    composition.layers.insert(index, layer);
    Some(id)
}

/// Adds the catalog effect `name`, with all its parameters at their defaults, to a layer.
/// Returns `None` for unknown effects or layers.
pub fn add_effect(project: &mut Project, comp_id: Id, layer_id: Id, name: &str) -> Option<Id> {
    let definition = crate::effects::find(name)?;
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

/// Removes layers from a composition. Layers parented to a removed layer are unparented.
pub fn delete_layers(composition: &mut Composition, ids: &[Id]) -> usize {
    let before = composition.layers.len();
    composition.layers.retain(|layer| !ids.contains(&layer.id));
    for layer in &mut composition.layers {
        if layer.parent.is_some_and(|parent| ids.contains(&parent)) {
            layer.parent = None;
        }
    }
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
    let id = project.allocate_id();
    let composition = project.composition_mut(comp_id)?;
    let style = TextStyle::default();
    let size = style.estimate_size(text);
    let mut transform = Transform::centered(size, position);
    // Text layers anchor at the center of their text box so they scale and rotate in place.
    transform.anchor_point = crate::animation::Animated::constant(size * 0.5);
    composition.layers.insert(
        0,
        Layer {
            id,
            name: text.lines().next().unwrap_or("Text").chars().take(40).collect(),
            label: crate::model::LabelColor::Red,
            content: LayerContent::Text { text: text.to_owned(), style },
            size,
            in_frame: in_frame.clamp(0, composition.duration_frames - 1),
            out_frame: composition.duration_frames,
            start_frame: in_frame.clamp(0, composition.duration_frames - 1),
            blend_mode: BlendMode::Normal,
            track_matte: TrackMatte::None,
            parent: None,
            switches: LayerSwitches::default(),
            transform: Some(transform),
            effects: Vec::new(),
            has_audio: false,
        },
    );
    Some(id)
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

/// Sets a text layer's box to its measured size, keeping the anchor point in the same relative spot.
/// Returns whether anything changed.
pub fn set_text_box(composition: &mut Composition, layer_id: Id, size: Vec2) -> bool {
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    if !matches!(layer.content, LayerContent::Text { .. }) || (layer.size - size).x.abs() + (layer.size - size).y.abs() < 0.01 {
        return false;
    }
    if let Some(transform) = layer.transform.as_mut() {
        let anchor = transform.anchor_point.value;
        let relative = Vec2::new(anchor.x / layer.size.x.max(1.0), anchor.y / layer.size.y.max(1.0));
        transform.anchor_point.value = Vec2::new(relative.x * size.x, relative.y * size.y);
    }
    layer.size = size;
    true
}

/// Changes a text layer's text and/or style, keeping its size (and so its bounds) in step.
pub fn edit_text(composition: &mut Composition, layer_id: Id, edit: impl FnOnce(&mut String, &mut TextStyle)) {
    let Some(layer) = composition.layer_mut(layer_id) else { return };
    if let LayerContent::Text { text, style } = &mut layer.content {
        edit(text, style);
        style.font_size = style.font_size.clamp(1.0, 2000.0);
        style.line_height = style.line_height.clamp(1.0, 4000.0);
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

/// Sets a layer's parent. A layer cannot be parented to itself or to one of its own descendants.
///
/// As in After Effects, the layer stays where it is on screen: its position, rotation and scale are
/// re-expressed in the new parent's space, using the parents' transforms at `frame`.
pub fn set_parent(composition: &mut Composition, layer_id: Id, parent: Option<Id>, frame: i64) -> bool {
    if let Some(parent_id) = parent {
        let mut ancestor = Some(parent_id);
        while let Some(current) = ancestor {
            if current == layer_id {
                return false;
            }
            ancestor = composition.layer(current).and_then(|layer| layer.parent);
        }
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
}
