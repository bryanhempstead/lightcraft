//! U15 / R18: Lightroom history steps and snapshots from a (synthetic) catalog.

use super::*;
use crate::Session;
use serde_json::json;
use std::path::{Path, PathBuf};

/// The tables (and columns) of a Lightroom catalog that LightCraft reads.
pub(crate) const SCHEMA: &str = "
CREATE TABLE Adobe_images (id_local INTEGER PRIMARY KEY, masterImage, copyName, copyReason, rating, pick, colorLabels, orientation, touchTime, fileFormat, fileWidth, fileHeight, rootFile);
CREATE TABLE AgLibraryFile (id_local INTEGER PRIMARY KEY, folder, baseName, extension);
CREATE TABLE AgLibraryFolder (id_local INTEGER PRIMARY KEY, rootFolder, pathFromRoot);
CREATE TABLE AgLibraryRootFolder (id_local INTEGER PRIMARY KEY, absolutePath);
CREATE TABLE Adobe_imageDevelopSettings (id_local INTEGER PRIMARY KEY, image, text);
CREATE TABLE Adobe_libraryImageDevelopHistoryStep (id_local INTEGER PRIMARY KEY, dateCreated, image, name, text);
CREATE TABLE Adobe_libraryImageDevelopSnapshot (id_local INTEGER PRIMARY KEY, image, name, text);
CREATE TABLE AgLibraryIPTC (id_local INTEGER PRIMARY KEY, image, caption, copyright);
CREATE TABLE AgHarvestedIptcMetadata (id_local INTEGER PRIMARY KEY, image, creatorRef, locationRef, cityRef, stateRef, countryRef);
CREATE TABLE AgInternedIptcCreator (id_local INTEGER PRIMARY KEY, value);
CREATE TABLE AgInternedIptcLocation (id_local INTEGER PRIMARY KEY, value);
CREATE TABLE AgInternedIptcCity (id_local INTEGER PRIMARY KEY, value);
CREATE TABLE AgInternedIptcState (id_local INTEGER PRIMARY KEY, value);
CREATE TABLE AgInternedIptcCountry (id_local INTEGER PRIMARY KEY, value);
CREATE TABLE AgLibraryKeyword (id_local INTEGER PRIMARY KEY, parent, name);
CREATE TABLE AgLibraryKeywordImage (id_local INTEGER PRIMARY KEY, image, tag);
CREATE TABLE AgLibraryCollection (id_local INTEGER PRIMARY KEY, name, parent, creationId, systemOnly);
CREATE TABLE AgLibraryCollectionImage (id_local INTEGER PRIMARY KEY, collection, image, positionInCollection);
CREATE TABLE AgLibraryCollectionContent (id_local INTEGER PRIMARY KEY, collection, content, owningModule);
";

/// Is the system `sqlite3` there (the tests that build catalogs skip without it)?
pub(crate) fn have_sqlite() -> bool {
    std::process::Command::new("sqlite3").arg("-version").output().is_ok_and(|o| o.status.success())
}

/// Run SQL on `db` (created if needed).
pub(crate) fn exec(db: &Path, sql: &str) {
    use std::io::Write;
    let mut c = std::process::Command::new("sqlite3").arg(db).stdin(std::process::Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(sql.as_bytes()).unwrap();
    assert!(c.wait().unwrap().success(), "sqlite3 failed on {sql}");
}

/// A history-step blob as Lightroom writes it (4-byte big-endian length, then zlib), as an SQL literal.
pub(crate) fn blob(text: &str) -> String {
    let mut b = (text.len() as u32).to_be_bytes().to_vec();
    b.extend(miniz_oxide::deflate::compress_to_vec_zlib(text.as_bytes(), 6));
    format!("X'{}'", b.iter().map(|x| format!("{x:02X}")).collect::<String>())
}

/// Cocoa seconds of an ISO-ish day offset (2026-01-01 + `n` hours).
pub(crate) fn when(n: f64) -> f64 {
    referenced_time() + n * 3600.0
}
fn referenced_time() -> f64 {
    // 2026-01-01T00:00:00Z in Cocoa seconds
    1_767_225_600.0 - 978_307_200.0
}

/// A shoot folder with `names` as small, distinct PNGs.
pub(crate) fn photos(dir: &Path, names: &[&str]) -> PathBuf {
    let shoot = dir.join("shoot");
    std::fs::create_dir_all(&shoot).unwrap();
    for (i, n) in names.iter().enumerate() {
        let v = 30 + 40 * i as u8;
        let img = lightcraft_raster::Rgba8::filled(24, 16, [v, v, v, 255]);
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(shoot.join(n), bytes).unwrap();
    }
    shoot
}

/// A catalog with photos `a.png` (id 1) and `b.png` (id 2) under `dir/shoot/`, with `develop` texts.
pub(crate) fn catalog(dir: &Path, dev_a: &str, dev_b: &str) -> PathBuf {
    let db = dir.join("t.lrcat");
    let _ = std::fs::remove_file(&db);
    let root = format!("{}/", dir.display()).replace('\'', "''");
    exec(
        &db,
        &format!(
            "{SCHEMA}
INSERT INTO AgLibraryRootFolder VALUES (1, '{root}');
INSERT INTO AgLibraryFolder VALUES (1, 1, 'shoot/');
INSERT INTO AgLibraryFile VALUES (1, 1, 'a', 'png'), (2, 1, 'b', 'png');
INSERT INTO Adobe_images VALUES (1, NULL, NULL, NULL, 0, 0, '', 'AB', {t}, 'PNG', 24, 16, 1), (2, NULL, NULL, NULL, 0, 0, '', 'AB', {t}, 'PNG', 24, 16, 2);
INSERT INTO Adobe_imageDevelopSettings VALUES (1, 1, '{a}'), (2, 2, '{b}');
",
            t = when(0.0),
            a = dev_a.replace('\'', "''"),
            b = dev_b.replace('\'', "''")
        ),
    );
    db
}

fn lightroom_labels(s: &mut Session, id: u64) -> Vec<String> {
    let r = s.execute("history.lightroom", &json!({"id": id})).unwrap();
    r["steps"].as_array().unwrap().iter().map(|x| x["label"].as_str().unwrap().to_string()).collect()
}

/// Feature (U15 + R18): a migration brings each photo's Lightroom history (shown, restorable, usable as
/// Before) and its snapshots (as versions); running it again adds nothing twice.
#[test]
fn history_and_snapshots_come_over() {
    if !have_sqlite() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lc-lrhist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    photos(&dir, &["a.png", "b.png"]);
    let last = "s = { Exposure2012 = 0.5,\nContrast2012 = 20 }";
    let db = catalog(&dir, last, "s = { Exposure2012 = -1 }");
    exec(
        &db,
        &format!(
            "INSERT INTO Adobe_libraryImageDevelopHistoryStep VALUES
 (1, {t0}, 1, 'Import (1/1/26 9:00:00 AM)', {s0}),
 (2, {t1}, 1, 'Exposure', {s1}),
 (3, {t2}, 1, 'Contrast', {s2}),
 (4, {t0}, 2, 'From Lr mobile', 's = {{ Exposure2012 = -1 }}');
INSERT INTO Adobe_libraryImageDevelopSnapshot VALUES (1, 1, 'From Lr mobile', 's = {{ Exposure2012 = 0.3 }}');",
            t0 = when(1.0),
            t1 = when(2.0),
            t2 = when(3.0),
            s0 = blob("s = { Exposure2012 = 0 }"),
            s1 = blob("s = { Exposure2012 = 0.5 }"),
            s2 = blob(last),
        ),
    );
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r =
        s.execute("library.migrateLightroom", &json!({"catalog": db.to_string_lossy(), "presets": false, "scratch": dir.to_string_lossy()})).unwrap();
    let h = &r["lightroomHistory"];
    assert_eq!((h["history"]["photos"].clone(), h["history"]["steps"].clone()), (json!(2), json!(4)), "{r}");
    assert_eq!(h["snapshots"]["added"], json!(1), "{r}");
    let id = |s: &Session, n: char| s.catalog.photos().find(|p| p.file_name.starts_with(n)).unwrap().id;
    let (a, b) = (id(&s, 'a'), id(&s, 'b'));
    assert_eq!(lightroom_labels(&mut s, a.0), ["Import (1/1/26 9:00:00 AM)", "Exposure", "Contrast"]);
    assert_eq!(lightroom_labels(&mut s, b.0), ["From Lr mobile"]);
    // the migration writes no History steps of its own: the photo is unedited here
    assert!(s.catalog.photo(a).unwrap().history.is_empty());
    // the snapshot is a version, dated like the history step of that name
    let v = s.catalog.photo(a).unwrap().versions.clone();
    assert_eq!(v.len(), 1);
    assert_eq!((v[0].name.as_str(), v[0].settings.light.exposure, v[0].auto), ("From Lr mobile", 0.3, false));
    // go back to a Lightroom step: an undoable History step here
    s.selection = crate::Selection::single(a);
    s.execute("history.restoreLightroom", &json!({"index": 1})).unwrap();
    let p = s.catalog.photo(a).unwrap().clone();
    assert_eq!((p.develop.light.exposure, p.develop.light.contrast), (0.5, 0.0));
    assert_eq!(p.history.last().unwrap().label, "Lightroom: Exposure");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().develop.light.contrast, 20.0);
    // a Lightroom step as Before
    s.execute("beforeAfter.setBefore", &json!({"source": "lightroom", "index": 0})).unwrap();
    assert_eq!(s.before.get(&a).unwrap().light.exposure, 0.0);
    assert!(s.execute("history.restoreLightroom", &json!({"index": 99})).is_err());
    // again: the history is rewritten the same, the snapshot isn't added twice
    let r = s.execute("library.migrateLightroomHistory", &json!({"catalog": db.to_string_lossy(), "scratch": dir.to_string_lossy()})).unwrap();
    assert_eq!((r["snapshots"]["added"].clone(), r["snapshots"]["alreadyThere"].clone()), (json!(0), json!(1)), "{r}");
    assert_eq!(s.catalog.photo(a).unwrap().versions.len(), 1);
    assert_eq!(lightroom_labels(&mut s, a.0).len(), 3);
    // a dry run writes nothing
    let r = s
        .execute("library.migrateLightroomHistory", &json!({"catalog": db.to_string_lossy(), "dryRun": true, "scratch": dir.to_string_lossy()}))
        .unwrap();
    assert_eq!(r["history"]["photos"], json!(2), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Hostile input: an unreadable step is counted and skipped, a corrupt history file reads as none,
/// a file that isn't a catalog is an error — never a panic.
#[test]
fn corrupt_history_is_an_error_not_a_crash() {
    if !have_sqlite() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("lc-lrhistbad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    photos(&dir, &["a.png", "b.png"]);
    let db = catalog(&dir, "s = { Exposure2012 = 1 }", "s = { Exposure2012 = 1 }");
    exec(
        &db,
        &format!(
            "INSERT INTO Adobe_libraryImageDevelopHistoryStep VALUES (1, {t}, 1, 'Exposure', X'00000010DEADBEEF00'), (2, {t}, 1, 'Short', X'01'),
 (3, 'not a date', 1, 'Odd date', {ok}), (4, {t}, 2, 'Bad Lua', {bad});
INSERT INTO Adobe_libraryImageDevelopSnapshot VALUES (1, 1, 'Broken', X'0000'), (2, 999, 'Nobody', 's = {{ }}');",
            t = when(1.0),
            ok = blob("s = { Exposure2012 = 0.25 }"),
            bad = blob("s = { Exposure2012 = "),
        ),
    );
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r =
        s.execute("library.migrateLightroom", &json!({"catalog": db.to_string_lossy(), "presets": false, "scratch": dir.to_string_lossy()})).unwrap();
    let h = &r["lightroomHistory"];
    assert_eq!(h["history"]["unreadable"], json!(2), "{r}");
    assert_eq!(h["history"]["unmapped"], json!(1), "{r}");
    assert_eq!(h["snapshots"]["notInLibrary"], json!(1), "{r}");
    let a = s.catalog.photos().find(|p| p.file_name.starts_with('a')).unwrap().id;
    let labels = lightroom_labels(&mut s, a.0);
    assert_eq!(labels.first().map(String::as_str), Some("Odd date"), "{labels:?}");
    // a damaged history file on disk reads as none
    let path = crate::lr_history::history_path(&s, a).unwrap();
    std::fs::write(&path, b"{not json").unwrap();
    assert!(lightroom_labels(&mut s, a.0).is_empty());
    // a file that isn't a catalog
    let junk = dir.join("junk.lrcat");
    std::fs::write(&junk, b"SQLite format 3\0 garbage garbage").unwrap();
    assert!(s.execute("library.migrateLightroomHistory", &json!({"catalog": junk.to_string_lossy(), "scratch": dir.to_string_lossy()})).is_err());
    assert!(s.execute("library.migrateLightroomHistory", &json!({"catalog": dir.join("none.lrcat").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Steps are stored as changes and rebuild exactly; a change JSON can't say (a removed field) is
/// stored whole.
#[test]
fn steps_rebuild_exactly() {
    use lightcraft_develop::DevelopSettings;
    let a = DevelopSettings::default();
    let mut b = a.clone();
    b.light.exposure = 1.25;
    b.spots.push(lightcraft_develop::Spot::default());
    let mut c = b.clone();
    c.spots.clear();
    c.light.contrast = -30.0;
    let st = encode_steps(&[("a".into(), None, a.clone()), ("b".into(), None, b.clone()), ("c".into(), None, c.clone())]);
    let h = LrHistory { steps: st, ..Default::default() };
    assert_eq!(h.settings(0).unwrap(), a);
    assert_eq!(h.settings(1).unwrap(), b);
    assert_eq!(h.settings(2).unwrap(), c);
    assert!(h.settings(3).is_none());
    // the default first step is tiny
    assert_eq!(h.steps[0].delta, Some(json!({})));
    // hostile bytes
    assert!(decode_text(&[]).is_err());
    assert!(decode_text(&[0, 0, 0, 9, 1, 2, 3]).is_err());
    assert_eq!(decode_text(b"s = { }").unwrap(), "s = { }");
}
