//! Facial expression, for Assisted Culling's smiles: OpenCV Zoo's `facial_expression_recognition_mobilefacenet_2022july`
//! (Jing Jiang's Progressive Teacher on a MobileFaceNet, Apache-2.0; 88.3 % on RAF-DB) — seven basic expressions.
//!
//! The face is aligned exactly as for recognition ([`crate::sface::align`]: the ArcFace template, 112 × 112; the
//! model's own demo uses the same five points), then fed as RGB `(v / 255 − 0.5) / 0.5`, NCHW. The network gives
//! seven logits; [`Expression::probs`] is their softmax in [`LABELS`] order.

use std::path::Path;

use crate::graph::{Model, T};
use crate::image::Rgb8;
use crate::sface::SIZE;
use crate::{Error, Face, Result};

/// The model's classes, in its output order.
pub const LABELS: [&str; 7] = ["angry", "disgust", "fearful", "happy", "neutral", "sad", "surprised"];
pub const ANGRY: usize = 0;
pub const DISGUST: usize = 1;
pub const FEARFUL: usize = 2;
pub const HAPPY: usize = 3;
pub const NEUTRAL: usize = 4;
pub const SAD: usize = 5;
pub const SURPRISED: usize = 6;

/// Faces whose eyes are closer together than this (pixels of the analysed picture) are too small to read.
pub const MIN_EYE_DISTANCE: f32 = 12.0;

pub struct ExpressionNet {
    model: Model,
}

/// What the network says about one face: the probability of each of [`LABELS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Expression {
    pub probs: [f32; 7],
}

impl Expression {
    /// How likely the face smiles (the `happy` class).
    pub fn smile(&self) -> f32 {
        self.probs[HAPPY]
    }

    /// The most likely class.
    pub fn top(&self) -> usize {
        self.probs.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap_or(NEUTRAL)
    }

    /// How likely the face shows an unflattering expression (angry, disgusted, fearful, sad).
    pub fn frown(&self) -> f32 {
        self.probs[ANGRY] + self.probs[DISGUST] + self.probs[FEARFUL] + self.probs[SAD]
    }

    /// Softmax of seven logits (non-finite logits count as very unlikely).
    pub fn from_logits(l: &[f32]) -> Option<Expression> {
        if l.len() != 7 {
            return None;
        }
        let l: Vec<f32> = l.iter().map(|v| if v.is_finite() { *v } else { -1e4 }).collect();
        let m = l.iter().copied().fold(f32::MIN, f32::max);
        let e: Vec<f32> = l.iter().map(|v| (v - m).exp()).collect();
        let s: f32 = e.iter().sum();
        if !(s > 0.0 && s.is_finite()) {
            return None;
        }
        let mut probs = [0f32; 7];
        for (p, v) in probs.iter_mut().zip(e) {
            *p = v / s;
        }
        Some(Expression { probs })
    }
}

impl ExpressionNet {
    pub fn load(path: &Path) -> Result<ExpressionNet> {
        Ok(ExpressionNet { model: Model::load(path)? })
    }

    /// The expression of an aligned face ([`crate::sface::align`]).
    pub fn classify_aligned(&self, aligned: &Rgb8) -> Result<Expression> {
        if aligned.width != SIZE || aligned.height != SIZE {
            return Err(Error::Model("the face isn't aligned to 112 × 112".into()));
        }
        let plane = SIZE * SIZE;
        let mut data = vec![0f32; 3 * plane];
        for (i, p) in aligned.data.as_chunks::<3>().0.iter().enumerate() {
            for c in 0..3 {
                if let Some(d) = data.get_mut(c * plane + i) {
                    *d = (f32::from(p[c]) / 255.0 - 0.5) / 0.5;
                }
            }
        }
        let out = self.model.run(vec![T::new(vec![1, 3, SIZE, SIZE], data)?])?;
        let v = out.into_values().next().ok_or_else(|| Error::Model("the expression model gave no output".into()))?;
        Expression::from_logits(&v.data).ok_or_else(|| Error::Model(format!("the expression model gave {} numbers, not 7", v.data.len())))
    }

    /// The expression of `face` in `img`; `None` when the face is too small to read.
    pub fn classify(&self, img: &Rgb8, face: &Face) -> Result<Option<Expression>> {
        let [re, le] = [face.landmarks[0], face.landmarks[1]];
        let d = (le[0] - re[0]).hypot(le[1] - re[1]);
        if d.is_nan() || d < MIN_EYE_DISTANCE {
            return Ok(None);
        }
        let Some(a) = crate::sface::align(img, face) else { return Ok(None) };
        self.classify_aligned(&a).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn softmax_and_classes() {
        let e = Expression::from_logits(&[0.0, 0.0, 0.0, 5.0, 1.0, 0.0, 0.0]).unwrap();
        assert_eq!(e.top(), HAPPY);
        assert!(e.smile() > 0.9);
        assert!((e.probs.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        let e = Expression::from_logits(&[f32::NAN, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0]).unwrap();
        assert_eq!(e.top(), NEUTRAL);
        assert!(e.probs[ANGRY] < 1e-6);
        assert!(Expression::from_logits(&[1.0; 6]).is_none());
        assert_eq!(LABELS[SURPRISED], "surprised");
    }
}
