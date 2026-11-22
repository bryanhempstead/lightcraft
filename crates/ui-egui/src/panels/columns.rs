//! The Library Filter's metadata column browser (L07, Lightroom's Metadata filter): under the
//! filter bar, a row of columns (Date, Camera, Lens, Label by default); each lists the values the
//! photos have with counts, narrowed by the columns to its left. Click = just this value,
//! ⌘-click = add / remove one, ⇧-click = the run from the last click, all. = clear the column.
//! A column's header picks its field; + adds a column, − removes the last. The columns are a fixed
//! height and scroll inside.

use egui::{Rect, RichText, Sense, Stroke, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// The browser's height (the lists scroll inside it).
pub const HEIGHT: f32 = 168.0;
const HEADER: f32 = 26.0;
const ROW: f32 = 20.0;

/// The column data, cached per (catalog revision, filter, source, columns).
fn data(app: &mut LightcraftApp, ctx: &egui::Context) -> Value {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        app.session.catalog.revision,
        format!("{:?}|{:?}|{:?}", app.session.filter, app.session.source, app.session.library_folder),
        &app.ui.metadata_columns,
    )
        .hash(&mut h);
    let key = h.finish();
    let id = egui::Id::new("column-browser-data");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(u64, Value)>(id))
        && k == key
    {
        return v;
    }
    let v = app.session.execute("library.columnBrowser", &json!({"columns": app.ui.metadata_columns})).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(id, (key, v.clone())));
    v
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), Sense::hover());
    ui.painter().rect_filled(area, 0.0, t.chrome);
    ui.painter().line_segment([area.left_bottom(), area.right_bottom()], Stroke::new(1.0, t.divider));
    register(ui.ctx(), "columnBrowser", area);
    let d = data(app, ui.ctx());
    let cols = d["columns"].as_array().cloned().unwrap_or_default();
    let available: Vec<(String, String)> = d["available"]
        .as_array()
        .map(|a| a.iter().map(|c| (c["id"].as_str().unwrap_or("").to_string(), c["label"].as_str().unwrap_or("").to_string())).collect())
        .unwrap_or_default();
    // + / − sit at the right edge, 16 px in; the columns share the rest, 8 px apart
    let inner = area.shrink2(vec2(16.0, 8.0));
    let buttons_w = 64.0;
    let n = cols.len().max(1) as f32;
    let gap = 8.0;
    let col_w = ((inner.width() - buttons_w - gap * n) / n).max(80.0);
    for (i, c) in cols.iter().enumerate() {
        let x = inner.left() + i as f32 * (col_w + gap);
        let r = Rect::from_min_size(egui::pos2(x, inner.top()), vec2(col_w, inner.height()));
        column(app, ui, &t, i, c, r, &available);
    }
    // + / −
    let bx = inner.right() - buttons_w + 8.0;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(egui::pos2(bx, inner.top()), vec2(buttons_w - 8.0, HEADER))));
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let add = ui.add_enabled(app.ui.metadata_columns.len() < 8, egui::Button::new("+"));
        register(ui.ctx(), "button:columnAdd", add.rect);
        if add.clicked()
            && let Some((id, _)) = available.iter().find(|(id, _)| !app.ui.metadata_columns.contains(id))
        {
            app.ui.metadata_columns.push(id.clone());
        }
        let rm = ui.add_enabled(app.ui.metadata_columns.len() > 1, egui::Button::new("−"));
        register(ui.ctx(), "button:columnRemove", rm.rect);
        if rm.clicked()
            && let Some(last) = app.ui.metadata_columns.pop()
        {
            let _ = app.run("library.columnPick", json!({"column": last, "values": []}));
        }
    });
}

fn column(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, i: usize, c: &Value, r: Rect, available: &[(String, String)]) {
    let col = c["column"].as_str().unwrap_or("").to_string();
    let picked: Vec<String> = c["picked"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    ui.painter().rect_filled(r, 4.0, t.inset);
    ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, t.divider), egui::StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(4.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 0.0;
    // header: the field picker
    let label = c["label"].as_str().unwrap_or("").to_string();
    let hr = egui::ComboBox::from_id_salt(("column-field", i))
        .selected_text(RichText::new(&label).color(t.text_label))
        .width(r.width() - 8.0)
        .show_ui(&mut child, |ui| {
            for (id, l) in available {
                if ui.selectable_label(*id == col, l).clicked() && *id != col {
                    let _ = app.run("library.columnPick", json!({"column": col, "values": []}));
                    if let Some(slot) = app.ui.metadata_columns.get_mut(i) {
                        *slot = id.clone();
                    }
                }
            }
        });
    register(child.ctx(), format!("button:columnField-{i}"), hr.response.rect);
    child.add_space(4.0);
    let values: Vec<(String, u64)> = c["values"]
        .as_array()
        .map(|a| a.iter().map(|v| (v["value"].as_str().unwrap_or("").to_string(), v["count"].as_u64().unwrap_or(0))).collect())
        .unwrap_or_default();
    let anchor_id = egui::Id::new(("column-anchor", i));
    egui::ScrollArea::vertical().id_salt(("column-scroll", i)).auto_shrink([false, false]).show(&mut child, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // all. (n): no pick in this column
        let all = row(ui, t, &format!("all. ({})", c["total"].as_u64().unwrap_or(0)), picked.is_empty(), "");
        register(ui.ctx(), format!("button:column-{i}-all"), all.rect);
        if all.clicked() {
            let _ = app.run("library.columnPick", json!({"column": col, "values": []}));
        }
        for (k, (v, n)) in values.iter().enumerate() {
            let shown = if v.is_empty() { "unknown".to_string() } else { v.clone() };
            let on = picked.contains(v);
            let resp = row(ui, t, &shown, on, &n.to_string());
            register(ui.ctx(), format!("button:column-{i}-{k}"), resp.rect);
            if resp.clicked() {
                let mods = ui.input(|i| i.modifiers);
                let next: Vec<String> = if mods.shift {
                    let from = ui.data(|d| d.get_temp::<usize>(anchor_id)).unwrap_or(k);
                    let (a, b) = (from.min(k), from.max(k));
                    let mut run: Vec<String> = values.get(a..=b).map(|s| s.iter().map(|x| x.0.clone()).collect()).unwrap_or_default();
                    if mods.command {
                        run.extend(picked.iter().cloned());
                        run.sort();
                        run.dedup();
                    }
                    run
                } else if mods.command {
                    let mut p = picked.clone();
                    if on {
                        p.retain(|x| x != v);
                    } else {
                        p.push(v.clone());
                    }
                    p
                } else if on && picked.len() == 1 {
                    vec![]
                } else {
                    vec![v.clone()]
                };
                if !mods.shift {
                    ui.data_mut(|d| d.insert_temp(anchor_id, k));
                }
                let _ = app.run("library.columnPick", json!({"column": col, "values": next}));
            }
        }
    });
}

/// One list row: the value on the left, its count on the right; picked rows are highlighted.
fn row(ui: &mut egui::Ui, t: &Tokens, text: &str, on: bool, count: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let p = ui.painter();
    if on {
        p.rect_filled(r, 3.0, t.tool_active);
    } else if resp.hovered() {
        p.rect_filled(r, 3.0, t.hover);
    }
    let font = t.font(12.0);
    let count_w = if count.is_empty() { 0.0 } else { p.layout_no_wrap(count.to_string(), font.clone(), t.text_dim).size().x + 8.0 };
    let max_w = (r.width() - 16.0 - count_w).max(20.0);
    let galley = p.layout(text.to_string(), font.clone(), if on { t.text } else { t.text_label }, f32::INFINITY);
    let text = if galley.size().x > max_w {
        let mut s: String = text.chars().collect();
        while !s.is_empty() && p.layout_no_wrap(format!("{s}…"), font.clone(), t.text).size().x > max_w {
            s.pop();
        }
        format!("{s}…")
    } else {
        text.to_string()
    };
    p.text(egui::pos2(r.left() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), if on { t.text } else { t.text_label });
    if !count.is_empty() {
        p.text(egui::pos2(r.right() - 8.0, r.center().y), egui::Align2::RIGHT_CENTER, count, font, t.text_dim);
    }
    resp
}
