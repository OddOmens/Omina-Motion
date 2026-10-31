//! Serves decoded frames to the renderer without ever blocking it. Each clip gets up to
//! two background threads:
//!
//! - the **full-quality** worker decodes the original file: the frame the viewer wants,
//!   and a few ahead while playing. A long decode (a jump in long-GOP footage) is
//!   abandoned as soon as the viewer has moved on, so it never holds up newer requests.
//! - the **proxy** worker starts once the clip's proxy is built. It decodes any wanted
//!   frame the full-quality worker does not have yet; proxies are all-intra, so that is
//!   a few milliseconds even for a far jump.
//!
//! A request returns the exact full-quality frame if it is ready, else the exact proxy
//! frame, else the nearest frame either worker has. `on_ready` fires when the frame the
//! viewer wants arrives, so it can redraw.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use studio_core::media::{VideoFrame, VideoSource};

use crate::decode::VideoDecoder;
use crate::proxy::{PROXY_WIDTH, ProxyManager};

/// Frames decoded ahead of the playhead while playing.
const PREFETCH: i64 = 12;
/// Frames kept behind the playhead, for stepping backwards.
const KEEP_BEHIND: i64 = 6;

type ReadyCallback = Box<dyn Fn() + Send + Sync>;

/// Decoded frames from one source (the original or the proxy), by source frame number.
#[derive(Default)]
struct Frames(BTreeMap<i64, Arc<VideoFrame>>);

impl Frames {
    fn has(&self, index: i64) -> bool {
        self.0.contains_key(&index)
    }

    /// Drops frames far from `wanted` so memory stays bounded.
    fn evict(&mut self, wanted: i64) {
        let keep = wanted - KEEP_BEHIND..=wanted + PREFETCH + KEEP_BEHIND;
        self.0.retain(|index, _| keep.contains(index));
    }

    /// The frame closest to `index`, and how far it is.
    fn nearest(&self, index: i64) -> Option<(i64, Arc<VideoFrame>)> {
        let before = self.0.range(..=index).next_back().map(|(i, f)| (index - i, f.clone()));
        let after = self.0.range(index..).next().map(|(i, f)| (i - index, f.clone()));
        match (before, after) {
            (Some(b), Some(a)) => Some(if b.0 <= a.0 { b } else { a }),
            (one, None) | (None, one) => one,
        }
    }
}

#[derive(Default)]
struct Clip {
    full: Frames,
    proxy: Frames,
    /// The frame the renderer last asked for.
    wanted: i64,
    /// Known once the full-quality worker has opened the file.
    frame_count: Option<i64>,
    error: Option<String>,
    proxy_worker: bool,
}

impl Clip {
    fn clamp(&self, index: i64) -> i64 {
        self.frame_count.map_or(index.max(0), |count| index.clamp(0, count - 1))
    }

    /// The next frame the full-quality worker should decode, if any.
    fn next_full_job(&self, playing: bool) -> Option<i64> {
        let last = self.frame_count? - 1;
        let wanted = self.wanted.clamp(0, last);
        if !self.full.has(wanted) {
            return Some(wanted);
        }
        if playing {
            return (wanted + 1..=(wanted + PREFETCH).min(last)).find(|index| !self.full.has(*index));
        }
        None
    }

    /// The frame the proxy worker should decode: the wanted one, if neither worker has it.
    fn next_proxy_job(&self) -> Option<i64> {
        let wanted = self.clamp(self.wanted);
        (!self.full.has(wanted) && !self.proxy.has(wanted)).then_some(wanted)
    }

    /// Whether a full-quality decode of `job` is no longer useful, because the viewer moved elsewhere.
    fn full_job_is_stale(&self, job: i64, playing: bool) -> bool {
        let wanted = self.clamp(self.wanted);
        let ahead = playing && job > wanted && job <= wanted + PREFETCH;
        job != wanted && !ahead
    }

    /// The best picture for `index` right now.
    fn best(&self, index: i64) -> Option<Arc<VideoFrame>> {
        if let Some(frame) = self.full.0.get(&index) {
            return Some(frame.clone());
        }
        if let Some(frame) = self.proxy.0.get(&index) {
            return Some(frame.clone());
        }
        match (self.full.nearest(index), self.proxy.nearest(index)) {
            (Some(full), Some(proxy)) => Some(if full.0 <= proxy.0 { full.1 } else { proxy.1 }),
            (one, None) | (None, one) => one.map(|(_, frame)| frame),
        }
    }
}

#[derive(Default)]
struct State {
    clips: HashMap<PathBuf, Clip>,
    playing: bool,
    shutting_down: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes workers when there is new work or the server is shutting down.
    work: Condvar,
    on_ready: Mutex<Option<ReadyCallback>>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A worker that panicked mid-decode leaves the data consistent, so keep going.
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn notify_ready(&self) {
        if let Some(callback) = self.on_ready.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            callback();
        }
    }
}

pub struct FrameServer {
    shared: Arc<Shared>,
    max_width: u32,
    proxies: Option<Arc<ProxyManager>>,
}

impl FrameServer {
    /// Full-quality frames are decoded at most `max_width` wide (the preview needs no more).
    /// With `proxies`, clips whose proxy is ready also get a fast proxy worker.
    pub fn new(max_width: u32, proxies: Option<Arc<ProxyManager>>) -> Self {
        let shared = Arc::new(Shared { state: Mutex::default(), work: Condvar::new(), on_ready: Mutex::new(None) });
        Self { shared, max_width, proxies }
    }

    /// Called (from a decode thread) whenever the frame the viewer wants becomes available.
    pub fn set_on_ready(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_ready.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(callback));
    }

    /// While playing, the full-quality worker decodes ahead of the playhead.
    pub fn set_playing(&self, playing: bool) {
        let mut state = self.shared.lock();
        if state.playing != playing {
            state.playing = playing;
            self.shared.work.notify_all();
        }
    }

    /// Why a clip could not be decoded, if it failed.
    pub fn error(&self, path: &Path) -> Option<String> {
        self.shared.lock().clips.get(path).and_then(|clip| clip.error.clone())
    }

    fn spawn(&self, name: String, job: impl FnOnce() + Send + 'static) {
        if let Err(error) = std::thread::Builder::new().name(name).spawn(job) {
            eprintln!("could not start a decode thread: {error}");
        }
    }
}

impl Drop for FrameServer {
    fn drop(&mut self) {
        self.shared.lock().shutting_down = true;
        self.shared.work.notify_all();
    }
}

impl VideoSource for FrameServer {
    fn frame(&self, path: &Path, index: i64) -> Option<Arc<VideoFrame>> {
        // Look up the proxy before taking our lock, so the two locks are never held together.
        let proxy = self.proxies.as_ref().and_then(|proxies| proxies.ready(path));
        let mut state = self.shared.lock();
        let is_new = !state.clips.contains_key(path);
        let clip = state.clips.entry(path.to_owned()).or_default();
        let index = clip.clamp(index);
        if clip.wanted != index || is_new {
            clip.wanted = index;
            self.shared.work.notify_all();
        }
        let start_proxy = proxy.filter(|_| !clip.proxy_worker);
        if start_proxy.is_some() {
            clip.proxy_worker = true;
        }
        let found = clip.best(index);
        drop(state);

        let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        if is_new {
            let (shared, path, width) = (self.shared.clone(), path.to_owned(), self.max_width);
            self.spawn(format!("decode {name}"), move || full_quality_worker(&shared, &path, width));
        }
        if let Some(proxy_file) = start_proxy {
            let (shared, path) = (self.shared.clone(), path.to_owned());
            self.spawn(format!("proxy {name}"), move || proxy_worker(&shared, &path, &proxy_file));
        }
        found
    }
}

/// Waits until `pick` finds a job for the clip at `path`, or returns `None` on shutdown.
fn wait_for_job(shared: &Shared, path: &Path, pick: impl Fn(&Clip, bool) -> Option<i64>) -> Option<i64> {
    let mut state = shared.lock();
    loop {
        if state.shutting_down {
            return None;
        }
        let playing = state.playing;
        if let Some(job) = state.clips.get(path).and_then(|clip| pick(clip, playing)) {
            return Some(job);
        }
        state = shared.work.wait(state).unwrap_or_else(|p| p.into_inner());
    }
}

/// Decodes the original file: what the viewer wants, and ahead while playing.
fn full_quality_worker(shared: &Shared, path: &Path, max_width: u32) {
    let mut decoder = match VideoDecoder::open_fastest(path, max_width) {
        Ok(decoder) => decoder,
        Err(error) => {
            if let Some(clip) = shared.lock().clips.get_mut(path) {
                clip.error = Some(error.to_string());
            }
            shared.notify_ready();
            return;
        }
    };
    if let Some(clip) = shared.lock().clips.get_mut(path) {
        clip.frame_count = Some(decoder.frame_count());
    }
    // Wake the proxy worker too, now that the clip's length is known.
    shared.work.notify_all();

    while let Some(job) = wait_for_job(shared, path, Clip::next_full_job) {
        let stale = || {
            let state = shared.lock();
            state.shutting_down || state.clips.get(path).is_none_or(|clip| clip.full_job_is_stale(job, state.playing))
        };
        let decoded = decoder.frame_unless(job, &stale);
        let wanted_now = {
            let mut state = shared.lock();
            let Some(clip) = state.clips.get_mut(path) else { return };
            match decoded {
                Ok(Some(frame)) => {
                    clip.full.0.insert(job, Arc::new(frame));
                    clip.full.evict(clip.wanted);
                }
                // Abandoned because the viewer moved on; the next request starts fresh.
                Ok(None) => {}
                Err(error) => clip.error = Some(error.to_string()),
            }
            clip.wanted == job
        };
        // Redraw when the frame on screen should change; ahead-of-time frames need no redraw.
        if wanted_now {
            shared.notify_ready();
        }
    }
}

/// Decodes the clip's proxy: any wanted frame the full-quality worker does not have yet.
fn proxy_worker(shared: &Shared, path: &Path, proxy_file: &Path) {
    let Ok(mut decoder) = VideoDecoder::open(proxy_file, PROXY_WIDTH, false) else { return };
    // Proxies match their source frame for frame, but wait for the source's length to be known.
    while let Some(job) = wait_for_job(shared, path, |clip, _| clip.frame_count.and(clip.next_proxy_job())) {
        let Ok(mut frame) = decoder.frame(job) else { continue };
        frame.from_proxy = true;
        let wanted_now = {
            let mut state = shared.lock();
            let Some(clip) = state.clips.get_mut(path) else { return };
            clip.proxy.0.insert(job, Arc::new(frame));
            clip.proxy.evict(clip.wanted);
            clip.wanted == job
        };
        if wanted_now {
            shared.notify_ready();
        }
    }
}
