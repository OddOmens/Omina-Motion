//! Keyboard shortcuts: every command that can have one, the three starting sets (Studio's own,
//! After Effects' and Premiere Pro's, for people switching over), and the person's own changes
//! on top (Settings ▸ Keyboard Shortcuts).
//!
//! GPUI can only clear every key binding at once, including the ones its text fields, menus
//! and dialogs need. So `init` keeps a copy of those, taken before Studio adds its own, and
//! `apply` rebuilds the whole keymap from that copy plus the shortcuts in use.

use std::collections::BTreeMap;

use gpui_kit::{Action, App, Global, KeyBinding, KeyBindingContextPredicate};
use serde::{Deserialize, Serialize};

use crate::workspace::*;

/// Where a shortcut works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    /// Anywhere, even with no window open (Quit, Hide).
    App,
    /// In the editor, including while typing in a field (⌘ shortcuts).
    Editor,
    /// In the editor but not while typing, so letters and Space still type.
    NotTyping,
}

/// One command that can have a shortcut.
pub struct Command {
    /// Saved in settings, so it never changes once released.
    pub id: &'static str,
    pub name: &'static str,
    /// The heading it is listed under in Settings.
    pub group: &'static str,
    scope: Scope,
    make: fn() -> Box<dyn Action>,
}

impl Command {
    /// The command's action, to bind or to show the shortcut of.
    pub fn action(&self) -> Box<dyn Action> {
        (self.make)()
    }
}

macro_rules! command {
    ($id:literal, $name:literal, $group:literal, $scope:ident, $action:expr) => {
        Command { id: $id, name: $name, group: $group, scope: Scope::$scope, make: || Box::new($action) }
    };
}

/// Every command, in the order Settings lists them.
pub static COMMANDS: &[Command] = &[
    command!("toggle-playback", "Play or pause", "Playback", NotTyping, TogglePlayback),
    command!("next-frame", "Next frame", "Playback", NotTyping, NextFrame),
    command!("previous-frame", "Previous frame", "Playback", NotTyping, PreviousFrame),
    command!("next-five-frames", "Five frames on", "Playback", NotTyping, NextFiveFrames),
    command!("previous-five-frames", "Five frames back", "Playback", NotTyping, PreviousFiveFrames),
    command!("next-ten-frames", "Ten frames on", "Playback", NotTyping, NextTenFrames),
    command!("previous-ten-frames", "Ten frames back", "Playback", NotTyping, PreviousTenFrames),
    command!("first-frame", "Go to the start", "Playback", NotTyping, FirstFrame),
    command!("last-frame", "Go to the end", "Playback", NotTyping, LastFrame),
    command!("toggle-marker", "Add or remove a marker", "Playback", NotTyping, ToggleMarker),
    command!("selection-tool", "Select", "Tools", NotTyping, SelectionTool),
    command!("track-select-tool", "Track Select Forward", "Tools", NotTyping, TrackSelectTool),
    command!("type-tool", "Text", "Tools", Editor, TypeTool),
    command!("shape-tool", "Shape", "Tools", NotTyping, ShapeTool),
    command!("pen-tool", "Pen", "Tools", NotTyping, PenTool),
    command!("finish-path", "Finish the pen path", "Tools", NotTyping, FinishPath),
    command!("rotate-tool", "Rotate", "Tools", NotTyping, RotateTool),
    command!("hand-tool", "Hand", "Tools", NotTyping, HandTool),
    command!("zoom-tool", "Zoom", "Tools", NotTyping, ZoomTool),
    command!("eyedropper-tool", "Eyedropper", "Tools", NotTyping, EyedropperTool),
    command!("cut-tool", "Cut (razor)", "Tools", NotTyping, CutTool),
    command!("ripple-tool", "Ripple Edit", "Tools", NotTyping, RippleTool),
    command!("roll-tool", "Rolling Edit", "Tools", NotTyping, RollTool),
    command!("slip-tool", "Slip", "Tools", NotTyping, SlipTool),
    command!("slide-tool", "Slide", "Tools", NotTyping, SlideTool),
    command!("rate-stretch-tool", "Rate Stretch", "Tools", NotTyping, RateStretchTool),
    command!("undo", "Undo", "Editing", Editor, Undo),
    command!("redo", "Redo", "Editing", Editor, Redo),
    command!("cut", "Cut", "Editing", NotTyping, Cut),
    command!("copy", "Copy", "Editing", NotTyping, Copy),
    command!("paste", "Paste", "Editing", NotTyping, Paste),
    command!("duplicate", "Duplicate", "Editing", NotTyping, Duplicate),
    command!("select-all", "Select all", "Editing", NotTyping, SelectAll),
    command!("delete", "Delete", "Editing", NotTyping, Delete),
    command!("ripple-delete", "Ripple delete", "Editing", NotTyping, RippleDelete),
    command!("split-layer", "Split at the playhead", "Editing", Editor, SplitLayer),
    command!("trim-in", "Trim the in point to the playhead", "Editing", NotTyping, TrimInToPlayhead),
    command!("trim-out", "Trim the out point to the playhead", "Editing", NotTyping, TrimOutToPlayhead),
    command!("precompose", "Pre-compose", "Editing", NotTyping, Precompose),
    command!("easy-ease", "Easy ease", "Editing", NotTyping, EasyEase),
    command!("toggle-snapping", "Snapping on or off", "Editing", NotTyping, ToggleSnapping),
    command!("new-solid", "New solid", "Layers", Editor, NewSolid),
    command!("new-adjustment-layer", "New adjustment layer", "Layers", Editor, NewAdjustmentLayer),
    command!("new-null", "New null object", "Layers", Editor, NewNull),
    command!("zoom-viewer-in", "Zoom the viewer in", "View", NotTyping, ZoomViewerIn),
    command!("zoom-viewer-out", "Zoom the viewer out", "View", NotTyping, ZoomViewerOut),
    command!("toggle-guides", "Show or hide guides", "View", NotTyping, ToggleGuides),
    command!("toggle-full-screen", "Full screen", "View", Editor, ToggleFullScreen),
    command!("dismiss", "Close a popup, or stop", "View", NotTyping, Dismiss),
    command!("new-project", "New project", "Project", Editor, NewProject),
    command!("new-composition", "New composition", "Project", Editor, NewComposition),
    command!("composition-settings", "Composition settings", "Project", Editor, CompositionSettings),
    command!("open", "Open", "Project", Editor, Open),
    command!("save", "Save", "Project", Editor, Save),
    command!("save-as", "Save as", "Project", Editor, SaveAs),
    command!("close-project", "Close project", "Project", Editor, CloseProject),
    command!("import", "Import", "Project", Editor, Import),
    command!("export", "Export", "Project", Editor, Export),
    command!("open-settings", "Settings", "Project", Editor, OpenSettings),
    command!("quit", "Quit Studio", "Project", App, Quit),
    command!("hide", "Hide Studio", "Project", App, HideStudio),
    command!("hide-others", "Hide other apps", "Project", App, HideOtherApps),
];

/// The command with this id.
pub fn command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

/// The set of shortcuts to start from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyPreset {
    /// Studio's own: After Effects' keys, plus Premiere's editing tools.
    #[default]
    Studio,
    AfterEffects,
    Premiere,
}

impl KeyPreset {
    pub const ALL: [Self; 3] = [Self::Studio, Self::AfterEffects, Self::Premiere];

    pub fn name(self) -> &'static str {
        match self {
            Self::Studio => "Studio",
            Self::AfterEffects => "After Effects",
            Self::Premiere => "Premiere Pro",
        }
    }
}

/// The shortcuts in use: a preset, and the commands given keys of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    pub preset: KeyPreset,
    /// Keys by command id, replacing the preset's for that command. An empty list leaves the
    /// command without a shortcut.
    pub custom: BTreeMap<String, Vec<String>>,
}

/// Studio's shortcuts: command id and key, one row per key.
const STUDIO: &[(&str, &str)] = &[
    ("toggle-playback", "space"),
    ("next-frame", "pagedown"),
    ("next-frame", "cmd-]"),
    ("previous-frame", "pageup"),
    ("previous-frame", "cmd-["),
    ("next-five-frames", "cmd-shift-]"),
    ("next-five-frames", "cmd-}"),
    ("previous-five-frames", "cmd-shift-["),
    ("previous-five-frames", "cmd-{"),
    ("next-ten-frames", "shift-pagedown"),
    ("previous-ten-frames", "shift-pageup"),
    ("first-frame", "home"),
    ("last-frame", "end"),
    ("toggle-marker", "m"),
    ("selection-tool", "v"),
    ("track-select-tool", "a"),
    ("type-tool", "cmd-t"),
    ("shape-tool", "q"),
    ("pen-tool", "g"),
    ("finish-path", "enter"),
    ("rotate-tool", "w"),
    ("hand-tool", "h"),
    ("zoom-tool", "z"),
    ("eyedropper-tool", "i"),
    ("cut-tool", "c"),
    ("ripple-tool", "b"),
    ("roll-tool", "n"),
    ("slip-tool", "y"),
    ("slide-tool", "u"),
    ("rate-stretch-tool", "r"),
    ("undo", "cmd-z"),
    ("redo", "cmd-shift-z"),
    ("cut", "cmd-x"),
    ("copy", "cmd-c"),
    ("paste", "cmd-v"),
    ("duplicate", "cmd-d"),
    ("select-all", "cmd-a"),
    ("delete", "delete"),
    ("delete", "backspace"),
    ("ripple-delete", "shift-delete"),
    ("ripple-delete", "shift-backspace"),
    ("split-layer", "cmd-shift-d"),
    ("trim-in", "alt-["),
    ("trim-out", "alt-]"),
    ("precompose", "cmd-shift-c"),
    ("easy-ease", "f9"),
    ("toggle-snapping", "s"),
    ("new-solid", "cmd-y"),
    ("new-adjustment-layer", "cmd-alt-y"),
    ("new-null", "cmd-alt-shift-y"),
    ("zoom-viewer-in", "cmd-="),
    ("zoom-viewer-in", "cmd-+"),
    ("zoom-viewer-out", "cmd--"),
    ("toggle-guides", "'"),
    ("toggle-full-screen", "ctrl-cmd-f"),
    ("dismiss", "escape"),
    ("new-project", "cmd-alt-n"),
    ("new-composition", "cmd-n"),
    ("composition-settings", "cmd-k"),
    ("open", "cmd-o"),
    ("save", "cmd-s"),
    ("save-as", "cmd-shift-s"),
    ("close-project", "cmd-w"),
    ("import", "cmd-i"),
    ("export", "cmd-m"),
    ("open-settings", "cmd-,"),
    ("quit", "cmd-q"),
    ("hide", "cmd-h"),
    ("hide-others", "cmd-alt-h"),
];

/// After Effects: Studio's keys without Premiere's single-letter editing tools (those letters
/// mean other things in After Effects), and its own keys for snapping and frame stepping.
const AFTER_EFFECTS_CHANGES: &[(&str, &[&str])] = &[
    ("track-select-tool", &[]),
    ("cut-tool", &[]),
    ("ripple-tool", &[]),
    ("roll-tool", &[]),
    ("slip-tool", &[]),
    ("slide-tool", &[]),
    ("rate-stretch-tool", &[]),
    ("eyedropper-tool", &[]),
    ("toggle-marker", &["*"]),
    ("toggle-snapping", &["cmd-shift-;"]),
    ("next-frame", &["pagedown", "cmd-right"]),
    ("previous-frame", &["pageup", "cmd-left"]),
];

/// Premiere Pro: arrows step frames, letters pick the editing tools, ⌘K adds an edit, Q and W
/// trim to the playhead, and T is the text tool.
const PREMIERE_CHANGES: &[(&str, &[&str])] = &[
    ("next-frame", &["right"]),
    ("previous-frame", &["left"]),
    ("next-five-frames", &["shift-right"]),
    ("previous-five-frames", &["shift-left"]),
    ("next-ten-frames", &[]),
    ("previous-ten-frames", &[]),
    ("type-tool", &["t"]),
    ("pen-tool", &["p"]),
    ("shape-tool", &[]),
    ("rotate-tool", &[]),
    ("split-layer", &["cmd-k"]),
    ("composition-settings", &[]),
    ("trim-in", &["q"]),
    ("trim-out", &["w"]),
    ("precompose", &[]),
    ("new-solid", &[]),
    ("new-adjustment-layer", &[]),
    ("new-null", &[]),
    ("zoom-viewer-in", &["="]),
    ("zoom-viewer-out", &["-"]),
];

/// Every command's keys in a preset, before the person's own changes.
pub fn preset_keys(preset: KeyPreset) -> BTreeMap<&'static str, Vec<String>> {
    let mut keys: BTreeMap<&'static str, Vec<String>> = COMMANDS.iter().map(|command| (command.id, Vec::new())).collect();
    for &(id, key) in STUDIO {
        keys.entry(id).or_default().push(key.to_owned());
    }
    let changes = match preset {
        KeyPreset::Studio => &[][..],
        KeyPreset::AfterEffects => AFTER_EFFECTS_CHANGES,
        KeyPreset::Premiere => PREMIERE_CHANGES,
    };
    for &(id, replacement) in changes {
        keys.insert(id, replacement.iter().map(|key| (*key).to_owned()).collect());
    }
    keys
}

/// Every command's keys with the person's changes applied.
pub fn keys_in_use(shortcuts: &Shortcuts) -> BTreeMap<&'static str, Vec<String>> {
    let mut keys = preset_keys(shortcuts.preset);
    for (id, custom) in &shortcuts.custom {
        if let Some(command) = command(id) {
            keys.insert(command.id, custom.clone());
        }
    }
    keys
}

/// The command that already uses `key`, other than `except`.
pub fn used_by(shortcuts: &Shortcuts, key: &str, except: &str) -> Option<&'static Command> {
    keys_in_use(shortcuts).into_iter().find(|(id, keys)| *id != except && keys.iter().any(|existing| existing == key)).and_then(|(id, _)| command(id))
}

/// Gives `id` the single shortcut `key` (or none, with `None`). Another command using the same
/// key loses it. Returns that command, if there was one.
pub fn assign(shortcuts: &mut Shortcuts, id: &str, key: Option<&str>) -> Option<&'static Command> {
    let taken = key.and_then(|key| used_by(shortcuts, key, id));
    if let (Some(other), Some(key)) = (taken, key) {
        let remaining: Vec<String> = keys_in_use(shortcuts).get(other.id).into_iter().flatten().filter(|existing| *existing != key).cloned().collect();
        shortcuts.custom.insert(other.id.to_owned(), remaining);
    }
    shortcuts.custom.insert(id.to_owned(), key.map(|key| vec![key.to_owned()]).unwrap_or_default());
    // A change that lands back on the preset's keys isn't a change.
    let preset = preset_keys(shortcuts.preset);
    shortcuts.custom.retain(|id, keys| preset.get(id.as_str()) != Some(keys));
    taken
}

/// The bindings GPUI's own components made at start-up (text fields, menus, dialogs).
struct BaseBindings(Vec<KeyBinding>);

impl Global for BaseBindings {}

/// Notes the components' bindings, then binds `shortcuts`. Call once, after `gpui_kit::init`.
pub fn init(shortcuts: &Shortcuts, cx: &mut App) {
    let base = cx.key_bindings().borrow().bindings().cloned().collect();
    cx.set_global(BaseBindings(base));
    apply(shortcuts, cx);
}

/// Rebuilds the keymap: the components' bindings, then `shortcuts`. Keys that can't be read are
/// skipped. The menu bar is rebuilt too, as it shows the shortcuts.
pub fn apply(shortcuts: &Shortcuts, cx: &mut App) {
    let base = cx.try_global::<BaseBindings>().map(|base| base.0.clone()).unwrap_or_default();
    let not_typing = KeyBindingContextPredicate::parse(NOT_TYPING).ok().map(std::rc::Rc::new);
    let editor = KeyBindingContextPredicate::parse(KEY_CONTEXT).ok().map(std::rc::Rc::new);
    let mut bindings = Vec::new();
    for (id, keys) in keys_in_use(shortcuts) {
        let Some(command) = command(id) else { continue };
        let context = match command.scope {
            Scope::App => None,
            Scope::Editor => editor.clone(),
            Scope::NotTyping => not_typing.clone(),
        };
        for key in keys {
            if let Ok(binding) = KeyBinding::load(&key, command.action(), context.clone(), false, None, cx) {
                bindings.push(binding);
            }
        }
    }
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(bindings);
    crate::workspace::refresh_menus(cx);
}

/// The first key a command has now, as the Mac writes it, or `None` if it has none.
pub fn key_for(shortcuts: &Shortcuts, id: &str) -> Option<String> {
    keys_in_use(shortcuts).get(id).and_then(|keys| keys.first()).map(|key| display(key))
}

/// A button or menu label with the command's current key after it ("Pen (G)"), or the name
/// alone when the command has no key. Follows the person's own shortcuts.
pub fn with_key(shortcuts: &Shortcuts, name: &str, id: &str) -> String {
    match key_for(shortcuts, id) {
        Some(key) => format!("{name} ({key})"),
        None => name.to_owned(),
    }
}

/// A key as the Mac writes it: modifiers as symbols in their usual order, then the key
/// ("cmd-shift-d" reads "⇧⌘D").
pub fn display(key: &str) -> String {
    // The key comes last; a minus key leaves a trailing "-" ("cmd--" is ⌘ and minus).
    let (modifiers, name) = match key.strip_suffix('-') {
        Some(rest) => (rest.strip_suffix('-').unwrap_or(rest), "-"),
        None => key.rsplit_once('-').unwrap_or(("", key)),
    };
    let modifiers: Vec<&str> = modifiers.split('-').collect();
    let mut out = String::new();
    for (modifier, symbol) in [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")] {
        if modifiers.contains(&modifier) {
            out.push_str(symbol);
        }
    }
    let named = match name {
        "space" => "Space".to_owned(),
        "enter" => "↩".to_owned(),
        "escape" => "Esc".to_owned(),
        "backspace" => "⌫".to_owned(),
        "delete" => "⌦".to_owned(),
        "tab" => "⇥".to_owned(),
        "pageup" => "Page Up".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "home" => "Home".to_owned(),
        "end" => "End".to_owned(),
        "left" => "←".to_owned(),
        "right" => "→".to_owned(),
        "up" => "↑".to_owned(),
        "down" => "↓".to_owned(),
        other => other.to_uppercase(),
    };
    out.push_str(&named);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_key_names_a_command() {
        for &(id, _) in STUDIO {
            assert!(command(id).is_some(), "{id}");
        }
        for (id, _) in AFTER_EFFECTS_CHANGES.iter().chain(PREMIERE_CHANGES) {
            assert!(command(id).is_some(), "{id}");
        }
        let ids: std::collections::HashSet<&str> = COMMANDS.iter().map(|command| command.id).collect();
        assert_eq!(ids.len(), COMMANDS.len(), "ids are unique");
    }

    #[test]
    fn no_preset_gives_one_key_to_two_commands() {
        for preset in KeyPreset::ALL {
            let mut seen = std::collections::HashMap::new();
            for (id, keys) in preset_keys(preset) {
                for key in keys {
                    assert!(seen.insert(key.clone(), id).is_none(), "{} uses {key} twice", preset.name());
                }
            }
        }
    }

    #[test]
    fn premiere_and_after_effects_differ_where_they_should() {
        let premiere = preset_keys(KeyPreset::Premiere);
        assert_eq!(premiere["split-layer"], vec!["cmd-k"]);
        assert_eq!(premiere["next-frame"], vec!["right"]);
        assert_eq!(premiere["type-tool"], vec!["t"]);
        let after_effects = preset_keys(KeyPreset::AfterEffects);
        assert!(after_effects["ripple-tool"].is_empty());
        assert_eq!(after_effects["composition-settings"], vec!["cmd-k"]);
    }

    #[test]
    fn assigning_a_taken_key_moves_it() {
        let mut shortcuts = Shortcuts::default();
        // V is Select; give it to the pen.
        let moved_from = assign(&mut shortcuts, "pen-tool", Some("v"));
        assert_eq!(moved_from.map(|command| command.id), Some("selection-tool"));
        let keys = keys_in_use(&shortcuts);
        assert_eq!(keys["pen-tool"], vec!["v"]);
        assert!(keys["selection-tool"].is_empty());
        // Back to the preset's key: no longer a change.
        assign(&mut shortcuts, "pen-tool", Some("g"));
        assert!(!shortcuts.custom.contains_key("pen-tool"));
        assign(&mut shortcuts, "pen-tool", None);
        assert!(keys_in_use(&shortcuts)["pen-tool"].is_empty());
    }

    #[test]
    fn keys_read_the_mac_way() {
        assert_eq!(display("cmd-shift-d"), "⇧⌘D");
        assert_eq!(display("cmd-alt-shift-y"), "⌥⇧⌘Y");
        assert_eq!(display("space"), "Space");
        assert_eq!(display("cmd--"), "⌘-");
        assert_eq!(display("-"), "-");
        assert_eq!(display("shift-pagedown"), "⇧Page Down");
    }
}
