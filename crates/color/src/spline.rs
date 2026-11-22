//! Curves for tone mapping: monotone cubic Hermite splines (Fritsch–Carlson), the natural cubic
//! spline Lightroom's point curve uses ([`NaturalCurve`]), and lookup tables.
//!
//! Monotone interpolation never overshoots between control points, so a curve can't invert tones
//! by accident; Lightroom's point curve does overshoot (and clamps), so its curves use
//! [`NaturalCurve`] to render as they do in Lightroom.

use serde::{Deserialize, Serialize};

/// A curve through control points in `0..1 × 0..1` (x strictly increasing after normalization).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonotoneCurve {
    xs: Vec<f64>,
    ys: Vec<f64>,
    ms: Vec<f64>,
}

impl MonotoneCurve {
    /// Build from points; sorts by x, merges duplicates. Fewer than two points → identity.
    pub fn new(points: &[(f64, f64)]) -> MonotoneCurve {
        let mut p: Vec<(f64, f64)> = points.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
        p.sort_by(|a, b| a.0.total_cmp(&b.0));
        p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
        if p.len() < 2 {
            p = vec![(0.0, 0.0), (1.0, 1.0)];
        }
        let n = p.len();
        let xs: Vec<f64> = p.iter().map(|q| q.0).collect();
        let ys: Vec<f64> = p.iter().map(|q| q.1).collect();
        let d: Vec<f64> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i])).collect();
        let mut ms = vec![0.0; n];
        ms[0] = d[0];
        ms[n - 1] = d[n - 2];
        for i in 1..n - 1 {
            ms[i] = if d[i - 1] * d[i] <= 0.0 { 0.0 } else { (d[i - 1] + d[i]) / 2.0 };
        }
        // Fritsch–Carlson limiter where the data is monotone within a segment.
        for i in 0..n - 1 {
            if d[i] == 0.0 {
                ms[i] = 0.0;
                ms[i + 1] = 0.0;
                continue;
            }
            let a = ms[i] / d[i];
            let b = ms[i + 1] / d[i];
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                ms[i] = t * a * d[i];
                ms[i + 1] = t * b * d[i];
            }
        }
        MonotoneCurve { xs, ys, ms }
    }

    pub fn identity() -> MonotoneCurve {
        MonotoneCurve::new(&[(0.0, 0.0), (1.0, 1.0)])
    }

    pub fn eval(&self, x: f64) -> f64 {
        let n = self.xs.len();
        if x <= self.xs[0] {
            return self.ys[0] + self.ms[0] * (x - self.xs[0]).min(0.0) * 0.0;
        }
        if x >= self.xs[n - 1] {
            return self.ys[n - 1];
        }
        let i = match self.xs.binary_search_by(|v| v.total_cmp(&x)) {
            Ok(i) => return self.ys[i],
            Err(i) => i - 1,
        };
        let h = self.xs[i + 1] - self.xs[i];
        let t = (x - self.xs[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
            + (t3 - 2.0 * t2 + t) * h * self.ms[i]
            + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
            + (t3 - t2) * h * self.ms[i + 1]
    }

    /// Sample into a LUT of `n` entries over `0..1`.
    pub fn to_lut(&self, n: usize) -> Lut1 {
        Lut1 { v: (0..n).map(|i| self.eval(i as f64 / (n - 1) as f64) as f32).collect() }
    }

    pub fn points(&self) -> Vec<(f64, f64)> {
        self.xs.iter().copied().zip(self.ys.iter().copied()).collect()
    }
}

/// Lightroom's / Camera Raw's point-curve interpolation: the natural cubic spline through the
/// points (C2 continuous, zero second derivative at both ends), flat outside them and clamped to
/// `0..1`. Measured with the Camera Raw oracle (`docs/lr-match.md` → Round 7): on Bryan's curve it
/// reproduces Camera Raw's to 0.01 of a level (the monotone spline: 1.3, up to 2.9); unlike
/// [`MonotoneCurve`] it overshoots between close points, as Lightroom's does.
#[derive(Clone, Debug, PartialEq)]
pub struct NaturalCurve {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Slope at each point.
    ms: Vec<f64>,
}

impl NaturalCurve {
    /// Build from points (sorted by x, duplicates merged); fewer than two points → identity.
    pub fn new(points: &[(f64, f64)]) -> NaturalCurve {
        let mut p: Vec<(f64, f64)> = points.iter().copied().filter(|(x, y)| x.is_finite() && y.is_finite()).collect();
        p.sort_by(|a, b| a.0.total_cmp(&b.0));
        p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9);
        if p.len() < 2 {
            p = vec![(0.0, 0.0), (1.0, 1.0)];
        }
        let n = p.len();
        let xs: Vec<f64> = p.iter().map(|q| q.0).collect();
        let ys: Vec<f64> = p.iter().map(|q| q.1).collect();
        let h: Vec<f64> = xs.windows(2).map(|w| w[1] - w[0]).collect();
        let d: Vec<f64> = (0..n - 1).map(|i| (ys[i + 1] - ys[i]) / h[i]).collect();
        // Slopes m of the natural spline: the tridiagonal system
        //   2 m0 + m1 = 3 d0;  h_i m_{i-1} + 2 (h_{i-1} + h_i) m_i + h_{i-1} m_{i+1} = 3 (h_i d_{i-1} + h_{i-1} d_i);
        //   m_{n-2} + 2 m_{n-1} = 3 d_{n-2}
        // solved by the Thomas algorithm (diagonally dominant: no pivoting needed).
        let mut a = vec![0.0; n]; // sub-diagonal
        let mut b = vec![0.0; n]; // diagonal
        let mut c = vec![0.0; n]; // super-diagonal
        let mut r = vec![0.0; n];
        b[0] = 2.0;
        c[0] = 1.0;
        r[0] = 3.0 * d[0];
        for i in 1..n - 1 {
            a[i] = h[i];
            b[i] = 2.0 * (h[i - 1] + h[i]);
            c[i] = h[i - 1];
            r[i] = 3.0 * (h[i] * d[i - 1] + h[i - 1] * d[i]);
        }
        a[n - 1] = 1.0;
        b[n - 1] = 2.0;
        r[n - 1] = 3.0 * d[n - 2];
        for i in 1..n {
            let w = a[i] / b[i - 1];
            b[i] -= w * c[i - 1];
            r[i] -= w * r[i - 1];
        }
        let mut ms = vec![0.0; n];
        ms[n - 1] = r[n - 1] / b[n - 1];
        for i in (0..n - 1).rev() {
            ms[i] = (r[i] - c[i] * ms[i + 1]) / b[i];
        }
        if ms.iter().any(|m| !m.is_finite()) {
            ms = vec![0.0; n];
        }
        NaturalCurve { xs, ys, ms }
    }

    pub fn eval(&self, x: f64) -> f64 {
        let n = self.xs.len();
        let y = if !x.is_finite() || x <= self.xs[0] {
            self.ys[0]
        } else if x >= self.xs[n - 1] {
            self.ys[n - 1]
        } else {
            let i = match self.xs.binary_search_by(|v| v.total_cmp(&x)) {
                Ok(i) => return self.ys[i].clamp(0.0, 1.0),
                Err(i) => i.saturating_sub(1).min(n - 2),
            };
            let h = self.xs[i + 1] - self.xs[i];
            let t = (x - self.xs[i]) / h;
            let (t2, t3) = (t * t, t * t * t);
            (2.0 * t3 - 3.0 * t2 + 1.0) * self.ys[i]
                + (t3 - 2.0 * t2 + t) * h * self.ms[i]
                + (-2.0 * t3 + 3.0 * t2) * self.ys[i + 1]
                + (t3 - t2) * h * self.ms[i + 1]
        };
        y.clamp(0.0, 1.0)
    }

    /// Sample into a LUT of `n` entries over `0..1`.
    pub fn to_lut(&self, n: usize) -> Lut1 {
        let n = n.max(2);
        Lut1 { v: (0..n).map(|i| self.eval(i as f64 / (n - 1) as f64) as f32).collect() }
    }
}

/// A 1-D lookup table over `0..1` with linear interpolation; inputs outside clamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut1 {
    pub v: Vec<f32>,
}

impl Lut1 {
    pub fn identity(n: usize) -> Lut1 {
        Lut1 { v: (0..n).map(|i| i as f32 / (n - 1) as f32).collect() }
    }
    pub fn from_fn(n: usize, f: impl Fn(f32) -> f32) -> Lut1 {
        Lut1 { v: (0..n).map(|i| f(i as f32 / (n - 1) as f32)).collect() }
    }
    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        let n = self.v.len();
        let f = x.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (f as usize).min(n - 2);
        let t = f - i as f32;
        self.v[i] + (self.v[i + 1] - self.v[i]) * t
    }
    /// `self ∘ inner` (apply `inner` first).
    pub fn compose(&self, inner: &Lut1) -> Lut1 {
        Lut1 { v: inner.v.iter().map(|&x| self.eval(x)).collect() }
    }
    pub fn is_identity(&self) -> bool {
        let n = self.v.len();
        self.v.iter().enumerate().all(|(i, &y)| (y - i as f32 / (n - 1) as f32).abs() < 1e-6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_curve() {
        let c = MonotoneCurve::identity();
        for i in 0..=10 {
            let x = i as f64 / 10.0;
            assert!((c.eval(x) - x).abs() < 1e-12);
        }
        assert!(c.to_lut(256).is_identity());
    }

    #[test]
    fn passes_through_points_and_is_monotone() {
        let pts = [(0.0, 0.0), (0.25, 0.15), (0.5, 0.5), (0.75, 0.88), (1.0, 1.0)];
        let c = MonotoneCurve::new(&pts);
        for (x, y) in pts {
            assert!((c.eval(x) - y).abs() < 1e-12);
        }
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.eval(i as f64 / 1000.0);
            assert!(y >= prev - 1e-12);
            prev = y;
        }
    }

    #[test]
    fn no_overshoot_on_flat_segment() {
        let c = MonotoneCurve::new(&[(0.0, 0.0), (0.4, 0.5), (0.6, 0.5), (1.0, 1.0)]);
        for i in 400..=600 {
            let y = c.eval(i as f64 / 1000.0);
            assert!((y - 0.5).abs() < 1e-9, "{y}");
        }
    }

    #[test]
    fn natural_curve_is_camera_raws_spline() {
        // Bryan's custom curve (0,6 62,39 98,84 181,167 255,211) and a steep one that overshoots;
        // reference values: the C2 / natural-end spline as Camera Raw evaluates it (DNG SDK
        // `dng_spline_solver`), clamped to 0..1 as Camera Raw clamps it (oracle ramp)
        let p = |v: &[f64]| v.chunks(2).map(|c| (c[0] / 255.0, c[1] / 255.0)).collect::<Vec<_>>();
        let c = NaturalCurve::new(&p(&[0.0, 6.0, 62.0, 39.0, 98.0, 84.0, 181.0, 167.0, 255.0, 211.0]));
        for (x, y) in [(0.05, 0.038555), (0.2, 0.114332), (0.3, 0.218648), (0.5, 0.466369), (0.6, 0.565173), (0.9, 0.774151)] {
            assert!((c.eval(x) - y).abs() < 2e-6, "{x}: {} vs {y}", c.eval(x));
        }
        assert!((c.eval(0.0) - 6.0 / 255.0).abs() < 1e-12 && (c.eval(1.0) - 211.0 / 255.0).abs() < 1e-12);
        let s = NaturalCurve::new(&p(&[0.0, 0.0, 100.0, 60.0, 110.0, 130.0, 255.0, 255.0]));
        assert_eq!(s.eval(0.2), 0.0); // the spline dips below 0 (−0.3245): clamped
        assert!((s.eval(0.41) - 0.358622).abs() < 2e-6);
        assert_eq!(s.eval(0.6), 1.0); // overshoots to 1.264: clamped
        // degenerate input never panics and gives a finite curve
        for pts in [vec![], vec![(0.5, 0.5)], vec![(0.0, f64::NAN), (1.0, 1.0)], vec![(0.2, 0.3), (0.2, 0.9)]] {
            let c = NaturalCurve::new(&pts);
            assert!((0..=10).all(|i| c.eval(i as f64 / 10.0).is_finite()));
        }
        assert!(NaturalCurve::new(&[(0.0, 0.0), (1.0, 1.0)]).to_lut(64).is_identity());
    }

    #[test]
    fn lut_compose() {
        let a = Lut1::from_fn(1024, |x| x * x);
        let b = Lut1::from_fn(1024, |x| x.sqrt());
        let c = a.compose(&b);
        assert!((c.eval(0.3) - 0.3).abs() < 2e-3);
    }

    proptest::proptest! {
        #[test]
        fn monotone_for_increasing_data(ys in proptest::collection::vec(0.0f64..1.0, 3..8)) {
            let mut ys = ys; ys.sort_by(|a, b| a.total_cmp(b));
            let n = ys.len();
            let pts: Vec<(f64, f64)> = ys.iter().enumerate().map(|(i, y)| (i as f64 / (n - 1) as f64, *y)).collect();
            let c = MonotoneCurve::new(&pts);
            let mut prev = f64::MIN;
            for i in 0..=200 { let y = c.eval(i as f64 / 200.0); proptest::prop_assert!(y >= prev - 1e-9); prev = y; }
        }
    }
}
