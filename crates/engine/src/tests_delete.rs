//! L03: Remove vs Delete from Disk (to the Trash), Delete Rejected Photos, virtual copies.

use std::path::{Path, PathBuf};

use lightcraft_catalog::{PhotoId, Source};
use serde_json::{Value, json};

use crate::Session;

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-delete-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_png(path: &Path, seed: u8) {
    let (w, h) = (24usize, 16usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A session with `n` imported PNGs in `dir/src`, its Trash in `dir/trash`.
fn setup(dir: &Path, n: u8) -> (Session, Vec<PhotoId>) {
    for i in 0..n {
        write_png(&dir.join(format!("src/p{i}.png")), i + 1);
    }
    let mut s = Session::new();
    s.trash_dir = Some(dir.join("trash"));
    s.execute("library.import", &json!({"paths": [dir.join("src").to_string_lossy()]})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort_by_key(|id| s.catalog.photo(*id).map(|p| p.file_name.clone()));
    (s, ids)
}

fn path(s: &Session, id: PhotoId) -> PathBuf {
    match &s.catalog.photo(id).unwrap().source {
        Source::File { path } => PathBuf::from(path),
        Source::Demo { .. } => PathBuf::new(),
    }
}

#[test]
fn remove_keeps_files_and_delete_from_disk_trashes_them() {
    let dir = temp_dir("both");
    let (mut s, ids) = setup(&dir, 3);
    let (a, b) = (path(&s, ids[0]), path(&s, ids[1]));
    std::fs::write(a.with_extension("xmp"), "<x:xmpmeta/>").unwrap();
    let r = s.execute("photo.removeFromLibrary", &json!({"ids": [ids[1].0]})).unwrap();
    assert_eq!(r["removed"], 1);
    assert!(b.exists(), "Remove leaves the file");
    assert!(s.catalog.photo(ids[1]).is_none());

    let r = s.execute("photo.deleteFromDisk", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!((r["removed"].as_u64(), r["failed"].as_array().map(Vec::len)), (Some(1), Some(0)), "{r}");
    assert!(!a.exists() && !a.with_extension("xmp").exists(), "original and sidecar trashed");
    assert!(dir.join("trash/p0.png").exists() && dir.join("trash/p0.xmp").exists());
    assert!(s.catalog.photo(ids[0]).is_none());
    // undo brings the record and the files back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.photo(ids[0]).is_some());
    assert!(a.exists() && a.with_extension("xmp").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⌘Z after a Delete from Disk moves every trashed file (originals and sidecars, a multi-photo
/// selection) back to its exact path; ⌘⇧Z trashes them again; undo again brings them back.
#[test]
fn undo_puts_trashed_files_back_and_redo_trashes_them_again() {
    let dir = temp_dir("undo");
    let (mut s, ids) = setup(&dir, 3);
    let (a, b, c) = (path(&s, ids[0]), path(&s, ids[1]), path(&s, ids[2]));
    std::fs::write(a.with_extension("xmp"), "<x:xmpmeta a/>").unwrap();
    let album = s.execute("album.create", &json!({"name": "Keepers"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0].0, ids[1].0]})).unwrap();
    let r = s.execute("photo.deleteFromDisk", &json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
    assert_eq!(r["removed"], 2, "{r}");
    assert_eq!(r["trashed"].as_array().unwrap().len(), 3, "two originals and a sidecar: {r}");
    assert!(!a.exists() && !b.exists() && !a.with_extension("xmp").exists() && c.exists());
    assert_eq!(s.undo.last().map(|e| e.trashed.len()), Some(3), "the undo step knows where they went");

    let r = s.execute("edit.undo", &json!({})).unwrap();
    assert!(r.get("notBack").is_none(), "{r}");
    assert!(a.exists() && b.exists(), "back at their exact paths");
    assert_eq!(std::fs::read_to_string(a.with_extension("xmp")).unwrap(), "<x:xmpmeta a/>", "the sidecar too");
    assert!(!dir.join("trash/p0.png").exists() && !dir.join("trash/p1.png").exists(), "out of the Trash");
    assert!(s.catalog.photo(ids[0]).is_some() && s.catalog.photo(ids[1]).is_some());
    assert_eq!(s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().photos.len(), 2, "album membership back");
    assert!(s.last_delete.is_none(), "nothing left for Put Back Last Delete");

    s.execute("edit.redo", &json!({})).unwrap();
    assert!(!a.exists() && !b.exists() && !a.with_extension("xmp").exists(), "redo trashes them again");
    assert!(dir.join("trash/p0.png").exists() && dir.join("trash/p0.xmp").exists());
    assert!(s.catalog.photo(ids[0]).is_none());
    assert_eq!(s.last_delete.as_ref().map(|l| l.photos.len()), Some(2));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(a.exists() && b.exists() && a.with_extension("xmp").exists(), "and back once more");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file emptied from the Trash, or whose original path is taken now, can't come back: the photo
/// returns anyway, nothing is overwritten, and the undo says which files stayed out.
#[test]
fn undo_after_the_trash_was_emptied_or_the_path_taken() {
    let dir = temp_dir("undo-gone");
    let (mut s, ids) = setup(&dir, 2);
    let (a, b) = (path(&s, ids[0]), path(&s, ids[1]));
    s.execute("photo.deleteFromDisk", &json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
    // p0 emptied from the Trash; a new file now sits at p1's path
    std::fs::remove_file(dir.join("trash/p0.png")).unwrap();
    std::fs::write(&b, b"someone else's file").unwrap();
    let r = s.execute("edit.undo", &json!({})).unwrap();
    let not_back = r["notBack"].as_array().unwrap();
    assert_eq!(not_back.len(), 2, "{r}");
    assert!(not_back[0].as_str().unwrap().contains("no longer in the Trash"), "{r}");
    assert!(not_back[1].as_str().unwrap().contains("another file is there"), "{r}");
    assert!(s.catalog.photo(ids[0]).is_some() && s.catalog.photo(ids[1]).is_some(), "the photos return anyway");
    assert!(!a.exists());
    assert_eq!(std::fs::read(&b).unwrap(), b"someone else's file", "never overwritten");
    assert!(dir.join("trash/p1.png").exists(), "p1 stays in the Trash");
    // redo: only files that came back go to the Trash again; the newcomer at p1's path stays
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(b.exists() && s.catalog.photo(ids[1]).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Delete Rejected Photos (from disk) undoes the same way.
#[test]
fn undo_of_delete_rejected_puts_the_files_back() {
    let dir = temp_dir("undo-rejected");
    let (mut s, ids) = setup(&dir, 3);
    s.execute("photo.flag", &json!({"ids": [ids[1].0], "flag": "reject"})).unwrap();
    s.execute("library.deleteRejected", &json!({"fromDisk": true})).unwrap();
    assert!(!path_exists(&dir, "src/p1.png"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(path_exists(&dir, "src/p1.png") && s.catalog.photo(ids[1]).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Put Back Last Delete: after the undo history moved on, and after the library is reopened (the
/// undo history is gone then), the last delete's files and photos come back.
#[test]
fn put_back_last_delete_works_after_history_moved_on_and_a_reopen() {
    let dir = temp_dir("putback");
    let (_memory, _) = setup(&dir, 2);
    let mut s = Session::new();
    s.trash_dir = Some(dir.join("trash"));
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.import", &json!({"paths": [dir.join("src").to_string_lossy()]})).unwrap();
    let id = s.catalog.photos().find(|p| p.file_name == "p0.png").unwrap().id;
    let album = s.execute("album.create", &json!({"name": "Best"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [id.0]})).unwrap();
    assert!(s.execute("photo.putBackLastDelete", &json!({})).is_err(), "nothing to put back yet");
    s.execute("photo.deleteFromDisk", &json!({"ids": [id.0]})).unwrap();
    assert!(!path_exists(&dir, "src/p0.png"));
    s.close_library().unwrap();
    drop(s);

    let mut s = Session::new();
    s.open_library(dir.join("lib"), false).unwrap();
    assert!(s.undo.is_empty() && s.catalog.photo(id).is_none());
    let r = s.execute("photo.putBackLastDelete", &json!({})).unwrap();
    assert_eq!(r["photos"], 1, "{r}");
    assert!(path_exists(&dir, "src/p0.png") && !path_exists(&dir, "trash/p0.png"));
    assert!(s.catalog.photo(id).is_some());
    assert!(s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().photos.contains(&id), "back in its album");
    assert!(s.execute("photo.putBackLastDelete", &json!({})).is_err(), "only once");
    s.close_library().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Delete key ▸ album.: in an album the photo leaves only that album; elsewhere it leaves the
/// library (files kept).
#[test]
fn remove_from_album_only_in_albums_and_from_the_library_elsewhere() {
    let dir = temp_dir("album");
    let (mut s, ids) = setup(&dir, 2);
    let album = s.execute("album.create", &json!({"name": "Trip"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0].0, ids[1].0]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let r = s.execute("photo.removeFromAlbum", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!((r["removed"].as_u64(), r["fromLibrary"].as_bool()), (Some(1), None), "{r}");
    assert_eq!(s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().photos, vec![ids[1]]);
    assert!(s.catalog.photo(ids[0]).is_some() && path(&s, ids[0]).exists(), "library and file untouched");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().photos.len(), 2);
    // All Photographs: out of the library, the file stays
    s.execute("library.source", &json!({"kind": "all"})).unwrap();
    let r = s.execute("photo.removeFromAlbum", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!(r["fromLibrary"], true, "{r}");
    assert!(s.catalog.photo(ids[0]).is_none() && dir.join("src/p0.png").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_photo_whose_file_cant_be_trashed_stays() {
    let dir = temp_dir("fail");
    let (mut s, ids) = setup(&dir, 1);
    // the "Trash" is a file, so nothing can be moved into it
    std::fs::write(dir.join("trash"), b"in the way").unwrap();
    let r = s.execute("photo.deleteFromDisk", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!(r["removed"], 0, "{r}");
    assert_eq!(r["failed"].as_array().unwrap().len(), 1);
    assert!(s.catalog.photo(ids[0]).is_some() && path(&s, ids[0]).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn virtual_copies_go_with_their_master_and_alone_only_lose_the_record() {
    let dir = temp_dir("vc");
    let (mut s, ids) = setup(&dir, 1);
    let master = ids[0];
    s.execute("photo.virtualCopy", &json!({"ids": [master.0]})).unwrap();
    let vc = s.catalog.photos().find(|p| p.copy_of == Some(master)).unwrap().id;
    // a virtual copy alone: its record goes, the master's file stays
    let r = s.execute("photo.deleteFromDisk", &json!({"ids": [vc.0]})).unwrap();
    assert_eq!(r["removed"], 1, "{r}");
    assert!(r["trashed"].as_array().unwrap().is_empty());
    assert!(path(&s, master).exists() && s.catalog.photo(master).is_some());
    // the master takes its copies with it
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.execute("photo.removeFromLibrary", &json!({"ids": [master.0]})).unwrap();
    assert_eq!(r["removed"], 2);
    assert!(s.catalog.photos().next().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn delete_rejected_acts_on_the_rejects_in_view() {
    let dir = temp_dir("rejected");
    let (mut s, ids) = setup(&dir, 4);
    for id in [ids[1], ids[3]] {
        s.execute("photo.flag", &json!({"ids": [id.0], "flag": "reject"})).unwrap();
    }
    let r = s.execute("library.rejected", &json!({})).unwrap();
    assert_eq!(r["count"], 2, "{r}");
    let r: Value = s.execute("library.deleteRejected", &json!({"fromDisk": true})).unwrap();
    assert_eq!(r["removed"], 2, "{r}");
    assert!(!path_exists(&dir, "src/p1.png") && !path_exists(&dir, "src/p3.png"));
    assert!(path_exists(&dir, "src/p0.png") && path_exists(&dir, "trash/p1.png"));
    assert_eq!(s.catalog.len(), 2);
    assert_eq!(s.execute("library.deleteRejected", &json!({})).unwrap()["removed"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

fn path_exists(dir: &Path, rel: &str) -> bool {
    dir.join(rel).exists()
}

#[test]
fn a_shared_stem_sidecar_stays_while_its_other_photo_does() {
    let dir = temp_dir("stem");
    write_png(&dir.join("src/IMG_1.png"), 1);
    let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[90, 120, 150, 255]; 64] };
    let tif = lightcraft_codecs::encode_tiff(&lightcraft_codecs::EncodeImage::rgba8(&img), Default::default(), &Default::default()).unwrap();
    std::fs::write(dir.join("src/IMG_1.tif"), tif).unwrap();
    let (mut s, ids) = setup(&dir, 0);
    assert_eq!(ids.len(), 2);
    std::fs::write(dir.join("src/IMG_1.xmp"), "<x:xmpmeta/>").unwrap();
    let png = s.catalog.photos().find(|p| p.file_name == "IMG_1.png").unwrap().id;
    let tif = s.catalog.photos().find(|p| p.file_name == "IMG_1.tif").unwrap().id;
    s.execute("photo.deleteFromDisk", &json!({"ids": [png.0]})).unwrap();
    assert!(!dir.join("src/IMG_1.png").exists());
    assert!(dir.join("src/IMG_1.xmp").exists(), "IMG_1.tif still uses it");
    s.execute("photo.deleteFromDisk", &json!({"ids": [tif.0]})).unwrap();
    assert!(!dir.join("src/IMG_1.xmp").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// L30: Import from Another Library — photos, virtual copies and albums come over; a second run
/// skips what's there, or with `replace` takes the other library's settings.
#[test]
fn import_from_another_library() {
    let dir = temp_dir("import-catalog");
    // two PNGs in dir/src
    let (_memory, _) = setup(&dir, 2);
    let mut a = Session::new();
    a.open_library(dir.join("libA"), false).unwrap();
    a.execute("library.import", &json!({"paths": [dir.join("src").to_string_lossy()]})).unwrap();
    let first = a.catalog.photos().next().unwrap().id;
    a.execute("photo.rate", &json!({"ids": [first.0], "rating": 5})).unwrap();
    a.execute("photo.virtualCopy", &json!({"ids": [first.0]})).unwrap();
    a.execute("album.create", &json!({"name": "Best", "addSelected": true})).unwrap();
    a.close_library().unwrap();
    drop(a);

    let mut b = Session::new();
    b.open_library(dir.join("libB"), false).unwrap();
    let r = b.execute("library.importCatalog", &json!({"path": dir.join("libA").to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["albums"].as_u64()), (Some(3), Some(1)), "{r}");
    assert!(b.catalog.photos().any(|p| p.copy_of.is_some()), "the virtual copy follows its original");
    assert!(b.catalog.photos().any(|p| p.rating == 5));
    // again: nothing new; with replace the ratings come over
    let r = b.execute("library.importCatalog", &json!({"path": dir.join("libA").to_string_lossy()})).unwrap();
    assert_eq!(r["added"], 0, "{r}");
    let some = b.catalog.photos().find(|p| p.rating == 5 && p.copy_of.is_none()).unwrap().id;
    b.selection = crate::Selection::single(some);
    b.execute("photo.rate", &json!({"ids": [some.0], "rating": 1})).unwrap();
    let r = b.execute("library.importCatalog", &json!({"path": dir.join("libA").to_string_lossy(), "replace": true})).unwrap();
    assert_eq!(r["replaced"], 2, "{r}");
    assert_eq!(b.catalog.photo(some).unwrap().rating, 5);
    assert!(b.execute("library.importCatalog", &json!({"path": dir.join("nothing").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
