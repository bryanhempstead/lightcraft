//! Black & White mix as Camera Raw renders it (R22, Bryan's fork; `docs/lr-match.md` → Round 10).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/black_white.rs`, the grid lookup of
//! `color_mixer.rs`, and the measured table `black_white_chart.bin`, here `bw_cr.bin`;
//! <https://github.com/pch/rawmakase>, MIT, Copyright (c) 2026 RAWmakase contributors; see NOTICE).
//! In the colour mixer's grid (36 hues × 6 saturations × 6 values of linear ProPhoto HSV, the
//! saturation and value axes warped by `√s` and `v^0.45`): log2 of the grey's luminance over the
//! colour's at a zero mix, then each band's change at −100, −50, +50 and +100 (linear between
//! them and towards no change at 0). A colour turns into the grey `Y · 2^Σ` of its luminance.

/// Use this model (and not the older OkLCh band mix) for B&W.
pub const MEASURED: bool = true;

const HUES: usize = 36;
const SATS: usize = 6;
const VALS: usize = 6;
/// Cells of the grid.
pub const CELLS: usize = HUES * SATS * VALS;
/// Tables in the chart: the zero mix, then 8 bands × 4 positions.
#[cfg(test)]
const TABLES: usize = 1 + 8 * 4;
const POSITIONS: [f32; 4] = [-1., -0.5, 0.5, 1.];
const SCALE: f32 = 1. / 4000.;

static CHART: &[u8] = include_bytes!("bw_cr.bin");

fn chart(table: usize, cell: usize) -> f32 {
    let i = 2 * (table * CELLS + cell);
    match CHART.get(i..i + 2) {
        Some(b) => i16::from_le_bytes([b[0], b[1]]) as f32 * SCALE,
        None => 0.0,
    }
}

/// The mix (8 bands, −1..1) resolved to one log2 factor per grid cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    pub grid: Vec<f32>,
}

impl Mix {
    pub fn new(mix: [f32; 8]) -> Mix {
        let mut weights: Vec<(usize, f32)> = vec![(0, 1.)];
        for (band, &v) in mix.iter().enumerate() {
            let v = if v.is_finite() { v.clamp(-1., 1.) } else { 0.0 };
            if v == 0. {
                continue;
            }
            // linear between the measured positions, and towards no change at 0
            let (near, far) = if v < 0. { (1, 0) } else { (2, 3) };
            let table = |position: usize| 1 + band * 4 + position;
            let half = POSITIONS[near].abs();
            if v.abs() <= half {
                weights.push((table(near), v.abs() / half));
            } else {
                let t = (v.abs() - half) / (1. - half);
                weights.push((table(near), 1. - t));
                weights.push((table(far), t));
            }
        }
        Mix { grid: (0..CELLS).map(|cell| weights.iter().map(|(t, w)| w * chart(*t, cell)).sum()).collect() }
    }

    /// The grey (linear luminance) linear ProPhoto `p` with luminance `y` turns into.
    #[inline]
    pub fn gray(&self, p: [f32; 3], y: f32) -> f32 {
        let q = p.map(|v| v.max(0.0));
        let Some([h, s, v]) = hsv(q) else { return y.max(0.0) };
        let g = y.max(0.0) * lookup(&self.grid, h, s, v).exp2();
        if g.is_finite() { g } else { 0.0 }
    }
}

/// Hue (turns), saturation and value of linear ProPhoto RGB, or `None` for black.
#[inline]
fn hsv(p: [f32; 3]) -> Option<[f32; 3]> {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    if max.is_nan() || max <= 1e-6 {
        return None;
    }
    let d = max - min;
    let h = if d < 1e-9 {
        0.
    } else if max == p[0] {
        ((p[1] - p[2]) / d).rem_euclid(6.) / 6.
    } else if max == p[1] {
        ((p[2] - p[0]) / d + 2.) / 6.
    } else {
        ((p[0] - p[1]) / d + 4.) / 6.
    };
    Some([h, d / max, max])
}

/// Trilinear lookup of the grid between cell centres; hue wraps.
#[inline]
fn lookup(grid: &[f32], h: f32, s: f32, v: f32) -> f32 {
    let fh = h.rem_euclid(1.) * HUES as f32 - 0.5;
    let fs = (s.clamp(0., 1.).sqrt() * SATS as f32 - 0.5).clamp(0., (SATS - 1) as f32);
    let fv = (v.clamp(0., 1.).powf(0.45) * VALS as f32 - 0.5).clamp(0., (VALS - 1) as f32);
    if !(fh.is_finite() && fs.is_finite() && fv.is_finite()) {
        return 0.0;
    }
    let h0 = fh.floor();
    let (th, ts, tv) = (fh - h0, fs.fract(), fv.fract());
    let hi = |d: isize| (h0 as isize + d).rem_euclid(HUES as isize) as usize;
    let (s0, v0) = (fs as usize, fv as usize);
    let (s1, v1) = ((s0 + 1).min(SATS - 1), (v0 + 1).min(VALS - 1));
    let mut out = 0.0;
    for (dh, wh) in [(0, 1. - th), (1, th)] {
        for (si, ws) in [(s0, 1. - ts), (s1, ts)] {
            for (vi, wv) in [(v0, 1. - tv), (v1, tv)] {
                out += grid.get((hi(dh) * SATS + si) * VALS + vi).copied().unwrap_or(0.0) * wh * ws * wv;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_has_the_expected_size_and_neutrals_keep_their_luminance() {
        assert_eq!(CHART.len(), TABLES * CELLS * 2);
        let m = Mix::new([0.6; 8]);
        // a neutral has no hue: the mix barely moves it
        assert!((m.gray([0.2; 3], 0.2) / 0.2 - 1.0).abs() < 0.05);
        assert_eq!(m.gray([0.0; 3], 0.0), 0.0);
        assert!(m.gray([f32::NAN, 0.1, 0.2], 0.1).is_finite());
    }

    /// Camera Raw (our oracle, `make_fx.py bw`): Blue −60 darkens a blue far more than Blue +60
    /// lightens it, and leaves an orange alone.
    #[test]
    fn blue_band_darkens_blues_only() {
        let blue = [0.05, 0.08, 0.5];
        let orange = [0.5, 0.2, 0.04];
        let y = |p: [f32; 3]| 0.2880402 * p[0] + 0.7118741 * p[1] + 0.0000857 * p[2];
        let mut mix = [0.0; 8];
        let zero = Mix::new(mix);
        mix[5] = -0.6;
        let dark = Mix::new(mix);
        mix[5] = 0.6;
        let light = Mix::new(mix);
        let (b0, bd, bl) = (zero.gray(blue, y(blue)), dark.gray(blue, y(blue)), light.gray(blue, y(blue)));
        assert!(bd < b0 * 0.8 && bl > b0, "{b0} {bd} {bl}");
        assert!((dark.gray(orange, y(orange)) / zero.gray(orange, y(orange)) - 1.0).abs() < 0.02);
    }
}
