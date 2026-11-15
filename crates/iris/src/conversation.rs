//! Keeping Iris's conversation when the app closes, so it is there again next time: each
//! app's messages, and what was typed for the arrow keys to go back through (see `complete`).
//! Each app keeps its own, in a file of its own beside Iris's shared settings.
//!
//! Only words are kept. Buttons offered under a message were about the project as it was, so
//! they are not brought back.

use std::path::Path;

use crate::App;

/// The most messages kept.
pub const MOST_LINES: usize = 200;

/// One message, as it is kept.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    /// Whether the person wrote it (else Iris did).
    pub from_person: bool,
    pub text: String,
}

/// A conversation as it is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Saved {
    /// The messages, oldest first.
    pub lines: Vec<Line>,
    /// What was typed, oldest first, for going back through with the arrow keys.
    pub typed: Vec<String>,
}

/// The file `app`'s conversation is kept in.
fn file_name(app: App) -> String {
    format!("conversation-{}.json", format!("{app:?}").to_lowercase())
}

/// `app`'s conversation from `folder`. Empty when there is none yet, or the file can't be read.
pub fn load(folder: &Path, app: App) -> Saved {
    std::fs::read_to_string(folder.join(file_name(app))).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// Keeps `saved` as `app`'s conversation, without its oldest messages past the limits.
/// Written through a temporary file, so a crash while saving never leaves half a file.
pub fn save(folder: &Path, app: App, saved: &Saved) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let mut kept = saved.clone();
    let excess = kept.lines.len().saturating_sub(MOST_LINES);
    kept.lines.drain(..excess);
    let excess = kept.typed.len().saturating_sub(crate::complete::MOST_RECALLED);
    kept.typed.drain(..excess);
    let text = serde_json::to_string(&kept).map_err(std::io::Error::other)?;
    let name = file_name(app);
    let partial = folder.join(format!("{name}.{}.partial", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, folder.join(name))
}

/// Forgets `app`'s conversation (for "/clear"). Nothing kept is no problem.
pub fn clear(folder: &Path, app: App) -> std::io::Result<()> {
    match std::fs::remove_file(folder.join(file_name(app))) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_app_keeps_its_own_conversation_within_the_limits() {
        let folder = std::env::temp_dir().join(format!("iris-conversation-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        assert_eq!(load(&folder, App::Studio), Saved::default());
        let lines: Vec<Line> = (0..250).map(|number| Line { from_person: number % 2 == 0, text: number.to_string() }).collect();
        save(&folder, App::Studio, &Saved { lines, typed: vec!["fade in".to_owned()] }).unwrap();
        save(&folder, App::Develop, &Saved { lines: vec![Line { from_person: true, text: "brighter".to_owned() }], typed: Vec::new() }).unwrap();
        let studio = load(&folder, App::Studio);
        assert_eq!(studio.lines.len(), MOST_LINES);
        assert_eq!(studio.lines[0].text, "50", "the oldest go first");
        assert_eq!(studio.typed, vec!["fade in"]);
        assert_eq!(load(&folder, App::Develop).lines.len(), 1);
        clear(&folder, App::Studio).unwrap();
        clear(&folder, App::Studio).unwrap();
        assert_eq!(load(&folder, App::Studio), Saved::default());
        assert_eq!(load(&folder, App::Develop).lines.len(), 1, "the other app's is kept");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_damaged_file_reads_as_empty() {
        let folder = std::env::temp_dir().join(format!("iris-conversation-damaged-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(file_name(App::Canvas)), "{ not json").unwrap();
        assert_eq!(load(&folder, App::Canvas), Saved::default());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
