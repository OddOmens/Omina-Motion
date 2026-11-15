//! Sums and conversions, so Iris can answer "1920 / 3", "15% of 240", "5 seconds in frames",
//! "8 inches in pixels" or "16:9 at 1080 wide" without anyone reaching for a calculator. The
//! app says what a frame and an inch are worth where it is (its frame rate and resolution);
//! a message can also say so itself ("120 frames at 24 fps in seconds").
//!
//! Only a message that is nothing but a sum or a conversion is answered here; anything with
//! other words in it is left for the app to read.

/// What the app's units are worth, where it knows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Units {
    /// Frames in a second, in the open composition.
    pub fps: Option<f64>,
    /// Pixels in an inch, in the open document.
    pub dpi: Option<f64>,
}

/// What a quantity measures.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    /// Time, with how many seconds one is worth.
    Time(f64),
    Frames,
    /// Printed length, with how many inches one is worth.
    Length(f64),
    Pixels,
}

/// The unit a word names, with its plural name for the answer.
fn unit(word: &str) -> Option<(Unit, &'static str)> {
    Some(match word {
        "ms" | "millisecond" | "milliseconds" => (Unit::Time(0.001), "ms"),
        "s" | "sec" | "secs" | "second" | "seconds" => (Unit::Time(1.0), "seconds"),
        "min" | "mins" | "minute" | "minutes" => (Unit::Time(60.0), "minutes"),
        "h" | "hr" | "hrs" | "hour" | "hours" => (Unit::Time(3600.0), "hours"),
        "f" | "fr" | "frame" | "frames" => (Unit::Frames, "frames"),
        "in" | "inch" | "inches" => (Unit::Length(1.0), "inches"),
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => (Unit::Length(1.0 / 2.54), "cm"),
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => (Unit::Length(1.0 / 25.4), "mm"),
        "pt" | "pts" | "point" | "points" => (Unit::Length(1.0 / 72.0), "points"),
        "px" | "pixel" | "pixels" => (Unit::Pixels, "pixels"),
        _ => return None,
    })
}

/// A number as it is shown: whole when it is, else up to three decimals without trailing
/// zeros, with thousands grouped.
fn shown(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let text = if rounded.fract() == 0.0 { format!("{rounded:.0}") } else { format!("{rounded:.3}").trim_end_matches('0').to_owned() };
    let (whole, fraction) = text.split_once('.').map_or((text.as_str(), None), |(whole, fraction)| (whole, Some(fraction)));
    let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |digits| ("-", digits));
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction}"),
        None => format!("{sign}{grouped}"),
    }
}

/// A value with its unit's name, singular for exactly one: "1 inch", "2.5 inches".
fn measured(value: f64, name: &str) -> String {
    let shown = shown(value);
    let name = if shown == "1" && name.len() > 2 { name.strip_suffix("es").filter(|_| name == "inches").or_else(|| name.strip_suffix('s')).unwrap_or(name) } else { name };
    format!("{shown} {name}")
}

/// A message as the pieces sums are made of: numbers, signs and words. "5s" is "5" "s",
/// "1,920" is one number, "16:9" is "16" ":" "9".
fn pieces(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut current = String::new();
    let characters: Vec<char> = text.chars().collect();
    for (index, &character) in characters.iter().enumerate() {
        let number_so_far = current.chars().next().is_some_and(|first| first.is_ascii_digit() || first == '.');
        let digit_next = characters.get(index + 1).is_some_and(char::is_ascii_digit);
        let keep = if character.is_ascii_digit() {
            current.is_empty() || number_so_far
        } else if character == '.' {
            (current.is_empty() || number_so_far) && digit_next && !current.contains('.')
        } else if character == ',' && number_so_far && digit_next {
            // A comma inside a number groups its thousands and is dropped.
            continue;
        } else {
            character.is_alphabetic() && !number_so_far
        };
        if keep {
            current.push(character);
            continue;
        }
        if !current.is_empty() {
            found.push(std::mem::take(&mut current));
        }
        if character.is_ascii_digit() || character.is_alphabetic() || character == '.' {
            current.push(character);
        } else if !character.is_whitespace() {
            found.push(character.to_string());
        }
    }
    if !current.is_empty() {
        found.push(current);
    }
    found
}

/// Reads a sum from `pieces` with the usual order: brackets, powers, times and divide, plus
/// and minus.
struct Sum<'a> {
    pieces: &'a [String],
    at: usize,
    /// Whether anything was actually worked out, so a number alone isn't taken for a sum.
    worked: bool,
}

impl Sum<'_> {
    fn peek(&self) -> Option<&str> {
        self.pieces.get(self.at).map(String::as_str)
    }

    fn next_is(&self, offset: usize, word: &str) -> bool {
        self.pieces.get(self.at + offset).is_some_and(|piece| piece == word)
    }

    fn added(&mut self) -> Option<f64> {
        let mut value = self.multiplied()?;
        loop {
            let sign = match self.peek() {
                Some("+" | "plus") => 1.0,
                Some("-" | "−" | "–" | "minus") => -1.0,
                _ => return Some(value),
            };
            self.at += 1;
            self.worked = true;
            // "200 + 10%" adds ten percent of the 200, as a calculator does.
            if let Some(percent) = self.peek().and_then(|piece| piece.parse::<f64>().ok()).filter(|_| self.next_is(1, "%") && !self.next_is(2, "of")) {
                self.at += 2;
                value += sign * value * percent / 100.0;
                continue;
            }
            value += sign * self.multiplied()?;
        }
    }

    fn multiplied(&mut self) -> Option<f64> {
        let mut value = self.raised()?;
        loop {
            let divide = match self.peek() {
                Some("*" | "x" | "×" | "·" | "times") => false,
                Some("/" | "÷" | "over") => true,
                Some("multiplied" | "divided") if self.next_is(1, "by") => {
                    let divide = self.peek() == Some("divided");
                    self.at += 1;
                    divide
                }
                _ => return Some(value),
            };
            self.at += 1;
            let other = self.raised()?;
            value = if divide { value / other } else { value * other };
            self.worked = true;
        }
    }

    fn raised(&mut self) -> Option<f64> {
        let base = self.single()?;
        if matches!(self.peek(), Some("^")) {
            self.at += 1;
            let power = self.raised()?;
            self.worked = true;
            return Some(base.powf(power));
        }
        Some(base)
    }

    fn single(&mut self) -> Option<f64> {
        match self.peek()? {
            "-" | "−" | "–" => {
                self.at += 1;
                Some(-self.single()?)
            }
            "(" => {
                self.at += 1;
                let value = self.added()?;
                (self.peek() == Some(")")).then(|| self.at += 1)?;
                Some(value)
            }
            piece => {
                let mut value: f64 = piece.parse().ok()?;
                self.at += 1;
                // "15% of 240". A percentage alone ("50%") is a number, not a sum.
                if self.peek() == Some("%") {
                    self.at += 1;
                    value /= 100.0;
                    if self.peek() == Some("of") {
                        self.at += 1;
                        value *= self.single()?;
                        self.worked = true;
                    }
                }
                Some(value)
            }
        }
    }
}

/// The value of a sum that takes up all of `pieces`. `None` when it isn't a sum, or is only a
/// number.
fn sum(pieces: &[String]) -> Option<f64> {
    let mut reader = Sum { pieces, at: 0, worked: false };
    let value = reader.added()?;
    (reader.at == pieces.len() && reader.worked).then_some(value)
}

/// A quantity at the start of `pieces`: a number and its unit, with how many pieces it took.
fn quantity(pieces: &[String]) -> Option<(f64, Unit, &'static str, usize)> {
    let value: f64 = pieces.first()?.parse().ok()?;
    let (unit, name) = unit(pieces.get(1)?)?;
    Some((value, unit, name, 2))
}

/// The words of a message that set a unit's worth ("at 24 fps", "at 300 dpi"), taken out of
/// `pieces`, with the units as they then stand.
fn stated_units(pieces: &mut Vec<String>, mut units: Units) -> Units {
    let mut index = 0;
    while index + 1 < pieces.len() {
        let rate = pieces[index].parse::<f64>().ok().filter(|rate| *rate > 0.0);
        let from = if index > 0 && matches!(pieces[index - 1].as_str(), "at" | "@") { index - 1 } else { index };
        match (rate, pieces[index + 1].as_str()) {
            (Some(rate), "fps") => units.fps = Some(rate),
            (Some(rate), "dpi" | "ppi") => units.dpi = Some(rate),
            _ => {
                index += 1;
                continue;
            }
        }
        pieces.drain(from..index + 2);
        index = from;
    }
    units
}

/// `value` of one unit as another, or why it can't be said.
fn convert(value: f64, from: Unit, to: Unit, units: Units) -> Result<f64, String> {
    let need_fps = || units.fps.ok_or_else(|| "I need a frame rate for that. Say it, as in “120 frames at 24 fps in seconds”.".to_owned());
    let need_dpi = || units.dpi.ok_or_else(|| "I need a resolution for that. Say it, as in “8 inches at 300 dpi in pixels”.".to_owned());
    let seconds = match from {
        Unit::Time(worth) => Some(value * worth),
        Unit::Frames => Some(value / need_fps()?),
        _ => None,
    };
    let inches = match from {
        Unit::Length(worth) => Some(value * worth),
        Unit::Pixels => Some(value / need_dpi()?),
        _ => None,
    };
    match (to, seconds, inches) {
        (Unit::Time(worth), Some(seconds), _) => Ok(seconds / worth),
        (Unit::Frames, Some(seconds), _) => Ok(seconds * need_fps()?),
        (Unit::Length(worth), _, Some(inches)) => Ok(inches / worth),
        (Unit::Pixels, _, Some(inches)) => Ok(inches * need_dpi()?),
        _ => Err("Those two measure different things, so one can't be turned into the other.".to_owned()),
    }
}

/// The largest whole number that divides both.
fn common_divisor(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { common_divisor(b, a % b) }
}

/// Answers `message` when it is a sum or a conversion and nothing else.
pub fn answer(message: &str, units: &Units) -> Option<String> {
    let mut text = message.trim().to_lowercase().replace('’', "'");
    for end in ["?", "=", "."] {
        text = text.trim_end_matches(end).trim().to_owned();
    }
    for start in ["/calc ", "what is ", "what's ", "whats ", "what are ", "calculate ", "calc ", "compute ", "work out ", "how much is ", "convert ", "="] {
        if let Some(rest) = text.strip_prefix(start) {
            text = rest.trim().to_owned();
            break;
        }
    }
    let mut pieces = pieces(&text);
    let units = stated_units(&mut pieces, *units);
    let words: Vec<&str> = pieces.iter().map(String::as_str).collect();

    // "Aspect ratio of 1920 x 1080" (also "by").
    if let ["aspect", "ratio", "of", width, "x" | "×" | "by", height] | ["ratio", "of", width, "x" | "×" | "by", height] = words.as_slice()
        && let (Ok(width), Ok(height)) = (width.parse::<u64>(), height.parse::<u64>())
        && width > 0
        && height > 0
    {
        let divisor = common_divisor(width, height);
        return Some(format!("{width} × {height} is {}:{}.", width / divisor, height / divisor));
    }
    // "16:9 at 1080 wide", "4:5 for 1350 tall".
    if let [across, ":", down, "at" | "for" | "with", side, which] = words.as_slice()
        && let (Ok(across), Ok(down), Ok(side)) = (across.parse::<f64>(), down.parse::<f64>(), side.parse::<f64>())
        && across > 0.0
        && down > 0.0
    {
        return match *which {
            "wide" | "width" | "across" => Some(format!("At {across}:{down}, {} wide is {} tall.", shown(side), shown(side * down / across))),
            "tall" | "high" | "height" => Some(format!("At {across}:{down}, {} tall is {} wide.", shown(side), shown(side * across / down))),
            _ => None,
        };
    }
    // "How many frames is 5 seconds", "how many pixels in 8 inches".
    if let ["how", "many", wanted, rest @ ..] = words.as_slice()
        && let Some((to, to_name)) = unit(wanted)
    {
        let rest = match rest {
            ["is" | "are" | "in" | "for" | "make" | "makes", rest @ ..] => rest,
            rest => rest,
        };
        let rest: Vec<String> = rest.iter().map(|word| (*word).to_owned()).collect();
        if let Some((value, from, from_name, _)) = quantity(&rest).filter(|found| found.3 == rest.len()) {
            return Some(match convert(value, from, to, units) {
                Ok(result) => format!("{} is {}.", measured(value, from_name), measured(result, to_name)),
                Err(why) => why,
            });
        }
    }
    // "5 seconds in frames", "8 in to px".
    if let Some((value, from, from_name, used)) = quantity(&pieces)
        && let [joiner, wanted] = &words[used..]
        && matches!(*joiner, "in" | "to" | "as" | "into")
        && let Some((to, to_name)) = unit(wanted)
    {
        return Some(match convert(value, from, to, units) {
            Ok(result) => format!("{} is {}.", measured(value, from_name), measured(result, to_name)),
            Err(why) => why,
        });
    }
    // A plain sum.
    let value = sum(&pieces)?;
    if !value.is_finite() {
        return Some("That divides by zero, so it has no answer.".to_owned());
    }
    Some(format!("{} = {}", text.trim(), shown(value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(message: &str) -> Option<String> {
        answer(message, &Units { fps: Some(30.0), dpi: Some(300.0) })
    }

    #[test]
    fn sums_follow_the_usual_order() {
        assert_eq!(ask("1920 / 3").as_deref(), Some("1920 / 3 = 640"));
        assert_eq!(ask("What is 2 + 3 * 4?").as_deref(), Some("2 + 3 * 4 = 14"));
        assert_eq!(ask("(2 + 3) × 4").as_deref(), Some("(2 + 3) × 4 = 20"));
        assert_eq!(ask("2^10").as_deref(), Some("2^10 = 1,024"));
        assert_eq!(ask("10 divided by 4").as_deref(), Some("10 divided by 4 = 2.5"));
        assert_eq!(ask("1,920 x 1,080").as_deref(), Some("1,920 x 1,080 = 2,073,600"));
        assert_eq!(ask("-5 + 2").as_deref(), Some("-5 + 2 = -3"));
        assert_eq!(ask("1 / 0").as_deref(), Some("That divides by zero, so it has no answer."));
    }

    #[test]
    fn percentages_work() {
        assert_eq!(ask("15% of 240").as_deref(), Some("15% of 240 = 36"));
        assert_eq!(ask("what's 200 + 10%").as_deref(), Some("200 + 10% = 220"));
        assert_eq!(ask("80 - 25%").as_deref(), Some("80 - 25% = 60"));
    }

    #[test]
    fn time_and_frames_convert_at_the_apps_frame_rate_or_a_stated_one() {
        assert_eq!(ask("5 seconds in frames").as_deref(), Some("5 seconds is 150 frames."));
        assert_eq!(ask("how many frames is 2.5s?").as_deref(), Some("2.5 seconds is 75 frames."));
        assert_eq!(ask("120 frames at 24 fps in seconds").as_deref(), Some("120 frames is 5 seconds."));
        assert_eq!(ask("90s to minutes").as_deref(), Some("90 seconds is 1.5 minutes."));
        let no_rate = answer("5 seconds in frames", &Units::default()).unwrap();
        assert!(no_rate.contains("frame rate"), "{no_rate}");
    }

    #[test]
    fn lengths_and_pixels_convert_at_the_documents_resolution() {
        assert_eq!(ask("8 inches in pixels").as_deref(), Some("8 inches is 2,400 pixels."));
        assert_eq!(ask("how many inches is 600 px").as_deref(), Some("600 pixels is 2 inches."));
        assert_eq!(ask("2.54 cm in inches").as_deref(), Some("2.54 cm is 1 inch."));
        assert_eq!(ask("10 in at 72 dpi in px").as_deref(), Some("10 inches is 720 pixels."));
        assert!(ask("5 seconds in pixels").unwrap().contains("different things"));
    }

    #[test]
    fn ratios_work_both_ways() {
        assert_eq!(ask("aspect ratio of 1920x1080").as_deref(), Some("1920 × 1080 is 16:9."));
        assert_eq!(ask("16:9 at 1080 wide").as_deref(), Some("At 16:9, 1,080 wide is 607.5 tall."));
        assert_eq!(ask("4:5 for 1350 tall").as_deref(), Some("At 4:5, 1,350 tall is 1,080 wide."));
    }

    #[test]
    fn anything_that_is_not_only_a_sum_is_left_alone() {
        for message in ["50%", "42", "-5", "make it 1920x1080", "start at 5s", "5 seconds", "fade in over 2 seconds", "1:05", "how many layers are there", "50% opacity", "shadows +30", "", "what is a keyframe"] {
            assert_eq!(ask(message), None, "{message}");
        }
    }
}
