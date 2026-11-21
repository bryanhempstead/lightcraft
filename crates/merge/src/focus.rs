//! Focus stacking: photos of one scene focused at different distances → one photo sharp from
//! front to back (macro, product, landscape near-far).
//!
//! 1. **Align** every frame on the first ([`crate::align`]: a homography, since focus breathing
//!    changes the scale a little between frames).
//! 2. **Sharpness** per pixel and frame: the local energy of the Laplacian of perceptual
//!    luminance (squared Laplacian, Gaussian-averaged over a few pixels).
//! 3. **Blend**: each frame weighted by its sharpness raised to a power, so the sharpest frame
//!    wins wherever there is detail, the Gaussian feathers the seams, and flat areas (no detail
//!    in any frame) average every frame instead of flickering between them.
//!
//! The method is the standard "contrast-based" focus stack (e.g. Pertuz et al., "Analysis of
//! focus measure operators for shape-from-focus", Pattern Recognition 2013, the Laplacian
//! energy operator); our own implementation.

use lightcraft_raster::{Plane, Rgb32f, blur::gaussian};
use rayon::prelude::*;

use crate::align::{self, Alignment};
use crate::{Frame, FrameColor, MergeError, Progress, Result, check};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FocusOptions {
    /// Line the frames up first (off for a locked-down tripod and no focus breathing).
    pub align: bool,
}

impl Default for FocusOptions {
    fn default() -> FocusOptions {
        FocusOptions { align: true }
    }
}

pub struct FocusResult {
    /// Scene-linear, in the frames' colour space, the first frame's geometry.
    pub image: Rgb32f,
    pub alignments: Vec<Alignment>,
    pub color: FrameColor,
    pub raw: bool,
    pub orientation: lightcraft_geom::Orientation,
    pub metadata: lightcraft_meta::Metadata,
    pub baseline_exposure: f64,
}

/// Sharpness power: higher → closer to "the sharpest frame only" (crisper, more seam risk).
const POWER: f32 = 2.0;

/// Perceptual luminance (0..1, gamma 2.2) of a frame normalised by `gain`.
fn luma(img: &Rgb32f, gain: f32) -> Plane {
    img.map(|c| ((0.25 * c[0] + 0.6 * c[1] + 0.15 * c[2]) * gain).clamp(0.0, 1.0).powf(1.0 / 2.2))
}

/// Local focus energy: squared 4-neighbour Laplacian, Gaussian-averaged (`sigma` px).
fn energy(l: &Plane, sigma: f32) -> Plane {
    let (w, h) = (l.width, l.height);
    let mut e = Plane::new(w, h);
    if w < 3 || h < 3 {
        return e;
    }
    e.data.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let (ym, yp) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for (x, o) in row.iter_mut().enumerate() {
            let (xm, xp) = (x.saturating_sub(1), (x + 1).min(w - 1));
            let c = l.get(x, y);
            let lap = 4.0 * c - l.get(xm, y) - l.get(xp, y) - l.get(x, ym) - l.get(x, yp);
            *o = lap * lap;
        }
    });
    gaussian(&e, sigma)
}

/// Stack frames focused at different distances. Frames must share their pixel size.
pub fn merge_focus(mut frames: Vec<Frame>, opts: &FocusOptions, progress: &Progress) -> Result<FocusResult> {
    let n = frames.len();
    if n < 2 {
        return Err(MergeError::TooFew(n, 2));
    }
    if frames.iter().any(|f| f.orientation != frames[0].orientation) {
        for f in frames.iter_mut() {
            f.orient();
        }
    }
    let (w, h) = (frames[0].width(), frames[0].height());
    if frames.iter().any(|f| f.width() != w || f.height() != h) {
        return Err(MergeError::Mismatch("Focus stacking needs photos of the same size".into()));
    }
    check(progress, 0.02, "Analysing frames")?;
    // brightness: frames of a stack are meant to match, but even them out for the sharpness measure
    let medians: Vec<f32> = frames.par_iter().map(|f| align::median_luma(&f.image, f.clip)).collect();
    let gains: Vec<f32> = medians.iter().map(|m| 0.18 / m.max(1e-6)).collect();

    // 1. alignment on the first frame
    let mut alignments = vec![Alignment::identity(); n];
    if opts.align {
        check(progress, 0.05, "Aligning")?;
        let feats: Vec<(crate::features::Features, f64)> = frames
            .par_iter()
            .zip(&gains)
            .map(|(f, &g)| {
                let (p, s) = align::work_image(&f.image, g, align::WORK_EDGE);
                (crate::features::detect(&p, 1500), s)
            })
            .collect();
        let found: Vec<(usize, Alignment)> =
            (1..n).into_par_iter().map(|i| (i, align::align_pair(&feats[0].0, &feats[i].0, feats[i].1, w, h, i as u64 + 1))).collect();
        for (i, a) in found {
            alignments[i] = a;
        }
    }

    // 2–3. sharpness-weighted blend, one frame at a time
    let sigma = (w.max(h) as f32 / 600.0).clamp(1.5, 8.0);
    let mut acc = vec![[0.0f32; 3]; w * h];
    let mut wsum = vec![0.0f32; w * h];
    for i in 0..n {
        check(progress, 0.25 + 0.7 * i as f32 / n as f32, "Finding the sharpest areas")?;
        let (img, valid) = if alignments[i].model == "identity" {
            (frames[i].image.clone(), vec![true; w * h])
        } else {
            align::warp(&frames[i].image, &alignments[i].h)
        };
        let e = energy(&luma(&img, gains[i]), sigma);
        acc.par_iter_mut().zip(wsum.par_iter_mut()).enumerate().for_each(|(k, (a, s))| {
            if !valid.get(k).copied().unwrap_or(false) {
                return;
            }
            // tiny floor: flat areas average every frame (no noise-driven flicker)
            let wt = (e.data.get(k).copied().unwrap_or(0.0) * 1e4 + 1e-6).powf(POWER);
            if !wt.is_finite() {
                return;
            }
            let p = img.data.get(k).copied().unwrap_or([0.0; 3]);
            a[0] += wt * p[0];
            a[1] += wt * p[1];
            a[2] += wt * p[2];
            *s += wt;
        });
    }
    check(progress, 0.96, "Blending")?;
    let mut out = Rgb32f::new(w, h);
    out.data.par_iter_mut().enumerate().for_each(|(k, o)| {
        let s = wsum.get(k).copied().unwrap_or(0.0);
        let a = acc.get(k).copied().unwrap_or([0.0; 3]);
        *o = if s > 0.0 { [a[0] / s, a[1] / s, a[2] / s] } else { frames[0].image.data.get(k).copied().unwrap_or([0.0; 3]) };
    });
    let rf = &frames[0];
    Ok(FocusResult {
        image: out,
        alignments,
        color: rf.color.clone(),
        raw: rf.raw,
        orientation: rf.orientation,
        metadata: rf.metadata.clone(),
        baseline_exposure: rf.baseline_exposure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fine checker pattern; `blur` px of Gaussian blur on the left (`left`) or right half.
    fn scene(w: usize, h: usize, sharp_left: bool) -> Rgb32f {
        let mut img = Rgb32f::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if ((x / 2) + (y / 2)) % 2 == 0 { 0.6 } else { 0.1 };
                img.data[y * w + x] = [v, v, v];
            }
        }
        let soft = gaussian(&img, 3.0);
        for y in 0..h {
            for x in 0..w {
                let blur_here = if sharp_left { x >= w / 2 } else { x < w / 2 };
                if blur_here {
                    img.data[y * w + x] = soft.data[y * w + x];
                }
            }
        }
        img
    }

    fn contrast(img: &Rgb32f, x0: usize, x1: usize) -> f32 {
        let w = img.width;
        let mut s = 0.0;
        for y in 10..img.height - 10 {
            for x in x0..x1 {
                s += (img.data[y * w + x][0] - img.data[y * w + x + 1][0]).abs();
            }
        }
        s
    }

    #[test]
    fn stack_is_sharp_on_both_halves() {
        let (w, h) = (160, 96);
        let a = Frame::from_linear_rec2020(scene(w, h, true), None);
        let b = Frame::from_linear_rec2020(scene(w, h, false), None);
        let r = merge_focus(vec![a, b], &FocusOptions { align: false }, &crate::no_progress).unwrap();
        let (l, rr) = (contrast(&r.image, 10, w / 2 - 10), contrast(&r.image, w / 2 + 10, w - 11));
        let sharp = contrast(&scene(w, h, true), 10, w / 2 - 10);
        assert!(l > 0.9 * sharp && rr > 0.9 * sharp, "left {l} right {rr} vs sharp {sharp}");
    }

    #[test]
    fn rejects_too_few_and_mismatched() {
        let a = Frame::from_linear_rec2020(Rgb32f::new(20, 20), None);
        assert!(matches!(merge_focus(vec![a.clone()], &FocusOptions::default(), &crate::no_progress), Err(MergeError::TooFew(1, 2))));
        let b = Frame::from_linear_rec2020(Rgb32f::new(30, 20), None);
        assert!(matches!(merge_focus(vec![a, b], &FocusOptions::default(), &crate::no_progress), Err(MergeError::Mismatch(_))));
    }
}
