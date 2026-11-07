//! Rendering ahead, like After Effects' cached preview: while the editor shows a composition,
//! a background thread renders the frames after the playhead (through the work area, then from
//! its start) at the viewer's size and keeps them in memory, so playback shows them instead of
//! rendering each frame as it plays. A green bar on the timeline's ruler marks them.
//!
//! The thread has its own renderer and its own exact decoder (as export does), so it never
//! competes with the viewer for either. Any change to the project, the composition or the
//! viewer's size starts afresh; when the memory set aside is full, the frames furthest behind
//! the playhead make room for the ones ahead.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use studio_core::model::{Id, Project};
use studio_core::time::FrameRange;

/// One frame rendered ahead, ready to show (made into an image on the rendering thread, so
/// showing it costs the editor nothing).
pub struct CachedFrame {
    pub image: Arc<gpui_kit::RenderImage>,
}

/// What to render ahead.
#[derive(Clone)]
struct Job {
    project: Arc<Project>,
    comp: Id,
    width: u32,
    height: u32,
    /// The frames wanted, in order: from the playhead to the end of the range, then from its start.
    order: Vec<i64>,
    /// Each frame's place in `order`, to find the one least needed.
    rank: HashMap<i64, usize>,
    /// How many frames fit in the memory set aside.
    budget: usize,
}

#[derive(Default)]
struct State {
    /// Bumped whenever the job changes, so a frame rendered for an older one is thrown away.
    generation: u64,
    job: Option<Job>,
    frames: BTreeMap<i64, Arc<CachedFrame>>,
    stop: bool,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    /// Called (from the rendering thread) after frames are added, so the ruler can redraw.
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

/// The cache and its rendering thread.
pub struct PreviewCache {
    shared: Arc<Shared>,
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewCache {
    /// Starts the rendering thread (idle until there is something to render).
    pub fn new() -> Self {
        let shared = Arc::new(Shared { state: Mutex::default(), work: Condvar::new(), on_change: Mutex::new(None) });
        let thread_shared = shared.clone();
        if let Err(error) = std::thread::Builder::new().name("render-ahead".into()).spawn(move || work(&thread_shared)) {
            eprintln!("rendering ahead is off: {error}");
        }
        Self { shared }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Says what the viewer shows now. Unchanged, the thread carries on; otherwise it starts
    /// again for the new composition, size or project (a moved playhead only changes which
    /// frames come first). `budget_bytes` of zero turns rendering ahead off.
    #[allow(clippy::too_many_arguments)]
    pub fn show(&self, project: &Project, comp: Id, width: u32, height: u32, playhead: i64, range: FrameRange, budget_bytes: u64) {
        let frame_bytes = u64::from(width.max(1)) * u64::from(height.max(1)) * 4;
        let budget = (budget_bytes / frame_bytes) as usize;
        let mut state = self.lock();
        if budget == 0 {
            if state.job.take().is_some() {
                state.generation += 1;
                state.frames.clear();
            }
            return;
        }
        let same_picture = state.job.as_ref().is_some_and(|job| job.comp == comp && job.width == width && job.height == height && *job.project == *project);
        let (start, end) = (range.start.floor() as i64, (range.end.ceil() as i64).max(range.start.floor() as i64 + 1));
        let playhead = playhead.clamp(start, end - 1);
        let first = state.job.as_ref().and_then(|job| job.order.first().copied());
        if same_picture && first == Some(playhead) && state.job.as_ref().is_some_and(|job| job.order.len() as i64 == end - start && job.budget == budget) {
            return;
        }
        let order: Vec<i64> = (playhead..end).chain(start..playhead).collect();
        let rank = order.iter().enumerate().map(|(place, &frame)| (frame, place)).collect();
        let project = match state.job.as_ref().filter(|_| same_picture) {
            Some(job) => job.project.clone(),
            None => Arc::new(project.clone()),
        };
        if !same_picture {
            state.generation += 1;
            state.frames.clear();
        }
        state.job = Some(Job { project, comp, width, height, order, rank, budget });
        drop(state);
        self.shared.work.notify_one();
    }

    /// Runs `callback` (on the rendering thread) whenever frames are added. (Headless tests
    /// don't use it: their scheduler refuses wake-ups from other threads.)
    #[cfg_attr(test, allow(dead_code))]
    pub fn set_on_change(&self, callback: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_change.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(callback));
    }

    /// While playing: the playhead moved on, so frames ahead of it come first. (Cheaper than
    /// `show`, which compares the whole project.)
    pub fn follow(&self, playhead: i64) {
        let mut state = self.lock();
        let Some(job) = state.job.as_mut() else { return };
        let Some(place) = job.order.iter().position(|&frame| frame == playhead).filter(|&place| place > 0) else { return };
        job.order.rotate_left(place);
        job.rank = job.order.iter().enumerate().map(|(place, &frame)| (frame, place)).collect();
        drop(state);
        self.shared.work.notify_one();
    }

    /// The frame rendered ahead for `frame` of `comp`, if it was rendered from exactly `project`.
    pub fn frame_for(&self, project: &Project, comp: Id, frame: i64) -> Option<Arc<CachedFrame>> {
        let state = self.lock();
        state.job.as_ref().filter(|job| job.comp == comp && *job.project == *project)?;
        state.frames.get(&frame).cloned()
    }

    /// The frame rendered ahead for `frame`, whatever it was rendered from.
    #[cfg(test)]
    pub fn frame(&self, frame: i64) -> Option<Arc<CachedFrame>> {
        self.lock().frames.get(&frame).cloned()
    }

    /// The runs of frames rendered ahead (first frame, frame after the last), for the ruler.
    pub fn ranges(&self) -> Vec<(i64, i64)> {
        let state = self.lock();
        let mut ranges: Vec<(i64, i64)> = Vec::new();
        for &frame in state.frames.keys() {
            match ranges.last_mut() {
                Some(last) if last.1 == frame => last.1 = frame + 1,
                _ => ranges.push((frame, frame + 1)),
            }
        }
        ranges
    }

}

impl Drop for PreviewCache {
    fn drop(&mut self) {
        self.lock().stop = true;
        self.shared.work.notify_all();
    }
}

/// The next frame to render, and the frame to drop for it if memory is full; `None` when every
/// frame that fits is done.
fn next(state: &State) -> Option<(i64, Option<i64>)> {
    let job = state.job.as_ref()?;
    let wanted = job.order.iter().copied().find(|frame| !state.frames.contains_key(frame))?;
    if state.frames.len() < job.budget {
        return Some((wanted, None));
    }
    // Full: make room only if a frame kept is needed later than this one.
    let worst = state.frames.keys().copied().max_by_key(|frame| job.rank.get(frame).copied().unwrap_or(usize::MAX))?;
    (job.rank.get(&worst).copied().unwrap_or(usize::MAX) > job.rank[&wanted]).then_some((wanted, Some(worst)))
}

/// The rendering thread: renders the next frame wanted, again and again, sleeping when there
/// is none.
fn work(shared: &Shared) {
    let mut renderer: Option<studio_render::Renderer> = None;
    let mut video: Option<(u32, studio_media::ExactFrames)> = None;
    loop {
        let (job, generation, frame) = {
            let mut state = shared.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            loop {
                if state.stop {
                    return;
                }
                if let Some((frame, _)) = next(&state) {
                    break (state.job.clone().expect("next found a job"), state.generation, frame);
                }
                state = shared.work.wait(state).unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        };
        if renderer.is_none() {
            match studio_render::Renderer::new() {
                Ok(made) => renderer = Some(made),
                Err(error) => {
                    eprintln!("rendering ahead is off: {error}");
                    return;
                }
            }
        }
        // Footage is decoded no wider than the frames are drawn.
        if video.as_ref().is_none_or(|(width, _)| *width != job.width) {
            video = Some((job.width, studio_media::ExactFrames::with_max_width(job.width.max(job.height))));
        }
        let (Some(renderer), Some((_, source))) = (renderer.as_mut(), video.as_ref()) else { return };
        let Ok(rendered) = renderer.render(&job.project, job.comp, frame, job.width, job.height, source) else {
            // A frame that can't be drawn isn't tried again for this job.
            let mut state = shared.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.generation == generation
                && let Some(job) = state.job.as_mut()
            {
                job.order.retain(|wanted| *wanted != frame);
            }
            continue;
        };
        let mut state = shared.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.generation != generation {
            continue;
        }
        // The playhead may have moved meanwhile: check it is still worth keeping.
        if let Some((wanted, evict)) = next(&state)
            && wanted == frame
        {
            if let Some(dropped) = evict {
                state.frames.remove(&dropped);
            }
            let (width, height) = (rendered.width, rendered.height);
            let Some(buffer) = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, rendered.bgra) else { continue };
            let image = Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(buffer)]));
            state.frames.insert(frame, Arc::new(CachedFrame { image }));
            drop(state);
            if let Some(callback) = shared.on_change.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                callback();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(cache: &PreviewCache, count: usize) {
        let started = std::time::Instant::now();
        while cache.lock().frames.len() < count {
            assert!(started.elapsed().as_secs() < 30, "the frames never came");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn frames_ahead_of_the_playhead_render_first_and_edits_start_again() {
        let project = studio_core::sample::sample_project();
        let comp = studio_core::sample::ids::MAIN_COMP;
        let cache = PreviewCache::new();
        // Room for exactly six frames of 64 × 36.
        cache.show(&project, comp, 64, 36, 10, FrameRange::new(0.0, 24.0), 64 * 36 * 4 * 6);
        wait_for(&cache, 6);
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(cache.ranges(), vec![(10, 16)], "the six after the playhead, and no more");
        let frame = cache.frame(12).unwrap();
        let size = frame.image.size(0);
        assert_eq!((size.width.0, size.height.0), (64, 36));

        // The playhead moves on: frames behind it make room for the ones ahead.
        cache.show(&project, comp, 64, 36, 13, FrameRange::new(0.0, 24.0), 64 * 36 * 4 * 6);
        let started = std::time::Instant::now();
        while cache.ranges() != vec![(13, 19)] {
            assert!(started.elapsed().as_secs() < 30, "{:?}", cache.ranges());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // An edit: everything rendered before is out of date.
        let mut edited = project.clone();
        edited.composition_mut(comp).unwrap().layers[0].name = "Changed".into();
        let before = cache.lock().generation;
        cache.show(&edited, comp, 64, 36, 13, FrameRange::new(0.0, 24.0), 64 * 36 * 4 * 6);
        assert!(cache.lock().generation > before, "frames for the old project are thrown away");
        wait_for(&cache, 6);
        assert_eq!(*cache.lock().job.as_ref().unwrap().project, edited);

        // No memory set aside: off, and nothing kept.
        cache.show(&edited, comp, 64, 36, 13, FrameRange::new(0.0, 24.0), 0);
        assert!(cache.ranges().is_empty());
    }
}
