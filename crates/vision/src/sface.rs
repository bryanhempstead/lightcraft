//! SFace face recognition (Zhong et al., "SFace: Sigmoid-Constrained Hypersphere Loss for Robust Face Recognition",
//! 2021; OpenCV Zoo `face_recognition_sface_2021dec`, Apache-2.0): a 128-number embedding of a face, compared by
//! cosine similarity (OpenCV gives 0.363 as its same-person threshold).
//!
//! Alignment follows OpenCV's `FaceRecognizerSF::alignCrop`: the similarity transform (rotation, uniform scale,
//! shift) that best maps the detector's five landmarks onto the ArcFace template, then a bilinear warp to 112 × 112.
//! The network takes RGB 0..255 (it normalises itself).

use std::path::Path;

use crate::graph::{Model, T};
use crate::image::Rgb8;
use crate::{Error, Face, Result};

/// Embedding length.
pub const DIM: usize = 128;
/// The aligned face's size.
pub const SIZE: usize = 112;

/// Where the five landmarks go in the 112 × 112 aligned face (InsightFace's ArcFace template).
const TEMPLATE: [[f32; 2]; 5] = [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

pub struct SFace {
    model: Model,
}

/// The least-squares similarity transform taking `src` onto `dst` (`[a, b, c, d, e, f]` as in
/// [`Rgb8::warp_affine`]). In 2-D this is Umeyama's estimate whenever the best fit isn't a mirror image.
pub fn similarity(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> Option<[f32; 6]> {
    let n = src.len() as f64;
    let mean = |p: &[[f32; 2]; 5]| {
        let (x, y) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + f64::from(q[0]), y + f64::from(q[1])));
        (x / n, y / n)
    };
    let ((sx, sy), (dx, dy)) = (mean(src), mean(dst));
    // z_dst ≈ (a + ib) z_src: a + ib = Σ conj(s) d / Σ |s|²
    let (mut re, mut im, mut norm) = (0.0f64, 0.0f64, 0.0f64);
    for (s, d) in src.iter().zip(dst) {
        let (px, py) = (f64::from(s[0]) - sx, f64::from(s[1]) - sy);
        let (qx, qy) = (f64::from(d[0]) - dx, f64::from(d[1]) - dy);
        re += px * qx + py * qy;
        im += px * qy - py * qx;
        norm += px * px + py * py;
    }
    if norm.is_nan() || norm <= 1e-9 {
        return None;
    }
    let (a, b) = (re / norm, im / norm);
    let (tx, ty) = (dx - (a * sx - b * sy), dy - (b * sx + a * sy));
    let m = [a, -b, tx, b, a, ty].map(|v| v as f32);
    m.iter().all(|v| v.is_finite()).then_some(m)
}

/// The face cut out and aligned for recognition (112 × 112).
pub fn align(img: &Rgb8, face: &Face) -> Option<Rgb8> {
    let m = similarity(&face.landmarks, &TEMPLATE)?;
    img.warp_affine(m, SIZE, SIZE)
}

impl SFace {
    pub fn load(path: &Path) -> Result<SFace> {
        Ok(SFace { model: Model::load(path)? })
    }

    /// The unit-length embedding of an aligned face ([`align`]).
    pub fn embed_aligned(&self, aligned: &Rgb8) -> Result<Vec<f32>> {
        if aligned.width != SIZE || aligned.height != SIZE {
            return Err(Error::Model("the face isn't aligned to 112 × 112".into()));
        }
        let plane = SIZE * SIZE;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in aligned.data.as_chunks::<3>().0.iter().enumerate() {
            for c in 0..3 {
                if let Some(d) = data.get_mut(c * plane + i) {
                    *d = f32::from(p[c]);
                }
            }
        }
        let out = self.model.run(vec![T::new(vec![1, 3, SIZE, SIZE], data)?])?;
        let v = out.into_values().next().ok_or_else(|| Error::Model("SFace gave no output".into()))?;
        if v.data.len() != DIM {
            return Err(Error::Model(format!("SFace gave {} numbers, not {DIM}", v.data.len())));
        }
        Ok(normalize(v.data))
    }

    /// The embedding of `face` in `img`.
    pub fn embed(&self, img: &Rgb8, face: &Face) -> Result<Vec<f32>> {
        let a = align(img, face).ok_or_else(|| Error::Model("the face's landmarks can't be aligned".into()))?;
        self.embed_aligned(&a)
    }
}

/// `v` scaled to unit length (zeros stay zeros).
pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 && n.is_finite() {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

/// Cosine similarity of two unit-length embeddings.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_maps_onto_itself_and_a_scaled_copy_back() {
        let m = similarity(&TEMPLATE, &TEMPLATE).unwrap();
        assert!((m[0] - 1.0).abs() < 1e-5 && m[1].abs() < 1e-5 && m[2].abs() < 1e-3);
        let big = TEMPLATE.map(|p| [p[0] * 2.0 + 10.0, p[1] * 2.0 + 5.0]);
        let m = similarity(&big, &TEMPLATE).unwrap();
        assert!((m[0] - 0.5).abs() < 1e-5);
        assert!(similarity(&[[1.0, 1.0]; 5], &TEMPLATE).is_none());
    }
}
