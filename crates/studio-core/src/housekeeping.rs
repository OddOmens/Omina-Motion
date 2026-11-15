//! Carrying out everyday housekeeping typed to Iris ("hide the logo", "duplicate it", "bring it
//! to the front", "put it in the top right corner", "make it red", "a bit bigger") on a
//! composition. The words are read by the shared `iris::layers`; this makes the changes with
//! Studio's own edits and says what it did in plain words, including what it left alone and
//! why. `instructions` does the same for timing and motion.
//!
//! The steps are carried out in the order said, each on every layer meant. Duplicating hands
//! the copies to the steps after it, so "duplicate it and flip it" flips the copy.

use iris::layers::{Along, Stack, Step};

use crate::align::{AlignTarget, Alignment, align_layers};
use crate::instructions::Context;
use crate::model::{Composition, Id, LayerContent, Paint, Project, Rgba};
use crate::property::{PropertyPath, PropertyValue, TransformProperty};
use crate::tracks::{self, MAIN_TRACK};

/// What was done, to report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// The names of the layers the steps were about, as they were before anything changed.
    pub who: Vec<String>,
    /// What happened to each of them, as phrases ("is hidden", "has a copy just above it").
    pub done: Vec<String>,
    /// What couldn't be done and why, and anything else worth knowing, as sentences.
    pub problems: Vec<String>,
    /// The layers the steps ended on: the copies after duplicating, none after deleting.
    /// Instructions later in the same message are about these.
    pub layers: Vec<Id>,
    /// What should be selected afterwards, when the steps change that (selecting, duplicating,
    /// deleting and locking do).
    pub selection: Option<Vec<Id>>,
    /// Whether anything in the project changed. Selecting alone changes nothing.
    pub changed: bool,
}

/// A layer's name in quotes, for a sentence.
fn quoted(composition: &Composition, id: Id) -> String {
    composition.layer(id).map_or_else(String::new, |layer| format!("“{}”", layer.name))
}

/// Carries out `steps` on `layers` in composition `comp`. `selected` is what is selected now,
/// so the outcome can say what should be afterwards.
pub fn carry_out(project: &mut Project, comp: Id, layers: &[Id], steps: &[Step], context: Context, selected: &[Id]) -> Outcome {
    let mut outcome = Outcome { layers: layers.to_vec(), ..Outcome::default() };
    let Some(composition) = project.composition(comp) else { return outcome };
    outcome.who = layers.iter().filter_map(|id| composition.layer(*id)).map(|layer| layer.name.clone()).collect();
    let mut copied = false;
    for step in steps {
        if outcome.layers.is_empty() {
            break;
        }
        let Some(phrase) = one_step(project, comp, step, context, selected, &mut outcome) else { continue };
        // After duplicating, what happens is about the copies.
        outcome.done.push(if copied { format!("the copy {phrase}") } else { phrase });
        copied |= *step == Step::Duplicate;
    }
    outcome
}

/// Carries out one step on every layer the outcome is on. Returns the phrase for it when it
/// changed (or selected) at least one layer.
fn one_step(project: &mut Project, comp: Id, step: &Step, context: Context, selected: &[Id], outcome: &mut Outcome) -> Option<String> {
    let ids = outcome.layers.clone();
    match step {
        Step::Select => {
            let composition = project.composition(comp)?;
            let (free, locked): (Vec<Id>, Vec<Id>) = ids.iter().partition(|id| composition.layer(**id).is_some_and(|layer| !layer.switches.locked));
            if !locked.is_empty() {
                outcome.problems.push(format!("{} locked, so {} be selected.", counted_are(locked.len()), if locked.len() == 1 { "it can't" } else { "they can't" }));
            }
            if free.is_empty() {
                return None;
            }
            outcome.selection = Some(free);
            Some("is selected".to_owned())
        }
        Step::Hide | Step::Show => {
            let show = *step == Step::Show;
            let composition = project.composition_mut(comp)?;
            let (mut any, mut already) = (false, Vec::new());
            for id in ids {
                let Some(layer) = composition.layer_mut(id) else { continue };
                if !layer.content.has_picture() {
                    outcome.problems.push(format!("“{}” is sound, so there is no picture to {}. Say “{}” for its sound.", layer.name, if show { "show" } else { "hide" }, if show { "unmute it" } else { "mute it" }));
                } else if layer.switches.visible == show {
                    already.push(layer.name.clone());
                } else {
                    layer.switches.visible = show;
                    any = true;
                }
            }
            outcome.problems.extend(were_already(&already, if show { "showing" } else { "hidden" }));
            any.then(|| if show { "is showing again" } else { "is hidden" }.to_owned())
        }
        Step::Lock | Step::Unlock => {
            let lock = *step == Step::Lock;
            let composition = project.composition_mut(comp)?;
            let (mut changed, mut already) = (Vec::new(), Vec::new());
            for id in ids {
                let Some(layer) = composition.layer_mut(id) else { continue };
                if layer.switches.locked == lock {
                    already.push(layer.name.clone());
                } else {
                    layer.switches.locked = lock;
                    changed.push(id);
                }
            }
            outcome.problems.extend(were_already(&already, if lock { "locked" } else { "unlocked" }));
            // A locked layer can't stay selected, as when it is locked in the timeline.
            if lock && !changed.is_empty() {
                let kept = outcome.selection.as_deref().unwrap_or(selected).iter().copied().filter(|id| !changed.contains(id)).collect();
                outcome.selection = Some(kept);
            }
            (!changed.is_empty()).then(|| if lock { "is locked" } else { "is unlocked" }.to_owned())
        }
        Step::Duplicate => {
            let copies = crate::copy::duplicate_layers(project, comp, &ids);
            if copies.is_empty() {
                outcome.problems.push("I couldn't make a copy.".to_owned());
                return None;
            }
            let composition = project.composition(comp)?;
            outcome.selection = Some(copies.iter().copied().filter(|id| composition.layer(*id).is_some_and(|layer| !layer.switches.locked)).collect());
            outcome.layers = copies;
            Some("has a copy just above it".to_owned())
        }
        Step::Delete => {
            let composition = project.composition_mut(comp)?;
            let (free, locked): (Vec<Id>, Vec<Id>) = ids.iter().partition(|id| composition.layer(**id).is_some_and(|layer| !layer.switches.locked));
            for id in &locked {
                outcome.problems.push(format!("{} is locked, so I left it. Unlock it first.", quoted(composition, *id)));
            }
            if crate::edit::delete_layers(composition, &free) == 0 {
                return None;
            }
            outcome.selection = Some(outcome.selection.as_deref().unwrap_or(selected).iter().copied().filter(|id| !free.contains(id)).collect());
            outcome.layers = Vec::new();
            Some("is deleted".to_owned())
        }
        _ => {
            let composition = project.composition_mut(comp)?;
            // Several layers moved in the stacking order go in the order that keeps them as
            // they were among themselves.
            let mut ordered: Vec<Id> = composition.layers.iter().map(|layer| layer.id).filter(|id| ids.contains(id)).collect();
            if matches!(step, Step::Stack(Stack::Front | Stack::Backward)) {
                ordered.reverse();
            }
            let many = ordered.len() > 1;
            let mut phrase = None;
            for (index, id) in ordered.into_iter().enumerate() {
                match change_one(composition, id, step, context, many.then_some(index + 1)) {
                    Ok(said) => phrase = phrase.or(Some(said)),
                    Err(why) => {
                        if !outcome.problems.contains(&why) {
                            outcome.problems.push(why);
                        }
                    }
                }
            }
            phrase
        }
    }
    .inspect(|_| outcome.changed |= *step != Step::Select)
}

/// A sentence for the layers a step found as it would have left them: one is named, several
/// are counted, so "unlock everything" doesn't list every layer that was never locked.
fn were_already(names: &[String], state: &str) -> Option<String> {
    match names {
        [] => None,
        [one] => Some(format!("“{one}” was already {state}.")),
        many => Some(format!("{} were already {state}.", many.len())),
    }
}

/// "2 layers are", "1 layer is".
fn counted_are(count: usize) -> String {
    if count == 1 { "1 layer is".to_owned() } else { format!("{count} layers are") }
}

/// Carries out a step that changes one layer in place. `number` is the layer's place among
/// several being changed together, for naming them apart. Returns the phrase for what
/// happened, or why nothing did.
fn change_one(composition: &mut Composition, id: Id, step: &Step, context: Context, number: Option<usize>) -> Result<String, String> {
    let layer = composition.layer(id).ok_or_else(|| "That layer is gone.".to_owned())?;
    let name = layer.name.clone();
    if layer.switches.locked {
        return Err(format!("“{name}” is locked, so I left it as it is."));
    }
    match step {
        Step::Rename(new_name) => {
            let new_name = match number {
                Some(number) => format!("{new_name} {number}"),
                None => new_name.clone(),
            };
            if new_name == name {
                return Err(format!("“{name}” is already called that."));
            }
            composition.layer_mut(id).expect("found above").name = new_name.clone();
            Ok(match number {
                Some(_) => "is renamed, with a number each".to_owned(),
                None => format!("is now called “{new_name}”"),
            })
        }
        Step::Stack(stack) => restack(composition, id, *stack, context.magnetic_main_track),
        Step::Place { across, down } => {
            let alignments = [
                across.map(|along| match along {
                    Along::Start => Alignment::Left,
                    Along::Middle => Alignment::HorizontalCenter,
                    Along::End => Alignment::Right,
                }),
                down.map(|along| match along {
                    Along::Start => Alignment::Top,
                    Along::Middle => Alignment::VerticalCenter,
                    Along::End => Alignment::Bottom,
                }),
            ];
            let animated = layer.transform.as_ref().is_some_and(|transform| transform.position.is_animated());
            let moved: usize = alignments.into_iter().flatten().map(|alignment| align_layers(composition, &[id], alignment, AlignTarget::Composition, context.playhead)).sum();
            if moved == 0 {
                return Err(format!("“{name}” has no picture to place."));
            }
            let place = place_words(*across, *down);
            Ok(if animated { format!("{place} (a keyframe at the playhead, since its position was already animated)") } else { place })
        }
        Step::Flip { horizontal } => {
            if !crate::edit::flip_layer(composition, id, *horizontal) {
                return Err(format!("“{name}” has no picture to flip."));
            }
            Ok(if *horizontal { "is flipped left to right" } else { "is flipped top to bottom" }.to_owned())
        }
        Step::Color { rgb, name: color_name } => {
            let color = Rgba::hex((u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2]));
            recolor(composition, id, color, context.playhead)?;
            Ok(format!("is {color_name}"))
        }
        Step::Resize(percent) => {
            let path = PropertyPath::Transform(TransformProperty::Scale);
            let layer = composition.layer_mut(id).expect("found above");
            let Some(PropertyValue::Pair(now)) = layer.property(path, context.playhead).map(|snapshot| snapshot.value) else {
                return Err(format!("“{name}” has no size to change."));
            };
            // Multiplied before dividing, so 100% made 20% bigger is exactly 120%.
            let scaled = |side: f32| side * *percent as f32 / 100.0;
            if !layer.set_property(path, context.playhead, PropertyValue::Pair(crate::animation::Vec2::new(scaled(now.x), scaled(now.y)))) {
                return Err(format!("“{name}” couldn't change size."));
            }
            Ok(match *percent {
                200 => "is twice the size".to_owned(),
                50 => "is half the size".to_owned(),
                percent if percent >= 100 => format!("is {}% bigger", percent - 100),
                percent => format!("is {}% smaller", 100 - percent),
            })
        }
        Step::Select | Step::Hide | Step::Show | Step::Lock | Step::Unlock | Step::Duplicate | Step::Delete => unreachable!("`one_step` carries these out itself"),
    }
}

/// Where a layer was put, as a phrase: "is in the top right corner", "is centered".
fn place_words(across: Option<Along>, down: Option<Along>) -> String {
    let side = |along: Along, start: &'static str, end: &'static str| match along {
        Along::Start => start,
        Along::Middle => "center",
        Along::End => end,
    };
    match (across, down) {
        (Some(Along::Middle), Some(Along::Middle)) => "is centered in the frame".to_owned(),
        (Some(Along::Middle), None) => "is centered across the frame".to_owned(),
        (None, Some(Along::Middle)) => "is centered up and down".to_owned(),
        (Some(across), None) => format!("is against the {} edge", side(across, "left", "right")),
        (None, Some(down)) => format!("is against the {} edge", side(down, "top", "bottom")),
        (Some(Along::Middle), Some(down)) => format!("is at the {} center", side(down, "top", "bottom")),
        (Some(across), Some(Along::Middle)) => format!("is at the {} center", side(across, "left", "right")),
        (Some(across), Some(down)) => format!("is in the {} {} corner", side(down, "top", "bottom"), side(across, "left", "right")),
        (None, None) => "is where it was".to_owned(),
    }
}

/// Gives a layer a plain color: text and captions their fill, a shape its fill, a solid its
/// color. Other layers have no color of their own.
fn recolor(composition: &mut Composition, id: Id, color: Rgba, playhead: i64) -> Result<(), String> {
    let layer = composition.layer_mut(id).ok_or_else(|| "That layer is gone.".to_owned())?;
    let name = layer.name.clone();
    match &mut layer.content {
        // The style is changed in place: a new color doesn't change the text's size, which
        // `edit::edit_text` would put back to an estimate until the viewer measures it again.
        LayerContent::Text { style, .. } | LayerContent::Captions { style: crate::captions::CaptionStyle { text: style, .. }, .. } => {
            style.fill = Some(color);
            // A color of its own is no longer the linked text style's.
            style.template = None;
            Ok(())
        }
        LayerContent::Shape { .. } => {
            crate::morph::edit_style_at(composition, id, playhead, |style| style.fill = Some(Paint::Solid(color)));
            Ok(())
        }
        LayerContent::Solid { color: now } if now.a > 0.0 => {
            *now = color;
            Ok(())
        }
        // A null object is a solid that draws nothing, and stays that way.
        LayerContent::Solid { .. } => Err(format!("“{name}” is a null object, which draws nothing, so it has no color.")),
        _ => Err(format!("“{name}” is a {} layer, which has no color of its own to change. Text, shapes and solids do.", crate::chat::kind(layer))),
    }
}

/// Moves a layer in the stacking order. Tracks decide what covers what, so the layer goes to a
/// track of its own above or below the others. With a magnetic main track, the main track
/// keeps its clips (they are packed from the start, which would move this one in time), so
/// "the back" is just above it.
fn restack(composition: &mut Composition, id: Id, stack: Stack, magnetic: bool) -> Result<String, String> {
    let layer = composition.layer(id).ok_or_else(|| "That layer is gone.".to_owned())?;
    let name = layer.name.clone();
    if tracks::is_sound_only(layer) {
        return Err(format!("“{name}” is sound, which is neither in front of nor behind the picture."));
    }
    let track = layer.track;
    let others: Vec<i32> = composition.layers.iter().filter(|other| other.id != id && !tracks::is_sound_only(other)).map(|other| other.track).collect();
    let floor = if magnetic { MAIN_TRACK + 1 } else { MAIN_TRACK };
    let on_packed_main = magnetic && track == MAIN_TRACK;
    let already = |place: &str| Err(format!("“{name}” is already {place}."));
    // The track it goes to, with every other picture track from there on moving up one.
    let to = match stack {
        Stack::Front => match others.iter().copied().max() {
            Some(top) if top > track => top + 1,
            _ => return already("in front of everything"),
        },
        Stack::Forward => match others.iter().copied().filter(|other| *other > track).min() {
            Some(next) => next + 1,
            None => return already("in front of everything"),
        },
        Stack::Back if on_packed_main => return already("on the main track, at the back"),
        Stack::Back => match others.iter().copied().filter(|other| *other >= floor).min() {
            Some(lowest) if lowest < track => floor,
            _ if magnetic => return already("behind everything but the main track, whose clips stay packed from the start"),
            _ => return already("behind everything"),
        },
        Stack::Backward if on_packed_main => return already("on the main track, at the back"),
        Stack::Backward => match others.iter().copied().filter(|other| *other < track && *other >= floor).max() {
            Some(previous) => previous,
            None if magnetic => return already("behind everything but the main track, whose clips stay packed from the start"),
            None => return already("behind everything"),
        },
    };
    let order_before: Vec<Id> = composition.layers.iter().map(|layer| layer.id).collect();
    let before = composition.clone();
    for other in composition.layers.iter_mut().filter(|other| other.id != id && !tracks::is_sound_only(other) && other.track >= to) {
        other.track += 1;
    }
    composition.layer_mut(id).expect("found above").track = to;
    tracks::normalize(composition);
    // A matte stays just above the layer it shapes, so moving one may change nothing.
    if composition.layers.iter().map(|layer| layer.id).eq(order_before) {
        *composition = before;
        return Err(format!("“{name}” is tied to the layer under it as its matte, so it stays where it is."));
    }
    Ok(match stack {
        Stack::Front => "is in front of everything",
        Stack::Forward => "is one step forward",
        Stack::Back if magnetic => "is behind everything but the main track",
        Stack::Back => "is behind everything",
        Stack::Backward => "is one step back",
    }
    .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::Vec2;
    use crate::edit;
    use crate::time::FrameRate;

    /// A 10-second 1920 × 1080 composition with, from the top: a title (text), a badge (a
    /// small ellipse) and a panel (a full-frame rectangle), each on a track of its own.
    fn scene() -> (Project, Id, [Id; 3]) {
        let mut project = Project::new(Vec::new(), Vec::new());
        let settings = edit::CompositionSettings { name: "Promo".into(), width: 1920, height: 1080, frame_rate: FrameRate::new(30, 1), duration_frames: 300, background: Rgba::hex(0x000000) };
        let comp = edit::new_composition(&mut project, &settings).unwrap();
        let rectangle = crate::model::ShapeStyle::new(crate::model::ShapeKind::Rectangle);
        let ellipse = crate::model::ShapeStyle::new(crate::model::ShapeKind::Ellipse);
        let panel = edit::add_shape_layer(&mut project, comp, rectangle, Vec2::new(1920.0, 1080.0), Vec2::new(960.0, 540.0), 0).unwrap();
        let badge = edit::add_shape_layer(&mut project, comp, ellipse, Vec2::new(100.0, 100.0), Vec2::new(400.0, 300.0), 0).unwrap();
        let title = edit::add_text_layer(&mut project, comp, "Big Sale", Vec2::new(960.0, 540.0), 0).unwrap();
        (project, comp, [title, badge, panel])
    }

    fn context(magnetic: bool) -> Context {
        Context { playhead: 0, magnetic_main_track: magnetic }
    }

    /// Reads `message` as housekeeping and carries it out on `layers`.
    fn run(project: &mut Project, comp: Id, layers: &[Id], message: &str) -> Outcome {
        let request = iris::layers::parse(message).unwrap_or_else(|| panic!("“{message}” should read"));
        carry_out(project, comp, layers, &request.steps, context(false), layers)
    }

    fn stack(project: &Project, comp: Id) -> Vec<Id> {
        project.composition(comp).unwrap().layers.iter().map(|layer| layer.id).collect()
    }

    #[test]
    fn hiding_locking_and_renaming_say_what_changed() {
        let (mut project, comp, [title, badge, _]) = scene();
        let outcome = run(&mut project, comp, &[title], "hide it and lock it");
        let layer = project.composition(comp).unwrap().layer(title).unwrap();
        assert!(!layer.switches.visible && layer.switches.locked);
        assert_eq!(outcome.done, vec!["is hidden".to_owned(), "is locked".to_owned()]);
        assert_eq!(outcome.selection, Some(Vec::new()), "a locked layer lets go of the selection");
        // Doing it again changes nothing, and says so instead of claiming it did.
        let outcome = run(&mut project, comp, &[title], "hide it");
        assert!(outcome.done.is_empty() && !outcome.changed && outcome.problems[0].contains("already hidden"), "{outcome:?}");
        // A locked layer is left as it is by everything but unlocking.
        let outcome = run(&mut project, comp, &[title], "rename it to Hero");
        assert!(outcome.problems[0].contains("locked"), "{outcome:?}");
        run(&mut project, comp, &[title], "unlock it");
        let outcome = run(&mut project, comp, &[title, badge], "rename it to Hero");
        let names: Vec<&str> = [title, badge].iter().map(|id| project.composition(comp).unwrap().layer(*id).unwrap().name.as_str()).collect();
        assert_eq!(names, ["Hero 1", "Hero 2"], "several get a number each");
        assert_eq!(outcome.who, ["Big Sale".to_owned(), "Ellipse 1".to_owned()], "named as they were");
    }

    #[test]
    fn duplicating_hands_the_copy_to_the_steps_after_it() {
        let (mut project, comp, [title, ..]) = scene();
        let outcome = run(&mut project, comp, &[title], "make a copy of it and flip it");
        let composition = project.composition(comp).unwrap();
        assert_eq!(composition.layers.len(), 4);
        let [copy] = outcome.layers.as_slice() else { panic!("one copy: {outcome:?}") };
        assert_ne!(*copy, title);
        let scale = |id: Id| composition.layer(id).unwrap().transform.as_ref().unwrap().scale.value;
        assert!(scale(*copy).x < 0.0 && scale(title).x > 0.0, "only the copy is flipped");
        assert_eq!(outcome.done, vec!["has a copy just above it".to_owned(), "the copy is flipped left to right".to_owned()]);
        assert_eq!(outcome.selection, Some(vec![*copy]));
    }

    #[test]
    fn deleting_leaves_locked_layers_and_ends_the_steps() {
        let (mut project, comp, [title, badge, panel]) = scene();
        project.composition_mut(comp).unwrap().layer_mut(panel).unwrap().switches.locked = true;
        let outcome = carry_out(&mut project, comp, &[title, panel], &[Step::Delete, Step::Hide], context(false), &[title, badge]);
        assert_eq!(stack(&project, comp), vec![badge, panel]);
        assert_eq!(outcome.done, vec!["is deleted".to_owned()], "nothing is left to hide");
        assert!(outcome.problems[0].contains("locked"));
        assert_eq!(outcome.selection, Some(vec![badge]));
    }

    #[test]
    fn the_stacking_order_changes_by_moving_tracks() {
        let (mut project, comp, [title, badge, panel]) = scene();
        assert_eq!(stack(&project, comp), vec![title, badge, panel]);
        let outcome = run(&mut project, comp, &[panel], "bring it to the front");
        assert_eq!(stack(&project, comp), vec![panel, title, badge]);
        assert_eq!(outcome.done, vec!["is in front of everything".to_owned()]);
        assert!(run(&mut project, comp, &[panel], "bring it to the front").problems[0].contains("already in front"));
        run(&mut project, comp, &[panel], "send it backward");
        assert_eq!(stack(&project, comp), vec![title, panel, badge]);
        run(&mut project, comp, &[panel], "move it forward");
        assert_eq!(stack(&project, comp), vec![panel, title, badge]);
        run(&mut project, comp, &[panel], "send it to the back");
        assert_eq!(stack(&project, comp), vec![title, badge, panel]);
        // Two at once keep their order among themselves.
        run(&mut project, comp, &[badge, panel], "bring them to the front");
        assert_eq!(stack(&project, comp), vec![badge, panel, title]);
    }

    #[test]
    fn a_magnetic_main_track_keeps_its_clips_at_the_back() {
        let (mut project, comp, [title, badge, panel]) = scene();
        let request = iris::layers::parse("send it to the back").unwrap();
        let outcome = carry_out(&mut project, comp, &[title], &request.steps, context(true), &[]);
        assert_eq!(stack(&project, comp), vec![badge, title, panel], "just above the main track");
        assert_eq!(outcome.done, vec!["is behind everything but the main track".to_owned()]);
        let outcome = carry_out(&mut project, comp, &[panel], &request.steps, context(true), &[]);
        assert!(outcome.done.is_empty() && outcome.problems[0].contains("main track"), "{outcome:?}");
    }

    #[test]
    fn layers_are_placed_in_the_frame_by_their_edges() {
        let (mut project, comp, [_, badge, _]) = scene();
        let bounds = |project: &Project| crate::align::layer_bounds(project.composition(comp).unwrap(), badge, 0).unwrap();
        let outcome = run(&mut project, comp, &[badge], "put it in the top right corner");
        assert!((bounds(&project).max.x - 1920.0).abs() < 0.5 && bounds(&project).min.y.abs() < 0.5, "{:?}", bounds(&project));
        assert_eq!(outcome.done, vec!["is in the top right corner".to_owned()]);
        run(&mut project, comp, &[badge], "center it");
        let center = bounds(&project).center();
        assert!((center.x - 960.0).abs() < 0.5 && (center.y - 540.0).abs() < 0.5, "{center:?}");
        // Only across: its height stays where it is.
        run(&mut project, comp, &[badge], "put it at the bottom");
        run(&mut project, comp, &[badge], "align left");
        assert!(bounds(&project).min.x.abs() < 0.5 && (bounds(&project).max.y - 1080.0).abs() < 0.5, "{:?}", bounds(&project));
    }

    #[test]
    fn colors_go_to_text_shapes_and_solids_only() {
        let (mut project, comp, [title, badge, _]) = scene();
        let solid = edit::add_solid_layer(&mut project, comp, Rgba::hex(0x112233), 0).unwrap();
        let null = edit::add_null_layer(&mut project, comp, 0).unwrap();
        let outcome = run(&mut project, comp, &[title, badge, solid, null], "make it red");
        let red = Rgba::hex(0xe53935);
        let composition = project.composition(comp).unwrap();
        assert!(matches!(&composition.layer(title).unwrap().content, LayerContent::Text { style, .. } if style.fill == Some(red)));
        assert!(matches!(&composition.layer(badge).unwrap().content, LayerContent::Shape { style, .. } if style.fill == Some(Paint::Solid(red))));
        assert!(matches!(composition.layer(solid).unwrap().content, LayerContent::Solid { color } if color == red));
        assert!(matches!(composition.layer(null).unwrap().content, LayerContent::Solid { color } if color.a == 0.0), "a null object still draws nothing");
        assert_eq!(outcome.done, vec!["is red".to_owned()]);
        assert!(outcome.problems.iter().any(|problem| problem.contains("null object")), "{:?}", outcome.problems);
    }

    #[test]
    fn bigger_and_smaller_scale_from_the_size_it_has() {
        let (mut project, comp, [title, ..]) = scene();
        let scale = |project: &Project| project.composition(comp).unwrap().layer(title).unwrap().transform.as_ref().unwrap().scale.value;
        let outcome = run(&mut project, comp, &[title], "make it bigger");
        assert_eq!((scale(&project), outcome.done), (Vec2::new(120.0, 120.0), vec!["is 20% bigger".to_owned()]));
        run(&mut project, comp, &[title], "make it half as big");
        assert_eq!(scale(&project), Vec2::new(60.0, 60.0));
    }

    #[test]
    fn selecting_changes_nothing_but_the_selection() {
        let (mut project, comp, [title, badge, panel]) = scene();
        project.composition_mut(comp).unwrap().layer_mut(panel).unwrap().switches.locked = true;
        let before = project.clone();
        let outcome = carry_out(&mut project, comp, &[title, badge, panel], &[Step::Select], context(false), &[]);
        assert_eq!(outcome.selection, Some(vec![title, badge]));
        assert!(!outcome.changed && project == before);
        assert!(outcome.problems[0].contains("locked"));
    }
}
