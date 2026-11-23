//! LC-GALLERIES: the galleries. module tab — Second Brain's client galleries (Pic-Time style,
//! self-hosted), in LightCraft. Modelled on Second Brain's Client tab (`renderer/client.js`):
//!
//! - **Left:** `new gallery from…` the selection (`sel.`) or what the grid shows (`view.`: a
//!   collection, a shoot, a camera-folder part), with a name, client and watermark; the **client
//!   book** (clients, their shoots) under it.
//! - **Centre:** the galleries as cards (cover, name, client, photo count, live / not live,
//!   views, client picks); a card opens the gallery: its photos with the client's hearts (♥), his
//!   highlights (★) and hidden photos marked, picked like everything else (click, ⌘-click,
//!   ⇧-click, `all.` / `none.`, ⌘A). Double-click (or `dev.`) opens a photo's source in Develop.
//! - **Right:** the open gallery's settings (title, client, email, password, expiry, downloads,
//!   print store), its link (`copy.` / `open.`), `pub.` / `unpub.`, re-export (`re.exp.`), client
//!   picks → flags (`flag.`, `star.`, the picks collection `show.`) and a slideshow (`slide.`).
//!
//! Second Brain owns the galleries; everything here goes through the `gallery.*` commands
//! (`lightcraft_engine::galleries`). Network and disk work runs on worker threads
//! ([`crate::tasks`]); the panel only draws what came back. Without Second Brain the galleries
//! show read-only with a notice and `start.`.

use std::collections::HashMap;

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::ViewMode;
use crate::theme::Tokens;
use crate::widgets::{register, text_button};

const PAD: f32 = 20.0;
const HEADER_H: f32 = 48.0;
const CARD_W: f32 = 220.0;
const CARD_IMG_H: f32 = 146.0;
const CARD_TEXT_H: f32 = 66.0;
const GAP: f32 = 16.0;
const TILE: f32 = 150.0;
const THUMB_EDGE: u32 = 360;
/// Thumbnail loads in flight at once.
const MAX_LOADS: usize = 6;
/// The task labels (`ui.inspect` → `tasks`).
pub const LISTING: &str = "galleries";
pub const MAKING: &str = "gallery export";
pub const THUMB: &str = "gallery thumb";
pub const PUBLISHING: &str = "gallery publish";

enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

/// The open gallery's settings as typed (saved with `save.`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Form {
    pub name: String,
    pub client: String,
    pub email: String,
    pub password: String,
    /// `YYYY-MM-DD`, empty = never.
    pub expires: String,
    pub downloads: bool,
    pub prints: bool,
}

/// The new-gallery fields in the left panel.
#[derive(Clone, Debug, Default)]
pub struct NewForm {
    pub name: String,
    pub client: String,
    pub watermark: String,
}

/// What the tab holds between frames (not saved).
#[derive(Default)]
pub struct GalleriesRuntime {
    pub list: Vec<Value>,
    pub online: bool,
    pub notice: Option<String>,
    pub loaded: bool,
    /// The open gallery (Second Brain id) and its photos (`gallery.get`).
    pub open: Option<String>,
    pub detail: Option<Value>,
    pub form: Form,
    pub new: NewForm,
    /// Picked photos of the open gallery (file names), and the last one clicked.
    pub picked: Vec<String>,
    pub anchor: Option<String>,
    pub link: Option<String>,
    pub clients: Option<Value>,
    /// A slideshow being made: (render id, last status).
    pub show: Option<(String, Value)>,
    thumbs: HashMap<String, Thumb>,
    /// When the offline tab last looked for Second Brain again (seconds, egui time).
    last_poll: f64,
}

impl GalleriesRuntime {
    fn gallery(&self) -> Option<&Value> {
        self.detail.as_ref().and_then(|d| d.get("gallery"))
    }
    fn photos(&self) -> Vec<Value> {
        self.detail.as_ref().and_then(|d| d.get("photos")).and_then(Value::as_array).cloned().unwrap_or_default()
    }
    fn read_only(&self) -> bool {
        !self.online
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// `YYYY-MM-DD` ↔ ms since 1970 (UTC midnight) for the expiry field.
pub fn date_to_ms(d: &str) -> Option<u64> {
    let d = d.trim();
    if d.is_empty() {
        return Some(0);
    }
    let mut it = d.split('-');
    let (y, m, day): (i64, i64, i64) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    if it.next().is_some() || !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // days from civil (Howard Hinnant)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days).ok().and_then(|d| d.checked_mul(86_400_000))
}

pub fn ms_to_date(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let z = (ms / 86_400_000) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn form_of(g: &Value) -> Form {
    Form {
        name: s(g, "name"),
        client: s(g, "clientName"),
        email: s(g, "clientEmail"),
        password: s(g, "password"),
        expires: ms_to_date(n(g, "expires")),
        downloads: g.get("allowDownload").and_then(Value::as_bool).unwrap_or(true),
        prints: b(g, "allowPrints"),
    }
}

// ---------- background work

/// Re-read the list (and the open gallery) on a worker thread.
pub fn refresh(app: &mut LightcraftApp) {
    if app.tasks.is_running(LISTING) {
        return;
    }
    let dir = app.session.second_brain_dir.clone();
    let open = app.galleries.open.clone();
    let _ = crate::tasks::spawn(
        app,
        LISTING,
        move || {
            let list = lightcraft_engine::galleries::fetch_list(dir.as_deref());
            let detail = open.as_ref().map(|id| lightcraft_engine::galleries::fetch_gallery(dir.as_deref(), id));
            let clients = if list.as_ref().is_ok_and(|l| l.online) {
                lightcraft_engine::galleries::connect(dir.as_deref()).ok().and_then(|a| a.get("/v1/clientbook").ok())
            } else {
                None
            };
            (list, open, detail, clients)
        },
        |app, _ctx, (list, open, detail, clients)| {
            let rt = &mut app.galleries;
            rt.loaded = true;
            match list {
                Ok(l) => {
                    rt.online = l.online;
                    rt.notice = l.notice;
                    rt.list = l.galleries.into_iter().map(|g| lightcraft_engine::galleries::annotate(&app.session.catalog, g)).collect();
                }
                Err(e) => {
                    rt.notice = Some(e);
                }
            }
            rt.clients = clients;
            if rt.open == open {
                match detail {
                    Some(Ok(mut d)) => {
                        if let Some(id) = &open {
                            lightcraft_engine::galleries::annotate_detail(&app.session.catalog, id, &mut d);
                        }
                        let g = d.get("gallery").cloned().unwrap_or(Value::Null);
                        let fresh = form_of(&g);
                        // keep what he is typing; take the server's values otherwise
                        let was_server = rt.detail.as_ref().and_then(|x| x.get("gallery")).map(form_of);
                        if was_server.as_ref().is_none_or(|w| *w == rt.form) {
                            rt.form = fresh;
                        }
                        let names: Vec<String> =
                            d.get("photos").and_then(Value::as_array).map(|a| a.iter().map(|p| s(p, "name")).collect()).unwrap_or_default();
                        rt.picked.retain(|p| names.contains(p));
                        rt.detail = Some(d);
                    }
                    Some(Err(e)) => {
                        rt.notice = Some(e);
                        rt.open = None;
                        rt.detail = None;
                    }
                    None => {}
                }
            }
        },
    );
}

fn open_gallery(app: &mut LightcraftApp, id: Option<String>) {
    let rt = &mut app.galleries;
    if rt.open != id {
        rt.picked.clear();
        rt.anchor = None;
        rt.link = None;
        rt.show = None;
        rt.detail = None;
        rt.form = Form::default();
    }
    rt.open = id;
    refresh(app);
}

/// A thumbnail: Second Brain's cached one when it runs, else the file itself, decoded off the UI
/// thread.
fn load_thumb(app: &mut LightcraftApp, gallery: &str, folder: &str, name: &str) {
    let key = format!("{gallery}/{name}");
    if app.galleries.thumbs.contains_key(&key) {
        return;
    }
    let loading = app.galleries.thumbs.values().filter(|t| matches!(t, Thumb::Loading)).count();
    if loading >= MAX_LOADS {
        return;
    }
    app.galleries.thumbs.insert(key.clone(), Thumb::Loading);
    let failed_key = key.clone();
    let (dir, online) = (app.session.second_brain_dir.clone(), app.galleries.online);
    let (gid, folder, name) = (gallery.to_string(), folder.to_string(), name.to_string());
    let r = crate::tasks::spawn(
        app,
        THUMB,
        move || {
            let from_api = if online {
                lightcraft_engine::galleries::Api::discover(&lightcraft_engine::galleries::sb_dir(dir.as_deref()).unwrap_or_default())
                    .ok()
                    .and_then(|a| a.thumb(&gid, &name, THUMB_EDGE).ok())
            } else {
                None
            };
            let bytes = from_api.or_else(|| {
                let p = std::path::Path::new(&folder).join(&name);
                let f = std::fs::File::open(p).ok()?;
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut std::io::Read::take(f, 200 << 20), &mut buf).ok()?;
                Some(buf)
            })?;
            lightcraft_codecs::decode_thumbnail(&bytes, THUMB_EDGE).ok().map(|t| t.image)
        },
        move |app, ctx, img| {
            let t = match img {
                Some(img) => {
                    let color = crate::display_p3::screen_image(app.renderer.display_p3, [img.width, img.height], &img.as_bytes());
                    Thumb::Ready(ctx.load_texture(format!("gallery-{key}"), color, egui::TextureOptions::LINEAR))
                }
                None => Thumb::Failed,
            };
            app.galleries.thumbs.insert(key, t);
        },
    );
    if r.is_err() {
        app.galleries.thumbs.insert(failed_key, Thumb::Failed);
    }
}

/// `galleries.newFrom`: export the photos with their edits into a gallery folder and register it,
/// on a worker (prepare here, run there, finish here).
fn start_create(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.tasks.is_running(MAKING) {
        return Err("a gallery is being exported already".into());
    }
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        let nf = &app.galleries.new;
        if !o.contains_key("name") && !nf.name.trim().is_empty() {
            o.insert("name".into(), json!(nf.name.trim()));
        }
        if !o.contains_key("clientName") && !nf.client.trim().is_empty() {
            o.insert("clientName".into(), json!(nf.client.trim()));
        }
        if !o.contains_key("watermark") && !nf.watermark.trim().is_empty() {
            o.insert("watermark".into(), json!(nf.watermark.trim()));
        }
    }
    // no thread-safe writer (the browser build): here and now
    let Some(write) = app.services.write_shared.clone() else {
        let r = app.run("gallery.create", p)?;
        after_create(app, Ok(r.clone()));
        return Ok(r);
    };
    let job = lightcraft_engine::galleries::prepare_create(&mut app.session, &p)?;
    let n = job.photos();
    crate::tasks::spawn(
        app,
        MAKING,
        move || job.run(&mut |path, bytes| write(path, bytes), &mut |_, _| true),
        |app, _ctx, out| {
            let r = lightcraft_engine::galleries::finish_create(&mut app.session, out).map_err(|e| e.to_string());
            after_create(app, r);
        },
    )?;
    app.toast_soon(format!("Exporting {n} photo{} for the gallery…", if n == 1 { "" } else { "s" }), 3.0);
    Ok(json!({"background": true, "photos": n}))
}

fn after_create(app: &mut LightcraftApp, r: Result<Value, String>) {
    match r {
        Ok(v) => {
            let k = n(&v, "exported");
            let errs = v.get("errors").and_then(Value::as_array).map_or(0, Vec::len);
            let mut msg = match v.get("gallery").and_then(Value::as_str) {
                Some(_) => format!("Gallery ready: {k} photo{}", if k == 1 { "" } else { "s" }),
                None => format!("Exported {k} photos — {}", s(&v, "note")),
            };
            if errs > 0 {
                let first = v.pointer("/errors/0/error").and_then(Value::as_str).unwrap_or("");
                msg.push_str(&format!(" — {errs} failed: {first}"));
            }
            app.toast_soon(msg, 6.0);
            if let Some(id) = v.get("gallery").and_then(Value::as_str).map(str::to_string) {
                app.galleries.new = NewForm::default();
                open_gallery(app, Some(id));
            } else {
                refresh(app);
            }
        }
        Err(e) => app.toast_soon(e, 6.0),
    }
}

fn start_reexport(app: &mut LightcraftApp, id: &str, all: bool) -> Result<Value, String> {
    if app.tasks.is_running(MAKING) {
        return Err("a gallery is being exported already".into());
    }
    let Some(write) = app.services.write_shared.clone() else {
        let r = app.run("gallery.reexport", json!({"id": id, "all": all}))?;
        refresh(app);
        return Ok(r);
    };
    let job = lightcraft_engine::galleries::prepare_reexport(&mut app.session, id, &[], all)?;
    let count = job.photos();
    if count == 0 {
        app.toast_soon("Every photo in the gallery is up to date", 4.0);
        return Ok(json!({"exported": 0}));
    }
    crate::tasks::spawn(
        app,
        MAKING,
        move || job.run(&mut |path, bytes| write(path, bytes), &mut |_, _| true),
        |app, _ctx, out| {
            match lightcraft_engine::galleries::finish_create(&mut app.session, out) {
                Ok(v) => app.toast_soon(format!("Updated {} photo(s) in the gallery", n(&v, "exported")), 5.0),
                Err(e) => app.toast_soon(e.to_string(), 6.0),
            }
            // the thumbnails show the new files
            let gid = app.galleries.open.clone().unwrap_or_default();
            app.galleries.thumbs.retain(|k, _| !k.starts_with(&format!("{gid}/")));
            refresh(app);
        },
    )?;
    Ok(json!({"background": true, "photos": count}))
}

/// Publishing copies a whole gallery to LOT48: a worker, not the UI thread.
fn start_publish(app: &mut LightcraftApp, id: &str, on: bool) -> Result<Value, String> {
    if on && app.tasks.is_running(PUBLISHING) {
        return Err("a gallery is being published already".into());
    }
    let dir = app.session.second_brain_dir.clone();
    let id = id.to_string();
    let what = if on { "publish" } else { "unpublish" };
    crate::tasks::spawn(
        app,
        PUBLISHING,
        move || {
            let mut a = lightcraft_engine::galleries::connect(dir.as_deref()).map_err(|e| e.to_string())?;
            a.timeout = std::time::Duration::from_secs(15 * 60);
            a.post(&format!("/v1/galleries/{}/{what}", lightcraft_engine::galleries::enc(&id)), &json!({})).map_err(|e| e.to_string())
        },
        move |app, _ctx, r| {
            match r {
                Ok(_) => {
                    app.toast_soon(if on { "Published: the link works away from home" } else { "Taken off LOT48 (the link stops working)" }, 5.0)
                }
                Err(e) => app.toast_soon(e, 6.0),
            }
            refresh(app);
        },
    )?;
    Ok(json!({"background": true}))
}

// ---------- commands (UI side)

fn open_id(app: &LightcraftApp, p: &Value) -> Result<String, String> {
    p.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| app.galleries.open.clone()).ok_or_else(|| "open a gallery first".into())
}

fn open_external(app: &mut LightcraftApp, url: &str) -> Result<Value, String> {
    if url.starts_with("https://") {
        return crate::links::open(app, url);
    }
    // a LAN (http) link: the system opener, which hands it to the browser
    let open = app.services.open_with.as_mut().ok_or("can't open links here")?;
    open(url, "")?;
    Ok(json!({"url": url}))
}

/// Where Second Brain is installed (it is started with the system opener).
fn second_brain_app() -> Option<std::path::PathBuf> {
    let mut places = vec![std::path::PathBuf::from("/Applications/Second Brain.app")];
    if let Some(h) = std::env::var_os("HOME") {
        places.push(std::path::PathBuf::from(h).join("Applications").join("Second Brain.app"));
    }
    places.into_iter().find(|p| p.exists())
}

/// The UI commands of the tab (`galleries.*`, `view.galleries`); engine `gallery.*` commands do
/// the work.
pub fn run_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let r = match id {
        "view.galleries" => {
            crate::menus::enter_library(app, Some(ViewMode::Galleries));
            match p.get("id").and_then(Value::as_str) {
                Some(g) => open_gallery(app, Some(g.to_string())),
                None => refresh(app),
            }
            Ok(json!({"view": "galleries", "online": app.galleries.online, "loaded": app.galleries.loaded}))
        }
        "galleries.refresh" => {
            refresh(app);
            Ok(json!({"refreshing": true}))
        }
        "galleries.open" => {
            open_gallery(app, p.get("id").and_then(Value::as_str).map(str::to_string));
            Ok(json!({"open": app.galleries.open}))
        }
        "galleries.newFrom" => start_create(app, p),
        "galleries.reexport" => open_id(app, p).and_then(|g| start_reexport(app, &g, p.get("all").and_then(Value::as_bool).unwrap_or(false))),
        "galleries.publish" => open_id(app, p).and_then(|g| start_publish(app, &g, p.get("on").and_then(Value::as_bool).unwrap_or(true))),
        "galleries.pick" => {
            let names: Vec<String> = app.galleries.photos().iter().map(|x| s(x, "name")).collect();
            let rt = &mut app.galleries;
            if b(p, "all") {
                rt.picked = names;
            } else if b(p, "none") {
                rt.picked.clear();
            } else if let Some(list) = p.get("names").and_then(Value::as_array) {
                rt.picked = list.iter().filter_map(Value::as_str).filter(|x| names.iter().any(|n| n == x)).map(str::to_string).collect();
            }
            Ok(json!({"picked": rt.picked}))
        }
        "galleries.save" => save(app),
        "galleries.develop" => (|| {
            let g = open_id(app, p)?;
            let name =
                p.get("name").and_then(Value::as_str).map(str::to_string).or_else(|| app.galleries.picked.first().cloned()).ok_or("pick a photo")?;
            let r = app.run("gallery.sourcePhoto", json!({"id": g, "name": name}))?;
            crate::menus::enter_develop(app);
            Ok(r)
        })(),
        "galleries.showPicks" => (|| {
            let g = open_id(app, p)?;
            let r = app.run("gallery.applyPicks", json!({"id": g}))?;
            let album = r.get("album").and_then(Value::as_u64).ok_or("no picks collection")?;
            app.run("library.source", json!({"kind": "album", "id": album}))?;
            crate::menus::enter_library(app, Some(ViewMode::PhotoGrid));
            Ok(r)
        })(),
        "galleries.applyPicks" => (|| {
            let g = open_id(app, p)?;
            let mut q = p.clone();
            if let Some(o) = q.as_object_mut() {
                o.insert("id".into(), json!(g));
            }
            let r = app.run("gallery.applyPicks", q)?;
            let k = r.get("photos").and_then(Value::as_array).map_or(0, Vec::len);
            app.toast_soon(format!("Client picks: {k} photo{} in the library", if k == 1 { "" } else { "s" }), 4.0);
            Ok(r)
        })(),
        "galleries.copyLink" | "galleries.openLink" => (|| {
            let g = open_id(app, p)?;
            let r = app.run("gallery.link", json!({"id": g}))?;
            let url = s(&r, "url");
            app.galleries.link = Some(url.clone());
            if id == "galleries.copyLink" {
                if let Some(ctx) = app.tasks.repaint.clone() {
                    ctx.copy_text(url.clone());
                }
                app.toast_soon("Link copied", 3.0);
                Ok(json!({"url": url}))
            } else {
                open_external(app, &url)
            }
        })(),
        "galleries.hide" => (|| {
            let g = open_id(app, p)?;
            let names = p.get("names").cloned().unwrap_or_else(|| json!(app.galleries.picked));
            let r = app.run("gallery.hide", json!({"id": g, "names": names, "hide": p.get("hide").and_then(Value::as_bool).unwrap_or(true)}))?;
            refresh(app);
            Ok(r)
        })(),
        "galleries.slideshow" => (|| {
            let g = open_id(app, p)?;
            let photos = if app.galleries.picked.is_empty() { Value::Null } else { json!(app.galleries.picked) };
            let r = app.run("gallery.slideshow", json!({"id": g, "photos": photos}))?;
            let rid = s(&r, "renderId");
            app.galleries.show = Some((rid, json!({"state": "running"})));
            Ok(r)
        })(),
        "galleries.start" => (|| {
            let app_path = second_brain_app().ok_or("Second Brain isn't in /Applications")?;
            let open = app.services.open_with.as_mut().ok_or("can't start apps here")?;
            open(&app_path.to_string_lossy(), "")?;
            app.toast_soon("Starting Second Brain…", 4.0);
            Ok(json!({"starting": app_path}))
        })(),
        _ => return None,
    };
    Some(r)
}

fn save(app: &mut LightcraftApp) -> Result<Value, String> {
    let g = app.galleries.open.clone().ok_or("open a gallery first")?;
    let f = app.galleries.form.clone();
    let expires = date_to_ms(&f.expires).ok_or("the expiry is a date like 2027-01-31 (or empty: never)")?;
    let r = app.run(
        "gallery.update",
        json!({
            "id": g,
            "name": f.name.trim(),
            "clientName": f.client.trim(),
            "clientEmail": f.email.trim(),
            "password": f.password,
            "expires": expires,
            "allowDownload": f.downloads,
            "allowPrints": f.prints,
        }),
    )?;
    app.toast_soon("Gallery saved", 3.0);
    refresh(app);
    Ok(r)
}

/// ⌘A picks every photo of the open gallery, Esc picks none (then closes it). True when used.
pub fn keys(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    if app.galleries.open.is_none() {
        return false;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.galleries.picked = app.galleries.photos().iter().map(|x| s(x, "name")).collect();
        return true;
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        if app.galleries.picked.is_empty() {
            open_gallery(app, None);
        } else {
            app.galleries.picked.clear();
        }
        return true;
    }
    false
}

/// Click / ⌘-click / ⇧-click on photo `i`.
fn pick(app: &mut LightcraftApp, names: &[String], i: usize, m: egui::Modifiers) {
    let Some(k) = names.get(i).cloned() else { return };
    let rt = &mut app.galleries;
    let anchor = rt.anchor.as_ref().and_then(|a| names.iter().position(|x| x == a));
    if m.shift
        && let Some(a) = anchor
    {
        for x in names.iter().take(a.max(i) + 1).skip(a.min(i)) {
            if !rt.picked.contains(x) {
                rt.picked.push(x.clone());
            }
        }
    } else if m.command {
        if let Some(p) = rt.picked.iter().position(|x| *x == k) {
            rt.picked.remove(p);
        } else {
            rt.picked.push(k.clone());
        }
        rt.anchor = Some(k);
    } else {
        rt.picked = vec![k.clone()];
        rt.anchor = Some(k);
    }
}

fn toast_err(app: &mut LightcraftApp, ctx: &egui::Context, r: Result<Value, String>) {
    if let Err(e) = r {
        app.toast_error(ctx, e);
    }
}

// ---------- drawing

fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    if !app.galleries.loaded {
        refresh(app);
    }
    let now = ctx.input(|i| i.time);
    // offline: look for Second Brain again now and then (start. takes a few seconds)
    if !app.galleries.online && now - app.galleries.last_poll > 4.0 {
        app.galleries.last_poll = now;
        refresh(app);
    }
    // a slideshow being made
    if let Some((rid, st)) = app.galleries.show.clone()
        && s(&st, "state") == "running"
        && now - app.galleries.last_poll > 1.5
    {
        app.galleries.last_poll = now;
        if let Ok(v) = app.run("gallery.slideshowStatus", json!({"renderId": rid})) {
            app.galleries.show = Some((rid, v));
        }
    }
    if !app.galleries.online || app.galleries.show.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(1500));
    }
}

/// The centre: the cards, or the open gallery's photos.
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    poll(app, &ctx);
    header(app, ui);
    notice(app, ui);
    if app.galleries.open.is_some() {
        photos(app, ui);
    } else {
        cards(app, ui);
    }
}

fn header(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    ui.painter().hline(head.x_range(), head.bottom(), Stroke::new(1.0, t.divider));
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(PAD, 10.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    let open = app.galleries.gallery().cloned();
    if app.galleries.open.is_some()
        && text_button(&mut child, "galleriesBack", "back.", false).on_hover_text(crate::i18n::tr("Back to the galleries")).clicked()
    {
        open_gallery(app, None);
        return;
    }
    let title = match &open {
        Some(g) => s(g, "name"),
        None => crate::i18n::tr("Client Galleries").to_string(),
    };
    child.label(egui::RichText::new(title).font(t.semibold(15.0)).color(t.text));
    let sub = match &open {
        Some(g) => {
            let mut parts = vec![];
            if !s(g, "clientName").is_empty() {
                parts.push(s(g, "clientName"));
            }
            parts.push(format!("{} photos", app.galleries.photos().len()));
            parts.push(format!("♥ {}", app.galleries.photos().iter().filter(|p| b(p, "picked")).count()));
            parts.push(format!("{} views", n(g, "views")));
            parts.join(" · ")
        }
        None => {
            let picks: u64 = app.galleries.list.iter().map(|g| n(g, "picks")).sum();
            let k = app.galleries.list.len();
            format!("{k} galler{} · ♥ {picks}", if k == 1 { "y" } else { "ies" })
        }
    };
    child.label(egui::RichText::new(sub).font(t.font(12.0)).color(t.text_dim));
    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if text_button(ui, "galleriesRefresh", "ref.", false).on_hover_text(crate::i18n::tr("Read the galleries again")).clicked() {
            refresh(app);
        }
        let busy = app.tasks.is_running(LISTING) || app.tasks.is_running(MAKING) || app.tasks.is_running(PUBLISHING);
        if busy {
            ui.label(egui::RichText::new("…").font(t.font(13.0)).color(t.text_dim));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
        }
        if app.galleries.open.is_some() {
            grid_actions(app, ui);
        }
    });
}

/// all. / none. and what to do with the picked photos.
fn grid_actions(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let picked = app.galleries.picked.len();
    if text_button(ui, "galleriesNone", "none.", false).clicked() {
        app.galleries.picked.clear();
    }
    if text_button(ui, "galleriesAll", "all.", false).clicked() {
        app.galleries.picked = app.galleries.photos().iter().map(|x| s(x, "name")).collect();
    }
    if picked > 0 {
        let any_hidden = app.galleries.photos().iter().any(|p| b(p, "hidden") && app.galleries.picked.contains(&s(p, "name")));
        if !app.galleries.read_only() {
            let (label, tip) = if any_hidden {
                ("show.", "Put the picked photos back in the client's gallery")
            } else {
                ("hide.", "Take the picked photos out of the client's gallery (the files stay)")
            };
            if text_button(ui, "galleriesHide", label, false).on_hover_text(crate::i18n::tr(tip)).clicked() {
                let r = app.run("galleries.hide", json!({"hide": !any_hidden}));
                toast_err(app, &ctx, r);
            }
        }
        let linked = app.galleries.photos().iter().any(|p| p.get("photo").is_some_and(Value::is_u64) && app.galleries.picked.contains(&s(p, "name")));
        if linked && text_button(ui, "galleriesDevelop", "dev.", false).on_hover_text(crate::i18n::tr("Open the source photo in Develop")).clicked() {
            let r = app.run("galleries.develop", json!({}));
            toast_err(app, &ctx, r);
        }
        ui.label(egui::RichText::new(format!("{picked} picked")).font(t.font(12.0)).color(t.text_dim));
    }
}

fn notice(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let Some(msg) = app.galleries.notice.clone().filter(|_| !app.galleries.online) else { return };
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
    let band = r.shrink2(vec2(PAD, 6.0));
    ui.painter().rect(band, 4.0, t.inset, Stroke::new(1.0, t.caution), StrokeKind::Inside);
    register(ui.ctx(), "notice:galleriesOffline", band);
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(band.shrink2(vec2(10.0, 4.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    child.spacing_mut().item_spacing.x = 8.0;
    if second_brain_app().is_some()
        && text_button(&mut child, "galleriesStart", "start.", false).on_hover_text(crate::i18n::tr("Open Second Brain")).clicked()
    {
        let r = app.run("galleries.start", json!({}));
        let ctx = child.ctx().clone();
        toast_err(app, &ctx, r);
    }
    let w = child.available_width();
    child.add_sized(vec2(w, 24.0), egui::Label::new(egui::RichText::new(format!("⚠ {msg}")).font(t.font(12.0)).color(t.caution)).truncate());
}

fn cards(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let list = app.galleries.list.clone();
    if list.is_empty() {
        let (title, body) = if !app.galleries.loaded {
            ("Looking for Second Brain…", "")
        } else if app.galleries.online {
            ("No galleries yet", "Pick photos (or show a collection) and use sel. or view. on the left to make one.")
        } else {
            ("No galleries to show", "Second Brain keeps the galleries: start it to see and make them.")
        };
        super::empty_message(ui, ui.available_rect_before_wrap(), title, body);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("galleries-cards").show(ui, |ui| {
        let width = ui.available_width();
        let cols = (((width - PAD * 2.0 + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
        let row_h = CARD_IMG_H + CARD_TEXT_H + GAP;
        ui.add_space(PAD);
        let (area, _) = ui.allocate_exact_size(vec2(width, list.len().div_ceil(cols) as f32 * row_h + PAD), Sense::hover());
        for (i, g) in list.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let r =
                Rect::from_min_size(area.min + vec2(PAD + col as f32 * (CARD_W + GAP), row as f32 * row_h), vec2(CARD_W, CARD_IMG_H + CARD_TEXT_H));
            if !ui.clip_rect().intersects(r) {
                continue;
            }
            card(app, ui, g, r, &t);
        }
    });
}

fn draw_tex(ui: &egui::Ui, tex: &egui::TextureHandle, r: Rect, cover: bool) {
    let [w, h] = tex.size();
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let uv = if cover {
        // fill the box, cropping the long side
        let (ra, ta) = (r.width() / r.height().max(1.0), w / h);
        if ta > ra {
            let f = ra / ta;
            Rect::from_min_max(pos2((1.0 - f) / 2.0, 0.0), pos2((1.0 + f) / 2.0, 1.0))
        } else {
            let f = ta / ra;
            Rect::from_min_max(pos2(0.0, (1.0 - f) / 2.0), pos2(1.0, (1.0 + f) / 2.0))
        }
    } else {
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))
    };
    let dest = if cover {
        r
    } else {
        let s = (r.width() / w).min(r.height() / h);
        Rect::from_center_size(r.center(), vec2(w * s, h * s))
    };
    ui.painter().image(tex.id(), dest, uv, Color32::WHITE);
}

fn thumb_at(app: &mut LightcraftApp, ui: &egui::Ui, gid: &str, folder: &str, name: &str, r: Rect, cover: bool) {
    let t = Tokens::get(ui.ctx());
    if name.is_empty() {
        return;
    }
    match app.galleries.thumbs.get(&format!("{gid}/{name}")) {
        Some(Thumb::Ready(tex)) => {
            let tex = tex.clone();
            draw_tex(ui, &tex, r, cover);
        }
        Some(Thumb::Failed) => {
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "⚠", t.font(14.0), t.text_dim);
        }
        Some(Thumb::Loading) => {
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "…", t.font(14.0), t.text_dim);
        }
        None => {
            load_thumb(app, gid, folder, name);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, "…", t.font(14.0), t.text_dim);
        }
    }
}

fn card(app: &mut LightcraftApp, ui: &mut egui::Ui, g: &Value, r: Rect, t: &Tokens) {
    let id = s(g, "id");
    let resp = ui.interact(r, egui::Id::new(("galleryCard", &id)), Sense::click());
    register(ui.ctx(), format!("galleryCard:{id}"), r);
    let p = ui.painter();
    p.rect(r, 6.0, if resp.hovered() { t.hover } else { t.cell }, Stroke::new(1.0, t.divider), StrokeKind::Inside);
    let img = Rect::from_min_size(r.min + vec2(1.0, 1.0), vec2(r.width() - 2.0, CARD_IMG_H - 1.0));
    p.rect_filled(img, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.inset);
    let cover = s(g, "cover");
    thumb_at(app, ui, &id, &s(g, "folder"), &cover, img, true);
    let p = ui.painter();
    // live / not live, top right
    let live = b(g, "published");
    let chip_text = if live { "live." } else { "local." };
    let cg = p.layout_no_wrap(chip_text.to_string(), t.semibold(10.5), t.text);
    let chip = Rect::from_min_size(pos2(img.right() - cg.size().x - 18.0, img.top() + 8.0), vec2(cg.size().x + 10.0, 20.0));
    p.rect_filled(chip, 4.0, if live { t.pick.gamma_multiply(0.85) } else { Color32::from_black_alpha(170) });
    p.galley(chip.center() - cg.size() / 2.0, cg, t.text);
    if b(g, "linked") {
        let lg = p.layout_no_wrap("lc.".to_string(), t.semibold(10.5), t.text);
        let lchip = Rect::from_min_size(pos2(chip.left() - lg.size().x - 18.0, chip.top()), vec2(lg.size().x + 10.0, 20.0));
        p.rect_filled(lchip, 4.0, Color32::from_black_alpha(170));
        p.galley(lchip.center() - lg.size() / 2.0, lg, t.text);
    }
    // name, client, counts
    let tx = r.left() + 12.0;
    let w = r.width() - 24.0;
    let mut job = egui::text::LayoutJob::simple_singleline(s(g, "name"), t.semibold(13.0), t.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(w);
    p.galley(pos2(tx, img.bottom() + 10.0), p.layout_job(job), t.text);
    let client = if s(g, "clientName").is_empty() { "—".to_string() } else { s(g, "clientName") };
    p.text(pos2(tx, img.bottom() + 30.0), Align2::LEFT_TOP, client, t.font(12.0), t.text_label);
    let mut stats = format!("{} photos · ♥ {} · {} views", n(g, "count"), n(g, "picks"), n(g, "views"));
    if n(g, "orders") > 0 {
        stats.push_str(&format!(" · {} orders", n(g, "orders")));
    }
    if b(g, "hasPassword") {
        stats.push_str(" · pw");
    }
    p.text(pos2(tx, img.bottom() + 48.0), Align2::LEFT_TOP, stats, t.font(11.0), t.text_dim);
    if resp.clicked() {
        open_gallery(app, Some(id));
    }
}

fn photos(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let list = app.galleries.photos();
    let gid = app.galleries.open.clone().unwrap_or_default();
    let folder = app.galleries.gallery().map(|g| s(g, "folder")).unwrap_or_default();
    if app.galleries.detail.is_none() {
        super::empty_message(ui, ui.available_rect_before_wrap(), "Opening the gallery…", "");
        return;
    }
    if list.is_empty() {
        super::empty_message(ui, ui.available_rect_before_wrap(), "No photos in this gallery", "Its folder is empty, or not reachable right now.");
        return;
    }
    let names: Vec<String> = list.iter().map(|p| s(p, "name")).collect();
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("gallery-photos").show(ui, |ui| {
        let width = ui.available_width();
        let cols = (((width - PAD * 2.0 + 8.0) / (TILE + 8.0)).floor() as usize).max(1);
        let cell = ((width - PAD * 2.0 - (cols as f32 - 1.0) * 8.0) / cols as f32).max(60.0);
        ui.add_space(12.0);
        let (area, _) = ui.allocate_exact_size(vec2(width, list.len().div_ceil(cols) as f32 * (cell + 8.0) + PAD), Sense::hover());
        for (i, ph) in list.iter().enumerate() {
            let (row, col) = (i / cols, i % cols);
            let r = Rect::from_min_size(area.min + vec2(PAD + col as f32 * (cell + 8.0), row as f32 * (cell + 8.0)), vec2(cell, cell));
            if !ui.clip_rect().intersects(r) {
                continue;
            }
            let name = s(ph, "name");
            let resp = ui.interact(r, egui::Id::new(("galleryPhoto", &gid, &name)), Sense::click());
            register(ui.ctx(), format!("galleryPhoto:{name}"), r);
            let picked = app.galleries.picked.contains(&name);
            let p = ui.painter();
            p.rect_filled(r, 3.0, if picked { t.cell_selected } else { t.cell });
            let inner = r.shrink(6.0);
            thumb_at(app, ui, &gid, &folder, &name, inner, false);
            let p = ui.painter();
            if b(ph, "hidden") {
                p.rect_filled(inner, 0.0, Color32::from_black_alpha(150));
                p.text(inner.center(), Align2::CENTER_CENTER, "hidden.", t.semibold(11.0), t.text_label);
            }
            // badges: the client's heart, his highlight, made here
            let mut bx = inner.left() + 4.0;
            let mut badge = |text: &str, fill: Color32| {
                let g = p.layout_no_wrap(text.to_string(), t.semibold(11.0), Color32::WHITE);
                let br = Rect::from_min_size(pos2(bx, inner.top() + 4.0), vec2(g.size().x + 10.0, 20.0));
                p.rect_filled(br, 4.0, fill);
                p.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
                bx = br.right() + 8.0;
            };
            if b(ph, "picked") {
                badge("♥", Color32::from_rgb(214, 74, 120));
            }
            if b(ph, "highlight") {
                badge("★", Color32::from_black_alpha(170));
            }
            if ph.get("photo").is_some_and(Value::is_u64) {
                badge("lc.", Color32::from_black_alpha(170));
            }
            if picked {
                p.rect_stroke(r, 3.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
            }
            let resp = resp.on_hover_text(&name);
            if resp.double_clicked() && ph.get("photo").is_some_and(Value::is_u64) {
                let r = app.run("galleries.develop", json!({"name": name}));
                let ctx = ui.ctx().clone();
                toast_err(app, &ctx, r);
            } else if resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                pick(app, &names, i, m);
            }
        }
    });
}

// ---------- side panels

fn side_title(ui: &mut egui::Ui, title: &str, t: &Tokens) {
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().text(pos2(hr.left() + 18.0, hr.center().y), Align2::LEFT_CENTER, crate::i18n::tr(title), t.semibold(13.5), t.text_label);
    ui.painter().hline(hr.x_range(), hr.bottom(), Stroke::new(1.0, t.divider));
}

fn field(ui: &mut egui::Ui, id: &str, label: &str, value: &mut String, hint: &str, password: bool, enabled: bool) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    ui.label(egui::RichText::new(crate::i18n::tr(label)).font(t.font(11.5)).color(t.text_dim));
    ui.add_space(4.0);
    let r =
        ui.add_enabled(enabled, egui::TextEdit::singleline(value).hint_text(crate::i18n::tr(hint)).password(password).desired_width(f32::INFINITY));
    register(ui.ctx(), format!("field:{id}"), r.rect);
}

/// Left: new gallery from… and the client book.
pub fn left(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let width = app.ui.left_width;
    let resized = super::resizable_side(ui, true, "left_panel", frame, width, crate::state::LEFT_WIDTH, 0.0, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        side_title(ui, "New Gallery", &t);
        let ctx = ui.ctx().clone();
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 4)).show(ui, |ui| {
            let online = app.galleries.online;
            let nf = &mut app.galleries.new;
            field(ui, "galleryNewName", "Name", &mut nf.name, "the collection's name", false, online);
            field(ui, "galleryNewClient", "Client", &mut nf.client, "who it's for", false, online);
            field(ui, "galleryNewWatermark", "Watermark", &mut nf.watermark, "none (text or a watermark preset)", false, online);
            ui.add_space(12.0);
            let sel = app.session.selection.ids.len().max(usize::from(app.session.selection.active.is_some()));
            let view = app.session.visible().len();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let busy = app.tasks.is_running(MAKING);
                if text_button(ui, "galleryFromSelection", "sel.", false)
                    .on_hover_text(format!("{} ({sel})", crate::i18n::tr("Make a gallery of the selected photos, with their edits")))
                    .clicked()
                {
                    let r = if busy {
                        Err("a gallery is being exported already".into())
                    } else {
                        app.run("galleries.newFrom", json!({"source": "selection"}))
                    };
                    toast_err(app, &ctx, r);
                }
                if text_button(ui, "galleryFromView", "view.", false)
                    .on_hover_text(format!(
                        "{} ({view})",
                        crate::i18n::tr("Make a gallery of what the grid shows: a collection, a shoot or one camera folder of it")
                    ))
                    .clicked()
                {
                    let r = if busy {
                        Err("a gallery is being exported already".into())
                    } else {
                        app.run("galleries.newFrom", json!({"source": "view"}))
                    };
                    toast_err(app, &ctx, r);
                }
            });
            ui.add_space(8.0);
            let how = if online {
                format!("{sel} selected · {view} in view. Exported as web JPEGs (2048 px, sRGB) into <shoot>/gallery/<name>.")
            } else {
                "Second Brain isn't running: galleries can't be made now.".to_string()
            };
            ui.add(egui::Label::new(egui::RichText::new(how).font(t.font(11.5)).color(t.text_dim)).wrap());
            ui.add_space(12.0);
        });
        side_title(ui, "Client Book", &t);
        egui::ScrollArea::vertical().id_salt("galleries-clients").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(6.0);
            let clients = app.galleries.clients.as_ref().and_then(|c| c.get("clients")).and_then(Value::as_array).cloned().unwrap_or_default();
            if clients.is_empty() {
                let msg = if app.galleries.online {
                    "No clients in the book yet: add them in Second Brain (client tab, book)."
                } else {
                    "The client book shows while Second Brain runs."
                };
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 8)).show(ui, |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(msg)).font(t.font(12.0)).color(t.text_dim)).wrap());
                });
            }
            for c in clients.iter().take(500) {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                register(ui.ctx(), format!("client:{}", s(c, "id")), r);
                if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let shoots = n(c, "shoots");
                ui.painter().text(pos2(r.left() + 18.0, r.top() + 7.0), Align2::LEFT_TOP, s(c, "name"), t.font(13.0), t.text_label);
                let mut meta = format!("{shoots} shoot{}", if shoots == 1 { "" } else { "s" });
                if !s(c, "email").is_empty() {
                    meta.push_str(&format!(" · {}", s(c, "email")));
                }
                ui.painter().text(pos2(r.left() + 18.0, r.top() + 24.0), Align2::LEFT_TOP, meta, t.font(11.0), t.text_dim);
                // a click fills the new gallery's client
                if resp.clicked() {
                    app.galleries.new.client = s(c, "name");
                }
            }
            ui.add_space(12.0);
        });
    });
    if let Some(w) = resized {
        app.ui.left_width = w;
    }
}

/// Right: the open gallery's settings, link, publishing, picks and slideshow.
pub fn right(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let width = app.ui.right_width;
    let resized = super::resizable_side(ui, false, "right_panel", frame, width, crate::state::RIGHT_WIDTH, 0.0, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(g) = app.galleries.gallery().cloned() else {
            side_title(ui, "Gallery", &t);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 12)).show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(crate::i18n::tr("Open a gallery to see its settings, its link and the client's picks."))
                            .font(t.font(12.0))
                            .color(t.text_dim),
                    )
                    .wrap(),
                );
            });
            return;
        };
        let ctx = ui.ctx().clone();
        let ro = app.galleries.read_only();
        egui::ScrollArea::vertical().id_salt("gallery-settings").auto_shrink([false, false]).show(ui, |ui| {
            side_title(ui, "Settings", &t);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 4)).show(ui, |ui| {
                let f = &mut app.galleries.form;
                field(ui, "galleryName", "Title", &mut f.name, "", false, !ro);
                field(ui, "galleryClient", "Client", &mut f.client, "", false, !ro);
                field(ui, "galleryEmail", "Email", &mut f.email, "", false, !ro);
                field(ui, "galleryPassword", "Password", &mut f.password, "none", true, !ro);
                field(ui, "galleryExpires", "Expires", &mut f.expires, "never (YYYY-MM-DD)", false, !ro);
                ui.add_space(10.0);
                let r =
                    ui.add_enabled(!ro, egui::Checkbox::new(&mut f.downloads, egui::RichText::new(crate::i18n::tr("Downloads")).font(t.font(12.0))));
                register(ui.ctx(), "check:galleryDownloads", r.rect);
                ui.add_space(8.0);
                let r =
                    ui.add_enabled(!ro, egui::Checkbox::new(&mut f.prints, egui::RichText::new(crate::i18n::tr("Print store")).font(t.font(12.0))));
                register(ui.ctx(), "check:galleryPrints", r.rect);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let changed = app.galleries.gallery().map(form_of).is_some_and(|server| server != app.galleries.form);
                    if !ro && text_button(ui, "gallerySave", "save.", changed).clicked() {
                        let r = save(app);
                        toast_err(app, &ctx, r);
                    }
                    if !ro
                        && changed
                        && text_button(ui, "galleryRevert", "undo.", false).on_hover_text(crate::i18n::tr("Back to the saved settings")).clicked()
                    {
                        app.galleries.form = app.galleries.gallery().map(form_of).unwrap_or_default();
                    }
                });
                ui.add_space(12.0);
            });
            side_title(ui, "Link", &t);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                let state = if b(&g, "published") { "live on LOT48: works anywhere" } else { "on this Mac: works at home (pub. puts it on LOT48)" };
                ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(state)).font(t.font(12.0)).color(t.text_dim)).wrap());
                if let Some(url) = app.galleries.link.clone() {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(egui::RichText::new(url).font(t.font(11.0)).color(t.text_label)).truncate());
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if !ro {
                        if text_button(ui, "galleryCopyLink", "copy.", false).on_hover_text(crate::i18n::tr("Copy the client's link")).clicked() {
                            let r = app.run("galleries.copyLink", json!({}));
                            toast_err(app, &ctx, r);
                        }
                        if text_button(ui, "galleryOpenLink", "open.", false)
                            .on_hover_text(crate::i18n::tr("Open the client's link in the browser"))
                            .clicked()
                        {
                            let r = app.run("galleries.openLink", json!({}));
                            toast_err(app, &ctx, r);
                        }
                        let live = b(&g, "published");
                        let (label, tip) = if live {
                            ("unpub.", "Take it off LOT48 (the link stops working away from home)")
                        } else {
                            ("pub.", "Copy the photos to LOT48 so the link works anywhere")
                        };
                        if text_button(ui, "galleryPublish", label, false).on_hover_text(crate::i18n::tr(tip)).clicked() {
                            let r = app.run("galleries.publish", json!({"on": !live}));
                            toast_err(app, &ctx, r);
                        }
                    }
                });
                ui.add_space(4.0);
            });
            side_title(ui, "Client Picks", &t);
            egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                let hearts = app.galleries.photos().iter().filter(|p| b(p, "picked")).count();
                let linked = b(&g, "linked");
                let msg = if linked {
                    format!("♥ {hearts} picked by the client. flag. / star. mark their source photos; show. opens the picks collection.")
                } else {
                    format!("♥ {hearts} picked by the client. Made outside LightCraft, so they don't map to library photos.")
                };
                ui.add(egui::Label::new(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim)).wrap());
                if linked && !ro {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if text_button(ui, "galleryPicksFlag", "flag.", false)
                            .on_hover_text(crate::i18n::tr("Flag the client's picks as Picks"))
                            .clicked()
                        {
                            let r = app.run("galleries.applyPicks", json!({"flag": "pick"}));
                            toast_err(app, &ctx, r);
                        }
                        if text_button(ui, "galleryPicksStar", "star.", false)
                            .on_hover_text(crate::i18n::tr("Give the client's picks 4 stars"))
                            .clicked()
                        {
                            let r = app.run("galleries.applyPicks", json!({"rating": 4}));
                            toast_err(app, &ctx, r);
                        }
                        if text_button(ui, "galleryPicksShow", "show.", false)
                            .on_hover_text(crate::i18n::tr("Show the client picks collection in the Library"))
                            .clicked()
                        {
                            let r = app.run("galleries.showPicks", json!({}));
                            toast_err(app, &ctx, r);
                        }
                    });
                }
                ui.add_space(4.0);
            });
            if b(&g, "linked") && !ro {
                side_title(ui, "Photos", &t);
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                    let msg = format!("{} photos exported from this library. re.exp. writes the ones edited since, in place.", n(&g, "linkedPhotos"));
                    ui.add(egui::Label::new(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim)).wrap());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if text_button(ui, "galleryReexport", "re.exp.", false)
                            .on_hover_text(crate::i18n::tr("Export the photos edited since again"))
                            .clicked()
                        {
                            let r = app.run("galleries.reexport", json!({}));
                            toast_err(app, &ctx, r);
                        }
                        if text_button(ui, "galleryReexportAll", "all.", false).on_hover_text(crate::i18n::tr("Export every photo again")).clicked() {
                            let r = app.run("galleries.reexport", json!({"all": true}));
                            toast_err(app, &ctx, r);
                        }
                    });
                    ui.add_space(4.0);
                });
            }
            if !ro {
                side_title(ui, "Slideshow", &t);
                egui::Frame::NONE.inner_margin(egui::Margin::symmetric(18, 10)).show(ui, |ui| {
                    let what = if app.galleries.picked.is_empty() {
                        "every photo".to_string()
                    } else {
                        format!("the {} picked photos", app.galleries.picked.len())
                    };
                    let state = app.galleries.show.as_ref().map(|(_, st)| (s(st, "state"), s(st, "path"), s(st, "error")));
                    let msg = match &state {
                        Some((st, _, _)) if st == "running" => "Making the slideshow…".to_string(),
                        Some((st, path, _)) if st == "done" => format!("Done: {path}"),
                        Some((_, _, err)) => format!("⚠ {err}"),
                        None => format!("slide. makes a film of {what} (Second Brain's editor has music and titles)."),
                    };
                    ui.add(egui::Label::new(egui::RichText::new(msg).font(t.font(12.0)).color(t.text_dim)).wrap());
                    ui.add_space(10.0);
                    let running = state.as_ref().is_some_and(|(st, _, _)| st == "running");
                    if !running && text_button(ui, "gallerySlideshow", "slide.", false).clicked() {
                        let r = app.run("galleries.slideshow", json!({}));
                        toast_err(app, &ctx, r);
                    }
                    ui.add_space(12.0);
                });
            }
        });
    });
    if let Some(w) = resized {
        app.ui.right_width = w;
    }
}
