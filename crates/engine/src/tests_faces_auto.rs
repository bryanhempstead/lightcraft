//! Faces found without being asked (Lightroom's Find faces in entire catalog / Only as needed), photos joining a
//! running face run, the expression scores of Assisted Culling and culling at import / in the background.
//! Placeholder model files (the right sizes, nothing in them) stand in for installed models: what is planned and
//! queued is checked, and a run that can't load them must end in an error, never a panic.

use serde_json::json;

use crate::Session;
use crate::vision::{FaceDetect, Kind};
use lightcraft_catalog::PhotoId;
use lightcraft_vision::index::{FaceEntry, FaceRecord};
use lightcraft_vision::models::{self, Package};

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-faces-auto-{}-{name}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A models folder where `packages` look installed (files of the pinned sizes, all zeros).
fn placeholder_models(name: &str, packages: &[Package]) -> std::path::PathBuf {
    let root = temp(name);
    for p in packages {
        let d = models::dir(&root, *p);
        std::fs::create_dir_all(&d).unwrap();
        for f in models::info(*p).files {
            std::fs::File::create(d.join(f.name)).unwrap().set_len(f.size).unwrap();
        }
        assert!(models::installed(&root, *p));
    }
    root
}

fn demo(name: &str, packages: &[Package]) -> (Session, Vec<PhotoId>) {
    let mut s = Session::with_demo();
    s.vision.dir = Some(temp(&format!("{name}-index")));
    s.vision.root = placeholder_models(name, packages);
    let ids: Vec<PhotoId> = s.catalog.photos().filter(|p| p.in_library()).map(|p| p.id).collect();
    (s, ids)
}

fn put_record(s: &mut Session, id: PhotoId, record: FaceRecord) {
    let key = crate::media::content_key(s.catalog.photo(id).unwrap());
    s.faces_index().unwrap().lock().unwrap().put(&key, record).unwrap();
    s.vision.generation += 1;
}

fn desc(mut v: Vec<PhotoId>) -> Vec<PhotoId> {
    v.sort_unstable_by(|a, b| b.cmp(a));
    v
}

#[test]
fn automatic_face_finding_plans_newest_first_and_pauses() {
    let (mut s, ids) = demo("plan", &[Package::Faces]);
    assert!(ids.len() >= 4);
    // off: nothing, whatever is open
    s.face_prefs.detect = FaceDetect::Off;
    assert_eq!(s.faces_auto_tick(true), 0);
    assert!(s.vision.auto_last_plan.is_empty());
    // as needed: only while the People view is open, and only the photos in view
    s.face_prefs.detect = FaceDetect::AsNeeded;
    assert_eq!(s.faces_auto_tick(false), 0);
    assert!(s.vision.auto_last_plan.is_empty());
    s.filter.only = ids[..3].to_vec();
    assert_eq!(s.faces_auto_tick(true), 0, "the placeholder models don't load");
    assert_eq!(s.vision.auto_last_plan, desc(ids[..3].to_vec()));
    let p = s.vision.progress(Kind::Faces);
    assert!(p.auto && !p.running && p.error.is_some(), "a clean error: {p:?}");
    // the same library isn't planned again right away
    s.vision.auto_last_plan.clear();
    assert_eq!(s.faces_auto_tick(true), 0);
    assert!(s.vision.auto_last_plan.is_empty());
    // all: every photo of the library, newest first, without the ones already analysed
    s.filter.only.clear();
    s.face_prefs.detect = FaceDetect::All;
    put_record(&mut s, ids[0], FaceRecord::default());
    s.vision.resume_auto();
    s.faces_auto_tick(false);
    assert_eq!(s.vision.auto_last_plan, desc(ids[1..].to_vec()));
    // paused (stop. on an automatic run): nothing until resumed
    s.vision.auto_last_plan.clear();
    let r = s.execute("faces.auto", &json!({"paused": true, "tick": true})).unwrap();
    assert_eq!(r["paused"], true);
    assert!(s.vision.auto_last_plan.is_empty());
    let r = s.execute("faces.auto", &json!({"paused": false, "tick": true})).unwrap();
    assert_eq!((r["paused"].as_bool(), r["mode"].as_str()), (Some(false), Some("all")));
    assert_eq!(s.vision.auto_last_plan.len(), ids.len() - 1);
    // without the models nothing is planned
    s.vision.root = temp("plan-none");
    s.vision.resume_auto();
    s.vision.auto_last_plan.clear();
    assert_eq!(s.faces_auto_tick(false), 0);
    assert!(s.vision.auto_last_plan.is_empty());
    assert!(s.execute("faces.auto", &json!({"mode": "sometimes"})).is_err());
    let st = s.execute("vision.status", &json!({})).unwrap();
    assert_eq!(st["faces"]["detect"], "all");
}

#[test]
fn photos_join_a_running_face_run_and_leftovers_run_again() {
    let (mut s, ids) = demo("queue", &[Package::Faces]);
    s.vision.fake_running(Kind::Faces, true);
    assert_eq!(s.vision_enqueue(Kind::Faces, &ids[..2]).unwrap(), 2);
    // an import's photos go ahead of what is left; photos already queued aren't queued twice
    assert_eq!(s.vision_enqueue(Kind::Faces, &[ids[2], ids[0]]).unwrap(), 1);
    assert_eq!(s.vision.queued(Kind::Faces), vec![ids[2], ids[0], ids[1]]);
    assert_eq!(s.vision.progress(Kind::Faces).total, 3);
    // analysed photos aren't queued at all
    put_record(&mut s, ids[3], FaceRecord::default());
    assert_eq!(s.vision_enqueue(Kind::Faces, &[ids[3]]).unwrap(), 0);
    // stopping an automatic run pauses automatic finding
    s.vision.cancel(Some(Kind::Faces));
    assert!(s.vision.auto_paused);
    // photos left in the queue of a run that ended on its own start a new run (here: the placeholder models fail)
    s.vision.resume_auto();
    s.vision.fake_running(Kind::Faces, false);
    s.vision_enqueue(Kind::Faces, &[ids[1]]).unwrap();
    s.vision.fake_finish(Kind::Faces);
    s.vision_poll();
    let p = s.vision.progress(Kind::Faces);
    assert!(!p.auto && p.total == 1 && p.error.as_deref().is_some_and(|e| e.contains("model")), "{p:?}");
}

#[test]
fn installing_the_expression_model_analyses_photos_again() {
    let (mut s, ids) = demo("expr", &[Package::Faces]);
    put_record(&mut s, ids[0], FaceRecord { expressions: false, ..Default::default() });
    put_record(&mut s, ids[1], FaceRecord { expressions: true, ..Default::default() });
    let todo = |s: &mut Session| s.vision_items(Kind::Faces, &ids[..2], false).unwrap().iter().map(|i| i.id).collect::<Vec<_>>();
    assert!(todo(&mut s).is_empty(), "without the expression model both are done");
    s.vision.root = placeholder_models("expr2", &[Package::Faces, Package::Expressions]);
    assert_eq!(todo(&mut s), vec![ids[0]], "with it, the photo analysed before it is not");
    // a hand-drawn unnamed face gets its embedding like a named one
    let rect = lightcraft_meta::Rect { x0: 0.1, y0: 0.1, x1: 0.3, y1: 0.4 };
    s.add_face(ids[2], rect, None).unwrap();
    let p = s.catalog.photo(ids[2]).unwrap();
    assert!(p.meta.regions.last().unwrap().drawn);
    let items = s.vision_items(Kind::Faces, &[ids[2]], true).unwrap();
    assert_eq!(items[0].regions.len(), 1);
}

fn entry(x: f32, happy: f32, neutral: f32, sad: f32) -> FaceEntry {
    let mut e = [0f32; 7];
    e[lightcraft_vision::expression::HAPPY] = happy;
    e[lightcraft_vision::expression::NEUTRAL] = neutral;
    e[lightcraft_vision::expression::SAD] = sad;
    FaceEntry {
        rect: [x, 0.2, 0.2, 0.25],
        landmarks: [[0.0; 2]; 5],
        score: 0.9,
        embedding: vec![0.0; 128],
        eyes: Some([0.05, 0.05]),
        sharpness: 40.0,
        expression: Some(e),
    }
}

#[test]
fn smiles_and_expressions_in_assisted_culling() {
    // the scores of one photo: two main faces, one smiling
    let r = FaceRecord { faces: vec![entry(0.1, 0.9, 0.1, 0.0), entry(0.5, 0.05, 0.25, 0.7)], width: 100, height: 100, expressions: true };
    let f = crate::cmd::cull_scores(&r);
    assert_eq!(f.smiles, 1);
    let e = f.expression.unwrap();
    assert!((e - 100.0 * (0.95 + 0.175) / 2.0).abs() < 0.01, "{e}");
    // a record made before the expression model: no expression score
    let old = FaceRecord { expressions: false, ..r.clone() };
    assert_eq!(crate::cmd::cull_scores(&old).expression, None);

    // a burst of two: the smiling one is its best although the other is a little sharper
    let (mut s, ids) = demo("burst", &[]);
    put_record(&mut s, ids[0], FaceRecord { faces: vec![entry(0.3, 0.92, 0.08, 0.0)], width: 100, height: 100, expressions: true });
    put_record(&mut s, ids[1], FaceRecord { faces: vec![entry(0.3, 0.02, 0.3, 0.68)], width: 100, height: 100, expressions: true });
    let row = |id: PhotoId, sharp: f32| crate::cmd::cull::Row { id, t: Some(100), sharp, clip: 0.0, sig: [0.5; 64] };
    let opts = crate::cmd::cull::CullOptions { reject_expression_below: Some(30.0), ..Default::default() };
    let out = crate::cmd::cull::cull_finish(&mut s, vec![row(ids[0], 50.0), row(ids[1], 60.0)], Vec::new(), opts, 0).unwrap();
    assert_eq!(out["groups"], 1, "{out}");
    let a0 = s.catalog.photo(ids[0]).unwrap().analysis.unwrap();
    let a1 = s.catalog.photo(ids[1]).unwrap().analysis.unwrap();
    assert!(a0.best && !a1.best, "{out}");
    assert_eq!((a0.smiles, a1.smiles), (1, 0));
    assert!(a0.expression.unwrap() > 90.0 && a1.expression.unwrap() < 30.0);
    // the frowning one is rejected below the expression score; one undo step
    assert_eq!(s.catalog.photo(ids[1]).unwrap().flag, lightcraft_catalog::Flag::Reject);
    assert_eq!(out["rejected"], 1);
    // smart album fields
    let rules = |field: &str, v: f64| json!({"ruleSet": {"match": "all", "rules": [{"field": field, "op": "gte", "value": v}]}});
    s.execute("album.createSmart", &json!({"name": "Smiles", "rules": rules("smiles", 1.0)})).unwrap();
    let album = s.catalog.albums().find(|a| a.name == "Smiles").unwrap().id;
    s.execute("library.source", &json!({"kind": "album", "id": album.0})).unwrap();
    assert_eq!(s.visible_cloned(), vec![ids[0]]);
}

#[test]
fn culling_in_the_background_and_at_import() {
    // in the background: measured on worker threads, applied by cull_poll as one undo step
    let (mut s, ids) = demo("bg", &[]);
    s.vision.background = true;
    let undo0 = s.undo.len();
    let r = s.execute("photo.analyze", &json!({"ids": [ids[0].0, ids[1].0, ids[2].0], "background": true})).unwrap();
    assert_eq!(r["queued"], 3, "{r}");
    let t0 = std::time::Instant::now();
    let out = loop {
        if let Some(r) = s.cull_poll() {
            break r.unwrap();
        }
        assert!(t0.elapsed().as_secs() < 60, "culling never finished");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(out["photos"].as_array().unwrap().len(), 3, "{out}");
    assert!(ids[..3].iter().all(|id| s.catalog.photo(*id).unwrap().analysis.is_some()));
    assert_eq!(s.undo.len(), undo0 + 1);
    assert!(s.cull_job.is_none() && s.cull_poll().is_none());
    // bookkeeping (culling at import): no undo step, never a flag
    let r = s.execute("photo.analyze", &json!({"ids": [ids[3].0], "bookkeeping": true, "rejectBelow": 101})).unwrap();
    assert_eq!(r["rejected"], 0);
    assert_eq!(s.undo.len(), undo0 + 1);
    assert!(s.catalog.photo(ids[3]).unwrap().analysis.is_some());
    // a run can be stopped before it lands
    s.execute("photo.analyze", &json!({"ids": [ids[4].0], "background": true})).unwrap();
    assert!(s.cull_cancel());
    assert!(s.cull_poll().is_none());

    // library.import {cull: true}: the new photos are scored as they come in, inside the import's undo step
    let src = temp("import-src");
    for i in 0..2u8 {
        let img = lightcraft_raster::Rgba8 {
            width: 40,
            height: 30,
            data: (0..40 * 30).map(|k| [(k % 40 * 6) as u8, i * 90, (k / 40 * 8) as u8, 255]).collect(),
        };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(src.join(format!("p{i}.png")), png).unwrap();
    }
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [src.to_string_lossy()], "cull": true})).unwrap();
    assert_eq!(r["culled"]["photos"].as_array().map(Vec::len), Some(2), "{r}");
    assert!(s.catalog.photos().all(|p| p.analysis.is_some()));
    assert_eq!(s.undo.len(), 1, "the import is one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.len(), 0);
}

#[test]
fn face_prefs_are_saved_with_the_library() {
    let dir = temp("prefs");
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    assert_eq!(s.face_prefs.detect, FaceDetect::AsNeeded, "Lightroom's default: only as needed");
    s.execute("faces.auto", &json!({"mode": "all"})).unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    assert_eq!(s.face_prefs.detect, FaceDetect::All);
}

/// With the real models (`LC_VISION_MODELS` holding faces/ and expressions/, `LC_VISION_IMG` the public-domain
/// portraits of `crates/vision/tests/real_models.rs`): import, find faces (expressions included), cull, print.
#[test]
#[ignore]
fn real_expressions_end_to_end() {
    let (Some(models), Some(img)) = (std::env::var_os("LC_VISION_MODELS"), std::env::var_os("LC_VISION_IMG")) else { return };
    let mut s = Session::new().with_fs();
    s.vision.root = models.into();
    s.vision.dir = Some(temp("real-index"));
    let r = s.execute("library.import", &json!({"paths": [img.to_string_lossy()]})).unwrap();
    let n = r["imported"].as_array().map_or(0, Vec::len);
    let t = std::time::Instant::now();
    s.execute("faces.find", &json!({"scope": "all", "wait": true})).unwrap();
    println!("{n} photos analysed in {:?}", t.elapsed());
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    let out = s.execute("photo.analyze", &json!({"ids": ids})).unwrap();
    let name = |s: &Session, v: &serde_json::Value| s.catalog.photo(PhotoId(v["id"].as_u64().unwrap())).unwrap().file_name.clone();
    for p in out["photos"].as_array().unwrap() {
        println!("{}: faces {} smiles {} expression {} eyes closed {}", name(&s, p), p["faces"], p["smiles"], p["expression"], p["eyesClosed"]);
    }
    let by_name = |n: &str| out["photos"].as_array().unwrap().iter().find(|p| name(&s, p).starts_with(n)).cloned().unwrap();
    assert!(by_name("President_Barack_Obama")["smiles"].as_u64() >= Some(1));
    assert!(by_name("Official_portrait_of_Barack_Obama")["expression"].as_f64().is_some());
}

/// While the whole library is analysed, newly imported photos join the front of the automatic run.
#[test]
fn new_photos_join_the_front_of_an_automatic_run() {
    let (mut s, ids) = demo("front", &[Package::Faces]);
    s.face_prefs.detect = FaceDetect::All;
    s.vision.fake_running(Kind::Faces, true);
    // a run it didn't plan: it knows of no photo yet, so all of them join (newest first)
    assert_eq!(s.faces_auto_tick(false), ids.len());
    assert_eq!(s.vision.queued(Kind::Faces), desc(ids.clone()));
    // nothing new: nothing more (even after the throttle)
    s.vision.resume_auto();
    assert_eq!(s.faces_auto_tick(false), 0);
    // a new photo (a virtual copy stands in for an import) goes first
    s.selection = crate::Selection::single(ids[1]);
    s.execute("photo.virtualCopy", &json!({})).unwrap();
    let new = s.catalog.photos().map(|p| p.id).max().unwrap();
    assert!(new > *ids.iter().max().unwrap());
    s.vision.resume_auto();
    assert_eq!(s.faces_auto_tick(false), 1);
    assert_eq!(s.vision.queued(Kind::Faces).first(), Some(&new));
    // a run the user started (find.) isn't added to
    s.vision.fake_running(Kind::Faces, false);
    s.execute("photo.virtualCopy", &json!({})).unwrap();
    s.vision.resume_auto();
    assert_eq!(s.faces_auto_tick(false), 0);
}
