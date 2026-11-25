//! Files the operating system asks the app to open: a project double-clicked in Finder or File
//! Explorer, dropped on the app's icon, or picked with Open With. Windows and Linux start the
//! app with the file named on its command line; macOS sends an "open" event instead, both at
//! launch and while the app is already running. Either way the files wait in one queue: the
//! launch opens what is waiting in place of the start screen, and once the app has launched,
//! files that arrive later open as they come. (Omina Edit and Omina Develop do the same.)

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui_kit::{Application, Context, Window};

use crate::workspace::Workspace;

/// Files handed over by the system and not opened yet.
static WAITING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Whether the launch has taken what was waiting; until then, new files wait for it.
static LAUNCHED: AtomicBool = AtomicBool::new(false);
/// A signal for each hand-over, picked up by the window (see `watch`).
static ARRIVALS: Mutex<Option<UnboundedReceiver<()>>> = Mutex::new(None);

/// Opens files in the workspace, as each app does it.
pub type OpenFiles = fn(&mut Workspace, Vec<PathBuf>, &mut Window, &mut Context<Workspace>);

/// Starts listening for the system's "open these files" events. Called once, before the app
/// runs, so a file double-clicked to launch the app isn't missed.
pub fn listen(app: &Application) {
    let (arrived, arrivals) = unbounded();
    *lock(&ARRIVALS) = Some(arrivals);
    app.on_open_urls(move |urls| {
        let paths: Vec<PathBuf> = urls.iter().filter_map(|url| path_from_url(url)).collect();
        if paths.is_empty() {
            return;
        }
        lock(&WAITING).extend(paths);
        // Only fails once the window is gone, when there is nothing left to open into.
        let _ = arrived.unbounded_send(());
    });
}

/// The files to open at launch: those named on the command line, then those the system sent.
/// From now on, files that arrive open straight away (see `watch`).
pub fn take_at_launch() -> Vec<PathBuf> {
    LAUNCHED.store(true, Ordering::SeqCst);
    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).filter(|path| path.exists()).collect();
    paths.append(&mut lock(&WAITING));
    paths
}

/// Opens files the system hands over after launch with `open`, bringing the app to the front.
/// Does nothing where `listen` never ran (headless tests).
pub fn watch(window: &mut Window, cx: &mut Context<Workspace>, open: OpenFiles) {
    let Some(mut arrivals) = lock(&ARRIVALS).take() else { return };
    cx.spawn_in(window, async move |this, cx| {
        while arrivals.next().await.is_some() {
            if !LAUNCHED.load(Ordering::SeqCst) {
                continue;
            }
            let paths = std::mem::take(&mut *lock(&WAITING));
            if paths.is_empty() {
                continue;
            }
            let opened = this.update_in(cx, |this, window, cx| {
                cx.activate(true);
                window.activate_window();
                open(this, paths, window, cx);
            });
            if opened.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// The queue's lock. A panic while it was held leaves a list that is still usable, so a
/// poisoned lock is simply taken over.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The file a `file://` URL names (as macOS sends them, with spaces and accents %-escaped), or
/// `None` for any other kind of URL or a malformed one.
fn path_from_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let high = (chars.next()? as char).to_digit(16)?;
            let low = (chars.next()? as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_become_paths_with_their_escapes_undone() {
        assert_eq!(path_from_url("file:///Users/me/Tide%20Keep.studio"), Some(PathBuf::from("/Users/me/Tide Keep.studio")));
        assert_eq!(path_from_url("file://localhost/tmp/caf%C3%A9.studio"), Some(PathBuf::from("/tmp/café.studio")));
        assert_eq!(path_from_url("file:///tmp/plain.studio"), Some(PathBuf::from("/tmp/plain.studio")));
    }

    #[test]
    fn other_or_malformed_urls_are_left_alone() {
        assert_eq!(path_from_url("https://example.com/a.studio"), None);
        assert_eq!(path_from_url("file:///tmp/broken%2"), None);
        assert_eq!(path_from_url("file:///tmp/bad%zz"), None);
        assert_eq!(path_from_url("file://server/share/a.studio"), None);
    }
}
