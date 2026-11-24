//! Develop ▸ Lens Blur (Lightroom Classic 13+): Apply, Blur Amount, Bokeh, Boost, the Focal Range over the photo's
//! depth (with the depth histogram behind it and a focus picker), Visualize Depth (the map, near warm, far cool),
//! and Refine brushes that paint focus or blur. Every change is a command (`lensBlur.*`, `develop.set`).

use egui::{Color32, Sense, vec2};
use lightcraft_develop::{Bokeh, DevelopSettings};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;

fn row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 20, top: 4, bottom: 4 }).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
            add(ui);
        });
    });
}

/// Near warm, far cool (Lightroom's depth visualisation).
fn depth_color(d: f32) -> Color32 {
    let d = d.clamp(0.0, 1.0);
    let (a, b) = ([250.0, 200.0, 60.0], [70.0, 50.0, 170.0]);
    let c: [f32; 3] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * d);
    Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8)
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings) {
    let t = Tokens::get(ui.ctx());
    let l = d.lens_blur.clone();
    row(ui, |ui| {
        let mut on = l.enabled;
        let r = ui.checkbox(&mut on, crate::i18n::tr("Apply"));
        crate::widgets::register(ui.ctx(), "checkbox:lensBlurApply", r.rect);
        if r.changed() {
            match app.run("lensBlur.apply", json!({"on": on})) {
                Ok(_) => {}
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        if d.depth.is_none() {
            ui.label(egui::RichText::new(crate::i18n::tr("no depth map yet")).color(t.text_dim).size(12.0));
        }
    });
    // the depth model, when it is missing (its download needs the user's click)
    let models = app.session.enhancer.models_status();
    let depth_missing = models["models"].as_array().and_then(|a| a.iter().find(|m| m["model"] == "depth")).is_some_and(|m| m["installed"] != true);
    if d.depth.is_none() && depth_missing {
        row(ui, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Depth Anything V2 model:")).color(t.text_label).size(12.0));
        });
        egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 20, top: 0, bottom: 4 }).show(ui, |ui| {
            crate::enhance::model_row(app, ui, "depth", &models);
        });
    }
    if !l.enabled {
        ui.add_space(6.0);
        return;
    }
    super::edit::control(app, ui, d, "lensBlur.amount", true);
    row(ui, |ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Bokeh")).color(t.text_label).size(12.0));
        for b in Bokeh::ALL {
            let key = serde_json::to_value(b).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
            if crate::widgets::text_button(ui, &format!("lensBlurBokeh-{key}"), b.label(), l.bokeh == b).clicked() && l.bokeh != b {
                let _ = app.run("lensBlur.bokeh", json!({"shape": key}));
            }
        }
    });
    super::edit::control(app, ui, d, "lensBlur.boost", true);
    super::edit::sub_title(ui, "Focal Range");
    depth_bar(ui, d);
    row(ui, |ui| {
        let picking = app.ui.tool == "lensFocus";
        if crate::widgets::text_button(ui, "lensBlurFocus", "focus.", picking)
            .on_hover_text(crate::i18n::tr("Click the photo where it should be in focus"))
            .clicked()
        {
            app.ui.tool = if picking { String::new() } else { "lensFocus".into() };
        }
        let vis = app.enhance.visualize_depth;
        if crate::widgets::text_button(ui, "lensBlurVisualize", "depth.", vis).on_hover_text(crate::i18n::tr("Visualize Depth")).clicked() {
            app.enhance.visualize_depth = !vis;
        }
        if d.depth.is_some()
            && crate::widgets::text_button(ui, "lensBlurRedepth", "redo.", false).on_hover_text(crate::i18n::tr("Estimate the depth again")).clicked()
            && let Err(e) = app.run("enhance.depth", json!({}))
        {
            app.toast(ui.ctx(), e);
        }
    });
    super::edit::control(app, ui, d, "lensBlur.focalDistance", true);
    super::edit::control(app, ui, d, "lensBlur.focalRange", true);
    if app.enhance.visualize_depth {
        depth_map(ui, d);
    }
    super::edit::sub_title(ui, "Refine");
    row(ui, |ui| {
        for (kind, label, tip) in [("focus", "focus.brush.", "Paint what stays sharp"), ("blur", "blur.brush.", "Paint what is fully blurred")] {
            let tool = format!("lensBrush:{kind}");
            let on = app.ui.tool == tool;
            if crate::widgets::text_button(ui, &format!("lensBlurBrush-{kind}"), label, on).on_hover_text(crate::i18n::tr(tip)).clicked() {
                app.ui.tool = if on { String::new() } else { tool };
            }
        }
        let n = l.focus_strokes.len() + l.blur_strokes.len();
        if crate::widgets::text_button(ui, "lensBlurBrushClear", "clear.", false).on_hover_text(crate::i18n::tr("Remove the brush strokes")).clicked()
            && n > 0
        {
            let _ = app.run("lensBlur.clearBrushes", json!({}));
        }
        ui.label(egui::RichText::new(format!("{n}")).color(t.text_dim).size(12.0));
    });
    if app.ui.tool.starts_with("lensBrush:") {
        let mut size = app.enhance.brush_size;
        if let Some(v) = crate::widgets::slider(ui, &BRUSH_SIZE, size, true, None).value {
            size = v;
            app.enhance.brush_size = size;
        }
    }
    ui.add_space(6.0);
}

/// The depth histogram with the in-focus band over it.
fn depth_bar(ui: &mut egui::Ui, d: &DevelopSettings) {
    let t = Tokens::get(ui.ctx());
    let w = (ui.available_width() - 32.0).max(80.0);
    egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 20, top: 2, bottom: 6 }).show(ui, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
        crate::widgets::register(ui.ctx(), "lensBlur.depthBar", r);
        let p = ui.painter();
        p.rect_filled(r, 3.0, t.canvas);
        let Some(g) = d.depth.as_ref().and_then(|m| m.grid()) else { return };
        let bins = g.histogram(64);
        let peak = bins.iter().cloned().fold(1e-6, f32::max);
        let bw = r.width() / bins.len() as f32;
        for (i, v) in bins.iter().enumerate() {
            let h = (v / peak).sqrt() * (r.height() - 4.0);
            let x = r.left() + i as f32 * bw;
            let c = depth_color((i as f32 + 0.5) / bins.len() as f32);
            p.rect_filled(egui::Rect::from_min_max(egui::pos2(x, r.bottom() - h), egui::pos2(x + bw - 0.5, r.bottom())), 0.0, c.gamma_multiply(0.85));
        }
        let c = d.lens_blur.focal_distance.clamp(0.0, 100.0) as f32 / 100.0;
        let half = d.lens_blur.focal_range.clamp(0.0, 100.0) as f32 / 200.0;
        let (a, b) = ((c - half).max(0.0), (c + half).min(1.0));
        let band =
            egui::Rect::from_min_max(egui::pos2(r.left() + a * r.width(), r.top()), egui::pos2(r.left() + b.max(a + 0.004) * r.width(), r.bottom()));
        p.rect_stroke(band, 2.0, egui::Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
    });
}

/// Visualize Depth: the map itself.
fn depth_map(ui: &mut egui::Ui, d: &DevelopSettings) {
    let Some(g) = d.depth.as_ref().and_then(|m| m.grid()) else { return };
    let w = (ui.available_width() - 32.0).max(80.0);
    let h = w * g.height as f32 / g.width.max(1) as f32;
    egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 20, top: 2, bottom: 6 }).show(ui, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        crate::widgets::register(ui.ctx(), "lensBlur.depthMap", r);
        let (cols, rows) = (48usize, ((48.0 * h / w) as usize).max(1));
        let (cw, ch) = (r.width() / cols as f32, r.height() / rows as f32);
        let p = ui.painter();
        for y in 0..rows {
            for x in 0..cols {
                let v = g.sample((x as f64 + 0.5) / cols as f64, (y as f64 + 0.5) / rows as f64);
                let cell = egui::Rect::from_min_size(egui::pos2(r.left() + x as f32 * cw, r.top() + y as f32 * ch), vec2(cw + 0.5, ch + 0.5));
                p.rect_filled(cell, 0.0, depth_color(v));
            }
        }
    });
}

/// Refine brush size (of the long edge, ‰).
static BRUSH_SIZE: lightcraft_develop::ControlSpec = lightcraft_develop::ControlSpec {
    id: "lensBlur.brushSize",
    label: "Size",
    section: lightcraft_develop::Section::LensBlur,
    min: 5.0,
    max: 200.0,
    default: 30.0,
    step: 1.0,
    decimals: 0,
    track: lightcraft_develop::Track::Plain,
};

/// The photo canvas while a Lens Blur tool is active: a click focuses, a drag paints a Refine stroke.
/// `norm` maps a screen point to normalised photo coordinates.
pub fn canvas(app: &mut LightcraftApp, resp: &egui::Response, norm: impl Fn(egui::Pos2) -> lightcraft_geom::Point) -> bool {
    if app.ui.tool == "lensFocus" {
        if resp.clicked()
            && let Some(q) = resp.interact_pointer_pos()
        {
            let n = norm(q);
            if let Err(e) = app.run("lensBlur.focusAt", json!({"x": n.x, "y": n.y})) {
                app.ui.status = e;
            }
            app.ui.tool.clear();
        }
        return true;
    }
    let Some(kind) = app.ui.tool.strip_prefix("lensBrush:").map(str::to_string) else { return false };
    if (resp.dragged() || resp.drag_started() || resp.clicked())
        && let Some(q) = resp.interact_pointer_pos()
    {
        let n = norm(q);
        app.enhance.lens_stroke.push([n.x, n.y]);
    }
    if (resp.drag_stopped() || resp.clicked()) && !app.enhance.lens_stroke.is_empty() {
        let points: Vec<Value> = std::mem::take(&mut app.enhance.lens_stroke).into_iter().map(|p| json!(p)).collect();
        let size = app.enhance.brush_size / 1000.0;
        if let Err(e) = app.run("lensBlur.brush", json!({"kind": kind, "points": points, "size": size})) {
            app.ui.status = e;
        }
    }
    true
}
