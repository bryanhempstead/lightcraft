//! Region renders: one rectangle of a whole render, at the whole render's scale, without rendering
//! the rest (the loupe's native-resolution tiles at 1:1, `crates/ui-egui/src/tiles.rs`).
//!
//! A region is rendered as a padded window of the whole render ([`crate::geometry::Window`]):
//! every mapping between output pixels and the photo (sampling, lens corrections, masks, spots,
//! red eye, vignette, grain, the scene tone map) places the window's pixels where they sit in the
//! whole render, and the window extends past the requested pixels by the reach of the spatial
//! filters (guided-filter bases, clarity, dehaze, noise reduction, texture pyramid, sharpening)
//! so they see the same neighbourhood. Its origin is aligned to [`ALIGN`] pixels, the size of
//! the coarsest pyramid / subsampling grid, so subsampled stages land on the whole render's grid.
//! What depends on the whole photo — tone statistics, negative Dehaze's percentile, the
//! airlight, Camera Raw's scene tone map and haze — comes from a whole-photo pass ([`Globals`],
//! usually at preview size). Spot removal reads its source pixels anywhere in the photo: the
//! window grows to every spot box the requested pixels depend on (and the spots those read).
//! Lens Blur, auto-sourced spots and very wide filters render the whole frame instead (the region
//! is then cut from it), so a region always equals the same pixels of the whole render.

use std::sync::Arc;

use lightcraft_develop::{DevelopSettings, SpotMode};
use lightcraft_geom::Point;

use crate::geometry::{Frame, Window};
use crate::{Quality, local};

/// A rectangle of a whole render, normalized to its width and height (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Region {
    pub const WHOLE: Region = Region { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };

    /// The region's pixels in a `full_w × full_h` render (rounded to whole pixels; `None` when
    /// it is empty, not finite or outside the render).
    pub fn pixels(&self, full_w: usize, full_h: usize) -> Option<RegionPx> {
        let v = [self.x0, self.y0, self.x1, self.y1];
        if !v.iter().all(|c| c.is_finite()) || full_w == 0 || full_h == 0 {
            return None;
        }
        let px = |c: f64, n: usize| (c.clamp(0.0, 1.0) * n as f64).round() as usize;
        let (x0, x1) = (px(self.x0, full_w), px(self.x1, full_w).min(full_w));
        let (y0, y1) = (px(self.y0, full_h), px(self.y1, full_h).min(full_h));
        (x1 > x0 && y1 > y0).then_some(RegionPx { x: x0, y: y0, w: x1 - x0, h: y1 - y0, full_w, full_h })
    }

    /// Part of a render's cache key.
    pub fn key(&self) -> u64 {
        [self.x0, self.y0, self.x1, self.y1].iter().fold(0x5245_4749_4f4e_u64, |h, v| (h ^ v.to_bits()).wrapping_mul(0x0100_0000_01b3))
    }
}

/// A region's pixels in its whole render: `w × h` at (`x`, `y`) of `full_w × full_h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionPx {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub full_w: usize,
    pub full_h: usize,
}

impl RegionPx {
    /// The rectangle normalized to the whole render (where the pixels go on screen).
    pub fn norm(&self) -> Region {
        let (fw, fh) = (self.full_w.max(1) as f64, self.full_h.max(1) as f64);
        Region { x0: self.x as f64 / fw, y0: self.y as f64 / fh, x1: (self.x + self.w) as f64 / fw, y1: (self.y + self.h) as f64 / fh }
    }
}

/// What a region render takes from the whole photo (see the module docs), as a whole render at
/// another size computes it ([`globals`]).
#[derive(Clone, Debug, Default)]
pub struct Globals {
    pub tone_stats: Option<crate::tone::lr::ToneStats>,
    pub p99: Option<f32>,
    pub air: Option<f32>,
    pub scene_map: Option<Arc<crate::scene_cr::MapBase>>,
    pub haze: Option<Arc<crate::scene_cr::Haze>>,
}

/// Window origins (and the window's far edges, inside the render) are multiples of this: the
/// texture pyramid's coarsest level (2⁶) and the fast guided filter's largest step divide it.
pub const ALIGN: usize = 64;

/// A window covering more than this share of the whole render renders the whole frame instead.
const WHOLE_SHARE: f64 = 0.6;

/// How far (output px) the spatial stages of `s` reach at `px_per_long` output pixels per long
/// edge, for a render whose long edge is `out_long` px from a source of `src_long` px.
pub fn reach(s: &DevelopSettings, px_per_long: f64, q: Quality, src_long: usize, out_long: usize) -> f64 {
    let sig = local::plane_sigmas(s, px_per_long, q);
    // guided filters blur twice (the image, then its coefficients) and the fast one reads its
    // coefficients off a subsampled grid; Gaussians are three box passes (~3σ)
    let guided = |sg: f32| 6.0 * sg as f64 + 2.0 * local::guided_fast_step(sg) as f64;
    let gauss = |sg: f32| 3.5 * sg as f64;
    let mut r: f64 = 48.0; // sharpening, defringe, red eye, the per-pixel stage's neighbours
    for g in [sig.base, sig.clarity].into_iter().flatten() {
        r = r.max(guided(g));
    }
    for g in [sig.texture, sig.dark, sig.chroma, sig.dz].into_iter().flatten() {
        r = r.max(gauss(g));
    }
    let (lum, col) = local::nr_params(s, src_long, out_long);
    if let Some(n) = lum {
        r = r.max(6.0 * n.sigma as f64);
    }
    if let Some(n) = col {
        r = r.max(gauss(n.sigma));
    }
    if local::texture_level(s, px_per_long, 0.0).is_some() || s.effects.texture != 0.0 {
        // Camera Raw's Texture: a 6-level pyramid of 5-tap filters
        r = r.max(4.0 * (1u32 << crate::texture_cr::LEVELS) as f64);
    }
    for m in s.masks.iter().filter(|m| m.visible && m.refine > 0.0) {
        let k = (m.refine / 100.0).clamp(0.0, 1.0);
        r = r.max(6.0 * (0.04 * out_long as f64 * k).max(1.0));
    }
    r.ceil()
}

/// A pixel rectangle `[x0, y0, x1, y1)` of the whole render.
type Box4 = [f64; 4];

fn overlaps(a: &Box4, b: &Box4) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn union(a: &mut Box4, b: &Box4) {
    a[0] = a[0].min(b[0]);
    a[1] = a[1].min(b[1]);
    a[2] = a[2].max(b[2]);
    a[3] = a[3].max(b[3]);
}

/// The boxes (whole-render px) spot `i` writes and reads, `None` when it can't be placed without
/// the whole image (its source is found by searching the photo).
fn spot_boxes(sp: &lightcraft_develop::Spot, to_out: &lightcraft_geom::Affine, ppl: f64) -> Option<Option<(Box4, Box4)>> {
    let r = (sp.size * ppl).max(1.0);
    let fill = sp.mode == SpotMode::Fill;
    let mut b: Box4 = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in &sp.points {
        let q = to_out.apply(*p);
        if !(q.x.is_finite() && q.y.is_finite()) {
            continue;
        }
        union(&mut b, &[q.x - r, q.y - r, q.x + r, q.y + r]);
    }
    if b[0] > b[2] {
        return Some(None); // no points: the spot does nothing
    }
    // as `spots::apply_with_wb` pads its box (content-aware fill reads two radii around it)
    let pad = if fill { 2.0 * r } else { r } + 2.0;
    let target = [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad];
    let offset = match sp.source_offset {
        _ if fill => (0.0, 0.0),
        Some(o) => {
            let (a, c) = (to_out.apply(Point::new(0.0, 0.0)), to_out.apply(o));
            (c.x - a.x, c.y - a.y)
        }
        None => return None,
    };
    let source = [target[0] + offset.0 - 2.0, target[1] + offset.1 - 2.0, target[2] + offset.0 + 2.0, target[3] + offset.1 + 2.0];
    Some(Some((target, source)))
}

/// Place `region` of a `full_w × full_h` render: set `frame`'s window (unless the window would
/// be the whole render) and return the requested pixels and the window's size. `None`: the
/// region is empty.
#[allow(clippy::too_many_arguments)]
pub fn place(
    region: Region,
    frame: &mut Frame,
    s: &DevelopSettings,
    full_w: usize,
    full_h: usize,
    px_per_long: f64,
    q: Quality,
    src_long: usize,
) -> Option<(RegionPx, usize, usize)> {
    let pad = reach(s, px_per_long, q, src_long, full_w.max(full_h));
    place_with(region, frame, s, full_w, full_h, px_per_long, pad, WHOLE_SHARE)
}

/// [`place`] with the window padded by `pad` px, the whole render once it covers more than
/// `whole_share` of it.
#[allow(clippy::too_many_arguments)]
fn place_with(
    region: Region,
    frame: &mut Frame,
    s: &DevelopSettings,
    full_w: usize,
    full_h: usize,
    px_per_long: f64,
    pad: f64,
    whole_share: f64,
) -> Option<(RegionPx, usize, usize)> {
    let px = region.pixels(full_w, full_h)?;
    let whole = |frame: &mut Frame| {
        frame.window = None;
        Some((px, full_w, full_h))
    };
    if crate::lens_blur::active(s) {
        return whole(frame);
    }
    let mut win: Box4 = [px.x as f64 - pad, px.y as f64 - pad, (px.x + px.w) as f64 + pad, (px.y + px.h) as f64 + pad];
    // spot removal: every spot whose box meets the window, with the pixels it reads (which may
    // be another spot's result: repeat until nothing more is pulled in)
    if !s.spots.is_empty() {
        let to_out = frame.whole().norm_to_out(full_w, full_h);
        let mut boxes = Vec::with_capacity(s.spots.len());
        for sp in &s.spots {
            match spot_boxes(sp, &to_out, px_per_long) {
                Some(b) => boxes.push(b),
                None => {
                    // an auto-sourced spot near the region: its source is searched for in the photo
                    let r = (sp.size * px_per_long).max(1.0) * 6.0;
                    let near = sp.points.iter().any(|p| {
                        let q = to_out.apply(*p);
                        overlaps(&[q.x - r, q.y - r, q.x + r, q.y + r], &win)
                    });
                    if near {
                        return whole(frame);
                    }
                    boxes.push(None);
                }
            }
        }
        let mut taken = vec![false; boxes.len()];
        for _ in 0..=boxes.len() {
            let mut grew = false;
            for (i, b) in boxes.iter().enumerate() {
                let Some((t, src)) = b else { continue };
                if taken[i] || !overlaps(t, &win) {
                    continue;
                }
                taken[i] = true;
                union(&mut win, t);
                union(&mut win, &[src[0] - pad, src[1] - pad, src[2] + pad, src[3] + pad]);
                grew = true;
            }
            if !grew {
                break;
            }
        }
    }
    // aligned, inside the render
    let lo = |v: f64| ((v.max(0.0) / ALIGN as f64).floor() as usize) * ALIGN;
    let hi = |v: f64, n: usize| (((v.max(0.0) / ALIGN as f64).ceil() as usize).saturating_mul(ALIGN)).min(n);
    let (x0, y0) = (lo(win[0]).min(px.x), lo(win[1]).min(px.y));
    let (x1, y1) = (hi(win[2], full_w).max(px.x + px.w), hi(win[3], full_h).max(px.y + px.h));
    let (ww, wh) = (x1.saturating_sub(x0).max(1), y1.saturating_sub(y0).max(1));
    if (ww as f64 * wh as f64) > whole_share * full_w as f64 * full_h as f64 || (x0 == 0 && y0 == 0 && ww == full_w && wh == full_h) {
        return whole(frame);
    }
    frame.window = Some(Window { x: x0, y: y0, full_w, full_h });
    Some((px, ww, wh))
}

/// Cut `w × h` pixels at (`x`, `y`) out of a buffer of `T` samples, `n` per pixel, `src_w`
/// pixels wide (missing samples: `T::default()`).
pub fn crop_samples<T: Copy + Default>(data: &[T], src_w: usize, n: usize, (x, y, w, h): (usize, usize, usize, usize)) -> Vec<T> {
    let mut out = Vec::with_capacity(w * h * n);
    for row in y..y + h {
        let at = (row * src_w + x) * n;
        out.extend_from_slice(data.get(at..at + w * n).unwrap_or(&[]));
        out.resize((row - y + 1) * w * n, T::default());
    }
    out
}

/// Identifies the [`Globals`] of `s` for one photo: they follow the scene-linear image (white
/// balance, lens corrections, spots, noise reduction…) and which of them the settings use, not
/// the crop (they are the uncropped photo's) nor the per-pixel stage's sliders — an Exposure,
/// tone or colour drag, or a crop drag, keeps them.
pub fn globals_key(s: &DevelopSettings) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
    let e = s.effective();
    let sig = local::plane_sigmas(&e, 1000.0, Quality::Full);
    let needs = (
        sig.base.is_some(),
        sig.dark.is_some(),
        local::dehaze_key_needed(&e),
        // per photo the Adobe base is fixed; `true` keys every operator it could need
        crate::scene_cr::needs(&e, true),
        crate::settings_for(&e, &crate::RenderRequest::fit(1, 1)).hdr.enabled,
    );
    let mut k = e.into_owned();
    // the photo is measured whole and uncropped (`crate::globals`): no crop, straighten, aspect
    // or flip moves what it says
    k.crop = Default::default();
    // what only the per-pixel stage reads
    k.light = Default::default();
    k.curve = Default::default();
    k.color = Default::default();
    k.mixer = Default::default();
    k.point_colors.clear();
    k.bw_mix = Default::default();
    k.grading = Default::default();
    k.effects = Default::default();
    k.vignette = Default::default();
    k.grain = Default::default();
    k.hdr = Default::default();
    k.calibration = Default::default();
    k.masks.clear();
    let d = &mut k.detail;
    (d.sharpen_amount, d.sharpen_radius, d.sharpen_detail, d.sharpen_masking) = Default::default();
    BuildHasherDefault::<DefaultHasher>::default().hash_one((k.hash64(), needs))
}

/// [`Globals`] for a region render of `s` (of `_full`, the source it renders from, for `_req`):
/// the whole, uncropped photo's tone statistics, percentile, airlight, scene map and haze
/// ([`crate::globals`]), measured from `ctx` (the preview-size source) whatever the size and crop
/// of the region's whole render. The render's own source and request take no part (they once did:
/// the scene map was measured on the render's cropped frame): a region reads the photo-wide
/// values the whole render reads.
#[allow(clippy::too_many_arguments)]
pub fn globals_for(
    _full: &lightcraft_raster::Rgb32f,
    _full_info: &crate::SourceInfo,
    ctx: &lightcraft_raster::Rgb32f,
    ctx_info: &crate::SourceInfo,
    s: &DevelopSettings,
    _req: &crate::RenderRequest,
    ctx_req: &crate::RenderRequest,
) -> Globals {
    crate::globals(ctx, ctx_info, s, ctx_req)
}
