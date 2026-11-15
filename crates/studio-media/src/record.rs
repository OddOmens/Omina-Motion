//! Records the microphone to a WAV file, for voiceovers.
//!
//! A recording runs on a thread of its own, which owns the input stream (sound streams must
//! stay on the thread that made them). The sound card's callback only hands each block of
//! samples to that thread, which writes them to the file, so the callback never waits on the
//! disk. The file is 16-bit PCM at the microphone's own sample rate and channel count.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample as _, SampleFormat, SizedSample, Stream, StreamConfig};

use crate::decode::MediaError;

/// Writes 16-bit PCM WAV: a header with the sizes left blank, the samples as they arrive, and
/// the sizes filled in at the end.
struct WavWriter<W: Write + Seek> {
    out: W,
    rate: u32,
    channels: u16,
    /// Samples written so far, counting every channel.
    samples: u64,
}

impl<W: Write + Seek> WavWriter<W> {
    /// The length of the header before the samples.
    const HEADER_BYTES: u32 = 44;

    fn new(mut out: W, rate: u32, channels: u16) -> std::io::Result<Self> {
        let block = channels * 2;
        out.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        out.write_all(&16u32.to_le_bytes())?; // The format chunk's length.
        out.write_all(&1u16.to_le_bytes())?; // PCM.
        out.write_all(&channels.to_le_bytes())?;
        out.write_all(&rate.to_le_bytes())?;
        out.write_all(&(rate * u32::from(block)).to_le_bytes())?; // Bytes a second.
        out.write_all(&block.to_le_bytes())?;
        out.write_all(&16u16.to_le_bytes())?; // Bits a sample.
        out.write_all(b"data\0\0\0\0")?;
        Ok(Self { out, rate, channels, samples: 0 })
    }

    /// Adds samples (interleaved, -1 to 1). Louder ones are clipped to full scale.
    fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
            self.out.write_all(&value.to_le_bytes())?;
        }
        self.samples += samples.len() as u64;
        Ok(())
    }

    /// Fills in the sizes and returns what was written to, with how long the recording is in
    /// seconds. A WAV file holds at most 4 GB; a longer recording's sizes are capped (it would
    /// be over six hours).
    fn finish(mut self) -> std::io::Result<(W, f64)> {
        let data_bytes = u32::try_from(self.samples * 2).unwrap_or(u32::MAX - Self::HEADER_BYTES);
        self.out.seek(SeekFrom::Start(4))?;
        self.out.write_all(&(data_bytes + Self::HEADER_BYTES - 8).to_le_bytes())?;
        self.out.seek(SeekFrom::Start(40))?;
        self.out.write_all(&data_bytes.to_le_bytes())?;
        self.out.flush()?;
        let seconds = self.samples as f64 / f64::from(self.channels.max(1)) / f64::from(self.rate.max(1));
        Ok((self.out, seconds))
    }
}

/// A microphone recording in progress. `finish` ends it and completes the file.
pub struct Recording {
    stop: Sender<()>,
    thread: Option<JoinHandle<Result<f64, MediaError>>>,
    /// The loudest sample since the meter last looked, as the bits of a non-negative `f32`.
    peak: Arc<AtomicU32>,
}

impl Recording {
    /// Starts recording the default microphone into a new file at `path`. Fails when there is
    /// no microphone, it can't be opened (permission was refused, say), or the file can't be made.
    pub fn start(path: &Path) -> Result<Self, MediaError> {
        let (stop, stopped) = mpsc::channel();
        let (ready, is_ready) = mpsc::channel();
        let peak = Arc::new(AtomicU32::new(0));
        let (path, thread_peak) = (path.to_owned(), peak.clone());
        let thread = std::thread::Builder::new()
            .name("voiceover".into())
            .spawn(move || record(&path, &stopped, &ready, &thread_peak))
            .map_err(|error| MediaError(format!("the recording could not start: {error}")))?;
        // Wait until the microphone is open (or could not be), so the caller knows at once.
        match is_ready.recv() {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread), peak }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(MediaError("the recording stopped before it began".into())),
        }
    }

    /// The loudest level heard since this was last called (1.0 is full scale), for a meter.
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
    }

    /// Stops recording and completes the file. Returns its length in seconds.
    pub fn finish(mut self) -> Result<f64, MediaError> {
        let _ = self.stop.send(());
        match self.thread.take().map(JoinHandle::join) {
            Some(Ok(result)) => result,
            _ => Err(MediaError("the recording thread stopped unexpectedly".into())),
        }
    }
}

impl Drop for Recording {
    /// A recording that is dropped without `finish` still stops and completes its file.
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens the microphone and the file, reports through `ready` whether that worked, then writes
/// what the microphone hears until told to stop. Returns the recording's length in seconds.
fn record(path: &Path, stopped: &Receiver<()>, ready: &Sender<Result<(), MediaError>>, peak: &Arc<AtomicU32>) -> Result<f64, MediaError> {
    let (blocks, heard) = mpsc::channel::<Vec<f32>>();
    let opened = open(path, blocks, peak);
    let (stream, mut writer) = match opened {
        Ok(opened) => {
            let _ = ready.send(Ok(()));
            opened
        }
        Err(error) => {
            let _ = ready.send(Err(MediaError(error.0.clone())));
            return Err(error);
        }
    };
    let write = |writer: &mut WavWriter<BufWriter<File>>, block: &[f32]| writer.write(block).map_err(|error| MediaError(format!("the recording could not be written ({error})")));
    loop {
        match heard.recv_timeout(Duration::from_millis(50)) {
            Ok(block) => write(&mut writer, &block)?,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if stopped.try_recv().is_ok() {
            break;
        }
    }
    // Close the microphone, then write whatever it had already handed over.
    drop(stream);
    while let Ok(block) = heard.try_recv() {
        write(&mut writer, &block)?;
    }
    writer.finish().map(|(_, seconds)| seconds).map_err(|error| MediaError(format!("the recording could not be completed ({error})")))
}

/// Opens the default microphone, sending what it hears to `blocks`, and the file to write.
fn open(path: &Path, blocks: Sender<Vec<f32>>, peak: &Arc<AtomicU32>) -> Result<(Stream, WavWriter<BufWriter<File>>), MediaError> {
    let device = cpal::default_host().default_input_device().ok_or_else(|| MediaError("no microphone was found".into()))?;
    let supported = device.default_input_config().map_err(|error| MediaError(format!("the microphone can't be used ({error})")))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let stream = match format {
        SampleFormat::F32 => listen::<f32>(&device, &config, blocks, peak),
        SampleFormat::I16 => listen::<i16>(&device, &config, blocks, peak),
        SampleFormat::I32 => listen::<i32>(&device, &config, blocks, peak),
        SampleFormat::U16 => listen::<u16>(&device, &config, blocks, peak),
        other => Err(MediaError(format!("the microphone uses an unsupported sample format ({other})"))),
    }?;
    stream.play().map_err(|error| MediaError(format!("the microphone could not be started ({error})")))?;
    let file = File::create(path).map_err(|error| MediaError(format!("{} could not be created ({error})", path.display())))?;
    let writer = WavWriter::new(BufWriter::new(file), config.sample_rate, config.channels).map_err(|error| MediaError(format!("the recording could not be written ({error})")))?;
    Ok((stream, writer))
}

/// Builds the input stream in the microphone's own sample type. Its callback turns each block
/// into plain numbers, notes the loudest for the meter, and hands the block on.
fn listen<T>(device: &cpal::Device, config: &StreamConfig, blocks: Sender<Vec<f32>>, peak: &Arc<AtomicU32>) -> Result<Stream, MediaError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let peak = peak.clone();
    let data_callback = move |data: &[T], _: &cpal::InputCallbackInfo| {
        let block: Vec<f32> = data.iter().map(|sample| f32::from_sample(*sample)).collect();
        let loudest = block.iter().fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
        peak.fetch_max(loudest.to_bits(), Ordering::Relaxed);
        // The recording thread is gone only once recording has stopped.
        let _ = blocks.send(block);
    };
    let error_callback = |error: cpal::Error| eprintln!("the microphone reported a problem: {error}");
    device.build_input_stream(*config, data_callback, error_callback, None).map_err(|error| MediaError(format!("the microphone can't be opened ({error})")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_wav_file_has_its_sizes_and_samples() {
        let mut writer = WavWriter::new(Cursor::new(Vec::new()), 48_000, 2).unwrap();
        // One second of stereo, with one sample louder than full scale.
        let mut samples = vec![0.5f32; 96_000];
        samples[0] = 2.0;
        writer.write(&samples).unwrap();
        let (out, seconds) = writer.finish().unwrap();
        let bytes = out.into_inner();
        assert!((seconds - 1.0).abs() < 1e-9);
        assert_eq!(bytes.len(), 44 + 96_000 * 2);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 96_000 * 2, "the data chunk's size");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 96_000 * 2, "the file's size after its first eight bytes");
        assert_eq!(i16::from_le_bytes(bytes[44..46].try_into().unwrap()), i16::MAX, "a too-loud sample is clipped");
        assert_eq!(i16::from_le_bytes(bytes[46..48].try_into().unwrap()), (0.5 * f32::from(i16::MAX)).round() as i16);
    }
}
