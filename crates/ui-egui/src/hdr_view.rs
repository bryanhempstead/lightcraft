//! HDR photos on an HDR (EDR) display (LR-VIEW-HDR-DISPLAY). On a Mac whose screen has headroom
//! above SDR white (the Studio Display, a MacBook Pro's XDR screen), the window surface is half
//! float in extended sRGB (`patches/egui-wgpu`, asked for by the app before the window opens;
//! colour-managed by macOS, so photos are no longer converted to Display P3 values for it). A photo edited in HDR is then rendered as its float HDR rendition fitted to the
//! display's headroom ([`lightcraft_engine::media::RenderJob::with_hdr_view`]) and shown from a
//! half-float texture whose values go above 1.0, so its highlights are brighter than the UI's white,
//! as in Lightroom. Everything else, and every other display, stays the 8-bit SDR path.
//!
//! The frontend crate knows no GPU: the app hands it an [`HdrView`] that uploads the pixels and
//! reports the display's headroom. Without one (headless, the browser, an SDR surface) nothing
//! changes.

/// The window's HDR output, provided by the app (it owns the wgpu renderer).
pub trait HdrView {
    /// The display's EDR headroom right now (peak over SDR white; 1.0 = none at the moment), when
    /// the window surface can show it. `None`: an SDR surface or unknown.
    fn headroom(&self) -> Option<f32>;
    /// A texture of `px` for the view `slot` (re-uploaded only when `px.key` changes).
    fn texture(&mut self, slot: &str, px: &HdrPixels) -> Option<egui::TextureId>;
    /// Whether the window surface is tagged with its colour space (macOS colour-manages it): photo
    /// textures are then uploaded as sRGB, not converted to the display's space
    /// ([`crate::display_p3`]).
    fn color_managed(&self) -> bool {
        true
    }
}

/// An HDR rendition ready for the screen: half-float RGBA (IEEE 754 binary16 bits), the colours
/// in the window's extended colour space with the sRGB curve, SDR white = 1.0.
#[derive(Clone, Debug, PartialEq)]
pub struct HdrPixels {
    /// The render's key (a new one = new pixels).
    pub key: u64,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u16>,
    /// The brightest value (linear, SDR white = 1), for the HDR histogram's range.
    pub peak: f32,
}

/// Linear sRGB → linear Display P3 (both D65).
const SRGB_TO_P3: [[f32; 3]; 3] = [[0.822_461_9, 0.177_538_1, 0.0], [0.033_194_2, 0.966_805_8, 0.0], [0.017_082_7, 0.072_397_4, 0.910_519_9]];

/// The sRGB curve continued past 1 (and mirrored below 0), as extended-range colour spaces use it.
pub fn encode_extended(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = v.abs();
    let e = if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    e.copysign(v)
}

/// IEEE 754 binary32 → binary16 bits (round to nearest; overflow → the largest finite half).
pub fn f32_to_f16(v: f32) -> u16 {
    if v.is_nan() {
        return 0;
    }
    let sign = if v.is_sign_negative() { 0x8000u16 } else { 0 };
    let a = v.abs();
    if a >= 65504.0 {
        return sign | 0x7bff;
    }
    if a < 6.103_515_6e-5 {
        // subnormal half: multiples of 2^-24
        return sign | (a * 16_777_216.0).round() as u16;
    }
    let bits = a.to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x7f_ffff;
    // round the 23-bit mantissa to 10 bits (to nearest, carry into the exponent)
    let mut h = ((exp as u32) << 10) | (mant >> 13);
    if mant & 0x1fff > 0x1000 || (mant & 0x1fff == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    sign | (h.min(0x7bff) as u16)
}

/// The float HDR render `deep` (linear sRGB primaries, SDR white = 1) as [`HdrPixels`] for a
/// window in extended Display P3 (`p3`) or extended sRGB. `None` for anything but float samples.
pub fn pixels(deep: &lightcraft_pipeline::DeepImage, key: u64, p3: bool) -> Option<HdrPixels> {
    let lightcraft_pipeline::DeepSamples::F32(v) = &deep.samples else { return None };
    let n = deep.width.checked_mul(deep.height)?;
    let px = v.as_chunks::<3>().0.get(..n)?;
    let mut rgba = Vec::with_capacity(n.checked_mul(4)?);
    let mut peak = 0f32;
    let one = f32_to_f16(1.0);
    for c in px {
        let c = c.map(|x| if x.is_finite() { x } else { 0.0 });
        let d = if p3 { SRGB_TO_P3.map(|r| r[0] * c[0] + r[1] * c[1] + r[2] * c[2]) } else { c };
        peak = peak.max(c[0].max(c[1]).max(c[2]));
        rgba.extend(d.map(|x| f32_to_f16(encode_extended(x))));
        rgba.push(one);
    }
    Some(HdrPixels { key, width: deep.width, height: deep.height, rgba, peak })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f16_to_f32(h: u16) -> f32 {
        let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
        let e = i32::from((h >> 10) & 0x1f);
        let f = f32::from(h & 0x3ff);
        if e == 0 { s * f * 2f32.powi(-24) } else { s * (1.0 + f / 1024.0) * 2f32.powi(e - 15) }
    }

    #[test]
    fn halves_round_trip() {
        for v in [0.0f32, 1.0, 0.5, 0.18, 2.0, 4.92, 16.0, -0.25, 1e-6, 60000.0] {
            let back = f16_to_f32(f32_to_f16(v));
            assert!((back - v).abs() <= v.abs() * 1e-3 + 1e-7, "{v} -> {back}");
        }
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(f32::NAN), 0);
        assert_eq!(f32_to_f16(f32::INFINITY), 0x7bff);
        assert_eq!(f32_to_f16(1e9), 0x7bff);
    }

    #[test]
    fn extended_curve_keeps_sdr_and_goes_past_white() {
        assert!((encode_extended(1.0) - 1.0).abs() < 1e-6);
        assert!(encode_extended(4.0) > 1.5);
        assert!(encode_extended(-0.5) < 0.0);
        assert_eq!(encode_extended(f32::NAN), 0.0);
        for i in 0..=20 {
            let v = i as f32 / 20.0;
            let srgb = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            assert!((encode_extended(v) - srgb).abs() < 1e-6);
        }
    }

    #[test]
    fn pixels_convert_float_renders_only() {
        let deep = lightcraft_pipeline::DeepImage {
            width: 2,
            height: 1,
            space: lightcraft_pipeline::OutputSpace::Srgb,
            samples: lightcraft_pipeline::DeepSamples::F32(vec![0.18, 0.18, 0.18, 4.0, 3.0, 2.0]),
        };
        let p = pixels(&deep, 7, true).unwrap();
        assert_eq!((p.width, p.height, p.rgba.len(), p.key), (2, 1, 8, 7));
        assert!((p.peak - 4.0).abs() < 1e-6);
        // a neutral stays neutral in P3; the bright one is above 1
        assert_eq!(p.rgba[0], p.rgba[1]);
        assert!(f16_to_f32(p.rgba[4]) > 1.0);
        let short = lightcraft_pipeline::DeepImage { samples: lightcraft_pipeline::DeepSamples::F32(vec![1.0; 3]), ..deep.clone() };
        assert!(pixels(&short, 0, true).is_none());
        let u16s = lightcraft_pipeline::DeepImage { samples: lightcraft_pipeline::DeepSamples::U16(vec![0; 6]), ..deep };
        assert!(pixels(&u16s, 0, false).is_none());
    }
}
