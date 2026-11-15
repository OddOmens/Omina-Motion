//! Iris beyond the conversation itself: recipes run from a shortcut, a button or a moment
//! (before an export, after opening, before a save, after an import); help with typing in the
//! chat field (completing a command or recipe, and going back through what was typed with the
//! arrow keys); keeping the conversation for next time; and hand-offs between Motion and
//! Iris's other apps, Omina Edit and Omina Develop.
//!
//! What is shared with the other apps (the recipes, the history, the saved conversation, the
//! hand-off folders) is kept by the `iris` crate in Iris's shared folder; this is Motion's side
//! of it. Each recipe still runs through the same front door and readers as a typed message
//! (see `assistant`), so a recipe does exactly what its steps would do typed one by one.

use gpui_kit::component::button::Button;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::{AnyElement, Context, InteractiveElement, StatefulInteractiveElement, IntoElement, ParentElement, Styled, TestSupportExt as _, Window, div, px};
use iris::App;
use iris::recipes::Trigger;
use studio_core::model::Id;

use super::assistant::{Message, shared_folder};
use super::assistant_chores::Done;
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The most completions shown under the chat field at once.
const MOST_COMPLETIONS: usize = 5;

// Recipes run by a key, a button or a moment ----------------------------------------------------

/// Saves the recipes and binds their shortcuts again, after anything about them changed: a
/// recipe taught or forgotten, a key, button or moment set, or a change on Settings ▸ Recipes.
pub fn recipes_changed(workspace: &mut Workspace, cx: &mut gpui_kit::App) {
    let memory = super::assistant::memory(workspace);
    let (taught, keys) = (memory.taught.clone(), memory.keyed());
    if let Some(folder) = shared_folder()
        && let Err(error) = iris::recipes::save(&folder, App::Studio, &taught)
    {
        eprintln!("could not save Iris's recipes: {error}");
    }
    crate::keymap::set_recipe_keys(&workspace.ui.settings, keys, cx);
}

/// Gets Iris ready when the editor opens: her memory with the recipes taught so far, so their
/// shortcuts work before her chat is ever opened.
pub fn start(workspace: &mut Workspace, cx: &mut gpui_kit::App) {
    let keys = super::assistant::memory(workspace).keyed();
    crate::keymap::set_recipe_keys(&workspace.ui.settings, keys, cx);
}

/// What came of running a recipe by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ran {
    /// How many of its steps changed the project.
    pub changes: usize,
    /// Whether a step failed or wasn't understood, so the steps after it weren't run.
    pub failed: bool,
}

/// Runs the recipe called `name` as if it had been asked for by name, saying in the chat what
/// ran it (`how`: "its shortcut ⌘⇧T", "its button", "before the export"). Its changes are one
/// step for Undo, unless it runs inside another request, which records them with its own.
pub fn run_recipe(workspace: &mut Workspace, name: &str, how: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Ran {
    if super::assistant::memory(workspace).recipe(name).is_none() {
        return Ran { changes: 0, failed: true };
    }
    workspace.ui.assistant.messages.push(Message::from_assistant(format!("Running “{name}”, from {how}.")));
    let ran = super::assistant::answer_by_itself(workspace, |workspace, cx| {
        let units = super::assistant::units(workspace);
        let reserved = super::assistant::reserved();
        let reading = iris::front::run(name, super::assistant::memory(workspace), &units, &reserved);
        if reading.recipes_changed {
            recipes_changed(workspace, cx);
        }
        reading.turns
    }, window, cx);
    remember(workspace);
    ran
}

/// Runs a recipe from its shortcut: the `RunRecipe` action the keymap binds.
pub fn run_from_key(workspace: &mut Workspace, name: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let key = super::assistant::memory(workspace).recipe(name).and_then(|recipe| recipe.key).map_or_else(|| "its shortcut".to_owned(), |key| format!("its shortcut {}", iris::recipes::key_label(&key)));
    run_recipe(workspace, name, &key, window, cx);
    cx.notify();
}

/// Runs the recipes set to run by themselves at `trigger`, one after another. A failed step is
/// said in the chat and stops that recipe only; whatever the moment was for (an export, a save)
/// still goes ahead. While they run no moment sets off recipes again, so a recipe run before an
/// export that itself exports doesn't run itself in a loop.
pub fn at(workspace: &mut Workspace, trigger: Trigger, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.assistant.triggering {
        return;
    }
    let names = super::assistant::memory(workspace).triggered(trigger);
    if names.is_empty() {
        return;
    }
    // What happened just before (an import, an open) is its own step for Undo.
    if !workspace.ui.assistant.is_answering() {
        workspace.state.record_history(false);
    }
    workspace.ui.assistant.triggering = true;
    let goes_ahead = match trigger {
        Trigger::BeforeExport => Some("The export goes ahead all the same."),
        Trigger::BeforeSave => Some("The save goes ahead all the same."),
        Trigger::OnOpen | Trigger::AfterImport => None,
    };
    for name in names {
        let ran = run_recipe(workspace, &name, trigger.phrase(), window, cx);
        if ran.failed && let Some(note) = goes_ahead {
            workspace.ui.assistant.messages.push(Message::from_assistant(format!("“{name}” didn't finish. {note}")));
            remember(workspace);
        }
    }
    workspace.ui.assistant.triggering = false;
    cx.notify();
}

/// The strip of buttons at the top of the chat, one for each recipe given a button. `None`
/// when there are none.
pub fn recipe_buttons(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let names = workspace.ui.assistant.memory.as_ref()?.buttons();
    if names.is_empty() {
        return None;
    }
    let buttons: Vec<_> = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let label = name.clone();
            Button::new(("assistant-recipe-button", index)).outline().xsmall().label(label).tooltip(format!("Run the recipe “{name}”")).on_click(cx.listener(move |this, _, window, cx| {
                this.ui.assistant.chat_chosen = Some(true);
                run_recipe(this, &name, "its button", window, cx);
                cx.notify();
            }))
        })
        .collect();
    Some(div().id("assistant-recipe-buttons").px_2().flex().flex_wrap().gap_1().children(buttons).test_support().into_any_element())
}

// Settings ▸ Recipes ------------------------------------------------------------------------------

/// The recipe called `name`, ready to change: a taught one, or a copy of one Motion comes with
/// (which keeps only its shortcut, button and moments; its steps are always Motion's own).
/// `None` when there is no such recipe.
fn adjustable<'a>(memory: &'a mut iris::front::Memory, name: &str) -> Option<&'a mut iris::recipes::Recipe> {
    if !memory.taught.iter().any(|recipe| recipe.name == name) {
        let own = memory.built_in.iter().find(|own| own.name == name).cloned()?;
        memory.taught.push(own);
    }
    memory.taught.iter_mut().find(|recipe| recipe.name == name)
}

/// Whether `name` is a recipe Motion comes with.
pub fn is_built_in(workspace: &mut Workspace, name: &str) -> bool {
    super::assistant::memory(workspace).built_in.iter().any(|own| own.name == name)
}

/// Changes the recipe called `name`: its name and steps (joined with "then" or a semicolon, as
/// in a typed message, or one a line), for a taught recipe,
/// and its shortcut, typed as Iris reads one ("⌘⇧T", "command shift T"; empty for none). Saves
/// and binds the shortcut. Returns what to say, or why it won't do (and then nothing changed).
pub fn edit_recipe(workspace: &mut Workspace, name: &str, new_name: &str, steps: &str, key: &str, cx: &mut gpui_kit::App) -> Result<String, String> {
    let built_in = is_built_in(workspace, name);
    let memory = super::assistant::memory(workspace);
    let renamed = if built_in { name.to_owned() } else { iris::recipes::tidy_name(new_name)? };
    if renamed != name && memory.recipe(&renamed).is_some() {
        return Err(format!("There is already a recipe called “{renamed}”."));
    }
    let lines: Vec<String> = steps.lines().flat_map(iris::script::steps).collect();
    if !built_in && lines.is_empty() {
        return Err("A recipe needs at least one step: put each on a line of its own.".to_owned());
    }
    let key = match key.trim() {
        "" => None,
        typed => Some(iris::recipes::normalize_key(typed)?),
    };
    if let Some(key) = &key {
        if memory.app_keys.contains(key) {
            return Err(format!("{} is one of Omina Motion's own shortcuts. Pick another.", iris::recipes::key_label(key)));
        }
        if let Some((_, other)) = memory.keyed().into_iter().find(|(used, other)| used == key && *other != name) {
            return Err(format!("{} already runs “{other}”. Pick another, or take it from that one first.", iris::recipes::key_label(key)));
        }
    }
    let recipe = adjustable(memory, name).ok_or_else(|| format!("There is no recipe called “{name}” any more."))?;
    if !built_in {
        recipe.name = renamed.clone();
        recipe.steps = lines;
    }
    recipe.key = key.clone();
    recipes_changed(workspace, cx);
    let shortcut = key.map_or_else(|| "no shortcut".to_owned(), |key| format!("the shortcut {}", iris::recipes::key_label(&key)));
    Ok(format!("Saved “{renamed}”, with {shortcut}."))
}

/// Gives the recipe called `name` a button at the top of the chat, or takes it away.
pub fn set_button(workspace: &mut Workspace, name: &str, on: bool, cx: &mut gpui_kit::App) {
    if let Some(recipe) = adjustable(super::assistant::memory(workspace), name) {
        recipe.button = on;
        recipes_changed(workspace, cx);
    }
}

/// Makes the recipe called `name` run by itself at `trigger`, or stop doing so.
pub fn set_trigger(workspace: &mut Workspace, name: &str, trigger: Trigger, on: bool, cx: &mut gpui_kit::App) {
    if let Some(recipe) = adjustable(super::assistant::memory(workspace), name) {
        recipe.when.retain(|when| *when != trigger);
        if on {
            recipe.when.push(trigger);
            recipe.when.sort_by_key(|when| Trigger::ALL.iter().position(|known| known == when));
        }
        recipes_changed(workspace, cx);
    }
}

/// Moves the taught recipe called `name` one place up (earlier) or down the list.
pub fn move_recipe(workspace: &mut Workspace, name: &str, up: bool, cx: &mut gpui_kit::App) {
    let taught = &mut super::assistant::memory(workspace).taught;
    let Some(at) = taught.iter().position(|recipe| recipe.name == name) else { return };
    let to = if up { at.checked_sub(1) } else { Some(at + 1).filter(|to| *to < taught.len()) };
    if let Some(to) = to {
        taught.swap(at, to);
        recipes_changed(workspace, cx);
    }
}

/// Deletes the taught recipe called `name` (for one Motion comes with, forgets its shortcut,
/// button and moments). Returns it with its place, for Undo.
pub fn delete_recipe(workspace: &mut Workspace, name: &str, cx: &mut gpui_kit::App) -> Option<(usize, iris::recipes::Recipe)> {
    let taught = &mut super::assistant::memory(workspace).taught;
    let at = taught.iter().position(|recipe| recipe.name == name)?;
    let removed = taught.remove(at);
    recipes_changed(workspace, cx);
    Some((at, removed))
}

/// Puts back a recipe deleted with `delete_recipe`, where it was.
pub fn undelete_recipe(workspace: &mut Workspace, at: usize, recipe: iris::recipes::Recipe, cx: &mut gpui_kit::App) {
    let taught = &mut super::assistant::memory(workspace).taught;
    if taught.iter().any(|kept| kept.name == recipe.name) {
        return;
    }
    taught.insert(at.min(taught.len()), recipe);
    recipes_changed(workspace, cx);
}

/// Every recipe, taught and Motion's own, written out as a file to keep or give to someone.
pub fn recipes_file(workspace: &mut Workspace) -> String {
    let known = super::assistant::memory(workspace).known();
    iris::recipes::share_text(App::Studio, &known)
}

/// Adds the recipes in a file written by `recipes_file` (here or on another Mac). Returns what
/// was added, or why the file won't do.
pub fn import_recipes(workspace: &mut Workspace, text: &str, cx: &mut gpui_kit::App) -> Result<String, String> {
    let incoming = iris::recipes::from_share_text(text, App::Studio)?;
    let said = super::assistant::memory(workspace).import(incoming);
    recipes_changed(workspace, cx);
    Ok(said)
}

// Typing in the chat field ----------------------------------------------------------------------

/// Notes what the chat field holds now: what it could become is listed under it, and a recalled
/// message that was edited is no longer the one recalled.
pub fn typed(workspace: &mut Workspace, text: &str) {
    let assistant = &mut workspace.ui.assistant;
    if assistant.recalled.as_deref() != Some(text) {
        assistant.recalled = None;
    }
    let recipes = assistant.memory.as_ref().map(iris::front::Memory::known).unwrap_or_default();
    assistant.completions = if assistant.recalled.is_some() { Vec::new() } else { iris::complete::suggestions(text, studio_core::chat::COMMANDS, &recipes, MOST_COMPLETIONS) };
}

/// Puts `text` in the chat field, with the cursor at its end.
fn set_field(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some((input, _)) = workspace.ui.assistant.ask.as_ref() else { return };
    let input = input.clone();
    input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
}

/// The up arrow (`older`) or down arrow in the chat field: goes back or forward through what was
/// typed before, but only when the field is empty or shows a message recalled this way, so the
/// arrows still move the cursor in text being written. Returns whether it took the key.
pub fn recall(workspace: &mut Workspace, older: bool, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    let Some((input, _)) = workspace.ui.assistant.ask.as_ref() else { return false };
    let current = input.read(cx).value().to_string();
    let assistant = &mut workspace.ui.assistant;
    let ours = current.is_empty() || assistant.recalled.as_deref() == Some(current.as_str());
    if !ours {
        return false;
    }
    let Some(text) = (if older { assistant.recall.older(&current) } else { assistant.recall.newer() }) else { return false };
    assistant.completions.clear();
    set_field(workspace, &text, window, cx);
    // The draft given back on coming forward past the newest isn't a recalled message.
    let assistant = &mut workspace.ui.assistant;
    assistant.recalled = (!text.is_empty() && assistant.recall.entries().contains(&text)).then(|| text.clone());
    cx.notify();
    true
}

/// Tab in the chat field: takes the first completion shown. Returns whether it took the key.
pub fn complete_first(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    let Some(first) = workspace.ui.assistant.completions.first().map(|completion| completion.text.clone()) else { return false };
    take_completion(workspace, &first, window, cx);
    true
}

/// Puts a completion in the chat field, ready to send or to finish typing.
fn take_completion(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    set_field(workspace, text, window, cx);
    workspace.ui.assistant.completions.clear();
    cx.notify();
}

/// The completions for what is typed, drawn just above the chat field. `None` when there are none.
pub fn completions(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let shown = &workspace.ui.assistant.completions;
    if shown.is_empty() {
        return None;
    }
    let rows: Vec<_> = shown
        .iter()
        .enumerate()
        .map(|(index, completion)| {
            let text = completion.text.clone();
            div()
                .id(("assistant-completion", index))
                .px_2()
                .py_1()
                .flex()
                .gap_2()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|style| style.bg(hsla(color::HOVER)))
                .child(div().flex_none().text_color(hsla(color::TEXT)).child(completion.text.trim_end().to_owned()))
                .child(div().flex_1().min_w_0().truncate().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(completion.about.clone()))
                .on_click(cx.listener(move |this, _, window, cx| take_completion(this, &text, window, cx)))
                .test_support()
        })
        .collect();
    let hint = div().px_2().text_size(px(10.5)).text_color(hsla(color::MUTED)).child("Tab takes the first");
    Some(div().id("assistant-completions").mx_2().p_1().flex().flex_col().rounded(px(8.0)).bg(hsla(color::RAISED)).border_1().border_color(hsla(color::HAIRLINE)).children(rows).child(hint).test_support().into_any_element())
}

// Keeping the conversation ------------------------------------------------------------------------

/// Brings back the conversation and what was typed from last time, the first time the chat is
/// built. The messages brought back are drawn above an "Earlier" line.
pub fn restore(workspace: &mut Workspace) {
    let Some(folder) = shared_folder() else { return };
    let saved = iris::conversation::load(&folder, App::Studio);
    let assistant = &mut workspace.ui.assistant;
    assistant.recall = iris::complete::Recall::new(saved.typed);
    let restored: Vec<Message> = saved.lines.into_iter().map(|line| Message { from_person: line.from_person, ..Message::from_assistant(line.text) }).collect();
    assistant.restored = restored.len();
    assistant.messages.splice(0..0, restored);
}

/// Keeps the conversation (its words) and what was typed, for next time.
pub fn remember(workspace: &Workspace) {
    let Some(folder) = shared_folder() else { return };
    let assistant = &workspace.ui.assistant;
    let lines = assistant.messages.iter().map(|message| iris::conversation::Line { from_person: message.from_person, text: message.text.clone() }).collect();
    let saved = iris::conversation::Saved { lines, typed: assistant.recall.entries().to_vec() };
    if let Err(error) = iris::conversation::save(&folder, App::Studio, &saved) {
        eprintln!("could not keep Iris's conversation: {error}");
    }
}

/// Forgets the kept conversation, for /clear. What was typed is kept for the arrow keys.
pub fn forget(workspace: &mut Workspace) {
    workspace.ui.assistant.restored = 0;
    if let Some(folder) = shared_folder()
        && let Err(error) = iris::conversation::clear(&folder, App::Studio)
    {
        eprintln!("could not clear Iris's conversation: {error}");
    }
}

/// The quiet line drawn between the messages brought back from last time and the new ones.
pub fn earlier_divider() -> AnyElement {
    let line = || div().flex_1().h(px(1.0)).bg(hsla(color::HAIRLINE));
    div().id("assistant-earlier").px_3().flex().items_center().gap_2().text_size(px(10.5)).text_color(hsla(color::MUTED)).child(line()).child("Earlier").child(line()).test_support().into_any_element()
}

// Hand-offs between the apps ----------------------------------------------------------------------

/// Saves the frame under the playhead as a picture and sends it to `to` (Omina Edit or Omina
/// Develop), then opens that app. Says in the chat what was sent once it is, or why not.
pub fn send_frame(workspace: &mut Workspace, to: App, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    if to == App::Studio {
        return Done::said("This frame is already in Omina Motion. To send it on, say “send this frame to Edit” or “send this frame to Develop”.");
    }
    if !workspace.has(crate::features::Feature::Export) {
        return super::assistant_chores::turned_off(crate::features::Feature::Export);
    }
    let Some(root) = shared_folder() else { return Done::failed("There is no home folder to hand the frame over through.") };
    let (frame, comp) = (workspace.state.current_frame, workspace.state.composition().name.clone());
    let title = format!("frame {frame} of “{comp}”");
    let sending = title.clone();
    let started = super::export::save_frame_then(workspace, window, cx, move |this, path, saved, window, cx| {
        let said = match saved.and_then(|()| iris::handoff::send(&root, App::Studio, to, &sending, &[path.to_path_buf()])) {
            Ok(_) => match open(to) {
                Ok(()) => format!("Sent {sending} to {}, and opened it there.", to.name()),
                Err(why) => format!("Sent {sending} to {}. {why}", to.name()),
            },
            Err(why) => format!("I couldn't send {sending} to {}: {why}", to.name()),
        };
        this.ui.assistant.messages.push(Message::from_assistant(said));
        remember(this);
        super::assistant::show_newest(window, cx);
    });
    match started {
        Ok(_) => Done::did(format!("Saving {title} to send to {}. I'll say here once it is there.", to.name())),
        Err(why) => Done::failed(why),
    }
}

/// Opens `app`. Tests never start another app.
fn open(app: App) -> Result<(), String> {
    if cfg!(test) { Ok(()) } else { iris::handoff::open_app(app) }
}

/// Brings in what Iris's other apps sent to Motion: each package's files are imported into the
/// project and put on the open composition as layers, from the playhead, and the chat says so.
/// Waits while another import is running. Called when Motion starts and every few seconds.
pub fn bring_in_handoffs(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.import_job.is_some() || workspace.ui.svg_question.is_some() {
        return;
    }
    let Some(root) = shared_folder() else { return };
    let packages = iris::handoff::take_ready(&root, App::Studio);
    if packages.is_empty() {
        return;
    }
    let titles: Vec<String> = packages.iter().map(|package| format!("{} from {}", package.title, package.from.name())).collect();
    let files: Vec<std::path::PathBuf> = packages.into_iter().flat_map(|package| package.files).collect();
    let then: super::importing::Then = Box::new(move |this, items: Vec<Id>, window, cx| {
        let frame = this.state.current_frame;
        let center = this.state.composition().size() * 0.5;
        let placed = this.state.add_items_at(&items, center);
        let at = studio_core::time::format_timecode(frame, this.state.composition().frame_rate);
        let said = if placed {
            format!("Brought in {}: on {} as {} from {at}, and in the project. One Undo takes it out.", titles.join(" and "), this.state.composition().name, if items.len() == 1 { "a layer" } else { "layers" })
        } else {
            format!("Brought {} into the project, but it couldn't go on {}.", titles.join(" and "), this.state.composition().name)
        };
        window.push_notification(said.as_str(), cx);
        this.ui.assistant.messages.push(Message::from_assistant(said));
        remember(this);
        cx.notify();
    });
    super::importing::start(workspace, files, Some(then), window, cx);
}
