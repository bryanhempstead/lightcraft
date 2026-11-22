//! Color Grading as Camera Raw renders it (R20, Bryan's fork; `docs/lr-match.md` → Round 10).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/color_grade_curves.rs` and its
//! measured table `color_grade_curves.bin`, here `grading_cr.bin`; <https://github.com/pch/rawmakase>,
//! MIT, Copyright (c) 2026 RAWmakase contributors; see NOTICE), measured on Camera Raw 18.7's
//! renders of a synthetic chart for every hue, saturation, Luminance, Blending and Balance.
//! Our own oracle agrees on the structure (`oracle/make_fx.py grade`): Camera Raw grades each
//! channel of linear ProPhoto RGB with its own curve — a channel's output depends on that
//! channel's input only (Global at hue 210° / saturation 100 leaves green exactly as it was).
//! A region's tint gains are `1 + Σ A_k(x)·p_k`, with profiles `A` over the channel value that
//! Blending and Balance shape and per-channel coefficients `p` that hue and saturation set;
//! regions multiply. Luminance is one curve for all channels, applied before the tint. The
//! settings collapse to three gain curves over `E = x^(1/2.2)`, 64 samples each
//! ([`Curves::gain`]), applied per pixel with one lookup per channel.

use lightcraft_develop::{ColorGrading as Grading, DevelopSettings};

/// Samples of each curve over E = x^(1/2.2), 0–1.
pub const SAMPLES: usize = 64;
const BLENDS: [f32; 5] = [0., 0.25, 0.5, 0.75, 1.];
const BALANCES: [f32; 17] = [-1., -0.88, -0.75, -0.62, -0.5, -0.38, -0.25, -0.12, 0., 0.12, 0.25, 0.38, 0.5, 0.62, 0.75, 0.88, 1.];
const HUES: usize = 12;
const SATURATIONS: [f32; 5] = [0., 0.25, 0.5, 0.75, 1.];
/// Luminance slider positions, 0 being no change.
const LUMINANCES: [f32; 9] = [-1., -0.75, -0.5, -0.25, 0., 0.25, 0.5, 0.75, 1.];
/// Profile terms per region; Global's third is used by Global only.
const TERMS: usize = 3;
const SHAPED_TERMS: usize = 2;

static DATA: &[u8] = include_bytes!("grading_cr.bin");
// Offsets in f32 values, in the order RAWmakase's script writes the tables.
const GRID: usize = 0;
const GRID_LEN: usize = 3 * BLENDS.len() * BALANCES.len() * SAMPLES * SHAPED_TERMS;
const GLOBAL: usize = GRID + GRID_LEN;
const COEF: usize = GLOBAL + SAMPLES * TERMS;
const COEF_LEN: usize = 4 * HUES * (SATURATIONS.len() - 1) * 3 * TERMS;
const LUM: usize = COEF + COEF_LEN;
/// Length of the table in f32 values.
pub const LEN: usize = LUM + 4 * (LUMINANCES.len() - 1) * SAMPLES;

/// Use this model (and not the older OkLab wheels) for Color Grading.
pub const MEASURED: bool = true;

fn value(i: usize) -> f32 {
    let at = 4 * i;
    match DATA.get(at..at + 4) {
        Some(b) => {
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if v.is_finite() { v } else { 0.0 }
        }
        None => 0.0,
    }
}

/// Index and weight of `v` between the entries of `values` (sorted, ≥ 2 entries).
fn bracket(values: &[f32], v: f32) -> (usize, f32) {
    let n = values.len();
    if n < 2 {
        return (0, 0.0);
    }
    let v = if v.is_finite() { v.clamp(values[0], values[n - 1]) } else { values[0] };
    let j = values.windows(2).position(|w| v <= w[1]).unwrap_or(n - 2);
    let span = values[j + 1] - values[j];
    (j, if span > 0.0 { (v - values[j]) / span } else { 0.0 })
}

/// Shadows, Midtones, Highlights and Global.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Region {
    Shadows,
    Midtones,
    Highlights,
    Global,
}
const REGIONS: [Region; 4] = [Region::Shadows, Region::Midtones, Region::Highlights, Region::Global];

type Profiles = [[f32; TERMS]; SAMPLES];

fn profiles(region: Region, blending: f32, balance: f32) -> Profiles {
    let mut out = [[0.; TERMS]; SAMPLES];
    if region == Region::Global {
        for (k, p) in out.iter_mut().enumerate() {
            *p = std::array::from_fn(|t| value(GLOBAL + k * TERMS + t));
        }
        return out;
    }
    let (i, u) = bracket(&BLENDS, blending);
    let (j, v) = bracket(&BALANCES, balance);
    let at = |b: usize, a: usize, k: usize, t: usize| {
        let cell = ((region as usize * BLENDS.len() + b) * BALANCES.len() + a) * SAMPLES + k;
        value(GRID + cell * SHAPED_TERMS + t)
    };
    for (k, p) in out.iter_mut().enumerate() {
        for (t, slot) in p.iter_mut().enumerate().take(SHAPED_TERMS) {
            *slot = at(i, j, k, t) * (1. - u) * (1. - v)
                + at(i + 1, j, k, t) * u * (1. - v)
                + at(i, j + 1, k, t) * (1. - u) * v
                + at(i + 1, j + 1, k, t) * u * v;
        }
    }
    out
}

/// The region's per-channel coefficients for `hue` (0–1, a turn) and `saturation` (0–1).
fn coefficients(region: Region, hue: f32, saturation: f32) -> [[f32; TERMS]; 3] {
    let hue = if hue.is_finite() { hue } else { 0.0 };
    let f = hue.rem_euclid(1.) * HUES as f32;
    let (h0, w) = ((f as usize) % HUES, f.fract());
    let (s, u) = bracket(&SATURATIONS, saturation);
    // Saturation 0 has no tint; the table starts at 25.
    let at = |h: usize, s: usize, c: usize, t: usize| match s {
        0 => 0.,
        s => value(COEF + (((region as usize * HUES + h) * (SATURATIONS.len() - 1) + s - 1) * 3 + c) * TERMS + t),
    };
    std::array::from_fn(|c| {
        std::array::from_fn(|t| {
            let hue_at = |s: usize| at(h0, s, c, t) * (1. - w) + at((h0 + 1) % HUES, s, c, t) * w;
            hue_at(s) * (1. - u) + hue_at((s + 1).min(SATURATIONS.len() - 1)) * u
        })
    })
}

/// log2 of the measured Luminance gains at `amount` (−1 to 1), Blending 50 and Balance 0.
fn luminance_log(region: Region, amount: f32) -> [f32; SAMPLES] {
    let (j, u) = bracket(&LUMINANCES, amount);
    let table = |i: usize| -> [f32; SAMPLES] {
        match i.cmp(&4) {
            std::cmp::Ordering::Equal => [0.; SAMPLES],
            order => {
                let row = if order == std::cmp::Ordering::Less { i } else { i - 1 };
                let at = LUM + (region as usize * (LUMINANCES.len() - 1) + row) * SAMPLES;
                std::array::from_fn(|k| value(at + k).max(1e-6).log2())
            }
        }
    };
    let (a, b) = (table(j), table((j + 1).min(LUMINANCES.len() - 1)));
    std::array::from_fn(|k| a[k] * (1. - u) + b[k] * u)
}

/// The region's first profile, scaled to peak at 1: where it sits along the channel.
fn weight(region: Region, blending: f32, balance: f32) -> [f32; SAMPLES] {
    let p = profiles(region, blending, balance);
    let peak = p.iter().map(|t| t[0]).fold(0f32, |m, v| if v.abs() > m.abs() { v } else { m });
    std::array::from_fn(|k| if peak == 0. { 0. } else { p[k][0] / peak })
}

fn argmax(v: &[f32]) -> usize {
    (0..v.len()).fold(0, |m, i| if v[i] > v[m] { i } else { m })
}

/// Linear interpolation of `table` at fractional sample `at`.
fn sample(table: &[f32; SAMPLES], at: f32) -> f32 {
    let at = if at.is_finite() { at.clamp(0., (SAMPLES - 1) as f32) } else { 0.0 };
    let i = (at as usize).min(SAMPLES - 2);
    let t = at - i as f32;
    table[i] * (1. - t) + table[i + 1] * t
}

fn encoded(k: usize) -> f32 {
    k as f32 / (SAMPLES - 1) as f32
}

/// The Luminance gains at `amount`, Blending and Balance: Luminance keeps the strength it has at
/// the default where the region's weight is the same, on the same side of the weight's peak.
/// Darkening keeps its gain there, lifts their encoded offset.
fn luminance(region: Region, amount: f32, blending: f32, balance: f32) -> [f32; SAMPLES] {
    let log = luminance_log(region, amount);
    let default = (blending - 0.5).abs() < 1e-4 && balance.abs() < 1e-4;
    if region == Region::Global || default {
        return log.map(f32::exp2);
    }
    let (w, w0) = (weight(region, blending, balance), weight(region, 0.5, 0.));
    let (peak, peak0) = (argmax(&w), argmax(&w0));
    let at: [f32; SAMPLES] = std::array::from_fn(|k| {
        let seg: Vec<usize> = if k <= peak { (0..peak0 + 1).collect() } else { (peak0..SAMPLES).collect() };
        let near = (0..seg.len()).min_by(|&a, &b| (w0[seg[a]] - w[k]).abs().total_cmp(&(w0[seg[b]] - w[k]).abs())).unwrap_or(0);
        for (a, b) in [(near.wrapping_sub(1), near), (near, near + 1)] {
            if a < seg.len() && b < seg.len() {
                let (lo, hi) = (w0[seg[a]], w0[seg[b]]);
                if (lo - w[k]) * (hi - w[k]) <= 0. && lo != hi {
                    let t = (w[k] - lo) / (hi - lo);
                    return seg[a] as f32 + t * (seg[b] as f32 - seg[a] as f32);
                }
            }
        }
        seg.get(near).copied().unwrap_or(k) as f32
    });
    if amount < 0. {
        return std::array::from_fn(|k| sample(&log, at[k]).exp2());
    }
    let offset: [f32; SAMPLES] = std::array::from_fn(|k| encoded(k) * ((log[k] / 2.2).exp2() - 1.));
    std::array::from_fn(|k| match k {
        0 => log[0].exp2(),
        k => ((encoded(k) + sample(&offset, at[k])) / encoded(k)).max(0.0).powf(2.2),
    })
}

/// Per-channel gains of linear ProPhoto RGB at [`SAMPLES`] points of x^(1/2.2).
pub type Gains = [[f32; 3]; SAMPLES];

/// The grading curves for Shadows, Midtones, Highlights and Global (hue 0–1, saturation 0–1,
/// luminance −1–1), or `None` when they change nothing.
fn gains(settings: [[f32; 3]; 4], blending: f32, balance: f32) -> Option<Gains> {
    if settings.iter().all(|g| g[1] == 0. && g[2] == 0.) {
        return None;
    }
    let mut lum = [1f32; SAMPLES];
    let mut tint = [[1f32; 3]; SAMPLES];
    for (region, [hue, saturation, amount]) in REGIONS.into_iter().zip(settings) {
        if amount != 0. {
            let l = luminance(region, amount, blending, balance);
            lum.iter_mut().zip(l).for_each(|(a, b)| *a *= b);
        }
        if saturation != 0. {
            let p = profiles(region, blending, balance);
            let c = coefficients(region, hue, saturation);
            for (g, a) in tint.iter_mut().zip(p) {
                for (ch, coef) in g.iter_mut().zip(c) {
                    *ch *= 1. + a.iter().zip(coef).map(|(a, c)| a * c).sum::<f32>();
                }
            }
        }
    }
    // Luminance first, then the tint on its result.
    let tint: [[f32; SAMPLES]; 3] = std::array::from_fn(|c| std::array::from_fn(|i| tint[i][c]));
    Some(std::array::from_fn(|k| {
        let after = (encoded(k).powf(2.2) * lum[k]).clamp(0., 1.).powf(1. / 2.2);
        std::array::from_fn(|c| {
            let g = lum[k] * sample(&tint[c], after * (SAMPLES - 1) as f32).max(0.);
            if g.is_finite() { g } else { 1.0 }
        })
    }))
}

/// A grading's three channel curves (linear ProPhoto).
#[derive(Clone, Debug, PartialEq)]
pub struct Curves {
    pub gain: Gains,
}

impl Curves {
    /// The curves of `g`, or `None` when it changes nothing.
    pub fn new(g: &Grading) -> Option<Curves> {
        let w = |w: &lightcraft_develop::Wheel| [(w.hue / 360.0) as f32, (w.sat / 100.0) as f32, (w.lum / 100.0) as f32];
        let settings = [w(&g.shadows), w(&g.midtones), w(&g.highlights), w(&g.global)];
        gains(settings, (g.blending / 100.0) as f32, (g.balance / 100.0) as f32).map(|gain| Curves { gain })
    }

    /// The curves for `s` when this model renders its grading.
    pub fn for_settings(s: &DevelopSettings) -> Option<Curves> {
        if !MEASURED || s.grading.is_neutral() { None } else { Curves::new(&s.grading) }
    }

    /// Channel `c` (0..3) of linear ProPhoto through its curve.
    #[inline]
    pub fn channel(&self, c: usize, v: f32) -> f32 {
        let at = v.clamp(0., 1.).powf(1. / 2.2) * (SAMPLES - 1) as f32;
        let at = if at.is_finite() { at } else { 0.0 };
        let i = (at as usize).min(SAMPLES - 2);
        let t = at - i as f32;
        let c = c.min(2);
        v * (self.gain[i][c] * (1. - t) + self.gain[i + 1][c] * t)
    }

    /// Linear ProPhoto `p` graded.
    #[inline]
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        [self.channel(0, p[0]), self.channel(1, p[1]), self.channel(2, p[2])]
    }

    /// The curves flattened for the GPU: sample-major, three channels each.
    pub fn words(&self) -> Vec<f32> {
        self.gain.iter().flat_map(|g| g.iter().copied()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::Wheel;

    fn graded(g: &Grading, p: [f32; 3]) -> [f32; 3] {
        Curves::new(g).unwrap().apply(p)
    }

    #[test]
    fn table_has_the_expected_size() {
        assert_eq!(DATA.len(), LEN * 4);
    }

    #[test]
    fn neutral_grading_has_no_curves() {
        assert!(Curves::new(&Grading::default()).is_none());
    }

    /// Our Camera Raw oracle (`make_fx.py grade`, grid neutrals): Global at 210° / 100 leaves
    /// green as it is and scales red ×0.565, blue ×1.774 in the shadows, fading towards white.
    #[test]
    fn global_wheel_follows_camera_raw() {
        let g = Grading { global: Wheel { hue: 210.0, sat: 100.0, lum: 0.0 }, ..Default::default() };
        let dark = graded(&g, [0.0063; 3]).map(|v| v / 0.0063);
        assert!((dark[1] - 1.0).abs() < 0.03, "{dark:?}");
        assert!((dark[0] - 0.565).abs() < 0.06 && (dark[2] - 1.774).abs() < 0.15, "{dark:?}");
        let bright = graded(&g, [0.82; 3]).map(|v| v / 0.82);
        assert!(bright[2] < dark[2] && bright[0] > dark[0], "{bright:?}");
    }

    #[test]
    fn channels_are_graded_independently() {
        let g = Grading { shadows: Wheel { hue: 30.0, sat: 50.0, lum: 0.0 }, ..Default::default() };
        let a = graded(&g, [0.02, 0.3, 0.6]);
        let b = graded(&g, [0.02, 0.05, 0.9]);
        assert_eq!(a[0], b[0]);
        assert!(a[0] > 0.02 * 1.2, "{a:?}");
        // hostile values stay finite
        let w = Grading { shadows: Wheel { hue: f64::NAN, sat: 1e9, lum: -1e9 }, blending: f64::INFINITY, ..Default::default() };
        if let Some(c) = Curves::new(&w) {
            assert!(c.apply([0.2, f32::NAN, 2.0]).iter().take(1).all(|v| v.is_finite()));
        }
    }
}
