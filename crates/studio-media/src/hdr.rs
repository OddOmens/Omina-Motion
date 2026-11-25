//! Showing HDR video (an iPhone's HLG clips, or PQ from other cameras) as ordinary video, the way
//! QuickTime and Photos show it on a normal screen. Without this, an HDR picture's numbers are
//! shown as if they were ordinary video: flat and washed out, with skin and reds off, because
//! they mean something else (a different brightness curve and wider BT.2020 colours).
//!
//! Each pixel is taken back to light (undoing the HLG or PQ curve), its BT.2020 colours mapped
//! into the ordinary BT.709 ones, the brightest highlights rolled off smoothly instead of clipped,
//! and the result encoded as ordinary video (BT.1886, gamma 2.4). Paper white (HLG's 75% level,
//! or 203 nits for PQ, as ITU-R BT.2408 defines it) lands at `PAPER_WHITE` of full brightness,
//! leaving room above it for highlights. Those settings were fitted to how macOS's own
//! AVFoundation (QuickTime) draws iPhone HLG video on a normal screen: on real iPhone footage the
//! result is within about 3 levels (of 255) of Apple's, pixel for pixel.

use ffmpeg_next::util::color::{Primaries, Space, TransferCharacteristic};

/// An HDR brightness curve this module converts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transfer {
    /// Hybrid Log-Gamma (ARIB STD-B67): iPhone HDR video, and broadcast HDR.
    Hlg,
    /// Perceptual Quantizer (SMPTE ST 2084): HDR10 and most other HDR cameras.
    Pq,
}

impl Transfer {
    /// The HDR curve a video uses, or `None` for ordinary video (shown as it is).
    pub fn of(transfer: TransferCharacteristic) -> Option<Transfer> {
        match transfer {
            TransferCharacteristic::ARIB_STD_B67 => Some(Transfer::Hlg),
            TransferCharacteristic::SMPTE2084 => Some(Transfer::Pq),
            _ => None,
        }
    }
}

/// Lookup tables are this many steps from 0 to 1: far finer than the 8-bit result.
const STEPS: usize = 4096;
/// Where paper white lands, as linear light (1.0 is full brightness): about 150 of 255 once
/// encoded, as Apple shows it.
const PAPER_WHITE: f32 = 0.28;
/// Where highlight roll-off starts, as linear light. Below it nothing changes.
const KNEE: f32 = 0.7;

/// Converts HDR pictures to ordinary video for one curve and set of primaries.
pub struct ToneMapper {
    /// Code value (0–1, in `STEPS`) → linear light, with paper white at `PAPER_WHITE`.
    to_linear: Vec<f32>,
    /// Linear light (0–1, in `STEPS`) → an 8-bit ordinary video value.
    to_video: Vec<u8>,
    /// Whether the colours are BT.2020 and need mapping into BT.709.
    wide_gamut: bool,
    /// The YUV formula's red and blue weights (Kr, Kb): BT.2020's for HDR, BT.709's otherwise.
    weights: (f32, f32),
    /// Whether the YUV uses the full 16-bit range rather than video range.
    full_range: bool,
}

/// HLG's curve, undone: a code value (0–1) to scene light (0–1). ITU-R BT.2100.
fn hlg_to_scene(value: f32) -> f32 {
    let (a, b, c) = (0.178_832_77_f32, 0.284_668_92_f32, 0.559_910_7_f32);
    if value <= 0.5 { value * value / 3.0 } else { (((value - c) / a).exp() + b) / 12.0 }
}

/// PQ's curve, undone: a code value (0–1) to light in nits (0–10,000). SMPTE ST 2084.
fn pq_to_nits(value: f32) -> f32 {
    let (m1, m2) = (0.159_301_76_f32, 78.843_75_f32);
    let (c1, c2, c3) = (0.835_937_5_f32, 18.851_562_f32, 18.6875_f32);
    let power = value.max(0.0).powf(1.0 / m2);
    10_000.0 * ((power - c1).max(0.0) / (c2 - c3 * power)).powf(1.0 / m1)
}

/// Rolls off light above `KNEE` so everything up to the brightest highlight fits under 1.0,
/// smoothly (no bend where it starts) and never going down.
fn roll_off(light: f32) -> f32 {
    if light <= KNEE { light } else { KNEE + (1.0 - KNEE) * (1.0 - (-(light - KNEE) / (1.0 - KNEE)).exp()) }
}

impl ToneMapper {
    /// A mapper for pictures with `transfer`, in BT.2020 colours unless `primaries` says BT.709,
    /// whose YUV uses formula `matrix` in full or video range.
    pub fn new(transfer: Transfer, primaries: Primaries, matrix: Space, full_range: bool) -> ToneMapper {
        // HLG's paper white is its 75% level, in scene light (Apple applies no further display gamma).
        let hlg_white = hlg_to_scene(0.75);
        let to_linear = (0..STEPS)
            .map(|step| {
                let value = step as f32 / (STEPS - 1) as f32;
                match transfer {
                    Transfer::Hlg => hlg_to_scene(value) / hlg_white * PAPER_WHITE,
                    Transfer::Pq => pq_to_nits(value) / 203.0 * PAPER_WHITE,
                }
            })
            .collect();
        let to_video = (0..STEPS).map(|step| ((step as f32 / (STEPS - 1) as f32).powf(1.0 / 2.4) * 255.0).round() as u8).collect();
        let weights = match matrix {
            Space::BT2020NCL | Space::BT2020CL => (0.2627, 0.0593),
            Space::SMPTE170M | Space::BT470BG => (0.299, 0.114),
            _ => (0.2126, 0.0722),
        };
        ToneMapper { to_linear, to_video, wide_gamut: primaries != Primaries::BT709, weights, full_range }
    }

    /// One pixel's 16-bit YUV to RGB code values (0–1), by the picture's formula and range.
    pub fn yuv_to_rgb(&self, y: u16, u: u16, v: u16) -> (f32, f32, f32) {
        let (luma, cb, cr) = if self.full_range {
            let scale = |value: u16| f32::from(value) / 65_535.0;
            (scale(y), scale(u) - 0.5, scale(v) - 0.5)
        } else {
            // Video range is the 8-bit levels times 256: black at 16, white at 235, no colour at
            // 128, colour reaching 16–240.
            let level = |value: u16| f32::from(value) / 256.0;
            ((level(y) - 16.0) / 219.0, (level(u) - 128.0) / 224.0, (level(v) - 128.0) / 224.0)
        };
        let (kr, kb) = self.weights;
        let kg = 1.0 - kr - kb;
        let red = luma + 2.0 * (1.0 - kr) * cr;
        let blue = luma + 2.0 * (1.0 - kb) * cb;
        let green = (luma - kr * red - kb * blue) / kg;
        (red.clamp(0.0, 1.0), green.clamp(0.0, 1.0), blue.clamp(0.0, 1.0))
    }

    fn index(value: f32) -> usize {
        (value.clamp(0.0, 1.0) * (STEPS - 1) as f32 + 0.5) as usize
    }

    /// One pixel: RGB code values (0–1) in, 8-bit ordinary video RGB out.
    pub fn pixel(&self, red: f32, green: f32, blue: f32) -> [u8; 3] {
        let linear = |code: f32| self.to_linear[Self::index(code)];
        let (mut r, mut g, mut b) = (linear(red), linear(green), linear(blue));
        if self.wide_gamut {
            // BT.2020 to BT.709 in linear light (ITU-R BT.2087). Colours outside BT.709 come out
            // below zero and are clipped there.
            (r, g, b) = (
                (1.660_491 * r - 0.587_641_1 * g - 0.072_849_9 * b).max(0.0),
                (-0.124_550_5 * r + 1.132_899_9 * g - 0.008_349_4 * b).max(0.0),
                (-0.018_150_8 * r - 0.100_578_9 * g + 1.118_729_7 * b).max(0.0),
            );
        }
        // Highlights roll off by the brightest channel, so a colour keeps its hue as it brightens.
        let brightest = r.max(g).max(b);
        if brightest > KNEE {
            let scale = roll_off(brightest) / brightest;
            (r, g, b) = (r * scale, g * scale, b * scale);
        }
        [self.to_video[Self::index(r)], self.to_video[Self::index(g)], self.to_video[Self::index(b)]]
    }

    /// Converts a 16-bit YUV picture with full-resolution colour (three planes, each row
    /// `strides` bytes apart, native byte order) to RGBA bytes, on every core.
    pub fn convert(&self, planes: [&[u8]; 3], strides: [usize; 3], width: usize, height: usize) -> Vec<u8> {
        let mut rgba = vec![255_u8; width * height * 4];
        let threads = std::thread::available_parallelism().map_or(1, |count| count.get()).clamp(1, 16);
        let rows_each = height.div_ceil(threads).max(1);
        std::thread::scope(|scope| {
            for (chunk_index, out) in rgba.chunks_mut(rows_each * width * 4).enumerate() {
                scope.spawn(move || {
                    for (row_in_chunk, out_row) in out.chunks_mut(width * 4).enumerate() {
                        let y = chunk_index * rows_each + row_in_chunk;
                        let sample = |plane: usize, x: usize| {
                            let at = y * strides[plane] + x * 2;
                            u16::from_ne_bytes([planes[plane][at], planes[plane][at + 1]])
                        };
                        for (x, out_pixel) in out_row.chunks_exact_mut(4).enumerate() {
                            let (r, g, b) = self.yuv_to_rgb(sample(0, x), sample(1, x), sample(2, x));
                            out_pixel[..3].copy_from_slice(&self.pixel(r, g, b));
                        }
                    }
                });
            }
        });
        rgba
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iphone() -> ToneMapper {
        ToneMapper::new(Transfer::Hlg, Primaries::BT2020, Space::BT2020NCL, false)
    }

    #[test]
    fn hlg_black_stays_black_and_its_75_percent_grey_is_paper_white() {
        let mapper = iphone();
        assert_eq!(mapper.pixel(0.0, 0.0, 0.0), [0, 0, 0]);
        let white = mapper.pixel(0.75, 0.75, 0.75);
        assert!(white.iter().all(|&channel| (145..=155).contains(&channel)), "paper white sits where Apple puts it: {white:?}");
        let full = mapper.pixel(1.0, 1.0, 1.0);
        assert!(full.iter().all(|&channel| (235..=255).contains(&channel)), "the brightest highlight is near full brightness: {full:?}");
    }

    #[test]
    fn brighter_code_values_never_come_out_darker() {
        for transfer in [Transfer::Hlg, Transfer::Pq] {
            let mapper = ToneMapper::new(transfer, Primaries::BT2020, Space::BT2020NCL, false);
            let mut last = 0;
            for step in 0..=200 {
                let grey = step as f32 / 200.0;
                let [value, ..] = mapper.pixel(grey, grey, grey);
                assert!(value >= last, "{transfer:?} at {step}: {value} after {last}");
                last = value;
            }
        }
    }

    #[test]
    fn greys_stay_grey_and_bt2020_red_becomes_a_strong_bt709_red() {
        let mapper = iphone();
        let [r, g, b] = mapper.pixel(0.5, 0.5, 0.5);
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1, "a grey has no colour cast: {r} {g} {b}");
        let [r, g, b] = mapper.pixel(0.6, 0.2, 0.15);
        assert!(r > 100 && g < r / 2 && b < r / 2, "a deep red stays red: {r} {g} {b}");
    }

    #[test]
    fn pq_paper_white_is_203_nits() {
        let mapper = ToneMapper::new(Transfer::Pq, Primaries::BT2020, Space::BT2020NCL, false);
        // 203 nits in PQ is a code value of about 0.58.
        let white = mapper.pixel(0.5806, 0.5806, 0.5806);
        assert!(white.iter().all(|&channel| (145..=155).contains(&channel)), "{white:?}");
    }

    #[test]
    fn video_range_yuv_becomes_the_right_rgb() {
        let mapper = iphone();
        // 16-bit video range: black at 16·256, white at 235·256, no colour at 128·256.
        let (r, g, b) = mapper.yuv_to_rgb(235 << 8, 128 << 8, 128 << 8);
        assert!([r, g, b].iter().all(|channel| (channel - 1.0).abs() < 0.01), "white: {r} {g} {b}");
        let (r, g, b) = mapper.yuv_to_rgb(16 << 8, 128 << 8, 128 << 8);
        assert!([r, g, b].iter().all(|channel| channel.abs() < 0.01), "black: {r} {g} {b}");
        // Strong Cr with no Cb is red, with green pulled down and blue left alone.
        let (r, g, b) = mapper.yuv_to_rgb(100 << 8, 128 << 8, 200 << 8);
        assert!(r > 0.6 && g < r && (b - (100.0 - 16.0) / 219.0).abs() < 0.02, "red: {r} {g} {b}");
    }

    #[test]
    fn planes_convert_to_rgba_with_their_strides() {
        let mapper = iphone();
        let (width, height) = (3, 2);
        let strides = [width * 2 + 4, width * 2 + 2, width * 2 + 6];
        let plane = |stride: usize, value: u16| {
            let mut bytes = vec![0_u8; stride * height];
            for y in 0..height {
                for x in 0..width {
                    bytes[y * stride + x * 2..][..2].copy_from_slice(&value.to_ne_bytes());
                }
            }
            bytes
        };
        // Video-range Y at HLG's 75% level, no colour.
        let luma = ((16.0 + 0.75 * 219.0) * 256.0) as u16;
        let (y, u, v) = (plane(strides[0], luma), plane(strides[1], 128 << 8), plane(strides[2], 128 << 8));
        let rgba = mapper.convert([&y, &u, &v], strides, width, height);
        assert_eq!(rgba.len(), width * height * 4);
        assert!(rgba.chunks_exact(4).all(|pixel| (145..=155).contains(&pixel[0]) && pixel[0].abs_diff(pixel[1]) <= 1 && pixel[3] == 255), "{rgba:?}");
    }
}
