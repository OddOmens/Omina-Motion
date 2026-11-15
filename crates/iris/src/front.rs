//! The front door every message comes through before an app reads it. It handles what is the
//! same in every app, so each one behaves alike and gets it for free:
//!
//! - several requests in one message ("fade in, then pulse") become one turn each (`script`).
//!   Requests joined by "and" are the app's to take apart, with `script::joined`, since only
//!   the app knows whether one of its readers understands the whole;
//! - recipes are taught, listed, forgotten and run (`recipes`);
//! - sums and conversions are answered (`calc`);
//! - "again" repeats the last thing that changed something, and the other follow-ups
//!   ("more", "too much", "undo that", "same for the logo") are handed to the app as what
//!   they are, since their meaning depends on the app (`followup`).
//!
//! What is left of a message is handed back as `Turn::Ask`, for the app to read exactly as if
//! it had been typed on its own. The app keeps a `Memory` between messages and tells it when a
//! turn changed something (`Memory::changed`), which is all "again" and "save that as…" need.

use crate::calc::{self, Units};
use crate::followup::{self, Followup};
use crate::recipes::{self, Recipe, Said, Trigger};

/// How deep a recipe may run other recipes before it is taken for a loop.
const DEEPEST: usize = 4;
/// The most turns one message becomes, however its recipes unfold.
const MOST_TURNS: usize = 40;

/// What every app's Iris understands, for its `/help`.
pub const HELP: &str = "In every app I also understand:\n• Several steps at once: join them with “and” or “then”, or put each on its own line\n• Follow-ups: “again”, “a bit more”, “too much”, “undo that”\n• Recipes: “teach tidy: step one, then step two” and from then on just say “tidy”. Or do something and say “save that as tidy”. /recipes lists them, “forget tidy” drops one, “what would tidy do?” shows its steps without running them\n• Recipes that take a value: “teach lower third {text}: add a text layer saying {text}”, then “lower third Jane Doe”\n• Recipes on a key, a button or a moment: “give tidy the shortcut ⌘⇧T”, “put tidy on the toolbar”, “run tidy before export” (also when I open, before saving, after importing)\n• Sums and conversions: “1920 / 3”, “15% of 240”, “5 seconds in frames”, “8 inches in pixels”\n• Any menu command by its name: “export”, “open settings”\n• Typos: I'll say how I read a word I had to put right";

/// What an app's Iris remembers between messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Memory {
    /// The last message that changed something, as typed. "Again" repeats it.
    pub last_change: Option<String>,
    /// The message being answered now, kept until the app says whether it changed anything.
    pending: Option<String>,
    /// The recipes taught in this app.
    pub taught: Vec<Recipe>,
    /// The recipes the app comes with, which can't be replaced or forgotten. One given a
    /// shortcut, a button or a moment is kept in `taught` as a copy holding those, which
    /// "forget" takes away again.
    pub built_in: Vec<Recipe>,
    /// The shortcuts the app's own commands use, as `recipes::normalize_key` writes them, so a
    /// recipe is never given one of them.
    pub app_keys: Vec<String>,
}

impl Memory {
    /// A memory holding the recipes taught so far and the ones the app comes with.
    pub fn new(mut taught: Vec<Recipe>, built_in: Vec<Recipe>) -> Self {
        // A copy of one the app comes with holds only its shortcut, button and moments: its
        // steps are always the app's current ones.
        for recipe in &mut taught {
            if let Some(own) = built_in.iter().find(|own| own.name == recipe.name) {
                recipe.steps = own.steps.clone();
            }
        }
        Self { taught, built_in, ..Self::default() }
    }

    /// Every recipe that can be run: the taught ones, then the app's own (each once).
    pub fn known(&self) -> Vec<Recipe> {
        let own = self.built_in.iter().filter(|own| !self.taught.iter().any(|taught| taught.name == own.name));
        self.taught.iter().chain(own).cloned().collect()
    }

    /// The recipe with this name, taught or the app's own.
    pub fn recipe(&self, name: &str) -> Option<Recipe> {
        self.known().into_iter().find(|recipe| recipe.name == name)
    }

    /// The recipes that run by themselves at `trigger`, by name, in the order they are known.
    pub fn triggered(&self, trigger: Trigger) -> Vec<String> {
        self.known().into_iter().filter(|recipe| recipe.when.contains(&trigger)).map(|recipe| recipe.name).collect()
    }

    /// Each recipe that has a shortcut, as the shortcut and the recipe's name.
    pub fn keyed(&self) -> Vec<(String, String)> {
        self.known().into_iter().filter_map(|recipe| Some((recipe.key?, recipe.name))).collect()
    }

    /// The recipes that have a button beside the chat, by name.
    pub fn buttons(&self) -> Vec<String> {
        self.known().into_iter().filter(|recipe| recipe.button).map(|recipe| recipe.name).collect()
    }

    /// Changes the recipe called `name` with `change`, copying one the app comes with into
    /// `taught` first. `false` when there is no such recipe.
    fn adjust(&mut self, name: &str, change: impl FnOnce(&mut Recipe)) -> bool {
        if !self.taught.iter().any(|recipe| recipe.name == name) {
            let Some(own) = self.built_in.iter().find(|own| own.name == name).cloned() else { return false };
            self.taught.push(own);
        }
        self.taught.iter_mut().find(|recipe| recipe.name == name).map(change).is_some()
    }

    /// Adds recipes read from a shared file (`recipes::from_share_text`): new names are added
    /// and taught ones of the same name replaced. Ones named as the app's own only bring their
    /// shortcut, button and moments. A shortcut the app or another recipe already uses is left
    /// off. Returns what to say.
    pub fn import(&mut self, incoming: Vec<Recipe>) -> String {
        let (mut added, mut replaced, mut keys_left_off) = (0, 0, Vec::new());
        for mut recipe in incoming {
            if let Some(key) = recipe.key.clone()
                && (self.app_keys.contains(&key) || self.keyed().iter().any(|(used, name)| *used == key && *name != recipe.name))
            {
                recipe.key = None;
                keys_left_off.push(recipe.called());
            }
            if let Some(own) = self.built_in.iter().find(|own| own.name == recipe.name) {
                recipe.steps = own.steps.clone();
            }
            match self.taught.iter().position(|taught| taught.name == recipe.name) {
                Some(index) => {
                    self.taught[index] = recipe;
                    replaced += 1;
                }
                None if self.taught.len() < recipes::MOST_RECIPES => {
                    self.taught.push(recipe);
                    added += 1;
                }
                None => {}
            }
        }
        let mut said = format!("Added {added} recipe{} and replaced {replaced}.", if added == 1 { "" } else { "s" });
        if !keys_left_off.is_empty() {
            said.push_str(&format!(" These came without their shortcut, which is already in use here: {}.", keys_left_off.join(", ")));
        }
        said
    }

    /// Tells the memory that the message being answered changed something in the project, so
    /// "again" and "save that as…" mean it from now on. Call it once for each turn that made a
    /// change; calling it for a turn that changed nothing would make "again" repeat nothing.
    pub fn changed(&mut self) {
        if let Some(message) = self.pending.take() {
            self.last_change = Some(message);
        }
    }
}

/// One thing to do for a message, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Words Iris says herself: the answer to a sum, a recipe learned or listed.
    Say(String),
    /// A request for the app to read and answer exactly as if it had been typed.
    Ask(String),
    /// A reply that refers back to what was just done, for the app to give its meaning to.
    /// `Followup::Again` never arrives here: it comes as the turns of the message it repeats.
    Followup(Followup),
}

/// A message read at the front door.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    /// What to do, in order.
    pub turns: Vec<Turn>,
    /// Whether the taught recipes changed, so the app should save them (`recipes::save`).
    pub recipes_changed: bool,
}

/// A recipe's steps as a numbered list.
fn numbered(steps: &[String]) -> String {
    steps.iter().enumerate().map(|(index, step)| format!("{}. {step}", index + 1)).collect::<Vec<_>>().join("\n")
}

/// Learns `recipe`, unless its name is taken or it would run itself. Returns what to say and
/// whether the taught recipes changed.
fn learn(recipe: Recipe, memory: &mut Memory, reserved: &[&str]) -> (String, bool) {
    let name = recipe.name.clone();
    if reserved.iter().any(|taken| taken.trim_start_matches('/').eq_ignore_ascii_case(&name)) {
        return (format!("/{name} is already one of my commands, so pick another name."), false);
    }
    if memory.built_in.iter().any(|known| known.name == name) {
        return (format!("“{name}” is a recipe I come with, so pick another name."), false);
    }
    // A step that is the recipe's own name would run it for ever.
    if recipe.steps.iter().any(|step| matches!(recipes::read(step, std::slice::from_ref(&recipe)), Some(Said::Run { .. } | Said::Problem(_)))) {
        return (format!("A recipe can't run itself, so I haven't learned “{name}”."), false);
    }
    let replaced = memory.taught.iter().position(|known| known.name == name);
    if replaced.is_none() && memory.taught.len() >= recipes::MOST_RECIPES {
        return (format!("I'm keeping {} recipes already, which is the most I can. Forget one first.", recipes::MOST_RECIPES), false);
    }
    let called = recipe.called();
    let example = if recipe.slots().is_empty() { called.clone() } else { format!("{called} …") };
    let said = format!(
        "{} “{name}” now means:\n{}\nSay “{example}” or type {} to run it, “what would {called} do?” to see it without running it, and “forget {called}” to drop it.",
        if replaced.is_some() { "Replaced." } else { "Got it." },
        numbered(&recipe.steps),
        recipe.command()
    );
    match replaced {
        // A replaced recipe keeps its shortcut, button and moments.
        Some(index) => {
            let old = std::mem::replace(&mut memory.taught[index], recipe);
            let new = &mut memory.taught[index];
            (new.key, new.button, new.when) = (old.key, old.button, old.when);
        }
        None => memory.taught.push(recipe),
    }
    (said, true)
}

/// Everything a message about recipes leads to: words, turns to run, and whether the taught
/// recipes changed.
fn recipe_turns(said: Said, memory: &mut Memory, units: &Units, reserved: &[&str], depth: usize, turns: &mut Vec<Turn>) -> bool {
    match said {
        Said::Problem(why) => turns.push(Turn::Say(why)),
        Said::Teach(recipe) => {
            let (said, changed) = learn(recipe, memory, reserved);
            turns.push(Turn::Say(said));
            return changed;
        }
        Said::TeachLast(name) => {
            let Some(last) = memory.last_change.clone() else {
                turns.push(Turn::Say(format!("There's nothing to save yet. Ask me to do something first, then say “save that as {name}”.")));
                return false;
            };
            let (said, changed) = learn(Recipe::new(name, crate::script::steps(&last)), memory, reserved);
            turns.push(Turn::Say(said));
            return changed;
        }
        Said::Forget(name) => {
            let own = memory.built_in.iter().any(|own| own.name == name);
            if let Some(index) = memory.taught.iter().position(|known| known.name == name) {
                memory.taught.remove(index);
                let said = if own { format!("“{name}” is back as it came: no shortcut, button or moments.") } else { format!("Forgotten: “{name}”.") };
                turns.push(Turn::Say(said));
                return true;
            }
            turns.push(Turn::Say(format!("“{name}” is a recipe I come with, so I can't forget it.")));
        }
        Said::SetKey { name, key } => return set_key(&name, key, memory, turns),
        Said::SetButton { name, on } => {
            let Some(recipe) = memory.recipe(&name) else { return false };
            if !recipe.slots().is_empty() && on {
                turns.push(Turn::Say(format!("“{}” needs values each time, so it can't have a button.", recipe.called())));
                return false;
            }
            memory.adjust(&name, |recipe| recipe.button = on);
            let said = if on { format!("“{}” has a button beside the chat now.", recipe.called()) } else { format!("“{}” has no button now.", recipe.called()) };
            turns.push(Turn::Say(said));
            return true;
        }
        Said::SetTrigger { name, trigger, on } => {
            let Some(recipe) = memory.recipe(&name) else { return false };
            memory.adjust(&name, |recipe| {
                recipe.when.retain(|known| *known != trigger);
                if on {
                    recipe.when.push(trigger);
                }
            });
            let said = if on {
                format!("“{}” will run by itself {}. Say “stop running {} {}” to undo that.", recipe.called(), trigger.phrase(), recipe.called(), trigger.phrase())
            } else {
                format!("“{}” won't run by itself {} any more.", recipe.called(), trigger.phrase())
            };
            turns.push(Turn::Say(said));
            return true;
        }
        Said::Describe { index, values } => {
            let Some(recipe) = memory.known().into_iter().nth(index) else { return false };
            let steps = if values.is_empty() { recipe.steps.clone() } else { recipe.filled(&values) };
            let mut said = format!("“{}” would do this, in order:\n{}", recipe.called(), numbered(&steps));
            // How else it runs, as a sentence: "It also runs from ⇧⌘G, from its button in the
            // chat and by itself before each export."
            let mut ways: Vec<String> = Vec::new();
            if let Some(key) = &recipe.key {
                ways.push(format!("from {}", recipes::key_label(key)));
            }
            if recipe.button {
                ways.push("from its button in the chat".to_owned());
            }
            ways.extend(recipe.when.iter().map(|trigger| format!("by itself {}", trigger.phrase())));
            if let [rest @ .., last] = ways.as_slice() {
                let listed = if rest.is_empty() { last.clone() } else { format!("{} and {last}", rest.join(", ")) };
                said.push_str(&format!("\nIt also runs {listed}."));
            }
            said.push_str("\nNothing has been changed.");
            turns.push(Turn::Say(said));
        }
        Said::List => {
            let mut said = if memory.taught.is_empty() {
                "You haven't taught me any recipes yet. Try “teach tidy: step one, then step two”, or do something and say “save that as tidy”.".to_owned()
            } else {
                format!("Recipes you've taught me:\n{}", recipes::listed(&memory.taught))
            };
            if !memory.built_in.is_empty() {
                said.push_str(&format!("\n\nRecipes I come with:\n{}", recipes::listed(&memory.built_in)));
            }
            turns.push(Turn::Say(said));
        }
        Said::Run { index, values } => {
            let Some(recipe) = memory.known().into_iter().nth(index) else { return false };
            if depth >= DEEPEST {
                turns.push(Turn::Say(format!("“{}” runs itself in a loop, so I stopped there.", recipe.called())));
                return false;
            }
            let mut changed = false;
            for step in recipe.filled(&values) {
                changed |= piece_turns(&step, memory, units, reserved, depth + 1, turns);
            }
            return changed;
        }
    }
    false
}

/// Gives the recipe called `name` the shortcut `key`, or takes its shortcut away. A shortcut
/// the app's own commands use is refused; one another recipe has moves to this one.
fn set_key(name: &str, key: Option<String>, memory: &mut Memory, turns: &mut Vec<Turn>) -> bool {
    let Some(recipe) = memory.recipe(name) else { return false };
    let called = recipe.called();
    let Some(key) = key else {
        if recipe.key.is_none() {
            turns.push(Turn::Say(format!("“{called}” has no shortcut.")));
            return false;
        }
        memory.adjust(name, |recipe| recipe.key = None);
        turns.push(Turn::Say(format!("“{called}” has no shortcut now.")));
        return true;
    };
    let label = recipes::key_label(&key);
    if memory.app_keys.contains(&key) {
        turns.push(Turn::Say(format!("{label} already does something here, so I haven't given it to “{called}”. Pick another, such as ⌃⌥ with a letter.")));
        return false;
    }
    if !recipe.slots().is_empty() {
        turns.push(Turn::Say(format!("“{called}” needs values each time, so it can't have a shortcut.")));
        return false;
    }
    let mut said = format!("{label} now runs “{called}”.");
    if let Some((_, other)) = memory.keyed().into_iter().find(|(used, other)| *used == key && other != name) {
        memory.adjust(&other, |recipe| recipe.key = None);
        said.push_str(&format!(" It was “{other}”'s, which has no shortcut now."));
    }
    memory.adjust(name, |recipe| recipe.key = Some(key));
    turns.push(Turn::Say(said));
    true
}

/// The turns one request leads to. Returns whether the taught recipes changed.
fn piece_turns(piece: &str, memory: &mut Memory, units: &Units, reserved: &[&str], depth: usize, turns: &mut Vec<Turn>) -> bool {
    if turns.len() >= MOST_TURNS {
        return false;
    }
    if let Some(said) = recipes::read(piece, &memory.known()) {
        return recipe_turns(said, memory, units, reserved, depth, turns);
    }
    if let Some(answer) = calc::answer(piece, units) {
        turns.push(Turn::Say(answer));
        return false;
    }
    match followup::read(piece) {
        Some(Followup::Again) => match memory.last_change.clone() {
            // The last message is read afresh, so a script or a recipe runs whole again.
            Some(last) if depth < DEEPEST => {
                let mut changed = false;
                for step in crate::script::steps(&last) {
                    changed |= piece_turns(&step, memory, units, reserved, depth + 1, turns);
                }
                return changed;
            }
            _ => turns.push(Turn::Say("There's nothing to repeat yet: I haven't changed anything.".to_owned())),
        },
        Some(other) => turns.push(Turn::Followup(other)),
        None => turns.push(Turn::Ask(piece.to_owned())),
    }
    false
}

/// Reads `message` at the front door. `reserved` is the app's own slash commands ("/help",
/// "/clear"…), which a recipe can't be named after.
pub fn read(message: &str, memory: &mut Memory, units: &Units, reserved: &[&str]) -> Reading {
    let message = message.trim();
    let mut reading = Reading::default();
    if message.is_empty() {
        return reading;
    }
    // What "again" will mean if this message changes something. Follow-ups and messages about
    // recipes themselves don't count: "again" after "more" still means the first request.
    let about_recipes = recipes::read(message, &memory.known());
    let is_followup = followup::read(message).is_some();
    let runs_recipe = matches!(about_recipes, Some(Said::Run { .. }));
    memory.pending = (!is_followup && (about_recipes.is_none() || runs_recipe)).then(|| message.to_owned());
    // A whole message about recipes is read whole: its steps hold the "then"s that would
    // otherwise split it.
    if let Some(said) = about_recipes {
        reading.recipes_changed = recipe_turns(said, memory, units, reserved, 0, &mut reading.turns);
        return reading;
    }
    // An app's own command takes the rest of its line as it is ("/name Now and Then").
    if message.starts_with('/') && calc::answer(message, units).is_none() {
        reading.turns.push(Turn::Ask(message.to_owned()));
        return reading;
    }
    for piece in crate::script::steps(message) {
        reading.recipes_changed |= piece_turns(&piece, memory, units, reserved, 0, &mut reading.turns);
    }
    reading
}

/// Runs the recipe called `name` (one without values) as if it had been asked for by name: from
/// its shortcut, its button or a moment it runs at. Empty when there is no such recipe.
pub fn run(name: &str, memory: &mut Memory, units: &Units, reserved: &[&str]) -> Reading {
    let mut reading = Reading::default();
    let Some(index) = memory.known().iter().position(|recipe| recipe.name == name) else { return reading };
    // One that needs values can't run from a key or a moment: say so rather than do nothing.
    let recipe = &memory.known()[index];
    if !recipe.slots().is_empty() {
        reading.turns.push(Turn::Say(format!("“{}” needs a value each time, so type it with one: “{} …”.", recipe.called(), recipe.called())));
        return reading;
    }
    memory.pending = Some(memory.known()[index].called());
    reading.recipes_changed = recipe_turns(Said::Run { index, values: Vec::new() }, memory, units, reserved, 0, &mut reading.turns);
    reading
}

/// Every word the front door and the shared readers know, for putting a mistyped message
/// right (see `spelling`). An app adds the words of its own patterns.
pub fn vocabulary() -> Vec<String> {
    let own = ["teach", "forget", "recipe", "recipes", "seconds", "frames", "inches", "pixels", "minutes"].map(str::to_owned);
    let mut words: Vec<String> = own.into_iter().chain(crate::intent::vocabulary()).chain(crate::layers::vocabulary()).chain(followup::vocabulary()).chain(crate::knowledge::vocabulary()).collect();
    words.sort_unstable();
    words.dedup();
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESERVED: &[&str] = &["/help", "/clear"];

    fn ask(message: &str, memory: &mut Memory) -> Vec<Turn> {
        read(message, memory, &Units { fps: Some(30.0), dpi: None }, RESERVED).turns
    }

    fn asked(text: &str) -> Turn {
        Turn::Ask(text.to_owned())
    }

    #[test]
    fn a_plain_message_is_handed_to_the_app_as_typed() {
        let mut memory = Memory::default();
        assert_eq!(ask("  How long is the video?  ", &mut memory), vec![asked("How long is the video?")]);
        assert_eq!(ask("/name Now and Then", &mut memory), vec![asked("/name Now and Then")]);
        assert!(ask("   ", &mut memory).is_empty());
    }

    #[test]
    fn several_steps_sums_and_follow_ups_each_get_a_turn() {
        let mut memory = Memory::default();
        assert_eq!(ask("fade in, then pulse; 2 + 2", &mut memory), vec![asked("fade in"), asked("pulse"), Turn::Say("2 + 2 = 4".to_owned())]);
        assert_eq!(ask("5 seconds in frames", &mut memory), vec![Turn::Say("5 seconds is 150 frames.".to_owned())]);
        assert_eq!(ask("too much", &mut memory), vec![Turn::Followup(Followup::Less)]);
        assert_eq!(ask("brighter then undo that", &mut memory), vec![asked("brighter"), Turn::Followup(Followup::Undo)]);
    }

    #[test]
    fn again_repeats_the_last_message_that_changed_something() {
        let mut memory = Memory::default();
        assert!(matches!(ask("again", &mut memory).as_slice(), [Turn::Say(_)]), "nothing to repeat yet");
        // A question changes nothing, so it is never what "again" means.
        ask("how long is it?", &mut memory);
        ask("fade in then pulse", &mut memory);
        memory.changed();
        ask("how long is it?", &mut memory);
        assert_eq!(ask("do that again", &mut memory), vec![asked("fade in"), asked("pulse")]);
        // "Again" changing something doesn't make "again" mean "again".
        memory.changed();
        assert_eq!(ask("again", &mut memory), vec![asked("fade in"), asked("pulse")]);
    }

    #[test]
    fn recipes_are_taught_run_listed_and_forgotten() {
        let mut memory = Memory::new(Vec::new(), vec![Recipe::new("polish", ["fix everything"])]);
        let reading = read("teach intro: fade in over a second, then pulse", &mut memory, &Units::default(), RESERVED);
        assert!(reading.recipes_changed);
        assert!(matches!(reading.turns.as_slice(), [Turn::Say(said)] if said.contains("1. fade in over a second") && said.contains("2. pulse")));
        assert_eq!(ask("intro", &mut memory), vec![asked("fade in over a second"), asked("pulse")]);
        assert_eq!(ask("start at 0 then /intro", &mut memory), vec![asked("start at 0"), asked("fade in over a second"), asked("pulse")]);
        assert_eq!(ask("polish", &mut memory), vec![asked("fix everything")]);
        assert!(matches!(ask("/recipes", &mut memory).as_slice(), [Turn::Say(said)] if said.contains("intro") && said.contains("polish")));
        // Running a recipe is what "again" then repeats.
        ask("intro", &mut memory);
        memory.changed();
        assert_eq!(ask("again", &mut memory), vec![asked("fade in over a second"), asked("pulse")]);
        let reading = read("forget intro", &mut memory, &Units::default(), RESERVED);
        assert!(reading.recipes_changed && memory.taught.is_empty());
        assert_eq!(ask("intro", &mut memory), vec![asked("intro")], "forgotten, so it is the app's to read");
    }

    #[test]
    fn recipes_with_values_keys_buttons_and_moments_work_through_the_front_door() {
        let mut memory = Memory::new(Vec::new(), vec![Recipe::new("polish", ["fix everything"])]);
        memory.app_keys = vec!["cmd-s".to_owned()];
        let units = Units::default();
        assert!(read("teach lower third {text}: add a text layer saying {text}, then put it in the bottom left", &mut memory, &units, RESERVED).recipes_changed);
        assert_eq!(ask("lower third Jane Doe", &mut memory), vec![asked("add a text layer saying Jane Doe"), asked("put it in the bottom left")]);
        assert!(matches!(ask("what would lower third Ana do?", &mut memory).as_slice(), [Turn::Say(said)] if said.contains("1. add a text layer saying Ana") && said.contains("Nothing has been changed")));
        // A key the app uses is refused; a built-in recipe can have one, kept as a copy.
        assert!(!read("give polish the shortcut cmd s", &mut memory, &units, RESERVED).recipes_changed);
        assert!(read("give polish the shortcut ctrl alt p", &mut memory, &units, RESERVED).recipes_changed);
        assert_eq!(memory.keyed(), vec![("ctrl-alt-p".to_owned(), "polish".to_owned())]);
        assert_eq!(memory.known().iter().filter(|recipe| recipe.name == "polish").count(), 1);
        // Moving the key to another recipe takes it from the first.
        ask("teach intro: fade in", &mut memory);
        ask("bind intro to ctrl alt p", &mut memory);
        assert_eq!(memory.keyed(), vec![("ctrl-alt-p".to_owned(), "intro".to_owned())]);
        // Buttons and moments, and running by name as a key or moment would.
        ask("put polish on the toolbar", &mut memory);
        ask("run polish before export", &mut memory);
        assert_eq!(memory.buttons(), vec!["polish".to_owned()]);
        assert_eq!(memory.triggered(Trigger::BeforeExport), vec!["polish".to_owned()]);
        assert_eq!(run("polish", &mut memory, &units, RESERVED).turns, vec![asked("fix everything")]);
        assert!(matches!(run("lower third {text}", &mut memory, &units, RESERVED).turns.as_slice(), [Turn::Say(said)] if said.contains("needs a value")));
        // Forgetting the copy puts the app's own back as it came.
        ask("forget polish", &mut memory);
        assert!(memory.buttons().is_empty() && memory.triggered(Trigger::BeforeExport).is_empty());
        assert_eq!(ask("polish", &mut memory), vec![asked("fix everything")]);
        // A replaced recipe keeps its shortcut.
        ask("teach intro: fade in slowly", &mut memory);
        assert_eq!(memory.recipe("intro").and_then(|recipe| recipe.key), Some("ctrl-alt-p".to_owned()));
    }

    #[test]
    fn shared_recipes_are_imported_without_clashing_keys() {
        let mut memory = Memory::new(vec![Recipe { key: Some("ctrl-alt-i".to_owned()), ..Recipe::new("intro", ["fade in"]) }], Vec::new());
        memory.app_keys = vec!["cmd-e".to_owned()];
        let incoming = vec![Recipe { key: Some("cmd-e".to_owned()), ..Recipe::new("outro", ["fade out"]) }, Recipe { key: Some("ctrl-alt-i".to_owned()), ..Recipe::new("pop", ["pop in"]) }, Recipe::new("intro", ["slide in"])];
        let said = memory.import(incoming);
        assert!(said.starts_with("Added 2 recipes and replaced 1.") && said.contains("outro") && said.contains("pop"), "{said}");
        assert!(memory.keyed().is_empty(), "the replaced intro brought no key, and the others' clashed");
    }

    #[test]
    fn the_last_change_can_be_saved_as_a_recipe() {
        let mut memory = Memory::default();
        assert!(matches!(ask("save that as glow", &mut memory).as_slice(), [Turn::Say(said)] if said.contains("nothing to save")));
        ask("fade in then make it pulse", &mut memory);
        memory.changed();
        let reading = read("save that as glow", &mut memory, &Units::default(), RESERVED);
        assert!(reading.recipes_changed);
        assert_eq!(memory.taught, vec![Recipe::new("glow", ["fade in", "make it pulse"])]);
    }

    #[test]
    fn names_that_are_taken_or_would_loop_are_refused() {
        let mut memory = Memory::new(Vec::new(), vec![Recipe::new("polish", ["fix everything"])]);
        for message in ["teach help: fade in", "teach polish: fade in", "teach loop: fade in then loop"] {
            let reading = read(message, &mut memory, &Units::default(), RESERVED);
            assert!(!reading.recipes_changed && memory.taught.is_empty(), "{message}");
            assert!(matches!(reading.turns.as_slice(), [Turn::Say(_)]), "{message}");
        }
        // Two recipes that run each other stop instead of going round for ever.
        memory.taught = vec![Recipe::new("ping", ["pong"]), Recipe::new("pong", ["ping"])];
        let turns = ask("ping", &mut memory);
        assert!(turns.len() <= MOST_TURNS && matches!(turns.last(), Some(Turn::Say(said)) if said.contains("loop")), "{turns:?}");
    }

    #[test]
    fn the_vocabulary_holds_the_words_of_every_shared_reader() {
        let words = vocabulary();
        for word in ["fade", "duplicate", "again", "recipe", "keyframe"] {
            assert!(words.iter().any(|known| known == word), "{word}");
        }
    }
}
