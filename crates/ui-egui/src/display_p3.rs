//! The photo view on a wide-gamut (Display P3) screen. macOS shows LightCraft's window surface
//! untagged, i.e. in the display's own colour space: on a P3 display (every recent Mac, the Studio
//! Display) sRGB pixel values come out more saturated than they are, so the loupe looked richer
//! than the exported JPEG viewed in Preview. Here every rendered photo texture is converted from
//! sRGB to Display P3 values before upload, so the screen shows the colours the render (and the
//! export) has. Pixels kept for screenshots / pixel probes stay sRGB; exports never pass here.
//!
//! Display P3 shares sRGB's primaries' white (D65) and transfer curve; only the primaries differ,
//! so the conversion is decode → 3×3 matrix in linear light → encode.

use serde::{Deserialize, Serialize};

/// The colour space the window's pixels are shown in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewSpace {
    /// Display P3 on macOS (its window surface is untagged), sRGB elsewhere.
    #[default]
    Auto,
    /// Photos converted to Display P3 values (Apple displays: Studio Display, MacBook, iMac…).
    DisplayP3,
    /// Pixels shown as they are (an sRGB monitor).
    Srgb,
}

impl ViewSpace {
    /// Whether photo textures are converted to Display P3.
    pub fn is_p3(self) -> bool {
        match self {
            ViewSpace::Auto => cfg!(target_os = "macos"),
            ViewSpace::DisplayP3 => true,
            ViewSpace::Srgb => false,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            ViewSpace::Auto => "auto",
            ViewSpace::DisplayP3 => "displayP3",
            ViewSpace::Srgb => "srgb",
        }
    }
}

/// Linear sRGB → linear Display P3 (both D65), rows summing to 1.
const SRGB_TO_P3: [[f32; 3]; 3] = [[0.822_461_9, 0.177_538_1, 0.0], [0.033_194_2, 0.966_805_8, 0.0], [0.017_082_7, 0.072_397_4, 0.910_519_9]];

fn decode(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn encode(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Encoding table: linear (4096 steps) → 8-bit.
const ENC_STEPS: usize = 4096;

struct Tables {
    dec: [f32; 256],
    enc: Vec<u8>,
}

fn tables() -> &'static Tables {
    static T: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    T.get_or_init(|| Tables {
        dec: std::array::from_fn(|i| decode(i as f32 / 255.0)),
        enc: (0..=ENC_STEPS).map(|i| (encode(i as f32 / ENC_STEPS as f32) * 255.0).round().clamp(0.0, 255.0) as u8).collect(),
    })
}

/// Convert sRGB-encoded RGBA pixels to Display P3 values in place (alpha kept); large images in
/// parallel (helper threads share a queue with this one, so one that can't start costs nothing).
pub fn srgb_to_p3(rgba: &mut [u8]) {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    let px = rgba.len() / 4;
    if cfg!(target_arch = "wasm32") || threads <= 1 || px < 1 << 16 {
        convert(rgba);
        return;
    }
    let chunk = (px.div_ceil(threads * 4)).max(1) * 4;
    let queue = std::sync::Mutex::new(rgba.chunks_mut(chunk).collect::<Vec<_>>());
    let work = || {
        while let Some(part) = queue.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop() {
            convert(part);
        }
    };
    std::thread::scope(|s| {
        for _ in 1..threads {
            let _ = std::thread::Builder::new().spawn_scoped(s, work);
        }
        work();
    });
}

fn convert(rgba: &mut [u8]) {
    let t = tables();
    let enc = |v: f32| {
        let i = (v.clamp(0.0, 1.0) * ENC_STEPS as f32).round() as usize;
        t.enc.get(i).copied().unwrap_or(255)
    };
    for px in rgba.as_chunks_mut::<4>().0 {
        let (r, g, b) = (t.dec[px[0] as usize], t.dec[px[1] as usize], t.dec[px[2] as usize]);
        let m = &SRGB_TO_P3;
        px[0] = enc(m[0][0] * r + m[0][1] * g + m[0][2] * b);
        px[1] = enc(m[1][0] * r + m[1][1] * g + m[1][2] * b);
        px[2] = enc(m[2][0] * r + m[2][1] * g + m[2][2] * b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greys_stay_and_saturated_colours_move_inward() {
        let mut px = vec![0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255, 255, 0, 0, 255, 0, 255, 0, 200];
        srgb_to_p3(&mut px);
        assert_eq!(&px[..12], &[0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255], "neutrals unchanged");
        // sRGB red is inside P3: less than full red, some green and blue
        assert_eq!(&px[12..16], &[234, 51, 35, 255]);
        assert_eq!((px[16], px[17], px[19]), (117, 251, 200), "green, alpha kept");
    }

    #[test]
    fn large_images_convert_like_small_ones() {
        let mut big: Vec<u8> = (0..300_000u32).flat_map(|i| [(i % 256) as u8, (i / 256 % 256) as u8, (i * 7 % 256) as u8, 255]).collect();
        let mut one = big.clone();
        srgb_to_p3(&mut big);
        convert(&mut one);
        assert_eq!(big, one);
    }

    #[test]
    fn matrix_keeps_white() {
        for row in SRGB_TO_P3 {
            assert!((row.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        assert!(!ViewSpace::Srgb.is_p3() && ViewSpace::DisplayP3.is_p3());
    }
}
