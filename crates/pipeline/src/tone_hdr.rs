//! The HDR tone of an HDR edit: an SDR tone table extended into the headroom as Camera Raw does
//! (Bryan's fork, `docs/lr-match.md` → HDR tone; the measured tables are [`crate::tone_hdr_cr`]).
//! Used by [`crate::tone::ToneMap::with_hdr`] for every tone path (CPU and GPU share the table).

use crate::tone_hdr_cr as t;

/// A measured table at `d` (EV past the SDR tone's half-white point): linear between nodes, held
/// before the first, continued at the last two nodes' slope after the last (`extend`).
fn table_at(tab: &[f32], d: f32, extend: bool) -> f32 {
    let n = tab.len();
    if n < 2 || !d.is_finite() {
        return 0.0;
    }
    let f = (d - t::D0) / t::STEP;
    let last = (n - 1) as f32;
    if f >= last {
        let (a, b) = (tab.get(n - 2).copied().unwrap_or(0.0), tab.get(n - 1).copied().unwrap_or(0.0));
        return if extend { b + (b - a) * (f - last) } else { b };
    }
    let f = f.max(0.0);
    let i = (f as usize).min(n - 2);
    let k = f - i as f32;
    let (a, b) = (tab.get(i).copied().unwrap_or(0.0), tab.get(i + 1).copied().unwrap_or(0.0));
    a + (b - a) * k
}

/// A slider's delta at value `v`: the tables at the measured (sorted) `values`, 0 at 0, linear in
/// between, held beyond the ends.
fn slider_at<const V: usize>(values: &[f32; V], tables: &[[f32; t::N]; V], v: f32, d: f32) -> f32 {
    if !v.is_finite() || v == 0.0 {
        return 0.0;
    }
    let mut pts = [(0.0f32, 0.0f32); 16];
    let mut n = 0;
    let mut zero = false;
    for (x, tab) in values.iter().zip(tables) {
        if !zero && *x > 0.0 && n < pts.len() {
            pts[n] = (0.0, 0.0);
            n += 1;
            zero = true;
        }
        if n < pts.len() {
            pts[n] = (*x, table_at(tab, d, false));
            n += 1;
        }
    }
    if !zero && n < pts.len() {
        pts[n] = (0.0, 0.0);
        n += 1;
    }
    let pts = &pts[..n];
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return 0.0 };
    let v = v.clamp(first.0, last.0);
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if v >= x0 && v <= x1 {
            return if x1 > x0 { y0 + (y1 - y0) * (v - x0) / (x1 - x0) } else { y0 };
        }
    }
    0.0
}

/// The edit's Basic-tone sliders (−100..100) that change Camera Raw's HDR tone beyond what they do to
/// the SDR tone (measured: Highlights and Blacks don't, beyond the SDR tone's own change).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sliders {
    pub whites: f32,
    pub contrast: f32,
}

/// Camera Raw's extra stops over the SDR tone at `d` EV past the SDR tone's half-white point.
pub fn gain_ev(d: f32, s: Sliders) -> f32 {
    let g = table_at(&t::G0, d, true) + slider_at(&t::W_VALUES, &t::W_TABLE, s.whites, d) + slider_at(&t::C_VALUES, &t::C_TABLE, s.contrast, d);
    // a slider can darken the HDR tone below the SDR one's shadows (negative Contrast lifts them)
    if g.is_finite() { g.clamp(-4.0, 16.0) } else { 0.0 }
}

/// The HDR tone table for the SDR tone table `sdr` (entry `i` at scene EV `ev_at(i)`), reaching at
/// most `peak` (linear, SDR white = 1; Camera Raw cuts the curve there, it doesn't bend it), for
/// the edit's sliders `s`. Monotone wherever `sdr` is; `sdr` itself when `peak` is 1 or less or
/// not finite.
pub fn extend(sdr: &[f32], ev_at: impl Fn(usize) -> f32, peak: f32, s: Sliders) -> Vec<f32> {
    if !(peak.is_finite() && peak > 1.0) {
        return sdr.to_vec();
    }
    // where the SDR tone reaches half of white (a tone that never does: its top)
    let half = sdr.iter().position(|v| *v >= 0.5).unwrap_or(sdr.len().saturating_sub(1));
    let e50 = ev_at(half);
    let mut prev = 0.0f32;
    sdr.iter()
        .enumerate()
        .map(|(i, v0)| {
            let g = gain_ev(ev_at(i) - e50, s);
            let v = if v0.is_finite() { v0.max(0.0) * g.exp2() } else { 0.0 };
            // never below the previous entry (monotone), never above the limit
            let v = v.max(prev).clamp(0.0, peak);
            prev = v;
            v
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev_at(i: usize) -> f32 {
        -14.0 + 24.0 * i as f32 / 4095.0
    }

    fn ramp() -> (Vec<f32>, fn(usize) -> f32) {
        // an SDR-like tone: grey 0.18 → 0.38, white near 3 EV over grey, clipped at 1
        let lut = (0..4096).map(|i| (0.38 * ev_at(i).exp2() / (1.0 + 0.15 * ev_at(i).exp2())).min(1.0)).collect();
        (lut, ev_at)
    }

    #[test]
    fn keeps_midtones_cuts_at_the_limit_and_is_monotone() {
        let (sdr, ev_at) = ramp();
        assert_eq!(extend(&sdr, ev_at, 1.0, Sliders::default()), sdr);
        assert_eq!(extend(&sdr, ev_at, f32::NAN, Sliders::default()), sdr);
        for peak in [2.0f32, 16.0, 32.0] {
            let h = extend(&sdr, ev_at, peak, Sliders::default());
            assert!(h.windows(2).all(|w| w[1] >= w[0]));
            for (a, b) in h.iter().zip(&sdr) {
                assert!(*a >= *b && *a <= peak);
                if *b < 0.4 {
                    assert!((a - b).abs() < 1e-3, "midtones moved: {b} -> {a}");
                }
            }
            assert_eq!(h.last().copied(), Some(peak), "reaches the limit");
        }
    }

    #[test]
    fn the_curve_does_not_depend_on_the_limit_below_it() {
        let (sdr, ev_at) = ramp();
        let (h2, h4) = (extend(&sdr, ev_at, 4.0, Sliders::default()), extend(&sdr, ev_at, 16.0, Sliders::default()));
        for (a, b) in h2.iter().zip(&h4) {
            assert!(*b >= 4.0 || (a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn whites_open_the_headroom_faster() {
        let (sdr, ev_at) = ramp();
        let w = |whites: f32| extend(&sdr, ev_at, 16.0, Sliders { whites, ..Sliders::default() });
        let (lo, hi) = (w(-50.0), w(100.0));
        let first = |h: &[f32], level: f32| h.iter().position(|v| *v >= level);
        assert!(first(&hi, 8.0) < first(&lo, 8.0));
    }

    #[test]
    fn hostile_input_never_panics() {
        let ev_at = |i: usize| i as f32;
        assert!(extend(&[], ev_at, 16.0, Sliders::default()).is_empty());
        let wild = Sliders { whites: f32::NAN, contrast: f32::INFINITY };
        let h = extend(&[f32::NAN, f32::INFINITY, -1.0, 0.7], ev_at, 16.0, wild);
        assert!(h.iter().all(|v| v.is_finite() && (0.0..=16.0).contains(v)));
        assert!(gain_ev(f32::NAN, Sliders::default()) == 0.0 && gain_ev(1e30, wild).is_finite());
    }
}
