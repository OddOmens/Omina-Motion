//! Writing the cleaned copy: a WAV file of 32-bit float samples, which keeps every bit of the
//! result and which the app's decoder reads like any other sound file.

use std::io::Write;
use std::path::Path;

/// Writes `channels` (all the same length) at `rate` to `path`, interleaved. Written under a
/// temporary name and renamed when complete, so a half-written file is never picked up.
pub fn write_wav(path: &Path, channels: &[Vec<f32>], rate: u32) -> std::io::Result<()> {
    let count = channels.len().max(1) as u16;
    let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
    let data_bytes = (frames * usize::from(count) * 4) as u32;
    let partial = path.with_extension("partial.wav");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&partial)?);
    // RIFF header, then the format (3 = IEEE float), then the samples.
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&3u16.to_le_bytes())?;
    file.write_all(&count.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * u32::from(count) * 4).to_le_bytes())?;
    file.write_all(&(count * 4).to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    for frame in 0..frames {
        for channel in channels {
            file.write_all(&channel[frame].to_le_bytes())?;
        }
    }
    file.into_inner().map_err(|error| error.into_error())?.sync_all()?;
    std::fs::rename(&partial, path)
}

/// Reads a 32-bit float WAV file as written by `write_wav` (or by most tools that write float
/// WAV): its channels and sample rate.
pub fn read_float_wav(path: &Path) -> std::io::Result<(Vec<Vec<f32>>, u32)> {
    let bytes = std::fs::read(path)?;
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a 32-bit float WAV file");
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad());
    }
    let (mut channels, mut rate, mut at) = (0usize, 0u32, 12usize);
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().map_err(|_| bad())?) as usize;
        let body = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => {
                channels = usize::from(u16::from_le_bytes([body[2], body[3]]));
                rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                if u16::from_le_bytes([body[14], body[15]]) != 32 {
                    return Err(bad());
                }
            }
            b"data" if channels > 0 => {
                let samples: Vec<f32> = body.as_chunks::<4>().0.iter().map(|&bytes| f32::from_le_bytes(bytes)).collect();
                let split = (0..channels).map(|c| samples.iter().skip(c).step_by(channels).copied().collect()).collect();
                return Ok((split, rate));
            }
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_file_reads_back_the_same() {
        let path = std::env::temp_dir().join(format!("studio-enhance-wav-{}.wav", std::process::id()));
        let channels = vec![vec![0.0, 0.5, -0.25], vec![1.0, -1.0, 0.125]];
        write_wav(&path, &channels, 48_000).unwrap();
        assert_eq!(read_float_wav(&path).unwrap(), (channels, 48_000));
        std::fs::remove_file(&path).ok();
    }
}
