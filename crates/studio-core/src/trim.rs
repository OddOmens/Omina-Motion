//! Trimming and slipping layers in time, as in Premiere Pro.
//!
//! - **Trimming** moves a layer's in or out point. Footage and audio can only be trimmed
//!   within their own length; stills, solids and text can be any length.
//! - **Slipping** changes which part of the source plays, keeping the layer where it is on
//!   the timeline: the in and out points stay, and the source moves underneath them.
//!   Keyframes stay where they are in the composition.
//!
//! Locked layers are never changed. A layer always keeps at least one frame.

use crate::model::{Composition, Id, LayerContent, Layer, Project, ProjectItemKind};

/// How long a layer's source is, in composition frames, or `None` if it has no fixed length
/// (stills, solids, text and adjustment layers can be stretched as far as wanted).
pub fn source_frames(project: &Project, composition: &Composition, layer: &Layer) -> Option<i64> {
    let item = match layer.content {
        LayerContent::Footage { item } | LayerContent::Audio { item } => item,
        LayerContent::Composition { composition } => composition,
        _ => return None,
    };
    let item = project.item(item)?;
    if !matches!(item.kind, ProjectItemKind::Footage | ProjectItemKind::Audio | ProjectItemKind::Composition) {
        return None;
    }
    item.duration_seconds.map(|seconds| composition.frame_rate.seconds_to_frames(seconds).max(1))
}

/// The room a trim has on the layer's track: the out point of the clip before it, and the in
/// point of the clip after it. Clips on a track never overlap.
fn neighbors(composition: &Composition, layer: &Layer) -> (i64, i64) {
    let same_track = composition.layers.iter().filter(|other| other.id != layer.id && other.track == layer.track);
    let before = same_track.clone().filter(|other| other.out_frame <= layer.in_frame).map(|other| other.out_frame).max();
    let after = same_track.filter(|other| other.in_frame >= layer.out_frame).map(|other| other.in_frame).min();
    (before.unwrap_or(i64::MIN), after.unwrap_or(i64::MAX))
}

/// The layer (unlocked), its source length, and the room its neighbors on the track leave, for an edit.
fn editable(project: &mut Project, comp: Id, layer: Id) -> Option<(&mut Layer, Option<i64>, (i64, i64))> {
    let composition = project.composition(comp)?;
    let found = composition.layer(layer)?;
    let length = source_frames(project, composition, found);
    let room = neighbors(composition, found);
    let layer = project.composition_mut(comp)?.layer_mut(layer)?;
    (!layer.switches.locked).then_some((layer, length, room))
}

/// Moves the layer's in point to `frame`, as far as its source and out point allow.
/// Returns whether anything changed.
pub fn trim_in(project: &mut Project, comp: Id, layer: Id, frame: i64) -> bool {
    let Some((layer, length, (before, _))) = editable(project, comp, layer) else { return false };
    // Not before the source's first frame or into the clip before it on the track, and at
    // least one frame before the out point.
    let earliest = length.map_or(i64::MIN, |_| layer.start_frame).max(before);
    let frame = frame.max(earliest).min(layer.out_frame - 1);
    let changed = frame != layer.in_frame;
    layer.in_frame = frame;
    changed
}

/// Moves the layer's out point (the frame after its last) to `frame`, as far as its source
/// and in point allow. Returns whether anything changed.
pub fn trim_out(project: &mut Project, comp: Id, layer: Id, frame: i64) -> bool {
    let Some((layer, length, (_, after))) = editable(project, comp, layer) else { return false };
    // Not past the source's last frame or into the clip after it on the track, and at least
    // one frame after the in point.
    let latest = length.map_or(i64::MAX, |length| layer.start_frame + length).min(after);
    let frame = frame.min(latest).max(layer.in_frame + 1);
    let changed = frame != layer.out_frame;
    layer.out_frame = frame;
    changed
}

/// Whether the layer's source can be slipped: it has a fixed length and more of it than is
/// showing, so there is something to slip to.
pub fn can_slip(project: &Project, composition: &Composition, layer: &Layer) -> bool {
    source_frames(project, composition, layer).is_some_and(|length| length > layer.out_frame - layer.in_frame)
}

/// Slips the layer's source so its first frame plays at composition frame `start`, keeping
/// the in and out points; the source must still cover them. Returns whether anything changed.
pub fn slip_to(project: &mut Project, comp: Id, layer: Id, start: i64) -> bool {
    let Some((layer, Some(length), _)) = editable(project, comp, layer) else { return false };
    // A source shorter than the layer (after a frame rate change, say) cannot cover it anywhere.
    if length < layer.out_frame - layer.in_frame {
        return false;
    }
    // The source has to start by the in point and run to the out point.
    let start = start.clamp(layer.out_frame - length, layer.in_frame);
    let changed = start != layer.start_frame;
    layer.start_frame = start;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit::{self, FootageInfo};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    /// The sample project with a 4-second (96-frame) clip placed at frame 24.
    fn with_clip() -> (Project, Id) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 4.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/clip.mp4".into(), &info);
        let layer = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 24).unwrap();
        (project, layer)
    }

    fn timing(project: &Project, layer: Id) -> (i64, i64, i64) {
        let layer = project.composition(ids::MAIN_COMP).unwrap().layer(layer).unwrap();
        (layer.start_frame, layer.in_frame, layer.out_frame)
    }

    #[test]
    fn footage_trims_only_within_its_length() {
        let (mut project, clip) = with_clip();
        assert_eq!(timing(&project, clip), (24, 24, 120));
        assert!(trim_in(&mut project, ids::MAIN_COMP, clip, 48));
        assert!(trim_out(&mut project, ids::MAIN_COMP, clip, 100));
        assert_eq!(timing(&project, clip), (24, 48, 100), "the source stays put");
        // Back past the source's ends: stops at its first and last frames.
        trim_in(&mut project, ids::MAIN_COMP, clip, 0);
        trim_out(&mut project, ids::MAIN_COMP, clip, 500);
        assert_eq!(timing(&project, clip), (24, 24, 120));
    }

    #[test]
    fn a_layer_keeps_at_least_one_frame() {
        let (mut project, clip) = with_clip();
        trim_in(&mut project, ids::MAIN_COMP, clip, 1000);
        assert_eq!(timing(&project, clip), (24, 119, 120));
        trim_out(&mut project, ids::MAIN_COMP, clip, -5);
        assert_eq!(timing(&project, clip), (24, 119, 120));
    }

    #[test]
    fn stills_and_text_stretch_freely() {
        let mut project = sample_project();
        let title = Id(100);
        let (in_frame, out_frame) = {
            let layer = project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap();
            (layer.in_frame, layer.out_frame)
        };
        assert!(trim_in(&mut project, ids::MAIN_COMP, title, in_frame - 20));
        assert!(trim_out(&mut project, ids::MAIN_COMP, title, out_frame + 500));
        let layer = project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (in_frame - 20, out_frame + 500));
        assert!(!can_slip(&project, project.composition(ids::MAIN_COMP).unwrap(), layer), "nothing to slip");
    }

    #[test]
    fn slipping_moves_the_source_under_fixed_in_and_out_points() {
        let (mut project, clip) = with_clip();
        trim_in(&mut project, ids::MAIN_COMP, clip, 48);
        trim_out(&mut project, ids::MAIN_COMP, clip, 96);
        // 48 frames shown of 96: the source can start anywhere from frame 0 to 48.
        assert!(slip_to(&mut project, ids::MAIN_COMP, clip, 10));
        assert_eq!(timing(&project, clip), (10, 48, 96));
        slip_to(&mut project, ids::MAIN_COMP, clip, -100);
        assert_eq!(timing(&project, clip), (0, 48, 96), "the source must still reach the out point");
        slip_to(&mut project, ids::MAIN_COMP, clip, 100);
        assert_eq!(timing(&project, clip), (48, 48, 96), "the source must start by the in point");
    }

    #[test]
    fn a_source_shorter_than_its_layer_cannot_slip() {
        let (mut project, clip) = with_clip();
        // At half the frame rate, the 4-second source is 48 frames, but the layer is still 96 long.
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.frame_rate = FrameRate::new(12, 1);
        assert!(!slip_to(&mut project, ids::MAIN_COMP, clip, 30));
        assert_eq!(timing(&project, clip), (24, 24, 120));
    }

    #[test]
    fn trims_stop_at_the_next_clip_on_the_same_track() {
        let mut project = sample_project();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        // The title (24–168) and the logo (48–240) put on one track, the logo moved after the title.
        let (title, logo) = (comp.layers[0].id, comp.layers[2].id);
        let logo_layer = comp.layer_mut(logo).unwrap();
        logo_layer.shift_in_time(200 - 48);
        logo_layer.track = 5;
        assert!(trim_out(&mut project, ids::MAIN_COMP, title, 300));
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layer(title).unwrap().out_frame, 200, "stops at the logo");
        assert!(!trim_in(&mut project, ids::MAIN_COMP, logo, 100), "no room: nothing changes");
        assert_eq!(project.composition(ids::MAIN_COMP).unwrap().layer(logo).unwrap().in_frame, 200, "the logo can't reach into the title");
    }

    #[test]
    fn locked_layers_do_not_change() {
        let (mut project, clip) = with_clip();
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(clip).unwrap().switches.locked = true;
        assert!(!trim_in(&mut project, ids::MAIN_COMP, clip, 48));
        assert!(!trim_out(&mut project, ids::MAIN_COMP, clip, 48));
        assert!(!slip_to(&mut project, ids::MAIN_COMP, clip, 0));
        assert_eq!(timing(&project, clip), (24, 24, 120));
    }

    #[test]
    fn moving_a_layer_carries_its_source_along() {
        let (mut project, clip) = with_clip();
        trim_in(&mut project, ids::MAIN_COMP, clip, 48);
        project.composition_mut(ids::MAIN_COMP).unwrap().layer_mut(clip).unwrap().shift_in_time(10);
        assert_eq!(timing(&project, clip), (34, 58, 130));
    }
}
