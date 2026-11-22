//! Generative Remove: LaMa, "Resolution-robust Large Mask Inpainting with Fourier Convolutions" (Suvorov et al.,
//! WACV 2022; Samsung Research, Apache-2.0), the `big-lama` generator, written here on candle from the published
//! architecture (`saicinpainting/training/modules/ffc.py` in advimman/lama) and read from its safetensors copy.
//!
//! The network: a 7 × 7 convolution, three stride-2 downsamplings, 18 Fast Fourier Convolution residual blocks at
//! 1/8 of the size (a quarter of the channels local 3 × 3 convolutions, three quarters "global": a 1 × 1
//! convolution in the 2-D Fourier domain, so every block sees the whole picture), three transposed-convolution
//! upsamplings and a 7 × 7 convolution with a sigmoid. Batch norms are folded into a scale and shift.
//!
//! candle has no FFT: the real 2-D FFT (`torch.fft.rfftn(norm="ortho")`) and its inverse are matrix products with
//! cosine and sine tables, which on the bottleneck's small sizes (64² for a 512-pixel picture) cost next to nothing
//! on the GPU. Convolutions go through [`crate::netrun::conv2d`] (right on the CPU too); stride-2 and transposed
//! convolutions are split into their four phases, so no work is spent on skipped or inserted zeros.

use std::path::Path;

use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use lightcraft_raster::Rgb32f;

use crate::{Error, Result};

const BLOCKS: usize = 18;
/// Local and global channels in the bottleneck (512 · (1 − 0.75), 512 · 0.75).
const LOCAL: usize = 128;
const GLOBAL: usize = 384;
const EPS: f64 = 1e-5;

/// A batch norm in inference: `x · scale + shift`, per channel (`[1, C, 1, 1]`).
struct Bn {
    scale: Tensor,
    shift: Tensor,
}

impl Bn {
    fn load(vb: &VarBuilder, c: usize) -> candle_core::Result<Self> {
        let (g, b) = (vb.get(c, "weight")?, vb.get(c, "bias")?);
        let (m, v) = (vb.get(c, "running_mean")?, vb.get(c, "running_var")?);
        let scale = g.div(&(v + EPS)?.sqrt()?)?;
        let shift = (b - m.mul(&scale)?)?;
        Ok(Bn { scale: scale.reshape((1, c, 1, 1))?, shift: shift.reshape((1, c, 1, 1))? })
    }
    fn relu(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        x.broadcast_mul(&self.scale)?.broadcast_add(&self.shift)?.relu()
    }
}

/// A 1 × 1 convolution as `[cout, cin]`.
fn mat(vb: &VarBuilder, cout: usize, cin: usize, name: &str) -> candle_core::Result<Tensor> {
    vb.get((cout, cin, 1, 1), name)?.reshape((cout, cin))
}

fn conv1x1(x: &Tensor, w: &Tensor) -> candle_core::Result<Tensor> {
    let (n, cin, h, wd) = x.dims4()?;
    if n != 1 {
        candle_core::bail!("one picture at a time");
    }
    let cout = w.dim(0)?;
    w.matmul(&x.contiguous()?.reshape((cin, h * wd))?)?.reshape((1, cout, h, wd))
}

/// Mirror `x` (`[1, C, H, W]`) by `p` pixels on every side (PyTorch's `reflect`: the edge pixel isn't repeated).
pub fn reflect_pad(x: &Tensor, p: usize) -> candle_core::Result<Tensor> {
    if p == 0 {
        return Ok(x.clone());
    }
    let (_, _, h, w) = x.dims4()?;
    if p >= h || p >= w {
        candle_core::bail!("the picture is too small to mirror by {p}");
    }
    let idx = |n: usize| -> Vec<u32> {
        (0..n + 2 * p)
            .map(|i| {
                let j = i as i64 - p as i64;
                let j = if j < 0 { -j } else if j >= n as i64 { 2 * (n as i64 - 1) - j } else { j };
                j.clamp(0, n as i64 - 1) as u32
            })
            .collect()
    };
    let (iy, ix) = (Tensor::new(idx(h), x.device())?, Tensor::new(idx(w), x.device())?);
    x.contiguous()?.index_select(&iy, 2)?.index_select(&ix, 3)
}

/// A 3 × 3, stride-2 convolution with mirrored padding 1 (`H`, `W` even): each kernel tap reads one of the four
/// phases (even / odd rows and columns) of the padded picture, as a flat slice like [`crate::netrun::conv2d`].
pub fn conv3_s2(x: &Tensor, w: &Tensor) -> candle_core::Result<Tensor> {
    let (n, cin, h, wd) = x.dims4()?;
    let (cout, cin_k, kh, kw) = w.dims4()?;
    if n != 1 || cin != cin_k || (kh, kw) != (3, 3) || h % 2 != 0 || wd % 2 != 0 {
        candle_core::bail!("stride-2 convolution shapes do not match");
    }
    let xp = reflect_pad(x, 1)?;
    let (hp, wp) = (h / 2 + 1, wd / 2 + 1);
    let (oh, ow) = (h / 2, wd / 2);
    // [cin, hp, 2, wp, 2]
    let xp = xp.reshape((cin, hp, 2, wp, 2))?;
    let taps = w.permute((2, 3, 1, 0))?.contiguous()?;
    let mut acc: Option<Tensor> = None;
    for a in 0..2 {
        for b in 0..2 {
            // the phase, with one more zero row so every tap's slice stays inside
            let ph = xp.i((.., .., a, .., b))?.pad_with_zeros(1, 0, 1)?.reshape((cin, (hp + 1) * wp))?.t()?.contiguous()?;
            for ky in (a..3).step_by(2) {
                for kx in (b..3).step_by(2) {
                    let y = ph.narrow(0, (ky / 2) * wp + kx / 2, oh * wp)?.matmul(&taps.get(ky)?.get(kx)?)?;
                    acc = Some(match acc {
                        Some(s) => (s + y)?,
                        None => y,
                    });
                }
            }
        }
    }
    let acc = acc.ok_or_else(|| candle_core::Error::Msg("empty kernel".into()))?;
    acc.reshape((oh, wp, cout))?.narrow(1, 0, ow)?.permute((2, 0, 1))?.contiguous()?.reshape((1, cout, oh, ow))
}

/// `ConvTranspose2d(kernel 3, stride 2, padding 1, output_padding 1)`, weights `[cin, cout, 3, 3]`: output row
/// `2m` takes tap 1 of input row `m`; row `2m + 1` taps 0 of row `m + 1` and 2 of row `m` (columns alike).
pub fn conv_t3_s2(x: &Tensor, w: &Tensor, bias: &Tensor) -> candle_core::Result<Tensor> {
    let (n, cin, h, wd) = x.dims4()?;
    let (cin_k, cout, kh, kw) = w.dims4()?;
    if n != 1 || cin != cin_k || (kh, kw) != (3, 3) {
        candle_core::bail!("transposed convolution shapes do not match");
    }
    let wp = wd + 1;
    // zero row and column after the picture (+ one more row for the slices)
    let flat = x.pad_with_zeros(2, 0, 2)?.pad_with_zeros(3, 0, 1)?.reshape((cin, (h + 2) * wp))?.t()?.contiguous()?;
    // [ky][kx][cin][cout]
    let taps = w.permute((2, 3, 0, 1))?.contiguous()?;
    let phase_taps = |a: usize| -> &'static [(usize, usize)] { if a == 0 { &[(1, 0)] } else { &[(0, 1), (2, 0)] } };
    let mut rows = Vec::with_capacity(2);
    for a in 0..2 {
        let mut cols = Vec::with_capacity(2);
        for b in 0..2 {
            let mut acc: Option<Tensor> = None;
            for &(ky, dy) in phase_taps(a) {
                for &(kx, dx) in phase_taps(b) {
                    let y = flat.narrow(0, dy * wp + dx, h * wp)?.matmul(&taps.get(ky)?.get(kx)?)?;
                    acc = Some(match acc {
                        Some(s) => (s + y)?,
                        None => y,
                    });
                }
            }
            let acc = acc.ok_or_else(|| candle_core::Error::Msg("empty kernel".into()))?;
            // [h·wp, cout] → [cout, h, wd]
            cols.push(acc.reshape((h, wp, cout))?.narrow(1, 0, wd)?.permute((2, 0, 1))?);
        }
        rows.push(Tensor::stack(&cols, 3)?);
    }
    // [cout, h, 2, wd, 2] → [1, cout, 2h, 2wd]
    Tensor::stack(&rows, 2)?.reshape((1, cout, 2 * h, 2 * wd))?.broadcast_add(&bias.reshape((1, cout, 1, 1))?)
}

/// The cosine and sine tables of the real 2-D DFT of an `h × w` picture (`w' = w / 2 + 1` frequencies across).
pub struct Dft {
    h: usize,
    w: usize,
    /// `[h, h]`: cos / sin(2π k y / h).
    ch: Tensor,
    sh: Tensor,
    /// `[w, w']`: cos(2π l x / w), −sin(2π l x / w).
    cw: Tensor,
    sw: Tensor,
    /// `[w', w]`: the inverse's c2r weights: `a_l` cos / sin(2π l x / w), `a_l` = 1 at 0 and Nyquist, else 2.
    icw: Tensor,
    isw: Tensor,
    norm: f64,
}

impl Dft {
    pub fn new(h: usize, w: usize, device: &Device) -> candle_core::Result<Self> {
        let wf = w / 2 + 1;
        let tau = std::f64::consts::TAU;
        let table = |rows: usize, cols: usize, f: &dyn Fn(usize, usize) -> f64| -> candle_core::Result<Tensor> {
            let v: Vec<f32> = (0..rows * cols).map(|i| f(i / cols, i % cols) as f32).collect();
            Tensor::from_vec(v, (rows, cols), device)
        };
        // reduce the products mod n so the angles stay small and exact
        let ang = |a: usize, b: usize, n: usize| tau * ((a * b) % n) as f64 / n as f64;
        let wt = |l: usize| if l == 0 || (w % 2 == 0 && l == w / 2) { 1.0 } else { 2.0 };
        Ok(Dft {
            h,
            w,
            ch: table(h, h, &|k, y| ang(k, y, h).cos())?,
            sh: table(h, h, &|k, y| ang(k, y, h).sin())?,
            cw: table(w, wf, &|x, l| ang(l, x, w).cos())?,
            sw: table(w, wf, &|x, l| -ang(l, x, w).sin())?,
            icw: table(wf, w, &|l, x| wt(l) * ang(l, x, w).cos())?,
            isw: table(wf, w, &|l, x| wt(l) * ang(l, x, w).sin())?,
            norm: 1.0 / ((h * w) as f64).sqrt(),
        })
    }

    /// `rfftn(x, dim=(-2, -1), norm="ortho")` of `[C, h, w]` → (real, imaginary), each `[C, h, w']`.
    pub fn forward(&self, x: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
        let (c, h, w) = x.dims3()?;
        if (h, w) != (self.h, self.w) {
            candle_core::bail!("DFT table size");
        }
        let flat = x.contiguous()?.reshape((c * h, w))?;
        let wf = self.cw.dim(1)?;
        let (are, aim) = (flat.matmul(&self.cw)?.reshape((c, h, wf))?, flat.matmul(&self.sw)?.reshape((c, h, wf))?);
        // along the height: (a_re + i a_im)(cos − i sin)
        let re = (self.ch.broadcast_matmul(&are)? + self.sh.broadcast_matmul(&aim)?)?;
        let im = (self.ch.broadcast_matmul(&aim)? - self.sh.broadcast_matmul(&are)?)?;
        Ok(((re * self.norm)?, (im * self.norm)?))
    }

    /// `irfftn(X, s=(h, w), dim=(-2, -1), norm="ortho")` of (real, imaginary) `[C, h, w']` → `[C, h, w]`.
    pub fn inverse(&self, re: &Tensor, im: &Tensor) -> candle_core::Result<Tensor> {
        let (c, h, wf) = re.dims3()?;
        // along the height: (re + i im)(cos + i sin)
        let bre = (self.ch.broadcast_matmul(re)? - self.sh.broadcast_matmul(im)?)?.reshape((c * h, wf))?;
        let bim = (self.sh.broadcast_matmul(re)? + self.ch.broadcast_matmul(im)?)?.reshape((c * h, wf))?;
        let x = (bre.matmul(&self.icw)? - bim.matmul(&self.isw)?)?;
        x.reshape((c, h, self.w))? * self.norm
    }
}

/// The global → global path: 1 × 1 down to half the channels, the Fourier unit, 1 × 1 back.
struct Spectral {
    conv1: Tensor,
    bn1: Bn,
    /// `[2c, 2c]` over interleaved (real, imaginary) channels.
    fu: Tensor,
    fu_bn: Bn,
    conv2: Tensor,
}

impl Spectral {
    fn load(vb: &VarBuilder) -> candle_core::Result<Self> {
        let half = GLOBAL / 2;
        Ok(Spectral {
            conv1: mat(&vb.pp("conv1.0"), half, GLOBAL, "weight")?,
            bn1: Bn::load(&vb.pp("conv1.1"), half)?,
            fu: mat(&vb.pp("fu.conv_layer"), 2 * half, 2 * half, "weight")?,
            fu_bn: Bn::load(&vb.pp("fu.bn"), 2 * half)?,
            conv2: mat(&vb.pp("conv2"), GLOBAL, half, "weight")?,
        })
    }

    fn forward(&self, x: &Tensor, dft: &Dft) -> candle_core::Result<Tensor> {
        let x = self.bn1.relu(&conv1x1(x, &self.conv1)?)?;
        let (_, c, h, w) = x.dims4()?;
        let (re, im) = dft.forward(&x.reshape((c, h, w))?)?;
        let wf = re.dim(2)?;
        // channel 2c is the real part of c, 2c + 1 its imaginary part
        let f = Tensor::stack(&[re, im], 1)?.reshape((1, 2 * c, h, wf))?;
        let f = self.fu_bn.relu(&conv1x1(&f, &self.fu)?)?.reshape((c, 2, h, wf))?;
        let (re, im) = (f.i((.., 0))?.contiguous()?, f.i((.., 1))?.contiguous()?);
        let fu = dft.inverse(&re, &im)?.reshape((1, c, h, w))?;
        conv1x1(&(x + fu)?, &self.conv2)
    }
}

/// One FFC + batch norms + ReLU of the bottleneck.
struct Ffc {
    /// local → (local, global), one 3 × 3 convolution `[LOCAL + GLOBAL, LOCAL, 3, 3]`.
    from_local: Tensor,
    g2l: Tensor,
    spectral: Spectral,
    bn_l: Bn,
    bn_g: Bn,
}

impl Ffc {
    fn load(vb: &VarBuilder) -> candle_core::Result<Self> {
        let f = vb.pp("ffc");
        let l2l = f.get((LOCAL, LOCAL, 3, 3), "convl2l.weight")?;
        let l2g = f.get((GLOBAL, LOCAL, 3, 3), "convl2g.weight")?;
        Ok(Ffc {
            from_local: Tensor::cat(&[l2l, l2g], 0)?,
            g2l: f.get((LOCAL, GLOBAL, 3, 3), "convg2l.weight")?,
            spectral: Spectral::load(&f.pp("convg2g"))?,
            bn_l: Bn::load(&vb.pp("bn_l"), LOCAL)?,
            bn_g: Bn::load(&vb.pp("bn_g"), GLOBAL)?,
        })
    }

    fn forward(&self, l: &Tensor, g: &Tensor, dft: &Dft) -> candle_core::Result<(Tensor, Tensor)> {
        let fl = crate::netrun::conv2d(&reflect_pad(l, 1)?, &self.from_local, 0)?;
        let gl = crate::netrun::conv2d(&reflect_pad(g, 1)?, &self.g2l, 0)?;
        let yl = (fl.narrow(1, 0, LOCAL)? + gl)?;
        let yg = (fl.narrow(1, LOCAL, GLOBAL)? + self.spectral.forward(g, dft)?)?;
        Ok((self.bn_l.relu(&yl)?, self.bn_g.relu(&yg)?))
    }
}

struct Up {
    w: Tensor,
    b: Tensor,
    bn: Bn,
}

/// The loaded big-lama generator.
pub struct Inpainter {
    init: Tensor,
    init_bn: Bn,
    down: [(Tensor, Bn); 2],
    /// The last downsampling: local and global outputs.
    down_l: Tensor,
    down_g: Tensor,
    down_bn_l: Bn,
    down_bn_g: Bn,
    blocks: Vec<(Ffc, Ffc)>,
    up: Vec<Up>,
    last_w: Tensor,
    last_b: Tensor,
    device: Device,
}

/// The model's input size must be a multiple of this (three halvings).
pub const MULTIPLE: usize = 8;

impl Inpainter {
    /// Load `big-lama.safetensors` (keys `model.generator.model.N…`, as converted from the published checkpoint).
    pub fn load(path: &Path, device: Device) -> Result<Self> {
        let _gpu = crate::gpu_lock(&device);
        let tensors = candle_core::safetensors::load(path, &Device::Cpu).map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        if tensors.len() > 4096 {
            return Err(Error::Model("not a LaMa checkpoint (too many tensors)".into()));
        }
        // the generator's keys, whichever prefix the conversion kept
        let prefix = ["model.generator.model.", "generator.model.", "model."]
            .into_iter()
            .find(|p| tensors.contains_key(&format!("{p}1.ffc.convl2l.weight")))
            .ok_or_else(|| Error::Model(format!("{}: not a big-lama checkpoint", path.display())))?;
        let tensors = tensors.into_iter().filter_map(|(k, v)| k.strip_prefix(prefix).map(|k| (k.to_string(), v))).collect();
        let vb = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let load = || -> candle_core::Result<Self> {
            let down = |i: usize, cin: usize, cout: usize| -> candle_core::Result<(Tensor, Bn)> {
                let v = vb.pp(i.to_string());
                Ok((v.get((cout, cin, 3, 3), "ffc.convl2l.weight")?, Bn::load(&v.pp("bn_l"), cout)?))
            };
            let blocks = (0..BLOCKS)
                .map(|i| {
                    let v = vb.pp((5 + i).to_string());
                    Ok((Ffc::load(&v.pp("conv1"))?, Ffc::load(&v.pp("conv2"))?))
                })
                .collect::<candle_core::Result<Vec<_>>>()?;
            let up = [(24, 512, 256), (27, 256, 128), (30, 128, 64)]
                .into_iter()
                .map(|(i, cin, cout)| {
                    let v = vb.pp(i.to_string());
                    Ok(Up { w: v.get((cin, cout, 3, 3), "weight")?, b: v.get(cout, "bias")?, bn: Bn::load(&vb.pp((i + 1).to_string()), cout)? })
                })
                .collect::<candle_core::Result<Vec<_>>>()?;
            let v4 = vb.pp("4");
            Ok(Inpainter {
                init: vb.get((64, 4, 7, 7), "1.ffc.convl2l.weight")?,
                init_bn: Bn::load(&vb.pp("1.bn_l"), 64)?,
                down: [down(2, 64, 128)?, down(3, 128, 256)?],
                down_l: v4.get((LOCAL, 256, 3, 3), "ffc.convl2l.weight")?,
                down_g: v4.get((GLOBAL, 256, 3, 3), "ffc.convl2g.weight")?,
                down_bn_l: Bn::load(&v4.pp("bn_l"), LOCAL)?,
                down_bn_g: Bn::load(&v4.pp("bn_g"), GLOBAL)?,
                blocks,
                up,
                last_w: vb.get((3, 64, 7, 7), "34.weight")?,
                last_b: vb.get(3, "34.bias")?.reshape((1, 3, 1, 1))?,
                device: device.clone(),
            })
        };
        load().map_err(|e| Error::Model(format!("{}: {e}", path.display())))
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// The network on `[1, 4, H, W]` (masked picture 0..1 and the mask, `H`, `W` multiples of [`MULTIPLE`]) →
    /// `[1, 3, H, W]` (0..1).
    pub fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (_, c, h, w) = x.dims4()?;
        if c != 4 || h % MULTIPLE != 0 || w % MULTIPLE != 0 || h < 2 * MULTIPLE || w < 2 * MULTIPLE {
            candle_core::bail!("LaMa takes a 4-channel picture a multiple of {MULTIPLE} in size, at least {} across", 2 * MULTIPLE);
        }
        let mut t = self.init_bn.relu(&crate::netrun::conv2d(&reflect_pad(x, 3)?, &self.init, 0)?)?;
        for (wt, bn) in &self.down {
            t = bn.relu(&conv3_s2(&t, wt)?)?;
        }
        let mut l = self.down_bn_l.relu(&conv3_s2(&t, &self.down_l)?)?;
        let mut g = self.down_bn_g.relu(&conv3_s2(&t, &self.down_g)?)?;
        let dft = Dft::new(h / MULTIPLE, w / MULTIPLE, x.device())?;
        for (a, b) in &self.blocks {
            let (l1, g1) = a.forward(&l, &g, &dft)?;
            let (l2, g2) = b.forward(&l1, &g1, &dft)?;
            l = (l + l2)?;
            g = (g + g2)?;
        }
        let mut t = Tensor::cat(&[l, g], 1)?;
        for u in &self.up {
            t = u.bn.relu(&conv_t3_s2(&t, &u.w, &u.b)?)?;
        }
        let y = crate::netrun::conv2d(&reflect_pad(&t, 3)?, &self.last_w, 0)?.broadcast_add(&self.last_b)?;
        candle_nn::ops::sigmoid(&y)
    }

    /// Fill the pixels of `img` (display-encoded RGB, 0..1) where `hole` is set; the rest is returned as it was.
    /// Any size of at least 16 × 16; padded (mirrored) to a multiple of [`MULTIPLE`] for the network.
    pub fn inpaint(&self, img: &Rgb32f, hole: &[bool]) -> Result<Rgb32f> {
        let (w, h) = (img.width, img.height);
        if hole.len() != w * h {
            return Err(Error::Input("the hole does not match the picture".into()));
        }
        if w < 2 * MULTIPLE || h < 2 * MULTIPLE {
            return Err(Error::Input("the picture is too small to fill".into()));
        }
        if w.saturating_mul(h) > 4096 * 4096 {
            return Err(Error::Input("the picture is too large to fill in one go".into()));
        }
        let (pw, ph) = (w.div_ceil(MULTIPLE) * MULTIPLE, h.div_ceil(MULTIPLE) * MULTIPLE);
        let n = pw * ph;
        let mut input = vec![0f32; 4 * n];
        // mirrored past the right and bottom edges
        let mirror = |i: usize, len: usize| if i < len { i } else { (2 * len).saturating_sub(i + 2).min(len - 1) };
        for y in 0..ph {
            let sy = mirror(y, h);
            for x in 0..pw {
                let sx = mirror(x, w);
                let i = sy * w + sx;
                let m = hole.get(i).copied().unwrap_or(false);
                let p = img.data.get(i).copied().unwrap_or([0.0; 3]);
                for c in 0..3 {
                    input[c * n + y * pw + x] = if m { 0.0 } else { p[c].clamp(0.0, 1.0) };
                }
                input[3 * n + y * pw + x] = if m { 1.0 } else { 0.0 };
            }
        }
        let out = {
            let _gpu = crate::gpu_lock(&self.device);
            let x = Tensor::from_vec(input, (1, 4, ph, pw), &self.device)?;
            self.forward(&x)?.flatten_all()?.to_vec1::<f32>()?
        };
        let mut res = img.clone();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if hole.get(i).copied().unwrap_or(false) {
                    let v = [0, 1, 2].map(|c| out.get(c * n + y * pw + x).copied().unwrap_or(0.0));
                    if let Some(d) = res.data.get_mut(i) {
                        *d = v;
                    }
                }
            }
        }
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rnd(n: usize, s: u64) -> Vec<f32> {
        (0..n).map(|i| (((i as u64 * 7919 + s * 104_729) % 1000) as f32 / 1000.0) - 0.5).collect()
    }

    fn devices() -> Vec<Device> {
        let mut d = vec![Device::Cpu];
        let best = crate::best_device();
        if !best.is_cpu() {
            d.push(best);
        }
        d
    }

    fn vec4(t: &Tensor) -> Vec<f32> {
        t.flatten_all().unwrap().to_vec1::<f32>().unwrap()
    }

    #[test]
    fn dft_matches_the_definition_and_inverts() {
        for (h, w) in [(8usize, 8usize), (6, 10), (7, 9), (16, 12)] {
            let x = rnd(2 * h * w, 3);
            for d in devices() {
                let _gpu = crate::gpu_lock(&d);
                let dft = Dft::new(h, w, &d).unwrap();
                let xt = Tensor::from_slice(&x, (2, h, w), &d).unwrap();
                let (re, im) = dft.forward(&xt).unwrap();
                let (re, im) = (vec4(&re), vec4(&im));
                let wf = w / 2 + 1;
                let norm = 1.0 / ((h * w) as f64).sqrt();
                let mut worst = 0f64;
                for c in 0..2 {
                    for k in 0..h {
                        for l in 0..wf {
                            let (mut sr, mut si) = (0f64, 0f64);
                            for y in 0..h {
                                for xx in 0..w {
                                    let a = -std::f64::consts::TAU * (k as f64 * y as f64 / h as f64 + l as f64 * xx as f64 / w as f64);
                                    let v = x[(c * h + y) * w + xx] as f64;
                                    sr += v * a.cos();
                                    si += v * a.sin();
                                }
                            }
                            let i = (c * h + k) * wf + l;
                            worst = worst.max((sr * norm - re[i] as f64).abs()).max((si * norm - im[i] as f64).abs());
                        }
                    }
                }
                assert!(worst < 1e-4, "{h}×{w} on {d:?}: {worst}");
                // and back
                let (re, im) = dft.forward(&xt).unwrap();
                let back = vec4(&dft.inverse(&re, &im).unwrap());
                let err = back.iter().zip(&x).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                assert!(err < 1e-4, "{h}×{w} round trip on {d:?}: {err}");
            }
        }
    }

    #[test]
    fn mirrored_padding_does_not_repeat_the_edge() {
        let x = Tensor::from_vec((0..12).map(|v| v as f32).collect::<Vec<_>>(), (1, 1, 3, 4), &Device::Cpu).unwrap();
        let p = reflect_pad(&x, 2).unwrap();
        assert_eq!(p.dims4().unwrap(), (1, 1, 7, 8));
        // row 2 of the padded picture is the original row 0: [2 1 | 0 1 2 3 | 2 1]
        assert_eq!(vec4(&p.i((0, 0, 2)).unwrap()), vec![2., 1., 0., 1., 2., 3., 2., 1.]);
        // row 0 mirrors row 2
        assert_eq!(vec4(&p.i((0, 0, 0)).unwrap()), vec![10., 9., 8., 9., 10., 11., 10., 9.]);
        assert!(reflect_pad(&x, 3).is_err());
    }

    /// Plain-loop references for the strided and transposed convolutions.
    fn naive_conv_s2(x: &[f32], cin: usize, h: usize, w: usize, k: &[f32], cout: usize) -> Vec<f32> {
        let refl = |i: i64, n: usize| -> usize {
            let i = if i < 0 { -i } else if i >= n as i64 { 2 * (n as i64 - 1) - i } else { i };
            i as usize
        };
        let (oh, ow) = (h / 2, w / 2);
        let mut out = vec![0f32; cout * oh * ow];
        for co in 0..cout {
            for oy in 0..oh {
                for ox in 0..ow {
                    let mut s = 0f32;
                    for ci in 0..cin {
                        for ky in 0..3 {
                            for kx in 0..3 {
                                let (yy, xx) = (refl(2 * oy as i64 + ky as i64 - 1, h), refl(2 * ox as i64 + kx as i64 - 1, w));
                                s += x[(ci * h + yy) * w + xx] * k[((co * cin + ci) * 3 + ky) * 3 + kx];
                            }
                        }
                    }
                    out[(co * oh + oy) * ow + ox] = s;
                }
            }
        }
        out
    }

    fn naive_conv_t(x: &[f32], cin: usize, h: usize, w: usize, k: &[f32], cout: usize, b: &[f32]) -> Vec<f32> {
        let (oh, ow) = (2 * h, 2 * w);
        let mut out = vec![0f32; cout * oh * ow];
        for co in 0..cout {
            for v in out[co * oh * ow..(co + 1) * oh * ow].iter_mut() {
                *v = b[co];
            }
        }
        for ci in 0..cin {
            for y in 0..h {
                for xx in 0..w {
                    let v = x[(ci * h + y) * w + xx];
                    for co in 0..cout {
                        for ky in 0..3 {
                            for kx in 0..3 {
                                // out = in · 2 − 1 + k
                                let (oy, ox) = (2 * y as i64 - 1 + ky as i64, 2 * xx as i64 - 1 + kx as i64);
                                if oy >= 0 && ox >= 0 && (oy as usize) < oh && (ox as usize) < ow {
                                    out[(co * oh + oy as usize) * ow + ox as usize] += v * k[((ci * cout + co) * 3 + ky) * 3 + kx];
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn strided_and_transposed_convolutions_match_plain_loops() {
        for (cin, cout, h, w) in [(3usize, 4usize, 8usize, 8usize), (8, 8, 6, 10), (5, 2, 12, 4)] {
            let x = rnd(cin * h * w, 1);
            let k = rnd(cout * cin * 9, 2);
            let kt = rnd(cin * cout * 9, 4);
            let b = rnd(cout, 5);
            let want_s2 = naive_conv_s2(&x, cin, h, w, &k, cout);
            let want_t = naive_conv_t(&x, cin, h, w, &kt, cout, &b);
            for d in devices() {
                let _gpu = crate::gpu_lock(&d);
                let xt = Tensor::from_slice(&x, (1, cin, h, w), &d).unwrap();
                let got = vec4(&conv3_s2(&xt, &Tensor::from_slice(&k, (cout, cin, 3, 3), &d).unwrap()).unwrap());
                let e = got.iter().zip(&want_s2).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                assert!(e < 1e-4, "stride 2 {cin}→{cout} {h}×{w} on {d:?}: {e}");
                let got = vec4(
                    &conv_t3_s2(&xt, &Tensor::from_slice(&kt, (cin, cout, 3, 3), &d).unwrap(), &Tensor::from_slice(&b, cout, &d).unwrap()).unwrap(),
                );
                let e = got.iter().zip(&want_t).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                assert!(e < 1e-4, "transposed {cin}→{cout} {h}×{w} on {d:?}: {e}");
            }
        }
    }

    #[test]
    fn a_file_that_is_not_lama_is_an_error() {
        let p = std::env::temp_dir().join(format!("lc-lama-not-{}.safetensors", std::process::id()));
        std::fs::write(&p, b"definitely not safetensors").unwrap();
        assert!(Inpainter::load(&p, Device::Cpu).is_err());
        let t = Tensor::zeros(3, DType::F32, &Device::Cpu).unwrap();
        candle_core::safetensors::save(&std::collections::HashMap::from([("x".to_string(), t)]), &p).unwrap();
        assert!(Inpainter::load(&p, Device::Cpu).is_err());
        let _ = std::fs::remove_file(&p);
    }

    /// Opt-in, against the Python reference (`scripts/lama_ref.py` in the builder's scratch: advimman/lama's
    /// `ffc.py` on the same weights): `LC_LAMA=<big-lama.safetensors> LC_LAMA_REF=<ref.safetensors> cargo test -p
    /// lightcraft-enhance --release -- --ignored lama_matches`.
    #[test]
    #[ignore]
    fn lama_matches_the_reference() {
        let (Ok(p), Ok(r)) = (std::env::var("LC_LAMA"), std::env::var("LC_LAMA_REF")) else { return };
        let refs = candle_core::safetensors::load(&r, &Device::Cpu).unwrap();
        for d in devices() {
            let m = Inpainter::load(Path::new(&p), d.clone()).unwrap();
            let _gpu = crate::gpu_lock(&d);
            let x = refs["input"].to_device(&d).unwrap();
            let t = std::time::Instant::now();
            let y = vec4(&m.forward(&x).unwrap());
            eprintln!("{:?}: {:?} in {:?}", d, x.dims(), t.elapsed());
            let want = vec4(&refs["output"]);
            let e = y.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            let mean = y.iter().zip(&want).map(|(a, b)| (a - b).abs()).sum::<f32>() / want.len() as f32;
            eprintln!("worst {e}, mean {mean}");
            assert!(e < 2e-3, "{d:?}: worst difference {e}");
        }
    }
}
