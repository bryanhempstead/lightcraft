//! Folder-scoped commands ("a shoot" = the photos under one folder on disk), for pipelines that
//! drive LightCraft the way they drive Lightroom Classic: is the shoot in the library, import the
//! keepers with their stars, open the shoot, export it (`app.export {folder, …}`), and export it
//! as a standalone library (Lightroom's "Export as Catalog").

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use lightcraft_catalog::{Album, AlbumId, ColorLabel, Flag, Op, PhotoId, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::{Result, Selection, Session};

/// `folder` without trailing separators, or an error for an empty one.
fn folder_param(p: &Value, c: &str) -> Result<String> {
    let f = str_param(p, "folder").map(|f| f.trim().trim_end_matches(['/', '\\'])).filter(|f| !f.is_empty());
    f.map(str::to_string).ok_or_else(|| bad(c, "missing `folder`"))
}

/// Is `path` inside `folder` (directly, or anywhere below it with `subfolders`)?
pub fn is_under(path: &str, folder: &str, subfolders: bool) -> bool {
    let folder = folder.trim_end_matches(['/', '\\']);
    let Some(rest) = path.strip_prefix(folder).and_then(|r| r.strip_prefix(['/', '\\'])) else { return false };
    !rest.is_empty() && (subfolders || !rest.contains(['/', '\\']))
}

/// The library's photos (virtual copies included) whose files are under `folder`, by capture
/// date then file name.
pub fn photos_under(s: &Session, folder: &str, subfolders: bool) -> Vec<PhotoId> {
    let mut v: Vec<_> = s
        .catalog
        .photos()
        .filter(|p| p.in_library())
        .filter(|p| matches!(&p.source, Source::File { path } if is_under(path, folder, subfolders)))
        .map(|p| (p.date().to_string(), p.file_name.clone(), p.id))
        .collect();
    v.sort();
    v.into_iter().map(|(_, _, id)| id).collect()
}

fn status(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.folderStatus";
    let folder = folder_param(p, C)?;
    let sub = bool_or(p, "subfolders", true);
    let ids = photos_under(s, &folder, sub);
    let mut ratings: BTreeMap<String, usize> = (0..=5).map(|r| (r.to_string(), 0)).collect();
    let (mut picks, mut rejects, mut edited, mut copies) = (0, 0, 0, 0);
    let mut paths = HashSet::new();
    for id in &ids {
        let Some(ph) = s.catalog.photo(*id) else { continue };
        *ratings.entry(ph.rating.min(5).to_string()).or_default() += 1;
        match ph.flag {
            Flag::Pick => picks += 1,
            Flag::Reject => rejects += 1,
            Flag::None => {}
        }
        edited += usize::from(ph.is_edited());
        copies += usize::from(ph.copy_of.is_some());
        if let Source::File { path } = &ph.source {
            paths.insert(path.clone());
        }
    }
    let mut out = json!({
        "folder": folder,
        "exists": Path::new(&folder).is_dir(),
        "inLibrary": ids.len(),
        "virtualCopies": copies,
        "ratings": ratings,
        "picks": picks,
        "rejects": rejects,
        "edited": edited,
        "resume": crate::lr_migrate::resume_point(s, Some(&folder), None, sub),
    });
    // the files on disk an import would take, and those not in the library yet
    if bool_or(p, "scanDisk", true) && Path::new(&folder).is_dir() {
        let files: Vec<String> =
            crate::import::expand(std::slice::from_ref(&folder), None).into_iter().filter(|f| is_under(f, &folder, sub)).collect();
        let missing: Vec<&String> = files.iter().filter(|f| !paths.contains(*f)).collect();
        out["onDisk"] = json!(files.len());
        out["notInLibrary"] = json!(missing.len());
        out["notInLibraryFiles"] = json!(missing.iter().take(200).collect::<Vec<_>>());
    }
    if bool_or(p, "ids", false) {
        out["ids"] = json!(ids.iter().map(|i| i.0).collect::<Vec<_>>());
    }
    Ok(out)
}

fn import_rated(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.importRated";
    let files = p.get("files").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or_else(|| bad(C, "missing `files`"))?;
    // {path, rating?, flag?, label?, keywords?} or a plain path
    let entries: Vec<(String, &Value)> = files
        .iter()
        .filter_map(|f| match f {
            Value::String(path) => Some((path.clone(), &Value::Null)),
            o => Some((o.get("path")?.as_str()?.to_string(), o)),
        })
        .collect();
    let mut params = json!({"paths": entries.iter().map(|e| &e.0).collect::<Vec<_>>(), "mode": str_param(p, "mode").unwrap_or("add")});
    for k in ["album", "albumName", "destination", "organize", "rename", "preset", "keywords", "metadataPreset"] {
        if let Some(v) = p.get(k) {
            params[k] = v.clone();
        }
    }
    let undo0 = s.undo.len();
    // photos already here before this import keep what was set in LightCraft: a rating, flag or
    // label from the camera/card only fills in what is still unset (Bryan, 11-23)
    let existed: std::collections::HashSet<PhotoId> = s.catalog.photos().map(|ph| ph.id).collect();
    let report = s.execute("library.import", &params)?;
    // the files' photos: imported now, or already in the library (duplicates by path)
    let by_path: std::collections::HashMap<String, PhotoId> = s
        .catalog
        .photos()
        .filter(|ph| ph.in_library() && ph.copy_of.is_none())
        .filter_map(|ph| match &ph.source {
            Source::File { path } => Some((path.clone(), ph.id)),
            _ => None,
        })
        .collect();
    let mut ops = Vec::new();
    let mut set = 0usize;
    let mut not_found = Vec::new();
    // moved files are found at their new place
    let moved: std::collections::HashMap<&str, &str> =
        report["moved"].as_array().into_iter().flatten().filter_map(|m| Some((m["from"].as_str()?, m["to"].as_str()?))).collect();
    for (path, e) in &entries {
        let now_at = moved.get(path.as_str()).copied().unwrap_or(path.as_str());
        let Some(id) = by_path.get(now_at).copied() else {
            not_found.push(path.clone());
            continue;
        };
        let Some(ph) = s.catalog.photo(id) else { continue };
        let before = ops.len();
        let keep = existed.contains(&id);
        if let Some(r) = e.get("rating").and_then(Value::as_u64).filter(|_| !keep || ph.rating == 0) {
            ops.push(Op::SetRating { id, rating: r.min(5) as u8 });
        }
        if let Some(f) = e.get("flag").and_then(Value::as_str).filter(|_| !keep || ph.flag == Flag::None) {
            ops.push(Op::SetFlag { id, flag: Flag::parse(f).ok_or_else(|| bad(C, format!("flag `{f}`: pick, reject or none")))? });
        }
        if let Some(l) = e.get("label").filter(|_| !keep || ph.label.is_none()) {
            let label = match l.as_str().map(str::trim) {
                None | Some("") | Some("none") => None,
                Some(name) => {
                    Some(s.catalog.label_from_name(name).or_else(|| ColorLabel::parse(name)).ok_or_else(|| bad(C, format!("label `{name}`")))?)
                }
            };
            ops.push(Op::SetLabel { id, label });
        }
        if let Some(k) = e.get("keywords").and_then(Value::as_array) {
            let mut m = ph.meta.clone();
            for w in k.iter().filter_map(Value::as_str).map(str::trim).filter(|w| !w.is_empty()) {
                if !m.keywords.iter().any(|x| x.eq_ignore_ascii_case(w)) {
                    m.keywords.push(w.to_string());
                }
            }
            ops.push(Op::SetMeta { id, meta: Box::new(m) });
        }
        set += usize::from(ops.len() > before);
    }
    if !ops.is_empty() {
        s.commit("Set Ratings", Op::Batch { ops })?;
    }
    let n = s.undo.len().saturating_sub(undo0);
    s.merge_undo(n, "Import Rated Photos");
    Ok(json!({"import": report, "rated": set, "notFound": not_found}))
}

fn show(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.showFolder";
    let folder = folder_param(p, C)?;
    let sub = bool_or(p, "subfolders", true);
    let ids = photos_under(s, &folder, sub);
    s.end_interaction()?;
    s.source = crate::LibrarySource::All;
    s.filter = if sub {
        let rules = json!({"match": "all", "rules": [{"field": "filePath", "op": "startsWith", "value": format!("{folder}/")}]});
        lightcraft_catalog::Filter { rule_set: serde_json::from_value(rules).ok(), ..Default::default() }
    } else {
        lightcraft_catalog::Filter { only: ids.clone(), ..Default::default() }
    };
    let resume = crate::lr_migrate::resume_point(s, Some(&folder), None, sub);
    let vis = s.visible_cloned();
    let active = resume["photoId"].as_u64().map(PhotoId).filter(|a| vis.contains(a)).or(vis.first().copied());
    s.selection = if bool_or(p, "select", true) { Selection { ids: vis.clone(), active } } else { active.map(Selection::single).unwrap_or_default() };
    Ok(json!({"folder": folder, "count": vis.len(), "active": active.map(|a| a.0), "resume": resume}))
}

fn export_catalog(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.exportCatalog";
    let dest = str_param(p, "dest").map(str::trim).filter(|d| !d.is_empty()).ok_or_else(|| bad(C, "missing `dest` (the new library's folder)"))?;
    let ids: Vec<PhotoId> = match p.get("ids").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).map(PhotoId).collect(),
        None => photos_under(s, &folder_param(p, C)?, bool_or(p, "subfolders", true)),
    };
    let mut photos: Vec<lightcraft_catalog::Photo> = ids.iter().filter_map(|id| s.catalog.photo(*id)).map(|ph| (**ph).clone()).collect();
    // a virtual copy goes with its master
    let have: HashSet<PhotoId> = photos.iter().map(|ph| ph.id).collect();
    let masters: Vec<PhotoId> = photos.iter().filter_map(|ph| ph.copy_of).filter(|m| !have.contains(m)).collect();
    photos.extend(masters.iter().filter_map(|m| s.catalog.photo(*m)).map(|ph| (**ph).clone()));
    if photos.is_empty() {
        return Err(bad(C, "no photos to export"));
    }
    let dir = Path::new(dest);
    if dir.exists() && std::fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(true) {
        return Err(bad(C, format!("{dest}: the folder exists and isn't empty (choose a new one)")));
    }
    let keep: HashSet<PhotoId> = photos.iter().map(|ph| ph.id).collect();
    // regular albums with any of the photos, with the folders above them
    let mut albums: Vec<Album> = Vec::new();
    let mut seen: HashSet<AlbumId> = HashSet::new();
    for a in s.catalog.albums().filter(|a| !a.folder && !a.is_smart() && !a.quick && a.photos.iter().any(|x| keep.contains(x))) {
        let mut chain = vec![a.clone()];
        let mut parent = a.parent;
        while let Some(pid) = parent.filter(|pid| !seen.contains(pid) && chain.len() < 32) {
            let Some(f) = s.catalog.album(pid) else { break };
            chain.push(f.clone());
            parent = f.parent;
        }
        for mut al in chain.into_iter().rev() {
            if seen.insert(al.id) {
                al.photos.retain(|x| keep.contains(x));
                al.cover = al.cover.filter(|c| keep.contains(c));
                albums.push(al);
            }
        }
    }
    let mut out = Session::new().with_fs();
    out.open_library(dir, false)?;
    let n = photos.len();
    let mut ops: Vec<Op> = photos
        .into_iter()
        .map(|mut ph| {
            ph.local = false;
            ph.deleted = false;
            Op::AddPhoto { photo: Box::new(ph) }
        })
        .collect();
    let n_albums = albums.len();
    // parents before children (the chain order above)
    ops.extend(albums.into_iter().map(|album| Op::AddAlbum { album }));
    out.execute_fn(C, |o| o.commit("Export as Catalog", Op::Batch { ops }).map(|_| Value::Null))?;
    Ok(json!({"dest": dest, "photos": n, "albums": n_albums}))
}

/// Lightroom's Import from Another Catalog (L30): the photos (with their settings, metadata,
/// history and virtual copies), regular albums and smart albums of another LightCraft library,
/// read from a copy of its files (never written). A photo whose file this library already has is
/// skipped, or with `replace: true` takes the other library's settings, metadata, rating, flag and
/// label. One undo step.
fn import_catalog(s: &mut Session, p: &Value) -> Result<Value> {
    use lightcraft_catalog::{Journal, MemStore};
    const C: &str = "library.importCatalog";
    let path = str_param(p, "path").map(str::trim).filter(|d| !d.is_empty()).ok_or_else(|| bad(C, "missing `path` (the other library's folder)"))?;
    let dir = Path::new(path);
    if s.library.as_ref().is_some_and(|l| l.dir == dir) {
        return Err(bad(C, "that is this library"));
    }
    let copy = MemStore::new();
    let mut found = false;
    for name in [lightcraft_catalog::journal::SNAPSHOT, lightcraft_catalog::journal::LOG] {
        match std::fs::read(dir.join(name)) {
            Ok(b) => {
                copy.set(name, b);
                found = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(bad(C, format!("{}: {e}", dir.join(name).display()))),
        }
    }
    if !found {
        return Err(bad(C, format!("{path}: not a LightCraft library (no catalog.snap)")));
    }
    let (_, other, _) = Journal::open(Box::new(copy)).map_err(|e| bad(C, format!("{path}: {e}")))?;
    // what a photo both libraries have takes from the other one: nothing, its metadata (rating,
    // flag, label, metadata), or everything (also its develop settings)
    let replace = match p.get("replace") {
        None | Some(Value::Null) | Some(Value::Bool(false)) => "none",
        Some(Value::Bool(true)) => "all",
        Some(Value::String(r)) if ["none", "all", "metadata"].contains(&r.as_str()) => r.as_str(),
        Some(_) => return Err(bad(C, "`replace` is none, metadata or all (true = all)")),
    };
    let dry = bool_or(p, "dryRun", false);
    let mine: std::collections::HashMap<String, PhotoId> = s
        .catalog
        .photos()
        .filter(|q| q.copy_of.is_none() && q.in_library())
        .filter_map(|q| match &q.source {
            Source::File { path } => Some((path.clone(), q.id)),
            Source::Demo { .. } => None,
        })
        .collect();
    let mut ids: std::collections::HashMap<PhotoId, PhotoId> = Default::default();
    let mut fresh: HashSet<PhotoId> = HashSet::new();
    let mut ops: Vec<Op> = Vec::new();
    let (mut added, mut replaced, mut skipped) = (0usize, 0usize, 0usize);
    let mut theirs: Vec<&lightcraft_catalog::Photo> = other.photos().map(|q| q.as_ref()).filter(|q| q.in_library()).collect();
    // originals before their virtual copies
    theirs.sort_by_key(|q| (q.copy_of.is_some(), q.id));
    for q in theirs {
        let existing = match (&q.source, q.copy_of) {
            (Source::File { path }, None) => mine.get(path).copied(),
            _ => None,
        };
        if let Some(e) = existing {
            ids.insert(q.id, e);
            if replace != "none" {
                if replace == "all" {
                    ops.push(Op::SetDevelop { id: e, settings: q.develop.clone(), label: "Import from Catalog".into(), edited: q.edited.clone() });
                }
                ops.push(Op::SetMeta { id: e, meta: Box::new(q.meta.clone()) });
                ops.push(Op::SetRating { id: e, rating: q.rating });
                ops.push(Op::SetFlag { id: e, flag: q.flag });
                ops.push(Op::SetLabel { id: e, label: q.label });
                replaced += 1;
            } else {
                skipped += 1;
            }
            continue;
        }
        // a virtual copy comes with an original that comes now (one this library already has
        // keeps its own copies)
        let copy_of = match q.copy_of {
            Some(m) => match ids.get(&m).filter(|n| fresh.contains(n)) {
                Some(n) => Some(*n),
                None => {
                    skipped += 1;
                    continue;
                }
            },
            None => None,
        };
        let new = s.catalog.alloc_photo_id();
        ids.insert(q.id, new);
        fresh.insert(new);
        let mut ph = q.clone();
        ph.id = new;
        ph.copy_of = copy_of;
        ph.local = false;
        ops.push(Op::AddPhoto { photo: Box::new(ph) });
        added += 1;
    }
    // albums: folders first (parents before children), photos remapped
    let mut album_ids: std::collections::HashMap<lightcraft_catalog::AlbumId, lightcraft_catalog::AlbumId> = Default::default();
    let mut albums: Vec<lightcraft_catalog::Album> = other.albums().filter(|a| !a.quick).cloned().collect();
    let depth = |a: &lightcraft_catalog::Album| {
        let mut d = 0;
        let mut cur = a.parent;
        while let Some(pid) = cur.filter(|_| d < 32) {
            d += 1;
            cur = other.album(pid).and_then(|x| x.parent);
        }
        d
    };
    albums.sort_by_key(|a| (depth(a), a.id));
    let mut n_albums = 0;
    for mut a in albums {
        let new = s.catalog.alloc_album_id();
        album_ids.insert(a.id, new);
        a.id = new;
        a.parent = a.parent.and_then(|pid| album_ids.get(&pid).copied());
        a.photos = a.photos.iter().filter_map(|x| ids.get(x).copied()).collect();
        a.cover = a.cover.and_then(|c| ids.get(&c).copied());
        if !a.folder && !a.is_smart() && a.photos.is_empty() {
            continue;
        }
        ops.push(Op::AddAlbum { album: a });
        n_albums += 1;
    }
    if dry {
        // the dialog's preview: nothing changes
        let both = replaced + skipped;
        return Ok(
            json!({"dryRun": true, "added": added, "inBoth": both, "albums": n_albums, "photos": other.photos().filter(|q| q.in_library()).count()}),
        );
    }
    if !ops.is_empty() {
        s.commit("Import from Catalog", Op::Batch { ops })?;
    }
    Ok(json!({"added": added, "replaced": replaced, "skipped": skipped, "albums": n_albums}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.importCatalog",
            "Import from Another Library…",
            [],
            None,
            "{path: another LightCraft library's folder, replace?: none | metadata | all (photos both have: keep this library's, take the other's metadata, or its metadata and develop settings; true = all), dryRun?: bool (count only)} — Lightroom's Import from Another Catalog: its photos (settings, metadata, history, virtual copies), albums and smart albums; the other library is only read; one undo step → {added, replaced, skipped, albums}",
            always,
            import_catalog
        ),
        cmd!(
            query "library.folderStatus",
            "Folder Status",
            [],
            None,
            "{folder, subfolders?: bool (default true), scanDisk?: bool (default true), ids?: bool} → {folder, exists, inLibrary, virtualCopies, ratings: {\"0\"..\"5\": n}, picks, rejects, edited, resume (library.resumePoint), onDisk?, notInLibrary?, notInLibraryFiles? (≤ 200), ids?} — is a shoot in the library",
            always,
            status
        ),
        cmd!(
            "library.importRated",
            "Import Rated Photos",
            [],
            None,
            "{files: [path | {path, rating?: 0..5, flag?: pick|reject|none, label?: name|none, keywords?: [..]}], mode?: add|copy|move (default add), album?, albumName?, destination?, organize?, rename?, preset?, keywords?, metadataPreset?} → {import (library.import's report), rated, notFound} — import the keepers with their stars (files already in the library just get the ratings); one undo step",
            always,
            import_rated
        ),
        cmd!(
            "library.showFolder",
            "Show Folder",
            [],
            None,
            "{folder, subfolders?: bool (default true), select?: bool (default true)} → {folder, count, active, resume} — shows the library's photos under a folder (filter on the file path), selects them, and makes the photo where you left off (library.resumePoint) the active one",
            always,
            show
        ),
        cmd!(
            "library.exportCatalog",
            "Export as Catalog",
            [],
            None,
            "{dest: new folder for the library, folder?: photos under it, subfolders?: bool (default true), ids?: [photoId]} → {dest, photos, albums} — a standalone LightCraft library with these photos (edits, versions, history, metadata, virtual copies + their masters) and the albums they're in; the originals stay where they are (referenced, not copied)",
            always,
            export_catalog
        ),
    ]
}
