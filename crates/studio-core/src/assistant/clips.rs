//! The assistant's rules about clips rather than keyframes: how clips sit on their tracks,
//! how their sound is set, and how text and pictures are shown. Each reads only what the
//! composition already records (frames, sizes, volumes), so nothing here listens to or looks
//! at the media itself. The one rule that needs the sound measured, `clipping`, is handed the
//! loudest level by the caller.

use std::collections::BTreeMap;

use super::{Check, Fix, Options, Suggestion, plural};
use crate::animation::Vec2;
use crate::model::{Composition, Id, Layer, LayerContent};
use crate::property::{PropertyPath, PropertyValue, TransformProperty};

/// The longest gap between two clips, in frames, that counts as a slip rather than a pause.
const LONGEST_SLIP_GAP: i64 = 3;
/// A clip shorter than this many frames is a sliver.
const SHORTEST_CLIP: i64 = 6;
/// The fade given to a sound that starts abruptly, in frames: long enough to stop a click and
/// too short to hear as a fade.
const CLICK_FADE_FRAMES: f32 = 3.0;
/// A sound clip at least this many seconds long is taken to be music or a background bed.
const BED_SECONDS: f64 = 10.0;
/// Music and a voice must play together for at least this many seconds to be worth mentioning.
const TOGETHER_SECONDS: f64 = 2.0;
/// Music this many decibels or more below the voice is already out of its way.
const CLEAR_OF_VOICE_DB: f32 = 6.0;
/// A comfortable reading pace for a title, and the fastest pace captions can keep up with
/// speech, in words a second.
const TITLE_WORDS_A_SECOND: f64 = 3.0;
const CAPTION_WORDS_A_SECOND: f64 = 5.0;
/// Text shorter than this many words is read at a glance, however briefly it shows.
const FEWEST_WORDS_TO_TIME: usize = 3;
/// How far past the picture's edge text must reach to be mentioned, as a part of the picture's
/// width. A text layer's size is an estimate, so a small overhang isn't trusted.
const OFF_FRAME_SLACK: f32 = 0.02;
/// A picture scaled past this percentage of its own size starts to look soft.
const LARGEST_SHARP_SCALE: f32 = 110.0;

/// Whether the layer plays sound of its own from a file.
fn sounds(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Audio { .. }) && layer.has_audio && layer.switches.audio
}

/// The file a footage or sound layer plays, as the project item's id.
fn source(layer: &Layer) -> Option<Id> {
    match layer.content {
        LayerContent::Footage { item } | LayerContent::Audio { item } => Some(item),
        _ => None,
    }
}

/// How a track is named in a suggestion: picture tracks count up from 1, sound tracks too.
fn track_name(track: i32) -> String {
    if track >= 0 { format!("Video track {}", track + 1) } else { format!("Audio track {}", -track) }
}

fn seconds(frames: i64, fps: f64) -> f64 {
    frames as f64 / fps
}

/// The clips of each track, in the order they play.
fn by_track(composition: &Composition) -> BTreeMap<i32, Vec<&Layer>> {
    let mut tracks: BTreeMap<i32, Vec<&Layer>> = BTreeMap::new();
    for layer in &composition.layers {
        tracks.entry(layer.track).or_default().push(layer);
    }
    for clips in tracks.values_mut() {
        clips.sort_by_key(|layer| layer.in_frame);
    }
    tracks
}

/// A gap of a few frames between two clips on a track.
fn tiny_gaps(tracks: &BTreeMap<i32, Vec<&Layer>>, found: &mut Vec<Suggestion>) {
    for (track, clips) in tracks {
        // A locked clip can't be moved, so its whole track is left alone.
        if clips.iter().any(|layer| layer.switches.locked) {
            continue;
        }
        for pair in clips.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            let gap = after.in_frame - before.out_frame;
            if !(1..=LONGEST_SLIP_GAP).contains(&gap) {
                continue;
            }
            found.push(Suggestion {
                check: Check::TinyGap,
                layer: after.id,
                path: None,
                frame: before.out_frame,
                title: format!("{} before {}", plural(gap as usize, "empty frame", "empty frames"), after.name),
                detail: "A gap this short shows as a flash of whatever is underneath. The fix moves this clip and the ones after it on the track earlier to close it.".to_owned(),
                evidence: format!("{} · frame {} to {}", track_name(*track), before.out_frame, after.in_frame),
                fix: Some(Fix::CloseGap { track: *track, at: after.in_frame, by: gap }),
            });
        }
    }
}

/// A clip only a few frames long.
fn short_clips(composition: &Composition, found: &mut Vec<Suggestion>) {
    for layer in composition.layers.iter().filter(|layer| !layer.switches.locked) {
        let length = layer.out_frame - layer.in_frame;
        if !(1..SHORTEST_CLIP).contains(&length) {
            continue;
        }
        found.push(Suggestion {
            check: Check::ShortClip,
            layer: layer.id,
            path: None,
            frame: layer.in_frame,
            title: format!("{} is only {} long", layer.name, plural(length as usize, "frame", "frames")),
            detail: "That is too short to register, and is often a sliver left over from trimming. Delete it if it isn't meant.".to_owned(),
            evidence: format!("{} · frame {} to {}", track_name(layer.track), layer.in_frame, layer.out_frame),
            fix: Some(Fix::DeleteClip { layer: layer.id }),
        });
    }
}

/// Two pieces of the same footage side by side with a little of it missing between them.
fn jump_cuts(tracks: &BTreeMap<i32, Vec<&Layer>>, fps: f64, found: &mut Vec<Suggestion>) {
    let longest_skip = fps.round() as i64;
    for clips in tracks.values() {
        for pair in clips.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            let same_shot = matches!((&before.content, &after.content), (LayerContent::Footage { item: first }, LayerContent::Footage { item: second }) if first == second);
            // Speed changes move where in the source a frame comes from, so those are skipped.
            if !same_shot || before.out_frame != after.in_frame || !before.time.is_normal() || !after.time.is_normal() || after.switches.locked {
                continue;
            }
            // How much of the source is passed over at the cut; none means a plain split.
            let skipped = before.start_frame - after.start_frame;
            if !(1..=longest_skip).contains(&skipped) {
                continue;
            }
            found.push(Suggestion {
                check: Check::JumpCut,
                layer: after.id,
                path: None,
                frame: after.in_frame,
                title: format!("Jump cut in {}", after.name),
                detail: "Both sides of this cut are the same shot with a moment missing, so the picture jumps. Cover it with another shot, or cut to a different angle or size.".to_owned(),
                evidence: format!("{} of the shot skipped at frame {}", plural(skipped as usize, "frame", "frames"), after.in_frame),
                fix: None,
            });
        }
    }
}

/// A sound clip trimmed to start partway into its file, with no fade in.
fn abrupt_sound(tracks: &BTreeMap<i32, Vec<&Layer>>, fps: f64, found: &mut Vec<Suggestion>) {
    for clips in tracks.values() {
        for (index, layer) in clips.iter().enumerate() {
            // Footage is cut mid-sound all the time, on purpose; a sound clip of its own isn't.
            let is_sound_clip = matches!(layer.content, LayerContent::Audio { .. });
            if !is_sound_clip || !sounds(layer) || layer.switches.locked || layer.in_frame <= layer.start_frame || layer.audio.fade_in > 0.0 {
                continue;
            }
            // The second half of a split carries straight on from the first, so it can't click.
            let carries_on = index > 0 && {
                let before = clips[index - 1];
                before.out_frame == layer.in_frame && source(before) == source(layer) && before.start_frame == layer.start_frame
            };
            if carries_on {
                continue;
            }
            found.push(Suggestion {
                check: Check::AbruptSound,
                layer: layer.id,
                path: None,
                frame: layer.in_frame,
                title: format!("{} starts abruptly", layer.name),
                detail: "A sound cut in partway through usually begins with a click. A fade of a few frames removes it without being heard as a fade.".to_owned(),
                evidence: format!("Starts {:.2} s into its file, with no fade in", seconds(layer.in_frame - layer.start_frame, fps)),
                fix: Some(Fix::FadeIn { layer: layer.id, seconds: CLICK_FADE_FRAMES / fps as f32 }),
            });
        }
    }
}

/// The layers taken to carry a voice: sound with Enhance Speech on, and footage with sound
/// that plays while a caption layer is showing.
fn voices(composition: &Composition) -> Vec<&Layer> {
    let captions: Vec<(i64, i64)> = composition.layers.iter().filter(|layer| matches!(layer.content, LayerContent::Captions { .. })).map(|layer| (layer.in_frame, layer.out_frame)).collect();
    let captioned = |layer: &Layer| matches!(layer.content, LayerContent::Footage { .. }) && captions.iter().any(|(start, end)| layer.in_frame < *end && *start < layer.out_frame);
    composition.layers.iter().filter(|layer| sounds(layer) && (layer.audio.enhance.is_some() || captioned(layer))).collect()
}

/// A long sound clip playing under a voice at nearly the voice's volume.
fn music_over_voice(composition: &Composition, fps: f64, found: &mut Vec<Suggestion>) {
    let voices = voices(composition);
    for music in composition.layers.iter().filter(|layer| !layer.switches.locked) {
        let is_bed = matches!(music.content, LayerContent::Audio { .. }) && sounds(music) && seconds(music.out_frame - music.in_frame, fps) >= BED_SECONDS;
        if !is_bed || music.audio.duck_under.is_some() || voices.iter().any(|voice| voice.id == music.id) {
            continue;
        }
        // The voice it shares the most time with.
        let together = |voice: &Layer| music.out_frame.min(voice.out_frame) - music.in_frame.max(voice.in_frame);
        let Some(voice) = voices.iter().copied().max_by_key(|voice| together(voice)) else { continue };
        let shared = seconds(together(voice), fps);
        // Both volumes as they are where the two start playing together.
        let meet = music.in_frame.max(voice.in_frame) as f64;
        let (music_db, voice_db) = (music.audio.volume_db.value_at(meet), voice.audio.volume_db.value_at(meet));
        if shared < TOGETHER_SECONDS || music_db <= voice_db - CLEAR_OF_VOICE_DB {
            continue;
        }
        found.push(Suggestion {
            check: Check::MusicOverVoice,
            layer: music.id,
            path: None,
            frame: music.in_frame.max(voice.in_frame),
            title: format!("{} is as loud as {}", music.name, voice.name),
            detail: "Music at the same volume as a voice makes the words hard to follow. Ducking turns the music down whenever the voice is sounding, and back up in between.".to_owned(),
            evidence: format!("{} {music_db:+.1} dB · {} {voice_db:+.1} dB · {shared:.0} s together", music.name, voice.name),
            fix: Some(Fix::Duck { layer: music.id, under: voice.id }),
        });
    }
}

/// Text gone before it can be read: titles at a comfortable pace, captions at the pace of
/// fast speech.
fn brief_text(composition: &Composition, fps: f64, found: &mut Vec<Suggestion>) {
    let mut note = |layer: &Layer, frame: i64, words: usize, frames: i64, pace: f64| {
        let (shown, needed) = (seconds(frames, fps), words as f64 / pace);
        if words < FEWEST_WORDS_TO_TIME || shown >= needed {
            return;
        }
        found.push(Suggestion {
            check: Check::BriefText,
            layer: layer.id,
            path: None,
            frame,
            title: format!("{} is on screen too briefly to read", layer.name),
            detail: "Leave text up long enough to read it through at an easy pace, or use fewer words.".to_owned(),
            evidence: format!("{words} words in {shown:.1} s; about {needed:.1} s is needed"),
            fix: None,
        });
    };
    for layer in composition.layers.iter().filter(|layer| !layer.switches.locked && layer.switches.visible) {
        match &layer.content {
            LayerContent::Text { text, .. } => note(layer, layer.in_frame, text.split_whitespace().count(), layer.out_frame - layer.in_frame, TITLE_WORDS_A_SECOND),
            LayerContent::Captions { words, style } => {
                for phrase in crate::captions::phrases(words, style.words_per_phrase, fps) {
                    let (Some(first), Some(last)) = (words.get(phrase.start), phrase.end.checked_sub(1).and_then(|index| words.get(index))) else { continue };
                    note(layer, layer.start_frame + style.offset + first.start, phrase.len(), last.end - first.start, CAPTION_WORDS_A_SECOND);
                }
            }
            _ => {}
        }
    }
}

/// A text layer that reaches past the edge of the picture when it first shows.
fn text_off_frame(composition: &Composition, found: &mut Vec<Suggestion>) {
    let (width, height) = (composition.width as f32, composition.height as f32);
    let slack = width * OFF_FRAME_SLACK;
    for layer in composition.layers.iter().filter(|layer| !layer.switches.locked && layer.switches.visible && matches!(layer.content, LayerContent::Text { .. })) {
        let Some(corners) = crate::transform::layer_corners(composition, layer.id, layer.in_frame as f64) else { continue };
        // How far the text reaches past each edge, and the edge it passes the most.
        let past = |reach: fn(&Vec2) -> f32, limit: f32, sign: f32| corners.iter().map(|corner| (reach(corner) - limit) * sign).fold(0.0_f32, f32::max);
        let edges = [("left", past(|corner| corner.x, 0.0, -1.0)), ("right", past(|corner| corner.x, width, 1.0)), ("top", past(|corner| corner.y, 0.0, -1.0)), ("bottom", past(|corner| corner.y, height, 1.0))];
        let Some((edge, over)) = edges.into_iter().max_by(|first, second| first.1.total_cmp(&second.1)).filter(|(_, over)| *over > slack) else { continue };
        found.push(Suggestion {
            check: Check::TextOffFrame,
            layer: layer.id,
            path: None,
            frame: layer.in_frame,
            title: format!("{} runs off the {edge} of the picture", layer.name),
            detail: "Part of the text won't be seen. Move it in, make it smaller, or break the line. If it slides in from outside on purpose, dismiss this.".to_owned(),
            evidence: format!("About {over:.0} px past the {edge} edge at frame {}", layer.in_frame),
            fix: None,
        });
    }
}

/// Footage and images scaled past their own size.
fn upscaled(composition: &Composition, found: &mut Vec<Suggestion>) {
    for layer in composition.layers.iter().filter(|layer| !layer.switches.locked && layer.switches.visible) {
        if !matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Image { .. }) {
            continue;
        }
        let Some(PropertyValue::Pair(scale)) = layer.property(PropertyPath::Transform(TransformProperty::Scale), layer.in_frame).map(|scale| scale.value) else { continue };
        let largest = scale.x.abs().max(scale.y.abs());
        if largest <= LARGEST_SHARP_SCALE {
            continue;
        }
        found.push(Suggestion {
            check: Check::Upscaled,
            layer: layer.id,
            path: None,
            frame: layer.in_frame,
            title: format!("{} is scaled to {largest:.0}%", layer.name),
            detail: "Past its own size a picture has no more detail to show, so it looks soft. Use a larger original if there is one.".to_owned(),
            evidence: format!("{:.0} × {:.0} px original shown at {largest:.0}%", layer.size.x, layer.size.y),
            fix: None,
        });
    }
}

/// Adds what the switched-on clip rules notice in `composition` to `found`.
pub(super) fn find(composition: &Composition, options: &Options, found: &mut Vec<Suggestion>) {
    let fps = composition.frame_rate.fps().max(1.0);
    let tracks = by_track(composition);
    if options.tiny_gaps {
        tiny_gaps(&tracks, found);
    }
    if options.short_clips {
        short_clips(composition, found);
    }
    if options.jump_cuts {
        jump_cuts(&tracks, fps, found);
    }
    if options.abrupt_sound {
        abrupt_sound(&tracks, fps, found);
    }
    if options.music_over_voice {
        music_over_voice(composition, fps, found);
    }
    if options.brief_text {
        brief_text(composition, fps, found);
    }
    if options.text_off_frame {
        text_off_frame(composition, found);
    }
    if options.upscaled {
        upscaled(composition, found);
    }
}

/// Sound boosted past full volume. `loudest` is the loudest moment of the part of the file
/// the clip plays, from 0 (silence) to 1 (full volume), as measured by the caller. A clip left
/// at its recorded volume is never mentioned: only raising the volume can push it over. A
/// clip whose volume is keyframed is judged by its highest keyframe, and gets no automatic
/// fix, since turning it down is a choice about each keyframe.
pub fn clipping(layer: &Layer, loudest: f32) -> Option<Suggestion> {
    let volume = &layer.audio.volume_db;
    let highest = volume.keyframe_frames().into_iter().map(|frame| volume.value_at(frame as f64)).fold(volume.value_at(layer.in_frame as f64), f32::max);
    if layer.switches.locked || !sounds(layer) || highest <= 0.0 || loudest <= 0.0 {
        return None;
    }
    let over = 20.0 * loudest.log10() + highest;
    if over <= 0.0 {
        return None;
    }
    // Rounded down to a tenth, so the new volume is never still over.
    let db = ((highest - over) * 10.0).floor() / 10.0;
    Some(Suggestion {
        check: Check::Clipping,
        layer: layer.id,
        path: None,
        frame: layer.in_frame,
        title: format!("{} is boosted past full volume", layer.name),
        detail: "Its loudest moment is pushed beyond what can be played, which distorts. The fix turns the clip down until that moment just fits.".to_owned(),
        evidence: format!("Volume {highest:+.1} dB · loudest moment {over:+.1} dB over"),
        fix: (!volume.is_animated()).then_some(Fix::SetVolume { layer: layer.id, db }),
    })
}

/// Makes one of the fixes that change clips rather than keyframes. Returns whether anything
/// changed; locked layers are left alone.
pub(super) fn apply(composition: &mut Composition, fix: &Fix) -> bool {
    let unlocked = |composition: &mut Composition, id: Id| composition.layers.iter().position(|layer| layer.id == id && !layer.switches.locked);
    match fix {
        Fix::CloseGap { track, at, by } => {
            let on_track = |layer: &&mut Layer| layer.track == *track;
            // The gap must still be there, and nothing on the track may be locked.
            let still_open = composition.layers.iter().filter(|layer| layer.track == *track).all(|layer| !layer.switches.locked && !(layer.in_frame < *at && layer.out_frame > *at - *by));
            if *by <= 0 || !still_open {
                return false;
            }
            let mut moved = false;
            for layer in composition.layers.iter_mut().filter(on_track).filter(|layer| layer.in_frame >= *at) {
                layer.shift_in_time(-*by);
                moved = true;
            }
            moved
        }
        Fix::DeleteClip { layer } => unlocked(composition, *layer).is_some() && crate::edit::delete_layers(composition, &[*layer]) > 0,
        Fix::FadeIn { layer, seconds } => unlocked(composition, *layer).is_some_and(|index| {
            composition.layers[index].audio.fade_in = *seconds;
            true
        }),
        Fix::Duck { layer, under } => composition.layer(*under).is_some() && unlocked(composition, *layer).is_some_and(|index| {
            composition.layers[index].audio.duck_under = Some(*under);
            true
        }),
        Fix::SetVolume { layer, db } => unlocked(composition, *layer).is_some_and(|index| {
            composition.layers[index].audio.volume_db = crate::animation::Animated::constant(*db);
            true
        }),
        Fix::Ease { .. } | Fix::Lengthen { .. } | Fix::Remove { .. } | Fix::Move { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{apply, suggestions};
    use super::*;
    use crate::edit::{self, FootageInfo};
    use crate::model::{Project, SpeechEnhance};
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    const COMP: Id = ids::MAIN_COMP;

    /// A project whose main composition is empty, at 24 frames a second.
    fn empty() -> Project {
        let mut project = sample_project();
        let composition = project.composition_mut(COMP).unwrap();
        composition.layers.clear();
        composition.duration_frames = 2400;
        project
    }

    /// Only `check` switched on.
    fn only(check: Check) -> Options {
        let mut options = Options::default();
        for other in Check::ALL {
            options.set(other, other == check);
        }
        options
    }

    fn found(project: &Project, check: Check) -> Vec<Suggestion> {
        suggestions(project.composition(COMP).unwrap(), &only(check))
    }

    /// Adds a 20-second clip of talking footage starting at `frame`.
    fn add_footage(project: &mut Project, frame: i64) -> Id {
        let info = FootageInfo { size: Vec2::new(640.0, 360.0), duration_seconds: 20.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(project, "/media/talk.mp4".into(), &info);
        edit::add_layer_from_item(project, COMP, item, 0, frame).unwrap()
    }

    /// Adds a sound clip `seconds` long starting at `frame`.
    fn add_sound(project: &mut Project, seconds: f64, frame: i64) -> Id {
        let item = edit::import_audio(project, "/media/music.m4a".into(), seconds);
        edit::add_layer_from_item(project, COMP, item, 0, frame).unwrap()
    }

    fn layer_mut(project: &mut Project, id: Id) -> &mut Layer {
        project.composition_mut(COMP).unwrap().layer_mut(id).unwrap()
    }

    /// Puts two solids on one track, the second starting `gap` frames after the first ends.
    fn two_clips(gap: i64) -> (Project, Id, Id) {
        let mut project = empty();
        let first = edit::add_text_layer(&mut project, COMP, "One", Vec2::new(400.0, 300.0), 0).unwrap();
        let second = edit::add_text_layer(&mut project, COMP, "Two", Vec2::new(400.0, 300.0), 0).unwrap();
        for (id, start, end) in [(first, 0, 100), (second, 100 + gap, 200 + gap)] {
            let layer = layer_mut(&mut project, id);
            (layer.in_frame, layer.out_frame, layer.start_frame, layer.track) = (start, end, start, 3);
        }
        (project, first, second)
    }

    #[test]
    fn a_two_frame_gap_is_noticed_and_closed() {
        let (mut project, _, second) = two_clips(2);
        let gaps = found(&project, Check::TinyGap);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].fix, Some(Fix::CloseGap { track: 3, at: 102, by: 2 }));
        let composition = project.composition_mut(COMP).unwrap();
        assert!(apply(composition, gaps[0].fix.as_ref().unwrap()));
        assert_eq!(composition.layer(second).unwrap().in_frame, 100);
        assert!(!apply(composition, gaps[0].fix.as_ref().unwrap()), "the gap is gone, so the fix does nothing a second time");
        assert!(found(&project, Check::TinyGap).is_empty());
    }

    #[test]
    fn touching_clips_and_real_pauses_are_not_gaps() {
        assert!(found(&two_clips(0).0, Check::TinyGap).is_empty());
        assert!(found(&two_clips(12).0, Check::TinyGap).is_empty());
    }

    #[test]
    fn a_sliver_of_a_clip_is_noticed_and_deleted() {
        let (mut project, first, _) = two_clips(0);
        layer_mut(&mut project, first).out_frame = 4;
        let slivers = found(&project, Check::ShortClip);
        assert_eq!(slivers.len(), 1);
        assert_eq!(slivers[0].fix, Some(Fix::DeleteClip { layer: first }));
        let composition = project.composition_mut(COMP).unwrap();
        assert!(apply(composition, slivers[0].fix.as_ref().unwrap()));
        assert!(composition.layer(first).is_none());
    }

    #[test]
    fn a_shot_with_a_moment_cut_out_is_a_jump_cut_but_a_plain_split_is_not() {
        let mut project = empty();
        let clip = add_footage(&mut project, 0);
        crate::cut::split_layers(&mut project, COMP, &[clip], 100);
        assert!(found(&project, Check::JumpCut).is_empty(), "a split plays straight through");
        // Take six frames off the start of the second piece and close up, as a ripple trim does.
        let second = project.composition(COMP).unwrap().layers.iter().find(|layer| layer.in_frame == 100).unwrap().id;
        layer_mut(&mut project, second).start_frame -= 6;
        let jumps = found(&project, Check::JumpCut);
        assert_eq!(jumps.len(), 1);
        assert_eq!((jumps[0].layer, jumps[0].frame, &jumps[0].fix), (second, 100, &None));
    }

    #[test]
    fn a_sound_trimmed_to_start_partway_in_gets_a_short_fade() {
        let mut project = empty();
        let sound = add_sound(&mut project, 5.0, 0);
        assert!(found(&project, Check::AbruptSound).is_empty(), "it plays from its own start");
        layer_mut(&mut project, sound).in_frame = 24;
        let abrupt = found(&project, Check::AbruptSound);
        assert_eq!(abrupt.len(), 1);
        let composition = project.composition_mut(COMP).unwrap();
        assert!(apply(composition, abrupt[0].fix.as_ref().unwrap()));
        assert!((composition.layer(sound).unwrap().audio.fade_in - 3.0 / 24.0).abs() < 1e-6, "three frames");
        assert!(found(&project, Check::AbruptSound).is_empty());
    }

    #[test]
    fn music_under_a_voice_is_ducked_once_it_is_known_to_be_a_voice() {
        let mut project = empty();
        let voice = add_footage(&mut project, 0);
        let music = add_sound(&mut project, 60.0, 0);
        assert!(found(&project, Check::MusicOverVoice).is_empty(), "nothing marks the footage as speech yet");
        layer_mut(&mut project, voice).audio.enhance = Some(SpeechEnhance::default());
        let loud = found(&project, Check::MusicOverVoice);
        assert_eq!(loud.len(), 1);
        assert_eq!(loud[0].fix, Some(Fix::Duck { layer: music, under: voice }));
        // Music already well under the voice is left alone.
        layer_mut(&mut project, music).audio.volume_db = crate::animation::Animated::constant(-12.0);
        assert!(found(&project, Check::MusicOverVoice).is_empty());
        layer_mut(&mut project, music).audio.volume_db = crate::animation::Animated::constant(0.0);
        let composition = project.composition_mut(COMP).unwrap();
        assert!(apply(composition, loud[0].fix.as_ref().unwrap()));
        assert_eq!(composition.layer(music).unwrap().audio.duck_under, Some(voice));
        assert!(found(&project, Check::MusicOverVoice).is_empty());
    }

    #[test]
    fn a_boosted_clip_is_turned_down_until_its_loudest_moment_fits() {
        let mut project = empty();
        let sound = add_sound(&mut project, 5.0, 0);
        assert!(clipping(project.composition(COMP).unwrap().layer(sound).unwrap(), 1.0).is_none(), "as recorded, it can't be over");
        layer_mut(&mut project, sound).audio.volume_db = crate::animation::Animated::constant(6.0);
        let layer = project.composition(COMP).unwrap().layer(sound).unwrap();
        assert!(clipping(layer, 0.4).is_none(), "a quiet clip has room for the boost");
        let over = clipping(layer, 0.9).unwrap();
        let Some(Fix::SetVolume { db, .. }) = over.fix else { panic!("no fix") };
        assert!(0.9 * 10.0_f32.powf(db / 20.0) <= 1.0 && db > 0.5, "just fits: {db}");
    }

    #[test]
    fn a_long_title_shown_for_a_moment_is_too_brief() {
        let mut project = empty();
        let title = edit::add_text_layer(&mut project, COMP, "Nine words take about three seconds to read through", Vec2::new(400.0, 300.0), 0).unwrap();
        layer_mut(&mut project, title).out_frame = 24;
        assert_eq!(found(&project, Check::BriefText).len(), 1);
        layer_mut(&mut project, title).out_frame = 96;
        assert!(found(&project, Check::BriefText).is_empty());
    }

    #[test]
    fn text_past_the_edge_of_the_picture_is_noticed() {
        let mut project = empty();
        let width = project.composition(COMP).unwrap().width as f32;
        let title = edit::add_text_layer(&mut project, COMP, "Hello there", Vec2::new(width / 2.0, 300.0), 0).unwrap();
        assert!(found(&project, Check::TextOffFrame).is_empty(), "in the middle of the picture");
        let position = &mut layer_mut(&mut project, title).transform.as_mut().unwrap().position;
        *position = crate::animation::Animated::constant(Vec2::new(width, 300.0));
        let off = found(&project, Check::TextOffFrame);
        assert_eq!(off.len(), 1);
        assert!(off[0].title.contains("right"), "{}", off[0].title);
    }

    #[test]
    fn footage_scaled_up_is_noticed() {
        let mut project = empty();
        let clip = add_footage(&mut project, 0);
        assert!(found(&project, Check::Upscaled).is_empty());
        let scale = &mut layer_mut(&mut project, clip).transform.as_mut().unwrap().scale;
        *scale = crate::animation::Animated::constant(Vec2::new(240.0, 240.0));
        let soft = found(&project, Check::Upscaled);
        assert_eq!(soft.len(), 1);
        assert!(soft[0].title.contains("240%"));
    }
}
