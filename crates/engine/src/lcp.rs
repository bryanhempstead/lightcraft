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

use lightcraft_develop::{EmbeddedLens, EmbeddedVignette, EmbeddedWarp, LensProfileRef};
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
    I.get_or_init(|| scan(&dirs()))
}

/// The `.lcp` files under `roots` (four folders deep at most), by their normalised lens part.
fn scan(roots: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for d in roots.iter().cloned() {
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
    /// `ProfileName`, e.g. "Adobe (Canon EF 24mm f/1.4L II USM)".
    pub profile_name: String,
    /// A fisheye model (`FisheyeModel`): `geom` is then fx, fy, cx, cy, k1, k2, 0 of
    /// `r_d = θ(1 + k1 θ² + k2 θ⁴)`, θ the angle off the axis.
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
            profile_name: field(li, "ProfileName").unwrap_or_default(),
            fisheye: li.contains("FisheyeModel"),
            ..Entry::default()
        };
        // without FocalLengthX: the focal length over the sensor's long side (36 mm full frame,
        // divided by the crop factor)
        let format = num(li, "SensorFormatFactor").filter(|f| *f > 0.05 && *f < 20.0).unwrap_or(1.0);
        let default_fx = (e.focal > 0.0).then(|| e.focal * format / 36.0);
        let model = if e.fisheye { element(li, "FisheyeModel") } else { element(li, "PerspectiveModel") };
        if let Some(p) = model {
            let own = without(p, &["VignetteModel", "ChromaticRedGreenModel", "ChromaticGreenModel", "ChromaticBlueGreenModel"]);
            let base = |k: &str, d: f64| num(&own, k).unwrap_or(d);
            let fx = num(&own, "FocalLengthX").filter(|f| *f > 0.0).or(default_fx);
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
        // a manual lens (no FocalLength: one focal length) is a prime: its model's focal length
        if e.focal.is_nan() || e.focal <= 0.0 {
            e.focal = e.geom.or(e.vignette).map_or(0.0, |m| m[0] * 36.0 / format);
        }
        if e.focal.is_finite() && e.focal > 0.0 && (e.geom.is_some() || e.vignette.is_some()) {
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

/// How many installed lens profiles are filed under a camera make (Camera Raw keeps one folder per
/// maker: `LensProfiles/1.0/Canon/…`), and the folder's name.
pub fn count_for_make(make: &str) -> usize {
    let first = norm(make.split(|c: char| c.is_whitespace() || c == ',').next().unwrap_or(make));
    if first.len() < 2 {
        return 0;
    }
    index()
        .iter()
        .filter(|(_, p)| {
            let folder = p.parent().and_then(Path::file_name).map(|n| norm(&n.to_string_lossy())).unwrap_or_default();
            let file = p.file_name().map(|n| norm(&n.to_string_lossy())).unwrap_or_default();
            (!folder.is_empty() && (folder.starts_with(&first) || first.starts_with(&folder))) || file.starts_with(&first)
        })
        .count()
}

/// A lens's installed profile as the Optics panel names it ("Adobe (Canon RF 24-105mm …)"),
/// and its file, for a camera.
pub fn profile_name(make: Option<&str>, model: Option<&str>, lens: &str) -> Option<(String, PathBuf)> {
    let (path, entries) = find(make, model, lens)?;
    let name = entries.iter().map(|e| e.profile_name.trim()).find(|n| !n.is_empty()).map(str::to_string);
    let name = name.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    Some((name, path))
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
    let fisheye = entries.iter().any(|e| e.fisheye && e.geom.is_some());
    let geom = interpolate(entries, focal, f_number, distance, |e| e.geom.filter(|_| e.fisheye == fisheye));
    let warp = if fisheye {
        geom.and_then(|g| {
            let (cx, cy) = centre(g[2], g[3]);
            defish(g, w, h, cx, cy, far(cx, cy))
        })
    } else {
        geom.and_then(|g| {
            let (cx, cy) = centre(g[2], g[3]);
            let r = far(cx, cy);
            let s = r / (g[0] * dmax).max(1e-9);
            let plane = [1.0, g[4] * s.powi(2), g[5] * s.powi(4), g[6] * s.powi(6), 0.0, 0.0];
            plane.iter().all(|v| v.is_finite() && v.abs() < 10.0).then(|| EmbeddedWarp {
                planes: [plane; 3],
                center: Point::new(cx / w, cy / h),
                radius: r / dmax,
                fisheye: false,
            })
        })
    };
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

/// A fisheye profile's correction for a `w × h` image (optical centre `cx, cy` px, `far` = its
/// distance to the farthest corner): the rectilinear view of the scene, scaled as Lightroom does —
/// as little as possible while every pixel of the frame still comes from the photo (measured on
/// Lightroom's renders of the Sigma 15 mm diagonal fisheye: the frame's short-side edges touch
/// the photo's, ±0.1 %).
fn defish(g: [f64; 7], w: f64, h: f64, cx: f64, cy: f64, far: f64) -> Option<EmbeddedWarp> {
    let dmax = w.max(h);
    let f = g[0] * dmax;
    let (k1, k2) = (g[4], g[5]);
    if !(f.is_finite() && f > 0.0 && k1.is_finite() && k2.is_finite()) {
        return None;
    }
    let rho = |th: f64| th * (1.0 + th * th * (k1 + k2 * th * th));
    // the source position of frame position (x, y) at output scale `s`
    let source = |x: f64, y: f64, s: f64| {
        let (dx, dy) = (x - cx, y - cy);
        let r = dx.hypot(dy);
        if r < 1e-9 {
            return (cx, cy);
        }
        let k = f * rho((r / (f * s)).atan()) / r;
        (cx + dx * k, cy + dy * k)
    };
    const N: usize = 64;
    let tol = 1e-6 * dmax;
    let covered = |s: f64| {
        (0..=N).all(|i| {
            let t = i as f64 / N as f64;
            [(t * w, 0.0), (t * w, h), (0.0, t * h), (w, t * h)].iter().all(|&(x, y)| {
                let (sx, sy) = source(x, y, s);
                sx >= -tol && sy >= -tol && sx <= w + tol && sy <= h + tol
            })
        })
    };
    let (mut lo, mut hi) = (0.02, 50.0);
    if !covered(hi) {
        return None;
    }
    for _ in 0..60 {
        let mid = (lo * hi).sqrt();
        if covered(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let plane = [far / (f * hi), k1, k2, f / far, 0.0, 0.0];
    plane.iter().all(|v| v.is_finite() && v.abs() < 1e3).then(|| EmbeddedWarp {
        planes: [plane; 3],
        center: Point::new(cx / w, cy / h),
        radius: far / dmax,
        fisheye: true,
    })
}

/// The installed profile a develop setting names (Lightroom's `LensProfileFilename`,
/// `LensProfileName`, `LensProfileDigest`).
pub fn find_ref(r: &LensProfileRef) -> Option<(PathBuf, Arc<Vec<Entry>>)> {
    find_ref_in(index(), r)
}

/// [`find_ref`] among `index`: the file of that name (the one with that digest when several are
/// installed: Camera Raw's folder and Lightroom's own hold copies), else a file whose entries carry
/// that profile name (that digest first).
fn find_ref_in(index: &[(String, PathBuf)], r: &LensProfileRef) -> Option<(PathBuf, Arc<Vec<Entry>>)> {
    let digest = r.digest.trim().to_ascii_lowercase();
    let digest_ok = |p: &Path| !digest.is_empty() && file_md5(p).is_some_and(|d| d == digest);
    let first_ok = |cands: Vec<(PathBuf, Arc<Vec<Entry>>)>| {
        let i = cands.iter().position(|(p, _)| digest_ok(p)).unwrap_or(0);
        cands.into_iter().nth(i)
    };
    let file = r.filename.trim();
    if !file.is_empty() {
        let same: Vec<(PathBuf, Arc<Vec<Entry>>)> = index
            .iter()
            .filter(|(_, p)| p.file_name().is_some_and(|n| n.to_string_lossy().trim().eq_ignore_ascii_case(file)))
            .filter_map(|(_, p)| Some((p.clone(), parsed(p)?)))
            .collect();
        if !same.is_empty() {
            return first_ok(same);
        }
    }
    let name = r.name.trim();
    let want = norm(name);
    if want.len() < 4 {
        return None;
    }
    let named: Vec<(PathBuf, Arc<Vec<Entry>>)> = index
        .iter()
        .filter(|(key, _)| key.len() >= 4 && want.contains(key.as_str()))
        .filter_map(|(_, p)| Some((p.clone(), parsed(p)?)))
        .filter(|(_, e)| e.iter().any(|e| e.profile_name.trim().eq_ignore_ascii_case(name)))
        .collect();
    if named.is_empty() { None } else { first_ok(named) }
}

/// The lens corrections of the profile `r` names for a `w × h` photo (oriented, default crop) at
/// `focal` mm and `f_number`: `None` when it isn't installed (or has no usable model).
pub fn for_ref(r: &LensProfileRef, w: f64, h: f64, focal: Option<f64>, f_number: Option<f64>) -> Option<EmbeddedLens> {
    let (_, entries) = find_ref(r)?;
    corrections(&entries, w, h, focal.unwrap_or(0.0), f_number, None)
}

/// The MD5 of a file, lower-case hex (Lightroom's `LensProfileDigest`), cached.
fn file_md5(path: &Path) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(PoisonError::into_inner).get(path) {
        return hit.clone();
    }
    let v = std::fs::metadata(path).ok().filter(|m| m.len() <= MAX_LCP).and_then(|_| std::fs::read(path).ok()).map(|b| md5_hex(&b));
    cache.lock().unwrap_or_else(PoisonError::into_inner).insert(path.to_path_buf(), v.clone());
    v
}

/// MD5 (RFC 1321) of `data` as lower-case hex: only to recognise the profile file a digest names.
fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] = std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut st: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes(chunk.get(i * 4..i * 4 + 4).and_then(|b| b.try_into().ok()).unwrap_or([0; 4])));
        let [mut a, mut b, mut c, mut d] = st;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let t = d;
            d = c;
            c = b;
            b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(S[i]));
            a = t;
        }
        st = [st[0].wrapping_add(a), st[1].wrapping_add(b), st[2].wrapping_add(c), st[3].wrapping_add(d)];
    }
    st.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02x}")).collect()
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

    /// A diagonal fisheye's entry the way Adobe writes them: no FocalLengthX (the focal length
    /// over the 36 mm frame), no principal point, nested chromatic models.
    const FISHEYE: &str = r#"<x:xmpmeta><rdf:RDF><rdf:Description><photoshop:CameraProfiles><rdf:Seq>
     <rdf:li><rdf:Description stCamera:Make="Canon" stCamera:CameraRawProfile="True" stCamera:Lens="15mm"
       stCamera:ProfileName="Adobe (Test 15mm FISHEYE, Canon)" stCamera:SensorFormatFactor="1"
       stCamera:FocalLength="15" stCamera:FocusDistance="1" stCamera:ApertureValue="2.97">
      <stCamera:FisheyeModel><rdf:Description stCamera:RadialDistortParam1="-0.015918" stCamera:RadialDistortParam2="-0.004867">
       <stCamera:ChromaticRedGreenModel stCamera:ScaleFactor="1.0004" stCamera:RadialDistortParam1="0.5"/>
       <stCamera:VignetteModel stCamera:VignetteModelParam1="-0.015" stCamera:VignetteModelParam2="-0.25" stCamera:VignetteModelParam3="0.05"/>
      </rdf:Description></stCamera:FisheyeModel></rdf:Description></rdf:li>
    </rdf:Seq></photoshop:CameraProfiles></rdf:Description></rdf:RDF></x:xmpmeta>"#;

    #[test]
    fn fisheye_profiles_defish_and_fill_the_frame() {
        let e = parse(FISHEYE);
        assert_eq!(e.len(), 1);
        assert!(e[0].fisheye);
        assert_eq!(e[0].profile_name, "Adobe (Test 15mm FISHEYE, Canon)");
        let g = e[0].geom.unwrap();
        assert!((g[0] - 15.0 / 36.0).abs() < 1e-12, "{g:?}");
        assert_eq!((g[4], g[5]), (-0.015918, -0.004867), "the fisheye's own terms, not a chromatic model's");
        let (w, h) = (6000.0, 4000.0);
        let lens = corrections(&e, w, h, 15.0, Some(2.8), None).unwrap();
        let warp = lens.warp.unwrap();
        assert!(warp.fisheye && lens.vignette.is_some());
        let at = |x: f64, y: f64| lightcraft_pipeline::optics::embedded_warp(&warp, Point::new(x, y), 1, w, h);
        // every frame edge comes from the photo, and the short side's edges are where it stops
        // (Lightroom's scale: measured 0.764 on its renders of the Sigma 15 mm with this model)
        for i in 0..=40 {
            let t = i as f64 / 40.0;
            for (x, y) in [(t * w, 0.0), (t * w, h), (0.0, t * h), (w, t * h)] {
                let s = at(x, y);
                assert!(s.x >= -0.01 && s.y >= -0.01 && s.x <= w + 0.01 && s.y <= h + 0.01, "({x}, {y}) → {s:?}");
            }
        }
        assert!(at(w / 2.0, 0.0).y.abs() < 0.5, "{:?}", at(w / 2.0, 0.0));
        let fs = (15.0 / 36.0) * w;
        let scale = 1.0 / (warp.planes[0][0] * fs / (warp.radius * w));
        assert!((scale - 0.7613).abs() < 0.002, "{scale}");
        // the centre is shown 1 / scale larger than the fisheye shows it
        let c = at(w / 2.0 + 10.0, h / 2.0);
        assert!(((c.x - w / 2.0) - 10.0 / scale).abs() < 0.01, "{c:?}");
        // portrait frames fill too
        let p = corrections(&e, h, w, 15.0, None, None).unwrap().warp.unwrap();
        let s = lightcraft_pipeline::optics::embedded_warp(&p, Point::new(0.0, w / 2.0), 1, h, w);
        assert!(s.x.abs() < 0.5, "{s:?}");
    }

    /// A manual lens's profile (Voigtländer on a Leica M): no FocalLength in its entries.
    #[test]
    fn manual_lens_profiles_without_a_focal_length_parse() {
        let manual = LCP.replace("<stCamera:FocalLength>24</stCamera:FocalLength>", "").replace(" stCamera:FocalLength=\"24\"", "");
        let e = parse(&manual);
        assert_eq!(e.len(), 2, "{manual}");
        assert!((e[0].focal - 0.708 * 36.0).abs() < 1e-9);
        let lens = corrections(&e, 6000.0, 4000.0, 0.0, None, None).unwrap();
        assert!(lens.warp.is_some() && lens.vignette.is_some());
    }

    #[test]
    fn md5_matches_the_rfc_vectors() {
        assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(md5_hex(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"), "57edf4a22be3c955ac49da2e2107b67a");
    }

    #[test]
    fn profiles_are_found_by_file_name_digest_and_name() {
        let dir = std::env::temp_dir().join(format!("lc-lcp-ref-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b) = (dir.join("CameraRaw/LensProfiles/1.0/Sigma"), dir.join("Lightroom/LensProfiles/1.0/Sigma"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let file = "Canon (Test_15mm_FISHEYE ) - RAW.lcp";
        // two copies of the same file name: an older one, and the one Lightroom's digest names
        let old = FISHEYE.replace("-0.015918", "-0.02");
        std::fs::write(a.join(file), &old).unwrap();
        std::fs::write(b.join(file), FISHEYE).unwrap();
        std::fs::write(a.join("Canon (Other lens) - RAW.lcp"), LCP).unwrap();
        let idx = scan(std::slice::from_ref(&dir));
        assert_eq!(idx.len(), 3);
        let digest = md5_hex(FISHEYE.as_bytes()).to_ascii_uppercase();
        let by_file = LensProfileRef { filename: file.into(), digest: digest.clone(), ..Default::default() };
        let (p, e) = find_ref_in(&idx, &by_file).unwrap();
        assert_eq!(p, b.join(file), "the copy with Lightroom's digest");
        assert_eq!(e[0].geom.unwrap()[4], -0.015918);
        // without a digest: a file of that name
        assert!(find_ref_in(&idx, &LensProfileRef { filename: file.to_lowercase(), ..Default::default() }).is_some());
        // by profile name only (the file renamed since), digest first
        let by_name = LensProfileRef { name: "Adobe (Test 15mm FISHEYE, Canon)".into(), filename: "gone.lcp".into(), digest, custom: true };
        assert_eq!(find_ref_in(&idx, &by_name).unwrap().0, b.join(file));
        // unknown: nothing (the file's own lens match stays)
        assert!(find_ref_in(&idx, &LensProfileRef { name: "Adobe (Nope)".into(), ..Default::default() }).is_none());
        assert!(find_ref_in(&idx, &LensProfileRef::default()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(parse("").is_empty());
        assert!(parse("<rdf:li><stCamera:FocalLength>abc</stCamera:FocalLength>").is_empty());
        assert!(corrections(&[], 100.0, 100.0, 24.0, None, None).is_none());
        let e = parse(LCP);
        assert!(corrections(&e, 0.0, 0.0, 24.0, None, None).is_none());
        assert!(corrections(&e, 1000.0, 800.0, f64::NAN, Some(f64::NAN), Some(-1.0)).is_some());
        // a fisheye model that can't fill any frame, or with nonsense terms: no warp, no panic
        for (k1, k2) in [("-5", "0"), ("NaN", "1e308"), ("1e300", "-1e300")] {
            let bad = FISHEYE.replace("-0.015918", k1).replace("-0.004867", k2);
            let e = parse(&bad);
            let _ = corrections(&e, 600.0, 400.0, 15.0, None, None);
        }
        assert!(defish([0.4, 0.4, f64::NAN, f64::NAN, 0.0, 0.0, 0.0], 600.0, 400.0, 300.0, 200.0, 360.0).is_some());
        assert!(defish([0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 0.0], 600.0, 400.0, 300.0, 200.0, 360.0).is_none());
    }
}
