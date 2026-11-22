//! LRC-LIB-PUBLISH in the app: the Publish Services panel lists a service and its collections,
//! a collection shows its photos grouped by state, `pub.` publishes in the background.

use serde_json::json;

use crate::tests_library2::{T, click, headless, settle, shot, temp};

#[test]
fn publish_services_panel_publishes_in_the_background() {
    let dir = temp("publish");
    let out = dir.0.join("out");
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    let sid = s
        .execute("publish.createService", &json!({"name": "Portfolio", "dir": out.to_string_lossy(), "export": {"format": "jpeg", "longEdge": 64}}))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let set = s.execute("publish.createCollection", &json!({"service": sid, "name": "Clients", "set": true})).unwrap()["id"].as_u64().unwrap();
    let col = s.execute("publish.createCollection", &json!({"service": sid, "name": "Ann", "parent": set, "ids": [ids[0], ids[1]]})).unwrap()["id"]
        .as_u64()
        .unwrap();
    let mut h = headless(s);
    // the other sections folded, so Publish Services is in view
    h.app.ui.collapsed_sidebar = ["navigator", "catalog", "shoots", "folders"].map(String::from).to_vec();
    h.app.services.write_shared = Some(std::sync::Arc::new(|p: &str, b: &[u8]| lightcraft_engine::export::write_file(p, b)));
    settle(&mut h);
    // the panel lists the service, the set and the collection with its count of photos to publish
    let r = h.request("ui.widgets", json!({}), T);
    let has = |id: &str| r["result"].as_array().unwrap().iter().any(|w| w["id"] == id);
    assert!(has(&format!("source:publishService:{sid}")), "service row");
    assert!(has(&format!("source:publishSet:{set}")), "set row");
    assert!(has(&format!("source:published:{col}")), "collection row");
    click(&mut h, &format!("source:published:{col}"));
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Published(col));
    h.app.ui.view = crate::state::ViewMode::PhotoGrid;
    settle(&mut h);
    let r = h.request("ui.widgets", json!({}), T);
    assert!(r["result"].as_array().unwrap().iter().any(|w| w["id"] == "group:publish:new"), "grouped by state");
    shot(&mut h, "publish-panel-new.png");
    // pub.: publishes on a worker, the panel keeps drawing, the result lands on the UI thread
    click(&mut h, "button:publishNow");
    assert!(
        h.app.tasks.is_running(crate::publish_ui::PUBLISHING)
            || h.app.session.execute("publish.status", &json!({"id": col})).unwrap()["counts"]["published"] == 2
    );
    let ctx = h.view.ctx.clone();
    assert!(crate::tasks::wait(&mut h.app, &ctx, std::time::Duration::from_secs(60)));
    settle(&mut h);
    let st = h.app.session.execute("publish.status", &json!({"id": col})).unwrap();
    assert_eq!(st["counts"]["published"], 2, "{st}");
    assert_eq!(std::fs::read_dir(out.join("Clients").join("Ann")).unwrap().count(), 2);
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Published “Ann”: 2 photos"), "{toast}");
    // drag-free add: the collection's menu adds the selection; it shows as New
    h.app.session.execute("library.select", &json!({"ids": [ids[2]]})).unwrap();
    h.app.run("publish.addPhotos", json!({"id": col})).unwrap();
    settle(&mut h);
    let r = h.request("ui.widgets", json!({}), T);
    assert!(r["result"].as_array().unwrap().iter().any(|w| w["id"] == "group:publish:published"), "published group");
    shot(&mut h, "publish-panel-mixed.png");
}
