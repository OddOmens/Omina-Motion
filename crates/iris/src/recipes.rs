//! Recipes: things Iris is taught to do by name. "Teach intro: fade in over a second, then
//! pulse" gives her a recipe called "intro"; from then on "intro" (or "/intro", or "do intro")
//! runs those steps in order, each exactly as if it had been typed. This is how people
//! automate what they do often, with no scripting: a recipe is just messages she already
//! understands, said in a row.
//!
//! A recipe can take values: "teach lower third {text}: add a text layer saying {text}, then
//! put it in the bottom left" is run as "lower third Jane Doe", with "Jane Doe" put where
//! `{text}` is written. It can also be given a keyboard shortcut ("give tidy up the shortcut
//! ⌘⇧T"), a button ("put tidy up on the toolbar") and moments it runs by itself ("run print
//! check before export"); the app reads those from the recipe and does the binding.
//!
//! Each app keeps its own recipes (a recipe of Motion's instructions means nothing in
//! Develop), all in one file beside Iris's shared settings, `recipes.json`. An app may also
//! come with recipes of its own; those can't be forgotten or replaced, though they can be
//! given a shortcut, a button or a moment like any other (see `front::Memory`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::App;

/// The file every app's recipes are kept in, inside Iris's shared folder.
const FILE_NAME: &str = "recipes.json";
/// The longest name a recipe can have, in characters, and the most words in it.
pub const NAME_LIMIT: usize = 24;
const NAME_WORDS: usize = 3;
/// The most recipes one app keeps, and the most steps in one.
pub const MOST_RECIPES: usize = 100;

/// The longest name of a value a recipe takes ("{text}"), in characters.
const SLOT_LIMIT: usize = 16;

/// Names a recipe can't have, because they already mean something in every app.
const TAKEN: &[&str] = &["teach", "forget", "recipes", "recipe", "help", "undo", "redo", "again", "more", "less", "that", "it", "this", "yes", "no"];

/// Words a recipe that takes a value can't be named after alone: "make {thing}" would catch
/// nearly every request.
const TOO_COMMON: &[&str] = &[
    "make", "add", "set", "put", "go", "export", "open", "show", "hide", "turn", "change", "move", "fade", "start", "crop", "do", "run", "use", "play", "save", "give", "get", "take", "send", "rename", "delete", "remove", "select", "what", "how",
];

/// A moment a recipe can run by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Just before an export starts. A recipe run this way doesn't set off another export's
    /// trigger: the app must not run triggers for exports a triggered recipe asks for.
    BeforeExport,
    /// When a project, document or photo is opened.
    OnOpen,
    /// Just before the work is saved.
    BeforeSave,
    /// When files have been brought in.
    AfterImport,
}

impl Trigger {
    pub const ALL: [Trigger; 4] = [Trigger::BeforeExport, Trigger::OnOpen, Trigger::BeforeSave, Trigger::AfterImport];

    /// How the moment reads in a sentence: "before each export".
    pub fn phrase(self) -> &'static str {
        match self {
            Self::BeforeExport => "before each export",
            Self::OnOpen => "whenever something is opened",
            Self::BeforeSave => "before each save",
            Self::AfterImport => "after each import",
        }
    }

    /// The moment a message names, if any: "before export", "when I open a project".
    fn named_in(lower: &str) -> Option<Self> {
        const SAID: &[(&[&str], Trigger)] = &[
            (&["before export", "before i export", "before an export", "before each export", "before every export", "before exporting", "when i export", "whenever i export", "on export"], Trigger::BeforeExport),
            (&["when i open", "whenever i open", "on open", "after opening", "when opening", "after i open", "when something is opened", "whenever something is opened"], Trigger::OnOpen),
            (&["before saving", "before i save", "before each save", "before every save", "when i save", "whenever i save", "on save"], Trigger::BeforeSave),
            (&["after import", "after importing", "after i import", "when i import", "whenever i import", "on import", "after each import"], Trigger::AfterImport),
        ];
        SAID.iter().find(|(phrases, _)| phrases.iter().any(|phrase| lower.contains(phrase))).map(|(_, trigger)| *trigger)
    }
}

/// One recipe: a name and the messages it stands for, with how it can be run besides by name.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Recipe {
    /// Lower case, as it is said to run it. Values it takes are written at the end in braces:
    /// "lower third {text}".
    pub name: String,
    pub steps: Vec<String>,
    /// A keyboard shortcut that runs it, written as the app's key bindings are ("cmd-shift-t").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Whether it has a button of its own beside Iris's chat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub button: bool,
    /// The moments it runs by itself.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub when: Vec<Trigger>,
}

impl Recipe {
    /// A recipe with no shortcut, button or moments.
    pub fn new(name: impl Into<String>, steps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self { name: name.into(), steps: steps.into_iter().map(Into::into).collect(), ..Self::default() }
    }

    /// The words that call it, without the values it takes: "lower third".
    pub fn called(&self) -> String {
        fixed_words(&self.name)
    }

    /// The values it takes, in order: `["text"]` for "lower third {text}".
    pub fn slots(&self) -> Vec<String> {
        slots_of(&self.name)
    }

    /// The command that runs it: "/lower-third".
    pub fn command(&self) -> String {
        format!("/{}", self.called().replace(' ', "-"))
    }

    /// Its steps with `values` put in place of its slots. A value holding words that would
    /// split a request in two ("and", a comma) is put in quotes unless the step already
    /// quotes it, so it arrives whole.
    pub fn filled(&self, values: &[String]) -> Vec<String> {
        let slots = self.slots();
        self.steps
            .iter()
            .map(|step| {
                let mut step = step.clone();
                for (slot, value) in slots.iter().zip(values) {
                    let marker = format!("{{{slot}}}");
                    let splits = [" and ", " & ", ",", ";", " then ", " plus "].iter().any(|joiner| value.to_lowercase().contains(joiner));
                    // One pass, searching on past each value put in, so a value that holds a
                    // marker of its own is put in as it is rather than filled again.
                    let mut out = String::with_capacity(step.len());
                    let mut rest = step.as_str();
                    while let Some(at) = rest.find(&marker) {
                        out.push_str(&rest[..at]);
                        let quoted_already = out.ends_with(['"', '“']);
                        out.push_str(&if splits && !quoted_already { format!("“{value}”") } else { value.clone() });
                        rest = &rest[at + marker.len()..];
                    }
                    out.push_str(rest);
                    step = out;
                }
                step
            })
            .collect()
    }

    /// What else it is set up with, for listing: "⌘⇧T, on the toolbar, before each export".
    pub fn extras(&self) -> String {
        let mut extras: Vec<String> = Vec::new();
        if let Some(key) = &self.key {
            extras.push(key_label(key));
        }
        if self.button {
            extras.push("on the toolbar".to_owned());
        }
        extras.extend(self.when.iter().map(|trigger| trigger.phrase().to_owned()));
        extras.join(", ")
    }
}

/// A name's words without its slots: "lower third {text}" gives "lower third".
fn fixed_words(name: &str) -> String {
    name.split_whitespace().filter(|word| !word.starts_with('{')).collect::<Vec<_>>().join(" ")
}

/// A name's slots: "card {name} {title}" gives `["name", "title"]`.
fn slots_of(name: &str) -> Vec<String> {
    name.split_whitespace().filter_map(|word| word.strip_prefix('{').and_then(|word| word.strip_suffix('}'))).map(str::to_owned).collect()
}

/// A keyboard shortcut as typed ("cmd+shift+t", "⌘⇧T", "control option k", "ctrl-alt-1")
/// written the way the apps' key bindings are: modifiers in a fixed order, then the key, joined
/// with dashes ("ctrl-alt-shift-cmd-k"). `Err` says why it won't do: it needs ⌘, ⌃ or ⌥ (a
/// letter alone or with ⇧ would stop that letter being typed) and exactly one key.
pub fn normalize_key(typed: &str) -> Result<String, String> {
    let mut text = String::new();
    for character in typed.trim().chars() {
        match character {
            '⌘' => text.push_str(" cmd "),
            '⇧' => text.push_str(" shift "),
            '⌥' => text.push_str(" alt "),
            '⌃' | '^' => text.push_str(" ctrl "),
            '+' | '-' | ',' => text.push(' '),
            other => text.extend(other.to_lowercase()),
        }
    }
    let (mut ctrl, mut alt, mut shift, mut cmd) = (false, false, false, false);
    let mut keys: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match word {
            "cmd" | "command" | "meta" | "super" => cmd = true,
            "shift" => shift = true,
            "alt" | "option" | "opt" => alt = true,
            "ctrl" | "control" => ctrl = true,
            "and" | "plus" | "key" | "the" => {}
            key => keys.push(key.to_owned()),
        }
    }
    let usable = |key: &str| (key.chars().count() == 1 && key.chars().all(|character| character.is_ascii_alphanumeric())) || (key.len() <= 3 && key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|number| (1..=12).contains(&number)));
    // "⌘⇧T" arrives as one word "t" after the symbols are spaced; "cmdshiftt" never does.
    let [key] = keys.as_slice() else { return Err(format!("“{}” isn't a shortcut I can set: say it as, for example, ⌘⇧T or “command shift T”.", typed.trim())) };
    if !usable(key) {
        return Err(format!("“{key}” isn't a key I can give a recipe: use a letter, a number or F1 to F12."));
    }
    if !(cmd || ctrl || alt) {
        return Err("A recipe's shortcut needs ⌘, ⌃ or ⌥ in it, so it doesn't stop that key being typed.".to_owned());
    }
    let mut parts: Vec<&str> = Vec::new();
    for (on, name) in [(ctrl, "ctrl"), (alt, "alt"), (shift, "shift"), (cmd, "cmd")] {
        if on {
            parts.push(name);
        }
    }
    parts.push(key);
    Ok(parts.join("-"))
}

/// A shortcut written as `normalize_key` writes it, shown as a Mac menu shows it: "⌃⌥⇧⌘K".
pub fn key_label(key: &str) -> String {
    key.split('-')
        .map(|part| match part {
            "ctrl" => "⌃".to_owned(),
            "alt" => "⌥".to_owned(),
            "shift" => "⇧".to_owned(),
            "cmd" => "⌘".to_owned(),
            key => key.to_uppercase(),
        })
        .collect()
}

/// What a message about recipes asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Said {
    /// Learn this recipe.
    Teach(Recipe),
    /// Learn the last thing done under this name ("save that as intro").
    TeachLast(String),
    /// Forget the recipe with this name.
    Forget(String),
    /// List the recipes.
    List,
    /// Run the known recipe at this position, with the values it takes.
    Run { index: usize, values: Vec<String> },
    /// Say what the known recipe at this position would do, with these values, doing nothing.
    Describe { index: usize, values: Vec<String> },
    /// Give the recipe with this name a shortcut, or (`None`) take its shortcut away.
    SetKey { name: String, key: Option<String> },
    /// Give the recipe with this name a button beside the chat, or take it away.
    SetButton { name: String, on: bool },
    /// Run the recipe with this name by itself at a moment, or stop doing so.
    SetTrigger { name: String, trigger: Trigger, on: bool },
    /// A recipe was being taught, but something about it won't do; the words say what.
    Problem(String),
}

/// A typed name made fit to keep: lower case, single spaces, without the quotes or slash
/// around it. `Err` says why a name won't do.
///
/// A name may end in values it takes, each a word in braces: "lower third {text}".
pub fn tidy_name(typed: &str) -> Result<String, String> {
    let trimmed = typed.trim().trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '\'' | '/' | '.' | ',' | ':')).to_lowercase();
    let name = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("A recipe needs a name, as in “teach intro: fade in, then pulse”.".to_owned());
    }
    let called = fixed_words(&name);
    let slots = slots_of(&name);
    if called.is_empty() {
        return Err("A recipe needs a word to call it by before the values it takes, as in “lower third {text}”.".to_owned());
    }
    if called.chars().count() > NAME_LIMIT || called.split(' ').count() > NAME_WORDS {
        return Err(format!("“{called}” is too long for a recipe's name. Keep it to {NAME_WORDS} words and {NAME_LIMIT} letters."));
    }
    if !called.chars().all(|character| character.is_alphanumeric() || matches!(character, ' ' | '-' | '_')) || !called.chars().any(char::is_alphabetic) {
        return Err(format!("“{called}” won't do as a recipe's name: use letters and numbers."));
    }
    if TAKEN.contains(&called.as_str()) {
        return Err(format!("“{called}” already means something to me, so pick another name."));
    }
    // Values come after the words that call it, each a plain word in braces, each once.
    let words: Vec<&str> = name.split(' ').collect();
    let first_slot = words.iter().position(|word| word.starts_with('{')).unwrap_or(words.len());
    let well_formed = words[first_slot..].iter().all(|word| {
        word.strip_prefix('{').and_then(|word| word.strip_suffix('}')).is_some_and(|slot| !slot.is_empty() && slot.chars().count() <= SLOT_LIMIT && slot.chars().all(|character| character.is_alphanumeric() || character == '_'))
    });
    if !well_formed || words[..first_slot].iter().any(|word| word.contains(['{', '}'])) {
        return Err("Put the values a recipe takes at the end of its name, each a word in braces: “lower third {text}”.".to_owned());
    }
    let mut unique = slots.clone();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() != slots.len() {
        return Err("Give each value a recipe takes a different name.".to_owned());
    }
    if !slots.is_empty() && TOO_COMMON.contains(&called.as_str()) {
        return Err(format!("“{called} …” would catch too many of your requests, so give it a more particular name, such as “quick {called}”."));
    }
    Ok(name)
}

/// The steps written after a recipe's name, split where they say so (see `script`).
fn steps_of(text: &str) -> Vec<String> {
    crate::script::steps(text.trim().trim_start_matches([':', '=', ',']).trim())
}

/// `rest` split at the first of `marks`, as the text before it and the text after.
fn split_at_first<'a>(rest: &'a str, marks: &[&str]) -> Option<(&'a str, &'a str)> {
    let lower = rest.to_lowercase();
    let (at, mark) = marks.iter().filter_map(|mark| lower.find(mark).map(|at| (at, mark))).min_by_key(|(at, _)| *at)?;
    Some((rest.get(..at)?, rest.get(at + mark.len()..)?))
}

/// The `{word}`s written in a step.
fn markers_in(step: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = step;
    while let Some(start) = rest.find('{') {
        let Some(length) = rest[start..].find('}') else { break };
        found.push(rest[start + 1..start + length].to_lowercase());
        rest = &rest[start + length + 1..];
    }
    found
}

/// A recipe from its typed name and steps, or what is wrong with it.
fn teach(name: &str, steps: &str) -> Said {
    let name = match tidy_name(name) {
        Ok(name) => name,
        Err(why) => return Said::Problem(why),
    };
    let steps = steps_of(steps);
    if steps.is_empty() {
        if !slots_of(&name).is_empty() {
            return Said::Problem("A recipe that takes values needs its steps written out, with each value where it goes: “teach lower third {text}: add a text layer saying {text}”.".to_owned());
        }
        return Said::TeachLast(name);
    }
    // Every value is used, and every value used is one the recipe takes.
    let slots = slots_of(&name);
    let used: Vec<String> = steps.iter().flat_map(|step| markers_in(step)).collect();
    if let Some(unknown) = used.iter().find(|marker| !slots.contains(marker)) {
        return Said::Problem(format!("A step uses {{{unknown}}}, but the recipe's name doesn't take it. Add it to the name: “{} {{{unknown}}}”.", fixed_words(&name)));
    }
    if let Some(unused) = slots.iter().find(|slot| !used.contains(slot)) {
        return Said::Problem(format!("The recipe takes {{{unused}}} but no step uses it. Write {{{unused}}} where the value goes."));
    }
    Said::Teach(Recipe::new(name, steps))
}

/// The values typed after a recipe's name, for a recipe taking `count` of them. One value is
/// the whole of the rest; several are taken from quotes, or else split at commas. `Err` says
/// what was missing.
fn values_of(rest: &str, recipe: &Recipe) -> Result<Vec<String>, String> {
    let slots = recipe.slots();
    let rest = rest.trim();
    let unquote = |text: &str| text.trim().trim_matches(|character: char| matches!(character, '"' | '“' | '”')).trim().to_owned();
    let values: Vec<String> = match slots.len() {
        0 => return Ok(Vec::new()),
        1 => vec![unquote(rest)],
        _ => {
            let quoted: Vec<String> = rest.split(['"', '“', '”']).skip(1).step_by(2).map(|value| value.trim().to_owned()).collect();
            if quoted.len() == slots.len() { quoted } else { rest.split([',', ';']).map(unquote).collect() }
        }
    };
    if values.len() != slots.len() || values.iter().any(String::is_empty) {
        let wanted: Vec<String> = slots.iter().map(|slot| format!("{{{slot}}}")).collect();
        let example = if slots.len() == 1 { format!("“{} …”", recipe.called()) } else { format!("“{} first, second”", recipe.called()) };
        return Err(format!("“{}” takes {}: say it as {example}.", recipe.called(), wanted.join(" and ")));
    }
    Ok(values)
}

/// The known recipe a run of `typed` words calls, with what was typed after its name. A
/// recipe that takes no values must be called by exactly its name.
fn called_by<'a>(typed: &'a str, known: &[Recipe]) -> Option<(usize, &'a str)> {
    let lower = typed.to_lowercase();
    // Longest names first, so "quick title {text}" wins over "quick {thing}".
    let mut order: Vec<usize> = (0..known.len()).collect();
    order.sort_by_key(|index| std::cmp::Reverse(known[*index].called().len()));
    for index in order {
        let recipe = &known[index];
        let called = recipe.called();
        for written in [called.clone(), called.replace(' ', "-")] {
            if recipe.slots().is_empty() {
                if lower == written {
                    return Some((index, ""));
                }
            } else if lower == written || lower.starts_with(&format!("{written} ")) {
                return Some((index, typed.get(written.len()..).unwrap_or("")));
            }
        }
    }
    None
}

/// The recipe a message names anywhere in it, by the words that call it, longest first.
fn named_in<'a>(lower: &str, known: &'a [Recipe]) -> Option<&'a Recipe> {
    let padded = format!(" {} ", lower.replace(['“', '”', '"', '?', '.', '!', ','], " "));
    let mut found: Vec<&Recipe> = known.iter().filter(|recipe| padded.contains(&format!(" {} ", recipe.called()))).collect();
    found.sort_by_key(|recipe| std::cmp::Reverse(recipe.called().len()));
    found.first().copied()
}

/// "Give tidy up the shortcut ⌘⇧T", "bind tidy up to cmd shift t", "remove the shortcut from
/// tidy up", "put tidy up on the toolbar", "run print check before export", "stop running tidy
/// up when I open".
fn settings_said(lower: &str, typed: &str, known: &[Recipe]) -> Option<Said> {
    let starts = |starts: &[&str]| starts.iter().any(|start| lower.starts_with(start));
    let recipe = || named_in(lower, known);
    let unknown = || Said::Problem("I don't have a recipe by that name. /recipes lists the ones I know.".to_owned());
    // Shortcuts.
    let unbinding = starts(&["remove the shortcut", "remove the key", "take the shortcut", "take away the shortcut", "unbind ", "clear the shortcut", "no shortcut for "]);
    let binding = starts(&["give ", "bind ", "set the shortcut", "set a shortcut", "assign "]) && (lower.contains("shortcut") || lower.contains(" key") || lower.starts_with("bind "));
    if unbinding || binding {
        let Some(recipe) = recipe() else { return Some(unknown()) };
        if unbinding {
            return Some(Said::SetKey { name: recipe.name.clone(), key: None });
        }
        // The key is what comes after the recipe's name and the joining words.
        let after_name = lower.find(&recipe.called()).map(|at| at + recipe.called().len()).unwrap_or(0);
        let mut key = typed.get(after_name..).unwrap_or("").trim();
        for lead in ["the shortcut", "a shortcut", "the key", "a key", "shortcut", "key", "to", "as", "on"] {
            key = key.strip_prefix(lead).map(str::trim).unwrap_or(key);
        }
        // "Set the shortcut for tidy up to ⌘⇧T" puts the key after "to".
        for lead in ["to", "as", "on", "the shortcut", "shortcut"] {
            key = key.strip_prefix(lead).map(str::trim).unwrap_or(key);
        }
        return Some(match normalize_key(key) {
            Ok(key) => Said::SetKey { name: recipe.name.clone(), key: Some(key) },
            Err(why) => Said::Problem(why),
        });
    }
    // Buttons.
    let toolbar = [" toolbar", " the bar", " button"].iter().any(|word| lower.contains(word));
    let pinning = (starts(&["put ", "add ", "pin ", "give "]) && toolbar) || starts(&["pin "]);
    let unpinning = (starts(&["take ", "remove "]) && toolbar && (lower.contains(" off ") || lower.contains(" from "))) || starts(&["unpin "]);
    if (pinning || unpinning) && (toolbar || starts(&["pin ", "unpin "])) {
        let Some(recipe) = recipe() else { return toolbar.then(unknown) };
        return Some(Said::SetButton { name: recipe.name.clone(), on: !unpinning });
    }
    // Moments it runs by itself.
    if let Some(trigger) = Trigger::named_in(lower) {
        let stopping = starts(&["stop running ", "don't run ", "dont run ", "never run ", "stop doing ", "no longer run "]);
        let running = starts(&["run ", "always run ", "do ", "always do ", "please run ", "automatically run ", "auto run "]);
        if stopping || running {
            let Some(recipe) = recipe() else { return Some(unknown()) };
            if !recipe.slots().is_empty() {
                return Some(Said::Problem(format!("“{}” needs values each time, so it can't run by itself.", recipe.called())));
            }
            return Some(Said::SetTrigger { name: recipe.name.clone(), trigger, on: running });
        }
    }
    None
}

/// Reads `message` as being about recipes, given the ones `known` (the app's own and the
/// taught ones). `None` when it is about something else.
pub fn read(message: &str, known: &[Recipe]) -> Option<Said> {
    let message = message.trim();
    let lower = message.to_lowercase();
    // What follows `start`, as typed. The starts are plain letters, so the same number of
    // bytes begins the typed message; `get` covers the rare letter whose lower case is shorter.
    let after = |start: &str| lower.starts_with(start).then(|| message.get(start.len()..)).flatten();
    // "/teach intro = fade in; pulse", "teach intro: fade in then pulse".
    if let Some(rest) = after("/teach ").or_else(|| after("teach ")) {
        // "Teach me…" asks for a tip; it isn't a recipe.
        if after("teach me").is_some() {
            return None;
        }
        return Some(match split_at_first(rest, &["=", ":"]) {
            Some((name, steps)) => teach(name, steps),
            // "/teach intro" alone saves the last thing done; "teach intro fade in" is unclear.
            None if rest.split_whitespace().count() <= NAME_WORDS && lower.starts_with('/') => teach(rest, ""),
            None => Said::Problem("To teach me a recipe, give it a name, a colon and the steps: “teach intro: fade in, then pulse”.".to_owned()),
        });
    }
    if lower == "/teach" || lower == "teach" {
        return Some(Said::Problem("To teach me a recipe, give it a name, a colon and the steps: “teach intro: fade in, then pulse”. Or do something and then say “save that as intro”.".to_owned()));
    }
    // "When I say intro, fade in then pulse", "when I say intro do …".
    if let Some(rest) = after("when i say ").or_else(|| after("whenever i say ")).or_else(|| after("if i say ")) {
        return Some(match split_at_first(rest, &[",", ":", " do ", " then ", " means ", " mean "]) {
            Some((name, steps)) => teach(name, steps),
            None => Said::Problem("Tell me what it should do: “when I say intro, fade in then pulse”.".to_owned()),
        });
    }
    // "Save that as intro", "remember that as intro", "call that intro".
    for start in ["save that as ", "save this as ", "save it as ", "remember that as ", "remember this as ", "call that ", "name that ", "teach that as "] {
        if let Some(name) = after(start) {
            // "Save this as a PNG" is about a file, not a recipe.
            if matches!(name.trim().to_lowercase().split_whitespace().next(), Some("a" | "an" | "png" | "jpg" | "jpeg" | "pdf" | "svg" | "gif" | "mp4" | "mov" | "tiff" | "webp" | "preset" | "snapshot" | "template" | "copy")) {
                return None;
            }
            return Some(teach(name, ""));
        }
    }
    // "Forget intro", "/forget intro".
    if let Some(name) = after("/forget ").or_else(|| after("forget about ")).or_else(|| after("forget ")).or_else(|| after("unlearn ")) {
        let name = name.trim().trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '/' | '.')).to_lowercase();
        let name = name.strip_prefix("the recipe ").or_else(|| name.strip_prefix("recipe ")).unwrap_or(&name).to_owned();
        // Only a recipe that exists is forgotten, so "forget it" stays small talk.
        return known.iter().any(|recipe| recipe.name == name).then_some(Said::Forget(name));
    }
    if matches!(lower.trim_end_matches(['?', '.']), "/recipes" | "/recipe" | "/macros" | "/taught" | "recipes" | "my recipes" | "list recipes" | "list my recipes" | "show recipes" | "show my recipes" | "what recipes do you know" | "what have i taught you" | "what did i teach you" | "what recipes are there") {
        return Some(Said::List);
    }
    if let Some(said) = settings_said(&lower, message, known) {
        return Some(said);
    }
    // Saying what one would do: "what would tidy up do?", "preview lower third Jane".
    for (start, end) in [("what would ", " do"), ("what does ", " do"), ("what's in ", ""), ("what is in ", ""), ("preview ", ""), ("/preview ", ""), ("dry run ", ""), ("/dry-run ", ""), ("explain ", "")] {
        let Some(middle) = after(start).map(|rest| rest.trim_end_matches(['?', '.', '!']).trim_end()) else { continue };
        let Some(middle) = (if end.is_empty() { Some(middle) } else { middle.to_lowercase().ends_with(end).then(|| &middle[..middle.len() - end.len()]) }) else { continue };
        let middle = middle.trim();
        let middle = if middle.to_lowercase().starts_with("the ") { &middle[4..] } else { middle };
        let middle = if middle.to_lowercase().ends_with(" recipe") { &middle[..middle.len() - " recipe".len()] } else { middle };
        if let Some((index, rest)) = called_by(middle, known) {
            // A preview without its values still says what it would do, slots and all.
            let values = values_of(rest, &known[index]).unwrap_or_default();
            return Some(Said::Describe { index, values });
        }
    }
    // Running one: its name alone, "/intro", "do intro", "run the intro recipe", "lower third
    // Jane Doe".
    let mut typed = message.trim_end_matches(['.', '!']).trim();
    for start in ["/", "do the ", "do ", "run the ", "run ", "apply the ", "apply ", "use the ", "use ", "play the ", "play "] {
        if typed.to_lowercase().starts_with(start) {
            typed = typed.get(start.len()..).unwrap_or(typed).trim();
            break;
        }
    }
    let typed = typed.trim_matches(|character: char| matches!(character, '"' | '“' | '”')).trim();
    let typed = if typed.to_lowercase().ends_with(" recipe") { &typed[..typed.len() - " recipe".len()] } else { typed };
    let (index, rest) = called_by(typed, known)?;
    Some(match values_of(rest, &known[index]) {
        Ok(values) => Said::Run { index, values },
        Err(why) => Said::Problem(why),
    })
}

/// Each recipe on a line of its own with its steps numbered, for listing.
pub fn listed(recipes: &[Recipe]) -> String {
    recipes
        .iter()
        .map(|recipe| {
            let extras = recipe.extras();
            let extras = if extras.is_empty() { String::new() } else { format!(" ({extras})") };
            format!("• {}{extras}: {}", recipe.name, recipe.steps.join(" → "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first line of a file of shared recipes, so a file of anything else is refused.
const SHARE_MARK: &str = "omina-recipes";

/// Recipes written out to give to someone else, or to keep: a small JSON file that says which
/// app they are for.
pub fn share_text(app: App, recipes: &[Recipe]) -> String {
    let file = serde_json::json!({ SHARE_MARK: 1, "app": key(app), "recipes": recipes });
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Recipes read back from `share_text`, checked as if each had been taught. `Err` says why
/// the file won't do: it isn't a recipes file, it is another app's, or a recipe in it is
/// unfit (named so, with why).
pub fn from_share_text(text: &str, app: App) -> Result<Vec<Recipe>, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| "That isn't a recipes file.".to_owned())?;
    if value.get(SHARE_MARK).is_none() {
        return Err("That isn't a recipes file.".to_owned());
    }
    let for_app = value.get("app").and_then(serde_json::Value::as_str).unwrap_or("");
    if for_app != key(app) {
        let other = App::ALL.into_iter().find(|other| key(*other) == for_app).map_or("another app", App::name);
        return Err(format!("Those recipes are for {other}, and its requests mean nothing in {}.", app.name()));
    }
    let recipes: Vec<Recipe> = serde_json::from_value(value.get("recipes").cloned().unwrap_or_default()).map_err(|_| "The recipes in that file can't be read.".to_owned())?;
    if recipes.len() > MOST_RECIPES {
        return Err(format!("That file holds {} recipes, and I keep at most {MOST_RECIPES}.", recipes.len()));
    }
    for recipe in &recipes {
        let steps = recipe.steps.join(" then ");
        if let Said::Problem(why) = teach(&recipe.name, &steps) {
            return Err(format!("“{}” won't do: {why}", recipe.name));
        }
        if let Some(key) = &recipe.key {
            normalize_key(key).map_err(|why| format!("“{}” won't do: {why}", recipe.name))?;
        }
    }
    Ok(recipes)
}

/// Every app's recipes as they are kept in the file, by the app's lower-case name.
type Book = BTreeMap<String, Vec<Recipe>>;

fn key(app: App) -> String {
    format!("{app:?}").to_lowercase()
}

fn read_book(folder: &Path) -> Book {
    std::fs::read_to_string(folder.join(FILE_NAME)).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

/// The recipes taught in `app`, from `folder`. Empty when none have been, or the file can't
/// be read.
pub fn load(folder: &Path, app: App) -> Vec<Recipe> {
    read_book(folder).remove(&key(app)).unwrap_or_default()
}

/// Keeps `recipes` as the ones taught in `app`, leaving the other apps' as they are. Written
/// through a temporary file so another app reading at the same moment never sees half a file.
pub fn save(folder: &Path, app: App, recipes: &[Recipe]) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let mut book = read_book(folder);
    book.insert(key(app), recipes.to_vec());
    let text = serde_json::to_string_pretty(&book).map_err(std::io::Error::other)?;
    let partial = folder.join(format!("{FILE_NAME}.{}.partial", std::process::id()));
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, folder.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intro() -> Recipe {
        Recipe::new("intro", ["fade in over a second", "pulse"])
    }

    fn lower_third() -> Recipe {
        Recipe::new("lower third {text}", ["add a text layer saying {text}", "put it in the bottom left"])
    }

    #[test]
    fn a_recipe_can_take_values() {
        assert_eq!(read("teach lower third {text}: add a text layer saying {text}, then put it in the bottom left", &[]), Some(Said::Teach(lower_third())));
        let known = [intro(), lower_third()];
        assert_eq!(read("lower third Jane Doe", &known), Some(Said::Run { index: 1, values: vec!["Jane Doe".to_owned()] }));
        assert_eq!(read("/lower-third “Ana López”", &known), Some(Said::Run { index: 1, values: vec!["Ana López".to_owned()] }));
        assert!(matches!(read("lower third", &known), Some(Said::Problem(_))), "a value is needed");
        assert_eq!(lower_third().filled(&["Jane Doe".to_owned()]), vec!["add a text layer saying Jane Doe", "put it in the bottom left"]);
        // A value with a joining word arrives whole.
        assert_eq!(lower_third().filled(&["Salt and Pepper".to_owned()])[0], "add a text layer saying “Salt and Pepper”");
        // A value holding the recipe's own marker is put in as typed, once.
        assert_eq!(lower_third().filled(&["{text}".to_owned()])[0], "add a text layer saying {text}");
        // Several values, by quotes or commas.
        let card = Recipe::new("card {name} {role}", ["add a text layer saying {name}", "add a text layer saying {role}"]);
        let known = [card];
        assert_eq!(read("card Jane Doe, Director", &known), Some(Said::Run { index: 0, values: vec!["Jane Doe".to_owned(), "Director".to_owned()] }));
        assert_eq!(read("card “Doe, Jane” “Head, Sales”", &known), Some(Said::Run { index: 0, values: vec!["Doe, Jane".to_owned(), "Head, Sales".to_owned()] }));
        assert!(matches!(read("card Jane", &known), Some(Said::Problem(_))));
    }

    #[test]
    fn values_must_be_named_and_used_sensibly() {
        for message in [
            "teach lower third {text}: add a title",
            "teach lower third: add a text layer saying {text}",
            "teach {text} third: fade in",
            "teach make {thing}: add {thing}",
            "teach card {a} {a}: say {a}",
            "/teach lower third {text}",
        ] {
            assert!(matches!(read(message, &[]), Some(Said::Problem(_))), "{message}");
        }
    }

    #[test]
    fn shortcuts_buttons_and_moments_are_set_by_word() {
        let known = [intro(), Recipe::new("tidy up", ["fix everything"])];
        let key = |key: &str| Some(Said::SetKey { name: "tidy up".to_owned(), key: Some(key.to_owned()) });
        assert_eq!(read("give tidy up the shortcut cmd shift t", &known), key("shift-cmd-t"));
        assert_eq!(read("bind tidy up to ⌘⇧T", &known), key("shift-cmd-t"));
        assert_eq!(read("set the shortcut for tidy up to control option 7", &known), key("ctrl-alt-7"));
        assert_eq!(read("remove the shortcut from tidy up", &known), Some(Said::SetKey { name: "tidy up".to_owned(), key: None }));
        assert!(matches!(read("give tidy up the shortcut T", &known), Some(Said::Problem(_))), "a letter alone");
        assert!(matches!(read("give lunch the shortcut cmd l", &known), Some(Said::Problem(_))), "no such recipe");
        assert_eq!(read("put tidy up on the toolbar", &known), Some(Said::SetButton { name: "tidy up".to_owned(), on: true }));
        assert_eq!(read("take intro off the toolbar", &known), Some(Said::SetButton { name: "intro".to_owned(), on: false }));
        assert_eq!(read("always run tidy up before export", &known), Some(Said::SetTrigger { name: "tidy up".to_owned(), trigger: Trigger::BeforeExport, on: true }));
        assert_eq!(read("run intro when I open a project", &known), Some(Said::SetTrigger { name: "intro".to_owned(), trigger: Trigger::OnOpen, on: true }));
        assert_eq!(read("stop running tidy up before saving", &known), Some(Said::SetTrigger { name: "tidy up".to_owned(), trigger: Trigger::BeforeSave, on: false }));
        // Not about recipes.
        assert_eq!(read("put the logo on the left", &known), None);
        assert_eq!(read("give it a drop shadow", &known), None);
        assert_eq!(key_label("ctrl-alt-shift-cmd-k"), "⌃⌥⇧⌘K");
    }

    #[test]
    fn a_recipe_can_be_described_without_running_it() {
        let known = [intro(), lower_third()];
        assert_eq!(read("what would intro do?", &known), Some(Said::Describe { index: 0, values: Vec::new() }));
        assert_eq!(read("preview lower third Jane", &known), Some(Said::Describe { index: 1, values: vec!["Jane".to_owned()] }));
        assert_eq!(read("what does the intro recipe do", &known), Some(Said::Describe { index: 0, values: Vec::new() }));
        assert_eq!(read("what would you do", &known), None);
    }

    #[test]
    fn recipes_are_shared_as_a_file_for_the_same_app_only() {
        let mut tidy = Recipe::new("tidy up", ["fix everything"]);
        tidy.key = Some("shift-cmd-t".to_owned());
        tidy.when = vec![Trigger::BeforeExport];
        let text = share_text(App::Studio, &[tidy.clone(), lower_third()]);
        assert_eq!(from_share_text(&text, App::Studio), Ok(vec![tidy, lower_third()]));
        assert!(from_share_text(&text, App::Develop).unwrap_err().contains("Omina Motion"));
        assert!(from_share_text("{}", App::Studio).is_err());
        let bad = share_text(App::Studio, &[Recipe::new("undo", ["fade in"])]);
        assert!(from_share_text(&bad, App::Studio).is_err());
    }

    #[test]
    fn a_recipe_is_taught_in_several_wordings() {
        let taught = Some(Said::Teach(intro()));
        assert_eq!(read("teach intro: fade in over a second, then pulse", &[]), taught);
        assert_eq!(read("/teach Intro = fade in over a second; pulse", &[]), taught);
        assert_eq!(read("When I say intro, fade in over a second then pulse", &[]), taught);
        assert_eq!(read("save that as Intro", &[]), Some(Said::TeachLast("intro".to_owned())));
        assert_eq!(read("/teach intro", &[]), Some(Said::TeachLast("intro".to_owned())));
    }

    #[test]
    fn names_that_wont_do_are_explained() {
        assert!(matches!(read("teach : fade in", &[]), Some(Said::Problem(_))));
        assert!(matches!(read("teach undo: fade in", &[]), Some(Said::Problem(_))));
        assert!(matches!(read("teach a very long name for one recipe: fade in", &[]), Some(Said::Problem(_))));
        assert!(matches!(read("teach intro fade in", &[]), Some(Said::Problem(_))));
        assert!(matches!(read("teach", &[]), Some(Said::Problem(_))));
    }

    #[test]
    fn recipes_run_by_name_and_are_listed_and_forgotten() {
        let known = [intro()];
        let run = Some(Said::Run { index: 0, values: Vec::new() });
        for said in ["intro", "/intro", "Do intro", "run the intro recipe", "Intro."] {
            assert_eq!(read(said, &known), run, "{said}");
        }
        let two_words = [Recipe::new("brand it", ["center it"])];
        assert_eq!(read("/brand-it", &two_words), run);
        assert_eq!(read("brand it", &two_words), run);
        assert_eq!(read("intro now", &known), None, "a recipe without values is called by its name alone");
        assert_eq!(read("/recipes", &known), Some(Said::List));
        assert_eq!(read("What have I taught you?", &known), Some(Said::List));
        assert_eq!(read("forget intro", &known), Some(Said::Forget("intro".to_owned())));
        assert_eq!(read("forget it", &known), None);
        assert_eq!(listed(&known), "• intro: fade in over a second → pulse");
    }

    #[test]
    fn other_messages_are_left_alone() {
        let known = [intro()];
        assert_eq!(read("teach me how to crop", &known), None);
        assert_eq!(read("save this as a PNG", &known), None);
        assert_eq!(read("fade in", &known), None);
        assert_eq!(read("make an intro", &known), None);
        assert_eq!(read("/help", &known), None);
    }

    #[test]
    fn each_app_keeps_its_own_recipes_in_one_file() {
        let folder = std::env::temp_dir().join(format!("iris-recipes-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        assert!(load(&folder, App::Studio).is_empty(), "nothing taught yet");
        save(&folder, App::Studio, &[intro()]).unwrap();
        let look = Recipe::new("moody", ["darker"]);
        save(&folder, App::Develop, std::slice::from_ref(&look)).unwrap();
        assert_eq!(load(&folder, App::Studio), vec![intro()]);
        assert_eq!(load(&folder, App::Develop), vec![look]);
        assert!(load(&folder, App::Canvas).is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
