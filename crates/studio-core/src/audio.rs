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
    /// More fades the level is multiplied by: those of the composition layers a nested clip
    /// plays inside, and its own when its span was cut short by theirs.
    pub outer_fades: Vec<Fade>,
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
        Self {
            layer,
            path,
            offset_seconds,
            start_seconds,
            end_seconds,
            gain: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            outer_fades: Vec::new(),
            speed: 1.0,
            duck_under: None,
            duck_db: 0.0,
            noise_gate: 0.0,
        }
    }

    /// Whether the clip's level changes over time (from any fade), rather than being one gain.
    pub fn fades(&self) -> bool {
        self.fade_in > 0.0 || self.fade_out > 0.0 || !self.outer_fades.is_empty()
    }

    /// How loud the clip is at `seconds` of composition time, from its volume and fades.
    pub fn level_at(&self, seconds: f64) -> f32 {
        let own = Fade { start_seconds: self.start_seconds, end_seconds: self.end_seconds, fade_in: self.fade_in, fade_out: self.fade_out };
        let outer: f64 = self.outer_fades.iter().map(|fade| fade.level_at(seconds)).product();
        self.gain * (own.level_at(seconds) * outer) as f32
    }
}

/// A fade up from silence at the start of a span and down to it at the end. Times are in
/// seconds of composition time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fade {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub fade_in: f64,
    pub fade_out: f64,
}

impl Fade {
    /// The level from 0 to 1 at `seconds`.
    pub fn level_at(&self, seconds: f64) -> f64 {
        let rising = if self.fade_in > 0.0 { ((seconds - self.start_seconds) / self.fade_in).clamp(0.0, 1.0) } else { 1.0 };
        let falling = if self.fade_out > 0.0 { ((self.end_seconds - seconds) / self.fade_out).clamp(0.0, 1.0) } else { 1.0 };
        rising.min(falling)
    }
}

/// How deep nested compositions are listened into, matching how deep the renderer draws them.
const MAX_DEPTH: usize = 8;

/// The sound of every audible layer in `composition`: footage and audio layers that have
/// audio and the audio switch on, and the sound inside nested compositions. As in After
/// Effects, soloing any layer silences every layer that is not soloed (in each composition).
/// Footage playing backwards or ramping its speed is silent: only sound at a steady speed can
/// be played.
pub fn composition_audio(project: &Project, composition: &Composition) -> Vec<AudioClip> {
    let mut clips = Vec::new();
    let everything = Placement { shift: 0.0, window: (f64::NEG_INFINITY, f64::INFINITY), gain: 1.0, fades: Vec::new(), chain: Vec::new(), duck: None, noise_gate: 0.0 };
    collect(project, composition, &everything, &mut clips);
    clips
}

/// Whether composition `comp` plays any sound, looking inside the compositions nested in it.
pub fn composition_has_sound(project: &Project, comp: Id) -> bool {
    fn search(project: &Project, comp: Id, depth: usize) -> bool {
        let Some(composition) = project.composition(comp) else { return false };
        composition.layers.iter().any(|layer| match layer.content {
            LayerContent::Footage { .. } | LayerContent::Audio { .. } => layer.has_audio,
            LayerContent::Composition { composition } => depth < MAX_DEPTH && search(project, composition, depth + 1),
            _ => false,
        })
    }
    search(project, comp, 0)
}

/// Whether `layer` makes sound: footage or audio with sound, or a nested composition with
/// sound inside it.
pub fn layer_has_sound(project: &Project, layer: &crate::model::Layer) -> bool {
    match layer.content {
        LayerContent::Footage { .. } | LayerContent::Audio { .. } => layer.has_audio,
        LayerContent::Composition { composition } => composition_has_sound(project, composition),
        _ => false,
    }
}

/// Where a nested composition's sound is heard in the composition being played, and what the
/// composition layers it is nested through do to it.
struct Placement {
    /// Seconds added to this composition's times to get the played composition's.
    shift: f64,
    /// The span it can be heard in, in seconds of the played composition: where every
    /// composition layer it sits in is showing.
    window: (f64, f64),
    /// The composition layers' volumes, multiplied.
    gain: f32,
    /// The composition layers' fades.
    fades: Vec<Fade>,
    /// The composition layers it is nested through, outermost first (empty at the top).
    chain: Vec<Id>,
    /// Ducking set on the outermost composition layer that has it: the clip to duck under (as
    /// the mixer knows it) and by how many decibels.
    duck: Option<(Id, f32)>,
    /// The strongest noise gate set on the composition layers.
    noise_gate: f32,
}

impl Placement {
    /// What the mixer calls `layer` of the composition at the end of `chain`. Layers of the
    /// played composition keep their own ids. A nested one gets an id made from the whole
    /// chain, so a composition used twice plays as two separate clips, with the top bit set so
    /// it never matches a real layer's id.
    fn key(&self, layer: Id) -> Id {
        if self.chain.is_empty() {
            return layer;
        }
        // FNV-1a over the chain's ids: stable from one call to the next, as the mixer needs.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for id in self.chain.iter().chain(std::iter::once(&layer)) {
            for byte in id.0.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        }
        Id(hash | 1 << 63)
    }
}

/// Adds the sound of `composition`, placed by `placement`, to `clips`.
fn collect(project: &Project, composition: &Composition, placement: &Placement, clips: &mut Vec<AudioClip>) {
    let fps = composition.frame_rate.fps();
    let any_solo = composition.layers.iter().any(|layer| layer.switches.solo);
    let audible = composition.layers.iter().filter(|layer| layer.has_audio && layer.switches.audio && (!any_solo || layer.switches.solo));
    for layer in audible {
        match layer.content {
            LayerContent::Footage { .. } | LayerContent::Audio { .. } => {
                if let Some(clip) = own_clip(project, composition, layer) {
                    clips.extend(placed(clip, placement));
                }
            }
            LayerContent::Composition { composition: inner_id } if placement.chain.len() < MAX_DEPTH => {
                let Some(inner) = project.composition(inner_id) else { continue };
                // Inner time 0 plays where the layer starts; it is heard while the layer shows
                // and the nested composition lasts.
                let starts = placement.shift + layer.start_frame as f64 / fps;
                let shown = (placement.shift + layer.in_frame as f64 / fps, placement.shift + layer.out_frame as f64 / fps);
                let lasts = (starts, starts + inner.duration_frames as f64 / inner.frame_rate.fps());
                let window = (placement.window.0.max(shown.0).max(lasts.0), placement.window.1.min(shown.1).min(lasts.1));
                if window.0 >= window.1 {
                    continue;
                }
                let mut fades = placement.fades.clone();
                let (fade_in, fade_out) = (f64::from(layer.audio.fade_in.max(0.0)), f64::from(layer.audio.fade_out.max(0.0)));
                if fade_in > 0.0 || fade_out > 0.0 {
                    fades.push(Fade { start_seconds: shown.0, end_seconds: shown.1, fade_in, fade_out });
                }
                let duck = placement.duck.or_else(|| ducks_under(composition, layer).map(|under| (placement.key(under), duck_db(layer))));
                let mut chain = placement.chain.clone();
                chain.push(layer.id);
                let nested = Placement {
                    shift: starts,
                    window,
                    gain: placement.gain * layer.audio.gain(),
                    fades,
                    chain,
                    duck,
                    noise_gate: placement.noise_gate.max(layer.audio.noise_gate.clamp(0.0, 100.0)),
                };
                collect(project, inner, &nested, clips);
            }
            _ => {}
        }
    }
}

/// A footage or audio layer's sound, in its own composition's time.
fn own_clip(project: &Project, composition: &Composition, layer: &crate::model::Layer) -> Option<AudioClip> {
    let fps = composition.frame_rate.fps();
    let item = match layer.content {
        LayerContent::Footage { item } | LayerContent::Audio { item } => item,
        _ => return None,
    };
    let speed = layer.time.constant_factor().filter(|_| !layer.time.reverse)?;
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
        outer_fades: Vec::new(),
        speed,
        duck_under: ducks_under(composition, layer),
        duck_db: duck_db(layer),
        noise_gate: layer.audio.noise_gate.clamp(0.0, 100.0),
    })
}

/// The layer `layer` ducks under, if that is another layer of the same composition with sound
/// of its own (footage or audio: a nested composition's sound is many clips, not one level).
fn ducks_under(composition: &Composition, layer: &crate::model::Layer) -> Option<Id> {
    layer.audio.duck_under.filter(|&under| {
        under != layer.id && composition.layer(under).is_some_and(|other| other.has_audio && matches!(other.content, LayerContent::Footage { .. } | LayerContent::Audio { .. }))
    })
}

fn duck_db(layer: &crate::model::Layer) -> f32 {
    layer.audio.duck_db.clamp(crate::model::LayerAudio::DUCK_RANGE.0, crate::model::LayerAudio::DUCK_RANGE.1)
}

/// `clip` (in its own composition's time) moved to where `placement` puts it, cut to the span
/// it can be heard in, and given the composition layers' volume, fades, ducking and noise
/// gate. `None` when none of it can be heard.
fn placed(mut clip: AudioClip, placement: &Placement) -> Option<AudioClip> {
    if placement.chain.is_empty() {
        return Some(clip);
    }
    let own = Fade { start_seconds: clip.start_seconds + placement.shift, end_seconds: clip.end_seconds + placement.shift, fade_in: clip.fade_in, fade_out: clip.fade_out };
    let (start, end) = (own.start_seconds.max(placement.window.0), own.end_seconds.min(placement.window.1));
    if start >= end {
        return None;
    }
    // The clip's own fades stay where they were, even where the span cuts into them.
    clip.outer_fades = placement.fades.clone();
    if own.fade_in > 0.0 || own.fade_out > 0.0 {
        clip.outer_fades.push(own);
    }
    clip.fade_in = 0.0;
    clip.fade_out = 0.0;
    clip.layer = placement.key(clip.layer);
    clip.offset_seconds += placement.shift;
    clip.start_seconds = start;
    clip.end_seconds = end;
    clip.gain *= placement.gain;
    match clip.duck_under {
        // Ducking under a neighbour inside the same composition.
        Some(under) => clip.duck_under = Some(placement.key(under)),
        None => {
            if let Some((under, db)) = placement.duck {
                clip.duck_under = Some(under);
                clip.duck_db = db;
            }
        }
    }
    clip.noise_gate = clip.noise_gate.max(placement.noise_gate);
    Some(clip)
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
    /// A 4-second composition with 2 seconds of footage with sound from 0.5 s, shown in
    /// another composition from 2 s. Returns the project, the outer composition, the layer
    /// showing the inner one, the inner composition and its footage layer.
    fn nested_sound() -> (Project, Id, Id, Id, Id) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = |name: &str| edit::CompositionSettings {
            name: name.into(),
            width: 640,
            height: 360,
            frame_rate: FrameRate::new(30, 1),
            duration_frames: 120,
            background: crate::model::Rgba::TRANSPARENT,
        };
        let inner = edit::new_composition(&mut project, &settings("Inner")).unwrap();
        let outer = edit::new_composition(&mut project, &edit::CompositionSettings { duration_frames: 300, ..settings("Outer") }).unwrap();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 2.0, frame_rate: FrameRate::new(30, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/voice.mp4".into(), &info);
        let footage = edit::add_layer_from_item(&mut project, inner, item, 0, 15).unwrap();
        let shown = edit::add_layer_from_item(&mut project, outer, inner, 0, 60).unwrap();
        (project, outer, shown, inner, footage)
    }

    fn sound_of(project: &Project, comp: Id) -> Vec<AudioClip> {
        composition_audio(project, project.composition(comp).unwrap())
    }

    #[test]
    fn a_nested_composition_plays_its_sound_where_its_layer_starts() {
        let (mut project, outer, shown, _, footage) = nested_sound();
        let clips = sound_of(&project, outer);
        assert_eq!(clips.len(), 1, "the footage inside is heard");
        let clip = &clips[0];
        assert_eq!((clip.offset_seconds, clip.start_seconds, clip.end_seconds), (2.5, 2.5, 4.5));
        assert_ne!(clip.layer, footage, "a nested clip has an id of its own");
        // Trimming the composition layer's start to 3 s cuts the sound there; its volume applies.
        let layer = project.composition_mut(outer).unwrap().layer_mut(shown).unwrap();
        layer.in_frame = 90;
        layer.audio.volume_db = -6.0;
        let clip = &sound_of(&project, outer)[0];
        assert_eq!((clip.offset_seconds, clip.start_seconds, clip.end_seconds), (2.5, 3.0, 4.5));
        assert!((clip.gain - 0.501).abs() < 0.001, "gain {}", clip.gain);
    }

    #[test]
    fn muting_or_soloing_around_a_nested_composition_silences_it() {
        let (mut project, outer, shown, _, _) = nested_sound();
        project.composition_mut(outer).unwrap().layer_mut(shown).unwrap().switches.audio = false;
        assert!(sound_of(&project, outer).is_empty(), "muted");
        let outer_comp = project.composition_mut(outer).unwrap();
        outer_comp.layer_mut(shown).unwrap().switches.audio = true;
        let title = edit::add_text_layer(&mut project, outer, "Title", Vec2::new(10.0, 10.0), 0).unwrap();
        project.composition_mut(outer).unwrap().layer_mut(title).unwrap().switches.solo = true;
        assert!(sound_of(&project, outer).is_empty(), "another layer soloed");
    }

    #[test]
    fn a_composition_used_twice_plays_twice_as_separate_clips() {
        let (mut project, outer, _, inner, _) = nested_sound();
        edit::add_layer_from_item(&mut project, outer, inner, 0, 0).unwrap();
        let clips = sound_of(&project, outer);
        assert_eq!(clips.len(), 2);
        assert_ne!(clips[0].layer, clips[1].layer, "the mixer tells them apart");
        assert_eq!(sound_of(&project, outer), clips, "and the ids stay the same from call to call");
    }

    #[test]
    fn the_composition_layers_fades_and_ducking_reach_the_sound_inside() {
        let (mut project, outer, shown, _, _) = nested_sound();
        let info = FootageInfo { size: Vec2::new(64.0, 36.0), duration_seconds: 9.0, frame_rate: FrameRate::new(30, 1), has_audio: true };
        let item = edit::import_footage(&mut project, "/media/voiceover.mp4".into(), &info);
        let voiceover = edit::add_layer_from_item(&mut project, outer, item, 0, 0).unwrap();
        let layer = project.composition_mut(outer).unwrap().layer_mut(shown).unwrap();
        layer.audio.fade_in = 1.0;
        layer.audio.duck_under = Some(voiceover);
        let clips = sound_of(&project, outer);
        let nested = clips.iter().find(|clip| clip.layer != voiceover).unwrap();
        assert_eq!(nested.duck_under, Some(voiceover), "ducks under the voiceover");
        // The layer shows from 2 s and fades in over a second: half way up at 2.5 s.
        assert!((nested.level_at(2.5) - 0.5).abs() < 0.001, "level {}", nested.level_at(2.5));
        assert!((nested.level_at(3.5) - 1.0).abs() < 0.001);
    }

    #[test]
    fn a_composition_has_sound_only_if_something_inside_it_does() {
        let (mut project, outer, shown, inner, footage) = nested_sound();
        assert!(composition_has_sound(&project, outer));
        assert!(layer_has_sound(&project, project.composition(outer).unwrap().layer(shown).unwrap()));
        project.composition_mut(inner).unwrap().layer_mut(footage).unwrap().has_audio = false;
        assert!(!composition_has_sound(&project, outer));
    }
}
