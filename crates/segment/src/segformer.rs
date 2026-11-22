//! SegFormer (Xie et al., NVIDIA 2021) semantic segmentation: the "parsers" behind Select Sky,
//! Landscape classes and People parts. One architecture, several public checkpoints
//! ([`crate::parsers`]): ADE20K scene classes (sky, water, trees, mountains, buildings, ground…),
//! human parsing (hair, face, arms, legs, clothes) and face parsing (skin, brows, eyes, lips,
//! mouth, hair).
//!
//! Modified work: ported by the LightCraft contributors in 2026 from candle-transformers'
//! `models/segformer.rs` (Copyright the Candle authors, MIT OR Apache-2.0), itself a port of
//! Hugging Face Transformers' `models/segformer/modeling_segformer.py` (Apache-2.0); changed to
//! match Transformers where candle's differs (bilinear upsampling in the decode head, the
//! classifier's bias, PyTorch's LayerNorm / BatchNorm eps), the depthwise convolution done as nine
//! shifted products (candle runs grouped convolutions one group at a time), and attention scores in
//! bounded blocks. See NOTICE.

use std::path::Path;
use std::sync::Arc;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::{Conv2d, Conv2dConfig, LayerNorm, Linear, VarBuilder};

use crate::nn::{TORCH_LN_EPS, sdpa};
use crate::weights::{Backend, Weights};
use crate::{Error, Result};

/// The parts of a checkpoint's `config.json` the model needs.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Config {
    pub hidden_sizes: Vec<usize>,
    pub depths: Vec<usize>,
    pub num_attention_heads: Vec<usize>,
    pub sr_ratios: Vec<usize>,
    pub patch_sizes: Vec<usize>,
    pub strides: Vec<usize>,
    pub mlp_ratios: Vec<usize>,
    pub decoder_hidden_size: usize,
    #[serde(default)]
    pub id2label: std::collections::BTreeMap<String, String>,
}

impl Config {
    /// Parse and sanity-check a config (hostile files: four stages, bounded sizes).
    pub fn parse(json: &str) -> Result<Config> {
        let c: Config = serde_json::from_str(json).map_err(|e| Error::Model(format!("config.json: {e}")))?;
        let n = 4;
        let lists = [&c.hidden_sizes, &c.depths, &c.num_attention_heads, &c.sr_ratios, &c.patch_sizes, &c.strides, &c.mlp_ratios];
        if lists.iter().any(|l| l.len() != n) {
            return Err(Error::Model("config.json: SegFormer has four stages".into()));
        }
        let ok = |l: &[usize], lo: usize, hi: usize| l.iter().all(|v| (lo..=hi).contains(v));
        if !ok(&c.hidden_sizes, 8, 2048)
            || !ok(&c.depths, 1, 64)
            || !ok(&c.num_attention_heads, 1, 64)
            || !ok(&c.sr_ratios, 1, 16)
            || !ok(&c.patch_sizes, 1, 15)
            || !ok(&c.strides, 1, 8)
            || !ok(&c.mlp_ratios, 1, 8)
            || !(8..=4096).contains(&c.decoder_hidden_size)
            || c.hidden_sizes.iter().zip(&c.num_attention_heads).any(|(h, a)| h % a != 0)
        {
            return Err(Error::Model("config.json: SegFormer sizes out of range".into()));
        }
        if c.id2label.is_empty() || c.id2label.len() > 1024 {
            return Err(Error::Model("config.json: no class labels".into()));
        }
        Ok(c)
    }

    /// Class names by index (`id2label`); a missing index is named by its number.
    pub fn labels(&self) -> Vec<String> {
        let n = self.id2label.keys().filter_map(|k| k.parse::<usize>().ok()).max().map_or(0, |m| m + 1).min(1024);
        (0..n).map(|i| self.id2label.get(&i.to_string()).map(|s| s.trim().to_string()).unwrap_or_else(|| i.to_string())).collect()
    }
}

fn conv(i: usize, o: usize, k: usize, stride: usize, padding: usize, vb: VarBuilder) -> Result<Conv2d> {
    Ok(candle_nn::conv2d(i, o, k, Conv2dConfig { stride, padding, ..Default::default() }, vb)?)
}

fn ln(d: usize, vb: VarBuilder) -> Result<LayerNorm> {
    crate::nn::ln(d, TORCH_LN_EPS, vb)
}

fn lin(i: usize, o: usize, vb: VarBuilder) -> Result<Linear> {
    crate::nn::linear(i, o, vb)
}

/// `[B, C, H, W]` → `[B, H·W, C]`.
fn tokens(x: &Tensor) -> Result<Tensor> {
    Ok(x.flatten_from(2)?.transpose(1, 2)?)
}

/// `[B, H·W, C]` → `[B, C, H, W]`.
fn grid(x: &Tensor, h: usize, w: usize) -> Result<Tensor> {
    let (b, _, c) = x.dims3()?;
    Ok(x.transpose(1, 2)?.contiguous()?.reshape((b, c, h, w))?)
}

struct PatchEmbed {
    proj: Conv2d,
    norm: LayerNorm,
}

impl PatchEmbed {
    fn forward(&self, x: &Tensor) -> Result<(Tensor, usize, usize)> {
        let x = self.proj.forward(x)?;
        let (_, _, h, w) = x.dims4()?;
        Ok((self.norm.forward(&tokens(&x)?)?, h, w))
    }
}

struct SelfAttention {
    heads: usize,
    q: Linear,
    k: Linear,
    v: Linear,
    sr: Option<(Conv2d, LayerNorm)>,
    out: Linear,
}

impl SelfAttention {
    fn new(hidden: usize, heads: usize, sr: usize, vb: VarBuilder) -> Result<Self> {
        let s = vb.pp("self");
        let sr = if sr > 1 { Some((conv(hidden, hidden, sr, sr, 0, s.pp("sr"))?, ln(hidden, s.pp("layer_norm"))?)) } else { None };
        Ok(Self {
            heads,
            q: lin(hidden, hidden, s.pp("query"))?,
            k: lin(hidden, hidden, s.pp("key"))?,
            v: lin(hidden, hidden, s.pp("value"))?,
            sr,
            out: lin(hidden, hidden, vb.pp("output").pp("dense"))?,
        })
    }

    /// `x`: `[B, N, C]` tokens of an `h × w` grid.
    fn forward(&self, x: &Tensor, h: usize, w: usize) -> Result<Tensor> {
        let (b, n, c) = x.dims3()?;
        let d = c / self.heads;
        let split = |t: Tensor, len: usize| -> Result<Tensor> { Ok(t.reshape((b, len, self.heads, d))?.transpose(1, 2)?.contiguous()?) };
        let q = split(self.q.forward(x)?, n)?;
        let kv = match &self.sr {
            Some((conv, norm)) => norm.forward(&tokens(&conv.forward(&grid(x, h, w)?)?)?)?,
            None => x.clone(),
        };
        let m = kv.dim(1)?;
        let k = split(self.k.forward(&kv)?, m)?;
        let v = split(self.v.forward(&kv)?, m)?;
        // the first stage's long sequence: score blocks, not one huge matrix
        let o = query_blocks(&q, &k, &v, 1.0 / (d as f64).sqrt())?;
        let o = o.transpose(1, 2)?.contiguous()?.reshape((b, n, c))?;
        Ok(self.out.forward(&o)?)
    }
}

/// Largest attention-score block (elements) computed at once.
const SCORE_BLOCK: usize = 48 << 20;

/// Attention over query blocks small enough that no score matrix exceeds [`SCORE_BLOCK`].
fn query_blocks(q: &Tensor, k: &Tensor, v: &Tensor, scale: f64) -> Result<Tensor> {
    let (b, h, n, _) = q.dims4()?;
    let m = k.dim(2)?;
    let per_query = b.saturating_mul(h).saturating_mul(m).max(1);
    let rows = (SCORE_BLOCK / per_query).clamp(1, n.max(1));
    if rows >= n {
        return sdpa(q, k, v, None, scale);
    }
    let mut outs = Vec::with_capacity(n.div_ceil(rows));
    let mut s = 0;
    while s < n {
        let len = rows.min(n - s);
        outs.push(sdpa(&q.narrow(2, s, len)?, k, v, None, scale)?);
        s += len;
    }
    Ok(Tensor::cat(&outs, 2)?)
}

/// A 3 × 3 depthwise convolution (padding 1) as nine shifted products.
struct DepthwiseConv {
    /// `[C, 9]`, and the bias `[C]`.
    weight: Tensor,
    bias: Tensor,
}

impl DepthwiseConv {
    fn new(c: usize, vb: VarBuilder) -> Result<Self> {
        let weight = vb.get((c, 1, 3, 3), "weight")?.reshape((c, 9))?;
        let bias = vb.get(c, "bias")?;
        Ok(Self { weight, bias })
    }

    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let (_, c, h, w) = x.dims4()?;
        let p = x.pad_with_zeros(2, 1, 1)?.pad_with_zeros(3, 1, 1)?;
        let mut acc = self.bias.reshape((1, c, 1, 1))?.broadcast_as(x.shape())?.contiguous()?;
        for ky in 0..3 {
            for kx in 0..3 {
                let k = self.weight.narrow(1, ky * 3 + kx, 1)?.reshape((1, c, 1, 1))?;
                let shifted = p.narrow(2, ky, h)?.narrow(3, kx, w)?;
                acc = (acc + shifted.broadcast_mul(&k)?)?;
            }
        }
        Ok(acc)
    }
}

struct MixFfn {
    dense1: Linear,
    dw: DepthwiseConv,
    dense2: Linear,
}

impl MixFfn {
    fn forward(&self, x: &Tensor, h: usize, w: usize) -> Result<Tensor> {
        let y = self.dense1.forward(x)?;
        let y = tokens(&self.dw.forward(&grid(&y, h, w)?)?)?;
        Ok(self.dense2.forward(&y.gelu_erf()?)?)
    }
}

struct Layer {
    norm1: LayerNorm,
    attn: SelfAttention,
    norm2: LayerNorm,
    mlp: MixFfn,
}

impl Layer {
    fn forward(&self, x: &Tensor, h: usize, w: usize) -> Result<Tensor> {
        let x = (x + self.attn.forward(&self.norm1.forward(x)?, h, w)?)?;
        Ok((&x + self.mlp.forward(&self.norm2.forward(&x)?, h, w)?)?)
    }
}

struct Stage {
    embed: PatchEmbed,
    layers: Vec<Layer>,
    norm: LayerNorm,
}

struct DecodeHead {
    linear_c: Vec<Linear>,
    fuse: Tensor,
    bn_scale: Tensor,
    bn_shift: Tensor,
    classifier: Conv2d,
}

/// A loaded SegFormer checkpoint.
pub struct SegFormer {
    stages: Vec<Stage>,
    head: DecodeHead,
    labels: Vec<String>,
    device: Device,
}

/// Side multiple the input is padded to (the four stages downsample 32×).
const ALIGN: usize = 32;

impl SegFormer {
    /// Load `model.safetensors` + `config.json` from `dir`.
    pub fn load(dir: &Path, device: Device) -> Result<Self> {
        let weights_path = dir.join(crate::WEIGHTS_FILE);
        let config_path = dir.join("config.json");
        if !weights_path.is_file() || !config_path.is_file() {
            return Err(Error::Missing(dir.to_path_buf()));
        }
        let text = std::fs::metadata(&config_path)
            .ok()
            .filter(|m| m.len() < 1 << 20)
            .and_then(|_| std::fs::read_to_string(&config_path).ok())
            .ok_or_else(|| Error::Model(format!("{}: unreadable", config_path.display())))?;
        let cfg = Config::parse(&text)?;
        let weights = Weights::open(&weights_path)?;
        if !weights.has_prefix("segformer.encoder.") || !weights.has_prefix("decode_head.") {
            return Err(Error::Model(format!("{}: not a SegFormer segmentation checkpoint", weights_path.display())));
        }
        let labels = cfg.labels();
        let vb = VarBuilder::from_backend(Box::new(Backend(Arc::clone(&weights))), DType::F32, device.clone());
        let enc = vb.pp("segformer").pp("encoder");
        let mut stages = Vec::with_capacity(4);
        for i in 0..4 {
            let hidden = cfg.hidden_sizes[i];
            let cin = if i == 0 { 3 } else { cfg.hidden_sizes[i - 1] };
            let pe = enc.pp(format!("patch_embeddings.{i}"));
            let embed = PatchEmbed {
                proj: conv(cin, hidden, cfg.patch_sizes[i], cfg.strides[i], cfg.patch_sizes[i] / 2, pe.pp("proj"))?,
                norm: ln(hidden, pe.pp("layer_norm"))?,
            };
            let mut layers = Vec::with_capacity(cfg.depths[i]);
            for j in 0..cfg.depths[i] {
                let l = enc.pp(format!("block.{i}.{j}"));
                let inner = hidden * cfg.mlp_ratios[i];
                layers.push(Layer {
                    norm1: ln(hidden, l.pp("layer_norm_1"))?,
                    attn: SelfAttention::new(hidden, cfg.num_attention_heads[i], cfg.sr_ratios[i], l.pp("attention"))?,
                    norm2: ln(hidden, l.pp("layer_norm_2"))?,
                    mlp: MixFfn {
                        dense1: lin(hidden, inner, l.pp("mlp").pp("dense1"))?,
                        dw: DepthwiseConv::new(inner, l.pp("mlp").pp("dwconv").pp("dwconv"))?,
                        dense2: lin(inner, hidden, l.pp("mlp").pp("dense2"))?,
                    },
                });
            }
            stages.push(Stage { embed, layers, norm: ln(hidden, enc.pp(format!("layer_norm.{i}")))? });
        }
        let hd = vb.pp("decode_head");
        let dh = cfg.decoder_hidden_size;
        let linear_c = (0..4).map(|i| lin(cfg.hidden_sizes[i], dh, hd.pp(format!("linear_c.{i}")).pp("proj"))).collect::<Result<Vec<_>>>()?;
        let fuse = hd.pp("linear_fuse").get((dh, 4 * dh, 1, 1), "weight")?.reshape((dh, 4 * dh))?;
        // eval-mode BatchNorm folded into a scale and a shift
        let bn = hd.pp("batch_norm");
        let (g, b, mean, var) = (bn.get(dh, "weight")?, bn.get(dh, "bias")?, bn.get(dh, "running_mean")?, bn.get(dh, "running_var")?);
        let bn_scale = g.div(&(var + 1e-5)?.sqrt()?)?;
        let bn_shift = (b - mean.mul(&bn_scale)?)?;
        let classifier = candle_nn::conv2d(dh, labels.len(), 1, Conv2dConfig::default(), hd.pp("classifier"))?;
        Ok(Self { stages, head: DecodeHead { linear_c, fuse, bn_scale, bn_shift, classifier }, labels, device })
    }

    pub fn labels(&self) -> &[String] {
        &self.labels
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Class logits `[C, H/4, W/4]` for preprocessed pixels `[1, 3, H, W]` (H, W multiples of 32).
    pub fn forward(&self, pixels: &Tensor) -> Result<Tensor> {
        let mut x = pixels.to_device(&self.device)?;
        let mut feats = Vec::with_capacity(4);
        for s in &self.stages {
            let (mut t, h, w) = s.embed.forward(&x)?;
            for l in &s.layers {
                t = l.forward(&t, h, w)?;
            }
            x = grid(&s.norm.forward(&t)?, h, w)?;
            feats.push(x.clone());
        }
        let (_, _, h0, w0) = feats.first().ok_or_else(|| Error::Model("no features".into()))?.dims4()?;
        let mut ups = Vec::with_capacity(4);
        for (f, l) in feats.iter().zip(&self.head.linear_c) {
            let (_, _, h, w) = f.dims4()?;
            let y = grid(&l.forward(&tokens(f)?)?, h, w)?;
            ups.push(if (h, w) == (h0, w0) { y } else { y.upsample_bilinear2d(h0, w0, false)? });
        }
        ups.reverse();
        let cat = Tensor::cat(&ups, 1)?;
        let dh = self.head.fuse.dim(0)?;
        // 1 × 1 conv without bias = a matmul over channels
        let y = self.head.fuse.matmul(&cat.flatten_from(2)?.squeeze(0)?)?.reshape((1, dh, h0, w0))?;
        let y = y.broadcast_mul(&self.head.bn_scale.reshape((1, dh, 1, 1))?)?.broadcast_add(&self.head.bn_shift.reshape((1, dh, 1, 1))?)?.relu()?;
        Ok(self.head.classifier.forward(&y)?.squeeze(0)?)
    }

    /// Per-class probabilities for an 8-bit RGB image, resized (aspect kept) so its long edge is
    /// `long_edge` (padded to a multiple of 32 with the mean colour).
    pub fn parse(&self, rgb: &[u8], w: usize, h: usize, long_edge: usize) -> Result<Parsed> {
        let (iw, ih) = fit(w, h, long_edge.clamp(ALIGN, 2048));
        let (pw, ph) = (iw.div_ceil(ALIGN) * ALIGN, ih.div_ceil(ALIGN) * ALIGN);
        let px = imagenet_pixels(rgb, w, h, iw, ih, pw, ph)?;
        let t = Tensor::from_vec(px, (1, 3, ph, pw), &self.device)?;
        let logits = self.forward(&t)?;
        let (c, oh, ow) = logits.dims3()?;
        let probs = candle_nn::ops::softmax(&logits, 0)?;
        let probs: Vec<f32> = probs.flatten_all()?.to_dtype(DType::F32)?.to_vec1()?;
        // the part of the output grid that covers the image (not the padding)
        let (vw, vh) = ((iw * ow).div_ceil(pw).clamp(1, ow), (ih * oh).div_ceil(ph).clamp(1, oh));
        Ok(Parsed { classes: c, w: ow, h: oh, vw, vh, probs })
    }
}

/// `w × h` scaled so its long edge is `long` (at least 1 × 1).
fn fit(w: usize, h: usize, long: usize) -> (usize, usize) {
    let (w, h) = (w.max(1), h.max(1));
    if w >= h { (long, (h * long).div_ceil(w).max(1)) } else { ((w * long).div_ceil(h).max(1), long) }
}

/// An 8-bit RGB image resampled to `iw × ih` (antialiased bilinear), normalized with ImageNet's
/// mean and deviation, channel-first in a `pw × ph` frame padded with zeros (the mean).
fn imagenet_pixels(rgb: &[u8], w: usize, h: usize, iw: usize, ih: usize, pw: usize, ph: usize) -> Result<Vec<f32>> {
    let resized = crate::mask::resize_rgb(rgb, w, h, iw, ih)?;
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let mut out = vec![0f32; 3 * pw * ph];
    for y in 0..ih {
        for x in 0..iw {
            for c in 0..3 {
                if let (Some(v), Some(o)) = (resized.get((y * iw + x) * 3 + c), out.get_mut((c * ph + y) * pw + x)) {
                    *o = (*v / 255.0 - MEAN[c]) / STD[c];
                }
            }
        }
    }
    Ok(out)
}

/// A parse: `classes` probability planes of `w × h` cells, of which the top-left `vw × vh` cover
/// the image.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub classes: usize,
    pub w: usize,
    pub h: usize,
    pub vw: usize,
    pub vh: usize,
    pub probs: Vec<f32>,
}

impl Parsed {
    /// The summed probability of `classes` at each image cell (`vw × vh`, row-major).
    pub fn prob(&self, classes: &[usize]) -> Vec<f32> {
        let plane = self.w * self.h;
        let mut out = vec![0f32; self.vw * self.vh];
        for c in classes.iter().filter(|c| **c < self.classes) {
            let base = c * plane;
            for y in 0..self.vh {
                for x in 0..self.vw {
                    if let (Some(o), Some(p)) = (out.get_mut(y * self.vw + x), self.probs.get(base + y * self.w + x)) {
                        *o += *p;
                    }
                }
            }
        }
        out
    }

    /// The most likely class at each image cell (`vw × vh`).
    pub fn argmax(&self) -> Vec<usize> {
        let plane = self.w * self.h;
        let mut out = vec![0usize; self.vw * self.vh];
        for y in 0..self.vh {
            for x in 0..self.vw {
                let mut best = (0, f32::MIN);
                for c in 0..self.classes {
                    if let Some(p) = self.probs.get(c * plane + y * self.w + x)
                        && *p > best.1
                    {
                        best = (c, *p);
                    }
                }
                if let Some(o) = out.get_mut(y * self.vw + x) {
                    *o = best.0;
                }
            }
        }
        out
    }
}

/// Probabilities (`w × h`) resampled bilinearly to a `side × side` grid stretched over the same
/// image, as logits (clamped to ±16).
pub fn square_logits(p: &[f32], w: usize, h: usize, side: usize) -> Vec<f32> {
    let at = |x: usize, y: usize| p.get(y.min(h.saturating_sub(1)) * w + x.min(w.saturating_sub(1))).copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(side * side);
    for oy in 0..side {
        let fy = ((oy as f32 + 0.5) / side as f32 * h as f32 - 0.5).clamp(0.0, h.saturating_sub(1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy - fy.floor());
        for ox in 0..side {
            let fx = ((ox as f32 + 0.5) / side as f32 * w as f32 - 0.5).clamp(0.0, w.saturating_sub(1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx - fx.floor());
            let v = (at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx) * (1.0 - ty) + (at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx) * ty;
            out.push(logit(v));
        }
    }
    out
}

/// `ln(p / (1 − p))`, clamped to ±16.
pub fn logit(p: f32) -> f32 {
    let p = p.clamp(1e-7, 1.0 - 1e-7);
    (p / (1.0 - p)).ln().clamp(-16.0, 16.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const B0: &str = r#"{"hidden_sizes":[32,64,160,256],"depths":[1,1,1,1],"num_attention_heads":[1,2,5,8],"sr_ratios":[8,4,2,1],
        "patch_sizes":[7,3,3,3],"strides":[4,2,2,2],"mlp_ratios":[4,4,4,4],"decoder_hidden_size":32,"id2label":{"0":"sky","1":"tree ","2":"person"}}"#;

    #[test]
    fn configs_are_checked() {
        let c = Config::parse(B0).unwrap();
        assert_eq!(c.labels(), ["sky", "tree", "person"]);
        assert!(Config::parse(r#"{"hidden_sizes":[32]}"#).is_err());
        let bad = B0.replace("\"depths\":[1,1,1,1]", "\"depths\":[1,1,1,100000]");
        assert!(Config::parse(&bad).is_err());
        let heads = B0.replace("\"num_attention_heads\":[1,2,5,8]", "\"num_attention_heads\":[3,2,5,8]");
        assert!(Config::parse(&heads).is_err(), "hidden size not a multiple of the heads");
        assert!(Config::parse("not json").is_err());
    }

    #[test]
    fn depthwise_conv_matches_a_direct_sum() {
        let dev = Device::Cpu;
        let (c, h, w) = (3, 4, 5);
        let x: Vec<f32> = (0..c * h * w).map(|i| (i as f32 * 0.37).sin()).collect();
        let k: Vec<f32> = (0..c * 9).map(|i| (i as f32 * 0.11).cos()).collect();
        let b = [0.5f32, -0.25, 0.1];
        let conv = DepthwiseConv { weight: Tensor::from_vec(k.clone(), (c, 9), &dev).unwrap(), bias: Tensor::from_vec(b.to_vec(), c, &dev).unwrap() };
        let y = conv.forward(&Tensor::from_vec(x.clone(), (1, c, h, w), &dev).unwrap()).unwrap();
        let y: Vec<f32> = y.flatten_all().unwrap().to_vec1().unwrap();
        for ch in 0..c {
            for oy in 0..h {
                for ox in 0..w {
                    let mut s = b[ch];
                    for ky in 0..3 {
                        for kx in 0..3 {
                            let (iy, ix) = (oy as i64 + ky as i64 - 1, ox as i64 + kx as i64 - 1);
                            if iy >= 0 && ix >= 0 && (iy as usize) < h && (ix as usize) < w {
                                s += x[(ch * h + iy as usize) * w + ix as usize] * k[ch * 9 + ky * 3 + kx];
                            }
                        }
                    }
                    assert!((y[(ch * h + oy) * w + ox] - s).abs() < 1e-5);
                }
            }
        }
    }

    #[test]
    fn blocked_attention_equals_the_whole() {
        let dev = Device::Cpu;
        let q = Tensor::randn(0f32, 1.0, (1, 2, 37, 4), &dev).unwrap();
        let k = Tensor::randn(0f32, 1.0, (1, 2, 9, 4), &dev).unwrap();
        let v = Tensor::randn(0f32, 1.0, (1, 2, 9, 4), &dev).unwrap();
        let whole = sdpa(&q, &k, &v, None, 0.5).unwrap();
        let mut parts = Vec::new();
        for s in (0..37).step_by(10) {
            let len = 10.min(37 - s);
            parts.push(sdpa(&q.narrow(2, s, len).unwrap(), &k, &v, None, 0.5).unwrap());
        }
        let blocked = Tensor::cat(&parts, 2).unwrap();
        let d: f32 = (whole - blocked).unwrap().abs().unwrap().max_all().unwrap().to_scalar().unwrap();
        assert!(d < 1e-6);
    }

    #[test]
    fn square_logits_resample_and_clamp() {
        // left half sure, right half not: the square grid keeps the split at the middle
        let p: Vec<f32> = (0..8 * 4).map(|i| if i % 8 < 4 { 1.0 } else { 0.0 }).collect();
        let l = square_logits(&p, 8, 4, 16);
        assert_eq!(l.len(), 256);
        assert!(l[0] >= 15.9 && l[15] <= -15.9);
        assert!(l.iter().all(|v| v.is_finite()));
        // empty input stays finite
        assert!(square_logits(&[], 0, 0, 4).iter().all(|v| *v <= -15.9));
        assert_eq!(fit(4000, 3000, 1024), (1024, 768));
        assert_eq!(fit(1, 100_000, 1024), (1, 1024));
    }
}
