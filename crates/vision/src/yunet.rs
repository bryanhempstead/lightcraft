//! YuNet face detection (Wu et al., "YuNet: A Tiny Millisecond-level Face Detector", 2023; OpenCV Zoo
//! `face_detection_yunet_2023mar`, MIT): boxes, five landmarks and a confidence for every face in a picture.
//!
//! Pre- and post-processing follow OpenCV's `FaceDetectorYN` (`modules/objdetect/src/face_detect.cpp`): the picture
//! goes in as BGR 0..255, padded with black on the right and bottom to a multiple of 32; each of the three strides
//! (8, 16, 32) gives a class and an objectness probability per cell, whose geometric mean is the score, and the box
//! and landmarks as offsets from the cell in units of the stride; overlapping boxes are then reduced by greedy
//! non-maximum suppression.

use std::path::Path;

use crate::graph::{Model, T};
use crate::image::Rgb8;
use crate::{Error, Face, Result};

pub struct YuNet {
    model: Model,
    /// Faces scoring below this are dropped.
    pub score_threshold: f32,
    /// Boxes overlapping a better one by more than this (intersection over union) are dropped.
    pub nms_threshold: f32,
    /// At most this many faces a picture.
    pub top_k: usize,
}

const STRIDES: [usize; 3] = [8, 16, 32];

impl YuNet {
    pub fn load(path: &Path) -> Result<YuNet> {
        let model = Model::load(path)?;
        for s in STRIDES {
            for head in ["cls", "obj", "bbox", "kps"] {
                if !model.output_names().iter().any(|n| *n == format!("{head}_{s}")) {
                    return Err(Error::Model(format!("{} is not a YuNet model (no {head}_{s} output)", path.display())));
                }
            }
        }
        Ok(YuNet { model, score_threshold: 0.75, nms_threshold: 0.3, top_k: 500 })
    }

    /// The faces in `img`, best first, in `img`'s pixels.
    pub fn detect(&self, img: &Rgb8) -> Result<Vec<Face>> {
        let (w, h) = (img.width, img.height);
        if w < 16 || h < 16 {
            return Ok(Vec::new());
        }
        let (pw, ph) = (w.div_ceil(32) * 32, h.div_ceil(32) * 32);
        // BGR planes, 0..255, black padding
        let mut data = vec![0f32; 3 * pw * ph];
        for y in 0..h {
            for x in 0..w {
                let Some(p) = img.data.get((y * w + x) * 3..(y * w + x) * 3 + 3) else { continue };
                for (c, &v) in [p[2], p[1], p[0]].iter().enumerate() {
                    if let Some(d) = data.get_mut(c * pw * ph + y * pw + x) {
                        *d = f32::from(v);
                    }
                }
            }
        }
        let out = self.model.run(vec![T::new(vec![1, 3, ph, pw], data)?])?;
        let get = |name: String| out.get(&name).ok_or_else(|| Error::Model(format!("YuNet gave no {name}")));
        let mut faces = Vec::new();
        for s in STRIDES {
            let (cls, obj, bbox, kps) = (get(format!("cls_{s}"))?, get(format!("obj_{s}"))?, get(format!("bbox_{s}"))?, get(format!("kps_{s}"))?);
            let (rows, cols) = (ph / s, pw / s);
            let cells = rows * cols;
            if cls.data.len() < cells || obj.data.len() < cells || bbox.data.len() < cells * 4 || kps.data.len() < cells * 10 {
                return Err(Error::Model("YuNet outputs are smaller than the picture".into()));
            }
            for r in 0..rows {
                for c in 0..cols {
                    let i = r * cols + c;
                    let score = (cls.data[i].clamp(0.0, 1.0) * obj.data[i].clamp(0.0, 1.0)).sqrt();
                    if score.is_nan() || score < self.score_threshold {
                        continue;
                    }
                    let b = &bbox.data[i * 4..i * 4 + 4];
                    let (sf, cf, rf) = (s as f32, c as f32, r as f32);
                    let (cx, cy) = ((cf + b[0]) * sf, (rf + b[1]) * sf);
                    let (bw, bh) = (b[2].clamp(-20.0, 20.0).exp() * sf, b[3].clamp(-20.0, 20.0).exp() * sf);
                    let k = &kps.data[i * 10..i * 10 + 10];
                    let mut landmarks = [[0f32; 2]; 5];
                    for (n, l) in landmarks.iter_mut().enumerate() {
                        *l = [(k[2 * n] + cf) * sf, (k[2 * n + 1] + rf) * sf];
                    }
                    let f = Face { rect: [cx - bw / 2.0, cy - bh / 2.0, bw, bh], landmarks, score };
                    if f.rect.iter().all(|v| v.is_finite()) {
                        faces.push(f);
                    }
                }
            }
        }
        Ok(nms(faces, self.nms_threshold, self.top_k))
    }
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (x0, y0) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x1, y1) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let inter = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

/// Greedy non-maximum suppression (OpenCV's `NMSBoxes`): best first, each kept unless it overlaps a kept one.
pub fn nms(mut faces: Vec<Face>, threshold: f32, top_k: usize) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for f in faces {
        if kept.len() >= top_k {
            break;
        }
        if kept.iter().all(|k| iou(&k.rect, &f.rect) <= threshold) {
            kept.push(f);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppression_keeps_the_best_of_overlapping_boxes() {
        let f = |x: f32, s: f32| Face { rect: [x, 0.0, 10.0, 10.0], landmarks: [[0.0; 2]; 5], score: s };
        let kept = nms(vec![f(0.0, 0.8), f(1.0, 0.9), f(50.0, 0.7)], 0.3, 10);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].score, 0.9);
        assert_eq!(nms(vec![f(0.0, 0.8), f(50.0, 0.7)], 0.3, 1).len(), 1);
    }
}
