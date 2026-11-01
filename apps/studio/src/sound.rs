//! Keeps the sound in step with playback. It starts the audio when playback starts and stops
//! it when playback stops, passes on layer changes (mute, solo, moved or deleted layers) while
//! playing, and hands the playhead the time actually heard, so the picture follows the sound.

use std::time::{Duration, Instant};

use studio_core::audio::{AudioClip, composition_audio};
use studio_core::model::Id;
use studio_core::time::{FrameRange, FrameRate};
use studio_media::{AudioOutput, AudioPlayer, AudioTimeline};

use crate::state::EditorState;

/// Everything the sound was started from. If any of it changes mid-playback, the sound has to
/// start over to stay in step with the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Session {
    started_at: Instant,
    start_frame: i64,
    composition: Id,
    frame_rate: FrameRate,
    work_area: FrameRange,
    looping: bool,
}

impl Session {
    fn of(state: &EditorState, started_at: Instant, start_frame: i64) -> Self {
        Self {
            started_at,
            start_frame,
            composition: state.active_comp,
            frame_rate: state.composition().frame_rate,
            work_area: state.work_area,
            looping: state.loop_playback,
        }
    }

    fn is_same_playback(&self, other: &Self) -> bool {
        (self.started_at, self.start_frame) == (other.started_at, other.start_frame)
    }
}

struct Playing {
    player: AudioPlayer,
    session: Session,
    clips: Vec<AudioClip>,
    /// Where the sound was started from, for tests to check against the picture.
    #[cfg_attr(not(test), allow(dead_code))]
    start_seconds: f64,
}

pub struct Sound {
    /// `None` plays no sound; playback then runs on the wall clock.
    output: Option<AudioOutput>,
    playing: Option<Playing>,
    /// A file playing in the Project panel's preview, apart from the composition.
    preview: Option<AudioPlayer>,
}

impl Sound {
    /// Sound through the default output device, opened in the background right away.
    pub fn new() -> Self {
        Self { output: Some(AudioOutput::new()), playing: None, preview: None }
    }

    /// No sound at all, for headless tests.
    #[cfg(test)]
    pub fn silent() -> Self {
        Self { output: None, playing: None, preview: None }
    }

    /// Plays `path`'s sound from `from_seconds` to its end (`length` seconds in all), for the
    /// Project panel's preview. Stops any earlier preview.
    pub fn play_preview(&mut self, path: &std::path::Path, from_seconds: f64, length: f64) {
        self.preview = None;
        let Some(output) = self.output.as_ref() else { return };
        let clip = AudioClip { layer: Id(0), path: path.to_path_buf(), offset_seconds: 0.0, start_seconds: 0.0, end_seconds: length, gain: 1.0, fade_in: 0.0, fade_out: 0.0 };
        let timeline = AudioTimeline { clips: vec![clip], start_seconds: from_seconds, range_seconds: (0.0, length), looping: false };
        self.preview = Some(output.play(timeline));
    }

    pub fn stop_preview(&mut self) {
        self.preview = None; // Dropping the player stops the sound.
    }

    /// How long the preview's sound has been heard, while one plays.
    pub fn preview_heard(&self) -> Option<Duration> {
        self.preview.as_ref().filter(|player| player.error().is_none()).map(AudioPlayer::heard)
    }

    /// Brings the sound in line with `state`. Call it whenever playback may have started,
    /// stopped or changed. While playing, returns how long the current playback has been
    /// heard: the playhead's clock.
    pub fn sync(&mut self, state: &mut EditorState) -> Option<Duration> {
        let Some((started_at, start_frame)) = state.playback_started() else {
            self.playing = None; // Dropping the player stops the sound.
            return None;
        };
        // The composition playing takes over from a preview.
        self.preview = None;
        let output = self.output.as_ref()?;
        let mut session = Session::of(state, started_at, start_frame);
        if let Some(playing) = &self.playing
            && playing.session.is_same_playback(&session)
            && playing.session != session
        {
            // The work area, looping or frame rate changed mid-playback: start over from the
            // playhead so the sound and the picture agree on where they are.
            state.restart_playback(Instant::now());
            let (started_at, start_frame) = state.playback_started()?;
            session = Session::of(state, started_at, start_frame);
        }

        let clips = composition_audio(&state.project, state.composition());
        match &mut self.playing {
            Some(playing) if playing.session == session => {
                if playing.clips != clips {
                    playing.player.set_clips(clips.clone());
                    playing.clips = clips;
                }
            }
            _ => {
                let fps = session.frame_rate.fps();
                // From the session's start frame, which a restart above may have moved.
                let start_seconds = session.start_frame as f64 / fps;
                let timeline = AudioTimeline {
                    clips: clips.clone(),
                    start_seconds,
                    range_seconds: (session.work_area.start / fps, session.work_area.end / fps),
                    looping: session.looping,
                };
                // Replacing the old player stops its sound first.
                self.playing = None;
                self.playing = Some(Playing { player: output.play(timeline), session, clips, start_seconds });
            }
        }
        self.playing.as_ref().map(|playing| playing.player.heard())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These open the real sound output, but play only silence (the sample project's media has
    // no files behind it). Without a sound device the clock falls back to the wall clock.

    #[test]
    fn sound_starts_and_stops_with_playback() {
        let mut sound = Sound::new();
        let mut state = EditorState::new();
        assert_eq!(sound.sync(&mut state), None);
        state.toggle_playback(Instant::now());
        assert!(sound.sync(&mut state).is_some());
        assert!(sound.playing.is_some());
        state.toggle_playback(Instant::now());
        assert_eq!(sound.sync(&mut state), None);
        assert!(sound.playing.is_none());
    }

    #[test]
    fn changing_the_work_area_mid_playback_starts_over_from_the_playhead() {
        let mut sound = Sound::new();
        let mut state = EditorState::new();
        state.toggle_playback(Instant::now());
        sound.sync(&mut state);
        state.current_frame = 30;
        let first = state.playback_started();
        state.set_work_area(FrameRange::new(24.0, 120.0));
        sound.sync(&mut state);
        let restarted = state.playback_started().unwrap();
        assert_ne!(Some(restarted), first);
        assert_eq!(restarted.1, 30);
        let playing = sound.playing.as_ref().unwrap();
        assert_eq!(playing.session.work_area, FrameRange::new(24.0, 120.0));
        assert_eq!(playing.start_seconds, 30.0 / 24.0, "the sound restarts from the playhead too, not from the old start");
    }

    #[test]
    fn heard_time_drives_the_playhead_when_sound_is_available() {
        let mut sound = Sound::new();
        let mut state = EditorState::new();
        state.toggle_playback(Instant::now());
        let started = Instant::now();
        // Whether from the sound card or the wall-clock fallback, time must move forward.
        let mut heard = Duration::ZERO;
        while heard < Duration::from_millis(100) && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(5));
            heard = sound.sync(&mut state).unwrap();
        }
        assert!(heard >= Duration::from_millis(100), "the clock never advanced");
        assert!(heard <= started.elapsed(), "heard time cannot run ahead of real time");
    }
}
