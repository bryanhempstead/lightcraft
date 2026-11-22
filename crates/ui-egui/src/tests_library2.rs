//! Library / Import / Export rows (second round), headless: each drives the real UI and, with
//! `LIGHTCRAFT_TEST_SHOTS=<dir>`, writes a real-size screenshot there.

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

pub(crate) const T: Duration = Duration::from_secs(20);

#[allow(dead_code)]
pub(crate) struct TempDir(pub std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[allow(dead_code)]
pub(crate) fn temp(tag: &str) -> TempDir {
    let d = std::env::temp_dir().join(format!("lc-ui2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    TempDir(d)
}

pub(crate) fn headless(session: lightcraft_engine::Session) -> Headless {
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = Services { png: Some(Box::new(png)), write: Some(Box::new(lightcraft_engine::export::write_file)), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1300.0, 900.0], 1.0);
    h.step();
    h.step();
    h
}

pub(crate) fn settle(h: &mut Headless) {
    for _ in 0..4 {
        h.step();
    }
}

pub(crate) fn click(h: &mut Headless, id: &str) {
    h.step();
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    settle(h);
}

pub(crate) fn request(h: &mut Headless, method: &str, params: Value) -> Value {
    let r = h.request(method, params, T);
    assert_eq!(r["ok"], true, "{method}: {r}");
    r
}

pub(crate) fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        for _ in 0..3 {
            h.step();
        }
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

/// L11: the Keyword List's imp. / exp. / purge. chips and Edit Keyword Tag.
#[test]
fn keyword_list_edit_tag_and_purge() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id;
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [id.0], "keywords": ["Places|Portugal|Lisbon", "Ann Lee"]})).unwrap();
    s.execute("keyword.create", &json!({"name": "Unused", "parent": null})).unwrap();
    let mut h = headless(s);
    h.app.ui.library_sections = vec!["keywordList".into()];
    settle(&mut h);
    h.app.run("dialog.editKeyword", json!({"keyword": "Ann Lee"})).unwrap();
    click(&mut h, "check:editKeyword-person");
    let r = h.request("ui.clickWidget", json!({"id": "field:editKeywordSynonyms"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    request(&mut h, "ui.text", json!({"text": "Ann, A. Lee"}));
    shot(&mut h, "l11-edit-keyword.png");
    click(&mut h, "button:dialogOk");
    let info = h.app.session.execute("keyword.info", &json!({"keyword": "ann lee"})).unwrap();
    assert_eq!(info["person"], true, "{info}");
    assert_eq!(info["synonyms"], json!(["Ann", "A. Lee"]));
    click(&mut h, "button:keywordListPurge");
    assert!(h.app.session.execute("keyword.info", &json!({"keyword": "Unused"})).is_err(), "purged");
    shot(&mut h, "l11-keyword-list.png");
}

/// L24: folder / collection labels and favourites in the sidebar; the collections filter field,
/// name. / kind. sort and fav. chip.
#[test]
fn folder_and_collection_marks_in_the_sidebar() {
    let dir = temp("marks");
    let src = dir.0.join("Trip");
    std::fs::create_dir_all(&src).unwrap();
    let mut s = lightcraft_engine::Session::with_demo();
    let path = src.to_string_lossy().to_string();
    // a photo in a real folder so Folders lists it
    let (w, hh) = (16usize, 12usize);
    let img = lightcraft_raster::Rgba8 { width: w, height: hh, data: vec![[90, 120, 150, 255]; w * hh] };
    let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(src.join("a.png"), png).unwrap();
    s.execute("library.import", &json!({"paths": [path.clone()]})).unwrap();
    s.execute("folder.label", &json!({"path": path, "label": "green"})).unwrap();
    s.execute("folder.favorite", &json!({"path": path})).unwrap();
    let ids: Vec<u64> = s.catalog.albums().filter(|a| !a.folder && !a.quick).map(|a| a.id.0).collect();
    s.execute("album.favorite", &json!({"id": ids[1]})).unwrap();
    s.execute("album.label", &json!({"id": ids[0], "label": "purple"})).unwrap();
    let mut h = headless(s);
    h.app.ui.collapsed_sidebar = vec!["navigator".into(), "catalog".into(), "shoots".into()];
    settle(&mut h);
    shot(&mut h, "l24-sidebar.png");
    click(&mut h, "button:collectionsFavorites");
    let widgets = request(&mut h, "ui.widgets", json!({}));
    let text = widgets.to_string();
    assert!(text.contains(&format!("album:{}\"", ids[1])), "the favourite stays: {text}");
    assert!(!text.contains(&format!("album:{}\"", ids[0])), "the others go");
    shot(&mut h, "l24-favorites.png");
}

/// E05: the Export dialog's Several row picks presets (all. / none. / click / ⌘-click) and
/// export.N.presets. exports with each into its own folder.
#[test]
fn export_dialog_several_presets() {
    let dir = temp("e05");
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    h.app.run("dialog.export", json!({})).unwrap();
    if let Some(crate::state::Dialog::Export { dir: d, .. }) = &mut h.app.ui.dialog {
        *d = dir.0.to_string_lossy().to_string();
    }
    settle(&mut h);
    click(&mut h, "button:exportBatch-0");
    click(&mut h, "button:exportBatchAll");
    click(&mut h, "button:exportBatchNone");
    click(&mut h, "button:exportBatch-1");
    shot(&mut h, "e05-export-several.png");
    click(&mut h, "button:exportBatchRun");
    assert!(h.app.ui.dialog.is_none(), "the dialog closes");
    let folders: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().filter(|e| e.path().is_dir()).collect();
    assert_eq!(folders.len(), 1, "one preset picked: one folder");
    assert!(std::fs::read_dir(folders[0].path()).unwrap().count() >= 1, "with the photo in it");
}

/// L30: File ▸ Import from Another Library… previews what the other library holds and asks what to
/// do with the photos both have (keep mine / take metadata / take all).
#[test]
fn import_from_another_library_dialog() {
    let dir = temp("l30");
    let src = dir.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (i, n) in ["a.png", "b.png"].iter().enumerate() {
        let img = lightcraft_raster::Rgba8 { width: 8, height: 6, data: vec![[i as u8 * 90, 50, 70, 255]; 48] };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(src.join(n), png).unwrap();
    }
    // library A has both photos, titled; B has a.png only
    let mut a = lightcraft_engine::Session::new().with_fs();
    a.open_library(dir.0.join("libA"), false).unwrap();
    let r = a.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap();
    for id in r["imported"].as_array().unwrap() {
        a.execute("photo.setMeta", &json!({"ids": [id], "title": "From A"})).unwrap();
    }
    a.close_library().unwrap();
    drop(a);
    let mut b = lightcraft_engine::Session::new().with_fs();
    b.open_library(dir.0.join("libB"), false).unwrap();
    b.execute("library.import", &json!({"paths": [src.join("a.png").to_string_lossy()]})).unwrap();
    let mut h = headless(b);
    h.app.run("dialog.importCatalog", json!({"path": dir.0.join("libA").to_string_lossy()})).unwrap();
    settle(&mut h);
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::ImportCatalog { preview, .. }) => {
            assert_eq!((preview["added"].as_u64(), preview["inBoth"].as_u64()), (Some(1), Some(1)), "{preview}");
        }
        other => panic!("no dialog: {other:?}"),
    }
    assert_eq!(h.app.session.catalog.photos().count(), 1, "a preview changes nothing");
    click(&mut h, "button:importCatalogReplace-1");
    shot(&mut h, "l30-import-library.png");
    click(&mut h, "button:dialogOk");
    assert_eq!(h.app.session.catalog.photos().count(), 2);
    assert!(h.app.session.catalog.photos().all(|p| p.meta.title == "From A"), "the shared photo took A's metadata");
}

/// E07: the Export dialog offers PSD and JPEG XL.
#[test]
fn export_dialog_offers_psd_and_jpeg_xl() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    click(&mut h, "button:exportFormat-7");
    assert!(
        matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Export { opts, .. }) if opts.format == lightcraft_engine::export::ExportFormat::Psd)
    );
    click(&mut h, "button:exportFormat-8");
    assert!(
        matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Export { opts, .. }) if opts.format == lightcraft_engine::export::ExportFormat::Jxl)
    );
    shot(&mut h, "e07-export-formats.png");
}

/// E08: the Export dialog's Profile popup (ColorMatch RGB, installed profiles).
#[test]
fn export_dialog_profile_popup() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    h.app.run("dialog.export", json!({})).unwrap();
    if let Some(crate::state::Dialog::Export { opts, .. }) = &mut h.app.ui.dialog {
        opts.icc_profile = Some("colorMatch".into());
    }
    settle(&mut h);
    let w = request(&mut h, "ui.widgets", json!({})).to_string();
    assert!(w.contains("combo:exportIccProfile"), "{w}");
    shot(&mut h, "e08-export-profile.png");
}

/// I03: the Import window's Embedded & Sidecar previews (grid thumbnails first) and Assisted
/// Culling at import.
#[test]
fn import_previews_and_culling() {
    let dir = temp("i03");
    let src = dir.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..3u8 {
        let img = lightcraft_raster::Rgba8 {
            width: 32,
            height: 24,
            data: (0..32 * 24).map(|k| [(k % 32 * 7) as u8, i * 60, (k / 32 * 9) as u8, 255]).collect(),
        };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(src.join(format!("p{i}.png")), png).unwrap();
    }
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(dir.0.join("lib"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap();
    let ids: Vec<u64> = r["imported"].as_array().unwrap().iter().filter_map(|v| v.as_u64()).collect();
    let mut h = headless(s);
    h.app.session.execute("library.select", &json!({"ids": ids})).unwrap();
    let prefs = crate::import::ImportPrefs { previews: "embedded".into(), cull: true, ..Default::default() };
    crate::import_window::after_window_import(&mut h.app, &prefs, ids.len(), &[]);
    settle(&mut h);
    for id in &ids {
        assert!(h.app.session.catalog.photo(lightcraft_catalog::PhotoId(*id)).unwrap().analysis.is_some(), "scored at import");
    }
    let b = h.app.session.preview_build.clone().expect("a thumbnail build started");
    assert_eq!(b.total, ids.len());
    assert!(h.app.session.execute("library.buildPreviews", &json!({"size": "huge"})).is_err());
    // the option in the Import window
    h.app.run("file.addPhotos", json!({"source": src.to_string_lossy()})).unwrap();
    settle(&mut h);
    shot(&mut h, "i03-import-window.png");
}

/// L26: Compare's Link Focus (the select keeps its own zoom while unlinked) and the Navigator's
/// other zoom levels.
#[test]
fn compare_link_focus_and_navigator_zooms() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    let mut h = headless(s);
    h.app.run("view.compare", json!({})).unwrap();
    settle(&mut h);
    shot(&mut h, "l26-compare.png");
    assert!(h.app.ui.compare_frozen.is_none(), "linked by default");
    click(&mut h, "button:compareLink");
    assert!(h.app.ui.compare_frozen.is_some(), "unlinked");
    let frozen = h.app.ui.compare_frozen;
    h.app.run("view.navigate", json!({"zoom": {"percent": 1600}})).unwrap();
    settle(&mut h);
    assert_eq!(h.app.ui.compare_frozen, frozen, "the select keeps its view");
    shot(&mut h, "l26-compare-unlinked.png");
    click(&mut h, "button:compareLink");
    assert!(h.app.ui.compare_frozen.is_none());
    // the navigator's ▾ shows a level outside the presets
    h.app.run("view.navigate", json!({"zoom": {"percent": 25}})).unwrap();
    h.app.run("view.detail", json!({})).unwrap();
    settle(&mut h);
    let w = request(&mut h, "ui.widgets", json!({})).to_string();
    assert!(w.contains("button:navZoomMore"), "{w}");
    shot(&mut h, "l26-navigator.png");
}

/// E12: File ▸ Email Photos… exports the selection small and opens a mail message with them
/// attached (here a stand-in for Mail records the files).
#[test]
fn email_photos_attaches_small_exports() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    let got: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
    let mut h = headless(s);
    let g = got.clone();
    h.app.services.compose_email = Some(Box::new(move |files: &[String]| {
        g.lock().unwrap().extend(files.iter().cloned());
        Ok(())
    }));
    let r = h.app.run("file.emailPhotos", json!({"longEdge": 400})).unwrap();
    let files = got.lock().unwrap().clone();
    assert_eq!(files.len(), 2, "{r}");
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        let d = lightcraft_codecs::decode(&bytes, lightcraft_codecs::DecodeOptions::default()).unwrap();
        assert_eq!(d.image.width.max(d.image.height), 400);
        let _ = std::fs::remove_file(f);
    }
    h.app.services.compose_email = None;
    assert!(h.app.run("file.emailPhotos", json!({})).is_err(), "no Mail here: an error, not a crash");
}

/// L23: Library ▸ Validate DNG Files… says what it found in a toast.
#[test]
fn validate_dng_menu_toast() {
    let s = lightcraft_engine::Session::with_demo();
    let mut h = headless(s);
    let r = h.app.run("dialog.validateDng", json!({})).unwrap();
    assert_eq!(r["checked"], 0, "the demo photos are JPEGs");
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Checked 0 DNGs"), "{toast}");
    settle(&mut h);
    shot(&mut h, "l23-validate-dng.png");
}

/// L33: File ▸ Start / Stop Tethered Capture.
#[test]
fn tethered_capture_menu() {
    let dir = temp("tether");
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(dir.0.join("lib"), false).unwrap();
    let mut h = headless(s);
    let r = h.app.run("dialog.tetherStart", json!({"folder": dir.0.join("shots").to_string_lossy(), "session": "Studio"})).unwrap();
    assert_eq!(r["running"], true, "{r}");
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Tethered:"), "{toast}");
    settle(&mut h);
    shot(&mut h, "l33-tethered.png");
    h.app.run("dialog.tetherStop", json!({})).unwrap();
    assert_eq!(h.app.session.execute("tether.status", &json!({})).unwrap()["running"], false);
}
