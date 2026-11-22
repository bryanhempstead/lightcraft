//! CLIP (Radford et al., 2021) image and text encoders for natural-language search: a photo and a phrase are each
//! turned into a unit-length 512-number embedding, and the cosine of the two says how well the phrase describes the
//! photo. Weights: LAION's ViT-B/32 trained on LAION-2B (MIT), in Hugging Face's `CLIPModel` layout.
//!
//! The architecture follows candle-transformers' `models::clip` (Apache-2.0 / MIT, Copyright the candle authors),
//! rewritten here for our needs (configured from `config.json`; the patch embedding as one matrix product, since its
//! stride equals its size). Runs on Metal on macOS, else the CPU.

use std::path::Path;

use candle_core::{D, DType, Device, Module, Tensor};
use candle_nn::{Embedding, LayerNorm, Linear, VarBuilder};

use crate::image::Rgb8;
use crate::{Error, Result};

fn e(err: candle_core::Error) -> Error {
    Error::Model(format!("CLIP: {err}"))
}

/// The model sizes `config.json` gives.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub text_hidden: usize,
    pub text_layers: usize,
    pub text_heads: usize,
    pub text_intermediate: usize,
    pub vocab: usize,
    pub max_positions: usize,
    pub vision_hidden: usize,
    pub vision_layers: usize,
    pub vision_heads: usize,
    pub vision_intermediate: usize,
    pub image_size: usize,
    pub patch: usize,
    pub projection: usize,
    /// `quick_gelu` (OpenAI's checkpoints) rather than `gelu` (LAION's).
    pub quick_gelu: bool,
    pub eps: f64,
}

impl Config {
    /// LAION / OpenAI ViT-B/32.
    pub fn vit_b32() -> Config {
        Config {
            text_hidden: 512,
            text_layers: 12,
            text_heads: 8,
            text_intermediate: 2048,
            vocab: 49408,
            max_positions: 77,
            vision_hidden: 768,
            vision_layers: 12,
            vision_heads: 12,
            vision_intermediate: 3072,
            image_size: 224,
            patch: 32,
            projection: 512,
            quick_gelu: false,
            eps: 1e-5,
        }
    }

    /// From a Hugging Face `config.json` (missing values: ViT-B/32's).
    pub fn from_json(text: &str) -> Result<Config> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|err| Error::Model(format!("config.json: {err}")))?;
        let d = Config::vit_b32();
        let num = |section: &str, key: &str, default: usize| -> usize {
            v.get(section)
                .and_then(|s| s.get(key))
                .and_then(serde_json::Value::as_u64)
                .map(|n| n as usize)
                .filter(|&n| (1..=1 << 20).contains(&n))
                .unwrap_or(default)
        };
        let act = v.get("text_config").and_then(|s| s.get("hidden_act")).and_then(serde_json::Value::as_str).unwrap_or("gelu");
        let c = Config {
            text_hidden: num("text_config", "hidden_size", d.text_hidden),
            text_layers: num("text_config", "num_hidden_layers", d.text_layers),
            text_heads: num("text_config", "num_attention_heads", d.text_heads),
            text_intermediate: num("text_config", "intermediate_size", d.text_intermediate),
            vocab: num("text_config", "vocab_size", d.vocab),
            max_positions: num("text_config", "max_position_embeddings", d.max_positions),
            vision_hidden: num("vision_config", "hidden_size", d.vision_hidden),
            vision_layers: num("vision_config", "num_hidden_layers", d.vision_layers),
            vision_heads: num("vision_config", "num_attention_heads", d.vision_heads),
            vision_intermediate: num("vision_config", "intermediate_size", d.vision_intermediate),
            image_size: num("vision_config", "image_size", d.image_size),
            patch: num("vision_config", "patch_size", d.patch),
            projection: v.get("projection_dim").and_then(serde_json::Value::as_u64).map(|n| n as usize).unwrap_or(d.projection),
            quick_gelu: act == "quick_gelu",
            eps: 1e-5,
        };
        if !c.image_size.is_multiple_of(c.patch)
            || !c.text_hidden.is_multiple_of(c.text_heads)
            || !c.vision_hidden.is_multiple_of(c.vision_heads)
            || c.layers_too_many()
        {
            return Err(Error::Model("config.json describes a CLIP model of unsupported shape".into()));
        }
        Ok(c)
    }

    fn layers_too_many(&self) -> bool {
        self.text_layers > 64 || self.vision_layers > 64
    }
}

struct Attention {
    q: Linear,
    k: Linear,
    v: Linear,
    out: Linear,
    heads: usize,
    scale: f64,
}

impl Attention {
    fn new(vb: VarBuilder, hidden: usize, heads: usize) -> candle_core::Result<Attention> {
        Ok(Attention {
            q: candle_nn::linear(hidden, hidden, vb.pp("q_proj"))?,
            k: candle_nn::linear(hidden, hidden, vb.pp("k_proj"))?,
            v: candle_nn::linear(hidden, hidden, vb.pp("v_proj"))?,
            out: candle_nn::linear(hidden, hidden, vb.pp("out_proj"))?,
            heads,
            scale: ((hidden / heads) as f64).powf(-0.5),
        })
    }

    fn forward(&self, x: &Tensor, mask: Option<&Tensor>) -> candle_core::Result<Tensor> {
        let (b, n, h) = x.dims3()?;
        let hd = h / self.heads;
        let shape = |t: Tensor| t.reshape((b, n, self.heads, hd))?.transpose(1, 2)?.contiguous();
        let q = shape((self.q.forward(x)? * self.scale)?)?;
        let k = shape(self.k.forward(x)?)?;
        let v = shape(self.v.forward(x)?)?;
        let mut w = q.matmul(&k.t()?.contiguous()?)?;
        if let Some(m) = mask {
            w = w.broadcast_add(m)?;
        }
        let w = candle_nn::ops::softmax_last_dim(&w)?;
        let y = w.matmul(&v)?.transpose(1, 2)?.contiguous()?.reshape((b, n, h))?;
        self.out.forward(&y)
    }
}

struct Layer {
    ln1: LayerNorm,
    attn: Attention,
    ln2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    quick_gelu: bool,
}

impl Layer {
    fn new(vb: VarBuilder, hidden: usize, heads: usize, inter: usize, c: &Config) -> candle_core::Result<Layer> {
        Ok(Layer {
            ln1: candle_nn::layer_norm(hidden, c.eps, vb.pp("layer_norm1"))?,
            attn: Attention::new(vb.pp("self_attn"), hidden, heads)?,
            ln2: candle_nn::layer_norm(hidden, c.eps, vb.pp("layer_norm2"))?,
            fc1: candle_nn::linear(hidden, inter, vb.pp("mlp").pp("fc1"))?,
            fc2: candle_nn::linear(inter, hidden, vb.pp("mlp").pp("fc2"))?,
            quick_gelu: c.quick_gelu,
        })
    }

    fn forward(&self, x: &Tensor, mask: Option<&Tensor>) -> candle_core::Result<Tensor> {
        let x = (x + self.attn.forward(&self.ln1.forward(x)?, mask)?)?;
        let h = self.fc1.forward(&self.ln2.forward(&x)?)?;
        let h = if self.quick_gelu { (&h * candle_nn::ops::sigmoid(&(&h * 1.702)?)?)? } else { h.gelu_erf()? };
        x + self.fc2.forward(&h)?
    }
}

/// The CLIP model: both encoders and their projections.
pub struct Clip {
    config: Config,
    device: Device,
    // text
    token: Embedding,
    text_pos: Tensor,
    text_layers: Vec<Layer>,
    text_ln: LayerNorm,
    text_proj: Linear,
    // vision
    class: Tensor,
    patch: Tensor,
    vision_pos: Tensor,
    pre_ln: LayerNorm,
    vision_layers: Vec<Layer>,
    post_ln: LayerNorm,
    vision_proj: Linear,
    tokenizer: Tokenizer,
}

const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

impl Clip {
    /// Load `model.safetensors`, `config.json` (optional), `vocab.json` and `merges.txt` from `dir`.
    pub fn load(dir: &Path, device: &Device) -> Result<Clip> {
        let config = match std::fs::read_to_string(dir.join("config.json")) {
            Ok(t) => Config::from_json(&t)?,
            Err(_) => Config::vit_b32(),
        };
        let weights = dir.join(crate::models::CLIP_WEIGHTS);
        if !weights.is_file() {
            return Err(Error::NotInstalled(format!("{} is missing", weights.display())));
        }
        let tokenizer = Tokenizer::load(dir)?;
        // SAFETY-free: the weights are read into memory (no mmap)
        let bytes = std::fs::read(&weights).map_err(|err| Error::Io(format!("{}: {err}", weights.display())))?;
        let vb = VarBuilder::from_buffered_safetensors(bytes, DType::F32, device).map_err(e)?;
        Clip::build(config, vb, device.clone(), tokenizer).map_err(e)
    }

    fn build(c: Config, vb: VarBuilder, device: Device, tokenizer: Tokenizer) -> candle_core::Result<Clip> {
        let t = vb.pp("text_model");
        let token = candle_nn::embedding(c.vocab, c.text_hidden, t.pp("embeddings").pp("token_embedding"))?;
        let text_pos = t.pp("embeddings").pp("position_embedding").get((c.max_positions, c.text_hidden), "weight")?;
        let text_layers = (0..c.text_layers)
            .map(|i| Layer::new(t.pp("encoder").pp("layers").pp(i.to_string()), c.text_hidden, c.text_heads, c.text_intermediate, &c))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let text_ln = candle_nn::layer_norm(c.text_hidden, c.eps, t.pp("final_layer_norm"))?;
        let text_proj = candle_nn::linear_no_bias(c.text_hidden, c.projection, vb.pp("text_projection"))?;
        let v = vb.pp("vision_model");
        let ve = v.pp("embeddings");
        let class = ve.get(c.vision_hidden, "class_embedding")?;
        let patch = ve.pp("patch_embedding").get((c.vision_hidden, 3, c.patch, c.patch), "weight")?;
        // [hidden, 3·p·p] for one matrix product over the unfolded patches
        let patch = patch.reshape((c.vision_hidden, 3 * c.patch * c.patch))?.t()?.contiguous()?;
        let n = (c.image_size / c.patch).pow(2) + 1;
        let vision_pos = ve.pp("position_embedding").get((n, c.vision_hidden), "weight")?;
        let pre_ln = candle_nn::layer_norm(c.vision_hidden, c.eps, v.pp("pre_layrnorm"))?;
        let vision_layers = (0..c.vision_layers)
            .map(|i| Layer::new(v.pp("encoder").pp("layers").pp(i.to_string()), c.vision_hidden, c.vision_heads, c.vision_intermediate, &c))
            .collect::<candle_core::Result<Vec<_>>>()?;
        let post_ln = candle_nn::layer_norm(c.vision_hidden, c.eps, v.pp("post_layernorm"))?;
        let vision_proj = candle_nn::linear_no_bias(c.vision_hidden, c.projection, vb.pp("visual_projection"))?;
        Ok(Clip {
            config: c,
            device,
            token,
            text_pos,
            text_layers,
            text_ln,
            text_proj,
            class,
            patch,
            vision_pos,
            pre_ln,
            vision_layers,
            post_ln,
            vision_proj,
            tokenizer,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Embedding length.
    pub fn dim(&self) -> usize {
        self.config.projection
    }

    /// The model input for `img`: shortest side resized to the model's size, centre crop, normalised, CHW.
    pub fn preprocess(&self, img: &Rgb8) -> Vec<f32> {
        let s = self.config.image_size;
        let (w, h) = (img.width.max(1), img.height.max(1));
        let k = s as f32 / w.min(h) as f32;
        let (rw, rh) = (((w as f32 * k).round() as usize).max(s), ((h as f32 * k).round() as usize).max(s));
        // area-average when shrinking (the usual case), bilinear otherwise
        let resized = if rw <= w {
            img.resize_area(rw, rh)
        } else {
            img.warp_affine([k, 0.0, 0.0, 0.0, k, 0.0], rw, rh).unwrap_or_else(|| img.resize_area(rw, rh))
        };
        let (x0, y0) = ((rw - s) / 2, (rh - s) / 2);
        let mut out = vec![0f32; 3 * s * s];
        for y in 0..s {
            for x in 0..s {
                let i = ((y + y0) * rw + x + x0) * 3;
                let Some(p) = resized.data.get(i..i + 3) else { continue };
                for c in 0..3 {
                    if let Some(o) = out.get_mut(c * s * s + y * s + x) {
                        *o = (f32::from(p[c]) / 255.0 - MEAN[c]) / STD[c];
                    }
                }
            }
        }
        out
    }

    /// Unit-length embeddings of preprocessed pictures ([`Self::preprocess`]), one row each.
    pub fn embed_images(&self, inputs: &[Vec<f32>]) -> Result<Vec<Vec<f32>>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let c = &self.config;
        let (s, p) = (c.image_size, c.patch);
        let g = s / p;
        let b = inputs.len();
        let flat: Vec<f32> = inputs.iter().flat_map(|v| v.iter().copied()).collect();
        if flat.len() != b * 3 * s * s {
            return Err(Error::Model("a CLIP input has the wrong size".into()));
        }
        let run = || -> candle_core::Result<Vec<Vec<f32>>> {
            let x = Tensor::from_vec(flat, (b, 3, g, p, g, p), &self.device)?;
            // patches: [b, g·g, 3·p·p] (channel, row, column within the patch, as the conv weight is laid out)
            let x = x.permute((0, 2, 4, 1, 3, 5))?.contiguous()?.reshape((b * g * g, 3 * p * p))?;
            let x = x.matmul(&self.patch)?.reshape((b, g * g, c.vision_hidden))?;
            let cls = self.class.reshape((1, 1, c.vision_hidden))?.broadcast_as((b, 1, c.vision_hidden))?;
            let x = Tensor::cat(&[&cls, &x], 1)?.broadcast_add(&self.vision_pos.unsqueeze(0)?)?;
            let mut x = self.pre_ln.forward(&x)?;
            for l in &self.vision_layers {
                x = l.forward(&x, None)?;
            }
            let pooled = self.post_ln.forward(&x.narrow(1, 0, 1)?.squeeze(1)?)?;
            let y = self.vision_proj.forward(&pooled)?;
            let y = y.broadcast_div(&y.sqr()?.sum_keepdim(D::Minus1)?.sqrt()?)?;
            y.to_device(&Device::Cpu)?.to_vec2::<f32>()
        };
        run().map_err(e)
    }

    /// The unit-length embedding of a phrase.
    pub fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let c = &self.config;
        let (ids, eos) = self.tokenizer.encode(text, c.max_positions);
        let n = ids.len();
        let run = || -> candle_core::Result<Vec<f32>> {
            let ids = Tensor::from_vec(ids, (1, n), &self.device)?;
            let x = self.token.forward(&ids)?.broadcast_add(&self.text_pos.narrow(0, 0, n)?.unsqueeze(0)?)?;
            // causal mask
            let mask: Vec<f32> = (0..n).flat_map(|i| (0..n).map(move |j| if j > i { f32::NEG_INFINITY } else { 0.0 })).collect();
            let mask = Tensor::from_vec(mask, (1, 1, n, n), &self.device)?;
            let mut x = x;
            for l in &self.text_layers {
                x = l.forward(&x, Some(&mask))?;
            }
            let x = self.text_ln.forward(&x)?;
            let pooled = x.narrow(1, eos, 1)?.squeeze(1)?;
            let y = self.text_proj.forward(&pooled)?;
            let y = y.broadcast_div(&y.sqr()?.sum_keepdim(D::Minus1)?.sqrt()?)?;
            y.to_device(&Device::Cpu)?.flatten_all()?.to_vec1::<f32>()
        };
        run().map_err(e)
    }
}

// ------------------------------------------------------------------------------------------------ tokenizer

/// CLIP's byte-pair-encoding tokenizer (lower-cased, `</w>` word ends), from OpenAI's `simple_tokenizer.py` (MIT):
/// the same algorithm as `lightcraft_segment::tokenizer`, without padding.
pub struct Tokenizer {
    vocab: std::collections::HashMap<String, u32>,
    ranks: std::collections::HashMap<(String, String), usize>,
    byte_char: [char; 256],
}

const BOS: u32 = 49406;
const EOS: u32 = 49407;

fn bytes_to_unicode() -> [char; 256] {
    let mut map = ['\0'; 256];
    let mut n = 0u32;
    for b in 0..=255u8 {
        let printable = (b'!'..=b'~').contains(&b) || (0xA1..=0xAC).contains(&b) || (0xAE..=0xFF).contains(&b);
        let c = if printable {
            u32::from(b)
        } else {
            n += 1;
            255 + n
        };
        if let Some(slot) = map.get_mut(usize::from(b)) {
            *slot = char::from_u32(c).unwrap_or('?');
        }
    }
    map
}

impl Tokenizer {
    pub fn load(dir: &Path) -> Result<Tokenizer> {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).map_err(|err| Error::NotInstalled(format!("{name}: {err}")));
        let vocab: std::collections::HashMap<String, u32> =
            serde_json::from_str(&read(crate::models::CLIP_VOCAB)?).map_err(|err| Error::Model(format!("vocab.json: {err}")))?;
        Ok(Tokenizer::new(vocab, &read(crate::models::CLIP_MERGES)?))
    }

    pub fn new(vocab: std::collections::HashMap<String, u32>, merges: &str) -> Tokenizer {
        let ranks = merges
            .lines()
            .filter(|l| !l.starts_with("#version") && !l.trim().is_empty())
            .filter_map(|l| l.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())))
            .enumerate()
            .map(|(i, p)| (p, i))
            .collect();
        Tokenizer { vocab, ranks, byte_char: bytes_to_unicode() }
    }

    /// Token ids (start, words, end; at most `context`) and the position of the end token.
    pub fn encode(&self, text: &str, context: usize) -> (Vec<u32>, usize) {
        let context = context.max(2);
        let mut ids = vec![BOS];
        for word in pre_tokenize(&text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()) {
            let mapped: String = word.bytes().map(|b| self.byte_char.get(usize::from(b)).copied().unwrap_or('?')).collect();
            for piece in self.bpe(&mapped) {
                if let Some(id) = self.vocab.get(&piece) {
                    ids.push(*id);
                }
            }
        }
        ids.truncate(context - 1);
        ids.push(EOS);
        let eos = ids.len() - 1;
        (ids, eos)
    }

    fn bpe(&self, word: &str) -> Vec<String> {
        let mut syms: Vec<String> = word.chars().map(String::from).collect();
        if let Some(last) = syms.last_mut() {
            last.push_str("</w>");
        }
        // a word is at most a few hundred symbols; each merge removes at least one
        for _ in 0..4096 {
            let best = syms
                .windows(2)
                .enumerate()
                .filter_map(|(i, w)| match w {
                    [a, b] => self.ranks.get(&(a.clone(), b.clone())).map(|r| (*r, i)),
                    _ => None,
                })
                .min();
            let Some((_, i)) = best else { break };
            let (Some(a), Some(b)) = (syms.get(i).cloned(), syms.get(i + 1).cloned()) else { break };
            let merged = a.clone() + &b;
            let mut out = Vec::with_capacity(syms.len());
            let mut j = 0;
            while j < syms.len() {
                if j + 1 < syms.len() && syms.get(j) == Some(&a) && syms.get(j + 1) == Some(&b) {
                    out.push(merged.clone());
                    j += 2;
                } else {
                    out.extend(syms.get(j).cloned());
                    j += 1;
                }
            }
            syms = out;
            if syms.len() == 1 {
                break;
            }
        }
        syms
    }
}

/// CLIP's pre-tokenizer: contractions, letter runs, single digits, and runs of other non-space characters.
fn pre_tokenize(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().take(4096).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '\'' {
            let rest: String = chars.get(i + 1..(i + 3).min(chars.len())).map(|s| s.iter().collect()).unwrap_or_default();
            if let Some(k) = ["re", "ve", "ll"]
                .iter()
                .find(|k| rest.starts_with(**k))
                .map(|k| k.len())
                .or_else(|| rest.chars().next().filter(|c| matches!(c, 's' | 't' | 'm' | 'd')).map(|_| 1))
            {
                out.push(chars.get(i..i + 1 + k).map(|s| s.iter().collect()).unwrap_or_default());
                i += 1 + k;
                continue;
            }
        }
        let start = i;
        if c.is_alphabetic() {
            while chars.get(i).is_some_and(|c| c.is_alphabetic()) {
                i += 1;
            }
        } else if c.is_numeric() {
            i += 1;
        } else {
            while chars.get(i).is_some_and(|c| !c.is_whitespace() && !c.is_alphabetic() && !c.is_numeric()) {
                i += 1;
            }
        }
        out.push(chars.get(start..i).map(|s| s.iter().collect()).unwrap_or_default());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_like_clip() {
        assert_eq!(pre_tokenize("a dog's toy, 25 cats!"), ["a", "dog", "'s", "toy", ",", "2", "5", "cats", "!"]);
        let vocab = [("c", 1), ("a", 2), ("t</w>", 3), ("ca", 4), ("cat</w>", 5)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        let t = Tokenizer::new(vocab, "#version: 0.2\nc a\nca t</w>\n");
        assert_eq!(t.encode("  CAT ", 77), (vec![BOS, 5, EOS], 2));
        assert_eq!(t.encode("cat cat cat", 3), (vec![BOS, 5, EOS], 2));
    }

    #[test]
    fn config_from_json() {
        let c = Config::from_json(r#"{"projection_dim": 512, "text_config": {"hidden_act": "quick_gelu"}, "vision_config": {"patch_size": 16}}"#)
            .unwrap();
        assert!(c.quick_gelu);
        assert_eq!(c.patch, 16);
        assert!(Config::from_json(r#"{"vision_config": {"patch_size": 33}}"#).is_err());
        assert!(Config::from_json("nope").is_err());
    }
}
