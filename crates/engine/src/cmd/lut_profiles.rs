//! Imported profiles: `.cube` 3D LUTs and camera-raw creative ("Look") profiles — XMP files
//! whose look is an RGB colour table ([`crate::crs_table`]) — used as creative profiles
//! (Amount 0–200 % like the built-in ones). A library on disk keeps a copy of each file in its
//! `Profiles/` folder; the list lives in the library's prefs and the LUTs are registered with
//! the pipeline when the library opens.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

/// One imported LUT profile.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LutProfile {
    /// `lut:<slug>`.
    pub id: String,
    pub name: String,
    pub group: String,
    /// The `.cube` or `.xmp` file (the library's copy when there is a library on disk).
    pub file: String,
    /// For an XMP profile: the digest of its colour table (two files with one table are one
    /// profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// The LUT in an imported profile file.
fn load(file: &str) -> std::result::Result<lightcraft_pipeline::lut::Lut3d, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    if file.to_ascii_lowercase().ends_with(".xmp") {
        crate::crs_table::profile_from_xmp(&text)?.map(|p| p.lut).ok_or_else(|| format!("{file}: no colour table"))
    } else {
        lightcraft_pipeline::lut::Lut3d::parse_cube(&text)
    }
}

pub(crate) fn profiles_dir(s: &Session) -> Option<PathBuf> {
    s.library.as_ref().filter(|l| l.on_disk).map(|l| l.dir.join("Profiles"))
}

/// Register every imported profile's LUT with the pipeline (missing files are skipped).
pub fn register_all(s: &Session) {
    for p in &s.lut_profiles {
        match load(&p.file) {
            Ok(l) => lightcraft_pipeline::lut::register(&p.id, l),
            Err(e) => log::warn!("profile {}: {e}", p.name),
        }
    }
}

fn is_profile_file(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    l.ends_with(".cube") || l.ends_with(".xmp")
}

/// `.cube` and `.xmp` files in `paths` (folders searched, zip bundles opened): (name, text,
/// group, is XMP).
fn profile_files(paths: &[String]) -> Vec<(String, String, Option<String>, bool)> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            let mut stack = vec![path.to_path_buf()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten() {
                    let ep = e.path();
                    if ep.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                        continue;
                    }
                    if ep.is_dir() {
                        stack.push(ep);
                    } else if is_profile_file(&ep.to_string_lossy())
                        && let Ok(t) = std::fs::read_to_string(&ep)
                    {
                        let group = ep.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string());
                        let xmp = ep.extension().is_some_and(|x| x.eq_ignore_ascii_case("xmp"));
                        out.push((ep.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), t, group, xmp));
                    }
                }
            }
        } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
            let Ok(b) = std::fs::read(path) else { continue };
            for (inner, data) in crate::preset_import::read_zip(&b).unwrap_or_default() {
                if is_profile_file(&inner) {
                    let ip = Path::new(&inner);
                    let group = ip
                        .parent()
                        .and_then(|d| d.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .or_else(|| path.file_stem().map(|n| n.to_string_lossy().to_string()));
                    out.push((
                        ip.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        String::from_utf8_lossy(&data).to_string(),
                        group,
                        inner.to_ascii_lowercase().ends_with(".xmp"),
                    ));
                }
            }
        } else if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("dcp"))
            && let Ok(t) = std::fs::read_to_string(path)
        {
            let xmp = path.extension().is_some_and(|x| x.eq_ignore_ascii_case("xmp"));
            out.push((path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), t, None, xmp));
        }
    }
    out
}

/// Largest camera profile (`.dcp`) imported.
const MAX_DCP: u64 = 16 << 20;

/// `.dcp` camera profiles in `paths` (folders searched, zips opened): (file name, bytes).
fn dcp_files(paths: &[String]) -> Vec<(String, Vec<u8>)> {
    let is_dcp = |n: &str| n.to_ascii_lowercase().ends_with(".dcp");
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            let mut stack = vec![(path.to_path_buf(), 0usize)];
            while let Some((d, depth)) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                for e in rd.flatten().take(10_000) {
                    let ep = e.path();
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    if ep.is_dir() && depth < 8 {
                        stack.push((ep, depth + 1));
                    } else if is_dcp(&name)
                        && std::fs::metadata(&ep).is_ok_and(|m| m.len() <= MAX_DCP)
                        && let Ok(b) = std::fs::read(&ep)
                    {
                        out.push((name, b));
                    }
                }
            }
        } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("zip")) {
            let Ok(b) = std::fs::read(path) else { continue };
            for (inner, data) in crate::preset_import::read_zip(&b).unwrap_or_default() {
                if is_dcp(&inner) && data.len() as u64 <= MAX_DCP {
                    let name = Path::new(&inner).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(inner);
                    out.push((name, data));
                }
            }
        } else if path.file_name().is_some_and(|n| is_dcp(&n.to_string_lossy()))
            && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_DCP)
            && let Ok(b) = std::fs::read(path)
        {
            out.push((path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), b));
        }
    }
    out
}

/// Import the camera profiles (`.dcp`) in `paths` (Bryan's fork; U09): each is checked with the
/// DNG SDK, copied into the library's `Profiles/` folder (or a temporary folder without a library
/// on disk) and listed for its camera in the profile browser's Profiles group (`cam:<name>`).
fn import_dcps(s: &mut Session, paths: &[String], dir: Option<&Path>, r: &mut ImportReport) -> Result<()> {
    const C: &str = "profile.import";
    let files = dcp_files(paths);
    if files.is_empty() {
        return Ok(());
    }
    let dir = match dir {
        Some(d) => d.to_path_buf(),
        None => std::env::temp_dir().join("lightcraft-profiles"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| bad(C, format!("{}: {e}", dir.display())))?;
    let mut any = false;
    for (name, bytes) in files {
        match crate::adobe::dcp_facts(&bytes) {
            Ok((camera, profile)) if !camera.trim().is_empty() => {
                let profile = if profile.trim().is_empty() {
                    Path::new(&name).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
                } else {
                    profile.trim().to_string()
                };
                let target = dir.join(format!("{}.dcp", crate::presets::slug(&format!("{camera}-{profile}"))));
                if std::fs::read(&target).is_ok_and(|b| b == bytes) {
                    r.known += 1;
                    continue;
                }
                std::fs::write(&target, &bytes).map_err(|e| bad(C, format!("{}: {e}", target.display())))?;
                any = true;
                r.imported.push(json!({"id": format!("cam:{profile}"), "name": profile, "group": crate::adobe::GROUP_IMPORTED, "camera": camera}));
            }
            Ok(_) => r.failed.push(json!([name, "the profile names no camera"])),
            Err(e) => r.failed.push(json!([name, e])),
        }
    }
    if any {
        let mut dirs = profiles_dir(s).into_iter().collect::<Vec<_>>();
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
        crate::adobe::set_imported_dirs(dirs);
    }
    Ok(())
}

/// Names of the profiles in `paths` (as [`import_paths`] would import them), without importing
/// anything: for dry runs.
pub fn profile_names(paths: &[String]) -> Vec<String> {
    profile_files(paths)
        .into_iter()
        .filter_map(|(name, text, _, xmp)| {
            if xmp {
                crate::crs_table::profile_from_xmp(&text).ok().flatten().map(|p| p.name)
            } else {
                let l = lightcraft_pipeline::lut::Lut3d::parse_cube(&text).ok()?;
                Some(l.title.filter(|t| !t.trim().is_empty()).unwrap_or(name))
            }
        })
        .collect()
}

/// What [`import_paths`] did.
#[derive(Default)]
pub struct ImportReport {
    pub imported: Vec<Value>,
    pub failed: Vec<Value>,
    /// XMP files that are presets or profiles without a colour table (nothing to import).
    pub skipped: usize,
    /// Profiles already in the library (same colour table, or same name and group).
    pub known: usize,
}

/// Import the profiles in `paths` (`.cube` files, XMP creative profiles, presets carrying a
/// look's colour table; folders searched, zips opened). A profile already imported (same
/// colour table, or `.cube` of the same name and group) is not imported twice.
pub fn import_paths(s: &mut Session, paths: &[String]) -> Result<ImportReport> {
    const C: &str = "profile.import";
    let dir = profiles_dir(s);
    if let Some(d) = &dir {
        std::fs::create_dir_all(d).map_err(|e| bad(C, format!("{}: {e}", d.display())))?;
    }
    let mut r = ImportReport::default();
    import_dcps(s, paths, dir.as_deref(), &mut r)?;
    for (name, text, group, xmp) in profile_files(paths) {
        let (lut, display, digest, group) = if xmp {
            match crate::crs_table::profile_from_xmp(&text) {
                Ok(Some(p)) => (p.lut, p.name, Some(p.table_digest), p.group.or(group)),
                Ok(None) => {
                    r.skipped += 1;
                    continue;
                }
                Err(e) => {
                    r.failed.push(json!([name, e]));
                    continue;
                }
            }
        } else {
            match lightcraft_pipeline::lut::Lut3d::parse_cube(&text) {
                Ok(l) => {
                    let display = l.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| name.clone());
                    (l, display, None, group)
                }
                Err(e) => {
                    r.failed.push(json!([name, e]));
                    continue;
                }
            }
        };
        let group = super::super::preset_import::group_from_dir(&group.unwrap_or_default()).unwrap_or_else(|| "Imported".into());
        let same = |x: &LutProfile| match (&digest, &x.digest) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => x.name == display && x.group == group,
        };
        if s.lut_profiles.iter().any(same) {
            r.known += 1;
            continue;
        }
        let slug = crate::presets::slug(&format!("{group}-{display}"));
        let mut id = format!("lut:{slug}");
        let mut n = 2;
        while s.lut_profiles.iter().any(|x| x.id == id) {
            id = format!("lut:{slug}-{n}");
            n += 1;
        }
        let ext = if xmp { "xmp" } else { "cube" };
        let file = match &dir {
            Some(d) => {
                let f = d.join(format!("{}.{ext}", id.trim_start_matches("lut:")));
                std::fs::write(&f, &text).map_err(|e| bad(C, format!("{}: {e}", f.display())))?;
                f.to_string_lossy().to_string()
            }
            None => {
                // no library on disk: remember the original (the in-memory session ends with the app)
                let f = std::env::temp_dir().join(format!("lightcraft-{}.{ext}", id.trim_start_matches("lut:")));
                std::fs::write(&f, &text).map_err(|e| bad(C, e.to_string()))?;
                f.to_string_lossy().to_string()
            }
        };
        lightcraft_pipeline::lut::register(&id, lut);
        s.lut_profiles.push(LutProfile { id: id.clone(), name: display.clone(), group: group.clone(), file, digest });
        r.imported.push(json!({"id": id, "name": display, "group": group}));
    }
    if !r.imported.is_empty() {
        s.save_prefs()?;
    }
    Ok(r)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let paths: Vec<String> =
        p.get("paths").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if paths.is_empty() {
        return Err(bad("profile.import", "no paths"));
    }
    let r = import_paths(s, &paths)?;
    Ok(json!({"imported": r.imported, "failed": r.failed, "alreadyImported": r.known, "skipped": r.skipped}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "profile.import",
            "Import Profiles",
            [],
            None,
            "{paths: [.cube / .xmp / .dcp file, folder or .zip]} — camera profiles (.dcp: listed for their camera in the Profiles group, `cam:<name>`), 3D LUTs and camera-raw creative profiles (XMP Look profiles, or presets carrying one, whose look is an RGB colour table) become creative profiles (Amount 0–200 %; rendered on the CPU or the GPU) in the profile browser's groups (the profile's group, folder / zip name, else Imported); one already imported (same table, or same name and group) is skipped → {imported: [{id, name, group}], failed, alreadyImported, skipped}",
            always,
            import
        ),
        cmd!("profile.deleteImported", "Delete Imported Profile", [], None, "{id: lut:…}", always, |s, p| {
            let id = str_param(p, "id").ok_or_else(|| bad("profile.deleteImported", "missing id"))?.to_string();
            let Some(i) = s.lut_profiles.iter().position(|x| x.id == id) else {
                return Err(bad("profile.deleteImported", format!("no imported profile `{id}`")));
            };
            let gone = s.lut_profiles.remove(i);
            if profiles_dir(s).is_some_and(|d| Path::new(&gone.file).starts_with(d)) {
                let _ = std::fs::remove_file(&gone.file);
            }
            lightcraft_pipeline::lut::unregister(&id);
            s.save_prefs()?;
            Ok(json!({"deleted": id}))
        }),
    ]
}
