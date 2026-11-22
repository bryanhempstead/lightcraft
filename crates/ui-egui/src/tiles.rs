//! Native-resolution detail in the loupe (LR-VIEW-ZOOM): the whole-image preview is rendered
//! no larger than Settings ▸ Performance ▸ Preview size, so at 1:1 (and 200–800 %) of a large
//! photo it would only be magnified. Here the visible part is rendered again at the photo's own
//! resolution — a region render through the same pipeline (GPU or CPU,
//! [`lightcraft_pipeline::region`]: masks, spots, crop and lens corrections land on the same
//! pixels as in the whole render and the export) — and drawn over the preview in pieces of
//! [`PIECE`] px.
//!
//! * Levels: the photo's native resolution (1:1 and closer) for regions; between the preview
//!   size and 1:1, half or a quarter of it, rendered whole (cheap at that size, and panning
//!   needs nothing more).
//! * Requests: the missing tiles of a [`PIECE`]-px grid over the visible part (a ring of tiles
//!   around it once nothing visible is missing), as one region; a newer request replaces a
//!   queued one, results of older ones are kept if their look is still current.
//! * Progressive: while a slider is dragged the visible part renders at draft quality; the
//!   pieces of the previous look stay on screen until the new ones arrive, then go.
//! * A cache of pieces per view (LRU, [`BUDGET`] bytes), dropped when the photo changes.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, pos2};
use lightcraft_catalog::PhotoId;
use lightcraft_pipeline::{Region, RegionPx};

/// Tile size (px of the level's render) of the request grid and of the uploaded pieces.
pub const PIECE: usize = 512;
/// Texture bytes kept per view.
pub const BUDGET: usize = 512 << 20;
/// Render priority (below the loupe's whole-image render, above its "before" view).
pub const PRIORITY: u32 = 98;

/// One uploaded piece of a detail render.
pub struct Piece {
    /// The look it shows (photo settings, level, quality; not the region).
    pub look: u64,
    /// Where it sits on the photo (normalized to the whole render).
    pub rect: Region,
    pub size: [usize; 2],
    pub tex: egui::TextureHandle,
    /// sRGB copy for headless screenshots ([`crate::render::Renderer::keep_pixels`]).
    pub pixels: Option<Arc<egui::ColorImage>>,
    /// Frame it was last drawn or wanted in.
    pub used: u64,
}

/// What a request was for (kept until its result arrives).
#[derive(Clone, Copy, Debug)]
pub struct Asked {
    pub look: u64,
    pub photo: PhotoId,
    /// Order of the requests: a result newer than what is shown replaces it, even when the look
    /// moved on since (a slider drag keeps updating the view instead of waiting for its end).
    pub seq: u64,
}

/// One view's detail layer (the loupe, a Compare candidate).
#[derive(Default)]
pub struct TileView {
    pub photo: Option<PhotoId>,
    pub pieces: Vec<Piece>,
    /// Look of the latest piece received (the previous look stays drawn until this one covers)
    /// and its request's order.
    pub shown: Option<u64>,
    shown_seq: u64,
    seq: u64,
    /// The look wanted now (set every frame by [`want`]).
    pub look: Option<u64>,
    /// Requests by job key.
    pub asked: HashMap<u64, Asked>,
    /// Render time of the latest result (ms) and its size.
    pub last_ms: f64,
    pub last_px: [usize; 2],
    pub level: f64,
    /// Frames counted for the LRU.
    frame: u64,
    /// When a view last asked for detail ([`update`]; egui time, s): views not shown for
    /// [`UNSEEN_SECS`] give their pieces back ([`forget_unseen`]).
    pub seen: f64,
}

/// See [`TileView::seen`].
pub const UNSEEN_SECS: f64 = 20.0;

/// Drop the pieces of views not drawn for [`UNSEEN_SECS`] (Compare left, the loupe closed).
pub fn forget_unseen(views: &mut HashMap<u8, TileView>, now: f64) {
    views.retain(|_, v| now - v.seen <= UNSEEN_SECS);
}

impl TileView {
    pub fn bytes(&self) -> usize {
        self.pieces.iter().map(|p| p.size[0] * p.size[1] * 4).sum()
    }

    /// Forget everything (another photo, the renderer reset).
    pub fn clear(&mut self) {
        *self = TileView::default();
    }

    /// Take a finished region render: cut into pieces and uploaded. `false` when it was for
    /// another photo or a look no longer wanted (dropped).
    #[allow(clippy::too_many_arguments)]
    pub fn accept(
        &mut self,
        ctx: &egui::Context,
        name: &str,
        job_key: u64,
        image: &lightcraft_raster::Rgba8,
        px: RegionPx,
        ms: f64,
        keep_pixels: bool,
        p3: bool,
    ) -> bool {
        let Some(asked) = self.asked.remove(&job_key) else { return false };
        let current = Some(asked.look) == self.look || Some(asked.look) == self.shown;
        if Some(asked.photo) != self.photo || !(current || asked.seq > self.shown_seq) {
            return false;
        }
        self.frame += 1;
        let (fw, fh) = (px.full_w.max(1) as f64, px.full_h.max(1) as f64);
        for ty in (0..px.h).step_by(PIECE) {
            for tx in (0..px.w).step_by(PIECE) {
                let (w, h) = (PIECE.min(px.w - tx), PIECE.min(px.h - ty));
                let rect = Region {
                    x0: (px.x + tx) as f64 / fw,
                    y0: (px.y + ty) as f64 / fh,
                    x1: (px.x + tx + w) as f64 / fw,
                    y1: (px.y + ty + h) as f64 / fh,
                };
                let bytes = crop_bytes(image, tx, ty, w, h);
                let srgb = Arc::new(egui::ColorImage::from_rgba_unmultiplied([w, h], &bytes));
                let pixels = keep_pixels.then(|| srgb.clone());
                let shown = if p3 {
                    let mut b = bytes;
                    crate::display_p3::srgb_to_p3(&mut b);
                    Arc::new(egui::ColorImage::from_rgba_unmultiplied([w, h], &b))
                } else {
                    srgb
                };
                // the same place in the same look: replaced
                self.pieces.retain(|p| !(p.look == asked.look && same(&p.rect, &rect)));
                let tex = ctx.load_texture(format!("{name}-{:.5}-{:.5}", rect.x0, rect.y0), shown, egui::TextureOptions::LINEAR);
                self.pieces.push(Piece { look: asked.look, rect, size: [w, h], tex, pixels, used: self.frame });
            }
        }
        self.shown = Some(asked.look);
        self.shown_seq = self.shown_seq.max(asked.seq);
        self.last_ms = ms;
        self.last_px = [px.w, px.h];
        // once the current look has arrived the others go; until then the one shown stays
        let keep = (asked.look, self.look);
        self.pieces.retain(|p| p.look == keep.0 || Some(p.look) == keep.1);
        self.trim();
        true
    }

    /// Keep the pieces under [`BUDGET`] (least recently used go first).
    fn trim(&mut self) {
        while self.bytes() > BUDGET && self.pieces.len() > 1 {
            let Some(i) = self.pieces.iter().enumerate().min_by_key(|(_, p)| p.used).map(|(i, _)| i) else { break };
            self.pieces.remove(i);
        }
    }

    /// Is `r` covered by pieces of `look`?
    pub fn covered(&self, look: u64, r: &Region) -> bool {
        // pieces of one look don't overlap: the covered area adds up
        let area = |a: &Region| (a.x1 - a.x0).max(0.0) * (a.y1 - a.y0).max(0.0);
        let want = area(r);
        if want <= 0.0 {
            return true;
        }
        let got: f64 = self.pieces.iter().filter(|p| p.look == look).map(|p| area(&intersect(&p.rect, r))).sum();
        got >= want * (1.0 - 1e-6)
    }

    /// Draw the pieces that belong on screen over `img` (the photo's rectangle; clipped by the
    /// painter): the previous look's under the current one's.
    pub fn draw(&mut self, painter: &egui::Painter, img: Rect, visible: Rect) {
        let Some(look) = self.look else { return };
        self.frame += 1;
        let frame = self.frame;
        let at = |r: &Region| {
            Rect::from_min_max(
                pos2(img.left() + r.x0 as f32 * img.width(), img.top() + r.y0 as f32 * img.height()),
                pos2(img.left() + r.x1 as f32 * img.width(), img.top() + r.y1 as f32 * img.height()),
            )
        };
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        let stale = self.shown.filter(|s| *s != look);
        for pass in [stale, Some(look)].into_iter().flatten() {
            for p in self.pieces.iter_mut().filter(|p| p.look == pass) {
                let r = at(&p.rect);
                if r.intersects(visible) {
                    painter.image(p.tex.id(), r, uv, Color32::WHITE);
                    p.used = frame;
                }
            }
        }
    }
}

fn same(a: &Region, b: &Region) -> bool {
    (a.x0 - b.x0).abs() < 1e-9 && (a.y0 - b.y0).abs() < 1e-9 && (a.x1 - b.x1).abs() < 1e-9 && (a.y1 - b.y1).abs() < 1e-9
}

fn intersect(a: &Region, b: &Region) -> Region {
    Region { x0: a.x0.max(b.x0), y0: a.y0.max(b.y0), x1: a.x1.min(b.x1), y1: a.y1.min(b.y1) }
}

/// RGBA bytes of the `w × h` pixels at (`x`, `y`) of `img` (missing ones transparent black).
fn crop_bytes(img: &lightcraft_raster::Rgba8, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let at = row * img.width + x;
        for p in img.data.get(at..at + w).unwrap_or(&[]) {
            out.extend_from_slice(p);
        }
        out.resize((row - y + 1) * w * 4, 0);
    }
    out
}

/// The render scale (of the native resolution) detail at a displayed scale of `shown` needs,
/// when the whole-image preview (`preview` of the native resolution) falls short: 1 at 1:1 and
/// closer, else ½ or ¼ (rendered whole). `None`: the preview suffices.
pub fn level(shown: f64, preview: f64) -> Option<f64> {
    if !(shown.is_finite() && preview.is_finite()) || shown <= preview * 1.05 {
        return None;
    }
    let r = [0.25, 0.5, 1.0].into_iter().find(|r| *r >= shown * 0.999).unwrap_or(1.0);
    // a level no finer than the preview adds nothing
    (r > preview * 1.05).then_some(r)
}

/// The tiles (normalized rectangles on a [`PIECE`]-px grid of a `full_w × full_h` render)
/// covering `vis` (normalized), grown by `ring` tiles.
pub fn grid(full_w: usize, full_h: usize, vis: &Region, ring: usize) -> Vec<Region> {
    let (fw, fh) = (full_w.max(1), full_h.max(1));
    let (nx, ny) = (fw.div_ceil(PIECE), fh.div_ceil(PIECE));
    let idx = |v: f64, n: usize, size: usize| ((v.clamp(0.0, 1.0) * size as f64 / PIECE as f64).floor() as usize).min(n.saturating_sub(1));
    let (x0, y0) = (idx(vis.x0, nx, fw).saturating_sub(ring), idx(vis.y0, ny, fh).saturating_sub(ring));
    let (x1, y1) = ((idx(vis.x1 - 1e-9, nx, fw) + ring).min(nx - 1), (idx(vis.y1 - 1e-9, ny, fh) + ring).min(ny - 1));
    let mut out = Vec::new();
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let (px0, py0) = (tx * PIECE, ty * PIECE);
            let (px1, py1) = ((px0 + PIECE).min(fw), (py0 + PIECE).min(fh));
            out.push(Region { x0: px0 as f64 / fw as f64, y0: py0 as f64 / fh as f64, x1: px1 as f64 / fw as f64, y1: py1 as f64 / fh as f64 });
        }
    }
    out
}

/// The bounding box of `tiles`.
pub fn bbox(tiles: &[Region]) -> Option<Region> {
    let first = *tiles.first()?;
    Some(tiles.iter().fold(first, |a, t| Region { x0: a.x0.min(t.x0), y0: a.y0.min(t.y0), x1: a.x1.max(t.x1), y1: a.y1.max(t.y1) }))
}

/// What a view asks of [`update`].
pub struct Want {
    pub view: u8,
    pub photo: PhotoId,
    /// The photo's rectangle on screen (its final rectangle while a click-zoom animates) and
    /// the part of the screen it can be seen in.
    pub img: Rect,
    pub area: Rect,
    pub ppp: f32,
    pub apply_crop: bool,
    pub before: bool,
    /// Long edge (px) of the whole-image preview shown under the tiles at this zoom (the loupe's
    /// full-quality render: the Preview size setting caps it).
    pub preview_long: usize,
    /// A slider or tool is being dragged: draft quality, the visible part only.
    pub interacting: bool,
    pub overlay: lightcraft_pipeline::Overlay,
    pub proof: Option<lightcraft_pipeline::Proof>,
    /// egui time now (s), for [`forget_unseen`].
    pub now: f64,
}

/// Request the detail `w` needs and say which look its pieces should show this frame
/// ([`TileView::look`]; `None`: no detail at this zoom).
pub fn update(app: &mut crate::LightcraftApp, w: &Want) -> Option<u64> {
    let now = w.now;
    let view = app.renderer.tiles.entry(w.view).or_default();
    if view.photo != Some(w.photo) {
        view.clear();
        view.photo = Some(w.photo);
    }
    view.seen = now;
    let photo = app.session.catalog.photo(w.photo)?;
    if photo.kind == lightcraft_catalog::MediaKind::Video || photo.width < 2 || photo.height < 2 {
        view.look = None;
        return None;
    }
    let (nw, nh) = app.session.detail_size(w.photo, 1.0, w.apply_crop)?;
    let native = nw.max(nh).max(1) as f64;
    let shown_px = (w.img.width().max(w.img.height()) * w.ppp) as f64;
    let Some(level) = level(shown_px / native, w.preview_long as f64 / native) else {
        let view = app.renderer.tiles.entry(w.view).or_default();
        view.look = None;
        return None;
    };
    let (fw, fh) = app.session.detail_size(w.photo, level, w.apply_crop)?;
    let vis_screen = w.img.intersect(w.area);
    if !vis_screen.is_positive() {
        return None;
    }
    let norm = |p: Pos2| ((p.x - w.img.left()) / w.img.width().max(1e-3), (p.y - w.img.top()) / w.img.height().max(1e-3));
    let (a, b) = (norm(vis_screen.min), norm(vis_screen.max));
    let vis = Region { x0: a.0 as f64, y0: a.1 as f64, x1: b.0 as f64, y1: b.1 as f64 };
    // the job for a region, as asked for (draft while dragging; the loupe's overlay and proof)
    let job_for = |app: &mut crate::LightcraftApp, r: Region| {
        app.session.detail_job(w.photo, level, r, w.apply_crop, w.before).map(|j| {
            let j = if w.interacting { j.draft() } else { j };
            j.with_overlay(w.overlay).with_proof(w.proof)
        })
    };
    // the look: the job's key without its region
    let look = job_for(app, Region::WHOLE)?.key ^ Region::WHOLE.key().rotate_left(7) ^ (level.to_bits().rotate_left(13));
    let view = app.renderer.tiles.entry(w.view).or_default();
    view.look = Some(look);
    view.level = level;
    // below 1:1 the level renders whole; at 1:1 the visible tiles, then a ring around them
    let target = if level < 1.0 {
        vec![Region::WHOLE]
    } else {
        let visible = grid(fw, fh, &vis, 0);
        let missing: Vec<Region> = visible.into_iter().filter(|t| !view.covered(look, t)).collect();
        if missing.is_empty() && !w.interacting { grid(fw, fh, &vis, 1).into_iter().filter(|t| !view.covered(look, t)).collect() } else { missing }
    };
    if target.iter().all(|t| view.covered(look, t)) {
        return Some(look);
    }
    let Some(region) = bbox(&target) else { return Some(look) };
    let slot = crate::render::Slot::Detail(w.view);
    if let Some(job) = job_for(app, region) {
        let key = job.key;
        if app.renderer.wanted(slot) != Some(key) {
            let view = app.renderer.tiles.entry(w.view).or_default();
            view.seq += 1;
            let seq = view.seq;
            view.asked.insert(key, Asked { look, photo: w.photo, seq });
            // (bounded: a request no longer queued and never answered is forgotten)
            if view.asked.len() > 64 {
                let k = view.asked.keys().next().copied();
                if let Some(k) = k.filter(|k| *k != key) {
                    view.asked.remove(&k);
                }
            }
            app.renderer.request(slot, job, PRIORITY);
        }
    }
    Some(look)
}

/// Inspect: per view, what the detail layer holds.
pub fn inspect(views: &HashMap<u8, TileView>) -> serde_json::Value {
    let v: serde_json::Map<String, serde_json::Value> = views
        .iter()
        .map(|(k, t)| {
            (
                k.to_string(),
                serde_json::json!({
                    "photo": t.photo.map(|p| p.0),
                    "level": t.level,
                    "active": t.look.is_some(),
                    "pieces": t.pieces.len(),
                    "current": t.look.map_or(0, |l| t.pieces.iter().filter(|p| p.look == l).count()),
                    "bytes": t.bytes(),
                    "lastMs": t.last_ms,
                    "lastPx": t.last_px,
                }),
            )
        })
        .collect();
    serde_json::Value::Object(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_follow_the_zoom() {
        // a 6000 px photo, a 2560 px preview (0.43): fit and 40 % need nothing
        assert_eq!(level(0.2, 0.43), None);
        assert_eq!(level(0.4, 0.43), None);
        assert_eq!(level(0.5, 0.43), Some(0.5));
        assert_eq!(level(0.75, 0.43), Some(1.0));
        assert_eq!(level(1.0, 0.43), Some(1.0));
        assert_eq!(level(8.0, 0.43), Some(1.0), "beyond 1:1 the native pixels, magnified");
        // a preview as large as the photo needs no detail
        assert_eq!(level(2.0, 1.0), None);
        assert_eq!(level(f64::NAN, 0.4), None);
    }

    #[test]
    fn the_grid_covers_the_visible_part_and_its_ring() {
        let vis = Region { x0: 0.5, y0: 0.5, x1: 0.6, y1: 0.6 };
        let tiles = grid(6000, 4000, &vis, 0);
        // x 3000..3600 → tiles 5..=7, y 2000..2400 → tiles 3..=4
        assert_eq!(tiles.len(), 3 * 2);
        let b = bbox(&tiles).unwrap();
        assert!(b.x0 <= 0.5 && b.x1 >= 0.6 && b.y0 <= 0.5 && b.y1 >= 0.6);
        assert_eq!(b.x0 * 6000.0, 5.0 * PIECE as f64);
        let ring = grid(6000, 4000, &vis, 1);
        assert_eq!(ring.len(), 5 * 4);
        // the last tiles end at the render's edge
        let edge = grid(6000, 4000, &Region { x0: 0.99, y0: 0.99, x1: 1.0, y1: 1.0 }, 0);
        assert_eq!(edge.len(), 1);
        assert_eq!((edge[0].x1, edge[0].y1), (1.0, 1.0));
    }
}
