//! CPU ↔ GPU equivalence of the HDR render (`OutputDepth::F32Hdr` of a photo edited in HDR):
//! the linear float samples, compared in the extended sRGB encoding the HDR loupe shows (1.0 =
//! SDR white, so one "LSB" is 1/255 of the SDR range and the headroom above it is compared on
//! the same perceptual scale), plus the clipped 8-bit image. Also Visualize HDR, which now
//! renders both halves on the GPU. Skips (passes with a note) without a GPU adapter.

use std::sync::Arc;

use lightcraft_develop::{DevelopSettings, VignetteStyle};
use lightcraft_pipeline::{DeepSamples, OutputDepth, Overlay, RenderRequest, SourceInfo, render};
use lightcraft_raster::Rgb32f;

fn gpu() -> bool {
    let ok = lightcraft_gpu::available();
    if !ok {
        eprintln!("skipped: no GPU adapter");
    }
    ok
}

fn sunset(w: usize, h: usize) -> Arc<Rgb32f> {
    let lib = lightcraft_scenes::demo_library();
    let s = lib.iter().find(|s| s.kind == lightcraft_scenes::Kind::OceanSunset).map(|s| s.render(w, h));
    Arc::new(s.unwrap_or_else(|| lib[0].render(w, h)))
}

fn hdr_on(max_ev: f64) -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.hdr.enabled = true;
    s.hdr.max_ev = max_ev;
    s
}

fn enc(v: f32) -> f32 {
    let a = v.max(0.0);
    if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 }
}

fn floats(r: &lightcraft_pipeline::Rendered) -> &[f32] {
    match r.deep.as_ref().map(|d| &d.samples) {
        Some(DeepSamples::F32(v)) => v,
        other => panic!("expected float samples, got {other:?}"),
    }
}

/// (mean, max) |Δ| of the encoded samples, in 1/255 of SDR white.
fn check(name: &str, src: &Arc<Rgb32f>, info: &SourceInfo, s: &DevelopSettings, w: usize, h: usize) {
    let req = RenderRequest { depth: OutputDepth::F32Hdr, ..RenderRequest::fit(w, h) };
    let cpu = render(src, info, s, &req);
    let gpu = lightcraft_gpu::render(src, info, s, &req, None).expect("gpu HDR render");
    let (a, b) = (floats(&cpu), floats(&gpu));
    assert_eq!(a.len(), b.len(), "{name}: sample count");
    let (mut sum, mut max, mut peak) = (0f64, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        let d = (enc(*x) - enc(*y)).abs() * 255.0;
        sum += f64::from(d);
        max = max.max(d);
        peak = peak.max(*x);
    }
    let mean = sum / a.len() as f64;
    let (mut s8, mut m8) = (0u64, 0u8);
    for (p, q) in cpu.image.data.iter().zip(&gpu.image.data) {
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            s8 += u64::from(d);
            m8 = m8.max(d);
        }
    }
    let mean8 = s8 as f64 / (cpu.image.data.len() * 3) as f64;
    eprintln!("{name:<28} {w}x{h}  peak {peak:.2}  float mean {mean:.4} max {max:.2}  8-bit mean {mean8:.4} max {m8}");
    assert!(peak > 1.2, "{name}: the scene should reach above SDR white (peak {peak})");
    assert!(mean < 0.5 && max <= 4.0, "{name}: float mean {mean:.4}, max {max:.2} (1/255 of SDR white)");
    assert!(mean8 < 0.5 && m8 <= 3, "{name}: 8-bit mean {mean8:.4}, max {m8}");
}

#[test]
fn hdr_render_matches_the_cpu() {
    if !gpu() {
        return;
    }
    let src = sunset(360, 240);
    let raw = SourceInfo { raw: true, ..Default::default() };
    check("hdr default", &src, &raw, &hdr_on(3.0), 360, 240);
    check("hdr limit 1", &src, &raw, &hdr_on(1.0), 360, 240);
    let mut s = hdr_on(4.0);
    s.light.exposure = 0.7;
    s.light.highlights = -40.0;
    s.light.whites = 30.0;
    s.light.shadows = 20.0;
    s.effects.clarity = 20.0;
    s.color.vibrance = 25.0;
    check("hdr basic tone", &src, &raw, &s, 360, 240);
    // the look acts on the SDR part, the excess is carried over
    let mut c = hdr_on(3.0);
    c.curve.lights = 30.0;
    c.curve.darks = -20.0;
    c.grain.amount = 25.0;
    check("hdr curve + grain", &src, &raw, &c, 360, 240);
    // lightening vignette: towards SDR white, never pulling HDR highlights down
    let mut v = hdr_on(3.0);
    v.vignette.amount = 40.0;
    v.vignette.style = VignetteStyle::ColorPriority;
    check("hdr vignette +40", &src, &raw, &v, 360, 240);
    // a display with less headroom than the HDR Limit (the loupe's `RenderJob::with_hdr_view`)
    let mut d = hdr_on(3.0);
    d.hdr.view_ev = 1.5;
    check("hdr on a 1.5-stop display", &src, &raw, &d, 360, 240);
    // a rendered file (display tone map)
    check("hdr rendered file", &src, &SourceInfo::default(), &hdr_on(2.0), 360, 240);
    // a camera tone curve
    let curve = lightcraft_pipeline::tone::CameraTone::new(std::array::from_fn(|i| {
        let x = 0.004 * 1.18f32.powi(i as i32);
        [x, 1.0 - (-2.0 * x).exp()]
    }))
    .expect("curve");
    let cam = SourceInfo { raw: true, relative_wb: true, camera_tone: Some(curve), ..Default::default() };
    check("hdr camera tone", &src, &cam, &s, 360, 240);
}

/// An HDR render without an HDR edit is an ordinary float render (the CPU's, not the kernel's).
#[test]
fn hdr_depth_without_hdr_on_falls_back_to_the_cpu() {
    if !gpu() {
        return;
    }
    let src = sunset(120, 80);
    let req = RenderRequest { depth: OutputDepth::F32Hdr, ..RenderRequest::fit(120, 80) };
    assert!(lightcraft_gpu::render(&src, &SourceInfo::default(), &DevelopSettings::default(), &req, None).is_none());
}

/// The SDR rendition's highlight saturation (`crs:SDRBlend`) renders the same on both.
#[test]
fn sdr_blend_matches_the_cpu() {
    if !gpu() {
        return;
    }
    let src = Arc::new(Rgb32f::from_fn(240, 30, |x, y| {
        let v = 0.004 * 1.03f32.powi(x as i32);
        match y / 10 {
            0 => [v, v * 0.45, v * 0.15],
            1 => [v * 0.3, v * 0.5, v],
            _ => [v * 0.4, v, v * 0.5],
        }
    }));
    let raw = SourceInfo { raw: true, ..Default::default() };
    let req = RenderRequest::fit(240, 30);
    let plain = render(&src, &raw, &hdr_on(3.0), &req).image;
    for v in [100.0, -100.0] {
        let mut s = hdr_on(3.0);
        s.hdr.sdr_blend = v;
        let cpu = render(&src, &raw, &s, &req).image;
        let gpu = lightcraft_gpu::render(&src, &raw, &s, &req, None).expect("gpu render").image;
        let max = cpu.data.iter().zip(&gpu.data).flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c]))).max().unwrap_or(0);
        assert!(max <= 3, "SDRBlend {v}: max {max} LSB");
        let nd = |a: &lightcraft_raster::Rgba8| a.data.iter().zip(&plain.data).filter(|(p, q)| p != q).count();
        eprintln!("SDRBlend {v}: cpu differs from plain in {}, gpu in {}", nd(&cpu), nd(&gpu));
        assert_ne!(gpu.data, plain.data, "SDRBlend {v} changes the SDR rendition");
    }
}

#[test]
fn visualize_hdr_matches_the_cpu() {
    if !gpu() {
        return;
    }
    let src = sunset(240, 160);
    let raw = SourceInfo { raw: true, ..Default::default() };
    // a display with 1.5 stops of headroom: the sun's core is beyond it
    let req = RenderRequest { overlay: Overlay::HdrRange(15), ..RenderRequest::fit(240, 160) };
    let s = hdr_on(3.0);
    let cpu = render(&src, &raw, &s, &req).image;
    let gpu = lightcraft_gpu::render(&src, &raw, &s, &req, None).expect("gpu render").image;
    let differ = cpu.data.iter().zip(&gpu.data).filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 3)).count();
    // a pixel right at a band's edge may fall on the other side of it
    assert!(differ * 200 < cpu.data.len(), "{differ} of {} pixels differ", cpu.data.len());
    let beyond = gpu.data.iter().filter(|p| p[0] > 180 && p[1] < 90 && p[2] < 90).count();
    let banded = gpu.data.iter().filter(|p| p[2] > p[0].saturating_add(60)).count();
    assert!(beyond > 0 && banded > 0, "the sun should be painted beyond the display ({beyond}) and with an HDR band ({banded})");
}

/// The loupe's case: timings of a ~2.5 MP draft HDR render on each (printed; `--nocapture`).
#[test]
#[ignore]
fn bench_hdr_loupe() {
    if !gpu() {
        return;
    }
    let (w, h) = (1936, 1290);
    let src = sunset(w, h);
    let raw = SourceInfo { raw: true, ..Default::default() };
    let mut s = hdr_on(4.0);
    s.light.exposure = 0.3;
    s.light.highlights = -30.0;
    s.effects.clarity = 10.0;
    let req = RenderRequest { depth: OutputDepth::F32Hdr, quality: lightcraft_pipeline::Quality::Draft, ..RenderRequest::fit(w, h) };
    let cache = lightcraft_pipeline::StageCache::default();
    let mut run = |name: &str, gpu: bool| {
        for i in 0..8 {
            s.light.exposure = 0.3 + 0.05 * f64::from(i);
            let t = std::time::Instant::now();
            let r = if gpu {
                lightcraft_gpu::render(&src, &raw, &s, &req, Some(&cache)).expect("gpu")
            } else {
                lightcraft_pipeline::render_cached(&src, &raw, &s, &req, &cache)
            };
            eprintln!("{name} drag {i}: {:.1} ms ({}x{})", t.elapsed().as_secs_f64() * 1e3, r.image.width, r.image.height);
        }
    };
    run("cpu", false);
    run("gpu", true);
}
