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
    #[cfg(test)]
    if let Some(v) = TEST_ROOTS.with(|r| r.borrow().clone()) {
        return v;
    }
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

#[cfg(test)]
thread_local! {
    /// A test's own profile folders (each test runs on its own thread).
    static TEST_ROOTS: std::cell::RefCell<Option<Vec<PathBuf>>> = const { std::cell::RefCell::new(None) };
}

/// Look for profiles in `roots` instead of Adobe's folders, on this thread (tests).
#[cfg(test)]
pub(crate) fn use_test_roots(roots: Option<Vec<PathBuf>>) {
    TEST_ROOTS.with(|r| *r.borrow_mut() = roots);
}

type Scanned = (Arc<Vec<(String, PathBuf)>>, Arc<Vec<Installed>>);

/// The scan of [`dirs`] (once per set of folders).
fn scanned() -> Scanned {
    static CACHE: OnceLock<Mutex<Vec<(Vec<PathBuf>, Scanned)>>> = OnceLock::new();
    let roots = dirs();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, hit)) = cache.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|(r, _)| *r == roots) {
        return hit.clone();
    }
    let v: Scanned = (Arc::new(scan(&roots)), Arc::new(listing(&roots)));
    let mut c = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if c.len() > 16 {
        c.clear();
    }
    c.push((roots, v.clone()));
    v
}

/// Every `.lcp` by its lens part (the file name's last parenthesis), normalised.
fn index() -> Arc<Vec<(String, PathBuf)>> {
    scanned().0
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

/// The profile entries (the top-level `rdf:li`s) of an `.lcp`, each without its opening tag's
/// `<rdf:li`. An entry holds lists of its own (`AlternateLensNames`, `AlternateLensIDs`,
/// `VignetteModelPiecewiseParam`): their `rdf:li`s are part of it, not ends of it — Leica's
/// profiles list the lens's other names before its model, which a plain split cut off.
fn profile_items(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut at) = (0usize, 0usize, 0usize);
    while at < text.len() {
        let rest = text.get(at..).unwrap_or("");
        let open = rest.find("<rdf:li");
        let close = rest.find("</rdf:li>");
        match (open, close) {
            (Some(o), c) if c.is_none_or(|c| o < c) => {
                if depth == 0 {
                    start = at + o + "<rdf:li".len();
                }
                depth += 1;
                at += o + "<rdf:li".len();
            }
            (_, Some(c)) => {
                if depth == 1
                    && let Some(item) = text.get(start..at + c)
                {
                    out.push(item);
                }
                depth = depth.saturating_sub(1);
                at += c + "</rdf:li>".len();
            }
            _ => break,
        }
    }
    // an entry never closed (a cut-off file): what there is of it
    if depth > 0
        && let Some(item) = text.get(start..)
    {
        out.push(item);
    }
    out
}

/// Parse an `.lcp` (XMP) into its entries; entries without a usable model are skipped.
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    for li in profile_items(text).into_iter().take(10_000) {
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
    for (key, path) in index().iter() {
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

/// One installed lens profile file, as the Lens Corrections panel's Make / Model / Profile
/// pickers list it (Lightroom's Profile tab when the lens isn't recognised).
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    /// The lens maker: Camera Raw's folder (`LensProfiles/1.0/<Make>/…`).
    pub make: String,
    /// The lens: the file name's parenthesis ("Leica SUMMICRON-M 50 mm f2").
    pub model: String,
    pub path: PathBuf,
}

/// The `.lcp` files under `roots` by lens maker and lens; one per file name and maker (Camera
/// Raw's folder and Lightroom's own hold copies: the first root wins), sorted.
fn listing(roots: &[PathBuf]) -> Vec<Installed> {
    let mut out: Vec<Installed> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for root in roots {
        for (_, path) in scan(std::slice::from_ref(root)) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let rel = path.strip_prefix(root).ok().map(|r| r.components().count()).unwrap_or(0);
            let folder = path.strip_prefix(root).ok().and_then(|r| r.components().next()).map(|c| c.as_os_str().to_string_lossy().into_owned());
            let make = match folder {
                Some(f) if rel > 1 => f,
                _ => stem.split_whitespace().next().unwrap_or("").to_string(),
            };
            let model = match (name.find('('), name.rfind(')')) {
                (Some(a), Some(b)) if b > a + 1 => name.get(a + 1..b).unwrap_or("").trim().to_string(),
                _ => stem.trim_end_matches(" - RAW").trim().to_string(),
            };
            if make.is_empty() || model.is_empty() || !seen.insert((make.to_lowercase(), name.to_lowercase())) {
                continue;
            }
            out.push(Installed { make, model, path });
        }
    }
    out.sort_by(|a, b| {
        (a.make.to_lowercase(), a.model.to_lowercase(), a.path.file_name()).cmp(&(b.make.to_lowercase(), b.model.to_lowercase(), b.path.file_name()))
    });
    out
}

/// Every installed lens profile file ([`Installed`]).
pub fn installed() -> Arc<Vec<Installed>> {
    scanned().1
}

/// The lens makers that have installed profiles.
pub fn makes() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for i in installed().iter() {
        if !v.iter().any(|m| m.eq_ignore_ascii_case(&i.make)) {
            v.push(i.make.clone());
        }
    }
    v
}

/// The lenses of a maker that have installed profiles.
pub fn models(make: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for i in installed().iter().filter(|i| i.make.eq_ignore_ascii_case(make)) {
        if !v.iter().any(|m| m.eq_ignore_ascii_case(&i.model)) {
            v.push(i.model.clone());
        }
    }
    v
}

/// The profiles of a lens: (the name the panel shows, its file).
pub fn profiles(make: &str, model: &str) -> Vec<(String, PathBuf)> {
    let files: Vec<PathBuf> =
        installed().iter().filter(|i| i.make.eq_ignore_ascii_case(make) && i.model.eq_ignore_ascii_case(model)).map(|i| i.path.clone()).collect();
    let mut out: Vec<(String, PathBuf)> = files.into_iter().filter_map(|p| Some((reference(&p)?.name, p))).collect();
    // two files with one profile name (a raw and a JPEG profile): tell them apart by file
    let names: Vec<String> = out.iter().map(|(n, _)| n.clone()).collect();
    for (n, p) in &mut out {
        if names.iter().filter(|x| *x == n).count() > 1 {
            let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            *n = format!("{n} · {stem}");
        }
    }
    out
}

/// The develop-settings reference to the profile in `path` (Lightroom's `LensProfileName`,
/// `LensProfileFilename`, `LensProfileDigest`): `None` when it isn't a readable profile.
pub fn reference(path: &Path) -> Option<LensProfileRef> {
    let entries = parsed(path)?;
    let name = entries.iter().map(|e| e.profile_name.trim()).find(|n| !n.is_empty()).map(str::to_string);
    let name = name.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    Some(LensProfileRef {
        name,
        filename: path.file_name()?.to_string_lossy().into_owned(),
        digest: file_md5(path).map(|d| d.to_ascii_uppercase()).unwrap_or_default(),
        custom: false,
    })
}

/// Where the profile `r` names is listed ([`installed`]), if it is installed.
pub fn listed(r: &LensProfileRef) -> Option<Installed> {
    let (path, _) = find_ref(r)?;
    let all = installed();
    let hit = all.iter().find(|i| i.path == path).or_else(|| all.iter().find(|i| i.path.file_name() == path.file_name()));
    hit.cloned()
}

/// A lens name taken apart: focal length, fastest aperture and its words ("Leica Summicron-M
/// 1:2/50" → 50 mm, f/2, `summicron`; "Leica SUMMICRON-M 50 mm f2" → the same).
#[derive(Debug, Default, PartialEq)]
struct LensName {
    focal: Option<f64>,
    aperture: Option<f64>,
    /// A zoom (a focal range): never matched by name.
    zoom: bool,
    words: Vec<String>,
}

/// Words that say nothing about which lens it is.
const PLAIN_WORDS: &[&str] = &["leica", "leitz", "camera", "voigtlander", "cosina", "raw", "lens", "adobe", "wetzlar"];

fn lens_name(s: &str) -> LensName {
    let s: String = s
        .chars()
        .map(|c| match c {
            'ä' | 'Ä' => 'a',
            'ö' | 'Ö' => 'o',
            'ü' | 'Ü' => 'u',
            'é' | 'è' => 'e',
            ',' => '.',
            c => c,
        })
        .collect();
    let lower = s.to_lowercase();
    let mut n = LensName::default();
    let num = |t: &str| t.trim_matches('.').parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
    // Leica's "1:2/50", "1:4.5/15"
    if let Some(rest) = lower.split("1:").nth(1)
        && let Some((a, f)) = rest.split_once('/')
    {
        n.aperture = num(a);
        n.focal = num(f.split(|c: char| !(c.is_ascii_digit() || c == '.')).next().unwrap_or(""));
    }
    let tokens: Vec<&str> = lower.split(|c: char| c.is_whitespace() || c == '(' || c == ')').filter(|t| !t.is_empty()).collect();
    for (i, t) in tokens.iter().enumerate() {
        // "50mm", "50 mm", "16-18-21 mm"
        let focal_part = if let Some(v) = t.strip_suffix("mm").filter(|v| !v.is_empty()) {
            Some(v)
        } else if tokens.get(i + 1) == Some(&"mm") {
            Some(*t)
        } else {
            None
        };
        if let Some(v) = focal_part {
            if v.contains('-') {
                n.zoom = true;
            } else if n.focal.is_none() {
                n.focal = num(v);
            }
        }
        // "f2", "f/2.8", "F4.5" (not a zoom's "f3.5-4")
        if n.aperture.is_none()
            && let Some(v) = t.strip_prefix('f').map(|v| v.trim_start_matches('/'))
            && !v.contains('-')
        {
            n.aperture = num(v);
        }
    }
    for token in lower.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ',') {
        for (i, w) in token.split(|c: char| !c.is_ascii_alphanumeric()).enumerate() {
            let w = if w == "aspherical" { "asph" } else { w };
            // a word, or the mount letter after a hyphen ("Summicron-M" vs "Summicron-R")
            let mount = i > 0 && w.len() == 1 && token.contains('-');
            if (w.len() >= 3 || mount)
                && w.chars().all(|c| c.is_ascii_alphabetic())
                && w != "mm"
                && !PLAIN_WORDS.contains(&w)
                && !n.words.iter().any(|x| x == w)
            {
                n.words.push(w.to_string());
            }
        }
    }
    n
}

/// The installed profile for a lens set by hand (right-click ▸ Set Lens: "Leica Summicron-M
/// 1:2/50") on a `camera` ("LEICA M (Typ 262)"): same focal length and fastest aperture and the
/// lens's own name ("Summicron"), fewest extra words ("APO", "ASPH."), that camera maker's folder
/// and raw profiles first. `None` without a confident match.
pub fn match_lens(lens: &str, camera: &str) -> Option<PathBuf> {
    match_lens_in(&installed(), lens, camera)
}

fn match_lens_in(list: &[Installed], lens: &str, camera: &str) -> Option<PathBuf> {
    let want = lens_name(lens);
    let focal = want.focal.filter(|_| !want.zoom)?;
    if want.words.is_empty() {
        return None;
    }
    let brand = norm(camera.split_whitespace().next().unwrap_or(""));
    let mut best: Option<(i64, &Installed)> = None;
    for i in list {
        let have = lens_name(&i.model);
        if have.zoom || have.focal.is_none_or(|f| (f - focal).abs() > 0.6) {
            continue;
        }
        if let (Some(a), Some(b)) = (want.aperture, have.aperture)
            && (a - b).abs() > 0.06
        {
            continue;
        }
        let shared = want.words.iter().filter(|w| have.words.contains(w)).count() as i64;
        if shared == 0 {
            continue;
        }
        let extra = have.words.iter().filter(|w| !want.words.contains(w)).count() as i64;
        let mut score = shared * 10 - extra;
        let folder_brand = brand.len() >= 2 && i.path.components().any(|c| norm(&c.as_os_str().to_string_lossy()) == brand);
        if folder_brand {
            score += 3;
        }
        if i.path.file_name().is_some_and(|n| n.to_string_lossy().contains("RAW")) {
            score += 1;
        }
        if best.is_none_or(|(b, _)| score > b) {
            best = Some((score, i));
        }
    }
    best.map(|(_, i)| i.path.clone())
}

/// The profile "Enable Profile Corrections" picks for a photo by itself when the file carries no
/// lens corrections of its own: the installed profile for the lens its metadata names (Set Lens on
/// a Leica M with an uncoded lens), on its camera.
pub fn auto_for(p: &lightcraft_catalog::Photo) -> Option<LensProfileRef> {
    if p.embedded_lens.is_some() || p.meta.lens.trim().is_empty() {
        return None;
    }
    let path = find(None, None, p.meta.lens.trim()).map(|(path, _)| path).or_else(|| match_lens(&p.meta.lens, &p.meta.camera))?;
    reference(&path)
}

/// The installed profile a develop setting names (Lightroom's `LensProfileFilename`,
/// `LensProfileName`, `LensProfileDigest`).
pub fn find_ref(r: &LensProfileRef) -> Option<(PathBuf, Arc<Vec<Entry>>)> {
    find_ref_in(&index(), r)
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

    /// Installed profiles laid out like Camera Raw's folder (one synthetic profile per file).
    fn fake_profiles(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("lc-lcp-{tag}-{}", std::process::id())).join("LensProfiles/1.0");
        let _ = std::fs::remove_dir_all(&root);
        for (folder, lens) in [
            ("Leica", "Leica SUMMICRON-M 50 mm f2"),
            ("Leica", "Leica APO-SUMMICRON-M 50 mm f2 ASPH."),
            ("Leica", "Leica SUMMICRON-M 35 mm f2 ASPH."),
            ("Leica", "LEICA SUMMICRON-R 35 mm f2"),
            ("Leica", "Leica Apo-Summicron-M 2_35 ASPH."),
            ("Leica", "Leica APO-TELYT-M 135 mm f3.4"),
            ("Leica", "Leica TRI-ELMAR-M 16-18-21 mm f4 ASPH."),
            ("Voigtlander/Leica", "Voigtlander VM 15mm f4.5 Super Wide Heliar II Aspherical"),
            ("Voigtlander/Leica", "Voigtlander VM 15mm f4.5 Super Wide Heliar III Aspherical"),
            ("Voigtlander/Sony", "Voigtlander SUPER WIDE-HELIAR 15mm F4.5 III"),
        ] {
            let dir = root.join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            let body = LCP.replace(
                "<stCamera:Make>Canon</stCamera:Make>",
                &format!("<stCamera:Make>Leica Camera AG</stCamera:Make><stCamera:ProfileName>Adobe ({lens})</stCamera:ProfileName>"),
            );
            std::fs::write(dir.join(format!("Leica Camera AG ({lens}) - RAW.lcp")), body).unwrap();
        }
        root
    }

    #[test]
    fn lens_names_come_apart() {
        let n = lens_name("Leica Summicron-M 1:2/50");
        assert_eq!((n.focal, n.aperture, n.zoom, n.words.clone()), (Some(50.0), Some(2.0), false, vec!["summicron".to_string(), "m".to_string()]));
        let n = lens_name("Voigtländer Heliar 1:4.5/15");
        assert_eq!((n.focal, n.aperture, n.words.clone()), (Some(15.0), Some(4.5), vec!["heliar".to_string()]));
        let n = lens_name("Leica SUMMICRON-M 50 mm f2");
        assert_eq!((n.focal, n.aperture), (Some(50.0), Some(2.0)));
        assert_eq!(n.words, ["summicron", "m"]);
        assert_eq!(lens_name("LEICA SUMMICRON-R 35 mm f2").words, ["summicron", "r"], "the mount tells them apart");
        let n = lens_name("Voigtlander VM 15mm f4.5 Super Wide Heliar II Aspherical");
        assert_eq!((n.focal, n.aperture), (Some(15.0), Some(4.5)));
        assert_eq!(n.words, ["super", "wide", "heliar", "asph"]);
        assert!(lens_name("Leica TRI-ELMAR-M 16-18-21 mm f4 ASPH.").zoom);
        assert_eq!(lens_name("Leica VARIO-ELMAR-R 21-35 mm f3,5-4 ASPH., 24 mm").aperture, None);
        assert_eq!(lens_name(""), LensName::default());
        let _ = lens_name("1:/ f/ mm 1:9999999999999999999999/1e400 f-");
    }

    /// Bryan's Leica M lenses (AppSettings.lenses) find their installed profiles by name.
    #[test]
    fn manual_leica_lenses_match_their_profiles() {
        let root = fake_profiles("match");
        let list = listing(std::slice::from_ref(&root));
        assert_eq!(list.len(), 10);
        assert!(list.iter().any(|i| i.make == "Voigtlander" && i.model == "Voigtlander VM 15mm f4.5 Super Wide Heliar II Aspherical"));
        let m = |lens: &str| match_lens_in(&list, lens, "LEICA M (Typ 262)").and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        assert_eq!(m("Leica Summicron-M 1:2/50").as_deref(), Some("Leica Camera AG (Leica SUMMICRON-M 50 mm f2) - RAW.lcp"), "not the APO");
        assert_eq!(m("Leica Summicron-M 1:2/35").as_deref(), Some("Leica Camera AG (Leica SUMMICRON-M 35 mm f2 ASPH.) - RAW.lcp"));
        assert_eq!(
            m("Voigtländer Heliar 1:4.5/15").as_deref(),
            Some("Leica Camera AG (Voigtlander VM 15mm f4.5 Super Wide Heliar II Aspherical) - RAW.lcp"),
            "the Leica-mount one"
        );
        assert_eq!(m("Leitz Elmar 1:4/135"), None, "no Elmar 135 installed: not the APO-Telyt");
        assert_eq!(m("Summicron"), None, "no focal length: no guess");
        assert_eq!(m(""), None);
        let _ = std::fs::remove_dir_all(root.parent().unwrap().parent().unwrap());
    }

    /// Set Lens → Enable Profile Corrections takes that lens's profile; a profile picked by hand
    /// (Make / Model / Profile) stays; nothing is applied while corrections are off.
    #[test]
    fn set_lens_then_profile_corrections_pick_the_installed_profile() {
        let root = fake_profiles("session");
        use_test_roots(Some(vec![root.clone()]));
        let mut s = crate::Session::with_demo();
        let id = s.active().unwrap();
        let optics = |s: &crate::Session| s.develop_of(id).unwrap().optics.clone();
        let name = |s: &crate::Session| optics(s).lens_profile_ref.map(|r| r.name);
        s.execute("photo.setLens", &serde_json::json!({"lens": "Leica Summicron-M 1:2/50", "focal": 50})).unwrap();
        assert_eq!(name(&s), None, "corrections off: nothing picked");
        let facts = crate::media::LensFacts { width: 6000.0, height: 4000.0, focal: Some(50.0), f_number: None };
        let r = s.execute("lens.profileCorrections", &serde_json::json!({"on": true})).unwrap();
        assert_eq!(r["profile"], "Adobe (Leica SUMMICRON-M 50 mm f2)", "{r}");
        assert!(facts.chosen(&s.develop_of(id).unwrap()).is_some(), "the render gets its corrections");
        // the lens changes: the profile it picked itself follows
        s.execute("photo.setLens", &serde_json::json!({"lens": "Leica Summicron-M 1:2/35", "focal": 35})).unwrap();
        assert_eq!(name(&s).as_deref(), Some("Adobe (Leica SUMMICRON-M 35 mm f2 ASPH.)"));
        // ⌘Z takes the lens and the profile back in one step
        s.execute("edit.undo", &serde_json::json!({})).unwrap();
        assert_eq!(name(&s).as_deref(), Some("Adobe (Leica SUMMICRON-M 50 mm f2)"));
        // the pickers
        let makes = s.execute("lens.profiles", &serde_json::json!({})).unwrap();
        assert_eq!(makes["makes"], serde_json::json!(["Leica", "Voigtlander"]));
        let models = s.execute("lens.profiles", &serde_json::json!({"make": "Voigtlander"})).unwrap();
        assert_eq!(models["models"].as_array().unwrap().len(), 3, "{models}");
        let model = "Voigtlander VM 15mm f4.5 Super Wide Heliar III Aspherical";
        let profs = s.execute("lens.profiles", &serde_json::json!({"make": "Voigtlander", "model": model})).unwrap();
        assert_eq!(profs["profiles"][0]["name"], format!("Adobe ({model})"));
        s.execute("lens.setProfile", &serde_json::json!({"make": "Voigtlander", "model": model})).unwrap();
        assert!(optics(&s).lens_profile_ref.unwrap().custom);
        s.execute("photo.setLens", &serde_json::json!({"lens": "Leica Summicron-M 1:2/50", "focal": 50})).unwrap();
        assert_eq!(name(&s), Some(format!("Adobe ({model})")), "picked by hand: it stays");
        assert!(s.execute("lens.setProfile", &serde_json::json!({"make": "Nope", "model": "x"})).is_err());
        s.execute("lens.setProfile", &serde_json::json!({"auto": true})).unwrap();
        assert_eq!(name(&s).as_deref(), Some("Adobe (Leica SUMMICRON-M 50 mm f2)"));
        // off: the profile is remembered but not applied
        s.execute("lens.profileCorrections", &serde_json::json!({"on": false})).unwrap();
        assert!(facts.chosen(&s.develop_of(id).unwrap()).is_none());
        s.execute("lens.setProfile", &serde_json::json!({"clear": true})).unwrap();
        assert_eq!(name(&s), None);
        use_test_roots(None);
        let _ = std::fs::remove_dir_all(root.parent().unwrap().parent().unwrap());
    }

    /// What Bryan's lenses match among the profiles installed on this Mac (ignored:
    /// `cargo test --release -p lightcraft-engine --lib -- --ignored installed_matches --nocapture`).
    #[test]
    #[ignore]
    fn installed_matches() {
        eprintln!("{} profiles, {} makes", installed().len(), makes().len());
        for lens in ["Voigtländer Heliar 1:4.5/15", "Leica Summicron-M 1:2/35", "Leica Summicron-M 1:2/50", "Leitz Elmar 1:4/135"] {
            let m = match_lens(lens, "LEICA M (Typ 262)");
            eprintln!("{lens} → {:?} {:?}", m.as_deref().and_then(reference).map(|r| r.name), m);
        }
    }

    /// Leica's profiles list the lens's other names (and Camera Raw's piecewise vignette) as lists
    /// inside an entry, before its model: the entry used to be cut off there and the profile
    /// read as empty (no corrections for any of those lenses).
    #[test]
    fn entries_with_lists_inside_keep_their_model() {
        let leica = r#"<x:xmpmeta><rdf:RDF><rdf:Description><photoshop:CameraProfiles><rdf:Seq>
     <rdf:li>
      <rdf:Description stCamera:Make="Leica Camera AG" stCamera:Lens="Leica Summicron-M 50mm f/2 (IV, V)"
       stCamera:ProfileName="Adobe (Leica SUMMICRON-M 50 mm f/2)" stCamera:SensorFormatFactor="1"
       stCamera:FocalLength="50" stCamera:FocusDistance="10000" stCamera:ApertureValue="6">
      <stCamera:AlternateLensNames><rdf:Seq><rdf:li>Summicron-M 1:2/50</rdf:li></rdf:Seq></stCamera:AlternateLensNames>
      <stCamera:PerspectiveModel stCamera:Version="2" stCamera:RadialDistortParam1="0.032306"
       stCamera:RadialDistortParam2="-0.13572" stCamera:RadialDistortParam3="-0.149371">
       <stCamera:VignetteModel stCamera:FocalLengthX="1.4" stCamera:VignetteModelParam1="-0.5">
        <stCamera:VignetteModelPiecewiseParam><rdf:Seq><rdf:li>0, 1</rdf:li><rdf:li>0.5, 0.8</rdf:li></rdf:Seq></stCamera:VignetteModelPiecewiseParam>
       </stCamera:VignetteModel>
      </stCamera:PerspectiveModel>
      </rdf:Description>
     </rdf:li>
     <rdf:li>
      <rdf:Description stCamera:ProfileName="Adobe (Leica SUMMICRON-M 50 mm f/2)" stCamera:FocalLength="50"
       stCamera:FocusDistance="10000" stCamera:ApertureValue="2">
      <stCamera:PerspectiveModel stCamera:RadialDistortParam1="0.03"/>
      </rdf:Description>
     </rdf:li>
    </rdf:Seq></photoshop:CameraProfiles></rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let e = parse(leica);
        assert_eq!(e.len(), 2, "{e:?}");
        assert_eq!(e[0].profile_name, "Adobe (Leica SUMMICRON-M 50 mm f/2)");
        assert_eq!(e[0].geom.unwrap()[4], 0.032306);
        assert_eq!(e[0].vignette.unwrap()[4], -0.5);
        assert!(corrections(&e, 6000.0, 4000.0, 50.0, Some(2.8), None).is_some_and(|l| l.warp.is_some()));
        // cut off mid-entry: no panic
        for n in [10, 200, 600, leica.len() - 40] {
            let _ = parse(&leica[..n]);
        }
        assert!(profile_items("</rdf:li></rdf:li><rdf:li").len() <= 1);
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
