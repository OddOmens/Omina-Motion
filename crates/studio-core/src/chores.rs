//! Reading Motion's own chores from a message typed to Iris: moving the playhead and playing,
//! the composition's settings and work area, arranging layers in time, sound, markers, making
//! layers, speed, and the check before exporting.
//!
//! Like the shared readers in the `iris` crate, this turns words into a `Request` with fixed
//! rules and knows nothing about the project; the app carries the chore out and says what
//! happened. It is tried before the shared instruction reader, which would take "make the
//! composition 10 seconds long" for a layer's length, so each rule here asks for the words
//! that make its meaning certain and leaves everything else alone.
//!
//! Moments and lengths ("frame 120", "half a second", "1:05", "the end") are read by the
//! shared instruction reader, so they are understood here exactly as they are in "start at
//! 5s". That reader only gives a time as part of a step, so the words are handed to it with a
//! plain instruction in front (see `moment` and `length`).

use iris::intent::{self, Step, Target, Time};

use crate::model::ShapeKind;

/// Where the playhead is asked to go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Jump {
    /// A moment: "5s", "frame 120", "the end".
    To(Time),
    /// So far from where it is, forward or back.
    By { by: Time, forward: bool },
    /// The next marker after the playhead, or the one before it.
    Marker { next: bool },
    /// The marker with this label.
    MarkerCalled(String),
    /// The next keyframe after the playhead, or the one before it.
    Keyframe { next: bool },
    /// Where the layer meant starts, or its last frame.
    LayerEdge { start: bool },
    /// Where the work area starts, or its last frame.
    WorkAreaEdge { start: bool },
}

/// A volume asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// A percentage of the sound as recorded: 100 leaves it alone, 50 is half as loud.
    Percent(i64),
    /// Tenths of a decibel: -60 is -6 dB.
    Decibels(i64),
    /// Louder (or, negative, quieter) by so many tenths of a decibel.
    By(i64),
}

/// Which end of a clip's sound a fade belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ends {
    In,
    Out,
    Both,
}

/// A speed asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// A percentage of normal speed.
    Percent(i64),
    /// So many times the speed it has now: twice as fast is 2 over 1, half speed 1 over 2.
    Times { times: i64, over: i64 },
    /// Back to normal: 100%, forwards, with no ramp.
    Normal,
}

/// Whose length the others take in "make them the same length".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// The first one selected.
    First,
    Longest,
    Shortest,
}

/// A layer to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Made {
    /// Text showing these words. Empty when the message didn't say what.
    Text(String),
    Solid,
    /// A shape; `even` when its sides are equal (a square, a circle).
    Shape { kind: ShapeKind, even: bool },
    /// A rectangle the size of the frame, behind everything.
    Background,
    Null,
    Adjustment,
}

/// A file format an export can be asked for by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// H.264 in an MP4 file.
    Mp4,
    /// H.265 (HEVC) in an MP4 file.
    Hevc,
    /// ProRes 422 in a QuickTime movie.
    Mov,
    /// ProRes 4444, which keeps transparency.
    MovWithAlpha,
    WebM,
    Gif,
    PngSequence,
    Lottie,
    DotLottie,
    Svg,
}

/// How much of a composition an export takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    WorkArea,
    Whole,
}

/// An export asked for outright, with enough said to start it without the Export dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    /// The formats named, in the order said. Empty means the one the Export dialog is set to.
    pub formats: Vec<Format>,
    /// Every composition in the project, rather than the open one.
    pub all: bool,
    /// How much of it, when said; otherwise as the Export dialog is set.
    pub span: Option<Span>,
    /// Export a copy made at this size first ("export a vertical version"): width, height and
    /// the label for the copy's name.
    pub size: Option<(u32, u32, String)>,
}

/// The step a length or a start changes by when no amount is said ("make it longer", "move
/// it later"), in milliseconds: a little, usually, a lot.
pub const LENGTH_STEPS: (i64, i64, i64) = (500, 1000, 2000);
pub const SHIFT_STEPS: (i64, i64, i64) = (200, 500, 1000);
/// How a speed changes when no amount is said ("speed it up"), as so many over 100: a little,
/// usually, a lot. Slowing down is the same the other way round.
const SPEED_STEPS: (i64, i64, i64) = (110, 125, 150);
/// How long an In or Out animation becomes when no amount is said ("make the fade slower"),
/// in percent of what it takes now: a little, usually, a lot.
const SLOWER_STEPS: (i64, i64, i64) = (125, 150, 200);
const FASTER_STEPS: (i64, i64, i64) = (80, 67, 50);

/// One chore asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Chore {
    Go(Jump),
    /// Start playing, from a moment when one is said.
    Play { from: Option<Time> },
    Pause,
    /// Make the composition this long.
    Duration(Time),
    /// Set the composition's frame rate, in thousandths of a frame a second (23976 is 23.976).
    FrameRate(u32),
    /// Set the composition's own size (not a resized copy). A side left `None` is kept.
    Size { width: Option<u32>, height: Option<u32> },
    /// Set the work area's start, its end, or both.
    WorkArea { from: Option<Time>, to: Option<Time> },
    /// Put the work area back over the whole composition.
    WorkAreaWhole,
    /// Put the work area around the layers meant.
    WorkAreaAround,
    /// Cut the composition down to its work area.
    TrimToWorkArea,
    /// Start each layer so long after the one before (the app's usual step when `None`).
    Stagger(Option<Time>),
    /// Play the layers one after another.
    Sequence,
    /// Start every layer at this moment.
    LineUp(Time),
    CloseGaps,
    SameLength(Pick),
    /// Swap the layers' places in time, so the last comes first.
    ReverseOrder,
    Mute(bool),
    Volume(Level),
    /// Fade the sound in or out over this long (the usual length when `None`; zero takes the
    /// fade away).
    SoundFade { ends: Ends, length: Option<Time> },
    /// Turn music down under a voice, or stop doing so.
    Duck(bool),
    /// Add a marker, or name the one already there.
    Marker { at: Time, label: Option<String> },
    RemoveMarker(Time),
    ClearMarkers,
    /// Make a layer, in this color when one is said (red, green and blue with its name).
    Make { what: Made, color: Option<([u8; 3], String)> },
    Speed(Pace),
    Reverse(bool),
    /// Hold the frame under the playhead.
    Freeze,
    /// Check the project before exporting.
    Preflight,
    /// Make the composition so much longer or shorter than it is.
    DurationBy { by: Time, longer: bool },
    /// Make each layer so much longer or shorter than it is.
    LengthBy { by: Time, longer: bool },
    /// Move each layer so much later or earlier, for a request that says no amount.
    ShiftBy { by: Time, later: bool },
    /// Make the layers' In and Out animations take this many percent of the time they take now.
    MotionPace { percent: i64, ends: Ends },
    /// Export straight away, without the Export dialog.
    Export(Export),
    /// Save the frame under the playhead as a picture.
    ExportFrame,
    /// Show the file Iris last exported in the Finder.
    RevealExport,
    /// Save the frame under the playhead as a picture and hand it to another of Iris's apps.
    SendFrame(iris::App),
}

/// A message read as a chore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub chore: Chore,
    /// Which layers it is about, for the chores that are about layers.
    pub target: Target,
}

/// The longest label a marker is given, in characters.
const LABEL_LIMIT: usize = 60;
/// How much "louder" and "quieter" change the volume, in tenths of a decibel: a little,
/// usually, a lot.
const LOUDER_BY: (i64, i64, i64) = (15, 30, 60);
/// The frame rates that can be asked for, in frames a second.
const RATE_RANGE: (f64, f64) = (1.0, 240.0);

/// Words for the composition itself.
const COMP_WORDS: &[&str] = &["composition", "comp", "timeline", "sequence"];
/// Words that ask for something to be made.
const MAKE_VERBS: &[&str] = &["add", "create", "insert", "new", "draw", "make", "write"];
/// The most words between "add" and what is added: "add a nice dark red *solid*".
const BEFORE_NOUN: usize = 4;
/// Words that stand for layers between "line" and "up", or after "sequence".
const THEM: &[&str] = &["them", "these", "those", "the", "layers", "clips", "selected", "everything", "all", "it"];
/// Words that end the search for what is added: "add a fade *to* the title" adds no title.
const PREPOSITIONS: &[&str] = &["to", "on", "onto", "for", "of", "in", "at", "into", "from", "with", "under", "over"];
/// Words that ask for something to be taken away.
const REMOVING: &[&str] = &["remove", "delete", "clear", "erase", "no", "without", "rid"];

/// A moment said in words ("5s", "frame 120", "1:05", "the end", "the playhead", "here").
fn moment(text: &str) -> Option<Time> {
    let text = text.trim();
    let text = text.strip_prefix("at ").or_else(|| text.strip_prefix("to ")).unwrap_or(text);
    match intent::parse(&format!("end at {text}"))?.steps.as_slice() {
        [Step::EndAt(time)] => Some(*time),
        _ => None,
    }
}

/// A length of time said in words ("5 frames", "0.2s", "half a second", "2 minutes").
fn length(text: &str) -> Option<Time> {
    let text = text.trim();
    let text = text.strip_prefix("by ").or_else(|| text.strip_prefix("of ")).unwrap_or(text);
    match intent::parse(&format!("last {text}"))?.steps.as_slice() {
        [Step::Last(time)] => Some(*time),
        _ => None,
    }
}

/// The first length of time said anywhere in `text`: each place a number could start is tried.
fn first_length(text: &str) -> Option<Time> {
    const NUMBER_WORDS: &[&str] = &["a", "an", "half", "quarter", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "twenty", "thirty"];
    let mut at = 0;
    for word in text.split(' ') {
        let starts_number = word.chars().next().is_some_and(|first| first.is_ascii_digit()) || NUMBER_WORDS.contains(&word);
        if starts_number && let Some(found) = length(&text[at..]) {
            return Some(found);
        }
        at += word.len() + 1;
    }
    None
}

/// A color said anywhere in the message ("red", "dark blue", "#ff8800"), read as the shared
/// housekeeping reader reads "make it red".
fn color_said(message: &str) -> Option<([u8; 3], String)> {
    let words: Vec<&str> = message.split_whitespace().map(|word| word.trim_matches(|character: char| matches!(character, ',' | '.' | '!' | '"' | '“' | '”'))).collect();
    let color_of = |text: &str| {
        iris::layers::parse(&format!("make it {text}"))?.steps.into_iter().find_map(|step| match step {
            iris::layers::Step::Color { rgb, name } => Some((rgb, name)),
            _ => None,
        })
    };
    (0..words.len()).find_map(|index| {
        let plain = color_of(words[index])?;
        // "Dark red", "light blue": the word before may shade it.
        let shaded = index.checked_sub(1).and_then(|before| color_of(&format!("{} {}", words[before], words[index])));
        Some(shaded.unwrap_or(plain))
    })
}

/// A message made ready to read: its text in lower case without the politeness around it, and
/// its plain words.
struct Said<'a> {
    /// The message as typed, for names and text that keep their capitals.
    typed: &'a str,
    /// Lower case, single spaces, without "please", "can you" or the stop at the end. Numbers
    /// keep their marks ("1:05", "0.5s", "50%").
    text: String,
    /// The words of `text`, letters and digits only.
    words: Vec<String>,
}

impl<'a> Said<'a> {
    fn new(typed: &'a str) -> Self {
        let lower = typed.to_lowercase().replace(['’', '‘'], "'");
        let mut text = lower.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim_end_matches(['.', '!', '?']).to_owned();
        for start in ["please ", "can you please ", "could you please ", "can you ", "could you ", "would you ", "will you ", "iris, ", "iris ", "ok ", "okay ", "now ", "just ", "let's ", "lets "] {
            if let Some(rest) = text.strip_prefix(start) {
                text = rest.to_owned();
            }
        }
        for end in [" please", " for me", " now", " thanks"] {
            if let Some(rest) = text.strip_suffix(end) {
                text = rest.to_owned();
            }
        }
        let words = text.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_owned).collect();
        Self { typed, text, words }
    }

    fn has(&self, word: &str) -> bool {
        self.words.iter().any(|said| said == word)
    }

    fn any(&self, words: &[&str]) -> bool {
        self.words.iter().any(|said| words.contains(&said.as_str()))
    }

    /// What follows the first of `marks` in the text.
    fn after(&self, marks: &[&str]) -> Option<&str> {
        let (at, mark) = marks.iter().filter_map(|mark| self.text.find(mark).map(|at| (at, mark))).min_by_key(|(at, _)| *at)?;
        Some(self.text[at + mark.len()..].trim())
    }

    /// What follows the first of `marks` in the message as it was typed, capitals kept. The
    /// marks are plain letters, so they sit at the same place in the typed message.
    fn typed_after(&self, marks: &[&str]) -> Option<&'a str> {
        let lower = self.typed.to_lowercase();
        let (at, mark) = marks.iter().filter_map(|mark| lower.find(mark).map(|at| (at, mark))).min_by_key(|(at, _)| *at)?;
        self.typed.get(at + mark.len()..).map(str::trim)
    }

    /// The moment after an "at", trying the last "at" first: a label may have one of its own
    /// ("a marker called Look At Me at 3s").
    fn moment_after_at(&self) -> Option<Time> {
        self.text.rmatch_indices(" at ").find_map(|(at, mark)| moment(&self.text[at + mark.len()..]))
    }

    /// The first text in quotes, as typed.
    fn quoted(&self) -> Option<&'a str> {
        for (open, close) in [('“', '”'), ('"', '"')] {
            if let Some(start) = self.typed.find(open) {
                let rest = &self.typed[start + open.len_utf8()..];
                if let Some(end) = rest.find(close) {
                    return Some(rest[..end].trim()).filter(|text| !text.is_empty());
                }
            }
        }
        None
    }

    /// The label in "go to the Chorus marker", as typed: the words between "to (the)" and
    /// "marker".
    fn typed_before_marker(&self) -> Option<&'a str> {
        let lower = self.typed.to_lowercase();
        let end = lower.rfind(" marker")?;
        let start = lower[..end].rfind(" to ")? + " to ".len();
        let label = self.typed.get(start..end)?.trim();
        Some(label.strip_prefix("the ").or_else(|| label.strip_prefix("The ")).unwrap_or(label))
    }

    /// The first number that is a percentage: "50%", "50 percent".
    fn percent(&self) -> Option<i64> {
        let tokens: Vec<&str> = self.text.split(' ').collect();
        tokens.iter().enumerate().find_map(|(index, token)| {
            let token = token.trim_end_matches(',');
            let figure = match token.strip_suffix('%') {
                Some(figure) => figure,
                None if matches!(tokens.get(index + 1), Some(&("percent" | "%"))) => token,
                None => return None,
            };
            figure.parse::<f64>().ok().filter(|value| value.is_finite()).map(|value| value.round() as i64)
        })
    }

    /// The first number followed by `unit`, joined to it or as the next word: "-6db", "-6 db".
    fn number_with(&self, units: &[&str]) -> Option<f64> {
        let tokens: Vec<&str> = self.text.split(' ').collect();
        tokens.iter().enumerate().find_map(|(index, token)| {
            let joined = units.iter().find_map(|unit| token.strip_suffix(unit)).filter(|figure| !figure.is_empty());
            let figure = match joined {
                Some(figure) => figure,
                None if tokens.get(index + 1).is_some_and(|next| units.contains(next)) => token,
                None => return None,
            };
            figure.parse::<f64>().ok().filter(|value| value.is_finite())
        })
    }

    /// The first plain number in what follows the first of `marks`.
    fn number_after(&self, marks: &[&str]) -> Option<f64> {
        self.after(marks)?.split(' ').find_map(|token| token.trim_end_matches(',').parse::<f64>().ok().filter(|value| value.is_finite()))
    }
}

/// Words that introduce a name or text to show, which is kept as typed and never read as a
/// request: "add a marker *called* Clear Sky", "add a title *saying* mute the music".
const WORDING_MARKS: &[&str] = &[" called ", " named ", " labeled ", " labelled ", " saying ", " that says ", " which says ", " reading ", " that reads ", " with the text ", " with the words ", "“", "\""];

/// The part of a message that asks for something: what comes before a name or text to show.
fn request_part(message: &str) -> &str {
    let lower = message.to_lowercase();
    let cut = WORDING_MARKS.iter().filter_map(|mark| lower.find(mark)).min();
    cut.and_then(|cut| message.get(..cut)).unwrap_or(message)
}

/// A typed label made fit to keep: without the quotes and stops around it, and not too long.
fn tidy_label(typed: &str) -> Option<String> {
    let label = typed.trim().trim_end_matches(['.', '!']).trim_matches(|character: char| matches!(character, '"' | '“' | '”' | '‘' | '’' | '\'')).trim();
    let label: String = label.chars().take(LABEL_LIMIT).collect();
    (!label.is_empty()).then(|| label.trim_end().to_owned())
}

/// "Am I ready to export?", "is this ready to render", "preflight": the one chore that is
/// asked as a question.
fn preflight(said: &Said) -> Option<Chore> {
    let whole = matches!(said.text.as_str(), "preflight" | "pre-flight" | "pre flight" | "preflight check" | "final check" | "export check" | "ready to export" | "ready to render" | "am i ready" | "is it ready" | "is this ready" | "are we ready");
    let ready = said.any(&["ready", "good"]) && said.any(&["export", "render", "deliver", "publish", "ship", "upload", "post"]) && said.any(&["am", "is", "are", "we", "i", "it", "this"]);
    let before = said.has("check") && said.any(&["before", "export", "exporting"]) && said.any(&["export", "exporting", "render", "rendering"]);
    (whole || ready || before).then_some(Chore::Preflight)
}

/// "Play", "pause", "stop", "play from the start".
fn playback(said: &Said) -> Option<Chore> {
    const PLAY: &[&str] = &["play", "play it", "play this", "play it back", "play the video", "play the composition", "play the comp", "play the preview", "press play", "hit play", "start playing", "start playback", "playback", "play from here", "resume", "resume playing"];
    const PAUSE: &[&str] = &["pause", "pause it", "pause playback", "pause the video", "stop", "stop it", "stop playing", "stop playback", "stop the video", "stop the playback"];
    let text = said.text.as_str();
    if PLAY.contains(&text) {
        return Some(Chore::Play { from: None });
    }
    if PAUSE.contains(&text) {
        return Some(Chore::Pause);
    }
    let from = ["play from ", "play it from ", "play back from ", "play this from "].iter().find_map(|start| text.strip_prefix(start))?;
    moment(from).map(|time| Chore::Play { from: Some(time) })
}

/// "Go to 5s", "jump to frame 120", "go to the next marker", "back 10 frames", "move the
/// playhead to the end".
fn go(said: &Said) -> Option<Chore> {
    const TO: &[&str] = &[
        "go to ", "goto ", "jump to ", "skip to ", "seek to ", "scrub to ", "cue to ", "take me to ", "move the playhead to ", "put the playhead at ", "put the playhead on ", "set the playhead to ", "set the playhead at ", "move playhead to ", "playhead to ", "playhead at ",
    ];
    const FORWARD: &[&str] = &["forward ", "forwards ", "ahead ", "skip ahead ", "skip forward ", "go forward ", "jump forward ", "jump ahead ", "advance "];
    const BACK: &[&str] = &["back ", "backward ", "backwards ", "rewind ", "go back ", "skip back ", "jump back "];
    let text = said.text.as_str();
    let neighbor = |place: &str| -> Option<Jump> {
        let place = place.strip_prefix("the ").unwrap_or(place);
        let (way, thing) = place.split_once(' ')?;
        let next = match way {
            "next" | "following" => true,
            "previous" | "prev" | "last" | "preceding" => false,
            _ => return None,
        };
        match thing {
            "marker" => Some(Jump::Marker { next }),
            "keyframe" | "key" => Some(Jump::Keyframe { next }),
            "frame" => Some(Jump::By { by: Time::Span { millis: 0, frames: 1 }, forward: next }),
            _ => None,
        }
    };
    if let Some(found) = neighbor(text) {
        return Some(Chore::Go(found));
    }
    if text == "rewind" {
        return Some(Chore::Go(Jump::To(Time::START)));
    }
    if let Some(place) = TO.iter().find_map(|start| text.strip_prefix(start)) {
        if let Some(found) = neighbor(place) {
            return Some(Chore::Go(found));
        }
        // "The Chorus marker", "marker “Chorus”", "the marker called Chorus".
        if said.has("marker") {
            let label = said.quoted().or_else(|| said.typed_after(&["marker called ", "marker named ", "marker labeled ", "marker labelled ", "marker "])).or_else(|| said.typed_before_marker());
            return label.and_then(tidy_label).map(|label| Chore::Go(Jump::MarkerCalled(label)));
        }
        // "The start of the title", "the end of the work area", "where it ends".
        let bare = place.strip_prefix("the ").unwrap_or(place);
        let edge = [("start of ", true), ("beginning of ", true), ("end of ", false)].iter().find_map(|(lead, start)| bare.strip_prefix(lead).map(|of| (of, *start))).or_else(|| {
            let of = bare.strip_prefix("where ")?;
            of.strip_suffix(" starts").or_else(|| of.strip_suffix(" begins")).map(|of| (of, true)).or_else(|| of.strip_suffix(" ends").map(|of| (of, false)))
        });
        if let Some((of, start)) = edge {
            let of_comp = of.split(' ').any(|word| COMP_WORDS.contains(&word) || matches!(word, "video" | "everything"));
            return Some(Chore::Go(if of.contains("work area") {
                Jump::WorkAreaEdge { start }
            } else if of_comp {
                Jump::To(if start { Time::START } else { Time::End })
            } else {
                Jump::LayerEdge { start }
            }));
        }
        return moment(place).map(|time| Chore::Go(Jump::To(time)));
    }
    for (starts, forward) in [(FORWARD, true), (BACK, false)] {
        if let Some(by) = starts.iter().find_map(|start| text.strip_prefix(start)).and_then(length) {
            return Some(Chore::Go(Jump::By { by, forward }));
        }
    }
    None
}

/// "Set the work area from 2s to 6s", "end the work area here", "reset the work area", "trim
/// the composition to the work area".
fn work_area(said: &Said) -> Option<Chore> {
    let about = ["work area", "workarea", "preview range", "render range"].iter().any(|name| said.text.contains(name));
    if !about {
        return None;
    }
    if said.any(&["trim", "crop", "cut", "shorten"]) {
        // "Trim it to the work area" may mean a layer, so the composition has to be what is trimmed.
        let about_a_layer = said.any(&["layer", "layers", "clip", "clips", "it", "them", "this", "these"]) && !said.any(COMP_WORDS);
        return (!about_a_layer).then_some(Chore::TrimToWorkArea);
    }
    if said.any(&["whole", "entire", "reset", "clear", "full", "everything"]) {
        return Some(Chore::WorkAreaWhole);
    }
    // "From 2s to 6s", "between 2s and 6s", or just "2s to 6s" after its name.
    let both = |range: &str, joiner: &str| {
        let (from, to) = range.split_once(joiner)?;
        Some(Chore::WorkArea { from: Some(moment(from)?), to: Some(moment(to)?) })
    };
    if let Some(found) = said.after(&[" from "]).and_then(|range| both(range, " to ")).or_else(|| said.after(&[" between "]).and_then(|range| both(range, " and "))) {
        return Some(found);
    }
    let named = said.after(&["work area ", "workarea ", "preview range ", "render range "]).map(|rest| rest.strip_prefix("to ").unwrap_or(rest));
    if let Some(found) = named.and_then(|range| both(range, " to ")) {
        return Some(found);
    }
    // One end: "start the work area at 2s", "set the work area's end to the playhead".
    let at = said.after(&[" at ", " to "]).and_then(moment).or_else(|| said.any(&["here", "playhead", "now"]).then_some(Time::Playhead));
    if let Some(at) = at {
        if said.any(&["start", "starts", "begin", "begins", "beginning", "in"]) {
            return Some(Chore::WorkArea { from: Some(at), to: None });
        }
        if said.any(&["end", "ends", "finish", "out", "stop", "stops"]) {
            return Some(Chore::WorkArea { from: None, to: Some(at) });
        }
    }
    said.any(&["selection", "selected", "layer", "layers", "around", "them", "these"]).then_some(Chore::WorkAreaAround)
}

/// The composition's frame rate, size and length.
fn composition(said: &Said) -> Option<Chore> {
    let setting = said.any(&["set", "make", "change", "switch", "use", "put", "convert"]);
    // "Set the frame rate to 30", "make it 24 fps", "30 fps".
    let about_rate = said.has("fps") || ["frame rate", "framerate", "frames per second", "frames a second"].iter().any(|name| said.text.contains(name)) || said.words.iter().any(|word| word.ends_with("fps"));
    if about_rate && !said.any(&["export", "render", "in", "how"]) {
        let starts_with_it = said.text.starts_with(|first: char| first.is_ascii_digit()) || said.text.starts_with("frame rate") || said.text.starts_with("fps");
        let rate = said.number_with(&["fps"]).or_else(|| said.number_after(&[" to ", " at ", " of ", "frame rate ", "framerate ", "fps "]));
        return rate.filter(|rate| (setting || starts_with_it) && (RATE_RANGE.0..=RATE_RANGE.1).contains(rate)).map(|rate| Chore::FrameRate((rate * 1000.0).round() as u32));
    }
    // "Set the size to 1080x1920": the composition's own size. Asking for another size any
    // other way makes a resized copy (see `chat`).
    if said.has("set") && said.any(&["size", "dimension", "dimensions", "resolution"]) && !said.any(&["copy", "version", "layer"]) {
        return crate::chat::wanted_size(said.typed).map(|(width, height, _)| Chore::Size { width: Some(width), height: Some(height) });
    }
    let about_comp = said.any(COMP_WORDS);
    if about_comp && said.has("set") && said.any(&["width", "height"]) {
        let side = said.number_after(&[" to "]).filter(|side| (16.0..=8192.0).contains(side)).map(|side| side.round() as u32)?;
        return Some(if said.has("width") { Chore::Size { width: Some(side), height: None } } else { Chore::Size { width: None, height: Some(side) } });
    }
    // "Make the composition 10 seconds long", "extend the comp to 15s". "Shorten it by 2
    // seconds" changes the length by an amount, which `length_by` reads.
    if about_comp && said.any(&["long", "length", "duration", "last", "lasts", "extend", "shorten", "lengthen", "longer", "shorter"]) && !said.has("by") {
        return said.after(&[" to "]).and_then(length).or_else(|| first_length(&said.text)).map(Chore::Duration);
    }
    None
}

/// Arranging several layers in time.
fn arranging(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    if said.any(&["stagger", "cascade", "staggered"]) || (said.has("offset") && said.any(&["each", "them", "layers", "these"])) {
        return Some(Chore::Stagger(said.after(&[" by "]).and_then(length).or_else(|| first_length(text))));
    }
    // "Sequence them", "sequence the layers": the verb, not the composition ("sequence settings").
    let sequencing = said.words.first().is_some_and(|first| first == "sequence") && said.words.get(1).is_none_or(|next| THEM.contains(&next.as_str())) && !said.any(&["settings", "length", "duration", "name"]);
    if sequencing || ["one after another", "one after the other", "back to back", "end to end", "after each other"].iter().any(|phrase| text.contains(phrase)) {
        return Some(Chore::Sequence);
    }
    // "Line them all up at the playhead", "line up the layers at 2s". Only layers may come
    // between the two words: "rise up line by line" is a way in.
    let lining_up = said.words.iter().position(|word| word == "line" || word == "lined").is_some_and(|line| {
        let between = said.words[line + 1..].iter().take_while(|word| *word != "up").count();
        between <= BEFORE_NOUN && said.words.get(line + 1 + between).is_some_and(|word| word == "up") && said.words[line + 1..line + 1 + between].iter().all(|word| THEM.contains(&word.as_str()))
    });
    if lining_up {
        return Some(Chore::LineUp(said.after(&[" at ", " with ", " to ", " on "]).and_then(moment).unwrap_or(Time::Playhead)));
    }
    if said.any(&["gap", "gaps"]) && said.any(&["close", "closing", "remove", "delete", "kill", "rid", "fill", "tighten"]) {
        return Some(Chore::CloseGaps);
    }
    if ["same length", "same duration", "equal length", "equally long"].iter().any(|phrase| text.contains(phrase)) {
        let pick = if said.has("longest") {
            Pick::Longest
        } else if said.has("shortest") {
            Pick::Shortest
        } else {
            Pick::First
        };
        return Some(Chore::SameLength(pick));
    }
    (said.has("order") && said.any(&["reverse", "flip", "swap", "invert"])).then_some(Chore::ReverseOrder)
}

/// "Add a marker here", "add a marker called Chorus at 3s", "remove the marker here", "clear
/// the markers".
///
/// `asked` is the part of the message before any label, where the request itself is read, so
/// a marker can be called "Clear Sky" without being cleared.
fn markers(said: &Said, asked: &Said) -> Option<Chore> {
    let text = asked.text.as_str();
    let marking = text == "mark here" || text.starts_with("mark this ") || text.starts_with("mark the ");
    if !asked.any(&["marker", "markers"]) && !marking {
        return None;
    }
    let at = said.moment_after_at();
    if asked.any(&["remove", "delete", "clear", "erase", "rid"]) {
        let all = asked.any(&["all", "every"]) || (asked.has("markers") && at.is_none() && !asked.any(&["here", "this"]));
        return Some(if all { Chore::ClearMarkers } else { Chore::RemoveMarker(at.unwrap_or(Time::Playhead)) });
    }
    let naming = asked.any(&["rename", "relabel", "call", "label", "name"]);
    if !marking && !naming && !asked.any(&["add", "put", "drop", "set", "place", "make", "create", "new", "insert", "leave"]) && !text.starts_with("marker ") {
        return None;
    }
    // The label: in quotes, or after "called". An "at 3s" after it is when, not part of it.
    let label = said.quoted().or_else(|| said.typed_after(&[" called ", " named ", " labeled ", " labelled ", " saying ", " that says "])).or_else(|| if naming { said.typed_after(&[" marker to ", " marker as ", " marker "]) } else { None });
    let label = label.map(|label| {
        let lower = label.to_lowercase();
        match lower.rfind(" at ").filter(|cut| moment(&lower[cut + 4..]).is_some()) {
            Some(cut) => label.get(..cut).unwrap_or(label),
            None => label,
        }
    });
    Some(Chore::Marker { at: at.unwrap_or(Time::Playhead), label: label.and_then(tidy_label) })
}

/// Muting, volume, fades of the sound and ducking.
fn sound(said: &Said) -> Option<Chore> {
    if said.any(&["duck", "ducking", "unduck"]) {
        let stop = said.any(&["stop", "remove", "no", "unduck", "off", "without", "don't", "dont"]);
        return Some(Chore::Duck(!stop));
    }
    if said.has("unmute") {
        return Some(Chore::Mute(false));
    }
    if said.has("mute") || said.text.starts_with("silence ") {
        return Some(Chore::Mute(true));
    }
    let about_sound = said.any(&["audio", "sound", "music", "volume", "voice", "voiceover", "song"]);
    if about_sound && said.any(&["fade", "fades"]) {
        let ends = match (said.has("in"), said.any(&["out", "down", "away"])) {
            (true, true) => Ends::Both,
            (_, true) => Ends::Out,
            _ => Ends::In,
        };
        let length = if said.any(REMOVING) { Some(Time::millis(0)) } else { said.after(&[" over ", " for ", " lasting "]).and_then(length).or_else(|| first_length(&said.text)) };
        return Some(Chore::SoundFade { ends, length });
    }
    let about_volume = said.any(&["volume", "gain", "loudness"]);
    if let Some(decibels) = said.number_with(&["db", "decibels", "decibel"]).filter(|_| about_volume || about_sound) {
        return Some(Chore::Volume(Level::Decibels((decibels * 10.0).round() as i64)));
    }
    if about_volume && let Some(percent) = said.percent().or_else(|| said.number_after(&[" to ", " at "]).map(|value| value.round() as i64)) {
        return Some(Chore::Volume(Level::Percent(percent)));
    }
    // "Louder", "a bit quieter", "turn the music down".
    // "Turn it up" needs no more words; "turn it upside down" is something else.
    let turned = said.has("turn") && (about_sound || matches!(said.words.as_slice(), [turn, it, way, ..] if turn == "turn" && matches!(it.as_str(), "it" | "this" | "them") && matches!(way.as_str(), "up" | "down")));
    let way = if said.has("louder") || ((turned || about_volume) && said.any(&["up", "raise", "increase", "boost"])) {
        1
    } else if said.has("quieter") || ((turned || about_volume) && said.any(&["down", "lower", "reduce", "decrease"])) {
        -1
    } else {
        return None;
    };
    let by = if said.any(&["bit", "little", "slightly", "touch", "tad"]) {
        LOUDER_BY.0
    } else if said.any(&["lot", "much", "way", "far"]) {
        LOUDER_BY.2
    } else {
        LOUDER_BY.1
    };
    Some(Chore::Volume(Level::By(way * by)))
}

/// "Add a text layer saying “Hello”", "add a red solid", "add a circle", "new null object".
///
/// `asked` is the part of the message before any text to show, where the request itself is
/// read, so the text can say anything.
fn making(said: &Said, asked: &Said) -> Option<Chore> {
    // "Make" also changes what is there ("make the title red"), so it only counts with "a" or
    // "new" straight after it.
    let verb = asked.words.iter().enumerate().position(|(index, word)| match word.as_str() {
        "make" => asked.words.get(index + 1).is_some_and(|next| matches!(next.as_str(), "a" | "an" | "another" | "new" | "some")),
        word => MAKE_VERBS.contains(&word),
    })?;
    // What is added is named straight after the verb: "add a fade *to the title*" adds no title.
    let named = asked.words[verb + 1..].iter().take(BEFORE_NOUN + 1).take_while(|word| !PREPOSITIONS.contains(&word.as_str())).find_map(|word| {
        let shape = |kind, even| Made::Shape { kind, even };
        Some(match word.as_str() {
            "text" | "title" | "headline" | "heading" => Made::Text(String::new()),
            "solid" => Made::Solid,
            "background" | "backdrop" => Made::Background,
            "null" => Made::Null,
            "adjustment" => Made::Adjustment,
            "rectangle" | "rect" | "box" => shape(ShapeKind::Rectangle, false),
            "square" | "shape" => shape(ShapeKind::Rectangle, true),
            "circle" => shape(ShapeKind::Ellipse, true),
            "ellipse" | "oval" => shape(ShapeKind::Ellipse, false),
            "triangle" => shape(ShapeKind::Triangle, true),
            "star" => shape(ShapeKind::Star, true),
            "polygon" | "hexagon" | "pentagon" => shape(ShapeKind::Polygon, true),
            _ => return None,
        })
    });
    let wording = said.quoted().or_else(|| said.typed_after(&[" saying ", " that says ", " which says ", " reading ", " that reads ", " with the text ", " with the words "]));
    let what = match named {
        // Another size is asked for with the same words ("make a square version").
        Some(Made::Shape { .. }) if asked.any(&["version", "copy", "format", "composition", "comp", "video", "size"]) => return None,
        Some(Made::Text(_)) => Made::Text(wording.map(|text| text.trim_matches(|character: char| matches!(character, '"' | '“' | '”')).trim().to_owned()).unwrap_or_default()),
        Some(other) => other,
        // "Write “Hello”" needs no noun.
        None if asked.words[verb] == "write" => Made::Text(said.quoted()?.to_owned()),
        None => return None,
    };
    // A color said inside the text to show is part of the text, not the layer's color.
    let color = if matches!(what, Made::Text(_)) { None } else { color_said(said.typed) };
    Some(Chore::Make { what, color })
}

/// "Make it twice as fast", "50% speed", "reverse it", "freeze frame here".
fn speed(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    if said.has("freeze") || ["hold this frame", "frame hold", "hold frame", "hold the frame"].iter().any(|phrase| text.contains(phrase)) {
        return Some(Chore::Freeze);
    }
    // "Backwards" and "forwards" are about playing only when playing is said: "move it
    // backwards" is about the stacking order.
    let playing = said.any(&["play", "plays", "playing", "run", "runs"]);
    if said.has("unreverse") || text.contains("un-reverse") || (playing && said.any(&["forward", "forwards"])) {
        return Some(Chore::Reverse(false));
    }
    if said.any(&["reverse", "reversed"]) || (playing && said.any(&["backward", "backwards"])) {
        return Some(Chore::Reverse(true));
    }
    if ["normal speed", "regular speed", "original speed", "reset the speed", "reset speed", "real time"].iter().any(|phrase| text.contains(phrase)) {
        return Some(Chore::Speed(Pace::Normal));
    }
    if ["slow motion", "slow mo", "slow-mo", "slo-mo", "slowmo"].iter().any(|phrase| text.contains(phrase)) {
        return Some(Chore::Speed(Pace::Percent(50)));
    }
    let about = said.any(&["speed", "fast", "faster", "slow", "slower", "quick"]);
    if !about {
        return None;
    }
    let slower = said.any(&["slow", "slower"]) && !said.any(&["fast", "faster"]);
    // "Three times as fast", "twice the speed", "half speed".
    let times = if said.any(&["twice", "double"]) {
        Some(2)
    } else if said.any(&["triple", "thrice"]) {
        Some(3)
    } else {
        said.words.iter().position(|word| word == "times").and_then(|at| at.checked_sub(1)).and_then(|before| match said.words[before].as_str() {
            "two" => Some(2),
            "three" => Some(3),
            "four" => Some(4),
            "five" => Some(5),
            "ten" => Some(10),
            figure => figure.parse::<i64>().ok().filter(|times| (2..=20).contains(times)),
        })
    };
    if let Some(times) = times {
        return Some(Chore::Speed(if slower { Pace::Times { times: 1, over: times } } else { Pace::Times { times, over: 1 } }));
    }
    // "Half" and a percentage are said of many things ("fade in over half a second, fast",
    // "50% opacity"), so they count only right beside the speed.
    for (part, over) in [("half", 2), ("quarter", 4)] {
        if ["speed", "as fast", "the speed", "its speed"].iter().any(|what| text.contains(&format!("{part} {what}"))) {
            return Some(Chore::Speed(Pace::Times { times: 1, over }));
        }
    }
    let percent_of_speed = said.any(&["speed", "play", "plays", "playback"]) || ["% faster", "% slower", "slow it down to", "slow down to", "slow it to"].iter().any(|phrase| text.contains(phrase));
    if let Some(percent) = said.percent().filter(|_| percent_of_speed) {
        // "20% faster" and "30% slower" are measured from the speed it has now.
        return Some(Chore::Speed(if text.contains("% faster") {
            Pace::Times { times: 100 + percent, over: 100 }
        } else if text.contains("% slower") {
            Pace::Times { times: (100 - percent).max(1), over: 100 }
        } else {
            Pace::Percent(percent)
        }));
    }
    // "2x speed", "play it at 1.5x".
    if let Some(times) = said.number_with(&["x"]) {
        return Some(Chore::Speed(Pace::Percent((times * 100.0).round() as i64)));
    }
    // "Speed it up", "slow it down", "faster": a moderate step from the speed it has. Said of
    // an animation ("fade in faster"), it is about the animation instead.
    if said.any(&["animate", "animation", "fade", "slide", "pop", "intro", "outro"]) {
        return None;
    }
    let step = step_said(said, SPEED_STEPS);
    let up = said.any(&["faster", "quicker"]) || ["speed it up", "speed up", "speed this up", "speed them up"].iter().any(|phrase| text.contains(phrase));
    let down = said.has("slower") || ["slow it down", "slow down", "slow this down", "slow them down"].iter().any(|phrase| text.contains(phrase));
    match (up, down) {
        (true, false) => Some(Chore::Speed(Pace::Times { times: step, over: 100 })),
        (false, true) => Some(Chore::Speed(Pace::Times { times: 100, over: step })),
        _ => None,
    }
}

/// Which of three steps a request with no amount means, from the words around it: "a bit",
/// nothing, or "a lot".
fn step_said(said: &Said, steps: (i64, i64, i64)) -> i64 {
    if said.any(&["bit", "little", "slightly", "touch", "tad"]) {
        steps.0
    } else if said.any(&["lot", "much", "way", "far"]) {
        steps.2
    } else {
        steps.1
    }
}

/// "Make the fade slower", "a faster intro", "speed the animation up": how long the layer's
/// way in or out takes, with no amount said.
fn motion_pace(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    let about = said.any(&["fade", "fades", "animation", "animations", "intro", "outro", "entrance", "exit", "transition"]) || text.contains("way in") || text.contains("way out");
    if !about || first_length(text).is_some() || said.percent().is_some() {
        return None;
    }
    let slower = said.any(&["slower", "longer", "gentler"]) || (said.has("slow") && said.has("down"));
    let faster = said.any(&["faster", "quicker", "shorter", "snappier"]) || (said.has("speed") && said.has("up"));
    let percent = match (slower, faster) {
        (true, false) => step_said(said, SLOWER_STEPS),
        (false, true) => step_said(said, FASTER_STEPS),
        _ => return None,
    };
    let ends = if said.any(&["in", "intro", "entrance"]) && !said.has("out") {
        Ends::In
    } else if said.any(&["out", "outro", "exit"]) && !said.has("in") {
        Ends::Out
    } else {
        Ends::Both
    };
    Some(Chore::MotionPace { percent, ends })
}

/// "Make it longer", "make it 2 seconds shorter", "extend the composition by 5s", "shorten
/// the comp": a length changed by an amount, or by a stated step when none is said. A length
/// to change *to* ("extend it to 5s") is an instruction, read elsewhere.
fn length_by(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    let longer = said.any(&["longer", "lengthen", "extend"]);
    if longer == said.any(&["shorter", "shorten"]) || said.after(&[" to "]).and_then(length).is_some() {
        return None;
    }
    // The amount: after "by", or right before the word ("2 seconds longer").
    let before_word = || {
        let at = text.find("longer").or_else(|| text.find("shorter"))?;
        let words: Vec<&str> = text[..at].split_whitespace().collect();
        (1..=words.len().min(4)).rev().find_map(|count| length(&words[words.len() - count..].join(" ")))
    };
    let by = said.after(&[" by "]).and_then(length).or_else(before_word).unwrap_or_else(|| Time::millis(step_said(said, LENGTH_STEPS)));
    Some(if said.any(COMP_WORDS) { Chore::DurationBy { by, longer } } else { Chore::LengthBy { by, longer } })
}

/// "Move it later", "a bit earlier", "delay it": a start moved with no amount said. With an
/// amount it is an instruction, read elsewhere.
fn shift_by(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    let later = said.has("later") || text == "delay it" || text == "delay them" || text == "delay this";
    if later == said.any(&["earlier", "sooner"]) || first_length(text).is_some() || text.contains(|character: char| character.is_ascii_digit()) {
        return None;
    }
    // "See you later" and "later" on its own say goodbye: a request points at something.
    let asking = said.any(&["move", "make", "push", "start", "starts", "shift", "delay", "put", "bring", "it", "them", "this", "bit", "little", "much"]);
    asking.then(|| Chore::ShiftBy { by: Time::millis(step_said(said, SHIFT_STEPS)), later })
}

/// The formats an export names: "as mp4", "a gif", "png sequence", "prores with transparency".
fn formats_said(said: &Said) -> Vec<Format> {
    let text = said.text.as_str();
    let mut found = Vec::new();
    let transparent = said.any(&["4444", "transparent", "transparency", "alpha"]);
    for (at, word) in said.words.iter().enumerate() {
        let next = said.words.get(at + 1).map(String::as_str);
        let format = match word.as_str() {
            "mp4" | "h264" => Format::Mp4,
            "h265" | "hevc" => Format::Hevc,
            "mov" | "prores" | "quicktime" if transparent => Format::MovWithAlpha,
            "mov" | "prores" | "quicktime" => Format::Mov,
            "webm" => Format::WebM,
            "gif" | "gifs" => Format::Gif,
            "pngs" => Format::PngSequence,
            "png" | "image" | "frame" if next == Some("sequence") => Format::PngSequence,
            "dotlottie" => Format::DotLottie,
            "lottie" | "json" => Format::Lottie,
            "svg" => Format::Svg,
            _ => continue,
        };
        if !found.contains(&format) {
            found.push(format);
        }
    }
    // "H.264" and "H.265" are written with a stop, which splits them into two words.
    for (written, format) in [("h.264", Format::Mp4), ("h.265", Format::Hevc)] {
        if text.contains(written) && !found.contains(&format) {
            found.push(format);
        }
    }
    found
}

/// "Export as mp4", "export now", "export the work area as a gif", "export all compositions",
/// "export a vertical version", "save this frame as a PNG", "show it in the Finder". Plain
/// "export" says too little to start one and opens the Export dialog, as the command does.
fn export(said: &Said) -> Option<Chore> {
    let text = said.text.as_str();
    if said.any(&["reveal", "show", "open", "find"]) && said.any(&["finder", "folder"]) && said.any(&["export", "exports", "exported", "file", "it", "that", "movie"]) {
        return Some(Chore::RevealExport);
    }
    let exporting = said.any(&["export", "render"]) && !said.any(&["play", "queue", "settings"]);
    // One frame as a picture: "export a frame", "save this frame as a PNG".
    // It is always the frame under the playhead, so a frame named by its number is left alone.
    let a_frame = said.any(&["frame", "still"]) && !said.any(&["rate", "freeze", "sequence", "frames", "hold"]) && !text.contains(|character: char| character.is_ascii_digit());
    if a_frame && (exporting || said.any(&["save", "grab"])) {
        return Some(Chore::ExportFrame);
    }
    if !exporting {
        return None;
    }
    let formats = formats_said(said);
    // The ends of the message are trimmed of "now" as politeness, so it is looked for as typed.
    let typed = format!(" {} ", said.typed.to_lowercase());
    let now = [" now", " immediately", " straight away", " right away"].iter().any(|word| typed.contains(word));
    // "All" means every composition; all of something else isn't an export this reads.
    let all = said.any(&["all", "every", "everything"]) && !said.any(&["layer", "layers", "clip", "clips", "frames"]);
    let span = if text.contains("work area") {
        Some(Span::WorkArea)
    } else if said.any(&["whole", "entire", "full"]) {
        Some(Span::Whole)
    } else {
        None
    };
    let size = if said.any(&["version", "copy"]) { crate::chat::wanted_size(said.typed) } else { None };
    if formats.is_empty() && !now && !all && span.is_none() && size.is_none() {
        return None;
    }
    Some(Chore::Export(Export { formats, all, span, size }))
}

/// "Send this frame to Edit", "open this frame in Develop", "put the frame in Omina Edit": the
/// frame under the playhead, handed to another of Iris's apps.
fn send_frame(said: &Said) -> Option<Chore> {
    let sending = said.any(&["send", "open", "hand", "take", "put", "move", "give"]);
    let a_frame = said.any(&["frame", "still"]) && !said.any(&["rate", "frames", "sequence", "freeze"]);
    if !sending || !a_frame {
        return None;
    }
    let to = if said.any(&["edit", "canvas"]) {
        iris::App::Canvas
    } else if said.has("develop") {
        iris::App::Develop
    } else if said.any(&["motion", "studio"]) {
        iris::App::Studio
    } else {
        return None;
    };
    Some(Chore::SendFrame(to))
}

/// Reads `message` as one of Motion's chores. `None` when it isn't one: a question, or words
/// no rule here is certain of.
pub fn read(message: &str) -> Option<Request> {
    let said = Said::new(message.trim());
    if said.words.is_empty() {
        return None;
    }
    // A marker's name and the text of a new layer are kept as typed and never read as requests
    // of their own, so those two are read first, from what comes before the name or text.
    let asked = Said::new(request_part(message.trim()));
    let chore = if intent::asks(message) {
        preflight(&said)?
    } else {
        playback(&said)
            .or_else(|| go(&said))
            .or_else(|| markers(&said, &asked))
            .or_else(|| making(&said, &asked))
            .or_else(|| preflight(&said))
            .or_else(|| send_frame(&said))
            .or_else(|| export(&said))
            .or_else(|| work_area(&said))
            .or_else(|| composition(&said))
            .or_else(|| arranging(&said))
            .or_else(|| sound(&said))
            .or_else(|| motion_pace(&said))
            .or_else(|| length_by(&said))
            .or_else(|| shift_by(&said))
            .or_else(|| speed(&said))?
    };
    Some(Request { chore, target: crate::instructions::target_of(message) })
}

/// The words chores are written with, for putting a mistyped one right (see
/// `iris::spelling`).
pub fn vocabulary() -> Vec<String> {
    const WORDS: &[&str] = &[
        "playhead", "pause", "rewind", "forward", "backward", "marker", "markers", "keyframe", "composition", "duration", "length", "frame", "rate", "width", "height", "size", "resolution", "work", "area", "stagger", "cascade", "sequence", "another", "gaps", "close",
        "order", "reverse", "backwards", "unmute", "volume", "louder", "quieter", "audio", "sound", "music", "voice", "solid", "background", "rectangle", "square", "circle", "ellipse", "triangle", "polygon", "adjustment", "saying", "headline", "title",
        "speed", "faster", "slower", "twice", "double", "freeze", "ready", "export", "preflight", "called", "shorten", "extend", "seconds", "frames",
    ];
    WORDS.iter().map(|word| (*word).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chore(message: &str) -> Chore {
        read(message).unwrap_or_else(|| panic!("“{message}” should read as a chore")).chore
    }

    fn secs(seconds: f64) -> Time {
        Time::millis((seconds * 1000.0).round() as i64)
    }

    fn frames(frames: i64) -> Time {
        Time::Span { millis: 0, frames }
    }

    #[test]
    fn the_playhead_goes_where_it_is_told() {
        assert_eq!(chore("go to 5s"), Chore::Go(Jump::To(secs(5.0))));
        assert_eq!(chore("Jump to frame 120."), Chore::Go(Jump::To(frames(120))));
        assert_eq!(chore("go to the start"), Chore::Go(Jump::To(Time::START)));
        assert_eq!(chore("please go to the end"), Chore::Go(Jump::To(Time::End)));
        assert_eq!(chore("move the playhead to 1:05"), Chore::Go(Jump::To(secs(65.0))));
        assert_eq!(chore("go to the next marker"), Chore::Go(Jump::Marker { next: true }));
        assert_eq!(chore("previous marker"), Chore::Go(Jump::Marker { next: false }));
        assert_eq!(chore("go to the previous keyframe"), Chore::Go(Jump::Keyframe { next: false }));
        assert_eq!(chore("next frame"), Chore::Go(Jump::By { by: frames(1), forward: true }));
        assert_eq!(chore("back 10 frames"), Chore::Go(Jump::By { by: frames(10), forward: false }));
        assert_eq!(chore("skip ahead 2 seconds"), Chore::Go(Jump::By { by: secs(2.0), forward: true }));
        assert_eq!(chore("go to the Chorus marker"), Chore::Go(Jump::MarkerCalled("Chorus".to_owned())));
        assert_eq!(chore("jump to marker “Drop 2”"), Chore::Go(Jump::MarkerCalled("Drop 2".to_owned())));
        assert_eq!(chore("rewind"), Chore::Go(Jump::To(Time::START)));
    }

    #[test]
    fn playing_and_pausing_read_only_as_whole_messages() {
        assert_eq!(chore("play"), Chore::Play { from: None });
        assert_eq!(chore("Play it!"), Chore::Play { from: None });
        assert_eq!(chore("can you play the video"), Chore::Play { from: None });
        assert_eq!(chore("play from the start"), Chore::Play { from: Some(Time::START) });
        assert_eq!(chore("play it from 2s"), Chore::Play { from: Some(secs(2.0)) });
        assert_eq!(chore("pause"), Chore::Pause);
        assert_eq!(chore("stop playing"), Chore::Pause);
        assert_eq!(read("play or pause"), None, "that is a command's name");
        assert_eq!(read("stop it pulsing"), None);
    }

    #[test]
    fn the_compositions_settings_read() {
        assert_eq!(chore("make the composition 10 seconds long"), Chore::Duration(secs(10.0)));
        assert_eq!(chore("set the comp's duration to 300 frames"), Chore::Duration(frames(300)));
        assert_eq!(chore("extend the composition to 15s"), Chore::Duration(secs(15.0)));
        assert_eq!(chore("make the composition 2 hours long"), Chore::Duration(secs(7200.0)), "hours aren't taken for seconds");
        assert_eq!(chore("set the frame rate to 30"), Chore::FrameRate(30_000));
        assert_eq!(chore("make it 23.976 fps"), Chore::FrameRate(23_976));
        assert_eq!(chore("60fps"), Chore::FrameRate(60_000));
        assert_eq!(chore("set the size to 1080x1920"), Chore::Size { width: Some(1080), height: Some(1920) });
        assert_eq!(chore("set the composition width to 1280"), Chore::Size { width: Some(1280), height: None });
        assert_eq!(read("set the frame rate to 9000"), None, "not a frame rate anyone means");
    }

    #[test]
    fn the_work_area_reads() {
        assert_eq!(chore("set the work area from 2s to 6s"), Chore::WorkArea { from: Some(secs(2.0)), to: Some(secs(6.0)) });
        assert_eq!(chore("work area 1s to the end"), Chore::WorkArea { from: Some(secs(1.0)), to: Some(Time::End) });
        assert_eq!(chore("start the work area at the playhead"), Chore::WorkArea { from: Some(Time::Playhead), to: None });
        assert_eq!(chore("end the work area here"), Chore::WorkArea { from: None, to: Some(Time::Playhead) });
        assert_eq!(chore("reset the work area"), Chore::WorkAreaWhole);
        assert_eq!(chore("set the work area to the selected layers"), Chore::WorkAreaAround);
        assert_eq!(chore("trim the composition to the work area"), Chore::TrimToWorkArea);
    }

    #[test]
    fn arranging_layers_in_time_reads() {
        assert_eq!(chore("stagger the selected layers by 5 frames"), Chore::Stagger(Some(frames(5))));
        assert_eq!(chore("cascade them 0.2s apart"), Chore::Stagger(Some(secs(0.2))));
        assert_eq!(chore("stagger them"), Chore::Stagger(None));
        assert_eq!(chore("sequence them one after another"), Chore::Sequence);
        assert_eq!(chore("put these back to back"), Chore::Sequence);
        assert_eq!(chore("line them all up at the playhead"), Chore::LineUp(Time::Playhead));
        assert_eq!(chore("line the layers up at 2s"), Chore::LineUp(secs(2.0)));
        assert_eq!(chore("close the gaps"), Chore::CloseGaps);
        assert_eq!(chore("make them all the same length"), Chore::SameLength(Pick::First));
        assert_eq!(chore("make them the same length as the longest"), Chore::SameLength(Pick::Longest));
        assert_eq!(chore("reverse their order"), Chore::ReverseOrder);
    }

    #[test]
    fn sound_reads() {
        assert_eq!(chore("mute it"), Chore::Mute(true));
        assert_eq!(chore("unmute the music"), Chore::Mute(false));
        assert_eq!(chore("set the volume to 50%"), Chore::Volume(Level::Percent(50)));
        assert_eq!(chore("volume to -6 dB"), Chore::Volume(Level::Decibels(-60)));
        assert_eq!(chore("make it a bit louder"), Chore::Volume(Level::By(15)));
        assert_eq!(chore("turn the music down"), Chore::Volume(Level::By(-30)));
        assert_eq!(chore("fade the audio in over 1s"), Chore::SoundFade { ends: Ends::In, length: Some(secs(1.0)) });
        assert_eq!(chore("fade the music out over 2 seconds"), Chore::SoundFade { ends: Ends::Out, length: Some(secs(2.0)) });
        assert_eq!(chore("fade the sound in and out"), Chore::SoundFade { ends: Ends::Both, length: None });
        assert_eq!(chore("remove the audio fade out"), Chore::SoundFade { ends: Ends::Out, length: Some(secs(0.0)) });
        assert_eq!(chore("duck the music under the voice"), Chore::Duck(true));
        assert_eq!(chore("stop ducking the music"), Chore::Duck(false));
    }

    #[test]
    fn markers_read_with_their_labels_as_typed() {
        assert_eq!(chore("add a marker here"), Chore::Marker { at: Time::Playhead, label: None });
        assert_eq!(chore("add a marker at 3s"), Chore::Marker { at: secs(3.0), label: None });
        assert_eq!(chore("add a marker called Chorus"), Chore::Marker { at: Time::Playhead, label: Some("Chorus".to_owned()) });
        assert_eq!(chore("drop a marker called Big Drop at 1:05"), Chore::Marker { at: secs(65.0), label: Some("Big Drop".to_owned()) });
        assert_eq!(chore("add a marker “Look At Me” at frame 12"), Chore::Marker { at: frames(12), label: Some("Look At Me".to_owned()) });
        assert_eq!(chore("mark here"), Chore::Marker { at: Time::Playhead, label: None });
        assert_eq!(chore("add a marker called Clear Sky"), Chore::Marker { at: Time::Playhead, label: Some("Clear Sky".to_owned()) }, "a label is not a request");
        assert_eq!(chore("add a marker called Reset The Work Area"), Chore::Marker { at: Time::Playhead, label: Some("Reset The Work Area".to_owned()) });
        assert_eq!(chore("rename this marker to Verse"), Chore::Marker { at: Time::Playhead, label: Some("Verse".to_owned()) });
        assert_eq!(chore("remove the marker at 3s"), Chore::RemoveMarker(secs(3.0)));
        assert_eq!(chore("delete this marker"), Chore::RemoveMarker(Time::Playhead));
        assert_eq!(chore("clear all markers"), Chore::ClearMarkers);
    }

    #[test]
    fn making_layers_reads() {
        let made = |what: Made| Chore::Make { what, color: None };
        assert_eq!(chore("add a text layer saying “Hello”"), made(Made::Text("Hello".to_owned())));
        assert_eq!(chore("add a title that says Big News Today"), made(Made::Text("Big News Today".to_owned())));
        assert_eq!(chore("write “Sale, then more”"), made(Made::Text("Sale, then more".to_owned())));
        assert_eq!(chore("add a title"), made(Made::Text(String::new())), "what it says is asked for");
        // The words to show are never read as a request of their own.
        assert_eq!(chore("add a title saying mute the music"), made(Made::Text("mute the music".to_owned())));
        assert_eq!(chore("add text “Play it twice as fast”"), made(Made::Text("Play it twice as fast".to_owned())));
        assert_eq!(chore("add a solid"), made(Made::Solid));
        assert_eq!(chore("add a dark red solid"), Chore::Make { what: Made::Solid, color: Some(([137, 34, 32], "dark red".to_owned())) });
        assert_eq!(chore("make a blue circle"), Chore::Make { what: Made::Shape { kind: ShapeKind::Ellipse, even: true }, color: Some(([30, 136, 229], "blue".to_owned())) });
        assert_eq!(chore("add a shape"), made(Made::Shape { kind: ShapeKind::Rectangle, even: true }));
        assert_eq!(chore("new null object"), made(Made::Null));
        assert_eq!(chore("add an adjustment layer"), made(Made::Adjustment));
        assert_eq!(chore("add a white background"), Chore::Make { what: Made::Background, color: Some(([255, 255, 255], "white".to_owned())) });
    }

    #[test]
    fn speed_reads() {
        assert_eq!(chore("make it twice as fast"), Chore::Speed(Pace::Times { times: 2, over: 1 }));
        assert_eq!(chore("three times as fast"), Chore::Speed(Pace::Times { times: 3, over: 1 }));
        assert_eq!(chore("half speed"), Chore::Speed(Pace::Times { times: 1, over: 2 }));
        assert_eq!(chore("twice as slow"), Chore::Speed(Pace::Times { times: 1, over: 2 }));
        assert_eq!(chore("50% speed"), Chore::Speed(Pace::Percent(50)));
        assert_eq!(chore("set the speed to 150%"), Chore::Speed(Pace::Percent(150)));
        assert_eq!(chore("make it 20% faster"), Chore::Speed(Pace::Times { times: 120, over: 100 }));
        assert_eq!(chore("play it at 2x speed"), Chore::Speed(Pace::Percent(200)));
        assert_eq!(chore("slow motion"), Chore::Speed(Pace::Percent(50)));
        assert_eq!(chore("back to normal speed"), Chore::Speed(Pace::Normal));
        assert_eq!(chore("reverse it"), Chore::Reverse(true));
        assert_eq!(chore("play it backwards"), Chore::Reverse(true));
        assert_eq!(chore("play it forwards again"), Chore::Reverse(false));
        assert_eq!(chore("freeze frame here"), Chore::Freeze);
    }

    #[test]
    fn requests_with_no_amount_get_a_stated_step() {
        assert_eq!(chore("slow it down"), Chore::Speed(Pace::Times { times: 100, over: 125 }));
        assert_eq!(chore("speed it up a bit"), Chore::Speed(Pace::Times { times: 110, over: 100 }));
        assert_eq!(chore("make it much faster"), Chore::Speed(Pace::Times { times: 150, over: 100 }));
        assert_eq!(chore("make the composition longer"), Chore::DurationBy { by: secs(1.0), longer: true });
        assert_eq!(chore("shorten the comp by 2 seconds"), Chore::DurationBy { by: secs(2.0), longer: false });
        assert_eq!(chore("make it longer"), Chore::LengthBy { by: secs(1.0), longer: true });
        assert_eq!(chore("make it last a bit longer"), Chore::LengthBy { by: secs(0.5), longer: true });
        assert_eq!(chore("make it 10 frames shorter"), Chore::LengthBy { by: frames(10), longer: false });
        assert_eq!(chore("extend it by half a second"), Chore::LengthBy { by: secs(0.5), longer: true });
        assert_eq!(chore("move it later"), Chore::ShiftBy { by: secs(0.5), later: true });
        assert_eq!(chore("a bit earlier"), Chore::ShiftBy { by: secs(0.2), later: false });
        assert_eq!(chore("make the fade slower"), Chore::MotionPace { percent: 150, ends: Ends::Both });
        assert_eq!(chore("fade it in faster"), Chore::MotionPace { percent: 67, ends: Ends::In });
        assert_eq!(chore("make the outro a little longer"), Chore::MotionPace { percent: 125, ends: Ends::Out });
        // With an amount, or a length to change to, these are instructions read elsewhere.
        for message in ["extend it to 5s", "delay it by half a second", "move it 2 seconds later", "fade in over 2 seconds", "see you later", "later", "animate this, faster"] {
            assert_eq!(read(message), None, "{message}");
        }
    }

    #[test]
    fn a_layers_start_and_end_can_be_gone_to() {
        assert_eq!(chore("go to the start of the title"), Chore::Go(Jump::LayerEdge { start: true }));
        assert_eq!(chore("jump to the end of “Big Sale”"), Chore::Go(Jump::LayerEdge { start: false }));
        assert_eq!(chore("go to where it ends"), Chore::Go(Jump::LayerEdge { start: false }));
        assert_eq!(chore("go to the start of the work area"), Chore::Go(Jump::WorkAreaEdge { start: true }));
        assert_eq!(chore("go to the end of the composition"), Chore::Go(Jump::To(Time::End)));
        assert_eq!(read("jump to the end of “Big Sale”").map(|request| request.target), Some(Target::Named("Big Sale".to_owned())));
    }

    #[test]
    fn a_frame_is_sent_to_another_app_by_name() {
        assert_eq!(chore("send this frame to Edit"), Chore::SendFrame(iris::App::Canvas));
        assert_eq!(chore("open this frame in Omina Develop"), Chore::SendFrame(iris::App::Develop));
        assert_eq!(chore("put the frame in Develop please"), Chore::SendFrame(iris::App::Develop));
        assert_eq!(chore("send the still to Motion"), Chore::SendFrame(iris::App::Studio));
        for message in ["send this frame", "open the frame rate settings", "edit this frame", "save this frame as a PNG"] {
            assert!(!matches!(read(message).map(|request| request.chore), Some(Chore::SendFrame(_))), "{message}");
        }
    }

    #[test]
    fn exports_that_say_enough_are_read_and_plain_export_is_not() {
        let exported = |formats: &[Format], all: bool, span: Option<Span>| Chore::Export(Export { formats: formats.to_vec(), all, span, size: None });
        assert_eq!(chore("export as mp4"), exported(&[Format::Mp4], false, None));
        assert_eq!(chore("export a gif of the work area"), exported(&[Format::Gif], false, Some(Span::WorkArea)));
        assert_eq!(chore("export it as mp4 and gif"), exported(&[Format::Mp4, Format::Gif], false, None));
        assert_eq!(chore("export as a png sequence"), exported(&[Format::PngSequence], false, None));
        assert_eq!(chore("export as prores with transparency"), exported(&[Format::MovWithAlpha], false, None));
        assert_eq!(chore("export now"), exported(&[], false, None));
        assert_eq!(chore("render the whole composition as H.265"), exported(&[Format::Hevc], false, Some(Span::Whole)));
        assert_eq!(chore("export all compositions"), exported(&[], true, None));
        assert_eq!(chore("export a vertical version"), Chore::Export(Export { formats: Vec::new(), all: false, span: None, size: Some((1080, 1920, "9:16".to_owned())) }));
        assert_eq!(chore("export a frame"), Chore::ExportFrame);
        assert_eq!(chore("save this frame as a PNG"), Chore::ExportFrame);
        assert_eq!(chore("show it in the Finder"), Chore::RevealExport);
        // Too little said to start one: the Export dialog opens, as the command does.
        for message in ["export", "export this", "open the export queue", "export settings", "render and play the work area", "how do I export a gif?", "export as png", "export all layers", "export frame 12"] {
            assert_eq!(read(message), None, "{message}");
        }
    }

    #[test]
    fn the_check_before_exporting_reads_as_a_question_too() {
        for message in ["am I ready to export?", "Is this ready to render?", "preflight", "check everything before I export", "ready to export?"] {
            assert_eq!(chore(message), Chore::Preflight, "{message}");
        }
    }

    #[test]
    fn the_layers_meant_come_with_the_chore() {
        assert_eq!(read("mute all audio layers").map(|request| request.target), Some(Target::Kind(intent::Kind::Sound)));
        assert_eq!(read("set the volume of the voice layer to 50%").map(|request| request.target), Some(Target::Named("voice".to_owned())));
        assert_eq!(read("make “Big Sale” twice as fast").map(|request| request.target), Some(Target::Named("Big Sale".to_owned())));
        assert_eq!(read("reverse it").map(|request| request.target), Some(Target::Selection));
        assert_eq!(read("mute").map(|request| request.target), Some(Target::Unnamed));
    }

    #[test]
    fn messages_about_other_things_are_left_alone() {
        for message in [
            // Instructions the shared reader carries out.
            "start at 5s and last 2 seconds",
            "fade in over a second",
            "slide the title in from the left",
            "cut it at the playhead",
            "make it 4s long",
            "make it pulse",
            "50% opacity",
            "fade it out quickly",
            "fade in over half a second, fast",
            "50% opacity, quick",
            "turn it upside down",
            "move it backwards",
            "rise up line by line",
            "sequence settings",
            "move it 20 px left",
            "delay it by half a second",
            // Housekeeping.
            "make the title red",
            "make the text white",
            "duplicate it",
            "put it in the top right corner",
            "make a copy of it",
            "hide the logo",
            // Another size, animating, and questions about the project.
            "make a square version",
            "convert this comp to vertical",
            "change the resolution to 1000 by 1500 please",
            "animate this, fast and punchy",
            "how long is the video?",
            "what's the frame rate?",
            "fps?",
            "where is the playhead",
            "what is the work area",
            "is anything muted",
            "which layers have sound",
            "are there any markers?",
            "how do I add a marker?",
            "how many frames is it",
            "add a fade to the title",
            "hello",
            "",
        ] {
            assert_eq!(read(message), None, "{message}");
        }
    }
}
