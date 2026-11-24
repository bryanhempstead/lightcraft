//! LR-MAP-MODULE: geotagging (drop on the map) with undo, Saved Locations round trip through the
//! catalog (and a library reopened from disk), and the "photos in this location" filter.

use serde_json::json;

use crate::Session;
use lightcraft_catalog::PhotoId;

fn gps(s: &Session, id: u64) -> Option<(f64, f64)> {
    s.catalog.photo(PhotoId(id)).and_then(|p| p.meta.gps)
}

#[test]
fn geotag_and_undo() {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    let before: Vec<_> = ids.iter().map(|i| gps(&s, *i)).collect();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    let r = s.execute("map.geotag", &json!({"lat": 38.7223, "lon": -9.1393})).unwrap();
    assert_eq!(r["photos"], 2);
    assert!(ids.iter().all(|i| gps(&s, *i) == Some((38.7223, -9.1393))));
    let pins = s.execute("map.pins", &json!({"ids": ids})).unwrap();
    assert_eq!(pins["pins"].as_array().unwrap().len(), 2, "{pins}");
    s.execute("edit.undo", &json!({})).unwrap();
    let after: Vec<_> = ids.iter().map(|i| gps(&s, *i)).collect();
    assert_eq!(before, after, "undo puts the old positions back");
    // nonsense is an error, never a panic or a write
    for bad in [json!({"lat": 91.0, "lon": 0.0}), json!({"lat": "x", "lon": 0.0}), json!({"lat": f64::MAX, "lon": -181.0}), json!({})] {
        assert!(s.execute("map.geotag", &bad).is_err(), "{bad}");
    }
    s.execute("map.geotag", &json!({"ids": [ids[0]], "lat": 1.0, "lon": 2.0})).unwrap();
    s.execute("map.clearGps", &json!({"ids": [ids[0]]})).unwrap();
    assert_eq!(gps(&s, ids[0]), None);
}

#[test]
fn saved_locations_round_trip_and_filter() {
    let dir = std::env::temp_dir().join(format!("lc-map-loc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.open_library(dir.clone(), false).unwrap();
    // three photos: two in Lisbon (~300 m apart), one in Porto
    let mut ids = Vec::new();
    for (i, (lat, lon)) in [(38.7223, -9.1393), (38.7250, -9.1400), (41.1579, -8.6291)].into_iter().enumerate() {
        let id = s.catalog.alloc_photo_id();
        let mut p = lightcraft_catalog::Photo::new(
            id,
            lightcraft_catalog::Source::File { path: format!("/nowhere/m{i}.jpg") },
            "m.jpg",
            "JPEG",
            4,
            3,
            "2026-11-22",
        );
        p.meta.gps = Some((lat, lon));
        s.commit("add", lightcraft_catalog::Op::AddPhoto { photo: Box::new(p) }).unwrap();
        ids.push(id.0);
    }
    let lis = s.execute("map.saveLocation", &json!({"name": "Lisbon", "lat": 38.7223, "lon": -9.1393, "radius": 1000})).unwrap();
    assert_eq!(lis["photos"], 2, "{lis}");
    let por = s.execute("map.saveLocation", &json!({"name": "Porto", "lat": 41.1579, "lon": -8.6291})).unwrap();
    assert_eq!((por["radius"].as_f64(), por["photos"].as_u64()), (Some(100.0), Some(1)));
    assert!(s.execute("map.saveLocation", &json!({"name": "", "lat": 0, "lon": 0})).is_err());
    assert!(s.execute("map.saveLocation", &json!({"name": "x", "lat": 0, "lon": 0, "radius": -5})).is_err());
    assert!(s.execute("map.saveLocation", &json!({"id": 99, "name": "x"})).is_err());
    // edit: rename + private, keeps its place
    let lid = lis["id"].as_u64().unwrap();
    s.execute("map.saveLocation", &json!({"id": lid, "name": "Lisboa", "private": true})).unwrap();
    let list = s.execute("map.locations", &json!({})).unwrap();
    let names: Vec<&str> = list.as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Lisboa", "Porto"]);
    assert_eq!(list[0]["private"], true);
    assert_eq!(list[0]["lat"], 38.7223);
    // a private location's photos are exported without their position; others keep it
    let o = crate::export::ExportOptions::default();
    let gps_out = |s: &Session, id: u64| {
        let p = s.catalog.photo(PhotoId(id)).unwrap();
        crate::export::export_metadata_with(Some(&s.catalog), p, &o).and_then(|m| m.gps)
    };
    assert!(gps_out(&s, ids[0]).is_none() && gps_out(&s, ids[1]).is_none(), "inside the private location");
    assert!(gps_out(&s, ids[2]).is_some(), "Porto isn't private");
    // the filter shows only the photos there
    let r = s.execute("map.photosIn", &json!({"id": lid})).unwrap();
    assert_eq!(r["photos"], json!([ids[0], ids[1]]));
    let vis: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    assert_eq!(vis.len(), 2, "{vis:?}");
    assert!(vis.contains(&ids[0]) && vis.contains(&ids[1]));
    // an empty place shows nothing (not everything)
    s.execute("map.photosIn", &json!({"lat": 0.0, "lon": 0.0, "radius": 10})).unwrap();
    assert!(s.visible_cloned().is_empty());
    // delete + undo
    s.execute("map.deleteLocation", &json!({"id": por["id"]})).unwrap();
    assert_eq!(s.execute("map.locations", &json!({})).unwrap().as_array().unwrap().len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("map.locations", &json!({})).unwrap().as_array().unwrap().len(), 2);
    assert!(s.execute("map.deleteLocation", &json!({"id": 1234})).is_err());
    // saved with the library: reopened, they are still there
    s.close_library().unwrap();
    drop(s);
    let mut s2 = Session::new();
    s2.open_library(dir.clone(), false).unwrap();
    let list = s2.execute("map.locations", &json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    drop(s2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// LRC-MAP-FILTER: the location filter bar narrows the view (not the map's pins), follows the
/// map's bounds (across the antimeridian too), only applies while it is on, and Clear Filters
/// turns it off.
#[test]
fn location_filter_bar() {
    let mut s = Session::with_demo();
    let all: Vec<u64> = s.visible_cloned().iter().map(|i| i.0).collect();
    assert!(all.len() >= 4);
    s.execute("map.clearGps", &json!({"ids": all})).unwrap();
    // Lisbon, Tokyo, Fiji (just east of 180°), the rest untagged
    s.execute("map.geotag", &json!({"ids": [all[0]], "lat": 38.72, "lon": -9.14})).unwrap();
    s.execute("map.geotag", &json!({"ids": [all[1]], "lat": 35.68, "lon": 139.69})).unwrap();
    s.execute("map.geotag", &json!({"ids": [all[2]], "lat": -17.7, "lon": -179.5})).unwrap();
    let shown = |s: &mut Session| -> Vec<u64> {
        let mut v: Vec<u64> = s.visible_cloned().iter().map(|i| i.0).collect();
        v.sort();
        v
    };
    let r = s.execute("map.locationFilter", &json!({"mode": "tagged"})).unwrap();
    assert_eq!((r["photos"].as_u64(), r["of"].as_u64()), (Some(3), Some(all.len() as u64)), "{r}");
    assert_eq!(shown(&mut s).len(), 3);
    // the pins stay the whole view's
    assert_eq!(s.execute("map.pins", &json!({})).unwrap()["pins"].as_array().unwrap().len(), 3);
    s.execute("map.locationFilter", &json!({"mode": "untagged"})).unwrap();
    assert_eq!(shown(&mut s).len(), all.len() - 3);
    // Visible on Map: Europe only, then a view across the antimeridian (130°E … 190°E)
    s.execute("map.locationFilter", &json!({"mode": "visible", "bounds": [30.0, -20.0, 60.0, 30.0]})).unwrap();
    assert_eq!(shown(&mut s), vec![all[0]]);
    s.execute("map.locationFilter", &json!({"mode": "visible", "bounds": [-30.0, 130.0, 40.0, 190.0]})).unwrap();
    let mut want = vec![all[1], all[2]];
    want.sort();
    assert_eq!(shown(&mut s), want);
    // a view wider than the world: everything placed
    s.execute("map.locationFilter", &json!({"mode": "visible", "bounds": [-85.0, -400.0, 85.0, 400.0]})).unwrap();
    assert_eq!(shown(&mut s).len(), 3);
    // in a saved location
    s.execute("map.saveLocation", &json!({"name": "Tokyo", "lat": 35.68, "lon": 139.7, "radius": 5000})).unwrap();
    s.execute("map.locationFilter", &json!({"mode": "saved"})).unwrap();
    assert_eq!(shown(&mut s), vec![all[1]]);
    // off outside the Map module
    s.location_filter_on = false;
    assert_eq!(shown(&mut s).len(), all.len());
    s.location_filter_on = true;
    assert_eq!(shown(&mut s).len(), 1);
    // read back, then Clear Filters
    assert_eq!(s.execute("map.locationFilter", &json!({})).unwrap()["mode"], "saved");
    s.execute("library.clearFilter", &json!({})).unwrap();
    assert_eq!(shown(&mut s).len(), all.len());
    // nonsense is an error and leaves the filter alone
    for bad in [
        json!({"mode": "visible"}),
        json!({"mode": "visible", "bounds": [10.0, 0.0, -10.0, 5.0]}),
        json!({"mode": "visible", "bounds": [0.0, "x", 1.0, 2.0]}),
        json!({"mode": "everywhere"}),
        json!({"mode": 3}),
    ] {
        assert!(s.execute("map.locationFilter", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.execute("map.locationFilter", &json!({})).unwrap()["mode"], "none");
}

/// Photos without a position, or at exactly 0, 0 (a GPS block written without a fix), get no pin:
/// they were all dropped in the ocean off Africa. They are counted, filtered as untagged, and
/// never "in" a saved location around null island.
#[test]
fn no_position_or_null_island_is_not_placed() {
    let mut s = Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().map(|p| p.id.0).collect();
    s.execute("photo.setMeta", &json!({"ids": ids, "gps": null})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[0]], "gps": [47.6062, -122.3321]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[1], ids[2]], "gps": [0.0, 0.0]})).unwrap();
    // (on the equator / the prime meridian alone is a real place)
    s.execute("photo.setMeta", &json!({"ids": [ids[3]], "gps": [0.0, 32.58]})).unwrap();
    let pins = s.execute("map.pins", &json!({})).unwrap();
    let mut placed: Vec<u64> = pins["pins"].as_array().unwrap().iter().map(|p| p["id"].as_u64().unwrap()).collect();
    placed.sort();
    assert_eq!(placed, vec![ids[0], ids[3]], "{pins}");
    assert_eq!(pins["withoutGps"], ids.len() - 2);
    s.location_filter_on = true;
    s.execute("map.locationFilter", &json!({"mode": "untagged"})).unwrap();
    let vis: Vec<u64> = s.visible_cloned().iter().map(|p| p.0).collect();
    assert!(vis.contains(&ids[1]) && vis.contains(&ids[2]) && !vis.contains(&ids[0]), "0, 0 counts as untagged");
    assert!(crate::map::photos_within(&s.catalog, 0.0, 0.0, 1000.0).is_empty());
    assert_eq!(crate::map::located(Some((0.0, 0.0))), None);
    assert_eq!(crate::map::located(Some((f64::NAN, 1.0))), None);
    assert_eq!(crate::map::located(Some((0.0, 1.0))), Some((0.0, 1.0)));
}
