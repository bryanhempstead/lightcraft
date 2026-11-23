//! Move files to the system Trash (Library ▸ Delete from Disk).
//!
//! - macOS: `FileManager.trashItem` through a small Swift helper (`trash.swift`, our own source,
//!   built once with `swiftc` into LightCraft's support folder, like the face finder), so the
//!   Finder's Put Back works. Without the Swift compiler: a rename into `~/.Trash` (files on the
//!   home volume only).
//! - Linux / BSD: the freedesktop.org trash (`$XDG_DATA_HOME/Trash` with a `.trashinfo`).
//! - Elsewhere: an error ("use Remove"); a file is never deleted outright.
//!
//! `LIGHTCRAFT_TRASH_DIR` (tests, scripts) sends every file into that folder instead.

use std::path::{Path, PathBuf};

/// Move `paths` to the Trash — or into `instead` (a session's test folder) — one result per path
/// (where it went, or why not). Missing files are errors too (nothing to move).
pub fn trash(paths: &[PathBuf], instead: Option<&Path>) -> Vec<Result<PathBuf, String>> {
    let env = std::env::var_os("LIGHTCRAFT_TRASH_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
    if let Some(dir) = instead.map(Path::to_path_buf).or(env) {
        return paths.iter().map(|p| into_folder(p, &dir)).collect();
    }
    platform(paths)
}

/// A free name for `name` in `dir` ("a.jpg", "a 2.jpg", …).
fn free_name(dir: &Path, name: &std::ffi::OsStr) -> Option<PathBuf> {
    let first = dir.join(name);
    if !first.exists() {
        return Some(first);
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..10_000).map(|i| dir.join(format!("{stem} {i}{ext}"))).find(|c| !c.exists())
}

/// Rename `path` into `dir` (same volume only: never a copy + delete of someone's original).
fn into_folder(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("{}: no such file", path.display()));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let name = path.file_name().ok_or_else(|| format!("{}: not a file", path.display()))?;
    let to = free_name(dir, name).ok_or_else(|| format!("no free name in {}", dir.display()))?;
    std::fs::rename(path, &to).map_err(|e| format!("{}: can't move it to the Trash ({e})", path.display()))?;
    Ok(to)
}

#[cfg(target_os = "macos")]
const SWIFT: &str = include_str!("trash.swift");

#[cfg(target_os = "macos")]
fn helper() -> Result<PathBuf, String> {
    use std::hash::{Hash, Hasher};
    let home = std::env::var_os("HOME").ok_or("no HOME")?;
    let dir = PathBuf::from(home).join("Library/Application Support/LightCraft/bin");
    let mut h = std::collections::hash_map::DefaultHasher::new();
    SWIFT.hash(&mut h);
    let bin = dir.join(format!("lc-trash-{:016x}", h.finish()));
    if bin.exists() {
        return Ok(bin);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let src = dir.join("lc-trash.swift");
    std::fs::write(&src, SWIFT).map_err(|e| format!("{}: {e}", src.display()))?;
    let out = std::process::Command::new("/usr/bin/swiftc")
        .args(["-O", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .map_err(|e| format!("the Swift compiler isn't available ({e})"))?;
    if !out.status.success() || !bin.exists() {
        return Err(format!("building the Trash helper failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(300).collect::<String>()));
    }
    Ok(bin)
}

#[cfg(target_os = "macos")]
fn platform(paths: &[PathBuf]) -> Vec<Result<PathBuf, String>> {
    if paths.is_empty() {
        return vec![];
    }
    let fallback = |paths: &[PathBuf]| -> Vec<Result<PathBuf, String>> {
        let Some(home) = std::env::var_os("HOME") else { return paths.iter().map(|_| Err("no HOME".to_string())).collect() };
        let dir = PathBuf::from(home).join(".Trash");
        paths.iter().map(|p| into_folder(p, &dir)).collect()
    };
    let bin = match helper() {
        Ok(b) => b,
        Err(e) => {
            log::warn!("trash: {e}; moving files into ~/.Trash");
            return fallback(paths);
        }
    };
    let out = match std::process::Command::new(&bin).args(paths).output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("trash helper: {e}; moving files into ~/.Trash");
            return fallback(paths);
        }
    };
    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| match lines.get(i) {
            Some(v) => match (v.get("trashed").and_then(|t| t.as_str()), v.get("error").and_then(|e| e.as_str())) {
                (Some(t), _) => Ok(PathBuf::from(t)),
                (None, Some(e)) => Err(format!("{}: {e}", p.display())),
                (None, None) => Err(format!("{}: the Trash helper gave no answer", p.display())),
            },
            None => Err(format!("{}: the Trash helper gave no answer", p.display())),
        })
        .collect()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform(paths: &[PathBuf]) -> Vec<Result<PathBuf, String>> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    let Some(base) = base else { return paths.iter().map(|_| Err("no HOME".to_string())).collect() };
    let (files, info) = (base.join("Trash/files"), base.join("Trash/info"));
    paths
        .iter()
        .map(|p| {
            let abs = std::fs::canonicalize(p).map_err(|e| format!("{}: {e}", p.display()))?;
            std::fs::create_dir_all(&info).map_err(|e| format!("{}: {e}", info.display()))?;
            let to = into_folder(&abs, &files)?;
            let name = to.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            let when = lightcraft_catalog::dates::civil(secs);
            let body = format!("[Trash Info]\nPath={}\nDeletionDate={when}\n", abs.display());
            if let Err(e) = std::fs::write(info.join(format!("{name}.trashinfo")), body) {
                log::warn!("trash: {name}.trashinfo: {e}");
            }
            Ok(to)
        })
        .collect()
}

#[cfg(not(unix))]
fn platform(paths: &[PathBuf]) -> Vec<Result<PathBuf, String>> {
    paths.iter().map(|p| Err(format!("{}: moving files to the Recycle Bin isn't available on this system; use Remove", p.display()))).collect()
}

/// A file a delete moved to the Trash: where it was, and where it is now.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrashedFile {
    pub original: String,
    pub trash: String,
}

/// Move `f` back out of the Trash to its original path (undo of a delete, Put Back Last Delete).
/// A rename, as the Trash is on the file's own volume; never over a file that is there now
/// (no-clobber through a hard link where the volume has them). The folder it was in comes back
/// if it is gone.
pub fn put_back(f: &TrashedFile) -> Result<(), String> {
    let (from, to) = (Path::new(&f.trash), Path::new(&f.original));
    if std::fs::symlink_metadata(from).is_err() {
        return Err(format!("{}: no longer in the Trash", f.original));
    }
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(format!("{}: another file is there now, so it stays in the Trash", f.original));
    }
    if let Some(dir) = to.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    match std::fs::hard_link(from, to) {
        Ok(()) => {
            if let Err(e) = std::fs::remove_file(from) {
                log::warn!("put back: {} stays in the Trash too ({e})", from.display());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(format!("{}: another file is there now, so it stays in the Trash", f.original));
        }
        // no hard links here (exFAT, a folder): a plain rename, the path checked just above
        Err(_) => std::fs::rename(from, to).map_err(|e| format!("{}: can't put it back ({e})", f.original))?,
    }
    forget_trash_info(from);
    Ok(())
}

/// The freedesktop.org Trash keeps a `.trashinfo` beside each file it holds: drop it once the
/// file is back.
fn forget_trash_info(trashed: &Path) {
    let (Some(files), Some(name)) = (trashed.parent(), trashed.file_name()) else { return };
    if files.file_name().is_none_or(|n| n != "files") {
        return;
    }
    let Some(base) = files.parent() else { return };
    let mut info = name.to_os_string();
    info.push(".trashinfo");
    let info = base.join("info").join(info);
    if info.is_file() {
        let _ = std::fs::remove_file(info);
    }
}

/// The last delete that moved files to the Trash, kept for Put Back Last Delete
/// (`photo.putBackLastDelete`) once the undo history has moved on, and saved with the library
/// (`last-delete.json`) so it survives a reopen: the photos as they were, the albums they were
/// in, and where their files went.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LastDelete {
    pub label: String,
    pub photos: Vec<lightcraft_catalog::Photo>,
    /// Album → the deleted photos that were in it.
    pub albums: Vec<(lightcraft_catalog::AlbumId, Vec<lightcraft_catalog::PhotoId>)>,
    pub files: Vec<TrashedFile>,
}

impl LastDelete {
    /// What `ids` are now (records and album memberships), before a delete that sends `files`
    /// to the Trash.
    pub(crate) fn of(s: &crate::Session, label: &str, ids: &[lightcraft_catalog::PhotoId], files: Vec<TrashedFile>) -> LastDelete {
        let photos: Vec<lightcraft_catalog::Photo> = ids.iter().filter_map(|id| s.catalog.photo(*id)).map(|p| (**p).clone()).collect();
        let albums = s
            .catalog
            .albums()
            .filter(|a| !a.is_smart())
            .filter_map(|a| {
                let mine: Vec<_> = a.photos.iter().copied().filter(|p| ids.contains(p)).collect();
                (!mine.is_empty()).then_some((a.id, mine))
            })
            .collect();
        LastDelete { label: label.to_string(), photos, albums, files }
    }

    /// [`LastDelete::of`] for the photos an undo/redo op removes.
    pub(crate) fn before(s: &crate::Session, label: &str, op: &lightcraft_catalog::Op, files: Vec<TrashedFile>) -> LastDelete {
        let mut ids = Vec::new();
        removed_photos(op, &mut ids, 0);
        LastDelete::of(s, label, &ids, files)
    }
}

/// The photos `op` removes (`RemovePhoto`, nested batches up to a few levels).
fn removed_photos(op: &lightcraft_catalog::Op, out: &mut Vec<lightcraft_catalog::PhotoId>, depth: usize) {
    match op {
        lightcraft_catalog::Op::RemovePhoto { id } => out.push(*id),
        lightcraft_catalog::Op::Batch { ops } if depth < 8 => ops.iter().for_each(|o| removed_photos(o, out, depth + 1)),
        _ => {}
    }
}
