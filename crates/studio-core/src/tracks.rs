//! Tracks, as in Premiere Pro and CapCut: rows that each hold any number of clips side by side.
//!
//! Every layer belongs to a track. Track 0 is the main track; picture tracks stack upward from
//! it (1, 2, ...), drawn on top of the ones below, and sound-only tracks sit below it (-1, -2,
//! ...). Clips on one track never overlap in time, so a track reads as a single line of clips.
//!
//! The composition's layer list stays the render order (top first). It is kept sorted by track,
//! highest first, so tracks decide what covers what. Track mattes still pair a layer with the
//! one directly above it in that list; sorting keeps each matte directly above its layer.
//!
//! With the magnetic main track on, clips on track 0 are kept packed together from frame 0, so
//! removing, trimming or moving one closes the gap, as in CapCut.

use crate::model::{Composition, Id, Layer, LayerContent, TrackMatte};

/// The main track.
pub const MAIN_TRACK: i32 = 0;

/// Marks a track missing from an older project file (see `Layer::track`).
pub(crate) const UNSET_TRACK: i32 = i32::MIN;

/// Whether a layer is sound only, and so lives on the sound tracks below the main track.
pub fn is_sound_only(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Audio { .. })
}

/// Whether `layer`'s time span overlaps any other clip on `track`.
fn overlaps(composition: &Composition, layer: &Layer, track: i32, in_frame: i64, out_frame: i64) -> bool {
    composition
        .layers
        .iter()
        .any(|other| other.id != layer.id && other.track == track && other.in_frame < out_frame && in_frame < other.out_frame)
}

/// Gives every layer of an older file (or one built without tracks) its own track, in its
/// current stacking order: the bottom picture layer becomes the main track, the ones above it
/// tracks 1, 2, ..., and sound-only layers the tracks below. Nothing is drawn differently.
pub fn assign_tracks_by_stack(composition: &mut Composition) {
    let mut next_picture = MAIN_TRACK;
    let mut next_sound = -1;
    for layer in composition.layers.iter_mut().rev() {
        if is_sound_only(layer) {
            layer.track = next_sound;
            next_sound -= 1;
        } else {
            layer.track = next_picture;
            next_picture += 1;
        }
    }
}

/// Puts layers back in track order (highest track first, keeping each track's clips in their
/// existing order), removes empty tracks by closing up the numbering, and keeps every track
/// matte directly above the layer that uses it.
pub fn normalize(composition: &mut Composition) {
    // Which layer each matte belongs to, before anything moves.
    let mattes: Vec<(Id, Id)> = composition
        .layers
        .windows(2)
        .filter(|pair| pair[1].track_matte != TrackMatte::None)
        .map(|pair| (pair[0].id, pair[1].id))
        .collect();

    // Close up gaps in the numbering, above and below the main track separately.
    let mut used: Vec<i32> = composition.layers.iter().map(|layer| layer.track).collect();
    used.sort_unstable();
    used.dedup();
    let renumber = |track: i32| -> i32 {
        if track >= MAIN_TRACK {
            used.iter().filter(|&&t| (MAIN_TRACK..track).contains(&t)).count() as i32
        } else {
            -(used.iter().filter(|&&t| t < MAIN_TRACK && t >= track).count() as i32)
        }
    };
    // The main track stays track 0 even when it is empty, so other tracks don't fall onto it.
    let has_main = used.contains(&MAIN_TRACK);
    for layer in &mut composition.layers {
        let track = renumber(layer.track);
        layer.track = if layer.track > MAIN_TRACK && !has_main { track + 1 } else { track };
    }

    composition.layers.sort_by_key(|layer| std::cmp::Reverse(layer.track));

    // Re-seat each matte directly above its layer. (A matte is not drawn itself, so where it
    // sits in the stack matters only for this pairing.)
    for (matte, user) in mattes {
        let Some(from) = composition.layers.iter().position(|layer| layer.id == matte) else { continue };
        let moved = composition.layers.remove(from);
        let at = composition.layers.iter().position(|layer| layer.id == user).unwrap_or(from.min(composition.layers.len()));
        composition.layers.insert(at, moved);
    }
}

/// Every track in use, top row first (highest track number first), with the main track always
/// included so there is somewhere to put the first clip.
pub fn tracks(composition: &Composition) -> Vec<i32> {
    let mut tracks: Vec<i32> = composition.layers.iter().map(|layer| layer.track).chain([MAIN_TRACK]).collect();
    tracks.sort_unstable_by(|a, b| b.cmp(a));
    tracks.dedup();
    tracks
}

/// The track a new layer gets when added at `stack_index` in the layer list (0 = top): a new
/// track of its own at that height, made by moving the tracks above it up (or, for sound,
/// the ones below it down). The layer list is renormalized afterwards by the caller.
pub fn make_room_at(composition: &mut Composition, stack_index: usize, sound: bool) -> i32 {
    let index = stack_index.min(composition.layers.len());
    if sound {
        // Just below the sound track above this spot, or at the top of the sound tracks.
        let above = composition.layers[..index].iter().rev().find(|layer| is_sound_only(layer)).map(|layer| layer.track);
        let track = above.map_or(-1, |track| track - 1);
        for layer in composition.layers.iter_mut().filter(|layer| layer.track <= track) {
            layer.track -= 1;
        }
        return track;
    }
    // Just above the picture track below this spot; at the very bottom, the main track.
    let below = composition.layers[index..].iter().find(|layer| !is_sound_only(layer)).map(|layer| layer.track);
    let track = below.map_or(MAIN_TRACK, |track| track + 1);
    let taken = composition.layers.iter().any(|layer| layer.track == track);
    if taken {
        for layer in composition.layers.iter_mut().filter(|layer| layer.track >= track) {
            layer.track += 1;
        }
    }
    track
}

/// Where a clip lands when moved to `track` starting at `in_frame`: that track if the clip fits
/// there without overlapping another, otherwise a new track just above it (just below, for
/// sound tracks). Picture clips stay on picture tracks and sound on sound tracks. Returns the
/// track it went to.
pub fn place_clip(composition: &mut Composition, layer_id: Id, track: i32, in_frame: i64) -> Option<i32> {
    let layer = composition.layer(layer_id)?.clone();
    if layer.switches.locked {
        return Some(layer.track);
    }
    let sound = is_sound_only(&layer);
    // Picture belongs at or above the main track, sound below it.
    let track = if sound { track.min(-1) } else { track.max(MAIN_TRACK) };
    let out_frame = in_frame + (layer.out_frame - layer.in_frame);
    let target = if overlaps(composition, &layer, track, in_frame, out_frame) {
        // No room: a new track next to it, moving the others out of the way.
        let new_track = if sound { track - 1 } else { track + 1 };
        for other in composition.layers.iter_mut().filter(|other| other.id != layer_id) {
            if sound && other.track <= new_track {
                other.track -= 1;
            } else if !sound && other.track >= new_track {
                other.track += 1;
            }
        }
        new_track
    } else {
        track
    };
    let moved = composition.layer_mut(layer_id)?;
    moved.shift_in_time(in_frame - moved.in_frame);
    moved.track = target;
    normalize(composition);
    composition.layer(layer_id).map(|layer| layer.track)
}

/// Packs the main track's clips together from frame 0, in time order, keeping their lengths
/// (keyframes move with them). Locked clips stay where they are and the rest flow around them.
/// Returns whether anything moved.
pub fn compact_main_track(composition: &mut Composition) -> bool {
    let mut clips: Vec<(i64, Id)> = composition.layers.iter().filter(|layer| layer.track == MAIN_TRACK).map(|layer| (layer.in_frame, layer.id)).collect();
    clips.sort_unstable();
    let mut next = 0;
    let mut moved = false;
    for (_, id) in clips {
        let Some(layer) = composition.layer_mut(id) else { continue };
        if layer.switches.locked {
            next = next.max(layer.out_frame);
            continue;
        }
        if layer.in_frame != next {
            layer.shift_in_time(next - layer.in_frame);
            moved = true;
        }
        next = layer.out_frame;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Rgba;
    use crate::sample::sample_project;

    /// A composition of plain solids, top first, each with (track, in, out).
    fn comp(clips: &[(i32, i64, i64)]) -> Composition {
        let project = sample_project();
        let mut composition = project.compositions[0].clone();
        let template = composition.layers.iter().find(|l| matches!(l.content, LayerContent::Solid { .. })).unwrap().clone();
        composition.layers = clips
            .iter()
            .enumerate()
            .map(|(index, &(track, in_frame, out_frame))| Layer {
                id: Id(500 + index as u64),
                track,
                in_frame,
                out_frame,
                start_frame: in_frame,
                content: LayerContent::Solid { color: Rgba::hex(0x808080) },
                ..template.clone()
            })
            .collect();
        composition
    }

    fn rows(composition: &Composition) -> Vec<(u64, i32, i64)> {
        composition.layers.iter().map(|l| (l.id.0, l.track, l.in_frame)).collect()
    }

    #[test]
    fn older_files_get_one_track_per_layer_in_stacking_order() {
        let mut project = sample_project();
        let composition = &mut project.compositions[0];
        assign_tracks_by_stack(composition);
        let tracks: Vec<i32> = composition.layers.iter().map(|l| l.track).collect();
        // Six picture layers, top first: tracks 5 down to 0.
        assert_eq!(tracks, vec![5, 4, 3, 2, 1, 0]);
    }

    #[test]
    fn clips_move_onto_a_track_where_they_fit() {
        let mut composition = comp(&[(1, 0, 24), (0, 0, 48)]);
        // The top clip moves onto the main track after the other clip: one line, two clips.
        assert_eq!(place_clip(&mut composition, Id(500), 0, 48), Some(0));
        assert_eq!(rows(&composition), vec![(500, 0, 48), (501, 0, 0)]);
        assert_eq!(tracks(&composition), vec![0], "the emptied track is gone");
    }

    #[test]
    fn a_clip_that_would_overlap_gets_a_new_track_above() {
        let mut composition = comp(&[(1, 0, 24), (0, 0, 48)]);
        assert_eq!(place_clip(&mut composition, Id(500), 0, 10), Some(1), "no room on the main track at frame 10");
        // Back onto the main track's free time: fits.
        assert_eq!(place_clip(&mut composition, Id(500), 0, 60), Some(0));
    }

    #[test]
    fn sound_stays_below_the_main_track_and_picture_above() {
        let mut composition = comp(&[(0, 0, 48)]);
        composition.layers.push(Layer { id: Id(600), track: -1, content: LayerContent::Audio { item: Id(7) }, transform: None, ..composition.layers[0].clone() });
        assert_eq!(place_clip(&mut composition, Id(600), 3, 0), Some(-1), "sound can't go on a picture track");
        assert_eq!(place_clip(&mut composition, Id(500), -2, 0), Some(0), "picture can't go on a sound track");
    }

    #[test]
    fn higher_tracks_draw_on_top() {
        let mut composition = comp(&[(0, 0, 48), (2, 0, 48), (1, 0, 48)]);
        normalize(&mut composition);
        assert_eq!(composition.layers.iter().map(|l| l.track).collect::<Vec<_>>(), vec![2, 1, 0]);
    }

    #[test]
    fn a_matte_stays_directly_above_its_layer() {
        // Track 2 holds the matte M and another clip X; the matted layer L is on track 1.
        let mut composition = comp(&[(2, 0, 48), (1, 0, 48), (2, 60, 90)]);
        composition.layers[1].track_matte = TrackMatte::Alpha;
        normalize(&mut composition);
        let ids: Vec<u64> = composition.layers.iter().map(|l| l.id.0).collect();
        let (matte, user) = (ids.iter().position(|&id| id == 500).unwrap(), ids.iter().position(|&id| id == 501).unwrap());
        assert_eq!(user, matte + 1, "the matte is directly above its layer: {ids:?}");
    }

    #[test]
    fn new_layers_get_their_own_track_at_the_drop_height() {
        let mut composition = comp(&[(2, 0, 48), (1, 0, 48), (0, 0, 48)]);
        // Dropped between the track-2 and track-1 clips (stack position 1): a new track 2.
        let track = make_room_at(&mut composition, 1, false);
        assert_eq!(track, 2);
        assert_eq!(composition.layers.iter().map(|l| l.track).collect::<Vec<_>>(), vec![3, 1, 0], "the top clip moved up");
        // Into an empty composition: the main track.
        let mut empty = comp(&[]);
        assert_eq!(make_room_at(&mut empty, 0, false), MAIN_TRACK);
    }

    #[test]
    fn the_magnetic_main_track_closes_gaps() {
        let mut composition = comp(&[(0, 30, 50), (0, 100, 120), (1, 5, 10)]);
        assert!(compact_main_track(&mut composition));
        let main: Vec<(i64, i64)> = composition.layers.iter().filter(|l| l.track == 0).map(|l| (l.in_frame, l.out_frame)).collect();
        assert_eq!(main, vec![(0, 20), (20, 40)]);
        assert_eq!(composition.layers[2].in_frame, 5, "other tracks are not magnetic");
        assert!(!compact_main_track(&mut composition), "already packed");
    }

    #[test]
    fn locked_clips_hold_their_place_on_the_main_track() {
        let mut composition = comp(&[(0, 0, 10), (0, 50, 60), (0, 80, 90)]);
        composition.layers[1].switches.locked = true;
        compact_main_track(&mut composition);
        let main: Vec<(i64, i64)> = composition.layers.iter().map(|l| (l.in_frame, l.out_frame)).collect();
        assert_eq!(main, vec![(0, 10), (50, 60), (60, 70)]);
    }
}
