//! merges. through the command layer: brackets and panorama frames on disk → found as groups →
//! the merges. view → merged: the frames hidden (never deleted), the merged photo in its folder,
//! its collections and merges., one record everywhere (edits shared); skip., unmerge., both. /
//! merges.only.

use lightcraft_catalog::{Album, Op, PhotoId};
use serde_json::{Value, json};

use crate::{LibrarySource, Session};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-merges-wedding-{tag}-{}", std::process::id()));
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

/// Panorama frames as PNGs in `dir` (`yaws` degrees apart), each `ev` stops brighter.
fn pano_files(dir: &std::path::Path, prefix: &str, yaws: &[f64], evs: &[f32]) -> Vec<String> {
    let mut paths = Vec::new();
    for (i, v) in lightcraft_merge::synth::pano_views(480, 360, 420.0, yaws).unwrap().into_iter().enumerate() {
        let k = 2f32.powf(evs.get(i).copied().unwrap_or(0.0));
        let v = lightcraft_raster::Rgb32f::from_fn(v.width, v.height, |x, y| v.data[y * v.width + x].map(|c| (c * k).min(1.0)));
        let img = v.to_srgb8();
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        let p = dir.join(format!("{prefix}{i}.png"));
        std::fs::write(&p, png).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    paths
}

/// Bryan shoots handheld panoramas in aperture priority: the shutter drifts between frames. They
/// are found as one panorama (not an HDR), with its confidence, its reasons and its frames.
#[test]
fn a_drifting_handheld_panorama_is_a_panorama_not_an_hdr() {
    let dir = temp_dir("drift");
    let mut s = Session::new().with_fs();
    // the shutter steps a whole stop per frame: in the metadata alone, a bracket
    let shutters = ["1/500", "1/250", "1/125"];
    let ids = import(&mut s, pano_files(&dir, "D_", &[-25.0, 0.0, 25.0], &[-1.0, 0.0, 1.0]));
    for (i, id) in ids.iter().enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T12:00:0{}", i * 2), shutters[i], None);
    }
    let r = s.execute("merge.find", &json!({"path": dir.to_string_lossy()})).unwrap();
    assert_eq!(r["found"], 1, "{r}");
    let g = &r["groups"][0];
    assert_eq!(g["kind"], "pano", "{g}");
    assert_eq!(g["tag"], "panorama.");
    assert_eq!(ids_of(&g["photos"]), ids);
    assert!(["sure", "likely", "maybe"].contains(&g["confidence"].as_str().unwrap()), "{g}");
    let reasons: Vec<&str> = g["reasons"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(reasons.iter().any(|r| r.starts_with("3 frames in 4 s")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.starts_with("each frame overlaps the last by ~")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.starts_with("exposure drifts by")), "{reasons:?}");
    // a small drift (1/250, 1/320, 1/200) too
    let dir2 = temp_dir("drift2");
    let ids2 = import(&mut s, pano_files(&dir2, "E_", &[-20.0, 0.0, 20.0], &[0.0, -0.33, 0.33]));
    for (i, (id, sh)) in ids2.iter().zip(["1/250", "1/320", "1/200"]).enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T13:00:0{}", i * 2), sh, None);
    }
    let r = s.execute("merge.find", &json!({"path": dir2.to_string_lossy()})).unwrap();
    assert_eq!(r["found"], 1, "{r}");
    assert_eq!(r["groups"][0]["kind"], "pano");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// A bracket at one framing whose camera stepped only the shutter (manual mode) is an HDR; a burst
/// (same framing, same exposure) is nothing.
#[test]
fn a_shutter_bracket_is_hdr_and_a_burst_is_nothing() {
    let (mut s, dir, ids, _, _) = brackets("shutter");
    // no exposure compensation recorded: the pixels say it is one framing
    for (i, (id, sh)) in ids.iter().zip(["1/500", "1/125", "1/30"]).enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T09:30:0{i}"), sh, None);
    }
    let r = s.execute("merge.find", &json!({"path": dir.to_string_lossy()})).unwrap();
    assert_eq!(r["found"], 1, "{r}");
    assert_eq!(r["groups"][0]["kind"], "hdr");
    assert_eq!(r["groups"][0]["confidence"], "sure", "{r}");
    // a burst: the same picture three times at one exposure
    let burst = temp_dir("burst");
    let mut paths = Vec::new();
    for i in 0..3 {
        let p = burst.join(format!("B_{i}.dng"));
        std::fs::write(&p, lightcraft_merge::synth::bracket_dngs(240, 160, &[0.0]).unwrap().remove(0)).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let bids = import(&mut s, paths);
    for (i, id) in bids.iter().enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T15:00:0{i}"), "1/250", None);
    }
    let r = s.execute("merge.find", &json!({"path": burst.to_string_lossy()})).unwrap();
    assert_eq!(r["found"], 0, "a burst is not a merge: {r}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&burst);
}

/// A shoot folder with camera subfolders is one pool: found together, listed together, counted
/// together; a group never mixes two cameras, and each says which camera and subfolder it is from.
#[test]
fn a_shoot_folder_with_camera_subfolders_is_one_pool() {
    let shoot = temp_dir("pool");
    let mut s = Session::new().with_fs();
    let mut by_cam = Vec::new();
    for (cam, sub, w) in [("LEICA M (Typ 262)", "M262", 240), ("RICOH GR III", "GR3", 200)] {
        let d = shoot.join(sub);
        std::fs::create_dir_all(&d).unwrap();
        let mut paths = Vec::new();
        // (another size: the same bytes would be one photo imported twice)
        for (i, b) in lightcraft_merge::synth::bracket_dngs(w, 160, &[-2.0, 0.0, 2.0]).unwrap().into_iter().enumerate() {
            let p = d.join(format!("IMG_{i}.dng"));
            std::fs::write(&p, b).unwrap();
            paths.push(p.to_string_lossy().to_string());
        }
        let ids = import(&mut s, paths);
        assert_eq!(ids.len(), 3);
        // both cameras fire at the same moment
        for (i, (id, (sh, b))) in ids.iter().zip([("1/500", -2.0), ("1/125", 0.0), ("1/30", 2.0)]).enumerate() {
            exif(&mut s, *id, &format!("2026-11-20T09:30:0{i}"), sh, Some(b));
            let mut m = s.catalog.photo(*id).unwrap().meta.clone();
            m.camera = cam.into();
            s.catalog.apply(Op::SetMeta { id: *id, meta: Box::new(m) }).unwrap();
        }
        by_cam.push((cam, sub, ids));
    }
    let root = shoot.to_string_lossy().to_string();
    let r = s.execute("merge.find", &json!({"path": root, "deep": true})).unwrap();
    assert_eq!(r["found"], 2, "{r}");
    let c = s.execute("merge.candidates", &json!({"path": root, "deep": true})).unwrap();
    let gs = c["groups"].as_array().unwrap();
    assert_eq!(gs.len(), 2, "{c}");
    for g in gs {
        assert_eq!(g["cameras"].as_array().unwrap().len(), 1, "never two cameras in a group: {g}");
        let cam = g["cameras"][0].as_str().unwrap();
        let (_, sub, ids) = by_cam.iter().find(|(c, _, _)| *c == cam).unwrap();
        assert_eq!(g["subfolders"], json!([sub]));
        assert_eq!(ids_of(&g["photos"]), *ids);
        assert_eq!(g["frames"][0]["subfolder"], *sub);
    }
    // the shoot folder's own (not deep) pool is empty; the deep one holds both
    let counts = crate::merge_find::counts(&s.catalog);
    assert_eq!(counts.folder(&root, true), 2);
    assert_eq!(counts.folder(&root, false), 0);
    s.execute("library.source", &json!({"kind": "merges", "path": root, "deep": true})).unwrap();
    assert_eq!(s.visible_cloned().len(), 6);
    let _ = std::fs::remove_dir_all(&shoot);
}

/// merge.setFrames takes frames out and puts them back (they stay listed, inGroup false);
/// merge.dismiss sends a group back, merge.restore brings it back open.
#[test]
fn frames_can_be_taken_out_and_groups_sent_back_and_restored() {
    let (mut s, dir, ids, other, _) = brackets("frames");
    let folder = dir.to_string_lossy().to_string();
    let gid = s.execute("merge.find", &json!({"path": folder})).unwrap()["groups"][0]["id"].as_str().unwrap().to_string();
    let r = s.execute("merge.setFrames", &json!({"id": gid, "photos": [ids[2].0, ids[0].0]})).unwrap();
    assert_eq!(ids_of(&r["photos"]), vec![ids[0], ids[2]], "capture order");
    assert_eq!(ids_of(&r["out"]), vec![ids[1]]);
    let c = s.execute("merge.candidates", &json!({"path": folder})).unwrap();
    let g = &c["groups"][0];
    assert_eq!(ids_of(&g["photos"]), vec![ids[0], ids[2]]);
    let frames = g["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 3, "a frame taken out is still listed");
    assert_eq!(frames.iter().map(|f| f["inGroup"].as_bool().unwrap()).collect::<Vec<_>>(), vec![true, false, true]);
    // still in the view, to be put back
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert_eq!(s.visible_cloned(), ids);
    // only found frames; one undo step
    assert!(s.execute("merge.setFrames", &json!({"id": gid, "photos": [other.0]})).is_err());
    assert!(s.execute("merge.setFrames", &json!({"id": "nope", "photos": []})).is_err());
    assert!(s.execute("merge.setFrames", &json!({"id": gid})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(crate::merge_find::group(&s.catalog, &gid).unwrap().photos, ids);
    s.execute("merge.setFrames", &json!({"id": gid, "photos": [ids[0].0]})).unwrap();
    s.execute("merge.setFrames", &json!({"id": gid, "photos": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    assert_eq!(crate::merge_find::group(&s.catalog, &gid).unwrap().photos, ids);
    // send.back. → restore.
    assert_eq!(s.execute("merge.dismiss", &json!({"ids": [gid]})).unwrap()["dismissed"], 1);
    assert_eq!(s.execute("merge.candidates", &json!({"path": folder})).unwrap()["groups"], json!([]));
    let sent = crate::merge_find::sent_back(&s.catalog, &crate::merge_find::Scope::Folder { path: folder.clone(), deep: false });
    assert_eq!(sent.len(), 1);
    assert!(s.execute("merge.setFrames", &json!({"id": gid, "photos": [ids[0].0]})).is_err(), "only an open group");
    assert_eq!(s.execute("merge.restore", &json!({"ids": [gid]})).unwrap()["restored"], 1);
    assert_eq!(s.execute("merge.restore", &json!({"ids": [gid]})).unwrap()["restored"], 0);
    let c = s.execute("merge.candidates", &json!({"path": folder})).unwrap();
    assert_eq!(c["groups"][0]["state"], "open");
    assert!(s.execute("merge.restore", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The JSON Second Brain reads (the shared contract, 2026-11-24): merge.candidates and
/// merge.thumbs.
#[test]
fn candidates_and_thumbs_json_shape() {
    let (mut s, dir, ids, _, _) = brackets("shape");
    let folder = dir.to_string_lossy().to_string();
    s.execute("merge.find", &json!({"path": folder})).unwrap();
    let c = s.execute("merge.candidates", &json!({"path": folder, "deep": true, "state": "all"})).unwrap();
    assert_eq!(c["running"], false);
    let g = &c["groups"][0];
    for k in ["id", "kind", "tag", "photos", "state", "result", "folder", "found", "confidence", "reasons", "frames", "cameras", "subfolders"] {
        assert!(g.get(k).is_some(), "{k} in {g}");
    }
    assert_eq!(g["confidence"], "sure");
    assert_eq!(g["reasons"][1], "exposures \u{2212}2 / 0 / +2");
    assert_eq!(g["cameras"], json!(["LEICA M (Typ 262)"]));
    assert_eq!(g["subfolders"], json!([""]), "the pool's own folder");
    let f = &g["frames"][0];
    let keys: Vec<&String> = f.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 8, "{f}");
    assert_eq!(f["id"], ids[0].0);
    assert!(f["file"].as_str().unwrap().ends_with("IMG_0.dng") && f["file"].as_str().unwrap().starts_with('/'));
    assert_eq!(f["folder"], g["folder"]);
    assert_eq!(f["camera"], "LEICA M (Typ 262)");
    assert_eq!(f["captured"], "2026-11-20T09:30:00");
    assert_eq!(f["exposure"], "1/500 f/8 ISO 200");
    assert_eq!(f["inGroup"], true);
    // an old group (stored before confidence and reasons): it still reads, with reasons from its
    // frames' metadata
    let gid = g["id"].as_str().unwrap().to_string();
    let mut old = s.catalog.record(&format!("merge.group.{gid}")).unwrap().clone();
    for k in ["confidence", "reasons", "foundFrames"] {
        old.as_object_mut().unwrap().remove(k);
    }
    s.catalog.apply(Op::SetRecord { key: format!("merge.group.{gid}"), value: Some(old) }).unwrap();
    let c = s.execute("merge.candidates", &json!({"path": folder})).unwrap();
    assert_eq!(c["groups"][0]["confidence"], "likely");
    assert_eq!(c["groups"][0]["reasons"], json!(["3 frames in 2 s", "exposures \u{2212}2 / 0 / +2"]));
    // thumbnails: none cached yet → left out; rendered on request
    let t = s.execute("merge.thumbs", &json!({"ids": [ids[0].0, 999_999]})).unwrap();
    assert_eq!(t, json!({"thumbs": {}}));
    let t = s.execute("merge.thumbs", &json!({"ids": [ids[0].0], "size": 160, "render": true})).unwrap();
    let p = t["thumbs"][ids[0].0.to_string()].as_str().unwrap().to_string();
    assert!(p.ends_with(".jpg") && std::path::Path::new(&p).exists(), "{t}");
    let jpg = std::fs::read(&p).unwrap();
    let img = lightcraft_preview::decode_jpeg(&jpg).unwrap();
    assert!(img.width.max(img.height) <= 160, "{}x{}", img.width, img.height);
    assert!(s.execute("merge.thumbs", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// re-check.: open groups found by an older finder and never touched are looked at again with
/// today's (a burst the old one called a panorama goes away); sent-back and changed groups stay;
/// one undo brings the old ones back.
#[test]
fn recheck_replaces_untouched_old_guesses_only() {
    let mut s = Session::with_demo();
    let burst = temp_dir("recheck");
    let mut paths = Vec::new();
    for i in 0..6 {
        let p = burst.join(format!("B_{i}.dng"));
        std::fs::write(&p, lightcraft_merge::synth::bracket_dngs(240, 160, &[0.0]).unwrap().remove(0)).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let b = import(&mut s, paths);
    for (i, id) in b.iter().enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T15:00:0{i}"), "1/250", None);
    }
    let folder = lightcraft_catalog::query::folder_key(&burst.to_string_lossy());
    let old = |ids: &[PhotoId], state: &str, found: &[PhotoId]| {
        let g = json!({"id": crate::merge_find::group_id(ids), "kind": "pano", "photos": ids.iter().map(|p| p.0).collect::<Vec<_>>(), "state": state,
            "folder": folder, "foundFrames": found.iter().map(|p| p.0).collect::<Vec<_>>()});
        Op::SetRecord { key: format!("{}{}", crate::merge_find::GROUP_PREFIX, g["id"].as_str().unwrap()), value: Some(g) }
    };
    let (a, d, c) = ([b[0], b[1]], [b[2], b[3]], [b[4], b[5]]);
    s.commit("old finder", Op::Batch { ops: vec![old(&a, "open", &[]), old(&d, "dismissed", &[]), old(&c, "open", &c)] }).unwrap();
    // C: he took a frame out
    let cid = crate::merge_find::group_id(&c);
    s.execute("merge.setFrames", &json!({"id": cid, "photos": [b[4].0]})).unwrap();
    let r = s.execute("merge.recheck", &json!({"path": burst.to_string_lossy()})).unwrap();
    assert_eq!(r["removed"], 1, "{r}");
    assert_eq!(r["find"]["found"], 0, "a burst is not a merge: {r}");
    let ids: Vec<String> = crate::merge_find::groups(&s.catalog).iter().map(|g| g.id.clone()).collect();
    assert!(!ids.contains(&crate::merge_find::group_id(&a)), "the untouched old guess is gone");
    assert!(ids.contains(&crate::merge_find::group_id(&d)) && ids.contains(&cid), "sent back and changed stay: {ids:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(crate::merge_find::group(&s.catalog, &crate::merge_find::group_id(&a)).is_some(), "undo brings it back");
    let _ = std::fs::remove_dir_all(&burst);
}

/// A device key's `path: ""` = the folder on screen; with none on screen it refuses (never the
/// whole library by accident).
#[test]
fn an_empty_path_means_the_folder_on_screen() {
    let (mut s, dir, _, _, _) = brackets("here");
    s.execute("library.source", &json!({"kind": "all"})).unwrap();
    assert!(s.execute("merge.find", &json!({"path": ""})).is_err(), "nothing on screen: refused");
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": dir.to_string_lossy()})).unwrap();
    let r = s.execute("merge.recheck", &json!({"path": "", "deep": true})).unwrap();
    assert!(r["find"].is_object(), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Settings ▸ merge. (Bryan, 2026-11-24: "pick and choose by default for the different types of
/// merges"): an hdr. group merged with its kind's defaults — originals kept visible, the merged
/// photo in merges. only, not stacked — and the automatic finder's every-folder rule.
#[test]
fn merge_settings_decide_what_a_group_merge_does() {
    let (mut s, dir, ids, other, _) = brackets("prefs");
    let folder = dir.to_string_lossy().to_string();
    s.merge_prefs.hdr.hide_originals = false;
    s.merge_prefs.hdr.merges_only = true;
    s.merge_prefs.hdr.stack = false;
    s.merge_prefs.panorama.hide_originals = true; // another kind's settings don't apply
    let r = s.execute("merge.find", &json!({"path": folder})).unwrap();
    let gid = r["groups"][0]["id"].as_str().unwrap().to_string();
    let counts = crate::merge_find::counts(&s.catalog);
    assert_eq!((counts.open_folder(&folder, false), counts.folder(&folder, true)), (1, 1));
    let r = s.execute("merge.mergeGroups", &json!({"ids": [gid], "options": {"autoSettings": false}})).unwrap();
    assert_eq!(r["failed"], json!([]), "{r}");
    let merged = PhotoId(r["merged"][0]["id"].as_u64().unwrap());
    s.execute("library.source", &json!({"kind": "libraryFolder", "path": folder})).unwrap();
    let shown = s.visible_cloned();
    assert!(ids.iter().all(|i| shown.contains(i)), "the originals stay visible: {shown:?}");
    assert!(shown.contains(&other) && !shown.contains(&merged), "the merged photo only in merges.: {shown:?}");
    assert!(s.catalog.stack_of(merged).is_none(), "not stacked");
    s.execute("library.source", &json!({"kind": "merges", "path": folder})).unwrap();
    assert!(s.visible_cloned().contains(&merged));
    let counts = crate::merge_find::counts(&s.catalog);
    assert_eq!(counts.open_folder(&folder, false), 0, "merged: no longer open");
    let _ = std::fs::remove_dir_all(&dir);

    // find automatically: every new folder looks at a folder not named like a wedding too
    let plain = std::env::temp_dir().join(format!("lc-merges-plain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&plain);
    std::fs::create_dir_all(&plain).unwrap();
    let mut paths = Vec::new();
    for (i, b) in lightcraft_merge::synth::bracket_dngs(200, 120, &[-2.0, 0.0, 2.0]).unwrap().into_iter().enumerate() {
        let p = plain.join(format!("IMG_{i}.dng"));
        std::fs::write(&p, b).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let mut s = Session::new().with_fs();
    s.vision.background = true;
    let ids = import(&mut s, paths);
    for (i, (id, (sh, b))) in ids.iter().zip([("1/500", -2.0), ("1/125", 0.0), ("1/30", 2.0)]).enumerate() {
        exif(&mut s, *id, &format!("2026-11-20T09:30:0{i}"), sh, Some(b));
    }
    let r = s.merge_find_auto().expect("a first look").unwrap();
    assert_eq!(r["found"], 0, "weddings only: {r}");
    s.merge_scanned.clear();
    s.catalog.apply(Op::SetRecord { key: crate::merge_find::SCANNED.into(), value: None }).unwrap();
    let r = s.merge_find_auto_in(true).expect("every folder").unwrap();
    assert_eq!(r["found"], 1, "{r}");
    let _ = std::fs::remove_dir_all(&plain);
}
