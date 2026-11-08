//! Reads the image embedding and the prompt tokens and predicts masks, with the model's estimate
//! of how good each one is.
//!
//! Adapted from candle-transformers 0.9.1, `src/models/segment_anything/mask_decoder.rs`
//! (Copyright the Candle authors, MIT OR Apache-2.0). Changed here: `forward` returns all four
//! masks (the single-answer one first, then the three alternative readings) so callers can choose,
//! the always-off sigmoid option and tracing are removed, and plain `candle_nn::Linear` layers are
//! used (same weights).

use candle_core::{IndexOp, Module, Result, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::segment_anything::transformer::TwoWayTransformer;
use candle_transformers::models::segment_anything::LayerNorm2d;

/// A small stack of fully connected layers with ReLU between them.
#[derive(Debug)]
struct MlpMaskDecoder {
    layers: Vec<candle_nn::Linear>,
}

impl MlpMaskDecoder {
    fn new(input_dim: usize, hidden_dim: usize, output_dim: usize, num_layers: usize, vb: VarBuilder) -> Result<Self> {
        let vb = vb.pp("layers");
        let layers = (0..num_layers)
            .map(|i| {
                let in_dim = if i == 0 { input_dim } else { hidden_dim };
                let out_dim = if i + 1 == num_layers { output_dim } else { hidden_dim };
                candle_nn::linear(in_dim, out_dim, vb.pp(i))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { layers })
    }
}

impl Module for MlpMaskDecoder {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let mut xs = xs.clone();
        for (i, layer) in self.layers.iter().enumerate() {
            xs = layer.forward(&xs)?;
            if i + 1 < self.layers.len() {
                xs = xs.relu()?
            }
        }
        Ok(xs)
    }
}

/// The mask decoder: a two-way transformer between prompt tokens and the image, then an
/// upscaler that turns each mask token into a 256 × 256 mask of logits.
#[derive(Debug)]
pub(crate) struct MaskDecoder {
    iou_token: candle_nn::Embedding,
    mask_tokens: candle_nn::Embedding,
    iou_prediction_head: MlpMaskDecoder,
    output_upscaling_conv1: candle_nn::ConvTranspose2d,
    output_upscaling_ln: LayerNorm2d,
    output_upscaling_conv2: candle_nn::ConvTranspose2d,
    num_mask_tokens: usize,
    output_hypernetworks_mlps: Vec<MlpMaskDecoder>,
    transformer: TwoWayTransformer,
}

impl MaskDecoder {
    pub(crate) fn new(transformer_dim: usize, num_multimask_outputs: usize, iou_head_depth: usize, iou_head_hidden_dim: usize, vb: VarBuilder) -> Result<Self> {
        let num_mask_tokens = num_multimask_outputs + 1;
        let iou_prediction_head = MlpMaskDecoder::new(transformer_dim, iou_head_hidden_dim, num_mask_tokens, iou_head_depth, vb.pp("iou_prediction_head"))?;
        let iou_token = candle_nn::embedding(1, transformer_dim, vb.pp("iou_token"))?;
        let mask_tokens = candle_nn::embedding(num_mask_tokens, transformer_dim, vb.pp("mask_tokens"))?;
        let cfg = candle_nn::ConvTranspose2dConfig { stride: 2, ..Default::default() };
        let output_upscaling_conv1 = candle_nn::conv_transpose2d(transformer_dim, transformer_dim / 4, 2, cfg, vb.pp("output_upscaling.0"))?;
        let output_upscaling_ln = LayerNorm2d::new(transformer_dim / 4, 1e-6, vb.pp("output_upscaling.1"))?;
        let output_upscaling_conv2 = candle_nn::conv_transpose2d(transformer_dim / 4, transformer_dim / 8, 2, cfg, vb.pp("output_upscaling.3"))?;
        let vb_o = vb.pp("output_hypernetworks_mlps");
        let output_hypernetworks_mlps = (0..num_mask_tokens).map(|i| MlpMaskDecoder::new(transformer_dim, transformer_dim, transformer_dim / 8, 3, vb_o.pp(i))).collect::<Result<Vec<_>>>()?;
        let transformer = TwoWayTransformer::new(/* depth */ 2, /* embedding_dim */ transformer_dim, /* num_heads */ 8, /* mlp_dim */ 2048, vb.pp("transformer"))?;
        Ok(Self {
            iou_token,
            mask_tokens,
            iou_prediction_head,
            output_upscaling_conv1,
            output_upscaling_ln,
            output_upscaling_conv2,
            num_mask_tokens,
            output_hypernetworks_mlps,
            transformer,
        })
    }

    /// All the masks, `(batch, 4, 256, 256)` logits, and their predicted quality, `(batch, 4)`.
    /// Mask 0 is the model's single best answer for a precise prompt; masks 1–3 are the
    /// alternative readings it offers for an ambiguous one (a part, the whole, a group).
    pub(crate) fn forward(&self, image_embeddings: &Tensor, image_pe: &Tensor, sparse_prompt_embeddings: &Tensor, dense_prompt_embeddings: &Tensor) -> Result<(Tensor, Tensor)> {
        // The output tokens go first, then the prompt's own tokens.
        let output_tokens = Tensor::cat(&[self.iou_token.embeddings(), self.mask_tokens.embeddings()], 0)?;
        let (d1, d2) = output_tokens.dims2()?;
        let output_tokens = output_tokens.unsqueeze(0)?.expand((sparse_prompt_embeddings.dim(0)?, d1, d2))?;
        let tokens = Tensor::cat(&[&output_tokens, sparse_prompt_embeddings], 1)?;

        // The image, with the dense prompt added, once per prompt in the batch.
        let src = repeat_interleave(image_embeddings, tokens.dim(0)?, 0)?;
        let src = src.broadcast_add(dense_prompt_embeddings)?;
        let pos_src = repeat_interleave(image_pe, tokens.dim(0)?, 0)?;
        let (b, c, h, w) = src.dims4()?;

        let (hs, src) = self.transformer.forward(&src, &pos_src, &tokens)?;
        let iou_token_out = hs.i((.., 0))?;
        let mask_tokens_out = hs.i((.., 1..1 + self.num_mask_tokens))?;

        // Upscale the image features, then each mask token becomes a mask through its own
        // small network.
        let src = src.transpose(1, 2)?.reshape((b, c, h, w))?;
        let upscaled_embedding = self.output_upscaling_conv1.forward(&src)?.apply(&self.output_upscaling_ln)?.gelu()?.apply(&self.output_upscaling_conv2)?.gelu()?;
        let hyper_in_list = self.output_hypernetworks_mlps.iter().enumerate().map(|(i, mlp)| mlp.forward(&mask_tokens_out.i((.., i))?)).collect::<Result<Vec<_>>>()?;
        let hyper_in = Tensor::stack(hyper_in_list.as_slice(), 1)?.contiguous()?;
        let (b, c, h, w) = upscaled_embedding.dims4()?;
        let masks = hyper_in.matmul(&upscaled_embedding.reshape((b, c, h * w))?)?;
        let masks = masks.reshape((b, (), h, w))?;

        let iou_pred = self.iou_prediction_head.forward(&iou_token_out)?;
        Ok((masks, iou_pred))
    }
}

/// Repeats each slice of `img` along `dim` `repeats` times in a row, like PyTorch's
/// `repeat_interleave`.
fn repeat_interleave(img: &Tensor, repeats: usize, dim: usize) -> Result<Tensor> {
    let img = img.unsqueeze(dim + 1)?;
    let mut dims = img.dims().to_vec();
    dims[dim + 1] = repeats;
    img.broadcast_as(dims)?.flatten(dim, dim + 1)
}
