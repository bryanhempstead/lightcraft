//! Point Color as Camera Raw renders it (R19, Bryan's fork; `docs/lr-match.md` → Round 10).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/point_color.rs`,
//! <https://github.com/pch/rawmakase>, MIT, Copyright (c) 2026 RAWmakase contributors; see NOTICE),
//! fitted to Camera Raw 18.7 renders of synthetic charts. Camera Raw works in HSV of linear
//! ProPhoto RGB after the colour mixer: a swatch's hue is the HSV hue in sixths of a turn, its
//! saturation the HSV saturation and its luminance the HSV value; the saturation range is on HSV
//! saturation, the luminance range on sRGB-encoded value. Each range is a trapezoid (outer and
//! inner points), smoothed; Range scales the distance from the swatch before the trapezoids
//! apply. A pixel's weight is the product of its hue, saturation and luminance weights, faded out
//! towards neutral. Within the selection Hue turns the HSV hue (±35° at ±100), Saturation and
//! Luminance scale HSV saturation and value, and Variance spreads or gathers hues, saturations
//! and values around the swatch. Swatches apply in turn, each to the result of the ones before.
//!
//! Used for swatches that came from Lightroom (XMP `crs:PointColors` or the catalog), which carry
//! Camera Raw's own sample and ranges ([`lightcraft_develop::LrSwatch`]).

use lightcraft_develop::PointColor;
use std::f32::consts::TAU;

/// Use this model for swatches that carry Lightroom's own sample.
pub const MEASURED: bool = true;

/// Fitted constants (RAWmakase, docs/color-mixer.md#point-color).
mod fit {
    /// Half the hue window at Range 50, in radians.
    pub(super) const HUE_WINDOW: f32 = 0.7236;
    /// How Range scales the distance from the swatch, per dimension (hue, saturation,
    /// luminance): 2^(k · (0.5 − Range)), with k below and above Range 50.
    pub(super) const RANGE_BELOW: [f32; 3] = [0.767, 1.7637, 0.5482];
    pub(super) const RANGE_ABOVE: [f32; 3] = [0.1186, 0.3886, 0.0598];
    /// Exponents of the smoothed ramps: hue, then saturation and luminance rising and falling.
    pub(super) const HUE_RAMP: f32 = 1.0282;
    pub(super) const SATURATION_RAMP: [f32; 2] = [1.8348, 0.9897];
    pub(super) const LUMINANCE_RAMP: [f32; 2] = [2.7765, 1.1354];
    /// Saturation below which pixels fade out of every selection.
    pub(super) const NEUTRAL: f32 = 0.0578;
    /// Hue turn at Hue ±100, radians (35°).
    pub(super) const HUE_SHIFT: f32 = 0.6109;
    /// Saturation ±100 multiplies or divides saturation by 1.9.
    pub(super) const SATURATION_SHIFT: f32 = 0.9;
    /// Luminance: log2 of the value factor is k1·|x| + k2·x².
    pub(super) const LUMINANCE_SHIFT: [f32; 2] = [1.3932, -0.2366];
    /// Variance ±100 scales distances from the swatch in saturation and encoded value by 2^±k
    /// (hue distances by 1 ± Variance).
    pub(super) const VARIANCE_SATURATION: f32 = 0.7904;
    pub(super) const VARIANCE_LUMINANCE: f32 = 0.2123;
}

/// Values per swatch in [`Swatch::words`].
pub const WORDS: usize = 22;

/// One swatch, prepared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swatch {
    hue: f32,
    saturation: f32,
    luminance: f32,
    scale: [f32; 3],
    hue_points: [f32; 4],
    saturation_points: [f32; 4],
    luminance_points: [f32; 4],
    hue_turn: f32,
    log_saturation: f32,
    log_value: f32,
    variance: f32,
}

#[inline]
fn srgb_encode(v: f32) -> f32 {
    lightcraft_color::transfer::linear_to_srgb(v.clamp(0.0, 1.0))
}

#[inline]
fn srgb_decode(v: f32) -> f32 {
    lightcraft_color::transfer::srgb_to_linear(v.clamp(0.0, 1.0))
}

fn finite4(r: [f64; 4]) -> Option<[f32; 4]> {
    let r = r.map(|v| v as f32);
    (r.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x)) && r.windows(2).all(|w| w[0] <= w[1])).then_some(r)
}

impl Swatch {
    /// The swatch of `p` when it carries Lightroom's sample and Camera Raw would accept it (values
    /// in range, a hue range with an inner span, saturation and luminance ranges that hold the
    /// sample) and it changes something.
    pub fn new(p: &PointColor) -> Option<Swatch> {
        let lr = p.lr.as_ref()?;
        let [h, s, v] = lr.source.map(|x| x as f32);
        if !((0.0..6.0).contains(&h) && (0.0..=1.0).contains(&s) && (0.0..=1.0).contains(&v)) {
            return None;
        }
        let (hr, sr, lrng) = (finite4(lr.hue_range)?, finite4(lr.sat_range)?, finite4(lr.lum_range)?);
        let holds = |r: &[f32; 4], x: f32| r[1] <= x + 1e-4 && x <= r[2] + 1e-4;
        if hr[1] >= hr[2] || !holds(&sr, s) || !holds(&lrng, srgb_encode(v)) || p.is_neutral() {
            return None;
        }
        let pct = |x: f64| if x.is_finite() { (x / 100.0).clamp(-1.0, 1.0) as f32 } else { 0.0 };
        let range = if p.range.is_finite() { (p.range / 100.0).clamp(0.0, 1.0) as f32 } else { 0.5 };
        let scale = std::array::from_fn(|d| {
            let k = if range < 0.5 { fit::RANGE_BELOW[d] } else { fit::RANGE_ABOVE[d] };
            (k * (0.5 - range)).exp2()
        });
        let (dh, ds, dl) = (pct(p.hue_shift), pct(p.sat_shift), pct(p.lum_shift));
        let l = dl.abs();
        Some(Swatch {
            hue: h / 6. * TAU,
            saturation: s,
            luminance: srgb_encode(v),
            scale,
            hue_points: hr.map(|x| (x - 0.5) * 2. * fit::HUE_WINDOW),
            saturation_points: sr,
            luminance_points: lrng,
            hue_turn: dh * fit::HUE_SHIFT,
            log_saturation: (1. + fit::SATURATION_SHIFT * ds.abs()).log2() * ds.signum(),
            log_value: (fit::LUMINANCE_SHIFT[0] * l + fit::LUMINANCE_SHIFT[1] * l * l) * dl.signum(),
            variance: pct(p.variance),
        })
    }

    /// The swatch for the GPU kernel ([`WORDS`] values; `point_color_cr` in `finish.wgsl`).
    pub fn words(&self) -> [f32; WORDS] {
        let mut w = [0.0; WORDS];
        w[0] = self.hue;
        w[1] = self.saturation;
        w[2] = self.luminance;
        w[3..6].copy_from_slice(&self.scale);
        w[6..10].copy_from_slice(&self.hue_points);
        w[10..14].copy_from_slice(&self.saturation_points);
        w[14..18].copy_from_slice(&self.luminance_points);
        w[18] = self.hue_turn;
        w[19] = self.log_saturation;
        w[20] = self.log_value;
        w[21] = self.variance;
        w
    }

    /// Linear ProPhoto `p` through this swatch.
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let q = p.map(|v| if v.is_finite() { v.max(0.) } else { 0.0 });
        let max = q[0].max(q[1]).max(q[2]);
        if max <= 1e-6 {
            return p;
        }
        let [h, s, v] = rgb_to_hsv(q);
        let ev = srgb_encode(v.min(1.));
        let d = wrap(h - self.hue);
        let weight = smoothed(d * self.scale[0], &self.hue_points, [fit::HUE_RAMP; 2])
            * smoothed(self.saturation + (s - self.saturation) * self.scale[1], &self.saturation_points, fit::SATURATION_RAMP)
            * smoothed(self.luminance + (ev - self.luminance) * self.scale[2], &self.luminance_points, fit::LUMINANCE_RAMP)
            * (s / fit::NEUTRAL).min(1.);
        if weight.is_nan() || weight <= 0.0 {
            return p;
        }
        let turn = weight * (self.hue_turn + self.variance * d);
        let mut log_s = weight * self.log_saturation;
        let mut log_v = weight * self.log_value;
        if self.variance != 0. {
            let spread = |x: f32, c: f32, k: f32| (c + (x - c) * (k * self.variance).exp2()).clamp(1e-4, 1.);
            let s2 = spread(s, self.saturation, fit::VARIANCE_SATURATION);
            let v2 = srgb_decode(spread(ev, self.luminance, fit::VARIANCE_LUMINANCE));
            log_s += weight * (s2 / s.max(1e-4)).log2();
            log_v += weight * (v2 / srgb_decode(ev).max(1e-6)).log2();
        }
        let out = hsv_to_rgb(h + turn, (s * log_s.exp2()).clamp(0., 1.), v * log_v.exp2());
        // channels clip at white: a brightened orange turns towards yellow rather than past white;
        // what HSV cannot carry (negative channels) stays as an offset
        let r: [f32; 3] = std::array::from_fn(|c| out[c].min(q[c].max(1.)) + (p[c] - q[c]));
        if r.iter().all(|v| v.is_finite()) { r } else { p }
    }
}

/// Hue (radians), saturation and value.
#[inline]
fn rgb_to_hsv(p: [f32; 3]) -> [f32; 3] {
    let max = p[0].max(p[1]).max(p[2]);
    let min = p[0].min(p[1]).min(p[2]);
    let d = max - min;
    let h = if d < 1e-9 {
        0.
    } else if max == p[0] {
        ((p[1] - p[2]) / d).rem_euclid(6.)
    } else if max == p[1] {
        (p[2] - p[0]) / d + 2.
    } else {
        (p[0] - p[1]) / d + 4.
    };
    [h / 6. * TAU, if max > 0.0 { d / max } else { 0.0 }, max]
}

/// HSV (hue in radians) back to RGB.
#[inline]
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h / TAU).rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1. - (h6 % 2. - 1.).abs());
    let q = match h6 as usize {
        0 => [c, x, 0.],
        1 => [x, c, 0.],
        2 => [0., c, x],
        3 => [0., x, c],
        4 => [x, 0., c],
        _ => [c, 0., x],
    };
    q.map(|w| w + v - c)
}

/// Hue difference in radians, −π to π.
#[inline]
fn wrap(d: f32) -> f32 {
    (d + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI
}

#[inline]
fn smoothstep(t: f32) -> f32 {
    t * t * (3. - 2. * t)
}

/// A trapezoid over `x` with outer points `p[0]`, `p[3]` and inner points `p[1]`, `p[2]`, its
/// rising and falling ramps smoothed and raised to `ramp`.
#[inline]
fn smoothed(x: f32, p: &[f32; 4], ramp: [f32; 2]) -> f32 {
    let rise = if x >= p[1] {
        1.
    } else if x <= p[0] {
        0.
    } else {
        smoothstep((x - p[0]) / (p[1] - p[0])).powf(ramp[0])
    };
    let fall = if x <= p[2] {
        1.
    } else if x >= p[3] {
        0.
    } else {
        smoothstep((p[3] - x) / (p[3] - p[2])).powf(ramp[1])
    };
    rise.min(fall)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::LrSwatch;

    fn sampled(src: [f64; 3]) -> PointColor {
        let around = |c: f64| [c - 0.73, c - 0.18, c + 0.18, c + 0.73].map(|v| v.clamp(0., 1.));
        PointColor {
            lr: Some(LrSwatch {
                source: src,
                hue_range: [0., 1. / 3., 2. / 3., 1.],
                sat_range: around(src[1]),
                lum_range: around(srgb_encode(src[2] as f32) as f64),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn desaturating_a_swatch_greys_its_colour_only() {
        // a blue (HSV hue 4 sixths) swatch at full desaturation
        let p = PointColor { sat_shift: -100.0, ..sampled([4.0, 0.6, 0.3]) };
        let s = Swatch::new(&p).unwrap();
        let blue = s.apply(hsv_to_rgb(4.0 / 6.0 * TAU, 0.6, 0.3));
        let [_, sat, _] = rgb_to_hsv(blue);
        assert!(sat < 0.4, "{blue:?}");
        let red = hsv_to_rgb(0.0, 0.6, 0.3);
        assert_eq!(s.apply(red), red);
        assert!(s.apply([f32::NAN, 0.1, 0.2]).iter().skip(1).all(|v| v.is_finite()));
    }

    #[test]
    fn camera_raw_drops_invalid_swatches() {
        assert!(Swatch::new(&PointColor { sat_shift: -100.0, ..Default::default() }).is_none(), "no Lightroom sample");
        let mut p = PointColor { sat_shift: -100.0, ..sampled([4.0, 0.6, 0.3]) };
        assert!(Swatch::new(&p).is_some());
        if let Some(lr) = p.lr.as_mut() {
            lr.hue_range = [-1.0; 4];
        }
        assert!(Swatch::new(&p).is_none(), "Lightroom's empty selection");
        assert!(Swatch::new(&PointColor { sat_shift: 0.0, ..sampled([4.0, 0.6, 0.3]) }).is_none(), "changes nothing");
    }
}
