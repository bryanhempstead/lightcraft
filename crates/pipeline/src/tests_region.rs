//! Region renders ([`crate::region`]): a region equals the same pixels of the whole render.

use std::sync::Arc;

use lightcraft_develop::{BrushStroke, DevelopSettings, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, Spot, SpotMode};
use lightcraft_geom::{Orientation, Point, Rect};
use lightcraft_raster::{Rgb32f, Rgba8};

use crate::region::{Region, RegionPx};
use crate::{Globals, RenderRequest, SourceInfo, StageCache, globals, render, render_region};

const W: usize = 1600;
const H: usize = 1067;

fn scene() -> Arc<Rgb32f> {
    Arc::new(lightcraft_scenes::demo_library()[0].render(W, H))
}

/// (max, mean) |Δ| over the channels of `tile` against the same pixels of `full`.
fn diff(full: &Rgba8, tile: &Rgba8, px: &RegionPx) -> (i32, f64) {
    assert_eq!((tile.width, tile.height), (px.w, px.h), "the region's size");
    assert_eq!((full.width, full.height), (px.full_w, px.full_h), "the whole render's size");
    let (mut worst, mut sum) = (0, 0i64);
    for y in 0..px.h {
        for x in 0..px.w {
            let a = full.data[(px.y + y) * full.width + px.x + x];
            let b = tile.data[y * tile.width + x];
            for c in 0..3 {
                let d = (a[c] as i32 - b[c] as i32).abs();
                worst = worst.max(d);
                sum += d as i64;
            }
        }
    }
    (worst, sum as f64 / (px.w * px.h * 3) as f64)
}

/// Render `region` of `s` alone (globals from `ctx`, a whole render's request) and the whole
/// render; their difference over the region.
fn compare(src: &Arc<Rgb32f>, s: &DevelopSettings, region: Region, ctx: &RenderRequest) -> (i32, f64, RegionPx) {
    let info = SourceInfo::default();
    let req = RenderRequest::fit(W, H);
    let full = render(src, &info, s, &req).image;
    // (what the loupe does: the whole-photo values of a preview-size pass, the scene map at size)
    let g = crate::region::globals_for(src, &info, src, &info, s, &req, ctx);
    let tile = render_region(src, &info, s, &RenderRequest { region: Some(region), ..req }, &g, None);
    let px = tile.region.expect("a region render says where it goes");
    let (worst, mean) = diff(&full, &tile.image, &px);
    (worst, mean, px)
}

fn typical_edits() -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.light.exposure = 0.4;
    s.light.highlights = -40.0;
    s.light.shadows = 30.0;
    s.light.contrast = 12.0;
    s.effects.clarity = 15.0;
    s.effects.texture = 10.0;
    s.effects.dehaze = 12.0;
    s.detail.nr_luminance = 30.0;
    s.detail.nr_color = 25.0;
    s.detail.sharpen_amount = 40.0;
    s.vignette.amount = -30.0;
    s.grain.amount = 25.0;
    s.color.vibrance = 20.0;
    s
}

const TILE: Region = Region { x0: 0.55, y0: 0.4, x1: 0.71, y1: 0.64 };

#[test]
fn a_region_equals_the_crop_of_the_whole_render() {
    let src = scene();
    for (name, s) in [("default", DevelopSettings::default()), ("typical edits", typical_edits())] {
        // globals from a whole render at the same size: only the windowing differs
        let (worst, mean, px) = compare(&src, &s, TILE, &RenderRequest::fit(W, H));
        assert!(worst <= 2 && mean < 0.05, "{name}: max {worst}, mean {mean:.3} at {px:?}");
    }
}

#[test]
fn a_region_with_preview_size_globals_stays_within_a_few_levels() {
    // what the loupe does: the whole-photo values from a preview-size pass
    let (worst, mean, _) = compare(&scene(), &typical_edits(), TILE, &RenderRequest::fit(W / 2, H / 2));
    eprintln!("preview-size globals: max {worst}, mean {mean:.3}");
    assert!(worst <= 3 && mean < 0.1, "max {worst}, mean {mean:.3}");
}

#[test]
fn regions_at_the_edges_and_corners_match_too() {
    let src = scene();
    let s = typical_edits();
    for r in [
        Region { x0: 0.0, y0: 0.0, x1: 0.2, y1: 0.25 },
        Region { x0: 0.8, y0: 0.75, x1: 1.0, y1: 1.0 },
        Region { x0: 0.3, y0: 0.9, x1: 0.5, y1: 1.0 },
    ] {
        let (worst, mean, px) = compare(&src, &s, r, &RenderRequest::fit(W, H));
        assert!(worst <= 2 && mean < 0.05, "{r:?}: max {worst}, mean {mean:.3} at {px:?}");
    }
}

fn mask(id: u32, shape: MaskShape, adjust: LocalAdjustments) -> Mask {
    Mask { id, components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape }], adjust, ..Default::default() }
}

#[test]
fn local_adjustments_line_up_between_a_region_and_the_whole_render() {
    let src = scene();
    let mut s = typical_edits();
    // a gradient across the region, a radial whose edge crosses it, and a brush stroke through it
    s.masks.push(mask(
        1,
        MaskShape::Linear { start: Point::new(0.58, 0.5), end: Point::new(0.66, 0.5) },
        LocalAdjustments { exposure: 1.2, ..Default::default() },
    ));
    s.masks.push(mask(
        2,
        MaskShape::Radial { center: Point::new(0.6, 0.45), rx: 0.08, ry: 0.05, angle: 20.0, feather: 30.0, invert: false },
        LocalAdjustments { saturation: -80.0, contrast: 40.0, ..Default::default() },
    ));
    let points = (0..40).map(|i| Point::new(0.56 + i as f64 * 0.004, 0.55)).collect();
    let stroke = BrushStroke { points, size: 0.01, feather: 50.0, flow: 100.0, density: 100.0, erase: false, auto_mask: false };
    s.masks.push(mask(3, MaskShape::Brush { strokes: vec![stroke] }, LocalAdjustments { temp: 60.0, ..Default::default() }));
    let (worst, mean, _) = compare(&src, &s, TILE, &RenderRequest::fit(W, H));
    assert!(worst <= 2 && mean < 0.05, "max {worst}, mean {mean:.3}");
    // the gradient really falls inside the region: full effect on its left, none on its right
    let info = SourceInfo::default();
    let req = RenderRequest { region: Some(TILE), ..RenderRequest::fit(W, H) };
    let tile = |s: &DevelopSettings| render_region(&src, &info, s, &req, &globals(&src, &info, s, &RenderRequest::fit(W, H)), None).image;
    let (with, without) = (tile(&s), tile(&DevelopSettings { masks: Vec::new(), ..s.clone() }));
    let lift = |x: usize| {
        (0..with.height).map(|y| with.data[y * with.width + x][1] as f64 - without.data[y * with.width + x][1] as f64).sum::<f64>()
            / with.height as f64
    };
    assert!(lift(1) > 20.0 && lift(with.width - 2).abs() < 3.0, "the gradient's edge inside the region: {} / {}", lift(1), lift(with.width - 2));
}

#[test]
fn geometry_lens_corrections_and_spots_line_up() {
    let src = scene();
    let mut s = typical_edits();
    s.orientation = Orientation::Rotate90;
    s.crop.geometry.rect = Rect::new(0.05, 0.08, 0.93, 0.9);
    s.crop.geometry.angle = 3.5;
    s.crop.flip_h = true;
    s.optics.distortion = 25.0;
    s.optics.vignetting = 30.0;
    // a heal spot in the region whose source lies far outside it, and a clone near its edge
    s.spots.push(Spot {
        mode: SpotMode::Heal,
        points: vec![Point::new(0.62, 0.5)],
        size: 0.02,
        source_offset: Some(Point::new(-0.4, 0.3)),
        ..Default::default()
    });
    s.spots.push(Spot {
        mode: SpotMode::Clone,
        points: vec![Point::new(0.5, 0.35)],
        size: 0.03,
        source_offset: Some(Point::new(0.2, -0.2)),
        ..Default::default()
    });
    let (worst, mean, px) = compare(&src, &s, TILE, &RenderRequest::fit(W, H));
    assert!(worst <= 2 && mean < 0.05, "max {worst}, mean {mean:.3} at {px:?}");
}

#[test]
fn an_auto_sourced_spot_near_the_region_renders_the_whole_frame() {
    let src = scene();
    let mut s = DevelopSettings::default();
    s.spots.push(Spot { mode: SpotMode::Remove, points: vec![Point::new(0.62, 0.5)], size: 0.02, source_offset: None, ..Default::default() });
    let info = SourceInfo::default();
    let req = RenderRequest::fit(W, H);
    let plan = crate::plan(&src, &info, &s, &RenderRequest { region: Some(TILE), ..req });
    assert!(plan.frame.window.is_none() && (plan.w, plan.h) == (W, H), "the whole frame, cut afterwards");
    let (worst, _, _) = compare(&src, &s, TILE, &req);
    assert_eq!(worst, 0);
}

#[test]
fn the_window_is_aligned_padded_and_inside_the_render() {
    let src = scene();
    let s = typical_edits();
    let plan = crate::plan(&src, &SourceInfo::default(), &s, &RenderRequest { region: Some(TILE), ..RenderRequest::fit(W, H) });
    let win = plan.frame.window.expect("a window");
    let px = plan.region.expect("the region");
    assert_eq!((win.x % crate::region::ALIGN, win.y % crate::region::ALIGN), (0, 0));
    assert!(win.x <= px.x && win.y <= px.y && win.x + plan.w >= px.x + px.w && win.y + plan.h >= px.y + px.h);
    assert!(win.x + plan.w <= W && win.y + plan.h <= H);
    let reach = crate::region::reach(&s, plan.px_per_long, crate::Quality::Full, W, W);
    assert!(px.x - win.x >= reach as usize || win.x == 0, "padded by the filters' reach");
    // the region's own size, the whole render's scale
    assert_eq!((px.full_w, px.full_h), (W, H));
    assert!((plan.px_per_long - W as f64).abs() < 1e-6);
}

#[test]
fn region_pixels_round_and_reject_nonsense() {
    let px = Region { x0: 0.25, y0: 0.5, x1: 0.5, y1: 1.0 }.pixels(400, 200).unwrap();
    assert_eq!(px, RegionPx { x: 100, y: 100, w: 100, h: 100, full_w: 400, full_h: 200 });
    assert_eq!(px.norm(), Region { x0: 0.25, y0: 0.5, x1: 0.5, y1: 1.0 });
    assert!(Region { x0: 0.5, y0: 0.0, x1: 0.5, y1: 1.0 }.pixels(400, 200).is_none());
    assert!(Region { x0: f64::NAN, y0: 0.0, x1: 1.0, y1: 1.0 }.pixels(400, 200).is_none());
    assert!(Region { x0: 2.0, y0: 2.0, x1: 3.0, y1: 3.0 }.pixels(400, 200).is_none());
    // clamped to the render
    let all = Region { x0: -1.0, y0: -1.0, x1: 9.0, y1: 9.0 }.pixels(400, 200).unwrap();
    assert_eq!((all.x, all.y, all.w, all.h), (0, 0, 400, 200));
}

#[test]
fn a_cached_region_render_equals_a_fresh_one_through_a_slider_drag() {
    let src = scene();
    let info = SourceInfo::default();
    let mut s = typical_edits();
    let req = RenderRequest { region: Some(TILE), ..RenderRequest::fit(W, H) };
    let cache = StageCache::default();
    let g: Globals = globals(&src, &info, &s, &RenderRequest::fit(W, H));
    for ev in [0.0, 0.3, 0.6] {
        s.light.exposure = ev;
        let warm = render_region(&src, &info, &s, &req, &g, Some(&cache)).image;
        let fresh = render_region(&src, &info, &s, &req, &g, None).image;
        assert_eq!(warm.data, fresh.data, "exposure {ev}");
    }
    // the globals follow the scene-linear image, not the sliders of the per-pixel stage
    let mut t = s.clone();
    t.light.exposure += 1.0;
    t.color.saturation = 30.0;
    assert_eq!(crate::region::globals_key(&s), crate::region::globals_key(&t));
    t.detail.nr_luminance += 20.0;
    assert_ne!(crate::region::globals_key(&s), crate::region::globals_key(&t));
}
