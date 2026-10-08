//! Migrate from Lightroom Classic (`library.migrateLightroom`) and the resume point it feeds
//! (`library.resumePoint`). The work is in [`crate::lr_migrate`].

use serde_json::Value;

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::lr_migrate::{
    MigrateOptions, Records, default_catalog, default_preset_dirs, dump_catalog, flatten_wrapper, migrate, read_records, resume_point,
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

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let rec = load_records(p).map_err(|e| bad(C, e))?;
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
        look_map: p
            .get("lookMap")
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
    };
    migrate(s, &rec, &opts)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.migrateLightroom",
            "Migrate from Lightroom Classic…",
            ["File"],
            None,
            "{catalog?: path.lrcat (default: the newest in ~/Pictures/Lightroom; read from a copy with the system sqlite3, the catalog itself is never opened) | records?: records JSON (from recordsOut), recordsOut?: path, import?: bool (default true: add the files in place; false when they were imported already), limit?: N (only the first N photos whose files exist), dryRun?: bool, developAll?: bool (also photos never edited in Lightroom), collectionsOnly?: bool (only the collections, for a library migrated already: nothing imported, no photo changed, no presets), onlyNew?: bool (only the photos this run imports — e.g. files an earlier run failed on — photos already in the library are left as they are), lookMap?: {lookName: profileId} (creative looks → our profiles; an imported .cube profile named like the look matches by itself), presets?: bool (default true), presetDirs?: [folders] (default: Lightroom Settings/{Settings, Develop Presets, Keyword Sets} next to the catalog + Camera Raw / Lightroom preset folders)} → {found, missing, missingByRoot, matched, rated, flagged, labelled, keyworded, virtualCopies, develop: {applied, unmapped: [{key, photos}], photosWithUnmapped…}, albums: {albums, smart, sets, quick, skipped}, presets} — ratings, flags, colour labels, keywords, caption/copyright/creator/location, collections (the catalog's own tree: sets as album folders, smart collections whose rules map, the Quick Collection into ours), virtual copies, develop settings and Lightroom's edit times; one undo step",
            always,
            run
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
