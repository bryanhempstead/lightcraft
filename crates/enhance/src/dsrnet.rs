//! Remove reflections: DSRNet, "Single Image Reflection Separation via Component Synergy" (Hu & Guo, ICCV 2023;
//! mingcv/DSRNet, Apache-2.0), the `dsrnet_l_nature` network written here on candle from the published code
//! (`models/arch/dsrnet.py`, `lrm.py`) and read from its released checkpoint, with the ImageNet VGG-19 features it
//! is built on (torchvision's `vgg19`, BSD-3-Clause).
//!
//! The picture is split into two streams that trade information at every block (MuGI blocks: NAFNet-style gated
//! convolutions whose gates mix the streams): the transmission (the scene behind the glass) and the reflection.
//! A VGG-19 feature pyramid of the input seeds both. Everything is 1 × 1, depthwise 3 × 3 or plain 3 × 3
//! convolutions ([`crate::netrun::conv2d`]), pixel shuffles and one bilinear (aligned corners) upsampling.

use std::path::Path;

use candle_core::{D, DType, Device, Tensor};
use lightcraft_raster::Rgb32f;

use crate::{Error, Result};

/// The input's sides must be multiples of this (four halvings).
pub const MULTIPLE: usize = 16;

/// Named tensors of a checkpoint, read one at a time: a PyTorch (zip) checkpoint, or a safetensors file of which only
/// the tensors asked for are read (positional reads, every range checked against the file's length).
enum Source {
    Pth(candle_core::pickle::PthTensors),
    Safetensors { path: std::path::PathBuf, len: u64, start: u64, header: serde_json::Map<String, serde_json::Value> },
}

struct Weights {
    src: Source,
    device: Device,
}

/// Largest safetensors header read (the VGG-19 file's is a few KB).
const MAX_HEADER: u64 = 16 << 20;

impl Weights {
    fn open(path: &Path, device: &Device) -> Result<Self> {
        let bad = |e: String| Error::Model(format!("{}: {e}", path.display()));
        let is_st = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("safetensors"));
        let src = if is_st {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(path).map_err(|e| bad(e.to_string()))?;
            let len = f.metadata().map_err(|e| bad(e.to_string()))?.len();
            let mut n = [0u8; 8];
            f.read_exact(&mut n).map_err(|e| bad(e.to_string()))?;
            let n = u64::from_le_bytes(n);
            if n > MAX_HEADER || n.saturating_add(8) > len {
                return Err(bad("not a safetensors file".into()));
            }
            let mut h = vec![0u8; n as usize];
            f.seek(SeekFrom::Start(8)).map_err(|e| bad(e.to_string()))?;
            f.read_exact(&mut h).map_err(|e| bad(e.to_string()))?;
            let header: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&h).map_err(|e| bad(e.to_string()))?;
            Source::Safetensors { path: path.to_path_buf(), len, start: 8 + n, header }
        } else {
            let pth = candle_core::pickle::PthTensors::new(path, None).map_err(|e| bad(e.to_string()))?;
            if pth.tensor_infos().len() > 8192 {
                return Err(bad("too many tensors".into()));
            }
            Source::Pth(pth)
        };
        Ok(Weights { src, device: device.clone() })
    }

    fn get(&self, name: &str, shape: &[usize]) -> candle_core::Result<Tensor> {
        let t = match &self.src {
            Source::Pth(p) => p.get(name)?.ok_or_else(|| candle_core::Error::Msg(format!("`{name}` is missing")))?,
            Source::Safetensors { path, len, start, header } => {
                use std::io::{Read, Seek, SeekFrom};
                let info = header.get(name).ok_or_else(|| candle_core::Error::Msg(format!("`{name}` is missing")))?;
                if info.get("dtype").and_then(|d| d.as_str()) != Some("F32") {
                    candle_core::bail!("`{name}` is not float32");
                }
                let off: Vec<u64> =
                    info.get("data_offsets").and_then(|o| o.as_array()).map(|a| a.iter().filter_map(|v| v.as_u64()).collect()).unwrap_or_default();
                let &[a, b] = off.as_slice() else { candle_core::bail!("`{name}` has no data range") };
                let want = shape.iter().product::<usize>() as u64 * 4;
                let (a, b) = (start.saturating_add(a), start.saturating_add(b));
                if b < a || b - a != want || b > *len {
                    candle_core::bail!("`{name}`'s data doesn't fit the file or its shape");
                }
                let mut f = std::fs::File::open(path)?;
                f.seek(SeekFrom::Start(a))?;
                let mut bytes = vec![0u8; want as usize];
                f.read_exact(&mut bytes)?;
                Tensor::from_raw_buffer(&bytes, DType::F32, shape, &Device::Cpu)?
            }
        };
        if t.dims() != shape {
            candle_core::bail!("`{name}` has shape {:?}, not {shape:?}", t.dims());
        }
        t.to_dtype(DType::F32)?.to_device(&self.device)
    }
}

fn col(t: Tensor, c: usize) -> candle_core::Result<Tensor> {
    t.reshape((1, c, 1, 1))
}

/// A 1 × 1 convolution `[cout, cin]` with an optional bias.
struct Conv1 {
    w: Tensor,
    b: Option<Tensor>,
}

impl Conv1 {
    fn load(w: &Weights, name: &str, cin: usize, cout: usize, bias: bool) -> candle_core::Result<Self> {
        Ok(Conv1 {
            w: w.get(&format!("{name}.weight"), &[cout, cin, 1, 1])?.reshape((cout, cin))?,
            b: if bias { Some(col(w.get(&format!("{name}.bias"), &[cout])?, cout)?) } else { None },
        })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (_, cin, h, wd) = x.dims4()?;
        let cout = self.w.dim(0)?;
        let y = self.w.matmul(&x.contiguous()?.reshape((cin, h * wd))?)?.reshape((1, cout, h, wd))?;
        match &self.b {
            Some(b) => y.broadcast_add(b),
            None => Ok(y),
        }
    }
}

/// A 3 × 3 convolution, zero padding 1, with bias.
struct Conv3 {
    w: Tensor,
    b: Tensor,
}

impl Conv3 {
    fn load(w: &Weights, name: &str, cin: usize, cout: usize) -> candle_core::Result<Self> {
        Ok(Conv3 { w: w.get(&format!("{name}.weight"), &[cout, cin, 3, 3])?, b: col(w.get(&format!("{name}.bias"), &[cout])?, cout)? })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        crate::netrun::conv2d(x, &self.w, 1)?.broadcast_add(&self.b)
    }
}

/// A depthwise 3 × 3 convolution (one kernel per channel), zero padding 1, with bias.
struct DwConv3 {
    /// `[9][1, C, 1, 1]`, tap by tap.
    taps: Vec<Tensor>,
    b: Tensor,
}

impl DwConv3 {
    fn load(w: &Weights, name: &str, c: usize) -> candle_core::Result<Self> {
        let k = w.get(&format!("{name}.weight"), &[c, 1, 3, 3])?.reshape((c, 9))?;
        let taps = (0..9).map(|i| k.narrow(1, i, 1)?.contiguous()?.reshape((1, c, 1, 1))).collect::<candle_core::Result<Vec<_>>>()?;
        Ok(DwConv3 { taps, b: col(w.get(&format!("{name}.bias"), &[c])?, c)? })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (_, _, h, wd) = x.dims4()?;
        let p = x.pad_with_zeros(2, 1, 1)?.pad_with_zeros(3, 1, 1)?;
        let mut acc = self.b.clone();
        for (i, t) in self.taps.iter().enumerate() {
            let s = p.narrow(2, i / 3, h)?.narrow(3, i % 3, wd)?;
            acc = s.broadcast_mul(t)?.broadcast_add(&acc)?;
        }
        Ok(acc)
    }
}

/// A 2 × 2, stride-2 convolution with bias (`H`, `W` even): one matrix product per phase.
struct Down {
    /// `[2][2]` of `[cout, cin]`.
    taps: Vec<Tensor>,
    b: Tensor,
}

impl Down {
    fn load(w: &Weights, name: &str, cin: usize, cout: usize) -> candle_core::Result<Self> {
        let k = w.get(&format!("{name}.weight"), &[cout, cin, 2, 2])?;
        let mut taps = Vec::with_capacity(4);
        for a in 0..2 {
            for b in 0..2 {
                taps.push(k.narrow(2, a, 1)?.narrow(3, b, 1)?.contiguous()?.reshape((cout, cin))?);
            }
        }
        Ok(Down { taps, b: col(w.get(&format!("{name}.bias"), &[cout])?, cout)? })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let (_, cin, h, wd) = x.dims4()?;
        if !h.is_multiple_of(2) || !wd.is_multiple_of(2) {
            candle_core::bail!("a stride-2 layer needs even sides");
        }
        let (oh, ow) = (h / 2, wd / 2);
        let x = x.contiguous()?.reshape((cin, oh, 2, ow, 2))?;
        let cout = self.b.dim(1)?;
        let mut acc: Option<Tensor> = None;
        for a in 0..2 {
            for b in 0..2 {
                let ph = x.narrow(2, a, 1)?.narrow(4, b, 1)?.contiguous()?.reshape((cin, oh * ow))?;
                let y = self.taps[a * 2 + b].matmul(&ph)?;
                acc = Some(match acc {
                    Some(s) => (s + y)?,
                    None => y,
                });
            }
        }
        let acc = acc.ok_or_else(|| candle_core::Error::Msg("empty kernel".into()))?;
        acc.reshape((1, cout, oh, ow))?.broadcast_add(&self.b)
    }
}

/// Layer norm over the channels of each pixel (eps 1e-6).
struct Ln {
    w: Tensor,
    b: Tensor,
}

impl Ln {
    fn load(w: &Weights, name: &str, c: usize) -> candle_core::Result<Self> {
        Ok(Ln { w: col(w.get(&format!("{name}.weight"), &[c])?, c)?, b: col(w.get(&format!("{name}.bias"), &[c])?, c)? })
    }
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let mu = x.mean_keepdim(1)?;
        let d = x.broadcast_sub(&mu)?;
        let var = d.sqr()?.mean_keepdim(1)?;
        d.broadcast_div(&(var + 1e-6)?.sqrt()?)?.broadcast_mul(&self.w)?.broadcast_add(&self.b)
    }
}

fn halves(x: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
    let c = x.dim(1)? / 2;
    Ok((x.narrow(1, 0, c)?, x.narrow(1, c, c)?))
}

/// Channel attention: `x · conv(mean(x))`.
fn ca(conv: &Conv1, x: &Tensor) -> candle_core::Result<Tensor> {
    let m = x.mean_keepdim(D::Minus1)?.mean_keepdim(D::Minus2)?;
    x.broadcast_mul(&conv.forward(&m)?)
}

/// The two-stream block (MuGIBlock / R2Block): each stream's halves gate the other's.
struct Mugi {
    ln1: Ln,
    c1: Conv1,
    dw: DwConv3,
    ca: Conv1,
    c2: Conv1,
    a_l: Tensor,
    a_r: Tensor,
    ln2: Ln,
    c3: Conv1,
    c4: Conv1,
    b_l: Tensor,
    b_r: Tensor,
}

impl Mugi {
    /// `seq` is the name of the shared stream modules (`seq`, or `seq_l` in the LRM), `shared_b` a single `b`.
    fn load(w: &Weights, name: &str, c: usize, seq: &str, shared_b: bool) -> candle_core::Result<Self> {
        let p = |s: &str| format!("{name}.{s}");
        let (b_l, b_r) = if shared_b {
            let b = w.get(&p("b"), &[1, c, 1, 1])?;
            (b.clone(), b)
        } else {
            (w.get(&p("b_l"), &[1, c, 1, 1])?, w.get(&p("b_r"), &[1, c, 1, 1])?)
        };
        Ok(Mugi {
            ln1: Ln::load(w, &p(&format!("block1.0.{seq}.0")), c)?,
            c1: Conv1::load(w, &p(&format!("block1.0.{seq}.1")), c, 2 * c, true)?,
            dw: DwConv3::load(w, &p(&format!("block1.0.{seq}.2")), 2 * c)?,
            ca: Conv1::load(w, &p(&format!("block1.2.{seq}.0.ca.1")), c, c, true)?,
            c2: Conv1::load(w, &p(&format!("block1.3.{seq}.0")), c, c, true)?,
            a_l: w.get(&p("a_l"), &[1, c, 1, 1])?,
            a_r: w.get(&p("a_r"), &[1, c, 1, 1])?,
            ln2: Ln::load(w, &p(&format!("block2.0.{seq}.0")), c)?,
            c3: Conv1::load(w, &p(&format!("block2.0.{seq}.1")), c, 2 * c, true)?,
            c4: Conv1::load(w, &p(&format!("block2.2.{seq}.0")), c, c, true)?,
            b_l,
            b_r,
        })
    }

    fn forward(&self, l: &Tensor, r: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
        let (x, y) = (self.dw.forward(&self.c1.forward(&self.ln1.forward(l)?)?)?, self.dw.forward(&self.c1.forward(&self.ln1.forward(r)?)?)?);
        let ((x1, x2), (y1, y2)) = (halves(&x)?, halves(&y)?);
        let (x, y) = ((x1 * y2)?, (y1 * x2)?);
        let (x, y) = (self.c2.forward(&ca(&self.ca, &x)?)?, self.c2.forward(&ca(&self.ca, &y)?)?);
        let (sl, sr) = ((l + x.broadcast_mul(&self.a_l)?)?, (r + y.broadcast_mul(&self.a_r)?)?);
        let (x, y) = (self.c3.forward(&self.ln2.forward(&sl)?)?, self.c3.forward(&self.ln2.forward(&sr)?)?);
        let ((x1, x2), (y1, y2)) = (halves(&x)?, halves(&y)?);
        let (x, y) = (self.c4.forward(&(x1 * y2)?)?, self.c4.forward(&(y1 * x2)?)?);
        Ok(((sl + x.broadcast_mul(&self.b_l)?)?, (sr + y.broadcast_mul(&self.b_r)?)?))
    }
}

fn mugis(w: &Weights, name: &str, n: usize, c: usize) -> candle_core::Result<Vec<Mugi>> {
    (0..n).map(|i| Mugi::load(w, &format!("{name}.{i}"), c, "seq", true)).collect()
}

fn run_all(blocks: &[Mugi], l: Tensor, r: Tensor) -> candle_core::Result<(Tensor, Tensor)> {
    blocks.iter().try_fold((l, r), |(l, r), b| b.forward(&l, &r))
}

/// `PixelShuffle(2)`: channel `4c + 2dy + dx` → pixel `(2y + dy, 2x + dx)` of channel `c`.
fn shuffle(x: &Tensor) -> candle_core::Result<Tensor> {
    let (n, c, h, w) = x.dims4()?;
    x.reshape((n, c / 4, 2, 2, h, w))?.permute((0, 1, 4, 2, 5, 3))?.contiguous()?.reshape((n, c / 4, h * 2, w * 2))
}

/// `UpsamplingBilinear2d(scale_factor=2)` (aligned corners), as two matrix products.
fn upsample2(x: &Tensor) -> candle_core::Result<Tensor> {
    let (_, c, h, w) = x.dims4()?;
    let table = |n: usize| -> candle_core::Result<Tensor> {
        let m = 2 * n;
        let mut v = vec![0f32; m * n];
        for o in 0..m {
            let s = if m > 1 { o as f64 * (n as f64 - 1.0) / (m as f64 - 1.0) } else { 0.0 };
            let i0 = (s.floor() as usize).min(n - 1);
            let i1 = (i0 + 1).min(n - 1);
            let f = (s - i0 as f64) as f32;
            v[o * n + i0] += 1.0 - f;
            v[o * n + i1] += f;
        }
        Tensor::from_vec(v, (m, n), x.device())
    };
    let (uh, uw) = (table(h)?, table(w)?);
    // [c, h, w] · uwᵀ → [c, h, 2w]; uh · → [c, 2h, 2w]
    let y = x.contiguous()?.reshape((c * h, w))?.matmul(&uw.t()?)?.reshape((c, h, 2 * w))?;
    uh.broadcast_matmul(&y)?.reshape((1, c, 2 * h, 2 * w))
}

/// VGG-19's convolutions up to relu5_1, and where its five feature maps come from.
struct Vgg {
    convs: Vec<Conv3>,
}

/// VGG-19 `features` indices of the convolutions used (input / output channels).
const VGG: [(usize, usize, usize); 13] = [
    (0, 3, 64),
    (2, 64, 64),
    (5, 64, 128),
    (7, 128, 128),
    (10, 128, 256),
    (12, 256, 256),
    (14, 256, 256),
    (16, 256, 256),
    (19, 256, 512),
    (21, 512, 512),
    (23, 512, 512),
    (25, 512, 512),
    (28, 512, 512),
];

impl Vgg {
    fn load(w: &Weights) -> candle_core::Result<Self> {
        Ok(Vgg { convs: VGG.iter().map(|(i, cin, cout)| Conv3::load(w, &format!("features.{i}"), *cin, *cout)).collect::<candle_core::Result<_>>()? })
    }
    /// relu1_1, relu2_1, relu3_1, relu4_1, relu5_1.
    fn forward(&self, x: &Tensor) -> candle_core::Result<Vec<Tensor>> {
        let mut out = Vec::with_capacity(5);
        let mut t = x.clone();
        for (k, (i, _, _)) in VGG.iter().enumerate() {
            if matches!(i, 5 | 10 | 19 | 28) {
                t = t.max_pool2d(2)?;
            }
            t = self.convs[k].forward(&t)?.relu()?;
            if matches!(i, 0 | 5 | 10 | 19 | 28) {
                out.push(t.clone());
            }
        }
        Ok(out)
    }
}

/// The loaded network and its VGG-19.
pub struct Dereflector {
    vgg: Vgg,
    // feature pyramid
    block5: Mugi,
    block4: Mugi,
    ch4: (Conv1, Mugi),
    block3: Mugi,
    ch3: (Conv1, Mugi),
    block2: Mugi,
    ch2: (Conv1, Mugi),
    block1: Mugi,
    ch1: (Conv1, Mugi, Conv1, Mugi),
    intro_conv: Conv3,
    intro_mugi: Mugi,
    ch0: Conv1,
    // U-Net
    encoders: Vec<Vec<Mugi>>,
    downs: Vec<Down>,
    middle: Vec<Mugi>,
    ups: Vec<Conv1>,
    decoders: Vec<Vec<Mugi>>,
    // the residual (LRM) is part of the checkpoint but not of the transmission and reflection used here
    ending: Conv3,
    device: Device,
}

const WIDTH: usize = 64;
const ENC: [usize; 4] = [2, 2, 4, 8];
const MIDDLE: usize = 12;
const DEC: [usize; 4] = [2, 2, 2, 2];

impl Dereflector {
    /// Load the DSRNet checkpoint (`dsrnet_l_4000_epoch33.pt`) and VGG-19's ImageNet weights (torchvision's, as
    /// `timm/vgg19.tv_in1k`'s `model.safetensors`; a zip `.pth` with `features.*` works too).
    pub fn load(dsrnet: &Path, vgg19: &Path, device: Device) -> Result<Self> {
        let _gpu = crate::gpu_lock(&device);
        let wv = Weights::open(vgg19, &device)?;
        let vgg = Vgg::load(&wv).map_err(|e| Error::Model(format!("{}: {e}", vgg19.display())))?;
        let w = Weights::open(dsrnet, &device)?;
        let load = || -> candle_core::Result<Self> {
            let m = |name: &str, c: usize| Mugi::load(&w, &format!("intro.{name}"), c, "seq", true);
            let c1 = |name: &str, cin: usize, cout: usize| Conv1::load(&w, &format!("intro.{name}"), cin, cout, true);
            let mut encoders = Vec::new();
            let mut downs = Vec::new();
            let mut c = WIDTH;
            for (i, n) in ENC.iter().enumerate() {
                encoders.push(mugis(&w, &format!("encoders.{i}"), *n, c)?);
                downs.push(Down::load(&w, &format!("downs.{i}.seq.0"), c, 2 * c)?);
                c *= 2;
            }
            let middle = mugis(&w, "middle_blks", MIDDLE, c)?;
            let mut ups = Vec::new();
            let mut decoders = Vec::new();
            for (i, n) in DEC.iter().enumerate() {
                ups.push(Conv1::load(&w, &format!("ups.{i}.seq.0"), c, 2 * c, false)?);
                c /= 2;
                decoders.push(mugis(&w, &format!("decoders.{i}"), *n, c)?);
            }
            Ok(Dereflector {
                vgg,
                block5: m("block5.0", 512)?,
                block4: m("block4.0", 512)?,
                ch4: (c1("ch_map4.0.seq.0", 1024, 1024)?, m("ch_map4.1", 256)?),
                block3: m("block3.0", 256)?,
                ch3: (c1("ch_map3.0.seq.0", 512, 512)?, m("ch_map3.1", 128)?),
                block2: m("block2.0", 128)?,
                ch2: (c1("ch_map2.0.seq.0", 256, 256)?, m("ch_map2.1", 64)?),
                block1: m("block1.0", 64)?,
                ch1: (c1("ch_map1.0.seq.0", 128, 128)?, m("ch_map1.1", 128)?, c1("ch_map1.2.seq.0", 128, 32)?, m("ch_map1.3", 32)?),
                intro_conv: Conv3::load(&w, "intro.block_intro.0.seq.0", 3, 32)?,
                intro_mugi: m("block_intro.1", 32)?,
                ch0: c1("ch_map0.seq.0", 64, WIDTH)?,
                encoders,
                downs,
                middle,
                ups,
                decoders,
                ending: Conv3::load(&w, "ending.seq.0", WIDTH, 3)?,
                device: device.clone(),
            })
        };
        load().map_err(|e| Error::Model(format!("{}: {e}", dsrnet.display())))
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// `[1, 3, H, W]` (0..1, display-encoded; sides multiples of [`MULTIPLE`]) → (transmission, reflection).
    pub fn forward(&self, x: &Tensor) -> candle_core::Result<(Tensor, Tensor)> {
        let (_, c, h, w) = x.dims4()?;
        if c != 3 || !h.is_multiple_of(MULTIPLE) || !w.is_multiple_of(MULTIPLE) || h < MULTIPLE || w < MULTIPLE {
            candle_core::bail!("the network takes RGB a multiple of {MULTIPLE} pixels in size");
        }
        let vf = self.vgg.forward(x)?;
        let [vf1, vf2, vf3, vf4, vf5] = vf.as_slice() else { candle_core::bail!("VGG features") };
        let cat = |a: &Tensor, b: &Tensor| Tensor::cat(&[a, b], 1);
        // the feature pyramid (both streams start from the same features)
        let (f5l, f5r) = self.block5.forward(vf5, vf5)?;
        let (f5l, f5r) = (upsample2(&f5l)?, upsample2(&f5r)?);
        let (f4l, f4r) = self.block4.forward(vf4, vf4)?;
        let (l, r) = (shuffle(&self.ch4.0.forward(&cat(&f5l, &f4l)?)?)?, shuffle(&self.ch4.0.forward(&cat(&f5r, &f4r)?)?)?);
        let (f4l, f4r) = self.ch4.1.forward(&l, &r)?;
        let (f3l, f3r) = self.block3.forward(vf3, vf3)?;
        let (l, r) = (shuffle(&self.ch3.0.forward(&cat(&f4l, &f3l)?)?)?, shuffle(&self.ch3.0.forward(&cat(&f4r, &f3r)?)?)?);
        let (f3l, f3r) = self.ch3.1.forward(&l, &r)?;
        let (f2l, f2r) = self.block2.forward(vf2, vf2)?;
        let (l, r) = (shuffle(&self.ch2.0.forward(&cat(&f3l, &f2l)?)?)?, shuffle(&self.ch2.0.forward(&cat(&f3r, &f2r)?)?)?);
        let (f2l, f2r) = self.ch2.1.forward(&l, &r)?;
        let (f1l, f1r) = self.block1.forward(vf1, vf1)?;
        let (l, r) = (self.ch1.0.forward(&cat(&f2l, &f1l)?)?, self.ch1.0.forward(&cat(&f2r, &f1r)?)?);
        let (l, r) = self.ch1.1.forward(&l, &r)?;
        let (l, r) = (self.ch1.2.forward(&l)?, self.ch1.2.forward(&r)?);
        let (f1l, f1r) = self.ch1.3.forward(&l, &r)?;
        let i0 = self.intro_conv.forward(x)?;
        let (f0l, f0r) = self.intro_mugi.forward(&i0, &i0)?;
        let (mut l, mut r) = (self.ch0.forward(&cat(&f1l, &f0l)?)?, self.ch0.forward(&cat(&f1r, &f0r)?)?);
        // the U-Net
        let mut skips = Vec::with_capacity(ENC.len());
        for (enc, down) in self.encoders.iter().zip(&self.downs) {
            (l, r) = run_all(enc, l, r)?;
            skips.push((l.clone(), r.clone()));
            (l, r) = (down.forward(&l)?, down.forward(&r)?);
        }
        (l, r) = run_all(&self.middle, l, r)?;
        for ((dec, up), (sl, sr)) in self.decoders.iter().zip(&self.ups).zip(skips.iter().rev()) {
            (l, r) = ((shuffle(&up.forward(&l)?)? + sl)?, (shuffle(&up.forward(&r)?)? + sr)?);
            (l, r) = run_all(dec, l, r)?;
        }
        Ok((self.ending.forward(&l)?, self.ending.forward(&r)?))
    }

    /// Separate `img` (display-encoded RGB, 0..1, any size of at least 16 × 16, at most 4096²) into the scene behind
    /// the glass and the reflection; padded (mirrored) to a multiple of [`MULTIPLE`] for the network.
    pub fn separate(&self, img: &Rgb32f) -> Result<(Rgb32f, Rgb32f)> {
        let (w, h) = (img.width, img.height);
        if w < MULTIPLE || h < MULTIPLE {
            return Err(Error::Input("the picture is too small".into()));
        }
        if w.saturating_mul(h) > 4096 * 4096 {
            return Err(Error::Input("the picture is too large to separate in one go".into()));
        }
        let (pw, ph) = (w.div_ceil(MULTIPLE) * MULTIPLE, h.div_ceil(MULTIPLE) * MULTIPLE);
        let n = pw * ph;
        let mirror = |i: usize, len: usize| if i < len { i } else { (2 * len).saturating_sub(i + 2).min(len - 1) };
        let mut input = vec![0f32; 3 * n];
        for y in 0..ph {
            for x in 0..pw {
                let p = img.data.get(mirror(y, h) * w + mirror(x, w)).copied().unwrap_or([0.0; 3]);
                for c in 0..3 {
                    input[c * n + y * pw + x] = p[c].clamp(0.0, 1.0);
                }
            }
        }
        let (t, r) = {
            let _gpu = crate::gpu_lock(&self.device);
            let x = Tensor::from_vec(input, (1, 3, ph, pw), &self.device)?;
            let (t, r) = self.forward(&x)?;
            (t.flatten_all()?.to_vec1::<f32>()?, r.flatten_all()?.to_vec1::<f32>()?)
        };
        let pick = |v: &[f32]| Rgb32f::from_fn(w, h, |x, y| [0, 1, 2].map(|c| v.get(c * n + y * pw + x).copied().unwrap_or(0.0)));
        Ok((pick(&t), pick(&r)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec4(t: &Tensor) -> Vec<f32> {
        t.flatten_all().unwrap().to_vec1::<f32>().unwrap()
    }

    #[test]
    fn upsampling_aligns_the_corners_and_shuffle_interleaves() {
        let x = Tensor::from_vec(vec![0f32, 1.0, 2.0, 3.0], (1, 1, 2, 2), &Device::Cpu).unwrap();
        let y = vec4(&upsample2(&x).unwrap());
        // corners stay, the middle is interpolated over (n - 1) / (2n - 1)
        assert_eq!((y[0], y[3], y[12], y[15]), (0.0, 1.0, 2.0, 3.0));
        assert!((y[1] - 1.0 / 3.0).abs() < 1e-6, "{y:?}");
        let s = Tensor::from_vec((0..8).map(|v| v as f32).collect::<Vec<_>>(), (1, 8, 1, 1), &Device::Cpu).unwrap();
        // channels 0..3 make the first output channel's 2 × 2 block
        assert_eq!(vec4(&shuffle(&s).unwrap()), vec![0., 1., 2., 3., 4., 5., 6., 7.]);
    }

    #[test]
    fn depthwise_and_strided_layers_match_plain_loops() {
        let (c, h, w) = (3usize, 4usize, 6usize);
        let x: Vec<f32> = (0..c * h * w).map(|i| ((i * 37) % 11) as f32 / 11.0 - 0.4).collect();
        let k: Vec<f32> = (0..c * 9).map(|i| ((i * 13) % 7) as f32 / 7.0 - 0.5).collect();
        let xt = Tensor::from_slice(&x, (1, c, h, w), &Device::Cpu).unwrap();
        let taps =
            (0..9).map(|i| Tensor::from_vec((0..c).map(|ch| k[ch * 9 + i]).collect::<Vec<_>>(), (1, c, 1, 1), &Device::Cpu).unwrap()).collect();
        let dw = DwConv3 { taps, b: Tensor::zeros((1, c, 1, 1), DType::F32, &Device::Cpu).unwrap() };
        let got = vec4(&dw.forward(&xt).unwrap());
        for ch in 0..c {
            for y in 0..h {
                for xx in 0..w {
                    let mut s = 0f32;
                    for ky in 0..3 {
                        for kx in 0..3 {
                            let (yy, xs) = (y as isize + ky as isize - 1, xx as isize + kx as isize - 1);
                            if yy >= 0 && xs >= 0 && (yy as usize) < h && (xs as usize) < w {
                                s += x[(ch * h + yy as usize) * w + xs as usize] * k[ch * 9 + ky * 3 + kx];
                            }
                        }
                    }
                    assert!((s - got[(ch * h + y) * w + xx]).abs() < 1e-5);
                }
            }
        }
        let kd: Vec<f32> = (0..2 * c * 4).map(|i| ((i * 5) % 9) as f32 / 9.0 - 0.3).collect();
        let kt = Tensor::from_slice(&kd, (2, c, 2, 2), &Device::Cpu).unwrap();
        let mut d = Down { taps: Vec::new(), b: Tensor::zeros((1, 2, 1, 1), DType::F32, &Device::Cpu).unwrap() };
        for a in 0..2 {
            for b in 0..2 {
                d.taps.push(kt.narrow(2, a, 1).unwrap().narrow(3, b, 1).unwrap().contiguous().unwrap().reshape((2, c)).unwrap());
            }
        }
        let got = vec4(&d.forward(&xt).unwrap());
        for co in 0..2 {
            for y in 0..h / 2 {
                for xx in 0..w / 2 {
                    let mut s = 0f32;
                    for ci in 0..c {
                        for a in 0..2 {
                            for b in 0..2 {
                                s += x[(ci * h + 2 * y + a) * w + 2 * xx + b] * kd[((co * c + ci) * 2 + a) * 2 + b];
                            }
                        }
                    }
                    assert!((s - got[(co * (h / 2) + y) * (w / 2) + xx]).abs() < 1e-5);
                }
            }
        }
    }

    /// Opt-in, against the Python reference (DSRNet's own modules on the same weights):
    /// `LC_DSRNET=<dsrnet_l_4000_epoch33.pt> LC_VGG19=<vgg19 model.safetensors> LC_DSRNET_REF=<ref.safetensors> cargo test
    /// -p lightcraft-enhance --release -- --ignored dsrnet_matches --nocapture`.
    #[test]
    #[ignore]
    fn dsrnet_matches_the_reference() {
        let (Ok(p), Ok(v), Ok(r)) = (std::env::var("LC_DSRNET"), std::env::var("LC_VGG19"), std::env::var("LC_DSRNET_REF")) else { return };
        let refs = candle_core::safetensors::load(&r, &Device::Cpu).unwrap();
        let mut devices = vec![Device::Cpu];
        if !crate::best_device().is_cpu() {
            devices.push(crate::best_device());
        }
        for d in devices {
            let m = Dereflector::load(Path::new(&p), Path::new(&v), d.clone()).unwrap();
            let _gpu = crate::gpu_lock(&d);
            let x = refs["input"].to_device(&d).unwrap();
            let f = m.vgg.forward(&x).unwrap();
            let e = vec4(&f[0]).iter().zip(vec4(&refs["vf1"])).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            eprintln!("{d:?}: vgg relu1_1 worst {e}");
            let t0 = std::time::Instant::now();
            let (t, rr) = m.forward(&x).unwrap();
            eprintln!("{d:?}: {:?} in {:?}", x.dims(), t0.elapsed());
            for (got, want) in [(vec4(&t), vec4(&refs["t"])), (vec4(&rr), vec4(&refs["r"]))] {
                let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
                let mean = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).sum::<f32>() / want.len() as f32;
                eprintln!("worst {worst}, mean {mean}");
                assert!(worst < 5e-3, "{d:?}: {worst}");
            }
        }
    }
}
