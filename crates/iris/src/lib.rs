//! Iris, the assistant that Omina Motion, Omina Edit and Omina Develop share. (In code the
//! apps keep their first names: Studio, Canvas and Develop.)
//!
//! Each app links this crate and draws Iris its own way, but these parts are the same in all
//! of them, so a fix or a new answer made here reaches every app:
//!
//! - `front`: the front door every message comes through first. It splits a message into its
//!   steps, runs recipes, answers sums, and repeats the last change for "again";
//! - `script`: several requests in one message ("fade in, then pulse");
//! - `followup`: short replies that refer back ("again", "more", "too much", "undo that");
//! - `recipes`: things Iris is taught to do by name, kept for each app, which can take values
//!   and be run from a shortcut, a button or a moment such as "before export";
//! - `complete`: finishing half-typed commands and recipes, and the arrow keys' history;
//! - `conversation`: keeping each app's conversation when it closes;
//! - `export_presets`: export choices kept by name ("export for Instagram");
//! - `handoff`: sending files from one app to another ("open this photo in Omina Edit");
//! - `calc`: sums and conversions ("5 seconds in frames", "8 inches in pixels");
//! - `intent`: instructions about timing and motion typed in everyday words ("start at 5s,
//!   last 2 seconds and fade in") read into steps each app carries out its own way;
//! - `layers`: everyday housekeeping for layers read the same way ("hide the logo", "bring
//!   it to the front", "make it red");
//! - `commands`: finding one of an app's own commands by its name ("export", "show rulers");
//! - `knowledge`: the ready-written help, as data tagged with the apps it belongs to, so Iris
//!   can answer a question about another app and only offers to *do* things in the app she is
//!   in;
//! - `matching`: how a typed question finds its answers;
//! - `spelling`: putting mistyped words right, as a second try when nothing was understood;
//! - `settings`: her name and manner, kept in one file every app reads, so renaming her in one
//!   app renames her in all of them.
//!
//! Nothing here is AI: answers are written by hand, requests are read with fixed rules, and a
//! message that fits none of them is said to be not understood.
//!
//! Apps outside this repository use it as a path dependency:
//! `iris = { path = "../Omina Motion/crates/iris" }`.

pub mod calc;
pub mod commands;
pub mod complete;
pub mod conversation;
pub mod export_presets;
pub mod followup;
pub mod handoff;
pub mod front;
pub mod intent;
pub mod knowledge;
pub mod layers;
pub mod matching;
pub mod recipes;
pub mod script;
pub mod settings;
pub mod spelling;

/// The apps Iris lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum App {
    /// Motion and video.
    Studio,
    /// Designs and layouts.
    Canvas,
    /// Photo editing.
    Develop,
}

impl App {
    pub const ALL: [App; 3] = [App::Studio, App::Canvas, App::Develop];

    /// The app's name as people see it.
    pub fn name(self) -> &'static str {
        match self {
            App::Studio => "Omina Motion",
            App::Canvas => "Omina Edit",
            App::Develop => "Omina Develop",
        }
    }
}
