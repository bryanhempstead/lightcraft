//! The Develop module's left panel (Lightroom Classic): Navigator (FIT / FILL / 1:1 / 2:1 and a
//! pan box over the photo), Presets, Snapshots, History and Collections (the library's albums, as a
//! read-only shortcut), with "Copy…" and "Paste" at the bottom.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::{Album, AlbumId, PhotoId};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::state::Zoom;
use crate::theme::Tokens;
use crate::widgets::register;

/// The left panel's sections in Classic's order: (id, title).
pub const SECTIONS: [(&str, &str); 5] =
    [("navigator", "Navigator"), ("presets", "Presets"), ("snapshots", "Snapshots"), ("history", "History"), ("collections", "Collections")];

/// Height of the Copy… / Paste row at the bottom.
const FOOTER_H: f32 = 40.0;

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let width = app.ui.left_width;
    let resized = super::resizable_side(ui, true, "left_panel", frame, width, crate::state::LEFT_WIDTH, 0.0, |ui| {
        let full = ui.max_rect();
        let body = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - FOOTER_H));
        let mut top = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        crate::widgets::set_classic_rows(top.ctx(), true);
        egui::ScrollArea::vertical().id_salt("develop-left-scroll").auto_shrink([false, false]).show(&mut top, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let active = app.session.active();
            for (id, title) in SECTIONS {
                if !header(app, ui, id, title) {
                    continue;
                }
                match (id, active) {
                    ("navigator", Some(p)) => navigator(app, ui, p),
                    ("presets", _) => super::presets::list(app, ui),
                    ("snapshots", Some(p)) => super::right::without_headers(ui, |ui| super::right::versions(app, ui, p)),
                    ("history", Some(p)) => super::right::without_headers(ui, |ui| super::right::activity(app, ui, p)),
                    ("collections", _) => collections(app, ui),
                    _ => note(ui, "Select a photo"),
                }
            }
            ui.add_space(12.0);
        });
        crate::widgets::set_classic_rows(ui.ctx(), false);
        footer(app, ui, Rect::from_min_max(pos2(full.left(), body.bottom()), full.max));
        // the panel keeps its width (its contents are drawn in child areas)
        ui.expand_to_include_rect(full);
    });
    if let Some(w) = resized {
        app.ui.left_width = w;
    }
}

/// A section header; returns whether the section is open. ⌥-click = solo.
fn header(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let open = app.ui.develop_left_sections.iter().any(|s| s == id);
    let (resp, _) = crate::widgets::classic_header(ui, &format!("dev.{id}"), title, open, false, None);
    if resp.clicked() {
        let solo = app.ui.solo_left || ui.input(|i| i.modifiers.alt);
        crate::state::UiState::toggle_in(&mut app.ui.develop_left_sections, id, solo && !open);
    }
    resp.context_menu(|ui| {
        let mut solo = app.ui.solo_left;
        if ui.checkbox(&mut solo, crate::i18n::tr("Solo Mode")).changed() {
            app.ui.solo_left = solo;
        }
    });
    open
}

fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(text), t.font(12.0), t.text_dim);
}

/// The zoom presets of the Navigator header: (label, `view.navigate` zoom).
pub const NAV_ZOOMS: [(&str, &str); 4] = [("FIT", "fit"), ("FILL", "fill"), ("1:1", "100"), ("2:1", "200")];

/// Which navigator zoom preset `z` is (index into [`NAV_ZOOMS`]).
pub fn nav_zoom_index(z: Zoom) -> Option<usize> {
    match z {
        Zoom::Fit => Some(0),
        Zoom::Fill => Some(1),
        Zoom::Percent(p) if (p - 100.0).abs() < 0.5 => Some(2),
        Zoom::Percent(p) if (p - 200.0).abs() < 0.5 => Some(3),
        Zoom::Percent(_) => None,
    }
}

/// The part of the photo the loupe shows, as a fraction of the photo (0..1 each way), from the
/// canvas and the displayed image rect. `None` when the whole photo is visible.
pub fn visible_fraction(canvas: Rect, image: Rect) -> Option<Rect> {
    if !(image.width() > 1.0 && image.height() > 1.0) {
        return None;
    }
    let v = canvas.intersect(image);
    if !v.is_positive() {
        return None;
    }
    let f = Rect::from_min_max(
        pos2((v.left() - image.left()) / image.width(), (v.top() - image.top()) / image.height()),
        pos2((v.right() - image.left()) / image.width(), (v.bottom() - image.top()) / image.height()),
    );
    if f.width() > 0.995 && f.height() > 0.995 { None } else { Some(f) }
}

fn navigator(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    // zoom presets
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let mut x = row.right() - 12.0;
    let cur = nav_zoom_index(app.ui.zoom);
    for (i, (label, z)) in NAV_ZOOMS.iter().enumerate().rev() {
        let g = ui.painter().layout_no_wrap(label.to_string(), t.semibold(11.0), t.text);
        let r = Rect::from_min_max(pos2(x - g.size().x - 6.0, row.top() + 3.0), pos2(x, row.bottom() - 3.0));
        let resp = ui.interact(r, egui::Id::new(("nav-zoom", i)), Sense::click());
        register(ui.ctx(), format!("button:navZoom-{z}"), r);
        let on = cur == Some(i);
        let c = if on {
            t.text
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_dim
        };
        ui.painter().galley(pos2(r.left() + 3.0, r.center().y - g.size().y / 2.0), g, c);
        if resp.clicked() {
            let zoom = match *z {
                "fit" => json!("fit"),
                "fill" => json!("fill"),
                pct => json!({"percent": pct.parse::<f32>().unwrap_or(100.0)}),
            };
            let _ = app.run("view.navigate", json!({"zoom": zoom}));
        }
        x = r.left() - 6.0;
    }
    // the photo with the visible area boxed; click or drag to pan there
    let w = ui.available_width();
    let (area, resp) = ui.allocate_exact_size(vec2(w, (w * 0.66).clamp(90.0, 260.0)), Sense::click_and_drag());
    register(ui.ctx(), "navigator", area);
    ui.painter().rect_filled(area, 0.0, t.canvas);
    super::grid::request_thumb(app, id, 384, 4);
    let Some(tex) = app.renderer.thumb(id) else { return };
    let [tw, th] = tex.size;
    let inner = area.shrink(10.0);
    let s = (inner.width() / tw.max(1) as f32).min(inner.height() / th.max(1) as f32);
    let img = Rect::from_center_size(inner.center(), vec2(tw as f32 * s, th as f32 * s));
    ui.painter().image(tex.tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    if let (Some(canvas), Some(shown)) = (app.canvas_rect, app.image_rect)
        && app.ui.view == crate::state::ViewMode::Detail
        && let Some(f) = visible_fraction(canvas, shown)
    {
        let b = Rect::from_min_max(
            pos2(img.left() + f.left() * img.width(), img.top() + f.top() * img.height()),
            pos2(img.left() + f.right() * img.width(), img.top() + f.bottom() * img.height()),
        );
        ui.painter().rect_stroke(b, 0.0, Stroke::new(2.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
        ui.painter().rect_stroke(b, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Outside);
    }
    if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        let pan = [((p.x - img.left()) / img.width()).clamp(0.0, 1.0), ((p.y - img.top()) / img.height()).clamp(0.0, 1.0)];
        // a click on the fitted photo zooms to 1:1 there (Classic)
        let zoom = if app.ui.zoom == Zoom::Fit || app.ui.zoom == Zoom::Fill {
            json!({"percent": 100.0})
        } else {
            serde_json::to_value(app.ui.zoom).unwrap_or_default()
        };
        let _ = app.run("view.navigate", json!({"zoom": zoom, "pan": pan}));
    }
}

/// Collections: the library's albums (folders open), click = show that album in Library.
fn collections(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let mut all: Vec<Album> = app.session.catalog.albums().cloned().collect();
    all.sort_by_key(|a| (!a.folder, a.name.to_lowercase()));
    if all.is_empty() {
        note(ui, "No collections yet");
        return;
    }
    ui.add_space(4.0);
    album_rows(app, ui, &all, None, 0, 0);
    ui.add_space(4.0);
}

fn album_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, all: &[Album], parent: Option<AlbumId>, depth: usize, guard: usize) {
    // a damaged catalog could loop its folders: never deeper than this
    if guard > 32 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    for a in all.iter().filter(|a| a.parent == parent) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        register(ui.ctx(), format!("devCollection:{}", a.id.0), r);
        let current = app.session.source == lightcraft_engine::LibrarySource::Album(a.id);
        if current {
            ui.painter().rect_filled(r, 0.0, t.tool_active);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.7));
        }
        let x = r.left() + 14.0 + depth as f32 * 14.0;
        let icon = if a.folder {
            Icon::CollectionSet
        } else if a.is_smart() {
            Icon::SmartAlbum
        } else {
            Icon::Album
        };
        paint(ui.painter(), Rect::from_min_size(pos2(x, r.center().y - 7.0), vec2(14.0, 14.0)), icon, t.icon);
        ui.painter().text(pos2(x + 20.0, r.center().y), Align2::LEFT_CENTER, &a.name, t.font(12.0), t.text_label);
        if !a.folder {
            let n = app.session.catalog.album_count(a.id);
            ui.painter().text(pos2(r.right() - 20.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), t.font(11.0), t.text_dim);
        }
        if resp.clicked() && !a.folder {
            let _ = app.run("library.source", json!({"kind": "album", "id": a.id.0}));
        }
        if a.folder {
            album_rows(app, ui, all, Some(a.id), depth + 1, guard + 1);
        }
    }
}

/// "Copy…" (choose the settings to copy) and "Paste" at the bottom of the Develop left panel.
fn footer(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, t.header);
    let half = (r.width() - 12.0 * 2.0 - 8.0) / 2.0;
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    if super::right::wide_button(&mut row, "developCopy", "Copy…", half).on_hover_text(crate::i18n::tr("Choose the settings to copy (⌘⇧C)")).clicked()
    {
        let _ = app.run("dialog.copySettings", json!({}));
    }
    let paste = super::right::wide_button(&mut row, "developPaste", "Paste", half).on_hover_text(crate::i18n::tr("Paste the copied settings (⌘⇧V)"));
    if paste.clicked() {
        match app.run("develop.paste", json!({})) {
            Ok(_) => app.toast(ui.ctx(), crate::i18n::tr("Settings pasted")),
            Err(e) => app.toast(ui.ctx(), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigator_zoom_presets_name_the_zoom() {
        assert_eq!(nav_zoom_index(Zoom::Fit), Some(0));
        assert_eq!(nav_zoom_index(Zoom::Fill), Some(1));
        assert_eq!(nav_zoom_index(Zoom::Percent(100.0)), Some(2));
        assert_eq!(nav_zoom_index(Zoom::Percent(200.0)), Some(3));
        assert_eq!(nav_zoom_index(Zoom::Percent(300.0)), None);
    }

    #[test]
    fn navigator_box_is_the_visible_part_of_the_photo() {
        let canvas = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        // fitted: nothing to box
        assert_eq!(visible_fraction(canvas, Rect::from_min_size(pos2(10.0, 0.0), vec2(80.0, 100.0))), None);
        // zoomed 2×, centred: the middle half
        let f = visible_fraction(canvas, Rect::from_min_size(pos2(-50.0, -50.0), vec2(200.0, 200.0))).unwrap();
        assert!((f.left() - 0.25).abs() < 1e-5 && (f.right() - 0.75).abs() < 1e-5);
        // degenerate image rects never divide by zero
        assert_eq!(visible_fraction(canvas, Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 0.0))), None);
        assert_eq!(visible_fraction(canvas, Rect::NOTHING), None);
    }
}
