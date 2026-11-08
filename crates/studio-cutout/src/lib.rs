//! Background removal: a few clicks or painted strokes on a picture ("keep this", "not this")
//! pick out a subject, with the MobileSAM segmentation model run locally through Candle (on the
//! Mac's GPU where there is one). Nothing leaves the computer.
//!
//! The model is one file, `MODEL_FILE`, published at huggingface.co/lmz/candle-sam (a Candle
//! conversion of MobileSAM, Apache-2.0). The app downloads it once; this crate only reads it.
//!
//! For video, the subject is followed from frame to frame: the model is given a few points deep
//! inside the previous frame's mask and a box around it, and of the model's guesses the one that
//! overlaps the previous mask most is kept, so the cut-out does not jump between objects. Masks can then have their edges refined (`Mask::with_edges`) and their flicker over
//! time calmed (`reduce_chatter`).

mod edges;
mod prompt;
mod sam;
mod strokes;

use std::path::Path;

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;

pub use edges::{reduce_chatter, EdgeSettings, Guide, SHIFT_EDGE_RANGE};
use prompt::{follow_prompt, low_res_overlap, stroke_prompt, Prompt};
use sam::{ModelPrompt, Sam, LOW_RES};
pub use strokes::Stroke;

/// The model's file, as named in its Hugging Face repository.
pub const MODEL_FILE: &str = "mobile_sam-tiny-vitt.safetensors";
/// Where it can be downloaded from.
pub const MODEL_URL: &str = "https://huggingface.co/lmz/candle-sam/resolve/main/mobile_sam-tiny-vitt.safetensors";
/// Its SHA-256, to check a download is complete and untampered.
pub const MODEL_SHA256: &str = "5ad43a841b57cc798e2837e251c15aa9f602868a1780d195459383d117784cf7";
/// Its size in bytes, for telling people what a download costs.
pub const MODEL_BYTES: u64 = 40_613_024;

/// The longest side pictures are segmented at (the model's own input size).
pub const WORK_SIZE: u32 = sam::IMAGE_SIZE as u32;

/// Something that went wrong, in words for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct CutoutError(pub String);

impl std::fmt::Display for CutoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<candle_core::Error> for CutoutError {
    fn from(error: candle_core::Error) -> Self {
        Self(format!("the cut-out model failed: {error}"))
    }
}

/// Whether the model file is in `folder`.
pub fn model_present(folder: &Path) -> bool {
    folder.join(MODEL_FILE).is_file()
}

/// A click on the picture: where (as fractions of its width and height) and whether it marks
/// something to keep (`true`) or something to remove (`false`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Click {
    pub x: f32,
    pub y: f32,
    pub keep: bool,
}

/// A cut-out: how much of each pixel is kept, 0 (removed) to 255 (kept), row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub alpha: Vec<u8>,
}

impl Mask {
    /// A mask of `width × height` that keeps nothing.
    fn empty(width: u32, height: u32) -> Mask {
        Mask { width, height, alpha: vec![0; (width * height) as usize] }
    }

    /// The kept amount of the pixel at column `x`, row `y`.
    pub(crate) fn at(&self, x: u32, y: u32) -> u8 {
        self.alpha[(y * self.width + x) as usize]
    }

    /// The share of the picture that is kept (0–1).
    pub fn coverage(&self) -> f32 {
        let kept = self.alpha.iter().filter(|&&a| a >= 128).count();
        kept as f32 / self.alpha.len().max(1) as f32
    }

    /// How much this mask and `other` (the same size) overlap: shared area over combined area.
    pub fn overlap(&self, other: &Mask) -> f32 {
        let (mut both, mut either) = (0usize, 0usize);
        for (a, b) in self.alpha.iter().zip(&other.alpha) {
            let (a, b) = (*a >= 128, *b >= 128);
            both += usize::from(a && b);
            either += usize::from(a || b);
        }
        if either == 0 { 0.0 } else { both as f32 / either as f32 }
    }

    /// Saves it as a grayscale PNG.
    pub fn save(&self, path: &Path) -> Result<(), CutoutError> {
        let image = image::GrayImage::from_raw(self.width, self.height, self.alpha.clone()).ok_or_else(|| CutoutError("the mask is the wrong size".into()))?;
        image.save(path).map_err(|error| CutoutError(format!("could not save the mask: {error}")))
    }
}

/// The MobileSAM model, loaded and ready.
pub struct Segmenter {
    model: Sam,
    device: Device,
}

/// A picture prepared for the model: its encoding (the slow part, done once per picture) and
/// its size, so several sets of clicks can be tried on it quickly.
pub struct Prepared {
    embedding: Tensor,
    width: u32,
    height: u32,
}

impl Segmenter {
    /// Loads the model from `folder`.
    pub fn load(folder: &Path) -> Result<Self, CutoutError> {
        if !model_present(folder) {
            return Err(CutoutError("the cut-out model has not been downloaded yet".into()));
        }
        let bytes = std::fs::read(folder.join(MODEL_FILE)).map_err(|error| CutoutError(format!("could not read the cut-out model: {error}")))?;
        let device = if cfg!(target_os = "macos") { Device::new_metal(0).unwrap_or(Device::Cpu) } else { Device::Cpu };
        let weights = VarBuilder::from_buffered_safetensors(bytes, DType::F32, &device)?;
        let model = Sam::new_tiny(weights)?;
        Ok(Self { model, device })
    }

    /// Prepares an RGBA picture (`width * 4` bytes a row) for segmenting. It is scaled down to
    /// `WORK_SIZE` on its longest side, the size the model sees.
    pub fn prepare(&self, rgba: &[u8], width: u32, height: u32) -> Result<Prepared, CutoutError> {
        let picture = image::RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or_else(|| CutoutError("the picture is the wrong size".into()))?;
        let scale = (WORK_SIZE as f32 / width.max(height) as f32).min(1.0);
        let (work_width, work_height) = (((width as f32 * scale).round() as u32).max(1), ((height as f32 * scale).round() as u32).max(1));
        let picture = if scale < 1.0 { image::imageops::resize(&picture, work_width, work_height, image::imageops::FilterType::Triangle) } else { picture };
        // The model takes channels first: all the reds, then the greens, then the blues.
        let pixels = (work_width * work_height) as usize;
        let mut planes = vec![0u8; pixels * 3];
        for (index, pixel) in picture.pixels().enumerate() {
            for channel in 0..3 {
                planes[channel * pixels + index] = pixel[channel];
            }
        }
        let tensor = Tensor::from_vec(planes, (3, work_height as usize, work_width as usize), &self.device)?;
        let embedding = self.model.embeddings(&tensor)?;
        Ok(Prepared { embedding, width: work_width, height: work_height })
    }

    /// The model's four masks for `prompt` on a prepared picture, as low-resolution logits, with
    /// its confidence in each (0–1). Mask 0 is its answer for a precise prompt; masks 1–3 its
    /// alternative readings of an ambiguous one.
    fn decode(&self, picture: &Prepared, prompt: &Prompt) -> Result<Guesses, CutoutError> {
        let (width, height) = (picture.width as f32, picture.height as f32);
        let points: Vec<(f32, f32, bool)> = prompt.points.iter().map(|click| (click.x * width, click.y * height, click.keep)).collect();
        let model_prompt = ModelPrompt { points: &points, bbox: prompt.bbox.map(|bounds| bounds.in_pixels(picture.width, picture.height)), mask: None };
        let (low_res, scores) = self.model.predict(&picture.embedding, &model_prompt)?;
        let logits = low_res.i(0)?.to_dtype(DType::F32)?.to_vec3::<f32>()?;
        let scores = scores.flatten_all()?.to_dtype(DType::F32)?.to_vec1::<f32>()?;
        Ok(Guesses { logits, scores })
    }

    /// Asks the model about `prompt` and picks its best guess, at the prepared picture's size.
    /// With a reference mask, the guess most like it wins (the model's confidence breaks near ties),
    /// so the result stays on the same subject. Otherwise, a lone point is ambiguous (a shirt,
    /// the person, the group) and the most confident alternative reading wins; for anything more
    /// precise the model's single answer is used, as its authors advise.
    fn best_guess(&self, picture: &Prepared, prompt: &Prompt) -> Result<Mask, CutoutError> {
        let guesses = self.decode(picture, prompt)?;
        let candidates: Vec<usize> = match &prompt.reference {
            Some(_) => (0..guesses.scores.len()).collect(),
            None if prompt.points.len() <= 1 && prompt.bbox.is_none() => (1..guesses.scores.len()).collect(),
            None => vec![0],
        };
        let rating = |index: usize| match &prompt.reference {
            Some(reference) => low_res_overlap(&guesses.logits[index], reference, picture.width, picture.height) + guesses.scores[index] * 0.1,
            None => guesses.scores[index],
        };
        let best = candidates.into_iter().filter(|&index| index < guesses.logits.len()).max_by(|&a, &b| rating(a).total_cmp(&rating(b))).ok_or_else(|| CutoutError("the model gave no answer".into()))?;
        Ok(upsample(&guesses.logits[best], picture.width, picture.height))
    }

    /// Picks out what `clicks` point to on a still picture, or the first frame of a video.
    pub fn segment(&self, picture: &Prepared, clicks: &[Click]) -> Result<Mask, CutoutError> {
        self.best_guess(picture, &Prompt { points: clicks.to_vec(), ..Default::default() })
    }

    /// Picks out what painted `strokes` mark, at the prepared picture's size, refining `previous`
    /// (this frame's mask so far, if any). The strokes become the model's prompt: points along
    /// them, plus, with a previous mask, points deep inside it and a box around it once the strokes
    /// are painted on. Then they are enforced: whatever an add stroke covers is kept and whatever a
    /// remove stroke covers is removed, whatever the model thought.
    pub fn segment_strokes(&self, picture: &Prepared, strokes: &[Stroke], previous: Option<&Mask>) -> Result<Mask, CutoutError> {
        let mut mask = match stroke_prompt(strokes, previous, picture.width, picture.height) {
            Some(prompt) => self.best_guess(picture, &prompt)?,
            None => Mask::empty(picture.width, picture.height),
        };
        strokes::paint_strokes(&mut mask, strokes);
        Ok(mask)
    }

    /// Picks out the same subject in the next frame of a video from the previous frame's mask:
    /// the model is given points deep inside it and a box around it (with room to move), and of
    /// its four guesses the one most like the previous mask wins. The box keeps the cut-out from
    /// spreading onto, or jumping to, similar things nearby.
    /// Keeps nothing once the previous mask keeps nothing (the subject left the picture).
    pub fn propagate(&self, picture: &Prepared, previous: &Mask) -> Result<Mask, CutoutError> {
        self.follow(picture, previous, &[])
    }

    /// `propagate`, also passing `removes`, the clicks marking things to remove, which are kept
    /// where they still fall outside the subject.
    pub fn follow(&self, picture: &Prepared, previous: &Mask, removes: &[Click]) -> Result<Mask, CutoutError> {
        match follow_prompt(previous, removes, picture.width, picture.height) {
            Some(prompt) => self.best_guess(picture, &prompt),
            None => Ok(Mask::empty(picture.width, picture.height)),
        }
    }
}

/// The model's masks for one prompt: low-resolution logits (`LOW_RES` square, covering the padded
/// input) and the model's confidence in each.
struct Guesses {
    logits: Vec<Vec<Vec<f32>>>,
    scores: Vec<f32>,
}

/// Scales the model's low-resolution mask (logits over the padded square input) up to the
/// picture's size, smoothly, and turns it into kept amounts. Interpolating the logits before
/// squashing them gives clean, slightly soft edges instead of blocky ones.
fn upsample(logits: &[Vec<f32>], width: u32, height: u32) -> Mask {
    let cell = sam::IMAGE_SIZE as f32 / LOW_RES as f32;
    let sample = |x: f32, y: f32| {
        let (x, y) = (x.clamp(0.0, (LOW_RES - 1) as f32), y.clamp(0.0, (LOW_RES - 1) as f32));
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(LOW_RES - 1), (y0 + 1).min(LOW_RES - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = logits[y0][x0] * (1.0 - fx) + logits[y0][x1] * fx;
        let bottom = logits[y1][x0] * (1.0 - fx) + logits[y1][x1] * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let mut alpha = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let logit = sample((x as f32 + 0.5) / cell - 0.5, (y as f32 + 0.5) / cell - 0.5);
            // A little steeper than a plain sigmoid, so edges are soft over about a pixel.
            let kept = 1.0 / (1.0 + (-logit * 2.0).exp());
            alpha.push((kept * 255.0).round() as u8);
        }
    }
    Mask { width, height, alpha }
}

/// Up to three "keep" clicks deep inside a mask, for finding the same subject in the next frame:
/// the point farthest from its edge overall, and the farthest in its top and bottom thirds
/// (so a standing person gets a point on their head and their legs, not only their middle).
/// Empty when the mask keeps nothing.
pub fn inside_points(mask: &Mask) -> Vec<Click> {
    // Work on a coarse grid: plenty to find the middle of a subject, and quick.
    const GRID: u32 = 96;
    let step = (mask.width.max(mask.height) as f32 / GRID as f32).max(1.0);
    let (columns, rows) = (((mask.width as f32 / step).ceil() as usize).max(1), ((mask.height as f32 / step).ceil() as usize).max(1));
    let inside = |column: usize, row: usize| {
        let (x, y) = (((column as f32 + 0.5) * step) as u32, ((row as f32 + 0.5) * step) as u32);
        mask.at(x.min(mask.width - 1), y.min(mask.height - 1)) >= 128
    };
    // Distance to the nearest outside cell (city-block), in two passes.
    let far = u32::MAX / 2;
    let mut distance = vec![0u32; columns * rows];
    for row in 0..rows {
        for column in 0..columns {
            if inside(column, row) {
                let up = if row > 0 { distance[(row - 1) * columns + column] } else { 0 };
                let left = if column > 0 { distance[row * columns + column - 1] } else { 0 };
                distance[row * columns + column] = (up.min(left) + 1).min(far);
            }
        }
    }
    for row in (0..rows).rev() {
        for column in (0..columns).rev() {
            let index = row * columns + column;
            if distance[index] > 0 {
                let down = if row + 1 < rows { distance[index + columns] } else { 0 };
                let right = if column + 1 < columns { distance[index + 1] } else { 0 };
                distance[index] = distance[index].min(down.min(right) + 1);
            }
        }
    }
    let inside_rows: Vec<usize> = (0..rows).filter(|&row| (0..columns).any(|column| distance[row * columns + column] > 0)).collect();
    let (Some(&top), Some(&bottom)) = (inside_rows.first(), inside_rows.last()) else { return Vec::new() };
    let deepest = |rows_from: usize, rows_to: usize| {
        (rows_from..=rows_to)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .max_by_key(|&(column, row)| distance[row * columns + column])
            .filter(|&(column, row)| distance[row * columns + column] > 0)
    };
    let third = (bottom - top) / 3;
    let mut found: Vec<(usize, usize)> = [deepest(top, bottom), deepest(top, top + third), deepest(bottom - third, bottom)].into_iter().flatten().collect();
    found.dedup();
    found
        .into_iter()
        .map(|(column, row)| Click { x: ((column as f32 + 0.5) * step / mask.width as f32).min(1.0), y: ((row as f32 + 0.5) * step / mask.height as f32).min(1.0), keep: true })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A mask keeping a filled rectangle.
    pub(crate) fn rectangle(width: u32, height: u32, keep: std::ops::Range<u32>, rows: std::ops::Range<u32>) -> Mask {
        let alpha = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| if keep.contains(&x) && rows.contains(&y) { 255 } else { 0 }).collect();
        Mask { width, height, alpha }
    }

    #[test]
    fn points_for_the_next_frame_sit_inside_the_subject() {
        // A tall subject in the left half: points on it, top, middle and bottom.
        let mask = rectangle(200, 400, 20..80, 40..360);
        let points = inside_points(&mask);
        assert!(!points.is_empty() && points.len() <= 3);
        for point in &points {
            let (x, y) = (point.x * 200.0, point.y * 400.0);
            assert!((20.0..80.0).contains(&x) && (40.0..360.0).contains(&y), "{point:?} is inside");
        }
        let ys: Vec<f32> = points.iter().map(|point| point.y).collect();
        assert!(ys.iter().cloned().fold(f32::MAX, f32::min) < 0.4 && ys.iter().cloned().fold(0.0, f32::max) > 0.6, "spread top to bottom: {ys:?}");
        assert!(inside_points(&rectangle(50, 50, 0..0, 0..0)).is_empty(), "nothing to follow in an empty mask");
    }

    #[test]
    fn refining_moves_a_rough_edge_onto_the_pictures_edge() {
        // The picture is bright left of x = 50; the rough mask stops at x = 44.
        let (width, height) = (100u32, 20u32);
        let picture: Vec<u8> = (0..width * height).flat_map(|i| if i % width < 50 { [230, 230, 230, 255] } else { [20, 20, 20, 255] }).collect();
        let rough = rectangle(width, height, 0..44, 0..height);
        let refined = rough.refine(&picture, width, height);
        assert!(refined.at(47, 10) > rough.at(47, 10) + 60, "kept up to the real edge: {}", refined.at(47, 10));
        assert!(refined.at(55, 10) < 40, "and not past it: {}", refined.at(55, 10));
        assert!(refined.at(10, 10) > 240, "the inside stays kept");
    }

    #[test]
    fn overlap_and_coverage_measure_masks() {
        let a = rectangle(10, 10, 0..5, 0..10);
        let b = rectangle(10, 10, 0..10, 0..10);
        assert_eq!(a.coverage(), 0.5);
        assert_eq!(a.overlap(&b), 0.5);
        assert_eq!(a.overlap(&a), 1.0);
    }

    #[test]
    fn upsampling_keeps_the_kept_area_and_softens_its_edge() {
        // Logits: positive (kept) in the left half of the square input.
        let logits: Vec<Vec<f32>> = (0..LOW_RES).map(|_| (0..LOW_RES).map(|x| if x < LOW_RES / 2 { 8.0 } else { -8.0 }).collect()).collect();
        let mask = upsample(&logits, 1024, 512);
        assert_eq!(mask.at(100, 100), 255);
        assert_eq!(mask.at(900, 100), 0);
        let edge = mask.at(511, 100);
        assert!(edge > 0 && edge < 255 || mask.at(512, 100) > 0 && mask.at(512, 100) < 255, "a soft edge where they meet");
    }
}
