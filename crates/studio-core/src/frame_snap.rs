//! Snapping a box that is being moved in the viewer. While a layer is dragged, the edges and
//! middle of its bounding box are pulled onto the nearest line of interest within reach:
//!
//! - the composition's exact center (dead center), which pulls on both axes at once;
//! - the composition's vertical and horizontal center lines, each on its own axis;
//! - the lines of the grid.
//!
//! Each kind can be turned off. The result says how far to nudge the box and which lines it
//! landed on, so the viewer can draw them as guides. (Snapping in time, in the timeline, is
//! in `snap`.) Omina Edit snaps moved layers the same way.

use crate::animation::Vec2;

/// Dead center pulls from a little further away than the other lines, so dragging toward the
/// middle settles there instead of catching on just one of the two center lines first.
const DEAD_CENTER_PULL: f32 = 1.5;

/// What a moved box snaps to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnapOptions {
    /// The grid's spacing in composition pixels, or `None` when the box doesn't snap to a grid.
    pub grid: Option<f32>,
    /// Whether the box's middle snaps to the frame's vertical and horizontal center lines.
    pub center_lines: bool,
    /// Whether the box's middle snaps to the exact center of the frame, on both axes at once.
    pub dead_center: bool,
}

/// Where a moved box snapped.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Snapped {
    /// How far to move the box (added to the drag) so it sits on the lines below.
    pub nudge: Vec2,
    /// The x of the vertical line it snapped to, in composition pixels, if any.
    pub vertical: Option<f32>,
    /// The y of the horizontal line it snapped to, in composition pixels, if any.
    pub horizontal: Option<f32>,
    /// Whether it snapped to the frame's exact center.
    pub dead_center: bool,
}

/// Snaps a box running from `min` to `max` (where it is after the drag so far) in a frame
/// `size` across, whose top-left corner is at the origin. `reach` is how near, in composition
/// pixels, a line must be to pull the box onto it; the viewer sets it from the zoom so the
/// pull feels the same at any magnification.
pub fn snap_box(min: Vec2, max: Vec2, size: Vec2, options: SnapOptions, reach: f32) -> Snapped {
    if reach <= 0.0 {
        return Snapped::default();
    }
    let middle = (min + max) * 0.5;
    let frame_middle = size * 0.5;

    if options.dead_center && (frame_middle - middle).length() <= reach * DEAD_CENTER_PULL {
        return Snapped { nudge: frame_middle - middle, vertical: Some(frame_middle.x), horizontal: Some(frame_middle.y), dead_center: true };
    }

    let across = snap_axis(min.x, max.x, frame_middle.x, options, reach);
    let down = snap_axis(min.y, max.y, frame_middle.y, options, reach);
    Snapped {
        nudge: Vec2::new(across.map_or(0.0, |(nudge, _)| nudge), down.map_or(0.0, |(nudge, _)| nudge)),
        vertical: across.map(|(_, line)| line),
        horizontal: down.map(|(_, line)| line),
        dead_center: false,
    }
}

/// Snaps one axis of a box running from `start` to `end`: the nearest line within `reach`
/// to either edge or the middle, as (nudge, line). The center line wins a tie, since it is
/// the one people aim for.
fn snap_axis(start: f32, end: f32, frame_middle: f32, options: SnapOptions, reach: f32) -> Option<(f32, f32)> {
    let middle = (start + end) * 0.5;
    let mut best: Option<(f32, f32)> = None;
    let mut consider = |from: f32, line: f32| {
        let nudge = line - from;
        if nudge.abs() <= reach && best.is_none_or(|(current, _)| nudge.abs() < current.abs()) {
            best = Some((nudge, line));
        }
    };
    if options.center_lines {
        consider(middle, frame_middle);
    }
    if let Some(spacing) = options.grid.filter(|spacing| *spacing > 0.0) {
        for edge in [start, middle, end] {
            consider(edge, (edge / spacing).round() * spacing);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Vec2 = Vec2 { x: 1000.0, y: 800.0 };
    const EVERYTHING: SnapOptions = SnapOptions { grid: Some(50.0), center_lines: true, dead_center: true };

    fn boxed(x: f32, y: f32, width: f32, height: f32) -> (Vec2, Vec2) {
        (Vec2::new(x, y), Vec2::new(x + width, y + height))
    }

    #[test]
    fn a_box_near_the_middle_lands_on_dead_center() {
        // Its middle is at (497, 404): within reach of (500, 400) on both axes.
        let (min, max) = boxed(447.0, 354.0, 100.0, 100.0);
        let snapped = snap_box(min, max, FRAME, EVERYTHING, 6.0);
        assert_eq!(snapped.nudge, Vec2::new(3.0, -4.0));
        assert!(snapped.dead_center);
        assert_eq!((snapped.vertical, snapped.horizontal), (Some(500.0), Some(400.0)));
    }

    #[test]
    fn center_lines_snap_each_axis_on_its_own() {
        // Middle at (502, 250): near the vertical center line only.
        let options = SnapOptions { center_lines: true, ..Default::default() };
        let (min, max) = boxed(452.0, 200.0, 100.0, 100.0);
        let snapped = snap_box(min, max, FRAME, options, 6.0);
        assert_eq!(snapped.nudge, Vec2::new(-2.0, 0.0));
        assert_eq!((snapped.vertical, snapped.horizontal, snapped.dead_center), (Some(500.0), None, false));
    }

    #[test]
    fn edges_snap_to_the_nearest_grid_line() {
        let options = SnapOptions { grid: Some(50.0), ..Default::default() };
        // The left edge is 3 past a grid line, and the top edge 2 short of one.
        let (min, max) = boxed(103.0, 248.0, 30.0, 30.0);
        let snapped = snap_box(min, max, FRAME, options, 5.0);
        assert_eq!(snapped.nudge, Vec2::new(-3.0, 2.0));
        assert_eq!((snapped.vertical, snapped.horizontal), (Some(100.0), Some(250.0)));
    }

    #[test]
    fn nothing_snaps_when_it_is_off_or_out_of_reach() {
        let (min, max) = boxed(447.0, 354.0, 100.0, 100.0);
        assert_eq!(snap_box(min, max, FRAME, SnapOptions::default(), 6.0), Snapped::default());
        let (far_min, far_max) = boxed(120.0, 120.0, 10.0, 10.0);
        assert_eq!(snap_box(far_min, far_max, FRAME, EVERYTHING, 4.0), Snapped::default());
        assert_eq!(snap_box(min, max, FRAME, EVERYTHING, 0.0), Snapped::default());
    }

    #[test]
    fn dead_center_alone_does_not_pull_on_one_axis() {
        // Centered across but far from the middle down: dead center needs both.
        let options = SnapOptions { dead_center: true, ..Default::default() };
        let (min, max) = boxed(451.0, 100.0, 100.0, 100.0);
        assert_eq!(snap_box(min, max, FRAME, options, 6.0), Snapped::default());
    }
}
