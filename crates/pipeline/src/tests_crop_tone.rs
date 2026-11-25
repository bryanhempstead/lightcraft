//! A crop never changes the tone of the pixels inside it: Camera Raw processes the whole image
//! and crops afterwards, so the photo's statistics (tone key, airlight, the scene map's dark key,
//! Highlights / Shadows rule, haze) come from the whole uncropped photo, whatever the crop, the
//! straighten angle or the size of the render.

use std::sync::Arc;

use lightcraft_develop::DevelopSettings;
use lightcraft_geom::Rect;
use lightcraft_raster::{Rgb32f, Rgba8};

use crate::region::Region;
use crate::{Quality, RenderRequest, SourceInfo, StageCache, render, render_cached, render_region};

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
        // negative Dehaze's key is a percentile of the subsampled regional level: a crop's sample lattice moves it by a hair
        // where the scene is sparse near it (max ΔE 0.8 in one pixel, mean 0.000)
        let limit = if name == "negative dehaze" { 1.0 } else { 0.5 };
        assert!(worst < limit, "{name}: the crop changed the tone inside it: max ΔE {worst:.3} (mean {mean:.3}, L* {l_whole:.2} vs {l_crop:.2})");
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
    // and a cached render equals one given the same values
    let cached = render_cached(&src, &info, &t, &RenderRequest::fit(300, 200), &cache).image;
    let fresh = render_region(&src, &info, &t, &RenderRequest::fit(300, 200), &first, None).image;
    assert_eq!(cached.data, fresh.data);
}

/// The edits the geometry tests below render: local base, Clarity and haze, all read off the
/// photo's one map.
fn tone_edits() -> DevelopSettings {
    edits(|s| {
        s.light.highlights = -50.0;
        s.light.shadows = 40.0;
        s.effects.clarity = 35.0;
        s.effects.dehaze = 15.0;
    })
}

/// A flipped crop reads the whole photo's map where it is mirrored: its pixels are the uncropped
/// render's, mirrored (flip_h, flip_v, both).
#[test]
fn a_flipped_crop_does_not_change_the_tone_inside_it() {
    let src = scene();
    let info = SourceInfo::default();
    let s = tone_edits();
    let whole = render(&src, &info, &s, &RenderRequest { apply_crop: false, ..RenderRequest::fit(W, H) }).image;
    let (cw, ch) = (W / 2, H / 2);
    for (flip_h, flip_v) in [(true, false), (false, true), (true, true)] {
        let mut c = s.clone();
        c.crop.geometry.rect = Rect::new(0.0, 0.5, 0.5, 1.0);
        c.crop.flip_h = flip_h;
        c.crop.flip_v = flip_v;
        let inside = render(&src, &info, &c, &RenderRequest::fit(cw, ch)).image;
        // un-flip the crop and compare it with its place in the uncropped render
        let mut undone = inside.clone();
        for y in 0..ch {
            for x in 0..cw {
                let (sx, sy) = (if flip_h { cw - 1 - x } else { x }, if flip_v { ch - 1 - y } else { y });
                undone.data[y * cw + x] = inside.data[sy * cw + sx];
            }
        }
        let (mx, my) = (cw / 5, ch / 5);
        let (worst, mean, l_whole, l_crop) = compare(&undone, (mx, my, cw - 2 * mx, ch - 2 * my), &whole, (mx, my + H / 2));
        eprintln!("flip_h {flip_h} flip_v {flip_v}: max ΔE {worst:.3}, mean {mean:.3}, L* {l_whole:.3} / {l_crop:.3}");
        assert!(worst < 0.6, "flip_h {flip_h} flip_v {flip_v}: max ΔE {worst:.3}");
    }
}

/// A straightened crop and a smaller crop inside it (an integer number of pixels over, at the same
/// scale) have the same tone where they overlap, though one has the sky and the other doesn't.
#[test]
fn a_straightened_crop_does_not_change_the_tone_inside_it() {
    let src = scene();
    let info = SourceInfo::default();
    let mut big = tone_edits();
    big.crop.geometry.angle = 3.0;
    big.crop.geometry.rect = Rect::new(0.25, 0.3, 0.75, 0.9);
    // the lower part of it: 300 px down at a 1:1 scale (the crop is 600 × 480 px)
    let mut small = big.clone();
    small.crop.geometry.rect = Rect::new(0.25, 0.3 + 300.0 / H as f64, 0.75, 0.9);
    let (bw, bh) = (W / 2, (H as f64 * 0.6) as usize);
    let a = render(&src, &info, &big, &RenderRequest::fit(bw, bh)).image;
    let b = render(&src, &info, &small, &RenderRequest::fit(W / 2, bh - 300)).image;
    assert_eq!((a.width, a.height, b.width, b.height), (bw, bh, bw, bh - 300));
    let (mx, my) = (bw / 5, 80);
    let (worst, mean, la, lb) = compare(&b, (mx, my, bw - 2 * mx, bh - 300 - 2 * my), &a, (mx, 300 + my));
    eprintln!("straightened: max ΔE {worst:.3}, mean {mean:.3}, L* {la:.3} / {lb:.3}");
    assert!(worst < 0.6, "straightened: max ΔE {worst:.3} (mean {mean:.3})");
}

/// A region (a window of the cropped, flipped, straightened render) equals the same pixels of the
/// whole render of that crop.
#[test]
fn a_region_of_a_flipped_straightened_crop_equals_the_whole_render() {
    let src = scene();
    let info = SourceInfo::default();
    let mut s = tone_edits();
    s.crop.geometry.rect = Rect::new(0.15, 0.3, 0.9, 0.95);
    s.crop.geometry.angle = -2.5;
    s.crop.flip_v = true;
    s.crop.flip_h = true;
    let req = RenderRequest::fit(W, H);
    let whole = render(&src, &info, &s, &req).image;
    let g = crate::globals(&src, &info, &s, &req);
    let tile = render_region(&src, &info, &s, &RenderRequest { region: Some(Region { x0: 0.4, y0: 0.3, x1: 0.7, y1: 0.7 }), ..req }, &g, None);
    let px = tile.region.expect("a region render says where it goes");
    let (mut worst, mut sum) = (0i32, 0i64);
    for y in 0..px.h {
        for x in 0..px.w {
            let (p, q) = (whole.data[(px.y + y) * whole.width + px.x + x], tile.image.data[y * px.w + x]);
            for c in 0..3 {
                let d = (p[c] as i32 - q[c] as i32).abs();
                worst = worst.max(d);
                sum += d as i64;
            }
        }
    }
    let mean = sum as f64 / (px.w * px.h * 3) as f64;
    assert!(worst <= 2 && mean < 0.05, "max {worst}, mean {mean:.3} at {px:?}");
}

/// A render that keeps nothing takes the statistics of its own frame when that is the whole photo
/// (no second pass for a thumbnail), and measures no more than its own size otherwise.
#[test]
fn renders_without_a_cache_measure_no_more_than_they_need() {
    let src = scene();
    let info = SourceInfo::default();
    let s = tone_edits();
    // the whole photo, small: its own statistics
    assert!(crate::analysis(&src, &info, &s, &RenderRequest::fit(256, 256), None).is_none());
    assert!(crate::analysis(&src, &info, &s, &RenderRequest { apply_crop: false, ..RenderRequest::fit(256, 256) }, None).is_none());
    // a crop: the whole photo's, at the output's size (not 1024)
    let mut cropped = s.clone();
    cropped.crop.geometry.rect = Rect::new(0.0, 0.5, 0.5, 1.0);
    assert!(crate::analysis(&src, &info, &cropped, &RenderRequest::fit(256, 256), None).is_some());
    // a view that keeps them: the whole photo, small, is the same with or without a cache; a crop has them
    let cache = StageCache::default();
    assert!(crate::analysis(&src, &info, &s, &RenderRequest::fit(256, 256), Some(&cache)).is_none());
    assert!(crate::analysis(&src, &info, &cropped, &RenderRequest::fit(256, 256), Some(&cache)).is_some());
    // nothing to measure: a JPEG with no tone edits, an Adobe raw with none
    assert!(!crate::needs_globals(&DevelopSettings::default(), false));
    assert!(crate::needs_globals(&DevelopSettings::default(), true), "an Adobe base reads the photo's dark key");
    assert!(crate::analysis(&src, &info, &DevelopSettings::default(), &RenderRequest::fit(256, 256), Some(&cache)).is_none());
}

/// Generated pixels that arrive after the photo was measured replace the stand-in fill the
/// measurement saw: the cached statistics are measured again.
#[test]
fn generated_pixels_arriving_later_measure_the_photo_again() {
    let src = scene();
    let info = SourceInfo::default();
    let mut s = tone_edits();
    let spot = lightcraft_develop::Spot {
        mode: lightcraft_develop::SpotMode::Fill,
        points: vec![lightcraft_geom::Point::new(0.8, 0.1)],
        size: 0.1,
        feather: 0.0,
        generative: Some(lightcraft_develop::Generative { variation: 0, count: 1, key: "00000000000000cd".into(), quick: false }),
        ..Default::default()
    };
    s.spots.push(spot.clone());
    s.crop.geometry.rect = Rect::new(0.0, 0.0, 0.8, 1.0);
    let cache = StageCache::default();
    let req = RenderRequest::fit(300, 200);
    let before = crate::analysis(&src, &info, &s, &req, Some(&cache)).expect("tone settings read statistics");
    assert!(Arc::ptr_eq(&before, &crate::analysis(&src, &info, &s, &req, Some(&cache)).expect("kept")), "kept while nothing changes");
    // the pixels arrive (the settings are the same)
    let image = Rgb32f::from_fn(40, 40, |_, _| [0.01; 3]);
    crate::spots::test_fill("00000000000000cd-0", crate::spots::GenFill { geometry: crate::spots::gen_geometry(&spot), rect: [0.7, 0.0, 0.9, 0.2], image });
    let after = crate::analysis(&src, &info, &s, &req, Some(&cache)).expect("tone settings read statistics");
    assert!(!Arc::ptr_eq(&before, &after), "measured again");
}

/// A render of the whole uncropped photo above the analysis size measures it at a smaller size
/// ([`crate::ANALYSIS_EDGE`]): on an Adobe base too (whose default black reads the scene map's dark
/// key) it must look as if it measured its own frame, since a crop measuring the whole photo must
/// not make the uncropped render drift from what it was.
#[test]
fn an_uncropped_render_matches_one_measuring_its_own_frame() {
    use crate::adobe;
    let src = scene();
    let curve: Vec<f32> = (0..=adobe::CURVE_N).map(|i| (i as f32 / adobe::CURVE_N as f32).powf(0.45)).collect();
    assert!(adobe::register_base(0xc20b_0001, adobe::Base { hue_sat: None, hue_sat_ev: 0.0, look: None, curve, black: 0.005 }));
    let info = SourceInfo { raw: true, adobe: Some(0xc20b_0001), as_shot_temp: 5000.0, ..Default::default() };
    assert!(W.max(H) > crate::ANALYSIS_EDGE, "the render is bigger than the analysis frame");
    let mut variants = variants();
    variants.insert(0, ("base", DevelopSettings::default()));
    for (name, s) in variants {
        let req = RenderRequest { apply_crop: false, ..RenderRequest::fit(W, H) };
        let analysed = crate::analysis(&src, &info, &s, &req, None).expect("an Adobe base reads the photo's dark key");
        let own = crate::globals_at(&src, &info, &s, &req, W.max(H));
        let (a, b) = (analysed.scene_map.as_ref().expect("map"), own.scene_map.as_ref().expect("map"));
        eprintln!("{name}: dark key {:.3} (analysis) vs {:.3} (own frame)", a.dark, b.dark);
        assert!((a.dark - b.dark).abs() < 0.25, "{name}: dark key {:.3} vs {:.3}", a.dark, b.dark);
        let new = render(&src, &info, &s, &req).image;
        let old = render_region(&src, &info, &s, &req, &own, None).image;
        let (worst, mean, l_new, l_old) = compare(&old, (0, 0, W, H), &new, (0, 0));
        eprintln!("{name}: max ΔE {worst:.3}, mean ΔE {mean:.3}, mean L* {l_new:.3} vs {l_old:.3}");
        assert!(mean < 0.3 && (l_new - l_old).abs() < 0.1, "{name}: mean ΔE {mean:.3}, L* {l_new:.2} vs {l_old:.2}");
    }
}
