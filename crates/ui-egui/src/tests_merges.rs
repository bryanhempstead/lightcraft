//! merges., headless (Bryan, 2026-11-23): "recognize possible photo merges and put those in a
//! separate area … on the side bar — interact and group there … When it's merged, get rid of the
//! originals showing up in the album — don't delete them — and have the image show up in that area
//! AND in the corresponding place … and mark it on the thumbnail what it is".
//!
//! * The automatic finder (Settings ▸ General ▸ find merges: auto., the default) finds a bracket
//!   by itself; a `merges.` row shows under its folder and its collection, with a count.
//! * The row shows the merges. view: the group under a header (its kind, merge., send.back.) and a
//!   bar (find., all., none., merge., send.back., both., merges.only., unmerge., smaller., bigger.),
//!   nothing clipped or on top
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

pub(crate) const T: Duration = Duration::from_secs(20);
/// The merges. bar's buttons.
const BAR: [&str; 10] = [
    "mergesFind",
    "mergesAll",
    "mergesNone",
    "mergesMerge",
    "mergesSendBack",
    "mergesBoth",
    "mergesOnly",
    "mergesUnmerge",
    "mergesSmaller",
    "mergesBigger",
];
pub(crate) const SETTLE: Duration = Duration::from_secs(120);

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
    let dir = std::env::temp_dir().join(format!("lc-ui-merges-wedding-{tag}-{}", std::process::id()));
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

pub(crate) fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

pub(crate) fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

pub(crate) fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
    h.step();
}

pub(crate) fn shot(h: &mut Headless, name: &str) {
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
    for b in BAR {
        assert!(has(h, &format!("button:{b}")), "{b}");
    }
    let head = format!("group:merges:group:{}", g.id);
    assert!(has(h, &head), "a header for the group");
    for b in [format!("mergesKind-{}", g.id), format!("mergesGroupMerge-{}", g.id), format!("mergesGroupSendBack-{}", g.id)] {
        let r = widget(h, &format!("button:{b}"));
        assert!(widget(h, &head).contains_rect(r.shrink(1.0)), "{b} inside its header");
    }
    // the bar's buttons: one height, 8 px apart, inside the bar
    let bar = widget(h, "merges:bar");
    let mut bs: Vec<egui::Rect> = BAR.iter().map(|b| widget(h, &format!("button:{b}"))).collect();
    // (a narrow view wraps the bar: per line, left to right; lines 10 px apart)
    bs.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    for w in bs.windows(2) {
        if (w[1].top() - w[0].top()).abs() < 0.5 {
            assert!(w[1].left() - w[0].right() >= 8.0 - 0.5, "{w:?}");
        } else {
            assert!(w[1].top() - w[0].bottom() >= 10.0 - 0.5, "wrapped lines: {w:?}");
        }
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
    // merge. opens the Merge box on the group's kind and frames; its OK merges the group
    click(h, &format!("button:mergesGroupMerge-{}", g.id));
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => assert_eq!(opts.command, "merge.hdr"),
        other => panic!("the Merge box: {other:?}"),
    }
    assert_eq!(h.app.merge.ids, frames);
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
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

/// A shoot folder with two camera subfolders: a panorama from the GR III (its shutter drifting) and
/// a bracket from the M262, both in the shoot's one merges. pool.
pub(crate) fn wave_fixture(tag: &str) -> (Headless, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("lc-ui-mergewave-wedding-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (m262, gr3) = (dir.join("M262"), dir.join("GR3"));
    std::fs::create_dir_all(&m262).unwrap();
    std::fs::create_dir_all(&gr3).unwrap();
    let mut paths = Vec::new();
    for (i, b) in lightcraft_merge::synth::bracket_dngs(300, 200, &[-2.0, 0.0, 2.0]).unwrap().into_iter().enumerate() {
        let p = m262.join(format!("L100{i}.dng"));
        std::fs::write(&p, b).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    for (i, v) in lightcraft_merge::synth::pano_views(480, 320, 400.0, &[-40.0, -20.0, 0.0, 20.0, 40.0]).unwrap().into_iter().enumerate() {
        let png =
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&v.to_srgb8()), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        let p = gr3.join(format!("R00{i}.png"));
        std::fs::write(&p, png).unwrap();
        paths.push(p.to_string_lossy().to_string());
    }
    let mut session = lightcraft_engine::Session::new().with_fs();
    let r = session.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<PhotoId> = r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), 8);
    for (k, id) in ids.iter().enumerate() {
        let mut m = session.catalog.photo(*id).unwrap().meta.clone();
        let (t, sh, b) = if k < 3 {
            m.camera = "LEICA M (Typ 262)".into();
            m.lens = "Summicron 35".into();
            m.focal_mm = Some(35.0);
            [("2026-11-20T09:30:00", "1/500", Some(-2.0)), ("2026-11-20T09:30:01", "1/125", Some(0.0)), ("2026-11-20T09:30:01", "1/30", Some(2.0))][k]
        } else {
            m.camera = "RICOH GR III".into();
            m.lens = "GR 18.3".into();
            m.focal_mm = Some(18.3);
            let i = k - 3;
            (
                ["2026-11-20T09:31:00", "2026-11-20T09:31:02", "2026-11-20T09:31:04", "2026-11-20T09:31:05", "2026-11-20T09:31:07"][i],
                ["1/250", "1/320", "1/200", "1/250", "1/320"][i],
                None,
            )
        };
        m.aperture = Some(8.0);
        m.iso = Some(200);
        m.shutter = sh.into();
        m.exposure_bias = b;
        session.catalog.apply(Op::SetMeta { id: *id, meta: Box::new(m) }).unwrap();
        session.catalog.apply(Op::SetCaptured { id: *id, captured: Some(t.into()) }).unwrap();
    }
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
    (h, dir)
}

pub(crate) fn rect_of(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

/// The cards of group `gid`, left to right as drawn.
pub(crate) fn cards(h: &Headless, gid: &str) -> Vec<(u64, egui::Rect)> {
    let pre = format!("mergeCard:{gid}:");
    h.app.widgets.iter().filter_map(|(w, r)| w.strip_prefix(&pre).and_then(|p| p.parse().ok()).map(|p| (p, *r))).collect()
}

fn overlapping(cs: &[(u64, egui::Rect)]) -> bool {
    cs.windows(2).any(|w| w[1].1.left() < w[0].1.right() - 1.0 && w[1].1.top() < w[0].1.bottom())
}

/// Bryan, 2026-11-24: the merges. area "should not look like the normal grid": each group is "a
/// stacked wave that shows all the photos", hover or click spreads it out, a whole group or single
/// frames are easy to mark; a shoot's camera subfolders are one pool; send.back. and restore.
#[test]
fn the_merges_view_is_a_stacked_wave_per_group() {
    let (mut h, dir) = wave_fixture("view");
    let shoot = dir.to_string_lossy().to_string();
    assert!(
        h.step_until(Duration::from_secs(60), |h| merge_find::groups(&h.app.session.catalog).len() == 2),
        "found: {:?}",
        merge_find::groups(&h.app.session.catalog)
    );
    h.settle(SETTLE);
    let gs = merge_find::groups(&h.app.session.catalog);
    let pano = gs.iter().find(|g| g.kind == lightcraft_catalog::MergeType::Pano).expect("the GR III panorama").clone();
    let hdr = gs.iter().find(|g| g.kind == lightcraft_catalog::MergeType::Hdr).expect("the M262 bracket").clone();
    assert_eq!(pano.photos.len(), 5, "{pano:?}");
    // one merges. row: on the shoot (its deep pool), none on its camera subfolders
    let rows: Vec<String> = h.app.widgets.iter().filter_map(|(w, _)| w.strip_prefix("source:merges:folder:").map(str::to_string)).collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(lightcraft_catalog::query::folder_within(&rows[0], &shoot) && !rows[0].ends_with("M262") && !rows[0].ends_with("GR3"), "{rows:?}");
    click(&mut h, &format!("source:merges:folder:{}", rows[0]));
    h.settle(SETTLE);
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Merges);
    assert_eq!(h.app.session.visible_cloned().len(), 8, "both cameras in one view");
    // not the grid: no grid cells, a wave of cards per group
    assert!(!h.app.widgets.iter().any(|(w, _)| w.starts_with("thumb:")), "no grid cells");
    let (pc, hc) = (cards(&h, &pano.id), cards(&h, &hdr.id));
    assert_eq!((pc.len(), hc.len()), (5, 3));
    assert!(overlapping(&pc), "stacked: the cards overlap {pc:?}");
    assert!(pc.iter().all(|(_, r)| r.height() >= 110.0), "big cards (4 frames per row): {pc:?}");
    let ys: Vec<f32> = pc.iter().map(|(_, r)| r.top()).collect();
    assert!(ys.iter().any(|y| (y - ys[0]).abs() > 2.0), "on a wave: {ys:?}");
    // the headers: kind, confidence, camera, subfolder, merge., send.back.
    for g in [&pano, &hdr] {
        let head = widget(&h, &format!("group:merges:group:{}", g.id));
        for w in [
            format!("button:mergesKind-{}", g.id),
            format!("chip:mergesConfidence-{}", g.id),
            format!("button:mergesGroupMerge-{}", g.id),
            format!("button:mergesGroupSendBack-{}", g.id),
            format!("chip:mergesCamera-{}-0", g.id),
            format!("chip:mergesSubfolder-{}-1", g.id),
        ] {
            let r = widget(&h, &w);
            assert!(head.contains_rect(r.shrink(0.5)), "{w} inside its header {r:?} {head:?}");
            assert!((r.height() - 24.0).abs() < 0.5, "one control height: {w}");
        }
    }
    assert_eq!(widget(&h, &format!("button:mergesKind-{}", pano.id)).width() > 0.0, true);
    let faults = crate::tests_panels::layout_faults(&h, h.app.canvas_rect.unwrap());
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "mergewave-1-stacked.png");

    // hover spreads the wave: every frame side by side
    let r = h.request("ui.hoverWidget", json!({"id": format!("mergeWave:{}", pano.id), "fx": 0.9}), T);
    assert_eq!(r["ok"], true, "{r}");
    for _ in 0..30 {
        h.step();
    }
    let spread = cards(&h, &pano.id);
    assert!(!overlapping(&spread), "spread: {spread:?}");
    shot(&mut h, "mergewave-2-hover.png");
    // a click on its empty area keeps it spread after the pointer leaves
    let wave = widget(&h, &format!("mergeWave:{}", pano.id));
    let taken = cards(&h, &pano.id);
    let (fx, fy) = (1..20)
        .flat_map(|i| (1..20).map(move |j| (i as f32 / 20.0, j as f32 / 20.0)))
        .find(|(fx, fy)| {
            let at = egui::pos2(wave.left() + wave.width() * fx, wave.top() + wave.height() * fy);
            !taken.iter().any(|(_, r)| r.expand(4.0).contains(at))
        })
        .expect("an empty spot in the wave");
    let r = h.request("ui.clickWidget", json!({"id": format!("mergeWave:{}", pano.id), "fx": fx, "fy": fy}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.request("ui.move", json!({"x": 5.0, "y": 5.0}), T);
    for _ in 0..30 {
        h.step();
    }
    assert!(!overlapping(&cards(&h, &pano.id)), "kept spread");
    assert!(crate::folds::is_open_or(&h.view.ctx, &format!("merges.spread:{}", pano.id), false));

    // a click on a card takes the frame out (dimmed, still there); again puts it back
    let out = pano.photos[2];
    click(&mut h, &format!("mergeCard:{}:{}", pano.id, out.0));
    assert!(h.step_until(T, |h| !merge_find::group(&h.app.session.catalog, &pano.id).unwrap().photos.contains(&out)), "taken out");
    h.settle(SETTLE);
    assert!(rect_of(&h, &format!("mergeCard:{}:{}", pano.id, out.0)).is_some(), "still shown");
    let faults = crate::tests_panels::layout_faults(&h, h.app.canvas_rect.unwrap());
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "mergewave-3-frame-out.png");
    click(&mut h, &format!("mergeCard:{}:{}", pano.id, out.0));
    assert!(h.step_until(T, |h| merge_find::group(&h.app.session.catalog, &pano.id).unwrap().photos.contains(&out)), "put back");
    // a double-click opens the photo, and changes nothing in the group (a while after the last
    // clicks: three quick clicks are a triple-click)
    h.advance_time(1.0);
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": format!("mergeCard:{}:{}", pano.id, out.0), "count": 2}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(h.app.ui.view, crate::state::ViewMode::Detail);
    assert_eq!(h.app.session.selection.active, Some(out));
    assert_eq!(merge_find::group(&h.app.session.catalog, &pano.id).unwrap().photos.len(), 5);
    let r = h.request("ui.set", json!({"view": "photoGrid"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);

    // groups are picked by their header: click, ⌘-click adds, again takes away, ⇧-click, none.
    click(&mut h, &format!("group:merges:group:{}", hdr.id));
    assert_eq!(h.app.session.selection.ids, hdr.photos);
    let r = h.request("ui.clickWidget", json!({"id": format!("group:merges:group:{}", pano.id), "cmd": true, "fx": 0.6}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    assert_eq!(h.app.session.selection.ids.len(), 8);
    let r = h.request("ui.clickWidget", json!({"id": format!("group:merges:group:{}", hdr.id), "cmd": true, "fx": 0.6}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    assert_eq!(h.app.session.selection.ids, pano.photos);
    click(&mut h, "button:mergesNone");
    assert!(h.app.session.selection.ids.is_empty());
    let r = h.request("ui.clickWidget", json!({"id": format!("group:merges:group:{}", hdr.id), "shift": true, "fx": 0.6}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
    click(&mut h, "button:mergesAll");
    assert_eq!(h.app.session.selection.ids.len(), 8);
    click(&mut h, "button:mergesNone");

    // send.back.: the group goes to the sent back. section; restore. brings it back
    click(&mut h, &format!("button:mergesGroupSendBack-{}", hdr.id));
    h.settle(SETTLE);
    assert_eq!(merge_find::group(&h.app.session.catalog, &hdr.id).unwrap().state, GroupState::Dismissed);
    assert!(rect_of(&h, &format!("group:merges:group:{}", hdr.id)).is_none());
    assert!(has(&h, "group:merges:sentBack"));
    click(&mut h, "fold:merges.sentBack");
    h.settle(SETTLE);
    let faults = crate::tests_panels::layout_faults(&h, h.app.canvas_rect.unwrap());
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "mergewave-4-sent-back.png");
    click(&mut h, &format!("button:mergesRestore-{}", hdr.id));
    h.settle(SETTLE);
    assert_eq!(merge_find::group(&h.app.session.catalog, &hdr.id).unwrap().state, GroupState::Open);
    assert!(has(&h, &format!("group:merges:group:{}", hdr.id)));
    // smaller. / bigger.
    let before = crate::merges::view_prefs(&h.app).per_row;
    click(&mut h, "button:mergesBigger");
    assert_eq!(crate::merges::view_prefs(&h.app).per_row, before - 1, "bigger.: one frame fewer per row");
    click(&mut h, "button:mergesSmaller");
    assert_eq!(crate::merges::view_prefs(&h.app).per_row, before);
    // the triangle folds a group to its header
    click(&mut h, &format!("fold:merges.group:{}", pano.id));
    h.settle(SETTLE);
    assert!(cards(&h, &pano.id).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Left off in a merges. view: resuming (and the next launch) goes back to it, not to All
/// Photographs (its scope was written as JSON and `library.source` didn't know it).
#[test]
fn resuming_goes_back_to_a_merges_view() {
    let mut f = fixture("resume");
    let folder = f.dir.to_string_lossy().to_string();
    let frames = f.frames.clone();
    let h = &mut f.h;
    assert!(h.step_until(T, |h| merge_find::groups(&h.app.session.catalog).len() == 1));
    h.app.run("library.source", json!({"kind": "merges", "path": folder})).unwrap();
    h.app.run("library.select", json!({"ids": [frames[0].0]})).unwrap();
    h.settle(SETTLE);
    let spot = h.app.ui.left_off.last.clone().expect("a left-off spot");
    assert!(spot.source.starts_with("merges:"), "{}", spot.source);
    h.app.run("library.source", json!({"kind": "all"})).unwrap();
    h.settle(SETTLE);
    h.app.ui.left_off.last = Some(spot);
    h.app.run("view.resumeLastLeftOff", json!({})).unwrap();
    assert!(matches!(h.app.session.source, lightcraft_engine::LibrarySource::Merges), "{:?}", h.app.session.source);
}

/// The merge. tab (after develop.) follows the sidebar like Develop: the picked folder's merges
/// (with its folders'), a collection's, or all; library. leaves it.
#[test]
fn the_merge_tab_opens_every_merge() {
    let mut f = fixture("tab");
    let folder = f.dir.to_string_lossy().to_string();
    let h = &mut f.h;
    assert!(h.step_until(T, |h| merge_find::groups(&h.app.session.catalog).len() == 1));
    h.settle(SETTLE);
    let (develop, merge) = (widget(h, "module:develop"), widget(h, "module:merge"));
    assert!(merge.left() > develop.right() + 8.0, "right after develop.: {develop:?} {merge:?}");
    // a folder picked in the sidebar: merge. shows that folder's merges (and its folders')
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": folder})).unwrap();
    let r = h.request("ui.clickWidget", json!({"id": "module:merge"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(matches!(h.app.session.source, lightcraft_engine::LibrarySource::Merges));
    assert_eq!(h.app.session.merges_scope.clone().unwrap_or_default(), lightcraft_engine::merge_find::Scope::Folder { path: folder.clone(), deep: true });
    assert!(has(h, "merges:bar"), "the merges. view");
    // another pick in the sidebar while on merge.: its merges (All Photographs → all)
    h.app.run("library.source", json!({"kind": "all"})).unwrap();
    h.settle(SETTLE);
    assert!(matches!(h.app.session.source, lightcraft_engine::LibrarySource::Merges));
    assert_eq!(h.app.session.merges_scope.clone().unwrap_or_default(), lightcraft_engine::merge_find::Scope::All);
    // library. leaves the tab, back on the folder the merges were of
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": folder})).unwrap();
    h.settle(SETTLE);
    h.app.run("view.library", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(matches!(h.app.session.source, lightcraft_engine::LibrarySource::LibraryFolder), "{:?}", h.app.session.source);
    assert_eq!(h.app.session.library_folder.as_deref(), Some(folder.as_str()));
}
