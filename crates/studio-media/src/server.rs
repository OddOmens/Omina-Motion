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
    /// Frames that just failed to decode (full quality, proxy). They are not tried again until
    /// the viewer asks for another frame; retrying straight away would spin forever.
    failed_full: Option<i64>,
    failed_proxy: Option<i64>,
    /// Tells this clip's workers apart from those of an earlier entry for the same file (one
    /// released and asked for again), so the old ones stop.
    generation: u64,
}

impl Clip {
    fn clamp(&self, index: i64) -> i64 {
        self.frame_count.map_or(index.max(0), |count| index.clamp(0, count - 1))
    }

    /// The next frame the full-quality worker should decode, if any.
    fn next_full_job(&self, playing: bool) -> Option<i64> {
        let last = self.frame_count? - 1;
        let wanted = self.wanted.clamp(0, last);
        if !self.full.has(wanted) && self.failed_full != Some(wanted) {
            return Some(wanted);
        }
        if playing {
            return (wanted + 1..=(wanted + PREFETCH).min(last)).find(|index| !self.full.has(*index) && self.failed_full != Some(*index));
        }
        None
    }

    /// The frame the proxy worker should decode: the wanted one, if neither worker has it.
    fn next_proxy_job(&self) -> Option<i64> {
        let wanted = self.clamp(self.wanted);
        (!self.full.has(wanted) && !self.proxy.has(wanted) && self.failed_proxy != Some(wanted)).then_some(wanted)
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
    next_generation: u64,
}

impl State {
    /// The clip for `path`, if it is still the entry the worker of `generation` belongs to.
    fn clip(&mut self, path: &Path, generation: u64) -> Option<&mut Clip> {
        self.clips.get_mut(path).filter(|clip| clip.generation == generation)
    }
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

    /// Lets go of every clip `keep` says is no longer needed (for example, footage not in the
    /// open composition): its decoded frames are freed and its decoder threads stop.
    pub fn release_unused(&self, keep: impl Fn(&Path) -> bool) {
        let mut state = self.shared.lock();
        let before = state.clips.len();
        state.clips.retain(|path, _| keep(path));
        if state.clips.len() != before {
            // Wake the released clips' workers so they notice and stop.
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
        let generation = state.next_generation;
        if is_new {
            state.next_generation += 1;
        }
        let clip = state.clips.entry(path.to_owned()).or_insert_with(|| Clip { generation, ..Clip::default() });
        let generation = clip.generation;
        let index = clip.clamp(index);
        if clip.wanted != index || is_new {
            clip.wanted = index;
            (clip.failed_full, clip.failed_proxy) = (None, None);
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
            self.spawn(format!("decode {name}"), move || full_quality_worker(&shared, &path, generation, width));
        }
        if let Some(proxy_file) = start_proxy {
            let (shared, path) = (self.shared.clone(), path.to_owned());
            self.spawn(format!("proxy {name}"), move || proxy_worker(&shared, &path, generation, &proxy_file));
        }
        found
    }
}

/// Waits until `pick` finds a job for the clip at `path`, or returns `None` on shutdown or
/// once the clip has been released (the worker then stops).
fn wait_for_job(shared: &Shared, path: &Path, generation: u64, pick: impl Fn(&Clip, bool) -> Option<i64>) -> Option<i64> {
    let mut state = shared.lock();
    loop {
        if state.shutting_down {
            return None;
        }
        let playing = state.playing;
        let clip = state.clip(path, generation)?;
        if let Some(job) = pick(clip, playing) {
            return Some(job);
        }
        state = shared.work.wait(state).unwrap_or_else(|p| p.into_inner());
    }
}

/// Decodes the original file: what the viewer wants, and ahead while playing.
fn full_quality_worker(shared: &Shared, path: &Path, generation: u64, max_width: u32) {
    let mut decoder = match VideoDecoder::open_fastest(path, max_width) {
        Ok(decoder) => decoder,
        Err(error) => {
            if let Some(clip) = shared.lock().clip(path, generation) {
                clip.error = Some(error.to_string());
            }
            shared.notify_ready();
            return;
        }
    };
    if let Some(clip) = shared.lock().clip(path, generation) {
        clip.frame_count = Some(decoder.frame_count());
        // A frame asked for before the length was known may lie past the end. Keep it within the
        // clip, or the last frame would be decoded, dropped as too far from it, and decoded again.
        clip.wanted = clip.clamp(clip.wanted);
    }
    // Wake the proxy worker too, now that the clip's length is known.
    shared.work.notify_all();

    while let Some(job) = wait_for_job(shared, path, generation, Clip::next_full_job) {
        let stale = || {
            let state = shared.lock();
            state.shutting_down || state.clips.get(path).filter(|clip| clip.generation == generation).is_none_or(|clip| clip.full_job_is_stale(job, state.playing))
        };
        let decoded = decoder.frame_unless(job, &stale);
        let wanted_now = {
            let mut state = shared.lock();
            let Some(clip) = state.clip(path, generation) else { return };
            match decoded {
                Ok(Some(frame)) => {
                    clip.full.0.insert(job, Arc::new(frame));
                    clip.full.evict(clip.wanted);
                }
                // Abandoned because the viewer moved on; the next request starts fresh.
                Ok(None) => {}
                Err(error) => {
                    clip.error = Some(error.to_string());
                    clip.failed_full = Some(job);
                }
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
fn proxy_worker(shared: &Shared, path: &Path, generation: u64, proxy_file: &Path) {
    let Ok(mut decoder) = VideoDecoder::open(proxy_file, PROXY_WIDTH, false) else { return };
    // Proxies match their source frame for frame, but wait for the source's length to be known.
    while let Some(job) = wait_for_job(shared, path, generation, |clip, _| clip.frame_count.and(clip.next_proxy_job())) {
        let Ok(mut frame) = decoder.frame(job) else {
            // Don't retry the same frame straight away (see `failed_proxy`).
            if let Some(clip) = shared.lock().clip(path, generation) {
                clip.failed_proxy = Some(job);
            }
            continue;
        };
        frame.from_proxy = true;
        let wanted_now = {
            let mut state = shared.lock();
            let Some(clip) = state.clip(path, generation) else { return };
            clip.proxy.0.insert(job, Arc::new(frame));
            clip.proxy.evict(clip.wanted);
            clip.wanted == job
        };
        if wanted_now {
            shared.notify_ready();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_that_failed_is_not_retried_until_the_viewer_moves() {
        let mut clip = Clip { frame_count: Some(48), wanted: 5, ..Clip::default() };
        assert_eq!(clip.next_full_job(false), Some(5));
        clip.failed_full = Some(5);
        assert_eq!(clip.next_full_job(false), None, "no retry of the failed frame");
        assert_eq!(clip.next_full_job(true), Some(6), "while playing, later frames still decode");
        clip.failed_proxy = Some(5);
        assert_eq!(clip.next_proxy_job(), None);
    }

    #[test]
    fn released_clips_free_their_frames_and_stop_their_threads() {
        let clip = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colors.mp4");
        let server = FrameServer::new(64, None);
        let started = std::time::Instant::now();
        while server.frame(&clip, 3).is_none() {
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "the clip never decoded");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(Arc::strong_count(&server.shared) > 1, "a decoder thread holds the clip");
        server.release_unused(|_| false);
        assert!(server.shared.lock().clips.is_empty());
        // The worker notices and stops, dropping its share of the server's state.
        let started = std::time::Instant::now();
        while Arc::strong_count(&server.shared) > 1 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "the decoder thread never stopped");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Asking again starts afresh.
        while server.frame(&clip, 3).is_none() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_first_request_past_the_end_shows_the_last_frame() {
        let clip = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colors.mp4");
        let server = FrameServer::new(64, None);
        let ready = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = ready.clone();
        server.set_on_ready(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        // Asked once, before the clip's 48 frames are known, then never again.
        server.frame(&clip, 1000);
        let started = std::time::Instant::now();
        while ready.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "the last frame was never reported ready");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let state = server.shared.lock();
        let clip = &state.clips[clip.as_path()];
        assert_eq!(clip.wanted, 47);
        assert!(clip.full.has(47), "the decoded frame is kept");
    }

    #[test]
    fn asking_for_another_frame_clears_the_failure() {
        let server = FrameServer::new(64, None);
        let path = Path::new("/no/such/clip.mp4");
        server.frame(path, 5);
        {
            let mut state = server.shared.lock();
            let clip = state.clips.get_mut(path).unwrap();
            clip.failed_full = Some(5);
            clip.failed_proxy = Some(5);
        }
        server.frame(path, 6);
        let state = server.shared.lock();
        let clip = &state.clips[path];
        assert_eq!((clip.failed_full, clip.failed_proxy), (None, None));
    }
}
