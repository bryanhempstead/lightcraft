//! The Detail (loupe) view: the developed photo, before/after, zoom & pan, the filmstrip, and the
//! on-canvas tools (crop, brush/gradient masks, remove spots, white-balance picker).

use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{DevelopSettings, MaskShape};
use lightcraft_geom::{Affine, Point};
use lightcraft_pipeline::geometry::Frame;
use serde_json::json;

use crate::LightcraftApp;
use crate::render::Slot;
use crate::state::{BeforeAfter, RightPanel, Zoom};
use crate::theme::Tokens;
use crate::widgets::register;

/// An in-progress on-canvas gesture.
#[derive(Clone, Debug)]
pub enum Gesture {
    Brush {
        points: Vec<Point>,
    },
    Spot {
        points: Vec<Point>,
    },
    /// Remove tool: a ⌥-drag marquee from `a` (normalized) whose spots are deleted on release.
    SpotRect {
        a: Point,
    },
    /// Remove tool: dragging spot `spot`'s target (or its source).
    SpotMove {
        spot: usize,
        source: bool,
    },
    CropHandle {
        handle: u8,
        start: lightcraft_geom::Rect,
        angle: f64,
    },
    /// Crop tool: a plain drag inside the frame pans the photo under it, straight (the frame goes
    /// the other way; never a rotation).
    CropPan {
        start: lightcraft_geom::Rect,
        angle: f64,
    },
    /// Straighten tool: a line drawn along something that should be level (or plumb).
    StraightenLine {
        a: Pos2,
    },
    CropRotate {
        start_angle: f64,
        a0: f32,
    },
    Pan {
        start: (f32, f32),
        at: Pos2,
    },
    /// Dragging component `comp` of `mask`: its pin (`handle` 0) or a linear gradient's start (1)
    /// or end (2).
    MaskHandle {
        mask: u32,
        comp: usize,
        handle: u8,
        original_shape: MaskShape,
    },
    /// Guided Upright: a guide being drawn from `a` (normalized transformed coordinates).
    Guide {
        a: Point,
    },
    /// Red Eye: an ellipse being dragged from corner `a` (normalized).
    Eye {
        a: Point,
    },
    /// Targeted adjustment drag from `at` (normalized); `acc` = vertical travel not yet applied.
    Targeted {
        at: Point,
        acc: f32,
    },
}

/// Screen ↔ normalized-image mapping for the displayed frame.
#[derive(Clone, Copy)]
pub struct CanvasMap {
    rect: Rect,
    to_norm: Affine,
    from_norm: Affine,
    k: f32,
}

impl CanvasMap {
    fn new(frame: &Frame, rect: Rect) -> CanvasMap {
        let k = 8.0f32;
        let (w, h) = ((rect.width() * k).round().max(1.0) as usize, (rect.height() * k).round().max(1.0) as usize);
        let to_norm = frame.out_to_norm(w, h);
        CanvasMap { rect, to_norm, from_norm: to_norm.inverse().unwrap_or(Affine::IDENTITY), k }
    }
    pub fn norm(&self, p: Pos2) -> Point {
        self.to_norm.apply(Point::new(((p.x - self.rect.left()) * self.k) as f64, ((p.y - self.rect.top()) * self.k) as f64))
    }
    pub fn screen(&self, n: Point) -> Pos2 {
        let q = self.from_norm.apply(n);
        pos2(self.rect.left() + q.x as f32 / self.k, self.rect.top() + q.y as f32 / self.k)
    }
}

/// What every detail layer of the loupe shares this frame ([`detail_layer`]).
struct DetailAt {
    photo: PhotoId,
    ppp: f32,
    max_edge: f32,
    apply_crop: bool,
    interacting: bool,
    now: f64,
}

/// One view's native-resolution detail (`crate::tiles`; `(view, before)`): asked for the part of
/// the photo at `target` (its final rectangle; `img` where it is drawn now) visible in `clip`, and
/// drawn there over its preview. Whether the view has detail at this zoom.
#[allow(clippy::too_many_arguments)]
fn detail_layer(
    app: &mut LightcraftApp,
    p: &egui::Painter,
    at: &DetailAt,
    (view, before): (u8, bool),
    target: Rect,
    img: Rect,
    clip: Rect,
    overlay: lightcraft_pipeline::Overlay,
    proof: Option<lightcraft_pipeline::Proof>,
) -> bool {
    let want = crate::tiles::Want {
        view,
        photo: at.photo,
        img: target,
        area: clip,
        ppp: at.ppp,
        apply_crop: at.apply_crop,
        before,
        preview_long: (target.width().max(target.height()) * at.ppp).min(at.max_edge) as usize,
        interacting: at.interacting,
        overlay,
        proof,
        now: at.now,
    };
    if crate::tiles::update(app, &want).is_none() {
        return false;
    }
    if let Some(t) = app.renderer.tiles.get_mut(&view) {
        // never the previous look's tiles: over the (already new) preview they flashed the old edit
        // back in patches on every slider / dial move (Bryan, 2026-11-23)
        t.draw(&p.with_clip_rect(clip), img, clip);
    }
    true
}

pub(crate) fn fit_rect(area: Rect, aspect: f32, zoom: Zoom, img_px: [usize; 2], ppp: f32, pan: (f32, f32)) -> Rect {
    let zoom = match zoom {
        Zoom::Percent(p) if !p.is_finite() || p <= 0.0 => Zoom::Fit,
        _ => zoom,
    };
    let (aw, ah) = (area.width(), area.height());
    let (w, h) = match zoom {
        Zoom::Fit => {
            if aw / ah > aspect {
                (ah * aspect, ah)
            } else {
                (aw, aw / aspect)
            }
        }
        Zoom::Fill => {
            if aw / ah > aspect {
                (aw, aw / aspect)
            } else {
                (ah * aspect, ah)
            }
        }
        Zoom::Percent(p) => {
            // full-resolution pixels at p% (the photo's native width)
            let w = img_px[0] as f32 * p / 100.0 / ppp;
            (w, w / aspect)
        }
    };
    let pan = bounded_pan(area, vec2(w, h), pan);
    let c = pos2(area.center().x - (pan.0 - 0.5) * w, area.center().y - (pan.1 - 0.5) * h);
    Rect::from_center_size(c, vec2(w, h))
}

/// Fit a decoded image inside the frame calculated from catalog metadata. Unsupported RAWs can
/// only show their embedded camera JPEG, whose crop and aspect ratio can differ from that metadata.
pub(crate) fn fit_texture_rect(area: Rect, size: [usize; 2]) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let scale = (area.width() / w).min(area.height() / h);
    Rect::from_center_size(area.center(), vec2(w * scale, h * scale))
}

/// A render made for this view drawn pixel for pixel: when `rect` is within 2 % of the texture's own
/// size on screen, the texture's exact size, its corner on the screen's pixel grid. 🚨 Bryan,
/// 2026-11-23 ("the images look compressed … when I zoom it clears up"): a 2560 px render drawn
/// into 2550.5 px starting at x = 733.8 px blended every pixel with its neighbour — half the
/// sharpness of the same render shown 1:1.
pub(crate) fn pixel_exact(rect: Rect, size: [usize; 2], ppp: f32) -> Rect {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    if ppp.is_nan() || ppp <= 0.0 || ((rect.width() * ppp) / w - 1.0).abs() > 0.02 || ((rect.height() * ppp) / h - 1.0).abs() > 0.02 {
        return rect;
    }
    let want = vec2(w / ppp, h / ppp);
    let min = rect.center() - want / 2.0;
    Rect::from_min_size(pos2((min.x * ppp).round() / ppp, (min.y * ppp).round() / ppp), want)
}

/// Keep the viewport inside the photo; axes smaller than the viewport stay centred.
fn bounded_pan(area: Rect, size: egui::Vec2, pan: (f32, f32)) -> (f32, f32) {
    let bound = |visible: f32, image: f32, centre: f32| {
        let centre = if centre.is_finite() { centre } else { 0.5 };
        if image > visible.max(0.0) {
            let half = visible.max(0.0) / image / 2.0;
            centre.clamp(half, 1.0 - half)
        } else {
            0.5
        }
    };
    (bound(area.width(), size.x, pan.0), bound(area.height(), size.y, pan.1))
}

pub(crate) fn pan_image(app: &mut LightcraftApp, area: Rect, img: Rect, delta: egui::Vec2) {
    let centre = area.center() - img.min - delta;
    let pan = bounded_pan(area, img.size(), (centre.x / img.width().max(1.0), centre.y / img.height().max(1.0)));
    let _ = app.run("view.navigate", json!({"pan": pan}));
}

/// Points of scrolling egui makes of one mouse-wheel notch (`InputOptions::line_scroll_speed`).
const NOTCH: f32 = 40.0;

/// Does this scroll zoom (rather than pan)? `mouse`: it comes from a notched wheel; `alt`: ⌥ is
/// held (pans instead in the zooming modes).
pub(crate) fn scroll_zooms(mode: crate::state::WheelMode, mouse: bool, alt: bool) -> bool {
    use crate::state::WheelMode;
    match mode {
        WheelMode::Auto => mouse && !alt,
        WheelMode::Zoom => !alt,
        WheelMode::Pan | WheelMode::Photos => false,
    }
}

/// Native pinch (also ⌘-wheel zoom), the plain scroll wheel (zooms or pans as View ▸ Zoom ▸
/// Scroll Wheel says, anchored under the pointer) and two-finger scroll, scoped to this image view.
pub(crate) fn navigate_gesture(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, area: Rect, img: Rect, native: [usize; 2]) -> bool {
    // where the scroll being smoothed came from: a notched mouse wheel (lines) or a trackpad /
    // smooth-scrolling surface (points, with touch phases)
    if let Some(mouse) = ui.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::MouseWheel { unit, phase, .. } => Some(*unit != egui::MouseWheelUnit::Point && *phase == egui::TouchPhase::Move),
            _ => None,
        })
    }) {
        app.ui.wheel_mouse = mouse;
    }
    if !resp.hovered() {
        return false;
    }
    let (mut factor, mut delta, anchor, touch, alt) = ui.input(|i| {
        (
            i.zoom_delta(),
            i.translation_delta(),
            i.multi_touch().map(|t| t.center_pos).or(i.pointer.hover_pos()),
            i.multi_touch().is_some(),
            i.modifiers.alt,
        )
    });
    // Photos mode: each mouse-wheel notch over the photo steps a photo (down / towards you = next);
    // ⌘-scroll still zooms (egui makes it a zoom), a trackpad still pans
    if app.ui.wheel == crate::state::WheelMode::Photos && !touch && app.ui.wheel_mouse && factor == 1.0 {
        let notches: Vec<f32> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers, .. }
                        if *unit != egui::MouseWheelUnit::Point && !modifiers.command && delta.y != 0.0 =>
                    {
                        Some(delta.y)
                    }
                    _ => None,
                })
                .collect()
        });
        ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        for d in notches.into_iter().take(4) {
            let _ = app.run(if d < 0.0 { "library.next" } else { "library.previous" }, json!({}));
        }
        return false;
    }
    if !touch && delta != egui::Vec2::ZERO && scroll_zooms(app.ui.wheel, app.ui.wheel_mouse, alt) {
        // one notch = one step; a trackpad's points scale the same way, so it feels continuous.
        // Down / towards you zooms out. Sideways (⇧-wheel) still pans on a mouse.
        let step = 1.0 + app.ui.wheel_step.clamp(1, 400) as f32 / 100.0;
        factor *= step.powf((delta.y / NOTCH).clamp(-8.0, 8.0));
        delta = if app.ui.wheel_mouse { vec2(delta.x, 0.0) } else { egui::Vec2::ZERO };
    }
    if !factor.is_finite() || factor <= 0.0 || !delta.is_finite() || (factor == 1.0 && delta == egui::Vec2::ZERO) {
        return false;
    }
    let Some(anchor) = anchor.filter(|p| area.contains(*p)) else { return false };
    let ppp = ui.ctx().pixels_per_point();
    let aspect = img.width() / img.height().max(1.0);
    let fit = fit_rect(area, aspect, Zoom::Fit, native, ppp, (0.5, 0.5));
    let fit_pct = fit.width() * ppp * 100.0 / native[0].max(1) as f32;
    let mut zoom = app.ui.zoom;
    let size = if factor != 1.0 {
        // Start at the displayed scale (including Fit/Fill and an interrupted click animation).
        let pct = (img.width() * ppp * 100.0 / native[0].max(1) as f32 * factor).clamp(fit_pct.min(1600.0), 1600.0);
        zoom = if pct <= fit_pct { Zoom::Fit } else { Zoom::Percent(pct) };
        fit_rect(area, aspect, zoom, native, ppp, (0.5, 0.5)).size()
    } else {
        img.size()
    };
    // The image point under the pointer remains there as its size changes, then follows the pan.
    let point = (anchor - img.min) / img.size();
    let offset = area.center() - anchor - delta;
    let pan = bounded_pan(area, size, (point.x + offset.x / size.x.max(1.0), point.y + offset.y / size.y.max(1.0)));
    let _ = app.run("view.navigate", json!({"zoom": zoom, "pan": pan}));
    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    ui.ctx().request_repaint();
    true
}

/// Ease the loupe rect toward `target` while a click-zoom animation runs; otherwise follow it exactly.
fn animated_rect(ctx: &egui::Context, anim: &mut bool, target: Rect) -> Rect {
    let t = if *anim { 0.22 } else { 0.0 };
    let id = egui::Id::new("loupe_anim");
    let v = |k: &str, x: f32| ctx.animate_value_with_time(id.with(k), x, t);
    let (c, s) = (target.center(), target.size());
    let r = Rect::from_center_size(pos2(v("cx", c.x), v("cy", c.y)), vec2(v("w", s.x), v("h", s.y)));
    if *anim && (r.center() - c).abs().max_elem() < 0.5 && (r.size() - s).abs().max_elem() < 0.5 {
        *anim = false;
    }
    r
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    // the loupe steps through every photo, folded grid groups or not
    if !app.session.step_skip.is_empty() {
        app.session.step_skip.clear();
    }
    let t = Tokens::get(ui.ctx());
    let full = ui.max_rect();
    let fullscreen = app.ui.fullscreen;
    // the filmstrip runs along the window's bottom (panels::filmstrip), outside the canvas
    let canvas = full;
    app.canvas_rect = Some(canvas);
    let Some(id) = app.session.active() else {
        super::empty_message(ui, canvas, "No photo selected", "Choose a photo in the grid or filmstrip");
        return;
    };
    let Some(photo) = app.session.catalog.photo(id).cloned() else { return };
    let d = (*photo.develop).clone();
    // the full-screen preview shows the photo only: no tool overlays
    let right = if fullscreen { RightPanel::None } else { app.ui.right };
    let crop_tool = right == RightPanel::Crop;
    let frame = Frame::with_lens(photo.width.max(1) as usize, photo.height.max(1) as usize, &d, !crop_tool, photo.embedded_lens.as_ref());
    let aspect = frame.aspect() as f32;
    let ppp = ui.ctx().pixels_per_point();
    let area = canvas.shrink(if fullscreen {
        0.0
    } else if crop_tool {
        48.0
    } else {
        24.0
    });
    let max_edge = app.ui.settings.preview_edge.clamp(512, 8192) as f32;
    let native = [photo.width.max(1) as usize, photo.height.max(1) as usize];
    // two views (before, after): side by side or stacked
    let split = matches!(app.ui.before_after, BeforeAfter::SideBySide | BeforeAfter::TopBottom);
    let split_view = matches!(app.ui.before_after, BeforeAfter::Split | BeforeAfter::SplitTopBottom);
    let areas: Vec<Rect> = match app.ui.before_after {
        BeforeAfter::SideBySide => {
            let half = area.width() / 2.0 - 6.0;
            vec![
                Rect::from_min_size(area.min, vec2(half, area.height())),
                Rect::from_min_size(pos2(area.center().x + 6.0, area.top()), vec2(half, area.height())),
            ]
        }
        BeforeAfter::TopBottom => {
            let half = area.height() / 2.0 - 14.0;
            vec![
                Rect::from_min_size(area.min, vec2(area.width(), half)),
                Rect::from_min_size(pos2(area.left(), area.center().y + 14.0), vec2(area.width(), half)),
            ]
        }
        _ => vec![area],
    };
    let main_area = *areas.last().unwrap_or(&area);
    // (a crop dial's uncropped draft is drawn cut to the crop: its own aspect says nothing about the
    // frame it is drawn into, and following it stretched the crop out of shape)
    let main_is_cut = app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.photo == id && app.reframe_shows(t.key));
    let visible_texture_aspect = app
        .renderer
        .textures
        .get(&Slot::Main)
        .filter(|t| t.photo == id)
        .or_else(|| app.renderer.textures.get(&Slot::Preview).filter(|t| t.photo == id))
        .or_else(|| app.renderer.thumb(id))
        .map(|tex| tex.size[0].max(1) as f32 / tex.size[1].max(1) as f32)
        .filter(|_| !main_is_cut);
    // Follow the texture only where it really differs (an unsupported raw's embedded JPEG with
    // another crop): a texture's whole-pixel size is a little off the exact aspect, and the crop
    // tool's overlays need the frame's own geometry.
    let display_aspect = visible_texture_aspect.filter(|a| !crop_tool && (a / aspect - 1.0).abs() > 0.02).unwrap_or(aspect);
    let resp = ui.interact(canvas, egui::Id::new("loupe"), Sense::click_and_drag());
    let mut target_rect = fit_rect(main_area, display_aspect, app.ui.zoom, native, ppp, app.ui.pan);
    let mut img_rect = animated_rect(ui.ctx(), &mut app.ui.zoom_anim, target_rect);
    // the Crop tool's wheel punches the crop in and out (before the loupe's own wheel zoom sees it)
    crop_wheel(app, ui, &resp, crop_tool);
    let mut navigating = false;
    for &view_area in &areas {
        let view_img = if view_area == main_area { img_rect } else { fit_rect(view_area, display_aspect, app.ui.zoom, native, ppp, app.ui.pan) };
        if navigate_gesture(app, ui, &resp, view_area, view_img, native) {
            navigating = true;
            target_rect = fit_rect(main_area, display_aspect, app.ui.zoom, native, ppp, app.ui.pan);
            img_rect = target_rect;
            break;
        }
    }
    app.image_rect = Some(img_rect);
    // request renders: the loupe at display resolution (drafts during drags)
    let interacting = app.session.interaction.is_some();
    // a crop / straighten dial changes the geometry: every draft is a whole-pipeline render (no
    // stage can be kept), so it is made smaller to keep up with the detents
    let legacy = app.renderer.legacy();
    let reframing = !legacy && !crop_tool && app.console_drag.as_ref().is_some_and(|(_, c)| c.starts_with("crop."));
    let scale = if !interacting {
        1.0
    } else if reframing {
        DRAFT_SCALE_REFRAME
    } else {
        DRAFT_SCALE
    };
    // render at the final size: a click-zoom animation only changes how the result is drawn
    let want = (target_rect.width().max(target_rect.height()) * ppp * scale).min(max_edge) as usize;
    let (rw, rh) = if aspect >= 1.0 { (want, (want as f32 / aspect) as usize) } else { ((want as f32 * aspect) as usize, want) };
    // the long edge of the sharp render (what source a draft shares with it)
    let view_edge = (target_rect.width().max(target_rect.height()) * ppp).min(max_edge) as usize;
    if !(interacting && reframing) {
        app.reframe_pin = None; // the turn is over: the next one sizes its own draft
    }
    let job = if interacting && reframing {
        // one uncropped render for the whole turn; each detent's crop is drawn out of it (`draw`)
        let job = app.session.reframe_draft_job(id, want.max(8), view_edge, &mut app.reframe_pin);
        app.note_reframe_key(job.as_ref().map(|j| j.key));
        job
    } else if interacting && !legacy {
        app.session.view_draft_job(id, rw.max(8), rh.max(8), !crop_tool, view_edge)
    } else {
        app.session.loupe_job(id, rw.max(8), rh.max(8), !crop_tool).map(|j| if interacting { j.draft() } else { j })
    };
    if let Some(job) = job {
        app.ui.alt_down = ui.input(|i| i.modifiers.alt);
        let job = job.with_overlay(view_overlay(app, &d)).with_proof(app.ui.soft_proof.then_some(app.ui.proof));
        // an HDR edit on an HDR display: its HDR rendition (unless previewing the SDR one)
        let job = match hdr_headroom(app) {
            Some(h) if !app.ui.soft_proof => job.with_hdr_view(h),
            _ => job,
        };
        app.renderer.request(Slot::Main, job, 100);
    }
    // hovering a preset or profile: the photo with that look, shown instead of the loupe render
    // once it is ready (nothing is committed)
    let hover_key = match app.hover_preview.clone() {
        Some(h) if !interacting => app.session.preview_job(id, rw.max(8), rh.max(8), !crop_tool, &h.settings).map(|job| {
            let key = job.key;
            app.renderer.request(Slot::Hover, job, 105);
            (key, h.label)
        }),
        _ => None,
    };
    // the sharp render is on screen: get the draft size's stages ready for the next drag / dial
    // turn ([`crate::render::Renderer::warm`])
    if !interacting
        && !legacy
        && !app.renderer.is_pending(Slot::Main)
        && app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.photo == id && t.quick.is_none())
    {
        let dw = (target_rect.width().max(target_rect.height()) * ppp * DRAFT_SCALE).min(max_edge) as usize;
        let (ww, wh) = if aspect >= 1.0 { (dw, (dw as f32 / aspect) as usize) } else { ((dw as f32 * aspect) as usize, dw) };
        if let Some(job) = app.session.view_draft_job(id, ww.max(8), wh.max(8), !crop_tool, view_edge) {
            let job = job.with_overlay(view_overlay(app, &d)).with_proof(app.ui.soft_proof.then_some(app.ui.proof));
            app.renderer.warm(Slot::Main, job, PREFETCH_PRIORITY + 1);
        }
    }
    // once this photo is on screen: prepare its neighbours in filmstrip order (source decoded and
    // kept, view render cached) so stepping to them is instant
    if !interacting && !app.renderer.is_pending(Slot::Main) && app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.photo == id) {
        let ids = app.session.visible_cloned();
        if let Some(i) = ids.iter().position(|p| *p == id) {
            let next = ids.get(i + 1).copied();
            let prev = i.checked_sub(1).and_then(|j| ids.get(j)).copied();
            for (n, nid) in [next, prev].into_iter().enumerate() {
                let Some(nid) = nid else { continue };
                let Some(np) = app.session.catalog.photo(nid).cloned() else { continue };
                let nf = Frame::with_lens(np.width.max(1) as usize, np.height.max(1) as usize, &np.develop, !crop_tool, np.embedded_lens.as_ref());
                let na = nf.aspect() as f32;
                let nr = fit_rect(main_area, na, app.ui.zoom, [np.width.max(1) as usize, np.height.max(1) as usize], ppp, app.ui.pan);
                let nw = (nr.width().max(nr.height()) * ppp).min(max_edge) as usize;
                let (w, h) = if na >= 1.0 { (nw, (nw as f32 / na) as usize) } else { ((nw as f32 * na) as usize, nw) };
                if let Some(job) = app.session.loupe_job(nid, w.max(8), h.max(8), !crop_tool) {
                    app.renderer.prefetch(Slot::Prefetch(n as u8), job, PREFETCH_PRIORITY);
                }
            }
        }
    }
    // until the loupe has this photo: show its cached render / embedded preview / a thumbnail
    if app.renderer.textures.get(&Slot::Main).is_none_or(|t| t.photo != id)
        && let Some(q) = app.session.quick_view_job(id, want.max(8), !crop_tool)
    {
        app.renderer.request_quick(Slot::Preview, q, 110);
    }
    let show_before = app.ui.before_after == BeforeAfter::Original || ui.input(|i| i.key_down(egui::Key::Backslash));
    if (split || show_before || split_view)
        && let Some(job) = app.session.render_job(id, rw.max(8), rh.max(8), true, !crop_tool)
    {
        app.renderer.request(Slot::Before, job, 90);
    }
    let p = ui.painter_at(canvas);
    // the window's HDR output, lent to `draw` (which borrows the rest of `app` immutably)
    app.reframe_drawn.set(None);
    let hdr_shown = std::cell::RefCell::new(app.hdr_view.take());
    // what a view slot shows: its own render of this photo, else the stand-ins (no blank frame
    // between photos, and the full render replaces them in place)
    let draw = |slot: Slot, r: Rect| -> &'static str {
        let mine = |s: Slot| app.renderer.textures.get(&s).filter(|t| t.photo == id);
        let (tex, what) = if let Some(t) = mine(slot) {
            (t, "render")
        } else if let Some(t) = mine(Slot::Preview) {
            (t, t.quick.map(quick_name).unwrap_or("preview"))
        } else if let Some(t) = app.renderer.thumb(id) {
            (t, "thumb")
        } else {
            return "none";
        };
        let image_rect = pixel_exact(fit_texture_rect(r, tex.size), tex.size, ppp);
        // the HDR rendition where the window can show it (`hdr_view`), else the 8-bit texture
        let id = match (&tex.hdr, hdr_shown.borrow_mut().as_mut()) {
            (Some(px), Some(view)) => view.texture(&format!("{slot:?}"), px).unwrap_or(tex.tex.id()),
            _ => tex.tex.id(),
        };
        if slot == Slot::Main && app.reframe_shows(tex.key) {
            // a crop dial's uncropped draft: this detent's crop out of it
            let mesh = crop_mesh(id, r, &frame);
            app.reframe_drawn.set(Some((r, [0, 1, 2, 3].map(|i| mesh.vertices[i].uv), tex.size)));
            p.add(egui::Shape::mesh(mesh));
            return what;
        }
        p.image(id, image_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        what
    };
    // soft proofing: a paper-white surround and the proof's name, as Lightroom shows it
    if app.ui.soft_proof && !fullscreen {
        p.rect_filled(canvas, 0.0, Color32::from_gray(238));
        let label = crate::i18n::tr_format!("Proof Preview · {}", app.ui.proof.space.label());
        p.text(pos2(canvas.right() - 16.0, canvas.top() + 14.0), Align2::RIGHT_CENTER, label, t.font(12.5), Color32::from_gray(60));
    }
    let shown;
    if split {
        let br = fit_rect(areas[0], aspect, app.ui.zoom, native, ppp, app.ui.pan);
        draw(Slot::Before, br);
        shown = draw(Slot::Main, img_rect);
        p.text(pos2(br.left(), br.bottom() + 14.0), Align2::LEFT_CENTER, crate::i18n::tr("Before"), t.font(12.0), t.text_dim);
        p.text(pos2(img_rect.left(), img_rect.bottom() + 14.0), Align2::LEFT_CENTER, crate::i18n::tr("After"), t.font(12.0), t.text_dim);
    } else if show_before {
        shown = draw(Slot::Before, img_rect);
        p.text(pos2(img_rect.left() + 8.0, img_rect.top() + 14.0), Align2::LEFT_CENTER, crate::i18n::tr("Before"), t.font(12.0), t.text);
    } else if let Some((_key, label)) = &hover_key
        && let Some(tex) = app.renderer.textures.get(&Slot::Hover).filter(|t| t.photo == id)
    {
        let hover_rect = pixel_exact(fit_texture_rect(img_rect, tex.size), tex.size, ppp);
        p.image(tex.tex.id(), hover_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let g = p.layout_no_wrap(label.clone(), t.font(12.0), Color32::WHITE);
        let bg = Rect::from_min_size(hover_rect.left_top() + vec2(8.0, 8.0), g.size() + vec2(16.0, 8.0));
        p.rect_filled(bg, 4.0, Color32::from_black_alpha(160));
        p.galley(bg.min + vec2(8.0, 4.0), g, Color32::WHITE);
        shown = "hover";
    } else {
        shown = draw(Slot::Main, img_rect);
        if shown == "none" {
            match app.renderer.failure(Slot::Main) {
                Some(e) => {
                    p.text(
                        canvas.center() - vec2(0.0, 10.0),
                        Align2::CENTER_CENTER,
                        crate::i18n::tr("This photo can't be opened"),
                        t.semibold(14.0),
                        t.text,
                    );
                    p.text(canvas.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, e, t.font(12.0), t.text_dim);
                }
                None if app.ui.loading_message() => {
                    p.text(canvas.center(), Align2::CENTER_CENTER, crate::i18n::tr("Rendering…"), t.font(13.0), t.text_dim);
                }
                None => {}
            }
        } else if app.ui.loading_message()
            && !fullscreen
            && app.renderer.textures.get(&Slot::Main).is_some_and(|x| x.photo == id && x.quick.is_some())
        {
            // View Options ▸ Show message when loading or rendering photos: a stand-in (embedded
            // preview, thumbnail) shows while the real render is on its way (Lightroom's "Loading…")
            let g = p.layout_no_wrap(crate::i18n::tr("Loading…").to_string(), t.font(12.0), Color32::WHITE);
            let r = Rect::from_center_size(pos2(canvas.center().x, canvas.bottom() - 28.0), g.size() + vec2(20.0, 10.0));
            p.rect_filled(r, 5.0, Color32::from_black_alpha(170));
            p.galley(r.min + vec2(10.0, 5.0), g, Color32::WHITE);
            register(ui.ctx(), "canvas:loading", r);
        }
    }
    app.hdr_view = hdr_shown.into_inner();
    app.loupe_shown = Some((id, shown));
    // native-resolution detail over the preview at 1:1 and beyond the preview size (`tiles`):
    // the after view (or the before view while it is shown alone), and the before half of a
    // two-view or split Before/After (view 3); not over an HDR rendition
    let hdr_view = d.hdr.enabled && hdr_headroom(app).is_some();
    let detail_ok = !hdr_view && hover_key.is_none() && matches!(shown, "render" | "preview" | "thumb");
    let at = DetailAt { photo: id, ppp, max_edge, apply_crop: !crop_tool, interacting, now: ui.input(|i| i.time) };
    let mut used: Vec<u8> = Vec::new();
    if detail_ok {
        let (view, before) = if show_before && !split { (3u8, true) } else { (0u8, false) };
        let overlay = if before { lightcraft_pipeline::Overlay::None } else { view_overlay(app, &d) };
        let proof = app.ui.soft_proof.then_some(app.ui.proof);
        // a split view's after detail is only wanted on its after half
        let mut clip = main_area.intersect(canvas);
        if split_view && !before {
            let c = img_rect.center();
            if app.ui.before_after == BeforeAfter::Split {
                clip.min.x = clip.min.x.max(c.x);
            } else {
                clip.min.y = clip.min.y.max(c.y);
            }
        }
        if clip.is_positive() && detail_layer(app, &p, &at, (view, before), target_rect, img_rect, clip, overlay, proof) {
            used.push(view);
        }
        // side by side / stacked: the before view in its own half
        if split {
            let before_img = fit_rect(areas[0], aspect, app.ui.zoom, native, ppp, app.ui.pan);
            let none = lightcraft_pipeline::Overlay::None;
            if detail_layer(app, &p, &at, (3, true), before_img, before_img, areas[0].intersect(canvas), none, None) {
                used.push(3);
            }
        }
    }
    if split_view {
        let left_right = app.ui.before_after == BeforeAfter::Split;
        let mid = if left_right { img_rect.center().x } else { img_rect.center().y };
        if let Some(tex) = app.renderer.textures.get(&Slot::Before).filter(|t| t.photo == id) {
            let before_rect = pixel_exact(fit_texture_rect(img_rect, tex.size), tex.size, ppp);
            let (half, uv) = if left_right {
                (Rect::from_min_max(before_rect.min, pos2(before_rect.center().x, before_rect.bottom())), pos2(0.5, 1.0))
            } else {
                (Rect::from_min_max(before_rect.min, pos2(before_rect.right(), before_rect.center().y)), pos2(1.0, 0.5))
            };
            p.image(tex.tex.id(), half, Rect::from_min_max(pos2(0.0, 0.0), uv), Color32::WHITE);
        }
        // the before half's own detail, over its preview
        if detail_ok {
            let clip = main_area.intersect(canvas);
            let clip = if left_right {
                Rect::from_min_max(clip.min, pos2(clip.right().min(mid), clip.bottom()))
            } else {
                Rect::from_min_max(clip.min, pos2(clip.right(), clip.bottom().min(mid)))
            };
            let none = lightcraft_pipeline::Overlay::None;
            if clip.is_positive() && detail_layer(app, &p, &at, (3, true), target_rect, img_rect, clip, none, None) {
                used.push(3);
            }
        }
        let line = if left_right {
            [pos2(mid, img_rect.top()), pos2(mid, img_rect.bottom())]
        } else {
            [pos2(img_rect.left(), mid), pos2(img_rect.right(), mid)]
        };
        p.line_segment(line, Stroke::new(1.5, Color32::WHITE));
    }
    for v in [0u8, 3] {
        if !used.contains(&v)
            && let Some(t) = app.renderer.tiles.get_mut(&v)
        {
            t.look = None;
        }
    }
    if app.ui.show_clipping && !show_before && !fullscreen {
        clipping_overlay(app, &p, img_rect);
    }
    register(ui.ctx(), "canvas:image", img_rect);
    // LRC-VIEW-LOUPEOVERLAY: grid / guides / layout image / composition guides / frames on the
    // photo as shown (the crop tool draws its own guides on the uncropped frame)
    let guide_drag = !crop_tool && !show_before && super::crop_overlay::loupe(app, ui, &resp, img_rect, main_area.intersect(canvas));
    // a crop dial turned outside the crop tool: the thirds over the (cropped) photo while it moves
    if !crop_tool && !show_before && crop_grid_on(app, ui.ctx()) {
        let p = ui.painter_at(main_area.intersect(canvas));
        let guide = Stroke::new(0.8, crate::theme::CROP.gamma_multiply(0.6));
        for line in super::crop_overlay::paths(crate::state::CropOverlay::Thirds, 0, img_rect.width(), img_rect.height()) {
            p.add(egui::Shape::line(
                line.into_iter().map(|(x, y)| img_rect.min + vec2(x * img_rect.width(), y * img_rect.height())).collect(),
                guide,
            ));
        }
    }
    let map = CanvasMap::new(&frame, img_rect);
    brief_info(app, ui.ctx(), id);
    info_overlay(app, &p, canvas, &photo);
    if !fullscreen {
        filter_pill(app, ui, canvas);
    }
    if app.ui.face_boxes || app.ui.draw_face {
        match region_overlay(ui, &p, &map, &photo, d.orientation, app.ui.draw_face && !fullscreen) {
            Some(RegionEdit::Remove(index)) => {
                let _ = app.run("photo.removeRegion", json!({"id": id.0, "index": index}));
            }
            Some(RegionEdit::Resize(index, r)) => {
                let _ = app.run("photo.setRegion", json!({"id": id.0, "index": index, "rect": {"x0": r.x0, "y0": r.y0, "x1": r.x1, "y1": r.y1}}));
            }
            None => {}
        }
        face_names(app, ui, &map, &photo, d.orientation);
    }
    // a fine grid while a transform (geometry) slider is dragged, to judge verticals
    if app.ui.dragging_control.as_deref().is_some_and(|c| c.starts_with("geometry.")) {
        let n = 12;
        let stroke = Stroke::new(1.0, Color32::from_white_alpha(70));
        for i in 1..n {
            let fx = img_rect.left() + img_rect.width() * i as f32 / n as f32;
            p.line_segment([pos2(fx, img_rect.top()), pos2(fx, img_rect.bottom())], stroke);
        }
        let rows = ((n as f32) * img_rect.height() / img_rect.width()).round().max(2.0) as usize;
        for i in 1..rows {
            let fy = img_rect.top() + img_rect.height() * i as f32 / rows as f32;
            p.line_segment([pos2(img_rect.left(), fy), pos2(img_rect.right(), fy)], stroke);
        }
    }
    // Cursor selection must also run on gesture frames, when tool input is suppressed.
    if !matches!(right, RightPanel::Crop | RightPanel::Masking | RightPanel::Remove | RightPanel::RedEye) {
        general_cursor(app, ui, &resp, img_rect, main_area);
    }
    if !navigating && !guide_drag {
        match right {
            RightPanel::Crop => crop_overlay(app, ui, &resp, &map, &frame, &d, id),
            RightPanel::Masking => mask_overlay(app, ui, &resp, &map, &d),
            RightPanel::Remove => remove_overlay(app, ui, &resp, &map, &d),
            RightPanel::RedEye => eye_overlay(app, ui, &resp, &map, &d),
            // Draw Face Region: a drag on the photo draws a face box (clicks still zoom)
            _ if app.ui.draw_face && !fullscreen => {
                draw_face_tool(app, ui, &resp, &map, &photo, d.orientation);
                if !resp.dragged() && !resp.drag_stopped() {
                    general_interaction(app, ui, &resp, &map, img_rect, main_area, native, aspect);
                }
            }
            _ => general_interaction(app, ui, &resp, &map, img_rect, main_area, native, aspect),
        }
    }
    // drawn and hit-tested above the loupe and its tools: clicks on it pan
    navigator(app, ui, canvas, img_rect, id);
    denoise_hint(app, ui, canvas, img_rect, &photo);
    // the original is gone (moved, renamed, its disk not connected): say so over the photo, with
    // the ways to point the library at it again (a cached answer, checked off the UI thread)
    let missing = !fullscreen && missing_banner(app, ui, canvas, &photo, shown != "none");
    if let Some(why) = photo.preview_only.as_deref().filter(|_| !fullscreen) {
        preview_only_pill(ui, canvas, why, if missing { 66.0 } else { 0.0 });
    }
    // a video: the played / scrubbed frame and its control bar (LR-VID-PLAY)
    if photo.kind == lightcraft_catalog::MediaKind::Video {
        crate::video_ui::loupe(app, ui, canvas, img_rect, &photo);
    }
    resp.context_menu(|ui| {
        ui.menu_button(crate::i18n::tr("Zoom"), |ui| {
            for (label, cmd) in [("Fit", "view.zoomFit"), ("100%", "view.zoom100"), ("Zoom In", "view.zoomIn"), ("Zoom Out", "view.zoomOut")] {
                if ui.button(label).clicked() {
                    let _ = app.run(cmd, json!({}));
                }
            }
        });
        ui.separator();
        super::grid::context_menu(app, ui, id);
    });
}

/// View Options' Show Briefly When Photo Changes: note when the loupe changes to photo `id`, and
/// whether the brief info overlay shows this frame (a repaint is asked for when it ends).
fn brief_info(app: &mut LightcraftApp, ctx: &egui::Context, id: lightcraft_catalog::PhotoId) {
    let now = ctx.input(|i| i.time);
    let brief = &mut app.ui.brief_info;
    if brief.photo != Some(id) {
        brief.photo = Some(id);
        brief.since = now;
    }
    let left = crate::state::BRIEF_INFO_SECS - (now - brief.since);
    let on = app.ui.loupe_info().briefly != crate::state::InfoOverlay::Off;
    let brief = &mut app.ui.brief_info;
    brief.shown = on && left > 0.0;
    if brief.shown {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left.max(0.01)));
    }
}

/// One line of the info overlay for `photo` (`None`: nothing to say, the line is left out).
pub(crate) fn info_line(field: crate::state::LoupeInfoField, photo: &lightcraft_catalog::Photo) -> Option<String> {
    use crate::state::LoupeInfoField as F;
    let m = &photo.meta;
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    // Leica bodies write "not selected" for an uncoded lens: that's no lens name
    let lens = if m.lens.trim().eq_ignore_ascii_case("not selected") { "" } else { m.lens.trim() };
    match field {
        F::None => None,
        F::FileName => Some(photo.file_name.clone()),
        F::FileAndCopyName => Some(match &photo.copy_name {
            Some(c) => format!("{} / {c}", photo.file_name),
            None => photo.file_name.clone(),
        }),
        F::CaptureDate => Some(pretty_date(photo.captured.as_deref().unwrap_or(&photo.imported))),
        F::Dimensions => Some(format!("{} × {}", photo.width, photo.height)),
        F::Exposure => {
            let mut exp = Vec::new();
            if !m.shutter.is_empty() {
                exp.push(format!("{} s", m.shutter));
            }
            if let Some(a) = m.aperture {
                exp.push(format!("f/{a:.1}"));
            }
            if let Some(i) = m.iso {
                exp.push(format!("ISO {i}"));
            }
            if let Some(f) = m.focal_mm {
                exp.push(format!("{f:.0} mm"));
            }
            Some(if exp.is_empty() { "No exposure information".into() } else { exp.join("  ") })
        }
        F::Camera => text(camera_model(&m.camera)),
        F::Lens => text(lens),
        F::CameraAndLens => text(&[camera_model(&m.camera), lens].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ")),
        F::Title => text(&m.title),
        F::Caption => text(&m.caption),
        F::RatingAndLabel => {
            let stars = "★".repeat(photo.rating.min(5) as usize);
            let label = photo.label.map(|l| format!("{l:?}")).unwrap_or_default();
            text(&[stars.as_str(), label.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("  "))
        }
        F::Keywords => text(&m.keywords.iter().map(|k| k.rsplit('|').next().unwrap_or(k)).collect::<Vec<_>>().join(", ")),
        F::Folder => match &photo.source {
            lightcraft_catalog::Source::File { path } => {
                std::path::Path::new(path).parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
            }
            _ => None,
        },
    }
}

/// The info overlay's lines for `photo` (`view.infoOverlay`: Info 1 / Info 2 as View Options
/// chose them; Show Briefly shows one for a moment when the photo changes), or `None` when off.
pub(crate) fn info_overlay_lines(app: &LightcraftApp, photo: &lightcraft_catalog::Photo) -> Option<Vec<String>> {
    use crate::state::InfoOverlay;
    // (Library and Develop each have their own overlay and lines: LRC-DEV-VIEWOPTIONS)
    let info = app.ui.loupe_info();
    let brief = app.ui.info_mode() == InfoOverlay::Off && app.ui.brief_info.shown;
    let mode = if brief { info.briefly } else { app.ui.info_mode() };
    let fields = match mode {
        InfoOverlay::Off => return None,
        InfoOverlay::Basic => &info.info1,
        InfoOverlay::Exposure => &info.info2,
    };
    let lines: Vec<String> = fields.iter().filter_map(|f| info_line(*f, photo)).collect();
    (!lines.is_empty()).then_some(lines)
}

/// The info overlay at the canvas' top left.
fn info_overlay(app: &LightcraftApp, p: &egui::Painter, canvas: Rect, photo: &lightcraft_catalog::Photo) {
    let Some(lines) = info_overlay_lines(app, photo) else { return };
    let t = Tokens::get(p.ctx());
    let mut y = canvas.top() + 12.0;
    for (i, l) in lines.iter().enumerate() {
        let font = if i == 0 { t.semibold(15.0) } else { t.font(12.5) };
        // PLACEHOLDER: the colour given to `galley` is the one drawn (black shadow, white text) —
        // laid out white, the "shadow" was a second white copy and every line looked doubled
        let g = p.layout_no_wrap(l.clone(), font, Color32::PLACEHOLDER);
        let at = pos2(canvas.left() + 14.0, y);
        // a soft shadow keeps the text readable on bright photos
        p.galley(at + vec2(0.0, 1.0), g.clone(), Color32::from_black_alpha(150));
        p.galley(at, g.clone(), Color32::WHITE);
        y += g.size().y + 3.0;
    }
    register(p.ctx(), "canvas:infoOverlay", Rect::from_min_max(canvas.min, pos2(canvas.left() + 320.0, y)));
}

/// The camera as Lightroom shows it (the Exif model): see [`lightcraft_engine::cameras::display_name`].
pub(crate) fn camera_model(camera: &str) -> &str {
    lightcraft_engine::cameras::display_name(camera)
}

#[cfg(test)]
mod camera_model_tests {
    #[test]
    fn camera_model_drops_a_repeated_make() {
        use super::camera_model;
        assert_eq!(camera_model("Canon Canon EOS R6"), "Canon EOS R6");
        assert_eq!(camera_model("Leica Camera AG LEICA M (Typ 262)"), "LEICA M (Typ 262)");
        assert_eq!(camera_model("RICOH IMAGING COMPANY, LTD. RICOH GR III"), "RICOH GR III");
        assert_eq!(camera_model("Canon EOS R6"), "Canon EOS R6");
        assert_eq!(camera_model("SONY ILCE-7M3"), "SONY ILCE-7M3");
        assert_eq!(camera_model("  "), "");
    }
}

/// What the user did to a face box this frame.
enum RegionEdit {
    /// The × was clicked.
    Remove(usize),
    /// A handle drag or (with the Draw Face Region tool) a move ended: the region's new box (normalized, upright
    /// frame).
    Resize(usize, lightcraft_geom::Rect),
}

/// Handles of a box: (x, y) as fractions of its width and height, and the cursor they show.
const REGION_HANDLES: [(f32, f32, egui::CursorIcon); 8] = [
    (0.0, 0.0, egui::CursorIcon::ResizeNwSe),
    (0.5, 0.0, egui::CursorIcon::ResizeVertical),
    (1.0, 0.0, egui::CursorIcon::ResizeNeSw),
    (1.0, 0.5, egui::CursorIcon::ResizeHorizontal),
    (1.0, 1.0, egui::CursorIcon::ResizeNwSe),
    (0.5, 1.0, egui::CursorIcon::ResizeVertical),
    (0.0, 1.0, egui::CursorIcon::ResizeNeSw),
    (0.0, 0.5, egui::CursorIcon::ResizeHorizontal),
];

/// A screen box (`n`) back to the normalized, upright frame a region is stored in.
fn upright_rect(map: &CanvasMap, orient: lightcraft_geom::Orientation, n: Rect) -> lightcraft_geom::Rect {
    let (a, b) = (map.norm(n.min), map.norm(n.max));
    let shown = lightcraft_geom::Rect {
        x0: a.x.min(b.x).clamp(0.0, 1.0),
        y0: a.y.min(b.y).clamp(0.0, 1.0),
        x1: a.x.max(b.x).clamp(0.0, 1.0),
        y1: a.y.max(b.y).clamp(0.0, 1.0),
    };
    orient.inverse().map_norm_rect(shown)
}

/// Where a region's box is on screen.
fn region_screen_rect(map: &CanvasMap, orient: lightcraft_geom::Orientation, r: lightcraft_geom::Rect) -> Rect {
    let norm = orient.map_norm_rect(r);
    Rect::from_two_pos(map.screen(Point::new(norm.x0, norm.y0)), map.screen(Point::new(norm.x1, norm.y1)))
}

/// The dark name label of a face box: centred above it (below it when there is no room), kept inside `clip`. The
/// label's rect, the caret's tip and base y, and the laid-out name.
fn name_label(p: &egui::Painter, t: &Tokens, rect: Rect, name: &str, clip: Rect) -> (Rect, f32, f32, std::sync::Arc<egui::Galley>) {
    let g = p.layout_no_wrap(name.to_string(), t.font(13.0), Color32::from_gray(225));
    let (pad, caret) = (vec2(14.0, 7.0), 5.0);
    let size = g.size() + pad * 2.0;
    let above = rect.top() - caret - size.y >= clip.top();
    let (top, tip, base) = if above {
        (rect.top() - caret - size.y, rect.top() - 1.0, rect.top() - caret - 1.0)
    } else {
        (rect.bottom() + caret, rect.bottom() + 1.0, rect.bottom() + caret + 1.0)
    };
    let left = (rect.center().x - size.x / 2.0).clamp(clip.left(), (clip.right() - size.x).max(clip.left()));
    (Rect::from_min_size(pos2(left, top), size), tip, base, g)
}

/// Face/pet/focus regions read from XMP (MWG-RS) or found or drawn here, drawn as boxes over the photo. Hovering a
/// box shows a × in its corner and eight resize handles; with the Draw Face Region tool on (`tool`) the box itself
/// can be dragged to move it. A drag previews the new box live and is reported once, on release (one undo step);
/// the × reports the region to remove. Both are catalog-only edits: LightCraft doesn't write unnamed regions to
/// XMP. Regions are stored on the upright (EXIF-oriented) photo; `orient` is the user's Rotate / Flip on top of it,
/// which the loupe's normalized frame includes.
fn region_overlay(
    ui: &egui::Ui,
    p: &egui::Painter,
    map: &CanvasMap,
    photo: &lightcraft_catalog::Photo,
    orient: lightcraft_geom::Orientation,
    tool: bool,
) -> Option<RegionEdit> {
    let t = Tokens::get(p.ctx());
    let clip = p.clip_rect();
    let pointer = ui.input(|i| i.pointer.hover_pos());
    // the box being dragged: (region index, its box so far)
    let drag_key = egui::Id::new("region-drag");
    let live: Option<(usize, lightcraft_geom::Rect)> = ui.data(|d| d.get_temp(drag_key));
    let mut edit = None;
    for (index, r) in photo.meta.regions.iter().enumerate() {
        let dragging = live.filter(|(i, _)| *i == index);
        let rect = region_screen_rect(map, orient, dragging.map_or(r.rect, |(_, n)| n));
        // white with a black keyline just outside it, so the box shows on any background
        p.rect_stroke(rect.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_black_alpha(190)), StrokeKind::Outside);
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Outside);
        // the Draw Face Region tool: drag the box to move it (under the handles, which win at the edges)
        if tool && rect.width() > 4.0 && rect.height() > 4.0 {
            let body = ui.interact(rect, egui::Id::new(("region-body", index)), Sense::drag());
            register(ui.ctx(), format!("regionBody:{index}"), rect);
            if body.hovered() || body.dragged() {
                ui.ctx().set_cursor_icon(if body.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
            }
            if body.dragged() {
                let d = body.drag_delta();
                let base = dragging.map_or(rect, |_| rect);
                let mut moved = base.translate(d);
                // stay on the photo
                let img = Rect::from_two_pos(map.screen(Point::new(0.0, 0.0)), map.screen(Point::new(1.0, 1.0)));
                let shift = vec2(
                    (img.left() - moved.left()).max(0.0) + (img.right() - moved.right()).min(0.0),
                    (img.top() - moved.top()).max(0.0) + (img.bottom() - moved.bottom()).min(0.0),
                );
                moved = moved.translate(shift);
                ui.data_mut(|dd| dd.insert_temp(drag_key, (index, upright_rect(map, orient, moved))));
                ui.ctx().request_repaint();
            }
            if body.drag_stopped() {
                if let Some((i, new)) = ui.data(|d| d.get_temp::<(usize, lightcraft_geom::Rect)>(drag_key)) {
                    edit = Some(RegionEdit::Resize(i, new));
                }
                ui.data_mut(|d| d.remove_temp::<(usize, lightcraft_geom::Rect)>(drag_key));
            }
        }
        if dragging.is_some() || (live.is_none() && pointer.is_some_and(|h| rect.expand(8.0).contains(h))) {
            for (h, (fx, fy, cursor)) in REGION_HANDLES.iter().enumerate() {
                let c = pos2(rect.left() + fx * rect.width(), rect.top() + fy * rect.height());
                let resp = ui.interact(Rect::from_center_size(c, vec2(14.0, 14.0)), egui::Id::new(("region-handle", index, h)), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(*cursor);
                }
                let sq = Rect::from_center_size(c, vec2(7.0, 7.0));
                p.rect_filled(sq, 0.0, if resp.hovered() || resp.dragged() { Color32::from_rgb(120, 190, 255) } else { Color32::WHITE });
                p.rect_stroke(sq, 0.0, Stroke::new(1.0, Color32::from_black_alpha(220)), StrokeKind::Outside);
                if resp.dragged()
                    && let Some(pp) = resp.interact_pointer_pos()
                {
                    // move the dragged edge(s) to the pointer, never closer than 12 px to the opposite one
                    let mut n = rect;
                    if *fx == 0.0 {
                        n.min.x = pp.x.min(n.max.x - 12.0);
                    } else if *fx == 1.0 {
                        n.max.x = pp.x.max(n.min.x + 12.0);
                    }
                    if *fy == 0.0 {
                        n.min.y = pp.y.min(n.max.y - 12.0);
                    } else if *fy == 1.0 {
                        n.max.y = pp.y.max(n.min.y + 12.0);
                    }
                    ui.data_mut(|d| d.insert_temp(drag_key, (index, upright_rect(map, orient, n))));
                    ui.ctx().request_repaint();
                }
                if resp.drag_stopped() {
                    if let Some((i, new)) = ui.data(|d| d.get_temp::<(usize, lightcraft_geom::Rect)>(drag_key)) {
                        edit = Some(RegionEdit::Resize(i, new));
                    }
                    ui.data_mut(|d| d.remove_temp::<(usize, lightcraft_geom::Rect)>(drag_key));
                }
            }
        }
        if dragging.is_none() && live.is_none() && pointer.is_some_and(|h| rect.expand(3.0).contains(h)) {
            let xr = Rect::from_center_size(pos2(rect.right() - 13.0, rect.top() + 13.0), vec2(16.0, 16.0));
            let resp = ui.interact(xr, egui::Id::new(("region-x", index)), Sense::click());
            register(ui.ctx(), format!("regionRemove:{index}"), xr);
            let fill = Color32::from_black_alpha(if resp.hovered() { 235 } else { 190 });
            p.rect_filled(xr, 2.0, fill);
            p.rect_stroke(xr, 2.0, Stroke::new(1.0, Color32::from_white_alpha(120)), StrokeKind::Inside);
            let (c, m) = (xr.center(), 3.5);
            let cross = Stroke::new(1.4, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(210) });
            p.line_segment([c - vec2(m, m), c + vec2(m, m)], cross);
            p.line_segment([c - vec2(m, -m), c + vec2(m, -m)], cross);
            if resp.on_hover_text(crate::i18n::tr("Remove this face box (undo with Edit ▸ Undo)")).clicked() {
                edit = Some(RegionEdit::Remove(index));
            }
        }
        let Some(name) = &r.name else { continue };
        let (label, tip, base, g) = name_label(p, &t, rect, name, clip);
        let fill = Color32::from_rgba_unmultiplied(56, 56, 56, 235);
        p.rect_filled(label, 3.0, fill);
        p.rect_stroke(label, 3.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Inside);
        let caret = 5.0;
        let cx = rect.center().x.clamp(label.left() + caret + 4.0, label.right() - caret - 4.0);
        p.add(egui::Shape::convex_polygon(vec![pos2(cx - caret, base), pos2(cx + caret, base), pos2(cx, tip)], fill, Stroke::NONE));
        p.galley(label.min + vec2(14.0, 7.0), g, Color32::from_gray(225));
    }
    edit
}

/// Lightroom's Draw Face Region tool: a drag on the photo (not on a box) draws a new face box, which is added (one
/// undo step) and gets the name field at once. Esc leaves the tool (`shortcuts::handle`).
fn draw_face_tool(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    map: &CanvasMap,
    photo: &lightcraft_catalog::Photo,
    orient: lightcraft_geom::Orientation,
) {
    let key = egui::Id::new("face-draw-start");
    let img = Rect::from_two_pos(map.screen(Point::new(0.0, 0.0)), map.screen(Point::new(1.0, 1.0)));
    if resp.hovered() && ui.input(|i| i.pointer.hover_pos()).is_some_and(|h| img.contains(h)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    // from where the button went down (the drag only counts as started once the pointer has moved)
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or_else(|| resp.interact_pointer_pos()).filter(|p| img.contains(*p))
    {
        ui.data_mut(|d| d.insert_temp(key, p));
    }
    let Some(start) = ui.data(|d| d.get_temp::<egui::Pos2>(key)) else { return };
    let end = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.hover_pos())).unwrap_or(start);
    let end = pos2(end.x.clamp(img.left(), img.right()), end.y.clamp(img.top(), img.bottom()));
    let r = Rect::from_two_pos(start, end);
    let p = ui.painter();
    p.rect_stroke(r.expand(1.0), 0.0, Stroke::new(1.0, Color32::from_black_alpha(190)), StrokeKind::Outside);
    p.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
    register(ui.ctx(), "faceDraw:box", r);
    ui.ctx().request_repaint();
    if resp.drag_stopped() || !ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.remove_temp::<egui::Pos2>(key));
        if r.width() >= 8.0 && r.height() >= 8.0 {
            let u = upright_rect(map, orient, r);
            let params = json!({"id": photo.id.0, "rect": {"x0": u.x0, "y0": u.y0, "x1": u.x1, "y1": u.y1}});
            match app.run("photo.addFace", params) {
                Ok(v) => {
                    if let Some(i) = v["index"].as_u64().and_then(|i| usize::try_from(i).ok()) {
                        app.ui.face_name_edit = Some((i, String::new()));
                    }
                }
                Err(e) => app.toast_error(ui.ctx(), e),
            }
        }
    }
}

/// Under each unnamed face box: `?` (Lightroom Classic's name field on a face region); a click opens a field, Enter
/// names the face (a person keyword, as in the People view). A named box's label opens the same field with its name:
/// Enter renames that face, an empty name takes the name off.
fn face_names(app: &mut LightcraftApp, ui: &mut egui::Ui, map: &CanvasMap, photo: &lightcraft_catalog::Photo, orient: lightcraft_geom::Orientation) {
    let t = Tokens::get(ui.ctx());
    let clip = ui.clip_rect();
    for (index, r) in photo.meta.regions.iter().enumerate() {
        if r.kind != lightcraft_meta::RegionKind::Face {
            continue;
        }
        let rect = region_screen_rect(map, orient, r.rect);
        let editing = app.ui.face_name_edit.as_ref().is_some_and(|(i, _)| *i == index);
        if editing {
            let w = rect.width().max(140.0);
            let fr = Rect::from_min_size(pos2(rect.center().x - w / 2.0, rect.bottom() + 8.0), vec2(w, 24.0));
            let Some((_, text)) = app.ui.face_name_edit.as_mut() else { continue };
            let edit = ui.put(fr, egui::TextEdit::singleline(text).hint_text("name").font(t.font(12.5)).desired_width(w - 8.0));
            register(ui.ctx(), format!("faceName:{index}"), fr);
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            if edit.lost_focus() {
                let name = text.trim().to_string();
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                app.ui.face_name_edit = None;
                let face = json!([{"photo": photo.id.0, "index": index}]);
                let r = match (enter, name.is_empty(), r.name.as_deref()) {
                    (true, false, old) if old != Some(name.as_str()) => Some(app.run("people.name", json!({"faces": face, "name": name}))),
                    (true, true, Some(_)) => Some(app.run("people.unname", json!({"faces": face}))),
                    _ => None,
                };
                if let Some(Err(e)) = r {
                    app.toast_error(ui.ctx(), e);
                }
            }
            continue;
        }
        if let Some(name) = &r.name {
            // the label drawn by `region_overlay`: a click renames
            let (label, ..) = name_label(ui.painter(), &t, rect, name, clip);
            let resp = ui.interact(label, egui::Id::new(("face-label", index)), Sense::click());
            register(ui.ctx(), format!("faceLabel:{index}"), label);
            if resp.on_hover_text(crate::i18n::tr("Click to rename this face")).clicked() {
                app.ui.face_name_edit = Some((index, name.clone()));
            }
            continue;
        }
        let q = Rect::from_center_size(pos2(rect.center().x, rect.bottom() + 16.0), vec2(26.0, 20.0));
        let resp = ui.interact(q, egui::Id::new(("face-name", index)), Sense::click());
        register(ui.ctx(), format!("faceNameButton:{index}"), q);
        let p = ui.painter();
        p.rect_filled(q, 3.0, Color32::from_rgba_unmultiplied(56, 56, 56, if resp.hovered() { 250 } else { 215 }));
        p.text(q.center(), Align2::CENTER_CENTER, "?", t.semibold(12.0), Color32::from_gray(225));
        if resp.on_hover_text(crate::i18n::tr("Name this person")).clicked() {
            app.ui.face_name_edit = Some((index, String::new()));
        }
    }
}

/// A small pill at the loupe's top left naming the active filters: the filmstrip and Next / Previous
/// follow them, so a filter must never be invisible here. A click clears them all. It sits in the
/// canvas margin, so the photo does not move.
fn filter_pill(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect) {
    let chips = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog);
    if chips.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let label = chips.iter().map(|c| c.label.as_str()).collect::<Vec<_>>().join("  ·  ");
    let g = p.layout_no_wrap(crate::i18n::tr_format!("Filtered: {label}", label = label), t.font(12.0), Color32::from_gray(225));
    let w = (g.size().x + 40.0).min((canvas.width() - 24.0).max(60.0));
    let r = Rect::from_min_size(canvas.min + vec2(12.0, 3.0), vec2(w, 20.0));
    let resp = ui.interact(r, egui::Id::new("loupe-filter-pill"), Sense::click());
    register(ui.ctx(), "loupe:filters", r);
    p.rect_filled(r, 10.0, Color32::from_black_alpha(if resp.hovered() { 225 } else { 185 }));
    let text_clip = Rect::from_min_max(r.min, pos2(r.right() - 24.0, r.max.y));
    p.with_clip_rect(text_clip).galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, Color32::from_gray(225));
    let (c, m) = (pos2(r.right() - 13.0, r.center().y), 3.0);
    let cross = Stroke::new(1.3, if resp.hovered() { Color32::WHITE } else { Color32::from_gray(190) });
    p.line_segment([c - vec2(m, m), c + vec2(m, m)], cross);
    p.line_segment([c - vec2(m, -m), c + vec2(m, -m)], cross);
    if resp.on_hover_text(crate::i18n::tr("Click to clear these filters")).clicked() {
        let _ = app.run("library.clearFilter", json!({}));
    }
}

/// The original file of a library photo is missing (`availability`: moved, renamed, its disk not
/// connected): a banner at the canvas' top centre — "Original missing — showing a preview" (or
/// "nothing to show" without one), that edits still apply and export needs the original — with
/// loc. (Locate Missing File…) and fldr. (Find Missing Folder… for the folder it was in).
/// Registered as `notice:missing:loupe`. Returns whether it is shown.
fn missing_banner(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, photo: &lightcraft_catalog::Photo, showing: bool) -> bool {
    let lightcraft_catalog::Source::File { path } = &photo.source else { return false };
    if !app.session.media.availability.is_offline(path) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    let head =
        if showing { crate::i18n::tr("Original missing — showing a preview") } else { crate::i18n::tr("Original missing — no preview to show") };
    let note = crate::i18n::tr("edits apply; export needs the original");
    let p = ui.painter_at(canvas);
    let g1 = p.layout_no_wrap(head.to_string(), t.semibold(12.5), Color32::WHITE);
    let g2 = p.layout_no_wrap(format!("· {note}"), t.font(12.0), Color32::from_gray(200));
    let chip = |label: &str| p.layout_no_wrap(label.to_string(), t.semibold(11.5), Color32::WHITE);
    let (c1, c2) = (chip("locate."), chip("folder."));
    let chip_w = |g: &std::sync::Arc<egui::Galley>| (g.size().x + 18.0).max(37.0);
    let (w1, w2) = (chip_w(&c1), chip_w(&c2));
    // icon 16 · 8 · head · 8 · note · 12 · chip · 8 · chip, 12 in from each end
    let full_w = 12.0 + 16.0 + 8.0 + g1.size().x + 8.0 + g2.size().x + 12.0 + w1 + 8.0 + w2 + 12.0;
    let narrow = full_w > canvas.width() - 24.0;
    let w = if narrow { full_w - g2.size().x - 8.0 } else { full_w };
    let r = Rect::from_min_size(pos2(canvas.center().x - w / 2.0, canvas.top() + 32.0), vec2(w, 34.0));
    p.rect_filled(r, 8.0, Color32::from_black_alpha(205));
    p.rect_stroke(r, 8.0, Stroke::new(1.0, t.caution.gamma_multiply(0.7)), StrokeKind::Inside);
    let mut x = r.left() + 12.0;
    crate::icons::paint(&p, Rect::from_center_size(pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), crate::icons::Icon::Info, t.caution);
    x += 16.0 + 8.0;
    let gy = |g: &std::sync::Arc<egui::Galley>| r.center().y - g.size().y / 2.0;
    p.galley(pos2(x, gy(&g1)), g1.clone(), Color32::WHITE);
    x += g1.size().x + 8.0;
    if !narrow {
        p.galley(pos2(x, gy(&g2)), g2.clone(), Color32::from_gray(200));
        x += g2.size().x + 8.0;
    }
    x += 4.0;
    register(ui.ctx(), "notice:missing:loupe", r);
    let tip = crate::i18n::tr("The original file isn't where the library expects it (moved, renamed, or its disk isn't connected).");
    ui.interact(r, egui::Id::new("missing-banner"), Sense::hover()).on_hover_text(format!("{tip}\n{path}"));
    let mut clicked = None;
    for (id, g, cw, tip) in [
        ("missingLocate", c1, w1, "Locate Missing File… — point this photo at its file"),
        ("missingFolder", c2, w2, "Find Missing Folder… — point every photo from its folder at where the folder went"),
    ] {
        let b = Rect::from_min_size(pos2(x, r.center().y - 12.0), vec2(cw, 24.0));
        let resp = ui.interact(b, egui::Id::new(("missing-banner", id)), Sense::click());
        register(ui.ctx(), format!("button:{id}"), b);
        p.rect(b, 4.0, if resp.hovered() { t.hover } else { t.button }, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
        p.galley(b.center() - g.size() / 2.0, g, Color32::WHITE);
        if resp.on_hover_text(crate::i18n::tr(tip)).clicked() {
            clicked = Some(id);
        }
        x = b.right() + 8.0;
    }
    match clicked {
        Some("missingLocate") => {
            if let Err(e) = app.run("photo.locate", json!({})) {
                app.toast(ui.ctx(), e);
            }
        }
        Some(_) => {
            let folder = std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(to) = app.services.pick_folder.as_mut().and_then(|f| f()) {
                match app.run("folder.locate", json!({"path": folder, "to": to})) {
                    Ok(r) => app.toast(
                        ui.ctx(),
                        format!("{} {} · {} {}", r["relinked"], crate::i18n::tr("relinked"), r["stillMissing"], crate::i18n::tr("still missing")),
                    ),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
        }
        None => {}
    }
    true
}

/// A raw shown from its embedded JPEG (issue #10): a pill at the canvas' top centre saying so;
/// hovering it explains (registered as `notice:previewOnly:loupe`).
fn preview_only_pill(ui: &mut egui::Ui, canvas: Rect, reason: &str, below: f32) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let text = crate::i18n::tr_format!("Preview only — editing the camera's embedded JPEG ({})", crate::widgets::preview_only_variant(reason));
    let g = p.layout_no_wrap(text, t.font(12.0), Color32::WHITE);
    let size = vec2(g.size().x + 40.0, 26.0);
    let r = Rect::from_min_size(pos2(canvas.center().x - size.x / 2.0, canvas.top() + 12.0 + below), size);
    p.rect_filled(r, 13.0, Color32::from_black_alpha(185));
    crate::icons::paint(&p, Rect::from_center_size(pos2(r.left() + 16.0, r.center().y), vec2(15.0, 15.0)), crate::icons::Icon::Info, t.caution);
    p.galley(pos2(r.left() + 29.0, r.center().y - g.size().y / 2.0), g, Color32::WHITE);
    register(ui.ctx(), "notice:previewOnly:loupe", r);
    ui.interact(r, egui::Id::new("preview-only-pill"), Sense::hover()).on_hover_text(crate::widgets::preview_only_explanation(reason));
}

/// `2026-09-30T12:00:00` → `2026-09-30 12:00`.
fn pretty_date(iso: &str) -> String {
    let d = iso.replacen('T', " ", 1);
    d.get(..16).map(str::to_string).unwrap_or(d)
}

/// Size of the Navigator mini map (points).
const NAV_W: f32 = 180.0;

/// denoise.: a raw shot at Settings ▸ General ▸ denoise:'s ISO or above offers Denoise in a corner
/// of the preview — `denoise.` opens the Enhance box with Denoise ticked (amount and the rest to
/// play with there), `hide.` puts it away for this photo. Not on Enhanced copies, in full screen
/// or behind a dialog; above the Navigator when both want the bottom right.
fn denoise_hint(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, img: Rect, photo: &lightcraft_catalog::Photo) {
    let s = &app.ui.settings;
    let iso = photo.meta.iso.unwrap_or(0);
    if !s.denoise_hint
        || iso < s.denoise_hint_iso.max(1)
        || !lightcraft_engine::enhance::AVAILABLE
        || app.ui.fullscreen
        || app.ui.dialog.is_some()
        || !photo.develops_raw()
        || photo.file_name.contains("-Enhanced-")
        || app.denoise_hidden.contains(&photo.id)
    {
        return;
    }
    use crate::state::HintCorner as C;
    let corner = s.denoise_hint_corner;
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    let nav = if corner == C::BottomRight && app.ui.navigator && zoomed { NAV_W + 8.0 } else { 0.0 };
    let (pivot, at) = match corner {
        C::TopLeft => (egui::Align2::LEFT_TOP, canvas.left_top() + vec2(16.0, 16.0)),
        C::TopRight => (egui::Align2::RIGHT_TOP, canvas.right_top() + vec2(-16.0, 16.0)),
        C::BottomLeft => (egui::Align2::LEFT_BOTTOM, canvas.left_bottom() + vec2(16.0, -16.0)),
        C::BottomRight => (egui::Align2::RIGHT_BOTTOM, canvas.right_bottom() + vec2(-16.0, -16.0 - nav)),
    };
    let t = Tokens::get(ui.ctx());
    let mut act: Option<(&str, serde_json::Value)> = None;
    let r = egui::Area::new(egui::Id::new("denoise-hint")).pivot(pivot).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(Color32::from_black_alpha(185)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(format!("ISO {iso}")).color(Color32::WHITE).font(t.font(12.0)));
                if crate::widgets::text_button(ui, "denoiseHint", "denoise.", true).on_hover_text(crate::i18n::tr("Shot at a high ISO: open Enhance with Denoise to clean it up")).clicked() {
                    act = Some(("dialog.enhance", json!({"denoise": true})));
                }
                if crate::widgets::text_button(ui, "denoiseHintHide", "hide.", false).on_hover_text(crate::i18n::tr("Not for this photo (Settings ▸ General ▸ denoise: turns it off)")).clicked() {
                    act = Some(("view.hideDenoiseHint", json!({"id": photo.id.0})));
                }
            });
        });
    });
    register(ui.ctx(), "notice:denoiseHint", r.response.rect);
    if let Some((cmd, p)) = act {
        if cmd == "dialog.enhance" {
            let _ = app.run("library.select", json!({"ids": [photo.id.0]}));
        }
        if let Err(e) = app.run(cmd, p) {
            app.toast_soon(e.to_string(), 4.0);
        }
    }
}

/// The Navigator: while zoomed in, a mini map of the photo at the canvas' bottom right with the
/// visible region outlined; click or drag on it to pan.
fn navigator(app: &mut LightcraftApp, ui: &mut egui::Ui, canvas: Rect, img: Rect, id: PhotoId) {
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    if !app.ui.navigator || !zoomed || app.ui.fullscreen {
        return;
    }
    let tex = app.renderer.thumb(id).or_else(|| app.renderer.textures.get(&Slot::Main).filter(|t| t.photo == id));
    let aspect = img.width() / img.height().max(1.0);
    let (w, h) = if aspect >= 1.0 { (NAV_W, NAV_W / aspect) } else { (NAV_W * aspect, NAV_W) };
    let frame = Rect::from_min_size(pos2(canvas.right() - w - 16.0, canvas.bottom() - h - 16.0), vec2(w, h));
    let p = ui.painter_at(canvas);
    p.rect_filled(frame.expand(5.0), 4.0, Color32::from_black_alpha(190));
    match tex {
        Some(t) => p.image(t.tex.id(), frame, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE),
        None => p.rect_filled(frame, 0.0, Color32::from_gray(40)),
    };
    // the visible part of the photo, in normalized image coordinates
    let vis = canvas.intersect(img);
    let n = |q: Pos2| pos2((q.x - img.left()) / img.width(), (q.y - img.top()) / img.height());
    let (a, b) = (n(vis.min), n(vis.max));
    let to_nav = |q: Pos2| pos2(frame.left() + q.x * frame.width(), frame.top() + q.y * frame.height());
    let view = Rect::from_min_max(to_nav(a), to_nav(b));
    p.rect_stroke(view, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Inside);
    p.rect_stroke(view.expand(1.5), 0.0, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
    register(ui.ctx(), "canvas:navigator", frame);
    let resp = ui.interact(frame.expand(5.0), egui::Id::new("navigator"), Sense::click_and_drag());
    if (resp.clicked() || resp.dragged())
        && let Some(q) = resp.interact_pointer_pos()
    {
        // centre the view on the point under the pointer
        let u = ((q.x - frame.left()) / frame.width()).clamp(0.0, 1.0);
        let v = ((q.y - frame.top()) / frame.height()).clamp(0.0, 1.0);
        app.ui.pan = (u, v);
    }
}

/// Neighbour prefetch: below on-screen thumbnails, above background thumbnail refreshes.
const PREFETCH_PRIORITY: u32 = 4;
/// Loupe drafts (slider drags, console turns) at this share of the view's pixel size.
const DRAFT_SCALE: f32 = 0.6;
/// … and while a console dial moves the crop (every draft re-renders the whole pipeline).
const DRAFT_SCALE_REFRAME: f32 = 0.35;

/// The crop of `frame` drawn into `r` out of a texture of the whole uncropped frame
/// ([`lightcraft_engine::Session::reframe_draft_job`]): each corner of `r` takes the texture
/// coordinate of the crop corner it shows (straighten, flips and Constrain Crop included).
fn crop_mesh(tex: egui::TextureId, r: Rect, frame: &Frame) -> egui::Mesh {
    let to_src = frame.crop.output_to_source(frame.ow, frame.oh, 1.0, 1.0);
    let (ow, oh) = (frame.ow.max(1.0), frame.oh.max(1.0));
    let uv = |u: f64, v: f64| {
        let u = if frame.flip_h { 1.0 - u } else { u };
        let v = if frame.flip_v { 1.0 - v } else { v };
        let q = to_src.apply(lightcraft_geom::Point::new(u, v));
        pos2((q.x / ow) as f32, (q.y / oh) as f32)
    };
    let mut mesh = egui::Mesh::with_texture(tex);
    for (corner, (u, v)) in [(r.left_top(), (0.0, 0.0)), (r.right_top(), (1.0, 0.0)), (r.right_bottom(), (1.0, 1.0)), (r.left_bottom(), (0.0, 1.0))] {
        mesh.vertices.push(egui::epaint::Vertex { pos: corner, uv: uv(u, v), color: Color32::WHITE });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    mesh
}

fn quick_name(q: lightcraft_engine::media::QuickSource) -> &'static str {
    use lightcraft_engine::media::QuickSource;
    match q {
        QuickSource::Cached => "cached",
        QuickSource::Embedded => "embedded",
        QuickSource::Small => "small",
    }
}

/// Targeted adjustment tool: dragging up/down on the photo raises/lowers the tone-curve region or
/// the colour-mixer bands under the press point (`develop.targeted`, one call per whole step, all
/// in one interaction = one undo step).
fn targeted_drag(app: &mut LightcraftApp, resp: &egui::Response, map: &CanvasMap, target: &str) {
    if resp.drag_started()
        && let Some(q) = resp.interact_pointer_pos()
    {
        let _ = app.run("develop.beginInteraction", json!({"label": "Targeted Adjustment"}));
        app.gesture = Some(Gesture::Targeted { at: map.norm(q), acc: 0.0 });
    }
    if resp.dragged()
        && let Some(Gesture::Targeted { at, acc }) = &mut app.gesture
    {
        *acc -= resp.drag_delta().y * 0.5;
        let step = acc.trunc();
        if step != 0.0 {
            *acc -= step;
            let at = *at;
            let mut p = json!({"target": target, "x": at.x, "y": at.y, "delta": step});
            if target == "curve" && app.ui.curve_channel != "parametric" {
                p["channel"] = json!(app.ui.curve_channel);
            }
            let _ = app.run("develop.targeted", p);
        }
    }
    if resp.drag_stopped() && matches!(app.gesture, Some(Gesture::Targeted { .. })) {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
}

/// The diagnostic overlay the loupe shows (an Alt-drag preview, the selected mask, Point Color's
/// visualized range, Visualize Spots).
pub(crate) fn view_overlay(app: &LightcraftApp, d: &DevelopSettings) -> lightcraft_pipeline::Overlay {
    use lightcraft_pipeline::{MaskView, Overlay};
    if app.ui.fullscreen {
        return Overlay::None;
    }
    // Alt/Option-drag on Exposure / Highlights / Whites, Shadows / Blacks, Sharpening, Noise
    // Reduction: Lightroom's temporary preview while the slider moves
    if app.ui.alt_down
        && let Some(o) = app.ui.dragging_control.as_deref().and_then(Overlay::for_control)
    {
        return o;
    }
    if app.ui.right == RightPanel::Masking
        && app.ui.mask_overlay
        && let Some(m) = app.session.active_mask.and_then(|id| d.masks.iter().find(|m| m.id == id))
        && !m.components.is_empty()
    {
        return Overlay::Mask {
            id: m.id.min(u16::MAX as u32) as u16,
            view: MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default(),
            color: app.ui.mask_overlay_color,
            opacity: app.ui.mask_overlay_opacity.clamp(0.0, 100.0).round() as u8,
        };
    }
    if app.ui.right == RightPanel::Remove && app.ui.visualize_spots {
        return Overlay::Spots(app.ui.spots_threshold.clamp(0.0, 100.0).round() as u8);
    }
    let edit = app.ui.right == RightPanel::Edit;
    if edit && app.ui.point_color_visualize && app.ui.flyout_open("pointColor") && app.ui.point_color < d.point_colors.len() {
        return Overlay::PointColorRange(app.ui.point_color as u8);
    }
    if edit && app.ui.hdr_visualize && d.hdr.enabled {
        // what lies beyond the HDR display's headroom is marked red (none on an SDR display)
        let display = app.hdr_view.as_ref().and_then(|v| v.headroom());
        return Overlay::HdrRange(lightcraft_pipeline::visualize::headroom_tenths(display));
    }
    Overlay::None
}

fn clipping_overlay(app: &LightcraftApp, p: &egui::Painter, r: Rect) {
    // Highlight clipped regions using the histogram's extremes isn't spatial; show a subtle frame hint.
    if let Some(h) = app.renderer.textures.get(&Slot::Main).and_then(|t| t.histogram.as_ref()) {
        let (lo, hi) = h.clipping();
        if hi > 0.003 {
            p.rect_stroke(r, 0.0, Stroke::new(2.0, Color32::from_rgb(255, 60, 60)), StrokeKind::Outside);
        }
        if lo > 0.003 {
            p.rect_stroke(r.expand(3.0), 0.0, Stroke::new(2.0, Color32::from_rgb(60, 120, 255)), StrokeKind::Outside);
        }
    }
}

fn general_cursor(app: &LightcraftApp, ui: &egui::Ui, resp: &egui::Response, img: Rect, canvas: Rect) {
    // A panel or overlay under the pointer keeps its own cursor.
    if !resp.dragged() && !resp.hovered() {
        return;
    }
    let cursor = match app.ui.tool.as_str() {
        // the White Balance Selector draws its own dropper at the pointer (general_interaction)
        "wbPicker" => egui::CursorIcon::None,
        "fringePicker" | "colorRange" | "pointColor" | "lensFocus" => egui::CursorIcon::Crosshair,
        tool if tool.starts_with("lensBrush:") => egui::CursorIcon::Crosshair,
        tool if tool.starts_with("tat:") => egui::CursorIcon::ResizeVertical,
        _ if img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0 => {
            if resp.dragged() {
                egui::CursorIcon::Grabbing
            } else {
                egui::CursorIcon::Grab
            }
        }
        _ => return,
    };
    ui.ctx().set_cursor_icon(cursor);
}

fn general_interaction(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    map: &CanvasMap,
    img: Rect,
    canvas: Rect,
    native: [usize; 2],
    aspect: f32,
) {
    if crate::panels::lens_blur::canvas(app, resp, |q| map.norm(q)) {
        return;
    }
    if app.ui.tool == "wbPicker" {
        // say what the tool wants, on the photo (Bryan couldn't find the selector, 2026-11-23)
        let t = Tokens::get(ui.ctx());
        let p = ui.painter_at(canvas);
        let g =
            p.layout_no_wrap(crate::i18n::tr("click something white or neutral grey · Esc to stop").to_string(), t.font(12.5), egui::Color32::WHITE);
        let pill = Rect::from_center_size(egui::pos2(img.center().x, img.top() + 26.0), g.size() + vec2(28.0, 14.0));
        p.rect_filled(pill, pill.height() / 2.0, egui::Color32::from_black_alpha(175));
        p.galley(pill.center() - g.size() / 2.0, g, egui::Color32::WHITE);
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            app.ui.tool.clear();
            return;
        }
        // the pointer is a dropper, its tip on the spot it samples (outlined to show on any photo)
        if let Some(pos) = resp.hover_pos() {
            let s = 26.0;
            let k = s / 20.0;
            let r = Rect::from_min_size(pos - vec2(4.0 * k, 16.0 * k), vec2(s, s));
            let fg = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("wbDropper")));
            for d in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
                crate::icons::paint(&fg, r.translate(d), crate::icons::Icon::Picker, egui::Color32::from_black_alpha(200));
            }
            crate::icons::paint(&fg, r, crate::icons::Icon::Picker, egui::Color32::WHITE);
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let _ = app.run("develop.wbPick", json!({"x": n.x, "y": n.y}));
            app.ui.tool.clear();
        }
        return;
    }
    if app.ui.tool == "fringePicker" {
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let _ = app.run("develop.defringePick", json!({"x": n.x, "y": n.y}));
            app.ui.tool.clear();
        }
        return;
    }
    if let Some(target) = app.ui.tool.strip_prefix("tat:").map(str::to_string) {
        targeted_drag(app, resp, map, &target);
        return;
    }
    // LrSuperKeys' HSL keys (Bryan's SpeedKeys.xml: H, J, K, B): hold one and drag up/down on the
    // photo = targeted hue / saturation / luminance / B&W mix of the colour under the press point.
    // The target is kept for the whole drag, so letting go of the key mid-drag still ends it.
    if app.ui.tool.is_empty() {
        let sk_id = egui::Id::new("superkey-target");
        let held: Option<&'static str> = if ui.ctx().egui_wants_keyboard_input() || crate::widgets::slider_typing(ui.ctx()) {
            None
        } else {
            ui.input(|i| {
                [(egui::Key::H, "hue"), (egui::Key::J, "sat"), (egui::Key::K, "lum"), (egui::Key::B, "lum")]
                    .into_iter()
                    .find(|(k, _)| i.key_down(*k) && !i.modifiers.command && !i.modifiers.alt && !i.modifiers.ctrl)
                    .map(|(_, t)| t)
            })
        };
        let dragging: Option<String> = ui.data(|m| m.get_temp(sk_id));
        let target = dragging.or_else(|| held.map(str::to_string));
        if let Some(target) = target {
            if resp.drag_started() {
                ui.data_mut(|m| m.insert_temp(sk_id, target.clone()));
            }
            if held.is_some() || resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            targeted_drag(app, resp, map, &target);
            if resp.drag_stopped() || !(resp.dragged() || resp.drag_started()) && held.is_none() {
                ui.data_mut(|m| m.remove::<String>(sk_id));
            }
            if resp.dragged() || resp.drag_started() || resp.drag_stopped() {
                return;
            }
        }
    }
    if app.ui.tool == "colorRange" {
        // click: sample the colour for the selected mask's colour range; ⇧-click adds (up to 5)
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let add = ui.input(|i| i.modifiers.shift);
            if let Err(e) = app.run("mask.sampleColor", json!({"x": n.x, "y": n.y, "add": add})) {
                app.toast(ui.ctx(), e);
            }
        }
        return;
    }
    if app.ui.tool == "pointColor" {
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            if let Ok(r) = app.run("pointColor.pick", json!({"x": n.x, "y": n.y})) {
                app.ui.point_color = r["index"].as_u64().unwrap_or(0) as usize;
            }
            app.ui.tool.clear();
        }
        return;
    }
    // a click zooms to the chosen click level (Fill, 1:1, 2:1 …; the toolbar's click. menu),
    // centred on the clicked point, and the next click goes back to Fit; drag pans when zoomed
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    if resp.clicked() {
        let target = crate::state::click_zoom_target(app.ui.click_zoom);
        let zoom_in = app.ui.zoom == Zoom::Fit || (!zoomed && !app.ui.zoom_anim && app.ui.zoom != target);
        if zoom_in {
            if let Some(q) = resp.interact_pointer_pos() {
                let u = ((q.x - img.left()) / img.width()).clamp(0.0, 1.0);
                let v = ((q.y - img.top()) / img.height()).clamp(0.0, 1.0);
                app.ui.pan = (u, v);
            }
            app.ui.zoom = target;
        } else {
            app.ui.zoom = Zoom::Fit;
        }
        app.ui.zoom_anim = true;
    }
    if zoomed && resp.dragged() {
        pan_image(app, canvas, img, resp.drag_delta());
    }
    let _ = (native, aspect);
}

// ------------------------------------------------------------------------ crop

/// How much one wheel notch punches the crop: up (away from you) goes in by this factor.
const CROP_WHEEL_STEP: f32 = 0.94;

/// The scroll wheel in the Crop tool (and only there): up punches the crop in, down out, about the
/// crop's centre and keeping its shape (`crop.nudge`), instead of zooming the loupe. The notches of a
/// burst are one undo step, closed [`crate::CROP_WHEEL_HOLD`] after the last. Pinch and ⌘-wheel (a zoom
/// to egui), ⌥-scroll (pan) and a trackpad's pan of a zoomed loupe keep their loupe meanings.
fn crop_wheel(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, crop_tool: bool) {
    let now = ui.input(|i| i.time);
    // close a finished burst (also when the tool was left meanwhile)
    if let Some(t) = app.crop_wheel {
        let idle = now - t;
        if !crop_tool || idle >= crate::CROP_WHEEL_HOLD {
            app.crop_wheel = None;
            let _ = app.run("develop.endInteraction", json!({}));
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(crate::CROP_WHEEL_HOLD - idle));
        }
    }
    if !crop_tool || !resp.hovered() || app.gesture.is_some() {
        return;
    }
    let (scroll, zoom, alt, touch, mouse) = ui.input(|i| {
        let mouse = i.events.iter().rev().find_map(|e| match e {
            egui::Event::MouseWheel { unit, phase, .. } => Some(*unit != egui::MouseWheelUnit::Point && *phase == egui::TouchPhase::Move),
            _ => None,
        });
        (i.smooth_scroll_delta.y, i.zoom_delta(), i.modifiers.alt || i.modifiers.command, i.multi_touch().is_some(), mouse)
    });
    let mouse = mouse.unwrap_or(app.ui.wheel_mouse);
    // a trackpad's scroll of a zoomed loupe is its pan; everything else vertical is the crop's
    let pans_loupe = !mouse && app.ui.zoom != crate::state::Zoom::Fit;
    if scroll == 0.0 || !scroll.is_finite() || zoom != 1.0 || alt || touch || pans_loupe {
        return;
    }
    let k = CROP_WHEEL_STEP.powf((scroll / NOTCH).clamp(-8.0, 8.0));
    if app.crop_wheel.is_none() && app.session.interaction.is_none() {
        let _ = app.run("develop.beginInteraction", json!({"label": "Crop"}));
        app.crop_wheel = Some(now);
    }
    if app.crop_wheel.is_some() {
        app.crop_wheel = Some(now);
    }
    let _ = app.run("crop.nudge", json!({"scale": (1.0 - k as f64) * 100.0}));
    app.crop_touch = Some(std::time::Instant::now());
    ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    ui.ctx().request_repaint();
}

fn crop_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, frame: &Frame, d: &DevelopSettings, id: PhotoId) {
    if app.ui.tool == "guidedUpright" {
        guided_overlay(app, ui, resp, map, frame, d);
        return;
    }
    if app.ui.tool == "straighten" {
        straighten_overlay(app, ui, resp, false);
        return;
    }
    // hold ⌘ and drag: draw a straighten line without leaving the crop tool
    let cmd_drag = resp.drag_started() && ui.input(|i| i.modifiers.command);
    if cmd_drag || matches!(app.gesture, Some(Gesture::StraightenLine { .. })) {
        straighten_overlay(app, ui, resp, true);
        return;
    }
    if resp.hovered() && ui.input(|i| i.modifiers.command) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    let quad = frame_crop_quad(d, frame);
    let pts: Vec<Pos2> = quad.iter().map(|q| map.screen(*q)).collect();
    let p = ui.painter();
    // darken outside the crop
    let img = map.rect;
    let shade = Color32::from_black_alpha(150);
    let mut mesh = egui::epaint::Mesh::default();
    let outer = [img.left_top(), img.right_top(), img.right_bottom(), img.left_bottom()];
    for i in 0..4 {
        let a = outer[i];
        let b = outer[(i + 1) % 4];
        let c = pts[(i + 1) % 4];
        let dd = pts[i];
        let base = mesh.vertices.len() as u32;
        for v in [a, b, c, dd] {
            mesh.vertices.push(egui::epaint::Vertex { pos: v, uv: Pos2::ZERO, color: shade });
        }
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    p.add(mesh);
    p.add(egui::Shape::closed_line(pts.clone(), Stroke::new(1.0, crate::theme::CROP)));
    // overlay guides
    let lerp = |a: Pos2, b: Pos2, t: f32| a + (b - a) * t;
    let guide = Stroke::new(0.8, crate::theme::CROP.gamma_multiply(0.6));
    // (u, v) across / down the (rotated) crop frame
    let at = |(u, v): (f32, f32)| pts[0] + (pts[1] - pts[0]) * u + (pts[3] - pts[0]) * v;
    let (cw, ch) = (pts[0].distance(pts[1]), pts[0].distance(pts[3]));
    // the guides while the crop is being adjusted, gone 0.8 s after (Bryan, 2026-11-23)
    if resp.dragged() {
        app.crop_touch = Some(std::time::Instant::now());
    }
    if crop_grid_on(app, ui.ctx()) {
        for line in super::crop_overlay::paths(app.ui.crop_overlay, app.ui.crop_overlay_orient, cw, ch) {
            p.add(egui::Shape::line(line.into_iter().map(at).collect(), guide));
        }
    }
    // handles: 0..3 corners, 4..7 edges (top, right, bottom, left)
    let handles: Vec<Pos2> = (0..8).map(|i| if i < 4 { pts[i] } else { lerp(pts[i - 4], pts[(i - 3) % 4], 0.5) }).collect();
    for (i, h) in handles.iter().enumerate() {
        register(ui.ctx(), format!("cropHandle:{i}"), Rect::from_center_size(*h, vec2(14.0, 14.0)));
        let s = if i < 4 { 12.0 } else { 9.0 };
        p.rect_filled(Rect::from_center_size(*h, vec2(s, 3.0)), 0.0, crate::theme::CROP);
        p.rect_filled(Rect::from_center_size(*h, vec2(3.0, s)), 0.0, crate::theme::CROP);
    }
    let inside = |q: Pos2| {
        let n = map.norm(q);
        let s = to_straight(n, d.crop.geometry.angle, frame);
        d.crop.geometry.rect.contains(Point::new(s.x.clamp(-1.0, 2.0), s.y))
    };
    // the handle under `q`: a tick first, else anywhere along an edge (as in Lightroom)
    let pick = |q: Pos2| {
        handles.iter().position(|h| h.distance(q) < 12.0).or_else(|| {
            // the nearest edge, so a thin crop's bottom band doesn't resolve to its top edge
            (0..4)
                .map(|e| {
                    let (a, b) = (pts[e], pts[(e + 1) % 4]);
                    let t = ((q - a).dot(b - a) / (b - a).length_sq().max(1e-6)).clamp(0.0, 1.0);
                    (e, (a + (b - a) * t).distance(q))
                })
                .filter(|&(_, dist)| dist < 8.0)
                .min_by(|x, y| x.1.total_cmp(&y.1))
                .map(|(e, _)| e + 4)
        })
    };
    if let Some(hq) = resp.hover_pos() {
        // ⇧ moves the frame itself (even from an edge); plain drags pan the photo under it
        let free = ui.input(|i| i.modifiers.shift) && (inside(hq) || pick(hq).is_some());
        let panning = matches!(app.gesture, Some(Gesture::CropPan { .. }));
        ui.ctx().set_cursor_icon(match pick(hq) {
            _ if free => egui::CursorIcon::Move,
            Some(0 | 2) => egui::CursorIcon::ResizeNwSe,
            Some(1 | 3) => egui::CursorIcon::ResizeNeSw,
            Some(4 | 6) => egui::CursorIcon::ResizeVertical,
            Some(_) => egui::CursorIcon::ResizeHorizontal,
            None if inside(hq) && panning => egui::CursorIcon::Grabbing,
            None if inside(hq) => egui::CursorIcon::Grab,
            None => egui::CursorIcon::Alias,
        });
    }
    // double-click inside the crop box applies the crop (same as Return / Done)
    if resp.double_clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && inside(q)
        && pick(q).is_none()
    {
        let _ = app.run("tool.done", json!({}));
        return;
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        // Pick by where the button went DOWN: by `drag_started` the pointer has already moved, so a
        // quick drag on the bottom edge used to land inside the frame and move the whole crop
        // (Bryan, 2026-11-24).
        let _ = app.run("develop.beginInteraction", json!({"label": "Crop"}));
        let (start, angle) = (d.crop.geometry.rect, d.crop.geometry.angle);
        // ⇧ held where the button went down on the frame (inside it, or on an edge or corner): move the
        // whole crop freely, following the pointer, with no edge resize and no ratio in the way
        let free_move = ui.input(|i| i.modifiers.shift) && (inside(q) || pick(q).is_some());
        app.gesture = Some(match pick(q) {
            _ if free_move => Gesture::CropHandle { handle: 8, start, angle },
            Some(h) => Gesture::CropHandle { handle: h as u8, start, angle },
            None if inside(q) => Gesture::CropPan { start, angle },
            None => {
                let c = map.screen(Point::new(0.5, 0.5));
                Gesture::CropRotate { start_angle: d.crop.geometry.angle, a0: (q - c).angle() }
            }
        });
    }
    if resp.dragged()
        && let Some(q) = resp.interact_pointer_pos()
    {
        match app.gesture.clone() {
            Some(Gesture::CropHandle { handle, start, angle }) => {
                // The crop model decides what the drag does (anchor, aspect lock, image bounds): see
                // `lightcraft_geom::drag_crop`; the UI only reports where the pointer started and is.
                let n = to_straight(map.norm(q), angle, frame);
                let orig = ui.input(|i| i.pointer.press_origin()).map(|q0| to_straight(map.norm(q0), angle, frame)).unwrap_or(n);
                let _ = app.run(
                    "crop.drag",
                    json!({"handle": handle, "from": [orig.x, orig.y], "to": [n.x, n.y], "start": [start.x0, start.y0, start.x1, start.y1]}),
                );
            }
            Some(Gesture::CropPan { start, angle }) => {
                // The photo follows the pointer under the frame: the frame moves by the opposite
                // amount, as a pure translation (`crop.drag` ▸ move keeps the size and the angle).
                let n = to_straight(map.norm(q), angle, frame);
                let orig = ui.input(|i| i.pointer.press_origin()).map(|q0| to_straight(map.norm(q0), angle, frame)).unwrap_or(n);
                let to = [2.0 * orig.x - n.x, 2.0 * orig.y - n.y];
                let _ = app.run(
                    "crop.drag",
                    json!({"handle": 8, "from": [orig.x, orig.y], "to": to, "start": [start.x0, start.y0, start.x1, start.y1]}),
                );
            }
            Some(Gesture::CropRotate { start_angle, a0 }) => {
                let c = map.screen(Point::new(0.5, 0.5));
                let a = (q - c).angle();
                let ang = (start_angle + (a - a0).to_degrees() as f64).clamp(-45.0, 45.0);
                let _ = app.run("crop.straighten", json!({"angle": (ang * 100.0).round() / 100.0}));
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
    let _ = id;
}

/// Guided Upright: draw up to four guides along lines that should be vertical or horizontal. Guides are
/// stored in lens-corrected (pre-perspective) coordinates, so they stay attached to the image as it warps.
fn guided_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, frame: &Frame, d: &DevelopSettings) {
    let p = ui.painter_at(map.rect.expand(8.0));
    let col = Color32::from_rgb(255, 196, 40);
    let draw = |a: Pos2, b: Pos2| {
        p.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(120)));
        p.line_segment([a, b], Stroke::new(1.5, col));
        for q in [a, b] {
            p.circle_filled(q, 4.0, col);
            p.circle_stroke(q, 4.0, Stroke::new(1.0, Color32::BLACK));
        }
    };
    for (i, (a, b)) in d.geometry.guides.iter().enumerate() {
        let (sa, sb) = (map.screen(frame.corrected_to_transformed(*a)), map.screen(frame.corrected_to_transformed(*b)));
        register(ui.ctx(), format!("uprightGuide:{i}"), Rect::from_two_pos(sa, sb));
        draw(sa, sb);
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if resp.drag_started()
        && let Some(q) = resp.interact_pointer_pos()
    {
        app.gesture = Some(Gesture::Guide { a: map.norm(q) });
    }
    if let (Some(Gesture::Guide { a }), Some(q)) = (&app.gesture, resp.interact_pointer_pos()) {
        draw(map.screen(*a), q);
    }
    if resp.drag_stopped()
        && let (Some(Gesture::Guide { a }), Some(q)) = (app.gesture.clone(), resp.interact_pointer_pos())
    {
        app.gesture = None;
        let b = map.norm(q);
        if a.dist(b) > 0.02 {
            let (ca, cb) = (frame.transformed_to_corrected(a), frame.transformed_to_corrected(b));
            let _ = app.run("geometry.guides", json!({"guides": [[ca.x, ca.y, cb.x, cb.y]], "add": true}));
        }
    }
}

/// Normalized oriented coords → the straightened (rotated) frame the crop rect lives in.
fn to_straight(n: Point, angle: f64, frame: &Frame) -> Point {
    let (w, h) = (frame.ow, frame.oh);
    let px = Point::new(n.x * w, n.y * h);
    let r = Affine::rotate_about(angle.to_radians(), Point::new(w / 2.0, h / 2.0)).apply(px);
    Point::new(r.x / w, r.y / h)
}

fn frame_crop_quad(d: &DevelopSettings, frame: &Frame) -> [Point; 4] {
    let f = Frame { crop: d.crop.geometry, ..frame.clone() };
    f.crop_quad_norm()
}

// ------------------------------------------------------------------------ masks

/// Where a mask component's pin sits (normalized), for shapes that have a place on the photo.
pub(crate) fn component_pin(shape: &MaskShape) -> Option<Point> {
    match shape {
        MaskShape::Radial { center, .. } => Some(*center),
        MaskShape::Linear { start, end } => Some(Point::new((start.x + end.x) / 2.0, (start.y + end.y) / 2.0)),
        MaskShape::Brush { strokes } => strokes.iter().find(|s| !s.erase).and_then(|s| s.points.first()).copied(),
        _ => None,
    }
}

/// `shape` moved by `dn` (normalized); `handle` 1/2 = a linear gradient's start/end set to `at`.
fn moved_shape(shape: &MaskShape, handle: u8, dn: Point, at: Point, map: &CanvasMap, alt: bool) -> MaskShape {
    let mv = |p: Point| Point::new(p.x + dn.x, p.y + dn.y);
    match shape.clone() {
        MaskShape::Radial { center, rx, ry, angle, feather, invert } => match handle {
            1 | 3 => {
                let l = frame_long_norm(map);
                let dx = (at.x - center.x) / l.0;
                let dy = (at.y - center.y) / l.1;
                let (s, co) = angle.to_radians().sin_cos();
                let new_rx = (dx * co + dy * s).abs().max(0.01);
                let mut new_ry = ry;
                if alt {
                    new_ry = ry * (new_rx / rx.max(0.01));
                }
                if (new_rx - new_ry).abs() / new_rx.max(new_ry) < 0.05 {
                    new_ry = new_rx;
                }
                MaskShape::Radial { center, rx: new_rx, ry: new_ry, angle, feather, invert }
            }
            2 | 4 => {
                let l = frame_long_norm(map);
                let dx = (at.x - center.x) / l.0;
                let dy = (at.y - center.y) / l.1;
                let (s, co) = angle.to_radians().sin_cos();
                let new_ry = (dx * (-s) + dy * co).abs().max(0.01);
                let mut new_rx = rx;
                if alt {
                    new_rx = rx * (new_ry / ry.max(0.01));
                }
                if (new_rx - new_ry).abs() / new_rx.max(new_ry) < 0.05 {
                    new_rx = new_ry;
                }
                MaskShape::Radial { center, rx: new_rx, ry: new_ry, angle, feather, invert }
            }
            5 => {
                let l = frame_long_norm(map);
                let dx = (at.x - center.x) / l.0;
                let dy = (at.y - center.y) / l.1;
                let new_angle = dy.atan2(dx).to_degrees();
                MaskShape::Radial { center, rx, ry, angle: new_angle, feather, invert }
            }
            _ => MaskShape::Radial { center: mv(center), rx, ry, angle, feather, invert },
        },
        MaskShape::Linear { start, end } => match handle {
            1 => MaskShape::Linear { start: at, end },
            2 => MaskShape::Linear { start, end: at },
            _ => MaskShape::Linear { start: mv(start), end: mv(end) },
        },
        MaskShape::Brush { mut strokes } => {
            for st in &mut strokes {
                st.points.iter_mut().for_each(|p| *p = mv(*p));
            }
            MaskShape::Brush { strokes }
        }
        s => s,
    }
}

/// The Masking tool on the photo: outlines and handles of the selected mask, a pin per mask
/// component (click selects its mask, drag moves the component), and brush painting. The mask
/// itself shows as a rendered overlay ([`view_overlay`]).
fn mask_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    let tint = {
        let [r, g, b] = app.ui.mask_overlay_color;
        Color32::from_rgba_unmultiplied(r, g, b, 110)
    };
    let long = (map.rect.width().max(map.rect.height())) as f64;
    let active = app.session.active_mask;
    let clip = app.canvas_rect.unwrap_or(map.rect);
    let p = &ui.painter_at(clip);
    // (mask, component, handle, screen position) of everything that can be grabbed
    let mut grips: Vec<(u32, usize, u8, Pos2)> = Vec::new();
    for m in &d.masks {
        let sel = Some(m.id) == active;
        for (ci, c) in m.components.iter().enumerate() {
            match &c.shape {
                MaskShape::Radial { center, rx, ry, angle, .. } if sel => {
                    let pts: Vec<Pos2> = (0..64)
                        .map(|i| {
                            let a = i as f64 / 64.0 * std::f64::consts::TAU;
                            let (s, co) = angle.to_radians().sin_cos();
                            let (x, y) = (rx * a.cos(), ry * a.sin());
                            let l = frame_long_norm(map);
                            map.screen(Point::new(center.x + (x * co - y * s) * l.0, center.y + (x * s + y * co) * l.1))
                        })
                        .collect();
                    p.add(egui::Shape::closed_line(pts, Stroke::new(1.5, Color32::from_white_alpha(230))));

                    let l = frame_long_norm(map);
                    for (i, a_deg) in [0.0_f64, 90.0_f64, 180.0_f64, 270.0_f64].into_iter().enumerate() {
                        let a_local = a_deg.to_radians();
                        let (s, co) = angle.to_radians().sin_cos();
                        let (x, y) = (rx * a_local.cos(), ry * a_local.sin());
                        let q = map.screen(Point::new(center.x + (x * co - y * s) * l.0, center.y + (x * s + y * co) * l.1));
                        p.circle_stroke(q, 5.0, Stroke::new(1.5, Color32::WHITE));
                        register(ui.ctx(), format!("maskHandle:{}:{ci}:{}", m.id, i + 1), Rect::from_center_size(q, vec2(14.0, 14.0)));
                        grips.push((m.id, ci, i as u8 + 1, q));
                    }

                    let (s, co) = angle.to_radians().sin_cos();
                    let rot_r = rx + 0.08;
                    let rot_q = map.screen(Point::new(center.x + (rot_r * co) * l.0, center.y + (rot_r * s) * l.1));
                    let h1_q = map.screen(Point::new(center.x + (rx * co) * l.0, center.y + (rx * s) * l.1));
                    p.line_segment([h1_q, rot_q], Stroke::new(1.0, Color32::from_white_alpha(200)));
                    p.circle_stroke(rot_q, 4.0, Stroke::new(1.5, Color32::WHITE));
                    register(ui.ctx(), format!("maskHandle:{}:{ci}:5", m.id), Rect::from_center_size(rot_q, vec2(14.0, 14.0)));
                    grips.push((m.id, ci, 5, rot_q));
                }
                MaskShape::Linear { start, end } if sel => {
                    let (a, b) = (map.screen(*start), map.screen(*end));
                    let dir = (b - a).normalized();
                    let perp = vec2(-dir.y, dir.x) * 3000.0;
                    for (q, alpha) in [(a, 230), (b, 110)] {
                        p.line_segment([q - perp, q + perp], Stroke::new(1.0, Color32::from_white_alpha(alpha)));
                    }
                    for (i, q) in [a, b].iter().enumerate() {
                        p.circle_stroke(*q, 5.0, Stroke::new(1.5, Color32::WHITE));
                        register(ui.ctx(), format!("maskHandle:{}:{ci}:{i}", m.id), Rect::from_center_size(*q, vec2(14.0, 14.0)));
                        grips.push((m.id, ci, i as u8 + 1, *q));
                    }
                }
                _ => {}
            }
            if app.ui.mask_pins
                && let Some(at) = component_pin(&c.shape)
            {
                let q = map.screen(at);
                pin(p, q, sel);
                register(ui.ctx(), format!("maskPin:{}:{ci}", m.id), Rect::from_center_size(q, vec2(14.0, 14.0)));
                grips.push((m.id, ci, 0, q));
            }
        }
    }
    // Object selections (SAM 3) of the selected mask: each click, green to include, red to exclude
    // (clicks still on their way to the model included)
    let pending = app.session.segmenter.pending_clicks().cloned();
    for m in d.masks.iter().filter(|m| Some(m.id) == active) {
        for (k, c) in m.components.iter().enumerate() {
            if let MaskShape::Object { hint, exclude, .. } = &c.shape {
                let (hint, exclude) = match pending.as_ref().filter(|q| (q.mask, q.comp) == (m.id, k)) {
                    Some(q) => (&q.hint, &q.exclude),
                    None => (hint, exclude),
                };
                for (pts, col) in [(hint, Color32::from_rgb(40, 200, 90)), (exclude, Color32::from_rgb(230, 60, 60))] {
                    for q in pts {
                        let q = map.screen(*q);
                        p.circle_filled(q, 5.0, col);
                        p.circle_stroke(q, 5.0, Stroke::new(1.5, Color32::WHITE));
                    }
                }
            }
        }
    }
    // Object tool: a click includes what's under it, ⌥-click leaves it out
    if app.ui.tool == "object" {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = map.norm(q);
            let exclude = ui.input(|i| i.modifiers.alt);
            match app.run("mask.objectPoint", json!({"x": n.x, "y": n.y, "exclude": exclude})) {
                Ok(_) => {
                    if let Some(mid) = app.session.active_mask {
                        app.ui.detail_due = Some((ui.input(|i| i.time) + 1.0, mid));
                    }
                }
                Err(e) => app.ai_error(ui.ctx(), e, Some(("object", "new"))),
            }
        }
        return;
    }
    // brush tool
    if app.ui.tool == "brush" {
        let r = (app.ui.brush_size as f64 * long) as f32;
        if let Some(h) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            p.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
            p.circle_stroke(h, r * (1.0 - app.ui.brush_feather / 100.0).max(0.05), Stroke::new(1.0, Color32::from_white_alpha(120)));
        }
        if let Some(q) = resp.interact_pointer_pos()
            && (resp.drag_started() || resp.dragged() || resp.clicked())
        {
            let n = map.norm(q);
            match &mut app.gesture {
                Some(Gesture::Brush { points }) => points.push(n),
                _ => app.gesture = Some(Gesture::Brush { points: vec![n] }),
            }
        }
        // the stroke being painted (the render shows it once it's committed)
        if let Some(Gesture::Brush { points }) = &app.gesture {
            for q in points {
                p.circle_filled(map.screen(*q), r, tint);
            }
        }
        if (resp.drag_stopped() || resp.clicked())
            && let Some(Gesture::Brush { points }) = app.gesture.take()
        {
            let pts: Vec<[f64; 2]> = points.iter().map(|q| [q.x, q.y]).collect();
            // holding ⌥ (Alt) paints with the other mode: Erase while adding, Add while erasing
            let erase = app.ui.brush_erase != resp.ctx.input(|i| i.modifiers.alt);
            let _ = app.run(
                "mask.brushStroke",
                json!({"points": pts, "size": app.ui.brush_size, "feather": app.ui.brush_feather, "flow": app.ui.brush_flow,
                       "erase": erase, "autoMask": app.ui.brush_auto_mask}),
            );
        }
        return;
    }
    // the grip under the pointer: the selected mask's first, then the nearest
    let hit = |q: Pos2| {
        grips
            .iter()
            .filter(|g| g.3.distance(q) < 10.0)
            .min_by(|a, b| {
                (Some(a.0) != active, a.3.distance(q)).partial_cmp(&(Some(b.0) != active, b.3.distance(q))).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    };
    if resp.clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && let Some((mid, ..)) = hit(q)
        && Some(mid) != active
    {
        let _ = app.run("mask.select", json!({"id": mid}));
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let grip = hit(q).map(|(m, c, h, _)| (m, c, h)).or_else(|| {
            // anywhere on the photo drags the selected linear gradient
            let m = d.masks.iter().find(|m| Some(m.id) == active)?;
            matches!(m.components.first()?.shape, MaskShape::Linear { .. }).then_some((m.id, 0, 0))
        });
        if let Some((mask, comp, handle)) = grip
            && let Some(original_shape) = d.masks.iter().find(|m| m.id == mask).and_then(|m| m.components.get(comp)).map(|c| c.shape.clone())
        {
            if Some(mask) != active {
                let _ = app.run("mask.select", json!({"id": mask}));
            }
            let _ = app.run("develop.beginInteraction", json!({"label": "Edit Mask"}));
            app.gesture = Some(Gesture::MaskHandle { mask, comp, handle, original_shape });
        }
    }
    if resp.dragged()
        && let (Some(Gesture::MaskHandle { mask, comp, handle, original_shape }), Some(q)) = (app.gesture.clone(), resp.interact_pointer_pos())
    {
        let n = map.norm(q);
        let press_origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(q);
        let n0 = map.norm(press_origin);
        let dn = Point::new(n.x - n0.x, n.y - n0.y);
        let alt = ui.input(|i| i.modifiers.alt);
        let new_shape = moved_shape(&original_shape, handle, dn, n, map, alt);
        let _ = app.run("mask.update", json!({"id": mask, "component": comp, "shape": new_shape}));
    }
    if resp.drag_stopped() && matches!(app.gesture, Some(Gesture::MaskHandle { .. })) {
        app.gesture = None;
        let _ = app.run("develop.endInteraction", json!({}));
    }
}

/// Long-edge units → normalized units for x and y.
fn frame_long_norm(map: &CanvasMap) -> (f64, f64) {
    // derive from the mapping: 1 normalized unit in x and y in screen points
    let o = map.screen(Point::new(0.0, 0.0));
    let x = map.screen(Point::new(1.0, 0.0)).distance(o) as f64;
    let y = map.screen(Point::new(0.0, 1.0)).distance(o) as f64;
    let l = x.max(y);
    (l / x.max(1e-9), l / y.max(1e-9))
}

fn pin(p: &egui::Painter, c: Pos2, sel: bool) {
    p.circle_filled(c, 6.0, if sel { Color32::from_rgb(1, 101, 221) } else { Color32::from_gray(200) });
    p.circle_stroke(c, 6.0, Stroke::new(1.5, Color32::WHITE));
}

// ------------------------------------------------------------------------ remove

/// The Remove tool on the photo: every spot's outline and pin (the selected one with its source),
/// click a pin to select its spot, drag a target or source to move it, paint elsewhere to add one.
fn remove_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    let long = (map.rect.width().max(map.rect.height())) as f64;
    let p = ui.painter_at(app.canvas_rect.unwrap_or(map.rect));
    let active = app.session.active_spot.filter(|i| *i < d.spots.len());
    // (spot, is source, screen centre, radius) of everything that can be grabbed
    let mut grips: Vec<(usize, bool, Pos2, f32)> = Vec::new();
    for (i, s) in d.spots.iter().enumerate() {
        let r = (s.size * long) as f32;
        let sel = Some(i) == active;
        // a spot over a whole detected object has hundreds of points: draw it quieter unless selected
        let col = Color32::from_white_alpha(if sel {
            230
        } else if s.points.len() > 24 {
            60
        } else {
            150
        });
        for q in &s.points {
            p.circle_stroke(map.screen(*q), r, Stroke::new(if sel { 1.5 } else { 1.0 }, col));
        }
        let Some(&t) = s.points.first() else { continue };
        let tq = map.screen(t);
        if let Some(o) = s.source_offset.filter(|_| sel) {
            let sq = map.screen(Point::new(t.x + o.x, t.y + o.y));
            for q in &s.points {
                p.circle_stroke(map.screen(Point::new(q.x + o.x, q.y + o.y)), r, Stroke::new(1.0, Color32::from_white_alpha(170)));
            }
            let dir = (tq - sq).normalized();
            p.arrow(sq + dir * r, (tq - sq) - dir * (2.0 * r).min((tq - sq).length()), Stroke::new(1.0, Color32::from_white_alpha(200)));
            register(ui.ctx(), format!("spotSource:{i}"), Rect::from_center_size(sq, vec2(14.0, 14.0)));
            grips.push((i, true, sq, r));
        }
        pin(&p, tq, sel);
        register(ui.ctx(), format!("spotPin:{i}"), Rect::from_center_size(tq, vec2(14.0, 14.0)));
        grips.push((i, false, tq, r));
    }
    // Remove ▸ Detect: the candidates, highlighted when hovered (here or on their chip) and when picked
    let photo_hover = resp.hover_pos().and_then(|q| candidate_at(app, map, q, long));
    if app.session.remove_found.photo == app.session.active() {
        let accent = Tokens::get(ui.ctx()).accent;
        for (i, c) in app.session.remove_found.items.iter().enumerate() {
            let picked = app.ui.remove_picked.contains(&i);
            let hot = photo_hover == Some(i) || app.ui.remove_hover_chip == Some(i);
            let a = map.screen(Point::new(c.rect[0], c.rect[1]));
            let b = map.screen(Point::new(c.rect[2], c.rect[3]));
            let rect = Rect::from_two_pos(a, b);
            if picked || hot {
                let fill = if picked { accent.gamma_multiply(0.22) } else { Color32::from_white_alpha(28) };
                for s in &c.spots {
                    let r = (s.size * long) as f32;
                    for q in &s.points {
                        p.circle_filled(map.screen(*q), r, fill);
                    }
                }
            }
            let col = if picked {
                accent
            } else if hot {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(140)
            };
            if c.kind == lightcraft_engine::remove_detect::Kind::Dust {
                // dust is scattered: a ring on each speck, no box
                for s in &c.spots {
                    for q in &s.points {
                        p.circle_stroke(map.screen(*q), ((s.size * long) as f32).max(4.0), Stroke::new(1.0, col));
                    }
                }
            } else {
                p.rect_stroke(rect, 2.0, Stroke::new(if picked || hot { 1.5 } else { 1.0 }, col), egui::StrokeKind::Outside);
                p.text(rect.left_top() + vec2(4.0, 4.0), egui::Align2::LEFT_TOP, &c.label, egui::FontId::proportional(12.0), col);
            }
            register(ui.ctx(), format!("removeCandidate:{i}"), rect);
        }
    }
    let r = (app.ui.remove_size as f64 * long) as f32;
    // a target or source under `q`: the selected spot's first, then the nearest pin
    let hit = |q: Pos2| {
        grips
            .iter()
            .filter(|g| g.2.distance(q) < g.3.max(8.0))
            .min_by(|a, b| {
                (Some(a.0) != active, a.2.distance(q)).partial_cmp(&(Some(b.0) != active, b.2.distance(q))).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied()
    };
    let hover = resp.hover_pos().and_then(hit);
    if let Some(h) = resp.hover_pos() {
        if hover.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            p.circle_stroke(h, r, Stroke::new(1.0, Color32::WHITE));
            p.circle_stroke(h, r * (1.0 - app.ui.remove_feather / 100.0).max(0.05), Stroke::new(1.0, Color32::from_white_alpha(110)));
        }
    }
    let press = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
    // ⌥-drag: a marquee that deletes the spots inside it (Lightroom)
    if resp.drag_started()
        && ui.input(|i| i.modifiers.alt)
        && let Some(q) = press
    {
        app.gesture = Some(Gesture::SpotRect { a: map.norm(q) });
    }
    if let Some(Gesture::SpotRect { a }) = app.gesture.clone() {
        if let Some(q) = resp.interact_pointer_pos() {
            let b = map.norm(q);
            let rect = Rect::from_two_pos(map.screen(a), map.screen(b));
            p.rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(220, 60, 60, 40));
            p.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_rgb(230, 80, 80)), egui::StrokeKind::Inside);
            if resp.drag_stopped() {
                app.gesture = None;
                let _ = app.run("spot.deleteInRect", json!({"rect": [a.x, a.y, b.x, b.y]}));
            }
        } else if resp.drag_stopped() {
            app.gesture = None;
        }
        return;
    }
    if resp.drag_started()
        && let Some((spot, source, ..)) = press.and_then(hit)
    {
        if Some(spot) != active {
            let _ = app.run("spot.select", json!({"index": spot}));
        }
        let _ = app.run("develop.beginInteraction", json!({"label": "Edit Spot"}));
        app.gesture = Some(Gesture::SpotMove { spot, source });
    }
    if let Some(Gesture::SpotMove { spot, source }) = app.gesture.clone() {
        if resp.dragged()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let (n, n0) = (map.norm(q), map.norm(q - resp.drag_delta()));
            let dn = [n.x - n0.x, n.y - n0.y];
            if dn != [0.0, 0.0] {
                let _ = app.run("spot.update", json!({"index": spot, if source { "moveSource" } else { "move" }: dn}));
            }
        }
        if resp.drag_stopped() {
            app.gesture = None;
            let _ = app.run("develop.endInteraction", json!({}));
        }
        return;
    }
    if resp.clicked()
        && let Some((spot, ..)) = press.and_then(hit)
    {
        let _ = app.run("spot.select", json!({"index": spot}));
        return;
    }
    // a click on a detected object picks it (⌘ adds / removes, ⇧ the run from the last one); painting still drags
    if resp.clicked()
        && let Some(i) = press.and_then(|q| candidate_at(app, map, q, long))
    {
        let m = ui.input(|i| i.modifiers);
        let n = app.session.remove_found.items.len();
        pick_candidate(app, i, n, m.command, m.shift);
        return;
    }
    if let Some(q) = resp.interact_pointer_pos()
        && (resp.drag_started() || resp.dragged() || resp.clicked())
    {
        let n = map.norm(q);
        match &mut app.gesture {
            Some(Gesture::Spot { points }) => points.push(n),
            _ => app.gesture = Some(Gesture::Spot { points: vec![n] }),
        }
    }
    if let Some(Gesture::Spot { points }) = &app.gesture {
        for q in points {
            p.circle_filled(map.screen(*q), r, Color32::from_white_alpha(60));
        }
    }
    if (resp.drag_stopped() || resp.clicked())
        && let Some(Gesture::Spot { points }) = app.gesture.take()
    {
        let mode = match app.ui.tool.as_str() {
            "heal" => "heal",
            "clone" => "clone",
            "fill" => "fill",
            _ => "remove",
        };
        let pts: Vec<[f64; 2]> = points.iter().map(|q| [q.x, q.y]).collect();
        // Use Generative AI: Remove and Fill strokes become generative spots (Heal and Clone keep their source)
        let generative = app.ui.remove_generative && matches!(mode, "remove" | "fill");
        match app.run(
            "spot.add",
            json!({"mode": mode, "points": pts, "size": app.ui.remove_size, "feather": app.ui.remove_feather, "opacity": app.ui.remove_opacity, "generative": generative, "quick": app.ui.remove_quick}),
        ) {
            Ok(r) => {
                if let Some(n) = r["notice"].as_str()
                    && !app.session.genremove.missing_said
                {
                    app.session.genremove.missing_said = true;
                    app.toast_for(ui.ctx(), n.to_string(), 6.0);
                }
            }
            Err(e) => app.toast(ui.ctx(), e.to_string()),
        }
    }
}

/// The detected object (Remove ▸ Detect) under screen point `q`: inside one of its discs; the smallest wins.
fn candidate_at(app: &LightcraftApp, map: &CanvasMap, q: Pos2, long: f64) -> Option<usize> {
    if app.session.remove_found.photo != app.session.active() {
        return None;
    }
    app.session
        .remove_found
        .items
        .iter()
        .enumerate()
        .filter(|(_, c)| c.spots.iter().any(|s| s.points.iter().any(|p| map.screen(*p).distance(q) <= (s.size * long) as f32)))
        .min_by(|a, b| {
            let area = |r: &[f64; 4]| (r[2] - r[0]) * (r[3] - r[1]);
            area(&a.1.rect).partial_cmp(&area(&b.1.rect)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// Pick detected object `i` of `n`: click = just this one, ⌘ = add / remove it, ⇧ = the run from the last click.
pub(crate) fn pick_candidate(app: &mut LightcraftApp, i: usize, n: usize, command: bool, shift: bool) {
    let picked = &mut app.ui.remove_picked;
    picked.retain(|p| *p < n);
    if shift && let Some(a) = app.ui.remove_pick_anchor.filter(|a| *a < n) {
        for k in a.min(i)..=a.max(i) {
            if !picked.contains(&k) {
                picked.push(k);
            }
        }
    } else if command {
        match picked.iter().position(|p| *p == i) {
            Some(k) => {
                picked.remove(k);
            }
            None => picked.push(i),
        }
    } else {
        *picked = vec![i];
    }
    picked.sort_unstable();
    app.ui.remove_pick_anchor = Some(i);
}

// ------------------------------------------------------------------------ red eye

/// Red Eye tool: drag an ellipse over an eye (a new correction), click one to select it.
fn eye_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, map: &CanvasMap, d: &DevelopSettings) {
    // screen points per long-edge unit
    let o = map.screen(Point::new(0.0, 0.0));
    let (sx, sy) = (map.screen(Point::new(1.0, 0.0)).distance(o), map.screen(Point::new(0.0, 1.0)).distance(o));
    let per_long = sx.max(sy);
    let p = ui.painter();
    for (i, e) in d.red_eye.iter().enumerate() {
        let c = map.screen(e.center);
        let r = vec2(e.rx as f32 * per_long, e.ry as f32 * per_long);
        let sel = i == app.ui.eye;
        let col = if sel { Color32::WHITE } else { Color32::from_white_alpha(150) };
        p.add(egui::Shape::ellipse_stroke(c, r, Stroke::new(if sel { 1.5 } else { 1.0 }, col)));
        p.line_segment([c - vec2(4.0, 0.0), c + vec2(4.0, 0.0)], Stroke::new(1.0, col));
        p.line_segment([c - vec2(0.0, 4.0), c + vec2(0.0, 4.0)], Stroke::new(1.0, col));
    }
    if resp.hover_pos().is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        // from where the button went down (a drag is recognized only after some movement)
        app.gesture = Some(Gesture::Eye { a: map.norm(q) });
    }
    if let Some(Gesture::Eye { a }) = app.gesture
        && let Some(q) = resp.interact_pointer_pos()
    {
        let (a, b) = (map.screen(a), q);
        p.add(egui::Shape::ellipse_stroke(a.lerp(b, 0.5), ((b - a) / 2.0).abs(), Stroke::new(1.0, Color32::WHITE)));
    }
    if resp.drag_stopped()
        && let Some(Gesture::Eye { a }) = app.gesture.take()
        && let Some(q) = resp.interact_pointer_pos()
    {
        let (sa, sb) = (map.screen(a), q);
        let half = ((sb - sa) / 2.0).abs() / per_long;
        if half.x > 0.002 && half.y > 0.002 {
            let c = map.norm(sa.lerp(sb, 0.5));
            let r = app.run("redeye.add", json!({"center": [c.x, c.y], "rx": half.x, "ry": half.y, "pet": app.ui.eye_pet}));
            if let Ok(v) = r {
                app.ui.eye = v["index"].as_u64().unwrap_or(0) as usize;
            }
        }
    } else if resp.clicked()
        && let Some(q) = resp.interact_pointer_pos()
        && let Some(i) = d.red_eye.iter().position(|e| {
            let c = map.screen(e.center);
            let (dx, dy) = ((q.x - c.x) / (e.rx as f32 * per_long).max(4.0), (q.y - c.y) / (e.ry as f32 * per_long).max(4.0));
            dx * dx + dy * dy <= 1.0
        })
    {
        app.ui.eye = i;
    }
}

// ------------------------------------------------------------------------ filmstrip

/// Straighten tool: drag along a horizon (or a vertical) to set the crop angle; double-click = Auto.
/// The image is shown unrotated in the crop view, so the line's on-screen angle is its image angle.
/// `held`: drawn with ⌘ held in the crop tool, which stays active afterwards.
fn straighten_overlay(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, held: bool) {
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if !held && resp.double_clicked() {
        let _ = app.run("crop.autoStraighten", json!({}));
        app.ui.tool.clear();
        app.gesture = None;
        return;
    }
    if resp.drag_started()
        && let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        app.gesture = Some(Gesture::StraightenLine { a: q });
    }
    let Some(Gesture::StraightenLine { a }) = app.gesture.clone() else { return };
    let Some(b) = resp.interact_pointer_pos().or(resp.hover_pos()) else { return };
    let p = ui.painter();
    p.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(140)));
    p.line_segment([a, b], Stroke::new(1.5, crate::theme::CROP));
    if resp.drag_stopped() {
        app.gesture = None;
        let v = b - a;
        if v.length() > 8.0 {
            let mut deg = v.y.atan2(v.x).to_degrees() as f64;
            // fold to the nearest axis: a line closer to vertical straightens plumb lines
            while deg > 45.0 {
                deg -= 90.0;
            }
            while deg < -45.0 {
                deg += 90.0;
            }
            let _ = app.run("crop.straighten", json!({"angle": (-deg * 100.0).round() / 100.0}));
        }
        if !held {
            app.ui.tool.clear();
        }
    }
}

#[cfg(test)]
mod preview_geometry_tests {
    use super::{fit_rect, fit_texture_rect, pixel_exact};
    use crate::state::Zoom;
    use egui::{Rect, pos2, vec2};

    #[test]
    fn a_render_made_for_the_view_is_drawn_pixel_for_pixel() {
        // Bryan's loupe on 11-23: a 2560 px render fitted into 1275.2 pt at x = 366.9 pt, 2 px per point
        let fitted = fit_texture_rect(Rect::from_min_size(pos2(366.88, 66.0), vec2(1275.23, 850.16)), [2560, 1707]);
        let r = pixel_exact(fitted, [2560, 1707], 2.0);
        assert_eq!((r.width() * 2.0, r.height() * 2.0), (2560.0, 1707.0));
        assert_eq!(((r.min.x * 2.0).fract(), (r.min.y * 2.0).fract()), (0.0, 0.0));
        assert!((r.center() - fitted.center()).length() < 1.0);
        // a stand-in (thumbnail) or a draft is still scaled to fit
        let small = fit_texture_rect(Rect::from_min_size(pos2(0.0, 0.0), vec2(1275.0, 850.0)), [1024, 683]);
        assert_eq!(pixel_exact(small, [1024, 683], 2.0), small);
    }

    #[test]
    fn portrait_preview_keeps_its_ratio_inside_a_landscape_frame() {
        let frame = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let image = fit_texture_rect(frame, [100, 200]);

        assert!((image.width() / image.height() - 0.5).abs() < 1e-6);
        assert_eq!(image.center(), frame.center());
        assert!(frame.contains_rect(image));
    }

    #[test]
    fn portrait_preview_aspect_fits_detail_area_without_stretching() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 200.0));
        let image = fit_rect(area, 0.5, Zoom::Fit, [4000, 6000], 1.0, (0.5, 0.5));

        assert!((image.width() / image.height() - 0.5).abs() < 1e-6);
        assert_eq!(image.size(), vec2(100.0, 200.0));
        assert_eq!(image.center(), area.center());
    }
}

/// The display headroom an HDR edit is shown with in the loupe: the HDR display's current headroom,
/// unless Preview for SDR Display is on, Visualize HDR draws its bands, or there is no HDR display.
pub(crate) fn hdr_headroom(app: &LightcraftApp) -> Option<f32> {
    if app.ui.hdr_sdr_preview || app.ui.hdr_visualize {
        return None;
    }
    app.hdr_view.as_ref().and_then(|v| v.headroom()).filter(|h| h.is_finite() && *h > 1.0)
}

/// The crop was adjusted within [`crate::CROP_GRID_HOLD`]: show its grid (and look again then).
fn crop_grid_on(app: &LightcraftApp, ctx: &egui::Context) -> bool {
    let Some(at) = app.crop_touch else { return false };
    let age = at.elapsed();
    if age < crate::CROP_GRID_HOLD {
        ctx.request_repaint_after(crate::CROP_GRID_HOLD - age);
        true
    } else {
        false
    }
}
