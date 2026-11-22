//! cameras.*: the library's cameras, cameras added by hand, new cameras recognised on import.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;

struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn setup(name: &str) -> (Session, Tmp) {
    let dir = std::env::temp_dir().join(format!("lc-cameras-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.cameras_file = Some(dir.join("config/cameras.json"));
    (s, Tmp(dir))
}

fn dng(dir: &std::path::Path, name: &str, make: &str, model: &str, lens: &str) -> String {
    let meta = lightcraft_meta::Metadata { make: Some(make.into()), model: Some(model.into()), lens_model: Some(lens.into()), ..Default::default() };
    let path = dir.join("files").join(name);
    std::fs::write(&path, crate::tests_xmp::synthetic_dng_with(None, meta)).unwrap();
    path.to_string_lossy().into_owned()
}

fn camera<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["cameras"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap_or_else(|| panic!("{name} not in {list}"))
}

#[test]
fn a_new_camera_is_recognised_on_import_once() {
    let (mut s, t) = setup("import");
    let a = dng(&t.0, "a.dng", "Zeta", "Zeta Z-1", "Zeta 35mm F2");
    let r = s.execute("library.import", &json!({"paths": [a]})).unwrap();
    assert_eq!(r["imported"].as_array().unwrap().len(), 1);
    let new = r["newCameras"].as_array().expect("newCameras");
    assert_eq!(new.len(), 1);
    assert_eq!(new[0]["name"], "Zeta Z-1");
    assert!(new[0]["notice"].as_str().unwrap().starts_with("New camera: Zeta Z-1"), "{}", new[0]);
    assert!(t.0.join("config/cameras.json").is_file(), "resolved data is cached");
    // the same camera again: nothing new to say
    let b = dng(&t.0, "b.dng", "Zeta", "Zeta Z-1", "Zeta 50mm F1.8");
    let r = s.execute("library.import", &json!({"paths": [b]})).unwrap();
    assert!(r.get("newCameras").is_none(), "{r}");

    let list = s.execute("cameras.list", &json!({})).unwrap();
    let c = camera(&list, "Zeta Z-1");
    assert_eq!(c["photos"], 2);
    assert_eq!(c["raws"], 2);
    assert_eq!(c["decoder"], "native");
    assert_eq!(c["format"], "Dng");
    assert_eq!(c["make"], "Zeta");
    assert_eq!(c["profile"]["status"], "embedded", "a DNG with its own matrices, no Adobe profile for a made-up camera");
    // resolved once on import, with the one lens seen then; refresh picks up the second
    assert_eq!(c["lenses"].as_array().unwrap().len(), 1);
    let list = s.execute("cameras.refresh", &json!({"names": ["Zeta Z-1"]})).unwrap();
    assert_eq!(list["refreshed"], json!(["Zeta Z-1"]));
    let lenses = camera(&list, "Zeta Z-1")["lenses"].as_array().unwrap().clone();
    assert_eq!(lenses.len(), 2);
    assert!(lenses.iter().all(|l| l["photos"] == 1 && l["profile"].is_null()), "{lenses:?}");
}

#[test]
fn an_offline_raw_says_so_and_never_fails() {
    let (mut s, t) = setup("offline");
    let a = dng(&t.0, "a.dng", "Zeta", "Zeta Z-2", "");
    s.execute("library.import", &json!({"paths": [a.clone()]})).unwrap();
    std::fs::remove_file(&a).unwrap();
    let list = s.execute("cameras.refresh", &json!({"all": true})).unwrap();
    let c = camera(&list, "Zeta Z-2");
    assert_eq!(c["decoder"], "unknown");
    assert_eq!(c["decoderNote"], "original offline");
}

#[test]
fn cameras_added_by_hand_are_kept_and_removed() {
    let (mut s, _t) = setup("add");
    assert!(s.execute("cameras.add", &json!({})).is_err());
    assert!(s.execute("cameras.add", &json!({"name": "   "})).is_err());
    assert!(s.execute("cameras.add", &json!({"name": "x".repeat(200)})).is_err());
    let r = s.execute("cameras.add", &json!({"name": "Made Up  Cam 9000"})).unwrap();
    assert_eq!(r["name"], "Made Up Cam 9000");
    assert_eq!(r["profile"], "fallback");
    let list = s.execute("cameras.list", &json!({})).unwrap();
    let c = camera(&list, "Made Up Cam 9000");
    assert_eq!((c["photos"].as_u64(), c["added"].as_bool(), c["inLibrary"].as_bool()), (Some(0), Some(true), Some(false)));
    assert_eq!(c["make"], "Made");
    // kept across sessions (the cache file)
    let mut s2 = Session::new();
    s2.cameras_file = s.cameras_file.clone();
    let list = s2.execute("cameras.list", &json!({})).unwrap();
    camera(&list, "Made Up Cam 9000");
    assert_eq!(s.execute("cameras.remove", &json!({"name": "made up cam 9000"})).unwrap()["removed"], true);
    let list = s.execute("cameras.list", &json!({})).unwrap();
    assert!(list["cameras"].as_array().unwrap().is_empty(), "{list}");
    assert_eq!(s.execute("cameras.remove", &json!({"name": "nope"})).unwrap()["removed"], false);
    assert!(s.execute("cameras.refresh", &json!({"names": 3})).is_err());
    assert!(s.execute("cameras.refresh", &json!({"names": ["a", 3]})).is_err());
}

#[test]
fn lightroom_delta_e_and_damaged_files() {
    let (mut s, t) = setup("de");
    let a = dng(&t.0, "a.dng", "Zeta", "Zeta Z-3", "");
    s.execute("library.import", &json!({"paths": [a]})).unwrap();
    std::fs::write(t.0.join("config/lr-compare.json"), r#"{"cameras": {"zeta z-3": {"de": 2.52, "de128": 1.9, "photos": 40, "tag": "fin"}}}"#)
        .unwrap();
    let list = s.execute("cameras.list", &json!({})).unwrap();
    assert_eq!(camera(&list, "Zeta Z-3")["deltaE"]["de"], 2.52);
    // damaged caches: listed unresolved, never an error; refresh writes them again
    std::fs::write(t.0.join("config/cameras.json"), b"{not json").unwrap();
    std::fs::write(t.0.join("config/lr-compare.json"), b"\xff\xfe").unwrap();
    let list = s.execute("cameras.list", &json!({})).unwrap();
    let c = camera(&list, "Zeta Z-3");
    assert_eq!((c["resolved"].as_bool(), c["photos"].as_u64()), (Some(false), Some(1)));
    assert!(c["deltaE"].is_null());
    s.execute("cameras.refresh", &json!({})).unwrap();
    let list = s.execute("cameras.list", &json!({})).unwrap();
    assert_eq!(camera(&list, "Zeta Z-3")["resolved"], true);
    // a newer format is left alone (not read, not overwritten by a list)
    std::fs::write(t.0.join("config/cameras.json"), r#"{"version": 99, "cameras": []}"#).unwrap();
    s.execute("cameras.list", &json!({})).unwrap();
    assert!(std::fs::read_to_string(t.0.join("config/cameras.json")).unwrap().contains("99"));
}

#[test]
fn in_memory_sessions_write_nothing() {
    let mut s = Session::new();
    assert!(crate::cameras::store_path(&s).is_none());
    s.execute("cameras.add", &json!({"name": "Made Up Cam 1"})).unwrap();
    let list = s.execute("cameras.list", &json!({})).unwrap();
    camera(&list, "Made Up Cam 1");
    assert!(s.execute("cameras.known", &json!({"query": "zzzz-no-such", "limit": 5})).unwrap().as_array().unwrap().is_empty());
}
