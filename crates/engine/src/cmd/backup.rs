//! Catalog backups (Lightroom's Catalog Settings ▸ Backup, File ▸ Back Up Catalog): settings,
//! back up now, list / test / restore backups, test the catalog. See [`crate::backup`].

use std::path::PathBuf;

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::backup::Schedule;
use crate::{Result, Session};

fn on_disk(s: &Session) -> bool {
    s.backup_root().is_some()
}

fn settings_json(s: &Session) -> Value {
    json!({
        "schedule": s.backup.schedule.id(),
        "dir": s.backup_root().map(|d| d.display().to_string()),
        "customDir": s.backup.dir,
        "keep": s.backup.keep,
        "verify": s.backup.verify,
        "optimize": s.backup.optimize,
        "last": s.backup.last,
        "due": s.backup_due(),
        "available": on_disk(s),
    })
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.backupSettings";
    let mut b = s.backup.clone();
    if let Some(v) = p.get("schedule") {
        let id = v.as_str().unwrap_or_default();
        b.schedule = Schedule::parse(id).ok_or_else(|| bad(C, format!("unknown schedule `{id}` (never|everyExit|daily|weekly|monthly|nextExit)")))?;
    }
    match p.get("dir") {
        Some(Value::Null) => b.dir = None,
        Some(Value::String(d)) if d.trim().is_empty() => b.dir = None,
        Some(Value::String(d)) => {
            let path = PathBuf::from(d.trim());
            if !path.is_absolute() {
                return Err(bad(C, "dir must be an absolute folder path"));
            }
            b.dir = Some(d.trim().to_string());
        }
        Some(_) => return Err(bad(C, "dir must be a folder path or null (the library's Backups folder)")),
        None => {}
    }
    if let Some(v) = p.get("keep") {
        b.keep = v.as_u64().ok_or_else(|| bad(C, "keep must be a whole number (0 = keep every backup)"))?.min(1000) as u32;
    }
    if let Some(v) = p.get("verify").and_then(Value::as_bool) {
        b.verify = v;
    }
    if let Some(v) = p.get("optimize").and_then(Value::as_bool) {
        b.optimize = v;
    }
    if b != s.backup {
        s.backup = b;
        s.save_prefs()?;
    }
    Ok(settings_json(s))
}

fn backup(s: &mut Session, p: &Value) -> Result<Value> {
    let verify = p.get("verify").and_then(Value::as_bool).unwrap_or(s.backup.verify);
    let optimize = p.get("optimize").and_then(Value::as_bool).unwrap_or(s.backup.optimize);
    let note = str_param(p, "note").map(str::to_string);
    s.backup_now(note, verify, optimize)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let Some(root) = s.backup_root() else { return Ok(json!({"dir": null, "backups": []})) };
    let backups: Vec<Value> = crate::backup::list(&root)
        .into_iter()
        .map(|(path, m)| {
            json!({
                "path": path.display().to_string(),
                "name": path.file_name().map(|n| n.to_string_lossy().to_string()),
                "created": m.created,
                "photos": m.photos,
                "albums": m.albums,
                "bytes": m.files.iter().map(|f| f.bytes).sum::<u64>(),
                "note": m.note,
            })
        })
        .collect();
    Ok(json!({"dir": root.display().to_string(), "backups": backups}))
}

fn path_param(s: &Session, p: &Value, c: &str) -> Result<PathBuf> {
    let path = str_param(p, "path")
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .ok_or_else(|| bad(c, "missing `path` (a backup folder, see library.backups)"))?;
    let path = PathBuf::from(path);
    // a bare folder name means one in the backup folder
    Ok(if path.is_absolute() { path } else { s.backup_root().map(|r| r.join(&path)).unwrap_or(path) })
}

fn verify(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = path_param(s, p, "library.verifyBackup")?;
    Ok(match crate::backup::verify(&dir) {
        Ok(v) => json!({"ok": true, "path": dir.display().to_string(), "created": v.manifest.created, "photos": v.manifest.photos}),
        Err(e) => json!({"ok": false, "path": dir.display().to_string(), "problem": e}),
    })
}

fn restore(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = path_param(s, p, "library.restoreBackup")?;
    s.restore_backup(&dir)
}

fn integrity(s: &mut Session, _p: &Value) -> Result<Value> {
    s.check_integrity()
}

fn has_library(s: &Session) -> std::result::Result<(), String> {
    if on_disk(s) { Ok(()) } else { Err("the library isn't a folder on disk".into()) }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.backupSettings",
            "Catalog Backup Settings",
            [],
            None,
            "{schedule?: never|everyExit|daily|weekly|monthly|nextExit (asked when quitting), dir?: folder|null (null = Backups in the library), keep?: n (newest n kept, 0 = all), verify?: bool (read each backup back), optimize?: bool (compact the catalog after)} — Lightroom's Catalog Settings ▸ Backup, saved with the library → {schedule, dir, customDir, keep, verify, optimize, last, due, available}",
            always,
            settings
        ),
        cmd!(
            query "library.backup",
            "Back Up Catalog",
            ["Library"],
            None,
            "{verify?: bool, optimize?: bool (default: the backup settings), note?: text} — writes a dated backup of the catalog (from memory, saved or not) and the library's presets/prefs/view into the backup folder, reads it back, keeps the newest `keep` → {path, created, photos, bytes, verified, optimized, removed: [path], ms}",
            has_library,
            backup
        ),
        cmd!(
            query "library.backups",
            "Catalog Backups",
            [],
            None,
            "{} — the backups in the backup folder, newest first → {dir, backups: [{path, name, created, photos, albums, bytes, note}]}",
            always,
            list
        ),
        cmd!(
            query "library.verifyBackup",
            "Test Backup",
            [],
            None,
            "{path: backup folder (or its name in the backup folder)} — checks every file's size and checksum and loads the catalog → {ok, path, created, photos} | {ok: false, problem}",
            always,
            verify
        ),
        cmd!(
            query "library.restoreBackup",
            "Restore Catalog Backup",
            [],
            None,
            "{path: backup folder (or its name)} — replaces the catalog and settings with the backup's after testing it; the current state is backed up first (`safetyBackup`), so restoring that one undoes it → {restored, created, photos, safetyBackup}",
            has_library,
            restore
        ),
        cmd!(
            query "library.checkIntegrity",
            "Test Catalog Integrity",
            ["Library"],
            None,
            "{} — reads the catalog on disk again from scratch and compares it with the one in memory → {ok, photos, albums, unsaved, problems: [text]}",
            has_library,
            integrity
        ),
    ]
}
