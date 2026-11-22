//! Lightroom Classic history steps and snapshots for migrated photos (read from a *copy* of the
//! catalog, see [`crate::lr_migrate::with_catalog_copy`]).
//!
//! - **History** (`Adobe_libraryImageDevelopHistoryStep`: name, date, and the develop settings
//!   after the step as zlib-compressed Lua text behind a 4-byte length) is mapped step by step
//!   like the photo's own settings and kept next to the library, one file per photo in
//!   [`HISTORY_DIR`] (the catalog itself only keeps LightCraft's own steps, so loading the
//!   library stays as fast as before: Bryan's catalog has 102,408 steps). Each step is stored
//!   as the JSON change from the step before; the History panel lists them under LightCraft's
//!   own steps and `history.restoreLightroom` goes back to one.
//! - **Snapshots** (`Adobe_libraryImageDevelopSnapshot`, e.g. "From Lr mobile") become versions.
//!
//! Written from the catalog's visible schema and black-box observation; no Adobe code is used.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use lightcraft_catalog::{Op, Photo, PhotoId, Version};
use lightcraft_develop::DevelopSettings;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Session;
use crate::lr_migrate::{LrImage, Records, cocoa_time, library_ids, lr_quarter_turns, map_develop, query, reorient_partial};

/// The folder (in the library) with one `<photo id>.json` per photo: its Lightroom history.
pub const HISTORY_DIR: &str = "Lightroom History";
/// The last step: the photo as the migration left it, when that differs from Lightroom's last.
pub const MIGRATED_STEP: &str = "Migrated to LightCraft";
/// Largest decompressed step / snapshot text (hostile input stays bounded).
const MAX_TEXT: usize = 8 << 20;
/// Steps kept per photo (the newest); Bryan's longest history has 241.
const MAX_STEPS: usize = 1000;
/// Photos per catalog query.
const CHUNK: usize = 300;
/// Largest history file read back.
const MAX_FILE: u64 = 64 << 20;

/// A step or snapshot as the catalog has it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LrStep {
    pub image: i64,
    pub name: String,
    /// ISO 8601 UTC (history steps).
    pub at: Option<String>,
    /// The develop settings after the step (Lua text).
    pub text: String,
}

/// One stored step: the change from the step before (`delta`; the first from the default
/// settings), or the whole settings when a change can't say it (`full`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredStep {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<Value>,
}

/// A photo's Lightroom history file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LrHistory {
    pub catalog: String,
    /// Lightroom's image id.
    pub image: i64,
    pub written: String,
    /// Oldest first (like `Photo::history`).
    pub steps: Vec<StoredStep>,
}

impl LrHistory {
    /// The settings after step `i`.
    pub fn settings(&self, i: usize) -> Option<DevelopSettings> {
        let mut cur = DevelopSettings::default().to_json();
        for st in self.steps.get(..=i)? {
            match (&st.full, &st.delta) {
                (Some(f), _) => cur = f.clone(),
                (None, Some(d)) => merge(&mut cur, d),
                (None, None) => {}
            }
        }
        DevelopSettings::from_json(&cur).ok()
    }
}

/// JSON merge: objects key by key, anything else replaced.
fn merge(into: &mut Value, patch: &Value) {
    match (into, patch) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, v) in b {
                merge(a.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (slot, p) => *slot = p.clone(),
    }
}

/// Steps stored as changes, each checked to rebuild exactly (else kept whole).
pub fn encode_steps(steps: &[(String, Option<String>, DevelopSettings)]) -> Vec<StoredStep> {
    let mut prev = DevelopSettings::default().to_json();
    steps
        .iter()
        .map(|(label, at, d)| {
            let cur = d.to_json();
            let delta = crate::json_delta(&prev, &cur).unwrap_or_else(|| json!({}));
            let mut check = prev.clone();
            merge(&mut check, &delta);
            let st = if check == cur {
                StoredStep { label: label.clone(), at: at.clone(), delta: Some(delta), full: None }
            } else {
                StoredStep { label: label.clone(), at: at.clone(), delta: None, full: Some(cur.clone()) }
            };
            prev = cur;
            st
        })
        .collect()
}

// ------------------------------------------------------------------------------------ reading

/// Hex digits → bytes (`None` on anything else).
fn unhex(h: &str) -> Option<Vec<u8>> {
    let h = h.trim().as_bytes();
    if !h.len().is_multiple_of(2) {
        return None;
    }
    let d = |c: u8| (c as char).to_digit(16).map(|v| v as u8);
    h.chunks(2).map(|p| Some((d(*p.first()?)? << 4) | d(*p.get(1)?)?)).collect()
}

/// A step's stored settings: zlib after a 4-byte big-endian length (history), or plain text.
pub fn decode_text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(b"s =") || bytes.starts_with(b"s=") {
        return String::from_utf8(bytes.to_vec()).map_err(|_| "settings text is not UTF-8".to_string());
    }
    let body = bytes.get(4..).ok_or("settings blob too short")?;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(body, MAX_TEXT).map_err(|e| format!("settings blob: {e:?}"))?;
    String::from_utf8(raw).map_err(|_| "settings text is not UTF-8".to_string())
}

/// A row's text column (`blob` = hex of a blob, or `plain`).
fn row_text(r: &Value) -> Result<String, String> {
    if let Some(p) = r["plain"].as_str() {
        return Ok(p.to_string());
    }
    let h = r["blob"].as_str().ok_or("no settings")?;
    decode_text(&unhex(h).ok_or("settings blob: bad hex")?)
}

fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|x| x.is_finite() && x.abs() < 9e15).map(|x| x as i64))
}

const TEXT_COLS: &str = "CASE typeof(text) WHEN 'blob' THEN hex(text) END AS blob, CASE typeof(text) WHEN 'text' THEN text END AS plain";

/// Read the history of the catalog copy `db`'s photos (`only`: just these Lightroom ids), a
/// chunk of photos at a time (`f` gets each photo's steps, oldest first). Steps whose settings
/// can't be read are counted in the result, not passed on.
pub fn read_history(
    db: &Path,
    only: Option<&HashSet<i64>>,
    mut f: impl FnMut(Vec<(i64, Vec<LrStep>)>) -> Result<(), String>,
) -> Result<usize, String> {
    let mut images: Vec<i64> = query(db, "SELECT DISTINCT image AS image FROM Adobe_libraryImageDevelopHistoryStep WHERE image IS NOT NULL")?
        .iter()
        .filter_map(|r| int(&r["image"]))
        .filter(|i| only.is_none_or(|o| o.contains(i)))
        .collect();
    images.sort_unstable();
    let mut bad = 0usize;
    for chunk in images.chunks(CHUNK) {
        let list = chunk.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let rows = query(
            db,
            &format!(
                "SELECT image, dateCreated AS at, name, {TEXT_COLS} FROM Adobe_libraryImageDevelopHistoryStep WHERE image IN ({list}) ORDER BY image, dateCreated, id_local"
            ),
        )?;
        let mut by: Vec<(i64, Vec<LrStep>)> = Vec::new();
        for r in &rows {
            let Some(image) = int(&r["image"]) else { continue };
            let Ok(text) = row_text(r) else {
                bad += 1;
                continue;
            };
            let step = LrStep { image, name: r["name"].as_str().unwrap_or("").to_string(), at: r["at"].as_f64().and_then(cocoa_time), text };
            match by.last_mut() {
                Some((i, v)) if *i == image => v.push(step),
                _ => by.push((image, vec![step])),
            }
        }
        f(by)?;
    }
    Ok(bad)
}

/// The catalog copy's snapshots (`only`: just these Lightroom ids), per photo.
pub fn read_snapshots(db: &Path, only: Option<&HashSet<i64>>) -> Result<HashMap<i64, Vec<LrStep>>, String> {
    let rows = query(
        db,
        &format!("SELECT image, name, {TEXT_COLS} FROM Adobe_libraryImageDevelopSnapshot WHERE image IS NOT NULL ORDER BY image, id_local"),
    )?;
    let mut out: HashMap<i64, Vec<LrStep>> = HashMap::new();
    for r in &rows {
        let Some(image) = int(&r["image"]).filter(|i| only.is_none_or(|o| o.contains(i))) else { continue };
        let Ok(text) = row_text(r) else { continue };
        out.entry(image).or_default().push(LrStep { image, name: r["name"].as_str().unwrap_or("").trim().to_string(), at: None, text });
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------ mapping

/// What mapping a photo's Lightroom settings needs (cloned out of the session so steps map on
/// worker threads).
struct Target {
    photo: Photo,
    turns: u8,
    raw: bool,
    aspect: f64,
}

impl Target {
    fn new(p: &Photo, im: &LrImage) -> Target {
        let turns = lr_quarter_turns(&im.orientation).unwrap_or(0);
        let aspect = if im.width > 0 && im.height > 0 {
            im.width as f64 / im.height as f64
        } else if p.width > 0 && p.height > 0 {
            let a = p.width as f64 / p.height as f64;
            if turns % 2 == 1 { 1.0 / a } else { a }
        } else {
            0.0
        };
        Target { photo: p.clone(), turns, raw: matches!(im.format.as_str(), "RAW" | "DNG"), aspect }
    }

    /// Lightroom's settings text as ours, the way the migration maps the photo's own: from the
    /// photo's import defaults, turned like the photo, its creative look matched by name.
    fn settings(&self, text: &str, looks: &[(String, String)]) -> Option<DevelopSettings> {
        let mut m = map_develop(text, self.raw, self.aspect).ok()?;
        reorient_partial(&mut m.partial, lightcraft_geom::Orientation::from_parts(false, self.turns));
        if let Some((name, amount)) = &m.creative_look
            && let Some((_, id)) = looks.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
        {
            m.partial["profile"] = json!({"id": id, "amount": (amount * 100.0).clamp(0.0, 200.0)});
        }
        let mut d = lightcraft_develop::apply_partial(&self.photo.import_defaults(), &m.partial, 1.0);
        let now = &self.photo.develop;
        d.orientation = now.orientation;
        // the lens profile the migration picked by name (Lightroom's text only names it)
        if d.optics.lens_profile && d.optics.lens_profile_ref.is_none() {
            d.optics.lens_profile_ref = now.optics.lens_profile_ref.clone();
        }
        Some(d)
    }
}

fn looks(s: &Session) -> Vec<(String, String)> {
    s.lut_profiles.iter().map(|l| (l.name.trim().to_string(), l.id.clone())).collect()
}

fn history_dir(s: &Session) -> Option<PathBuf> {
    s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join(HISTORY_DIR))
}

/// The history file of photo `id`.
pub fn history_path(s: &Session, id: PhotoId) -> Option<PathBuf> {
    history_dir(s).map(|d| d.join(format!("{}.json", id.0)))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Options of [`migrate_extras`].
#[derive(Clone, Debug, Default)]
pub struct ExtrasOptions {
    pub history: bool,
    pub snapshots: bool,
    pub dry_run: bool,
    /// Only these Lightroom image ids (a re-sync's changed photos); `None` = all.
    pub only: Option<HashSet<i64>>,
}

/// Bring the Lightroom history and snapshots of the library's photos over from the catalog copy
/// `db` (`rec`: the same catalog's records, to find the photos). History files are (re)written;
/// snapshots become versions (one undo step; a version already there isn't added twice).
pub fn migrate_extras(s: &mut Session, db: &Path, rec: &Records, opts: &ExtrasOptions) -> crate::Result<Value> {
    store_extras(s, Source::Catalog(db), rec, opts)
}

/// Where [`store_extras`] reads steps and snapshots from.
#[derive(Clone, Copy)]
pub enum Source<'a> {
    /// A catalog copy (history read a chunk of photos at a time).
    Catalog(&'a Path),
    /// Read ahead (e.g. on a worker thread, [`read_extras`]).
    Read(&'a Extras),
}

/// History and snapshots read ahead from a catalog copy (a few photos' worth: a re-sync's).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Extras {
    pub history: Vec<(i64, Vec<LrStep>)>,
    pub snapshots: HashMap<i64, Vec<LrStep>>,
    /// Steps whose settings couldn't be read.
    pub unreadable: usize,
}

/// Read the history and snapshots of the photos `only` (Lightroom ids) from catalog copy `db`.
pub fn read_extras(db: &Path, only: &HashSet<i64>) -> Result<Extras, String> {
    let mut ex = Extras::default();
    if only.is_empty() {
        return Ok(ex);
    }
    ex.unreadable = read_history(db, Some(only), |chunk| {
        ex.history.extend(chunk);
        Ok(())
    })?;
    ex.snapshots = read_snapshots(db, Some(only))?;
    Ok(ex)
}

/// [`migrate_extras`] from either source.
pub fn store_extras(s: &mut Session, src: Source, rec: &Records, opts: &ExtrasOptions) -> crate::Result<Value> {
    let ids = library_ids(s, rec);
    let by_lr: HashMap<i64, &LrImage> = rec.images.iter().map(|i| (i.id, i)).collect();
    let looks = looks(s);
    let now = (s.clock)();
    let err = |e: String| crate::EngineError::Other(format!("Lightroom history: {e}"));
    let mut report = json!({"dryRun": opts.dry_run});

    // ---- history → files
    let mut snapshot_times: HashMap<i64, HashMap<String, String>> = HashMap::new();
    if opts.history {
        let dir = history_dir(s);
        if let (Some(d), false) = (&dir, opts.dry_run) {
            std::fs::create_dir_all(d).map_err(|e| err(format!("{}: {e}", d.display())))?;
        }
        let (mut photos, mut steps, mut unmapped, mut failed) = (0usize, 0usize, 0usize, 0usize);
        let mut not_in_library = 0usize;
        let catalog = rec.catalog.clone();
        let session: &Session = s;
        let mut each = |chunk: Vec<(i64, Vec<LrStep>)>| -> Result<(), String> {
            let chunk: Vec<(i64, Vec<LrStep>)> = chunk.into_iter().filter(|(i, _)| opts.only.as_ref().is_none_or(|o| o.contains(i))).collect();
            let jobs: Vec<(i64, PhotoId, Target, Vec<LrStep>)> = chunk
                .into_iter()
                .filter_map(|(image, st)| {
                    // (snapshots take the time of the step that made them)
                    let times = snapshot_times.entry(image).or_default();
                    for x in &st {
                        if let Some(at) = &x.at {
                            times.insert(x.name.trim().to_string(), at.clone());
                        }
                    }
                    let Some((id, im)) = ids.get(&image).copied().zip(by_lr.get(&image)) else {
                        not_in_library += 1;
                        return None;
                    };
                    let p = session.catalog.photo(id)?;
                    Some((image, id, Target::new(p, im), st))
                })
                .collect();
            let done: Vec<(i64, PhotoId, Vec<StoredStep>, usize)> = jobs
                .into_par_iter()
                .map(|(image, id, t, st)| {
                    let skip = st.len().saturating_sub(MAX_STEPS);
                    let mut mapped: Vec<(String, Option<String>, DevelopSettings)> = Vec::new();
                    let mut lost = 0usize;
                    for x in st.into_iter().skip(skip) {
                        match t.settings(&x.text, &looks) {
                            Some(d) => mapped.push((x.name, x.at, d)),
                            None => lost += 1,
                        }
                    }
                    if mapped.last().is_some_and(|(_, _, d)| *d != *t.photo.develop) || (mapped.is_empty() && lost > 0) {
                        mapped.push((MIGRATED_STEP.to_string(), Some(now.clone()), (*t.photo.develop).clone()));
                    }
                    (image, id, encode_steps(&mapped), lost)
                })
                .collect();
            for (image, id, st, lost) in done {
                photos += 1;
                steps += st.len();
                unmapped += lost;
                if opts.dry_run {
                    continue;
                }
                let Some(d) = &dir else { continue };
                let h = LrHistory { catalog: catalog.clone(), image, written: now.clone(), steps: st };
                let bytes = serde_json::to_vec(&h).map_err(|e| e.to_string())?;
                if write_atomic(&d.join(format!("{}.json", id.0)), &bytes).is_err() {
                    failed += 1;
                }
            }
            Ok(())
        };
        let bad = match src {
            Source::Catalog(db) => read_history(db, opts.only.as_ref(), each).map_err(err)?,
            Source::Read(ex) => {
                each(ex.history.clone()).map_err(err)?;
                ex.unreadable
            }
        };
        report["history"] = json!({
            "photos": photos,
            "steps": steps,
            "unreadable": bad,
            "unmapped": unmapped,
            "notInLibrary": not_in_library,
            "failedWrites": failed,
            "stored": dir.is_some(),
        });
    }

    // ---- snapshots → versions
    if opts.snapshots {
        let snaps = match src {
            Source::Catalog(db) => read_snapshots(db, opts.only.as_ref()).map_err(err)?,
            Source::Read(ex) => {
                ex.snapshots.iter().filter(|(i, _)| opts.only.as_ref().is_none_or(|o| o.contains(i))).map(|(i, v)| (*i, v.clone())).collect()
            }
        };
        let (mut added, mut already, mut not_in_library, mut unmapped) = (0usize, 0usize, 0usize, 0usize);
        let mut ops = Vec::new();
        let mut list: Vec<(&i64, &Vec<LrStep>)> = snaps.iter().collect();
        list.sort_by_key(|(i, _)| **i);
        for (image, snaps) in list {
            let Some((id, im)) = ids.get(image).copied().zip(by_lr.get(image)) else {
                not_in_library += snaps.len();
                continue;
            };
            let Some(p) = s.catalog.photo(id) else { continue };
            let t = Target::new(p, im);
            let mut versions = p.versions.clone();
            for sn in snaps {
                let Some(d) = t.settings(&sn.text, &looks) else {
                    unmapped += 1;
                    continue;
                };
                let name = if sn.name.is_empty() { "Lightroom Snapshot".to_string() } else { sn.name.clone() };
                if versions.iter().any(|v| v.name == name && *v.settings == d) {
                    already += 1;
                    continue;
                }
                let created = snapshot_times.get(image).and_then(|m| m.get(&name)).cloned().unwrap_or_else(|| now.clone());
                versions.push(Version { name, created, settings: Arc::new(d), auto: false });
                added += 1;
            }
            if versions != p.versions {
                ops.push(Op::SetVersions { id, versions });
            }
        }
        if !opts.dry_run && !ops.is_empty() {
            s.commit("Lightroom Snapshots", Op::Batch { ops })?;
        }
        report["snapshots"] = json!({"added": added, "alreadyThere": already, "notInLibrary": not_in_library, "unmapped": unmapped});
    }
    Ok(report)
}

// ------------------------------------------------------------------------------------ reading back

/// Recently read history files (path, modified, contents).
type Cache = Vec<(PathBuf, std::time::SystemTime, Arc<LrHistory>)>;
static CACHE: Mutex<Cache> = Mutex::new(Vec::new());

/// Photo `id`'s Lightroom history (cached; `None` when it has none).
pub fn history_of(s: &Session, id: PhotoId) -> Option<Arc<LrHistory>> {
    let path = history_path(s, id)?;
    let meta = std::fs::metadata(&path).ok()?;
    let modified = meta.modified().ok()?;
    if meta.len() > MAX_FILE {
        return None;
    }
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, _, h)) = cache.iter().find(|(p, m, _)| *p == path && *m == modified) {
        return Some(h.clone());
    }
    let h: LrHistory = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok())?;
    let h = Arc::new(h);
    cache.retain(|(p, _, _)| *p != path);
    cache.push((path, modified, h.clone()));
    if cache.len() > 16 {
        cache.remove(0);
    }
    Some(h)
}

#[cfg(test)]
#[path = "tests_lr_history.rs"]
pub(crate) mod tests;
