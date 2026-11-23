//! LC-GALLERIES: client galleries owned by Second Brain, driven from LightCraft — with a stand-in
//! for Second Brain's gallery API ([`crate::galleries::fake`]): never the real app or its files.

use serde_json::{Value, json};

use crate::Session;
use crate::galleries::fake::FakeSecondBrain;

struct Tmp(std::path::PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(tag: &str) -> Tmp {
    let d = std::env::temp_dir().join(format!("lc-galleries-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Tmp(d)
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default();
    v.sort();
    v
}

fn session(sb: &std::path::Path) -> Session {
    let mut s = Session::with_demo();
    s.second_brain_dir = Some(sb.to_path_buf());
    s
}

#[test]
fn create_from_a_collection_exports_with_edits_and_registers_the_gallery() {
    let t = tmp("create");
    let sb = FakeSecondBrain::start(&t.0.join("sb")).unwrap();
    let mut s = session(&sb.dir);
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    let album = s.execute("album.create", &json!({"name": "Erika & Connor"})).unwrap()["id"].as_u64().unwrap();
    s.execute("album.addPhotos", &json!({"id": album, "ids": ids})).unwrap();
    s.execute("library.source", &json!({"kind": "album", "id": album})).unwrap();
    // an edit that has to reach the client's copy
    s.execute("develop.set", &json!({"ids": [ids[0]], "values": {"light.exposure": 1.0}})).unwrap();
    let folder = t.0.join("shoot").join("gallery").join("Erika & Connor");
    let r = s
        .execute(
            "gallery.create",
            &json!({"source": "view", "folder": folder.to_string_lossy(), "clientName": "Erika", "password": "pw", "export": {"longEdge": 96}}),
        )
        .unwrap();
    assert_eq!(r["exported"], 3, "{r}");
    assert_eq!(r["registered"], true, "{r}");
    let gid = r["gallery"].as_str().unwrap().to_string();
    // the files are there, as web JPEGs sized as asked
    let files = files_in(&folder);
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(files.iter().all(|f| f.ends_with(".jpg")));
    let bytes = std::fs::read(folder.join(&files[0])).unwrap();
    let d = lightcraft_codecs::decode(&bytes, Default::default()).unwrap();
    assert!(d.width.max(d.height) <= 96, "{}x{}", d.width, d.height);
    // Second Brain was asked to register it, with the collection's name and the settings
    let reqs = sb.lock().requests.clone();
    let made = reqs.iter().find(|(m, p, _)| m == "POST" && p == "/v1/galleries").expect("registered");
    assert_eq!(made.2["name"], "Erika & Connor");
    assert_eq!(made.2["clientName"], "Erika");
    assert_eq!(made.2["password"], "pw");
    assert_eq!(made.2["folder"], folder.to_string_lossy().as_ref());
    // the link: every file maps back to its library photo
    let l = crate::galleries::link(&s.catalog, &gid).unwrap();
    let mut back: Vec<u64> = l.files.values().copied().collect();
    back.sort();
    let mut want = ids.clone();
    want.sort();
    assert_eq!(back, want);
    // the list says it was made here
    let list = s.execute("gallery.list", &json!({})).unwrap();
    assert_eq!(list["online"], true);
    let row = list["galleries"].as_array().unwrap().iter().find(|g| g["id"] == gid.as_str()).cloned().unwrap();
    assert_eq!(row["linked"], true);
    assert_eq!(row["count"], 3);
    assert!(row.get("token").is_none(), "no link key");
    // each gallery photo knows its source photo; jump to it
    let g = s.execute("gallery.get", &json!({"id": gid})).unwrap();
    let first = &g["photos"][0];
    assert!(first["photo"].is_u64(), "{g}");
    let src = s.execute("gallery.sourcePhoto", &json!({"id": gid, "name": first["name"]})).unwrap();
    assert_eq!(src["photo"], first["photo"]);
    assert_eq!(s.selection.active.map(|p| p.0), first["photo"].as_u64());
}

#[test]
fn reexport_replaces_each_edited_photo_in_place() {
    let t = tmp("reexport");
    let sb = FakeSecondBrain::start(&t.0.join("sb")).unwrap();
    let mut s = session(&sb.dir);
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    let folder = t.0.join("g");
    let r = s.execute("gallery.create", &json!({"ids": ids, "name": "Two", "folder": folder.to_string_lossy(), "export": {"longEdge": 64}})).unwrap();
    let gid = r["gallery"].as_str().unwrap().to_string();
    let before = files_in(&folder);
    // nothing changed: nothing to do
    assert_eq!(s.execute("gallery.reexport", &json!({"id": gid})).unwrap()["exported"], 0);
    s.execute("develop.set", &json!({"ids": [ids[1]], "values": {"light.exposure": -1.0}})).unwrap();
    let r = s.execute("gallery.reexport", &json!({"id": gid})).unwrap();
    assert_eq!(r["exported"], 1, "{r}");
    assert_eq!(files_in(&folder), before, "same files, replaced in place (no -2 copies)");
    // Second Brain was told the folder changed
    assert!(sb.lock().requests.iter().any(|(m, p, _)| m == "POST" && p == &format!("/v1/galleries/{gid}/folder")));
}

#[test]
fn client_picks_flow_back_as_flags_and_a_collection() {
    let t = tmp("picks");
    let sb = FakeSecondBrain::start(&t.0.join("sb")).unwrap();
    let mut s = session(&sb.dir);
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    let folder = t.0.join("g");
    let r = s.execute("gallery.create", &json!({"ids": ids, "name": "Ann", "folder": folder.to_string_lossy(), "export": {"longEdge": 48}})).unwrap();
    let gid = r["gallery"].as_str().unwrap().to_string();
    let l = crate::galleries::link(&s.catalog, &gid).unwrap();
    let file_of = |id: u64| l.file_of(id).unwrap().to_string();
    let (a, c) = (file_of(ids[0]), file_of(ids[2]));
    sb.set_selected(&gid, &[a.as_str(), c.as_str(), "not-there.jpg"]);
    let p = s.execute("gallery.picks", &json!({"id": gid})).unwrap();
    let mut got: Vec<u64> = p["photos"].as_array().unwrap().iter().filter_map(Value::as_u64).collect();
    got.sort();
    assert_eq!(got, vec![ids[0], ids[2]]);
    let r = s.execute("gallery.applyPicks", &json!({"id": gid, "flag": "pick", "rating": 4, "label": "green"})).unwrap();
    assert_eq!(r["photos"].as_array().unwrap().len(), 2);
    for id in [ids[0], ids[2]] {
        let ph = s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap();
        assert_eq!(ph.flag, lightcraft_catalog::Flag::Pick);
        assert_eq!(ph.rating, 4);
        assert!(ph.label.is_some());
    }
    assert_ne!(s.catalog.photo(lightcraft_catalog::PhotoId(ids[1])).unwrap().flag, lightcraft_catalog::Flag::Pick);
    // the "client picks" collection, in a Client Picks set
    let album = r["album"].as_u64().unwrap();
    let al = s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().clone();
    assert_eq!(al.photos.len(), 2);
    assert!(al.name.contains("client picks"));
    let set = s.catalog.album(al.parent.unwrap()).unwrap();
    assert!(set.folder && set.name == crate::galleries::PICKS_SET);
    // the client changes their mind: the collection follows, the same one
    sb.set_selected(&gid, &[a.as_str()]);
    let r2 = s.execute("gallery.applyPicks", &json!({"id": gid})).unwrap();
    assert_eq!(r2["album"].as_u64(), Some(album));
    assert_eq!(s.catalog.album(lightcraft_catalog::AlbumId(album)).unwrap().photos.len(), 1);
}

#[test]
fn offline_reads_galleries_json_and_refuses_changes() {
    let t = tmp("offline");
    let sb = FakeSecondBrain::start(&t.0.join("sb")).unwrap();
    let shoot = t.0.join("their shoot");
    std::fs::create_dir_all(&shoot).unwrap();
    std::fs::write(shoot.join("a.jpg"), b"x").unwrap();
    std::fs::write(shoot.join(".hidden.jpg"), b"x").unwrap();
    let id = sb.add("Made in Second Brain", &shoot.to_string_lossy());
    sb.stop();
    let galleries_json = sb.dir.join("client").join("galleries.json");
    let before = std::fs::read(&galleries_json).unwrap();
    let mut s = session(&sb.dir);
    let st = s.execute("gallery.status", &json!({})).unwrap();
    assert_eq!(st["online"], false);
    let list = s.execute("gallery.list", &json!({})).unwrap();
    assert_eq!(list["readOnly"], true);
    assert!(list["notice"].as_str().unwrap().contains("isn't running"), "{list}");
    let g = &list["galleries"][0];
    assert_eq!(g["id"], id.as_str());
    assert!(g.get("token").is_none() && g.get("password").is_none(), "no key, no password: {g}");
    let one = s.execute("gallery.get", &json!({"id": id})).unwrap();
    assert_eq!(one["photos"].as_array().unwrap().len(), 1, "{one}");
    // every change is an error that says why, and the file is never written
    for (c, p) in [
        ("gallery.update", json!({"id": id, "name": "x"})),
        ("gallery.publish", json!({"id": id})),
        ("gallery.delete", json!({"id": id})),
        ("gallery.create", json!({"ids": [1], "folder": t.0.join("x").to_string_lossy()})),
    ] {
        let e = s.execute(c, &p).unwrap_err().to_string();
        assert!(e.contains("isn't running"), "{c}: {e}");
    }
    assert_eq!(std::fs::read(&galleries_json).unwrap(), before, "galleries.json untouched");
    // a missing or broken file: an empty list with a notice, never a crash
    std::fs::write(&galleries_json, b"{broken").unwrap();
    let l = s.execute("gallery.list", &json!({})).unwrap();
    assert_eq!(l["galleries"].as_array().unwrap().len(), 0);
    assert!(l["notice"].as_str().unwrap().contains("not JSON"), "{l}");
    std::fs::remove_file(&galleries_json).unwrap();
    assert!(s.execute("gallery.list", &json!({})).unwrap()["notice"].as_str().unwrap().contains("can't read"));
}

#[test]
fn api_errors_are_errors_not_crashes() {
    let t = tmp("errors");
    let sb = FakeSecondBrain::start(&t.0.join("sb")).unwrap();
    let mut s = session(&sb.dir);
    assert!(s.execute("gallery.get", &json!({"id": "nope"})).unwrap_err().to_string().contains("no gallery"));
    assert!(s.execute("gallery.update", &json!({"id": "nope", "name": "x"})).is_err());
    assert!(s.execute("gallery.update", &json!({"id": "x"})).unwrap_err().to_string().contains("nothing to change"));
    assert!(s.execute("gallery.applyPicks", &json!({"id": "nope"})).is_err());
    assert!(s.execute("gallery.create", &json!({"ids": [999999]})).is_err());
    // a discovery file with the wrong token: offline, with a notice
    std::fs::write(sb.dir.join(crate::galleries::DISCOVERY_FILE), json!({"port": sb.port, "token": "wrong"}).to_string()).unwrap();
    assert_eq!(s.execute("gallery.status", &json!({})).unwrap()["online"], false);
    // something that isn't HTTP on the port
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for c in l.incoming().take(3).flatten() {
            use std::io::Write;
            let mut c = c;
            let _ = c.write_all(b"garbage\n");
        }
    });
    std::fs::write(sb.dir.join(crate::galleries::DISCOVERY_FILE), json!({"port": port, "token": "abc"}).to_string()).unwrap();
    let st = s.execute("gallery.status", &json!({})).unwrap();
    assert_eq!(st["online"], false, "{st}");
}
