//! LR-VID-*: video commands — what a clip is, trim (in / out points), poster frame, Capture Frame
//! and export. The playback itself is the Loupe's (`crates/ui-egui/src/video_ui.rs`); everything
//! the catalog keeps goes through [`lightcraft_catalog::Op::SetVideo`] (undoable).

use lightcraft_catalog::{MediaKind, Op, PhotoId, Source, VideoEdit};
use serde_json::{Value, json};

use super::{CommandSpec, bad, cmd, str_param};
use crate::video::{VideoExport, VideoFormat, VideoQuality};
use crate::{EngineError, Result, Session};

/// The one video a command acts on: `id`, else the active photo.
fn the_video(s: &Session, c: &str, p: &Value) -> Result<(PhotoId, std::sync::Arc<lightcraft_catalog::Photo>)> {
    let id = p.get("id").and_then(Value::as_u64).map(PhotoId).or_else(|| s.active()).ok_or_else(|| bad(c, "no photo selected"))?;
    let photo = s.catalog.photo(id).cloned().ok_or_else(|| bad(c, format!("no photo {}", id.0)))?;
    if photo.kind != MediaKind::Video {
        return Err(bad(c, format!("{} is not a video", photo.file_name)));
    }
    Ok((id, photo))
}

fn path_of(c: &str, photo: &lightcraft_catalog::Photo) -> Result<String> {
    match &photo.source {
        Source::File { path } => Ok(path.clone()),
        Source::Demo { .. } => Err(bad(c, format!("{} has no file", photo.file_name))),
    }
}

/// A number of seconds: a JSON number, or `"m:ss.ff"` / `"h:mm:ss"`.
pub fn seconds(v: &Value) -> Option<f64> {
    if let Some(f) = v.as_f64() {
        return f.is_finite().then_some(f);
    }
    let s = v.as_str()?.trim();
    let mut total = 0.0;
    for part in s.split(':') {
        total = total * 60.0 + part.trim().parse::<f64>().ok().filter(|x| x.is_finite() && *x >= 0.0)?;
    }
    Some(total)
}

/// `m:ss.ff` (or `h:mm:ss.ff` from an hour), for the UI and file names.
pub fn timecode(secs: f64) -> String {
    let s = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let cs = (s * 100.0).round() as u64;
    let (h, m, sec, frac) = (cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100);
    if h > 0 { format!("{h}:{m:02}:{sec:02}.{frac:02}") } else { format!("{m}:{sec:02}.{frac:02}") }
}

fn info(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "video.info";
    let (id, photo) = the_video(s, C, p)?;
    let edit = photo.video.unwrap_or_default();
    let (a, b) = edit.range(photo.duration);
    let mut out = json!({
        "id": id.0,
        "duration": photo.duration,
        "width": photo.width,
        "height": photo.height,
        "trimIn": a,
        "trimOut": b,
        "poster": edit.poster.unwrap_or(0.0),
        "edited": crate::video::has_edits(&photo.develop),
        "tools": crate::video::status(),
    });
    // the file itself (rate, codec, sound) when it can be read now
    if p.get("probe").and_then(Value::as_bool).unwrap_or(true) {
        match crate::video::probe(&path_of(C, &photo)?) {
            Ok(i) => {
                out["fps"] = json!(i.fps);
                out["codec"] = json!(i.codec);
                out["audio"] = json!(i.audio);
                if photo.duration.is_none() && i.duration > 0.0 {
                    out["duration"] = json!(i.duration);
                }
            }
            Err(e) => out["probeError"] = json!(e),
        }
    }
    Ok(out)
}

fn set_edit(s: &mut Session, id: PhotoId, edit: VideoEdit, label: &str) -> Result<Value> {
    s.commit(label, Op::SetVideo { id, video: Some(edit) })?;
    Ok(json!({"id": id.0, "trimIn": edit.trim_in, "trimOut": edit.trim_out, "poster": edit.poster}))
}

fn set_trim(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "video.setTrim";
    let (id, photo) = the_video(s, C, p)?;
    let mut edit = photo.video.unwrap_or_default();
    if p.get("clear").and_then(Value::as_bool).unwrap_or(false) {
        edit.trim_in = 0.0;
        edit.trim_out = None;
        return set_edit(s, id, edit, "Clear Trim");
    }
    let dur = photo.duration.filter(|d| d.is_finite() && *d > 0.0);
    let clamp = |t: f64| match dur {
        Some(d) => t.clamp(0.0, d),
        None => t.max(0.0),
    };
    if let Some(v) = p.get("in").filter(|v| !v.is_null()) {
        edit.trim_in = clamp(seconds(v).ok_or_else(|| bad(C, "`in` must be seconds (a number or m:ss)"))?);
    }
    match p.get("out") {
        Some(Value::Null) => edit.trim_out = None,
        Some(v) => edit.trim_out = Some(clamp(seconds(v).ok_or_else(|| bad(C, "`out` must be seconds (a number or m:ss), or null for the end"))?)),
        None => {}
    }
    // the end itself is no out point
    if let (Some(o), Some(d)) = (edit.trim_out, dur)
        && o >= d - 1e-3
    {
        edit.trim_out = None;
    }
    // at least a frame or so between the points
    if let Some(o) = edit.trim_out
        && o < edit.trim_in + 0.04
    {
        return Err(bad(C, format!("the out point ({}) must come after the in point ({})", timecode(o), timecode(edit.trim_in))));
    }
    set_edit(s, id, edit, "Trim Video")
}

fn set_poster(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "video.setPoster";
    let (id, photo) = the_video(s, C, p)?;
    let t = p.get("time").and_then(seconds).ok_or_else(|| bad(C, "missing `time` (seconds, or m:ss)"))?;
    let t = match photo.duration.filter(|d| d.is_finite() && *d > 0.0) {
        Some(d) => t.clamp(0.0, d),
        None => t.max(0.0),
    };
    let mut edit = photo.video.unwrap_or_default();
    edit.poster = (t > 0.0).then_some(t);
    let r = set_edit(s, id, edit, "Set Poster Frame")?;
    // its pixels are another frame now: decode again (thumbnails follow from the content key)
    s.media.forget(id);
    Ok(r)
}

/// `<dir>/<stem>-<m>m<ss>s<ff>.jpg` candidates (Capture Frame's file).
fn capture_names(video: &str, t: f64) -> impl Iterator<Item = std::path::PathBuf> {
    let p = std::path::Path::new(video);
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Video".into());
    let tc = timecode(t).replace([':', '.'], "-");
    (1u64..).map(move |k| dir.join(if k == 1 { format!("{stem}-{tc}.jpg") } else { format!("{stem}-{tc}-{k}.jpg") }))
}

fn capture_frame(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "video.captureFrame";
    let (id, photo) = the_video(s, C, p)?;
    let path = path_of(C, &photo)?;
    let t = p.get("time").and_then(seconds).unwrap_or(0.0).max(0.0);
    let t = photo.duration.filter(|d| *d > 0.0).map_or(t, |d| t.min(d));
    // the frame at full size, with the video's edits, as a JPEG next to the video
    let img = crate::video::frame_rgba(&path, t, usize::MAX, Some(&photo.develop), lightcraft_pipeline::Quality::Full).map_err(|e| bad(C, e))?;
    let jpeg = lightcraft_codecs::encode_jpeg(
        &lightcraft_codecs::EncodeImage::rgba8(&img),
        92,
        lightcraft_codecs::ChromaSubsampling::default(),
        &lightcraft_codecs::EncodeMeta::default(),
    )
    .map_err(|e| bad(C, e.to_string()))?;
    let dir_override = str_param(p, "dir").map(str::trim).filter(|d| !d.is_empty());
    let base = match dir_override {
        Some(dir) => std::path::Path::new(dir)
            .join(std::path::Path::new(&path).file_name().map(|n| n.to_os_string()).unwrap_or_default())
            .to_string_lossy()
            .to_string(),
        None => path.clone(),
    };
    let out = lightcraft_catalog::safe_file::write_new_unique(&mut capture_names(&base, t), &jpeg)
        .map_err(|e| EngineError::Other(format!("could not write the captured frame next to {base}: {e}")))?
        .to_string_lossy()
        .to_string();
    let report = crate::import::import(s, std::slice::from_ref(&out), crate::import::ImportMode::Add)?;
    let new = report
        .imported
        .first()
        .copied()
        .map(PhotoId)
        .ok_or_else(|| EngineError::Other(format!("{out} could not be imported: {:?}", report.failed)))?;
    // with the video, as Lightroom stacks a captured frame
    if p.get("stack").and_then(Value::as_bool).unwrap_or(true)
        && let Some(op) = s.catalog.stack_with_ops(new, &[id])
    {
        s.commit("Stack with Video", op)?;
    }
    if p.get("select").and_then(Value::as_bool).unwrap_or(false) {
        s.selection = crate::Selection::single(new);
    }
    Ok(json!({"id": new.0, "path": out, "time": t, "width": img.width, "height": img.height}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "video.export";
    let dir =
        str_param(p, "dir").map(str::trim).filter(|d| !d.is_empty()).ok_or_else(|| bad(C, "missing `dir` (the folder to export into)"))?.to_string();
    let format = match str_param(p, "format").map(str::to_ascii_lowercase).as_deref() {
        None | Some("h264" | "h.264" | "mp4") => VideoFormat::H264,
        Some("original") => VideoFormat::Original,
        Some(f) => return Err(bad(C, format!("unknown format `{f}` (h264 or original)"))),
    };
    let quality = match str_param(p, "quality") {
        None => VideoQuality::default(),
        Some(q) => VideoQuality::parse(q).ok_or_else(|| bad(C, format!("unknown quality `{q}` (max, high, medium or low)")))?,
    };
    let ids: Vec<PhotoId> = s.targets(p).into_iter().filter(|id| s.catalog.photo(*id).is_some_and(|p| p.kind == MediaKind::Video)).collect();
    if ids.is_empty() {
        return Err(bad(C, "no videos among the photos named or selected"));
    }
    let o = crate::export::ExportOptions { video: VideoExport { include: true, format, quality }, ..Default::default() };
    let to = crate::export::Destination { dir, exact: None };
    let mut write = |path: &str, bytes: &[u8]| crate::export::write_file(path, bytes);
    let exists = |path: &str| std::path::Path::new(path).exists();
    let items = crate::export::prepare_batch(s, &ids, &o).map_err(|e| bad(C, e))?;
    let files = crate::export::run_batch(items, &o, &to, &mut write, &exists, false, &mut |_, _| true).map_err(|e| bad(C, e))?;
    Ok(json!({"files": files}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "video.status", "Video Tools", [], None, "{} → {backend: ffmpeg|avFoundation|none, ffmpeg?, ffprobe?, ffplay?, avfoundation, avconvert, notice?}", super::always, |_, _| Ok(
            crate::video::status()
        )),
        cmd!(
            query "video.info",
            "Video Info",
            [],
            None,
            "{id?, probe?: bool (default true)} → {duration, width, height, trimIn, trimOut, poster, edited, fps?, codec?, audio?, tools}",
            super::has_active,
            info
        ),
        cmd!(
            "video.setTrim",
            "Trim Video",
            [],
            None,
            "{id?, in?: seconds or m:ss, out?: seconds, m:ss or null (= the end), clear?: bool} — non-destructive in/out points (export and playback use them)",
            super::has_active,
            set_trim
        ),
        cmd!(
            "video.setPoster",
            "Set Poster Frame",
            [],
            None,
            "{id?, time: seconds or m:ss} — the frame shown as the video's thumbnail",
            super::has_active,
            set_poster
        ),
        cmd!(
            "video.captureFrame",
            "Capture Frame",
            [],
            None,
            "{id?, time: seconds or m:ss, dir?: folder (default: the video's), stack?: bool (default true), select?: bool} — the frame (with the video's edits) as a JPEG, imported and stacked with the video → {id, path}",
            super::has_active,
            capture_frame
        ),
        cmd!(
            "video.export",
            "Export Video",
            [],
            None,
            "{ids?, dir, format?: h264|original, quality?: max|high|medium|low} — trimmed, with the edits Classic applies to video (H.264 .mp4); original = the file as it is → {files}",
            super::has_selection,
            export
        ),
    ]
}
