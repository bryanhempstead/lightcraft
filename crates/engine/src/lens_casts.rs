//! Lens and device colour casts, learned from the user's own edits (`develop.buildLensCasts`).
//!
//! Even on one body, lenses render slightly different colour (coatings, glass), and the user
//! corrects for it. At events where he shot one camera with two or more lenses, each lens's
//! median finished white balance (mireds, tint) minus the event's median is that lens's cast as he
//! corrects it; the table keeps each lens's median over all such events, with how many events and
//! photos back it and how far they spread. Phones' rendered files (JPEG / HEIC) get the same
//! between their own lenses (in the rendered files' relative scale), plus, at events shot with a
//! phone and a camera, the phone's median Saturation / Vibrance relative to the camera's.
//!
//! The table is derived from the user's catalog, so it stays on this machine:
//! `<config>/lens-casts.json` (`~/Library/Application Support/LightCraft/` on macOS;
//! `LIGHTCRAFT_LENS_CASTS` names another file). Never committed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lightcraft_catalog::Photo;
use lightcraft_develop::{DevelopSettings, WbMode};
use lightcraft_pipeline::SourceInfo;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// One photo's finished colour, as the table builder reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub camera: String,
    pub lens: String,
    /// The event (camera + day): `YYYY-MM-DD`.
    pub day: String,
    /// A rendered file (JPEG / HEIC…): `mired` is relative to its own rendering.
    pub rendered: bool,
    /// Finished white balance: 10⁶ / Kelvin for raws; for rendered files the relative scale's
    /// mired offset (−0.8 per Lightroom `IncrementalTemperature` unit).
    pub mired: f64,
    pub tint: f64,
    pub saturation: f64,
    pub vibrance: f64,
}

/// A lens's (or device's) cast as the user corrects it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Cast {
    /// Median finished white balance relative to the event's other lenses (mireds; + = he
    /// renders it cooler), and tint.
    pub mired: f64,
    pub tint: f64,
    /// Phones (rendered files) only: median Saturation / Vibrance relative to a camera's shots
    /// at the same events.
    pub saturation: f64,
    pub vibrance: f64,
    pub sessions: usize,
    pub photos: usize,
    /// Half the interquartile range of the per-event values (mireds, tint).
    pub spread_mired: f64,
    pub spread_tint: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LensCasts {
    /// Where it was learned from, and when.
    pub source: String,
    pub built: String,
    /// camera → lens → cast.
    pub lenses: BTreeMap<String, BTreeMap<String, Cast>>,
    /// Phones (rendered files): device → cast vs cameras at the same events.
    pub devices: BTreeMap<String, Cast>,
}

/// Evidence a cast needs to be kept.
#[derive(Clone, Copy, Debug)]
pub struct Evidence {
    pub min_sessions: usize,
    pub min_photos: usize,
}

impl Default for Evidence {
    fn default() -> Self {
        Self { min_sessions: 3, min_photos: 10 }
    }
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

fn half_iqr(v: &mut [f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    (q(0.75) - q(0.25)) / 2.0
}

fn r1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Build the table from `rows`.
pub fn build(rows: &[Row], ev: Evidence, source: &str) -> LensCasts {
    let ok = |r: &&Row| r.mired.is_finite() && r.tint.is_finite() && r.saturation.is_finite() && r.vibrance.is_finite();
    // ---- lenses: per camera, per event
    let mut events: BTreeMap<(&str, &str), BTreeMap<&str, Vec<&Row>>> = BTreeMap::new();
    for r in rows.iter().filter(ok).filter(|r| !r.lens.trim().is_empty() && !r.day.is_empty()) {
        events.entry((r.camera.as_str(), r.day.as_str())).or_default().entry(r.lens.as_str()).or_default().push(r);
    }
    let mut per: BTreeMap<(&str, &str), (Vec<f64>, Vec<f64>, usize)> = BTreeMap::new();
    for ((camera, _), lenses) in &events {
        let used: Vec<(&&str, &Vec<&Row>)> = lenses.iter().filter(|(_, v)| v.len() >= 2).collect();
        if used.len() < 2 {
            continue;
        }
        let mut all_m: Vec<f64> = used.iter().flat_map(|(_, v)| v.iter().map(|r| r.mired)).collect();
        let mut all_t: Vec<f64> = used.iter().flat_map(|(_, v)| v.iter().map(|r| r.tint)).collect();
        let (em, et) = (median(&mut all_m), median(&mut all_t));
        for (lens, v) in used {
            let mut m: Vec<f64> = v.iter().map(|r| r.mired).collect();
            let mut t: Vec<f64> = v.iter().map(|r| r.tint).collect();
            let e = per.entry((camera, lens)).or_default();
            e.0.push(median(&mut m) - em);
            e.1.push(median(&mut t) - et);
            e.2 += v.len();
        }
    }
    let mut lenses: BTreeMap<String, BTreeMap<String, Cast>> = BTreeMap::new();
    for ((camera, lens), (mut m, mut t, photos)) in per {
        if m.len() < ev.min_sessions || photos < ev.min_photos {
            continue;
        }
        let cast = Cast {
            mired: r1(median(&mut m)),
            tint: r1(median(&mut t)),
            sessions: m.len(),
            photos,
            spread_mired: r1(half_iqr(&mut m)),
            spread_tint: r1(half_iqr(&mut t)),
            ..Cast::default()
        };
        lenses.entry(camera.to_string()).or_default().insert(lens.to_string(), cast);
    }
    // ---- phones vs cameras at the same events (any day with both)
    let mut days: BTreeMap<&str, (BTreeMap<&str, Vec<&Row>>, Vec<&Row>)> = BTreeMap::new();
    for r in rows.iter().filter(ok).filter(|r| !r.day.is_empty()) {
        let d = days.entry(r.day.as_str()).or_default();
        if r.rendered {
            d.0.entry(r.camera.as_str()).or_default().push(r);
        } else {
            d.1.push(r);
        }
    }
    let mut dev: BTreeMap<&str, (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>, usize)> = BTreeMap::new();
    for (phones, cams) in days.values() {
        if cams.len() < 2 {
            continue;
        }
        let mut cs: Vec<f64> = cams.iter().map(|r| r.saturation).collect();
        let mut cv: Vec<f64> = cams.iter().map(|r| r.vibrance).collect();
        let (ms, mv) = (median(&mut cs), median(&mut cv));
        for (device, v) in phones.iter().filter(|(_, v)| v.len() >= 2) {
            let col = |f: fn(&Row) -> f64| -> f64 { median(&mut v.iter().map(|r| f(r)).collect::<Vec<_>>()) };
            let e = dev.entry(device).or_default();
            e.0.push(col(|r| r.mired));
            e.1.push(col(|r| r.tint));
            e.2.push(col(|r| r.saturation) - ms);
            e.3.push(col(|r| r.vibrance) - mv);
            e.4 += v.len();
        }
    }
    let devices = dev
        .into_iter()
        .filter(|(_, e)| e.0.len() >= ev.min_sessions.min(2) && e.4 >= ev.min_photos.min(6))
        .map(|(d, (mut m, mut t, mut s, mut v, photos))| {
            let cast = Cast {
                mired: r1(median(&mut m)),
                tint: r1(median(&mut t)),
                saturation: r1(median(&mut s)),
                vibrance: r1(median(&mut v)),
                sessions: m.len(),
                photos,
                spread_mired: r1(half_iqr(&mut m)),
                spread_tint: r1(half_iqr(&mut t)),
            };
            (d.to_string(), cast)
        })
        .collect();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    LensCasts { source: source.to_string(), built: crate::import::civil(now), lenses, devices }
}

// ------------------------------------------------------------------------------- reading edits

/// The value after `key = ` in Lightroom's Lua settings text (whole-word key).
fn lua_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("{key} = ");
    let mut from = 0;
    while let Some(i) = text.get(from..)?.find(&pat) {
        let at = from + i;
        let before = text.get(..at)?.chars().next_back();
        if before.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) {
            let rest = text.get(at + pat.len()..)?;
            let end = rest.find([',', '\n', '}']).unwrap_or(rest.len());
            return rest.get(..end).map(str::trim);
        }
        from = at + pat.len();
    }
    None
}

fn lua_num(text: &str, key: &str) -> Option<f64> {
    lua_value(text, key)?.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn lua_str<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    lua_value(text, key)?.strip_prefix('"')?.strip_suffix('"')
}

const RENDERED_FORMATS: [&str; 5] = ["JPG", "HEIC", "PNG", "TIFF", "PSD"];

/// One catalog photo as a [`Row`]: its camera model, Exif lens (else Lightroom's lens profile),
/// capture time, file format and develop settings text. `None` when the settings say nothing of
/// white balance.
pub fn row_from_lightroom(camera: &str, lens: &str, captured: &str, format: &str, text: &str) -> Option<Row> {
    let camera = camera.trim();
    if camera.is_empty() {
        return None;
    }
    let lens = match lens.trim() {
        l @ ("" | "not selected") => lua_str(text, "LensProfileName").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(l),
        l => l,
    };
    let rendered = RENDERED_FORMATS.iter().any(|f| f.eq_ignore_ascii_case(format.trim()));
    let (mired, tint) = if rendered {
        (-0.8 * lua_num(text, "IncrementalTemperature").unwrap_or(0.0), lua_num(text, "IncrementalTint").unwrap_or(0.0))
    } else {
        let t = lua_num(text, "Temperature").filter(|t| (1500.0..=60000.0).contains(t))?;
        (1e6 / t, lua_num(text, "Tint").unwrap_or(0.0))
    };
    Some(Row {
        camera: camera.to_string(),
        lens: lens.to_string(),
        day: captured.get(..10).unwrap_or("").to_string(),
        rendered,
        mired,
        tint,
        saturation: lua_num(text, "Saturation").unwrap_or(0.0),
        vibrance: lua_num(text, "Vibrance").unwrap_or(0.0),
    })
}

const SQL_ROWS: &str = "SELECT cm.value AS camera, l.value AS lens, i.captureTime AS captured, i.fileFormat AS format, d.text AS text \
    FROM Adobe_images i JOIN AgHarvestedExifMetadata e ON e.image = i.id_local \
    LEFT JOIN AgInternedExifCameraModel cm ON cm.id_local = e.cameraModelRef \
    LEFT JOIN AgInternedExifLens l ON l.id_local = e.lensRef \
    JOIN Adobe_imageDevelopSettings d ON d.image = i.id_local \
    WHERE i.masterImage IS NULL";

/// Every photo of a Lightroom catalog as a [`Row`] (the catalog is copied into `scratch` and
/// read there with `sqlite3`; the original is never opened).
pub fn rows_from_lightroom(lrcat: &Path, scratch: &Path) -> Result<Vec<Row>, String> {
    if !lrcat.is_file() {
        return Err(format!("{}: no such catalog", lrcat.display()));
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = scratch.join(format!("lightcraft-lenscasts-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let db = dir.join("catalog.lrcat");
    let r = (|| {
        std::fs::copy(lrcat, &db).map_err(|e| format!("copying {}: {e}", lrcat.display()))?;
        let wal = PathBuf::from(format!("{}-wal", lrcat.display()));
        if wal.is_file() {
            std::fs::copy(&wal, dir.join("catalog.lrcat-wal")).map_err(|e| format!("copying {}: {e}", wal.display()))?;
        }
        let rows = crate::lr_migrate::query(&db, SQL_ROWS)?;
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Ok(rows
            .iter()
            .filter_map(|v| row_from_lightroom(&s(v, "camera"), &s(v, "lens"), &s(v, "captured"), &s(v, "format"), &s(v, "text")))
            .collect())
    })();
    for f in ["catalog.lrcat", "catalog.lrcat-wal", "catalog.lrcat-shm", "catalog.lrcat-journal"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    let _ = std::fs::remove_dir(&dir);
    r
}

/// The model part of a LightCraft camera name ("LEICA CAMERA AG LEICA M (Typ 262)" → matched
/// against the catalog's "LEICA M (Typ 262)").
fn same_camera(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    !a.is_empty() && !b.is_empty() && (a == b || a.ends_with(&format!(" {b}")) || b.ends_with(&format!(" {a}")))
}

/// Every photo of the open library as a [`Row`].
pub fn rows_from_library(s: &crate::Session) -> Vec<Row> {
    s.catalog
        .photos()
        .filter_map(|p| {
            let d = &p.develop;
            let rendered = !p.develops_raw();
            let (mired, tint) = if rendered {
                if d.wb.mode == WbMode::Custom { (1e6 / d.wb.temp.clamp(2000.0, 50000.0) - 1e6 / 6500.0, d.wb.tint) } else { (0.0, 0.0) }
            } else if d.wb.mode == WbMode::Custom {
                (1e6 / d.wb.temp.clamp(2000.0, 50000.0), d.wb.tint)
            } else {
                let (t, ti) = p.as_shot_wb?;
                (1e6 / t.clamp(2000.0, 50000.0), ti)
            };
            Some(Row {
                camera: p.meta.camera.clone(),
                lens: crate::camera_match::lens_name(p),
                day: p.captured.as_deref().and_then(|c| c.get(..10)).unwrap_or("").to_string(),
                rendered,
                mired,
                tint,
                saturation: d.color.saturation,
                vibrance: d.color.vibrance,
            })
        })
        .collect()
}

// ------------------------------------------------------------------------------- the file

pub fn path() -> Option<PathBuf> {
    std::env::var_os("LIGHTCRAFT_LENS_CASTS").map(PathBuf::from).or_else(|| crate::camera_profiles::config_dir().map(|d| d.join("lens-casts.json")))
}

/// The saved table (`None`: none built, or unreadable — logged).
pub fn load() -> Option<LensCasts> {
    let p = path()?;
    let len = std::fs::metadata(&p).ok()?.len();
    if len > 16 << 20 {
        log::warn!("lens casts: {} is too large", p.display());
        return None;
    }
    let bytes = std::fs::read(&p).ok()?;
    serde_json::from_slice(&bytes).map_err(|e| log::warn!("lens casts: {}: {e}", p.display())).ok()
}

pub fn save(t: &LensCasts) -> Result<PathBuf, String> {
    let p = path().ok_or("no config folder")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(t).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(p)
}

impl LensCasts {
    /// A camera's lens cast (`None`: the camera or lens isn't in the table).
    pub fn lens(&self, camera: &str, lens: &str) -> Option<&Cast> {
        let (_, lenses) = self.lenses.iter().find(|(c, _)| same_camera(c, camera))?;
        lenses.iter().find(|(l, _)| l.trim().eq_ignore_ascii_case(lens.trim())).map(|(_, c)| c)
    }

    pub fn device(&self, camera: &str) -> Option<&Cast> {
        self.devices.iter().find(|(c, _)| same_camera(c, camera)).map(|(_, c)| c)
    }

    /// The table as a short report.
    pub fn summary(&self) -> Value {
        let lenses: Vec<Value> = self
            .lenses
            .iter()
            .flat_map(|(cam, ls)| ls.iter().map(move |(l, c)| json!({"camera": cam, "lens": l, "mired": c.mired, "tint": c.tint, "sessions": c.sessions, "photos": c.photos, "spreadMired": c.spread_mired, "spreadTint": c.spread_tint})))
            .collect();
        let devices: Vec<Value> = self
            .devices
            .iter()
            .map(|(d, c)| json!({"device": d, "mired": c.mired, "tint": c.tint, "saturation": c.saturation, "vibrance": c.vibrance, "sessions": c.sessions, "photos": c.photos}))
            .collect();
        json!({"lenses": lenses, "devices": devices})
    }
}

/// Give `nd` (the target `tp`'s settings, synced from `rp`) the target lens's cast instead of
/// the reference lens's: white balance by the difference of the two casts (only on a Custom
/// white balance: an As Shot one already follows the camera's own reading of that lens), and
/// for a phone's rendered file its Saturation / Vibrance offset. `None`: nothing to do.
pub fn apply_difference(t: &LensCasts, rp: &Photo, tp: &Photo, nd: &mut DevelopSettings, info: &SourceInfo) -> Option<Value> {
    let lens_of = |p: &Photo| crate::camera_match::lens_name(p);
    let cast = |p: &Photo| {
        if p.develops_raw() {
            t.lens(&p.meta.camera, &lens_of(p)).cloned()
        } else {
            t.lens(&p.meta.camera, &lens_of(p)).cloned().or_else(|| t.device(&p.meta.camera).cloned())
        }
    };
    let (cr, ct) = (cast(rp), cast(tp));
    if cr.is_none() && ct.is_none() {
        return None;
    }
    let (cr, ct) = (cr.unwrap_or_default(), ct.unwrap_or_default());
    let (dm, dt) = (ct.mired - cr.mired, ct.tint - cr.tint);
    let mut out = serde_json::Map::new();
    out.insert("reference".into(), json!({"lens": lens_of(rp), "mired": cr.mired, "tint": cr.tint}));
    out.insert("target".into(), json!({"lens": lens_of(tp), "mired": ct.mired, "tint": ct.tint}));
    if dm.abs() >= 1.0 || dt.abs() >= 1.0 {
        if nd.wb.mode == WbMode::Custom || !tp.develops_raw() {
            let (m0, t0) = if nd.wb.mode == WbMode::Custom {
                (1e6 / nd.wb.temp.clamp(2000.0, 50000.0), nd.wb.tint)
            } else {
                (1e6 / info.as_shot_temp.clamp(2000.0, 50000.0), info.as_shot_tint)
            };
            let before = (nd.wb.temp, nd.wb.tint);
            nd.wb.mode = WbMode::Custom;
            nd.wb.temp = ((1e6 / (m0 + dm).clamp(20.0, 500.0)).clamp(2000.0, 50000.0) / 10.0).round() * 10.0;
            nd.wb.tint = (t0 + dt).clamp(-150.0, 150.0).round();
            out.insert("wb".into(), json!({"temp": [before.0.round(), nd.wb.temp], "tint": [before.1.round(), nd.wb.tint]}));
        } else {
            out.insert("wbSkipped".into(), json!("As Shot"));
        }
    }
    if !tp.develops_raw() && (ct.saturation.abs() >= 1.0 || ct.vibrance.abs() >= 1.0) && rp.develops_raw() {
        let (s0, v0) = (nd.color.saturation, nd.color.vibrance);
        nd.color.saturation = (s0 + ct.saturation).clamp(-100.0, 100.0).round();
        nd.color.vibrance = (v0 + ct.vibrance).clamp(-100.0, 100.0).round();
        out.insert("color".into(), json!({"saturation": [s0, nd.color.saturation], "vibrance": [v0, nd.color.vibrance]}));
    }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(camera: &str, lens: &str, day: &str, kelvin: f64, tint: f64) -> Row {
        Row { camera: camera.into(), lens: lens.into(), day: day.into(), mired: 1e6 / kelvin, tint, ..Row::default() }
    }

    #[test]
    fn a_lens_he_renders_cooler_gets_its_cast() {
        // three events with two lenses each; the 21 mm always finished 20 mireds cooler, +8 tint
        let mut rows = Vec::new();
        for (day, k) in [("2024-01-01", 5000.0), ("2024-02-01", 5600.0), ("2024-03-01", 4400.0)] {
            for _ in 0..3 {
                rows.push(row("LEICA M (Typ 262)", "Summicron 35", day, k, 0.0));
                rows.push(row("LEICA M (Typ 262)", "Elmarit 21", day, 1e6 / (1e6 / k + 20.0), 8.0));
            }
        }
        // a day with one lens only tells nothing
        rows.push(row("LEICA M (Typ 262)", "Elmarit 21", "2024-04-01", 3000.0, 50.0));
        rows.push(row("LEICA M (Typ 262)", "Elmarit 21", "2024-04-01", 3000.0, 50.0));
        let t = build(&rows, Evidence { min_sessions: 3, min_photos: 6 }, "test");
        let e = t.lens("LEICA CAMERA AG LEICA M (Typ 262)", "elmarit 21").expect("learned");
        assert!((e.mired - 10.0).abs() < 0.2 && (e.tint - 4.0).abs() < 0.2, "{e:?}");
        assert_eq!((e.sessions, e.photos), (3, 9));
        let s = t.lens("LEICA M (Typ 262)", "Summicron 35").expect("learned");
        assert!((s.mired + 10.0).abs() < 0.2, "{s:?}");
        // too little evidence: dropped
        let strict = build(&rows, Evidence { min_sessions: 4, min_photos: 6 }, "test");
        assert!(strict.lenses.is_empty());
    }

    #[test]
    fn phones_get_their_saturation_against_the_camera_at_the_same_event() {
        let mut rows = Vec::new();
        for day in ["2024-05-01", "2024-05-02", "2024-05-03"] {
            for _ in 0..3 {
                rows.push(Row {
                    camera: "Canon EOS R6".into(),
                    lens: "50".into(),
                    day: day.into(),
                    mired: 200.0,
                    saturation: 10.0,
                    vibrance: 5.0,
                    ..Row::default()
                });
                rows.push(Row {
                    camera: "iPhone 13 Pro Max".into(),
                    lens: "wide".into(),
                    day: day.into(),
                    rendered: true,
                    mired: -8.0,
                    tint: 3.0,
                    saturation: -5.0,
                    vibrance: 5.0,
                });
            }
        }
        let t = build(&rows, Evidence::default(), "test");
        let d = t.device("iPhone 13 Pro Max").expect("learned");
        assert_eq!((d.saturation, d.vibrance, d.mired, d.tint, d.sessions), (-15.0, 0.0, -8.0, 3.0, 3));
    }

    #[test]
    fn lightroom_settings_text_is_read_by_whole_keys() {
        let text = "s = { IncrementalTemperature = 12, Temperature = 5150, Tint = -3, WhiteBalance = \"Custom\",\n Saturation = -10, Vibrance = 15, LensProfileName = \"Voigtlander VM 50mm f1.5 Nokton\" }";
        let r = row_from_lightroom("LEICA M (Typ 262)", "not selected", "2023-06-01T10:00:00", "DNG", text).unwrap();
        assert_eq!((r.lens.as_str(), r.day.as_str(), r.rendered), ("Voigtlander VM 50mm f1.5 Nokton", "2023-06-01", false));
        assert!((r.mired - 1e6 / 5150.0).abs() < 1e-9 && r.tint == -3.0 && r.saturation == -10.0 && r.vibrance == 15.0);
        let j = row_from_lightroom("iPhone 13 Pro Max", "back camera", "2023-06-01", "JPG", text).unwrap();
        assert!(j.rendered && (j.mired + 9.6).abs() < 1e-9);
        // hostile / empty text
        assert!(row_from_lightroom("", "x", "", "DNG", text).is_none());
        assert!(row_from_lightroom("cam", "x", "", "DNG", "Temperature = NaN").is_none());
        assert!(row_from_lightroom("cam", "x", "é", "DNG", "Temperature = 1e400").is_none());
        assert_eq!(lua_value("XTemperature = 4", "Temperature"), None);
        assert_eq!(lua_value("Temperature = ", "Temperature"), Some(""));
        assert!(build(&[], Evidence::default(), "empty").lenses.is_empty());
    }

    #[test]
    fn the_lens_difference_moves_a_custom_white_balance_only() {
        use lightcraft_catalog::{MediaKind, PhotoId, Source};
        let mut t = LensCasts::default();
        let lens = |m: f64, ti: f64| Cast { mired: m, tint: ti, sessions: 5, photos: 50, ..Cast::default() };
        t.lenses.entry("Canon EOS R6".into()).or_default().insert("EF24mm".into(), lens(-2.0, 0.0));
        t.lenses.entry("Canon EOS R6".into()).or_default().insert("15.0 mm".into(), lens(8.0, -2.0));
        let photo = |lens: &str| {
            let mut p = Photo::new(PhotoId(1), Source::File { path: "/nonexistent/x.CR3".into() }, "x.CR3", "CR3", 6000, 4000, "2024-01-01");
            p.kind = MediaKind::Raw;
            p.meta.camera = "Canon EOS R6".into();
            p.meta.lens = lens.into();
            p
        };
        let (rp, tp) = (photo("EF24mm"), photo("15.0 mm"));
        let info = SourceInfo { raw: true, as_shot_temp: 5000.0, ..SourceInfo::default() };
        let mut nd =
            DevelopSettings { wb: lightcraft_develop::WhiteBalance { mode: WbMode::Custom, temp: 5000.0, tint: 4.0 }, ..DevelopSettings::default() };
        let v = apply_difference(&t, &rp, &tp, &mut nd, &info).expect("a difference");
        assert!((nd.wb.temp - (1e6f64 / (200.0 + 10.0) / 10.0).round() * 10.0).abs() < 1e-9, "{v} {:?}", nd.wb);
        assert_eq!(nd.wb.tint, 2.0);
        // As Shot stays as shot
        let mut asshot = DevelopSettings::default();
        let v = apply_difference(&t, &rp, &tp, &mut asshot, &info).expect("reported");
        assert_eq!(asshot.wb, DevelopSettings::default().wb, "{v}");
        // unknown lenses: nothing
        assert!(apply_difference(&t, &photo("a"), &photo("b"), &mut nd, &info).is_none());
    }
}
