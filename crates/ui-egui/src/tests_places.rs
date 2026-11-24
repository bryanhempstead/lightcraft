//! LR-MAP-PLACES, headless and offline (a fake geocoder: no network in tests): the Library's
//! Place & Tags section looks a business up on a worker thread, puts the picked result on the
//! album shown (one undo step, kept as the album's place), and offers suggested keywords as chips
//! picked with click / ⌘-click / ⇧-click / all. / none., added with add. and dropped with drop.
//! Its layout passes the audit. With `LIGHTCRAFT_TEST_SHOTS=<dir>` it writes real-size screenshots.

use std::sync::Arc;

use lightcraft_catalog::PhotoId;
use lightcraft_engine::places::{Geocoder, Place};
use serde_json::json;

use crate::headless::Headless;
use crate::tests_library2::{T, click, headless, settle, shot};

struct Fake;

impl Geocoder for Fake {
    fn search(&self, q: &str, _: usize) -> Result<Vec<Place>, String> {
        let pike = Place {
            name: "Pike Place Market".into(),
            address: "Pike Place Market, 85, Pike Street, Belltown, Seattle, King County, Washington, 98101, United States".into(),
            lat: 47.6097,
            lon: -122.3422,
            location: "Pike Place Market".into(),
            city: "Seattle".into(),
            state: "Washington".into(),
            country: "United States".into(),
            kind: "marketplace".into(),
        };
        let mut other = pike.clone();
        other.name = "Pike Place Chowder".into();
        other.location = "Pike Place Chowder".into();
        other.kind = "restaurant".into();
        Ok(if q.to_lowercase().contains("pike") { vec![pike, other] } else { vec![] })
    }
    fn reverse(&self, _: f64, _: f64) -> Result<Option<Place>, String> {
        Ok(None)
    }
    fn name(&self) -> &str {
        "fake"
    }
}

fn has(h: &mut Headless, id: &str) -> bool {
    let r = h.request("ui.widgets", json!({}), T);
    r["result"].as_array().is_some_and(|a| a.iter().any(|w| w["id"] == id))
}

fn keywords(h: &Headless, id: u64) -> Vec<String> {
    h.app.session.catalog.photo(PhotoId(id)).map(|p| p.meta.keywords.clone()).unwrap_or_default()
}

#[test]
fn place_and_tags_section_finds_applies_and_suggests() {
    let mut s = lightcraft_engine::Session::with_demo();
    s.places.geocoder = Arc::new(Fake);
    let dir = std::env::temp_dir().join(format!("lc-ui-places-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    s.places.cache_file = Some(dir.join("cache.json"));
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    let album = s.execute("album.create", &json!({"name": "*C&J WEDDING"})).unwrap()["id"].as_u64().unwrap();
    s.execute("library.select", &json!({"ids": [ids[0]]})).unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0], ids[1], ids[2]]})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    let mut h = headless(s);
    // Library ▸ Set Location and Tags…: the section opens with the search started
    h.app.run("view.places", json!({"q": "Pike Place"})).unwrap();
    let t0 = std::time::Instant::now();
    while (h.app.places.searching() || h.app.places.results.is_empty()) && t0.elapsed() < std::time::Duration::from_secs(10) {
        settle(&mut h);
    }
    settle(&mut h);
    assert_eq!(h.app.places.results.len(), 2, "{:?}", h.app.places.error);
    assert!(has(&mut h, "placeResult:1") && has(&mut h, "button:placeApply") && has(&mut h, "button:placeScopeAlbum"));
    // the album's name already says what it is
    assert!(has(&mut h, "tagChip:wedding"), "a suggestion from the album's name");
    let right = egui::Rect::from_min_max(egui::pos2(1300.0 - h.app.ui.right_width + 2.0, 40.0), egui::pos2(1300.0, 900.0));
    let faults = crate::tests_panels::layout_faults(&h, right);
    assert!(faults.is_empty(), "{faults:?}");
    shot(&mut h, "places-found.png");
    // pick the second result, apply.: the whole album, one undo step, kept as its place
    click(&mut h, "placeResult:1");
    assert_eq!(h.app.places.picked, Some(1));
    let undo = h.app.session.undo.len();
    click(&mut h, "button:placeApply");
    assert_eq!(h.app.session.undo.len(), undo + 1);
    for id in &ids[..3] {
        let m = h.app.session.catalog.photo(PhotoId(*id)).unwrap().meta.clone();
        assert_eq!((m.gps, m.location.as_str(), m.city.as_str()), (Some((47.6097, -122.3422)), "Pike Place Chowder", "Seattle"));
    }
    assert_ne!(h.app.session.catalog.photo(PhotoId(ids[3])).unwrap().meta.location, "Pike Place Chowder", "outside the album: untouched");
    let d = h.app.session.execute("location.default", &json!({"album": album})).unwrap();
    assert_eq!(d["place"]["name"], "Pike Place Chowder");
    settle(&mut h);
    assert!(has(&mut h, "button:placeClear"), "clr. forgets the album's place");
    // tags: place keywords were added with the place; wedding is still a suggestion
    assert!(keywords(&h, ids[0]).iter().any(|k| k == "Seattle"));
    assert!(!keywords(&h, ids[0]).iter().any(|k| k == "wedding"));
    // picking: click = just this, ⌘-click adds, all. / none.
    click(&mut h, "tagChip:wedding");
    assert_eq!(h.app.places.picked_tags, vec!["wedding".to_string()]);
    let r = h.request("ui.clickWidget", json!({"id": "tagChip:Seattle", "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert_eq!(h.app.places.picked_tags.len(), 2);
    click(&mut h, "button:tagNone");
    assert!(h.app.places.picked_tags.is_empty());
    click(&mut h, "button:tagAll");
    assert!(h.app.places.picked_tags.len() >= 5, "{:?}", h.app.places.picked_tags);
    click(&mut h, "tagChip:wedding");
    click(&mut h, "button:tagAdd");
    for id in &ids[..3] {
        assert!(keywords(&h, *id).iter().any(|k| k == "wedding"), "add. puts it on every photo of the album");
    }
    shot(&mut h, "places-applied.png");
    // drop.: an added one comes off the photos and isn't suggested here again
    click(&mut h, "tagChip:restaurant");
    click(&mut h, "button:tagDrop");
    assert!(!keywords(&h, ids[0]).iter().any(|k| k == "restaurant"));
    settle(&mut h);
    assert!(!has(&mut h, "tagChip:restaurant"));
    let faults = crate::tests_panels::layout_faults(&h, right);
    assert!(faults.is_empty(), "{faults:?}");
    // sel.: just the selected photos
    click(&mut h, "button:placeScopeSel");
    assert!(h.app.places.use_selection);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Settings ▸ Import: the automatic parts can be switched (place on by default, name keywords off).
#[test]
fn settings_switch_the_automatic_parts() {
    let s = lightcraft_engine::Session::with_demo();
    let mut h = headless(s);
    assert!(h.app.session.import_defaults.auto_place && h.app.session.import_defaults.place_keywords);
    assert!(!h.app.session.import_defaults.auto_keywords);
    h.app.run("app.settings", json!({"tab": "import"})).unwrap();
    settle(&mut h);
    click(&mut h, "check:settings.autoKeywords");
    click(&mut h, "check:settings.autoPlace");
    assert!(h.app.session.import_defaults.auto_keywords && !h.app.session.import_defaults.auto_place);
    shot(&mut h, "places-settings.png");
}
