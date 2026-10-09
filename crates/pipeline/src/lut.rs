//! 3D LUT profiles, registered here by id (`lut:…`) and found by the finishing stage; LUT
//! profiles render on the CPU.
//!
//! - `.cube` files (the plain-text Cube LUT format: `LUT_3D_SIZE`, optional `DOMAIN_MIN` /
//!   `DOMAIN_MAX`, then size³ "r g b" lines with red changing fastest) apply to the
//!   display-encoded output colour ([`LutStage::Output`]), blended by the profile amount.
//! - Colour tables of creative profiles (camera-raw `RGBTable`s, decoded by the engine) apply as
//!   [`LutStage::Profile`]: to the finished colour (after the colour adjustments and tone curves,
//!   before grain), in the table's own primaries and encoding, blended by the amount through
//!   `strength`. Where they go was measured against Lightroom's renders (`docs/lr-match.md`):
//!   after the tone curves matched clearly better than right after the base tone map.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Primaries of the RGB space a table is indexed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutPrimaries {
    Srgb,
    AdobeRgb,
    ProPhoto,
    DisplayP3,
    Rec2020,
}

impl LutPrimaries {
    fn space(self) -> lightcraft_color::RgbSpace {
        use lightcraft_color as c;
        match self {
            Self::Srgb => c::SRGB,
            Self::AdobeRgb => c::ADOBE_RGB,
            Self::ProPhoto => c::PROPHOTO,
            Self::DisplayP3 => c::DISPLAY_P3,
            Self::Rec2020 => c::REC2020,
        }
    }
}

/// How a table's inputs and outputs are encoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LutTransfer {
    Linear,
    Srgb,
    /// A pure power curve (`encoded = linear^(1/γ)`).
    Gamma(f32),
}

impl LutTransfer {
    #[inline]
    fn encode(self, v: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        match self {
            Self::Linear => v,
            Self::Srgb => lightcraft_color::transfer::linear_to_srgb(v),
            Self::Gamma(g) => v.powf(1.0 / g.max(0.1)),
        }
    }
    #[inline]
    fn decode(self, v: f32) -> f32 {
        match self {
            Self::Linear => v,
            Self::Srgb => lightcraft_color::transfer::srgb_to_linear(v.clamp(0.0, 1.0)),
            Self::Gamma(g) => v.clamp(0.0, 1.0).powf(g.max(0.1)),
        }
    }
}

/// Where in the pipeline a table applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutStage {
    /// On the display-encoded output colour, after the tone curves (`.cube` files).
    Output,
    /// A creative profile's table: on the finished colour in the table's own space and encoding.
    Profile,
}

/// A 3D colour lookup table.
#[derive(Clone, Debug, PartialEq)]
pub struct Lut3d {
    pub size: usize,
    pub data: Vec<[f32; 3]>,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    pub title: Option<String>,
    pub stage: LutStage,
    /// The space the table is indexed in (a [`LutStage::Profile`] table; output tables index the
    /// output's own encoded values).
    pub primaries: LutPrimaries,
    pub transfer: LutTransfer,
    /// Blend at amount 0 %, 100 % and 200 % (1 = the table as is); in between it is linear.
    pub strength: [f32; 3],
    /// Colours outside the table's gamut are clipped to it (a camera-raw table's "gamut clip"),
    /// instead of keeping their offset ("gamut extend").
    pub clip: bool,
}

impl Lut3d {
    /// Parse a `.cube` file.
    pub fn parse_cube(text: &str) -> Result<Lut3d, String> {
        let (mut size, mut title) = (0usize, None);
        let (mut min, mut max) = ([0.0f32; 3], [1.0f32; 3]);
        let mut data = Vec::new();
        let three = |rest: &str| -> Option<[f32; 3]> {
            let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        };
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            if let Some(r) = l.strip_prefix("TITLE") {
                title = Some(r.trim().trim_matches('"').to_string());
            } else if let Some(r) = l.strip_prefix("LUT_3D_SIZE") {
                size = r.trim().parse().map_err(|_| format!("bad LUT_3D_SIZE `{}`", r.trim()))?;
                if !(2..=256).contains(&size) {
                    return Err(format!("LUT_3D_SIZE {size} out of range"));
                }
            } else if l.starts_with("LUT_1D_SIZE") {
                return Err("1D LUTs are not supported (only 3D .cube files)".into());
            } else if let Some(r) = l.strip_prefix("DOMAIN_MIN") {
                min = three(r).ok_or("bad DOMAIN_MIN")?;
            } else if let Some(r) = l.strip_prefix("DOMAIN_MAX") {
                max = three(r).ok_or("bad DOMAIN_MAX")?;
            } else if l.starts_with(|c: char| c.is_ascii_alphabetic()) {
                // other keywords (LUT_IN_VIDEO_RANGE…) are ignored
            } else {
                data.push(three(l).ok_or_else(|| format!("bad line `{l}`"))?);
            }
        }
        if size == 0 {
            return Err("not a 3D .cube LUT (no LUT_3D_SIZE)".into());
        }
        if data.len() != size * size * size {
            return Err(format!("expected {} entries, found {}", size * size * size, data.len()));
        }
        Ok(Lut3d {
            size,
            data,
            domain_min: min,
            domain_max: max,
            title,
            stage: LutStage::Output,
            primaries: LutPrimaries::Srgb,
            transfer: LutTransfer::Srgb,
            strength: [0.0, 1.0, 2.0],
            clip: false,
        })
    }

    /// The blend for a profile amount (`amount` 0..2, 1 = 100 %).
    pub fn blend(&self, amount: f32) -> f32 {
        let a = if amount.is_finite() { amount.clamp(0.0, 2.0) } else { 1.0 };
        let [lo, mid, hi] = self.strength.map(|v| if v.is_finite() { v.clamp(0.0, 4.0) } else { 0.0 });
        if a <= 1.0 { lo + (mid - lo) * a } else { mid + (hi - mid) * (a - 1.0) }
    }

    /// Linear working-space (Rec.2020) ↔ linear table-space matrices.
    pub fn matrices(&self) -> ([[f32; 3]; 3], [[f32; 3]; 3]) {
        let sp = self.primaries.space();
        let to = lightcraft_color::WORKING.to_space(&sp);
        let from = sp.to_space(&lightcraft_color::WORKING);
        let f = |m: lightcraft_color::Mat3| m.0.map(|r| r.map(|v| v as f32));
        (f(to), f(from))
    }

    /// A [`LutStage::Profile`] table on linear working-space colour `c` with blend `k`, given
    /// [`Self::matrices`]: looked up in the table's encoding (inputs clipped to its range) and
    /// added as the table's change, so colours outside the table's gamut keep their offset.
    #[inline]
    pub fn apply_linear(&self, c: [f32; 3], k: f32, m: &([[f32; 3]; 3], [[f32; 3]; 3])) -> [f32; 3] {
        let mul = |m: &[[f32; 3]; 3], v: [f32; 3]| std::array::from_fn::<f32, 3, _>(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]);
        let t = mul(&m.0, c);
        let e = t.map(|v| self.transfer.encode(v));
        let o = self.apply(e);
        let r = if self.clip {
            mul(&m.1, std::array::from_fn(|i| self.transfer.decode(e[i] + (o[i] - e[i]) * k)))
        } else {
            let d: [f32; 3] = std::array::from_fn(|i| self.transfer.decode(e[i] + (o[i] - e[i]) * k) - self.transfer.decode(e[i]));
            mul(&m.1, [t[0] + d[0], t[1] + d[1], t[2] + d[2]])
        };
        r.map(|v| if v.is_finite() { v } else { 0.0 })
    }

    /// Look up `c` (trilinear).
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        if n < 2 || self.data.len() != n * n * n {
            return c;
        }
        let s = (n - 1) as f32;
        let mut idx = [0usize; 3];
        let mut frac = [0.0f32; 3];
        for k in 0..3 {
            let span = (self.domain_max[k] - self.domain_min[k]).max(1e-6);
            let x = (((c[k] - self.domain_min[k]) / span).clamp(0.0, 1.0)) * s;
            let i = (x.floor() as usize).min(n - 2);
            idx[k] = i;
            frac[k] = x - i as f32;
        }
        let at = |r: usize, g: usize, b: usize| self.data.get(r + n * (g + n * b)).copied().unwrap_or([0.0; 3]);
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        let (r, g, b) = (idx[0], idx[1], idx[2]);
        let c00 = lerp(at(r, g, b), at(r + 1, g, b), frac[0]);
        let c10 = lerp(at(r, g + 1, b), at(r + 1, g + 1, b), frac[0]);
        let c01 = lerp(at(r, g, b + 1), at(r + 1, g, b + 1), frac[0]);
        let c11 = lerp(at(r, g + 1, b + 1), at(r + 1, g + 1, b + 1), frac[0]);
        lerp(lerp(c00, c10, frac[1]), lerp(c01, c11, frac[1]), frac[2])
    }
}

fn registry() -> &'static RwLock<HashMap<String, Arc<Lut3d>>> {
    static R: OnceLock<RwLock<HashMap<String, Arc<Lut3d>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Make a LUT available to renders under profile id `id` (`lut:…`).
pub fn register(id: &str, lut: Lut3d) {
    registry().write().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), Arc::new(lut));
}

/// The LUT registered for profile `id`.
pub fn get(id: &str) -> Option<Arc<Lut3d>> {
    if !id.starts_with("lut:") {
        return None;
    }
    registry().read().unwrap_or_else(|e| e.into_inner()).get(id).cloned()
}

/// Forget a LUT profile.
pub fn unregister(id: &str) {
    registry().write().unwrap_or_else(|e| e.into_inner()).remove(id);
}

/// Profile ids that render with a LUT (and so on the CPU).
pub fn is_lut_profile(id: &str) -> bool {
    id.starts_with("lut:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(f: impl Fn([f32; 3]) -> [f32; 3], n: usize) -> String {
        let mut s = format!("TITLE \"test\"\nLUT_3D_SIZE {n}\n");
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let c = f([r as f32 / (n - 1) as f32, g as f32 / (n - 1) as f32, b as f32 / (n - 1) as f32]);
                    s.push_str(&format!("{} {} {}\n", c[0], c[1], c[2]));
                }
            }
        }
        s
    }

    #[test]
    fn identity_and_swap() {
        let id = Lut3d::parse_cube(&cube(|c| c, 17)).unwrap();
        assert_eq!(id.title.as_deref(), Some("test"));
        let v = id.apply([0.2, 0.55, 0.9]);
        assert!(v.iter().zip([0.2, 0.55, 0.9]).all(|(a, b)| (a - b).abs() < 1e-5), "{v:?}");
        let swap = Lut3d::parse_cube(&cube(|c| [c[2], c[1], c[0]], 9)).unwrap();
        let v = swap.apply([0.1, 0.5, 0.8]);
        assert!((v[0] - 0.8).abs() < 1e-5 && (v[2] - 0.1).abs() < 1e-5, "{v:?}");
        assert!(Lut3d::parse_cube("LUT_1D_SIZE 4\n0 0 0").is_err());
        assert!(Lut3d::parse_cube("LUT_3D_SIZE 2\n0 0 0\n").is_err(), "too few entries");
    }

    #[test]
    fn profile_tables_blend_and_keep_out_of_gamut_offsets() {
        let mut swap = Lut3d::parse_cube(&cube(|c| [c[2], c[1], c[0]], 9)).unwrap();
        swap.stage = LutStage::Profile;
        swap.strength = [0.0, 0.6, 1.5];
        assert_eq!((swap.blend(0.0), swap.blend(1.0), swap.blend(2.0)), (0.0, 0.6, 1.5));
        assert!((swap.blend(0.5) - 0.3).abs() < 1e-6 && (swap.blend(1.5) - 1.05).abs() < 1e-6);
        assert_eq!(swap.blend(f32::NAN), 0.6, "a bad amount is 100 %");
        let m = swap.matrices();
        let c = [0.4f32, 0.2, 0.05];
        let none = swap.apply_linear(c, 0.0, &m);
        assert!(none.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-4), "{none:?}");
        let full = swap.apply_linear(c, 1.0, &m);
        assert!(full[2] > full[0], "red and blue swapped: {full:?}");
        // a colour beyond the table's range keeps what the table can't express
        let hot = swap.apply_linear([3.0, 3.0, 3.0], 1.0, &m);
        assert!(hot.iter().all(|v| (v - 3.0).abs() < 1e-3), "{hot:?}");
        let bad = swap.apply_linear([f32::NAN, 0.5, 0.5], 1.0, &m);
        assert!(bad.iter().all(|v| v.is_finite()));
        // a malformed table (wrong data length) is the identity, not a panic
        let mut broken = swap.clone();
        broken.data.truncate(5);
        assert_eq!(broken.apply([0.1, 0.2, 0.3]), [0.1, 0.2, 0.3]);
    }
}
