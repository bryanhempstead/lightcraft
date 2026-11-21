//! Best Take: a burst or group shot where everyone looks their best in *some* frame → one photo
//! with each person's face taken from the frame you pick for them.
//!
//! For each swap: the donor frame's face is placed on the base frame's face by the two face boxes
//! (one scale + a slide), fine-tuned by a small search on the face patch (zero-mean luminance
//! SSD over a shift and a few scales), brightness/colour matched on a ring just outside the face
//! (hair, neck, background: the same in both frames, so any difference there is exposure or white
//! balance drift, not the face), and composited through a feathered oval. Works on scene-linear
//! frames, so the result is a raw-like DNG like the other merges. Our own implementation.

use lightcraft_raster::Rgb32f;
use serde::{Deserialize, Serialize};

use crate::{Frame, FrameColor, MergeError, Progress, Result, check};

/// A face box: normalized `[x, y, w, h]`, top-left origin, in a frame's upright geometry.
pub type FaceBox = [f64; 4];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceSwap {
    /// Index of the frame the face comes from.
    pub donor: usize,
    /// The face in the base frame (where it goes) and in the donor (what is taken).
    pub base_box: FaceBox,
    pub donor_box: FaceBox,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TakeOptions {
    /// Index of the frame everything else comes from.
    pub base: usize,
    pub swaps: Vec<FaceSwap>,
}

pub struct TakeResult {
    pub image: Rgb32f,
    /// Per swap: whether the patch search moved the face from where the boxes put it (px, at full size).
    pub shifts: Vec<[f64; 2]>,
    pub color: FrameColor,
    pub raw: bool,
    pub orientation: lightcraft_geom::Orientation,
    pub metadata: lightcraft_meta::Metadata,
    pub baseline_exposure: f64,
}

fn luma(p: [f32; 3]) -> f32 {
    (0.25 * p[0] + 0.6 * p[1] + 0.15 * p[2]).max(0.0).powf(1.0 / 2.2)
}

fn valid_box(b: &FaceBox) -> bool {
    b.iter().all(|v| v.is_finite()) && b[2] > 1e-4 && b[3] > 1e-4 && b[2] <= 1.5 && b[3] <= 1.5
}

/// Base pixel → donor pixel: `d = (p - t) / s + o` with the boxes' scale `s`, slide `t` and a
/// search offset.
#[derive(Clone, Copy)]
struct Place {
    s: f64,
    tx: f64,
    ty: f64,
}

impl Place {
    fn donor(&self, x: f64, y: f64) -> (f32, f32) {
        (((x - self.tx) / self.s) as f32, ((y - self.ty) / self.s) as f32)
    }
}

fn place(bb: &FaceBox, db: &FaceBox, (bw, bh): (f64, f64), (dw, dh): (f64, f64)) -> Place {
    let s = ((bb[2] * bw) / (db[2] * dw).max(1.0) + (bb[3] * bh) / (db[3] * dh).max(1.0)) / 2.0;
    let tx = (bb[0] + bb[2] / 2.0) * bw - s * (db[0] + db[2] / 2.0) * dw;
    let ty = (bb[1] + bb[3] / 2.0) * bh - s * (db[1] + db[3] / 2.0) * dh;
    Place { s: s.max(1e-3), tx, ty }
}

/// Fine-tune `p` on the face patch: try shifts (and ±4 % scale about the face centre) and keep
/// the one whose donor patch best matches the base patch (zero-mean luminance SSD, ~48 px grid).
fn refine(base: &Rgb32f, donor: &Rgb32f, bb: &FaceBox, p: Place) -> Place {
    let (w, h) = (base.width as f64, base.height as f64);
    let (x0, y0) = (((bb[0] - bb[2] * 0.15) * w).max(0.0), ((bb[1] - bb[3] * 0.15) * h).max(0.0));
    let (x1, y1) = (((bb[0] + bb[2] * 1.15) * w).min(w - 1.0), ((bb[1] + bb[3] * 1.15) * h).min(h - 1.0));
    if x1 - x0 < 8.0 || y1 - y0 < 8.0 {
        return p;
    }
    let n = 48usize;
    let pts: Vec<(f64, f64)> = (0..n)
        .flat_map(|j| (0..n).map(move |i| (x0 + (x1 - x0) * (i as f64 + 0.5) / n as f64, y0 + (y1 - y0) * (j as f64 + 0.5) / n as f64)))
        .collect();
    let bl: Vec<f32> = pts.iter().map(|&(x, y)| luma(base.sample_bilinear(x as f32, y as f32))).collect();
    let bm = bl.iter().sum::<f32>() / bl.len() as f32;
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (dwid, dht) = (donor.width as f32, donor.height as f32);
    let cost = |q: &Place| -> f64 {
        let dl: Vec<f32> = pts
            .iter()
            .map(|&(x, y)| {
                let (u, v) = q.donor(x, y);
                luma(donor.sample_bilinear(u.clamp(0.0, dwid), v.clamp(0.0, dht)))
            })
            .collect();
        let dm = dl.iter().sum::<f32>() / dl.len() as f32;
        bl.iter().zip(&dl).map(|(a, b)| f64::from((a - bm) - (b - dm)).powi(2)).sum()
    };
    // scale about the face centre: keep the centre's donor point fixed
    let scaled = |q: Place, k: f64| {
        let s = q.s * k;
        let (ux, uy) = ((cx - q.tx) / q.s, (cy - q.ty) / q.s);
        Place { s, tx: cx - s * ux, ty: cy - s * uy }
    };
    let mut best = (cost(&p), p);
    let span = (x1 - x0).max(y1 - y0);
    for (range, step) in [(span * 0.12, span / 48.0), (span / 48.0, span / 192.0)] {
        let c = best.1;
        let k = (range / step).round() as i32;
        for &sk in &[0.96, 1.0, 1.04] {
            let base_q = if sk == 1.0 { c } else { scaled(c, sk) };
            for j in -k..=k {
                for i in -k..=k {
                    let q = Place { tx: base_q.tx + i as f64 * step, ty: base_q.ty + j as f64 * step, ..base_q };
                    let e = cost(&q);
                    if e < best.0 {
                        best = (e, q);
                    }
                }
            }
        }
    }
    best.1
}

#[inline]
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Put each picked face onto the base frame.
pub fn best_take(mut frames: Vec<Frame>, opts: &TakeOptions, progress: &Progress) -> Result<TakeResult> {
    let n = frames.len();
    if n < 2 {
        return Err(MergeError::TooFew(n, 2));
    }
    for f in frames.iter_mut() {
        f.orient();
    }
    if opts.base >= n {
        return Err(MergeError::Mismatch(format!("no photo {} to start from", opts.base + 1)));
    }
    let mut out = frames[opts.base].image.clone();
    let (w, h) = (out.width as f64, out.height as f64);
    let mut shifts = Vec::new();
    for (k, sw) in opts.swaps.iter().enumerate() {
        check(progress, 0.05 + 0.9 * k as f32 / opts.swaps.len().max(1) as f32, "Placing faces")?;
        let Some(donor) = frames.get(sw.donor).map(|f| &f.image) else {
            return Err(MergeError::Mismatch(format!("no photo {} to take a face from", sw.donor + 1)));
        };
        if sw.donor == opts.base {
            continue;
        }
        if !valid_box(&sw.base_box) || !valid_box(&sw.donor_box) {
            return Err(MergeError::Mismatch("a face box is empty or out of range".into()));
        }
        let bb = sw.base_box;
        let first = place(&bb, &sw.donor_box, (w, h), (donor.width as f64, donor.height as f64));
        let p = refine(&frames[opts.base].image, donor, &bb, first);
        shifts.push([p.tx - first.tx, p.ty - first.ty]);
        // the oval a little bigger than the face box, feathered; the ring outside it for colour
        let (cx, cy) = ((bb[0] + bb[2] / 2.0) * w, (bb[1] + bb[3] / 2.0) * h);
        let (ax, ay) = ((bb[2] * w * 0.58).max(2.0), (bb[3] * h * 0.62).max(2.0));
        let r_of = |x: f64, y: f64| (((x - cx) / ax).powi(2) + ((y - cy) / ay).powi(2)).sqrt();
        let (xa, xb) = ((cx - ax * 1.6).max(0.0) as usize, ((cx + ax * 1.6).min(w - 1.0)).max(0.0) as usize);
        let (ya, yb) = ((cy - ay * 1.6).max(0.0) as usize, ((cy + ay * 1.6).min(h - 1.0)).max(0.0) as usize);
        let (dwid, dht) = (donor.width as f32, donor.height as f32);
        let sample = |x: usize, y: usize| {
            let (u, v) = p.donor(x as f64 + 0.5, y as f64 + 0.5);
            donor.sample_bilinear(u.clamp(0.0, dwid), v.clamp(0.0, dht))
        };
        let (mut sb, mut sd, mut cnt) = ([0.0f64; 3], [0.0f64; 3], 0usize);
        for y in ya..=yb {
            for x in xa..=xb {
                let r = r_of(x as f64 + 0.5, y as f64 + 0.5);
                if (1.1..1.5).contains(&r) {
                    let (b, d) = (out.get(x, y), sample(x, y));
                    for c in 0..3 {
                        sb[c] += f64::from(b[c]);
                        sd[c] += f64::from(d[c]);
                    }
                    cnt += 1;
                }
            }
        }
        let gain: [f32; 3] = std::array::from_fn(|c| if cnt > 50 { ((sb[c] + 1e-6) / (sd[c] + 1e-6)).clamp(0.8, 1.25) as f32 } else { 1.0 });
        for y in ya..=yb {
            for x in xa..=xb {
                let m = 1.0 - smoothstep(0.7, 1.15, r_of(x as f64 + 0.5, y as f64 + 0.5));
                if m <= 0.0 {
                    continue;
                }
                let (b, d) = (out.get(x, y), sample(x, y));
                let m = m as f32;
                let v: [f32; 3] = std::array::from_fn(|c| b[c] * (1.0 - m) + d[c] * gain[c] * m);
                if let Some(px) = out.data.get_mut(y * out.width + x) {
                    *px = v;
                }
            }
        }
    }
    check(progress, 0.97, "Finishing")?;
    let rf = &frames[opts.base];
    Ok(TakeResult {
        image: out,
        shifts,
        color: rf.color.clone(),
        raw: rf.raw,
        orientation: lightcraft_geom::Orientation::Normal,
        metadata: rf.metadata.clone(),
        baseline_exposure: rf.baseline_exposure,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grey frame with a textured "face" disc at (fx, fy), radius r px; `eyes` paints a dark bar.
    fn frame(w: usize, h: usize, fx: f64, fy: f64, r: f64, eyes: bool, gain: f32) -> Rgb32f {
        let mut img = Rgb32f::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (x as f64 - fx, y as f64 - fy);
                let mut v = 0.2 + 0.05 * (((x / 3) + (y / 5)) % 2) as f32;
                if dx * dx + dy * dy < r * r {
                    v = 0.5 + 0.2 * ((dx * 0.4).sin() * (dy * 0.3).cos()) as f32;
                    if eyes && dy.abs() < r * 0.12 && dy < 0.0 + r * 0.1 && dy > -r * 0.3 {
                        v = 0.05;
                    }
                }
                img.data[y * w + x] = [v * gain, v * gain, v * gain];
            }
        }
        img
    }

    #[test]
    fn swaps_the_face_and_lines_it_up() {
        let (w, h) = (200, 160);
        // base: eyes "shut" (dark bar); donor: eyes open, shifted 6 px right, 10 % brighter
        let base = Frame::from_linear_rec2020(frame(w, h, 100.0, 80.0, 30.0, true, 1.0), None);
        let donor = Frame::from_linear_rec2020(frame(w, h, 106.0, 80.0, 30.0, false, 1.1), None);
        let bb = [70.0 / w as f64, 50.0 / h as f64, 60.0 / w as f64, 60.0 / h as f64];
        // the donor's box is a little off (as a detector's would be)
        let db = [(76.0 + 3.0) / w as f64, 50.0 / h as f64, 60.0 / w as f64, 60.0 / h as f64];
        let opts = TakeOptions { base: 0, swaps: vec![FaceSwap { donor: 1, base_box: bb, donor_box: db }] };
        let r = best_take(vec![base, donor], &opts, &crate::no_progress).unwrap();
        // the dark bar is gone from the face …
        let eye = r.image.get(100, 77)[0];
        assert!(eye > 0.2, "eyes open now: {eye}");
        // … the donor was lined up (the search undid the 3 px box error) …
        assert!((r.shifts[0][0] - 3.0).abs() <= 1.5, "shift {:?}", r.shifts[0]);
        // … its brightness matched to the base, and the background untouched
        let face = r.image.get(100, 95)[0];
        let want = frame(w, h, 100.0, 80.0, 30.0, false, 1.0).get(100, 95)[0];
        assert!((face - want).abs() < 0.05, "face {face} vs {want}");
        assert_eq!(r.image.get(5, 5), frame(w, h, 100.0, 80.0, 30.0, true, 1.0).get(5, 5));
    }

    #[test]
    fn rejects_bad_input() {
        let a = Frame::from_linear_rec2020(Rgb32f::new(20, 20), None);
        assert!(matches!(best_take(vec![a.clone()], &TakeOptions::default(), &crate::no_progress), Err(MergeError::TooFew(1, 2))));
        let bad = TakeOptions { base: 0, swaps: vec![FaceSwap { donor: 5, base_box: [0.1, 0.1, 0.2, 0.2], donor_box: [0.1, 0.1, 0.2, 0.2] }] };
        assert!(best_take(vec![a.clone(), a.clone()], &bad, &crate::no_progress).is_err());
        let nan = TakeOptions { base: 0, swaps: vec![FaceSwap { donor: 1, base_box: [f64::NAN, 0.1, 0.2, 0.2], donor_box: [0.1, 0.1, 0.2, 0.2] }] };
        assert!(best_take(vec![a.clone(), a], &nan, &crate::no_progress).is_err());
        assert!(
            best_take(
                vec![Frame::from_linear_rec2020(Rgb32f::new(20, 20), None), Frame::from_linear_rec2020(Rgb32f::new(20, 20), None)],
                &TakeOptions { base: 3, swaps: vec![] },
                &crate::no_progress
            )
            .is_err()
        );
    }
}
