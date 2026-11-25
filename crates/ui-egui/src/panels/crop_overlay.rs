//! Crop guide overlays as polylines in crop-relative coordinates (`(u, v)` in `0..=1`, `u` across
//! the crop's width, `v` down its height). The crop tool maps them onto the (rotated) frame.
//! Composition guides are classic geometry: thirds, a grid, golden-section lines, 45° diagonals
//! from the corners, the harmonic triangle, the golden spiral, fifths, the centre and the outlines
//! of common aspect ratios.
//!
//! The same guides on the loupe outside the crop tool, with Lightroom Classic's Loupe Overlay
//! (grid, guides, layout image) and aspect-ratio frames, are [`loupe`] and the `view.loupe*` /
//! `view.composition*` / `view.frameGuides` commands ([`run_command`]; LRC-VIEW-LOUPEOVERLAY).

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Align2, Color32, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{CropOverlay, LoupeOverlay, OverlayShow, frame_ratio};
use crate::widgets::register;

/// The loupe-overlay commands; `None` for any other id.
pub fn run_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let o = &mut app.ui.loupe_overlay;
    let on = |cur: bool| p.get("on").and_then(Value::as_bool).unwrap_or(!cur);
    let unit = |k: &str, lo: f64, hi: f64| -> Result<Option<f32>, String> {
        match p.get(k) {
            None => Ok(None),
            Some(v) => match v.as_f64().filter(|x| x.is_finite() && (lo..=hi).contains(x)) {
                Some(x) => Ok(Some(x as f32)),
                None => Err(format!("{id}: `{k}` must be a number from {lo} to {hi}")),
            },
        }
    };
    // only settings given (size, opacity…): change them without toggling
    let only_settings = |keys: &[&str]| p.get("on").is_none() && keys.iter().any(|k| p.get(*k).is_some());
    let r: Result<(), String> = (|| {
        match id {
            "view.loupeOverlay" => {
                // {show?: bool} — View ▸ Loupe Overlay ▸ Show (⌘⌥O): all overlays on / off
                let shown = o.show != OverlayShow::Never;
                if p.get("show").and_then(Value::as_bool).unwrap_or(!shown) {
                    o.show = o.last_show;
                } else {
                    o.show = OverlayShow::Never;
                }
            }
            "view.overlayShow" => {
                // {mode: always|auto|never}
                let m = p.get("mode").cloned().unwrap_or(Value::Null);
                let mode = serde_json::from_value::<OverlayShow>(m.clone()).map_err(|_| format!("{id}: mode {m} (always, auto or never)"))?;
                o.show = mode;
                if mode != OverlayShow::Never {
                    o.last_show = mode;
                }
            }
            "view.loupeGrid" => {
                // {on?, size?: 8..200 points, opacity?: 0.05..1}
                let (size, opacity) = (unit("size", 8.0, 200.0)?, unit("opacity", 0.05, 1.0)?);
                if !only_settings(&["size", "opacity"]) {
                    o.grid = on(o.grid);
                }
                o.grid_size = size.unwrap_or(o.grid_size);
                o.grid_opacity = opacity.unwrap_or(o.grid_opacity);
                reveal(o.grid, o);
            }
            "view.loupeGuides" => {
                // {on?, at?: [x, y] (fraction of the photo), reset?}
                if let Some(v) = p.get("at") {
                    let at = serde_json::from_value::<(f32, f32)>(v.clone())
                        .ok()
                        .filter(|(x, y)| x.is_finite() && y.is_finite() && (0.0..=1.0).contains(x) && (0.0..=1.0).contains(y))
                        .ok_or_else(|| format!("{id}: `at` is [x, y], each 0 to 1"))?;
                    o.guides_at = at;
                }
                if p.get("reset").and_then(Value::as_bool) == Some(true) {
                    o.guides_at = (0.5, 0.5);
                }
                if !only_settings(&["at", "reset"]) {
                    o.guides = on(o.guides);
                }
                reveal(o.guides, o);
            }
            "view.layoutImage" => {
                // {on?, path?: a PNG / JPEG, opacity?: 0.05..1}
                let opacity = unit("opacity", 0.05, 1.0)?;
                if let Some(path) = p.get("path").and_then(Value::as_str) {
                    if !std::path::Path::new(path).is_file() {
                        return Err(format!("{id}: {path}: no such file"));
                    }
                    o.layout_path = path.to_string();
                    o.layout = true;
                } else if !only_settings(&["opacity"]) {
                    if o.layout_path.is_empty() && on(o.layout) {
                        return Err(format!("{id}: no layout image chosen yet (View ▸ Loupe Overlay ▸ Choose Layout Image…)"));
                    }
                    o.layout = on(o.layout);
                }
                o.layout_opacity = opacity.unwrap_or(o.layout_opacity);
                reveal(o.layout, o);
            }
            "view.compositionOverlay" => {
                // {overlay?: thirds|grid|golden|diagonal|triangle|spiral|aspectRatios|center|fifths|none}:
                // that one, else the next of the overlays chosen to cycle (O in the loupe)
                o.composition = match p.get("overlay") {
                    Some(v) => serde_json::from_value::<CropOverlay>(v.clone()).map_err(|e| format!("{id}: `overlay`: {e}"))?,
                    None => o.composition.next_in(&app.ui.crop_overlay_cycle),
                };
                if let Some(op) = unit("opacity", 0.05, 1.0)? {
                    o.line_opacity = op;
                }
                reveal(o.composition != CropOverlay::None, o);
            }
            "view.compositionOverlayOrientation" => {
                o.orient = (o.orient + 1) % 4;
            }
            "view.frameGuides" => {
                // {frames?: ["4:5", …] (replace), toggle?: "4:5", dim?: bool, opacity?} → the frames
                if let Some(v) = p.get("frames") {
                    let list =
                        serde_json::from_value::<Vec<String>>(v.clone()).map_err(|_| format!("{id}: `frames` is a list like [\"4:5\", \"16:9\"]"))?;
                    if let Some(bad) = list.iter().find(|s| frame_ratio(s).is_none()) {
                        return Err(format!("{id}: {bad:?} is not a ratio like 4:5"));
                    }
                    let mut seen: Vec<String> = Vec::new();
                    for s in list.into_iter().map(|s| s.trim().to_string()) {
                        if !seen.contains(&s) && seen.len() < 8 {
                            seen.push(s);
                        }
                    }
                    o.frames = seen;
                }
                if let Some(t) = p.get("toggle").and_then(Value::as_str) {
                    let t = t.trim().to_string();
                    if frame_ratio(&t).is_none() {
                        return Err(format!("{id}: {t:?} is not a ratio like 4:5"));
                    }
                    if let Some(i) = o.frames.iter().position(|f| *f == t) {
                        o.frames.remove(i);
                    } else if o.frames.len() < 8 {
                        o.frames.push(t);
                    }
                }
                if let Some(d) = p.get("dim").and_then(Value::as_bool) {
                    o.frame_dim = d;
                }
                if let Some(op) = unit("opacity", 0.05, 1.0)? {
                    o.line_opacity = op;
                }
                reveal(!o.frames.is_empty(), o);
            }
            "view.chooseLayoutImage" => return Ok(()),
            _ => return Err(String::new()),
        }
        Ok(())
    })();
    match r {
        Err(e) if e.is_empty() => None,
        Err(e) => Some(Err(e)),
        Ok(()) if id == "view.chooseLayoutImage" => {
            let path = app.services.pick_files.as_mut().and_then(|f| f().into_iter().next());
            Some(match path {
                Some(path) => app.run("view.layoutImage", json!({"path": path})),
                None => Ok(Value::Null),
            })
        }
        Ok(()) => Some(Ok(serde_json::to_value(&app.ui.loupe_overlay).unwrap_or_default())),
    }
}

/// Turning an overlay on while they are all hidden (Never) shows them again.
fn reveal(turned_on: bool, o: &mut LoupeOverlay) {
    if turned_on && o.show == OverlayShow::Never {
        o.show = o.last_show;
    }
}

/// A layout image being read off the UI thread, then its texture.
#[derive(Clone)]
struct LayoutTexture {
    path: String,
    pending: Arc<Mutex<Option<Result<egui::ColorImage, String>>>>,
    tex: Option<egui::TextureHandle>,
    error: Option<String>,
}

/// The layout image's texture for `path` (read on a worker thread the first time; `None` while
/// it loads or when it can't be read — the error is then in the second value).
fn layout_texture(ctx: &egui::Context, path: &str) -> (Option<egui::TextureHandle>, Option<String>) {
    let key = egui::Id::new("loupe-layout-image");
    let mut cur: Option<LayoutTexture> = ctx.data(|d| d.get_temp(key));
    if cur.as_ref().is_none_or(|c| c.path != path) {
        let pending = Arc::new(Mutex::new(None));
        let (out, file, repaint) = (pending.clone(), path.to_string(), ctx.clone());
        let spawned = std::thread::Builder::new().name("layout-image".into()).spawn(move || {
            let r = std::fs::read(&file).map_err(|e| e.to_string()).and_then(|bytes| {
                lightcraft_codecs::decode_thumbnail(&bytes, 4096).map_err(|e| e.to_string()).map(|t| {
                    let size = [t.image.width, t.image.height];
                    let rgba: Vec<u8> = t.image.data.iter().flatten().copied().collect();
                    egui::ColorImage::from_rgba_unmultiplied(size, &rgba)
                })
            });
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
            repaint.request_repaint();
        });
        let error = spawned.err().map(|e| e.to_string());
        cur = Some(LayoutTexture { path: path.to_string(), pending, tex: None, error });
    }
    let Some(mut c) = cur else { return (None, None) };
    let done = c.pending.lock().unwrap_or_else(PoisonError::into_inner).take();
    match done {
        Some(Ok(img)) if img.size[0] > 0 && img.size[1] > 0 => {
            c.tex = Some(ctx.load_texture("loupe-layout-image", img, egui::TextureOptions::LINEAR))
        }
        Some(Ok(_)) => c.error = Some("empty image".into()),
        Some(Err(e)) => c.error = Some(e),
        None => {}
    }
    let out = (c.tex.clone(), c.error.clone());
    ctx.data_mut(|d| d.insert_temp(key, c));
    out
}

/// The loupe's overlays over the photo shown at `img` (the cropped photo as displayed, zoomed and
/// panned), clipped to `clip`. Only ⌘-dragging a guide takes the pointer; returns true while one
/// is being dragged (the loupe then doesn't pan or zoom).
pub fn loupe(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, img: Rect, clip: Rect) -> bool {
    let o = app.ui.loupe_overlay.clone();
    if !o.any() || app.ui.fullscreen || !img.is_positive() {
        return false;
    }
    let shown = match o.show {
        OverlayShow::Never => false,
        OverlayShow::Always => true,
        OverlayShow::Auto => app.ui.dragging_control.is_some() || resp.hovered() || resp.dragged() || app.session.interaction.is_some(),
    };
    let dragging = guides_drag(app, ui, resp, img, o.guides && shown);
    if !shown {
        return dragging;
    }
    let ctx = ui.ctx().clone();
    let p = ui.painter_at(clip);
    let line = |a: f32| Stroke::new(1.0, crate::theme::CROP.gamma_multiply(a.clamp(0.0, 1.0)));
    let shadow = |a: f32| Stroke::new(1.0, Color32::from_black_alpha((a.clamp(0.0, 1.0) * 120.0) as u8));
    let at = |u: f32, v: f32| pos2(img.left() + u * img.width(), img.top() + v * img.height());
    // the layout image under the lines, fitted inside the photo
    if o.layout && !o.layout_path.is_empty() {
        match layout_texture(&ctx, &o.layout_path) {
            (Some(tex), _) => {
                let [w, h] = tex.size();
                let s = (img.width() / w.max(1) as f32).min(img.height() / h.max(1) as f32);
                let r = Rect::from_center_size(img.center(), vec2(w as f32 * s, h as f32 * s));
                p.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE.gamma_multiply(o.layout_opacity));
                register(&ctx, "overlay:layout", r);
            }
            (None, Some(e)) => {
                let t = crate::theme::Tokens::get(&ctx);
                p.text(
                    img.left_top() + vec2(10.0, 12.0),
                    Align2::LEFT_CENTER,
                    format!("{}: {e}", crate::i18n::tr("Layout image")),
                    t.font(11.5),
                    Color32::WHITE,
                );
            }
            (None, None) => {}
        }
    }
    // the grid: square cells of a fixed size on screen, from the photo's centre
    if o.grid {
        let step = o.grid_size.max(8.0);
        let (c, a) = (img.center(), o.grid_opacity);
        let nx = (img.width() / 2.0 / step).floor().min(400.0) as i32;
        let ny = (img.height() / 2.0 / step).floor().min(400.0) as i32;
        for i in -nx..=nx {
            let x = c.x + i as f32 * step;
            p.line_segment([pos2(x, img.top()), pos2(x, img.bottom())], line(a));
        }
        for j in -ny..=ny {
            let y = c.y + j as f32 * step;
            p.line_segment([pos2(img.left(), y), pos2(img.right(), y)], line(a));
        }
        register(&ctx, "overlay:grid", img);
    }
    // the composition guide on the photo as shown
    if o.composition != CropOverlay::None {
        for path in paths(o.composition, o.orient, img.width(), img.height()) {
            let pts: Vec<egui::Pos2> = path.iter().map(|&(u, v)| at(u, v)).collect();
            p.add(egui::Shape::line(pts.iter().map(|q| *q + vec2(1.0, 1.0)).collect(), shadow(o.line_opacity)));
            p.add(egui::Shape::line(pts, line(o.line_opacity)));
        }
        register(&ctx, format!("overlay:composition:{}", o.composition.id()), img);
    }
    // aspect-ratio frames, each as large as fits, centred; one alone darkens what falls outside
    let frames: Vec<(String, Rect)> = o.frames.iter().filter_map(|f| frame_ratio(f).map(|r| (f.clone(), frame_rect(img, r)))).collect();
    if let [(_, only)] = frames.as_slice()
        && o.frame_dim
    {
        let dim = Color32::from_black_alpha(115);
        for r in [
            Rect::from_min_max(img.min, pos2(img.right(), only.top())),
            Rect::from_min_max(pos2(img.left(), only.bottom()), img.max),
            Rect::from_min_max(pos2(img.left(), only.top()), pos2(only.left(), only.bottom())),
            Rect::from_min_max(pos2(only.right(), only.top()), pos2(img.right(), only.bottom())),
        ] {
            if r.is_positive() {
                p.rect_filled(r, 0.0, dim);
            }
        }
    }
    let t = crate::theme::Tokens::get(&ctx);
    for (label, r) in &frames {
        p.rect_stroke(r.translate(vec2(1.0, 1.0)), 0.0, shadow(o.line_opacity), egui::StrokeKind::Inside);
        p.rect_stroke(*r, 0.0, Stroke::new(1.5, crate::theme::CROP.gamma_multiply(o.line_opacity.clamp(0.0, 1.0))), egui::StrokeKind::Inside);
        let g = p.layout_no_wrap(label.clone(), t.semibold(11.0), Color32::WHITE);
        let tag = Rect::from_min_size(r.left_top() + vec2(6.0, 6.0), g.size() + vec2(10.0, 4.0));
        p.rect_filled(tag, 3.0, Color32::from_black_alpha(150));
        p.galley(tag.min + vec2(5.0, 2.0), g, Color32::WHITE);
        register(&ctx, format!("overlay:frame:{label}"), *r);
    }
    // the guides: a vertical and a horizontal line, ⌘-drag to move them
    if o.guides {
        let q = at(o.guides_at.0, o.guides_at.1);
        let s = Stroke::new(1.0, crate::theme::CROP);
        for seg in [[pos2(q.x, img.top()), pos2(q.x, img.bottom())], [pos2(img.left(), q.y), pos2(img.right(), q.y)]] {
            p.line_segment([seg[0] + vec2(1.0, 1.0), seg[1] + vec2(1.0, 1.0)], shadow(0.8));
            p.line_segment(seg, s);
        }
        p.circle_stroke(q, 4.0, s);
        register(&ctx, "overlay:guides", Rect::from_center_size(q, vec2(9.0, 9.0)));
    }
    dragging
}

/// The toolbar's ovl. picker: when overlays show, the composition guide, aspect-ratio frames,
/// grid, guides and layout image, each with its settings. Everything goes through the commands.
pub fn overlay_menu(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.set_min_width(250.0);
    let o = app.ui.loupe_overlay.clone();
    let weak = |ui: &mut egui::Ui, s: &str| {
        ui.label(egui::RichText::new(crate::i18n::tr(s)).weak());
    };
    weak(ui, "Show");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (label, m) in [("always.", OverlayShow::Always), ("adjusting.", OverlayShow::Auto), ("never.", OverlayShow::Never)] {
            if ui.selectable_label(o.show == m, label).clicked() {
                let _ = app.run("view.overlayShow", json!({"mode": m.id()}));
            }
        }
    });
    ui.separator();
    weak(ui, "Composition (O cycles, ⇧O turns)");
    egui::Grid::new("ovl-composition").num_columns(2).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
        for (i, k) in CropOverlay::ALL.into_iter().enumerate() {
            if ui.selectable_label(o.composition == k, crate::i18n::tr(k.label())).clicked() {
                let _ = app.run("view.compositionOverlay", json!({"overlay": k.id()}));
            }
            if i % 2 == 1 {
                ui.end_row();
            }
        }
    });
    if matches!(o.composition, CropOverlay::Triangle | CropOverlay::Spiral) && ui.button("turn.").clicked() {
        let _ = app.run("view.compositionOverlayOrientation", json!({}));
    }
    ui.separator();
    weak(ui, "Aspect ratio frames");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        for f in crate::state::FRAME_GUIDES {
            if ui.selectable_label(o.frames.iter().any(|x| x == f), f).clicked() {
                let _ = app.run("view.frameGuides", json!({"toggle": f}));
            }
        }
        // custom ratios already chosen
        for f in o.frames.iter().filter(|f| !crate::state::FRAME_GUIDES.contains(&f.as_str())) {
            if ui.selectable_label(true, f).clicked() {
                let _ = app.run("view.frameGuides", json!({"toggle": f}));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let id = egui::Id::new("ovl-custom-frame");
        let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
        let edit = ui.add(egui::TextEdit::singleline(&mut text).hint_text("w:h").desired_width(70.0));
        let add = ui.button("add.");
        if (add.clicked() || edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) && !text.trim().is_empty() {
            match app.run("view.frameGuides", json!({"toggle": text.trim()})) {
                Ok(_) => text.clear(),
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        ui.data_mut(|d| d.insert_temp(id, text));
        let mut dim = o.frame_dim;
        if ui.checkbox(&mut dim, crate::i18n::tr("dim outside")).changed() {
            let _ = app.run("view.frameGuides", json!({"dim": dim}));
        }
    });
    let mut lo = o.line_opacity;
    if ui.add(egui::Slider::new(&mut lo, 0.05..=1.0).text(crate::i18n::tr("line opacity")).fixed_decimals(2)).changed() {
        let _ = app.run("view.frameGuides", json!({"opacity": lo}));
    }
    ui.separator();
    weak(ui, "Loupe overlay (⌘⌥O)");
    let mut grid = o.grid;
    if ui.checkbox(&mut grid, crate::i18n::tr("Grid")).changed() {
        let _ = app.run("view.loupeGrid", json!({"on": grid}));
    }
    if o.grid {
        let (mut size, mut op) = (o.grid_size, o.grid_opacity);
        if ui.add(egui::Slider::new(&mut size, 8.0..=200.0).text(crate::i18n::tr("size")).fixed_decimals(0)).changed() {
            let _ = app.run("view.loupeGrid", json!({"size": size}));
        }
        if ui.add(egui::Slider::new(&mut op, 0.05..=1.0).text(crate::i18n::tr("opacity")).fixed_decimals(2)).changed() {
            let _ = app.run("view.loupeGrid", json!({"opacity": op}));
        }
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let mut guides = o.guides;
        if ui.checkbox(&mut guides, crate::i18n::tr("Guides")).on_hover_text(crate::i18n::tr("⌘-drag a guide to move it")).changed() {
            let _ = app.run("view.loupeGuides", json!({"on": guides}));
        }
        if o.guides && ui.button("reset.").clicked() {
            let _ = app.run("view.loupeGuides", json!({"reset": true}));
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let mut layout = o.layout;
        // never greyed out: with no image chosen yet, ticking it asks for one
        if ui.checkbox(&mut layout, crate::i18n::tr("Layout image")).changed() {
            let cmd = if o.layout_path.is_empty() { "view.chooseLayoutImage" } else { "view.layoutImage" };
            if let Err(e) = app.run(cmd, json!({"on": layout})) {
                app.toast(ui.ctx(), e);
            }
        }
        if ui.button("choose.").on_hover_text(crate::i18n::tr("Choose Layout Image… (a PNG with transparency works best)")).clicked()
            && let Err(e) = app.run("view.chooseLayoutImage", json!({}))
        {
            app.toast(ui.ctx(), e);
        }
    });
    if !o.layout_path.is_empty() {
        let name = std::path::Path::new(&o.layout_path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        ui.label(egui::RichText::new(name).weak().small());
        let mut op = o.layout_opacity;
        if ui.add(egui::Slider::new(&mut op, 0.05..=1.0).text(crate::i18n::tr("opacity")).fixed_decimals(2)).changed() {
            let _ = app.run("view.layoutImage", json!({"opacity": op}));
        }
    }
}

/// The largest `ratio` (width / height) rect centred in `img`.
pub fn frame_rect(img: Rect, ratio: f32) -> Rect {
    let (w, h) =
        if ratio >= img.width() / img.height().max(1e-3) { (img.width(), img.width() / ratio) } else { (img.height() * ratio, img.height()) };
    Rect::from_center_size(img.center(), vec2(w, h))
}

/// ⌘-drag near a guide (within 6 points; the crossing moves both): it follows the pointer.
fn guides_drag(app: &mut LightcraftApp, ui: &egui::Ui, resp: &egui::Response, img: Rect, active: bool) -> bool {
    let key = egui::Id::new("loupe-guide-drag");
    // which guides the drag holds: (vertical, horizontal)
    let mut held: Option<(bool, bool)> = ui.data(|d| d.get_temp(key));
    if !active {
        ui.data_mut(|d| d.remove::<(bool, bool)>(key));
        return false;
    }
    let cmd = ui.input(|i| i.modifiers.command);
    let g = app.ui.loupe_overlay.guides_at;
    let q = pos2(img.left() + g.0 * img.width(), img.top() + g.1 * img.height());
    let near = |pt: egui::Pos2| ((pt.x - q.x).abs() <= 6.0, (pt.y - q.y).abs() <= 6.0);
    if cmd
        && held.is_none()
        && let Some(pt) = resp.hover_pos()
    {
        let (v, h) = near(pt);
        if v || h {
            ui.ctx().set_cursor_icon(match (v, h) {
                (true, true) => egui::CursorIcon::Move,
                (true, false) => egui::CursorIcon::ResizeHorizontal,
                _ => egui::CursorIcon::ResizeVertical,
            });
        }
    }
    // where the button went down (the drag starts a few points later)
    if resp.drag_started()
        && cmd
        && let Some(pt) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let (v, h) = near(pt);
        if v || h {
            held = Some((v, h));
            ui.data_mut(|d| d.insert_temp(key, (v, h)));
        }
    }
    let Some((v, h)) = held else { return false };
    if let Some(pt) = resp.interact_pointer_pos()
        && (resp.dragged() || resp.drag_stopped())
    {
        let u = ((pt.x - img.left()) / img.width()).clamp(0.0, 1.0);
        let w = ((pt.y - img.top()) / img.height()).clamp(0.0, 1.0);
        let at = (if v { u } else { g.0 }, if h { w } else { g.1 });
        app.ui.loupe_overlay.guides_at = at;
    }
    if resp.drag_stopped() || !(resp.dragged() || resp.drag_started()) {
        ui.data_mut(|d| d.remove::<(bool, bool)>(key));
    }
    true
}

/// Guide lines for `kind` on a `w × h` (pixels, any unit) crop. `orient` (0..4) mirrors the
/// asymmetric guides (triangle, spiral): bit 0 across, bit 1 down.
pub fn paths(kind: CropOverlay, orient: u8, w: f32, h: f32) -> Vec<Vec<(f32, f32)>> {
    let (w, h) = (w.max(1e-3), h.max(1e-3));
    let lines = |fracs: &[f32]| fracs.iter().flat_map(|&f| [vec![(f, 0.0), (f, 1.0)], vec![(0.0, f), (1.0, f)]]).collect();
    let out: Vec<Vec<(f32, f32)>> = match kind {
        CropOverlay::None => vec![],
        CropOverlay::Thirds => lines(&[1.0 / 3.0, 2.0 / 3.0]),
        CropOverlay::Grid => lines(&(1..8).map(|i| i as f32 / 8.0).collect::<Vec<_>>()),
        CropOverlay::Golden => lines(&[0.382, 0.618]),
        // 45° (in pixels) from each corner until it meets the far side
        CropOverlay::Diagonal => {
            let (du, dv) = (w.min(h) / w, w.min(h) / h);
            vec![
                vec![(0.0, 0.0), (du, dv)],
                vec![(1.0, 0.0), (1.0 - du, dv)],
                vec![(0.0, 1.0), (du, 1.0 - dv)],
                vec![(1.0, 1.0), (1.0 - du, 1.0 - dv)],
            ]
        }
        // the main diagonal and the perpendiculars to it from the other two corners
        CropOverlay::Triangle => {
            let foot = |px: f32, py: f32| {
                let n = (w * w + h * h).sqrt();
                let (dx, dy) = (w / n, h / n);
                let t = px * dx + py * dy;
                (t * dx / w, t * dy / h)
            };
            vec![vec![(0.0, 0.0), (1.0, 1.0)], vec![(1.0, 0.0), foot(w, 0.0)], vec![(0.0, 1.0), foot(0.0, h)]]
        }
        CropOverlay::Spiral => spiral(w, h),
        CropOverlay::Fifths => lines(&[0.2, 0.4, 0.6, 0.8]),
        // the centre lines, and a small square at the middle (a twentieth of the short side)
        CropOverlay::Center => {
            let (du, dv) = (0.05 * w.min(h) / w, 0.05 * w.min(h) / h);
            let (a, b) = (0.5 - du, 0.5 + du);
            let (c, d) = (0.5 - dv, 0.5 + dv);
            vec![vec![(0.5, 0.0), (0.5, 1.0)], vec![(0.0, 0.5), (1.0, 0.5)], vec![(a, c), (b, c), (b, d), (a, d), (a, c)]]
        }
        // 1:1, 4:5, 2:3 and 16:9 in the crop's orientation, each as large as fits, centred
        CropOverlay::AspectRatios => [1.0f32, 5.0 / 4.0, 3.0 / 2.0, 16.0 / 9.0]
            .iter()
            .map(|&long| {
                let r = if w >= h { long } else { 1.0 / long };
                let (rw, rh) = if r <= w / h { (h * r / w, 1.0) } else { (1.0, w / r / h) };
                let (u0, v0) = ((1.0 - rw) / 2.0, (1.0 - rh) / 2.0);
                vec![(u0, v0), (u0 + rw, v0), (u0 + rw, v0 + rh), (u0, v0 + rh), (u0, v0)]
            })
            .collect(),
    };
    let (fu, fv) = (orient & 1 == 1, orient & 2 == 2);
    let flip = |(u, v): (f32, f32)| (if fu { 1.0 - u } else { u }, if fv { 1.0 - v } else { v });
    match kind {
        CropOverlay::Triangle | CropOverlay::Spiral => out.into_iter().map(|l| l.into_iter().map(flip).collect()).collect(),
        _ => out,
    }
}

/// The golden spiral: squares carved off a `w × h` rectangle (left, top, right, bottom, …), a
/// quarter circle in each, joined into one curve; plus the square boundaries.
fn spiral(w: f32, h: f32) -> Vec<Vec<(f32, f32)>> {
    // work landscape; transpose back for portrait crops
    let portrait = h > w;
    let (rw, rh) = if portrait { (h, w) } else { (w, h) };
    let (mut x, mut y, mut cw, mut ch) = (0.0f32, 0.0f32, rw, rh);
    let mut curve = Vec::new();
    let mut squares = Vec::new();
    for k in 0..10 {
        let s = cw.min(ch);
        if s < rw.min(rh) * 0.01 {
            break;
        }
        // square corner origin, arc centre, start angle (degrees, y down)
        let (sx, sy, cx, cy, a0) = match k % 4 {
            0 => (x, y, x + s, y + s, 180.0f32),
            1 => (x, y, x, y + s, 270.0),
            2 => (x + cw - s, y, x + cw - s, y, 0.0),
            _ => (x, y + ch - s, x + s, y + ch - s, 90.0),
        };
        for i in 0..=12 {
            let a = (a0 + 90.0 * i as f32 / 12.0).to_radians();
            curve.push((cx + s * a.cos(), cy + s * a.sin()));
        }
        squares.push(vec![(sx, sy), (sx + s, sy), (sx + s, sy + s), (sx, sy + s), (sx, sy)]);
        match k % 4 {
            0 => {
                x += s;
                cw -= s;
            }
            1 => {
                y += s;
                ch -= s;
            }
            2 => cw -= s,
            _ => ch -= s,
        }
    }
    let norm = |(px, py): (f32, f32)| if portrait { (py / rh, px / rw) } else { (px / rw, py / rh) };
    squares.push(curve);
    squares.into_iter().map(|l| l.into_iter().map(norm).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_unit(paths: &[Vec<(f32, f32)>]) -> bool {
        paths.iter().flatten().all(|&(u, v)| (-1e-4..=1.0001).contains(&u) && (-1e-4..=1.0001).contains(&v))
    }

    #[test]
    fn every_overlay_stays_inside_the_crop() {
        for kind in CropOverlay::ALL.into_iter().filter(|k| *k != CropOverlay::None) {
            for (w, h) in [(3.0, 2.0), (2.0, 3.0), (1.0, 1.0), (16.0, 9.0)] {
                for o in 0..4 {
                    let p = paths(kind, o, w, h);
                    assert!(!p.is_empty() && in_unit(&p), "{kind:?} {w}×{h} {o}");
                }
            }
        }
        assert!(paths(CropOverlay::None, 0, 3.0, 2.0).is_empty());
    }

    #[test]
    fn diagonals_are_45_degrees_and_triangle_feet_are_perpendicular() {
        let (w, h) = (3.0, 2.0);
        for l in paths(CropOverlay::Diagonal, 0, w, h) {
            let (a, b) = (l[0], l[1]);
            assert!((((b.0 - a.0) * w).abs() - ((b.1 - a.1) * h).abs()).abs() < 1e-4, "{l:?}");
        }
        let t = paths(CropOverlay::Triangle, 0, w, h);
        for l in &t[1..] {
            let (a, b) = (l[0], l[1]);
            // perpendicular to the main diagonal (w, h) in pixels
            let dot = (b.0 - a.0) * w * w + (b.1 - a.1) * h * h;
            assert!(dot.abs() < 1e-3, "{l:?}");
        }
        // orientation mirrors the triangle
        assert_eq!(paths(CropOverlay::Triangle, 1, w, h)[0], vec![(1.0, 0.0), (0.0, 1.0)]);
    }

    #[test]
    fn aspect_ratio_outlines_have_their_shapes() {
        let (w, h) = (3.0, 2.0);
        let p = paths(CropOverlay::AspectRatios, 0, w, h);
        assert_eq!(p.len(), 4);
        for (l, want) in p.iter().zip([1.0f32, 1.25, 1.5, 16.0 / 9.0]) {
            let (rw, rh) = ((l[1].0 - l[0].0) * w, (l[2].1 - l[1].1) * h);
            assert!((rw / rh - want).abs() < 1e-3, "{rw}×{rh} vs {want}");
        }
        // portrait crops get portrait outlines
        let p = paths(CropOverlay::AspectRatios, 0, 2.0, 3.0);
        assert!(p.iter().all(|l| (l[1].0 - l[0].0) * 2.0 <= (l[2].1 - l[1].1) * 3.0 + 1e-4));
        assert_eq!(paths(CropOverlay::Fifths, 0, w, h).len(), 8);
    }

    #[test]
    fn overlays_cycle_through_the_chosen_ones() {
        use CropOverlay as O;
        assert_eq!(O::Thirds.next_in(&[]), O::Grid);
        assert_eq!(O::None.next_in(&[]), O::Thirds);
        let cycle = [O::Thirds, O::Fifths, O::None];
        assert_eq!(O::Thirds.next_in(&cycle), O::Fifths);
        assert_eq!(O::Fifths.next_in(&cycle), O::None);
        assert_eq!(O::None.next_in(&cycle), O::Thirds);
        assert_eq!(O::Grid.next_in(&cycle), O::Fifths, "from one not in the cycle: the next chosen one");
    }

    #[test]
    fn spiral_is_one_continuous_curve() {
        for (w, h) in [(1.618, 1.0), (1.0, 1.618), (3.0, 2.0)] {
            let p = paths(CropOverlay::Spiral, 0, w, h);
            let curve = p.last().unwrap();
            for pair in curve.windows(2) {
                let d = ((pair[1].0 - pair[0].0) * w).hypot((pair[1].1 - pair[0].1) * h);
                assert!(d < 0.2 * w.max(h), "jump {d} in {w}×{h}");
            }
        }
    }
}
