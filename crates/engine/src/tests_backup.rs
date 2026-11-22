//! L02: catalog backups — back up, test, keep N, restore (undoable), schedule, catalog test.

use serde_json::json;

use crate::Session;
use crate::backup::{BackupPrefs, Schedule};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-backup-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn session(dir: &std::path::Path) -> Session {
    let mut s = Session::new();
    s.open_library(dir, true).unwrap();
    s
}

/// A clock that moves on a minute per reading (each backup gets its own folder name).
fn ticking(s: &mut Session) {
    let n = std::sync::atomic::AtomicI64::new(0);
    s.clock = Box::new(move || {
        let i = n.fetch_add(60, std::sync::atomic::Ordering::SeqCst);
        lightcraft_catalog::dates::civil(1_795_000_000 + i)
    });
}

#[test]
fn backup_verifies_and_restore_brings_the_catalog_back() {
    let dir = temp_dir("restore");
    let mut s = session(&dir);
    ticking(&mut s);
    let n = s.catalog.len();
    let id = s.selection.active.unwrap();
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    let r = s.execute("library.backup", &json!({})).unwrap();
    assert_eq!(r["photos"], n);
    assert_eq!(r["verified"], true);
    let path = r["path"].as_str().unwrap().to_string();
    assert!(path.starts_with(&dir.join("Backups").display().to_string()), "{path}");
    assert!(std::path::Path::new(&path).join("catalog.snap.gz").exists());
    assert_eq!(s.execute("library.verifyBackup", &json!({"path": path})).unwrap()["ok"], true);

    // later changes …
    s.execute("photo.rate", &json!({"rating": 1})).unwrap();
    s.execute("album.create", &json!({"name": "After", "addSelected": true})).unwrap();
    // … are gone after restoring, and come back by restoring the safety backup
    let r = s.execute("library.restoreBackup", &json!({"path": path})).unwrap();
    assert_eq!(s.catalog.photo(id).unwrap().rating, 4);
    assert!(!s.catalog.albums().any(|a| a.name == "After"));
    let safety = r["safetyBackup"].as_str().unwrap().to_string();
    let list = s.execute("library.backups", &json!({})).unwrap();
    assert_eq!(list["backups"].as_array().unwrap().len(), 2, "{list}");
    assert!(list["backups"][0]["note"].as_str().unwrap().starts_with("before restoring"), "{list}");
    s.execute("library.restoreBackup", &json!({"path": safety})).unwrap();
    assert_eq!(s.catalog.photo(id).unwrap().rating, 1);
    assert!(s.catalog.albums().any(|a| a.name == "After"));
    // the restored library is what's on disk
    s.close_library().unwrap();
    drop(s);
    let s = session(&dir);
    assert_eq!(s.catalog.photo(id).unwrap().rating, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_damaged_backup_is_refused_and_never_restored() {
    let dir = temp_dir("damaged");
    let mut s = session(&dir);
    let path = s.execute("library.backup", &json!({})).unwrap()["path"].as_str().unwrap().to_string();
    let gz = std::path::Path::new(&path).join("catalog.snap.gz");
    let mut bytes = std::fs::read(&gz).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x55;
    std::fs::write(&gz, &bytes).unwrap();
    let v = s.execute("library.verifyBackup", &json!({"path": path})).unwrap();
    assert_eq!(v["ok"], false, "{v}");
    let n = s.catalog.len();
    assert!(s.execute("library.restoreBackup", &json!({"path": path})).is_err());
    assert_eq!(s.catalog.len(), n, "the library is untouched");
    // a manifest pointing outside the backup is refused too
    std::fs::write(std::path::Path::new(&path).join("backup.json"), br#"{"format":"lightcraft-backup","version":1,"created":"x","library":"","photos":0,"albums":0,"seq":0,"files":[{"name":"../catalog.snap","bytes":0,"crc32":0}]}"#).unwrap();
    assert_eq!(s.execute("library.verifyBackup", &json!({"path": path})).unwrap()["ok"], false);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_newest_backups_are_kept_and_other_files_are_left_alone() {
    let dir = temp_dir("keep");
    let mut s = session(&dir);
    ticking(&mut s);
    s.execute("library.backupSettings", &json!({"keep": 2})).unwrap();
    let mut paths = Vec::new();
    for _ in 0..3 {
        paths.push(s.execute("library.backup", &json!({"optimize": false})).unwrap()["path"].as_str().unwrap().to_string());
    }
    // someone's own file in the oldest backup's folder survives the pruning (the folder stays)
    let list = s.execute("library.backups", &json!({})).unwrap();
    assert_eq!(list["backups"].as_array().unwrap().len(), 2, "{list}");
    assert!(!std::path::Path::new(&paths[0]).exists());
    std::fs::write(std::path::Path::new(&paths[1]).join("mine.txt"), b"keep me").unwrap();
    s.execute("library.backup", &json!({})).unwrap();
    assert!(std::path::Path::new(&paths[1]).join("mine.txt").exists());
    assert!(!std::path::Path::new(&paths[1]).join("catalog.snap.gz").exists());
    assert_eq!(s.execute("library.backups", &json!({})).unwrap()["backups"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn schedule_settings_persist_and_say_when_a_backup_is_due() {
    let dir = temp_dir("schedule");
    let mut s = session(&dir);
    let r = s.execute("library.backupSettings", &json!({})).unwrap();
    assert_eq!((r["schedule"].as_str(), r["due"].as_bool()), (Some("weekly"), Some(true)), "never backed up: due");
    assert!(s.execute("library.backupSettings", &json!({"schedule": "hourly"})).is_err());
    assert!(s.execute("library.backupSettings", &json!({"dir": "relative/path"})).is_err());
    s.execute("library.backupSettings", &json!({"schedule": "daily", "keep": 5})).unwrap();
    s.execute("library.backup", &json!({})).unwrap();
    assert!(!s.backup_due(), "just backed up");
    s.close_library().unwrap();
    drop(s);
    let s = session(&dir);
    assert_eq!((s.backup.schedule, s.backup.keep), (Schedule::Daily, 5));
    assert!(s.backup.last.is_some());

    let mut p = BackupPrefs { schedule: Schedule::Weekly, last: Some("2026-11-01T10:00:00".into()), ..Default::default() };
    assert!(!p.due("2026-11-07T10:00:00") && p.due("2026-11-08T10:00:00"));
    p.schedule = Schedule::Never;
    assert!(!p.due("2030-01-01T00:00:00"));
    p.schedule = Schedule::EveryExit;
    assert!(p.due("2026-11-01T10:00:01"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn next_exit_goes_back_to_weekly_and_in_memory_libraries_have_no_backups() {
    let dir = temp_dir("next");
    let mut s = session(&dir);
    s.execute("library.backupSettings", &json!({"schedule": "nextExit"})).unwrap();
    s.execute("library.backup", &json!({})).unwrap();
    assert_eq!(s.backup.schedule, Schedule::Weekly);
    let _ = std::fs::remove_dir_all(&dir);

    let mut m = Session::with_demo();
    assert!(!m.backup_due());
    assert!(m.execute("library.backup", &json!({})).is_err());
    assert_eq!(m.execute("library.backups", &json!({})).unwrap()["backups"], json!([]));
}

#[test]
fn the_catalog_test_finds_a_damaged_log() {
    let dir = temp_dir("integrity");
    let mut s = session(&dir);
    s.execute("photo.rate", &json!({"rating": 3})).unwrap();
    let r = s.execute("library.checkIntegrity", &json!({})).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    // a torn record at the end of the log
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(dir.join("catalog.log")).unwrap();
    f.write_all(b"{\"seq\":99999,\"crc\":1,\"op\":{").unwrap();
    drop(f);
    let r = s.execute("library.checkIntegrity", &json!({})).unwrap();
    assert_eq!(r["ok"], false, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gzip_round_trips_and_rejects_damage() {
    let data: Vec<u8> = (0..100_000u32).flat_map(|i| (i % 251).to_le_bytes()).collect();
    let gz = crate::backup::gzip(&data, 3);
    assert_eq!(crate::backup::gunzip(&gz).unwrap(), data);
    assert!(crate::backup::gunzip(&gz[..gz.len() - 3]).is_err());
    assert!(crate::backup::gunzip(b"").is_err());
    assert!(crate::backup::gunzip(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1, 2]).is_err());
    assert_eq!(crate::backup::folder_name("2026-11-21T15:30:12"), "2026-11-21 153012");
}
