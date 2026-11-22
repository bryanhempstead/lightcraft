//! LR-EXP-PHOTOS: export to the system photo library (Apple Photos, macOS).
//!
//! The photos are exported as usual (JPEG by default, any `app.export` options) into a staging
//! folder, then handed to Photos through its own AppleScript dictionary — Photos' `import`
//! command, run by `/usr/bin/osascript` — optionally into an album (made when missing). No
//! keystrokes or UI scripting: macOS asks once whether LightCraft may control Photos. Photos
//! copies the files into its library, so the staging folder is removed afterwards.
//!
//! The script takes everything as arguments (never pasted into its text), so names and paths
//! need no quoting. Elsewhere than macOS, or without `osascript`, it is a clear error.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::export::{Conflict, Destination, ExportOptions, PreparedExport};

pub const OSASCRIPT: &str = "/usr/bin/osascript";

/// How long Photos may take (a first run waits for the "control Photos" permission prompt).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

/// The import script. `argv`: a fixed first item (so no later argument is read as an option),
/// the album name (empty = no album), then the files.
pub const IMPORT_SCRIPT: &str = r#"on run argv
	set albumName to item 2 of argv
	set theFiles to {}
	repeat with i from 3 to count of argv
		set end of theFiles to (POSIX file (item i of argv)) as alias
	end repeat
	tell application "Photos"
		if albumName is "" then
			set imported to import theFiles skip check duplicates true
		else
			if exists album albumName then
				set theAlbum to album albumName
			else
				set theAlbum to make new album named albumName
			end if
			set imported to import theFiles into theAlbum skip check duplicates true
		end if
		return count of imported
	end tell
end run"#;

/// The `osascript` arguments that import `files` into `album` (empty: none).
pub fn import_args(album: &str, files: &[String]) -> Vec<String> {
    let mut v = vec!["-e".to_string(), IMPORT_SCRIPT.to_string(), "lightcraft".to_string(), album.trim().to_string()];
    v.extend(files.iter().cloned());
    v
}

/// Runs `osascript` with these arguments → its output (tests pass their own).
pub type Runner<'a> = dyn Fn(&[String]) -> Result<String, String> + 'a;

/// Photos can be scripted here.
pub fn available() -> bool {
    cfg!(target_os = "macos") && Path::new(OSASCRIPT).is_file()
}

/// The real runner: `/usr/bin/osascript` with a timeout.
pub fn run_osascript(args: &[String]) -> Result<String, String> {
    if !available() {
        return Err("Export to Photos needs macOS (Apple Photos and /usr/bin/osascript)".into());
    }
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(OSASCRIPT)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("osascript: {e}"))?;
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Photos didn't answer within {} minutes", TIMEOUT.as_secs() / 60));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(e) => return Err(format!("osascript: {e}")),
        }
    };
    let (mut out, mut err) = (String::new(), String::new());
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut err);
    }
    if !status.success() {
        let err = err.trim();
        // -1743: the user (or the system) didn't allow LightCraft to control Photos
        if err.contains("-1743") || err.contains("Not authorized") {
            return Err("LightCraft may not control Photos: allow it in System Settings ▸ Privacy & Security ▸ Automation".into());
        }
        return Err(if err.is_empty() { format!("Photos import failed ({status})") } else { format!("Photos: {err}") });
    }
    Ok(out.trim().to_string())
}

/// Import `files` into Photos (into `album` when not empty) → how many Photos took.
pub fn import(album: &str, files: &[String], runner: &Runner<'_>) -> Result<usize, String> {
    if files.is_empty() {
        return Ok(0);
    }
    let out = runner(&import_args(album, files))?;
    Ok(out.trim().parse::<usize>().unwrap_or(files.len()))
}

/// An export to Photos set up from the session ([`prepare`]); [`PhotosJob::run`] needs none.
pub struct PhotosJob {
    items: Vec<PreparedExport>,
    opts: ExportOptions,
    pub album: String,
    pub staging: PathBuf,
}

impl PhotosJob {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The options: `params` (`app.export` options) over JPEG, quality 92, sRGB; never Same Folder,
/// Add to Catalog or post-processing (Photos is the destination).
pub fn options(params: &Value) -> Result<ExportOptions, String> {
    let mut p = json!({"format": "jpeg", "quality": 92, "colorSpace": "srgb"});
    if let (Some(o), Some(own)) = (p.as_object_mut(), params.as_object()) {
        for (k, v) in own.iter().filter(|(k, _)| !matches!(k.as_str(), "ids" | "album" | "dir" | "path" | "preset" | "folder" | "subfolders")) {
            o.insert(k.clone(), v.clone());
        }
    }
    let mut o = ExportOptions::from_params(&p).map_err(|e| e.to_string())?;
    o.same_folder = false;
    o.add_to_catalog = false;
    o.add_to_stack = None;
    o.subfolder.clear();
    o.after = crate::export::AfterExport::default();
    o.conflict = Conflict::Unique;
    Ok(o)
}

/// Set up exporting `ids` to Photos.
pub fn prepare(s: &mut crate::Session, ids: &[lightcraft_catalog::PhotoId], album: &str, params: &Value) -> Result<PhotosJob, String> {
    if ids.is_empty() {
        return Err("no photos to export".into());
    }
    let opts = options(params)?;
    if !opts.format.is_rendered() {
        return Err("Photos takes rendered files: choose JPEG, PNG, TIFF… (not Original or DNG)".into());
    }
    let items = crate::export::prepare_batch(s, ids, &opts)?;
    let stamp: String = (s.clock)().chars().filter(char::is_ascii_digit).collect();
    let staging = std::env::temp_dir().join(format!("LightCraft to Photos {stamp}-{}", std::process::id()));
    Ok(PhotosJob { items, opts, album: album.trim().to_string(), staging })
}

impl PhotosJob {
    /// Export into the staging folder, import into Photos, remove the staging folder →
    /// `{imported, album, files, errors}`. Photos refusing is an error; a photo that failed to
    /// export is listed in `errors` and the others still go.
    pub fn run(self, write: &mut dyn FnMut(&str, &[u8]) -> Result<(), String>, runner: &Runner<'_>) -> Result<Value, String> {
        let to = Destination { dir: self.staging.to_string_lossy().to_string(), exact: None };
        let r = crate::export::run_batch(self.items, &self.opts, &to, write, &|p| Path::new(p).exists(), false, &mut |_, _| true);
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&self.staging);
                return Err(e);
            }
        };
        let files: Vec<String> = r.iter().filter_map(|v| v.get("path").and_then(Value::as_str).map(str::to_string)).collect();
        let errors: Vec<Value> = r.iter().filter(|v| v.get("error").is_some()).cloned().collect();
        if files.is_empty() {
            let _ = std::fs::remove_dir_all(&self.staging);
            let first = errors.first().and_then(|e| e["error"].as_str()).unwrap_or("nothing was exported").to_string();
            return Err(first);
        }
        let n = import(&self.album, &files, runner);
        let _ = std::fs::remove_dir_all(&self.staging);
        let n = n?;
        Ok(json!({"imported": n, "album": self.album, "files": files.len(), "errors": errors}))
    }
}

/// `export.toPhotos`: [`prepare`] + [`PhotosJob::run`] with native writes and `runner`.
pub fn export_to_photos(s: &mut crate::Session, p: &Value, runner: &Runner<'_>) -> crate::Result<Value> {
    const C: &str = "export.toPhotos";
    let ids = s.targets(p);
    let album = p.get("album").and_then(Value::as_str).unwrap_or("");
    let job = prepare(s, &ids, album, p).map_err(|e| crate::cmd::bad(C, e))?;
    job.run(&mut |path, b| crate::export::write_file(path, b), runner).map_err(|e| crate::cmd::bad(C, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_args_keep_names_out_of_the_script() {
        let files = vec!["/tmp/a \"b\".jpg".to_string(), "/tmp/-c.jpg".to_string()];
        let a = import_args(" Wedding \"2026\" ", &files);
        assert_eq!(a[0], "-e");
        assert_eq!(a[1], IMPORT_SCRIPT);
        // a fixed first argument: nothing after it is read as an osascript option
        assert_eq!(a[2], "lightcraft");
        assert_eq!(a[3], "Wedding \"2026\"");
        assert_eq!(&a[4..], &files[..]);
        // the script reads its arguments; no name or path is ever spliced into it
        assert!(IMPORT_SCRIPT.contains("item 2 of argv") && IMPORT_SCRIPT.contains("repeat with i from 3"));
        assert!(IMPORT_SCRIPT.contains("tell application \"Photos\"") && IMPORT_SCRIPT.contains("import theFiles"));
        assert!(!IMPORT_SCRIPT.contains("System Events") && !IMPORT_SCRIPT.contains("keystroke"));
    }

    #[test]
    fn import_reports_the_count_and_errors() {
        let ok = |_: &[String]| Ok::<_, String>("2".to_string());
        assert_eq!(import("", &["/a.jpg".into(), "/b.jpg".into()], &ok), Ok(2));
        let no = |_: &[String]| Err::<String, _>("Photos: not allowed".to_string());
        assert!(import("x", &["/a.jpg".into()], &no).is_err());
        // nothing to import: Photos isn't asked
        let never = |_: &[String]| -> Result<String, String> { panic!("called") };
        assert_eq!(import("x", &[], &never), Ok(0));
    }

    #[test]
    fn options_default_to_jpeg_and_stay_out_of_the_catalog() {
        let o = options(&json!({"ids": [1], "album": "A", "addToCatalog": true, "sameFolder": true, "longEdge": 1024})).unwrap();
        assert_eq!(o.format, crate::export::ExportFormat::Jpeg);
        assert!(!o.add_to_catalog && !o.same_folder);
        assert!(o.resize.is_some());
        assert!(options(&json!({"format": "nonsense"})).is_err());
    }
}
