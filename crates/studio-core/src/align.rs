//! Aligning and distributing layers, as in the Align panel. Each layer is measured by the
//! upright box around it as it appears, so rotation, scale and parenting all count.

use crate::animation::Vec2;
use crate::model::{Composition, Id};

/// An axis-aligned rectangle in composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn center(self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: Vec2::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Vec2::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }
}

/// The axis-aligned box around a layer at `frame`, in composition pixels, including rotation and
/// parenting. `None` for layers without a picture (audio).
pub fn layer_bounds(composition: &Composition, layer_id: Id, frame: i64) -> Option<Rect> {
    let corners = crate::transform::layer_corners(composition, layer_id, frame as f64)?;
    let min = corners.iter().fold(Vec2::new(f32::MAX, f32::MAX), |m, c| Vec2::new(m.x.min(c.x), m.y.min(c.y)));
    let max = corners.iter().fold(Vec2::new(f32::MIN, f32::MIN), |m, c| Vec2::new(m.x.max(c.x), m.y.max(c.y)));
    Some(Rect { min, max })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

/// What layers are aligned against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignTarget {
    /// The composition frame.
    Composition,
    /// The combined bounds of the selected layers. Needs at least two layers.
    Selection,
}

/// How far `bounds` must move so its chosen edge or center lines up with `target`.
fn offset_to(bounds: Rect, target: Rect, alignment: Alignment) -> Vec2 {
    match alignment {
        Alignment::Left => Vec2::new(target.min.x - bounds.min.x, 0.0),
        Alignment::HorizontalCenter => Vec2::new(target.center().x - bounds.center().x, 0.0),
        Alignment::Right => Vec2::new(target.max.x - bounds.max.x, 0.0),
        Alignment::Top => Vec2::new(0.0, target.min.y - bounds.min.y),
        Alignment::VerticalCenter => Vec2::new(0.0, target.center().y - bounds.center().y),
        Alignment::Bottom => Vec2::new(0.0, target.max.y - bounds.max.y),
    }
}

/// Moves layers `ids` so they line up, by changing their position at `frame`
/// (adding a keyframe if position is animated). Locked layers and layers without
/// a picture are skipped. Returns how many layers moved.
pub fn align_layers(
    composition: &mut Composition,
    ids: &[Id],
    alignment: Alignment,
    target: AlignTarget,
    frame: i64,
) -> usize {
    let movable: Vec<(Id, Rect)> = composition
        .layers
        .iter()
        .filter(|layer| ids.contains(&layer.id) && !layer.switches.locked)
        .filter_map(|layer| layer_bounds(composition, layer.id, frame).map(|bounds| (layer.id, bounds)))
        .collect();

    let target_rect = match target {
        AlignTarget::Composition => Rect { min: Vec2::default(), max: composition.size() },
        AlignTarget::Selection if movable.len() >= 2 => {
            movable.iter().map(|(_, bounds)| *bounds).reduce(Rect::union).expect("at least two layers")
        }
        AlignTarget::Selection => return 0,
    };

    let mut moved = 0;
    for (id, bounds) in movable {
        let offset = crate::transform::parent_space(composition, id, frame as f64).apply_vector(offset_to(bounds, target_rect, alignment));
        let Some(transform) = composition.layer_mut(id).and_then(|layer| layer.transform.as_mut()) else { continue };
        let position = transform.position.value_at(frame as f64);
        transform.position.set_at(frame, position + offset);
        moved += 1;
    }
    moved
}

/// Which edge or center is spaced evenly when distributing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribution {
    Top,
    VerticalCenter,
    Bottom,
    Left,
    HorizontalCenter,
    Right,
}

/// The coordinate of `bounds` that `distribution` spaces evenly.
fn distribution_key(bounds: Rect, distribution: Distribution) -> f32 {
    match distribution {
        Distribution::Top => bounds.min.y,
        Distribution::VerticalCenter => bounds.center().y,
        Distribution::Bottom => bounds.max.y,
        Distribution::Left => bounds.min.x,
        Distribution::HorizontalCenter => bounds.center().x,
        Distribution::Right => bounds.max.x,
    }
}

/// Spaces three or more layers evenly between the outermost two, by the chosen edge or center.
/// The outermost layers stay put. Returns how many layers moved.
pub fn distribute_layers(composition: &mut Composition, ids: &[Id], distribution: Distribution, frame: i64) -> usize {
    let mut movable: Vec<(Id, f32)> = composition
        .layers
        .iter()
        .filter(|layer| ids.contains(&layer.id) && !layer.switches.locked)
        .filter_map(|layer| layer_bounds(composition, layer.id, frame).map(|bounds| (layer.id, distribution_key(bounds, distribution))))
        .collect();
    if movable.len() < 3 {
        return 0;
    }
    movable.sort_by(|a, b| a.1.total_cmp(&b.1));
    let (first, last) = (movable[0].1, movable[movable.len() - 1].1);
    let gap = (last - first) / (movable.len() - 1) as f32;
    let horizontal = matches!(distribution, Distribution::Left | Distribution::HorizontalCenter | Distribution::Right);

    let mut moved = 0;
    for (index, (id, key)) in movable.iter().enumerate().take(movable.len() - 1).skip(1) {
        let shift = first + gap * index as f32 - key;
        let offset = if horizontal { Vec2::new(shift, 0.0) } else { Vec2::new(0.0, shift) };
        let offset = crate::transform::parent_space(composition, *id, frame as f64).apply_vector(offset);
        let Some(transform) = composition.layer_mut(*id).and_then(|layer| layer.transform.as_mut()) else { continue };
        let position = transform.position.value_at(frame as f64);
        transform.position.set_at(frame, position + offset);
        moved += 1;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{self, ids};

    fn comp() -> Composition {
        sample::sample_project().composition(ids::MAIN_COMP).unwrap().clone()
    }

    #[test]
    fn bounds_follow_anchor_position_and_scale() {
        let comp = comp();
        // The logo is 1200 × 400, centered at (960, 300) and scaled to 50% from frame 72 on.
        let logo = comp.layers[2].id;
        let bounds = layer_bounds(&comp, logo, 100).unwrap();
        assert_eq!(bounds.min, Vec2::new(660.0, 200.0));
        assert_eq!(bounds.max, Vec2::new(1260.0, 400.0));
    }

    #[test]
    fn aligns_to_the_composition_edges() {
        let mut comp = comp();
        let logo = comp.layers[2].id;
        assert_eq!(align_layers(&mut comp, &[logo], Alignment::Left, AlignTarget::Composition, 100), 1);
        assert_eq!(layer_bounds(&comp, logo, 100).unwrap().min.x, 0.0);

        align_layers(&mut comp, &[logo], Alignment::Bottom, AlignTarget::Composition, 100);
        assert_eq!(layer_bounds(&comp, logo, 100).unwrap().max.y, 1080.0);
    }

    #[test]
    fn a_rotated_layer_aligns_by_the_box_around_it() {
        let mut comp = comp();
        let logo = comp.layers[2].id;
        comp.layer_mut(logo).unwrap().transform.as_mut().unwrap().rotation = crate::animation::Animated::constant(45.0);
        let upright = layer_bounds(&comp, logo, 100).unwrap();
        // A 600 × 200 box turned 45° spans (600 + 200) / √2 ≈ 565.7 pixels each way.
        assert!((upright.max.x - upright.min.x - 565.69).abs() < 0.1, "{upright:?}");
        assert!((upright.max.y - upright.min.y - 565.69).abs() < 0.1, "{upright:?}");
        align_layers(&mut comp, &[logo], Alignment::Left, AlignTarget::Composition, 100);
        align_layers(&mut comp, &[logo], Alignment::Top, AlignTarget::Composition, 100);
        let moved = layer_bounds(&comp, logo, 100).unwrap();
        assert!(moved.min.x.abs() < 0.01 && moved.min.y.abs() < 0.01, "its turned corners touch the edges: {moved:?}");
    }

    #[test]
    fn aligns_a_selection_to_its_combined_bounds() {
        let mut comp = comp();
        let (title, logo) = (comp.layers[0].id, comp.layers[2].id);
        align_layers(&mut comp, &[title, logo], Alignment::Top, AlignTarget::Selection, 100);
        let top = |id| layer_bounds(&comp, id, 100).unwrap().min.y;
        assert_eq!(top(title), top(logo));
    }

    #[test]
    fn skips_locked_layers_and_single_layer_selections() {
        let mut comp = comp();
        let logo = comp.layers[2].id;
        assert_eq!(align_layers(&mut comp, &[logo], Alignment::Left, AlignTarget::Selection, 100), 0);
        comp.layer_mut(logo).unwrap().switches.locked = true;
        assert_eq!(align_layers(&mut comp, &[logo], Alignment::Left, AlignTarget::Composition, 100), 0);
    }

    #[test]
    fn distributes_middle_layers_evenly() {
        let mut comp = comp();
        let ids: Vec<Id> = comp.layers[3..6].iter().map(|layer| layer.id).collect();
        // Spread three full-frame layers out horizontally first.
        for (index, id) in ids.iter().enumerate() {
            let transform = comp.layer_mut(*id).unwrap().transform.as_mut().unwrap();
            transform.position.set_at(0, Vec2::new([0.0, 100.0, 1000.0][index], 540.0));
        }
        assert_eq!(distribute_layers(&mut comp, &ids, Distribution::HorizontalCenter, 0), 1);
        let centers: Vec<f32> = ids.iter().map(|id| layer_bounds(&comp, *id, 0).unwrap().center().x).collect();
        assert!((centers[1] - centers[0] - (centers[2] - centers[1])).abs() < 1e-3);
        assert_eq!(distribute_layers(&mut comp, &ids[..2], Distribution::Left, 0), 0, "needs three layers");
    }

    #[test]
    fn animated_positions_gain_a_keyframe() {
        let mut comp = comp();
        let title = comp.layers[0].id;
        let before = comp.layer(title).unwrap().transform.as_ref().unwrap().position.keyframes().len();
        align_layers(&mut comp, &[title], Alignment::Right, AlignTarget::Composition, 100);
        let after = comp.layer(title).unwrap().transform.as_ref().unwrap().position.keyframes().len();
        assert_eq!(after, before + 1);
    }
}
