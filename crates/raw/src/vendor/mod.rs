//! Vendor raw formats (TIFF-based) and helpers shared by them.

pub mod arw;
pub mod cr2;
pub mod cr3;
pub mod crx;
pub(crate) mod crx_wavelet;
pub mod nef;
pub mod nefc;
pub mod orf;
pub mod pef;
pub mod raf;
mod rafc;
pub mod rw2;

use crate::{BlackLevel, Rect};
use std::ops::Range;

/// Black level per 2×2 CFA position (anchored at the active area origin) from masked sensor columns `cols` over
/// rows `rows`. Falls back to 0 when the region is empty.
pub(crate) fn black_from_columns(data: &[u16], width: usize, cols: Range<usize>, rows: Range<usize>, active: Rect) -> BlackLevel {
    let height = data.len() / width.max(1);
    let (mut sum, mut n) = ([0f64; 4], [0u64; 4]);
    for y in rows.start..rows.end.min(height) {
        let py = (y as isize - active.y as isize).rem_euclid(2) as usize;
        for x in cols.start..cols.end.min(width) {
            let px = (x as isize - active.x as isize).rem_euclid(2) as usize;
            sum[py * 2 + px] += data[y * width + x] as f64;
            n[py * 2 + px] += 1;
        }
    }
    if n.contains(&0) {
        return BlackLevel::uniform(0.0);
    }
    BlackLevel { repeat_rows: 2, repeat_cols: 2, values: (0..4).map(|i| (sum[i] / n[i] as f64) as f32).collect(), delta_h: vec![], delta_v: vec![] }
}

/// The part of the masked columns `cols` that is really black: Canon's `SensorInfo` left border
/// can lie past the first image columns (the EOS Rebel T3 / 1100D's image starts ~10 columns
/// before the border it reports; those columns lifted its black level by up to ~230 of 16383,
/// varying with the scene, and crushed the shadows). The reference is the median column mean of
/// the left half; the range ends two columns before the first column that leaves it by more than
/// a noise margin, so a clean masked area is kept whole.
pub(crate) fn black_columns(data: &[u16], width: usize, cols: Range<usize>, rows: Range<usize>) -> Range<usize> {
    let height = data.len() / width.max(1);
    let (r0, r1) = (rows.start.min(height), rows.end.min(height));
    let c1 = cols.end.min(width);
    if cols.start >= c1 || r0 >= r1 {
        return cols.start..cols.start;
    }
    let step = ((r1 - r0) / 512).max(1);
    let mean = |x: usize| {
        let (mut s, mut n) = (0f64, 0u32);
        for y in (r0..r1).step_by(step) {
            if let Some(&v) = data.get(y * width + x) {
                s += f64::from(v);
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { s / f64::from(n) }
    };
    let means: Vec<f64> = (cols.start..c1).map(mean).collect();
    let mut left: Vec<f64> = means.iter().take(means.len().div_ceil(2)).copied().collect();
    left.sort_by(f64::total_cmp);
    let Some(&reference) = left.get(left.len() / 2) else { return cols.start..cols.start };
    let mut dev: Vec<f64> = left.iter().map(|m| (m - reference).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let spread = dev.get(dev.len() / 2).copied().unwrap_or(0.0);
    let tol = (8.0 * spread).max(16.0);
    match means.iter().position(|m| (m - reference).abs() > tol) {
        Some(i) => cols.start..(cols.start + i).saturating_sub(2).max(cols.start),
        None => cols.start..c1,
    }
}

/// White level estimate: the saturation plateau if a noticeable number of samples sit at one top
/// value, otherwise the full `bits` range. The plateau is looked for under a few stray samples
/// above it (hot pixels: the EOS Rebel T3 saturates at 15698 with a handful of samples up to
/// 16353, which hid the plateau, left white at 16383 and turned clipped highlights pink).
pub(crate) fn white_from_data(data: &[u16], bits: u32) -> f32 {
    let full = ((1u32 << bits.clamp(1, 16)) - 1) as f32;
    let step = (data.len() / 2_000_000).max(1);
    let mut hist = vec![0u32; 1 << 16];
    let mut seen = 0usize;
    for &v in data.iter().step_by(step) {
        // samples above the nominal range are padding / invalid (e.g. 0xFFFF fill), not saturation
        if v as f32 <= full {
            hist[v as usize] += 1;
            seen += 1;
        }
    }
    // the highest value held by more than a few stray samples
    let stray = (seen / 200_000).max(3) as u32;
    let Some(mx) = (0..hist.len()).rev().find(|&v| hist[v] > stray).map(|v| v as u16) else { return full };
    let band = (mx / 256).max(1);
    let near = mx.saturating_sub(band);
    let below = near.saturating_sub(band);
    let count = |r: std::ops::RangeInclusive<u16>| r.map(|v| hist[v as usize] as usize).sum::<usize>();
    let top = count(near..=mx);
    let under = if near > below { count(below..=near - 1) } else { 0 };
    // a saturation plateau is a spike: many more samples in the top band than in the band just below it
    if mx as f32 > full * 0.5 && top * step >= (data.len() / 20_000).max(16) && top >= 4 * under + 4 {
        // plateau: use its lower edge so everything at saturation maps to ≥ 1.0
        near as f32
    } else {
        full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_columns_by_parity() {
        // 8 wide, masked columns 0..4, active starts at x=4 (even)
        let data: Vec<u16> = (0..8 * 6)
            .map(|i| {
                let (x, y) = (i % 8, i / 8);
                if x < 4 { [100, 101, 102, 103][(y % 2) * 2 + x % 2] } else { 5000 }
            })
            .collect();
        let b = black_from_columns(&data, 8, 0..4, 0..6, Rect::new(4, 0, 4, 6));
        assert_eq!(b.values, vec![100.0, 101.0, 102.0, 103.0]);
        let b = black_from_columns(&data, 8, 0..4, 0..6, Rect::new(3, 1, 4, 5));
        assert_eq!(b.at(0, 0, 0, 1), 103.0);
        assert_eq!(black_from_columns(&data, 8, 0..0, 0..6, Rect::new(0, 0, 8, 6)), BlackLevel::uniform(0.0));
    }

    #[test]
    fn white_plateau() {
        let mut d: Vec<u16> = (0..100_000).map(|i| (i % 10000) as u16).collect();
        assert_eq!(white_from_data(&d, 14), 16383.0);
        d.extend(std::iter::repeat_n(15000u16, 1000));
        let w = white_from_data(&d, 14);
        assert!((14900.0..=15000.0).contains(&w), "{w}");
        assert_eq!(white_from_data(&[], 12), 4095.0);
    }

    #[test]
    fn white_plateau_under_stray_hot_pixels() {
        // Rebel T3: saturation at 15698, a few hot pixels above it; the plateau was missed and
        // white stayed at the 14-bit maximum
        let mut d: Vec<u16> = (0..400_000).map(|i| 2048 + (i % 9000) as u16).collect();
        d.extend(std::iter::repeat_n(15698u16, 20_000));
        d.extend([16353u16, 16200, 16011]);
        let w = white_from_data(&d, 14);
        assert!((15600.0..=15698.0).contains(&w), "{w}");
    }

    #[test]
    fn black_columns_stop_before_image_data() {
        // masked columns 0..61 black (2048 ± 1), image from 61 on, as on the Rebel T3
        let (w, h) = (90usize, 40usize);
        let data: Vec<u16> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                if x < 61 { 2047 + ((x + y) % 3) as u16 } else { 3600 + (x as u16) * 10 }
            })
            .collect();
        let cols = black_columns(&data, w, 2..78, 0..h);
        assert_eq!(cols, 2..59);
        let b = black_from_columns(&data, w, cols, 0..h, Rect::new(80, 0, 10, h));
        assert!(b.values.iter().all(|v| (2047.0..=2049.0).contains(v)), "{:?}", b.values);
        // the old range took the image columns in
        let old = black_from_columns(&data, w, 2..78, 0..h, Rect::new(80, 0, 10, h));
        assert!(old.values.iter().all(|v| *v > 2300.0), "{:?}", old.values);
        // a clean masked area is kept whole; empty input stays empty
        let clean: Vec<u16> = (0..w * h).map(|i| if i % w < 78 { 512 + (i % 5) as u16 } else { 9000 }).collect();
        assert_eq!(black_columns(&clean, w, 2..76, 0..h), 2..76);
        assert_eq!(black_columns(&clean, w, 5..5, 0..h), 5..5);
        assert_eq!(black_columns(&[], 0, 0..4, 0..4), 0..0);
    }
}
