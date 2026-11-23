//! merges., headless (Bryan, 2026-11-23): "recognize possible photo merges and put those in a
//! separate area … on the side bar — interact and group there … When it's merged, get rid of the
//! originals showing up in the album — don't delete them — and have the image show up in that area
//! AND in the corresponding place … and mark it on the thumbnail what it is".
//!
//! * The automatic finder (Settings ▸ General ▸ find merges: auto., the default) finds a bracket
//!   by itself; a `merges.` row shows under its folder and its collection, with a count.
//! * The row shows the merges. view: the group under a header (its kind, merge., skip.) and a bar
//!   (find., all., none., merge., skip., both., merges.only., unmerge.), nothing clipped or on top
//!   of anything else.
//! * merge. merges on the worker: the originals leave the folder and the collection (not deleted),
//!   the merged photo is there and in merges., tagged `hdr.`.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes screenshots there.

use std::time::Duration;

use lightcraft_catalog::{Album, AlbumId, Op, PhotoId};
use lightcraft_engine::merge_find::{self, GroupState};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

struct Fixture {
    h: Headless,
    dir: std::path::PathBuf,
    frames: Vec<PhotoId>,
    other: PhotoId,
    album: AlbumId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(tag: &str) -> Fixture {
    let dir = std::env::temp_dir().join(format!("lc-ui-merges-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::new();
    for (i, b) in lightcraft_merge::synth::bracket_dngs(240, 160, &[-2.0, 0.0, 2.0, 1.0]).unwrap().into_iter().enumerate() {
        let p = dir.join(format!("IMG_{i}.dng"));
        std::fs::write(&p, b).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let mut session = lightcraft_engine::Session::new().with_fs();
    let r = session.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<PhotoId> = r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 4);
    // what the camera wrote: three brackets, then one photo much later
    let shots = [
        ("2026-11-20T09:30:00", "1/500", -2.0),
        ("2026-11-20T09:30:01", "1/125", 0.0),
        ("2026-11-20T09:30:01", "1/30", 2.0),
        ("2026-11-20T11:00:00", "1/60", 1.0),
    ];
    for (id, (t, sh, b)) in ids.iter().zip(shots) {
        let mut m = session.catalog.photo(*id).unwrap().meta.clone();
        m.camera = "LEICA M (Typ 262)".into();
        m.lens = "Summicron 35".into();
        m.focal_mm = Some(35.0);
        m.aperture = Some(8.0);
        m.iso = Some(200);
        m.shutter = sh.into();
        m.exposure_bias = Some(b);
        session.catalog.apply(Op::SetMeta { id: *id, meta: Box::new(m) }).unwrap();
        session.catalog.apply(Op::SetCaptured { id: *id, captured: Some(t.into()) }).unwrap();
    }
    let album = session.catalog.alloc_album_id();
    let mut a = Album::new(album, "Sunrise at the lake");
    a.photos = ids.clone();
    session.catalog.apply(Op::AddAlbum { album: a }).unwrap();
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(session, services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.app.ui.toggle_sidebar_section("navigator");
    h.settle(SETTLE);
    Fixture { h, dir, frames: ids[..3].to_vec(), other: ids[3], album }
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
    h.step();
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let p = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

#[test]
fn merges_are_found_shown_merged_and_tagged() {
    let mut f = fixture("flow");
    let (frames, folder, other, album) = (f.frames.clone(), f.dir.to_string_lossy().to_string(), f.other, f.album);
    let h = &mut f.h;
    // found by itself (auto. is the default)
    assert_eq!(h.app.ui.settings.merge_find, crate::merges::FindMode::Auto);
    assert!(h.step_until(T, |h| merge_find::groups(&h.app.session.catalog).len() == 1), "the automatic finder found the bracket");
    let g = merge_find::groups(&h.app.session.catalog).remove(0);
    assert_eq!((g.kind, g.photos.clone()), (lightcraft_catalog::MergeType::Hdr, frames.clone()));
    h.settle(SETTLE);
    // merges. rows: under the collection and under the folder, with a count
    let arow = format!("merges:album:{}", album.0);
    assert!(has(h, &format!("source:{arow}")), "a merges. row under the collection");
    assert_eq!(h.app.widgets.iter().filter(|(w, _)| w.starts_with("source:merges:folder:")).count(), 1, "and under the folder");
    assert!(has(h, &format!("count:{arow}")));
    assert!(widget(h, &format!("label:{arow}")).left() > widget(h, &format!("label:album:{}", album.0)).left(), "indented under it");
    assert!((widget(h, &format!("source:{arow}")).height() - 29.0).abs() < 0.5, "one row height");
    shot(h, "merges-1-sidebar.png");

    // the merges. view: the group, its header and the bar
    click(h, &format!("source:{arow}"));
    h.settle(SETTLE);
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Merges);
    assert_eq!(h.app.session.visible_cloned(), frames);
    assert!(has(h, &format!("highlight:{arow}")));
    for b in ["mergesFind", "mergesAll", "mergesNone", "mergesMerge", "mergesSkip", "mergesBoth", "mergesOnly", "mergesUnmerge"] {
        assert!(has(h, &format!("button:{b}")), "{b}");
    }
    let head = format!("group:merges:group:{}", g.id);
    assert!(has(h, &head), "a header for the group");
    for b in [format!("mergesKind-{}", g.id), format!("mergesGroupMerge-{}", g.id), format!("mergesGroupSkip-{}", g.id)] {
        let r = widget(h, &format!("button:{b}"));
        assert!(widget(h, &head).contains_rect(r.shrink(1.0)), "{b} inside its header");
    }
    // the bar's buttons: one height, 8 px apart, inside the bar
    let bar = widget(h, "merges:bar");
    let mut bs: Vec<egui::Rect> = ["mergesFind", "mergesAll", "mergesNone", "mergesMerge", "mergesSkip", "mergesBoth", "mergesOnly", "mergesUnmerge"]
        .iter()
        .map(|b| widget(h, &format!("button:{b}")))
        .collect();
    bs.sort_by(|a, b| a.left().total_cmp(&b.left()));
    for w in bs.windows(2) {
        assert!(w[1].left() - w[0].right() >= 8.0 - 0.5, "{w:?}");
        assert!((w[1].height() - w[0].height()).abs() < 0.5);
    }
    assert!(bs.iter().all(|b| bar.contains_rect(*b) && b.top() - bar.top() >= 8.0 - 0.5), "{bar:?} {bs:?}");
    let canvas = h.app.canvas_rect.expect("the grid");
    let faults = crate::tests_panels::layout_faults(h, canvas);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "merges-2-view.png");

    // all. / none.; a header click picks its group
    click(h, "button:mergesAll");
    assert_eq!(h.app.session.selection.ids, frames);
    click(h, "button:mergesNone");
    assert!(h.app.session.selection.ids.is_empty());
    // the kind menu, then back
    h.app.run("merge.setKind", json!({"id": g.id, "kind": "pano"})).unwrap();
    h.settle(SETTLE);
    assert_eq!(merge_find::group(&h.app.session.catalog, &g.id).unwrap().kind, lightcraft_catalog::MergeType::Pano);
    h.app.run("merge.setKind", json!({"id": g.id, "kind": "hdr."})).unwrap();

    // merge.: on the worker; the originals leave, the merged photo comes in, tagged
    click(h, &format!("button:mergesGroupMerge-{}", g.id));
    assert!(
        h.step_until(Duration::from_secs(120), |h| merge_find::group(&h.app.session.catalog, &g.id).is_some_and(|g| g.state == GroupState::Merged)),
        "merged: {}",
        h.app.ui.status
    );
    let merged = merge_find::group(&h.app.session.catalog, &g.id).unwrap().result.unwrap();
    h.app.run("library.source", json!({"kind": "merges", "id": album.0})).unwrap();
    h.settle(SETTLE);
    assert_eq!(h.app.session.visible_cloned(), vec![merged]);
    assert!(has(h, "group:merges:merged"));
    assert!(has(h, &format!("badge:merge:{merged}", merged = merged.0)), "tagged");
    let faults = crate::tests_panels::layout_faults(h, h.app.canvas_rect.unwrap());
    assert!(faults.is_empty(), "{faults:#?}");
    shot(h, "merges-3-merged.png");
    // the collection: the merged photo where the frames were; the frames gone, never deleted
    h.app.run("library.source", json!({"kind": "album", "id": album.0})).unwrap();
    h.settle(SETTLE);
    assert_eq!(h.app.session.visible_cloned().len(), 2);
    assert!(h.app.session.visible_cloned().contains(&merged) && h.app.session.visible_cloned().contains(&other));
    assert!(has(h, &format!("badge:merge:{}", merged.0)));
    shot(h, "merges-4-collection.png");
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": folder})).unwrap();
    assert!(frames.iter().all(|f| !h.app.session.visible_cloned().contains(f)));
    for f in &frames {
        let p = h.app.session.catalog.photo(*f).unwrap();
        assert!(!p.deleted && p.merge_hidden());
    }
    // merges.only.: out of the folder, still in merges.
    h.app.run("library.source", json!({"kind": "merges", "id": album.0})).unwrap();
    h.app.run("library.select", json!({"ids": [merged.0]})).unwrap();
    h.settle(SETTLE);
    click(h, "button:mergesOnly");
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": folder})).unwrap();
    assert!(!h.app.session.visible_cloned().contains(&merged));
    // the setting
    assert_eq!(h.app.run("merge.findMode", json!({"mode": "find."})).unwrap()["mode"], "manual");
    assert_eq!(h.app.ui.settings.merge_find, crate::merges::FindMode::Manual);
    assert!(h.app.run("merge.findMode", json!({"mode": "sometimes"})).is_err());
}

/// find. mode: nothing is found until find. is clicked in a merges. view.
#[test]
fn manual_mode_waits_for_find() {
    let mut f = fixture("manual");
    let h = &mut f.h;
    // the first frames may have looked already: start from nothing
    h.app.ui.settings.merge_find = crate::merges::FindMode::Manual;
    for g in merge_find::groups(&h.app.session.catalog) {
        h.app.session.catalog.apply(Op::SetRecord { key: g.key(), value: None }).unwrap();
    }
    h.settle(SETTLE);
    assert!(merge_find::groups(&h.app.session.catalog).is_empty());
    h.app.run("library.source", json!({"kind": "merges", "id": f.album.0})).unwrap();
    h.settle(SETTLE);
    assert!(h.app.session.visible_cloned().is_empty());
    shot(h, "merges-0-empty.png");
    click(h, "button:mergesFind");
    assert!(h.step_until(T, |h| merge_find::groups(&h.app.session.catalog).len() == 1), "find. found it");
    h.settle(SETTLE);
    assert_eq!(h.app.session.visible_cloned(), f.frames);
}
