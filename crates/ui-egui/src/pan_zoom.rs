//! Pan and zoom for dialog previews (Enhance's Before / After, the Merge preview): hovering a
//! preview shows a grab hand, dragging pans, the scroll wheel or a pinch zooms around the pointer,
//! a double-click goes back to the whole picture. Previews shown side by side share one
//! [`PanZoom`], so they stay in sync (Bryan, 2026-11-23: "if I hover over it it turns into a hand and
//! I can click and drag around and zoom in and out").

use egui::{CursorIcon, Rect, Sense, Vec2, pos2, vec2};

/// Largest zoom, relative to the whole picture fitting the pane.
pub const MAX_ZOOM: f32 = 16.0;

/// How a preview is framed in its pane: `zoom` relative to the whole picture fitting (1 = fit),
/// `center` the picture point (0..1 each way) at the pane's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanZoom {
    pub zoom: f32,
    pub center: Vec2,
}

impl Default for PanZoom {
    fn default() -> Self {
        PanZoom { zoom: 1.0, center: vec2(0.5, 0.5) }
    }
}

impl PanZoom {
    /// The scale that fits a picture of `size` pixels in `pane` (pane points per picture pixel).
    fn fit(pane: Rect, size: [usize; 2]) -> f32 {
        let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
        (pane.width() / w).min(pane.height() / h)
    }

    /// The picture's size on screen at this zoom.
    fn dims(&self, pane: Rect, size: [usize; 2]) -> Vec2 {
        let s = Self::fit(pane, size) * self.zoom;
        vec2(size[0].max(1) as f32 * s, size[1].max(1) as f32 * s)
    }

    /// Where the whole picture is drawn for `pane` (the caller clips to the pane).
    pub fn image_rect(&self, pane: Rect, size: [usize; 2]) -> Rect {
        let dims = self.dims(pane, size);
        let min = pane.center() - vec2(self.center.x * dims.x, self.center.y * dims.y);
        Rect::from_min_size(min, dims)
    }

    /// Zoom and centre brought into range: the zoom within fit … [`MAX_ZOOM`], the picture
    /// covering the pane wherever it is larger than the pane (centred where it is smaller).
    pub fn clamped(mut self, pane: Rect, size: [usize; 2]) -> Self {
        self.zoom = if self.zoom.is_finite() { self.zoom.clamp(1.0, MAX_ZOOM) } else { 1.0 };
        let dims = self.dims(pane, size);
        let axis = |c: f32, pane_len: f32, len: f32| {
            let c = if c.is_finite() { c } else { 0.5 };
            if len <= pane_len + 0.5 {
                0.5
            } else {
                let half = pane_len / len / 2.0;
                c.clamp(half, 1.0 - half)
            }
        };
        self.center = vec2(axis(self.center.x, pane.width(), dims.x), axis(self.center.y, pane.height(), dims.y));
        self
    }

    /// Zoom by `factor` keeping the picture point under `at` (a point in `pane`) where it is.
    pub fn zoom_at(self, pane: Rect, size: [usize; 2], at: egui::Pos2, factor: f32) -> Self {
        let v = self.clamped(pane, size);
        let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
        v.zoom_at_clamped(pane, size, at, factor)
    }

    fn zoom_at_clamped(self, pane: Rect, size: [usize; 2], at: egui::Pos2, factor: f32) -> Self {
        let before = self.image_rect(pane, size);
        let u = vec2((at.x - before.left()) / before.width().max(1e-3), (at.y - before.top()) / before.height().max(1e-3));
        let zoomed = PanZoom { zoom: self.zoom * factor, ..self }.clamped(pane, size);
        let dims = zoomed.dims(pane, size);
        // the picture point u stays at `at`: pane.center - center·dims + u·dims = at
        let off = at - pane.center();
        let center = vec2(u.x - off.x / dims.x.max(1e-3), u.y - off.y / dims.y.max(1e-3));
        PanZoom { center, ..zoomed }.clamped(pane, size)
    }

    /// Move the picture by `delta` points (a drag).
    pub fn panned(self, pane: Rect, size: [usize; 2], delta: Vec2) -> Self {
        let v = self.clamped(pane, size);
        let dims = v.dims(pane, size);
        let center = v.center - vec2(delta.x / dims.x.max(1e-3), delta.y / dims.y.max(1e-3));
        PanZoom { center, ..v }.clamped(pane, size)
    }
}

/// The pointer over `pane` (a preview of a picture `size` pixels): grab hand, drag to pan, scroll
/// or pinch to zoom around the pointer, double-click for the whole picture. Returns the response;
/// `view` changes in place.
pub fn interact(ui: &mut egui::Ui, pane: Rect, id: &str, size: [usize; 2], view: &mut PanZoom) -> egui::Response {
    let resp = ui.interact(pane, egui::Id::new(("pan-zoom", id)), Sense::click_and_drag());
    crate::widgets::register(ui.ctx(), format!("preview:{id}"), pane);
    if size[0] == 0 || size[1] == 0 {
        return resp;
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        *view = view.panned(pane, size, resp.drag_delta());
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if resp.double_clicked() {
        *view = PanZoom::default();
    }
    if resp.hovered()
        && let Some(at) = ui.input(|i| i.pointer.hover_pos())
    {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = pinch * (scroll / 240.0).exp();
        if (factor - 1.0).abs() > 1e-4 && factor.is_finite() {
            *view = view.zoom_at(pane, size, at, factor);
            // the wheel zooms the preview, it doesn't scroll what's around it
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    resp
}

/// Draw `tex` (a picture `size` pixels) in `pane` framed by `view`, clipped to the pane.
pub fn paint(ui: &egui::Ui, pane: Rect, tex: egui::TextureId, size: [usize; 2], view: &PanZoom) {
    let r = view.clamped(pane, size).image_rect(pane, size);
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter_at(pane).image(tex, r, uv, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 200.0))
    }

    #[test]
    fn fit_is_whole_and_centred() {
        let v = PanZoom::default();
        let r = v.image_rect(pane(), [400, 200]);
        assert_eq!(r, Rect::from_min_size(pos2(100.0, 150.0), vec2(200.0, 100.0)));
        // panning a fitted picture does nothing
        assert_eq!(v.panned(pane(), [400, 200], vec2(50.0, 50.0)), v);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let size = [400, 400];
        let at = pos2(150.0, 260.0);
        let v = PanZoom::default().zoom_at(pane(), size, at, 4.0);
        assert_eq!(v.zoom, 4.0);
        let before = PanZoom::default().image_rect(pane(), size);
        let after = v.image_rect(pane(), size);
        let u = |r: Rect| ((at.x - r.left()) / r.width(), (at.y - r.top()) / r.height());
        let (a, b) = (u(before), u(after));
        assert!((a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4, "{a:?} {b:?}");
        // limits: never below fit, never past MAX_ZOOM, nothing non-finite
        assert_eq!(v.zoom_at(pane(), size, at, 0.01).zoom, 1.0);
        assert_eq!(v.zoom_at(pane(), size, at, 1000.0).zoom, MAX_ZOOM);
        assert_eq!(PanZoom { zoom: f32::NAN, center: vec2(f32::INFINITY, 0.5) }.clamped(pane(), size), PanZoom::default());
    }

    #[test]
    fn a_drag_pans_and_stops_at_the_edges() {
        let size = [400, 400];
        let v = PanZoom { zoom: 2.0, center: vec2(0.5, 0.5) };
        // dragging right shows more of the left: the centre moves left
        let p = v.panned(pane(), size, vec2(40.0, 0.0));
        assert!((p.center.x - 0.4).abs() < 1e-4, "{p:?}");
        let far = v.panned(pane(), size, vec2(4000.0, -4000.0));
        assert_eq!(far.center, vec2(0.25, 0.75));
        let r = far.image_rect(pane(), size);
        assert!(r.left() <= pane().left() + 0.01 && r.bottom() >= pane().bottom() - 0.01, "{r:?}");
    }
}
