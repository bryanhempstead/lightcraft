//! L02 catalog backups and L03 Delete from Disk / Remove, headless: the bkup. settings tab backs
//! up, the backup list restores, quitting asks when a backup is due; ⌫ asks Lightroom's question
//! and each answer does what it says; ⌘⌫ deletes the rejects.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes screenshots there.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(tag: &str) -> TempDir {
    let d = std::env::temp_dir().join(format!("lc-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    TempDir(d)
}

fn headless(session: lightcraft_engine::Session) -> Headless {
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = Services {
        png: Some(Box::new(png)),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1300.0, 900.0], 1.0);
    h.step();
    h.step();
    h
}

fn click(h: &mut Headless, id: &str) {
    h.step();
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    for _ in 0..4 {
        h.step();
    }
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        for _ in 0..3 {
            h.step();
        }
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

#[test]
fn backups_from_settings_restore_from_the_list_and_quitting_asks() {
    let dir = temp("backup");
    let lib = dir.0.join("lib");
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(&lib, true).unwrap();
    let mut h = headless(s);
    let id = h.app.session.active().unwrap();
    h.app.session.execute("photo.rate", &json!({"ids": [id.0], "rating": 5})).unwrap();

    h.app.run("app.settings", json!({"tab": "backup"})).unwrap();
    click(&mut h, "button:settingsBackupWhen-2");
    assert_eq!(h.app.session.backup.schedule, lightcraft_engine::backup::Schedule::Daily);
    click(&mut h, "button:settingsBackupNow");
    assert!(h.app.session.backup.last.is_some(), "backed up");
    shot(&mut h, "backup-settings.png");

    // a later change, then File ▸ Restore Library from Backup… ▸ the backup ▸ restore.
    h.app.session.execute("photo.rate", &json!({"ids": [id.0], "rating": 1})).unwrap();
    h.app.ui.dialog = None;
    h.app.run("file.restoreLibrary", json!({})).unwrap();
    assert!(matches!(h.app.ui.dialog, Some(Dialog::Backups { pick: None })));
    click(&mut h, "button:backup-0");
    assert!(matches!(&h.app.ui.dialog, Some(Dialog::Backups { pick: Some(_) })));
    shot(&mut h, "backup-list.png");
    click(&mut h, "button:dialogOk");
    assert_eq!(h.app.session.catalog.photo(id).unwrap().rating, 5, "restored");
    assert!(h.app.ui.dialog.is_none());

    // quitting while a backup is due asks; skip. lets it close
    h.app.session.execute("library.backupSettings", &json!({"schedule": "everyExit"})).unwrap();
    h.app.headless_host = false;
    assert!(!crate::panels::notices::may_close(&mut h.app));
    assert!(h.app.backup_prompt);
    shot(&mut h, "backup-quit.png");
    click(&mut h, "button:backupSkip");
    assert!(h.app.backup_answered && crate::panels::notices::may_close(&mut h.app));
}

fn import_pngs(dir: &std::path::Path, n: u8) -> lightcraft_engine::Session {
    for i in 0..n {
        let (w, h) = (24usize, 16usize);
        let data: Vec<[u8; 4]> = (0..w * h).map(|k| [(k % w * 5) as u8, (k / w * 7) as u8, i * 40, 255]).collect();
        let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join(format!("src/p{i}.png")), bytes).unwrap();
    }
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.trash_dir = Some(dir.join("trash"));
    s.execute("library.import", &json!({"paths": [dir.join("src").to_string_lossy()]})).unwrap();
    s
}

#[test]
fn delete_asks_delete_from_disk_or_remove() {
    let dir = temp("delete");
    let mut h = headless(import_pngs(&dir.0, 4));
    let ids: Vec<_> = h.app.session.visible_cloned();
    assert_eq!(ids.len(), 4);
    let file = |h: &Headless, id| match &h.app.session.catalog.photo(id).unwrap().source {
        lightcraft_engine::catalog::Source::File { path } => std::path::PathBuf::from(path),
        _ => std::path::PathBuf::new(),
    };

    // ⌫ → the question; remove. keeps the file
    h.app.session.selection = lightcraft_engine::Selection::single(ids[0]);
    let kept = file(&h, ids[0]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, Some(Dialog::DeletePhotos { count: 1, copies: 0, rejected: false }));
    shot(&mut h, "delete-question.png");
    click(&mut h, "button:dialogRemove");
    assert!(h.app.session.catalog.photo(ids[0]).is_none() && kept.exists());

    // delete.from.disk. sends the file to the Trash
    h.app.session.selection = lightcraft_engine::Selection::single(ids[1]);
    let gone = file(&h, ids[1]);
    h.request("ui.key", json!({"key": "delete"}), T);
    click(&mut h, "button:dialogOk");
    assert!(h.app.session.catalog.photo(ids[1]).is_none());
    assert!(!gone.exists() && dir.0.join("trash").join(gone.file_name().unwrap()).exists());

    // ⌘⌫: Delete Rejected Photos
    h.app.session.execute("photo.flag", &json!({"ids": [ids[3].0], "flag": "reject"})).unwrap();
    h.request("ui.key", json!({"key": "delete", "cmd": true}), T);
    assert_eq!(h.app.ui.dialog, Some(Dialog::DeletePhotos { count: 1, copies: 0, rejected: true }));
    shot(&mut h, "delete-rejected.png");
    click(&mut h, "button:dialogOk");
    assert!(h.app.session.catalog.photo(ids[3]).is_none());
    assert_eq!(h.app.session.catalog.len(), 1);
}

/// L07 + L06: the filter bar's columns. toggle shows the metadata column browser; clicking a value
/// narrows the grid, ⌘-click adds a second one; No Label and Copy Status are in the bar.
#[test]
fn column_browser_narrows_the_grid() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids = s.visible_cloned();
    for (i, id) in ids.iter().enumerate() {
        let label = ["red", "blue", "none"][i % 3];
        s.execute("photo.label", &json!({"ids": [id.0], "label": label})).unwrap();
    }
    let n = ids.len();
    let mut h = headless(s);
    h.app.run("view.library", json!({})).ok();
    h.app.ui.view = crate::state::ViewMode::PhotoGrid;
    h.app.ui.filter_bar = true;
    click(&mut h, "button:filterColumns");
    assert!(h.app.ui.column_browser);
    // the fourth column is Label
    click(&mut h, "button:column-3-0");
    let one = h.app.session.visible_cloned().len();
    assert!(one < n && one > 0, "{one} of {n}");
    shot(&mut h, "column-browser.png");
    // ⌘-click a second camera: both
    h.step();
    let id = "button:column-3-1";
    let rect = h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap();
    let c = rect.center();
    let r = h.request("ui.click", json!({"x": c.x, "y": c.y, "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    for _ in 0..4 {
        h.step();
    }
    let two = h.app.session.visible_cloned().len();
    assert!(two > one, "{one} -> {two}");
    assert!(h.app.widgets.iter().any(|(w, _)| w == "filter:label-none"));
    assert!(h.app.widgets.iter().any(|(w, _)| w.contains("copies")), "Copy Status picker");
}

/// I01: import presets save the Import window's choices and apply them (the source stays).
#[test]
fn import_presets_save_and_apply() {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    h.app.ui.import_prefs.mode = "copy".into();
    h.app.ui.import_prefs.rename = "{date}-{seq:3}".into();
    h.app.ui.import_prefs.source = "/Volumes/CARD".into();
    h.app.run("import.savePreset", json!({"name": "Wedding card"})).unwrap();
    h.app.ui.import_prefs.mode = "add".into();
    h.app.ui.import_prefs.rename.clear();
    h.app.ui.import_prefs.source = "/Volumes/OTHER".into();
    h.app.run("import.applyPreset", json!({"name": "wedding card"})).unwrap();
    assert_eq!((h.app.ui.import_prefs.mode.as_str(), h.app.ui.import_prefs.rename.as_str()), ("copy", "{date}-{seq:3}"));
    assert_eq!(h.app.ui.import_prefs.source, "/Volumes/OTHER", "the source stays");
    assert_eq!(h.app.run("import.presets", json!({})).unwrap(), json!(["Wedding card"]));
    assert!(h.app.run("import.savePreset", json!({"name": " "})).is_err());
    h.app.run("import.deletePreset", json!({"name": "Wedding card"})).unwrap();
    assert!(h.app.run("import.applyPreset", json!({"name": "Wedding card"})).is_err());
    // the Import window shows the Import Preset popup in its bottom bar
    h.app.run("file.addPhotos", json!({})).ok();
    for _ in 0..4 {
        h.step();
    }
    if crate::import_window::has_window() {
        assert!(h.app.widgets.iter().any(|(w, _)| w == "button:importPresets"));
    }
}

/// L12: the Painter paints labels, ratings and albums onto grid photos; a label comes off again.
#[test]
fn painter_modes() {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    h.app.ui.view = crate::state::ViewMode::PhotoGrid;
    let target = h.app.session.visible_cloned()[0];
    h.app.session.execute("photo.label", &json!({"ids": [target.0], "label": "none"})).unwrap();
    let paint_click = |h: &mut Headless| {
        for _ in 0..3 {
            h.step();
        }
        let rect = h.app.widgets.iter().find(|(w, _)| *w == format!("thumb:{}", target.0)).map(|(_, r)| *r).unwrap();
        let c = rect.center();
        let r = h.request("ui.drag", json!({"x": c.x, "y": c.y, "toX": c.x, "toY": c.y, "steps": 2}), T);
        assert_eq!(r["ok"], true, "{r}");
        for _ in 0..3 {
            h.step();
        }
    };
    let r = h.app.run("tool.painter", json!({"mode": "label", "value": "red"})).unwrap();
    assert_eq!(r["painter"]["mode"], "label");
    paint_click(&mut h);
    assert_eq!(h.app.session.catalog.photo(target).unwrap().label, Some(lightcraft_catalog::ColorLabel::Red));
    paint_click(&mut h);
    assert_eq!(h.app.session.catalog.photo(target).unwrap().label, None, "a second click takes it off");
    h.app.run("tool.painter", json!({"mode": "rating", "value": "4"})).unwrap();
    paint_click(&mut h);
    assert_eq!(h.app.session.catalog.photo(target).unwrap().rating, 4);
    assert!(h.app.run("tool.painter", json!({"mode": "glitter", "value": "x"})).is_err());
    h.request("ui.key", json!({"key": "escape"}), T);
    assert!(h.app.ui.painter.is_none());
}
