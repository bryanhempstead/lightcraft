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
    let services = Services { png: Some(Box::new(png)), write: Some(Box::new(lightcraft_engine::export::write_file)), ..Default::default() };
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

    // Settings ▸ Delete key ▸ ask.: ⌫ → the question; remove. keeps the file
    h.app.ui.settings.delete_key = crate::state::DeleteKey::Ask;
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

/// Bryan's default (Settings ▸ Delete key ▸ trash., Confirm first off): ⌫ deletes at once — the
/// files go to the Trash, a toast says so with undo., and undo. (or ⌘Z) puts the files back.
/// Confirm first brings a question back; ⌘⌫ follows the same setting.
#[test]
fn delete_goes_straight_to_the_trash_and_undo_brings_it_back() {
    let dir = temp("delete-now");
    let mut h = headless(import_pngs(&dir.0, 4));
    let ids: Vec<_> = h.app.session.visible_cloned();
    let file = |h: &Headless, id| match &h.app.session.catalog.photo(id).unwrap().source {
        lightcraft_engine::catalog::Source::File { path } => std::path::PathBuf::from(path),
        _ => std::path::PathBuf::new(),
    };
    assert_eq!(h.app.ui.settings.delete_key, crate::state::DeleteKey::Trash);
    assert!(!h.app.ui.settings.confirm_delete);

    // two picked, ⌫: no dialog, both files in the Trash, a toast with undo.
    let (a, b) = (file(&h, ids[0]), file(&h, ids[1]));
    h.app.session.selection = lightcraft_engine::Selection { ids: vec![ids[0], ids[1]], active: Some(ids[0]) };
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, None, "no question");
    assert!(h.app.session.catalog.photo(ids[0]).is_none() && h.app.session.catalog.photo(ids[1]).is_none());
    assert!(!a.exists() && !b.exists() && dir.0.join("trash").join(a.file_name().unwrap()).exists());
    let toast = h.app.ui.toast.as_ref().map(|t| t.0.clone()).unwrap_or_default();
    assert_eq!(toast, "2 photos moved to the Trash");
    assert_eq!(h.app.ui.toast_undo.as_deref(), Some(toast.as_str()));
    click(&mut h, "button:toastUndo");
    assert!(a.exists() && b.exists(), "undo. puts the files back at their paths");
    assert!(h.app.session.catalog.photo(ids[0]).is_some() && h.app.session.catalog.photo(ids[1]).is_some());
    assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("Undo"));

    // ⌘Z works the same after another delete
    h.app.session.selection = lightcraft_engine::Selection::single(ids[2]);
    let c = file(&h, ids[2]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert!(!c.exists());
    // (the screenshot waits for renders: keep the toast up for it)
    h.step();
    if let Some(t) = h.app.ui.toast.as_mut() {
        t.1 = 1e9;
    }
    shot(&mut h, "delete-toast.png");
    h.request("ui.key", json!({"key": "z", "cmd": true}), T);
    assert!(c.exists() && h.app.session.catalog.photo(ids[2]).is_some());

    // Confirm first: the question comes back, and OK deletes to the Trash
    h.app.ui.settings.confirm_delete = true;
    h.app.session.selection = lightcraft_engine::Selection::single(ids[3]);
    let d = file(&h, ids[3]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, Some(Dialog::ConfirmDelete { count: 1, rejected: false }));
    assert!(d.exists());
    shot(&mut h, "delete-confirm-first.png");
    click(&mut h, "button:dialogOk");
    assert!(!d.exists() && h.app.session.catalog.photo(ids[3]).is_none());
    h.app.ui.settings.confirm_delete = false;

    // ⌘⌫ with trash.: the rejects go to the Trash without asking
    h.app.session.execute("photo.flag", &json!({"ids": [ids[2].0], "flag": "reject"})).unwrap();
    h.request("ui.key", json!({"key": "delete", "cmd": true}), T);
    assert_eq!(h.app.ui.dialog, None);
    assert!(!c.exists() && h.app.session.catalog.photo(ids[2]).is_none());

    // Settings ▸ General: the Delete key chips and Confirm first
    h.request("ui.key", json!({"key": ",", "cmd": true}), T);
    click(&mut h, "button:settingsDeleteKey-2");
    assert_eq!(h.app.ui.settings.delete_key, crate::state::DeleteKey::Album);
    click(&mut h, "button:settingsDeleteKey-0");
    click(&mut h, "check:settings.confirmDelete");
    assert!(h.app.ui.settings.confirm_delete);
    shot(&mut h, "settings-delete-key.png");
}

/// album.: in an album ⌫ takes the photo out of that album only; elsewhere out of the library
/// (files kept), and the first time it says so.
#[test]
fn album_choice_removes_from_the_album_or_else_the_library() {
    let dir = temp("delete-album");
    let mut h = headless(import_pngs(&dir.0, 2));
    let ids: Vec<_> = h.app.session.visible_cloned();
    h.app.ui.settings.delete_key = crate::state::DeleteKey::Album;
    let album = h.app.session.execute("album.create", &json!({"name": "Trip"})).unwrap()["id"].as_u64().unwrap();
    h.app.session.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0].0, ids[1].0]})).unwrap();
    h.app.session.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    h.app.session.selection = lightcraft_engine::Selection::single(ids[0]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, None);
    assert_eq!(h.app.session.catalog.album(lightcraft_engine::catalog::AlbumId(album)).unwrap().photos, vec![ids[1]]);
    assert!(h.app.session.catalog.photo(ids[0]).is_some());
    assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.as_str()), Some("1 photo removed from Trip"));
    // All Photographs: Remove from Library, told once
    h.app.session.execute("library.source", &json!({"kind": "all"})).unwrap();
    h.app.session.selection = lightcraft_engine::Selection::single(ids[1]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert!(h.app.session.catalog.photo(ids[1]).is_none() && dir.0.join("src/p1.png").exists());
    assert!(h.app.ui.toast.as_ref().is_some_and(|t| t.0.contains("not in an album")), "{:?}", h.app.ui.toast);
    assert!(h.app.ui.settings.album_fallback_told);
    h.app.session.selection = lightcraft_engine::Selection::single(ids[0]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert!(h.app.ui.toast.as_ref().is_some_and(|t| !t.0.contains("not in an album")), "only the first time");
}

/// The two old Delete checkboxes move to the Delete key choice: trashDelete true → ask.; false →
/// trash. (Bryan's ui.json had false and he asked for no dialog); none → trash.
#[test]
fn old_delete_settings_migrate_to_the_delete_key_choice() {
    use crate::state::{AppSettings, DeleteKey};
    let read = |s: &str| serde_json::from_str::<AppSettings>(s).unwrap().migrated();
    let asked = read(r#"{"trashDelete": true, "confirmDelete": false}"#);
    assert_eq!((asked.delete_key, asked.confirm_delete), (DeleteKey::Ask, false));
    let bryan = read(r#"{"trashDelete": false, "confirmDelete": false}"#);
    assert_eq!((bryan.delete_key, bryan.confirm_delete), (DeleteKey::Trash, false));
    let confirmed = read(r#"{"trashDelete": false, "confirmDelete": true}"#);
    assert_eq!((confirmed.delete_key, confirmed.confirm_delete), (DeleteKey::Trash, true));
    assert_eq!(read("{}").delete_key, DeleteKey::Trash);
    assert_eq!(read(r#"{"deleteKey": "recent"}"#).delete_key, DeleteKey::Recent);
    // the old key is never written again, so the migration runs once
    let saved = serde_json::to_value(&bryan).unwrap();
    assert!(saved.get("trashDelete").is_none() && saved["deleteKey"] == "trash", "{saved}");
    // a whole UI state goes through it on load
    let ui: crate::UiState = serde_json::from_str(r#"{"settings": {"trashDelete": true}}"#).unwrap();
    assert_eq!(ui.sanitized().settings.delete_key, DeleteKey::Ask);
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

/// E05: one export with several presets, each into its own subfolder.
#[test]
fn export_with_several_presets() {
    let dir = temp("presets-export");
    let mut h = headless(lightcraft_engine::Session::with_demo());
    let names: Vec<String> = vec!["JPEG (Small)".into(), "JPEG (Large)".into()];
    assert_eq!(names.len(), 2);
    let id = h.app.session.visible_cloned()[0];
    let r = h.app.run("app.export", json!({"presets": names, "ids": [id.0], "dir": dir.0.to_string_lossy(), "longEdge": 64})).unwrap();
    let done = r["presets"].as_array().unwrap();
    assert_eq!(done.len(), 2, "{r}");
    for (d, name) in done.iter().zip(&names) {
        assert_eq!(d["preset"], json!(name));
        let path = d["files"][0]["path"].as_str().unwrap_or_default();
        assert!(std::path::Path::new(path).exists() && path.contains(&name.replace(['/', '\\', ':'], "-")), "{r}");
    }
    assert!(h.app.run("app.export", json!({"presets": []})).is_err());
}

/// L18: ⌘J opens the grid's view options; the index number shows in Square Grid captions.
#[test]
fn grid_view_options() {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    h.app.ui.view = crate::state::ViewMode::SquareGrid;
    h.request("ui.key", json!({"key": "j", "cmd": true}), T);
    assert_eq!(h.app.ui.dialog, Some(Dialog::Settings { tab: "interface".into() }));
    click(&mut h, "check:settings.gridIndex");
    assert!(h.app.ui.settings.grid_index);
    shot(&mut h, "grid-options.png");
}

/// fav. in the filter bar: only the hearted photos, click again for all.
#[test]
fn filter_bar_heart_shows_only_favorites() {
    let mut s = lightcraft_engine::Session::with_demo();
    let all = s.visible_cloned();
    assert!(all.len() > 1);
    s.execute("photo.favorite", &json!({"favorite": true, "ids": [all[0].0]})).unwrap();
    let mut h = headless(s);
    h.app.run("view.library", json!({})).ok();
    h.app.ui.view = crate::state::ViewMode::PhotoGrid;
    h.app.ui.filter_bar = true;
    click(&mut h, "filter:favorite");
    assert_eq!(h.app.session.visible_cloned(), vec![all[0]]);
    click(&mut h, "filter:favorite");
    assert_eq!(h.app.session.visible_cloned().len(), all.len());
}

/// Catalog ▸ favorites.: every hearted photo; All Photographs shows them all again.
#[test]
fn catalog_favorites_row_shows_the_hearted_photos() {
    let mut s = lightcraft_engine::Session::with_demo();
    let all = s.visible_cloned();
    s.execute("photo.favorite", &json!({"favorite": true, "ids": [all[1].0]})).unwrap();
    let mut h = headless(s);
    h.app.run("view.library", json!({})).ok();
    h.step();
    let id = h.app.widgets.iter().map(|(w, _)| w.clone()).find(|w| w.contains("favorites") && w.starts_with("source:")).expect("a favorites. row");
    click(&mut h, &id);
    assert_eq!(h.app.session.visible_cloned(), vec![all[1]]);
    let all_row = h.app.widgets.iter().map(|(w, _)| w.clone()).find(|w| w == "source:all").expect("All Photographs");
    click(&mut h, &all_row);
    assert_eq!(h.app.session.visible_cloned().len(), all.len(), "All Photographs: every photo again");
}
