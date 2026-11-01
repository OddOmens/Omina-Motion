//! Waveforms for the timeline's audio clips: how loud each short stretch of a file is, worked
//! out on a background thread so drawing the timeline never waits on a decoder.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::audio::{AudioDecoder, CHANNELS};
use crate::decode::MediaError;

/// Waveform buckets per second that `PeakCache` computes: fine enough for close zooms.
const PEAKS_PER_SECOND: u32 = 100;
/// The rate audio is decoded at for waveforms. Far below playback quality, but a waveform only
/// needs the loudness of each bucket, and decoding to fewer samples is quicker.
const PEAK_SAMPLE_RATE: u32 = 8000;
/// Sample frames decoded per read.
const READ_FRAMES: usize = 4096;

type ReadyCallback = Box<dyn Fn() + Send + Sync>;

/// A file's waveform: the loudest sample in each `1 / per_second` stretch, from the file's time 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Peaks {
    pub per_second: u32,
    /// The peak absolute amplitude of each bucket (0 to 1, the louder of the two channels).
    pub values: Vec<f32>,
}

/// Works out the waveform of `path`'s sound, `per_second` buckets per second, for the whole
/// file. Time 0 is where the picture starts, as for playback (see `AudioDecoder::open`).
/// Blocks until the whole file is decoded; `PeakCache` runs it in the background.
pub fn audio_peaks(path: &Path, per_second: u32) -> Result<Peaks, MediaError> {
    audio_peaks_unless(path, per_second, &|| false).map(|peaks| peaks.expect("never cancelled"))
}

/// Like `audio_peaks`, but checks `cancelled` between reads and gives up (returning `Ok(None)`)
/// once it says so, so a long file does not hold up shutting down.
fn audio_peaks_unless(path: &Path, per_second: u32, cancelled: &dyn Fn() -> bool) -> Result<Option<Peaks>, MediaError> {
    if per_second == 0 {
        return Err(MediaError("a waveform needs at least one bucket per second".into()));
    }
    // Decode at least as many samples per second as there are buckets, so none is empty.
    let rate = PEAK_SAMPLE_RATE.max(per_second);
    let mut decoder = AudioDecoder::open(path, rate)?;
    let mut values: Vec<f32> = Vec::new();
    let mut block = vec![0.0_f32; READ_FRAMES * CHANNELS];
    // Sample frames read so far, to place each frame in its bucket.
    let mut position = 0_u64;
    loop {
        if cancelled() {
            return Ok(None);
        }
        let frames = decoder.read(&mut block)?;
        for frame in block[..frames * CHANNELS].as_chunks::<CHANNELS>().0 {
            let bucket = (position * u64::from(per_second) / u64::from(rate)) as usize;
            if bucket >= values.len() {
                values.resize(bucket + 1, 0.0);
            }
            let loudest = frame.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
            values[bucket] = values[bucket].max(loudest.min(1.0));
            position += 1;
        }
        if frames < READ_FRAMES {
            return Ok(Some(Peaks { per_second, values }));
        }
    }
}

/// What is known about one file's waveform.
enum Entry {
    /// Waiting for the worker, or being computed right now.
    Pending,
    Ready(Arc<Peaks>),
    /// The file could not be read (missing, damaged, or no sound). Never retried.
    Failed,
}

#[derive(Default)]
struct State {
    files: HashMap<PathBuf, Entry>,
    /// Files waiting for the worker, newest request at the front.
    queue: VecDeque<PathBuf>,
    shutting_down: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the worker when there is a new file or the cache is being dropped.
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

/// Computes and keeps the waveform of each audio file on a background thread.
pub struct PeakCache {
    shared: Arc<Shared>,
}

impl PeakCache {
    /// Starts the worker thread. Waveforms are computed at 100 buckets per second.
    pub fn new() -> Self {
        let shared = Arc::new(Shared { state: Mutex::default(), work: Condvar::new(), on_ready: Mutex::new(None) });
        let worker_shared = shared.clone();
        if let Err(error) = std::thread::Builder::new().name("waveforms".into()).spawn(move || worker(&worker_shared)) {
            eprintln!("could not start the waveform thread: {error}");
        }
        Self { shared }
    }

    /// Called (from the worker thread) whenever a waveform is ready, so the timeline can redraw.
    pub fn set_on_ready(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_ready.lock().unwrap_or_else(|p| p.into_inner()) = Some(Box::new(callback));
    }

    /// The waveform of `path`, once computed. Never waits: the first request queues the file.
    /// `None` while it is being computed, or for good if the file has no readable sound.
    pub fn peaks(&self, path: &Path) -> Option<Arc<Peaks>> {
        let mut state = self.shared.lock();
        match state.files.get(path) {
            Some(Entry::Ready(peaks)) => Some(peaks.clone()),
            Some(Entry::Pending | Entry::Failed) => None,
            None => {
                state.files.insert(path.to_owned(), Entry::Pending);
                state.queue.push_front(path.to_owned());
                self.shared.work.notify_one();
                None
            }
        }
    }
}

impl Default for PeakCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PeakCache {
    fn drop(&mut self) {
        self.shared.lock().shutting_down = true;
        self.shared.work.notify_all();
    }
}

/// Waits for the next file to work on, newest request first. `None` once the cache is dropped.
fn next_file(shared: &Shared) -> Option<PathBuf> {
    let mut state = shared.lock();
    loop {
        if state.shutting_down {
            return None;
        }
        if let Some(path) = state.queue.pop_front() {
            return Some(path);
        }
        state = shared.work.wait(state).unwrap_or_else(|p| p.into_inner());
    }
}

/// Computes queued waveforms until the cache is dropped.
fn worker(shared: &Shared) {
    while let Some(path) = next_file(shared) {
        let shutting_down = || shared.lock().shutting_down;
        let entry = match audio_peaks_unless(&path, PEAKS_PER_SECOND, &shutting_down) {
            Ok(Some(peaks)) => Entry::Ready(Arc::new(peaks)),
            Ok(None) => return,
            Err(_) => Entry::Failed,
        };
        shared.lock().files.insert(path, entry);
        shared.notify_ready();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn zero_buckets_per_second_is_refused() {
        let tone = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        assert!(audio_peaks(&tone, 0).is_err());
    }

    #[test]
    fn dropping_the_cache_stops_its_thread() {
        let tone = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        let cache = PeakCache::new();
        let started = Instant::now();
        while cache.peaks(&tone).is_none() {
            assert!(started.elapsed() < Duration::from_secs(5), "no waveform arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        let shared = Arc::downgrade(&cache.shared);
        assert_eq!(shared.strong_count(), 2, "the worker holds the shared state");
        drop(cache);
        let started = Instant::now();
        while shared.strong_count() > 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "the waveform thread never stopped");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
