//! LRC-LIB-COLL-PARTS: the folders and cameras under a Collections row, headless.
//!
//! Bryan: "I'm missing the drop downs from every main folder that show the separate folders
//! underneath it, like the camera ones … so I can click on those individually and work on a camera
//! at a time easily and fast."
//!
//! * Every collection row has a disclosure triangle (closed until opened; kept in `ui.json`) that
//!   lists the folders its photos come from, named from the shoot (`raw/M262`, `raw/R6`, `jpg`),
//!   the camera after a folder that holds one (`raw/M262 · LEICA M (Typ 262)`).
//! * A click on one shows the collection narrowed to it (filmstrip: `Collection: … › raw/M262`);
//!   ⌘-click adds another, ⇧-click a run; all. / none.; photos dropped on one go into the collection.
//! * `cam.` lists cameras instead, across folders. Smart collections and collections in sets too,
//!   and the Map module's left panel.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes the panel's screenshots there.

use std::time::Duration;

use lightcraft_catalog::{Album, AlbumId, Op, Photo, PhotoId, Source};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
const SHOOT: &str = "/Volumes/LC-parts-test/Erika and Connor wedding";

struct Fixture {
    h: Headless,
    erika: AlbumId,
    smart: AlbumId,
    nested: AlbumId,
    /// A photo in no collection (dropped on a row).
    loose: PhotoId,
}

fn fixture() -> Fixture {
    let files = [
        ("raw/M262/a.dng", "LEICA M (Typ 262)"),
        ("raw/M262/b.dng", "LEICA M (Typ 262)"),
        ("raw/M262/c.dng", "LEICA M (Typ 262)"),
        ("raw/R6/d.cr3", "Canon EOS R6"),
        ("raw/R6/e.cr3", "Canon EOS R6"),
        ("raw/GR3/f.dng", "RICOH GR III"),
        ("jpg/g.jpg", "Canon EOS R6"),
        ("jpg/h.jpg", "LEICA M (Typ 262)"),
    ];
    let mut session = lightcraft_engine::Session::new();
    let cat = &mut session.catalog;
    let mut ids = Vec::new();
    for (i, (rel, cam)) in files.iter().enumerate() {
        let id = cat.alloc_photo_id();
        let mut p = Photo::new(id, Source::File { path: format!("{SHOOT}/{rel}") }, "x.jpg", "JPEG", 60, 40, &format!("2026-03-0{}T10:00:00", 1 + i));
        p.meta.camera = cam.to_string();
        cat.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
        ids.push(id);
    }
    let loose = cat.alloc_photo_id();
    let p = Photo::new(loose, Source::File { path: format!("{SHOOT}/raw/R6/loose.cr3") }, "loose.cr3", "JPEG", 60, 40, "2026-03-09T10:00:00");
    cat.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let album = |cat: &mut lightcraft_catalog::Catalog, name: &str, f: &dyn Fn(&mut Album)| {
        let id = cat.alloc_album_id();
        let mut a = Album::new(id, name);
        f(&mut a);
        cat.apply(Op::AddAlbum { album: a }).unwrap();
        id
    };
    let erika = album(cat, "Erika and Connor", &|a| a.photos = ids.clone());
    let smart = album(cat, "R6 only", &|a| {
        a.smart = Some(Box::new(lightcraft_catalog::Filter { camera: Some("Canon EOS R6".into()), ..Default::default() }));
    });
    let set = album(cat, "Weddings", &|a| a.folder = true);
    let nested = album(cat, "C&J WEDDING", &|a| {
        a.parent = Some(set);
        a.photos = ids[..3].to_vec();
    });
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let app = LightcraftApp::new(session, services);
    let mut h = Headless::new(app, [1400.0, 1000.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    // Navigator, Catalog, Folders folded: Collections in view
    for s in ["navigator", "catalog", "folders"] {
        h.app.ui.toggle_sidebar_section(s);
    }
    h.settle(SETTLE);
    Fixture { h, erika, smart, nested, loose }
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn click_with(h: &mut Headless, id: &str, mods: serde_json::Value) {
    let mut p = json!({"id": id});
    if let (Some(p), Some(m)) = (p.as_object_mut(), mods.as_object()) {
        p.extend(m.clone());
    }
    let r = h.request("ui.clickWidget", p, T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

fn click(h: &mut Headless, id: &str) {
    click_with(h, id, json!({}));
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

/// The sub-rows of collection `a`, top to bottom: their keys.
fn part_rows(h: &Headless, a: AlbumId) -> Vec<String> {
    let prefix = format!("source:albumPart:{}:", a.0);
    let mut v: Vec<(f32, String)> = h.app.widgets.iter().filter_map(|(w, r)| w.strip_prefix(&prefix).map(|k| (r.top(), k.to_string()))).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, k)| k).collect()
}

fn folder(rel: &str) -> String {
    lightcraft_catalog::query::folder_key(&format!("{SHOOT}/{rel}"))
}

fn shown(h: &mut Headless) -> Vec<String> {
    let ids = h.app.session.visible_cloned();
    let mut v: Vec<String> = ids
        .iter()
        .filter_map(|id| h.app.session.catalog.photo(*id))
        .filter_map(|p| match &p.source {
            Source::File { path } => path.strip_prefix(&format!("{SHOOT}/")).map(str::to_string),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

fn crumb(h: &Headless) -> String {
    crate::panels::filmstrip::source_breadcrumb(&h.app)
}

#[test]
fn a_collection_opens_into_its_folders_and_a_click_shows_one_camera() {
    let Fixture { mut h, erika, .. } = fixture();
    let toggle = format!("albumPartsToggle:{}", erika.0);
    assert!(has(&h, &toggle), "every collection row has a triangle");
    assert!(part_rows(&h, erika).is_empty(), "closed until opened");
    shot(&mut h, "parts-1-closed.png");
    click(&mut h, &toggle);
    assert_eq!(part_rows(&h, erika), vec![folder("jpg"), folder("raw/GR3"), folder("raw/M262"), folder("raw/R6")]);
    assert!(h.app.ui.parts_open.contains(&format!("album:{}", erika.0)));
    let back: crate::state::UiState = serde_json::from_value(serde_json::to_value(&h.app.ui).unwrap()).unwrap();
    assert!(back.parts_open.contains(&format!("album:{}", erika.0)), "stays open after a restart");
    // the row says the camera of a one-camera folder whose name doesn't, the count is its photos
    let m262 = format!("albumPart:{}:{}", erika.0, folder("raw/M262"));
    let r6 = format!("albumPart:{}:{}", erika.0, folder("raw/R6"));
    let rows = crate::panels::left::album_parts(
        &h.app,
        &h.view.ctx,
        &lightcraft_engine::album_parts::PartOf::Album { id: erika },
        lightcraft_engine::album_parts::PartBy::Folder,
    );
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, vec!["jpg", "raw/GR3 · RICOH GR III", "raw/M262 · LEICA M (Typ 262)", "raw/R6"]);
    // rows: one height, indented under the collection, the count 8 px or more from the name
    for k in part_rows(&h, erika) {
        let id = format!("albumPart:{}:{k}", erika.0);
        let row = widget(&h, &format!("source:{id}"));
        assert!((row.height() - 29.0).abs() < 0.5, "{id}");
        assert!(widget(&h, &format!("count:{id}")).left() - widget(&h, &format!("label:{id}")).right() >= 8.0 - 0.5, "{id}");
        assert!(widget(&h, &format!("label:{id}")).left() > widget(&h, &format!("label:album:{}", erika.0)).left(), "{id} is indented");
    }
    let scans = crate::panels::left::album_parts_scans(&h.view.ctx);
    // a click: just that folder
    click(&mut h, &format!("source:{m262}"));
    assert_eq!(shown(&mut h), vec!["raw/M262/a.dng", "raw/M262/b.dng", "raw/M262/c.dng"]);
    assert_eq!(crumb(&h), "Collection: Erika and Connor › raw/M262");
    assert!(has(&h, &format!("highlight:{m262}")));
    assert!(!has(&h, &format!("highlight:album:{}", erika.0)), "the folder is chosen, not the whole collection");
    shot(&mut h, "parts-2-one-folder.png");
    // ⌘-click: that one too
    click_with(&mut h, &format!("source:{r6}"), json!({"cmd": true}));
    assert_eq!(shown(&mut h), vec!["raw/M262/a.dng", "raw/M262/b.dng", "raw/M262/c.dng", "raw/R6/d.cr3", "raw/R6/e.cr3"]);
    assert_eq!(crumb(&h), "Collection: Erika and Connor › raw/M262 + raw/R6");
    // ⌘-click again: off
    click_with(&mut h, &format!("source:{m262}"), json!({"cmd": true}));
    assert_eq!(shown(&mut h), vec!["raw/R6/d.cr3", "raw/R6/e.cr3"]);
    // ⇧-click: the run from the last one clicked (jpg … raw/M262)
    click(&mut h, &format!("source:albumPart:{}:{}", erika.0, folder("jpg")));
    click_with(&mut h, &format!("source:{m262}"), json!({"shift": true}));
    assert_eq!(h.app.session.active_album_part().map(|p| p.keys.len()), Some(3));
    assert_eq!(shown(&mut h).len(), 6);
    assert_eq!(crumb(&h), "Collection: Erika and Connor › 3 folders");
    assert_eq!(crate::panels::left::album_parts_scans(&h.view.ctx), scans, "clicks read the cached rows: no rescan");
    // no all. / none. under the rows (Bryan 11-24): the collection row itself shows all of it
    assert!(!has(&h, &format!("button:albumPartsAll-{}", erika.0)) && !has(&h, &format!("button:albumPartsNone-{}", erika.0)));
    // another source and back: the whole collection, not the folders picked before
    h.request("engine.execute", json!({"id": "library.source", "params": {"kind": "all"}}), T);
    h.request("engine.execute", json!({"id": "library.source", "params": {"kind": "album", "id": erika.0}}), T);
    assert_eq!(shown(&mut h).len(), 8);
}

#[test]
fn cam_lists_one_row_per_camera_across_folders() {
    let Fixture { mut h, erika, .. } = fixture();
    click(&mut h, &format!("albumPartsToggle:{}", erika.0));
    click(&mut h, "button:collectionsPartsBy");
    assert!(h.app.ui.collection_parts_by_camera);
    assert_eq!(part_rows(&h, erika), vec!["Canon EOS R6", "LEICA M (Typ 262)", "RICOH GR III"]);
    click(&mut h, &format!("source:albumPart:{}:Canon EOS R6", erika.0));
    assert_eq!(shown(&mut h), vec!["jpg/g.jpg", "raw/R6/d.cr3", "raw/R6/e.cr3"], "the R6's raws and its JPEG");
    assert_eq!(crumb(&h), "Collection: Erika and Connor › Canon EOS R6");
    shot(&mut h, "parts-3-cameras.png");
    // back to folders: the narrowing by camera goes (no rows on screen would say it)
    click(&mut h, "button:collectionsPartsBy");
    assert!(h.app.session.active_album_part().is_none());
    assert_eq!(shown(&mut h).len(), 8);
}

#[test]
fn smart_collections_and_collections_in_sets_open_too() {
    let Fixture { mut h, smart, nested, .. } = fixture();
    // a set has a triangle of its own, as on Folders rows
    assert!(h.app.widgets.iter().any(|(w, _)| w.starts_with("albumSetToggle:")));
    click(&mut h, &format!("albumPartsToggle:{}", smart.0));
    assert_eq!(part_rows(&h, smart), vec![folder("jpg"), folder("raw/R6")]);
    click(&mut h, &format!("source:albumPart:{}:{}", smart.0, folder("jpg")));
    assert_eq!(shown(&mut h), vec!["jpg/g.jpg"]);
    click(&mut h, &format!("albumPartsToggle:{}", nested.0));
    assert_eq!(part_rows(&h, nested), vec![folder("raw/M262")]);
    assert!(!has(&h, &format!("button:albumPartsAll-{}", nested.0)), "no all. / none.");
    shot(&mut h, "parts-4-smart-and-set.png");
}

#[test]
fn photos_dropped_on_a_folder_row_join_the_collection() {
    let Fixture { mut h, erika, loose, .. } = fixture();
    click(&mut h, &format!("albumPartsToggle:{}", erika.0));
    // dragged from the grid (All Photographs) onto the collection's raw/R6 row
    let from = widget(&h, &format!("thumb:{}", loose.0)).center();
    let to = widget(&h, &format!("source:albumPart:{}:{}", erika.0, folder("raw/R6"))).center();
    let r = h.request("ui.drag", json!({"x": from.x, "y": from.y, "toX": to.x, "toY": to.y, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(h.app.ui.dragging_photos.is_none(), "the drag ended");
    assert!(h.app.session.catalog.album(erika).unwrap().photos.contains(&loose));
    let rows = crate::panels::left::album_parts(
        &h.app,
        &h.view.ctx,
        &lightcraft_engine::album_parts::PartOf::Album { id: erika },
        lightcraft_engine::album_parts::PartBy::Folder,
    );
    assert_eq!(rows.iter().find(|r| r.key == folder("raw/R6")).map(|r| r.count), Some(3), "the row counts it (new revision, new rows)");
}

#[test]
fn arrow_keys_over_a_collection_open_and_close_it() {
    let Fixture { mut h, erika, .. } = fixture();
    let r = h.request("ui.hoverWidget", json!({"id": format!("source:album:{}", erika.0)}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let active = h.app.session.selection.active;
    h.request("ui.key", json!({"key": "right"}), T);
    h.step();
    h.step();
    assert!(h.app.ui.parts_open.contains(&format!("album:{}", erika.0)), "→ opens");
    assert_eq!(h.app.session.selection.active, active, "and moves no photo");
    h.request("ui.hoverWidget", json!({"id": format!("source:album:{}", erika.0)}), T);
    h.step();
    h.request("ui.key", json!({"key": "left"}), T);
    h.step();
    h.step();
    assert!(!h.app.ui.parts_open.contains(&format!("album:{}", erika.0)), "← closes");
}

#[test]
fn the_map_module_lists_collections_with_their_folders() {
    let Fixture { mut h, erika, .. } = fixture();
    let r = h.request("ui.set", json!({"view": "map"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    click(&mut h, &format!("albumPartsToggle:{}", erika.0));
    click(&mut h, &format!("source:albumPart:{}:{}", erika.0, folder("raw/R6")));
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Album(erika));
    assert_eq!(h.app.session.visible_unlocated().len(), 2);
    shot(&mut h, "parts-5-map.png");
}

#[test]
fn album_parts_and_library_source_for_agents() {
    let Fixture { mut h, erika, .. } = fixture();
    let r = h.request("engine.execute", json!({"id": "album.parts", "params": {"id": erika.0, "by": "camera"}}), T);
    assert_eq!(r["result"]["rows"].as_array().map(Vec::len), Some(3), "{r}");
    let r = h.request("engine.execute", json!({"id": "library.source", "params": {"kind": "album", "id": erika.0, "cameras": ["RICOH GR III"]}}), T);
    assert_eq!(r["result"]["count"], 1, "{r}");
    let r = h.request("engine.execute", json!({"id": "library.source", "params": {"kind": "album", "id": erika.0, "cameras": ["Nikon"]}}), T);
    assert_ne!(r["ok"], true, "an unknown row is an error, not an empty grid: {r}");
    let r = h.request("engine.execute", json!({"id": "library.source", "params": {"kind": "all", "folders": ["x"]}}), T);
    assert_ne!(r["ok"], true, "{r}");
    let r = h.request("engine.execute", json!({"id": "album.parts", "params": {"id": erika.0}}), T);
    assert_eq!(r["result"]["picked"], json!([]), "{r}");
}

/// Bryan's weddings had whole camera folders on disk that never came into the library
/// (`raw/GR3` beside `raw/M262`): they show under the collection with what is missing, and add.
/// brings them in place into the collection.
#[test]
fn camera_folders_not_in_the_library_show_beside_and_add_brings_them_in() {
    let dir = std::env::temp_dir().join(format!("lc-ui-beside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let shoot = dir.join("Erika and Connor wedding");
    // three different pictures (identical files would be duplicates, never imported twice)
    for (i, f) in ["raw/M262/a.png", "raw/GR3/x.png", "raw/GR3/y.png"].iter().enumerate() {
        let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[90, 3 + 60 * i as u8, 9, 255]; 64] };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        let p = shoot.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, &png).unwrap();
    }
    // a sidecar is no photo
    std::fs::write(shoot.join("raw/GR3/x.xmp"), b"<x:xmpmeta/>").unwrap();
    let path = |f: &str| shoot.join(f).to_string_lossy().to_string();
    let mut session = lightcraft_engine::Session::new().with_fs();
    let id = session.catalog.alloc_photo_id();
    let p = Photo::new(id, Source::File { path: path("raw/M262/a.png") }, "a.png", "PNG", 8, 8, "2026-03-01T10:00:00");
    session.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    let erika = session.catalog.alloc_album_id();
    let mut album = Album::new(erika, "Erika and Connor");
    album.photos = vec![id];
    session.catalog.apply(Op::AddAlbum { album }).unwrap();
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1400.0, 1000.0], 1.0);
    h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    for s in ["navigator", "catalog", "folders"] {
        h.app.ui.toggle_sidebar_section(s);
    }
    h.settle(SETTLE);
    click(&mut h, &format!("albumPartsToggle:{}", erika.0));
    // the disk is read on a worker thread: the row comes once it answers
    h.settle(SETTLE);
    let gr3 = path("raw/GR3");
    let row = format!("albumBeside:{}:{gr3}", erika.0);
    assert!(has(&h, &format!("source:{row}")), "{:?}", h.app.widgets.iter().map(|(w, _)| w).filter(|w| w.contains("album")).collect::<Vec<_>>());
    assert!(widget(&h, &format!("count:{row}")).width() > 0.0, "a quiet count");
    assert!(!has(&h, &format!("dim:{row}")), "full strength, not greyed out");
    let rows = h.request("engine.execute", json!({"id": "album.parts", "params": {"id": erika.0, "scanDisk": true}}), T);
    assert_eq!(rows["result"]["beside"], json!([{"path": gr3, "name": "GR3", "count": 2}]), "two photos, the sidecar not counted: {rows}");
    // the count stays 8 px or more from the name and from add.
    let (label, count, add) =
        (widget(&h, &format!("label:{row}")), widget(&h, &format!("count:{row}")), widget(&h, &format!("button:albumBesideAdd-{}-{gr3}", erika.0)));
    assert!(count.left() - label.right() >= 8.0 - 0.5 && add.left() - count.right() >= 8.0 - 0.5, "{label:?} {count:?} {add:?}");
    shot(&mut h, "parts-6-not-in-library.png");
    click(&mut h, &format!("button:albumBesideAdd-{}-{gr3}", erika.0));
    h.settle(SETTLE);
    assert_eq!(h.app.session.catalog.album(erika).unwrap().photos.len(), 3, "imported in place into the collection");
    assert!(!has(&h, &format!("source:{row}")), "nothing left out: the row goes");
    assert!(has(&h, &format!("source:albumPart:{}:{}", erika.0, lightcraft_catalog::query::folder_key(&gr3))), "GR3 is one of its folders now");
    assert!(std::path::Path::new(&path("raw/GR3/x.png")).exists(), "added in place: nothing moved");
    shot(&mut h, "parts-7-added.png");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Bryan's real case: the shoots. row "Erika and Connor wedding" (/Volumes/BONUSBOY/Erika and
/// Connor wedding/raw/{M262,GR3,X-T2}) opens into its camera folders; one click works on one
/// camera, ⌘-click adds another; a camera folder never imported shows with add.
#[test]
fn a_shoot_opens_into_its_camera_folders() {
    let dir = std::env::temp_dir().join(format!("lc-ui-shoot-parts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let shoot = dir.join("Erika and Connor wedding");
    let files = [
        ("raw/M262/a.png", "LEICA M (Typ 262)", true),
        ("raw/M262/b.png", "LEICA M (Typ 262)", true),
        ("raw/X-T2/c.png", "FUJIFILM X-T2", true),
        ("raw/GR3/d.png", "", false),
        ("raw/GR3/e.png", "", false),
    ];
    let path = |f: &str| shoot.join(f).to_string_lossy().to_string();
    let mut session = lightcraft_engine::Session::new().with_fs();
    for (i, (f, cam, in_library)) in files.iter().enumerate() {
        let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[90, 3 + 40 * i as u8, 9, 255]; 64] };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        let p = shoot.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, &png).unwrap();
        if *in_library {
            let id = session.catalog.alloc_photo_id();
            let mut ph = Photo::new(id, Source::File { path: path(f) }, "x.png", "PNG", 8, 8, &format!("2026-03-0{}T10:00:00", 1 + i));
            ph.meta.camera = cam.to_string();
            session.catalog.apply(Op::AddPhoto { photo: Box::new(ph) }).unwrap();
        }
    }
    // sidecars are no photos (Finder counts them, LightCraft doesn't)
    std::fs::write(shoot.join("raw/GR3/d.xmp"), b"<x:xmpmeta/>").unwrap();
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1400.0, 1000.0], 1.0);
    h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    for s in ["navigator", "catalog", "folders"] {
        h.app.ui.toggle_sidebar_section(s);
    }
    h.settle(SETTLE);
    let r = h.request("engine.execute", json!({"id": "library.shoots", "params": {}}), T);
    let shoots = r["result"]["shoots"].as_array().cloned().unwrap_or_default();
    assert_eq!(shoots.len(), 1, "{r}");
    let sp = shoots[0]["path"].as_str().unwrap().to_string();
    assert_eq!(shoots[0]["name"], "Erika and Connor wedding");
    let toggle = format!("shootPartsToggle:{sp}");
    assert!(has(&h, &toggle), "the shoot row has a triangle");
    assert!(has(&h, "button:shootsPartsBy"), "fldr. / cam. in the shoots. header");
    // the header: fldr. 8 px or more before sort.
    assert!(widget(&h, "button:shootsSort").left() - widget(&h, "button:shootsPartsBy").right() >= 8.0 - 0.5);
    click(&mut h, &toggle);
    h.settle(SETTLE);
    let key = |rel: &str| lightcraft_catalog::query::folder_key(&path(rel));
    let rows = |h: &Headless| {
        let prefix = format!("source:shootPart:{sp}:");
        let mut v: Vec<(f32, String)> = h.app.widgets.iter().filter_map(|(w, r)| w.strip_prefix(&prefix).map(|k| (r.top(), k.to_string()))).collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, k)| k).collect::<Vec<_>>()
    };
    assert_eq!(rows(&h), vec![key("raw/M262"), key("raw/X-T2")]);
    let m262 = format!("source:shootPart:{sp}:{}", key("raw/M262"));
    let xt2 = format!("source:shootPart:{sp}:{}", key("raw/X-T2"));
    assert!(h.app.widgets.iter().any(|(w, _)| w == &format!("label:shootPart:{sp}:{}", key("raw/M262"))));
    // one camera
    click(&mut h, &m262);
    assert_eq!(h.app.session.visible().len(), 2);
    assert!(crate::panels::filmstrip::source_breadcrumb(&h.app).ends_with("› raw/M262"), "{}", crate::panels::filmstrip::source_breadcrumb(&h.app));
    assert!(has(&h, &format!("highlight:shootPart:{sp}:{}", key("raw/M262"))));
    assert!(!has(&h, &format!("highlight:shoot:{sp}")), "the folder is chosen, not the whole shoot");
    // ⌘-click: two cameras
    click_with(&mut h, &xt2, json!({"cmd": true}));
    assert_eq!(h.app.session.visible().len(), 3);
    // the camera folder that never came in: full strength, its photos counted (the sidecar not), add.
    let beside = format!("shootBeside:{sp}:{}", path("raw/GR3"));
    assert!(has(&h, &format!("source:{beside}")), "{:?}", h.app.widgets.iter().map(|(w, _)| w).filter(|w| w.contains("shoot")).collect::<Vec<_>>());
    assert!(!has(&h, &format!("dim:{beside}")));
    let r = h.request("engine.execute", json!({"id": "shoot.parts", "params": {"path": sp, "scanDisk": true}}), T);
    assert_eq!(r["result"]["beside"], json!([{"path": path("raw/GR3"), "name": "raw/GR3", "count": 2}]), "{r}");
    assert_eq!(r["result"]["picked"].as_array().map(Vec::len), Some(2), "{r}");
    shot(&mut h, "parts-8-shoot.png");
    // by camera
    click(&mut h, "button:shootsPartsBy");
    assert_eq!(rows(&h), vec!["FUJIFILM X-T2".to_string(), "LEICA M (Typ 262)".to_string()]);
    assert_eq!(h.app.session.visible().len(), 3, "back to the whole shoot");
    click(&mut h, "button:shootsPartsBy");
    // add.: imported in place, the shoot holds them, the row becomes one of its folders
    click(&mut h, &format!("button:shootBesideAdd-{sp}-{}", path("raw/GR3")));
    h.settle(SETTLE);
    assert!(!has(&h, &format!("source:{beside}")));
    assert_eq!(rows(&h), vec![key("raw/GR3"), key("raw/M262"), key("raw/X-T2")]);
    shot(&mut h, "parts-9-shoot-added.png");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Develop's Collections panel has the Library's rows: each collection's triangle opens its
/// camera folders, and a folder row narrows the collection the filmstrip shows.
#[test]
fn develop_collections_open_into_their_folders_too() {
    let Fixture { mut h, erika, .. } = fixture();
    let r = h.request("ui.set", json!({"module": "develop", "view": "detail", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    // only Collections open in Develop's left panel
    h.app.ui.develop_left_sections = vec!["collections".into()];
    h.settle(SETTLE);
    let toggle = format!("albumPartsToggle:{}", erika.0);
    let r6 = format!("source:albumPart:{}:{}", erika.0, folder("raw/R6"));
    assert!(has(&h, &toggle), "the collection row has its triangle in Develop");
    assert!(!has(&h, &r6), "closed by default");
    click(&mut h, &toggle);
    assert!(h.app.ui.parts_open.contains(&format!("album:{}", erika.0)));
    assert_eq!(part_rows(&h, erika).len(), 4, "{:?}", part_rows(&h, erika));
    // the rows stay inside the panel
    let (panel, row) = (widget(&h, "panel:left_panel"), widget(&h, &r6));
    assert!(row.left() >= panel.left() - 0.5 && row.right() <= panel.right() + 0.5, "{row:?} in {panel:?}");
    click(&mut h, &r6);
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Album(erika));
    assert_eq!(h.app.session.visible_unlocated().len(), 2, "the R6 folder only");
    assert_eq!(h.app.ui.module, crate::state::Module::Develop, "still in Develop");
    shot(&mut h, "parts-6-develop.png");
}
