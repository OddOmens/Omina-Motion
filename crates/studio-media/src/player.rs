//! Plays a composition's mixed audio through the sound card, and reports how much of it has
//! actually been heard. Playback reads that as its clock, so the picture follows the sound
//! instead of drifting against it.
//!
//! `AudioOutput` keeps one output stream open for the life of the app, paused while nothing
//! plays: opening a sound device takes ~100 ms, un-pausing it a few. Each playback gets its
//! own lock-free queue, which a background thread keeps about 100 ms ahead of the sound card.
//! The sound card's callback only copies samples out of that queue, so it never waits on a
//! decoder or a lock.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, OutputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};
use studio_core::audio::AudioClip;

use crate::audio::CHANNELS;
use crate::decode::MediaError;
use crate::mixer::{MixRange, Mixer};

/// Sample frames mixed at a time.
const BLOCK_FRAMES: usize = 512;
/// How far ahead of the sound card the mix runs. Short enough that muting a layer is heard
/// quickly, long enough to ride out a busy moment on the mixing thread.
const AHEAD_SECONDS: f64 = 0.1;
/// How often the mixing thread tops up the queue while playing.
const TOP_UP_INTERVAL: Duration = Duration::from_millis(5);
/// The sound card buffer asked for, in frames (about 12 ms at 44.1 kHz). Smaller buffers make
/// sound start sooner after pressing play.
const DEVICE_BUFFER_FRAMES: u32 = 512;
/// How long the stream keeps running (silently) after playback stops. Un-pausing a device
/// costs tens of milliseconds, so quickly playing again skips that.
const IDLE_BEFORE_PAUSE: Duration = Duration::from_secs(10);

/// What to play: the composition's clips, from `start_seconds`, inside a range that loops or not.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioTimeline {
    pub clips: Vec<AudioClip>,
    pub start_seconds: f64,
    /// The playback range (the work area) in timeline seconds.
    pub range_seconds: (f64, f64),
    pub looping: bool,
}

/// How much audio has been heard, from what the sound card reports on each callback.
struct Clock {
    created: Instant,
    /// The sound card's sample rate; 0 until it is known.
    rate: f64,
    /// Frames of mix the sound card took before its latest callback, how many it took then,
    /// and when the first of those reaches the speakers.
    taken: u64,
    latest_block: u64,
    plays_at: Option<Instant>,
    /// Once the sound stops (the mix ended, or the device failed): the seconds heard and when.
    /// From then on the clock runs on by wall-clock time, so playback still advances.
    stopped: Option<(f64, Instant)>,
    /// The largest value reported, so the playhead never steps backwards on timing jitter.
    reported: f64,
}

impl Clock {
    fn new(created: Instant) -> Self {
        Self { created, rate: 0.0, taken: 0, latest_block: 0, plays_at: None, stopped: None, reported: 0.0 }
    }

    fn heard(&mut self, now: Instant) -> f64 {
        let seconds = if let Some((seconds, at)) = self.stopped {
            seconds + now.saturating_duration_since(at).as_secs_f64()
        } else if let Some(plays_at) = self.plays_at {
            // Negative while earlier blocks are still playing; capped at the end of the latest block.
            let since = if now >= plays_at { (now - plays_at).as_secs_f64() } else { -(plays_at - now).as_secs_f64() };
            let frames = self.taken as f64 + (since * self.rate).min(self.latest_block as f64);
            frames.max(0.0) / self.rate
        } else {
            0.0 // The sound card has not started yet; playback waits for it.
        };
        self.reported = self.reported.max(seconds);
        self.reported
    }

    fn stop(&mut self, seconds: f64, at: Instant) {
        if self.stopped.is_none() {
            self.stopped = Some((seconds, at));
        }
    }
}

/// What one playback shares between the app, the mixing thread and the sound card's callback.
struct Session {
    id: u64,
    clock: Mutex<Clock>,
    clips: Mutex<Vec<AudioClip>>,
    /// Bumped whenever `clips` changes, so the mixing thread copies them only then.
    clips_version: AtomicU64,
    /// Set when the mix reached the end of a range that does not loop.
    finished: AtomicBool,
    error: Mutex<Option<String>>,
    /// The loudest sample mixed on the left and on the right since the meter last looked, as
    /// the bits of a non-negative `f32` (which sort the same way the numbers do).
    peaks: [AtomicU32; 2],
}

impl Session {
    fn clock(&self) -> MutexGuard<'_, Clock> {
        self.clock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records why there is no sound; playback carries on by the wall clock from here.
    fn fail(&self, error: String) {
        let now = Instant::now();
        let mut clock = self.clock();
        let heard = clock.heard(now);
        clock.stop(heard, now);
        drop(clock);
        *self.error.lock().unwrap_or_else(|p| p.into_inner()) = Some(error);
    }
}

/// Requests from the app to the mixing thread.
enum Command {
    Play(Arc<Session>, AudioTimeline),
    Stop(u64),
}

/// The queue the sound card's callback plays from, handed over when a playback starts or stops.
enum Handoff {
    Play(Arc<Session>, Consumer<f32>),
    Stop,
}

/// The sound output. Create one per app; it opens the device in the background right away so
/// the first playback starts quickly too.
pub struct AudioOutput {
    commands: Sender<Command>,
    next_session: AtomicU64,
}

impl AudioOutput {
    pub fn new() -> Self {
        let (commands, received) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("audio mix".into()).spawn(move || mix_thread(&received));
        if let Err(error) = spawned {
            // Every playback will report this and run on the wall clock.
            eprintln!("could not start the audio thread: {error}");
        }
        Self { commands, next_session: AtomicU64::new(1) }
    }

    /// Starts playing `timeline`. Returns at once; the clock stays at zero until sound comes
    /// out. Dropping the returned player stops the sound.
    pub fn play(&self, timeline: AudioTimeline) -> AudioPlayer {
        let session = Arc::new(Session {
            id: self.next_session.fetch_add(1, Ordering::Relaxed),
            clock: Mutex::new(Clock::new(Instant::now())),
            clips: Mutex::new(timeline.clips.clone()),
            clips_version: AtomicU64::new(0),
            finished: AtomicBool::new(false),
            error: Mutex::new(None),
            peaks: [AtomicU32::new(0), AtomicU32::new(0)],
        });
        if self.commands.send(Command::Play(session.clone(), timeline)).is_err() {
            let created = session.clock().created;
            session.clock().stop(0.0, created);
            *session.error.lock().unwrap_or_else(|p| p.into_inner()) = Some("the audio thread is not running".into());
        }
        AudioPlayer { session, commands: self.commands.clone() }
    }
}

impl Default for AudioOutput {
    fn default() -> Self {
        Self::new()
    }
}

/// One playback in progress. Dropping it stops the sound.
pub struct AudioPlayer {
    session: Arc<Session>,
    commands: Sender<Command>,
}

impl AudioPlayer {
    /// Replaces the clips being played, for example after a layer is muted. Heard within ~100 ms.
    pub fn set_clips(&self, clips: Vec<AudioClip>) {
        *self.session.clips.lock().unwrap_or_else(|p| p.into_inner()) = clips;
        self.session.clips_version.fetch_add(1, Ordering::Release);
    }

    /// How long this playback has been heard: the playhead's clock while playing.
    pub fn heard(&self) -> Duration {
        Duration::from_secs_f64(self.session.clock().heard(Instant::now()))
    }

    /// The loudest level mixed on the left and on the right since this was last called (1.0 is
    /// full scale; more than that clips). For level meters: call it once per screen frame. The
    /// mix runs about a tenth of a second ahead of the speakers, so the meter leads by that.
    pub fn take_peaks(&self) -> [f32; 2] {
        [0, 1].map(|side| f32::from_bits(self.session.peaks[side].swap(0, Ordering::Relaxed)))
    }

    /// Why there is no sound, if the device could not be used.
    pub fn error(&self) -> Option<String> {
        self.session.error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        // The thread is gone only if the whole output is; then there is nothing to stop.
        let _ = self.commands.send(Command::Stop(self.session.id));
    }
}

/// The open sound device: its stream (paused while idle), the slot its callback picks up new
/// queues from, and its format.
struct Device {
    stream: Stream,
    handoff: Arc<Mutex<Option<Handoff>>>,
    rate: u32,
    /// Set by the stream's error callback; the device is reopened on the next playback.
    broken: Arc<AtomicBool>,
}

impl Device {
    fn open() -> Result<Self, MediaError> {
        let device = cpal::default_host().default_output_device().ok_or_else(|| MediaError("no sound output device".into()))?;
        let supported = device.default_output_config().map_err(|error| MediaError(error.to_string()))?;
        let format = supported.sample_format();
        let buffer_size = match *supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => cpal::BufferSize::Fixed(DEVICE_BUFFER_FRAMES.clamp(min, max)),
            cpal::SupportedBufferSize::Unknown => cpal::BufferSize::Default,
        };
        let config = StreamConfig { buffer_size, ..supported.into() };
        let handoff = Arc::new(Mutex::new(None));
        let broken = Arc::new(AtomicBool::new(false));
        let stream = match format {
            SampleFormat::F32 => open_stream::<f32>(&device, &config, &handoff, &broken),
            SampleFormat::I16 => open_stream::<i16>(&device, &config, &handoff, &broken),
            SampleFormat::I32 => open_stream::<i32>(&device, &config, &handoff, &broken),
            SampleFormat::U16 => open_stream::<u16>(&device, &config, &handoff, &broken),
            other => Err(MediaError(format!("the sound output uses an unsupported sample format ({other})"))),
        }?;
        // Some systems start a stream as soon as it is built; stay silent and idle until asked.
        let _ = stream.pause();
        Ok(Self { stream, handoff, rate: config.sample_rate, broken })
    }

    fn hand_off(&self, handoff: Handoff) {
        *self.handoff.lock().unwrap_or_else(|p| p.into_inner()) = Some(handoff);
    }
}

/// The playback being mixed: its queue's input end and the mixer feeding it.
struct Playing {
    session: Arc<Session>,
    producer: Producer<f32>,
    mixer: Mixer,
    clips: Vec<AudioClip>,
    clips_version: u64,
}

/// The mixing thread: owns the device, starts and stops playbacks, and keeps the current
/// playback's queue topped up.
fn mix_thread(commands: &Receiver<Command>) {
    let mut device = Device::open().map_err(|error| eprintln!("sound output unavailable: {error}")).ok();
    let mut playing: Option<Playing> = None;
    let mut block = vec![0.0; BLOCK_FRAMES * CHANNELS];
    // While the stream runs silently after a stop: when it should pause.
    let mut pause_at: Option<Instant> = None;
    loop {
        // Wake regularly to top up while playing; otherwise sleep until told something, or
        // until it is time to pause an idle stream.
        let wait = match (&playing, pause_at) {
            (Some(_), _) => Some(TOP_UP_INTERVAL),
            (None, Some(at)) => Some(at.saturating_duration_since(Instant::now())),
            (None, None) => None,
        };
        let command = match wait {
            Some(timeout) => match commands.recv_timeout(timeout) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            },
        };
        match command {
            Some(Command::Play(session, timeline)) => {
                if device.as_ref().is_none_or(|open| open.broken.load(Ordering::Acquire)) {
                    // Close a broken stream before opening a new one.
                    drop(device.take());
                    device = Device::open().map_err(|error| session.fail(error.to_string())).ok();
                }
                pause_at = None;
                playing = device.as_ref().and_then(|open| start(open, session, &timeline, &mut block));
            }
            Some(Command::Stop(id)) => {
                if playing.as_ref().is_some_and(|current| current.session.id == id) {
                    playing = None;
                    if let Some(open) = device.as_ref() {
                        open.hand_off(Handoff::Stop);
                        pause_at = Some(Instant::now() + IDLE_BEFORE_PAUSE);
                    }
                }
            }
            None => {
                if playing.is_none() && pause_at.is_some_and(|at| Instant::now() >= at) {
                    pause_at = None;
                    if let Some(open) = device.as_ref() {
                        let _ = open.stream.pause();
                    }
                }
            }
        }
        // A device that fails mid-playback (unplugged, switched) stops calling back; let the
        // playhead run on by the wall clock, and reopen the device on the next playback.
        if let (Some(current), Some(open)) = (playing.as_ref(), device.as_ref())
            && open.broken.load(Ordering::Acquire)
        {
            current.session.fail("the sound output stopped".into());
            playing = None;
        }
        if let Some(current) = playing.as_mut() {
            let latest = current.session.clips_version.load(Ordering::Acquire);
            if latest != current.clips_version {
                current.clips_version = latest;
                current.clips = current.session.clips.lock().unwrap_or_else(|p| p.into_inner()).clone();
            }
            top_up(current, &mut block);
        }
    }
}

/// Starts a playback on the open device: a fresh queue, filled before the stream un-pauses so
/// the first callback already has sound.
fn start(device: &Device, session: Arc<Session>, timeline: &AudioTimeline, block: &mut [f32]) -> Option<Playing> {
    let rate = f64::from(device.rate);
    let to_frame = |seconds: f64| (seconds * rate).round() as i64;
    let range = MixRange { start: to_frame(timeline.range_seconds.0), end: to_frame(timeline.range_seconds.1), looping: timeline.looping };
    let capacity = ((rate * AHEAD_SECONDS) as usize).max(BLOCK_FRAMES * 2) * CHANNELS;
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    let mut playing = Playing {
        session,
        producer,
        mixer: Mixer::new(device.rate, range, to_frame(timeline.start_seconds)),
        clips: timeline.clips.clone(),
        clips_version: 0,
    };
    top_up(&mut playing, block);
    playing.session.clock().rate = rate;
    device.hand_off(Handoff::Play(playing.session.clone(), consumer));
    if let Err(error) = device.stream.play() {
        playing.session.fail(error.to_string());
        return None;
    }
    Some(playing)
}

/// Mixes blocks into the playback's queue until it is full or the mix has ended.
fn top_up(playing: &mut Playing, block: &mut [f32]) {
    let session = &playing.session;
    while !session.finished.load(Ordering::Relaxed) && playing.producer.slots() >= block.len() {
        let frames = playing.mixer.mix(&playing.clips, block);
        // Note each side's loudest sample for the level meters.
        for frame in block[..frames * CHANNELS].chunks_exact(CHANNELS) {
            for (side, sample) in frame.iter().enumerate().take(2) {
                session.peaks[side].fetch_max(sample.abs().to_bits(), Ordering::Relaxed);
            }
        }
        if let Ok(chunk) = playing.producer.write_chunk_uninit(frames * CHANNELS) {
            chunk.fill_from_iter(block[..frames * CHANNELS].iter().copied());
        }
        if frames < block.len() / CHANNELS {
            session.finished.store(true, Ordering::Release);
        }
    }
}

/// Builds the output stream in the device's own sample type. The callback copies queued stereo
/// frames to the device's channels (mono devices get both sides averaged), plays silence when
/// there is nothing queued, and tells the playback's clock how much real sound it handed over
/// and when that will be heard.
fn open_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    handoff: &Arc<Mutex<Option<Handoff>>>,
    broken: &Arc<AtomicBool>,
) -> Result<Stream, MediaError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let handoff = handoff.clone();
    let mut current: Option<(Arc<Session>, Consumer<f32>)> = None;
    let mut taken: u64 = 0;
    let data_callback = move |data: &mut [T], info: &OutputCallbackInfo| {
        // Pick up a new or stopped playback. Never wait for the lock: if the mixing thread
        // holds it, the next callback picks it up instead.
        if let Some(next) = handoff.try_lock().ok().and_then(|mut slot| slot.take()) {
            current = match next {
                Handoff::Play(session, consumer) => Some((session, consumer)),
                Handoff::Stop => None,
            };
            taken = 0;
        }
        let frames = data.len() / channels;
        let Some((session, consumer)) = current.as_mut() else {
            data.iter_mut().for_each(|sample| *sample = T::from_sample(0.0));
            return;
        };

        let real = (consumer.slots() / CHANNELS).min(frames);
        let queued = consumer.read_chunk(real * CHANNELS).ok();
        let (first, second) = queued.as_ref().map_or((&[][..], &[][..]), |chunk| chunk.as_slices());
        let mut samples = first.iter().chain(second).copied();
        for frame in data.chunks_mut(channels) {
            let (left, right) = samples.next().zip(samples.next()).unwrap_or((0.0, 0.0));
            write_frame(frame, left, right);
        }
        if let Some(chunk) = queued {
            chunk.commit_all();
        }

        let timestamp = info.timestamp();
        let latency = timestamp.playback.checked_duration_since(timestamp.callback).unwrap_or_default();
        let plays_at = Instant::now() + latency;
        if let Ok(mut clock) = session.clock.try_lock() {
            clock.taken = taken;
            clock.latest_block = real as u64;
            clock.plays_at = Some(plays_at);
            if real < frames && session.finished.load(Ordering::Acquire) {
                // The mix has ended: the clock runs on from the last real sample.
                let rate = clock.rate.max(1.0);
                let end = (taken + real as u64) as f64 / rate;
                clock.stop(end, plays_at + Duration::from_secs_f64(real as f64 / rate));
            }
        }
        taken += real as u64;
    };
    let broken = broken.clone();
    let error_callback = move |error: cpal::Error| {
        // A dropout or a denied real-time priority is a glitch, not the end of the stream.
        if !matches!(error.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied) {
            eprintln!("sound output stopped: {error}");
            broken.store(true, Ordering::Release);
        }
    };
    device.build_output_stream(*config, data_callback, error_callback, None).map_err(|error| MediaError(error.to_string()))
}

/// Writes one stereo frame to a device frame with any number of channels.
fn write_frame<T: SizedSample + FromSample<f32>>(frame: &mut [T], left: f32, right: f32) {
    match frame {
        [only] => *only = T::from_sample((left + right) * 0.5),
        [first, second, rest @ ..] => {
            *first = T::from_sample(left);
            *second = T::from_sample(right);
            for extra in rest {
                *extra = T::from_sample(0.0);
            }
        }
        [] => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(created: Instant) -> Clock {
        Clock { rate: 1000.0, ..Clock::new(created) }
    }

    #[test]
    fn waits_at_zero_until_the_sound_card_starts() {
        let now = Instant::now();
        let mut clock = clock(now);
        assert_eq!(clock.heard(now + Duration::from_millis(50)), 0.0);
    }

    #[test]
    fn follows_the_samples_reaching_the_speakers() {
        let start = Instant::now();
        let mut clock = clock(start);
        // 500 frames (0.5 s at 1 kHz) were taken before; the next 100 start playing at `plays_at`.
        let plays_at = start + Duration::from_millis(600);
        (clock.taken, clock.latest_block, clock.plays_at) = (500, 100, Some(plays_at));
        assert!((clock.heard(plays_at - Duration::from_millis(20)) - 0.48).abs() < 1e-9, "earlier audio still playing");
        assert!((clock.heard(plays_at + Duration::from_millis(40)) - 0.54).abs() < 1e-9);
        // Never runs past the audio handed over so far.
        assert!((clock.heard(plays_at + Duration::from_millis(500)) - 0.6).abs() < 1e-9);
    }

    #[test]
    fn never_steps_backwards() {
        let start = Instant::now();
        let mut clock = clock(start);
        (clock.taken, clock.latest_block, clock.plays_at) = (500, 100, Some(start));
        let ahead = clock.heard(start + Duration::from_millis(50));
        // A later callback that predicts a slightly later play time must not pull the playhead back.
        clock.plays_at = Some(start + Duration::from_millis(20));
        assert_eq!(clock.heard(start + Duration::from_millis(51)), ahead);
    }

    #[test]
    fn runs_on_by_the_wall_clock_once_the_sound_stops() {
        let start = Instant::now();
        let mut clock = clock(start);
        clock.stop(2.0, start + Duration::from_secs(3));
        assert_eq!(clock.heard(start + Duration::from_secs(3)), 2.0);
        assert!((clock.heard(start + Duration::from_millis(3250)) - 2.25).abs() < 1e-9);
        // The first stop wins; a later failure report does not move it.
        clock.stop(9.0, start);
        assert!((clock.heard(start + Duration::from_millis(3500)) - 2.5).abs() < 1e-9);
    }
}
