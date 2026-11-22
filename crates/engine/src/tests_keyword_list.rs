//! L11 (ported from upstream): the keyword list — keyword tag options (synonyms, include on export,
//! export containing keywords, person), Import / Export Keywords, Purge Unused, the keyword
//! shortcut — and what an export writes from them.

use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::Session;

fn session() -> (Session, PhotoId) {
    let mut s = Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id;
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [id.0], "keywords": ["Places|Portugal|Lisbon", "Ann Lee", "dawn"]})).unwrap();
    (s, id)
}

#[test]
fn keyword_options_decide_what_an_export_writes() {
    let (mut s, id) = session();
    s.execute("keyword.edit", &json!({"keyword": "places|portugal|lisbon", "synonyms": ["Lisboa"], "exportContaining": false})).unwrap();
    s.execute("keyword.edit", &json!({"keyword": "dawn", "includeOnExport": false})).unwrap();
    s.execute("keyword.edit", &json!({"keyword": "Ann Lee", "person": true})).unwrap();
    let info = s.execute("keyword.info", &json!({"keyword": "Places|Portugal|Lisbon"})).unwrap();
    assert_eq!(info["synonyms"], json!(["Lisboa"]));
    assert_eq!((info["listed"].as_bool(), info["exportContaining"].as_bool(), info["count"].as_u64()), (Some(true), Some(false), Some(1)));
    let p = s.catalog.photo(id).unwrap().clone();
    let o = crate::export::ExportOptions::default();
    let (flat, _) = crate::export::export_keywords_with(Some(&s.catalog), &p, &o);
    assert_eq!(flat, ["Lisbon", "Lisboa", "Ann Lee"], "no containing keywords, a synonym, dawn left out");
    let o = crate::export::ExportOptions::from_params(&json!({"removePeople": true})).unwrap();
    let (flat, _) = crate::export::export_keywords_with(Some(&s.catalog), &p, &o);
    assert_eq!(flat, ["Lisbon", "Lisboa"], "a person keyword goes with Remove Person Info");
    // one undo step each, and undo restores the options
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("keyword.info", &json!({"keyword": "Ann Lee"})).unwrap()["person"], false);
}

#[test]
fn import_export_and_purge_the_keyword_list() {
    let (mut s, _) = session();
    s.execute("keyword.create", &json!({"name": "Unused", "parent": null})).unwrap();
    let out = s.execute("keyword.export", &json!({})).unwrap();
    let text = out["text"].as_str().unwrap().to_string();
    assert!(text.contains("Places\n\tPortugal\n\t\tLisbon") && text.contains("Unused"), "{text}");
    // a list from Lightroom: tabs for levels, synonyms in braces, [left out of export]
    let r = s.execute("keyword.import", &json!({"text": "Animals\n\tBirds\n\t\t{Aves}\n[Private]\n"})).unwrap();
    assert_eq!((r["added"].as_u64(), r["updated"].as_u64()), (Some(3), Some(0)), "{r}");
    assert_eq!(s.execute("keyword.info", &json!({"keyword": "animals|birds"})).unwrap()["synonyms"], json!(["Aves"]));
    assert_eq!(s.execute("keyword.info", &json!({"keyword": "Private"})).unwrap()["includeOnExport"], false);
    assert!(s.execute("keyword.import", &json!({"text": "  spaced"})).is_err(), "levels are tabs");
    assert!(s.execute("keyword.import", &json!({})).is_err());
    let purged = s.execute("keyword.purgeUnused", &json!({})).unwrap();
    assert_eq!(purged["purged"], 4, "Unused, Animals, Animals|Birds, Private: {purged}");
    assert!(s.execute("keyword.info", &json!({"keyword": "Unused"})).is_err());
    assert!(s.execute("keyword.info", &json!({"keyword": "dawn"})).is_ok(), "photos still carry it");
}

#[test]
fn keyword_shortcut_adds_then_removes() {
    let (mut s, id) = session();
    s.execute("keyword.setShortcut", &json!({"keywords": "Favourites, Print"})).unwrap();
    assert_eq!(s.execute("keyword.shortcut", &json!({})).unwrap()["keywords"], json!(["Favourites", "Print"]));
    s.execute("keyword.toggleShortcut", &json!({"ids": [id.0]})).unwrap();
    let kw = s.catalog.photo(id).unwrap().meta.keywords.clone();
    assert!(kw.iter().any(|k| k == "Favourites") && kw.iter().any(|k| k == "Print"), "{kw:?}");
    s.execute("keyword.toggleShortcut", &json!({"ids": [id.0]})).unwrap();
    assert!(!s.catalog.photo(id).unwrap().meta.keywords.iter().any(|k| k == "Print"));
    assert!(s.execute("keyword.setShortcut", &json!({"keywords": 3})).is_err());
}

/// A catalog with the keyword list survives the journal (format v3) and reloads.
#[test]
fn keyword_list_persists() {
    let dir = std::env::temp_dir().join(format!("lc-kwlist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.open_library(&dir, true).unwrap();
    s.execute("keyword.create", &json!({"name": "Kept", "synonyms": ["Held"], "parent": null})).unwrap();
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new();
    s.open_library(&dir, false).unwrap();
    assert_eq!(s.execute("keyword.info", &json!({"keyword": "kept"})).unwrap()["synonyms"], json!(["Held"]));
    let _ = std::fs::remove_dir_all(&dir);
}
