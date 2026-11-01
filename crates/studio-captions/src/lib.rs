//! Automatic captions: speech in, timed words out, with OpenAI's Whisper "tiny" model run
//! locally through Candle (on the Mac's GPU where there is one). Nothing leaves the computer.
//!
//! The model is three files in one folder (see `MODEL_FILES`): its weights, its settings and
//! its vocabulary, as published at huggingface.co/openai/whisper-tiny. The app downloads them
//! once; this crate only reads them.
//!
//! Sound is decoded at 16 kHz mono and transcribed 30 seconds at a time, greedily, with
//! Whisper's timestamp rules, giving phrases with start and end times. Words get times by
//! sharing out their phrase's time by length, which is close enough to time captions by.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as whisper, Config, audio, model::Whisper};

/// The model's files, as named in its Hugging Face repository.
pub const MODEL_FILES: [&str; 3] = ["config.json", "tokenizer.json", "model.safetensors"];
/// Where each file can be downloaded from.
pub const MODEL_URL: &str = "https://huggingface.co/openai/whisper-tiny/resolve/main";
/// The sample rate Whisper listens at.
pub const SAMPLE_RATE: u32 = whisper::SAMPLE_RATE as u32;

/// Seconds per timestamp step in Whisper's output.
const TIME_STEP: f64 = 0.02;
/// The longest a phrase's text may run, in tokens (Whisper's own limit is half its context).
const MAX_TOKENS: usize = 224;

/// One spoken word and when it is said, in seconds from the start of the sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// A phrase as Whisper heard it.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Something that went wrong, in words for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptionError(pub String);

impl std::fmt::Display for CaptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<candle_core::Error> for CaptionError {
    fn from(error: candle_core::Error) -> Self {
        Self(format!("the speech model failed: {error}"))
    }
}

/// Whether all of the model's files are in `folder`.
pub fn model_present(folder: &Path) -> bool {
    MODEL_FILES.iter().all(|name| folder.join(name).is_file())
}

/// GPT-2's mapping from bytes to the characters its vocabulary spells them with, reversed:
/// vocabulary characters back to bytes.
fn byte_decoder() -> HashMap<char, u8> {
    let mut bytes: Vec<u32> = (u32::from('!')..=u32::from('~')).chain(u32::from('¡')..=u32::from('¬')).chain(u32::from('®')..=u32::from('ÿ')).collect();
    let mut chars = bytes.clone();
    let mut extra = 0;
    for byte in 0..256 {
        if !bytes.contains(&byte) {
            bytes.push(byte);
            chars.push(256 + extra);
            extra += 1;
        }
    }
    bytes.into_iter().zip(chars).filter_map(|(byte, ch)| Some((char::from_u32(ch)?, byte as u8))).collect()
}

/// Whisper's vocabulary, for turning token numbers back into text.
struct Vocabulary {
    tokens: Vec<String>,
    by_name: HashMap<String, u32>,
    bytes: HashMap<char, u8>,
}

impl Vocabulary {
    fn read(path: &Path) -> Result<Self, CaptionError> {
        let text = std::fs::read_to_string(path).map_err(|error| CaptionError(format!("could not read the vocabulary: {error}")))?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|error| CaptionError(format!("the vocabulary is damaged: {error}")))?;
        let mut by_name: HashMap<String, u32> = HashMap::new();
        if let Some(vocab) = json["model"]["vocab"].as_object() {
            for (name, id) in vocab {
                if let Some(id) = id.as_u64() {
                    by_name.insert(name.clone(), id as u32);
                }
            }
        }
        for added in json["added_tokens"].as_array().into_iter().flatten() {
            if let (Some(name), Some(id)) = (added["content"].as_str(), added["id"].as_u64()) {
                by_name.insert(name.to_owned(), id as u32);
            }
        }
        if by_name.is_empty() {
            return Err(CaptionError("the vocabulary has no tokens".into()));
        }
        let size = by_name.values().max().map_or(0, |id| *id as usize + 1);
        let mut tokens = vec![String::new(); size];
        for (name, &id) in &by_name {
            tokens[id as usize] = name.clone();
        }
        Ok(Self { tokens, by_name, bytes: byte_decoder() })
    }

    fn id(&self, name: &str) -> Result<u32, CaptionError> {
        self.by_name.get(name).copied().ok_or_else(|| CaptionError(format!("the vocabulary has no {name}")))
    }

    /// The text of ordinary (non-special) tokens.
    fn text(&self, ids: &[u32]) -> String {
        let bytes: Vec<u8> = ids
            .iter()
            .filter_map(|&id| self.tokens.get(id as usize))
            .flat_map(|token| token.chars())
            .filter_map(|ch| self.bytes.get(&ch).copied())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// The special tokens decoding needs.
struct Specials {
    start: u32,
    transcribe: u32,
    no_timestamps: u32,
    end: u32,
    /// The first timestamp token ("<|0.00|>"); every token from here up is a timestamp.
    timestamp_begin: u32,
    /// Language tokens ("<|en|>", "<|fr|>", ...), with their codes.
    languages: Vec<(u32, String)>,
}

/// The mel filter bank Whisper was trained with: 80 triangular filters over 201 frequency
/// bins, on the Slaney mel scale with Slaney area normalization (as librosa makes them).
fn mel_filters(mels: usize) -> Vec<f32> {
    let (sample_rate, fft) = (whisper::SAMPLE_RATE as f64, whisper::N_FFT);
    let bins = fft / 2 + 1;
    let (linear_step, log_start_hz) = (200.0 / 3.0, 1000.0);
    let log_start_mel = log_start_hz / linear_step;
    let log_step = 6.4f64.ln() / 27.0;
    let to_mel = |hz: f64| if hz < log_start_hz { hz / linear_step } else { log_start_mel + (hz / log_start_hz).ln() / log_step };
    let to_hz = |mel: f64| if mel < log_start_mel { mel * linear_step } else { log_start_hz * ((mel - log_start_mel) * log_step).exp() };
    let top = to_mel(sample_rate / 2.0);
    let edges: Vec<f64> = (0..mels + 2).map(|i| to_hz(top * i as f64 / (mels + 1) as f64)).collect();
    let frequencies: Vec<f64> = (0..bins).map(|i| sample_rate / 2.0 * i as f64 / (bins - 1) as f64).collect();
    let mut filters = vec![0.0f32; mels * bins];
    for mel in 0..mels {
        let (low, center, high) = (edges[mel], edges[mel + 1], edges[mel + 2]);
        let area = 2.0 / (high - low);
        for (bin, &frequency) in frequencies.iter().enumerate() {
            let rising = (frequency - low) / (center - low);
            let falling = (high - frequency) / (high - center);
            filters[mel * bins + bin] = (rising.min(falling).max(0.0) * area) as f32;
        }
    }
    filters
}

/// A loaded model, ready to transcribe.
pub struct Transcriber {
    model: Whisper,
    device: Device,
    filters: Vec<f32>,
    vocabulary: Vocabulary,
    specials: Specials,
    suppress: Vec<u32>,
}

impl Transcriber {
    /// Loads the model from `folder` (see `MODEL_FILES`), on the GPU where there is one.
    pub fn load(folder: &Path) -> Result<Self, CaptionError> {
        if !model_present(folder) {
            return Err(CaptionError("the captions model has not been downloaded yet".into()));
        }
        let read = |name: &str| std::fs::read(folder.join(name)).map_err(|error| CaptionError(format!("could not read {name}: {error}")));
        let config: Config = serde_json::from_slice(&read("config.json")?).map_err(|error| CaptionError(format!("the model settings are damaged: {error}")))?;
        let device = if cfg!(target_os = "macos") { Device::new_metal(0).unwrap_or(Device::Cpu) } else { Device::Cpu };
        let weights = VarBuilder::from_buffered_safetensors(read("model.safetensors")?, whisper::DTYPE, &device)?;
        let model = Whisper::load(&weights, config.clone())?;
        let vocabulary = Vocabulary::read(&folder.join("tokenizer.json"))?;
        let languages = vocabulary
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(id, token)| {
                let code = token.strip_prefix("<|")?.strip_suffix("|>")?;
                (2..=3).contains(&code.len()).then_some(())?;
                code.chars().all(|c| c.is_ascii_lowercase()).then(|| (id as u32, code.to_owned()))
            })
            .collect();
        let specials = Specials {
            start: vocabulary.id(whisper::SOT_TOKEN)?,
            transcribe: vocabulary.id(whisper::TRANSCRIBE_TOKEN)?,
            no_timestamps: vocabulary.id(whisper::NO_TIMESTAMPS_TOKEN)?,
            end: vocabulary.id(whisper::EOT_TOKEN)?,
            timestamp_begin: vocabulary.id("<|0.00|>")?,
            languages,
        };
        let suppress = config.suppress_tokens.clone();
        Ok(Self { filters: mel_filters(config.num_mel_bins), model, device, vocabulary, specials, suppress })
    }

    /// The 30 seconds of sound from `start` (samples) as the model's input: a log-mel
    /// spectrogram, padded with silence to the full window.
    fn window(&self, samples: &[f32], start: usize) -> Result<Tensor, CaptionError> {
        let mut chunk = samples[start..(start + whisper::N_SAMPLES).min(samples.len())].to_vec();
        chunk.resize(whisper::N_SAMPLES, 0.0);
        let mels = self.model.config.num_mel_bins;
        let mel = audio::pcm_to_mel(&self.model.config, &chunk, &self.filters);
        let frames = mel.len() / mels;
        let mel = Tensor::from_vec(mel, (1, mels, frames), &self.device)?;
        Ok(mel.narrow(2, 0, frames.min(whisper::N_FRAMES))?)
    }

    /// The language most likely spoken in the window, by asking the model which language
    /// token it expects first.
    fn detect_language(&mut self, features: &Tensor) -> Result<u32, CaptionError> {
        let tokens = Tensor::new(&[self.specials.start], &self.device)?.unsqueeze(0)?;
        let hidden = self.model.decoder.forward(&tokens, features, true)?;
        let logits = self.model.decoder.final_linear(&hidden.i((..1, 0..1))?)?.i(0)?.i(0)?.to_vec1::<f32>()?;
        self.specials
            .languages
            .iter()
            .max_by(|a, b| logits[a.0 as usize].total_cmp(&logits[b.0 as usize]))
            .map(|(id, _)| *id)
            .ok_or_else(|| CaptionError("the model knows no languages".into()))
    }

    /// The next token: the most likely one that Whisper's rules allow. Timestamps come in
    /// pairs around each phrase, never go back in time, and the first token is a timestamp.
    fn next_token(&self, logits: &mut [f32], generated: &[u32]) -> u32 {
        let begin = self.specials.timestamp_begin as usize;
        let block = |logits: &mut [f32], range: std::ops::Range<usize>| {
            let length = logits.len();
            logits[range.start.min(length)..range.end.min(length)].fill(f32::NEG_INFINITY);
        };
        for &token in &self.suppress {
            if let Some(logit) = logits.get_mut(token as usize) {
                *logit = f32::NEG_INFINITY;
            }
        }
        logits[self.specials.no_timestamps as usize] = f32::NEG_INFINITY;
        let is_time = |token: u32| token >= self.specials.timestamp_begin;
        match generated {
            // The first token is a timestamp no later than one second in.
            [] => {
                block(logits, 0..begin);
                block(logits, begin + 51..logits.len());
            }
            // A timestamp that opens a phrase (the first one, or one after a closing one) is
            // followed by text.
            [last] if is_time(*last) => block(logits, begin..logits.len()),
            [.., before, last] if is_time(*last) && is_time(*before) => block(logits, begin..logits.len()),
            // One that follows text closes the phrase: next comes a timestamp or the end.
            [.., last] if is_time(*last) => block(logits, 0..self.specials.end as usize),
            _ => {}
        }
        // Time never runs backward.
        if let Some(&latest) = generated.iter().rev().find(|&&token| is_time(token)) {
            block(logits, begin..latest as usize);
        }
        logits.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(self.specials.end, |(id, _)| id as u32)
    }

    /// Transcribes `samples` (16 kHz mono). `language` is a code such as "en", or `None` to
    /// detect it. Calls `progress` with the fraction done and stops early if `cancelled`.
    pub fn transcribe(
        &mut self,
        samples: &[f32],
        language: Option<&str>,
        progress: &mut dyn FnMut(f32),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Phrase>, CaptionError> {
        let mut phrases = Vec::new();
        let mut language_token = match language {
            Some(code) => Some(self.vocabulary.id(&format!("<|{code}|>"))?),
            None => None,
        };
        let mut start = 0;
        while start < samples.len() {
            if cancelled() {
                return Err(CaptionError("cancelled".into()));
            }
            let mel = self.window(samples, start)?;
            let features = self.model.encoder.forward(&mel, true)?;
            let language = match language_token {
                Some(token) => token,
                None => *language_token.insert(self.detect_language(&features)?),
            };
            let prompt = [self.specials.start, language, self.specials.transcribe];
            let mut tokens: Vec<u32> = prompt.to_vec();
            for step in 0..MAX_TOKENS {
                let input = Tensor::new(tokens.as_slice(), &self.device)?.unsqueeze(0)?;
                let hidden = self.model.decoder.forward(&input, &features, step == 0)?;
                let last = hidden.dim(1)? - 1;
                let mut logits = self.model.decoder.final_linear(&hidden.i((..1, last..))?)?.i(0)?.i(0)?.to_vec1::<f32>()?;
                let token = self.next_token(&mut logits, &tokens[prompt.len()..]);
                if token == self.specials.end {
                    break;
                }
                tokens.push(token);
            }
            let offset = start as f64 / f64::from(SAMPLE_RATE);
            phrases.extend(self.phrases(&tokens[prompt.len()..], offset));
            start += whisper::N_SAMPLES;
            progress((start as f32 / samples.len().max(1) as f32).min(1.0));
        }
        Ok(phrases)
    }

    /// The phrases in a window's tokens: text between a pair of timestamps, `offset` seconds in.
    fn phrases(&self, tokens: &[u32], offset: f64) -> Vec<Phrase> {
        let begin = self.specials.timestamp_begin;
        let mut phrases = Vec::new();
        let (mut opened, mut text) = (None::<f64>, Vec::new());
        for &token in tokens {
            if token >= begin {
                let time = offset + f64::from(token - begin) * TIME_STEP;
                match opened {
                    Some(start) if !text.is_empty() => {
                        let words = self.vocabulary.text(&text).trim().to_owned();
                        if !words.is_empty() {
                            phrases.push(Phrase { text: words, start, end: time.max(start) });
                        }
                        text.clear();
                        opened = None;
                    }
                    _ => opened = Some(time),
                }
            } else if token < self.specials.end {
                text.push(token);
            }
        }
        phrases
    }
}

/// Splits phrases into words, sharing each phrase's time among its words by their length.
pub fn words(phrases: &[Phrase]) -> Vec<Word> {
    let mut words = Vec::new();
    for phrase in phrases {
        let parts: Vec<&str> = phrase.text.split_whitespace().collect();
        let letters: usize = parts.iter().map(|part| part.chars().count().max(1)).sum();
        let mut at = phrase.start;
        for part in parts {
            let length = (phrase.end - phrase.start) * part.chars().count().max(1) as f64 / letters.max(1) as f64;
            words.push(Word { text: part.to_owned(), start: at, end: at + length });
            at += length;
        }
    }
    words
}

/// The sound of a media file between `start` and `end` seconds, at 16 kHz mono, for
/// `Transcriber::transcribe`.
pub fn read_speech(path: &Path, start: f64, end: f64) -> Result<Vec<f32>, CaptionError> {
    let mut decoder = studio_media::AudioDecoder::open(path, SAMPLE_RATE).map_err(|error| CaptionError(format!("could not read the sound: {error}")))?;
    let first = (start.max(0.0) * f64::from(SAMPLE_RATE)) as i64;
    let frames = ((end - start).max(0.0) * f64::from(SAMPLE_RATE)) as usize;
    decoder.seek(first).map_err(|error| CaptionError(format!("could not read the sound: {error}")))?;
    let mut stereo = vec![0.0f32; frames * studio_media::CHANNELS];
    let read = decoder.read(&mut stereo).map_err(|error| CaptionError(format!("could not read the sound: {error}")))?;
    Ok(stereo[..read * studio_media::CHANNELS].chunks(studio_media::CHANNELS).map(|pair| pair.iter().sum::<f32>() / pair.len() as f32).collect())
}

/// Where the model's files go in `folder`, for downloading.
pub fn model_paths(folder: &Path) -> Vec<(String, PathBuf)> {
    MODEL_FILES.iter().map(|name| (format!("{MODEL_URL}/{name}"), folder.join(name))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mel_filters_have_whispers_shape() {
        let filters = mel_filters(80);
        assert_eq!(filters.len(), 80 * 201);
        // Every filter has some weight, and the first one sits at the bottom of the spectrum.
        for mel in 0..80 {
            assert!(filters[mel * 201..(mel + 1) * 201].iter().any(|&w| w > 0.0), "filter {mel} is empty");
        }
        assert!(filters[1] > 0.0 && filters[50] == 0.0);
    }

    #[test]
    fn bytes_decode_like_gpt2() {
        let decoder = byte_decoder();
        assert_eq!(decoder.len(), 256);
        assert_eq!(decoder[&'Ġ'], b' ', "Ġ marks a space");
        assert_eq!(decoder[&'a'], b'a');
    }

    #[test]
    fn words_share_their_phrases_time() {
        let words = words(&[Phrase { text: " Hi there".into(), start: 1.0, end: 2.4 }]);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Hi");
        assert!((words[0].end - 1.4).abs() < 1e-9 && (words[1].end - 2.4).abs() < 1e-9);
    }
}
