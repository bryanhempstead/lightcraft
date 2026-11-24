//! Window regions and panels.

pub mod bottombar;
pub mod chips;
pub mod columns;
pub mod compare;
pub mod crop_overlay;
pub mod detail;
pub mod develop_left;
pub mod dialogs;
pub mod edit;
pub mod filmstrip;
pub mod filterbar;
pub mod galleries;
pub mod grid;
pub mod hdr_histogram;
pub mod left;
pub mod lens_blur;
pub mod library_problem;
pub mod library_right;
pub mod map;
pub mod mask_effects;
pub mod masking;
pub mod notices;
pub mod order;
pub mod people;
pub mod presets;
pub mod profiles;
pub mod right;
pub mod rules_editor;
pub mod second;
pub mod settings;
pub mod topbar;

use egui::{Align2, Rect, pos2, vec2};

use crate::LightcraftApp;
use crate::theme::Tokens;

/// Show a side panel the user resizes by dragging its inner edge. `width` (in [`crate::UiState`],
/// so it survives navigation and restarts) is the source of truth: egui's own remembered size is
/// dropped every frame, and a drag of the edge returns the new width to store. The panel never
/// takes more than `available − reserve − MIN_PHOTO_WIDTH` (its minimum permitting), so the photo
/// stays usable; a narrower window shrinks it without forgetting the chosen width.
pub fn resizable_side(
    ui: &mut egui::Ui,
    left: bool,
    id: &str,
    frame: egui::Frame,
    width: f32,
    limits: crate::state::PanelWidth,
    reserve: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> Option<f32> {
    let pid = egui::Id::new(id);
    ui.ctx().data_mut(|d| d.remove::<egui::containers::panel::PanelState>(pid));
    let avail = ui.available_rect_before_wrap();
    let max = (avail.width() - reserve - crate::state::MIN_PHOTO_WIDTH).clamp(limits.min, limits.max);
    let panel = if left { egui::Panel::left(pid) } else { egui::Panel::right(pid) };
    let r = panel.frame(frame).resizable(true).size_range(limits.min..=max).default_size(limits.clamp(width).min(max)).show(ui, add);
    crate::widgets::register(ui.ctx(), format!("panel:{id}"), r.response.rect);
    // egui names the edge's drag widget `<panel id>.with("__resize")`; the width is measured from
    // the panel's fixed edge to the pointer (contents wider than the panel don't count)
    if !ui.ctx().is_being_dragged(pid.with("__resize")) {
        return None;
    }
    let x = ui.ctx().pointer_interact_pos()?.x;
    let w = if left { x - avail.left() } else { avail.right() - x };
    Some(w.round().clamp(limits.min, max))
}

/// The HUD toast at the bottom centre of the canvas.
pub fn toast(app: &mut LightcraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    let Some((text, mut until, label)) = app.ui.toast.clone() else { return };
    // (a negative expiry is a duration that starts now: `LightcraftApp::toast_soon`)
    if until < 0.0 {
        until = now - until;
        if let Some(t) = app.ui.toast.as_mut() {
            t.1 = until;
        }
    }
    if now > until {
        app.ui.toast = None;
        return;
    }
    let Some(canvas) = app.canvas_rect else { return };
    let t = Tokens::get(ctx);
    let fade = ((until - now) / 0.25).clamp(0.0, 1.0) as f32;
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("toast")));
    // long messages (a save warning, the web build's notices) wrap within the canvas
    let wrap = (canvas.width() - 80.0).clamp(200.0, 720.0);
    let (background, foreground) = label.map(crate::theme::label_toast_colors).unwrap_or((egui::Color32::from_black_alpha(200), t.text));
    let foreground = foreground.gamma_multiply(fade);
    // a delete's toast: "3 photos moved to the Trash · undo." (the button runs Undo)
    let undo = app.ui.toast_undo.as_deref() == Some(text.as_str());
    let shown = if undo { format!("{text} ·") } else { text };
    let galley = painter.layout(shown, t.font(14.0), foreground, wrap);
    let button_w = if undo {
        (painter.layout_no_wrap(crate::i18n::tr("undo.").to_string(), t.semibold(11.5), t.text).size().x + 18.0).max(37.0) + 10.0
    } else {
        0.0
    };
    let size = vec2(galley.size().x + 40.0 + button_w, (galley.size().y + 22.0).max(40.0));
    let r = Rect::from_center_size(pos2(canvas.center().x, canvas.bottom() - 60.0), size);
    painter.rect_filled(r, 6.0, background.gamma_multiply(fade));
    let text_at = pos2(r.left() + 20.0, r.center().y - galley.size().y / 2.0);
    painter.galley(text_at, galley, foreground);
    if undo {
        let at = pos2(r.right() - 20.0 - (button_w - 10.0), r.center().y - 12.0);
        let clicked = egui::Area::new(egui::Id::new("toast-undo"))
            .order(egui::Order::Tooltip)
            .fixed_pos(at)
            .show(ctx, |ui| crate::widgets::text_button(ui, "toastUndo", "undo.", false).clicked())
            .inner;
        if clicked {
            let msg = match app.run("edit.undo", serde_json::json!({})) {
                Ok(v) => crate::menus::undo_message(&v),
                Err(e) => e,
            };
            app.ui.toast_undo = None;
            app.toast(ctx, msg);
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(30));
}

/// Paint a centred, dimmed message (empty states).
pub fn empty_message(ui: &egui::Ui, rect: Rect, title: &str, body: &str) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    p.text(rect.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, crate::i18n::tr(title), t.semibold(18.0), t.text_label);
    p.text(rect.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, crate::i18n::tr(body), t.font(13.0), t.text_dim);
}

/// Lights Out (L): everything but the photo painted over, dimmed then black (Esc or L again ends
/// it). Covers the window around the displayed image, above the panels and below dialogs.
pub fn lights_out(app: &mut LightcraftApp, ctx: &egui::Context) {
    let a = app.ui.lights_out.alpha();
    if a == 0 {
        return;
    }
    let screen = ctx.content_rect();
    let photo = app.image_rect.filter(|_| matches!(app.ui.view, crate::state::ViewMode::Detail | crate::state::ViewMode::Reference));
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::new("lights-out")));
    let c = egui::Color32::from_black_alpha(a);
    match photo.map(|p| p.intersect(screen)).filter(|p| p.is_positive()) {
        Some(p) => {
            for r in [
                Rect::from_min_max(screen.min, pos2(screen.right(), p.top())),
                Rect::from_min_max(pos2(screen.left(), p.bottom()), screen.max),
                Rect::from_min_max(pos2(screen.left(), p.top()), pos2(p.left(), p.bottom())),
                Rect::from_min_max(pos2(p.right(), p.top()), pos2(screen.right(), p.bottom())),
            ] {
                painter.rect_filled(r, 0.0, c);
            }
        }
        None => {
            painter.rect_filled(screen, 0.0, c);
        }
    }
    // a click anywhere brings the lights back (Classic)
    if ctx.input(|i| i.pointer.any_click()) {
        app.ui.lights_out = crate::state::LightsOut::On;
    }
}

/// Lightroom's edge tabs (Bryan, 2026-11-23): a small arrow on the window's left, right and bottom
/// edges hides / shows the left panel, the right panel and the filmstrip. Faint until hovered; the
/// arrow points where the panel goes.
pub fn edge_toggles(app: &mut crate::LightcraftApp, ctx: &egui::Context) {
    use egui::{Color32, Order, Rect, Sense, pos2, vec2};
    let t = crate::theme::Tokens::get(ctx);
    let w = ctx.content_rect();
    let (long, short) = (44.0, 14.0);
    // (id, tab rect, arrow pointing: -1 left / +1 right / 2 down / -2 up, command)
    let left = if app.ui.left_panel { -1 } else { 1 };
    let right = if app.ui.right_panel { 1 } else { -1 };
    let bottom = if app.ui.filmstrip { 2 } else { -2 };
    let tabs = [
        ("edgeLeft", Rect::from_min_size(pos2(w.left(), w.center().y - long / 2.0), vec2(short, long)), left, "view.leftPanel", "Hide / show the left panel"),
        ("edgeRight", Rect::from_min_size(pos2(w.right() - short, w.center().y - long / 2.0), vec2(short, long)), right, "view.rightPanel", "Hide / show the right panel"),
        ("edgeBottom", Rect::from_min_size(pos2(w.center().x - long / 2.0, w.bottom() - short), vec2(long, short)), bottom, "view.filmstrip", "Hide / show the filmstrip"),
    ];
    for (id, r, dir, cmd, tip) in tabs {
        let resp = egui::Area::new(egui::Id::new(id))
            .order(Order::Foreground)
            .fixed_pos(r.min)
            .show(ctx, |ui| {
                let (rr, resp) = ui.allocate_exact_size(r.size(), Sense::click());
                let hot = resp.hovered();
                ui.painter().rect_filled(rr, 4.0, if hot { t.header } else { t.header.gamma_multiply(0.55) });
                let c = rr.center();
                let col = if hot { t.text } else { Color32::from_gray(150) };
                let (a, b) = (4.0, 3.0);
                let pts = match dir {
                    -1 => vec![c + vec2(-b, 0.0), c + vec2(b, -a), c + vec2(b, a)],
                    1 => vec![c + vec2(b, 0.0), c + vec2(-b, -a), c + vec2(-b, a)],
                    2 => vec![c + vec2(0.0, b), c + vec2(-a, -b), c + vec2(a, -b)],
                    _ => vec![c + vec2(0.0, -b), c + vec2(-a, b), c + vec2(a, b)],
                };
                ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
                crate::widgets::register(ui.ctx(), format!("button:{id}"), rr);
                resp.on_hover_text(crate::i18n::tr(tip))
            })
            .inner;
        if resp.clicked() {
            let _ = app.run(cmd, serde_json::json!({}));
        }
    }
}
