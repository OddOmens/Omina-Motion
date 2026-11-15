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
    /// Where the sound sits between the speakers, from -1 (all left) to 1 (all right). Panning
    /// one way turns the other side down; the side panned toward stays as loud as it was.
    pub pan: f32,
    /// The clip's volume over time, when it is keyframed. `gain` then leaves the clip's own
    /// volume out, and `level_at` reads it from here.
    pub volume_curve: Option<Curve>,
    /// The clip's pan over time, when it is keyframed, in place of `pan`.
    pub pan_curve: Option<Curve>,
    /// Effects on the sound, in the order they run: the layer's own, then those of the
    /// composition layers it plays inside, innermost first.
    pub effects: Vec<ClipEffect>,
}

/// A keyframed setting of a clip (its volume in decibels, or its pan from -100 to 100), with
/// the clock its keyframes are read on: where its composition's frame 0 falls on the played
/// timeline, and that composition's frame rate.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub values: crate::animation::Animated<f32>,
    pub shift_seconds: f64,
    pub fps: f64,
}

impl Curve {
    /// A layer's setting as a curve, if it is keyframed, on a clock that starts with the
    /// layer's composition.
    fn of(setting: &crate::animation::Animated<f32>, fps: f64) -> Option<Self> {
        setting.is_animated().then(|| Self { values: setting.clone(), shift_seconds: 0.0, fps })
    }

    /// The setting at `seconds` of the played timeline.
    pub fn at(&self, seconds: f64) -> f32 {
        self.values.value_at((seconds - self.shift_seconds) * self.fps)
    }
}

/// A sound effect on a clip, with the clock its keyframes are read on: the layer's effect,
/// where its composition's frame 0 falls on the played timeline, and that composition's frame
/// rate.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipEffect {
    pub effect: crate::model::Effect,
    pub shift_seconds: f64,
    pub fps: f64,
}

impl ClipEffect {
    /// The effect's settings at `seconds` of the played timeline, in catalog order.
    pub fn settings_at(&self, seconds: f64) -> Vec<f32> {
        let frame = (seconds - self.shift_seconds) * self.fps;
        self.effect.properties.iter().map(|property| property.value.value_at(frame)).collect()
    }
}

/// The sound effects of `layer` that are switched on, on a clock that starts `shift_seconds`
/// into the played timeline at `fps`.
fn sound_effects(layer: &crate::model::Layer, shift_seconds: f64, fps: f64) -> Vec<ClipEffect> {
    if !layer.switches.effects_enabled {
        return Vec::new();
    }
    layer
        .effects
        .iter()
        .filter(|effect| effect.enabled && crate::effects::is_audio(&effect.name))
        .map(|effect| ClipEffect { effect: effect.clone(), shift_seconds, fps })
        .collect()
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
            pan: 0.0,
            volume_curve: None,
            pan_curve: None,
            effects: Vec::new(),
        }
    }

    /// The factor each side is multiplied by for the clip's pan at `seconds` of composition
    /// time: left, then right.
    pub fn side_gains_at(&self, seconds: f64) -> [f32; 2] {
        let pan = self.pan_curve.as_ref().map_or(self.pan, |curve| crate::model::LayerAudio::balance_of(curve.at(seconds)));
        let pan = if pan.is_finite() { pan.clamp(-1.0, 1.0) } else { 0.0 };
        [(1.0 - pan).min(1.0), (1.0 + pan).min(1.0)]
    }

    /// Whether the clip's pan moves over time, rather than staying in one place.
    pub fn pans(&self) -> bool {
        self.pan_curve.is_some()
    }

    /// Whether the clip's level changes over time (from a fade or volume keyframes), rather
    /// than being one gain.
    pub fn fades(&self) -> bool {
        self.fade_in > 0.0 || self.fade_out > 0.0 || !self.outer_fades.is_empty() || self.volume_curve.is_some()
    }

    /// How loud the clip is at `seconds` of composition time, from its volume and fades.
    pub fn level_at(&self, seconds: f64) -> f32 {
        let own = Fade { start_seconds: self.start_seconds, end_seconds: self.end_seconds, fade_in: self.fade_in, fade_out: self.fade_out };
        let outer: f64 = self.outer_fades.iter().map(|fade| fade.level_at(seconds)).product();
        let keyframed = self.volume_curve.as_ref().map_or(1.0, |curve| crate::model::LayerAudio::gain_of(curve.at(seconds)));
        self.gain * keyframed * (own.level_at(seconds) * outer) as f32
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
    let everything = Placement { shift: 0.0, window: (f64::NEG_INFINITY, f64::INFINITY), gain: 1.0, fades: Vec::new(), chain: Vec::new(), duck: None, noise_gate: 0.0, pan: 0.0, effects: Vec::new() };
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
    /// The composition layers' pans added together, from -1 (all left) to 1 (all right).
    pan: f32,
    /// The sound effects of the composition layers, innermost first, on their own clocks.
    effects: Vec<ClipEffect>,
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
                // This composition layer's own sound effects run before those around it.
                let mut effects = sound_effects(layer, placement.shift, fps);
                effects.extend(placement.effects.iter().cloned());
                let nested = Placement {
                    shift: starts,
                    window,
                    gain: placement.gain * layer.audio.gain(),
                    fades,
                    chain,
                    duck,
                    noise_gate: placement.noise_gate.max(layer.audio.noise_gate.clamp(0.0, 100.0)),
                    pan: (placement.pan + crate::model::LayerAudio::balance_of(layer.audio.pan.value)).clamp(-1.0, 1.0),
                    effects,
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
    // A freeze frame is silent.
    if layer.time.hold.is_some() {
        return None;
    }
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
        // Keyframed volume is read moment by moment from `volume_curve` instead.
        gain: if layer.audio.volume_db.is_animated() { 1.0 } else { layer.audio.gain() },
        fade_in: f64::from(layer.audio.fade_in.max(0.0)),
        fade_out: f64::from(layer.audio.fade_out.max(0.0)),
        outer_fades: Vec::new(),
        speed,
        duck_under: ducks_under(composition, layer),
        duck_db: duck_db(layer),
        noise_gate: layer.audio.noise_gate.clamp(0.0, 100.0),
        pan: crate::model::LayerAudio::balance_of(layer.audio.pan.value),
        volume_curve: Curve::of(&layer.audio.volume_db, fps),
        pan_curve: Curve::of(&layer.audio.pan, fps),
        effects: sound_effects(layer, 0.0, fps),
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
    // Keyframes inside a nested composition are timed from where that composition starts.
    for curve in clip.volume_curve.iter_mut().chain(clip.pan_curve.iter_mut()) {
        curve.shift_seconds += placement.shift;
    }
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
    // The composition layers' pan moves the clip's own along, keyframed or not.
    if placement.pan != 0.0 {
        clip.pan = (clip.pan + placement.pan).clamp(-1.0, 1.0);
        if let Some(curve) = &mut clip.pan_curve {
            curve.values.map_values(|pan| pan + placement.pan * 100.0);
        }
    }
    // The clip's own effects keep their keyframes in step as it moves; the composition
    // layers' effects follow.
    for effect in &mut clip.effects {
        effect.shift_seconds += placement.shift;
    }
    clip.effects.extend(placement.effects.iter().cloned());
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
    fn keyframed_volume_and_pan_change_over_the_clip_even_inside_a_nested_composition() {
        use crate::animation::{Animated, Keyframe};
        let (mut project, outer, _, inner, footage) = nested_sound();
        // In the inner composition (30 fps): silent at frame 15, full volume by frame 45, and
        // panned from all left to all right over the same second.
        let audio = &mut project.composition_mut(inner).unwrap().layer_mut(footage).unwrap().audio;
        audio.volume_db = Animated::keyframed(vec![Keyframe::linear(15, -60.0), Keyframe::linear(45, 0.0)]);
        audio.pan = Animated::keyframed(vec![Keyframe::linear(15, -100.0), Keyframe::linear(45, 100.0)]);
        let own = &sound_of(&project, inner)[0];
        assert!(own.fades() && own.pans(), "the mixer reads the level and pan moment by moment");
        assert_eq!(own.level_at(0.5), 0.0, "silent at its first keyframe");
        assert!((own.level_at(1.5) - 1.0).abs() < 1e-4, "full volume at its last");
        assert!(own.level_at(1.0) > 0.0 && own.level_at(1.0) < 1.0);
        assert_eq!(own.side_gains_at(0.5), [1.0, 0.0]);
        assert_eq!(own.side_gains_at(1.0), [1.0, 1.0]);
        assert_eq!(own.side_gains_at(1.5), [0.0, 1.0]);
        // The outer composition shows the inner one from 2 s, so the same keyframes land 2 s later.
        let nested = &sound_of(&project, outer)[0];
        assert_eq!(nested.level_at(2.5), 0.0);
        assert!((nested.level_at(3.5) - 1.0).abs() < 1e-4);
        assert_eq!(nested.side_gains_at(3.5), [0.0, 1.0]);
    }

    #[test]
    fn a_composition_layers_pan_moves_the_sound_inside_it() {
        let (mut project, outer, shown, inner, footage) = nested_sound();
        project.composition_mut(outer).unwrap().layer_mut(shown).unwrap().audio.pan.value = -100.0;
        assert_eq!(sound_of(&project, outer)[0].side_gains_at(2.5), [1.0, 0.0], "all left");
        // A clip panned inside the composition is moved along by the layer showing it.
        project.composition_mut(outer).unwrap().layer_mut(shown).unwrap().audio.pan.value = 50.0;
        project.composition_mut(inner).unwrap().layer_mut(footage).unwrap().audio.pan.value = -50.0;
        assert_eq!(sound_of(&project, outer)[0].side_gains_at(2.5), [1.0, 1.0], "the two pans cancel out");
    }

    #[test]
    fn volume_and_pan_saved_as_plain_numbers_still_open() {
        // Project files from before volume could be keyframed hold it as a bare number.
        let old: crate::model::LayerAudio = serde_json::from_str(r#"{"volume_db":-6.5,"fade_in":0.0,"fade_out":0.0,"pan":40.0}"#).unwrap();
        assert_eq!((old.volume_db.value, old.pan.value), (-6.5, 40.0));
        assert!(!old.volume_db.is_animated());
        // And one saved now, keyframes included, reads back the same.
        let mut now = crate::model::LayerAudio::default();
        now.volume_db = crate::animation::Animated::keyframed(vec![crate::animation::Keyframe::linear(0, -12.0), crate::animation::Keyframe::linear(24, 0.0)]);
        let read: crate::model::LayerAudio = serde_json::from_str(&serde_json::to_string(&now).unwrap()).unwrap();
        assert_eq!(read, now);
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
        layer.audio.volume_db = crate::animation::Animated::constant(-6.0);
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
    fn sound_effects_run_inside_out_with_their_keyframes_on_their_own_clocks() {
        let (mut project, outer, shown, inner, footage) = nested_sound();
        // A Gain on the footage, rising from 0 dB at inner frame 30 (1 s) to -12 dB at frame 60.
        edit::add_effect(&mut project, inner, footage, "Gain").unwrap();
        edit::add_effect(&mut project, inner, footage, "Gaussian Blur").unwrap();
        let gain = project.composition_mut(inner).unwrap().layer_mut(footage).unwrap();
        gain.effects[0].properties[0].value = crate::animation::Animated::keyframed(vec![crate::animation::Keyframe::linear(30, 0.0), crate::animation::Keyframe::linear(60, -12.0)]);
        // A Pan on the composition layer around it, and one that is switched off.
        edit::add_effect(&mut project, outer, shown, "Pan").unwrap();
        edit::add_effect(&mut project, outer, shown, "Echo").unwrap();
        project.composition_mut(outer).unwrap().layer_mut(shown).unwrap().effects[1].enabled = false;

        let clip = &sound_of(&project, outer)[0];
        let names: Vec<&str> = clip.effects.iter().map(|effect| effect.effect.name.as_str()).collect();
        assert_eq!(names, ["Gain", "Pan"], "picture effects and switched-off ones are left out; the inner layer's run first");
        // The inner composition starts 2 s into the outer one, so its frame 45 (1.5 s) is at 3.5 s.
        assert!((clip.effects[0].settings_at(3.5)[0] + 6.0).abs() < 1e-4, "{:?}", clip.effects[0].settings_at(3.5));

        // With the layer's effects switch off, its sound effects are off too.
        project.composition_mut(inner).unwrap().layer_mut(footage).unwrap().switches.effects_enabled = false;
        let clip = &sound_of(&project, outer)[0];
        assert_eq!(clip.effects.len(), 1);
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
