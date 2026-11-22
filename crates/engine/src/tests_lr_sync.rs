//! L01: syncing a migrated library from its Lightroom catalog again (synthetic catalogs).

use super::*;
use crate::Session;
use crate::lr_history::tests::{blob, catalog, exec, have_sqlite, photos, when};
use serde_json::json;

fn photo(s: &Session, name: char) -> lightcraft_catalog::Photo {
    (**s.catalog.photos().find(|p| p.file_name.starts_with(name) && p.copy_of.is_none()).unwrap()).clone()
}

/// Feature (L01): what changed in Lightroom after the migration comes over (new photo, settings,
/// rating, keywords, collections, history); what changed on both sides is listed and kept until
/// resolved, then Lightroom's replaces ours with ours kept as a version; a dry run changes nothing.
#[test]
fn sync_brings_lightroom_changes_and_keeps_ours() {
    if !have_sqlite() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lc-lrsync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    photos(&dir, &["a.png", "b.png", "c.png"]);
    let db = catalog(&dir, "s = { Exposure2012 = 0.2 }", "s = { Exposure2012 = 0.4 }");
    exec(
        &db,
        &format!(
            "INSERT INTO Adobe_libraryImageDevelopHistoryStep VALUES (1, {t}, 1, 'Exposure', {a}), (2, {t}, 2, 'Exposure', {b});
INSERT INTO AgLibraryKeyword VALUES (1, NULL, ''), (2, 1, 'old'), (3, 1, 'new');
INSERT INTO AgLibraryKeywordImage VALUES (1, 1, 2);
INSERT INTO AgLibraryCollection VALUES (1, 'Wedding', NULL, 'com.adobe.ag.library.collection', 0);
INSERT INTO AgLibraryCollectionImage VALUES (1, 1, 2, 1);",
            t = when(1.0),
            a = blob("s = { Exposure2012 = 0.2 }"),
            b = blob("s = { Exposure2012 = 0.4 }"),
        ),
    );
    let (db_s, dir_s) = (db.to_string_lossy().to_string(), dir.to_string_lossy().to_string());
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"catalog": db_s, "presets": false, "scratch": dir_s})).unwrap();
    assert_eq!(photo(&s, 'a').develop.light.exposure, 0.2);
    assert_eq!(photo(&s, 'b').develop.light.exposure, 0.4);
    assert_eq!(photo(&s, 'a').meta.keywords, ["old"]);
    // nothing changed yet
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!((r["develop"]["fromLightroom"].clone(), r["develop"]["conflicts"].clone()), (json!(0), json!(0)), "{r}");
    assert_eq!(r["firstSync"], json!(true));

    // later, in Lightroom: a re-edited, rated, keyworded; b re-edited; c added; and here, b edited
    exec(
        &db,
        &format!(
            "UPDATE Adobe_imageDevelopSettings SET text = 's = {{ Exposure2012 = 1 }}' WHERE image = 1;
UPDATE Adobe_imageDevelopSettings SET text = 's = {{ Exposure2012 = -1 }}' WHERE image = 2;
INSERT INTO Adobe_libraryImageDevelopHistoryStep VALUES (3, {t}, 1, 'Exposure', {a}), (4, {t}, 2, 'Exposure', {b});
UPDATE Adobe_images SET rating = 3, touchTime = {t} WHERE id_local = 1;
INSERT INTO AgLibraryKeywordImage VALUES (2, 1, 3);
INSERT INTO AgLibraryFile VALUES (3, 1, 'c', 'png');
INSERT INTO Adobe_images VALUES (3, NULL, NULL, NULL, 4, 0, '', 'AB', {t}, 'PNG', 24, 16, 3);
INSERT INTO Adobe_imageDevelopSettings VALUES (3, 3, 's = {{ Exposure2012 = 0.7 }}');
INSERT INTO Adobe_libraryImageDevelopHistoryStep VALUES (5, {t}, 3, 'Exposure', {c});
INSERT INTO AgLibraryCollectionImage VALUES (2, 1, 3, 2);",
            t = when(10.0),
            a = blob("s = { Exposure2012 = 1 }"),
            b = blob("s = { Exposure2012 = -1 }"),
            c = blob("s = { Exposure2012 = 0.7 }"),
        ),
    );
    let b = photo(&s, 'b').id;
    s.selection = crate::Selection::single(b);
    s.execute("develop.merge", &json!({"settings": {"light": {"contrast": 25.0}}})).unwrap();
    let before: Vec<_> = s.catalog.photos().cloned().collect();
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!(r["newPhotos"]["found"], json!(1), "{r}");
    assert_eq!(r["develop"]["fromLightroom"], json!(1), "{r}");
    assert_eq!(r["develop"]["conflicts"], json!(1), "{r}");
    assert_eq!(r["develop"]["conflictList"][0]["photoId"], json!(b.0), "{r}");
    assert_eq!(r["metadata"]["rating"], json!(1), "{r}");
    assert_eq!(r["metadata"]["keywords"], json!(1), "{r}");
    assert_eq!(s.catalog.photos().cloned().collect::<Vec<_>>(), before, "a dry run changes nothing");
    assert!(!dir.join("lib").join(SYNC_FILE).exists());

    // sync (conflicts asked: kept)
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s})).unwrap();
    let a = photo(&s, 'a');
    assert_eq!((a.develop.light.exposure, a.rating), (1.0, 3), "{r}");
    assert_eq!(a.meta.keywords, ["old", "new"]);
    assert_eq!(a.history.last().unwrap().label, "Lightroom Sync");
    let pb = photo(&s, 'b');
    assert_eq!((pb.develop.light.exposure, pb.develop.light.contrast), (0.4, 25.0), "ours kept");
    let c = photo(&s, 'c');
    assert_eq!((c.develop.light.exposure, c.rating), (0.7, 4), "{r}");
    // the new photo joined its collection; its history came along
    let wedding = s.catalog.albums().find(|x| x.name == "Wedding").unwrap().clone();
    assert!(wedding.photos.contains(&c.id) && wedding.photos.contains(&b), "{:?}", wedding.photos);
    assert!(!wedding.photos.contains(&a.id));
    let steps = s.execute("history.lightroom", &json!({"id": c.id.0})).unwrap();
    assert_eq!(steps["steps"].as_array().unwrap().len(), 1, "{steps}");
    assert!(dir.join("lib").join(SYNC_FILE).is_file());
    // one undo step takes it all back
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(photo(&s, 'a').develop.light.exposure, 0.2);
    s.execute("edit.redo", &json!({})).unwrap();

    // again: nothing new, b still asked about
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!(r["firstSync"], json!(false));
    assert_eq!((r["develop"]["fromLightroom"].clone(), r["develop"]["conflicts"].clone()), (json!(0), json!(1)), "{r}");
    assert_eq!(r["newPhotos"]["found"], json!(0), "{r}");
    // take Lightroom's for b: ours become a version
    s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "take": [b.0]})).unwrap();
    let pb = photo(&s, 'b');
    assert_eq!((pb.develop.light.exposure, pb.develop.light.contrast), (-1.0, 0.0));
    let kept = pb.versions.iter().find(|v| v.name == KEPT_VERSION).unwrap();
    assert_eq!(kept.settings.light.contrast, 25.0);
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!(r["develop"]["conflicts"], json!(0), "settled: {r}");

    // since the last sync: Lightroom drops a keyword, puts a in the collection and rates it 2;
    // here a was rated 5 → that rating is a conflict (kept), the rest comes over
    exec(
        &db,
        &format!(
            "DELETE FROM AgLibraryKeywordImage WHERE tag = 2; INSERT INTO AgLibraryCollectionImage VALUES (3, 1, 1, 3); UPDATE Adobe_images SET rating = 2, touchTime = {} WHERE id_local = 1;",
            when(20.0)
        ),
    );
    s.selection = crate::Selection::single(a.id);
    s.execute("photo.rate", &json!({"rating": 5})).unwrap();
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s})).unwrap();
    let a = photo(&s, 'a');
    assert_eq!(a.meta.keywords, ["new"], "{r}");
    assert_eq!(a.rating, 5, "{r}");
    assert_eq!(r["metadata"]["conflicts"], json!(1), "{r}");
    assert!(s.catalog.albums().find(|x| x.name == "Wedding").unwrap().photos.contains(&a.id), "{r}");
    // newest wins on request: Lightroom's rating
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "conflicts": "lightroom"})).unwrap();
    assert_eq!(photo(&s, 'a').rating, 2, "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Hostile input: a catalog that isn't one, a bad records file, a damaged sync record, a bad option
/// — errors (or a first sync again), never a panic.
#[test]
fn bad_input_is_an_error_not_a_crash() {
    if !have_sqlite() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lc-lrsyncbad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    photos(&dir, &["a.png", "b.png"]);
    let db = catalog(&dir, "s = { Exposure2012 = 0.2 }", "s = { Exposure2012 = 0.4 }");
    let (db_s, dir_s) = (db.to_string_lossy().to_string(), dir.to_string_lossy().to_string());
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"catalog": db_s, "presets": false, "scratch": dir_s})).unwrap();
    let junk = dir.join("junk.lrcat");
    std::fs::write(&junk, b"not a catalog at all").unwrap();
    assert!(s.execute("library.syncLightroom", &json!({"catalog": junk.to_string_lossy(), "scratch": dir_s})).is_err());
    let bad = dir.join("bad.json");
    std::fs::write(&bad, b"{\"images\": 5}").unwrap();
    assert!(s.execute("library.syncLightroom", &json!({"records": bad.to_string_lossy()})).is_err());
    assert!(s.execute("library.syncLightroom", &json!({"catalog": db_s, "conflicts": "whatever"})).is_err());
    // a damaged sync record reads as none: a first sync again
    std::fs::write(dir.join("lib").join(SYNC_FILE), b"\x00\x01garbage").unwrap();
    let r = s.execute("library.syncLightroom", &json!({"catalog": db_s, "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!(r["firstSync"], json!(true), "{r}");
    // an empty catalog (no photos) syncs to nothing
    let empty = dir.join("empty.lrcat");
    exec(&empty, crate::lr_history::tests::SCHEMA);
    let r = s.execute("library.syncLightroom", &json!({"catalog": empty.to_string_lossy(), "scratch": dir_s, "dryRun": true})).unwrap();
    assert_eq!(r["develop"]["fromLightroom"], json!(0), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn digests_and_conflict_rules() {
    assert_eq!(digest(b""), "cbf29ce484222325");
    assert_ne!(digest(b"s = { Exposure2012 = 1 }"), digest(b"s = { Exposure2012 = 2 }"));
    assert_eq!(Conflicts::parse("Lightroom"), Some(Conflicts::Lightroom));
    assert_eq!(Conflicts::parse("newest"), Some(Conflicts::Newest));
    assert_eq!(Conflicts::parse("x"), None);
    assert_eq!(settle(false, true), Some(false));
    assert_eq!(settle(true, false), Some(true));
    assert_eq!(settle(true, true), None);
}
