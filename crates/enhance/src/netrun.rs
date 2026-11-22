//! A [`Net`] (the U-Net denoisers' layers, read from ONNX by [`crate::onnx`]) run on candle: Metal on macOS, else
//! the CPU. It is a [`TileRunner`], so `lightcraft_denoise_core::denoise_bayer` drives it over a whole mosaic.

use std::sync::Mutex;

use candle_core::{Device, Tensor};
use lightcraft_denoise_core::run::{Error, TileRunner};

use crate::net::{Net, Op};

enum Layer {
    Conv { src: usize, src2: Option<usize>, w: Tensor, b: Tensor, pad: usize, leaky: Option<f32> },
    ConvT2 { src: usize, w: Tensor, b: Tensor, leaky: Option<f32> },
    MaxPool2 { src: usize },
    DepthToSpace2 { src: usize },
}

/// A network with its weights on a device.
pub struct NetRunner {
    layers: Vec<Layer>,
    in_channels: usize,
    out_channels: usize,
    tile: usize,
    device: Device,
    /// One tile at a time on the GPU (its queue serialises them anyway).
    gate: Mutex<()>,
}

fn rt(e: candle_core::Error) -> Error {
    Error::Runtime(e.to_string())
}

fn leaky(x: Tensor, alpha: Option<f32>) -> candle_core::Result<Tensor> {
    match alpha {
        // max(x, αx) for 0 ≤ α ≤ 1
        Some(a) if (0.0..=1.0).contains(&a) => x.maximum(&(&x * f64::from(a))?),
        Some(a) => {
            let pos = x.relu()?;
            let neg = (x.neg()?.relu()? * f64::from(-a))?;
            pos + neg
        }
        None => Ok(x),
    }
}

impl NetRunner {
    /// Put `net`'s weights on `device` for `tile × tile`-cell tiles.
    pub fn new(net: &Net, tile: usize, device: Device) -> Result<Self, Error> {
        if !net.fits_tile(tile) {
            return Err(Error::Load(format!("the network does not run on tiles of {tile} cells")));
        }
        let mut layers = Vec::with_capacity(net.ops().len());
        for op in net.ops() {
            layers.push(match op {
                Op::Conv(c) => Layer::Conv {
                    src: c.src,
                    src2: c.src2,
                    w: Tensor::from_slice(&c.weight, (c.cout, c.cin, c.k, c.k), &device).map_err(rt)?,
                    b: Tensor::from_slice(&c.bias, (1, c.cout, 1, 1), &device).map_err(rt)?,
                    pad: (c.k - 1) / 2,
                    leaky: c.leaky,
                },
                Op::ConvT2(c) => Layer::ConvT2 {
                    src: c.src,
                    // [cin][cout·4] → [cout·4][cin]: one matrix product, then depth-to-space
                    w: Tensor::from_slice(&c.weight, (c.cin, c.cout * 4), &device).and_then(|w| w.t()?.contiguous()).map_err(rt)?,
                    b: Tensor::from_slice(&c.bias, (1, c.cout, 1, 1), &device).map_err(rt)?,
                    leaky: c.leaky,
                },
                Op::MaxPool2 { src } => Layer::MaxPool2 { src: *src },
                Op::DepthToSpace2 { src } => Layer::DepthToSpace2 { src: *src },
            });
        }
        Ok(NetRunner { layers, in_channels: net.in_channels, out_channels: net.out_channels(), tile, device, gate: Mutex::new(()) })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    fn forward(&self, x: Tensor) -> candle_core::Result<Tensor> {
        let mut t: Vec<Tensor> = vec![x];
        for l in &self.layers {
            let get = |i: usize| t.get(i).cloned().ok_or_else(|| candle_core::Error::Msg(format!("tensor {i} does not exist yet")));
            let out = match l {
                Layer::Conv { src, src2, w, b, pad, leaky: a } => {
                    let x = match src2 {
                        Some(s2) => Tensor::cat(&[get(*src)?.contiguous()?, get(*s2)?.contiguous()?], 1)?,
                        None => get(*src)?,
                    };
                    leaky(conv2d(&x, w, *pad)?.broadcast_add(b)?, *a)?
                }
                Layer::ConvT2 { src, w, b, leaky: a } => {
                    // a 2 × 2, stride-2 transposed convolution: every input pixel becomes its own 2 × 2 block
                    let x = get(*src)?.contiguous()?;
                    let (n, cin, h, wd) = x.dims4()?;
                    if n != 1 {
                        candle_core::bail!("one tile at a time");
                    }
                    let y = w.matmul(&x.reshape((cin, h * wd))?)?;
                    let cout = y.dim(0)? / 4;
                    let y = y.reshape((1, cout, 2, 2, h, wd))?.permute((0, 1, 4, 2, 5, 3))?.contiguous()?.reshape((1, cout, h * 2, wd * 2))?;
                    leaky(y.broadcast_add(b)?, *a)?
                }
                Layer::MaxPool2 { src } => get(*src)?.contiguous()?.max_pool2d(2)?,
                Layer::DepthToSpace2 { src } => {
                    let x = get(*src)?;
                    let (n, c, h, w) = x.dims4()?;
                    // CRD: channel 4c + 2dy + dx → (2y + dy, 2x + dx) of channel c
                    x.reshape((n, c / 4, 2, 2, h, w))?.permute((0, 1, 4, 2, 5, 3))?.contiguous()?.reshape((n, c / 4, h * 2, w * 2))?
                }
            };
            t.push(out);
        }
        t.pop().ok_or_else(|| candle_core::Error::Msg("empty network".into()))
    }
}

/// A stride-1 convolution of one picture (`[1, cin, h, w]`) padded by `pad`, as one matrix product per kernel tap.
///
/// The padded picture is flattened row by row; tap `(ky, kx)` then reads the same flat array shifted by
/// `ky · width + kx`, so each tap is a plain slice (no im2col copy) and the result is computed on the padded width,
/// whose extra columns are dropped. This is ~3× faster than candle 0.9.2's Metal convolution (its im2col kernel), and
/// right on the CPU, where candle's tiled kernel is wrong when the channel count equals the width
/// (`conv2d_is_right_when_the_channels_equal_the_width`).
pub fn conv2d(x: &Tensor, w: &Tensor, pad: usize) -> candle_core::Result<Tensor> {
    let (n, cin, h, wd) = x.dims4()?;
    let (cout, cin_k, kh, kw) = w.dims4()?;
    if cin != cin_k || h + 2 * pad < kh || wd + 2 * pad < kw {
        candle_core::bail!("convolution shapes do not match");
    }
    if n != 1 {
        let one: Vec<Tensor> = (0..n).map(|i| conv2d(&x.narrow(0, i, 1)?, w, pad)).collect::<candle_core::Result<_>>()?;
        return Tensor::cat(&one, 0);
    }
    if kh == 1 && kw == 1 && pad == 0 {
        return w.reshape((cout, cin))?.matmul(&x.contiguous()?.reshape((cin, h * wd))?)?.reshape((1, cout, h, wd));
    }
    let (hp, wp) = (h + 2 * pad, wd + 2 * pad);
    let (oh, ow) = (hp + 1 - kh, wp + 1 - kw);
    // pixels × channels, one more zero row below so the last tap's slice stays inside: each tap's rows are then one
    // contiguous block
    let flat = x.pad_with_zeros(2, pad, pad + 1)?.pad_with_zeros(3, pad, pad)?.reshape((cin, (hp + 1) * wp))?.t()?.contiguous()?;
    // [ky][kx][cin][cout]
    let taps = w.permute((2, 3, 1, 0))?.contiguous()?;
    let len = oh * wp;
    let mut acc: Option<Tensor> = None;
    for ky in 0..kh {
        for kx in 0..kw {
            let y = flat.narrow(0, ky * wp + kx, len)?.matmul(&taps.get(ky)?.get(kx)?)?;
            acc = Some(match acc {
                Some(a) => (a + y)?,
                None => y,
            });
        }
    }
    let acc = acc.ok_or_else(|| candle_core::Error::Msg("empty kernel".into()))?;
    // [oh·wp, cout] → [1, cout, oh, ow]
    acc.reshape((oh, wp, cout))?.narrow(1, 0, ow)?.permute((2, 0, 1))?.contiguous()?.reshape((1, cout, oh, ow))
}

impl TileRunner for NetRunner {
    fn run(&self, input: &[f32]) -> Result<Vec<f32>, Error> {
        let t = self.tile;
        if input.len() != self.in_channels * t * t {
            return Err(Error::Input("the tile does not fit the network".into()));
        }
        let _one = if self.device.is_cpu() { None } else { Some(self.gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner)) };
        let x = Tensor::from_slice(input, (1, self.in_channels, t, t), &self.device).map_err(rt)?;
        let y = self.forward(x).map_err(rt)?;
        let dims = y.dims().to_vec();
        if dims != [1, self.out_channels, 2 * t, 2 * t] {
            return Err(Error::Model(format!("an output of shape {dims:?}")));
        }
        y.flatten_all().and_then(|v| v.to_vec1::<f32>()).map_err(rt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::{Conv, ConvT2};

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

    fn worst(net: &Net, tile: usize, input: &[f32], d: &Device) -> f32 {
        let want = crate::reference::run(net, tile, input).unwrap();
        let got = NetRunner::new(net, tile, d.clone()).unwrap().run(input).unwrap();
        assert_eq!(got.len(), want.len());
        got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max)
    }

    #[test]
    fn candle_runner_matches_the_reference_interpreter() {
        for (depth, ch) in [(0, 8), (1, 4), (2, 8)] {
            let bytes = crate::synthetic::unet_onnx(32, ch, depth, 7);
            let p = std::env::temp_dir().join(format!("lc-enhance-netrun-{depth}-{}.onnx", std::process::id()));
            std::fs::write(&p, bytes).unwrap();
            let net = crate::onnx::read(&p).unwrap();
            let _ = std::fs::remove_file(&p);
            let input: Vec<f32> = (0..net.in_channels * 32 * 32).map(|i| ((i * 37 % 101) as f32) / 101.0).collect();
            for d in devices() {
                let w = worst(&net, 32, &input, &d);
                assert!(w < 1e-3, "depth {depth} ch {ch}: worst difference {w} on {d:?}");
            }
        }
    }

    #[test]
    fn each_layer_kind_matches_the_reference() {
        let conv = |src, src2, cin, cout, k, s| {
            Op::Conv(Conv { src, src2, cin, cout, k, weight: rnd(cin * cout * k * k, s), bias: rnd(cout, s + 1), leaky: Some(0.1) })
        };
        let up = |src, cin, s| Op::ConvT2(ConvT2 { src, cin, cout: 12, weight: rnd(cin * 12 * 4, s), bias: rnd(12, s + 2), leaky: None });
        let nets = [
            ("1×1 + depth-to-space", vec![conv(0, None, 4, 12, 1, 5), Op::DepthToSpace2 { src: 1 }]),
            ("pool + up", vec![Op::MaxPool2 { src: 0 }, up(1, 4, 9), Op::DepthToSpace2 { src: 2 }]),
            ("skip join", vec![conv(0, None, 4, 4, 3, 3), conv(1, Some(0), 8, 12, 3, 4), Op::DepthToSpace2 { src: 2 }]),
            // 8 channels 8 wide: candle 0.9.2's own CPU convolution gets this wrong
            (
                "8 × 8 × 8",
                vec![Op::MaxPool2 { src: 0 }, conv(1, None, 4, 8, 3, 3), conv(2, None, 8, 8, 3, 4), up(3, 8, 5), Op::DepthToSpace2 { src: 4 }],
            ),
        ];
        let input = rnd(4 * 16 * 16, 3);
        for (name, ops) in nets {
            let net = Net::new(4, ops).unwrap();
            for d in devices() {
                let w = worst(&net, 16, &input, &d);
                assert!(w < 1e-4, "{name} on {d:?}: {w}");
            }
        }
    }

    #[test]
    fn conv2d_is_right_when_the_channels_equal_the_width() {
        for (cin, side) in [(8usize, 8usize), (8, 16), (16, 8), (32, 32)] {
            let cout = 8;
            let x = rnd(cin * side * side, 1);
            let w = rnd(cout * cin * 9, 2);
            for d in devices() {
                let xt = Tensor::from_slice(&x, (1, cin, side, side), &d).unwrap();
                let wt = Tensor::from_slice(&w, (cout, cin, 3, 3), &d).unwrap();
                let got = conv2d(&xt, &wt, 1).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
                let mut worst = 0f32;
                for co in 0..cout {
                    for y in 0..side {
                        for xx in 0..side {
                            let mut s = 0f32;
                            for ci in 0..cin {
                                for ky in 0..3 {
                                    for kx in 0..3 {
                                        let (yy, xs) = (y as isize + ky as isize - 1, xx as isize + kx as isize - 1);
                                        if yy >= 0 && xs >= 0 && (yy as usize) < side && (xs as usize) < side {
                                            s += x[(ci * side + yy as usize) * side + xs as usize] * w[((co * cin + ci) * 3 + ky) * 3 + kx];
                                        }
                                    }
                                }
                            }
                            worst = worst.max((s - got[(co * side + y) * side + xx]).abs());
                        }
                    }
                }
                assert!(worst < 1e-4, "cin {cin} side {side} on {d:?}: {worst}");
            }
        }
    }
}
