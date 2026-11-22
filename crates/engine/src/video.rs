//! LR-VID-*: videos — what a clip is (probe), its frames (poster / thumbnail / loupe / capture),
//! a playback stream for the Loupe, and trimmed, edited H.264 export.
//!
//! Nothing here is decoded by LightCraft itself; two backends do it, picked per call:
//! - **ffmpeg / ffprobe** when installed (PATH, Homebrew's folders; `LIGHTCRAFT_FFMPEG` /
//!   `LIGHTCRAFT_FFPROBE` point at others). Audio plays through `ffplay` when it is there too.
//! - **macOS AVFoundation** otherwise, through a small helper of our own (`video.swift`, built once
//!   with `swiftc` into LightCraft's support folder, like the face finder): probe, frames and the
//!   playback stream. Its export is `/usr/bin/avconvert`: trims, but can't apply edits (the result
//!   says so).
//!
//! With neither, videos still import (from what the file system says) but show no frames, and
//! every command says what to install; nothing panics.
//!
//! A video's catalogued source path can carry the poster frame's time ([`frame_source`]), so the
//! normal file loader, thumbnail and preview caches render the poster through the develop
//! pipeline like any photo: Quick Develop edits show on it. Playback and export apply only the
//! settings Lightroom Classic applies to video ([`video_settings`]).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use lightcraft_develop::DevelopSettings;
use lightcraft_pipeline::{Quality, RenderRequest, SourceInfo};
use lightcraft_raster::{Rgb32f, Rgba8};
use serde::{Deserialize, Serialize};

/// Video file extensions the library imports.
pub const EXTENSIONS: &[&str] = &["mov", "mp4", "m4v", "avi", "mts", "m2ts", "mkv", "3gp", "mpg", "mpeg", "webm"];

/// How long a probe or a single frame may take before it is abandoned (a damaged file can hang a
/// decoder).
const QUICK_TIMEOUT: Duration = Duration::from_secs(60);

/// Separates a source path from the poster time in [`frame_source`] (never part of a real path).
const FRAME_SEP: char = '\u{0}';

pub fn is_video_ext(ext: &str) -> bool {
    EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// `path` names a video (by its extension).
pub fn is_video_path(path: &str) -> bool {
    Path::new(split_source(path).0).extension().is_some_and(|e| is_video_ext(&e.to_string_lossy()))
}

/// The source string the file loader gets for a video whose poster frame is at `poster` seconds.
pub fn frame_source(path: &str, poster: Option<f64>) -> String {
    match poster.filter(|t| t.is_finite() && *t > 0.0) {
        Some(t) => format!("{path}{FRAME_SEP}t={t:.3}"),
        None => path.to_string(),
    }
}

/// The file and the frame time (0 = the first frame) of a [`frame_source`] string.
pub fn split_source(s: &str) -> (&str, f64) {
    match s.split_once(FRAME_SEP) {
        Some((p, t)) => (p, t.strip_prefix("t=").and_then(|t| t.parse::<f64>().ok()).filter(|t| t.is_finite() && *t >= 0.0).unwrap_or(0.0)),
        None => (s, 0.0),
    }
}

// ------------------------------------------------------------------------------------- tools

/// Which decoder serves videos on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    Ffmpeg,
    AvFoundation,
    None,
}

fn find_tool(env: &str, name: &str) -> Option<PathBuf> {
    if std::env::var_os("LIGHTCRAFT_NO_FFMPEG").is_some() {
        return None;
    }
    if let Some(p) = std::env::var_os(env) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

pub fn ffmpeg() -> Option<PathBuf> {
    find_tool("LIGHTCRAFT_FFMPEG", "ffmpeg")
}
pub fn ffprobe() -> Option<PathBuf> {
    find_tool("LIGHTCRAFT_FFPROBE", "ffprobe")
}
pub fn ffplay() -> Option<PathBuf> {
    find_tool("LIGHTCRAFT_FFPLAY", "ffplay")
}

/// AVFoundation can serve (macOS, with the Swift compiler or the helper already built).
pub fn avfoundation() -> bool {
    cfg!(target_os = "macos")
        && std::env::var_os("LIGHTCRAFT_NO_AVFOUNDATION").is_none()
        && (Path::new("/usr/bin/swiftc").is_file() || helper_path().is_some_and(|p| p.is_file()))
}

pub fn backend() -> Backend {
    if ffmpeg().is_some() && ffprobe().is_some() {
        Backend::Ffmpeg
    } else if avfoundation() {
        Backend::AvFoundation
    } else {
        Backend::None
    }
}

/// What to tell someone whose machine can't decode video.
pub const INSTALL_HINT: &str = "videos need ffmpeg (Homebrew: brew install ffmpeg) or macOS's AVFoundation";

/// The tools found, for `video.status` and the UI's notice.
pub fn status() -> serde_json::Value {
    let p = |o: Option<PathBuf>| o.map(|p| p.display().to_string());
    serde_json::json!({
        "backend": backend(),
        "ffmpeg": p(ffmpeg()),
        "ffprobe": p(ffprobe()),
        "ffplay": p(ffplay()),
        "avfoundation": avfoundation(),
        "avconvert": Path::new(AVCONVERT).is_file(),
        "notice": match backend() {
            Backend::Ffmpeg => None,
            Backend::AvFoundation => Some("Videos are decoded by macOS (AVFoundation): playback is silent and exports can't carry edits. Install ffmpeg (brew install ffmpeg) for sound and edited exports."),
            Backend::None => Some(INSTALL_HINT),
        },
    })
}

const AVCONVERT: &str = "/usr/bin/avconvert";

/// Run a command to completion (stdout collected), killing it after `timeout`.
fn run(cmd: &mut Command, timeout: Duration) -> Result<Vec<u8>, String> {
    let name = cmd.get_program().to_string_lossy().rsplit('/').next().unwrap_or("tool").to_string();
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("{name}: {e}"))?;
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_end(&mut v);
        }
        v
    });
    let t_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(o) = err.as_mut() {
            let _ = o.read_to_end(&mut v);
        }
        v
    });
    let t0 = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if t0.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} took longer than {} s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{name}: {e}")),
        }
    };
    let stdout = t_out.join().map_err(|_| format!("{name}: reading its output failed"))?;
    let stderr = t_err.join().unwrap_or_default();
    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr);
        let msg: String = msg.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ").chars().take(400).collect();
        return Err(if msg.is_empty() { format!("{name} failed ({status})") } else { format!("{name}: {msg}") });
    }
    Ok(stdout)
}

// ------------------------------------------------------------------------- AVFoundation helper

#[cfg(target_os = "macos")]
const SWIFT: &str = include_str!("video.swift");

fn helper_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        use std::hash::{Hash, Hasher};
        let home = std::env::var_os("HOME")?;
        let mut h = std::collections::hash_map::DefaultHasher::new();
        SWIFT.hash(&mut h);
        Some(PathBuf::from(home).join(format!("Library/Application Support/LightCraft/bin/lc-video-{:016x}", h.finish())))
    }
    #[cfg(not(target_os = "macos"))]
    None
}

/// The AVFoundation helper binary, built from [`SWIFT`] on first use (rebuilt when it changes).
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
        let src = dir.join(format!("lc-video-{}.swift", std::process::id()));
        std::fs::write(&src, SWIFT).map_err(|e| format!("{}: {e}", src.display()))?;
        let part = dir.join(format!("lc-video-{}.part", std::process::id()));
        let out = Command::new("/usr/bin/swiftc").args(["-O", "-o"]).arg(&part).arg(&src).output();
        let _ = std::fs::remove_file(&src);
        let out = out.map_err(|e| format!("the Swift compiler isn't available ({e}); {INSTALL_HINT}"))?;
        if !out.status.success() || !part.is_file() {
            let _ = std::fs::remove_file(&part);
            return Err(format!("building the video helper failed: {}", String::from_utf8_lossy(&out.stderr).chars().take(300).collect::<String>()));
        }
        std::fs::rename(&part, &bin).map_err(|e| format!("{}: {e}", bin.display()))?;
        Ok(bin)
    }
    #[cfg(not(target_os = "macos"))]
    Err(INSTALL_HINT.into())
}

// ------------------------------------------------------------------------------------- probe

/// What a clip is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    /// Upright size (a rotated phone clip is taller than wide).
    pub width: u32,
    pub height: u32,
    /// Seconds.
    pub duration: f64,
    /// Frames per second (25 when the file doesn't say).
    pub fps: f64,
    pub codec: String,
    pub audio: bool,
    /// When it was recorded (ISO 8601 local form, no zone), when the file says.
    pub created: Option<String>,
}

fn parse_rate(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/').unwrap_or((s, "1"));
    let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
    let r = a / b;
    (r.is_finite() && r > 0.5 && r < 1000.0).then_some(r)
}

/// `2024-05-01T12:00:00.000000Z` → `2024-05-01T12:00:00`.
fn iso_seconds(s: &str) -> Option<String> {
    let s = s.trim();
    let head = s.get(..19)?;
    let ok = head.char_indices().all(|(i, c)| match i {
        4 | 7 => c == '-',
        10 => c == 'T' || c == ' ',
        13 | 16 => c == ':',
        _ => c.is_ascii_digit(),
    });
    ok.then(|| head.replacen(' ', "T", 1)).filter(|h| !h.starts_with("0000") && !h.starts_with("1904") && !h.starts_with("1970"))
}

/// Read ffprobe's JSON (`-show_format -show_streams`).
pub fn parse_ffprobe(json: &[u8]) -> Result<VideoInfo, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("ffprobe output: {e}"))?;
    let streams = v["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1).ok_or("no video stream")?;
    let num = |x: &serde_json::Value| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()));
    let (mut w, mut h) = (num(&video["width"]).unwrap_or(0.0), num(&video["height"]).unwrap_or(0.0));
    let rotation = video["side_data_list"]
        .as_array()
        .and_then(|l| l.iter().find_map(|d| num(&d["rotation"])))
        .or_else(|| num(&video["tags"]["rotate"]))
        .unwrap_or(0.0);
    if ((rotation.abs() as i64) % 180) == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    if !(w >= 1.0 && h >= 1.0 && w <= 65536.0 && h <= 65536.0) {
        return Err("the video stream has no size".into());
    }
    let duration = num(&v["format"]["duration"]).or_else(|| num(&video["duration"])).filter(|d| d.is_finite() && *d >= 0.0).unwrap_or(0.0);
    let fps = video["avg_frame_rate"].as_str().and_then(parse_rate).or_else(|| video["r_frame_rate"].as_str().and_then(parse_rate)).unwrap_or(25.0);
    let created = v["format"]["tags"]["creation_time"].as_str().or_else(|| video["tags"]["creation_time"].as_str()).and_then(iso_seconds);
    Ok(VideoInfo {
        width: w as u32,
        height: h as u32,
        duration,
        fps,
        codec: video["codec_name"].as_str().unwrap_or("").to_string(),
        audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        created,
    })
}

fn parse_helper_probe(json: &[u8]) -> Result<VideoInfo, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("video helper output: {e}"))?;
    let w = v["width"].as_f64().unwrap_or(0.0);
    let h = v["height"].as_f64().unwrap_or(0.0);
    if !(w >= 1.0 && h >= 1.0 && w <= 65536.0 && h <= 65536.0) {
        return Err("the video has no size".into());
    }
    Ok(VideoInfo {
        width: w as u32,
        height: h as u32,
        duration: v["duration"].as_f64().filter(|d| d.is_finite() && *d >= 0.0).unwrap_or(0.0),
        fps: v["fps"].as_f64().filter(|f| f.is_finite() && *f > 0.5 && *f < 1000.0).unwrap_or(25.0),
        codec: v["codec"].as_str().unwrap_or("").to_string(),
        audio: v["audio"].as_bool().unwrap_or(false),
        created: v["created"].as_str().and_then(iso_seconds),
    })
}

/// What `path` is (size, length, rate, codec, sound).
pub fn probe(path: &str) -> Result<VideoInfo, String> {
    probe_with(backend(), path)
}

/// [`probe`] through a given backend.
pub fn probe_with(b: Backend, path: &str) -> Result<VideoInfo, String> {
    let path = split_source(path).0;
    if !Path::new(path).is_file() {
        return Err(format!("{path}: not found"));
    }
    match b {
        Backend::Ffmpeg => {
            let bin = ffprobe().ok_or(INSTALL_HINT)?;
            let out =
                run(Command::new(bin).args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"]).arg(path), QUICK_TIMEOUT)?;
            parse_ffprobe(&out).map_err(|e| format!("{path}: {e}"))
        }
        Backend::AvFoundation => {
            let out = run(Command::new(helper()?).arg("probe").arg(path), QUICK_TIMEOUT)?;
            parse_helper_probe(&out).map_err(|e| format!("{path}: {e}"))
        }
        Backend::None => Err(format!("{path}: {INSTALL_HINT}")),
    }
}

/// Import's description of a video: kind, size, length, capture time. The content hash covers the
/// file's size and its first and last MiB (not 32 digits, so Import → Copy doesn't verify copies
/// against it as a whole-file hash).
pub fn probe_info(path: &str) -> Result<crate::media::ProbeInfo, String> {
    let info = probe(path)?;
    let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(crate::media::ProbeInfo {
        width: info.width,
        height: info.height,
        format: Path::new(path).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default(),
        kind: lightcraft_catalog::MediaKind::Video,
        file_size,
        captured: info.created.clone(),
        duration: Some(info.duration).filter(|d| *d > 0.0),
        content_hash: quick_hash(path, file_size).map(|h| format!("video:{h}")),
        ..Default::default()
    })
}

fn quick_hash(path: &str, len: u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    const PART: u64 = 1 << 20;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(2 * PART as usize + 8);
    buf.extend_from_slice(&len.to_le_bytes());
    let mut head = Vec::new();
    (&mut f).take(PART).read_to_end(&mut head).ok()?;
    buf.extend_from_slice(&head);
    if len > 2 * PART {
        f.seek(SeekFrom::Start(len - PART)).ok()?;
        let mut tail = Vec::new();
        f.take(PART).read_to_end(&mut tail).ok()?;
        buf.extend_from_slice(&tail);
    }
    Some(lightcraft_preview::hash_bytes(&buf).to_string())
}

// ------------------------------------------------------------------------------------ frames

/// The upright frame at `secs` as PNG bytes, no larger than `max_edge` (never upscaled).
pub fn frame_png(path: &str, secs: f64, max_edge: usize) -> Result<Vec<u8>, String> {
    frame_png_with(backend(), path, secs, max_edge)
}

/// [`frame_png`] through a given backend.
pub fn frame_png_with(b: Backend, path: &str, secs: f64, max_edge: usize) -> Result<Vec<u8>, String> {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let m = max_edge.clamp(16, 65536);
    match b {
        Backend::Ffmpeg => {
            let bin = ffmpeg().ok_or(INSTALL_HINT)?;
            let once = |t: f64| {
                let mut c = Command::new(&bin);
                c.args(["-nostdin", "-v", "error", "-ss", &format!("{t:.3}"), "-i"]).arg(path).args(["-frames:v", "1", "-an"]);
                if max_edge < 65536 {
                    c.args(["-vf", &format!("scale='min(iw,{m})':'min(ih,{m})':force_original_aspect_ratio=decrease")]);
                }
                c.args(["-f", "image2pipe", "-vcodec", "png", "-"]);
                run(&mut c, QUICK_TIMEOUT)
            };
            match once(secs) {
                Ok(v) if !v.is_empty() => Ok(v),
                // past the last frame (a poster at the very end, a rounded duration): the first one
                Ok(_) if secs > 0.0 => once(0.0).and_then(|v| if v.is_empty() { Err(format!("{path}: no frames")) } else { Ok(v) }),
                Ok(_) => Err(format!("{path}: no frames")),
                Err(e) => Err(e),
            }
        }
        Backend::AvFoundation => {
            let tmp = std::env::temp_dir().join(format!(
                "lc-frame-{}-{:x}.png",
                std::process::id(),
                lightcraft_preview::hash_bytes(format!("{path}{secs}{m}").as_bytes()).0
            ));
            let edge = if max_edge < 65536 { m.to_string() } else { "0".to_string() };
            let r = run(Command::new(helper()?).arg("frame").arg(path).arg(format!("{secs:.3}")).arg(edge).arg(&tmp), QUICK_TIMEOUT)
                .and_then(|_| std::fs::read(&tmp).map_err(|e| format!("{}: {e}", tmp.display())));
            let _ = std::fs::remove_file(&tmp);
            r
        }
        Backend::None => Err(format!("{path}: {INSTALL_HINT}")),
    }
}

/// The frame at `secs` as display sRGB, no larger than `max_edge`, with the clip's edits when
/// `develop` has any ([`has_edits`]): what the Loupe shows while scrubbing, what Capture Frame
/// writes.
pub fn frame_rgba(path: &str, secs: f64, max_edge: usize, develop: Option<&DevelopSettings>, quality: Quality) -> Result<Rgba8, String> {
    let png = frame_png(path, secs, max_edge)?;
    let d = lightcraft_codecs::decode(&png, lightcraft_codecs::DecodeOptions::default()).map_err(|e| format!("{path}: {e}"))?;
    let img = d.to_srgb8();
    Ok(match develop.filter(|s| has_edits(s)) {
        Some(s) => develop_frame(&img, s, quality),
        None => img,
    })
}

/// The file loader's view of a video ([`frame_source`]): its frame decoded like a rendered photo.
pub fn load_frame(source: &str, max_edge: usize) -> Result<(Rgb32f, SourceInfo), String> {
    let (path, t) = split_source(source);
    let png = frame_png(path, t, max_edge)?;
    crate::files::load_vec(png, max_edge)
}

// ------------------------------------------------------------------------- edits on video

/// The develop settings Lightroom Classic applies to video (Quick Develop's and a preset's
/// supported part): treatment / black & white mix, white balance, exposure, contrast, whites,
/// blacks, tone curve, vibrance, saturation and split toning. Everything else is left out.
pub fn video_settings(s: &DevelopSettings) -> DevelopSettings {
    let d = DevelopSettings::default();
    let mut v = d.clone();
    v.version = s.version;
    v.treatment = s.treatment;
    v.bw_mix = s.bw_mix;
    v.wb = s.wb;
    v.light.exposure = s.light.exposure;
    v.light.contrast = s.light.contrast;
    v.light.whites = s.light.whites;
    v.light.blacks = s.light.blacks;
    v.curve = s.curve.clone();
    v.color.vibrance = s.color.vibrance;
    v.color.saturation = s.color.saturation;
    v.grading = s.grading;
    v
}

/// Any setting [`video_settings`] keeps differs from the defaults (frames go through the
/// pipeline only then).
pub fn has_edits(s: &DevelopSettings) -> bool {
    video_settings(s) != video_settings(&DevelopSettings::default())
}

/// The source a rendered 8-bit frame stands for (linear Rec.2020, taken back to scene-like values
/// as a JPEG is: `files::load_rendered`).
pub fn frame_to_source(img: &Rgba8) -> (Rgb32f, SourceInfo) {
    let m = lightcraft_color::SRGB.to_space(&lightcraft_color::REC2020).to_f32();
    let curve = lightcraft_pipeline::tone::rendered_reference();
    let mut lin = img.to_linear();
    for p in lin.data.iter_mut() {
        let q = [
            m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
            m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
            m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
        ];
        *p = match &curve {
            Some(c) => lightcraft_pipeline::tone::rendered_to_scene(c, q),
            None => q,
        };
    }
    (lin, SourceInfo { camera_tone: curve, ..SourceInfo::default() })
}

/// One frame with the video's edits applied (`video_settings` of `s`), same size.
pub fn develop_frame(img: &Rgba8, s: &DevelopSettings, quality: Quality) -> Rgba8 {
    let (src, info) = frame_to_source(img);
    let req = RenderRequest { quality, ..RenderRequest::fit(img.width.max(1), img.height.max(1)) };
    let out = lightcraft_pipeline::render(&src, &info, &video_settings(s), &req).image;
    if out.width == img.width && out.height == img.height {
        out
    } else {
        // (never expected: video settings have no crop) the stream's size must not change
        img.clone()
    }
}

/// An even output size no larger than `max_edge` for a `w × h` clip (encoders want even sizes).
pub fn fit_even(w: u32, h: u32, max_edge: u32) -> (u32, u32) {
    let (w, h) = (w.max(2) as f64, h.max(2) as f64);
    let s = (max_edge.max(2) as f64 / w.max(h)).min(1.0);
    let even = |v: f64| (((v * s) / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

// ---------------------------------------------------------------------------------- playback

/// One decoded frame of a playback stream: its time in the clip (seconds) and pixels.
pub struct Frame {
    pub t: f64,
    pub image: Rgba8,
}

/// Plays `from..to` of a clip at `w × h`: frames arrive on a worker thread (decoded, and developed
/// when the clip has edits) a few ahead of the clock; [`Player::frame_at`] hands out the one due.
/// Dropping it stops the decoder and the sound.
pub struct Player {
    rx: std::sync::mpsc::Receiver<Frame>,
    pending: Option<Frame>,
    decoder: Option<std::process::Child>,
    audio: Option<std::process::Child>,
    done: Arc<std::sync::atomic::AtomicBool>,
    error: Arc<std::sync::Mutex<Option<String>>>,
    pub from: f64,
    pub to: f64,
    /// Whether sound plays (ffplay found and the clip has sound).
    pub sound: bool,
}

impl Player {
    pub fn start(
        path: &str,
        info: &VideoInfo,
        from: f64,
        to: f64,
        size: (u32, u32),
        develop: Option<Arc<DevelopSettings>>,
        sound: bool,
    ) -> Result<Player, String> {
        Self::start_with(backend(), path, info, from, to, size, develop, sound)
    }

    /// [`Player::start`] through a given backend.
    #[allow(clippy::too_many_arguments)]
    pub fn start_with(
        b: Backend,
        path: &str,
        info: &VideoInfo,
        from: f64,
        to: f64,
        (w, h): (u32, u32),
        develop: Option<Arc<DevelopSettings>>,
        sound: bool,
    ) -> Result<Player, String> {
        let path = split_source(path).0;
        let from = if from.is_finite() { from.max(0.0) } else { 0.0 };
        let to = if to.is_finite() && to > from { to } else { info.duration.max(from) };
        let (w, h) = (w.clamp(2, 8192), h.clamp(2, 8192));
        let mut cmd = match b {
            Backend::Ffmpeg => {
                let mut c = Command::new(ffmpeg().ok_or(INSTALL_HINT)?);
                c.args(["-nostdin", "-v", "error", "-ss", &format!("{from:.3}"), "-i"]).arg(path).args([
                    "-t",
                    &format!("{:.3}", (to - from).max(0.04)),
                    "-an",
                    "-vf",
                    &format!("scale={w}:{h}:flags=bilinear,format=rgba"),
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgba",
                    "-",
                ]);
                c
            }
            Backend::AvFoundation => {
                let mut c = Command::new(helper()?);
                c.arg("stream").arg(path).arg(format!("{from:.3}")).arg(format!("{to:.3}")).arg(w.to_string()).arg(h.to_string());
                c
            }
            Backend::None => return Err(INSTALL_HINT.into()),
        };
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| format!("video decoder: {e}"))?;
        let mut out = child.stdout.take().ok_or("video decoder: no output")?;
        let (tx, rx) = std::sync::mpsc::sync_channel::<Frame>(4);
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let error = Arc::new(std::sync::Mutex::new(None));
        let (d2, e2) = (done.clone(), error.clone());
        let fps = if info.fps.is_finite() && info.fps > 0.5 { info.fps } else { 25.0 };
        std::thread::Builder::new()
            .name("lc-video-play".into())
            .spawn(move || {
                let len = (w as usize) * (h as usize) * 4;
                let mut buf = vec![0u8; len];
                let mut n: u64 = 0;
                loop {
                    if let Err(e) = out.read_exact(&mut buf) {
                        if e.kind() != std::io::ErrorKind::UnexpectedEof {
                            *e2.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(e.to_string());
                        }
                        break;
                    }
                    let Some(mut image) = Rgba8::from_bytes(w as usize, h as usize, &buf) else { break };
                    if let Some(s) = &develop {
                        image = develop_frame(&image, s, Quality::Draft);
                    }
                    if tx.send(Frame { t: from + n as f64 / fps, image }).is_err() {
                        break; // the player went away
                    }
                    n += 1;
                }
                d2.store(true, std::sync::atomic::Ordering::Release);
            })
            .map_err(|e| format!("video thread: {e}"))?;
        let audio = match (sound && info.audio && b == Backend::Ffmpeg, ffplay()) {
            (true, Some(bin)) => Command::new(bin)
                .args([
                    "-nodisp",
                    "-autoexit",
                    "-vn",
                    "-loglevel",
                    "quiet",
                    "-ss",
                    &format!("{from:.3}"),
                    "-t",
                    &format!("{:.3}", (to - from).max(0.04)),
                ])
                .arg(path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok(),
            _ => None,
        };
        let sound = audio.is_some();
        Ok(Player { rx, pending: None, decoder: Some(child), audio, done, error, from, to, sound })
    }

    /// The newest frame due at clip time `pos` (frames before it are skipped), if a new one is.
    pub fn frame_at(&mut self, pos: f64) -> Option<Frame> {
        let mut due = None;
        loop {
            if self.pending.is_none() {
                self.pending = self.rx.try_recv().ok();
            }
            match &self.pending {
                Some(f) if f.t <= pos + 0.002 => due = self.pending.take(),
                _ => break,
            }
        }
        due
    }

    /// No more frames will come (the range ended, or decoding failed: [`Player::error`]).
    pub fn finished(&self) -> bool {
        self.pending.is_none()
            && self.done.load(std::sync::atomic::Ordering::Acquire)
            && matches!(self.rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected))
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        for c in [self.decoder.as_mut(), self.audio.as_mut()].into_iter().flatten() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// ------------------------------------------------------------------------------------ export

/// Video export format (Lightroom Classic's Video section of Export).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoFormat {
    /// H.264 in an MP4: trimmed, with the edits.
    #[default]
    H264,
    /// The file as it is (no trim, no edits), like Lightroom's "Original".
    Original,
}

/// H.264 quality: the largest size and the encoder quality.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoQuality {
    /// The clip's own size, near-lossless.
    Max,
    /// Up to 1080p.
    #[default]
    High,
    /// Up to 720p.
    Medium,
    /// Up to 480p.
    Low,
}

impl VideoQuality {
    pub fn parse(s: &str) -> Option<VideoQuality> {
        match s.to_ascii_lowercase().as_str() {
            "max" | "maximum" => Some(VideoQuality::Max),
            "high" => Some(VideoQuality::High),
            "medium" => Some(VideoQuality::Medium),
            "low" => Some(VideoQuality::Low),
            _ => None,
        }
    }
    /// (largest short side, x264 CRF).
    fn params(self) -> (u32, u32) {
        match self {
            VideoQuality::Max => (u32::MAX, 16),
            VideoQuality::High => (1080, 20),
            VideoQuality::Medium => (720, 23),
            VideoQuality::Low => (480, 26),
        }
    }
    fn avconvert_preset(self) -> &'static str {
        match self {
            VideoQuality::Max => "PresetHighestQuality",
            VideoQuality::High => "Preset1920x1080",
            VideoQuality::Medium => "Preset1280x720",
            VideoQuality::Low => "Preset640x480",
        }
    }
}

/// How videos export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoExport {
    /// Export the video files themselves (else a selection's videos are skipped).
    pub include: bool,
    pub format: VideoFormat,
    pub quality: VideoQuality,
}

impl Default for VideoExport {
    fn default() -> Self {
        VideoExport { include: true, format: VideoFormat::H264, quality: VideoQuality::High }
    }
}

/// The output size for `quality` (even, never larger than the clip).
pub fn export_size(info: &VideoInfo, quality: VideoQuality) -> (u32, u32) {
    let (short, _) = quality.params();
    let s = info.width.min(info.height).max(1);
    let long = info.width.max(info.height).max(1);
    let max_edge = if short == u32::MAX || s <= short { long } else { ((long as f64) * short as f64 / s as f64).round() as u32 };
    fit_even(info.width, info.height, max_edge)
}

/// The file name an export of `path` gets (`.mp4` for H.264, the original's own otherwise).
pub fn export_name(path: &str, format: VideoFormat) -> String {
    let p = Path::new(split_source(path).0);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "video".into());
    match format {
        VideoFormat::H264 => format!("{stem}.mp4"),
        VideoFormat::Original => p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| format!("{stem}.mov")),
    }
}

/// Export `src` to `dst`: `range` = trim (start, end), `develop` = the clip's settings (applied
/// when [`has_edits`]). `progress(fraction)` returns false to cancel. Returns a notice when the
/// result differs from what was asked (AVFoundation can't apply edits).
pub fn export(
    src: &str,
    dst: &Path,
    info: &VideoInfo,
    range: (f64, Option<f64>),
    opts: VideoExport,
    develop: Option<&DevelopSettings>,
    progress: &dyn Fn(f64) -> bool,
) -> Result<Option<String>, String> {
    export_with(backend(), src, dst, info, range, opts, develop, progress)
}

/// [`export`] through a given backend.
#[allow(clippy::too_many_arguments)]
pub fn export_with(
    b: Backend,
    src: &str,
    dst: &Path,
    info: &VideoInfo,
    range: (f64, Option<f64>),
    opts: VideoExport,
    develop: Option<&DevelopSettings>,
    progress: &dyn Fn(f64) -> bool,
) -> Result<Option<String>, String> {
    let src = split_source(src).0;
    if let Some(dir) = dst.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    if opts.format == VideoFormat::Original {
        std::fs::copy(src, dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        progress(1.0);
        return Ok(None);
    }
    let (a, end) = range;
    let end = end.unwrap_or(info.duration).max(a + 0.04);
    let dur = end - a;
    let (w, h) = export_size(info, opts.quality);
    let (_, crf) = opts.quality.params();
    let edits = develop.filter(|s| has_edits(s));
    let part = dst.with_extension(format!("part.{}", dst.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "mp4".into())));
    let r = match b {
        Backend::Ffmpeg => {
            let bin = ffmpeg().ok_or(INSTALL_HINT)?;
            match edits {
                None => ffmpeg_transcode(&bin, src, &part, a, dur, (w, h), crf, progress).map(|()| None),
                Some(s) => ffmpeg_developed(&bin, src, &part, info, a, dur, (w, h), crf, s, progress).map(|()| None),
            }
        }
        Backend::AvFoundation if Path::new(AVCONVERT).is_file() => {
            let out = run(
                Command::new(AVCONVERT)
                    .args(["--preset", opts.quality.avconvert_preset(), "--replace", "--source"])
                    .arg(src)
                    .arg("--output")
                    .arg(&part)
                    .args(["--start", &format!("{a:.3}"), "--duration", &format!("{dur:.3}")]),
                Duration::from_secs(3600 * 4),
            );
            progress(1.0);
            out.map(|_| edits.map(|_| "exported without its edits: applying edits to video needs ffmpeg (brew install ffmpeg)".to_string()))
        }
        _ => Err(INSTALL_HINT.into()),
    };
    match r {
        Ok(notice) => {
            std::fs::rename(&part, dst).map_err(|e| format!("{}: {e}", dst.display()))?;
            Ok(notice)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

fn x264_args(crf: u32) -> Vec<String> {
    [
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-crf",
        &crf.to_string(),
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Wait for an ffmpeg that reports `-progress pipe:1`, feeding `progress`; kill it on cancel.
fn wait_progress(mut child: std::process::Child, total: f64, progress: &dyn Fn(f64) -> bool) -> Result<(), String> {
    use std::io::BufRead;
    let mut err = child.stderr.take();
    let t_err = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut v);
        }
        v
    });
    let mut cancelled = false;
    if let Some(out) = child.stdout.take() {
        for line in std::io::BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if let Some(us) =
                line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")).and_then(|v| v.trim().parse::<f64>().ok())
                && !progress((us / 1e6 / total.max(0.04)).clamp(0.0, 1.0))
            {
                cancelled = true;
                let _ = child.kill();
                break;
            }
        }
    }
    let status = child.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    let stderr = t_err.join().unwrap_or_default();
    if cancelled {
        return Err("cancelled".into());
    }
    if !status.success() {
        let msg = String::from_utf8_lossy(&stderr).lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ");
        return Err(format!("ffmpeg: {}", msg.chars().take(400).collect::<String>()));
    }
    progress(1.0);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ffmpeg_transcode(
    bin: &Path,
    src: &str,
    dst: &Path,
    a: f64,
    dur: f64,
    (w, h): (u32, u32),
    crf: u32,
    progress: &dyn Fn(f64) -> bool,
) -> Result<(), String> {
    let child = Command::new(bin)
        .args(["-nostdin", "-v", "error", "-y", "-ss", &format!("{a:.3}"), "-i"])
        .arg(src)
        .args(["-t", &format!("{dur:.3}"), "-map", "0:v:0", "-map", "0:a:0?", "-vf", &format!("scale={w}:{h}")])
        .args(x264_args(crf))
        .args(["-progress", "pipe:1", "-nostats"])
        .arg(dst)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    wait_progress(child, dur, progress)
}

/// Decode → develop every frame through the pipeline → encode, the sound copied from the source.
#[allow(clippy::too_many_arguments)]
fn ffmpeg_developed(
    bin: &Path,
    src: &str,
    dst: &Path,
    info: &VideoInfo,
    a: f64,
    dur: f64,
    (w, h): (u32, u32),
    crf: u32,
    s: &DevelopSettings,
    progress: &dyn Fn(f64) -> bool,
) -> Result<(), String> {
    use std::io::Write;
    // a frame buffer sized by what the file claims: capped (8K is 33 MP)
    if u64::from(w) * u64::from(h) > 8192 * 8192 {
        return Err(format!("{w} × {h} is larger than LightCraft exports video at"));
    }
    let fps = if info.fps.is_finite() && info.fps > 0.5 { info.fps } else { 25.0 };
    let mut dec = Command::new(bin)
        .args(["-nostdin", "-v", "error", "-ss", &format!("{a:.3}"), "-i"])
        .arg(src)
        .args(["-t", &format!("{dur:.3}"), "-an", "-vf", &format!("scale={w}:{h},format=rgba"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut enc = Command::new(bin)
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{w}x{h}"), "-r", &format!("{fps:.6}"), "-i", "-"])
        .args(["-ss", &format!("{a:.3}"), "-t", &format!("{dur:.3}"), "-i"])
        .arg(src)
        .args(["-map", "0:v:0", "-map", "1:a:0?", "-shortest"])
        .args(x264_args(crf))
        .arg(dst)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let total = (dur * fps).ceil().max(1.0);
    let len = (w as usize) * (h as usize) * 4;
    let mut buf = vec![0u8; len];
    let mut n = 0.0;
    let mut failure = None;
    {
        let (Some(mut input), Some(mut output)) = (dec.stdout.take(), enc.stdin.take()) else {
            let _ = dec.kill();
            let _ = enc.kill();
            return Err("ffmpeg: no pipes".into());
        };
        while input.read_exact(&mut buf).is_ok() {
            let Some(frame) = Rgba8::from_bytes(w as usize, h as usize, &buf) else { break };
            let out = develop_frame(&frame, s, Quality::Full);
            if let Err(e) = output.write_all(&out.as_bytes()) {
                failure = Some(format!("ffmpeg (encoding): {e}"));
                break;
            }
            n += 1.0;
            if !progress((n / total).min(0.99)) {
                failure = Some("cancelled".to_string());
                break;
            }
        }
        // closing the encoder's input ends the file
    }
    let _ = dec.kill();
    let _ = dec.wait();
    if failure.is_some() {
        let _ = enc.kill();
    }
    let mut err = Vec::new();
    if let Some(mut e) = enc.stderr.take() {
        let _ = e.read_to_end(&mut err);
    }
    let status = enc.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    if let Some(f) = failure {
        return Err(f);
    }
    if !status.success() {
        let msg = String::from_utf8_lossy(&err).lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("; ");
        return Err(format!("ffmpeg: {}", msg.chars().take(400).collect::<String>()));
    }
    if n < 1.0 {
        return Err("the video has no frames in that range".into());
    }
    progress(1.0);
    Ok(())
}
