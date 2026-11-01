//! Premiere Pro's trimming tools, for editing clips that sit end to end on a track:
//!
//! - **Ripple** trims one end of a clip and moves every later clip on its track by the same
//!   amount, so no gap opens (or the later clips make room).
//! - **Roll** moves the cut between two touching clips: one gets longer as the other gets
//!   shorter, and nothing else moves.
//! - **Slide** moves a clip between its touching neighbors: the clip before it gets longer or
//!   shorter to meet it, and so does the clip after it.
//! - **Move group** moves several clips together (Track Select Forward's selection), stopping
//!   before any of them would run into a clip left behind.
//!
//! Each function applies a whole drag at once to the timing as it was when the drag began, so
//! callers keep that starting state and apply the edit to it again as the pointer moves.
//! Locked clips never change; an edit that would change one does nothing.

use crate::model::{Composition, Id, Layer, Project};
use crate::trim::source_frames;

/// Which end of a clip is being trimmed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    In,
    Out,
}

/// The first and last composition frames a clip's source can reach (start, end exclusive);
/// unbounded for stills, text and other sources without a length.
fn source_reach(project: &Project, composition: &Composition, layer: &Layer) -> (i64, i64) {
    match source_frames(project, composition, layer) {
        Some(length) => (layer.start_frame, layer.start_frame + length),
        None => (i64::MIN, i64::MAX),
    }
}

/// The clips on `layer`'s track that touch its in point (ending there) and out point
/// (starting there).
fn touching(composition: &Composition, layer: &Layer) -> (Option<Id>, Option<Id>) {
    let same_track = || composition.layers.iter().filter(|other| other.id != layer.id && other.track == layer.track);
    let before = same_track().find(|other| other.out_frame == layer.in_frame).map(|other| other.id);
    let after = same_track().find(|other| other.in_frame == layer.out_frame).map(|other| other.id);
    (before, after)
}

/// Ripple-trims `layer`'s `end` to `frame`. Later clips on its track (those starting at or after
/// its out point) move by however much the clip's length changed. Trimming the in point keeps
/// the clip starting where it was: its first frames are cut (or uncovered) and the rest of the
/// clip moves to meet the start. Returns whether anything changed.
pub fn ripple_trim(project: &mut Project, comp: Id, layer_id: Id, end: End, frame: i64) -> bool {
    let Some(composition) = project.composition(comp) else { return false };
    let Some(layer) = composition.layer(layer_id) else { return false };
    let (first, last) = source_reach(project, composition, layer);
    let later: Vec<Id> = composition.layers.iter().filter(|other| other.id != layer_id && other.track == layer.track && other.in_frame >= layer.out_frame).map(|other| other.id).collect();
    if layer.switches.locked || later.iter().any(|&id| composition.layer(id).is_some_and(|other| other.switches.locked)) {
        return false;
    }
    let (in_frame, out_frame) = (layer.in_frame, layer.out_frame);
    // How much shorter (positive) or longer the clip gets, and so how far later clips move back.
    let shortened = match end {
        End::Out => out_frame - frame.clamp(in_frame + 1, last),
        End::In => frame.clamp(first, out_frame - 1) - in_frame,
    };
    if shortened == 0 {
        return false;
    }
    let Some(composition) = project.composition_mut(comp) else { return false };
    let Some(layer) = composition.layer_mut(layer_id) else { return false };
    match end {
        End::Out => layer.out_frame -= shortened,
        End::In => {
            layer.in_frame += shortened;
            // Slide what is left back to where the clip started.
            layer.shift_in_time(-shortened);
        }
    }
    for id in later {
        if let Some(other) = composition.layer_mut(id) {
            other.shift_in_time(-shortened);
        }
    }
    true
}

/// Rolls the cut at `layer`'s `end` to `frame`: the clip touching it there on the same track
/// changes with it, so the cut moves and nothing else does. Without a touching clip, this is an
/// ordinary trim. Returns whether anything changed.
pub fn roll(project: &mut Project, comp: Id, layer_id: Id, end: End, frame: i64) -> bool {
    let Some(composition) = project.composition(comp) else { return false };
    let Some(layer) = composition.layer(layer_id) else { return false };
    let (before, after) = touching(composition, layer);
    let partner = match end {
        End::In => before,
        End::Out => after,
    };
    let Some(partner) = partner else {
        return match end {
            End::In => crate::trim::trim_in(project, comp, layer_id, frame),
            End::Out => crate::trim::trim_out(project, comp, layer_id, frame),
        };
    };
    // The clip on the left of the cut, and the one on the right.
    let (left_id, right_id) = if end == End::Out { (layer_id, partner) } else { (partner, layer_id) };
    let (Some(left), Some(right)) = (composition.layer(left_id), composition.layer(right_id)) else { return false };
    if left.switches.locked || right.switches.locked {
        return false;
    }
    // The cut must leave each clip a frame, and stay within both sources.
    let (_, left_last) = source_reach(project, composition, left);
    let (right_first, _) = source_reach(project, composition, right);
    let low = (left.in_frame + 1).max(right_first);
    let high = (right.out_frame - 1).min(left_last);
    if low > high {
        return false;
    }
    let cut = frame.clamp(low, high);
    if cut == left.out_frame {
        return false;
    }
    let Some(composition) = project.composition_mut(comp) else { return false };
    if let Some(left) = composition.layer_mut(left_id) {
        left.out_frame = cut;
    }
    if let Some(right) = composition.layer_mut(right_id) {
        right.in_frame = cut;
    }
    true
}

/// Slides `layer` `delta` frames along its track. A clip touching its in point gets longer or
/// shorter to keep touching it, and so does a clip touching its out point; where nothing
/// touches, it moves only as far as the gap allows. Returns how far it moved.
pub fn slide(project: &mut Project, comp: Id, layer_id: Id, delta: i64) -> i64 {
    let Some(composition) = project.composition(comp) else { return 0 };
    let Some(layer) = composition.layer(layer_id) else { return 0 };
    let (before, after) = touching(composition, layer);
    let (before, after) = (before.and_then(|id| composition.layer(id)), after.and_then(|id| composition.layer(id)));
    if layer.switches.locked || before.is_some_and(|clip| clip.switches.locked) || after.is_some_and(|clip| clip.switches.locked) {
        return 0;
    }
    let same_track = || composition.layers.iter().filter(|other| other.id != layer.id && other.track == layer.track);
    let (mut low, mut high) = (-layer.in_frame, i64::MAX);
    match before {
        // The clip before keeps at least a frame and stays within its source.
        Some(clip) => {
            let (_, last) = source_reach(project, composition, clip);
            low = low.max(clip.in_frame + 1 - clip.out_frame);
            high = high.min(last.saturating_sub(clip.out_frame));
        }
        None => {
            let gap_start = same_track().filter(|other| other.out_frame <= layer.in_frame).map(|other| other.out_frame).max().unwrap_or(0);
            low = low.max(gap_start - layer.in_frame);
        }
    }
    match after {
        Some(clip) => {
            let (first, _) = source_reach(project, composition, clip);
            low = low.max(first.saturating_sub(clip.in_frame));
            high = high.min(clip.out_frame - 1 - clip.in_frame);
        }
        None => {
            if let Some(gap_end) = same_track().filter(|other| other.in_frame >= layer.out_frame).map(|other| other.in_frame).min() {
                high = high.min(gap_end - layer.out_frame);
            }
        }
    }
    if low > high {
        return 0;
    }
    let delta = delta.clamp(low, high);
    if delta == 0 {
        return 0;
    }
    let (before, after) = (before.map(|clip| clip.id), after.map(|clip| clip.id));
    let Some(composition) = project.composition_mut(comp) else { return 0 };
    if let Some(clip) = before.and_then(|id| composition.layer_mut(id)) {
        clip.out_frame += delta;
    }
    if let Some(clip) = after.and_then(|id| composition.layer_mut(id)) {
        clip.in_frame += delta;
    }
    if let Some(layer) = composition.layer_mut(layer_id) {
        layer.shift_in_time(delta);
    }
    delta
}

/// The clips Track Select Forward picks from `layer`: it and every clip starting at or after
/// its in point, on every track (or only its own track, with `one_track`).
pub fn select_forward(composition: &Composition, layer_id: Id, one_track: bool) -> Vec<Id> {
    let Some(layer) = composition.layer(layer_id) else { return Vec::new() };
    composition
        .layers
        .iter()
        .filter(|other| other.id == layer_id || (other.in_frame >= layer.in_frame && (!one_track || other.track == layer.track)))
        .map(|other| other.id)
        .collect()
}

/// Moves `ids` together by `delta` frames, keeping their tracks. Moving earlier stops at frame 0
/// and before any of them would run into a clip that is not moving on its track. Locked clips
/// stay put. Returns how far they moved.
pub fn move_group(composition: &mut Composition, ids: &[Id], delta: i64) -> i64 {
    let moving: Vec<&Layer> = composition.layers.iter().filter(|layer| ids.contains(&layer.id) && !layer.switches.locked).collect();
    let mut low = moving.iter().map(|layer| -layer.in_frame).max().unwrap_or(0);
    let mut high = i64::MAX;
    for layer in &moving {
        let staying = composition.layers.iter().filter(|other| other.track == layer.track && !moving.iter().any(|moved| moved.id == other.id));
        for other in staying {
            if other.out_frame <= layer.in_frame {
                low = low.max(other.out_frame - layer.in_frame);
            } else if other.in_frame >= layer.out_frame {
                high = high.min(other.in_frame - layer.out_frame);
            }
        }
    }
    if low > high {
        return 0;
    }
    let delta = delta.clamp(low, high);
    let moving: Vec<Id> = moving.iter().map(|layer| layer.id).collect();
    for id in moving {
        if let Some(layer) = composition.layer_mut(id) {
            layer.shift_in_time(delta);
        }
    }
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit::{self, FootageInfo};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    /// Three clips end to end on one track: A at 0–48, B at 48–96, C at 96–144. Each shows the
    /// middle 48 frames of a 4-second (96-frame) source, so it has 24 frames to spare either way.
    fn three_clips() -> (Project, [Id; 3]) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: false };
        let clips = [0, 1, 2].map(|index| {
            let item = edit::import_footage(&mut project, format!("/media/{index}.mp4").into(), &info);
            let id = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
            let layer = project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(id).unwrap();
            // Source 96 frames from start; shown 48 frames from start + 24.
            layer.start_frame = index * 48 - 24;
            layer.in_frame = index * 48;
            layer.out_frame = index * 48 + 48;
            id
        });
        // All on one track (set last: adding a layer renumbers the tracks).
        for id in clips {
            project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(id).unwrap().track = 40;
        }
        (project, clips)
    }

    fn timing(project: &Project, id: Id) -> (i64, i64) {
        let layer = project.composition(ids::MAIN_COMP).unwrap().layer(id).unwrap();
        (layer.in_frame, layer.out_frame)
    }

    #[test]
    fn ripple_trims_move_later_clips_with_the_cut() {
        let (mut project, [a, b, c]) = three_clips();
        // Shorten A's end by 10: B and C move 10 earlier, touching still.
        assert!(ripple_trim(&mut project, ids::MAIN_COMP, a, End::Out, 38));
        assert_eq!([timing(&project, a), timing(&project, b), timing(&project, c)], [(0, 38), (38, 86), (86, 134)]);
        // Lengthen B's start by 12 frames of source: B still starts at 38, and C moves 12 later.
        let (mut project, [_, b, c]) = three_clips();
        assert!(ripple_trim(&mut project, ids::MAIN_COMP, b, End::In, 36));
        assert_eq!([timing(&project, b), timing(&project, c)], [(48, 108), (108, 156)]);
        let source_start = project.composition(ids::MAIN_COMP).unwrap().layer(b).unwrap().start_frame;
        assert_eq!(source_start, 36, "B now shows 12 more frames of its source at its start");
        // No further than the source allows.
        let (mut project, [a, ..]) = three_clips();
        ripple_trim(&mut project, ids::MAIN_COMP, a, End::Out, 500);
        assert_eq!(timing(&project, a), (0, 72), "A's source ends at frame 72");
    }

    #[test]
    fn a_roll_moves_the_cut_between_two_clips_only() {
        let (mut project, [a, b, c]) = three_clips();
        assert!(roll(&mut project, ids::MAIN_COMP, a, End::Out, 60));
        assert_eq!([timing(&project, a), timing(&project, b), timing(&project, c)], [(0, 60), (60, 96), (96, 144)]);
        // Rolling past what B's source has stops there (B's source starts at 24).
        roll(&mut project, ids::MAIN_COMP, b, End::In, 0);
        assert_eq!(timing(&project, b).0, 24);
    }

    #[test]
    fn sliding_a_clip_reshapes_its_neighbors() {
        let (mut project, [a, b, c]) = three_clips();
        assert_eq!(slide(&mut project, ids::MAIN_COMP, b, 10), 10);
        assert_eq!([timing(&project, a), timing(&project, b), timing(&project, c)], [(0, 58), (58, 106), (106, 144)]);
        // A can only grow as far as its source (to 72), so B slides no further.
        let (mut project, [_, b, _]) = three_clips();
        assert_eq!(slide(&mut project, ids::MAIN_COMP, b, 100), 24);
    }

    #[test]
    fn track_select_forward_picks_everything_from_the_clip_on() {
        let (mut project, [a, b, c]) = three_clips();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let picked = select_forward(composition, b, true);
        assert!(picked.contains(&b) && picked.contains(&c) && !picked.contains(&a));
        // Moving them earlier stops at A.
        let composition = project.composition_mut(ids::MAIN_COMP).unwrap();
        assert_eq!(move_group(composition, &picked, -30), 0, "B already touches A");
        assert_eq!(move_group(composition, &picked, 20), 20);
        assert_eq!(move_group(composition, &picked, -50), -20, "back to touching A");
    }

    #[test]
    fn locked_clips_hold_their_ground() {
        let (mut project, [a, b, _]) = three_clips();
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(b).unwrap().switches.locked = true;
        assert!(!ripple_trim(&mut project, ids::MAIN_COMP, a, End::Out, 30), "B would have to move");
        assert!(!roll(&mut project, ids::MAIN_COMP, a, End::Out, 30));
        assert_eq!(slide(&mut project, ids::MAIN_COMP, b, 5), 0);
    }
}
