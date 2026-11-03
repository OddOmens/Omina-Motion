//! Color grading helpers that need no GPU: lookup tables (LUTs) read from `.cube` files or
//! built in as named looks, and the white balance that makes a picked color neutral.
//!
//! A LUT maps a color to another through a cube of samples. Like most grading tools, Studio
//! feeds it display colors (sRGB encoded, 0 to 1), which is what `.cube` files from cameras,
//! Premiere and Resolve expect for Rec. 709 footage.

/// The most samples along a side of a 3D LUT (128 × 128 × 128 is about 25 MB as floats).
pub const MAX_3D_SIZE: usize = 128;
/// The most entries in a 1D LUT.
pub const MAX_1D_SIZE: usize = 65_536;
/// The longest `.cube` file read, in bytes. Big 3D LUTs (65 per side) are around 10 MB.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Samples per side when a 1D LUT or a built-in look is turned into a 3D cube.
const BUILT_SIZE: usize = 33;

/// A 3D lookup table: `size`³ output colors, red changing fastest, then green, then blue (the
/// order `.cube` files use and GPU 3D textures expect). Inputs between `domain_min` and
/// `domain_max` map across the cube.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut {
    pub title: String,
    pub size: usize,
    pub table: Vec<[f32; 3]>,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

impl Lut {
    /// A cube of `size` samples per side filled by `map` (inputs 0 to 1).
    pub fn from_fn(title: &str, size: usize, map: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        let step = 1.0 / (size - 1) as f32;
        let mut table = Vec::with_capacity(size * size * size);
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    table.push(map([r as f32 * step, g as f32 * step, b as f32 * step]));
                }
            }
        }
        Self { title: title.to_owned(), size, table, domain_min: [0.0; 3], domain_max: [1.0; 3] }
    }

    /// The table's color for `rgb`, blended between the eight nearest samples (as the GPU does).
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let last = (self.size - 1) as f32;
        // Where the color falls in the cube, in samples, kept inside it.
        let place: [f32; 3] = std::array::from_fn(|c| {
            let span = (self.domain_max[c] - self.domain_min[c]).max(f32::EPSILON);
            ((rgb[c] - self.domain_min[c]) / span).clamp(0.0, 1.0) * last
        });
        let low: [usize; 3] = std::array::from_fn(|c| (place[c].floor() as usize).min(self.size - 2));
        let t: [f32; 3] = std::array::from_fn(|c| place[c] - low[c] as f32);
        let at = |r: usize, g: usize, b: usize| self.table[r + g * self.size + b * self.size * self.size];
        let mut out = [0.0; 3];
        for corner in 0..8 {
            let pick = |axis: usize| corner >> axis & 1;
            let weight: f32 = (0..3).map(|axis| if pick(axis) == 1 { t[axis] } else { 1.0 - t[axis] }).product();
            let sample = at(low[0] + pick(0), low[1] + pick(1), low[2] + pick(2));
            for c in 0..3 {
                out[c] += sample[c] * weight;
            }
        }
        out
    }

    /// The table as `.cube` text, which other grading tools can open.
    pub fn to_cube(&self) -> String {
        let mut text = format!("TITLE \"{}\"\nLUT_3D_SIZE {}\n", self.title.replace('"', "'"), self.size);
        if self.domain_min != [0.0; 3] || self.domain_max != [1.0; 3] {
            let [a, b, c] = self.domain_min;
            let [d, e, f] = self.domain_max;
            text.push_str(&format!("DOMAIN_MIN {a} {b} {c}\nDOMAIN_MAX {d} {e} {f}\n"));
        }
        for [r, g, b] in &self.table {
            text.push_str(&format!("{r:.6} {g:.6} {b:.6}\n"));
        }
        text
    }
}

/// Reads a `.cube` LUT (the Adobe and Resolve text format), 3D or 1D. A 1D table is turned
/// into a cube, so both draw the same way. Errors say what is wrong in plain words.
pub fn parse_cube(text: &str) -> Result<Lut, String> {
    let mut title = String::new();
    let mut size_3d = None;
    let mut size_1d = None;
    let mut domain_min = [0.0f32; 3];
    let mut domain_max = [1.0f32; 3];
    let mut rows: Vec<[f32; 3]> = Vec::new();
    let numbers = |words: &[&str], line: usize| -> Result<Vec<f32>, String> {
        words.iter().map(|word| word.parse::<f32>().ok().filter(|value| value.is_finite()).ok_or_else(|| format!("line {line} has \"{word}\" where a number should be"))).collect()
    };
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let words: Vec<&str> = content.split_whitespace().collect();
        match words[0] {
            "TITLE" => title = content["TITLE".len()..].trim().trim_matches('"').to_owned(),
            "LUT_3D_SIZE" | "LUT_1D_SIZE" => {
                let size = words.get(1).and_then(|word| word.parse::<usize>().ok()).ok_or_else(|| format!("line {line} has no size"))?;
                if words[0] == "LUT_3D_SIZE" { size_3d = Some(size) } else { size_1d = Some(size) }
            }
            "DOMAIN_MIN" | "DOMAIN_MAX" => {
                let values = numbers(&words[1..], line)?;
                let [a, b, c] = values[..] else { return Err(format!("line {line} needs three numbers")) };
                if words[0] == "DOMAIN_MIN" { domain_min = [a, b, c] } else { domain_max = [a, b, c] }
            }
            // Resolve's way of giving one range for all three channels.
            "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE" => {
                let values = numbers(&words[1..], line)?;
                let [low, high] = values[..] else { return Err(format!("line {line} needs two numbers")) };
                (domain_min, domain_max) = ([low; 3], [high; 3]);
            }
            first if first.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.' || c == '+') => {
                let values = numbers(&words, line)?;
                let [r, g, b] = values[..] else { return Err(format!("line {line} needs three numbers")) };
                rows.push([r, g, b]);
            }
            // Keywords other tools add (LUT_IN_VIDEO_RANGE and so on) don't change the table.
            _ => {}
        }
    }
    if (0..3).any(|c| domain_max[c] <= domain_min[c]) {
        return Err("its input range is empty (DOMAIN_MAX must be above DOMAIN_MIN)".into());
    }
    match (size_3d, size_1d) {
        (Some(size), None) => {
            if !(2..=MAX_3D_SIZE).contains(&size) {
                return Err(format!("its size, {size}, is outside 2 to {MAX_3D_SIZE}"));
            }
            if rows.len() != size * size * size {
                return Err(format!("it should have {} colors but has {}", size * size * size, rows.len()));
            }
            Ok(Lut { title, size, table: rows, domain_min, domain_max })
        }
        (None, Some(size)) => {
            if !(2..=MAX_1D_SIZE).contains(&size) {
                return Err(format!("its size, {size}, is outside 2 to {MAX_1D_SIZE}"));
            }
            if rows.len() != size {
                return Err(format!("it should have {size} colors but has {}", rows.len()));
            }
            // Each channel looks itself up in its own column of the table.
            let curve = |channel: usize, value: f32| {
                let span = domain_max[channel] - domain_min[channel];
                let place = ((value - domain_min[channel]) / span).clamp(0.0, 1.0) * (size - 1) as f32;
                let low = (place.floor() as usize).min(size - 2);
                let t = place - low as f32;
                rows[low][channel] * (1.0 - t) + rows[low + 1][channel] * t
            };
            // The cube's inputs run across the 1D table's domain, so the cube keeps its range.
            let span: [f32; 3] = std::array::from_fn(|c| domain_max[c] - domain_min[c]);
            let mut lut = Lut::from_fn(&title, BUILT_SIZE, |rgb| std::array::from_fn(|c| curve(c, domain_min[c] + rgb[c] * span[c])));
            (lut.domain_min, lut.domain_max) = (domain_min, domain_max);
            Ok(lut)
        }
        (Some(_), Some(_)) => Err("it says it is both a 1D and a 3D table".into()),
        (None, None) => Err("it has no LUT_3D_SIZE or LUT_1D_SIZE line, so it is not a .cube LUT".into()),
    }
}

/// The looks Studio has built in, in groups as menus and the LUT browser show them. Each is a
/// LUT made from a few grading moves (see `look_color`).
pub const LOOK_GROUPS: &[(&str, &[&str])] = &[
    ("Cinematic", &["Teal & Orange", "Blockbuster", "Moody Green", "Desaturated Drama", "Bleach Bypass", "Day for Night", "Neon Night"]),
    ("Film", &["Warm Film", "Print Film", "Faded Matte", "Vintage", "Cross Process", "Instant Photo", "Super 8"]),
    ("Warm & Cool", &["Golden Hour", "Sunset", "Summer", "Autumn", "Desert", "Cool Night", "Arctic"]),
    ("Black & White", &["Noir", "Silver", "Sepia", "Selenium", "Infrared"]),
    ("Vivid & Soft", &["Vivid", "Crisp", "Pastel", "Dreamy", "Muted Earth"]),
];

/// Every built-in look, group by group.
pub fn looks() -> impl Iterator<Item = &'static str> {
    LOOK_GROUPS.iter().flat_map(|(_, names)| names.iter().copied())
}

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

fn luma(c: [f32; 3]) -> f32 {
    c[0] * LUMA[0] + c[1] * LUMA[1] + c[2] * LUMA[2]
}

/// Moves `c` toward (or past, above 1) its own gray.
fn saturate(c: [f32; 3], amount: f32) -> [f32; 3] {
    let y = luma(c);
    c.map(|v| y + (v - y) * amount)
}

/// An S-shaped contrast curve around middle gray; `amount` 0 leaves the color alone.
fn contrast(c: [f32; 3], amount: f32) -> [f32; 3] {
    c.map(|v| {
        let v = v.clamp(0.0, 1.0);
        let s = v * v * (3.0 - 2.0 * v);
        v + (s - v) * amount
    })
}

/// Adds `tint` to the shadows (weighted toward black) and `glow` to the highlights.
fn split_tone(c: [f32; 3], tint: [f32; 3], glow: [f32; 3]) -> [f32; 3] {
    let y = luma(c).clamp(0.0, 1.0);
    let (shadow, highlight) = ((1.0 - y) * (1.0 - y), y * y);
    std::array::from_fn(|i| c[i] + tint[i] * shadow + glow[i] * highlight)
}

/// Lifts the blacks to `floor` and pulls the whites down to `ceiling`.
fn fade(c: [f32; 3], floor: f32, ceiling: f32) -> [f32; 3] {
    c.map(|v| floor + v.clamp(0.0, 1.0) * (ceiling - floor))
}

/// Multiplies each channel by its own amount, as a color filter over the lens would.
fn filter(c: [f32; 3], gains: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| c[i] * gains[i])
}

/// Brightens the midtones (`power` below 1) or darkens them (above 1), keeping black and white.
fn midtones(c: [f32; 3], power: f32) -> [f32; 3] {
    c.map(|v| v.clamp(0.0, 1.0).powf(power))
}

/// Black and white from `y`, toned from `ink` in the shadows to `paper` in the highlights.
fn toned(y: f32, ink: [f32; 3], paper: [f32; 3]) -> [f32; 3] {
    let y = y.clamp(0.0, 1.0);
    std::array::from_fn(|i| y * (ink[i] + (paper[i] - ink[i]) * y))
}

/// What the built-in look called `name` does to a display color.
fn look_color(name: &str, c: [f32; 3]) -> [f32; 3] {
    let out = match name {
        // Cinematic
        "Teal & Orange" => {
            let toned = split_tone(c, [-0.10, 0.02, 0.10], [0.10, 0.03, -0.08]);
            contrast(saturate(toned, 1.1), 0.25)
        }
        "Blockbuster" => contrast(saturate(split_tone(c, [-0.14, 0.03, 0.14], [0.14, 0.04, -0.10]), 1.15), 0.4),
        "Moody Green" => fade(contrast(split_tone(saturate(c, 0.7), [-0.05, 0.05, 0.02], [0.02, 0.04, -0.04]), 0.3), 0.03, 1.0),
        "Desaturated Drama" => filter(contrast(saturate(c, 0.5), 0.55), [0.98, 1.0, 1.04]),
        "Bleach Bypass" => contrast(saturate(c, 0.45), 0.5),
        "Day for Night" => midtones(filter(saturate(c, 0.4), [0.45, 0.55, 0.8]), 1.2),
        "Neon Night" => contrast(saturate(split_tone(c, [0.08, -0.04, 0.14], [-0.06, 0.08, 0.10]), 1.3), 0.3),
        // Film
        "Warm Film" => fade(contrast(split_tone(c, [0.04, 0.01, -0.03], [0.06, 0.02, -0.05]), 0.15), 0.04, 0.96),
        "Print Film" => fade(saturate(filter(contrast(split_tone(c, [-0.03, 0.01, 0.03], [0.0; 3]), 0.35), [1.03, 1.0, 0.94]), 0.9), 0.02, 0.97),
        "Faded Matte" => fade(saturate(c, 0.8), 0.08, 0.92),
        "Vintage" => {
            let warm = split_tone(saturate(c, 0.75), [0.06, 0.03, -0.02], [0.05, 0.03, -0.06]);
            fade(warm, 0.06, 0.94)
        }
        // Each channel developed differently: red and green harder, blue lifted and squeezed.
        "Cross Process" => {
            let [r, g, b] = c;
            let curved = [contrast([r; 3], 0.5)[0], contrast([g; 3], 0.3)[0] * 1.05, 0.15 + b * 0.65];
            saturate(curved, 1.1)
        }
        "Instant Photo" => fade(saturate(split_tone(c, [-0.02, 0.03, 0.06], [0.05, 0.05, -0.03]), 0.85), 0.07, 0.95),
        "Super 8" => fade(saturate(contrast(filter(c, [1.08, 1.0, 0.82]), 0.3), 1.1), 0.05, 0.93),
        // Warm & Cool
        "Golden Hour" => saturate(split_tone(c, [0.05, 0.01, -0.04], [0.12, 0.06, -0.10]), 1.1),
        "Sunset" => saturate(split_tone(c, [0.06, -0.02, 0.06], [0.14, 0.02, -0.06]), 1.15),
        "Summer" => filter(saturate(midtones(c, 0.9), 1.2), [1.03, 1.02, 0.96]),
        "Autumn" => contrast(saturate(split_tone(filter(c, [1.08, 0.98, 0.85]), [0.03, 0.0, -0.03], [0.0; 3]), 1.1), 0.2),
        "Desert" => contrast(fade(saturate(filter(c, [1.1, 1.0, 0.8]), 0.85), 0.04, 1.0), 0.15),
        "Cool Night" => {
            let cool = saturate(c, 0.7).map(|v| v * 0.85);
            split_tone(cool, [-0.04, 0.0, 0.10], [-0.03, 0.0, 0.06])
        }
        "Arctic" => fade(midtones(filter(saturate(c, 0.75), [0.9, 0.98, 1.1]), 0.92), 0.03, 1.0),
        // Black & White
        "Noir" => {
            let y = luma(c);
            contrast([y, y, y], 0.6)
        }
        "Silver" => {
            let y = luma(c);
            fade(midtones(contrast([y; 3], 0.35), 0.95), 0.02, 0.98)
        }
        "Sepia" => toned(luma(c), [0.75, 0.55, 0.38], [1.06, 0.98, 0.84]),
        "Selenium" => contrast(toned(luma(c), [0.86, 0.88, 1.0], [0.98, 1.0, 1.04]), 0.4),
        // Infrared film sees leaves as bright and blue sky as dark: the gray is mostly green,
        // with red and blue taken away.
        "Infrared" => {
            let y = (1.6 * c[1] - 0.3 * c[0] - 0.5 * c[2]).clamp(0.0, 1.0);
            contrast(midtones([y; 3], 0.75), 0.4)
        }
        // Vivid & Soft
        "Vivid" => contrast(saturate(c, 1.35), 0.2),
        "Crisp" => filter(contrast(saturate(c, 1.1), 0.35), [1.0, 1.01, 1.03]),
        "Pastel" => fade(saturate(c, 0.65), 0.1, 1.0),
        "Dreamy" => split_tone(midtones(fade(saturate(c, 0.85), 0.06, 1.0), 0.85), [0.0; 3], [0.04, 0.0, 0.03]),
        "Muted Earth" => contrast(fade(filter(saturate(c, 0.6), [1.04, 1.0, 0.9]), 0.05, 0.95), 0.15),
        _ => c,
    };
    out.map(|v| v.clamp(0.0, 1.0))
}

/// The built-in look called `name` as a LUT, or `None` if there is no such look.
pub fn look(name: &str) -> Option<Lut> {
    looks().any(|look| look == name).then(|| Lut::from_fn(name, BUILT_SIZE, |c| look_color(name, c)))
}

/// How strongly Basic Correction's Temperature and Tint change the channels: at ±100 a channel
/// is multiplied or divided by 2 to the power of this. Kept in step with `effect.wgsl`.
pub const WHITE_BALANCE_STOPS: f32 = 0.5;

/// The change to Basic Correction's Temperature and Tint (each −100 to 100) that makes
/// `rgb`, a linear-light color that should be gray, come out neutral. Temperature warms
/// (more red, less blue); Tint goes toward magenta (less green). Black has no color to
/// judge by, so it gives no change.
pub fn neutralize(rgb: [f32; 3]) -> (f32, f32) {
    if rgb.iter().any(|&v| v <= 1e-4 || !v.is_finite()) {
        return (0.0, 0.0);
    }
    let [r, g, b] = rgb.map(f32::log2);
    let k = WHITE_BALANCE_STOPS;
    // With r·2^(k(t + n/2)) = g·2^(−kn) = b·2^(k(n/2 − t)), red and blue meeting gives t, then
    // red and green meeting gives n.
    let temperature = (b - r) / (2.0 * k);
    let tint = (g - r - k * temperature) / (1.5 * k);
    ((temperature * 100.0).clamp(-100.0, 100.0), (tint * 100.0).clamp(-100.0, 100.0))
}

/// The channel multipliers Basic Correction applies for `temperature` and `tint` (−100 to 100),
/// scaled so brightness stays the same.
pub fn white_balance_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let (t, n) = (temperature / 100.0, tint / 100.0);
    let k = WHITE_BALANCE_STOPS;
    let gains = [(k * (t + n * 0.5)).exp2(), (-k * n).exp2(), (k * (n * 0.5 - t)).exp2()];
    let brightness = luma(gains);
    gains.map(|gain| gain / brightness)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|c| (a[c] - b[c]).abs() < 1e-3)
    }

    #[test]
    fn an_identity_cube_round_trips_and_leaves_colors_alone() {
        let identity = Lut::from_fn("Identity", 9, |c| c);
        let read = parse_cube(&identity.to_cube()).unwrap();
        assert_eq!(read.size, 9);
        assert_eq!(read.title, "Identity");
        for color in [[0.0, 0.0, 0.0], [0.25, 0.5, 0.75], [1.0, 1.0, 1.0], [0.13, 0.91, 0.42]] {
            assert!(close(read.apply(color), color), "{color:?}");
        }
    }

    #[test]
    fn cube_files_are_read_with_comments_titles_and_domains() {
        let text = "# made by hand\nTITLE \"Swap\"\nLUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 1 1\n\
                    0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let lut = parse_cube(text).unwrap();
        assert_eq!(lut.title, "Swap");
        // This table swaps red and blue.
        assert!(close(lut.apply([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
        assert!(close(lut.apply([0.0, 0.5, 0.25]), [0.25, 0.5, 0.0]));
    }

    #[test]
    fn one_dimensional_tables_become_cubes() {
        // Inverts every channel.
        let lut = parse_cube("LUT_1D_SIZE 2\n1 1 1\n0 0 0\n").unwrap();
        assert_eq!(lut.size, BUILT_SIZE);
        assert!(close(lut.apply([0.2, 0.6, 1.0]), [0.8, 0.4, 0.0]));
    }

    #[test]
    fn broken_files_say_what_is_wrong() {
        assert!(parse_cube("hello").unwrap_err().contains("not a .cube"));
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 0\n").unwrap_err().contains("should have 8"));
        assert!(parse_cube("LUT_3D_SIZE 1000\n").unwrap_err().contains("outside"));
        assert!(parse_cube("LUT_3D_SIZE 2\n0 0 x\n").unwrap_err().contains("\"x\""));
        assert!(parse_cube("LUT_3D_SIZE 2\nDOMAIN_MIN 1 1 1\nDOMAIN_MAX 0 0 0\n").unwrap_err().contains("empty"));
    }

    #[test]
    fn every_look_is_made_and_changes_the_picture() {
        let mut seen = std::collections::HashSet::new();
        for name in looks() {
            assert!(seen.insert(name), "{name} is listed twice");
            let lut = look(name).unwrap();
            let skin = [0.8, 0.55, 0.45];
            assert!(!close(lut.apply(skin), skin), "{name} changes nothing");
            assert!(lut.table.iter().flatten().all(|v| (0.0..=1.0).contains(v)), "{name} leaves the range");
        }
        assert!(look("Nope").is_none());
        let noir = look("Noir").unwrap().apply([0.9, 0.2, 0.1]);
        assert!((noir[0] - noir[1]).abs() < 1e-3 && (noir[1] - noir[2]).abs() < 1e-3, "Noir is gray");
    }

    #[test]
    fn neutralizing_a_color_makes_it_gray() {
        for cast in [[0.6, 0.4, 0.3], [0.3, 0.4, 0.6], [0.4, 0.6, 0.4], [0.5, 0.3, 0.5], [0.2, 0.2, 0.2]] {
            let (temperature, tint) = neutralize(cast);
            let gains = white_balance_gains(temperature, tint);
            let fixed: [f32; 3] = std::array::from_fn(|c| cast[c] * gains[c]);
            assert!((fixed[0] - fixed[1]).abs() < 1e-3 && (fixed[1] - fixed[2]).abs() < 1e-3, "{cast:?} gave {fixed:?}");
        }
        // A blue cast is fixed by warming, a green one by going toward magenta.
        assert!(neutralize([0.3, 0.4, 0.6]).0 > 0.0);
        assert!(neutralize([0.4, 0.6, 0.4]).1 > 0.0);
        assert_eq!(neutralize([0.0, 0.0, 0.0]), (0.0, 0.0));
    }

    #[test]
    fn white_balance_keeps_brightness() {
        let gains = white_balance_gains(60.0, -30.0);
        assert!((luma(gains) - 1.0).abs() < 1e-4);
        assert!(gains[0] > gains[2], "warmer has more red than blue");
    }
}
