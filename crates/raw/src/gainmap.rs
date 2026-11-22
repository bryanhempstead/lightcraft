//! DNG 1.6 `ProfileGainTableMap` (tag 52525, raw IFD) and DNG 1.7 `ProfileGainTableMap2` (tag
//! 52544, IFD 0): spatially varying gain tables a camera stores as the starting point of its
//! rendering. Apple iPhone ProRAW carries one (its local tone mapping); without it those files
//! render several stops too dark in the shadows and too bright in the highlights.
//!
//! Layout (Adobe DNG Specification 1.7, the tag's byte order is the file's): `MapPointsV`,
//! `MapPointsH` (u32), `MapSpacingV`, `MapSpacingH`, `MapOriginV`, `MapOriginH` (f64, in
//! image-relative coordinates 0..1), `MapPointsN` (u32), `MapInputWeights[5]` (f32); version 2 adds
//! `DataType` (u32: 0 = u8, 1 = u16, 2 = f16, 3 = f32), `Gamma`, `GainMin`, `GainMax` (f32). Then
//! `V × H × N` gains, the table index fastest.
//!
//! Applied per pixel to linear ProPhoto RGB after the baseline exposure (and after the hue/saturation
//! map), before the look table and tone curve, as Adobe's reference renderer does: the table input
//! is `clamp(w · [R, G, B, min, max], 0, 1) ^ Gamma`, scaled by `N`; the gain is interpolated over
//! (row, column, table) and multiplies all three channels. The map position is the pixel's centre
//! relative to the whole stage-3 image (the active area), clamped to the grid.
//!
//! The data comes from the user's own file at run time; nothing here ships map data. A malformed
//! map is rejected (and the image renders without it), never trusted.

use crate::{RawImage, Rect};
use lightcraft_color::{D50, D65, Mat3, PROPHOTO, REC2020, bradford};
use lightcraft_tiff::{ByteOrder, Ifd};
use serde::{Deserialize, Serialize};

/// `ProfileGainTableMap` (DNG 1.6, raw IFD).
pub const PROFILE_GAIN_TABLE_MAP: u16 = 52525;
/// `ProfileGainTableMap2` (DNG 1.7, IFD 0; supersedes the first).
pub const PROFILE_GAIN_TABLE_MAP_2: u16 = 52544;

/// Largest map accepted, in gains (iPhone 13 Pro: 48 × 62 × 257 ≈ 0.76 M).
const MAX_ENTRIES: usize = 1 << 24;
/// Largest grid dimension or table size accepted.
const MAX_POINTS: u32 = 1 << 16;
/// Gain range accepted (the DNG SDK's limits), and Gamma's.
const MIN_GAIN: f32 = 1.0 / 4096.0;
const MAX_GAIN: f32 = 4096.0;
const MIN_GAMMA: f32 = 0.125;
const MAX_GAMMA: f32 = 8.0;

/// A parsed, validated gain table map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GainTableMap {
    /// Grid size (rows, columns) and table size.
    pub points_v: usize,
    pub points_h: usize,
    pub points_n: usize,
    /// Grid spacing and origin, image-relative.
    pub spacing_v: f64,
    pub spacing_h: f64,
    pub origin_v: f64,
    pub origin_h: f64,
    /// `MapInputWeights`: R, G, B, min(R, G, B), max(R, G, B).
    pub weights: [f32; 5],
    /// Applied to the clamped table input (1 for version-1 maps).
    pub gamma: f32,
    /// `points_v × points_h × points_n` gains, table index fastest.
    pub gains: Vec<f32>,
}

/// Bounded big/little-endian reader over the tag's bytes.
struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    order: ByteOrder,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.pos.checked_add(N).ok_or("offset overflow")?;
        let s = self.b.get(self.pos..end).ok_or("truncated")?;
        self.pos = end;
        let mut a = [0u8; N];
        a.copy_from_slice(s);
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(self.order.u16(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(self.order.u32(self.take()?))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.order.u64(self.take()?)))
    }
}

/// IEEE 754 half → single precision.
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = f32::from(h & 0x3ff);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        e => sign * (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

impl GainTableMap {
    /// Parse a `ProfileGainTableMap` (`version2 = false`) or `ProfileGainTableMap2` tag in byte
    /// order `order`. Every count, size and value is checked; a bad map is an error.
    pub fn parse(bytes: &[u8], order: ByteOrder, version2: bool) -> Result<GainTableMap, String> {
        let mut r = Reader { b: bytes, pos: 0, order };
        let (pv, ph) = (r.u32()?, r.u32()?);
        let (mut sv, mut sh) = (r.f64()?, r.f64()?);
        let (mut ov, mut oh) = (r.f64()?, r.f64()?);
        let pn = r.u32()?;
        let mut weights = [0f32; 5];
        for w in &mut weights {
            *w = r.f32()?;
        }
        let (data_type, gamma, gain_min, gain_max) = if version2 { (r.u32()?, r.f32()?, r.f32()?, r.f32()?) } else { (3, 1.0, 1.0, 1.0) };
        if !(1..=MAX_POINTS).contains(&pv) || !(1..=MAX_POINTS).contains(&ph) || !(1..=MAX_POINTS).contains(&pn) {
            return Err(format!("bad map size {pv} × {ph} × {pn}"));
        }
        // a single row / column has no spacing (as the DNG SDK reads it)
        if pv == 1 {
            (sv, ov) = (1.0, 0.0);
        }
        if ph == 1 {
            (sh, oh) = (1.0, 0.0);
        }
        if !(sv.is_finite() && sh.is_finite() && sv > 0.0 && sh > 0.0 && ov.is_finite() && oh.is_finite()) {
            return Err("bad map spacing or origin".into());
        }
        if weights.iter().any(|w| !w.is_finite()) {
            return Err("bad MapInputWeights".into());
        }
        if !(MIN_GAMMA..=MAX_GAMMA).contains(&gamma) || data_type > 3 {
            return Err(format!("bad Gamma {gamma} or DataType {data_type}"));
        }
        if data_type < 2 && !(gain_min.is_finite() && gain_max.is_finite() && gain_min >= MIN_GAIN && gain_max <= MAX_GAIN && gain_min <= gain_max) {
            return Err(format!("bad GainMin / GainMax {gain_min} / {gain_max}"));
        }
        let n =
            (pv as usize).checked_mul(ph as usize).and_then(|n| n.checked_mul(pn as usize)).filter(|&n| n <= MAX_ENTRIES).ok_or("map too large")?;
        let entry = [1usize, 2, 2, 4].get(data_type as usize).copied().unwrap_or(4);
        if bytes.len().saturating_sub(r.pos) < n.saturating_mul(entry) {
            return Err(format!("truncated: {n} gains need {} bytes, {} left", n.saturating_mul(entry), bytes.len().saturating_sub(r.pos)));
        }
        let mut gains = Vec::with_capacity(n);
        for _ in 0..n {
            let g = match data_type {
                0 => gain_min + f32::from(r.u8()?) / 255.0 * (gain_max - gain_min),
                1 => gain_min + f32::from(r.u16()?) / 65535.0 * (gain_max - gain_min),
                2 => f16_to_f32(r.u16()?),
                _ => r.f32()?,
            };
            if !g.is_finite() || g < 0.0 {
                return Err(format!("bad gain {g}"));
            }
            gains.push(g.min(MAX_GAIN));
        }
        Ok(GainTableMap {
            points_v: pv as usize,
            points_h: ph as usize,
            points_n: pn as usize,
            spacing_v: sv,
            spacing_h: sh,
            origin_v: ov,
            origin_h: oh,
            weights,
            gamma,
            gains,
        })
    }

    /// [`Self::parse`] in the file's byte order, else the other one (a writer that put the
    /// structure in its native order): the order whose sizes add up wins.
    pub fn parse_any(bytes: &[u8], order: ByteOrder, version2: bool) -> Result<GainTableMap, String> {
        let other = match order {
            ByteOrder::Little => ByteOrder::Big,
            ByteOrder::Big => ByteOrder::Little,
        };
        GainTableMap::parse(bytes, order, version2).or_else(|e| GainTableMap::parse(bytes, other, version2).map_err(|_| e))
    }

    /// The tags of a DNG: `ProfileGainTableMap2` (IFD 0, else the raw IFD) supersedes
    /// `ProfileGainTableMap` (raw IFD, else IFD 0). `None` without one; a malformed map is
    /// ignored with a warning (the photo renders without it).
    pub fn from_ifds(ifd0: &Ifd, raw: &Ifd, order: ByteOrder) -> Option<GainTableMap> {
        let tags = [
            (ifd0, PROFILE_GAIN_TABLE_MAP_2, true),
            (raw, PROFILE_GAIN_TABLE_MAP_2, true),
            (raw, PROFILE_GAIN_TABLE_MAP, false),
            (ifd0, PROFILE_GAIN_TABLE_MAP, false),
        ];
        tags.into_iter().find_map(|(ifd, tag, v2)| {
            let bytes = ifd.bytes(tag)?;
            GainTableMap::parse_any(bytes, order, v2)
                .inspect_err(|e| eprintln!("lightcraft: ignoring ProfileGainTableMap{}: {e}", if v2 { "2" } else { "" }))
                .ok()
        })
    }

    #[inline]
    fn entry(&self, row: usize, col: usize, t: usize) -> f32 {
        self.gains.get((row * self.points_h + col) * self.points_n + t).copied().unwrap_or(1.0)
    }

    /// The gain at image-relative position `(u, v)` (0..1 over the stage-3 image: `u`
    /// horizontal) for linear ProPhoto `rgb` (after the baseline exposure).
    pub fn gain(&self, u: f32, v: f32, rgb: [f32; 3]) -> f32 {
        let w = &self.weights;
        let (lo, hi) = (rgb[0].min(rgb[1]).min(rgb[2]), rgb[0].max(rgb[1]).max(rgb[2]));
        let mut weight = w[0] * rgb[0] + w[1] * rgb[1] + w[2] * rgb[2] + w[3] * lo + w[4] * hi;
        if !weight.is_finite() {
            weight = 0.0;
        }
        weight = weight.clamp(0.0, 1.0);
        if self.gamma != 1.0 {
            weight = weight.powf(self.gamma);
        }
        let axis = |pos: f32, origin: f64, spacing: f64, n: usize| {
            let p = ((f64::from(pos) - origin) / spacing) as f32;
            let p = if p.is_finite() { p.clamp(0.0, (n - 1) as f32) } else { 0.0 };
            let i0 = (p as usize).min(n - 1);
            (i0, (i0 + 1).min(n - 1), p - i0 as f32)
        };
        let (x0, x1, xf) = axis(u, self.origin_h, self.spacing_h, self.points_h);
        let (y0, y1, yf) = axis(v, self.origin_v, self.spacing_v, self.points_v);
        // the table input is scaled by N (not N − 1), as in Adobe's reference renderer
        let n = self.points_n;
        let ws = weight * n as f32;
        let w0 = (ws as usize).min(n - 1);
        let w1 = (w0 + 1).min(n - 1);
        let wf = ws - w0 as f32;
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let at = |row: usize, col: usize| lerp(self.entry(row, col, w0), self.entry(row, col, w1), wf);
        let top = lerp(at(y0, x0), at(y0, x1), xf);
        let bottom = lerp(at(y1, x0), at(y1, x1), xf);
        lerp(top, bottom, yf)
    }
}

/// A gain table map placed on a developed image: maps the image's pixels to the stage-3 image
/// and its linear Rec.2020 D65 colour to the map's ProPhoto input.
#[derive(Clone, Debug)]
pub struct GainMapper<'a> {
    map: &'a GainTableMap,
    /// u = u0 + (x + 0.5)·du, v = v0 + (y + 0.5)·dv
    u0: f32,
    du: f32,
    v0: f32,
    dv: f32,
    to_prophoto: [[f32; 3]; 3],
}

impl<'a> GainMapper<'a> {
    /// `map` on an image of `width × height` pixels covering `frame` of a stage-3 image of
    /// `stage3_width × stage3_height` (frame and stage-3 sizes in sensor pixels).
    pub fn new(map: &'a GainTableMap, frame: Rect, stage3_width: usize, stage3_height: usize, width: usize, height: usize) -> Option<GainMapper<'a>> {
        if width == 0 || height == 0 || stage3_width == 0 || stage3_height == 0 || frame.width == 0 || frame.height == 0 {
            return None;
        }
        let (sw, sh) = (stage3_width as f32, stage3_height as f32);
        let to: Mat3 = PROPHOTO.from_xyz().mul(&bradford(D65, D50)).mul(&REC2020.to_xyz());
        Some(GainMapper {
            map,
            u0: frame.x as f32 / sw,
            du: frame.width as f32 / width as f32 / sw,
            v0: frame.y as f32 / sh,
            dv: frame.height as f32 / height as f32 / sh,
            to_prophoto: to.to_f32(),
        })
    }

    /// The map placed on `raw`'s developed image of `width × height` ([`RawImage::develop`] or
    /// [`RawImage::develop_binned`]: its default crop, else the whole active area, unoriented).
    /// `None` when the file has no gain table map.
    pub fn for_raw(raw: &'a RawImage, width: usize, height: usize) -> Option<GainMapper<'a>> {
        let map = raw.color.profile.gain_table_map.as_deref()?;
        let a = raw.active_area;
        let c = raw.crop.clipped(a.width, a.height);
        // `develop` keeps the whole area when the crop is empty, `develop_binned` when it is
        // smaller than a block: an image wider (or taller) than the crop covers the whole area
        let frame = if c.width == 0 || c.height == 0 || width > c.width || height > c.height { Rect::new(0, 0, a.width, a.height) } else { c };
        GainMapper::new(map, frame, a.width, a.height, width, height)
    }

    /// The gain at pixel `(x, y)` for linear ProPhoto `prophoto`.
    #[inline]
    pub fn gain_prophoto(&self, x: usize, y: usize, prophoto: [f32; 3]) -> f32 {
        let u = self.u0 + (x as f32 + 0.5) * self.du;
        let v = self.v0 + (y as f32 + 0.5) * self.dv;
        self.map.gain(u, v, prophoto)
    }

    /// Pixel `(x, y)`'s linear Rec.2020 `rgb` times its gain.
    #[inline]
    pub fn apply_rec2020(&self, x: usize, y: usize, rgb: [f32; 3]) -> [f32; 3] {
        let m = &self.to_prophoto;
        let p: [f32; 3] = std::array::from_fn(|i| m[i][0] * rgb[0] + m[i][1] * rgb[1] + m[i][2] * rgb[2]);
        let g = self.gain_prophoto(x, y, p);
        rgb.map(|c| c * g)
    }
}

/// Serialise `map` as a tag of the version it needs (`(tag, bytes)`), in byte order `order`.
pub fn write_tag(map: &GainTableMap, order: ByteOrder) -> (u16, Vec<u8>) {
    let mut out = Vec::with_capacity(80 + map.gains.len() * 4);
    let v2 = map.gamma != 1.0;
    for v in [map.points_v, map.points_h] {
        order.put_u32(&mut out, v as u32);
    }
    for v in [map.spacing_v, map.spacing_h, map.origin_v, map.origin_h] {
        order.put_u64(&mut out, v.to_bits());
    }
    order.put_u32(&mut out, map.points_n as u32);
    for w in map.weights {
        order.put_u32(&mut out, w.to_bits());
    }
    if v2 {
        order.put_u32(&mut out, 3);
        for v in [map.gamma, 1.0, 1.0] {
            order.put_u32(&mut out, v.to_bits());
        }
    }
    for g in &map.gains {
        order.put_u32(&mut out, g.to_bits());
    }
    (if v2 { PROFILE_GAIN_TABLE_MAP_2 } else { PROFILE_GAIN_TABLE_MAP }, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(order: ByteOrder, dims: [u32; 3], spacing: [f64; 2], origin: [f64; 2], weights: [f32; 5], gains: &[f32]) -> Vec<u8> {
        let map = GainTableMap {
            points_v: dims[0] as usize,
            points_h: dims[1] as usize,
            points_n: dims[2] as usize,
            spacing_v: spacing[0],
            spacing_h: spacing[1],
            origin_v: origin[0],
            origin_h: origin[1],
            weights,
            gamma: 1.0,
            gains: gains.to_vec(),
        };
        write_tag(&map, order).1
    }

    /// 2 × 2 grid over the whole image, 2 table points; weight = G.
    fn small() -> GainTableMap {
        // (row, col, t): gains
        let gains = [1.0, 2.0, 1.0, 4.0, 0.5, 1.0, 1.0, 1.0];
        let b = blob(ByteOrder::Big, [2, 2, 2], [1.0, 1.0], [0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0], &gains);
        GainTableMap::parse(&b, ByteOrder::Big, false).unwrap()
    }

    #[test]
    fn parses_and_interpolates_like_the_reference() {
        let m = small();
        assert_eq!((m.points_v, m.points_h, m.points_n), (2, 2, 2));
        // top-left corner, weight 0 → entry (0, 0, 0)
        assert!((m.gain(0.0, 0.0, [0.0; 3]) - 1.0).abs() < 1e-6);
        // weight 0.25 → table position 0.5 (scaled by N = 2): halfway between 1 and 2
        assert!((m.gain(0.0, 0.0, [0.0, 0.25, 0.0]) - 1.5).abs() < 1e-6);
        // weight ≥ 0.5 reaches the last table entry
        assert!((m.gain(0.0, 0.0, [0.0, 0.5, 0.0]) - 2.0).abs() < 1e-6);
        assert!((m.gain(0.0, 0.0, [9.0, 9.0, 9.0]) - 2.0).abs() < 1e-6, "weight clamps to 1");
        // top-right (col 1): table (1, 4)
        assert!((m.gain(1.0, 0.0, [0.0, 0.25, 0.0]) - 2.5).abs() < 1e-6);
        // centre, weight 0.25: mean of 1.5, 2.5, 0.75, 1.0
        let want = (1.5 + 2.5 + 0.75 + 1.0) / 4.0;
        assert!((m.gain(0.5, 0.5, [0.0, 0.25, 0.0]) - want).abs() < 1e-6, "{}", m.gain(0.5, 0.5, [0.0, 0.25, 0.0]));
        // outside the grid clamps to its edge
        assert_eq!(m.gain(-3.0, 7.0, [0.0; 3]), m.gain(0.0, 1.0, [0.0; 3]));
        // NaN colour or position: no panic, a finite gain
        assert!(m.gain(f32::NAN, 0.5, [f32::NAN, f32::INFINITY, 0.0]).is_finite());
    }

    #[test]
    fn weights_use_min_and_max_and_gamma() {
        let gains = [0.0, 1.0];
        let b = blob(ByteOrder::Little, [1, 1, 2], [0.0, 0.0], [5.0, 5.0], [0.0, 0.0, 0.0, 0.5, 0.5], &gains);
        let m = GainTableMap::parse(&b, ByteOrder::Little, false).unwrap();
        // one point: spacing / origin are ignored; weight = (min + max) / 2 = 0.2 → t = 0.4
        assert!((m.gain(0.3, 0.9, [0.1, 0.3, 0.2]) - 0.4).abs() < 1e-6);
        let mut g = m.clone();
        g.gamma = 2.0;
        assert!((g.gain(0.0, 0.0, [0.1, 0.3, 0.2]) - 0.08).abs() < 1e-6);
        // the version-2 tag round-trips gamma
        let (tag, b2) = write_tag(&g, ByteOrder::Big);
        assert_eq!(tag, PROFILE_GAIN_TABLE_MAP_2);
        assert_eq!(GainTableMap::parse(&b2, ByteOrder::Big, true).unwrap(), g);
    }

    #[test]
    fn version2_integer_and_half_data() {
        let mut b = Vec::new();
        let o = ByteOrder::Big;
        for v in [1u32, 1] {
            o.put_u32(&mut b, v);
        }
        for v in [1f64, 1.0, 0.0, 0.0] {
            o.put_u64(&mut b, v.to_bits());
        }
        o.put_u32(&mut b, 3);
        for w in [1f32, 0.0, 0.0, 0.0, 0.0] {
            o.put_u32(&mut b, w.to_bits());
        }
        let head = b.clone();
        // u8 between GainMin 0.5 and GainMax 2.5
        o.put_u32(&mut b, 0);
        for v in [1f32, 0.5, 2.5] {
            o.put_u32(&mut b, v.to_bits());
        }
        b.extend_from_slice(&[0, 255, 51]);
        let m = GainTableMap::parse(&b, o, true).unwrap();
        assert!(m.gains.iter().zip([0.5, 2.5, 0.9]).all(|(a, b)| (a - b).abs() < 1e-6), "{:?}", m.gains);
        // f16: 1.0, 2.0, 0.5
        let mut h = head.clone();
        o.put_u32(&mut h, 2);
        for v in [1f32, 1.0, 1.0] {
            o.put_u32(&mut h, v.to_bits());
        }
        for v in [0x3c00u16, 0x4000, 0x3800] {
            o.put_u16(&mut h, v);
        }
        assert_eq!(GainTableMap::parse(&h, o, true).unwrap().gains, vec![1.0, 2.0, 0.5]);
        // a NaN half and an out-of-range gamma are rejected
        let n = h.len();
        h[n - 2..].copy_from_slice(&[0x7e, 0x00]);
        assert!(GainTableMap::parse(&h, o, true).is_err());
        let mut g = head;
        o.put_u32(&mut g, 3);
        for v in [100f32, 1.0, 1.0] {
            o.put_u32(&mut g, v.to_bits());
        }
        assert!(GainTableMap::parse(&g, o, true).is_err());
    }

    #[test]
    fn hostile_maps_are_rejected_without_panicking() {
        let good = blob(ByteOrder::Big, [2, 2, 2], [1.0, 1.0], [0.0, 0.0], [0.0, 1.0, 0.0, 0.0, 0.0], &[1.0; 8]);
        assert!(GainTableMap::parse(&good, ByteOrder::Big, false).is_ok());
        // every truncation
        for n in 0..good.len() {
            assert!(GainTableMap::parse(&good[..n], ByteOrder::Big, false).is_err(), "truncated at {n}");
            assert!(GainTableMap::parse(&good[..n], ByteOrder::Big, true).is_err());
        }
        // wrong byte order is caught by the size check, and parse_any recovers
        assert!(GainTableMap::parse(&good, ByteOrder::Little, false).is_err());
        assert!(GainTableMap::parse_any(&good, ByteOrder::Little, false).is_ok());
        // huge counts
        let mut huge = good.clone();
        huge[0..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(GainTableMap::parse(&huge, ByteOrder::Big, false).is_err());
        let mut big = good.clone();
        big[0..4].copy_from_slice(&60000u32.to_be_bytes());
        big[4..8].copy_from_slice(&60000u32.to_be_bytes());
        assert!(GainTableMap::parse(&big, ByteOrder::Big, false).is_err(), "over MAX_ENTRIES");
        let mut zero = good.clone();
        zero[40..44].copy_from_slice(&0u32.to_be_bytes());
        assert!(GainTableMap::parse(&zero, ByteOrder::Big, false).is_err());
        // NaN / inf / negative values
        for (at, v) in [(8, f64::NAN), (16, 0.0), (24, f64::INFINITY), (32, f64::NAN)] {
            let mut b = good.clone();
            b[at..at + 8].copy_from_slice(&v.to_be_bytes());
            assert!(GainTableMap::parse(&b, ByteOrder::Big, false).is_err(), "f64 at {at}");
        }
        let mut w = good.clone();
        w[44..48].copy_from_slice(&f32::NAN.to_be_bytes());
        assert!(GainTableMap::parse(&w, ByteOrder::Big, false).is_err());
        for v in [f32::NAN, f32::INFINITY, -1.0] {
            let mut b = good.clone();
            b[64..68].copy_from_slice(&v.to_be_bytes());
            assert!(GainTableMap::parse(&b, ByteOrder::Big, false).is_err(), "gain {v}");
        }
        // random bytes never panic
        let mut s = 0x2545_f491_4f6c_dd1du64;
        for len in [0usize, 7, 63, 64, 80, 81, 200] {
            for _ in 0..200 {
                let b: Vec<u8> = (0..len)
                    .map(|_| {
                        s ^= s << 13;
                        s ^= s >> 7;
                        s ^= s << 17;
                        s as u8
                    })
                    .collect();
                if let Ok(m) = GainTableMap::parse_any(&b, ByteOrder::Big, len % 2 == 0) {
                    assert!(m.gain(0.5, 0.5, [0.5; 3]).is_finite());
                }
            }
        }
    }

    #[test]
    fn mapper_places_the_map_on_the_cropped_image() {
        let m = small();
        // a 4 × 2 image covering the right half of an 8 × 2 stage-3 image
        let p = GainMapper::new(&m, Rect::new(4, 0, 4, 2), 8, 2, 4, 2).unwrap();
        // pixel (3, 0) centre at stage-3 x 7.5 → u = 0.9375; v = 0.25
        let want = m.gain(0.9375, 0.25, [0.0, 0.25, 0.0]);
        assert!((p.gain_prophoto(3, 0, [0.0, 0.25, 0.0]) - want).abs() < 1e-6);
        // a neutral Rec.2020 colour stays neutral in ProPhoto: G = 0.25 there too
        let out = p.apply_rec2020(3, 0, [0.25; 3]);
        assert!(out.iter().all(|c| (c - 0.25 * want).abs() < 1e-3), "{out:?} vs {}", 0.25 * want);
        assert!(GainMapper::new(&m, Rect::new(0, 0, 0, 0), 8, 2, 4, 2).is_none());
    }
}
