//! Transcribes real speech with the downloaded model. The model is not part of the repository
//! (it is 150 MB), so this runs on demand:
//! `CAPTIONS_MODEL=<folder> CAPTIONS_SPEECH=<sound file> cargo test -p studio-captions --test whisper -- --ignored --nocapture`

use std::path::PathBuf;

#[test]
#[ignore = "needs the Whisper model on disk"]
fn transcribes_speech() {
    let folder = PathBuf::from(std::env::var("CAPTIONS_MODEL").expect("set CAPTIONS_MODEL to the model folder"));
    let speech = PathBuf::from(std::env::var("CAPTIONS_SPEECH").expect("set CAPTIONS_SPEECH to a sound file with speech"));
    let started = std::time::Instant::now();
    let mut transcriber = studio_captions::Transcriber::load(&folder).unwrap();
    let samples = studio_captions::read_speech(&speech, 0.0, 30.0).unwrap();
    let loaded = started.elapsed();
    let phrases = transcriber.transcribe(&samples, None, &mut |_| {}, &|| false).unwrap();
    println!("loaded in {loaded:?}, transcribed {:.1} s of sound in {:?}", samples.len() as f64 / 16_000.0, started.elapsed() - loaded);
    for phrase in &phrases {
        println!("{:6.2} – {:6.2}  {}", phrase.start, phrase.end, phrase.text);
    }
    let text = phrases.iter().map(|phrase| phrase.text.as_str()).collect::<Vec<_>>().join(" ").to_lowercase();
    assert!(text.contains("studio") && text.contains("captions"), "heard: {text}");
    let words = studio_captions::words(&phrases);
    assert!(words.windows(2).all(|pair| pair[0].start <= pair[1].start), "words are in order");
}
