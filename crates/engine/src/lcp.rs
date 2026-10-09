//! Adobe lens profiles (`.lcp`) installed with Camera Raw / Lightroom (Bryan's fork, AGENTS.md →
//! *Fork rules*): read at run time, never copied. A raw file without lens-correction opcodes of
//! its own gets its lens's profile as the same DNG-style corrections LightCraft already applies
//! with "Enable Profile Corrections" (`WarpRectilinear` distortion, `FixVignetteRadial`
//! vignetting), the way Adobe's DNG Converter turns a profile into opcodes.
//!
//! The Adobe Camera Model: for a pixel `(u, v)` of the `W × H` image (`Dmax` = the longer side),
//! `x = (u − cx·W) / (fx·Dmax)`, `y = (v − cy·H) / (fy·Dmax)`, `r² = x² + y²`; the lens maps
//! an ideal point to `x·(1 + k1 r² + k2 r⁴ + k3 r⁶)` and darkens it by
//! `1 + α1 r² + α2 r⁴ + α3 r⁶`. Entries are interpolated in aperture (APEX) and focus distance
//! (1 / d) for the photo's focal length.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use lightcraft_develop::{EmbeddedLens, EmbeddedVignette, EmbeddedWarp};
use lightcraft_geom::Point;

/// Largest `.lcp` read.
const MAX_LCP: u64 = 16 << 20;

/// Folders holding lens profiles: Camera Raw's (system, user) and Lightroom Classic's own.
fn dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = crate::adobe::camera_raw_dirs().into_iter().map(|d| d.join("LensProfiles/1.0")).collect();
    if std::env::var_os("LIGHTCRAFT_ADOBE_CAMERARAW").is_none() && cfg!(target_os = "macos") {
        v.push(PathBuf::from("/Applications/Adobe Lightroom Classic/Adobe Lightroom Classic.app/Contents/Resources/LensProfiles/1.0"));
    }
    v
}

/// Lower-case letters and digits only (`EF24mm f/1.4L II USM` → `ef24mmf14liiusm`).
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// Every `.lcp` by its lens part (the file name's last parenthesis), normalised.
fn index() -> &'static Vec<(String, PathBuf)> {
    static I: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    I.get_or_init(|| {
        let mut out = Vec::new();
        for d in dirs() {
            let mut stack = vec![(d, 0usize)];
            while let Some((dir, depth)) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                for e in rd.flatten() {
                    if out.len() > 50_000 {
                        break;
                    }
                    let p = e.path();
                    if p.is_dir() && depth < 4 {
                        stack.push((p, depth + 1));
                    } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lcp")) {
                        let name = e.file_name().to_string_lossy().into_owned();
                        let lens = name.rfind('(').and_then(|a| name[a + 1..].find(')').map(|b| &name[a + 1..a + 1 + b])).unwrap_or(&name);
                        out.push((norm(lens), p));
                    }
                }
            }
        }
        out
    })
}

/// One profile entry (`rdf:li`) of an `.lcp`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub make: String,
    pub model: String,
    pub lens: String,
    pub raw: bool,
    pub focal: f64,
    pub distance: f64,
    /// APEX aperture value (`2·log2(N)`).
    pub aperture: f64,
    pub fisheye: bool,
    /// Perspective model: fx, fy, cx, cy (in units of the long side), k1, k2, k3.
    pub geom: Option<[f64; 7]>,
    /// Vignette model: fx, fy, cx, cy, α1, α2, α3.
    pub vignette: Option<[f64; 7]>,
}

fn field(block: &str, name: &str) -> Option<String> {
    let tag = format!("<stCamera:{name}>");
    if let Some(a) = block.find(&tag) {
        let rest = &block[a + tag.len()..];
        let b = rest.find('<')?;
        return Some(rest[..b].trim().to_string());
    }
    let attr = format!("stCamera:{name}=\"");
    let a = block.find(&attr)?;
    let rest = &block[a + attr.len()..];
    let b = rest.find('"')?;
    Some(rest[..b].trim().to_string())
}

fn num(block: &str, name: &str) -> Option<f64> {
    field(block, name)?.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The element `name`'s content (`<stCamera:name …>…</stCamera:name>`), if any.
fn element<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<stCamera:{name}");
    let close = format!("</stCamera:{name}>");
    let a = block.find(&open)?;
    // a self-closing element (`<stCamera:name a="…"/>`) holds only its attributes
    let gt = block[a..].find('>')? + a;
    if block[..gt].ends_with('/') {
        return block.get(a + open.len()..gt - 1);
    }
    let b = block[a..].find(&close)? + a;
    block.get(a + open.len()..b)
}

/// `block` without the nested elements `names` (so their fields don't shadow the parent's).
fn without(block: &str, names: &[&str]) -> String {
    let mut s = block.to_string();
    for n in names {
        while let Some(inner) = element(&s, n) {
            let open = format!("<stCamera:{n}");
            let Some(a) = s.find(&open) else { break };
            let end = a + open.len() + inner.len() + format!("</stCamera:{n}>").len();
            s.replace_range(a..end.min(s.len()), "");
        }
    }
    s
}

/// Parse an `.lcp` (XMP) into its entries; entries without a usable model are skipped.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for li in text.split("<rdf:li").skip(1).take(10_000) {
        let li = li.split("</rdf:li>").next().unwrap_or(li);
        let mut e = Entry {
            make: field(li, "Make").unwrap_or_default(),
            model: field(li, "Model").unwrap_or_default(),
            lens: field(li, "Lens").unwrap_or_default(),
            raw: field(li, "CameraRawProfile").is_some_and(|v| v.eq_ignore_ascii_case("true")),
            focal: num(li, "FocalLength").unwrap_or(0.0),
            distance: num(li, "FocusDistance").unwrap_or(0.0),
            aperture: num(li, "ApertureValue").unwrap_or(0.0),
            fisheye: li.contains("FisheyeModel"),
            ..Entry::default()
        };
        if let Some(p) = element(li, "PerspectiveModel") {
            let own = without(p, &["VignetteModel", "ChromaticRedGreenModel", "ChromaticGreenModel", "ChromaticBlueGreenModel"]);
            let base = |k: &str, d: f64| num(&own, k).unwrap_or(d);
            let fx = num(&own, "FocalLengthX").filter(|f| *f > 0.0);
            let (cx, cy) = (base("ImageXCenter", f64::NAN), base("ImageYCenter", f64::NAN));
            if let Some(fx) = fx {
                let fy = base("FocalLengthY", fx);
                e.geom = Some([fx, fy, cx, cy, base("RadialDistortParam1", 0.0), base("RadialDistortParam2", 0.0), base("RadialDistortParam3", 0.0)]);
            }
            if let Some(v) = element(p, "VignetteModel")
                && let Some(vfx) = num(v, "FocalLengthX").or(fx).filter(|f| *f > 0.0)
            {
                let g = |k: &str, d: f64| num(v, k).unwrap_or(d);
                e.vignette = Some([
                    vfx,
                    g("FocalLengthY", vfx),
                    g("ImageXCenter", cx),
                    g("ImageYCenter", cy),
                    g("VignetteModelParam1", 0.0),
                    g("VignetteModelParam2", 0.0),
                    g("VignetteModelParam3", 0.0),
                ]);
            }
        }
        if e.focal > 0.0 && (e.geom.is_some() || e.vignette.is_some()) && !e.fisheye {
            out.push(e);
        }
    }
    out
}

fn parsed(path: &Path) -> Option<Arc<Vec<Entry>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<Arc<Vec<Entry>>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(path) {
        return hit.clone();
    }
    let v = std::fs::metadata(path)
        .ok()
        .filter(|m| m.len() <= MAX_LCP)
        .and_then(|_| std::fs::read(path).ok())
        .map(|b| Arc::new(parse(&String::from_utf8_lossy(&b))))
        .filter(|v| !v.is_empty());
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(path.to_path_buf(), v.clone());
    v
}

/// The installed profile for a lens (the file's Exif `LensModel`) on a camera: entries for that
/// lens name, preferring the same camera, then the same make, raw profiles over JPEG ones.
pub fn find(make: Option<&str>, model: Option<&str>, lens: &str) -> Option<(PathBuf, Arc<Vec<Entry>>)> {
    let want = norm(lens);
    if want.len() < 4 {
        return None;
    }
    let (make_n, model_n) = (make.map(norm).unwrap_or_default(), model.map(norm).unwrap_or_default());
    let mut best: Option<(i32, PathBuf, Arc<Vec<Entry>>)> = None;
    for (key, path) in index() {
        if !(key.ends_with(&want) || key.contains(&want)) {
            continue;
        }
        let Some(entries) = parsed(path) else { continue };
        let Some(e) = entries.iter().find(|e| norm(&e.lens) == want).or_else(|| entries.first()) else { continue };
        if norm(&e.lens) != want && !key.ends_with(&want) {
            continue;
        }
        let mut score = 0;
        if !model_n.is_empty() && norm(&e.model) == model_n {
            score += 4;
        }
        if !make_n.is_empty() && make_n.starts_with(&norm(&e.make).chars().take(4).collect::<String>()) {
            score += 2;
        }
        if e.raw {
            score += 1;
        }
        if best.as_ref().is_none_or(|b| score > b.0) {
            best = Some((score, path.clone(), entries));
        }
    }
    best.map(|(_, p, e)| (p, e))
}

/// The model parameters interpolated for a photo: focal length, f-number, focus distance (m,
/// unknown = 2 m), from the entries carrying `pick`'s model.
fn interpolate(
    entries: &[Entry],
    focal: f64,
    f_number: Option<f64>,
    distance: Option<f64>,
    pick: impl Fn(&Entry) -> Option<[f64; 7]>,
) -> Option<[f64; 7]> {
    let with: Vec<(&Entry, [f64; 7])> = entries.iter().filter_map(|e| pick(e).map(|m| (e, m))).collect();
    if with.is_empty() {
        return None;
    }
    // the nearest focal lengths below and above (log scale)
    let mut focals: Vec<f64> = with.iter().map(|(e, _)| e.focal).collect();
    focals.sort_by(f64::total_cmp);
    focals.dedup();
    let f = if focal.is_finite() && focal > 0.0 { focal } else { focals[focals.len() / 2] };
    let (f0, f1) = bracket(&focals, f);
    let av = f_number.filter(|n| n.is_finite() && *n > 0.0).map(|n| 2.0 * n.log2());
    let inv_d = 1.0 / distance.filter(|d| d.is_finite() && *d > 0.0).unwrap_or(2.0);
    let at_focal = |fl: f64| -> Option<[f64; 7]> {
        let rows: Vec<&(&Entry, [f64; 7])> = with.iter().filter(|(e, _)| e.focal == fl).collect();
        // per focus distance: interpolate in aperture; then across distances in 1/d
        let mut dists: Vec<f64> = rows.iter().map(|(e, _)| e.distance).collect();
        dists.sort_by(f64::total_cmp);
        dists.dedup();
        let at_dist = |d: f64| -> Option<[f64; 7]> {
            let mut r: Vec<(f64, [f64; 7])> = rows.iter().filter(|(e, _)| e.distance == d).map(|(e, m)| (e.aperture, *m)).collect();
            r.sort_by(|a, b| a.0.total_cmp(&b.0));
            let avs: Vec<f64> = r.iter().map(|x| x.0).collect();
            let a = av.unwrap_or(avs[avs.len() - 1]);
            let (a0, a1) = bracket(&avs, a);
            let m0 = r.iter().find(|x| x.0 == a0)?.1;
            let m1 = r.iter().find(|x| x.0 == a1)?.1;
            Some(lerp(m0, m1, if a1 > a0 { ((a - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 0.0 }))
        };
        let invs: Vec<f64> = dists.iter().map(|d| 1.0 / d.max(0.01)).collect();
        let mut pairs: Vec<(f64, f64)> = invs.iter().copied().zip(dists.iter().copied()).collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let sorted: Vec<f64> = pairs.iter().map(|p| p.0).collect();
        let (i0, i1) = bracket(&sorted, inv_d);
        let d0 = pairs.iter().find(|p| p.0 == i0)?.1;
        let d1 = pairs.iter().find(|p| p.0 == i1)?.1;
        let (m0, m1) = (at_dist(d0)?, at_dist(d1)?);
        Some(lerp(m0, m1, if i1 > i0 { ((inv_d - i0) / (i1 - i0)).clamp(0.0, 1.0) } else { 0.0 }))
    };
    let (m0, m1) = (at_focal(f0)?, at_focal(f1)?);
    let t = if f1 > f0 { ((f.ln() - f0.ln()) / (f1.ln() - f0.ln())).clamp(0.0, 1.0) } else { 0.0 };
    let m = lerp(m0, m1, t);
    m.iter().all(|v| v.is_finite() || v.is_nan()).then_some(m)
}

fn lerp(a: [f64; 7], b: [f64; 7], t: f64) -> [f64; 7] {
    std::array::from_fn(|i| if a[i].is_nan() || b[i].is_nan() { a[i] } else { a[i] + (b[i] - a[i]) * t })
}

/// The sorted values on either side of `x` (equal at the ends).
fn bracket(sorted: &[f64], x: f64) -> (f64, f64) {
    let lo = sorted.iter().rev().find(|v| **v <= x).copied().unwrap_or(sorted[0]);
    let hi = sorted.iter().find(|v| **v >= x).copied().unwrap_or(sorted[sorted.len() - 1]);
    (lo, hi)
}

/// The profile's corrections for a `w × h` image (the raw's default crop, in its stored
/// orientation): `None` without usable entries.
pub fn corrections(entries: &[Entry], w: f64, h: f64, focal: f64, f_number: Option<f64>, distance: Option<f64>) -> Option<EmbeddedLens> {
    if !(w >= 2.0 && h >= 2.0) {
        return None;
    }
    let dmax = w.max(h);
    let centre = |cx: f64, cy: f64| -> (f64, f64) {
        // the principal point is in fractions of the width and height (focal lengths: of the long side)
        let (cx, cy) = (if cx.is_finite() { cx * w } else { w / 2.0 }, if cy.is_finite() { cy * h } else { h / 2.0 });
        (cx.clamp(0.0, w), cy.clamp(0.0, h))
    };
    let far = |cx: f64, cy: f64| [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].iter().map(|&(x, y)| (x - cx).hypot(y - cy)).fold(0.0, f64::max).max(1.0);
    let geom = interpolate(entries, focal, f_number, distance, |e| e.geom);
    let warp = geom.and_then(|g| {
        let (cx, cy) = centre(g[2], g[3]);
        let r = far(cx, cy);
        let s = r / (g[0] * dmax).max(1e-9);
        let plane = [1.0, g[4] * s.powi(2), g[5] * s.powi(4), g[6] * s.powi(6), 0.0, 0.0];
        plane.iter().all(|v| v.is_finite() && v.abs() < 10.0).then(|| EmbeddedWarp {
            planes: [plane; 3],
            center: Point::new(cx / w, cy / h),
            radius: r / dmax,
        })
    });
    let vig = interpolate(entries, focal, f_number, distance, |e| e.vignette).and_then(|v| {
        let (cx, cy) = centre(v[2], v[3]);
        let r = far(cx, cy);
        let s = r / (v[0] * dmax).max(1e-9);
        // the gain 1 / V as `1 + k0 t + … + k4 t⁵` in t = (dist / r)² (least squares)
        let mut a = [[0.0f64; 5]; 5];
        let mut b = [0.0f64; 5];
        for i in 0..=48 {
            let t = i as f64 / 48.0;
            let rho2 = t * s * s;
            let vv = 1.0 + v[4] * rho2 + v[5] * rho2 * rho2 + v[6] * rho2 * rho2 * rho2;
            if vv <= 0.05 {
                continue;
            }
            let g = 1.0 / vv - 1.0;
            let f: [f64; 5] = std::array::from_fn(|j| t.powi(j as i32 + 1));
            for p in 0..5 {
                b[p] += f[p] * g;
                for q in 0..5 {
                    a[p][q] += f[p] * f[q];
                }
            }
        }
        for (p, row) in a.iter_mut().enumerate() {
            row[p] += 1e-10;
        }
        let k = crate::adobe::solve5(a, b)?;
        k.iter().all(|x| x.abs() < 100.0).then(|| EmbeddedVignette { k, center: Point::new(cx / w, cy / h), radius: r / dmax })
    });
    (warp.is_some() || vig.is_some()).then_some(EmbeddedLens { warp, vignette: vig })
}

/// The lens profile corrections for a raw (its make, model, lens, focal length and aperture;
/// `w × h` = its default crop): `None` when no installed profile matches.
pub fn for_raw(meta: &lightcraft_meta::Metadata, w: f64, h: f64) -> Option<EmbeddedLens> {
    let lens = meta.lens_model.as_deref()?.trim();
    let (_, entries) = find(meta.make.as_deref(), meta.model.as_deref(), lens)?;
    corrections(&entries, w, h, meta.focal_length.unwrap_or(0.0), meta.f_number, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LCP: &str = r#"<x:xmpmeta><rdf:RDF><rdf:Description><photoshop:CameraProfiles><rdf:Seq>
     <rdf:li rdf:parseType="Resource">
      <stCamera:Make>Canon</stCamera:Make><stCamera:Model>Canon EOS 5D Mark II</stCamera:Model>
      <stCamera:Lens>EF24mm f/1.4L II USM</stCamera:Lens><stCamera:CameraRawProfile>True</stCamera:CameraRawProfile>
      <stCamera:FocalLength>24</stCamera:FocalLength><stCamera:FocusDistance>10000</stCamera:FocusDistance><stCamera:ApertureValue>2</stCamera:ApertureValue>
      <stCamera:PerspectiveModel rdf:parseType="Resource"><stCamera:FocalLengthX>0.708</stCamera:FocalLengthX>
       <stCamera:ImageXCenter>0.5</stCamera:ImageXCenter><stCamera:ImageYCenter>0.333</stCamera:ImageYCenter>
       <stCamera:RadialDistortParam1>-0.1</stCamera:RadialDistortParam1>
       <stCamera:VignetteModel rdf:parseType="Resource"><stCamera:FocalLengthX>0.708</stCamera:FocalLengthX>
        <stCamera:VignetteModelParam1>-0.6</stCamera:VignetteModelParam1><stCamera:VignetteModelParam2>0.2</stCamera:VignetteModelParam2></stCamera:VignetteModel>
      </stCamera:PerspectiveModel></rdf:li>
     <rdf:li rdf:parseType="Resource" stCamera:FocalLength="24" stCamera:FocusDistance="10000" stCamera:ApertureValue="4">
      <stCamera:PerspectiveModel rdf:parseType="Resource" stCamera:FocalLengthX="0.708" stCamera:RadialDistortParam1="-0.1">
       <stCamera:VignetteModel rdf:parseType="Resource" stCamera:VignetteModelParam1="-0.2"/>
      </stCamera:PerspectiveModel></rdf:li>
    </rdf:Seq></photoshop:CameraProfiles></rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn parses_entries_and_interpolates_by_aperture() {
        let e = parse(LCP);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].lens, "EF24mm f/1.4L II USM");
        assert!(e[0].raw && e[0].geom.is_some() && e[0].vignette.is_some());
        assert_eq!(e[1].vignette.unwrap()[4], -0.2);
        // f/2.8 (APEX 3) sits halfway between the two entries
        let v = interpolate(&e, 24.0, Some(2f64.powf(1.5)), None, |x| x.vignette).unwrap();
        assert!((v[4] + 0.4).abs() < 1e-9, "{v:?}");
        let lens = corrections(&e, 6000.0, 4000.0, 24.0, Some(1.4), None).unwrap();
        let vig = lens.vignette.unwrap();
        // the corner (r = 1) is brightened: gain > 1
        assert!(vig.k.iter().sum::<f64>() > 0.2, "{vig:?}");
        let warp = lens.warp.unwrap();
        assert!(warp.planes[0][1] < 0.0 && (warp.planes[0][0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(parse("").is_empty());
        assert!(parse("<rdf:li><stCamera:FocalLength>abc</stCamera:FocalLength>").is_empty());
        assert!(corrections(&[], 100.0, 100.0, 24.0, None, None).is_none());
        let e = parse(LCP);
        assert!(corrections(&e, 0.0, 0.0, 24.0, None, None).is_none());
        assert!(corrections(&e, 1000.0, 800.0, f64::NAN, Some(f64::NAN), Some(-1.0)).is_some());
    }
}
