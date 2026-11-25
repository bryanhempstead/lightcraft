//! The per-pixel stage: everything after the spatial planes are ready, in one parallel pass.

use lightcraft_color::luminance_2020;
use lightcraft_color::spline::{Lut1, NaturalCurve};
use lightcraft_color::transfer::linear_to_srgb;
use lightcraft_develop::{DevelopSettings, LocalAdjustments, ToneCurve, VignetteStyle};
use lightcraft_geom::Point;
use lightcraft_raster::Rgba8;

use crate::colorops::ColorOps;
use crate::geometry::Frame;
use crate::local::log_lum;
use crate::output::{DeepImage, DeepSamples, OutputDepth, OutputSpace, OutputTrc};
use crate::tone::ToneMap;
use crate::{Prepared, SourceInfo, for_rows};

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Parametric curve: peak change of a region at ±100 (encoded values, before the `4x(1 − x)`
/// envelope) and the half-width of its window relative to the region. Matched to Lightroom's
/// renders (`docs/lr-match.md`; were 0.22 and 0.75).
const PARAMETRIC_AMOUNT: f32 = 0.11;
const PARAMETRIC_WIDTH: f32 = 1.05;

/// Camera Raw's measured parametric curve (R08, [`crate::parametric_adobe`]) instead of the
/// preview-fitted one. Off: alone it makes Bryan's acceptance set worse (3.85 → 4.04 ΔE); it goes
/// on with [`ADOBE_HS_MEASURED`] (`docs/lr-match.md` → *Round 7*).
const PARAMETRIC_MEASURED: bool = false;

/// Camera Raw's measured Highlights / Shadows (R02, [`crate::tone::lr::adobe_hs_lut`]) over an
/// Adobe base instead of the preview-fitted tables. Off: closer to Camera Raw on Bryan's raws
/// (oracle ΔE with his tone sliders 3.88 → 2.67) but worse on the acceptance set (4.01 → 4.33)
/// until the rest is refitted (`docs/lr-match.md` → *Round 7*).
const ADOBE_HS_MEASURED: bool = false;

/// The tone curves as Lightroom applies them (Camera Raw oracle, `docs/lr-match.md` → *Recipes, curves and the process version*):
/// the parametric curve, then the point curve (natural cubic spline, clamped), as one *master*
/// curve applied hue-preserving to the encoded style-space colour (largest and smallest channel
/// through the curve, the middle one interpolated: [`crate::adobe::rgb_tone`]), then the red /
/// green / blue point curves per channel. 1024 entries each.
#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    pub master: Lut1,
    pub chans: [Lut1; 3],
}

impl Curves {
    /// The curves on an encoded style-space colour.
    #[inline]
    pub fn apply(&self, e: [f32; 3]) -> [f32; 3] {
        let m = crate::adobe::rgb_tone(e, |v| self.master.eval(v));
        [self.chans[0].eval(m[0]), self.chans[1].eval(m[1]), self.chans[2].eval(m[2])]
    }
}

const CURVE_LUT_N: usize = 1024;

fn point_lut(p: &[Point]) -> Option<Lut1> {
    (!ToneCurve::point_curve_is_identity(p)).then(|| NaturalCurve::new(&p.iter().map(|q| (q.x, q.y)).collect::<Vec<_>>()).to_lut(CURVE_LUT_N))
}

/// The user's tone curves, under a look profile's own curves (`look`: master, red, green, blue
/// point curves, applied first — Bryan's fork, [`crate::adobe::Look`]). A look's per-channel curves
/// are moved after the user's master curve (none of the Adobe looks has any).
fn curve_luts(c: &ToneCurve, look: Option<&[Vec<Point>; 4]>) -> Option<Curves> {
    let look = look.map(|l| [point_lut(&l[0]), point_lut(&l[1]), point_lut(&l[2]), point_lut(&l[3])]);
    let user_master = parametric_lut(c).map(|par| match point_lut(&c.master) {
        Some(m) => m.compose(&par),
        None => par,
    });
    let user_master = user_master.or_else(|| point_lut(&c.master));
    let user_ch = [point_lut(&c.red), point_lut(&c.green), point_lut(&c.blue)];
    let look_master = look.as_ref().and_then(|l| l[0].clone());
    let master = match (user_master, look_master) {
        (Some(u), Some(l)) => Some(u.compose(&l)),
        (u, l) => u.or(l),
    };
    let chans: [Option<Lut1>; 3] = std::array::from_fn(|i| {
        let l = look.as_ref().and_then(|l| l[i + 1].clone());
        match (user_ch[i].clone(), l) {
            (Some(u), Some(l)) => Some(u.compose(&l)),
            (u, l) => u.or(l),
        }
    });
    if master.is_none() && chans.iter().all(Option::is_none) {
        return None;
    }
    Some(Curves {
        master: master.unwrap_or_else(|| Lut1::identity(CURVE_LUT_N)),
        chans: chans.map(|c| c.unwrap_or_else(|| Lut1::identity(CURVE_LUT_N))),
    })
}

/// The parametric curve (encoded domain), `None` when all four regions are 0. Fitted on previews
/// (`PARAMETRIC_AMOUNT`, `PARAMETRIC_WIDTH`), or Camera Raw's measured curve
/// ([`parametric_adobe_lut`]) with [`PARAMETRIC_MEASURED`].
fn parametric_lut(c: &ToneCurve) -> Option<Lut1> {
    if c.highlights == 0.0 && c.lights == 0.0 && c.darks == 0.0 && c.shadows == 0.0 {
        return None;
    }
    if PARAMETRIC_MEASURED {
        return Some(parametric_adobe_lut(c));
    }
    let (s1, s2, s3) = ((c.split_shadows / 100.0) as f32, (c.split_mid / 100.0) as f32, (c.split_highlights / 100.0) as f32);
    let regions = [(0.0, s1, c.shadows), (s1, s2, c.darks), (s2, s3, c.lights), (s3, 1.0, c.highlights)];
    let mut base = Lut1::from_fn(CURVE_LUT_N, |x| {
        let mut d = 0.0;
        for (a, b, amt) in regions {
            if amt == 0.0 {
                continue;
            }
            let (ctr, half) = ((a + b) / 2.0, (b - a) * PARAMETRIC_WIDTH + 0.05);
            let t = ((x - ctr) / half).clamp(-1.0, 1.0);
            let win = 0.5 + 0.5 * (t * std::f32::consts::PI).cos();
            d += (amt / 100.0) as f32 * PARAMETRIC_AMOUNT * win;
        }
        (x + d * 4.0 * x * (1.0 - x)).clamp(0.0, 1.0)
    });
    // keep monotone
    for i in 1..CURVE_LUT_N {
        base.v[i] = base.v[i].max(base.v[i - 1]);
    }
    Some(base)
}

/// Camera Raw's parametric curve: its measured change per region ([`crate::parametric_adobe`]),
/// interpolated in the slider value and summed, with the input moved from the photo's splits to the
/// default 25 / 50 / 75 (exact for Bryan's preset: 0.3 of a level; non-default splits are
/// approximate, ~2–6 levels).
fn parametric_adobe_lut(c: &ToneCurve) -> Lut1 {
    use crate::parametric_adobe::{PARAMETRIC_N, PARAMETRIC_STOPS, PARAMETRIC_TABLE};
    let fin = |v: f64, d: f64| if v.is_finite() { v } else { d };
    // the slider value as (stop index, weight) pairs into the table (0 contributes nothing)
    let weights = |v: f64| -> [(usize, f32); 2] {
        let v = fin(v, 0.0).clamp(-100.0, 100.0) as f32;
        let k = [PARAMETRIC_STOPS[0], PARAMETRIC_STOPS[1], 0.0, PARAMETRIC_STOPS[2], PARAMETRIC_STOPS[3]];
        let i = (0..4).find(|&i| v <= k[i + 1]).unwrap_or(3);
        let t = ((v - k[i]) / (k[i + 1] - k[i])).clamp(0.0, 1.0);
        // stops 0, 1, (zero), 2, 3 → table index or none
        let idx = |j: usize| match j {
            0 => Some(0),
            1 => Some(1),
            3 => Some(2),
            4 => Some(3),
            _ => None,
        };
        let a = idx(i).map_or((0, 0.0), |s| (s, 1.0 - t));
        let b = idx(i + 1).map_or((0, 0.0), |s| (s, t));
        [a, b]
    };
    let regions = [c.shadows, c.darks, c.lights, c.highlights].map(weights);
    let splits = [fin(c.split_shadows, 25.0), fin(c.split_mid, 50.0), fin(c.split_highlights, 75.0)].map(|v| (v / 100.0).clamp(0.0, 1.0) as f32);
    let knots = [0.0, splits[0], splits[1].max(splits[0]), splits[2].max(splits[1]).max(splits[0]), 1.0];
    let warp = |x: f32| {
        // the photo's splits → the default ones the table was measured with
        let i = (0..4).find(|&i| x <= knots[i + 1]).unwrap_or(3);
        let span = knots[i + 1] - knots[i];
        let t = if span > 1e-6 { ((x - knots[i]) / span).clamp(0.0, 1.0) } else { 0.0 };
        (i as f32 + t) * 0.25
    };
    Lut1::from_fn(CURVE_LUT_N, |x| {
        let f = warp(x) * (PARAMETRIC_N - 1) as f32;
        let j = (f as usize).min(PARAMETRIC_N - 2);
        let t = f - j as f32;
        let mut d = 0.0;
        for (r, w) in regions.iter().enumerate() {
            for &(s, k) in w {
                if k != 0.0 {
                    let row = &PARAMETRIC_TABLE[r][s];
                    d += k * (row[j] + (row[j + 1] - row[j]) * t);
                }
            }
        }
        (x + d).clamp(0.0, 1.0)
    })
}

/// Vignette (post-crop) parameters.
#[derive(Clone, Copy, Debug)]
pub struct Vig {
    pub amount: f32,
    pub start: f32,
    pub width: f32,
    pub aspect_mix: f32,
    pub power: f32,
    pub highlights: f32,
    pub style: VignetteStyle,
}

fn vignette(s: &DevelopSettings) -> Option<Vig> {
    let v = &s.vignette;
    (v.amount != 0.0).then(|| {
        let r = (v.roundness / 100.0) as f32;
        Vig {
            amount: (v.amount / 100.0) as f32,
            start: 0.15 + (v.midpoint / 100.0) as f32 * 0.95,
            width: 0.05 + (v.feather / 100.0) as f32 * 1.1,
            aspect_mix: ((r + 1.0) / 2.0).clamp(0.0, 1.0),
            power: if r >= 0.0 { 2.0 } else { 2.0 + (-r) * 6.0 },
            highlights: (v.highlights / 100.0) as f32,
            style: v.style,
        }
    })
}

/// Local (mask) Sharpness gain per unit of its ±100 slider.
pub const SHARPEN_GAIN: f32 = 1.0;
/// Capture sharpening, measured with Camera Raw (R07, `docs/lr-match.md` → Round 8;
/// `oracle/make_detail.py sharp` + `sharp_ana.py`): gain `SHARPEN_AMOUNT_GAIN · (Amount/150)^SHARPEN_AMOUNT_EXP`
/// on log luminance at the source's full size.
pub const SHARPEN_AMOUNT_GAIN: f32 = 8.14;
pub const SHARPEN_AMOUNT_EXP: f32 = 0.8225;
/// Detail: differences beyond about `T0 + T1 · Detail/100` EV are damped.
pub const SHARPEN_DETAIL_T0: f32 = 0.006;
pub const SHARPEN_DETAIL_T1: f32 = 0.405;
/// Edge Masking: sharpening fades in over log-luminance gradients (EV per source px, smoothed by
/// a Gaussian of `SHARPEN_MASK_SIGMA` source px) from `SHARPEN_MASK_LO · c` to
/// `SHARPEN_MASK_HI · c`, `c = SHARPEN_MASK_C0 · exp(SHARPEN_MASK_K · Masking/100)`.
pub const SHARPEN_MASK_SIGMA: f32 = 1.6;
pub const SHARPEN_MASK_C0: f32 = 0.0075;
pub const SHARPEN_MASK_K: f32 = 3.52;
pub const SHARPEN_MASK_LO: f32 = -0.275;
pub const SHARPEN_MASK_HI: f32 = 2.275;
/// Gaussian sigma per unit of Radius, in the source's own pixels.
pub const SHARPEN_SIGMA: f32 = 0.818;
/// Largest kernel half-width (taps each side): radii beyond ~1.6 source px are truncated there.
pub const SHARPEN_TAPS: usize = 4;

/// Lightroom-style capture sharpening: an unsharp mask of log luminance whose radius is in the
/// source's own pixels (so a preview shows what the full-size file gets, scaled), with Detail
/// damping large differences (halos) and Masking limiting it to edges (by the kernel's
/// least-squares gradient). The kernel is read straight from the log-luminance plane (CPU and
/// GPU alike), `r` taps each side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sharpen {
    /// Taps each side (0 = the radius is below a pixel's worth at this scale: no effect).
    pub r: u32,
    /// 1-D Gaussian weights for offsets 0..=r (normalised so the 2-D kernel sums to 1).
    pub w: [f32; SHARPEN_TAPS + 1],
    /// Detail: differences larger than about `t` EV are damped (`d / (1 + |d| / t)`).
    pub t: f32,
    /// Edge Masking's gradient: taps each side, 1-D Gaussian weights ([`SHARPEN_MASK_SIGMA`])
    /// and the normaliser turning `Σ wg(dx) wg(dy) dx · l` into EV per source pixel.
    pub gr: u32,
    pub gw: [f32; SHARPEN_TAPS + 1],
    pub gn: f32,
}

/// 1-D Gaussian weights of `sigma` (normalised to sum to 1) and their taps each side.
fn gauss_taps(sigma: f32) -> (usize, [f32; SHARPEN_TAPS + 1]) {
    let mut w = [0.0; SHARPEN_TAPS + 1];
    if !(sigma.is_finite() && sigma >= 0.25) {
        return (0, w);
    }
    let r = ((2.5 * sigma).ceil() as usize).clamp(1, SHARPEN_TAPS);
    for (k, v) in w.iter_mut().enumerate().take(r + 1) {
        *v = (-((k * k) as f32) / (2.0 * sigma * sigma)).exp();
    }
    let sum: f32 = w[0] + 2.0 * w[1..=r].iter().sum::<f32>();
    for v in w.iter_mut() {
        *v /= sum;
    }
    (r, w)
}

impl Sharpen {
    pub fn new(radius: f64, detail: f64, px_per_long: f64, native_long: f32) -> Sharpen {
        let native = if native_long.is_finite() && native_long > 16.0 { native_long } else { 6000.0 };
        let scale = (px_per_long as f32 / native).clamp(0.0, 4.0);
        let scale = if scale.is_finite() { scale } else { 0.0 };
        let fin = |v: f64, d: f32| if v.is_finite() { v as f32 } else { d };
        let t = SHARPEN_DETAIL_T0 + SHARPEN_DETAIL_T1 * (fin(detail, 25.0) / 100.0).clamp(0.0, 1.0);
        let (r, w) = gauss_taps(SHARPEN_SIGMA * fin(radius, 1.0).clamp(0.5, 3.0) * scale);
        if r == 0 {
            return Sharpen { r: 0, w, t, gr: 0, gw: [0.0; SHARPEN_TAPS + 1], gn: 1.0 };
        }
        let (gr, gw) = gauss_taps(SHARPEN_MASK_SIGMA * scale);
        // Σ_dy wg(dy) · Σ_dx wg(dx) dx² (the first factor is 1), per output px → per source px
        let gn: f32 = (1..=gr).map(|k| 2.0 * gw[k] * (k * k) as f32).sum::<f32>() / scale.max(1e-6);
        Sharpen { r: r as u32, w, t, gr: gr as u32, gw, gn: if gn > 1e-9 && gn.is_finite() { gn } else { 1.0 } }
    }

    /// The Edge Masking weight of a pixel whose smoothed gradient is `grad` (`masking` 0..1).
    #[inline]
    pub fn mask(masking: f32, grad: f32) -> f32 {
        if masking <= 0.0 {
            return 1.0;
        }
        let c = SHARPEN_MASK_C0 * (SHARPEN_MASK_K * masking.min(1.0)).exp();
        smooth(SHARPEN_MASK_LO * c, SHARPEN_MASK_HI * c, grad)
    }

    /// (blurred log luminance, smoothed gradient magnitude in EV per source px — 0 unless
    /// `grad`) around `(x, y)` of the `w × h` plane `l`.
    #[inline]
    pub fn sample(&self, l: &[f32], w: usize, h: usize, x: usize, y: usize, grad: bool) -> (f32, f32) {
        if w == 0 || h == 0 {
            return (0.0, 0.0);
        }
        let r = if grad { self.r.max(self.gr) } else { self.r } as i64;
        let tap = |k: &[f32; SHARPEN_TAPS + 1], d: i64| k.get(d.unsigned_abs() as usize).copied().unwrap_or(0.0);
        let (mut acc, mut gx, mut gy) = (0.0f32, 0.0f32, 0.0f32);
        for dy in -r..=r {
            let yy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
            let (wy, gwy) = (tap(&self.w, dy), tap(&self.gw, dy));
            for dx in -r..=r {
                let xx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                let v = l.get(yy * w + xx).copied().unwrap_or(0.0);
                acc += wy * tap(&self.w, dx) * v;
                if grad {
                    let g = gwy * tap(&self.gw, dx) * v;
                    gx += g * dx as f32;
                    gy += g * dy as f32;
                }
            }
        }
        (acc, gx.hypot(gy) / self.gn)
    }
}

/// How far (EV) a pixel may stray from the local base before Highlights / Shadows follow the
/// pixel itself instead (see [`halo_weight`]).
pub const HALO_TAU: f32 = 0.3;

/// Weight of the local base for a pixel `d` EV from it: 1 in texture, falling to 0 next to edges
/// the base blurred across (where a base-driven change would draw a halo).
#[inline]
pub fn halo_weight(d: f32) -> f32 {
    let q = d / HALO_TAU;
    if q.is_finite() { (-(q * q)).exp() } else { 0.0 }
}

/// Grain measured with Camera Raw (R06, `docs/lr-match.md` → Detail and Effects; `oracle/make_detail.py
/// grain6000` + `grain_ana.py`): by Size (knots [`GRAIN_SIZES`]), the grain's cell in pixels of a
/// [`GRAIN_REF_LONG`] px long edge at Roughness 50 (matched to Camera Raw's lag-1 autocorrelation)…
pub const GRAIN_SIZES: [f32; 5] = [0.0, 11.0, 25.0, 50.0, 100.0];
pub const GRAIN_CELL_REF: [f32; 5] = [0.45, 0.72, 1.37, 2.33, 4.05];
/// … and its strength: L* standard deviation at mid grey for Amount 50 (finer grain is stronger).
pub const GRAIN_SD_REF: [f32; 5] = [9.31, 6.25, 3.72, 2.77, 2.38];
pub const GRAIN_REF_LONG: f32 = 6000.0;
/// Camera Raw's grain follows the long edge, less than in proportion: cells × (long / 6000)^this.
pub const GRAIN_RES_EXP: f32 = 0.7;
/// L* per unit of encoded value at mid grey, and the standard deviation of [`grain_noise`].
const GRAIN_L_PER_E: f32 = 100.0;
const GRAIN_NOISE_SD: f32 = 0.386;
/// The noise lattice is turned by atan(1/2) (cos, sin) so it never lines up with the pixels.
pub const GRAIN_ROT: [f32; 2] = [0.894_427_2, 0.447_213_6];
/// Grain strength by encoded luminance (Camera Raw's, measured in L* on five grey levels): full
/// from `GRAIN_KNEE` up, `(lum / knee)^0.75` below, falling by `GRAIN_HI` per unit above `GRAIN_HI0`.
pub const GRAIN_KNEE: f32 = 0.21;
pub const GRAIN_HI0: f32 = 0.74;
pub const GRAIN_HI: f32 = 0.39;

/// Piecewise-linear `ys` at `x` over increasing `xs` (held at the ends).
fn knots(x: f32, xs: &[f32; 5], ys: &[f32; 5]) -> f32 {
    if !x.is_finite() || x <= xs[0] {
        return ys[0];
    }
    for i in 1..5 {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]).max(1e-6);
            return ys[i - 1] + (ys[i] - ys[i - 1]) * t;
        }
    }
    ys[4]
}

/// Grain's strength by the encoded luminance it lands on ([`GRAIN_KNEE`]).
#[inline]
pub fn grain_tone(lum: f32) -> f32 {
    if !lum.is_finite() {
        return 0.0;
    }
    let lo = (lum / GRAIN_KNEE).clamp(0.0, 1.0).powf(0.75);
    lo * (1.0 - GRAIN_HI * (lum - GRAIN_HI0).clamp(0.0, 1.0))
}

/// Grain as Camera Raw renders it (measured, R06): luminance-only noise on the finished image,
/// cells sized in the frame (by Size and Roughness, scaled with the long edge as Camera Raw does),
/// strength by Size, Roughness (stronger at both ends) and Amount (linear) and by tone
/// ([`grain_tone`]). A render smaller than the source averages several cells into each pixel:
/// finer, weaker grain (amplitude × cell size in output pixels, below one pixel), as the
/// downscaled full-size render. Returns (amplitude, cells per long edge, seed).
pub fn grain_params(amount: f64, size: f64, roughness: f64, seed: u32, px_per_long: f64, native_long: f32) -> (f32, f32, u32) {
    let native = if native_long.is_finite() && native_long > 16.0 { native_long } else { GRAIN_REF_LONG };
    let fin = |v: f64, d: f32| if v.is_finite() { v as f32 } else { d };
    let d = (fin(roughness, 50.0) / 100.0).clamp(0.0, 1.0) - 0.5;
    let res = (native / GRAIN_REF_LONG).clamp(0.05, 8.0).powf(GRAIN_RES_EXP);
    let c0 = knots(fin(size, 25.0).clamp(0.0, 100.0), &GRAIN_SIZES, &GRAIN_CELL_REF) * res;
    let sd = knots(c0, &GRAIN_CELL_REF, &GRAIN_SD_REF) * (1.0 - 0.19 * d + 1.66 * d * d);
    let cell = (c0 * (1.0 + 0.58 * d + 0.4 * d * d)).max(0.05);
    let cell_out = cell * (fin(px_per_long, native) / native);
    let att = if cell_out.is_finite() { cell_out.clamp(0.05, 1.0) / cell.clamp(0.05, 1.0) } else { 1.0 };
    let amp = (fin(amount, 0.0) / 50.0).clamp(0.0, 2.0) * sd / (GRAIN_L_PER_E * GRAIN_NOISE_SD) * att;
    (amp, native / cell, seed)
}

/// Hash constants of [`grain_noise`] (shared with the GPU kernel).
pub const GRAIN_HASH: [u32; 4] = [0x8da6_b343, 0xd816_3841, 0xcb1a_b31f, 0x5bd1_e995];

#[inline]
fn grain_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let h = |i: i32, j: i32| {
        let mut v = (i as u32).wrapping_mul(GRAIN_HASH[0]) ^ (j as u32).wrapping_mul(GRAIN_HASH[1]) ^ seed.wrapping_mul(GRAIN_HASH[2]);
        v ^= v >> 13;
        v = v.wrapping_mul(GRAIN_HASH[3]);
        v ^= v >> 15;
        (v & 0xffff) as f32 / 32768.0 - 1.0
    };
    let (i, j) = (x0 as i32, y0 as i32);
    let (u, v) = (fx, fy);
    let a = h(i, j) + (h(i + 1, j) - h(i, j)) * u;
    let b = h(i, j + 1) + (h(i + 1, j + 1) - h(i, j + 1)) * u;
    a + (b - a) * v
}

/// Number of per-mask terms in [`mask_terms`].
pub const MASK_TERMS: usize = 21;

/// Number of alpha-weighted terms at the start of [`mask_terms`].
pub const MASK_SUMS: usize = 17;

/// A mask's local adjustments as the per-pixel stage uses them (alpha-weighted sums), in order:
/// exposure, temp, tint, contrast, highlights, shadows, whites, blacks, texture, clarity, dehaze,
/// saturation, hue, sharpness, noise, moiré, defringe, then the colour overlay: on (0/1), cos and
/// sin of its OkLCh hue, and its strength.
pub fn mask_terms(j: &LocalAdjustments) -> [f32; MASK_TERMS] {
    let (on, cos, sin, amt) = if j.color_sat > 0.0 {
        let hue = crate::colorops::oklch_hue_of_srgb_hue(j.color_hue);
        (1.0, hue.cos(), hue.sin(), (j.color_sat / 100.0) as f32)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    [
        j.exposure as f32,
        (j.temp / 100.0) as f32,
        (j.tint / 100.0) as f32,
        (j.contrast / 100.0) as f32,
        (j.highlights / 100.0) as f32,
        (j.shadows / 100.0) as f32,
        (j.whites / 100.0) as f32,
        (j.blacks / 100.0) as f32,
        (j.texture / 100.0) as f32,
        (j.clarity / 100.0) as f32,
        (j.dehaze / 100.0) as f32,
        (j.saturation / 100.0) as f32,
        (j.hue / 100.0) as f32 * 0.6,
        (j.sharpness / 100.0) as f32,
        (j.noise / 100.0) as f32,
        (j.moire / 100.0) as f32,
        (j.defringe / 100.0) as f32,
        on,
        cos,
        sin,
        amt,
    ]
}

/// Everything the per-pixel stage computes once per render: the CPU loop below and the GPU kernel
/// (`lightcraft-gpu`) both read their parameters from here, so the two cannot drift apart.
/// The space the "look" stages act in: the tone curves (parametric + point), a creative profile's
/// colour table and grain see the colour in ProPhoto primaries encoded with the sRGB curve
/// (Lightroom's "Melissa RGB", the space of its curve and histogram), for every output space.
/// Before 2026-11-21 they acted on the *output* primaries, so an Adobe RGB, Display P3 or ProPhoto
/// export of a photo with a curve or a creative look differed in colour from its sRGB export (and
/// from the screen), and the look's table only saw colours already clipped to the output gamut.
pub const STYLE_SPACE: lightcraft_color::RgbSpace = lightcraft_color::PROPHOTO;
/// Luminance weights of [`STYLE_SPACE`] (`PROPHOTO.luma()`, rounded), for Refine Saturation and
/// grain on style-space colours (the GPU kernel has the same numbers).
pub const STYLE_LUMA: [f32; 3] = [0.288_075, 0.711_835, 0.000_090];

pub struct FinishParams {
    /// A LUT profile and its blend (the amount 0..2 through [`crate::lut::Lut3d::blend`]):
    /// applied to the display-encoded colour or, for a profile table, after the tone map.
    pub lut: Option<(std::sync::Arc<crate::lut::Lut3d>, f32)>,
    pub tone: ToneMap,
    pub ops: ColorOps,
    /// Calibration: primaries matrix (row-major, linear Rec.2020) and shadows tint (−1..1).
    pub calib: Option<[[f32; 3]; 3]>,
    pub shadow_tint: f32,
    /// Tone curves (parametric ∘ point as the hue-preserving master curve, then per channel) on
    /// encoded values.
    pub curves: Option<Curves>,
    /// Refine Saturation as 0..1 (1 = the curves' own saturation).
    pub refine_sat: f32,
    pub vig: Option<Vig>,
    /// Linear Rec.2020 → linear output RGB, the output's luminance weights (gamut mapping) and its
    /// encoding curve (see [`crate::output`]).
    pub to_out: [[f32; 3]; 3],
    pub out_luma: [f32; 3],
    pub out_trc: OutputTrc,
    /// Linear Rec.2020 → linear [`STYLE_SPACE`] and back: where the tone curves, a creative
    /// profile's colour table and grain act, whatever the output space (see [`STYLE_SPACE`]).
    pub to_style: [[f32; 3]; 3],
    pub from_style: [[f32; 3]; 3],
    /// Soft proofing (CPU only; the GPU path declines proof renders).
    pub proof: Option<crate::output::ProofParams>,
    pub hl: f32,
    pub sh: f32,
    /// Lightroom-matched Highlights / Shadows ([`crate::tone::lr::hs_lut`]): replaces `hl` / `sh`
    /// (which are then 0) for raw files with a camera tone curve.
    pub hs_lut: Option<Vec<f32>>,
    /// An Adobe camera base (+ look) for this render ([`crate::adobe::Finish`]): look tables and
    /// the base curve channel-wise in ProPhoto instead of the luminance tone map.
    pub adobe: Option<crate::adobe::Finish>,
    pub clar: f32,
    pub tex: f32,
    pub dehaze: f32,
    /// Sharpening ([`Sharpen`]): amount gain, edge masking 0..1 and the kernel.
    pub sharpen: f32,
    pub sharpen_mask: f32,
    pub sharp: Sharpen,
    /// Airlight after and before exposure, exposure gain and EV (see [`crate::Prepared`]).
    pub air: f32,
    pub air_pre: f32,
    pub gain: f32,
    pub ev: f32,
    /// Camera Raw's negative Dehaze key (the photo's 99th log2 luminance percentile plus
    /// Exposure, [`crate::dehaze_cr`]) when it applies; read by the GPU kernel.
    pub dz_key: Option<f32>,
    /// Grain ([`grain_params`]): amplitude, cells per long edge, seed.
    pub grain: Option<(f32, f32, u32)>,
    /// Output px → normalized oriented coordinates.
    pub out_to_norm: lightcraft_geom::Affine,
    pub ow: f64,
    pub oh: f64,
    pub w: usize,
    pub h: usize,
    /// Where the `w × h` buffer sits in the whole render: x, y and the whole render's width and
    /// height (0, 0, w, h unless this is a region render, [`crate::region`]). Positions over the
    /// whole frame (vignette, the scene tone map) are taken there.
    pub win: (usize, usize, usize, usize),
    pub px_per_long: f64,
    /// Peak linear output relative to SDR white: 1 for SDR renders; `2^max_ev` for an HDR render
    /// ([`OutputDepth::F32Hdr`] with HDR on), which keeps values above 1 instead of clipping them;
    /// an HDR display's view with less headroom: that headroom ([`lightcraft_develop::Hdr::view_peak`]).
    pub peak: f32,
    /// The SDR rendition's highlight saturation (`crs:SDRBlend` / 100, −1..1, see
    /// [`highlight_saturation`]); 0 for every other render.
    pub hl_sat: f32,
    /// Camera Raw's scene tone map for this render ([`crate::scene_cr`], [`FinishParams::with_scene`]):
    /// Clarity, Highlights / Shadows and positive Dehaze, when the settings use them.
    pub scene: Option<std::sync::Arc<crate::scene_cr::SceneTone>>,
    /// The camera RGB Texture's gain applies in ([`crate::texture_cr::space`]): working → camera, back.
    pub tex_space: Option<([[f32; 3]; 3], [[f32; 3]; 3])>,
    /// The photo's tone statistics the tone map was made with (Contrast's pivot key, Whites' white).
    pub tone_stats: Option<crate::tone::lr::ToneStats>,
}

/// The sliders that move Camera Raw's HDR tone ([`crate::tone_hdr`]).
fn hdr_sliders(s: &DevelopSettings) -> crate::tone_hdr::Sliders {
    let f = |v: f64| if v.is_finite() { v as f32 } else { 0.0 };
    crate::tone_hdr::Sliders { whites: f(s.light.whites), contrast: f(s.light.contrast) }
}

/// Chroma gain of `crs:SDRBlend` at ±100 on the brightest SDR tones (Camera Raw's "highlight
/// saturation": RAWmakase measured no luminance change and about ±0.1 saturation at ±100,
/// above about half of SDR white).
pub const HL_SAT_GAIN: f32 = 0.25;

/// `crs:SDRBlend` on a display-linear colour `d` (Rec.2020): its chroma around its luminance
/// scaled by `1 + HL_SAT_GAIN · k`, faded in from half of SDR white to white. Luminance is kept.
#[inline]
pub fn highlight_saturation(d: [f32; 3], k: f32) -> [f32; 3] {
    let y = luminance_2020(d);
    let f = 1.0 + HL_SAT_GAIN * k * smooth(0.5, 1.0, y);
    if f == 1.0 {
        return d;
    }
    d.map(|v| (y + (v - y) * f).max(0.0))
}

impl FinishParams {
    /// Parameters for a `w × h` render into `space`; `ev`/`air_pre` as in [`crate::Prepared`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        s: &DevelopSettings,
        frame: &Frame,
        info: &SourceInfo,
        w: usize,
        h: usize,
        px_per_long: f64,
        air_pre: f32,
        tone_stats: Option<crate::tone::lr::ToneStats>,
        space: OutputSpace,
    ) -> FinishParams {
        let effects = s.section_enabled("effects");
        let (clar, tex, dehaze) = if effects {
            ((s.effects.clarity / 100.0) as f32, (s.effects.texture / 100.0) as f32, (s.effects.dehaze / 100.0) as f32)
        } else {
            (0.0, 0.0, 0.0)
        };
        let ev = s.light.exposure as f32;
        let gain = 2f32.powf(ev);
        let grain = (s.grain.amount > 0.0 && effects)
            .then(|| grain_params(s.grain.amount, s.grain.size, s.grain.roughness, s.grain.seed, px_per_long, info.native_long));
        let calibration = s.section_enabled("calibration");
        // raw files with a camera tone curve take Lightroom-matched Basic tone
        // raw files with a camera tone curve, and rendered files seen through the reference
        // curve (`tone::rendered_reference`), take Lightroom-matched Basic tone
        let adobe_key = info.adobe.map(|k| white::for_render(k, info, s));
        let adobe = crate::adobe::Finish::new(adobe_key, &s.profile.id, s.profile.amount / 100.0, s.light.exposure);
        let lr_tone = info.camera_tone.is_some() || adobe.is_some();
        let (hl, sh) = ((s.light.highlights / 100.0) as f32, (s.light.shadows / 100.0) as f32);
        let tone_key = tone_stats.map(|t| t.key);
        // Lightroom's Highlights / Shadows follow the whole photo (`tone::lr::adobe_hs_lut`)
        let key_offset = tone_key.map_or(0.0, |k| k + s.light.exposure as f32 - crate::tone::lr::KEY_REF);
        let hs_lut = (lr_tone && (hl != 0.0 || sh != 0.0)).then(|| match adobe.as_ref() {
            Some(_) if ADOBE_HS_MEASURED => crate::tone::lr::adobe_hs_lut(hl, sh, tone_stats, s.light.exposure as f32),
            Some(_) => crate::tone::lr::adobe_hs_lut_previews(hl, sh, key_offset),
            None => crate::tone::lr::hs_lut(hl, sh, key_offset),
        });
        let (hl, sh) = if lr_tone { (0.0, 0.0) } else { (hl, sh) };
        // HDR edits (an HDR render only: every SDR render gets the SDR rendition, HDR off)
        // (an HDR display with less headroom than the limit sees the edit as a gain map shows it)
        let (peak, view) = (s.hdr.peak(), s.hdr.view_peak());
        FinishParams {
            peak: view,
            hl_sat: if s.hdr.enabled || !s.hdr.rendition_blend.is_finite() { 0.0 } else { (s.hdr.rendition_blend / 100.0).clamp(-1.0, 1.0) as f32 },
            calib: if calibration { crate::colorops::calibration_matrix(&s.calibration) } else { None },
            shadow_tint: if calibration { (s.calibration.shadows_tint / 100.0) as f32 } else { 0.0 },
            tone: if let Some(a) = adobe.as_ref() {
                ToneMap::adobe_lr(&a.base, s.light.exposure, s.light.contrast, s.light.whites, s.light.blacks, tone_stats)
            } else if let Some(curve) = info.camera_tone.as_ref() {
                ToneMap::camera_lr(curve, s.light.exposure, s.light.contrast, s.light.whites, s.light.blacks)
            } else if info.raw {
                ToneMap::new(s.light.contrast, s.light.whites, s.light.blacks)
            } else {
                ToneMap::display(s.light.contrast, s.light.whites, s.light.blacks)
            }
            .with_hdr_view(peak, view, hdr_sliders(s)),
            lut: crate::lut::get(&s.profile.id).map(|l| {
                let k = l.blend((s.profile.amount / 100.0) as f32);
                (l, k)
            }),
            ops: ColorOps::new(s).with_measured(s),
            // an Adobe look's own curves go under the user's (with an Adobe base only: the looks
            // are raw-only in Lightroom too)
            curves: match adobe.as_ref().and(crate::adobe::look_for(&s.profile.id)) {
                Some(look) => curve_luts(&s.curve, Some(&look.curves)),
                None => curve_luts(&s.curve, None),
            },
            refine_sat: (s.curve.refine_saturation / 100.0).clamp(0.0, 1.0) as f32,
            vig: if effects { vignette(s) } else { None },
            to_out: space.from_working(),
            out_luma: space.luma(),
            out_trc: space.trc(),
            to_style: lightcraft_color::REC2020.to_space(&STYLE_SPACE).to_f32(),
            from_style: STYLE_SPACE.to_space(&lightcraft_color::REC2020).to_f32(),
            proof: None,
            hl,
            sh,
            hs_lut,
            adobe,
            clar,
            tex,
            dehaze,
            sharpen: ((s.detail.sharpen_amount / 150.0).clamp(0.0, 2.0) as f32).powf(SHARPEN_AMOUNT_EXP) * SHARPEN_AMOUNT_GAIN,
            sharpen_mask: (s.detail.sharpen_masking / 100.0) as f32,
            sharp: Sharpen::new(s.detail.sharpen_radius, s.detail.sharpen_detail, px_per_long, info.native_long),
            air: air_pre * gain,
            air_pre,
            gain,
            ev,
            dz_key: None,
            grain,
            out_to_norm: frame.out_to_norm(w, h),
            ow: frame.ow,
            oh: frame.oh,
            w,
            h,
            win: {
                let ((x, y), (fw, fh)) = (frame.offset(), frame.full_size(w, h));
                (x, y, fw, fh)
            },
            px_per_long,
            scene: None,
            tex_space: crate::texture_cr::space(info),
            tone_stats,
        }
    }

    /// Attach the photo's scene tone map (`map`, `haze`: [`crate::Prepared`]'s) for `s`: Clarity and
    /// positive Dehaze always (with their `MEASURED` switches), Highlights / Shadows of an Adobe
    /// base with [`crate::scene_cr::HS_MEASURED`] (which then replaces `hs_lut`).
    pub fn with_scene(
        mut self,
        map: Option<&std::sync::Arc<crate::scene_cr::MapBase>>,
        haze: Option<&std::sync::Arc<crate::scene_cr::Haze>>,
        s: &DevelopSettings,
    ) -> FinishParams {
        use crate::scene_cr as sc;
        let masks = |f: fn(&LocalAdjustments) -> f64| s.masks.iter().any(|m| f(&m.adjust) != 0.0);
        let clarity = sc::CLARITY_MEASURED && (self.clar != 0.0 || masks(|a| a.clarity));
        let hs_used = s.light.highlights != 0.0 || s.light.shadows != 0.0 || masks(|a| a.highlights) || masks(|a| a.shadows);
        let hs = sc::hs_measured() && self.adobe.is_some() && hs_used;
        let dehaze = sc::HAZE_MEASURED && (self.dehaze > 0.0 || s.masks.iter().any(|m| m.adjust.dehaze > 0.0));
        // Camera Raw's default black follows the photo's dark key, after Shadows (`black_cr`); an
        // HDR render too (its shadows are the SDR rendition's in Camera Raw: `docs/lr-match.md` → HDR tone)
        if let (Some(m), true) = (map, self.adobe.is_some()) {
            self.black_from_map(m, s, hs);
        }
        let map = map.filter(|_| clarity || hs);
        // (the haze is read on the map's grid: of the same frame, or both of the whole photo)
        let haze = haze.filter(|h| dehaze && map.is_none_or(|m| (m.w, m.h, m.whole) == (h.w, h.h, h.whole)));
        // a flat stand-in map when only the haze is needed (its grid is the haze's)
        let base = match (map, haze) {
            (Some(m), _) => m.clone(),
            (None, Some(h)) => match sc::MapBase::new(vec![0.18; h.w * h.h], h.w, h.h) {
                Some(m) => std::sync::Arc::new(if h.whole { m.whole_photo() } else { m }),
                None => return self,
            },
            (None, None) => return self,
        };
        let whole = base.whole;
        let mut t = sc::SceneTone::new(base, self.win.2, self.win.3, self.ev);
        if whole {
            // (the buffer's pixels are looked up at their place in the whole render: `out_to_norm` is the buffer's)
            t = t.framed(&self.out_to_norm, (self.win.0, self.win.1));
        }
        t.clarity = clarity && map.is_some();
        if t.clarity {
            // made here, outside the per-pixel stage's parallel rows: a lazily made detail inside
            // them would block every row on the one making it (whose parallel work then waits)
            let _ = t.map.detail();
        }
        if hs && let Some(m) = map {
            let hs_ev = if sc::hs_before_exposure() { 0.0 } else { self.ev };
            t.hs = Some((m.hs_curves(hs_ev), (s.light.shadows / 100.0) as f32, (s.light.highlights / 100.0) as f32));
            t.hs_shift = self.ev - hs_ev;
            self.hs_lut = None;
        }
        t.haze = haze.map(|h| (h.clone(), h.air.map(|a| a * self.gain)));
        self.scene = Some(std::sync::Arc::new(t));
        self
    }
}

impl FinishParams {
    /// The tone map with Camera Raw's default black at the photo's dark key ([`crate::black_cr`]),
    /// moved by Shadows when the rule renders it (`hs`).
    fn black_from_map(&mut self, m: &crate::scene_cr::MapBase, s: &DevelopSettings, hs: bool) {
        use crate::black_cr as b;
        let mode = b::mode();
        if mode == b::Mode::Off {
            return;
        }
        let Some(a) = self.adobe.as_ref() else { return };
        let ev = self.ev;
        let sh = (s.light.shadows / 100.0) as f32;
        let shift = if hs && sh != 0.0 { m.shadows_key_shift(ev, sh) } else { 0.0 };
        let key = m.dark + ev;
        let from = if mode == b::Mode::Shift { key } else { b::RAMP_KEY + ev };
        let Some(c) = b::correction(key + shift, from, b::remaining((s.light.blacks / 100.0) as f32)) else { return };
        self.tone = ToneMap::adobe_lr_black(&a.base, s.light.exposure, s.light.contrast, s.light.whites, s.light.blacks, self.tone_stats, Some(&c[..]))
            .with_hdr_view(s.hdr.peak(), self.peak, hdr_sliders(s));
    }
}

pub(crate) fn finish(p: &Prepared, s: &DevelopSettings, frame: &Frame, info: &SourceInfo, space: OutputSpace, proof: Option<crate::Proof>) -> Rgba8 {
    let (w, h) = (p.img.width, p.img.height);
    let mut fp =
        FinishParams::new(s, frame, info, w, h, p.px_per_long, p.air, p.tone_stats, space).with_scene(p.scene_map.as_ref(), p.haze.as_ref(), s);
    fp.proof = proof.map(|pr| pr.params(space));
    let trc = fp.out_trc;
    let data = finish_with(p, &fp, false, |e| match trc {
        OutputTrc::Srgb => [enc(e[0]), enc(e[1]), enc(e[2]), 255],
        t => {
            let x = e.map(|v| enc(t.encode(lightcraft_color::transfer::srgb_to_linear(v.clamp(0.0, 1.0)))));
            [x[0], x[1], x[2], 255]
        }
    });
    Rgba8 { width: w, height: h, data }
}

/// [`finish`] into 16-bit display-encoded or 32-bit float linear samples (output primaries).
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish_deep(
    p: &Prepared,
    s: &DevelopSettings,
    frame: &Frame,
    info: &SourceInfo,
    space: OutputSpace,
    depth: OutputDepth,
    proof: Option<crate::Proof>,
) -> DeepImage {
    use lightcraft_color::transfer::srgb_to_linear;
    let (w, h) = (p.img.width, p.img.height);
    let mut fp =
        FinishParams::new(s, frame, info, w, h, p.px_per_long, p.air, p.tone_stats, space).with_scene(p.scene_map.as_ref(), p.haze.as_ref(), s);
    fp.proof = proof.map(|pr| pr.params(space));
    let trc = fp.out_trc;
    let samples = match depth {
        OutputDepth::F32Linear => {
            let v = finish_with(p, &fp, true, |e| e.map(|v| srgb_to_linear(v.clamp(0.0, 1.0))));
            DeepSamples::F32(v.into_flattened())
        }
        OutputDepth::F32Hdr => {
            // the sRGB curve extended past 1 (it is unbounded), so values up to `peak` survive
            let peak = fp.peak;
            let v = finish_with(p, &fp, true, |e| e.map(|v| if v.is_finite() { srgb_to_linear(v.max(0.0)).clamp(0.0, peak) } else { 0.0 }));
            DeepSamples::F32(v.into_flattened())
        }
        _ => {
            let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
            let v = finish_with(p, &fp, true, |e| match trc {
                OutputTrc::Srgb => e.map(q),
                t => e.map(|v| q(t.encode(srgb_to_linear(v.clamp(0.0, 1.0))))),
            });
            DeepSamples::U16(v.into_flattened())
        }
    };
    DeepImage { width: w, height: h, space, samples }
}

/// The per-pixel stage: every output pixel's colour in the output primaries, encoded with the sRGB
/// curve (tone curves and grain applied; not clamped), handed to `store` for the final encoding.
/// `exact`: encode with the exact sRGB curve instead of the (8/10-bit accurate) table.
pub(crate) fn finish_with<T: Copy + Default + Send>(
    p: &Prepared,
    fp: &FinishParams,
    exact: bool,
    store: impl Fn([f32; 3]) -> T + Sync + Send,
) -> Vec<T> {
    let (w, h) = (p.img.width, p.img.height);
    let p_lut = fp.lut.clone().filter(|(l, k)| l.stage == crate::lut::LutStage::Output && *k > 0.0);
    let profile_lut = fp.lut.clone().filter(|(l, k)| l.stage == crate::lut::LutStage::Profile && *k > 0.0).map(|(l, k)| {
        let m = l.matrices();
        (l, k, m)
    });

    let FinishParams {
        tone,
        ops,
        curves,
        vig,
        to_out,
        out_luma,
        hl,
        sh,
        clar,
        tex,
        dehaze,
        sharpen,
        sharpen_mask,
        air,
        air_pre,
        gain,
        ev,
        grain,
        ..
    } = fp;
    let (hl, sh, clar, tex, dehaze, sharpen, sharpen_mask, air, air_pre, gain, ev) =
        (*hl, *sh, *clar, *tex, *dehaze, *sharpen, *sharpen_mask, *air, *air_pre, *gain, *ev);
    let terms: Vec<[f32; MASK_TERMS]> = p.masks.iter().map(|m| mask_terms(&m.adjust)).collect();
    let out_to_norm = fp.out_to_norm;
    let long = fp.ow.max(fp.oh);
    // a region render's pixels sit at (ox, oy) of a fw × fh render
    let (ox, oy, fw, fh) = fp.win;
    let aspect = fw.max(1) as f32 / fh.max(1) as f32;

    let srgb = srgb_lut();
    let peak = fp.peak;
    let hdr = peak > 1.0;
    let mut out = vec![T::default(); w * h];
    // measurement hook for tools/lr-compare (`LIGHTCRAFT_LR_DUMP=<file>`, CPU renders only): per
    // pixel the source colour, base, tone input EV, mask weight, encoded result, position, log L
    let dump = crate::dump::Dump::from_env(w, h);
    // measurement hook for refitting Clarity (`LIGHTCRAFT_CLARITY_DUMP=<file>`, CPU renders with
    // Camera Raw's Clarity): at every 4th pixel x, y, its level and `SceneTone::clarity_terms`
    let cla_dump: Option<std::sync::Mutex<Vec<f32>>> = std::env::var("LIGHTCRAFT_CLARITY_DUMP").ok().map(|_| std::sync::Mutex::new(Vec::new()));
    for_rows(&mut out, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let raw = p.img.data[i];
            let mut c = if gain == 1.0 { raw } else { raw.map(|v| v * gain) };
            let l_pre = p.log_l.data[i];
            let l0 = l_pre + ev;

            // --- local (mask) contributions: alpha-weighted sums of the masks' terms
            let mut l = [0.0f32; MASK_SUMS];
            let mut tint_col: Option<([f32; 3], f32)> = None;
            for (m, t) in p.masks.iter().zip(&terms) {
                let a = m.alpha.data[i];
                if a <= 0.0 {
                    continue;
                }
                for k in 0..MASK_SUMS {
                    l[k] += a * t[k];
                }
                if t[MASK_SUMS] > 0.0 {
                    tint_col = Some(([t[MASK_SUMS + 1], t[MASK_SUMS + 2], 0.0], a * t[MASK_SUMS + 3]));
                }
            }
            let [l_exp, l_temp, l_tint, l_con, l_hl, l_sh, l_wh, l_bl, l_tex, l_clar, l_dehaze, l_sat, l_hue, l_sharp, l_noise, l_moire, l_defringe] =
                l;

            // --- local Moiré (and the colour part of Noise): chromaticity towards its blur
            let mt = (l_moire + 0.5 * l_noise.max(0.0)).clamp(-1.0, 1.0);
            if mt != 0.0
                && let Some(cb) = &p.chroma_blur
            {
                let (y, ch0, cb) = (luminance_2020(c), crate::masks::chromaticity(raw), cb.data[i]);
                c = std::array::from_fn(|k| ((ch0[k] + (cb[k] - ch0[k]) * mt) * y).max(0.0));
            }
            // --- local Defringe: desaturate purple / green fringes along edges
            let df = l_defringe.clamp(0.0, 1.0);
            if df > 0.0
                && let Some(b) = &p.texture_blur
            {
                let k = df * defringe_weight(c, p.log_l.data[i] - b.data[i]);
                if k > 0.0 {
                    let y = luminance_2020(c);
                    c = c.map(|v| v + (y - v) * k);
                }
            }

            // Camera Raw's scene tone map (`scene_cr`): the pixel's neighbours on the map, and its
            // level after Exposure, before Dehaze and a mask's Exposure
            let sc = fp.scene.as_deref();
            let smp = sc.map(|t| t.sample(x + ox, y + oy));
            let l_scene = crate::scene_cr::level(luminance_2020(c));
            // --- dehaze (scene linear): positive with Camera Raw's haze model after local tone
            let dz = dehaze + l_dehaze;
            let haze_cr = dz > 0.0 && sc.is_some_and(|t| t.haze.is_some());
            if dz != 0.0
                && !haze_cr
                && let Some(dark) = &p.dark
            {
                let d = (dark.data[i] / air_pre).clamp(0.0, 1.0);
                if dz > 0.0 {
                    let t = (1.0 - 0.95 * dz.min(1.0) * d).max(0.12);
                    c = c.map(|v| ((v - air * (1.0 - t)) / t).max(0.0));
                } else if let Some(p99) = p.p99.filter(|_| crate::dehaze_cr::MEASURED) {
                    // Camera Raw's negative Dehaze (`dehaze_cr`): a tone curve per ProPhoto
                    // channel, relative to the photo's 99th percentile
                    let key = p99 + ev;
                    let m = crate::colorops::pp_matrices();
                    let q = mul3(&m.0, c);
                    // round 13: read at the regional level, the luminance detail scaled by (1 + s)
                    let detail = p.dz_blur.as_ref().and_then(|b| Some(p.log_l.data.get(i)? - b.data.get(i)?)).unwrap_or(0.0);
                    let s_dz = dz.max(-1.0);
                    let q =
                        q.map(|v| if v > 0.0 { v * crate::dehaze_cr::gain_spatial(s_dz, key, crate::dehaze_cr::level(v), detail).exp2() } else { v });
                    c = mul3(&m.1, q);
                } else {
                    let k = (-dz).min(1.0) * 0.7 * (0.35 + 0.65 * d);
                    c = c.map(|v| v + (air * 0.9 - v) * k);
                }
            }

            // --- local exposure / temp / tint
            if l_exp != 0.0 {
                let g = l_exp.exp2();
                c = c.map(|v| v * g);
            }
            if l_temp != 0.0 || l_tint != 0.0 {
                let y0 = luminance_2020(c);
                c = [c[0] * (1.0 + 0.3 * l_temp), c[1] * (1.0 - 0.22 * l_tint), c[2] * (1.0 - 0.3 * l_temp).max(0.0)];
                let y1 = luminance_2020(c).max(1e-9);
                c = c.map(|v| v * y0 / y1);
            }

            // --- local tone in log luminance
            let l1 = if dz != 0.0 || l_exp != 0.0 { log_lum(c) } else { l0 };
            let shift = l1 - l0;
            let base = p.base.data[i] + ev + shift;
            // Highlights / Shadows act on the edge-aware base, falling back to the pixel's own
            // level where it strays from the base (next to an edge the base filter blurred): no
            // halos, while texture keeps its local contrast
            let wb = halo_weight(l1 - base);
            let mut delta = match &fp.hs_lut {
                Some(t) => wb * crate::tone::lr::hs_at(t, base) + (1.0 - wb) * crate::tone::lr::hs_at(t, l1),
                None => 0.0f32,
            };
            let mut hs_delta = delta;
            let (hh, ss) = (hl + l_hl, sh + l_sh);
            // Camera Raw's Highlights / Shadows (`scene_cr`): the photo's rule at the base level
            let hs_cr = match (sc.and_then(|t| t.hs.as_ref().map(|h| (t, h))), smp.as_ref()) {
                (Some((t, (curves, gs, gh))), Some(m)) => {
                    let g = curves.gain(t.base(m, l_scene + l_exp) - t.hs_shift, gs + l_sh, gh + l_hl);
                    delta += g;
                    hs_delta = g;
                    true
                }
                _ => false,
            };
            if !hs_cr && (hh != 0.0 || ss != 0.0) {
                let legacy = |b: f32| {
                    let ws = 1.0 - smooth(-4.8, 0.3, b);
                    let wh = smooth(-1.0, 2.8, b);
                    ss * 1.7 * ws * ws.sqrt() + hh * 1.7 * wh
                };
                delta += wb * legacy(base) + (1.0 - wb) * legacy(l1);
            }
            if crate::local_tone_cr::MEASURED {
                // Camera Raw's local Contrast / Whites / Blacks (`local_tone_cr`): curves of the level
                use crate::local_tone_cr::{BLACKS, CONTRAST, WHITES, at};
                delta += at(&WHITES, l_wh, l1) + at(&BLACKS, l_bl, l1) + at(&CONTRAST, l_con, l1);
            } else {
                if l_wh != 0.0 {
                    delta += l_wh * 0.8 * smooth(0.5, 3.0, l1);
                }
                if l_bl != 0.0 {
                    delta += l_bl * 0.8 * (1.0 - smooth(-6.0, -1.5, l1));
                }
                if l_con != 0.0 {
                    delta += l_con * 0.14 * l1.clamp(-6.0, 4.0);
                }
            }
            let cl = clar + l_clar;
            let mut scene_gain = hs_delta;
            if let (Some(t), Some(m)) = (sc.filter(|t| t.clarity && crate::scene_cr::clarity_by_model(cl)), smp.as_ref()) {
                // Camera Raw's Clarity (`scene_cr`): four scales on the map, upsampled by the level
                let g = t.clarity_gain(m, cl, l_scene);
                if let Some(cd) = &cla_dump
                    && x % 4 == 0
                    && y % 4 == 0
                {
                    let terms = t.clarity_terms(m, l_scene);
                    let mut v = cd.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    v.extend([x as f32, y as f32, l_scene]);
                    v.extend(terms);
                }
                delta += g;
                scene_gain += g;
            } else if cl != 0.0
                && let Some(b) = &p.clarity_blur
            {
                let det = (l_pre - b.data[i]).clamp(-2.5, 2.5);
                let mid = (-(base / 3.2).powi(2)).exp();
                delta += cl * 0.85 * det * (0.35 + 0.65 * mid);
            }
            let tx = tex + l_tex;
            // Camera Raw's Texture (`texture_cr`): each channel's pyramid detail, scaled
            let tex_gain = match &p.texture_detail {
                Some(d) if tx != 0.0 => {
                    let k = crate::texture_cr::strength(tx);
                    let dd = d.data[i];
                    Some([(k * dd[0]).exp2(), (k * dd[1]).exp2(), (k * dd[2]).exp2()])
                }
                _ => None,
            };
            if tex_gain.is_none()
                && tx != 0.0
                && let Some(b) = &p.texture_blur
            {
                let det = l_pre - b.data[i];
                let tame = 1.0 - 0.6 * smooth(0.4, 1.6, det.abs());
                delta += tx * 1.1 * det.clamp(-1.0, 1.0) * tame;
            }
            let sp = l_sharp * 0.6 * SHARPEN_GAIN + sharpen;
            if sp != 0.0 && fp.sharp.r > 0 {
                let (blur, grad) = fp.sharp.sample(&p.log_l.data, w, h, x, y, sharpen_mask > 0.0);
                let det = l_pre - blur;
                let det = det / (1.0 + det.abs() / fp.sharp.t);
                delta += sp * det * Sharpen::mask(sharpen_mask, grad);
            }
            // local Noise: smooth (or, negative, boost) small-amplitude detail, keep edges
            if l_noise != 0.0
                && let Some(b) = &p.texture_blur
            {
                let det = l_pre - b.data[i];
                delta -= l_noise.clamp(-1.0, 1.0) * 0.9 * det * (1.0 - smooth(0.1, 0.5, det.abs()));
            }
            let local_delta = delta;
            if delta != 0.0 {
                let g = delta.exp2();
                c = c.map(|v| v * g);
            }
            // Camera Raw's positive Dehaze (`scene_cr`): its airlight follows the gains applied so far
            if haze_cr && let (Some(t), Some(m)) = (sc, smp.as_ref()) {
                c = t.dehaze(m, c, dz, (scene_gain + l_exp).exp2());
            }
            if let Some(g) = tex_gain {
                c = crate::texture_cr::apply(c, g, fp.tex_space.as_ref());
            }

            let tone_in = c;
            // --- calibration (scene linear, before the tone map)
            if fp.calib.is_some() || fp.shadow_tint != 0.0 {
                c = crate::colorops::calibrate(c, fp.calib.as_ref(), fp.shadow_tint);
            }

            // --- tone map: an Adobe base's curve channel-wise in ProPhoto (after its look
            // tables), else on luminance with highlight desaturation
            let mut d = match &fp.adobe {
                // channels above raw white reach the table (Camera Raw's tone is defined past it: Exposure +3
                // puts a quarter of a bright photo at white, which a clamp at 1.0 never reaches)
                Some(a) => a.post_tone(crate::adobe::rgb_tone_to(a.pre_tone(c), |v| tone.apply(v), crate::tone::HDR_SCENE_TOP)),
                None => {
                    let yl = luminance_2020(c);
                    let o = tone.apply(yl);
                    let mut d = if yl > 1e-9 { c.map(|v| v * o / yl) } else { [0.0; 3] };
                    let k = tone.chroma_scale(o);
                    if k != 1.0 {
                        d = d.map(|v| o + (v - o) * k);
                    }
                    let mx = d[0].max(d[1]).max(d[2]);
                    if mx > peak {
                        let t = ((mx - peak) / (mx - o).max(1e-6)).clamp(0.0, 1.0);
                        d = d.map(|v| v + (o - v) * t);
                    }
                    d
                }
            };

            // --- colour
            d = ops.apply(d, l_sat, l_hue);
            if fp.hl_sat != 0.0 {
                d = highlight_saturation(d, fp.hl_sat);
            }
            if let Some((dir, amt)) = tint_col {
                let lab = lightcraft_color::perceptual::oklab_from_2020(d);
                d = lightcraft_color::perceptual::oklab_to_2020([lab[0], lab[1] + dir[0] * 0.08 * amt, lab[2] + dir[1] * 0.08 * amt]);
            }

            // --- vignette (display linear, post-crop)
            if let Some(v) = vig {
                let u = ((x + ox) as f32 + 0.5) / fw.max(1) as f32 * 2.0 - 1.0;
                let vv = ((y + oy) as f32 + 0.5) / fh.max(1) as f32 * 2.0 - 1.0;
                let sx = 1.0 + (aspect - 1.0) * v.aspect_mix;
                let sy = 1.0 + (1.0 / aspect - 1.0) * v.aspect_mix;
                let (ax, ay) = ((u * sx.max(1.0) / sx.max(sy)).abs(), (vv * sy.max(1.0) / sx.max(sy)).abs());
                let dist = (ax.powf(v.power) + ay.powf(v.power)).powf(1.0 / v.power);
                let t = smooth(v.start, v.start + v.width, dist);
                if t > 0.0 {
                    let lum = luminance_2020(d).clamp(0.0, 1.0);
                    if v.amount < 0.0 {
                        let mut f = 1.0 + v.amount * t;
                        if v.style == VignetteStyle::HighlightPriority {
                            f += (1.0 - f) * v.highlights * smooth(0.4, 1.0, lum);
                        }
                        if v.style == VignetteStyle::PaintOverlay {
                            d = d.map(|c| c * (1.0 - (-v.amount) * t) + 0.0);
                        } else {
                            d = d.map(|c| c * f);
                        }
                    } else if hdr {
                        // lighten towards SDR white; never pull HDR highlights down to it
                        d = d.map(|c| c + (1.0 - c).max(0.0) * v.amount * t * 0.85);
                    } else {
                        d = d.map(|c| c + (1.0 - c) * v.amount * t * 0.85);
                    }
                }
            }

            // --- the look: curves, the profile's colour table (a creative profile's look, measured
            // against Lightroom: after the curves) and grain, in the style space (ProPhoto
            // primaries, sRGB curve) whatever the output space, so every export and the screen
            // agree; then back to linear working colour
            let styled = curves.is_some() || profile_lut.is_some() || grain.is_some();
            let style = |d: [f32; 3]| -> [f32; 3] {
                let enc_s = |v: f32| if exact { linear_to_srgb(v.clamp(0.0, 1.0)) } else { encode_srgb(srgb, v) };
                let mut e = mul3(&fp.to_style, d).map(enc_s);
                if let Some(l) = curves {
                    let e0 = e;
                    e = l.apply(e);
                    if fp.refine_sat < 1.0 {
                        e = refine_saturation(e0, e, fp.refine_sat);
                    }
                }
                if let Some((l, k, m)) = &profile_lut {
                    let lin = mul3(&fp.from_style, e.map(lightcraft_color::transfer::srgb_to_linear));
                    e = mul3(&fp.to_style, l.apply_linear(lin, *k, m)).map(enc_s);
                }
                if let Some((amt, cell, seed)) = *grain {
                    let n = out_to_norm.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                    let (gx, gy) = ((n.x * fp.ow) as f32 / long as f32 * cell, (n.y * fp.oh) as f32 / long as f32 * cell);
                    let [cr, sr] = GRAIN_ROT;
                    let g = grain_noise(cr * gx - sr * gy, sr * gx + cr * gy, seed);
                    let lum = STYLE_LUMA[0] * e[0] + STYLE_LUMA[1] * e[1] + STYLE_LUMA[2] * e[2];
                    let k = amt * g * grain_tone(lum);
                    e = e.map(|v| v + k);
                }
                mul3(&fp.from_style, e.map(|v| lightcraft_color::transfer::srgb_to_linear(v.clamp(0.0, 1.0))))
            };
            if styled && hdr {
                // HDR: the look acts on the SDR part (up to white, in the style space); what lies
                // above SDR white is carried over unchanged (the curves continue with slope 1)
                let q = mul3(&fp.to_style, d);
                let lq = mul3(&fp.to_style, style(mul3(&fp.from_style, q.map(|v| v.min(1.0)))));
                d = mul3(&fp.from_style, std::array::from_fn(|k| lq[k] + (q[k] - 1.0).max(0.0)));
            } else if styled {
                d = style(d);
            }

            // --- gamut map to the output space (desaturate towards luminance until in range);
            // soft proofing maps into the proof space first and shows that in the output space
            let mut warn = None;
            let mut r = match &fp.proof {
                Some(pp) => {
                    let q0 = mul3(&pp.to_proof, d);
                    let (q, t) = gamut_map(q0, pp.luma);
                    if pp.dest_warning && out_of_gamut(q0, t) {
                        warn = Some(crate::output::PROOF_DEST_WARNING);
                    }
                    mul3(&pp.proof_to_out, q)
                }
                None => mul3(to_out, d),
            };
            let (mapped, t) = gamut_map_to(r, *out_luma, peak);
            if fp.proof.is_some_and(|pp| pp.display_warning) && warn.is_none() && out_of_gamut(r, t) {
                warn = Some(crate::output::PROOF_DISPLAY_WARNING);
            }
            r = mapped;

            // --- encode (sRGB curve; `store` re-encodes with the output's own curve)
            let e = if hdr {
                // HDR: the (unbounded) sRGB curve, so values above SDR white encode above 1
                r.map(|v| if v > 1.0 { linear_to_srgb(v) } else { linear_to_srgb(v.max(0.0)) })
            } else if exact {
                r.map(|v| linear_to_srgb(v.clamp(0.0, 1.0)))
            } else {
                r.map(|v| encode_srgb(srgb, v))
            };
            if let Some(d) = &dump {
                let n = out_to_norm.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
                let asum: f32 = p.masks.iter().map(|m| m.alpha.data[i].max(0.0)).sum();
                let ev_in = (luminance_2020(tone_in).max(1e-12) / crate::tone::GREY).log2();
                let (mx, mn) = (tone_in[0].max(tone_in[1]).max(tone_in[2]), tone_in[0].min(tone_in[1]).min(tone_in[2]));
                let sat = if mx > 1e-12 { (mx - mn) / mx } else { 0.0 };
                d.put(i, [l0, hs_delta, local_delta, base, l1, wb, ev_in, asum, e[0], e[1], e[2], n.x as f32, n.y as f32, sat, l_exp, 0.0]);
            }
            let e = match &p_lut {
                Some((l, k)) if hdr => {
                    // the LUT sees SDR values; the part above SDR white is carried over
                    let c = e.map(|v| v.clamp(0.0, 1.0));
                    let m = l.apply(c);
                    [e[0] + (m[0] - c[0]) * k, e[1] + (m[1] - c[1]) * k, e[2] + (m[2] - c[2]) * k]
                }
                Some((l, k)) => {
                    let m = l.apply(e.map(|v| v.clamp(0.0, 1.0)));
                    [e[0] + (m[0] - e[0]) * k, e[1] + (m[1] - e[1]) * k, e[2] + (m[2] - e[2]) * k]
                }
                None => e,
            };
            *px = store(warn.unwrap_or(e));
        }
    });
    if let Some(d) = dump {
        let key = p.tone_stats.map_or(f32::NAN, |t| t.key + ev - crate::tone::lr::KEY_REF);
        d.write([fp.ow as f32, fp.oh as f32, ev, key, 0.0, 0.0]);
    }
    if let (Some(cd), Ok(path)) = (cla_dump, std::env::var("LIGHTCRAFT_CLARITY_DUMP")) {
        let v = cd.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes: Vec<u8> = [w as f32, h as f32].iter().chain(&v).flat_map(|f| f.to_le_bytes()).collect();
        let _ = std::fs::write(path, bytes);
    }
    out
}

/// A colour needing more desaturation than this (scale < `GAMUT_WARN`) to fit is out of gamut for
/// the gamut warnings (tolerates rounding at the gamut boundary).
const GAMUT_WARN: f32 = 0.995;

/// For the gamut warnings: `r` needed desaturating by `t` to fit, and isn't simply a neutral
/// beyond white or below black (that's clipping, which the clipping warnings show).
#[inline]
fn out_of_gamut(r: [f32; 3], t: f32) -> bool {
    let (lo, hi) = (r[0].min(r[1]).min(r[2]), r[0].max(r[1]).max(r[2]));
    t < GAMUT_WARN && (hi - lo) > 0.02 * hi.abs().max(1e-3) && hi - lo > 1e-3 && (lo < -1e-3 || (hi > 1.0 && lo < 1.0))
}

#[inline]
fn mul3(m: &[[f32; 3]; 3], d: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * d[0] + m[0][1] * d[1] + m[0][2] * d[2],
        m[1][0] * d[0] + m[1][1] * d[1] + m[1][2] * d[2],
        m[2][0] * d[0] + m[2][1] * d[1] + m[2][2] * d[2],
    ]
}

/// Desaturate linear `r` towards its luminance (weights `luma`) until every channel is in 0..1;
/// returns the mapped colour and the chroma scale used (1 = already in gamut).
#[inline]
pub fn gamut_map(r: [f32; 3], luma: [f32; 3]) -> ([f32; 3], f32) {
    gamut_map_to(r, luma, 1.0)
}

/// [`gamut_map`] into `0..peak` (an HDR render's range; `peak` = 1 is [`gamut_map`]).
#[inline]
pub fn gamut_map_to(r: [f32; 3], luma: [f32; 3], peak: f32) -> ([f32; 3], f32) {
    let yy = (luma[0] * r[0] + luma[1] * r[1] + luma[2] * r[2]).clamp(0.0, peak);
    let mut t = 1.0f32;
    for c in r {
        if c < 0.0 {
            t = t.min(yy / (yy - c).max(1e-9));
        } else if c > peak {
            t = t.min((peak - yy) / (c - yy).max(1e-9));
        }
    }
    if t < 1.0 { (r.map(|c| yy + (c - yy) * t), t) } else { (r, 1.0) }
}

/// Local Defringe weight of a scene-linear colour `c` whose log luminance differs by `det` from
/// its fine blur: high on strong edges with a purple or green cast.
#[inline]
pub fn defringe_weight(c: [f32; 3], det: f32) -> f32 {
    let y = luminance_2020(c).max(1e-6);
    let purple = (c[0].min(c[2]) - c[1]) / y;
    let green = (c[1] - c[0].max(c[2])) / y;
    smooth(0.04, 0.3, det.abs()) * smooth(0.02, 0.2, purple).max(smooth(0.02, 0.2, green))
}

/// Refine Saturation: scale the curved colour's chroma so its saturation (chroma / luma of the
/// encoded values) moves from the curve's towards the pre-curve one by the ratio
/// `(s_before / s_curve)^(1 - refine)` (1 keeps the curve, 0 about restores the original
/// saturation). The chroma is scaled in linear light around the colour's luminance, so the
/// curve's tone change stays whatever the colour.
#[inline]
pub fn refine_saturation(before: [f32; 3], after: [f32; 3], refine: f32) -> [f32; 3] {
    use lightcraft_color::transfer::srgb_to_linear;
    let luma = |e: [f32; 3]| STYLE_LUMA[0] * e[0] + STYLE_LUMA[1] * e[1] + STYLE_LUMA[2] * e[2];
    let chroma = |e: [f32; 3]| e[0].max(e[1]).max(e[2]) - e[0].min(e[1]).min(e[2]);
    let (y0, y1) = (luma(before), luma(after));
    let s0 = chroma(before) / y0.max(1e-4);
    let s1 = chroma(after) / y1.max(1e-4);
    if s1 <= 1e-6 || s0 <= 1e-6 {
        return after;
    }
    let k = (s0 / s1).powf(1.0 - refine).clamp(0.0, 4.0);
    if (k - 1.0).abs() < 1e-6 {
        return after;
    }
    let lin = after.map(|v| srgb_to_linear(v.clamp(0.0, 1.0)));
    let y = luma(lin);
    lin.map(|v| linear_to_srgb((y + (v - y) * k).max(0.0)))
}

pub const SRGB_LUT_N: usize = 4096;

/// Linear → sRGB-encoded table (`SRGB_LUT_N + 1` entries over 0..1), interpolated linearly.
pub fn srgb_lut() -> &'static [f32; SRGB_LUT_N + 1] {
    static LUT: std::sync::OnceLock<Box<[f32; SRGB_LUT_N + 1]>> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = Box::new([0.0f32; SRGB_LUT_N + 1]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = linear_to_srgb(i as f32 / SRGB_LUT_N as f32);
        }
        t
    })
}

/// Linear → sRGB-encoded (clamped to 0..1) by an interpolated table: within 2e-5 of the exact
/// curve (≪ one 8-bit or 10-bit step), several times faster than `powf`.
#[inline]
fn encode_srgb(lut: &[f32; SRGB_LUT_N + 1], v: f32) -> f32 {
    let f = v.clamp(0.0, 1.0) * SRGB_LUT_N as f32;
    let i = (f as usize).min(SRGB_LUT_N - 1);
    let t = f - i as f32;
    lut[i] + (lut[i + 1] - lut[i]) * t
}

#[inline]
fn enc(v: f32) -> u8 {
    // `v` is already sRGB-encoded; round to 8 bits.
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The camera profile's hue/sat map follows the white balance (Bryan's fork, `docs/lr-match.md` →
/// Temperature 2500 K). Camera Raw (and the DNG SDK's `dng_render`) read the map for the white of the
/// render: the profile's two calibration tables are weighted by the white's temperature, so a custom
/// Temp / Tint moves the map exactly as an as-shot white at the same place does. LightCraft registers
/// one base per photo with the map of the as-shot white; the engine registers, for that base, a
/// function from the render's Temp / Tint to the base carrying the map of that white
/// ([`register_white_bases`]) and the per-pixel stage swaps to it ([`for_render`]).
pub mod white {
    use std::collections::HashMap;
    use std::sync::{Arc, OnceLock, RwLock};

    use crate::SourceInfo;
    use lightcraft_develop::DevelopSettings;

    /// Temp / Tint → the key of the base with that white's hue/sat map (`None`: keep the photo's own).
    pub type WhiteBases = Box<dyn Fn(f64, f64) -> Option<u64> + Send + Sync>;

    fn whites() -> &'static RwLock<HashMap<u64, Arc<WhiteBases>>> {
        static R: OnceLock<RwLock<HashMap<u64, Arc<WhiteBases>>>> = OnceLock::new();
        R.get_or_init(Default::default)
    }

    /// Make the per-white bases of the photo base `key` available.
    pub fn register_white_bases(key: u64, f: WhiteBases) {
        whites().write().unwrap_or_else(|e| e.into_inner()).insert(key, Arc::new(f));
    }

    /// The base to render photo base `key` with at white `temp` / `tint` (`key` itself without a
    /// registered function, or when it has no answer).
    pub fn base_at_white(key: u64, temp: f64, tint: f64) -> u64 {
        if !(temp.is_finite() && tint.is_finite()) {
            return key;
        }
        let f = whites().read().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
        f.and_then(|f| f(temp.clamp(1500.0, 50000.0), tint.clamp(-150.0, 150.0))).unwrap_or(key)
    }

    /// The base for this render: the photo's own for its as-shot white, else the one with the map of
    /// the white balance in effect.
    pub fn for_render(key: u64, info: &SourceInfo, s: &DevelopSettings) -> u64 {
        if !info.raw {
            return key;
        }
        let (t, tint) = crate::local::effective_wb(info, s);
        if (t - info.as_shot_temp).abs() < 1e-6 && (tint - info.as_shot_tint).abs() < 1e-6 {
            return key;
        }
        base_at_white(key, t, tint)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn unregistered_or_hostile_whites_keep_the_photos_base() {
            assert_eq!(base_at_white(0xfeed_0001, 3000.0, 0.0), 0xfeed_0001);
            register_white_bases(0xfeed_0002, Box::new(|t, _| Some(t as u64)));
            assert_eq!(base_at_white(0xfeed_0002, 3000.0, 0.0), 3000);
            assert_eq!(base_at_white(0xfeed_0002, f64::NAN, 0.0), 0xfeed_0002);
            assert_eq!(base_at_white(0xfeed_0002, 1.0e9, 0.0), 50000);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_luma_is_the_style_spaces_luminance() {
        for (a, b) in STYLE_LUMA.iter().zip(STYLE_SPACE.luma()) {
            assert!((f64::from(*a) - b).abs() < 1e-5, "{STYLE_LUMA:?} vs {:?}", STYLE_SPACE.luma());
        }
    }

    #[test]
    fn the_master_curve_keeps_hue_and_the_channel_curves_act_alone() {
        use lightcraft_geom::Point;
        let pts = |v: &[f64]| v.chunks(2).map(|c| Point::new(c[0] / 255.0, c[1] / 255.0)).collect::<Vec<_>>();
        // Bryan's custom curve: Lightroom applies it hue-preserving (Camera Raw oracle)
        let c = ToneCurve { master: pts(&[0.0, 6.0, 62.0, 39.0, 98.0, 84.0, 181.0, 167.0, 255.0, 211.0]), ..ToneCurve::default() };
        let k = curve_luts(&c, None).expect("a curve");
        let n = k.apply([0.5, 0.5, 0.5]);
        assert!(n.iter().all(|v| (v - n[0]).abs() < 1e-6) && (n[0] - 0.466369).abs() < 2e-3, "{n:?}");
        let e = [0.7, 0.4, 0.2];
        let o = k.apply(e);
        let f = |x: f32| k.master.eval(x);
        assert!((o[0] - f(0.7)).abs() < 1e-6 && (o[2] - f(0.2)).abs() < 1e-6);
        // the middle channel keeps its place between the others (per channel it would not)
        let t = (o[1] - o[2]) / (o[0] - o[2]);
        assert!((t - 0.4).abs() < 1e-4, "{o:?}");
        // a red curve alone changes red only
        let r = ToneCurve { red: pts(&[0.0, 0.0, 128.0, 160.0, 255.0, 255.0]), ..ToneCurve::default() };
        let k = curve_luts(&r, None).expect("a curve");
        let o = k.apply(e);
        assert!(o[0] > 0.7 && (o[1] - 0.4).abs() < 1e-3 && (o[2] - 0.2).abs() < 1e-3, "{o:?}");
        assert!(curve_luts(&ToneCurve::default(), None).is_none());
    }

    #[test]
    fn refine_saturation_restores_the_pre_curve_saturation() {
        let before = [0.5, 0.3, 0.2];
        let after = [0.7, 0.35, 0.15]; // a contrasty curve: more saturated
        assert_eq!(refine_saturation(before, after, 1.0), after);
        let r = refine_saturation(before, after, 0.0);
        let y = |e: [f32; 3]| STYLE_LUMA[0] * e[0] + STYLE_LUMA[1] * e[1] + STYLE_LUMA[2] * e[2];
        let sat = |e: [f32; 3]| (e[0] - e[2]) / y(e);
        assert!((sat(r) - sat(before)).abs() < 0.15 * sat(before), "{r:?}");
        let half = refine_saturation(before, after, 0.5);
        assert!(sat(half) > sat(r) && sat(half) < sat(after));
        // luminance (linear light) is kept
        let lin = |e: [f32; 3]| e.map(lightcraft_color::transfer::srgb_to_linear);
        let (r, after) = (lin(r), lin(after));
        assert!((y(r) - y(after)).abs() < 1e-5);
    }

    #[test]
    fn sharpening_radius_is_in_source_pixels() {
        // full size: Radius 1 → about a pixel; a 1/8-size preview has nothing left to sharpen
        let full = Sharpen::new(1.0, 25.0, 6000.0, 6000.0);
        assert!(full.r >= 1 && full.r as usize <= SHARPEN_TAPS);
        let sum = full.w[0] + 2.0 * full.w[1..=full.r as usize].iter().sum::<f32>();
        assert!((sum - 1.0).abs() < 1e-5, "{sum}");
        assert_eq!(Sharpen::new(1.0, 25.0, 750.0, 6000.0).r, 0);
        assert!(Sharpen::new(3.0, 25.0, 6000.0, 6000.0).r as usize <= SHARPEN_TAPS, "kernel stays bounded");
        assert!(Sharpen::new(1.0, 100.0, 6000.0, 6000.0).t > full.t, "Detail lets bigger differences through");
        // a flat plane has nothing to sharpen; a step reads as a step
        let flat = vec![0.5f32; 64];
        let (b, grad) = full.sample(&flat, 8, 8, 3, 3, true);
        assert!((b - 0.5).abs() < 1e-6 && grad.abs() < 1e-6);
        let step: Vec<f32> = (0..64).map(|i| if i % 8 < 4 { 0.0 } else { 1.0 }).collect();
        let (b, grad) = full.sample(&step, 8, 8, 3, 3, true);
        assert!(b > 0.0 && b < 0.5 && grad > 0.2, "{b} {grad}");
        // a ramp of 0.1 EV per px reads as 0.1 EV per px
        let ramp: Vec<f32> = (0..256).map(|i| (i % 16) as f32 * 0.1).collect();
        assert!((full.sample(&ramp, 16, 16, 8, 8, true).1 - 0.1).abs() < 1e-4);
        // a half-size preview reads the same gradient per source pixel
        let half = Sharpen::new(1.0, 25.0, 3000.0, 6000.0);
        let ramp2: Vec<f32> = (0..256).map(|i| (i % 16) as f32 * 0.2).collect();
        assert!((half.sample(&ramp2, 16, 16, 8, 8, true).1 - 0.1).abs() < 1e-3, "{}", half.sample(&ramp2, 16, 16, 8, 8, true).1);
        // Edge Masking keeps edges, drops fine texture; 0 is no mask
        assert_eq!(Sharpen::mask(0.0, 0.0), 1.0);
        assert!(Sharpen::mask(0.94, 1.0) > 0.99 && Sharpen::mask(0.94, 0.0) < 0.05);
        assert!(Sharpen::mask(0.3, 0.1) > 0.99 && Sharpen::mask(f32::NAN, 0.1).is_finite());
        // hostile values: no panic, nothing applied
        for (r, ppl, n) in [(f64::NAN, 6000.0, 6000.0), (1.0, f64::INFINITY, 6000.0), (1.0, 6000.0, f32::NAN), (1.0, -5.0, 0.0)] {
            let k = Sharpen::new(r, f64::NAN, ppl, n);
            assert!(k.r as usize <= SHARPEN_TAPS && k.w.iter().all(|v| v.is_finite()) && k.t.is_finite());
        }
        assert_eq!(full.sample(&[], 0, 0, 0, 0, true).0, 0.0);
    }

    #[test]
    fn grain_is_finer_and_weaker_in_smaller_renders() {
        let (big, cells, _) = grain_params(31.0, 11.0, 43.0, 7, 6000.0, 6000.0);
        let (small, cells2, _) = grain_params(31.0, 11.0, 43.0, 7, 1000.0, 6000.0);
        assert_eq!(cells, cells2, "cells sit in the image, not on the output grid");
        assert!(small < big * 0.5 && small > 0.0, "{small} vs {big}");
        // coarser grain has fewer, weaker cells per pixel (Camera Raw's, measured)
        let (coarse, coarse_cells, _) = grain_params(31.0, 80.0, 43.0, 7, 6000.0, 6000.0);
        assert!(coarse_cells < cells && coarse < big);
        // Bryan's 31 / 11 / 43 on a 6000 px file: L* sd 3.9 at mid grey (Camera Raw 3.96)
        let sd = big * GRAIN_NOISE_SD * GRAIN_L_PER_E;
        assert!((sd - 3.96).abs() < 0.25, "{sd}");
        // the cells follow the frame: a 3000 px file has 1.6x fewer cells per long edge, not 2x
        let (_, half, _) = grain_params(31.0, 11.0, 43.0, 7, 3000.0, 3000.0);
        assert!((cells / half - 2f32.powf(1.0 - GRAIN_RES_EXP)).abs() < 0.01, "{cells} / {half}");
        assert!(grain_tone(0.44) == 1.0 && grain_tone(0.05) < 0.5 && grain_tone(0.95) < 1.0 && grain_tone(-1.0) == 0.0);
        for v in [f64::NAN, f64::INFINITY, -1e9] {
            let (a, c, _) = grain_params(v, v, v, 1, v, f32::NAN);
            assert!(a.is_finite() && c.is_finite());
        }
        assert_eq!(grain_tone(f32::NAN), 0.0);
    }

    #[test]
    fn srgb_table_matches_the_exact_curve() {
        let lut = srgb_lut();
        let mut worst = 0.0f32;
        for i in 0..=200_000 {
            // dense near black, where the curve bends most
            let v = (i as f32 / 200_000.0).powi(3);
            worst = worst.max((encode_srgb(lut, v) - linear_to_srgb(v)).abs());
        }
        assert!(worst < 2e-5, "{worst}");
        assert_eq!(encode_srgb(lut, -1.0), 0.0);
        assert!((encode_srgb(lut, 2.0) - 1.0).abs() < 1e-6);
    }
}
