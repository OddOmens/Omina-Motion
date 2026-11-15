//! Editing by transcript: cutting words out of a caption layer cuts the time they are spoken
//! out of the whole composition, the way text-based editing works in Premiere. Every unlocked
//! layer loses that span and what follows closes up, so picture, sound, titles and captions
//! all stay in step.

use std::ops::Range;

use crate::captions::CaptionWord;
use crate::model::{Composition, Id, Layer, LayerContent, Project};

/// Sounds people make while thinking, which "Remove Filler Words" cuts. Only single words:
/// phrases such as "you know" are often meant, so they are left for the editor to judge.
pub const FILLER_WORDS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "er", "erm", "ah", "ahh", "eh", "hm", "hmm", "mm", "mhm"];

/// Whether a transcribed word is a filler sound, ignoring capitals and the punctuation speech
/// recognition attaches ("Um," counts).
pub fn is_filler(text: &str) -> bool {
    let bare: String = text.chars().filter(|character| character.is_alphabetic()).flat_map(char::to_lowercase).collect();
    FILLER_WORDS.contains(&bare.as_str())
}

/// When words `range` of a caption layer are spoken, in composition frames: from the first
/// word's start to the last word's end, with the layer's sync applied.
pub fn word_span(layer: &Layer, range: Range<usize>) -> Option<(i64, i64)> {
    let LayerContent::Captions { words, style } = &layer.content else { return None };
    let (first, last) = (words.get(range.start)?, words.get(range.end.checked_sub(1)?)?);
    let base = layer.start_frame + style.offset;
    (last.end > first.start).then_some((base + first.start, base + last.end))
}

/// Where a moment lands once the frames `from..to` are gone: moments before the cut stay,
/// moments inside it collapse onto its start, and later ones move earlier by its length.
fn after_cut(frame: i64, from: i64, to: i64) -> i64 {
    if frame <= from {
        frame
    } else if frame < to {
        from
    } else {
        frame - (to - from)
    }
}

/// Takes the frames `from..to` (relative to the words' own clock) out of a caption layer's
/// words: a word spoken mostly inside the cut is dropped, and the rest are retimed to follow
/// on directly.
fn cut_caption_words(words: &mut Vec<CaptionWord>, from: i64, to: i64) {
    words.retain(|word| {
        let middle = (word.start + word.end) / 2;
        !(from..to).contains(&middle)
    });
    for word in words.iter_mut() {
        word.start = after_cut(word.start, from, to);
        word.end = after_cut(word.end, from, to).max(word.start + 1);
    }
}

/// Removes the frames `from..to` from a composition and closes the gap. Each unlocked layer
/// that crosses the span is split around it and loses the part inside; layers wholly inside
/// are deleted; everything later moves earlier, keyframes and markers included. A caption
/// layer is not split: it loses the words spoken in the span and the rest are retimed, so it
/// stays one layer with one transcript. Locked layers are left exactly as they are. Returns
/// whether anything was removed.
pub fn remove_time(project: &mut Project, comp: Id, from: i64, to: i64) -> bool {
    if to <= from || project.composition(comp).is_none() {
        return false;
    }
    let length = to - from;
    let is_captions = |layer: &Layer| matches!(layer.content, LayerContent::Captions { .. });
    // The layers to split at one edge of the span: unlocked, not captions, and crossing it.
    let crossing = |composition: &Composition, frame: i64| -> Vec<Id> {
        composition.layers.iter().filter(|layer| !layer.switches.locked && !is_captions(layer) && layer.in_frame < frame && frame < layer.out_frame).map(|layer| layer.id).collect()
    };
    // The later edge first, so the pieces after the span exist before the span is cut loose.
    for edge in [to, from] {
        let ids = crossing(project.composition(comp).expect("checked above"), edge);
        // An empty list would mean "every layer" to `split_layers`, locked ones included.
        if !ids.is_empty() {
            crate::cut::split_layers(project, comp, &ids, edge);
        }
    }
    let composition = project.composition_mut(comp).expect("checked above");
    let mut doomed: Vec<Id> = Vec::new();
    for layer in composition.layers.iter_mut().filter(|layer| !layer.switches.locked) {
        let captions = is_captions(layer);
        if layer.in_frame >= to {
            layer.shift_in_time(-length);
        } else if captions && layer.out_frame > from {
            // Its words are timed from its start frame plus its sync, which don't move.
            let start_frame = layer.start_frame;
            if let LayerContent::Captions { words, style } = &mut layer.content {
                let base = start_frame + style.offset;
                cut_caption_words(words, from - base, to - base);
            }
            layer.in_frame = after_cut(layer.in_frame, from, to);
            layer.out_frame = after_cut(layer.out_frame, from, to);
            if layer.out_frame <= layer.in_frame {
                doomed.push(layer.id);
            }
        } else if !captions && layer.in_frame >= from && layer.out_frame <= to {
            doomed.push(layer.id);
        }
    }
    crate::edit::delete_layers(composition, &doomed);
    composition.markers.retain(|marker| !(from..to).contains(&marker.frame));
    for marker in composition.markers.iter_mut().filter(|marker| marker.frame >= to) {
        marker.frame -= length;
    }
    crate::tracks::normalize(composition);
    true
}

/// Cuts words `range` of caption layer `captions` out of the composition: the time they are
/// spoken is removed from every unlocked layer (see `remove_time`). Returns how many frames
/// were removed, or `None` when the layer is locked, isn't captions, or the range is empty.
pub fn cut_words(project: &mut Project, comp: Id, captions: Id, range: Range<usize>) -> Option<i64> {
    let layer = project.composition(comp)?.layer(captions)?;
    if layer.switches.locked {
        return None;
    }
    let (from, to) = word_span(layer, range)?;
    remove_time(project, comp, from, to).then_some(to - from)
}

/// Cuts every filler word (see `is_filler`) of caption layer `captions` out of the
/// composition. Returns how many were cut.
pub fn cut_filler_words(project: &mut Project, comp: Id, captions: Id) -> usize {
    let fillers: Vec<usize> = match project.composition(comp).and_then(|composition| composition.layer(captions)).map(|layer| &layer.content) {
        Some(LayerContent::Captions { words, .. }) => words.iter().enumerate().filter(|(_, word)| is_filler(&word.text)).map(|(index, _)| index).collect(),
        _ => return 0,
    };
    // From the last back, so the earlier words keep their numbers as later ones go.
    fillers.into_iter().rev().filter(|&index| cut_words(project, comp, captions, index..index + 1).is_some()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::captions::CaptionStyle;
    use crate::edit::{self, FootageInfo};
    use crate::model::Marker;
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;

    /// A ten-second talking clip from frame 0 with captions over it: "So um this works", each
    /// word a second long (24 frames), starting at 0, 24, 48 and 72.
    fn talk() -> (Project, Id, Id) {
        let mut project = sample_project();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 10.0, frame_rate: FrameRate::new(24, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/talk.mp4".into(), &info);
        let clip = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 0).unwrap();
        project.composition_mut(ids::MAIN_COMP).unwrap().duration_frames = 480;
        let words = ["So", "um,", "this", "works"].iter().enumerate().map(|(index, text)| CaptionWord::new(*text, index as i64 * 24, index as i64 * 24 + 24)).collect();
        let captions = edit::add_caption_layer(&mut project, ids::MAIN_COMP, words, CaptionStyle::default(), 0..240, 0).unwrap();
        (project, clip, captions)
    }

    fn caption_words(project: &Project, captions: Id) -> Vec<(String, i64, i64)> {
        match &project.composition(ids::MAIN_COMP).unwrap().layer(captions).unwrap().content {
            LayerContent::Captions { words, .. } => words.iter().map(|word| (word.text.clone(), word.start, word.end)).collect(),
            _ => panic!("not a caption layer"),
        }
    }

    #[test]
    fn filler_words_are_recognized_whatever_their_punctuation() {
        assert!(is_filler("Um,"));
        assert!(is_filler("uh..."));
        assert!(!is_filler("umbrella"));
        assert!(!is_filler("so"));
    }

    #[test]
    fn cutting_a_word_cuts_its_time_from_the_clip_and_the_captions() {
        let (mut project, clip, captions) = talk();
        assert_eq!(cut_words(&mut project, ids::MAIN_COMP, captions, 1..2), Some(24));
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let track = composition.layer(clip).unwrap().track;
        let mut pieces: Vec<(i64, i64, i64)> = composition.layers.iter().filter(|layer| layer.track == track).map(|layer| (layer.in_frame, layer.out_frame, layer.start_frame)).collect();
        pieces.sort();
        // The clip plays its first second, then straight on from its third.
        assert_eq!(pieces, vec![(0, 24, 0), (24, 216, -24)]);
        // The captions are still one layer, a second shorter, without the word.
        assert_eq!(caption_words(&project, captions), vec![("So".to_owned(), 0, 24), ("this".to_owned(), 24, 48), ("works".to_owned(), 48, 72)]);
        let layer = composition.layer(captions).unwrap();
        assert_eq!((layer.in_frame, layer.out_frame), (0, 216));
    }

    #[test]
    fn layers_after_the_cut_and_markers_close_up_and_locked_layers_stay() {
        let (mut project, _, captions) = talk();
        let composition = project.composition_mut(ids::MAIN_COMP).unwrap();
        // The sample's title runs over frames 24–168; lock the layer below it to leave it alone.
        let title = composition.layer(Id(100)).map(|layer| (layer.in_frame, layer.out_frame)).unwrap();
        let locked = composition.layers.iter().find(|layer| layer.id != Id(100) && !matches!(layer.content, LayerContent::Captions { .. })).map(|layer| layer.id).unwrap();
        let locked_span = {
            let layer = composition.layer_mut(locked).unwrap();
            layer.switches.locked = true;
            (layer.in_frame, layer.out_frame)
        };
        composition.markers = vec![Marker { frame: 30, label: "Inside".into() }, Marker { frame: 100, label: "After".into() }];
        // Cut "um," (frames 24–48).
        cut_words(&mut project, ids::MAIN_COMP, captions, 1..2).unwrap();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let after = composition.layer(locked).unwrap();
        assert_eq!((after.in_frame, after.out_frame), locked_span, "a locked layer is untouched");
        assert_eq!(composition.markers.iter().map(|marker| marker.frame).collect::<Vec<_>>(), vec![76], "the marker inside went, the later one moved up");
        // The title started inside the cut's first frame, so it now starts at the cut and is 24 frames shorter in all.
        let mut title_pieces: Vec<(i64, i64)> = composition.layers.iter().filter(|layer| matches!(&layer.content, LayerContent::Text { text, .. } if text == "STUDIO")).map(|layer| (layer.in_frame, layer.out_frame)).collect();
        title_pieces.sort();
        assert_eq!(title_pieces.first().map(|piece| piece.0), Some(title.0.min(24)));
        assert_eq!(title_pieces.last().map(|piece| piece.1), Some(title.1 - 24));
    }

    #[test]
    fn removing_filler_words_cuts_each_one() {
        let (mut project, _, captions) = talk();
        assert_eq!(cut_filler_words(&mut project, ids::MAIN_COMP, captions), 1);
        assert_eq!(caption_words(&project, captions).iter().map(|word| word.0.as_str()).collect::<Vec<_>>(), vec!["So", "this", "works"]);
        assert_eq!(cut_filler_words(&mut project, ids::MAIN_COMP, captions), 0, "nothing left to cut");
    }
}
