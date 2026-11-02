//! A uniform way to address and edit any animatable property of a layer, so the
//! Properties panel, Effect Controls, the timeline and the viewer all edit values
//! (and auto-keyframe them) through the same code.

use crate::animation::{Animated, Easing, Vec2};
use crate::model::{Id, Layer};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformProperty {
    AnchorPoint,
    Position,
    Scale,
    Rotation,
    Opacity,
}

impl TransformProperty {
    pub const ALL: [Self; 5] = [Self::AnchorPoint, Self::Position, Self::Scale, Self::Rotation, Self::Opacity];
}

/// Which property of a layer: one of its transform properties or a parameter of one of its effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropertyPath {
    Transform(TransformProperty),
    Effect { effect: Id, parameter: usize },
}

/// A property's value at one moment. Two-dimensional values edit each component separately.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropertyValue {
    Scalar(f32),
    Pair(Vec2),
}

impl PropertyValue {
    /// Component 0 is x (or the scalar), component 1 is y.
    pub fn component(self, index: usize) -> f32 {
        match (self, index) {
            (Self::Scalar(value), _) => value,
            (Self::Pair(value), 0) => value.x,
            (Self::Pair(value), _) => value.y,
        }
    }

    pub fn with_component(self, index: usize, component: f32) -> Self {
        match (self, index) {
            (Self::Scalar(_), _) => Self::Scalar(component),
            (Self::Pair(value), 0) => Self::Pair(Vec2::new(component, value.y)),
            (Self::Pair(value), _) => Self::Pair(Vec2::new(value.x, component)),
        }
    }

    pub fn components(self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::Pair(_) => 2,
        }
    }
}

/// How a property is shown and edited: its name, unit, drag sensitivity and allowed range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertyInfo {
    pub name: &'static str,
    pub unit: &'static str,
    /// Value change per pixel dragged.
    pub step: f32,
    pub min: f32,
    pub max: f32,
    pub decimals: usize,
}

impl PropertyInfo {
    /// For a parameter the effect catalog does not know (say, from a newer version of Studio):
    /// shown under its saved name with an unlimited range.
    pub const fn unknown() -> Self {
        Self { name: "", unit: "", step: 0.5, min: -1.0e6, max: 1.0e6, decimals: 2 }
    }
}

pub const fn transform_info(property: TransformProperty) -> PropertyInfo {
    let wide = 1.0e6;
    match property {
        TransformProperty::AnchorPoint => PropertyInfo { name: "Anchor Point", unit: "", step: 1.0, min: -wide, max: wide, decimals: 1 },
        TransformProperty::Position => PropertyInfo { name: "Position", unit: "", step: 1.0, min: -wide, max: wide, decimals: 1 },
        TransformProperty::Scale => PropertyInfo { name: "Scale", unit: "%", step: 0.5, min: -wide, max: wide, decimals: 1 },
        TransformProperty::Rotation => PropertyInfo { name: "Rotation", unit: "°", step: 0.5, min: -wide, max: wide, decimals: 1 },
        TransformProperty::Opacity => PropertyInfo { name: "Opacity", unit: "%", step: 0.5, min: 0.0, max: 100.0, decimals: 0 },
    }
}

/// Mutable access to one property, whatever its value type, for keyframe operations.
pub enum AnimatedMut<'a> {
    Scalar(&'a mut Animated<f32>),
    Pair(&'a mut Animated<Vec2>),
}

/// Runs the same expression on either kind of animated value.
macro_rules! on_animated {
    ($target:expr, $animated:ident => $body:expr) => {
        match $target {
            AnimatedMut::Scalar($animated) => $body,
            AnimatedMut::Pair($animated) => $body,
        }
    };
}

impl AnimatedMut<'_> {
    pub fn toggle_stopwatch(self, frame: i64) {
        on_animated!(self, animated => animated.toggle_stopwatch(frame))
    }
    pub fn toggle_keyframe(self, frame: i64) {
        on_animated!(self, animated => animated.toggle_keyframe(frame))
    }
    pub fn remove_keyframe(self, frame: i64) {
        on_animated!(self, animated => animated.remove_keyframe(frame))
    }
    pub fn move_keyframe(self, from: i64, to: i64) -> bool {
        on_animated!(self, animated => animated.move_keyframe(from, to))
    }
    pub fn set_easing(self, frame: i64, easing: Easing) {
        on_animated!(self, animated => animated.set_easing(frame, easing))
    }
}

/// Read-only facts about a property that do not depend on its value type.
#[derive(Clone, Debug, PartialEq)]
pub struct PropertySnapshot {
    pub path: PropertyPath,
    /// The name shown in the UI. Effect parameters keep the name saved with them, so it is
    /// right even for an effect the catalog does not know.
    pub name: String,
    pub info: PropertyInfo,
    pub value: PropertyValue,
    pub keyframes: Vec<i64>,
}

impl Layer {
    fn effect_parameter(&self, effect: Id, parameter: usize) -> Option<&crate::model::EffectProperty> {
        self.effects.iter().find(|candidate| candidate.id == effect)?.properties.get(parameter)
    }

    /// The property's value, metadata and keyframe frames at `frame`, or `None` if the layer lacks it.
    pub fn property(&self, path: PropertyPath, frame: i64) -> Option<PropertySnapshot> {
        let at = frame as f64;
        let (name, info, value, keyframes) = match path {
            PropertyPath::Transform(property) => {
                let transform = self.transform.as_ref()?;
                let (value, keyframes) = match property {
                    TransformProperty::AnchorPoint => {
                        (PropertyValue::Pair(transform.anchor_point.value_at(at)), transform.anchor_point.keyframe_frames())
                    }
                    TransformProperty::Position => {
                        (PropertyValue::Pair(transform.position.value_at(at)), transform.position.keyframe_frames())
                    }
                    TransformProperty::Scale => {
                        (PropertyValue::Pair(transform.scale.value_at(at)), transform.scale.keyframe_frames())
                    }
                    TransformProperty::Rotation => {
                        (PropertyValue::Scalar(transform.rotation.value_at(at)), transform.rotation.keyframe_frames())
                    }
                    TransformProperty::Opacity => {
                        (PropertyValue::Scalar(transform.opacity.value_at(at)), transform.opacity.keyframe_frames())
                    }
                };
                let info = transform_info(property);
                (info.name.to_owned(), info, value, keyframes)
            }
            PropertyPath::Effect { effect, parameter } => {
                let property = self.effect_parameter(effect, parameter)?;
                let value = PropertyValue::Scalar(property.value.value_at(at));
                (property.name.clone(), property.info, value, property.value.keyframe_frames())
            }
        };
        Some(PropertySnapshot { path, name, info, value, keyframes })
    }

    /// Mutable access to a property for keyframe operations.
    pub fn animated_mut(&mut self, path: PropertyPath) -> Option<AnimatedMut<'_>> {
        match path {
            PropertyPath::Transform(property) => {
                let transform = self.transform.as_mut()?;
                Some(match property {
                    TransformProperty::AnchorPoint => AnimatedMut::Pair(&mut transform.anchor_point),
                    TransformProperty::Position => AnimatedMut::Pair(&mut transform.position),
                    TransformProperty::Scale => AnimatedMut::Pair(&mut transform.scale),
                    TransformProperty::Rotation => AnimatedMut::Scalar(&mut transform.rotation),
                    TransformProperty::Opacity => AnimatedMut::Scalar(&mut transform.opacity),
                })
            }
            PropertyPath::Effect { effect, parameter } => {
                let effect = self.effects.iter_mut().find(|candidate| candidate.id == effect)?;
                Some(AnimatedMut::Scalar(&mut effect.properties.get_mut(parameter)?.value))
            }
        }
    }

    /// Sets a property's value at `frame`, clamped to its allowed range. Animated properties get a
    /// keyframe at `frame` (auto-keyframing). Returns false if the value type does not match.
    pub fn set_property(&mut self, path: PropertyPath, frame: i64, value: PropertyValue) -> bool {
        let Some(info) = self.property(path, frame).map(|snapshot| snapshot.info) else { return false };
        let clamp = |v: f32| if v.is_finite() { v.clamp(info.min, info.max) } else { info.min.max(0.0) };
        match (self.animated_mut(path), value) {
            (Some(AnimatedMut::Scalar(animated)), PropertyValue::Scalar(v)) => animated.set_at(frame, clamp(v)),
            (Some(AnimatedMut::Pair(animated)), PropertyValue::Pair(v)) => {
                animated.set_at(frame, Vec2::new(clamp(v.x), clamp(v.y)));
            }
            _ => return false,
        }
        true
    }

    /// Each keyframe of a property with the easing of the segment leaving it.
    pub fn keyframe_easings(&self, path: PropertyPath) -> Vec<(i64, Easing)> {
        match path {
            PropertyPath::Transform(property) => {
                let Some(transform) = self.transform.as_ref() else { return Vec::new() };
                match property {
                    TransformProperty::AnchorPoint => transform.anchor_point.keyframe_easings(),
                    TransformProperty::Position => transform.position.keyframe_easings(),
                    TransformProperty::Scale => transform.scale.keyframe_easings(),
                    TransformProperty::Rotation => transform.rotation.keyframe_easings(),
                    TransformProperty::Opacity => transform.opacity.keyframe_easings(),
                }
            }
            PropertyPath::Effect { effect, parameter } => self
                .effect_parameter(effect, parameter)
                .map_or_else(Vec::new, |property| property.value.keyframe_easings()),
        }
    }

    /// Moves the layer `delta` frames later (or earlier, if negative), keyframes included.
    pub fn shift_in_time(&mut self, delta: i64) {
        self.in_frame += delta;
        self.out_frame += delta;
        self.start_frame += delta;
        // Speed ramp keyframes are in composition frames too.
        self.time.speed.shift(delta);
        if let Some(transform) = self.transform.as_mut() {
            transform.anchor_point.shift(delta);
            transform.position.shift(delta);
            transform.scale.shift(delta);
            transform.rotation.shift(delta);
            transform.opacity.shift(delta);
        }
        for effect in &mut self.effects {
            for property in &mut effect.properties {
                property.value.shift(delta);
            }
        }
    }

    /// Every animatable property of the layer, in display order.
    pub fn property_paths(&self) -> Vec<PropertyPath> {
        let mut paths = Vec::new();
        if self.transform.is_some() {
            paths.extend(TransformProperty::ALL.map(PropertyPath::Transform));
        }
        for effect in &self.effects {
            paths.extend((0..effect.properties.len()).map(|parameter| PropertyPath::Effect { effect: effect.id, parameter }));
        }
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ids, sample_project};

    fn logo() -> Layer {
        sample_project().composition(ids::MAIN_COMP).unwrap().layers[2].clone()
    }

    #[test]
    fn reads_and_sets_transform_components() {
        let mut layer = logo();
        let path = PropertyPath::Transform(TransformProperty::Position);
        let before = layer.property(path, 0).unwrap().value;
        assert!(layer.set_property(path, 0, before.with_component(0, 123.0)));
        assert_eq!(layer.property(path, 0).unwrap().value.component(0), 123.0);
        assert_eq!(layer.property(path, 0).unwrap().value.component(1), before.component(1));
    }

    #[test]
    fn values_are_clamped_and_type_checked() {
        let mut layer = logo();
        let opacity = PropertyPath::Transform(TransformProperty::Opacity);
        layer.set_property(opacity, 0, PropertyValue::Scalar(250.0));
        assert_eq!(layer.property(opacity, 0).unwrap().value, PropertyValue::Scalar(100.0));
        assert!(!layer.set_property(opacity, 0, PropertyValue::Pair(Vec2::default())));
        layer.set_property(opacity, 0, PropertyValue::Scalar(f32::NAN));
        assert_eq!(layer.property(opacity, 0).unwrap().value, PropertyValue::Scalar(0.0), "NaN never gets stored");
    }

    #[test]
    fn animated_properties_auto_keyframe() {
        let mut layer = logo();
        let scale = PropertyPath::Transform(TransformProperty::Scale);
        layer.set_property(scale, 100, PropertyValue::Pair(Vec2::new(80.0, 80.0)));
        assert_eq!(layer.property(scale, 0).unwrap().keyframes, vec![48, 72, 100]);
    }

    #[test]
    fn shifting_moves_the_layer_and_its_keyframes() {
        let mut layer = logo();
        let scale = PropertyPath::Transform(TransformProperty::Scale);
        layer.shift_in_time(10);
        assert_eq!((layer.in_frame, layer.out_frame), (58, 250));
        assert_eq!(layer.property(scale, 0).unwrap().keyframes, vec![58, 82]);
        assert_eq!(layer.keyframe_easings(scale).len(), 2);
    }

    #[test]
    fn effect_parameters_are_addressable() {
        let mut layer = logo();
        let glow = layer.effects[0].id;
        let radius = PropertyPath::Effect { effect: glow, parameter: 0 };
        layer.set_property(radius, 0, PropertyValue::Scalar(40.0));
        assert_eq!(layer.property(radius, 0).unwrap().value, PropertyValue::Scalar(40.0));
        assert_eq!(layer.property_paths().len(), 5 + layer.effects[0].properties.len());
    }
}
