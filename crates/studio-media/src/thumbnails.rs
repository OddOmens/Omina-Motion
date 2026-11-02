//! Small still pictures of video files for the timeline's filmstrips, decoded on a background
//! thread so drawing the timeline never waits on a decoder.
//!
//! The timeline asks for a thumbnail at some time in a file. If that picture is ready it is
//! returned; otherwise the nearest one already decoded for that file is returned (if any) and
//! the requested time is queued. Times are rounded to a quarter of a second, so the many
//! requests a redraw makes for nearly the same spot share one picture.
//!
//! The queue is served newest request first, because the user scrolls and the pictures now on
//! screen matter more than ones asked for a moment ago. Old requests beyond `MAX_QUEUED` are
//! dropped, and decoded pictures beyond `MAX_THUMBNAILS` are freed least recently used first.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use studio_core::media::VideoFrame;
use studio_core::time::FrameRate;

use crate::decode::{VideoDecoder, probe};

/// Thumbnail times are rounded to this many steps per second.
const STEPS_PER_SECOND: f64 = 4.0;
/// Requests waiting beyond this many are dropped, oldest first: by then the user has scrolled on.
const MAX_QUEUED: usize = 200;
/// Decoded thumbnails kept in memory across all files.
const MAX_THUMBNAILS: usize = 1500;
/// Files kept open for decoding at once. Opening is the slow part, so recently used files stay
/// open; beyond this the least recently used is closed, since each open decoder holds memory.
const MAX_OPEN_DECODERS: usize = 8;

type ReadyCallback = Box<dyn Fn() + Send + Sync>;

/// A decoded thumbnail and when it was last asked for, for least-recently-used eviction.
struct Cached {
    frame: Arc<VideoFrame>,
    last_used: u64,
}

/// Everything known about one file's thumbnails.
#[derive(Default)]
struct FileThumbnails {
    /// Decoded thumbnails, by time step (quarter seconds from the start of the file).
    ready: BTreeMap<i64, Cached>,
    /// Steps that failed to decode. Not tried again, so a bad spot cannot make the worker spin.
    failed_steps: HashSet<i64>,
    /// The file could not be opened (missing, damaged, or no video). Never retried.
    failed: bool,
}

impl FileThumbnails {
    /// The thumbnail at `step`, or the one nearest to it, marking it as just used.
    fn nearest(&mut self, step: i64, now: u64) -> Option<Arc<VideoFrame>> {
        let before = self.ready.range(..=step).next_back().map(|(key, _)| *key);
        let after = self.ready.range(step..).next().map(|(key, _)| *key);
        let key = match (before, after) {
            (Some(b), Some(a)) => if step - b <= a - step { b } else { a },
            (Some(one), None) | (None, Some(one)) => one,
            (None, None) => return None,
        };
        let cached = self.ready.get_mut(&key)?;
        cached.last_used = now;
        Some(cached.frame.clone())
    }
}

/// One thumbnail to decode.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Job {
    path: PathBuf,
    step: i64,
}

#[derive(Default)]
struct State {
    files: HashMap<PathBuf, FileThumbnails>,
    /// Waiting jobs, newest at the front.
    queue: VecDeque<Job>,
    /// The job the worker is decoding right now, so it is not queued a second time.
    in_progress: Option<Job>,
    /// Counts requests, giving each use of a thumbnail an order for eviction.
    clock: u64,
    shutting_down: bool,
}

impl State {
    /// Puts `job` at the front of the queue (moving it there if it was already waiting), and
    /// drops the oldest requests if the queue is now too long.
    fn queue_newest(&mut self, job: Job) {
        if self.in_progress.as_ref() == Some(&job) {
            return;
        }
        if let Some(position) = self.queue.iter().position(|queued| *queued == job) {
            self.queue.remove(position);
        }
        self.queue.push_front(job);
        self.queue.truncate(MAX_QUEUED);
    }

    /// Stores a decoded thumbnail, then frees the least recently used ones beyond the limit.
    fn insert(&mut self, job: Job, frame: VideoFrame) {
        self.clock += 1;
        let cached = Cached { frame: Arc::new(frame), last_used: self.clock };
        self.files.entry(job.path).or_default().ready.insert(job.step, cached);
        while self.thumbnail_count() > MAX_THUMBNAILS {
            self.evict_least_recently_used();
        }
    }

    fn thumbnail_count(&self) -> usize {
        self.files.values().map(|file| file.ready.len()).sum()
    }

    /// Frees the one thumbnail asked for longest ago. A linear search, but over at most a few
    /// thousand entries, which costs far less than the decode that preceded it.
    fn evict_least_recently_used(&mut self) {
        let oldest = self
            .files
            .iter()
            .flat_map(|(path, file)| file.ready.iter().map(move |(step, cached)| (cached.last_used, path, *step)))
            .min_by_key(|(last_used, ..)| *last_used)
            .map(|(_, path, step)| (path.clone(), step));
        if let Some((path, step)) = oldest
            && let Some(file) = self.files.get_mut(&path)
        {
            file.ready.remove(&step);
        }
    }
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the worker when there is a new job or the thumbnailer is being dropped.
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

/// Decodes and caches filmstrip thumbnails of video files on a background thread.
pub struct Thumbnailer {
    shared: Arc<Shared>,
}

impl Thumbnailer {
    /// Thumbnails are `height` pixels tall, as wide as the picture's shape needs (rounded to an
    /// even width). Sources shorter than `height` are scaled up to it.
    pub fn new(height: u32) -> Self {
        let shared = Arc::new(Shared { state: Mutex::default(), work: Condvar::new(), on_ready: Mutex::new(None) });
        let worker_shared = shared.clone();
        let height = height.max(2);
        if let Err(error) = std::thread::Builder::new().name("thumbnails".into()).spawn(move || worker(&worker_shared, height)) {
            eprintln!("could not start the thumbnail thread: {error}");
        }
        Self { shared }
    }

    /// Called (from the worker thread) whenever a new thumbnail is ready, so the timeline can redraw.
    pub fn set_on_ready(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_ready.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(callback));
    }

    /// The thumbnail of `path` at `seconds` from its start, or the nearest one already decoded
    /// while the exact one is on its way. Never waits: a missing picture is queued for decoding.
    /// `None` if nothing for this file is decoded yet, or the file has no usable video.
    pub fn thumbnail(&self, path: &Path, seconds: f64) -> Option<Arc<VideoFrame>> {
        let step = step_of(seconds);
        let mut state = self.shared.lock();
        state.clock += 1;
        let now = state.clock;
        let file = state.files.entry(path.to_owned()).or_default();
        if file.failed {
            return None;
        }
        let exact = file.ready.contains_key(&step);
        let wanted = !exact && !file.failed_steps.contains(&step);
        let found = file.nearest(step, now);
        if wanted {
            state.queue_newest(Job { path: path.to_owned(), step });
            self.shared.work.notify_one();
        }
        found
    }
}

impl Drop for Thumbnailer {
    fn drop(&mut self) {
        self.shared.lock().shutting_down = true;
        self.shared.work.notify_all();
    }
}

/// The quarter-second step nearest to `seconds`. Times before the start (or not a number) are the start.
fn step_of(seconds: f64) -> i64 {
    if seconds.is_finite() { (seconds.max(0.0) * STEPS_PER_SECOND).round() as i64 } else { 0 }
}

/// An open file, ready to decode thumbnails from.
struct OpenFile {
    decoder: VideoDecoder,
    frame_rate: FrameRate,
    /// The thumbnail size for this file's picture shape.
    size: (u32, u32),
    last_used: u64,
}

impl OpenFile {
    /// Opens `path` and works out the thumbnail size: `height` tall, keeping the picture's
    /// shape, with an even width. Decoding is in software, which is quicker to start and plenty
    /// fast for pictures this small.
    fn open(path: &Path, height: u32) -> Result<Self, crate::decode::MediaError> {
        let info = probe(path)?;
        let width = ((f64::from(height) * f64::from(info.width) / f64::from(info.height.max(1))).round() as u32).max(2) & !1;
        let decoder = VideoDecoder::open(path, width, false)?;
        Ok(Self { decoder, frame_rate: info.frame_rate, size: (width, height), last_used: 0 })
    }

    /// Decodes the thumbnail for time step `step`.
    fn thumbnail(&mut self, step: i64) -> Result<VideoFrame, crate::decode::MediaError> {
        let index = self.frame_rate.seconds_to_frames(step as f64 / STEPS_PER_SECOND);
        let frame = self.decoder.frame(index)?;
        Ok(fit(frame, self.size))
    }
}

/// Resizes `frame` to exactly `size` if the decoder's rounding (or a source smaller than the
/// thumbnail) left it slightly off. Nearest-neighbour is enough for a difference of a pixel or
/// two, and keeps filmstrip thumbnails all the same height.
fn fit(frame: VideoFrame, (width, height): (u32, u32)) -> VideoFrame {
    if (frame.width, frame.height) == (width, height) || frame.width == 0 || frame.height == 0 {
        return frame;
    }
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        let source_y = (u64::from(y) * u64::from(frame.height) / u64::from(height)) as usize;
        for x in 0..width {
            let source_x = (u64::from(x) * u64::from(frame.width) / u64::from(width)) as usize;
            let at = (source_y * frame.width as usize + source_x) * 4;
            rgba.extend_from_slice(&frame.rgba[at..at + 4]);
        }
    }
    VideoFrame { width, height, rgba, ..frame }
}

/// Waits for the next job, newest first. `None` once the thumbnailer is dropped.
fn next_job(shared: &Shared) -> Option<Job> {
    let mut state = shared.lock();
    loop {
        if state.shutting_down {
            return None;
        }
        if let Some(job) = state.queue.pop_front() {
            state.in_progress = Some(job.clone());
            return Some(job);
        }
        state = shared.work.wait(state).unwrap_or_else(|p| p.into_inner());
    }
}

/// Decodes queued thumbnails until the thumbnailer is dropped. Decoders stay open between jobs
/// (this thread owns them), so a filmstrip along one clip reuses one decoder.
fn worker(shared: &Shared, height: u32) {
    let mut open: HashMap<PathBuf, OpenFile> = HashMap::new();
    let mut uses = 0_u64;
    while let Some(job) = next_job(shared) {
        uses += 1;
        if !open.contains_key(&job.path) {
            match OpenFile::open(&job.path, height) {
                Ok(file) => {
                    close_least_recently_used(&mut open);
                    open.insert(job.path.clone(), file);
                }
                Err(_) => {
                    // Remember the failure and drop the file's other waiting jobs: none can succeed.
                    let mut state = shared.lock();
                    state.files.entry(job.path.clone()).or_default().failed = true;
                    state.queue.retain(|queued| queued.path != job.path);
                    state.in_progress = None;
                    continue;
                }
            }
        }
        let file = open.get_mut(&job.path).expect("opened above");
        file.last_used = uses;
        let decoded = file.thumbnail(job.step);
        {
            let mut state = shared.lock();
            state.in_progress = None;
            match decoded {
                Ok(frame) => state.insert(job, frame),
                Err(_) => {
                    state.files.entry(job.path).or_default().failed_steps.insert(job.step);
                    continue;
                }
            }
        }
        shared.notify_ready();
    }
}

/// Closes the least recently used decoder if opening another would exceed `MAX_OPEN_DECODERS`.
fn close_least_recently_used(open: &mut HashMap<PathBuf, OpenFile>) {
    if open.len() < MAX_OPEN_DECODERS {
        return;
    }
    let oldest = open.iter().min_by_key(|(_, file)| file.last_used).map(|(path, _)| path.clone());
    if let Some(path) = oldest {
        open.remove(&path);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn frame(index: i64) -> VideoFrame {
        VideoFrame { width: 2, height: 2, index, rgba: vec![0; 16], from_proxy: false, has_alpha: false }
    }

    #[test]
    fn times_round_to_quarter_seconds() {
        assert_eq!(step_of(0.0), 0);
        assert_eq!(step_of(0.12), 0);
        assert_eq!(step_of(0.13), 1);
        assert_eq!(step_of(1.25), 5);
        assert_eq!(step_of(-3.0), 0);
        assert_eq!(step_of(f64::NAN), 0);
    }

    #[test]
    fn the_queue_serves_newest_first_and_drops_the_oldest() {
        let mut state = State::default();
        let job = |step| Job { path: PathBuf::from("a.mp4"), step };
        for step in 0..MAX_QUEUED as i64 + 10 {
            state.queue_newest(job(step));
        }
        assert_eq!(state.queue.len(), MAX_QUEUED);
        assert_eq!(state.queue.front(), Some(&job(MAX_QUEUED as i64 + 9)));
        assert_eq!(state.queue.back(), Some(&job(10)), "the ten oldest were dropped");
        // Asking again for a waiting job moves it to the front rather than adding a copy.
        state.queue_newest(job(50));
        assert_eq!(state.queue.front(), Some(&job(50)));
        assert_eq!(state.queue.len(), MAX_QUEUED);
    }

    #[test]
    fn the_least_recently_used_thumbnails_are_freed_first() {
        let mut state = State::default();
        let path = PathBuf::from("a.mp4");
        for step in 0..MAX_THUMBNAILS as i64 {
            state.insert(Job { path: path.clone(), step }, frame(step));
        }
        // Using step 0 again makes step 1 the oldest.
        state.clock += 1;
        let now = state.clock;
        state.files.get_mut(&path).unwrap().nearest(0, now);
        state.insert(Job { path: path.clone(), step: 99_999 }, frame(0));
        let ready = &state.files[&path].ready;
        assert_eq!(ready.len(), MAX_THUMBNAILS);
        assert!(ready.contains_key(&0) && !ready.contains_key(&1));
    }

    #[test]
    fn the_nearest_thumbnail_stands_in_for_a_missing_one() {
        let mut file = FileThumbnails::default();
        for step in [0, 8] {
            file.ready.insert(step, Cached { frame: Arc::new(frame(step)), last_used: 0 });
        }
        assert_eq!(file.nearest(3, 1).unwrap().index, 0);
        assert_eq!(file.nearest(5, 1).unwrap().index, 8);
        assert_eq!(file.nearest(8, 1).unwrap().index, 8);
    }

    #[test]
    fn small_differences_in_size_are_fitted_exactly() {
        let source = VideoFrame { width: 4, height: 2, index: 0, rgba: (0..32).collect(), from_proxy: false, has_alpha: false };
        let fitted = fit(source, (2, 1));
        assert_eq!((fitted.width, fitted.height), (2, 1));
        assert_eq!(fitted.rgba, vec![0, 1, 2, 3, 8, 9, 10, 11]);
    }

    #[test]
    fn a_file_without_video_is_marked_failed_and_never_queued_again() {
        let sound = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        let thumbnailer = Thumbnailer::new(18);
        assert!(thumbnailer.thumbnail(&sound, 0.0).is_none());
        let started = Instant::now();
        while !thumbnailer.shared.lock().files.get(&sound).is_some_and(|file| file.failed) {
            assert!(started.elapsed() < Duration::from_secs(5), "the failure was never recorded");
            std::thread::sleep(Duration::from_millis(5));
        }
        for seconds in [0.0, 0.5, 1.0] {
            assert!(thumbnailer.thumbnail(&sound, seconds).is_none());
        }
        let state = thumbnailer.shared.lock();
        assert!(state.queue.is_empty() && state.in_progress.is_none(), "nothing waits on the failed file");
    }

    #[test]
    fn dropping_the_thumbnailer_stops_its_thread() {
        let clip = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/colors.mp4");
        let thumbnailer = Thumbnailer::new(18);
        let started = Instant::now();
        while thumbnailer.thumbnail(&clip, 0.0).is_none() {
            assert!(started.elapsed() < Duration::from_secs(5), "no thumbnail arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        let shared = Arc::downgrade(&thumbnailer.shared);
        assert_eq!(shared.strong_count(), 2, "the worker holds the shared state");
        drop(thumbnailer);
        let started = Instant::now();
        while shared.strong_count() > 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "the thumbnail thread never stopped");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
