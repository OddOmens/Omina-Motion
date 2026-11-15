//! Video scopes: pictures that measure the frame in the viewer, for judging exposure and color
//! by numbers instead of by eye. Each scope is drawn into a small image from the frame's pixels.
//!
//! - The histogram counts how many pixels have each brightness, per color channel.
//! - The waveform plots every pixel's brightness against where it is across the frame, so
//!   clipped highlights and crushed shadows show as flat lines at the top and bottom.
//! - The vectorscope plots every pixel's color: hue as the angle around the center, saturation
//!   as the distance from it. Gray sits in the middle.
//!
//! Pixels are BGRA bytes, the layout the viewer's images use.

/// Which scope to draw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScopeKind {
    #[default]
    Waveform,
    Histogram,
    Vectorscope,
}

impl ScopeKind {
    pub const ALL: [ScopeKind; 3] = [ScopeKind::Waveform, ScopeKind::Histogram, ScopeKind::Vectorscope];

    pub fn name(self) -> &'static str {
        match self {
            ScopeKind::Waveform => "Waveform",
            ScopeKind::Histogram => "Histogram",
            ScopeKind::Vectorscope => "Vectorscope",
        }
    }

    /// What the scope shows, for its tooltip.
    pub fn about(self) -> &'static str {
        match self {
            ScopeKind::Waveform => "Brightness against position across the frame: left to right matches the picture, bottom is black and top is white.",
            ScopeKind::Histogram => "How many pixels have each brightness, per color: black on the left, white on the right.",
            ScopeKind::Vectorscope => "Each pixel's color: hue as the angle, saturation as the distance from the center. The line marks skin tones.",
        }
    }
}

/// A scope drawn as an image: BGRA bytes, `width` × `height`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// The size scopes are drawn at, in pixels. They are stretched to the panel's width.
const SCOPE_WIDTH: usize = 256;
const SCOPE_HEIGHT: usize = 128;
/// About how many pixels of the frame are measured. Every pixel isn't needed for the shape of
/// a scope, and measuring fewer keeps it quick enough to follow playback.
const SAMPLES: usize = 120_000;
/// The scopes' background and guide lines.
const BACKGROUND: [u8; 4] = [0x14, 0x12, 0x12, 0xff];
const GUIDE: [u8; 4] = [0x48, 0x44, 0x44, 0xff];

/// A pixel's brightness as the eye weighs it (Rec. 709 luma), from 0 to 1.
fn luma(red: f32, green: f32, blue: f32) -> f32 {
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

/// Calls `visit` with a spread of the frame's pixels: each one's column and its red, green and
/// blue from 0 to 1.
fn sample(bgra: &[u8], width: usize, height: usize, mut visit: impl FnMut(usize, f32, f32, f32)) {
    if width == 0 || height == 0 || bgra.len() < width * height * 4 {
        return;
    }
    // Every `step`th pixel in both directions.
    let step = (((width * height) as f64 / SAMPLES as f64).sqrt().ceil() as usize).max(1);
    for row in (0..height).step_by(step) {
        for column in (0..width).step_by(step) {
            let pixel = &bgra[(row * width + column) * 4..][..4];
            visit(column, f32::from(pixel[2]) / 255.0, f32::from(pixel[1]) / 255.0, f32::from(pixel[0]) / 255.0);
        }
    }
}

/// An image filled with the scopes' background.
fn blank(width: usize, height: usize) -> Vec<u8> {
    BACKGROUND.repeat(width * height)
}

/// Adds `color` (blue, green, red) into the pixel at (`x`, `y`), brightening what is there.
fn add(image: &mut [u8], width: usize, x: usize, y: usize, color: [f32; 3]) {
    let pixel = &mut image[(y * width + x) * 4..][..3];
    for (channel, amount) in pixel.iter_mut().zip(color) {
        *channel = (f32::from(*channel) + amount).min(255.0) as u8;
    }
}

/// Draws a level guide line across the image at row `y`.
fn guide_row(image: &mut [u8], width: usize, y: usize) {
    for x in 0..width {
        image[(y * width + x) * 4..][..4].copy_from_slice(&GUIDE);
    }
}

/// How bright a dot is for `count` pixels landing on it, when the busiest dot has `most`:
/// a curve that keeps rare values visible beside common ones.
fn glow(count: u32, most: u32) -> f32 {
    if count == 0 || most == 0 {
        return 0.0;
    }
    (count as f32 / most as f32).powf(0.4)
}

/// The histogram: one curve per color channel, added together so that where all three agree
/// the curve is white.
fn histogram(bgra: &[u8], width: usize, height: usize) -> ScopeImage {
    let mut counts = [[0u32; SCOPE_WIDTH]; 3];
    sample(bgra, width, height, |_, red, green, blue| {
        for (channel, value) in [blue, green, red].into_iter().enumerate() {
            counts[channel][((value * 255.0).round() as usize).min(SCOPE_WIDTH - 1)] += 1;
        }
    });
    // Pure black and pure white are left out of the scale, so a clipped sky or a black
    // background doesn't flatten everything else.
    let most = counts.iter().flat_map(|channel| channel[1..SCOPE_WIDTH - 1].iter()).copied().max().unwrap_or(0).max(1);
    let mut image = blank(SCOPE_WIDTH, SCOPE_HEIGHT);
    for (channel, bins) in counts.iter().enumerate() {
        let mut color = [0.0; 3];
        color[channel] = 200.0;
        for (x, &count) in bins.iter().enumerate() {
            let tall = ((count as f32 / most as f32).min(1.0) * (SCOPE_HEIGHT - 1) as f32).round() as usize;
            for y in SCOPE_HEIGHT - 1 - tall..SCOPE_HEIGHT {
                add(&mut image, SCOPE_WIDTH, x, y, color);
            }
        }
    }
    ScopeImage { width: SCOPE_WIDTH as u32, height: SCOPE_HEIGHT as u32, bgra: image }
}

/// The waveform: brightness (up) against position across the frame.
fn waveform(bgra: &[u8], width: usize, height: usize) -> ScopeImage {
    let mut counts = vec![0u32; SCOPE_WIDTH * SCOPE_HEIGHT];
    sample(bgra, width, height, |column, red, green, blue| {
        let x = (column * SCOPE_WIDTH / width).min(SCOPE_WIDTH - 1);
        let level = (luma(red, green, blue).clamp(0.0, 1.0) * (SCOPE_HEIGHT - 1) as f32).round() as usize;
        counts[(SCOPE_HEIGHT - 1 - level) * SCOPE_WIDTH + x] += 1;
    });
    let most = counts.iter().copied().max().unwrap_or(0);
    let mut image = blank(SCOPE_WIDTH, SCOPE_HEIGHT);
    // Guides at 0, 25, 50, 75 and 100 percent brightness.
    for quarter in 0..=4 {
        guide_row(&mut image, SCOPE_WIDTH, (SCOPE_HEIGHT - 1) * quarter / 4);
    }
    for (index, &count) in counts.iter().enumerate() {
        let bright = glow(count, most) * 235.0;
        if bright > 0.0 {
            add(&mut image, SCOPE_WIDTH, index % SCOPE_WIDTH, index / SCOPE_WIDTH, [bright * 0.75, bright, bright * 0.75]);
        }
    }
    ScopeImage { width: SCOPE_WIDTH as u32, height: SCOPE_HEIGHT as u32, bgra: image }
}

/// Where a color lands on a vectorscope `side` pixels square: its blue and red color
/// differences (Rec. 709 Cb and Cr) across and up from the center.
fn vector_position(red: f32, green: f32, blue: f32, side: usize) -> (usize, usize) {
    let y = luma(red, green, blue);
    let (cb, cr) = ((blue - y) / 1.8556, (red - y) / 1.5748);
    let center = (side - 1) as f32 / 2.0;
    let place = |offset: f32| (center + offset * (side - 1) as f32).round().clamp(0.0, (side - 1) as f32) as usize;
    (place(cb), place(-cr))
}

/// The vectorscope: every pixel's color around the center, with a ring at full saturation's
/// half and a line along the hue skin tones share.
fn vectorscope(bgra: &[u8], width: usize, height: usize) -> ScopeImage {
    let side = SCOPE_HEIGHT;
    let mut counts = vec![0u32; side * side];
    sample(bgra, width, height, |_, red, green, blue| {
        let (x, y) = vector_position(red, green, blue, side);
        counts[y * side + x] += 1;
    });
    // The center (gray) is left out of the scale: most pictures are mostly gray-ish.
    let center = (side - 1) / 2;
    let most = counts.iter().enumerate().filter(|(index, _)| (index % side).abs_diff(center) > 1 || (index / side).abs_diff(center) > 1).map(|(_, &count)| count).max().unwrap_or(0).max(1);
    let mut image = blank(side, side);
    // A ring a quarter of the way out, and the skin tone line (about 123° on a vectorscope).
    let middle = (side - 1) as f32 / 2.0;
    for step in 0..720 {
        let angle = step as f32 * std::f32::consts::TAU / 720.0;
        let (x, y) = ((middle + angle.cos() * middle * 0.5).round() as usize, (middle + angle.sin() * middle * 0.5).round() as usize);
        image[(y * side + x) * 4..][..4].copy_from_slice(&GUIDE);
    }
    let skin = 123.0_f32.to_radians();
    for step in 0..side {
        let reach = step as f32 / side as f32 * middle;
        let (x, y) = ((middle + skin.cos() * reach).round() as usize, (middle - skin.sin() * reach).round() as usize);
        image[(y * side + x) * 4..][..4].copy_from_slice(&GUIDE);
    }
    for (index, &count) in counts.iter().enumerate() {
        let bright = glow(count.min(most), most) * 235.0;
        if bright > 0.0 {
            add(&mut image, side, index % side, index / side, [bright, bright, bright]);
        }
    }
    ScopeImage { width: side as u32, height: side as u32, bgra: image }
}

/// Draws scope `kind` for a frame of `width` × `height` BGRA pixels.
pub fn draw(kind: ScopeKind, bgra: &[u8], width: usize, height: usize) -> ScopeImage {
    match kind {
        ScopeKind::Waveform => waveform(bgra, width, height),
        ScopeKind::Histogram => histogram(bgra, width, height),
        ScopeKind::Vectorscope => vectorscope(bgra, width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of one color, as BGRA bytes.
    fn flat(width: usize, height: usize, [red, green, blue]: [u8; 3]) -> Vec<u8> {
        [blue, green, red, 255].repeat(width * height)
    }

    /// Whether the scope drew anything (brighter than its guides) at a pixel.
    fn lit(image: &ScopeImage, x: usize, y: usize) -> bool {
        let pixel = &image.bgra[(y * image.width as usize + x) * 4..][..3];
        pixel.iter().any(|&channel| channel > 0x80)
    }

    #[test]
    fn the_waveform_puts_white_at_the_top_and_black_at_the_bottom() {
        let white = draw(ScopeKind::Waveform, &flat(64, 36, [255, 255, 255]), 64, 36);
        assert!(lit(&white, 100, 0) && !lit(&white, 100, SCOPE_HEIGHT - 1));
        let black = draw(ScopeKind::Waveform, &flat(64, 36, [0, 0, 0]), 64, 36);
        assert!(lit(&black, 100, SCOPE_HEIGHT - 1) && !lit(&black, 100, 0));
    }

    #[test]
    fn the_waveform_follows_the_picture_from_left_to_right() {
        // Black on the left half, white on the right.
        let mut frame = flat(64, 36, [0, 0, 0]);
        for row in 0..36 {
            for column in 32..64 {
                frame[(row * 64 + column) * 4..][..3].copy_from_slice(&[255, 255, 255]);
            }
        }
        let scope = draw(ScopeKind::Waveform, &frame, 64, 36);
        assert!(lit(&scope, 40, SCOPE_HEIGHT - 1) && !lit(&scope, 40, 0), "the left is black");
        assert!(lit(&scope, 220, 0) && !lit(&scope, 220, SCOPE_HEIGHT - 1), "the right is white");
    }

    #[test]
    fn the_histogram_rises_where_the_pixels_are() {
        // Mid gray: one column in the middle, in all three colors (so it is white).
        let scope = draw(ScopeKind::Histogram, &flat(64, 36, [128, 128, 128]), 64, 36);
        assert!(lit(&scope, 128, SCOPE_HEIGHT / 2));
        assert!(!lit(&scope, 40, SCOPE_HEIGHT / 2) && !lit(&scope, 220, SCOPE_HEIGHT / 2));
    }

    #[test]
    fn the_vectorscope_puts_gray_in_the_middle_and_colors_away_from_it() {
        let side = SCOPE_HEIGHT;
        assert_eq!(vector_position(0.5, 0.5, 0.5, side), ((side - 1) / 2 + 1, (side - 1) / 2 + 1), "gray has no color (rounded from the exact center)");
        let (red_x, red_y) = vector_position(1.0, 0.0, 0.0, side);
        assert!(red_y < side / 2 && red_x < side / 2, "red sits up and to the left");
        let (blue_x, blue_y) = vector_position(0.0, 0.0, 1.0, side);
        assert!(blue_x > side / 2 && blue_y > side / 2, "blue sits right and down");
    }

    #[test]
    fn an_empty_or_short_frame_draws_a_blank_scope() {
        for kind in ScopeKind::ALL {
            let scope = draw(kind, &[], 0, 0);
            assert_eq!(scope.bgra.len(), (scope.width * scope.height * 4) as usize);
        }
    }
}
