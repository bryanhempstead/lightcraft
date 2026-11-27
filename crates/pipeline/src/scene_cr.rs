//! Camera Raw's scene tone map (Bryan's fork; `docs/lr-match.md` → Round 11): Clarity and
//! positive Dehaze (R21), Highlights / Shadows (R02), as Camera Raw 18.7 renders them.
//!
//! Ported from RAWmakase (<https://github.com/pch/rawmakase>, `crates/rawmakase-engine/src/develop`:
//! `local_tone.rs` (the measurement map, its guided base and grid sampling), `clarity.rs`,
//! `scene_tone/haze.rs`, `local_tone_rule.rs` and its data, here `scene_cr_rule_*.rs`; MIT License,
//! Copyright (c) 2026 RAWmakase contributors; see NOTICE), whose operators were fitted to Camera
//! Raw 18.7 renders of synthetic scenes and linear-profile photos, and checked against our own
//! Camera Raw oracle (`oracle/make_r11.py`).
//!
//! The render's image before Exposure is reduced to a *map* of at most [`MAP_EDGE`] pixels on
//! its long edge (box means), so previews, regions and exports measure the same photo:
//! - **base level**: a guided filter of the map's log2 luminance (radius 3.2 % of the long edge,
//!   ε 0.5) kept as its coefficients, so each pixel's base is `A · level + B` (edge-aware at the
//!   render's own resolution);
//! - **Clarity**: a log2 gain of detail at four scales (edge-aware blurs of σ 0.4 / 1.5 / 5 /
//!   15 % of the long edge, range σ 2 EV), each weighted by the base level relative to the
//!   photo's brightest levels (0.25·p99.9 + 0.75·p99); gains made on the map at ±50 / ±100 and
//!   upsampled with the pixel's luminance as guide (Cauchy range weight, 0.25 EV);
//! - **Highlights / Shadows**: Camera Raw's gain for a base level follows where it sits in the
//!   photo's histogram of base levels — four small networks per control from 24 (28) histogram
//!   and texture descriptors, evaluated once per photo at 48 levels; the slider scales the
//!   ±100 curve; together, each curve is blended towards a uniform gain at its extreme;
//! - **positive Dehaze**: a dark-channel model on the map (airlight = mean of the haziest 1 %,
//!   density = guided-filtered window minimum of the colour relative to it); a pixel moves away
//!   from the airlight (scaled by Exposure and the local tone gain already applied) by
//!   `t = max(1 − ω · density, 0.3)`, ω 0 / 0.3 / 0.45 / 0.9 at 0 / +20 / +40 / +100.
//!
//! Everything is measured at Exposure 0; Exposure shifts the levels (the base, the guide and the
//! histogram's absolute statistics) and scales the airlight, as Camera Raw's renders do.

use std::sync::{Arc, OnceLock};

use lightcraft_color::luminance_2020;
use lightcraft_raster::{Rgb32f, par_rows};

#[path = "scene_cr_rule_data.rs"]
mod rule_data;
#[path = "scene_cr_rule_ext.rs"]
mod rule_ext;
#[path = "scene_cr_rule_pair.rs"]
mod rule_pair;

/// Render Clarity (global and in masks) with Camera Raw's four-scale model; `false` brings back
/// LightCraft's single guided band.
pub const CLARITY_MEASURED: bool = true;
/// Negative Clarity with the same model; `false` keeps LightCraft's band for it (round 11: the
/// model is 1.36 → 1.43 ΔE on Bryan's raws at −50 and 0.77 → 0.80 on the synthetic scenes, so it
/// stays off; positive Clarity gains 3.75 → 2.51 at +100).
pub const CLARITY_NEGATIVE_MEASURED: bool = false;

/// Whether a pixel's Clarity `amount` (the slider plus its masks', −1..1) renders with the model.
#[inline]
pub fn clarity_by_model(amount: f32) -> bool {
    amount > 0. || (CLARITY_NEGATIVE_MEASURED && amount < 0.)
}
/// Clarity's gain at each pixel's own level, so detail finer than the map gains too: Camera Raw's
/// Clarity +100 multiplies log contrast by 2.16 at every period from 3 to 233 px (round 11's
/// frequency scene), where RAWmakase's gains made at the map's own levels give 1.2 at 3 px.
pub const CLARITY_FINE: bool = true;
/// Render positive Dehaze with Camera Raw's haze model; `false`: LightCraft's dark-channel mix.
pub const HAZE_MEASURED: bool = true;
/// Render Highlights / Shadows of Adobe-profiled raws with Camera Raw's global rule; `false`:
/// LightCraft's preview-fitted tables (`tone::lr::adobe_hs_lut_previews`). On since round 12, with
/// the photo's default black (`black_cr`) and [`SHADOWS_UP_SCALE`]: Bryan's acceptance set 3.99 →
/// 3.77 ΔE, photos passing 25 → 31, Summer Fields · R6 5.83 → 4.63 (`docs/lr-match.md` → Round 12).
pub const HS_MEASURED: bool = true;

/// Whether Highlights / Shadows render with the rule: [`HS_MEASURED`], or `LIGHTCRAFT_HS_CR=1|0` in
/// the environment (measurement runs: the oracle and the acceptance gate compare both with one
/// build), read once; [`force_hs_measured`] wins.
pub fn hs_measured() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    HS_FORCED.load(std::sync::atomic::Ordering::Relaxed)
        || *ON.get_or_init(|| match std::env::var("LIGHTCRAFT_HS_CR").as_deref() {
            Ok("1") => true,
            Ok("0") => false,
            _ => HS_MEASURED,
        })
}

static HS_FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Render Highlights / Shadows with the rule from now on in this process (tests and measurement
/// tools; the app follows [`HS_MEASURED`]).
pub fn force_hs_measured(on: bool) {
    HS_FORCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Highlights / Shadows read the photo before Exposure (round 8's `hsorder` reading); `false`:
/// RAWmakase's map at the render's Exposure, which round 11 measured closer on Bryan's tone recipe
/// (Exposure −1.1 with H +30 / S +69: 3.37 before Exposure, 3.22 at it, mean of 9 raws).
pub const HS_BEFORE_EXPOSURE: bool = false;

/// [`HS_BEFORE_EXPOSURE`], or `LIGHTCRAFT_HS_PRE=1|0` in the environment (measurement runs), read once.
pub fn hs_before_exposure() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("LIGHTCRAFT_HS_PRE").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => HS_BEFORE_EXPOSURE,
    })
}

/// Long edge of the map (RAWmakase's `MAP_EDGE`).
pub const MAP_EDGE: usize = 512;
const RADIUS: f32 = 0.032;
const EPSILON: f32 = 0.5;
/// The darkest luminance the map tells apart (−14 EV).
pub const FLOOR: f32 = 6.103_515_6e-5;

/// log2 of a scene luminance as the map reads it (finite for every input).
#[inline]
pub fn level(y: f32) -> f32 {
    if y.is_nan() { FLOOR.log2() } else { y.clamp(FLOOR, 1e9).log2() }
}

/// Luminance of linear ProPhoto RGB.
#[inline]
pub fn luminance_pp(p: [f32; 3]) -> f32 {
    0.288_040_2 * p[0] + 0.711_874_1 * p[1] + 0.000_085_7 * p[2]
}

/// The map's size for a `w × h` render.
pub fn map_size(w: usize, h: usize) -> (usize, usize) {
    let long = w.max(h);
    if long <= MAP_EDGE {
        return (w, h);
    }
    let k = MAP_EDGE as f64 / long as f64;
    (((w as f64 * k).round() as usize).clamp(1, MAP_EDGE), ((h as f64 * k).round() as usize).clamp(1, MAP_EDGE))
}

fn finite(v: f32) -> f32 {
    if v.is_finite() { v.clamp(-1e6, 1e6) } else { 0.0 }
}

/// Means of `img` over the map's blocks (RAWmakase's `box_reduce`).
fn box_reduce(img: &Rgb32f, mw: usize, mh: usize) -> Vec<[f32; 3]> {
    let (w, h) = (img.width, img.height);
    let mut out = vec![[0f32; 3]; mw * mh];
    par_rows(&mut out, mw, |y, row| {
        let y0 = (y * h / mh).min(h.saturating_sub(1));
        let y1 = ((y + 1) * h / mh).max(y0 + 1).min(h);
        for (x, o) in row.iter_mut().enumerate() {
            let x0 = (x * w / mw).min(w.saturating_sub(1));
            let x1 = ((x + 1) * w / mw).max(x0 + 1).min(w);
            let mut s = [0f64; 3];
            for yy in y0..y1 {
                for p in img.data.get(yy * w + x0..yy * w + x1).unwrap_or(&[]) {
                    for c in 0..3 {
                        s[c] += f64::from(finite(p[c]));
                    }
                }
            }
            let n = ((x1 - x0) * (y1 - y0)).max(1) as f64;
            *o = s.map(|v| (v / n) as f32);
        }
    });
    out
}

/// The map of `img` (white-balanced, before Exposure): its block means, `None` for an empty image.
pub fn reduce(img: &Rgb32f) -> Option<(Vec<[f32; 3]>, usize, usize)> {
    if img.width == 0 || img.height == 0 || img.data.len() != img.width * img.height {
        return None;
    }
    let (mw, mh) = map_size(img.width, img.height);
    Some((box_reduce(img, mw, mh), mw, mh))
}

// ---------------------------------------------------------------- box filters

/// One line of a (2r+1)-wide mean, clamped at the ends, via a running sum.
fn box_line(len: usize, r: usize, get: impl Fn(usize) -> f32, mut put: impl FnMut(usize, f32)) {
    if len == 0 {
        return;
    }
    let last = len as isize - 1;
    let g = |i: isize| f64::from(get(i.clamp(0, last) as usize));
    let ri = r as isize;
    let mut sum: f64 = (-ri..=ri).map(g).sum();
    let n = (2 * r + 1) as f64;
    for i in 0..len {
        put(i, (sum / n) as f32);
        let i = i as isize;
        sum += g(i + ri + 1) - g(i - ri);
    }
}

/// Mean over a (2r+1)² window, clamped at the borders (RAWmakase's `local_tone::blur`).
fn blur(x: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut rows = vec![0f32; x.len()];
    for y in 0..h {
        box_line(w, r, |i| x[y * w + i], |i, v| rows[y * w + i] = v);
    }
    let mut out = vec![0f32; x.len()];
    for c in 0..w {
        box_line(h, r, |i| rows[i * w + c], |i, v| out[i * w + c] = v);
    }
    out
}

/// Minimum over a (2r+1)² window, clamped at the borders: rows, then columns.
fn min_filter(x: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let line = |len: usize, get: &dyn Fn(usize) -> f32, out: &mut dyn FnMut(usize, f32)| {
        for i in 0..len {
            let (a, b) = (i.saturating_sub(r), (i + r).min(len - 1));
            out(i, (a..=b).map(get).fold(f32::INFINITY, f32::min));
        }
    };
    let mut rows = vec![0f32; x.len()];
    for y in 0..h {
        line(w, &|i| x[y * w + i], &mut |i, v| rows[y * w + i] = v);
    }
    let mut out = vec![0f32; x.len()];
    for c in 0..w {
        line(h, &|i| rows[i * w + c], &mut |i, v| out[i * w + c] = v);
    }
    out
}

/// He et al.'s guided filter of `x` with itself as the guide, box radius `r`: the coefficients
/// `(A, B)` (blurred), so the filtered value is `A · x + B`.
fn guided_coefficients(x: &[f32], w: usize, h: usize, r: usize, epsilon: f32) -> (Vec<f32>, Vec<f32>) {
    let sq: Vec<f32> = x.iter().map(|v| v * v).collect();
    let (m, m2) = (blur(x, w, h, r), blur(&sq, w, h, r));
    let a: Vec<f32> = m
        .iter()
        .zip(&m2)
        .map(|(m, m2)| {
            let var = (m2 - m * m).max(0.);
            var / (var + epsilon)
        })
        .collect();
    let b: Vec<f32> = m.iter().zip(&a).map(|(m, a)| m - a * m).collect();
    (blur(&a, w, h, r), blur(&b, w, h, r))
}

fn percentile(v: &[f32], q: f32) -> f32 {
    let mut v = v.to_vec();
    if v.is_empty() {
        return 0.0;
    }
    let k = (((v.len() - 1) as f32 * q) as usize).min(v.len() - 1);
    *v.select_nth_unstable_by(k, f32::total_cmp).1
}

// ---------------------------------------------------------------- the map

/// What the map takes from the photo whatever the sliders (RAWmakase's `MapBase`): log2
/// luminance, the guided base's coefficients and Clarity's key, plus (made the first time a
/// render reads them) Clarity's detail and the Highlights / Shadows rule's statistics.
pub struct MapBase {
    pub w: usize,
    pub h: usize,
    logs: Vec<f32>,
    a: Vec<f32>,
    b: Vec<f32>,
    clarity_key: f32,
    /// The photo's dark key before Exposure: the 0.1th percentile of the map's log2 luminance
    /// ([`crate::black_cr::dark_of`]).
    pub dark: f32,
    detail: OnceLock<ClarityDetail>,
    rule: OnceLock<RuleStats>,
    /// The map covers the whole uncropped photo ([`MapBase::whole_photo`]), not the frame of
    /// the render that made it: a render finds its pixels on it through [`SceneTone::framed`].
    pub whole: bool,
}

impl std::fmt::Debug for MapBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapBase").field("w", &self.w).field("h", &self.h).finish_non_exhaustive()
    }
}

impl MapBase {
    /// The map of `img` (before Exposure).
    pub fn of(img: &Rgb32f) -> Option<MapBase> {
        let (px, w, h) = reduce(img)?;
        Self::new(px.iter().map(|p| luminance_2020(*p)).collect(), w, h)
    }

    /// From scene luminance on a `w × h` grid.
    pub fn new(lum: Vec<f32>, w: usize, h: usize) -> Option<MapBase> {
        if w == 0 || h == 0 || lum.len() != w * h {
            return None;
        }
        let logs: Vec<f32> = lum.iter().map(|y| level(*y)).collect();
        let r = ((RADIUS * w.max(h) as f32).round() as usize).max(1);
        let (a, b) = guided_coefficients(&logs, w, h, r, EPSILON);
        let clarity_key = 0.25 * percentile(&logs, 0.999) + 0.75 * percentile(&logs, 0.99);
        let dark = crate::black_cr::dark_of(&logs).unwrap_or(FLOOR.log2());
        Some(MapBase { w, h, logs, a, b, clarity_key, dark, detail: OnceLock::new(), rule: OnceLock::new(), whole: false })
    }

    /// This map marked as the whole uncropped photo's: Camera Raw measures the whole image
    /// (its tone statistics, the dark key, Clarity's and the rule's levels, the local base) and
    /// crops afterwards, so every render of the photo, whatever its crop, straighten angle or
    /// size, reads this one map.
    pub fn whole_photo(mut self) -> MapBase {
        self.whole = true;
        self
    }

    /// The guided base's coefficients on the map (the base is `A · level + B`), before Exposure.
    pub fn coefficients(&self) -> (&[f32], &[f32]) {
        (&self.a, &self.b)
    }

    fn bases(&self) -> Vec<f32> {
        self.logs.iter().zip(self.a.iter().zip(&self.b)).map(|(l, (a, b))| a * l + b).collect()
    }

    /// Clarity's detail.
    /// Made outside the lock (as RAWmakase does): a render waiting on it inside parallel work
    /// could otherwise be the task the maker's own parallel work needs; two renders making it at
    /// once make the same detail.
    pub fn detail(&self) -> &ClarityDetail {
        if let Some(d) = self.detail.get() {
            return d;
        }
        let d = ClarityDetail::of(&self.logs, &self.bases(), self.w, self.h, self.clarity_key);
        self.detail.get_or_init(|| d)
    }

    fn rule(&self) -> &RuleStats {
        if let Some(r) = self.rule.get() {
            return r;
        }
        // Camera Raw's rule reads the photo as it renders: highlights past white are clipped (a sun or a lamp
        // far above white would otherwise stretch the photo's range and the rule would lift its shadows 3 EV
        // more than Camera Raw does)
        let ceil = rule_ceiling();
        let logs: Vec<f32> = self.logs.iter().map(|l| l.min(ceil)).collect();
        let bases: Vec<f32> = self.bases().into_iter().map(|b| b.min(ceil)).collect();
        let r = RuleStats::new(&logs, &bases, self.w, self.h);
        self.rule.get_or_init(|| r)
    }

    /// How far Shadows `sh` (−1..1, alone) moves the dark key Camera Raw's default black reads, at
    /// Exposure `ev` (RAWmakase `pipeline/tone.rs`): positive, the median of its gain over the map;
    /// negative, the median on the darkest 0.1 % of it.
    pub fn shadows_key_shift(&self, ev: f32, sh: f32) -> f32 {
        if sh == 0.0 || !sh.is_finite() {
            return 0.0;
        }
        let curves = self.hs_curves(ev);
        let bases = self.bases();
        let mut gains: Vec<f32> = if sh > 0.0 {
            bases.iter().map(|b| curves.gain(b + ev, sh, 0.0)).collect()
        } else {
            let count = (self.logs.len() / 1000).max(1);
            let mut order: Vec<usize> = (0..self.logs.len()).collect();
            if count < order.len() {
                order.select_nth_unstable_by(count - 1, |a, b| self.logs[*a].total_cmp(&self.logs[*b]));
            }
            order.truncate(count);
            order.iter().filter_map(|&i| bases.get(i)).map(|b| curves.gain(b + ev, sh, 0.0)).collect()
        };
        if gains.is_empty() {
            return 0.0;
        }
        let k = (gains.len() - 1) / 2;
        let g = *gains.select_nth_unstable_by(k, f32::total_cmp).1;
        if g.is_finite() { g } else { 0.0 }
    }

    /// The four unit curves (Shadows +100, Highlights −100, Shadows −100, Highlights +100) at
    /// Exposure `ev`, and how the two sliders act together.
    pub fn hs_curves(&self, ev: f32) -> HsCurves {
        let rule = self.rule();
        let units = rule.curves(ev);
        let pair = rule.pair(&units, ev);
        HsCurves { units, pair }
    }
}

// ---------------------------------------------------------------- Clarity

const SCALES: [f32; 4] = [0.004, 0.015, 0.05, 0.15];
const RANGE: f32 = 2.;
/// Range σ (EV) of the Cauchy weight Clarity's gains are upsampled with.
pub const UPSAMPLE_RANGE: f32 = 0.25;
const KNOT_LO: f32 = -8.;
const KNOT_HI: f32 = 0.5;
const KNOTS: usize = 9;
// RAWmakase `clarity_data.rs`: per blur scale the log2 gain per unit of detail at base levels from
// `KNOT_LO` to `KNOT_HI` relative to the photo's brightest levels, fitted to Camera Raw 18.7 at
// Clarity −100, −50, +50 and +100.
const WEIGHTS_M100: [[f32; KNOTS]; 4] = [
    [-0.0272, -0.1927, -0.0764, -0.0689, 0.0068, 0.0567, -0.0718, 0.0036, -0.0100],
    [-0.0118, -0.1986, -0.1022, -0.3007, -0.5374, -0.5815, -0.5830, -0.4256, -0.0465],
    [0.0353, 0.0537, 0.0165, 0.0106, 0.0613, -0.0626, -0.0075, 0.0230, 0.0015],
    [-0.0129, -0.0107, -0.0058, -0.0347, -0.0396, 0.0305, 0.0223, 0.0098, -0.0591],
];
const WEIGHTS_M50: [[f32; KNOTS]; 4] = [
    [-0.0168, -0.1137, -0.0451, -0.0490, 0.0023, 0.0330, -0.0431, 0.0020, -0.0053],
    [-0.0064, -0.1204, -0.0596, -0.1772, -0.3246, -0.3499, -0.3455, -0.2508, -0.0234],
    [0.0213, 0.0354, 0.0092, 0.0078, 0.0372, -0.0391, -0.0068, 0.0141, 0.0001],
    [-0.0078, -0.0079, -0.0029, -0.0225, -0.0251, 0.0186, 0.0153, 0.0058, -0.0346],
];
const WEIGHTS_50: [[f32; KNOTS]; 4] = [
    [0.2292, 0.0713, 0.2070, 0.1321, -0.0268, -0.0161, -0.0212, 0.0981, 0.0086],
    [0.0056, 0.4016, 0.2260, 0.3029, 0.2107, 0.1972, 0.1403, 0.0193, 0.0556],
    [0.0158, 0.1688, -0.0357, -0.0904, 0.0172, -0.2251, 0.0159, 0.2423, 0.1087],
    [0.3042, 0.2691, 0.4679, 0.3970, 0.3296, 0.4703, 0.2079, -0.1511, -0.0771],
];
const WEIGHTS_100: [[f32; KNOTS]; 4] = [
    [0.5227, 0.1151, 0.3593, 0.2642, -0.0568, -0.0340, -0.0428, 0.1978, 0.0093],
    [0.1186, 0.7805, 0.4183, 0.6086, 0.4204, 0.3962, 0.2819, 0.0322, 0.0939],
    [0.0989, 0.2990, 0.0878, -0.2082, 0.0490, -0.4586, 0.0347, 0.4822, 0.2054],
    [0.5386, 0.5871, 0.7840, 0.8144, 0.6365, 0.9440, 0.4094, -0.3026, -0.1482],
];

// Round 11's refit for evaluation at each pixel's own level (`CLARITY_FINE`): the same model with a
// fifth row, a gain by the base level alone (Camera Raw's Clarity also moves regions' levels:
// −20 L* on a hazy landscape's ridges at +100), fitted by ridge regression towards RAWmakase's
// weights on our oracle (`oracle/make_r11.py`: 5 synthetic scenes and 9 of Bryan's raws, Camera
// Raw's change of scene log2 luminance read through its own base render; `oracle/clafit.py`).
const FINE_M100: [[f32; KNOTS]; 5] = [
    [-0.0370, -0.1170, -0.0416, -0.0379, 0.0033, 0.0252, 0.0392, 0.0545, 0.0008],
    [-0.0290, -0.1223, -0.0966, -0.2805, -0.5007, -0.5871, -0.4988, -0.3701, -0.1018],
    [0.0403, 0.0780, 0.0391, 0.0421, 0.0580, -0.0475, 0.0339, 0.0028, -0.0070],
    [-0.0088, 0.0169, 0.0122, 0.0228, 0.0025, 0.0052, 0.0330, -0.0414, -0.0684],
    [0.0007, -0.0004, 0.0015, 0.0300, 0.0076, 0.0170, -0.0020, -0.0028, -0.0104],
];
const FINE_M50: [[f32; KNOTS]; 5] = [
    [-0.0181, -0.0557, -0.0159, -0.0398, -0.0234, -0.0200, 0.0091, 0.0242, -0.0016],
    [-0.0125, -0.0591, -0.0504, -0.1845, -0.3080, -0.3525, -0.2971, -0.2320, -0.0615],
    [0.0215, 0.0435, 0.0103, -0.0120, 0.0282, -0.0061, 0.0212, -0.0104, -0.0090],
    [-0.0113, -0.0063, 0.0031, 0.0073, 0.0083, 0.0177, 0.0145, -0.0324, -0.0436],
    [-0.0012, 0.0042, -0.0064, 0.0300, 0.0269, 0.0104, -0.0075, 0.0033, -0.0049],
];
const FINE_50: [[f32; KNOTS]; 5] = [
    [0.1888, 0.0569, 0.0940, 0.0546, 0.0167, 0.0115, -0.0492, 0.0335, 0.0122],
    [0.0572, 0.2778, 0.1781, 0.2448, 0.2001, 0.1644, 0.0980, 0.0393, 0.0506],
    [-0.0194, 0.0295, -0.0812, -0.0484, -0.0586, -0.2130, -0.0726, 0.2035, 0.1077],
    [0.2336, 0.1798, 0.3723, 0.4113, 0.3714, 0.5355, 0.2140, 0.0281, -0.0589],
    [0.0158, 0.1505, 0.1790, 0.1603, 0.0724, -0.0709, -0.1611, -0.1758, 0.0284],
];
const FINE_100: [[f32; KNOTS]; 5] = [
    [0.4108, 0.1015, 0.1677, 0.1177, 0.0299, 0.0193, -0.0940, 0.0758, 0.0192],
    [0.1624, 0.5436, 0.3217, 0.4884, 0.3956, 0.3252, 0.2056, 0.0816, 0.0879],
    [-0.0194, 0.0489, -0.0711, -0.1062, -0.1028, -0.4205, -0.1344, 0.3970, 0.2092],
    [0.3884, 0.3868, 0.6734, 0.8267, 0.7253, 1.0242, 0.3804, 0.0017, -0.1111],
    [0.0492, 0.3224, 0.3591, 0.3106, 0.1390, -0.1261, -0.3197, -0.3286, 0.0892],
];

/// The weights at a fitted Clarity (−1, −0.5, 0.5, 1): four scales' detail weights, then (fine
/// evaluation) the level row.
fn weights(amount: f32) -> &'static [[f32; KNOTS]] {
    let fine = CLARITY_FINE;
    if amount <= -1. {
        if fine { &FINE_M100 } else { &WEIGHTS_M100 }
    } else if amount < 0. {
        if fine { &FINE_M50 } else { &WEIGHTS_M50 }
    } else if amount < 1. {
        if fine { &FINE_50 } else { &WEIGHTS_50 }
    } else if fine {
        &FINE_100
    } else {
        &WEIGHTS_100
    }
}

/// Rewrite Σ weight · (log Y − blur) (+ the level row) as slope · log Y + offset at `base`
/// (relative to the key).
fn coefficients(weights: &[[f32; KNOTS]], base: f32, blurs: [f32; 4]) -> [f32; 2] {
    let step = (KNOT_HI - KNOT_LO) / (KNOTS - 1) as f32;
    let u = ((base - KNOT_LO) / step).clamp(0., (KNOTS - 1) as f32);
    let k = (u as usize).min(KNOTS - 2);
    let t = u - k as f32;
    let at = |w: &[f32; KNOTS]| w[k] * (1. - t) + w[k + 1] * t;
    let [a, b] = weights.iter().take(4).zip(blurs).fold([0.; 2], |[a, b], (w, blur)| {
        let weight = at(w);
        [a + weight, b - weight * blur]
    });
    [a, b + clarity_level_scale() * weights.get(4).map_or(0., at)]
}

/// The fine Clarity's level row (its regional move) scaled by this.
pub const CLARITY_LEVEL_SCALE: f32 = 1.0;

/// [`CLARITY_LEVEL_SCALE`], or `LIGHTCRAFT_CLA_LEVEL=<scale>` in the environment (measurement runs), read once.
pub fn clarity_level_scale() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("LIGHTCRAFT_CLA_LEVEL")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && (0.0..=4.0).contains(v))
            .unwrap_or(CLARITY_LEVEL_SCALE)
    })
}

/// The bracket of fitted positions for `amount` (RAWmakase's `bracket`): the first fit (0 for
/// −0.5 / −1, 2 for +0.5 / +1) and the weights of it and the next.
pub fn bracket(amount: f32) -> (usize, [f32; 2]) {
    let a = if amount.is_finite() { amount.clamp(-1., 1.) } else { 0. };
    let half = if a < 0. { 0 } else { 2 };
    let a = a.abs() * 2.;
    (half, if a <= 1. { [a, 0.] } else { [2. - a, a - 1.] })
}

/// Clarity's measured gains on the map at −0.5, −1, +0.5, +1, and their log-luminance guide
/// (before Exposure).
pub struct ClarityDetail {
    /// Per fit, each cell's gain as `slope · level + offset` (Σ weight · (level − blur)).
    pub slopes: [Vec<f32>; 4],
    pub offsets: [Vec<f32>; 4],
    pub guide: Vec<f32>,
    /// The four blurs and each cell's base level less the key (the terms `clarity_terms` reads).
    pub blurs: [Vec<f32>; 4],
    pub rel: Vec<f32>,
}

impl ClarityDetail {
    pub fn of(logs: &[f32], base: &[f32], w: usize, h: usize, key: f32) -> ClarityDetail {
        let long = w.max(h) as f32;
        let blurs = bilateral(logs, w, h, SCALES.map(|s| s * long), RANGE);
        let fits = [-0.5f32, -1., 0.5, 1.].map(|amount| {
            let wt = weights(amount);
            base.iter().enumerate().map(|(i, b)| coefficients(wt, b - key, std::array::from_fn(|s| blurs[s][i]))).collect::<Vec<_>>()
        });
        ClarityDetail {
            slopes: std::array::from_fn(|k| fits[k].iter().map(|c| c[0]).collect()),
            offsets: std::array::from_fn(|k| fits[k].iter().map(|c| c[1]).collect()),
            guide: logs.to_vec(),
            rel: base.iter().map(|b| b - key).collect(),
            blurs,
        }
    }
}

/// A blurred level on a grid `factor` times coarser than the map.
struct Grid {
    values: Vec<f32>,
    width: usize,
    height: usize,
    factor: usize,
}

impl Grid {
    fn sample(&self, x: usize, y: usize) -> f32 {
        let v = &self.values;
        if self.factor == 1 {
            return v[y * self.width + x];
        }
        let f = self.factor as f32;
        let fx = ((x as f32 + 0.5) / f - 0.5).clamp(0., (self.width - 1) as f32);
        let fy = ((y as f32 + 0.5) / f - 0.5).clamp(0., (self.height - 1) as f32);
        let (ix, iy) = (fx as usize, fy as usize);
        let (jx, jy) = ((ix + 1).min(self.width - 1), (iy + 1).min(self.height - 1));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let top = v[iy * self.width + ix] * (1. - tx) + v[iy * self.width + jx] * tx;
        let bottom = v[jy * self.width + ix] * (1. - tx) + v[jy * self.width + jx] * tx;
        top * (1. - ty) + bottom * ty
    }
}

/// Means of `f × f` blocks (partial at the right and bottom edges).
fn block_means(x: &[f32], w: usize, h: usize, f: usize) -> (Vec<f32>, usize, usize) {
    if f <= 1 {
        return (x.to_vec(), w, h);
    }
    let (gw, gh) = (w.div_ceil(f), h.div_ceil(f));
    let mut sum = vec![0f32; gw * gh];
    let mut count = vec![0u32; gw * gh];
    for (i, v) in x.iter().enumerate() {
        let g = (i / w / f) * gw + (i % w) / f;
        sum[g] += v;
        count[g] += 1;
    }
    (sum.iter().zip(&count).map(|(s, c)| s / (*c).max(1) as f32).collect(), gw, gh)
}

/// A Gaussian blur of σ `sigma` px as three box blurs, clamped at the borders.
fn gauss(x: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    if sigma < 0.5 {
        return x.to_vec();
    }
    let r = (((12. * sigma * sigma / 3. + 1.).sqrt() / 2.).round() as usize).max(1);
    let mut a = x.to_vec();
    let mut b = vec![0f32; x.len()];
    for _ in 0..3 {
        for y in 0..h {
            box_line(w, r, |i| a[y * w + i], |i, v| b[y * w + i] = v);
        }
        std::mem::swap(&mut a, &mut b);
    }
    for _ in 0..3 {
        for c in 0..w {
            box_line(h, r, |i| a[i * w + c], |i, v| b[i * w + c] = v);
        }
        std::mem::swap(&mut a, &mut b);
    }
    a
}

/// Edge-aware blurs of `x` at spatial σ `sigmas` (px) and range σ `range`: the piecewise linear
/// bilateral filter of Durand and Dorsey (RAWmakase's `clarity::bilateral`).
fn bilateral<const N: usize>(x: &[f32], w: usize, h: usize, sigmas: [f32; N], range: f32) -> [Vec<f32>; N] {
    let (lo, hi) = x.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
    let (lo, hi) = if lo.is_finite() && hi.is_finite() { (lo, hi) } else { (0., 0.) };
    let step = range / 2.5;
    let count = ((((hi - lo + 2. * range) / step).floor() as usize) + 1).clamp(2, 256);
    let level = |j: usize| lo - range + step * j as f32;
    let factors = sigmas.map(|s| ((s / 3.).floor() as usize).max(1));
    let mut filtered: Vec<Vec<Grid>> = (0..count).map(|_| Vec::new()).collect();
    par_rows(&mut filtered, 1, |j, slot| {
        let l = level(j);
        let weight: Vec<f32> = x
            .iter()
            .map(|v| {
                let d = (v - l) / range;
                (-0.5 * d * d).exp()
            })
            .collect();
        let weighted: Vec<f32> = weight.iter().zip(x).map(|(a, v)| a * v).collect();
        if let Some(out) = slot.first_mut() {
            *out = (0..N)
                .map(|s| {
                    let f = factors[s];
                    let (den, gw, gh) = block_means(&weight, w, h, f);
                    let (num, ..) = block_means(&weighted, w, h, f);
                    let sigma = sigmas[s] / f as f32;
                    let (den, num) = (gauss(&den, gw, gh, sigma), gauss(&num, gw, gh, sigma));
                    Grid { values: num.iter().zip(&den).map(|(n, d)| n / d.max(1e-12)).collect(), width: gw, height: gh, factor: f }
                })
                .collect();
        }
    });
    std::array::from_fn(|s| {
        (0..x.len())
            .map(|i| {
                let v = x[i];
                let j = (((v - lo + range) / step).floor().max(0.) as usize).min(count - 2);
                let t = ((v - level(j)) / step).clamp(0., 1.);
                let (px, py) = (i % w, i / w);
                let at = |k: usize| filtered.get(k).and_then(|g| g.get(s)).map_or(v, |g| g.sample(px, py));
                at(j) * (1. - t) + at(j + 1) * t
            })
            .collect()
    })
}

// ---------------------------------------------------------------- positive Dehaze

const HAZE_RADIUS: f32 = 0.005;
const HAZE_EPSILON: f32 = 0.01;
/// The lowest transmission.
pub const MIN_TRANSMISSION: f32 = 0.3;
/// The share of the colour-preserving result no channel falls below.
pub const CHANNEL_FLOOR: f32 = 0.5;
/// Dehaze amounts and the fraction of the haze they remove; linear between them.
pub const OMEGA: [(f32, f32); 4] = [(0., 0.), (0.2, 0.3), (0.4, 0.45), (1., 0.9)];

/// A photo's haze, from the map in linear ProPhoto at Exposure 0 (RAWmakase's `Haze`).
#[derive(Debug, PartialEq)]
pub struct Haze {
    pub w: usize,
    pub h: usize,
    /// The airlight per ProPhoto channel.
    pub air: [f32; 3],
    /// The haze's density on the map's grid.
    pub density: Vec<f32>,
    /// Of the whole uncropped photo, as [`MapBase::whole`].
    pub whole: bool,
}

impl Haze {
    /// This haze marked as the whole uncropped photo's ([`MapBase::whole_photo`]).
    pub fn whole_photo(mut self) -> Haze {
        self.whole = true;
        self
    }

    /// The haze of `img` (linear Rec.2020, before Exposure).
    pub fn of(img: &Rgb32f) -> Option<Haze> {
        let (px, w, h) = reduce(img)?;
        let (to, _) = crate::colorops::pp_matrices();
        let pp: Vec<[f32; 3]> = px.iter().map(|c| mul3(to, *c)).collect();
        Some(Self::of_pp(&pp, w, h))
    }

    /// The haze of `scene`, `w × h` linear ProPhoto values.
    pub fn of_pp(scene: &[[f32; 3]], w: usize, h: usize) -> Haze {
        if scene.is_empty() || scene.len() != w * h {
            return Haze { w: 1, h: 1, air: [1.; 3], density: vec![0.], whole: false };
        }
        let r = ((HAZE_RADIUS * w.max(h) as f32).round() as usize).max(1);
        let dark = min_filter(&scene.iter().map(|p| p[0].min(p[1]).min(p[2])).collect::<Vec<_>>(), w, h, r);
        // the haziest 1 %: at or above the dark channel's 99th percentile, its edge softened over
        // 1 % of the level so the airlight moves continuously with the photo (a CPU and a GPU
        // render of one photo, whose values differ in the last bits, get the same airlight)
        let threshold = percentile(&dark, 0.99);
        let soft = (0.01 * threshold.abs()).max(1e-6);
        let (mut sum, mut n) = ([0f64; 3], 0f64);
        for (p, d) in scene.iter().zip(&dark) {
            let k = f64::from(((d - threshold) / soft + 0.5).clamp(0., 1.));
            if k > 0. {
                (0..3).for_each(|c| sum[c] += k * f64::from(p[c]));
                n += k;
            }
        }
        let air = sum.map(|v| ((v / n.max(1e-9)) as f32).max(1e-6));
        let relative: Vec<f32> = scene.iter().map(|p| (p[0] / air[0]).min(p[1] / air[1]).min(p[2] / air[2])).collect();
        let raw = min_filter(&relative, w, h, r);
        let (a, b) = guided_coefficients(&raw, w, h, 2 * r, HAZE_EPSILON);
        let density = raw.iter().zip(a.iter().zip(&b)).map(|(v, (a, b))| a * v + b).collect();
        Haze { w, h, air, density, whole: false }
    }
}

/// The fraction of the haze Dehaze `s` (0 to 1) removes.
pub fn omega(s: f32) -> f32 {
    let s = if s.is_finite() { s.clamp(0., 1.) } else { 0. };
    let j = OMEGA.windows(2).position(|w| s <= w[1].0).unwrap_or(OMEGA.len() - 2);
    let ((s0, w0), (s1, w1)) = (OMEGA[j], OMEGA[j + 1]);
    w0 + (w1 - w0) * (s - s0) / (s1 - s0)
}

/// A linear ProPhoto pixel `p` moved away from the airlight `air` by transmission `t`, no
/// channel below [`CHANNEL_FLOOR`] of `p` scaled by the luminance's change.
pub fn dehaze_pp(p: [f32; 3], air: [f32; 3], t: f32) -> [f32; 3] {
    let j: [f32; 3] = std::array::from_fn(|c| (p[c] - air[c]) / t + air[c]);
    let ratio = luminance_pp(j).max(0.) / luminance_pp(p).max(1e-9);
    std::array::from_fn(|c| j[c].max(CHANNEL_FLOOR * p[c] * ratio))
}

fn mul3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

// ---------------------------------------------------------------- Highlights / Shadows

/// Positive Shadows' curve scaled by this (round 12: on 40 of Bryan's split-A raws, Camera Raw
/// against LightCraft, the rule lifted the darks by a median 1.12× Camera Raw's, R6, M262 and
/// GR III alike; at 0.89 the oracle's real-raw H/S renders go 1.92 / 1.50 / 1.91 → 1.77 / 1.38 /
/// 1.87 ΔE on its three sets, Shadows +100 alone on the R6 4.63 → 2.72). 0.88 since round 15, with
/// [`SHADOWS_UP_SPREAD`] on top.
pub const SHADOWS_UP_SCALE: f32 = 0.88;

/// [`SHADOWS_UP_SCALE`], or `LIGHTCRAFT_HS_SUP=<scale>` in the environment (measurement runs), read once.
pub fn shadows_up_scale() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("LIGHTCRAFT_HS_SUP")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && (0.0..=2.0).contains(v))
            .unwrap_or(SHADOWS_UP_SCALE)
    })
}

/// Positive Shadows' curve adds this many times the members' spread (`up_spread`, above [`SHADOWS_UP_GATE`]) to its
/// scaled mean. Round 15: where the four networks disagree the photo is unlike the ones they were fitted on (BHM_4860,
/// a black jacket in front of a bright waterfall: members' Shadows +100 gain at −7 EV 0.5 … 2.6) and Camera Raw lifts
/// it near the highest member; where they agree the mean is right. On 38 raws (Camera Raw's Shadows +100 / +50
/// renders of Bryan's R6 / M262 / GR III / A7S III photos) mean ΔE 2.50 / 1.55 → 2.33 / 1.49, BHM_4860 9.90 → 4.34,
/// BHM_9815 5.44 → 4.98, no raw worse by more than 0.17.
pub const SHADOWS_UP_SPREAD: f32 = 2.2;

/// [`SHADOWS_UP_SPREAD`], or `LIGHTCRAFT_HS_SPREAD=<k>` in the environment (measurement runs), read once.
pub fn shadows_up_spread() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("LIGHTCRAFT_HS_SPREAD")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && (0.0..=8.0).contains(v))
            .unwrap_or(SHADOWS_UP_SPREAD)
    })
}

/// The brightest level (EV from white, before Exposure) the rule reads. Round 15: a sun or a lamp far above white
/// (DSC00603: +7 EV, a 15 EV range) stretched the photo's range and the rule lifted its shadows ~3.7 EV where Camera
/// Raw lifts 0.4 (Shadows +100 ΔE 43.6 → 4.8); at +2 no photo of the 38 whose tops reach +1.8 changes, while
/// a ceiling at white (0) breaks the ones with highlights past it (R0002574 2.27 → 4.45, DSC01353 3.32 → 5.58).
pub const RULE_CEILING: f32 = 2.0;

/// [`RULE_CEILING`], or `LIGHTCRAFT_HS_CEIL=<EV>` in the environment (measurement runs), read once.
pub fn rule_ceiling() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("LIGHTCRAFT_HS_CEIL")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && (-4.0..=100.0).contains(v))
            .unwrap_or(RULE_CEILING)
    })
}

/// Spread below which the members agree and [`SHADOWS_UP_SPREAD`] adds nothing.
pub const SHADOWS_UP_GATE: f32 = 0.15;

/// [`SHADOWS_UP_GATE`], or `LIGHTCRAFT_HS_GATE=<spread>` in the environment (measurement runs), read once.
pub fn shadows_up_gate() -> f32 {
    static S: OnceLock<f32> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var("LIGHTCRAFT_HS_GATE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .filter(|v| v.is_finite() && (0.0..=4.0).contains(v))
            .unwrap_or(SHADOWS_UP_GATE)
    })
}

/// Levels at which a curve samples the rule.
pub const LEVELS: usize = 48;
const TEXTURE_RADIUS: usize = 2;
const TOP_BAND: f32 = 2.;
const TEX_CLAMP: f32 = 0.6;
const TEX_TOP_CLAMP: f32 = 0.05;
const COARSE_RADIUS: usize = 12;
const LEVEL_WINDOW: f32 = 0.5;
const TEX_AT_CLAMP: f32 = 1.0;
const LEVEL_MIN: usize = 50;
const BOTTOM_GAP: f32 = 3.5;
const PAIR_LOW: f32 = 0.01;
const PAIR_HIGH: f32 = 0.99;
const PAIR_EXTREME_MIN: f32 = 0.05;
/// How a slider's share of the pair blend sets in with the other's ratio to it (same signs: a power;
/// opposite signs: 1 − e^(−rate · ratio), normalized).
pub const PAIR_SAME_POWER: f32 = 1.7;
pub const PAIR_OPPOSITE_RATE: f32 = 2.4;

/// A unit curve: log2 gain at 48 absolute base levels from `lo` to `hi` (held outside).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub lo: f32,
    pub hi: f32,
    pub table: [f32; LEVELS],
}

impl Curve {
    pub fn eval(&self, base: f32) -> f32 {
        let span = (self.hi - self.lo).max(1e-6);
        let f = ((base - self.lo) / span * LEVELS as f32 - 0.5).clamp(0., (LEVELS - 1) as f32);
        let f = if f.is_finite() { f } else { 0. };
        let i = (f as usize).min(LEVELS - 2);
        self.table[i] + (self.table[i + 1] - self.table[i]) * (f - i as f32)
    }
}

/// Shadows and Highlights together on one photo (RAWmakase's `local_tone_rule::Pair`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    /// Each unit curve's gain at the photo's extreme for its control.
    pub extremes: [f32; 4],
    /// Per sign combination: Shadows' and Highlights' fraction at equal sliders.
    pub fractions: [[f32; 2]; 4],
}

const KINDS: [[usize; 2]; 4] = [[0, 3], [2, 1], [0, 1], [2, 3]];

impl Pair {
    fn combination(shadows: f32, highlights: f32) -> usize {
        match (shadows > 0., highlights > 0.) {
            (true, true) => 0,
            (false, false) => 1,
            (true, false) => 2,
            (false, true) => 3,
        }
    }

    /// For Shadows and Highlights at these slider values (−1..1): each one's fraction and extreme.
    pub fn at(&self, shadows: f32, highlights: f32) -> Option<[[f32; 2]; 2]> {
        if shadows == 0. || highlights == 0. || !shadows.is_finite() || !highlights.is_finite() {
            return None;
        }
        let c = Self::combination(shadows, highlights);
        let same = c < 2;
        let (s, h) = (shadows.abs().min(1.), highlights.abs().min(1.));
        let extreme = |slider: usize| self.extremes[KINDS[c][slider]];
        Some([[self.fractions[c][0] * ramp(h / s, same), extreme(0)], [self.fractions[c][1] * ramp(s / h, same), extreme(1)]])
    }
}

/// Scale of the pair blend's fractions per sign combination (0: both up, 1: both down, 2: Shadows up / Highlights
/// down, 3: the other way). Round 13: without Highlights' share with both sliders up the R6 night photos at
/// Shadows +69 / Highlights +30 come 3.62 → 1.88 ΔE closer to Camera Raw, but Bryan's own same-sign edits (M262,
/// Summer Fields) get darker and fail the acceptance gate (+0.24), so RAWmakase's fractions stay
/// (`docs/lr-match.md` → Round 13).
pub const PAIR_SCALE: [[f32; 2]; 4] = [[1., 1.], [1., 1.], [1., 1.], [1., 1.]];

/// [`PAIR_SCALE`], or for measurement `LIGHTCRAFT_HS_PAIR=c,shadows,highlights` (combination `c`'s pair), read once.
fn pair_scale(c: usize) -> [f32; 2] {
    static S: OnceLock<Option<(usize, [f32; 2])>> = OnceLock::new();
    let v = S.get_or_init(|| {
        let v: Vec<f32> = std::env::var("LIGHTCRAFT_HS_PAIR").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        (v.len() == 3 && v.iter().all(|x| x.is_finite())).then(|| (v[0] as usize, [v[1], v[2]]))
    });
    match v {
        Some((k, s)) if *k == c => *s,
        _ => PAIR_SCALE.get(c).copied().unwrap_or([1.; 2]),
    }
}

fn ramp(ratio: f32, same: bool) -> f32 {
    let r = ratio.clamp(0., 1.);
    if same { r.powf(PAIR_SAME_POWER) } else { (1. - (-PAIR_OPPOSITE_RATE * r).exp()) / (1. - (-PAIR_OPPOSITE_RATE).exp()) }
}

/// A render's Highlights / Shadows: the four unit curves (Shadows +100, Highlights −100, Shadows
/// −100, Highlights +100) at its Exposure, and how the two act together.
#[derive(Clone, Debug, PartialEq)]
pub struct HsCurves {
    pub units: [Curve; 4],
    pub pair: Pair,
}

impl HsCurves {
    /// log2 gain at base level `base` for Shadows `sh` and Highlights `hl` (−1..1; a mask's added
    /// to the slider's).
    pub fn gain(&self, base: f32, sh: f32, hl: f32) -> f32 {
        let blend = self.pair.at(sh, hl);
        let ruled = |up: usize, down: usize, slider: f32, which: usize| {
            if slider == 0. || !slider.is_finite() {
                return 0.;
            }
            let u = self.units[if slider > 0. { up } else { down }].eval(base);
            let g = match blend {
                Some(b) => u + b[which][0] * (b[which][1] - u),
                None => u,
            };
            g * slider.abs().min(1.)
        };
        ruled(0, 2, sh, 0) + ruled(3, 1, hl, 1)
    }

    /// GPU layout: per unit curve `lo, hi, table`, then the pair's extremes and fractions.
    pub fn words(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(4 * (LEVELS + 2) + 12);
        for u in &self.units {
            v.extend([u.lo, u.hi]);
            v.extend_from_slice(&u.table);
        }
        v.extend_from_slice(&self.pair.extremes);
        v.extend(self.pair.fractions.iter().flatten());
        v
    }
}

/// Histogram statistics of a photo's base levels (RAWmakase's `RuleStats`), at Exposure 0.
struct RuleStats {
    sorted: Vec<f32>,
    lo: f32,
    hi: f32,
    /// The first ensemble's Shadows +100 / Highlights −100 gains at the 48 levels (they read
    /// only levels relative to the photo, so Exposure does not move them).
    up_down: [[f32; 2]; LEVELS],
    /// The spread of the first ensemble's Shadows +100 members at the 48 levels.
    up_spread: [f32; LEVELS],
    /// The second ensemble's inputs at the 48 levels before standardizing; the last four (the
    /// photo's top, median and mean and the level) move with Exposure.
    ext: Vec<[f32; rule_ext::FEATURES]>,
}

fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.;
    }
    let mut v = values.to_vec();
    let n = v.len();
    let (lower, upper, _) = v.select_nth_unstable_by(n / 2, f32::total_cmp);
    if n % 2 == 1 {
        return *upper;
    }
    let below = lower.iter().copied().fold(f32::MIN, f32::max);
    (below + *upper) / 2.
}

fn silu(x: f32) -> f32 {
    x / (1. + (-x).exp())
}

/// The mean of an ensemble's two outputs for the standardized inputs `z`, and the spread (population
/// standard deviation over the members) of the first one: the members disagree where the photo is
/// unlike the ones the networks were fitted on.
#[allow(clippy::type_complexity)]
fn ensemble<const F: usize, const H: usize, const M: usize>(
    z: &[f32; F],
    w1: &[[[f32; F]; H]; M],
    b1: &[[f32; H]; M],
    w2: &[[[f32; H]; H]; M],
    b2: &[[f32; H]; M],
    w3: &[[[f32; H]; 2]; M],
    b3: &[[f32; 2]; M],
) -> ([f32; 2], f32) {
    let dot = |input: &[f32], weights: &[f32], bias: f32| input.iter().zip(weights).fold(bias, |s, (v, w)| s + w * v);
    let mut sum = [0f32; 2];
    let mut first_sq = 0f32;
    for m in 0..M {
        let h1: [f32; H] = std::array::from_fn(|j| silu(dot(z, &w1[m][j], b1[m][j])));
        let h2: [f32; H] = std::array::from_fn(|j| silu(dot(&h1, &w2[m][j], b2[m][j])));
        for (k, out) in sum.iter_mut().enumerate() {
            let v = dot(&h2, &w3[m][k], b3[m][k]);
            *out += v;
            if k == 0 {
                first_sq += v * v;
            }
        }
    }
    let mean = sum.map(|v| v / M as f32);
    let var = (first_sq / M as f32 - mean[0] * mean[0]).max(0.);
    (mean, var.sqrt())
}

/// Projects output log luminance (level + gain) of a table with columns `step` apart onto
/// non-decreasing values (pool adjacent violators).
fn monotone(table: &mut [f32; LEVELS], step: f32) {
    let mut sums = [0f32; LEVELS];
    let mut counts = [0usize; LEVELS];
    let mut blocks = 0;
    for (i, g) in table.iter().enumerate() {
        sums[blocks] = g + i as f32 * step;
        counts[blocks] = 1;
        blocks += 1;
        while blocks > 1 && sums[blocks - 2] / counts[blocks - 2] as f32 > sums[blocks - 1] / counts[blocks - 1] as f32 {
            sums[blocks - 2] += sums[blocks - 1];
            counts[blocks - 2] += counts[blocks - 1];
            blocks -= 1;
        }
    }
    let mut i = 0;
    for block in 0..blocks {
        for _ in 0..counts[block] {
            if counts[block] > 1 && i < LEVELS {
                table[i] = sums[block] / counts[block] as f32 - i as f32 * step;
            }
            i += 1;
        }
    }
}

struct Hist {
    sorted: Vec<f32>,
    top: f32,
    bot: f32,
    mean: f32,
    q: [f32; 5], // q10, q25, q50, q75, q90
    m_top1: f32,
    m_top2: f32,
    tex: f32,
    tex_top: f32,
    fine_by_level: Vec<f32>,
    coarse_by_level: Vec<f32>,
    fine_median: f32,
    coarse_median: f32,
}

impl Hist {
    fn texture_at(&self, x: f32) -> (f32, f32) {
        let lo = self.sorted.partition_point(|v| *v < x - LEVEL_WINDOW);
        let hi = self.sorted.partition_point(|v| *v <= x + LEVEL_WINDOW);
        if hi.saturating_sub(lo) < LEVEL_MIN {
            return (self.fine_median.min(TEX_AT_CLAMP), self.coarse_median.min(TEX_AT_CLAMP));
        }
        (median(&self.fine_by_level[lo..hi]).min(TEX_AT_CLAMP), median(&self.coarse_by_level[lo..hi]).min(TEX_AT_CLAMP))
    }

    fn cdf(&self, x: f32) -> f32 {
        self.sorted.partition_point(|v| *v <= x) as f32 / self.sorted.len().max(1) as f32
    }

    fn features(&self, x: f32, bot: f32) -> [f32; rule_data::FEATURES] {
        let m = |lo: f32, hi: f32| self.cdf(x + hi) - self.cdf(x + lo);
        let (fine, coarse) = self.texture_at(x);
        let [q10, q25, q50, q75, q90] = self.q;
        [
            self.top - x,
            x - bot,
            self.mean - x,
            q50 - x,
            q75 - x,
            q90 - x,
            q25 - x,
            q10 - x,
            self.cdf(x),
            self.top - bot,
            m(0., 1.),
            m(1., 2.),
            m(2., 3.),
            m(3., 5.),
            m(-1., 0.),
            m(-2., -1.),
            m(-3., -2.),
            m(-5., -3.),
            self.tex.min(TEX_CLAMP),
            self.tex_top.min(TEX_TOP_CLAMP) / TEX_TOP_CLAMP,
            fine,
            coarse,
            self.m_top1,
            self.m_top2,
        ]
    }
}

impl RuleStats {
    fn new(logs: &[f32], base: &[f32], w: usize, h: usize) -> RuleStats {
        let mut order: Vec<u32> = (0..base.len() as u32).collect();
        order.sort_by(|&a, &b| base[a as usize].total_cmp(&base[b as usize]));
        let sorted: Vec<f32> = order.iter().map(|&i| base[i as usize]).collect();
        let n = sorted.len().max(1);
        let q = |p: f32| sorted.get(((p * (n - 1) as f32) as usize).min(n - 1)).copied().unwrap_or(0.);
        let mean = (sorted.iter().map(|v| f64::from(*v)).sum::<f64>() / n as f64) as f32;
        let top = sorted.last().copied().unwrap_or(0.);
        let smooth = blur(logs, w, h, TEXTURE_RADIUS);
        let (mut sum, mut sum_top, mut count_top) = (0f64, 0f64, 0usize);
        for ((l, s), b) in logs.iter().zip(&smooth).zip(base) {
            let d = f64::from(l - s) * f64::from(l - s);
            sum += d;
            if *b > top - TOP_BAND {
                sum_top += d;
                count_top += 1;
            }
        }
        let tex = (sum / n as f64).sqrt() as f32;
        let tex_top = if count_top > 0 { (sum_top / count_top as f64).sqrt() as f32 } else { tex };
        let coarse = blur(logs, w, h, COARSE_RADIUS);
        let fine_by_level: Vec<f32> = order.iter().map(|&i| (logs[i as usize] - smooth[i as usize]).abs()).collect();
        let coarse_by_level: Vec<f32> = order.iter().map(|&i| (logs[i as usize] - coarse[i as usize]).abs()).collect();
        let hist = Hist {
            top,
            bot: q(0.001),
            mean,
            q: [q(0.1), q(0.25), q(0.5), q(0.75), q(0.9)],
            m_top1: 1. - sorted.partition_point(|v| *v <= top - 1.) as f32 / n as f32,
            m_top2: 1. - sorted.partition_point(|v| *v <= top - 2.) as f32 / n as f32,
            tex,
            tex_top,
            fine_median: median(&fine_by_level),
            coarse_median: median(&coarse_by_level),
            fine_by_level,
            coarse_by_level,
            sorted,
        };
        let (lo, hi) = (hist.bot - 1., hist.top + 1.);
        let step = (hi - lo) / LEVELS as f32;
        let floor = hist.bot.max(hist.q[0] - BOTTOM_GAP);
        let mut up_down = [[0f32; 2]; LEVELS];
        let mut up_spread = [0f32; LEVELS];
        let mut ext = Vec::with_capacity(LEVELS);
        for (i, ud) in up_down.iter_mut().enumerate() {
            let x = lo + (i as f32 + 0.5) * step;
            let f = hist.features(x, hist.bot);
            let z: [f32; rule_data::FEATURES] = std::array::from_fn(|k| (f[k] - rule_data::MU[k]) / rule_data::SD[k]);
            (*ud, up_spread[i]) = ensemble(&z, &rule_data::W1, &rule_data::B1, &rule_data::W2, &rule_data::B2, &rule_data::W3, &rule_data::B3);
            let (x2, f2) = if floor > hist.bot {
                let x2 = x.max(floor);
                (x2, hist.features(x2, floor))
            } else {
                (x, f)
            };
            let absolute = [hist.top, hist.q[2], hist.mean, x2];
            ext.push(std::array::from_fn(|k| if k < rule_data::FEATURES { f2[k] } else { absolute[k - rule_data::FEATURES] }));
        }
        RuleStats { sorted: hist.sorted, lo, hi, up_down, up_spread, ext }
    }

    /// The four unit curves at Exposure `ev`.
    fn curves(&self, ev: f32) -> [Curve; 4] {
        let ev = if ev.is_finite() { ev.clamp(-10., 10.) } else { 0. };
        let gains: Vec<[f32; 4]> = self
            .up_down
            .iter()
            .zip(&self.up_spread)
            .zip(&self.ext)
            .map(|((ud, spread), e)| {
                let z: [f32; rule_ext::FEATURES] = std::array::from_fn(|k| {
                    let v = if k >= rule_data::FEATURES { e[k] + ev } else { e[k] };
                    (v - rule_ext::MU[k]) / rule_ext::SD[k]
                });
                let ([sd, hu], _) = ensemble(&z, &rule_ext::W1, &rule_ext::B1, &rule_ext::W2, &rule_ext::B2, &rule_ext::W3, &rule_ext::B3);
                [ud[0] * shadows_up_scale() + (spread - shadows_up_gate()).max(0.) * shadows_up_spread(), ud[1], sd, hu]
            })
            .collect();
        let (lo, hi) = (self.lo + ev, self.hi + ev);
        let step = (hi - lo) / LEVELS as f32;
        std::array::from_fn(|k| {
            let lifts = k == 0 || k == 3;
            let mut table: [f32; LEVELS] = std::array::from_fn(|i| {
                let g = gains.get(i).map_or(0., |g| g[k]);
                let g = if g.is_finite() { g } else { 0. };
                if lifts { g.max(0.) } else { g.min(0.) }
            });
            monotone(&mut table, step);
            Curve { lo, hi, table }
        })
    }

    fn pair(&self, units: &[Curve; 4], ev: f32) -> Pair {
        let n = self.sorted.len();
        let at = |q: f32| self.sorted.get(((n.max(1) - 1) as f32 * q) as usize).copied().unwrap_or(0.) + ev;
        let (low, high) = (at(PAIR_LOW), at(PAIR_HIGH));
        let extremes: [f32; 4] = std::array::from_fn(|k| units[k].eval(if k == 0 || k == 2 { low } else { high }));
        let reach: [f32; 4] = std::array::from_fn(|k| {
            if extremes[k] == 0. || n == 0 {
                return 0.;
            }
            let sum: f64 = self.sorted.iter().map(|b| f64::from((units[k].eval(b + ev) / extremes[k]).clamp(0., 1.))).sum();
            (sum / n as f64) as f32
        });
        let fractions = std::array::from_fn(|c| {
            let [s, h] = KINDS[c];
            let size = |k: usize| extremes[k].abs();
            let x = [reach[s], reach[h], size(s), size(h), 1. / size(s).max(PAIR_EXTREME_MIN), 1. / size(h).max(PAIR_EXTREME_MIN)];
            std::array::from_fn(|slider| {
                let w = &rule_pair::BETA[c][slider];
                let z = x.iter().zip(&rule_pair::MU[c]).zip(&rule_pair::SD[c]);
                let f = z.zip(&w[1..]).fold(w[0], |sum, (((x, mu), sd), w)| sum + w * (x - mu) / sd).clamp(0., 1.);
                f * pair_scale(c)[slider]
            })
        });
        Pair { extremes, fractions }
    }
}

// ---------------------------------------------------------------- one render

/// Which of the operators a render needs (`(map, haze)`), from its settings; `adobe`: the photo
/// renders on an Adobe base (whose default black reads the map's dark key, [`crate::black_cr`]).
pub fn needs(s: &lightcraft_develop::DevelopSettings, adobe: bool) -> (bool, bool) {
    let effects = s.section_enabled("effects");
    let masks = |f: fn(&lightcraft_develop::LocalAdjustments) -> f64| s.masks.iter().any(|m| f(&m.adjust) != 0.0);
    let clarity = CLARITY_MEASURED && ((effects && s.effects.clarity != 0.0) || masks(|a| a.clarity));
    let hs = hs_measured() && (s.light.highlights != 0.0 || s.light.shadows != 0.0 || masks(|a| a.highlights) || masks(|a| a.shadows));
    // the photo's dark key for Camera Raw's default black (`black_cr`): every render with the
    // photo's key, Shadows' shift of it with the rule
    let black = adobe
        && match crate::black_cr::mode() {
            crate::black_cr::Mode::Photo => true,
            crate::black_cr::Mode::Shift => hs && s.light.shadows != 0.0,
            crate::black_cr::Mode::Off => false,
        };
    let haze = HAZE_MEASURED && ((effects && s.effects.dehaze != 0.0) || s.masks.iter().any(|m| m.adjust.dehaze != 0.0));
    (clarity || hs || black, haze)
}

/// Bilinear neighbours of a render pixel on the map.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub idx: [usize; 4],
    pub w: [f32; 4],
}

/// What one render's per-pixel stage reads of the photo's scene tone map.
#[derive(Clone)]
pub struct SceneTone {
    pub map: Arc<MapBase>,
    /// Render pixel centre `(X, Y)` (in the whole render, a region render's offset included) →
    /// map cell coordinates: `x = xf[0]·X + xf[1]·Y + xf[2]`, `y = xf[3]·X + xf[4]·Y + xf[5]`.
    /// Map size over render size for a map of the render's own frame; for the whole photo's map
    /// ([`MapBase::whole`]) where the render's crop, angle and flips put the pixel ([`Self::framed`]).
    pub xf: [f32; 6],
    pub ev: f32,
    /// Clarity (global or masks) reads the map's detail.
    pub clarity: bool,
    /// Highlights / Shadows by the rule, with the global sliders (−1..1).
    pub hs: Option<(HsCurves, f32, f32)>,
    /// How far (EV) the levels the rule reads sit below the render's: its Exposure when the rule
    /// sees the photo before Exposure ([`HS_BEFORE_EXPOSURE`]), else 0.
    pub hs_shift: f32,
    /// Positive Dehaze: the photo's haze and its airlight at this Exposure.
    pub haze: Option<(Arc<Haze>, [f32; 3])>,
}

impl SceneTone {
    /// For a `w × h` render at Exposure `ev`.
    pub fn new(map: Arc<MapBase>, w: usize, h: usize, ev: f32) -> SceneTone {
        let xf = [map.w as f32 / w.max(1) as f32, 0., -0.5, 0., map.h as f32 / h.max(1) as f32, -0.5];
        SceneTone { map, xf, ev, clarity: false, hs: None, hs_shift: 0., haze: None }
    }

    /// Find the render's pixels on a map of the whole uncropped photo: `to_norm` takes a pixel
    /// centre of the render's buffer to normalized image coordinates ([`crate::geometry::Frame::out_to_norm`]),
    /// `offset` is where the buffer sits in the whole render (pixels are read there).
    pub fn framed(mut self, to_norm: &lightcraft_geom::Affine, offset: (usize, usize)) -> SceneTone {
        let [a, b, c, d, e, f] = to_norm.0;
        let (mw, mh) = (self.map.w as f64, self.map.h as f64);
        let (ox, oy) = (offset.0 as f64, offset.1 as f64);
        // normalized x = a·(X − ox) + c·(Y − oy) + e; the cell is mw · that − ½
        let xf = [mw * a, mw * c, mw * (e - a * ox - c * oy) - 0.5, mh * b, mh * d, mh * (f - b * ox - d * oy) - 0.5];
        self.xf = xf.map(|v| v as f32);
        self
    }

    pub fn sample(&self, x: usize, y: usize) -> Sample {
        let (mw, mh) = (self.map.w, self.map.h);
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let xf = &self.xf;
        let fx = (xf[0] * px + xf[1] * py + xf[2]).clamp(0., (mw - 1) as f32);
        let fy = (xf[3] * px + xf[4] * py + xf[5]).clamp(0., (mh - 1) as f32);
        let (ix, iy) = (fx as usize, fy as usize);
        let (jx, jy) = ((ix + 1).min(mw - 1), (iy + 1).min(mh - 1));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        Sample { idx: [iy * mw + ix, iy * mw + jx, jy * mw + ix, jy * mw + jx], w: [(1. - tx) * (1. - ty), tx * (1. - ty), (1. - tx) * ty, tx * ty] }
    }

    fn bil(s: &Sample, v: &[f32]) -> f32 {
        (0..4).map(|k| v.get(s.idx[k]).copied().unwrap_or(0.) * s.w[k]).sum()
    }

    /// The base level of a pixel of level `l` (log2 luminance after Exposure and a mask's).
    pub fn base(&self, s: &Sample, l: f32) -> f32 {
        let a = Self::bil(s, &self.map.a);
        a * l + Self::bil(s, &self.map.b) + (1. - a) * self.ev
    }

    /// Clarity's log2 gain at `amount` (−1..1) for a pixel of level `l` (after Exposure, before
    /// a mask's).
    pub fn clarity_gain(&self, s: &Sample, amount: f32, l: f32) -> f32 {
        if amount == 0. || !self.clarity {
            return 0.;
        }
        let d = self.map.detail();
        let (i, [a, b]) = bracket(amount);
        let (mut num, mut den) = (0f32, 0f32);
        let at = |v: &[f32], j: usize| v.get(j).copied().unwrap_or(0.);
        for k in 0..4 {
            let j = s.idx[k];
            let guide = at(&d.guide, j);
            // the cell's gain at the pixel's own level (fine detail gains as Camera Raw's does), or
            // at the cell's (RAWmakase)
            let x = if CLARITY_FINE { l - self.ev } else { guide };
            let fit = |f: usize| at(&d.slopes[f], j) * x + at(&d.offsets[f], j);
            let g = a * fit(i) + b * fit(i + 1);
            let q = (guide + self.ev - l) / UPSAMPLE_RANGE;
            let wt = s.w[k] / (1. + q * q);
            num += g * wt;
            den += wt;
        }
        if den > 1e-12 { num / den } else { 0. }
    }

    /// The 45 terms of the fine Clarity gain at level `l` (detail: scale-major, knot-minor; then the
    /// level row's 9): its gain is their dot product with the weights. For refitting the weights
    /// (`LIGHTCRAFT_CLARITY_DUMP`, `oracle/clafit.py`).
    pub fn clarity_terms(&self, s: &Sample, l: f32) -> [f32; 45] {
        let d = self.map.detail();
        let mut out = [0f32; 45];
        let mut den = 0f32;
        let step = (KNOT_HI - KNOT_LO) / (KNOTS - 1) as f32;
        for k in 0..4 {
            let j = s.idx[k];
            let guide = d.guide.get(j).copied().unwrap_or(0.);
            let q = (guide + self.ev - l) / UPSAMPLE_RANGE;
            let wt = s.w[k] / (1. + q * q);
            den += wt;
            let u = ((d.rel.get(j).copied().unwrap_or(0.) - KNOT_LO) / step).clamp(0., (KNOTS - 1) as f32);
            let kk = (u as usize).min(KNOTS - 2);
            let t = u - kk as f32;
            for sc in 0..4 {
                let det = (l - self.ev) - d.blurs[sc].get(j).copied().unwrap_or(0.);
                out[sc * KNOTS + kk] += wt * (1. - t) * det;
                out[sc * KNOTS + kk + 1] += wt * t * det;
            }
            out[36 + kk] += wt * (1. - t);
            out[36 + kk + 1] += wt * t;
        }
        if den > 1e-12 {
            out.iter_mut().for_each(|v| *v /= den);
        }
        out
    }

    /// Positive Dehaze `dz` (> 0) on linear Rec.2020 `c`, whose local tone gain is `tone_gain`.
    pub fn dehaze(&self, s: &Sample, c: [f32; 3], dz: f32, tone_gain: f32) -> [f32; 3] {
        let Some((haze, air)) = &self.haze else { return c };
        let t = (1. - omega(dz) * Self::bil(s, &haze.density)).max(MIN_TRANSMISSION);
        let (to, from) = crate::colorops::pp_matrices();
        let air = air.map(|v| v * tone_gain);
        mul3(from, dehaze_pp(mul3(to, c), air, t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(i: usize) -> Sample {
        Sample { idx: [i; 4], w: [1., 0., 0., 0.] }
    }

    fn tone(lum: &[f32], w: usize, h: usize) -> SceneTone {
        let mut t = SceneTone::new(Arc::new(MapBase::new(lum.to_vec(), w, h).unwrap()), w, h, 0.);
        t.clarity = true;
        t
    }

    #[test]
    fn flat_photos_and_zero_clarity_are_unchanged() {
        let (w, h) = (40, 30);
        let t = tone(&vec![0.25; w * h], w, h);
        // a flat photo has no detail: at most the level row's uniform change
        let g0 = t.clarity_gain(&point(0), 1., -2.);
        assert!(g0.abs() < 0.15 && (0..w * h).all(|i| (t.clarity_gain(&point(i), 1., -2.) - g0).abs() < 1e-5), "{g0}");
        let lum: Vec<f32> = (0..w * h).map(|i| 2f32.powf(((i * 7919) % 13) as f32 * 0.3 - 4.)).collect();
        let t = tone(&lum, w, h);
        assert!((0..w * h).all(|i| t.clarity_gain(&point(i), 0., -2.) == 0.));
    }

    #[test]
    fn clarity_lifts_texture_and_the_blur_keeps_edges() {
        let (w, h) = (64, 48);
        let logs: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let level = if (16..48).contains(&x) && (12..36).contains(&y) { -1. } else { -5. };
                level + if (x + y) % 2 == 0 { 0.2 } else { -0.2 }
            })
            .collect();
        let t = tone(&logs.iter().map(|l| l.exp2()).collect::<Vec<_>>(), w, h);
        let i = 24 * w + 32;
        let out = |i: usize| logs[i] + t.clarity_gain(&point(i), 1., logs[i]);
        assert!((out(i) - out(i + 1)).abs() > (logs[i] - logs[i + 1]).abs());
        let [b, ..] = bilateral(&logs, w, h, [4.; 4], RANGE);
        assert!((b[24 * w + 17] + 1.).abs() < 1. && (b[24 * w + 14] + 5.).abs() < 1.);
    }

    #[test]
    fn clarity_follows_exposure() {
        // the gain is the same with Exposure added to the map's guide and the pixel's level
        let (w, h) = (48, 32);
        let lum: Vec<f32> = (0..w * h).map(|i| 2f32.powf(-4. + 0.8 * ((i % w) as f32 * 0.7).sin())).collect();
        let t0 = tone(&lum, w, h);
        let mut t1 = t0.clone();
        t1.ev = 1.5;
        let i = 10 * w + 17;
        let l = level(lum[i]);
        assert!((t0.clarity_gain(&point(i), 0.6, l) - t1.clarity_gain(&point(i), 0.6, l + 1.5)).abs() < 1e-5);
        assert!((t0.base(&point(i), l) + 1.5 - t1.base(&point(i), l + 1.5)).abs() < 1e-4);
    }

    #[test]
    fn omega_interpolates_between_the_fitted_amounts() {
        assert_eq!(omega(0.), 0.);
        assert!((omega(0.1) - 0.15).abs() < 1e-6);
        assert!((omega(0.3) - 0.375).abs() < 1e-6);
        assert!((omega(1.) - 0.9).abs() < 1e-6);
        assert!((omega(1.6) - 0.9).abs() < 1e-6);
        assert_eq!(omega(f32::NAN), 0.);
    }

    fn hazy(w: usize, h: usize) -> Vec<[f32; 3]> {
        let air = [0.6, 0.65, 0.75];
        (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) as f32 / w as f32, (i / w) as f32 / h as f32);
                let surface = [0.02 + 0.3 * x, 0.015 + 0.25 * x, 0.01 + 0.2 * x];
                let t = 0.35 + 0.5 * y;
                std::array::from_fn(|c| surface[c] * t + air[c] * (1. - t))
            })
            .collect()
    }

    #[test]
    fn dehaze_darkens_hazy_shadows_and_adds_contrast() {
        let (w, h) = (120, 80);
        let scene = hazy(w, h);
        let haze = Haze::of_pp(&scene, w, h);
        let density = |y: usize| haze.density[y * w + w / 2];
        assert!(density(5) > density(h - 5) + 0.1);
        let at = |i: usize, s: f32| dehaze_pp(scene[i], haze.air, (1. - omega(s) * haze.density[i]).max(MIN_TRANSMISSION));
        let (dark, light) = ((h / 3) * w + 5, (h / 3) * w + w - 5);
        for s in [0.2, 0.4, 1.] {
            let (d0, l0) = (luminance_pp(scene[dark]), luminance_pp(scene[light]));
            let (d, l) = (luminance_pp(at(dark, s)), luminance_pp(at(light, s)));
            assert!(d < d0 && l / d > l0 / d0, "{s}");
        }
        // hostile input: an empty or mis-sized scene is no haze, never a panic
        assert_eq!(Haze::of_pp(&[], 3, 3).density, vec![0.]);
    }

    #[test]
    fn rule_curves_brighten_shadows_and_pull_highlights() {
        // a textured scene, dark left, bright right
        let (w, h) = (96, 64);
        let lum: Vec<f32> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let l = if x < w / 2 { -7. } else { -1. } + 0.3 * (((x * 31 + y * 17) % 7) as f32 / 7. - 0.5);
                l.exp2()
            })
            .collect();
        let map = MapBase::new(lum, w, h).unwrap();
        let hs = map.hs_curves(0.);
        let (dark, bright) = (-7., -1.);
        // Shadows +100 lifts the dark half more than the bright one, Highlights −100 pulls the bright half
        assert!(hs.gain(dark, 1., 0.) > hs.gain(bright, 1., 0.) && hs.gain(dark, 1., 0.) > 0.2, "{hs:?}");
        assert!(hs.gain(bright, 0., -1.) < hs.gain(dark, 0., -1.) && hs.gain(bright, 0., -1.) < -0.1);
        // linear in the slider alone, zero at zero, finite for hostile sliders
        assert!((hs.gain(dark, 0.5, 0.) - 0.5 * hs.gain(dark, 1., 0.)).abs() < 1e-5);
        assert_eq!(hs.gain(dark, 0., 0.), 0.);
        assert!(hs.gain(dark, f32::NAN, f32::INFINITY).is_finite());
        // output log luminance never decreases along a unit curve
        for u in &hs.units {
            let step = (u.hi - u.lo) / LEVELS as f32;
            assert!(u.table.windows(2).all(|p| p[1] + step >= p[0] - 1e-4), "{u:?}");
        }
    }

    #[test]
    fn a_lamp_far_above_white_does_not_stretch_the_rule() {
        // the same dark photo with one small light at +3 EV and at +8 EV: the rule reads highlights only up to
        // RULE_CEILING, so the shadows' curve is the one of the first (a sun in frame lifted the shadows ~3 EV more
        // than Camera Raw does: DSC00603 ΔE 43.6 → 4.5)
        let (w, h) = (96, 64);
        let photo = |lamp: f32| -> Vec<f32> {
            (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let l = if x < 16 && y < 16 { lamp } else { -5. + 0.3 * (((x * 31 + y * 17) % 7) as f32 / 7. - 0.5) };
                    l.exp2()
                })
                .collect()
        };
        let (a, b) = (MapBase::new(photo(RULE_CEILING + 1.), w, h).unwrap(), MapBase::new(photo(RULE_CEILING + 6.), w, h).unwrap());
        let (ca, cb) = (a.hs_curves(0.), b.hs_curves(0.));
        for k in [0, 1] {
            let (ua, ub) = (&ca.units[k], &cb.units[k]);
            // the range stops at the ceiling (+1 for the table's edge), and the shadows' lift is the same to within the
            // edge pixels' difference (the guided base around the lamp's edge was 5 EV apart before the clamp)
            assert!((ua.hi - ub.hi).abs() < 0.05 && (ub.hi - (RULE_CEILING + 1.)).abs() < 0.05, "{ua:?} {ub:?}");
            assert!((ua.eval(-5.) - ub.eval(-5.)).abs() < 0.3, "{ua:?} {ub:?}");
        }
    }

    #[test]
    fn the_members_spread_is_finite_and_not_negative() {
        let z = [0f32; rule_data::FEATURES];
        let (mean, spread) = ensemble(&z, &rule_data::W1, &rule_data::B1, &rule_data::W2, &rule_data::B2, &rule_data::W3, &rule_data::B3);
        assert!(mean[0].is_finite() && spread.is_finite() && spread >= 0.);
        let hostile = [f32::MAX; rule_data::FEATURES];
        let (_, spread) = ensemble(&hostile, &rule_data::W1, &rule_data::B1, &rule_data::W2, &rule_data::B2, &rule_data::W3, &rule_data::B3);
        assert!(spread >= 0. || spread.is_nan());
    }

    #[test]
    fn hostile_maps_are_refused() {
        assert!(MapBase::new(vec![], 0, 0).is_none());
        assert!(MapBase::new(vec![0.1; 5], 2, 2).is_none());
        let m = MapBase::new(vec![f32::NAN, f32::INFINITY, -1., 0.], 2, 2).unwrap();
        let hs = m.hs_curves(f32::NAN);
        assert!(hs.units.iter().all(|u| u.table.iter().all(|v| v.is_finite())));
        assert_eq!(map_size(6000, 4000), (512, 341));
        assert_eq!(map_size(300, 200), (300, 200));
    }
}
