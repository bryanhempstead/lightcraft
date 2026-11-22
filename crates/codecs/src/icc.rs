//! ICC profiles: parsing (via `moxcms`), matrix/TRC extraction, CMS fallback for LUT/CMYK profiles,
//! and writing matrix/TRC profiles for export embedding.

use crate::space::{NamedSpace, Trc, rgb_to_xyz_d50};
use lightcraft_color::{D50, Mat3, RgbSpace, bradford};
use moxcms::{
    ColorProfile, DataColorSpace, Layout, LocalizableString, Matrix3d, ProfileText, RenderingIntent, ToneReprCurve, TransformOptions, Xyzd,
};
use serde::{Deserialize, Serialize};

/// Colour model of the profile's device space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IccColorModel {
    Rgb,
    Gray,
    Cmyk,
    Other,
}

/// How a profile can be applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum IccKind {
    /// RGB matrix/TRC (shaper) profile: exact, applied by us.
    MatrixTrc { to_xyz_d50: Mat3, trc: [Trc; 3] },
    /// Grayscale profile with a `kTRC`.
    Gray { trc: Trc },
    /// LUT-based (A2B), CMYK or otherwise needs a full CMS conversion.
    NeedsCms,
}

/// Summary of a parsed ICC profile.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IccInfo {
    pub model: IccColorModel,
    pub kind: IccKind,
    /// Recognised well-known RGB space (by colorants), if any.
    pub named: Option<NamedSpace>,
    pub description: Option<String>,
}

/// Parse an ICC profile. Returns `None` for malformed or unsupported data (never panics).
pub fn parse(bytes: &[u8]) -> Option<IccInfo> {
    let p = std::panic::catch_unwind(|| ColorProfile::new_from_slice(bytes)).ok()?.ok()?;
    Some(info_of(&p))
}

fn info_of(p: &ColorProfile) -> IccInfo {
    let description = p.description.as_ref().and_then(text_of);
    let model = match p.color_space {
        DataColorSpace::Rgb => IccColorModel::Rgb,
        DataColorSpace::Gray => IccColorModel::Gray,
        DataColorSpace::Cmyk => IccColorModel::Cmyk,
        _ => IccColorModel::Other,
    };
    let has_lut = p.lut_a_to_b_perceptual.is_some() || p.lut_a_to_b_colorimetric.is_some();
    let kind = if model == IccColorModel::Rgb && p.is_matrix_shaper() && p.pcs == DataColorSpace::Xyz {
        let c = [p.red_colorant, p.green_colorant, p.blue_colorant];
        let to_xyz_d50 = Mat3([[c[0].x, c[1].x, c[2].x], [c[0].y, c[1].y, c[2].y], [c[0].z, c[1].z, c[2].z]]);
        let trcs = [&p.red_trc, &p.green_trc, &p.blue_trc];
        let trc = trcs.map(|t| t.as_ref().map(trc_from_icc).unwrap_or(Trc::Srgb));
        if to_xyz_d50.determinant().abs() > 1e-6 && trc.iter().all(valid_trc) { IccKind::MatrixTrc { to_xyz_d50, trc } } else { IccKind::NeedsCms }
    } else if model == IccColorModel::Gray && p.gray_trc.is_some() && !has_lut {
        let trc = p.gray_trc.as_ref().map(trc_from_icc).unwrap_or(Trc::Srgb);
        if valid_trc(&trc) { IccKind::Gray { trc } } else { IccKind::NeedsCms }
    } else {
        IccKind::NeedsCms
    };
    let named = match &kind {
        IccKind::MatrixTrc { to_xyz_d50, .. } => NamedSpace::recognize(to_xyz_d50),
        _ => None,
    };
    IccInfo { model, kind, named, description }
}

fn valid_trc(t: &Trc) -> bool {
    match t {
        Trc::Parametric(p) => matches!(p.len(), 1 | 3 | 4 | 5 | 7) && p.iter().all(|v| v.is_finite()),
        Trc::Gamma(g) => g.is_finite() && *g > 0.0,
        _ => true,
    }
}

fn text_of(t: &ProfileText) -> Option<String> {
    let s = match t {
        ProfileText::PlainString(s) => s.clone(),
        ProfileText::Localizable(v) => v.first().map(|l| l.value.clone())?,
        ProfileText::Description(d) => {
            if d.unicode_string.is_empty() {
                d.ascii_string.clone()
            } else {
                d.unicode_string.clone()
            }
        }
    };
    let s = s.trim_end_matches('\0').trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn trc_from_icc(t: &ToneReprCurve) -> Trc {
    match t {
        ToneReprCurve::Lut(v) if v.is_empty() => Trc::Linear,
        ToneReprCurve::Lut(v) if v.len() == 1 => Trc::Gamma(v[0] as f32 / 256.0),
        ToneReprCurve::Lut(v) => Trc::Table(v.clone()),
        ToneReprCurve::Parametric(p) => Trc::Parametric(p.clone()),
    }
}

fn trc_to_icc(t: &Trc) -> ToneReprCurve {
    match t {
        Trc::Linear => ToneReprCurve::Parametric(vec![1.0]),
        Trc::Srgb => ToneReprCurve::Parametric(vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045]),
        Trc::Gamma(g) => ToneReprCurve::Parametric(vec![*g]),
        Trc::Rec709 => ToneReprCurve::Parametric(vec![1.0 / 0.45, 1.0 / 1.099, 0.099 / 1.099, 1.0 / 4.5, 0.081]),
        Trc::Parametric(p) => ToneReprCurve::Parametric(p.clone()),
        Trc::Table(v) => ToneReprCurve::Lut(v.clone()),
    }
}

fn xyzd(v: [f64; 3]) -> Xyzd {
    Xyzd { x: v[0], y: v[1], z: v[2] }
}

/// ICC PCS illuminant (D50) as stored in profiles.
const ICC_D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// Build an ICC v4 display-class matrix/TRC profile for `space` with the given encoding curve
/// (D50 PCS, Bradford `chad`), for embedding in exported files.
pub fn write_matrix_trc(space: &RgbSpace, trc: &Trc) -> Vec<u8> {
    let m = rgb_to_xyz_d50(space).0;
    let mut p = ColorProfile::new_srgb();
    p.cicp = None;
    p.red_colorant = xyzd([m[0][0], m[1][0], m[2][0]]);
    p.green_colorant = xyzd([m[0][1], m[1][1], m[2][1]]);
    p.blue_colorant = xyzd([m[0][2], m[1][2], m[2][2]]);
    let curve = trc_to_icc(trc);
    p.red_trc = Some(curve.clone());
    p.green_trc = Some(curve.clone());
    p.blue_trc = Some(curve);
    p.white_point = xyzd(ICC_D50);
    p.media_white_point = Some(xyzd(ICC_D50));
    p.chromatic_adaptation = Some(Matrix3d { v: bradford(space.white, D50).0 });
    p.rendering_intent = RenderingIntent::Perceptual;
    let desc = match trc {
        t if t.is_linear() => format!("{} (linear)", space.name),
        _ => space.name.to_string(),
    };
    p.description = Some(ProfileText::Localizable(vec![LocalizableString::new("en".into(), "US".into(), desc)]));
    p.copyright = Some(ProfileText::Localizable(vec![LocalizableString::new("en".into(), "US".into(), "No copyright, use freely".into())]));
    align_tags(&fixed_date(p.encode().unwrap_or_default()))
}

/// Zero the header's creation date/time (ICC.1 §7.2.8, bytes 24..36) so a profile's bytes depend
/// only on its colour space and curve. `moxcms` stamps the wall clock there on every `encode`
/// (ignoring `ColorProfile::creation_date_time`), which made two exports of one photo differ in
/// the embedded profile and nowhere else, so output hashes could not be compared (#180). Readers
/// do not interpret the field; all zeros is the conventional "unspecified" value.
fn fixed_date(mut b: Vec<u8>) -> Vec<u8> {
    if let Some(date) = b.get_mut(24..36) {
        date.fill(0);
    }
    b
}

/// Re-lay a profile's tag data on 4-byte boundaries (ICC.1 §7.3.1). The encoder packs tags back to
/// back, so a tag of odd length (e.g. a `mluc` description with an odd number of characters)
/// misaligns every following tag — which some readers (macOS ImageIO for PNG `iCCP`) reject.
fn align_tags(b: &[u8]) -> Vec<u8> {
    let be = |i: usize| b.get(i..i + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let Some(n) = be(128) else { return b.to_vec() };
    let table_end = 132 + 12 * n;
    let tags: Option<Vec<(usize, usize)>> = (0..n).map(|t| Some((be(132 + 12 * t + 4)?, be(132 + 12 * t + 8)?))).collect();
    let Some(tags) = tags.filter(|t| table_end <= b.len() && t.iter().all(|&(o, l)| o >= table_end && o + l <= b.len())) else {
        return b.to_vec();
    };
    let mut out = b[..table_end].to_vec();
    let mut placed: Vec<((usize, usize), usize)> = Vec::new();
    for (t, &(off, len)) in tags.iter().enumerate() {
        let new_off = match placed.iter().find(|(k, _)| *k == (off, len)) {
            // tags sharing data keep sharing it
            Some(&(_, o)) => o,
            None => {
                while !out.len().is_multiple_of(4) {
                    out.push(0);
                }
                let o = out.len();
                out.extend_from_slice(&b[off..off + len]);
                placed.push(((off, len), o));
                o
            }
        };
        out[132 + 12 * t + 4..132 + 12 * t + 8].copy_from_slice(&(new_off as u32).to_be_bytes());
    }
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
    let size = out.len() as u32;
    out[0..4].copy_from_slice(&size.to_be_bytes());
    // the profile ID (MD5) would no longer match: clear it (= "not computed")
    out[84..100].fill(0);
    out
}

/// A profile for one of the well-known spaces with its standard curve, as Lightroom embeds them:
/// ICC v2 "sRGB IEC61966-2.1" and "Adobe RGB (1998)" (the forms every reader knows; their
/// colorants and curves generated here from the published specs), v4 for the others.
pub fn write_named(n: NamedSpace) -> Vec<u8> {
    match n {
        NamedSpace::Srgb => write_v2(&n.rgb_space(), &n.trc(), "sRGB IEC61966-2.1"),
        NamedSpace::AdobeRgb => write_v2(&n.rgb_space(), &n.trc(), "Adobe RGB (1998)"),
        _ => write_matrix_trc(&n.rgb_space(), &n.trc()),
    }
}

/// An ICC v2.1 display-class matrix/TRC profile for `space` (D50 PCS, Bradford-adapted colorants,
/// the media white point = the space's own white, as v2 profiles state it), its curve `curv` (a
/// pure gamma in u8Fixed8 when it is one exactly, else a 1024-entry table), and `desc` as the
/// `textDescriptionType` description. Deterministic: no date, no profile ID.
pub fn write_v2(space: &RgbSpace, trc: &Trc, desc: &str) -> Vec<u8> {
    let m = rgb_to_xyz_d50(space).0;
    let s15 = |v: f64| ((v * 65536.0).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32).to_be_bytes();
    let xyz = |v: [f64; 3]| -> Vec<u8> {
        let mut t = b"XYZ \0\0\0\0".to_vec();
        for c in v {
            t.extend_from_slice(&s15(c));
        }
        t
    };
    let text = |s: &str| -> Vec<u8> {
        let mut t = b"text\0\0\0\0".to_vec();
        t.extend(s.bytes().filter(u8::is_ascii));
        t.push(0);
        t
    };
    let description = {
        let ascii: Vec<u8> = desc.bytes().filter(u8::is_ascii).collect();
        let mut t = b"desc\0\0\0\0".to_vec();
        t.extend_from_slice(&(ascii.len() as u32 + 1).to_be_bytes());
        t.extend_from_slice(&ascii);
        t.push(0);
        // no Unicode or ScriptCode strings: language + count 0, code + count 0, 67 bytes
        t.extend_from_slice(&[0; 8]);
        t.extend_from_slice(&[0; 3]);
        t.extend_from_slice(&[0; 67]);
        t
    };
    let curve = {
        let mut t = b"curv\0\0\0\0".to_vec();
        let gamma = match trc {
            Trc::Gamma(g) if ((*g as f64) * 256.0).fract() == 0.0 && *g > 0.0 && *g < 255.0 => Some(*g as f64),
            Trc::Linear => Some(1.0),
            _ => None,
        };
        match gamma {
            Some(g) => {
                t.extend_from_slice(&1u32.to_be_bytes());
                t.extend_from_slice(&((g * 256.0).round() as u16).to_be_bytes());
            }
            None => {
                const N: usize = 1024;
                t.extend_from_slice(&(N as u32).to_be_bytes());
                for i in 0..N {
                    let v = trc.to_linear(i as f32 / (N - 1) as f32).clamp(0.0, 1.0);
                    t.extend_from_slice(&((v as f64 * 65535.0).round() as u16).to_be_bytes());
                }
            }
        }
        t
    };
    let w = space.white;
    let white = [w.x / w.y, 1.0, (1.0 - w.x - w.y) / w.y];
    let col = |j: usize| [m[0][j], m[1][j], m[2][j]];
    // tag data, then the tags pointing at it (the three curves share one)
    let data: Vec<(Vec<u8>, &[&[u8; 4]])> = vec![
        (description, &[b"desc"]),
        (text("No copyright, use freely"), &[b"cprt"]),
        (xyz(white), &[b"wtpt"]),
        (xyz(col(0)), &[b"rXYZ"]),
        (xyz(col(1)), &[b"gXYZ"]),
        (xyz(col(2)), &[b"bXYZ"]),
        (curve, &[b"rTRC", b"gTRC", b"bTRC"]),
    ];
    let n_tags: usize = data.iter().map(|(_, sigs)| sigs.len()).sum();
    let mut table = Vec::new();
    let mut body = Vec::new();
    let mut off = 128 + 4 + 12 * n_tags;
    for (bytes, sigs) in &data {
        for sig in sigs.iter() {
            table.extend_from_slice(*sig);
            table.extend_from_slice(&(off as u32).to_be_bytes());
            table.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        }
        body.extend_from_slice(bytes);
        off += bytes.len();
        while !body.len().is_multiple_of(4) {
            body.push(0);
            off += 1;
        }
    }
    let size = 128 + 4 + table.len() + body.len();
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&(size as u32).to_be_bytes());
    out.extend_from_slice(&[0; 4]); // preferred CMM: none
    out.extend_from_slice(&[0x02, 0x10, 0, 0]); // version 2.1.0
    out.extend_from_slice(b"mntrRGB XYZ ");
    out.extend_from_slice(&[0; 12]); // date: unspecified
    out.extend_from_slice(b"acsp");
    out.extend_from_slice(b"APPL");
    out.extend_from_slice(&[0; 4]); // flags
    out.extend_from_slice(&[0; 8]); // manufacturer, model
    out.extend_from_slice(&[0; 8]); // attributes: reflective, glossy, positive, colour
    out.extend_from_slice(&0u32.to_be_bytes()); // perceptual
    for c in ICC_D50 {
        out.extend_from_slice(&s15(c));
    }
    out.extend_from_slice(&[0; 4]); // creator
    out.resize(128, 0);
    out.extend_from_slice(&(n_tags as u32).to_be_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

fn linear_rec2020_profile() -> ColorProfile {
    let mut p = ColorProfile::new_bt2020();
    p.cicp = None;
    let lin = ToneReprCurve::Parametric(vec![1.0]);
    p.red_trc = Some(lin.clone());
    p.green_trc = Some(lin.clone());
    p.blue_trc = Some(lin);
    p
}

/// ColorMatch RGB (E08), from its published numbers: D50 white, gamma 1.8, primaries
/// 0.630/0.340, 0.295/0.605, 0.150/0.075.
pub fn colormatch_profile() -> Vec<u8> {
    let space = RgbSpace {
        name: "ColorMatch RGB",
        r: lightcraft_color::Xy::new(0.630, 0.340),
        g: lightcraft_color::Xy::new(0.295, 0.605),
        b: lightcraft_color::Xy::new(0.150, 0.075),
        white: D50,
    };
    write_v2(&space, &Trc::Gamma(1.8), "ColorMatch RGB")
}

/// Largest ICC profile an export takes (printer profiles with big LUTs are a few MB).
pub const MAX_EXPORT_ICC: usize = 16 << 20;

/// Whether `icc` can be an export's target profile: it parses, and its device space is RGB.
pub fn check_export_profile(icc: &[u8]) -> std::result::Result<(), String> {
    if icc.len() > MAX_EXPORT_ICC {
        return Err("the profile is larger than 16 MB".into());
    }
    let p = std::panic::catch_unwind(|| ColorProfile::new_from_slice(icc)).map_err(|_| "the profile can't be read".to_string())?;
    let p = p.map_err(|e| format!("not an ICC profile: {e:?}"))?;
    if p.color_space != DataColorSpace::Rgb {
        return Err("an export profile must be an RGB profile (this is not)".into());
    }
    Ok(())
}

/// Convert interleaved RGB / RGBA samples (`channels` 3 or 4; alpha kept) in place from profile
/// `from` to profile `to` (relative colorimetric). `max` is the samples' full scale (255 or 65535).
pub fn convert_samples(from: &[u8], to: &[u8], samples: &mut [f32], channels: usize) -> std::result::Result<(), String> {
    let run = || -> std::result::Result<(), String> {
        let src = ColorProfile::new_from_slice(from).map_err(|e| format!("source profile: {e:?}"))?;
        let dst = ColorProfile::new_from_slice(to).map_err(|e| format!("target profile: {e:?}"))?;
        let layout = match channels {
            3 => Layout::Rgb,
            4 => Layout::Rgba,
            _ => return Err("RGB or RGBA samples only".into()),
        };
        let opts = TransformOptions { rendering_intent: RenderingIntent::RelativeColorimetric, ..TransformOptions::default() };
        let t = src.create_transform_f32(layout, &dst, layout, opts).map_err(|e| format!("profile transform: {e:?}"))?;
        const PIXELS: usize = 1 << 16;
        let mut out = vec![0f32; PIXELS * channels];
        for chunk in samples.chunks_mut(PIXELS * channels) {
            let o = out.get_mut(..chunk.len()).ok_or("buffer")?;
            t.transform(chunk, o).map_err(|e| format!("profile transform: {e:?}"))?;
            chunk.copy_from_slice(o);
        }
        Ok(())
    };
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("the profile transform failed".into()))
}

/// Convert device samples (0..1, interleaved, `channels` = 1, 3 or 4 matching the profile's colour
/// model) to linear Rec.2020 through a full CMS transform. `None` if the profile cannot be applied.
pub(crate) fn cms_to_linear_rec2020(icc: &[u8], samples: &[f32], channels: usize) -> Option<Vec<[f32; 3]>> {
    let src = ColorProfile::new_from_slice(icc).ok()?;
    let layout = match (src.color_space, channels) {
        (DataColorSpace::Rgb, 3) => Layout::Rgb,
        (DataColorSpace::Gray, 1) => Layout::Gray,
        (DataColorSpace::Cmyk, 4) => Layout::Rgba,
        _ => return None,
    };
    let dst = linear_rec2020_profile();
    let opts = TransformOptions { rendering_intent: RenderingIntent::RelativeColorimetric, ..TransformOptions::default() };
    let run = || -> Option<Vec<[f32; 3]>> {
        let t = src.create_transform_f32(layout, &dst, Layout::Rgb, opts).ok()?;
        let n = samples.len() / channels;
        let mut out = vec![0f32; n * 3];
        // Chunk to bound temporary memory and allow parallelism.
        const ROWS: usize = 1 << 16;
        let src_chunks = samples.chunks(ROWS * channels);
        let dst_chunks = out.chunks_mut(ROWS * 3);
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            let pairs: Vec<_> = src_chunks.zip(dst_chunks).collect();
            let ok = pairs.into_par_iter().all(|(s, d)| t.transform(s, d).is_ok());
            if !ok {
                return None;
            }
        }
        #[cfg(not(feature = "parallel"))]
        for (s, d) in src_chunks.zip(dst_chunks) {
            t.transform(s, d).ok()?;
        }
        Some(out.as_chunks::<3>().0.iter().map(|c| [c[0], c[1], c[2]]).collect())
    };
    std::panic::catch_unwind(run).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_color::{DISPLAY_P3, PROPHOTO, SRGB};

    #[test]
    fn write_then_parse_recognises() {
        for n in NamedSpace::ALL {
            let bytes = write_named(n);
            assert!(bytes.len() > 128, "{n:?}");
            let info = parse(&bytes).expect("parses");
            assert_eq!(info.named, Some(n), "{n:?}");
            match info.kind {
                IccKind::MatrixTrc { trc, .. } => assert!(trc[0].approx_eq(&n.trc(), 2e-4), "{n:?} {:?}", trc[0]),
                k => panic!("{k:?}"),
            }
        }
    }

    #[test]
    fn tags_are_aligned() {
        for n in NamedSpace::ALL {
            for trc in [n.trc(), Trc::Linear] {
                let b = write_matrix_trc(&n.rgb_space(), &trc);
                assert_eq!(b.len() % 4, 0, "{n:?}");
                assert_eq!(u32::from_be_bytes(b[0..4].try_into().unwrap()) as usize, b.len());
                let count = u32::from_be_bytes(b[128..132].try_into().unwrap()) as usize;
                for t in 0..count {
                    let off = u32::from_be_bytes(b[132 + 12 * t + 4..132 + 12 * t + 8].try_into().unwrap());
                    assert_eq!(off % 4, 0, "{n:?} tag {t}");
                }
                assert!(parse(&b).is_some_and(|i| i.named == Some(n)), "{n:?}");
            }
        }
    }

    #[test]
    fn linear_profile() {
        let bytes = write_matrix_trc(&PROPHOTO, &Trc::Linear);
        let info = parse(&bytes).unwrap();
        assert_eq!(info.named, Some(NamedSpace::ProPhoto));
        let IccKind::MatrixTrc { trc, .. } = info.kind else { panic!() };
        assert!(trc[1].is_linear());
    }

    #[test]
    fn cms_path_matches_matrix_path() {
        let bytes = write_named(NamedSpace::DisplayP3);
        let px = [0.2f32, 0.5, 0.8];
        let out = cms_to_linear_rec2020(&bytes, &px, 3).unwrap()[0];
        let lin = px.map(|v| Trc::Srgb.to_linear(v));
        let m = DISPLAY_P3.to_space(&lightcraft_color::REC2020);
        let want = m.apply_f32(lin);
        for i in 0..3 {
            assert!((out[i] - want[i]).abs() < 2e-3, "{out:?} vs {want:?}");
        }
        let _ = SRGB;
    }

    /// Two profiles for one space are byte-identical, and the header date is fixed: exports of the
    /// same photo with the same settings hash the same (#180).
    #[test]
    fn written_profiles_are_deterministic() {
        for n in NamedSpace::ALL {
            let a = write_named(n);
            let b = write_named(n);
            assert_eq!(a, b, "{n:?}");
            assert_eq!(&a[24..36], &[0u8; 12], "{n:?} header date/time");
            // the profile ID is "not computed", not a hash of a timestamp
            assert_eq!(&a[84..100], &[0u8; 16], "{n:?} profile ID");
        }
        let lin = write_matrix_trc(&PROPHOTO, &Trc::Linear);
        assert_eq!(lin, write_matrix_trc(&PROPHOTO, &Trc::Linear));
        assert_eq!(&lin[24..36], &[0u8; 12]);
    }

    /// sRGB and Adobe RGB are embedded as ICC v2 like Lightroom's files: version 2.1, the
    /// descriptions Lightroom's profiles carry, the published colorants (Bradford to D50; the
    /// values HP's and Adobe's v2 profiles state, to 1e-4) and curves (sRGB as a 1024-entry table,
    /// Adobe RGB as gamma 563/256), and every reader we have recognises them.
    #[test]
    fn srgb_and_adobe_rgb_are_v2_like_lightrooms() {
        let be32 = |b: &[u8], i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let tag = |b: &[u8], sig: &[u8; 4]| -> Option<(usize, usize)> {
            (0..be32(b, 128) as usize)
                .map(|t| 132 + 12 * t)
                .find(|&e| &b[e..e + 4] == sig)
                .map(|e| (be32(b, e + 4) as usize, be32(b, e + 8) as usize))
        };
        let s15 = |b: &[u8], i: usize| i32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as f64 / 65536.0;
        for (n, desc, red) in
            [(NamedSpace::Srgb, "sRGB IEC61966-2.1", [0.4361, 0.2225, 0.0139]), (NamedSpace::AdobeRgb, "Adobe RGB (1998)", [0.6097, 0.3111, 0.0195])]
        {
            let b = write_named(n);
            assert_eq!(be32(&b, 0) as usize, b.len());
            assert_eq!(&b[8..12], &[0x02, 0x10, 0, 0], "{n:?}: v2.1");
            assert_eq!(&b[12..24], b"mntrRGB XYZ ");
            assert_eq!(&b[36..40], b"acsp");
            for (off, len) in
                ["desc", "cprt", "wtpt", "rXYZ", "gXYZ", "bXYZ", "rTRC", "gTRC", "bTRC"].map(|t| tag(&b, t.as_bytes().try_into().unwrap()).unwrap())
            {
                assert!(off % 4 == 0 && off + len <= b.len());
            }
            let (r, _) = tag(&b, b"rXYZ").unwrap();
            for k in 0..3 {
                assert!((s15(&b, r + 8 + 4 * k) - red[k]).abs() < 1e-4, "{n:?} rXYZ");
            }
            let (wp, _) = tag(&b, b"wtpt").unwrap();
            assert!((s15(&b, wp + 8) - 0.9505).abs() < 1e-3 && (s15(&b, wp + 16) - 1.0891).abs() < 1e-3, "D65 media white");
            let (c, _) = tag(&b, b"rTRC").unwrap();
            assert_eq!(&b[c..c + 4], b"curv");
            let count = be32(&b, c + 8);
            match n {
                NamedSpace::AdobeRgb => assert_eq!((count, &b[c + 12..c + 14]), (1, &[0x02, 0x33][..])),
                _ => {
                    assert_eq!(count, 1024);
                    let at = |i: usize| u16::from_be_bytes([b[c + 12 + 2 * i], b[c + 13 + 2 * i]]) as f64 / 65535.0;
                    for i in [0, 10, 300, 700, 1023] {
                        let want = Trc::Srgb.to_linear(i as f32 / 1023.0) as f64;
                        assert!((at(i) - want).abs() < 2e-5, "{i}");
                    }
                }
            }
            let info = parse(&b).expect("parses");
            assert_eq!(info.named, Some(n));
            assert_eq!(info.description.as_deref(), Some(desc));
            assert!(matches!(info.kind, IccKind::MatrixTrc { .. }));
            assert_eq!(b, write_named(n), "deterministic");
        }
        // Display P3 stays v4, as Apple's (and Lightroom's) is
        let p3 = write_named(NamedSpace::DisplayP3);
        assert_eq!(p3[8], 4);
        assert_eq!(parse(&p3).unwrap().description.as_deref(), Some("Display P3"));
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0u8; 200]).is_none());
    }
}
