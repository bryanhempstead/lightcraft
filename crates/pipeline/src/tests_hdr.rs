//! HDR editing: the HDR render ([`OutputDepth::F32Hdr`]) and the SDR rendition every other render uses.

use lightcraft_develop::DevelopSettings;
use lightcraft_raster::Rgb32f;

use crate::{DeepSamples, OutputDepth, RenderRequest, SourceInfo, render};

fn sunset() -> Rgb32f {
    let lib = lightcraft_scenes::demo_library();
    let s = lib.iter().find(|s| s.kind == lightcraft_scenes::Kind::OceanSunset).map(|s| s.render(240, 160));
    s.unwrap_or_else(|| lib[0].render(240, 160))
}

fn raw() -> SourceInfo {
    SourceInfo { raw: true, ..Default::default() }
}

fn hdr_on(max_ev: f64) -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.hdr.enabled = true;
    s.hdr.max_ev = max_ev;
    s
}

fn deep(src: &Rgb32f, s: &DevelopSettings, depth: OutputDepth) -> Vec<f32> {
    let req = RenderRequest { depth, ..RenderRequest::fit(240, 240) };
    match render(src, &raw(), s, &req).deep.map(|d| d.samples) {
        Some(DeepSamples::F32(v)) => v,
        other => panic!("expected float samples, got {other:?}"),
    }
}

#[test]
fn hdr_render_keeps_highlights_up_to_the_peak() {
    let src = sunset();
    let v = deep(&src, &hdr_on(3.0), OutputDepth::F32Hdr);
    assert!(v.iter().all(|x| x.is_finite() && (0.0..=8.0 + 1e-4).contains(x)));
    let max = v.iter().copied().fold(0.0f32, f32::max);
    assert!(max > 1.5, "the sun should rise above SDR white, max {max}");
    // a smaller headroom limit caps lower
    let v1 = deep(&src, &hdr_on(1.0), OutputDepth::F32Hdr);
    assert!(v1.iter().all(|x| *x <= 2.0 + 1e-4));
}

#[test]
fn hdr_midtones_match_the_sdr_render() {
    let src = sunset();
    let hdr = deep(&src, &hdr_on(3.0), OutputDepth::F32Hdr);
    let sdr = deep(&src, &DevelopSettings::default(), OutputDepth::F32Linear);
    // pixels whose brightest channel is a midtone (bright saturated colours legitimately differ:
    // SDR desaturates them at white, HDR keeps their colour)
    let mut n = 0;
    for (h, s) in hdr.as_chunks::<3>().0.iter().zip(sdr.as_chunks::<3>().0) {
        if s.iter().all(|v| *v < 0.15) {
            for (a, b) in h.iter().zip(s) {
                assert!((a - b).abs() < 2e-3, "{h:?} vs {s:?}");
            }
            n += 1;
        }
    }
    assert!(n > 100, "the scene should have midtones and shadows ({n})");
}

#[test]
fn f32hdr_without_hdr_is_the_sdr_float_render() {
    let src = sunset();
    let s = DevelopSettings::default();
    assert_eq!(deep(&src, &s, OutputDepth::F32Hdr), deep(&src, &s, OutputDepth::F32Linear));
}

#[test]
fn sdr_renders_of_an_hdr_edit_use_the_sdr_rendition() {
    let src = sunset();
    let req = RenderRequest::fit(240, 240);
    // neutral offsets: identical to the edit with HDR off
    let plain = render(&src, &raw(), &DevelopSettings::default(), &req);
    let on = render(&src, &raw(), &hdr_on(3.0), &req);
    assert_eq!(plain.image.data, on.image.data);
    // offsets apply to every SDR render
    let mut s = hdr_on(3.0);
    s.hdr.sdr_brightness = 50.0;
    s.hdr.sdr_highlights = -40.0;
    let brighter = render(&src, &raw(), &s, &req);
    let expect = render(&src, &raw(), &s.sdr_rendition(), &req);
    assert_eq!(brighter.image.data, expect.image.data);
    assert_ne!(brighter.image.data, plain.image.data);
    let r = s.sdr_rendition();
    assert!(!r.hdr.enabled && (r.light.exposure - 0.5).abs() < 1e-9 && r.light.highlights == -40.0);
}

#[test]
fn camera_tone_raws_open_their_highlights_into_the_headroom() {
    // the Lightroom-matched camera tone path (Bryan's fork): same shadows, highlights above 1
    let src = sunset();
    let info = SourceInfo { raw: true, camera_tone: crate::tone::rendered_reference(), ..Default::default() };
    let render_f = |s: &DevelopSettings, depth: OutputDepth| {
        let req = RenderRequest { depth, ..RenderRequest::fit(240, 240) };
        match render(&src, &info, s, &req).deep.map(|d| d.samples) {
            Some(DeepSamples::F32(v)) => v,
            other => panic!("expected float samples, got {other:?}"),
        }
    };
    let mut s = hdr_on(3.0);
    s.light.exposure = 1.0;
    let hdr = render_f(&s, OutputDepth::F32Hdr);
    let mut plain = s.clone();
    plain.hdr.enabled = false;
    let sdr = render_f(&plain, OutputDepth::F32Linear);
    let max = hdr.iter().copied().fold(0.0f32, f32::max);
    assert!(max > 1.5 && max <= 8.0 + 1e-4, "max {max}");
    let dark = sdr.iter().filter(|b| **b < 0.15).count();
    let close = hdr.iter().zip(&sdr).filter(|(a, b)| **b < 0.15 && (**a - **b).abs() < 3e-3).count();
    assert!(dark > 100 && close == dark, "{close} of {dark} dark samples unchanged");
}
