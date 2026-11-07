//! The `.studio` project file: the project as JSON, plus where the editor was (open
//! composition, playhead, work area) and how it was set up (timeline view and zoom, open
//! rows, snapping and so on), so a project reopens exactly as it was left. Text keeps files readable and lets them diff cleanly
//! in version control. This module only converts to and from text; the app reads and
//! writes the file.
//!
//! Files are untrusted input: anything loaded is checked before the editor uses it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::edit::CompositionSettings;
use crate::effects;
use crate::model::{Id, Project};
use crate::property::PropertyInfo;
use crate::time::FrameRange;

/// The file name extension for project files.
pub const EXTENSION: &str = "studio";
/// Identifies a Studio project file.
const FORMAT: &str = "studio-project";
/// Bumped whenever the format changes in a way older versions cannot read.
pub const FORMAT_VERSION: u32 = 1;

/// Where the editor was when the project was saved.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EditorView {
    pub active_composition: Id,
    pub current_frame: i64,
    pub work_area: FrameRange,
}

/// How the editor was set up for this project. Files from before these were saved, or with
/// some missing, get the defaults for what is missing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorSettings {
    /// The timeline's zoom: the frames it showed. `None` shows the whole composition.
    pub view_range: Option<FrameRange>,
    pub snapping: bool,
    pub magnetic_main_track: bool,
    pub loop_playback: bool,
    /// Panel sections closed or opened (effects in Effect Controls).
    pub expanded_rows: Vec<String>,
    /// Clips opened in the Tracks view to show their keyframes.
    pub open_clips: Vec<Id>,
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            view_range: None,
            snapping: true,
            magnetic_main_track: true,
            loop_playback: true,
            expanded_rows: Vec::new(),
            open_clips: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    version: u32,
    view: EditorView,
    settings: &'a EditorSettings,
    project: &'a Project,
}

/// Read in two steps, so the format and version are checked before the rest is interpreted:
/// a file from a newer Studio then gets a clear message instead of a parse error.
#[derive(Deserialize)]
struct Header {
    format: String,
    version: u32,
}

#[derive(Deserialize)]
struct FileIn {
    view: EditorView,
    #[serde(default)]
    settings: EditorSettings,
    project: Project,
}

/// Writes a project as file text, with default editor settings.
pub fn to_text(project: &Project, view: EditorView) -> Result<String, String> {
    to_text_with(project, view, &EditorSettings::default())
}

/// Writes a project and how the editor was set up as file text. Fails only if the result
/// could not be read back, which is checked here so a save never produces a file that will
/// not open.
pub fn to_text_with(project: &Project, view: EditorView, settings: &EditorSettings) -> Result<String, String> {
    let file = FileOut { format: FORMAT, version: FORMAT_VERSION, view, settings, project };
    let text = serde_json::to_string_pretty(&file).map_err(|error| format!("The project could not be saved: {error}"))?;
    from_text(&text).map_err(|error| format!("The project could not be saved, because it would not open again: {error}"))?;
    Ok(text)
}

/// Reads file text into a checked project and editor view.
pub fn from_text(text: &str) -> Result<(Project, EditorView), String> {
    from_text_with(text).map(|(project, view, _)| (project, view))
}

/// Reads file text into a checked project, editor view and editor settings.
pub fn from_text_with(text: &str) -> Result<(Project, EditorView, EditorSettings), String> {
    let header: Header = serde_json::from_str(text).map_err(|error| {
        // A file cut short (an interrupted copy, a full disk) is a damaged project, not some other file.
        if error.is_eof() { "The project file is damaged: it ends too early.".to_owned() } else { "This is not a Studio project file.".to_owned() }
    })?;
    if header.format != FORMAT {
        return Err("This is not a Studio project file.".into());
    }
    if header.version > FORMAT_VERSION {
        return Err("This project was saved by a newer version of Studio. Update Studio to open it.".into());
    }
    let value: serde_json::Value = serde_json::from_str(text).map_err(|error| format!("The project file is damaged: {error}"))?;
    if has_oversized_number(&value) {
        return Err("The project file is damaged: it holds a number too large to use.".into());
    }
    let file: FileIn = serde_json::from_value(value).map_err(|error| format!("The project file is damaged: {error}"))?;
    let FileIn { mut project, mut view, mut settings } = file;
    restore_effect_info(&mut project);
    fill_missing_start_frames(&mut project);
    switch_on_nested_sound(&mut project);
    fill_missing_tracks(&mut project);
    check(&project)?;
    project.refresh_next_id();

    // Fall back to the first composition if the saved one is missing, and keep the playhead
    // and work area inside the composition.
    if project.composition(view.active_composition).is_none() {
        view.active_composition = project.compositions[0].id;
    }
    let duration = project.composition(view.active_composition).map_or(1, |comp| comp.duration_frames);
    view.current_frame = view.current_frame.clamp(0, duration - 1);
    let work_area = if view.work_area.start.is_finite() && view.work_area.end.is_finite() { view.work_area } else { FrameRange::new(0.0, duration as f64) };
    view.work_area = work_area.clamped(duration as f64, 1.0);
    // A zoom that doesn't fit the composition falls back to showing all of it.
    settings.view_range = settings
        .view_range
        .filter(|range| range.start.is_finite() && range.end.is_finite() && range.end > range.start)
        .map(|range| range.clamped(duration as f64, 1.0));
    // Clips opened for keyframes must still exist.
    let layers: std::collections::HashSet<Id> = project.compositions.iter().flat_map(|comp| &comp.layers).map(|layer| layer.id).collect();
    settings.open_clips.retain(|id| layers.contains(id));
    Ok((project, view, settings))
}

/// Ids are kept far below the top of their range, so new ones can always be allocated.
const MAX_ID: u64 = 1 << 53;
/// The longest media a project item may say it is, in seconds (about 30 years).
const MAX_MEDIA_SECONDS: f64 = 1.0e9;

/// Whether any number in the file is too large for the values it fills (32-bit floats), which
/// would load as infinity and spread through the math.
fn has_oversized_number(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => number.as_f64().is_some_and(|n| n.abs() > f64::from(f32::MAX)) && number.as_i64().is_none() && number.as_u64().is_none(),
        serde_json::Value::Array(items) => items.iter().any(has_oversized_number),
        serde_json::Value::Object(fields) => fields.values().any(has_oversized_number),
        _ => false,
    }
}

/// The frames a layer uses: its in, out and source start points and all its keyframes.
fn layer_frames(layer: &crate::model::Layer) -> Vec<i64> {
    let mut frames = vec![layer.in_frame, layer.out_frame, layer.start_frame];
    for path in layer.property_paths() {
        if let Some(property) = layer.property(path, 0) {
            frames.extend(property.keyframes);
        }
    }
    frames.extend(layer.morph.iter().flat_map(crate::morph::ShapeMorph::keyframe_frames));
    frames
}

/// Checks what the editor relies on, so a hand-edited or damaged file cannot crash it.
pub(crate) fn check(project: &Project) -> Result<(), String> {
    use crate::model::MAX_FRAME;
    let damaged = |what: &str| Err(format!("The project file is damaged: {what}."));
    if project.compositions.is_empty() {
        return Err("The project has no compositions.".into());
    }
    let mut item_ids = std::collections::HashSet::new();
    for item in &project.items {
        if !item_ids.insert(item.id) {
            return Err(format!("The project file is damaged: two items share the id {}.", item.id.0));
        }
        if item.frame_rate.is_some_and(|rate| rate.numerator == 0 || rate.denominator == 0) {
            return Err(format!("The project file is damaged: \"{}\" has no valid frame rate.", item.name));
        }
        if item.id.0 > MAX_ID {
            return damaged(&format!("\"{}\" has an id too large to use", item.name));
        }
        if item.duration_seconds.is_some_and(|seconds| !(0.0..=MAX_MEDIA_SECONDS).contains(&seconds)) {
            return damaged(&format!("\"{}\" has an impossible length", item.name));
        }
    }
    // Every item sits in a folder that exists, and following folders up always reaches the top
    // (a loop would hang anything that walks up the folders).
    for item in &project.items {
        let mut parent = item.parent;
        for _ in 0..=project.items.len() {
            let Some(folder) = parent else { break };
            match project.item(folder) {
                Some(found) if found.kind == crate::model::ProjectItemKind::Folder => parent = found.parent,
                _ => return damaged(&format!("\"{}\" is in a folder that doesn't exist", item.name)),
            }
        }
        if parent.is_some() {
            return damaged(&format!("\"{}\" is in folders that loop inside each other", item.name));
        }
    }
    for comp in &project.compositions {
        let settings = CompositionSettings {
            name: comp.name.clone(),
            width: comp.width,
            height: comp.height,
            frame_rate: comp.frame_rate,
            duration_frames: comp.duration_frames,
            background: comp.background,
        };
        settings.validate().map_err(|error| format!("Composition \"{}\" is damaged: {error}", comp.name))?;
        if comp.duration_frames > MAX_FRAME || comp.id.0 > MAX_ID {
            return damaged(&format!("composition \"{}\" is impossibly long", comp.name));
        }
        let mut layer_ids = std::collections::HashSet::new();
        for layer in &comp.layers {
            if !layer_ids.insert(layer.id) {
                return Err(format!("Composition \"{}\" is damaged: two layers share the id {}.", comp.name, layer.id.0));
            }
            if layer.out_frame <= layer.in_frame {
                return Err(format!("Layer \"{}\" is damaged: it ends before it starts.", layer.name));
            }
            if layer_frames(layer).iter().any(|frame| frame.abs() > MAX_FRAME) {
                return damaged(&format!("layer \"{}\" has a time too far from the start", layer.name));
            }
            if layer.id.0 > MAX_ID || layer.effects.iter().any(|effect| effect.id.0 > MAX_ID) {
                return damaged(&format!("layer \"{}\" has an id too large to use", layer.name));
            }
            // Sizes the renderer can draw (a pen path is drawn at its size, so a huge one would
            // need a huge picture).
            let side_ok = |side: f32| side.is_finite() && (0.0..=crate::model::MAX_LAYER_SIDE).contains(&side);
            if !(side_ok(layer.size.x) && side_ok(layer.size.y)) {
                return damaged(&format!("layer \"{}\" has an impossible size", layer.name));
            }
            // Text the renderer can lay out.
            let text_style = match &layer.content {
                crate::model::LayerContent::Text { style, .. } => Some(style),
                crate::model::LayerContent::Captions { style, .. } => Some(&style.text),
                _ => None,
            };
            if text_style.is_some_and(|style| !style.is_sound()) {
                return damaged(&format!("text layer \"{}\" has an impossible font size or spacing", layer.name));
            }
            // A composition shown inside itself, however deeply, would be drawn without end.
            if let crate::model::LayerContent::Composition { composition: inner } = layer.content
                && crate::edit::contains_composition(project, inner, comp.id)
            {
                return damaged(&format!("composition \"{}\" is shown inside itself", comp.name));
            }
            if let crate::model::LayerContent::Shape { style, .. } = &layer.content
                && !shape_is_sound(style)
            {
                return damaged(&format!("shape layer \"{}\" has impossible corners or points", layer.name));
            }
            // A pen path needs its outline, and only a pen path has one.
            if let crate::model::LayerContent::Shape { style, path } = &layer.content
                && (style.kind == crate::model::ShapeKind::Path) != path.as_ref().is_some_and(crate::path::ShapePath::is_sound)
            {
                return damaged(&format!("shape layer \"{}\" has a missing or broken path", layer.name));
            }
            // A morph is for a pen path, with a drawable outline and a sound style at every keyframe.
            if let Some(morph) = &layer.morph
                && (!matches!(&layer.content, crate::model::LayerContent::Shape { path: Some(_), .. })
                    || !morph.path.is_sound_with(crate::path::ShapePath::is_sound)
                    || !morph.style.is_sound_with(shape_is_sound))
            {
                return damaged(&format!("shape layer \"{}\" has a broken morph", layer.name));
            }
            // Background removal is for pictures, with a sane number of clicks on the picture.
            if let Some(cutout) = &layer.cutout
                && (!crate::cutout::can_cut_out(layer)
                    || cutout.clicks.len() > crate::model::MAX_CUTOUT_CLICKS
                    || cutout.clicks.iter().any(|click| !((0.0..=1.0).contains(&click.at.x) && (0.0..=1.0).contains(&click.at.y)))
                    || cutout.frames.is_some_and(|(start, end)| end <= start || start.abs() > MAX_FRAME || end.abs() > MAX_FRAME))
            {
                return damaged(&format!("layer \"{}\" has a damaged background removal", layer.name));
            }
            // Text set on a path needs a pen path in the same composition.
            if let Some(on) = layer.text_path
                && (!crate::edit::can_set_text_on(comp, layer.id, on.layer) || !on.start.is_finite())
            {
                return damaged(&format!("layer \"{}\" is set on a missing path", layer.name));
            }
            // A mask must be another layer of the same composition.
            if let Some(mask) = layer.mask
                && (mask.layer == layer.id || comp.layer(mask.layer).is_none())
            {
                return damaged(&format!("layer \"{}\" has a missing mask layer", layer.name));
            }
            if let crate::model::LayerContent::Captions { words, style } = &layer.content
                && (words.iter().any(|word| word.start.abs() > MAX_FRAME || word.end.abs() > MAX_FRAME || word.end < word.start)
                    || !(1..=crate::captions::MAX_WORDS_PER_PHRASE).contains(&style.words_per_phrase))
            {
                return damaged(&format!("caption layer \"{}\" has impossible timing", layer.name));
            }
            // Parents must exist in the same composition and never loop back.
            if let Some(parent) = layer.parent
                && !crate::edit::can_parent(comp, layer.id, parent)
            {
                return damaged(&format!("layer \"{}\" has a missing or looping parent", layer.name));
            }
        }
    }
    Ok(())
}

/// Whether a shape's numbers are ones the renderer can draw (finite, and in range).
fn shape_is_sound(style: &crate::model::ShapeStyle) -> bool {
    use crate::model::{INNER_RADIUS_RANGE, MAX_SHAPE_POINTS, MAX_STROKE_WIDTH, Paint};
    let paint_ok = |paint: &Paint| paint.gradient().is_none_or(|gradient| gradient.is_sound());
    style.corner_radius.is_finite()
        && style.corner_radius >= 0.0
        && (3..=MAX_SHAPE_POINTS).contains(&style.points)
        && (INNER_RADIUS_RANGE.0..=INNER_RADIUS_RANGE.1).contains(&style.inner_radius)
        && style.fill.as_ref().is_none_or(paint_ok)
        && style.stroke.as_ref().is_none_or(|stroke| paint_ok(&stroke.paint) && (0.0..=MAX_STROKE_WIDTH).contains(&stroke.width))
}

/// Files from before tracks: each layer gets its own track in its stacking order, so the file
/// looks exactly as it did. Then the stack is brought into track order (and empty tracks
/// closed up), in case a hand-edited file left it otherwise.
fn fill_missing_tracks(project: &mut Project) {
    for composition in &mut project.compositions {
        if composition.layers.iter().any(|layer| layer.track == crate::tracks::UNSET_TRACK) {
            crate::tracks::assign_tracks_by_stack(composition);
        }
        crate::tracks::normalize(composition);
    }
}

/// Files from before nested compositions played sound saved their layers with the audio
/// switch off, as it did nothing then; they are switched on, so their sound plays.
fn switch_on_nested_sound(project: &mut Project) {
    for layer in project.compositions.iter_mut().flat_map(|comp| &mut comp.layers) {
        if matches!(layer.content, crate::model::LayerContent::Composition { .. }) && !layer.has_audio {
            layer.has_audio = true;
            layer.switches.audio = true;
        }
    }
}

/// Files from before trimming have no start frames: their sources started at the in point.
fn fill_missing_start_frames(project: &mut Project) {
    for layer in project.compositions.iter_mut().flat_map(|comp| &mut comp.layers) {
        if layer.start_frame == crate::model::Layer::UNSET_START {
            layer.start_frame = layer.in_frame;
        }
    }
}

/// Effect parameters are saved without their display and range information; it comes from
/// the effect catalog. Parameters the catalog does not know keep an unlimited range.
fn restore_effect_info(project: &mut Project) {
    let effects = project.compositions.iter_mut().flat_map(|comp| &mut comp.layers).flat_map(|layer| &mut layer.effects);
    for effect in effects {
        let definition = effects::find(&effect.name);
        for (index, property) in effect.properties.iter_mut().enumerate() {
            let known = definition.and_then(|definition| definition.parameters.get(index)).map(|(info, _)| *info);
            property.info = known.filter(|info| info.name == property.name).unwrap_or(PropertyInfo::unknown());
        }
    }
}

/// Points the project's media files at where they are on this machine. Paths saved relative to
/// the project file (as File ▸ Package writes them) are made absolute against `project_folder`,
/// so a packaged project opens wherever it is unzipped; absolute paths are kept as they are.
/// Media files that are missing where the project says they are, but found under the same
/// name next to the project file, are pointed there. This keeps a project and its media
/// working after the folder holding both is moved or copied to another machine. Returns the
/// files that are still missing.
pub fn relink_missing_media(project: &mut Project, project_folder: &Path, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    for path in project.items.iter_mut().filter_map(|item| item.path.as_mut()) {
        if !relink(path, project_folder, &exists) {
            missing.push(path.clone());
        }
    }
    // LUT files chosen for Apply LUT effects are relinked the same way.
    let effects = project.compositions.iter_mut().flat_map(|comp| &mut comp.layers).flat_map(|layer| &mut layer.effects);
    for path in effects.filter_map(|effect| match effect.lut.as_mut() {
        Some(crate::model::LutSource::File(path)) => Some(path),
        _ => None,
    }) {
        if !relink(path, project_folder, &exists) && !missing.contains(path) {
            missing.push(path.clone());
        }
    }
    missing
}

/// Makes one linked path absolute (against the project's folder) and, if the file isn't there,
/// looks for it under the same name next to the project file. Returns whether it was found.
fn relink(path: &mut PathBuf, project_folder: &Path, exists: &impl Fn(&Path) -> bool) -> bool {
    if path.is_relative() {
        *path = project_folder.join(&*path);
    }
    if exists(path) {
        return true;
    }
    let beside = path.file_name().map(|name| project_folder.join(name));
    match beside.filter(|candidate| exists(candidate)) {
        Some(found) => {
            *path = found;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit;
    use crate::sample::{ids, sample_project};

    fn view() -> EditorView {
        EditorView { active_composition: ids::MAIN_COMP, current_frame: 42, work_area: FrameRange::new(24.0, 120.0) }
    }

    #[test]
    fn shapes_with_gradients_and_strokes_round_trip_and_old_fills_still_open() {
        use crate::model::{Paint, Rgba, ShapeKind, ShapeStyle, Stroke};
        let mut project = sample_project();
        let style = ShapeStyle {
            fill: Some(Paint::linear(Rgba::hex(0xff0000), Rgba::hex(0x0000ff), 45.0)),
            stroke: Some(Stroke { paint: Paint::radial(Rgba::hex(0xffffff), Rgba::hex(0x000000)), width: 6.0 }),
            ..ShapeStyle::new(ShapeKind::Star)
        };
        edit::add_shape_layer(&mut project, ids::MAIN_COMP, style, crate::animation::Vec2::new(200.0, 200.0), crate::animation::Vec2::new(500.0, 500.0), 0).unwrap();
        let text = to_text(&project, view()).unwrap();
        let (opened, _) = from_text(&text).unwrap();
        assert_eq!(opened, project);

        // A shape from before gradients: the fill is a bare color, with no stroke or inner radius.
        let old: ShapeStyle = serde_json::from_str(r#"{"kind":"Star","fill":{"r":1.0,"g":0.0,"b":0.0,"a":1.0},"corner_radius":0.0,"points":5}"#).unwrap();
        assert_eq!(old.fill, Some(Paint::Solid(Rgba::hex(0xff0000))));
        assert_eq!((old.stroke, old.inner_radius), (None, 0.5));
    }

    #[test]
    fn a_saved_project_opens_unchanged() {
        let mut project = sample_project();
        // Cover effects, keyframes, text and parenting: the sample already has all of these.
        let title = project.compositions[0].layers[0].id;
        edit::add_effect(&mut project, ids::MAIN_COMP, title, "Gaussian Blur").unwrap();
        let text = to_text(&project, view()).unwrap();
        let (opened, opened_view) = from_text(&text).unwrap();
        assert_eq!(opened, project);
        assert_eq!(opened_view, view());
    }

    #[test]
    fn ids_stay_unique_after_opening() {
        let mut project = sample_project();
        let (mut opened, _) = from_text(&to_text(&project, view()).unwrap()).unwrap();
        assert_eq!(opened.allocate_id(), project.allocate_id());
        // No new id may collide with an effect's (the sample's effects have the highest ids).
        let highest_effect = opened.compositions[0].layers.iter().flat_map(|l| &l.effects).map(|e| e.id).max().unwrap();
        assert!(opened.allocate_id() > highest_effect);
    }

    #[test]
    fn other_files_and_newer_versions_are_refused_clearly() {
        assert_eq!(from_text("hello").unwrap_err(), "This is not a Studio project file.");
        assert_eq!(from_text(r#"{"format": "something-else", "version": 1}"#).unwrap_err(), "This is not a Studio project file.");
        let newer = to_text(&sample_project(), view()).unwrap().replacen("\"version\": 1", "\"version\": 99", 1);
        assert!(from_text(&newer).unwrap_err().contains("newer version"));
    }

    #[test]
    fn damaged_files_are_refused_not_trusted() {
        let text = to_text(&sample_project(), view()).unwrap();
        let zero_rate = text.replacen("\"numerator\": 24", "\"numerator\": 0", 1);
        assert!(from_text(&zero_rate).is_err());
        assert!(from_text(&text[..text.len() / 2]).unwrap_err().contains("damaged"));
    }

    /// The sample's first layer (the title) in a saved file.
    fn title(file: &mut serde_json::Value) -> &mut serde_json::Value {
        &mut file["project"]["compositions"][0]["layers"][0]
    }

    /// The title's text style in a saved file.
    fn style(file: &mut serde_json::Value) -> &mut serde_json::Value {
        &mut title(file)["content"]["Text"]["style"]
    }

    /// The sample saved as JSON, changed by `damage`, then opened.
    fn open_damaged(damage: impl FnOnce(&mut serde_json::Value)) -> Result<(Project, EditorView), String> {
        let mut file: serde_json::Value = serde_json::from_str(&to_text(&sample_project(), view()).unwrap()).unwrap();
        damage(&mut file);
        from_text(&file.to_string())
    }

    #[test]
    fn hostile_values_are_refused_instead_of_crashing_later() {
        // A parent loop (title ↔ grade), and a parent that doesn't exist.
        let looped = open_damaged(|file| {
            let grade = file["project"]["compositions"][0]["layers"][1]["id"].clone();
            let title_id = title(file)["id"].clone();
            title(file)["parent"] = grade;
            file["project"]["compositions"][0]["layers"][1]["parent"] = title_id;
        });
        assert!(looped.unwrap_err().contains("parent"));
        assert!(open_damaged(|file| title(file)["parent"] = serde_json::json!(424242)).unwrap_err().contains("parent"));
        // Frames at the edge of the integer range, an id with no room above it, a huge length, a huge float.
        assert!(open_damaged(|file| title(file)["out_frame"] = serde_json::json!(i64::MAX)).is_err());
        assert!(open_damaged(|file| title(file)["transform"]["opacity"]["keyframes"][0]["frame"] = serde_json::json!(-i64::MAX)).is_err());
        assert!(open_damaged(|file| title(file)["id"] = serde_json::json!(u64::MAX)).is_err());
        assert!(open_damaged(|file| file["project"]["items"][3]["duration_seconds"] = serde_json::json!(1e30)).is_err());
        assert!(open_damaged(|file| title(file)["size"]["x"] = serde_json::json!(1e300)).unwrap_err().contains("too large"));
    }

    #[test]
    fn text_sizes_layer_sizes_and_nesting_loops_that_would_stop_the_renderer_are_refused() {
        // A line height of 0 stops text layout; a negative one never finishes.
        for line_height in [0.0, -10.0] {
            assert!(open_damaged(|file| style(file)["line_height"] = serde_json::json!(line_height)).unwrap_err().contains("font size or spacing"));
        }
        // A glyph thousands of times the frame.
        assert!(open_damaged(|file| style(file)["font_size"] = serde_json::json!(200_000.0)).is_err());
        assert!(open_damaged(|file| style(file)["tracking"] = serde_json::json!(-5000.0)).is_err());
        // A layer box ten million pixels across.
        assert!(open_damaged(|file| title(file)["size"] = serde_json::json!([1e7, 1e7])).unwrap_err().contains("impossible size"));
        // A composition that shows itself.
        let shown_inside_itself = open_damaged(|file| {
            let main = file["project"]["compositions"][0]["id"].clone();
            title(file)["content"] = serde_json::json!({ "Composition": { "composition": main } });
        });
        assert!(shown_inside_itself.unwrap_err().contains("inside itself"));
        // The sample itself is fine.
        assert!(open_damaged(|_| {}).is_ok());
    }

    #[test]
    fn keyframes_out_of_order_are_sorted_on_opening() {
        let text = to_text(&sample_project(), view()).unwrap();
        let mut file: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Reverse the title's position keyframes, as a hand edit might.
        let keys = &mut file["project"]["compositions"][0]["layers"][0]["transform"]["position"]["keyframes"];
        keys.as_array_mut().unwrap().reverse();
        let (opened, _) = from_text(&file.to_string()).unwrap();
        let frames = opened.compositions[0].layers[0].transform.as_ref().unwrap().position.keyframe_frames();
        assert!(frames.windows(2).all(|pair| pair[0] < pair[1]), "{frames:?}");
    }

    #[test]
    fn files_from_before_trimming_start_sources_at_the_in_point() {
        let text = to_text(&sample_project(), view()).unwrap();
        let mut file: serde_json::Value = serde_json::from_str(&text).unwrap();
        for layer in file["project"]["compositions"][0]["layers"].as_array_mut().unwrap() {
            layer.as_object_mut().unwrap().remove("start_frame");
        }
        let (opened, _) = from_text(&file.to_string()).unwrap();
        assert!(opened.compositions[0].layers.iter().all(|layer| layer.start_frame == layer.in_frame));
    }

    #[test]
    fn nested_compositions_in_older_files_get_their_sound_switched_on() {
        let mut project = sample_project();
        let inner = crate::edit::new_composition(&mut project, &crate::edit::CompositionSettings {
            name: "Inner".into(),
            width: 320,
            height: 180,
            frame_rate: crate::time::FrameRate::new(30, 1),
            duration_frames: 60,
            background: crate::model::Rgba::TRANSPARENT,
        }).unwrap();
        let shown = crate::edit::add_layer_from_item(&mut project, crate::sample::ids::MAIN_COMP, inner, 0, 0).unwrap();
        // Saved as older versions saved it: no sound, switch off.
        let layer = project.composition_mut(crate::sample::ids::MAIN_COMP).unwrap().layer_mut(shown).unwrap();
        layer.has_audio = false;
        layer.switches.audio = false;
        let (opened, _) = from_text(&to_text(&project, view()).unwrap()).unwrap();
        let layer = opened.composition(crate::sample::ids::MAIN_COMP).unwrap().layer(shown).unwrap();
        assert!(layer.has_audio && layer.switches.audio);
        // Muted on purpose after that, it stays muted.
        let mut muted = opened.clone();
        muted.composition_mut(crate::sample::ids::MAIN_COMP).unwrap().layer_mut(shown).unwrap().switches.audio = false;
        let (reopened, _) = from_text(&to_text(&muted, view()).unwrap()).unwrap();
        assert!(!reopened.composition(crate::sample::ids::MAIN_COMP).unwrap().layer(shown).unwrap().switches.audio);
    }

    #[test]
    fn files_from_before_tracks_get_one_track_per_layer() {
        let text = to_text(&sample_project(), view()).unwrap();
        let mut file: serde_json::Value = serde_json::from_str(&text).unwrap();
        for layer in file["project"]["compositions"][0]["layers"].as_array_mut().unwrap() {
            layer.as_object_mut().unwrap().remove("track");
        }
        let (opened, _) = from_text(&file.to_string()).unwrap();
        let tracks: Vec<i32> = opened.compositions[0].layers.iter().map(|layer| layer.track).collect();
        assert_eq!(tracks, vec![5, 4, 3, 2, 1, 0], "stacking order is kept exactly");
    }

    /// Builds a project using as much of the editor as possible (every template in every
    /// format with its placeholders filled, every mockup, element and effect, layer and text
    /// animations, a shape morph, a transition and sound settings) so that anything the
    /// project file fails to keep shows up as a difference after reopening.
    #[test]
    fn a_project_using_every_feature_reopens_exactly_as_saved() {
        use crate::animation::Vec2;
        use crate::model::{LayerContent, ShapeKind, ShapeStyle};
        use crate::{effects, elements, magic_move, mockups, morph, presets, slots, templates, text_animation, transitions};

        let mut project = sample_project();
        let picture = edit::import_image(&mut project, "/media/photo.png".into(), Vec2::new(1200.0, 800.0));

        // Every template in every format, with each placeholder filled as a drop would.
        let mut built = Vec::new();
        for template in &templates::TEMPLATES {
            for format in templates::Format::ALL {
                let comp = templates::build(&mut project, template, format).unwrap();
                let placeholders: Vec<Id> = project.composition(comp).unwrap().layers.iter().filter(|layer| slots::is_placeholder(layer)).map(|layer| layer.id).collect();
                for layer in placeholders {
                    let slot = slots::Slot { comp, layer, kind: slots::SlotKind::Placeholder, root: layer, outline: [Vec2::default(); 4] };
                    slots::fill(&mut project, &slot, picture).unwrap();
                }
                built.push(comp);
            }
        }

        // Every mockup device in each of its finishes, holding the picture.
        for device in mockups::Device::ALL {
            for (_, finish) in device.finishes() {
                let request = mockups::MockupRequest { device, orientation: mockups::Orientation::default(), finish, footage: Some(picture), seconds: 5.0 };
                mockups::add_to(&mut project, ids::MAIN_COMP, &request).unwrap();
            }
        }

        // Every element, then every effect on each layer it suits.
        for index in 0..elements::ELEMENTS.len() {
            elements::add(&mut project, ids::MAIN_COMP, index, Vec2::new(400.0, 300.0), 0);
        }
        let layers: Vec<(Id, LayerContent)> = project.composition(ids::MAIN_COMP).unwrap().layers.iter().map(|layer| (layer.id, layer.content.clone())).collect();
        for definition in effects::CATEGORIES.iter().flat_map(|category| category.effects) {
            if let Some((layer, _)) = layers.iter().find(|(_, content)| effects::suits_layer(definition.name, content)) {
                edit::add_effect(&mut project, ids::MAIN_COMP, *layer, definition.name).unwrap();
            }
        }

        // Layer animations, a text animation, a shape morph and a transition.
        let text = edit::add_text_layer(&mut project, ids::MAIN_COMP, "Saved", Vec2::new(960.0, 540.0), 0).unwrap();
        presets::set_motion(&mut project, ids::MAIN_COMP, text, presets::Phase::In, Some(presets::Motion::Pop), 0.5);
        presets::set_motion(&mut project, ids::MAIN_COMP, text, presets::Phase::Out, Some(presets::Motion::Fade), 0.5);
        presets::set_loop(&mut project, ids::MAIN_COMP, text, Some(presets::Loop::Wiggle));
        assert!(text_animation::set_animator(&mut project, ids::MAIN_COMP, text, presets::Phase::In, Some(text_animation::TextAnimator::default())));
        let circle = edit::add_shape_layer(&mut project, ids::MAIN_COMP, ShapeStyle::new(ShapeKind::Ellipse), Vec2::new(200.0, 200.0), Vec2::new(300.0, 300.0), 0).unwrap();
        let star = edit::add_shape_layer(&mut project, ids::MAIN_COMP, ShapeStyle::new(ShapeKind::Star), Vec2::new(200.0, 200.0), Vec2::new(900.0, 300.0), 0).unwrap();
        morph::morph_to(project.composition_mut(ids::MAIN_COMP).unwrap(), circle, star, 10, 20, morph::default_easing()).unwrap();
        // A transition goes on a cut between touching clips, as the timeline offers it: split a
        // clip and put one on the cut.
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let clip = composition.layers.iter().find(|layer| matches!(layer.content, LayerContent::Footage { .. })).unwrap();
        let (clip, middle) = (clip.id, (clip.in_frame + clip.out_frame) / 2);
        crate::cut::split_layers(&mut project, ids::MAIN_COMP, &[clip], middle);
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let after = transitions::touching(composition, composition.layer(clip).unwrap(), false).unwrap();
        assert!(transitions::apply(&mut project, ids::MAIN_COMP, Some(clip), Some(after), transitions::Transition::Whip, 0.5));
        if let Some(&to) = magic_move::targets(&project, built[0]).first() {
            magic_move::magic_move(&mut project, built[0], to, 15, morph::default_easing()).unwrap();
        }

        // Sound settings on a clip.
        let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layers.iter_mut().find(|layer| layer.has_audio).unwrap();
        layer.audio.volume_db = -6.5;
        layer.audio.noise_gate = 0.3;
        layer.audio.fade_in = 0.25;
        layer.switches.audio = false;

        let settings = EditorSettings {
            view_range: Some(FrameRange::new(10.0, 90.0)),
            snapping: false,
            magnetic_main_track: false,
            loop_playback: false,
            expanded_rows: vec!["effect-1".into()],
            open_clips: vec![text],
        };
        let saved = to_text_with(&project, view(), &settings).unwrap();
        let (opened, opened_view, opened_settings) = from_text_with(&saved).unwrap();
        assert_eq!(opened_view, view());
        assert_eq!(opened_settings, settings);
        // Compare composition by composition so a failure names what changed.
        assert_eq!(opened.items.len(), project.items.len());
        for (after, before) in opened.items.iter().zip(&project.items) {
            assert_eq!(after, before, "project item {:?} changed after reopening", before.name);
        }
        assert_eq!(opened.compositions.len(), project.compositions.len());
        for (after, before) in opened.compositions.iter().zip(&project.compositions) {
            assert_eq!(after, before, "composition {:?} changed after reopening", before.name);
        }
        assert_eq!(opened, project);
        // Saving the reopened project gives the same file, so nothing drifts save after save.
        assert_eq!(to_text_with(&opened, view(), &settings).unwrap(), saved);
    }

    #[test]
    fn editor_settings_are_saved_and_older_files_get_defaults() {
        let settings = EditorSettings {
            view_range: Some(FrameRange::new(24.0, 96.0)),
            snapping: false,
            magnetic_main_track: false,
            loop_playback: false,
            expanded_rows: vec!["layer:100".into()],
            open_clips: vec![Id(100), Id(424242)],
        };
        let text = to_text_with(&sample_project(), view(), &settings).unwrap();
        let (_, _, opened) = from_text_with(&text).unwrap();
        assert_eq!(opened, EditorSettings { open_clips: vec![Id(100)], ..settings }, "a clip that no longer exists is dropped");
        // A file without settings (from before they were saved) opens with the defaults.
        let (_, _, defaults) = from_text_with(&to_text(&sample_project(), view()).unwrap().replace("\"settings\"", "\"unused\"")).unwrap();
        assert_eq!(defaults, EditorSettings::default());
    }

    #[test]
    fn the_view_is_kept_inside_the_composition() {
        let outside = EditorView { active_composition: Id(9999), current_frame: 10_000, work_area: FrameRange::new(-50.0, 9000.0) };
        let (_, opened) = from_text(&to_text(&sample_project(), outside).unwrap()).unwrap();
        assert_eq!(opened.active_composition, ids::MAIN_COMP);
        assert_eq!(opened.current_frame, 239);
        assert_eq!(opened.work_area, FrameRange::new(0.0, 240.0));
    }

    #[test]
    fn missing_media_is_found_next_to_the_project() {
        let mut project = sample_project();
        let info = edit::FootageInfo {
            size: crate::animation::Vec2::new(64.0, 36.0),
            duration_seconds: 2.0,
            frame_rate: crate::time::FrameRate::new(24, 1),
            has_audio: false,
        };
        edit::import_footage(&mut project, "/old/machine/clip.mp4".into(), &info);
        edit::import_audio(&mut project, "/old/machine/gone.wav".into(), 1.0);
        let exists = |path: &Path| path == Path::new("/new/place/clip.mp4");
        let missing = relink_missing_media(&mut project, Path::new("/new/place"), exists);
        assert_eq!(missing, vec![PathBuf::from("/old/machine/gone.wav")]);
        assert!(project.items.iter().any(|item| item.path.as_deref() == Some(Path::new("/new/place/clip.mp4"))));
    }

    #[test]
    fn lut_choices_are_saved_and_their_files_relinked() {
        use crate::model::LutSource;
        let mut project = sample_project();
        let grade = edit::ensure_color_grade_layer(&mut project, ids::MAIN_COMP).unwrap();
        edit::add_lut(&mut project, ids::MAIN_COMP, grade, LutSource::Look("Noir".into())).unwrap();
        edit::add_lut(&mut project, ids::MAIN_COMP, grade, LutSource::File("/old/machine/Film.cube".into())).unwrap();
        let (mut opened, _) = from_text(&to_text(&project, view()).unwrap()).unwrap();
        assert_eq!(opened, project);
        let exists = |path: &Path| path == Path::new("/new/place/Film.cube");
        assert!(relink_missing_media(&mut opened, Path::new("/new/place"), exists).is_empty());
        let luts: Vec<_> = opened.compositions[0].layers.iter().flat_map(|layer| &layer.effects).filter_map(|effect| effect.lut.clone()).collect();
        assert_eq!(luts, vec![LutSource::Look("Noir".into()), LutSource::File("/new/place/Film.cube".into())]);
    }

    #[test]
    fn paths_relative_to_the_project_file_open_wherever_it_is() {
        let mut project = sample_project();
        edit::import_audio(&mut project, "Media/score.wav".into(), 1.0);
        edit::import_audio(&mut project, "/absolute/voice.wav".into(), 1.0);
        let exists = |path: &Path| path == Path::new("/unzipped/Promo/Media/score.wav") || path == Path::new("/absolute/voice.wav");
        assert!(relink_missing_media(&mut project, Path::new("/unzipped/Promo"), exists).is_empty());
        let paths: Vec<_> = project.items.iter().filter_map(|item| item.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("/unzipped/Promo/Media/score.wav"), PathBuf::from("/absolute/voice.wav")], "absolute paths are kept");
        // A relative path that isn't there is reported as the full path it was looked for at.
        let mut project = sample_project();
        edit::import_audio(&mut project, "Media/gone.wav".into(), 1.0);
        assert_eq!(relink_missing_media(&mut project, Path::new("/unzipped/Promo"), |_| false), [PathBuf::from("/unzipped/Promo/Media/gone.wav")]);
    }

    #[test]
    fn folders_inside_each_other_in_a_loop_are_refused() {
        let mut project = crate::sample::sample_project();
        let a = crate::folders::add_folder(&mut project, "A", None);
        let b = crate::folders::add_folder(&mut project, "B", Some(a));
        project.items.iter_mut().find(|item| item.id == a).unwrap().parent = Some(b);
        let view = EditorView { active_composition: crate::sample::ids::MAIN_COMP, current_frame: 0, work_area: FrameRange::new(0.0, 10.0) };
        let error = to_text(&project, view).unwrap_err();
        assert!(error.contains("folder"), "{error}");
        // So is an item inside something that isn't a folder.
        let mut project = crate::sample::sample_project();
        project.items.iter_mut().find(|item| item.id == crate::sample::ids::LOGO).unwrap().parent = Some(crate::sample::ids::A_CAM);
        assert!(to_text(&project, view).is_err());
    }
}
