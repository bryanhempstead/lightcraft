//! The Library module's right panel (Lightroom Classic): Histogram, Quick Develop (saved preset,
//! white balance, Auto Tone and the ±⅓ / ±1 tone buttons on every selected photo), Keywording
//! (keyword tags, suggestions, keyword set), Keyword List and Metadata, with "Sync Metadata" and
//! "Sync Settings" at the bottom.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::{KeywordNode, PhotoId};
use lightcraft_develop::WbMode;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// The panel's sections in Classic's order: (id, title).
pub const SECTIONS: [(&str, &str); 5] = [
    ("histogram", "Histogram"),
    ("quickDevelop", "Quick Develop"),
    ("keywording", "Keywording"),
    ("keywordList", "Keyword List"),
    ("metadata", "Metadata"),
];

const FOOTER_H: f32 = 40.0;

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let reserve = if app.ui.left_panel { crate::state::LEFT_WIDTH.min } else { 0.0 };
    let width = app.ui.right_width;
    let resized = super::resizable_side(ui, false, "right_panel", frame, width, crate::state::RIGHT_WIDTH, reserve, |ui| {
        let full = ui.max_rect();
        let body = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - FOOTER_H));
        let mut top = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        crate::widgets::set_classic_rows(top.ctx(), true);
        egui::ScrollArea::vertical().id_salt("library-right-scroll").auto_shrink([false, false]).show(&mut top, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let active = app.session.active();
            for (id, title) in SECTIONS {
                if !header(app, ui, id, title) {
                    continue;
                }
                match (id, active) {
                    ("histogram", Some(p)) => super::edit::histogram(app, ui, p),
                    ("quickDevelop", Some(_)) => quick_develop(app, ui),
                    ("keywording", Some(p)) => super::right::without_headers(ui, |ui| super::right::keywords(app, ui, p)),
                    ("keywordList", _) => keyword_list(app, ui, active),
                    ("metadata", Some(p)) => super::right::without_headers(ui, |ui| super::right::info(app, ui, p)),
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
        app.ui.right_width = w;
    }
}

/// A section header; returns whether the section is open. ⌥-click (or Solo Mode) = only this one.
fn header(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let open = app.ui.library_sections.iter().any(|s| s == id);
    let (resp, _) = crate::widgets::classic_header(ui, &format!("lib.{id}"), title, open, true, None);
    if resp.clicked() {
        let solo = app.ui.single_panel || ui.input(|i| i.modifiers.alt);
        crate::state::UiState::toggle_in(&mut app.ui.library_sections, id, solo && !open);
    }
    resp.context_menu(|ui| {
        let mut solo = app.ui.single_panel;
        if ui.checkbox(&mut solo, crate::i18n::tr("Solo Mode")).changed() {
            app.ui.single_panel = solo;
        }
    });
    open
}

fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(text), t.font(12.0), t.text_dim);
}

/// The Quick Develop rows: (label, control, small step, big step). Classic's buttons are ⅓ and 1
/// stop for Exposure and 5 / 20 for the rest.
pub const QUICK_ROWS: [(&str, &str, f64, f64); 10] = [
    ("Temperature", "wb.temp", 100.0, 500.0),
    ("Tint", "wb.tint", 5.0, 20.0),
    ("Exposure", "light.exposure", 1.0 / 3.0, 1.0),
    ("Contrast", "light.contrast", 5.0, 20.0),
    ("Highlights", "light.highlights", 5.0, 20.0),
    ("Shadows", "light.shadows", 5.0, 20.0),
    ("Whites", "light.whites", 5.0, 20.0),
    ("Blacks", "light.blacks", 5.0, 20.0),
    ("Clarity", "effects.clarity", 5.0, 20.0),
    ("Vibrance", "color.vibrance", 5.0, 20.0),
];

/// Run `cmd` on every selected photo: develop commands that act on the active photo carry their
/// change to the rest through Auto Sync, switched on for this one command.
fn on_selection(app: &mut LightcraftApp, cmd: &str, params: Value) -> Result<Value, String> {
    let was = app.session.auto_sync;
    app.session.auto_sync = true;
    let r = app.run(cmd, params);
    app.session.auto_sync = was;
    r
}

fn quick_develop(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let n = app.session.selection.ids.len().max(1);
    let label_w = 84.0;
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 10, top: 2, bottom: 2 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (r, _) = ui.allocate_exact_size(vec2(label_w, 22.0), Sense::hover());
                ui.painter().text(r.right_center(), Align2::RIGHT_CENTER, crate::i18n::tr(label), t.font(12.0), t.text_label);
                ui.add_space(4.0);
                add(ui);
            });
        });
    };
    ui.add_space(6.0);
    // Saved Preset
    row(ui, "Saved Preset", &mut |ui| {
        let r = crate::widgets::dropdown(ui, "quickPreset", crate::i18n::tr("Choose…"), t.font(12.0), t.text);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(220.0);
            let mut groups: Vec<String> = app.session.presets.iter().map(|p| p.group.clone()).collect();
            groups.sort();
            groups.dedup();
            let mut pick = None;
            for g in groups {
                ui.menu_button(g.clone(), |ui| {
                    for p in app.session.presets.iter().filter(|p| p.group == g) {
                        if ui.button(crate::i18n::builtin_label(&p.name, p.builtin)).clicked() {
                            pick = Some((p.id.clone(), p.name.clone()));
                        }
                    }
                });
            }
            if let Some((id, name)) = pick {
                match app.run("preset.apply", json!({"id": id})) {
                    Ok(_) => app
                        .toast(ui.ctx(), crate::i18n::tr_format!("Preset: {name} · {n} photo{}", if n == 1 { "" } else { "s" }, name = name, n = n)),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
        });
    });
    // White Balance
    row(ui, "White Balance", &mut |ui| {
        let cur = app.session.active().and_then(|id| app.session.develop_of(id)).map(|d| d.wb.mode).unwrap_or_default();
        let r = crate::widgets::dropdown(ui, "quickWb", crate::i18n::tr(cur.label()), t.font(12.0), t.text);
        egui::Popup::menu(&r).show(|ui| {
            for m in WbMode::ALL {
                if m == WbMode::Custom {
                    continue;
                }
                if ui.selectable_label(cur == m, m.label()).clicked() {
                    let mode = serde_json::to_value(m).unwrap_or_default();
                    let _ = on_selection(app, "develop.wb", json!({"mode": mode}));
                }
            }
        });
    });
    // Tone Control: Auto Tone
    row(ui, "Tone Control", &mut |ui| {
        if crate::widgets::text_button(ui, "quickAutoTone", "Auto Tone", false).clicked() {
            let _ = on_selection(app, "develop.auto", json!({}));
            app.toast(ui.ctx(), crate::i18n::tr("Auto Tone applied"));
        }
    });
    for (label, ctl, small, big) in QUICK_ROWS {
        row(ui, label, &mut |ui| {
            for (key, glyph, d) in [("ll", "◀◀", -big), ("l", "◀", -small), ("r", "▶", small), ("rr", "▶▶", big)] {
                // Classic-size nudge buttons (Bryan, 2026-10-08: "make those arrow buttons smaller and fit better")
                let (r, resp) = ui.allocate_exact_size(vec2(20.0, 16.0), Sense::click());
                register(ui.ctx(), format!("button:quick-{ctl}-{key}"), r);
                let fill = if resp.hovered() { t.hover } else { t.button };
                ui.painter().rect(r, 3.0, fill, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, t.font(7.5), t.text_label);
                let tip =
                    crate::i18n::tr_format!("{label} {d:+} on every selected photo", d = (d * 100.0).round() / 100.0, label = crate::i18n::tr(label));
                if resp.on_hover_text(tip).clicked() {
                    let _ = app.run("develop.quickAdjust", json!({"control": ctl, "delta": d}));
                }
            }
        });
    }
    egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 10, top: 6, bottom: 8 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(label_w + 8.0);
            if crate::widgets::text_button(ui, "quickResetAll", "Reset All", false).clicked() {
                let _ = app.run("develop.reset", json!({"ids": app.session.selection.ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
            }
        });
    });
}

/// Keyword List: every keyword (nested), its photo count, and a checkbox showing whether the
/// active photo has it; the checkbox adds / removes it on the selection, the arrow shows the
/// photos with it.
fn keyword_list(app: &mut LightcraftApp, ui: &mut egui::Ui, active: Option<PhotoId>) {
    let tree = app.caches.keyword_tree(&app.session.catalog);
    let have: Vec<String> = active.and_then(|id| app.session.catalog.photo(id)).map(|p| p.meta.keywords.clone()).unwrap_or_default();
    let fid = egui::Id::new("keyword-list-filter");
    let mut filter: String = ui.data(|d| d.get_temp(fid)).unwrap_or_default();
    egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 20, top: 6, bottom: 4 }).show(ui, |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut filter).hint_text(crate::i18n::tr("Filter Keywords")).desired_width(f32::INFINITY));
        register(ui.ctx(), "field:keywordListFilter", r.rect);
    });
    ui.data_mut(|d| d.insert_temp(fid, filter.clone()));
    if tree.is_empty() {
        note(ui, "No keywords yet");
        return;
    }
    let needle = filter.trim().to_lowercase();
    keyword_rows(app, ui, &tree, &have, &needle, 0, active.is_some());
    ui.add_space(6.0);
}

/// `node` or something under it matches the filter.
pub fn keyword_matches(node: &KeywordNode, needle: &str) -> bool {
    needle.is_empty() || node.name.to_lowercase().contains(needle) || node.children.iter().any(|c| keyword_matches(c, needle))
}

fn keyword_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, nodes: &[KeywordNode], have: &[String], needle: &str, depth: usize, can_tag: bool) {
    // keyword trees come from user data: never deeper than this
    if depth > 24 {
        return;
    }
    let t = Tokens::get(ui.ctx());
    for n in nodes.iter().filter(|n| keyword_matches(n, needle)) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        register(ui.ctx(), format!("keywordList:{}", n.path), r);
        if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.7));
        }
        let x = r.left() + 12.0 + depth as f32 * 14.0;
        let on = have.contains(&n.path);
        let cb = Rect::from_center_size(pos2(x + 6.0, r.center().y), vec2(11.0, 11.0));
        ui.painter().rect(cb, 2.0, if on { t.text_label } else { t.inset }, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
        if on {
            ui.painter().text(cb.center(), Align2::CENTER_CENTER, "✓", t.semibold(9.0), t.chrome);
        }
        ui.painter().text(pos2(x + 18.0, r.center().y), Align2::LEFT_CENTER, &n.name, t.font(12.0), t.text_label);
        ui.painter().text(pos2(r.right() - 20.0, r.center().y), Align2::RIGHT_CENTER, n.count.to_string(), t.font(11.0), t.text_dim);
        if resp.clicked() {
            let in_box = resp.interact_pointer_pos().is_some_and(|p| p.x < cb.right() + 4.0);
            if in_box && can_tag {
                let key = if on { "removeKeywords" } else { "addKeywords" };
                let _ = app.run("photo.setMeta", json!({key: [n.path.clone()]}));
            } else {
                let _ = app.run("library.filter", json!({"keyword": n.path.clone()}));
            }
        }
        let path = n.path.clone();
        resp.on_hover_text(crate::i18n::tr_format!("{path}: click the box to tag the selection, the name to show its photos", path = path));
        if !n.children.is_empty() {
            keyword_rows(app, ui, &n.children, have, needle, depth + 1, can_tag);
        }
    }
}

/// The fields "Sync Metadata" copies from the active photo to the rest of the selection.
pub const SYNC_FIELDS: [&str; 10] =
    ["title", "caption", "copyright", "creator", "usageTerms", "copyrightUrl", "location", "city", "state", "country"];

/// `photo.setMeta` params that give the other selected photos the active photo's metadata
/// (non-empty text fields of [`SYNC_FIELDS`] and its keywords, added).
pub fn sync_metadata_params(m: &lightcraft_catalog::Meta, ids: &[u64]) -> Value {
    let mut p = json!({"ids": ids});
    for (k, v) in [
        ("title", &m.title),
        ("caption", &m.caption),
        ("copyright", &m.copyright),
        ("creator", &m.creator),
        ("usageTerms", &m.usage_terms),
        ("copyrightUrl", &m.copyright_url),
        ("location", &m.location),
        ("city", &m.city),
        ("state", &m.state),
        ("country", &m.country),
    ] {
        if !v.trim().is_empty() {
            p[k] = json!(v);
        }
    }
    if !m.keywords.is_empty() {
        p["addKeywords"] = json!(m.keywords);
    }
    p
}

/// Library ▸ Sync Metadata: the active photo's metadata onto the other selected photos.
pub fn sync_metadata(app: &mut LightcraftApp) -> Result<Value, String> {
    let active = app.session.active().ok_or("select the photo to copy from, then the photos to copy to")?;
    let others: Vec<u64> = app.session.selection.ids.iter().filter(|i| **i != active).map(|i| i.0).collect();
    if others.is_empty() {
        return Err("select the photos to copy the metadata to as well".into());
    }
    let meta = app.session.catalog.photo(active).map(|p| p.meta.clone()).ok_or("no photo")?;
    app.run("photo.setMeta", sync_metadata_params(&meta, &others))?;
    Ok(json!({"changed": others.len()}))
}

fn footer(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, t.header);
    let half = (r.width() - 12.0 * 2.0 - 8.0) / 2.0;
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    let tip = "Copy the active photo's metadata and keywords to the other selected photos";
    if super::right::wide_button(&mut row, "syncMetadata", "Sync Metadata", half).on_hover_text(crate::i18n::tr(tip)).clicked() {
        match app.run("library.syncMetadata", json!({})) {
            Ok(v) => {
                let n = v["changed"].as_u64().unwrap_or(0);
                app.toast(ui.ctx(), crate::i18n::tr_format!("Metadata synced to {n} photo{}", if n == 1 { "" } else { "s" }, n = n));
            }
            Err(e) => app.toast(ui.ctx(), e),
        }
    }
    let tip = "Copy the active photo's develop settings to the other selected photos";
    if super::right::wide_button(&mut row, "syncSettings", "Sync Settings", half).on_hover_text(crate::i18n::tr(tip)).clicked() {
        if app.session.selection.ids.len() > 1 {
            let _ = app.run("dialog.syncSettings", json!({}));
        } else {
            app.toast(ui.ctx(), crate::i18n::tr("Select the photos to sync as well"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_metadata_copies_filled_fields_and_adds_keywords() {
        let m = lightcraft_catalog::Meta {
            title: "Ceremony".into(),
            copyright: "© Bryan".into(),
            keywords: vec!["BH WEDDING".into()],
            ..Default::default()
        };
        let p = sync_metadata_params(&m, &[2, 3]);
        assert_eq!(p["ids"], json!([2, 3]));
        assert_eq!(p["title"], "Ceremony");
        assert_eq!(p["copyright"], "© Bryan");
        // empty fields never blank the other photos' own
        assert!(p.get("caption").is_none());
        assert_eq!(p["addKeywords"], json!(["BH WEDDING"]));
        assert!(p.get("keywords").is_none(), "keywords are added, not replaced");
    }

    #[test]
    fn keyword_filter_keeps_parents_of_matches() {
        let leaf = KeywordNode { name: "Ceremony".into(), path: "BH WEDDING|Ceremony".into(), count: 3, children: vec![] };
        let root = KeywordNode { name: "BH WEDDING".into(), path: "BH WEDDING".into(), count: 3, children: vec![leaf] };
        assert!(keyword_matches(&root, "cere"));
        assert!(keyword_matches(&root, ""));
        assert!(!keyword_matches(&root, "travel"));
    }
}
