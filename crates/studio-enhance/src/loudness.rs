//! Loudness as people hear it, measured the broadcast way (ITU-R BS.1770 / EBU R128): the
//! signal is weighted like the ear ("K-weighting"), averaged in overlapping 400 ms blocks, and
//! quiet blocks (pauses) are left out so silence doesn't drag the number down. The result is
//! in LUFS; podcasts and social video aim for about -16.

use crate::filters::Biquad;

/// Builds the two K-weighting stages for 48 kHz: a shelf that lifts the treble the way the head
/// does, then a low cut. The coefficients are the ones ITU-R BS.1770 gives for 48 kHz.
fn k_weighting() -> [Biquad; 2] {
    [
        Biquad::from_coefficients(1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85, 1.0, -1.690_659_293_182_41, 0.732_480_774_215_85),
        Biquad::from_coefficients(1.0, -2.0, 1.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21),
    ]
}

/// The integrated loudness of `channels` (each a whole recording at 48 kHz), in LUFS. `None`
/// when there is nothing louder than the silence gate (-70 LUFS).
pub fn integrated(channels: &[&[f32]]) -> Option<f32> {
    const BLOCK: usize = 19_200; // 400 ms
    const STEP: usize = 4_800; // 100 ms: blocks overlap by three quarters
    let length = channels.iter().map(|channel| channel.len()).min()?;
    if length < BLOCK {
        return None;
    }
    // The squared, ear-weighted signal of each channel.
    let weighted: Vec<Vec<f32>> = channels
        .iter()
        .map(|channel| {
            let [mut shelf, mut low_cut] = k_weighting();
            channel[..length].iter().map(|&sample| low_cut.run(shelf.run(sample)).powi(2)).collect()
        })
        .collect();
    // Mean square per block, summed over channels (stereo channels weigh 1 each).
    let mut blocks = Vec::new();
    let mut start = 0;
    while start + BLOCK <= length {
        let power: f64 = weighted.iter().map(|squares| squares[start..start + BLOCK].iter().map(|&s| f64::from(s)).sum::<f64>() / BLOCK as f64).sum();
        blocks.push(power);
        start += STEP;
    }
    let loudness = |power: f64| -0.691 + 10.0 * power.max(1e-20).log10();
    let mean_loudness = |blocks: &mut dyn Iterator<Item = f64>| {
        let (sum, count) = blocks.fold((0.0, 0usize), |(sum, count), power| (sum + power, count + 1));
        (count > 0).then(|| loudness(sum / count as f64))
    };
    // Leave out silence, then anything 10 LU below what remains.
    let audible: Vec<f64> = blocks.into_iter().filter(|&power| loudness(power) > -70.0).collect();
    let relative_gate = mean_loudness(&mut audible.iter().copied())? - 10.0;
    mean_loudness(&mut audible.into_iter().filter(|&power| loudness(power) > relative_gate)).map(|lufs| lufs as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_1k_sine_measures_about_minus_three_lufs() {
        // The standard's own check: a 0 dBFS 1 kHz sine in both channels reads -3.01 LUFS... per
        // channel; two channels add 3 dB, so 0.0 LUFS.
        let sine: Vec<f32> = (0..48_000 * 3).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin()).collect();
        let mono = integrated(&[&sine]).unwrap();
        assert!((mono + 3.01).abs() < 0.1, "{mono}");
        let stereo = integrated(&[&sine, &sine]).unwrap();
        assert!(stereo.abs() < 0.1, "{stereo}");
        // Half the level is 6 dB quieter, and the silence after it is left out (all but the
        // three blocks that straddle the cut, which the standard counts: 0.22 dB).
        let half: Vec<f32> = sine.iter().map(|s| s * 0.5).chain(std::iter::repeat_n(0.0, 48_000 * 3)).collect();
        let measured = integrated(&[&half]).unwrap();
        assert!((measured + 9.25).abs() < 0.1, "{measured}");
        assert_eq!(integrated(&[&[0.0; 48_000]]), None);
    }
}
