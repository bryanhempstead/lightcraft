//! LRC-LIB-PUBLISH commands (`publish.*`, see [`crate::publish`]) and LR-EXP-PHOTOS
//! (`export.toPhotos`, see [`crate::apple_photos`]).

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::publish::{self, Collection, Service, ServiceKind};
use crate::{Result, Session};

fn id_param(p: &Value, key: &str, c: &str) -> Result<u64> {
    p.get(key).and_then(Value::as_u64).ok_or_else(|| bad(c, format!("missing `{key}` (a number)")))
}

fn name_param(p: &Value, c: &str) -> Result<Option<String>> {
    match p.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(n)) if !n.trim().is_empty() && n.trim().chars().count() <= 200 => Ok(Some(n.trim().to_string())),
        Some(_) => Err(bad(c, "`name` is 1–200 characters")),
    }
}

/// The export params a service is given: `export` (app.export options), or a named `preset`'s,
/// checked like `app.export`'s.
fn export_param(s: &Session, p: &Value, c: &str) -> Result<Option<Value>> {
    let mut v = match (p.get("export"), str_param(p, "preset")) {
        (Some(Value::Object(_)), _) => p["export"].clone(),
        (Some(Value::Null) | None, Some(name)) => s.export_params(&json!({"preset": name})).map_err(|e| bad(c, e))?,
        (Some(Value::Null) | None, None) => return Ok(None),
        (Some(_), _) => return Err(bad(c, "`export` is an object of app.export options")),
    };
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| !matches!(k.as_str(), "ids" | "dir" | "path" | "preset" | "folder" | "subfolders" | "background"));
    }
    crate::export::ExportOptions::validate(c, &v)?;
    Ok(Some(v))
}

fn dir_param(p: &Value, c: &str) -> Result<Option<String>> {
    match p.get("dir") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(d)) if d.trim().is_empty() => Ok(Some(String::new())),
        Some(Value::String(d)) if std::path::Path::new(d.trim()).is_absolute() => Ok(Some(d.trim().to_string())),
        Some(_) => Err(bad(c, "`dir` must be an absolute folder path")),
    }
}

fn service_json(s: &Session, svc: &Service) -> Value {
    let all = publish::collections(&s.catalog);
    let cols: Vec<Value> = all.iter().filter(|c| c.service == svc.id).map(|c| collection_json(s, c)).collect();
    json!({
        "id": svc.id, "kind": svc.kind, "kindLabel": svc.kind.label(), "name": svc.name, "dir": svc.dir,
        "export": svc.export, "collections": cols,
    })
}

fn collection_json(s: &Session, c: &Collection) -> Value {
    json!({"id": c.id, "service": c.service, "name": c.name, "set": c.set, "parent": c.parent, "counts": publish::counts(&s.catalog, c)})
}

fn get_service(s: &Session, id: u64, c: &str) -> Result<Service> {
    publish::service(&s.catalog, id).ok_or_else(|| bad(c, format!("no publish service {id}")))
}

fn get_collection(s: &Session, id: u64, c: &str) -> Result<Collection> {
    publish::collection(&s.catalog, id).ok_or_else(|| bad(c, format!("no published collection {id}")))
}

/// A published collection (not a set) to put photos in.
fn get_photo_collection(s: &Session, id: u64, c: &str) -> Result<Collection> {
    let col = get_collection(s, id, c)?;
    if col.set {
        return Err(bad(c, format!("“{}” is a collection set: photos go in its collections", col.name)));
    }
    Ok(col)
}

fn create_service(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.createService";
    let kind = match str_param(p, "kind") {
        None => ServiceKind::HardDrive,
        Some(k) => ServiceKind::parse(k).ok_or_else(|| bad(C, format!("unknown service kind `{k}` (hardDrive, photos)")))?,
    };
    let name = name_param(p, C)?.unwrap_or_else(|| kind.label().to_string());
    let dir = dir_param(p, C)?.unwrap_or_default();
    let export = export_param(s, p, C)?.unwrap_or_else(|| json!({"format": "jpeg", "quality": 90, "colorSpace": "srgb"}));
    if publish::services(&s.catalog).iter().any(|x| x.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(C, format!("a service named “{name}” exists")));
    }
    let svc = publish::new_service(&s.catalog, kind, &name, &dir, export);
    s.commit("Create Publish Service", publish::set_service(&svc))?;
    Ok(service_json(s, &svc))
}

fn edit_service(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.editService";
    let mut svc = get_service(s, id_param(p, "id", C)?, C)?;
    if let Some(n) = name_param(p, C)? {
        svc.name = n;
    }
    if let Some(d) = dir_param(p, C)? {
        svc.dir = d;
    }
    if let Some(e) = export_param(s, p, C)? {
        svc.export = e;
    }
    s.commit("Edit Publish Service", publish::set_service(&svc))?;
    Ok(service_json(s, &svc))
}

fn delete_service(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.deleteService";
    let svc = get_service(s, id_param(p, "id", C)?, C)?;
    let mut ops = vec![lightcraft_catalog::Op::SetRecord { key: format!("{}{}", publish::SERVICE_PREFIX, svc.id), value: None }];
    for c in publish::collections(&s.catalog).into_iter().filter(|c| c.service == svc.id) {
        ops.push(lightcraft_catalog::Op::SetRecord { key: format!("{}{}", publish::COLLECTION_PREFIX, c.id), value: None });
    }
    s.commit("Delete Publish Service", lightcraft_catalog::Op::Batch { ops })?;
    Ok(json!({"deleted": svc.id}))
}

fn create_collection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.createCollection";
    let svc = get_service(s, id_param(p, "service", C)?, C)?;
    let name = name_param(p, C)?.ok_or_else(|| bad(C, "missing `name`"))?;
    let set = p.get("set").and_then(Value::as_bool).unwrap_or(false);
    let parent = match p.get("parent").and_then(Value::as_u64) {
        Some(pid) => {
            let par = get_collection(s, pid, C)?;
            if !par.set || par.service != svc.id {
                return Err(bad(C, "`parent` must be a collection set of the same service"));
            }
            Some(pid)
        }
        None => None,
    };
    let mut col = publish::new_collection(&s.catalog, svc.id, &name, set, parent);
    if !set {
        let ids: Vec<u64> = match p.get("ids") {
            Some(Value::Array(_)) => s.targets(&json!({"ids": p["ids"]})).iter().map(|i| i.0).collect(),
            _ => Vec::new(),
        };
        let ids: Vec<u64> = ids.into_iter().filter(|i| s.catalog.photo(lightcraft_catalog::PhotoId(*i)).is_some()).collect();
        publish::add_photos(&mut col, &ids);
    }
    s.commit(if set { "Create Published Collection Set" } else { "Create Published Collection" }, publish::set_collection(&col))?;
    Ok(collection_json(s, &col))
}

fn edit_collection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.editCollection";
    let mut col = get_collection(s, id_param(p, "id", C)?, C)?;
    if let Some(n) = name_param(p, C)? {
        col.name = n;
    }
    match p.get("parent") {
        Some(Value::Null) => col.parent = None,
        Some(v) => {
            let pid = v.as_u64().ok_or_else(|| bad(C, "`parent` is a set's id or null"))?;
            let par = get_collection(s, pid, C)?;
            let all = publish::collections(&s.catalog);
            // never into itself or one of its own sets
            let mut up = Some(pid);
            let mut depth = 0;
            while let Some(u) = up {
                if u == col.id || depth > 64 {
                    return Err(bad(C, "a set can't go inside itself"));
                }
                up = all.iter().find(|x| x.id == u).and_then(|x| x.parent);
                depth += 1;
            }
            if !par.set || par.service != col.service {
                return Err(bad(C, "`parent` must be a collection set of the same service"));
            }
            col.parent = Some(pid);
        }
        None => {}
    }
    s.commit("Edit Published Collection", publish::set_collection(&col))?;
    Ok(collection_json(s, &col))
}

fn delete_collection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.deleteCollection";
    let col = get_collection(s, id_param(p, "id", C)?, C)?;
    let all = publish::collections(&s.catalog);
    // a set goes with everything in it
    let mut gone = vec![col.id];
    let mut i = 0;
    while let Some(id) = gone.get(i).copied() {
        let kids: Vec<u64> = all.iter().filter(|c| c.parent == Some(id) && !gone.contains(&c.id)).map(|c| c.id).collect();
        gone.extend(kids);
        i += 1;
    }
    let ops = gone.iter().map(|id| lightcraft_catalog::Op::SetRecord { key: format!("{}{id}", publish::COLLECTION_PREFIX), value: None }).collect();
    s.commit("Delete Published Collection", lightcraft_catalog::Op::Batch { ops })?;
    Ok(json!({"deleted": gone}))
}

fn photo_ids(s: &Session, p: &Value) -> Vec<u64> {
    let mut q = p.clone();
    // `id` names the collection here, not a photo
    if let Some(o) = q.as_object_mut() {
        o.remove("id");
    }
    s.targets(&q).iter().map(|i| i.0).collect()
}

fn add_photos(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.addPhotos";
    let mut col = get_photo_collection(s, id_param(p, "id", C)?, C)?;
    let ids: Vec<u64> =
        photo_ids(s, p).into_iter().filter(|i| s.catalog.photo(lightcraft_catalog::PhotoId(*i)).is_some_and(|x| x.in_library())).collect();
    if ids.is_empty() {
        return Err(bad(C, "no photos to add"));
    }
    let n = publish::add_photos(&mut col, &ids);
    s.commit("Add to Published Collection", publish::set_collection(&col))?;
    Ok(json!({"added": n, "collection": collection_json(s, &col)}))
}

fn remove_photos(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.removePhotos";
    let mut col = get_photo_collection(s, id_param(p, "id", C)?, C)?;
    let ids = photo_ids(s, p);
    let n = publish::remove_photos(&mut col, &ids);
    s.commit("Remove from Published Collection", publish::set_collection(&col))?;
    Ok(json!({"removed": n, "collection": collection_json(s, &col)}))
}

fn status(s: &Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.status";
    let col = get_collection(s, id_param(p, "id", C)?, C)?;
    let photos: Vec<Value> = publish::states(&s.catalog, &col).into_iter().map(|(id, st)| json!({"id": id, "state": st})).collect();
    let mut v = collection_json(s, &col);
    v["photos"] = Value::Array(photos);
    Ok(v)
}

fn mark_up_to_date(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.markUpToDate";
    let mut col = get_photo_collection(s, id_param(p, "id", C)?, C)?;
    let ids = if p.get("ids").is_some() { photo_ids(s, p) } else { col.photos.clone() };
    let at = (s.clock)();
    let n = publish::mark_up_to_date(&s.catalog, &mut col, &ids, &at);
    s.commit("Mark as Up-to-Date", publish::set_collection(&col))?;
    Ok(json!({"marked": n, "collection": collection_json(s, &col)}))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "publish.run";
    let id = id_param(p, "id", C)?;
    let only: Option<Vec<u64>> = p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect());
    publish::publish_now(s, id, only.as_deref())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "publish.services",
            "Publish Services",
            [],
            None,
            "{} → [{id, kind, name, dir, export, collections: [{id, name, set, parent, counts: {new, modified, published, deleted, total}}]}]",
            always,
            |s, _| Ok(Value::Array(publish::services(&s.catalog).iter().map(|svc| service_json(s, svc)).collect()))
        ),
        cmd!(
            "publish.createService",
            "Create Publish Service",
            [],
            None,
            "{kind?: hardDrive|photos (default hardDrive), name?, dir: absolute folder (hardDrive), export?: app.export options | preset?: export preset name (default JPEG, quality 90, sRGB)} → service",
            always,
            create_service
        ),
        cmd!(
            "publish.editService",
            "Edit Publish Service",
            [],
            None,
            "{id, name?, dir?, export? | preset?} → service (published photos keep their state; Mark as Up-to-Date or re-publish after changing the export settings)",
            always,
            edit_service
        ),
        cmd!(
            "publish.deleteService",
            "Delete Publish Service",
            [],
            None,
            "{id} — the service and its published collections (published copies stay where they are) → {deleted}",
            always,
            delete_service
        ),
        cmd!(
            "publish.createCollection",
            "Create Published Collection",
            [],
            None,
            "{service, name, set?: bool (a collection set), parent?: set id, ids?: photos to put in it} → collection",
            always,
            create_collection
        ),
        cmd!(
            "publish.editCollection",
            "Edit Published Collection",
            [],
            None,
            "{id, name?, parent?: set id | null} → collection",
            always,
            edit_collection
        ),
        cmd!(
            "publish.deleteCollection",
            "Delete Published Collection",
            [],
            None,
            "{id} — a collection, or a set with everything in it (published copies stay) → {deleted: [ids]}",
            always,
            delete_collection
        ),
        cmd!(
            "publish.addPhotos",
            "Add to Published Collection",
            [],
            None,
            "{id: collection, ids?: photos (default the selection)} — they show as New until published → {added, collection}",
            always,
            add_photos
        ),
        cmd!(
            "publish.removePhotos",
            "Remove from Published Collection",
            [],
            None,
            "{id: collection, ids?: photos (default the selection)} — a published one's copy is removed from the destination on the next publish → {removed, collection}",
            always,
            remove_photos
        ),
        cmd!(
            query "publish.status",
            "Published Collection Status",
            [],
            None,
            "{id} → {id, name, counts: {new, modified, published, deleted, total}, photos: [{id, state: new|modified|published|deleted}]}",
            always,
            |s, p| status(s, p)
        ),
        cmd!(
            "publish.markUpToDate",
            "Mark as Up-to-Date",
            [],
            None,
            "{id: collection, ids?: photos (default all of it)} — counts them as published as they are now, without publishing → {marked, collection}",
            always,
            mark_up_to_date
        ),
        cmd!(
            "publish.run",
            "Publish Now",
            [],
            None,
            "{id: collection, ids?: only these of its New / Modified photos} — exports New and Modified photos with the service's settings, delivers them, removes the copies of photos taken out → {published, removed, errors: [{photo, file?, error}], counts}",
            always,
            run
        ),
        cmd!(
            "export.toPhotos",
            "Export to Photos",
            [],
            None,
            "{ids? (default the selection), album?: Photos album (made when missing), …app.export options (default JPEG 92, sRGB)} — macOS: exports, then imports into Apple Photos through its AppleScript import command → {imported, album, files, errors}",
            super::has_selection,
            |s, p| crate::apple_photos::export_to_photos(s, p, &crate::apple_photos::run_osascript)
        ),
    ]
}
