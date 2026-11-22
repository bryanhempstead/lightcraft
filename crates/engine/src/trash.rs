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
