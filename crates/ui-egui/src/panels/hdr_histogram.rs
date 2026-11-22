//! The histogram of a photo edited in HDR, laid out as Lightroom's (LR-VIEW-HISTOGRAM with HDR
//! on): the SDR section on the left (black to SDR white, 40 % of the width), the HDR section to its
//! right (stops above SDR white up to the edit's HDR Limit, a dashed line at each whole stop), a
//! solid line at SDR white, a yellow bar under the part of the HDR section the display can show
//! right now and a red bar (with that part dimmed) under what lies beyond its headroom — all of it
//! on an SDR display or while the window shows the SDR rendition. The highlight triangle is
//! yellow when light above white is all within the headroom, red when some is beyond it or
//! flattened at the HDR Limit.
//!
//! Binned from a small HDR rendition at the edit's own limit ([`Slot::HdrMaster`],
//! [`lightcraft_pipeline::hdr_histogram`]); the display's headroom is applied only here, so a
//! brightness change of the screen renders nothing.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Painter, Pos2, Rect, Stroke, pos2};
use lightcraft_catalog::PhotoId;
use lightcraft_pipeline::hdr_histogram::{BINS_PER_STOP, HdrHistogram, SDR_SHARE};

use crate::LightcraftApp;
use crate::render::Slot;
use crate::widgets::hex;

/// Long edge of the HDR rendition the histogram is binned from.
const MASTER_EDGE: usize = 512;

/// Yellow: HDR light the display shows; red: beyond its headroom (Lightroom's colours).
pub const WITHIN: Color32 = Color32::from_rgb(236, 200, 60);
pub const BEYOND: Color32 = Color32::from_rgb(230, 52, 52);

/// The HDR histogram of photo `id` once its HDR rendition has been binned; asks for that render
/// (latest settings only) while the photo is edited in HDR. `None`: draw the SDR histogram.
pub fn current(app: &mut LightcraftApp, id: PhotoId) -> Option<std::sync::Arc<HdrHistogram>> {
    let p = app.session.catalog.photo(id)?;
    if !p.develop.hdr.enabled {
        return None;
    }
    let aspect = p.width.max(1) as f32 / p.height.max(1) as f32;
    let (w, h) = if aspect >= 1.0 {
        (MASTER_EDGE, ((MASTER_EDGE as f32 / aspect) as usize).max(8))
    } else {
        (((MASTER_EDGE as f32 * aspect) as usize).max(8), MASTER_EDGE)
    };
    if let Some(job) = app.session.render_job(id, w, h, false, true).map(|j| j.hdr_master()) {
        let key = job.key;
        let have = app.renderer.textures.get(&Slot::HdrMaster).is_some_and(|t| t.key == key);
        if !have {
            app.renderer.request(Slot::HdrMaster, job, 55);
        }
    }
    // the last histogram of this photo stays until the next one is ready (another photo's never)
    app.renderer.textures.get(&Slot::HdrMaster).filter(|t| t.photo == id).and_then(|t| t.hdr_histogram.clone())
}

/// The display headroom (stops above SDR white) the histogram marks against: 0 on an SDR
/// display, under Preview for SDR Display or Visualize HDR (the window then shows SDR).
pub fn display_stops(app: &LightcraftApp) -> f32 {
    super::detail::hdr_headroom(app).map_or(0.0, |h| h.log2().max(0.0))
}

/// Whether the highlight triangle warns (red): light beyond the display's headroom, or
/// flattened at the HDR Limit.
pub fn warns(h: &HdrHistogram, display: f32) -> bool {
    let tiny = (h.total / 2000).max(1);
    h.above(display) >= tiny || h.at_limit >= tiny
}

/// Draw `h` into `plot` (`fill` is the panel's background, for the dimmed part).
pub fn draw(p: &Painter, plot: Rect, h: &HdrHistogram, display: f32, fill: Color32) {
    let limit = h.limit.max(1.0 / BINS_PER_STOP as f32);
    let n_hdr = h.hdr_bins();
    let sdr_w = plot.width() * SDR_SHARE;
    let hdr_w = plot.width() - sdr_w;
    // densities: a bin's count over its width, so the narrow SDR bins and the wide HDR ones compare
    let sdr_dx = sdr_w / 256.0;
    let hdr_dx = hdr_w / n_hdr as f32;
    let smooth = |v: &[u32], i: usize, r: usize| -> f32 {
        let (a, b) = (i.saturating_sub(r), (i + r + 1).min(v.len()));
        v.get(a..b).map_or(0.0, |s| s.iter().map(|x| *x as f32).sum::<f32>() / (b - a).max(1) as f32)
    };
    let series = |sdr: &[u32], hdr: &[u32]| -> Vec<(f32, f32)> {
        let mut out: Vec<(f32, f32)> = (0..256).map(|i| (plot.left() + sdr_dx * (i as f32 + 0.5), smooth(sdr, i, 2) / sdr_dx)).collect();
        out.extend((0..n_hdr).map(|i| (plot.left() + sdr_w + hdr_dx * (i as f32 + 0.5), smooth(hdr, i, 1) / hdr_dx)));
        out
    };
    let chans = [
        (series(&h.sdr[0], &h.hdr[0]), hex("#df3939")),
        (series(&h.sdr[1], &h.hdr[1]), hex("#44b072")),
        (series(&h.sdr[2], &h.hdr[2]), hex("#3b6fe0")),
    ];
    let lum = series(&h.luma_sdr, &h.luma_hdr);
    // the peak ignores the black and white end bins (clipping spikes), as the SDR histogram does
    let peak =
        chans.iter().flat_map(|(v, _)| v.iter().enumerate().filter(|(i, _)| *i > 2 && *i != 255).map(|(_, (_, d))| *d)).fold(1e-6f32, f32::max);
    let y = |d: f32| plot.bottom() - plot.height() * (d / peak).sqrt().min(1.0);
    // luminance fill
    let grey = Color32::from_gray(120).gamma_multiply(0.35);
    let mut mesh = Mesh::default();
    for (i, (x, d)) in lum.iter().enumerate() {
        mesh.vertices.push(Vertex { pos: pos2(*x, y(*d)), uv: Pos2::ZERO, color: grey });
        mesh.vertices.push(Vertex { pos: pos2(*x, plot.bottom()), uv: Pos2::ZERO, color: grey });
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.indices.extend([k - 2, k - 1, k, k - 1, k + 1, k]);
        }
    }
    p.add(mesh);
    for (v, c) in &chans {
        let pts: Vec<Pos2> = v.iter().map(|(x, d)| pos2(*x, y(*d))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.5, *c)));
    }
    // the part beyond the display's headroom: dimmed, a red bar under it; a yellow one under
    // the part it shows
    let white_x = plot.left() + sdr_w;
    let shown = display.clamp(0.0, limit);
    let edge_x = white_x + hdr_w * shown / limit;
    let bar = 3.0;
    if edge_x < plot.right() - 0.5 {
        let beyond = Rect::from_min_max(pos2(edge_x, plot.top()), plot.right_bottom());
        p.rect_filled(beyond, 0.0, fill.gamma_multiply(0.55));
        p.rect_filled(Rect::from_min_max(pos2(edge_x, plot.bottom() - bar), plot.right_bottom()), 0.0, BEYOND);
    }
    if edge_x > white_x + 0.5 {
        p.rect_filled(Rect::from_min_max(pos2(white_x, plot.bottom() - bar), pos2(edge_x, plot.bottom())), 0.0, WITHIN);
    }
    // SDR white (solid), each whole stop above it (dashed)
    let line = Color32::from_gray(150);
    p.line_segment([pos2(white_x, plot.top()), pos2(white_x, plot.bottom())], Stroke::new(1.0, line));
    let mut stop = 1.0f32;
    while stop < limit - 1e-3 {
        let x = white_x + hdr_w * stop / limit;
        p.extend(egui::Shape::dashed_line(
            &[pos2(x, plot.top()), pos2(x, plot.bottom() - bar)],
            Stroke::new(1.0, line.gamma_multiply(0.7)),
            3.0,
            3.0,
        ));
        stop += 1.0;
    }
}

/// The plot's size hint for tests: where SDR white falls in a `plot`.
pub fn white_x(plot: Rect) -> f32 {
    plot.left() + plot.width() * SDR_SHARE
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;
    use lightcraft_pipeline::{DeepImage, DeepSamples, OutputSpace};

    fn hist(px: &[[f32; 3]], limit: f32) -> HdrHistogram {
        let img = DeepImage { width: px.len(), height: 1, space: OutputSpace::Srgb, samples: DeepSamples::F32(px.concat()) };
        HdrHistogram::of(&img, limit).unwrap()
    }

    #[test]
    fn triangle_warns_beyond_the_display_or_at_the_limit() {
        let mut px = vec![[0.2f32; 3]; 1000];
        px.extend([[3.0, 2.5, 2.0]; 10]);
        let h = hist(&px, 4.0);
        assert!(!warns(&h, 2.0), "1.6 stops fits a 2-stop display");
        assert!(warns(&h, 1.0), "beyond a 1-stop display");
        assert!(warns(&h, 0.0), "an SDR display shows none of it");
        let flat = hist(&[[16.0; 3]; 10], 4.0);
        assert!(warns(&flat, 5.0), "flattened at the limit");
    }

    #[test]
    fn draws_without_panicking_on_odd_input() {
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let p = ui.painter();
            let plot = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 90.0));
            for (px, limit) in [(vec![[0.0f32; 3]], 0.0), (vec![[f32::NAN, 1e9, -3.0]], 5.0), (vec![[2.0, 0.5, 0.1]; 3], 2.3)] {
                draw(p, plot, &hist(&px, limit), 1.2, Color32::BLACK);
            }
        });
        out.textures_delta.clear();
        assert!((white_x(Rect::from_min_size(pos2(10.0, 0.0), vec2(100.0, 1.0))) - 50.0).abs() < 1e-4);
    }
}
