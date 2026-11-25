//! Favorites kept safe in the app, headless: off unless the app may write (tests never touch
//! LOT48); a heart copies on the worker and the grid heart gets its `kept.` mark; Settings ▸
//! General ▸ Favorites; ⌫ on a kept favorite asks whether the kept copy goes too (keep. by default).
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
    let d = std::env::temp_dir().join(format!("lc-ui-fav-{tag}-{}", std::process::id()));
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

fn import_pngs(dir: &std::path::Path, n: u8) -> lightcraft_engine::Session {
    for i in 0..n {
        let (w, h) = (24usize, 16usize);
        let data: Vec<[u8; 4]> = (0..w * h).map(|k| [(k % w * 5) as u8, (k / w * 7) as u8, i * 40, 255]).collect();
        let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::create_dir_all(dir.join("Shoot")).unwrap();
        std::fs::write(dir.join(format!("Shoot/p{i}.png")), bytes).unwrap();
    }
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.trash_dir = Some(dir.join("trash"));
    s.execute("library.import", &json!({"paths": [dir.join("Shoot").to_string_lossy()]})).unwrap();
    s
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
fn tests_never_keep_to_the_default_drive() {
    let dir = temp("off");
    let mut h = headless(import_pngs(&dir.0, 1));
    // the default settings say on. + LOT48, but this app isn't the desktop app with saved settings
    assert!(h.app.ui.settings.fav_keep);
    assert_eq!(h.app.ui.settings.fav_keep_dir, lightcraft_engine::fav_keep::DEFAULT_ROOT);
    h.step();
    assert_eq!(h.app.session.fav_keep.root, None, "never LOT48 from a test");
}

#[test]
fn a_heart_is_kept_on_the_worker_and_the_grid_says_kept() {
    let dir = temp("heart");
    let root = dir.0.join("keep");
    let mut h = headless(import_pngs(&dir.0, 3));
    h.app.fav_keep_live = true;
    h.app.ui.settings.fav_keep_dir = root.display().to_string();
    h.app.ui.view = crate::state::ViewMode::PhotoGrid;
    let ids = h.app.session.visible_cloned();
    h.app.session.selection = lightcraft_engine::Selection::single(ids[0]);
    h.request("engine.execute", json!({"id": "photo.toggleFavorite", "params": {}}), T);
    let kept = |h: &Headless| lightcraft_engine::fav_keep::kept(&h.app.session.catalog, ids[0]);
    assert!(h.step_until(T, |h| kept(h).is_some()), "kept by the worker");
    let k = kept(&h).unwrap();
    let name = h.app.session.catalog.photo(ids[0]).unwrap().file_name.clone();
    assert!(root.join("Shoot").join(&name).is_file() && std::path::Path::new(&k.path).is_file(), "{k:?}");
    assert!(lightcraft_engine::fav_keep::is_kept(&h.app.session.catalog, h.app.session.catalog.photo(ids[0]).unwrap()));
    let summary = crate::fav_keep_ui::summary(&h.app);
    assert!(summary.starts_with("1 kept on "), "{summary}");
    shot(&mut h, "fav-kept-grid.png");

    // Settings ▸ General ▸ Favorites: off. stops it
    h.request("ui.key", json!({"key": ",", "cmd": true}), T);
    click(&mut h, "button:settingsFavKeep-1");
    assert!(!h.app.ui.settings.fav_keep);
    h.step();
    assert_eq!(h.app.session.fav_keep.root, None);
    shot(&mut h, "fav-settings.png");
    click(&mut h, "button:settingsFavKeep-0");
    h.step();
    assert_eq!(h.app.session.fav_keep.root.as_deref(), Some(root.as_path()));
    h.app.ui.dialog = None;
    h.step();

    // ⌫ on the kept favorite asks first; keep. (Return) leaves the kept copy
    h.app.session.selection = lightcraft_engine::Selection::single(ids[0]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, Some(Dialog::DeleteKept { count: 1, kept: 1, rejected: false }));
    shot(&mut h, "fav-delete-question.png");
    h.request("ui.key", json!({"key": "enter"}), T);
    h.step();
    assert!(h.app.session.catalog.photo(ids[0]).is_none(), "the photo went");
    assert!(std::path::Path::new(&k.path).is_file(), "the kept copy stayed");

    // a second kept favorite, delete.too.: its kept copy goes to the Trash with it
    h.app.session.selection = lightcraft_engine::Selection::single(ids[1]);
    h.request("engine.execute", json!({"id": "photo.toggleFavorite", "params": {}}), T);
    let kept1 = |h: &Headless| lightcraft_engine::fav_keep::kept(&h.app.session.catalog, ids[1]);
    assert!(h.step_until(T, |h| kept1(h).is_some()));
    let k1 = kept1(&h).unwrap();
    h.request("ui.key", json!({"key": "delete"}), T);
    assert!(matches!(h.app.ui.dialog, Some(Dialog::DeleteKept { .. })));
    click(&mut h, "button:dialogDeleteKept");
    assert!(h.app.session.catalog.photo(ids[1]).is_none());
    assert!(!std::path::Path::new(&k1.path).exists(), "kept copy trashed too");
    assert_eq!(h.app.ui.delete_kept, None, "the answer is used once");

    // an unkept photo: no question (Bryan's default: straight to the Trash)
    h.app.session.selection = lightcraft_engine::Selection::single(ids[2]);
    h.request("ui.key", json!({"key": "delete"}), T);
    assert_eq!(h.app.ui.dialog, None);
    assert!(h.app.session.catalog.photo(ids[2]).is_none());
}
