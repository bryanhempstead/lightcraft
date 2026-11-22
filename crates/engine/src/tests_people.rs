//! People: naming faces (person keywords), suggestions with ✓ / ✗, unnamed stacks, renaming, removing,
//! drawing a face, XMP regions; the models' absence; AI search's filter. Synthetic embeddings stand in for the
//! models here. With the models and photos (`LC_VISION_MODELS`, `LC_VISION_IMG`: see
//! `crates/vision/tests/real_models.rs`) `real_models_end_to_end` runs the whole flow and measures it.

use serde_json::json;

use crate::Session;
use crate::people::FaceRef;
use lightcraft_catalog::{Op, PhotoId};
use lightcraft_vision::index::{FaceEntry, FaceRecord};

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-people-{}-{name}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A unit vector near axis `k` (people are axes; `jitter` makes each face a little different).
fn person(k: usize, jitter: f32) -> Vec<f32> {
    let mut v = vec![0f32; 128];
    v[k] = 1.0;
    v[(k + 7) % 128] = jitter;
    lightcraft_vision::sface::normalize(v)
}

fn face(x: f32, embedding: Vec<f32>) -> FaceEntry {
    FaceEntry { rect: [x, 0.2, 0.15, 0.2], landmarks: [[0.0; 2]; 5], score: 0.9, embedding, eyes: Some([0.1, 0.1]), sharpness: 40.0 }
}

/// A session with demo photos whose faces are indexed (one face per photo: `people[i]` is the person on photo i).
fn session(people: &[usize]) -> (Session, Vec<PhotoId>) {
    let mut s = Session::with_demo();
    s.vision.dir = Some(temp("index"));
    s.vision.root = temp("no-models");
    let ids: Vec<PhotoId> = s.catalog.photos().map(|p| p.id).take(people.len()).collect();
    assert_eq!(ids.len(), people.len(), "enough demo photos");
    let index = s.faces_index().unwrap();
    for (i, (id, k)) in ids.iter().zip(people).enumerate() {
        let key = crate::media::content_key(s.catalog.photo(*id).unwrap());
        let record = FaceRecord { faces: vec![face(0.3, person(*k, 0.05 * i as f32))], width: 100, height: 100 };
        index.lock().unwrap().put(&key, record.clone()).unwrap();
        let op = s.merge_found_faces(*id, &record).unwrap();
        s.apply_bookkeeping(op).unwrap();
    }
    s.vision.generation += 1;
    (s, ids)
}

#[test]
fn naming_suggesting_confirming_and_rejecting() {
    // photos 0..3: Ann, Ann, Ann, Bob, Bob
    let (mut s, ids) = session(&[1, 1, 1, 2, 2]);
    let f = |i: usize| json!({"photo": ids[i].0, "index": 0});
    let list = s.execute("people.list", &json!({"faces": true})).unwrap();
    assert_eq!(list["namedCount"], 0);
    assert_eq!(list["stacks"], 2, "{list}");
    assert_eq!(list["unnamed"][0]["faces"], 3, "the largest stack first");
    // name one: region, person keyword
    s.execute("people.name", &json!({"faces": [f(0)], "name": "Ann"})).unwrap();
    let p0 = s.catalog.photo(ids[0]).unwrap();
    assert_eq!(p0.meta.regions[0].name.as_deref(), Some("Ann"));
    assert!(p0.meta.keywords.contains(&"Ann".to_string()));
    assert!(s.catalog.keyword_info("Ann").unwrap().person);
    // the others are suggested as Ann; Bob's aren't
    let list = s.execute("people.list", &json!({"faces": true})).unwrap();
    assert_eq!(list["named"][0]["name"], "Ann");
    assert_eq!(list["named"][0]["similar"], 2, "{list}");
    let ann_stack = list["unnamed"].as_array().unwrap().iter().find(|st| st["faces"] == 2 && st["suggestion"] == "Ann");
    assert!(ann_stack.is_some(), "{list}");
    // ✓ one, ✗ the other
    s.execute("people.confirm", &json!({"faces": [f(1)]})).unwrap();
    assert_eq!(s.catalog.photo(ids[1]).unwrap().meta.regions[0].name.as_deref(), Some("Ann"));
    s.execute("people.reject", &json!({"faces": [f(2)]})).unwrap();
    assert_eq!(s.catalog.photo(ids[2]).unwrap().meta.regions[0].rejected, vec!["Ann".to_string()]);
    let list = s.execute("people.list", &json!({"faces": true})).unwrap();
    assert_eq!(list["named"][0]["similar"], 0, "a rejected face isn't suggested again: {list}");
    // the person filter finds Ann's photos
    s.execute("library.filter", &json!({"person": "ann"})).unwrap();
    assert_eq!(s.visible_cloned().len(), 2);
    s.execute("library.filter", &json!({"person": null})).unwrap();
    // undo the confirmation: unnamed again, no keyword
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.photo(ids[1]).unwrap().meta.regions[0].name.is_none());
    assert!(!s.catalog.photo(ids[1]).unwrap().meta.keywords.contains(&"Ann".to_string()));
    // name a whole stack, rename, unname, remove
    s.execute("people.name", &json!({"faces": [f(3), f(4)], "name": "Bob"})).unwrap();
    s.execute("people.rename", &json!({"from": "bob", "to": "Robert"})).unwrap();
    let p3 = s.catalog.photo(ids[3]).unwrap();
    assert_eq!(p3.meta.regions[0].name.as_deref(), Some("Robert"));
    assert!(p3.meta.keywords.contains(&"Robert".to_string()) && !p3.meta.keywords.contains(&"Bob".to_string()), "{:?}", p3.meta.keywords);
    s.execute("people.unname", &json!({"faces": [f(3)]})).unwrap();
    assert!(!s.catalog.photo(ids[3]).unwrap().meta.keywords.contains(&"Robert".to_string()));
    s.execute("people.removeFaces", &json!({"faces": [f(4)]})).unwrap();
    let p4 = s.catalog.photo(ids[4]).unwrap();
    assert!(p4.meta.regions.is_empty() && !p4.meta.keywords.contains(&"Robert".to_string()));
    // bad input is an error, never a panic
    assert!(s.execute("people.name", &json!({"faces": [{"photo": 999999, "index": 0}], "name": "X"})).is_err());
    assert!(s.execute("people.name", &json!({"faces": [f(0)], "name": "a|b"})).is_err());
    assert!(s.execute("people.name", &json!({"faces": "nope", "name": "X"})).is_err());
    assert!(s.execute("people.rename", &json!({"from": "Nobody", "to": "X"})).is_err());
}

#[test]
fn an_existing_person_keyword_is_reused_and_export_respects_it() {
    let (mut s, ids) = session(&[1]);
    s.apply_bookkeeping(Op::SetKeyword {
        path: "People|Ann".into(),
        info: Some(lightcraft_catalog::keywords::KeywordInfo { person: true, include_on_export: false, ..Default::default() }),
    })
    .unwrap();
    s.name_faces(&[FaceRef { photo: ids[0], index: 0 }], Some("ann"), "Name Person").unwrap();
    let p = s.catalog.photo(ids[0]).unwrap().clone();
    assert!(p.meta.keywords.contains(&"People|Ann".to_string()), "{:?}", p.meta.keywords);
    // XMP: the named face is written, with its name
    let x = crate::sidecar::sidecar_packet(&p, &s.catalog);
    assert!(x.contains("<mwg-rs:Name>ann</mwg-rs:Name>"), "{x}");
    // a sidecar read back keeps the catalog's unnamed faces
    let mut file = p.meta.regions.clone();
    file[0].rejected.clear();
    let extra = lightcraft_meta::Region::face(lightcraft_meta::Rect { x0: 0.7, y0: 0.1, x1: 0.8, y1: 0.2 });
    let merged = crate::sidecar::merge_regions(&file, &[p.meta.regions[0].clone(), extra.clone()]);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[1], extra);
}

#[test]
fn drawing_a_face_and_missing_models() {
    let (mut s, ids) = session(&[3]);
    s.selection = crate::Selection::single(ids[0]);
    let r = s.execute("photo.addFace", &json!({"rect": {"x0": 0.6, "y0": 0.1, "x1": 0.75, "y1": 0.3}, "name": "Cy"})).unwrap();
    assert_eq!(r["index"], 1);
    let p = s.catalog.photo(ids[0]).unwrap();
    assert_eq!(p.meta.regions[1].name.as_deref(), Some("Cy"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(ids[0]).unwrap().meta.regions.len(), 1, "one undo step");
    assert!(s.execute("photo.addFace", &json!({"rect": {"x0": 0.6, "y0": 0.1, "x1": 0.6, "y1": 0.3}})).is_err());
    // no models: a clear error that names the download
    let e = s.execute("faces.find", &json!({"scope": "all"})).unwrap_err().to_string();
    assert!(e.contains(lightcraft_vision::models::NOT_INSTALLED) && e.contains("vision.model.download"), "{e}");
    let e = s.execute("search.ai", &json!({"query": "a dog"})).unwrap_err().to_string();
    assert!(e.contains("AI search") || e.contains(lightcraft_vision::models::NOT_INSTALLED), "{e}");
    let e = s.execute("vision.model.download", &json!({"model": "faces"})).unwrap_err().to_string();
    assert!(e.contains("acknowledged") && e.contains("MB"), "{e}");
    let st = s.execute("vision.status", &json!({})).unwrap();
    assert_eq!(st["faces"]["model"]["installed"], false);
    assert_eq!(st["faces"]["indexed"], 1);
}

#[test]
fn ai_search_filters_with_an_index() {
    // the ranking needs the text model; without it the filter shows nothing rather than everything
    let (mut s, _) = session(&[1, 2]);
    s.filter.semantic = Some("beach".into());
    assert!(s.visible_cloned().is_empty());
    s.filter.semantic = None;
    assert_eq!(s.visible_cloned().len(), s.catalog.len());
}

/// Throughput on a folder of real photos (`LC_VISION_BENCH`, with `LC_VISION_MODELS`): faces and search indexing,
/// photos a second. Run in release: `cargo test -p lightcraft-engine --release --features enhance
/// tests_people::bench -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_indexing() {
    let (Some(models), Some(dir)) = (std::env::var_os("LC_VISION_MODELS"), std::env::var_os("LC_VISION_BENCH")) else { return };
    let mut s = Session::new().with_fs();
    s.vision.root = models.into();
    s.vision.dir = Some(temp("bench"));
    let r = s.execute("library.import", &json!({"paths": [std::path::PathBuf::from(dir).to_string_lossy()]})).unwrap();
    let n = s.catalog.len();
    println!("imported {n} ({} failed)", r["failed"].as_array().map(Vec::len).unwrap_or(0));
    let t = std::time::Instant::now();
    let r = s.execute("faces.find", &json!({"scope": "all"})).unwrap();
    let dt = t.elapsed().as_secs_f64();
    println!("faces: {n} photos in {dt:.1} s = {:.2} photos/s, {} faces, failed {}", n as f64 / dt, r["progress"]["faces"], r["progress"]["failed"]);
    let t = std::time::Instant::now();
    let people = s.execute("people.list", &json!({})).unwrap();
    println!("people.list: {} stacks in {:?}", people["stacks"], t.elapsed());
    let t = std::time::Instant::now();
    s.execute("search.index", &json!({})).unwrap();
    let dt = t.elapsed().as_secs_f64();
    println!("search: {n} photos in {dt:.1} s = {:.2} photos/s", n as f64 / dt);
    for q in ["people smiling at a wedding", "a waterfall", "a dog", "food on a table"] {
        let t = std::time::Instant::now();
        let r = s.execute("search.ai", &json!({"query": q, "limit": 3})).unwrap();
        let names: Vec<String> = r["top"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| s.catalog.photo(PhotoId(x["id"].as_u64().unwrap())).map(|p| format!("{} {}", p.file_name, x["score"])))
            .collect();
        println!("{q:?}: {} matches {names:?} ({:?})", r["matches"], t.elapsed());
    }
    let r = s.execute("photo.analyze", &json!({"ids": s.catalog.photos().map(|p| p.id.0).collect::<Vec<_>>()})).unwrap();
    for p in r["photos"].as_array().unwrap() {
        if p["eyesClosed"].as_u64().unwrap_or(0) > 0 {
            let name = s.catalog.photo(PhotoId(p["id"].as_u64().unwrap())).map(|q| q.file_name.clone()).unwrap_or_default();
            println!("blink: {name} faces {} eyesClosed {} faceFocus {}", p["faces"], p["eyesClosed"], p["faceFocus"]);
        }
    }
}

/// The real models on real photos: faces found, people named and suggested, search ranked; with timings.
#[test]
#[ignore]
fn real_models_end_to_end() {
    let (Some(models), Some(img)) = (std::env::var_os("LC_VISION_MODELS"), std::env::var_os("LC_VISION_IMG")) else { return };
    let mut s = Session::new().with_fs();
    s.vision.root = models.into();
    s.vision.dir = Some(temp("real"));
    let img = std::path::PathBuf::from(img);
    let names = [
        "Solvay_conference_1927.jpg",
        "President_Barack_Obama.jpg",
        "Official_portrait_of_Barack_Obama.jpg",
        "Donald_Trump_official_portrait.jpg",
        "closed_Dylan_LeBlan.jpg",
    ];
    let paths: Vec<String> = names.iter().map(|n| img.join(n).to_string_lossy().into_owned()).collect();
    s.execute("library.import", &json!({"paths": paths})).unwrap();
    let id_of = |s: &Session, n: &str| s.catalog.photos().find(|p| p.file_name == n).map(|p| p.id).unwrap();
    let t = std::time::Instant::now();
    let r = s.execute("faces.find", &json!({"scope": "all"})).unwrap();
    println!("faces.find: {r} in {:?}", t.elapsed());
    let solvay = id_of(&s, names[0]);
    assert_eq!(s.catalog.photo(solvay).unwrap().meta.regions.len(), 29);
    let (o1, o2) = (id_of(&s, names[1]), id_of(&s, names[2]));
    s.execute("people.name", &json!({"faces": [{"photo": o1.0, "index": 0}], "name": "Barack Obama"})).unwrap();
    let list = s.execute("people.list", &json!({"faces": true})).unwrap();
    let similar = &list["named"][0]["similarList"];
    assert_eq!(similar.as_array().map(Vec::len), Some(1), "{list}");
    assert_eq!(similar[0]["photo"], o2.0);
    println!("stacks {}, suggestion score {}", list["stacks"], similar[0]["score"]);
    // a second run has nothing to do
    assert_eq!(s.execute("faces.find", &json!({"scope": "all"})).unwrap()["queued"], 0);
    // culling: eyes closed on the closed-eyes portrait
    let r = s.execute("photo.analyze", &json!({"ids": s.catalog.photos().map(|p| p.id.0).collect::<Vec<_>>()})).unwrap();
    println!("analyze: {r}");
    let dylan = id_of(&s, names[4]);
    assert_eq!(s.catalog.photo(dylan).unwrap().analysis.unwrap().eyes_closed, 1);
    assert_eq!(s.catalog.photo(o1).unwrap().analysis.unwrap().eyes_closed, 0);
    // AI search
    let t = std::time::Instant::now();
    s.execute("search.index", &json!({})).unwrap();
    println!("search.index: {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let r = s.execute("search.ai", &json!({"query": "a black and white group photo of many men"})).unwrap();
    println!("search.ai: {r} in {:?}", t.elapsed());
    assert_eq!(r["top"][0]["id"], solvay.0);
    assert_eq!(s.visible_cloned().first(), Some(&solvay));
}
