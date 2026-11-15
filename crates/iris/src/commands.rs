//! Running an app's own commands by name: "export", "open the mixer", "show rulers", "zoom to
//! fit". The app hands over its commands (each with the name its menu shows) and gets back
//! the one the message names, so anything in the menus can be asked for in a sentence.
//!
//! This is deliberately strict, since running the wrong command is worse than running none:
//! the message must be a short request, not a question, and its words must be the command's
//! own (or the extra words the app lists for it) with little left over. When two commands fit
//! equally well, both are returned for the app to offer.

use crate::matching::root;

/// One command, as the finding sees it.
#[derive(Clone, Copy, Debug)]
pub struct Command<'a> {
    /// The name its menu or button shows: "Export…", "Show Rulers".
    pub name: &'a str,
    /// Extra words people might ask with that the name doesn't use.
    pub keywords: &'a str,
}

/// What a message names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// No command, or the message isn't a request for one.
    None,
    /// This command, as a position in the list given.
    One(usize),
    /// Commands for the app to offer rather than run: several that fit equally well, or a
    /// single one that was found only by its extra words with other words around them ("show
    /// everything" for a command that lists "everything"), which is too loose to act on.
    Several(Vec<usize>),
}

/// The most words that matter a request for a command has. Longer messages are sentences
/// about something else that happen to mention a command.
const MOST_WORDS: usize = 6;
/// The most commands offered when several fit.
const MOST_OFFERED: usize = 4;

/// Words that ask for a command without being part of its name: "*open the* export *window*".
const ASKING: &[&str] = &[
    "open", "show", "run", "start", "launch", "bring", "up", "go", "goto", "toggle", "switch", "turn", "use", "pick", "choose", "get", "give", "let", "lets", "now", "just", "take", "do", "make", "create", "window", "panel", "dialog", "menu", "command", "tool", "mode", "view", "screen", "page", "tab", "some", "new", "again",
];

/// Words that say nothing in a request. Unlike a question's small words, "in", "out", "on"
/// and "off" stay: they tell Zoom In from Zoom Out.
const FILLER: &[&str] = &["a", "an", "the", "please", "pls", "my", "me", "i", "you", "can", "could", "would", "will", "it", "this", "that", "of", "for", "to", "s"];

/// The words of a name or request that matter, as roots.
fn words(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric()).map(str::to_lowercase).filter(|word| !word.is_empty() && !FILLER.contains(&word.as_str())).map(|word| root(&word)).collect()
}

/// The command `message` names among `commands`.
pub fn find(message: &str, commands: &[Command]) -> Found {
    if crate::intent::asks(message) {
        return Found::None;
    }
    let said = words(message);
    if said.is_empty() || said.len() > MOST_WORDS {
        return Found::None;
    }
    let asking: Vec<String> = ASKING.iter().map(|word| root(word)).collect();
    // For each command: how many of its name's words were said, and how many of the words
    // said are accounted for by its name, its extra words or the asking words.
    let mut fits: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let (name, keywords) = (words(command.name), words(command.keywords));
        if name.is_empty() {
            continue;
        }
        let named = name.iter().filter(|word| said.contains(word)).count();
        let by_keyword = said.iter().filter(|word| !name.contains(word) && keywords.contains(word)).count();
        let spare = said.iter().filter(|word| !name.contains(word) && !keywords.contains(word) && !asking.contains(word)).count();
        // Every word said must be accounted for, and at least one must be the command's own:
        // a word of its name or one of its extra words.
        let whole_name = named == name.len();
        if spare == 0 && (named > 0 || by_keyword > 0) {
            // Best first: the whole name said, then the most of it, then the shortest name
            // (so "export" means Export rather than Export Frame).
            let rank = if whole_name { 0 } else if named > 0 { 1 } else { 2 };
            fits.push((rank, name.len() - named, name.len(), index));
        }
    }
    fits.sort_unstable();
    let Some(&(rank, missing, length, first)) = fits.first() else { return Found::None };
    let equal: Vec<usize> = fits.iter().take_while(|fit| (fit.0, fit.1, fit.2) == (rank, missing, length)).map(|fit| fit.3).take(MOST_OFFERED).collect();
    // Found by extra words alone, with asking words beside them: likely, but not sure enough.
    let loose = rank == 2 && said.iter().any(|word| asking.contains(word));
    if equal.len() == 1 && !loose { Found::One(first) } else { Found::Several(equal) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands() -> Vec<Command<'static>> {
        vec![
            Command { name: "Export…", keywords: "render save out deliver" },
            Command { name: "Export Frame", keywords: "still screenshot" },
            Command { name: "Show Rulers", keywords: "ruler measure" },
            Command { name: "Zoom to Fit", keywords: "fit whole" },
            Command { name: "Zoom In", keywords: "closer" },
            Command { name: "Zoom Out", keywords: "" },
            Command { name: "Audio Mixer", keywords: "levels volume" },
            Command { name: "Split Layer", keywords: "cut razor" },
        ]
    }

    #[test]
    fn a_command_is_found_by_its_name_however_it_is_asked_for() {
        let commands = commands();
        assert_eq!(find("export", &commands), Found::One(0));
        assert_eq!(find("Open the export window please", &commands), Found::One(0));
        assert_eq!(find("export frame", &commands), Found::One(1));
        assert_eq!(find("can you show the rulers", &commands), Found::One(2));
        assert_eq!(find("rulers", &commands), Found::One(2));
        assert_eq!(find("zoom to fit", &commands), Found::One(3));
        assert_eq!(find("open the mixer", &commands), Found::One(6));
    }

    #[test]
    fn extra_words_the_app_lists_find_it_too() {
        let commands = commands();
        assert_eq!(find("render", &commands), Found::One(0));
        assert_eq!(find("screenshot", &commands), Found::One(1));
        assert_eq!(find("take a screenshot", &commands), Found::Several(vec![1]), "loose wording is offered, not run");
        assert_eq!(find("levels", &commands), Found::One(6));
    }

    #[test]
    fn commands_that_fit_equally_are_all_offered() {
        assert_eq!(find("zoom", &commands()), Found::Several(vec![3, 4, 5]));
        assert_eq!(find("zoom out", &commands()), Found::One(5));
        assert_eq!(find("zoom in", &commands()), Found::One(4));
    }

    #[test]
    fn questions_long_sentences_and_other_words_find_nothing() {
        let commands = commands();
        assert_eq!(find("how do I export?", &commands), Found::None);
        assert_eq!(find("where is the mixer", &commands), Found::None);
        assert_eq!(find("export the blue title with a long fade and a big zoom", &commands), Found::None);
        assert_eq!(find("export my holiday", &commands), Found::None);
        assert_eq!(find("bake a cake", &commands), Found::None);
        assert_eq!(find("", &commands), Found::None);
    }
}
