//! LC-SB-PIPELINE: Second Brain's shoot pipeline read from a sample `shootpipe.json` in a temp
//! folder (never the real one), and `pipeline.add` queueing a request in its inbox.

use serde_json::{Value, json};

use crate::Session;

struct Tmp(std::path::PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(tag: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("lc-sbpipe-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

/// A pipeline like Bryan's: a wedding in editing with camera folders under raw/, a shoot
/// delivering, an archived one.
fn sample(stage: &str) -> Value {
    json!({"v": 1, "clock": null, "settings": {}, "bjobs": [], "dismissed": [], "shoots": [
        {"id": "s1", "path": "/Volumes/D/Wedding", "name": "Wedding", "stage": stage, "archived": null,
         "children": [{"path": "/Volumes/D/Wedding/raw/M262", "name": "raw/M262"}, {"path": "/Volumes/Backup/Wedding/GR3", "name": "GR3", "tied": true}]},
        {"id": "s2", "path": "/Volumes/D/Portraits", "name": "Portraits", "stage": "dlvr", "children": []},
        {"id": "s3", "path": "/Volumes/D/Old", "name": "Old", "stage": "done", "archived": 1763000000000u64, "children": []},
    ]})
}

fn session(dir: &std::path::Path) -> Session {
    let mut s = Session::with_demo();
    s.second_brain_dir = Some(dir.to_path_buf());
    s
}

fn status(s: &mut Session, folder: &str) -> Value {
    s.execute("pipeline.status", &json!({"folder": folder})).unwrap()
}

#[test]
fn reads_the_pipeline_and_marks_shoots_parts_and_folders_inside() {
    let t = tmp("read");
    std::fs::write(t.0.join("shootpipe.json"), serde_json::to_vec(&sample("edit")).unwrap()).unwrap();
    let mut s = session(&t.0);
    let r = status(&mut s, "/Volumes/D/Wedding");
    assert_eq!((r["inPipeline"].clone(), r["tag"].clone(), r["stage"].clone()), (json!(true), json!("pipeline.editing"), json!("edit")), "{r}");
    assert_eq!(r["shoot"]["name"], "Wedding");
    // a camera folder inside it, one tied to its backup elsewhere, and a folder deeper in
    for f in ["/Volumes/D/Wedding/raw/M262", "/Volumes/Backup/Wedding/GR3", "/Volumes/D/Wedding/raw/M262/extra/"] {
        assert_eq!(status(&mut s, f)["shoot"]["id"], "s1", "{f}");
    }
    assert_eq!(status(&mut s, "/Volumes/D/Portraits")["tag"], "pipeline.delivering");
    // not marked: the disk the shoots are on, an archived shoot, a folder beside them
    for f in ["/Volumes/D", "/Volumes/D/Old", "/Volumes/D/Wedding2"] {
        let r = status(&mut s, f);
        assert_eq!((r["inPipeline"].clone(), r["tag"].clone()), (json!(false), Value::Null), "{f}: {r}");
    }
    let all = s.execute("pipeline.status", &json!({})).unwrap();
    assert_eq!(all["shoots"].as_array().unwrap().len(), 2, "{all}");

    // a stage change in Second Brain shows after the next read
    std::fs::write(t.0.join("shootpipe.json"), serde_json::to_vec(&sample("finedit")).unwrap()).unwrap();
    assert_eq!(status(&mut s, "/Volumes/D/Wedding")["tag"], "pipeline.exporting");
}

#[test]
fn a_missing_or_bad_file_means_no_marks() {
    let t = tmp("bad");
    let mut s = session(&t.0);
    let r = status(&mut s, "/Volumes/D/Wedding");
    assert_eq!((r["found"].clone(), r["tag"].clone(), r["error"].clone()), (json!(false), Value::Null, Value::Null), "{r}");
    for bad in ["{not json", "null", "{\"shoots\": {}}", ""] {
        std::fs::write(t.0.join("shootpipe.json"), bad).unwrap();
        let r = status(&mut s, "/Volumes/D/Wedding");
        assert_eq!(r["tag"], Value::Null, "{bad}: {r}");
        assert!(r["error"].is_string(), "{bad}: {r}");
    }
}

#[test]
fn pipeline_add_writes_a_request_for_second_brain_and_shows_pending() {
    let t = tmp("add");
    std::fs::write(t.0.join("shootpipe.json"), serde_json::to_vec(&sample("edit")).unwrap()).unwrap();
    let mut s = session(&t.0);
    let r = s.execute("pipeline.add", &json!({"folder": "/Volumes/D/Street/", "backup": "already.backed.up.", "nudge": false})).unwrap();
    assert_eq!((r["queued"].clone(), r["tag"].clone()), (json!(true), json!("pipeline.pending")), "{r}");
    // one request file in the inbox, nothing half-written beside it
    let inbox = t.0.join("lightcraft-inbox");
    let files: Vec<String> = std::fs::read_dir(&inbox).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].ends_with(".json") && !files[0].starts_with('.'));
    let req: Value = serde_json::from_slice(&std::fs::read(inbox.join(&files[0])).unwrap()).unwrap();
    assert_eq!(req["kind"], "pipeline.add");
    assert_eq!(req["path"], "/Volumes/D/Street");
    assert_eq!(req["name"], "Street");
    assert_eq!(req["backup"], "done");
    assert_eq!(req["stage"], "edit");
    assert_eq!(req["from"], "lightcraft");
    // the folder (and a camera folder inside it) shows pending until Second Brain takes it
    let st = status(&mut s, "/Volumes/D/Street/R6");
    assert_eq!((st["pending"].clone(), st["tag"].clone()), (json!(true), json!("pipeline.pending")), "{st}");
    // sent again: the same request, not a second one
    let again = s.execute("pipeline.add", &json!({"folder": "/Volumes/D/Street", "nudge": false})).unwrap();
    assert_eq!(again["request"]["id"], r["request"]["id"]);
    assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 1);
    // Second Brain took it: the file is gone from the inbox, the shoot is on the board
    std::fs::remove_file(inbox.join(&files[0])).unwrap();
    let mut p = sample("edit");
    p["shoots"].as_array_mut().unwrap().push(json!({"id": "s9", "path": "/Volumes/D/Street", "name": "Street", "stage": "edit", "children": []}));
    std::fs::write(t.0.join("shootpipe.json"), serde_json::to_vec(&p).unwrap()).unwrap();
    let st = status(&mut s, "/Volumes/D/Street");
    assert_eq!((st["inPipeline"].clone(), st["tag"].clone()), (json!(true), json!("pipeline.editing")), "{st}");
}

#[test]
fn pipeline_add_refuses_what_is_already_there_or_bad_params() {
    let t = tmp("refuse");
    std::fs::write(t.0.join("shootpipe.json"), serde_json::to_vec(&sample("cull")).unwrap()).unwrap();
    let mut s = session(&t.0);
    let e = s.execute("pipeline.add", &json!({"folder": "/Volumes/D/Wedding/raw/M262", "nudge": false})).unwrap_err().to_string();
    assert!(e.contains("already in the second brain pipeline (culling)"), "{e}");
    assert!(s.execute("pipeline.add", &json!({"nudge": false})).is_err());
    assert!(s.execute("pipeline.add", &json!({"folder": "relative/path", "nudge": false})).is_err());
    assert!(s.execute("pipeline.add", &json!({"folder": "/x", "backup": "sometimes", "nudge": false})).is_err());
    // no Second Brain folder at all: an error, nothing written
    let mut gone = session(&t.0.join("not-installed"));
    assert!(gone.execute("pipeline.add", &json!({"folder": "/Volumes/D/Street", "nudge": false})).is_err());
    assert!(!t.0.join("not-installed").exists());
}
