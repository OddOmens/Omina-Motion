//! Understanding everyday housekeeping for layers: "hide the logo", "duplicate it", "rename it
//! to Hero", "bring it to the front", "put it in the top right corner", "make it red", "a bit
//! bigger", "flip it", "lock all text layers". It sits beside `intent`, which reads timing and
//! motion, and shares its way of saying which layers are meant.
//!
//! Like `intent`, it turns a message into a `Request` without knowing the app's project, and
//! each app carries the steps out its own way (or says it can't). It reads the words with
//! fixed rules; nothing here is AI.
//!
//! Several of these verbs are also said about other things ("hide the rulers", "delete the
//! keyframes"), so a request says whether it is `sure` the words name a layer. When it isn't,
//! the app should look for a layer of that name and, finding none, treat the message as not
//! being about layers at all.

use crate::intent::{self, Target};

/// Where along the frame something sits: its left or top, the middle, or its right or bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Along {
    Start,
    Middle,
    End,
}

/// A move in the stacking order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stack {
    /// In front of everything.
    Front,
    /// Behind everything.
    Back,
    /// One place forward.
    Forward,
    /// One place back.
    Backward,
}

/// One thing asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Select the layers meant, and nothing more.
    Select,
    Hide,
    Show,
    Lock,
    Unlock,
    Duplicate,
    Delete,
    /// Give it this name, as it was typed.
    Rename(String),
    Stack(Stack),
    /// Put it at this place in the frame. A side left `None` is kept as it is ("center it
    /// horizontally" moves it only across).
    Place { across: Option<Along>, down: Option<Along> },
    /// Mirror it, left to right or (with `horizontal` false) top to bottom.
    Flip { horizontal: bool },
    /// Make it this color: red, green and blue from 0 to 255, with the color as it was said.
    Color { rgb: [u8; 3], name: String },
    /// Make it this many percent of the size it is now ("bigger" is 120, "half the size" 50).
    Resize(i64),
}

/// A message read as housekeeping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub target: Target,
    pub steps: Vec<Step>,
    /// Whether the words certainly name a layer. When false, `target` is a `Target::Named`
    /// made of words that may be a layer's name or something else entirely ("hide the
    /// rulers"): only a layer that exists by that name makes this a request about layers.
    pub sure: bool,
}

/// The longest name a layer can be renamed to, in characters.
const NAME_LIMIT: usize = 60;
/// How much "bigger" and "smaller" change the size, in percent: a little, usually, a lot.
const RESIZE_BY: (i64, i64, i64) = (10, 20, 50);

/// The colors that can be asked for by name.
const COLORS: &[(&str, [u8; 3])] = &[
    ("red", [229, 57, 53]),
    ("orange", [251, 140, 0]),
    ("yellow", [253, 216, 53]),
    ("green", [67, 160, 71]),
    ("blue", [30, 136, 229]),
    ("purple", [142, 36, 170]),
    ("violet", [126, 87, 194]),
    ("pink", [236, 64, 122]),
    ("magenta", [216, 27, 96]),
    ("cyan", [0, 188, 212]),
    ("teal", [0, 137, 123]),
    ("turquoise", [38, 198, 218]),
    ("navy", [26, 35, 126]),
    ("indigo", [57, 73, 171]),
    ("brown", [121, 85, 72]),
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("gray", [158, 158, 158]),
    ("grey", [158, 158, 158]),
    ("gold", [255, 193, 7]),
    ("silver", [192, 192, 192]),
    ("beige", [245, 245, 220]),
    ("cream", [255, 253, 231]),
    ("maroon", [136, 14, 79]),
    ("olive", [130, 119, 23]),
    ("lime", [192, 202, 51]),
    ("mint", [128, 203, 196]),
    ("coral", [255, 112, 97]),
    ("salmon", [250, 128, 114]),
    ("lavender", [179, 157, 219]),
    ("peach", [255, 204, 170]),
    ("tan", [210, 180, 140]),
];

/// Verbs that can change a color when a color follows: "*make* it red", "*fill* it with blue".
const COLOR_VERBS: &[&str] = &["make", "turn", "color", "colour", "recolor", "recolour", "paint", "fill", "tint", "change", "set"];
/// Verbs that can put a layer somewhere in the frame.
const PLACE_VERBS: &[&str] = &["put", "place", "position", "align", "snap", "move", "dock"];
/// Words that go with a place without saying where: "the top right *corner of the page*".
const PLACE_WORDS: &[&str] = &["corner", "edge", "side", "hand", "page", "frame", "canvas", "screen", "artboard", "horizontally", "vertically"];
/// Words that say nothing about which layer is meant, beyond those `intent` already skips.
const FILLER: &[&str] = &[
    "a", "an", "my", "please", "pls", "can", "could", "would", "will", "you", "and", "then", "to", "of", "on", "in", "at", "with", "just", "also", "now", "too", "make", "set", "turn", "change", "its", "it's", "be", "so", "color", "colour", "size", "as", "by", "up", "down", "into", "is", "them", "ones", "clips", "elements", "objects", "object", "element", "layers", "layer", "the", "it", "this", "that", "these", "those", "selected", "selection", "current", "all", "every", "each", "everything", "bit", "little",
    "i", "want", "need", "let's", "lets", "again", "for", "me", "from",
];
/// Words for a kind of layer, which `intent` reads as a group when they are one.
const KIND_WORDS: &[&str] = &["text", "texts", "image", "images", "picture", "pictures", "photo", "photos", "pic", "pics", "shape", "shapes", "vector", "vectors", "video", "videos", "footage", "audio", "sound", "sounds", "music"];

/// Where a pattern of plain letters starts in `text`, whatever its case. The position is
/// always between two characters, as the pattern's letters are single bytes.
fn find_plain(text: &str, pattern: &str) -> Option<usize> {
    text.as_bytes().windows(pattern.len()).position(|window| window.eq_ignore_ascii_case(pattern.as_bytes()))
}

/// A typed name made fit to keep: without the quotes and stops around it, and not too long.
fn tidy_name(typed: &str) -> Option<String> {
    let name = typed.trim().trim_end_matches(['.', '!']).trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '‘' | '’')).trim();
    let name: String = name.chars().take(NAME_LIMIT).collect();
    (!name.is_empty()).then(|| name.trim_end().to_owned())
}

/// A message asking for a rename, as the part before the new name and the new name: "rename
/// the title layer to Hero" gives "rename the title layer" and "Hero"; "call it Hero" gives
/// "call it" and "Hero".
fn rename_parts(message: &str) -> Option<(&str, String)> {
    if let Some(start) = find_plain(message, "rename") {
        let rest = &message[start..];
        // The first "to" ends the layer's part: a new name may have a "to" of its own
        // ("rename it to Back to Front").
        let (at, joiner) = [" to ", " as "].iter().filter_map(|joiner| find_plain(rest, joiner).map(|at| (at, joiner))).min_by_key(|(at, _)| *at)?;
        return Some((&message[..start + at], tidy_name(&rest[at + joiner.len()..])?));
    }
    for start in ["call it ", "call this ", "call that ", "name it ", "name this ", "name that "] {
        if let Some(at) = find_plain(message, start) {
            // Only at the start of the message or of a word: "recall it" is something else.
            let at_word_start = at == 0 || message[..at].ends_with(' ');
            if at_word_start {
                let end = at + start.len();
                return Some((&message[..end], tidy_name(&message[end..])?));
            }
        }
    }
    None
}

/// A color written as "#rgb" or "#rrggbb" in `message`, with how it was written.
fn hex_color(message: &str) -> Option<([u8; 3], String)> {
    let start = message.find('#')?;
    let digits: String = message[start + 1..].chars().take_while(char::is_ascii_hexdigit).collect();
    let value = |text: &str| u8::from_str_radix(text, 16).ok();
    let rgb = match digits.len() {
        3 => {
            let twice = |index: usize| value(&digits[index..=index].repeat(2));
            [twice(0)?, twice(1)?, twice(2)?]
        }
        6 => [value(&digits[0..2])?, value(&digits[2..4])?, value(&digits[4..6])?],
        _ => return None,
    };
    Some((rgb, format!("#{}", digits.to_lowercase())))
}

/// A named color lightened or darkened as said: "dark blue", "light green".
fn shaded(rgb: [u8; 3], shade: Option<&str>) -> [u8; 3] {
    match shade {
        Some("dark" | "deep") => rgb.map(|channel| (f32::from(channel) * 0.6).round() as u8),
        Some("light" | "pale" | "pastel") => rgb.map(|channel| ((f32::from(channel) + 255.0) / 2.0).round() as u8),
        _ => rgb,
    }
}

struct Reader<'a> {
    tokens: &'a [String],
    used: Vec<bool>,
    steps: Vec<Step>,
    /// Whether a Show step came from the word "show" itself, which also asks for lists.
    show_word: bool,
}

impl<'a> Reader<'a> {
    /// The word at `at`, borrowed from the message's words rather than from the reader, so it
    /// can be held while the reader marks words as taken.
    fn word(&self, at: usize) -> Option<&'a str> {
        self.tokens.get(at).map(String::as_str)
    }

    fn is(&self, at: usize, words: &[&str]) -> bool {
        self.word(at).is_some_and(|word| words.contains(&word))
    }

    fn mark(&mut self, at: usize) {
        if let Some(used) = self.used.get_mut(at) {
            *used = true;
        }
    }

    fn take(&mut self, at: usize, step: Step) {
        self.mark(at);
        self.steps.push(step);
    }

    /// Whether any of `words` is said anywhere.
    fn says(&self, words: &[&str]) -> bool {
        self.tokens.iter().any(|token| words.contains(&token.as_str()))
    }

    /// How much "bigger" or "smaller" is meant, from the words around it.
    fn resize_by(&self) -> i64 {
        if self.says(&["little", "bit", "slightly", "touch", "tad", "tiny"]) {
            RESIZE_BY.0
        } else if self.says(&["lot", "much", "way", "far", "really", "loads"]) {
            RESIZE_BY.2
        } else {
            RESIZE_BY.1
        }
    }

    /// The simple verbs: one word, one step.
    fn simple(&mut self, at: usize) -> bool {
        let step = match self.word(at) {
            Some("select" | "highlight") => Step::Select,
            Some("hide") => Step::Hide,
            Some("unhide" | "reveal") => Step::Show,
            Some("show") => {
                self.show_word = true;
                Step::Show
            }
            Some("invisible") => Step::Hide,
            // "50% visible" is an opacity, which `intent` reads.
            Some("visible") if !self.is(at.wrapping_sub(1), &["%", "percent"]) && self.says(&["make", "set", "turn"]) => Step::Show,
            Some("lock") => Step::Lock,
            Some("unlock") => Step::Unlock,
            Some("duplicate" | "clone") => Step::Duplicate,
            Some("delete" | "erase" | "trash" | "remove") => Step::Delete,
            Some("copy") if at >= 2 && self.is(at - 1, &["a", "another"]) && self.is(at - 2, &["make", "create"]) => Step::Duplicate,
            Some("rid") if at >= 1 && self.is(at - 1, &["get"]) => {
                self.mark(at - 1);
                Step::Delete
            }
            Some("flip" | "mirror") => {
                let vertical = (at + 1..(at + 5).min(self.tokens.len())).any(|index| self.is(index, &["vertically", "vertical", "upside"]));
                for index in at + 1..(at + 6).min(self.tokens.len()) {
                    if self.is(index, &["horizontally", "horizontal", "vertically", "vertical", "upside", "sideways"]) {
                        self.mark(index);
                    }
                }
                Step::Flip { horizontal: !vertical }
            }
            _ => return false,
        };
        self.take(at, step);
        true
    }

    /// "Bring it to the front", "send to back", "move it forward", "put it behind everything".
    fn stack(&mut self, at: usize) -> bool {
        if !self.is(at, &["bring", "send", "move", "put", "push", "pull"]) {
            return false;
        }
        for look in at + 1..(at + 8).min(self.tokens.len()) {
            let stack = match self.word(look) {
                Some("front") => Stack::Front,
                Some("back") => Stack::Back,
                Some("forward" | "forwards") => Stack::Forward,
                Some("backward" | "backwards") => Stack::Backward,
                Some("behind") => Stack::Back,
                Some("top") if self.is(look - 1, &["on"]) => Stack::Front,
                _ => continue,
            };
            for index in at..=look {
                // The words between are small ones ("it to the"), but a name may be among
                // them: only the verb and the place are taken.
                if index == at || index == look || self.is(index, &["to", "in", "on"]) {
                    self.mark(index);
                }
            }
            // "Behind everything", "in front of everything": the "everything" is part of the
            // place, not the layers meant.
            for index in look + 1..(look + 3).min(self.tokens.len()) {
                if self.is(index, &["everything", "all", "of", "else", "rest", "others"]) {
                    self.mark(index);
                }
            }
            self.steps.push(Step::Stack(stack));
            return true;
        }
        false
    }

    /// "Center it", "put it in the top right corner", "align left", "move it to the bottom".
    fn place(&mut self, at: usize) -> bool {
        let centering = self.is(at, &["center", "centre"]);
        if !centering && !self.is(at, PLACE_VERBS) {
            return false;
        }
        let (mut across, mut down, mut middle) = (None, None, centering);
        let mut found = Vec::new();
        for look in at + 1..(at + 9).min(self.tokens.len()) {
            match self.word(look) {
                Some("left") => across = Some(Along::Start),
                Some("right") => across = Some(Along::End),
                Some("top" | "upper") => down = Some(Along::Start),
                Some("bottom" | "lower") => down = Some(Along::End),
                Some("middle" | "center" | "centre" | "centered" | "centred") => middle = true,
                Some("horizontally") if middle => across = across.or(Some(Along::Middle)),
                Some("vertically") if middle => down = down.or(Some(Along::Middle)),
                // A number means a distance or a time, which other readers handle.
                Some(word) if word.chars().next().is_some_and(|first| first.is_ascii_digit()) => return false,
                _ => continue,
            }
            found.push(look);
        }
        // "Move" alone is too common a word: it needs "to" before the place.
        let moved_to = !self.is(at, &["move"]) || found.first().is_some_and(|first| (at + 1..*first).any(|index| self.is(index, &["to", "into"])));
        if (found.is_empty() && !centering) || !moved_to {
            return false;
        }
        // "Center" with no side said is the middle both ways; with one side said ("top
        // center") it is the middle of the other.
        if middle {
            let only_one_way = found.iter().any(|index| self.is(*index, &["horizontally", "vertically"]));
            if !only_one_way {
                across = across.or(Some(Along::Middle));
                down = down.or(Some(Along::Middle));
            }
        }
        self.mark(at);
        for index in found {
            self.mark(index);
        }
        for index in at + 1..(at + 10).min(self.tokens.len()) {
            if self.is(index, PLACE_WORDS) {
                self.mark(index);
            }
        }
        self.steps.push(Step::Place { across, down });
        true
    }

    /// "Make it red", "fill it with dark blue", "change the color to teal".
    fn color(&mut self, at: usize, hex: &Option<([u8; 3], String)>) -> bool {
        if !self.is(at, COLOR_VERBS) || self.steps.iter().any(|step| matches!(step, Step::Color { .. })) {
            return false;
        }
        // "Black and white" is a look for a photo, not a color for a layer.
        if self.tokens.windows(3).any(|words| words[0] == "black" && matches!(words[1].as_str(), "and" | "&" | "n") && words[2] == "white") {
            return false;
        }
        for look in at + 1..(at + 8).min(self.tokens.len()) {
            let Some(&(name, rgb)) = self.word(look).and_then(|word| COLORS.iter().find(|(name, _)| *name == word)) else { continue };
            let shade = look.checked_sub(1).and_then(|before| self.word(before)).filter(|word| matches!(*word, "dark" | "deep" | "light" | "pale" | "pastel"));
            let said = match shade {
                Some(shade) => {
                    self.mark(look - 1);
                    format!("{shade} {name}")
                }
                None => name.to_owned(),
            };
            self.mark(at);
            self.mark(look);
            self.steps.push(Step::Color { rgb: shaded(rgb, shade), name: said });
            return true;
        }
        if let Some((rgb, written)) = hex {
            self.mark(at);
            // The hex digits may have been read as one or more words; whatever they are, they
            // are not a layer's name.
            let digits = written.trim_start_matches('#');
            for index in 0..self.tokens.len() {
                if digits.contains(self.tokens[index].as_str()) && self.tokens[index].chars().all(|character| character.is_ascii_hexdigit()) {
                    self.mark(index);
                }
            }
            self.steps.push(Step::Color { rgb: *rgb, name: written.clone() });
            return true;
        }
        false
    }

    /// "Bigger", "a bit smaller", "shrink it", "double the size", "half as big".
    fn resize(&mut self, at: usize) -> bool {
        // "20% bigger" is an exact size, which `intent` reads.
        if at >= 1 && self.is(at - 1, &["%", "percent"]) {
            return false;
        }
        let by = self.resize_by();
        let percent = match self.word(at) {
            Some("bigger" | "larger" | "enlarge" | "grow") => 100 + by,
            Some("smaller" | "shrink") => 100 - by,
            Some("double") if self.says(&["size", "it", "big", "large"]) => 200,
            Some("twice") if self.says(&["size", "big", "large"]) => 200,
            Some("halve") => 50,
            Some("half") if self.is(at + 1, &["the", "its", "as"]) && self.says(&["size", "big", "large"]) => 50,
            _ => return false,
        };
        self.mark(at);
        for index in 0..self.tokens.len() {
            if self.is(index, &["little", "bit", "slightly", "touch", "tad", "tiny", "lot", "much", "way", "far", "really", "loads", "big", "large", "twice", "double", "half"]) {
                self.mark(index);
            }
        }
        self.steps.push(Step::Resize(percent));
        true
    }
}

/// Reads `message` as housekeeping for layers. `None` when it isn't: a question, or nothing
/// in it could be read with confidence.
pub fn parse(message: &str) -> Option<Request> {
    if intent::asks(message) {
        return None;
    }
    // A new name is taken off first, as typed, so its words aren't read as anything else.
    let (head, new_name) = match rename_parts(message) {
        Some((head, name)) => (head, Some(name)),
        None => (message, None),
    };
    let tokens = intent::tokenize(head);
    let hex = hex_color(head);
    let mut reader = Reader { tokens: &tokens, used: vec![false; tokens.len()], steps: Vec::new(), show_word: false };
    for at in 0..tokens.len() {
        if reader.used[at] {
            continue;
        }
        // "Rename", "call" and "name" only introduce the new name.
        if new_name.is_some() && reader.is(at, &["rename", "call", "name"]) {
            reader.mark(at);
            continue;
        }
        let _ = reader.simple(at) || reader.stack(at) || reader.place(at) || reader.color(at, &hex) || reader.resize(at);
    }
    let Reader { used, mut steps, show_word, .. } = reader;
    if let Some(name) = new_name {
        steps.push(Step::Rename(name));
    }
    if steps.is_empty() {
        return None;
    }
    // Which layers: as `intent` reads them, from the words the steps didn't take.
    let unused: Vec<String> = tokens.iter().zip(&used).filter(|(_, used)| !**used).map(|(token, _)| token.clone()).collect();
    let target = intent::target(head, &unused);
    let spare: Vec<&str> = unused.iter().map(String::as_str).filter(|word| !FILLER.contains(word) && !KIND_WORDS.contains(word) && !word.chars().all(|character| character.is_ascii_digit())).collect();
    let (target, sure) = match target {
        // A layer was named outright. Any other word must be part of that name.
        Target::Named(name) => {
            let lower = name.to_lowercase();
            if !spare.iter().all(|word| lower.contains(word)) {
                return None;
            }
            (Target::Named(name), true)
        }
        // Words left over beside "everything" or a kind mean the message is about something
        // else ("delete all keyframes"), and guessing here could remove every layer.
        Target::Everything | Target::Kind(_) | Target::Number(_) if !spare.is_empty() => return None,
        Target::Unnamed | Target::Selection if !spare.is_empty() => {
            if spare.len() > 4 {
                return None;
            }
            (Target::Named(spare.join(" ")), false)
        }
        target => (target, true),
    };
    // "Show all layers" and "show the text layers" ask for a list, not for hidden layers back.
    if show_word && matches!(target, Target::Everything | Target::Kind(_) | Target::Unnamed) {
        return None;
    }
    // Selecting needs something said to select.
    if steps == [Step::Select] && matches!(target, Target::Unnamed | Target::Selection) {
        return None;
    }
    Some(Request { target, steps, sure })
}

/// The words housekeeping is written with, for putting a mistyped one right (see `spelling`).
pub fn vocabulary() -> Vec<String> {
    const WORDS: &[&str] = &[
        "select", "highlight", "hide", "unhide", "reveal", "invisible", "visible", "unlock", "duplicate", "clone", "delete", "erase", "trash", "remove", "rename", "flip", "mirror", "horizontally", "vertically", "bring", "front",
        "forward", "backward", "behind", "center", "centre", "middle", "bottom", "corner", "align", "position", "place", "bigger", "larger", "smaller", "shrink", "enlarge", "double", "twice", "color", "colour", "recolor", "paint",
    ];
    WORDS.iter().map(|word| (*word).to_owned()).chain(COLORS.iter().map(|(name, _)| (*name).to_owned())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Kind;

    fn read(message: &str) -> (Target, Vec<Step>, bool) {
        let request = parse(message).unwrap_or_else(|| panic!("“{message}” should read"));
        (request.target, request.steps, request.sure)
    }

    fn named(name: &str) -> Target {
        Target::Named(name.to_owned())
    }

    #[test]
    fn simple_verbs_read_with_the_layers_they_are_about() {
        assert_eq!(read("hide it"), (Target::Selection, vec![Step::Hide], true));
        assert_eq!(read("Duplicate"), (Target::Unnamed, vec![Step::Duplicate], true));
        assert_eq!(read("please delete the selected layer"), (Target::Selection, vec![Step::Delete], true));
        assert_eq!(read("lock all text layers"), (Target::Kind(Kind::Text), vec![Step::Lock], true));
        assert_eq!(read("unhide everything"), (Target::Everything, vec![Step::Show], true));
        assert_eq!(read("select every image"), (Target::Kind(Kind::Picture), vec![Step::Select], true));
        assert_eq!(read("hide layer 2"), (Target::Number(2), vec![Step::Hide], true));
        assert_eq!(read("hide the logo layer"), (named("logo"), vec![Step::Hide], true));
        assert_eq!(read("delete “Big Sale”"), (named("Big Sale"), vec![Step::Delete], true));
        assert_eq!(read("make a copy of it and flip it"), (Target::Selection, vec![Step::Duplicate, Step::Flip { horizontal: true }], true));
        assert_eq!(read("flip it upside down"), (Target::Selection, vec![Step::Flip { horizontal: false }], true));
        assert_eq!(read("show it"), (Target::Selection, vec![Step::Show], true));
    }

    #[test]
    fn a_name_without_quotes_is_a_guess_the_app_checks() {
        assert_eq!(read("hide the logo"), (named("logo"), vec![Step::Hide], false));
        assert_eq!(read("hide the rulers"), (named("rulers"), vec![Step::Hide], false));
        assert_eq!(read("delete the fade"), (named("fade"), vec![Step::Delete], false));
        assert_eq!(read("show the guides"), (named("guides"), vec![Step::Show], false));
    }

    #[test]
    fn renames_keep_the_name_as_typed() {
        assert_eq!(read("rename it to Hero Shot"), (Target::Selection, vec![Step::Rename("Hero Shot".to_owned())], true));
        assert_eq!(read("Rename the title layer to “Go To Market”."), (named("title"), vec![Step::Rename("Go To Market".to_owned())], true));
        assert_eq!(read("call it Backdrop"), (Target::Selection, vec![Step::Rename("Backdrop".to_owned())], true));
        assert_eq!(read("rename it to Back to Front"), (Target::Selection, vec![Step::Rename("Back to Front".to_owned())], true));
        assert_eq!(read("rename “Big Sale” to Hero"), (named("Big Sale"), vec![Step::Rename("Hero".to_owned())], true));
        assert_eq!(parse("rename it"), None);
    }

    #[test]
    fn stacking_and_places_read() {
        assert_eq!(read("bring it to the front"), (Target::Selection, vec![Step::Stack(Stack::Front)], true));
        assert_eq!(read("send the logo layer to the back"), (named("logo"), vec![Step::Stack(Stack::Back)], true));
        assert_eq!(read("move it forward"), (Target::Selection, vec![Step::Stack(Stack::Forward)], true));
        assert_eq!(read("put it behind everything"), (Target::Selection, vec![Step::Stack(Stack::Back)], true));
        assert_eq!(read("center it"), (Target::Selection, vec![Step::Place { across: Some(Along::Middle), down: Some(Along::Middle) }], true));
        assert_eq!(read("centre it horizontally"), (Target::Selection, vec![Step::Place { across: Some(Along::Middle), down: None }], true));
        assert_eq!(read("put it in the top right corner"), (Target::Selection, vec![Step::Place { across: Some(Along::End), down: Some(Along::Start) }], true));
        assert_eq!(read("move it to the bottom center"), (Target::Selection, vec![Step::Place { across: Some(Along::Middle), down: Some(Along::End) }], true));
        assert_eq!(read("align left"), (Target::Unnamed, vec![Step::Place { across: Some(Along::Start), down: None }], true));
        assert_eq!(read("put the title layer at the top"), (named("title"), vec![Step::Place { across: None, down: Some(Along::Start) }], true));
    }

    #[test]
    fn colors_and_sizes_read() {
        assert_eq!(read("make it red"), (Target::Selection, vec![Step::Color { rgb: [229, 57, 53], name: "red".to_owned() }], true));
        assert_eq!(read("fill it with dark blue"), (Target::Selection, vec![Step::Color { rgb: [18, 82, 137], name: "dark blue".to_owned() }], true));
        assert_eq!(read("change the color to #FF8800"), (Target::Unnamed, vec![Step::Color { rgb: [255, 136, 0], name: "#ff8800".to_owned() }], true));
        assert_eq!(read("make the text white"), (Target::Unnamed, vec![Step::Color { rgb: [255, 255, 255], name: "white".to_owned() }], true));
        assert_eq!(read("make it bigger"), (Target::Selection, vec![Step::Resize(120)], true));
        assert_eq!(read("a bit smaller"), (Target::Unnamed, vec![Step::Resize(90)], true));
        assert_eq!(read("make it much bigger"), (Target::Selection, vec![Step::Resize(150)], true));
        assert_eq!(read("double the size"), (Target::Unnamed, vec![Step::Resize(200)], true));
        assert_eq!(read("make it half as big"), (Target::Selection, vec![Step::Resize(50)], true));
    }

    #[test]
    fn messages_about_other_things_are_left_alone() {
        for message in [
            "how do I hide a layer?",
            "delete all keyframes",
            "remove every text animation",
            "show all layers",
            "show the text layers",
            "select",
            "fade it in",
            "start at 5s",
            "move it 20 px left",
            "make it black and white",
            "make it pulse",
            "half a second later",
            "what is selected",
            "hello",
            "",
        ] {
            assert_eq!(parse(message), None, "{message}");
        }
    }
}
