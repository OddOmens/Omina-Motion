//! Edge refinement for masks, done on the CPU: following the picture's own edges (a guided
//! filter), moving the edge in or out, feathering it, sharpening it, and calming its flicker from
//! frame to frame.

use crate::Mask;

/// How far (in mask pixels) `Mask::refine` looks around each pixel, and how much it smooths:
/// larger smoothing keeps the mask solid over texture inside the subject.
pub(crate) const REFINE_RADIUS: usize = 6;
pub(crate) const REFINE_SMOOTHING: f32 = 0.002;
/// How far a whole step of `EdgeSettings::shift_edge` (±1) moves the edge, as a share of the
/// mask's longer side.
pub const SHIFT_EDGE_RANGE: f32 = 0.02;
/// How different (on the 0–255 scale) a neighbouring frame's value may be before
/// `reduce_chatter` treats the change as real movement and leaves the pixel alone.
const CHATTER_TOLERANCE: f32 = 160.0;
/// Stands in for "no such pixel anywhere" in distance calculations; large, but finite so the
/// arithmetic never produces NaN.
const FAR_AWAY: f32 = 1e20;

/// The average of `values` (a `width × height` grid) over the square of `radius` around each
/// cell, smaller at the borders. Uses a summed-area table, so the cost doesn't grow with radius.
pub(crate) fn box_mean(values: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    // Sums in f64: a whole frame of f32 additions would lose precision.
    let stride = width + 1;
    let mut sums = vec![0.0f64; stride * (height + 1)];
    for y in 0..height {
        let mut row = 0.0;
        for x in 0..width {
            row += f64::from(values[y * width + x]);
            sums[(y + 1) * stride + x + 1] = sums[y * stride + x + 1] + row;
        }
    }
    let mut means = Vec::with_capacity(width * height);
    for y in 0..height {
        let (top, bottom) = (y.saturating_sub(radius), (y + radius + 1).min(height));
        for x in 0..width {
            let (left, right) = (x.saturating_sub(radius), (x + radius + 1).min(width));
            let total = sums[bottom * stride + right] - sums[top * stride + right] - sums[bottom * stride + left] + sums[top * stride + left];
            means.push((total / ((bottom - top) * (right - left)) as f64) as f32);
        }
    }
    means
}

/// The picture a mask was made from, used to steer its edges. Either colour (RGBA, `width * 4`
/// bytes a row) or brightness alone (one byte a pixel).
#[derive(Clone, Copy, Debug)]
pub enum Guide<'a> {
    Rgba { pixels: &'a [u8], width: u32, height: u32 },
    Luma { pixels: &'a [u8], width: u32, height: u32 },
}

impl Guide<'_> {
    /// The picture's brightness (0–1) at `width × height`, scaled smoothly if its size differs.
    /// `None` when the pixel data doesn't match the stated size.
    fn brightness(&self, width: u32, height: u32) -> Option<Vec<f32>> {
        use image::imageops::{resize, FilterType};
        match *self {
            Guide::Rgba { pixels, width: own_width, height: own_height } => {
                let picture = image::RgbaImage::from_raw(own_width, own_height, pixels.to_vec())?;
                let picture = if (own_width, own_height) == (width, height) { picture } else { resize(&picture, width, height, FilterType::Triangle) };
                Some(picture.pixels().map(|p| (0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])) / 255.0).collect())
            }
            Guide::Luma { pixels, width: own_width, height: own_height } => {
                let picture = image::GrayImage::from_raw(own_width, own_height, pixels.to_vec())?;
                let picture = if (own_width, own_height) == (width, height) { picture } else { resize(&picture, width, height, FilterType::Triangle) };
                Some(picture.pixels().map(|p| f32::from(p[0]) / 255.0).collect())
            }
        }
    }
}

/// A guided filter (He, Sun and Tang, 2010) of `kept` (0–1 values) steered by `guide`
/// (brightness, 0–1), both `width × height`. Within each window of `radius` the mask is fitted
/// as a straight-line function of the brightness, so the mask changes where the picture changes
/// and stays smooth where it doesn't. Larger `smoothing` keeps it solid over fine texture.
fn guided_filter(kept: &[f32], guide: &[f32], width: usize, height: usize, radius: usize, smoothing: f32) -> Vec<f32> {
    let mean = |values: &[f32]| box_mean(values, width, height, radius);
    let product = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(a, b)| a * b).collect::<Vec<f32>>();
    let (mean_guide, mean_kept) = (mean(guide), mean(kept));
    let (mean_square, mean_cross) = (mean(&product(guide, guide)), mean(&product(guide, kept)));
    let mut slope = vec![0.0; width * height];
    let mut offset = vec![0.0; width * height];
    for i in 0..width * height {
        let variance = mean_square[i] - mean_guide[i] * mean_guide[i];
        let covariance = mean_cross[i] - mean_guide[i] * mean_kept[i];
        slope[i] = covariance / (variance + smoothing);
        offset[i] = mean_kept[i] - slope[i] * mean_guide[i];
    }
    let (slope, offset) = (mean(&slope), mean(&offset));
    (0..width * height).map(|i| (slope[i] * guide[i] + offset[i]).clamp(0.0, 1.0)).collect()
}

/// The After Effects style "Refine Edge" controls for a mask. All zero (the default) leaves the
/// mask as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EdgeSettings {
    /// How far the edge is blurred, in mask pixels (the edge fades over about twice this).
    pub feather: f32,
    /// How much the edge is sharpened around half-kept, 0 (not at all) to 1 (a hard cut).
    pub contrast: f32,
    /// Moves the edge in (negative, choke) or out (positive, spread), -1 to 1. A whole step moves
    /// it by `SHIFT_EDGE_RANGE` of the mask's longer side.
    pub shift_edge: f32,
    /// How far (in mask pixels) the edge looks for the picture's own edges to follow, such as
    /// hair against the sky; 0 turns this off.
    pub detail_radius: f32,
}

impl EdgeSettings {
    /// The settings with anything out of range or not a number brought back into range.
    fn sanitized(&self) -> EdgeSettings {
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        EdgeSettings {
            feather: finite(self.feather).max(0.0),
            contrast: finite(self.contrast).clamp(0.0, 1.0),
            shift_edge: finite(self.shift_edge).clamp(-1.0, 1.0),
            detail_radius: finite(self.detail_radius).max(0.0),
        }
    }
}

impl Mask {
    /// The mask's kept amounts as 0–1 values.
    fn kept_fractions(&self) -> Vec<f32> {
        self.alpha.iter().map(|&a| f32::from(a) / 255.0).collect()
    }

    /// A mask of the same size from 0–1 values.
    fn with_fractions(&self, kept: &[f32]) -> Mask {
        let alpha = kept.iter().map(|&k| (k.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        Mask { width: self.width, height: self.height, alpha }
    }

    /// Makes the mask's edges follow the edges in the picture it was made from, so hair,
    /// fingers and the outline of clothing are cut along what is really there rather than the
    /// model's rounded, low-resolution outline. `rgba` is that picture, `width * 4` bytes a
    /// row; it is scaled to the mask's size first if it differs.
    pub fn refine(&self, rgba: &[u8], width: u32, height: u32) -> Mask {
        self.refine_with(&Guide::Rgba { pixels: rgba, width, height }, REFINE_RADIUS, REFINE_SMOOTHING)
    }

    /// `refine` with its window `radius` (mask pixels) and `smoothing` chosen by the caller.
    /// Returns the mask unchanged if the guide's pixel data doesn't match its stated size.
    pub fn refine_with(&self, guide: &Guide, radius: usize, smoothing: f32) -> Mask {
        let (w, h) = (self.width as usize, self.height as usize);
        let Some(brightness) = guide.brightness(self.width, self.height) else { return self.clone() };
        self.with_fractions(&guided_filter(&self.kept_fractions(), &brightness, w, h, radius, smoothing.max(1e-6)))
    }

    /// The mask with After Effects style edge refinement applied, in this order: follow the
    /// picture's detail, shift the edge, feather it, then sharpen it.
    pub fn with_edges(&self, guide: &Guide, settings: &EdgeSettings) -> Mask {
        let settings = settings.sanitized();
        let (w, h) = (self.width as usize, self.height as usize);
        let mut kept = self.kept_fractions();
        let detail_radius = settings.detail_radius.round() as usize;
        if detail_radius > 0
            && let Some(brightness) = guide.brightness(self.width, self.height)
        {
            kept = guided_filter(&kept, &brightness, w, h, detail_radius, REFINE_SMOOTHING);
        }
        if settings.shift_edge != 0.0 {
            let pixels = settings.shift_edge * SHIFT_EDGE_RANGE * self.width.max(self.height) as f32;
            kept = shift_edge(&kept, w, h, pixels);
        }
        if settings.feather > 0.0 {
            kept = feather(&kept, w, h, settings.feather);
        }
        if settings.contrast > 0.0 {
            kept = steepen(&kept, settings.contrast);
        }
        self.with_fractions(&kept)
    }
}

/// Blurs `kept` with a close approximation of a Gaussian blur (three box blurs in a row), so the
/// edge fades over about `2 * feather` pixels.
fn feather(kept: &[f32], width: usize, height: usize, feather: f32) -> Vec<f32> {
    // Three box blurs of radius r add up to a variance of ((2r+1)² - 1) / 4; aim for a standard
    // deviation of half the feather.
    let sigma = feather / 2.0;
    let radius = ((((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0).round() as usize).max(1);
    let mut blurred = kept.to_vec();
    for _ in 0..3 {
        blurred = box_mean(&blurred, width, height, radius);
    }
    blurred
}

/// Sharpens values around half-kept: `contrast` 0 changes nothing, 1 cuts hard at 0.5.
fn steepen(kept: &[f32], contrast: f32) -> Vec<f32> {
    if contrast >= 1.0 {
        return kept.iter().map(|&k| if k >= 0.5 { 1.0 } else { 0.0 }).collect();
    }
    let gain = 1.0 / (1.0 - contrast);
    kept.iter().map(|&k| ((k - 0.5) * gain + 0.5).clamp(0.0, 1.0)).collect()
}

/// Moves the mask's edge outward by `pixels` (inward when negative), measuring true distances
/// from the edge, and leaves a one-pixel soft edge.
fn shift_edge(kept: &[f32], width: usize, height: usize, pixels: f32) -> Vec<f32> {
    let inside: Vec<bool> = kept.iter().map(|&k| k >= 0.5).collect();
    let outside: Vec<bool> = inside.iter().map(|&i| !i).collect();
    let to_outside = distance_to_nearest(&outside, width, height);
    let to_inside = distance_to_nearest(&inside, width, height);
    (0..width * height)
        .map(|i| {
            // Distance from the edge, which runs halfway between inside and outside pixels:
            // positive inside, negative outside.
            let from_edge = if inside[i] { to_outside[i] - 0.5 } else { 0.5 - to_inside[i] };
            (from_edge + pixels + 0.5).clamp(0.0, 1.0)
        })
        .collect()
}

/// The straight-line distance from each cell of a `width × height` grid to the nearest cell where
/// `targets` is true (0 on those cells; `FAR_AWAY` or more when there are none). Exact, in two
/// passes of the method of Felzenszwalb and Huttenlocher (2012).
fn distance_to_nearest(targets: &[bool], width: usize, height: usize) -> Vec<f32> {
    let mut squared: Vec<f32> = targets.iter().map(|&t| if t { 0.0 } else { FAR_AWAY }).collect();
    // Down each column, then along each row.
    let mut column = vec![0.0; height];
    for x in 0..width {
        for y in 0..height {
            column[y] = squared[y * width + x];
        }
        let distances = squared_distances_1d(&column);
        for y in 0..height {
            squared[y * width + x] = distances[y];
        }
    }
    for y in 0..height {
        let row = squared_distances_1d(&squared[y * width..(y + 1) * width]);
        squared[y * width..(y + 1) * width].copy_from_slice(&row);
    }
    squared.iter().map(|d| d.sqrt()).collect()
}

/// For a row of costs `f`, the smallest `f[q] + (p - q)²` for every `p`: the lower envelope of
/// parabolas rooted at each cell.
fn squared_distances_1d(f: &[f32]) -> Vec<f32> {
    let n = f.len();
    if n == 0 {
        return Vec::new();
    }
    // `roots` are the cells whose parabolas form the envelope; `starts[k]` is where parabola k
    // starts to be the lowest.
    let mut roots = vec![0usize; n];
    let mut starts = vec![0.0f32; n + 1];
    let mut count = 0usize;
    starts[0] = f32::NEG_INFINITY;
    starts[1] = f32::INFINITY;
    let crossing = |q: usize, r: usize| ((f[q] + (q * q) as f32) - (f[r] + (r * r) as f32)) / (2.0 * q as f32 - 2.0 * r as f32);
    for q in 1..n {
        let mut s = crossing(q, roots[count]);
        while s <= starts[count] {
            count -= 1;
            s = crossing(q, roots[count]);
        }
        count += 1;
        roots[count] = q;
        starts[count] = s;
        starts[count + 1] = f32::INFINITY;
    }
    let mut distances = vec![0.0; n];
    let mut k = 0;
    for (p, distance) in distances.iter_mut().enumerate() {
        while starts[k + 1] < p as f32 {
            k += 1;
        }
        let offset = p as f32 - roots[k] as f32;
        *distance = offset * offset + f[roots[k]];
    }
    distances
}

/// Calms the flicker ("chatter") of a mask's soft edge over time by blending each pixel of
/// `current` toward its value in the neighbouring frames' masks. `amount` runs from 0 (no change)
/// to 1 (a full average). Pixels whose value changes a lot between frames are moving, not
/// chattering, so they are left alone; neighbours of a different size are ignored.
pub fn reduce_chatter(previous: Option<&Mask>, current: &Mask, next: Option<&Mask>, amount: f32) -> Mask {
    let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.0 };
    let same_size = |mask: &&Mask| mask.width == current.width && mask.height == current.height && mask.alpha.len() == current.alpha.len();
    let neighbours: Vec<&Mask> = [previous, next].into_iter().flatten().filter(same_size).collect();
    if amount == 0.0 || neighbours.is_empty() {
        return current.clone();
    }
    let alpha = current
        .alpha
        .iter()
        .enumerate()
        .map(|(i, &value)| {
            let value = f32::from(value);
            // Each neighbour counts fully when it is close to this frame, less as it differs more,
            // and not at all past the tolerance.
            let (mut total, mut weight) = (value, 1.0);
            for neighbour in &neighbours {
                let other = f32::from(neighbour.alpha[i]);
                let likeness = (1.0 - (other - value).abs() / CHATTER_TOLERANCE).max(0.0);
                total += other * likeness;
                weight += likeness;
            }
            let average = total / weight;
            (value + (average - value) * amount).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    Mask { width: current.width, height: current.height, alpha }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rectangle;

    /// A grayscale guide, bright left of `edge`.
    fn split_picture(width: u32, height: u32, edge: u32) -> Vec<u8> {
        (0..width * height).map(|i| if i % width < edge { 230 } else { 20 }).collect()
    }

    #[test]
    fn box_means_average_their_neighbourhood() {
        let values = [0.0, 0.0, 3.0, 3.0];
        assert_eq!(box_mean(&values, 4, 1, 1), vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn distances_are_straight_line_distances() {
        // One target in the middle of a 5 × 5 grid.
        let mut targets = vec![false; 25];
        targets[12] = true;
        let distances = distance_to_nearest(&targets, 5, 5);
        assert_eq!(distances[12], 0.0);
        assert_eq!(distances[13], 1.0);
        assert!((distances[0] - 8f32.sqrt()).abs() < 1e-5, "corner: {}", distances[0]);
        assert!(distance_to_nearest(&[false; 4], 2, 2).iter().all(|&d| d > 1e9), "no target is far away");
    }

    #[test]
    fn default_settings_change_nothing() {
        let mask = rectangle(40, 20, 10..30, 5..15);
        let guide = split_picture(40, 20, 20);
        assert_eq!(mask.with_edges(&Guide::Luma { pixels: &guide, width: 40, height: 20 }, &EdgeSettings::default()), mask);
    }

    #[test]
    fn detail_follows_the_pictures_edge_like_refine() {
        let (width, height) = (100u32, 20u32);
        let luma = split_picture(width, height, 50);
        let rgba: Vec<u8> = luma.iter().flat_map(|&v| [v, v, v, 255]).collect();
        let rough = rectangle(width, height, 0..44, 0..height);
        let settings = EdgeSettings { detail_radius: REFINE_RADIUS as f32, ..Default::default() };
        let detailed = rough.with_edges(&Guide::Rgba { pixels: &rgba, width, height }, &settings);
        assert_eq!(detailed, rough.refine(&rgba, width, height), "the same filter as refine");
        assert!(detailed.at(47, 10) > 60 && detailed.at(55, 10) < 40, "kept up to the real edge and not past it");
        // A larger window reaches further toward the picture's edge.
        let wide = rough.refine_with(&Guide::Rgba { pixels: &rgba, width, height }, 12, REFINE_SMOOTHING);
        assert!(wide.at(49, 10) >= detailed.at(49, 10));
    }

    #[test]
    fn feather_softens_the_edge_over_about_twice_its_size() {
        let mask = rectangle(100, 10, 0..50, 0..10);
        let guide = [0u8; 1000];
        let feathered = mask.with_edges(&Guide::Luma { pixels: &guide, width: 100, height: 10 }, &EdgeSettings { feather: 8.0, ..Default::default() });
        let row: Vec<u8> = (0..100).map(|x| feathered.at(x, 5)).collect();
        assert!(row[49] > 100 && row[49] < 200 && row[50] > 60 && row[50] < 160, "half kept at the edge: {:?}", &row[44..56]);
        assert!(row[44] > row[49] && row[49] > row[54], "falls off across the edge");
        assert_eq!(row[20], 255, "far inside is untouched");
        assert_eq!(row[80], 0, "far outside is untouched");
    }

    #[test]
    fn contrast_sharpens_around_half() {
        let soft = Mask { width: 3, height: 1, alpha: vec![64, 128, 192] };
        let guide = [0u8; 3];
        let guide = Guide::Luma { pixels: &guide, width: 3, height: 1 };
        let some = soft.with_edges(&guide, &EdgeSettings { contrast: 0.5, ..Default::default() });
        assert!(some.alpha[0] < 64 && some.alpha[2] > 192, "{:?}", some.alpha);
        assert!((127..=129).contains(&some.alpha[1]), "half stays half");
        let hard = soft.with_edges(&guide, &EdgeSettings { contrast: 1.0, ..Default::default() });
        assert_eq!(hard.alpha, vec![0, 255, 255]);
    }

    #[test]
    fn shift_edge_chokes_and_spreads() {
        // 100 wide: a whole step moves the edge 2 pixels.
        let mask = rectangle(100, 20, 30..70, 0..20);
        let guide = [0u8; 2000];
        let guide = Guide::Luma { pixels: &guide, width: 100, height: 20 };
        let spread = mask.with_edges(&guide, &EdgeSettings { shift_edge: 1.0, ..Default::default() });
        assert_eq!((spread.at(28, 10), spread.at(71, 10), spread.at(26, 10)), (255, 255, 0), "two pixels wider on each side");
        let choked = mask.with_edges(&guide, &EdgeSettings { shift_edge: -1.0, ..Default::default() });
        assert_eq!((choked.at(31, 10), choked.at(32, 10), choked.at(68, 10)), (0, 255, 0), "two pixels narrower");
        let half = mask.with_edges(&guide, &EdgeSettings { shift_edge: 0.25, ..Default::default() });
        assert!(half.at(29, 10) > 100 && half.at(29, 10) < 155, "a half-pixel shift is a soft edge: {}", half.at(29, 10));
    }

    #[test]
    fn nonsense_settings_are_ignored() {
        let mask = rectangle(10, 10, 2..8, 2..8);
        let guide = [0u8; 100];
        let settings = EdgeSettings { feather: f32::NAN, contrast: -3.0, shift_edge: f32::INFINITY, detail_radius: -1.0 };
        assert_eq!(mask.with_edges(&Guide::Luma { pixels: &guide, width: 10, height: 10 }, &settings), mask);
        // A guide of the wrong size skips only the detail step.
        let refined = mask.with_edges(&Guide::Luma { pixels: &guide[..5], width: 10, height: 10 }, &EdgeSettings { detail_radius: 3.0, ..Default::default() });
        assert_eq!(refined, mask);
    }

    #[test]
    fn chatter_is_smoothed_but_movement_is_kept() {
        let frame = |values: [u8; 2]| Mask { width: 2, height: 1, alpha: values.to_vec() };
        // Pixel 0 flickers slightly; pixel 1 changes from removed to kept (something moved).
        let (before, now, after) = (frame([100, 0]), frame([140, 255]), frame([100, 255]));
        let calmed = reduce_chatter(Some(&before), &now, Some(&after), 1.0);
        assert!(calmed.alpha[0] < 125 && calmed.alpha[0] > 100, "flicker pulled toward its neighbours: {}", calmed.alpha[0]);
        assert_eq!(calmed.alpha[1], 255, "a real change is left alone");
        assert_eq!(reduce_chatter(Some(&before), &now, Some(&after), 0.0), now, "no amount, no change");
        assert_eq!(reduce_chatter(None, &now, None, 1.0), now, "nothing to compare with");
        let half = reduce_chatter(Some(&before), &now, None, 0.5);
        assert!(half.alpha[0] > calmed.alpha[0] && half.alpha[0] < 140);
        let other_size = Mask { width: 1, height: 1, alpha: vec![0] };
        assert_eq!(reduce_chatter(Some(&other_size), &now, None, 1.0), now, "a different size is ignored");
    }
}
