//! The HDR histogram (LR-VIEW-HISTOGRAM with an HDR edit): Lightroom splits the histogram of a
//! photo edited in HDR into an SDR section (black to SDR white, by the sRGB-encoded value as the
//! ordinary histogram bins it) and an HDR section to its right (stops above SDR white, up to the
//! edit's HDR Limit, a dashed line at each whole stop), and marks what lies beyond the display's
//! current headroom. The layout follows RAWmakase's write-up of Lightroom Classic 13
//! (`rendered::HdrHistogram`, MIT): SDR the left 40 %, 32 bins a stop above white.
//!
//! The histogram describes the edit, not the display: it is binned from the HDR rendition at the
//! edit's own limit, and the display's headroom is applied only where it is drawn
//! ([`HdrHistogram::above`]).

use crate::{DeepImage, DeepSamples};

/// Bins per stop above SDR white.
pub const BINS_PER_STOP: usize = 32;
/// Stops above SDR white the histogram holds (the largest HDR Limit, `Hdr::MAX_EV_LIMIT`).
pub const STOPS: usize = 5;
const HDR_BINS: usize = BINS_PER_STOP * STOPS;
/// Channels up to this count as SDR white (a primaries conversion's rounding).
const WHITE_ROUNDING: f32 = 1.001;
/// Share of the HDR Limit's peak from which a channel counts as at the limit (flattened).
const AT_LIMIT: f32 = 0.995;
/// Share of the width the SDR section takes (Lightroom's layout).
pub const SDR_SHARE: f32 = 0.4;

/// An HDR rendition's histogram: per channel (linear output primaries, negative values as 0),
/// values up to SDR white by their sRGB-encoded value, values above it by stops above white.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrHistogram {
    /// 256 bins per channel from black to SDR white, by sRGB-encoded value.
    pub sdr: [[u32; 256]; 3],
    /// [`BINS_PER_STOP`] bins per stop above SDR white: bin `i` holds 2^(i/32) up to 2^((i+1)/32).
    pub hdr: [[u32; HDR_BINS]; 3],
    /// Luminance of each pixel binned the same way (the grey fill).
    pub luma_sdr: [u32; 256],
    pub luma_hdr: [u32; HDR_BINS],
    /// Each pixel's brightest channel above SDR white, binned as `hdr` (what lies beyond a
    /// display's headroom, [`HdrHistogram::above`]).
    pub max_hdr: [u32; HDR_BINS],
    /// Pixels with a channel at the HDR Limit (flattened by its roll-off).
    pub at_limit: u32,
    /// Pixels with a channel at or below the shadow clip.
    pub shadows: u32,
    /// Pixels counted.
    pub total: u32,
    /// The edit's HDR Limit in stops above SDR white.
    pub limit: f32,
}

impl HdrHistogram {
    /// The histogram of `img` (an [`crate::OutputDepth::F32Hdr`] render) for an HDR Limit of
    /// `limit` stops. `None` for anything but float samples.
    pub fn of(img: &DeepImage, limit: f32) -> Option<HdrHistogram> {
        let DeepSamples::F32(v) = &img.samples else { return None };
        let limit = if limit.is_finite() { limit.clamp(0.0, STOPS as f32) } else { STOPS as f32 };
        let luma = img.space.luma();
        let mut h = HdrHistogram {
            sdr: [[0; 256]; 3],
            hdr: [[0; HDR_BINS]; 3],
            luma_sdr: [0; 256],
            luma_hdr: [0; HDR_BINS],
            max_hdr: [0; HDR_BINS],
            at_limit: 0,
            shadows: 0,
            total: 0,
            limit,
        };
        let top = AT_LIMIT * limit.exp2();
        for c in v.as_chunks::<3>().0 {
            let c = c.map(|x| if x.is_finite() { x.max(0.0) } else { 0.0 });
            let mut flat = false;
            let mut dark = false;
            for (k, x) in c.into_iter().enumerate() {
                if let Some(b) = Self::bin(x) {
                    Self::put(&mut h.sdr[k], &mut h.hdr[k], b);
                }
                flat |= limit > 0.0 && x >= top;
                dark |= x <= 0.0005;
            }
            if let Some(Err(b)) = Self::bin(c[0].max(c[1]).max(c[2]))
                && let Some(n) = h.max_hdr.get_mut(b)
            {
                *n = n.saturating_add(1);
            }
            let y = luma[0] * c[0] + luma[1] * c[1] + luma[2] * c[2];
            if let Some(b) = Self::bin(y) {
                Self::put(&mut h.luma_sdr, &mut h.luma_hdr, b);
            }
            h.at_limit += u32::from(flat);
            h.shadows += u32::from(dark);
            h.total = h.total.saturating_add(1);
        }
        Some(h)
    }

    /// `Ok(sdr bin)` or `Err(hdr bin)` of a linear value (≥ 0).
    fn bin(x: f32) -> Option<Result<usize, usize>> {
        if !x.is_finite() {
            return None;
        }
        Some(if x <= WHITE_ROUNDING {
            let e = lightcraft_color::transfer::linear_to_srgb(x.clamp(0.0, 1.0));
            Ok(((e * 255.0 + 0.5) as usize).min(255))
        } else {
            Err(((x.log2() * BINS_PER_STOP as f32) as usize).min(HDR_BINS - 1))
        })
    }

    fn put(sdr: &mut [u32; 256], hdr: &mut [u32; HDR_BINS], b: Result<usize, usize>) {
        let slot = match b {
            Ok(i) => sdr.get_mut(i),
            Err(i) => hdr.get_mut(i),
        };
        if let Some(n) = slot {
            *n = n.saturating_add(1);
        }
    }

    /// Pixels with a channel above `stops` over SDR white (to a bin): what a display with
    /// 2^`stops` of headroom can't show. At 0 stops or less, every pixel above white.
    pub fn above(&self, stops: f32) -> u32 {
        let from = (stops.max(0.0) * BINS_PER_STOP as f32).ceil() as usize;
        self.max_hdr.get(from..).map_or(0, |b| b.iter().sum::<u32>())
    }

    /// Pixels with any light above SDR white.
    pub fn above_white(&self) -> u32 {
        self.above(0.0)
    }

    /// The bins of the HDR section up to the limit (per channel, then luminance).
    pub fn hdr_bins(&self) -> usize {
        ((self.limit * BINS_PER_STOP as f32).ceil() as usize).clamp(1, HDR_BINS)
    }
}

/// Where a value sits across a histogram drawn `width` wide with the HDR section for `limit`
/// stops: the SDR section's [`SDR_SHARE`] by encoded value, the rest by stops above white.
pub fn x_of(linear: f32, limit: f32, width: f32) -> f32 {
    let sdr_w = width * SDR_SHARE;
    if !(linear.is_finite() && linear > 1.0) {
        let e = lightcraft_color::transfer::linear_to_srgb(if linear.is_finite() { linear.clamp(0.0, 1.0) } else { 0.0 });
        return e * sdr_w;
    }
    let stops = linear.log2().min(limit.max(0.01));
    sdr_w + (width - sdr_w) * stops / limit.max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(px: &[[f32; 3]]) -> DeepImage {
        DeepImage { width: px.len(), height: 1, space: crate::OutputSpace::Srgb, samples: DeepSamples::F32(px.concat()) }
    }

    #[test]
    fn bins_split_at_white_and_count_stops_above() {
        let h = HdrHistogram::of(&img(&[[0.0, 0.18, 1.0], [2.0, 4.1, 0.5], [16.0, 15.99, 1.0], [f32::NAN, -1.0, 1e9]]), 4.0).unwrap();
        assert_eq!(h.total, 4);
        assert_eq!(h.sdr[2][255], 2, "SDR white is SDR");
        assert_eq!(h.hdr[0][32], 1, "2.0 is the first bin of the second stop");
        assert_eq!(h.hdr[1][65], 1);
        assert_eq!(h.hdr[2][HDR_BINS - 1], 1, "past the top lands in the last bin");
        assert_eq!(h.at_limit, 2, "16 = 2^4 is at the limit, as is 1e9");
        assert_eq!(h.above_white(), 3);
        assert_eq!(h.above(1.0), 3);
        assert_eq!(h.above(2.5), 2);
        assert_eq!(h.above(3.99), 2);
        assert_eq!(h.above(5.0), 0);
        assert_eq!(h.hdr_bins(), 128);
        let u16s = DeepImage { samples: DeepSamples::U16(vec![0; 3]), ..img(&[[0.0; 3]]) };
        assert!(HdrHistogram::of(&u16s, 4.0).is_none());
    }

    #[test]
    fn layout_puts_white_at_forty_percent() {
        assert_eq!(x_of(0.0, 4.0, 100.0), 0.0);
        assert!((x_of(1.0, 4.0, 100.0) - 40.0).abs() < 1e-4);
        assert!((x_of(4.0, 4.0, 100.0) - 70.0).abs() < 1e-4);
        assert!((x_of(16.0, 4.0, 100.0) - 100.0).abs() < 1e-4);
        assert!((x_of(1e6, 4.0, 100.0) - 100.0).abs() < 1e-4);
        assert_eq!(x_of(f32::NAN, 0.0, 100.0), 0.0);
    }
}
