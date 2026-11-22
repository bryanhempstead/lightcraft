//! Camera Raw's default black point as it follows the photo (R02, Bryan's fork; `docs/lr-match.md`
//! → Round 12).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/scene_tone/global.rs`:
//! `default_black`, `black_correction`, `without_black`, `evaluate`, and its measured
//! `default_black.bin`, here `black_cr.bin`; `pipeline/tone.rs`: the Shadows shift of the key;
//! <https://github.com/pch/rawmakase>, MIT, Copyright (c) 2026 RAWmakase contributors; see NOTICE),
//! fitted to Camera Raw 18.7 renders of neutral ramps.
//!
//! A profile whose DefaultBlackRender is Auto (Adobe's camera profiles) gets a black point that
//! follows the photo's *dark key*, the 0.1th percentile of its scene luminance (log2, with
//! Exposure): the darker the photo's darkest tones, the gentler the toe (`black_cr.bin`: the output
//! with the default black against the output without it, per key from −14 to −7 EV, over log2
//! outputs −16 … 0). Positive Shadows lifts the darks, and Camera Raw reads the key after it (moved
//! by the Shadows gain's median over the photo; negative Shadows: its median on the darkest
//! 0.1 %), so the toe deepens as Shadows lifts — the lift Camera Raw shows in the deepest tones is
//! smaller than the rule's gain alone (round 12: Bryan's Canon R6 photos, Shadows +100 lifted
//! their darkest band 25.7 L* in Camera Raw, 32.7 in LightCraft without this).
//!
//! LightCraft's Basic-tone tables (`tone_adobe`) were measured on a ramp whose dark key is
//! [`RAMP_KEY`] (plus Exposure), so the black response they hold is that key's; [`correction`]
//! swaps it for the photo's, on the tone map's input (the value the profile's tone curve reads).

/// Follow the photo's dark key (and the Shadows shift of it).
pub const MEASURED: bool = true;
/// Only the Shadows shift (the photo's own key replaced by itself): the base keeps the ramp's
/// response. Off: the photo's key replaces the ramp's for every render.
pub const SHIFT_ONLY: bool = false;

/// How the default black follows the photo ([`mode`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The ramp's response for every photo (LightCraft before round 12).
    Off,
    /// Only the Shadows shift of the photo's key.
    Shift,
    /// The photo's key (with the Shadows shift) in place of the ramp's.
    Photo,
}

/// The mode renders use: [`MEASURED`] / [`SHIFT_ONLY`], or `LIGHTCRAFT_BLACK_CR=0|shift|photo` in
/// the environment (measurement runs), read once.
pub fn mode() -> Mode {
    static M: std::sync::OnceLock<Mode> = std::sync::OnceLock::new();
    *M.get_or_init(|| match std::env::var("LIGHTCRAFT_BLACK_CR").as_deref() {
        Ok("0") => Mode::Off,
        Ok("shift") => Mode::Shift,
        Ok("photo") => Mode::Photo,
        _ if !MEASURED => Mode::Off,
        _ if SHIFT_ONLY => Mode::Shift,
        _ => Mode::Photo,
    })
}

static DATA: &[u8] = include_bytes!("black_cr.bin");
/// The dark keys `black_cr.bin` was measured at (−14 to −7 in half stops).
const KEYS: usize = 15;
const KEY_FIRST: f32 = -14.0;
const KEY_STEP: f32 = 0.5;
/// log2 outputs of the samples: −16 to 0 in eighth stops.
pub const SAMPLES: usize = 129;
const Y_FIRST: f32 = -16.0;
const Y_STEP: f32 = 0.125;
/// The dark key of the ramp `tone_adobe` was measured on (its darkest patch, −14 EV, at Exposure 0).
pub const RAMP_KEY: f32 = -14.0;
/// Positive Blacks removes the default black by +25 (RAWmakase: +25 renders as no default black at 0).
const BLACK_REMOVED: f32 = 0.25;

fn row(key: usize, k: usize) -> f32 {
    let i = (key * SAMPLES + k) * 4;
    match DATA.get(i..i + 4) {
        Some(b) => {
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            if v.is_finite() { v.max(0.0) } else { 0.0 }
        }
        None => (Y_FIRST + k as f32 * Y_STEP).exp2(),
    }
}

/// The measured default-black transfer at dark key `key` (clamped to the measured keys), sample `k`.
fn default_black(key: f32, k: usize) -> f32 {
    let key = if key.is_finite() { key } else { KEY_FIRST };
    let f = ((key - KEY_FIRST) / KEY_STEP).clamp(0.0, (KEYS - 1) as f32);
    let i = (f as usize).min(KEYS - 2);
    let t = f - i as f32;
    row(i, k) * (1.0 - t) + row(i + 1, k) * t
}

/// A curve sampled at log2 inputs `Y_FIRST + k · Y_STEP`: linear below the first sample, constant above the last.
#[inline]
pub fn evaluate(samples: &[f32], x: f32) -> f32 {
    let (Some(&first), Some(&last)) = (samples.first(), samples.last()) else { return x };
    let low = Y_FIRST.exp2();
    if !x.is_finite() || x <= low {
        return if x.is_finite() { x * first / low } else { 0.0 };
    }
    let f = (x.log2() - Y_FIRST) / Y_STEP;
    let n = samples.len();
    if f >= (n - 1) as f32 {
        return last;
    }
    let i = f as usize;
    let t = f - i as f32;
    let a = samples.get(i).copied().unwrap_or(last);
    let b = samples.get(i + 1).copied().unwrap_or(last);
    a * (1.0 - t) + b * t
}

/// The value without a default black that `calibration` maps to `y`.
fn without_black(calibration: &[f32; SAMPLES], y: f32) -> f32 {
    if y < calibration[0] {
        return y / calibration[0].max(1e-12) * Y_FIRST.exp2();
    }
    let j = calibration.partition_point(|v| *v < y).clamp(1, SAMPLES - 1);
    let t = ((y - calibration[j - 1]) / (calibration[j] - calibration[j - 1]).max(1e-12)).clamp(0.0, 1.0);
    (Y_FIRST + (j as f32 - 1.0 + t) * Y_STEP).exp2()
}

/// The share of the default black Blacks `blacks` (−1..1) leaves: all of it at 0 or below, none from
/// +25 (fading out faster near +25: RAWmakase measured 0.90 left at +12.5 on a key of −8).
pub fn remaining(blacks: f32) -> f32 {
    if !blacks.is_finite() || blacks <= 0.0 {
        return 1.0;
    }
    1.0 - (blacks / BLACK_REMOVED).min(1.0).powi(3)
}

/// The tone map's input `y` (0..1, 1 = white) with the black response of dark key `from` replaced
/// by that of `to`, at `share` of the difference, as samples over log2 outputs ([`evaluate`]).
/// `None` when the two are the same.
pub fn correction(to: f32, from: f32, share: f32) -> Option<Box<[f32; SAMPLES]>> {
    let share = if share.is_finite() { share.clamp(0.0, 1.0) } else { 0.0 };
    let last = KEY_FIRST + (KEYS - 1) as f32 * KEY_STEP;
    if !to.is_finite() || !from.is_finite() || share == 0.0 || (to.clamp(KEY_FIRST, last) - from.clamp(KEY_FIRST, last)).abs() < 1e-4 {
        return None;
    }
    let cal: [f32; SAMPLES] = std::array::from_fn(|k| default_black(from, k));
    let want: [f32; SAMPLES] = std::array::from_fn(|k| default_black(to, k));
    let mut out: [f32; SAMPLES] = std::array::from_fn(|k| {
        let y = (Y_FIRST + k as f32 * Y_STEP).exp2();
        let x = without_black(&cal, y);
        (y + share * (evaluate(&want, x) - evaluate(&cal, x))).max(0.0)
    });
    for k in 1..SAMPLES {
        out[k] = out[k].max(out[k - 1]);
    }
    Some(Box::new(out))
}

/// The dark key of a photo from its map's log2 luminances (the 0.1th percentile; RAWmakase reads
/// it on its 512 px measurement copy too), before Exposure.
pub fn dark_of(logs: &[f32]) -> Option<f32> {
    let mut v: Vec<f32> = logs.iter().copied().filter(|v| v.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    let k = (v.len() - 1) / 1000;
    Some(*v.select_nth_unstable_by(k, f32::total_cmp).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_whole_and_monotone() {
        assert_eq!(DATA.len(), KEYS * SAMPLES * 4);
        for key in 0..KEYS {
            let r: Vec<f32> = (0..SAMPLES).map(|k| row(key, k)).collect();
            assert!(r.windows(2).all(|p| p[1] + 1e-7 >= p[0]), "key {key}");
            assert!((r[SAMPLES - 1] - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn a_brighter_dark_key_deepens_the_toe() {
        assert!(correction(-12.0, -12.0, 1.0).is_none());
        let c = correction(-9.0, -14.0, 1.0).unwrap();
        // a dark tone renders darker, white stays white, nothing reverses
        let y = 2f32.powi(-9);
        assert!(evaluate(&c[..], y) < 0.8 * y, "{}", evaluate(&c[..], y));
        assert!((evaluate(&c[..], 1.0) - 1.0).abs() < 1e-3);
        assert!(c.windows(2).all(|p| p[1] >= p[0]));
        // half the share moves half as far; positive Blacks takes the default black away
        let h = correction(-9.0, -14.0, 0.5).unwrap();
        assert!(evaluate(&h[..], y) > evaluate(&c[..], y) && evaluate(&h[..], y) < y);
        assert_eq!(remaining(0.3), 0.0);
        assert_eq!(remaining(-0.5), 1.0);
        assert!(remaining(0.125) > 0.85);
    }

    #[test]
    fn hostile_input_is_harmless() {
        assert!(correction(f32::NAN, -14.0, 1.0).is_none());
        assert!(correction(-9.0, -14.0, f32::NAN).is_none());
        assert_eq!(dark_of(&[]), None);
        assert_eq!(dark_of(&[f32::NAN, -3.0]), Some(-3.0));
        let c = correction(40.0, -40.0, 1.0).unwrap();
        assert!(c.iter().all(|v| v.is_finite()));
        assert_eq!(evaluate(&c[..], f32::NAN), 0.0);
    }
}
