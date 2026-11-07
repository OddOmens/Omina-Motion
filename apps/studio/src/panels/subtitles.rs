//! Subtitle files (.srt and .vtt): importing one as a caption layer in the open composition
//! (File ▸ Import, a drop, or Import Subtitles… in the Captions panel), and saving a caption
//! layer's captions as one (Properties, or the layer's right-click menu). Reading and checking
//! the file is `studio_core::subtitles`; this module asks for files and says how it went.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, Window};
use studio_core::model::Id;
use studio_core::subtitles::{self, ReadCues, SubtitleFormat};

use crate::workspace::Workspace;

/// Asks for subtitle files, then imports them.
pub fn choose_and_import(window: &mut Window, cx: &mut Context<Workspace>) {
    let chosen = cx.prompt_for_paths(gpui_kit::PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Import Subtitles".into()) });
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(paths))) = chosen.await else { return };
        this.update_in(cx, |this, window, cx| import(this, paths, window, cx)).ok();
    })
    .detach();
}

/// Reads subtitle files in the background, then adds a caption layer for each to the open
/// composition (one undo step for them all) and says what came in, what was left out and why.
pub fn import(workspace: &mut Workspace, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Workspace>) {
    if paths.is_empty() {
        return;
    }
    let rate = workspace.state.composition().frame_rate;
    let reading = cx.background_executor().spawn(async move { paths.into_iter().map(|path| { let read = read_file(&path, rate); (path, read) }).collect::<Vec<_>>() });
    cx.spawn_in(window, async move |this, cx| {
        let read = reading.await;
        this.update_in(cx, |this, window, cx| add_read(this, read, window, cx)).ok();
    })
    .detach();
}

/// Reads and checks one subtitle file, no more than the largest one allowed.
fn read_file(path: &Path, rate: studio_core::time::FrameRate) -> Result<ReadCues, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    // One byte past the limit is enough to tell the file is too big.
    file.take(subtitles::MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    subtitles::read(&bytes, rate)
}

/// Adds a caption layer for each file read and posts a notification for each file.
fn add_read(workspace: &mut Workspace, read: Vec<(PathBuf, Result<ReadCues, String>)>, window: &mut Window, cx: &mut Context<Workspace>) {
    for (path, read) in read {
        let file = file_name(&path);
        let message = match read {
            Ok(read) => {
                let name = path.file_stem().map_or_else(|| "Subtitles".to_owned(), |stem| stem.to_string_lossy().into_owned());
                match workspace.state.add_subtitle_captions(&name, &read.cues) {
                    Some(_) => {
                        let count = read.cues.len();
                        let mut message = format!("Added {count} subtitle{} from “{file}” as a caption layer.", if count == 1 { "" } else { "s" });
                        if read.skipped > 0 {
                            message.push_str(&format!(" Left out {} with times that couldn't be read.", read.skipped));
                        }
                        message
                    }
                    None => format!("“{file}” couldn't be added to this composition."),
                }
            }
            Err(reason) => format!("“{file}” wasn't imported: {reason}."),
        };
        window.push_notification(message.as_str(), cx);
    }
    cx.notify();
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Asks where to save caption layer `layer`'s captions as `format`, then writes the file.
pub fn export(workspace: &mut Workspace, layer: Id, format: SubtitleFormat, window: &mut Window, cx: &mut Context<Workspace>) {
    let cues = workspace.state.subtitle_cues(layer);
    if cues.is_empty() {
        window.push_notification("This caption layer has no captions on screen to export.", cx);
        return;
    }
    let rate = workspace.state.composition().frame_rate;
    let name = workspace.state.composition().layer(layer).map_or_else(|| "Captions".to_owned(), |layer| layer.name.clone());
    let suggested = format!("{name}.{}", format.extension());
    let chosen = cx.prompt_for_new_path(&super::export::starting_folder(workspace.state.file()), Some(&suggested));
    cx.spawn_in(window, async move |this, cx| {
        let Ok(Ok(Some(mut path))) = chosen.await else { return };
        // Players go by the extension, so it must match what the file holds.
        if path.extension().and_then(|extension| extension.to_str()).is_none_or(|extension| !extension.eq_ignore_ascii_case(format.extension())) {
            path.set_extension(format.extension());
        }
        let text = subtitles::write(&cues, format, rate);
        let message = match write_file(&path, &text) {
            Ok(()) => format!("Saved {} captions as “{}”.", cues.len(), file_name(&path)),
            Err(error) => format!("The captions couldn't be saved: {error}"),
        };
        this.update_in(cx, |_, window, cx| window.push_notification(message.as_str(), cx)).ok();
    })
    .detach();
}

/// Writes `text` to a temporary file beside `path`, then renames it into place, so a failed
/// write never leaves half a file where an older one was.
fn write_file(path: &Path, text: &str) -> std::io::Result<()> {
    let temporary = path.with_file_name(format!("{}.partial", file_name(path)));
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path).inspect_err(|_| {
        std::fs::remove_file(&temporary).ok();
    })
}
