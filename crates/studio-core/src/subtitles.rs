//! Subtitle files: reading SubRip (.srt) and WebVTT (.vtt) cues into caption words, and writing
//! a caption layer's phrases back out as either.
//!
//! Subtitle files come from anywhere, so reading is forgiving about what is harmless (a byte
//! order mark, Windows line endings, a missing blank line at the end or between cues, extra
//! spaces, formatting tags like `<i>`, UTF-16 or Windows-1252 text) and strict about the rest:
//! a cue whose timing can't be read is left out and counted, a file with no readable cues is
//! refused, and sizes are capped so a huge or hostile file can't exhaust memory.
//!
//! Times are converted between milliseconds and frames with the frame rate's exact fraction,
//! so a file written here reads back to the same frames.

use crate::captions::{CaptionWord, MAX_WORDS_PER_PHRASE};
use crate::model::{Layer, LayerContent};
use crate::time::FrameRate;

/// The two subtitle formats Studio reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtitleFormat {
    /// SubRip: numbered cues, times like `00:01:02,500`.
    Srt,
    /// WebVTT, the web's format: a `WEBVTT` header, times like `00:01:02.500`.
    WebVtt,
}

impl SubtitleFormat {
    pub const ALL: [Self; 2] = [Self::Srt, Self::WebVtt];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::WebVtt => "vtt",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Srt => "SRT",
            Self::WebVtt => "WebVTT",
        }
    }

    /// The format a file's extension names, if it is a subtitle file.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::ALL.into_iter().find(|format| format.extension().eq_ignore_ascii_case(extension))
    }
}

/// The largest subtitle file read. A feature film's subtitles are well under a megabyte.
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// The most cues read from one file.
const MAX_CUES: usize = 100_000;
/// The most characters kept of one cue's text.
const MAX_CUE_CHARS: usize = 2_000;
/// The largest hour a timestamp may name (a little over 41 days).
const MAX_HOURS: u64 = 999;

/// One subtitle: its text on one line, and when it shows, in composition frames (from `start`
/// to the frame before `end`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub start: i64,
    pub end: i64,
    pub text: String,
}

/// What reading a subtitle file found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadCues {
    /// The cues, in time order, none overlapping the next.
    pub cues: Vec<Cue>,
    /// How many cues were left out because their timing couldn't be read.
    pub skipped: usize,
}

/// Reads the cues of an SRT or WebVTT file's bytes, timed in frames at `rate`. Either format is
/// accepted whatever the file is called. Fails, with a message to show, when the file is too
/// big, isn't text, or has no readable cues.
pub fn read(bytes: &[u8], rate: FrameRate) -> Result<ReadCues, String> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("it is larger than {} MB, too big to be subtitles", MAX_FILE_BYTES / (1024 * 1024)));
    }
    let text = decode_text(bytes)?;
    let (timed, skipped) = parse_cues(&text)?;
    if timed.is_empty() {
        return Err(if skipped > 0 {
            format!("none of its {skipped} subtitles has a time that can be read")
        } else {
            "it has no subtitles in it. Is it an SRT or WebVTT file?".to_owned()
        });
    }
    let cues = timed.into_iter().map(|cue| Cue { start: millis_to_frame(cue.start, rate), end: millis_to_frame(cue.end, rate), text: cue.text }).collect();
    Ok(ReadCues { cues: tidy(cues), skipped })
}

/// Writes `cues` (composition frames at `rate`) as a subtitle file. Cues before the
/// composition's start are cut at zero.
pub fn write(cues: &[Cue], format: SubtitleFormat, rate: FrameRate) -> String {
    let mut out = String::new();
    if format == SubtitleFormat::WebVtt {
        out.push_str("WEBVTT\n\n");
    }
    let shown = cues.iter().filter(|cue| cue.end > cue.start.max(0));
    for (index, cue) in shown.enumerate() {
        let (start, end) = (frame_to_millis(cue.start.max(0), rate), frame_to_millis(cue.end, rate));
        let text = one_line(&cue.text);
        match format {
            SubtitleFormat::Srt => {
                // A "-->" in the text would read back as a timing line.
                let text = text.replace("-->", "->");
                out.push_str(&format!("{}\n{} --> {}\n{text}\n\n", index + 1, timestamp(start, ','), timestamp(end, ',')));
            }
            SubtitleFormat::WebVtt => {
                // WebVTT text is markup, so these three must be escaped (which also keeps "-->" out).
                let text = text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                out.push_str(&format!("{} --> {}\n{text}\n\n", timestamp(start, '.'), timestamp(end, '.')));
            }
        }
    }
    out
}

/// A caption layer's phrases as cues, as the composition shows them: in composition frames
/// with the layer's sync applied, and only while the layer is on screen. Text keeps the words'
/// own capitals (the All capitals setting is a look, not part of the words). Other layers have
/// no cues.
pub fn layer_cues(layer: &Layer, rate: FrameRate) -> Vec<Cue> {
    let LayerContent::Captions { words, style } = &layer.content else { return Vec::new() };
    let shift = layer.start_frame + style.offset;
    crate::captions::shown_phrases(words, style.words_per_phrase, rate.fps())
        .into_iter()
        .filter_map(|phrase| {
            let start = (phrase.start + shift).max(layer.in_frame);
            let end = (phrase.end + shift).min(layer.out_frame);
            let text = one_line(&words[phrase.words].iter().map(|word| word.text.as_str()).collect::<Vec<_>>().join(" "));
            (end > start && !text.is_empty()).then_some(Cue { start, end, text })
        })
        .collect()
}

/// Caption words for `cues`, timed in the same frames: each cue's time is shared among its
/// words by length, and its last word ends the cue, so the cue shows whole and leaves on time.
pub fn caption_words(cues: &[Cue]) -> Vec<CaptionWord> {
    let mut words = Vec::new();
    for cue in cues {
        let mut cue_words = crate::captions::spread_words(&cue.text, cue.start, cue.end);
        if let Some(last) = cue_words.last_mut() {
            last.ends_cue = true;
        }
        words.extend(cue_words);
    }
    words
}

/// How many words a caption layer made from `cues` should show at once: enough for its longest
/// cue (up to the most a phrase can hold), so cues aren't broken up.
pub fn words_per_phrase(cues: &[Cue]) -> u32 {
    let longest = cues.iter().map(|cue| cue.text.split_whitespace().count()).max().unwrap_or(1);
    (longest as u32).clamp(1, MAX_WORDS_PER_PHRASE)
}

/// A cue as read from the file, timed in milliseconds.
struct TimedCue {
    start: u64,
    end: u64,
    text: String,
}

/// The file's bytes as text. A byte order mark says UTF-8 or UTF-16; without one the text is
/// UTF-8 if it is valid, else Windows-1252 (which older subtitle files often are). Text with
/// NUL characters in it is a binary file, not subtitles.
fn decode_text(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |bytes: &[u8], little_endian: bool| {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&pair| if little_endian { u16::from_le_bytes(pair) } else { u16::from_be_bytes(pair) }).collect();
        String::from_utf16_lossy(&units)
    };
    let text = match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => text.to_owned(),
            Err(_) => bytes.iter().map(|&byte| windows_1252(byte)).collect(),
        },
    };
    if text.contains('\0') {
        return Err("it isn't a text file, so it can't be subtitles".to_owned());
    }
    Ok(text)
}

/// A Windows-1252 byte as a character. It matches Latin-1 except for 0x80–0x9F, where it has
/// curly quotes, dashes and a few letters.
fn windows_1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž', '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}', 'ž', 'Ÿ',
    ];
    match byte {
        0x80..=0x9F => HIGH[usize::from(byte - 0x80)],
        _ => char::from(byte),
    }
}

/// Reads the cues out of an SRT or WebVTT file's text, in milliseconds, and counts the cues
/// whose timing line couldn't be read. A cue starts at a line holding `-->` and runs until a
/// blank line or the next timing line; anything outside a cue (WebVTT's header, NOTE and STYLE
/// blocks, cue numbers and names) is passed over.
fn parse_cues(text: &str) -> Result<(Vec<TimedCue>, usize), String> {
    let mut cues: Vec<TimedCue> = Vec::new();
    let mut skipped = 0;
    // The cue being read: its timing and text lines. `None` when between cues or inside one
    // whose timing couldn't be read (whose text is passed over).
    let mut current: Option<(u64, u64, Vec<String>)> = None;
    let finish = |current: &mut Option<(u64, u64, Vec<String>)>, cues: &mut Vec<TimedCue>| -> Result<(), String> {
        if let Some((start, end, lines)) = current.take() {
            let text = clean_text(&lines);
            if !text.is_empty() {
                if cues.len() >= MAX_CUES {
                    return Err(format!("it has more than {MAX_CUES} subtitles"));
                }
                cues.push(TimedCue { start, end, text });
            }
        }
        Ok(())
    };
    for line in text.lines() {
        // `lines` leaves a carriage return only where one stands alone (old Mac line endings).
        for line in line.split('\r') {
            if line.contains("-->") {
                // A number on the line before the timing is the next cue's SRT index, even
                // when the blank line that should come first is missing.
                if let Some((_, _, lines)) = current.as_mut()
                    && lines.last().is_some_and(|last| is_cue_number(last))
                {
                    lines.pop();
                }
                finish(&mut current, &mut cues)?;
                match timing_line(line) {
                    Some((start, end)) => current = Some((start, end, Vec::new())),
                    None => skipped += 1,
                }
            } else if line.trim().is_empty() {
                finish(&mut current, &mut cues)?;
            } else if let Some((_, _, lines)) = current.as_mut() {
                lines.push(line.to_owned());
            }
        }
    }
    finish(&mut current, &mut cues)?;
    Ok((cues, skipped))
}

/// Whether a line is just a number, as SRT numbers its cues.
fn is_cue_number(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.bytes().all(|byte| byte.is_ascii_digit())
}

/// The start and end of a timing line such as `00:00:01,000 --> 00:00:02,500` (WebVTT's cue
/// settings, or SRT's screen coordinates, may follow the end). `None` when either time can't be
/// read or the cue would end before it starts.
fn timing_line(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    let (start, end) = (parse_timestamp(start.trim())?, parse_timestamp(end)?);
    (end >= start).then_some((start, end))
}

/// A timestamp in milliseconds: `hours:minutes:seconds.fraction` or `minutes:seconds.fraction`,
/// with a comma or a full stop before the fraction (SRT and WebVTT differ there; files mix them
/// up). The fraction may be left out or have fewer than three digits (`,5` is half a second).
fn parse_timestamp(text: &str) -> Option<u64> {
    let (clock, fraction) = match text.find([',', '.']) {
        Some(at) => (&text[..at], Some(&text[at + 1..])),
        None => (text, None),
    };
    let number = |digits: &str, longest: usize| -> Option<u64> {
        (!digits.is_empty() && digits.len() <= longest && digits.bytes().all(|byte| byte.is_ascii_digit())).then(|| digits.parse().ok()).flatten()
    };
    let parts: Vec<&str> = clock.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [hours, minutes, seconds] => (number(hours, 3)?, number(minutes, 2)?, number(seconds, 2)?),
        [minutes, seconds] => (0, number(minutes, 2)?, number(seconds, 2)?),
        _ => return None,
    };
    if hours > MAX_HOURS || minutes >= 60 || seconds >= 60 {
        return None;
    }
    let millis = match fraction {
        // Only the first three digits count: they are the milliseconds.
        Some(digits) => {
            let kept: String = digits.chars().take(3).collect();
            if digits.len() > 9 || !digits.bytes().all(|byte| byte.is_ascii_digit()) || kept.is_empty() {
                return None;
            }
            format!("{kept:0<3}").parse::<u64>().ok()?
        }
        None => 0,
    };
    Some(((hours * 60 + minutes) * 60 + seconds) * 1000 + millis)
}

/// A cue's text lines as one line of plain text: formatting tags (`<i>`, `<font …>`, WebVTT's
/// `<v Speaker>` and `<c.class>`), SubStation override codes like `{\an8}` and doubled spaces
/// are removed, and HTML character references (`&amp;`) are turned into their characters.
fn clean_text(lines: &[String]) -> String {
    let joined = lines.iter().map(|line| strip_markup(line)).collect::<Vec<_>>().join(" ");
    let text = one_line(&decode_entities(&joined));
    match text.char_indices().nth(MAX_CUE_CHARS) {
        Some((cut, _)) => text[..cut].trim_end().to_owned(),
        None => text,
    }
}

/// `line` without `<…>` tags and `{\…}` override codes. A `<` or `{` with no closing mark on
/// the line is ordinary text and stays.
fn strip_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(['<', '{']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let close = if tail.starts_with('<') { tail.find('>') } else if tail.starts_with("{\\") { tail.find('}') } else { None };
        match close {
            Some(end) => rest = &tail[end + 1..],
            None => {
                out.push_str(&tail[..1]);
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Turns HTML character references into characters: the named ones subtitles use, and
/// numbered ones (`&#233;`, `&#xE9;`). Anything else is left as written.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let decoded = tail.get(1..tail.find(';').filter(|&end| end <= 10).unwrap_or(0)).and_then(|name| {
            let character = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                "lrm" | "rlm" => return Some(None),
                _ => {
                    let code = match name.strip_prefix('#')? {
                        hex if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16).ok()?,
                        decimal => decimal.parse().ok()?,
                    };
                    char::from_u32(code).filter(|character| !character.is_control() || character.is_whitespace())?
                }
            };
            Some(Some(character))
        });
        match decoded {
            Some(character) => {
                out.extend(character);
                rest = &tail[tail.find(';').unwrap_or(0) + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text with every run of whitespace (line breaks included) made one space, and none at the ends.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Puts cues in time order and makes them fit one caption layer, which shows one phrase at a
/// time: cues that start together become one, each lasts at least a frame, and a cue still
/// showing when the next starts ends there.
fn tidy(mut cues: Vec<Cue>) -> Vec<Cue> {
    cues.sort_by_key(|cue| cue.start);
    let mut tidied: Vec<Cue> = Vec::with_capacity(cues.len());
    for mut cue in cues {
        cue.end = cue.end.max(cue.start + 1);
        match tidied.last_mut() {
            Some(previous) if previous.start == cue.start => {
                previous.text = format!("{} {}", previous.text, cue.text);
                previous.end = previous.end.max(cue.end);
            }
            Some(previous) => {
                previous.end = previous.end.min(cue.start);
                tidied.push(cue);
            }
            None => tidied.push(cue),
        }
    }
    tidied
}

/// The frame showing at `millis` milliseconds, to the nearest frame (exact for any frame rate).
fn millis_to_frame(millis: u64, rate: FrameRate) -> i64 {
    let (numerator, denominator) = (i128::from(rate.numerator.max(1)), i128::from(rate.denominator.max(1)) * 1000);
    ((i128::from(millis) * numerator * 2 + denominator) / (denominator * 2)) as i64
}

/// When frame `frame` (not negative) starts, to the nearest millisecond.
fn frame_to_millis(frame: i64, rate: FrameRate) -> u64 {
    let (numerator, denominator) = (i128::from(rate.numerator.max(1)), i128::from(rate.denominator.max(1)) * 1000);
    ((i128::from(frame.max(0)) * denominator * 2 + numerator) / (numerator * 2)) as u64
}

/// `millis` as `HH:MM:SS,mmm`, with `separator` before the milliseconds.
fn timestamp(millis: u64, separator: char) -> String {
    let (seconds, millis) = (millis / 1000, millis % 1000);
    format!("{:02}:{:02}:{:02}{separator}{millis:03}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::captions::CaptionStyle;

    const FPS_24: FrameRate = FrameRate::new(24, 1);
    const NTSC: FrameRate = FrameRate::new(30000, 1001);

    fn cue(start: i64, end: i64, text: &str) -> Cue {
        Cue { start, end, text: text.into() }
    }

    fn texts(read: &ReadCues) -> Vec<&str> {
        read.cues.iter().map(|cue| cue.text.as_str()).collect()
    }

    #[test]
    fn reads_a_plain_srt_file() {
        let file = "1\n00:00:01,000 --> 00:00:02,500\nHello there.\n\n2\n00:00:03,000 --> 00:00:04,000\nTwo\nlines\n\n";
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert_eq!(read.cues, vec![cue(24, 60, "Hello there."), cue(72, 96, "Two lines")]);
        assert_eq!(read.skipped, 0);
    }

    #[test]
    fn tolerates_bom_crlf_missing_blank_lines_and_extra_space() {
        let file = "\u{FEFF}1\r\n00:00:01,000-->00:00:02,000  \r\n  Spaced    out  \r\n2\r\n 00:00:02,000 -->   00:00:03,000 X1:10 X2:20\r\nNo blank line before me\r\n\r\n\r\n3\r\n00:00:04,000 --> 00:00:05,000\r\nNo trailing newline";
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert_eq!(texts(&read), ["Spaced out", "No blank line before me", "No trailing newline"]);
        assert_eq!((read.cues[1].start, read.cues[1].end), (48, 72));
        // Old Mac line endings too.
        let mac = "1\r00:00:01,000 --> 00:00:02,000\rCarriage returns\r";
        assert_eq!(texts(&super::read(mac.as_bytes(), FPS_24).unwrap()), ["Carriage returns"]);
    }

    #[test]
    fn strips_formatting_and_decodes_characters() {
        let file = "1\n00:00:01,000 --> 00:00:02,000\n{\\an8}<i>Leaning</i> <font color=\"#ff0\">yellow</font> &amp; <b>bold</b> &lt;3 caf&#233; 2 < 3\n";
        assert_eq!(texts(&read(file.as_bytes(), FPS_24).unwrap()), ["Leaning yellow & bold <3 café 2 < 3"]);
    }

    #[test]
    fn reads_webvtt_with_header_notes_settings_and_short_times() {
        let file = "WEBVTT - a title\n\nNOTE written by hand\nover two lines\n\nSTYLE\n::cue { color: red }\n\nintro\n00:01.500 --> 00:02.000 align:start position:10%\n<v Roger>Hi <c.loud>there</c>\n\n00:00:03.000 --> 00:00:04.250\n<00:00:03.500>Karaoke timing\n";
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert_eq!(texts(&read), ["Hi there", "Karaoke timing"]);
        assert_eq!((read.cues[0].start, read.cues[0].end), (36, 48));
        assert_eq!(read.cues[1].end, 102, "4.25 s at 24 fps");
    }

    #[test]
    fn reads_utf16_and_windows_1252_text() {
        let text = "1\n00:00:01,000 --> 00:00:02,000\n“Curly” — café\n";
        let mut little: Vec<u8> = vec![0xFF, 0xFE];
        little.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(texts(&read(&little, FPS_24).unwrap()), ["“Curly” — café"]);
        let mut big: Vec<u8> = vec![0xFE, 0xFF];
        big.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        assert_eq!(texts(&read(&big, FPS_24).unwrap()), ["“Curly” — café"]);
        let mut latin: Vec<u8> = b"1\n00:00:01,000 --> 00:00:02,000\n".to_vec();
        latin.extend([0x93, b'C', b'a', b'f', 0xE9, 0x94]);
        assert_eq!(texts(&read(&latin, FPS_24).unwrap()), ["“Café”"]);
    }

    #[test]
    fn leaves_out_cues_with_bad_times_and_says_how_many() {
        let file = "1\n00:00:01,000 --> 00:00:02,000\nGood\n\n2\n00:00:61,000 --> 00:00:62,000\nSixty-one seconds\n\n3\n00:00:05,000 --> 00:00:04,000\nEnds before it starts\n\n4\nab:cd:ef,ghi --> 00:00:09,000\nLetters\n\n5\n00:00:07,5 --> 00:00:08,25\nShort fraction\n\n6\n1000:00:00,000 --> 1000:00:01,000\nToo many hours\n";
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert_eq!(texts(&read), ["Good", "Short fraction"]);
        assert_eq!((read.cues[1].start, read.cues[1].end), (180, 198), "7.5 s and 8.25 s");
        assert_eq!(read.skipped, 4);
    }

    #[test]
    fn refuses_files_that_are_not_subtitles() {
        assert!(read(b"", FPS_24).unwrap_err().contains("no subtitles"));
        assert!(read(b"Dear diary,\n\nToday I wrote a letter.\n", FPS_24).unwrap_err().contains("no subtitles"));
        assert!(read(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR", FPS_24).unwrap_err().contains("isn't a text file"));
        assert!(read(b"1\nsoon --> later\nText\n", FPS_24).unwrap_err().contains("none of its 1"));
        let huge = vec![b' '; MAX_FILE_BYTES as usize + 1];
        assert!(read(&huge, FPS_24).unwrap_err().contains("too big"));
    }

    #[test]
    fn caps_how_much_is_kept() {
        let long = "word ".repeat(1000);
        let file = format!("1\n00:00:01,000 --> 00:00:02,000\n{long}\n");
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert!(read.cues[0].text.chars().count() <= MAX_CUE_CHARS);
        let many: String = (0..=MAX_CUES).map(|index| format!("00:00:01,000 --> 00:00:02,000\n{index}x\n\n")).collect();
        assert!(super::read(many.as_bytes(), FPS_24).unwrap_err().contains("more than"));
    }

    #[test]
    fn overlapping_and_simultaneous_cues_fit_one_layer() {
        let file = "00:00:03,000 --> 00:00:05,000\nLater\n\n00:00:01,000 --> 00:00:04,000\nOverlaps the next\n\n00:00:01,000 --> 00:00:02,000\nSame start\n\n00:00:06,000 --> 00:00:06,000\nZero length\n";
        let read = read(file.as_bytes(), FPS_24).unwrap();
        assert_eq!(read.cues, vec![cue(24, 72, "Overlaps the next Same start"), cue(72, 120, "Later"), cue(144, 145, "Zero length")]);
    }

    #[test]
    fn writes_srt_and_webvtt() {
        let cues = [cue(24, 60, "Hello <world> & more"), cue(-10, 12, "Cut at zero"), cue(-10, -2, "Before the start")];
        let srt = write(&cues, SubtitleFormat::Srt, FPS_24);
        assert_eq!(srt, "1\n00:00:01,000 --> 00:00:02,500\nHello <world> & more\n\n2\n00:00:00,000 --> 00:00:00,500\nCut at zero\n\n");
        let vtt = write(&cues[..1], SubtitleFormat::WebVtt, FPS_24);
        assert_eq!(vtt, "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello &lt;world&gt; &amp; more\n\n");
        assert_eq!(timestamp(3_723_004, ','), "01:02:03,004");
        // Text that looks like a timing line can't break the file.
        let tricky = write(&[cue(0, 24, "a --> b")], SubtitleFormat::Srt, FPS_24);
        assert_eq!(texts(&read(tricky.as_bytes(), FPS_24).unwrap()), ["a -> b"]);
    }

    #[test]
    fn frames_survive_milliseconds_at_any_rate() {
        for rate in [FPS_24, NTSC, FrameRate::new(24000, 1001), FrameRate::new(60, 1), FrameRate::new(25, 1)] {
            for frame in [0, 1, 2, 999, 1_000_001, 107_892] {
                assert_eq!(millis_to_frame(frame_to_millis(frame, rate), rate), frame, "frame {frame} at {rate:?}");
            }
        }
    }

    /// A caption layer made from speech, the way Generate Captions makes one.
    fn caption_layer(words: Vec<CaptionWord>, style: CaptionStyle) -> Layer {
        let mut project = crate::sample::sample_project();
        let comp = crate::sample::ids::MAIN_COMP;
        let id = crate::edit::add_caption_layer(&mut project, comp, words, style, 10..400, 10).unwrap();
        project.composition(comp).unwrap().layer(id).unwrap().clone()
    }

    #[test]
    fn exported_captions_import_to_the_same_cues_and_frames() {
        let spoken = ["So", "this", "is", "Studio.", "It", "edits", "video", "and", "makes", "captions", "too"];
        let words: Vec<CaptionWord> = spoken.iter().enumerate().map(|(index, text)| {
            // A long pause before "and", which starts a new phrase.
            let start = index as i64 * 7 + if index >= 7 { 40 } else { 0 };
            CaptionWord::new(*text, start, start + 6)
        }).collect();
        let style = CaptionStyle { offset: 3, ..CaptionStyle::default() };
        for rate in [FPS_24, NTSC] {
            let layer = caption_layer(words.clone(), style.clone());
            let cues = layer_cues(&layer, rate);
            assert_eq!(cues.iter().map(|cue| cue.text.as_str()).collect::<Vec<_>>(), ["So this is", "Studio.", "It edits video", "and makes captions", "too"]);
            for format in SubtitleFormat::ALL {
                let file = write(&cues, format, rate);
                let read = read(file.as_bytes(), rate).unwrap();
                assert_eq!(read.cues, cues, "{format:?} at {rate:?}");
                // As a caption layer of its own (timed from frame 0), it shows the same cues.
                let imported = Layer { start_frame: 0, in_frame: 0, out_frame: 1000, ..layer.clone() };
                let imported = Layer {
                    content: LayerContent::Captions { words: caption_words(&read.cues), style: CaptionStyle { words_per_phrase: words_per_phrase(&read.cues), ..CaptionStyle::default() } },
                    ..imported
                };
                assert_eq!(layer_cues(&imported, rate), cues, "{format:?} at {rate:?} as a layer");
            }
        }
    }

    #[test]
    fn imported_cues_show_whole_and_leave_on_time() {
        let cues = vec![cue(0, 30, "First line. Still the first"), cue(30, 40, "Second"), cue(100, 103, "one two three four five")];
        let words = caption_words(&cues);
        let style = CaptionStyle { words_per_phrase: words_per_phrase(&cues), uppercase: false, ..CaptionStyle::default() };
        assert_eq!(style.words_per_phrase, 5);
        // A full stop inside a cue doesn't split it, and a cue with no gap after it still ends.
        let shown = |frame| crate::captions::frame_at(&words, &style, frame, 24.0).map(|frame| frame.text());
        assert_eq!(shown(29).as_deref(), Some("First line. Still the first"));
        assert_eq!(shown(30).as_deref(), Some("Second"));
        assert_eq!(shown(40), None, "gone when the cue ends, without lingering");
        // More words than frames share frames, and the cue still ends on time.
        assert_eq!(words.iter().filter(|word| word.start >= 100).count(), 3);
        assert_eq!(shown(102).as_deref(), Some("one two three four five"));
        assert_eq!(shown(103), None);
    }
}
