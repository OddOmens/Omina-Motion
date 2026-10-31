//! Snapping: while dragging something in time, pulling it onto a nearby frame of interest
//! (the playhead, a layer's edge, a keyframe) when it comes within a small distance.

/// Where a drag snapped: how far to nudge it, and the frame it lined up with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snap {
    pub nudge: i64,
    pub at: i64,
}

/// The smallest nudge that puts one of `moving` (frames where the dragged things would be)
/// exactly onto one of `targets`, if one is within `threshold` frames. `targets` must be
/// sorted. A threshold of 0 never snaps.
pub fn snap(moving: &[i64], targets: &[i64], threshold: i64) -> Option<Snap> {
    if threshold <= 0 {
        return None;
    }
    let mut best: Option<Snap> = None;
    for &frame in moving {
        // Only the targets on either side of `frame` can be the closest.
        let index = targets.partition_point(|&target| target < frame);
        for &target in targets[index.saturating_sub(1)..(index + 1).min(targets.len())].iter() {
            let nudge = target - frame;
            if nudge.abs() <= threshold && best.is_none_or(|best| nudge.abs() < best.nudge.abs()) {
                best = Some(Snap { nudge, at: target });
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_to_the_closest_target_within_reach() {
        let targets = [0, 24, 48, 100];
        assert_eq!(snap(&[46], &targets, 3), Some(Snap { nudge: 2, at: 48 }));
        assert_eq!(snap(&[27], &targets, 3), Some(Snap { nudge: -3, at: 24 }));
        assert_eq!(snap(&[36], &targets, 3), None, "nothing within reach");
    }

    #[test]
    fn whichever_moving_edge_is_closest_wins() {
        // A layer from 10 to 50 nearing targets 13 (3 away from its start) and 51 (1 away from its end).
        assert_eq!(snap(&[10, 50], &[13, 51], 5), Some(Snap { nudge: 1, at: 51 }));
    }

    #[test]
    fn no_threshold_or_no_targets_never_snaps() {
        assert_eq!(snap(&[10], &[10], 0), None);
        assert_eq!(snap(&[10], &[], 5), None);
    }
}
