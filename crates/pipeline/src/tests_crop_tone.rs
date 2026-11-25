//! A crop never changes the tone of the pixels inside it: Camera Raw processes the whole image
//! and crops afterwards, so the photo's statistics (tone key, airlight, the scene map's dark key,
//! Highlights / Shadows rule, haze) come from the whole uncropped photo, whatever the crop, the
//! straighten angle or the size of the render.

use std::sync::Arc;

use lightcraft_develop::DevelopSettings;
use lightcraft_geom::Rect;
use lightcraft_raster::{Rgb32f, Rgba8};

use crate::{Quality, RenderRequest, SourceInfo, StageCache, render, render_cached};

const W: usize = 1200;
const H: usize = 800;

/// A raw-like linear scene: dark textured ground under the horizon (45 % down), a bright sky above
/// it with a blazing sun glow in the top-right corner. Its statistics (key, 99th percentile,
/// airlight, the darkest 0.1 %) are dominated by the sky, so a crop of the ground changes them
/// when they're measured on the crop.
fn scene() -> Arc<Rgb32f> {
    let noise = |x: usize, y: usize| {
        let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
    };
    Arc::new(Rgb32f::from_fn(W, H, |x, y| {
        let (u, v) = (x as f32 / W as f32, y as f32 / H as f32);
        let horizon = 0.45;
        if v < horizon {
            // sky: brighter toward the horizon and the top-right corner
            let glow = (-(((1.0 - u).powi(2) + v.powi(2)) / 0.05)).exp() * 6.0;
            let base = 0.9 + 1.2 * (v / horizon);
            let l = base + glow;
            [l * 0.85, l * 0.95, l]
        } else {
            // ground: dark, with texture and a slow gradient, a few deep shadows
            let t = (v - horizon) / (1.0 - horizon);
            let tex = 0.6 + 0.8 * noise(x / 3, y / 3);
            let l = (0.012 + 0.10 * (1.0 - t) * (0.5 + u)) * tex;
            [l * 1.1, l, l * 0.8]
        }
    }))
}

/// CIE L* of an sRGB-encoded pixel.
fn lstar(p: [u8; 4]) -> f64 {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let y = 0.2126 * lin(p[0]) + 0.7152 * lin(p[1]) + 0.0722 * lin(p[2]);
    let f = if y > 216.0 / 24389.0 { y.cbrt() } else { (24389.0 / 27.0 * y + 16.0) / 116.0 };
    116.0 * f - 16.0
}

/// CIE L*a*b* of an sRGB-encoded pixel (D65).
fn lab(p: [u8; 4]) -> [f64; 3] {
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
    let xyz = [
        (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / 0.95047,
        0.2126729 * r + 0.7151522 * g + 0.0721750 * b,
        (0.0193339 * r + 0.1191920 * g + 0.9503041 * b) / 1.08883,
    ];
    let f = |t: f64| if t > 216.0 / 24389.0 { t.cbrt() } else { (24389.0 / 27.0 * t + 16.0) / 116.0 };
    let (fx, fy, fz) = (f(xyz[0]), f(xyz[1]), f(xyz[2]));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// (max, mean) ΔE76 between `a` and `b`, and each one's mean L*, over the rectangle `(x, y, w, h)`
/// of `a` placed at `(bx, by)` in `b`.
fn compare(a: &Rgba8, (x0, y0, w, h): (usize, usize, usize, usize), b: &Rgba8, (bx, by): (usize, usize)) -> (f64, f64, f64, f64) {
    let (mut worst, mut sum, mut la, mut lb) = (0f64, 0f64, 0f64, 0f64);
    for y in 0..h {
        for x in 0..w {
            let p = a.data[(y0 + y) * a.width + x0 + x];
            let q = b.data[(by + y) * b.width + bx + x];
            let (u, v) = (lab(p), lab(q));
            let d = ((u[0] - v[0]).powi(2) + (u[1] - v[1]).powi(2) + (u[2] - v[2]).powi(2)).sqrt();
            worst = worst.max(d);
            sum += d;
            la += u[0];
            lb += v[0];
        }
    }
    let n = (w * h) as f64;
    (worst, sum / n, la / n, lb / n)
}

/// The edits Bryan's Lightroom photos carry: tone, local contrast, haze.
fn edits(f: impl Fn(&mut DevelopSettings)) -> DevelopSettings {
    let mut s = DevelopSettings::default();
    f(&mut s);
    s
}

fn variants() -> Vec<(&'static str, DevelopSettings)> {
    vec![
        ("highlights / shadows", edits(|s| {
            s.light.highlights = -60.0;
            s.light.shadows = 50.0;
            s.light.exposure = 0.3;
        })),
        ("clarity", edits(|s| s.effects.clarity = 40.0)),
        ("positive dehaze", edits(|s| s.effects.dehaze = 30.0)),
        ("negative dehaze", edits(|s| s.effects.dehaze = -40.0)),
        ("all together", edits(|s| {
            s.light.highlights = -40.0;
            s.light.shadows = 30.0;
            s.light.contrast = 12.0;
            s.effects.clarity = 15.0;
            s.effects.dehaze = 12.0;
        })),
    ]
}

/// The same photo rendered uncropped and cropped to a quarter that leaves the sky out: the pixels
/// inside the crop match (apart from where the local filters read past the crop's edge).
#[test]
fn a_crop_does_not_change_the_tone_inside_it() {
    let src = scene();
    let info = SourceInfo::default();
    // the bottom-left quarter of the picture (ground only), at the full render's scale
    let crop = Rect::new(0.0, 0.5, 0.5, 1.0);
    let (cw, ch) = (W / 2, H / 2);
    // away from the crop's edges, where the guided filters and the clarity map see other pixels
    let margin = 0.2;
    let (mx, my) = ((cw as f64 * margin) as usize, (ch as f64 * margin) as usize);
    for (name, s) in variants() {
        let whole = render(&src, &info, &s, &RenderRequest { apply_crop: false, ..RenderRequest::fit(W, H) }).image;
        let mut cropped = s.clone();
        cropped.crop.geometry.rect = crop;
        let inside = render(&src, &info, &cropped, &RenderRequest::fit(cw, ch)).image;
        assert_eq!((inside.width, inside.height), (cw, ch), "{name}");
        // the crop's pixel (x, y) is the whole render's (x, y + H / 2)
        let (worst, mean, l_whole, l_crop) = compare(&inside, (mx, my, cw - 2 * mx, ch - 2 * my), &whole, (mx, my + H / 2));
        eprintln!("{name}: max ΔE {worst:.3}, mean ΔE {mean:.3}, mean L* uncropped {l_whole:.3} cropped {l_crop:.3}");
        assert!(worst < 0.5, "{name}: the crop changed the tone inside it: max ΔE {worst:.3} (mean {mean:.3}, L* {l_whole:.2} vs {l_crop:.2})");
    }
}

/// While a crop is dragged the loupe shows a low-resolution draft of the uncropped frame, which
/// must have the tone the cropped result settles on (it used to jump when the crop settled).
#[test]
fn a_draft_of_the_whole_frame_has_the_tone_of_the_cropped_render() {
    let src = scene();
    let info = SourceInfo::default();
    let s = edits(|s| {
        s.light.highlights = -60.0;
        s.light.shadows = 50.0;
        s.effects.clarity = 20.0;
        s.effects.dehaze = 15.0;
        s.crop.geometry.rect = Rect::new(0.0, 0.5, 0.5, 1.0);
    });
    let (cw, ch) = (W / 2, H / 2);
    let settled = render(&src, &info, &s, &RenderRequest::fit(cw, ch)).image;
    // the draft: uncropped, at 0.35 of the full scale, draft quality
    let (dw, dh) = ((W as f64 * 0.35) as usize, (H as f64 * 0.35) as usize);
    let draft = render(&src, &info, &s, &RenderRequest { apply_crop: false, quality: Quality::Draft, ..RenderRequest::fit(dw, dh) }).image;
    // the settled crop's mean L* against the same part of the draft
    let (x0, y0, w, h) = (0, dh / 2, dw / 2, dh - dh / 2);
    let mean = |im: &Rgba8, (x0, y0, w, h): (usize, usize, usize, usize)| {
        let mut sum = 0.0;
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                sum += lstar(im.data[y * im.width + x]);
            }
        }
        sum / (w * h) as f64
    };
    let (l_draft, l_settled) = (mean(&draft, (x0, y0, w, h)), mean(&settled, (0, 0, cw, ch)));
    eprintln!("draft mean L* {l_draft:.3}, settled crop {l_settled:.3}");
    assert!((l_draft - l_settled).abs() < 0.6, "the draft's tone {l_draft:.2} jumps to {l_settled:.2} when the crop settles");
}

/// A draft-scale render and a full-scale render of the same settings have the same mean tone.
#[test]
fn a_draft_scale_render_has_the_tone_of_the_full_render() {
    let src = scene();
    let info = SourceInfo::default();
    for (name, s) in variants() {
        let mean = |im: &Rgba8| im.data.iter().map(|p| lstar(*p)).sum::<f64>() / im.data.len() as f64;
        let full = render(&src, &info, &s, &RenderRequest::fit(W, H)).image;
        let draft = render(&src, &info, &s, &RenderRequest { quality: Quality::Draft, ..RenderRequest::fit(W * 35 / 100, H * 35 / 100) }).image;
        let (lf, ld) = (mean(&full), mean(&draft));
        eprintln!("{name}: mean L* full {lf:.3} draft {ld:.3}");
        assert!((lf - ld).abs() < 0.3, "{name}: full {lf:.2} vs draft {ld:.2}");
    }
}

/// Dragging the crop, straightening or flipping measures the photo once: the same numbers whatever
/// the frame (and the cache keeps them while the crop moves).
#[test]
fn the_photo_is_measured_whole_whatever_the_frame() {
    let src = scene();
    let info = SourceInfo::default();
    let s = edits(|s| {
        s.light.highlights = -50.0;
        s.effects.dehaze = -20.0;
    });
    let req = RenderRequest::fit(W, H);
    let a = crate::globals(&src, &info, &s, &req);
    let mut t = s.clone();
    t.crop.geometry.rect = Rect::new(0.0, 0.5, 0.5, 1.0);
    t.crop.geometry.angle = 4.0;
    t.crop.flip_h = true;
    let b = crate::globals(&src, &info, &t, &RenderRequest::fit(300, 200));
    assert_eq!(a.tone_stats, b.tone_stats);
    assert_eq!(a.p99, b.p99);
    assert_eq!(crate::region::globals_key(&s), crate::region::globals_key(&t), "a crop doesn't change the key");
    // cached: a crop drag doesn't measure again
    let cache = StageCache::default();
    let first = crate::analysis(&src, &info, &s, &req, Some(&cache)).expect("tone settings read statistics");
    let second = crate::analysis(&src, &info, &t, &RenderRequest::fit(300, 200), Some(&cache)).expect("tone settings read statistics");
    assert!(Arc::ptr_eq(&first, &second));
    // and a cached render equals an uncached one
    let cached = render_cached(&src, &info, &t, &RenderRequest::fit(300, 200), &cache).image;
    let fresh = render(&src, &info, &t, &RenderRequest::fit(300, 200)).image;
    assert_eq!(cached.data, fresh.data);
}
