//! Putting mistyped words right, so "make it brigther" still reads as "brighter". A word is
//! only changed when the app doesn't know it and exactly one word it does know is a slip of
//! the finger away: one letter wrong, missing, added or swapped with its neighbor (two such
//! slips for long words). Nothing is guessed from meaning and nothing is AI.
//!
//! Apps use this as a second try: a message is read as typed first, and only when nothing
//! understands it is it read again with its words put right, saying which were changed.

/// The shortest word that is put right. Shorter words are too close to each other ("cut",
/// "cat", "car") to tell a slip from another word.
const SHORTEST: usize = 5;
/// Words at least this long may be two slips away.
const LONG: usize = 9;

/// How many slips of the finger turn `from` into `to`: letters changed, added, left out, or
/// swapped with the one beside them.
pub fn distance(from: &str, to: &str) -> usize {
    let (from, to): (Vec<char>, Vec<char>) = (from.chars().collect(), to.chars().collect());
    // rows[i][j] is the distance between the first i letters of `from` and the first j of `to`.
    let mut rows = vec![vec![0usize; to.len() + 1]; from.len() + 1];
    for (index, row) in rows.iter_mut().enumerate() {
        row[0] = index;
    }
    for (column, cell) in rows[0].iter_mut().enumerate() {
        *cell = column;
    }
    for i in 1..=from.len() {
        for j in 1..=to.len() {
            let change = usize::from(from[i - 1] != to[j - 1]);
            let mut best = (rows[i - 1][j] + 1).min(rows[i][j - 1] + 1).min(rows[i - 1][j - 1] + change);
            if i > 1 && j > 1 && from[i - 1] == to[j - 2] && from[i - 2] == to[j - 1] {
                best = best.min(rows[i - 2][j - 2] + 1);
            }
            rows[i][j] = best;
        }
    }
    rows[from.len()][to.len()]
}

/// The words of a piece of text, lower case, for building a vocabulary.
pub fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_lowercase)
}

/// The one known word `word` is most likely a slip for, when it isn't known itself. `None`
/// when it is known (also as a plural), too short to tell, or when two known words are
/// equally near.
pub fn nearest<'a>(word: &str, known: &'a [String]) -> Option<&'a str> {
    let length = word.chars().count();
    if length < SHORTEST || !word.chars().all(char::is_alphabetic) {
        return None;
    }
    let is_known = |word: &str| known.iter().any(|known| known == word);
    if is_known(word) || word.strip_suffix('s').is_some_and(is_known) || is_known(&format!("{word}s")) {
        return None;
    }
    let most = if length >= LONG { 2 } else { 1 };
    let mut best: Option<(usize, &str)> = None;
    let mut tied = false;
    for candidate in known {
        let candidate_length = candidate.chars().count();
        if candidate_length < SHORTEST - 1 || candidate_length.abs_diff(length) > most {
            continue;
        }
        let slips = distance(word, candidate);
        if slips == 0 || slips > most {
            continue;
        }
        match best {
            Some((least, found)) if slips == least && found != candidate => {
                // Of two equally near words, the one starting with the same letter is meant:
                // people rarely get the first letter wrong.
                let first = word.chars().next();
                match (found.chars().next() == first, candidate.chars().next() == first) {
                    (true, false) => {}
                    (false, true) => {
                        best = Some((slips, candidate));
                        tied = false;
                    }
                    _ => tied = true,
                }
            }
            Some((least, _)) if slips >= least => {}
            _ => {
                best = Some((slips, candidate));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(_, found)| found)
}

/// A message with its mistyped words put right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    /// The message as it is now read.
    pub message: String,
    /// Each word changed, as typed and as read.
    pub swaps: Vec<(String, String)>,
}

impl Correction {
    /// A short note saying what was changed, to put before the answer: "(I read “brigther” as
    /// “brighter”.)".
    pub fn note(&self) -> String {
        let pairs: Vec<String> = self.swaps.iter().map(|(typed, read)| format!("“{typed}” as “{read}”")).collect();
        format!("(I read {}.)", pairs.join(" and "))
    }
}

/// `message` with each mistyped word replaced by the known word it is a slip for. `None` when
/// no word was changed. Words in quotes are left as they are, since they are names or text.
pub fn correct(message: &str, known: &[String]) -> Option<Correction> {
    let mut out = String::with_capacity(message.len());
    let mut swaps = Vec::new();
    let mut word = String::new();
    let mut in_quotes = false;
    let mut finish = |word: &mut String, out: &mut String, in_quotes: bool| {
        if word.is_empty() {
            return;
        }
        let lower = word.to_lowercase();
        match nearest(&lower, known).filter(|_| !in_quotes) {
            Some(read) => {
                swaps.push((std::mem::take(word), read.to_owned()));
                out.push_str(read);
            }
            None => out.push_str(&std::mem::take(word)),
        }
    };
    for character in message.chars() {
        if character.is_alphanumeric() {
            word.push(character);
            continue;
        }
        finish(&mut word, &mut out, in_quotes);
        if matches!(character, '"' | '“' | '”') {
            in_quotes = !in_quotes;
        }
        out.push(character);
    }
    finish(&mut word, &mut out, in_quotes);
    (!swaps.is_empty()).then_some(Correction { message: out, swaps })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<String> {
        ["brighter", "contrast", "keyframe", "shadow", "shadows", "later", "layer", "export", "saturation", "fade"].map(str::to_owned).to_vec()
    }

    #[test]
    fn slips_are_counted() {
        assert_eq!(distance("brighter", "brighter"), 0);
        assert_eq!(distance("brigther", "brighter"), 1, "two letters swapped");
        assert_eq!(distance("contrst", "contrast"), 1, "a letter left out");
        assert_eq!(distance("exxport", "export"), 1, "a letter added");
        assert_eq!(distance("shadaw", "shadow"), 1, "a letter wrong");
        assert_eq!(distance("", "fade"), 4);
    }

    #[test]
    fn a_mistyped_word_is_put_right_and_said() {
        let fixed = correct("Make it brigther and add contrst", &known()).unwrap();
        assert_eq!(fixed.message, "Make it brighter and add contrast");
        assert_eq!(fixed.note(), "(I read “brigther” as “brighter” and “contrst” as “contrast”.)");
        // A long word may be two slips away.
        assert_eq!(correct("more satruaton", &known()).map(|fixed| fixed.message).as_deref(), Some("more saturation"));
    }

    #[test]
    fn known_short_and_quoted_words_are_left_alone() {
        assert_eq!(correct("fade the layer later", &known()), None, "every word is known");
        assert_eq!(correct("fad it", &known()), None, "too short to be sure of");
        assert_eq!(correct("rename it to “Shadaw”", &known()), None, "a name in quotes is as it was meant");
        assert_eq!(correct("keyframes and exports", &known()), None, "plurals of known words are known");
        assert_eq!(correct("12345 67890", &known()), None);
    }

    #[test]
    fn a_word_between_two_known_ones_is_not_guessed() {
        // "laper" is one slip from both "later" and "layer".
        assert_eq!(nearest("laper", &known()), None);
        // The same first letter breaks the tie.
        let known = ["shade", "spade"].map(str::to_owned).to_vec();
        assert_eq!(nearest("shadr", &known), Some("shade"));
    }
}
