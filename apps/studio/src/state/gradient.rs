//! Editing a shape's gradient: its color stops (from the gradient bar in Properties) and where
//! it runs (from the two handles in the viewer). See `studio_core::gradient`.

use studio_core::animation::Vec2;
use studio_core::edit;
use studio_core::gradient::Gradient;
use studio_core::model::{Id, LayerContent};
use studio_core::transform::world_transform;

use super::viewer::ViewerDrag;
use super::{EditorState, PaintSlot};

/// One end of a gradient's line: where it starts, or where it ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradientEnd {
    From,
    To,
}

/// A shape's gradient as shown in the viewer: which paint it is, and its two ends in
/// composition pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenGradient {
    pub slot: PaintSlot,
    pub from: Vec2,
    pub to: Vec2,
}

impl EditorState {
    /// The gradient of a shape's fill or stroke, if that paint is a gradient.
    pub fn shape_gradient(&self, layer: Id, slot: PaintSlot) -> Option<Gradient> {
        self.shape_paint(layer, slot).and_then(|paint| paint.gradient())
    }

    /// Changes a shape's gradient in place; paints that aren't gradients are left alone.
    /// Returns what `change` returned, or `None` when there was no gradient.
    fn edit_gradient<T>(&mut self, layer: Id, slot: PaintSlot, change: impl FnOnce(&mut Gradient) -> T) -> Option<T> {
        let paint = self.shape_paint(layer, slot)?;
        let mut gradient = paint.gradient()?;
        let result = change(&mut gradient);
        self.set_shape_paint(layer, slot, Some(paint.with_gradient(gradient)));
        Some(result)
    }

    /// Adds a stop at `at` (0–1) in the color the gradient has there. Returns its index, or
    /// `None` when the gradient is full or there is none.
    pub fn add_gradient_stop(&mut self, layer: Id, slot: PaintSlot, at: f32) -> Option<usize> {
        self.edit_gradient(layer, slot, |gradient| gradient.add_stop(at)).flatten()
    }

    /// Removes stop `index`; a gradient keeps at least two. Returns whether it went.
    pub fn remove_gradient_stop(&mut self, layer: Id, slot: PaintSlot, index: usize) -> bool {
        self.edit_gradient(layer, slot, |gradient| gradient.remove_stop(index)).unwrap_or(false)
    }

    /// Moves stop `index` to `at` (0–1). Returns the stop's index afterwards, since it may pass
    /// others.
    pub fn move_gradient_stop(&mut self, layer: Id, slot: PaintSlot, index: usize, at: f32) -> usize {
        self.edit_gradient(layer, slot, |gradient| gradient.move_stop(index, at)).unwrap_or(index)
    }

    /// Sets where stop `index` sits (0–1), kept between the stops on either side so a typed
    /// location never reorders them.
    pub fn set_gradient_stop_location(&mut self, layer: Id, slot: PaintSlot, index: usize, at: f32) {
        self.edit_gradient(layer, slot, |gradient| {
            let stops = gradient.stops();
            let low = index.checked_sub(1).and_then(|before| stops.get(before)).map_or(0.0, |stop| stop.at);
            let high = stops.get(index + 1).map_or(1.0, |stop| stop.at);
            if at.is_finite() {
                gradient.move_stop(index, at.clamp(low, high));
            }
        });
    }

    /// Flips the gradient so its colors run the other way.
    pub fn reverse_gradient(&mut self, layer: Id, slot: PaintSlot) {
        self.edit_gradient(layer, slot, |gradient| *gradient = gradient.reversed());
    }

    /// The gradient to show handles for on a selected shape: its fill's, or else its stroke's,
    /// with its ends in composition pixels at the playhead.
    pub fn gradient_on_screen(&self, layer: Id) -> Option<ScreenGradient> {
        let comp = self.composition();
        let target = comp.layer(layer)?;
        if !matches!(target.content, LayerContent::Shape { .. }) {
            return None;
        }
        let slot = [PaintSlot::Fill, PaintSlot::Stroke].into_iter().find(|&slot| self.shape_gradient(layer, slot).is_some())?;
        let gradient = self.shape_gradient(layer, slot)?;
        let world = world_transform(comp, layer, self.current_frame as f64)?;
        let size = target.size;
        let place = |v: Vec2| world.apply(Vec2::new(v.x * size.x, v.y * size.y));
        Some(ScreenGradient { slot, from: place(gradient.from), to: place(gradient.to) })
    }

    /// Starts dragging one end of a shape's gradient in the viewer.
    pub fn begin_gradient_drag(&mut self, layer: Id, end: GradientEnd) {
        let Some(shown) = self.gradient_on_screen(layer) else { return };
        if self.composition().layer(layer).is_none_or(|target| target.switches.locked) {
            return;
        }
        let start = match end {
            GradientEnd::From => shown.from,
            GradientEnd::To => shown.to,
        };
        self.viewer_drag = Some(ViewerDrag::GradientPoint { layer, slot: shown.slot, end, start });
    }

    /// Puts one end of a gradient at `point` (composition pixels), through the layer's
    /// transform into fractions of its box.
    pub(super) fn place_gradient_end(&mut self, layer: Id, slot: PaintSlot, end: GradientEnd, point: Vec2) {
        let comp = self.composition();
        let Some(size) = comp.layer(layer).map(|target| target.size).filter(|size| size.x > 0.0 && size.y > 0.0) else { return };
        let Some(to_local) = world_transform(comp, layer, self.current_frame as f64).and_then(|world| world.inverse()) else { return };
        let local = to_local.apply(point);
        let fraction = Vec2::new(local.x / size.x, local.y / size.y);
        self.edit_gradient(layer, slot, |gradient| match end {
            GradientEnd::From => gradient.from = fraction,
            GradientEnd::To => gradient.to = fraction,
        });
        // Keep the shape's other settings tidy, as any shape edit does.
        edit::edit_shape(self.composition_mut(), layer, |_| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studio_core::model::{Paint, Rgba};

    /// A frame-filling rectangle with a black-to-white linear fill.
    fn gradient_shape() -> (EditorState, Id) {
        let mut state = EditorState::new();
        let layer = state.add_background().unwrap();
        state.set_shape_paint(layer, PaintSlot::Fill, Some(Paint::linear(Rgba::hex(0x000000), Rgba::hex(0xffffff), 0.0)));
        (state, layer)
    }

    #[test]
    fn stops_are_added_moved_and_removed() {
        let (mut state, layer) = gradient_shape();
        let index = state.add_gradient_stop(layer, PaintSlot::Fill, 0.5).unwrap();
        assert_eq!(state.shape_gradient(layer, PaintSlot::Fill).unwrap().stops().len(), 3);
        let index = state.move_gradient_stop(layer, PaintSlot::Fill, index, 0.25);
        assert_eq!(state.shape_gradient(layer, PaintSlot::Fill).unwrap().stops()[index].at, 0.25);
        assert!(state.remove_gradient_stop(layer, PaintSlot::Fill, index));
        assert_eq!(state.shape_gradient(layer, PaintSlot::Fill).unwrap().stops().len(), 2);
    }

    #[test]
    fn dragging_an_end_in_the_viewer_moves_it_on_the_shape() {
        let (mut state, layer) = gradient_shape();
        let shown = state.gradient_on_screen(layer).unwrap();
        state.begin_gradient_drag(layer, GradientEnd::To);
        // Drag the end back to the middle of the shape.
        let middle = (shown.from + shown.to) * 0.5;
        state.drag_in_viewer(middle - shown.to, false);
        state.end_viewer_drag();
        let gradient = state.shape_gradient(layer, PaintSlot::Fill).unwrap();
        assert!((gradient.to.x - 0.5).abs() < 1e-3 && (gradient.to.y - 0.5).abs() < 1e-3, "{gradient:?}");
        assert_eq!(gradient.from, studio_core::gradient::LINEAR_FROM, "the other end stays");
    }
}
