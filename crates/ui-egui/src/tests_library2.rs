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
