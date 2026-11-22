//! Migrate from Lightroom Classic (`library.migrateLightroom`) and the resume point it feeds
//! (`library.resumePoint`). The work is in [`crate::lr_migrate`].

use serde_json::Value;

use super::{CommandSpec, always, bad, bool_or, cmd, has_active, str_param};
use crate::lr_history::{ExtrasOptions, history_of, migrate_extras};
use crate::lr_migrate::{
    MigrateOptions, Records, default_catalog, default_preset_dirs, dump_catalog, flatten_wrapper, migrate, read_records, records_from_db,
    rematch_lens_profiles, rematch_profiles, repair_migration, resume_point, with_catalog_copy,
};
use crate::{Result, Session};

const C: &str = "library.migrateLightroom";

/// The records a `library.migrateLightroom` call names: a records file, or a catalog (read now).
pub fn load_records(p: &Value) -> std::result::Result<Records, String> {
    if let Some(r) = str_param(p, "records").filter(|r| !r.trim().is_empty()) {
        return read_records(r);
    }
    let catalog = match str_param(p, "catalog").filter(|c| !c.trim().is_empty()) {
        Some(c) => std::path::PathBuf::from(c),
        None => default_catalog().ok_or("no catalog given and none in ~/Pictures/Lightroom")?,
    };
    let scratch = str_param(p, "scratch").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dump_catalog(&catalog, &scratch)
}

/// The catalog a call names (`catalog`, default the newest in ~/Pictures/Lightroom) and the
/// scratch folder its copy goes in.
fn catalog_and_scratch(p: &Value) -> std::result::Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let catalog = match str_param(p, "catalog").filter(|c| !c.trim().is_empty()) {
        Some(c) => std::path::PathBuf::from(c),
        None => default_catalog().ok_or("no catalog given and none in ~/Pictures/Lightroom")?,
    };
    Ok((catalog, str_param(p, "scratch").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir)))
}

/// Run `f` with a copy of the catalog a call names and its records (see [`with_catalog_copy`]).
pub fn with_catalog(p: &Value, f: impl FnOnce(&std::path::Path, Records) -> Result<Value>) -> std::result::Result<Result<Value>, String> {
    let (catalog, scratch) = catalog_and_scratch(p)?;
    let mut out = None;
    with_catalog_copy(&catalog, &scratch, |db| {
        let rec = records_from_db(db, &catalog.display().to_string())?;
        out = Some(f(db, rec));
        Ok(())
    })?;
    out.ok_or_else(|| "nothing read".to_string())
}

fn extras_options(p: &Value) -> ExtrasOptions {
    ExtrasOptions { history: bool_or(p, "history", true), snapshots: bool_or(p, "snapshots", true), dry_run: bool_or(p, "dryRun", false), only: None }
}

/// The preset folders a call asks for: `presetDirs`, else (unless `presets: false`) the default
/// Lightroom folders next to the catalog and in Application Support.
pub fn preset_dirs(p: &Value, catalog: &str) -> Vec<String> {
    if !bool_or(p, "presets", true) {
        return Vec::new();
    }
    match p.get("presetDirs").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => default_preset_dirs(Some(std::path::Path::new(catalog))),
    }
}

/// The profile folders a call asks for: `profileDirs`, else (unless `profiles` is false; it
/// defaults to `default_on`) the default Lightroom / Camera Raw folders (as for presets).
pub fn profile_dirs(p: &Value, catalog: Option<&str>, default_on: bool) -> Vec<String> {
    if p.get("profileDirs").is_none() && !bool_or(p, "profiles", default_on) {
        return Vec::new();
    }
    match p.get("profileDirs").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => default_preset_dirs(catalog.map(std::path::Path::new)),
    }
}

fn look_map(p: &Value) -> std::collections::HashMap<String, String> {
    p.get("lookMap")
        .and_then(Value::as_object)
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

fn rematch(s: &mut Session, p: &Value) -> Result<Value> {
    const R: &str = "library.rematchProfiles";
    // the catalog is read only when asked for (the migration's record covers migrated photos)
    let rec = if str_param(p, "catalog").is_some() || str_param(p, "records").is_some() || bool_or(p, "readCatalog", false) {
        Some(load_records(p).map_err(|e| bad(R, e))?)
    } else {
        None
    };
    let catalog = rec.as_ref().map(|r| r.catalog.clone()).or_else(|| default_catalog().map(|c| c.display().to_string()));
    let opts = MigrateOptions {
        dry_run: bool_or(p, "dryRun", false),
        look_map: look_map(p),
        profile_dirs: profile_dirs(p, catalog.as_deref(), true),
        ..Default::default()
    };
    rematch_profiles(s, rec.as_ref(), &opts, bool_or(p, "force", false))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let one = |s: &mut Session, rec: Records, db: Option<&std::path::Path>| -> Result<Value> {
        if let Some(out) = str_param(p, "recordsOut").filter(|o| !o.trim().is_empty()) {
            let bytes = serde_json::to_vec(&rec).map_err(|e| bad(C, e.to_string()))?;
            std::fs::write(out, bytes).map_err(|e| bad(C, format!("{out}: {e}")))?;
        }
        let opts = MigrateOptions {
            import: bool_or(p, "import", true),
            limit: p.get("limit").and_then(Value::as_u64).map(|l| l.min(usize::MAX as u64) as usize),
            dry_run: bool_or(p, "dryRun", false),
            develop_all: bool_or(p, "developAll", false),
            collections_only: bool_or(p, "collectionsOnly", false),
            only_new: bool_or(p, "onlyNew", false),
            preset_dirs: preset_dirs(p, &rec.catalog),
            look_map: look_map(p),
            profile_dirs: profile_dirs(p, Some(&rec.catalog), bool_or(p, "presets", true)),
        };
        let mut r = migrate(s, &rec, &opts)?;
        // Lightroom's history and snapshots (read from the same catalog copy)
        let eo = extras_options(p);
        if let Some(db) = db
            && !opts.collections_only
            && (eo.history || eo.snapshots)
        {
            r["lightroomHistory"] = migrate_extras(s, db, &rec, &eo)?;
        }
        Ok(r)
    };
    if let Some(r) = str_param(p, "records").filter(|r| !r.trim().is_empty()) {
        return one(s, read_records(r).map_err(|e| bad(C, e))?, None);
    }
    with_catalog(p, |db, rec| one(s, rec, Some(db))).map_err(|e| bad(C, e))?
}

/// `history.lightroom`: the active (or `id`) photo's Lightroom history steps, oldest first.
fn lightroom_history(s: &mut Session, p: &Value) -> Result<Value> {
    let id = photo_param(s, p, "history.lightroom")?;
    Ok(match history_of(s, id) {
        Some(h) => serde_json::json!({
            "photoId": id.0,
            "catalog": h.catalog,
            "steps": h.steps.iter().enumerate().map(|(i, st)| serde_json::json!({"index": i, "label": st.label, "at": st.at})).collect::<Vec<_>>(),
        }),
        None => serde_json::json!({"photoId": id.0, "steps": []}),
    })
}

fn photo_param(s: &Session, p: &Value, c: &str) -> Result<lightcraft_catalog::PhotoId> {
    let id = match p.get("id").and_then(Value::as_u64) {
        Some(i) => lightcraft_catalog::PhotoId(i),
        None => s.active().ok_or_else(|| bad(c, "no active photo"))?,
    };
    s.catalog.photo(id).map(|_| id).ok_or_else(|| bad(c, format!("no photo {}", id.0)))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.migrateLightroom",
            "Migrate from Lightroom Classic…",
            ["File"],
            None,
            "{catalog?: path.lrcat (default: the newest in ~/Pictures/Lightroom; read from a copy with the system sqlite3, the catalog itself is never opened) | records?: records JSON (from recordsOut), recordsOut?: path, import?: bool (default true: add the files in place; false when they were imported already), limit?: N (only the first N photos whose files exist), dryRun?: bool, developAll?: bool (also photos never edited in Lightroom), collectionsOnly?: bool (only the collections, for a library migrated already: nothing imported, no photo changed, no presets), onlyNew?: bool (only the photos this run imports — e.g. files an earlier run failed on — photos already in the library are left as they are), lookMap?: {lookName: profileId} (creative looks → our profiles; an imported profile named like the look matches by itself), profiles?: bool (default: as presets; first import the creative profiles — XMP Look profiles with a colour table such as Summer Fields / Nautica, presets carrying one, .cube files — from profileDirs?: [folders] (default: the preset folders below) so photos using them match), presets?: bool (default true), presetDirs?: [folders] (default: Lightroom Settings/{Settings, Develop Presets, Keyword Sets} next to the catalog + Camera Raw / Lightroom preset folders), history?: bool (default true, with a catalog: each photo's Lightroom history steps, kept in <library>/Lightroom History/), snapshots?: bool (default true, with a catalog: Lightroom snapshots → versions)} → {found, missing, missingByRoot, matched, rated, flagged, labelled, keyworded, virtualCopies, develop: {applied, unmapped: [{key, photos}], photosWithUnmapped…}, albums: {albums, smart, sets, quick, skipped}, presets, lightroomHistory: {history: {photos, steps…}, snapshots: {added…}}} — ratings, flags, colour labels, keywords, caption/copyright/creator/location, collections (the catalog's own tree: sets as album folders, smart collections whose rules map, the Quick Collection into ours), virtual copies, develop settings, history, snapshots and Lightroom's edit times; one undo step (+ one for the snapshots)",
            always,
            run
        ),
        cmd!(
            "library.migrateLightroomHistory",
            "Migrate Lightroom History and Snapshots",
            [],
            None,
            "{catalog?: path.lrcat (default: the newest in ~/Pictures/Lightroom; read from a copy), history?: bool (default true), snapshots?: bool (default true), dryRun?: bool} — for a library migrated already: each photo's Lightroom history steps (shown under History, kept in <library>/Lightroom History/, rewritten each run) and its snapshots as versions (one undo step; a version already there isn't added twice) → {history: {photos, steps, unreadable, unmapped, notInLibrary, failedWrites}, snapshots: {added, alreadyThere, notInLibrary, unmapped}}",
            always,
            |s, p| {
                let c = "library.migrateLightroomHistory";
                let eo = extras_options(p);
                with_catalog(p, |db, rec| migrate_extras(s, db, &rec, &eo)).map_err(|e| bad(c, e))?
            }
        ),
        cmd!(
            query "history.lightroom",
            "Lightroom History",
            [],
            None,
            "{id?: photo (default the active one)} → {photoId, catalog?, steps: [{index, label, at}]} — the history the photo had in Lightroom Classic (migrated with it), oldest first",
            always,
            lightroom_history
        ),
        cmd!(
            "history.restoreLightroom",
            "Go to Lightroom History Step",
            [],
            None,
            "{index, id?: photo (default the active one)} — set the photo to its settings after that Lightroom history step (a History step here, undoable)",
            has_active,
            |s, p| {
                let c = "history.restoreLightroom";
                let id = photo_param(s, p, c)?;
                let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(c, "give `index`"))? as usize;
                let h = history_of(s, id).ok_or_else(|| bad(c, "this photo has no Lightroom history"))?;
                let st = h.steps.get(i).ok_or_else(|| bad(c, format!("no Lightroom step {i}")))?;
                let d = h.settings(i).ok_or_else(|| bad(c, format!("Lightroom step {i} can't be read")))?;
                s.set_develop(id, d, &format!("Lightroom: {}", st.label))?;
                Ok(serde_json::json!({"photoId": id.0, "label": st.label}))
            }
        ),
        cmd!(
            "library.rematchProfiles",
            "Match Lightroom Profiles",
            [],
            None,
            "{profileDirs?: [folders/files] (default: Camera Raw / Lightroom profile and preset folders), profiles?: bool (default true: import those first), catalog?: path.lrcat | records?: path | readCatalog?: bool (also read the catalog, default the newest, for photos the migration didn't record), lookMap?: {lookName: profileId}, force?: bool (also photos whose profile was changed since), dryRun?: bool} — give migrated photos the creative profile their Lightroom look names (Summer Fields, Nautica…), once it is imported; one undo step → {matched, byLook, keptOwnProfile, unmatched: {look: photos}, profiles}",
            always,
            rematch
        ),
        cmd!(
            "library.rematchLensProfiles",
            "Match Lightroom Lens Profiles",
            [],
            None,
            "{catalog?: path.lrcat (default: the newest in ~/Pictures/Lightroom; read from a copy) | records?: path, force?: bool (also photos given another lens profile here since), dryRun?: bool} — give library photos the lens profile Lightroom chose for them (by name, file and digest: e.g. a fisheye or a manual lens the camera didn't record), with Lightroom's Enable Profile Corrections and amounts; rendered from the installed Adobe .lcp; one undo step → {changed, byProfile, notInstalled, alreadySet, keptOwnChoice, notInLibrary}",
            always,
            |s, p| {
                let rec = load_records(p).map_err(|e| bad("library.rematchLensProfiles", e))?;
                rematch_lens_profiles(s, &rec, bool_or(p, "dryRun", false), bool_or(p, "force", false))
            }
        ),
        cmd!(
            "library.repairLightroomMigration",
            "Repair Lightroom Migration",
            [],
            None,
            "{catalog?: path.lrcat (default: the newest in ~/Pictures/Lightroom) | records?: path, crops?: bool (default true: straighten angles an earlier version mirrored), unedited?: bool (default true: photos Lightroom shows unedited that got settings from an XMP sidecar at import get Lightroom's), upright?: bool (default true: Lightroom's Upright mode — earlier versions read Level as Vertical — with the transform Lightroom stored instead of our own detection, and Constrain Crop), dryRun?: bool} — repair a library migrated by an earlier version; photos changed here since the migration are left alone; one undo step → {cropsFixed, uneditedReset, uprightFixed, keptChangedHere}",
            always,
            |s, p| {
                let rec = load_records(p).map_err(|e| bad("library.repairLightroomMigration", e))?;
                repair_migration(
                    s,
                    &rec,
                    bool_or(p, "crops", true),
                    bool_or(p, "unedited", true),
                    bool_or(p, "upright", true),
                    bool_or(p, "dryRun", false),
                )
            }
        ),
        cmd!(
            "library.flattenLightroomCollections",
            "Flatten Lightroom Collections",
            [],
            None,
            "{any?: bool} — undo what earlier Lightroom migrations did: the top-level \"From Lightroom\" album folder that wraps a second \"From Lightroom\" folder (with `any`: any top-level \"From Lightroom\" folder) is removed and everything in it moves to the top level, as the collections were in Lightroom (an album whose twin is already there gives it its photos); an album called \"quick collection\" joins the Quick Collection. One undo step → {flattened, moved, merged, quick}",
            always,
            |s, p| flatten_wrapper(s, bool_or(p, "any", false))
        ),
        cmd!(
            query "library.resumePoint",
            "Where I Left Off",
            [],
            None,
            "{folder?: path, subfolders?: bool (default true), album?: albumId} (neither = the whole library) → {photoId, at, source: edit | lightroomEdit | lightroomTouch} | null — the photo last edited here, or (from a Lightroom migration) last edited / touched in Lightroom Classic",
            always,
            |s, p| {
                let album = p.get("album").and_then(Value::as_u64).map(lightcraft_catalog::AlbumId);
                if let Some(a) = album
                    && s.catalog.album(a).is_none()
                {
                    return Err(bad("library.resumePoint", format!("no album {}", a.0)));
                }
                Ok(resume_point(s, str_param(p, "folder"), album, bool_or(p, "subfolders", true)))
            }
        ),
    ]
}
