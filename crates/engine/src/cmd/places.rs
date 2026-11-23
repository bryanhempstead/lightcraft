//! LR-MAP-PLACES: look places up (`location.search`), put them on photos, albums and folders
//! (`location.apply`, `location.setDefault`), and suggested keywords (`keywords.suggest`,
//! `keywords.applySuggested`, `keywords.dropSuggested`). See [`crate::places`].

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::places::{self, Place, Scope};
use crate::{Result, Session};

fn strs(p: &Value, key: &str) -> Option<Vec<String>> {
    p.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

/// `location.search {q, limit?}`: blocking (network) for agents and the CLI; the app searches on a
/// worker thread with the same [`places::lookup`].
fn search(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "location.search";
    let q = str_param(p, "q").ok_or_else(|| bad(C, "missing `q`: a place, business, address or coordinates"))?;
    let limit = p.get("limit").and_then(Value::as_u64).unwrap_or(8) as usize;
    let geo = s.places.geocoder.clone();
    let cache = places::cache_path(s);
    let (results, from) = places::lookup(&*geo, cache.as_deref(), q, limit).map_err(|e| bad(C, e))?;
    Ok(json!({"results": results, "from": from, "geocoder": geo.name(), "attribution": places::ATTRIBUTION}))
}

/// The place in `place` (an object), else the first result for `q`.
fn place_param(s: &mut Session, p: &Value, c: &str) -> Result<Place> {
    if let Some(v) = p.get("place").filter(|v| !v.is_null()) {
        return Place::from_json(v).map_err(|e| bad(c, e));
    }
    if let Some(q) = str_param(p, "q") {
        let geo = s.places.geocoder.clone();
        let cache = places::cache_path(s);
        let (r, _) = places::lookup(&*geo, cache.as_deref(), q, 1).map_err(|e| bad(c, e))?;
        return r.into_iter().next().ok_or_else(|| bad(c, format!("no place found for “{q}”")));
    }
    Err(bad(c, "give a `place` {name, lat, lon, location?, city?, state?, country?} or `q` to look one up"))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "location.apply";
    let scope = Scope::from_params(s, p, C)?;
    let place = place_param(s, p, C)?;
    let keywords = p.get("keywords").and_then(Value::as_bool).unwrap_or(s.import_defaults.place_keywords);
    let remember = p.get("setDefault").and_then(Value::as_bool).unwrap_or(true);
    let mut r = places::apply(s, &scope, &place, keywords, remember)?;
    r["scope"] = scope.json(&s.catalog);
    Ok(r)
}

fn scope_key(s: &Session, p: &Value, c: &str) -> Result<(Scope, String)> {
    let scope = Scope::from_params(s, p, c)?;
    let key = scope.key().ok_or_else(|| bad(c, "an album or folder has a place of its own: give `album` or `folder`"))?;
    Ok((scope, key))
}

fn set_default(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "location.setDefault";
    let (scope, key) = scope_key(s, p, C)?;
    let place = match p.get("place") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Place::from_json(v).map_err(|e| bad(C, e))?),
    };
    let op = places::scope_op(&s.catalog, &key, |i| i.place = place.clone());
    s.commit(if place.is_some() { "Set Album Place" } else { "Clear Album Place" }, op)?;
    Ok(json!({"scope": scope.json(&s.catalog), "place": place}))
}

fn get_default(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "location.default";
    let (scope, key) = scope_key(s, p, C)?;
    let info = places::scope_info(&s.catalog, &key);
    Ok(json!({"scope": scope.json(&s.catalog), "place": info.place, "dropped": info.dropped}))
}

fn suggest(s: &mut Session, p: &Value) -> Result<Value> {
    let scope = Scope::from_params(s, p, "keywords.suggest")?;
    let content = p.get("content").and_then(Value::as_bool).unwrap_or(true);
    Ok(places::suggestions(s, &scope, content))
}

fn apply_suggested(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "keywords.applySuggested";
    let scope = Scope::from_params(s, p, C)?;
    let remove = strs(p, "remove").unwrap_or_default();
    let drop = strs(p, "drop").unwrap_or_default();
    let add = match strs(p, "keywords") {
        Some(k) => k,
        // everything suggested that not every photo has yet
        None if remove.is_empty() && drop.is_empty() => {
            let sug = places::suggestions(s, &scope, true);
            let n = sug["photos"].as_u64().unwrap_or(0);
            sug["suggestions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|x| x["have"].as_u64().unwrap_or(0) < n)
                .filter_map(|x| x["keyword"].as_str().map(str::to_string))
                .collect()
        }
        None => Vec::new(),
    };
    places::apply_suggested(s, &scope, &add, &remove, &drop)
}

fn drop_suggested(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "keywords.dropSuggested";
    let scope = Scope::from_params(s, p, C)?;
    let drop = strs(p, "keywords").filter(|k| !k.is_empty()).ok_or_else(|| bad(C, "missing `keywords`: the suggestions to drop"))?;
    places::apply_suggested(s, &scope, &[], &[], &drop)
}

/// Library repairs: change every photo whose metadata `fix` changes, as one undo step (`dryRun`:
/// just count them).
fn repair(s: &mut Session, p: &Value, label: &str, fix: impl Fn(&mut lightcraft_catalog::Meta)) -> Result<Value> {
    let ids: Vec<lightcraft_catalog::PhotoId> = s.catalog.photos().filter(|p| !p.local).map(|p| p.id).collect();
    let ops = places::meta_ops(&s.catalog, &ids, fix);
    let n = ops.len();
    let dry = p.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
    if !dry && n > 0 {
        s.commit(label, lightcraft_catalog::Op::Batch { ops })?;
    }
    Ok(json!({"photos": n, "dryRun": dry}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "location.search",
            "Find Place",
            [],
            None,
            "{q: a business, place, street address or coordinates (47.6062, -122.3321 or 47°36'22\"N 122°19'56\"W), limit?: 8} — coordinates are read locally (and named when the geocoder can); names and addresses are looked up with OpenStreetMap Nominatim (only the typed text is sent; answers cached on disk) → {results: [{name, address, lat, lon, location, city, state, country, kind}], from: coordinates|cache|geocoder, geocoder, attribution}",
            always,
            search
        ),
        cmd!(
            "location.apply",
            "Set Location",
            [],
            None,
            "{ids? | album?: id | folder?: path (default: the selection), place: {name?, lat, lon, location?, city?, state?, country?, kind?} | q: text (its first result), keywords?: bool (add the place's keywords; default: Settings ▸ Import ▸ place keywords), setDefault?: bool (an album / folder keeps it for new imports; default true)} — GPS + Location / City / State / Country on every photo, written to XMP like other metadata; one undo step → {photos, changed, place, keywords, remembered, scope}",
            always,
            apply
        ),
        cmd!(
            "location.setDefault",
            "Set Album Place",
            [],
            None,
            "{album: id | folder: path, place: {…} | null} — the place photos imported into it get (null clears it; the photos already there are left alone)",
            always,
            set_default
        ),
        cmd!(query "location.default", "Album Place", [], None, "{album: id | folder: path} → {scope, place|null, dropped: [suggestions dropped here]}", always, get_default),
        cmd!(
            query "keywords.suggest",
            "Suggest Keywords",
            [],
            None,
            "{ids? | album?: id | folder?: path (default: the selection), content?: bool (default true: what the photos show, from the AI search index)} → {scope, photos, suggestions: [{keyword, from: place|name|content, score, have}], looksLike, place, dropped, content: ok|why not}",
            always,
            suggest
        ),
        cmd!(
            "keywords.applySuggested",
            "Add Suggested Keywords",
            [],
            None,
            "{ids? | album? | folder?, keywords?: [..] (default: every suggestion not on all the photos yet), remove?: [..] (taken off the photos and not suggested again here), drop?: [..] (not suggested again here)} — one undo step → {photos, changed, added, removed}",
            always,
            apply_suggested
        ),
        cmd!(
            "library.fixVoidGps",
            "Clear Void GPS Positions",
            [],
            None,
            "{dryRun?: bool} — photos stored at exactly 0, 0 (a GPS block written without a fix, e.g. Leica M262 frames with GPSStatus V) lose that position; one undo step → {photos, dryRun}",
            always,
            |s, p| repair(s, p, "Clear Void GPS", |m| {
                if m.gps.is_some_and(|g| g.0 == 0.0 && g.1 == 0.0) {
                    m.gps = None;
                }
            })
        ),
        cmd!(
            "library.fixLensNames",
            "Repair Lens Names",
            [],
            None,
            "{dryRun?: bool} — placeholder lens names (\"not selected\", \"----\", \"Unknown\") become no lens, and bodies with a built-in lens (RICOH GR III, Fujifilm X100V, Leica Q …) get theirs; one undo step → {photos, dryRun}",
            always,
            |s, p| repair(s, p, "Repair Lens Names", |m| m.lens = lightcraft_catalog::clean_lens(&m.camera, &m.lens))
        ),
        cmd!(
            "keywords.dropSuggested",
            "Drop Suggested Keywords",
            [],
            None,
            "{ids? | album? | folder?, keywords: [..]} — not suggested again for this album / folder (undoable; for a selection, until LightCraft quits)",
            always,
            drop_suggested
        ),
    ]
}
