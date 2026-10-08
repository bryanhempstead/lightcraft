//! macOS system-decoder fallback (Bryan's fork; see `docs/macos-decode.md`).
//!
//! Files the pure-Rust decoders can't read — HEIC/HEIF, AVIF, JPEG 2000, OpenEXR, TGA, … and raw
//! variants the raw crate doesn't decode — are converted by macOS's own ImageIO through
//! `/usr/bin/sips` (a tool that ships with the OS: no C code is compiled into LightCraft) to a
//! TIFF in a cache keyed by the file's path, size and modification time. That TIFF is decoded by
//! the normal TIFF path. Originals are only ever read; nothing is written next to them.
//!
//! Such a photo is marked through [`lightcraft_catalog::Photo::preview_only`] with a reason that
//! starts with [`MARK`]: the UI says "Decoded by macOS", and develop treats it as a rendered
//! (display-referred) source, like a TIFF, not as a raw.
//!
//! On other platforms (and on macOS without `sips`) nothing here is available and every file
//! behaves as before.

use std::path::{Path, PathBuf};

use lightcraft_pipeline::SourceInfo;
use lightcraft_raster::Rgb32f;

use crate::media::ProbeInfo;

/// Start of the `preview_only` reason of a photo decoded by macOS.
pub const MARK: &str = "decoded by macOS";

/// Image extensions only macOS's ImageIO reads (imported when [`available`]). The pure-Rust
/// formats are in [`crate::import::EXTENSIONS`].
pub const EXTENSIONS: &[&str] =
    &["heif", "hif", "heics", "avci", "jp2", "j2k", "jpf", "jpx", "exr", "tga", "dds", "sgi", "pbm", "pgm", "ppm", "mpo", "psb"];

/// Raw extensions the raw crate doesn't recognise but ImageIO renders (imported as raws).
pub const RAW_EXTENSIONS: &[&str] = &["crw", "mrw", "3fr", "fff", "iiq", "mos", "erf", "dcr", "kdc", "srw", "srf", "sr2", "nefx", "axr", "dxo"];

/// The size the cache may grow to before the least recently used conversions are removed.
const CACHE_BUDGET: u64 = 4 << 30;

/// A conversion that takes longer than this is abandoned (a damaged file can make sips hang).
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// How long a failed conversion is remembered (not retried).
const FAILED_FOR: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

const SIPS: &str = "/usr/bin/sips";

/// macOS decoding is possible here.
pub fn available() -> bool {
    cfg!(target_os = "macos") && Path::new(SIPS).is_file() && std::env::var_os("LIGHTCRAFT_NO_MACOS_DECODE").is_none()
}

/// The extension is one only macOS reads (and it is [`available`]).
pub fn handles_extension(ext: &str) -> bool {
    available() && EXTENSIONS.iter().chain(RAW_EXTENSIONS).any(|e| e.eq_ignore_ascii_case(ext))
}

/// Why `bytes` (the file at `path`) must be decoded by macOS, or `None` when the pure-Rust decoders
/// handle it (or macOS decoding isn't [`available`]). The reason starts with [`MARK`].
pub fn route(path: &str, bytes: &[u8]) -> Option<String> {
    if !available() {
        return None;
    }
    if lightcraft_raw::probe(bytes).is_some() {
        return match lightcraft_raw::probe_info(bytes) {
            Ok(_) => None,
            Err(e) => Some(format!("{MARK} (no native decoder for this raw: {e})")),
        };
    }
    match lightcraft_codecs::sniff(bytes) {
        Some(f) if f.can_decode() => None,
        Some(f) => Some(format!("{MARK} (no pure-Rust {f:?} decoder)")),
        None => {
            let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            handles_extension(&ext).then(|| format!("{MARK} ({} file)", ext.to_uppercase()))
        }
    }
}

// ------------------------------------------------------------------------------------ cache

static CACHE_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Where conversions go: the open library's `System Decodes` folder (set when a library is
/// opened), `LIGHTCRAFT_SYSDECODE_CACHE`, else `~/Library/Caches/LightCraft/System Decodes`.
pub fn set_cache_dir(dir: Option<PathBuf>) {
    *CACHE_DIR.write().unwrap_or_else(std::sync::PoisonError::into_inner) = dir;
}

pub fn cache_dir() -> PathBuf {
    // tests open and delete libraries in parallel: theirs is per process
    if cfg!(test) {
        return std::env::temp_dir().join(format!("lc-sysdecode-test-{}", std::process::id()));
    }
    if let Some(d) = std::env::var_os("LIGHTCRAFT_SYSDECODE_CACHE") {
        return PathBuf::from(d);
    }
    if let Some(d) = CACHE_DIR.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone() {
        return d;
    }
    match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h).join("Library/Caches/LightCraft/System Decodes"),
        None => std::env::temp_dir().join("LightCraft System Decodes"),
    }
}

/// Cache key: path + size + modification time (a changed file converts again).
fn key(path: &Path) -> Result<String, String> {
    let m = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let id = format!("v1\0{}\0{}\0{mtime}", path.display(), m.len());
    Ok(lightcraft_preview::hash_bytes(id.as_bytes()).to_string())
}

/// One conversion per file at a time (the import probe, the thumbnail and the loupe can ask for
/// the same file at once).
fn key_lock(k: &str) -> std::sync::Arc<std::sync::Mutex<()>> {
    static LOCKS: std::sync::Mutex<Vec<(String, std::sync::Arc<std::sync::Mutex<()>>)>> = std::sync::Mutex::new(Vec::new());
    let mut locks = LOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|(_, l)| std::sync::Arc::strong_count(l) > 1);
    if let Some((_, l)) = locks.iter().find(|(n, _)| n == k) {
        return l.clone();
    }
    let l = std::sync::Arc::new(std::sync::Mutex::new(()));
    locks.push((k.to_string(), l.clone()));
    l
}

/// The file converted to a TIFF by macOS (cached).
pub fn convert(path: &Path) -> Result<PathBuf, String> {
    if !available() {
        return Err(format!("{}: this format needs macOS to decode", path.display()));
    }
    let k = key(path)?;
    let dir = cache_dir();
    let out = dir.join(format!("{k}.tif"));
    let lock = key_lock(&k);
    let _guard = lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if out.is_file() {
        // recently used: kept longest by the pruning
        if let Ok(f) = std::fs::File::options().append(true).open(&out) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Ok(out);
    }
    // a file macOS couldn't read recently isn't tried on every thumbnail and loupe load
    let failed = dir.join(format!("{k}.failed"));
    if let Ok(m) = std::fs::metadata(&failed)
        && m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < FAILED_FOR)
    {
        let why = std::fs::read_to_string(&failed).unwrap_or_default();
        return Err(format!("{}: macOS can't decode it either ({})", path.display(), why.trim()));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = dir.join(format!("{k}.{}.part.tif", std::process::id()));
    let r = run_sips(path, &tmp);
    let r = r.and_then(|()| match std::fs::metadata(&tmp) {
        Ok(m) if m.len() > 0 => std::fs::rename(&tmp, &out).map_err(|e| format!("{}: {e}", out.display())),
        _ => Err("macOS wrote no image".into()),
    });
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::write(&failed, &e);
        return Err(format!("{}: macOS can't decode it either ({e})", path.display()));
    }
    prune(&dir, &out);
    Ok(out)
}

fn run_sips(src: &Path, dst: &Path) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(SIPS)
        .args(["-s", "format", "tiff"])
        .arg(src)
        .arg("--out")
        .arg(dst)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("sips: {e}"))?;
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("sips took longer than {} s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(15)),
            Err(e) => return Err(format!("sips: {e}")),
        }
    };
    let mut err = String::new();
    if let Some(mut s) = child.stderr.take() {
        use std::io::Read;
        let _ = s.read_to_string(&mut err);
    }
    let err = err.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("Try ")).collect::<Vec<_>>().join("; ");
    if !status.success() {
        return Err(if err.is_empty() { format!("sips failed ({status})") } else { err });
    }
    // sips can exit 0 without writing anything ("Error: Unable to render source image")
    if !dst.is_file() {
        return Err(if err.is_empty() { "sips wrote nothing".into() } else { err });
    }
    Ok(())
}

/// Keep the cache within [`CACHE_BUDGET`]: least recently used conversions go first (never
/// `keep`); leftovers of interrupted conversions older than an hour go too.
fn prune(dir: &Path, keep: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = e.metadata() else { continue };
        let t = m.modified().unwrap_or(now);
        if p.extension().is_some_and(|x| x == "failed") {
            if now.duration_since(t).is_ok_and(|d| d > FAILED_FOR) {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        if p.to_string_lossy().ends_with(".part.tif") {
            if now.duration_since(t).is_ok_and(|d| d.as_secs() > 3600) {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        if m.is_file() && p.extension().is_some_and(|x| x == "tif") {
            files.push((t, m.len(), p));
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= CACHE_BUDGET {
            break;
        }
        if p != keep && std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

// ---------------------------------------------------------------------------- probe / load

/// Describe a file macOS decodes (import): size from the conversion, metadata from the file
/// itself, `preview_only` = `why` (see [`route`]).
pub fn probe(path: &str, bytes: &[u8], why: String) -> Result<ProbeInfo, String> {
    let tif = convert(Path::new(path))?;
    let tbytes = std::fs::read(&tif).map_err(|e| format!("{}: {e}", tif.display()))?;
    let d = lightcraft_codecs::decode(&tbytes, lightcraft_codecs::DecodeOptions::fit(64, 64)).map_err(|e| format!("{path}: {e}"))?;
    let (mut w, mut h) = (d.source_width, d.source_height);
    if lightcraft_geom::Orientation::from_exif(d.orientation).swaps_axes() {
        std::mem::swap(&mut w, &mut h);
    }
    // the file's own metadata; where our reader can't parse the container (HEIF), the copy macOS
    // wrote carries the same EXIF
    let mut m = lightcraft_meta::extract(bytes);
    if m.make.is_none() && m.model.is_none() && m.capture_time.is_none() {
        m = lightcraft_meta::extract(&tbytes);
    }
    let (meta, captured) = super::meta_of(&m);
    let raw = lightcraft_raw::probe(bytes).is_some() || RAW_EXTENSIONS.iter().any(|e| super::ext_upper(path).eq_ignore_ascii_case(e));
    Ok(ProbeInfo {
        width: w,
        height: h,
        format: match super::ext_upper(path).as_str() {
            "" => "IMAGE".into(),
            e => e.to_string(),
        },
        kind: if raw { lightcraft_catalog::MediaKind::Raw } else { lightcraft_catalog::MediaKind::Image },
        file_size: bytes.len() as u64,
        captured,
        meta,
        as_shot_wb: None,
        content_hash: Some(lightcraft_preview::hash_bytes(bytes).to_string()),
        embedded_lens: None,
        xmp: if raw { lightcraft_meta::embedded(bytes).xmp } else { None },
        preview_only: Some(why),
    })
}

/// Load a file macOS decodes: its cached conversion through the TIFF path (a rendered source).
pub fn load(path: &str, max_edge: usize) -> Result<(Rgb32f, SourceInfo), String> {
    let tif = convert(Path::new(path))?;
    let bytes = std::fs::read(&tif).map_err(|e| format!("{}: {e}", tif.display()))?;
    let (img, _) = super::load_vec(bytes, max_edge).map_err(|e| format!("{path}: {e}"))?;
    Ok((img, SourceInfo::default()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lc-sysdecode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A 48 × 32 gradient PNG, converted to `ext` by macOS (the test's own fixture, never a user's photo).
    fn fixture(d: &Path, ext: &str) -> PathBuf {
        let (w, h) = (48usize, 32usize);
        let data: Vec<[u8; 4]> = (0..w * h).map(|i| [(i % w * 5) as u8, (i / w * 7) as u8, 90, 255]).collect();
        let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
        let png = d.join("src.png");
        std::fs::write(&png, lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &Default::default()).unwrap()).unwrap();
        let out = d.join(format!("fixture.{ext}"));
        let st = std::process::Command::new(SIPS).args(["-s", "format", ext]).arg(&png).arg("--out").arg(&out).output().unwrap();
        assert!(out.is_file(), "sips made no {ext}: {st:?}");
        out
    }

    #[test]
    fn pure_rust_formats_never_route_to_macos() {
        let png =
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&lightcraft_raster::Rgba8::new(4, 4)), &Default::default()).unwrap();
        assert_eq!(route("a.png", &png), None);
        assert_eq!(route("notes.txt", b"hello"), None);
        assert!(!handles_extension("txt"));
    }

    #[test]
    fn heic_avif_and_tga_decode_through_macos() {
        if !available() {
            eprintln!("skip: no macOS sips");
            return;
        }
        let d = dir("formats");
        let (loader, probe) = crate::files::fs_hooks();
        for ext in ["heic", "avif", "tga"] {
            let f = fixture(&d, ext);
            let path = f.to_string_lossy().to_string();
            let why = route(&path, &std::fs::read(&f).unwrap()).unwrap_or_else(|| panic!("{ext}: not routed"));
            assert!(why.starts_with(MARK), "{why}");
            let info = probe(&path).unwrap();
            assert_eq!((info.width, info.height, info.kind), (48, 32, lightcraft_catalog::MediaKind::Image), "{ext}");
            assert!(info.preview_only.as_deref().is_some_and(|w| w.starts_with(MARK)));
            assert_eq!(info.format, ext.to_uppercase());
            let (img, src) = loader(&path, 1024).unwrap();
            assert_eq!((img.width, img.height), (48, 32));
            assert!(!src.raw);
            // the gradient survived: left darker than right in red
            let (l, r) = (img.data[16 * 48 + 2][0], img.data[16 * 48 + 45][0]);
            assert!(r > l + 0.05, "{ext}: {l} {r}");
            // cached: a second probe converts nothing new
            let n = std::fs::read_dir(cache_dir()).unwrap().count();
            probe(&path).unwrap();
            assert_eq!(std::fs::read_dir(cache_dir()).unwrap().count(), n);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        if !available() {
            return;
        }
        let d = dir("damaged");
        let f = fixture(&d, "heic");
        let bytes = std::fs::read(&f).unwrap();
        let (loader, probe) = crate::files::fs_hooks();
        for (name, data) in [("cut.heic", &bytes[..bytes.len() / 3]), ("junk.heif", &b"not an image at all"[..]), ("empty.hif", &[][..])] {
            let p = d.join(name);
            std::fs::write(&p, data).unwrap();
            let p = p.to_string_lossy().to_string();
            assert!(probe(&p).is_err(), "{name}");
            assert!(loader(&p, 256).is_err(), "{name}");
            // remembered: not handed to sips again
            assert!(cache_dir().join(format!("{}.failed", key(Path::new(&p)).unwrap())).is_file(), "{name}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn import_accepts_heic_and_shows_it_decoded_by_macos() {
        if !available() {
            return;
        }
        let d = dir("import");
        fixture(&d, "heic");
        std::fs::remove_file(d.join("src.png")).unwrap();
        let mut s = crate::Session::new().with_fs();
        let r = s.execute("library.import", &serde_json::json!({"paths": [d.to_string_lossy()]})).unwrap();
        assert_eq!(r["imported"].as_array().map(Vec::len), Some(1), "{r}");
        let p = s.catalog.photos().next().unwrap();
        assert_eq!((p.width, p.height, p.format.as_str()), (48, 32, "HEIC"));
        assert!(p.preview_only.as_deref().is_some_and(|w| w.starts_with(MARK)));
        assert!(!p.develops_raw());
        let _ = std::fs::remove_dir_all(&d);
    }
}
