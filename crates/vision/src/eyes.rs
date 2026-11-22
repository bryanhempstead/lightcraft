//! Open or closed eyes, for Assisted Culling: the OpenVINO Open Model Zoo's `open-closed-eye-0001` (Intel,
//! Apache-2.0; a 11 k-parameter network trained on the MRL Eye Dataset's infrared eye crops) run on a square crop
//! around each eye landmark YuNet gives.
//!
//! The crop is grey (the training pictures are infrared, grey) and 0.6 × the distance between the eyes across,
//! resized to 32 × 32 and fed as `(v − 127) / 255` on three equal channels. The network's first output is the
//! probability that the eye is closed (measured: the model card lists the classes the other way round). Eyes closer
//! together than [`MIN_EYE_DISTANCE`] pixels aren't judged.

use std::path::Path;

use crate::graph::{Model, T};
use crate::image::Rgb8;
use crate::{Error, Face, Result};

/// Eyes closer together than this (pixels of the analysed picture) are too small to judge.
pub const MIN_EYE_DISTANCE: f32 = 14.0;
/// A crop's side, in eye distances.
const CROP: f32 = 0.6;
const SIZE: usize = 32;

pub struct EyeState {
    model: Model,
}

/// What the network says about one face.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Eyes {
    /// Probability that each eye (right, left of the person) is closed.
    pub closed: [f32; 2],
}

impl Eyes {
    /// Both eyes more likely closed than not (blinks), by `threshold` (0.5 = as likely as not).
    pub fn both_closed(&self, threshold: f32) -> bool {
        self.closed.iter().all(|&p| p >= threshold)
    }
}

impl EyeState {
    pub fn load(path: &Path) -> Result<EyeState> {
        Ok(EyeState { model: Model::load(path)? })
    }

    /// The eyes of `face` in `img`; `None` when the face is too small to tell.
    pub fn eyes(&self, img: &Rgb8, face: &Face) -> Result<Option<Eyes>> {
        let [re, le] = [face.landmarks[0], face.landmarks[1]];
        let d = (le[0] - re[0]).hypot(le[1] - re[1]);
        if d.is_nan() || d < MIN_EYE_DISTANCE {
            return Ok(None);
        }
        let mut closed = [0f32; 2];
        for (k, e) in [re, le].into_iter().enumerate() {
            let side = (d * CROP).max(4.0);
            let (x0, y0) = ((e[0] - side / 2.0).round(), (e[1] - side / 2.0).round());
            if x0 < 0.0 || y0 < 0.0 {
                return Ok(None);
            }
            let Some(crop) = img.crop(x0 as usize, y0 as usize, side as usize, side as usize) else { return Ok(None) };
            if crop.width * 4 < side as usize * 3 || crop.height * 4 < side as usize * 3 {
                // mostly outside the picture
                return Ok(None);
            }
            let small = crop.resize_area(SIZE, SIZE);
            let grey: Vec<f32> = small.luma().into_iter().map(|v| (v * 255.0 - 127.0) / 255.0).collect();
            let mut data = Vec::with_capacity(3 * SIZE * SIZE);
            for _ in 0..3 {
                data.extend_from_slice(&grey);
            }
            let out = self.model.run(vec![T::new(vec![1, 3, SIZE, SIZE], data)?])?;
            let p = out.into_values().next().ok_or_else(|| Error::Model("the eye model gave no output".into()))?;
            let v = p.data.first().copied().ok_or_else(|| Error::Model("the eye model gave no output".into()))?;
            if let Some(c) = closed.get_mut(k) {
                *c = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
            }
        }
        Ok(Some(Eyes { closed }))
    }
}
