//! Generative Remove's diffusion fill (`gen.ai.`): Stable Diffusion 1.5 Inpainting (RunwayML, CreativeML
//! OpenRAIL-M; the `stable-diffusion-v1-5/stable-diffusion-inpainting` copy on Hugging Face) on candle, with the
//! UNet, VAE and CLIP text encoder of candle-transformers ([`crate::sd`]).
//!
//! How a hole is filled:
//! - LaMa ([`crate::lama`]) fills it first (structure: a horizon, a path, a wall's edge continue through the hole);
//!   the diffusion model then repaints that fill from a partly noised start ([`Params::strength`]), which gives it
//!   real texture (grass, gravel, skin of water) instead of LaMa's smooth guess. Without LaMa it starts from noise.
//! - The model sees the crop at [`Params::edge`] pixels across (SD 1.5 was trained at 512). A crop much larger than
//!   that is then repainted again in overlapping tiles at up to [`Params::refine_edge`] pixels across, lightly
//!   ([`Params::refine_strength`]), so a large fill gets detail near the photo's own resolution; past that it is
//!   resampled and given the grain of the photo around it ([`match_grain`]).
//! - The fill is joined to the photo seamlessly ([`seamless`]): the difference between the fill and the photo just
//!   outside the hole is spread smoothly over the hole (a membrane, like Poisson cloning), so there is no seam in
//!   colour or brightness at the mask's edge.
//! - Every random number comes from [`Params::seed`] (a counter-based generator on the CPU), and the scheduler adds
//!   none (DDIM, eta 0): the same seed on the same crop gives the same fill, so a re-render or another Mac matches.
//!   Variations are seeds.
//!
//! Prompts are fixed (a removal fills with what is around the hole, so no prompt is asked for): the token ids are
//! below, checked against the model's `vocab.json` by an opt-in test.

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use lightcraft_raster::Rgb32f;

use crate::sd;
use crate::{Error, Result};

/// The model's files in its folder (renamed from the Hugging Face layout when downloaded).
pub const UNET: &str = "unet.safetensors";
pub const VAE: &str = "vae.safetensors";
pub const TEXT: &str = "text_encoder.safetensors";

/// The latent space's scale (SD 1.5's VAE).
const LATENT_SCALE: f64 = 0.18215;
/// The image size must be a multiple of this (VAE ×8, UNet ×8).
pub const MULTIPLE: usize = 64;
/// CLIP's context length and its start / end tokens.
const CONTEXT: usize = 77;
const BOS: u32 = 49406;
const EOS: u32 = 49407;

/// "empty background, seamless natural texture, high quality photo" (CLIP BPE ids).
pub const PROMPT: &[u32] = &[7893, 5994, 267, 29373, 3288, 16505, 267, 1400, 3027, 1125];
/// "person, people, object, animal, text, logo, frame, border, blurry, artifacts" (CLIP BPE ids).
pub const NEGATIVE: &[u32] = &[2533, 267, 1047, 267, 14115, 267, 4668, 267, 4160, 267, 5750, 267, 6481, 267, 6177, 267, 21977, 267, 30249];

/// How a fill is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// The variation's seed.
    pub seed: u64,
    /// DDIM steps for a fill from noise (a partial start runs `steps × strength` of them).
    pub steps: usize,
    /// How much of LaMa's fill is repainted (0..1; 1 = from noise).
    pub strength: f32,
    /// Classifier-free guidance towards the prompt and away from the negative one (≤ 1, the default: none — one
    /// UNet pass per step instead of two; with LaMa's start it added little on test photos for twice the time).
    pub guidance: f32,
    /// The model's view of the crop, pixels across (a multiple of [`MULTIPLE`]).
    pub edge: usize,
    /// Repaint large crops again in tiles up to this many pixels across (0: never).
    pub refine_edge: usize,
    /// How much the tiles repaint.
    pub refine_strength: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params { seed: 0, steps: 16, strength: 0.75, guidance: 1.0, edge: 512, refine_edge: 1536, refine_strength: 0.3 }
    }
}

/// `progress(fraction)`; false cancels.
pub type Progress<'a> = dyn Fn(f32) -> bool + Sync + 'a;

/// Stable Diffusion 1.5 Inpainting, loaded.
pub struct GenFiller {
    unet: sd::unet_2d::UNet2DConditionModel,
    vae: sd::vae::AutoEncoderKL,
    /// `[2, 77, 768]`: the negative prompt, then the prompt.
    context: Tensor,
    device: Device,
    dtype: DType,
}

fn tokens(ids: &[u32]) -> Vec<u32> {
    let mut t = Vec::with_capacity(CONTEXT);
    t.push(BOS);
    t.extend(ids.iter().copied().take(CONTEXT - 2));
    t.push(EOS);
    t.resize(CONTEXT, EOS);
    t
}

fn read(path: &Path) -> Result<Vec<u8>> {
    // the UNet is 1.7 GB; anything past 4 GB is not this model
    let len = std::fs::metadata(path).map_err(|e| Error::Model(format!("{}: {e}", path.display())))?.len();
    if len > 4 << 30 {
        return Err(Error::Model(format!("{}: too large to be the model", path.display())));
    }
    std::fs::read(path).map_err(|e| Error::Model(format!("{}: {e}", path.display())))
}

impl GenFiller {
    /// Load the model from `dir` ([`UNET`], [`VAE`], [`TEXT`]) on `device` (half precision on the GPU).
    pub fn load(dir: &Path, device: Device) -> Result<Self> {
        let dtype = if device.is_cpu() { DType::F32 } else { DType::F16 };
        let _gpu = crate::gpu_lock(&device);
        let vb = |file: &str, dt: DType| -> Result<VarBuilder<'static>> {
            let p = dir.join(file);
            VarBuilder::from_buffered_safetensors(read(&p)?, dt, &device).map_err(|e| Error::Model(format!("{}: {e}", p.display())))
        };
        let err = |file: &'static str| move |e: candle_core::Error| Error::Model(format!("{}: {e}", dir.join(file).display()));
        let context = {
            let clip = sd::clip::ClipTextTransformer::new(vb(TEXT, dtype)?, &sd::clip::Config::v1_5()).map_err(err(TEXT))?;
            // one prompt at a time (the upstream causal mask is for a batch of one)
            let encode =
                |ids: Vec<u32>| -> candle_core::Result<Tensor> { candle_nn::Module::forward(&clip, &Tensor::from_vec(ids, (1, CONTEXT), &device)?) };
            Tensor::cat(&[encode(tokens(NEGATIVE)).map_err(err(TEXT))?, encode(tokens(PROMPT)).map_err(err(TEXT))?], 0)?
        };
        // the VAE in half precision too (SD 1.5's is fine in it; a decode out of range is an error, not a picture)
        let vae = sd::vae::AutoEncoderKL::new(vb(VAE, dtype)?, 3, 3, sd::vae_config()).map_err(err(VAE))?;
        let unet = sd::unet_2d::UNet2DConditionModel::new(vb(UNET, dtype)?, 9, 4, false, sd::unet_config(None)).map_err(err(UNET))?;
        Ok(GenFiller { unet, vae, context, device, dtype })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Fill the pixels of `img` (display-encoded RGB, 0..1) where `hole` is set, starting from `init` (LaMa's
    /// fill of the same picture) when given. Returns the whole picture: the fill inside the hole, joined seamlessly;
    /// outside it the picture as it was.
    pub fn fill(&self, img: &Rgb32f, hole: &[bool], init: Option<&Rgb32f>, p: &Params, progress: &Progress) -> Result<Rgb32f> {
        let (w, h) = (img.width, img.height);
        if hole.len() != w * h || init.is_some_and(|i| (i.width, i.height) != (w, h)) {
            return Err(Error::Input("the hole does not match the picture".into()));
        }
        if w < 16 || h < 16 || w.saturating_mul(h) > 8192 * 8192 {
            return Err(Error::Input("the picture is too small or too large to fill".into()));
        }
        if !hole.iter().any(|m| *m) {
            return Ok(img.clone());
        }
        let edge = p.edge.clamp(MULTIPLE * 4, 1024) / MULTIPLE * MULTIPLE;
        let start = init.map_or_else(|| img.clone(), Rgb32f::clone);
        let long = w.max(h);
        let refine = p.refine_edge > 0 && long as f64 > edge as f64 * 1.5;
        let (share, rest) = if refine { (0.6, 0.4) } else { (1.0, 0.0) };
        let report = |f: f32| progress(f * share);
        // 1. the whole crop at the model's size
        let (mw, mh) = model_size(w, h, edge);
        let g =
            self.pass(&resize(&start, mw, mh), &resize_mask(hole, w, h, mw, mh), init.is_some(), p.strength, p.guidance, p.steps, p.seed, &report)?;
        // the model's whole picture at the crop's size (outside the hole it is the model's copy of the photo, which
        // `seamless` compares the photo with)
        let mut made = resize(&g, w, h);
        // 2. large crops: repainted again in tiles, nearer the photo's resolution
        let soft = if refine {
            let rl = p.refine_edge.min(long).max(edge);
            let (rw, rh) = ((w * rl).div_ceil(long).max(16), (h * rl).div_ceil(long).max(16));
            // what the tiles see (the photo with the fill so far) and the model's pictures, at the tiles' scale
            let mut canvas = resize(&compose(img, &made, hole), rw, rh);
            let mut gcanvas = resize(&made, rw, rh);
            let rhole = resize_mask(hole, w, h, rw, rh);
            let tiles = tiles_over(&rhole, rw, rh, edge);
            let n = tiles.len().max(1);
            for (k, (x0, y0, tw, th)) in tiles.into_iter().enumerate() {
                let sub = canvas.crop(x0, y0, tw, th);
                let shole: Vec<bool> = (0..th)
                    .flat_map(|y| (0..tw).map(move |x| (x, y)))
                    .map(|(x, y)| rhole.get((y0 + y) * rw + x0 + x).copied().unwrap_or(false))
                    .collect();
                if !shole.iter().any(|m| *m) {
                    continue;
                }
                let (mw, mh) = model_size(tw, th, edge);
                let tp = |f: f32| progress(share + rest * (k as f32 + f) / n as f32);
                let seed = p.seed ^ (0x9e37_79b9_7f4a_7c15u64.wrapping_mul(k as u64 + 1));
                let g = self.pass(&resize(&sub, mw, mh), &resize_mask(&shole, tw, th, mw, mh), true, p.refine_strength, 1.0, p.steps, seed, &tp)?;
                let g = resize(&g, tw, th);
                // feathered in (tiles overlap); the photo the next tiles see changes only inside the hole
                let (fx, fy) = ((tw / 8).max(1) as f32, (th / 8).max(1) as f32);
                for y in 0..th {
                    for x in 0..tw {
                        let ax = ((x as f32 + 0.5) / fx).min((tw - x) as f32 / fx).min(1.0);
                        let ay = ((y as f32 + 0.5) / fy).min((th - y) as f32 / fy).min(1.0);
                        let edge_x = x0 == 0 && x < tw / 2 || x0 + tw >= rw && x >= tw / 2;
                        let edge_y = y0 == 0 && y < th / 2 || y0 + th >= rh && y >= th / 2;
                        let a = if edge_x { 1.0 } else { ax } * if edge_y { 1.0 } else { ay };
                        let i = (y0 + y) * rw + x0 + x;
                        let Some(s) = g.data.get(y * tw + x) else { continue };
                        if let Some(d) = gcanvas.data.get_mut(i) {
                            *d = std::array::from_fn(|c| d[c] + (s[c] - d[c]) * a);
                        }
                        if shole.get(y * tw + x).copied().unwrap_or(false)
                            && let Some(d) = canvas.data.get_mut(i)
                        {
                            *d = std::array::from_fn(|c| d[c] + (s[c] - d[c]) * a);
                        }
                    }
                }
            }
            made = resize(&gcanvas, w, h);
            rl < long
        } else {
            long > edge
        };
        let mut out = seamless(img, &made, hole);
        if soft {
            match_grain(img, &mut out, hole, p.seed);
        }
        Ok(out)
    }

    /// One diffusion pass on a picture of the model's size: `start` (0..1) is the picture with its hole filled
    /// (LaMa) when `partial`, else only its surroundings count.
    #[allow(clippy::too_many_arguments)]
    fn pass(
        &self,
        start: &Rgb32f,
        hole: &[bool],
        partial: bool,
        strength: f32,
        guidance: f32,
        steps: usize,
        seed: u64,
        progress: &Progress,
    ) -> Result<Rgb32f> {
        let (w, h) = (start.width, start.height);
        let (lw, lh) = (w / 8, h / 8);
        let _gpu = crate::gpu_lock(&self.device);
        let d = &self.device;
        let to_tensor = |img: &Rgb32f, masked: bool| -> candle_core::Result<Tensor> {
            let n = w * h;
            let mut v = vec![0f32; 3 * n];
            for (i, px) in img.data.iter().enumerate().take(n) {
                let m = masked && hole.get(i).copied().unwrap_or(false);
                for c in 0..3 {
                    if let Some(o) = v.get_mut(c * n + i) {
                        *o = if m { 0.0 } else { px[c].clamp(0.0, 1.0) * 2.0 - 1.0 };
                    }
                }
            }
            Tensor::from_vec(v, (1, 3, h, w), d)
        };
        let encode = |x: Tensor| -> candle_core::Result<Tensor> { self.vae.encode(&x.to_dtype(self.dtype)?)?.mode() * LATENT_SCALE };
        // LIGHTCRAFT_PROFILE=1: the stages' times on stderr
        let profile = std::env::var_os("LIGHTCRAFT_PROFILE").is_some();
        let mut t0 = std::time::Instant::now();
        let masked = encode(to_tensor(start, true)?)?;
        let mask = {
            let mut m = vec![0f32; lw * lh];
            for (i, v) in m.iter_mut().enumerate() {
                let (x, y) = (i % lw * 8, i / lw * 8);
                if (y..y + 8).any(|yy| (x..x + 8).any(|xx| hole.get(yy * w + xx).copied().unwrap_or(false))) {
                    *v = 1.0;
                }
            }
            Tensor::from_vec(m, (1, 1, lh, lw), d)?.to_dtype(self.dtype)?
        };
        let mut sched = sd::scheduler(steps.clamp(1, 100))?;
        let ts = sched.timesteps().to_vec();
        let strength = if partial { strength.clamp(0.05, 1.0) } else { 1.0 };
        let run = ((ts.len() as f32 * strength).round() as usize).clamp(1, ts.len());
        let first = ts.len() - run;
        let noise = Tensor::from_vec(gaussian(seed, 4 * lw * lh), (1, 4, lh, lw), d)?.to_dtype(self.dtype)?;
        let mut latents = match (partial && run < ts.len(), ts.get(first)) {
            (true, Some(&t_start)) => {
                let z0 = encode(to_tensor(start, false)?)?;
                sched.add_noise(&z0, noise, t_start)?
            }
            _ => (noise * sched.init_noise_sigma())?,
        };
        if profile {
            d.synchronize()?;
            eprintln!("genfill {w}×{h}: VAE encode {:.2} s", t0.elapsed().as_secs_f64());
            t0 = std::time::Instant::now();
        }
        let cfg = guidance > 1.0;
        let (mask_in, masked_in, ctx) = if cfg {
            (Tensor::cat(&[&mask, &mask], 0)?, Tensor::cat(&[&masked, &masked], 0)?, self.context.clone())
        } else {
            (mask.clone(), masked.clone(), self.context.narrow(0, 1, 1)?)
        };
        for (k, &t) in ts.iter().enumerate().skip(first) {
            if !progress((k - first) as f32 / run as f32) {
                return Err(Error::Cancelled);
            }
            let x = if cfg { Tensor::cat(&[&latents, &latents], 0)? } else { latents.clone() };
            let x = sched.scale_model_input(x, t)?;
            let x = Tensor::cat(&[&x, &mask_in, &masked_in], 1)?;
            let eps = self.unet.forward(&x, t as f64, &ctx)?;
            let eps = if cfg {
                let parts = eps.chunk(2, 0)?;
                let (Some(u), Some(c)) = (parts.first(), parts.get(1)) else { return Err(Error::Model("guidance lost its halves".into())) };
                (u + ((c - u)? * f64::from(guidance))?)?
            } else {
                eps
            };
            latents = sched.step(&eps, t, &latents)?;
        }
        if profile {
            d.synchronize()?;
            let g = if cfg { " (guided, batch 2)" } else { "" };
            eprintln!("genfill {w}×{h}: {run} UNet steps{g}: {:.2} s", t0.elapsed().as_secs_f64());
        }
        let t1 = std::time::Instant::now();
        let img = self.vae.decode(&(latents / LATENT_SCALE)?)?.to_dtype(DType::F32)?;
        let img = ((img + 1.0)? / 2.0)?.clamp(0f32, 1f32)?.flatten_all()?.to_vec1::<f32>()?;
        let n = w * h;
        if img.len() != 3 * n || img.iter().any(|v| !v.is_finite()) {
            return Err(Error::Model("the diffusion model gave no picture (out of range)".into()));
        }
        if profile {
            eprintln!("genfill {w}×{h}: decode {:.2} s", t1.elapsed().as_secs_f64());
        }
        progress(1.0);
        Ok(Rgb32f::from_fn(w, h, |x, y| std::array::from_fn(|c| img.get(c * n + y * w + x).copied().unwrap_or(0.0))))
    }
}

/// The model's picture size for a `w × h` crop: `edge` pixels on the long side, multiples of [`MULTIPLE`].
pub fn model_size(w: usize, h: usize, edge: usize) -> (usize, usize) {
    let long = w.max(h).max(1) as f64;
    let snap = |v: f64| ((v / MULTIPLE as f64).round() as usize).max(2) * MULTIPLE;
    (snap(w as f64 * edge as f64 / long), snap(h as f64 * edge as f64 / long))
}

fn resize(img: &Rgb32f, w: usize, h: usize) -> Rgb32f {
    lightcraft_raster::resample::resize(img, w.max(1), h.max(1), lightcraft_raster::resample::Filter::Mitchell)
}

/// `hole` (`w × h`) at `nw × nh`: a pixel is in the hole when any of what it covers is.
pub fn resize_mask(hole: &[bool], w: usize, h: usize, nw: usize, nh: usize) -> Vec<bool> {
    (0..nw * nh)
        .map(|i| {
            let (x, y) = (i % nw, i / nw);
            let (x0, x1) = (x * w / nw, ((x + 1) * w).div_ceil(nw).min(w).max(x * w / nw + 1));
            let (y0, y1) = (y * h / nh, ((y + 1) * h).div_ceil(nh).min(h).max(y * h / nh + 1));
            (y0..y1).any(|yy| (x0..x1).any(|xx| hole.get(yy * w + xx).copied().unwrap_or(false)))
        })
        .collect()
}

/// `img` with `fill` inside `hole`.
fn compose(img: &Rgb32f, fill: &Rgb32f, hole: &[bool]) -> Rgb32f {
    let mut o = img.clone();
    for ((d, s), m) in o.data.iter_mut().zip(&fill.data).zip(hole) {
        if *m {
            *d = *s;
        }
    }
    o
}

/// Tiles of about `tile` pixels (overlapping by a quarter) over the part of a `w × h` picture `hole` touches.
pub fn tiles_over(hole: &[bool], w: usize, h: usize, tile: usize) -> Vec<(usize, usize, usize, usize)> {
    let mut b = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, _) in hole.iter().enumerate().filter(|(_, m)| **m) {
        let (x, y) = (i % w.max(1), i / w.max(1));
        b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
    }
    if b.0 >= b.2 || b.1 >= b.3 {
        return Vec::new();
    }
    let span = |lo: usize, hi: usize, len: usize| -> Vec<(usize, usize)> {
        let t = tile.min(len).max(1);
        let lo = lo.saturating_sub(t / 8);
        let hi = (hi + t / 8).min(len);
        let n = (hi - lo).saturating_sub(t).div_ceil((t * 3 / 4).max(1)) + 1;
        (0..n)
            .map(|k| {
                let s = if n == 1 { lo.min(len - t) } else { lo + (hi - lo - t.min(hi - lo)) * k / (n - 1) };
                (s.min(len - t), t)
            })
            .collect()
    };
    let (xs, ys) = (span(b.0, b.2, w), span(b.1, b.3, h));
    ys.iter().flat_map(|&(y, th)| xs.iter().map(move |&(x, tw)| (x, y, tw, th))).collect()
}

/// Join the fill to `img` without a seam: `gen` is the model's whole picture (its copy of the photo around the
/// hole too); the difference between the photo and it on the ring just outside `hole` is spread smoothly over the
/// hole (pull–push interpolation, a fast stand-in for solving Laplace's equation, as in Poisson cloning) and added
/// to the fill. Outside the hole the result is `img`.
pub fn seamless(img: &Rgb32f, made: &Rgb32f, hole: &[bool]) -> Rgb32f {
    let (w, h) = (img.width, img.height);
    let in_hole = |x: usize, y: usize| hole.get(y * w + x).copied().unwrap_or(false);
    // the ring: pixels outside the hole within 3 pixels of it
    let near = |x: usize, y: usize| -> bool {
        let (x0, x1, y0, y1) = (x.saturating_sub(3), (x + 4).min(w), y.saturating_sub(3), (y + 4).min(h));
        (y0..y1).any(|yy| (x0..x1).any(|xx| in_hole(xx, yy)))
    };
    let diff: Vec<Option<[f32; 3]>> = (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            if in_hole(x, y) || !near(x, y) {
                return None;
            }
            let (a, b) = (img.data.get(i)?, made.data.get(i)?);
            Some(std::array::from_fn(|c| a[c] - b[c]))
        })
        .collect();
    let field = pull_push(&diff, w, h);
    let mut o = img.clone();
    for (i, d) in o.data.iter_mut().enumerate() {
        if hole.get(i).copied().unwrap_or(false)
            && let (Some(s), Some(f)) = (made.data.get(i), field.get(i))
        {
            *d = std::array::from_fn(|c| (s[c] + f[c]).clamp(0.0, 1.0));
        }
    }
    o
}

/// Fill the unknown values of a field by pull–push: average down a pyramid of the known ones, then fill each level's
/// unknowns from the coarser level.
pub fn pull_push(known: &[Option<[f32; 3]>], w: usize, h: usize) -> Vec<[f32; 3]> {
    // (sum of values, weight) per pixel
    let mut levels: Vec<(Vec<([f32; 3], f32)>, usize, usize)> =
        vec![(known.iter().map(|k| k.map_or(([0.0; 3], 0.0), |v| (v, 1.0))).collect(), w.max(1), h.max(1))];
    while let Some((cur, cw, ch)) = levels.last() {
        if *cw <= 1 && *ch <= 1 {
            break;
        }
        let (nw, nh) = (cw.div_ceil(2), ch.div_ceil(2));
        let mut next = vec![([0f32; 3], 0f32); nw * nh];
        for y in 0..*ch {
            for x in 0..*cw {
                if let (Some(&(v, wt)), Some(n)) = (cur.get(y * cw + x), next.get_mut((y / 2) * nw + x / 2)) {
                    for c in 0..3 {
                        n.0[c] += v[c] * wt;
                    }
                    n.1 += wt;
                }
            }
        }
        for n in &mut next {
            if n.1 > 0.0 {
                n.0 = n.0.map(|v| v / n.1);
                n.1 = n.1.min(1.0);
            }
        }
        levels.push((next, nw, nh));
    }
    // push: from the coarsest level down, unknowns take the (bilinear) coarser value
    let mut coarse: Vec<[f32; 3]> = levels.last().map(|(l, _, _)| l.iter().map(|(v, _)| *v).collect()).unwrap_or_default();
    let mut cdim = levels.last().map_or((1, 1), |(_, w, h)| (*w, *h));
    for (lvl, lw, lh) in levels.iter().rev().skip(1) {
        let mut cur = vec![[0f32; 3]; lw * lh];
        for y in 0..*lh {
            for x in 0..*lw {
                let i = y * lw + x;
                let (v, wt) = lvl.get(i).copied().unwrap_or(([0.0; 3], 0.0));
                // the coarser level at this pixel's centre
                let (fx, fy) = (((x as f32 + 0.5) / 2.0 - 0.5).max(0.0), ((y as f32 + 0.5) / 2.0 - 0.5).max(0.0));
                let (x0, y0) = ((fx as usize).min(cdim.0 - 1), (fy as usize).min(cdim.1 - 1));
                let (x1, y1) = ((x0 + 1).min(cdim.0 - 1), (y0 + 1).min(cdim.1 - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                let g = |xx: usize, yy: usize| coarse.get(yy * cdim.0 + xx).copied().unwrap_or([0.0; 3]);
                let up: [f32; 3] = std::array::from_fn(|c| {
                    let top = g(x0, y0)[c] * (1.0 - tx) + g(x1, y0)[c] * tx;
                    let bot = g(x0, y1)[c] * (1.0 - tx) + g(x1, y1)[c] * tx;
                    top * (1.0 - ty) + bot * ty
                });
                if let Some(o) = cur.get_mut(i) {
                    *o = std::array::from_fn(|c| v[c] * wt + up[c] * (1.0 - wt));
                }
            }
        }
        coarse = cur;
        cdim = (*lw, *lh);
    }
    coarse
}

/// Give the fill inside `hole` the fine grain of the photo around it (a resampled fill is smoother than the photo):
/// deterministic noise from `seed`, scaled to make up the difference in high-frequency energy.
pub fn match_grain(img: &Rgb32f, out: &mut Rgb32f, hole: &[bool], seed: u64) {
    let (w, h) = (img.width, img.height);
    let hf = |im: &Rgb32f, inside: bool| -> f32 {
        let (mut s, mut n) = (0f64, 0f64);
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let i = y * w + x;
                if hole.get(i).copied().unwrap_or(false) != inside {
                    continue;
                }
                let lum = |j: usize| im.data.get(j).map_or(0.0, |p| (p[0] + p[1] + p[2]) / 3.0);
                let lap = 4.0 * lum(i) - lum(i - 1) - lum(i + 1) - lum(i - w) - lum(i + w);
                s += f64::from(lap * lap);
                n += 1.0;
            }
        }
        if n > 0.0 { (s / n).sqrt() as f32 } else { 0.0 }
    };
    let (around, inside) = (hf(img, false), hf(out, true));
    if around <= inside || around <= 0.0 {
        return;
    }
    // white noise of std σ has a Laplacian of std √20·σ
    let sigma = (around * around - inside * inside).sqrt() / 20f32.sqrt();
    let noise = gaussian(seed ^ 0x6772_6169_6e00_0000, w * h);
    for (i, d) in out.data.iter_mut().enumerate() {
        if hole.get(i).copied().unwrap_or(false)
            && let Some(n) = noise.get(i)
        {
            *d = d.map(|v| (v + n * sigma).clamp(0.0, 1.0));
        }
    }
}

/// `n` standard normal numbers from `seed` (SplitMix64 and Box–Muller: the same on every machine).
pub fn gaussian(seed: u64, n: usize) -> Vec<f32> {
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let mut v = Vec::with_capacity(n + 1);
    while v.len() < n {
        let (u1, u2) = (next().max(1e-300), next());
        let r = (-2.0 * u1.ln()).sqrt();
        let a = std::f64::consts::TAU * u2;
        v.push((r * a.cos()) as f32);
        v.push((r * a.sin()) as f32);
    }
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_the_same_for_a_seed_and_normal() {
        let a = gaussian(7, 10_001);
        assert_eq!(a, gaussian(7, 10_001));
        assert_ne!(a, gaussian(8, 10_001));
        let mean = a.iter().map(|v| f64::from(*v)).sum::<f64>() / a.len() as f64;
        let var = a.iter().map(|v| (f64::from(*v) - mean).powi(2)).sum::<f64>() / a.len() as f64;
        assert!(mean.abs() < 0.05 && (var - 1.0).abs() < 0.05, "{mean} {var}");
    }

    #[test]
    fn model_sizes_are_multiples_and_keep_the_aspect() {
        assert_eq!(model_size(1000, 500, 512), (512, 256));
        assert_eq!(model_size(300, 300, 512), (512, 512));
        let (w, h) = model_size(7, 3000, 512);
        assert!(w % MULTIPLE == 0 && h == 512 && w >= 2 * MULTIPLE);
    }

    #[test]
    fn tiles_cover_the_hole() {
        let (w, h) = (1500, 900);
        let hole: Vec<bool> = (0..w * h).map(|i| (300..1200).contains(&(i % w)) && (100..800).contains(&(i / w))).collect();
        let t = tiles_over(&hole, w, h, 512);
        assert!(t.len() >= 4 && t.len() <= 9, "{t:?}");
        for (i, _) in hole.iter().enumerate().filter(|(_, m)| **m).step_by(997) {
            let (x, y) = (i % w, i / w);
            assert!(t.iter().any(|&(x0, y0, tw, th)| (x0..x0 + tw).contains(&x) && (y0..y0 + th).contains(&y)), "{x},{y}");
        }
        for &(x0, y0, tw, th) in &t {
            assert!(x0 + tw <= w && y0 + th <= h);
        }
        assert!(tiles_over(&[false; 100], 10, 10, 512).is_empty());
        assert_eq!(tiles_over(&[true; 4], 2, 2, 512), vec![(0, 0, 2, 2)]);
    }

    #[test]
    fn seamless_removes_a_colour_step_at_the_edge() {
        // the photo is a gradient; the fill is right in shape but 0.2 too bright
        let (w, h) = (64, 48);
        let img = Rgb32f::from_fn(w, h, |x, _| [x as f32 / 128.0 + 0.2; 3]);
        let hole: Vec<bool> = (0..w * h).map(|i| (20..44).contains(&(i % w)) && (12..36).contains(&(i / w))).collect();
        let fill = Rgb32f::from_fn(w, h, |x, _| [x as f32 / 128.0 + 0.4; 3]);
        let out = seamless(&img, &fill, &hole);
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (out.get(x, y)[0], img.get(x, y)[0]);
                assert!((a - b).abs() < 0.02, "{x},{y}: {a} vs {b}");
            }
        }
        // outside the hole: untouched
        assert_eq!(out.get(2, 2), img.get(2, 2));
    }

    #[test]
    fn pull_push_fills_unknowns_and_keeps_knowns() {
        let (w, h) = (9, 7);
        let known: Vec<Option<[f32; 3]>> = (0..w * h).map(|i| (i % w == 0).then_some([1.0, 2.0, 3.0])).collect();
        let f = pull_push(&known, w, h);
        assert_eq!(f.len(), w * h);
        assert!(f.iter().all(|v| (v[0] - 1.0).abs() < 1e-4 && (v[2] - 3.0).abs() < 1e-4));
        // nothing known: zeros, never NaN
        assert!(pull_push(&[None; 12], 4, 3).iter().all(|v| v.iter().all(|c| *c == 0.0)));
        assert!(pull_push(&[], 0, 0).len() <= 1);
    }

    #[test]
    fn grain_is_added_only_where_the_fill_is_smoother() {
        let (w, h) = (48, 48);
        let noise = gaussian(3, w * h);
        let img = Rgb32f::from_fn(w, h, |x, y| [0.5 + 0.05 * noise[y * w + x]; 3]);
        let hole: Vec<bool> = (0..w * h).map(|i| (16..32).contains(&(i % w)) && (16..32).contains(&(i / w))).collect();
        let mut out = compose(&img, &Rgb32f::filled(w, h, [0.5; 3]), &hole);
        let before = out.clone();
        match_grain(&img, &mut out, &hole, 1);
        assert_ne!(out.get(24, 24), before.get(24, 24));
        assert_eq!(out.get(2, 2), before.get(2, 2));
        let mut again = before.clone();
        match_grain(&img, &mut again, &hole, 1);
        assert_eq!(again.data, out.data);
    }

    /// The pipeline on a network of SD's shape but tiny and with zero weights (no download): shapes, the hole,
    /// tiles, determinism and cancelling, on the CPU.
    fn tiny() -> GenFiller {
        let d = Device::Cpu;
        let vb = VarBuilder::zeros(DType::F32, &d);
        let bc = |out_channels, use_cross_attn| sd::unet_2d::BlockConfig { out_channels, use_cross_attn, attention_head_dim: 8 };
        let unet_cfg = sd::unet_2d::UNet2DConditionModelConfig {
            blocks: vec![bc(32, Some(1)), bc(32, Some(1)), bc(64, Some(1)), bc(64, None)],
            layers_per_block: 1,
            cross_attention_dim: 16,
            ..sd::unet_config(None)
        };
        let vae_cfg = sd::vae::AutoEncoderKLConfig { block_out_channels: vec![32, 32, 32, 32], layers_per_block: 1, ..sd::vae_config() };
        GenFiller {
            unet: sd::unet_2d::UNet2DConditionModel::new(vb.clone(), 9, 4, false, unet_cfg).unwrap(),
            vae: sd::vae::AutoEncoderKL::new(vb, 3, 3, vae_cfg).unwrap(),
            context: Tensor::zeros((2, CONTEXT, 16), DType::F32, &d).unwrap(),
            device: d,
            dtype: DType::F32,
        }
    }

    #[test]
    fn the_pipeline_fills_only_the_hole_repeatably_and_stops_when_asked() {
        let m = tiny();
        let (w, h) = (150, 100);
        let img = Rgb32f::from_fn(w, h, |x, y| [x as f32 / w as f32, y as f32 / h as f32, 0.3]);
        let hole: Vec<bool> = (0..w * h).map(|i| (60..90).contains(&(i % w)) && (30..70).contains(&(i / w))).collect();
        let init = compose(&img, &Rgb32f::filled(w, h, [0.5; 3]), &hole);
        let p = Params { steps: 4, edge: 256, guidance: 2.0, ..Params::default() };
        let a = m.fill(&img, &hole, Some(&init), &p, &|_| true).unwrap();
        assert_eq!((a.width, a.height), (w, h));
        assert!(a.data.iter().flatten().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        for (i, (x, y)) in a.data.iter().zip(&img.data).enumerate() {
            if !hole[i] {
                assert_eq!(x, y, "outside the hole the picture is kept");
            }
        }
        assert_eq!(m.fill(&img, &hole, Some(&init), &p, &|_| true).unwrap().data, a.data, "a seed repeats");
        // from noise, without guidance, and a crop large enough for tiles
        let big = resize(&img, 600, 400);
        let bhole = resize_mask(&hole, w, h, 600, 400);
        let q = Params { steps: 2, edge: 256, guidance: 1.0, refine_edge: 512, ..Params::default() };
        let seen = std::sync::Mutex::new(Vec::new());
        let b = m
            .fill(&big, &bhole, None, &q, &|f| {
                seen.lock().unwrap().push(f);
                true
            })
            .unwrap();
        assert_eq!((b.width, b.height), (600, 400));
        let seen = seen.into_inner().unwrap();
        assert!(seen.iter().any(|f| *f > 0.6), "the tiles report progress past the first pass: {seen:?}");
        // cancelled: an error, no picture
        assert!(matches!(m.fill(&img, &hole, None, &p, &|_| false), Err(Error::Cancelled)));
        // hostile input: errors, not panics
        assert!(m.fill(&img, &hole[1..], None, &p, &|_| true).is_err());
        assert!(m.fill(&Rgb32f::filled(8, 8, [0.0; 3]), &[true; 64], None, &p, &|_| true).is_err());
        assert_eq!(m.fill(&img, &vec![false; w * h], None, &p, &|_| true).unwrap().data, img.data);
    }

    #[test]
    fn a_folder_without_the_model_is_an_error() {
        let dir = std::env::temp_dir().join(format!("lc-genfill-none-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(TEXT), b"not safetensors").unwrap();
        assert!(GenFiller::load(&dir, Device::Cpu).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opt-in, with the real model: `LC_GENFILL=<folder with unet/vae/text_encoder.safetensors> [LC_LAMA=<big-lama>]
    /// [LC_GENFILL_IMG=<a PNG>] [LC_GENFILL_OUT=<folder>] cargo test -p lightcraft-enhance --release -- --ignored
    /// genfill_speed --nocapture`. Times a variation on a 512 and a 1024 crop and checks a seed repeats exactly.
    fn save_png(path: &str, a: &Rgb32f) {
        let f = std::fs::File::create(path).unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), a.width as u32, a.height as u32);
        e.set_color(png::ColorType::Rgb);
        let bytes: Vec<u8> = a.data.iter().flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        e.write_header().unwrap().write_image_data(&bytes).unwrap();
    }

    #[test]
    #[ignore]
    fn genfill_speed() {
        let Ok(dir) = std::env::var("LC_GENFILL") else { return };
        let d = crate::best_device();
        let t = std::time::Instant::now();
        let m = GenFiller::load(Path::new(&dir), d.clone()).unwrap();
        eprintln!("load {:.1} s on {d:?}", t.elapsed().as_secs_f64());
        let lama = std::env::var("LC_LAMA").ok().map(|p| crate::lama::Inpainter::load(Path::new(&p), d.clone()).unwrap());
        let photo = std::env::var("LC_GENFILL_IMG").ok().map(|p| {
            let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            let ch = info.color_type.samples();
            Rgb32f::from_fn(info.width as usize, info.height as usize, |x, y| {
                let i = (y * info.width as usize + x) * ch;
                [buf[i] as f32 / 255.0, buf[i + 1] as f32 / 255.0, buf[i + 2] as f32 / 255.0]
            })
        });
        let out = std::env::var("LC_GENFILL_OUT").ok();
        let sides: Vec<usize> = std::env::var("LC_GENFILL_SIDES").unwrap_or("512,1024,2048".into()).split(',').map(|v| v.parse().unwrap()).collect();
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
        for side in sides {
            // a photo: the crop around LC_GENFILL_HOLE (x0,y0,x1,y1, normalized) with the hole's size of room
            let rect: Vec<f64> =
                std::env::var("LC_GENFILL_HOLE").unwrap_or("0.36,0.25,0.64,0.75".into()).split(',').map(|v| v.parse().unwrap()).collect();
            let img = match &photo {
                Some(p) => {
                    let (pw, ph) = (p.width as f64, p.height as f64);
                    let (hw, hh) = ((rect[2] - rect[0]) * pw, (rect[3] - rect[1]) * ph);
                    let x0 = ((rect[0] * pw - hw).max(0.0)) as usize;
                    let y0 = ((rect[1] * ph - hh).max(0.0)) as usize;
                    let x1 = ((rect[2] * pw + hw).min(pw)) as usize;
                    let y1 = ((rect[3] * ph + hh).min(ph)) as usize;
                    let c = p.crop(x0, y0, x1 - x0, y1 - y0);
                    let long = c.width.max(c.height);
                    resize(&c, c.width * side / long, c.height * side / long)
                }
                None => Rgb32f::from_fn(side, side, |x, y| {
                    let n = ((x * 31 + y * 17) % 13) as f32 / 60.0;
                    if y < side / 2 { [0.5 + n, 0.6 + n, 0.9] } else { [0.3 + n, 0.5, 0.2 + n] }
                }),
            };
            let (w, h) = (img.width, img.height);
            let hole: Vec<bool> = if let Some(p) = &photo {
                // the hole inside the crop
                let (pw, ph) = (p.width as f64, p.height as f64);
                let (hw, hh) = ((rect[2] - rect[0]) * pw, (rect[3] - rect[1]) * ph);
                let (x0, y0) = ((rect[0] * pw - hw).max(0.0), (rect[1] * ph - hh).max(0.0));
                let (cw, ch) = (((rect[2] * pw + hw).min(pw)) - x0, ((rect[3] * ph + hh).min(ph)) - y0);
                (0..w * h)
                    .map(|i| {
                        let (x, y) = (x0 + (i % w) as f64 / w as f64 * cw, y0 + (i / w) as f64 / h as f64 * ch);
                        x >= rect[0] * pw && x < rect[2] * pw && y >= rect[1] * ph && y < rect[3] * ph
                    })
                    .collect()
            } else {
                (0..w * h).map(|i| (i % w).abs_diff(w / 2) < w / 7 && (i / w).abs_diff(h / 2) < h / 4).collect()
            };
            let t = std::time::Instant::now();
            let init = lama.as_ref().map(|l| l.inpaint(&img, &hole).unwrap());
            eprintln!("{side}: LaMa {:.1} s", t.elapsed().as_secs_f64());
            if let (Some(o), Some(i)) = (&out, &init) {
                save_png(&format!("{o}/lama-{side}.png"), i);
                save_png(&format!("{o}/input-{side}.png"), &img);
            }
            for seed in [1u64, 2] {
                let d = Params::default();
                let p = Params {
                    seed,
                    steps: env("LC_STEPS").map_or(d.steps, |v| v as usize),
                    strength: env("LC_STRENGTH").unwrap_or(d.strength),
                    guidance: env("LC_GUIDANCE").unwrap_or(d.guidance),
                    ..d
                };
                let t = std::time::Instant::now();
                let a = m.fill(&img, &hole, init.as_ref(), &p, &|_| true).unwrap();
                eprintln!("{w}×{h} seed {seed}: {:.1} s", t.elapsed().as_secs_f64());
                if let Some(o) = &out {
                    save_png(&format!("{o}/genfill-{side}-{seed}.png"), &a);
                }
                if side == 512 && seed == 1 {
                    let b = m.fill(&img, &hole, init.as_ref(), &p, &|_| true).unwrap();
                    assert_eq!(a.data, b.data, "the same seed must give the same fill");
                }
            }
        }
    }

    #[test]
    fn prompts_fit_the_context() {
        assert!(PROMPT.len() + 2 <= CONTEXT && NEGATIVE.len() + 2 <= CONTEXT);
        let t = tokens(PROMPT);
        assert_eq!((t.len(), t[0], t[t.len() - 1]), (CONTEXT, BOS, EOS));
    }
}
