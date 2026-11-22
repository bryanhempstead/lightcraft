//! 2× Super Resolution with Real-ESRGAN x2plus (Wang et al., 2021; BSD-3-Clause), an RRDBNet
//! (ESRGAN's Residual-in-Residual Dense Blocks) written here on candle, read from the published
//! PyTorch checkpoint (`params_ema`).
//!
//! The network was trained on display-encoded pictures, so linear camera RGB is put through the sRGB
//! curve on the way in and back out; values above white (highlights the camera recorded brighter
//! than the white level) are taken from a smooth 2× resize instead, so they keep their level.
//! Tiles of [`TILE`] pixels overlap by [`OVERLAP`] and are feathered together.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use lightcraft_raster::Rgb32f;

use crate::{Error, Result};

/// Tile side in input pixels (even), and the overlap between tiles.
pub const TILE: usize = 384;
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
pub struct SuperResolution {
    first: Conv2d,
    body: Vec<Rrdb>,
    conv_body: Conv2d,
    up1: Conv2d,
    up2: Conv2d,
    hr: Conv2d,
    last: Conv2d,
    device: Device,
    dtype: DType,
}

impl SuperResolution {
    pub fn load(path: &Path, device: Device) -> Result<Self> {
        let tensors = candle_core::pickle::read_all_with_key(path, Some("params_ema"))
            .or_else(|_| candle_core::pickle::read_all_with_key(path, Some("params")))
            .map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        if tensors.len() > 4096 {
            return Err(Error::Model("not a Real-ESRGAN checkpoint (too many tensors)".into()));
        }
        let map: HashMap<String, Tensor> = tensors.into_iter().collect();
        // half precision on the GPU: twice the arithmetic for the same memory traffic (the network was trained in it)
        let dtype = if device.is_cpu() || std::env::var_os("LIGHTCRAFT_ESRGAN_F32").is_some() { DType::F32 } else { DType::F16 };
        let vb = VarBuilder::from_tensors(map, dtype, &device);
        let body = (0..BLOCKS)
            .map(|i| {
                let b = vb.pp(format!("body.{i}"));
                Ok(Rrdb { rdb: [Rdb::new(b.pp("rdb1"))?, Rdb::new(b.pp("rdb2"))?, Rdb::new(b.pp("rdb3"))?] })
            })
            .collect::<candle_core::Result<Vec<_>>>()?;
        Ok(SuperResolution {
            first: conv(&vb, 12, FEAT, "conv_first")?,
            body,
            conv_body: conv(&vb, FEAT, FEAT, "conv_body")?,
            up1: conv(&vb, FEAT, FEAT, "conv_up1")?,
            up2: conv(&vb, FEAT, FEAT, "conv_up2")?,
            hr: conv(&vb, FEAT, FEAT, "conv_hr")?,
            last: conv(&vb, FEAT, 3, "conv_last")?,
            device,
            dtype,
        })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// `[1, 3, h, w]` (0..1, display-encoded, `h`, `w` even) → `[1, 3, 2h, 2w]`.
    pub fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
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
        let mut acc = vec![[0f32; 3]; ow * oh];
        let mut wsum = vec![0f32; ow * oh];
        let xs = starts(w, TILE, OVERLAP);
        let ys = starts(h, TILE, OVERLAP);
        let total = xs.len() * ys.len();
        let mut done = 0;
        for &y0 in &ys {
            for &x0 in &xs {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                let tw = TILE.min(w - x0) & !1;
                let th = TILE.min(h - y0) & !1;
                if tw < 2 || th < 2 {
                    continue;
                }
                let mut input = vec![0f32; 3 * tw * th];
                for y in 0..th {
                    for x in 0..tw {
                        let p = img.data[(y0 + y) * w + x0 + x];
                        for c in 0..3 {
                            input[c * tw * th + y * tw + x] = encode(p[c]);
                        }
                    }
                }
                let t = Tensor::from_vec(input, (1, 3, th, tw), &self.device)?.to_dtype(self.dtype)?;
                let out = self.forward(&t)?.to_dtype(DType::F32)?.clamp(0f32, 1f32)?.flatten_all()?.to_vec1::<f32>()?;
                let (tw2, th2) = (tw * 2, th * 2);
                if out.len() != 3 * tw2 * th2 {
                    return Err(Error::Model("unexpected output size".into()));
                }
                let fade = OVERLAP * 2;
                for y in 0..th2 {
                    let wy = ramp(y, th2, fade, y0 > 0, y0 + th < h);
                    for x in 0..tw2 {
                        let wgt = wy * ramp(x, tw2, fade, x0 > 0, x0 + tw < w);
                        let o = (y0 * 2 + y) * ow + x0 * 2 + x;
                        if let (Some(a), Some(s)) = (acc.get_mut(o), wsum.get_mut(o)) {
                            for c in 0..3 {
                                a[c] += wgt * decode(out[c * tw2 * th2 + y * tw2 + x]);
                            }
                            *s += wgt;
                        }
                    }
                }
                done += 1;
                progress(done, total);
            }
        }
        let mut res = Rgb32f::new(ow, oh);
        for (i, (o, (a, s))) in res.data.iter_mut().zip(acc.iter().zip(&wsum)).enumerate() {
            let (x, y) = (i % ow, i / ow);
            let src = bilinear(img, (x as f32 + 0.5) / 2.0 - 0.5, (y as f32 + 0.5) / 2.0 - 0.5);
            let net = if *s > 0.0 { a.map(|v| v / s) } else { src };
            // above white the network clipped: the smooth resize keeps the highlight's level
            let hot = src.iter().fold(0f32, |m, &v| m.max(v));
            let k = ((hot - 0.9) / 0.1).clamp(0.0, 1.0);
            *o = std::array::from_fn(|c| net[c] * (1.0 - k) + src[c] * k);
        }
        Ok(res)
    }
}

/// Tile starts covering `n` with tiles of `tile` overlapping by `overlap` (the last one flush with the end).
fn starts(n: usize, tile: usize, overlap: usize) -> Vec<usize> {
    if n <= tile {
        return vec![0];
    }
    let step = tile - overlap;
    let mut v: Vec<usize> = (0..).map(|i| i * step).take_while(|&s| s + tile < n).collect();
    v.push((n - tile) & !1);
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
        for n in [2usize, 100, 384, 385, 1000, 5999] {
            let s = starts(n, TILE, OVERLAP);
            assert_eq!(s[0], 0);
            assert!(s.last().unwrap() + TILE.min(n) >= n - 1, "{n}: {s:?}");
            assert!(s.windows(2).all(|p| p[1] > p[0] && p[1] - p[0] <= TILE - OVERLAP));
        }
        for v in [0.0f32, 0.001, 0.2, 0.5, 1.0] {
            assert!((decode(encode(v)) - v).abs() < 1e-5);
        }
    }

    /// Opt-in: `LC_ESRGAN=<RealESRGAN_x2plus.pth> cargo test -p lightcraft-enhance --release -- --ignored esrgan`
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
