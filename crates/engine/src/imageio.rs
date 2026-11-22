//! Image formats only macOS's own encoders write (ImageIO), through a small helper of our own
//! (`imageio.swift`, built once with `swiftc` into LightCraft's support folder, like the video
//! and face helpers).
//!
//! - [`avif_gain_map`]: an HDR AVIF as SDR base + ISO 21496-1 gain map (AVIF `tmap` item), made
//!   from LightCraft's gain-map JPEG. Our AVIF muxer (avif-serialize) can't write gain maps and
//!   libavif isn't linked; ImageIO writes the standard structure (`tmap` brand, `tmap` derived
//!   item, `altr` group with the base) and reads it back as HDR (LR-HDR-EXPORT).
//!
//! Elsewhere, or without the Swift compiler, [`available`] is false and callers keep their other
//! path; nothing here panics.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(target_os = "macos")]
const SWIFT: &str = include_str!("imageio.swift");

/// How long one conversion may take (a full-size photo takes about a second).
const TIMEOUT: Duration = Duration::from_secs(120);

/// ImageIO can serve: macOS, with the Swift compiler or the helper already built, unless
/// `LIGHTCRAFT_NO_IMAGEIO` is set.
pub fn available() -> bool {
    cfg!(target_os = "macos")
        && std::env::var_os("LIGHTCRAFT_NO_IMAGEIO").is_none()
        && (std::path::Path::new("/usr/bin/swiftc").is_file() || helper_path().is_some_and(|p| p.is_file()))
}

fn helper_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::hash::{Hash, Hasher};
        let home = std::env::var_os("HOME")?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        SWIFT.hash(&mut h);
        Some(PathBuf::from(home).join(format!("Library/Application Support/LightCraft/bin/lc-imageio-{:016x}", h.finish())))
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// The helper binary, built from [`SWIFT`] on first use (rebuilt when it changes).
fn helper() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _one = BUILD.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let bin = helper_path().ok_or("no HOME folder")?;
        if bin.is_file() {
            return Ok(bin);
        }
        let dir = bin.parent().ok_or("no support folder")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let tag = format!("{}-{:?}", std::process::id(), std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
        let src = dir.join(format!("lc-imageio-{tag}.swift"));
        std::fs::write(&src, SWIFT).map_err(|e| format!("{}: {e}", src.display()))?;
        let part = dir.join(format!("lc-imageio-{tag}.part"));
        let out = Command::new("/usr/bin/swiftc").args(["-O", "-o"]).arg(&part).arg(&src).output();
        let _ = std::fs::remove_file(&src);
        let out = out.map_err(|e| format!("the Swift compiler isn't available ({e}): AVIF gain maps need it (xcode-select --install)"))?;
        if !out.status.success() || !part.is_file() {
            let _ = std::fs::remove_file(&part);
            return Err(format!(
                "building the ImageIO helper failed: {}",
                String::from_utf8_lossy(&out.stderr).chars().take(300).collect::<String>()
            ));
        }
        std::fs::rename(&part, &bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        Ok(bin)
    }
    #[cfg(not(target_os = "macos"))]
    Err("AVIF gain maps are written by macOS's ImageIO; not available here".into())
}

/// A scratch file in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(ext: &str) -> Scratch {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Scratch(std::env::temp_dir().join(format!("lc-imageio-{}-{n}.{ext}", std::process::id())))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Re-encode `jpeg` (an ISO 21496-1 gain-map JPEG: SDR base, metadata and gain map) as an AVIF
/// with the same base and gain map, at `quality` (1–100).
pub fn avif_gain_map(jpeg: &[u8], quality: u8) -> Result<Vec<u8>, String> {
    if !available() {
        return Err("AVIF gain maps are written by macOS's ImageIO; not available here".into());
    }
    let bin = helper()?;
    let input = Scratch::new("jpg");
    let output = Scratch::new("avif");
    std::fs::write(&input.0, jpeg).map_err(|e| format!("{}: {e}", input.0.display()))?;
    let mut child = Command::new(&bin)
        .arg("gainmap-avif")
        .arg(&input.0)
        .arg(&output.0)
        .arg(quality.clamp(1, 100).to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ImageIO helper: {e}"))?;
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("writing the AVIF took longer than {} s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(format!("ImageIO helper: {e}")),
        }
    };
    if !status.success() {
        let mut msg = String::new();
        if let Some(mut e) = child.stderr.take() {
            use std::io::Read;
            let _ = e.read_to_string(&mut msg);
        }
        let msg = msg.trim().chars().take(300).collect::<String>();
        return Err(match status.code() {
            Some(2) => "the gain map JPEG has no ISO gain map ImageIO can read".to_string(),
            Some(3) => "this macOS can't write AVIF".to_string(),
            _ if msg.is_empty() => format!("writing the AVIF failed ({status})"),
            _ => format!("writing the AVIF failed: {msg}"),
        });
    }
    let bytes = std::fs::read(&output.0).map_err(|e| format!("{}: {e}", output.0.display()))?;
    if bytes.len() < 32 || bytes.get(4..8) != Some(b"ftyp") {
        return Err("ImageIO wrote no AVIF".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_input_without_a_gain_map() {
        if !available() {
            return;
        }
        // a plain JPEG: ImageIO reads it, but it carries no gain map
        let img = lightcraft_raster::Rgba8::filled(16, 16, [120, 130, 140, 255]);
        let jpeg = lightcraft_codecs::encode_jpeg(
            &lightcraft_codecs::EncodeImage::rgba8(&img),
            90,
            lightcraft_codecs::ChromaSubsampling::S444,
            &lightcraft_codecs::EncodeMeta::default(),
        )
        .expect("jpeg");
        let e = avif_gain_map(&jpeg, 80).expect_err("no gain map");
        assert!(e.contains("no ISO gain map"), "{e}");
        assert!(avif_gain_map(b"not an image", 80).is_err());
    }
}
