//! 2× Super Resolution with Real-ESRGAN (Wang et al., 2021; BSD-3-Clause), written here on candle and read from
//! the published PyTorch checkpoints: `realesr-general-x4v3` (SRVGGNetCompact: 32 convolutions at the input's
//! resolution and a pixel shuffle, ×4, box-averaged to ×2; the default, ~10× faster) or `RealESRGAN_x2plus` (an
//! RRDBNet of 23 residual-in-residual dense blocks, ×2).
//!
//! The network was trained on display-encoded pictures, so linear camera RGB is put through the sRGB
//! curve on the way in and back out; values above white (highlights the camera recorded brighter
//! than the white level) are taken from a smooth 2× resize instead, so they keep their level.
//! Tiles of at most [`TILE`] pixels (as few as cover the picture, all of one size per axis) overlap by [`OVERLAP`]
//! and are feathered together.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use lightcraft_raster::Rgb32f;
use rayon::prelude::*;

use crate::{Error, Result};

/// Largest tile side in input pixels (even), and the overlap between tiles.
pub const TILE: usize = 1024;
pub const OVERLAP: usize = 24;
const FEAT: usize = 64;
const GROW: usize = 32;
const BLOCKS: usize = 23;

/// A 3 × 3 convolution (through [`crate::netrun::conv2d`], right on the CPU too).
struct Conv2d {
    w: Tensor,
    b: Tensor,
}

impl Conv2d {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        crate::netrun::conv2d(x, &self.w, 1)?.broadcast_add(&self.b)
    }
}

fn conv(vb: &VarBuilder, cin: usize, cout: usize, name: &str) -> candle_core::Result<Conv2d> {
    let vb = vb.pp(name);
    Ok(Conv2d { w: vb.get((cout, cin, 3, 3), "weight")?, b: vb.get(cout, "bias")?.reshape((1, cout, 1, 1))? })
}

fn lrelu(x: &Tensor) -> candle_core::Result<Tensor> {
    x.maximum(&(x * 0.2)?)
}

struct Rdb {
    convs: Vec<Conv2d>,
}

impl Rdb {
    fn new(vb: VarBuilder) -> candle_core::Result<Self> {
        let convs = (0..5)
            .map(|i| conv(&vb, FEAT + i * GROW, if i == 4 { FEAT } else { GROW }, &format!("conv{}", i + 1)))
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(Rdb { convs })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let mut cat = vec![x.clone()];
        for (i, c) in self.convs.iter().enumerate() {
            let inp = Tensor::cat(&cat, 1)?;
            let y = c.forward(&inp)?;
            if i == 4 {
                return (y * 0.2)? + x;
            }
            cat.push(lrelu(&y)?);
        }
        Ok(x.clone())
    }
}

struct Rrdb {
    rdb: [Rdb; 3],
}

impl Rrdb {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let y = self.rdb[2].forward(&self.rdb[1].forward(&self.rdb[0].forward(x)?)?)?;
        (y * 0.2)? + x
    }
}

/// The loaded network.
struct RrdbNet {
    first: Conv2d,
    body: Vec<Rrdb>,
    conv_body: Conv2d,
    up1: Conv2d,
    up2: Conv2d,
    hr: Conv2d,
    last: Conv2d,
}

/// SRVGGNetCompact: convolutions with per-channel PReLU, then a pixel shuffle, plus the nearest-upscaled input.
struct CompactNet {
    convs: Vec<Conv2d>,
    /// `[1, C, 1, 1]` slopes, one per convolution but the last.
    prelu: Vec<Tensor>,
    scale: usize,
    /// The same network laid out for [`CompactNet::forward_cl`] (×2 output: a ×4 network's last convolution has
    /// the 2 × 2 average folded in).
    cl: Vec<ClConv>,
}

/// One convolution of [`CompactNet::forward_cl`]: the picture stays channels-last (`[pixels, channels]`, rows of
/// the padded width), so a 3 × 3 convolution is one copy (the three horizontal neighbours side by side) and one
/// batched matrix product over the three kernel rows, with no transposes between layers.
struct ClConv {
    /// `[3 (ky), K, cout]`: row `kx · cin + ci` is tap (ky, kx) of input channel ci; row `3 · cin` holds the bias
    /// (kernel row 0 only), multiplied by a column of ones; the rest is zero padding to a multiple of 32 (the matrix product's block).
    w: Tensor,
    /// `K − 3 · cin`.
    extra: usize,
    cout: usize,
    act: Act,
}

enum Act {
    None,
    /// PReLU with every slope in 0..=1: `max(x, x · diag(a))`.
    Max(Tensor),
    /// Any other PReLU: `relu(x) + (x − relu(x)) · diag(a)`.
    Prelu(Tensor),
}

/// Host copy of a tensor (f32).
fn host(t: &Tensor) -> candle_core::Result<Vec<f32>> {
    t.to_dtype(DType::F32)?.flatten_all()?.to_vec1::<f32>()
}

impl ClConv {
    /// From PyTorch's `[cout, cin, 3, 3]` weights and `[cout]` bias (f32, host).
    fn new(w: &[f32], b: &[f32], cin: usize, cout: usize, slope: Option<&[f32]>, device: &Device, dtype: DType) -> candle_core::Result<Self> {
        if w.len() != cout * cin * 9 || b.len() != cout || slope.is_some_and(|s| s.len() != cout) {
            candle_core::bail!("convolution weights of the wrong size");
        }
        let k = (3 * cin + 1).div_ceil(32) * 32;
        let mut m = vec![0f32; 3 * k * cout];
        for ky in 0..3 {
            for kx in 0..3 {
                for ci in 0..cin {
                    for co in 0..cout {
                        if let (Some(d), Some(s)) = (m.get_mut((ky * k + kx * cin + ci) * cout + co), w.get(((co * cin + ci) * 3 + ky) * 3 + kx)) {
                            *d = *s;
                        }
                    }
                }
            }
        }
        for (co, v) in b.iter().enumerate() {
            if let Some(d) = m.get_mut(3 * cin * cout + co) {
                *d = *v;
            }
        }
        let act = match slope {
            None => Act::None,
            Some(a) => {
                let mut diag = vec![0f32; cout * cout];
                for (i, v) in a.iter().enumerate() {
                    if let Some(d) = diag.get_mut(i * cout + i) {
                        *d = *v;
                    }
                }
                let d = Tensor::from_vec(diag, (cout, cout), device)?.to_dtype(dtype)?;
                if a.iter().all(|v| (0.0..=1.0).contains(v)) { Act::Max(d) } else { Act::Prelu(d) }
            }
        };
        Ok(ClConv { w: Tensor::from_vec(m, (3, k, cout), device)?.to_dtype(dtype)?, extra: k - 3 * cin, cout, act })
    }
}

enum Net {
    Rrdb(Box<RrdbNet>),
    Compact(CompactNet),
}

pub struct SuperResolution {
    net: Net,
    device: Device,
    dtype: DType,
}

impl SuperResolution {
    pub fn load(path: &Path, device: Device) -> Result<Self> {
        let _gpu = crate::gpu_lock(&device);
        let tensors = candle_core::pickle::read_all_with_key(path, Some("params_ema"))
            .or_else(|_| candle_core::pickle::read_all_with_key(path, Some("params")))
            .map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        if tensors.len() > 4096 {
            return Err(Error::Model("not a Real-ESRGAN checkpoint (too many tensors)".into()));
        }
        let map: HashMap<String, Tensor> = tensors.into_iter().collect();
        // half precision on the GPU: twice the arithmetic for the same memory traffic (the network was trained in it)
        let dtype = if device.is_cpu() || std::env::var_os("LIGHTCRAFT_ESRGAN_F32").is_some() { DType::F32 } else { DType::F16 };
        if !map.contains_key("conv_first.weight") {
            return Self::load_compact(map, device, dtype);
        }
        let vb = VarBuilder::from_tensors(map, dtype, &device);
        let body = (0..BLOCKS)
            .map(|i| {
                let b = vb.pp(format!("body.{i}"));
                Ok(Rrdb { rdb: [Rdb::new(b.pp("rdb1"))?, Rdb::new(b.pp("rdb2"))?, Rdb::new(b.pp("rdb3"))?] })
            })
            .collect::<candle_core::Result<Vec<_>>>()?;
        let net = RrdbNet {
            first: conv(&vb, 12, FEAT, "conv_first")?,
            body,
            conv_body: conv(&vb, FEAT, FEAT, "conv_body")?,
            up1: conv(&vb, FEAT, FEAT, "conv_up1")?,
            up2: conv(&vb, FEAT, FEAT, "conv_up2")?,
            hr: conv(&vb, FEAT, FEAT, "conv_hr")?,
            last: conv(&vb, FEAT, 3, "conv_last")?,
        };
        Ok(SuperResolution { net: Net::Rrdb(Box::new(net)), device, dtype })
    }

    /// An SRVGGNetCompact checkpoint (`body.{2i}` convolutions, `body.{2i+1}` PReLUs).
    fn load_compact(map: HashMap<String, Tensor>, device: Device, dtype: DType) -> Result<Self> {
        let n = (0..1024).take_while(|i| map.contains_key(&format!("body.{i}.weight"))).count();
        if n < 3 || n % 2 == 0 {
            return Err(Error::Model("not a Real-ESRGAN checkpoint".into()));
        }
        let cl = Self::compact_cl(&map, n, &device, dtype)?;
        let vb = VarBuilder::from_tensors(map, dtype, &device);
        let mut convs = Vec::new();
        let mut prelu = Vec::new();
        let mut cin = 3;
        for i in (0..n).step_by(2) {
            let w = vb.get_unchecked(&format!("body.{i}.weight"))?;
            let (cout, wcin, kh, kw) = w.dims4()?;
            if wcin != cin || (kh, kw) != (3, 3) {
                return Err(Error::Model(format!("body.{i}: unexpected shape {:?}", w.dims())));
            }
            convs.push(Conv2d { w, b: vb.get(cout, &format!("body.{i}.bias"))?.reshape((1, cout, 1, 1))? });
            if i + 1 < n {
                prelu.push(vb.get(cout, &format!("body.{}.weight", i + 1))?.reshape((1, cout, 1, 1))?);
            }
            cin = cout;
        }
        // the last convolution gives 3 · scale² channels
        let scale = ((cin / 3) as f64).sqrt().round() as usize;
        if scale < 2 || 3 * scale * scale != cin {
            return Err(Error::Model("not a Real-ESRGAN checkpoint (output channels)".into()));
        }
        Ok(SuperResolution { net: Net::Compact(CompactNet { convs, prelu, scale, cl }), device, dtype })
    }

    /// The [`ClConv`]s of an SRVGGNetCompact checkpoint with `n` `body.*` entries (from the f32 checkpoint, before
    /// any conversion to half precision). A ×4 network's last convolution gets the 2 × 2 average of its pixel
    /// shuffle folded in (it is linear, and the nearest-upscaled input averages to itself), so it gives ×2.
    fn compact_cl(map: &HashMap<String, Tensor>, n: usize, device: &Device, dtype: DType) -> Result<Vec<ClConv>> {
        let get = |k: String| -> Result<(Vec<f32>, Vec<usize>)> {
            let t = map.get(&k).ok_or_else(|| Error::Model(format!("{k} is missing")))?;
            Ok((host(t)?, t.dims().to_vec()))
        };
        let mut out = Vec::new();
        for i in (0..n).step_by(2) {
            let (w, dims) = get(format!("body.{i}.weight"))?;
            let [cout, cin, 3, 3] = dims[..] else { return Err(Error::Model(format!("body.{i}: unexpected shape {dims:?}"))) };
            let (b, _) = get(format!("body.{i}.bias"))?;
            if i + 1 < n {
                let (a, _) = get(format!("body.{}.weight", i + 1))?;
                out.push(ClConv::new(&w, &b, cin, cout, Some(&a), device, dtype)?);
                continue;
            }
            // the last: 3 · 16 channels (c · 16 + dy · 4 + dx) → 3 · 4 (c · 4 + dy2 · 2 + dx2), each the mean of
            // the 2 × 2 sub-pixels it covers
            if cout == 48 {
                let mut w2 = vec![0f32; 12 * cin * 9];
                let mut b2 = vec![0f32; 12];
                for c in 0..3 {
                    for dy2 in 0..2 {
                        for dx2 in 0..2 {
                            let o = c * 4 + dy2 * 2 + dx2;
                            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                                let s = c * 16 + (2 * dy2 + dy) * 4 + 2 * dx2 + dx;
                                for j in 0..cin * 9 {
                                    if let (Some(d), Some(v)) = (w2.get_mut(o * cin * 9 + j), w.get(s * cin * 9 + j)) {
                                        *d += v / 4.0;
                                    }
                                }
                                if let (Some(d), Some(v)) = (b2.get_mut(o), b.get(s)) {
                                    *d += v / 4.0;
                                }
                            }
                        }
                    }
                }
                out.push(ClConv::new(&w2, &b2, cin, 12, None, device, dtype)?);
            } else {
                out.push(ClConv::new(&w, &b, cin, cout, None, device, dtype)?);
            }
        }
        Ok(out)
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// `[1, 3, h, w]` (0..1, display-encoded, `h`, `w` even) → `[1, 3, 2h, 2w]`.
    pub fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        match &self.net {
            Net::Rrdb(n) => n.forward(x),
            Net::Compact(n) if !n.cl.is_empty() && !legacy() => n.forward_cl(x),
            Net::Compact(n) => {
                let y = n.forward(x)?;
                // ×4 → ×2: each output pixel the mean of 2 × 2 (supersampled)
                match n.scale {
                    2 => Ok(y),
                    4 => y.avg_pool2d(2),
                    s => candle_core::bail!("a ×{s} network"),
                }
            }
        }
    }
}

/// `LIGHTCRAFT_ESRGAN_LEGACY`: the compact network through its original layout (per-layer NCHW convolutions, ×4
/// then averaged), for comparing.
fn legacy() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LIGHTCRAFT_ESRGAN_LEGACY").is_some())
}

impl CompactNet {
    /// [`CompactNet::forward`], ×2, through [`ClConv`]s: `[1, 3, h, w]` → `[1, 3, 2h, 2w]`.
    ///
    /// The picture is kept as `[pixels, channels]` over rows of the padded width `w + 2`, with one zero row above,
    /// two below and a zero column each side, so tap (ky, kx) of output pixel `p` is input row `p + ky · (w + 2) +
    /// kx`. Each layer copies the three horizontal neighbours side by side (plus a column of ones for the bias),
    /// views that as three overlapping matrices one padded row apart and multiplies all three by their kernel rows
    /// at once; the sum is the convolution over the padded width, whose two extra columns are zeroed to become the
    /// next layer's padding.
    fn forward_cl(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (n, c, h, w) = x.dims4()?;
        if n != 1 {
            candle_core::bail!("one tile at a time");
        }
        let wp = w + 2;
        let rows = (h + 3) * wp;
        let len = h * wp;
        let n3 = rows - 2;
        let (dtype, device) = (x.dtype(), x.device());
        // the picture over the padded width (the two extra columns zero): `[h · wp, c]`
        let mut z = x.pad_with_zeros(3, 0, 2)?.reshape((c, len))?.t()?.contiguous()?;
        // per input width: `[n3, K]`, row q = padded pixels q, q + 1, q + 2 side by side, then the ones column.
        // Kept from layer to layer: only the picture's own rows of each block are written, so the padding rows and
        // the bias column stay as made.
        let mut cxs: Vec<(usize, Tensor)> = Vec::new();
        let mut zeros: Vec<(usize, Tensor)> = Vec::new();
        let last = self.cl.len().saturating_sub(1);
        for (i, l) in self.cl.iter().enumerate() {
            let cin = z.dim(1)?;
            let cx = match cxs.iter().find(|b| b.0 == cin) {
                Some(b) => b.1.clone(),
                None => {
                    let cx = Tensor::zeros((n3, 3 * cin + l.extra), dtype, device)?;
                    cx.slice_set(&Tensor::ones((n3, 1), dtype, device)?, 1, 3 * cin)?;
                    cxs.push((cin, cx.clone()));
                    cx
                }
            };
            if cx.dim(1)? != l.w.dim(1)? {
                candle_core::bail!("layer {i} does not fit its input");
            }
            // padded pixel t is picture pixel t − (wp + 1); block kx of row q holds padded pixel q + kx
            for kx in 0..3 {
                cx.narrow(0, wp + 1 - kx, len)?.slice_set(&z, 1, kx * cin)?;
            }
            // [3, len, K]: row p of matrix ky is cx row p + ky · wp
            let r = cx.unfold(0, len, wp)?.narrow(0, 0, 3)?.transpose(1, 2)?.matmul(&l.w)?;
            let s = ((r.get(0)? + r.get(1)?)? + r.get(2)?)?;
            z = match &l.act {
                Act::None => s,
                Act::Max(d) => s.maximum(&s.matmul(d)?)?,
                Act::Prelu(d) => {
                    let pos = s.relu()?;
                    (&pos + (s - &pos)?.matmul(d)?)?
                }
            };
            if i == last {
                // [h·wp, 12] → [1, 12, h, w] → pixel shuffle ×2, plus the nearest-upscaled input
                let co = l.cout;
                if co != 12 {
                    candle_core::bail!("a last layer of {co} channels");
                }
                let y = z.reshape((h, wp, co))?.narrow(1, 0, w)?.permute((2, 0, 1))?.contiguous()?;
                let y = y.reshape((1, 3, 2, 2, h, w))?.permute((0, 1, 4, 2, 5, 3))?.contiguous()?.reshape((1, 3, h * 2, w * 2))?;
                return y + x.upsample_nearest2d(h * 2, w * 2)?;
            }
            // the two columns past the width wrapped around: they become the padding of the next layer
            let zero = match zeros.iter().find(|z| z.0 == l.cout) {
                Some(z) => z.1.clone(),
                None => {
                    let t = Tensor::zeros((h, 2, l.cout), dtype, device)?;
                    zeros.push((l.cout, t.clone()));
                    t
                }
            };
            z.reshape((h, wp, l.cout))?.slice_set(&zero, 1, w)?;
        }
        candle_core::bail!("an empty network")
    }

    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let mut y = x.clone();
        for (i, c) in self.convs.iter().enumerate() {
            y = c.forward(&y)?;
            if let Some(a) = self.prelu.get(i) {
                let pos = y.relu()?;
                y = (&pos + (y - &pos)?.broadcast_mul(a)?)?;
            }
        }
        let (n, c, h, w) = y.dims4()?;
        let s = self.scale;
        // pixel shuffle: channel c·s² + dy·s + dx → (s·y + dy, s·x + dx)
        let y = y.reshape((n, c / (s * s), s, s, h, w))?.permute((0, 1, 4, 2, 5, 3))?.contiguous()?.reshape((n, c / (s * s), h * s, w * s))?;
        y + x.upsample_nearest2d(h * s, w * s)?
    }
}

impl RrdbNet {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (n, c, h, w) = x.dims4()?;
        // pixel unshuffle ×2: channel 4c + 2dy + dx
        let x = x.reshape((n, c, h / 2, 2, w / 2, 2))?.permute((0, 1, 3, 5, 2, 4))?.reshape((n, c * 4, h / 2, w / 2))?;
        let feat = self.first.forward(&x)?;
        let mut b = feat.clone();
        for r in &self.body {
            b = r.forward(&b)?;
        }
        let feat = (feat + self.conv_body.forward(&b)?)?;
        let (_, _, fh, fw) = feat.dims4()?;
        let feat = lrelu(&self.up1.forward(&feat.upsample_nearest2d(fh * 2, fw * 2)?)?)?;
        let feat = lrelu(&self.up2.forward(&feat.upsample_nearest2d(fh * 4, fw * 4)?)?)?;
        self.last.forward(&lrelu(&self.hr.forward(&feat)?)?)
    }
}

impl SuperResolution {
    /// Upscale linear camera RGB (white = 1) 2×. `progress(done, total)` after each tile; `cancel` stops between
    /// tiles.
    pub fn upscale(&self, img: &Rgb32f, cancel: &AtomicBool, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<Rgb32f> {
        let (w, h) = (img.width, img.height);
        if w < 2 || h < 2 {
            return Err(Error::Input("the picture is too small".into()));
        }
        let (ow, oh) = (w.checked_mul(2).ok_or(Error::Input("too large".into()))?, h * 2);
        if ow.checked_mul(oh).is_none_or(|n| n > 400_000_000) {
            return Err(Error::Input("the picture is too large for Super Resolution".into()));
        }
        // (colour sum, weight) per output pixel
        let mut acc = vec![[0f32; 4]; ow * oh];
        let (tile_w, tile_h) = (tile_side(w, TILE, OVERLAP), tile_side(h, TILE, OVERLAP));
        let xs = starts(w, tile_w, OVERLAP);
        let ys = starts(h, tile_h, OVERLAP);
        let tiles: Vec<[usize; 4]> = ys
            .iter()
            .flat_map(|&y0| xs.iter().map(move |&x0| [x0, y0, tile_w.min(w - x0) & !1, tile_h.min(h - y0) & !1]))
            .filter(|t| t[2] >= 2 && t[3] >= 2)
            .collect();
        let total = tiles.len();
        let profile = std::env::var_os("LIGHTCRAFT_PROFILE").is_some();
        let (mut t_gpu, mut t_host) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let t_all = std::time::Instant::now();
        // the GPU works on the next tile while the CPU blends the last one in (a read waits for all queued work,
        // so each tile is read before the next is queued)
        let mut pending: Option<([usize; 4], Tensor)> = None;
        for (done, next) in tiles.iter().map(Some).chain(std::iter::once(None)).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            let t0 = std::time::Instant::now();
            let out = match pending.take() {
                Some((tile, y)) => {
                    let _gpu = crate::gpu_lock(&self.device);
                    Some((tile, y.to_vec1::<f32>()?))
                }
                None => None,
            };
            t_gpu += t0.elapsed();
            if let Some(&[x0, y0, tw, th]) = next {
                let input = tile_input(img, x0, y0, tw, th);
                let _gpu = crate::gpu_lock(&self.device);
                let t = Tensor::from_vec(input, (1, 3, th, tw), &self.device)?.to_dtype(self.dtype)?;
                pending = Some(([x0, y0, tw, th], self.forward(&t)?.to_dtype(DType::F32)?.clamp(0f32, 1f32)?.flatten_all()?));
            }
            let t0 = std::time::Instant::now();
            if let Some((tile, out)) = out {
                blend(&mut acc, ow, (w, h), tile, &out)?;
                progress(done, total);
            }
            t_host += t0.elapsed();
        }
        let t0 = std::time::Instant::now();
        let mut res = Rgb32f::new(ow, oh);
        res.data.par_chunks_mut(ow).zip(acc.par_chunks(ow)).enumerate().for_each(|(y, (row, accs))| {
            for (x, (o, a)) in row.iter_mut().zip(accs).enumerate() {
                let src = bilinear(img, (x as f32 + 0.5) / 2.0 - 0.5, (y as f32 + 0.5) / 2.0 - 0.5);
                let net = if a[3] > 0.0 { [a[0] / a[3], a[1] / a[3], a[2] / a[3]] } else { src };
                // above white the network clipped: the smooth resize keeps the highlight's level
                let hot = src.iter().fold(0f32, |m, &v| m.max(v));
                let k = ((hot - 0.9) / 0.1).clamp(0.0, 1.0);
                *o = std::array::from_fn(|c| net[c] * (1.0 - k) + src[c] * k);
            }
        });
        if profile {
            eprintln!(
                "[profile] super resolution {w}×{h}: {total} tiles in {:.1} s (waiting for the GPU {:.1} s, blending {:.1} s, finishing {:.1} s)",
                t_all.elapsed().as_secs_f64(),
                t_gpu.as_secs_f64(),
                t_host.as_secs_f64(),
                t0.elapsed().as_secs_f64()
            );
        }
        Ok(res)
    }
}

/// A tile of `img` as the network's input: `[3, th, tw]`, display-encoded.
fn tile_input(img: &Rgb32f, x0: usize, y0: usize, tw: usize, th: usize) -> Vec<f32> {
    let mut input = vec![0f32; 3 * tw * th];
    let (r, rest) = input.split_at_mut(tw * th);
    let (g, b) = rest.split_at_mut(tw * th);
    r.par_chunks_mut(tw).zip(g.par_chunks_mut(tw)).zip(b.par_chunks_mut(tw)).enumerate().for_each(|(y, ((r, g), b))| {
        let start = (y0 + y) * img.width + x0;
        let Some(src) = img.data.get(start..start + tw) else { return };
        for (x, p) in src.iter().enumerate() {
            if let (Some(r), Some(g), Some(b)) = (r.get_mut(x), g.get_mut(x), b.get_mut(x)) {
                (*r, *g, *b) = (encode(p[0]), encode(p[1]), encode(p[2]));
            }
        }
    });
    input
}

/// Add a tile's ×2 output (`[3, 2th, 2tw]`, display-encoded) into `acc` (output width `ow`), feathered where it
/// overlaps its neighbours in a `w × h` input.
fn blend(acc: &mut [[f32; 4]], ow: usize, (w, h): (usize, usize), [x0, y0, tw, th]: [usize; 4], out: &[f32]) -> Result<()> {
    let (tw2, th2) = (tw * 2, th * 2);
    if out.len() != 3 * tw2 * th2 {
        return Err(Error::Model("unexpected output size".into()));
    }
    let fade = OVERLAP * 2;
    let plane = tw2 * th2;
    acc.par_chunks_mut(ow).skip(y0 * 2).take(th2).enumerate().for_each(|(y, row)| {
        let wy = ramp(y, th2, fade, y0 > 0, y0 + th < h);
        let Some(row) = row.get_mut(x0 * 2..x0 * 2 + tw2) else { return };
        for (x, a) in row.iter_mut().enumerate() {
            let wgt = wy * ramp(x, tw2, fade, x0 > 0, x0 + tw < w);
            let i = y * tw2 + x;
            for (c, v) in a.iter_mut().take(3).enumerate() {
                *v += wgt * decode(out.get(c * plane + i).copied().unwrap_or(0.0));
            }
            a[3] += wgt;
        }
    });
    Ok(())
}

/// The tile side (even, at most `max`) for as few tiles as cover `n` with `overlap`, each no larger than needed: a
/// 6112-wide photo takes 7 tiles of 894, not 7 of 1024 overlapping by far more than `overlap`.
fn tile_side(n: usize, max: usize, overlap: usize) -> usize {
    if n <= max || max <= overlap {
        return max;
    }
    let k = (n - overlap).div_ceil(max - overlap).max(2);
    (n + (k - 1) * overlap).div_ceil(k).next_multiple_of(2).min(max)
}

/// Tile starts covering `n` with tiles of `tile` overlapping by at least `overlap`, spread evenly (even starts).
fn starts(n: usize, tile: usize, overlap: usize) -> Vec<usize> {
    if n <= tile || tile <= overlap {
        return vec![0];
    }
    let k = (n - overlap).div_ceil(tile - overlap).max(2);
    let mut v: Vec<usize> = (0..k).map(|i| ((n - tile) * i / (k - 1)) & !1).collect();
    v.dedup();
    v
}

fn ramp(p: usize, len: usize, fade: usize, before: bool, after: bool) -> f32 {
    let mut w = 1f32;
    if before && p < fade {
        w = w.min((p as f32 + 0.5) / fade as f32);
    }
    if after && p + fade >= len {
        w = w.min((len - p) as f32 / fade as f32);
    }
    w.max(1e-4)
}

fn encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

fn decode(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn bilinear(img: &Rgb32f, x: f32, y: f32) -> [f32; 3] {
    let (w, h) = (img.width as isize, img.height as isize);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: isize, yi: isize| img.data[(yi.clamp(0, h - 1) * w + xi.clamp(0, w - 1)) as usize];
    let (x0, y0) = (x0 as isize, y0 as isize);
    let (a, b, c, d) = (at(x0, y0), at(x0 + 1, y0), at(x0, y0 + 1), at(x0 + 1, y0 + 1));
    std::array::from_fn(|i| (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_cover_the_picture() {
        for n in [2usize, 100, 384, 385, 1000, 1025, 2047, 4000, 4064, 5999, 6000, 6112, 9504] {
            let t = tile_side(n, TILE, OVERLAP);
            assert!(t <= TILE && t.is_multiple_of(2));
            let s = starts(n, t, OVERLAP);
            assert_eq!(s[0], 0);
            assert!(s.last().unwrap() + t.min(n) >= n - 1, "{n}: {s:?}");
            assert!(s.windows(2).all(|p| p[1] > p[0] && p[1] - p[0] <= t - OVERLAP));
        }
        for v in [0.0f32, 0.001, 0.2, 0.5, 1.0] {
            assert!((decode(encode(v)) - v).abs() < 1e-5);
        }
    }

    /// A small SRVGGNetCompact (3 → 8 → 8 → 48 channels, PReLU slopes inside and outside 0..1) through the
    /// channels-last layout matches the original NCHW one (×4, averaged), on the CPU and the GPU.
    #[test]
    fn channels_last_compact_net_matches_the_original_layout() {
        let rnd = |n: usize, s: u64, scale: f32| -> Vec<f32> {
            (0..n).map(|i| ((((i as u64 * 7919 + s * 104_729) % 1000) as f32 / 1000.0) - 0.5) * scale).collect()
        };
        let mut devices = vec![Device::Cpu];
        if !crate::best_device().is_cpu() {
            devices.push(crate::best_device());
        }
        for mixed in [false, true] {
            for d in &devices {
                let _gpu = crate::gpu_lock(d);
                let cpu = Device::Cpu;
                let mut map = HashMap::new();
                let mut put = |k: &str, v: Vec<f32>, shape: &[usize]| {
                    map.insert(k.to_string(), Tensor::from_vec(v, shape, &cpu).unwrap());
                };
                put("body.0.weight", rnd(8 * 3 * 9, 1, 0.6), &[8, 3, 3, 3]);
                put("body.0.bias", rnd(8, 2, 0.2), &[8]);
                put("body.1.weight", rnd(8, 3, if mixed { 3.0 } else { 0.5 }).iter().map(|v| if mixed { *v } else { v + 0.25 }).collect(), &[8]);
                put("body.2.weight", rnd(8 * 8 * 9, 4, 0.4), &[8, 8, 3, 3]);
                put("body.2.bias", rnd(8, 5, 0.2), &[8]);
                put("body.3.weight", rnd(8, 6, 0.4).iter().map(|v| v + 0.2).collect(), &[8]);
                put("body.4.weight", rnd(48 * 8 * 9, 7, 0.4), &[48, 8, 3, 3]);
                put("body.4.bias", rnd(48, 8, 0.1), &[48]);
                let sr = SuperResolution::load_compact(map, d.clone(), DType::F32).unwrap();
                let Net::Compact(n) = &sr.net else { panic!("not compact") };
                assert_eq!(n.cl.len(), 3);
                assert_eq!(matches!(n.cl[0].act, Act::Prelu(_)), mixed);
                let (h, w) = (14, 10);
                let x = Tensor::from_vec(rnd(3 * h * w, 9, 1.0).iter().map(|v| v + 0.5).collect::<Vec<f32>>(), (1, 3, h, w), d).unwrap();
                let want = n.forward(&x).unwrap().avg_pool2d(2).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
                let got = n.forward_cl(&x).unwrap();
                assert_eq!(got.dims(), &[1, 3, 2 * h, 2 * w]);
                let got = got.flatten_all().unwrap().to_vec1::<f32>().unwrap();
                let worst = want.iter().zip(&got).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                assert!(worst < 1e-4, "{d:?} mixed {mixed}: {worst}");
            }
        }
    }

    /// Opt-in: `LC_ESRGAN=<realesr-general-x4v3.pth or RealESRGAN_x2plus.pth> cargo test -p lightcraft-enhance --release -- --ignored esrgan`
    #[test]
    #[ignore]
    fn esrgan_upscales_a_gradient() {
        let Ok(p) = std::env::var("LC_ESRGAN") else { return };
        let m = SuperResolution::load(Path::new(&p), crate::best_device()).unwrap();
        let mut img = Rgb32f::new(500, 300);
        for (i, px) in img.data.iter_mut().enumerate() {
            let (x, y) = (i % 500, i / 500);
            *px = [x as f32 / 500.0, y as f32 / 300.0, if (x / 20 + y / 20) % 2 == 0 { 0.8 } else { 0.1 }];
        }
        let t = std::time::Instant::now();
        let out = m.upscale(&img, &AtomicBool::new(false), &|_, _| {}).unwrap();
        eprintln!("{}×{} → {}×{} in {:?}", img.width, img.height, out.width, out.height, t.elapsed());
        assert_eq!((out.width, out.height), (1000, 600));
        let p = out.data[300 * 1000 + 500];
        assert!((p[0] - 0.5).abs() < 0.1 && (p[1] - 0.5).abs() < 0.1, "{p:?}");
    }
}
