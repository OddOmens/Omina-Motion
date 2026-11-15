//! Understanding instructions about timing and motion, typed in everyday words: "make this start
//! at 5s and last 2 seconds, and fade it in", "cut it at the playhead", "slide the title in from
//! the left over a second", "fade out at 8s", "make it pulse", "50% opacity".
//!
//! This is shared by every app. It turns a message into a `Request` (which layers, and a list
//! of `Step`s) without knowing anything about the app's project, so each app carries the steps
//! out its own way: Studio on its timeline, and other apps where the step makes sense to them.
//! Nothing here is AI: it reads the words with fixed rules, and a message it can't read with
//! confidence gives `None`, so the app can answer some other way.

/// A length or a moment, as it was said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Time {
    /// So many milliseconds plus so many frames ("1.5s" is 1500 ms; "frame 12" is 12 frames;
    /// a timecode "0:00:05:12" is 5000 ms and 12 frames). The app adds them at its frame rate.
    Span { millis: i64, frames: i64 },
    /// Where the playhead is ("at the playhead", "here", "now").
    Playhead,
    /// The end of the composition or design ("at the end").
    End,
}

impl Time {
    /// A length in milliseconds.
    pub const fn millis(millis: i64) -> Self {
        Self::Span { millis, frames: 0 }
    }

    /// The moment the composition starts.
    pub const START: Self = Self::millis(0);
}

/// A side of the frame: where a slide comes in from, or goes out to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// How a layer comes in or goes out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Whatever suits the layer best ("animate it in").
    Suited,
    Fade,
    /// Slides in from (or out to) a side.
    Slide(Side),
    Pop,
    Zoom,
    Spin,
    Blur,
    Bounce,
    /// Typed on (or erased).
    Typewriter,
    /// Text coming in one word, letter or line at a time.
    ByWord,
    ByLetter,
    ByLine,
}

/// What a layer does between coming in and going out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    Pulse,
    Float,
    Swing,
    Blink,
    Turn,
    Wiggle,
}

/// Which end of a layer a motion belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    In,
    Out,
}

/// One thing asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Move the layer so it starts at this moment, keeping its length.
    StartAt(Time),
    /// Trim its start to this moment, keeping its end ("trim the start to 2s").
    TrimStart(Time),
    /// End it at this moment (trimming or extending its end).
    EndAt(Time),
    /// Make it last this long from its start.
    Last(Time),
    /// Move it later (or, with `later` false, earlier) by this much.
    Shift { by: Time, later: bool },
    /// Cut it in two at this moment.
    CutAt(Time),
    /// Give it a way in or out, taking `millis` (or the app's usual length when `None`).
    Animate { phase: Phase, motion: Motion, millis: Option<i64> },
    /// Take away its way in, its way out, or (`None`) both.
    RemoveMotion(Option<Phase>),
    /// Give it a loop between coming in and going out.
    Loop(Loop),
    RemoveLoop,
    /// Set its opacity, in percent.
    Opacity(i64),
    /// Set its size, in percent of its full size.
    Scale(i64),
    /// Turn it by so many degrees (clockwise).
    Rotate(i64),
    /// Move it by so many pixels (right and down are positive).
    Nudge { x: i64, y: i64 },
}

/// A kind of layer, for "all text layers" and "every image".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// Still pictures: images and photos.
    Picture,
    Shape,
    Video,
    Sound,
}

impl Kind {
    /// The kind as it reads in a sentence about several: "text layers", "pictures".
    pub fn plural(self) -> &'static str {
        match self {
            Self::Text => "text layers",
            Self::Picture => "pictures",
            Self::Shape => "shapes",
            Self::Video => "video clips",
            Self::Sound => "sound clips",
        }
    }
}

/// Which layers a request is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// None named: the selection, or the one layer the message mentions by name.
    Unnamed,
    /// "This", "it", "the selected layer".
    Selection,
    /// "Everything", "all layers".
    Everything,
    /// A layer named in the message: in quotes, or as "the title layer" or "layer called Title".
    Named(String),
    /// "Layer 2", counting from the top as the timeline numbers them.
    Number(usize),
    /// Every layer of one kind: "all text layers", "every image", "the shapes".
    Kind(Kind),
}

/// A message read as instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub target: Target,
    pub steps: Vec<Step>,
}

/// The usual lengths for "quickly" and "slowly", in milliseconds.
const QUICK_MILLIS: i64 = 300;
const SLOW_MILLIS: i64 = 1200;

/// Reads `message` as instructions. `None` when it isn't one (a question, a greeting) or
/// nothing in it could be read with confidence.
pub fn parse(message: &str) -> Option<Request> {
    let tokens = tokenize(message);
    if tokens.is_empty() || is_question(message, &tokens) || is_about_the_playhead(&tokens) {
        return None;
    }
    let mut reader = Reader { tokens: &tokens, used: vec![false; tokens.len()], steps: Vec::new() };
    reader.read_all();
    let mut steps = reader.steps;
    // "Animate the title in word by word": the named motion wins over the suited one.
    let named: Vec<Phase> = steps.iter().filter_map(|step| match step {
        Step::Animate { phase, motion, .. } if *motion != Motion::Suited => Some(*phase),
        _ => None,
    }).collect();
    steps.retain(|step| !matches!(step, Step::Animate { phase, motion: Motion::Suited, .. } if named.contains(phase)));
    if steps.is_empty() {
        return None;
    }
    Some(Request { target: target(message, &tokens), steps })
}

// Words ----------------------------------------------------------------------------------------

/// A message as lower-case words and numbers. "5s" becomes "5" "s", "1.5sec" "1.5" "sec",
/// "50%" "50" "%", "15°" "15" "°"; a clock time ("1:05", "0:00:05:12") stays one word.
pub(crate) fn tokenize(message: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let lower = message.to_lowercase().replace(['’', '‘'], "'");
    let mut current = String::new();
    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
        }
    };
    let chars: Vec<char> = lower.chars().collect();
    for (index, &character) in chars.iter().enumerate() {
        let next = chars.get(index + 1).copied();
        let in_number = current.chars().next().is_some_and(|first| first.is_ascii_digit());
        match character {
            c if c.is_ascii_digit() => {
                // "5s": a number after letters starts a new word, and so do letters after a number.
                if !in_number && !current.is_empty() {
                    flush(&mut current, &mut tokens);
                }
                current.push(c);
            }
            '.' | ':' if in_number && next.is_some_and(|n| n.is_ascii_digit()) => current.push(character),
            c if c.is_alphabetic() || c == '\'' => {
                if in_number {
                    flush(&mut current, &mut tokens);
                }
                current.push(c);
            }
            '%' | '°' => {
                flush(&mut current, &mut tokens);
                tokens.push(character.to_string());
            }
            '-' if next.is_some_and(|n| n.is_ascii_digit()) && current.is_empty() => current.push('-'),
            _ => flush(&mut current, &mut tokens),
        }
    }
    flush(&mut current, &mut tokens);
    tokens
}

/// Whether the message asks something rather than asking for something: "how do I fade in?",
/// "what starts at 5s". "Can you…", "could you…" and "please" are requests.
pub(crate) fn is_question(message: &str, tokens: &[String]) -> bool {
    let first = tokens.first().map(String::as_str).unwrap_or("");
    let polite = tokens.windows(2).any(|pair| matches!((pair[0].as_str(), pair[1].as_str()), ("can" | "could" | "would" | "will", "you"))) || tokens.iter().any(|token| token == "please");
    if polite {
        return false;
    }
    let asks = matches!(first, "how" | "what" | "why" | "when" | "where" | "which" | "who" | "is" | "are" | "does" | "do" | "did" | "should" | "can" | "could");
    asks || message.trim_end().ends_with('?')
}

/// Whether `message` asks something rather than asking for something, for the other readers
/// that must leave questions alone.
pub fn asks(message: &str) -> bool {
    is_question(message, &tokenize(message))
}

/// Which layers `message` is about, for an app's own readers that share this way of naming
/// them.
pub fn target_of(message: &str) -> Target {
    target(message, &tokenize(message))
}

/// Whether the message is about moving the playhead itself ("move the playhead to 5s"), which
/// isn't a change to any layer.
fn is_about_the_playhead(tokens: &[String]) -> bool {
    tokens.windows(3).any(|words| matches!(words[0].as_str(), "move" | "go" | "jump" | "set" | "put" | "take") && (words[1] == "playhead" || (words[1] == "the" && words[2] == "playhead")))
}

/// A number written in figures or words, with how many words it took: "5", "2.5", "two",
/// "twenty five", "a" (in "a second"), "half" (0.5), "one and a half" (1.5).
pub(crate) fn number_at(tokens: &[String], at: usize) -> Option<(f64, usize)> {
    let word = tokens.get(at)?.as_str();
    let (mut value, mut used) = if let Ok(value) = word.parse::<f64>() {
        (value, 1)
    } else if let Some(value) = small_number(word) {
        // "Twenty five".
        match tokens.get(at + 1).map(String::as_str).and_then(small_number) {
            Some(ones) if (20.0..=90.0).contains(&value) && value % 10.0 == 0.0 && ones < 10.0 => (value + ones, 2),
            _ => (value, 1),
        }
    } else {
        match word {
            "a" | "an" => (1.0, 1),
            "half" => return Some((0.5, 1)),
            "quarter" => return Some((0.25, 1)),
            _ => return None,
        }
    };
    if !value.is_finite() {
        return None;
    }
    // "One and a half", "2 and a half".
    if tokens.get(at + used).is_some_and(|word| word == "and") && tokens.get(at + used + 1).is_some_and(|word| word == "a") && tokens.get(at + used + 2).is_some_and(|word| word == "half") {
        value += 0.5;
        used += 3;
    }
    Some((value, used))
}

fn small_number(word: &str) -> Option<f64> {
    const WORDS: [&str; 21] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty"];
    if let Some(index) = WORDS.iter().position(|known| *known == word) {
        return Some(index as f64);
    }
    Some(match word {
        "thirty" => 30.0,
        "forty" => 40.0,
        "fifty" => 50.0,
        "sixty" => 60.0,
        "ninety" => 90.0,
        "hundred" => 100.0,
        _ => return None,
    })
}

/// A moment or length starting at `at`, with how many words it took: "5s", "2 seconds",
/// "half a second", "500ms", "1.5 minutes", "frame 120", "120 frames", "1:05", "0:00:05:12",
/// "the end", "the start", "the playhead". A bare number counts as seconds.
fn time_at(tokens: &[String], at: usize) -> Option<(Time, usize)> {
    let word = |offset: usize| tokens.get(at + offset).map(String::as_str);
    let skip = usize::from(word(0) == Some("the"));
    match word(skip)? {
        "end" | "ending" => return Some((Time::End, skip + 1)),
        "start" | "beginning" => return Some((Time::START, skip + 1)),
        "playhead" | "cursor" => return Some((Time::Playhead, skip + 1)),
        "here" | "now" if skip == 0 => return Some((Time::Playhead, 1)),
        "frame" => {
            let (frames, used) = number_at(tokens, at + skip + 1)?;
            return Some((Time::Span { millis: 0, frames: frames.round() as i64 }, skip + 1 + used));
        }
        _ => {}
    }
    if skip == 1 {
        return None;
    }
    // A clock time: m:ss, h:mm:ss or h:mm:ss:ff.
    if let Some(clock) = word(0).filter(|word| word.contains(':')) {
        let parts: Vec<f64> = clock.split(':').map(str::parse).collect::<Result<_, _>>().ok()?;
        let (hours, minutes, seconds, frames) = match parts.as_slice() {
            [minutes, seconds] => (0.0, *minutes, *seconds, 0.0),
            [hours, minutes, seconds] => (*hours, *minutes, *seconds, 0.0),
            [hours, minutes, seconds, frames] => (*hours, *minutes, *seconds, *frames),
            _ => return None,
        };
        let millis = ((hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0).round() as i64;
        return Some((Time::Span { millis, frames: frames.round() as i64 }, 1));
    }
    // "Half a second".
    if word(0) == Some("half") && word(1) == Some("a") && word(2).is_some_and(|unit| unit_millis(unit) == Some(1000.0)) {
        return Some((Time::millis(500), 3));
    }
    let (value, used) = number_at(tokens, at)?;
    let unit = word(used);
    if let Some(per) = unit.and_then(unit_millis) {
        // "A second and a half".
        let mut used = used + 1;
        let mut value = value;
        if word(used) == Some("and") && word(used + 1) == Some("a") && word(used + 2) == Some("half") {
            value += 0.5;
            used += 3;
        }
        return Some((Time::millis((value * per).round() as i64), used));
    }
    if matches!(unit, Some("frame" | "frames" | "f" | "fr")) {
        return Some((Time::Span { millis: 0, frames: value.round() as i64 }, used + 1));
    }
    // "A" and "half" alone aren't times; a bare figure is seconds.
    let figure = word(0).is_some_and(|word| word.parse::<f64>().is_ok()) || small_number(word(0)?).is_some();
    figure.then(|| (Time::millis((value * 1000.0).round() as i64), used))
}

/// Milliseconds in one of a time unit.
fn unit_millis(unit: &str) -> Option<f64> {
    Some(match unit {
        "ms" | "millisecond" | "milliseconds" | "msec" => 1.0,
        "s" | "sec" | "secs" | "second" | "seconds" | "seg" => 1000.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60_000.0,
        "hr" | "hrs" | "hour" | "hours" => 3_600_000.0,
        _ => return None,
    })
}

// Reading steps ----------------------------------------------------------------------------------

/// Reads steps out of the words, marking the words each one used so no word counts twice.
struct Reader<'a> {
    tokens: &'a [String],
    used: Vec<bool>,
    steps: Vec<Step>,
}

/// Words that refer to the layer between a verb and what follows ("fade *it* in").
const PRONOUNS: &[&str] = &["it", "this", "that", "them", "these", "those", "the", "layer", "layers", "one", "selected", "selection", "everything", "all", "clip", "text", "title", "me"];

impl Reader<'_> {
    fn word(&self, at: usize) -> Option<&str> {
        self.tokens.get(at).map(String::as_str)
    }

    fn is(&self, at: usize, words: &[&str]) -> bool {
        self.word(at).is_some_and(|word| words.contains(&word))
    }

    fn mark(&mut self, from: usize, count: usize) {
        for index in from..(from + count).min(self.used.len()) {
            self.used[index] = true;
        }
    }

    /// The first word at or after `at` that isn't a pronoun, within a few words.
    fn after_pronouns(&self, mut at: usize) -> usize {
        let limit = at + 3;
        while at < limit && self.is(at, PRONOUNS) {
            at += 1;
        }
        at
    }

    /// The first word at or after `at` past a layer's name or a pronoun ("fade *the logo* in",
    /// "start *the music* at 3s"): up to four words that aren't numbers or the small words
    /// instructions are built from.
    fn after_name(&self, mut at: usize) -> usize {
        // A name ending in "layer" may hold a joining word of its own: "the *half off* layer".
        if let Some(end) = self.name_before_layer(at) {
            return end;
        }
        let limit = at + 4;
        while at < limit && self.word(at).is_some_and(|word| PRONOUNS.contains(&word) || (word.chars().all(char::is_alphabetic) && !JOINING_WORDS.contains(&word))) {
            at += 1;
        }
        at
    }

    /// Where a layer's name that runs from `at` up to the word "layer" (or "clip") ends, just
    /// past that word, when it is a few plain words: "half off layer", "plug in clip". `None`
    /// when there is no such word close by.
    fn name_before_layer(&self, at: usize) -> Option<usize> {
        let end = (at..(at + 5).min(self.tokens.len())).find(|index| self.is(*index, &["layer", "clip"]))?;
        let plain = (at..end).all(|index| self.word(index).is_some_and(|word| word.chars().all(char::is_alphabetic) && !matches!(word, "at" | "to" | "for" | "by" | "over" | "and" | "then")));
        (end > at && plain).then_some(end + 1)
    }

    /// Whether the word at `at` is part of a layer's name that ends with "layer" just after it
    /// ("the half *off* layer"), rather than saying which way something moves.
    fn inside_layer_name(&self, at: usize) -> bool {
        self.is(at + 1, &["layer", "clip"]) && at > 0 && self.word(at - 1).is_some_and(|word| word.chars().all(char::is_alphabetic) && !matches!(word, "the" | "it" | "this" | "fade" | "slide" | "zoom" | "pop" | "spin" | "blur" | "bounce" | "type" | "animate"))
    }

    fn read_all(&mut self) {
        // Taking things away first, so "remove the fade" doesn't also add one.
        for at in 0..self.tokens.len() {
            self.removal(at);
        }
        for at in 0..self.tokens.len() {
            if self.used[at] {
                continue;
            }
            let _ = self.range(at) || self.trim_start(at) || self.cut(at) || self.shift(at) || self.start(at) || self.end(at) || self.last(at) || self.motion(at) || self.looping(at) || self.look(at);
        }
    }

    /// "Remove the fade", "no animation", "stop it pulsing", "get rid of the intro".
    fn removal(&mut self, at: usize) -> bool {
        let removing = self.is(at, &["remove", "delete", "clear", "no", "without", "drop", "kill", "lose"])
            || (self.is(at, &["stop"]) && !self.is(at + 1, &["at"]))
            || (self.is(at, &["get"]) && self.is(at + 1, &["rid"]))
            || (self.is(at, &["turn", "take"]) && self.is(at + 1, &["off", "away", "out"]))
            || (self.is(at, &["don't", "dont"]) && self.is(at + 1, &["animate", "fade"]));
        if !removing {
            return false;
        }
        // What is taken away, within the next few words.
        for look in at + 1..(at + 6).min(self.tokens.len()) {
            let word = self.word(look).unwrap_or("");
            let phase = || -> Option<Phase> {
                let near = (at..(look + 3).min(self.tokens.len())).filter_map(|index| self.word(index));
                let mut phase = None;
                for word in near {
                    match word {
                        "in" | "intro" | "entrance" | "entry" => phase = Some(Phase::In),
                        "out" | "outro" | "exit" | "away" => phase = Some(Phase::Out),
                        _ => {}
                    }
                }
                phase
            };
            let step = match word {
                "pulse" | "pulsing" | "float" | "floating" | "swing" | "swinging" | "blink" | "blinking" | "wiggle" | "wiggling" | "loop" | "looping" | "shake" | "shaking" | "spinning" => Some(Step::RemoveLoop),
                "intro" | "entrance" | "entry" => Some(Step::RemoveMotion(Some(Phase::In))),
                "outro" | "exit" => Some(Step::RemoveMotion(Some(Phase::Out))),
                "fade" | "fades" | "fading" | "slide" | "pop" | "zoom" | "bounce" | "animation" | "animations" | "animating" | "motion" | "motions" | "movement" | "transition" | "transitions" => Some(Step::RemoveMotion(phase())),
                _ => None,
            };
            if let Some(step) = step {
                self.mark(at, look - at + 1);
                // The "in" or "out" after it belongs to this removal too.
                if self.is(look + 1, &["in", "out"]) {
                    self.mark(look + 1, 1);
                }
                self.steps.push(step);
                return true;
            }
        }
        false
    }

    /// "From 2s to 5s", "between 2 and 4 seconds": a start and an end.
    fn range(&mut self, at: usize) -> bool {
        if !self.is(at, &["from", "between"]) {
            return false;
        }
        let Some((mut first, used)) = time_at(self.tokens, at + 1) else { return false };
        let joiner = at + 1 + used;
        if !self.is(joiner, &["to", "and", "until", "till", "through", "-"]) {
            return false;
        }
        let Some((second, used_second)) = time_at(self.tokens, joiner + 1) else { return false };
        // "Between 200 and 400 ms": a bare first number takes the unit said once, at the end.
        if used == 1
            && let Some(value) = self.word(at + 1).and_then(|word| word.parse::<f64>().ok())
            && let Some(per) = self.word(joiner + used_second).and_then(unit_millis)
        {
            first = Time::millis((value * per).round() as i64);
        }
        self.mark(at, joiner + 1 + used_second - at);
        self.steps.push(Step::StartAt(first));
        self.steps.push(Step::EndAt(second));
        true
    }

    /// "Trim the start to 2s", "move the beginning of it to 1s": the start trimmed, the end
    /// kept. "Trim the end to 6s" is an end.
    fn trim_start(&mut self, at: usize) -> bool {
        if !self.is(at, &["trim", "cut", "move", "set", "drag"]) {
            return false;
        }
        let mut next = at + 1;
        if self.is(next, &["the", "its"]) {
            next += 1;
        }
        let start = match self.word(next) {
            Some("start" | "beginning" | "front" | "head") => true,
            Some("end" | "tail" | "back") => false,
            _ => return false,
        };
        next += 1;
        // "Of it", "of the clip".
        if self.is(next, &["of"]) {
            next = self.after_pronouns(next + 1);
        }
        if !self.is(next, &["to", "at"]) {
            return false;
        }
        let Some((time, used)) = time_at(self.tokens, next + 1) else { return false };
        self.mark(at, next + 1 + used - at);
        self.steps.push(if start { Step::TrimStart(time) } else { Step::EndAt(time) });
        true
    }

    /// "Cut it at 4s", "split this at the playhead", "trim … " is `last`.
    fn cut(&mut self, at: usize) -> bool {
        if !self.is(at, &["cut", "split", "slice", "chop", "razor"]) {
            return false;
        }
        let mut next = self.after_name(at + 1);
        if self.is(next, &["in"]) && self.is(next + 1, &["two", "half"]) {
            next += 2;
        }
        if !self.is(next, &["at", "on", "@"]) {
            return false;
        }
        let Some((time, used)) = time_at(self.tokens, next + 1) else { return false };
        self.mark(at, next + 1 + used - at);
        self.steps.push(Step::CutAt(time));
        true
    }

    /// "Delay it by 1s", "move it 2 seconds later", "1 second earlier", "push it back half a second".
    fn shift(&mut self, at: usize) -> bool {
        // "Delay (it) (by) X".
        if self.is(at, &["delay", "postpone"]) {
            let mut next = self.after_pronouns(at + 1);
            if self.is(next, &["by", "for"]) {
                next += 1;
            }
            if let Some((by, used)) = time_at(self.tokens, next) {
                self.mark(at, next + used - at);
                self.steps.push(Step::Shift { by, later: true });
                return true;
            }
            return false;
        }
        // "X later" / "X earlier" / "later by X" / "earlier by X".
        let direction = |word: &str| match word {
            "later" | "afterwards" | "after" => Some(true),
            "earlier" | "sooner" | "before" => Some(false),
            _ => None,
        };
        if let Some((by, used)) = time_at(self.tokens, at)
            && let Some(later) = self.word(at + used).and_then(direction)
            && !matches!(by, Time::End | Time::Playhead)
        {
            // "Start 2 seconds after" with nothing following is a shift; "after the title" isn't read.
            if self.word(at + used + 1).is_none_or(|word| !matches!(word, "the" | "that")) || matches!(self.word(at + used), Some("later" | "earlier" | "sooner")) {
                self.mark(at, used + 1);
                self.steps.push(Step::Shift { by, later });
                return true;
            }
        }
        if let Some(later) = self.word(at).and_then(direction).filter(|_| self.is(at, &["later", "earlier", "sooner"]))
            && self.is(at + 1, &["by"])
            && let Some((by, used)) = time_at(self.tokens, at + 2)
        {
            self.mark(at, used + 2);
            self.steps.push(Step::Shift { by, later });
            return true;
        }
        // "Push it back X" (later), "pull it forward X" / "bring it forward X" (earlier).
        if self.is(at, &["push", "pull", "bring", "nudge", "shift", "move", "slide"]) {
            let next = self.after_pronouns(at + 1);
            let later = match self.word(next) {
                Some("back" | "later" | "right") => true,
                Some("forward" | "earlier" | "left") => false,
                _ => return false,
            };
            let mut after = next + 1;
            if self.is(after, &["by"]) {
                after += 1;
            }
            if let Some((by, used)) = time_at(self.tokens, after).filter(|(time, _)| matches!(time, Time::Span { .. })) {
                // "Move it right 20 px" is a nudge, not a shift in time.
                if self.is(after + used, &["px", "pixels", "pixel"]) || self.is(after + 1, &["px", "pixels", "pixel"]) {
                    return false;
                }
                self.mark(at, after + used - at);
                self.steps.push(Step::Shift { by, later });
                return true;
            }
        }
        false
    }

    /// "Start at 5s", "begin at 2", "come in at 3s", "appear at the playhead", "move it to 4s",
    /// "put it at 5s", "at 5s" after a way in.
    fn start(&mut self, at: usize) -> bool {
        let verb = self.is(at, &["start", "starts", "starting", "begin", "begins", "beginning", "appear", "appears", "show", "shows", "enter", "enters", "place", "put", "move", "position", "drop"])
            || (self.is(at, &["come", "comes", "coming", "pop", "pops"]) && self.is(at + 1, &["in", "on", "up"]));
        if !verb {
            return false;
        }
        let mut next = self.after_name(at + 1);
        // "Come in at", "show up at".
        if self.is(next, &["in", "up", "on"]) {
            next += 1;
        }
        // "Move it to 4s", "start at 5s", "start from 2s".
        if !self.is(next, &["at", "to", "from", "on", "@"]) {
            return false;
        }
        // "Put it at the end" means start where the composition ends: not useful, so not read.
        let Some((time, used)) = time_at(self.tokens, next + 1).filter(|(time, _)| *time != Time::End) else { return false };
        self.mark(at, next + 1 + used - at);
        self.steps.push(Step::StartAt(time));
        true
    }

    /// "End at 8s", "finish at the playhead", "stop at 6", "disappear at 7s", "until 9s",
    /// "go away at 8s", "be gone by 6s".
    fn end(&mut self, at: usize) -> bool {
        let next = if self.is(at, &["until", "till", "til"]) {
            at + 1
        } else {
            let verb = self.is(at, &["end", "ends", "ending", "finish", "finishes", "stop", "stops", "disappear", "disappears", "vanish", "vanishes", "leave", "leaves", "exit", "exits", "gone", "out"])
                || (self.is(at, &["go", "goes"]) && self.is(at + 1, &["away", "off", "out"]));
            if !verb {
                return false;
            }
            let mut next = self.after_name(at + 1);
            if self.is(next, &["away", "off", "out"]) {
                next += 1;
            }
            // "Disappear after 4 seconds": gone that long after it comes in, so a length.
            if self.is(next, &["after"])
                && let Some((time, used)) = time_at(self.tokens, next + 1).filter(|(time, _)| matches!(time, Time::Span { .. }))
            {
                self.mark(at, next + 1 + used - at);
                self.steps.push(Step::Last(time));
                return true;
            }
            if !self.is(next, &["at", "by", "on", "@"]) {
                return false;
            }
            next + 1
        };
        let Some((time, used)) = time_at(self.tokens, next) else { return false };
        self.mark(at, next + used - at);
        self.steps.push(Step::EndAt(time));
        true
    }

    /// "Last 2 seconds", "lasts for 2s", "for 3 seconds", "make it 4s long", "trim it to 3s",
    /// "a duration of 5s", "2 seconds long".
    fn last(&mut self, at: usize) -> bool {
        let mut next = if self.is(at, &["last", "lasts", "lasting", "stay", "stays", "hold", "holds", "run", "runs", "play", "plays", "show", "shows", "keep"]) {
            let mut next = self.after_pronouns(at + 1);
            if self.is(next, &["on", "up", "there", "visible", "around"]) {
                next += 1;
            }
            if self.is(next, &["for"]) {
                next += 1;
            }
            next
        } else if self.is(at, &["trim", "shorten", "lengthen", "extend", "cut", "make", "set"]) {
            let mut next = self.after_pronouns(at + 1);
            if self.is(next, &["down", "up", "out"]) {
                next += 1;
            }
            if self.is(at, &["make", "set"]) {
                // "Make it 4s long", "set it to 4 seconds": only with "long" or "duration" said.
                if self.is(next, &["to"]) {
                    next += 1;
                }
                let Some((time, used)) = time_at(self.tokens, next).filter(|(time, _)| matches!(time, Time::Span { .. })) else { return false };
                if !self.is(next + used, &["long", "duration", "total"]) {
                    return false;
                }
                self.mark(at, next + used + 1 - at);
                self.steps.push(Step::Last(time));
                return true;
            }
            if !self.is(next, &["to"]) {
                return false;
            }
            next + 1
        } else if self.is(at, &["for"]) {
            at + 1
        } else if self.is(at, &["duration", "length"]) {
            let mut next = at + 1;
            if self.is(next, &["of", "to", "is"]) {
                next += 1;
            }
            next
        } else {
            // "2 seconds long".
            if let Some((time, used)) = time_at(self.tokens, at).filter(|(time, _)| matches!(time, Time::Span { .. }))
                && self.is(at + used, &["long"])
            {
                self.mark(at, used + 1);
                self.steps.push(Step::Last(time));
                return true;
            }
            return false;
        };
        if self.is(next, &["about", "around", "roughly", "exactly", "just"]) {
            next += 1;
        }
        let Some((time, used)) = time_at(self.tokens, next).filter(|(time, _)| matches!(time, Time::Span { .. })) else { return false };
        let mut used_all = next + used - at;
        if self.is(next + used, &["long"]) {
            used_all += 1;
        }
        self.mark(at, used_all);
        self.steps.push(Step::Last(time));
        true
    }

    /// A way in or out: "fade in", "fade it out", "slide in from the left", "slide out to the
    /// right", "rise up", "drop in", "pop in", "zoom out", "type it on", "word by word",
    /// "animate it in", "bounce in", with "over 1s", "quickly" or "slowly" for its length and
    /// "at 5s" for when (the start for a way in, the end for a way out).
    fn motion(&mut self, at: usize) -> bool {
        let Some((motion, phase_hint, mut next)) = self.motion_word(at) else { return false };
        let mut phase = phase_hint;
        let mut motion = motion;
        // In or out, and for slides the side, from the words that follow.
        let limit = (next + 6).min(self.tokens.len());
        let mut index = next;
        while index < limit {
            // "Fade the half off layer in": the "off" is the layer's name.
            if self.inside_layer_name(index) {
                index += 2;
                continue;
            }
            match self.word(index).unwrap_or("") {
                "in" | "on" | "up" | "into" | "intro" | "onto" if phase.is_none() => {
                    if self.word(index) == Some("up") && matches!(motion, Motion::Slide(_)) && phase_hint.is_none() {
                        motion = Motion::Slide(Side::Bottom);
                    }
                    phase = Some(Phase::In);
                }
                "out" | "away" | "off" | "outro" | "exit" if phase.is_none() => phase = Some(Phase::Out),
                "down" if matches!(motion, Motion::Slide(_)) => {
                    motion = Motion::Slide(Side::Top);
                    phase.get_or_insert(Phase::In);
                }
                "from" | "to" | "toward" | "towards" | "the" | "it" | "this" | "them" | "layer" | "text" | "title" => {}
                // A layer's name before the "in" or "out": "fade the logo in".
                _ if phase.is_none() && self.after_name(index) > index && self.is(self.after_name(index), &["in", "out", "on", "off", "away", "up"]) => {
                    index = self.after_name(index);
                    continue;
                }
                "left" | "right" | "top" | "bottom" | "above" | "below" | "side" => {
                    if let Motion::Slide(_) = motion {
                        let side = match self.word(index).unwrap_or("") {
                            "left" => Side::Left,
                            "right" => Side::Right,
                            "top" | "above" => Side::Top,
                            _ => Side::Bottom,
                        };
                        motion = Motion::Slide(side);
                    }
                }
                _ => break,
            }
            index += 1;
        }
        next = index;
        // "Fade at the end" is a way out.
        let phase = phase.unwrap_or_else(|| if self.is(next, &["at"]) && self.is(next + 1, &["the"]) && self.is(next + 2, &["end"]) { Phase::Out } else { Phase::In });
        // How long, and when.
        let mut millis = None;
        let mut when = None;
        let limit = (next + 8).min(self.tokens.len());
        let mut index = next;
        while index < limit {
            let word = self.word(index).unwrap_or("");
            match word {
                "quickly" | "quick" | "fast" | "snappy" | "briskly" => {
                    millis = Some(QUICK_MILLIS);
                    index += 1;
                }
                "slowly" | "slow" | "gently" | "gentle" | "gradually" => {
                    millis = Some(SLOW_MILLIS);
                    index += 1;
                }
                "over" | "in" | "taking" | "within" | "across" | "for" => match time_at(self.tokens, index + 1).filter(|(time, _)| matches!(time, Time::Span { frames: 0, .. })) {
                    Some((Time::Span { millis: length, .. }, used)) => {
                        millis = Some(length);
                        index += 1 + used;
                    }
                    _ => break,
                },
                "at" => match time_at(self.tokens, index + 1) {
                    Some((time, used)) if !(phase == Phase::Out && time == Time::End) => {
                        when = Some(time);
                        index += 1 + used;
                    }
                    // "Fade out at the end": the end is where a way out is anyway.
                    Some((_, used)) => index += 1 + used,
                    None => break,
                },
                "a" | "bit" | "little" | "nice" | "and" | "then" | "very" | "really" | "super" => index += 1,
                _ => break,
            }
        }
        // "2 second fade in": a length said before the motion.
        // It must end right where the motion word starts ("a 2 second fade"), so a time from an
        // earlier instruction ("appear at 1s and fade out") isn't taken for it.
        if millis.is_none() {
            let before = [2, 3].into_iter().find_map(|back| {
                let from = at.checked_sub(back)?;
                let (time, used) = time_at(self.tokens, from)?;
                let unused = !self.used[from..at].iter().any(|used| *used);
                match time {
                    Time::Span { millis: length, frames: 0 } if from + used == at && unused => Some((from, length)),
                    _ => None,
                }
            });
            if let Some((from, length)) = before {
                millis = Some(length);
                self.mark(from, at - from);
            }
        }
        self.mark(at, index - at);
        self.steps.push(Step::Animate { phase, motion, millis });
        if let Some(time) = when {
            self.steps.push(if phase == Phase::In { Step::StartAt(time) } else { Step::EndAt(time) });
        }
        true
    }

    /// The motion a word names, a phase the word itself says ("intro", "typed out"), and where
    /// reading continues.
    fn motion_word(&self, at: usize) -> Option<(Motion, Option<Phase>, usize)> {
        let word = self.word(at)?;
        let next = at + 1;
        let by = |unit: &str| self.is(next, &["by"]) && self.is(next + 1, &[unit]);
        let motion = match word {
            "fade" | "fades" | "fading" | "dissolve" | "dissolves" => Motion::Fade,
            "slide" | "slides" | "sliding" | "fly" | "flies" | "flying" | "swoop" | "swoops" | "sweep" | "sweeps" | "wipe" | "wipes" | "glide" | "glides" => Motion::Slide(Side::Left),
            "rise" | "rises" | "rising" => return Some((Motion::Slide(Side::Bottom), None, next + usize::from(self.is(next, &["up"])))),
            "drop" | "drops" | "fall" | "falls" if self.is(next, &["in", "down", "into"]) || self.is(self.after_pronouns(next), &["in", "down"]) => return Some((Motion::Slide(Side::Top), None, next)),
            "float" | "floats" if self.is(self.after_pronouns(next), &["in", "up"]) => return Some((Motion::Slide(Side::Bottom), Some(Phase::In), self.after_pronouns(next) + 1)),
            "pop" | "pops" | "popping" => Motion::Pop,
            "zoom" | "zooms" | "zooming" | "scale" | "scales" | "grow" | "grows" | "shrink" | "shrinks" if !self.is(self.after_pronouns(next), &["to", "by"]) && time_at(self.tokens, self.after_pronouns(next)).is_none() => Motion::Zoom,
            // "Keep spinning" and "spin forever" are a loop (see `looping`), not a way in.
            "spin" | "spins" | "spinning" | "twirl" | "twirls" if !self.is(next, &["forever", "continuously", "constantly", "around"]) && !self.is(at.wrapping_sub(1), &["keep", "keeps"]) => Motion::Spin,
            "blur" | "blurs" | "blurring" | "focus" => Motion::Blur,
            "bounce" | "bounces" | "bouncing" => Motion::Bounce,
            "type" | "types" | "typed" | "typing" | "typewriter" => Motion::Typewriter,
            "word" | "words" if by("word") => return Some((Motion::ByWord, None, next + 2)),
            "letter" | "letters" | "character" | "characters" if by("letter") || by("character") => return Some((Motion::ByLetter, None, next + 2)),
            "line" | "lines" if by("line") => return Some((Motion::ByLine, None, next + 2)),
            "animate" | "animates" | "animating" | "animation" | "transition" => {
                // Only with in or out said: "animate this" alone is a whole-design request.
                let after = self.after_pronouns(next);
                if !self.is(after, &["in", "out", "on", "off", "away"]) && !self.is(next, &["in", "out"]) {
                    return None;
                }
                Motion::Suited
            }
            "intro" | "entrance" | "entry" => return Some((Motion::Suited, Some(Phase::In), next)),
            "outro" => return Some((Motion::Suited, Some(Phase::Out), next)),
            _ => return None,
        };
        // A way in or out said with "in", "out" or a side, or one word that is plainly a motion.
        let follows = (next..(next + 5).min(self.tokens.len())).any(|index| self.is(index, &["in", "out", "on", "off", "away", "up", "down", "from", "into", "left", "right"]));
        let plainly = matches!(motion, Motion::Fade | Motion::Typewriter | Motion::Bounce | Motion::Pop | Motion::Spin);
        if !follows && !plainly {
            return None;
        }
        Some((motion, None, next))
    }

    /// A loop: "make it pulse", "float", "wiggle", "keep spinning", "spin forever", "blink".
    fn looping(&mut self, at: usize) -> bool {
        let found = match self.word(at).unwrap_or("") {
            "pulse" | "pulses" | "pulsing" | "pulsate" | "throb" | "breathe" | "breathing" | "heartbeat" => Loop::Pulse,
            "float" | "floats" | "floating" | "hover" | "hovers" | "hovering" | "bob" | "bobbing" => Loop::Float,
            "swing" | "swings" | "swinging" | "sway" | "sways" | "swaying" | "rock" | "rocking" => Loop::Swing,
            "blink" | "blinks" | "blinking" | "flash" | "flashes" | "flashing" | "flicker" | "flickering" => Loop::Blink,
            "wiggle" | "wiggles" | "wiggling" | "shake" | "shakes" | "shaking" | "jiggle" | "jiggles" | "wobble" | "wobbles" | "vibrate" => Loop::Wiggle,
            "spin" | "spins" | "spinning" | "rotate" | "rotating" | "turn" | "turning" if self.is(at + 1, &["forever", "continuously", "constantly", "around"]) || self.is(at.wrapping_sub(1), &["keep", "keeps"]) => Loop::Turn,
            _ => return false,
        };
        self.mark(at, 1);
        if self.is(at + 1, &["forever", "continuously", "constantly", "around"]) {
            self.mark(at + 1, 1);
        }
        self.steps.push(Step::Loop(found));
        true
    }

    /// How it looks: "50% opacity", "opacity 80", "make it 120% bigger"… read plainly: "scale
    /// to 120%", "rotate 15 degrees", "tilt it -10°", "move it 20 px left".
    fn look(&mut self, at: usize) -> bool {
        let percent_at = |index: usize| -> Option<(i64, usize)> {
            let (value, used) = number_at(self.tokens, index)?;
            self.is(index + used, &["%", "percent", "per"]).then(|| (value.round() as i64, used + 1 + usize::from(self.is(index + used + 1, &["cent"]))))
        };
        // "Opacity (to) 50(%)" and "50% opacity", "50% transparent" (which is 50% opacity).
        if self.is(at, &["opacity", "opaque"]) {
            let mut next = at + 1;
            if self.is(next, &["to", "of", "at"]) {
                next += 1;
            }
            if let Some((value, used)) = percent_at(next).or_else(|| number_at(self.tokens, next).map(|(value, used)| (value.round() as i64, used))) {
                self.mark(at, next + used - at);
                self.steps.push(Step::Opacity(value.clamp(0, 100)));
                return true;
            }
        }
        if let Some((value, used)) = percent_at(at) {
            match self.word(at + used) {
                Some("opacity" | "opaque" | "visible") => {
                    self.mark(at, used + 1);
                    self.steps.push(Step::Opacity(value.clamp(0, 100)));
                    return true;
                }
                Some("transparent" | "see" | "transparency") => {
                    self.mark(at, used + 1);
                    self.steps.push(Step::Opacity((100 - value).clamp(0, 100)));
                    return true;
                }
                Some("size" | "scale" | "big" | "large") => {
                    self.mark(at, used + 1);
                    self.steps.push(Step::Scale(value.max(0)));
                    return true;
                }
                Some("bigger" | "larger") => {
                    self.mark(at, used + 1);
                    self.steps.push(Step::Scale(100 + value.max(0)));
                    return true;
                }
                Some("smaller") => {
                    self.mark(at, used + 1);
                    self.steps.push(Step::Scale((100 - value).max(0)));
                    return true;
                }
                _ => {}
            }
        }
        // "Scale (it) (to) 120%", "resize to 80%".
        if self.is(at, &["scale", "resize", "size"]) {
            let mut next = self.after_pronouns(at + 1);
            if self.is(next, &["to", "at"]) {
                next += 1;
            }
            if let Some((value, used)) = percent_at(next) {
                self.mark(at, next + used - at);
                self.steps.push(Step::Scale(value.max(0)));
                return true;
            }
        }
        // "Rotate (it) (by) 15 (degrees)", "tilt it -10°".
        if self.is(at, &["rotate", "turn", "tilt", "angle"]) {
            let mut next = self.after_pronouns(at + 1);
            if self.is(next, &["by", "to"]) {
                next += 1;
            }
            if let Some((value, used)) = number_at(self.tokens, next)
                && self.is(next + used, &["degrees", "degree", "deg", "°"])
            {
                let mut used = used + 1;
                let mut degrees = value.round() as i64;
                if self.is(next + used, &["counterclockwise", "anticlockwise", "left"]) {
                    degrees = -degrees;
                    used += 1;
                } else if self.is(next + used, &["clockwise", "right"]) {
                    used += 1;
                }
                self.mark(at, next + used - at);
                self.steps.push(Step::Rotate(degrees));
                return true;
            }
        }
        // "Move (it) 20 px left", "nudge it 10 pixels up", "move it left 20px".
        if self.is(at, &["move", "nudge", "shift", "push"]) {
            let next = self.after_pronouns(at + 1);
            let direction = |word: Option<&str>| match word {
                Some("left") => Some((-1, 0)),
                Some("right") => Some((1, 0)),
                Some("up") => Some((0, -1)),
                Some("down") => Some((0, 1)),
                _ => None,
            };
            let (amount_at, direction_at) = if direction(self.word(next)).is_some() { (next + 1, next) } else { (next, usize::MAX) };
            if let Some((value, used)) = number_at(self.tokens, amount_at)
                && self.is(amount_at + used, &["px", "pixels", "pixel"])
            {
                let mut end = amount_at + used + 1;
                let (dx, dy) = match direction(self.word(direction_at)).or_else(|| direction(self.word(end))) {
                    Some(found) => {
                        if direction_at == usize::MAX {
                            end += 1;
                        }
                        found
                    }
                    None => return false,
                };
                let amount = value.round() as i64;
                self.mark(at, end - at);
                self.steps.push(Step::Nudge { x: dx * amount, y: dy * amount });
                return true;
            }
        }
        false
    }
}

// Which layers -----------------------------------------------------------------------------------

/// The kind of layer a word names, and whether the word is a plural.
fn kind_word(word: &str) -> Option<(Kind, bool)> {
    Some(match word {
        "text" => (Kind::Text, false),
        "texts" => (Kind::Text, true),
        "image" | "picture" | "photo" | "pic" => (Kind::Picture, false),
        "images" | "pictures" | "photos" | "pics" => (Kind::Picture, true),
        "shape" | "vector" => (Kind::Shape, false),
        "shapes" | "vectors" => (Kind::Shape, true),
        "video" | "footage" => (Kind::Video, false),
        "videos" => (Kind::Video, true),
        "audio" | "sound" | "music" => (Kind::Sound, false),
        "sounds" => (Kind::Sound, true),
        _ => return None,
    })
}

/// The kind of layer the message is about as a group: "all text layers", "every image", "the
/// shapes", "each photo". One layer of a kind ("the photo") is not a group.
fn kind_meant(tokens: &[String]) -> Option<Kind> {
    let word = |index: usize| tokens.get(index).map(String::as_str);
    (0..tokens.len()).find_map(|index| {
        let (kind, plural) = kind_word(&tokens[index])?;
        let before = |back: usize| index.checked_sub(back).and_then(word);
        let followed_by_layers = matches!(word(index + 1), Some("layers" | "clips" | "elements" | "objects" | "ones"));
        let every = matches!(before(1), Some("all" | "every" | "each")) || (before(1) == Some("the") && before(2) == Some("all"));
        let the_group = plural && matches!(before(1), Some("the" | "my" | "those" | "these"));
        (followed_by_layers || every || the_group).then_some(kind)
    })
}

/// Which layers the message is about.
pub(crate) fn target(message: &str, tokens: &[String]) -> Target {
    // A name in quotes wins: "fade “Big Sale” in".
    for (open, close) in [('"', '"'), ('“', '”'), ('\'', '\''), ('‘', '’')] {
        if let Some(start) = message.find(open) {
            let rest = &message[start + open.len_utf8()..];
            if let Some(end) = rest.find(close) {
                let name = rest[..end].trim();
                // An apostrophe in "don't" isn't a quote: a quoted name has no letter right before it.
                let before_ok = open != '\'' || message[..start].chars().last().is_none_or(|c| !c.is_alphanumeric());
                if !name.is_empty() && before_ok {
                    return Target::Named(name.to_owned());
                }
            }
        }
    }
    let word = |index: usize| tokens.get(index).map(String::as_str);
    for index in 0..tokens.len() {
        // "Layer 2", "layer two".
        if word(index) == Some("layer")
            && let Some((number, _)) = number_at(tokens, index + 1).filter(|_| word(index + 1) != Some("a"))
            && number >= 1.0
            && number.fract() == 0.0
            && !tokens.get(index + 2).is_some_and(|unit| unit_millis(unit).is_some() || matches!(unit.as_str(), "%" | "px" | "frames"))
        {
            return Target::Number(number as usize);
        }
        // "The layer called Title", "layer named Big Sale" (to the next instruction word).
        if word(index) == Some("layer") && matches!(word(index + 1), Some("called" | "named")) {
            let name: Vec<&str> = tokens[index + 2..].iter().map(String::as_str).take_while(|word| !INSTRUCTION_WORDS.contains(word)).collect();
            if !name.is_empty() {
                return Target::Named(name.join(" "));
            }
        }
    }
    if let Some(kind) = kind_meant(tokens) {
        return Target::Kind(kind);
    }
    if tokens.iter().any(|token| matches!(token.as_str(), "everything" | "all" | "every")) {
        return Target::Everything;
    }
    // "The title layer", "the logo layer": the words between "the" and "layer".
    for index in 0..tokens.len() {
        if word(index) == Some("layer") && index >= 2 {
            let Some(start) = tokens[..index].iter().rposition(|token| token == "the") else { continue };
            let name: Vec<&str> = tokens[start + 1..index].iter().map(String::as_str).collect();
            if !name.is_empty() && name.len() <= 4 && !name.iter().any(|word| matches!(*word, "selected" | "current" | "this" | "same" | "top" | "bottom" | "first" | "last")) {
                return Target::Named(name.join(" "));
            }
        }
    }
    if tokens.iter().any(|token| matches!(token.as_str(), "this" | "it" | "these" | "them" | "selected" | "selection" | "current")) {
        return Target::Selection;
    }
    Target::Unnamed
}

/// Small words instructions are built from, which a layer's name never runs through.
const JOINING_WORDS: &[&str] = &["at", "to", "from", "on", "in", "out", "up", "down", "off", "away", "over", "for", "by", "until", "till", "after", "before", "and", "then", "with", "into", "onto", "of", "is", "be", "should", "slowly", "quickly", "later", "earlier", "left", "right", "word", "letter", "line", "forever"];

/// Words that start an instruction, which end a layer's name ("layer called Title fade in").
const INSTRUCTION_WORDS: &[&str] = &["fade", "start", "starts", "end", "ends", "last", "lasts", "cut", "split", "slide", "pop", "zoom", "move", "and", "to", "at", "for", "from", "then", "in", "out", "should", "make", "spin", "bounce", "rise", "drop", "type", "animate", "pulse", "float"];

/// The words instructions are written with, for putting a mistyped one right (see `spelling`).
pub fn vocabulary() -> Vec<String> {
    const WORDS: &[&str] = &[
        "fade", "slide", "start", "starts", "begin", "finish", "second", "seconds", "frame", "frames", "minute", "playhead", "opacity", "transparent", "rotate", "degrees", "scale", "bigger", "smaller", "larger", "pulse", "float",
        "swing", "blink", "wiggle", "bounce", "typewriter", "letter", "word", "delay", "later", "earlier", "quickly", "slowly", "remove", "animation", "animate", "layer", "layers", "everything", "selected", "selection", "trim",
        "split", "nudge", "pixels", "entrance", "intro", "outro", "exit", "shake", "appear", "disappear", "bottom", "right", "left", "image", "images", "picture", "pictures", "photo", "photos", "shape", "shapes", "video", "videos",
        "audio", "sound", "music", "text", "title", "every", "each", "called", "named",
    ];
    WORDS.iter().chain(JOINING_WORDS).chain(INSTRUCTION_WORDS).map(|word| (*word).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(message: &str) -> Vec<Step> {
        parse(message).map(|request| request.steps).unwrap_or_default()
    }

    fn secs(seconds: f64) -> Time {
        Time::millis((seconds * 1000.0).round() as i64)
    }

    #[test]
    fn times_read_in_many_forms() {
        let time = |text: &str| time_at(&tokenize(text), 0).map(|(time, _)| time);
        assert_eq!(time("5s"), Some(secs(5.0)));
        assert_eq!(time("5 seconds"), Some(secs(5.0)));
        assert_eq!(time("2.5sec"), Some(secs(2.5)));
        assert_eq!(time("half a second"), Some(secs(0.5)));
        assert_eq!(time("a second"), Some(secs(1.0)));
        assert_eq!(time("one and a half seconds"), Some(secs(1.5)));
        assert_eq!(time("a second and a half"), Some(secs(1.5)));
        assert_eq!(time("twenty five seconds"), Some(secs(25.0)));
        assert_eq!(time("500ms"), Some(secs(0.5)));
        assert_eq!(time("2 minutes"), Some(secs(120.0)));
        assert_eq!(time("1:05"), Some(secs(65.0)));
        assert_eq!(time("0:00:05:12"), Some(Time::Span { millis: 5000, frames: 12 }));
        assert_eq!(time("frame 120"), Some(Time::Span { millis: 0, frames: 120 }));
        assert_eq!(time("48 frames"), Some(Time::Span { millis: 0, frames: 48 }));
        assert_eq!(time("the playhead"), Some(Time::Playhead));
        assert_eq!(time("the end"), Some(Time::End));
        assert_eq!(time("7"), Some(secs(7.0)), "a bare number is seconds");
        assert_eq!(time("a"), None);
    }

    #[test]
    fn the_request_from_the_brief_reads_whole() {
        // Start, length and a way in, across two sentences.
        assert_eq!(
            steps("I want this layer to start at 5s and last for 2 seconds. I want it to fade in."),
            vec![Step::StartAt(secs(5.0)), Step::Last(secs(2.0)), Step::Animate { phase: Phase::In, motion: Motion::Fade, millis: None }]
        );
        assert_eq!(parse("make this start at 5s").unwrap().target, Target::Selection);
    }

    #[test]
    fn timing_reads_in_many_wordings() {
        assert_eq!(steps("start at 3 seconds"), vec![Step::StartAt(secs(3.0))]);
        assert_eq!(steps("have it come in at 2s"), vec![Step::StartAt(secs(2.0))]);
        assert_eq!(steps("move it to 4s"), vec![Step::StartAt(secs(4.0))]);
        assert_eq!(steps("make it appear at the playhead"), vec![Step::StartAt(Time::Playhead)]);
        assert_eq!(steps("end at 8s"), vec![Step::EndAt(secs(8.0))]);
        assert_eq!(steps("it should disappear at 7.5 seconds"), vec![Step::EndAt(secs(7.5))]);
        assert_eq!(steps("keep it until 9s"), vec![Step::EndAt(secs(9.0))]);
        assert_eq!(steps("make it last 2 seconds"), vec![Step::Last(secs(2.0))]);
        assert_eq!(steps("show it for 3s"), vec![Step::Last(secs(3.0))]);
        assert_eq!(steps("trim it to 1.5s"), vec![Step::Last(secs(1.5))]);
        assert_eq!(steps("make it 4 seconds long"), vec![Step::Last(secs(4.0))]);
        assert_eq!(steps("from 2s to 5s"), vec![Step::StartAt(secs(2.0)), Step::EndAt(secs(5.0))]);
        assert_eq!(steps("between 2 and 4 seconds"), vec![Step::StartAt(secs(2.0)), Step::EndAt(secs(4.0))]);
        assert_eq!(steps("from 200 to 900 ms"), vec![Step::StartAt(secs(0.2)), Step::EndAt(secs(0.9))]);
        assert_eq!(steps("delay it by half a second"), vec![Step::Shift { by: secs(0.5), later: true }]);
        assert_eq!(steps("move it 2 seconds later"), vec![Step::Shift { by: secs(2.0), later: true }]);
        assert_eq!(steps("start it 1s earlier"), vec![Step::Shift { by: secs(1.0), later: false }]);
        assert_eq!(steps("push it back 3 seconds"), vec![Step::Shift { by: secs(3.0), later: true }]);
        assert_eq!(steps("cut it at 4s"), vec![Step::CutAt(secs(4.0))]);
        assert_eq!(steps("split this at the playhead"), vec![Step::CutAt(Time::Playhead)]);
        assert_eq!(steps("cut the clip in two at 0:00:03:10"), vec![Step::CutAt(Time::Span { millis: 3000, frames: 10 })]);
    }

    #[test]
    fn ways_in_and_out_read_in_many_wordings() {
        let animate = |phase, motion, millis| Step::Animate { phase, motion, millis };
        assert_eq!(steps("fade it in"), vec![animate(Phase::In, Motion::Fade, None)]);
        assert_eq!(steps("fade out"), vec![animate(Phase::Out, Motion::Fade, None)]);
        assert_eq!(steps("fade it away"), vec![animate(Phase::Out, Motion::Fade, None)]);
        assert_eq!(steps("fade at the end"), vec![animate(Phase::Out, Motion::Fade, None)]);
        assert_eq!(steps("fade in over 1 second"), vec![animate(Phase::In, Motion::Fade, Some(1000))]);
        assert_eq!(steps("a 2 second fade in"), vec![animate(Phase::In, Motion::Fade, Some(2000))]);
        assert_eq!(steps("fade in slowly"), vec![animate(Phase::In, Motion::Fade, Some(SLOW_MILLIS))]);
        assert_eq!(steps("slide in from the left"), vec![animate(Phase::In, Motion::Slide(Side::Left), None)]);
        assert_eq!(steps("slide it out to the right quickly"), vec![animate(Phase::Out, Motion::Slide(Side::Right), Some(QUICK_MILLIS))]);
        assert_eq!(steps("fly in from the top"), vec![animate(Phase::In, Motion::Slide(Side::Top), None)]);
        assert_eq!(steps("slide up"), vec![animate(Phase::In, Motion::Slide(Side::Bottom), None)]);
        assert_eq!(steps("rise up"), vec![animate(Phase::In, Motion::Slide(Side::Bottom), None)]);
        assert_eq!(steps("drop it in"), vec![animate(Phase::In, Motion::Slide(Side::Top), None)]);
        assert_eq!(steps("pop in"), vec![animate(Phase::In, Motion::Pop, None)]);
        assert_eq!(steps("zoom out"), vec![animate(Phase::Out, Motion::Zoom, None)]);
        assert_eq!(steps("bounce in"), vec![animate(Phase::In, Motion::Bounce, None)]);
        assert_eq!(steps("type it on"), vec![animate(Phase::In, Motion::Typewriter, None)]);
        assert_eq!(steps("bring the words in word by word"), vec![animate(Phase::In, Motion::ByWord, None)]);
        assert_eq!(steps("animate it in"), vec![animate(Phase::In, Motion::Suited, None)]);
        assert_eq!(steps("animate this out"), vec![animate(Phase::Out, Motion::Suited, None)]);
        assert_eq!(steps("fade in at 2s"), vec![animate(Phase::In, Motion::Fade, None), Step::StartAt(secs(2.0))]);
        assert_eq!(steps("fade out at 8s"), vec![animate(Phase::Out, Motion::Fade, None), Step::EndAt(secs(8.0))]);
        assert_eq!(
            steps("fade in and slide out to the left"),
            vec![animate(Phase::In, Motion::Fade, None), animate(Phase::Out, Motion::Slide(Side::Left), None)]
        );
    }

    #[test]
    fn loops_looks_and_removals() {
        assert_eq!(steps("make it pulse"), vec![Step::Loop(Loop::Pulse)]);
        assert_eq!(steps("keep spinning"), vec![Step::Loop(Loop::Turn)]);
        assert_eq!(steps("wiggle it"), vec![Step::Loop(Loop::Wiggle)]);
        assert_eq!(steps("remove the fade in"), vec![Step::RemoveMotion(Some(Phase::In))]);
        assert_eq!(steps("no animation"), vec![Step::RemoveMotion(None)]);
        assert_eq!(steps("get rid of the outro"), vec![Step::RemoveMotion(Some(Phase::Out))]);
        assert_eq!(steps("stop it pulsing"), vec![Step::RemoveLoop]);
        assert_eq!(steps("set it to 50% opacity"), vec![Step::Opacity(50)]);
        assert_eq!(steps("make it 30% transparent"), vec![Step::Opacity(70)]);
        assert_eq!(steps("scale it to 120%"), vec![Step::Scale(120)]);
        assert_eq!(steps("make it 20% bigger"), vec![Step::Scale(120)]);
        assert_eq!(steps("rotate it 15 degrees"), vec![Step::Rotate(15)]);
        assert_eq!(steps("tilt it 10° counterclockwise"), vec![Step::Rotate(-10)]);
        assert_eq!(steps("move it 20px left"), vec![Step::Nudge { x: -20, y: 0 }]);
        assert_eq!(steps("nudge it up 5 pixels"), vec![Step::Nudge { x: 0, y: -5 }]);
    }

    #[test]
    fn layers_are_named_in_several_ways() {
        let target = |message: &str| parse(message).map(|request| request.target);
        assert_eq!(target("fade “Big Sale” in"), Some(Target::Named("Big Sale".into())));
        assert_eq!(target("make layer 2 start at 3s"), Some(Target::Number(2)));
        assert_eq!(target("the layer called Logo should fade in"), Some(Target::Named("logo".into())));
        assert_eq!(target("fade the title layer in"), Some(Target::Named("title".into())));
        assert_eq!(target("fade everything out at the end"), Some(Target::Everything));
        assert_eq!(target("fade in"), Some(Target::Unnamed));
        assert_eq!(target("don't fade it"), Some(Target::Selection), "an apostrophe isn't a quote");
    }

    #[test]
    fn a_layer_name_holding_a_direction_word_is_not_read_as_one() {
        assert_eq!(steps("fade the half off layer in"), vec![Step::Animate { phase: Phase::In, motion: Motion::Fade, millis: None }]);
        let request = parse("start the half off layer at 2s and make it red and fade out").unwrap();
        assert!(request.steps.contains(&Step::StartAt(secs(2.0))), "{request:?}");
        assert!(request.steps.contains(&Step::Animate { phase: Phase::Out, motion: Motion::Fade, millis: None }), "{request:?}");
        assert_eq!(request.target, Target::Named("half off".to_owned()));
        // Plain directions still read as they did.
        assert_eq!(steps("fade the logo out"), vec![Step::Animate { phase: Phase::Out, motion: Motion::Fade, millis: None }]);
        assert_eq!(steps("fade it off"), vec![Step::Animate { phase: Phase::Out, motion: Motion::Fade, millis: None }]);
    }

    #[test]
    fn hours_and_targets_read_on_their_own() {
        assert_eq!(steps("last 2 hours"), vec![Step::Last(Time::millis(7_200_000))]);
        assert_eq!(steps("make it last an hour"), vec![Step::Last(Time::millis(3_600_000))]);
        assert_eq!(target_of("the logo layer"), Target::Named("logo".to_owned()));
        assert_eq!(target_of("all text layers"), Target::Kind(Kind::Text));
        assert_eq!(target_of("them"), Target::Selection);
    }

    #[test]
    fn a_kind_of_layer_is_meant_as_a_group() {
        let target = |message: &str| parse(message).map(|request| request.target);
        assert_eq!(target("fade in all text layers"), Some(Target::Kind(Kind::Text)));
        assert_eq!(target("fade every image out at the end"), Some(Target::Kind(Kind::Picture)));
        assert_eq!(target("make the shapes pulse"), Some(Target::Kind(Kind::Shape)));
        assert_eq!(target("start all the photos at 2s"), Some(Target::Kind(Kind::Picture)));
        // One of a kind is a layer to find by name, and "all" alone is still everything.
        assert_eq!(target("fade the photo in"), Some(Target::Unnamed));
        assert_eq!(target("fade all layers in"), Some(Target::Everything));
    }

    #[test]
    fn layer_names_inside_instructions_and_other_trouble_spots() {
        let fade_in = |millis| Step::Animate { phase: Phase::In, motion: Motion::Fade, millis };
        assert_eq!(steps("fade the logo in over 2s"), vec![fade_in(Some(2000))]);
        assert_eq!(steps("start the music at 3s"), vec![Step::StartAt(secs(3.0))]);
        assert_eq!(steps("cut the intro clip at 3s"), vec![Step::CutAt(secs(3.0))]);
        assert_eq!(steps("make it disappear after 4 seconds"), vec![Step::Last(secs(4.0))]);
        assert_eq!(steps("trim the start to 2s"), vec![Step::TrimStart(secs(2.0))]);
        assert_eq!(steps("trim the end to 6s"), vec![Step::EndAt(secs(6.0))]);
        // The 1s belongs to "appear", not to the fade.
        assert_eq!(
            steps("I want the headline to appear at 1s and fade out at 4s"),
            vec![Step::StartAt(secs(1.0)), Step::Animate { phase: Phase::Out, motion: Motion::Fade, millis: None }, Step::EndAt(secs(4.0))]
        );
        // A named motion wins over "animate it in".
        assert_eq!(steps("animate the title in word by word"), vec![Step::Animate { phase: Phase::In, motion: Motion::ByWord, millis: None }]);
        assert_eq!(steps("slide in from left, hold for 2 seconds, slide out right"), vec![
            Step::Animate { phase: Phase::In, motion: Motion::Slide(Side::Left), millis: None },
            Step::Last(secs(2.0)),
            Step::Animate { phase: Phase::Out, motion: Motion::Slide(Side::Right), millis: None },
        ]);
    }

    #[test]
    fn questions_and_other_requests_are_left_alone() {
        for message in ["how do I fade in?", "what starts at 5s?", "is anything fading out", "animate this", "animate everything", "hello", "move the playhead to 5s", "thanks", "which layers are animated?"] {
            assert_eq!(parse(message), None, "{message}");
        }
        // Asked politely, it is still a request.
        assert_eq!(steps("can you fade this in?"), vec![Step::Animate { phase: Phase::In, motion: Motion::Fade, millis: None }]);
    }
}
