//! Reading one message as several requests in a row: "fade the title in, then make a vertical
//! version", "brighter; more contrast; export". Each piece is handed to the app in order as if
//! it had been typed on its own.
//!
//! A message is split where it says so: at "then", "after that", a semicolon, the end of a
//! sentence or a new line. A plain "and" does not split, since "start at 5s and fade in" is
//! one request about one layer. Nothing inside quotes splits, as that is a name or text to
//! show.

/// The most pieces one message is read as. Anything longer is kept as its last piece.
pub const MOST_STEPS: usize = 12;

/// Words that begin a piece without being part of it: "*first* fade in, *then* pulse".
const LEAD_INS: &[&str] = &["then", "and", "first", "finally", "lastly", "also", "afterwards", "afterward"];

/// `message` as the requests it holds, in order. A message that holds one comes back as it
/// was typed (trimmed), so reading it changes nothing.
pub fn steps(message: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_quotes = false;
    let words: Vec<&str> = message.split(' ').collect();
    let mut index = 0;
    while index < words.len() {
        // A new line always ends a piece, even inside a word run ("fade in\npulse").
        let mut lines = words[index].split('\n');
        let mut word = lines.next().unwrap_or("");
        for next_line in lines {
            current.push(word);
            close(&mut current, &mut pieces);
            in_quotes = false;
            word = next_line;
        }
        let was_in_quotes = in_quotes;
        in_quotes ^= word.chars().filter(|character| matches!(character, '"' | '“' | '”')).count() % 2 == 1;
        // A word with a quote mark in it opens, closes or is all of a quoted name.
        if was_in_quotes || in_quotes || word.contains(['"', '“', '”']) {
            current.push(word);
            index += 1;
            continue;
        }
        let bare = word.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase();
        let next_bare = words.get(index + 1).map(|next| next.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase());
        if bare == "then" && !current.is_empty() {
            // "…and then…", "…, then…": the joining word before it goes too.
            if current.last().is_some_and(|last| last.eq_ignore_ascii_case("and")) {
                current.pop();
            }
            close(&mut current, &mut pieces);
        } else if bare == "after" && next_bare.as_deref() == Some("that") && !current.is_empty() {
            close(&mut current, &mut pieces);
            index += 1;
        } else {
            current.push(word);
            // A semicolon or the end of a sentence ends the piece. "1.5" and "e.g" have no
            // space after the stop, so they are never at the end of a word here.
            let ends_sentence = word.ends_with(';') || ((word.ends_with('.') || word.ends_with('!')) && word.chars().rev().nth(1).is_some_and(char::is_alphanumeric) && index + 1 < words.len());
            if ends_sentence {
                close(&mut current, &mut pieces);
            }
        }
        index += 1;
    }
    close(&mut current, &mut pieces);
    if pieces.len() <= 1 {
        return if message.trim().is_empty() { Vec::new() } else { vec![message.trim().to_owned()] };
    }
    if pieces.len() > MOST_STEPS {
        let rest = pieces.split_off(MOST_STEPS - 1).join(" then ");
        pieces.push(rest);
    }
    pieces
}

/// Ends the piece being gathered: tidied and added to `pieces` unless nothing is left of it.
fn close(current: &mut Vec<&str>, pieces: &mut Vec<String>) {
    let mut words: &[&str] = current;
    // "First", "and", "then" at the front introduce the piece.
    while let Some((first, rest)) = words.split_first() {
        let bare = first.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase();
        if first.is_empty() || (LEAD_INS.contains(&bare.as_str()) && !rest.is_empty()) {
            words = rest;
        } else {
            break;
        }
    }
    let piece = words.join(" ");
    let piece = piece.trim().trim_end_matches([';', ',', '.', '!']).trim();
    if !piece.is_empty() {
        pieces.push(piece.to_owned());
    }
    current.clear();
}

/// The most requests one "and" list is read as. A longer list is left whole.
const MOST_PARTS: usize = 8;

/// Words joined by "and" that are one thing, not two requests: "black and white", "fade in
/// and out".
const PAIRS: &[(&str, &str)] = &[
    ("black", "white"),
    ("bright", "airy"),
    ("in", "out"),
    ("up", "down"),
    ("back", "forth"),
    ("left", "right"),
    ("top", "bottom"),
    ("width", "height"),
    ("drag", "drop"),
    ("copy", "paste"),
    ("cut", "paste"),
    ("salt", "pepper"),
    ("now", "then"),
];

/// Words that open a request without being its verb: "*please* make it red".
const POLITE: &[&str] = &["please", "pls", "can", "could", "would", "will", "you", "just", "now", "ok", "okay", "and", "also"];

/// A word without the punctuation around it, lower case.
fn bare(word: &str) -> String {
    word.trim_matches(|character: char| !character.is_alphanumeric()).to_lowercase()
}

/// One request as the smaller requests joined in it by "and", "&", "plus" or commas: "make it
/// bold and red" gives "make it bold" and "red". Nothing inside quotes splits, nor do pairs
/// that are one thing ("black and white"). A request with nothing joined comes back whole.
///
/// The parts are only candidates: "start at 5s and fade in" is one request to one reader. Use
/// `joined` to decide whether to read them apart.
pub fn parts(piece: &str) -> Vec<String> {
    let words: Vec<&str> = piece.split_whitespace().collect();
    let mut found: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_quotes = false;
    for (index, word) in words.iter().enumerate() {
        let was_in_quotes = in_quotes;
        in_quotes ^= word.chars().filter(|character| matches!(character, '"' | '“' | '”')).count() % 2 == 1;
        let quoted = was_in_quotes || in_quotes || word.contains(['"', '“', '”']);
        let joiner = !quoted && matches!(bare(word).as_str(), "and" | "plus") || *word == "&";
        if joiner {
            let before = current.last().map(|word| bare(word)).unwrap_or_default();
            let after = words.get(index + 1).map(|word| bare(word)).unwrap_or_default();
            let pair = PAIRS.iter().any(|(first, second)| *first == before && *second == after);
            // A joiner at either end, or inside a pair, joins nothing. One right after a comma
            // ("bold, italic, and red") only repeats what the comma said.
            let joins = !current.is_empty() || !found.is_empty();
            if !pair && joins && index + 1 < words.len() {
                if !current.is_empty() {
                    found.push(current.join(" "));
                    current.clear();
                }
                continue;
            }
        }
        current.push(word);
        // A comma ends a part ("bold, italic and red"), unless it is inside quotes.
        if !quoted && word.ends_with(',') && index + 1 < words.len() {
            found.push(current.join(" ").trim_end_matches(',').to_owned());
            current.clear();
        }
    }
    found.push(current.join(" "));
    found.retain(|part| !part.trim().is_empty());
    if found.len() < 2 || found.len() > MOST_PARTS { vec![piece.trim().to_owned()] } else { found }
}

/// Ways a part that says only what is new might be meant, given the part before it: after
/// "make the title uppercase", the part "red" may mean "make red" or "make it red". The app
/// tries the part as it stands first, then these in order.
pub fn carried(first: &str, part: &str) -> Vec<String> {
    let Some(verb) = first.split_whitespace().map(bare).find(|word| !word.is_empty() && !POLITE.contains(&word.as_str())) else { return Vec::new() };
    if part.split_whitespace().next().map(bare).as_deref() == Some(verb.as_str()) {
        return Vec::new();
    }
    vec![format!("{verb} {part}"), format!("{verb} it {part}")]
}

/// How a request joined by "and" should be read: whole, or as its parts one after another.
/// `kind_of` is the app's own judgement of a piece of text, made without acting on it: `None`
/// when no reader of the app understands it, otherwise something that says which reader does
/// and which layer (or other thing) it names, so two parts compare equal only when one reader
/// could take both together.
///
/// The request is read apart only when every part is understood on its own (if need be with
/// the first part's verb carried over, see `carried`) and the parts differ in kind, or the
/// whole is not understood as it stands. Otherwise it comes back whole, so a reader that
/// handles its own lists ("start at 5s and fade in") keeps doing so.
pub fn joined<K: PartialEq>(piece: &str, kind_of: impl Fn(&str) -> Option<K>) -> Vec<String> {
    let whole = vec![piece.trim().to_owned()];
    let parts = parts(piece);
    if parts.len() < 2 {
        return whole;
    }
    let mut read: Vec<(String, K)> = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let candidates = std::iter::once(part.clone()).chain(if index == 0 { Vec::new() } else { carried(&parts[0], part) });
        let Some(understood) = candidates.into_iter().find_map(|text| kind_of(&text).map(|kind| (text, kind))) else { return whole };
        read.push(understood);
    }
    let same_kind = read.windows(2).all(|pair| pair[0].1 == pair[1].1);
    if same_kind && kind_of(piece).is_some_and(|kind| kind == read[0].1) {
        return whole;
    }
    read.into_iter().map(|(text, _)| text).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_request_comes_back_as_typed() {
        assert_eq!(steps("  Start at 5s and fade in  "), vec!["Start at 5s and fade in"]);
        assert_eq!(steps("How long is the video?"), vec!["How long is the video?"]);
        assert_eq!(steps("Make it 1.5 seconds long."), vec!["Make it 1.5 seconds long."]);
        assert!(steps("   ").is_empty());
    }

    #[test]
    fn then_semicolons_sentences_and_lines_split() {
        assert_eq!(steps("fade the title in, then make a vertical version"), vec!["fade the title in", "make a vertical version"]);
        assert_eq!(steps("brighter; more contrast; export"), vec!["brighter", "more contrast", "export"]);
        assert_eq!(steps("Fade in. Pulse! And then export."), vec!["Fade in", "Pulse", "export"]);
        assert_eq!(steps("fade in\npulse\n\nexport"), vec!["fade in", "pulse", "export"]);
        assert_eq!(steps("First fade in and then pulse, after that export"), vec!["fade in", "pulse", "export"]);
        assert_eq!(steps("go to the start; next marker"), vec!["go to the start", "next marker"]);
    }

    #[test]
    fn nothing_inside_quotes_splits() {
        assert_eq!(steps("add text saying “now and then; maybe.” then center it"), vec!["add text saying “now and then; maybe.”", "center it"]);
        assert_eq!(steps("rename it to \"Then\""), vec!["rename it to \"Then\""]);
    }

    #[test]
    fn a_message_that_starts_with_then_is_one_request() {
        assert_eq!(steps("then fade out"), vec!["then fade out"]);
    }

    #[test]
    fn and_lists_give_their_parts_but_pairs_and_quotes_stay_together() {
        assert_eq!(parts("make it bold and red"), vec!["make it bold", "red"]);
        assert_eq!(parts("bold, italic, and red"), vec!["bold", "italic", "red"]);
        assert_eq!(parts("duplicate it & mute it"), vec!["duplicate it", "mute it"]);
        assert_eq!(parts("make it black and white"), vec!["make it black and white"]);
        assert_eq!(parts("fade in and out and pulse"), vec!["fade in and out", "pulse"]);
        assert_eq!(parts("add text saying “salt, pepper and more” and center it"), vec!["add text saying “salt, pepper and more”", "center it"]);
        assert_eq!(parts("and fade in"), vec!["and fade in"]);
        assert_eq!(parts("resize to 1,920 wide"), vec!["resize to 1,920 wide"]);
    }

    /// A stand-in for an app's readers: which one would take a piece of text, and the layer
    /// it names.
    fn kind_of(text: &str) -> Option<(&'static str, Option<&'static str>)> {
        let layer = ["title", "logo"].into_iter().find(|name| text.contains(name));
        let timing = ["start at", "fade"].iter().any(|word| text.contains(word));
        let chore = ["duplicate", "hide", "lock", "make it red", "make the title red"].iter().any(|word| text.contains(word));
        let text_style = ["uppercase", "bold"].iter().any(|word| text.contains(word));
        // The timing reader takes a message with any timing in it, as the real one does.
        if timing {
            Some(("timing", layer))
        } else if chore && !text_style {
            Some(("chore", layer))
        } else if text_style && !text.contains(" and ") {
            Some(("text", layer))
        } else {
            None
        }
    }

    #[test]
    fn a_joined_request_is_read_apart_only_when_its_parts_differ() {
        // One reader takes both parts, about the same layer: it stays whole.
        assert_eq!(joined("start at 5s and fade in", kind_of), vec!["start at 5s and fade in"]);
        // Two readers: read apart, though the timing reader would have taken the whole.
        assert_eq!(joined("duplicate it and fade it in", kind_of), vec!["duplicate it", "fade it in"]);
        // One reader but two layers: read apart, so each step goes to its own layer.
        assert_eq!(joined("hide the logo and lock the title", kind_of), vec!["hide the logo", "lock the title"]);
        // A part that says only what is new borrows the first part's verb.
        assert_eq!(joined("make the title uppercase and red", kind_of), vec!["make the title uppercase", "make it red"]);
        // A part nobody understands leaves the request whole, for the app to answer as before.
        assert_eq!(joined("fade in and frobnicate", kind_of), vec!["fade in and frobnicate"]);
        assert_eq!(joined("fade in", kind_of), vec!["fade in"]);
    }

    #[test]
    fn very_long_scripts_keep_their_tail_as_one_piece() {
        let long = vec!["pulse"; 20].join(" then ");
        let pieces = steps(&long);
        assert_eq!(pieces.len(), MOST_STEPS);
        assert!(pieces[MOST_STEPS - 1].contains("then"));
    }
}
