//! A photo's depth map (Lens Blur, Depth Range masks), kept with its settings like an AI mask's segmentation: a
//! small grid of depths over the whole photo as the masks see it (uncropped, oriented), 0 the nearest thing, 1 the
//! farthest, quantised to bytes, deflated and base64-encoded.

use serde::{Deserialize, Serialize};

/// Largest grid accepted (cells).
const MAX_CELLS: usize = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthMap {
    pub width: u32,
    pub height: u32,
    pub data: String,
}

impl DepthMap {
    /// From `width × height` depths 0..1 (clamped).
    pub fn from_values(width: usize, height: usize, values: &[f32]) -> DepthMap {
        let q: Vec<u8> = values.iter().take(width.saturating_mul(height)).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        let z = miniz_oxide::deflate::compress_to_vec(&q, 8);
        DepthMap { width: width as u32, height: height as u32, data: crate::segmask::base64_encode(&z) }
    }

    /// The decoded grid; `None` when damaged.
    pub fn grid(&self) -> Option<DepthGrid> {
        let (w, h) = (self.width as usize, self.height as usize);
        let n = w.checked_mul(h).filter(|&n| n > 0 && n <= MAX_CELLS)?;
        let z = crate::segmask::base64_decode(&self.data)?;
        let q = miniz_oxide::inflate::decompress_to_vec_with_limit(&z, n).ok()?;
        (q.len() == n).then(|| DepthGrid { width: w, height: h, data: q.into_iter().map(|v| v as f32 / 255.0).collect() })
    }
}

/// A decoded depth map.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthGrid {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl DepthGrid {
    /// The depth at normalised photo point `(x, y)` (bilinear; outside the photo: its edge).
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let (w, h) = (self.width as isize, self.height as isize);
        if w == 0 || h == 0 {
            return 0.5;
        }
        let fx = if x.is_finite() { x * w as f64 - 0.5 } else { 0.0 };
        let fy = if y.is_finite() { y * h as f64 - 0.5 } else { 0.0 };
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let at = |xi: f64, yi: f64| {
            let xi = (xi as isize).clamp(0, w - 1) as usize;
            let yi = (yi as isize).clamp(0, h - 1) as usize;
            self.data.get(yi * self.width + xi).copied().unwrap_or(0.5)
        };
        let a = at(x0, y0) * (1.0 - tx) + at(x0 + 1.0, y0) * tx;
        let b = at(x0, y0 + 1.0) * (1.0 - tx) + at(x0 + 1.0, y0 + 1.0) * tx;
        a * (1.0 - ty) + b * ty
    }

    /// Histogram of the depths (`bins` buckets, fractions of the photo), for the focal-range and depth-range
    /// pickers.
    pub fn histogram(&self, bins: usize) -> Vec<f32> {
        let bins = bins.clamp(1, 1024);
        let mut h = vec![0f32; bins];
        for v in &self.data {
            let b = ((v.clamp(0.0, 1.0) * bins as f32) as usize).min(bins - 1);
            h[b] += 1.0;
        }
        let n = self.data.len().max(1) as f32;
        h.iter_mut().for_each(|v| *v /= n);
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_depth_map_round_trips_and_samples() {
        let (w, h) = (8, 4);
        let v: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 / (w - 1) as f32).collect();
        let m = DepthMap::from_values(w, h, &v);
        let g = m.grid().unwrap();
        assert_eq!((g.width, g.height), (w, h));
        assert!((g.sample(0.0, 0.5) - 0.0).abs() < 0.01);
        assert!((g.sample(1.0, 0.5) - 1.0).abs() < 0.01);
        assert!((g.sample(0.5, 0.5) - 0.5).abs() < 0.08);
        assert!((g.histogram(4).iter().sum::<f32>() - 1.0).abs() < 1e-5);
        // damaged or hostile data is no map, never a panic
        assert!(DepthMap { width: 8, height: 4, data: "!!".into() }.grid().is_none());
        assert!(DepthMap { width: u32::MAX, height: u32::MAX, data: m.data.clone() }.grid().is_none());
        assert!(DepthMap { width: 4, height: 4, data: m.data }.grid().is_none());
        assert_eq!(g.sample(f64::NAN, f64::INFINITY), g.sample(0.0, 0.0));
    }
}
