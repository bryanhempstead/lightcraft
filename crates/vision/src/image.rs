//! The 8-bit RGB pictures the models read, and the few resampling steps they need.

/// An sRGB picture, 3 bytes a pixel, row by row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rgb8 {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Rgb8 {
    /// `None` when `data` doesn't hold `width × height` pixels.
    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Option<Rgb8> {
        (width.checked_mul(height)?.checked_mul(3)? == data.len()).then_some(Rgb8 { width, height, data })
    }

    /// From RGBA bytes (alpha dropped).
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Option<Rgb8> {
        if width.checked_mul(height)?.checked_mul(4)? != rgba.len() {
            return None;
        }
        let data = rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Some(Rgb8 { width, height, data })
    }

    fn px(&self, x: usize, y: usize) -> [f32; 3] {
        let i = (y * self.width + x) * 3;
        match self.data.get(i..i + 3) {
            Some(p) => [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])],
            None => [0.0; 3],
        }
    }

    /// Shrunk (box filter) so its long edge is at most `edge`; returned as is when it already fits. Also gives the
    /// scale from the result back to `self` (≥ 1).
    pub fn fit(&self, edge: usize) -> (Rgb8, f32) {
        let long = self.width.max(self.height);
        if long <= edge || edge == 0 || self.width == 0 || self.height == 0 {
            return (self.clone(), 1.0);
        }
        let s = edge as f64 / long as f64;
        let (w, h) = (((self.width as f64 * s).round() as usize).max(1), ((self.height as f64 * s).round() as usize).max(1));
        (self.resize_area(w, h), self.width as f32 / w as f32)
    }

    /// Box-filtered resize (for shrinking).
    pub fn resize_area(&self, w: usize, h: usize) -> Rgb8 {
        let mut out = vec![0u8; w * h * 3];
        if self.width == 0 || self.height == 0 {
            return Rgb8 { width: w, height: h, data: out };
        }
        for y in 0..h {
            let (y0, y1) = (y * self.height / h, ((y + 1) * self.height / h).max(y * self.height / h + 1).min(self.height));
            for x in 0..w {
                let (x0, x1) = (x * self.width / w, ((x + 1) * self.width / w).max(x * self.width / w + 1).min(self.width));
                let mut acc = [0f32; 3];
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        let p = self.px(xx, yy);
                        for c in 0..3 {
                            acc[c] += p[c];
                        }
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)).max(1) as f32;
                if let Some(o) = out.get_mut((y * w + x) * 3..(y * w + x) * 3 + 3) {
                    for c in 0..3 {
                        o[c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Rgb8 { width: w, height: h, data: out }
    }

    /// Bilinear sample at `(x, y)` (pixel centres at integers); outside the picture is black (OpenCV's constant
    /// border).
    pub fn bilinear(&self, x: f32, y: f32) -> [f32; 3] {
        if !x.is_finite() || !y.is_finite() {
            return [0.0; 3];
        }
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let mut acc = [0f32; 3];
        for (dy, wy) in [(0.0, 1.0 - fy), (1.0, fy)] {
            for (dx, wx) in [(0.0, 1.0 - fx), (1.0, fx)] {
                let (sx, sy) = (x0 + dx, y0 + dy);
                if sx < 0.0 || sy < 0.0 || sx >= self.width as f32 || sy >= self.height as f32 {
                    continue;
                }
                let p = self.px(sx as usize, sy as usize);
                for c in 0..3 {
                    acc[c] += p[c] * wx * wy;
                }
            }
        }
        acc
    }

    /// The picture through the affine map `m` (output pixel → `m⁻¹` → input), `w × h` pixels: `m` maps input
    /// coordinates to output ones (`[a, b, c, d, e, f]`: `x' = a x + b y + c`, `y' = d x + e y + f`), as OpenCV's
    /// `warpAffine` takes it. `None` when `m` can't be inverted.
    pub fn warp_affine(&self, m: [f32; 6], w: usize, h: usize) -> Option<Rgb8> {
        let [a, b, c, d, e, f] = m.map(f64::from);
        let det = a * e - b * d;
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        // inverse
        let (ia, ib, id, ie) = (e / det, -b / det, -d / det, a / det);
        let (ic, if_) = (-(ia * c + ib * f), -(id * c + ie * f));
        let mut out = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let (xf, yf) = (x as f64, y as f64);
                let sx = ia * xf + ib * yf + ic;
                let sy = id * xf + ie * yf + if_;
                let p = self.bilinear(sx as f32, sy as f32);
                if let Some(o) = out.get_mut((y * w + x) * 3..(y * w + x) * 3 + 3) {
                    for k in 0..3 {
                        o[k] = p[k].round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Some(Rgb8 { width: w, height: h, data: out })
    }

    /// A crop (clipped to the picture; `None` when nothing is left).
    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Option<Rgb8> {
        let (x1, y1) = (x.saturating_add(w).min(self.width), y.saturating_add(h).min(self.height));
        if x >= x1 || y >= y1 {
            return None;
        }
        let mut data = Vec::with_capacity((x1 - x) * (y1 - y) * 3);
        for yy in y..y1 {
            data.extend_from_slice(self.data.get((yy * self.width + x) * 3..(yy * self.width + x1) * 3)?);
        }
        Some(Rgb8 { width: x1 - x, height: y1 - y, data })
    }

    /// Luminance (Rec. 601 weights on the encoded values, 0..1) of every pixel.
    pub fn luma(&self) -> Vec<f32> {
        self.data.as_chunks::<3>().0.iter().map(|p| (0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2])) / 255.0).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitting_and_warping_stay_in_bounds() {
        let img = Rgb8::new(10, 4, (0..120).map(|v| v as u8).collect()).unwrap();
        let (s, k) = img.fit(5);
        assert_eq!((s.width, s.height), (5, 2));
        assert!((k - 2.0).abs() < 1e-6);
        assert!(img.warp_affine([0.0; 6], 4, 4).is_none());
        let id = img.warp_affine([1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 10, 4).unwrap();
        assert_eq!(id.data, img.data);
        assert!(img.crop(20, 0, 3, 3).is_none());
        assert_eq!(img.crop(8, 2, 5, 5).map(|c| (c.width, c.height)), Some((2, 2)));
        assert!(Rgb8::new(2, 2, vec![0; 11]).is_none());
        assert_eq!(img.bilinear(f32::NAN, 0.0), [0.0; 3]);
    }
}
