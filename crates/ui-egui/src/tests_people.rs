//! Headless tests of the People view: unnamed stacks with name fields, a named card and its page, picking, and the
//! loupe's `?` on unnamed face boxes. Synthetic embeddings stand in for the face models.

use std::time::Duration;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_vision::index::{FaceEntry, FaceRecord};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn unit(k: usize) -> Vec<f32> {
    let mut v = vec![0f32; 128];
    v[k] = 1.0;
    v
}

/// A demo app whose first four photos each hold one indexed face: two of person 1, two of person 2.
fn app() -> (Headless, Vec<PhotoId>) {
    let services = Services { png: None, ..Default::default() };
    let mut session = lightcraft_engine::Session::with_demo();
    let dir = std::env::temp_dir().join(format!("lc-ui-people-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    session.vision.dir = Some(dir.join("index"));
    session.vision.root = dir.join("no-models");
    let ids: Vec<PhotoId> = session.catalog.photos().map(|p| p.id).take(4).collect();
    let index = session.faces_index().unwrap();
    for (i, id) in ids.iter().enumerate() {
        let key = lightcraft_engine::media::content_key(session.catalog.photo(*id).unwrap());
        let face =
            FaceEntry { rect: [0.3, 0.2, 0.2, 0.3], landmarks: [[0.0; 2]; 5], score: 0.9, embedding: unit(1 + i / 2), eyes: None, sharpness: 50.0 };
        index.lock().unwrap().put(&key, FaceRecord { faces: vec![face], width: 100, height: 100 }).unwrap();
        let region = lightcraft_meta::Region::face(lightcraft_meta::Rect { x0: 0.3, y0: 0.2, x1: 0.5, y1: 0.5 });
        session.commit("test", Op::SetRegions { id: *id, regions: vec![region] }).unwrap();
    }
    session.vision.generation += 1;
    let app = LightcraftApp::new(session, services);
    (Headless::new(app, [1400.0, 1000.0], 1.0), ids)
}

fn exec(h: &mut Headless, command: &str, params: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

fn widgets(h: &mut Headless) -> Vec<String> {
    let r = h.request("ui.widgets", json!({}), T);
    r["result"].as_array().map(|a| a.iter().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

#[test]
fn stacks_name_fields_picking_and_a_persons_page() {
    let (mut h, ids) = app();
    exec(&mut h, "view.people", json!({}));
    let w = widgets(&mut h);
    let stacks: Vec<&String> = w.iter().filter(|x| x.starts_with("stack:")).collect();
    assert_eq!(stacks.len(), 2, "two people, two stacks: {w:?}");
    assert!(w.iter().any(|x| x.starts_with("stackName:")), "a name field under each stack");
    assert!(w.iter().any(|x| x == "button:peopleGet"), "no models: get. is offered");
    // pick both stacks: click, then ⌘-click
    let first = stacks[0].clone();
    let second = stacks[1].clone();
    let r = h.request("ui.clickWidget", json!({"id": first}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("ui.clickWidget", json!({"id": second, "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.people_sel.len(), 2, "⌘-click adds");
    let r = h.request("ui.clickWidget", json!({"id": "button:people:Unnamed People:none"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.app.ui.people_sel.is_empty(), "none. clears");
    // name one face: a named card appears, the other stack suggests the name
    exec(&mut h, "people.name", json!({"faces": [{"photo": ids[0].0, "index": 0}], "name": "Ann"}));
    let w = widgets(&mut h);
    assert!(w.iter().any(|x| x == "person:Ann"), "{w:?}");
    assert!(w.iter().any(|x| x.starts_with("button:stack-ok:")), "the suggestion's ✓: {w:?}");
    // the person's page: Similar with ✓ / ✗, Confirmed
    let r = h.request("ui.clickWidget", json!({"id": "person:Ann"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.people_page.as_deref(), Some("Ann"));
    let w = widgets(&mut h);
    assert!(w.iter().any(|x| x.starts_with("button:face-ok:")) && w.iter().any(|x| x == "button:peopleBack"), "{w:?}");
    let ok = w.iter().find(|x| x.starts_with("button:face-ok:")).cloned().unwrap();
    let r = h.request("ui.clickWidget", json!({"id": ok}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.session.catalog.photo(ids[1]).unwrap().meta.regions[0].name.as_deref(), Some("Ann"), "✓ confirms");
    // the loupe: an unnamed face box has its ? field
    exec(&mut h, "library.select", json!({"ids": [ids[2].0], "active": ids[2].0}));
    exec(&mut h, "view.detail", json!({}));
    let w = widgets(&mut h);
    assert!(w.iter().any(|x| x == "faceNameButton:0"), "{w:?}");
}
