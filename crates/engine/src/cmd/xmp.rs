//! XMP sidecar commands: save/read metadata to/from files, sidecar preferences.

use lightcraft_catalog::Op;
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, has_selection, str_param};
use crate::sidecar::SidecarNaming;
use crate::{Result, Session};

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let mut written = Vec::new();
    let mut merged = Vec::new();
    let mut backups = Vec::new();
    let mut failed = Vec::new();
    let owners = crate::sidecar::StemOwners::of(&s.catalog);
    let mut synced = Vec::new();
    for id in s.targets(p) {
        match s.save_sidecar_with(id, &owners) {
            Ok(r) => {
                synced.push((id, r.sync));
                let path = r.path.display().to_string();
                if r.merged {
                    merged.push(path.clone());
                }
                if let Some(b) = r.backup {
                    backups.push(json!({"path": path, "backup": b.display().to_string()}));
                }
                written.push(path);
            }
            Err(e) => failed.push(json!({"id": id.0, "error": e.to_string()})),
        }
    }
    s.record_xmp_sync(synced);
    Ok(json!({"written": written, "merged": merged, "backups": backups, "failed": failed}))
}

fn read(s: &mut Session, p: &Value) -> Result<Value> {
    let mut ops = Vec::new();
    let mut read = Vec::new();
    let mut failed = Vec::new();
    for id in s.targets(p) {
        match s.read_sidecar_op(id) {
            Ok(Some((op, from))) => {
                ops.push(op);
                read.push(json!({"id": id.0, "from": from.display().to_string()}));
            }
            Ok(None) => failed.push(json!({"id": id.0, "error": "no XMP sidecar or embedded XMP"})),
            Err(e) => failed.push(json!({"id": id.0, "error": e.to_string()})),
        }
    }
    if !ops.is_empty() {
        s.commit("Read Metadata from File", Op::Batch { ops })?;
    }
    // L22: catalog and file match now
    let synced: Vec<_> = read
        .iter()
        .filter_map(|r| r["id"].as_u64())
        .filter_map(|i| s.current_sync(lightcraft_catalog::PhotoId(i)).map(|y| (lightcraft_catalog::PhotoId(i), y)))
        .collect();
    s.record_xmp_sync(synced);
    Ok(json!({"read": read, "failed": failed}))
}

fn prefs(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(a) = p.get("autoWrite").and_then(Value::as_bool) {
        s.xmp.auto_write = a;
    }
    if let Some(n) = str_param(p, "naming") {
        s.xmp.naming = match n {
            "stem" => SidecarNaming::Stem,
            "full" => SidecarNaming::Full,
            other => return Err(bad("library.xmpPreferences", format!("unknown naming `{other}` (stem|full)"))),
        };
    }
    if p.as_object().is_some_and(|o| !o.is_empty()) {
        s.save_prefs()?;
    }
    Ok(serde_json::to_value(s.xmp).unwrap_or_default())
}

/// The photos a scan looks at: the library's own files (not Local records, Recently Deleted or
/// virtual copies).
fn scan_targets(s: &Session) -> Vec<lightcraft_catalog::PhotoId> {
    s.catalog
        .photos()
        .filter(|p| !p.deleted && !p.local && p.copy_of.is_none() && matches!(p.source, lightcraft_catalog::Source::File { .. }))
        .map(|p| p.id)
        .collect()
}

fn scan(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = if p.get("ids").is_some() { s.targets(p) } else { scan_targets(s) };
    let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
    let (mut on_disk, mut conflicts, mut changed) = (Vec::new(), Vec::new(), Vec::new());
    for id in ids {
        let st = s.metadata_status(id);
        *counts.entry(st.id()).or_default() += 1;
        match st {
            crate::sidecar::MetadataStatus::ChangedOnDisk => on_disk.push(id.0),
            crate::sidecar::MetadataStatus::Conflict => conflicts.push(id.0),
            crate::sidecar::MetadataStatus::Changed => changed.push(id.0),
            _ => {}
        }
        s.metadata_status.insert(id, st);
    }
    Ok(json!({"counts": counts, "changedOnDisk": on_disk, "conflict": conflicts, "changed": changed}))
}

fn resolve(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "photo.resolveMetadata";
    let ids: Vec<u64> = s.targets(p).iter().map(|i| i.0).collect();
    match str_param(p, "keep") {
        Some("disk") => s.execute("photo.readMetadataFromFile", &json!({"ids": ids})),
        Some("library") => s.execute("photo.saveMetadataToFile", &json!({"ids": ids})),
        _ => Err(bad(C, "`keep` is disk (Import Settings from Disk) or library (Overwrite Settings)")),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "photo.metadataStatus",
            "Metadata Status",
            [],
            None,
            "{ids?} → [{id, status: upToDate | changed (needs saving) | changedOnDisk | conflict | unknown | notWritten | none}] (reads each sidecar)",
            has_selection,
            |s, p| {
                let v: Vec<Value> = s.targets(p).iter().map(|id| json!({"id": id.0, "status": s.metadata_status(*id).id()})).collect();
                Ok(json!(v))
            }
        ),
        cmd!(
            "library.scanMetadata",
            "Scan for Metadata Updates",
            ["Library"],
            None,
            "{ids?} — compares each photo's metadata with its XMP sidecar (the grid shows ↑ needs saving, ↓ changed on disk, ! conflict) → {counts, changedOnDisk, conflict, changed}",
            always,
            scan
        ),
        cmd!(
            "photo.resolveMetadata",
            "Resolve Metadata Conflict",
            [],
            None,
            "{ids?, keep: disk | library} — disk: Import Settings from Disk (read the sidecar); library: Overwrite Settings (write it)",
            has_selection,
            resolve
        ),
        cmd!(
            "photo.saveMetadataToFile",
            "Save Metadata to File",
            ["Photo"],
            Some("Cmd+S"),
            "{ids?} — writes each photo's XMP sidecar (metadata + develop settings) next to the original, merged into an existing one (other apps' data kept; an unreadable one is backed up first) → {written: [path], merged: [path], backups: [{path, backup}], failed}",
            has_selection,
            save
        ),
        cmd!(
            "photo.readMetadataFromFile",
            "Read Metadata from File",
            ["Photo"],
            None,
            "{ids?} — reads each photo's XMP sidecar (or a raw/DNG's embedded XMP): metadata and develop settings, one undo step → {read, failed}",
            has_selection,
            read
        ),
        cmd!(
            "library.xmpPreferences",
            "XMP Sidecar Preferences",
            [],
            None,
            "{autoWrite?: bool, naming?: stem (IMG_1.xmp, default) | full (IMG_1.CR3.xmp)} → current preferences (saved with the library)",
            always,
            prefs
        ),
        cmd!("library.toggleAutoWriteXmp", "Automatically Write Changes into XMP", ["File"], None, "{} → {autoWrite}", always, |s, _| {
            let on = !s.xmp.auto_write;
            prefs(s, &json!({"autoWrite": on}))
        }),
    ]
}
