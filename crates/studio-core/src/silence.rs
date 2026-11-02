//! Removing silences from talking footage: finding the quiet stretches between phrases from a
//! clip's sound levels, and cutting them out so the rest closes up (jump cuts).
//!
//! "Quiet" is measured against the clip itself: well below its usual loud level, so it works
//! whatever the recording's volume. A little sound is kept either side of each cut, so words
//! aren't clipped.

use crate::model::{Id, Project};

/// How silences are found.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilenceSettings {
    /// How far below the clip's loud level counts as quiet, in decibels.
    pub below_db: f32,
    /// The shortest pause worth cutting, in seconds.
    pub min_seconds: f64,
    /// Sound kept either side of a cut, in seconds.
    pub padding_seconds: f64,
}

impl Default for SilenceSettings {
    fn default() -> Self {
        Self { below_db: 26.0, min_seconds: 0.45, padding_seconds: 0.12 }
    }
}

/// The quiet stretches in `levels` (peak level per bucket, `per_second` buckets a second), as
/// (start, end) seconds, already shrunk by the padding.
pub fn quiet_spans(levels: &[f32], per_second: u32, settings: SilenceSettings) -> Vec<(f64, f64)> {
    if levels.is_empty() || per_second == 0 {
        return Vec::new();
    }
    // The clip's loud level: the 90th percentile, so a few spikes don't set it.
    let mut sorted: Vec<f32> = levels.iter().copied().filter(|level| level.is_finite()).collect();
    sorted.sort_by(f32::total_cmp);
    let loud = sorted.get(sorted.len() * 9 / 10).copied().unwrap_or(0.0);
    if loud <= 1e-4 {
        return Vec::new(); // Silent throughout: nothing to tell apart.
    }
    let threshold = loud * 10f32.powf(-settings.below_db / 20.0);
    let seconds = |bucket: usize| bucket as f64 / f64::from(per_second);
    let mut spans = Vec::new();
    let mut start = None;
    for (index, &level) in levels.iter().chain(std::iter::once(&f32::MAX)).enumerate() {
        match (level < threshold, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                start = None;
                let (from, to) = (seconds(from), seconds(index));
                if to - from >= settings.min_seconds {
                    // Keep a little sound either side; the clip's very start and end need no padding.
                    let from = if from <= 0.0 { 0.0 } else { from + settings.padding_seconds };
                    let to = if index >= levels.len() { to } else { to - settings.padding_seconds };
                    if to > from {
                        spans.push((from, to));
                    }
                }
            }
            _ => {}
        }
    }
    spans
}

/// Cuts `spans` (seconds into the layer's source) out of a layer and closes the gaps on its
/// track, as ripple deletes do. Spans outside the part of the source the layer shows are
/// ignored. Returns how many were cut.
pub fn remove_spans(project: &mut Project, comp: Id, layer: Id, spans: &[(f64, f64)]) -> usize {
    let Some(composition) = project.composition(comp) else { return 0 };
    let Some(found) = composition.layer(layer) else { return 0 };
    if found.switches.locked {
        return 0;
    }
    let fps = composition.frame_rate.fps();
    let (in_frame, out_frame, start) = (found.in_frame, found.out_frame, found.start_frame);
    // Source seconds to composition frames, kept within the layer.
    let frames: Vec<(i64, i64)> = spans
        .iter()
        .map(|&(from, to)| (start + (from * fps).round() as i64, start + (to * fps).round() as i64))
        .map(|(from, to)| (from.max(in_frame), to.min(out_frame)))
        .filter(|(from, to)| to > from)
        .collect();
    let mut cut = 0;
    // From the last pause back, so earlier ones keep their place as later ones close up.
    for &(from, to) in frames.iter().rev() {
        let current = project.composition(comp).and_then(|composition| composition.layer(layer)).map(|layer| (layer.in_frame, layer.out_frame));
        let Some((now_in, now_out)) = current else { break };
        if from <= now_in && to >= now_out {
            continue; // Never cut the whole clip away.
        }
        // Split off what follows the pause, then the pause itself, and ripple delete the pause.
        if to < now_out {
            crate::cut::split_layers(project, comp, &[layer], to);
        }
        // The pause, with its share of any track matte the clip uses, so the matte stays in step.
        let pause: Vec<Id> = if from > now_in {
            crate::cut::split_layers(project, comp, &[layer], from).created
        } else {
            project.composition(comp).map(|composition| crate::cut::matte_group_ids(composition, layer)).unwrap_or_default()
        };
        if !pause.is_empty() && crate::cut::ripple_delete(project, comp, &pause) > 0 {
            cut += 1;
        }
        if from <= now_in {
            break; // The pause was the clip's start; the original layer is gone.
        }
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit::{self, FootageInfo};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    #[test]
    fn pauses_are_found_relative_to_the_clips_own_loudness() {
        // 10 buckets a second: talk 1 s, pause 1 s, talk 1 s, a short breath, talk 1 s.
        let mut levels = vec![0.5; 10];
        levels.extend(vec![0.01; 10]);
        levels.extend(vec![0.4; 10]);
        levels.extend(vec![0.01; 2]);
        levels.extend(vec![0.6; 10]);
        let spans = quiet_spans(&levels, 10, SilenceSettings::default());
        assert_eq!(spans.len(), 1, "the breath is too short to cut: {spans:?}");
        let (from, to) = spans[0];
        assert!((from - 1.12).abs() < 1e-6 && (to - 1.88).abs() < 1e-6, "padded inside the pause: {spans:?}");
        // The same recording ten times quieter finds the same pause.
        let quiet: Vec<f32> = levels.iter().map(|level| level * 0.1).collect();
        assert_eq!(quiet_spans(&quiet, 10, SilenceSettings::default()), spans);
    }

    #[test]
    fn cutting_pauses_closes_up_the_clip_and_what_follows() {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/talk.mp4".into(), &info);
        let clip = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
        // Two one-second pauses, at 1–2 s and 3–4 s of the source (frames 24–48 and 72–96).
        assert_eq!(remove_spans(&mut project, ids::MAIN_COMP, clip, &[(1.0, 2.0), (3.0, 4.0)]), 2);
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let track = composition.layer(clip).unwrap().track;
        let mut pieces: Vec<(i64, i64, i64)> = composition.layers.iter().filter(|layer| layer.track == track).map(|layer| (layer.in_frame, layer.out_frame, layer.start_frame)).collect();
        pieces.sort();
        // What was left plays back to back: 0–24 (source 0–1 s), then 24–48 (source 2–3 s).
        assert_eq!(pieces, vec![(0, 24, 0), (24, 48, -24)]);
    }

    #[test]
    fn a_clip_with_a_track_matte_loses_its_pause_and_keeps_its_matte_in_step() {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 10.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/talk.mp4".into(), &info);
        let clip = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.duration_frames = 480;
        // A matte the length of the clip, just above it.
        let index = comp.layers.iter().position(|layer| layer.id == clip).unwrap();
        let mut matte = comp.layers[index].clone();
        matte.id = Id(9_000);
        matte.name = "Matte".into();
        comp.layers.insert(index, matte);
        comp.layers[index + 1].track_matte = crate::model::TrackMatte::Alpha;
        assert_eq!(remove_spans(&mut project, ids::MAIN_COMP, clip, &[(2.0, 4.0)]), 1);
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        let pieces = |name: &str| {
            let mut spans: Vec<(i64, i64)> = comp.layers.iter().filter(|layer| layer.name == name).map(|layer| (layer.in_frame, layer.out_frame)).collect();
            spans.sort();
            spans
        };
        let clip_name = comp.layer(clip).unwrap().name.clone();
        let (footage, mattes) = (pieces(&clip_name), pieces("Matte"));
        assert_eq!(footage, vec![(0, 48), (48, 192)], "the pause (frames 48–96) is gone");
        assert_eq!(mattes, footage, "each piece keeps its matte");
    }
}
