//! LC-GALLERIES: the galleries. tab's commands — client galleries owned by Second Brain, driven
//! over its local gallery API ([`crate::galleries`]; `docs/galleries.md`). Every action is a
//! command, so the UI, the control channel and MCP do the same thing. Without Second Brain
//! running, `gallery.list` / `gallery.get` read its galleries file (read-only) and every change
//! is refused with a notice saying so.

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::galleries::{self, Api, ApiError, enc};
use crate::{Result, Session};
use lightcraft_catalog::PhotoId;

fn api(s: &Session, c: &str) -> Result<Api> {
    galleries::connect(s.second_brain_dir.as_deref()).map_err(|e| bad(c, e.to_string()))
}

fn id_param<'a>(p: &'a Value, c: &str) -> Result<&'a str> {
    str_param(p, "id").filter(|i| !i.is_empty() && i.len() < 100).ok_or_else(|| bad(c, "missing gallery `id`"))
}

fn fail(c: &str, e: ApiError) -> crate::EngineError {
    bad(c, e.to_string())
}

fn annotate(s: &Session, g: Value) -> Value {
    galleries::annotate(&s.catalog, g)
}

fn status(s: &Session) -> Value {
    let dir = galleries::sb_dir(s.second_brain_dir.as_deref());
    match galleries::connect(s.second_brain_dir.as_deref()) {
        Ok(a) => json!({"online": true, "port": a.port, "dir": dir}),
        Err(e) => json!({"online": false, "dir": dir, "notice": e.to_string()}),
    }
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let l = galleries::fetch_list(s.second_brain_dir.as_deref()).map_err(|e| bad("gallery.list", e))?;
    let list: Vec<Value> = l.galleries.into_iter().map(|g| annotate(s, g)).collect();
    Ok(json!({"online": l.online, "readOnly": !l.online, "notice": l.notice, "galleries": list}))
}

/// The photos of a gallery, each with the library photo it came from when it was made here.
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.get";
    let id = id_param(p, C)?.to_string();
    let mut v = galleries::fetch_gallery(s.second_brain_dir.as_deref(), &id).map_err(|e| bad(C, e))?;
    galleries::annotate_detail(&s.catalog, &id, &mut v);
    Ok(v)
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.create";
    let job = galleries::prepare_create(s, p).map_err(|e| bad(C, e))?;
    let out = job.run(&mut |path, bytes| crate::export::write_file(path, bytes), &mut |_, _| true);
    galleries::finish_create(s, out)
}

fn reexport(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.reexport";
    let id = id_param(p, C)?.to_string();
    let add: Vec<PhotoId> =
        p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(PhotoId).collect()).unwrap_or_default();
    let all = p.get("all").and_then(Value::as_bool).unwrap_or(false);
    let job = galleries::prepare_reexport(s, &id, &add, all).map_err(|e| bad(C, e))?;
    if job.photos() == 0 {
        return Ok(json!({"gallery": id, "exported": 0, "note": "every photo is up to date"}));
    }
    let out = job.run(&mut |path, bytes| crate::export::write_file(path, bytes), &mut |_, _| true);
    galleries::finish_create(s, out)
}

/// A PATCH of the record's settings.
fn update(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.update";
    let id = id_param(p, C)?.to_string();
    let a = api(s, C)?;
    let mut patch = serde_json::Map::new();
    for k in ["name", "clientName", "clientEmail", "password", "expires", "allowDownload", "allowPrints", "layout", "cover"] {
        if let Some(v) = p.get(k) {
            patch.insert(k.into(), v.clone());
        }
    }
    if patch.is_empty() {
        return Err(bad(C, "nothing to change (name, clientName, clientEmail, password, expires, allowDownload, allowPrints, layout, cover)"));
    }
    let v = a.call("PATCH", &format!("/v1/galleries/{}", enc(&id)), Some(&Value::Object(patch.clone()))).map_err(|e| fail(C, e))?;
    if let Some(name) = patch.get("name").and_then(Value::as_str)
        && let Some(mut l) = galleries::link(&s.catalog, &id)
    {
        l.name = name.to_string();
        s.apply_bookkeeping(galleries::set_link(&l))?;
    }
    Ok(json!({"gallery": annotate(s, v.get("gallery").cloned().unwrap_or(Value::Null))}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.delete";
    let id = id_param(p, C)?.to_string();
    api(s, C)?.call("DELETE", &format!("/v1/galleries/{}", enc(&id)), None).map_err(|e| fail(C, e))?;
    if galleries::link(&s.catalog, &id).is_some() {
        s.apply_bookkeeping(galleries::drop_link(&id))?;
    }
    Ok(json!({"deleted": id, "note": "the gallery's record is gone; its folder and photos are untouched"}))
}

/// `POST /v1/galleries/:id/<what>` with `body`.
fn post_on(s: &Session, c: &str, p: &Value, what: &str, body: Value) -> Result<Value> {
    let id = id_param(p, c)?;
    api(s, c)?.post(&format!("/v1/galleries/{}/{what}", enc(id)), &body).map_err(|e| fail(c, e))
}

fn get_on(s: &Session, c: &str, p: &Value, what: &str) -> Result<Value> {
    let id = id_param(p, c)?;
    api(s, c)?.get(&format!("/v1/galleries/{}/{what}", enc(id))).map_err(|e| fail(c, e))
}

/// The client's picks, each mapped to its library photo when the gallery was made here.
fn picks(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.picks";
    let id = id_param(p, C)?.to_string();
    let v =
        if p.get("pull").and_then(Value::as_bool).unwrap_or(true) { post_on(s, C, p, "picks/pull", json!({}))? } else { get_on(s, C, p, "picks")? };
    let more = if p.get("pull").and_then(Value::as_bool).unwrap_or(true) { get_on(s, C, p, "picks")? } else { v.clone() };
    let selected: Vec<String> =
        more.get("selected").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let photos: Vec<u64> = galleries::link(&s.catalog, &id)
        .map(|l| galleries::photos_for(&l, &selected, &s.catalog).into_iter().map(|p| p.0).collect())
        .unwrap_or_default();
    Ok(json!({
        "gallery": id,
        "selected": selected,
        "photos": photos,
        "highlights": more.get("highlights"),
        "comments": more.get("comments"),
        "pulled": v.get("pulled"),
    }))
}

/// Client picks → the library: flags / stars / a label on their source photos, and the gallery's
/// "client picks" collection (one undo step each).
fn apply_picks(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.applyPicks";
    let id = id_param(p, C)?.to_string();
    let mut l =
        galleries::link(&s.catalog, &id).ok_or_else(|| bad(C, "this gallery wasn't made in LightCraft, so its photos don't map to the library"))?;
    let got = picks(s, &json!({"id": id, "pull": p.get("pull").cloned().unwrap_or(json!(true))}))?;
    let ids: Vec<u64> = got.get("photos").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let mut done = serde_json::Map::new();
    if !ids.is_empty() {
        if let Some(f) = p.get("flag").and_then(Value::as_str) {
            s.execute("photo.flag", &json!({"flag": f, "ids": ids}))?;
            done.insert("flag".into(), json!(f));
        }
        if let Some(r) = p.get("rating").and_then(Value::as_u64) {
            s.execute("photo.rate", &json!({"rating": r, "ids": ids}))?;
            done.insert("rating".into(), json!(r));
        }
        if let Some(lb) = p.get("label").filter(|v| !v.is_null()) {
            s.execute("photo.label", &json!({"label": lb, "ids": ids}))?;
            done.insert("label".into(), lb.clone());
        }
    }
    let album = if p.get("collection").and_then(Value::as_bool).unwrap_or(true) {
        let ph: Vec<PhotoId> = ids.iter().copied().map(PhotoId).collect();
        Some(galleries::sync_picks_album(s, &mut l, &ph)?)
    } else {
        None
    };
    Ok(json!({"gallery": id, "photos": ids, "applied": done, "album": album}))
}

/// The library photo behind a gallery file (selected, so Develop opens on it).
fn source_photo(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.sourcePhoto";
    let id = id_param(p, C)?.to_string();
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name` (a file in the gallery)"))?;
    let l = galleries::link(&s.catalog, &id).ok_or_else(|| bad(C, "this gallery wasn't made in LightCraft"))?;
    let photo = l
        .files
        .get(name)
        .copied()
        .filter(|ph| s.catalog.photo(PhotoId(*ph)).is_some_and(|x| x.in_library()))
        .ok_or_else(|| bad(C, format!("{name} has no source photo in this library")))?;
    if p.get("select").and_then(Value::as_bool).unwrap_or(true) {
        // show it wherever it is: the whole library, then the photo
        if !s.visible().contains(&PhotoId(photo)) {
            let _ = s.execute("library.source", &json!({"kind": "all"}));
            let _ = s.execute("library.clearFilter", &json!({}));
        }
        s.execute("library.select", &json!({"ids": [photo]}))?;
    }
    Ok(json!({"photo": photo}))
}

fn slideshow(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "gallery.slideshow";
    let mut body = serde_json::Map::new();
    for k in ["photos", "vibe", "song", "title"] {
        if let Some(v) = p.get(k) {
            body.insert(k.into(), v.clone());
        }
    }
    post_on(s, C, p, "slideshow", Value::Object(body))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "gallery.status", "Galleries Status", [], None, "{} → {online, port?, dir, notice?}: is Second Brain's gallery API up", always, |s, _| Ok(status(s))),
        cmd!(
            query "gallery.list",
            "Client Galleries",
            [],
            None,
            "{} → {online, readOnly, notice?, galleries: [{id, name, clientName, count, cover, published, expires, allowDownload, allowPrints, hasPassword, views, picks, orders, folder, linked, linkedPhotos}]}. Second Brain not running: read-only, from its galleries file",
            always,
            list
        ),
        cmd!(
            query "gallery.get",
            "Gallery",
            [],
            None,
            "{id} → {gallery, photos: [{name, photo: library id | null, picked, highlight, hidden}], videos, files}",
            always,
            get
        ),
        cmd!(
            "gallery.create",
            "New Client Gallery",
            [],
            None,
            "{name?, source?: selection|view|album, album?, ids?, folder?: absolute (default <shoot>/gallery/<name>), clientName?, clientEmail?, password?, expires?: ms, allowDownload?, allowPrints?, watermark?: text | {…} | a watermark preset's name, export?: app.export overrides (default JPEG 2048 px sRGB, screen sharpening), exportOnly?: true (no Second Brain: just the files)} — exports the photos with their edits into the gallery folder and registers the gallery → {gallery, folder, exported, errors, registered}",
            always,
            create
        ),
        cmd!(
            "gallery.reexport",
            "Update Gallery Photos",
            [],
            None,
            "{id, all?: bool, ids?: [photo ids to add]} — re-exports the photos edited since (each file replaced in place) → {exported, errors}",
            always,
            reexport
        ),
        cmd!(
            "gallery.update",
            "Gallery Settings",
            [],
            None,
            "{id, name?, clientName?, clientEmail?, password?, expires?: ms (0 = never), allowDownload?, allowPrints?, layout?, cover?}",
            always,
            update
        ),
        cmd!(
            "gallery.delete",
            "Delete Gallery",
            [],
            None,
            "{id} — removes the gallery's record in Second Brain (folder and photos untouched)",
            always,
            delete
        ),
        cmd!("gallery.setFolder", "Point Gallery at Folder", [], None, "{id, folder}", always, |s, p| {
            let f = str_param(p, "folder").ok_or_else(|| bad("gallery.setFolder", "missing `folder`"))?.to_string();
            post_on(s, "gallery.setFolder", p, "folder", json!({"folder": f}))
        }),
        cmd!(
            "gallery.publish",
            "Publish Gallery",
            [],
            None,
            "{id} — copies the photos to LOT48 so the link works away from home",
            always,
            |s, p| post_on(s, "gallery.publish", p, "publish", json!({}))
        ),
        cmd!("gallery.unpublish", "Unpublish Gallery", [], None, "{id}", always, |s, p| post_on(s, "gallery.unpublish", p, "unpublish", json!({}))),
        cmd!(query "gallery.link", "Gallery Link", [], None, "{id} → {url, public, permanent, links: [per recipient]}", always, |s, p| get_on(s, "gallery.link", p, "link")),
        cmd!("gallery.hide", "Hide in Gallery", [], None, "{id, names: [file names], hide?: bool (default true)}", always, |s, p| {
            let names = p.get("names").cloned().ok_or_else(|| bad("gallery.hide", "missing `names`"))?;
            post_on(s, "gallery.hide", p, "hidden", json!({"names": names, "hide": p.get("hide").and_then(Value::as_bool).unwrap_or(true)}))
        }),
        cmd!(
            query "gallery.picks",
            "Client Picks",
            [],
            None,
            "{id, pull?: bool (default true: fetch from LOT48 first)} → {selected: [file names], photos: [library ids], highlights, comments}",
            always,
            picks
        ),
        cmd!(
            "gallery.applyPicks",
            "Apply Client Picks",
            [],
            None,
            "{id, flag?: pick|reject|none, rating?: 0..5, label?: red|yellow|green|blue|purple, collection?: bool (default true: keep the gallery's “client picks” collection in step), pull?: bool} → {photos, applied, album}",
            always,
            apply_picks
        ),
        cmd!(
            "gallery.sourcePhoto",
            "Show Source Photo",
            [],
            None,
            "{id, name, select?: bool (default true)} → {photo}: the library photo a gallery file was exported from",
            always,
            source_photo
        ),
        cmd!(query "gallery.stats", "Gallery Activity", [], None, "{id} → {views, viewers, downloads, events}", always, |s, p| get_on(s, "gallery.stats", p, "stats")),
        cmd!(query "gallery.orders", "Gallery Print Orders", [], None, "{id} → {orders}", always, |s, p| get_on(s, "gallery.orders", p, "orders")),
        cmd!("gallery.orderStatus", "Set Print Order Status", [], None, "{orderId, status}", always, |s, p| {
            const C: &str = "gallery.orderStatus";
            let o = str_param(p, "orderId").ok_or_else(|| bad(C, "missing `orderId`"))?;
            let st = str_param(p, "status").ok_or_else(|| bad(C, "missing `status`"))?;
            api(s, C)?.post(&format!("/v1/orders/{}/status", enc(o)), &json!({"status": st})).map_err(|e| fail(C, e))
        }),
        cmd!(
            "gallery.slideshow",
            "Make Slideshow",
            [],
            None,
            "{id, photos?: [names] (default all), vibe?, song?: audio path, title?: {text, subtitle}} → {renderId}; poll gallery.slideshowStatus",
            always,
            slideshow
        ),
        cmd!(query "gallery.slideshowStatus", "Slideshow Status", [], None, "{renderId} → {state: running|done|failed|cancelled, path, error}", always, |s, p| {
            const C: &str = "gallery.slideshowStatus";
            let r = str_param(p, "renderId").ok_or_else(|| bad(C, "missing `renderId`"))?;
            api(s, C)?.get(&format!("/v1/slideshows/{}", enc(r))).map_err(|e| fail(C, e))
        }),
        cmd!(query "gallery.proofs", "Print Proofs", [], None, "{} → {proofs}", always, |s, _| api(s, "gallery.proofs")?.get("/v1/proofs").map_err(|e| fail("gallery.proofs", e))),
        cmd!(query "gallery.clientBook", "Client Book", [], None, "{} → {clients: [{id, name, phone, email, shoots, …}], …}", always, |s, _| {
            api(s, "gallery.clientBook")?.get("/v1/clientbook").map_err(|e| fail("gallery.clientBook", e))
        }),
        cmd!("gallery.saveClient", "Save Client", [], None, "{client: {id?, name, phone?, email?, address?, notes?}}", always, |s, p| {
            const C: &str = "gallery.saveClient";
            let c = p.get("client").filter(|c| c.is_object()).ok_or_else(|| bad(C, "missing `client` {…}"))?;
            api(s, C)?.post("/v1/clientbook/clients", &json!({"client": c})).map_err(|e| fail(C, e))
        }),
        cmd!("gallery.saveShoot", "Save Shoot", [], None, "{shoot: {id?, clientId, type?, when?: ms, location?, total?}}", always, |s, p| {
            const C: &str = "gallery.saveShoot";
            let c = p.get("shoot").filter(|c| c.is_object()).ok_or_else(|| bad(C, "missing `shoot` {…}"))?;
            api(s, C)?.post("/v1/clientbook/shoots", &json!({"shoot": c})).map_err(|e| fail(C, e))
        }),
        cmd!(query "gallery.mailDraft", "Gallery Email Draft", [], None, "{id, recipient?, all?: bool} → {to, subject, body, url} (a draft; nothing is sent)", always, |s, p| {
            let mut b = serde_json::Map::new();
            for k in ["recipient", "all"] {
                if let Some(v) = p.get(k) {
                    b.insert(k.into(), v.clone());
                }
            }
            post_on(s, "gallery.mailDraft", p, "mail-draft", Value::Object(b))
        }),
    ]
}
