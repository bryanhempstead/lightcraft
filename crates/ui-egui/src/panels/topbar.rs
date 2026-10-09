//! The top bar: sidebar toggle, back/forward, search, filter, and the module picker (Library |
//! Develop) with the import progress on the right.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let left = if app.integrated_titlebar { 78 } else { 10 };
    egui::Panel::top("top_bar")
        .exact_size(t.top_bar_h)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin { left, right: 12, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            let mut sw = 640.0f32.min(full.width() - 460.0).max(200.0);
            if !app.native_menu {
                // leave room for the in-window menus left of the (centred) search field
                let menus_right = full.left() + 140.0 + crate::menubar::bar_width(ui) + 24.0;
                sw = sw.min(2.0 * (full.center().x - menus_right)).max(200.0);
            }
            let search_left = full.center().x - sw / 2.0;
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if icon_button(ui, "sidebar", Icon::Sidebar, vec2(30.0, 30.0), app.ui.left_panel, true, "Show/hide My Photos panel").clicked() {
                    let _ = app.run("view.leftPanel", json!({}));
                }
                ui.add_space(8.0);
                if icon_button(ui, "back", Icon::Back, vec2(30.0, 30.0), false, true, "Back").clicked() {
                    let _ = app.run("view.back", json!({}));
                }
                icon_button(ui, "forward", Icon::Forward, vec2(30.0, 30.0), false, false, "Forward");
                ui.add_space(8.0);
                // back to the photo last worked on (its folder or album, the photo, the view)
                let r = crate::widgets::text_button(ui, "leftOff", "left off.", false);
                if r.clicked()
                    && let Err(e) = app.run("view.resumeLastLeftOff", json!({}))
                {
                    app.toast(ui.ctx(), e);
                }
                r.on_hover_text(crate::i18n::tr("Go to where I left off"));
                if !app.native_menu {
                    // no native menu bar (web, Windows, Linux): menus in the top bar
                    ui.add_space(10.0);
                    let room = search_left - ui.cursor().left() - 16.0;
                    crate::menubar::show_in_window(app, ui, room);
                }
            });
            // search field (centred on the window)
            let sr = Rect::from_center_size(pos2(full.center().x, full.center().y), vec2(sw, 28.0));
            let id = egui::Id::new("search-field");
            if std::mem::take(&mut app.ui.focus_search) {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let focused = ui.memory(|m| m.has_focus(id));
            ui.painter().rect(
                sr,
                4.0,
                if focused { t.canvas } else { t.field },
                Stroke::new(1.0, if focused { t.accent } else { t.field_border }),
                StrokeKind::Inside,
            );
            register(ui.ctx(), "field:search", sr);
            let mut child =
                ui.new_child(egui::UiBuilder::new().max_rect(sr.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let empty = app.ui.search.is_empty();
            if empty && !focused {
                let g = child.painter().layout_no_wrap(crate::i18n::tr("Search Photos").into(), t.font(13.5), t.text_dim);
                let w = g.size().x + 24.0;
                let x0 = sr.center().x - w / 2.0;
                paint(child.painter(), Rect::from_min_size(pos2(x0, sr.center().y - 8.0), vec2(16.0, 16.0)), Icon::Search, t.text_dim);
                child.painter().galley(pos2(x0 + 24.0, sr.center().y - g.size().y / 2.0), g, t.text_dim);
            }
            let resp = child.add(
                egui::TextEdit::singleline(&mut app.ui.search)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(sr.width() - 20.0)
                    .font(t.font(13.5))
                    .text_color(t.text),
            );
            if resp.changed() {
                let q = app.ui.search.clone();
                let _ = app.run("library.filter", json!({"text": q}));
            }
            // filter icon right of the search field
            let fr = Rect::from_center_size(pos2(sr.right() + 22.0, sr.center().y), vec2(28.0, 28.0));
            let fresp = ui.interact(fr, egui::Id::new("filter-btn"), Sense::click());
            register(ui.ctx(), "icon:filter", fr);
            let filtering = app.session.filter != Default::default() || app.ui.filter_bar;
            paint(
                ui.painter(),
                fr.shrink(6.0),
                Icon::Filter,
                if filtering {
                    t.accent
                } else if fresp.hovered() {
                    t.text
                } else {
                    t.icon
                },
            );
            // badge: how many filters are on, even with the filter bar closed
            let active = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog).len();
            if active > 0 {
                let c = fr.right_top() + vec2(-3.0, 8.0);
                ui.painter().circle_filled(c, 7.0, t.accent);
                ui.painter().text(c, Align2::CENTER_CENTER, active.to_string(), t.semibold(9.5), t.canvas);
            }
            let fresp = fresp.on_hover_text(if active > 0 {
                crate::i18n::tr_format!("Filter bar — {active} active filter{}", if active == 1 { "" } else { "s" }, active = active)
            } else {
                "Filter bar".into()
            });
            if fresp.clicked() {
                let _ = app.run("view.filterBar", json!({}));
            }
            // right: Lightroom Classic's module picker ("Library | Develop"), then the save state
            let mut x = module_picker(app, ui, full) - 18.0;
            // saving is failing: a warning until a save succeeds
            if let Some(tip) = app.session.unsaved().map(|(n, e)| {
                crate::i18n::tr_format!(
                    "{n} change{} saved in memory but not written to disk: {e}\nLightCraft retries automatically; quitting now would lose {}.",
                    if n == 1 { "" } else { "s" },
                    if n == 1 { "it" } else { "them" },
                    e = e,
                    n = n
                )
            }) {
                let r = Rect::from_center_size(pos2(x, full.center().y), vec2(28.0, 28.0));
                ui.interact(r, egui::Id::new(("top", "cloud")), Sense::hover()).on_hover_text(tip);
                register(ui.ctx(), "icon:cloud", r);
                register(ui.ctx(), "indicator:unsaved", r);
                paint(ui.painter(), r.shrink(5.0), Icon::Cloud, t.caution);
                let c = r.right_top() + vec2(-5.0, 6.0);
                ui.painter().circle_filled(c, 6.0, t.reject);
                ui.painter().text(c, Align2::CENTER_CENTER, "!", t.semibold(9.5), t.canvas);
                x -= 40.0;
            }
            // an import or export in progress: Classic shows it where its identity plate is
            progress_strip(app, ui, Rect::from_min_max(pos2(x - 200.0, full.center().y - 9.0), pos2(x, full.center().y + 9.0)));
            let _ = Align2::CENTER_CENTER;
        });
}

/// The module picker: Classic's identity-plate text links, the active module bright, the other
/// dim with a divider between. Returns its left edge.
pub fn module_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, full: Rect) -> f32 {
    let t = Tokens::get(ui.ctx());
    let font = t.font(17.0);
    let mut x = full.right() - 6.0;
    let items =
        [("develop", "Develop", crate::state::Module::Develop, "Develop (D)"), ("library", "Library", crate::state::Module::Library, "Library (G)")];
    for (i, (id, label, module, tip)) in items.into_iter().enumerate() {
        let on = app.ui.module == module;
        let g = ui.painter().layout_no_wrap(crate::i18n::tr(label).to_string(), font.clone(), t.text);
        let r = Rect::from_min_max(pos2(x - g.size().x - 8.0, full.top() + 4.0), pos2(x + 2.0, full.bottom() - 4.0));
        let resp = ui.interact(r, egui::Id::new(("module", id)), Sense::click()).on_hover_text(crate::i18n::tr(tip));
        register(ui.ctx(), format!("module:{id}"), r);
        let c = if on {
            t.text
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_dim
        };
        ui.painter().galley(pos2(r.left() + 4.0, r.center().y - g.size().y / 2.0), g, c);
        if resp.clicked() {
            let cmd = if module == crate::state::Module::Develop { "view.develop" } else { "view.library" };
            if let Err(e) = app.run(cmd, json!({})) {
                app.toast(ui.ctx(), e);
            }
        }
        x = r.left();
        if i == 0 {
            // the divider between the links
            let d = Rect::from_center_size(pos2(x - 8.0, full.center().y), vec2(1.0, 16.0));
            ui.painter().rect_filled(d, 0.0, t.text_disabled);
            x -= 16.0;
        }
    }
    x
}

/// A small progress bar with its label (import / export running), ending at `r.right()`.
fn progress_strip(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    let (label, frac) = if let Some(task) = app.import.as_ref().filter(|t| !t.browse && !t.auto) {
        (crate::i18n::tr_format!("Importing {} of {}", task.done, task.total), task.done as f32 / task.total.max(1) as f32)
    } else if let Some(task) = app.scan.as_ref() {
        let s = task.status();
        let (done, total) = (s["done"].as_u64().unwrap_or(0), s["total"].as_u64().unwrap_or(0));
        (crate::i18n::tr("Reading photos…").to_string(), if total == 0 { 0.0 } else { done as f32 / total as f32 })
    } else {
        return;
    };
    let bar = Rect::from_min_max(pos2(r.right() - 90.0, r.center().y - 3.0), pos2(r.right(), r.center().y + 3.0));
    ui.painter().rect_filled(bar, 3.0, t.inset);
    let filled = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * frac.clamp(0.0, 1.0), bar.bottom()));
    ui.painter().rect_filled(filled, 3.0, t.text_label);
    ui.painter().text(pos2(bar.left() - 8.0, r.center().y), Align2::RIGHT_CENTER, label, t.font(11.5), t.text_label);
    register(ui.ctx(), "progress:top", r);
}
