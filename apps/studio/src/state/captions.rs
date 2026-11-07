//! Caption layers in the editor: making one from transcribed words, restyling it, and editing
//! its words.

use studio_core::captions::{CaptionStyle, CaptionWord, MAX_WORDS_PER_PHRASE};
use studio_core::model::{Id, LayerContent};
use studio_core::subtitles::Cue;

use super::EditorState;

/// A word as speech recognition hears it: its text, and when it starts and ends, in seconds
/// from the start of the sound that was transcribed.
pub struct HeardWord {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

impl EditorState {
    /// The sound a layer would be captioned from: its media file and the span of it the layer
    /// uses, in seconds from the file's start.
    pub fn caption_source(&self, layer: Id) -> Option<(std::path::PathBuf, f64, f64)> {
        let composition = self.composition();
        let layer = composition.layer(layer)?;
        let item = match layer.content {
            LayerContent::Footage { item } | LayerContent::Audio { item } if layer.has_audio || matches!(layer.content, LayerContent::Audio { .. }) => item,
            _ => return None,
        };
        let path = self.project.item(item)?.path.clone()?;
        let fps = composition.frame_rate.fps();
        let seconds = |frame: i64| (frame - layer.start_frame) as f64 / fps;
        Some((path, seconds(layer.in_frame), seconds(layer.out_frame)))
    }

    /// Adds a caption layer for `clip` from words heard in its sound (timed from the start of
    /// the span `caption_source` gave), spanning the clip, and selects it.
    pub fn add_captions(&mut self, clip: Id, heard: &[HeardWord]) -> Option<Id> {
        self.settle_drags();
        let composition = self.composition();
        let layer = composition.layer(clip)?;
        let fps = composition.frame_rate.fps();
        let (in_frame, out_frame, start_frame) = (layer.in_frame, layer.out_frame, layer.start_frame);
        // Words are timed from the layer's start frame; the transcribed span began at its in point.
        let offset = in_frame - start_frame;
        let frame = |seconds: f64| offset + (seconds * fps).round() as i64;
        let words: Vec<CaptionWord> = heard
            .iter()
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| CaptionWord::new(word.text.trim(), frame(word.start), frame(word.end).max(frame(word.start) + 1)))
            .collect();
        let comp = self.active_comp;
        let id = studio_core::edit::add_caption_layer(&mut self.project, comp, words, CaptionStyle::default(), in_frame..out_frame, start_frame)?;
        self.select_layer(id, false);
        Some(id)
    }

    /// Adds a caption layer showing subtitle `cues` (composition frames, as read from an .srt
    /// or .vtt file), named `name`, spanning the cues, and selects it. It has the default caption
    /// look, showing enough words at once for each cue to appear whole.
    pub fn add_subtitle_captions(&mut self, name: &str, cues: &[Cue]) -> Option<Id> {
        self.settle_drags();
        let (first, last) = (cues.first()?.start, cues.iter().map(|cue| cue.end).max()?);
        let words = studio_core::subtitles::caption_words(cues);
        let style = CaptionStyle { words_per_phrase: studio_core::subtitles::words_per_phrase(cues), ..CaptionStyle::default() };
        let comp = self.active_comp;
        // Cue times are composition frames, so the words are timed from frame 0.
        let id = studio_core::edit::add_caption_layer(&mut self.project, comp, words, style, first..last, 0)?;
        if let Some(layer) = self.composition_mut().layer_mut(id) {
            layer.name = name.to_owned();
        }
        self.select_layer(id, false);
        Some(id)
    }

    /// A caption layer's captions as subtitle cues, as the composition shows them (see
    /// `subtitles::layer_cues`). Empty for other layers.
    pub fn subtitle_cues(&self, layer: Id) -> Vec<Cue> {
        let composition = self.composition();
        composition.layer(layer).map_or_else(Vec::new, |layer| studio_core::subtitles::layer_cues(layer, composition.frame_rate))
    }

    pub fn caption_style(&self, layer: Id) -> Option<&CaptionStyle> {
        match &self.composition().layer(layer)?.content {
            LayerContent::Captions { style, .. } => Some(style),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn caption_words(&self, layer: Id) -> Option<&[CaptionWord]> {
        match &self.composition().layer(layer)?.content {
            LayerContent::Captions { words, .. } => Some(words),
            _ => None,
        }
    }

    /// Restyles every caption in a layer.
    pub fn edit_captions(&mut self, layer: Id, edit: impl FnOnce(&mut CaptionStyle)) {
        if let Some(LayerContent::Captions { style, .. }) = self.composition_mut().layer_mut(layer).map(|layer| &mut layer.content) {
            edit(style);
            style.words_per_phrase = style.words_per_phrase.clamp(1, MAX_WORDS_PER_PHRASE);
        }
    }

    /// When phrase `range` of caption layer `layer` starts (`start`) or ends, in composition
    /// frames, with the layer's sync applied: when it shows.
    pub fn caption_phrase_frame(&self, layer: Id, range: std::ops::Range<usize>, start: bool) -> Option<i64> {
        let layer = self.composition().layer(layer)?;
        let LayerContent::Captions { words, style } = &layer.content else { return None };
        let at = if start { words.get(range.start)?.start } else { words.get(range.end.checked_sub(1)?)?.end };
        Some(layer.start_frame + style.offset + at)
    }

    /// Moves when phrase `range` of caption layer `layer` starts (`start`) or ends to
    /// composition frame `frame`; its words stretch or squeeze to fit (see
    /// `captions::retime_phrase`).
    pub fn retime_caption_phrase(&mut self, layer: Id, range: std::ops::Range<usize>, start: bool, frame: i64) {
        let Some(layer) = self.composition_mut().layer_mut(layer) else { return };
        let start_frame = layer.start_frame;
        if let LayerContent::Captions { words, style } = &mut layer.content {
            studio_core::captions::retime_phrase(words, range, start, frame - start_frame - style.offset);
        }
    }

    /// Replaces words `range` with `text`, sharing their time among the new words by length
    /// (see `captions::spread_words`). An empty text deletes them. When the last word replaced
    /// ended a subtitle cue, the new last word does.
    pub fn set_caption_text(&mut self, layer: Id, range: std::ops::Range<usize>, text: &str) {
        let Some(LayerContent::Captions { words, .. }) = self.composition_mut().layer_mut(layer).map(|layer| &mut layer.content) else { return };
        if range.start >= range.end || range.end > words.len() {
            return;
        }
        let (start, end, ends_cue) = (words[range.start].start, words[range.end - 1].end, words[range.end - 1].ends_cue);
        let mut replacement = studio_core::captions::spread_words(text, start, end);
        if let Some(last) = replacement.last_mut() {
            last.ends_cue = ends_cue;
        }
        words.splice(range, replacement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard(words: &[(&str, f64, f64)]) -> Vec<HeardWord> {
        words.iter().map(|&(text, start, end)| HeardWord { text: text.into(), start, end }).collect()
    }

    #[test]
    fn captions_are_timed_to_their_clip() {
        let mut state = EditorState::new();
        let clip = state.composition().layers[0].id;
        let (in_frame, start_frame) = { let layer = state.composition().layer(clip).unwrap(); (layer.in_frame, layer.start_frame) };
        let captions = state.add_captions(clip, &heard(&[("Hello", 0.0, 0.5), (" world", 0.5, 1.0)])).unwrap();
        let layer = state.composition().layer(captions).unwrap();
        assert_eq!((layer.in_frame, layer.start_frame), (in_frame, start_frame));
        let words = state.caption_words(captions).unwrap();
        assert_eq!(words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hello", "world"]);
        assert_eq!(words[0].start, in_frame - start_frame, "the first word starts where the clip does");
        assert_eq!(words[1].start, in_frame - start_frame + 12, "half a second later at 24 fps");
        assert_eq!(state.selected_layers, vec![captions]);
    }

    #[test]
    fn editing_a_phrase_keeps_its_time() {
        let mut state = EditorState::new();
        let clip = state.composition().layers[0].id;
        let captions = state.add_captions(clip, &heard(&[("Helo", 0.0, 0.5), ("word", 0.5, 1.0)])).unwrap();
        let (start, end) = { let words = state.caption_words(captions).unwrap(); (words[0].start, words[1].end) };
        state.set_caption_text(captions, 0..2, "Hello brave world");
        let words = state.caption_words(captions).unwrap();
        assert_eq!(words.len(), 3);
        assert_eq!((words[0].start, words[2].end), (start, end));
        state.edit_captions(captions, |style| style.words_per_phrase = 99);
        assert_eq!(state.caption_style(captions).unwrap().words_per_phrase, MAX_WORDS_PER_PHRASE);
    }

    #[test]
    fn subtitles_become_a_caption_layer_that_exports_the_same_cues() {
        let mut state = EditorState::new();
        let cues = vec![Cue { start: 30, end: 60, text: "Hello there.".into() }, Cue { start: 60, end: 75, text: "Nice to meet you".into() }];
        let layer = state.add_subtitle_captions("Film", &cues).unwrap();
        let found = state.composition().layer(layer).unwrap();
        assert_eq!((found.name.as_str(), found.in_frame, found.out_frame, found.start_frame), ("Film", 30, 75, 0));
        assert_eq!(state.selected_layers, vec![layer]);
        assert_eq!(state.subtitle_cues(layer), cues);
        // Fixing a cue's text keeps it a cue of its own.
        state.set_caption_text(layer, 0..2, "Hi");
        assert_eq!(state.subtitle_cues(layer)[0], Cue { start: 30, end: 60, text: "Hi".into() });
        assert!(state.add_subtitle_captions("Empty", &[]).is_none());
    }
}
