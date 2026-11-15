//! Multicam editing: several recordings of the same moment (camera angles), lined up by their
//! sound and then cut between.
//!
//! Lining up works on each recording's loudness over time (its waveform's peaks): the shift at
//! which two recordings' loud and quiet moments agree best is how far apart they started.
//!
//! Cutting keeps things plain. Each angle is the clips of one video file on its own track. A cut
//! to an angle splits every angle's clip at that frame and, from there on, shows only the chosen
//! angle's picture. What is left are ordinary clips, to trim and move like any others.

use crate::model::{Composition, Id, Layer, LayerContent, Project};

/// How alike two recordings must be at their best shift, from 0 (unrelated) to 1 (identical),
/// to be called the same moment. Below this the match is a guess and is not used.
pub const MIN_SYNC_CONFIDENCE: f32 = 0.3;

/// How much of the shorter recording must overlap at a shift for it to be considered, so two
/// recordings are never "matched" on a few buckets at their ends.
const MIN_OVERLAP: f32 = 0.25;

/// How alike `reference` and `other` are when `other` is read `lag` buckets later than
/// `reference` (bucket `i` of one against bucket `i + lag` of the other): their correlation
/// over the part where both have sound, from -1 to 1. `None` when too little overlaps.
fn likeness(reference: &[f32], other: &[f32], lag: i64) -> Option<f32> {
    // The buckets of `reference` whose partner exists in `other`.
    let first = (-lag).max(0) as usize;
    let last = (other.len() as i64 - lag).min(reference.len() as i64);
    if last <= first as i64 {
        return None;
    }
    let ours = &reference[first..last as usize];
    let theirs = &other[(first as i64 + lag) as usize..(last + lag) as usize];
    let shorter = reference.len().min(other.len()) as f32;
    if (ours.len() as f32) < (shorter * MIN_OVERLAP).max(8.0) {
        return None;
    }
    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    let (our_mean, their_mean) = (mean(ours), mean(theirs));
    let (mut together, mut our_spread, mut their_spread) = (0.0f32, 0.0f32, 0.0f32);
    for (a, b) in ours.iter().zip(theirs) {
        let (a, b) = (a - our_mean, b - their_mean);
        together += a * b;
        our_spread += a * a;
        their_spread += b * b;
    }
    let scale = (our_spread * their_spread).sqrt();
    // A flat stretch (silence) has nothing to match on.
    (scale > 1e-9).then(|| together / scale)
}

/// The shift in `lags` at which the two recordings are most alike, and how alike they are there.
fn best_lag(reference: &[f32], other: &[f32], lags: impl Iterator<Item = i64>) -> Option<(i64, f32)> {
    lags.filter_map(|lag| likeness(reference, other, lag).map(|score| (lag, score))).max_by(|a, b| a.1.total_cmp(&b.1))
}

/// How many buckets later than in `reference` the same sound is found in `other` (negative when
/// it is earlier), looking up to `max_lag` buckets either way, and how alike the two are there.
/// Both are loudness over time at the same number of buckets per second. `None` when they are
/// not alike enough anywhere to trust (see `MIN_SYNC_CONFIDENCE`).
///
/// The search is done coarsely first (on every fourth bucket's loudest value) and then finely
/// around the coarse answer, which is far quicker than trying every shift at full detail.
pub fn sync_lag(reference: &[f32], other: &[f32], max_lag: usize) -> Option<(i64, f32)> {
    const COARSE: usize = 4;
    let coarsen = |values: &[f32]| -> Vec<f32> { values.chunks(COARSE).map(|chunk| chunk.iter().copied().fold(0.0, f32::max)).collect() };
    let (coarse_reference, coarse_other) = (coarsen(reference), coarsen(other));
    let coarse_reach = (max_lag / COARSE) as i64;
    let (coarse, _) = best_lag(&coarse_reference, &coarse_other, -coarse_reach..=coarse_reach)?;
    let center = coarse * COARSE as i64;
    let reach = max_lag as i64;
    let nearby = (center - 2 * COARSE as i64).max(-reach)..=(center + 2 * COARSE as i64).min(reach);
    best_lag(reference, other, nearby).filter(|(_, score)| *score >= MIN_SYNC_CONFIDENCE)
}

/// The video file a layer plays, if it is a footage clip.
fn footage_item(layer: &Layer) -> Option<Id> {
    match layer.content {
        LayerContent::Footage { item } => Some(item),
        _ => None,
    }
}

/// The angles `layers` stand for: the video file behind each footage clip among them, once
/// each, in the order the clips are stacked (top first). Other kinds of layer are left out.
pub fn angles_of(composition: &Composition, layers: &[Id]) -> Vec<Id> {
    let mut angles: Vec<Id> = Vec::new();
    for item in composition.layers.iter().filter(|layer| layers.contains(&layer.id)).filter_map(footage_item) {
        if !angles.contains(&item) {
            angles.push(item);
        }
    }
    angles
}

/// Cuts to angle number `angle` (counting from 0 in `angles`, the video files as given by
/// `angles_of`) at `frame`: every angle's clip under the playhead is split there, and from that
/// frame on only the chosen angle's picture shows. Sound is left as it is, so the angle whose
/// sound is used keeps playing. Locked clips are left alone. Returns whether a cut was made.
pub fn cut_to_angle(project: &mut Project, comp: Id, angles: &[Id], angle: usize, frame: i64) -> bool {
    let Some(&chosen) = angles.get(angle) else { return false };
    let Some(composition) = project.composition(comp) else { return false };
    let is_angle = |layer: &Layer| !layer.switches.locked && footage_item(layer).is_some_and(|item| angles.contains(&item));
    if !composition.layers.iter().any(|layer| is_angle(layer) && footage_item(layer) == Some(chosen)) {
        return false; // The chosen angle has no clip here to show.
    }
    let crossing: Vec<Id> = composition.layers.iter().filter(|layer| is_angle(layer) && layer.in_frame < frame && frame < layer.out_frame).map(|layer| layer.id).collect();
    // An empty list would mean "every layer" to `split_layers`.
    if !crossing.is_empty() {
        crate::cut::split_layers(project, comp, &crossing, frame);
    }
    let Some(composition) = project.composition_mut(comp) else { return false };
    for layer in composition.layers.iter_mut().filter(|layer| is_angle(layer) && layer.in_frame >= frame) {
        layer.switches.visible = footage_item(layer) == Some(chosen);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit::{self, FootageInfo};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    /// Loudness over time in bursts that never repeat, like speech: each run of five buckets
    /// is either quiet or at a level of its own, drawn from a small random number generator
    /// (xorshift) so no shift of the sound looks like the sound itself.
    fn bursts(length: usize) -> Vec<f32> {
        let mut state = 0x2545_f491u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let runs: Vec<f32> = (0..length.div_ceil(5)).map(|_| if next() > 0.45 { 0.15 + 0.8 * next() } else { 0.02 }).collect();
        (0..length).map(|index| runs[index / 5]).collect()
    }

    #[test]
    fn the_shift_between_two_recordings_of_one_sound_is_found() {
        let reference = bursts(2000);
        // The second camera started 130 buckets earlier, so the same sound is 130 later in it.
        let mut other = vec![0.02; 130];
        other.extend(&reference[..1700]);
        let (lag, confidence) = sync_lag(&reference, &other, 600).expect("they match");
        assert_eq!(lag, 130);
        assert!(confidence > 0.95);
        // And the other way around.
        let (back, _) = sync_lag(&other, &reference, 600).expect("they match");
        assert_eq!(back, -130);
    }

    #[test]
    fn unrelated_or_silent_sound_is_not_matched() {
        let reference = bursts(1000);
        let silent = vec![0.0; 1000];
        assert_eq!(sync_lag(&reference, &silent, 300), None);
        // A different recording altogether: the same kind of sound, read backwards.
        let unrelated: Vec<f32> = bursts(2000).into_iter().rev().take(1000).collect();
        assert_eq!(sync_lag(&reference, &unrelated, 300), None);
    }

    /// Two ten-second angles stacked from frame 0, as (project, [layer], [item]).
    fn two_angles() -> (Project, [Id; 2], [Id; 2]) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 10.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let wide = edit::import_footage(&mut project, "/media/wide.mp4".into(), &info);
        let close = edit::import_footage(&mut project, "/media/close.mp4".into(), &info);
        let wide_clip = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, wide, 0, 0).unwrap();
        let close_clip = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, close, 0, 0).unwrap();
        (project, [wide_clip, close_clip], [wide, close])
    }

    /// The spans in which an angle's picture shows, in order.
    fn shown(project: &Project, item: Id) -> Vec<(i64, i64)> {
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let mut spans: Vec<(i64, i64)> = composition.layers.iter().filter(|layer| footage_item(layer) == Some(item) && layer.switches.visible).map(|layer| (layer.in_frame, layer.out_frame)).collect();
        spans.sort();
        spans
    }

    #[test]
    fn cutting_to_an_angle_shows_only_it_from_there_on() {
        let (mut project, clips, items) = two_angles();
        let angles = angles_of(project.composition(ids::MAIN_COMP).unwrap(), &clips);
        assert_eq!(angles.len(), 2);
        let (first, second) = (angles[0], angles[1]);
        assert!(items.contains(&first) && items.contains(&second));
        // To the second angle at frame 48, back to the first at 120.
        assert!(cut_to_angle(&mut project, ids::MAIN_COMP, &angles, 1, 48));
        assert!(cut_to_angle(&mut project, ids::MAIN_COMP, &angles, 0, 120));
        assert_eq!(shown(&project, second), vec![(0, 48), (48, 120)], "the second shows until the cut back (both show before the first cut)");
        assert_eq!(shown(&project, first), vec![(0, 48), (120, 240)]);
        // Cutting again at the same frame changes that cut, without splitting anything more.
        let layers_before = project.composition(ids::MAIN_COMP).unwrap().layers.len();
        assert!(cut_to_angle(&mut project, ids::MAIN_COMP, &angles, 1, 120));
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layers.len(), layers_before);
        assert_eq!(shown(&project, second), vec![(0, 48), (48, 120), (120, 240)]);
        // An angle number that doesn't exist does nothing.
        assert!(!cut_to_angle(&mut project, ids::MAIN_COMP, &angles, 5, 60));
    }
}
