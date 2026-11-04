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
    let words = studio_captions::words(&phrases, &samples);
    assert!(words.windows(2).all(|pair| pair[0].start <= pair[1].start), "words are in order");
}

/// Speaks three sentences with pauses between them (macOS's `say`), then checks that each
/// sentence's first word is timed when it is actually spoken, not where Whisper's loose phrase
/// times put it: `CAPTIONS_MODEL=<folder> cargo test -p studio-captions --test whisper -- --ignored`
#[test]
#[ignore = "needs the Whisper model on disk, and macOS for speech"]
fn words_are_timed_when_they_are_spoken() {
    let folder = PathBuf::from(std::env::var("CAPTIONS_MODEL").expect("set CAPTIONS_MODEL to the model folder"));
    let speech = std::env::temp_dir().join(format!("studio-captions-speech-{}.aiff", std::process::id()));
    let said = std::process::Command::new("say")
        .arg("-o")
        .arg(&speech)
        .arg("[[slnc 3000]] Welcome to Studio captions. [[slnc 2000]] Every word should appear right when it is spoken. [[slnc 1500]] This is the final sentence of the test.")
        .status()
        .expect("macOS's say command runs");
    assert!(said.success());
    let samples = studio_captions::read_speech(&speech, 0.0, 30.0).unwrap();
    let mut transcriber = studio_captions::Transcriber::load(&folder).unwrap();
    let phrases = transcriber.transcribe(&samples, Some("en"), &mut |_| {}, &|| false).unwrap();
    let words = studio_captions::words(&phrases, &samples);
    for word in &words {
        println!("{:6.2} – {:6.2}  {}", word.start, word.end, word.text);
    }
    // Where each sentence really starts: the starts of the three stretches of speech.
    let spoken: Vec<f64> = studio_captions::voiced(&samples).iter().map(|&(start, _)| start).collect();
    assert_eq!(spoken.len(), 3, "three sentences: {spoken:?}");
    for (first_word, at) in ["Welcome", "Every", "This"].iter().zip(&spoken) {
        let word = words.iter().find(|word| word.text.trim_matches(|c: char| !c.is_alphanumeric()).eq_ignore_ascii_case(first_word)).unwrap_or_else(|| panic!("{first_word} was heard"));
        assert!((word.start - at).abs() < 0.15, "{first_word} is spoken at {at:.2} s but timed at {:.2} s", word.start);
    }
    std::fs::remove_file(&speech).ok();
}
