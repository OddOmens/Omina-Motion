//! Export presets by name: "export for Instagram", "save these export settings as Print",
//! "forget the Print export preset", "list export presets". A preset is a bundle of the app's
//! own export choices (format, size, quality…), which only the app knows how to read, so it is
//! kept here as the app wrote it (`settings`) and handed back when asked for.
//!
//! Each app keeps its own presets, all in one file beside Iris's shared settings,
//! `export-presets.json`. An app may also come with presets of its own ("Instagram", "Print",
//! "Web"); a saved preset of the same name is used instead of it.

use std::collections::BTreeMap;
use std::path::Path;

use crate::App;

/// The file every app's presets are kept in, inside Iris's shared folder.
const FILE_NAME: &str = "export-presets.json";
/// The longest name a preset can have, in characters.
pub const NAME_LIMIT: usize = 24;
/// The most presets one app keeps.
pub const MOST_PRESETS: usize = 50;

/// One preset: its name as it was typed, and the app's export choices.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Preset {
    pub name: String,
    pub settings: serde_json::Value,
}

/// What a message about export presets asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    /// Export with the preset of this name (as known, so its case is the preset's own).
    Use(String),
    /// Keep the export choices as they are now under this name.
    Save(String),
    /// Forget the saved preset of this name.
    Forget(String),
    /// List the presets.
    List,
    /// Something about presets that won't do; the words say what.
    Problem(String),
}

/// The name a preset is found by: lower case, single spaces.
fn plain(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A typed name made fit to keep, or why it won't do.
fn tidy_name(typed: &str) -> Result<String, String> {
    let name = typed.trim().trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '.' | '!' | '?')).split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the preset a name, as in “save these export settings as Print”.".to_owned());
    }
    if name.chars().count() > NAME_LIMIT {
        return Err(format!("Keep a preset's name to {NAME_LIMIT} letters."));
    }
    Ok(name)
}

/// Reads `message` as being about export presets, given the names of those `known` (saved and
/// the app's own). `None` when it is about something else; "export for the web" with no preset
/// of that name is the app's to read.
pub fn read(message: &str, known: &[String]) -> Option<Said> {
    let message = message.trim().trim_end_matches(['.', '!', '?']);
    let lower = message.to_lowercase();
    let after = |start: &str| lower.starts_with(start).then(|| message.get(start.len()..)).flatten();
    if matches!(lower.as_str(), "list export presets" | "export presets" | "/export-presets" | "show export presets" | "what export presets are there" | "which export presets are there" | "my export presets") {
        return Some(Said::List);
    }
    for start in ["save these export settings as ", "save this export setup as ", "save the export settings as ", "remember these export settings as ", "save an export preset called ", "save export preset ", "make an export preset called "] {
        if let Some(name) = after(start) {
            return Some(match tidy_name(name) {
                Ok(name) => Said::Save(name),
                Err(why) => Said::Problem(why),
            });
        }
    }
    for start in ["forget the export preset ", "forget export preset ", "delete the export preset ", "delete export preset ", "remove the export preset "] {
        if let Some(name) = after(start) {
            return Some(Said::Forget(tidy_name(name).ok()?));
        }
    }
    // "Forget the Print export preset".
    if let Some(name) = after("forget the ").or_else(|| after("delete the ")).and_then(|rest| rest.strip_suffix(" export preset")) {
        return Some(Said::Forget(tidy_name(name).ok()?));
    }
    // "Export for Instagram", "export with the Print preset", "use the Web export preset".
    let named = after("export for ")
        .or_else(|| after("export it for "))
        .or_else(|| after("export with the "))
        .or_else(|| after("export using the "))
        .or_else(|| after("use the "))
        .map(|rest| rest.trim_end_matches(" export preset").trim_end_matches(" preset").trim_end_matches(" settings").trim())
        .map(|rest| if rest.to_lowercase().starts_with("the ") { &rest[4..] } else { rest })?;
    known.iter().find(|name| plain(name) == plain(named)).map(|name| Said::Use(name.clone()))
}

/// Each preset name on a line, the saved ones first.
pub fn listed(saved: &[Preset], own: &[Preset]) -> String {
    let mut lines: Vec<String> = saved.iter().map(|preset| format!("• {}", preset.name)).collect();
    lines.extend(own.iter().filter(|preset| !saved.iter().any(|kept| plain(&kept.name) == plain(&preset.name))).map(|preset| format!("• {} (comes with the app)", preset.name)));
    lines.join("\n")
}

/// The preset to use for `name`: a saved one first, then the app's own.
pub fn find<'a>(name: &str, saved: &'a [Preset], own: &'a [Preset]) -> Option<&'a Preset> {
    saved.iter().chain(own).find(|preset| plain(&preset.name) == plain(name))
}

/// Keeps `preset` among `saved`, replacing one of the same name. `Err` when there are already
/// as many as can be kept.
pub fn keep(saved: &mut Vec<Preset>, preset: Preset) -> Result<(), String> {
    match saved.iter().position(|kept| plain(&kept.name) == plain(&preset.name)) {
        Some(index) => saved[index] = preset,
        None if saved.len() >= MOST_PRESETS => return Err(format!("I keep at most {MOST_PRESETS} export presets. Forget one first.")),
        None => saved.push(preset),
    }
    Ok(())
}

/// Every app's presets as they are kept in the file, by the app's lower-case name.
type Book = BTreeMap<String, Vec<Preset>>;

fn key(app: App) -> String {
    format!("{app:?}").to_lowercase()
}

fn read_book(folder: &Path) -> Book {
    std::fs::read_to_string(folder.join(FILE_NAME)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// The presets saved in `app`. Empty when there are none or the file can't be read.
pub fn load(folder: &Path, app: App) -> Vec<Preset> {
    read_book(folder).remove(&key(app)).unwrap_or_default()
}

/// Keeps `presets` as the ones saved in `app`, leaving the other apps' as they are.
pub fn save(folder: &Path, app: App, presets: &[Preset]) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let mut book = read_book(folder);
    book.insert(key(app), presets.to_vec());
    let text = serde_json::to_string_pretty(&book).map_err(std::io::Error::other)?;
    let partial = folder.join(format!("{FILE_NAME}.{}.partial", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, folder.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        vec!["Instagram".to_owned(), "Print".to_owned()]
    }

    #[test]
    fn presets_are_used_saved_forgotten_and_listed_by_word() {
        assert_eq!(read("export for instagram", &known()), Some(Said::Use("Instagram".to_owned())));
        assert_eq!(read("Export with the Print preset.", &known()), Some(Said::Use("Print".to_owned())));
        assert_eq!(read("use the print export preset", &known()), Some(Said::Use("Print".to_owned())));
        assert_eq!(read("save these export settings as Client Review", &known()), Some(Said::Save("Client Review".to_owned())));
        assert_eq!(read("forget the Print export preset", &known()), Some(Said::Forget("Print".to_owned())));
        assert_eq!(read("delete export preset Print", &known()), Some(Said::Forget("Print".to_owned())));
        assert_eq!(read("list export presets", &known()), Some(Said::List));
        assert!(matches!(read("save these export settings as", &known()), Some(Said::Problem(_)) | None));
    }

    #[test]
    fn other_messages_are_left_for_the_app() {
        assert_eq!(read("export for the web", &known()), None, "no preset of that name");
        assert_eq!(read("export as png", &known()), None);
        assert_eq!(read("use the pen tool", &known()), None);
        assert_eq!(read("export", &known()), None);
    }

    #[test]
    fn saved_presets_come_before_the_apps_own_and_are_kept_per_app() {
        let own = vec![Preset { name: "Print".to_owned(), settings: serde_json::json!({ "dpi": 300 }) }];
        let mut saved = Vec::new();
        keep(&mut saved, Preset { name: "print".to_owned(), settings: serde_json::json!({ "dpi": 600 }) }).unwrap();
        assert_eq!(find("PRINT", &saved, &own).map(|preset| preset.settings["dpi"].as_i64()), Some(Some(600)));
        assert_eq!(listed(&saved, &own), "• print");
        let folder = std::env::temp_dir().join(format!("iris-export-presets-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        save(&folder, App::Canvas, &saved).unwrap();
        assert_eq!(load(&folder, App::Canvas), saved);
        assert!(load(&folder, App::Studio).is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
