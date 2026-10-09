//! Creative profiles: the colour tables camera-raw "Look" profiles carry.
//!
//! A creative profile (an XMP file with `crs:PresetType="Look"`, or the same fields inside a
//! preset that references the look) names its colour table by digest — `crs:RGBTable="<md5>"` —
//! and stores it in `crs:Table_<md5>`. Worked out from the data itself (no third-party code):
//!
//! - **Text encoding**: base 85 in 5-character groups, least significant digit first, each group
//!   giving 4 little-endian bytes (a final short group of k characters gives k − 1 bytes), with
//!   the digits `0-9 a-z A-Z . - : + = ^ ! / * ? \` ' | ( ) [ ] { } @ % $ #` (the Z85 alphabet
//!   with `` ` ' | `` in place of the XML-unsafe `& < >`).
//! - The decoded bytes are a little-endian `u32` uncompressed length, then a zlib stream.
//! - **RGB table** (uncompressed): `u32` 1, `u32` 1, `u32` dimensions (3), `u32` divisions
//!   per axis (n); n³ entries of three `u16` (red index slowest, blue fastest), each the change
//!   from the identity `i·65535/(n−1)` modulo 2¹⁶; then `u32` primaries, `u32` encoding,
//!   `u32` gamut handling and two `f64`s: the blend at amount 0 % and at 200 %.
//!   `crs:RGBTableAmount` is the blend at 100 % (default 1).
//!
//! The profile's other table kind (`crs:LookTable`, a hue/saturation/value table) is not
//! decoded yet; profiles that only have one are reported as unsupported.
//!
//! Profile files are the user's own (bought) data: they are read at runtime and copied into
//! their library, never shipped with LightCraft.

use lightcraft_pipeline::lut::{Lut3d, LutPrimaries, LutStage, LutTransfer};

/// The digits of the table text encoding, in value order.
const DIGITS: &[u8; 85] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ.-:+=^!/*?`'|()[]{}@%$#";
/// Longest table text accepted (a 64³ table compresses to well under this).
const MAX_TEXT: usize = 32 << 20;
/// Largest uncompressed table accepted.
const MAX_BYTES: usize = 8 << 20;
/// Most divisions per axis accepted (64³ entries).
const MAX_DIVISIONS: usize = 64;

/// Decode a `crs:Table_<md5>` value to the table's uncompressed bytes.
pub fn decode_table_text(text: &str) -> Result<Vec<u8>, String> {
    let text: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if text.len() > MAX_TEXT {
        return Err(format!("colour table too large ({} characters)", text.len()));
    }
    let mut value = [0u8; 256];
    for (i, &d) in DIGITS.iter().enumerate() {
        value[d as usize] = i as u8 + 1;
    }
    let mut bytes = Vec::with_capacity(text.len() / 5 * 4 + 4);
    for group in text.chunks(5) {
        if group.len() < 2 {
            return Err("colour table text ends in a partial group".into());
        }
        let mut v: u64 = 0;
        for (k, &c) in group.iter().enumerate() {
            let d = value[c as usize];
            if d == 0 {
                return Err(format!("unexpected character `{}` in colour table", c as char));
            }
            v += u64::from(d - 1) * 85u64.pow(k as u32);
        }
        if v > u64::from(u32::MAX) {
            return Err("colour table text out of range".into());
        }
        bytes.extend_from_slice(&v.to_le_bytes()[..group.len() - 1]);
    }
    let Some(len) = bytes.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) else {
        return Err("colour table too short".into());
    };
    if len > MAX_BYTES {
        return Err(format!("colour table too large ({len} bytes)"));
    }
    let data = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(bytes.get(4..).unwrap_or_default(), MAX_BYTES)
        .map_err(|e| format!("colour table: bad compressed data ({:?})", e.status))?;
    if data.len() != len {
        return Err(format!("colour table: {} bytes, expected {len}", data.len()));
    }
    Ok(data)
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn f64_at(b: &[u8], o: usize) -> Option<f64> {
    let x = b.get(o..o.checked_add(8)?)?;
    Some(f64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]))
}

/// The table's primaries from its header code (see the module docs; the codes in use were
/// matched against renders, `tools/lr-compare`).
fn primaries(code: u32) -> Option<LutPrimaries> {
    Some(match code {
        0 => LutPrimaries::Srgb,
        1 => LutPrimaries::AdobeRgb,
        2 => LutPrimaries::ProPhoto,
        3 => LutPrimaries::DisplayP3,
        4 => LutPrimaries::Rec2020,
        _ => return None,
    })
}

fn transfer(code: u32) -> Option<LutTransfer> {
    Some(match code {
        0 => LutTransfer::Linear,
        1 => LutTransfer::Srgb,
        2 => LutTransfer::Gamma(1.8),
        3 => LutTransfer::Gamma(2.2),
        _ => return None,
    })
}

/// An RGB table's uncompressed bytes as a profile LUT; `amount` is `crs:RGBTableAmount` (the
/// blend at 100 %).
pub fn rgb_table(b: &[u8], amount: f64) -> Result<Lut3d, String> {
    let (kind, dims, n) = (u32_at(b, 0), u32_at(b, 8), u32_at(b, 12));
    let (Some(1), Some(dims), Some(n)) = (kind, dims, n) else {
        return Err("not an RGB colour table".into());
    };
    if dims != 3 {
        return Err(format!("{dims}-dimensional colour tables are not supported"));
    }
    let n = n as usize;
    if !(2..=MAX_DIVISIONS).contains(&n) {
        return Err(format!("colour table with {n} divisions is not supported"));
    }
    let entries = n * n * n;
    let body = 16 + entries * 6;
    let samples = b.get(16..body).ok_or("colour table truncated")?;
    let ident = |i: usize| ((i * 65535 + (n - 1) / 2) / (n - 1)) as u16;
    let mut data = vec![[0.0f32; 3]; entries];
    // stored order: red slowest, blue fastest; ours: red fastest
    for r in 0..n {
        for g in 0..n {
            for bl in 0..n {
                let src = ((r * n + g) * n + bl) * 6;
                let Some(s) = samples.get(src..src + 6) else { return Err("colour table truncated".into()) };
                let at = |k: usize, i: usize| ident(i).wrapping_add(u16::from_le_bytes([s[2 * k], s[2 * k + 1]])) as f32 / 65535.0;
                if let Some(d) = data.get_mut(r + n * (g + n * bl)) {
                    *d = [at(0, r), at(1, g), at(2, bl)];
                }
            }
        }
    }
    let (prim, enc, gamut) = (u32_at(b, body), u32_at(b, body + 4), u32_at(b, body + 8));
    let (lo, hi) = (f64_at(b, body + 12), f64_at(b, body + 20));
    let primaries = match prim {
        Some(p) => primaries(p).ok_or_else(|| format!("colour table in an unknown colour space ({p})"))?,
        None => LutPrimaries::Srgb,
    };
    let transfer = match enc {
        Some(e) => transfer(e).ok_or_else(|| format!("colour table with an unknown encoding ({e})"))?,
        None => LutTransfer::Srgb,
    };
    let fin = |v: Option<f64>, d: f64| v.filter(|v| v.is_finite()).unwrap_or(d).clamp(0.0, 4.0) as f32;
    let mid = fin(Some(amount), 1.0);
    let lo = fin(lo, 0.0).min(mid);
    let hi = fin(hi, 2.0 * f64::from(mid)).max(mid);
    Ok(Lut3d {
        size: n,
        data,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        title: None,
        stage: LutStage::Profile,
        primaries,
        transfer,
        strength: [lo, mid, hi],
        // gamut handling 0 = clip to the table's space (as the DNG SDK applies it), 1 = extend
        clip: gamut == Some(0),
    })
}

/// A creative profile read from XMP.
#[derive(Clone, Debug)]
pub struct CrsProfile {
    pub name: String,
    /// `crs:Group` (the profile browser group), else `crs:Cluster`.
    pub group: Option<String>,
    pub uuid: Option<String>,
    /// Digest naming the RGB table.
    pub table_digest: String,
    pub lut: Lut3d,
    pub grayscale: bool,
}

fn prop<'a>(p: &'a crate::crs::Props, k: &str) -> Option<&'a str> {
    p.get(k).and_then(|v| v.first()).map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// The creative profile in an XMP packet (a profile file, or a preset carrying one): its name
/// and decoded RGB table. `Ok(None)` when the packet references no RGB table.
pub fn profile_from_xmp(xmp: &str) -> Result<Option<CrsProfile>, String> {
    let props = lightcraft_meta::parse_xmp(xmp).map_err(|e| e.to_string())?.properties;
    // a preset carries the look's fields under crs:Look/crs:Parameters; a profile file at the top
    let (base, look) = if prop(&props, "crs:RGBTable").is_some() { ("crs:", false) } else { ("crs:Look/crs:Parameters/crs:", true) };
    let Some(digest) = prop(&props, &format!("{base}RGBTable")).map(str::to_string) else {
        return Ok(None);
    };
    let text = prop(&props, &format!("crs:Table_{digest}"))
        .or_else(|| prop(&props, &format!("{base}Table_{digest}")))
        .ok_or_else(|| format!("colour table {digest} is referenced but not included"))?;
    let amount = prop(&props, &format!("{base}RGBTableAmount")).and_then(|a| a.parse::<f64>().ok()).unwrap_or(1.0);
    let lut = rgb_table(&decode_table_text(text)?, amount)?;
    let name_key = if look { "crs:Look/crs:Name" } else { "crs:Name" };
    let name = prop(&props, name_key).map(str::to_string).ok_or("the profile has no name")?;
    let group = prop(&props, if look { "crs:Look/crs:Group" } else { "crs:Group" })
        .or_else(|| prop(&props, if look { "crs:Look/crs:Cluster" } else { "crs:Cluster" }))
        .map(str::to_string);
    let uuid = prop(&props, if look { "crs:Look/crs:UUID" } else { "crs:UUID" }).map(str::to_string);
    let grayscale = prop(&props, &format!("{base}ConvertToGrayscale")).is_some_and(|v| v.eq_ignore_ascii_case("true"));
    Ok(Some(CrsProfile { name, group, uuid, table_digest: digest, lut, grayscale }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encode bytes the way the tables are stored (the inverse of [`decode_table_text`]).
    pub(crate) fn encode_text(raw: &[u8]) -> String {
        let mut b = (raw.len() as u32).to_le_bytes().to_vec();
        b.extend(miniz_oxide::deflate::compress_to_vec_zlib(raw, 6));
        let mut out = String::new();
        for chunk in b.chunks(4) {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            let mut v = u32::from_le_bytes(w) as u64;
            for _ in 0..=chunk.len() {
                out.push(DIGITS[(v % 85) as usize] as char);
                v /= 85;
            }
        }
        out
    }

    /// A synthetic RGB table: `f` maps a grid colour (0..1, encoded) to its output.
    pub(crate) fn table_bytes(n: usize, f: impl Fn([f64; 3]) -> [f64; 3], tail: (u32, u32, f64, f64)) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [1u32, 1, 3, n as u32] {
            b.extend(v.to_le_bytes());
        }
        let ident = |i: usize| ((i * 65535 + (n - 1) / 2) / (n - 1)) as u16;
        for r in 0..n {
            for g in 0..n {
                for bl in 0..n {
                    let x = [r, g, bl].map(|i| i as f64 / (n - 1) as f64);
                    let o = f(x);
                    for (k, i) in [r, g, bl].into_iter().enumerate() {
                        let out = (o[k].clamp(0.0, 1.0) * 65535.0).round() as u16;
                        b.extend(out.wrapping_sub(ident(i)).to_le_bytes());
                    }
                }
            }
        }
        for v in [tail.0, tail.1, 0] {
            b.extend(v.to_le_bytes());
        }
        b.extend(tail.2.to_le_bytes());
        b.extend(tail.3.to_le_bytes());
        b
    }

    pub(crate) fn xmp(name: &str, digest: &str, text: &str, amount: &str) -> String {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:PresetType="Look" crs:Cluster="Test" crs:RGBTable="{digest}" crs:Table_{digest}="{text}" crs:RGBTableAmount="{amount}"><crs:Name><rdf:Alt><rdf:li xml:lang="x-default">{name}</rdf:li></rdf:Alt></crs:Name><crs:Group><rdf:Alt><rdf:li xml:lang="x-default">Grp</rdf:li></rdf:Alt></crs:Group></rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    }

    #[test]
    fn round_trips_a_synthetic_table() {
        let f = |c: [f64; 3]| [c[0] * 0.9 + 0.05, c[1].powf(1.2), 1.0 - c[2] * 0.5];
        let raw = table_bytes(9, f, (0, 1, 0.0, 1.5));
        let text = encode_text(&raw);
        assert_eq!(decode_table_text(&text).unwrap(), raw);
        let lut = rgb_table(&raw, 0.6).unwrap();
        assert_eq!((lut.size, lut.stage, lut.primaries, lut.transfer), (9, LutStage::Profile, LutPrimaries::Srgb, LutTransfer::Srgb));
        assert_eq!(lut.strength, [0.0, 0.6, 1.5]);
        for c in [[0.0, 0.0, 0.0], [0.25, 0.5, 0.75], [1.0, 0.125, 0.875]] {
            let want = f(c);
            let got = lut.apply(c.map(|v| v as f32));
            for k in 0..3 {
                assert!((got[k] as f64 - want[k]).abs() < 2e-3, "{c:?}: {got:?} vs {want:?}");
            }
        }
        // blend: 0 → 0, 100 % → RGBTableAmount, 200 % → the stored maximum
        assert_eq!((lut.blend(0.0), lut.blend(1.0), lut.blend(2.0)), (0.0, 0.6, 1.5));
        assert!((lut.blend(0.5) - 0.3).abs() < 1e-6);
        let p = profile_from_xmp(&xmp("Fields", "ABC123", &text, "0.6")).unwrap().unwrap();
        assert_eq!((p.name.as_str(), p.group.as_deref(), p.table_digest.as_str()), ("Fields", Some("Grp"), "ABC123"));
        assert_eq!(p.lut, lut);
    }

    #[test]
    fn identity_table_changes_nothing() {
        let lut = rgb_table(&table_bytes(5, |c| c, (0, 1, 0.0, 2.0)), 1.0).unwrap();
        assert!(lut.clip, "gamut handling 0 clips to the table's space");
        let m = lut.matrices();
        for c in [[0.3f32, 0.25, 0.2], [0.5, 0.5, 0.5], [0.05, 0.1, 0.08]] {
            let o = lut.apply_linear(c, 1.0, &m);
            for k in 0..3 {
                assert!((o[k] - c[k]).abs() < 2e-3, "{c:?} → {o:?}");
            }
        }
        // outside the table's gamut: clipped to it (as the DNG SDK applies a "gamut clip" table)…
        let wide = [1.4f32, 0.2, -0.05];
        let clipped = lut.apply_linear(wide, 1.0, &m);
        let back = lightcraft_color::WORKING.to_space(&lightcraft_color::SRGB).apply(clipped.map(f64::from));
        assert!(back.iter().all(|v| (-1e-3..=1.0 + 1e-3).contains(v)), "{back:?}");
        // …or, for a "gamut extend" table, keeping its offset
        let extend = rgb_table(&table_bytes(5, |c| c, (0, 1, 0.0, 2.0)), 1.0).map(|l| Lut3d { clip: false, ..l }).unwrap();
        let o = extend.apply_linear(wide, 1.0, &m);
        for k in 0..3 {
            assert!((o[k] - wide[k]).abs() < 2e-3, "{wide:?} → {o:?}");
        }
    }

    #[test]
    fn bad_tables_are_errors_not_panics() {
        let good = encode_text(&table_bytes(4, |c| c, (0, 1, 0.0, 2.0)));
        // truncated text, every prefix
        for cut in [0, 1, 3, 4, 5, 7, 12, good.len() / 2, good.len() - 1] {
            let _ = decode_table_text(&good[..cut]);
        }
        assert!(decode_table_text("").is_err());
        assert!(decode_table_text("<<<<<").is_err(), "characters outside the alphabet");
        assert!(decode_table_text("#####").is_err(), "a group above 2³²");
        // a header claiming a huge table
        let mut huge = (u32::MAX).to_le_bytes().to_vec();
        huge.extend(miniz_oxide::deflate::compress_to_vec_zlib(&[0u8; 16], 6));
        let text: String = huge
            .chunks(4)
            .flat_map(|c| {
                let mut w = [0u8; 4];
                w[..c.len()].copy_from_slice(c);
                let mut v = u32::from_le_bytes(w) as u64;
                (0..=c.len()).map(move |_| {
                    let d = DIGITS[(v % 85) as usize] as char;
                    v /= 85;
                    d
                })
            })
            .collect();
        assert!(decode_table_text(&text).is_err());
        // decoded tables: wrong kind, dimensions, divisions, truncated bodies, odd tails
        let raw = table_bytes(4, |c| c, (0, 1, 0.0, 2.0));
        for cut in 0..raw.len() {
            let _ = rgb_table(&raw[..cut], 1.0);
        }
        let mut b = raw.clone();
        b[12..16].copy_from_slice(&100_000u32.to_le_bytes());
        assert!(rgb_table(&b, 1.0).is_err(), "huge division count");
        b[12..16].copy_from_slice(&1u32.to_le_bytes());
        assert!(rgb_table(&b, 1.0).is_err(), "one division");
        let mut b = raw.clone();
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(rgb_table(&b, 1.0).is_err(), "1-D table");
        let mut b = raw.clone();
        let tail = 16 + 64 * 6;
        b[tail..tail + 4].copy_from_slice(&99u32.to_le_bytes());
        assert!(rgb_table(&b, 1.0).is_err(), "unknown colour space");
        let mut b = raw.clone();
        b[tail + 12..tail + 20].copy_from_slice(&f64::NAN.to_le_bytes());
        b[tail + 20..tail + 28].copy_from_slice(&f64::INFINITY.to_le_bytes());
        let l = rgb_table(&b, f64::NAN).unwrap();
        assert!(l.strength.iter().all(|v| v.is_finite()), "{:?}", l.strength);
        // an XMP packet referencing a table it doesn't carry
        assert!(profile_from_xmp(&xmp("X", "D1", "", "1")).is_err());
        assert!(profile_from_xmp(&xmp("X", "D1", "!!!!!", "1")).is_err());
    }
}
