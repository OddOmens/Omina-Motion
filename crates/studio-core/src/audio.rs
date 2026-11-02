//! Which audio a composition plays, and where on its timeline. The mixer in `studio-media`
//! turns this list into sound; keeping the rules here keeps them free of any decoder.

use std::path::PathBuf;

use crate::model::{Composition, Id, LayerContent, Project};

/// One layer's sound placed on a composition's timeline. Times are in seconds of
/// composition time.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    /// The layer the sound belongs to, so the mixer can tell clips from the same file apart.
    pub layer: Id,
    pub path: PathBuf,
    /// When the file's first sample plays.
    pub offset_seconds: f64,
    /// The span in which the clip is heard: from the layer's in point to its out point.
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Volume as a linear factor (1.0 = unchanged).
    pub gain: f32,
    /// Seconds of fade up from silence after `start_seconds`, and down to it before `end_seconds`.
    pub fade_in: f64,
    pub fade_out: f64,
    /// How fast the sound plays (1 is normal). Faster or slower sound changes pitch, as a
    /// tape played faster does.
    pub speed: f64,
    /// The clip whose sound this one gets quieter under, and by how many decibels.
    pub duck_under: Option<Id>,
    pub duck_db: f32,
    /// How strongly background sound is turned down between louder sound, 0 to 100.
    pub noise_gate: f32,
}

impl AudioClip {
    /// A clip at normal speed, without ducking or a noise gate.
    pub fn plain(layer: Id, path: PathBuf, offset_seconds: f64, start_seconds: f64, end_seconds: f64) -> Self {
        Self { layer, path, offset_seconds, start_seconds, end_seconds, gain: 1.0, fade_in: 0.0, fade_out: 0.0, speed: 1.0, duck_under: None, duck_db: 0.0, noise_gate: 0.0 }
    }

    /// How loud the clip is at `seconds` of composition time, from its volume and fades.
    pub fn level_at(&self, seconds: f64) -> f32 {
        let rising = if self.fade_in > 0.0 { ((seconds - self.start_seconds) / self.fade_in).clamp(0.0, 1.0) } else { 1.0 };
        let falling = if self.fade_out > 0.0 { ((self.end_seconds - seconds) / self.fade_out).clamp(0.0, 1.0) } else { 1.0 };
        self.gain * rising.min(falling) as f32
    }
}

/// The sound of every audible layer in `composition`: footage and audio layers that have
/// audio and the audio switch on. As in After Effects, soloing any layer silences every
/// layer that is not soloed. Footage playing backwards or ramping its speed is silent: only
/// sound at a steady speed can be played.
pub fn composition_audio(project: &Project, composition: &Composition) -> Vec<AudioClip> {
    let fps = composition.frame_rate.fps();
    let any_solo = composition.layers.iter().any(|layer| layer.switches.solo);
    composition
        .layers
        .iter()
        .filter(|layer| layer.has_audio && layer.switches.audio && (!any_solo || layer.switches.solo))
        .filter(|layer| !layer.time.reverse && layer.time.constant_factor().is_some())
        .filter_map(|layer| {
            let item = match layer.content {
                LayerContent::Footage { item } | LayerContent::Audio { item } => item,
                _ => return None,
            };
            // A cleaned copy plays in place of the original, once it is made.
            let cleaned = layer.audio.enhance.as_ref().filter(|enhance| enhance.on).and_then(|enhance| enhance.file.clone());
            let path = cleaned.or_else(|| project.item(item)?.path.clone())?;
            Some(AudioClip {
                layer: layer.id,
                path,
                // Where the file's first sample plays; trimming and slipping move it apart from the in point.
                offset_seconds: layer.start_frame as f64 / fps,
                start_seconds: layer.in_frame as f64 / fps,
                end_seconds: layer.out_frame as f64 / fps,
                gain: layer.audio.gain(),
                fade_in: f64::from(layer.audio.fade_in.max(0.0)),
                fade_out: f64::from(layer.audio.fade_out.max(0.0)),
                speed: layer.time.constant_factor().unwrap_or(1.0),
                // Only under another layer of this composition that has sound.
                duck_under: layer.audio.duck_under.filter(|&under| under != layer.id && composition.layer(under).is_some_and(|other| other.has_audio)),
                duck_db: layer.audio.duck_db.clamp(crate::model::LayerAudio::DUCK_RANGE.0, crate::model::LayerAudio::DUCK_RANGE.1),
                noise_gate: layer.audio.noise_gate.clamp(0.0, 100.0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::{self, FootageInfo};
    use crate::model::Project;
    use crate::sample::{ids, sample_project};
    use crate::time::FrameRate;
    use crate::animation::Vec2;

    /// The sample project with a real file behind a new footage layer at frame 48.
    fn project_with_footage() -> (Project, Id) {
        let mut project = sample_project();
        let info = FootageInfo {
            size: Vec2::new(64.0, 36.0),
            duration_seconds: 2.0,
            frame_rate: FrameRate::new(24, 1),
            has_audio: true,
        };
        let item = edit::import_footage(&mut project, "/media/clip.mp4".into(), &info);
        let layer = edit::add_layer_from_item(&mut project, ids::MAIN_COMP, item, 0, 48).expect("footage becomes a layer");
        (project, layer)
    }

    #[test]
    fn footage_with_sound_plays_from_its_in_point() {
        let (project, layer) = project_with_footage();
        let comp = project.composition(ids::MAIN_COMP).unwrap();
        // The sample's own footage has no file behind it, so only the imported clip is heard.
        let clips = composition_audio(&project, comp);
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!((clip.layer, clip.path.to_str()), (layer, Some("/media/clip.mp4")));
        assert_eq!((clip.offset_seconds, clip.start_seconds, clip.end_seconds), (2.0, 2.0, 4.0));
    }

    #[test]
    fn trimmed_and_slipped_sound_plays_the_right_part() {
        let (mut project, layer) = project_with_footage();
        // Trim the first half second off, then slip the source a quarter second later (with the
        // end untrimmed, the source can only slip later: it must still reach the out point).
        crate::trim::trim_in(&mut project, ids::MAIN_COMP, layer, 60);
        crate::trim::slip_to(&mut project, ids::MAIN_COMP, layer, 54);
        let clip = &composition_audio(&project, project.composition(ids::MAIN_COMP).unwrap())[0];
        assert_eq!((clip.offset_seconds, clip.start_seconds, clip.end_seconds), (2.25, 2.5, 4.0));
    }

    #[test]
    fn the_audio_switch_and_solo_silence_layers() {
        let (mut project, layer) = project_with_footage();
        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.layer_mut(layer).unwrap().switches.audio = false;
        assert!(composition_audio(&project, project.composition(ids::MAIN_COMP).unwrap()).is_empty());

        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.layer_mut(layer).unwrap().switches.audio = true;
        // Soloing the title (a layer without sound) silences everything else.
        comp.layer_mut(Id(100)).unwrap().switches.solo = true;
        assert!(composition_audio(&project, project.composition(ids::MAIN_COMP).unwrap()).is_empty());

        let comp = project.composition_mut(ids::MAIN_COMP).unwrap();
        comp.layer_mut(layer).unwrap().switches.solo = true;
        assert_eq!(composition_audio(&project, project.composition(ids::MAIN_COMP).unwrap()).len(), 1);
    }
}
