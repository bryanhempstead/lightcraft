//! LR-MAP-PLACES: place search (coordinates locally, names through a fake geocoder — no network in
//! tests — and the on-disk answer cache), places on photos / albums / folders as one undo step and
//! in the XMP sidecar, an album's place on its new imports, and suggested keywords (place, album
//! name, what the photos show from the AI search index) added and dropped.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::Session;
use crate::places::{Geocoder, Place};
use lightcraft_catalog::PhotoId;

/// A geocoder that knows two places and counts its calls (and can be switched off).
#[derive(Default)]
struct Fake {
    searches: AtomicUsize,
    reverses: AtomicUsize,
    offline: Mutex<bool>,
}

fn pike() -> Place {
    Place {
        name: "Pike Place Market".into(),
        address: "Pike Place Market, 85, Pike Street, Seattle, Washington, United States".into(),
        lat: 47.6097,
        lon: -122.3422,
        location: "Pike Place Market".into(),
        city: "Seattle".into(),
        state: "Washington".into(),
        country: "United States".into(),
        kind: "marketplace".into(),
    }
}

impl Geocoder for Fake {
    fn search(&self, q: &str, _: usize) -> Result<Vec<Place>, String> {
        if *self.offline.lock().unwrap() {
            return Err("offline".into());
        }
        self.searches.fetch_add(1, Ordering::SeqCst);
        Ok(if q.to_lowercase().contains("pike") { vec![pike()] } else { vec![] })
    }
    fn reverse(&self, lat: f64, lon: f64) -> Result<Option<Place>, String> {
        if *self.offline.lock().unwrap() {
            return Err("offline".into());
        }
        self.reverses.fetch_add(1, Ordering::SeqCst);
        Ok(Some(Place {
            name: "Seattle".into(),
            lat: lat + 0.01,
            lon,
            city: "Seattle".into(),
            state: "Washington".into(),
            country: "United States".into(),
            ..Default::default()
        }))
    }
    fn name(&self) -> &str {
        "fake"
    }
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-places-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn session(tag: &str) -> (Session, Arc<Fake>, PathBuf) {
    let mut s = Session::with_demo();
    let fake = Arc::new(Fake::default());
    s.places.geocoder = fake.clone();
    let dir = temp(tag);
    s.places.cache_file = Some(dir.join("geocode-cache.json"));
    (s, fake, dir)
}

fn meta(s: &Session, id: u64) -> lightcraft_catalog::Meta {
    s.catalog.photo(PhotoId(id)).unwrap().meta.clone()
}

#[test]
fn search_reads_coordinates_locally_and_caches_names() {
    let (mut s, fake, dir) = session("search");
    // typed coordinates: no search, named by the reverse lookup, the typed position kept
    let r = s.execute("location.search", &json!({"q": "47.6062, -122.3321"})).unwrap();
    assert_eq!(r["from"], "coordinates");
    assert_eq!(r["results"][0]["city"], "Seattle", "{r}");
    assert_eq!(r["results"][0]["lat"], 47.6062, "the typed position wins");
    assert_eq!(r["results"][1]["name"], "47.60620, -122.33210", "and the bare coordinates");
    assert_eq!(fake.searches.load(Ordering::SeqCst), 0);
    // a business: the geocoder, then the cache (the geocoder isn't asked twice)
    let r = s.execute("location.search", &json!({"q": "Pike Place Market"})).unwrap();
    assert_eq!((r["from"].as_str(), r["results"][0]["city"].as_str()), (Some("geocoder"), Some("Seattle")), "{r}");
    let r = s.execute("location.search", &json!({"q": "  pike   PLACE market "})).unwrap();
    assert_eq!(r["from"], "cache");
    assert_eq!(fake.searches.load(Ordering::SeqCst), 1);
    assert!(dir.join("geocode-cache.json").is_file(), "answers kept on disk");
    // offline: cached answers and typed coordinates still work, the rest is an error to act on
    *fake.offline.lock().unwrap() = true;
    assert_eq!(s.execute("location.search", &json!({"q": "Pike Place Market"})).unwrap()["from"], "cache");
    let r = s.execute("location.search", &json!({"q": "-33.8688, 151.2093"})).unwrap();
    assert_eq!(r["results"].as_array().unwrap().len(), 1, "just the coordinates offline: {r}");
    let e = s.execute("location.search", &json!({"q": "Space Needle"})).unwrap_err().to_string();
    assert!(e.contains("offline"), "{e}");
    // nonsense is an error, never a panic
    for bad in [json!({}), json!({"q": ""}), json!({"q": "0, 0"}), json!({"q": "x".repeat(400)})] {
        assert!(s.execute("location.search", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_place_on_an_album_is_one_undo_step_and_its_default() {
    let (mut s, _fake, dir) = session("apply");
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    let album = s.execute("album.create", &json!({"name": "*C&J WEDDING"})).unwrap()["id"].as_u64().unwrap();
    s.selection = crate::Selection::single(PhotoId(ids[0]));
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0], ids[1], ids[2]]})).unwrap();
    let before: Vec<_> = ids.iter().map(|i| meta(&s, *i)).collect();
    let undo_len = s.undo.len();
    let r = s.execute("location.apply", &json!({"album": album, "q": "Pike Place Market"})).unwrap();
    assert_eq!(r["photos"], 3, "{r}");
    assert_eq!(r["remembered"], true);
    assert_eq!(s.undo.len(), undo_len + 1, "one undo step");
    for i in &ids[..3] {
        let m = meta(&s, *i);
        assert_eq!(m.gps, Some((47.6097, -122.3422)));
        assert_eq!(
            (m.location.as_str(), m.city.as_str(), m.state.as_str(), m.country.as_str()),
            ("Pike Place Market", "Seattle", "Washington", "United States")
        );
        for k in ["Pike Place Market", "Seattle", "Washington", "United States", "market"] {
            assert!(m.keywords.iter().any(|x| x == k), "{k} in {:?}", m.keywords);
        }
    }
    assert_eq!(meta(&s, ids[3]), before[3], "photos outside the album are left alone");
    let d = s.execute("location.default", &json!({"album": album})).unwrap();
    assert_eq!(d["place"]["city"], "Seattle", "{d}");
    // undo: the photos and the album's place, all at once
    s.execute("edit.undo", &json!({})).unwrap();
    let after: Vec<_> = ids.iter().map(|i| meta(&s, *i)).collect();
    assert_eq!(before, after);
    assert!(s.execute("location.default", &json!({"album": album})).unwrap()["place"].is_null());
    // the selection, typed coordinates, no keywords, no default
    s.execute("library.select", &json!({"ids": [ids[4]]})).unwrap();
    let r = s
        .execute("location.apply", &json!({"place": {"lat": -33.8688, "lon": 151.2093, "city": "Sydney", "country": "Australia"}, "keywords": false}))
        .unwrap();
    assert_eq!(r["remembered"], false);
    let m = meta(&s, ids[4]);
    assert_eq!((m.gps, m.city.as_str(), m.location.as_str()), (Some((-33.8688, 151.2093)), "Sydney", ""));
    assert_eq!(m.keywords, before[4].keywords);
    // bad params: errors, nothing written
    for bad in [
        json!({"album": album, "place": {"lat": 0.0, "lon": 0.0}}),
        json!({"album": album, "place": {"lat": 91.0, "lon": 0.0}}),
        json!({"album": album, "place": "Seattle"}),
        json!({"album": 999_999, "q": "Pike"}),
        json!({"album": album}),
        json!({"album": album, "q": "nowhere at all"}),
    ] {
        assert!(s.execute("location.apply", &bad).is_err(), "{bad}");
    }
    s.execute("location.setDefault", &json!({"album": album, "place": {"name": "Home", "lat": 45.5, "lon": -122.6}})).unwrap();
    assert_eq!(s.execute("location.default", &json!({"album": album})).unwrap()["place"]["name"], "Home");
    s.execute("location.setDefault", &json!({"album": album, "place": null})).unwrap();
    assert!(s.execute("location.default", &json!({"album": album})).unwrap()["place"].is_null());
    assert!(s.execute("location.default", &json!({"ids": [ids[0]]})).is_err(), "a selection has no place of its own");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A small procedural PNG (distinct per seed).
fn write_png(path: &Path, seed: u8) {
    let (w, h) = (40usize, 30usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, seed, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn new_imports_get_their_album_or_folders_place_and_xmp() {
    let dir = temp("import");
    let mut s = Session::new().with_fs();
    s.places.geocoder = Arc::new(Fake::default());
    s.places.cache_file = Some(dir.join("cache.json"));
    s.execute("library.xmpPreferences", &json!({"autoWrite": true})).unwrap();
    // a folder with a place: what is imported into it later gets the place (XMP written too)
    let shoot = dir.join("2024 C&J Wedding");
    write_png(&shoot.join("a.png"), 1);
    let r = s.execute("library.import", &json!({"paths": [shoot.join("a.png").to_string_lossy()]})).unwrap();
    let first = r["imported"][0].as_u64().unwrap();
    s.execute("location.apply", &json!({"folder": shoot.to_string_lossy(), "place": pike()})).unwrap();
    assert_eq!(meta(&s, first).city, "Seattle");
    let xmp = std::fs::read_to_string(shoot.join("a.xmp")).expect("the sidecar is written like other metadata edits");
    assert!(xmp.contains("Seattle") && xmp.contains("GPSLatitude"), "{xmp}");
    write_png(&shoot.join("b.png"), 2);
    s.execute("library.preferences", &json!({"import": {"autoKeywords": true}})).unwrap();
    let r = s.execute("library.import", &json!({"paths": [shoot.join("b.png").to_string_lossy()]})).unwrap();
    let second = r["imported"][0].as_u64().unwrap();
    assert_eq!(r["places"]["placed"], 1, "{r}");
    let m = meta(&s, second);
    assert_eq!((m.gps, m.city.as_str()), (Some((47.6097, -122.3422)), "Seattle"));
    assert!(m.keywords.iter().any(|k| k == "Seattle"), "place keywords: {:?}", m.keywords);
    assert!(m.keywords.iter().any(|k| k == "wedding"), "the folder's name keyword (auto on): {:?}", m.keywords);
    // an album with a place: photos imported into it get that one
    let album = s.execute("album.create", &json!({"name": "Sydney"})).unwrap()["id"].as_u64().unwrap();
    s.execute("location.setDefault", &json!({"album": album, "place": {"name": "Opera House", "lat": -33.8568, "lon": 151.2153, "location": "Sydney Opera House", "city": "Sydney", "country": "Australia"}})).unwrap();
    let other = dir.join("elsewhere");
    write_png(&other.join("c.png"), 3);
    s.execute("library.preferences", &json!({"import": {"placeKeywords": false}})).unwrap();
    let r = s.execute("library.import", &json!({"paths": [other.join("c.png").to_string_lossy()], "album": album})).unwrap();
    let third = r["imported"][0].as_u64().unwrap();
    let m = meta(&s, third);
    assert_eq!((m.city.as_str(), m.location.as_str()), ("Sydney", "Sydney Opera House"));
    assert!(!m.keywords.iter().any(|k| k == "Sydney"), "place keywords off: {:?}", m.keywords);
    // switched off: nothing
    s.execute("library.preferences", &json!({"import": {"autoPlace": false, "autoKeywords": false}})).unwrap();
    write_png(&shoot.join("d.png"), 4);
    let r = s.execute("library.import", &json!({"paths": [shoot.join("d.png").to_string_lossy()]})).unwrap();
    assert_eq!(meta(&s, r["imported"][0].as_u64().unwrap()).gps, None);
    let _ = std::fs::remove_dir_all(&dir);
}

fn keywords_of(v: &Value) -> Vec<(String, String)> {
    v["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x["keyword"].as_str().unwrap().to_string(), x["from"].as_str().unwrap().to_string()))
        .collect()
}

#[test]
fn suggested_keywords_from_place_name_and_content() {
    let (mut s, _fake, dir) = session("suggest");
    s.vision.dir = Some(dir.join("vision"));
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    let album = s.execute("album.create", &json!({"name": "*C&J WEDDING"})).unwrap()["id"].as_u64().unwrap();
    s.selection = crate::Selection::single(PhotoId(ids[0]));
    s.execute("album.addPhotos", &json!({"id": album, "ids": [ids[0], ids[1]]})).unwrap();
    // (the demo photos come with places of their own: none here)
    s.execute("photo.setMeta", &json!({"ids": [ids[0], ids[1]], "location": "", "city": "", "state": "", "country": "", "gps": null})).unwrap();
    // no AI search index: place and name suggestions, and why there are no content ones
    let r = s.execute("keywords.suggest", &json!({"album": album})).unwrap();
    assert_eq!(keywords_of(&r), vec![("wedding".to_string(), "name".to_string())], "{r}");
    assert!(r["content"].as_str().is_some_and(|c| c != "ok"), "{r}");
    assert_eq!(r["looksLike"], json!(["wedding"]));
    // the album gets a place (no keywords yet): its keywords are suggested first
    s.execute("location.apply", &json!({"album": album, "place": pike(), "keywords": false})).unwrap();
    // what the photos show: fake text vectors, the photos' "CLIP" embeddings in the index
    let unit = |i: usize| (0..8).map(|j| if j == i { 1.0 } else { 0.0 }).collect::<Vec<f32>>();
    s.places.labels = Some(Arc::new(vec![("bride".into(), unit(0)), ("beach".into(), unit(1)), (String::new(), unit(2))]));
    {
        let index = s.search_index().unwrap();
        let mut idx = index.lock().unwrap();
        for id in [ids[0], ids[1]] {
            let key = crate::media::content_key(s.catalog.photo(PhotoId(id)).unwrap());
            idx.put(&key, lightcraft_vision::index::Embedding(vec![0.95, 0.05, 0.1, 0.0, 0.0, 0.0, 0.0, 0.0])).unwrap();
        }
    }
    let r = s.execute("keywords.suggest", &json!({"album": album})).unwrap();
    let got = keywords_of(&r);
    for (k, from) in [("Pike Place Market", "place"), ("Seattle", "place"), ("market", "place"), ("wedding", "name"), ("bride", "content")] {
        assert!(got.contains(&(k.to_string(), from.to_string())), "{k} from {from} in {got:?}");
    }
    assert!(!got.iter().any(|(k, _)| k == "beach"), "{got:?}");
    assert_eq!(r["content"], "ok");
    // add two, drop one: one undo step each; a dropped suggestion isn't offered again
    let undo_len = s.undo.len();
    s.execute("keywords.applySuggested", &json!({"album": album, "keywords": ["wedding", "Seattle"]})).unwrap();
    assert_eq!(s.undo.len(), undo_len + 1);
    assert!(meta(&s, ids[1]).keywords.iter().any(|k| k == "wedding"));
    let r = s.execute("keywords.suggest", &json!({"album": album})).unwrap();
    let wedding = r["suggestions"].as_array().unwrap().iter().find(|x| x["keyword"] == "wedding").cloned().unwrap();
    assert_eq!(wedding["have"], 2, "on every photo now: {wedding}");
    s.execute("keywords.dropSuggested", &json!({"album": album, "keywords": ["market"]})).unwrap();
    assert!(!keywords_of(&s.execute("keywords.suggest", &json!({"album": album})).unwrap()).iter().any(|(k, _)| k == "market"));
    // − on an added one takes it off the photos (and it isn't suggested again)
    s.execute("keywords.applySuggested", &json!({"album": album, "remove": ["wedding"]})).unwrap();
    assert!(!meta(&s, ids[0]).keywords.iter().any(|k| k == "wedding"));
    assert!(!keywords_of(&s.execute("keywords.suggest", &json!({"album": album})).unwrap()).iter().any(|(k, _)| k == "wedding"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(meta(&s, ids[0]).keywords.iter().any(|k| k == "wedding"), "undo puts it back");
    // everything suggested at once (no list): what not every photo has yet
    let r = s.execute("keywords.applySuggested", &json!({"album": album})).unwrap();
    assert!(r["added"].as_array().unwrap().iter().any(|k| k == "bride"), "{r}");
    assert!(meta(&s, ids[0]).keywords.iter().any(|k| k == "Pike Place Market"));
    // a selection: suggestions from the albums most of it is in
    let r = s.execute("keywords.suggest", &json!({"ids": [ids[0]], "content": false})).unwrap();
    assert_eq!(r["scope"]["kind"], "photos");
    assert!(r["looksLike"].as_array().unwrap().iter().any(|k| k == "wedding"), "{r}");
    assert!(s.execute("keywords.dropSuggested", &json!({"album": album})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Library repairs for metadata read before the readers knew better: void GPS (0, 0) cleared,
/// placeholder lens names emptied and fixed-lens bodies named — each one undo step, with a dry run.
#[test]
fn repairs_void_gps_and_lens_names() {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    let set = |s: &mut Session, id: u64, f: &dyn Fn(&mut lightcraft_catalog::Meta)| {
        let mut m = meta(s, id);
        f(&mut m);
        s.commit("t", lightcraft_catalog::Op::SetMeta { id: PhotoId(id), meta: Box::new(m) }).unwrap();
    };
    s.execute("photo.setMeta", &json!({"ids": ids, "gps": null})).unwrap();
    set(&mut s, ids[0], &|m| m.gps = Some((0.0, 0.0)));
    set(&mut s, ids[1], &|m| m.gps = Some((0.0, 0.0)));
    set(&mut s, ids[2], &|m| m.gps = Some((0.0, 9.0)));
    let r = s.execute("library.fixVoidGps", &json!({"dryRun": true})).unwrap();
    assert_eq!((r["photos"].as_u64(), meta(&s, ids[0]).gps), (Some(2), Some((0.0, 0.0))), "a dry run only counts");
    let undo_len = s.undo.len();
    assert_eq!(s.execute("library.fixVoidGps", &json!({})).unwrap()["photos"], 2);
    assert_eq!(s.undo.len(), undo_len + 1);
    assert_eq!((meta(&s, ids[0]).gps, meta(&s, ids[2]).gps), (None, Some((0.0, 9.0))));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(meta(&s, ids[1]).gps, Some((0.0, 0.0)));
    // lenses
    set(&mut s, ids[0], &|m| (m.camera, m.lens) = ("LEICA M (Typ 262)".into(), "not selected".into()));
    set(&mut s, ids[1], &|m| (m.camera, m.lens) = ("RICOH IMAGING COMPANY, LTD. RICOH GR III".into(), "----".into()));
    set(&mut s, ids[2], &|m| (m.camera, m.lens) = ("Canon EOS R5".into(), "RF50mm F1.2 L USM".into()));
    let before = s.execute("library.fixLensNames", &json!({"dryRun": true})).unwrap()["photos"].as_u64().unwrap();
    assert!(before >= 2);
    s.execute("library.fixLensNames", &json!({})).unwrap();
    assert_eq!(meta(&s, ids[0]).lens, "");
    assert_eq!(meta(&s, ids[1]).lens, "GR LENS 18.3mm F2.8");
    assert_eq!(meta(&s, ids[2]).lens, "RF50mm F1.2 L USM");
    assert_eq!(s.execute("library.fixLensNames", &json!({"dryRun": true})).unwrap()["photos"], 0, "nothing left");
}
