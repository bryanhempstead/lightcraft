//! Export presets: built-in ones ([`crate::export::builtin_presets`]: Lightroom's four, then
//! LightCraft's) and the user's, saved with the library in folders (User Presets and folders the
//! user makes), as in Lightroom's Export dialog. `app.export {preset}` (desktop app, MCP,
//! `lightcraft-cli run`) starts from a preset's params; the call's own params override them (see
//! [`Session::export_params`]). A preset keeps its destination folder (`dir`) like Lightroom's.
//! Presets go to and come from files: LightCraft's JSON, and Lightroom's `.lrtemplate` export
//! presets (read only, [`crate::export_lrtemplate`]).

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd};
use crate::export::{ExportPreset, LIGHTCRAFT_PRESETS, LIGHTROOM_PRESETS, USER_PRESETS, builtin_presets};
use crate::{Result, Session};

/// The `format` tag of a presets file written by [`write_presets`].
pub const PRESETS_FILE_FORMAT: &str = "lightcraft-export-presets";

impl Session {
    /// Built-in presets followed by the user's.
    pub fn all_export_presets(&self) -> Vec<(ExportPreset, bool)> {
        builtin_presets().into_iter().map(|p| (p, true)).chain(self.export_presets.iter().cloned().map(|p| (p, false))).collect()
    }

    /// The preset list's folders in order: Lightroom Presets, LightCraft Presets, User Presets,
    /// then the user's folders by name; each with whether it is built in and its presets' names.
    pub fn export_preset_tree(&self) -> Vec<(String, bool, Vec<String>)> {
        let mut out: Vec<(String, bool, Vec<String>)> =
            vec![(LIGHTROOM_PRESETS.into(), true, vec![]), (LIGHTCRAFT_PRESETS.into(), true, vec![]), (USER_PRESETS.into(), false, vec![])];
        let mut folders: Vec<String> = self.export_preset_folders.clone();
        for p in &self.export_presets {
            if !folders.iter().any(|f| f.eq_ignore_ascii_case(p.folder_name())) && !p.folder_name().eq_ignore_ascii_case(USER_PRESETS) {
                folders.push(p.folder_name().to_string());
            }
        }
        folders.sort_by_key(|f| f.to_lowercase());
        out.extend(folders.into_iter().filter(|f| !out.iter().any(|(o, _, _)| o.eq_ignore_ascii_case(f))).map(|f| (f, false, vec![])).collect::<Vec<_>>());
        for (p, _) in self.all_export_presets() {
            if let Some(row) = out.iter_mut().find(|(f, _, _)| f.eq_ignore_ascii_case(p.folder_name())) {
                row.2.push(p.name.clone());
            }
        }
        out
    }

    /// `app.export` params with a named `preset` expanded: the preset's params, overridden by the
    /// call's own (`preset` itself removed), and a `folder` (+ `subfolders`, default true) turned
    /// into the `ids` of the library's photos under it. Unknown preset names are an error.
    pub fn export_params(&self, p: &Value) -> std::result::Result<Value, String> {
        let mut p = self.export_preset_params(p)?;
        // `folder`: the library's photos under it (a shoot), unless `ids` names them
        if let Some(folder) = p.get("folder").and_then(Value::as_str).map(str::to_string)
            && let Some(o) = p.as_object_mut()
        {
            let sub = o.get("subfolders").and_then(Value::as_bool).unwrap_or(true);
            o.remove("folder");
            o.remove("subfolders");
            if !o.contains_key("ids") {
                let ids = super::shoot::photos_under(self, &folder, sub);
                if ids.is_empty() {
                    return Err(format!("no photos in the library under {folder}"));
                }
                o.insert("ids".into(), json!(ids.iter().map(|i| i.0).collect::<Vec<_>>()));
            }
        }
        Ok(p)
    }

    fn export_preset_params(&self, p: &Value) -> std::result::Result<Value, String> {
        let Some(name) = p.get("preset").and_then(Value::as_str) else { return Ok(p.clone()) };
        let (preset, _) = self
            .all_export_presets()
            .into_iter()
            .find(|(x, _)| x.name.eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| format!("unknown export preset `{name}` (see export.presets)"))?;
        // a preset saved by another version may hold keys this one doesn't know: not the caller's typo
        let mut out = crate::export::ExportOptions::known_keys_only(&preset.params);
        if let (Some(o), Some(own)) = (out.as_object_mut(), p.as_object()) {
            for (k, v) in own.iter().filter(|(k, _)| *k != "preset") {
                o.insert(k.clone(), v.clone());
            }
        }
        Ok(out)
    }
}

fn list(s: &Session) -> Value {
    Value::Array(
        s.all_export_presets()
            .into_iter()
            .map(|(p, builtin)| json!({"name": p.name, "builtin": builtin, "folder": p.folder_name(), "params": p.params}))
            .collect(),
    )
}

fn tree(s: &Session) -> Value {
    Value::Array(s.export_preset_tree().into_iter().map(|(name, builtin, presets)| json!({"name": name, "builtin": builtin, "presets": presets})).collect())
}

/// Params that describe a selection or a single run, not the export settings. (`dir` is kept:
/// a preset remembers its destination folder, as Lightroom's do.)
const NOT_SETTINGS: &[&str] = &["ids", "id", "path", "preset", "folder", "subfolders", "background", "resolve", "presets"];

fn is_builtin(name: &str) -> bool {
    builtin_presets().iter().any(|b| b.name.eq_ignore_ascii_case(name))
}

fn is_builtin_folder(name: &str) -> bool {
    [LIGHTROOM_PRESETS, LIGHTCRAFT_PRESETS].iter().any(|f| f.eq_ignore_ascii_case(name.trim()))
}

fn name_param<'a>(id: &str, p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key).and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(id, format!("missing `{key}`")))
}

/// A folder name as given (`User Presets` or empty = none), checked.
fn folder_param(id: &str, p: &Value) -> Result<Option<String>> {
    let Some(f) = p.get("folder") else { return Ok(None) };
    let f = f.as_str().ok_or_else(|| bad(id, "`folder` must be a string"))?.trim();
    if is_builtin_folder(f) {
        return Err(bad(id, format!("`{f}` holds the built-in presets; choose another folder")));
    }
    if f.chars().any(|c| c.is_control()) || f.chars().count() > 120 {
        return Err(bad(id, "`folder` must be a short name"));
    }
    Ok(Some(if f.eq_ignore_ascii_case(USER_PRESETS) { String::new() } else { f.to_string() }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.savePreset";
    let name = name_param(ID, p, "name")?;
    if is_builtin(name) {
        return Err(bad(ID, format!("`{name}` is a built-in preset; choose another name")));
    }
    let mut params = match p.get("params") {
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err(bad(ID, "`params` must be an object of app.export params")),
        None => s.last_export.clone().ok_or_else(|| bad(ID, "no `params` and no previous export to save"))?,
    };
    if let Some(o) = params.as_object_mut() {
        o.retain(|k, _| !NOT_SETTINGS.contains(&k.as_str()));
        // the folder only matters for a specific folder
        let specific = o.get("exportTo").and_then(Value::as_str).is_none_or(|t| t == "specificFolder");
        if !specific || o.get("sameFolder").and_then(Value::as_bool) == Some(true) || o.get("dir").and_then(Value::as_str).is_some_and(|d| d.trim().is_empty())
        {
            o.remove("dir");
        }
    }
    // a preset that app.export would refuse is refused here, where the typo is on screen
    crate::export::ExportOptions::validate(ID, &params)?;
    let folder = folder_param(ID, p)?;
    match s.export_presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) {
        // Update with Current Settings: the preset stays in its folder unless told otherwise
        Some(x) => {
            x.params = params;
            if let Some(f) = folder {
                x.folder = f;
            }
        }
        None => s.export_presets.push(ExportPreset { name: name.to_string(), params, folder: folder.unwrap_or_default() }),
    }
    s.save_prefs()?;
    Ok(list(s))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.deletePreset";
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad(ID, "missing `name`"))?;
    let before = s.export_presets.len();
    s.export_presets.retain(|x| !x.name.eq_ignore_ascii_case(name.trim()));
    if s.export_presets.len() == before {
        return Err(bad(ID, format!("no user export preset `{name}`")));
    }
    s.save_prefs()?;
    Ok(list(s))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.renamePreset";
    let (name, to) = (name_param(ID, p, "name")?, name_param(ID, p, "to")?);
    if is_builtin(name) {
        return Err(bad(ID, format!("`{name}` is a built-in preset and keeps its name")));
    }
    if is_builtin(to) || s.export_presets.iter().any(|x| x.name.eq_ignore_ascii_case(to) && !x.name.eq_ignore_ascii_case(name)) {
        return Err(bad(ID, format!("there is a preset called `{to}` already")));
    }
    let x = s.export_presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(ID, format!("no user export preset `{name}`")))?;
    x.name = to.to_string();
    s.save_prefs()?;
    Ok(list(s))
}

fn move_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.movePreset";
    let name = name_param(ID, p, "name")?;
    let folder = folder_param(ID, p)?.ok_or_else(|| bad(ID, "missing `folder`"))?;
    let x = s.export_presets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(ID, format!("no user export preset `{name}`")))?;
    x.folder = folder;
    s.save_prefs()?;
    Ok(tree(s))
}

fn create_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.createPresetFolder";
    let name = name_param(ID, p, "name")?;
    let Some(f) = folder_param(ID, &json!({"folder": name}))?.filter(|f| !f.is_empty()) else {
        return Err(bad(ID, format!("`{name}` is there already")));
    };
    if s.export_preset_tree().iter().any(|(x, _, _)| x.eq_ignore_ascii_case(&f)) {
        return Err(bad(ID, format!("`{f}` is there already")));
    }
    s.export_preset_folders.push(f);
    s.save_prefs()?;
    Ok(tree(s))
}

/// Remove a user folder; its presets move to User Presets (nothing is deleted with it).
fn delete_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.deletePresetFolder";
    let name = name_param(ID, p, "name")?;
    if is_builtin_folder(name) || name.eq_ignore_ascii_case(USER_PRESETS) {
        return Err(bad(ID, format!("`{name}` can't be removed")));
    }
    let exists = s.export_preset_folders.iter().any(|f| f.eq_ignore_ascii_case(name)) || s.export_presets.iter().any(|x| x.folder.eq_ignore_ascii_case(name));
    if !exists {
        return Err(bad(ID, format!("no preset folder `{name}`")));
    }
    s.export_preset_folders.retain(|f| !f.eq_ignore_ascii_case(name));
    for x in s.export_presets.iter_mut().filter(|x| x.folder.eq_ignore_ascii_case(name)) {
        x.folder.clear();
    }
    s.save_prefs()?;
    Ok(tree(s))
}

/// The presets named (`names`; default: all the user's) as a LightCraft presets file.
fn write_presets(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.writePresets";
    let path = name_param(ID, p, "path")?;
    let names: Option<Vec<String>> = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
    let chosen: Vec<ExportPreset> = match &names {
        Some(n) => s.all_export_presets().into_iter().map(|(x, _)| x).filter(|x| n.iter().any(|m| m.eq_ignore_ascii_case(&x.name))).collect(),
        None => s.export_presets.clone(),
    };
    if chosen.is_empty() {
        return Err(bad(ID, "no export presets to write (save one first, or pass names)"));
    }
    let mut path = std::path::PathBuf::from(path);
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let doc = json!({"format": PRESETS_FILE_FORMAT, "version": 1, "presets": chosen});
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| bad(ID, e.to_string()))?;
    s.check_write_target(&path.to_string_lossy()).map_err(|e| bad(ID, e))?;
    crate::export::write_file(&path.to_string_lossy(), &bytes).map_err(|e| bad(ID, e))?;
    Ok(json!({"path": path.to_string_lossy(), "count": chosen.len()}))
}

/// Largest presets file read (a presets file is a few KB).
const MAX_PRESET_FILE: u64 = 4 << 20;
/// Most files a folder import reads.
const MAX_PRESET_FILES: usize = 500;

fn read_capped(path: &std::path::Path) -> std::result::Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_PRESET_FILE {
        return Err(format!("{}: too large for a presets file", path.display()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The presets in one file: a LightCraft presets file (`.json`) or a Lightroom `.lrtemplate`.
fn presets_in_file(path: &std::path::Path) -> std::result::Result<(Vec<ExportPreset>, Vec<String>), String> {
    let text = read_capped(path)?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Imported".into());
    if ext == "lrtemplate" {
        let (p, notes) = crate::export_lrtemplate::read_export_lrtemplate(&text, &stem)?;
        let notes = notes.into_iter().map(|n| format!("{}: {n}", p.name)).collect();
        return Ok((vec![p], notes));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: not a presets file ({e})", path.display()))?;
    let list = match (&v, v.get("presets")) {
        (_, Some(Value::Array(a))) => a.clone(),
        (Value::Array(a), _) => a.clone(),
        // one preset ({name, params}) or bare app.export params
        (Value::Object(o), _) if o.contains_key("params") => vec![v.clone()],
        (Value::Object(_), _) => vec![json!({"name": stem, "params": v})],
        _ => return Err(format!("{}: not a presets file", path.display())),
    };
    let mut out = Vec::new();
    let mut notes = Vec::new();
    for item in list {
        let Some(name) = item.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else {
            notes.push("a preset without a name was left out".into());
            continue;
        };
        let params = crate::export::ExportOptions::known_keys_only(item.get("params").unwrap_or(&Value::Null));
        if !params.is_object() {
            notes.push(format!("{name}: no settings, left out"));
            continue;
        }
        if let Err(e) = crate::export::ExportOptions::validate("export.importPresets", &params) {
            notes.push(format!("{name}: {e}"));
            continue;
        }
        let folder = item.get("folder").and_then(Value::as_str).unwrap_or("").trim().to_string();
        out.push(ExportPreset { name: name.to_string(), params, folder: if is_builtin_folder(&folder) { String::new() } else { folder } });
    }
    Ok((out, notes))
}

/// Import presets from a file or a folder of them (LightCraft `.json`, Lightroom `.lrtemplate`);
/// a name that's taken gets a number; `folder` puts them all in that folder.
fn import_presets(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.importPresets";
    let path = std::path::PathBuf::from(name_param(ID, p, "path")?);
    let into = folder_param(ID, p)?;
    let files: Vec<std::path::PathBuf> = if path.is_dir() {
        let rd = std::fs::read_dir(&path).map_err(|e| bad(ID, format!("{}: {e}", path.display())))?;
        let mut v: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|f| f.extension().is_some_and(|x| x.eq_ignore_ascii_case("lrtemplate") || x.eq_ignore_ascii_case("json")))
            .take(MAX_PRESET_FILES)
            .collect();
        v.sort();
        v
    } else {
        vec![path.clone()]
    };
    let mut imported = Vec::new();
    let mut notes = Vec::new();
    for f in &files {
        let (presets, n) = match presets_in_file(f) {
            Ok(x) => x,
            Err(e) => {
                notes.push(e);
                continue;
            }
        };
        notes.extend(n);
        for mut preset in presets {
            let base = preset.name.clone();
            let taken = |n: &str, s: &Session| is_builtin(n) || s.export_presets.iter().any(|x| x.name.eq_ignore_ascii_case(n));
            let mut k = 2;
            while taken(&preset.name, s) && k < 10_000 {
                preset.name = format!("{base} {k}");
                k += 1;
            }
            if let Some(f) = &into {
                preset.folder = f.clone();
            }
            imported.push(preset.name.clone());
            s.export_presets.push(preset);
        }
    }
    if imported.is_empty() {
        return Err(bad(ID, notes.first().cloned().unwrap_or_else(|| format!("no export presets in {}", path.display()))));
    }
    s.save_prefs()?;
    Ok(json!({"imported": imported, "notes": notes}))
}

/// Existing Files ▸ Ask what to do: which of the photos' files are already in the destination.
fn existing_files(s: &mut Session, p: &Value) -> Result<Value> {
    const ID: &str = "export.existingFiles";
    let p = s.export_params(p).map_err(|e| bad(ID, e))?;
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        o.remove("resolve");
    }
    let o = crate::export::ExportOptions::from_params(&q)?;
    let ids = s.targets(&p);
    let dir = o.export_to.resolve(p.get("dir").and_then(Value::as_str).unwrap_or(""));
    let to = crate::export::Destination { dir, exact: None };
    let items = crate::export::prepare_batch(s, &ids, &o).map_err(|e| bad(ID, e))?;
    let found = crate::export::existing_targets(&items, &o, &to, &|f| std::path::Path::new(f).exists());
    Ok(Value::Array(found.into_iter().map(|(id, path)| json!({"photo": id.0, "path": path})).collect()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "export.presets",
            "Export Presets",
            [],
            None,
            "{} → [{name, builtin, folder, params}] (use with app.export {preset: name, …overrides})",
            always,
            |s, _| Ok(list(s))
        ),
        cmd!(
            query "export.presetFolders",
            "Export Preset Folders",
            [],
            None,
            "{} → [{name, builtin, presets: [names]}]: Lightroom Presets, LightCraft Presets, User Presets, then the user's folders",
            always,
            |s, _| Ok(tree(s))
        ),
        cmd!(
            "export.savePreset",
            "Save Export Preset",
            [],
            None,
            "{name, params?: app.export params (default: the last export's), folder?} — adds a user preset, or updates one with these settings (it keeps its folder unless `folder` is given); the destination folder is kept → presets",
            always,
            save
        ),
        cmd!("export.deletePreset", "Delete Export Preset", [], None, "{name} — removes a user preset → presets", always, delete),
        cmd!("export.renamePreset", "Rename Export Preset", [], None, "{name, to} — renames a user preset → presets", always, rename),
        cmd!("export.movePreset", "Move Export Preset", [], None, "{name, folder} — puts a user preset in a folder (`User Presets` = none) → folders", always, move_preset),
        cmd!("export.createPresetFolder", "New Export Preset Folder", [], None, "{name} — a folder for export presets → folders", always, create_folder),
        cmd!(
            "export.deletePresetFolder",
            "Remove Export Preset Folder",
            [],
            None,
            "{name} — removes a folder; its presets move to User Presets → folders",
            always,
            delete_folder
        ),
        cmd!(
            query "export.writePresets",
            "Export Presets to File",
            [],
            None,
            "{path, names?} — writes the user's presets (or those named) to a LightCraft presets file (.json) → {path, count}",
            always,
            write_presets
        ),
        cmd!(
            "export.importPresets",
            "Import Export Presets",
            [],
            None,
            "{path, folder?} — adds the presets in a LightCraft presets file, a Lightroom .lrtemplate export preset, or a folder of them (taken names get a number) → {imported, notes}",
            always,
            import_presets
        ),
        cmd!(
            query "export.existingFiles",
            "Existing Export Files",
            [],
            None,
            "{app.export params, dir} — the photos whose file is already in the destination (what Existing Files ▸ Ask what to do asks about) → [{photo, path}]",
            always,
            existing_files
        ),
        cmd!(
            query "export.actionsFolder",
            "Go to Export Actions Folder Now",
            [],
            None,
            "{} — makes the Export Actions folder if needed (apps, aliases and scripts put there are offered under After Export) → {path}",
            always,
            |_, _| {
                let dir = crate::export::export_actions_dir().ok_or_else(|| bad("export.actionsFolder", "no home folder"))?;
                std::fs::create_dir_all(&dir).map_err(|e| bad("export.actionsFolder", format!("{}: {e}", dir.display())))?;
                Ok(json!({"path": dir.to_string_lossy()}))
            }
        ),
        cmd!(
            query "export.checkTarget",
            "Check Output Path",
            [],
            None,
            "{path} — fails when writing `path` would replace a photo's original (or its XMP sidecar) in the library; front ends call it before saving a render or screenshot to a user-given path → {path}",
            always,
            |s, p| {
                let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad("export.checkTarget", "missing `path`"))?;
                s.check_write_target(path).map_err(|e| bad("export.checkTarget", e))?;
                Ok(json!({"path": path}))
            }
        ),
    ]
}
