//! Automatic captions: speech in, timed words out, with OpenAI's Whisper "tiny" model run
//! locally through Candle (on the Mac's GPU where there is one). Nothing leaves the computer.
//!
//! The model is three files in one folder (see `MODEL_FILES`): its weights, its settings and
//! its vocabulary, as published at huggingface.co/openai/whisper-tiny. The app downloads them
//! once; this crate only reads them.
//!
//! Sound is decoded at 16 kHz mono and transcribed 30 seconds at a time, greedily, with
//! Whisper's timestamp rules, giving phrases with start and end times. Those times are loose
//! (the small model often gives one phrase for several sentences, pauses included), so words
//! are timed against the sound itself: each phrase's words share out the time speech is
//! actually heard in, by length, skipping silences, and the pauses between sentences are
//! matched to the gaps between words (see `words`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as whisper, Config, audio, model::Whisper};

/// The model's files, as named in its Hugging Face repository, with the SHA-256 of each at
/// `MODEL_URL`'s revision, so a download can be checked before it is used.
pub const MODEL_FILES: [(&str, &str); 3] = [
    ("config.json", "ffdccec4f3211f4c63310f2b7098f309fe70f3952cedc5e4d11e43f5b2379b98"),
    ("tokenizer.json", "27fc476bfe7f17299480be2273fc0608e4d5a99aba2ab5dec5374b4482d1a566"),
    ("model.safetensors", "7ebd0e69e78190ffe1438491fa05cc1f5c1aa3a4c4db3bc1723adbb551ea2395"),
];
/// Where each file can be downloaded from: one fixed revision of the repository, so the files
/// can't change under their checksums.
pub const MODEL_URL: &str = "https://huggingface.co/openai/whisper-tiny/resolve/169d4a4341b33bc18d8881c4b69c2e104e1cc0af";
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
    MODEL_FILES.iter().all(|(name, _)| folder.join(name).is_file())
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

/// How finely the sound is checked for speech, in samples (10 ms).
const VOICE_FRAME: usize = 160;
/// Silences shorter than this, in seconds, are part of speech (between syllables and words).
const VOICE_BRIDGE: f64 = 0.15;
/// Sounds shorter than this, in seconds, are clicks rather than speech.
const VOICE_MIN: f64 = 0.05;
/// A silence at least this long, in seconds, is a pause: a word starts after it.
const PAUSE: f64 = 0.25;
/// How far before its phrase's start, and after its end, speech is looked for, in seconds:
/// Whisper's phrase times can be off by about this much.
const LEAD: f64 = 1.0;
const TAIL: f64 = 0.5;

/// Where speech is heard in `samples` (16 kHz mono), as (start, end) seconds: stretches
/// clearly louder than the recording's background noise.
pub fn voiced(samples: &[f32]) -> Vec<(f64, f64)> {
    let loudness: Vec<f32> = samples.chunks(VOICE_FRAME).map(|chunk| (chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32).sqrt()).collect();
    if loudness.is_empty() {
        return Vec::new();
    }
    let mut sorted = loudness.clone();
    sorted.sort_by(f32::total_cmp);
    let at = |fraction: f32| sorted[((sorted.len() - 1) as f32 * fraction) as usize];
    let (noise, peak) = (at(0.1), at(0.95));
    let threshold = (noise * 3.0).max(noise + (peak - noise) * 0.1).max(1e-4);
    let seconds = |frame: usize| (frame * VOICE_FRAME) as f64 / f64::from(SAMPLE_RATE);
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut start = None;
    for (index, &level) in loudness.iter().chain(std::iter::once(&0.0)).enumerate() {
        match (level > threshold, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                runs.push((seconds(from), seconds(index)));
                start = None;
            }
            _ => {}
        }
    }
    // Join stretches with only a short silence between them, then drop clicks.
    let mut joined: Vec<(f64, f64)> = Vec::new();
    for run in runs {
        match joined.last_mut() {
            Some(last) if run.0 - last.1 < VOICE_BRIDGE => last.1 = run.1,
            _ => joined.push(run),
        }
    }
    joined.retain(|(start, end)| end - start >= VOICE_MIN);
    joined
}

/// Times along a phrase's speech: the voiced stretches it covers, and how much speech comes
/// before each, so a point partway through the speech can be found in time.
struct SpeechTime {
    stretches: Vec<(f64, f64)>,
    before: Vec<f64>,
    total: f64,
}

impl SpeechTime {
    fn new(stretches: Vec<(f64, f64)>) -> Self {
        let mut before = Vec::with_capacity(stretches.len());
        let mut total = 0.0;
        for (start, end) in &stretches {
            before.push(total);
            total += end - start;
        }
        Self { stretches, before, total }
    }

    /// The moment `amount` seconds of speech in: a word starting right at a pause starts after it.
    fn start_at(&self, amount: f64) -> f64 {
        let index = self.before.iter().rposition(|&before| before <= amount + 1e-9).unwrap_or(0);
        let index = if index + 1 < self.stretches.len() && amount >= self.before[index + 1] - 1e-9 { index + 1 } else { index };
        self.stretches[index].0 + (amount - self.before[index]).max(0.0)
    }

    /// The moment `amount` seconds of speech in: a word ending right at a pause ends before it.
    fn end_at(&self, amount: f64) -> f64 {
        let index = self.before.iter().rposition(|&before| before < amount - 1e-9).unwrap_or(0);
        (self.stretches[index].0 + (amount - self.before[index])).min(self.stretches[index].1)
    }

    /// How much speech comes before each pause (a silence of at least `PAUSE`).
    fn pauses(&self) -> Vec<f64> {
        (1..self.stretches.len()).filter(|&index| self.stretches[index].0 - self.stretches[index - 1].1 >= PAUSE).map(|index| self.before[index]).collect()
    }
}

/// Splits phrases into words timed to the speech in `samples`, the 16 kHz mono sound they
/// were heard in. Each phrase's words share out the time its speech is heard in, by length,
/// skipping silences; then each pause is matched to the gap between words nearest it, so the
/// words after a pause start when speech starts again. A phrase with no speech found keeps
/// Whisper's times.
pub fn words(phrases: &[Phrase], samples: &[f32]) -> Vec<Word> {
    let speech = voiced(samples);
    let mut words = Vec::new();
    let mut looked_until = 0.0f64;
    for (index, phrase) in phrases.iter().enumerate() {
        let parts: Vec<&str> = phrase.text.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let next_start = phrases.get(index + 1).map_or(f64::INFINITY, |next| next.start);
        let window = ((phrase.start - LEAD).max(looked_until), (phrase.end + TAIL).min(next_start.max(phrase.end)));
        looked_until = window.1;
        let stretches: Vec<(f64, f64)> = speech.iter().map(|&(start, end)| (start.max(window.0), end.min(window.1))).filter(|(start, end)| end > start).collect();
        // Each word's share: its letters, plus one so short words aren't squeezed to nothing.
        let shares: Vec<f64> = parts.iter().map(|part| part.chars().count() as f64 + 1.0).collect();
        let total_share: f64 = shares.iter().sum();
        if stretches.is_empty() {
            let mut at = phrase.start;
            for (part, share) in parts.iter().zip(&shares) {
                let length = (phrase.end - phrase.start) * share / total_share;
                words.push(Word { text: (*part).to_owned(), start: at, end: at + length });
                at += length;
            }
            continue;
        }
        let time = SpeechTime::new(stretches);
        // Where each word starts and ends, in seconds of speech, by its share.
        let mut bounds: Vec<f64> = std::iter::once(0.0).chain(shares.iter().scan(0.0, |sum, share| {
            *sum += share;
            Some(*sum / total_share * time.total)
        })).collect();
        // People pause at the ends of sentences, and often at commas: those gaps match a pause
        // more readily.
        let likeliness: Vec<f64> = std::iter::once(1.0)
            .chain(parts.iter().map(|part| match part.chars().last() {
                Some('.' | '?' | '!' | ':' | ';') => 0.3,
                Some(',') => 0.6,
                _ => 1.0,
            }))
            .collect();
        snap_to_pauses(&mut bounds, &likeliness, &time.pauses());
        for (word, part) in parts.iter().enumerate() {
            let (start, end) = (time.start_at(bounds[word]), time.end_at(bounds[word + 1]));
            words.push(Word { text: (*part).to_owned(), start, end: end.max(start) });
        }
    }
    words
}

/// Moves word boundaries (in seconds of speech, first 0 and last the total) onto the pauses:
/// for each pause in turn, the nearest boundary not already used, keeping their order, with
/// the boundaries between two placed ones spaced as they were. A boundary's distance counts
/// for its `likeliness` times as much (lower after a sentence ends), so pauses go there first.
fn snap_to_pauses(bounds: &mut [f64], likeliness: &[f64], pauses: &[f64]) {
    let last = bounds.len() - 1;
    let original = bounds.to_vec();
    // (boundary index, where it goes), starting and ending with the ends of the phrase.
    let mut anchors = vec![(0usize, 0.0)];
    for &pause in pauses {
        let after = anchors.last().map_or(0, |&(index, _)| index) + 1;
        let cost = |index: usize| (original[index] - pause).abs() * likeliness[index];
        let nearest = (after..last).min_by(|&a, &b| cost(a).total_cmp(&cost(b)));
        if let Some(index) = nearest {
            anchors.push((index, pause));
        }
    }
    anchors.push((last, original[last]));
    for pair in anchors.windows(2) {
        let ((from, from_at), (to, to_at)) = (pair[0], pair[1]);
        let span = (original[to] - original[from]).max(1e-9);
        for index in from..=to {
            bounds[index] = from_at + (original[index] - original[from]) / span * (to_at - from_at);
        }
    }
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

/// For downloading: each of the model's files' URL, where it goes in `folder`, and its SHA-256.
pub fn model_paths(folder: &Path) -> Vec<(String, PathBuf, &'static str)> {
    MODEL_FILES.iter().map(|(name, sha256)| (format!("{MODEL_URL}/{name}"), folder.join(name), *sha256)).collect()
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

    /// `seconds` of sound at 16 kHz: a 200 Hz hum during each of `bursts` (start, end), quiet
    /// noise elsewhere.
    fn sound(seconds: f64, bursts: &[(f64, f64)]) -> Vec<f32> {
        (0..(seconds * 16_000.0) as usize)
            .map(|index| {
                let t = index as f64 / 16_000.0;
                let noise = ((index * 7919 % 101) as f32 / 101.0 - 0.5) * 0.002;
                let loud = bursts.iter().any(|&(start, end)| (start..end).contains(&t));
                noise + if loud { (t * 200.0 * std::f64::consts::TAU).sin() as f32 * 0.3 } else { 0.0 }
            })
            .collect()
    }

    #[test]
    fn speech_is_found_between_the_silences() {
        let found = voiced(&sound(4.0, &[(1.0, 1.6), (1.7, 2.0), (3.0, 3.5)]));
        assert_eq!(found.len(), 2, "a 0.1 s gap is inside speech: {found:?}");
        assert!((found[0].0 - 1.0).abs() < 0.02 && (found[0].1 - 2.0).abs() < 0.02);
        assert!((found[1].0 - 3.0).abs() < 0.02 && (found[1].1 - 3.5).abs() < 0.02);
    }

    #[test]
    fn words_after_a_pause_start_when_speech_does() {
        // Whisper hears one phrase over the whole four seconds; the speech is in two bursts.
        let samples = sound(4.0, &[(1.0, 1.6), (3.0, 3.8)]);
        let words = words(&[Phrase { text: " Hello there. Again now".into(), start: 0.0, end: 4.0 }], &samples);
        let starts: Vec<f64> = words.iter().map(|word| word.start).collect();
        assert!((starts[0] - 1.0).abs() < 0.03, "the first word starts with the speech, not the phrase: {starts:?}");
        assert!((starts[2] - 3.0).abs() < 0.03, "the word after the pause starts when speech resumes: {starts:?}");
        assert!((words[1].end - 1.6).abs() < 0.03, "and the one before it ends with the first burst: {:?}", words[1]);
        assert!(words.windows(2).all(|pair| pair[0].end <= pair[1].start + 1e-9), "in order, not overlapping");
    }

    #[test]
    fn a_phrase_without_speech_found_keeps_its_times() {
        let words = words(&[Phrase { text: " Hi there".into(), start: 1.0, end: 2.4 }], &vec![0.0; 16_000 * 3]);
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].start, 1.0);
        assert!((words[1].end - 2.4).abs() < 1e-9);
    }
}
