//! Lightroom Classic's Folders panel: Show Photos in Subfolders, Synchronize Folder, Find
//! Missing Folder, Create Folder Inside, Add Parent Folder / Promote Subfolders. The tree itself
//! is the catalog's ([`lightcraft_catalog::folders`]): the folders the library's photos are in,
//! never a browse of the disk (that is Local's job, `cmd::browse`).

use std::collections::HashSet;
use std::path::Path;

use lightcraft_catalog::{Filter, Op, Source};
use serde_json::{Value, json};

use super::{CommandSpec, always, bad, bool_or, cmd, str_param};
use crate::import::{ImportMode, ImportOptions, expand, import_with, is_supported};
use crate::{LibrarySource, Result, Session};

/// The most new files a dry run lists (for the app to import them in the background).
const MAX_LISTED: usize = 200_000;

/// The library's photos (not Local browse records, not in Recently Deleted) in `folder`, and in
/// the folders inside it when `subfolders`.
fn photos_in(s: &Session, folder: &str, subfolders: bool) -> Vec<lightcraft_catalog::PhotoId> {
    let f = Filter { library_folder: Some(folder.to_string()), library_folder_only: !subfolders, ..Default::default() };
    s.catalog.query(&f, &lightcraft_catalog::Sort::default())
}

/// A folder parameter: present, not blank, its trailing separators dropped.
fn folder_param<'a>(c: &str, p: &'a Value, key: &str) -> Result<&'a str> {
    let v = str_param(p, key).map(|v| v.trim_end_matches(['/', '\\'])).filter(|v| !v.trim().is_empty());
    v.ok_or_else(|| bad(c, format!("missing `{key}` (a folder)")))
}

/// Folders: the catalog's tree, as Lightroom Classic lists it ([`lightcraft_catalog::folders::root_folders`])
/// unless `all`, with the empty folders made in the library.
pub fn folder_view(s: &Session, classic: bool) -> Vec<lightcraft_catalog::FolderNode> {
    let tree = s.catalog.folder_tree_with(&s.empty_folders);
    if classic { lightcraft_catalog::folders::root_folders(&tree, &s.folder_parents) } else { tree }
}

fn show_subfolders(s: &mut Session, p: &Value) -> Result<Value> {
    s.library_subfolders = match p.get("on") {
        Some(Value::Bool(b)) => *b,
        None | Some(Value::Null) => !s.library_subfolders,
        Some(_) => return Err(bad("library.showSubfolders", "`on` is true or false")),
    };
    s.save_prefs()?;
    let count = if s.source == LibrarySource::LibraryFolder { Some(s.visible().len()) } else { None };
    Ok(json!({"on": s.library_subfolders, "count": count}))
}

/// Synchronize Folder: add the files in the folder that the library doesn't have (in place, like
/// Add), and say which of its photos have no file any more.
fn sync(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.sync";
    let folder = folder_param(C, p, "path")?.to_string();
    if !Path::new(&folder).is_dir() {
        return Err(bad(C, format!("{folder}: not found (is its disk connected? Find Missing Folder points the library at where it went)")));
    }
    let subfolders = bool_or(p, "subfolders", s.library_subfolders);
    let skip = s.library.as_ref().map(|l| l.dir.clone());
    let on_disk: Vec<String> = if subfolders {
        expand(std::slice::from_ref(&folder), skip.as_deref())
    } else {
        let mut v: Vec<String> = std::fs::read_dir(&folder)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|f| f.is_file() && is_supported(f) && !f.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                    .map(|f| f.to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    };
    let known: HashSet<String> = s
        .catalog
        .photos()
        .filter(|p| !p.local)
        .filter_map(|p| match &p.source {
            Source::File { path } => Some(lightcraft_catalog::query::folder_key(path)),
            _ => None,
        })
        .collect();
    let new: Vec<String> = on_disk.into_iter().filter(|f| !known.contains(&lightcraft_catalog::query::folder_key(f))).collect();
    let mut imported = 0;
    let mut failed = Vec::new();
    if !new.is_empty() && !bool_or(p, "dryRun", false) {
        let r = import_with(s, &new, &ImportOptions { mode: ImportMode::Add, ..Default::default() })?;
        imported = r.imported.len();
        failed = r.failed.into_iter().take(50).collect();
    }
    let missing: Vec<String> = photos_in(s, &folder, subfolders)
        .into_iter()
        .filter_map(|id| match s.catalog.photo(id).map(|p| &p.source) {
            Some(Source::File { path }) if !Path::new(path).exists() => Some(path.clone()),
            _ => None,
        })
        .collect();
    let new_files: Vec<&String> = if bool_or(p, "dryRun", false) { new.iter().take(MAX_LISTED).collect() } else { Vec::new() };
    Ok(json!({
        "path": folder,
        "new": new.len(),
        "newFiles": new_files,
        "imported": imported,
        "failed": failed,
        "missing": missing.len(),
        "missingFiles": missing.iter().take(50).collect::<Vec<_>>(),
    }))
}

/// Find Missing Folder: the folder was moved or renamed (or its disk renamed): point every
/// library photo in it at the same file under `to`. Only photos whose file is there are relinked;
/// one undo step.
fn locate(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.locate";
    let from = folder_param(C, p, "path")?.to_string();
    let to = folder_param(C, p, "to")?.to_string();
    if !Path::new(&to).is_dir() {
        return Err(bad(C, format!("{to}: not a folder")));
    }
    let src = Path::new(&from);
    let mut ops = Vec::new();
    let mut still = 0usize;
    for id in photos_in(s, &from, true) {
        let Some(ph) = s.catalog.photo(id) else { continue };
        let Source::File { path } = &ph.source else { continue };
        let Ok(rel) = Path::new(path).strip_prefix(src) else { continue };
        let np = Path::new(&to).join(rel);
        if np.is_file() {
            ops.push(Op::Relink {
                id,
                file_name: ph.file_name.clone(),
                source: Source::File { path: np.to_string_lossy().to_string() },
                format: None,
            });
        } else {
            still += 1;
        }
    }
    let n = ops.len();
    if n == 0 {
        return Err(bad(C, format!("none of the photos from {from} are in {to}")));
    }
    let name = Path::new(&to).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    s.commit(&format!("Find Missing Folder “{name}”"), Op::Batch { ops })?;
    super::browse::follow_folder(s, &from, &to);
    // a pinned, removed or renamed shoot in it keeps that choice (prefs.json is written with the
    // next preference change if it can't be now; the relink above is done either way)
    let before = s.shoot_prefs.clone();
    s.shoot_prefs.follow(&from, &to);
    if s.shoot_prefs != before {
        let _ = s.save_prefs();
    }
    Ok(json!({"path": to, "relinked": n, "stillMissing": still}))
}

/// Create Folder Inside: a new folder on disk, listed in Folders while it is empty.
fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.create";
    let parent = folder_param(C, p, "path")?.to_string();
    let name = str_param(p, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty() && !n.contains(['/', '\\']) && *n != "." && *n != "..")
        .ok_or_else(|| bad(C, "give a folder name (no slashes)"))?;
    if !Path::new(&parent).is_dir() {
        return Err(bad(C, format!("{parent}: not found")));
    }
    let dir = Path::new(&parent).join(name);
    if dir.exists() {
        return Err(bad(C, format!("{} already exists", dir.display())));
    }
    std::fs::create_dir(&dir).map_err(|e| bad(C, format!("{}: {e}", dir.display())))?;
    let dir = dir.to_string_lossy().to_string();
    s.empty_folders.retain(|f| !lightcraft_catalog::query::folder_within(f, &dir) || f == &dir);
    if !s.empty_folders.contains(&dir) {
        s.empty_folders.push(dir.clone());
    }
    s.save_prefs()?;
    Ok(json!({"path": dir}))
}

fn add_parent(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.addParent";
    let folder = folder_param(C, p, "path")?;
    let parent = lightcraft_catalog::folders::parent_folder(folder).ok_or_else(|| bad(C, format!("{folder} is the top of its disk")))?;
    if lightcraft_catalog::folders::is_disk_root(&parent) {
        return Err(bad(C, format!("{folder} is already at the top of its disk")));
    }
    let key = lightcraft_catalog::query::folder_key(&parent);
    if !s.folder_parents.iter().any(|f| lightcraft_catalog::query::folder_key(f) == key) {
        s.folder_parents.push(parent.clone());
    }
    s.save_prefs()?;
    Ok(json!({"path": parent}))
}

fn promote(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "folder.promoteSubfolders";
    let folder = folder_param(C, p, "path")?;
    let key = lightcraft_catalog::query::folder_key(folder);
    let before = s.folder_parents.len();
    s.folder_parents.retain(|f| lightcraft_catalog::query::folder_key(f) != key);
    if s.folder_parents.len() == before {
        return Err(bad(C, format!("{folder} was not added with Add Parent Folder (folders that hold photos stay listed)")));
    }
    s.save_prefs()?;
    Ok(json!({"path": folder}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "library.showSubfolders",
            "Show Photos in Subfolders",
            ["Library"],
            None,
            "{on?: bool} (default: toggle) — choosing a folder shows the photos of the folders inside it too (on, the default) or only its own; its count follows. Kept in the library's preferences → {on, count}",
            always,
            show_subfolders
        ),
        cmd!(
            "folder.sync",
            "Synchronize Folder",
            [],
            None,
            "{path, subfolders?: bool (default: Show Photos in Subfolders), dryRun?: bool (only look: `newFiles` lists them, for a background import)} — add the files in the folder the library doesn't have yet (in place) and report its photos whose file is gone → {new, newFiles, imported, failed, missing, missingFiles}",
            always,
            sync
        ),
        cmd!(
            "folder.locate",
            "Find Missing Folder",
            [],
            None,
            "{path: the folder the library knows, to: where it is now} — relink every library photo in it (and the folders inside it) whose file is at the same place under `to`; one undo step → {path, relinked, stillMissing}",
            always,
            locate
        ),
        cmd!(
            "folder.create",
            "Create Folder Inside",
            [],
            None,
            "{path: a folder, name} — make a folder inside it on disk; Folders lists it while it is empty → {path}",
            always,
            create
        ),
        cmd!(
            "folder.addParent",
            "Add Parent Folder",
            [],
            None,
            "{path} — list the folder holding this one in Folders, above it (Folders otherwise starts at the first folder where photos are or branch) → {path: the parent}",
            always,
            add_parent
        ),
        cmd!(
            "folder.promoteSubfolders",
            "Promote Subfolders",
            [],
            None,
            "{path: a folder added with Add Parent Folder} — stop listing it; the folders inside it are listed in its place → {path}",
            always,
            promote
        ),
    ]
}
