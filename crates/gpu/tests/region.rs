//! Region renders on the GPU (`lightcraft_gpu::render_region`, the loupe's 1:1 tiles): the same
//! pixels as the GPU's whole render and as the CPU's region render, within the CPU ↔ GPU bounds
//! of `equivalence.rs` (mean |Δ| < 0.5, max ≤ 3 LSB). Skips when no GPU adapter exists.

use std::sync::Arc;

use lightcraft_develop::{DevelopSettings, LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape, Spot, SpotMode};
use lightcraft_geom::{Point, Rect};
use lightcraft_pipeline::{Region, RenderRequest, SourceInfo, StageCache};
use lightcraft_raster::{Rgb32f, Rgba8};

const W: usize = 1600;
const H: usize = 1067;

fn scene() -> Arc<Rgb32f> {
    Arc::new(lightcraft_scenes::demo_library()[0].render(W, H))
}

/// (max, mean) |Δ| between `tile` and the pixels of `full` at `px`.
fn diff(full: &Rgba8, tile: &Rgba8, px: &lightcraft_pipeline::RegionPx) -> (u8, f64) {
    assert_eq!((tile.width, tile.height), (px.w, px.h));
    let (mut max, mut sum) = (0u8, 0u64);
    for y in 0..px.h {
        for x in 0..px.w {
            let a = full.data[(px.y + y) * full.width + px.x + x];
            let b = tile.data[y * tile.width + x];
            for c in 0..3 {
                let d = a[c].abs_diff(b[c]);
                max = max.max(d);
                sum += d as u64;
            }
        }
    }
    (max, sum as f64 / (px.w * px.h * 3) as f64)
}

fn edits() -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.light.exposure = 0.3;
    s.light.highlights = -40.0;
    s.light.shadows = 30.0;
    s.effects.clarity = 15.0;
    s.effects.texture = 10.0;
    s.effects.dehaze = 12.0;
    s.detail.nr_luminance = 30.0;
    s.detail.nr_color = 25.0;
    s.detail.sharpen_amount = 40.0;
    s.vignette.amount = -35.0;
    s.grain.amount = 30.0;
    s.crop.geometry.rect = Rect::new(0.04, 0.06, 0.95, 0.92);
    s.crop.geometry.angle = -2.0;
    s.optics.distortion = 20.0;
    s.masks.push(Mask {
        id: 1,
        components: vec![MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Radial { center: Point::new(0.62, 0.5), rx: 0.07, ry: 0.05, angle: 15.0, feather: 40.0, invert: false },
        }],
        adjust: LocalAdjustments { exposure: 0.8, saturation: -50.0, ..Default::default() },
        ..Default::default()
    });
    s.spots.push(Spot {
        mode: SpotMode::Heal,
        points: vec![Point::new(0.6, 0.52)],
        size: 0.015,
        source_offset: Some(Point::new(-0.3, 0.2)),
        ..Default::default()
    });
    s
}

#[test]
fn a_gpu_region_equals_the_gpu_and_cpu_renders_of_the_same_pixels() {
    if !lightcraft_gpu::available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let src = scene();
    let info = SourceInfo::default();
    let req = RenderRequest::fit(W, H);
    for (name, s) in [("default", DevelopSettings::default()), ("edits", edits())] {
        let full = lightcraft_gpu::render(&src, &info, &s, &req, None).expect("gpu render").image;
        let g = lightcraft_pipeline::region::globals_for(&src, &info, &src, &info, &s, &req, &req);
        for r in [
            Region { x0: 0.5, y0: 0.38, x1: 0.68, y1: 0.62 },
            Region { x0: 0.0, y0: 0.0, x1: 0.2, y1: 0.3 },
            Region { x0: 0.85, y0: 0.8, x1: 1.0, y1: 1.0 },
        ] {
            let rreq = RenderRequest { region: Some(r), ..req };
            let tile = lightcraft_gpu::render_region(&src, &info, &s, &rreq, &g, None).expect("gpu region render");
            let px = tile.region.expect("where the region goes");
            let (max, mean) = diff(&full, &tile.image, &px);
            eprintln!("{name} {r:?}: GPU region vs GPU whole: max {max}, mean {mean:.4}");
            assert!(max <= 3 && mean < 0.5, "{name} {r:?}: GPU region vs GPU whole: max {max}, mean {mean:.4}");
            let cpu = lightcraft_pipeline::render_region(&src, &info, &s, &rreq, &g, None).image;
            let whole = lightcraft_pipeline::RegionPx { x: 0, y: 0, ..px };
            let (max, mean) = diff(&cpu, &tile.image, &lightcraft_pipeline::RegionPx { full_w: px.w, full_h: px.h, ..whole });
            assert!(max <= 3 && mean < 0.5, "{name} {r:?}: GPU region vs CPU region: max {max}, mean {mean:.4}");
        }
    }
}

#[test]
fn a_cached_gpu_region_render_equals_a_fresh_one() {
    if !lightcraft_gpu::available() {
        return;
    }
    let src = scene();
    let info = SourceInfo::default();
    let mut s = edits();
    let req = RenderRequest { region: Some(Region { x0: 0.5, y0: 0.4, x1: 0.7, y1: 0.6 }), ..RenderRequest::fit(W, H) };
    let g = lightcraft_pipeline::region::globals_for(&src, &info, &src, &info, &s, &RenderRequest::fit(W, H), &RenderRequest::fit(800, 800));
    let cache = StageCache::default();
    for ev in [0.0, 0.4, 0.8] {
        s.light.exposure = ev;
        let warm = lightcraft_gpu::render_region(&src, &info, &s, &req, &g, Some(&cache)).expect("gpu").image;
        let fresh = lightcraft_gpu::render_region(&src, &info, &s, &req, &g, None).expect("gpu").image;
        assert_eq!(warm.data, fresh.data, "exposure {ev}");
    }
}
