//! Round 11 (Bryan's fork): Camera Raw's scene tone map (`lightcraft_pipeline::scene_cr`) renders on
//! the GPU as on the CPU — Clarity, positive Dehaze, Texture in camera RGB, and the Highlights /
//! Shadows rule (forced on: a process of its own, so the other equivalence tests keep the default).
//! Bounds as in `equivalence.rs`: mean |Δ| < 0.5 LSB, max |Δ| ≤ 3 LSB. Skips without a GPU adapter.

use std::sync::Arc;

use lightcraft_develop::{DevelopSettings, Mask, MaskComponent, MaskOp, MaskShape};
use lightcraft_geom::Point;
use lightcraft_pipeline::{RenderRequest, SourceInfo, render};
use lightcraft_raster::Rgb32f;

fn check(name: &str, src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings) {
    check_max(name, src, info, s, 3)
}

/// Positive Dehaze subtracts the airlight and divides by the transmission: a channel it takes to
/// almost 0 (deep blues' red) keeps the last bits of the photo's airlight, which the GPU measures
/// on its own (float-identical only to rounding) image, and the steep start of the tone and sRGB
/// curves turns those into a few LSB there. Such pixels may differ by 5 LSB; the mean bound holds.
const DEHAZE_MAX_LSB: u8 = 5;

fn check_max(name: &str, src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings, max_lsb: u8) {
    let req = RenderRequest::fit(320, 240);
    let cpu = render(src, info, s, &req).image;
    let gpu = lightcraft_gpu::render(src, info, s, &req, None).expect("gpu render").image;
    let (mut sum, mut max) = (0u64, 0u8);
    for (p, q) in cpu.data.iter().zip(&gpu.data) {
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            sum += d as u64;
            max = max.max(d);
        }
    }
    let mean = sum as f64 / (cpu.data.len() * 3) as f64;
    eprintln!("{name:<28} mean {mean:.4}  max {max}");
    assert!(mean < 0.5 && max <= max_lsb, "{name}: mean {mean:.4} LSB, max {max} LSB");
}

#[test]
fn scene_tone_map_matches_cpu() {
    if !lightcraft_gpu::available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    lightcraft_pipeline::scene_cr::force_hs_measured(true);
    use lightcraft_pipeline::adobe;
    let src = Arc::new(lightcraft_scenes::demo_library()[2].render(320, 240));
    let curve: Vec<f32> = (0..=adobe::CURVE_N).map(|i| (i as f32 / adobe::CURVE_N as f32).powf(0.45)).collect();
    assert!(adobe::register_base(0x5ce4e, adobe::Base { hue_sat: None, hue_sat_ev: 0.0, look: None, curve, black: 0.005 }));
    let m = [[0.9, 0.08, 0.02], [0.05, 0.9, 0.05], [0.01, 0.1, 0.89]];
    let inv = [[1.1162, -0.0952, -0.0198], [-0.0603, 1.1300, -0.0621], [-0.0058, -0.1259, 1.1325]];
    let camera_wb = lightcraft_pipeline::CameraWb { r: [0.0; 5], b: [0.0; 5], to_working: m, from_working: inv, shot: [1.0; 3], exact: None };
    let info = SourceInfo { raw: true, adobe: Some(0x5ce4e), camera_wb: Some(camera_wb), as_shot_temp: 5000.0, ..Default::default() };
    let mut s = DevelopSettings::default();
    s.effects.clarity = 60.0;
    check("clarity +60", &src, &info, &s);
    s.effects.clarity = -40.0;
    check("clarity -40 (band)", &src, &info, &s);
    s.effects.clarity = 0.0;
    s.effects.dehaze = 70.0;
    check_max("dehaze +70", &src, &info, &s, DEHAZE_MAX_LSB);
    // negative Dehaze: the curve, the photo's key (shifted with it), the detail scale and the airlight's colour
    s.effects.dehaze = -100.0;
    check_max("dehaze -100", &src, &info, &s, DEHAZE_MAX_LSB);
    s.effects.dehaze = -45.0;
    check_max("dehaze -45", &src, &info, &s, DEHAZE_MAX_LSB);
    s.effects.dehaze = 0.0;
    s.effects.texture = 50.0;
    check("texture in camera rgb", &src, &info, &s);
    s.effects.texture = 0.0;
    s.light.highlights = -80.0;
    check("highlights rule", &src, &info, &s);
    s.light.shadows = 60.0;
    s.light.exposure = -0.6;
    check("highlights + shadows rule", &src, &info, &s);
    // masks: local Clarity, Dehaze and Shadows on top of the sliders
    s.effects.clarity = 20.0;
    s.effects.dehaze = 15.0;
    s.masks = vec![Mask {
        components: vec![MaskComponent {
            name: None,
            op: MaskOp::Add,
            invert: false,
            shape: MaskShape::Linear { start: Point::new(0.5, 0.0), end: Point::new(0.5, 0.9) },
        }],
        adjust: lightcraft_develop::LocalAdjustments { clarity: 40.0, dehaze: 30.0, shadows: -50.0, highlights: 30.0, ..Default::default() },
        ..Default::default()
    }];
    check_max("masks on the scene map", &src, &info, &s, DEHAZE_MAX_LSB);
    // round 12: Camera Raw's default black at the photo's dark key, moved by Shadows and faded by
    // positive Blacks (`black_cr`), on a darker photo
    let dark = Arc::new(Rgb32f { width: src.width, height: src.height, data: src.data.iter().map(|p| p.map(|v| v * 0.02)).collect() });
    let mut s = DevelopSettings::default();
    check("default black at the photo's key", &dark, &info, &s);
    s.light.shadows = 70.0;
    s.light.blacks = 10.0;
    check("default black after shadows", &dark, &info, &s);
}
