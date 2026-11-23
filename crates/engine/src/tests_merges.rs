//! merges. through the command layer: brackets and panorama frames on disk → found as groups →
//! the merges. view → merged: the frames hidden (never deleted), the merged photo in its folder,
//! its collections and merges., one record everywhere (edits shared); skip., unmerge., both. /
//! merges.only.

use lightcraft_catalog::{Album, Op, PhotoId};
use serde_json::{Value, json};

use crate::{LibrarySource, Session};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-merges-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn import(s: &mut Session, paths: Vec<String>) -> Vec<PhotoId> {
    let r = s.execute("library.import", &json!({"paths": paths})).unwrap();
    r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect()
}

/// What a camera would have written: capture time, exposure, one body and lens.
fn exif(s: &mut Session, id: PhotoId, t: &str, shutter: &str, bias: Option<f32>) {
    let mut m = s.catalog.photo(id).unwrap().meta.clone();
    m.camera = "LEICA M (Typ 262)".into();
    m.lens = "Summicron 35".into();
    m.focal_mm = Some(35.0);
    m.aperture = Some(8.0);
    m.iso = Some(200);
    m.shutter = shutter.into();
    m.exposure_bias = bias;
    s.catalog.apply(Op::SetMeta { id, meta: Box::new(m) }).unwrap();
    s.catalog.apply(Op::SetCaptured { id, captured: Some(t.into()) }).unwrap();
}

fn ids_of(v: &Value) -> Vec<PhotoId> {
    v.as_array().unwrap().iter().map(|x| PhotoId(x.as_u64().unwrap())).collect()
}

/// Three brackets and an unrelated photo in one folder, two of the brackets in a collection.
fn brackets(tag: &str) -> (Session, std::path::PathBuf, Vec<PhotoId>, PhotoId, lightcraft_catalog::AlbumId) {
    let dir = temp_dir(tag);
    let mut paths = Vec::new();
    for (i, b) in lightcraft_merge::synth::bracket_dngs(240, 160, &[-2.0, 0.0, 2.0]).unwrap().into_iter().enumerate() {
        let p = dir.join(format!("IMG_{i}.dng"));
        std::fs::write(&p, b).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let mut s = Session::new().with_fs();
    let ids = import(&mut s, paths);
    for (i, (id, (sh, b))) in ids.iter().zip([("1/500", -2.0), ("1/125", 0.0), ("1/30", 2.0)]).enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T09:30:0{i}"), sh, Some(b));
    }
    // a photo of something else an hour later
    let other_path = dir.join("IMG_9.dng");
    std::fs::write(&other_path, lightcraft_merge::synth::bracket_dngs(240, 160, &[0.0]).unwrap().remove(0)).unwrap();
    let other = import(&mut s, vec![other_path.to_string_lossy().to_string()])[0];
    exif(&mut s, other, "2026-11-20T10:30:00", "1/125", Some(0.0));
    let album = s.catalog.alloc_album_id();
    let mut a = Album::new(album, "Sunrise");
    a.photos = vec![other, ids[0], ids[1]];
    s.catalog.apply(Op::AddAlbum { album: a }).unwrap();
    (s, dir, ids, other, album)
}

#[test]
fn brackets_are_found_merged_hidden_and_restored() {
    let (mut s, dir, ids, other, album) = brackets("flow");
    let folder = dir.to_string_lossy().to_string();
    // found
    let r = s.execute("merge.find", &json!({"path": folder})).unwrap();
    assert_eq!(r["found"], 1, "{r}");
    let g = &r["groups"][0];
    assert_eq!(g["kind"], "hdr");
    assert_eq!(g["tag"], "hdr.");
    assert_eq!(ids_of(&g["photos"]), ids);
    let gid = g["id"].as_str().unwrap().to_string();
    // looking again finds nothing new
    assert_eq!(s.execute("merge.find", &json!({"path": folder})).unwrap()["found"], 0);
    // the merges. view of the folder and of the collection: the group's frames
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert_eq!(s.source, LibrarySource::Merges);
    assert_eq!(s.visible_cloned(), ids);
    assert_eq!(s.source_total(), Some(3));
    s.execute("library.source", &json!({"kind": "merges", "id": album.0})).unwrap();
    assert_eq!(s.visible_cloned(), ids, "a collection holding some of the frames shows the group");
    let counts = crate::merge_find::counts(&s.catalog);
    assert_eq!(counts.folder(&folder, false), 1);
    assert_eq!(counts.album(album), 1);

    // merged
    let r = s.execute("merge.mergeGroups", &json!({"ids": [gid], "autoSettings": false})).unwrap();
    assert_eq!(r["failed"], json!([]), "{r}");
    let merged = PhotoId(r["merged"][0]["id"].as_u64().unwrap());
    let p = s.catalog.photo(merged).unwrap().clone();
    assert!(p.file_name.ends_with("-HDR.dng"), "{}", p.file_name);
    assert_eq!(p.merge_type(), Some(lightcraft_catalog::MergeType::Hdr));
    assert_eq!(p.captured.as_deref().map(|c| &c[..16]), Some("2026-11-20T09:30"), "the originals' place in capture order");
    // the frames: hidden from the folder, the collection and everything else; still in the
    // catalog, their files untouched
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": folder})).unwrap();
    let shown = s.visible_cloned();
    assert!(shown.contains(&merged) && shown.contains(&other), "{shown:?}");
    assert!(ids.iter().all(|i| !shown.contains(i)), "{shown:?}");
    for i in &ids {
        let f = s.catalog.photo(*i).unwrap();
        assert!(!f.deleted && f.merge_hidden());
        let lightcraft_catalog::Source::File { path } = &f.source else { panic!() };
        assert!(std::path::Path::new(path).exists());
    }
    s.execute("library.source", &json!({"kind": "all"})).unwrap();
    assert!(ids.iter().all(|i| !s.visible_cloned().contains(i)));
    s.execute("library.source", &json!({"kind": "album", "id": album.0})).unwrap();
    assert_eq!(s.visible_cloned().len(), 2, "the merged photo took the frames' place in the collection");
    assert_eq!(s.catalog.album(album).unwrap().photos, vec![other, merged, ids[0], ids[1]]);
    assert_eq!(s.catalog.album_count(album), 2);
    // merges.: the merged photo
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert_eq!(s.visible_cloned(), vec![merged]);
    let runs = crate::merge_find::view(&s.catalog, s.merges_scope.as_ref().unwrap());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].label, "merged.");

    // one record: an edit shows wherever it is
    s.selection = crate::Selection::single(merged);
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.75})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album.0})).unwrap();
    assert!(s.visible_cloned().contains(&merged));
    assert_eq!(s.develop_of(merged).unwrap().light.exposure, 0.75);

    // merges. only: gone from the folder, still in merges.
    s.execute("merge.showIn", &json!({"ids": [merged.0], "where": "merges"})).unwrap();
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": folder})).unwrap();
    assert_eq!(s.visible_cloned(), vec![other]);
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert_eq!(s.visible_cloned(), vec![merged]);
    s.execute("merge.showIn", &json!({"ids": [gid], "where": "both"})).unwrap();
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": folder})).unwrap();
    assert!(s.visible_cloned().contains(&merged));

    // unmerge.: the frames are back, the group is open again
    let r = s.execute("merge.unmerge", &json!({"ids": [merged.0]})).unwrap();
    assert_eq!(r["shown"], 3, "{r}");
    let shown = s.visible_cloned();
    assert!(ids.iter().all(|i| shown.contains(i)), "{shown:?}");
    let g = crate::merge_find::group(&s.catalog, &gid).unwrap();
    assert_eq!(g.state, crate::merge_find::GroupState::Open);
    // undo brings the merge back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(ids.iter().all(|i| s.catalog.photo(*i).unwrap().merge_hidden()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn skipped_groups_are_never_suggested_again_and_survive_a_save() {
    let (mut s, dir, ids, _, _) = brackets("skip");
    let folder = dir.to_string_lossy().to_string();
    let r = s.execute("merge.find", &json!({"path": folder})).unwrap();
    let gid = r["groups"][0]["id"].as_str().unwrap().to_string();
    s.execute("merge.setKind", &json!({"id": gid, "kind": "pano."})).unwrap();
    assert_eq!(crate::merge_find::group(&s.catalog, &gid).unwrap().kind, lightcraft_catalog::MergeType::Pano);
    assert!(s.execute("merge.setKind", &json!({"id": gid, "kind": "nope"})).is_err());
    s.execute("merge.dismiss", &json!({"ids": [gid]})).unwrap();
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert!(s.visible_cloned().is_empty());
    assert_eq!(s.execute("merge.find", &json!({"path": folder})).unwrap()["found"], 0);
    let all = s.execute("merge.candidates", &json!({"state": "all"})).unwrap();
    assert_eq!(all["groups"][0]["state"], "dismissed");
    assert_eq!(s.execute("merge.candidates", &json!({})).unwrap()["groups"], json!([]));
    // the records and the photos' marks go through the catalog's own format
    let text = serde_json::to_string(&s.catalog).unwrap();
    let back: lightcraft_catalog::Catalog = serde_json::from_str(&text).unwrap();
    assert_eq!(crate::merge_find::group(&back, &gid).unwrap().state, crate::merge_find::GroupState::Dismissed);
    let _ = ids;
    // bad parameters are errors, not panics
    assert!(s.execute("merge.dismiss", &json!({})).is_err());
    assert!(s.execute("merge.unmerge", &json!({"ids": "x"})).is_err());
    assert!(s.execute("merge.showIn", &json!({"ids": [], "where": "nowhere"})).is_err());
    assert!(s.execute("merge.find", &json!({"scope": {"kind": "nope"}})).is_err());
    assert_eq!(s.execute("merge.mergeGroups", &json!({"ids": ["0000"]})).unwrap()["failed"][0]["group"], "0000");
    assert!(s.execute("library.source", &json!({"kind": "merges", "id": 999})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn panorama_frames_are_found_from_their_pixels() {
    let dir = temp_dir("pano");
    let mut paths = Vec::new();
    for (i, v) in lightcraft_merge::synth::pano_views(480, 360, 420.0, &[-25.0, 0.0, 25.0]).unwrap().into_iter().enumerate() {
        let img = v.to_srgb8();
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        let p = dir.join(format!("P_{i}.png"));
        std::fs::write(&p, png).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let mut s = Session::new().with_fs();
    let ids = import(&mut s, paths);
    for (i, id) in ids.iter().enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T12:00:0{}", i * 2), "1/250", None);
    }
    let r = s.execute("merge.find", &json!({})).unwrap();
    assert_eq!(r["found"], 1, "{r}");
    assert_eq!(r["groups"][0]["kind"], "pano");
    assert_eq!(ids_of(&r["groups"][0]["photos"]), ids);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_automatic_finder_looks_once_per_change() {
    let (mut s, dir, _, _, _) = brackets("auto");
    s.vision.background = true;
    let r = s.merge_find_auto().expect("the first look").unwrap();
    // brackets need no pixels: done at once
    assert_eq!(r["found"], 1, "{r}");
    assert!(s.merge_find_auto().is_none(), "nothing changed since");
    // a new photo in the folder: looked at again
    let p = dir.join("IMG_10.dng");
    std::fs::write(&p, lightcraft_merge::synth::bracket_dngs(200, 120, &[1.0]).unwrap().remove(0)).unwrap();
    assert_eq!(import(&mut s, vec![p.to_string_lossy().to_string()]).len(), 1);
    let r = s.merge_find_auto().expect("looked again").unwrap();
    assert_eq!(r["found"], 0);
    let _ = std::fs::remove_dir_all(&dir);
}
