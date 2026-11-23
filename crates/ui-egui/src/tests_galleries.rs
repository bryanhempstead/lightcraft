//! LC-GALLERIES in the app: the galleries. module tab over a stand-in for Second Brain's gallery
//! API (`lightcraft_engine::galleries::fake`), headless. With `LIGHTCRAFT_TEST_SHOTS=<dir>` each
//! test writes real-size screenshots there.

use serde_json::{Value, json};

use crate::tests_library2::{T, click, headless, settle, shot, temp};
use lightcraft_engine::galleries::fake::FakeSecondBrain;

fn wait(h: &mut crate::headless::Headless) {
    let ctx = h.view.ctx.clone();
    for _ in 0..6 {
        assert!(crate::tasks::wait(&mut h.app, &ctx, std::time::Duration::from_secs(60)));
        settle(h);
    }
}

fn widgets(h: &mut crate::headless::Headless) -> Vec<String> {
    let r = h.request("ui.widgets", json!({}), T);
    r["result"].as_array().unwrap().iter().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()
}

/// A demo library whose photos live in a shoot folder (`<dir>/Ann & Bo/raw/X-T2/…`), so a new
/// gallery lands in `<dir>/Ann & Bo/gallery/<name>/`.
fn library_in_a_shoot(dir: &std::path::Path) -> (lightcraft_engine::Session, Vec<u64>) {
    let cam = dir.join("Ann & Bo").join("raw").join("X-T2");
    std::fs::create_dir_all(&cam).unwrap();
    for (i, c) in [[200u8, 120, 80], [80, 140, 200], [120, 190, 110]].iter().enumerate() {
        let (w, hh) = (48usize, 32usize);
        let img = lightcraft_raster::Rgba8 { width: w, height: hh, data: vec![[c[0], c[1], c[2], 255]; w * hh] };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(cam.join(format!("DSCF{:04}.png", i + 1)), png).unwrap();
    }
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.execute("library.import", &json!({"paths": [cam.to_string_lossy()]})).unwrap();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    (s, ids)
}

#[test]
fn galleries_tab_makes_a_gallery_from_the_collection_and_brings_picks_back() {
    let dir = temp("galleries-tab");
    let sb = FakeSecondBrain::start(&dir.0.join("sb")).unwrap();
    let (mut s, ids) = library_in_a_shoot(&dir.0);
    assert_eq!(ids.len(), 3);
    s.second_brain_dir = Some(sb.dir.clone());
    let album = s.execute("album.create", &json!({"name": "Ann & Bo"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": ids})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let other = dir.0.join("made in second brain");
    std::fs::create_dir_all(&other).unwrap();
    sb.add("Wedding (Second Brain)", &other.to_string_lossy());
    let mut h = headless(s);
    h.app.services.write_shared = Some(std::sync::Arc::new(|p: &str, b: &[u8]| lightcraft_engine::export::write_file(p, b)));
    settle(&mut h);
    // the fourth module link
    assert!(widgets(&mut h).iter().any(|w| w == "module:galleries"), "galleries. beside library. develop. map.");
    click(&mut h, "module:galleries");
    assert_eq!(h.app.ui.view, crate::state::ViewMode::Galleries);
    wait(&mut h);
    assert!(h.app.galleries.online, "{:?}", h.app.galleries.notice);
    assert_eq!(h.app.galleries.list.len(), 1);
    shot(&mut h, "galleries-list.png");
    // new gallery from… what the grid showed (the collection): exported on a worker, registered
    h.app.galleries.new.client = "Ann".into();
    click(&mut h, "button:galleryFromView");
    wait(&mut h);
    let folder = dir.0.join("Ann & Bo").join("gallery").join("Ann & Bo");
    let files: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files.len(), 3, "exported into <shoot>/gallery/<name>: {files:?} {:?}", h.app.ui.toast);
    let made = sb.lock().requests.iter().find(|(m, p, _)| m == "POST" && p == "/v1/galleries").cloned().expect("registered");
    assert_eq!(made.2["name"], "Ann & Bo");
    assert_eq!(made.2["clientName"], "Ann");
    // it opened: the photos, each from the library
    let gid = h.app.galleries.open.clone().expect("the new gallery opens");
    wait(&mut h);
    let photos = h.app.galleries.detail.as_ref().unwrap()["photos"].as_array().unwrap().clone();
    assert_eq!(photos.len(), 3);
    assert!(photos.iter().all(|p| p["photo"].is_u64()), "{photos:?}");
    // the client hearts two on the link
    let names: Vec<String> = photos.iter().map(|p| p["name"].as_str().unwrap().to_string()).collect();
    sb.set_selected(&gid, &[names[0].as_str(), names[2].as_str()]);
    click(&mut h, "button:galleriesRefresh");
    wait(&mut h);
    shot(&mut h, "galleries-gallery.png");
    // picks: click, ⌘-click, ⇧-click, all., none.
    click(&mut h, &format!("galleryPhoto:{}", names[0]));
    assert_eq!(h.app.galleries.picked, vec![names[0].clone()]);
    let r = h.request("ui.clickWidget", json!({"id": format!("galleryPhoto:{}", names[2]), "shift": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert_eq!(h.app.galleries.picked.len(), 3, "⇧-click picks the run");
    let r = h.request("ui.clickWidget", json!({"id": format!("galleryPhoto:{}", names[1]), "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert_eq!(h.app.galleries.picked.len(), 2, "⌘-click takes one out");
    click(&mut h, "button:galleriesNone");
    assert!(h.app.galleries.picked.is_empty());
    click(&mut h, "button:galleriesAll");
    assert_eq!(h.app.galleries.picked.len(), 3);
    shot(&mut h, "galleries-picked.png");
    // client picks → flags on the source photos, and the picks collection
    click(&mut h, "button:galleryPicksFlag");
    let flagged: Vec<u64> = h.app.session.catalog.photos().filter(|p| p.flag == lightcraft_catalog::Flag::Pick).map(|p| p.id.0).collect();
    assert_eq!(flagged.len(), 2, "the two hearted photos are flagged");
    // settings: type a new client name, save.
    h.app.galleries.form.client = "Ann Lee".into();
    h.app.galleries.form.expires = "2027-01-31".into();
    settle(&mut h);
    click(&mut h, "button:gallerySave");
    wait(&mut h);
    let patch = sb.lock().requests.iter().rev().find(|(m, _, _)| m == "PATCH").cloned().expect("patched");
    assert_eq!(patch.2["clientName"], "Ann Lee");
    assert_eq!(patch.2["expires"], 1_801_353_600_000u64);
    // a gallery photo → its source in Develop
    h.app.run("galleries.pick", json!({"names": [names[1]]})).unwrap();
    click(&mut h, "button:galleriesDevelop");
    assert_eq!(h.app.ui.module, crate::state::Module::Develop);
    let want = photos[1]["photo"].as_u64();
    assert_eq!(h.app.session.selection.active.map(|p| p.0), want);
    // and back: the picks collection in the Library
    h.app.run("view.galleries", json!({"id": gid})).unwrap();
    wait(&mut h);
    click(&mut h, "button:galleryPicksShow");
    assert_eq!(h.app.ui.view, crate::state::ViewMode::PhotoGrid);
    match h.app.session.source {
        lightcraft_engine::LibrarySource::Album(a) => assert_eq!(h.app.session.catalog.album(a).unwrap().photos.len(), 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn galleries_tab_offline_is_read_only_with_start() {
    let dir = temp("galleries-offline");
    let sb = FakeSecondBrain::start(&dir.0.join("sb")).unwrap();
    let shoot = dir.0.join("their shoot");
    std::fs::create_dir_all(&shoot).unwrap();
    let id = sb.add("Made in Second Brain", &shoot.to_string_lossy());
    sb.set_selected(&id, &["a.jpg"]);
    sb.stop();
    let before = std::fs::read(sb.dir.join("client").join("galleries.json")).unwrap();
    let mut s = lightcraft_engine::Session::with_demo();
    s.second_brain_dir = Some(sb.dir.clone());
    let mut h = headless(s);
    let started = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let rec = started.clone();
    h.app.services.open_with = Some(Box::new(move |p: &str, _a: &str| {
        rec.lock().unwrap().push(p.to_string());
        Ok(())
    }));
    h.app.run("view.galleries", json!({})).unwrap();
    wait(&mut h);
    assert!(!h.app.galleries.online);
    assert_eq!(h.app.galleries.list.len(), 1, "read from galleries.json");
    let w = widgets(&mut h);
    assert!(w.iter().any(|x| x == "notice:galleriesOffline"), "the notice");
    // no way to make one while it's down
    let r = h.app.run("galleries.newFrom", json!({"source": "view"}));
    assert!(r.unwrap_err().contains("isn't running"));
    shot(&mut h, "galleries-offline.png");
    click(&mut h, &format!("galleryCard:{id}"));
    wait(&mut h);
    let w = widgets(&mut h);
    assert!(!w.iter().any(|x| x == "button:gallerySave" || x == "button:galleryPublish"), "read-only: no save. / pub.");
    shot(&mut h, "galleries-offline-gallery.png");
    // start. (only when Second Brain is installed here)
    if w.iter().any(|x| x == "button:galleriesStart") {
        click(&mut h, "button:galleriesStart");
        assert!(started.lock().unwrap().iter().any(|p| p.ends_with("Second Brain.app")));
    }
    assert_eq!(std::fs::read(sb.dir.join("client").join("galleries.json")).unwrap(), before, "never written");
}

#[test]
fn expiry_dates_round_trip() {
    use crate::panels::galleries::{date_to_ms, ms_to_date};
    assert_eq!(date_to_ms(""), Some(0));
    assert_eq!(date_to_ms("2027-01-31"), Some(1_801_353_600_000));
    assert_eq!(ms_to_date(1_801_353_600_000), "2027-01-31");
    assert_eq!(ms_to_date(0), "");
    for bad in ["2027-13-01", "27-1-1x", "1969-01-01", "2027-01", "2027-01-01-01", "x"] {
        assert_eq!(date_to_ms(bad), None, "{bad}");
    }
    let _: Value = json!(null);
}
