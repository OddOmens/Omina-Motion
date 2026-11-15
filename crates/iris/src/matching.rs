//! Finding help by keyword, for Iris's "Ask" field. A question is matched
//! against a list of ready-written answers by the words they share; nothing is generated and
//! nothing is AI. Small words ("how", "do", "the") are ignored, word endings are trimmed so
//! "keyframes" finds "keyframe", and a few everyday words stand for the app's own ("smooth"
//! finds "ease", "hotkey" finds "shortcut"). A question that finds nothing is tried once more
//! with its mistyped words put right ("keyfarme" finds "keyframe"), and `closest` gives the
//! nearest topics to offer when even that finds nothing.

/// One ready-written answer, as the matching sees it.
pub struct Topic<'a> {
    /// The answer's heading. Words found here count the most.
    pub title: &'a str,
    /// The answer's text. Words found here count the least.
    pub body: &'a str,
    /// Extra words people might ask with that the heading and text don't use.
    pub keywords: &'a str,
}

/// How much a question word counts when it is found in a topic's heading, keywords or text.
const TITLE_POINTS: u32 = 4;
const KEYWORD_POINTS: u32 = 3;
const BODY_POINTS: u32 = 1;
/// A topic needs at least this many points to be offered, so one passing mention in an
/// answer's text isn't enough.
const LOWEST_SCORE: u32 = 3;

/// Words that say nothing about what is being asked.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "are", "can", "could", "do", "does", "for", "from", "have", "how", "i", "in", "is", "it", "its", "me", "my", "of", "on", "or", "please", "should", "that", "the", "them", "this", "to", "want", "what", "when",
    "where", "which", "why", "with", "would", "you", "your", "make", "get", "let", "just", "there", "be",
];

/// How many letters longer a question's word may be than the answer's word it starts with:
/// "brighter" finds "bright", but "background" doesn't find "back".
const LONGER_BY: usize = 3;

/// Everyday words and the word the app uses for the same thing.
const SAME_AS: &[(&str, &str)] = &[
    ("smooth", "ease"),
    ("smoother", "ease"),
    ("easing", "ease"),
    ("hotkey", "shortcut"),
    ("keybinding", "shortcut"),
    ("razor", "cut"),
    ("slice", "split"),
    ("erase", "delete"),
    ("remove", "delete"),
    ("photo", "picture"),
    ("image", "picture"),
    ("audio", "sound"),
    ("video", "footage"),
    ("movie", "footage"),
    ("render", "export"),
    ("animate", "keyframe"),
    ("animation", "keyframe"),
    ("preference", "setting"),
    ("vector", "svg"),
    ("box", "outline"),
    ("pic", "picture"),
    ("img", "picture"),
    ("photograph", "picture"),
    ("vid", "footage"),
    ("subtitle", "caption"),
    ("typeface", "font"),
    ("colour", "color"),
    ("centre", "center"),
    ("grey", "gray"),
    ("duplicate", "copy"),
    ("clone", "copy"),
    ("transparent", "opacity"),
    ("transparency", "opacity"),
    ("keybind", "shortcut"),
    ("config", "setting"),
    ("option", "setting"),
];

/// A word reduced to what is compared: lower case, without a plural or verb ending, and
/// swapped for the app's own word where there is one.
pub(crate) fn root(word: &str) -> String {
    let word = word.to_lowercase();
    let known = |word: &str| SAME_AS.iter().find(|(everyday, _)| *everyday == word).map(|(_, ours)| (*ours).to_owned());
    if let Some(ours) = known(&word) {
        return ours;
    }
    // Endings come off only when a real word is left ("easing" → "eas", "keys" → "key").
    let trimmed = ["ing", "es", "ed", "s"].iter().find_map(|ending| word.strip_suffix(ending).filter(|rest| rest.chars().count() >= 3)).unwrap_or(&word);
    known(trimmed).unwrap_or_else(|| trimmed.to_owned())
}

/// The words of a piece of text that matter, as roots.
pub(crate) fn roots(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty() && !SMALL_WORDS.contains(&word.to_lowercase().as_str())).map(root).collect()
}

/// Whether a question word and an answer word are the same word, allowing one to be the
/// start of the other ("key" finds "keyframe") once it is long enough not to match by chance.
fn same_word(asked: &str, written: &str) -> bool {
    let (asked_length, written_length) = (asked.chars().count(), written.chars().count());
    asked == written || (asked_length >= 3 && written.starts_with(asked)) || (written_length >= 4 && asked_length <= written_length + LONGER_BY && asked.starts_with(written))
}

/// How well a topic answers a question already reduced to `asked` roots: each question word
/// counts once, in the best place it is found.
fn score(asked: &[String], topic: &Topic) -> u32 {
    let (title, keywords, body) = (roots(topic.title), roots(topic.keywords), roots(topic.body));
    let found_in = |words: &[String], asked: &str| words.iter().any(|written| same_word(asked, written));
    asked
        .iter()
        .map(|asked| {
            if found_in(&title, asked) {
                TITLE_POINTS
            } else if found_in(&keywords, asked) {
                KEYWORD_POINTS
            } else if found_in(&body, asked) {
                BODY_POINTS
            } else {
                0
            }
        })
        .sum()
}

/// The topics that answer `question`, best first, as positions in `topics`; at most `limit`.
/// Topics that score the same keep the order they were given in. Empty when the question has
/// no words that matter or nothing fits.
///
/// A question that finds nothing as typed is tried once more with its mistyped words put
/// right against the words the topics use.
pub fn answers(question: &str, topics: &[Topic], limit: usize) -> Vec<usize> {
    let asked = roots(question);
    if asked.is_empty() {
        return Vec::new();
    }
    let found = best(&asked, topics, LOWEST_SCORE, limit);
    if !found.is_empty() {
        return found;
    }
    put_right(&asked, topics).map_or(found, |asked| best(&asked, topics, LOWEST_SCORE, limit))
}

/// The topics nearest to a question, for offering "did you mean…" when `answers` finds
/// nothing: any topic sharing a word with it counts, however slightly. Best first, at most
/// `limit`.
pub fn closest(question: &str, topics: &[Topic], limit: usize) -> Vec<usize> {
    let asked = roots(question);
    if asked.is_empty() {
        return Vec::new();
    }
    let asked = put_right(&asked, topics).unwrap_or(asked);
    best(&asked, topics, 1, limit)
}

/// The topics scoring at least `lowest` for the `asked` roots, best first, at most `limit`.
fn best(asked: &[String], topics: &[Topic], lowest: u32, limit: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, u32)> = topics.iter().enumerate().map(|(index, topic)| (index, score(asked, topic))).filter(|(_, points)| *points >= lowest).collect();
    scored.sort_by_key(|&(_, points)| std::cmp::Reverse(points));
    scored.into_iter().take(limit).map(|(index, _)| index).collect()
}

/// The `asked` roots with mistyped ones replaced by the heading or keyword word they are a
/// slip for. `None` when none was changed.
fn put_right(asked: &[String], topics: &[Topic]) -> Option<Vec<String>> {
    let mut known: Vec<String> = topics.iter().flat_map(|topic| roots(topic.title).into_iter().chain(roots(topic.keywords))).collect();
    known.sort_unstable();
    known.dedup();
    let mut changed = false;
    let fixed = asked
        .iter()
        .map(|word| match crate::spelling::nearest(word, &known) {
            Some(read) => {
                changed = true;
                read.to_owned()
            }
            None => word.clone(),
        })
        .collect();
    changed.then_some(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topics() -> Vec<Topic<'static>> {
        vec![
            Topic { title: "Split at the playhead", body: "Cuts the selected clips in two where the playhead is.", keywords: "cut divide" },
            Topic { title: "Easy ease", body: "Starts and ends a move gently.", keywords: "smooth curve" },
            Topic { title: "Change a keyboard shortcut", body: "Settings lists every command with its keys.", keywords: "hotkey rebind" },
            Topic { title: "See a layer's keyframes", body: "Press U to show every keyframe under the selected clip.", keywords: "show reveal" },
        ]
    }

    fn first(question: &str) -> Option<usize> {
        answers(question, &topics(), 3).first().copied()
    }

    #[test]
    fn a_question_finds_the_answer_that_shares_its_words() {
        assert_eq!(first("How do I split a clip?"), Some(0));
        assert_eq!(first("show keyframes"), Some(3));
    }

    #[test]
    fn everyday_words_and_endings_find_the_apps_own_words() {
        assert_eq!(first("make my motion smoother"), Some(1));
        assert_eq!(first("hotkeys"), Some(2));
        assert_eq!(first("splitting clips"), Some(0));
    }

    #[test]
    fn small_words_alone_and_unknown_words_find_nothing() {
        assert!(answers("how do I", &topics(), 3).is_empty());
        assert!(answers("", &topics(), 3).is_empty());
        assert!(answers("bake a cake", &topics(), 3).is_empty());
    }

    #[test]
    fn a_passing_mention_in_the_text_is_not_enough() {
        // "gently" is only in Easy Ease's text.
        assert!(answers("gently", &topics(), 3).is_empty());
    }

    #[test]
    fn a_request_is_not_answered_for_its_verb_or_a_word_inside_a_longer_one() {
        let topics = vec![Topic { title: "Make motion smoother", body: "Use Easy Ease.", keywords: "ease" }, Topic { title: "Go back to an earlier step", body: "Use History.", keywords: "undo" }];
        assert!(answers("make it purple", &topics, 3).is_empty(), "“make” alone says nothing");
        assert!(answers("blur the background", &topics, 3).is_empty(), "“background” is not “back”");
        assert_eq!(answers("going back a step", &topics, 3), vec![1]);
    }

    #[test]
    fn mistyped_questions_still_find_their_answer() {
        assert_eq!(first("keyfarmes"), Some(3));
        assert_eq!(first("change a keybaord shortcut"), Some(2));
        // A word that is merely unknown is not bent into one that is.
        assert!(answers("bake a cake", &topics(), 3).is_empty());
    }

    #[test]
    fn the_closest_topics_are_offered_when_nothing_answers() {
        // "gently" alone is too slight to answer with, but it is the nearest thing known.
        assert!(answers("gently", &topics(), 3).is_empty());
        assert_eq!(closest("gently", &topics(), 3), vec![1]);
        assert!(closest("bake a cake", &topics(), 3).is_empty());
    }

    #[test]
    fn the_best_answers_come_first_and_no_more_than_asked_for() {
        // "keyboard shortcut" is the whole heading of one topic; "keyframes" shares only "key".
        let found = answers("keyboard shortcut keys", &topics(), 1);
        assert_eq!(found, vec![2]);
    }
}
