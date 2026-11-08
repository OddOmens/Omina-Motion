//! The Segment Anything model (SAM) in its small MobileSAM form: an image encoder that runs once
//! per picture, and a quick prompt encoder and mask decoder that turn points, a box and a rough
//! mask into a mask.
//!
//! Adapted from candle-transformers 0.9.1, `src/models/segment_anything/sam.rs` (Copyright the
//! Candle authors, MIT OR Apache-2.0; https://github.com/huggingface/candle). Candle keeps the
//! prompt encoder and mask decoder private, so only point prompts were reachable; this copy keeps
//! just the TinyViT (MobileSAM) image encoder and accepts a box and a mask as well. The image
//! encoder, the two-way transformer and the layer norm are still Candle's own, and the weights
//! file is read under the same names, so the same `mobile_sam-tiny-vitt.safetensors` loads.

mod mask_decoder;
mod prompt_encoder;

use candle_core::{DType, Module, Result, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::segment_anything::tiny_vit::{tiny_vit_5m, TinyViT};

use mask_decoder::MaskDecoder;
use prompt_encoder::PromptEncoder;

/// The side of the square picture the model reads; smaller pictures are padded to it.
pub(crate) const IMAGE_SIZE: usize = 1024;
/// The side of the model's low-resolution masks, which cover the whole padded square.
pub(crate) const LOW_RES: usize = 256;
const PROMPT_EMBED_DIM: usize = 256;
const VIT_PATCH_SIZE: usize = 16;

/// What the model is asked to find. Coordinates are in input pixels (the prepared picture's own
/// pixels, which sit in the top-left corner of the padded square).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ModelPrompt<'a> {
    /// Points, each with whether it is on the object (`true`) or not.
    pub points: &'a [(f32, f32, bool)],
    /// A box around the object: left, top, right, bottom.
    pub bbox: Option<[f32; 4]>,
    /// A rough mask of the object, `LOW_RES × LOW_RES` logits row by row (positive = object).
    pub mask: Option<&'a [f32]>,
}

/// MobileSAM, loaded.
#[derive(Debug)]
pub(crate) struct Sam {
    image_encoder: TinyViT,
    prompt_encoder: PromptEncoder,
    mask_decoder: MaskDecoder,
    pixel_mean: Tensor,
    pixel_std: Tensor,
}

impl Sam {
    /// Builds the model from MobileSAM's weights.
    pub(crate) fn new_tiny(vb: VarBuilder) -> Result<Self> {
        let image_embedding_size = IMAGE_SIZE / VIT_PATCH_SIZE;
        let image_encoder = tiny_vit_5m(vb.pp("image_encoder"))?;
        let prompt_encoder = PromptEncoder::new(PROMPT_EMBED_DIM, (image_embedding_size, image_embedding_size), (IMAGE_SIZE, IMAGE_SIZE), 16, vb.pp("prompt_encoder"))?;
        let mask_decoder = MaskDecoder::new(PROMPT_EMBED_DIM, /* num_multimask_outputs */ 3, /* iou_head_depth */ 3, /* iou_head_hidden_dim */ 256, vb.pp("mask_decoder"))?;
        let pixel_mean = Tensor::new(&[123.675f32, 116.28, 103.53], vb.device())?.reshape((3, 1, 1))?;
        let pixel_std = Tensor::new(&[58.395f32, 57.12, 57.375], vb.device())?.reshape((3, 1, 1))?;
        Ok(Self { image_encoder, prompt_encoder, mask_decoder, pixel_mean, pixel_std })
    }

    /// Encodes a `(3, height, width)` picture of 0–255 values, at most `IMAGE_SIZE` a side. This is
    /// the slow step; its result can be prompted many times.
    pub(crate) fn embeddings(&self, img: &Tensor) -> Result<Tensor> {
        let img = self.preprocess(img)?.unsqueeze(0)?;
        self.image_encoder.forward(&img)
    }

    /// Normalizes the picture's colors the way the model was trained, and pads it to the square.
    fn preprocess(&self, img: &Tensor) -> Result<Tensor> {
        let (_c, h, w) = img.dims3()?;
        let img = img.to_dtype(DType::F32)?.broadcast_sub(&self.pixel_mean)?.broadcast_div(&self.pixel_std)?;
        if h > IMAGE_SIZE || w > IMAGE_SIZE {
            candle_core::bail!("image is too large ({w}, {h}), maximum size {IMAGE_SIZE}")
        }
        let img = img.pad_with_zeros(1, 0, IMAGE_SIZE - h)?;
        img.pad_with_zeros(2, 0, IMAGE_SIZE - w)
    }

    /// All four of the model's masks for `prompt` on an encoded picture: `(1, 4, 256, 256)`
    /// logits, and the model's predicted quality of each, `(1, 4)`. Mask 0 is its single answer
    /// for a precise prompt, masks 1–3 its alternative readings of an ambiguous one.
    pub(crate) fn predict(&self, img_embeddings: &Tensor, prompt: &ModelPrompt) -> Result<(Tensor, Tensor)> {
        let device = img_embeddings.device();
        let image_pe = self.prompt_encoder.get_dense_pe()?;
        let points = if prompt.points.is_empty() {
            None
        } else {
            let n_points = prompt.points.len();
            let xys = prompt.points.iter().flat_map(|&(x, y, _)| [x, y]).collect::<Vec<_>>();
            let labels = prompt.points.iter().map(|&(_, _, on)| if on { 1f32 } else { 0f32 }).collect::<Vec<_>>();
            Some((Tensor::from_vec(xys, (1, n_points, 2), device)?, Tensor::from_vec(labels, (1, n_points), device)?))
        };
        let bbox = prompt.bbox.map(|corners| Tensor::from_vec(corners.to_vec(), (1, 4), device)).transpose()?;
        let mask = prompt.mask.map(|logits| Tensor::from_vec(logits.to_vec(), (1, 1, LOW_RES, LOW_RES), device)).transpose()?;
        let (sparse, dense) = self.prompt_encoder.forward(points.as_ref().map(|(xy, labels)| (xy, labels)), bbox.as_ref(), mask.as_ref())?;
        self.mask_decoder.forward(img_embeddings, &image_pe, &sparse, &dense)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, IndexOp};

    /// Checks this copy gives the same masks as Candle's own model for point prompts, and that box
    /// and mask prompts run. Needs the downloaded model:
    /// `CUTOUT_MODEL=<model folder> cargo test -p studio-cutout -- --ignored same_masks`
    #[test]
    #[ignore = "needs the downloaded model; set CUTOUT_MODEL"]
    fn same_masks_as_candles_model_for_points() {
        use candle_transformers::models::segment_anything::sam::Sam as CandleSam;
        let folder = std::env::var("CUTOUT_MODEL").expect("CUTOUT_MODEL names the model folder");
        let bytes = std::fs::read(std::path::Path::new(&folder).join(crate::MODEL_FILE)).unwrap();
        let device = Device::Cpu;
        let ours = Sam::new_tiny(VarBuilder::from_buffered_safetensors(bytes.clone(), DType::F32, &device).unwrap()).unwrap();
        let theirs = CandleSam::new_tiny(VarBuilder::from_buffered_safetensors(bytes, DType::F32, &device).unwrap()).unwrap();

        // A bright disc on a dark background, 640 × 360.
        let (width, height) = (640usize, 360usize);
        let mut planes = vec![40u8; 3 * width * height];
        for y in 0..height {
            for x in 0..width {
                if (x as f32 - 200.0).hypot(y as f32 - 180.0) < 90.0 {
                    planes[y * width + x] = 250;
                    planes[width * height + y * width + x] = 150;
                }
            }
        }
        let picture = Tensor::from_vec(planes, (3, height, width), &device).unwrap();
        let embedding = ours.embeddings(&picture).unwrap();
        let difference = (&embedding - theirs.embeddings(&picture).unwrap()).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
        assert!(difference < 1e-4, "same image encoding: {difference}");

        for points in [vec![(200.0 / 640.0, 0.5, true)], vec![(200.0 / 640.0, 0.5, true), (0.9, 0.1, false)]] {
            let pixels: Vec<(f32, f32, bool)> = points.iter().map(|&(x, y, on)| (x as f32 * width as f32, y as f32 * height as f32, on)).collect();
            let (masks, scores) = ours.predict(&embedding, &ModelPrompt { points: &pixels, ..Default::default() }).unwrap();
            for several in [false, true] {
                let (their_masks, their_scores) = theirs.forward_for_embeddings(&embedding, height, width, &points, several).unwrap();
                let (ours_masks, ours_scores) = if several { (masks.i((.., 1..)).unwrap(), scores.i((.., 1..)).unwrap()) } else { (masks.i((.., 0..1)).unwrap(), scores.i((.., 0..1)).unwrap()) };
                let mask_difference = (ours_masks - their_masks).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
                let score_difference = (ours_scores - their_scores).unwrap().abs().unwrap().max_all().unwrap().to_scalar::<f32>().unwrap();
                assert!(mask_difference < 1e-3 && score_difference < 1e-4, "{points:?}, several {several}: masks differ by {mask_difference}, scores by {score_difference}");
            }
        }

        // A box with a point, and a mask prompt, both run and keep the disc.
        let logits: Vec<f32> = (0..LOW_RES * LOW_RES).map(|i| if ((i % LOW_RES) as f32 - 50.0).hypot((i / LOW_RES) as f32 - 45.0) < 22.0 { 10.0 } else { -10.0 }).collect();
        let point = [(200.0, 180.0, true)];
        let prompt = ModelPrompt { points: &point, bbox: Some([100.0, 80.0, 300.0, 280.0]), mask: Some(&logits) };
        let (masks, _) = ours.predict(&embedding, &prompt).unwrap();
        let middle = masks.i((0, 0, 45, 50)).unwrap().to_scalar::<f32>().unwrap();
        assert!(middle > 0.0, "the disc's middle is kept: {middle}");
    }
}
