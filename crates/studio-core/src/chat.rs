//! Talking to Iris, the assistant. A typed message is matched against a fixed list of things
//! people ask (by the words it contains) and answered from what the project already records:
//! how long the composition is, its settings, its layers, tracks, sound, keyframes and so on.
//! There is no AI here and nothing is made up: a message that fits none of the patterns gets
//! `Reply::Unknown`, and the caller may look for a ready-written help answer instead.
//!
//! Messages starting with `/` are commands (`/help`, `/clear`, ...); see `COMMANDS`.
//!
//! A message that asks for something to be done is read in this order, the first reader that
//! is certain winning: Motion's own chores (`chores`), housekeeping for layers
//! (`iris::layers`), a menu command named in full, instructions about timing and motion
//! (`instructions`), animating the design, another size, and then the questions. A command
//! named only in part ("export", "guides") is run when nothing else fits.

use std::collections::BTreeMap;

use iris::commands::Found;
use iris::intent::Target;

use crate::assistant::{Area, Suggestion};
use crate::model::{Composition, Id, Layer, LayerContent, Project, ProjectItemKind};
use crate::property::{PropertyPath, PropertyValue, TransformProperty};
use crate::time::format_timecode;

/// What the assistant can see when it answers.
pub struct Scene<'a> {
    pub project: &'a Project,
    /// The composition open in the timeline.
    pub composition: &'a Composition,
    /// Where the playhead is.
    pub current_frame: i64,
    pub selected: &'a [Id],
    /// The work area's first frame and the frame after its last.
    pub work_area: (i64, i64),
    /// What the assistant is called.
    pub name: &'a str,
    /// What its rules currently notice in the composition.
    pub suggestions: &'a [Suggestion],
    /// The app's own commands, by the names its menus show, so one can be asked for by name.
    pub commands: &'a [iris::commands::Command<'a>],
}

/// Something the app does for a reply, beyond showing its words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Empty the conversation.
    Clear,
    /// Show the list of suggestions.
    ShowSuggestions,
    /// Make every safe fix.
    FixAllSafe,
    /// Open Settings.
    OpenSettings,
    /// Call the assistant something else.
    Rename(String),
    /// Give the next tip.
    Tip,
    /// Make a copy of the open composition at another size, rearranged to suit it. `label`
    /// names the size in the copy's name ("9:16", "720p").
    Resize { width: u32, height: u32, label: String },
    /// Animate the design (see `auto_animate`): every layer, or with `selection` only the
    /// selected ones, in `style`.
    Animate { selection: bool, style: Option<crate::auto_animate::Style> },
    /// Carry out instructions about layers' timing and motion ("start at 5s and fade in"; see
    /// `instructions`).
    Instruct(iris::intent::Request),
    /// Carry out housekeeping for layers ("hide the logo", "duplicate it"; see `housekeeping`).
    Housekeep(iris::layers::Request),
    /// The message could mean several things: ask which, with these as replies to pick from.
    Ask(Vec<String>),
    /// Do one of Motion's own chores (see `chores`).
    Chore(crate::chores::Request),
    /// Run one of the app's commands: its place in `Scene::commands`.
    Run(usize),
    /// Several commands fit the message equally well: offer them rather than choosing.
    Offer(Vec<usize>),
}

/// The assistant's answer to one message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Words to show.
    Say(String),
    /// Words to show, and something for the app to do.
    Do(String, Action),
    /// The message fits nothing the assistant knows how to answer from the project.
    Unknown,
}

/// The commands, with what each does, in the order `/help` lists them.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/help", "List these commands and what you can ask"),
    ("/clear", "Clear this conversation"),
    ("/check", "Show what I've noticed in this composition"),
    ("/fix", "Make every safe fix at once"),
    ("/comp", "This composition's settings"),
    ("/layers", "List the layers"),
    ("/tracks", "Count the video and audio tracks"),
    ("/media", "Count what is in the project"),
    ("/selected", "Describe the selected layer"),
    ("/resize", "Copy this composition at another size: /resize vertical, /resize 1280x720"),
    ("/animate", "Animate the design for you: /animate, /animate bouncy, /animate snappy"),
    ("/ready", "Check the project before you export"),
    ("/recipes", "List the recipes I know"),
    ("/teach", "Teach me a recipe: /teach intro = fade in; pulse"),
    ("/forget", "Forget a recipe: /forget intro"),
    ("/tip", "Give a tip"),
    ("/name", "Rename me: /name Pixel"),
    ("/settings", "Open Settings"),
];

/// What a message is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    Hello,
    Goodbye,
    Thanks,
    HowAreYou,
    WhoAreYou,
    AreYouAi,
    WhatCanYouDo,
    Duration,
    FrameRate,
    Size,
    AspectRatio,
    Background,
    FrameCount,
    CompSettings,
    CompName,
    Playhead,
    TimeLeft,
    WorkArea,
    VideoTracks,
    AudioTracks,
    Tracks,
    TextLayers,
    HiddenLayers,
    LockedLayers,
    LongestClip,
    ShortestClip,
    FirstClip,
    LastClip,
    Selected,
    Effects,
    Keyframes,
    Animated,
    SoundLayers,
    Muted,
    Markers,
    Captions,
    Fonts,
    UnusedMedia,
    Compositions,
    Media,
    ListLayers,
    LayerKinds,
    LayerCount,
    AreaProblems(Area),
    Problems,
    FixAll,
    Tip,
}

/// One pattern: the message must contain a word from every group to mean `intent`.
struct Pattern {
    intent: Intent,
    groups: &'static [&'static [&'static str]],
}

const fn pattern(intent: Intent, groups: &'static [&'static [&'static str]]) -> Pattern {
    Pattern { intent, groups }
}

/// Words asking "how many" or "which".
const COUNT: &[&str] = &["many", "count", "number", "total", "amount"];
const WHICH: &[&str] = &["which", "what", "list", "show", "name", "any", "are", "is", "tell"];
const LAYER: &[&str] = &["layer", "clip", "element"];
const TRACK: &[&str] = &["track", "lane", "row"];
const COMP: &[&str] = &["comp", "composition", "video", "project", "sequence", "timeline", "edit", "film", "movie", "piece"];
const PROBLEM: &[&str] = &["wrong", "problem", "issue", "mistake", "error", "suggestion", "suggest", "notice", "noticed", "check", "review", "improve", "fix", "feedback", "better", "bad"];

/// Greetings and questions about the assistant itself. These are tried only when a message
/// fits nothing in `PATTERNS`, so "hey, how long is the video?" is answered, not just greeted.
const SMALL_TALK: &[Pattern] = &[
    pattern(Intent::AreYouAi, &[&["ai", "robot", "human", "real", "chatgpt", "gpt", "llm", "bot", "smart", "intelligent"], &["you", "are", "this", "is", "it"]]),
    pattern(Intent::WhoAreYou, &[&["who", "what"], &["you", "your"], &["are", "name", "called"]]),
    pattern(Intent::WhatCanYouDo, &[&["what", "how"], &["can", "do", "able", "help", "know", "work"], &["you"]]),
    pattern(Intent::WhatCanYouDo, &[&["what"], &["ask"]]),
    pattern(Intent::WhatCanYouDo, &[&["help"]]),
    pattern(Intent::HowAreYou, &[&["how"], &["are", "is"], &["you", "going", "thing"]]),
    pattern(Intent::Hello, &[&["hello", "hi", "hey", "hiya", "howdy", "yo", "morning", "afternoon", "evening", "greeting", "sup"]]),
    pattern(Intent::Goodbye, &[&["bye", "goodbye", "cya", "later", "night", "farewell", "done", "quit", "exit"]]),
    pattern(Intent::Goodbye, &[&["see"], &["you", "ya"]]),
    pattern(Intent::Thanks, &[&["thank", "thanks", "thx", "ty", "cheers", "appreciate", "great", "awesome", "nice", "perfect", "cool"]]),
];

/// Every question about the project. When several fit a message, the one with the most groups
/// wins (it is the most specific), and the earlier one on a tie, so the order here matters.
const PATTERNS: &[Pattern] = &[
    // What the rules have noticed.
    pattern(Intent::FixAll, &[&["fix", "repair", "clean", "tidy", "sort"], &["all", "everything", "every", "them", "it", "up"]]),
    pattern(Intent::AreaProblems(Area::Audio), &[PROBLEM, &["audio", "sound", "music", "volume", "mix"]]),
    pattern(Intent::AreaProblems(Area::Motion), &[PROBLEM, &["motion", "animation", "keyframe", "easing", "move"]]),
    pattern(Intent::AreaProblems(Area::Editing), &[PROBLEM, &["edit", "editing", "cut", "gap", "clip", "timeline"]]),
    pattern(Intent::AreaProblems(Area::Text), &[PROBLEM, &["text", "title", "caption", "subtitle"]]),
    pattern(Intent::AreaProblems(Area::Picture), &[PROBLEM, &["picture", "image", "footage", "quality", "sharp", "soft", "scale"]]),
    pattern(Intent::AreaProblems(Area::Editing), &[&["gap", "jump", "sliver"], WHICH]),
    pattern(Intent::Problems, &[PROBLEM]),
    pattern(Intent::Problems, &[&["look", "looking"], &["good", "ok", "okay", "fine", "right"]]),
    // The composition.
    pattern(Intent::FrameCount, &[COUNT, &["frame"]]),
    pattern(Intent::FrameRate, &[&["fps", "framerate"]]),
    pattern(Intent::FrameRate, &[&["frame"], &["rate", "second", "per", "speed"]]),
    pattern(Intent::AspectRatio, &[&["aspect", "ratio", "vertical", "horizontal", "portrait", "landscape", "square", "widescreen"]]),
    pattern(Intent::Size, &[&["size", "resolution", "dimension", "width", "height", "wide", "tall", "big", "pixel", "4k", "1080p", "hd"]]),
    pattern(Intent::Background, &[&["background", "backdrop"]]),
    pattern(Intent::TimeLeft, &[&["left", "remaining", "remain"], &["how", "much", "time", "long", "frame"]]),
    pattern(Intent::Duration, &[&["long", "length", "duration", "runtime", "run"], COMP]),
    pattern(Intent::Duration, &[&["duration", "runtime", "length"]]),
    pattern(Intent::Duration, &[&["how"], &["long"]]),
    pattern(Intent::Duration, &[&["when"], &["end", "finish", "over"]]),
    pattern(Intent::CompName, &[&["name", "called", "title"], &["comp", "composition", "sequence", "timeline"]]),
    pattern(Intent::CompSettings, &[&["setting", "setup", "spec", "format", "detail", "info", "about", "summary", "overview", "stat"], COMP]),
    pattern(Intent::CompSettings, &[&["setting", "spec", "summary", "overview", "stat"]]),
    // Time.
    pattern(Intent::Playhead, &[&["playhead", "cursor", "scrubber"]]),
    pattern(Intent::Playhead, &[&["where"], &["am", "i"]]),
    pattern(Intent::Playhead, &[&["what", "which", "current"], &["frame", "time", "timecode"], &["i", "on", "at", "current", "now", "am"]]),
    pattern(Intent::WorkArea, &[&["work", "preview", "render"], &["area", "range", "region"]]),
    // Tracks.
    pattern(Intent::VideoTracks, &[TRACK, &["video", "picture", "visual"]]),
    pattern(Intent::AudioTracks, &[TRACK, &["audio", "sound", "music"]]),
    pattern(Intent::Tracks, &[TRACK]),
    // Layers.
    pattern(Intent::Selected, &[&["selected", "selection", "select", "picked", "highlighted"]]),
    pattern(Intent::Selected, &[&["this", "that", "current"], LAYER]),
    pattern(Intent::TextLayers, &[&["text", "title", "word", "written", "say", "type", "typed"], &["layer", "what", "which", "list", "show", "any", "many", "count"]]),
    pattern(Intent::HiddenLayers, &[&["hidden", "invisible", "off", "disabled"], LAYER]),
    pattern(Intent::HiddenLayers, &[&["hidden", "invisible"]]),
    pattern(Intent::LockedLayers, &[&["locked", "lock"]]),
    pattern(Intent::LongestClip, &[&["longest", "biggest"], &["layer", "clip", "shot", "one"]]),
    pattern(Intent::LongestClip, &[&["longest"]]),
    pattern(Intent::ShortestClip, &[&["shortest", "smallest", "briefest"]]),
    pattern(Intent::FirstClip, &[&["first", "earliest", "opening", "start", "begin"], &["layer", "clip", "shot"]]),
    pattern(Intent::LastClip, &[&["last", "latest", "final", "closing", "end"], &["layer", "clip", "shot"]]),
    pattern(Intent::Effects, &[&["effect", "fx", "filter", "lut", "blur", "glow"]]),
    pattern(Intent::Keyframes, &[COUNT, &["keyframe", "key"]]),
    pattern(Intent::Animated, &[&["animated", "animation", "animate", "moving", "move", "motion", "keyframe", "keyframed"], &["what", "which", "list", "show", "any", "is", "are", "layer"]]),
    pattern(Intent::Muted, &[&["muted", "mute", "silent", "silenced"]]),
    pattern(Intent::SoundLayers, &[&["audio", "sound", "music", "volume", "loud", "level", "voice", "voiceover"], &["layer", "clip", "what", "which", "list", "show", "any", "many", "how", "count", "is", "are"]]),
    pattern(Intent::Markers, &[&["marker", "beat", "cue"]]),
    pattern(Intent::Captions, &[&["caption", "subtitle", "transcript"]]),
    pattern(Intent::Fonts, &[&["font", "typeface", "typography"]]),
    // The project.
    pattern(Intent::UnusedMedia, &[&["unused", "unneeded", "leftover", "orphan"]]),
    pattern(Intent::UnusedMedia, &[&["not", "never", "isn", "aren"], &["used", "use"]]),
    pattern(Intent::Compositions, &[COUNT, &["comp", "composition", "sequence", "precomp"]]),
    pattern(Intent::Compositions, &[&["list", "which", "what", "show", "other"], &["comps", "compositions", "sequences", "precomp", "precomps"]]),
    pattern(Intent::Tip, &[&["tip", "hint", "trick", "advice", "teach", "learn"]]),
];

/// Questions about the layers in general. These are tried after `PATTERNS`, so a question that
/// names something particular ("what effects are on my layers") isn't taken for one of these.
const GENERAL: &[Pattern] = &[
    pattern(Intent::Media, &[&["media", "file", "footage", "asset", "import", "imported", "picture", "image", "photo"], &["many", "count", "number", "what", "which", "list", "show", "total", "how", "any"]]),
    pattern(Intent::LayerKinds, &[&["kind", "type", "sort"], LAYER]),
    pattern(Intent::ListLayers, &[&["list", "name", "show", "which", "what"], LAYER]),
    pattern(Intent::LayerCount, &[COUNT, LAYER]),
    pattern(Intent::LayerCount, &[COUNT, &["shot", "thing", "item", "object"]]),
];

/// Words that ask for the composition to be made into another size.
const RESIZE_WORDS: &[&str] = &["convert", "resize", "resized", "make", "change", "turn", "create", "version", "copy", "reformat", "adapt", "export", "need", "want", "give", "scale", "crop"];

/// The sizes that can be asked for by name: the words or ratios that mean each (any one is
/// enough), then its width, height and the label used in the copy's name. Earlier entries win,
/// so "4k" is found before the plainer "landscape".
const NAMED_SIZES: &[(&[&str], u32, u32, &str)] = &[
    (&["4k", "uhd", "2160p"], 3840, 2160, "4K"),
    (&["720p"], 1280, 720, "720p"),
    (&["4:5", "4x5"], 1080, 1350, "4:5"),
    (&["2:3", "pinterest"], 1000, 1500, "2:3"),
    (&["1:1", "square"], 1080, 1080, "1:1"),
    (&["9:16", "vertical", "portrait", "tiktok", "reel", "reels", "short", "shorts", "story", "stories", "phone", "tall"], 1080, 1920, "9:16"),
    (&["16:9", "landscape", "horizontal", "widescreen", "youtube", "1080p", "hd", "wide"], 1920, 1080, "16:9"),
];

/// The smallest and largest side a composition can be asked to be, in pixels.
const SIDE_RANGE: (u32, u32) = (16, 8192);

/// The size a message asks for, with the label for the copy's name: either two numbers
/// ("1280x720", "1280 by 720", "1280 × 720") or a size known by name ("vertical", "4:5", "4K").
pub(crate) fn wanted_size(message: &str) -> Option<(u32, u32, String)> {
    let lower = message.to_lowercase();
    // Two numbers with "x", "×" or "by" between them, with or without spaces.
    let spaced = lower.replace('×', " x ").replace(',', "");
    let mut tokens: Vec<String> = Vec::new();
    for token in spaced.split_whitespace() {
        match token.split_once('x') {
            Some((width, height)) if !width.is_empty() && !height.is_empty() && width.chars().all(|c| c.is_ascii_digit()) => tokens.extend([width.to_owned(), "x".to_owned(), height.trim_end_matches(|c: char| !c.is_ascii_digit()).to_owned()]),
            _ => tokens.push(token.trim_matches(|c: char| !c.is_alphanumeric() && c != ':').to_owned()),
        }
    }
    let in_range = |side: u32| (SIDE_RANGE.0..=SIDE_RANGE.1).contains(&side);
    for trio in tokens.windows(3) {
        if let (Ok(width), true, Ok(height)) = (trio[0].parse::<u32>(), trio[1] == "x" || trio[1] == "by", trio[2].parse::<u32>())
            && in_range(width)
            && in_range(height)
        {
            return Some((width, height, format!("{width}x{height}")));
        }
    }
    NAMED_SIZES.iter().find(|(names, ..)| names.iter().any(|name| tokens.iter().any(|token| token == name))).map(|&(_, width, height, label)| (width, height, label.to_owned()))
}

/// The reply to a request for another size: the copy to make, or why not.
fn resize(size: Option<(u32, u32, String)>, scene: &Scene) -> Reply {
    let Some((width, height, label)) = size else {
        return Reply::Say("Which size? Say vertical (9:16), 4:5, square, landscape (16:9), 4K or 720p, or give the numbers, like 1280x720. I make a copy at that size and leave this composition as it is.".to_owned());
    };
    if (width, height) == (scene.composition.width, scene.composition.height) {
        return Reply::Say(format!("{} is already {width} × {height}.", scene.composition.name));
    }
    Reply::Do(String::new(), Action::Resize { width, height, label })
}

/// Words that, with "animation" or "motion", make a request rather than a question.
const ANIMATE_ASKS: &[&str] = &["add", "give", "put", "apply", "make", "do", "create", "want", "need", "auto", "automatic", "automatically"];

/// Whether `said` asks for the design to be animated: "animate this", "animate for me",
/// "add some motion", "make it move", "bring it to life". Questions about what is already
/// animated ("which layers are animated?") are not requests.
fn asks_to_animate(said: &[&str]) -> bool {
    let question = matches!(said.first(), Some(&("what" | "which" | "is" | "are" | "does" | "do" | "how" | "where" | "when" | "list" | "show" | "any")));
    if question && !said.contains(&"you") {
        return false;
    }
    let has = |words: &[&str]| said.iter().any(|word| words.contains(word));
    said.contains(&"animate")
        || (has(&["animation", "motion", "movement"]) && has(ANIMATE_ASKS))
        || (said.contains(&"move") && has(&["make", "get"]) && !has(&["layer", "playhead", "clip", "keyframe"]))
        || (said.contains(&"life") && has(&["bring", "give"]))
}

/// The reply to a request to animate: which layers (the selection when there is one and the
/// message doesn't ask for everything) and in what style.
fn animate_request(said: &[&str], scene: &Scene) -> Reply {
    use crate::auto_animate::Style;
    let has = |words: &[&str]| said.iter().any(|word| words.contains(word));
    // `None` when no feel is asked for, so the app's chosen default applies.
    let style = if has(&["bouncy", "bounce", "bouncing", "playful", "fun", "springy", "pop", "poppy", "cute"]) {
        Some(Style::Bouncy)
    } else if has(&["snappy", "snap", "fast", "quick", "quickly", "punchy", "energetic", "sharp", "dynamic"]) {
        Some(Style::Snappy)
    } else if has(&["smooth", "smoothly", "gentle", "gently", "calm", "subtle", "soft", "elegant"]) {
        Some(Style::Smooth)
    } else {
        None
    };
    // Words for the whole design. "Layer" isn't one: "animate this layer" means the selected
    // one, while "every layer" and "all layers" are caught by "every" and "all".
    let everything = has(&["all", "everything", "every", "whole", "entire", "design", "composition", "comp", "page", "scene"]);
    Reply::Do(String::new(), Action::Animate { selection: !scene.selected.is_empty() && !everything, style })
}

/// A message as the words that are compared: lower case, punctuation dropped, and plurals
/// reduced ("layers" matches "layer") except for very short words.
fn words(message: &str) -> Vec<String> {
    message
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let word = word.to_lowercase();
            match word.strip_suffix('s') {
                Some(single) if single.chars().count() >= 3 && !single.ends_with('s') => single.to_owned(),
                _ => word,
            }
        })
        .collect()
}

/// What a message is asking for, if it fits a pattern.
fn intent(message: &str) -> Option<Intent> {
    let said = words(message);
    let has = |word: &&str| said.iter().any(|said| said == word || said == word.strip_suffix('s').unwrap_or(word));
    let most_specific = |patterns: &'static [Pattern]| {
        let mut best: Option<&Pattern> = None;
        for pattern in patterns {
            if pattern.groups.iter().all(|group| group.iter().any(|word| has(word))) && best.is_none_or(|best| pattern.groups.len() > best.groups.len()) {
                best = Some(pattern);
            }
        }
        best.map(|pattern| pattern.intent)
    };
    most_specific(PATTERNS).or_else(|| most_specific(GENERAL)).or_else(|| most_specific(SMALL_TALK))
}

/// Whether a message is only small talk: a greeting, thanks, or a question about the assistant
/// itself. Said beside a request ("hello, how long is the video?") it asks for nothing of
/// its own.
pub fn is_small_talk(message: &str) -> bool {
    matches!(intent(message), Some(Intent::Hello | Intent::Goodbye | Intent::Thanks | Intent::HowAreYou | Intent::WhoAreYou | Intent::AreYouAi))
}

/// A count with its noun: "1 layer", "3 layers".
pub fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// A length of time as people say it: "12.5 seconds", "1 minute 4 seconds".
pub fn spoken(seconds: f64) -> String {
    if seconds < 60.0 {
        let text = format!("{seconds:.1}");
        let figure = text.strip_suffix(".0").unwrap_or(&text);
        return format!("{figure} {}", if figure == "1" { "second" } else { "seconds" });
    }
    let (minutes, rest) = ((seconds / 60.0).floor() as usize, (seconds % 60.0).round() as usize);
    format!("{} {}", plural(minutes, "minute", "minutes"), plural(rest, "second", "seconds"))
}

/// What kind of layer it is, in a word or two.
pub(crate) fn kind(layer: &Layer) -> &'static str {
    match layer.content {
        LayerContent::Footage { .. } => "video",
        LayerContent::Image { .. } => "image",
        LayerContent::Audio { .. } => "sound",
        LayerContent::Solid { .. } => "solid",
        LayerContent::Text { .. } => "text",
        LayerContent::Shape { .. } => "shape",
        LayerContent::Captions { .. } => "captions",
        LayerContent::Adjustment => "adjustment",
        LayerContent::Composition { .. } => "composition",
    }
}

/// Names joined as a sentence would list them, with at most `most` named and the rest counted.
pub fn listed(names: &[String], most: usize) -> String {
    let shown: Vec<&str> = names.iter().take(most).map(String::as_str).collect();
    match names.len().saturating_sub(most) {
        0 => shown.join(", "),
        more => format!("{}, and {more} more", shown.join(", ")),
    }
}

/// The most names a list in a reply spells out.
const MOST_NAMED: usize = 8;

fn sounds(layer: &Layer) -> bool {
    matches!(layer.content, LayerContent::Footage { .. } | LayerContent::Audio { .. }) && layer.has_audio
}

/// Answers from the project. Each method is one thing that can be asked.
impl Scene<'_> {
    fn fps(&self) -> f64 {
        self.composition.frame_rate.fps().max(1.0)
    }

    fn timecode(&self, frame: i64) -> String {
        format_timecode(frame, self.composition.frame_rate)
    }

    fn layers(&self) -> &[Layer] {
        &self.composition.layers
    }

    fn duration(&self) -> String {
        let frames = self.composition.duration_frames;
        let last = self.layers().iter().map(|layer| layer.out_frame).max().unwrap_or(0);
        let mut reply = format!("{} is {} long: {} at {} frames a second.", self.composition.name, spoken(frames as f64 / self.fps()), plural(frames.max(0) as usize, "frame", "frames"), self.composition.frame_rate.label());
        if last > 0 && last < frames {
            reply.push_str(&format!(" The last clip ends at {}, so the final {} are empty.", self.timecode(last), spoken((frames - last) as f64 / self.fps())));
        }
        reply
    }

    fn aspect(&self) -> String {
        let (width, height) = (self.composition.width.max(1), self.composition.height.max(1));
        let divisor = {
            let (mut a, mut b) = (width, height);
            while b != 0 {
                (a, b) = (b, a % b);
            }
            a
        };
        let shape = match width.cmp(&height) {
            std::cmp::Ordering::Greater => "landscape",
            std::cmp::Ordering::Less => "portrait (vertical)",
            std::cmp::Ordering::Equal => "square",
        };
        format!("{}:{}, which is {shape}. The picture is {width} × {height} pixels.", width / divisor, height / divisor)
    }

    fn background(&self) -> String {
        let color = &self.composition.background;
        let byte = |part: f32| (part.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("The background is #{:02X}{:02X}{:02X}.", byte(color.r), byte(color.g), byte(color.b))
    }

    fn settings(&self) -> String {
        let composition = self.composition;
        format!(
            "{}\n• Size: {} × {} pixels\n• Frame rate: {} frames a second\n• Length: {} ({})\n• Layers: {}\n• {}",
            composition.name,
            composition.width,
            composition.height,
            composition.frame_rate.label(),
            spoken(composition.duration_frames as f64 / self.fps()),
            plural(composition.duration_frames.max(0) as usize, "frame", "frames"),
            self.layers().len(),
            self.background()
        )
    }

    fn playhead(&self) -> String {
        format!("The playhead is at {} (frame {}), {} in.", self.timecode(self.current_frame), self.current_frame, spoken(self.current_frame as f64 / self.fps()))
    }

    fn time_left(&self) -> String {
        let left = (self.composition.duration_frames - self.current_frame).max(0);
        format!("{} left after the playhead ({}).", spoken(left as f64 / self.fps()), plural(left as usize, "frame", "frames"))
    }

    fn work_area(&self) -> String {
        let (start, end) = self.work_area;
        format!("The work area runs from {} to {}, which is {}.", self.timecode(start), self.timecode(end), spoken((end - start).max(0) as f64 / self.fps()))
    }

    /// How many clips are on each track, video tracks first.
    fn track_counts(&self) -> (BTreeMap<i32, usize>, BTreeMap<i32, usize>) {
        let (mut video, mut audio) = (BTreeMap::new(), BTreeMap::new());
        for layer in self.layers() {
            *(if layer.track >= 0 { &mut video } else { &mut audio }).entry(layer.track).or_insert(0) += 1;
        }
        (video, audio)
    }

    fn tracks(&self, which: Option<bool>) -> String {
        let (video, audio) = self.track_counts();
        let clips = |tracks: &BTreeMap<i32, usize>| plural(tracks.values().sum(), "clip", "clips");
        let video_line = format!("{} with {}", plural(video.len(), "video track", "video tracks"), clips(&video));
        let audio_line = format!("{} with {}", plural(audio.len(), "audio track", "audio tracks"), clips(&audio));
        match which {
            Some(true) => format!("There {} {video_line}.", if video.len() == 1 { "is" } else { "are" }),
            Some(false) => format!("There {} {audio_line}.", if audio.len() == 1 { "is" } else { "are" }),
            None => format!("{}: {video_line}, and {audio_line}.", plural(video.len() + audio.len(), "track", "tracks")),
        }
    }

    fn layer_count(&self) -> String {
        let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
        for layer in self.layers() {
            *kinds.entry(kind(layer)).or_insert(0) += 1;
        }
        if kinds.is_empty() {
            return format!("{} has no layers yet.", self.composition.name);
        }
        let breakdown: Vec<String> = kinds.iter().map(|(kind, count)| format!("{count} {kind}")).collect();
        format!("{} in {}: {}.", plural(self.layers().len(), "layer", "layers"), self.composition.name, breakdown.join(", "))
    }

    fn list_layers(&self) -> String {
        if self.layers().is_empty() {
            return format!("{} has no layers yet.", self.composition.name);
        }
        let lines: Vec<String> = self.layers().iter().take(MOST_NAMED * 2).map(|layer| format!("• {} ({}, {} to {})", layer.name, kind(layer), self.timecode(layer.in_frame), self.timecode(layer.out_frame))).collect();
        let more = self.layers().len().saturating_sub(lines.len());
        format!("From the top:\n{}{}", lines.join("\n"), if more > 0 { format!("\n…and {more} more.") } else { String::new() })
    }

    /// A sentence naming the layers that pass `keep`, or `none` when there are none.
    fn naming(&self, what: &str, none: &str, keep: impl Fn(&Layer) -> bool) -> String {
        let names: Vec<String> = self.layers().iter().filter(|layer| keep(layer)).map(|layer| layer.name.clone()).collect();
        if names.is_empty() { none.to_owned() } else { format!("{} {what}: {}.", names.len(), listed(&names, MOST_NAMED)) }
    }

    fn text_layers(&self) -> String {
        let lines: Vec<String> = self
            .layers()
            .iter()
            .filter_map(|layer| match &layer.content {
                LayerContent::Text { text, .. } => Some(format!("• \"{}\" at {}", text.lines().next().unwrap_or_default(), self.timecode(layer.in_frame))),
                _ => None,
            })
            .collect();
        if lines.is_empty() { "There are no text layers in this composition.".to_owned() } else { format!("{}:\n{}", plural(lines.len(), "text layer", "text layers"), lines.join("\n")) }
    }

    /// The clip `pick` chooses among those with any length, described with its times.
    fn one_clip(&self, label: &str, pick: impl Fn(&[&Layer]) -> Option<usize>) -> String {
        let clips: Vec<&Layer> = self.layers().iter().filter(|layer| layer.out_frame > layer.in_frame).collect();
        let Some(layer) = pick(&clips).and_then(|index| clips.get(index)) else { return format!("{} has no clips yet.", self.composition.name) };
        format!("The {label} is {}: {} to {}, {} long.", layer.name, self.timecode(layer.in_frame), self.timecode(layer.out_frame), spoken((layer.out_frame - layer.in_frame) as f64 / self.fps()))
    }

    fn selected(&self) -> String {
        let chosen: Vec<&Layer> = self.selected.iter().filter_map(|id| self.composition.layer(*id)).collect();
        let [layer] = chosen.as_slice() else {
            return match chosen.len() {
                0 => "Nothing is selected. Click a clip in the timeline or a layer in the viewer and ask again.".to_owned(),
                count => format!("{count} layers are selected: {}.", listed(&chosen.iter().map(|layer| layer.name.clone()).collect::<Vec<_>>(), MOST_NAMED)),
            };
        };
        let value = |property: TransformProperty| layer.property(PropertyPath::Transform(property), self.current_frame).map(|at| at.value);
        let mut lines = vec![
            format!("{} is a {} layer on {}.", layer.name, kind(layer), if layer.track >= 0 { format!("video track {}", layer.track + 1) } else { format!("audio track {}", -layer.track) }),
            format!("• Shows from {} to {} ({})", self.timecode(layer.in_frame), self.timecode(layer.out_frame), spoken((layer.out_frame - layer.in_frame).max(0) as f64 / self.fps())),
        ];
        if let Some(PropertyValue::Pair(position)) = value(TransformProperty::Position) {
            lines.push(format!("• Position: {:.0}, {:.0}", position.x, position.y));
        }
        if let Some(PropertyValue::Pair(scale)) = value(TransformProperty::Scale) {
            lines.push(format!("• Scale: {:.0}% × {:.0}%", scale.x, scale.y));
        }
        if let Some(PropertyValue::Scalar(rotation)) = value(TransformProperty::Rotation) {
            lines.push(format!("• Rotation: {rotation:.0}°"));
        }
        if let Some(PropertyValue::Scalar(opacity)) = value(TransformProperty::Opacity) {
            lines.push(format!("• Opacity: {opacity:.0}%"));
        }
        if sounds(layer) {
            lines.push(format!("• Volume: {:+.1} dB{}", layer.audio.volume_db.value_at(self.current_frame as f64), if layer.switches.audio { "" } else { " (muted)" }));
        }
        let keyframes: usize = layer.property_paths().into_iter().map(|path| layer.keyframe_easings(path).len()).sum();
        lines.push(format!("• {}, {}", plural(keyframes, "keyframe", "keyframes"), plural(layer.effects.len(), "effect", "effects")));
        lines.join("\n")
    }

    fn effects(&self) -> String {
        let lines: Vec<String> = self.layers().iter().filter(|layer| !layer.effects.is_empty()).map(|layer| format!("• {}: {}", layer.name, listed(&layer.effects.iter().map(|effect| effect.name.clone()).collect::<Vec<_>>(), 4))).collect();
        let total: usize = self.layers().iter().map(|layer| layer.effects.len()).sum();
        if total == 0 { "No layer in this composition has an effect.".to_owned() } else { format!("{} on {}:\n{}", plural(total, "effect", "effects"), plural(lines.len(), "layer", "layers"), lines.join("\n")) }
    }

    /// How many keyframes each layer has, for the layers with any.
    fn keyframe_counts(&self) -> Vec<(&Layer, usize)> {
        self.layers().iter().map(|layer| (layer, layer.property_paths().into_iter().map(|path| layer.keyframe_easings(path).len()).sum::<usize>())).filter(|(_, count)| *count > 0).collect()
    }

    fn keyframes(&self) -> String {
        let counts = self.keyframe_counts();
        let total: usize = counts.iter().map(|(_, count)| count).sum();
        if total == 0 { "There are no keyframes in this composition yet.".to_owned() } else { format!("{} across {}.", plural(total, "keyframe", "keyframes"), plural(counts.len(), "layer", "layers")) }
    }

    fn animated(&self) -> String {
        let lines: Vec<String> = self.keyframe_counts().into_iter().take(MOST_NAMED * 2).map(|(layer, count)| format!("• {} ({})", layer.name, plural(count, "keyframe", "keyframes"))).collect();
        if lines.is_empty() { "Nothing is animated yet. Click a stopwatch in Properties to start.".to_owned() } else { format!("{} animated:\n{}", plural(lines.len(), "layer is", "layers are"), lines.join("\n")) }
    }

    fn sound_layers(&self) -> String {
        let lines: Vec<String> = self.layers().iter().filter(|layer| sounds(layer)).map(|layer| format!("• {}: {:+.1} dB{}{}", layer.name, layer.audio.volume_db.value_at(self.current_frame as f64), if layer.audio.volume_db.is_animated() { " (keyframed)" } else { "" }, if layer.switches.audio { "" } else { ", muted" })).collect();
        if lines.is_empty() { "Nothing in this composition has sound.".to_owned() } else { format!("{} sound:\n{}", plural(lines.len(), "layer has", "layers have"), lines.join("\n")) }
    }

    fn markers(&self) -> String {
        let markers = &self.composition.markers;
        if markers.is_empty() {
            return "There are no markers on this composition.".to_owned();
        }
        let at: Vec<String> = markers.iter().map(|marker| if marker.label.is_empty() { self.timecode(marker.frame) } else { format!("{} ({})", self.timecode(marker.frame), marker.label) }).collect();
        format!("{}: {}.", plural(markers.len(), "marker", "markers"), listed(&at, MOST_NAMED))
    }

    fn captions(&self) -> String {
        let counts: Vec<usize> = self.layers().iter().filter_map(|layer| if let LayerContent::Captions { words, .. } = &layer.content { Some(words.len()) } else { None }).collect();
        if counts.is_empty() { "This composition has no captions.".to_owned() } else { format!("{} with {} in all.", plural(counts.len(), "caption layer", "caption layers"), plural(counts.iter().sum(), "word", "words")) }
    }

    fn fonts(&self) -> String {
        let mut fonts: Vec<String> = self
            .layers()
            .iter()
            .filter_map(|layer| match &layer.content {
                LayerContent::Text { style, .. } => Some(style.font_family.clone()),
                LayerContent::Captions { style, .. } => Some(style.text.font_family.clone()),
                _ => None,
            })
            .collect();
        fonts.sort();
        fonts.dedup();
        if fonts.is_empty() { "No text here, so no fonts are in use.".to_owned() } else { format!("{} in use: {}.", plural(fonts.len(), "font", "fonts"), listed(&fonts, MOST_NAMED)) }
    }

    /// The ids of the files and compositions any layer in the project plays.
    fn used_items(&self) -> Vec<Id> {
        self.project
            .compositions
            .iter()
            .flat_map(|composition| &composition.layers)
            .filter_map(|layer| match layer.content {
                LayerContent::Footage { item } | LayerContent::Image { item } | LayerContent::Audio { item } => Some(item),
                _ => None,
            })
            .collect()
    }

    fn is_file(kind: ProjectItemKind) -> bool {
        matches!(kind, ProjectItemKind::Footage | ProjectItemKind::Image | ProjectItemKind::Audio)
    }

    fn media(&self) -> String {
        let count = |kind: ProjectItemKind| self.project.items.iter().filter(|item| item.kind == kind).count();
        format!(
            "The project has {}, {} and {}, in {}.",
            plural(count(ProjectItemKind::Footage), "video file", "video files"),
            plural(count(ProjectItemKind::Image), "image", "images"),
            plural(count(ProjectItemKind::Audio), "sound file", "sound files"),
            plural(self.project.compositions.len(), "composition", "compositions")
        )
    }

    fn unused_media(&self) -> String {
        let used = self.used_items();
        let unused: Vec<String> = self.project.items.iter().filter(|item| Self::is_file(item.kind) && !used.contains(&item.id)).map(|item| item.name.clone()).collect();
        if unused.is_empty() { "Every imported file is used in a composition.".to_owned() } else { format!("{} not used anywhere: {}.", plural(unused.len(), "file is", "files are"), listed(&unused, MOST_NAMED)) }
    }

    fn compositions(&self) -> String {
        let names: Vec<String> = self.project.compositions.iter().map(|composition| if composition.id == self.composition.id { format!("{} (open now)", composition.name) } else { composition.name.clone() }).collect();
        format!("{}: {}.", plural(names.len(), "composition", "compositions"), listed(&names, MOST_NAMED))
    }

    /// What the rules have noticed, in all areas or one.
    fn problems(&self, area: Option<Area>) -> Reply {
        let found: Vec<&Suggestion> = self.suggestions.iter().filter(|suggestion| area.is_none_or(|area| suggestion.check.area() == area)).collect();
        let about = area.map_or_else(String::new, |area| format!(" about {}", area.name().to_lowercase()));
        if found.is_empty() {
            return Reply::Say(format!("Nothing{about} stands out to me. It looks good."));
        }
        let lines: Vec<String> = found.iter().take(5).map(|suggestion| format!("• {}", suggestion.title)).collect();
        let more = found.len().saturating_sub(lines.len());
        let safe = found.iter().filter(|suggestion| suggestion.fix.as_ref().is_some_and(|fix| fix.is_safe())).count();
        let mut reply = format!("I noticed {}{about}:\n{}", plural(found.len(), "thing", "things"), lines.join("\n"));
        if more > 0 {
            reply.push_str(&format!("\n…and {more} more."));
        }
        reply.push_str("\nThe Suggestions tab has each one with its fix.");
        if safe > 1 {
            reply.push_str(&format!(" {safe} are safe to fix together: say \"fix everything\" or type /fix."));
        }
        Reply::Do(reply, Action::ShowSuggestions)
    }

    fn help(&self) -> String {
        let commands: Vec<String> = COMMANDS.iter().map(|(command, about)| format!("{command}: {about}")).collect();
        format!(
            "Ask me in your own words about this composition, for example:\n• How long is the video?\n• What are the comp settings?\n• How many layers are there?\n• How many video and audio tracks?\n• What's selected?\n• Is anything wrong with the audio?\n• Make a vertical version\n• How do I split a clip?\n\nOr tell me what to do with a layer:\n• Start at 5s, last 2 seconds and fade in\n• Slide the title in from the left over a second\n• Fade out at 8s\n• Cut it at the playhead\n• Make it pulse, or 50% opacity\n• Hide the logo, duplicate it, rename it to Hero, bring it to the front\n• Put it in the top right corner, make it red, a bit bigger, select all text layers\n\nOr with the timeline and the composition:\n• Go to 5s, next marker, play, pause\n• Add a marker called Chorus\n• Stagger them by 5 frames, sequence them, close the gaps, reverse their order\n• Make the composition 10 seconds long, set the frame rate to 30\n• Set the work area from 2s to 6s, trim the composition to the work area\n• Mute it, set the volume to 50%, fade the music out over 2s, duck the music under the voice\n• Twice as fast, slow it down, reverse it, freeze frame here\n• Make it longer, move it later, make the fade slower\n• Export as mp4, export the work area as a gif, export all compositions, save this frame as a PNG\n• Export for Instagram Reel, save these export settings as Print, list export presets\n• Send this frame to Edit, or to Develop\n\nIn the chat field, Tab finishes a command or recipe and the up arrow brings back what you typed. Settings ▸ Recipes changes recipes, their shortcuts, buttons and moments.\n• Add a text layer saying “Hello”, add a red solid, add a circle\n• Am I ready to export?\n\n{}\n\nCommands:\n{}",
            iris::front::HELP,
            commands.join("\n")
        )
    }
}

/// The words the questions and requests here are matched by, for putting a mistyped message
/// right (see `iris::spelling`).
pub fn vocabulary() -> Vec<String> {
    let patterns = PATTERNS.iter().chain(GENERAL).chain(SMALL_TALK).flat_map(|pattern| pattern.groups.iter().flat_map(|group| group.iter().copied()));
    let sizes = NAMED_SIZES.iter().flat_map(|(names, ..)| names.iter().copied());
    patterns.chain(RESIZE_WORDS.iter().copied()).chain(sizes).filter(|word| word.chars().all(char::is_alphabetic)).map(str::to_owned).collect()
}

/// Whether `message` says every word of the name of command `index`, so nothing but that
/// command can be meant.
fn names_whole_command(message: &str, scene: &Scene, index: usize) -> bool {
    let said = words(message);
    scene.commands.get(index).is_some_and(|command| words(command.name).iter().all(|word| said.contains(word)))
}

/// A request that is clear about wanting a change but not about which: the question to ask
/// back, and the likely meanings to offer as replies.
fn unclear(said: &[&str]) -> Option<(String, Vec<String>)> {
    let has = |words: &[&str]| said.iter().any(|word| words.contains(word));
    let meanings = |replies: &[&str]| replies.iter().map(|reply| (*reply).to_owned()).collect();
    // A style for animating the whole design ("animate it, subtle") is read elsewhere.
    if has(&["animate", "animation"]) {
        return None;
    }
    if has(&["subtle", "subtler", "calmer", "gentler", "understated"]) {
        return Some(("More subtle in which way? I can do any of these to the layer:".to_owned(), meanings(&["Make the fade slower", "Fade it in", "Make it a bit smaller", "70% opacity"])));
    }
    if has(&["dramatic", "bolder", "punchier", "exciting", "striking"]) {
        return Some(("More dramatic in which way? I can do any of these to the layer:".to_owned(), meanings(&["Make it pop in", "Make it bounce in", "Make it bigger", "Make it pulse"])));
    }
    // "Export as PNG" could be the frame under the playhead or every frame.
    if has(&["export", "save", "render"]) && has(&["png"]) {
        return Some(("One picture or all of them?".to_owned(), meanings(&["Save this frame as a PNG", "Export as a PNG sequence"])));
    }
    None
}

/// A message read as housekeeping for layers, if it is. Words that only may name a layer
/// ("hide the rulers") count when a layer is called that and nothing else claims the message.
fn housekeeping(message: &str, scene: &Scene) -> Option<Action> {
    let request = iris::layers::parse(message)?;
    if request.sure {
        return Some(Action::Housekeep(request));
    }
    let Target::Named(name) = &request.target else { return None };
    let names_command = matches!(iris::commands::find(message, scene.commands), Found::One(index) if names_whole_command(message, scene, index));
    let is_about_layers = crate::instructions::has_layer_called(scene.composition, name) && crate::instructions::read(message).is_none() && !names_command;
    is_about_layers.then_some(Action::Housekeep(request))
}

/// The same change made half as far, for "too much": relative changes only (bigger and
/// smaller, nudges, shifts in time, turns, louder and quieter, and the steps taken when no
/// amount was said). `None` when nothing in `action` can be done by halves.
pub fn softened(action: &Action) -> Option<Action> {
    use iris::intent::{Step, Time};
    let halved = |time: Time| match time {
        Time::Span { millis, frames } => Some(Time::Span { millis: millis / 2, frames: frames / 2 }).filter(|half| *half != Time::Span { millis: 0, frames: 0 }),
        _ => None,
    };
    let mut softer = false;
    let softened = match action {
        Action::Housekeep(request) => {
            let steps = request.steps.iter().cloned().map(|step| match step {
                iris::layers::Step::Resize(percent) if (percent - 100).abs() >= 2 => {
                    softer = true;
                    iris::layers::Step::Resize(100 + (percent - 100) / 2)
                }
                other => other,
            });
            Action::Housekeep(iris::layers::Request { steps: steps.collect(), ..request.clone() })
        }
        Action::Instruct(request) => {
            let steps = request.steps.iter().map(|step| match *step {
                Step::Nudge { x, y } if x / 2 != 0 || y / 2 != 0 => {
                    softer = true;
                    Step::Nudge { x: x / 2, y: y / 2 }
                }
                Step::Rotate(degrees) if degrees / 2 != 0 => {
                    softer = true;
                    Step::Rotate(degrees / 2)
                }
                Step::Shift { by, later } => match halved(by) {
                    Some(half) => {
                        softer = true;
                        Step::Shift { by: half, later }
                    }
                    None => *step,
                },
                other => other,
            });
            Action::Instruct(iris::intent::Request { target: request.target.clone(), steps: steps.collect() })
        }
        Action::Chore(request) => {
            use crate::chores::{Chore, Level, Pace};
            let chore = match &request.chore {
                Chore::Volume(Level::By(tenths)) if tenths / 2 != 0 => Chore::Volume(Level::By(tenths / 2)),
                Chore::DurationBy { by, longer } => Chore::DurationBy { by: halved(*by)?, longer: *longer },
                Chore::LengthBy { by, longer } => Chore::LengthBy { by: halved(*by)?, longer: *longer },
                Chore::ShiftBy { by, later } => Chore::ShiftBy { by: halved(*by)?, later: *later },
                Chore::MotionPace { percent, ends } if (percent - 100) / 2 != 0 => Chore::MotionPace { percent: 100 + (percent - 100) / 2, ends: *ends },
                // Halfway between the speed asked for and the speed it had.
                Chore::Speed(Pace::Times { times, over }) if times != over => Chore::Speed(Pace::Times { times: times + over, over: over * 2 }),
                _ => return None,
            };
            softer = true;
            Action::Chore(crate::chores::Request { chore, target: request.target.clone() })
        }
        _ => return None,
    };
    softer.then_some(softened)
}

/// The same request about the layer called `name`, for "same for the logo". `None` when
/// `action` isn't about layers.
pub fn retargeted(action: &Action, name: &str) -> Option<Action> {
    use crate::chores::Chore;
    let target = Target::Named(name.to_owned());
    Some(match action {
        Action::Instruct(request) => Action::Instruct(iris::intent::Request { target, steps: request.steps.clone() }),
        Action::Housekeep(request) => Action::Housekeep(iris::layers::Request { target, ..request.clone() }),
        Action::Chore(request) if matches!(request.chore, Chore::Mute(_) | Chore::Volume(_) | Chore::SoundFade { .. } | Chore::Speed(_) | Chore::Reverse(_) | Chore::Freeze | Chore::LengthBy { .. } | Chore::ShiftBy { .. } | Chore::MotionPace { .. }) => {
            Action::Chore(crate::chores::Request { chore: request.chore.clone(), target })
        }
        _ => return None,
    })
}

/// Answers a command (a message starting with `/`).
fn command(message: &str, scene: &Scene) -> Reply {
    let (name, rest) = message.split_once(char::is_whitespace).map_or((message, ""), |(name, rest)| (name, rest.trim()));
    match name.to_lowercase().as_str() {
        "/help" | "/?" | "/commands" => Reply::Say(scene.help()),
        "/clear" | "/reset" | "/new" => Reply::Do(String::new(), Action::Clear),
        "/check" | "/suggestions" | "/review" => scene.problems(None),
        "/fix" => Reply::Do("Fixing everything that is safe to fix.".to_owned(), Action::FixAllSafe),
        "/comp" | "/composition" | "/info" => Reply::Say(scene.settings()),
        "/layers" => Reply::Say(scene.list_layers()),
        "/tracks" => Reply::Say(scene.tracks(None)),
        "/media" | "/files" => Reply::Say(scene.media()),
        "/selected" | "/selection" => Reply::Say(scene.selected()),
        "/resize" | "/size" | "/convert" => resize(wanted_size(rest), scene),
        "/animate" | "/anim" | "/motion" => animate_request(&words(rest).iter().map(String::as_str).collect::<Vec<_>>(), scene),
        "/ready" | "/preflight" => Reply::Do(String::new(), Action::Chore(crate::chores::Request { chore: crate::chores::Chore::Preflight, target: Target::Unnamed })),
        // Recipes that exist are run, listed and forgotten before a message gets here.
        "/teach" => Reply::Say("To teach me a recipe, give it a name and its steps: /teach intro = fade in; pulse.".to_owned()),
        "/forget" => Reply::Say(format!("I don't know a recipe called “{rest}”. Type /recipes for the ones I do.")),
        "/tip" | "/tips" => Reply::Do(String::new(), Action::Tip),
        "/settings" => Reply::Do("Opening Settings.".to_owned(), Action::OpenSettings),
        "/name" if rest.is_empty() => Reply::Say(format!("I'm {}. To call me something else, type /name and the new name.", scene.name)),
        "/name" => Reply::Do(String::new(), Action::Rename(rest.to_owned())),
        _ => Reply::Say(format!("I don't know the command {name}. Type /help for the ones I do.")),
    }
}

/// The assistant's reply to a message.
pub fn reply(message: &str, scene: &Scene) -> Reply {
    let message = message.trim();
    if message.starts_with('/') {
        return command(message, scene);
    }
    // "How do I ..." asks how to do something, not about the project: those go to the help answers.
    let said = words(message);
    let said: Vec<&str> = said.iter().map(String::as_str).collect();
    if matches!(said.as_slice(), ["how", "do", "i" | "you" | "we", ..] | ["how", "to", ..] | ["how", "can", "i" | "we", ..] | ["how", "would", "i", ..] | ["how", "should", "i", ..]) {
        return Reply::Unknown;
    }
    // Motion's own chores: the playhead, the composition, arranging, sound, markers and so on.
    if let Some(request) = crate::chores::read(message) {
        return Reply::Do(String::new(), Action::Chore(request));
    }
    // Words that could mean several things get a question, not a guess.
    if let Some((question, meanings)) = unclear(&said) {
        return Reply::Do(question, Action::Ask(meanings));
    }
    // Housekeeping for layers.
    if let Some(action) = housekeeping(message, scene) {
        return Reply::Do(String::new(), action);
    }
    // A command named in full is that command, whatever else its words could mean.
    let command = iris::commands::find(message, scene.commands);
    if let Found::One(index) = command
        && names_whole_command(message, scene, index)
    {
        return Reply::Do(String::new(), Action::Run(index));
    }
    // Instructions for layers: timing, ways in and out, loops and looks.
    if let Some(request) = crate::instructions::read(message) {
        return Reply::Do(String::new(), Action::Instruct(request));
    }
    // Asking for the design to be animated.
    if asks_to_animate(&said) {
        return animate_request(&said, scene);
    }
    // Asking for another size: a word such as "convert" or "make", with a size it can find.
    let asks_to_change = said.iter().any(|word| RESIZE_WORDS.contains(word));
    let size = wanted_size(message);
    if asks_to_change && (size.is_some() || said.iter().any(|word| matches!(*word, "resize" | "resized" | "resolution" | "reformat"))) {
        return resize(size, scene);
    }
    let Some(intent) = intent(message) else {
        // Nothing else fits, so a command named in part is meant ("export", "guides").
        return match command {
            Found::One(index) => Reply::Do(String::new(), Action::Run(index)),
            Found::Several(found) => Reply::Do(String::new(), Action::Offer(found)),
            Found::None => Reply::Unknown,
        };
    };
    let say = Reply::Say;
    match intent {
        Intent::Hello => say(format!("Hello! I'm {}. Ask me about this composition or how to do something in Omina Motion, or type /help.", scene.name)),
        Intent::Goodbye => say("Goodbye! I'll keep an eye on the timeline. Type /clear if you'd like a fresh conversation next time.".to_owned()),
        Intent::Thanks => say("You're welcome.".to_owned()),
        Intent::HowAreYou => say("Wide awake and watching your keyframes. What are you working on?".to_owned()),
        Intent::WhoAreYou => say(format!("I'm {}, your assistant in Omina Motion, Omina Edit and Omina Develop. Here I read the open composition and answer questions about it, point out things worth fixing, animate designs, and carry out instructions like “start at 5s, last 2 seconds and fade in”.", scene.name)),
        Intent::AreYouAi => say("No. I'm a set of fixed rules and ready-written answers, so I only know what Omina Motion records about your project, and I'll say so when I don't know something.".to_owned()),
        Intent::WhatCanYouDo => say(scene.help()),
        Intent::Duration => say(scene.duration()),
        Intent::FrameRate => say(format!("{} plays at {} frames a second.", scene.composition.name, scene.composition.frame_rate.label())),
        Intent::Size => say(format!("{} is {} × {} pixels.", scene.composition.name, scene.composition.width, scene.composition.height)),
        Intent::AspectRatio => say(scene.aspect()),
        Intent::Background => say(scene.background()),
        Intent::FrameCount => say(format!("{} is {} long.", scene.composition.name, plural(scene.composition.duration_frames.max(0) as usize, "frame", "frames"))),
        Intent::CompSettings => say(scene.settings()),
        Intent::CompName => say(format!("The open composition is called {}.", scene.composition.name)),
        Intent::Playhead => say(scene.playhead()),
        Intent::TimeLeft => say(scene.time_left()),
        Intent::WorkArea => say(scene.work_area()),
        Intent::VideoTracks => say(scene.tracks(Some(true))),
        Intent::AudioTracks => say(scene.tracks(Some(false))),
        Intent::Tracks => say(scene.tracks(None)),
        Intent::TextLayers => say(scene.text_layers()),
        Intent::HiddenLayers => say(scene.naming("hidden", "No layers are hidden.", |layer| !layer.switches.visible)),
        Intent::LockedLayers => say(scene.naming("locked", "No layers are locked.", |layer| layer.switches.locked)),
        Intent::LongestClip => say(scene.one_clip("longest clip", |clips| (0..clips.len()).max_by_key(|&index| clips[index].out_frame - clips[index].in_frame))),
        Intent::ShortestClip => say(scene.one_clip("shortest clip", |clips| (0..clips.len()).min_by_key(|&index| clips[index].out_frame - clips[index].in_frame))),
        Intent::FirstClip => say(scene.one_clip("first clip", |clips| (0..clips.len()).min_by_key(|&index| clips[index].in_frame))),
        Intent::LastClip => say(scene.one_clip("last clip to end", |clips| (0..clips.len()).max_by_key(|&index| clips[index].out_frame))),
        Intent::Selected => say(scene.selected()),
        Intent::Effects => say(scene.effects()),
        Intent::Keyframes => say(scene.keyframes()),
        Intent::Animated => say(scene.animated()),
        Intent::SoundLayers => say(scene.sound_layers()),
        Intent::Muted => say(scene.naming("muted", "Nothing with sound is muted.", |layer| sounds(layer) && !layer.switches.audio)),
        Intent::Markers => say(scene.markers()),
        Intent::Captions => say(scene.captions()),
        Intent::Fonts => say(scene.fonts()),
        Intent::UnusedMedia => say(scene.unused_media()),
        Intent::Compositions => say(scene.compositions()),
        Intent::Media => say(scene.media()),
        Intent::ListLayers => say(scene.list_layers()),
        Intent::LayerKinds | Intent::LayerCount => say(scene.layer_count()),
        Intent::AreaProblems(area) => scene.problems(Some(area)),
        Intent::Problems => scene.problems(None),
        Intent::FixAll => Reply::Do("Fixing everything that is safe to fix.".to_owned(), Action::FixAllSafe),
        Intent::Tip => Reply::Do(String::new(), Action::Tip),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    /// Replies to `message` about the sample project's main composition, with `selected` chosen.
    fn ask_with(message: &str, selected: &[Id]) -> Reply {
        let project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let found = crate::assistant::suggestions(composition, &crate::assistant::Options::default());
        reply(message, &Scene { project: &project, composition, current_frame: 48, selected, work_area: (0, composition.duration_frames), name: "Iris", suggestions: &found, commands: COMMANDS_FOR_TESTS })
    }

    /// A few commands as the app would hand them over, by their menu names.
    const COMMANDS_FOR_TESTS: &[iris::commands::Command] = &[
        iris::commands::Command { name: "Export", keywords: "render deliver" },
        iris::commands::Command { name: "Open the export queue", keywords: "" },
        iris::commands::Command { name: "Settings", keywords: "preferences" },
        iris::commands::Command { name: "Composition settings", keywords: "" },
        iris::commands::Command { name: "Show or hide guides", keywords: "" },
        iris::commands::Command { name: "Split at the playhead", keywords: "" },
        iris::commands::Command { name: "Zoom the viewer in", keywords: "" },
        iris::commands::Command { name: "Zoom the viewer out", keywords: "" },
        iris::commands::Command { name: "Add or remove a marker", keywords: "" },
        iris::commands::Command { name: "Duplicate", keywords: "" },
    ];

    fn ask(message: &str) -> String {
        match ask_with(message, &[]) {
            Reply::Say(text) | Reply::Do(text, _) => text,
            Reply::Unknown => panic!("no answer to {message:?}"),
        }
    }

    #[test]
    fn the_same_question_is_understood_in_many_wordings() {
        // Every wording is tried before any is reported, so one run shows all that are misread.
        let misread = std::cell::RefCell::new(Vec::new());
        let means = |message: &str, expected: Intent| {
            if intent(message) != Some(expected) {
                misread.borrow_mut().push(format!("{message:?} was read as {:?}, not {expected:?}", intent(message)));
            }
        };
        for message in ["How long is the video?", "how long is this comp", "what's the duration", "What is the runtime of the project?", "length of the timeline?", "when does it end"] {
            means(message, Intent::Duration);
        }
        for message in ["what are the comp settings", "composition settings?", "give me an overview of the project", "specs"] {
            means(message, Intent::CompSettings);
        }
        for message in ["how many layers are there", "number of clips?", "layer count", "How many layers do I have?"] {
            means(message, Intent::LayerCount);
        }
        for message in ["how many tracks for video", "video tracks?", "how many video tracks are there"] {
            means(message, Intent::VideoTracks);
        }
        for message in ["how many audio tracks", "count the sound tracks"] {
            means(message, Intent::AudioTracks);
        }
        means("how many tracks", Intent::Tracks);
        means("what's the frame rate?", Intent::FrameRate);
        means("fps?", Intent::FrameRate);
        means("what resolution is this", Intent::Size);
        means("is it vertical", Intent::AspectRatio);
        means("how many frames is it", Intent::FrameCount);
        means("where is the playhead", Intent::Playhead);
        means("what frame am I on", Intent::Playhead);
        means("how much time is left", Intent::TimeLeft);
        means("what is selected", Intent::Selected);
        means("tell me about this layer", Intent::Selected);
        means("which layers are animated?", Intent::Animated);
        means("how many keyframes", Intent::Keyframes);
        means("what effects are on my layers", Intent::Effects);
        means("which layers have sound", Intent::SoundLayers);
        means("is anything muted", Intent::Muted);
        means("what fonts am I using", Intent::Fonts);
        means("any unused files?", Intent::UnusedMedia);
        means("how many compositions", Intent::Compositions);
        means("how many files did I import", Intent::Media);
        means("list the layers", Intent::ListLayers);
        means("what's the longest clip", Intent::LongestClip);
        means("is anything wrong?", Intent::Problems);
        means("any problems with the audio", Intent::AreaProblems(Area::Audio));
        means("check my animation", Intent::AreaProblems(Area::Motion));
        means("fix everything", Intent::FixAll);
        means("give me a tip", Intent::Tip);
        means("hey, how long is the video?", Intent::Duration);
        means("how are you doing", Intent::HowAreYou);
        means("help", Intent::WhatCanYouDo);
        assert!(misread.borrow().is_empty(), "{:#?}", misread.borrow());
    }

    #[test]
    fn small_talk_is_answered() {
        assert!(ask("Hello").contains("Iris"));
        assert!(ask("hey there!").starts_with("Hello"));
        assert!(ask("goodbye").starts_with("Goodbye"));
        assert!(ask("see you later").starts_with("Goodbye"));
        assert_eq!(ask("thanks!"), "You're welcome.");
        assert!(ask("are you an AI?").starts_with("No."));
        assert!(ask("who are you").contains("your assistant"));
        assert!(ask("what can you do?").contains("/clear"));
    }

    #[test]
    fn answers_come_from_the_project() {
        let project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        let length = ask("how long is the video");
        assert!(length.contains(&composition.name) && length.contains(&format!("{} frames", composition.duration_frames)), "{length}");
        let settings = ask("what are the comp settings");
        assert!(settings.contains(&format!("{} × {} pixels", composition.width, composition.height)) && settings.contains("Frame rate"), "{settings}");
        assert!(ask("how many layers are there").starts_with(&format!("{} layers", composition.layers.len())));
        let video = composition.layers.iter().filter(|layer| layer.track >= 0).map(|layer| layer.track).collect::<std::collections::BTreeSet<_>>().len();
        assert!(ask("how many video tracks").contains(&format!("{video} video track")), "{}", ask("how many video tracks"));
        assert!(ask("where is the playhead").contains("frame 48"));
        assert!(ask("what is selected").starts_with("Nothing is selected"));
    }

    #[test]
    fn the_selected_layer_is_described() {
        let project = sample_project();
        let layer = &project.composition(ids::MAIN_COMP).unwrap().layers[0];
        let Reply::Say(text) = ask_with("what's selected?", &[layer.id]) else { panic!("no answer") };
        assert!(text.starts_with(&layer.name) && text.contains("Shows from") && text.contains("keyframe"), "{text}");
    }

    #[test]
    fn commands_do_what_they_say() {
        assert_eq!(ask_with("/clear", &[]), Reply::Do(String::new(), Action::Clear));
        assert_eq!(ask_with("/name  Pixel ", &[]), Reply::Do(String::new(), Action::Rename("Pixel".to_owned())));
        assert!(matches!(ask_with("/fix", &[]), Reply::Do(_, Action::FixAllSafe)));
        assert!(matches!(ask_with("/settings", &[]), Reply::Do(_, Action::OpenSettings)));
        let help = ask("/help");
        assert!(COMMANDS.iter().all(|(command, _)| help.contains(command)), "every command is listed");
        assert!(ask("/tracks").contains("video track"));
        assert!(ask("/wibble").contains("/help"));
    }

    #[test]
    fn animating_can_be_asked_for_in_many_wordings() {
        use crate::auto_animate::Style;
        let wants = |message: &str, selected: &[Id]| match ask_with(message, selected) {
            Reply::Do(_, Action::Animate { selection, style }) => Some((selection, style)),
            _ => None,
        };
        let title = [Id(100)];
        for message in ["animate", "Animate this", "animate for me", "animate it please", "can you animate this?", "add some animation", "auto animate", "make it move", "bring it to life", "/animate"] {
            assert_eq!(wants(message, &[]), Some((false, None)), "{message}");
        }
        // With a selection, "this" means it; "everything" means the whole design.
        assert_eq!(wants("animate this", &title), Some((true, None)));
        assert_eq!(wants("animate everything", &title), Some((false, None)));
        assert_eq!(wants("animate the whole design", &title), Some((false, None)));
        // Naming one layer means the selected one, not every layer.
        for message in ["animate the current layer", "animate this layer", "animate the selected layer", "animate my layer"] {
            assert_eq!(wants(message, &title), Some((true, None)), "{message}");
        }
        assert_eq!(wants("animate every layer", &title), Some((false, None)));
        assert_eq!(wants("animate all layers", &title), Some((false, None)));
        // Style words.
        assert_eq!(wants("make it bouncy and animate it", &[]), Some((false, Some(Style::Bouncy))));
        assert_eq!(wants("animate this, fast and punchy", &[]), Some((false, Some(Style::Snappy))));
        assert_eq!(wants("/animate bouncy", &[]), Some((false, Some(Style::Bouncy))));
        assert_eq!(wants("animate it smoothly", &[]), Some((false, Some(Style::Smooth))), "smooth can be asked for too");
        // Questions about what moves already, and how-tos, aren't requests.
        for message in ["which layers are animated?", "is anything animated", "how do I animate text?", "what animations are there", "move the playhead to the end"] {
            assert_eq!(wants(message, &[]), None, "{message}");
        }
    }

    #[test]
    fn another_size_can_be_asked_for_by_name_or_in_numbers() {
        let wants = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, Action::Resize { width, height, label }) => Some((width, height, label)),
            _ => None,
        };
        assert_eq!(wants("convert this comp to vertical"), Some((1080, 1920, "9:16".to_owned())));
        assert_eq!(wants("make a square version"), Some((1080, 1080, "1:1".to_owned())));
        assert_eq!(wants("I need this in 4K"), Some((3840, 2160, "4K".to_owned())));
        assert_eq!(wants("resize to 1280x720"), Some((1280, 720, "1280x720".to_owned())));
        assert_eq!(wants("change the resolution to 1000 by 1500 please"), Some((1000, 1500, "1000x1500".to_owned())));
        assert_eq!(wants("make it 4:5 for Instagram"), Some((1080, 1350, "4:5".to_owned())));
        assert_eq!(wants("/resize 720p"), Some((1280, 720, "720p".to_owned())));
        // Asked without a size, it says which sizes it knows; a size too large is not made.
        assert!(ask("can you resize this comp?").starts_with("Which size?"));
        assert!(ask("/resize").starts_with("Which size?"));
        assert_eq!(wants("resize to 99999x10"), None);
        // Asking what the size is still answers the question, and changes nothing.
        assert!(ask("what resolution is this").contains("pixels"));
        assert_eq!(intent("is it vertical"), Some(Intent::AspectRatio));
        // The size it already is isn't made again.
        let project = sample_project();
        let composition = project.composition(ids::MAIN_COMP).unwrap();
        assert!(ask(&format!("resize to {}x{}", composition.width, composition.height)).contains("already"));
    }

    #[test]
    fn housekeeping_is_read_and_checked_against_the_layers_there() {
        use iris::layers::Step;
        let wants = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, Action::Housekeep(request)) => Some((request.target, request.steps)),
            _ => None,
        };
        assert_eq!(wants("hide it"), Some((Target::Selection, vec![Step::Hide])));
        assert_eq!(wants("select all text layers").map(|(target, ..)| target), Some(Target::Kind(iris::intent::Kind::Text)));
        assert_eq!(wants("bring the title layer to the front").map(|(target, ..)| target), Some(Target::Named("title".to_owned())));
        // Words that only may name a layer count when a layer is called that.
        assert_eq!(wants("hide the title").map(|(target, ..)| target), Some(Target::Named("title".to_owned())));
        assert_eq!(wants("hide the rulers"), None);
        assert_eq!(wants("delete the keyframes"), None);
        // A new name is kept whole, whatever its words could mean.
        assert_eq!(wants("rename it to Fade and Slide In").map(|(_, steps)| steps), Some(vec![Step::Rename("Fade and Slide In".to_owned())]));
        // Instructions alone stay instructions, and questions stay questions.
        assert!(matches!(ask_with("start at 5s and fade in", &[]), Reply::Do(_, Action::Instruct(_))));
        assert!(matches!(ask_with("bring it forward by 2 seconds", &[]), Reply::Do(_, Action::Instruct(_))), "a move in time, not in the stack");
        assert!(ask("which layers are hidden?").contains("hidden"));
        assert!(ask("what is selected").starts_with("Nothing is selected"));
    }

    #[test]
    fn chores_are_read_before_instructions_that_share_their_words() {
        use crate::chores::{Chore, Jump};
        use iris::intent::Time;
        let chore = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, Action::Chore(request)) => Some(request.chore),
            _ => None,
        };
        assert_eq!(chore("go to 5s"), Some(Chore::Go(Jump::To(Time::millis(5000)))));
        assert_eq!(chore("move the playhead to the end"), Some(Chore::Go(Jump::To(Time::End))));
        assert_eq!(chore("make the composition 10 seconds long"), Some(Chore::Duration(Time::millis(10_000))));
        assert_eq!(chore("sequence them one after another"), Some(Chore::Sequence));
        assert_eq!(chore("mute it"), Some(Chore::Mute(true)));
        assert_eq!(chore("/ready"), Some(Chore::Preflight));
        assert_eq!(chore("am I ready to export?"), Some(Chore::Preflight));
        // The same words about a layer are still instructions for it.
        assert!(matches!(ask_with("make it 4s long", &[]), Reply::Do(_, Action::Instruct(_))));
        assert!(matches!(ask_with("fade it in over a second", &[]), Reply::Do(_, Action::Instruct(_))));
        // And the questions are still answered.
        assert!(ask("is anything muted").contains("muted"));
        assert!(ask("where is the playhead").contains("frame 48"));
    }

    #[test]
    fn commands_are_run_by_their_names() {
        let run = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, Action::Run(index)) => Some(COMMANDS_FOR_TESTS[index].name),
            _ => None,
        };
        assert_eq!(run("export"), Some("Export"));
        assert_eq!(run("Open the export window please"), Some("Export"));
        assert_eq!(run("render"), Some("Export"), "by a word the app lists for it");
        assert_eq!(run("open settings"), Some("Settings"));
        assert_eq!(run("composition settings"), Some("Composition settings"));
        assert_eq!(run("guides"), Some("Show or hide guides"));
        // A name said in full is that command, whatever else its words could mean.
        assert_eq!(run("zoom the viewer in"), Some("Zoom the viewer in"));
        assert_eq!(run("split at the playhead"), Some("Split at the playhead"));
        assert!(matches!(ask_with("zoom in", &[]), Reply::Do(_, Action::Instruct(_))), "a way in for a layer");
        // Commands that fit equally are offered, not chosen between.
        assert!(matches!(ask_with("zoom the viewer", &[]), Reply::Do(_, Action::Offer(found)) if found.len() == 2));
        // Questions, housekeeping and the project's own answers come first.
        assert_eq!(run("how do I export?"), None);
        assert_eq!(run("duplicate"), None, "the selected layer is duplicated, with a reply that says so");
        assert_eq!(run("markers"), None, "the markers are listed");
        assert!(ask("composition settings?").contains("Frame rate"));
        assert_eq!(ask_with("export my holiday", &[]), Reply::Unknown);
    }

    #[test]
    fn too_much_halves_what_can_be_done_by_halves() {
        use iris::intent::{Step as Timing, Time};
        use iris::layers::Step;
        let action = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, action) => action,
            other => panic!("{message:?} asks for nothing: {other:?}"),
        };
        let softer = |message: &str| softened(&action(message));
        assert!(matches!(softer("make it bigger"), Some(Action::Housekeep(request)) if request.steps == [Step::Resize(110)]));
        assert!(matches!(softer("make it a bit smaller"), Some(Action::Housekeep(request)) if request.steps == [Step::Resize(95)]));
        assert!(matches!(softer("move it 20 px left"), Some(Action::Instruct(request)) if request.steps == [Timing::Nudge { x: -10, y: 0 }]));
        assert!(matches!(softer("delay it by a second"), Some(Action::Instruct(request)) if request.steps == [Timing::Shift { by: Time::millis(500), later: true }]));
        assert!(matches!(softer("make it louder"), Some(Action::Chore(request)) if request.chore == crate::chores::Chore::Volume(crate::chores::Level::By(15))));
        // The steps taken when no amount is said can be halved too.
        let softer_chore = |message: &str| match softer(message) {
            Some(Action::Chore(request)) => Some(request.chore),
            _ => None,
        };
        use crate::chores::{Chore, Ends, Pace};
        assert_eq!(softer_chore("make it longer"), Some(Chore::LengthBy { by: Time::millis(500), longer: true }));
        assert_eq!(softer_chore("move it later"), Some(Chore::ShiftBy { by: Time::millis(250), later: true }));
        assert_eq!(softer_chore("make the composition shorter"), Some(Chore::DurationBy { by: Time::millis(500), longer: false }));
        assert_eq!(softer_chore("make the fade slower"), Some(Chore::MotionPace { percent: 125, ends: Ends::Both }));
        assert_eq!(softer_chore("speed it up"), Some(Chore::Speed(Pace::Times { times: 225, over: 200 })));
        // Changes that are one thing or another have no half.
        for message in ["hide it", "start at 5s", "make it red", "go to 5s", "mute it", "export"] {
            assert_eq!(softer(message), None, "{message}");
        }
    }

    #[test]
    fn the_same_can_be_asked_for_another_layer() {
        let action = |message: &str| match ask_with(message, &[]) {
            Reply::Do(_, action) => action,
            other => panic!("{message:?} asks for nothing: {other:?}"),
        };
        let logo = Target::Named("logo".to_owned());
        assert!(matches!(retargeted(&action("fade it in"), "logo"), Some(Action::Instruct(request)) if request.target == logo && request.steps.len() == 1));
        assert!(matches!(retargeted(&action("hide the title layer"), "logo"), Some(Action::Housekeep(request)) if request.target == logo));
        assert!(matches!(retargeted(&action("make it longer"), "logo"), Some(Action::Chore(request)) if request.target == logo));
        assert!(matches!(retargeted(&action("mute it"), "logo"), Some(Action::Chore(request)) if request.target == logo));
        assert_eq!(retargeted(&action("go to 5s"), "logo"), None, "the playhead isn't a layer's");
        assert_eq!(retargeted(&action("export"), "logo"), None);
    }

    #[test]
    fn a_request_that_could_mean_several_things_is_asked_about() {
        let asked = |message: &str| match ask_with(message, &[]) {
            Reply::Do(question, Action::Ask(meanings)) => Some((question, meanings)),
            _ => None,
        };
        let (question, meanings) = asked("make it more subtle").unwrap();
        assert!(question.starts_with("More subtle in which way?") && meanings.contains(&"Make the fade slower".to_owned()));
        assert!(asked("more dramatic").is_some_and(|(_, meanings)| meanings.contains(&"Make it pulse".to_owned())));
        assert_eq!(asked("export as png").map(|(_, meanings)| meanings), Some(vec!["Save this frame as a PNG".to_owned(), "Export as a PNG sequence".to_owned()]));
        // Every meaning offered is itself understood, so picking one does something.
        for message in ["make it more subtle", "more dramatic", "export as png"] {
            for meaning in asked(message).unwrap().1 {
                assert!(matches!(ask_with(&meaning, &[]), Reply::Do(_, Action::Chore(_) | Action::Housekeep(_) | Action::Instruct(_))), "{meaning}");
            }
        }
        // A greeting beside a question asks for nothing of its own.
        assert!(is_small_talk("Hello") && is_small_talk("thanks!") && !is_small_talk("hello, how long is the video?") && !is_small_talk("fade in"));
        // A feel for animating the whole design is still that.
        assert!(matches!(ask_with("animate it, subtle", &[]), Reply::Do(_, Action::Animate { .. })));
    }

    #[test]
    fn something_it_cannot_answer_is_left_unknown() {
        assert_eq!(ask_with("xylophone marmalade", &[]), Reply::Unknown);
        assert_eq!(ask_with("", &[]), Reply::Unknown);
        // Asking how to do something is for the help answers, not for a fact about the project.
        assert_eq!(ask_with("How do I fix a clip?", &[]), Reply::Unknown);
        assert_eq!(ask_with("how to add a keyframe", &[]), Reply::Unknown);
    }
}
