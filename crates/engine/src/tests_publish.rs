//! LRC-LIB-PUBLISH (Publish Services) and LR-EXP-PHOTOS (export to Apple Photos, with a stand-in
//! for osascript: never the real Photos library).

use serde_json::{Value, json};

use crate::Session;

struct Tmp(std::path::PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(tag: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("lc-publish-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

fn state(s: &mut Session, col: u64, photo: u64) -> String {
    let r = s.execute("publish.status", &json!({"id": col})).unwrap();
    r["photos"].as_array().unwrap().iter().find(|p| p["id"] == photo).map(|p| p["state"].as_str().unwrap().to_string()).unwrap_or_default()
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    v.sort();
    v
}

#[test]
fn hard_drive_publish_new_modified_published_deleted() {
    let dir = tmp("hd");
    let mut s = Session::with_demo();
    s.trash_dir = Some(dir.0.join("trash"));
    std::fs::create_dir_all(dir.0.join("trash")).unwrap();
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    // bad input is an error, never a panic
    assert!(s.execute("publish.createService", &json!({"kind": "flickr"})).is_err());
    assert!(s.execute("publish.createService", &json!({"dir": "relative"})).is_err());
    assert!(s.execute("publish.createService", &json!({"export": {"format": "nope"}})).is_err());
    assert!(s.execute("publish.createCollection", &json!({"service": 99, "name": "x"})).is_err());
    let out = dir.0.join("out");
    let svc = s
        .execute("publish.createService", &json!({"name": "Portfolio", "dir": out.to_string_lossy(), "export": {"format": "jpeg", "longEdge": 64}}))
        .unwrap();
    let sid = svc["id"].as_u64().unwrap();
    assert!(s.execute("publish.createService", &json!({"name": "portfolio", "dir": out.to_string_lossy()})).is_err(), "names are unique");
    let set = s.execute("publish.createCollection", &json!({"service": sid, "name": "Clients", "set": true})).unwrap()["id"].as_u64().unwrap();
    let col =
        s.execute("publish.createCollection", &json!({"service": sid, "name": "Ann & Bo", "parent": set, "ids": [ids[0], ids[1]]})).unwrap()["id"]
            .as_u64()
            .unwrap();
    assert!(s.execute("publish.addPhotos", &json!({"id": set, "ids": [ids[2]]})).is_err(), "a set holds collections, not photos");
    assert_eq!(state(&mut s, col, ids[0]), "new");
    let r = s.execute("publish.services", &json!({})).unwrap();
    assert_eq!(r[0]["collections"].as_array().unwrap().len(), 2);

    // the grid shows it grouped by state
    s.execute("library.source", &json!({"kind": "published", "id": col})).unwrap();
    assert_eq!(s.visible_cloned().iter().map(|p| p.0).collect::<Vec<_>>(), vec![ids[0], ids[1]]);

    // publish: both written under <dir>/Clients/Ann & Bo
    let r = s.execute("publish.run", &json!({"id": col})).unwrap();
    assert_eq!(r["published"], 2, "{r}");
    let folder = out.join("Clients").join("Ann & Bo");
    assert_eq!(files_in(&folder).len(), 2, "{:?}", files_in(&folder));
    assert_eq!(state(&mut s, col, ids[0]), "published");
    assert_eq!(r["counts"]["published"], 2);

    // an edit makes it Modified; the next publish replaces its copy in place
    s.execute("library.select", &json!({"ids": [ids[0]]})).unwrap();
    s.execute("develop.set", &json!({"ids": [ids[0]], "values": {"light.exposure": 0.7}})).unwrap();
    assert_eq!(state(&mut s, col, ids[0]), "modified");
    assert_eq!(state(&mut s, col, ids[1]), "published");
    s.execute("library.source", &json!({"kind": "published", "id": col})).unwrap();
    assert_eq!(s.visible_cloned()[0].0, ids[0], "modified photos come before published ones");
    let r = s.execute("publish.run", &json!({"id": col})).unwrap();
    assert_eq!(r["published"], 1, "only the modified photo: {r}");
    assert_eq!(files_in(&folder).len(), 2, "replaced, not duplicated: {:?}", files_in(&folder));

    // taken out: Deleted until the next publish removes its copy (to the Trash)
    s.execute("publish.removePhotos", &json!({"id": col, "ids": [ids[1]]})).unwrap();
    assert_eq!(state(&mut s, col, ids[1]), "deleted");
    let r = s.execute("publish.run", &json!({"id": col})).unwrap();
    assert_eq!((r["published"].as_u64(), r["removed"].as_u64()), (Some(0), Some(1)), "{r}");
    assert_eq!(files_in(&folder).len(), 1);
    assert_eq!(files_in(&dir.0.join("trash")).len(), 1, "the copy went to the Trash");
    assert_eq!(state(&mut s, col, ids[1]), "");

    // Mark as Up-to-Date: an edit counted as published without publishing
    s.execute("publish.addPhotos", &json!({"id": col, "ids": [ids[2]]})).unwrap();
    assert_eq!(state(&mut s, col, ids[2]), "new");
    s.execute("publish.markUpToDate", &json!({"id": col, "ids": [ids[2]]})).unwrap();
    assert_eq!(state(&mut s, col, ids[2]), "published");

    // undo takes back the last structural change
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(state(&mut s, col, ids[2]), "new");

    // deleting the set takes the collection with it; the published files stay
    s.execute("publish.deleteCollection", &json!({"id": set})).unwrap();
    assert!(s.execute("publish.status", &json!({"id": col})).is_err());
    assert_eq!(files_in(&folder).len(), 1);
    s.execute("publish.deleteService", &json!({"id": sid})).unwrap();
    assert_eq!(s.execute("publish.services", &json!({})).unwrap(), json!([]));
}

#[test]
fn publish_without_a_folder_is_an_error() {
    let mut s = Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    let sid = s.execute("publish.createService", &json!({})).unwrap()["id"].as_u64().unwrap();
    let col = s.execute("publish.createCollection", &json!({"service": sid, "name": "A", "ids": [id]})).unwrap()["id"].as_u64().unwrap();
    let e = s.execute("publish.run", &json!({"id": col})).unwrap_err().to_string();
    assert!(e.contains("folder"), "{e}");
    assert_eq!(state(&mut s, col, id), "new", "nothing recorded");
}

#[test]
fn photos_export_hands_the_files_to_photos_and_cleans_up() {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    let seen = std::sync::Mutex::new(Vec::<String>::new());
    let runner = |args: &[String]| -> Result<String, String> {
        // the files exist while Photos imports them
        assert!(args[4..].iter().all(|f| std::path::Path::new(f).is_file()), "{args:?}");
        *seen.lock().unwrap() = args.to_vec();
        Ok((args.len() - 4).to_string())
    };
    let r: Value = crate::apple_photos::export_to_photos(&mut s, &json!({"ids": ids, "album": "From LightCraft", "longEdge": 64}), &runner).unwrap();
    assert_eq!((r["imported"].as_u64(), r["album"].as_str()), (Some(2), Some("From LightCraft")), "{r}");
    let args = seen.lock().unwrap().clone();
    assert_eq!(args[3], "From LightCraft");
    assert!(args[4].ends_with(".jpg"), "{args:?}");
    // the staging folder is gone afterwards
    assert!(!std::path::Path::new(&args[4]).exists());
    // Photos refusing is an error
    let no = |_: &[String]| Err::<String, String>("LightCraft may not control Photos".into());
    assert!(crate::apple_photos::export_to_photos(&mut s, &json!({"ids": ids}), &no).is_err());
    assert!(crate::apple_photos::export_to_photos(&mut s, &json!({"ids": ids, "format": "dng"}), &runner).is_err(), "rendered files only");
}
