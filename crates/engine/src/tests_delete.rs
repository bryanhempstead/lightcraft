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
    // undo brings the record back (the file stays in the Trash: Put Back from there)
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.photo(ids[0]).is_some());
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
