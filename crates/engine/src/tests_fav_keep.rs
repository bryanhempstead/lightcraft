//! Favorites kept safe (`fav_keep`): a heart copies the original + a sidecar with its edits to the
//! keep folder; offline → copied when it appears; original gone → the photo shows the kept copy
//! with its edits; never overwrites; unheart keeps the copy; `fav.forget` / delete remove it.
//! Every keep folder here is a temp dir (the engine's default is off).

use std::path::{Path, PathBuf};

use lightcraft_catalog::{PhotoId, Source};
use serde_json::json;

use crate::Session;
use crate::fav_keep::{self, kept};

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-favkeep-{tag}-{}", std::process::id()));
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

/// `n` PNGs in `dir/Erika wedding/raw/`, imported; the keep folder `root`, Trash in `dir/trash`.
fn setup(dir: &Path, n: u8, root: &Path) -> (Session, Vec<PhotoId>) {
    for i in 0..n {
        write_png(&dir.join(format!("Erika wedding/raw/p{i}.png")), i + 1);
    }
    let mut s = Session::new();
    s.trash_dir = Some(dir.join("trash"));
    s.execute("library.import", &json!({"paths": [dir.join("Erika wedding").to_string_lossy()]})).unwrap();
    let mut ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).collect();
    ids.sort_by_key(|id| s.catalog.photo(*id).map(|p| p.file_name.clone()));
    s.set_fav_keep_root(Some(root.to_path_buf()));
    (s, ids)
}

fn path(s: &Session, id: PhotoId) -> PathBuf {
    match &s.catalog.photo(id).unwrap().source {
        Source::File { path } => PathBuf::from(path),
        Source::Demo { .. } => PathBuf::new(),
    }
}

fn heart(s: &mut Session, id: PhotoId) {
    s.execute("photo.favorite", &json!({"favorite": true, "ids": [id.0]})).unwrap();
}

/// What a background-free poll does after the debounce: run it as the next full pass.
fn pass(s: &mut Session) {
    s.execute("fav.keep", &json!({})).unwrap();
}

#[test]
fn a_heart_copies_the_original_and_a_sidecar_with_its_edits() {
    let dir = temp_dir("heart");
    let root = dir.join("LOT48/LightCraft Favorites");
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    let (mut s, ids) = setup(&dir, 2, &root);
    s.execute("develop.set", &json!({"ids": [ids[0].0], "values": {"light.exposure": 0.75}})).unwrap();
    heart(&mut s, ids[0]);
    let r = s.execute("fav.keep", &json!({})).unwrap();
    assert_eq!(r["kept"], 1, "{r}");
    let k = kept(&s.catalog, ids[0]).expect("a kept record");
    // the shoot folder, not `raw`
    assert_eq!(PathBuf::from(&k.path), root.join("Erika wedding/p0.png"), "{k:?}");
    assert_eq!(std::fs::read(&k.path).unwrap(), std::fs::read(path(&s, ids[0])).unwrap(), "the same bytes");
    let sc = std::fs::read_to_string(k.sidecar.as_deref().expect("a sidecar")).unwrap();
    assert!(sc.contains("Exposure2012") || sc.contains("exposure"), "the edits are in the kept sidecar: {sc}");
    assert_eq!(k.from, path(&s, ids[0]).to_string_lossy());
    assert!(kept(&s.catalog, ids[1]).is_none(), "an unhearted photo isn't kept");
    let st = s.fav_keep_status();
    assert_eq!((st.favorites, st.kept, st.waiting), (1, 1, 0), "{st:?}");
    assert!(st.disk.ends_with("LightCraft Favorites"), "outside /Volumes the disk is the folder: {st:?}");
    // an edit later writes the kept sidecar again
    s.execute("develop.set", &json!({"ids": [ids[0].0], "values": {"light.contrast": 33}})).unwrap();
    s.fav_keep.refresh_due_now_for_tests();
    s.poll_fav_keep();
    let again = kept(&s.catalog, ids[0]).unwrap();
    assert_ne!(again.packet, k.packet, "sidecar written again with the new edit");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn background_copy_lands_off_the_calling_thread() {
    let dir = temp_dir("bg");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 1, &root);
    s.fav_keep.background = true;
    heart(&mut s, ids[0]);
    // the first poll is the full pass: queued on the worker
    s.poll_fav_keep();
    for _ in 0..400 {
        s.poll_fav_keep();
        if kept(&s.catalog, ids[0]).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let k = kept(&s.catalog, ids[0]).expect("kept by the worker");
    assert!(Path::new(&k.path).is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn offline_keep_folder_queues_then_copies_when_it_appears() {
    let dir = temp_dir("offline");
    // the disk the keep folder goes on isn't there
    let root = dir.join("LOT48/LightCraft Favorites");
    let (mut s, ids) = setup(&dir, 2, &root);
    heart(&mut s, ids[0]);
    heart(&mut s, ids[1]);
    let r = s.execute("fav.keep", &json!({})).unwrap();
    assert_eq!((r["kept"].as_u64(), r["offline"].as_bool()), (Some(0), Some(true)), "{r}");
    assert!(!dir.join("LOT48").exists(), "the disk's own folder is never made");
    let st = s.fav_keep_status();
    assert_eq!((st.waiting, st.online), (2, Some(false)), "{st:?}");
    // a poll before the retry time does nothing
    s.poll_fav_keep();
    assert!(kept(&s.catalog, ids[0]).is_none());
    // LOT48 is plugged in
    std::fs::create_dir_all(dir.join("LOT48")).unwrap();
    s.fav_keep.full_pass_now_for_tests();
    s.poll_fav_keep();
    assert!(kept(&s.catalog, ids[0]).is_some() && kept(&s.catalog, ids[1]).is_some());
    assert!(root.join("Erika wedding/p1.png").is_file());
    assert_eq!(s.fav_keep_status().online, Some(true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_crash_before_the_copy_leaves_the_heart_as_the_queue() {
    let dir = temp_dir("crash");
    let lib = dir.join("lib");
    let root = dir.join("keep");
    write_png(&dir.join("Shoot/a.png"), 3);
    {
        let mut s = Session::new();
        s.open_library(&lib, false).unwrap();
        s.execute("library.import", &json!({"paths": [dir.join("Shoot").to_string_lossy()]})).unwrap();
        let id = s.catalog.photos().next().unwrap().id;
        // hearted with no keep folder set (or the app quit before the copy ran)
        heart(&mut s, id);
        s.close_library().unwrap();
    }
    let mut s = Session::new();
    s.open_library(&lib, false).unwrap();
    s.set_fav_keep_root(Some(root.clone()));
    s.poll_fav_keep();
    let id = s.catalog.photos().next().unwrap().id;
    let k = kept(&s.catalog, id).expect("kept on the next run");
    assert!(root.join("Shoot/a.png").is_file(), "{k:?}");
    // and the record is journaled: a reopen still knows where the copy is
    s.close_library().unwrap();
    drop(s);
    let mut s2 = Session::new();
    s2.open_library(&lib, false).unwrap();
    assert_eq!(kept(&s2.catalog, id), Some(k));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_deleted_folder_leaves_the_photo_on_its_kept_copy_with_its_edits() {
    let dir = temp_dir("gone");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 1, &root);
    s.execute("develop.set", &json!({"ids": [ids[0].0], "values": {"light.exposure": 1.25}})).unwrap();
    heart(&mut s, ids[0]);
    pass(&mut s);
    let k = kept(&s.catalog, ids[0]).unwrap();
    let original = path(&s, ids[0]);
    std::fs::remove_dir_all(dir.join("Erika wedding")).unwrap();
    s.fav_keep.full_pass_now_for_tests();
    s.poll_fav_keep();
    assert_eq!(path(&s, ids[0]), PathBuf::from(&k.path), "the photo shows its kept copy");
    assert!(fav_keep::uses_kept(&s.catalog, ids[0]));
    let p = s.catalog.photo(ids[0]).unwrap();
    assert!(p.favorite && p.in_library(), "still in the library, still hearted");
    assert!((p.develop.light.exposure - 1.25).abs() < 1e-9, "edits stay");
    assert!(crate::cmd::missing::missing(&s).is_empty(), "not a missing photo");
    assert_eq!(s.fav_keep_status().using_kept, 1);
    // forget skips it: the kept copy is its only file
    let r = s.execute("fav.forget", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!(r["forgot"], 0, "{r}");
    assert!(Path::new(&k.path).is_file());
    // the folder comes back (a drive plugged in again): back on the original
    write_png(&original, 1);
    s.fav_keep.full_pass_now_for_tests();
    s.poll_fav_keep();
    assert_eq!(path(&s, ids[0]), original, "back on its original");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_missing_folder_brings_a_kept_photo_back_to_its_moved_original() {
    let dir = temp_dir("locate");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 1, &root);
    heart(&mut s, ids[0]);
    pass(&mut s);
    let from = dir.join("Erika wedding");
    std::fs::rename(&from, dir.join("Moved")).unwrap();
    s.fav_keep.full_pass_now_for_tests();
    s.poll_fav_keep();
    assert!(fav_keep::uses_kept(&s.catalog, ids[0]));
    let r = s.execute("folder.locate", &json!({"path": from.to_string_lossy(), "to": dir.join("Moved").to_string_lossy()})).unwrap();
    assert_eq!(r["relinked"], 1, "{r}");
    assert_eq!(path(&s, ids[0]), dir.join("Moved/raw/p0.png"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn never_overwrites_a_different_file_and_reuses_the_same_one() {
    let dir = temp_dir("names");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 2, &root);
    // someone else's p0.png is in the shoot folder already
    std::fs::create_dir_all(root.join("Erika wedding")).unwrap();
    std::fs::write(root.join("Erika wedding/p0.png"), b"not this photo").unwrap();
    // and p1.png is there with exactly the photo's bytes (copied before a crash)
    std::fs::copy(path(&s, ids[1]), root.join("Erika wedding/p1.png")).unwrap();
    heart(&mut s, ids[0]);
    heart(&mut s, ids[1]);
    pass(&mut s);
    assert_eq!(std::fs::read(root.join("Erika wedding/p0.png")).unwrap(), b"not this photo", "never overwritten");
    assert_eq!(PathBuf::from(kept(&s.catalog, ids[0]).unwrap().path), root.join("Erika wedding/p0-1.png"));
    assert_eq!(PathBuf::from(kept(&s.catalog, ids[1]).unwrap().path), root.join("Erika wedding/p1.png"), "same bytes reused");
    assert!(!root.join("Erika wedding/p1-1.png").exists());
    let leftovers: Vec<_> = std::fs::read_dir(root.join("Erika wedding"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with("lightcraft-partial"))
        .collect();
    assert!(leftovers.is_empty(), "no temporary files left");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unheart_keeps_the_copy_and_forget_takes_it_to_the_trash() {
    let dir = temp_dir("forget");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 1, &root);
    heart(&mut s, ids[0]);
    pass(&mut s);
    let k = kept(&s.catalog, ids[0]).unwrap();
    s.execute("photo.favorite", &json!({"favorite": false, "ids": [ids[0].0]})).unwrap();
    pass(&mut s);
    assert!(Path::new(&k.path).is_file() && kept(&s.catalog, ids[0]).is_some(), "unhearting keeps the copy");
    // hearted again and forgotten: gone to the Trash, not kept again while hearted
    heart(&mut s, ids[0]);
    let r = s.execute("fav.forget", &json!({"ids": [ids[0].0]})).unwrap();
    assert_eq!(r["forgot"], 1, "{r}");
    assert!(!Path::new(&k.path).exists() && !Path::new(k.sidecar.as_deref().unwrap()).exists());
    assert!(dir.join("trash/p0.png").exists(), "to the Trash, not erased");
    pass(&mut s);
    s.fav_keep.full_pass_now_for_tests();
    s.poll_fav_keep();
    assert!(kept(&s.catalog, ids[0]).is_none(), "forgotten stays forgotten");
    // unheart + heart again: kept again
    s.execute("photo.favorite", &json!({"favorite": false, "ids": [ids[0].0]})).unwrap();
    heart(&mut s, ids[0]);
    pass(&mut s);
    assert!(kept(&s.catalog, ids[0]).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deleting_a_photo_keeps_its_copy_unless_told() {
    let dir = temp_dir("delete");
    let root = dir.join("keep");
    let (mut s, ids) = setup(&dir, 2, &root);
    heart(&mut s, ids[0]);
    heart(&mut s, ids[1]);
    pass(&mut s);
    let (k0, k1) = (kept(&s.catalog, ids[0]).unwrap(), kept(&s.catalog, ids[1]).unwrap());
    // the default: the original goes, the kept copy stays
    s.execute("photo.deleteFromDisk", &json!({"ids": [ids[0].0]})).unwrap();
    assert!(s.catalog.photo(ids[0]).is_none() && Path::new(&k0.path).is_file());
    // told to: the kept copy and its sidecar go to the Trash too; undo brings everything back
    let r = s.execute("photo.removeFromLibrary", &json!({"ids": [ids[1].0], "keptCopies": true})).unwrap();
    assert_eq!(r["removed"], 1);
    assert!(!Path::new(&k1.path).exists(), "kept copy trashed");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Path::new(&k1.path).is_file(), "undo puts the kept copy back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shoot_names_skip_camera_and_format_folders() {
    assert_eq!(fav_keep::group_name("/Volumes/X/Erika and Connor wedding/raw/M262/L1000001.DNG"), "Erika and Connor wedding");
    assert_eq!(fav_keep::group_name("/Volumes/X/Iceland/IMG_1.CR3"), "Iceland");
    // nothing meaningful above it on the disk: the folder it is in
    assert_eq!(fav_keep::group_name("/Volumes/X/DCIM/100CANON/IMG_1.CR3"), "100CANON");
    assert_eq!(fav_keep::disk_name(Path::new("/Volumes/LOT48/LightCraft Favorites")), "LOT48");
}

#[test]
fn off_by_default_nothing_is_written() {
    let mut s = Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id;
    heart(&mut s, id);
    s.poll_fav_keep();
    assert!(s.execute("fav.keep", &json!({})).is_err(), "no keep folder: an error, nothing copied");
    assert!(!s.fav_keep_status().on);
}
