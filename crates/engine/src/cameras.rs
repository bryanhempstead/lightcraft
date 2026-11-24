//! Cameras: the cameras in the library (and ones Bryan adds by hand), what LightCraft uses for each
//! — which decoder reads its raws, which camera profile colours them (Adobe's DCP on this Mac, the
//! DNG's own, LightCraft's fitted one, or the fallback), the lens profiles found for its lenses —
//! and the last Lightroom ΔE measured for it (`tools/lr-compare`).
//!
//! What is resolved is cached in `<config>/cameras.json` (`~/Library/Application
//! Support/LightCraft/` on macOS: derived data, never Adobe's files and never in the repository).
//! An import that brings a camera the cache hasn't seen resolves it right after the photos are in
//! (`newCameras` in the import's report) — never before or during, so it can't hold an import up.
//! `<config>/lr-compare.json` (`{"cameras": {"Canon EOS R6": {"de": 2.5, …}}}`) is written by
//! `tools/lr-compare/lr_compare.py compare`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lightcraft_catalog::{MediaKind, Photo, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Session;

/// Format of `cameras.json`.
const VERSION: u32 = 1;
/// Largest `cameras.json` / `lr-compare.json` read.
const MAX_FILE: u64 = 8 << 20;
/// Largest raw read to see which decoder takes it.
const MAX_RAW: u64 = 600 << 20;
/// Most cameras kept (a hostile file can't grow the list without bound).
const MAX_CAMERAS: usize = 2000;

/// The camera as Lightroom shows it (the Exif model): the stored "make model" loses the make when
/// the model repeats it — "Canon Canon EOS R6" → "Canon EOS R6", "Leica Camera AG LEICA M (Typ 262)"
/// → "LEICA M (Typ 262)"; "Canon EOS R6" or "SONY ILCE-7M3" stay as they are.
pub fn display_name(camera: &str) -> &str {
    let words: Vec<&str> = camera.split_whitespace().collect();
    let Some(first) = words.first() else { return camera.trim() };
    for k in 1..words.len() {
        if words[k].eq_ignore_ascii_case(first) {
            // the model starts at the k-th word: return the rest of the original text
            let mut rest = camera.trim_start();
            for w in &words[..k] {
                rest = rest.trim_start().strip_prefix(w).unwrap_or(rest);
            }
            return rest.trim();
        }
    }
    camera.trim()
}

/// (make, model) of a stored "make model" camera: the part [`display_name`] drops is the make;
/// otherwise the first word ("SONY ILCE-7M3" → SONY, ILCE-7M3).
pub fn split(camera: &str) -> (String, String) {
    let camera = camera.trim();
    let shown = display_name(camera);
    if shown.len() < camera.len()
        && let Some(make) = camera.strip_suffix(shown)
    {
        return (make.trim().to_string(), shown.to_string());
    }
    match camera.split_once(char::is_whitespace) {
        Some((make, model)) => (make.to_string(), model.trim().to_string()),
        None => (String::new(), camera.to_string()),
    }
}

fn key(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// A lens seen on a camera and the profile found for it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensRecord {
    pub lens: String,
    /// The installed `.lcp`'s profile name ("Adobe (Canon RF 24-105mm F4 L IS USM)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

/// What LightCraft resolved for one camera.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// As Lightroom shows it ([`display_name`]).
    pub name: String,
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
    /// Added by hand (cameras.add), kept when no photo of it is in the library.
    #[serde(default)]
    pub added: bool,
    #[serde(default)]
    pub first_seen: String,
    #[serde(default)]
    pub resolved: String,
    /// `native` (LightCraft's decoder), `libraw`, `embedded preview` (raw variant not decoded:
    /// the camera's JPEG), `rendered` (JPEG / HEIC only), `unknown` (no file to read).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder_note: Option<String>,
    /// Raw format read ("Cr3", "Dng", …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// The camera's `Adobe Standard` DCP on this Mac.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard: Option<String>,
    /// Camera Raw's Camera Matching profiles for it ("Camera Standard", …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matching: Vec<String>,
    /// Its DNGs carry their own colour profile (matrices / tables).
    #[serde(default)]
    pub embedded: bool,
    /// LightCraft's own fitted profile (`camera-profiles/<model>.json` or built in).
    #[serde(default)]
    pub own: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lenses: Vec<LensRecord>,
    /// Lens profiles installed for its make.
    #[serde(default)]
    pub make_lens_profiles: usize,
}

impl Record {
    /// Which profile colours it: `adobe`, `embedded`, `own` or `fallback`.
    pub fn profile_status(&self) -> &'static str {
        if self.standard.is_some() {
            "adobe"
        } else if self.embedded {
            "embedded"
        } else if self.own {
            "own"
        } else {
            "fallback"
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    cameras: Vec<Record>,
}

/// `cameras.json`: the session's override, else (a library on disk only, so demo and test
/// sessions never write there) `LIGHTCRAFT_CAMERAS`, else `<config>/cameras.json`.
pub fn store_path(s: &Session) -> Option<PathBuf> {
    if let Some(p) = &s.cameras_file {
        return Some(p.clone());
    }
    s.library.as_ref()?;
    std::env::var_os("LIGHTCRAFT_CAMERAS").map(PathBuf::from).or_else(|| crate::camera_profiles::config_dir().map(|d| d.join("cameras.json")))
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > MAX_FILE {
        log::warn!("cameras: {} is larger than {} MB, ignored", path.display(), MAX_FILE >> 20);
        return None;
    }
    std::fs::read(path).ok()
}

/// The cached records (empty when there is no file or it is damaged: they are re-resolved).
pub fn load(s: &Session) -> Vec<Record> {
    let Some(path) = store_path(s) else { return s.cameras_mem.clone() };
    let Some(bytes) = read_capped(&path) else { return Vec::new() };
    match serde_json::from_slice::<Store>(&bytes) {
        Ok(st) if st.version <= VERSION => st.cameras.into_iter().take(MAX_CAMERAS).filter(|r| !r.name.trim().is_empty()).collect(),
        Ok(st) => {
            log::warn!("cameras: {} is format v{}, newer than this build's v{VERSION}; not used", path.display(), st.version);
            Vec::new()
        }
        Err(e) => {
            log::warn!("cameras: {}: {e}; resolving again", path.display());
            Vec::new()
        }
    }
}

fn save(s: &mut Session, records: &[Record]) -> Result<(), String> {
    let Some(path) = store_path(s) else {
        s.cameras_mem = records.to_vec();
        return Ok(());
    };
    let st = Store { version: VERSION, cameras: records.to_vec() };
    let text = serde_json::to_vec_pretty(&st).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Lightroom ΔE measured by `lr_compare.py` for a camera (by name, case-insensitive).
fn delta_e(s: &Session) -> BTreeMap<String, Value> {
    let Some(path) = store_path(s).and_then(|p| p.parent().map(|d| d.join("lr-compare.json"))) else { return BTreeMap::new() };
    let Some(v) = read_capped(&path).and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return BTreeMap::new() };
    v["cameras"].as_object().into_iter().flatten().take(MAX_CAMERAS).map(|(k, v)| (key(display_name(k)), v.clone())).collect()
}

/// A camera's photos in the library (not in Recently Deleted, not Local browse records).
struct Seen<'a> {
    camera: String,
    photos: Vec<&'a Photo>,
}

fn library_cameras(s: &Session) -> BTreeMap<String, Seen<'_>> {
    let mut m: BTreeMap<String, Seen> = BTreeMap::new();
    for p in s.catalog.photos().filter(|p| !p.deleted && !p.local) {
        let cam = p.meta.camera.trim();
        if cam.is_empty() {
            continue;
        }
        m.entry(key(display_name(cam))).or_insert_with(|| Seen { camera: cam.to_string(), photos: Vec::new() }).photos.push(p);
    }
    m
}

/// Which decoder reads `p` (and its format, whether it is a DNG with its own profile).
fn probe(p: &Photo) -> (String, Option<String>, Option<String>, bool) {
    if let Some(why) = &p.preview_only {
        return ("embedded preview".into(), Some(why.clone()), None, false);
    }
    let Source::File { path } = &p.source else { return ("unknown".into(), Some("not a file".into()), None, false) };
    let bytes = match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_RAW => return ("unknown".into(), Some(format!("{path}: too large to read")), None, false),
        Ok(_) => match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => return ("unknown".into(), Some(format!("{path}: {e}")), None, false),
        },
        Err(_) => return ("unknown".into(), Some("original offline".into()), None, false),
    };
    match lightcraft_raw::probe_info_reporting(&bytes) {
        Ok((info, d)) => {
            let format = format!("{:?}", info.format);
            let embedded = info.format == lightcraft_raw::RawFormat::Dng && (info.color.color_matrix[0].is_some() || !info.color.profile.is_empty());
            let dec = match d {
                lightcraft_raw::Decoder::Native => "native",
                lightcraft_raw::Decoder::LibRaw => "libraw",
            };
            (dec.into(), None, Some(format), embedded)
        }
        Err(e) => ("embedded preview".into(), Some(e.to_string()), None, false),
    }
}

/// Resolve one camera: profiles and lens data from the Adobe data on this Mac, the decoder from
/// one of its photos (`photos`: its photos in the library, maybe none).
pub fn resolve(name: &str, camera: &str, photos: &[&Photo], old: Option<&Record>, now: &str) -> Record {
    let (make, model) = split(camera);
    let make_opt = Some(make.as_str()).filter(|m| !m.is_empty());
    let mut r = Record {
        name: name.to_string(),
        make: make.clone(),
        model: model.clone(),
        added: old.is_some_and(|o| o.added),
        first_seen: old.map(|o| o.first_seen.clone()).filter(|f| !f.is_empty()).unwrap_or_else(|| now.to_string()),
        resolved: now.to_string(),
        ..Default::default()
    };
    r.standard = crate::adobe::standard_path(make_opt, &model).map(|p| p.display().to_string());
    r.matching = crate::adobe::camera_matching(make_opt, &model);
    r.own = crate::camera_profiles::get(&model).is_some();
    r.make_lens_profiles = if make.is_empty() { 0 } else { crate::lcp::count_for_make(&make) };
    let mut lenses: Vec<String> = Vec::new();
    for p in photos {
        let l = p.meta.lens.trim();
        if !l.is_empty() && !lenses.iter().any(|x| x.eq_ignore_ascii_case(l)) && lenses.len() < 64 {
            lenses.push(l.to_string());
        }
    }
    lenses.sort_by_key(|a| a.to_lowercase());
    r.lenses = lenses
        .into_iter()
        .map(|lens| {
            let found = crate::lcp::profile_name(make_opt, Some(&model), &lens);
            LensRecord { profile: found.as_ref().map(|f| f.0.clone()), file: found.map(|f| f.1.display().to_string()), lens }
        })
        .collect();
    // the decoder: a raw whose file is here (newest first), else what the photos are
    let mut raws: Vec<&&Photo> = photos.iter().filter(|p| p.kind == MediaKind::Raw).collect();
    raws.sort_by(|a, b| b.imported.cmp(&a.imported));
    let pick =
        raws.iter().find(|p| p.preview_only.is_none() && matches!(&p.source, Source::File { path } if Path::new(path).is_file())).or(raws.first());
    match pick {
        Some(p) => {
            let (d, note, format, embedded) = probe(p);
            r.decoder = Some(d);
            r.decoder_note = note;
            r.format = format.or_else(|| Some(p.format.clone()));
            r.embedded = embedded;
        }
        None if !photos.is_empty() => {
            r.decoder = Some("rendered".into());
            r.format = photos.first().map(|p| p.format.clone());
        }
        None => {}
    }
    r
}

fn json_of(r: Option<&Record>, name: &str, seen: Option<&Seen>, de: &BTreeMap<String, Value>) -> Value {
    let photos = seen.map_or(0, |s| s.photos.len());
    let raws = seen.map_or(0, |s| s.photos.iter().filter(|p| p.kind == MediaKind::Raw).count());
    let preview_only = seen.map_or(0, |s| s.photos.iter().filter(|p| p.preview_only.is_some()).count());
    let mut lens_counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for p in seen.map(|s| s.photos.as_slice()).unwrap_or_default() {
        let l = p.meta.lens.trim();
        if !l.is_empty() {
            lens_counts.entry(l.to_lowercase()).or_insert_with(|| (l.to_string(), 0)).1 += 1;
        }
    }
    let lenses: Vec<Value> = match r {
        Some(r) if !r.lenses.is_empty() => r
            .lenses
            .iter()
            .map(|l| json!({"lens": l.lens, "photos": lens_counts.get(&l.lens.to_lowercase()).map_or(0, |c| c.1), "profile": l.profile, "file": l.file}))
            .collect(),
        _ => lens_counts.values().map(|(l, n)| json!({"lens": l, "photos": n, "profile": null})).collect(),
    };
    let mut v = json!({
        "name": name,
        "photos": photos,
        "raws": raws,
        "previewOnly": preview_only,
        "inLibrary": photos > 0,
        "resolved": r.is_some(),
        "lenses": lenses,
        "deltaE": de.get(&key(name)).cloned(),
    });
    if let Some(r) = r {
        v["make"] = json!(r.make);
        v["model"] = json!(r.model);
        v["added"] = json!(r.added);
        v["firstSeen"] = json!(r.first_seen);
        v["resolvedAt"] = json!(r.resolved);
        v["decoder"] = json!(r.decoder);
        v["decoderNote"] = json!(r.decoder_note);
        v["format"] = json!(r.format);
        v["profile"] = json!({
            "status": r.profile_status(),
            "standard": r.standard,
            "matching": r.matching,
            "embedded": r.embedded,
            "own": r.own,
        });
        v["makeLensProfiles"] = json!(r.make_lens_profiles);
    } else if let Some(s) = seen {
        let (make, model) = split(&s.camera);
        v["make"] = json!(make);
        v["model"] = json!(model);
    }
    v
}

/// `cameras.list`: the library's cameras (most photos first), then the ones added by hand.
pub fn list(s: &Session) -> Value {
    let records = load(s);
    let seen = library_cameras(s);
    let de = delta_e(s);
    let mut rows: Vec<(usize, Value)> = Vec::new();
    for (k, sn) in &seen {
        let name = display_name(&sn.camera).to_string();
        let r = records.iter().find(|r| key(&r.name) == *k);
        rows.push((sn.photos.len(), json_of(r, &name, Some(sn), &de)));
    }
    for r in records.iter().filter(|r| r.added && !seen.contains_key(&key(&r.name))) {
        rows.push((0, json_of(Some(r), &r.name, None, &de)));
    }
    rows.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| a.1["name"].as_str().unwrap_or("").to_lowercase().cmp(&b.1["name"].as_str().unwrap_or("").to_lowercase()))
    });
    json!({
        "cameras": rows.into_iter().map(|r| r.1).collect::<Vec<_>>(),
        "adobe": crate::adobe::enabled(),
        "cache": store_path(s).map(|p| p.display().to_string()),
    })
}

/// Resolve the library's cameras (`only`: those names; `all`: every one again, else only the
/// ones not resolved yet) and the hand-added ones. Returns the names resolved.
pub fn refresh(s: &mut Session, only: Option<&[String]>, all: bool) -> Result<Vec<String>, String> {
    let now = (s.clock)();
    let mut records = load(s);
    let seen = library_cameras(s);
    let mut done = Vec::new();
    let wanted = |k: &str| only.is_none_or(|o| o.iter().any(|n| key(display_name(n)) == k));
    for (k, sn) in &seen {
        if !wanted(k) {
            continue;
        }
        let pos = records.iter().position(|r| key(&r.name) == *k);
        if pos.is_some() && !all && only.is_none() {
            continue;
        }
        let name = display_name(&sn.camera).to_string();
        let r = resolve(&name, &sn.camera, &sn.photos, pos.and_then(|i| records.get(i)), &now);
        match pos {
            Some(i) => {
                if let Some(slot) = records.get_mut(i) {
                    *slot = r;
                }
            }
            None => records.push(r),
        }
        done.push(name);
    }
    for r in records.iter_mut().filter(|r| r.added && !seen.contains_key(&key(&r.name))) {
        if !wanted(&key(&r.name)) || (!all && only.is_none() && !r.resolved.is_empty()) {
            continue;
        }
        let camera = if r.make.is_empty() { r.model.clone() } else { format!("{} {}", r.make, r.model) };
        *r = resolve(&r.name.clone(), &camera, &[], Some(&r.clone()), &now);
        done.push(r.name.clone());
    }
    records.truncate(MAX_CAMERAS);
    if !done.is_empty() {
        save(s, &records)?;
    }
    Ok(done)
}

/// The Adobe camera name that best matches what was typed ("r6" → "Canon EOS R6"), if any.
pub fn known_match(query: &str) -> Option<String> {
    let q = key(query);
    if q.is_empty() {
        return None;
    }
    let known = crate::adobe::standard_cameras();
    known.iter().find(|n| key(n) == q).or_else(|| known.iter().find(|n| key(n).ends_with(&format!(" {q}")))).cloned()
}

/// Camera Raw's cameras matching `query` (for the add field's suggestions).
pub fn known(query: &str, limit: usize) -> Vec<String> {
    let q = key(query);
    crate::adobe::standard_cameras().into_iter().filter(|n| q.is_empty() || key(n).contains(&q)).take(limit).collect()
}

/// `cameras.add`: add a camera by name (typed or picked), resolve it and keep it.
pub fn add(s: &mut Session, typed: &str) -> Result<Value, String> {
    let typed = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    if typed.is_empty() {
        return Err("type a camera, e.g. Canon EOS R5".into());
    }
    if typed.chars().count() > 120 {
        return Err("a camera name is at most 120 characters".into());
    }
    // Camera Raw's own name when one matches ("eos r5" → "Canon EOS R5")
    let camera = known_match(&typed).unwrap_or(typed);
    let name = display_name(&camera).to_string();
    let mut records = load(s);
    let k = key(&name);
    let seen = library_cameras(s);
    let now = (s.clock)();
    let photos: Vec<&Photo> = seen.get(&k).map(|sn| sn.photos.clone()).unwrap_or_default();
    let pos = records.iter().position(|r| key(&r.name) == k);
    let mut r = resolve(&name, &camera, &photos, pos.and_then(|i| records.get(i)), &now);
    r.added = true;
    match pos {
        Some(i) => {
            if let Some(slot) = records.get_mut(i) {
                *slot = r.clone();
            }
        }
        None => {
            if records.len() >= MAX_CAMERAS {
                return Err(format!("at most {MAX_CAMERAS} cameras"));
            }
            records.push(r.clone());
        }
    }
    drop(seen);
    save(s, &records)?;
    Ok(
        json!({"name": r.name, "profile": r.profile_status(), "standard": r.standard, "matching": r.matching, "makeLensProfiles": r.make_lens_profiles, "notice": notice(&r)}),
    )
}

/// `cameras.remove`: forget a camera added by hand (the library's own come back on refresh).
pub fn remove(s: &mut Session, name: &str) -> Result<bool, String> {
    let k = key(display_name(name));
    let mut records = load(s);
    let before = records.len();
    records.retain(|r| key(&r.name) != k);
    let removed = records.len() != before;
    if removed {
        save(s, &records)?;
    }
    Ok(removed)
}

/// One line about a camera just recognised: what colours it and what was found for its lenses.
pub fn notice(r: &Record) -> String {
    let profile = match r.profile_status() {
        "adobe" if r.matching.is_empty() => "Adobe Standard profile found".to_string(),
        "adobe" => format!("Adobe Standard + {} camera matching profile{} found", r.matching.len(), if r.matching.len() == 1 { "" } else { "s" }),
        "embedded" => "colors from its DNG's own profile".to_string(),
        "own" => "LightCraft's fitted profile".to_string(),
        _ => "no Adobe profile on this Mac — LightCraft's own color".to_string(),
    };
    let with = r.lenses.iter().filter(|l| l.profile.is_some()).count();
    let lenses = if r.lenses.is_empty() {
        String::new()
    } else {
        format!(
            ", lens profile{} for {with} of {} lens{}",
            if with == 1 { "" } else { "s" },
            r.lenses.len(),
            if r.lenses.len() == 1 { "" } else { "es" }
        )
    };
    format!("New camera: {} — {profile}{lenses}", r.name)
}

/// After an import: cameras of `imported` the cache hasn't seen are resolved and kept; their
/// notices (`newCameras`). A failure to save only logs: an import never fails on this.
pub fn recognise_new(s: &mut Session, imported: &[u64]) -> Vec<Value> {
    if imported.is_empty() {
        return Vec::new();
    }
    let known: std::collections::HashSet<String> = load(s).iter().map(|r| key(&r.name)).collect();
    let mut fresh: Vec<String> = Vec::new();
    for id in imported {
        let Some(p) = s.catalog.photo(lightcraft_catalog::PhotoId(*id)) else { continue };
        let cam = p.meta.camera.trim();
        if cam.is_empty() {
            continue;
        }
        let k = key(display_name(cam));
        if !known.contains(&k) && !fresh.contains(&k) {
            fresh.push(k);
        }
    }
    if fresh.is_empty() {
        return Vec::new();
    }
    let names: Vec<String> = fresh.clone();
    if let Err(e) = refresh(s, Some(&names), false) {
        log::warn!("cameras: couldn't keep the new cameras: {e}");
    }
    let records = load(s);
    fresh
        .iter()
        .filter_map(|k| records.iter().find(|r| key(&r.name) == *k))
        .map(|r| json!({"name": r.name, "profile": r.profile_status(), "notice": notice(r)}))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_drops_a_repeated_make() {
        assert_eq!(display_name("Canon Canon EOS R6"), "Canon EOS R6");
        assert_eq!(display_name("Leica Camera AG LEICA M (Typ 262)"), "LEICA M (Typ 262)");
        assert_eq!(display_name("RICOH IMAGING COMPANY, LTD. RICOH GR III"), "RICOH GR III");
        assert_eq!(display_name("Canon EOS R6"), "Canon EOS R6");
        assert_eq!(display_name("SONY ILCE-7M3"), "SONY ILCE-7M3");
        assert_eq!(display_name("  "), "");
    }

    #[test]
    fn split_finds_make_and_model() {
        assert_eq!(split("Canon Canon EOS R6"), ("Canon".into(), "Canon EOS R6".into()));
        assert_eq!(split("SONY ILCE-7M3"), ("SONY".into(), "ILCE-7M3".into()));
        assert_eq!(split("Leica Camera AG LEICA M (Typ 262)"), ("Leica Camera AG".into(), "LEICA M (Typ 262)".into()));
        assert_eq!(split("X100F"), (String::new(), "X100F".into()));
        assert_eq!(split(""), (String::new(), String::new()));
    }
}
