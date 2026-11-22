//! Relative depth with Depth Anything V2 Small (Yang et al., 2024; Apache-2.0): DINOv2 ViT-S/14 and a DPT head,
//! from candle-transformers (vendored as [`crate::dinov2`] and [`crate::depth_anything_v2`]).
//!
//! The model gives relative inverse depth (bigger is nearer). [`DepthModel::estimate`] turns it into a distance-like
//! map: 0 is the nearest thing in the photo, 1 the farthest, at about [`OUT_EDGE`] pixels on the long edge.

use std::path::Path;
use std::sync::Arc;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::VarBuilder;
use lightcraft_raster::Rgb32f;

use crate::depth_anything_v2::{DepthAnythingV2, DepthAnythingV2Config};
use crate::{Error, Result};

/// Side of the square model input (a multiple of the 14-pixel patch).
pub const INPUT: usize = 518;
/// Long edge of the depth maps [`DepthModel::estimate`] gives.
pub const OUT_EDGE: usize = 384;

const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A depth map: `width × height` values, 0 nearest … 1 farthest, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Depth {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

pub struct DepthModel {
    model: DepthAnythingV2,
    device: Device,
}

impl DepthModel {
    pub fn load(path: &Path, device: Device) -> Result<Self> {
        let tensors = candle_core::safetensors::load(path, &Device::Cpu).map_err(|e| Error::Model(format!("{}: {e}", path.display())))?;
        let vb = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let dino = crate::dinov2::vit_small(vb.pp("pretrained"))?;
        let model = DepthAnythingV2::new(Arc::new(dino), DepthAnythingV2Config::vit_small(), vb)?;
        Ok(DepthModel { model, device })
    }

    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Depth of `img` (display-encoded RGB, 0..1).
    pub fn estimate(&self, img: &Rgb32f) -> Result<Depth> {
        if img.width < 2 || img.height < 2 {
            return Err(Error::Input("the picture is too small".into()));
        }
        let n = INPUT * INPUT;
        let mut input = vec![0f32; 3 * n];
        for y in 0..INPUT {
            for x in 0..INPUT {
                let p = sample(img, (x as f32 + 0.5) / INPUT as f32, (y as f32 + 0.5) / INPUT as f32);
                for c in 0..3 {
                    input[c * n + y * INPUT + x] = (p[c].clamp(0.0, 1.0) - MEAN[c]) / STD[c];
                }
            }
        }
        let t = Tensor::from_vec(input, (1, 3, INPUT, INPUT), &self.device)?;
        let out = self.model.forward(&t)?;
        let (_, _, oh, ow) = out.dims4()?;
        let disp = out.flatten_all()?.to_dtype(DType::F32)?.to_vec1::<f32>()?;
        // robust range: 1st..99th percentile
        let mut sorted: Vec<f32> = disp.iter().copied().filter(|v| v.is_finite()).collect();
        if sorted.is_empty() {
            return Err(Error::Model("the depth model gave no usable values".into()));
        }
        sorted.sort_by(f32::total_cmp);
        let lo = sorted[sorted.len() / 100];
        let hi = sorted[sorted.len() * 99 / 100];
        let span = (hi - lo).max(1e-6);
        let long = img.width.max(img.height) as f32;
        let (w, h) = (
            ((img.width as f32 / long * OUT_EDGE as f32).round() as usize).max(1),
            ((img.height as f32 / long * OUT_EDGE as f32).round() as usize).max(1),
        );
        let mut data = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = ((x as f32 + 0.5) / w as f32 * ow as f32 - 0.5, (y as f32 + 0.5) / h as f32 * oh as f32 - 0.5);
                let d = bilinear1(&disp, ow, oh, u, v);
                data.push((1.0 - (d - lo) / span).clamp(0.0, 1.0));
            }
        }
        Ok(Depth { width: w, height: h, data })
    }
}

fn sample(img: &Rgb32f, u: f32, v: f32) -> [f32; 3] {
    let x = ((u * img.width as f32) as usize).min(img.width - 1);
    let y = ((v * img.height as f32) as usize).min(img.height - 1);
    img.data[y * img.width + x]
}

fn bilinear1(d: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: isize, yi: isize| d.get(yi.clamp(0, h as isize - 1) as usize * w + xi.clamp(0, w as isize - 1) as usize).copied().unwrap_or(0.0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    (at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx) * (1.0 - fy) + (at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx) * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in: `LC_DEPTH=<depth_anything_v2_vits.safetensors> cargo test -p lightcraft-enhance --release -- --ignored depth`
    #[test]
    #[ignore]
    fn depth_of_a_floor_grows_upwards() {
        let Ok(p) = std::env::var("LC_DEPTH") else { return };
        let m = DepthModel::load(Path::new(&p), crate::best_device()).unwrap();
        // a checkered floor in perspective under a plain sky: the bottom is near, the top far
        let (w, h) = (600, 400);
        let mut img = Rgb32f::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let p = if y < 150 {
                    [0.5, 0.7, 0.95]
                } else {
                    let z = 150.0 / (y as f32 - 140.0);
                    let u = (x as f32 - 300.0) * z / 40.0;
                    if ((u.floor() as i32) + (z * 8.0) as i32) % 2 == 0 { [0.8, 0.8, 0.75] } else { [0.2, 0.2, 0.25] }
                };
                img.data[y * w + x] = p;
            }
        }
        let t = std::time::Instant::now();
        let d = m.estimate(&img).unwrap();
        eprintln!("depth {}×{} in {:?}", d.width, d.height, t.elapsed());
        let at = |fy: f32| d.data[(fy * d.height as f32) as usize * d.width + d.width / 2];
        assert!(at(0.95) < at(0.5), "near floor {} vs middle {}", at(0.95), at(0.5));
    }
}
