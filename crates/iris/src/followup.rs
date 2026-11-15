//! Short replies that only make sense after something else: "again", "a bit more", "too
//! much", "undo that", "same for the logo". They are read here so every app understands the
//! same ones; what each means in an app is up to the app, with the help of what it remembers
//! of the conversation (see `front::Memory`).
//!
//! Only a message that is nothing but the reply counts. "More contrast" names what it wants
//! and is read by the app as any other request.

/// A reply that refers back to what was just done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Followup {
    /// The same thing once more.
    Again,
    /// The same thing, further.
    More,
    /// Not so far: it was too much.
    Less,
    /// Take the last change back.
    Undo,
    /// Put back what was just undone.
    Redo,
    /// The same thing to something else, named here ("same for the logo").
    SameFor(String),
}

/// Words that add nothing to a reply: "*please* do that again", "*ok* undo".
const FILLER: &[&str] = &["please", "pls", "ok", "okay", "yes", "yeah", "now", "just", "can", "could", "you", "iris", "hmm", "oops", "oh", "actually", "and", "so"];

const AGAIN: &[&str] = &["again", "do it again", "do that again", "do again", "once more", "one more time", "repeat", "repeat that", "repeat it", "same again", "the same again", "do it once more", "redo that again", "another", "another one"];
const MORE: &[&str] = &[
    "more", "a bit more", "bit more", "a little more", "little more", "even more", "some more", "more of that", "keep going", "further", "a bit further", "go further", "stronger", "not enough", "that's not enough", "thats not enough", "more than that", "much more", "a lot more",
];
const LESS: &[&str] = &[
    "less", "a bit less", "bit less", "a little less", "little less", "too much", "that's too much", "thats too much", "that is too much", "way too much", "not so much", "not that much", "tone it down", "tone that down", "ease off", "back off", "back off a bit", "weaker", "too strong", "that's too strong", "thats too strong", "less than that", "much less", "a lot less",
];
const UNDO: &[&str] = &["undo", "undo that", "undo it", "undo this", "go back", "revert", "revert that", "take that back", "take it back", "never mind", "nevermind", "cancel that", "scrap that", "no undo", "undo please", "put it back how it was", "that's worse", "thats worse"];
const REDO: &[&str] = &["redo", "redo that", "redo it", "put it back", "put that back", "bring it back", "bring that back"];

/// The message as its words alone, lower case, without filler and punctuation.
fn bare(message: &str) -> String {
    let lower = message.to_lowercase().replace('’', "'");
    lower.split(|character: char| !(character.is_alphanumeric() || character == '\'')).filter(|word| !word.is_empty() && !FILLER.contains(word)).collect::<Vec<_>>().join(" ")
}

/// The name in "same for the logo", as it was typed: the words after the joining word, without
/// a leading "the" or the quotes and stops around it.
fn named_after<'a>(message: &'a str, lower: &str, joiners: &[&str]) -> Option<&'a str> {
    let (at, joiner) = joiners.iter().filter_map(|joiner| lower.find(joiner).map(|at| (at, joiner))).min_by_key(|(at, _)| *at)?;
    let rest = message.get(at + joiner.len()..)?.trim();
    let rest = rest.trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '.' | '!' | '?' | ','));
    let rest = rest.strip_prefix("the ").or_else(|| rest.strip_prefix("The ")).unwrap_or(rest).trim();
    let short = !rest.is_empty() && rest.split_whitespace().count() <= 5;
    short.then_some(rest)
}

/// Reads `message` as a follow-up. `None` when it is anything else.
pub fn read(message: &str) -> Option<Followup> {
    let said = bare(message);
    let is = |list: &[&str]| list.contains(&said.as_str());
    if is(AGAIN) {
        return Some(Followup::Again);
    }
    if is(MORE) {
        return Some(Followup::More);
    }
    if is(LESS) {
        return Some(Followup::Less);
    }
    if is(UNDO) {
        return Some(Followup::Undo);
    }
    if is(REDO) {
        return Some(Followup::Redo);
    }
    // "Same for the logo", "do the same to layer 2", "the same with “Big Sale”".
    let lower = message.to_lowercase();
    if said.starts_with("same ") || said.starts_with("the same ") || said.starts_with("do the same ") || said.starts_with("do same ") {
        return named_after(message, &lower, &["same thing to ", "same thing for ", "same thing with ", "same thing on ", "same to ", "same for ", "same with ", "same on "]).map(|name| Followup::SameFor(name.to_owned()));
    }
    None
}

/// Every word the follow-ups are written with, for putting a mistyped one right (see
/// `spelling`).
pub fn vocabulary() -> Vec<String> {
    [AGAIN, MORE, LESS, UNDO, REDO].iter().flat_map(|list| list.iter()).flat_map(|phrase| crate::spelling::words(phrase)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_replies_read_whatever_the_politeness() {
        assert_eq!(read("again"), Some(Followup::Again));
        assert_eq!(read("Please do that again!"), Some(Followup::Again));
        assert_eq!(read("ok, one more time"), Some(Followup::Again));
        assert_eq!(read("a bit more"), Some(Followup::More));
        assert_eq!(read("More."), Some(Followup::More));
        assert_eq!(read("that’s too much"), Some(Followup::Less));
        assert_eq!(read("tone it down"), Some(Followup::Less));
        assert_eq!(read("Undo that please"), Some(Followup::Undo));
        assert_eq!(read("oops, go back"), Some(Followup::Undo));
        assert_eq!(read("redo"), Some(Followup::Redo));
    }

    #[test]
    fn the_same_for_something_else_keeps_its_name_as_typed() {
        assert_eq!(read("same for the Logo"), Some(Followup::SameFor("Logo".to_owned())));
        assert_eq!(read("Do the same to layer 2"), Some(Followup::SameFor("layer 2".to_owned())));
        assert_eq!(read("and the same with “Big Sale”."), Some(Followup::SameFor("Big Sale".to_owned())));
        assert_eq!(read("same"), None);
    }

    #[test]
    fn requests_that_name_what_they_want_are_not_follow_ups() {
        assert_eq!(read("more contrast"), None);
        assert_eq!(read("undo the crop"), None);
        assert_eq!(read("less saturation please"), None);
        assert_eq!(read("how do I undo?"), None);
        assert_eq!(read(""), None);
    }
}
