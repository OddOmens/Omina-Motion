//! Export presets: a named set of the Export window's choices (format, quality, size, frame
//! rate, frames and sound), and for some a frame shape to export at. Motion comes with a few
//! ("Instagram Reel", "YouTube", "GIF for Slack", "ProRes Master"); more are saved from the
//! Export window's "Save as Preset" or by asking Iris to "save these export settings as Print".
//! Iris exports with one when asked "export for Instagram Reel"; the Export window's Preset
//! menu puts one's choices in place.
//!
//! The presets are kept by the shared `iris::export_presets` module, which holds each one's
//! settings as Motion writes them here (`PresetSettings`), so Iris in each app keeps its own.

use iris::App;
use iris::export_presets::Preset;
use studio_core::time::FrameRate;
use studio_export::ExportFormat;
use studio_media::{EncodeSpeed, MovieFormat, VideoQuality};

use super::export::ExportChoice;

/// One preset's settings as they are kept: the Export window's choices, written as words so the
/// file reads plainly and outlives changes to the app's own types, and the frame shape the
/// export is meant for, if it has one.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PresetSettings {
    /// The format by its short name (see `format_name`).
    pub format: String,
    /// The output's frame rate in frames a second, or `None` for the composition's own.
    pub frame_rate: Option<f64>,
    pub work_area_only: bool,
    pub include_audio: bool,
    /// The size as a fraction of the composition's.
    pub scale: f32,
    /// The quality by its short name (see `quality_name`).
    pub quality: String,
    /// The encoding speed by its short name (see `speed_name`).
    pub speed: String,
    pub audio_kbps: u32,
    /// The frame the export is meant for, as width and height. A composition of another shape
    /// is copied at this size first (Magic Resize), so the export fills the frame.
    pub frame: Option<(u32, u32)>,
}

impl Default for PresetSettings {
    fn default() -> Self {
        Self::from_choice(&ExportChoice::default(), None)
    }
}

/// Every format, with the short name a preset keeps it by.
const FORMATS: &[(&str, ExportFormat)] = &[
    ("h264", ExportFormat::Movie(MovieFormat::H264)),
    ("h265", ExportFormat::Movie(MovieFormat::H265)),
    ("webm", ExportFormat::Movie(MovieFormat::WebM)),
    ("prores-4444", ExportFormat::Movie(MovieFormat::ProRes4444)),
    ("prores-422-hq", ExportFormat::Movie(MovieFormat::ProRes422Hq)),
    ("prores-422", ExportFormat::Movie(MovieFormat::ProRes422)),
    ("prores-lt", ExportFormat::Movie(MovieFormat::ProResLt)),
    ("prores-proxy", ExportFormat::Movie(MovieFormat::ProResProxy)),
    ("gif", ExportFormat::Gif),
    ("png-sequence", ExportFormat::PngSequence),
    ("lottie", ExportFormat::Lottie),
    ("dotlottie", ExportFormat::DotLottie),
    ("svg", ExportFormat::AnimatedSvg),
];

fn format_name(format: ExportFormat) -> &'static str {
    FORMATS.iter().find(|(_, known)| *known == format).map_or("h264", |(name, _)| name)
}

fn quality_name(quality: VideoQuality) -> String {
    match quality {
        VideoQuality::Draft => "draft".to_owned(),
        VideoQuality::Good => "good".to_owned(),
        VideoQuality::High => "high".to_owned(),
        VideoQuality::Maximum => "maximum".to_owned(),
        VideoQuality::Bitrate(kbps) => format!("{kbps} kbps"),
    }
}

fn quality_named(name: &str) -> VideoQuality {
    match name {
        "draft" => VideoQuality::Draft,
        "good" => VideoQuality::Good,
        "maximum" => VideoQuality::Maximum,
        other => other.strip_suffix(" kbps").and_then(|kbps| kbps.parse().ok()).map_or(VideoQuality::High, VideoQuality::Bitrate),
    }
}

fn speed_name(speed: EncodeSpeed) -> &'static str {
    match speed {
        EncodeSpeed::Fast => "fast",
        EncodeSpeed::Balanced => "balanced",
        EncodeSpeed::Small => "small",
    }
}

fn speed_named(name: &str) -> EncodeSpeed {
    match name {
        "fast" => EncodeSpeed::Fast,
        "small" => EncodeSpeed::Small,
        _ => EncodeSpeed::Balanced,
    }
}

/// The frame rates a preset can name, as the Export window offers them: whole rates, and the
/// broadcast ones by their exact fractions.
fn rate_of(fps: f64) -> FrameRate {
    super::composition_settings::parse_frame_rate(&fps.to_string()).unwrap_or(FrameRate::new(fps.round().max(1.0) as u32, 1))
}

impl PresetSettings {
    /// The settings of `choice`, for the frame `frame` if it has one.
    pub fn from_choice(choice: &ExportChoice, frame: Option<(u32, u32)>) -> Self {
        Self {
            format: format_name(choice.format).to_owned(),
            frame_rate: choice.frame_rate.map(FrameRate::fps),
            work_area_only: choice.work_area_only,
            include_audio: choice.include_audio,
            scale: choice.scale,
            quality: quality_name(choice.quality),
            speed: speed_name(choice.speed).to_owned(),
            audio_kbps: choice.audio_kbps,
            frame,
        }
    }

    /// The Export window's choices these settings stand for. A value that can't be read (from
    /// a file written by hand, say) falls back to the Export window's usual choice.
    pub fn choice(&self) -> ExportChoice {
        let usual = ExportChoice::default();
        let scale = if self.scale.is_finite() && self.scale > 0.0 { self.scale.min(1.0) } else { usual.scale };
        ExportChoice {
            format: FORMATS.iter().find(|(name, _)| *name == self.format).map_or(usual.format, |(_, format)| *format),
            frame_rate: self.frame_rate.filter(|fps| fps.is_finite() && *fps > 0.0).map(rate_of),
            work_area_only: self.work_area_only,
            include_audio: self.include_audio,
            scale,
            quality: quality_named(&self.quality),
            speed: speed_named(&self.speed),
            audio_kbps: if self.audio_kbps == 0 { usual.audio_kbps } else { self.audio_kbps },
        }
    }

    /// The preset kept with these settings, under `name`.
    pub fn named(&self, name: &str) -> Preset {
        Preset { name: name.to_owned(), settings: serde_json::to_value(self).unwrap_or_default() }
    }

    /// The settings a kept preset holds.
    pub fn of(preset: &Preset) -> Self {
        serde_json::from_value(preset.settings.clone()).unwrap_or_default()
    }

    /// What the settings export, in a few words: "H.264 (MP4), 1080 × 1920, 30 fps".
    pub fn describe(&self) -> String {
        let choice = self.choice();
        let mut parts = vec![choice.format.label().to_owned()];
        if let Some((width, height)) = self.frame {
            parts.push(format!("{width} × {height}"));
        }
        if choice.scale < 1.0 {
            parts.push(format!("{:.0}% size", choice.scale * 100.0));
        }
        if let Some(rate) = choice.frame_rate {
            parts.push(rate.label());
        }
        parts.push(if choice.work_area_only { "the work area".to_owned() } else { "the whole composition".to_owned() });
        parts.join(", ")
    }
}

/// The presets Motion comes with. Each is a combination the Export window can make, plus the
/// frame shape its destination wants.
pub fn built_in() -> Vec<Preset> {
    let whole = |format: ExportFormat| ExportChoice { format, work_area_only: false, ..ExportChoice::default() };
    let h264 = ExportFormat::Movie(MovieFormat::H264);
    let reel = ExportChoice { frame_rate: Some(FrameRate::new(30, 1)), quality: VideoQuality::High, ..whole(h264) };
    let youtube = ExportChoice { quality: VideoQuality::High, ..whole(h264) };
    // Small enough to post in a chat: half size, 15 frames a second, no sound (GIFs have none).
    let slack = ExportChoice { scale: 0.5, frame_rate: Some(FrameRate::new(15, 1)), include_audio: false, ..whole(ExportFormat::Gif) };
    let master = whole(ExportFormat::Movie(MovieFormat::ProRes422Hq));
    vec![
        PresetSettings::from_choice(&reel, Some((1080, 1920))).named("Instagram Reel"),
        PresetSettings::from_choice(&youtube, Some((1920, 1080))).named("YouTube"),
        PresetSettings::from_choice(&slack, None).named("GIF for Slack"),
        PresetSettings::from_choice(&master, None).named("ProRes Master"),
    ]
}

/// The presets saved in Motion, from Iris's shared folder.
pub fn saved() -> Vec<Preset> {
    super::assistant::shared_folder().map(|folder| iris::export_presets::load(&folder, App::Studio)).unwrap_or_default()
}

/// Keeps `presets` as the ones saved in Motion. `Err` says why they couldn't be written.
pub fn keep_saved(presets: &[Preset]) -> Result<(), String> {
    let folder = super::assistant::shared_folder().ok_or("There is no home folder to keep presets in.")?;
    iris::export_presets::save(&folder, App::Studio, presets).map_err(|error| format!("The presets couldn't be saved: {error}"))
}

/// Saves the Export window's current choices as a preset called `name`, replacing a saved one
/// of that name. Returns what to say.
pub fn save_current(choice: &ExportChoice, name: &str) -> Result<String, String> {
    let mut presets = saved();
    let settings = PresetSettings::from_choice(choice, None);
    let described = settings.describe();
    let replacing = presets.iter().any(|preset| preset.name.eq_ignore_ascii_case(name));
    iris::export_presets::keep(&mut presets, settings.named(name))?;
    keep_saved(&presets)?;
    Ok(format!("{} the export preset “{name}”: {described}. Say “export for {name}” to export with it, or pick it from the Preset menu in the Export window.", if replacing { "Replaced" } else { "Saved" }))
}

/// Every preset name known: the saved ones, then Motion's own.
pub fn names() -> Vec<String> {
    let saved = saved();
    let own = built_in();
    saved.iter().chain(own.iter().filter(|preset| !saved.iter().any(|kept| kept.name.eq_ignore_ascii_case(&preset.name)))).map(|preset| preset.name.clone()).collect()
}

/// The preset called `name`, a saved one first.
pub fn find(name: &str) -> Option<Preset> {
    let (saved, own) = (saved(), built_in());
    iris::export_presets::find(name, &saved, &own).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_survives_being_kept_as_a_preset() {
        let choice = ExportChoice { format: ExportFormat::Movie(MovieFormat::ProRes4444), frame_rate: Some(FrameRate::new(30000, 1001)), work_area_only: false, include_audio: false, scale: 0.75, quality: VideoQuality::Bitrate(16_000), speed: EncodeSpeed::Small, audio_kbps: 192 };
        let kept = PresetSettings::from_choice(&choice, Some((1080, 1350))).named("Test");
        let back = PresetSettings::of(&kept);
        assert_eq!(back.choice(), choice);
        assert_eq!(back.frame, Some((1080, 1350)));
    }

    #[test]
    fn the_built_in_presets_are_real_export_choices() {
        let presets = built_in();
        let names: Vec<&str> = presets.iter().map(|preset| preset.name.as_str()).collect();
        assert_eq!(names, ["Instagram Reel", "YouTube", "GIF for Slack", "ProRes Master"]);
        let reel = PresetSettings::of(&presets[0]);
        assert_eq!((reel.choice().format, reel.frame, reel.choice().frame_rate), (ExportFormat::Movie(MovieFormat::H264), Some((1080, 1920)), Some(FrameRate::new(30, 1))));
        let slack = PresetSettings::of(&presets[2]).choice();
        assert!(slack.format == ExportFormat::Gif && slack.scale == 0.5 && slack.format.allows_rate(slack.frame_rate.unwrap()));
        assert_eq!(PresetSettings::of(&presets[3]).choice().format, ExportFormat::Movie(MovieFormat::ProRes422Hq));
        assert_eq!(PresetSettings::of(&presets[0]).describe(), "H.264 (MP4), 1080 × 1920, 30 fps, the whole composition");
    }

    #[test]
    fn unreadable_settings_fall_back_to_the_usual_choice() {
        let odd = Preset { name: "Odd".to_owned(), settings: serde_json::json!({ "format": "betamax", "scale": -3.0, "quality": "great" }) };
        let choice = PresetSettings::of(&odd).choice();
        let usual = ExportChoice::default();
        assert_eq!((choice.format, choice.scale, choice.quality), (usual.format, usual.scale, VideoQuality::High));
    }
}
