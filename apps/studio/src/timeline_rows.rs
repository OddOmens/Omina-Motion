//! The timeline shows one flat list of rows: layers, plus the property groups
//! and properties of any layer that is twirled open. The layer outline and the
//! time graph both render this list, so their rows always line up.

use std::collections::HashSet;

use studio_core::animation::Easing;
use studio_core::model::{Id, Layer};
use studio_core::property::{PropertyPath, TransformProperty};

/// One timeline row. Property rows carry their keyframe frames for the time graph.
#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Layer { index: usize },
    Group { name: String, is_effect: bool },
    /// An animatable property: where it lives, its display name, and its keyframes with their easing.
    Property { path: PropertyPath, name: String, keyframes: Vec<(i64, Easing)> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineRow {
    /// Stable key used for expand/collapse state and element ids.
    pub key: String,
    pub layer: Id,
    pub depth: usize,
    pub kind: RowKind,
}

pub fn layer_key(layer: Id) -> String {
    format!("layer/{}", layer.0)
}

pub fn effects_key(layer: Id) -> String {
    format!("layer/{}/effects", layer.0)
}

fn transform_key(layer: Id) -> String {
    format!("layer/{}/transform", layer.0)
}

fn effect_key(layer: Id, effect: Id) -> String {
    format!("layer/{}/fx/{}", layer.0, effect.0)
}

/// A row for one property of `layer`, or `None` if the layer lacks it.
fn property_row(layer: &Layer, path: PropertyPath, key: String, depth: usize) -> Option<TimelineRow> {
    let snapshot = layer.property(path, 0)?;
    Some(TimelineRow {
        key,
        layer: layer.id,
        depth,
        kind: RowKind::Property { path, name: snapshot.name.clone(), keyframes: layer.keyframe_easings(path) },
    })
}

/// Builds the visible rows for `layers`, honoring which rows are expanded.
pub fn build_rows(layers: &[&Layer], expanded: &HashSet<String>) -> Vec<TimelineRow> {
    let mut rows = Vec::new();
    for (position, layer) in layers.iter().enumerate() {
        let key = layer_key(layer.id);
        let is_open = expanded.contains(&key);
        rows.push(TimelineRow { key, layer: layer.id, depth: 0, kind: RowKind::Layer { index: position + 1 } });
        if !is_open {
            continue;
        }

        if !layer.effects.is_empty() {
            let key = effects_key(layer.id);
            let effects_open = expanded.contains(&key);
            rows.push(TimelineRow { key, layer: layer.id, depth: 1, kind: RowKind::Group { name: "Effects".into(), is_effect: false } });
            if effects_open {
                for effect in &layer.effects {
                    let key = effect_key(layer.id, effect.id);
                    let effect_open = expanded.contains(&key);
                    rows.push(TimelineRow {
                        key: key.clone(),
                        layer: layer.id,
                        depth: 2,
                        kind: RowKind::Group { name: effect.name.clone(), is_effect: true },
                    });
                    if effect_open {
                        for parameter in 0..effect.properties.len() {
                            let path = PropertyPath::Effect { effect: effect.id, parameter };
                            rows.extend(property_row(layer, path, format!("{key}/{parameter}"), 3));
                        }
                    }
                }
            }
        }

        if layer.transform.is_some() {
            let key = transform_key(layer.id);
            let transform_open = expanded.contains(&key);
            rows.push(TimelineRow { key, layer: layer.id, depth: 1, kind: RowKind::Group { name: "Transform".into(), is_effect: false } });
            if transform_open {
                for property in TransformProperty::ALL {
                    let path = PropertyPath::Transform(property);
                    rows.extend(property_row(layer, path, format!("{}/{property:?}", transform_key(layer.id)), 2));
                }
            }
        }
    }
    rows
}

/// Where a drop at visible row `row_index` lands: the layer under the pointer (effects drop onto it)
/// and the stack index a new layer is inserted at (directly above that layer, or at the bottom).
pub fn drop_target(rows: &[TimelineRow], layers: &[Layer], row_index: usize) -> (Option<Id>, usize, usize) {
    let Some(row) = rows.get(row_index) else { return (None, layers.len(), rows.len()) };
    let insert_index = layers.iter().position(|layer| layer.id == row.layer).unwrap_or(layers.len());
    let indicator_row = rows.iter().position(|candidate| candidate.layer == row.layer).unwrap_or(row_index);
    (Some(row.layer), insert_index, indicator_row)
}

/// Every keyframe inside a box selection: visible rows `rows[first_row..=last_row]` and frames
/// `first_frame..=last_frame`. Row and frame bounds may be given in either order.
pub fn keyframes_in_box(rows: &[TimelineRow], row_a: usize, row_b: usize, frame_a: f64, frame_b: f64) -> Vec<crate::state::KeyframeRef> {
    let (first_row, last_row) = (row_a.min(row_b), row_a.max(row_b));
    let (first_frame, last_frame) = (frame_a.min(frame_b), frame_a.max(frame_b));
    rows.iter()
        .enumerate()
        .filter(|(index, _)| (first_row..=last_row).contains(index))
        .flat_map(|(_, row)| match &row.kind {
            RowKind::Property { path, keyframes, .. } => keyframes
                .iter()
                .filter(|(frame, _)| (first_frame..=last_frame).contains(&(*frame as f64)))
                .map(|(frame, _)| crate::state::KeyframeRef { layer: row.layer, path: *path, frame: *frame })
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::sample::{ids, sample_project};

    fn layers() -> Vec<Layer> {
        sample_project().composition(ids::MAIN_COMP).unwrap().layers.clone()
    }

    fn names(rows: &[TimelineRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match &row.kind {
                RowKind::Layer { .. } => "layer".to_owned(),
                RowKind::Group { name, .. } | RowKind::Property { name, .. } => name.clone(),
            })
            .collect()
    }

    #[test]
    fn collapsed_layers_show_one_row_each() {
        let layers = layers();
        let refs: Vec<&Layer> = layers.iter().collect();
        assert_eq!(build_rows(&refs, &HashSet::new()).len(), layers.len());
    }

    #[test]
    fn expanding_reveals_groups_then_properties() {
        let layers = layers();
        let logo = &layers[2];
        let mut expanded = HashSet::from([layer_key(logo.id)]);
        assert_eq!(names(&build_rows(&[logo], &expanded)), ["layer", "Effects", "Transform"]);
        expanded.insert(transform_key(logo.id));
        let rows = build_rows(&[logo], &expanded);
        assert_eq!(names(&rows)[3..], ["Anchor Point", "Position", "Scale", "Rotation", "Opacity"]);
        let RowKind::Property { keyframes, .. } = &rows[5].kind else { panic!("scale row") };
        let frames: Vec<i64> = keyframes.iter().map(|(frame, _)| *frame).collect();
        assert_eq!(frames, [48, 72]);
    }

    #[test]
    fn row_keys_are_unique() {
        let layers = layers();
        let refs: Vec<&Layer> = layers.iter().collect();
        let expanded: HashSet<String> =
            layers.iter().flat_map(|l| [layer_key(l.id), transform_key(l.id), effects_key(l.id)]).collect();
        let rows = build_rows(&refs, &expanded);
        let keys: HashSet<&String> = rows.iter().map(|row| &row.key).collect();
        assert_eq!(keys.len(), rows.len());
    }

    #[test]
    fn box_selection_picks_keyframes_inside_the_box() {
        let layers = layers();
        let title = &layers[0];
        let expanded = HashSet::from([layer_key(title.id), transform_key(title.id)]);
        let rows = build_rows(&[title], &expanded);
        // Rows: 0 layer, 1 Transform, 2 Anchor, 3 Position, 4 Scale, 5 Rotation, 6 Opacity.
        let picked = keyframes_in_box(&rows, 6, 3, 30.0, 0.0);
        let frames: Vec<i64> = picked.iter().map(|key| key.frame).collect();
        assert_eq!(frames, vec![24, 24], "position and opacity keyframes at 24; 36 and 48 are outside");
        assert!(keyframes_in_box(&rows, 0, 2, 0.0, 240.0).is_empty(), "no keyframes on those rows");
    }

    #[test]
    fn drops_insert_above_the_hovered_layer_or_at_the_bottom() {
        let layers = layers();
        let refs: Vec<&Layer> = layers.iter().collect();
        let rows = build_rows(&refs, &HashSet::new());
        assert_eq!(drop_target(&rows, &layers, 2), (Some(layers[2].id), 2, 2));
        assert_eq!(drop_target(&rows, &layers, 99), (None, layers.len(), rows.len()));
    }
}
