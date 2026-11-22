//! Remove / Heal / Clone spots, rendered at output resolution in scene-linear light.
//!
//! - **Clone** copies the source patch with a feathered edge.
//! - **Heal** copies the source's *detail* and keeps the target's *tone*: result = src + blur(target − src)
//!   (frequency separation — a fast, stable approximation of gradient-domain healing).
//! - **Remove** is Heal with an automatically chosen source: candidate offsets on rings around the
//!   spot are scored by how well the source's surrounding annulus matches the target's.
//! - **Generative** (a Fill spot with [`Spot::generative`]): the pixels an inpainting model made for the spot
//!   ([`GenFill`], handed over by the engine through [`set_gen_loader`]) in normalized photo coordinates, resampled
//!   to the output's resolution; without them (no model, not made yet) the spot is filled like any Fill spot.

use std::sync::{Arc, PoisonError, RwLock};

use lightcraft_develop::{Spot, SpotMode};
use lightcraft_geom::Point;
use lightcraft_raster::Rgb32f;

use crate::geometry::Frame;

#[inline]
fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Mean colour of `img` in a disc/annulus.
fn ring_stats(img: &Rgb32f, c: (f32, f32), r0: f32, r1: f32) -> Option<Vec<[f32; 3]>> {
    let mut out = Vec::new();
    let n = 24;
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        for rr in [r0, (r0 + r1) / 2.0, r1] {
            let (x, y) = (c.0 + a.cos() * rr, c.1 + a.sin() * rr);
            if x < 0.0 || y < 0.0 || x >= img.width as f32 || y >= img.height as f32 {
                return None;
            }
            out.push(img.sample_bilinear(x, y));
        }
    }
    Some(out)
}

fn score(img: &Rgb32f, target: (f32, f32), source: (f32, f32), r: f32) -> f32 {
    let (Some(a), Some(b)) = (ring_stats(img, target, r * 1.05, r * 1.6), ring_stats(img, source, r * 1.05, r * 1.6)) else { return f32::MAX };
    // also compare the inside of the source against the target's surroundings (texture continuity)
    let inside = ring_stats(img, source, r * 0.2, r * 0.9);
    let mut s = 0.0;
    for (p, q) in a.iter().zip(&b) {
        for c in 0..3 {
            let (u, v) = (p[c].max(0.0).sqrt(), q[c].max(0.0).sqrt());
            s += (u - v) * (u - v);
        }
    }
    if let Some(ins) = inside {
        let mean_a: f32 = a.iter().map(|p| p[1].max(0.0).sqrt()).sum::<f32>() / a.len() as f32;
        let var: f32 = ins.iter().map(|p| (p[1].max(0.0).sqrt() - mean_a).powi(2)).sum::<f32>() / ins.len() as f32;
        s += var * a.len() as f32 * 0.5;
    }
    s
}

/// Candidate source offsets (output pixels) for a spot of radius `r` at `target`, best first
/// (candidates reaching outside the image are left out).
pub fn ranked_sources(img: &Rgb32f, target: (f32, f32), r: f32) -> Vec<(f32, f32)> {
    let mut c: Vec<(f32, (f32, f32))> = Vec::new();
    for ring in [2.4f32, 3.2, 4.2] {
        for i in 0..16 {
            let a = i as f32 / 16.0 * std::f32::consts::TAU;
            let d = (a.cos() * r * ring, a.sin() * r * ring);
            let s = score(img, target, (target.0 + d.0, target.1 + d.1), r);
            if s < f32::MAX {
                c.push((s, d));
            }
        }
    }
    c.sort_by(|a, b| a.0.total_cmp(&b.0));
    c.into_iter().map(|(_, d)| d).collect()
}

/// Pick a source offset (in output pixels) for a remove/heal spot.
pub fn auto_source(img: &Rgb32f, target: (f32, f32), r: f32) -> (f32, f32) {
    ranked_sources(img, target, r).first().copied().unwrap_or((r * 2.5, 0.0))
}

/// A source offset (normalized, as [`Spot::source_offset`]) for `spot` on `src` developed with
/// `s`: the best match, or with `avoid` (the current offset) the best one at least a spot radius
/// away from it ("refresh source"). `None` when no candidate fits in the image.
pub fn pick_source(
    src: &Rgb32f,
    info: &crate::SourceInfo,
    s: &lightcraft_develop::DevelopSettings,
    spot: &Spot,
    avoid: Option<Point>,
) -> Option<Point> {
    let frame = crate::frame_for(src, info, s, true);
    let (w, h) = frame.fit(512, 512);
    if w == 0 || h == 0 {
        return None;
    }
    let img = frame.sample(src, w, h);
    let (to_out, to_norm) = (frame.norm_to_out(w, h), frame.out_to_norm(w, h));
    let t = to_out.apply(*spot.points.first()?);
    let r = (spot.size * frame.px_per_long(w)).max(1.0) as f32;
    let avoid = avoid.map(|o| {
        let (a, b) = (to_out.apply(Point::new(0.0, 0.0)), to_out.apply(o));
        ((b.x - a.x) as f32, (b.y - a.y) as f32)
    });
    let far = |d: &(f32, f32)| avoid.is_none_or(|a| ((d.0 - a.0).powi(2) + (d.1 - a.1).powi(2)).sqrt() > r);
    let d = ranked_sources(&img, (t.x as f32, t.y as f32), r).into_iter().find(far)?;
    let (n0, n1) = (to_norm.apply(t), to_norm.apply(Point::new(t.x + d.0 as f64, t.y + d.1 as f64)));
    Some(Point::new(n1.x - n0.x, n1.y - n0.y))
}

/// Content-aware fill of the box: `src` becomes `tgt` with every pixel under the stroke (`alpha > 0`)
/// rebuilt from the rest of the box (PatchMatch completion, ported from upstream); a smooth
/// membrane fill when the box holds no whole patch to copy from.
fn fill_box(src: &mut [[f32; 3]], tgt: &[[f32; 3]], alpha: &[f32], bw: usize, bh: usize) {
    let hole: Vec<bool> = alpha.iter().map(|a| *a > 0.0).collect();
    let flat: Vec<f32> = tgt.iter().flatten().copied().collect();
    let out = lightcraft_inpaint::complete(bw, bh, 3, &flat, &hole, &lightcraft_inpaint::CompleteParams::default())
        .unwrap_or_else(|| lightcraft_inpaint::membrane_fill(bw, bh, 3, &flat, &hole));
    let (pixels, _) = out.as_chunks::<3>();
    for (d, v) in src.iter_mut().zip(pixels) {
        *d = *v;
    }
}

/// Generative Remove's pixels for one spot.
pub struct GenFill {
    /// [`gen_geometry`] of the spot they were made for: a moved or resized spot doesn't use them.
    pub geometry: u64,
    /// The rectangle they cover, normalized photo coordinates `[x0, y0, x1, y1]` (like the spot's points).
    pub rect: [f64; 4],
    /// Scene-linear working RGB *before* white balance (so a later white-balance change applies to them too).
    pub image: Rgb32f,
}

/// Finds the pixels a [`lightcraft_develop::Generative::name`] names (the engine's cache).
pub type GenLoader = dyn Fn(&str) -> Option<Arc<GenFill>> + Send + Sync;

static LOADER: RwLock<Option<Arc<GenLoader>>> = RwLock::new(None);

/// Install (or remove) the function renders get Generative Remove pixels from.
pub fn set_gen_loader(f: Option<Arc<GenLoader>>) {
    *LOADER.write().unwrap_or_else(PoisonError::into_inner) = f;
}

/// FNV-1a: stable across builds (the hash is stored with generated pixels).
fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// What a spot's generated pixels depend on in the spot itself: where it is and how big (feather and opacity
/// only blend them).
pub fn gen_geometry(spot: &Spot) -> u64 {
    let nums = spot.points.iter().flat_map(|p| [p.x, p.y]).chain([spot.size]);
    fnv(nums.flat_map(|v| v.to_bits().to_le_bytes()))
}

/// The generated pixels of `spot`, when it is a generative spot and they are there for its current geometry.
pub fn gen_fill(spot: &Spot) -> Option<Arc<GenFill>> {
    let g = spot.generative.as_ref()?;
    let load = LOADER.read().unwrap_or_else(PoisonError::into_inner).clone()?;
    let fill = load(&g.name())?;
    let ok = fill.geometry == gen_geometry(spot)
        && fill.image.width > 0
        && fill.image.height > 0
        && fill.image.data.len() == fill.image.width * fill.image.height
        && fill.rect.iter().all(|v| v.is_finite())
        && fill.rect[2] > fill.rect[0]
        && fill.rect[3] > fill.rect[1];
    ok.then_some(fill)
}

/// Which generative spots have their pixels (part of the scene-linear stage's cache key: pixels that arrive later
/// must replace the stand-in fill).
pub fn gen_state(spots: &[Spot]) -> u64 {
    spots.iter().enumerate().filter(|(_, s)| s.generative.is_some()).fold(0, |h, (i, s)| {
        if gen_fill(s).is_some() { h ^ (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15) } else { h }
    })
}

/// The generated pixels under the stroke (`alpha > 0`) of the box at `(x0, y0)`, resampled from `g` (averaging
/// several of its pixels when they are finer than the output's) and white-balanced like the photo.
fn generated_box(
    src: &mut [[f32; 3]],
    alpha: &[f32],
    g: &GenFill,
    wb: Option<[[f32; 3]; 3]>,
    to_norm: &lightcraft_geom::Affine,
    (x0, y0, bw, bh): (usize, usize, usize, usize),
) {
    let (gw, gh) = (g.image.width as f64, g.image.height as f64);
    let (rw, rh) = (g.rect[2] - g.rect[0], g.rect[3] - g.rect[1]);
    let at = |px: f64, py: f64| {
        let n = to_norm.apply(Point::new(px, py));
        ((n.x - g.rect[0]) / rw * gw, (n.y - g.rect[1]) / rh * gh)
    };
    // generated pixels per output pixel
    let (a, b) = (at(0.0, 0.0), at(1.0, 0.0));
    let k = (b.0 - a.0).hypot(b.1 - a.1);
    let taps = if k.is_finite() { (k.ceil() as usize).clamp(1, 4) } else { 1 };
    for y in 0..bh {
        for x in 0..bw {
            let i = y * bw + x;
            if alpha.get(i).is_none_or(|a| *a <= 0.0) {
                continue;
            }
            let mut acc = [0f32; 3];
            for ty in 0..taps {
                for tx in 0..taps {
                    let (px, py) = ((x0 + x) as f64 + (tx as f64 + 0.5) / taps as f64, (y0 + y) as f64 + (ty as f64 + 0.5) / taps as f64);
                    let (u, v) = at(px, py);
                    let c = g.image.sample_bilinear(u as f32, v as f32);
                    for (o, c) in acc.iter_mut().zip(c) {
                        *o += c;
                    }
                }
            }
            let n = (taps * taps) as f32;
            let c = acc.map(|v| v / n);
            let c = match &wb {
                Some(m) => std::array::from_fn(|r| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2]),
                None => c,
            };
            if let Some(d) = src.get_mut(i) {
                *d = c;
            }
        }
    }
}

/// Apply all spots to `img` (output resolution). `ppl` = output pixels per long-edge unit.
pub fn apply(img: &mut Rgb32f, spots: &[Spot], frame: &Frame, ppl: f64) {
    apply_with_wb(img, spots, frame, ppl, None);
}

/// [`apply`], with the white balance `img` has (generated pixels are kept before it).
pub fn apply_with_wb(img: &mut Rgb32f, spots: &[Spot], frame: &Frame, ppl: f64, wb: Option<[[f32; 3]; 3]>) {
    if spots.is_empty() {
        return;
    }
    let (w, h) = (img.width, img.height);
    let to_out = frame.norm_to_out(w, h);
    let to_norm = frame.out_to_norm(w, h);
    for spot in spots {
        let r = (spot.size * ppl).max(1.0) as f32;
        let pts: Vec<(f32, f32)> = spot
            .points
            .iter()
            .map(|p| {
                let q = to_out.apply(*p);
                (q.x as f32, q.y as f32)
            })
            .collect();
        let Some(&first) = pts.first() else { continue };
        let generated = if spot.mode == SpotMode::Fill { gen_fill(spot) } else { None };
        let fill = spot.mode == SpotMode::Fill;
        let offset = match spot.source_offset {
            _ if fill => (0.0, 0.0),
            Some(o) => {
                let a = to_out.apply(Point::new(0.0, 0.0));
                let b = to_out.apply(o);
                ((b.x - a.x) as f32, (b.y - a.y) as f32)
            }
            None => auto_source(img, first, r),
        };
        // Bounding box of the stroke.
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &pts {
            x0 = x0.min(p.0 - r);
            y0 = y0.min(p.1 - r);
            x1 = x1.max(p.0 + r);
            y1 = y1.max(p.1 + r);
        }
        // content-aware fill reads the spot's surroundings (two radii around it)
        let pad = if fill { 2.0 * r } else { r };
        let (bx0, by0) = (((x0 - pad).floor().max(0.0)) as usize, ((y0 - pad).floor().max(0.0)) as usize);
        let (bx1, by1) = (((x1 + pad).ceil() as usize).min(w), ((y1 + pad).ceil() as usize).min(h));
        if bx1 <= bx0 || by1 <= by0 {
            continue;
        }
        let (bw, bh) = (bx1 - bx0, by1 - by0);
        let feather = (spot.feather / 100.0).clamp(0.0, 1.0) as f32;
        let opacity = (spot.opacity / 100.0).clamp(0.0, 1.0) as f32;
        // alpha of the stroke and source patch in the box
        let mut alpha = vec![0.0f32; bw * bh];
        let mut src = vec![[0.0f32; 3]; bw * bh];
        let mut tgt = vec![[0.0f32; 3]; bw * bh];
        // distance to the nearest point, from each point's own neighbourhood (a spot covering a detected object has
        // hundreds of points; beyond a radius a point doesn't change the alpha)
        let mut dist = vec![f32::MAX; bw * bh];
        for p in &pts {
            if !(p.0.is_finite() && p.1.is_finite()) {
                continue;
            }
            let lo = |c: f32, b: usize| ((c - r - 1.0).floor().max(b as f32) as usize).saturating_sub(b);
            let hi = |c: f32, b: usize, n: usize| (((c + r + 1.0).ceil().max(0.0) as usize).saturating_sub(b)).min(n);
            for y in lo(p.1, by0)..hi(p.1, by0, bh) {
                for x in lo(p.0, bx0)..hi(p.0, bx0, bw) {
                    let (px, py) = ((bx0 + x) as f32 + 0.5, (by0 + y) as f32 + 0.5);
                    let d = ((px - p.0).powi(2) + (py - p.1).powi(2)).sqrt();
                    if let Some(m) = dist.get_mut(y * bw + x) {
                        *m = m.min(d);
                    }
                }
            }
        }
        for y in 0..bh {
            for x in 0..bw {
                let (px, py) = ((bx0 + x) as f32 + 0.5, (by0 + y) as f32 + 0.5);
                let d = dist[y * bw + x];
                // a generated fill replaces the spot outright: its feather only softens the rim
                let inner = if generated.is_some() { r * (1.0 - feather * 0.25) } else { r * (1.0 - feather * 0.8) };
                alpha[y * bw + x] = (1.0 - smooth(inner, r, d)) * opacity;
                src[y * bw + x] = img.sample_bilinear(px + offset.0, py + offset.1);
                tgt[y * bw + x] = img.get(bx0 + x, by0 + y);
            }
        }
        match &generated {
            Some(g) => generated_box(&mut src, &alpha, g, wb, &to_norm, (bx0, by0, bw, bh)),
            None if fill => fill_box(&mut src, &tgt, &alpha, bw, bh),
            None => {}
        }
        let heal = matches!(spot.mode, SpotMode::Remove | SpotMode::Heal);
        let low = if heal {
            // Tone correction from the spot's *surroundings* only: normalized convolution of
            // (target − source) over pixels outside the stroke (a membrane-like interpolation).
            let outside: Vec<f32> = alpha.iter().map(|a| if *a < 0.02 { 1.0 } else { 0.0 }).collect();
            let diff = Rgb32f {
                width: bw,
                height: bh,
                data: tgt.iter().zip(&src).zip(&outside).map(|((t, s), m)| [(t[0] - s[0]) * m, (t[1] - s[1]) * m, (t[2] - s[2]) * m]).collect(),
            };
            let wts = lightcraft_raster::Plane { width: bw, height: bh, data: outside };
            let sigma = (r * 0.6).max(1.0);
            let (bd, bm) = (lightcraft_raster::blur::gaussian(&diff, sigma), lightcraft_raster::blur::gaussian(&wts, sigma));
            Some(Rgb32f {
                width: bw,
                height: bh,
                data: bd
                    .data
                    .iter()
                    .zip(&bm.data)
                    .map(|(d, m)| {
                        let m = m.max(1e-4);
                        [d[0] / m, d[1] / m, d[2] / m]
                    })
                    .collect(),
            })
        } else {
            None
        };
        for y in 0..bh {
            for x in 0..bw {
                let i = y * bw + x;
                let a = alpha[i];
                if a <= 0.0 {
                    continue;
                }
                let mut v = src[i];
                if let Some(l) = &low {
                    let d = l.data[i];
                    v = [v[0] + d[0], v[1] + d[1], v[2] + d[2]];
                }
                let t = tgt[i];
                img.set(bx0 + x, by0 + y, [t[0] + (v[0] - t[0]) * a, t[1] + (v[1] - t[1]) * a, t[2] + (v[2] - t[2]) * a].map(|c| c.max(0.0)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::DevelopSettings;

    #[test]
    fn remove_erases_a_dot() {
        // flat grey with a gentle gradient and a black dot in the middle
        let mut img = Rgb32f::from_fn(120, 80, |x, _| [0.2 + x as f32 * 0.001; 3]);
        for y in 36..44 {
            for x in 56..64 {
                img.set(x, y, [0.0; 3]);
            }
        }
        let frame = Frame::new(120, 80, &DevelopSettings::default(), true);
        let spot = Spot { points: vec![Point::new(0.5, 0.5)], size: 8.0 / 120.0, feather: 30.0, ..Default::default() };
        apply(&mut img, &[spot], &frame, frame.px_per_long(120));
        let c = img.get(60, 40);
        assert!((c[0] - 0.26).abs() < 0.03, "{c:?}");
    }

    /// Vertical stripes (period `period` px) with a black blot over the middle.
    fn striped_with_blot(w: usize, h: usize, period: usize) -> Rgb32f {
        let mut img = Rgb32f::from_fn(w, h, |x, _| if (x / period).is_multiple_of(2) { [0.2; 3] } else { [0.7; 3] });
        let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w as f32 * 0.04);
        for y in 0..h {
            for x in 0..w {
                if (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy) < r {
                    img.set(x, y, [0.0; 3]);
                }
            }
        }
        img
    }

    fn fill_spot() -> Spot {
        // a source offset is ignored by Fill
        Spot {
            mode: SpotMode::Fill,
            points: vec![Point::new(0.5, 0.5)],
            size: 0.06,
            feather: 0.0,
            source_offset: Some(Point::new(0.3, 0.0)),
            ..Default::default()
        }
    }

    #[test]
    fn fill_rebuilds_structure_deterministically() {
        let (w, h) = (200, 160);
        let mut img = striped_with_blot(w, h, 8);
        let frame = Frame::new(w, h, &DevelopSettings::default(), true);
        apply(&mut img, &[fill_spot()], &frame, frame.px_per_long(w));
        // the blot is gone and the stripes continue through it
        let mut err = 0.0f32;
        for x in 90..110 {
            let want = if (x / 8) % 2 == 0 { 0.2 } else { 0.7 };
            err += (img.get(x, 80)[0] - want).abs();
        }
        assert!(err / 20.0 < 0.12, "mean error {}", err / 20.0);
        let mut again = striped_with_blot(w, h, 8);
        apply(&mut again, &[fill_spot()], &frame, frame.px_per_long(w));
        assert_eq!(img.data, again.data);
    }

    #[test]
    fn fill_matches_between_preview_and_export_size() {
        // the same photo at two sizes: the half-size fill matches the full-size one, downsampled
        let (mut big, mut small) = (striped_with_blot(320, 240, 16), striped_with_blot(160, 120, 8));
        let (fb, fs) = (Frame::new(320, 240, &DevelopSettings::default(), true), Frame::new(160, 120, &DevelopSettings::default(), true));
        apply(&mut big, &[fill_spot()], &fb, fb.px_per_long(320));
        apply(&mut small, &[fill_spot()], &fs, fs.px_per_long(160));
        let (mut err, mut n) = (0.0f32, 0);
        for y in 50..70 {
            for x in 70..90 {
                let b: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| big.get(2 * x + dx, 2 * y + dy)[0]).sum::<f32>() / 4.0;
                err += (b - small.get(x, y)[0]).abs();
                n += 1;
            }
        }
        assert!(err / n as f32 <= 0.08, "mean difference {}", err / n as f32);
    }

    #[test]
    fn fill_survives_hostile_spots() {
        let frame = Frame::new(40, 30, &DevelopSettings::default(), true);
        for (p, size) in [((0.0, 0.0), 2.0), ((-5.0, 9.0), 0.1), ((0.5, 0.5), 0.0), ((f64::NAN, 0.5), 0.1), ((0.5, 0.5), f64::INFINITY)] {
            let mut img = Rgb32f::from_fn(40, 30, |x, _| [x as f32 / 40.0; 3]);
            let spot = Spot { mode: SpotMode::Fill, points: vec![Point::new(p.0, p.1)], size, ..Default::default() };
            apply(&mut img, &[spot], &frame, frame.px_per_long(40));
            assert!(img.data.iter().flatten().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn generated_pixels_render_at_any_size_white_balanced_and_only_for_their_spot() {
        use std::sync::Mutex;
        static FILLS: Mutex<Vec<(String, Arc<GenFill>)>> = Mutex::new(Vec::new());
        fn find(name: &str) -> Option<Arc<GenFill>> {
            FILLS.lock().unwrap().iter().find(|(n, _)| n == name).map(|(_, f)| f.clone())
        }
        set_gen_loader(Some(Arc::new(find)));
        let spot = Spot {
            mode: SpotMode::Fill,
            points: vec![Point::new(0.5, 0.5)],
            size: 0.1,
            feather: 0.0,
            generative: Some(lightcraft_develop::Generative { variation: 0, count: 3, key: "00000000000000ab".into() }),
            ..Default::default()
        };
        // a left-to-right ramp over the middle of the photo
        let image = Rgb32f::from_fn(100, 100, |x, _| [x as f32 / 100.0, 0.5, 0.25]);
        FILLS.lock().unwrap().push(("00000000000000ab-0".into(), Arc::new(GenFill { geometry: gen_geometry(&spot), rect: [0.25, 0.25, 0.75, 0.75], image })));
        assert_ne!(gen_state(std::slice::from_ref(&spot)), 0);
        for w in [80usize, 400] {
            let mut img = Rgb32f::from_fn(w, w, |_, _| [0.1; 3]);
            let frame = Frame::new(w, w, &DevelopSettings::default(), true);
            apply_with_wb(&mut img, std::slice::from_ref(&spot), &frame, frame.px_per_long(w), Some([[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]));
            let c = img.get(w / 2, w / 2);
            // the middle of the ramp (0.5), red doubled by the white balance
            assert!((c[0] - 1.0).abs() < 0.05 && (c[1] - 0.5).abs() < 0.02 && (c[2] - 0.25).abs() < 0.02, "{w}: {c:?}");
            let left = img.get(w / 2 - w / 20, w / 2)[0];
            assert!(left < c[0], "the ramp runs left to right: {left} vs {}", c[0]);
            // outside the spot the photo is untouched
            assert_eq!(img.get(2, 2), [0.1; 3]);
        }
        // another variation (none made) and a moved spot fall back to content-aware fill
        let mut other = spot.clone();
        other.generative.as_mut().unwrap().variation = 1;
        let mut moved = spot.clone();
        moved.points[0].x = 0.51;
        for sp in [other, moved] {
            assert!(gen_fill(&sp).is_none());
            assert_eq!(gen_state(std::slice::from_ref(&sp)), 0);
            let mut img = Rgb32f::from_fn(80, 80, |_, _| [0.1; 3]);
            let frame = Frame::new(80, 80, &DevelopSettings::default(), true);
            apply(&mut img, &[sp], &frame, frame.px_per_long(80));
            assert!((img.get(40, 40)[0] - 0.1).abs() < 0.02);
        }
    }

    #[test]
    fn clone_copies_source() {
        let mut img = Rgb32f::from_fn(100, 100, |x, _| if x < 50 { [0.1; 3] } else { [0.9; 3] });
        let frame = Frame::new(100, 100, &DevelopSettings::default(), true);
        let spot = Spot {
            mode: SpotMode::Clone,
            points: vec![Point::new(0.25, 0.5)],
            size: 0.05,
            feather: 0.0,
            source_offset: Some(Point::new(0.5, 0.0)),
            ..Default::default()
        };
        apply(&mut img, &[spot], &frame, frame.px_per_long(100));
        assert!(img.get(25, 50)[0] > 0.8);
        assert!(img.get(5, 50)[0] < 0.2);
    }
}
