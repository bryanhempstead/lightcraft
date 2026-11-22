//! Match camera & lens colour (beyond Lightroom): make a photo from one camera / lens render
//! like a reference photo from another with the same look.
//!
//! The same settings look different on a Leica M262 and a Ricoh GR III because each camera's
//! colour path differs after its matrix: the Adobe Standard DCP's hue/sat map and look table, its
//! tone curve and how Temp / Tint move its neutral. So, content-independent first:
//!
//! 1. **Chart.** A standard target in scene colours (the 24 ColorChecker patches from their
//!    published CIE L\*a\*b\* D50 values, skin tones, a neutral ramp, saturated primaries) is put
//!    through each photo's own raw decode with that camera's matrix inverted
//!    ([`crate::files::chart_source`]), both lit by the target photo's light, and rendered through
//!    the real pipeline: the reference with its settings (white balance and exposure taken from
//!    the target, so only the cameras differ), the target with its own. A damped Gauss-Newton
//!    search (finite differences, slider units, a prior that prefers small moves, clamped to the
//!    slider ranges) then moves the target's Temp / Tint, the 8 × 3 HSL sliders, Calibration and
//!    Saturation / Vibrance until the two charts agree in CIELAB. The result is ordinary, editable
//!    slider values.
//! 2. **Lens / device casts** learned from the user's own edits ([`crate::lens_casts`]): the
//!    difference between how he finishes each lens (or phone) at the same event.
//! 3. **Scene pass** (same scene: shot within [`SCENE_WINDOW_S`] of each other, or forced): the
//!    small renders' near-neutral pixels even out the remaining cast and each hue band's mean
//!    chroma / hue nudges HSL, with strong damping so content differences don't skew it. Rendered
//!    files (phone JPEG / HEIC: no sensor path) get only 2 and 3.

use lightcraft_catalog::{Photo, PhotoId, Source};
use lightcraft_color::{D50, D65, REC2020, Xy, bradford};
use lightcraft_develop::{DevelopSettings, WbMode, controls};
use lightcraft_pipeline::{DeepSamples, OutputDepth, OutputSpace, RenderRequest, SourceInfo};
use lightcraft_raster::Rgb32f;
use serde_json::{Value, json};

use crate::Session;
use crate::files::ChartShot;

/// Photos shot this close together (seconds) are taken to show the same scene and light.
pub const SCENE_WINDOW_S: i64 = 30 * 60;
/// Largest raw file read for the chart (bytes).
const MAX_RAW_BYTES: u64 = 1 << 30;

/// The chart's patches as CIE L\*a\*b\* (D50): the 24 ColorChecker Classic patches (X-Rite's
/// published post-2014 values), five skin tones, a neutral ramp and six saturated primaries.
pub const PATCHES: [(&str, [f64; 3]); 40] = [
    ("dark skin", [37.54, 14.37, 14.92]),
    ("light skin", [64.66, 19.27, 17.5]),
    ("blue sky", [49.32, -3.82, -22.54]),
    ("foliage", [43.46, -12.74, 22.72]),
    ("blue flower", [54.94, 9.61, -24.79]),
    ("bluish green", [70.48, -32.26, -0.37]),
    ("orange", [62.73, 35.83, 56.5]),
    ("purplish blue", [39.43, 10.75, -45.17]),
    ("moderate red", [50.57, 48.64, 16.67]),
    ("purple", [30.1, 22.54, -20.87]),
    ("yellow green", [71.77, -24.13, 58.19]),
    ("orange yellow", [71.51, 18.24, 67.37]),
    ("blue", [28.37, 15.42, -49.8]),
    ("green", [54.38, -39.72, 32.27]),
    ("red", [42.43, 51.05, 28.62]),
    ("yellow", [81.8, 2.67, 80.41]),
    ("magenta", [50.63, 51.28, -14.12]),
    ("cyan", [49.57, -29.71, -28.32]),
    ("white 9.5", [95.19, -1.03, 2.93]),
    ("neutral 8", [81.29, -0.57, 0.44]),
    ("neutral 6.5", [66.89, -0.75, -0.06]),
    ("neutral 5", [50.76, -0.13, 0.14]),
    ("neutral 3.5", [35.63, -0.46, -0.48]),
    ("black 2", [20.64, 0.07, -0.46]),
    ("skin fair", [72.0, 11.0, 15.0]),
    ("skin light", [65.0, 15.0, 19.0]),
    ("skin medium", [56.0, 17.0, 23.0]),
    ("skin tan", [47.0, 16.0, 24.0]),
    ("skin deep", [33.0, 12.0, 15.0]),
    ("neutral 10", [10.0, 0.0, 0.0]),
    ("neutral 30", [30.0, 0.0, 0.0]),
    ("neutral 60", [60.0, 0.0, 0.0]),
    ("neutral 75", [75.0, 0.0, 0.0]),
    ("neutral 88", [88.0, 0.0, 0.0]),
    ("primary red", [45.0, 62.0, 42.0]),
    ("primary green", [55.0, -55.0, 45.0]),
    ("primary blue", [32.0, 30.0, -62.0]),
    ("primary cyan", [62.0, -38.0, -32.0]),
    ("primary magenta", [48.0, 62.0, -38.0]),
    ("primary yellow", [86.0, -6.0, 78.0]),
];

/// Pixels per patch side, and patches per row.
const BLOCK: usize = 8;
const COLS: usize = 8;

fn f_inv(t: f64) -> f64 {
    if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0 / 29.0) * (6.0 / 29.0) * (t - 4.0 / 29.0) }
}

fn f_fwd(t: f64) -> f64 {
    if t > (6.0f64 / 29.0).powi(3) { t.cbrt() } else { t / (3.0 * (6.0 / 29.0) * (6.0 / 29.0)) + 4.0 / 29.0 }
}

/// L\*a\*b\* relative to `white` → XYZ (Y of white = 1).
fn lab_to_xyz(lab: [f64; 3], white: Xy) -> [f64; 3] {
    let w = white.to_xyz();
    let fy = (lab[0] + 16.0) / 116.0;
    [w[0] * f_inv(fy + lab[1] / 500.0), w[1] * f_inv(fy), w[2] * f_inv(fy - lab[2] / 200.0)]
}

fn xyz_to_lab(xyz: [f64; 3], white: Xy) -> [f64; 3] {
    let w = white.to_xyz();
    let (fx, fy, fz) = (f_fwd(xyz[0] / w[0]), f_fwd(xyz[1] / w[1]), f_fwd(xyz[2] / w[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// A D50 L\*a\*b\* colour as linear Rec.2020 (D65, Bradford), the working space.
pub fn lab_d50_to_working(lab: [f64; 3]) -> [f64; 3] {
    REC2020.from_xyz().mul(&bradford(D50, D65)).apply(lab_to_xyz(lab, D50))
}

/// Linear Rec.2020 → L\*a\*b\* (D65 white).
pub fn working_to_lab(rgb: [f64; 3]) -> [f64; 3] {
    xyz_to_lab(REC2020.to_xyz().apply(rgb.map(|v| v.max(0.0))), D65)
}

/// CIEDE2000 colour difference.
pub fn de2000(a: [f64; 3], b: [f64; 3]) -> f64 {
    use std::f64::consts::PI;
    let (l1, a1, b1, l2, a2, b2) = (a[0], a[1], a[2], b[0], b[1], b[2]);
    let c1 = a1.hypot(b1);
    let c2 = a2.hypot(b2);
    let cm = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (cm.powi(7) / (cm.powi(7) + 25f64.powi(7))).sqrt());
    let (a1p, a2p) = ((1.0 + g) * a1, (1.0 + g) * a2);
    let (c1p, c2p) = (a1p.hypot(b1), a2p.hypot(b2));
    let hue = |b: f64, a: f64| {
        let h = b.atan2(a);
        if h < 0.0 { h + 2.0 * PI } else { h }
    };
    let (h1p, h2p) = (hue(b1, a1p), hue(b2, a2p));
    let dl = l2 - l1;
    let dc = c2p - c1p;
    let mut dh = h2p - h1p;
    if c1p * c2p == 0.0 {
        dh = 0.0;
    } else if dh > PI {
        dh -= 2.0 * PI;
    } else if dh < -PI {
        dh += 2.0 * PI;
    }
    let dhh = 2.0 * (c1p * c2p).sqrt() * (dh / 2.0).sin();
    let lm = (l1 + l2) / 2.0;
    let cmp = (c1p + c2p) / 2.0;
    let hm = if c1p * c2p == 0.0 {
        h1p + h2p
    } else if (h1p - h2p).abs() <= PI {
        (h1p + h2p) / 2.0
    } else if h1p + h2p < 2.0 * PI {
        (h1p + h2p + 2.0 * PI) / 2.0
    } else {
        (h1p + h2p - 2.0 * PI) / 2.0
    };
    let t = 1.0 - 0.17 * (hm - PI / 6.0).cos() + 0.24 * (2.0 * hm).cos() + 0.32 * (3.0 * hm + PI / 30.0).cos()
        - 0.20 * (4.0 * hm - 63.0 * PI / 180.0).cos();
    let dtheta = 30.0 * (-((hm * 180.0 / PI - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (cmp.powi(7) / (cmp.powi(7) + 25f64.powi(7))).sqrt();
    let sl = 1.0 + 0.015 * (lm - 50.0).powi(2) / (20.0 + (lm - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * cmp;
    let sh = 1.0 + 0.015 * cmp * t;
    let rt = -(2.0 * dtheta * PI / 180.0).sin() * rc;
    let v = (dl / sl).powi(2) + (dc / sc).powi(2) + (dhh / sh).powi(2) + rt * (dc / sc) * (dhh / sh);
    v.max(0.0).sqrt()
}

/// The chart image: each patch a `BLOCK`² square, `COLS` per row, scaled by `scale` (linear).
pub fn chart_shot(scale: f64, white: Option<Xy>) -> ChartShot {
    let rows = PATCHES.len().div_ceil(COLS);
    let (w, h) = (COLS * BLOCK, rows * BLOCK);
    let grey = lab_d50_to_working([50.0, 0.0, 0.0]);
    let colours: Vec<[f64; 3]> = PATCHES.iter().map(|(_, lab)| lab_d50_to_working(*lab)).collect();
    let mut pixels = vec![[0f32; 3]; w * h];
    for (i, px) in pixels.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        let k = (y / BLOCK) * COLS + x / BLOCK;
        let c = colours.get(k).copied().unwrap_or(grey);
        *px = c.map(|v| (v * scale) as f32);
    }
    ChartShot { pixels, width: w, height: h, white, light: None }
}

/// Mean L\*a\*b\* of each patch's centre in a linear Rec.2020 render of the chart (`None`:
/// the render isn't the chart's size).
fn measure(deep: &[f32], w: usize, h: usize) -> Option<Vec<[f64; 3]>> {
    let rows = PATCHES.len().div_ceil(COLS);
    if w != COLS * BLOCK || h != rows * BLOCK || deep.len() < w * h * 3 {
        return None;
    }
    let lo = BLOCK / 4;
    let hi = BLOCK - lo;
    let mut out = Vec::with_capacity(PATCHES.len());
    for k in 0..PATCHES.len() {
        let (x0, y0) = ((k % COLS) * BLOCK, (k / COLS) * BLOCK);
        let mut sum = [0f64; 3];
        let mut n = 0.0;
        for y in y0 + lo..y0 + hi {
            for x in x0 + lo..x0 + hi {
                let o = (y * w + x) * 3;
                let px = deep.get(o..o + 3)?;
                for c in 0..3 {
                    sum[c] += f64::from(px[c]);
                }
                n += 1.0;
            }
        }
        out.push(working_to_lab(sum.map(|v| v / n)));
    }
    Some(out)
}

/// `d` without what depends on the photo's content or position (crop, geometry, lens
/// corrections, masks, spots, vignettes) or adds noise (grain, sharpening, noise reduction).
pub fn chart_settings(d: &DevelopSettings) -> DevelopSettings {
    let mut c = d.clone();
    c.grain.amount = 0.0;
    c.detail.sharpen_amount = 0.0;
    c.detail.nr_luminance = 0.0;
    c.detail.nr_color = 0.0;
    c.vignette.amount = 0.0;
    c.optics = Default::default();
    c.geometry = Default::default();
    c.crop = Default::default();
    c.orientation = Default::default();
    c.masks.clear();
    c.spots.clear();
    c.red_eye.clear();
    c.lens_blur.enabled = false;
    c.enhance = Default::default();
    c
}

/// One camera's chart: its decoded source and source info.
#[derive(Clone)]
pub struct CameraChart {
    pub src: Rgb32f,
    pub info: SourceInfo,
}

impl CameraChart {
    /// Render the chart with `d` → each patch's L\*a\*b\*.
    pub fn render(&self, d: &DevelopSettings) -> Option<Vec<[f64; 3]>> {
        let req = RenderRequest {
            apply_crop: false,
            depth: OutputDepth::F32Linear,
            space: OutputSpace::Rec2020,
            ..RenderRequest::fit(self.src.width, self.src.height)
        };
        let r = lightcraft_pipeline::render(&self.src, &self.info, &chart_settings(d), &req);
        let deep = r.deep?;
        match &deep.samples {
            DeepSamples::F32(v) => measure(v, deep.width, deep.height),
            DeepSamples::U16(_) => None,
        }
    }
}

/// Mean and largest CIEDE2000 between two charts.
pub fn chart_de(a: &[[f64; 3]], b: &[[f64; 3]]) -> (f64, f64) {
    let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| de2000(*x, *y)).collect();
    let n = d.len().max(1) as f64;
    (d.iter().sum::<f64>() / n, d.iter().copied().fold(0.0, f64::max))
}

/// A slider the solver may move: control id, finite-difference step and "small move" scale
/// (slider units; `wb.temp` is in mireds).
/// Largest move of any slider, in its `sigma`s.
const MAX_MOVE: f64 = 2.7;
/// Largest mean chart ΔE2000 a kept match may leave.
const MAX_AFTER: f64 = 3.5;

struct Var {
    id: &'static str,
    step: f64,
    sigma: f64,
}

const BANDS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];

fn vars() -> Vec<Var> {
    let mut v = vec![Var { id: "wb.temp", step: 4.0, sigma: 60.0 }, Var { id: "wb.tint", step: 3.0, sigma: 20.0 }];
    for id in HSL_IDS {
        v.push(Var { id, step: 6.0, sigma: 15.0 });
    }
    for id in [
        "calibration.redHue",
        "calibration.redSat",
        "calibration.greenHue",
        "calibration.greenSat",
        "calibration.blueHue",
        "calibration.blueSat",
        "calibration.shadowsTint",
    ] {
        v.push(Var { id, step: 6.0, sigma: 15.0 });
    }
    v.push(Var { id: "color.saturation", step: 5.0, sigma: 10.0 });
    v.push(Var { id: "color.vibrance", step: 5.0, sigma: 10.0 });
    v
}

const HSL_IDS: [&str; 24] = [
    "mixer.red.hue",
    "mixer.red.sat",
    "mixer.red.lum",
    "mixer.orange.hue",
    "mixer.orange.sat",
    "mixer.orange.lum",
    "mixer.yellow.hue",
    "mixer.yellow.sat",
    "mixer.yellow.lum",
    "mixer.green.hue",
    "mixer.green.sat",
    "mixer.green.lum",
    "mixer.aqua.hue",
    "mixer.aqua.sat",
    "mixer.aqua.lum",
    "mixer.blue.hue",
    "mixer.blue.sat",
    "mixer.blue.lum",
    "mixer.purple.hue",
    "mixer.purple.sat",
    "mixer.purple.lum",
    "mixer.magenta.hue",
    "mixer.magenta.sat",
    "mixer.magenta.lum",
];

/// The target's white balance as (mireds, tint), As Shot read as its as-shot values.
fn wb_of(d: &DevelopSettings, info: &SourceInfo) -> (f64, f64) {
    let (t, ti) = if d.wb.mode == WbMode::Custom { (d.wb.temp, d.wb.tint) } else { (info.as_shot_temp, info.as_shot_tint) };
    // finite always: hostile settings must never reach a clamp bound as NaN
    let t = if t.is_finite() { t.clamp(2000.0, 50000.0) } else { 5500.0 };
    (1e6 / t, if ti.is_finite() { ti.clamp(-150.0, 150.0) } else { 0.0 })
}

/// `base` with the solver's offsets `x` added (white balance in mireds / tint units).
fn apply(base: &DevelopSettings, info: &SourceInfo, vars: &[Var], x: &[f64]) -> DevelopSettings {
    let mut d = base.clone();
    let (m0, t0) = wb_of(base, info);
    for (v, dx) in vars.iter().zip(x) {
        match v.id {
            "wb.temp" | "wb.tint" => {}
            id => {
                let cur = controls::get(base, id).unwrap_or(0.0);
                controls::set(&mut d, id, cur + dx);
            }
        }
    }
    let dm = vars.iter().zip(x).find(|(v, _)| v.id == "wb.temp").map_or(0.0, |(_, x)| *x);
    let dt = vars.iter().zip(x).find(|(v, _)| v.id == "wb.tint").map_or(0.0, |(_, x)| *x);
    if dm.abs() > 1e-9 || dt.abs() > 1e-9 {
        d.wb.mode = WbMode::Custom;
        d.wb.temp = (1e6 / (m0 + dm).clamp(20.0, 500.0)).clamp(2000.0, 50000.0);
        d.wb.tint = (t0 + dt).clamp(-150.0, 150.0);
    }
    d
}

/// Solve `A x = b` (Gaussian elimination, partial pivoting); `None` if singular.
fn solve_n(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for c in 0..n {
        let p = (c..n).max_by(|x, y| a[*x][c].abs().total_cmp(&a[*y][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        for r in c + 1..n {
            let f = a[r][c] / a[c][c];
            for k in c..n {
                a[r][k] -= f * a[c][k];
            }
            b[r] -= f * b[c];
        }
    }
    let mut x = vec![0.0; n];
    for r in (0..n).rev() {
        let s: f64 = (r + 1..n).map(|k| a[r][k] * x[k]).sum();
        x[r] = (b[r] - s) / a[r][r];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// What the chart match found.
#[derive(Clone, Debug)]
pub struct ChartMatch {
    pub before: (f64, f64),
    pub after: (f64, f64),
    pub settings: DevelopSettings,
    /// (control id, old value, new value) of every slider moved.
    pub changed: Vec<(String, f64, f64)>,
}

/// Residuals (ΔL, Δa, Δb per patch) of `lab` against `reference`.
fn residuals(lab: &[[f64; 3]], reference: &[[f64; 3]]) -> Vec<f64> {
    lab.iter().zip(reference).flat_map(|(a, b)| [a[0] - b[0], a[1] - b[1], a[2] - b[2]]).collect()
}

/// Move `target`'s colour sliders until `render(settings)` matches `reference` (patch Lab).
/// `render` is the target camera's chart; `None` if it can't render.
pub fn solve(
    reference: &[[f64; 3]],
    target: &DevelopSettings,
    info: &SourceInfo,
    render: &(dyn Fn(&DevelopSettings) -> Option<Vec<[f64; 3]>> + Sync),
) -> Option<ChartMatch> {
    use rayon::prelude::*;
    let vars = vars();
    let n = vars.len();
    let start = render(target)?;
    let before = chart_de(&start, reference);
    let mut x = vec![0.0; n];
    let cost = |r: &[f64], x: &[f64]| r.iter().map(|v| v * v).sum::<f64>() + vars.iter().zip(x).map(|(v, x)| (x / v.sigma).powi(2)).sum::<f64>();
    let mut r = residuals(&start, reference);
    let mut c = cost(&r, &x);
    let mut mu = 1e-2;
    for _ in 0..8 {
        // Jacobian by forward differences (each column one render, in parallel)
        let cols: Vec<Option<Vec<f64>>> = (0..n)
            .into_par_iter()
            .map(|j| {
                let mut xp = x.clone();
                xp[j] += vars[j].step;
                let lab = render(&apply(target, info, &vars, &xp))?;
                let rp = residuals(&lab, reference);
                Some(rp.iter().zip(&r).map(|(a, b)| (a - b) / vars[j].step).collect())
            })
            .collect();
        let cols: Vec<Vec<f64>> = cols.into_iter().collect::<Option<_>>()?;
        let mut jtj = vec![vec![0.0; n]; n];
        let mut jtr = vec![0.0; n];
        for a in 0..n {
            for b in a..n {
                let s: f64 = cols[a].iter().zip(&cols[b]).map(|(p, q)| p * q).sum();
                jtj[a][b] = s;
                jtj[b][a] = s;
            }
            jtr[a] = cols[a].iter().zip(&r).map(|(p, q)| p * q).sum::<f64>() + x[a] / vars[a].sigma.powi(2);
        }
        let mut improved = false;
        for _ in 0..4 {
            let mut a = jtj.clone();
            for (i, row) in a.iter_mut().enumerate() {
                row[i] += 1.0 / vars[i].sigma.powi(2) + mu * (row[i] + 1e-3);
            }
            let Some(dx) = solve_n(a, jtr.iter().map(|v| -v).collect()) else {
                mu *= 10.0;
                continue;
            };
            // no slider moves more than MAX_MOVE of its "small move" scale (a match that needs
            // more is the scenes differing, not the cameras)
            let xn: Vec<f64> = x.iter().zip(&dx).zip(&vars).map(|((a, b), v)| (a + b).clamp(-MAX_MOVE * v.sigma, MAX_MOVE * v.sigma)).collect();
            let dn = apply(target, info, &vars, &xn);
            // read back the clamped values so the step stays inside the slider ranges
            let xn: Vec<f64> = vars.iter().zip(&xn).map(|(v, xv)| clamped_offset(target, &dn, info, v, *xv)).collect();
            let lab = render(&apply(target, info, &vars, &xn))?;
            let rn = residuals(&lab, reference);
            let cn = cost(&rn, &xn);
            if cn < c {
                (x, r, c) = (xn, rn, cn);
                mu = (mu / 3.0).max(1e-4);
                improved = true;
                break;
            }
            mu *= 8.0;
        }
        if !improved {
            break;
        }
    }
    // ordinary slider values: whole numbers, Temp to 10 K
    let mut out = apply(target, info, &vars, &x);
    for v in &vars {
        if let Some(val) = controls::get(&out, v.id).filter(|_| !v.id.starts_with("wb.")) {
            controls::set(&mut out, v.id, val.round());
        }
    }
    if out.wb.mode == WbMode::Custom && out.wb != target.wb {
        out.wb.temp = (out.wb.temp / 10.0).round() * 10.0;
        out.wb.tint = out.wb.tint.round();
    }
    let lab = render(&out)?;
    let after = chart_de(&lab, reference);
    // kept only when it clearly helps and lands close: a partial fix of a big difference (an
    // extreme white balance, scenes far apart) tends to look worse than none
    if after.0 > before.0 - 0.05 || after.0 > MAX_AFTER || after.0 > 0.6 * before.0 {
        return Some(ChartMatch { before, after: before, settings: target.clone(), changed: Vec::new() });
    }
    let mut changed = Vec::new();
    for v in &vars {
        if v.id == "wb.temp" {
            if out.wb != target.wb {
                let (m, t) = wb_of(target, info);
                changed.push(("wb.temp".to_string(), 1e6 / m, out.wb.temp));
                changed.push(("wb.tint".to_string(), t, out.wb.tint));
            }
            continue;
        }
        if v.id == "wb.tint" {
            continue;
        }
        let (a, b) = (controls::get(target, v.id).unwrap_or(0.0), controls::get(&out, v.id).unwrap_or(0.0));
        if (a - b).abs() >= 0.5 {
            changed.push((v.id.to_string(), a, b));
        }
    }
    Some(ChartMatch { before, after, settings: out, changed })
}

/// The offset `v` actually got in `d` (after the control's own clamping).
fn clamped_offset(base: &DevelopSettings, d: &DevelopSettings, info: &SourceInfo, v: &Var, wanted: f64) -> f64 {
    match v.id {
        "wb.temp" | "wb.tint" => {
            let (m0, t0) = wb_of(base, info);
            let (m, t) = wb_of(d, info);
            if v.id == "wb.temp" {
                if (m - m0 - wanted).abs() < 1e-6 { wanted } else { m - m0 }
            } else if (t - t0 - wanted).abs() < 1e-6 {
                wanted
            } else {
                t - t0
            }
        }
        id => controls::get(d, id).unwrap_or(0.0) - controls::get(base, id).unwrap_or(0.0),
    }
}

// ------------------------------------------------------------------------------- the photos

/// The reference's settings for the chart comparison: white balance and exposure taken from the
/// target (they belong to each photo's light, not to its camera).
pub fn reference_settings(r: &DevelopSettings, t: &DevelopSettings) -> DevelopSettings {
    let mut d = r.clone();
    d.wb = t.wb;
    d.light.exposure = t.light.exposure;
    d
}

fn raw_bytes(p: &Photo) -> Result<Vec<u8>, String> {
    let Source::File { path } = &p.source else { return Err("not a file".into()) };
    let len = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?.len();
    if len > MAX_RAW_BYTES {
        return Err(format!("{path}: too large"));
    }
    std::fs::read(path).map_err(|e| format!("{path}: {e}"))
}

/// Capture time in seconds (for "same scene"): `YYYY-MM-DDTHH:MM:SS…` read as one clock.
pub fn capture_seconds(s: &str) -> Option<i64> {
    let d = s.get(..19)?;
    let num = |a: usize, b: usize| d.get(a..b)?.parse::<i64>().ok();
    let (y, mo, da, h, mi, se) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    // days from a fixed epoch (proleptic Gregorian; exactness only matters within a few days)
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let days = 365 * yy + yy / 4 - yy / 100 + yy / 400 + (153 * mm + 2) / 5 + da;
    Some(days * 86_400 + h * 3600 + mi * 60 + se)
}

/// Whether two photos show the same scene and light (shot within [`SCENE_WINDOW_S`]).
pub fn same_scene(a: &Photo, b: &Photo) -> bool {
    match (a.captured.as_deref().and_then(capture_seconds), b.captured.as_deref().and_then(capture_seconds)) {
        (Some(x), Some(y)) => (x - y).abs() <= SCENE_WINDOW_S,
        _ => false,
    }
}

/// The lens a photo was shot with, as the cast table names it: its Exif lens, else the lens
/// profile Lightroom picked (manual lenses report none).
pub fn lens_name(p: &Photo) -> String {
    let l = p.meta.lens.trim();
    if !l.is_empty() && !l.eq_ignore_ascii_case("not selected") {
        return l.to_string();
    }
    p.develop.optics.lens_profile_ref.as_ref().map(|r| r.name.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| l.to_string())
}

/// Match the colour of `target` (settings `nd`, already synced) to `reference` (settings `rd`):
/// chart (raw ↔ raw, different cameras), lens / device casts, then the scene pass. Changes `nd`
/// in place and reports what it did.
pub fn match_photo(
    s: &mut Session,
    reference: PhotoId,
    rd: &DevelopSettings,
    target: PhotoId,
    nd: &mut DevelopSettings,
    scene: Option<bool>,
) -> Value {
    let (Some(rp), Some(tp)) = (s.catalog.photo(reference).cloned(), s.catalog.photo(target).cloned()) else {
        return json!({"id": target.0, "skipped": "no such photo"});
    };
    let mut report = serde_json::Map::new();
    report.insert("id".into(), json!(target.0));
    let same_camera = rp.meta.camera.trim().eq_ignore_ascii_case(tp.meta.camera.trim());
    let same_lens = lens_name(&rp).eq_ignore_ascii_case(&lens_name(&tp));
    let scene = scene.unwrap_or_else(|| same_scene(&rp, &tp));
    report.insert("sameScene".into(), json!(scene));
    // 1. chart
    if !same_camera && rp.develops_raw() && tp.develops_raw() {
        match chart_match(&rp, rd, &tp, nd, scene) {
            Ok(m) => {
                report.insert("deltaE_before".into(), json!(round2(m.before.0)));
                report.insert("deltaE_after".into(), json!(round2(m.after.0)));
                report.insert("deltaE_max_before".into(), json!(round2(m.before.1)));
                report.insert("deltaE_max_after".into(), json!(round2(m.after.1)));
                report.insert("chart".into(), changes_json(&m.changed));
                report.insert("chartKept".into(), json!(!m.changed.is_empty()));
                *nd = m.settings;
            }
            Err(e) => {
                report.insert("chartSkipped".into(), json!(e));
            }
        }
    } else if !same_camera {
        report.insert("chartSkipped".into(), json!("a rendered file (JPEG / HEIC) has no sensor path"));
    }
    // 2. lens / device casts
    if (!same_lens || !same_camera)
        && let Some(table) = crate::lens_casts::load()
    {
        let info = s.source_info(target);
        if let Some(v) = crate::lens_casts::apply_difference(&table, &rp, &tp, nd, &info) {
            report.insert("lensCast".into(), v);
        }
    }
    // 3. scene pass
    if scene && let Some(v) = scene_pass(s, reference, rd, target, nd) {
        report.insert("scene".into(), v);
    }
    Value::Object(report)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn changes_json(c: &[(String, f64, f64)]) -> Value {
    Value::Array(c.iter().map(|(id, a, b)| json!({"control": id, "from": round2(*a), "to": round2(*b)})).collect())
}

/// The chart match between the reference photo `rp` (settings `rd`) and the target `tp` (`nd`).
/// Different scenes: both charts are lit by the target's light and both cameras read it right
/// (only the profiles differ). The same scene (`same_scene`): the chart is lit by the light the
/// reference read, and the target camera keeps its own as-shot reading of it, so a difference in
/// how the two cameras' auto white balance read one light is matched too.
pub fn chart_match(rp: &Photo, rd: &DevelopSettings, tp: &Photo, nd: &DevelopSettings, same_scene: bool) -> Result<ChartMatch, String> {
    let scale = 2f64.powf(-nd.light.exposure.clamp(-5.0, 5.0));
    let ((tsrc, tinfo), (rsrc, rinfo)) = if same_scene {
        let rb = raw_bytes(rp)?;
        let (rsrc, rinfo, light) = crate::files::chart_source(&rb, &chart_shot(scale, None))?;
        drop(rb);
        let tb = raw_bytes(tp)?;
        let (tsrc, tinfo, _) = crate::files::chart_source(&tb, &ChartShot { light: Some(light), ..chart_shot(scale, None) })?;
        ((tsrc, tinfo), (rsrc, rinfo))
    } else {
        let tb = raw_bytes(tp)?;
        let (tsrc, tinfo, white) = crate::files::chart_source(&tb, &chart_shot(scale, None))?;
        drop(tb);
        let rb = raw_bytes(rp)?;
        let (rsrc, rinfo, _) = crate::files::chart_source(&rb, &chart_shot(scale, Some(white)))?;
        ((tsrc, tinfo), (rsrc, rinfo))
    };
    let rchart = CameraChart { src: rsrc, info: rinfo };
    let tchart = CameraChart { src: tsrc, info: tinfo };
    let reference = rchart.render(&reference_settings(rd, nd)).ok_or("the reference chart didn't render")?;
    let info = tchart.info;
    solve(&reference, nd, &info, &|d| tchart.render(d)).ok_or_else(|| "the target chart didn't render".to_string())
}

// ------------------------------------------------------------------------------- scene pass

/// A small render of `id` with `d` as L\*a\*b\* pixels (sRGB 8-bit decoded).
fn small_lab(s: &mut Session, id: PhotoId, d: &DevelopSettings) -> Option<Vec<[f64; 3]>> {
    let mut job = s.render_job(id, 256, 256, false, true)?;
    job.key ^= d.hash64();
    job.settings = std::sync::Arc::new(chart_settings_keep_crop(d));
    (job.cache, job.stages, job.view_cache) = (None, None, None);
    let r = job.run();
    s.accept(&r);
    let img = r.rendered.ok()?.image;
    let lin = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let m = REC2020.from_xyz().mul(&lightcraft_color::SRGB.to_xyz());
    Some(img.data.iter().map(|p| working_to_lab(m.apply([lin(p[0]), lin(p[1]), lin(p[2])]))).collect())
}

/// Like [`chart_settings`] but keeping the crop (the scene pass compares what the photos show).
fn chart_settings_keep_crop(d: &DevelopSettings) -> DevelopSettings {
    let mut c = d.clone();
    c.grain.amount = 0.0;
    c.vignette.amount = 0.0;
    c
}

/// Scene statistics: the scene's cast (median a\*, b\* of the mid-tone pixels that aren't
/// strongly coloured: robust to a subject, and still there when the whole frame is tinted), per
/// hue band (8 × 45°) the share, mean chroma and mean hue of the coloured pixels, and the share of
/// every band plus the near-neutrals (for "do the two photos show the same thing").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneStats {
    pub neutral: Option<(f64, f64)>,
    pub bands: [Option<(f64, f64, f64)>; 8],
    pub shares: [f64; 9],
}

pub fn scene_stats(lab: &[[f64; 3]]) -> SceneStats {
    let n = lab.len().max(1) as f64;
    let (mut ca, mut cb) = (Vec::new(), Vec::new());
    let mut acc = [(0.0f64, 0.0f64, 0.0f64, 0.0f64); 8]; // count, chroma, sin, cos
    let mut shares = [0.0f64; 9];
    for p in lab {
        let c = p[1].hypot(p[2]);
        if !(15.0..=92.0).contains(&p[0]) || !c.is_finite() {
            continue;
        }
        if c < 40.0 {
            ca.push(p[1]);
            cb.push(p[2]);
        }
        if c >= 12.0 {
            let h = p[2].atan2(p[1]);
            let k = (((h.to_degrees() + 360.0 + 22.5) % 360.0) / 45.0) as usize % 8;
            if let Some(a) = acc.get_mut(k) {
                *a = (a.0 + 1.0, a.1 + c, a.2 + h.sin(), a.3 + h.cos());
            }
        } else {
            shares[8] += 1.0 / n;
        }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let neutral = (ca.len() as f64 / n >= 0.2).then(|| (median(&mut ca), median(&mut cb)));
    for (s, a) in shares.iter_mut().zip(&acc) {
        *s = a.0 / n;
    }
    let bands = acc.map(|(k, c, sn, cs)| (k / n >= 0.02).then(|| (k / n, c / k, sn.atan2(cs).to_degrees())));
    SceneStats { neutral, bands, shares }
}

/// How much two photos' colour content overlaps (0..1): the shared share of each hue band and of
/// the near-neutrals.
pub fn overlap(a: &SceneStats, b: &SceneStats) -> f64 {
    let (sa, sb) = (a.shares.iter().sum::<f64>(), b.shares.iter().sum::<f64>());
    if sa <= 0.0 || sb <= 0.0 {
        return 0.0;
    }
    a.shares.iter().zip(&b.shares).map(|(x, y)| (x / sa).min(y / sb)).sum()
}

/// Least content overlap ([`overlap`]) for the scene pass to trust the two photos' difference.
const MIN_OVERLAP: f64 = 0.85;

/// The scene pass: even out the target's near-neutral cast against the reference's (Temp /
/// Tint, measured response, damped) and nudge each well-populated hue band's saturation toward
/// the reference's chroma (≤ ±10, half the measured gap).
fn scene_pass(s: &mut Session, reference: PhotoId, rd: &DevelopSettings, target: PhotoId, nd: &mut DevelopSettings) -> Option<Value> {
    let rs = scene_stats(&small_lab(s, reference, rd)?);
    let info = s.source_info(target);
    let start = nd.clone();
    let mut cur = scene_stats(&small_lab(s, target, nd)?);
    let mut out = serde_json::Map::new();
    let ov = overlap(&rs, &cur);
    out.insert("overlap".into(), json!(round2(ov)));
    if ov < MIN_OVERLAP {
        // different subjects: their colour difference is content, not camera or light
        out.insert("skipped".into(), json!("the photos show different things"));
        return Some(Value::Object(out));
    }
    let (m_start, t_start) = wb_of(&start, &info);
    if let (Some(rn), Some(_)) = (rs.neutral, cur.neutral) {
        // two measured responses (mireds → b*, tint → a*), then a damped step, up to three times
        for _ in 0..3 {
            let Some(tn) = cur.neutral else { break };
            let (da, db) = (rn.0 - tn.0, rn.1 - tn.1);
            if da.hypot(db) < 0.4 {
                break;
            }
            let probe = |s: &mut Session, dm: f64, dt: f64| -> Option<(f64, f64)> {
                let v = [Var { id: "wb.temp", step: 1.0, sigma: 1.0 }, Var { id: "wb.tint", step: 1.0, sigma: 1.0 }];
                let d = apply(nd, &info, &v, &[dm, dt]);
                scene_stats(&small_lab(s, target, &d)?).neutral
            };
            let pm = probe(s, 10.0, 0.0)?;
            let pt = probe(s, 0.0, 5.0)?;
            // Jacobian of (a, b) by (mired, tint)
            let j = [[(pm.0 - tn.0) / 10.0, (pt.0 - tn.0) / 5.0], [(pm.1 - tn.1) / 10.0, (pt.1 - tn.1) / 5.0]];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if !det.is_finite() || det.abs() < 1e-6 {
                break;
            }
            let dm = (j[1][1] * da - j[0][1] * db) / det;
            let dt = (-j[1][0] * da + j[0][0] * db) / det;
            // damped and bounded: content differences must not swing it far
            let v = [Var { id: "wb.temp", step: 1.0, sigma: 1.0 }, Var { id: "wb.tint", step: 1.0, sigma: 1.0 }];
            let (m, t) = wb_of(nd, &info);
            // ≤ ±25 mireds / ±10 tint from where the sync left it, in all
            let dm = (0.7 * dm).clamp(-20.0, 20.0).clamp(m_start - 25.0 - m, m_start + 25.0 - m);
            let dt = (0.7 * dt).clamp(-8.0, 8.0).clamp(t_start - 10.0 - t, t_start + 10.0 - t);
            *nd = apply(nd, &info, &v, &[dm, dt]);
            nd.wb.temp = (nd.wb.temp / 10.0).round() * 10.0;
            nd.wb.tint = nd.wb.tint.round();
            cur = scene_stats(&small_lab(s, target, nd)?);
        }
        if nd.wb != start.wb {
            let (m0, t0) = wb_of(&start, &info);
            out.insert("wb".into(), json!({"temp": [round2(1e6 / m0), nd.wb.temp], "tint": [round2(t0), nd.wb.tint]}));
        }
    }
    let mut sat = Vec::new();
    for (k, band) in BANDS.iter().enumerate() {
        let (Some(Some(r)), Some(Some(t))) = (rs.bands.get(k), cur.bands.get(k)) else { continue };
        if r.1 <= 0.0 || t.1 <= 0.0 {
            continue;
        }
        // ~ +1 saturation per 1 % chroma, half of it, at most ±10
        let step = (50.0 * (r.1 / t.1).ln()).clamp(-10.0, 10.0).round();
        if step.abs() >= 2.0 {
            let id = format!("mixer.{band}.sat");
            let v = controls::get(nd, &id).unwrap_or(0.0);
            controls::set(nd, &id, v + step);
            sat.push(json!({"control": id, "from": v, "to": controls::get(nd, &id).unwrap_or(v)}));
        }
    }
    if !sat.is_empty() {
        out.insert("hsl".into(), Value::Array(sat));
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests;
