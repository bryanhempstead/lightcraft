//! The top bar: sidebar toggle, back/forward, search, filter, and the module picker (Library |
//! Develop) with the import progress on the right.

use egui::{Align2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// What the search field says in AI mode: what's missing, the indexing, or what to type.
fn ai_placeholder(app: &LightcraftApp) -> String {
    use lightcraft_engine::vision::models::{self, Package};
    let v = &app.session.vision;
    let i = models::info(Package::Search);
    if !v.installed(Package::Search) {
        let dl = v.download_state(Package::Search);
        if dl.running {
            return crate::i18n::tr_format!(
                "downloading the AI search model {d} / {n} MB…",
                d = format!("{:.0}", dl.done as f64 / 1e6),
                n = format!("{:.0}", dl.total as f64 / 1e6)
            );
        }
        return crate::i18n::tr_format!(
            "AI search needs {m} ({mb} MB, {l}): Return downloads it",
            m = i.name,
            mb = format!("{:.0}", i.size() as f64 / 1e6),
            l = i.licence
        );
    }
    let p = v.progress(lightcraft_engine::vision::Kind::Search);
    if p.running {
        return crate::i18n::tr_format!("Describe a photo… (indexing {d} / {n})", d = p.done, n = p.total);
    }
    crate::i18n::tr("Describe a photo…").into()
}

/// The search model downloaded (the field said its size and licence) and the library indexed, in the background.
fn ai_search_prepare(app: &mut LightcraftApp, ctx: &egui::Context) {
    use lightcraft_engine::vision::models::Package;
    if !app.session.vision.installed(Package::Search) {
        return;
    }
    let p = app.session.vision.progress(lightcraft_engine::vision::Kind::Search);
    if !p.running
        && let Err(e) = app.run("search.index", json!({"scope": "all"}))
        && !e.contains("already running")
    {
        app.toast_error(ctx, e);
    }
}

fn ai_search(app: &mut LightcraftApp, ctx: &egui::Context, q: &str) {
    use lightcraft_engine::vision::models::Package;
    if !app.session.vision.installed(Package::Search) {
        if !app.session.vision.download_state(Package::Search).running
            && let Err(e) = app.run("vision.model.download", json!({"model": "search", "acknowledged": true}))
        {
            app.toast_error(ctx, e);
        }
        return;
    }
    ai_search_prepare(app, ctx);
    if q.is_empty() {
        let _ = app.run("library.filter", json!({"semantic": null}));
        return;
    }
    match app.run("search.ai", json!({"query": q})) {
        Ok(r) => app.toast(ctx, crate::i18n::tr_format!("{n} photos match “{q}”, best first", n = r["matches"].as_u64().unwrap_or(0), q = q)),
        Err(e) => app.toast_error(ctx, e),
    }
}

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
            let left_items = ui.horizontal_centered(|ui| {
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
                // notes about what isn't working, sent to Claude with fix. (LC-UI-FIXNOTES)
                ui.add_space(4.0);
                crate::fixnotes::top_button(app, ui);
                if !app.native_menu {
                    // no native menu bar (web, Windows, Linux): menus in the top bar
                    ui.add_space(10.0);
                    let room = search_left - ui.cursor().left() - 16.0;
                    crate::menubar::show_in_window(app, ui, room);
                }
            });
            // the free space between the left buttons and the search field: the identity plate's
            // spot (Lightroom Classic shows its plate at the top left)
            let slot =
                Rect::from_min_max(pos2(left_items.response.rect.right() + 16.0, full.top() + 5.0), pos2(search_left - 16.0, full.bottom() - 5.0));
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(crate::identity::BAR_SLOT), slot));
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
            // ai.: natural-language search (Lightroom's "search by what's in the photo") instead of text
            let ai_rect = Rect::from_center_size(pos2(sr.right() - 24.0, sr.center().y), vec2(34.0, 20.0));
            let ai_resp = ui.interact(ai_rect, egui::Id::new("search-ai"), Sense::click());
            register(ui.ctx(), "button:searchAi", ai_rect);
            ui.painter().rect(
                ai_rect,
                4.0,
                if app.ui.search_ai {
                    t.tools_accent.unwrap_or(t.accent)
                } else if ai_resp.hovered() {
                    t.hover
                } else {
                    t.button
                },
                Stroke::new(1.0, t.button_border),
                StrokeKind::Inside,
            );
            let ai_text = if app.ui.search_ai { crate::theme::text_on(t.tools_accent.unwrap_or(t.accent)) } else { t.text };
            ui.painter().text(ai_rect.center(), Align2::CENTER_CENTER, "ai.", t.semibold(11.0), ai_text);
            let ai_tip = crate::i18n::tr("AI search: describe what's in the photo (“dog on the beach”), Return to search");
            if ai_resp.on_hover_text(ai_tip).clicked() {
                app.ui.search_ai = !app.ui.search_ai;
                // one mode at a time: the text filter or the AI search
                let q = app.ui.search.clone();
                let _ = if app.ui.search_ai {
                    app.run("library.filter", json!({"text": ""}))
                } else {
                    app.run("library.filter", json!({"semantic": null, "text": q}))
                };
                if app.ui.search_ai {
                    ai_search_prepare(app, ui.ctx());
                }
            }
            let ai_hint = app.ui.search_ai.then(|| ai_placeholder(app));
            if empty && !focused {
                let hint = ai_hint.clone().unwrap_or_else(|| crate::i18n::tr("Search Photos").into());
                let g = child.painter().layout_no_wrap(hint, t.font(13.5), t.text_dim);
                let w = g.size().x + 24.0;
                let x0 = sr.center().x - w / 2.0;
                paint(child.painter(), Rect::from_min_size(pos2(x0, sr.center().y - 8.0), vec2(16.0, 16.0)), Icon::Search, t.text_dim);
                child.painter().galley(pos2(x0 + 24.0, sr.center().y - g.size().y / 2.0), g, t.text_dim);
            }
            let resp = child.add(
                egui::TextEdit::singleline(&mut app.ui.search)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_width(sr.width() - 60.0)
                    .font(t.font(13.5))
                    .text_color(t.text),
            );
            if app.ui.search_ai {
                let q = app.ui.search.trim().to_string();
                if resp.changed() && q.is_empty() {
                    let _ = app.run("library.filter", json!({"semantic": null}));
                }
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ai_search(app, ui.ctx(), &q);
                }
            } else if resp.changed() {
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
                let badge = t.badge_accent.unwrap_or(t.accent);
                ui.painter().circle_filled(c, 7.0, badge);
                ui.painter().text(c, Align2::CENTER_CENTER, active.to_string(), t.semibold(9.5), crate::theme::text_on(badge));
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
            progress_strip(app, ui, Rect::from_min_max(pos2(x - 260.0, full.center().y - 9.0), pos2(x, full.center().y + 9.0)));
            let _ = Align2::CENTER_CENTER;
        });
}

/// The module picker, as Lightroom Classic draws it: plain text links with a thin divider; the
/// module you're in bright, the other one dim (brighter on hover). Returns its left edge.
pub fn module_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, full: Rect) -> f32 {
    let t = Tokens::get(ui.ctx());
    let mut x = full.right() - 12.0;
    let map = app.ui.view == crate::state::ViewMode::Map;
    let galleries = app.ui.view == crate::state::ViewMode::Galleries;
    let library = app.ui.module == crate::state::Module::Library && !map && !galleries;
    let merges = library && app.merge_tab;
    // right to left: galleries. | map. | merge. | develop. | library.
    let items = [
        ("galleries", "Galleries", galleries, "view.galleries", "Client galleries, kept by Second Brain (⌘⌥4)"),
        ("map", "Map", map, "view.map", "Map (⌘⌥3)"),
        ("merge", "Merge", merges, "view.merges", "Every merge found — panoramas, HDR brackets, focus stacks — in one place (⌘⇧M merges the picked photos)"),
        ("develop", "Develop", app.ui.module == crate::state::Module::Develop, "view.develop", "Develop (D)"),
        ("library", "Library", library && !merges, "view.library", "Library (G)"),
    ];
    let last = items.len() - 1;
    for (i, (id, label, on, cmd, tip)) in items.into_iter().enumerate() {
        // Bryan's label style: lowercase with a period ("library.", "develop."), in every language
        let text = format!("{}.", crate::i18n::tr(label).to_lowercase());
        let font = if on { t.semibold(18.0) } else { t.font(18.0) };
        // PLACEHOLDER: the colour passed to `galley` below is the one used (bright / dim / hover)
        let g = ui.painter().layout_no_wrap(text, font, egui::Color32::PLACEHOLDER);
        let r = Rect::from_min_max(pos2(x - g.size().x - 8.0, full.top() + 4.0), pos2(x, full.bottom() - 4.0));
        let resp = ui.interact(r, egui::Id::new(("module", id)), Sense::click()).on_hover_text(crate::i18n::tr(tip));
        register(ui.ctx(), format!("module:{id}"), r);
        let c = if on {
            t.tab_accent.map_or(egui::Color32::from_gray(238), crate::theme::legible)
        } else if resp.hovered() {
            egui::Color32::from_gray(175)
        } else {
            // Classic's unselected modules: a dim grey, clearly behind the bright one
            egui::Color32::from_gray(112)
        };
        ui.painter().galley(pos2(r.left() + 4.0, r.center().y - g.size().y / 2.0), g, c);
        if resp.clicked()
            && let Err(e) = app.run(cmd, json!({}))
        {
            app.toast(ui.ctx(), e);
        }
        x = r.left();
        if i < last {
            // the divider between the links
            let d = Rect::from_center_size(pos2(x - 10.0, full.center().y), vec2(1.0, 18.0));
            ui.painter().rect_filled(d, 0.0, egui::Color32::from_gray(90));
            x -= 20.0;
        }
    }
    x
}

/// A small progress bar with its label (import / export running), ending at `r.right()`.
fn progress_strip(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    // an export: its count, the bar, then pause. / resume. and × (cancel) after it
    if let Some(task) = app.export.as_ref() {
        let (done, total, current) = task.counts();
        let paused = task.paused.load(std::sync::atomic::Ordering::Relaxed);
        let queued: usize = app.export_queue.len();
        let mut label = format!("Exporting {} of {}", (done + 1).min(total), total);
        if paused {
            label = format!("Paused at {} of {}", (done + 1).min(total), total);
        }
        if queued > 0 {
            label += &format!(" (+{queued})");
        }
        let cancel_r = Rect::from_center_size(pos2(r.right() - 8.0, r.center().y), vec2(16.0, 16.0));
        let pause_r = Rect::from_center_size(pos2(r.right() - 30.0, r.center().y), vec2(16.0, 16.0));
        let bar = Rect::from_min_max(pos2(r.right() - 132.0, r.center().y - 3.0), pos2(r.right() - 44.0, r.center().y + 3.0));
        ui.painter().rect_filled(bar, 3.0, t.inset);
        let frac = done as f32 / total.max(1) as f32;
        let filled = Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * frac.clamp(0.0, 1.0), bar.bottom()));
        ui.painter().rect_filled(filled, 3.0, if paused { t.caution } else { t.text_label });
        ui.painter().text(pos2(bar.left() - 8.0, r.center().y), Align2::RIGHT_CENTER, label, t.font(11.5), t.text_label);
        let pr = ui.interact(pause_r, egui::Id::new("export-pause"), Sense::click()).on_hover_text(crate::i18n::tr(if paused {
            "resume."
        } else {
            "pause."
        }));
        register(ui.ctx(), "button:exportPause", pause_r);
        let (a, b) = (pause_r.shrink(4.0).left_top(), pause_r.shrink(4.0).right_bottom());
        if paused {
            ui.painter().add(egui::Shape::convex_polygon(vec![a, pos2(b.x, pause_r.center().y), pos2(a.x, b.y)], t.text_label, egui::Stroke::NONE));
        } else {
            ui.painter().rect_filled(Rect::from_min_max(a, pos2(a.x + 3.0, b.y)), 1.0, t.text_label);
            ui.painter().rect_filled(Rect::from_min_max(pos2(b.x - 3.0, a.y), b), 1.0, t.text_label);
        }
        let cr = ui
            .interact(cancel_r, egui::Id::new("export-cancel"), Sense::click())
            .on_hover_text(crate::i18n::tr("cancel. the export (and the queued ones)"));
        register(ui.ctx(), "button:exportCancel", cancel_r);
        let c = cancel_r.shrink(4.0);
        let stroke = egui::Stroke::new(1.5, if cr.hovered() { t.text } else { t.text_label });
        ui.painter().line_segment([c.left_top(), c.right_bottom()], stroke);
        ui.painter().line_segment([c.right_top(), c.left_bottom()], stroke);
        register(ui.ctx(), "progress:top", r);
        if !current.is_empty() {
            ui.interact(bar, egui::Id::new("export-bar"), Sense::hover()).on_hover_text(current);
        }
        if pr.clicked() {
            crate::export_task::toggle_pause(app);
        }
        if cr.clicked() {
            crate::export_task::cancel_all(app);
        }
        return;
    }
    let (label, frac) = if let Some(task) = app.import.as_ref().filter(|t| !t.browse && !t.auto) {
        (crate::i18n::tr_format!("Importing {} of {}", task.done, task.total), task.done as f32 / task.total.max(1) as f32)
    } else if let Some(task) = app.scan.as_ref() {
        let s = task.status();
        let (done, total) = (s["done"].as_u64().unwrap_or(0), s["total"].as_u64().unwrap_or(0));
        (crate::i18n::tr("Reading photos…").to_string(), if total == 0 { 0.0 } else { done as f32 / total as f32 })
    } else if let Some(job) = app.session.cull_job.as_ref() {
        (crate::i18n::tr_format!("Culling {d} of {n}", d = job.done(), n = job.total), job.done() as f32 / job.total.max(1) as f32)
    } else if app.session.vision.running(lightcraft_engine::vision::Kind::Faces) {
        // Lightroom's activity area: face detection, with a click to stop it
        let p = app.session.vision.progress(lightcraft_engine::vision::Kind::Faces);
        let resp = ui.interact(r, egui::Id::new("progress-faces"), Sense::click());
        let tip = if p.auto {
            crate::i18n::tr("Finding faces in the background · click to stop (what's done is kept; People's find. starts it again)")
        } else {
            crate::i18n::tr("Finding faces · click to stop (what's done is kept)")
        };
        if resp.on_hover_text(tip).clicked() {
            let _ = app.run("vision.cancel", json!({"kind": "faces"}));
        }
        register(ui.ctx(), "progress:faces", r);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        (crate::i18n::tr_format!("Finding faces {d} of {n}", d = p.done, n = p.total), p.done as f32 / p.total.max(1) as f32)
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
