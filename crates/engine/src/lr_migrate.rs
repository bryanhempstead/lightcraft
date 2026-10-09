//! Migrate from Lightroom Classic: a `.lrcat` catalog's photos, ratings, flags, colour labels,
//! keywords, IPTC text, collections (smart ones where the rules map), virtual copies and develop
//! settings, plus the user's preset folders.
//!
//! The catalog is an SQLite database. LightCraft has no SQLite reader of its own (pure Rust, no
//! C dependencies), so [`dump_catalog`] copies the catalog (with its write-ahead log, so the
//! latest edits are included) to a scratch folder and reads it with the system's `sqlite3`
//! command-line tool (`-json`); the original is never opened. The joined result ([`Records`]) can
//! also be saved and read back as JSON. [`migrate`] then imports the files in place (add mode),
//! and applies everything else as one undo step. Develop settings are the catalog's Lua text
//! (`s = { Exposure2012 = …, … }`), mapped like an `.lrtemplate` preset ([`crate::crs`]).
//!
//! Written from the catalog's visible schema (table and column names) and black-box observation;
//! no Adobe code or content is used.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lightcraft_catalog::{Album, AlbumId, ColorLabel, Flag, Op, PhotoId, Source};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Session;
use crate::preset_import::{Lua, develop_text_props, parse_lua};

/// Seconds from the Unix epoch to Lightroom's (Cocoa) epoch, 2001-01-01 UTC.
const COCOA_EPOCH: f64 = 978_307_200.0;
/// The album folder earlier versions of the migration wrapped every collection in (now the
/// collections keep the catalog's own tree; see [`flatten_wrapper`]).
pub const ALBUM_FOLDER: &str = "From Lightroom";
/// The file (in the library folder) that keeps Lightroom's per-photo times for
/// `library.resumePoint`.
pub const RESUME_FILE: &str = "lightroom-migration.json";
/// Largest records file / develop text read (hostile input stays bounded).
const MAX_RECORDS_BYTES: u64 = 2 << 30;

/// One photo of the catalog (a master, or a virtual copy of one).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LrImage {
    pub id: i64,
    pub path: String,
    /// The catalog root folder the file is under (for reporting missing volumes).
    pub root: String,
    /// A virtual copy: the catalog id of its master.
    pub master: Option<i64>,
    pub copy_name: Option<String>,
    /// A copy Lightroom made itself to resolve a sync conflict.
    pub sync_duplicate: bool,
    pub rating: u8,
    /// 1 pick, −1 reject, 0 none.
    pub pick: i8,
    pub label: String,
    /// `AB` (as shot), `BC`, `CD`, `DA`: quarter turns clockwise of the photo as shown.
    pub orientation: String,
    pub keywords: Vec<String>,
    pub caption: String,
    pub copyright: String,
    pub creator: String,
    pub location: String,
    pub city: String,
    pub state: String,
    pub country: String,
    /// The develop settings (Lua text).
    pub develop: String,
    /// Develop steps beyond the import in Lightroom's history.
    pub edits: u32,
    /// When it was last edited in Lightroom (ISO 8601, UTC).
    pub last_edit: Option<String>,
    /// When it was last selected / changed in Lightroom (ISO 8601, UTC).
    pub touched: Option<String>,
    pub format: String,
    /// The file's stored (unrotated) size as Lightroom recorded it.
    pub width: u32,
    pub height: u32,
    /// Work done ahead (on a worker thread) by [`prepare`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepared: Option<Prepared>,
}

/// What [`prepare`] works out per photo without the session: is the file there, how its own
/// EXIF turns it, and its develop settings mapped to ours.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prepared {
    pub exists: bool,
    /// Quarter turns the file's own EXIF orientation makes (read only for photos Lightroom shows
    /// turned).
    pub file_turns: u8,
    /// Partial develop settings in our frame (crop, masks, spots turned with the photo).
    pub develop: Option<Value>,
    pub unmapped: Vec<String>,
    /// The look differs from Lightroom's defaults.
    pub custom: bool,
    pub creative_look: Option<(String, f64)>,
    pub error: Option<String>,
}

/// A collection, collection set or smart collection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LrCollection {
    pub id: i64,
    pub name: String,
    pub parent: Option<i64>,
    /// `collection`, `set`, `smart` or `quick` (Lightroom's Quick Collection, which becomes
    /// ours).
    pub kind: String,
    /// Smart collections: the rules (Lua text).
    pub rules: Option<String>,
    /// Catalog image ids in collection order.
    pub images: Vec<i64>,
}

/// Everything read from a catalog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Records {
    pub catalog: String,
    pub images: Vec<LrImage>,
    pub collections: Vec<LrCollection>,
}

/// Options of [`migrate`].
#[derive(Clone, Debug, Default)]
pub struct MigrateOptions {
    /// Import the files (add mode). Off when the app has imported them already.
    pub import: bool,
    /// Only the first `limit` photos whose files exist (and their virtual copies).
    pub limit: Option<usize>,
    pub dry_run: bool,
    /// Also apply develop settings to photos Lightroom never edited (its defaults).
    pub develop_all: bool,
    /// Only the collections (albums, smart albums, sets, the Quick Collection): for a library
    /// migrated already. Nothing is imported, no photo is changed, no preset is read.
    pub collections_only: bool,
    /// Only photos this run imports (e.g. files an earlier run failed on, now readable): photos
    /// already in the library keep their ratings, metadata and develop settings untouched.
    pub only_new: bool,
    /// Preset folders to import (none = no presets).
    pub preset_dirs: Vec<String>,
    /// Creative look name → one of our profile ids (e.g. a `.cube` imported with
    /// `profile.import`). Looks with an imported profile of the same name match by themselves.
    pub look_map: HashMap<String, String>,
    /// Folders (and files, zips) whose creative profiles — camera-raw XMP "Look" profiles with a
    /// colour table, presets carrying one, `.cube` files — are imported first so photos using
    /// them match (`profile.import`; one already imported isn't imported twice).
    pub profile_dirs: Vec<String>,
}

/// The profile one of our profiles a creative look `name` matches: `look_map`'s choice, else an
/// imported profile of that name (case-insensitive).
fn matching_profile(s: &Session, look_map: &HashMap<String, String>, name: &str) -> Option<String> {
    look_map.get(name).cloned().or_else(|| s.lut_profiles.iter().find(|l| l.name.eq_ignore_ascii_case(name.trim())).map(|l| l.id.clone()))
}

/// Import the creative profiles in `dirs` (none on a dry run: their names are returned instead,
/// to count matches).
fn import_profiles(s: &mut Session, dirs: &[String], dry: bool) -> crate::Result<(Value, Vec<String>)> {
    if dirs.is_empty() {
        return Ok((Value::Null, Vec::new()));
    }
    if dry {
        let names = crate::cmd::lut_profiles::profile_names(dirs);
        return Ok((json!({"found": names.len()}), names));
    }
    let r = crate::cmd::lut_profiles::import_paths(s, dirs)?;
    Ok((json!({"imported": r.imported, "alreadyImported": r.known, "failed": r.failed}), Vec::new()))
}

/// The creative look each photo had in Lightroom, as the migration recorded it in
/// [`RESUME_FILE`] (`looks: {photoId: {name, amount}}`).
fn recorded_looks(s: &Session) -> HashMap<PhotoId, (String, f64)> {
    let Some(dir) = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone()) else { return HashMap::new() };
    let doc: Value = std::fs::read(dir.join(RESUME_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    doc["looks"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| {
                    let id = PhotoId(k.parse().ok()?);
                    Some((id, (v["name"].as_str()?.to_string(), v["amount"].as_f64().filter(|a| a.is_finite()).unwrap_or(1.0))))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Repair a library migrated by an earlier version, from the catalog's records:
/// - `crops`: crops whose straighten angle is still exactly the mirror of Lightroom's (an earlier
///   version read `crs:CropAngle` the wrong way round) get the right one;
/// - `unedited`: photos Lightroom shows unedited but that carry other settings (taken from an XMP
///   sidecar next to the file at import) get Lightroom's settings.
///
/// Photos (and virtual copies) changed here since the migration are left alone. One undo step.
pub fn repair_migration(s: &mut Session, rec: &Records, crops: bool, unedited: bool, dry_run: bool) -> crate::Result<Value> {
    let by_path = photos_by_path(s);
    let lr_by_id: HashMap<i64, &LrImage> = rec.images.iter().map(|i| (i.id, i)).collect();
    let migrated_at = s
        .library
        .as_ref()
        .filter(|l| l.on_disk)
        .and_then(|l| std::fs::read(l.dir.join(RESUME_FILE)).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|d| d["migrated"].as_str().map(str::to_string));
    let (mut fixed, mut kept, mut reset, mut ops) = (0usize, 0usize, 0usize, Vec::new());
    for im in &rec.images {
        let target = match im.master {
            None => by_path.get(&im.path).copied(),
            Some(m) => {
                let master = lr_by_id.get(&m).and_then(|mi| by_path.get(&mi.path).copied());
                let name = im.copy_name.clone().unwrap_or_else(|| "Copy 1".into());
                master.and_then(|mid| {
                    s.catalog.photos().find(|p| p.copy_of == Some(mid) && p.copy_name.as_deref() == Some(name.as_str())).map(|p| p.id)
                })
            }
        };
        let Some(p) = target.and_then(|id| s.catalog.photo(id)) else { continue };
        let aspect = (p.width > 0 && p.height > 0).then(|| p.width as f64 / p.height as f64);
        let prep = prepare_image(im, aspect);
        let Some(partial) = prep.develop else { continue };
        let changed_here = match (&p.edited, &migrated_at) {
            (Some(e), Some(m)) => e.as_str() > m.as_str(),
            _ => false,
        };
        // Lightroom shows it unedited: its settings are Lightroom's defaults
        if unedited && im.edits == 0 && !prep.custom && im.master.is_none() && p.is_edited() {
            if changed_here {
                kept += 1;
                continue;
            }
            reset += 1;
            if !dry_run {
                let d = lightcraft_develop::apply_partial(&p.camera_defaults(), &partial, 1.0);
                ops.push(Op::SetDevelop { id: p.id, settings: Arc::new(d), label: "Lightroom Settings".into(), edited: None });
            }
            continue;
        }
        let Some(want) = partial["crop"]["geometry"]["angle"].as_f64().filter(|_| crops) else { continue };
        let have = p.develop.crop.geometry.angle;
        if want == 0.0 || (have - want).abs() < 1e-9 {
            continue;
        }
        if (have + want).abs() > 1e-6 {
            kept += 1;
            continue;
        }
        fixed += 1;
        if !dry_run {
            let mut d = (*p.develop).clone();
            d.crop.geometry.angle = want;
            ops.push(Op::SetDevelop { id: p.id, settings: Arc::new(d), label: "Fix Lightroom Crop".into(), edited: p.edited.clone() });
        }
    }
    if !ops.is_empty() {
        s.commit("Repair Lightroom Migration", Op::Batch { ops })?;
    }
    Ok(json!({"dryRun": dry_run, "cropsFixed": fixed, "uneditedReset": reset, "keptChangedHere": kept}))
}

/// Give photos the imported creative profile their Lightroom look names (Summer Fields,
/// Nautica, … imported from XMP profiles or `.cube` files): for a library migrated before those
/// profiles were imported. Profiles in `profile_dirs` are imported first. The looks come from the
/// migration's record ([`RESUME_FILE`]) or, for photos it doesn't list, from `rec` (the catalog).
/// A photo whose profile was changed here since (anything but the default) keeps it unless
/// `force`. One undo step.
pub fn rematch_profiles(s: &mut Session, rec: Option<&Records>, opts: &MigrateOptions, force: bool) -> crate::Result<Value> {
    let (profiles, dry_names) = import_profiles(s, &opts.profile_dirs, opts.dry_run)?;
    let mut looks = recorded_looks(s);
    if let Some(rec) = rec {
        let by_path = photos_by_path(s);
        for im in rec.images.iter().filter(|i| i.master.is_none()) {
            let Some(&id) = by_path.get(&im.path) else { continue };
            if looks.contains_key(&id) {
                continue;
            }
            if let Some(look) = prepare_image(im, None).creative_look {
                looks.insert(id, look);
            }
        }
    }
    let default_id = lightcraft_develop::Profile::default().id;
    let (mut matched, mut kept, mut unmatched) = (0usize, 0usize, BTreeMap::<String, usize>::new());
    let mut by_look: BTreeMap<String, usize> = BTreeMap::new();
    let mut ops = Vec::new();
    let mut ids: Vec<PhotoId> = looks.keys().copied().collect();
    ids.sort();
    for id in ids {
        let Some((name, amount)) = looks.get(&id) else { continue };
        let Some(p) = s.catalog.photo(id) else { continue };
        let target = if opts.dry_run {
            dry_names.iter().any(|n| n.eq_ignore_ascii_case(name.trim())).then(|| format!("lut:{name}"))
        } else {
            matching_profile(s, &opts.look_map, name)
        };
        let Some(pid) = target else {
            *unmatched.entry(name.clone()).or_default() += 1;
            continue;
        };
        let current = p.develop.profile.id.as_str();
        let current_name = s.lut_profiles.iter().find(|l| l.id == current).map(|l| l.name.as_str());
        if current == pid || current_name.is_some_and(|n| n.eq_ignore_ascii_case(name.trim())) {
            continue;
        }
        if !force && !(current.is_empty() || current == default_id) {
            kept += 1;
            continue;
        }
        matched += 1;
        *by_look.entry(name.clone()).or_default() += 1;
        if !opts.dry_run {
            let mut d = (*p.develop).clone();
            d.profile = lightcraft_develop::Profile { id: pid, amount: (amount * 100.0).clamp(0.0, 200.0) };
            ops.push(Op::SetDevelop { id, settings: Arc::new(d), label: "Match Profiles".into(), edited: p.edited.clone() });
        }
    }
    if !ops.is_empty() {
        s.commit("Match Lightroom Profiles", Op::Batch { ops })?;
    }
    Ok(json!({"dryRun": opts.dry_run, "profiles": profiles, "matched": matched, "byLook": by_look, "keptOwnProfile": kept, "unmatched": unmatched}))
}

// ---------------------------------------------------------------------------------- reading

/// Lightroom (Cocoa) seconds → ISO 8601 UTC.
pub fn cocoa_time(secs: f64) -> Option<String> {
    let unix = secs + COCOA_EPOCH;
    // 1970..2200
    (unix.is_finite() && (0.0..7.3e9).contains(&unix)).then(|| crate::import::civil(unix as i64))
}

fn text(v: &Value, k: &str) -> String {
    match v.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}
fn number(v: &Value, k: &str) -> Option<f64> {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_f64().filter(|x| x.is_finite()),
        Some(Value::String(s)) => s.trim().parse().ok().filter(|x: &f64| x.is_finite()),
        _ => None,
    }
}
fn int(v: &Value, k: &str) -> Option<i64> {
    number(v, k).filter(|x| x.abs() < 9e15).map(|x| x as i64)
}

const SQL_IMAGES: &str = "SELECT i.id_local AS id, i.masterImage AS master, i.copyName AS copyName, i.copyReason AS copyReason, \
 i.rating AS rating, i.pick AS pick, i.colorLabels AS label, i.orientation AS orientation, i.touchTime AS touchTime, \
 i.fileFormat AS format, i.fileWidth AS width, i.fileHeight AS height, rf.absolutePath AS root, fo.pathFromRoot AS folder, f.baseName AS base, f.extension AS ext, \
 d.text AS develop, h.lastEdit AS lastEdit, h.edits AS edits, ip.caption AS caption, ip.copyright AS copyright, \
 cr.value AS creator, lo.value AS location, ci.value AS city, st.value AS state, co.value AS country \
 FROM Adobe_images i \
 JOIN AgLibraryFile f ON f.id_local = i.rootFile \
 JOIN AgLibraryFolder fo ON fo.id_local = f.folder \
 JOIN AgLibraryRootFolder rf ON rf.id_local = fo.rootFolder \
 LEFT JOIN Adobe_imageDevelopSettings d ON d.image = i.id_local \
 LEFT JOIN (SELECT image, max(dateCreated) AS lastEdit, sum(name NOT LIKE 'Import (%') AS edits \
   FROM Adobe_libraryImageDevelopHistoryStep GROUP BY image) h ON h.image = i.id_local \
 LEFT JOIN AgLibraryIPTC ip ON ip.image = i.id_local \
 LEFT JOIN AgHarvestedIptcMetadata hm ON hm.image = i.id_local \
 LEFT JOIN AgInternedIptcCreator cr ON cr.id_local = hm.creatorRef \
 LEFT JOIN AgInternedIptcLocation lo ON lo.id_local = hm.locationRef \
 LEFT JOIN AgInternedIptcCity ci ON ci.id_local = hm.cityRef \
 LEFT JOIN AgInternedIptcState st ON st.id_local = hm.stateRef \
 LEFT JOIN AgInternedIptcCountry co ON co.id_local = hm.countryRef \
 ORDER BY i.id_local";
const SQL_KEYWORDS: &str = "SELECT id_local AS id, parent, name FROM AgLibraryKeyword";
const SQL_IMAGE_KEYWORDS: &str = "SELECT image, tag FROM AgLibraryKeywordImage";
const SQL_COLLECTIONS: &str = "SELECT id_local AS id, name, parent, creationId AS kind, systemOnly AS system FROM AgLibraryCollection";
const SQL_COLLECTION_IMAGES: &str = "SELECT collection, image FROM AgLibraryCollectionImage ORDER BY collection, positionInCollection, id_local";
const SQL_SMART: &str = "SELECT collection, content FROM AgLibraryCollectionContent WHERE owningModule = 'ag.library.smart_collection'";

/// Join the catalog's rows (each query's `sqlite3 -json` result) into [`Records`].
pub fn records_from_rows(
    catalog: &str,
    images: &[Value],
    keywords: &[Value],
    image_keywords: &[Value],
    collections: &[Value],
    collection_images: &[Value],
    smart: &[Value],
) -> Records {
    // keyword paths `parent|child` (the root keyword has no name)
    let kw: HashMap<i64, (Option<i64>, String)> =
        keywords.iter().filter_map(|r| Some((int(r, "id")?, (int(r, "parent"), text(r, "name"))))).collect();
    let kw_path = |mut id: i64| -> Option<String> {
        let mut parts = Vec::new();
        let mut seen = HashSet::new();
        while let Some((parent, name)) = kw.get(&id) {
            if !seen.insert(id) || parts.len() > 32 {
                break;
            }
            if !name.trim().is_empty() {
                parts.push(name.trim().replace('|', "/"));
            }
            match parent {
                Some(p) => id = *p,
                None => break,
            }
        }
        parts.reverse();
        (!parts.is_empty()).then(|| parts.join("|"))
    };
    let mut by_image: HashMap<i64, Vec<String>> = HashMap::new();
    for r in image_keywords {
        if let (Some(i), Some(t)) = (int(r, "image"), int(r, "tag"))
            && let Some(p) = kw_path(t)
        {
            let list = by_image.entry(i).or_default();
            if !list.contains(&p) {
                list.push(p);
            }
        }
    }
    let images = images
        .iter()
        .filter_map(|r| {
            let id = int(r, "id")?;
            let ext = text(r, "ext");
            let root = text(r, "root");
            let path = format!("{root}{}{}{}", text(r, "folder"), text(r, "base"), if ext.is_empty() { String::new() } else { format!(".{ext}") });
            let pick = number(r, "pick").unwrap_or(0.0);
            Some(LrImage {
                id,
                path,
                root,
                master: int(r, "master"),
                copy_name: Some(text(r, "copyName")).filter(|s| !s.is_empty()),
                sync_duplicate: text(r, "copyReason").contains("sync duplicate"),
                rating: number(r, "rating").unwrap_or(0.0).clamp(0.0, 5.0) as u8,
                pick: if pick > 0.0 {
                    1
                } else if pick < 0.0 {
                    -1
                } else {
                    0
                },
                label: text(r, "label"),
                orientation: text(r, "orientation"),
                keywords: by_image.remove(&id).unwrap_or_default(),
                caption: text(r, "caption"),
                copyright: text(r, "copyright"),
                creator: text(r, "creator"),
                location: text(r, "location"),
                city: text(r, "city"),
                state: text(r, "state"),
                country: text(r, "country"),
                develop: text(r, "develop"),
                edits: number(r, "edits").unwrap_or(0.0).clamp(0.0, 1e9) as u32,
                last_edit: number(r, "lastEdit").and_then(cocoa_time),
                touched: number(r, "touchTime").filter(|t| *t > 0.0).and_then(cocoa_time),
                format: text(r, "format"),
                width: number(r, "width").unwrap_or(0.0).clamp(0.0, 1e6) as u32,
                height: number(r, "height").unwrap_or(0.0).clamp(0.0, 1e6) as u32,
                prepared: None,
            })
        })
        .collect();
    let mut members: HashMap<i64, Vec<i64>> = HashMap::new();
    for r in collection_images {
        if let (Some(c), Some(i)) = (int(r, "collection"), int(r, "image")) {
            members.entry(c).or_default().push(i);
        }
    }
    let rules: HashMap<i64, String> = smart.iter().filter_map(|r| Some((int(r, "collection")?, text(r, "content")))).collect();
    let collections = collections
        .iter()
        .filter_map(|r| {
            let id = int(r, "id")?;
            // `systemOnly` is a number (`1.0`) in some catalogs and text in others
            let system = number(r, "system").is_some_and(|n| n != 0.0) || matches!(text(r, "system").as_str(), "1" | "true");
            let kind = match text(r, "kind").as_str() {
                // the one system-owned plain collection: the Quick Collection
                "com.adobe.ag.library.collection"
                    if system || (text(r, "name").eq_ignore_ascii_case("quick collection") && int(r, "parent").is_none()) =>
                {
                    "quick"
                }
                "com.adobe.ag.library.collection" => "collection",
                "com.adobe.ag.library.group" => "set",
                "com.adobe.ag.library.smart_collection" => "smart",
                // print / slideshow / web drafts, publish services, the quick collection
                _ => return None,
            };
            Some(LrCollection {
                id,
                name: text(r, "name"),
                parent: int(r, "parent"),
                kind: kind.into(),
                rules: rules.get(&id).cloned(),
                images: members.remove(&id).unwrap_or_default(),
            })
        })
        .collect();
    Records { catalog: catalog.to_string(), images, collections }
}

/// Read a records JSON file (written by [`dump_catalog`] callers / `recordsOut`).
pub fn read_records(path: &str) -> Result<Records, String> {
    let len = std::fs::metadata(path).map_err(|e| format!("{path}: {e}"))?.len();
    if len > MAX_RECORDS_BYTES {
        return Err(format!("{path}: too large ({len} bytes)"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{path}: not a records file ({e})"))
}

/// The `sqlite3` tool: `/usr/bin/sqlite3` (macOS), else the one on `PATH`.
fn sqlite3() -> PathBuf {
    let sys = Path::new("/usr/bin/sqlite3");
    if sys.is_file() { sys.to_path_buf() } else { PathBuf::from("sqlite3") }
}

fn query(db: &Path, sql: &str) -> Result<Vec<Value>, String> {
    let out = std::process::Command::new(sqlite3())
        .arg("-json")
        .arg(db)
        .arg(sql)
        .output()
        .map_err(|e| format!("can't run sqlite3 (needed to read Lightroom catalogs): {e}"))?;
    if !out.status.success() {
        return Err(format!("sqlite3: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let t = String::from_utf8_lossy(&out.stdout);
    if t.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(&t).map_err(|e| format!("sqlite3 output: {e}"))
}

/// Read a Lightroom Classic catalog. It is copied (with its `-wal` log, which holds the latest
/// changes) into a fresh folder under `scratch` and read from there; the copy is removed again.
pub fn dump_catalog(lrcat: &Path, scratch: &Path) -> Result<Records, String> {
    if !lrcat.is_file() {
        return Err(format!("{}: no such catalog", lrcat.display()));
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = scratch.join(format!("lightcraft-lrcat-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let db = dir.join("catalog.lrcat");
    let r = (|| {
        std::fs::copy(lrcat, &db).map_err(|e| format!("copying {}: {e}", lrcat.display()))?;
        let wal = PathBuf::from(format!("{}-wal", lrcat.display()));
        if wal.is_file() {
            std::fs::copy(&wal, dir.join("catalog.lrcat-wal")).map_err(|e| format!("copying {}: {e}", wal.display()))?;
        }
        let images = query(&db, SQL_IMAGES)?;
        let keywords = query(&db, SQL_KEYWORDS)?;
        let image_keywords = query(&db, SQL_IMAGE_KEYWORDS)?;
        let collections = query(&db, SQL_COLLECTIONS)?;
        let collection_images = query(&db, SQL_COLLECTION_IMAGES)?;
        let smart = query(&db, SQL_SMART)?;
        Ok(records_from_rows(&lrcat.display().to_string(), &images, &keywords, &image_keywords, &collections, &collection_images, &smart))
    })();
    // only what we made: the copy, its log and index, and the folder
    for f in ["catalog.lrcat", "catalog.lrcat-wal", "catalog.lrcat-shm", "catalog.lrcat-journal"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let _ = std::fs::remove_dir(&dir);
    r
}

/// The main catalog of a default Lightroom Classic setup, if there is one: the most recently
/// changed `.lrcat` in `~/Pictures/Lightroom`.
pub fn default_catalog() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = Path::new(&home).join("Pictures").join("Lightroom");
    std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrcat")) && p.is_file())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// The folders a default Lightroom Classic setup keeps its presets in (those that exist): the
/// catalog's `Lightroom Settings` (presets stored with the catalog), Camera Raw's settings, and
/// Lightroom's own preset folders.
pub fn default_preset_dirs(catalog: Option<&Path>) -> Vec<String> {
    let mut dirs = Vec::new();
    if let Some(c) = catalog.and_then(Path::parent) {
        let s = c.join("Lightroom Settings");
        dirs.extend(["Settings", "Develop Presets", "Keyword Sets"].iter().map(|d| s.join(d)));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let adobe = Path::new(&home).join("Library").join("Application Support").join("Adobe");
        dirs.push(adobe.join("CameraRaw").join("Settings"));
        dirs.push(adobe.join("CameraRaw").join("ImportedSettings"));
        dirs.push(adobe.join("Lightroom").join("Develop Presets"));
        dirs.push(adobe.join("Lightroom").join("Keyword Sets"));
        // develop presets and profiles that ended up in the import presets folder
        dirs.push(adobe.join("Lightroom").join("Import Presets").join("User Presets"));
    }
    dirs.into_iter().filter(|d| d.is_dir()).map(|d| d.display().to_string()).collect()
}

// --------------------------------------------------------------------------------- develop

/// Quarter turns clockwise of Lightroom's orientation code.
fn lr_quarter_turns(code: &str) -> Option<u8> {
    match code.trim() {
        "AB" => Some(0),
        "BC" => Some(1),
        "CD" => Some(2),
        "DA" => Some(3),
        _ => None,
    }
}

/// Did the user change the look in Lightroom (beyond its defaults)? Read from the settings
/// themselves, for photos whose history doesn't say (an import preset, settings synced in).
pub fn has_custom_look(props: &crate::crs::Props, values: &crate::crs_masks::Values) -> bool {
    let num = |k: &str| props.get(&format!("crs:{k}")).and_then(|v| v.first()).and_then(|s| s.trim().trim_start_matches('+').parse::<f64>().ok());
    let txt = |k: &str| props.get(&format!("crs:{k}")).and_then(|v| v.first()).map(|s| s.trim().to_ascii_lowercase());
    const ZERO: &[&str] = &[
        "Exposure2012",
        "Contrast2012",
        "Highlights2012",
        "Shadows2012",
        "Whites2012",
        "Blacks2012",
        "Texture",
        "Clarity2012",
        "Dehaze",
        "Vibrance",
        "Saturation",
        "CropAngle",
        "IncrementalTemperature",
        "IncrementalTint",
        "ParametricShadows",
        "ParametricDarks",
        "ParametricLights",
        "ParametricHighlights",
        "SplitToningShadowSaturation",
        "SplitToningHighlightSaturation",
        "ColorGradeMidtoneSat",
        "ColorGradeGlobalSat",
        "ColorGradeShadowLum",
        "ColorGradeMidtoneLum",
        "ColorGradeHighlightLum",
        "ColorGradeGlobalLum",
        "PostCropVignetteAmount",
        "GrainAmount",
        "PerspectiveVertical",
        "PerspectiveHorizontal",
        "PerspectiveRotate",
        "PerspectiveUpright",
        "LuminanceSmoothing",
        "RedHue",
        "RedSaturation",
        "GreenHue",
        "GreenSaturation",
        "BlueHue",
        "BlueSaturation",
        "ShadowTint",
    ];
    if ZERO.iter().any(|k| num(k).is_some_and(|v| v.abs() > 1e-9)) {
        return true;
    }
    let bands = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];
    if bands.iter().any(|b| ["Hue", "Saturation", "Luminance"].iter().any(|a| num(&format!("{a}Adjustment{b}")).is_some_and(|v| v.abs() > 1e-9))) {
        return true;
    }
    if txt("HasCrop").as_deref() == Some("true") || txt("ConvertToGrayscale").as_deref() == Some("true") {
        return true;
    }
    if txt("WhiteBalance").is_some_and(|w| w != "as shot") {
        return true;
    }
    let identity = |k: &str| {
        props
            .get(&format!("crs:{k}"))
            .is_none_or(|v| v.len() == 2 && v.iter().all(|p| p.split(',').map(str::trim).collect::<Vec<_>>().windows(2).all(|w| w[0] == w[1])))
    };
    if ["ToneCurvePV2012", "ToneCurvePV2012Red", "ToneCurvePV2012Green", "ToneCurvePV2012Blue"].iter().any(|k| !identity(k)) {
        return true;
    }
    if txt("Look/crs:Name").is_some_and(|n| n != "adobe color" && n != "adobe standard") {
        return true;
    }
    if crate::crs_masks::CONTAINERS.iter().any(|c| values.contains_key(*c)) {
        return true;
    }
    ["RetouchAreas", "RetouchInfo", "RedEyeInfo", "PointColors"].iter().any(|k| props.get(&format!("crs:{k}")).is_some_and(|v| !v.is_empty()))
}

/// Lightroom keeps crop edges, mask and spot positions in the file's stored (unrotated) frame;
/// ours are in the frame as shown. Move them by the photo's orientation `o` (stored → shown).
pub fn reorient_partial(partial: &mut Value, o: lightcraft_geom::Orientation) {
    if o == lightcraft_geom::Orientation::Normal {
        return;
    }
    let swaps = o.swaps_axes();
    let map_pt = |v: &mut Value| {
        if let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) {
            let (nx, ny) = o.map(x, y, 1.0, 1.0);
            *v = json!({"x": nx, "y": ny});
        }
    };
    if let Some(r) = partial.pointer_mut("/crop/geometry/rect")
        && let (Some(x0), Some(y0), Some(x1), Some(y1)) = (r["x0"].as_f64(), r["y0"].as_f64(), r["x1"].as_f64(), r["y1"].as_f64())
    {
        let m = o.map_norm_rect(lightcraft_geom::Rect { x0, y0, x1, y1 });
        *r = json!({"x0": m.x0.min(m.x1), "y0": m.y0.min(m.y1), "x1": m.x0.max(m.x1), "y1": m.y0.max(m.y1)});
    }
    for mask in partial.get_mut("masks").and_then(Value::as_array_mut).into_iter().flatten() {
        for c in mask.get_mut("components").and_then(Value::as_array_mut).into_iter().flatten() {
            let shape = &mut c["shape"];
            match shape["kind"].as_str() {
                Some("linear") => {
                    map_pt(&mut shape["start"]);
                    map_pt(&mut shape["end"]);
                }
                Some("radial") => {
                    map_pt(&mut shape["center"]);
                    // a quarter turn of an ellipse = its radii swapped (an ellipse is symmetric
                    // under a half turn, so the angle's direction convention doesn't matter)
                    if swaps && let Some(o) = shape.as_object_mut() {
                        let (rx, ry) = (o.get("rx").cloned(), o.get("ry").cloned());
                        if let (Some(rx), Some(ry)) = (rx, ry) {
                            o.insert("rx".into(), ry);
                            o.insert("ry".into(), rx);
                        }
                    }
                }
                Some("brush") => {
                    for st in shape["strokes"].as_array_mut().into_iter().flatten() {
                        for pnt in st["points"].as_array_mut().into_iter().flatten() {
                            map_pt(pnt);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for spot in partial.get_mut("spots").and_then(Value::as_array_mut).into_iter().flatten() {
        let first = spot["points"].get(0).and_then(|p| Some((p["x"].as_f64()?, p["y"].as_f64()?)));
        if let (Some((x, y)), Some(dx), Some(dy)) = (first, spot["source_offset"]["x"].as_f64(), spot["source_offset"]["y"].as_f64()) {
            let (ax, ay) = o.map(x, y, 1.0, 1.0);
            let (bx, by) = o.map(x + dx, y + dy, 1.0, 1.0);
            spot["source_offset"] = json!({"x": bx - ax, "y": by - ay});
        }
        for pnt in spot["points"].as_array_mut().into_iter().flatten() {
            map_pt(pnt);
        }
    }
}

/// What a catalog's develop text maps to.
pub struct MappedDevelop {
    /// Partial develop settings (merged like a preset).
    pub partial: Value,
    /// Fields that didn't map.
    pub unmapped: Vec<String>,
    /// The look differs from Lightroom's defaults ([`has_custom_look`]).
    pub custom: bool,
    /// A creative profile (look) we have no counterpart for: its name and amount (0..2).
    pub creative_look: Option<(String, f64)>,
}

/// Map a catalog's develop text (`raw`: the photo develops from raw data; `aspect` = width /
/// height, for radial masks).
pub fn map_develop(text: &str, raw: bool, aspect: f64) -> Result<MappedDevelop, String> {
    let (props, values) = develop_text_props(text)?;
    let custom = has_custom_look(&props, &values);
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { crate::crs_masks::DEFAULT_ASPECT };
    let (partial, unmapped) = crate::crs::to_partial_report(&props, Some(&values), Some(raw), aspect);
    let first = |k: &str| props.get(k).and_then(|v| v.first()).map(|s| s.trim().to_string());
    let creative_look = unmapped.iter().any(|k| k == "Look").then(|| first("crs:Look/crs:Name")).flatten().map(|n| {
        let amount = first("crs:Look/crs:Amount").and_then(|a| a.parse::<f64>().ok()).filter(|a| a.is_finite()).unwrap_or(1.0);
        (n, amount)
    });
    Ok(MappedDevelop { partial, unmapped, custom, creative_look })
}

// ----------------------------------------------------------------------------------- rules

/// A smart collection's rules (Lua) as a smart album's partial filter, or why they can't be.
pub fn smart_rules(text: &str) -> Result<Value, String> {
    let root = parse_lua(text)?;
    rule_set(&root, 0).map(|rs| json!({"ruleSet": rs}))
}

fn rule_set(t: &Lua, depth: usize) -> Result<Value, String> {
    if depth > 8 {
        return Err("rules nested too deeply".into());
    }
    let Lua::Table(items, fields) = t else { return Err("rules are not a table".into()) };
    let combine = t.get("combine").and_then(Lua::str).unwrap_or("intersect");
    let mode = match combine {
        "intersect" => "all",
        "union" => "any",
        "exclude" => "none",
        other => return Err(format!("combine `{other}`")),
    };
    let _ = fields;
    let mut rules = Vec::new();
    for it in items {
        if it.get("criteria").is_some() {
            rules.push(rule(it)?);
        } else {
            rules.push(json!({"group": rule_set(it, depth + 1)?}));
        }
    }
    Ok(json!({"match": mode, "rules": rules}))
}

fn rule(r: &Lua) -> Result<Value, String> {
    let criteria = r.get("criteria").and_then(Lua::str).unwrap_or("");
    let op = r.get("operation").and_then(Lua::str).unwrap_or("");
    let val = r.get("value");
    let s = |v: Option<&Lua>| match v {
        Some(Lua::Str(s)) => s.clone(),
        Some(Lua::Num(n)) => format!("{n}"),
        _ => String::new(),
    };
    let n = |v: Option<&Lua>| match v {
        Some(Lua::Num(n)) => Some(*n),
        Some(Lua::Str(s)) => s.trim().parse().ok(),
        _ => None,
    };
    let cmp = |op: &str| -> Option<&'static str> {
        Some(match op {
            "==" => "is",
            "!=" => "isNot",
            ">=" => "gte",
            "<=" => "lte",
            ">" => "gt",
            "<" => "lt",
            _ => return None,
        })
    };
    let text_op = |op: &str| -> Option<&'static str> {
        Some(match op {
            "any" | "all" | "words" | "contains" => "contains",
            "noneOf" | "notContains" => "notContains",
            "beginsWith" => "startsWith",
            "endsWith" => "endsWith",
            "==" => "is",
            "!=" => "isNot",
            "empty" => "isEmpty",
            "notEmpty" => "isNotEmpty",
            _ => return None,
        })
    };
    let bad = || format!("rule `{criteria} {op}` has no counterpart");
    let text_field = |field: &str| -> Result<Value, String> {
        let o = text_op(op).ok_or_else(bad)?;
        Ok(json!({"field": field, "op": o, "value": s(val)}))
    };
    match criteria {
        "rating" => Ok(json!({"field": "rating", "op": cmp(op).ok_or_else(bad)?, "value": n(val).ok_or_else(bad)?})),
        "pick" => {
            let flag = match n(val).ok_or_else(bad)? {
                v if v > 0.0 => "pick",
                v if v < 0.0 => "reject",
                _ => "none",
            };
            let o = match op {
                "==" => "is",
                "!=" => "isNot",
                _ => return Err(bad()),
            };
            Ok(json!({"field": "flag", "op": o, "value": flag}))
        }
        "labelColor" => {
            let label = match n(val).ok_or_else(bad)? as i64 {
                1 => "red",
                2 => "yellow",
                3 => "green",
                4 => "blue",
                5 => "purple",
                _ => "none",
            };
            let o = match op {
                "==" => "is",
                "!=" => "isNot",
                _ => return Err(bad()),
            };
            Ok(json!({"field": "label", "op": o, "value": label}))
        }
        "keywords" => text_field("keywords"),
        "filename" => text_field("fileName"),
        "folder" => text_field("filePath"),
        "title" => text_field("title"),
        "caption" => text_field("caption"),
        "creator" => text_field("creator"),
        "copyright" => text_field("copyright"),
        "location" | "city" => text_field("location"),
        "camera" | "cameraModel" => text_field("camera"),
        "lens" => text_field("lens"),
        "all" | "allSearchable" => text_field("text"),
        "isoSpeedRating" => Ok(json!({"field": "iso", "op": cmp(op).ok_or_else(bad)?, "value": n(val).ok_or_else(bad)?})),
        "aperture" => Ok(json!({"field": "aperture", "op": cmp(op).ok_or_else(bad)?, "value": n(val).ok_or_else(bad)?})),
        "focalLength" => Ok(json!({"field": "focalLength", "op": cmp(op).ok_or_else(bad)?, "value": n(val).ok_or_else(bad)?})),
        "hasGPSData" | "hasGPS" => Ok(json!({"field": "hasGps", "op": "is", "value": op == "==" && n(val).unwrap_or(1.0) != 0.0 || op == "isTrue"})),
        "hasAdjustments" => Ok(json!({"field": "edited", "op": "is", "value": op == "==" && n(val).unwrap_or(1.0) != 0.0 || op == "isTrue"})),
        "fileFormat" => {
            let f = s(val).to_ascii_uppercase();
            let neg = op == "!=";
            if op != "==" && !neg {
                return Err(bad());
            }
            let o = if neg { "isNot" } else { "is" };
            Ok(match f.as_str() {
                "VIDEO" => json!({"field": "kind", "op": o, "value": "video"}),
                "RAW" => json!({"field": "kind", "op": o, "value": "raw"}),
                _ => json!({"field": "format", "op": o, "value": f}),
            })
        }
        // last changed in Lightroom ≈ last edited here
        "captureTime" | "touchTime" | "importDate" => {
            let field = match criteria {
                "captureTime" => "captureDate",
                "importDate" => "importDate",
                _ => "editDate",
            };
            match op {
                "inLast" | "notInLast" => {
                    let unit = match r.get("value_units").and_then(Lua::str).unwrap_or("days") {
                        "hours" => "hours",
                        "weeks" => "weeks",
                        "months" => "months",
                        "years" => "years",
                        _ => "days",
                    };
                    Ok(json!({"field": field, "op": op, "value": {"n": n(val).ok_or_else(bad)?, "unit": unit}}))
                }
                "==" => Ok(json!({"field": field, "op": "is", "value": s(val)})),
                ">" | ">=" => Ok(json!({"field": field, "op": "after", "value": s(val)})),
                "<" | "<=" => Ok(json!({"field": field, "op": "before", "value": s(val)})),
                "in" => Ok(json!({"field": field, "op": "between", "value": [s(val), s(r.get("value2"))]})),
                _ => Err(bad()),
            }
        }
        _ => Err(bad()),
    }
}

// ----------------------------------------------------------------------------------- migrate

#[derive(Default)]
struct DevelopStats {
    applied: usize,
    unedited: usize,
    by_history: usize,
    by_look: usize,
    failed: Vec<Value>,
    unmapped: BTreeMap<String, usize>,
    photos_with_unmapped: usize,
    rotated: usize,
    /// Creative looks used (name → photos), and how many found a profile here.
    looks: BTreeMap<String, usize>,
    looks_matched: usize,
    /// Unedited in Lightroom, but given settings from an XMP sidecar on import: reset to Lightroom's.
    sidecar_reset: usize,
}

impl DevelopStats {
    fn note(&mut self, unmapped: &[String]) {
        if !unmapped.is_empty() {
            self.photos_with_unmapped += 1;
        }
        for k in unmapped {
            *self.unmapped.entry(k.clone()).or_default() += 1;
        }
    }
}

/// The colour label Lightroom's text names (its own colour names or a label set's), ignoring
/// plug-in bookkeeping labels.
fn label_of(s: &Session, text: &str) -> Option<ColorLabel> {
    let t = text.trim();
    if t.is_empty() || t.starts_with("TK#") {
        return None;
    }
    s.catalog.label_from_name(t)
}

/// A file's own EXIF orientation (read from its first bytes).
fn file_orientation(path: &str) -> lightcraft_geom::Orientation {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(512 << 10).read_to_end(&mut buf);
    }
    lightcraft_meta::extract(&buf).orientation.unwrap_or_default()
}

/// The catalog's photo for each file path.
fn photos_by_path(s: &Session) -> HashMap<String, PhotoId> {
    s.catalog
        .photos()
        .filter(|p| p.copy_of.is_none() && !p.local)
        .filter_map(|p| match &p.source {
            Source::File { path } => Some((path.clone(), p.id)),
            _ => None,
        })
        .collect()
}

/// [`Prepared`] for one photo. `photo_aspect`: the shown width / height when Lightroom didn't
/// record the file's size.
pub fn prepare_image(im: &LrImage, photo_aspect: Option<f64>) -> Prepared {
    let exists = Path::new(&im.path).is_file();
    let turns = lr_quarter_turns(&im.orientation).unwrap_or(0);
    let file_turns = if exists && turns != 0 { file_orientation(&im.path).to_parts().1 % 4 } else { 0 };
    let mut out = Prepared { exists, file_turns, ..Default::default() };
    if im.develop.trim().is_empty() {
        return out;
    }
    // Lightroom's positions are fractions of the stored frame
    let aspect = if im.width > 0 && im.height > 0 {
        im.width as f64 / im.height as f64
    } else {
        photo_aspect.map(|a| if turns % 2 == 1 { 1.0 / a } else { a }).unwrap_or(0.0)
    };
    let raw = matches!(im.format.as_str(), "RAW" | "DNG");
    match map_develop(&im.develop, raw, aspect) {
        Ok(MappedDevelop { mut partial, unmapped, custom, creative_look }) => {
            reorient_partial(&mut partial, lightcraft_geom::Orientation::from_parts(false, turns));
            out.develop = Some(partial);
            out.unmapped = unmapped;
            out.custom = custom;
            out.creative_look = creative_look;
        }
        Err(e) => out.error = Some(e),
    }
    out
}

/// Do [`prepare_image`] for the photos a migration with `limit` takes (the first `limit` masters
/// whose files exist, and their virtual copies). Needs no session: the app runs it on a worker
/// thread.
pub fn prepare(rec: &mut Records, limit: Option<usize>) {
    let mut found = 0usize;
    let mut wanted = HashSet::new();
    for im in rec.images.iter_mut().filter(|i| i.master.is_none()) {
        if limit.is_some_and(|l| found >= l) {
            break;
        }
        let p = im.prepared.take().unwrap_or_else(|| prepare_image(im, None));
        if p.exists {
            found += 1;
            wanted.insert(im.id);
        }
        im.prepared = Some(p);
    }
    for im in rec.images.iter_mut().filter(|i| i.master.is_some_and(|m| wanted.contains(&m))) {
        let p = im.prepared.take().unwrap_or_else(|| prepare_image(im, None));
        im.prepared = Some(p);
    }
}

/// The masters' files a migration with `limit` imports (after [`prepare`]).
pub fn files_to_import(rec: &Records, limit: Option<usize>) -> Vec<String> {
    rec.images
        .iter()
        .filter(|i| i.master.is_none() && i.prepared.as_ref().is_some_and(|p| p.exists))
        .take(limit.unwrap_or(usize::MAX))
        .map(|i| i.path.clone())
        .collect()
}

/// Migrate `rec` into the session. See the module docs.
pub fn migrate(s: &mut Session, rec: &Records, opts: &MigrateOptions) -> crate::Result<Value> {
    let undo0 = s.undo.len();
    let now = (s.clock)();
    // ---- which photos: masters whose file is there (missing volumes are reported by root)
    let mut missing_roots: BTreeMap<String, usize> = BTreeMap::new();
    let mut masters: Vec<&LrImage> = Vec::new();
    let mut missing = 0usize;
    for im in rec.images.iter().filter(|i| i.master.is_none()) {
        if opts.limit.is_some_and(|l| masters.len() >= l) {
            break;
        }
        let exists = im.prepared.as_ref().map_or_else(|| Path::new(&im.path).is_file(), |p| p.exists);
        if exists {
            masters.push(im);
        } else {
            missing += 1;
            *missing_roots.entry(im.root.clone()).or_default() += 1;
        }
    }
    let wanted: HashSet<i64> = masters.iter().map(|m| m.id).collect();
    let mut copies: Vec<&LrImage> = rec.images.iter().filter(|i| i.master.is_some_and(|m| wanted.contains(&m))).collect();
    let lr_by_id: HashMap<i64, &LrImage> = rec.images.iter().map(|i| (i.id, i)).collect();

    // ---- import (add: the files stay where they are)
    let mut import_report = Value::Null;
    let before: HashSet<PhotoId> = if opts.only_new { s.catalog.photos().map(|p| p.id).collect() } else { HashSet::new() };
    let mut imported_now: HashSet<u64> = HashSet::new();
    if opts.import && !opts.dry_run && !opts.collections_only && !masters.is_empty() {
        let paths: Vec<String> = masters.iter().map(|m| m.path.clone()).collect();
        let r = crate::import::import_with(s, &paths, &crate::import::ImportOptions { mode: crate::import::ImportMode::Add, ..Default::default() })?;
        import_report = json!({"imported": r.imported.len(), "duplicates": r.duplicates.len(), "failed": r.failed.len(), "failedFiles": r.failed.iter().take(50).collect::<Vec<_>>()});
        imported_now = r.imported.iter().copied().collect();
    }
    let by_path = photos_by_path(s);
    // catalog image id → our photo
    let mut ids: HashMap<i64, PhotoId> = masters.iter().filter_map(|m| Some((m.id, *by_path.get(&m.path)?))).collect();
    if opts.only_new {
        ids.retain(|_, id| !before.contains(id));
        masters.retain(|m| ids.contains_key(&m.id));
    }
    let not_in_library = masters.len().saturating_sub(ids.len());
    if opts.only_new {
        copies.retain(|c| c.master.is_some_and(|m| ids.contains_key(&m)));
    }

    // ---- virtual copies (sync-conflict copies identical to their master are left out)
    let mut copies_made = 0usize;
    let mut sync_skipped = 0usize;
    for c in &copies {
        let (Some(m), Some(mid)) = (c.master.and_then(|m| lr_by_id.get(&m)), c.master.and_then(|m| ids.get(&m).copied())) else { continue };
        let same = c.develop == m.develop && c.rating == m.rating && c.pick == m.pick && c.label == m.label && c.keywords == m.keywords;
        if c.sync_duplicate && same {
            sync_skipped += 1;
            continue;
        }
        let name = c.copy_name.clone().unwrap_or_else(|| "Copy 1".into());
        let existing = s.catalog.photos().find(|p| p.copy_of == Some(mid) && p.copy_name.as_deref() == Some(name.as_str())).map(|p| p.id);
        let id = match existing {
            Some(id) => Some(id),
            None if opts.dry_run || opts.collections_only => None,
            None => {
                // (the command wants a selection: the master is about to be it)
                s.selection = crate::Selection::single(mid);
                let r = s.execute("photo.virtualCopy", &json!({"ids": [mid.0], "name": name}))?;
                copies_made += 1;
                r["ids"].get(0).and_then(Value::as_u64).map(PhotoId)
            }
        };
        if let Some(id) = id {
            ids.insert(c.id, id);
        }
    }

    // ---- creative profiles (so photos using them match)
    let (profiles_report, dry_names) =
        if opts.collections_only { (Value::Null, Vec::new()) } else { import_profiles(s, &opts.profile_dirs, opts.dry_run)? };
    let mut looks_used = Map::new();

    // ---- per photo: rating, flag, label, text, keywords, develop
    let mut ops = Vec::new();
    let mut dev = DevelopStats::default();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut touched = Map::new();
    let mut edited_at = Map::new();
    // (collections only: no photo changes)
    let lr_images: Vec<&LrImage> = if opts.collections_only { Vec::new() } else { masters.iter().copied().chain(copies.iter().copied()).collect() };
    for im in &lr_images {
        let target = ids.get(&im.id).and_then(|id| s.catalog.photo(*id)).cloned();
        if let (Some(p), Some(t)) = (&target, &im.touched) {
            touched.insert(p.id.0.to_string(), json!(t));
        }
        // develop (also measured on a dry run): worked out ahead by `prepare`, else now
        let prep = match &im.prepared {
            Some(p) => p.clone(),
            None => prepare_image(im, target.as_ref().filter(|p| p.width > 0 && p.height > 0).map(|p| p.width as f64 / p.height as f64)),
        };
        let mut develop = None;
        if let Some(e) = &prep.error {
            dev.failed.push(json!([im.path, e]));
        }
        if let Some(mut partial) = prep.develop.clone() {
            let mut unmapped = prep.unmapped.clone();
            // a creative look: an imported LUT profile of that name (or `lookMap`'s choice)
            if let Some((name, amount)) = &prep.creative_look {
                *dev.looks.entry(name.clone()).or_default() += 1;
                if let Some(t) = &target {
                    looks_used.insert(t.id.0.to_string(), json!({"name": name, "amount": amount}));
                }
                let id = if opts.dry_run && !dry_names.is_empty() {
                    dry_names.iter().any(|n| n.eq_ignore_ascii_case(name.trim())).then(|| format!("lut:{name}"))
                } else {
                    matching_profile(s, &opts.look_map, name)
                };
                if let Some(id) = id {
                    partial["profile"] = json!({"id": id, "amount": (amount * 100.0).clamp(0.0, 200.0)});
                    unmapped.retain(|k| k != "Look");
                    dev.looks_matched += 1;
                }
            }
            if im.edits > 0 {
                dev.by_history += 1;
            } else if prep.custom {
                dev.by_look += 1;
            }
            // a photo Lightroom shows unedited that this import gave settings from an XMP sidecar
            // next to it (often another file's, or another program's): Lightroom's catalog wins
            let from_sidecar = target.as_ref().is_some_and(|p| imported_now.contains(&p.id.0) && *p.develop != p.camera_defaults());
            if im.edits > 0 || prep.custom || opts.develop_all || from_sidecar {
                dev.note(&unmapped);
                develop = Some(partial);
                if from_sidecar && im.edits == 0 && !prep.custom {
                    dev.sidecar_reset += 1;
                }
            } else {
                dev.unedited += 1;
            }
        }
        let Some(p) = target else { continue };
        let id = p.id;
        if p.rating != im.rating {
            ops.push(Op::SetRating { id, rating: im.rating });
            *counts.entry("rated").or_default() += 1;
        }
        let flag = match im.pick {
            1 => Flag::Pick,
            -1 => Flag::Reject,
            _ => Flag::None,
        };
        if p.flag != flag {
            ops.push(Op::SetFlag { id, flag });
            *counts.entry("flagged").or_default() += 1;
        }
        let label = label_of(s, &im.label);
        if label.is_some() && p.label != label {
            ops.push(Op::SetLabel { id, label });
            *counts.entry("labelled").or_default() += 1;
        }
        let mut m = p.meta.clone();
        for (src, dst) in [
            (&im.caption, &mut m.caption),
            (&im.copyright, &mut m.copyright),
            (&im.creator, &mut m.creator),
            (&im.location, &mut m.location),
            (&im.city, &mut m.city),
            (&im.state, &mut m.state),
            (&im.country, &mut m.country),
        ] {
            if !src.trim().is_empty() {
                *dst = src.clone();
            }
        }
        for k in &im.keywords {
            if !m.keywords.iter().any(|x| x.eq_ignore_ascii_case(k)) {
                m.keywords.push(k.clone());
            }
        }
        if !im.keywords.is_empty() {
            *counts.entry("keyworded").or_default() += 1;
        }
        if m != p.meta {
            ops.push(Op::SetMeta { id, meta: Box::new(m) });
        }
        // Lightroom's orientation is the photo as shown; the file's own EXIF turn is applied when
        // it's decoded, so only the difference is ours to keep (files shown as shot aren't read)
        let turn = lr_quarter_turns(&im.orientation).map(|q| lightcraft_geom::Orientation::from_parts(false, (q + 4 - prep.file_turns % 4) % 4));
        let applied = develop.is_some();
        let mut d = match develop {
            Some(partial) => lightcraft_develop::apply_partial(&p.import_defaults(), &partial, 1.0),
            None => (*p.develop).clone(),
        };
        if let Some(t) = turn {
            if t != lightcraft_geom::Orientation::Normal && t != d.orientation {
                dev.rotated += 1;
            }
            d.orientation = t;
        }
        if applied {
            dev.applied += 1;
            if let Some(at) = &im.last_edit {
                edited_at.insert(id.0.to_string(), json!(at));
            }
        }
        if d != *p.develop {
            let at = if applied { Some(im.last_edit.clone().unwrap_or_else(|| now.clone())) } else { p.edited.clone() };
            ops.push(Op::SetDevelop { id, settings: Arc::new(d), label: "Lightroom Classic".into(), edited: at });
        }
    }

    // ---- collections → albums under "From Lightroom" (sets → folders; smart where rules map;
    // the Quick Collection → ours)
    let cols = if opts.dry_run { plan_collections(&rec.collections) } else { migrate_collections(s, rec, &ids, &lr_by_id)? };
    let album_ops = cols.ops;
    ops.extend(album_ops);
    let changes = ops.len();
    if !opts.dry_run && !ops.is_empty() {
        s.commit("Migrate from Lightroom Classic", Op::Batch { ops })?;
    }

    // ---- presets and keyword sets
    let presets =
        if opts.preset_dirs.is_empty() || opts.collections_only { Value::Null } else { import_presets(s, &opts.preset_dirs, opts.dry_run)? };

    // ---- Lightroom's times, for library.resumePoint
    if !opts.dry_run
        && !opts.collections_only
        && let Some(dir) = s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.clone())
    {
        let path = dir.join(RESUME_FILE);
        let mut doc: Value = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
        if !doc.is_object() {
            doc = json!({});
        }
        doc["catalog"] = json!(rec.catalog);
        doc["migrated"] = json!(now);
        for (k, src) in [("touched", touched), ("edited", edited_at), ("looks", looks_used)] {
            if !doc[k].is_object() {
                doc[k] = json!({});
            }
            if let Some(o) = doc[k].as_object_mut() {
                o.extend(src);
            }
        }
        let tmp = dir.join(format!("{RESUME_FILE}.tmp"));
        let bytes = serde_json::to_vec(&doc).unwrap_or_default();
        if std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
            log::warn!("lightroom migration: could not write {}", path.display());
        }
    }
    let steps = s.undo.len().saturating_sub(undo0);
    s.merge_undo(steps, "Migrate from Lightroom Classic");

    let mut unmapped: Vec<(String, usize)> = dev.unmapped.into_iter().collect();
    unmapped.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(json!({
        "catalog": rec.catalog,
        "dryRun": opts.dry_run,
        "images": rec.images.len(),
        "masters": rec.images.iter().filter(|i| i.master.is_none()).count(),
        "found": masters.len(),
        "missing": missing,
        "missingByRoot": missing_roots,
        "import": import_report,
        "notInLibrary": not_in_library,
        "matched": ids.len(),
        "virtualCopies": {"made": copies_made, "syncDuplicatesSkipped": sync_skipped, "total": copies.len()},
        "rated": counts.get("rated").copied().unwrap_or(0),
        "flagged": counts.get("flagged").copied().unwrap_or(0),
        "labelled": counts.get("labelled").copied().unwrap_or(0),
        "keyworded": counts.get("keyworded").copied().unwrap_or(0),
        "develop": {
            "applied": dev.applied,
            "editedByHistory": dev.by_history,
            "editedByLook": dev.by_look,
            "untouched": dev.unedited,
            "rotated": dev.rotated,
            "creativeLooks": dev.looks,
            "creativeLooksMatched": dev.looks_matched,
            "sidecarSettingsReplaced": dev.sidecar_reset,
            "failed": dev.failed,
            "photosWithUnmapped": dev.photos_with_unmapped,
            "unmapped": unmapped.into_iter().map(|(k, n)| json!({"key": k, "photos": n})).collect::<Vec<_>>(),
        },
        "albums": cols.report,
        "changes": changes,
        "presets": presets,
        "profiles": profiles_report,
    }))
}

/// What the collections stage did (or, on a dry run, would do).
struct CollectionsDone {
    /// Album photo lists to set, committed with the photos' changes.
    ops: Vec<Op>,
    /// `{albums, smart, sets, quick, skipped: [{name, why}]}`.
    report: Value,
}

/// Whether collection `c` sits at the top of the catalog (no parent, or one that isn't read).
fn top_level(c: &LrCollection, all: &[LrCollection]) -> bool {
    c.parent.is_none_or(|pid| !all.iter().any(|x| x.id == pid))
}

/// A dry run's account of the collections: what would be made, counted from the catalog alone.
fn plan_collections(cols: &[LrCollection]) -> CollectionsDone {
    let mut n = BTreeMap::<&str, usize>::new();
    let mut skipped = Vec::new();
    for c in cols {
        match c.kind.as_str() {
            "set" => *n.entry("sets").or_default() += 1,
            "quick" => *n.entry("quick").or_default() += c.images.len(),
            "smart" => match c.rules.as_deref().map(smart_rules) {
                Some(Ok(_)) => *n.entry("smart").or_default() += 1,
                Some(Err(e)) => skipped.push(json!({"name": c.name, "why": e})),
                None => skipped.push(json!({"name": c.name, "why": "no rules"})),
            },
            _ => *n.entry("albums").or_default() += 1,
        }
    }
    let get = |k: &str| n.get(k).copied().unwrap_or(0);
    CollectionsDone {
        ops: Vec::new(),
        report: json!({"albums": get("albums"), "smart": get("smart"), "sets": get("sets"), "quick": get("quick"), "skipped": skipped}),
    }
}

/// Make the catalog's collections albums: sets become folders, collections albums (in catalog
/// order), smart collections smart albums where their rules map, and the Quick Collection's
/// photos join ours. Membership goes by catalog image id; a virtual copy left out as an
/// unchanged sync duplicate stands for its master, so a collection of such copies keeps its
/// photos. The tree is the catalog's own: top-level collections and sets are top-level albums
/// and folders, nothing is wrapped. Runs again without making anything twice, and first undoes
/// what earlier versions of the migration did ([`flatten_wrapper`]).
fn migrate_collections(
    s: &mut Session,
    rec: &Records,
    ids: &HashMap<i64, PhotoId>,
    lr_by_id: &HashMap<i64, &LrImage>,
) -> crate::Result<CollectionsDone> {
    let mut done = CollectionsDone { ops: Vec::new(), report: json!({"albums": 0, "smart": 0, "sets": 0, "quick": 0, "skipped": []}) };
    if rec.collections.is_empty() {
        return Ok(done);
    }
    let member = |i: &i64| ids.get(i).copied().or_else(|| lr_by_id.get(i).and_then(|im| im.master).and_then(|m| ids.get(&m).copied()));
    let (mut albums_made, mut smart_made, mut sets_made, mut quick_added) = (0usize, 0usize, 0usize, 0usize);
    let mut skipped = Vec::new();
    // the catalog's own top-level "From Lightroom" set (if it has one) is not our wrapper
    let own_set =
        rec.collections.iter().any(|c| c.kind == "set" && top_level(c, &rec.collections) && c.name.trim().eq_ignore_ascii_case(ALBUM_FOLDER));
    flatten_wrapper(s, !own_set)?;
    let mut made: HashMap<i64, AlbumId> = HashMap::new();
    // parents first (bounded: a parent cycle can't loop)
    let mut todo: Vec<&LrCollection> = rec.collections.iter().collect();
    for _ in 0..16 {
        let mut later = Vec::new();
        for c in todo {
            let parent = if top_level(c, &rec.collections) { Some(None) } else { c.parent.and_then(|pid| made.get(&pid).copied()).map(Some) };
            let Some(parent) = parent else {
                later.push(c);
                continue;
            };
            let name = if c.name.trim().is_empty() { "Untitled Collection".to_string() } else { c.name.clone() };
            match c.kind.as_str() {
                "set" => {
                    made.insert(c.id, find_or_make_album(s, &name, parent, true)?);
                    sets_made += 1;
                }
                "quick" => {
                    let id = match s.catalog.quick_collection() {
                        Some(id) => id,
                        None => {
                            let id = s.catalog.alloc_album_id();
                            s.commit("New Quick Collection", Op::AddAlbum { album: Album { quick: true, ..Album::new(id, "Quick Collection") } })?;
                            id
                        }
                    };
                    let mut photos = s.catalog.album(id).map(|a| a.photos.clone()).unwrap_or_default();
                    for pid in c.images.iter().filter_map(member) {
                        if !photos.contains(&pid) {
                            photos.push(pid);
                            quick_added += 1;
                        }
                    }
                    if s.catalog.album(id).is_some_and(|a| a.photos != photos) {
                        done.ops.push(Op::SetAlbumPhotos { id, photos });
                    }
                }
                "smart" => match c.rules.as_deref().map(smart_rules) {
                    Some(Ok(rules)) => {
                        if !s.catalog.albums().any(|a| a.parent == parent && a.name == name && a.is_smart()) {
                            s.execute("album.createSmart", &json!({"name": name, "rules": rules, "parent": parent.map(|p| p.0)}))?;
                        }
                        smart_made += 1;
                    }
                    Some(Err(e)) => skipped.push(json!({"name": name, "why": e})),
                    None => skipped.push(json!({"name": name, "why": "no rules"})),
                },
                _ => {
                    let id = find_or_make_album(s, &name, parent, false)?;
                    albums_made += 1;
                    let mut photos = s.catalog.album(id).map(|a| a.photos.clone()).unwrap_or_default();
                    for pid in c.images.iter().filter_map(member) {
                        if !photos.contains(&pid) {
                            photos.push(pid);
                        }
                    }
                    if s.catalog.album(id).is_some_and(|a| a.photos != photos) {
                        let cover = s.catalog.album(id).and_then(|a| a.cover).or(photos.first().copied());
                        done.ops.push(Op::SetAlbumPhotos { id, photos });
                        done.ops.push(Op::SetAlbumCover { id, cover });
                    }
                    made.insert(c.id, id);
                }
            }
        }
        if later.is_empty() {
            break;
        }
        todo = later;
    }
    done.report = json!({"albums": albums_made, "smart": smart_made, "sets": sets_made, "quick": quick_added, "skipped": skipped});
    Ok(done)
}

/// Undo what earlier versions of the migration did in a library: every collection went into a
/// top-level "From Lightroom" album folder (so a catalog that had its own "From Lightroom" set
/// showed From Lightroom ▸ From Lightroom ▸ …), and the Quick Collection came over as an album
/// called "quick collection". The wrapper's contents move to the top level (an album whose
/// twin is already there gives it its photos) and the empty wrapper goes; the "quick
/// collection" album's photos join the Quick Collection. One undo step.
///
/// The wrapper is the top-level "From Lightroom" folder that holds another "From Lightroom"
/// folder, or (`any`) any top-level "From Lightroom" folder — the migration passes `any` when
/// the catalog has no such set of its own. Returns `{flattened, moved, merged, quick}`.
pub fn flatten_wrapper(s: &mut Session, any: bool) -> crate::Result<Value> {
    let named = |a: &Album| a.folder && a.name.trim().eq_ignore_ascii_case(ALBUM_FOLDER);
    let wrapper = s
        .catalog
        .albums()
        .filter(|w| w.parent.is_none() && named(w))
        .find(|w| any || s.catalog.albums().any(|k| k.parent == Some(w.id) && named(k)))
        .map(|w| w.id);
    let old_quick: Vec<Album> = s
        .catalog
        .albums()
        .filter(|a| (a.parent.is_none() || a.parent == wrapper) && !a.folder && !a.quick && !a.is_smart())
        .filter(|a| a.name.trim().eq_ignore_ascii_case("quick collection"))
        .cloned()
        .collect();
    let (mut moved, mut merged) = (0usize, 0usize);
    let mut ops = Vec::new();
    if !old_quick.is_empty() {
        let quick = match s.catalog.quick_collection() {
            Some(id) => id,
            None => {
                let id = s.catalog.alloc_album_id();
                s.commit("New Quick Collection", Op::AddAlbum { album: Album { quick: true, ..Album::new(id, "Quick Collection") } })?;
                id
            }
        };
        let mut photos = s.catalog.album(quick).map(|a| a.photos.clone()).unwrap_or_default();
        for p in old_quick.iter().flat_map(|a| a.photos.iter()) {
            if !photos.contains(p) {
                photos.push(*p);
            }
        }
        ops.push(Op::SetAlbumPhotos { id: quick, photos });
        ops.extend(old_quick.iter().map(|a| Op::RemoveAlbum { id: a.id }));
    }
    if let Some(w) = wrapper {
        let kids: Vec<Album> = s.catalog.albums().filter(|a| a.parent == Some(w) && !old_quick.iter().any(|q| q.id == a.id)).cloned().collect();
        for k in kids {
            let twin = s
                .catalog
                .albums()
                .find(|a| a.parent.is_none() && a.id != w && a.name == k.name && a.folder == k.folder && a.is_smart() == k.is_smart() && !a.quick)
                .map(|a| a.id);
            match twin {
                Some(t) if !k.folder && !k.is_smart() => {
                    let mut photos = s.catalog.album(t).map(|a| a.photos.clone()).unwrap_or_default();
                    photos.extend(k.photos.iter().copied().filter(|p| !photos.contains(p)).collect::<Vec<_>>());
                    ops.push(Op::SetAlbumPhotos { id: t, photos });
                    ops.push(Op::RemoveAlbum { id: k.id });
                    merged += 1;
                }
                Some(_) if k.is_smart() => {
                    ops.push(Op::RemoveAlbum { id: k.id });
                    merged += 1;
                }
                _ => {
                    ops.push(Op::MoveAlbum { id: k.id, parent: None });
                    moved += 1;
                }
            }
        }
        ops.push(Op::RemoveAlbum { id: w });
    }
    if !ops.is_empty() {
        s.commit("Flatten Lightroom Collections", Op::Batch { ops })?;
    }
    Ok(json!({"flattened": wrapper.is_some(), "moved": moved, "merged": merged, "quick": old_quick.len()}))
}

/// An album (or folder) named `name` under `parent`, made if there is none.
fn find_or_make_album(s: &mut Session, name: &str, parent: Option<AlbumId>, folder: bool) -> crate::Result<AlbumId> {
    if let Some(a) = s.catalog.albums().find(|a| a.parent == parent && a.name == name && a.folder == folder && !a.is_smart() && !a.quick) {
        return Ok(a.id);
    }
    let id = s.catalog.alloc_album_id();
    s.commit(if folder { "New Folder" } else { "New Album" }, Op::AddAlbum { album: Album { parent, folder, ..Album::new(id, name.to_string()) } })?;
    Ok(id)
}

/// Import every preset in `dirs` (folders give their names as groups); a preset already here
/// (same id — Lightroom's UUID — or same name and group) is skipped. Keyword sets become keyword
/// sets.
fn import_presets(s: &mut Session, dirs: &[String], dry: bool) -> crate::Result<Value> {
    let mut imported = 0usize;
    let mut profiles = 0usize;
    let mut skipped = 0usize;
    let mut failed = Vec::new();
    let mut unmapped: BTreeMap<String, usize> = BTreeMap::new();
    let mut keyword_sets = Vec::new();
    let mut per_dir = Map::new();
    let mut seen: HashSet<String> = s.presets.iter().map(|p| p.id.clone()).collect();
    let mut seen_names: HashSet<(String, String)> = s.presets.iter().map(|p| (p.group.to_lowercase(), p.name.to_lowercase())).collect();
    let current_set = s.keyword_set.clone();
    for top in dirs {
        let mut n_dir = 0usize;
        let base = Path::new(top).parent().unwrap_or(Path::new(""));
        for f in crate::presets::expand_preset_paths(std::slice::from_ref(top)) {
            let bytes = match std::fs::read(&f) {
                Ok(b) => b,
                Err(e) => {
                    failed.push(json!([f, e.to_string()]));
                    continue;
                }
            };
            // a keyword set (`type = "KeywordSet"`)
            if f.to_ascii_lowercase().ends_with(".lrtemplate")
                && let Some((name, words)) = keyword_set(&String::from_utf8_lossy(&bytes))
            {
                if !dry {
                    s.execute("keyword.saveSet", &json!({"name": name, "keywords": words}))?;
                }
                keyword_sets.push(name);
                continue;
            }
            let rel = Path::new(&f).parent().and_then(|d| d.strip_prefix(base).ok()).map(|d| d.to_string_lossy().to_string());
            let dir_group = rel.and_then(|d| crate::preset_import::group_from_dir(&d));
            let look =
                String::from_utf8_lossy(&bytes).contains("PresetType=\"Look\"") || String::from_utf8_lossy(&bytes).contains("<crs:PresetType>Look<");
            match crate::preset_import::read_presets(&f, &bytes, dir_group) {
                Ok(list) => {
                    for it in list {
                        let key = (it.preset.group.to_lowercase(), it.preset.name.to_lowercase());
                        if seen.contains(&it.preset.id) || seen_names.contains(&key) {
                            skipped += 1;
                            continue;
                        }
                        for k in &it.unmapped {
                            *unmapped.entry(k.clone()).or_default() += 1;
                        }
                        seen.insert(it.preset.id.clone());
                        seen_names.insert(key);
                        if !dry {
                            s.add_presets(vec![it.preset]);
                        }
                        imported += 1;
                        n_dir += 1;
                        if look {
                            profiles += 1;
                        }
                    }
                }
                Err(e) => failed.push(json!([f, e])),
            }
        }
        per_dir.insert(top.clone(), json!(n_dir));
    }
    if !dry && current_set != s.keyword_set {
        s.keyword_set = current_set;
        let _ = s.save_prefs();
    }
    let mut unmapped: Vec<(String, usize)> = unmapped.into_iter().collect();
    unmapped.sort_by_key(|u| std::cmp::Reverse(u.1));
    Ok(json!({
        "imported": imported,
        "profiles": profiles,
        "skippedDuplicates": skipped,
        "failed": failed,
        "perFolder": per_dir,
        "keywordSets": keyword_sets,
        "unmapped": unmapped.into_iter().map(|(k, n)| json!({"key": k, "presets": n})).collect::<Vec<_>>(),
    }))
}

/// A Lightroom keyword set template → (name, its nine keywords).
pub fn keyword_set(text: &str) -> Option<(String, Vec<String>)> {
    let root = parse_lua(text.trim_start_matches('\u{feff}')).ok()?;
    if root.get("type").and_then(Lua::str) != Some("KeywordSet") {
        return None;
    }
    let name = crate::preset_import::delocalize(root.get("title").or_else(|| root.get("internalName")).and_then(Lua::str)?).trim().to_string();
    let value = root.get("value")?;
    let words = (1..=9)
        .filter_map(|i| value.get(&format!("shortcut{i}title")).and_then(Lua::str))
        .map(|w| crate::preset_import::delocalize(w).trim().to_string())
        .filter(|w| !w.is_empty())
        .collect();
    (!name.is_empty()).then_some((name, words))
}

// ------------------------------------------------------------------------------ resume point

/// Where to pick up in a folder or album: the photo last edited (here, or in Lightroom before the
/// migration), else the one last touched in Lightroom.
pub fn resume_point(s: &Session, folder: Option<&str>, album: Option<AlbumId>, subfolders: bool) -> Value {
    let in_scope: Box<dyn Fn(&lightcraft_catalog::Photo) -> bool> = match (folder, album) {
        (_, Some(a)) => {
            let members: HashSet<PhotoId> = s.catalog.album_photos(a).into_iter().collect();
            Box::new(move |p| members.contains(&p.id))
        }
        (Some(f), None) => {
            let f = f.trim_end_matches(['/', '\\']).to_string();
            Box::new(move |p| match &p.source {
                Source::File { path } => {
                    let Some(rest) = path.strip_prefix(&f).and_then(|r| r.strip_prefix(['/', '\\'])) else { return false };
                    subfolders || !rest.contains(['/', '\\'])
                }
                _ => false,
            })
        }
        (None, None) => Box::new(|_| true),
    };
    // Lightroom's own times, kept by the migration
    let lr: Value = s
        .library
        .as_ref()
        .filter(|l| l.on_disk)
        .and_then(|l| std::fs::read(l.dir.join(RESUME_FILE)).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let lr_edited = |id: PhotoId| lr["edited"].get(id.0.to_string()).and_then(Value::as_str).map(str::to_string);
    let lr_touched = |id: PhotoId| lr["touched"].get(id.0.to_string()).and_then(Value::as_str).map(str::to_string);
    let mut best: Option<(String, PhotoId, &str)> = None;
    for p in s.catalog.photos().filter(|p| !p.deleted && !p.local && in_scope(p)) {
        let mut cands: Vec<(String, &str)> = Vec::new();
        let lr_e = lr_edited(p.id);
        if let Some(e) = p.edited.as_ref().filter(|e| lr_e.as_ref() != Some(*e)) {
            cands.push((e.clone(), "edit"));
        }
        if let Some(e) = lr_e {
            cands.push((e, "lightroomEdit"));
        }
        if let Some(t) = lr_touched(p.id) {
            cands.push((t, "lightroomTouch"));
        }
        for (t, src) in cands {
            if best.as_ref().is_none_or(|b| t > b.0) {
                best = Some((t, p.id, src));
            }
        }
    }
    match best {
        Some((at, id, source)) => json!({"photoId": id.0, "at": at, "source": source}),
        None => Value::Null,
    }
}

#[cfg(test)]
#[path = "tests_lr_migrate.rs"]
mod tests;
