//! Lens Blur: a shallow depth of field added after the fact, from the photo's depth map.
//!
//! Each pixel's circle of confusion grows with how far its depth lies outside the focal range
//! ([`lightcraft_develop::LensBlur`]: `focal_distance ± focal_range / 2`, 0 nearest … 100 farthest), up to `R` set
//! by Blur Amount relative to the long edge, so previews and exports match. Refine brushes paint focus (sharp) or
//! blur (fully out of focus) by hand.
//!
//! The blur is gathered from [`LEVELS`] kernel blurs of growing radius, blended per pixel by its circle of
//! confusion. Each runs on a grid coarse enough that the kernel spans about [`DISC_CELLS`] cells (its cost doesn't
//! grow with the radius) and is a normalized convolution weighted by how far out of focus each pixel is, so the
//! sharp subject doesn't bleed into the blurred background around it. Boost brightens highlights before they are
//! spread, so they turn into bright bokeh, shaped by the Bokeh setting (circle, bubble, 5-blade, ring).
//!
//! It runs on the scene-linear, white-balanced image before exposure (a blur is linear, so exposure commutes with
//! it), on the CPU for both renderers.
//!
//! The multi-level normalized disc gather (`blur`, `level`, `level_grid`, the prefix-sum kernel) is taken from
//! upstream storytold/lightcraft's `crates/pipeline/src/lens_blur.rs` (MIT OR Apache-2.0); the depth-driven circle of
//! confusion, bokeh shapes and refine brushes are this fork's.

use lightcraft_develop::{Bokeh, DevelopSettings, MaskShape};
use lightcraft_raster::{Plane, Rgb32f};

use crate::geometry::Frame;

/// The largest circle of confusion (Blur Amount 100), as a fraction of the long edge.
pub const MAX_RADIUS: f64 = 0.035;
/// Depth outside the focal range at which the blur reaches its full size (depth runs 0..1).
const FULL_BLUR_DEPTH: f32 = 0.35;
/// Kernel blurs the per-pixel circle of confusion is interpolated between.
const LEVELS: usize = 4;
/// Radius, in grid cells, a level's kernel works at.
const DISC_CELLS: f32 = 8.0;
/// Most cells a level's grid holds (bounds the memory of a full-size export).
const MAX_CELLS: usize = 3_000_000;
/// Weight of in-focus pixels in the blurred levels: enough to keep a level defined where only in-focus pixels are
/// near, too little to bleed them into the background.
const FOCUS_WEIGHT: f32 = 1e-4;
/// The brightest highlights grow this much brighter at Boost 100 before they are spread.
const BOOST_GAIN: f32 = 4.0;

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Whether `s` renders a lens blur (switched on with some amount, its section's eye open, and a depth map or a
/// painted blur to go by).
pub fn active(s: &DevelopSettings) -> bool {
    let l = &s.lens_blur;
    l.enabled && l.amount > 0.0 && s.section_enabled("lensBlur") && (s.depth.is_some() || !l.blur_strokes.is_empty())
}

/// The largest circle of confusion in output pixels.
pub fn radius_px(s: &DevelopSettings, px_per_long: f64) -> f32 {
    let a = if s.lens_blur.amount.is_finite() { s.lens_blur.amount.clamp(0.0, 100.0) } else { 0.0 };
    (MAX_RADIUS * a / 100.0 * px_per_long) as f32
}

/// What the scene-linear stage's cache key needs from the lens blur.
pub fn key(s: &DevelopSettings) -> String {
    if !active(s) {
        return String::new();
    }
    let depth = s.depth.as_ref().map(|d| d.data.len() as u64 ^ d.data.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b))));
    format!("{:?} {depth:?}", s.lens_blur)
}

/// The circle of confusion (0..1 of the full blur) of a pixel at depth `d` (0 nearest … 1 farthest).
pub fn coc_of_depth(s: &DevelopSettings, d: f32) -> f32 {
    let l = &s.lens_blur;
    let c = (l.focal_distance.clamp(0.0, 100.0) / 100.0) as f32;
    let half = (l.focal_range.clamp(0.0, 100.0) / 200.0) as f32;
    let outside = ((d - c).abs() - half).max(0.0);
    smooth(0.0, FULL_BLUR_DEPTH, outside)
}

/// The circle of confusion of every output pixel (0..1).
pub fn coc_plane(s: &DevelopSettings, frame: &Frame, w: usize, h: usize, img: &Rgb32f) -> Vec<f32> {
    let m = frame.out_to_norm(w, h);
    let mut coc = vec![0f32; w * h];
    if let Some(grid) = frame.depth.as_deref() {
        crate::for_rows(&mut coc, w, |y, row| {
            for (x, c) in row.iter_mut().enumerate() {
                let n = m.apply(lightcraft_geom::Point::new(x as f64 + 0.5, y as f64 + 0.5));
                *c = coc_of_depth(s, grid.sample(n.x, n.y));
            }
        });
        // the depth map is coarse (a few hundred cells): snap its edges to the photo's (guided filter on log
        // luminance), so the subject's outline is cut where the photo has it, not on the map's grid
        let cell = (w.max(h) as f32 / grid.width.max(grid.height).max(1) as f32).max(1.0);
        if cell > 1.5 {
            let mut log_l = Plane::new(w, h);
            for (o, p) in log_l.data.iter_mut().zip(&img.data) {
                *o = (lightcraft_color::luminance_2020(*p).max(0.0) + 1e-4).log2();
            }
            let plane = Plane { width: w, height: h, data: coc };
            coc = crate::masks::guided_cross(&log_l, &plane, 1.5 * cell, 0.02).data;
        }
    }
    let l = &s.lens_blur;
    let brush = |strokes: &[lightcraft_develop::BrushStroke]| {
        let log_l = Plane::new(w, h);
        crate::masks::shape_alpha(&MaskShape::Brush { strokes: strokes.to_vec() }, frame, w, h, img, &log_l, 0.0)
    };
    if !l.focus_strokes.is_empty() {
        let a = brush(&l.focus_strokes);
        for (c, v) in coc.iter_mut().zip(&a.data) {
            *c *= 1.0 - v.clamp(0.0, 1.0);
        }
    }
    if !l.blur_strokes.is_empty() {
        let a = brush(&l.blur_strokes);
        for (c, v) in coc.iter_mut().zip(&a.data) {
            *c = c.max(v.clamp(0.0, 1.0));
        }
    }
    coc
}

/// Blur `img` (the output-sized scene-linear image) as `s` says.
pub fn apply(img: &mut Rgb32f, s: &DevelopSettings, frame: &Frame, px_per_long: f64) {
    if !active(s) {
        return;
    }
    let r = radius_px(s, px_per_long);
    if !(r.is_finite() && r >= 0.5) {
        return;
    }
    let coc = coc_plane(s, frame, img.width, img.height, img);
    let boost = (s.lens_blur.boost.clamp(0.0, 100.0) / 100.0) as f32;
    blur(img, &coc, r, boost, s.lens_blur.bokeh);
}

/// Blur `img` by a circle of confusion `coc · r` pixels per pixel (`coc` 0..1, one per pixel), highlights
/// brightened by `boost` (0..1) before they are spread, in the shape `bokeh`.
fn blur(img: &mut Rgb32f, coc: &[f32], r: f32, boost: f32, bokeh: Bokeh) {
    let w = img.width;
    let gain = |c: [f32; 3]| 1.0 + BOOST_GAIN * boost * smooth(0.5, 1.0, lightcraft_color::luminance_2020(c));
    let levels: Vec<Level> = (1..=LEVELS).map(|k| level(img, coc, k, r, &gain, bokeh)).collect();
    let src = img.clone();
    crate::for_rows(&mut img.data, w, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let t = coc.get(i).copied().unwrap_or(0.0) * LEVELS as f32;
            let k = (t.floor().max(0.0) as usize).min(LEVELS - 1);
            let fr = (t - k as f32).clamp(0.0, 1.0);
            let at = |k: usize| match k {
                0 => src.data.get(i).copied().unwrap_or(*px),
                k => levels.get(k - 1).map_or(*px, |l| l.sample(x, y)),
            };
            let (a, b) = (at(k), at(k + 1));
            *px = std::array::from_fn(|c| a[c] + (b[c] - a[c]) * fr);
        }
    });
}

/// One kernel blur, on a grid `f` times coarser than the image.
struct Level {
    img: Rgb32f,
    f: usize,
}

impl Level {
    fn sample(&self, x: usize, y: usize) -> [f32; 3] {
        let f = self.f as f32;
        self.img.sample_bilinear((x as f32 + 0.5) / f, (y as f32 + 0.5) / f)
    }
}

/// Level `k` of [`LEVELS`] (radius `k / LEVELS · r` pixels): the kernel blur of the pixels at least that far out
/// of focus.
fn level(img: &Rgb32f, coc: &[f32], k: usize, r: f32, gain: &(impl Fn([f32; 3]) -> f32 + Sync), bokeh: Bokeh) -> Level {
    let (w, h) = (img.width, img.height);
    let radius = r * k as f32 / LEVELS as f32;
    let (f, lw, lh) = level_grid(w, h, radius);
    let (lo, hi) = ((k - 1) as f32 / LEVELS as f32, k as f32 / LEVELS as f32);
    let mut cells = vec![[0f32; 4]; lw * lh];
    crate::for_rows(&mut cells, lw, |cy, row| {
        for (cx, cell) in row.iter_mut().enumerate() {
            let mut acc = [0f32; 4];
            for y in cy * f..((cy + 1) * f).min(h) {
                for x in cx * f..((cx + 1) * f).min(w) {
                    let i = y * w + x;
                    let (Some(c), Some(d)) = (img.data.get(i), coc.get(i)) else { continue };
                    if !c.iter().all(|v| v.is_finite()) {
                        continue;
                    }
                    let wt = FOCUS_WEIGHT + smooth(lo, hi, *d);
                    let g = gain(*c) * wt;
                    acc = [acc[0] + c[0] * g, acc[1] + c[1] * g, acc[2] + c[2] * g, acc[3] + wt];
                }
            }
            *cell = acc;
        }
    });
    let rho = radius / f as f32;
    let sums = |rho: f32| kernel(&cells, lw, lh, &spans(bokeh, rho));
    // ring and bubble: a disc minus (or plus a rim of) a smaller one
    let combined: Vec<[f64; 4]> = match bokeh {
        Bokeh::Circle | Bokeh::FiveBlade => sums(rho),
        Bokeh::Ring => {
            let (o, i) = (sums(rho), kernel(&cells, lw, lh, &spans(Bokeh::Circle, rho * 0.65)));
            o.iter().zip(&i).map(|(a, b)| std::array::from_fn(|c| (a[c] - 0.92 * b[c]).max(0.0))).collect()
        }
        Bokeh::Bubble => {
            let (o, i) = (sums(rho), kernel(&cells, lw, lh, &spans(Bokeh::Circle, rho * 0.8)));
            o.iter().zip(&i).map(|(a, b)| std::array::from_fn(|c| a[c] + 1.5 * (a[c] - b[c]).max(0.0))).collect()
        }
    };
    let mut out = Rgb32f::new(lw, lh);
    for (o, a) in out.data.iter_mut().zip(&combined) {
        *o = if a[3] > 1e-12 { std::array::from_fn(|c| (a[c] / a[3]) as f32) } else { [0.0; 3] };
    }
    // cells with no weight at all (a ring's hole on a tiny grid): the plain cell average
    for (o, c) in out.data.iter_mut().zip(&cells) {
        if *o == [0.0; 3] && c[3] > 0.0 {
            *o = [c[0] / c[3], c[1] / c[3], c[2] / c[3]];
        }
    }
    Level { img: out, f }
}

/// Grid scale and dimensions for one level, bounded.
fn level_grid(w: usize, h: usize, radius: f32) -> (usize, usize, usize) {
    let at_least = ((w as f64 * h as f64) / MAX_CELLS as f64).sqrt().ceil() as usize;
    let max_edge = w.max(h).max(1);
    let mut f = ((radius / DISC_CELLS).ceil() as usize).max(at_least).clamp(1, max_edge);
    loop {
        let (lw, lh) = (w.div_ceil(f), h.div_ceil(f));
        if lw.saturating_mul(lh) <= MAX_CELLS || f == max_edge {
            return (f, lw, lh);
        }
        f = f.saturating_add(1);
    }
}

/// The kernel of `bokeh` with radius `rho` cells as rows `(dy, half width)` (symmetric about its vertical axis).
fn spans(bokeh: Bokeh, rho: f32) -> Vec<(isize, f32)> {
    let rho = if rho.is_finite() { rho.clamp(0.0, 4096.0) } else { 0.0 };
    let n = rho.ceil() as isize;
    (-n..=n)
        .filter_map(|dy| {
            let y = dy as f32;
            let hw = match bokeh {
                Bokeh::FiveBlade => polygon_half_width(5, rho, y),
                _ => {
                    let q = rho * rho - y * y;
                    (q >= 0.0).then(|| q.sqrt())
                }
            }?;
            Some((dy, hw))
        })
        .collect()
}

/// Half the width at height `y` of a regular `n`-gon of circumradius `rho` with a vertex up; `None` above or below
/// it.
fn polygon_half_width(n: usize, rho: f32, y: f32) -> Option<f32> {
    let inside = |x: f32| {
        (0..n).all(|i| {
            // edge i between vertices i and i + 1 (vertex 0 at the top, y down)
            let a0 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / n as f32;
            let a1 = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (i + 1) as f32 / n as f32;
            let (x0, y0, x1, y1) = (rho * a0.cos(), rho * a0.sin(), rho * a1.cos(), rho * a1.sin());
            (x1 - x0) * (y - y0) - (y1 - y0) * (x - x0) >= -1e-4
        })
    };
    if !inside(0.0) {
        return None;
    }
    let (mut lo, mut hi) = (0.0f32, rho.max(0.0));
    for _ in 0..20 {
        let mid = 0.5 * (lo + hi);
        if inside(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(lo)
}

/// Weighted sums `cells` (`[r·w, g·w, b·w, w]`, `lw × lh`) gathered over the kernel `spans`: per row of the kernel,
/// a span read from that row's prefix sums, its two end cells weighted by the fraction of them inside.
fn kernel(cells: &[[f32; 4]], lw: usize, lh: usize, spans: &[(isize, f32)]) -> Vec<[f64; 4]> {
    let stride = lw + 1;
    let mut pre = vec![[0f64; 4]; stride * lh];
    crate::for_rows(&mut pre, stride, |y, p| {
        let Some(row) = cells.get(y * lw..(y + 1) * lw) else { return };
        let mut acc = [0f64; 4];
        for (x, c) in row.iter().enumerate() {
            for ch in 0..4 {
                acc[ch] += c[ch] as f64;
            }
            if let Some(slot) = p.get_mut(x + 1) {
                *slot = acc;
            }
        }
    });
    let spans: Vec<(isize, isize, f64)> = spans.iter().map(|&(dy, hw)| (dy, hw.floor() as isize, (hw - hw.floor()) as f64)).collect();
    let mut out = vec![[0f64; 4]; lw * lh];
    let (lwi, lhi) = (lw as isize, lh as isize);
    crate::for_rows(&mut out, lw, |y, row| {
        for (x, px) in row.iter_mut().enumerate() {
            let x = x as isize;
            let mut acc = [0f64; 4];
            for &(dy, hw, fr) in &spans {
                let yy = y as isize + dy;
                if yy < 0 || yy >= lhi {
                    continue;
                }
                let base = yy as usize * stride;
                let (a, b) = ((x - hw).max(0), (x + hw + 1).min(lwi));
                if a < b
                    && let (Some(pa), Some(pb)) = (pre.get(base + a as usize), pre.get(base + b as usize))
                {
                    for ch in 0..4 {
                        acc[ch] += pb[ch] - pa[ch];
                    }
                }
                if fr > 0.0 {
                    for xx in [x - hw - 1, x + hw + 1] {
                        if xx >= 0
                            && xx < lwi
                            && let Some(c) = cells.get(yy as usize * lw + xx as usize)
                        {
                            for ch in 0..4 {
                                acc[ch] += fr * c[ch] as f64;
                            }
                        }
                    }
                }
            }
            *px = acc;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_develop::DepthMap;

    /// Settings blurring everything deeper than the nearest tenth, with a depth map that is near on the left half
    /// and far on the right.
    fn left_near(amount: f64) -> DevelopSettings {
        let mut s = DevelopSettings::default();
        s.lens_blur.enabled = true;
        s.lens_blur.amount = amount;
        s.lens_blur.focal_distance = 5.0;
        s.lens_blur.focal_range = 10.0;
        let (w, h) = (16, 8);
        let v: Vec<f32> = (0..w * h).map(|i| if i % w < w / 2 { 0.0 } else { 1.0 }).collect();
        s.depth = Some(DepthMap::from_values(w, h, &v));
        s
    }

    fn stripes(w: usize, h: usize) -> Rgb32f {
        Rgb32f::from_fn(w, h, |x, _| if x % 4 < 2 { [0.8; 3] } else { [0.05; 3] })
    }

    fn contrast(img: &Rgb32f, x0: usize, x1: usize) -> f32 {
        let y = img.height / 2;
        let v: Vec<f32> = (x0..x1).map(|x| img.data[y * img.width + x][1]).collect();
        v.iter().cloned().fold(0.0, f32::max) - v.iter().cloned().fold(1.0, f32::min)
    }

    #[test]
    fn the_far_side_blurs_and_the_near_side_stays_sharp() {
        for scale in [1usize, 3] {
            let (w, h) = (160 * scale, 80 * scale);
            let s = left_near(100.0);
            let frame = Frame::new(w, h, &s, true);
            let mut img = stripes(w, h);
            apply(&mut img, &s, &frame, w as f64);
            let near = contrast(&img, w / 8, w / 4);
            let far = contrast(&img, w * 3 / 4, w * 7 / 8);
            assert!(near > 0.7, "near stripes keep their contrast ({near}) at ×{scale}");
            assert!(far < 0.1, "far stripes are blurred away ({far}) at ×{scale}");
        }
    }

    #[test]
    fn the_focal_range_picks_what_stays_sharp() {
        let mut s = left_near(100.0);
        s.lens_blur.focal_distance = 95.0;
        let (w, h) = (160, 80);
        let frame = Frame::new(w, h, &s, true);
        let mut img = stripes(w, h);
        apply(&mut img, &s, &frame, w as f64);
        assert!(contrast(&img, w * 3 / 4, w * 7 / 8) > 0.7);
        assert!(contrast(&img, w / 8, w / 4) < 0.1);
    }

    #[test]
    fn off_without_depth_or_amount_and_every_shape_is_finite() {
        let (w, h) = (64, 48);
        let mut s = left_near(0.0);
        let frame = Frame::new(w, h, &s, true);
        let mut img = stripes(w, h);
        let before = img.clone();
        apply(&mut img, &s, &frame, w as f64);
        assert_eq!(img, before);
        s.lens_blur.amount = 80.0;
        s.lens_blur.boost = 100.0;
        for b in Bokeh::ALL {
            s.lens_blur.bokeh = b;
            let mut img = stripes(w, h);
            img.data[20 * w + 50] = [20.0; 3];
            apply(&mut img, &s, &frame, w as f64);
            assert!(img.data.iter().flatten().all(|v| v.is_finite()), "{b:?}");
        }
        s.depth = None;
        assert!(!active(&s));
    }

    #[test]
    fn a_depth_range_mask_selects_by_depth() {
        let (w, h) = (80, 40);
        let s = left_near(0.0);
        let frame = Frame::new(w, h, &s, true);
        let img = stripes(w, h);
        let shape = MaskShape::DepthRange { lo: 0.0, hi: 0.3, feather: 0.05 };
        let a = crate::masks::shape_alpha(&shape, &frame, w, h, &img, &Plane::new(w, h), 0.0);
        assert!(a.data[20 * w + 10] > 0.99 && a.data[20 * w + 70] < 0.01, "{} {}", a.data[20 * w + 10], a.data[20 * w + 70]);
        // without a depth map it selects nothing
        let bare = Frame::new(w, h, &DevelopSettings::default(), true);
        assert!(crate::masks::shape_alpha(&shape, &bare, w, h, &img, &Plane::new(w, h), 0.0).data.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_pentagon_is_narrower_than_its_circle() {
        let rho = 10.0;
        let c: f32 = spans(Bokeh::Circle, rho).iter().map(|s| s.1).sum();
        let p: f32 = spans(Bokeh::FiveBlade, rho).iter().map(|s| s.1).sum();
        assert!(p < c && p > 0.6 * c, "{p} vs {c}");
        assert_eq!(polygon_half_width(5, 10.0, -11.0), None);
    }

    #[test]
    fn brushes_paint_focus_and_blur() {
        let (w, h) = (120, 60);
        let mut s = left_near(100.0);
        // focus brush over the far right, blur brush over the near left
        let stroke = |x: f64| lightcraft_develop::BrushStroke {
            points: vec![lightcraft_geom::Point::new(x, 0.2), lightcraft_geom::Point::new(x, 0.8)],
            size: 0.08,
            feather: 0.0,
            ..Default::default()
        };
        s.lens_blur.focus_strokes = vec![stroke(0.85)];
        s.lens_blur.blur_strokes = vec![stroke(0.15)];
        let frame = Frame::new(w, h, &s, true);
        let img = stripes(w, h);
        let coc = coc_plane(&s, &frame, w, h, &img);
        let at = |fx: f64| coc[(h / 2) * w + (fx * w as f64) as usize];
        assert!(at(0.85) < 0.2, "focus brush: {}", at(0.85));
        assert!(at(0.15) > 0.8, "blur brush: {}", at(0.15));
        assert!(at(0.6) > 0.8 && at(0.35) < 0.2);
    }
}
