//! Negative Dehaze as Camera Raw renders it (R21, Bryan's fork; `docs/lr-match.md` → Round 10).
//!
//! Ported from RAWmakase (`crates/rawmakase-engine/src/develop/local_tone.rs` `family`, its
//! `monotone` projection, and the measured `DEHAZE` family of `local_tone_data.rs`;
//! <https://github.com/pch/rawmakase>, MIT, Copyright (c) 2026 RAWmakase contributors; see NOTICE),
//! fitted to Camera Raw 18.7. Negative Dehaze is a tone curve on each channel of linear ProPhoto,
//! by the channel's level relative to the photo's 99th luminance percentile (plus Exposure):
//! the log2 gain is read from tables at Dehaze −100 / −40 / −20 (linear between them and towards
//! no change at 0) over levels −12 … +1 EV from that key. Our oracle measured the same shape: at
//! −25 a uniform ×0.75 of contrast with darker levels lifted most, the same at every scale.

/// Use this model for negative Dehaze (global and in masks).
pub const MEASURED: bool = true;

const VALUES: [f32; 6] = [-1., -0.4, -0.2, 0.2, 0.4, 1.];
const LO: f32 = -12.0;
const HI: f32 = 1.0;
/// Levels below this are read at it (RAWmakase's `FLOOR`, 2^-14).
const FLOOR: f32 = 6.103_515_6e-5;

#[rustfmt::skip]
const TABLES: [[f32; 48]; 6] = [
    [3.4917, 3.4979, 3.5041, 3.5104, 3.5166, 3.5228, 3.5291, 3.5353, 3.5415, 3.5477, 3.5539, 3.5568, 3.5413, 3.4787, 3.3115, 3.0159, 2.6718, 2.4367, 2.2560, 2.1242, 2.1061, 2.1549, 2.1596, 2.1024, 2.0404, 1.9908, 1.9948, 1.9993, 1.9527, 1.8830, 1.8087, 1.7310, 1.6146, 1.5180, 1.4454, 1.3416, 1.2635, 1.1568, 1.0805, 0.9805, 0.8527, 0.7195, 0.5092, 0.2729, 0.1253, 0.1509, 0.0756, -0.1455],
    [1.0912, 1.1079, 1.1246, 1.1413, 1.1580, 1.1748, 1.1915, 1.2082, 1.2249, 1.2416, 1.2583, 1.2739, 1.2820, 1.2664, 1.2041, 1.0959, 0.9642, 0.8713, 0.7943, 0.7406, 0.7431, 0.7846, 0.8048, 0.7953, 0.7826, 0.7704, 0.7782, 0.7776, 0.7598, 0.7347, 0.7074, 0.6808, 0.6334, 0.5962, 0.5699, 0.5293, 0.4993, 0.4587, 0.4294, 0.3902, 0.3392, 0.2883, 0.2039, 0.1137, 0.0532, 0.0593, 0.0283, -0.0575],
    [0.6780, 0.6750, 0.6721, 0.6691, 0.6661, 0.6632, 0.6602, 0.6573, 0.6543, 0.6514, 0.6484, 0.6447, 0.6386, 0.6249, 0.5959, 0.5502, 0.4854, 0.4314, 0.3864, 0.3607, 0.3658, 0.3877, 0.3977, 0.3910, 0.3809, 0.3717, 0.3755, 0.3759, 0.3626, 0.3501, 0.3388, 0.3297, 0.3078, 0.2910, 0.2800, 0.2605, 0.2462, 0.2274, 0.2136, 0.1946, 0.1693, 0.1441, 0.1017, 0.0572, 0.0271, 0.0291, 0.0127, -0.0306],
    [0.1328, 0.1061, 0.0793, 0.0526, 0.0258, -0.0010, -0.0277, -0.0545, -0.0812, -0.1080, -0.1347, -0.1615, -0.1874, -0.2085, -0.2150, -0.2015, -0.1841, -0.1711, -0.1588, -0.1515, -0.1622, -0.1848, -0.1946, -0.1948, -0.1893, -0.1749, -0.1698, -0.1718, -0.1673, -0.1605, -0.1542, -0.1488, -0.1397, -0.1358, -0.1332, -0.1203, -0.1181, -0.1164, -0.1267, -0.1288, -0.1303, -0.1553, -0.1273, -0.0876, -0.0430, -0.0400, -0.0265, 0.0069],
    [0.5472, 0.4793, 0.4115, 0.3436, 0.2757, 0.2079, 0.1400, 0.0722, 0.0043, -0.0635, -0.1314, -0.1993, -0.2669, -0.3284, -0.3705, -0.3718, -0.3425, -0.3121, -0.2880, -0.2771, -0.3060, -0.3712, -0.4009, -0.3985, -0.3879, -0.3610, -0.3443, -0.3486, -0.3471, -0.3306, -0.3245, -0.3103, -0.2923, -0.2862, -0.2804, -0.2508, -0.2465, -0.2429, -0.2674, -0.2725, -0.2782, -0.3352, -0.2697, -0.1820, -0.0928, -0.0866, -0.0596, 0.0109],
    [1.5139, 1.3648, 1.2157, 1.0666, 0.9175, 0.7684, 0.6193, 0.4702, 0.3211, 0.1720, 0.0229, -0.1263, -0.2745, -0.4141, -0.5277, -0.5927, -0.6392, -0.7127, -0.6795, -0.6096, -0.7216, -0.9731, -1.0807, -1.1210, -1.1462, -1.1115, -1.0462, -1.0240, -1.0122, -0.9857, -0.9890, -0.9750, -0.8771, -0.8607, -0.8664, -0.7553, -0.7529, -0.7319, -0.8617, -0.8765, -0.8850, -1.0829, -0.8554, -0.5386, -0.2762, -0.2647, -0.1875, 0.0132],
];

/// Round 13's spatial part (`docs/lr-match.md` → Round 13). On flat fields, full-frame gratings, edges and lines
/// Camera Raw's negative Dehaze leaves a flat image exactly as it is, keeps ×(1 + s) of the contrast of detail at
/// every scale, and lifts a region's mean only by how far the region's *blurred* level sits below the photo's
/// brightest blurred level (fine gratings keep their mean). So the curve is read at each channel's level less the
/// luminance detail — log luminance less its Gaussian of σ [`SIGMA`] × the long edge — the detail itself is scaled
/// by (1 + s), and the key is the [`KEY_PERCENTILE`]th percentile of the blurred luminance. Probes 2.94 → 1.95 ΔE,
/// Bryan's raws at −50 3.50 → 3.33.
pub const SPATIAL: bool = true;
/// σ of the regional level, as a fraction of the long edge.
pub const SIGMA: f32 = 0.004;
/// The key's percentile of the regional level.
pub const KEY_PERCENTILE: f32 = 99.9;

/// The key (log2, before exposure) from the regional level's values ([`crate::local`]'s `log_l` blurred, over
/// [`crate::tone::GREY`]), every `step`-th value; `None` without finite values.
pub fn key_of(values: impl Iterator<Item = f32>) -> Option<f32> {
    if !SPATIAL {
        return p99_of(values);
    }
    let mut v: Vec<f32> = values.filter(|v| v.is_finite()).map(|v| v.clamp(-20.0, 10.0)).collect();
    if v.is_empty() {
        return None;
    }
    let n = v.len();
    let i = ((KEY_PERCENTILE / 100.0 * n as f32) as usize).min(n - 1);
    Some(*v.select_nth_unstable_by(i, f32::total_cmp).1 + crate::tone::GREY.log2())
}

/// log2 gain of a channel at `level` whose pixel's luminance detail (log luminance less its regional level) is
/// `detail`: the curve at the regional level, plus the detail scaled by (1 + s).
#[inline]
pub fn gain_spatial(s: f32, key: f32, level: f32, detail: f32) -> f32 {
    if s.is_nan() || s >= 0.0 || !detail.is_finite() {
        return gain(s, key, level);
    }
    gain(s, key, level - detail) + s.max(-1.0) * detail
}

/// The tables after RAWmakase's `monotone` projection: output log luminance (input + gain)
/// never decreases along a row (pool adjacent violators, done once).
fn tables() -> &'static [[f32; 48]; 6] {
    static T: std::sync::OnceLock<[[f32; 48]; 6]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = TABLES;
        let step = (HI - LO) / 48.;
        for row in t.iter_mut() {
            let mut sums = [0f32; 48];
            let mut counts = [0usize; 48];
            let mut blocks = 0usize;
            for (i, v) in row.iter().enumerate() {
                sums[blocks] = v + i as f32 * step;
                counts[blocks] = 1;
                blocks += 1;
                while blocks > 1 && sums[blocks - 2] / counts[blocks - 2] as f32 > sums[blocks - 1] / counts[blocks - 1] as f32 {
                    sums[blocks - 2] += sums[blocks - 1];
                    counts[blocks - 2] += counts[blocks - 1];
                    blocks -= 1;
                }
            }
            let mut i = 0;
            for b in 0..blocks {
                let end = i + counts[b];
                while i < end {
                    if counts[b] > 1 {
                        row[i] = sums[b] / counts[b] as f32 - i as f32 * step;
                    }
                    i += 1;
                }
            }
        }
        t
    })
}

/// The tables flattened for the GPU kernel (6 × 48, row-major).
pub fn words() -> Vec<f32> {
    tables().iter().flatten().copied().collect()
}

/// log2 gain of a channel at `level` (log2 of its linear value) for Dehaze `s` (−1..0; positive
/// values are LightCraft's haze removal) relative to `key` (the photo's 99th luminance percentile
/// plus Exposure, log2).
#[inline]
pub fn gain(s: f32, key: f32, level: f32) -> f32 {
    if s.is_nan() || s >= 0.0 || !key.is_finite() || !level.is_finite() {
        return 0.0;
    }
    let s = s.max(-1.0);
    let t = tables();
    let x = ((level - key - LO) / (HI - LO) * 48. - 0.5).clamp(0., 47.);
    let i = (x as usize).min(46);
    let bin = |r: usize| t[r][i] + (t[r][i + 1] - t[r][i]) * (x - i as f32);
    // the measured positions below 0, then the identity at 0
    let (j, w) = if s <= VALUES[1] {
        (0, (s - VALUES[0]) / (VALUES[1] - VALUES[0]))
    } else if s <= VALUES[2] {
        (1, (s - VALUES[1]) / (VALUES[2] - VALUES[1]))
    } else {
        let y0 = bin(2);
        return y0 + (0.0 - y0) * (s - VALUES[2]) / (0.0 - VALUES[2]);
    };
    let (y0, y1) = (bin(j), bin(j + 1));
    y0 + (y1 - y0) * w
}

/// The level of a linear channel value ([`gain`]'s `level`).
#[inline]
pub fn level(v: f32) -> f32 {
    v.max(FLOOR).log2()
}

/// The photo's 99th percentile of log2 luminance (linear, before exposure) from LightCraft's
/// `log_l` (log2 of luminance over [`crate::tone::GREY`]), every `step`-th value.
pub fn p99_of(values: impl Iterator<Item = f32>) -> Option<f32> {
    let mut v: Vec<f32> = values.filter(|v| v.is_finite()).map(|v| v.clamp(-20.0, 10.0)).collect();
    if v.is_empty() {
        return None;
    }
    let n = v.len();
    let i = ((0.99 * n as f32) as usize).min(n - 1);
    Some(*v.select_nth_unstable_by(i, f32::total_cmp).1 + crate::tone::GREY.log2())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_dehaze_flattens_and_lifts_the_darks() {
        let key = -1.0;
        // −40: darker levels lifted more than bright ones; 0 and positive change nothing
        let (dark, bright) = (gain(-0.4, key, -8.0), gain(-0.4, key, -1.5));
        assert!(dark > bright && dark > 0.3, "{dark} {bright}");
        assert_eq!(gain(0.0, key, -4.0), 0.0);
        assert_eq!(gain(0.3, key, -4.0), 0.0);
        // stronger with the amount, finite at the edges
        assert!(gain(-1.0, key, -8.0) > gain(-0.2, key, -8.0));
        assert!(gain(-1.0, f32::NAN, -8.0) == 0.0 && gain(-5.0, key, -100.0).is_finite());
        assert_eq!(words().len(), 6 * 48);
        assert!(p99_of([f32::NAN].into_iter()).is_none());
    }
}
