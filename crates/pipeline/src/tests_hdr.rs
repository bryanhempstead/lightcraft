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

#[test]
fn visualize_hdr_paints_only_what_rises_above_white() {
    use crate::Overlay;
    use crate::visualize::HDR_BANDS;
    let src = sunset();
    let req = RenderRequest { overlay: Overlay::HdrRange, ..RenderRequest::fit(240, 240) };
    let banded = |p: &[u8; 4]| HDR_BANDS.iter().any(|b| (0..3).all(|k| (p[k] as i32 - b[k] as i32).abs() < 70) && p[0] > p[2] + 40);
    let on = render(&src, &raw(), &hdr_on(3.0), &req);
    let n = on.image.data.iter().filter(|p| banded(p)).count();
    assert!(n > 0 && n < on.image.data.len() / 3, "{n} banded pixels");
    // the rest is grey
    assert!(on.image.data.iter().filter(|p| !banded(p)).all(|p| p[0] == p[1] && p[1] == p[2]));
    // HDR off: the overlay draws nothing
    let off = render(&src, &raw(), &DevelopSettings::default(), &req);
    let plain = render(&src, &raw(), &DevelopSettings::default(), &RenderRequest::fit(240, 240));
    assert_eq!(off.image.data, plain.image.data);
}

/// `crs:SDRBlend` (Camera Raw's highlight saturation of the SDR rendition): more colour in the
/// brightest SDR tones at +100, less at −100, shadows and midtones untouched; nothing in the HDR
/// render, and nothing while HDR is off (a dormant value).
#[test]
fn sdr_blend_saturates_the_sdr_renditions_highlights() {
    // orange, blue and green ramps from deep shadow to past white (scene linear)
    let src = Rgb32f::from_fn(240, 30, |x, y| {
        let v = 0.004 * 1.03f32.powi(x as i32);
        match y / 10 {
            0 => [v, v * 0.45, v * 0.15],
            1 => [v * 0.3, v * 0.5, v],
            _ => [v * 0.4, v, v * 0.5],
        }
    });
    let req = RenderRequest::fit(240, 30);
    let base = render(&src, &raw(), &hdr_on(3.0), &req).image;
    let blend = |v: f64| {
        let mut s = hdr_on(3.0);
        s.hdr.sdr_blend = v;
        s
    };
    let chroma = |p: &[u8; 4]| i32::from(p[0].max(p[1]).max(p[2])) - i32::from(p[0].min(p[1]).min(p[2]));
    let more = render(&src, &raw(), &blend(100.0), &req).image;
    let less = render(&src, &raw(), &blend(-100.0), &req).image;
    let (mut up, mut down, mut dark_moved) = (0, 0, 0);
    for ((b, m), l) in base.data.iter().zip(&more.data).zip(&less.data) {
        let lum = 0.2126 * f32::from(b[0]) + 0.7152 * f32::from(b[1]) + 0.0722 * f32::from(b[2]);
        if lum < 150.0 {
            dark_moved += usize::from(b != m || b != l);
        } else if chroma(b) > 8 && chroma(b) < 200 {
            up += usize::from(chroma(m) > chroma(b));
            down += usize::from(chroma(l) < chroma(b));
        }
    }
    assert!(up > 20 && down > 20, "highlights: {up} more saturated, {down} less");
    assert_eq!(dark_moved, 0, "shadows and midtones stay put");
    // the HDR render ignores it
    let hreq = RenderRequest { depth: OutputDepth::F32Hdr, ..req };
    assert_eq!(render(&src, &raw(), &blend(100.0), &hreq).deep, render(&src, &raw(), &hdr_on(3.0), &hreq).deep);
    // dormant while HDR is off
    let mut off = DevelopSettings::default();
    off.hdr.sdr_blend = 100.0;
    assert_eq!(render(&src, &raw(), &off, &req).image.data, render(&src, &raw(), &DevelopSettings::default(), &req).image.data);
}
