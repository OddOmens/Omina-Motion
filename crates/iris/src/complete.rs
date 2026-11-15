//! Help with typing to Iris: what a half-typed message could become (the slash commands and
//! recipes that start that way) and going back through what was typed before with the arrow
//! keys, as a terminal does. The app draws the suggestions and catches the keys; this only
//! decides what they hold.

use crate::recipes::Recipe;

/// One way a half-typed message could be finished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The message as it would be after choosing it. A recipe that takes values ends in a
    /// space, ready for them.
    pub text: String,
    /// A few words on what it does, to show beside it.
    pub about: String,
}

/// The shortest plain (not slash) text that is completed, so typing one letter doesn't fill
/// the field with suggestions.
const SHORTEST_PLAIN: usize = 2;

/// What `typed` could become: the app's slash commands (`commands`, as "/name" and what it
/// does) and the recipes (by command or by name) that start the way it does, best first, at
/// most `limit`. Empty when nothing fits, or the text is already whole.
pub fn suggestions(typed: &str, commands: &[(&str, &str)], recipes: &[Recipe], limit: usize) -> Vec<Completion> {
    let lower = typed.trim_start().to_lowercase();
    // Once a space follows a command, its values are being typed: nothing to finish.
    if lower.is_empty() || (lower.starts_with('/') && lower.contains(' ')) {
        return Vec::new();
    }
    let about = |recipe: &Recipe| format!("Recipe: {}", recipe.steps.join(" → "));
    let finished = |recipe: &Recipe, text: String| if recipe.slots().is_empty() { text } else { format!("{text} ") };
    let mut found: Vec<Completion> = Vec::new();
    if lower.starts_with('/') {
        found.extend(commands.iter().filter(|(command, _)| command.starts_with(&lower) && *command != lower).map(|(command, about)| Completion { text: (*command).to_owned(), about: (*about).to_owned() }));
        found.extend(recipes.iter().filter(|recipe| recipe.command().starts_with(&lower) && recipe.command() != lower).map(|recipe| Completion { text: finished(recipe, recipe.command()), about: about(recipe) }));
    } else if lower.chars().count() >= SHORTEST_PLAIN {
        found.extend(recipes.iter().filter(|recipe| recipe.called().starts_with(&lower) && recipe.called() != lower).map(|recipe| Completion { text: finished(recipe, recipe.called()), about: about(recipe) }));
    }
    // Shorter first: the nearest finish to what is typed.
    found.sort_by_key(|completion| completion.text.len());
    found.dedup_by(|a, b| a.text == b.text);
    found.truncate(limit);
    found
}

/// The most messages kept to go back through.
pub const MOST_RECALLED: usize = 100;

/// What was typed before, newest last, and where the arrow keys have got to in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recall {
    typed: Vec<String>,
    /// The entry being shown, while going back through them.
    at: Option<usize>,
    /// What was in the field before going back, given back on coming forward past the newest.
    draft: String,
}

impl Recall {
    /// A recall holding these messages, oldest first (as `entries` gives them back).
    pub fn new(mut typed: Vec<String>) -> Self {
        typed.retain(|message| !message.trim().is_empty());
        let excess = typed.len().saturating_sub(MOST_RECALLED);
        typed.drain(..excess);
        Self { typed, ..Self::default() }
    }

    /// The messages kept, oldest first, for saving.
    pub fn entries(&self) -> &[String] {
        &self.typed
    }

    /// Keeps a message just sent, and starts again from the newest. The same message twice
    /// in a row is kept once.
    pub fn push(&mut self, message: &str) {
        let message = message.trim();
        self.at = None;
        self.draft.clear();
        if message.is_empty() || self.typed.last().is_some_and(|last| last == message) {
            return;
        }
        self.typed.push(message.to_owned());
        if self.typed.len() > MOST_RECALLED {
            self.typed.remove(0);
        }
    }

    /// The message before the one shown (the up arrow). `current` is the field as it is, kept
    /// to give back later. `None` when there is nothing older.
    pub fn older(&mut self, current: &str) -> Option<String> {
        let next = match self.at {
            None => {
                self.draft = current.to_owned();
                self.typed.len().checked_sub(1)?
            }
            Some(at) => at.checked_sub(1)?,
        };
        self.at = Some(next);
        self.typed.get(next).cloned()
    }

    /// The message after the one shown (the down arrow), or the draft once past the newest.
    /// `None` when not going back through them.
    pub fn newer(&mut self) -> Option<String> {
        let at = self.at?;
        if at + 1 < self.typed.len() {
            self.at = Some(at + 1);
            return self.typed.get(at + 1).cloned();
        }
        self.at = None;
        Some(std::mem::take(&mut self.draft))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMANDS: &[(&str, &str)] = &[("/help", "List commands"), ("/clear", "Clear the chat"), ("/check", "What I notice")];

    fn recipes() -> Vec<Recipe> {
        vec![Recipe::new("tidy up", ["fix everything"]), Recipe::new("lower third {text}", ["add a text layer saying {text}"]), Recipe::new("title card", ["add a title"])]
    }

    fn texts(typed: &str) -> Vec<String> {
        suggestions(typed, COMMANDS, &recipes(), 5).into_iter().map(|completion| completion.text).collect()
    }

    #[test]
    fn commands_and_recipes_are_finished_from_how_they_start() {
        assert_eq!(texts("/c"), vec!["/clear", "/check"]);
        assert_eq!(texts("/ti"), vec!["/tidy-up", "/title-card"]);
        assert_eq!(texts("/lo"), vec!["/lower-third "], "ready for its value");
        assert_eq!(texts("ti"), vec!["tidy up", "title card"]);
        assert_eq!(suggestions("ti", COMMANDS, &recipes(), 1).len(), 1);
    }

    #[test]
    fn nothing_is_offered_for_whole_or_unknown_text() {
        assert!(texts("").is_empty());
        assert!(texts("t").is_empty(), "one letter is too little");
        assert!(texts("/help").is_empty());
        assert!(texts("tidy up").is_empty());
        assert!(texts("/lower-third Jane").is_empty());
        assert!(texts("fade in").is_empty());
    }

    #[test]
    fn the_arrows_go_back_and_forth_and_give_the_draft_back() {
        let mut recall = Recall::new(vec!["fade in".to_owned(), "pulse".to_owned()]);
        assert_eq!(recall.newer(), None, "not going back yet");
        assert_eq!(recall.older("half typed").as_deref(), Some("pulse"));
        assert_eq!(recall.older("pulse").as_deref(), Some("fade in"));
        assert_eq!(recall.older("fade in"), None, "nothing older");
        assert_eq!(recall.newer().as_deref(), Some("pulse"));
        assert_eq!(recall.newer().as_deref(), Some("half typed"));
        recall.push("export");
        recall.push("export");
        assert_eq!(recall.entries(), ["fade in", "pulse", "export"]);
    }

    #[test]
    fn only_the_newest_messages_are_kept() {
        let mut recall = Recall::new((0..150).map(|number| number.to_string()).collect());
        assert_eq!(recall.entries().len(), MOST_RECALLED);
        assert_eq!(recall.entries()[0], "50");
        recall.push("new");
        assert_eq!(recall.entries().len(), MOST_RECALLED);
        assert_eq!(recall.entries().last().map(String::as_str), Some("new"));
    }
}
