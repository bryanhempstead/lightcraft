//! LR-MAP-MODULE: the Map module's commands — geotag photos (drop them on the map), clear their
//! GPS, list the pins of the photos in view, and Saved Locations (kept in the catalog under
//! [`crate::map::LOCATIONS_KEY`], so they are undoable, backed up and journalled like the rest).

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, has_selection, str_param};
use crate::map::{self, LocationFilter, SavedLocation};
use crate::{Result, Session};
use lightcraft_catalog::{Op, PhotoId};

fn lat_lon(p: &Value, c: &str) -> Result<(f64, f64)> {
    let (lat, lon) = match (p.get("lat").and_then(Value::as_f64), p.get("lon").and_then(Value::as_f64)) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(bad(c, "`lat` and `lon` are numbers (degrees)")),
    };
    if !map::valid(lat, lon) {
        return Err(bad(c, format!("{lat}, {lon} isn't a place on Earth (latitude ±90°, longitude ±180°)")));
    }
    Ok((lat, lon))
}

fn location_json(s: &Session, l: &SavedLocation) -> Value {
    let n = map::photos_within(&s.catalog, l.lat, l.lon, l.radius).len();
    json!({"id": l.id, "name": l.name, "lat": l.lat, "lon": l.lon, "radius": l.radius, "private": l.private, "photos": n})
}

fn store_locations(s: &mut Session, label: &str, list: &[SavedLocation]) -> Result<()> {
    let value = if list.is_empty() { None } else { Some(serde_json::to_value(list).map_err(|e| bad("map", e.to_string()))?) };
    s.commit(label, Op::SetRecord { key: map::LOCATIONS_KEY.into(), value })
}

fn save_location(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "map.saveLocation";
    let mut list = map::locations(&s.catalog);
    let id = p.get("id").and_then(Value::as_u64);
    let existing = id.and_then(|i| list.iter().position(|l| l.id == i));
    if id.is_some() && existing.is_none() {
        return Err(bad(C, format!("no saved location {}", id.unwrap_or_default())));
    }
    let old = existing.and_then(|i| list.get(i).cloned());
    let name = str_param(p, "name").map(str::trim).map(str::to_string).or_else(|| old.as_ref().map(|o| o.name.clone())).unwrap_or_default();
    if name.is_empty() {
        return Err(bad(C, "a saved location needs a `name`"));
    }
    if name.chars().count() > 200 {
        return Err(bad(C, "the name is longer than 200 characters"));
    }
    let (lat, lon) = match (p.get("lat"), old.as_ref()) {
        (None, Some(o)) => (o.lat, o.lon),
        _ => lat_lon(p, C)?,
    };
    let radius = match p.get("radius") {
        None => old.as_ref().map_or(100.0, |o| o.radius),
        Some(v) => {
            v.as_f64().filter(|r| r.is_finite() && *r > 0.0 && *r <= 2_000_000.0).ok_or_else(|| bad(C, "`radius` is metres, 0 < r ≤ 2000 km"))?
        }
    };
    let private = p.get("private").and_then(Value::as_bool).unwrap_or(old.as_ref().is_some_and(|o| o.private));
    let loc = SavedLocation { id: id.unwrap_or_else(|| list.iter().map(|l| l.id).max().unwrap_or(0) + 1), name, lat, lon, radius, private };
    match existing {
        Some(i) => {
            if let Some(slot) = list.get_mut(i) {
                *slot = loc.clone();
            }
        }
        None => list.push(loc.clone()),
    }
    store_locations(s, if existing.is_some() { "Edit Saved Location" } else { "Save Location" }, &list)?;
    Ok(location_json(s, &loc))
}

fn delete_location(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "map.deleteLocation";
    let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `id`"))?;
    let mut list = map::locations(&s.catalog);
    let n = list.len();
    list.retain(|l| l.id != id);
    if list.len() == n {
        return Err(bad(C, format!("no saved location {id}")));
    }
    store_locations(s, "Delete Saved Location", &list)?;
    Ok(json!({"deleted": id}))
}

fn location_or_place(s: &Session, p: &Value, c: &str) -> Result<(f64, f64, f64)> {
    if let Some(id) = p.get("id").and_then(Value::as_u64) {
        let l = map::locations(&s.catalog).into_iter().find(|l| l.id == id).ok_or_else(|| bad(c, format!("no saved location {id}")))?;
        return Ok((l.lat, l.lon, l.radius));
    }
    let (lat, lon) = lat_lon(p, c)?;
    let r = p.get("radius").and_then(Value::as_f64).filter(|r| r.is_finite() && *r >= 0.0).unwrap_or(100.0);
    Ok((lat, lon, r))
}

/// `map.locationFilter`: set (with `mode`) or read the Map module's location filter.
fn location_filter(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "map.locationFilter";
    if let Some(mode) = p.get("mode") {
        let mode = mode.as_str().ok_or_else(|| bad(C, "`mode` is none|visible|tagged|untagged|saved"))?;
        s.location_filter = match mode {
            "none" => LocationFilter::None,
            "tagged" => LocationFilter::Tagged,
            "untagged" => LocationFilter::Untagged,
            "saved" => LocationFilter::Saved,
            "visible" => {
                let b: Vec<f64> = p.get("bounds").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
                let [south, west, north, east] = b[..] else {
                    return Err(bad(C, "`visible` needs `bounds`: [south, west, north, east] in degrees (the map's view)"));
                };
                LocationFilter::visible(south, west, north, east)
                    .ok_or_else(|| bad(C, "`bounds` are [south, west, north, east] with south ≤ north and west ≤ east"))?
            }
            m => return Err(bad(C, format!("unknown mode `{m}` (none|visible|tagged|untagged|saved)"))),
        };
    }
    let f = s.location_filter;
    let shown = s.visible().len();
    let of = s.visible_unlocated().len();
    let mut out = json!({"mode": f.mode(), "photos": shown, "of": of, "on": s.location_filter_on});
    if let LocationFilter::Visible { south, west, north, east } = f {
        out["bounds"] = json!([south, west, north, east]);
    }
    Ok(out)
}

fn pins(s: &mut Session, p: &Value) -> Result<Value> {
    // the pins are the view's, before the location filter narrows the filmstrip
    let ids: Vec<PhotoId> = if p.get("ids").is_some() { s.targets(p) } else { s.visible_unlocated().to_vec() };
    let mut pins = Vec::new();
    let mut without = 0usize;
    for id in ids {
        let Some(ph) = s.catalog.photo(id) else { continue };
        // (no position, or 0, 0: never dropped in the ocean off Africa)
        match map::located(ph.meta.gps) {
            Some((lat, lon)) => pins.push(json!({"id": id.0, "lat": lat, "lon": lon})),
            None => without += 1,
        }
    }
    Ok(json!({"pins": pins, "withoutGps": without}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "map.geotag",
            "Set GPS Coordinates",
            [],
            None,
            "{ids?, lat, lon} — puts the photos (default: the selection) at this place, as dropping them on the map does (undoable) → {photos}",
            has_selection,
            |s, p| {
                let (lat, lon) = lat_lon(p, "map.geotag")?;
                let ids: Vec<u64> = s.targets(p).iter().map(|i| i.0).collect();
                if ids.is_empty() {
                    return Err(bad("map.geotag", "no photos to place"));
                }
                s.execute("photo.setMeta", &json!({"ids": ids, "gps": [lat, lon]}))?;
                Ok(json!({"photos": ids.len(), "lat": lat, "lon": lon}))
            }
        ),
        cmd!(
            "map.clearGps",
            "Delete GPS Coordinates",
            ["Metadata"],
            None,
            "{ids?} — removes the photos' GPS position (undoable)",
            has_selection,
            |s, p| {
                let ids: Vec<u64> = s.targets(p).iter().map(|i| i.0).collect();
                s.execute("photo.setMeta", &json!({"ids": ids, "gps": null}))?;
                Ok(json!({"photos": ids.len()}))
            }
        ),
        cmd!(
            "map.locationFilter",
            "Location Filter",
            [],
            None,
            "{mode?: none|visible|tagged|untagged|saved, bounds?: [south, west, north, east] (degrees, for visible)} — the Map module's location filter bar: Visible on Map, Tagged, Untagged, in a saved location, or none (applies while the Map shows) → {mode, photos, of, bounds?}",
            always,
            location_filter
        ),
        cmd!(query "map.pins", "Map Pins", [], None, "{ids?} → {pins: [{id, lat, lon}], withoutGps}: photos in view (or `ids`) with a GPS position; photos without one (or at exactly 0, 0) are only counted", always, pins),
        cmd!(query "map.locations", "Saved Locations", [], None, "{} → [{id, name, lat, lon, radius (m), private, photos}]", always, |s, _| {
            let list = map::locations(&s.catalog);
            Ok(Value::Array(list.iter().map(|l| location_json(s, l)).collect()))
        }),
        cmd!(
            "map.saveLocation",
            "Save Location",
            [],
            None,
            "{id?: edit this one, name, lat, lon, radius?: metres (default 100), private?: bool} → the location",
            always,
            save_location
        ),
        cmd!("map.deleteLocation", "Delete Saved Location", [], None, "{id}", always, delete_location),
        cmd!(
            "map.photosIn",
            "Photos in Location",
            [],
            None,
            "{id} | {lat, lon, radius?: metres} — library photos within the radius; filter?: true (default) shows only them, like Find Similar → {photos: [ids]}",
            always,
            |s, p| {
                let (lat, lon, r) = location_or_place(s, p, "map.photosIn")?;
                let ids = map::photos_within(&s.catalog, lat, lon, r);
                if p.get("filter").and_then(Value::as_bool).unwrap_or(true) {
                    s.filter.only = ids.clone();
                    // nothing there: an impossible id keeps the view empty rather than showing all
                    if s.filter.only.is_empty() {
                        s.filter.only = vec![PhotoId(0)];
                    }
                }
                Ok(json!({"photos": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))
            }
        ),
    ]
}
