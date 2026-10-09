//! The global tone map: scene luminance → display-linear luminance.
//!
//! Built in the log domain around middle grey (0.18): contrast scales log-exposure about grey,
//! whites move the shoulder (white point), blacks move the toe. The shoulder is an extended
//! Reinhard curve so highlights roll off smoothly instead of clipping.
//!
//! Rendered (display-referred) sources such as JPEGs use [`ToneMap::display`] instead: identity at
//! neutral settings (an unedited JPEG renders exactly as the file), with contrast/whites/blacks as
//! S-curve adjustments in a gamma-2.2 perceptual domain and a short shoulder above 0.95.

pub const GREY: f32 = 0.18;
/// The tone LUT spans `LUT_MIN_EV..LUT_MAX_EV` around grey in `LUT_N` steps.
pub const LUT_MIN_EV: f32 = -14.0;
pub const LUT_MAX_EV: f32 = 10.0;
pub const LUT_N: usize = 4096;
/// Nodes of a camera chroma curve, evenly spaced over display luminance 0..=1.
pub const CHROMA_N: usize = 8;
const NO_CHROMA: [f32; CHROMA_N] = [1.0; CHROMA_N];

/// A file-local camera look, fitted independently of the scene-linear colour transform.
/// Knots are scene/display-linear luminance pairs. Keeping this in the finish stage preserves
/// RAW exposure and highlight headroom; it is never baked into the decoded sensor pixels.
/// `chroma` scales colourfulness by display luminance after the curve (a camera's per-channel
/// curve saturates shadows and bleaches highlights toward white, which a luminance curve can't).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct CameraTone {
    knots: [[f32; 2]; 32],
    chroma: [f32; CHROMA_N],
}

impl<'de> serde::Deserialize<'de> for CameraTone {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            knots: [[f32; 2]; 32],
            // absent in smart previews written before the chroma curve existed
            #[serde(default)]
            chroma: Option<[f32; CHROMA_N]>,
        }
        let w = Wire::deserialize(d)?;
        let tone = Self::new(w.knots).ok_or_else(|| serde::de::Error::custom("invalid camera tone curve"))?;
        match w.chroma {
            Some(c) => tone.with_chroma(c).ok_or_else(|| serde::de::Error::custom("invalid camera chroma curve")),
            None => Ok(tone),
        }
    }
}

impl CameraTone {
    pub fn new(knots: [[f32; 2]; 32]) -> Option<Self> {
        let mut previous = [0.0, 0.0];
        for p in knots {
            if !p.iter().all(|v| v.is_finite()) || p[0] <= previous[0] || p[1] < previous[1] || p[1] >= 1.0 {
                return None;
            }
            previous = p;
        }
        Some(Self { knots, chroma: NO_CHROMA })
    }

    /// The curve with chroma scales at display luminance 0, 1/7 … 1 (each finite, 0..=4).
    pub fn with_chroma(self, chroma: [f32; CHROMA_N]) -> Option<Self> {
        chroma.iter().all(|k| k.is_finite() && (0.0..=4.0).contains(k)).then_some(Self { chroma, ..self })
    }

    pub fn chroma(&self) -> &[f32; CHROMA_N] {
        &self.chroma
    }

    pub fn apply(&self, y: f32) -> f32 {
        if !y.is_finite() || y <= 0.0 {
            return 0.0;
        }
        let mut previous = [0.0, 0.0];
        for p in self.knots {
            if y <= p[0] {
                let t = (y - previous[0]) / (p[0] - previous[0]);
                return previous[1] + t * (p[1] - previous[1]);
            }
            previous = p;
        }
        let a = self.knots[30];
        let b = self.knots[31];
        // Extend beyond observed (unclipped) highlights with a continuous, bounded shoulder.
        let slope = ((b[1] - a[1]) / (b[0] - a[0])).clamp(0.1, 16.0);
        1.0 - (1.0 - b[1]) * (-(y - b[0]) * slope / (1.0 - b[1]).max(0.01)).exp()
    }
}

/// Lightroom-matched Basic tone for raw files with a camera tone curve: how many EV each slider
/// moves a tone at EV `EV0`, `EV0 + 1` … (relative to middle grey), per 100 slider units
/// (Exposure: per EV, on top of its gain). Exposure, Contrast, Whites and Blacks act on the
/// scene EV after exposure (and after Highlights / Shadows); Highlights and Shadows act on the
/// edge-aware local base relative to the photo's own key ([`image_key`]): Lightroom's are image
/// adaptive, so a dark photo's "highlights" sit lower than a bright one's. Fitted (robust ridge
/// regression on smooth piecewise-linear functions, held-out split) to ~300 raw photos and
/// Lightroom Classic's own renders of them (`tools/lr-compare`, `docs/lr-match.md`); black-box
/// observation of output only.
pub mod lr {
    pub const EV0: f32 = -10.0;
    pub const N: usize = 15;
    pub const CONTRAST: [f32; N] = [0.137, -0.006, -0.162, -0.344, -0.551, -0.743, -0.832, -0.704, -0.4, -0.065, 0.172, 0.259, 0.251, 0.199, 0.133];
    pub const HIGHLIGHTS: [f32; N] = [0.014, -0.008, -0.032, -0.057, -0.082, -0.095, -0.061, 0.051, 0.262, 0.522, 0.728, 0.844, 0.84, 0.751, 0.626];
    pub const SHADOWS: [f32; N] = [-0.112, 0.016, 0.156, 0.316, 0.492, 0.651, 0.743, 0.701, 0.549, 0.429, 0.324, 0.186, 0.013, -0.17, -0.345];
    pub const WHITES: [f32; N] = [0.044, -0.003, -0.055, -0.115, -0.183, -0.244, -0.269, -0.207, -0.015, 0.234, 0.418, 0.463, 0.394, 0.272, 0.136];
    pub const BLACKS: [f32; N] = [-0.097, -0.011, 0.084, 0.199, 0.338, 0.481, 0.58, 0.575, 0.475, 0.304, 0.11, -0.031, -0.104, -0.133, -0.148];
    pub const EXPOSURE: [f32; N] =
        [0.055, -0.041, -0.142, -0.25, -0.352, -0.409, -0.353, -0.173, -0.052, -0.033, -0.07, -0.157, -0.202, -0.202, -0.184];
    /// The key (after exposure) at which [`HIGHLIGHTS`] / [`SHADOWS`] are tabulated: the median
    /// key of the photos they were fitted on.
    pub const KEY_REF: f32 = -2.243;
    /// [`image_key`] reads every `KEY_STEP`-th pixel.
    pub const KEY_STEP: usize = 7;

    /// A photo's key: the mean log2 luminance relative to grey (each clamped to −14..10 EV) of
    /// every [`KEY_STEP`]-th value of its log-luminance plane, before exposure. `None` when
    /// there are no finite values.
    pub fn image_key(log_l: &[f32]) -> Option<f32> {
        key_of(log_l.iter().step_by(KEY_STEP).copied())
    }

    /// [`image_key`] of values already taken every [`KEY_STEP`]-th (the GPU reads them back so).
    pub fn key_of(values: impl Iterator<Item = f32>) -> Option<f32> {
        let (mut sum, mut n) = (0.0f64, 0usize);
        for v in values.filter(|v| v.is_finite()) {
            sum += f64::from(v.clamp(-14.0, 10.0));
            n += 1;
        }
        (n > 0).then(|| (sum / n as f64) as f32)
    }

    /// `t` at scene EV `ev` (linear between knots, constant beyond).
    #[inline]
    pub fn at(t: &[f32; N], ev: f32) -> f32 {
        if !ev.is_finite() {
            return 0.0;
        }
        let f = (ev - EV0).clamp(0.0, (N - 1) as f32);
        let i = (f as usize).min(N - 2);
        let k = f - i as f32;
        let (a, b) = (t.get(i).copied().unwrap_or(0.0), t.get(i + 1).copied().unwrap_or(0.0));
        a + (b - a) * k
    }

    /// Entries of [`hs_lut`]: EV0..EV0+N-1 in quarter stops.
    pub const HS_N: usize = (N - 1) * 4 + 1;

    /// Highlights and Shadows (−1..1) as a table of EV changes by local base EV (quarter stops
    /// from `EV0`, after exposure), applied per pixel on the edge-aware base. `key_offset`: the
    /// photo's key after exposure minus [`KEY_REF`] (0 = tabulated as fitted).
    pub fn hs_lut(highlights: f32, shadows: f32, key_offset: f32) -> Vec<f32> {
        let off = if key_offset.is_finite() { key_offset.clamp(-12.0, 12.0) } else { 0.0 };
        (0..HS_N)
            .map(|i| {
                let ev = EV0 + i as f32 * 0.25 - off;
                highlights * at(&HIGHLIGHTS, ev) + shadows * at(&SHADOWS, ev)
            })
            .collect()
    }

    /// [`hs_lut`] at base EV `ev`.
    #[inline]
    pub fn hs_at(t: &[f32], ev: f32) -> f32 {
        if t.len() < 2 || !ev.is_finite() {
            return 0.0;
        }
        let f = ((ev - EV0) * 4.0).clamp(0.0, (t.len() - 1) as f32);
        let i = (f as usize).min(t.len() - 2);
        let k = f - i as f32;
        let (a, b) = (t.get(i).copied().unwrap_or(0.0), t.get(i + 1).copied().unwrap_or(0.0));
        a + (b - a) * k
    }
}

#[derive(Clone, Debug)]
pub struct ToneMap {
    lut: Vec<f32>,
    chroma: [f32; CHROMA_N],
}

impl ToneMap {
    pub fn camera(curve: &CameraTone, contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        let adjustment = Self::display(contrast, whites, blacks);
        let neutral = contrast == 0.0 && whites == 0.0 && blacks == 0.0;
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = curve.apply(GREY * 2f32.powf(ev));
                if neutral { y } else { adjustment.apply(y) }
            })
            .collect();
        ToneMap { lut, chroma: curve.chroma }
    }
    /// A raw file's camera tone with Lightroom-matched Basic tone ([`lr`]): `exposure` in EV,
    /// `contrast`, `whites`, `blacks` in −100..100, as global moves in EV before the camera curve.
    pub fn camera_lr(curve: &CameraTone, exposure: f64, contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        let (e, c, w, b) = (exposure as f32, (contrast / 100.0) as f32, (whites / 100.0) as f32, (blacks / 100.0) as f32);
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let d = e * lr::at(&lr::EXPOSURE, ev) + c * lr::at(&lr::CONTRAST, ev) + w * lr::at(&lr::WHITES, ev) + b * lr::at(&lr::BLACKS, ev);
                curve.apply(GREY * 2f32.powf(ev + d))
            })
            .collect();
        ToneMap { lut, chroma: curve.chroma }
    }

    /// `contrast`, `whites`, `blacks` in −100..100 (Lightroom slider units).
    pub fn new(contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        let c = (contrast / 100.0) as f32;
        let slope = if c >= 0.0 { 1.0 + 0.55 * c } else { 1.0 + 0.4 * c };
        // White point: scene luminance (after contrast) that maps to display 1.0.
        let white_ev = 2.9 - 1.6 * (whites as f32 / 100.0);
        let wl = GREY * 2f32.powf(white_ev);
        let pre = 1.0 + GREY / wl; // keep grey near grey
        let b = (blacks / 100.0) as f32;
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = GREY * 2f32.powf(ev * slope) * pre;
                // extended Reinhard with white point wl: y(1 + y/wl²)/(1 + y)
                let mut o = y * (1.0 + y / (wl * wl)) / (1.0 + y);
                o = o.min(1.0);
                // Toe: blacks < 0 crushes, > 0 lifts.
                if b < 0.0 {
                    // Smooth max(0, o − k) (a soft knee), renormalized so 1 stays 1.
                    let k = -b * 0.035;
                    let e = 0.004;
                    let soft = |v: f32| ((v - k) + ((v - k) * (v - k) + e * e).sqrt()) * 0.5;
                    o = (soft(o) - soft(0.0)) / (soft(1.0) - soft(0.0));
                } else if b > 0.0 {
                    let k = b * 0.03;
                    o = k + (1.0 - k) * o;
                }
                o.clamp(0.0, 1.0)
            })
            .collect();
        ToneMap { lut, chroma: NO_CHROMA }
    }

    /// Tone map for display-referred sources: identity at neutral settings.
    pub fn display(contrast: f64, whites: f64, blacks: f64) -> ToneMap {
        let c = (contrast / 100.0) as f32;
        let w = (whites / 100.0) as f32;
        let b = (blacks / 100.0) as f32;
        let m = GREY.powf(1.0 / 2.2);
        let lut = (0..LUT_N)
            .map(|i| {
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
                let y = GREY * 2f32.powf(ev);
                let mut p = y.powf(1.0 / 2.2);
                if p <= 1.0 {
                    // S-curve anchored at 0, grey and 1
                    p += c * 0.35 * (p - m) * (1.0 - (2.0 * p - 1.0).powi(2));
                    // whites: lift/lower the upper tones; blacks: the lower tones
                    let up = smooth(0.45, 1.0, p);
                    p += w * 0.12 * up * (1.0 - p * 0.5);
                    let lo = 1.0 - smooth(0.0, 0.45, p);
                    p += b * 0.07 * lo * (p * 2.0).min(1.0);
                } else {
                    p += w * 0.12 * 0.5;
                }
                let mut o = p.max(0.0).powf(2.2);
                // short shoulder: slope 1 at 0.95, reaching 1.0 at 1.05
                if o > 0.95 {
                    let d = (o - 0.95).min(0.1);
                    o = 0.95 + d - d * d / 0.2;
                }
                o.clamp(0.0, 1.0)
            })
            .collect();
        ToneMap { lut, chroma: NO_CHROMA }
    }

    /// The table (`LUT_N` entries, see [`ToneMap::apply`]).
    pub fn lut(&self) -> &[f32] {
        &self.lut
    }

    /// The chroma curve (`CHROMA_N` entries, see [`ToneMap::chroma_scale`]).
    pub fn chroma_lut(&self) -> &[f32] {
        &self.chroma
    }

    /// Chroma scale at display luminance `o` (exactly 1 everywhere unless a camera look sets it).
    #[inline]
    pub fn chroma_scale(&self, o: f32) -> f32 {
        if !o.is_finite() {
            return 1.0;
        }
        let f = o.clamp(0.0, 1.0) * (CHROMA_N - 1) as f32;
        let i = (f as usize).min(CHROMA_N - 2);
        let t = f - i as f32;
        let (a, b) = (self.chroma.get(i).copied().unwrap_or(1.0), self.chroma.get(i + 1).copied().unwrap_or(1.0));
        if a == b { a } else { a + (b - a) * t }
    }

    /// Scene luminance → display-linear luminance.
    #[inline]
    pub fn apply(&self, y: f32) -> f32 {
        if y <= 0.0 {
            return 0.0;
        }
        let ev = (y / GREY).log2();
        let f = ((ev - LUT_MIN_EV) / (LUT_MAX_EV - LUT_MIN_EV)).clamp(0.0, 1.0) * (LUT_N - 1) as f32;
        let i = (f as usize).min(LUT_N - 2);
        let t = f - i as f32;
        let v = self.lut[i] + (self.lut[i + 1] - self.lut[i]) * t;
        if ev < LUT_MIN_EV { v * (y / (GREY * 2f32.powf(LUT_MIN_EV))) } else { v }
    }
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightroom_tone_is_the_camera_curve_at_zero_and_moves_the_right_way() {
        let curve = CameraTone::new(std::array::from_fn(|i| {
            let x = 2f32.powf(-12.0 + 12.5 * i as f32 / 31.0);
            [x, (x / (x + 0.2)).min(0.999)]
        }))
        .unwrap();
        let plain = ToneMap::camera_lr(&curve, 0.0, 0.0, 0.0, 0.0);
        for y in [0.001f32, 0.02, 0.18, 0.9] {
            assert!((plain.apply(y) - curve.apply(y)).abs() < 1e-3, "{y}");
        }
        let lifted = ToneMap::camera_lr(&curve, 0.0, 0.0, 0.0, 60.0);
        assert!(lifted.apply(0.01) > plain.apply(0.01) * 1.2, "Blacks + lifts the shadows");
        let flat = ToneMap::camera_lr(&curve, 0.0, -80.0, 0.0, 0.0);
        assert!(flat.apply(0.01) > plain.apply(0.01), "Contrast − lifts the darks");
        // (the exposure gain itself comes before the tone map; its curve rolls the ends off)
        let up = ToneMap::camera_lr(&curve, 1.0, 0.0, 0.0, 0.0);
        assert!(up.apply(0.1) > plain.apply(0.05), "Exposure + brightens");
        assert!(up.apply(8.0) < plain.apply(8.0) + 1e-6, "and rolls the highlights off");
        for t in [&lifted, &flat, &up] {
            let lut = t.lut();
            assert!(lut.windows(2).all(|w| w[1] >= w[0] - 1e-6) && lut.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        }
        // Highlights / Shadows: shadows lift dark bases, highlights − pulls bright ones down
        let t = lr::hs_lut(-0.9, 0.5, 0.0);
        assert_eq!(t.len(), lr::HS_N);
        assert!(lr::hs_at(&t, -6.0) > 0.3 && lr::hs_at(&t, 1.0) < 0.0, "{t:?}");
        assert_eq!(lr::hs_at(&t, f32::NAN), 0.0);
        assert_eq!(lr::hs_at(&[], 1.0), 0.0);
        assert!((lr::hs_at(&t, 100.0) - t[t.len() - 1]).abs() < 1e-6 && (lr::hs_at(&t, -100.0) - t[0]).abs() < 1e-6, "clamped");
    }

    #[test]
    fn highlights_and_shadows_follow_the_photos_own_key() {
        // a photo 3 EV darker than the reference key: its "highlights" sit 3 EV lower
        let (mid, dark) = (lr::hs_lut(-0.8, 0.0, 0.0), lr::hs_lut(-0.8, 0.0, -3.0));
        for b in [-6.0f32, -3.0, 0.0] {
            assert!((lr::hs_at(&dark, b - 3.0) - lr::hs_at(&mid, b)).abs() < 1e-5, "{b}");
        }
        assert!(lr::hs_at(&dark, -2.0) < lr::hs_at(&mid, -2.0) - 0.1, "Highlights − reaches lower in a dark photo");
        // hostile offsets: finite tables, never a panic
        for off in [f32::NAN, f32::INFINITY, -1e9] {
            assert!(lr::hs_lut(1.0, 1.0, off).iter().all(|v| v.is_finite()));
        }
        // the key: mean clamped log luminance of every KEY_STEP-th value, None when empty
        assert_eq!(lr::image_key(&[]), None);
        assert_eq!(lr::image_key(&[f32::NAN; 20]), None);
        let plane: Vec<f32> = (0..70).map(|i| if i % 7 == 0 { -2.0 } else { 99.0 }).collect();
        assert!((lr::image_key(&plane).unwrap() + 2.0).abs() < 1e-6);
        assert_eq!(lr::key_of(plane.iter().step_by(lr::KEY_STEP).copied()), lr::image_key(&plane));
        assert_eq!(lr::image_key(&[-40.0]), Some(-14.0));
    }

    fn knots() -> [[f32; 2]; 32] {
        std::array::from_fn(|i| {
            let x = 0.004 * 1.18f32.powi(i as i32);
            [x, 1.0 - (-2.0 * x).exp()]
        })
    }

    #[test]
    fn chroma_curve_is_identity_unless_set_and_survives_serde() {
        let plain = CameraTone::new(knots()).unwrap();
        let map = ToneMap::camera(&plain, 0.0, 0.0, 0.0);
        assert!([0.0, 0.3, 0.77, 1.0, 2.0].iter().all(|o| map.chroma_scale(*o) == 1.0));
        assert!([0.0, 0.5, 1.0].iter().all(|o| ToneMap::new(0.0, 0.0, 0.0).chroma_scale(*o) == 1.0));
        // smart previews written before the chroma curve existed still load (identity)
        let old = serde_json::json!({ "knots": knots() });
        assert_eq!(serde_json::from_value::<CameraTone>(old).unwrap(), plain);
        let tone = plain.with_chroma([1.4, 1.3, 1.1, 1.0, 0.7, 0.4, 0.25, 0.2]).unwrap();
        let back: CameraTone = serde_json::from_value(serde_json::to_value(tone).unwrap()).unwrap();
        assert_eq!(back, tone);
        let map = ToneMap::camera(&tone, 0.0, 0.0, 0.0);
        assert!((map.chroma_scale(0.0) - 1.4).abs() < 1e-6 && (map.chroma_scale(1.0) - 0.2).abs() < 1e-6);
        assert!((map.chroma_scale(0.5 / 7.0) - 1.35).abs() < 1e-5, "interpolates between nodes");
        assert_eq!(map.chroma_scale(f32::NAN), 1.0);
        // hostile values are rejected, also when deserialized
        assert!(plain.with_chroma([f32::NAN; CHROMA_N]).is_none());
        assert!(plain.with_chroma([-1.0; CHROMA_N]).is_none());
        let bad = serde_json::json!({ "knots": knots(), "chroma": [9.0, 1, 1, 1, 1, 1, 1, 1] });
        assert!(serde_json::from_value::<CameraTone>(bad).is_err());
    }

    #[test]
    fn camera_curve_preserves_black_and_extends_headroom() {
        let knots = std::array::from_fn(|i| {
            let x = 0.005 * 1.15f32.powi(i as i32);
            [x, 1.0 - (-3.0 * x).exp()]
        });
        let curve = CameraTone::new(knots).unwrap();
        let base = ToneMap::camera(&curve, 0.0, 0.0, 0.0);
        assert_eq!(base.apply(0.0), 0.0);
        assert!(base.apply(0.1) < base.apply(0.2));
        let mut previous = 0.0;
        for i in 0..2000 {
            let y = 1e-6 * 1.01f32.powi(i);
            let v = base.apply(y);
            assert!((0.0..=1.0).contains(&v));
            assert!(v >= previous - 1e-6);
            previous = v;
        }
        assert!((base.apply(0.1) - curve.apply(0.1)).abs() < 0.001);
        assert!(ToneMap::camera(&curve, 50.0, 0.0, 0.0).apply(0.3) > base.apply(0.3));
        let mut invalid = knots;
        invalid[1][0] = invalid[0][0];
        assert!(CameraTone::new(invalid).is_none());
        assert!(serde_json::from_value::<CameraTone>(serde_json::json!({"knots": invalid})).is_err());
    }

    #[test]
    fn monotone_and_bounded() {
        for (c, w, b) in [(0.0, 0.0, 0.0), (100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (50.0, -30.0, -40.0)] {
            let t = ToneMap::new(c, w, b);
            let mut prev = -1.0;
            for i in 0..2000 {
                let y = 1e-5 * 1.012f32.powi(i);
                let o = t.apply(y);
                assert!((0.0..=1.0).contains(&o));
                assert!(o >= prev - 1e-6, "{c} {w} {b} at {y}: {o} < {prev}");
                prev = o;
            }
        }
    }

    #[test]
    fn display_identity_and_monotone() {
        let t = ToneMap::display(0.0, 0.0, 0.0);
        for i in 1..=95 {
            let y = i as f32 / 100.0;
            assert!((t.apply(y) - y).abs() < 2e-3, "{y} -> {}", t.apply(y));
        }
        for (c, w, b) in [(100.0, 100.0, -100.0), (-100.0, -100.0, 100.0), (60.0, -40.0, 30.0)] {
            let t = ToneMap::display(c, w, b);
            let mut prev = -1.0;
            for i in 0..1000 {
                let o = t.apply(i as f32 / 500.0);
                assert!(o >= prev - 1e-5, "{c} {w} {b}");
                prev = o;
            }
        }
        let c = ToneMap::display(60.0, 0.0, 0.0);
        assert!(c.apply(0.05) < 0.05 && c.apply(0.7) > 0.7);
    }

    #[test]
    fn grey_stays_near_grey_and_highlights_roll_off() {
        let t = ToneMap::new(0.0, 0.0, 0.0);
        let g = t.apply(0.18);
        assert!((0.15..0.24).contains(&g), "{g}");
        assert!(t.apply(1.0) < 0.95 && t.apply(1.0) > 0.6);
        assert!(t.apply(8.0) > 0.97);
    }

    #[test]
    fn sliders_move_the_right_way() {
        let base = ToneMap::new(0.0, 0.0, 0.0);
        let contrast = ToneMap::new(60.0, 0.0, 0.0);
        assert!(contrast.apply(0.05) < base.apply(0.05));
        assert!(contrast.apply(0.8) > base.apply(0.8));
        assert!(ToneMap::new(0.0, 60.0, 0.0).apply(0.8) > base.apply(0.8));
        assert!(ToneMap::new(0.0, 0.0, -60.0).apply(0.01) < base.apply(0.01));
        assert!(ToneMap::new(0.0, 0.0, 60.0).apply(0.01) > base.apply(0.01));
    }
}
