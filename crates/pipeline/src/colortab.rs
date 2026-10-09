//! Lightroom's HSL, Saturation and Vibrance as measured with Camera Raw itself (Bryan's fork,
//! `tools/lr-compare/oracle`: a dense colour grid rendered by Camera Raw with each slider at ±50 /
//! ±100, `hslfit.py` + `gen_colortab.py`). Each slider stop is a table over the finished colour's
//! OkLCh (hue × lightness × chroma) of the change it makes: hue rotation (radians), ln chroma ratio
//! and lightness shift. A render sums its sliders' tables (interpolated between stops) into one
//! table, which [`lookup`] reads per pixel; Camera Raw's own combinations match that sum.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use lightcraft_develop::DevelopSettings;

/// Hue bins (periodic).
pub const NH: usize = 36;
/// OkLab lightness and chroma axes (clamped at the ends).
pub const L_AXIS: [f32; 5] = [0.2, 0.4, 0.6, 0.8, 1.0];
pub const C_AXIS: [f32; 4] = [0.0, 0.06, 0.14, 0.26];
const NL: usize = L_AXIS.len();
const NC: usize = C_AXIS.len();
/// Floats in one table: hue × lightness × chroma × (Δhue, ln chroma ratio, Δlightness).
pub const LEN: usize = NH * NL * NC * 3;
/// Hue × 8, Saturation × 8, Luminance × 8 (Red … Magenta), Saturation, Vibrance.
const NOPS: usize = 26;
const STOPS: [f64; 4] = [-100.0, -50.0, 50.0, 100.0];

static RAW: &[u8] = include_bytes!("colortab.bin");

/// The stored tables, `[op][stop]`; empty when the data can't be read (the caller then keeps the
/// older built-in colour tools).
fn stored() -> &'static [[Vec<f32>; 4]] {
    static T: OnceLock<Vec<[Vec<f32>; 4]>> = OnceLock::new();
    T.get_or_init(|| decode(RAW).unwrap_or_default())
}

fn decode(b: &[u8]) -> Option<Vec<[Vec<f32>; 4]>> {
    if b.get(..4)? != b"LCT2" || b.get(4..9)? != [NH as u8, NL as u8, NC as u8, NOPS as u8, STOPS.len() as u8] {
        return None;
    }
    let mut p = 9;
    let mut out = Vec::with_capacity(NOPS);
    for _ in 0..NOPS {
        let mut stops: [Vec<f32>; 4] = Default::default();
        for t in stops.iter_mut() {
            let (first, cnt) = (*b.get(p)? as usize, *b.get(p + 1)? as usize);
            let mut sc = [0f32; 3];
            for (k, s) in sc.iter_mut().enumerate() {
                *s = f32::from_le_bytes(b.get(p + 2 + 4 * k..p + 6 + 4 * k)?.try_into().ok()?);
            }
            p += 14;
            let run = NL * NC * 3;
            let data = b.get(p..p + cnt * run)?;
            p += cnt * run;
            let mut v = vec![0f32; LEN];
            for i in 0..cnt.min(NH) {
                let h = (first + i) % NH;
                for (j, &q) in data[i * run..(i + 1) * run].iter().enumerate() {
                    v[h * run + j] = q as i8 as f32 * sc[j % 3];
                }
            }
            *t = v;
        }
        out.push(stops);
    }
    (p == b.len()).then_some(out)
}

/// The summed table for `s`'s HSL / Saturation / Vibrance sliders; `None` when they are all zero
/// (or the measured data is unavailable).
pub fn for_settings(s: &DevelopSettings) -> Option<Vec<f32>> {
    let t = stored();
    if t.len() != NOPS {
        return None;
    }
    let b = s.mixer.bands();
    let mut v = [0f64; NOPS];
    for i in 0..8 {
        v[i] = b[i].hue;
        v[8 + i] = b[i].sat;
        v[16 + i] = b[i].lum;
    }
    v[24] = s.color.saturation;
    v[25] = s.color.vibrance;
    if v.iter().all(|x| *x == 0.0) {
        return None;
    }
    let mut acc = vec![0f32; LEN];
    for (op, &x) in v.iter().enumerate() {
        if x == 0.0 || !x.is_finite() {
            continue;
        }
        let x = x.clamp(-100.0, 100.0);
        // stops −100, −50, (0), +50, +100: blend the two around x (0 is the identity)
        let (lo, hi, f) = if x < -50.0 {
            (Some(0), Some(1), (x + 100.0) / 50.0)
        } else if x < 0.0 {
            (Some(1), None, (x + 50.0) / 50.0)
        } else if x <= 50.0 {
            (None, Some(2), x / 50.0)
        } else {
            (Some(2), Some(3), (x - 50.0) / 50.0)
        };
        let f = f as f32;
        for (i, a) in acc.iter_mut().enumerate() {
            let l = lo.map_or(0.0, |k| t[op][k][i]);
            let h = hi.map_or(0.0, |k| t[op][k][i]);
            *a += l + (h - l) * f;
        }
    }
    Some(acc)
}

#[inline]
fn axis(a: &[f32], x: f32) -> (usize, f32) {
    let n = a.len();
    let x = x.clamp(a[0], a[n - 1]);
    let mut i = 0;
    while i + 2 < n && x > a[i + 1] {
        i += 1;
    }
    (i, (x - a[i]) / (a[i + 1] - a[i]))
}

/// (Δhue, ln chroma ratio, Δlightness) for an OkLCh colour (hue in radians) from a summed table.
#[inline]
pub fn lookup(t: &[f32], l: f32, c: f32, h: f32) -> [f32; 3] {
    if t.len() != LEN || !(l.is_finite() && c.is_finite() && h.is_finite()) {
        return [0.0; 3];
    }
    let u = (h / TAU).rem_euclid(1.0) * NH as f32;
    let h0 = (u.floor() as usize) % NH;
    let fh = u - u.floor();
    let (l0, fl) = axis(&L_AXIS, l);
    let (c0, fc) = axis(&C_AXIS, c);
    let mut out = [0f32; 3];
    for (dh, wh) in [(0, 1.0 - fh), (1, fh)] {
        for (dl, wl) in [(0, 1.0 - fl), (1, fl)] {
            for (dc, wc) in [(0, 1.0 - fc), (1, fc)] {
                let w = wh * wl * wc;
                let k = ((((h0 + dh) % NH) * NL + l0 + dl) * NC + c0 + dc) * 3;
                for j in 0..3 {
                    out[j] += w * t[k + j];
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_tables_decode() {
        assert_eq!(stored().len(), NOPS);
    }

    #[test]
    fn sliders_act_where_lightroom_does() {
        let mut s = DevelopSettings::default();
        assert!(for_settings(&s).is_none());
        // Green hue +100 turns greens towards aqua, leaves reds alone
        s.mixer.green.hue = 100.0;
        let t = for_settings(&s).unwrap();
        let green = lightcraft_color::perceptual::lab_to_lch(lightcraft_color::perceptual::oklab_from_2020([0.05, 0.3, 0.05]));
        let red = lightcraft_color::perceptual::lab_to_lch(lightcraft_color::perceptual::oklab_from_2020([0.3, 0.04, 0.03]));
        assert!(lookup(&t, green[0], green[1], green[2])[0] > 0.2);
        assert!(lookup(&t, red[0], red[1], red[2])[0].abs() < 0.01);
        // Saturation −100 leaves (almost) no chroma
        let mut s = DevelopSettings::default();
        s.color.saturation = -100.0;
        let t = for_settings(&s).unwrap();
        assert!(lookup(&t, red[0], red[1], red[2])[1] < -2.0);
    }

    #[test]
    fn malformed_data_is_refused() {
        assert!(decode(b"LCT2").is_none());
        assert!(decode(&RAW[..RAW.len() - 1]).is_none());
    }
}
