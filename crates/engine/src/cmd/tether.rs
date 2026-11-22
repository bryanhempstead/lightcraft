//! L33 Tethered capture: photos arrive in a shoot folder as they're taken and are imported at
//! once (the watched folder, `library.autoImport`). With `gphoto2` installed (Homebrew), LightCraft
//! runs `gphoto2 --capture-tethered` itself, so a USB-connected camera's shots land there;
//! otherwise any tethering app that saves into the folder works (Canon EOS Utility, Sony Imaging
//! Edge, Capture One…). Never blocks: the capture process runs on its own; stopping ends it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{CommandSpec, always, bad, cmd, str_param};
use crate::{Result, Session};

/// A running tethered session.
#[derive(Debug)]
pub struct Tether {
    pub folder: String,
    pub started: String,
    /// `gphoto2 --capture-tethered`, when LightCraft started it.
    #[cfg(not(target_arch = "wasm32"))]
    pub child: Option<std::process::Child>,
}

impl Drop for Tether {
    // a session that goes away (quit, another library) ends the capture process too
    fn drop(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// `gphoto2` on this machine (PATH, Homebrew's folders), if any.
pub fn gphoto2() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("LIGHTCRAFT_GPHOTO2") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].iter().map(PathBuf::from));
    dirs.into_iter().map(|d| d.join("gphoto2")).find(|p| p.is_file())
}

fn status(s: &Session) -> Value {
    match &s.tether {
        Some(t) => {
            #[cfg(not(target_arch = "wasm32"))]
            let capture = t.child.is_some();
            #[cfg(target_arch = "wasm32")]
            let capture = false;
            json!({"running": true, "folder": t.folder, "started": t.started, "gphoto2": capture})
        }
        None => json!({"running": false, "gphoto2Installed": gphoto2().is_some()}),
    }
}

fn start(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "tether.start";
    if s.tether.is_some() {
        return Err(bad(C, "tethered capture is running already (tether.stop first)"));
    }
    let base = str_param(p, "folder").map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "missing `folder` (where the shots go)"))?;
    let base = Path::new(base);
    if !base.is_absolute() {
        return Err(bad(C, "`folder` must be an absolute path"));
    }
    let name = str_param(p, "session").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Tethered");
    if name.contains(['/', '\\']) || name == ".." {
        return Err(bad(C, "`session` is a folder name (no slashes)"));
    }
    let folder = base.join(name);
    std::fs::create_dir_all(&folder).map_err(|e| bad(C, format!("{}: {e}", folder.display())))?;
    let folder_s = folder.display().to_string();
    // the watched folder: shots are imported in place, into an album named after the session
    s.execute("library.autoImport", &json!({"folder": folder_s, "copy": false, "album": name}))?;
    let binary = str_param(p, "gphoto2").map(PathBuf::from).filter(|b| b.is_file()).or_else(gphoto2);
    #[cfg(target_arch = "wasm32")]
    let _ = binary;
    #[cfg(not(target_arch = "wasm32"))]
    let child = match (p.get("capture").and_then(Value::as_bool).unwrap_or(true), binary) {
        (true, Some(g)) => {
            let pattern = folder.join("%f.%C").display().to_string();
            match std::process::Command::new(&g)
                .args(["--capture-tethered", "--keep", "--filename", &pattern])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(c) => Some(c),
                Err(e) => {
                    log::warn!("tether: {}: {e}", g.display());
                    None
                }
            }
        }
        _ => None,
    };
    s.tether = Some(Tether {
        folder: folder_s,
        started: (s.clock)(),
        #[cfg(not(target_arch = "wasm32"))]
        child,
    });
    Ok(status(s))
}

fn stop(s: &mut Session) -> Result<Value> {
    let Some(t) = s.tether.take() else { return Ok(json!({"running": false})) };
    let folder = t.folder.clone();
    drop(t);
    s.execute("library.autoImport", &json!({"folder": null}))?;
    Ok(json!({"running": false, "folder": folder}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "tether.start",
            "Start Tethered Capture",
            [],
            None,
            "{folder, session?: name (default Tethered), capture?: bool (run gphoto2 when installed; default true), gphoto2?: its path} — shots saved into <folder>/<session> are imported as they arrive, into an album named after the session → {running, folder, gphoto2}",
            always,
            start
        ),
        cmd!("tether.stop", "Stop Tethered Capture", [], None, "{} — ends the capture process and stops watching the folder", always, |s, _| stop(
            s
        )),
        cmd!(query "tether.status", "Tethered Capture Status", [], None, "{} → {running, folder?, gphoto2?, gphoto2Installed?}", always, |s, _| Ok(status(s))),
    ]
}
