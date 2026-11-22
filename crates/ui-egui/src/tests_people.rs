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
    // a PNG encoder for the screenshots `LIGHTCRAFT_TEST_SHOTS` asks for
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = Services { png: Some(Box::new(png)), write: Some(Box::new(lightcraft_engine::export::write_file)), ..Default::default() };
    let mut session = lightcraft_engine::Session::with_demo();
    let dir = std::env::temp_dir().join(format!("lc-ui-people-{}-{:?}", std::process::id(), std::thread::current().id()));
    let _ = std::fs::remove_dir_all(&dir);
    session.vision.dir = Some(dir.join("index"));
    session.vision.root = dir.join("no-models");
    let ids: Vec<PhotoId> = session.catalog.photos().map(|p| p.id).take(4).collect();
    let index = session.faces_index().unwrap();
    for (i, id) in ids.iter().enumerate() {
        let key = lightcraft_engine::media::content_key(session.catalog.photo(*id).unwrap());
        let face = FaceEntry {
            rect: [0.3, 0.2, 0.2, 0.3],
            landmarks: [[0.0; 2]; 5],
            score: 0.9,
            embedding: unit(1 + i / 2),
            eyes: None,
            sharpness: 50.0,
            expression: None,
        };
        index.lock().unwrap().put(&key, FaceRecord { faces: vec![face], width: 100, height: 100, expressions: false }).unwrap();
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

fn pointer(h: &mut Headless, events: serde_json::Value) {
    let r = h.request("ui.pointer", json!({"events": events}), T);
    assert_eq!(r["ok"], true, "{r}");
}

/// LR-LIB-PEOPLE: the Draw Face Region tool draws a face box with the mouse (a hand-drawn face, the name field open
/// at once), moves it by dragging, renames it from its label; Esc leaves the tool.
#[test]
fn drawing_moving_and_renaming_a_face_in_the_loupe() {
    let (mut h, ids) = app();
    exec(&mut h, "library.select", json!({"ids": [ids[3].0], "active": ids[3].0}));
    exec(&mut h, "view.detail", json!({}));
    exec(&mut h, "view.drawFace", json!({}));
    assert!(h.app.ui.draw_face && h.app.ui.face_boxes);
    let regions = |h: &Headless| h.app.session.catalog.photo(ids[3]).unwrap().meta.regions.clone();
    assert_eq!(regions(&h).len(), 1);
    // draw: a drag on the photo, away from the box that is there
    pointer(
        &mut h,
        json!([{"kind": "down", "x": 0.6, "y": 0.2}, {"kind": "drag", "x": 0.7, "y": 0.3}, {"kind": "drag", "x": 0.8, "y": 0.45}, {"kind": "up", "x": 0.8, "y": 0.45}]),
    );
    let r = regions(&h);
    assert_eq!(r.len(), 2, "{r:?}");
    assert!(r[1].drawn && r[1].name.is_none());
    assert!((r[1].rect.x0 - 0.6).abs() < 0.02 && (r[1].rect.y1 - 0.45).abs() < 0.02, "{:?}", r[1].rect);
    assert_eq!(h.app.ui.face_name_edit, Some((1, String::new())), "the name field opens at once");
    crate::tests_library2::shot(&mut h, "faces-draw.png");
    h.app.ui.face_name_edit = None;
    // move it: drag the box itself; one undo step puts it back
    let w0 = r[1].rect.x1 - r[1].rect.x0;
    pointer(
        &mut h,
        json!([{"kind": "down", "x": 0.7, "y": 0.33}, {"kind": "drag", "x": 0.68, "y": 0.33}, {"kind": "drag", "x": 0.65, "y": 0.35}, {"kind": "up", "x": 0.65, "y": 0.35}]),
    );
    let r = regions(&h);
    assert_eq!(r.len(), 2, "moving doesn't draw another box");
    assert!((r[1].rect.x0 - 0.55).abs() < 0.02 && ((r[1].rect.x1 - r[1].rect.x0) - w0).abs() < 1e-3, "{:?}", r[1].rect);
    exec(&mut h, "edit.undo", json!({}));
    assert!((regions(&h)[1].rect.x0 - 0.6).abs() < 0.02);
    // name it, then a click on its label opens the field with the name, to rename it
    exec(&mut h, "people.name", json!({"faces": [{"photo": ids[3].0, "index": 1}], "name": "Dee"}));
    let w = widgets(&mut h);
    assert!(w.iter().any(|x| x == "faceLabel:1"), "{w:?}");
    let r = h.request("ui.clickWidget", json!({"id": "faceLabel:1"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.face_name_edit, Some((1, "Dee".to_string())));
    h.app.ui.face_name_edit = None;
    h.step();
    // Esc leaves the tool
    let r = h.request("ui.key", json!({"key": "escape"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert!(!h.app.ui.draw_face);
}

/// LRC-LIB-PEOPLE: picked stacks are named, confirmed or rejected together; a stack's count opens it.
#[test]
fn picked_stacks_in_bulk_and_an_opened_stack() {
    let (mut h, ids) = app();
    exec(&mut h, "people.name", json!({"faces": [{"photo": ids[0].0, "index": 0}], "name": "Ann"}));
    exec(&mut h, "view.people", json!({}));
    let w = widgets(&mut h);
    let stacks: Vec<String> = w.iter().filter(|x| x.starts_with("stack:")).cloned().collect();
    assert_eq!(stacks.len(), 2, "{w:?}");
    // pick both: the bulk row offers a name, yes. (one is suggested as Ann), no. and rm.
    h.request("ui.clickWidget", json!({"id": "button:people:Unnamed People:all"}), T);
    h.step();
    let w = widgets(&mut h);
    for b in ["field:peopleBulkName", "button:peopleBulkName", "button:peopleBulkYes", "button:peopleBulkNo", "button:peopleBulkRemove"] {
        assert!(w.iter().any(|x| x == b), "{b}: {w:?}");
    }
    crate::tests_library2::shot(&mut h, "people-bulk.png");
    // yes.: the suggested face becomes Ann; the other stack stays unnamed
    let r = h.request("ui.clickWidget", json!({"id": "button:peopleBulkYes"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.session.catalog.photo(ids[1]).unwrap().meta.regions[0].name.as_deref(), Some("Ann"));
    assert!(h.app.session.catalog.photo(ids[2]).unwrap().meta.regions[0].name.is_none());
    // name the rest at once from the bulk row
    h.request("ui.clickWidget", json!({"id": "button:people:Unnamed People:all"}), T);
    h.step();
    h.app.ui.people_bulk_name = "Bo".into();
    let r = h.request("ui.clickWidget", json!({"id": "button:peopleBulkName"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!([2, 3].iter().all(|&i| h.app.session.catalog.photo(ids[i]).unwrap().meta.regions[0].name.as_deref() == Some("Bo")));
    // undo, then open the stack by its count: its faces one by one
    exec(&mut h, "edit.undo", json!({}));
    h.step();
    let w = widgets(&mut h);
    let count = w.iter().find(|x| x.starts_with("stackCount:")).cloned().expect("a stack of two has a count");
    let r = h.request("ui.clickWidget", json!({"id": count}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.app.ui.people_stack.is_some());
    let w = widgets(&mut h);
    assert_eq!(w.iter().filter(|x| x.starts_with("face:")).count(), 2, "{w:?}");
    assert!(w.iter().any(|x| x == "button:stackBack"));
    crate::tests_library2::shot(&mut h, "people-stack.png");
}
