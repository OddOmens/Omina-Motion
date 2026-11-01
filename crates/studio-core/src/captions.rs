//! Caption layers: timed words shown a phrase at a time, all in one style. The words keep
//! their own timing; the style (font, colors, background) and the animation belong to the
//! whole layer, so changing them restyles every caption at once.
//!
//! Word times are frames from the layer's start frame, so moving or trimming the layer in the
//! timeline moves its captions with it.

use crate::model::{Rgba, TextStyle};

/// One timed word.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CaptionWord {
    pub text: String,
    /// First frame the word is spoken, from the layer's start frame.
    pub start: i64,
    /// The frame after it ends.
    pub end: i64,
}

/// How captions come and go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CaptionAnimation {
    /// Each phrase appears whole, and goes when the next one comes.
    Appear,
    /// Words appear as they are spoken.
    WordByWord,
    /// The whole phrase shows, with the word being spoken in the highlight color.
    #[default]
    Highlight,
    /// Like Highlight, and the phrase pops up a little on every word.
    Pop,
    /// Phrases fade in and out.
    Fade,
}

impl CaptionAnimation {
    pub const ALL: [Self; 5] = [Self::Appear, Self::WordByWord, Self::Highlight, Self::Pop, Self::Fade];

    pub fn name(self) -> &'static str {
        match self {
            Self::Appear => "Appear",
            Self::WordByWord => "Word by Word",
            Self::Highlight => "Highlight Word",
            Self::Pop => "Pop",
            Self::Fade => "Fade",
        }
    }
}

/// How every caption in a layer looks.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CaptionStyle {
    pub text: TextStyle,
    /// The color of the word being spoken (Highlight and Pop).
    pub highlight: Rgba,
    pub animation: CaptionAnimation,
    /// The most words shown at once (1 to `MAX_WORDS_PER_PHRASE`).
    pub words_per_phrase: u32,
    pub uppercase: bool,
    /// A box behind the text, as social captions often have.
    pub background: Option<Rgba>,
}

/// The most words a phrase can hold.
pub const MAX_WORDS_PER_PHRASE: u32 = 12;
/// A pause this long (in seconds) between words starts a new phrase.
const PHRASE_PAUSE: f64 = 0.6;
/// How long Pop's bounce and Fade's fades last, in seconds.
const ANIMATION_TIME: f64 = 0.15;

impl Default for CaptionStyle {
    fn default() -> Self {
        let text = TextStyle { font_size: 72.0, line_height: 86.0, font_weight: 800, stroke: Some((Rgba::hex(0x000000), 6.0)), ..TextStyle::default() };
        Self { text, highlight: Rgba::hex(0xd4f25c), animation: CaptionAnimation::default(), words_per_phrase: 3, uppercase: true, background: None }
    }
}

/// What a caption layer shows at one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionFrame {
    /// The words shown, in order, each with whether it is the one being spoken.
    pub words: Vec<(String, bool)>,
    /// Opacity from the animation (0–1).
    pub opacity: f32,
    /// Size from the animation (1 is normal).
    pub scale: f32,
}

impl CaptionFrame {
    /// The words as one line of text.
    pub fn text(&self) -> String {
        self.words.iter().map(|(word, _)| word.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// Groups words into phrases: at most `per_phrase` words, breaking at pauses and after words
/// that end a sentence. Returns ranges into `words`.
pub fn phrases(words: &[CaptionWord], per_phrase: u32, fps: f64) -> Vec<std::ops::Range<usize>> {
    let per_phrase = per_phrase.clamp(1, MAX_WORDS_PER_PHRASE) as usize;
    let pause = (PHRASE_PAUSE * fps).round() as i64;
    let mut ranges = Vec::new();
    let mut start = 0;
    for index in 0..words.len() {
        let ends_sentence = words[index].text.ends_with(['.', '?', '!']);
        let gap_after = words.get(index + 1).is_some_and(|next| next.start - words[index].end >= pause);
        if index + 1 - start >= per_phrase || ends_sentence || gap_after || index + 1 == words.len() {
            ranges.push(start..index + 1);
            start = index + 1;
        }
    }
    ranges
}

/// What a caption layer shows at `frame` (from its start frame), or `None` between phrases.
pub fn frame_at(words: &[CaptionWord], style: &CaptionStyle, frame: i64, fps: f64) -> Option<CaptionFrame> {
    let groups = phrases(words, style.words_per_phrase, fps);
    // A phrase stays up from its first word until the next phrase starts, or a pause after it.
    let hold = (PHRASE_PAUSE * fps).round() as i64;
    let (_, range) = groups.iter().enumerate().find(|(index, range)| {
        let start = words[range.start].start;
        let next = groups.get(index + 1).map_or(i64::MAX, |next| words[next.start].start);
        let end = (words[range.end - 1].end + hold).min(next);
        (start..end).contains(&frame)
    })?;
    let spoken = |word: &CaptionWord| (word.start..word.end).contains(&frame);
    let shown = |text: &str| if style.uppercase { text.to_uppercase() } else { text.to_owned() };
    let words_shown: Vec<(String, bool)> = words[range.clone()]
        .iter()
        .filter(|word| style.animation != CaptionAnimation::WordByWord || word.start <= frame)
        .map(|word| (shown(&word.text), matches!(style.animation, CaptionAnimation::Highlight | CaptionAnimation::Pop) && spoken(word)))
        .collect();
    let seconds = |frames: i64| frames as f64 / fps.max(1.0);
    let (first, last_end) = (words[range.start].start, words[range.end - 1].end);
    let opacity = match style.animation {
        CaptionAnimation::Fade => {
            let fade_in = seconds(frame - first) / ANIMATION_TIME;
            let fade_out = seconds(last_end + hold - frame) / ANIMATION_TIME;
            fade_in.min(fade_out).clamp(0.0, 1.0) as f32
        }
        _ => 1.0,
    };
    let scale = match style.animation {
        // Each word start gives a little bounce that settles.
        CaptionAnimation::Pop => {
            let latest = words[range.clone()].iter().filter(|word| word.start <= frame).map(|word| word.start).max().unwrap_or(first);
            let t = (seconds(frame - latest) / ANIMATION_TIME).clamp(0.0, 1.0);
            1.0 + 0.12 * (1.0 - t * t) as f32
        }
        _ => 1.0,
    };
    Some(CaptionFrame { words: words_shown, opacity, scale })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, start: i64, end: i64) -> CaptionWord {
        CaptionWord { text: text.into(), start, end }
    }

    #[test]
    fn words_group_into_short_phrases_that_break_at_sentences_and_pauses() {
        let words = [word("Hi", 0, 10), word("there.", 10, 20), word("This", 22, 30), word("is", 30, 34), word("Studio", 34, 50), word("again", 100, 110)];
        // At 24 fps a 0.6 s pause is about 14 frames.
        assert_eq!(phrases(&words, 3, 24.0), vec![0..2, 2..5, 5..6]);
        assert_eq!(phrases(&words, 1, 24.0).len(), 6);
    }

    #[test]
    fn the_spoken_word_is_highlighted_and_word_by_word_builds_up() {
        let words = [word("make", 0, 10), word("it", 10, 20), word("pop", 20, 30)];
        let style = CaptionStyle { uppercase: false, ..CaptionStyle::default() };
        let shown = frame_at(&words, &style, 12, 24.0).unwrap();
        assert_eq!(shown.words, vec![("make".into(), false), ("it".into(), true), ("pop".into(), false)]);
        let building = CaptionStyle { animation: CaptionAnimation::WordByWord, ..style.clone() };
        assert_eq!(frame_at(&words, &building, 12, 24.0).unwrap().text(), "make it");
        assert!(frame_at(&words, &style, 500, 24.0).is_none(), "nothing long after the last word");
    }

    #[test]
    fn pop_bounces_on_each_word_and_fade_eases_in() {
        let words = [word("one", 0, 24), word("two", 24, 48)];
        let pop = CaptionStyle { animation: CaptionAnimation::Pop, ..CaptionStyle::default() };
        assert!(frame_at(&words, &pop, 24, 24.0).unwrap().scale > 1.1, "a bounce as the word starts");
        assert_eq!(frame_at(&words, &pop, 40, 24.0).unwrap().scale, 1.0, "settled after");
        let fade = CaptionStyle { animation: CaptionAnimation::Fade, ..CaptionStyle::default() };
        assert!(frame_at(&words, &fade, 1, 24.0).unwrap().opacity < 0.5);
        assert_eq!(frame_at(&words, &fade, 20, 24.0).unwrap().opacity, 1.0);
    }
}
