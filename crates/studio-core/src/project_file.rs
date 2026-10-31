//! The `.studio` project file: the project as JSON, plus where the editor was (open
//! composition, playhead, work area). Text keeps files readable and lets them diff cleanly
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

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    version: u32,
    view: EditorView,
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
    project: Project,
}

/// Writes a project as file text. Fails only if the result could not be read back, which is
/// checked here so a save never produces a file that will not open.
pub fn to_text(project: &Project, view: EditorView) -> Result<String, String> {
    let file = FileOut { format: FORMAT, version: FORMAT_VERSION, view, project };
    let text = serde_json::to_string_pretty(&file).map_err(|error| format!("The project could not be saved: {error}"))?;
    from_text(&text).map_err(|error| format!("The project could not be saved, because it would not open again: {error}"))?;
    Ok(text)
}

/// Reads file text into a checked project and editor view.
pub fn from_text(text: &str) -> Result<(Project, EditorView), String> {
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
    let file: FileIn = serde_json::from_str(text).map_err(|error| format!("The project file is damaged: {error}"))?;
    let FileIn { mut project, mut view } = file;
    restore_effect_info(&mut project);
    fill_missing_start_frames(&mut project);
    project.refresh_next_id();
    check(&project)?;

    // Fall back to the first composition if the saved one is missing, and keep the playhead
    // and work area inside the composition.
    if project.composition(view.active_composition).is_none() {
        view.active_composition = project.compositions[0].id;
    }
    let duration = project.composition(view.active_composition).map_or(1, |comp| comp.duration_frames);
    view.current_frame = view.current_frame.clamp(0, duration - 1);
    let work_area = if view.work_area.start.is_finite() && view.work_area.end.is_finite() { view.work_area } else { FrameRange::new(0.0, duration as f64) };
    view.work_area = work_area.clamped(duration as f64, 1.0);
    Ok((project, view))
}

/// Checks what the editor relies on, so a hand-edited or damaged file cannot crash it.
fn check(project: &Project) -> Result<(), String> {
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
        let mut layer_ids = std::collections::HashSet::new();
        for layer in &comp.layers {
            if !layer_ids.insert(layer.id) {
                return Err(format!("Composition \"{}\" is damaged: two layers share the id {}.", comp.name, layer.id.0));
            }
            if layer.out_frame <= layer.in_frame {
                return Err(format!("Layer \"{}\" is damaged: it ends before it starts.", layer.name));
            }
        }
    }
    Ok(())
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

/// Media files that are missing where the project says they are, but found under the same
/// name next to the project file, are pointed there. This keeps a project and its media
/// working after the folder holding both is moved or copied to another machine. Returns the
/// files that are still missing.
pub fn relink_missing_media(project: &mut Project, project_folder: &Path, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut missing = Vec::new();
    for item in &mut project.items {
        let Some(path) = item.path.as_mut() else { continue };
        if exists(path) {
            continue;
        }
        let beside = path.file_name().map(|name| project_folder.join(name));
        match beside.filter(|candidate| exists(candidate)) {
            Some(found) => *path = found,
            None => missing.push(path.clone()),
        }
    }
    missing
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
    fn a_saved_project_opens_unchanged() {
        let mut project = sample_project();
        // Cover effects, keyframes, text and parenting: the sample already has all of these.
        let title = project.compositions[0].layers[0].id;
        edit::add_effect(&mut project, ids::MAIN_COMP, title, "Exposure").unwrap();
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
}
