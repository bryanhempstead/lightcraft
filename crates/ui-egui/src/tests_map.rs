//! LR-MAP-MODULE, headless and offline (no tile server): the map. module shows pins for photos
//! with GPS and the OpenStreetMap attribution, pins pick photos (click / ⌘-click, all. / none.),
//! a photo dragged from the filmstrip onto the map is geotagged, and the Saved Locations panel
//! filters to a location. With `LIGHTCRAFT_TEST_SHOTS=<dir>` it writes real-size screenshots.

use serde_json::json;

use crate::headless::Headless;
use crate::tests_library2::{T, click, headless, settle, shot};

fn rect(h: &mut Headless, id: &str) -> Option<(f32, f32, f32, f32)> {
    let r = h.request("ui.widgets", json!({}), T);
    let w = r["result"].as_array()?.iter().find(|w| w["id"] == id)?.clone();
    let v = |i: usize| w["rect"][i].as_f64().unwrap_or(0.0) as f32;
    Some((v(0), v(1), v(2), v(3)))
}

fn centre(h: &mut Headless, id: &str) -> (f32, f32) {
    let (x, y, w, hh) = rect(h, id).unwrap_or_else(|| panic!("{id} not on screen"));
    (x + w / 2.0, y + hh / 2.0)
}

#[test]
fn map_module_pins_geotag_and_locations() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    // two photos in Lisbon, one in Tokyo; the rest without a position
    s.execute("photo.setMeta", &json!({"ids": [ids[0], ids[1]], "gps": [38.7223, -9.1393]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[2]], "gps": [35.6762, 139.6503]})).unwrap();
    for i in &ids[3..] {
        s.execute("photo.setMeta", &json!({"ids": [i], "gps": null})).unwrap();
    }
    let mut h = headless(s);
    h.app.map.offline = true;
    click(&mut h, "module:map");
    assert_eq!(h.app.ui.view, crate::state::ViewMode::Map);
    settle(&mut h);
    assert!(rect(&mut h, "map:canvas").is_some());
    assert!(rect(&mut h, "map:attribution").is_some(), "the OpenStreetMap attribution is always shown");
    assert_eq!(h.app.map.status()["online"], false);
    // Lisbon's two photos are one pin (grouped), Tokyo's another
    assert!(rect(&mut h, &format!("map:pin-{}", ids[0])).is_some(), "Lisbon pin");
    assert!(rect(&mut h, &format!("map:pin-{}", ids[2])).is_some(), "Tokyo pin");
    assert!(rect(&mut h, &format!("map:pin-{}", ids[1])).is_none(), "grouped with its neighbour");
    shot(&mut h, "map-module.png");
    // click a pin: just its photos; ⌘-click another: added
    click(&mut h, &format!("map:pin-{}", ids[0]));
    let sel = |h: &Headless| {
        let mut v: Vec<u64> = h.app.session.selection.ids.iter().map(|p| p.0).collect();
        v.sort();
        v
    };
    assert_eq!(sel(&h), vec![ids[0], ids[1]]);
    let r = h.request("ui.clickWidget", json!({"id": format!("map:pin-{}", ids[2]), "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert_eq!(sel(&h), vec![ids[0], ids[1], ids[2]]);
    click(&mut h, "button:mapNone");
    assert!(h.app.session.selection.ids.len() <= 1, "none. (the active photo stays active, as Deselect All does)");
    click(&mut h, "button:mapAll");
    assert_eq!(sel(&h).len(), 3);
    // drag a photo without a position from the filmstrip onto the map: it is placed there
    // (one the filmstrip shows on screen)
    let target = ids[3..]
        .iter()
        .copied()
        .find(|i| rect(&mut h, &format!("film:{i}")).is_some_and(|r| r.0 > 0.0 && r.0 + r.2 < 1300.0))
        .expect("a filmstrip cell on screen");
    let (fx, fy) = centre(&mut h, &format!("film:{target}"));
    let (mx, my) = centre(&mut h, "map:canvas");
    let r = h.request("ui.drag", json!({"x": fx, "y": fy, "toX": mx, "toY": my, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    let g = h.app.session.catalog.photo(lightcraft_catalog::PhotoId(target)).and_then(|p| p.meta.gps);
    assert!(g.is_some_and(|(la, lo)| la.is_finite() && lo.is_finite()), "geotagged by the drop: {g:?}");
    assert!(h.app.ui.dragging_photos.is_none(), "the drag ended");
    // undo takes it back
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.catalog.photo(lightcraft_catalog::PhotoId(target)).and_then(|p| p.meta.gps), None);
    // a saved location around Lisbon, listed on the left; show. filters to its photos
    let loc = h.app.run("map.saveLocation", json!({"name": "Lisbon", "lat": 38.7223, "lon": -9.1393, "radius": 500})).unwrap();
    let lid = loc["id"].as_u64().unwrap();
    settle(&mut h);
    assert!(rect(&mut h, &format!("mapLocation:{lid}")).is_some(), "listed in Saved Locations");
    shot(&mut h, "map-saved-location.png");
    click(&mut h, &format!("button:mapLocShow-{lid}"));
    let mut vis: Vec<u64> = h.app.session.visible_cloned().iter().map(|p| p.0).collect();
    vis.sort();
    assert_eq!(vis, vec![ids[0], ids[1]]);
    click(&mut h, "button:mapLocClear");
    assert!(h.app.session.visible_cloned().len() > 2, "clr. shows everything again");
    // library. leaves the map for the grid
    click(&mut h, "module:library");
    assert_ne!(h.app.ui.view, crate::state::ViewMode::Map);
}

/// Live (needs the network): the map with real OpenStreetMap tiles, for a screenshot
/// (`LIGHTCRAFT_TEST_SHOTS=… cargo test -p lightcraft-ui-egui map_module_live_tiles -- --ignored`).
#[test]
#[ignore]
fn map_module_live_tiles() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    s.execute("photo.setMeta", &json!({"ids": [ids[0], ids[1]], "gps": [38.7223, -9.1393]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[2]], "gps": [38.6916, -9.2160]})).unwrap();
    let mut h = headless(s);
    h.app.run("view.map", json!({"lat": 38.71, "lon": -9.17, "zoom": 12})).unwrap();
    h.app.run("map.saveLocation", json!({"name": "Baixa", "lat": 38.7223, "lon": -9.1393, "radius": 900})).unwrap();
    let t0 = std::time::Instant::now();
    loop {
        settle(&mut h);
        let st = h.app.map.status();
        if (st["pending"] == 0 && st["inMemory"].as_u64().unwrap_or(0) > 4) || t0.elapsed() > std::time::Duration::from_secs(30) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    settle(&mut h);
    assert!(h.app.map.status()["inMemory"].as_u64().unwrap_or(0) > 0, "{}", h.app.map.status());
    shot(&mut h, "map-live.png");
}

/// LRC-MAP-FILTER: the bar above the map narrows the filmstrip (visible. follows the view as it
/// moves), leaves the pins alone, and belongs to the Map module (the Library grid shows all).
#[test]
fn map_location_filter_bar() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    s.execute("photo.setMeta", &json!({"ids": [ids[0], ids[1]], "gps": [38.7223, -9.1393]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[2]], "gps": [35.6762, 139.6503]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": &ids[3..], "gps": null})).unwrap();
    let mut h = headless(s);
    h.app.map.offline = true;
    click(&mut h, "module:map");
    let n = |h: &mut Headless| h.app.session.visible_cloned().len();
    for b in ["visible", "tagged", "untagged", "saved", "none"] {
        assert!(rect(&mut h, &format!("button:mapFilter-{b}")).is_some(), "{b}. in the bar");
    }
    // the bar sits on the right of the button row, clear of the left group
    let (fx, ..) = rect(&mut h, "button:mapFilter-visible").unwrap();
    let (nx, _, nw, _) = rect(&mut h, "button:mapNone").unwrap();
    assert!(fx > nx + nw + 8.0, "visible. starts after none.");
    click(&mut h, "button:mapFilter-tagged");
    assert_eq!(n(&mut h), 3);
    assert!(rect(&mut h, &format!("map:pin-{}", ids[2])).is_some(), "pins stay");
    shot(&mut h, "map-filter-tagged.png");
    click(&mut h, "button:mapFilter-untagged");
    assert_eq!(n(&mut h), ids.len() - 3);
    // Visible on Map: the whole world, then zoomed on Lisbon (Tokyo leaves the filmstrip)
    h.app.run("view.map", json!({"lat": 20.0, "lon": 60.0, "zoom": 2})).unwrap();
    settle(&mut h);
    click(&mut h, "button:mapFilter-visible");
    assert_eq!(n(&mut h), 3);
    h.app.run("view.map", json!({"lat": 38.72, "lon": -9.14, "zoom": 11})).unwrap();
    settle(&mut h);
    let mut v: Vec<u64> = h.app.session.visible_cloned().iter().map(|p| p.0).collect();
    v.sort();
    assert_eq!(v, vec![ids[0], ids[1]]);
    shot(&mut h, "map-filter-visible.png");
    // the Library grid: no location filter there; back on the map it is on again
    click(&mut h, "module:library");
    assert_eq!(n(&mut h), ids.len());
    click(&mut h, "module:map");
    assert_eq!(n(&mut h), 2);
    click(&mut h, "button:mapFilter-none");
    assert_eq!(n(&mut h), ids.len());
}
