//! People (Lightroom Classic's People view): **Named People** — a card per person (their largest face, name,
//! photo count, how many look like them) — and **Unnamed People** — stacks of faces of the same person found
//! by the face finder, each with a name field (the suggestion, if any, with ✓ / ✗). A named card opens the
//! person's page: their confirmed faces and the **Similar** faces suggested as them, with ✓ / ✗ on each and for
//! the picked ones.
//!
//! Stacks and faces are picked like everything else: click, ⌘-click, ⇧-click, `all.` / `none.`; dragging picked
//! stacks or faces onto a named card names them all. With stacks picked, a row above them names them all at once,
//! confirms (`yes.`) or rejects (`no.`) their suggestions, or removes them (`rm.`: not faces). A stack's count opens
//! it: its faces one by one, to name, confirm or take out the odd one. The header finds faces in the view (`find.`),
//! shows the background run (`stop.`) and offers the models when they aren't installed (`get.`).
//!
//! Only the rows on screen ask for face renders (the engine caches them, memory and disk).

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_engine::people::{FaceInfo, People};
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

/// A card's face picture (points); the name and count sit below it.
const CARD: f32 = 132.0;
const LABEL_H: f32 = 52.0;
const GAP: f32 = 16.0;
const PAD: f32 = 20.0;
const HEADER_H: f32 = 48.0;
const SECTION_H: f32 = 40.0;

/// A picked face or stack: the photo and region index of the face (a stack by its cover face).
pub type Key = (u64, usize);

fn key(f: &FaceInfo) -> Key {
    (f.photo.0, f.index)
}

fn faces_json(faces: &[&FaceInfo]) -> serde_json::Value {
    json!(faces.iter().map(|f| json!({"photo": f.photo.0, "index": f.index})).collect::<Vec<_>>())
}

/// Click / ⌘-click / ⇧-click on item `i` of `keys`.
fn pick(app: &mut LightcraftApp, keys: &[Key], i: usize, m: egui::Modifiers) {
    let Some(k) = keys.get(i).copied() else { return };
    let sel = &mut app.ui.people_sel;
    let anchor = app.ui.people_anchor.and_then(|a| keys.iter().position(|x| *x == a));
    if m.shift
        && let Some(a) = anchor
    {
        for x in keys.iter().take(a.max(i) + 1).skip(a.min(i)) {
            if !sel.contains(x) {
                sel.push(*x);
            }
        }
    } else if m.command {
        if let Some(p) = sel.iter().position(|x| *x == k) {
            sel.remove(p);
        } else {
            sel.push(k);
        }
        app.ui.people_anchor = Some(k);
    } else {
        *sel = vec![k];
        app.ui.people_anchor = Some(k);
    }
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    app.session.vision_poll();
    let filter = app.session.view_filter();
    let people = app.session.people(&filter);
    if app.session.vision.busy() || app.session.people_cache.busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    // the person page, while that person still exists here
    if let Some(name) = app.ui.people_page.clone() {
        if let Some(p) = people.named.iter().find(|p| p.name.eq_ignore_ascii_case(&name)) {
            let p = p.clone();
            return person_page(app, ui, &p);
        }
        app.ui.people_page = None;
    }
    // an opened stack, while it is still there (found by any of its faces: regrouping can change its cover)
    if let Some(k) = app.ui.people_stack {
        if let Some(st) = people.unnamed.iter().find(|s| s.faces.iter().any(|f| key(f) == k)) {
            let st = st.clone();
            return stack_page(app, ui, &st);
        }
        app.ui.people_stack = None;
    }
    header(app, ui, &people, None);
    let chips = lightcraft_engine::filter_chips(&app.session.filter, &app.session.catalog);
    super::chips::show(app, ui, &chips);
    if people.named.is_empty() && people.unnamed.is_empty() {
        let (title, body) = if !chips.is_empty() {
            ("No people in these photos", "Remove a filter above, or choose Clear all")
        } else if app.session.vision.installed(lightcraft_engine::vision::models::Package::Faces) {
            ("No faces found yet", "find. looks for faces in the photos shown; named faces from Lightroom and XMP show up here too.")
        } else {
            ("No people yet", "Named faces from Lightroom and XMP show up here. get. downloads the face finder (39 MB) to find everyone else.")
        };
        // the sections are there, empty, as in Lightroom Classic
        section(ui, "Named People", 0, None, app);
        super::empty_message(ui, ui.available_rect_before_wrap(), title, body);
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let mut drop: Option<String> = None;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("people-scroll").show(ui, |ui| {
        let width = ui.available_width();
        let cols = (((width - PAD * 2.0 + GAP) / (CARD + GAP)).floor() as usize).max(1);
        let row_h = CARD + LABEL_H + GAP;
        // Named People
        let named_rows = people.named.len().div_ceil(cols);
        section(ui, "Named People", people.named.len(), None, app);
        if people.named.is_empty() {
            let t = Tokens::get(ui.ctx());
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.label(
                    egui::RichText::new(crate::i18n::tr("Nobody is named yet: type a name under a face below and press Return."))
                        .font(t.font(12.0))
                        .color(t.text_dim),
                );
            });
            ui.add_space(12.0);
        }
        let (area, _) = ui.allocate_exact_size(vec2(width, named_rows as f32 * row_h + 4.0), Sense::hover());
        for (i, person) in people.named.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let min = area.min + vec2(PAD + col as f32 * (CARD + GAP), row as f32 * row_h);
            let r = Rect::from_min_size(min, vec2(CARD, CARD + LABEL_H));
            if !ui.clip_rect().intersects(r) {
                continue;
            }
            if let Some(target) = named_card(app, ui, person, r, ppp) {
                drop = Some(target);
            }
        }
        ui.add_space(GAP);
        // Unnamed People
        let keys: Vec<Key> = people.unnamed.iter().map(|s| key(&s.cover)).collect();
        section(ui, "Unnamed People", people.unnamed.len(), Some(&keys), app);
        let picked: Vec<Vec<&FaceInfo>> =
            people.unnamed.iter().filter(|s| app.ui.people_sel.contains(&key(&s.cover))).map(|s| s.faces.iter().collect()).collect();
        if !picked.is_empty() {
            bulk_row(app, ui, &picked, "stacks");
        }
        if people.unindexed > 0 {
            let t = Tokens::get(ui.ctx());
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.label(
                    egui::RichText::new(crate::i18n::tr_format!("{n} faces not analysed yet: find. analyses them", n = people.unindexed))
                        .font(t.font(12.0))
                        .color(t.text_dim),
                );
            });
            ui.add_space(12.0);
        }
        let rows = people.unnamed.len().div_ceil(cols);
        let (area, _) = ui.allocate_exact_size(vec2(width, rows as f32 * row_h + PAD), Sense::hover());
        for (i, stack) in people.unnamed.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let r = Rect::from_min_size(area.min + vec2(PAD + col as f32 * (CARD + GAP), row as f32 * row_h), vec2(CARD, CARD + LABEL_H));
            if !ui.clip_rect().intersects(r) {
                continue;
            }
            stack_card(app, ui, stack, &keys, i, r, ppp);
        }
    });
    // picked stacks dropped on a named card: name them all
    if let Some(name) = drop {
        let picked: Vec<&FaceInfo> =
            people.unnamed.iter().filter(|s| app.ui.people_sel.contains(&key(&s.cover))).flat_map(|s| s.faces.iter()).collect();
        if !picked.is_empty() {
            let r = app.run("people.name", json!({"faces": faces_json(&picked), "name": name}));
            after(app, ui.ctx(), r);
        }
    }
}

/// The view's header: title, counts, and what the face finder is doing.
fn header(app: &mut LightcraftApp, ui: &mut egui::Ui, people: &People, page: Option<&str>) {
    let t = Tokens::get(ui.ctx());
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(PAD, 10.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    if page.is_some() && text_button(&mut child, "peopleBack", "back.", false).on_hover_text(crate::i18n::tr("Back to People")).clicked() {
        app.ui.people_page = None;
        app.ui.people_sel.clear();
    }
    let title = page.map(str::to_string).unwrap_or_else(|| crate::i18n::tr("People").to_string());
    child.label(egui::RichText::new(title).font(t.semibold(15.0)).color(t.text));
    if page.is_none() {
        child.label(
            egui::RichText::new(crate::i18n::tr_format!(
                "{n} named · {u} stacks · {f} faces",
                n = people.named.len(),
                u = people.unnamed.len(),
                f = people.faces
            ))
            .font(t.font(12.0))
            .color(t.text_dim),
        );
    }
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        finder(app, ui);
    });
}

/// `get.` (models missing), `find.` or the run's progress with `stop.`.
fn finder(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    use lightcraft_engine::vision::models::{self, Package};
    let t = Tokens::get(ui.ctx());
    let v = &app.session.vision;
    let dl = v.download_state(Package::Faces);
    if !v.installed(Package::Faces) {
        if dl.running {
            if text_button(ui, "peopleDlStop", "stop.", false).clicked() {
                app.session.vision.cancel_download(Package::Faces);
            }
            ui.label(
                egui::RichText::new(format!("{:.0} / {:.0} MB …", dl.done as f64 / 1e6, dl.total as f64 / 1e6)).font(t.font(12.0)).color(t.text_dim),
            );
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        let i = models::info(Package::Faces);
        let tip = crate::i18n::tr_format!(
            "Download the face finder: {m} — {mb} MB, {l}. Saved in {d}",
            m = i.models,
            mb = format!("{:.0}", i.size() as f64 / 1e6),
            l = i.licence,
            d = models::dir(&app.session.vision.root, Package::Faces).display().to_string()
        );
        if text_button(ui, "peopleGet", "get.", false).on_hover_text(tip).clicked() {
            let r = app.run("vision.model.download", json!({"model": "faces", "acknowledged": true}));
            after(app, ui.ctx(), r);
        }
        let msg = match &dl.error {
            Some(e) => format!("⚠ {e}"),
            None => crate::i18n::tr_format!("face finder not installed ({mb} MB, {l})", mb = format!("{:.0}", i.size() as f64 / 1e6), l = i.licence),
        };
        ui.label(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim));
        return;
    }
    let p = app.session.vision.progress(lightcraft_engine::vision::Kind::Faces);
    if p.running {
        if text_button(ui, "peopleStop", "stop.", false).on_hover_text(crate::i18n::tr("Stop finding faces (what's done is kept)")).clicked() {
            let _ = app.run("vision.cancel", json!({"kind": "faces"}));
        }
        let (w, h) = (120.0, 6.0);
        let (bar, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        let track = Rect::from_center_size(bar.center(), vec2(w, h));
        ui.painter().rect_filled(track, 3.0, t.field);
        let frac = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
        ui.painter().rect_filled(Rect::from_min_size(track.min, vec2(w * frac.clamp(0.0, 1.0), h)), 3.0, t.accent);
        ui.label(
            egui::RichText::new(crate::i18n::tr_format!("finding faces {d} / {n} · {r}/s", d = p.done, n = p.total, r = format!("{:.1}", p.rate)))
                .font(t.font(12.0))
                .color(t.text_dim),
        );
        return;
    }
    let tip = crate::i18n::tr("Find faces in the photos shown (those not analysed yet), in the background");
    if text_button(ui, "peopleFind", "find.", false).on_hover_text(tip).clicked() {
        let r = app.run("faces.find", json!({"scope": "view"}));
        after(app, ui.ctx(), r);
    }
    // automatic finding stopped with stop.: it waits until resumed
    if app.session.vision.auto_paused
        && app.session.face_prefs.detect != lightcraft_engine::vision::FaceDetect::Off
        && text_button(ui, "peopleResume", "resume.", false)
            .on_hover_text(crate::i18n::tr("Go on finding faces by themselves (Settings ▸ General ▸ Find faces)"))
            .clicked()
    {
        let r = app.run("faces.auto", json!({"paused": false}));
        after(app, ui.ctx(), r);
    }
    if let Some(e) = p.error.filter(|_| !p.cancelled) {
        ui.label(egui::RichText::new(format!("⚠ {e}")).font(t.font(12.0)).color(t.text_dim));
    } else if p.total > 0 {
        let msg = crate::i18n::tr_format!("{d} photos analysed, {f} faces", d = p.done, f = p.faces);
        ui.label(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim));
    }
}

fn after(app: &mut LightcraftApp, ctx: &egui::Context, r: Result<serde_json::Value, String>) {
    if let Err(e) = r {
        app.toast_error(ctx, e);
    }
}

/// A section title with its count, and `all.` / `none.` for a pickable list.
fn section(ui: &mut egui::Ui, title: &str, n: usize, keys: Option<&[Key]>, app: &mut LightcraftApp) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), SECTION_H), Sense::hover());
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(PAD, 6.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    child.label(egui::RichText::new(crate::i18n::tr(title)).font(t.semibold(13.0)).color(t.text_label));
    child.label(egui::RichText::new(n.to_string()).font(t.font(12.0)).color(t.text_dim));
    if let Some(keys) = keys {
        child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if text_button(ui, &format!("people:{title}:none"), "none.", false).clicked() {
                app.ui.people_sel.retain(|k| !keys.contains(k));
            }
            if text_button(ui, &format!("people:{title}:all"), "all.", false).clicked() {
                for k in keys {
                    if !app.ui.people_sel.contains(k) {
                        app.ui.people_sel.push(*k);
                    }
                }
            }
            let picked = keys.iter().filter(|k| app.ui.people_sel.contains(k)).count();
            if picked > 0 {
                ui.label(egui::RichText::new(crate::i18n::tr_format!("{n} picked", n = picked)).font(t.font(12.0)).color(t.text_dim));
            }
        });
    }
}

/// The face picture of a card.
fn face_tex(app: &mut LightcraftApp, ui: &egui::Ui, f: &FaceInfo, r: Rect, ppp: f32) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 3.0, t.canvas);
    if let Some(job) = app.session.face_job(f.photo, f.rect, (r.width() * ppp).ceil() as usize)
        && let Some(tex) = app.renderer.variant(job)
    {
        ui.painter().image(tex.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

fn short(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n - 1).collect::<String>()) } else { s.to_string() }
}

/// A named person's card; returns their name when picked stacks are dropped on it.
fn named_card(app: &mut LightcraftApp, ui: &mut egui::Ui, person: &lightcraft_engine::people::NamedPerson, r: Rect, ppp: f32) -> Option<String> {
    let t = Tokens::get(ui.ctx());
    let face = Rect::from_min_size(r.min, vec2(CARD, CARD));
    let resp = ui.interact(r, egui::Id::new(("person-card", &person.name)), Sense::click());
    register(ui.ctx(), format!("person:{}", person.name), r);
    face_tex(app, ui, &person.cover, face, ppp);
    let dragging = ui.ctx().dragged_id().is_some_and(|d| d == egui::Id::new("people-drag"));
    let hovered = resp.hovered() || (dragging && ui.rect_contains_pointer(r));
    if hovered {
        ui.painter().rect_stroke(
            face,
            3.0,
            Stroke::new(if dragging { 2.0 } else { 1.0 }, if dragging { t.accent } else { t.text_dim }),
            StrokeKind::Outside,
        );
    }
    if !person.similar.is_empty() {
        let b = Rect::from_min_size(face.right_top() + vec2(-34.0, 6.0), vec2(28.0, 18.0));
        ui.painter().rect_filled(b, 9.0, t.accent);
        ui.painter().text(b.center(), Align2::CENTER_CENTER, format!("+{}", person.similar.len()), t.semibold(10.5), t.canvas);
    }
    ui.painter().text(pos2(r.left() + 2.0, face.bottom() + 15.0), Align2::LEFT_CENTER, short(&person.name, 17), t.semibold(13.0), t.text);
    let photos = crate::i18n::tr_format!("{n} photo{}", if person.photos == 1 { "" } else { "s" }, n = person.photos);
    ui.painter().text(pos2(r.left() + 2.0, face.bottom() + 35.0), Align2::LEFT_CENTER, photos, t.font(12.0), t.text_dim);
    let resp =
        resp.on_hover_text(crate::i18n::tr_format!("{} — click: their faces and suggestions · ⌥-click: their photos in the grid", person.name));
    if resp.clicked() {
        if ui.input(|i| i.modifiers.alt) {
            let _ = app.run("library.filter", json!({"person": person.name}));
            let _ = app.run("view.photoGrid", json!({}));
        } else {
            app.ui.people_page = Some(person.name.clone());
            app.ui.people_sel.clear();
        }
    }
    (dragging && ui.rect_contains_pointer(r) && ui.input(|i| i.pointer.any_released())).then(|| person.name.clone())
}

/// An unnamed stack: its cover face with the count, the name field and the suggestion's ✓ / ✗.
fn stack_card(app: &mut LightcraftApp, ui: &mut egui::Ui, stack: &lightcraft_engine::people::Stack, keys: &[Key], i: usize, r: Rect, ppp: f32) {
    let t = Tokens::get(ui.ctx());
    let k = key(&stack.cover);
    let face = Rect::from_min_size(r.min, vec2(CARD, CARD));
    let resp = ui.interact(face, egui::Id::new(("stack-card", k)), Sense::click_and_drag());
    register(ui.ctx(), format!("stack:{}:{}", k.0, k.1), face);
    face_tex(app, ui, &stack.cover, face, ppp);
    let picked = app.ui.people_sel.contains(&k);
    if picked {
        ui.painter().rect_stroke(face, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(face, 3.0, Stroke::new(1.0, t.text_dim), StrokeKind::Outside);
    }
    let mut opened = false;
    if stack.faces.len() > 1 {
        // the count opens the stack (Lightroom: expand the stack)
        let b = Rect::from_min_size(face.right_top() + vec2(-34.0, 6.0), vec2(28.0, 18.0));
        let br = ui.interact(b, egui::Id::new(("stack-count", k)), Sense::click());
        register(ui.ctx(), format!("stackCount:{}:{}", k.0, k.1), b);
        ui.painter().rect_filled(b, 9.0, Color32::from_black_alpha(if br.hovered() { 240 } else { 190 }));
        ui.painter().text(b.center(), Align2::CENTER_CENTER, stack.faces.len().to_string(), t.semibold(10.5), Color32::WHITE);
        if br.on_hover_text(crate::i18n::tr("Open the stack: its faces one by one")).clicked() {
            app.ui.people_stack = Some(k);
            app.ui.people_sel.clear();
            opened = true;
        }
    }
    if resp.double_clicked() && !opened {
        app.ui.people_stack = Some(k);
        app.ui.people_sel.clear();
    } else if resp.clicked() && !opened {
        let m = ui.input(|i| i.modifiers);
        pick(app, keys, i, m);
    }
    if resp.drag_started() {
        if !picked {
            app.ui.people_sel = vec![k];
            app.ui.people_anchor = Some(k);
        }
        ui.ctx().set_dragged_id(egui::Id::new("people-drag"));
    }
    if ui.ctx().dragged_id().is_some_and(|d| d == egui::Id::new("people-drag"))
        && picked
        && let Some(pp) = ui.input(|i| i.pointer.hover_pos())
    {
        let n = app.ui.people_sel.len();
        let label = crate::i18n::tr_format!("{n} stack{}", if n == 1 { "" } else { "s" }, n = n);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("people-drag-layer")));
        let g = painter.layout_no_wrap(label, t.semibold(12.0), Color32::WHITE);
        let br = Rect::from_min_size(pp + vec2(12.0, 12.0), g.size() + vec2(16.0, 10.0));
        painter.rect_filled(br, 4.0, t.accent);
        painter.galley(br.min + vec2(8.0, 5.0), g, Color32::WHITE);
    }
    // the name field: the suggestion as a hint ("Ann?"), Enter names the stack (or the picked stacks)
    let field = Rect::from_min_size(pos2(r.left(), face.bottom() + 8.0), vec2(CARD, 26.0));
    let hint = stack.suggestion.as_ref().map(|s| format!("{s}?")).unwrap_or_else(|| "?".into());
    let text = app.ui.people_names.entry(k).or_default();
    let w = if stack.suggestion.is_some() { CARD - 60.0 } else { CARD };
    let edit = ui.put(
        Rect::from_min_size(field.min, vec2(w, field.height())),
        egui::TextEdit::singleline(text).hint_text(hint).font(t.font(12.5)).desired_width(w - 8.0),
    );
    register(ui.ctx(), format!("stackName:{}:{}", k.0, k.1), edit.rect);
    let typed = text.trim().to_string();
    let mut name_with: Option<String> = None;
    if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !typed.is_empty() {
        name_with = Some(typed);
    }
    if let Some(s) = &stack.suggestion {
        let ok = Rect::from_min_size(pos2(field.right() - 54.0, field.top()), vec2(25.0, 26.0));
        let no = Rect::from_min_size(pos2(field.right() - 25.0, field.top()), vec2(25.0, 26.0));
        if glyph_button(ui, ok, ("stack-ok", k), "✓", &crate::i18n::tr_format!("Yes, this is {s}", s = s)) {
            name_with = Some(s.clone());
        }
        if glyph_button(ui, no, ("stack-no", k), "✗", &crate::i18n::tr_format!("No, this isn't {s}", s = s)) {
            let faces: Vec<&FaceInfo> = stack.faces.iter().collect();
            let r = app.run("people.reject", json!({"faces": faces_json(&faces), "name": s}));
            after(app, ui.ctx(), r);
        }
    }
    if let Some(name) = name_with {
        // this stack, and the other picked stacks when this one is among them
        let mut faces: Vec<FaceInfo> = stack.faces.clone();
        if picked {
            let filter = app.session.view_filter();
            let people = app.session.people(&filter);
            for s in people.unnamed.iter().filter(|s| key(&s.cover) != k && app.ui.people_sel.contains(&key(&s.cover))) {
                faces.extend(s.faces.iter().cloned());
            }
            app.ui.people_sel.clear();
        }
        let refs: Vec<&FaceInfo> = faces.iter().collect();
        let r = app.run("people.name", json!({"faces": faces_json(&refs), "name": name}));
        app.ui.people_names.remove(&k);
        after(app, ui.ctx(), r);
    }
    let n = crate::i18n::tr_format!("{n} face{}", if stack.faces.len() == 1 { "" } else { "s" }, n = stack.faces.len());
    ui.painter().text(pos2(r.left() + 2.0, field.bottom() + 10.0), Align2::LEFT_CENTER, n, t.font(11.5), t.text_dim);
}

/// What to do with the picked stacks (or faces): a name for all of them (Return or `name.`), ✓ their suggestions
/// (`yes.`), ✗ them (`no.`), or remove them as not faces (`rm.`). `groups`: the faces of each picked stack.
fn bulk_row(app: &mut LightcraftApp, ui: &mut egui::Ui, groups: &[Vec<&FaceInfo>], what: &str) {
    let t = Tokens::get(ui.ctx());
    let faces: Vec<&FaceInfo> = groups.iter().flatten().copied().collect();
    let suggested: Vec<&FaceInfo> = faces.iter().copied().filter(|f| f.suggestion.is_some()).collect();
    let mut done = false;
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        ui.spacing_mut().item_spacing.x = 8.0;
        let edit = ui.add(
            egui::TextEdit::singleline(&mut app.ui.people_bulk_name)
                .hint_text(crate::i18n::tr_format!("name the {n} picked {what}", n = groups.len(), what = crate::i18n::tr(what)))
                .font(t.font(12.5))
                .desired_width(200.0)
                // the buttons' height: one control height per row
                .min_size(vec2(200.0, 24.0))
                .vertical_align(egui::Align::Center),
        );
        register(ui.ctx(), "field:peopleBulkName", edit.rect);
        let name = app.ui.people_bulk_name.trim().to_string();
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let named = text_button(ui, "peopleBulkName", "name.", false).on_hover_text(crate::i18n::tr("Give the picked faces this name")).clicked();
        if (enter || named) && !name.is_empty() {
            let r = app.run("people.name", json!({"faces": faces_json(&faces), "name": name}));
            app.ui.people_bulk_name.clear();
            after(app, ui.ctx(), r);
            done = true;
        }
        if !suggested.is_empty() {
            let n = suggested.len();
            if text_button(ui, "peopleBulkYes", "yes.", false)
                .on_hover_text(crate::i18n::tr_format!("Confirm the {n} suggested faces as who they're suggested as", n = n))
                .clicked()
            {
                let r = app.run("people.confirm", json!({"faces": faces_json(&suggested)}));
                after(app, ui.ctx(), r);
                done = true;
            }
            if text_button(ui, "peopleBulkNo", "no.", false)
                .on_hover_text(crate::i18n::tr_format!("These {n} faces aren't who they're suggested as", n = n))
                .clicked()
            {
                let r = app.run("people.reject", json!({"faces": faces_json(&suggested)}));
                after(app, ui.ctx(), r);
                done = true;
            }
        }
        if text_button(ui, "peopleBulkRemove", "remove.", false).on_hover_text(crate::i18n::tr("Not faces: remove these face boxes")).clicked() {
            let r = app.run("people.removeFaces", json!({"faces": faces_json(&faces)}));
            after(app, ui.ctx(), r);
            done = true;
        }
        let msg = crate::i18n::tr_format!("{f} faces · {s} suggested", f = faces.len(), s = suggested.len());
        ui.label(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim));
    });
    ui.add_space(12.0);
    if done {
        app.ui.people_sel.clear();
    }
}

/// An opened unnamed stack: its faces one by one, picked with click / ⌘ / ⇧ / `all.` / `none.`, and the bulk row for
/// the picked ones (name, ✓ / ✗ the suggestion, remove). Double-click opens a face's photo.
fn stack_page(app: &mut LightcraftApp, ui: &mut egui::Ui, stack: &lightcraft_engine::people::Stack) {
    let t = Tokens::get(ui.ctx());
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(PAD, 10.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    if text_button(&mut child, "stackBack", "back.", false).on_hover_text(crate::i18n::tr("Back to People")).clicked() {
        app.ui.people_stack = None;
        app.ui.people_sel.clear();
    }
    let title = match &stack.suggestion {
        Some(s) => crate::i18n::tr_format!("Unnamed · looks like {s}", s = s),
        None => crate::i18n::tr("Unnamed").to_string(),
    };
    child.label(egui::RichText::new(title).font(t.semibold(15.0)).color(t.text));
    child.label(
        egui::RichText::new(crate::i18n::tr_format!("{n} face{}", if stack.faces.len() == 1 { "" } else { "s" }, n = stack.faces.len()))
            .font(t.font(12.0))
            .color(t.text_dim),
    );
    let keys: Vec<Key> = stack.faces.iter().map(key).collect();
    let ppp = ui.ctx().pixels_per_point();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("stack-scroll").show(ui, |ui| {
        let width = ui.available_width();
        let cell = 112.0;
        let cols = (((width - PAD * 2.0 + GAP) / (cell + GAP)).floor() as usize).max(1);
        section(ui, "Faces", stack.faces.len(), Some(&keys), app);
        let picked: Vec<&FaceInfo> = stack.faces.iter().filter(|f| app.ui.people_sel.contains(&key(f))).collect();
        if !picked.is_empty() {
            let groups: Vec<Vec<&FaceInfo>> = picked.iter().map(|f| vec![*f]).collect();
            bulk_row(app, ui, &groups, "faces");
        }
        face_grid(app, ui, &stack.faces, &keys, cols, cell, ppp, None);
        ui.add_space(PAD);
    });
}

/// A small square glyph button (✓ / ✗); true when clicked.
fn glyph_button(ui: &mut egui::Ui, r: Rect, id: (&str, Key), glyph: &str, tip: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, egui::Id::new(id), Sense::click());
    register(ui.ctx(), format!("button:{}:{}:{}", id.0, id.1.0, id.1.1), r);
    let fill = if resp.hovered() { t.hover } else { t.button };
    ui.painter().rect(r, 4.0, fill, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, t.semibold(13.0), t.text);
    resp.on_hover_text(tip).clicked()
}

/// One person: their confirmed faces and the faces suggested as them.
fn person_page(app: &mut LightcraftApp, ui: &mut egui::Ui, person: &lightcraft_engine::people::NamedPerson) {
    let people = People::default();
    header(app, ui, &people, Some(&person.name));
    let t = Tokens::get(ui.ctx());
    // rename
    ui.horizontal(|ui| {
        ui.add_space(PAD);
        ui.spacing_mut().item_spacing.x = 8.0;
        let text = app.ui.people_rename.get_or_insert_with(|| person.name.clone());
        let edit = ui.add(egui::TextEdit::singleline(text).desired_width(220.0).font(t.font(13.0)));
        register(ui.ctx(), "field:personName", edit.rect);
        let to = text.trim().to_string();
        let commit =
            (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || text_button(ui, "personRename", "rename.", false).clicked();
        if commit && !to.is_empty() && to != person.name {
            let r = app.run("people.rename", json!({"from": person.name, "to": to}));
            app.ui.people_page = Some(to);
            app.ui.people_rename = None;
            after(app, ui.ctx(), r);
        }
        if text_button(ui, "personPhotos", "photos.", false).on_hover_text(crate::i18n::tr("Show their photos in the grid")).clicked() {
            let _ = app.run("library.filter", json!({"person": person.name}));
            let _ = app.run("view.photoGrid", json!({}));
        }
    });
    ui.add_space(12.0);
    let ppp = ui.ctx().pixels_per_point();
    let similar_keys: Vec<Key> = person.similar.iter().map(key).collect();
    let confirmed_keys: Vec<Key> = person.faces.iter().map(key).collect();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("person-scroll").show(ui, |ui| {
        let width = ui.available_width();
        let cell = 112.0;
        let cols = (((width - PAD * 2.0 + GAP) / (cell + GAP)).floor() as usize).max(1);
        // Similar first (what needs an answer), with the picked ones' ✓ / ✗
        section(ui, "Similar", person.similar.len(), Some(&similar_keys), app);
        let picked: Vec<&FaceInfo> = person.similar.iter().filter(|f| app.ui.people_sel.contains(&key(f))).collect();
        if !picked.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.spacing_mut().item_spacing.x = 8.0;
                if text_button(ui, "similarYes", "yes.", false).on_hover_text(crate::i18n::tr_format!("These are {n}", n = person.name)).clicked() {
                    let r = app.run("people.name", json!({"faces": faces_json(&picked), "name": person.name}));
                    app.ui.people_sel.clear();
                    after(app, ui.ctx(), r);
                }
                if text_button(ui, "similarNo", "no.", false).on_hover_text(crate::i18n::tr_format!("These aren't {n}", n = person.name)).clicked() {
                    let r = app.run("people.reject", json!({"faces": faces_json(&picked), "name": person.name}));
                    app.ui.people_sel.clear();
                    after(app, ui.ctx(), r);
                }
            });
            ui.add_space(12.0);
        }
        face_grid(app, ui, &person.similar, &similar_keys, cols, cell, ppp, Some(&person.name));
        ui.add_space(GAP);
        section(ui, "Confirmed", person.faces.len(), Some(&confirmed_keys), app);
        let picked: Vec<&FaceInfo> = person.faces.iter().filter(|f| app.ui.people_sel.contains(&key(f))).collect();
        if !picked.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(PAD);
                ui.spacing_mut().item_spacing.x = 8.0;
                if text_button(ui, "confirmedUnname", "unname.", false).on_hover_text(crate::i18n::tr("Take the name off these faces")).clicked() {
                    let r = app.run("people.unname", json!({"faces": faces_json(&picked)}));
                    app.ui.people_sel.clear();
                    after(app, ui.ctx(), r);
                }
                if text_button(ui, "confirmedRemove", "remove.", false)
                    .on_hover_text(crate::i18n::tr("Not a face: remove these face boxes"))
                    .clicked()
                {
                    let r = app.run("people.removeFaces", json!({"faces": faces_json(&picked)}));
                    app.ui.people_sel.clear();
                    after(app, ui.ctx(), r);
                }
            });
            ui.add_space(12.0);
        }
        face_grid(app, ui, &person.faces, &confirmed_keys, cols, cell, ppp, None);
        ui.add_space(PAD);
    });
}

/// A grid of faces; with `suggested`, each face has ✓ / ✗ for that name.
#[allow(clippy::too_many_arguments)]
fn face_grid(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    faces: &[FaceInfo],
    keys: &[Key],
    cols: usize,
    cell: f32,
    ppp: f32,
    suggested: Option<&str>,
) {
    let t = Tokens::get(ui.ctx());
    let extra = if suggested.is_some() { 34.0 } else { 0.0 };
    let row_h = cell + extra + GAP;
    let rows = faces.len().div_ceil(cols);
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), rows as f32 * row_h), Sense::hover());
    for (i, f) in faces.iter().enumerate() {
        let r = Rect::from_min_size(area.min + vec2(PAD + (i % cols) as f32 * (cell + GAP), (i / cols) as f32 * row_h), vec2(cell, cell));
        if !ui.clip_rect().intersects(r.expand(extra)) {
            continue;
        }
        let k = key(f);
        let resp = ui.interact(r, egui::Id::new(("face-cell", k)), Sense::click());
        register(ui.ctx(), format!("face:{}:{}", k.0, k.1), r);
        face_tex(app, ui, f, r, ppp);
        let picked = app.ui.people_sel.contains(&k);
        if picked {
            ui.painter().rect_stroke(r, 3.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Outside);
        } else if resp.hovered() {
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.text_dim), StrokeKind::Outside);
        }
        if resp.double_clicked() {
            let _ = app.run("library.select", json!({"ids": [f.photo.0], "active": f.photo.0}));
            let _ = app.run("view.detail", json!({}));
        } else if resp.clicked() {
            let m = ui.input(|i| i.modifiers);
            pick(app, keys, i, m);
        }
        let _ = resp.on_hover_text(match f.score {
            Some(s) => crate::i18n::tr_format!("photo {id} · match {p}% · double-click to open", id = f.photo.0, p = format!("{:.0}", s * 100.0)),
            None => crate::i18n::tr_format!("photo {id} · double-click to open", id = f.photo.0),
        });
        if let Some(name) = suggested {
            let y = r.bottom() + 8.0;
            let ok = Rect::from_min_size(pos2(r.left(), y), vec2((cell - 8.0) / 2.0, 26.0));
            let no = Rect::from_min_size(pos2(r.left() + (cell + 8.0) / 2.0, y), vec2((cell - 8.0) / 2.0, 26.0));
            if glyph_button(ui, ok, ("face-ok", k), "✓", &crate::i18n::tr_format!("Yes, this is {s}", s = name)) {
                let r = app.run("people.name", json!({"faces": [{"photo": f.photo.0, "index": f.index}], "name": name}));
                after(app, ui.ctx(), r);
            }
            if glyph_button(ui, no, ("face-no", k), "✗", &crate::i18n::tr_format!("No, this isn't {s}", s = name)) {
                let r = app.run("people.reject", json!({"faces": [{"photo": f.photo.0, "index": f.index}], "name": name}));
                after(app, ui.ctx(), r);
            }
        }
    }
}
