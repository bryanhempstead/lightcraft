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
/// Display level (linear, SDR white = 1) above which [`ToneMap::with_headroom`] opens the SDR
/// shoulder into the HDR headroom; below it an HDR render is the SDR one.
pub const HDR_KNEE: f32 = 0.45;
/// The brightest scene value (linear, raw white = 1) an HDR render's channel-wise tone keeps:
/// the top of the tone table (`GREY · 2^LUT_MAX_EV`).
pub const HDR_SCENE_TOP: f32 = 184.0;

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

    /// The scene value this curve maps to display value `o` (inverse of [`CameraTone::apply`];
    /// `o` is held below 1, which the shoulder only approaches).
    pub fn invert(&self, o: f32) -> f32 {
        if !o.is_finite() || o <= 0.0 {
            return 0.0;
        }
        let o = o.min(0.9995);
        let mut previous = [0.0, 0.0];
        for p in self.knots {
            if o <= p[1] {
                let d = p[1] - previous[1];
                let t = if d > 0.0 { (o - previous[1]) / d } else { 0.0 };
                return previous[0] + t * (p[0] - previous[0]);
            }
            previous = p;
        }
        let a = self.knots[30];
        let b = self.knots[31];
        let slope = ((b[1] - a[1]) / (b[0] - a[0])).clamp(0.1, 16.0);
        let room = (1.0 - b[1]).max(0.01);
        let r = ((1.0 - o) / (1.0 - b[1]).max(1e-6)).clamp(1e-6, 1.0);
        b[0] - r.ln() * room / slope
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

/// The curve rendered (display-referred) files are seen through, so that Lightroom-matched Basic
/// tone acts on them as on raws (Lightroom's sliders act on a JPEG's tones as on a raw's): the
/// file is taken back to scene-like values by its inverse ([`rendered_to_scene`]) and this
/// curve renders it again, so an unedited file comes out as it went in. The knots are a typical
/// default raw tone (scene → display linear) as Lightroom renders one (fitted black-box from a
/// user's previews, `docs/lr-match.md`); measured on iPhone JPEGs with his presets, a camera-like
/// curve here matches Lightroom better than LightCraft's own filmic one.
pub const RENDERED_REFERENCE: [[f32; 2]; 32] = [
    [0.00307039, 0.000670656],
    [0.00370867, 0.00099296],
    [0.00447963, 0.00141501],
    [0.00541086, 0.00196295],
    [0.00653567, 0.00258562],
    [0.00789432, 0.00318626],
    [0.00953539, 0.00399587],
    [0.0115176, 0.0051065],
    [0.0139119, 0.00666682],
    [0.0168039, 0.00926836],
    [0.0202971, 0.0131438],
    [0.0245165, 0.0189117],
    [0.029613, 0.0271389],
    [0.035769, 0.0381187],
    [0.0432047, 0.0527594],
    [0.0521861, 0.0714381],
    [0.0630346, 0.0955155],
    [0.0761383, 0.126497],
    [0.091966, 0.165215],
    [0.111084, 0.214393],
    [0.134176, 0.274017],
    [0.162069, 0.342229],
    [0.19576, 0.418042],
    [0.236455, 0.500454],
    [0.285609, 0.584237],
    [0.344982, 0.667193],
    [0.416697, 0.746025],
    [0.50332, 0.814484],
    [0.60795, 0.872135],
    [0.734332, 0.917933],
    [0.886985, 0.949045],
    [1.07137, 0.974309],
];

/// [`RENDERED_REFERENCE`] as a camera tone (`None` only if the table were invalid).
pub fn rendered_reference() -> Option<CameraTone> {
    CameraTone::new(RENDERED_REFERENCE)
}

/// A display-referred linear Rec.2020 colour taken back to scene-like values through
/// [`rendered_reference`] (luminance-wise, hue and saturation kept).
pub fn rendered_to_scene(curve: &CameraTone, c: [f32; 3]) -> [f32; 3] {
    let y = lightcraft_color::luminance_2020(c);
    if !(y.is_finite() && y > 1e-9) {
        return [0.0; 3];
    }
    let k = curve.invert(y) / y;
    c.map(|v| (v * k).max(0.0))
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
    /// The same moves over an Adobe base ([`super::ToneMap::adobe_lr`], Bryan's fork), fitted on
    /// the exact base (`docs/lr-match.md` → Round 3): Lightroom's default tone over the profile's
    /// curve ([`ADOBE_BASE`], at zero sliders) on default-setting photos, the sliders on edited
    /// ones (split A + training photos; reported on the held-out split B).
    pub const ADOBE_BASE: [f32; N] = [0.065, 0.149, 0.231, 0.302, 0.350, 0.349, 0.263, 0.099, 0.067, -0.031, 0.009, -0.003, -0.147, -0.258, -0.357];
    pub const ADOBE_EXPOSURE: [f32; N] =
        [-0.065, -0.200, -0.333, -0.454, -0.545, -0.577, -0.138, -0.043, 0.006, 0.022, 0.046, -0.057, -0.162, -0.290, -0.404];
    pub const ADOBE_CONTRAST: [f32; N] =
        [-0.278, -0.652, -1.018, -1.343, -1.563, -1.569, -1.180, -0.717, -0.461, -0.046, 0.346, 0.664, 0.992, 1.199, 1.361];
    pub const ADOBE_HIGHLIGHTS: [f32; N] =
        [-1.212, -1.291, -1.329, -1.243, -0.867, -0.264, 0.267, 0.302, 0.294, 0.694, 0.995, 1.749, 2.824, 3.405, 4.125];
    pub const ADOBE_SHADOWS: [f32; N] = [1.204, 1.438, 1.631, 1.694, 1.397, 1.064, 1.443, 0.924, 0.382, 0.485, 0.531, 0.655, 1.298, 1.493, 1.220];
    pub const ADOBE_WHITES: [f32; N] = [0.346, 0.418, 0.479, 0.503, 0.449, 0.257, -0.147, -0.161, 0.342, 0.597, 0.644, 1.205, 1.833, 2.213, 2.509];
    pub const ADOBE_BLACKS: [f32; N] =
        [-0.702, -0.720, -0.713, -0.636, -0.417, 0.038, 0.757, 0.783, 0.406, 0.637, 0.293, -0.152, -0.526, -0.804, -1.047];
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

    /// What Camera Raw's Highlights / Shadows read from the whole photo (R02, measured with the
    /// Camera Raw oracle, `docs/lr-match.md` → *Highlights / Shadows (R02)*), in log2 luminance
    /// before exposure, from every [`KEY_STEP`]-th value.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ToneStats {
        /// [`image_key`] (Contrast's pivot).
        pub key: f32,
        /// Log2 of the mean linear luminance.
        pub lin_mean: f32,
        /// The [`HsParams::q_lo`]-th percentile.
        pub lo: f32,
        /// The [`HsParams::q_hi`]-th percentile.
        pub hi: f32,
    }

    /// [`ToneStats`] of a log-luminance plane (every [`KEY_STEP`]-th value). `None` when there are
    /// no finite values.
    pub fn image_stats(log_l: &[f32]) -> Option<ToneStats> {
        stats_of(log_l.iter().step_by(KEY_STEP).copied())
    }

    /// [`image_stats`] of values already taken every [`KEY_STEP`]-th (the GPU reads them back so).
    pub fn stats_of(values: impl Iterator<Item = f32>) -> Option<ToneStats> {
        let mut v: Vec<f32> = values.filter(|v| v.is_finite()).map(|v| v.clamp(-14.0, 10.0)).collect();
        let key = key_of(v.iter().copied())?;
        let lin = v.iter().map(|&x| f64::from(x).exp2()).sum::<f64>() / v.len() as f64;
        let lin_mean = (lin.log2() as f32).clamp(-14.0, 10.0);
        let n = v.len();
        let mut at = |q: f32| {
            let i = ((q / 100.0 * n as f32).max(0.0) as usize).min(n - 1);
            *v.select_nth_unstable_by(i, f32::total_cmp).1
        };
        let p = HS_PARAMS;
        Some(ToneStats { key, lin_mean, lo: at(p.q_lo), hi: at(p.q_hi) })
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
        hs_lut_with(&HIGHLIGHTS, &SHADOWS, highlights, shadows, key_offset)
    }

    /// Camera Raw's Highlights / Shadows (R02, measured with the oracle on Bryan's raws and
    /// synthetic scenes) as a table of EV changes by local base EV (quarter stops from `EV0`,
    /// after exposure): a curve on the edge-aware base that depends on the whole photo. Around a
    /// pivot `P` ([`ToneStats::lin_mean`] + `pivot`), Shadows lifts the levels below `P` by
    /// `S · s_slope` per EV below it, down to the `q_lo`-th percentile (darker levels move with
    /// it); Highlights moves the levels above `P` by `H · h_slope` (its own slope for each sign)
    /// per EV, up to the `q_hi`-th percentile. Both sliders add; the corners are rounded
    /// (`s_round`, `h_round` EV) and a photo of a single tone (range `hi − lo` under 1 EV) moves
    /// less, a uniform one not at all. `ev`: exposure (the stats are before it).
    pub fn adobe_hs_lut(highlights: f32, shadows: f32, stats: Option<ToneStats>, ev: f32) -> Vec<f32> {
        let Some(st) = stats else { return vec![0.0; HS_N] };
        let p = HS_PARAMS;
        let ev = if ev.is_finite() { ev.clamp(-12.0, 12.0) } else { 0.0 };
        let pivot = st.lin_mean + ev + p.pivot;
        let (lo, hi) = ((st.lo + ev).min(pivot), (st.hi + ev).max(pivot));
        let range = smoothstep(0.0, 1.0, st.hi - st.lo);
        if !(pivot.is_finite() && lo.is_finite() && hi.is_finite() && range > 0.0) {
            return vec![0.0; HS_N];
        }
        let h_slope = if highlights < 0.0 { p.h_slope_neg } else { p.h_slope_pos };
        (0..HS_N)
            .map(|i| {
                let x = EV0 + i as f32 * 0.25;
                let s = shadows * p.s_slope * (round_ramp(pivot - x, p.s_round) - round_ramp(lo - x, p.s_round));
                let h = highlights * h_slope * (round_ramp(x - pivot, p.h_round) - round_ramp(x - hi, p.h_round));
                range * (s + h)
            })
            .collect()
    }

    /// The fitted constants of [`adobe_hs_lut`] and [`ToneStats`].
    #[derive(Clone, Copy, Debug)]
    pub struct HsParams {
        /// Pivot relative to the log2 mean linear luminance (EV).
        pub pivot: f32,
        pub s_slope: f32,
        pub h_slope_neg: f32,
        pub h_slope_pos: f32,
        /// Percentiles (%) where Shadows' / Highlights' moves stop growing.
        pub q_lo: f32,
        pub q_hi: f32,
        /// Half-widths (EV) of the rounded corners.
        pub s_round: f32,
        pub h_round: f32,
    }

    /// Fitted on 26 of Bryan's raws (`tools/lr-compare/oracle/hsfit_rhs.py`, sets `make_rhs.py`):
    /// mean |ΔEV| of the move 0.385 (no Highlights / Shadows) → 0.104.
    pub const HS_PARAMS: HsParams = HsParams {
        pivot: -0.5356,
        s_slope: 0.6916,
        h_slope_neg: 0.4926,
        h_slope_pos: 0.5901,
        q_lo: 12.72,
        q_hi: 95.48,
        s_round: 1.8001,
        h_round: 1.3803,
    };

    /// `max(d, 0)` with the corner rounded over ±`w` (a parabola; C1).
    #[inline]
    fn round_ramp(d: f32, w: f32) -> f32 {
        if w <= 1e-4 || d >= w {
            d.max(0.0)
        } else if d <= -w {
            0.0
        } else {
            (d + w) * (d + w) / (4.0 * w)
        }
    }

    fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
        let t = if b > a {
            ((x - a) / (b - a)).clamp(0.0, 1.0)
        } else if x >= b {
            1.0
        } else {
            0.0
        };
        t * t * (3.0 - 2.0 * t)
    }

    /// [`hs_lut`] over an Adobe base, fitted on Lightroom's previews ([`ADOBE_HIGHLIGHTS`],
    /// [`ADOBE_SHADOWS`]); Camera Raw's own model is [`adobe_hs_lut`].
    pub fn adobe_hs_lut_previews(highlights: f32, shadows: f32, key_offset: f32) -> Vec<f32> {
        hs_lut_with(&ADOBE_HIGHLIGHTS, &ADOBE_SHADOWS, highlights, shadows, key_offset)
    }

    fn hs_lut_with(ht: &[f32; N], st: &[f32; N], highlights: f32, shadows: f32, key_offset: f32) -> Vec<f32> {
        let off = if key_offset.is_finite() { key_offset.clamp(-12.0, 12.0) } else { 0.0 };
        (0..HS_N)
            .map(|i| {
                let ev = EV0 + i as f32 * 0.25 - off;
                highlights * at(ht, ev) + shadows * at(st, ev)
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

    /// Lightroom's Basic tone over an Adobe base's tone curve ([`crate::adobe::Base`]), as Camera
    /// Raw renders it (measured, [`crate::tone_adobe`]): Lightroom's default tone, then Exposure,
    /// Contrast (pivot following the photo's `key`, [`lr::image_key`] before exposure), Whites and
    /// Blacks in turn as EV moves; Highlights / Shadows are local (`hs_lut`). The table is applied
    /// channel-wise in ProPhoto (`RefBaselineRGBTone`), not on luminance.
    pub fn adobe_lr(base: &crate::adobe::Base, exposure: f64, contrast: f64, whites: f64, blacks: f64, key: Option<f32>) -> ToneMap {
        use crate::tone_adobe as t;
        let pivot = key.filter(|k| k.is_finite()).map_or(0.0, |k| (k - t::KEY_RAMP).clamp(-10.0, 10.0) * t::CONTRAST_PIVOT);
        let (e, c, w, b) = (exposure as f32, contrast as f32, whites as f32, blacks as f32);
        let lut = (0..LUT_N)
            .map(|i| {
                // the table is indexed after Exposure's gain (the per-pixel stage applies it first):
                // Camera Raw's Exposure is the measured shift from the photo's own EV
                let ev = LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32 - e;
                let mut x = ev;
                if e != 0.0 {
                    x += adobe_shift(&t::E_VALUES, &t::E_TABLE, e, x);
                }
                if c != 0.0 {
                    x += adobe_shift(&t::C_VALUES, &t::C_TABLE, c, x - pivot);
                }
                if w != 0.0 {
                    x += adobe_shift(&t::W_VALUES, &t::W_TABLE, w, x);
                }
                if b != 0.0 {
                    x += adobe_shift(&t::K_VALUES, &t::K_TABLE, b, x);
                }
                let d = table_at(&t::BASE, ev) + (x - ev);
                base.curve_at(GREY * 2f32.powf(ev + d))
            })
            .collect();
        ToneMap { lut, chroma: NO_CHROMA }
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

    /// This (SDR) tone map extended for an HDR render reaching `peak` (linear, SDR white = 1;
    /// 1 or less, or not finite = this map unchanged), as Camera Raw's HDR mode renders an edit:
    /// shadows and midtones stay the SDR curve's, and the highlights that the SDR shoulder squeezes
    /// under white open up into the headroom instead. From the scene EV where the SDR curve reaches
    /// [`HDR_KNEE`] the output gains up to `peak − 1` more along a smoothstep that ends
    /// `log2(peak) + 1` stops past the SDR white point (where the curve reaches 0.98), so Whites
    /// (which move that point) and Highlights (which move the tones) work into the headroom. The
    /// result is monotone where the SDR curve is, never exceeds `peak` and reaches it in range.
    pub fn with_headroom(self, peak: f32) -> ToneMap {
        if !(peak.is_finite() && peak > 1.0) {
            return self;
        }
        let ev_at = |i: usize| LUT_MIN_EV + (LUT_MAX_EV - LUT_MIN_EV) * i as f32 / (LUT_N - 1) as f32;
        let first = |level: f32| self.lut.iter().position(|v| *v >= level).map_or(LUT_MAX_EV, ev_at);
        let from = first(HDR_KNEE);
        let top = (first(0.98) + peak.log2() + 1.0).max(from + 0.5);
        let lut = self.lut.iter().enumerate().map(|(i, v)| (v + (peak - 1.0) * smooth(from, top, ev_at(i))).clamp(0.0, peak)).collect();
        ToneMap { lut, chroma: self.chroma }
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

/// A [`crate::tone_adobe`] grid table at `ev` (linear between nodes, held at the ends).
fn table_at(t: &[f32], ev: f32) -> f32 {
    use crate::tone_adobe::{EV0, STEP};
    if t.len() < 2 || !ev.is_finite() {
        return 0.0;
    }
    let f = ((ev - EV0) / STEP).clamp(0.0, (t.len() - 1) as f32);
    let i = (f as usize).min(t.len() - 2);
    let k = f - i as f32;
    t[i] + (t[i + 1] - t[i]) * k
}

/// The EV shift of a slider at value `v` at input `ev`: the tables at the measured (sorted)
/// `values`, a zero table at 0, linear in between, held beyond the ends.
fn adobe_shift<const V: usize>(values: &[f32; V], tables: &[[f32; crate::tone_adobe::N]; V], v: f32, ev: f32) -> f32 {
    if !v.is_finite() || v == 0.0 || V == 0 {
        return 0.0;
    }
    // (value, shift at ev) with 0 → 0 inserted, in order
    let mut pts = [(0.0f32, 0.0f32); 16];
    let mut n = 0;
    let mut zero_done = false;
    for (x, t) in values.iter().zip(tables) {
        if !zero_done && *x > 0.0 && n < pts.len() {
            pts[n] = (0.0, 0.0);
            n += 1;
            zero_done = true;
        }
        if n < pts.len() {
            pts[n] = (*x, table_at(t, ev));
            n += 1;
        }
    }
    if !zero_done && n < pts.len() {
        pts[n] = (0.0, 0.0);
        n += 1;
    }
    let pts = &pts[..n];
    let v = v.clamp(pts[0].0, pts[n - 1].0);
    for w in pts.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if v >= x0 && v <= x1 {
            return if x1 > x0 { y0 + (y1 - y0) * (v - x0) / (x1 - x0) } else { y0 };
        }
    }
    0.0
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests_headroom {
    use super::*;

    fn maps() -> Vec<ToneMap> {
        let mut v =
            vec![ToneMap::new(0.0, 0.0, 0.0), ToneMap::new(60.0, -40.0, 30.0), ToneMap::display(0.0, 0.0, 0.0), ToneMap::display(-30.0, 50.0, 0.0)];
        if let Some(c) = rendered_reference() {
            v.push(ToneMap::camera_lr(&c, 0.5, 20.0, 30.0, -10.0));
        }
        v
    }

    #[test]
    fn headroom_keeps_midtones_reaches_peak_and_is_monotone() {
        for sdr in maps() {
            assert_eq!(sdr.clone().with_headroom(1.0).lut(), sdr.lut());
            assert_eq!(sdr.clone().with_headroom(f32::NAN).lut(), sdr.lut());
            for peak in [2.0f32, 4.92, 16.0] {
                let h = sdr.clone().with_headroom(peak);
                let mut prev = -1.0;
                for (a, b) in h.lut().iter().zip(sdr.lut()) {
                    assert!((0.0..=peak).contains(a) && *a >= prev - 1e-6 && *a >= *b - 1e-6);
                    if *b < HDR_KNEE * 0.98 {
                        assert!((a - b).abs() < 1e-3, "midtones moved: {b} -> {a}");
                    }
                    prev = *a;
                }
                assert!(h.lut().last().copied().unwrap_or(0.0) > peak * 0.99, "{peak}");
            }
        }
    }

    #[test]
    fn whites_move_where_the_headroom_fills() {
        let lo = ToneMap::new(0.0, -60.0, 0.0).with_headroom(8.0);
        let hi = ToneMap::new(0.0, 60.0, 0.0).with_headroom(8.0);
        let y = GREY * 2f32.powf(4.0);
        assert!(hi.apply(y) > lo.apply(y));
    }
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

    /// R02 (Camera Raw oracle): a uniform photo never moves; Shadows lifts what lies below the
    /// photo's pivot, more the further below, and leaves the bright part; Highlights − pulls the
    /// bright part down; the sliders add; hostile stats give a finite (or empty) table.
    #[test]
    fn camera_raw_highlights_shadows_follow_the_whole_photo() {
        let uniform = lr::stats_of(std::iter::repeat_n(-2.0f32, 700)).unwrap();
        assert!(lr::adobe_hs_lut(-1.0, 1.0, Some(uniform), 0.0).iter().all(|v| v.abs() < 1e-6), "a uniform photo is never changed");
        // two halves, 5 EV apart (stats before exposure; the table after it)
        let field: Vec<f32> = (0..1400).map(|i| if i % 2 == 0 { -6.0 } else { -1.0 }).collect();
        let st = lr::stats_of(field.iter().copied()).unwrap();
        assert!(st.lo <= -5.9 && st.hi >= -1.1 && st.lin_mean > -2.0, "{st:?}");
        let s = lr::adobe_hs_lut(0.0, 1.0, Some(st), 0.0);
        let (dark, bright) = (lr::hs_at(&s, -6.0), lr::hs_at(&s, -1.0));
        assert!(dark > 0.5 && bright.abs() < 0.05, "Shadows: dark {dark} bright {bright}");
        assert!(lr::hs_at(&s, -8.0) >= dark - 1e-4, "darker tones move with the darkest ones");
        let h = lr::adobe_hs_lut(-1.0, 0.0, Some(st), 0.0);
        assert!(lr::hs_at(&h, -1.0) < -0.3 && lr::hs_at(&h, -6.0).abs() < 0.05, "Highlights −");
        // exposure moves the whole table with the photo
        let e = lr::adobe_hs_lut(0.0, 1.0, Some(st), -1.0);
        assert!((lr::hs_at(&e, -7.0) - dark).abs() < 1e-4);
        let both = lr::adobe_hs_lut(-1.0, 1.0, Some(st), 0.0);
        for x in [-8.0f32, -6.0, -3.5, -1.0, 1.0] {
            assert!((lr::hs_at(&both, x) - lr::hs_at(&s, x) - lr::hs_at(&h, x)).abs() < 1e-5, "{x}");
        }
        assert!(lr::adobe_hs_lut(1.0, 1.0, None, 0.0).iter().all(|v| *v == 0.0));
        let bad = lr::ToneStats { key: f32::NAN, lin_mean: f32::INFINITY, lo: f32::NAN, hi: -1e30 };
        for ev in [0.0, f32::NAN, 1e9] {
            assert!(lr::adobe_hs_lut(-1.0, 1.0, Some(bad), ev).iter().all(|v| v.is_finite()));
            assert!(lr::adobe_hs_lut(-1.0, 1.0, Some(st), ev).iter().all(|v| v.is_finite()));
        }
        assert!(lr::stats_of([f32::NAN, f32::INFINITY].into_iter()).is_none());
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

    #[test]
    fn rendered_files_round_trip_through_the_reference_curve() {
        let curve = rendered_reference().unwrap();
        for y in [0.0005f32, 0.003, 0.02, 0.18, 0.5, 0.9, 0.99] {
            let back = curve.apply(curve.invert(y));
            assert!((back - y).abs() < 1e-4 * y.max(0.01), "{y} -> {back}");
        }
        // a colour keeps its hue and saturation, scaled to scene-like values
        let c = rendered_to_scene(&curve, [0.4, 0.2, 0.1]);
        assert!((c[0] / c[1] - 2.0).abs() < 1e-4 && (c[1] / c[2] - 2.0).abs() < 1e-4);
        // hostile values: finite, never negative, never a panic
        for v in [f32::NAN, f32::INFINITY, -1.0, 0.0, 1.0, 2.0] {
            let x = curve.invert(v);
            assert!(x.is_finite() && x >= 0.0, "{v} -> {x}");
            assert!(rendered_to_scene(&curve, [v; 3]).iter().all(|c| c.is_finite() && *c >= 0.0));
        }
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

    /// Bryan's fork: the measured Camera Raw tone stays finite and non-decreasing for any sliders,
    /// is exact at the measured values (zero sliders: the base table alone), and survives NaN.
    #[test]
    fn adobe_tone_tables_are_monotone_and_safe() {
        let base = crate::adobe::Base {
            hue_sat: None,
            hue_sat_ev: 0.0,
            look: None,
            curve: (0..=crate::adobe::CURVE_N).map(|i| (i as f32 / crate::adobe::CURVE_N as f32).sqrt()).collect(),
            black: 0.0,
        };
        for (e, c, w, b) in
            [(0.0, 0.0, 0.0, 0.0), (1.3, -63.0, -36.0, 30.0), (-2.7, -100.0, 15.0, 68.0), (4.0, 100.0, 100.0, -100.0), (-5.0, -100.0, -100.0, 100.0)]
        {
            for key in [None, Some(-6.0), Some(2.0), Some(f32::NAN)] {
                let t = ToneMap::adobe_lr(&base, e, c, w, b, key);
                assert!(t.lut().iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)), "{e} {c} {w} {b} {key:?}");
                assert!(t.lut().windows(2).all(|p| p[1] >= p[0] - 1e-4), "not monotone: {e} {c} {w} {b} {key:?}");
            }
        }
        let t = ToneMap::adobe_lr(&base, f64::NAN, f64::NAN, f64::INFINITY, -f64::INFINITY, None);
        assert!(t.lut().iter().all(|v| v.is_finite()));
        // between measured slider values the shift is linear in the slider
        let a = adobe_shift(&crate::tone_adobe::W_VALUES, &crate::tone_adobe::W_TABLE, 25.0, 1.0);
        let b2 = adobe_shift(&crate::tone_adobe::W_VALUES, &crate::tone_adobe::W_TABLE, 50.0, 1.0);
        let m = adobe_shift(&crate::tone_adobe::W_VALUES, &crate::tone_adobe::W_TABLE, 37.5, 1.0);
        assert!((m - (a + b2) / 2.0).abs() < 1e-5);
        assert_eq!(adobe_shift(&crate::tone_adobe::W_VALUES, &crate::tone_adobe::W_TABLE, 0.0, 1.0), 0.0);
    }
}
