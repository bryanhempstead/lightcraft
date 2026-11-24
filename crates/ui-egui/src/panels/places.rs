//! LR-MAP-PLACES: the Library's Place & Tags section — look up a business, a place, a street
//! address or typed coordinates and put it on the selected photos or on the whole album / folder
//! shown (which then keeps it for the photos imported into it later), and the keywords suggested
//! for them (from the place, the album's name and what the photos show) as chips to add or drop.
//!
//! The search runs on a worker thread ([`lightcraft_engine::places::lookup`], the same as
//! `location.search`); placing and keywording go through commands (`location.apply`,
//! `location.setDefault`, `keywords.suggest`, `keywords.applySuggested`). Chips are picked like
//! grid cells: click = just this one, ⌘-click = add / remove one, ⇧-click = the run from the last
//! click, all. / none.

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_engine::places::{self, Found, Place, Scope};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

type Answer = Arc<Mutex<Option<Result<(Vec<Place>, Found), String>>>>;

/// What the section shows between frames.
#[derive(Default)]
pub struct PlacesRuntime {
    /// The search box.
    pub query: String,
    /// A search on its worker thread (its answer lands here).
    pending: Option<Answer>,
    pub results: Vec<Place>,
    pub found: Option<Found>,
    pub error: Option<String>,
    /// The result picked (apply. puts it on the photos).
    pub picked: Option<usize>,
    /// The scope is the selected photos even while an album or folder shows.
    pub use_selection: bool,
    /// An album / folder keeps the place for new imports (default on).
    pub keep_off: bool,
    /// The tag chips picked, and the last one clicked (⇧-click runs from it).
    pub picked_tags: Vec<String>,
    last_tag: Option<String>,
    /// `keywords.suggest`'s answer, by what it was asked for.
    suggestions: Option<(String, Value)>,
    /// Focus the search box next frame.
    focus: bool,
    /// Scroll the panel to the section next frame.
    reveal: bool,
}

impl PlacesRuntime {
    /// A search is on its way.
    pub fn searching(&self) -> bool {
        self.pending.is_some()
    }
}

/// Start a search on a worker thread (the app keeps drawing; the answer shows when it lands).
pub fn find(app: &mut LightcraftApp, q: &str) -> Result<(), String> {
    let q = q.trim().to_string();
    if q.is_empty() {
        return Err("Type a business, a place, an address or coordinates".into());
    }
    app.places.query = q.clone();
    app.places.error = None;
    let geo = app.session.places.geocoder.clone();
    let cache = places::cache_path(&app.session);
    let slot: Answer = Arc::new(Mutex::new(None));
    let out = slot.clone();
    let spawned = std::thread::Builder::new().name("place-search".into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| places::lookup(&*geo, cache.as_deref(), &q, 8)))
            .unwrap_or_else(|_| Err("the place search failed".into()));
        *out.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
    });
    match spawned {
        Ok(_) => {
            app.places.pending = Some(slot);
            Ok(())
        }
        Err(e) => Err(format!("can't start the place search: {e}")),
    }
}

/// Take a finished search's answer.
fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(slot) = app.places.pending.clone() else { return };
    let done = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    match done {
        None => ctx.request_repaint_after(std::time::Duration::from_millis(80)),
        Some(r) => {
            app.places.pending = None;
            match r {
                Ok((list, found)) => {
                    app.places.error = list.is_empty().then(|| format!("Nothing found for “{}”", app.places.query));
                    app.places.picked = (!list.is_empty()).then_some(0);
                    app.places.results = list;
                    app.places.found = Some(found);
                }
                Err(e) => {
                    app.places.results.clear();
                    app.places.picked = None;
                    app.places.error = Some(e);
                }
            }
        }
    }
}

/// The command params naming what the section acts on, its name, and whether it is an album or
/// folder (which can keep a place).
pub fn scope(app: &LightcraftApp) -> Option<(Value, String, bool)> {
    if !app.places.use_selection
        && let Some(sc) = Scope::of_source(&app.session)
    {
        let name = sc.name(&app.session.catalog);
        let p = match &sc {
            Scope::Album(id) => json!({"album": id.0}),
            Scope::Folder(f) => json!({"folder": f}),
            Scope::Photos(ids) => json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}),
        };
        return Some((p, name, true));
    }
    let ids = app.session.targets(&Value::Null);
    if ids.is_empty() {
        return None;
    }
    let n = ids.len();
    Some((json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}), format!("{n} selected"), false))
}

/// One line of text, cut with … to `width`.
fn one_line(ui: &egui::Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, color, width.max(10.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// The place the scope has now: the album's / folder's own, else what its photos share.
fn current_place(app: &mut LightcraftApp, params: &Value, album: bool) -> String {
    if album
        && let Ok(d) = app.session.execute("location.default", params)
        && let Ok(p) = serde_json::from_value::<Place>(d["place"].clone())
    {
        return format!("{} (this album's)", p.summary());
    }
    let ids: Vec<u64> = params["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let ids: Vec<lightcraft_catalog::PhotoId> = if album {
        match Scope::of_source(&app.session) {
            Some(sc) => sc.photos(&app.session.catalog),
            None => Vec::new(),
        }
    } else {
        ids.into_iter().map(lightcraft_catalog::PhotoId).collect()
    };
    let mut seen: Vec<String> = Vec::new();
    for id in ids.iter().take(2000) {
        let s = app.session.catalog.photo(*id).and_then(|p| Place::of_meta(&p.meta)).map(|p| p.summary()).unwrap_or_default();
        if !seen.contains(&s) {
            seen.push(s);
        }
        if seen.len() > 1 {
            return "mixed".into();
        }
    }
    match seen.first() {
        Some(s) if !s.is_empty() => s.clone(),
        _ => "none".into(),
    }
}

/// The Place & Tags section of the Library's right panel.
pub fn section(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    poll(app, ui.ctx());
    if std::mem::take(&mut app.places.reveal) {
        ui.scroll_to_cursor_animation(Some(egui::Align::Min), egui::style::ScrollAnimation::none());
    }
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 14, top: 10, bottom: 12 }).show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 12.0);
        let Some((params, name, album)) = scope(app) else {
            ui.label(egui::RichText::new(crate::i18n::tr("Select photos, or show an album or folder")).font(t.font(12.0)).color(t.text_dim));
            return;
        };
        // ---- for: album. / sel.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let has_album = Scope::of_source(&app.session).is_some();
            if has_album {
                if text_button(ui, "placeScopeAlbum", "album.", !app.places.use_selection)
                    .on_hover_text(crate::i18n::tr("The whole album or folder shown"))
                    .clicked()
                {
                    app.places.use_selection = false;
                    app.places.picked_tags.clear();
                }
                if text_button(ui, "placeScopeSel", "selected.", app.places.use_selection)
                    .on_hover_text(crate::i18n::tr("Just the selected photos"))
                    .clicked()
                {
                    app.places.use_selection = true;
                    app.places.picked_tags.clear();
                }
            }
            let w = ui.available_width();
            let g = one_line(ui, &name, t.semibold(12.0), t.text_label, w);
            let (r, _) = ui.allocate_exact_size(vec2(g.size().x.min(w), 24.0), Sense::hover());
            ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text_label);
            register(ui.ctx(), "place:scope", r);
        });
        // ---- the place it has now
        let now = current_place(app, &params, album);
        ui.horizontal(|ui| {
            let (lr, _) = ui.allocate_exact_size(vec2(44.0, 18.0), Sense::hover());
            ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, crate::i18n::tr("place:"), t.font(12.0), t.text_dim);
            let w = ui.available_width();
            let g = one_line(ui, &now, t.font(12.0), t.text, w);
            let (r, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text);
            register(ui.ctx(), "place:now", r);
            resp.on_hover_text(&now);
        });
        // ---- the search box and find.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let w = (ui.available_width() - 60.0).max(80.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.places.query)
                    .hint_text(crate::i18n::tr("business, place, address or 47.6, -122.3"))
                    .desired_width(w)
                    // (as tall as find. beside it: one control height per row)
                    .min_size(vec2(w, 24.0))
                    .vertical_align(egui::Align::Center),
            );
            register(ui.ctx(), "field:placeQuery", r.rect);
            if std::mem::take(&mut app.places.focus) {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let label = if app.places.searching() { "…" } else { "find." };
            let b = text_button(ui, "placeFind", label, false)
                .on_hover_text(crate::i18n::tr("Look the place up (OpenStreetMap; coordinates are read here)"));
            if (b.clicked() || enter) && !app.places.searching() {
                let q = app.places.query.clone();
                if let Err(e) = find(app, &q) {
                    app.places.error = Some(e);
                }
            }
        });
        if let Some(e) = app.places.error.clone() {
            ui.add(egui::Label::new(egui::RichText::new(e).font(t.font(11.5)).color(t.caution)).wrap());
        }
        // ---- the candidates: click picks one, double-click puts it on the photos
        let mut apply_now = false;
        for (i, p) in app.places.results.clone().iter().enumerate() {
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(w, 40.0), Sense::click());
            register(ui.ctx(), format!("placeResult:{i}"), r);
            let picked = app.places.picked == Some(i);
            let fill = if picked {
                t.pressed
            } else if resp.hovered() {
                t.hover
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect(r, CornerRadius::same(4), fill, Stroke::new(1.0, if picked { t.accent } else { t.divider }), StrokeKind::Inside);
            let g1 = one_line(ui, &p.name, t.semibold(12.0), t.text, w - 20.0);
            let sub = if p.address.is_empty() { p.summary() } else { p.address.clone() };
            let g2 = one_line(ui, &sub, t.font(11.0), t.text_dim, w - 20.0);
            ui.painter().galley(pos2(r.left() + 10.0, r.top() + 4.0), g1, t.text);
            ui.painter().galley(pos2(r.left() + 10.0, r.bottom() - 4.0 - g2.size().y), g2, t.text_dim);
            if resp.clicked() {
                app.places.picked = Some(i);
            }
            if resp.double_clicked() {
                app.places.picked = Some(i);
                apply_now = true;
            }
            resp.on_hover_text(format!("{}\n{:.5}, {:.5}", sub, p.lat, p.lon));
        }
        if matches!(app.places.found, Some(Found::Geocoder | Found::Cache)) && !app.places.results.is_empty() {
            ui.label(egui::RichText::new(places::ATTRIBUTION).font(t.font(10.5)).color(t.text_dim));
        }
        // ---- apply. (and whether the album keeps it)
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let can = app.places.picked.is_some_and(|i| i < app.places.results.len());
            if text_button(ui, "placeApply", "apply.", false)
                .on_hover_text(crate::i18n::tr("Put the picked place on these photos (GPS, location, city, state, country and its keywords)"))
                .clicked()
            {
                if can {
                    apply_now = true;
                } else {
                    app.toast(ui.ctx(), "Find a place first, then pick it in the list");
                }
            }
            if album {
                // keep. (on): the album / folder keeps the place for photos imported later
                if text_button(ui, "placeKeep", "keep.", !app.places.keep_off)
                    .on_hover_text(crate::i18n::tr("On: the album or folder keeps the place, and photos imported into it later get it"))
                    .clicked()
                {
                    app.places.keep_off = !app.places.keep_off;
                }
                let has = app.session.execute("location.default", &params).is_ok_and(|d| !d["place"].is_null());
                if has
                    && text_button(ui, "placeClear", "clear.", false)
                        .on_hover_text(crate::i18n::tr("Forget this album's place (its photos keep theirs)"))
                        .clicked()
                {
                    let mut p = params.clone();
                    p["place"] = Value::Null;
                    if let Err(e) = app.run("location.setDefault", p) {
                        app.toast(ui.ctx(), e);
                    }
                }
            }
        });
        if apply_now && let Some(place) = app.places.picked.and_then(|i| app.places.results.get(i)).cloned() {
            let mut p = params.clone();
            p["place"] = serde_json::to_value(&place).unwrap_or_default();
            p["setDefault"] = json!(album && !app.places.keep_off);
            match app.run("location.apply", p) {
                Ok(r) => {
                    let n = r["photos"].as_u64().unwrap_or(0);
                    app.toast(ui.ctx(), format!("Placed {n} photo{} at {}", if n == 1 { "" } else { "s" }, place.name));
                }
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        ui.add_space(2.0);
        let (dr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().hline(dr.x_range(), dr.center().y, Stroke::new(1.0, t.divider));
        tags(app, ui, &params, &t);
    });
}

/// `keywords.suggest` for the scope, asked again only when the library, the scope or the AI
/// search index changed.
fn suggestions(app: &mut LightcraftApp, params: &Value) -> Value {
    let key = format!("{}|{}|{}", app.session.catalog.revision, params, app.session.vision.generation);
    if let Some((k, v)) = &app.places.suggestions
        && *k == key
    {
        return v.clone();
    }
    let v = app.session.execute("keywords.suggest", params).unwrap_or_default();
    app.places.suggestions = Some((key, v.clone()));
    v
}

/// The suggested keywords as chips: `+ name` not on the photos yet, filled when every photo has
/// it; picked ones get the accent edge. add. / drop. act on the picked ones.
fn tags(app: &mut LightcraftApp, ui: &mut egui::Ui, params: &Value, t: &Tokens) {
    let sug = suggestions(app, params);
    let n = sug["photos"].as_u64().unwrap_or(0);
    let list: Vec<(String, String, f64, u64)> = sug["suggestions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| Some((x["keyword"].as_str()?.to_string(), x["from"].as_str()?.to_string(), x["score"].as_f64()?, x["have"].as_u64()?)))
        .collect();
    let looks: Vec<String> = sug["looksLike"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect();
    ui.horizontal(|ui| {
        let (lr, _) = ui.allocate_exact_size(vec2(44.0, 18.0), Sense::hover());
        ui.painter().text(lr.left_center(), Align2::LEFT_CENTER, crate::i18n::tr("tags:"), t.font(12.0), t.text_dim);
        let text = if looks.is_empty() { "suggested for these photos".to_string() } else { format!("looks like {}", looks.join(", ")) };
        let w = ui.available_width();
        let g = one_line(ui, &text, t.font(12.0), t.text_label, w);
        let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
        ui.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text_label);
        register(ui.ctx(), "place:looksLike", r);
    });
    app.places.picked_tags.retain(|k| list.iter().any(|x| x.0 == *k));
    if list.is_empty() {
        ui.label(
            egui::RichText::new(crate::i18n::tr("No suggestions: set a place, or name the album for what it is"))
                .font(t.font(11.5))
                .color(t.text_dim),
        );
    } else {
        chips(app, ui, &list, n, t);
    }
    let content = sug["content"].as_str().unwrap_or("ok");
    if content != "ok" && content != "not asked" {
        ui.add(egui::Label::new(egui::RichText::new(format!("what's in the photos: {content}")).font(t.font(11.0)).color(t.text_dim)).wrap());
    }
    // ---- all. none. add. drop.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if text_button(ui, "tagAll", "all.", false).on_hover_text(crate::i18n::tr("Pick every suggestion")).clicked() {
            app.places.picked_tags = list.iter().map(|x| x.0.clone()).collect();
        }
        if text_button(ui, "tagNone", "none.", false).on_hover_text(crate::i18n::tr("Pick none")).clicked() {
            app.places.picked_tags.clear();
        }
        let picked = app.places.picked_tags.clone();
        if text_button(ui, "tagAdd", "add.", false)
            .on_hover_text(crate::i18n::tr("Add the picked suggestions to the photos (none picked: every suggestion)"))
            .clicked()
        {
            let mut p = params.clone();
            if !picked.is_empty() {
                p["keywords"] = json!(picked);
            }
            match app.run("keywords.applySuggested", p) {
                Ok(_) => app.places.picked_tags.clear(),
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        if text_button(ui, "tagDrop", "drop.", false)
            .on_hover_text(crate::i18n::tr("Take the picked ones off the photos and don't suggest them here again"))
            .clicked()
        {
            if picked.is_empty() {
                app.toast(ui.ctx(), "Pick the tags to drop first (click a chip)");
            } else {
                let (on, off): (Vec<String>, Vec<String>) = picked.into_iter().partition(|k| list.iter().any(|x| x.0 == *k && x.3 > 0));
                let mut p = params.clone();
                p["remove"] = json!(on);
                p["drop"] = json!(off);
                match app.run("keywords.applySuggested", p) {
                    Ok(_) => app.places.picked_tags.clear(),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
        }
    });
}

const CHIP_H: f32 = 24.0;
const CHIP_GAP_X: f32 = 8.0;
const CHIP_GAP_Y: f32 = 10.0;

fn chips(app: &mut LightcraftApp, ui: &mut egui::Ui, list: &[(String, String, f64, u64)], n: u64, t: &Tokens) {
    let width = ui.available_width();
    let font = t.font(12.0);
    // lay them out in rows first, to know the height
    let mut placed: Vec<(Rect, Arc<egui::Galley>)> = Vec::new();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for (k, _, _, have) in list {
        let all = n > 0 && *have >= n;
        let text = if all { k.clone() } else { format!("+ {k}") };
        let g = one_line(ui, &text, font.clone(), t.text, width - 20.0);
        let w = (g.size().x + 20.0).max(36.0).min(width);
        if x > 0.0 && x + w > width {
            x = 0.0;
            y += CHIP_H + CHIP_GAP_Y;
        }
        placed.push((Rect::from_min_size(pos2(x, y), vec2(w, CHIP_H)), g));
        x += w + CHIP_GAP_X;
    }
    let (area, _) = ui.allocate_exact_size(vec2(width, y + CHIP_H), Sense::hover());
    let mods = ui.input(|i| i.modifiers);
    for ((k, from, score, have), (r, g)) in list.iter().zip(placed) {
        let r = r.translate(area.min.to_vec2());
        let resp = ui.interact(r, egui::Id::new(("tagChip", k)), Sense::click());
        register(ui.ctx(), format!("tagChip:{k}"), r);
        let all = n > 0 && *have >= n;
        let picked = app.places.picked_tags.contains(k);
        let fill = if all {
            t.pressed
        } else if resp.hovered() {
            t.hover
        } else {
            t.button
        };
        let edge = if picked { Stroke::new(2.0, t.accent) } else { Stroke::new(1.0, t.button_border) };
        ui.painter().rect(r, CornerRadius::same(12), fill, edge, StrokeKind::Inside);
        let color = if all { t.text } else { t.text_label };
        ui.painter().galley(r.center() - g.size() / 2.0, g, color);
        let why = match from.as_str() {
            "place" => "from the place".to_string(),
            "name" => "from the album's name".to_string(),
            _ => format!("in {:.0}% of the photos", score * 100.0),
        };
        let resp = resp.on_hover_text(format!("{k} — {why}; on {have} of {n} photos"));
        if resp.double_clicked() {
            // double-click: straight on the photos
            let mut params = scope(app).map(|s| s.0).unwrap_or_default();
            params["keywords"] = json!([k]);
            if let Err(e) = app.run("keywords.applySuggested", params) {
                app.toast(ui.ctx(), e);
            }
        } else if resp.clicked() {
            pick(app, list, k, mods.command, mods.shift);
        }
    }
}

/// Click = just this one, ⌘ = add / remove it, ⇧ = the run from the last click.
pub fn pick(app: &mut LightcraftApp, list: &[(String, String, f64, u64)], k: &str, cmd: bool, shift: bool) {
    let order: Vec<&str> = list.iter().map(|x| x.0.as_str()).collect();
    let picked = &mut app.places.picked_tags;
    if shift && let Some(from) = app.places.last_tag.as_deref().and_then(|l| order.iter().position(|x| *x == l)) {
        let to = order.iter().position(|x| *x == k).unwrap_or(from);
        let (a, b) = (from.min(to), from.max(to));
        for x in order.get(a..=b).unwrap_or_default() {
            if !picked.iter().any(|p| p == x) {
                picked.push(x.to_string());
            }
        }
    } else if cmd {
        if let Some(i) = picked.iter().position(|p| p == k) {
            picked.remove(i);
        } else {
            picked.push(k.to_string());
        }
        app.places.last_tag = Some(k.to_string());
    } else {
        *picked = vec![k.to_string()];
        app.places.last_tag = Some(k.to_string());
    }
}

/// `view.places`: the Library with this section open and its search box focused; with `q`, that
/// search started (`places.find`).
pub fn open(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if !app.ui.library_sections.iter().any(|s| s == "places") {
        app.ui.library_sections.push("places".into());
    }
    app.ui.right_panel = true;
    app.places.focus = true;
    app.places.reveal = true;
    if let Some(sel) = p.get("selection").and_then(Value::as_bool) {
        app.places.use_selection = sel;
    }
    if let Some(q) = p.get("q").and_then(Value::as_str) {
        find(app, q)?;
    }
    Ok(json!({"searching": app.places.searching()}))
}

/// The automatic parts, for Settings ▸ Import.
pub fn settings_rows(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let d = app.session.import_defaults.clone();
    ui.add_space(4.0);
    ui.label(egui::RichText::new(crate::i18n::tr("Places and keywords")).font(t.semibold(12.5)).color(t.text));
    let mut rows: [(&str, bool, &str); 3] = [
        ("autoPlace", d.auto_place, "Photos imported into an album or folder with a place get it"),
        ("placeKeywords", d.place_keywords, "… with the place's keywords (venue, city, state, country)"),
        ("autoKeywords", d.auto_keywords, "Add the keywords the album's or folder's name suggests (“C&J WEDDING” → wedding)"),
    ];
    for (key, value, label) in rows.iter_mut() {
        let (key, label): (&str, &str) = (*key, *label);
        let r = ui.checkbox(value, crate::i18n::tr(label));
        register(ui.ctx(), format!("check:settings.{key}"), r.rect);
        if r.changed() {
            let (k, v) = (key.to_string(), *value);
            let _ = app.run("library.preferences", json!({"import": {k: v}}));
        }
    }
}
