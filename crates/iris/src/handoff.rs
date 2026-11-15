//! Handing work from one app to another: "open this photo in Omina Edit", "send this frame to
//! Develop", "put this in Motion". The sending app writes the files into a package folder for
//! the other app, inside Iris's shared folder, and finishes with an empty `ready` file. The
//! receiving app checks for ready packages while it runs and when it starts (`take_ready`),
//! and brings in what it finds. The sender then asks macOS to open the other app (`open_app`),
//! which launches it or brings it to the front.
//!
//! The `ready` file is written last and taken away by the one app that brings a package in, so
//! half-written packages are never read and each is brought in once. The package folder stays,
//! since what was brought in may refer to its files.
//!
//! Motion's older "From Canvas" folder (Edit's Bring to Motion) is separate and keeps working.

use std::path::{Path, PathBuf};

use crate::App;

/// The file that says what a package holds.
const NOTE: &str = "handoff.json";
/// The file whose presence says a package is complete.
const READY: &str = "ready";

/// What a package holds, as written in its note.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Package {
    /// The app that sent it.
    pub from: App,
    /// What it is, in a few words, for the receiving app to say: "the photo “Beach”".
    pub title: String,
    /// The files in it, as paths inside the package folder once brought in. In the note they
    /// are file names.
    pub files: Vec<PathBuf>,
}

impl App {
    /// The app's bundle identifier on macOS, by which it is opened.
    pub fn bundle_id(self) -> &'static str {
        match self {
            App::Studio => "com.oddomens.studio",
            App::Canvas => "com.oddomens.canvas",
            App::Develop => "com.oddomens.develop",
        }
    }
}

/// The folder packages for `to` are written into, inside `root` (Iris's shared folder).
pub fn inbox(root: &Path, to: App) -> PathBuf {
    root.join("Hand-offs").join(format!("{to:?}"))
}

/// A file name that is safe on every system and not empty.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|character| if character.is_control() || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { character }).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_owned();
    if cleaned.is_empty() { "Untitled".to_owned() } else { cleaned }
}

/// Writes a package for `to` holding copies of `files`, said to be `title`, and marks it ready.
/// Returns its folder. `Err` says what went wrong, in words to show; nothing half-written is
/// ever marked ready.
pub fn send(root: &Path, from: App, to: App, title: &str, files: &[PathBuf]) -> Result<PathBuf, String> {
    if files.is_empty() {
        return Err("There was nothing to send.".to_owned());
    }
    let inbox = inbox(root, to);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_millis());
    let folder = inbox.join(format!("{stamp}-{}-{}", std::process::id(), safe_name(title)));
    std::fs::create_dir_all(&folder).map_err(|error| format!("The hand-off folder couldn't be made: {error}"))?;
    let mut names = Vec::new();
    for file in files {
        let name = safe_name(&file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        // Two files of the same name keep both.
        let name = if names.contains(&PathBuf::from(&name)) { format!("{} {name}", names.len() + 1) } else { name };
        std::fs::copy(file, folder.join(&name)).map_err(|error| format!("{} couldn't be copied for {}: {error}", file.display(), to.name()))?;
        names.push(PathBuf::from(name));
    }
    let note = Package { from, title: title.to_owned(), files: names };
    let text = serde_json::to_string_pretty(&note).map_err(|error| error.to_string())?;
    std::fs::write(folder.join(NOTE), text).map_err(|error| format!("The hand-off couldn't be written: {error}"))?;
    std::fs::write(folder.join(READY), "").map_err(|error| format!("The hand-off couldn't be finished: {error}"))?;
    Ok(folder)
}

/// The ready packages for `app`, oldest first, each with its files as full paths. Each one's
/// `ready` file is taken away, so it is handed out only once. A package whose note can't be
/// read, or whose files are missing, is skipped (and left unready, so it is not tried again).
pub fn take_ready(root: &Path, app: App) -> Vec<Package> {
    let Ok(entries) = std::fs::read_dir(inbox(root, app)) else { return Vec::new() };
    let mut ready: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|folder| Some((std::fs::metadata(folder.join(READY)).and_then(|metadata| metadata.modified()).ok()?, folder)))
        .collect();
    ready.sort();
    ready
        .into_iter()
        .filter_map(|(_, folder)| {
            // Only the app that takes the marker away brings the package in.
            std::fs::remove_file(folder.join(READY)).ok()?;
            let note: Package = serde_json::from_str(&std::fs::read_to_string(folder.join(NOTE)).ok()?).ok()?;
            let files: Vec<PathBuf> = note.files.iter().map(|name| folder.join(name)).collect();
            files.iter().all(|file| file.is_file()).then_some(Package { files, ..note })
        })
        .collect()
}

/// Opens `app`, launching it if it isn't running. `Err` says, in words to show, that it isn't
/// installed or couldn't be opened.
pub fn open_app(app: App) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(format!("Opening {} from here only works on a Mac. The hand-off is waiting for it.", app.name()));
    }
    let output = std::process::Command::new("open").arg("-b").arg(app.bundle_id()).output().map_err(|error| format!("{} couldn't be opened: {error}", app.name()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("{} doesn't seem to be installed on this Mac, so it couldn't be opened. What I sent will be brought in the next time it opens.", app.name()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("iris-handoff-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn a_package_is_sent_and_taken_once_by_the_app_it_is_for() {
        let root = scratch("send");
        let photo = root.join("Beach.jpg");
        std::fs::write(&photo, b"jpeg").unwrap();
        send(&root, App::Develop, App::Canvas, "the photo “Beach”", std::slice::from_ref(&photo)).unwrap();
        assert!(take_ready(&root, App::Studio).is_empty(), "it is for Edit");
        let taken = take_ready(&root, App::Canvas);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].from, App::Develop);
        assert_eq!(taken[0].title, "the photo “Beach”");
        assert_eq!(std::fs::read(&taken[0].files[0]).unwrap(), b"jpeg");
        assert!(take_ready(&root, App::Canvas).is_empty(), "each is taken once");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn half_written_and_damaged_packages_are_not_taken() {
        let root = scratch("half");
        let folder = inbox(&root, App::Studio).join("half");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(NOTE), "{ not json").unwrap();
        assert!(take_ready(&root, App::Studio).is_empty(), "no ready file yet");
        std::fs::write(folder.join(READY), "").unwrap();
        assert!(take_ready(&root, App::Studio).is_empty(), "a damaged note is skipped");
        assert!(send(&root, App::Canvas, App::Studio, "nothing", &[]).is_err());
        assert!(send(&root, App::Canvas, App::Studio, "missing", &[root.join("missing.png")]).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("a/b:c?.png"), "a-b-c-.png");
        assert_eq!(safe_name("  "), "Untitled");
        assert_eq!(safe_name(".hidden"), "hidden");
    }
}
