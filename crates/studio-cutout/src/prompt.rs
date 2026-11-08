//! Building the model's prompts (points and a box) from what the person did: painted strokes, or
//! the previous frame's mask when following a subject through a video. The mask the answer should
//! resemble is kept alongside, at the model's low resolution, to choose among the model's guesses.
//!
//! The previous mask is deliberately not given to the model as its mask prompt. MobileSAM was
//! trained to refine its own earlier answer with that input, and given any other mask it mostly
//! copies it: a moving subject stays where it was, and a mask grown by a stroke doesn't grow.

use crate::sam::{IMAGE_SIZE, LOW_RES};
use crate::strokes::{paint_strokes, Stroke};
use crate::{inside_points, Click, Mask};

/// How strongly a low-resolution mask says "object" or "not object", in the model's logit units.
/// Masks the model itself makes reach roughly this far from zero inside and outside the object.
const MASK_PROMPT_LOGIT: f32 = 10.0;
/// Points along strokes are spaced this far apart, as a share of the picture's longer side.
const STROKE_POINT_SPACING: f32 = 0.05;
/// At most this many points from add strokes, and as many from remove strokes: the model does best
/// with a few well-spread points rather than many.
const MAX_STROKE_POINTS: usize = 6;
/// How much the box around a mask refined by strokes is grown on each side, as a share of its own
/// size, so the object's edge isn't cut by it.
const STROKE_BOX_PADDING: f32 = 0.1;
/// How much the previous frame's box is grown on each side when following a subject, as a share of
/// its size, plus a minimum share of the picture: room for the subject to move between frames.
const FOLLOW_BOX_PADDING: f32 = 0.25;
const FOLLOW_BOX_MIN_PADDING: f32 = 0.03;

/// A box as fractions (0–1) of the picture's width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Bounds {
    /// The box grown on each side by `share` of its own width or height, but at least `minimum`
    /// of the picture's, and kept within the picture.
    fn padded(self, share: f32, minimum: f32) -> Bounds {
        let across = ((self.right - self.left) * share).max(minimum);
        let down = ((self.bottom - self.top) * share).max(minimum);
        Bounds { left: (self.left - across).max(0.0), top: (self.top - down).max(0.0), right: (self.right + across).min(1.0), bottom: (self.bottom + down).min(1.0) }
    }

    /// The box in pixels of a `width × height` picture: left, top, right, bottom.
    pub(crate) fn in_pixels(&self, width: u32, height: u32) -> [f32; 4] {
        let (width, height) = (width as f32, height as f32);
        [self.left * width, self.top * height, self.right * width, self.bottom * height]
    }
}

/// The box around a mask's kept pixels (those at least half kept), or `None` if it keeps nothing.
pub(crate) fn mask_bounds(mask: &Mask) -> Option<Bounds> {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..mask.height {
        for x in 0..mask.width {
            if mask.at(x, y) >= 128 {
                (left, top, right, bottom) = (left.min(x), top.min(y), right.max(x), bottom.max(y));
            }
        }
    }
    (left <= right).then(|| Bounds {
        left: left as f32 / mask.width as f32,
        top: top as f32 / mask.height as f32,
        right: (right + 1) as f32 / mask.width as f32,
        bottom: (bottom + 1) as f32 / mask.height as f32,
    })
}

/// A mask turned into the model's low-resolution form for a picture prepared at `width × height`: a
/// `LOW_RES × LOW_RES` grid of logits over the padded square input, each cell the average of the
/// mask over the picture pixels it covers (padding counts as "not object"). The mask may be a
/// different size from the picture; it is mapped by position. This is the form of the model's
/// mask prompt, and of its own answers, so they can be compared cell by cell.
pub(crate) fn mask_prompt(mask: &Mask, width: u32, height: u32) -> Vec<f32> {
    let cell = IMAGE_SIZE / LOW_RES;
    let mut logits = vec![-MASK_PROMPT_LOGIT; LOW_RES * LOW_RES];
    if mask.width == 0 || mask.height == 0 {
        return logits;
    }
    let (columns, rows) = ((width as usize).div_ceil(cell).min(LOW_RES), (height as usize).div_ceil(cell).min(LOW_RES));
    // Which mask column or row each picture pixel falls in.
    let mask_column = |x: usize| ((x as f32 + 0.5) / width as f32 * mask.width as f32).min((mask.width - 1) as f32) as u32;
    let mask_row = |y: usize| ((y as f32 + 0.5) / height as f32 * mask.height as f32).min((mask.height - 1) as f32) as u32;
    for row in 0..rows {
        for column in 0..columns {
            let (mut total, mut count) = (0u32, 0u32);
            for y in (row * cell..(row + 1) * cell).filter(|&y| y < height as usize) {
                for x in (column * cell..(column + 1) * cell).filter(|&x| x < width as usize) {
                    total += u32::from(mask.at(mask_column(x), mask_row(y)));
                    count += 1;
                }
            }
            let kept = total as f32 / (count.max(1) as f32 * 255.0);
            logits[row * LOW_RES + column] = (kept * 2.0 - 1.0) * MASK_PROMPT_LOGIT;
        }
    }
    logits
}

/// How much one of the model's low-resolution masks overlaps a reference mask, over the cells that
/// cover the picture (`width × height`): shared area over combined area, both taken as "object"
/// where the logit is above zero.
pub(crate) fn low_res_overlap(logits: &[Vec<f32>], reference: &[f32], width: u32, height: u32) -> f32 {
    let cell = IMAGE_SIZE / LOW_RES;
    let (columns, rows) = ((width as usize).div_ceil(cell).min(LOW_RES), (height as usize).div_ceil(cell).min(LOW_RES));
    let (mut both, mut either) = (0usize, 0usize);
    for (row, line) in logits.iter().enumerate().take(rows) {
        for (column, &logit) in line.iter().enumerate().take(columns) {
            let (a, b) = (logit > 0.0, reference[row * LOW_RES + column] > 0.0);
            both += usize::from(a && b);
            either += usize::from(a || b);
        }
    }
    if either == 0 { 0.0 } else { both as f32 / either as f32 }
}

/// What the model is asked (points and a box, in picture fractions), and the mask its answer
/// should resemble, if any, in the form `mask_prompt` makes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Prompt {
    pub points: Vec<Click>,
    pub bbox: Option<Bounds>,
    pub reference: Option<Vec<f32>>,
}

/// Up to `cap` items spread evenly through `items`, always including the first and last.
fn spread_evenly<T: Copy>(items: &[T], cap: usize) -> Vec<T> {
    match cap {
        _ if items.len() <= cap => items.to_vec(),
        0 => Vec::new(),
        1 => vec![items[items.len() / 2]],
        _ => (0..cap).map(|i| items[(i * (items.len() - 1) + (cap - 1) / 2) / (cap - 1)]).collect(),
    }
}

/// The prompt for following a subject into a new frame prepared at `width × height`: points deep
/// inside the previous mask and a box around it with room to move, with the previous mask as the
/// reference. `removes` are added as "not this" points where they fall outside the previous mask.
/// `None` when the previous mask keeps nothing.
pub(crate) fn follow_prompt(previous: &Mask, removes: &[Click], width: u32, height: u32) -> Option<Prompt> {
    let mut points = inside_points(previous);
    if points.is_empty() {
        return None;
    }
    let outside = |click: &&Click| {
        let x = ((click.x * previous.width as f32) as u32).min(previous.width - 1);
        let y = ((click.y * previous.height as f32) as u32).min(previous.height - 1);
        previous.at(x, y) < 128
    };
    points.extend(removes.iter().filter(outside).copied());
    let bbox = mask_bounds(previous).map(|bounds| bounds.padded(FOLLOW_BOX_PADDING, FOLLOW_BOX_MIN_PADDING));
    Some(Prompt { points, bbox, reference: Some(mask_prompt(previous, width, height)) })
}

/// The prompt for painted `strokes` on a picture prepared at `width × height`, refining `previous`
/// if there is one:
/// - points spread along add strokes ("this") and remove strokes ("not this");
/// - with a previous mask, the strokes are painted onto it first, and the result gives a few more
///   "this" points deep inside it, a box (grown a little) and the reference.
///
/// Strokes alone get no box: a stroke painted down the middle of an object says nothing about how
/// wide it is, and a box around it would squeeze the answer to a sliver.
///
/// `None` when nothing is marked to keep.
pub(crate) fn stroke_prompt(strokes: &[Stroke], previous: Option<&Mask>, width: u32, height: u32) -> Option<Prompt> {
    let spacing = STROKE_POINT_SPACING * width.max(height) as f32;
    let samples = |add: bool| -> Vec<(f32, f32)> { strokes.iter().filter(|stroke| stroke.add == add).flat_map(|stroke| stroke.sample(width, height, spacing)).collect() };
    let click = |keep: bool| move |(x, y): (f32, f32)| Click { x, y, keep };
    let mut points: Vec<Click> = spread_evenly(&samples(true), MAX_STROKE_POINTS).into_iter().map(click(true)).collect();

    let painted = previous.map(|previous| {
        let mut painted = previous.clone();
        paint_strokes(&mut painted, strokes);
        painted
    });
    if let Some(painted) = &painted {
        points.extend(inside_points(painted));
    }
    if points.is_empty() {
        return None;
    }
    points.extend(spread_evenly(&samples(false), MAX_STROKE_POINTS).into_iter().map(click(false)));

    let bbox = painted.as_ref().and_then(mask_bounds).map(|bounds| bounds.padded(STROKE_BOX_PADDING, 0.0));
    let reference = painted.map(|painted| mask_prompt(&painted, width, height));
    Some(Prompt { points, bbox, reference })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rectangle;

    #[test]
    fn mask_prompts_cover_the_padded_square() {
        // A 1024 × 512 picture whose left half is kept, given as a half-size mask.
        let mask = rectangle(512, 256, 0..256, 0..256);
        let logits = mask_prompt(&mask, 1024, 512);
        assert_eq!(logits.len(), LOW_RES * LOW_RES);
        let at = |column: usize, row: usize| logits[row * LOW_RES + column];
        assert_eq!(at(10, 10), MASK_PROMPT_LOGIT, "kept");
        assert_eq!(at(200, 10), -MASK_PROMPT_LOGIT, "removed");
        assert_eq!(at(10, 200), -MASK_PROMPT_LOGIT, "below the picture is padding");
        assert_eq!(at(127, 10), MASK_PROMPT_LOGIT, "the last kept column");
        assert_eq!(at(128, 10), -MASK_PROMPT_LOGIT, "the first removed one");
    }

    #[test]
    fn mask_prompts_average_partly_kept_cells() {
        // 6 × 4 picture, so one cell covers it with a partial edge; half its pixels kept.
        let mask = rectangle(6, 4, 0..2, 0..4);
        let logits = mask_prompt(&mask, 6, 4);
        assert!(logits[0].abs() < 1e-5, "half kept is undecided: {}", logits[0]);
        assert_eq!(logits[1], -MASK_PROMPT_LOGIT, "the next cell covers columns 4–5, removed");
    }

    #[test]
    fn low_res_overlap_compares_within_the_picture() {
        let reference = mask_prompt(&rectangle(400, 400, 0..200, 0..400), 400, 400);
        let same: Vec<Vec<f32>> = (0..LOW_RES).map(|row| (0..LOW_RES).map(|column| reference[row * LOW_RES + column]).collect()).collect();
        assert_eq!(low_res_overlap(&same, &reference, 400, 400), 1.0);
        // Beyond the picture (column 100 and on) nothing counts, even if marked kept.
        let spilled: Vec<Vec<f32>> = same.iter().map(|line| line.iter().enumerate().map(|(column, &v)| if column >= 100 { 5.0 } else { v }).collect()).collect();
        assert_eq!(low_res_overlap(&spilled, &reference, 400, 400), 1.0);
        let wider: Vec<Vec<f32>> = (0..LOW_RES).map(|_| (0..LOW_RES).map(|column| if column < 100 { 5.0 } else { -5.0 }).collect()).collect();
        assert!((low_res_overlap(&wider, &reference, 400, 400) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn mask_bounds_are_fractions_of_the_picture() {
        let bounds = mask_bounds(&rectangle(100, 50, 10..30, 5..25)).unwrap();
        assert_eq!(bounds, Bounds { left: 0.1, top: 0.1, right: 0.3, bottom: 0.5 });
        assert_eq!(mask_bounds(&rectangle(10, 10, 0..0, 0..0)), None);
        let padded = bounds.padded(0.5, 0.0);
        assert!((padded.left - 0.0).abs() < 1e-6 && (padded.right - 0.4).abs() < 1e-6 && (padded.bottom - 0.7).abs() < 1e-6);
        let point = Bounds { left: 0.5, top: 0.5, right: 0.5, bottom: 0.5 }.padded(0.25, 0.1);
        assert!((point.left - 0.4).abs() < 1e-6 && (point.top - 0.4).abs() < 1e-6 && (point.right - 0.6).abs() < 1e-6 && (point.bottom - 0.6).abs() < 1e-6, "at least the minimum: {point:?}");
    }

    #[test]
    fn spreading_keeps_the_ends_and_caps_the_count() {
        let items: Vec<usize> = (0..10).collect();
        assert_eq!(spread_evenly(&items, 3), vec![0, 5, 9]);
        assert_eq!(spread_evenly(&items, 20), items);
        assert_eq!(spread_evenly(&items, 1), vec![5]);
        assert!(spread_evenly(&items, 0).is_empty());
    }

    #[test]
    fn a_long_stroke_gives_spread_points_but_no_box() {
        // A stroke down the middle of a 1000 × 500 picture, covering 80% of its height.
        let stroke = Stroke { points: vec![(0.5, 0.1), (0.5, 0.9)], radius: 0.01, add: true };
        let prompt = stroke_prompt(&[stroke], None, 1000, 500).unwrap();
        assert_eq!(prompt.points.len(), MAX_STROKE_POINTS, "capped: {:?}", prompt.points);
        assert!(prompt.points.iter().all(|point| point.keep && (point.x - 0.5).abs() < 1e-5));
        assert!((prompt.points[0].y - 0.1).abs() < 1e-5 && (prompt.points[MAX_STROKE_POINTS - 1].y - 0.9).abs() < 1e-5, "spread end to end");
        assert_eq!(prompt.bbox, None, "a stroke doesn't show how wide the object is");
        assert_eq!(prompt.reference, None, "no previous mask, nothing to resemble");
    }

    #[test]
    fn remove_strokes_give_negative_points() {
        let dab = Stroke { points: vec![(0.3, 0.3)], radius: 0.02, add: true };
        let remove = Stroke { points: vec![(0.7, 0.3), (0.7, 0.5)], radius: 0.02, add: false };
        let prompt = stroke_prompt(&[dab, remove.clone()], None, 1000, 1000).unwrap();
        assert_eq!(prompt.points.iter().filter(|point| point.keep).count(), 1);
        let negatives: Vec<&Click> = prompt.points.iter().filter(|point| !point.keep).collect();
        assert_eq!(negatives.len(), 5, "every 0.05 along 0.2: {negatives:?}");
        assert!(negatives.iter().all(|point| (point.x - 0.7).abs() < 1e-5));
        assert_eq!(stroke_prompt(&[remove], None, 1000, 1000), None, "nothing marked to keep");
    }

    #[test]
    fn strokes_on_a_previous_mask_shape_the_reference_and_box() {
        // The previous mask keeps a square at the left; an add stroke extends it to the right and
        // a remove stroke cuts its top.
        let previous = rectangle(400, 400, 40..160, 100..300);
        let add = Stroke { points: vec![(0.4, 0.5), (0.7, 0.5)], radius: 0.03, add: true };
        let remove = Stroke { points: vec![(0.1, 0.28), (0.4, 0.28)], radius: 0.03, add: false };
        let prompt = stroke_prompt(&[add, remove], Some(&previous), 400, 400).unwrap();
        let mask = prompt.reference.as_ref().expect("the painted previous mask is the reference");
        let at = |x: f32, y: f32| mask[(y * 100.0) as usize * LOW_RES + (x * 100.0) as usize];
        assert!(at(0.6, 0.5) > 0.0, "the add stroke is in it");
        assert!(at(0.25, 0.28) < 0.0, "the remove stroke is cut out");
        assert!(at(0.25, 0.6) > 0.0, "the rest of the previous mask stays");
        let bbox = prompt.bbox.unwrap();
        assert!(bbox.left < 0.1 && bbox.right > 0.73 && bbox.bottom > 0.75 && bbox.top > 0.2, "{bbox:?}");
        // Points deep inside the painted mask join the stroke points, none under the cut.
        assert!(prompt.points.iter().filter(|point| point.keep).count() > MAX_STROKE_POINTS);
        assert!(prompt.points.iter().filter(|point| point.keep).all(|point| !(point.y > 0.25 && point.y < 0.31 && point.x < 0.4)));
    }

    #[test]
    fn following_uses_the_previous_mask_for_points_box_and_reference() {
        let previous = rectangle(200, 100, 50..100, 20..80);
        let removes = [Click { x: 0.9, y: 0.5, keep: false }, Click { x: 0.375, y: 0.5, keep: false }];
        let prompt = follow_prompt(&previous, &removes, 200, 100).unwrap();
        assert!(prompt.points.iter().filter(|point| point.keep).count() >= 1);
        assert_eq!(prompt.points.iter().filter(|point| !point.keep).count(), 1, "only the remove click outside the subject is kept");
        let bbox = prompt.bbox.unwrap();
        // The subject spans 0.25–0.5 across and 0.2–0.8 down; grown by a quarter on each side.
        assert!((bbox.left - 0.1875).abs() < 1e-5 && (bbox.right - 0.5625).abs() < 1e-5 && (bbox.top - 0.05).abs() < 1e-5 && (bbox.bottom - 0.95).abs() < 1e-5, "{bbox:?}");
        assert_eq!(prompt.reference.unwrap(), mask_prompt(&previous, 200, 100));
        assert_eq!(follow_prompt(&rectangle(10, 10, 0..0, 0..0), &[], 10, 10), None);
    }
}
