//! The Motion Assistant, opened from the rail: a list of things its rules noticed in the open
//! composition (its keyframes, clips, sound, text and pictures; see `studio_core::assistant`), each with a button that fixes it, a
//! button that shows it in the timeline, and a way to dismiss it. Nothing here is AI: every
//! suggestion comes from a plain rule, and the gear lists the rules so each can be switched off
//! or tuned. With tips on, the assistant's face also sits in a corner of the viewer with a
//! count of what it has noticed; a click opens the first suggestion there. The assistant has a
//! name the person can change, and its face blinks now and then.
//!
//! The panel has two tabs. Suggestions is the list above. Chat is a conversation: a message
//! typed at the foot of the panel is answered from the project itself (how long the
//! composition is, its layers and tracks, what is selected; see `studio_core::chat`), from
//! ready-written help answers (see `assistant_help`), or by a command such as `/clear`. A
//! click on the assistant's face gives a tip there.
//!
//! Every message comes through the front door Iris shares with the other apps
//! (`iris::front`) first, which splits it into its steps, runs recipes, answers sums and
//! repeats the last change for "again". Each step is then read by `studio_core::chat` and
//! carried out here: instructions and housekeeping for layers, Motion's own chores (see
//! `assistant_chores`), or one of the app's commands by its name. What can't be read is tried
//! once more with its mistyped words put right, and failing that Iris says so and offers the
//! nearest things she does know. An answer may carry short replies, drawn as buttons under
//! it, that send their words when clicked.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _};
use gpui_kit::{Animation, AnimationExt as _, AnyElement, AppContext as _, Context, ElementId, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, ScrollHandle, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _, Window, div, prelude::FluentBuilder as _, px};
use studio_core::assistant::{Area, Check, Options, Suggestion};
use studio_core::model::{Id, Layer, LayerContent, Project};

use super::assistant_chores::{Done, Outcome, Pointed, layers_of};
use super::chrome::panel;
use crate::app_settings::AppSettings;
use crate::features::Feature;
use crate::theme::{color, hsla, size};
use crate::workspace::{PanelId, Workspace};

// Iris's name rules are shared with the other apps (see the `iris` crate).
pub use iris::settings::{DEFAULT_NAME, tidy_name};
/// How long the face waits between blinks, used in turn so the blinking doesn't look mechanical.
#[cfg(not(test))]
pub const BLINK_PAUSES: [std::time::Duration; 5] = [
    std::time::Duration::from_millis(3200),
    std::time::Duration::from_millis(5100),
    std::time::Duration::from_millis(2400),
    std::time::Duration::from_millis(6300),
    std::time::Duration::from_millis(4000),
];
/// How long one blink takes, from open to closed and back.
const BLINK_LENGTH: std::time::Duration = std::time::Duration::from_millis(240);

/// What the assistant is called: the name the person gave it, or its usual one.
pub fn name(settings: &AppSettings) -> String {
    let given = tidy_name(&settings.assistant_name);
    if given.is_empty() { DEFAULT_NAME.to_owned() } else { given }
}

/// What the assistant keeps between frames.
#[derive(Default)]
pub struct AssistantUi {
    /// Suggestions the person dismissed (by `Suggestion::key`), until they are brought back or
    /// another project is opened.
    pub dismissed: std::collections::HashSet<String>,
    /// Whether the panel shows its rules and their settings.
    pub settings_open: bool,
    /// The viewer's tip was closed with "Not Now": no more tips until Studio is next opened, or
    /// tips are switched off and on again.
    pub tips_snoozed: bool,
    /// Whether the viewer's tip is open, showing the first suggestion, rather than folded down
    /// to the assistant's face.
    pub tip_open: bool,
    /// How many times the face has blinked. Each new number plays one blink wherever the face
    /// is drawn.
    pub blinks: u32,
    /// The field a question is typed into, made the first time the workspace is drawn, with
    /// the subscription that redraws the answers as the question changes.
    pub ask: Option<(Entity<InputState>, Subscription)>,
    /// Which tip was given last, by a click on the face or by asking (see `assistant_help::tip`).
    pub tip_shown: Option<usize>,
    /// The conversation so far, oldest first.
    pub messages: Vec<Message>,
    /// The tab the person chose: Chat (`true`) or Suggestions. Until they choose, the panel
    /// opens on Suggestions when there are some and on Chat when there are none.
    pub chat_chosen: Option<bool>,
    /// Keeps the conversation scrolled to its newest message.
    pub chat_scroll: ScrollHandle,
    /// The reply being "typed", while the typing dots show.
    pub pending: Option<Pending>,
    /// How many replies have been typed so far, to tell one wait from the next.
    pub typing_turns: u64,
    /// When something last happened in the project or with the assistant, for falling asleep.
    pub active_at: Option<std::time::Instant>,
    /// What the project looked like when the assistant last checked for changes (see `activity`).
    pub seen: Option<(i64, usize, usize, bool)>,
    /// A passing feeling (happy or sad) and when it ends.
    pub feeling: Option<(Mood, std::time::Instant)>,
    /// The one area the panel's list is narrowed to, or `None` for everything.
    pub area: Option<Area>,
    /// What Iris remembers between messages for "again" and recipes (see `iris::front`), made
    /// with the recipes taught so far the first time a message is answered.
    pub memory: Option<iris::front::Memory>,
    /// The last request that changed the project, for "too much" and "same for the logo".
    pub last_change: Option<LastChange>,
    /// The request of the step being answered, once it has changed the project.
    last_action: Option<studio_core::chat::Action>,
    /// The layers Iris last made or changed: what "it" means when nothing is selected.
    pub last_layers: Vec<Id>,
    /// The layers the step before the one being answered, in the same message, named or
    /// changed: what a step that names none means, ahead of the selection.
    pub carried: Vec<Id>,
    /// The file Iris last exported, to show in the Finder when asked.
    pub last_export: Option<std::path::PathBuf>,
    /// How many requests are being answered at once: a recipe run by a moment (before an
    /// export, say) may start while a typed request is answered. Only the outermost records
    /// what changed as a step for Undo.
    answering: u32,
    /// Whether the recipes set to run at a moment are running, so they don't set themselves
    /// off again (see `assistant_automation::at`).
    pub triggering: bool,
    /// Whether the last steps answered stopped at one that failed or wasn't understood.
    last_failed: bool,
    /// How many of the oldest messages were brought back from last time, drawn above an
    /// "Earlier" line.
    pub restored: usize,
    /// What was typed before, for the arrow keys in the chat field.
    pub recall: iris::complete::Recall,
    /// The message the arrow keys put in the chat field, while it is there unchanged.
    pub recalled: Option<String>,
    /// What the text in the chat field could become, shown above it.
    pub completions: Vec<iris::complete::Completion>,
}

impl AssistantUi {
    /// Whether a request is being answered now.
    pub fn is_answering(&self) -> bool {
        self.answering > 0
    }
}

/// The last request that changed the project.
pub struct LastChange {
    action: studio_core::chat::Action,
    /// The message it was read from.
    message: String,
    /// Whether it was the only step of its message to change anything. One Undo takes back a
    /// whole message, so only then is undoing it the same as undoing this request.
    alone: bool,
    /// The project as the change left it, to tell whether anything has changed since.
    after: Project,
}

/// One message in the conversation.
pub struct Message {
    /// Whether the person wrote it, rather than the assistant.
    pub from_person: bool,
    pub text: String,
    /// For an answer that describes a command: the command, offered as a "Do It" button.
    pub command: Option<&'static crate::keymap::Command>,
    /// Whether the message ends with a button that opens the Suggestions tab.
    pub opens_suggestions: bool,
    /// Short replies offered under the message, as buttons that send their words.
    pub replies: Vec<String>,
}

impl Message {
    pub(super) fn from_assistant(text: impl Into<String>) -> Self {
        Self { from_person: false, text: text.into(), command: None, opens_suggestions: false, replies: Vec::new() }
    }
}

/// A message the assistant is still "typing" its answer to: it shows its dots, then answers.
pub struct Pending {
    /// Which wait this is, so a wait that was overtaken by a newer message does nothing.
    turn: u64,
    /// The message being answered.
    message: String,
}

/// How long the typing dots show: a base wait, plus a little for each letter of the message up
/// to a limit, so a long request feels considered without making anyone wait for it.
const TYPING_SHORTEST: std::time::Duration = std::time::Duration::from_millis(450);
const TYPING_PER_LETTER: std::time::Duration = std::time::Duration::from_millis(4);
const TYPING_MOST_LETTERS: usize = 200;

/// The most messages kept; the oldest are dropped past this.
const MOST_MESSAGES: usize = 200;

/// Makes the field messages are typed into, the first time, and brings back the conversation
/// from last time. Called from the workspace's render.
pub fn keep_up(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if workspace.ui.assistant.ask.is_none() {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask a question, or type /help"));
        let sent = cx.subscribe_in(&input, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                let message = input.read(cx).value().to_string();
                input.update(cx, |input, cx| input.set_value("", window, cx));
                send(this, &message, window, cx);
            }
            // What is typed may be the start of a command or recipe, offered above the field.
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                super::assistant_automation::typed(this, &text);
                cx.notify();
            }
            _ => {}
        });
        workspace.ui.assistant.ask = Some((input, sent));
        super::assistant_automation::restore(workspace);
    }
}

/// Adds the assistant's next tip to the conversation.
fn give_tip(workspace: &mut Workspace) {
    let turn = workspace.ui.assistant.tip_shown.map_or(0, |turn| turn + 1);
    workspace.ui.assistant.tip_shown = Some(turn);
    let tip = super::assistant_help::tip(turn, &workspace.ui.settings);
    workspace.ui.assistant.messages.push(Message { command: tip.command, ..Message::from_assistant(format!("Tip: {}\n{}", tip.heading(), tip.body)) });
}

/// What the assistant says to a message that asks how to do something, from its ready-written
/// help answers: the best one in full, with the names of any others that fit. `None` when no
/// answer fits.
fn help_answer(workspace: &Workspace, message: &str) -> Option<Message> {
    let mut answers = super::assistant_help::find(message, &workspace.ui.settings).into_iter();
    let best = answers.next()?;
    let mut text = format!("{}\n{}", best.heading(), best.body);
    let others: Vec<String> = answers.map(|answer| answer.title).collect();
    if !others.is_empty() {
        text.push_str(&format!("\nYou can also ask about: {}.", others.join(", ")));
    }
    Some(Message { command: best.command, ..Message::from_assistant(text) })
}

/// What the assistant says when nothing understood a message: that it doesn't know, with the
/// nearest things it does know offered as replies.
fn unknown_answer(workspace: &Workspace, message: &str) -> Message {
    let nearest: Vec<String> = super::assistant_help::closest(message, &workspace.ui.settings).into_iter().map(|answer| format!("How to: {}", answer.title)).collect();
    let mut text = "I don't know that one. I can tell you about this composition (its length, settings, layers, tracks, sound and keyframes), do things in it for you, and explain Omina Motion's tools. Type /help to see what to ask.".to_owned();
    if !nearest.is_empty() {
        text.push_str("\nThe nearest things I do know:");
    }
    Message { replies: nearest, ..Message::from_assistant(text) }
}

/// Recipes Iris comes with: each is only messages she already understands, said in a row.
fn built_in_recipes() -> Vec<iris::recipes::Recipe> {
    let recipe = |name: &str, steps: &[&str]| iris::recipes::Recipe::new(name, steps.iter().copied());
    vec![
        recipe("tidy up", &["close the gaps", "fix everything"]),
        recipe("quick title", &["add text saying “Title”", "bring it in word by word", "fade out over half a second"]),
        recipe("social cut", &["make a vertical version", "set the work area from the start to 15s"]),
        recipe("watch", &["go to the start", "play"]),
        // Takes the words to show: "lower third Jane Doe".
        recipe("lower third {text}", &["add a text layer saying {text}", "put it in the bottom left corner", "slide it in from the left"]),
    ]
}

/// Iris's shared folder, where her recipes, export presets, kept conversation and hand-offs
/// live beside the settings the apps share. Tests keep theirs with the rest of their settings,
/// never in the real folder.
pub(crate) fn shared_folder() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return Some(crate::app_settings::folder().join("iris"));
    }
    iris::settings::default_folder()
}

/// What Iris remembers between messages, made the first time with the recipes taught so far.
/// It knows Motion's own shortcuts, so no recipe is given one of them.
pub(crate) fn memory(workspace: &mut Workspace) -> &mut iris::front::Memory {
    let app_keys = crate::keymap::keys_for_recipes(&workspace.ui.settings.shortcuts);
    let memory = workspace.ui.assistant.memory.get_or_insert_with(|| {
        let taught = shared_folder().map(|folder| iris::recipes::load(&folder, iris::App::Studio)).unwrap_or_default();
        iris::front::Memory::new(taught, built_in_recipes())
    });
    memory.app_keys = app_keys;
    memory
}

/// What a frame is worth in the open composition, for sums such as "2 seconds in frames".
pub(super) fn units(workspace: &Workspace) -> iris::calc::Units {
    iris::calc::Units { fps: Some(workspace.state.composition().frame_rate.fps()), dpi: None }
}

/// Motion's own slash commands, which a recipe can't be named after.
pub(super) fn reserved() -> Vec<&'static str> {
    studio_core::chat::COMMANDS.iter().map(|(name, _)| *name).collect()
}

/// Extra words people ask for a command with that its name doesn't use, by command id.
const COMMAND_WORDS: &[(&str, &str)] = &[("export", "render"), ("save-frame", "screenshot still"), ("open-settings", "preferences"), ("precompose", "precompose"), ("toggle-full-screen", "fullscreen"), ("toggle-guides", "guide grid")];

/// Commands Iris never runs herself: they put away the project or the app, so they are left
/// to the person. (Quit and the Hide commands are left out as commands of the whole app.)
const NEVER_RUN: &[&str] = &["close-project", "new-project"];

/// Every command as the shared finder sees it, in `keymap::COMMANDS`' order, so a position in
/// one is a position in the other.
fn commands() -> Vec<iris::commands::Command<'static>> {
    crate::keymap::COMMANDS.iter().map(|command| iris::commands::Command { name: command.name, keywords: COMMAND_WORDS.iter().find(|(id, _)| *id == command.id).map_or("", |(_, words)| words) }).collect()
}

/// Every word Iris's readers know, for putting a mistyped message right: the shared ones,
/// Motion's own, and the names of its commands.
fn vocabulary() -> Vec<String> {
    let mut words = iris::front::vocabulary();
    words.extend(studio_core::chores::vocabulary());
    words.extend(studio_core::chat::vocabulary());
    words.extend(crate::keymap::COMMANDS.iter().flat_map(|command| iris::spelling::words(command.name).collect::<Vec<_>>()));
    words.sort_unstable();
    words.dedup();
    words
}

/// Takes a message the person typed: adds it to the conversation, and answers it after a
/// moment of "typing".
pub fn send(workspace: &mut Workspace, message: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let message = message.trim();
    if message.is_empty() {
        return;
    }
    // A message still being answered is answered first, so the conversation stays in order.
    if let Some(earlier) = workspace.ui.assistant.pending.take() {
        answer(workspace, &earlier.message, window, cx);
    }
    wake(workspace);
    let assistant = &mut workspace.ui.assistant;
    assistant.recall.push(message);
    assistant.recalled = None;
    assistant.completions.clear();
    assistant.messages.push(Message { from_person: true, ..Message::from_assistant(message) });
    assistant.chat_chosen = Some(true);
    assistant.blinks = assistant.blinks.wrapping_add(1);
    // Clearing happens at once. Tests get every answer at once too, as they have no clock to wait on.
    let clears = matches!(message.to_lowercase().as_str(), "/clear" | "/reset" | "/new");
    if clears || cfg!(test) {
        answer(workspace, message, window, cx);
        return;
    }
    // Otherwise the assistant shows its typing dots for a moment, longer for a longer message.
    let wait = TYPING_SHORTEST + TYPING_PER_LETTER * message.chars().count().min(TYPING_MOST_LETTERS) as u32;
    assistant.typing_turns += 1;
    let turn = assistant.typing_turns;
    assistant.pending = Some(Pending { turn, message: message.to_owned() });
    cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(wait).await;
        this.update_in(cx, |this, window, cx| {
            // Only if this message is still the one waiting: a newer message answers it early.
            if let Some(pending) = this.ui.assistant.pending.take_if(|pending| pending.turn == turn) {
                answer(this, &pending.message, window, cx);
            }
        })
        .ok();
    })
    .detach();
    show_newest(window, cx);
}

/// Answers a message: reads it at the front door every app's Iris shares, then answers each
/// of its steps in order. A step that can't be done stops the ones after it, which are named
/// rather than run. Everything a message changes is one step for Undo.
fn answer(workspace: &mut Workspace, message: &str, window: &mut Window, cx: &mut Context<Workspace>) {
    let owned = message.to_owned();
    let ran = answer_by_itself(workspace, move |workspace, cx| read_at_the_front_door(workspace, &owned, cx), window, cx);
    if ran.changes > 0
        && let Some(action) = workspace.ui.assistant.last_action.take()
    {
        let alone = ran.changes == 1;
        workspace.ui.assistant.last_change = Some(LastChange { action, message: message.to_owned(), alone, after: workspace.state.project.clone() });
    }
    super::assistant_automation::remember(workspace);
}

/// Answers the turns `read` gives, as one request: what "it" means starts afresh, and what the
/// turns change is one step for Undo, unless this runs inside another request (a recipe set to
/// run before an export, while an export asked for in the chat starts), whose step it joins.
/// The turns are read inside, so a recipe they teach is in place before they run.
pub(super) fn answer_by_itself(workspace: &mut Workspace, read: impl FnOnce(&mut Workspace, &mut Context<Workspace>) -> Vec<iris::front::Turn>, window: &mut Window, cx: &mut Context<Workspace>) -> super::assistant_automation::Ran {
    let outer_carried = std::mem::take(&mut workspace.ui.assistant.carried);
    workspace.ui.assistant.answering += 1;
    let turns = read(workspace, cx);
    let changes = answer_turns(workspace, turns, window, cx);
    workspace.ui.assistant.answering -= 1;
    workspace.ui.assistant.carried = outer_carried;
    let failed = std::mem::take(&mut workspace.ui.assistant.last_failed);
    if changes > 0 && !workspace.ui.assistant.is_answering() {
        // Recorded now, so the whole request is one step and what it left is known.
        workspace.state.record_history(false);
    }
    trim_messages(workspace);
    show_newest(window, cx);
    super::assistant_automation::Ran { changes, failed }
}

/// Keeps the conversation to its most recent messages.
fn trim_messages(workspace: &mut Workspace) {
    let assistant = &mut workspace.ui.assistant;
    let excess = assistant.messages.len().saturating_sub(MOST_MESSAGES);
    if excess > 0 {
        assistant.messages.drain(..excess);
        assistant.restored = assistant.restored.saturating_sub(excess);
    }
}

/// A message as the front door reads it: its steps in order, with recipes run and sums
/// answered. Recipes taught or forgotten on the way are saved, and their shortcuts bound.
fn read_at_the_front_door(workspace: &mut Workspace, message: &str, cx: &mut gpui_kit::App) -> Vec<iris::front::Turn> {
    let (units, reserved) = (units(workspace), reserved());
    let reading = iris::front::read(message, memory(workspace), &units, &reserved);
    if reading.recipes_changed {
        super::assistant_automation::recipes_changed(workspace, cx);
    }
    reading.turns
}

/// One thing to answer for a message: a turn from the front door, or part of one.
enum Step {
    Say(String),
    Followup(iris::followup::Followup),
    /// A request to read and carry out. `apart` when it is already one part of a request
    /// joined by "and", so it isn't looked at for parts again.
    Ask { text: String, apart: bool },
}

impl Step {
    /// The step as it reads in "these weren't run".
    fn words(&self) -> Option<String> {
        use iris::followup::Followup;
        match self {
            Self::Ask { text, .. } => Some(format!("“{text}”")),
            Self::Followup(followup) => Some(
                match followup {
                    Followup::More => "“more”",
                    Followup::Less => "“less”",
                    Followup::Undo => "“undo”",
                    Followup::Redo => "“redo”",
                    Followup::Again | Followup::SameFor(_) => "“the same again”",
                }
                .to_owned(),
            ),
            Self::Say(_) => None,
        }
    }
}

/// Which reader takes a piece of a message, and for which layers, judged without doing
/// anything. Two pieces are alike only when one reader could take both in one go.
#[derive(PartialEq)]
enum Kind {
    /// A question, with its answer, so two different questions are both answered.
    Answer(String),
    /// Timing and motion for these layers (`None` for "it" or none named).
    Instruct(Option<iris::intent::Target>),
    /// Housekeeping for these layers.
    Housekeep(Option<iris::intent::Target>),
    /// Anything else that is done: readers that take one request at a time.
    Does(studio_core::chat::Action),
}

/// What `chat` makes of a piece of a message, as a `Kind`. `None` when nothing that answers
/// from the project or does something understands it (the help answers don't count).
fn kind_of(workspace: &Workspace, piece: &str) -> Option<Kind> {
    use iris::intent::Target;
    use studio_core::chat::{Action, Reply};
    let named = |target: Target| (!matches!(target, Target::Unnamed | Target::Selection)).then_some(target);
    Some(match read(workspace, piece) {
        Reply::Unknown => return None,
        // A greeting beside a request ("hello, how long is the video?") is not a part of its own.
        Reply::Say(_) if studio_core::chat::is_small_talk(piece) => return None,
        Reply::Say(answer) => Kind::Answer(answer),
        Reply::Do(_, Action::Instruct(request)) => Kind::Instruct(named(request.target)),
        Reply::Do(_, Action::Housekeep(request)) => Kind::Housekeep(named(request.target)),
        Reply::Do(_, action) => Kind::Does(action),
    })
}

/// Whether a request keeps words as typed for a name or for text to show: a new name for a
/// layer, a marker's label, the text of a new layer.
fn keeps_wording(reply: &studio_core::chat::Reply) -> bool {
    use studio_core::chat::{Action, Reply};
    use studio_core::chores::{Chore, Jump, Made};
    match reply {
        Reply::Do(_, Action::Housekeep(request)) => request.steps.iter().any(|step| matches!(step, iris::layers::Step::Rename(_))),
        Reply::Do(_, Action::Chore(request)) => matches!(&request.chore, Chore::Make { what: Made::Text(_), .. } | Chore::Marker { label: Some(_), .. } | Chore::Go(Jump::MarkerCalled(_))),
        _ => false,
    }
}

/// A request as the requests joined in it by "and", when different readers take its parts
/// ("duplicate it and mute it"), or whole when one reader understands all of it ("start at 5s
/// and fade in"). See `iris::script::joined`.
fn read_apart(workspace: &Workspace, text: &str) -> Vec<String> {
    let whole = vec![text.to_owned()];
    // A name or text to show may hold an "and" of its own ("rename it to Salt and Pepper").
    // Without quotes to say where it ends, the request is only read apart when what follows
    // the "and" plainly asks for something: it points at a layer and is understood as typed.
    if keeps_wording(&read(workspace, text)) && !text.contains(['"', '“', '”']) {
        let parts = iris::script::parts(text);
        let points = |part: &String| part.split_whitespace().any(|word| matches!(word.to_lowercase().as_str(), "it" | "them" | "this" | "these" | "that"));
        let asks = parts.len() > 1 && parts[1..].iter().all(|part| points(part) && kind_of(workspace, part).is_some());
        return if asks { parts } else { whole };
    }
    // A command named in full is that command, even when its name has an "and" in it
    // ("Render & Play the work area").
    if matches!(read(workspace, text), studio_core::chat::Reply::Do(_, studio_core::chat::Action::Run(_))) {
        return whole;
    }
    // An export lists its own formats ("as a gif and as a png sequence"). It is read apart only
    // when what follows the "and" is a request in its own right ("export as mp4 and mute it").
    let exports = matches!(read(workspace, text), studio_core::chat::Reply::Do(_, studio_core::chat::Action::Chore(request)) if matches!(request.chore, studio_core::chores::Chore::Export(_)));
    if exports && iris::script::parts(text).iter().skip(1).all(|part| kind_of(workspace, part).is_none()) {
        return whole;
    }
    iris::script::joined(text, |piece| kind_of(workspace, piece))
}

/// The instructions a step asks for, when that is all it is, read without doing anything.
fn as_instructions(workspace: &Workspace, text: &str) -> Option<iris::intent::Request> {
    match read(workspace, text) {
        studio_core::chat::Reply::Do(_, studio_core::chat::Action::Instruct(request)) => Some(request),
        _ => None,
    }
}

/// Answers `turns` in order and returns how many of them changed the project. A request
/// joined by "and" is answered part by part when different readers take its parts.
fn answer_turns(workspace: &mut Workspace, turns: Vec<iris::front::Turn>, window: &mut Window, cx: &mut Context<Workspace>) -> usize {
    use iris::front::Turn;
    use iris::intent::Target;
    let mut steps: std::collections::VecDeque<Step> = turns
        .into_iter()
        .map(|turn| match turn {
            Turn::Say(text) => Step::Say(text),
            Turn::Followup(followup) => Step::Followup(followup),
            Turn::Ask(text) => Step::Ask { text, apart: false },
        })
        .collect();
    let mut changes = 0;
    while let Some(step) = steps.pop_front() {
        let outcome = match step {
            Step::Say(text) => {
                workspace.ui.assistant.messages.push(Message::from_assistant(text));
                Outcome::Said
            }
            Step::Followup(followup) => follow_up(workspace, &followup, window, cx),
            Step::Ask { text, apart } => {
                let parts = if apart { Vec::new() } else { read_apart(workspace, &text) };
                if parts.len() > 1 {
                    for part in parts.into_iter().rev() {
                        steps.push_front(Step::Ask { text: part, apart: true });
                    }
                    continue;
                }
                // Instructions for the same layer said in several steps ("start at 5s. Fade it
                // in.") are carried out together and answered once.
                let mut text = text;
                if let Some(mut request) = as_instructions(workspace, &text) {
                    while let Some(Step::Ask { text: following, apart }) = steps.front() {
                        // A step that is itself several requests is left to be read apart.
                        if !apart && read_apart(workspace, following).len() > 1 {
                            break;
                        }
                        let Some(more) = as_instructions(workspace, following) else { break };
                        let same_layers = more.target == request.target || matches!(more.target, Target::Unnamed | Target::Selection) || matches!(request.target, Target::Unnamed | Target::Selection);
                        if !same_layers {
                            break;
                        }
                        if matches!(request.target, Target::Unnamed | Target::Selection) && !matches!(more.target, Target::Unnamed) {
                            request.target = more.target;
                        }
                        request.steps.extend(more.steps);
                        text = format!("{text} {following}");
                        steps.pop_front();
                    }
                    deliver(workspace, studio_core::chat::Reply::Do(String::new(), studio_core::chat::Action::Instruct(request)), &text, window, cx)
                } else {
                    ask(workspace, &text, window, cx)
                }
            }
        };
        match outcome {
            Outcome::Changed => {
                changes += 1;
                memory(workspace).changed();
            }
            // Moving the playhead or running a command is worth repeating with "again" too.
            Outcome::Did => memory(workspace).changed(),
            Outcome::Said => {}
            Outcome::Failed | Outcome::Unknown => {
                workspace.ui.assistant.last_failed = true;
                let left: Vec<String> = steps.iter().filter_map(Step::words).collect();
                if !left.is_empty() {
                    workspace.ui.assistant.messages.push(Message::from_assistant(format!("I stopped there, so {} run: {}.", if left.len() == 1 { "this wasn't" } else { "these weren't" }, left.join(", "))));
                }
                break;
            }
        }
    }
    changes
}

/// What `chat` makes of a message, with the project as it is now.
fn read(workspace: &Workspace, text: &str) -> studio_core::chat::Reply {
    let (state, found, name, commands) = (&workspace.state, open_suggestions(workspace), name(&workspace.ui.settings), commands());
    let scene = studio_core::chat::Scene {
        project: &state.project,
        composition: state.composition(),
        current_frame: state.current_frame,
        selected: &state.selected_layers,
        work_area: (state.work_area.start.round() as i64, state.work_area.end.round() as i64),
        name: &name,
        suggestions: &found,
        commands: &commands,
    };
    studio_core::chat::reply(text, &scene)
}

/// Answers one request. It is read as typed first, by the readers that answer from the
/// project or do something. What they can't read is tried once more with its mistyped words
/// put right, saying how it was read, and only then are the help answers searched: they
/// forgive typos of their own, and would answer a mistyped request with a how-to. Failing all
/// of that, the assistant says it doesn't know.
fn ask(workspace: &mut Workspace, text: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    use studio_core::chat::Reply;
    // Export presets by name ("export for Instagram Reel") come first: their names are the
    // person's own words, which no other reader knows.
    if let Some(said) = iris::export_presets::read(text, &super::export_presets::names()) {
        let done = super::assistant_chores::export_preset(workspace, said, window, cx);
        return finish(workspace, done);
    }
    let reply = read(workspace, text);
    if reply != Reply::Unknown {
        return deliver(workspace, reply, text, window, cx);
    }
    let fixed = iris::spelling::correct(text, &vocabulary());
    let first = workspace.ui.assistant.messages.len();
    if let Some(fixed) = &fixed {
        let reply = read(workspace, &fixed.message);
        if reply != Reply::Unknown {
            let outcome = deliver(workspace, reply, &fixed.message, window, cx);
            if let Some(reply) = workspace.ui.assistant.messages.get_mut(first) {
                reply.text = format!("{}\n{}", fixed.note(), reply.text);
            }
            return outcome;
        }
    }
    let put_right = fixed.as_ref().map(|fixed| (fixed.message.as_str(), Some(fixed.note())));
    for (question, note) in std::iter::once((text, None)).chain(put_right) {
        if let Some(mut answer) = help_answer(workspace, question) {
            if let Some(note) = note {
                answer.text = format!("{note}\n{}", answer.text);
            }
            workspace.ui.assistant.messages.push(answer);
            return Outcome::Said;
        }
    }
    feel(workspace, Mood::Sad);
    let unknown = unknown_answer(workspace, text);
    workspace.ui.assistant.messages.push(unknown);
    Outcome::Unknown
}

/// Answers a reply that refers back: undo and redo, more, less, and the same for another layer.
fn follow_up(workspace: &mut Workspace, followup: &iris::followup::Followup, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    use iris::followup::Followup;
    let say = |workspace: &mut Workspace, text: &str, replies: &[&str], outcome: Outcome| {
        workspace.ui.assistant.messages.push(Message { replies: replies.iter().map(|reply| (*reply).to_owned()).collect(), ..Message::from_assistant(text) });
        outcome
    };
    match followup {
        Followup::Undo | Followup::Redo => {
            let redo = *followup == Followup::Redo;
            // What is about to be taken back or put back, in the History panel's words. A
            // change not recorded yet is recorded first, so it is the one undone.
            workspace.state.record_history(false);
            let (made, undone) = workspace.state.history_changes();
            let label = if redo { undone.first() } else { made.last() }.map(|label| (*label).to_owned());
            let changed = if redo { workspace.state.redo() } else { workspace.state.undo() };
            if !changed {
                return say(workspace, if redo { "There's nothing to redo." } else { "There's nothing to undo." }, &[], Outcome::Failed);
            }
            // An undone deletion brings its media back.
            workspace.request_proxies();
            let what = label.map_or_else(String::new, |label| format!(": {label}"));
            let said = format!("{}{what}.", if redo { "Redone" } else { "Undone" });
            say(workspace, &said, if redo { &["Undo that"] } else { &["Redo"] }, Outcome::Did)
        }
        // More of the same is the last change made once again.
        Followup::More => {
            let turns = read_at_the_front_door(workspace, "again", cx);
            let before = workspace.state.project.clone();
            let changes = answer_turns(workspace, turns, window, cx);
            if changes > 0 && workspace.state.project == before {
                return say(workspace, "That is as far as it goes that way. Tell me how much more you'd like, as in “20% bigger” or “move it 40 px left”.", &[], Outcome::Failed);
            }
            if changes > 0 { Outcome::Changed } else { Outcome::Said }
        }
        Followup::Less => {
            let Some(last) = workspace.ui.assistant.last_change.take() else {
                return say(workspace, "There's nothing of mine to tone down yet.", &[], Outcome::Failed);
            };
            workspace.state.record_history(false);
            if workspace.state.project != last.after {
                return say(workspace, "Something has changed since I did that, so I've left it alone. Undo steps back through the changes one at a time.", &["Undo that"], Outcome::Failed);
            }
            if !workspace.state.undo() {
                return say(workspace, "There's nothing to undo.", &[], Outcome::Failed);
            }
            workspace.request_proxies();
            // Half as far, where the change is one that can be done by halves.
            match studio_core::chat::softened(&last.action).filter(|_| last.alone) {
                Some(softer) => {
                    let first = workspace.ui.assistant.messages.len();
                    let outcome = deliver(workspace, studio_core::chat::Reply::Do(String::new(), softer), &last.message, window, cx);
                    if let Some(reply) = workspace.ui.assistant.messages.get_mut(first) {
                        reply.text = format!("I took that back and went half as far. {}", reply.text);
                    }
                    outcome
                }
                None => say(workspace, "I've taken that back. Tell me how far to go instead, as in “10% bigger”, “move it 5 px left” or “fade in over half a second”.", &["Redo"], Outcome::Did),
            }
        }
        Followup::SameFor(name) => {
            let Some((action, message)) = workspace.ui.assistant.last_change.as_ref().map(|last| (studio_core::chat::retargeted(&last.action, name), last.message.clone())) else {
                return say(workspace, "There's nothing to do the same of yet: ask me to change a layer first.", &[], Outcome::Failed);
            };
            match action {
                Some(action) => deliver(workspace, studio_core::chat::Reply::Do(String::new(), action), &message, window, cx),
                None => say(workspace, "The last thing I did wasn't to a layer, so I can't do it to another one.", &[], Outcome::Failed),
            }
        }
        // The front door turns "again" into the steps it repeats, so it never arrives here.
        Followup::Again => Outcome::Said,
    }
}

/// Animates the open composition (or its selected layers when `selection`) in `style`, puts the
/// playhead at the start so it can be watched, and returns what the assistant says about it.
fn animate_design(workspace: &mut Workspace, selection: bool, style: studio_core::auto_animate::Style) -> Done {
    let comp = workspace.state.composition().id;
    let only = if selection { workspace.state.selected_layers.clone() } else { Vec::new() };
    let summary = studio_core::auto_animate::animate(&mut workspace.state.project, comp, &only, style);
    let mut notes = Vec::new();
    if summary.kept > 0 {
        let (count, them) = if summary.kept == 1 { ("1 layer".to_owned(), "it") } else { (format!("{} layers", summary.kept), "them") };
        notes.push(format!("I left {count} alone because you'd already animated {them}."));
    }
    if summary.backgrounds > 0 {
        notes.push("The background stays still, so the frame is never empty.".to_owned());
    }
    if summary.animated.is_empty() {
        let reason = if selection { "the selected layers" } else { "this composition" };
        return Done::failed(format!("I couldn't find anything to animate in {reason}: hidden and locked layers, video clips, sound and backgrounds are left as they are. {}", notes.join(" ")).trim_end());
    }
    workspace.state.set_current_frame(0);
    let count = summary.animated.len();
    let which = if selection { "the selection" } else { "the design" };
    let what = sentence(&summary.described);
    let mut said = format!("Done: I animated {which}, {count} layer{} in a {} style. {what}.", if count == 1 { "" } else { "s" }, style.name());
    for note in notes {
        said.push(' ');
        said.push_str(&note);
    }
    said.push_str(" The playhead is at the start: press play to watch. Each layer's choice is in the Motion panel, one Undo takes it all back, and you can ask for it bouncy, snappy or smooth.");
    Done::changed(said, summary.animated.into_iter().map(|(layer, _)| layer).collect())
}

/// Carries out instruction steps on `layers` and returns what happened as a sentence ("“Title”
/// fades in over 0.6s.", with anything that couldn't be done after it) and the layers changed.
/// `Err` says why nothing changed.
fn instruct(workspace: &mut Workspace, layers: &[Id], steps: &[iris::intent::Step]) -> Result<(String, Vec<Id>), String> {
    use studio_core::instructions::{Context, carry_out};
    let comp = workspace.state.composition().id;
    let context = Context { playhead: workspace.state.current_frame, magnetic_main_track: workspace.state.magnetic_main_track };
    let outcome = carry_out(&mut workspace.state.project, comp, layers, steps, context);
    let problems = outcome.problems.join(" ");
    if outcome.changed.is_empty() {
        return Err(if problems.is_empty() { "That didn't change anything.".to_owned() } else { problems });
    }
    let what = sentence(&outcome.done);
    let what = what.chars().next().map_or_else(String::new, |first| first.to_lowercase().chain(what.chars().skip(1)).collect());
    // The phrases are for one layer ("starts"), so several read "Each of “A” and “B” starts".
    let who = super::assistant_chores::named(&workspace.state, &outcome.changed);
    let mut said = if outcome.changed.len() == 1 { format!("{who} {what}.") } else { format!("Each of {who} {what}.") };
    if !problems.is_empty() {
        said.push(' ');
        said.push_str(&problems);
    }
    Ok((said, outcome.changed))
}

/// Carries out instruction steps on `layers`, puts the playhead at the first one's start,
/// ready to watch, and says what was done and what couldn't be.
pub(super) fn instruct_layers(workspace: &mut Workspace, layers: &[Id], steps: &[iris::intent::Step]) -> Done {
    match instruct(workspace, layers, steps) {
        Ok((said, changed)) => {
            if let Some(start) = changed.first().and_then(|id| workspace.state.composition().layer(*id)).map(|layer| layer.in_frame) {
                workspace.state.set_current_frame(start);
            }
            // One layer reads "Done: “A” starts"; several read "Done. Each of “A” and “B” starts".
            let opening = if changed.len() == 1 { "Done:" } else { "Done." };
            Done::changed(format!("{opening} {said} The playhead is at its start, and one Undo takes it all back."), changed)
        }
        Err(why) => Done::failed(why),
    }
}

/// The layers "it" can mean beyond the selection, as things stand.
fn pointed_layers(workspace: &Workspace) -> (Vec<Id>, Vec<Id>) {
    (workspace.ui.assistant.carried.clone(), workspace.ui.assistant.last_layers.clone())
}

/// Carries out housekeeping typed to the assistant ("hide the logo", "duplicate it", "bring it
/// to the front") on the layers it is about, and says what was done and what couldn't be.
fn housekeep(workspace: &mut Workspace, request: &iris::layers::Request, message: &str) -> Done {
    use studio_core::instructions::Context;
    let (carried, last) = pointed_layers(workspace);
    // "Unlock everything" is about the locked layers, which "everything" otherwise leaves out.
    let found = if request.steps.contains(&iris::layers::Step::Unlock) {
        studio_core::instructions::layers_meant_locked_too(workspace.state.composition(), &request.target, &workspace.state.selected_layers, message)
    } else {
        layers_of(&workspace.state, Pointed { carried: &carried, last: &last }, &request.target, message)
    };
    let layers = match found {
        Ok(layers) => layers,
        Err(why) => return Done::failed(why),
    };
    let comp = workspace.state.composition().id;
    let context = Context { playhead: workspace.state.current_frame, magnetic_main_track: workspace.state.magnetic_main_track };
    let selected = workspace.state.selected_layers.clone();
    let outcome = studio_core::housekeeping::carry_out(&mut workspace.state.project, comp, &layers, &request.steps, context, &selected);
    if let Some(selection) = &outcome.selection {
        workspace.state.clear_selection();
        workspace.state.selected_layers = selection.clone();
    }
    let problems = outcome.problems.join(" ");
    if outcome.done.is_empty() {
        return Done::failed(if problems.is_empty() { "That didn't change anything.".to_owned() } else { problems });
    }
    let who = super::assistant_chores::quoted_names(&outcome.who);
    let what = sentence(&outcome.done);
    let what = what.chars().next().map_or_else(String::new, |first| first.to_lowercase().chain(what.chars().skip(1)).collect::<String>());
    let mut said = if outcome.who.len() == 1 { format!("Done: {who} {what}.") } else { format!("Done. Each of {who} {what}.") };
    if !problems.is_empty() {
        said.push(' ');
        said.push_str(&problems);
    }
    if !outcome.changed {
        return Done { said, outcome: Outcome::Did, layers: outcome.layers, replies: Vec::new() };
    }
    said.push_str(if request.steps.len() > 1 { " One Undo takes it all back." } else { " One Undo takes it back." });
    Done::changed(said, outcome.layers)
}

/// Saves the frame under the playhead as a picture in the Exports folder and says where it
/// will land; the conversation hears when it is written (see `export_finished`).
pub(super) fn save_frame(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    if !workspace.has(Feature::Export) {
        return super::assistant_chores::turned_off(Feature::Export);
    }
    super::export::before_export(workspace, window, cx);
    let frame = workspace.state.current_frame;
    let started = super::export::save_frame_then(workspace, window, cx, |this, path, saved, window, cx| {
        export_finished(this, path, &saved.map(|()| studio_export::Outcome::Finished { left_out: Vec::new() }), window, cx);
    });
    match started {
        Ok(path) => Done::did(format!("Saving frame {frame} as a picture. It will land at {}. I'll say here when it is written.", path.display())),
        Err(why) => Done::failed(why),
    }
}

/// Says in the conversation how an export Iris was asked for ended: the file's full path once
/// it is written, or why it wasn't.
pub(super) fn export_finished(workspace: &mut Workspace, path: &std::path::Path, outcome: &Result<studio_export::Outcome, String>, window: &mut Window, cx: &mut Context<Workspace>) {
    let file = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let message = match outcome {
        Ok(studio_export::Outcome::Finished { left_out }) => {
            workspace.ui.assistant.last_export = Some(path.to_path_buf());
            feel(workspace, Mood::Happy);
            let mut text = format!("Written: {}", path.display());
            if !left_out.is_empty() {
                text.push_str(&format!("\nThe format couldn't hold everything:\n{}", left_out.iter().map(|line| format!("• {line}")).collect::<Vec<_>>().join("\n")));
            }
            Message { replies: vec!["Show it in the Finder".to_owned()], ..Message::from_assistant(text) }
        }
        Ok(studio_export::Outcome::Cancelled) => Message::from_assistant(format!("The export of “{file}” was cancelled, so nothing was written.")),
        Err(error) => {
            feel(workspace, Mood::Sad);
            Message::from_assistant(format!("The export of “{file}” failed, so nothing was written: {error}"))
        }
    };
    workspace.ui.assistant.messages.push(message);
    show_newest(window, cx);
}

/// Runs one of the app's commands by its place in `keymap::COMMANDS` and says so, with its
/// shortcut. A command whose feature is switched off is explained instead, and so is one that
/// puts the project or the app away, which is left to the person.
fn run_command(workspace: &mut Workspace, index: usize, window: &mut Window, cx: &mut Context<Workspace>) -> Done {
    let Some(command) = crate::keymap::COMMANDS.get(index) else { return Done::failed("I couldn't find that command.") };
    let named = match crate::keymap::key_for(&workspace.ui.settings.shortcuts, command.id) {
        Some(key) => format!("“{}” ({key})", command.name),
        None => format!("“{}”", command.name),
    };
    if !command.works_in_editor() || NEVER_RUN.contains(&command.id) {
        return Done::said(format!("I leave {named} to you, since it puts away what you are working on. It is in the menu bar."));
    }
    if let Some(feature) = Feature::of_command(command.id).filter(|feature| !workspace.has(*feature)) {
        return super::assistant_chores::turned_off(feature);
    }
    window.dispatch_action(command.action(), cx);
    let switches = if command.name.contains(" or ") { " It switches each time you ask." } else { "" };
    Done::did(format!("Running {named}.{switches}"))
}

/// A list read as a sentence, capitalized: "A, b and c".
fn sentence(parts: &[String]) -> String {
    let joined = match parts {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let mut characters = joined.chars();
    characters.next().map_or_else(String::new, |first| first.to_uppercase().chain(characters).collect())
}

/// Makes a copy of the open composition at `width` × `height` with Magic Resize and opens it.
/// Returns what the assistant says about it.
pub(super) fn make_resized_copy(workspace: &mut Workspace, width: u32, height: u32, label: &str) -> Done {
    if !workspace.has(Feature::MagicResize) {
        return Done::failed("Magic Resize is turned off in Settings ▸ Tools & Features. Turn it on and ask me again.");
    }
    let (comp, original) = (workspace.state.composition().id, workspace.state.composition().name.clone());
    match workspace.state.duplicate_resized(comp, width, height, label) {
        Some(_) => {
            let copy = workspace.state.composition().name.clone();
            Done::changed(format!("Done. {copy} is a {width} × {height} copy, and it is open now. Backgrounds are scaled to fill the new frame; titles and graphics keep their place and are resized to fit. {original} is unchanged."), Vec::new())
        }
        None => Done::failed(format!("I couldn't make a {width} × {height} copy of {original}.")),
    }
}

/// Brings the newest message into view once it has been laid out.
pub(super) fn show_newest(window: &mut Window, cx: &mut Context<Workspace>) {
    cx.on_next_frame(window, |this, _, cx| {
        this.ui.assistant.chat_scroll.scroll_to_bottom();
        cx.notify();
    });
    cx.notify();
}

/// Adds the assistant's reply to `message` to the conversation, does whatever the reply calls
/// for (making changes, clearing the conversation, renaming), and returns what became of it.
fn deliver(workspace: &mut Workspace, reply: studio_core::chat::Reply, message: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    use studio_core::chat::{Action, Reply};
    let (text, action) = match reply {
        Reply::Say(text) => {
            workspace.ui.assistant.messages.push(Message::from_assistant(text));
            return Outcome::Said;
        }
        Reply::Unknown => {
            let unknown = unknown_answer(workspace, message);
            workspace.ui.assistant.messages.push(unknown);
            return Outcome::Unknown;
        }
        Reply::Do(text, action) => (text, action),
    };
    let done = match &action {
        Action::Clear => {
            workspace.ui.assistant.messages.clear();
            super::assistant_automation::forget(workspace);
            return Outcome::Said;
        }
        Action::Tip => {
            give_tip(workspace);
            return Outcome::Said;
        }
        Action::ShowSuggestions => {
            workspace.ui.assistant.messages.push(Message { opens_suggestions: true, ..Message::from_assistant(text) });
            return Outcome::Said;
        }
        Action::Rename(new_name) => {
            let new_name = tidy_name(new_name);
            let said = if new_name.is_empty() { "That name is empty, so I'll keep mine.".to_owned() } else { format!("{new_name} it is.") };
            if !new_name.is_empty() {
                workspace.ui.settings.assistant_name = new_name.clone();
                crate::app_settings::update(move |settings| settings.assistant_name = new_name.clone());
            }
            Done::said(said)
        }
        Action::OpenSettings => {
            super::settings::open(workspace, window, cx);
            Done::did(text)
        }
        Action::FixAllSafe => match fix_all_safe(workspace) {
            0 => Done::said("There was nothing safe to fix. The rest are judgement calls: the Suggestions tab has each one."),
            1 => Done::changed("Done: 1 fix made. Undo takes it back.", Vec::new()),
            made => Done::changed(format!("Done: {made} fixes made. One Undo takes them all back."), Vec::new()),
        },
        Action::Resize { width, height, label } => make_resized_copy(workspace, *width, *height, label),
        // No feel asked for: the one chosen in Settings ▸ Iris.
        Action::Animate { selection, style } => animate_design(workspace, *selection, style.unwrap_or(workspace.ui.settings.assistant_style)),
        Action::Instruct(request) => {
            let (carried, last) = pointed_layers(workspace);
            match layers_of(&workspace.state, Pointed { carried: &carried, last: &last }, &request.target, message) {
                Ok(layers) => instruct_layers(workspace, &layers, &request.steps),
                Err(why) => Done::failed(why),
            }
        }
        Action::Housekeep(request) => housekeep(workspace, request, message),
        Action::Chore(request) => super::assistant_chores::carry_out(workspace, request, message, window, cx),
        // A request that could mean several things: the question, with the meanings to pick from.
        Action::Ask(meanings) => Done { replies: meanings.clone(), ..Done::said(text) },
        Action::Run(index) => run_command(workspace, *index, window, cx),
        Action::Offer(found) => {
            let names: Vec<&str> = found.iter().filter_map(|index| crate::keymap::COMMANDS.get(*index)).map(|command| command.name).collect();
            Done::said("Which one do you mean?").offering(&names)
        }
    };
    post(workspace, done, Some(action))
}

/// Says what was done about a request that no chat action stands for (an export preset, say).
fn finish(workspace: &mut Workspace, done: Done) -> Outcome {
    post(workspace, done, None)
}

/// Adds what was done to the conversation with the replies that fit it, shows it in the face,
/// and remembers it for the replies that refer back ("too much", "same for the logo"), when
/// `action` says what it was.
fn post(workspace: &mut Workspace, done: Done, action: Option<studio_core::chat::Action>) -> Outcome {
    let mut replies = Vec::new();
    match done.outcome {
        Outcome::Changed => {
            feel(workspace, Mood::Happy);
            replies.push("Undo that".to_owned());
            // "More" and "Less" mean something after a change that can be made by degrees.
            if action.as_ref().is_some_and(|action| studio_core::chat::softened(action).is_some()) {
                replies.extend(["More".to_owned(), "Less".to_owned()]);
            }
            if action.is_some() {
                workspace.ui.assistant.last_action = action;
            }
        }
        Outcome::Failed => feel(workspace, Mood::Sad),
        Outcome::Did | Outcome::Said | Outcome::Unknown => {}
    }
    // The layers this step was about are what "it" means next.
    if !done.layers.is_empty() {
        workspace.ui.assistant.carried = done.layers.clone();
        workspace.ui.assistant.last_layers = done.layers;
    }
    replies.extend(done.replies);
    workspace.ui.assistant.messages.push(Message { replies, ..Message::from_assistant(done.said) });
    done.outcome
}

/// The loudest moment of the part of its file a sound clip plays, from 0 to 1, read from the
/// waveform the timeline already keeps. `None` until that waveform is ready, and for clips
/// whose speed is changed (their frames don't map straight onto the file).
fn loudest_moment(workspace: &Workspace, layer: &Layer) -> Option<f32> {
    let (LayerContent::Footage { item } | LayerContent::Audio { item }) = layer.content else { return None };
    if !layer.time.is_normal() {
        return None;
    }
    let path = workspace.state.project.item(item)?.path.as_ref()?;
    let peaks = workspace.ui.timeline.media.peaks.peaks(path)?;
    let fps = workspace.state.composition().frame_rate.fps().max(1.0);
    let bucket = |frame: i64| (((frame - layer.start_frame).max(0) as f64 / fps) * f64::from(peaks.per_second)) as usize;
    let (from, to) = (bucket(layer.in_frame).min(peaks.values.len()), bucket(layer.out_frame).min(peaks.values.len()));
    peaks.values.get(from..to)?.iter().copied().reduce(f32::max)
}

/// The suggestions for the open composition that haven't been dismissed, earliest first.
pub fn open_suggestions(workspace: &Workspace) -> Vec<Suggestion> {
    let (composition, options) = (workspace.state.composition(), &workspace.ui.settings.assistant);
    let mut found = studio_core::assistant::suggestions(composition, options);
    // Sound boosted too far needs the sound itself, which only the app has measured.
    if options.runs(Check::Clipping) {
        // Only a clip turned up at some point can be pushed over, so the rest aren't measured.
        let boosted = composition.layers.iter().filter(|layer| layer.audio.volume_db.is_animated() || layer.audio.volume_db.value > 0.0);
        found.extend(boosted.filter_map(|layer| studio_core::assistant::clipping(layer, loudest_moment(workspace, layer)?)));
        found.sort_by_key(|suggestion| (suggestion.frame, suggestion.check));
    }
    found.retain(|suggestion| !workspace.ui.assistant.dismissed.contains(&suggestion.key()));
    found
}

/// The color an area's suggestions are tagged with in the panel and marked with on the
/// timeline's ruler.
pub fn area_color(area: Area) -> gpui_kit::Rgba {
    gpui_kit::rgb(match area {
        Area::Motion => 0x8f87e8,
        Area::Editing => 0xe0683c,
        Area::Audio => 0x2bb388,
        Area::Text => 0xdd6590,
        Area::Picture => 0x9a9992,
    })
}

/// Changes the assistant's rules, for now and for next launch.
fn change_options(workspace: &mut Workspace, change: impl Fn(&mut Options) + 'static) {
    change(&mut workspace.ui.settings.assistant);
    workspace.ui.settings.assistant = workspace.ui.settings.assistant.sound();
    let options = workspace.ui.settings.assistant;
    crate::app_settings::update(move |settings| settings.assistant = options);
}

/// Shows a suggestion in the timeline: its layer is selected and the playhead goes to the
/// frame it is about. For a suggestion about keyframes, that property's keyframe row opens.
fn show(workspace: &mut Workspace, suggestion: &Suggestion) {
    workspace.state.select_layer(suggestion.layer, false);
    workspace.state.set_current_frame(suggestion.frame);
    if let Some(path) = suggestion.path {
        let timeline = &mut workspace.ui.timeline;
        timeline.expanded_clips.insert(suggestion.layer);
        timeline.only_property.insert(suggestion.layer, path);
    }
}

/// Makes a suggestion's fix, as one step that Undo takes back.
fn fix(workspace: &mut Workspace, suggestion: &Suggestion) {
    if suggestion.fix.as_ref().is_some_and(|fix| workspace.state.apply_motion_fix(fix)) {
        feel(workspace, Mood::Happy);
    }
    wake(workspace);
}

/// Whether the viewer's corner may show the assistant: tips are switched on and not snoozed,
/// the assistant is one of the tools in use, nothing is playing, and its panel isn't already open.
fn tip_allowed(workspace: &Workspace) -> bool {
    let ui = &workspace.ui;
    ui.settings.assistant_tips && !ui.assistant.tips_snoozed && workspace.has(Feature::Assistant) && !workspace.state.is_playing() && ui.left_panel != crate::workspace::LeftPanel::Assistant
}

/// How the assistant is feeling, which its eyes show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    /// Nothing in particular: eyes open, glancing about now and then.
    #[default]
    Awake,
    /// Nothing has happened for a while: eyes shut.
    Asleep,
    /// Studio is busy with a long job (exporting, importing, analysing): eyes narrowed, darting.
    Working,
    /// The composition is playing: eyes wide.
    Watching,
    /// An answer is being typed: eyes turned up and aside.
    Thinking,
    /// Something just went well (a fix, a new copy): eyes curved into a smile.
    Happy,
    /// It had no answer: eyes drooping.
    Sad,
}

/// How the face is drawn at the moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub mood: Mood,
    /// The blink to play (see `face`), or `None` to hold still.
    pub blink: Option<u32>,
    /// Which way the eyes are turned while awake, each from -1 to 1: across, then down.
    pub glance: (f32, f32),
}

/// How long nothing must change before the assistant falls asleep.
const SLEEP_AFTER: std::time::Duration = std::time::Duration::from_secs(120);
/// How long it stays happy or sad after what caused it.
const FEELING_LASTS: std::time::Duration = std::time::Duration::from_secs(5);
/// The ways it glances while awake, taken in turn: mostly straight ahead, sometimes aside.
const GLANCES: [(f32, f32); 8] = [(0.0, 0.0), (-1.0, 0.0), (0.0, 0.0), (1.0, 0.0), (0.0, 0.0), (0.6, -0.8), (0.0, 0.0), (-0.6, 0.6)];

/// Notes that the person just did something with the assistant, so it wakes if it was asleep.
fn wake(workspace: &mut Workspace) {
    workspace.ui.assistant.active_at = Some(std::time::Instant::now());
}

/// Makes the assistant happy or sad for a few seconds.
fn feel(workspace: &mut Workspace, mood: Mood) {
    workspace.ui.assistant.feeling = Some((mood, std::time::Instant::now() + FEELING_LASTS));
}

/// Something that changes whenever the person works on the project, so the assistant can tell
/// whether anything has happened since it last looked.
fn activity(workspace: &Workspace) -> (i64, usize, usize, bool) {
    let state = &workspace.state;
    (state.current_frame, state.selected_layers.len(), state.composition().layers.len(), state.can_undo())
}

/// Whether Studio is in the middle of a long job.
fn busy(workspace: &Workspace) -> bool {
    let ui = &workspace.ui;
    ui.export_job.is_some() || ui.package_job.is_some() || ui.caption_job.is_some() || ui.analysis_job.is_some() || ui.import_job.is_some() || ui.cutout_job.is_some() || ui.enhance_job.is_some()
}

/// The assistant's mood now: what it is doing comes first, then how it feels, then sleep.
pub fn mood(workspace: &Workspace) -> Mood {
    let assistant = &workspace.ui.assistant;
    let now = std::time::Instant::now();
    if assistant.pending.is_some() {
        Mood::Thinking
    } else if busy(workspace) {
        Mood::Working
    } else if workspace.state.is_playing() {
        Mood::Watching
    } else if let Some((feeling, _)) = assistant.feeling.filter(|(_, until)| now < *until) {
        feeling
    } else if assistant.active_at.is_some_and(|at| now.duration_since(at) >= SLEEP_AFTER) {
        Mood::Asleep
    } else {
        Mood::Awake
    }
}

/// Called every few seconds: blinks the face and turns its eyes another way, and notes whether
/// anything has happened in the project (the assistant falls asleep when nothing has for a
/// while). Does nothing when the assistant isn't one of the tools in use.
pub fn blink(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if !workspace.has(Feature::Assistant) {
        return;
    }
    let seen = activity(workspace);
    let assistant = &mut workspace.ui.assistant;
    if assistant.active_at.is_none() || assistant.seen != Some(seen) {
        assistant.seen = Some(seen);
        assistant.active_at = Some(std::time::Instant::now());
    }
    assistant.blinks = assistant.blinks.wrapping_add(1);
    cx.notify();
    window.refresh();
}

/// How the face should be drawn now. With blinking switched off it holds still: no blinks and
/// no glancing about, though it still shows its mood.
pub fn current_look(workspace: &Workspace) -> Look {
    let blinks = workspace.ui.assistant.blinks;
    let lively = workspace.ui.settings.assistant_blinks && blinks > 0;
    Look { mood: mood(workspace), blink: lively.then_some(blinks), glance: if lively { GLANCES[blinks as usize % GLANCES.len()] } else { (0.0, 0.0) } }
}

/// The assistant's face: a circle in the accent color with two eyes, so its suggestions read
/// as coming from a helper rather than as error messages. The eyes show its mood (see `Mood`),
/// turn the way `look.glance` says, and close and open once each time `look.blink` changes.
/// `place` names where the face is drawn. The eyes narrow into a smile while the pointer is
/// over it.
pub fn face(place: &'static str, side: f32, look: Look) -> impl IntoElement {
    const GROUP: &str = "assistant-face";
    // The eyes as they are when simply awake, which each mood then reshapes.
    let (open_width, open_height, inset) = (side * 0.16, side * 0.32, side * 0.27);
    let (width, height, across, down) = match look.mood {
        Mood::Awake => (open_width, open_height, look.glance.0 * side * 0.06, look.glance.1 * side * 0.05),
        Mood::Asleep => (side * 0.2, side * 0.05, 0.0, side * 0.05),
        Mood::Working => (open_width, open_height * 0.45, 0.0, 0.0),
        Mood::Watching => (open_width * 1.1, open_height * 1.18, 0.0, 0.0),
        Mood::Thinking => (open_width, open_height, side * 0.06, -side * 0.08),
        Mood::Happy => (open_width * 1.15, open_height * 0.5, 0.0, -side * 0.02),
        Mood::Sad => (open_width * 1.1, open_height * 0.55, 0.0, side * 0.07),
    };
    let top = (side - height) / 2.0 + down;
    let eye = |index: u32| {
        let left = if index == 0 { inset + (open_width - width) / 2.0 } else { side - inset - open_width + (open_width - width) / 2.0 } + across;
        let smiling = open_height * 0.5;
        let shape = div().absolute().w(px(width)).h(px(height)).top(px(top)).left(px(left)).bg(gpui_kit::white());
        // A smile is flat underneath and a sad eye is flat on top; the rest are rounded all round.
        let shape = match look.mood {
            Mood::Happy => shape.rounded_t(px(width)),
            Mood::Sad => shape.rounded_b(px(width)),
            _ => shape.rounded_full().group_hover(GROUP, move |style| style.h(px(smiling)).top(px((side - smiling) / 2.0))),
        };
        match (look.mood, look.blink) {
            // Hard at work: the narrowed eyes dart from side to side for as long as the job runs.
            (Mood::Working, _) => shape
                .with_animation(ElementId::Name(format!("assistant-eye-working:{place}:{index}").into()), Animation::new(std::time::Duration::from_millis(1100)).repeat(), move |eye, progress| {
                    eye.left(px(left + side * 0.07 * (progress * std::f32::consts::TAU).sin()))
                })
                .into_any_element(),
            // Asleep, the eyes stay shut.
            (Mood::Asleep, _) | (_, None) => shape.into_any_element(),
            // Otherwise the eye squeezes shut around its middle and opens again.
            (_, Some(blink)) => shape
                .with_animation(ElementId::Name(format!("assistant-eye:{place}:{index}:{blink}").into()), Animation::new(BLINK_LENGTH), move |eye, progress| {
                    let shut = height * (1.0 - 0.88 * (progress * std::f32::consts::PI).sin());
                    eye.h(px(shut)).top(px(top + (height - shut) / 2.0))
                })
                .into_any_element(),
        }
    };
    div().group(GROUP).relative().size(px(side)).flex_none().rounded_full().bg(hsla(color::ACCENT)).child(eye(0)).child(eye(1))
}

/// Makes every safe fix among the open suggestions (see `Fix::is_safe`). They are all made
/// before the next undo step is recorded, so one Undo takes the lot back. Each fix is looked up afresh, as an earlier one can change what a later one
/// should do. Returns how many were made.
fn fix_all_safe(workspace: &mut Workspace) -> usize {
    let mut made = 0;
    let mut tried: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        let next = open_suggestions(workspace).into_iter().find(|suggestion| suggestion.fix.as_ref().is_some_and(|fix| fix.is_safe()) && !tried.contains(&suggestion.key()));
        let Some(suggestion) = next else { break };
        tried.insert(suggestion.key());
        if suggestion.fix.as_ref().is_some_and(|fix| workspace.state.apply_motion_fix(fix)) {
            made += 1;
        }
    }
    if made > 0 {
        feel(workspace, Mood::Happy);
    }
    made
}

/// The small colored label saying which area a suggestion is about.
fn area_tag(area: Area) -> impl IntoElement {
    let color = area_color(area);
    div().flex_none().px_1p5().rounded_full().bg(gpui_kit::Hsla::from(color).opacity(0.18)).text_size(px(size::CAPTION_TEXT)).text_color(color).child(area.name())
}

/// What the assistant says above its list.
fn greeting(count: usize) -> String {
    match count {
        0 => "Looking good. I have nothing to suggest.".to_owned(),
        1 => "I noticed one thing in this composition.".to_owned(),
        _ => format!("I noticed {count} things in this composition."),
    }
}

/// One suggestion: what was noticed, why it matters, and its buttons.
fn card(index: usize, suggestion: Suggestion, cx: &mut Context<Workspace>) -> impl IntoElement {
    let (for_fix, for_show, key) = (suggestion.clone(), suggestion.clone(), suggestion.key());
    let fix_button = suggestion.fix.as_ref().map(|found| {
        Button::new(("assistant-fix", index)).primary().small().label(found.label()).on_click(cx.listener(move |this, _, _, cx| {
            fix(this, &for_fix);
            cx.notify();
        }))
    });
    let show_button = Button::new(("assistant-show", index)).outline().small().label("Show").tooltip("Select the layer and go to this keyframe").on_click(cx.listener(move |this, _, _, cx| {
        show(this, &for_show);
        cx.notify();
    }));
    let dismiss = Button::new(("assistant-dismiss", index)).ghost().small().label("Dismiss").tooltip("Hide this suggestion").on_click(cx.listener(move |this, _, _, cx| {
        this.ui.assistant.dismissed.insert(key.clone());
        cx.notify();
    }));
    div()
        .id(("assistant-card", index))
        .mx_2()
        .p_2p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .rounded(px(8.0))
        .bg(hsla(color::RAISED))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().child(area_tag(suggestion.check.area())))
        .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(suggestion.title))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(suggestion.detail))
        // What was measured, so the suggestion can be judged rather than taken on trust.
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).opacity(0.75).child(suggestion.evidence))
        .child(div().pt_1().flex().items_center().gap_1p5().children(fix_button).child(show_button).child(div().flex_1()).child(dismiss))
        .test_support()
}

/// One message of the conversation: the person's on the right, the assistant's on the left
/// beside its face, with the buttons the message carries.
fn bubble(index: usize, message: &Message, newest: bool, workspace: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let lines = message.text.lines().map(|line| div().child(line.to_owned())).collect::<Vec<_>>();
    if message.from_person {
        let said = div().max_w(gpui_kit::relative(0.85)).px_2p5().py_1p5().rounded(px(10.0)).bg(hsla(color::ACCENT).opacity(0.18)).children(lines);
        return div().id(("assistant-message", index)).px_2().flex().justify_end().child(said).test_support().into_any_element();
    }
    let do_it = message.command.map(|command| {
        Button::new(("assistant-message-do", index)).outline().small().label("Do It").tooltip(crate::keymap::with_key(&workspace.ui.settings.shortcuts, command.name, command.id)).on_click(cx.listener(move |_, _, window, cx| {
            window.dispatch_action(command.action(), cx);
            cx.notify();
        }))
    });
    let open = message.opens_suggestions.then(|| {
        Button::new(("assistant-message-suggestions", index)).outline().small().label("Open Suggestions").on_click(cx.listener(|this, _, _, cx| {
            this.ui.assistant.chat_chosen = Some(false);
            cx.notify();
        }))
    });
    let buttons = (do_it.is_some() || open.is_some()).then(|| div().pt_1().flex().gap_1p5().children(do_it).children(open));
    // Short replies are offered under the newest answer only: an older "Undo that" would
    // undo something else by now.
    let replies = (newest && !message.replies.is_empty()).then(|| {
        let reply = |(number, text): (usize, &String)| {
            let said = text.clone();
            Button::new(ElementId::Name(format!("assistant-reply-{number}").into())).ghost().small().label(text.clone()).on_click(cx.listener(move |this, _, window, cx| {
                send(this, &said, window, cx);
                cx.notify();
            }))
        };
        div().id("assistant-replies").pt_1().flex().flex_wrap().gap_1().children(message.replies.iter().enumerate().map(reply).collect::<Vec<_>>()).test_support()
    });
    let said = div().flex_1().min_w_0().px_2p5().py_1p5().flex().flex_col().gap_0p5().rounded(px(10.0)).bg(hsla(color::RAISED)).border_1().border_color(hsla(color::HAIRLINE)).children(lines).children(buttons).children(replies);
    div().id(("assistant-message", index)).px_2().flex().items_start().gap_1p5().child(face("message", 20.0, Look::default())).child(said).test_support().into_any_element()
}

/// The conversation: a welcome until something has been said, then every message in order.
fn conversation(workspace: &Workspace, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let messages = &workspace.ui.assistant.messages;
    if messages.is_empty() {
        let welcome = Message::from_assistant(format!(
            "Hello! I'm {}. Ask me about this composition, like \"how long is the video?\", tell me what to do, like \"fade the title in\" or \"go to 5s\", or ask how to do something in Omina Motion.\nType /help for everything I understand.",
            name(&workspace.ui.settings)
        ));
        return vec![div().id("assistant-welcome").child(bubble(0, &welcome, false, workspace, cx)).test_support().into_any_element()];
    }
    let newest = messages.len() - 1;
    let restored = workspace.ui.assistant.restored;
    let mut bubbles: Vec<AnyElement> = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        // The messages kept from last time end at a quiet "Earlier" line.
        if index == restored && restored > 0 {
            bubbles.push(super::assistant_automation::earlier_divider());
        }
        bubbles.push(bubble(index, message, index == newest, workspace, cx));
    }
    if workspace.ui.assistant.pending.is_some() {
        bubbles.push(typing_dots().into_any_element());
    }
    bubbles
}

/// The assistant "typing": its face beside three dots that brighten one after another.
fn typing_dots() -> impl IntoElement {
    let dot = |index: usize| {
        div().size(px(6.0)).rounded_full().bg(hsla(color::MUTED)).with_animation(("assistant-typing-dot", index), Animation::new(std::time::Duration::from_millis(900)).repeat(), move |dot, progress| {
            // Each dot is brightest a third of the way after the one before it.
            let phase = (progress - index as f32 / 3.0).rem_euclid(1.0);
            dot.opacity(0.3 + 0.7 * (1.0 - (phase * 2.0 - 1.0).abs()).clamp(0.0, 1.0))
        })
    };
    let dots = div().h(px(30.0)).px_3().flex().items_center().gap_1().rounded(px(10.0)).bg(hsla(color::RAISED)).border_1().border_color(hsla(color::HAIRLINE)).children((0..3).map(dot));
    div().id("assistant-typing").px_2().flex().items_start().gap_1p5().child(face("typing", 20.0, Look { mood: Mood::Thinking, ..Look::default() })).child(dots).test_support()
}

/// A number setting with − and + buttons, kept inside `range`.
fn stepper(id: &'static str, label: &'static str, value: i64, range: (i64, i64), set: fn(&mut Options, i64), cx: &mut Context<Workspace>) -> impl IntoElement {
    let step = |suffix: &'static str, by: i64, text: &'static str| {
        let next = (value + by).clamp(range.0, range.1);
        Button::new(ElementId::Name(format!("{id}-{suffix}").into())).outline().small().label(text).on_click(cx.listener(move |this, _, _, cx| {
            change_options(this, move |options| set(options, next));
            cx.notify();
        }))
    };
    div()
        .pl(px(26.0))
        .flex()
        .items_center()
        .gap_2()
        .text_size(px(size::SMALL_TEXT))
        .text_color(hsla(color::MUTED))
        .child(div().flex_1().min_w_0().child(label))
        .child(step("less", -1, "−"))
        .child(div().id(id).w(px(64.0)).text_center().text_color(hsla(color::TEXT)).child(format!("{value} {}", if value == 1 { "frame" } else { "frames" })).test_support())
        .child(step("more", 1, "+"))
}

/// The rules, each with a switch, and the settings that go with them.
fn settings(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement {
    let options = workspace.ui.settings.assistant;
    let mut rows: Vec<AnyElement> = Vec::new();
    for (index, check) in Check::ALL.into_iter().enumerate() {
        // Each area's rules sit under its name.
        if index == 0 || Check::ALL[index - 1].area() != check.area() {
            rows.push(div().pt_1().flex().child(area_tag(check.area())).into_any_element());
        }
        let on = options.runs(check);
        let switch = Checkbox::new(("assistant-check", index)).checked(on).on_click(cx.listener(move |this, _, _, cx| {
            change_options(this, move |options| options.set(check, !on));
            cx.notify();
        }));
        rows.push(
            div()
                .flex()
                .items_start()
                .gap_2()
                .child(div().pt(px(1.0)).child(switch))
                .child(div().flex_1().min_w_0().flex().flex_col().child(check.name()).child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(check.about())))
                .into_any_element(),
        );
        // The two rules that depend on a number carry it under their switch.
        match check {
            Check::FastMove if on => rows.push(stepper("assistant-shortest", "Too fast is under", options.shortest_move, Options::SHORTEST_MOVE_RANGE, |options, value| options.shortest_move = value, cx).into_any_element()),
            Check::NearlyAligned if on => rows.push(stepper("assistant-within", "Nearly lined up is within", options.aligned_within, Options::ALIGNED_WITHIN_RANGE, |options, value| options.aligned_within = value, cx).into_any_element()),
            _ => {}
        }
    }
    let tips_on = workspace.ui.settings.assistant_tips;
    let tips = Checkbox::new("assistant-tips").checked(tips_on).label("Show tips in the viewer").on_click(cx.listener(move |this, _, _, cx| {
        this.ui.settings.assistant_tips = !tips_on;
        this.ui.assistant.tips_snoozed = false;
        crate::app_settings::update(move |settings| settings.assistant_tips = !tips_on);
        cx.notify();
    }));
    div()
        .id("assistant-settings-list")
        .mx_2()
        .p_2p5()
        .flex()
        .flex_col()
        .gap_2()
        .rounded(px(8.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        // First, so they stay in view above the long list of rules.
        .child(tips)
        .child(
            div()
                .flex()
                .gap_1p5()
                .child(Button::new("assistant-all-settings").outline().small().label("All Iris Settings…").on_click(cx.listener(|this, _, window, cx| {
                    super::settings::open_iris(this, window, cx);
                })))
                // Her recipes, with their shortcuts, buttons and moments.
                .child(Button::new("assistant-recipes").outline().small().label("Recipes…").on_click(cx.listener(|this, _, window, cx| {
                    super::settings::open_recipes(this, window, cx);
                }))),
        )
        .child(div().h(px(1.0)).bg(hsla(color::HAIRLINE)))
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child("WHAT I LOOK FOR"))
        .children(rows)
        .test_support()
}

pub fn render(workspace: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    let found = open_suggestions(workspace);
    let count = found.len();
    let dismissed = workspace.ui.assistant.dismissed.len();
    let settings_open = workspace.ui.assistant.settings_open;
    let gear = Button::new("assistant-settings").ghost().small().icon(Icon::new(IconName::Settings)).selected(settings_open).tooltip("Choose what the assistant looks for").on_click(cx.listener(|this, _, _, cx| {
        this.ui.assistant.settings_open = !this.ui.assistant.settings_open;
        cx.notify();
    }));
    let header = div()
        .px_2p5()
        .pt_2()
        .flex()
        .items_center()
        .gap_2p5()
        // A click on the face gets a blink and the next tip, in the conversation.
        .child(div().id("assistant-face").cursor_pointer().child(face("panel", 32.0, current_look(workspace))).on_click(cx.listener(|this, _, window, cx| {
            give_tip(this);
            wake(this);
            this.ui.assistant.chat_chosen = Some(true);
            this.ui.assistant.blinks = this.ui.assistant.blinks.wrapping_add(1);
            cx.on_next_frame(window, |this, _, cx| {
                this.ui.assistant.chat_scroll.scroll_to_bottom();
                cx.notify();
            });
            cx.notify();
        })).test_support())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().id("assistant-name").font_weight(gpui_kit::FontWeight::SEMIBOLD).child(name(&workspace.ui.settings)).test_support())
                .child(div().id("assistant-greeting").text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(greeting(found.len())).test_support()),
        )
        .child(gear);
    // Dismissed suggestions can be brought back, so dismissing one is never final.
    let restore = (dismissed > 0).then(|| {
        div().px_2().child(Button::new("assistant-restore").ghost().small().label(format!("Bring back {dismissed} dismissed")).on_click(cx.listener(|this, _, _, cx| {
            this.ui.assistant.dismissed.clear();
            cx.notify();
        })))
    });
    // One button makes every fix that can't change anything else about the piece.
    let safe = found.iter().filter(|suggestion| suggestion.fix.as_ref().is_some_and(|fix| fix.is_safe())).count();
    let fix_all = (safe > 1).then(|| {
        div().px_2().flex().child(Button::new("assistant-fix-all").outline().small().label(format!("Fix All {safe} Safe")).tooltip("Makes the fixes that only remove a problem and change nothing else. One Undo takes them all back.").on_click(cx.listener(
            |this, _, _, cx| {
                fix_all_safe(this);
                cx.notify();
            },
        )))
    });
    // The areas with something in them, to narrow the list to one. The cards keep their place
    // in the whole list as their number, whichever area is showing.
    let shown_area = workspace.ui.assistant.area.filter(|area| found.iter().any(|suggestion| suggestion.check.area() == *area));
    let chip = |id: usize, label: String, area: Option<Area>, cx: &mut Context<Workspace>| {
        Button::new(("assistant-area", id)).ghost().small().label(label).selected(shown_area == area).on_click(cx.listener(move |this, _, _, cx| {
            this.ui.assistant.area = area;
            cx.notify();
        }))
    };
    let areas: Vec<(usize, Area, usize)> = Area::ALL.into_iter().enumerate().map(|(index, area)| (index + 1, area, found.iter().filter(|suggestion| suggestion.check.area() == area).count())).filter(|(_, _, count)| *count > 0).collect();
    let chips = (areas.len() > 1).then(|| {
        let mut row = div().id("assistant-areas").px_2().flex().flex_wrap().gap_1().child(chip(0, format!("All {}", found.len()), None, cx));
        for (id, area, count) in areas {
            row = row.child(chip(id, format!("{} {count}", area.name()), Some(area), cx));
        }
        row.test_support()
    });
    let cards = found
        .into_iter()
        .enumerate()
        .filter(|(_, suggestion)| shown_area.is_none_or(|area| suggestion.check.area() == area))
        .map(|(index, suggestion)| card(index, suggestion, cx).into_any_element())
        .collect::<Vec<_>>();
    // The two tabs. Until one is chosen, Suggestions shows when there is something to suggest.
    let chatting = workspace.ui.assistant.chat_chosen.unwrap_or(count == 0);
    let tab = |id: &'static str, label: String, chat: bool, cx: &mut Context<Workspace>| {
        Button::new(id).ghost().small().label(label).selected(chatting == chat).on_click(cx.listener(move |this, _, _, cx| {
            this.ui.assistant.chat_chosen = Some(chat);
            cx.notify();
        }))
    };
    let tabs = div().px_2().flex().gap_1().child(tab("assistant-tab-chat", "Chat".to_owned(), true, cx)).child(tab("assistant-tab-suggestions", if count == 0 { "Suggestions".to_owned() } else { format!("Suggestions {count}") }, false, cx));
    let list = div().id("assistant-list").flex_1().min_h_0().overflow_y_scroll().flex().flex_col().gap_2().pb_3().when(settings_open, |list| list.child(settings(workspace, cx)));
    let list = if chatting { list.track_scroll(&workspace.ui.assistant.chat_scroll).children(conversation(workspace, cx)) } else { list.children(chips).children(fix_all).children(cards).children(restore) };
    // The message field stays at the foot of the panel under both tabs; Return sends.
    let field = workspace.ui.assistant.ask.as_ref().map(|(input, _)| {
        div()
            .id("assistant-ask")
            .flex_none()
            .mx_2()
            .mb_2()
            .h(px(32.0))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(8.0))
            .bg(hsla(color::INSET))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .child(div().flex_1().min_w_0().child(Input::new(input).appearance(false).small()))
            // Up and down go back through what was typed, and Tab takes a completion, before
            // the field moves its cursor; when they don't apply, the field gets them as usual.
            .capture_action(cx.listener(|this, _: &MoveUp, window, cx| {
                if super::assistant_automation::recall(this, true, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, window, cx| {
                if super::assistant_automation::recall(this, false, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if super::assistant_automation::complete_first(this, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .test_support()
    });
    let completions = super::assistant_automation::completions(workspace, cx);
    let recipe_buttons = chatting.then(|| super::assistant_automation::recipe_buttons(workspace, cx)).flatten();
    let body = div().size_full().flex().flex_col().gap_2().child(header).child(tabs).children(recipe_buttons).child(list).children(completions).children(field);
    panel(PanelId::Side, &[], None, body, workspace, cx)
}

/// The assistant in a corner of the viewer: its face, with a count while it has something to
/// suggest. A click opens the first suggestion with its fix, a way to see the rest, and "Not
/// Now"; with nothing to suggest, a click opens its panel, where it can be asked for help. It
/// stays folded until clicked so it never covers the picture uninvited (`tip_allowed` says
/// when it may show at all).
pub fn tip(workspace: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let ui = &workspace.ui;
    if !tip_allowed(workspace) {
        return None;
    }
    let look = current_look(workspace);
    let found = open_suggestions(workspace);
    let Some(suggestion) = found.first().cloned().filter(|_| ui.assistant.tip_open) else {
        // Folded: the face and how many suggestions are waiting.
        let badge = div()
            .id("assistant-tip-face")
            .absolute()
            .right(px(10.0))
            .bottom(px(10.0))
            // Kept this small so it stays clear of a full-frame layer's corner handle.
            .h(px(28.0))
            .pl(px(2.0))
            .pr(px(if found.is_empty() { 2.0 } else { 8.0 }))
            .flex()
            .items_center()
            .gap_1()
            .rounded_full()
            .bg(hsla(color::RAISED))
            .border_1()
            .border_color(hsla(color::HAIRLINE))
            .text_size(px(size::SMALL_TEXT))
            .text_color(hsla(color::TEXT))
            .cursor_pointer()
            .hover(|style| style.border_color(hsla(color::ACCENT)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                if open_suggestions(this).is_empty() {
                    this.ui.left_panel = crate::workspace::LeftPanel::Assistant;
                    this.reveal(PanelId::Side);
                } else {
                    this.ui.assistant.tip_open = true;
                }
                cx.notify();
            }))
            .child(face("badge", 24.0, look))
            .children((!found.is_empty()).then(|| format!("{}", found.len())))
            .test_support();
        return Some(badge.into_any_element());
    };
    let more = found.len() - 1;
    let for_fix = suggestion.clone();
    let fix_button = suggestion.fix.as_ref().map(|found| {
        Button::new("assistant-tip-fix").primary().small().label(found.label()).on_click(cx.listener(move |this, _, _, cx| {
            fix(this, &for_fix);
            cx.notify();
        }))
    });
    let all = Button::new("assistant-tip-all").outline().small().label(if more == 0 { "Open".to_owned() } else { format!("See All {}", more + 1) }).on_click(cx.listener(|this, _, _, cx| {
        this.ui.left_panel = crate::workspace::LeftPanel::Assistant;
        this.reveal(PanelId::Side);
        cx.notify();
    }));
    let later = Button::new("assistant-tip-later").ghost().small().label("Not Now").tooltip("No more tips until Omina Motion is next opened").on_click(cx.listener(|this, _, _, cx| {
        this.ui.assistant.tips_snoozed = true;
        this.ui.assistant.tip_open = false;
        cx.notify();
    }));
    let bubble = super::chrome::popup_surface(div().id("assistant-tip"))
        .absolute()
        .right(px(12.0))
        .bottom(px(12.0))
        .w(px(300.0))
        .p_2p5()
        .flex()
        .flex_col()
        .gap_1p5()
        .text_size(px(size::TEXT))
        .text_color(hsla(color::TEXT))
        .cursor(gpui_kit::CursorStyle::Arrow)
        // A press on the tip is for the tip: it must not start a drag or a selection in the viewer.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(face("tip", 28.0, look))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().id("assistant-tip-name").text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} noticed", name(&ui.settings))).test_support())
                        .child(div().font_weight(gpui_kit::FontWeight::MEDIUM).child(suggestion.title)),
                )
                // Folds the tip back down to the face.
                .child(Button::new("assistant-tip-fold").ghost().xsmall().icon(Icon::new(IconName::ChevronDown)).tooltip("Fold away").on_click(cx.listener(|this, _, _, cx| {
                    this.ui.assistant.tip_open = false;
                    cx.notify();
                }))),
        )
        .child(div().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(suggestion.detail))
        .child(div().text_size(px(size::CAPTION_TEXT)).text_color(hsla(color::MUTED)).opacity(0.75).child(suggestion.evidence))
        .child(div().pt_1().flex().items_center().gap_1p5().children(fix_button).child(all).child(div().flex_1()).child(later))
        .test_support();
    Some(bubble.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_trimmed_kept_short_and_falls_back_to_the_usual_one() {
        assert_eq!(tidy_name("  Pip  "), "Pip");
        assert_eq!(tidy_name("A very long name for a small face").chars().count(), iris::settings::NAME_LIMIT);
        let mut settings = AppSettings::default();
        assert_eq!(name(&settings), DEFAULT_NAME);
        settings.assistant_name = "   ".to_owned();
        assert_eq!(name(&settings), DEFAULT_NAME, "an empty name is never shown");
        settings.assistant_name = "Pip".to_owned();
        assert_eq!(name(&settings), "Pip");
    }
}
