//! Video, publish and the toast / Export dialog fixes, headless (with `LIGHTCRAFT_TEST_SHOTS=<dir>`
//! each writes a real-size screenshot there).

use serde_json::json;

use crate::headless::Headless;
use crate::tests_library2::{T, headless, settle, shot};

fn widget_rect(h: &mut Headless, id: &str) -> Option<[f64; 4]> {
    let r = h.request("ui.widgets", json!({}), T);
    let w = r["result"].as_array()?.iter().find(|w| w["id"] == id)?.clone();
    let v = |i: usize| w["rect"][i].as_f64().unwrap_or(0.0);
    Some([v(0), v(1), v(2), v(3)])
}

/// The Export dialog is taller than a small window: its options scroll inside it and the window,
/// with its OK button, stays on screen.
#[test]
fn export_dialog_fits_a_small_window() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    h.size = egui::vec2(1100.0, 560.0);
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    settle(&mut h);
    let w = widget_rect(&mut h, "dialog:window").expect("dialog on screen");
    assert!(w[1] >= -0.5 && w[1] + w[3] <= 560.5, "the dialog runs off the window: {w:?}");
    let ok = widget_rect(&mut h, "button:dialogOk").expect("OK button");
    assert!(ok[1] + ok[3] <= 560.0, "the OK button is off screen: {ok:?}");
    shot(&mut h, "export-small-window.png");
}

/// A toast set after a modal folder picker (every clock still at the time before the picker)
/// stays for its whole duration from the next frame on: the tethered-capture toast never showed.
#[test]
fn tether_toast_survives_a_modal_picker() {
    let dir = crate::tests_library2::temp("tether-toast");
    let mut h = headless(lightcraft_engine::Session::with_demo());
    let folder = dir.0.to_string_lossy().to_string();
    // the picker "blocks" for a minute: the frame clock jumps past any toast timed before it
    h.app.services.pick_folder = Some(Box::new(move || Some(folder.clone())));
    h.app.run("dialog.tetherStart", json!({})).unwrap();
    h.advance_time(60.0);
    settle(&mut h);
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Tethered:"), "the toast is gone: {toast:?}");
    let _ = h.app.run("dialog.tetherStop", json!({}));
}

/// A 4 s 320×240 clip with sound (ffmpeg), or `None` without ffmpeg.
fn clip(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let ff = lightcraft_engine::video::ffmpeg()?;
    let path = dir.join("clip.mp4");
    let ok = std::process::Command::new(ff)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=4", "-f", "lavfi", "-i", "sine=duration=4"])
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&path)
        .status()
        .ok()?
        .success();
    ok.then_some(path)
}

/// LR-VID-PLAY / TRIM / COVER, KEY-VIDEO-PLAY: a video in the Loupe has the control bar; play.
/// (and Space) play and pause it, the scrubber seeks, trim. handles set the in point, poster.
/// sets the thumbnail, cap. adds a stacked JPEG; the grid cell shows the clip's length.
#[test]
fn video_loupe_plays_trims_and_captures() {
    let dir = crate::tests_library2::temp("video-loupe");
    let Some(path) = clip(&dir.0) else { return eprintln!("skipped: no ffmpeg") };
    let mut s = lightcraft_engine::Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().expect("imported");
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    // the grid cell: its length
    h.app.run("view.library", json!({"view": "photoGrid"})).unwrap();
    h.app.run("library.source", json!({"kind": "all"})).unwrap();
    settle(&mut h);
    settle(&mut h);
    let ws = h.request("ui.widgets", json!({}), T);
    let ids: Vec<String> =
        ws["result"].as_array().map(|a| a.iter().filter_map(|w| w["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    assert!(
        ids.iter().any(|w| *w == format!("badge:video:{id}")),
        "the grid shows the clip's length: {:?}",
        ids.iter().filter(|w| w.contains(':')).take(40).collect::<Vec<_>>()
    );
    shot(&mut h, "video-grid.png");
    h.app.run("view.library", json!({"view": "detail"})).unwrap();
    settle(&mut h);
    assert!(widget_rect(&mut h, "video:bar").is_some() && widget_rect(&mut h, "button:videoPlay").is_some());
    shot(&mut h, "video-loupe-paused.png");

    // play., then Space pauses
    crate::tests_library2::click(&mut h, "button:videoPlay");
    assert!(h.app.video.playing, "playing: {:?}", h.app.video.error);
    h.step_until(T, |h| h.app.video.pos > 0.5);
    for _ in 0..20 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    shot(&mut h, "video-loupe-playing.png");
    h.request("ui.key", json!({"key": "space"}), T);
    settle(&mut h);
    assert!(!h.app.video.playing, "Space pauses: pos {} err {:?} zoom {:?}", h.app.video.pos, h.app.video.error, h.app.ui.zoom);
    let at = h.app.video.pos;
    assert!(at > 0.0);

    // the scrubber seeks (paused: the frame there is fetched)
    let track = widget_rect(&mut h, "video:scrubber").unwrap();
    h.request("ui.clickWidget", json!({"id": "video:scrubber", "fx": 0.75}), T);
    settle(&mut h);
    assert!((h.app.video.pos - 3.0).abs() < 0.3, "seeked to 3 s of 4: {} (track {track:?})", h.app.video.pos);

    // trim.: drag the in handle to about 1 s
    crate::tests_library2::click(&mut h, "button:videoTrim");
    assert!(h.app.video.trim_mode);
    let handle = widget_rect(&mut h, "video:in").expect("in handle");
    let dx = track[2] * 0.25;
    h.request("ui.dragWidget", json!({"id": "video:in", "dx": dx}), T);
    settle(&mut h);
    let edit = h.app.session.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().video.unwrap_or_default();
    assert!((edit.trim_in - 1.0).abs() < 0.3, "in point ≈ 1 s: {edit:?} (handle {handle:?})");
    shot(&mut h, "video-loupe-trim.png");

    // poster. at the playhead; cap. adds a JPEG stacked with the video
    h.app.run("video.seek", json!({"time": 2.0})).unwrap();
    crate::tests_library2::click(&mut h, "button:videoPoster");
    let edit = h.app.session.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().video.unwrap_or_default();
    assert_eq!(edit.poster, Some(2.0));
    let n = h.app.session.catalog.len();
    crate::tests_library2::click(&mut h, "button:videoCapture");
    assert_eq!(h.app.session.catalog.len(), n + 1, "{:?}", h.app.video.error);
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Captured frame"), "{toast}");
    // a still photo has no bar
    h.app.run("view.library", json!({"view": "photoGrid"})).unwrap();
    settle(&mut h);
}

/// LR-VID-EXPORT: the Export dialog's Video section (shown when videos are selected) exports
/// the trimmed clip as H.264 in the background.
#[test]
fn export_dialog_exports_a_trimmed_video() {
    let dir = crate::tests_library2::temp("video-export");
    let Some(path) = clip(&dir.0) else { return eprintln!("skipped: no ffmpeg") };
    let mut s = lightcraft_engine::Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().expect("imported");
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    s.execute("video.setTrim", &json!({"id": id, "in": 1.0, "out": 2.5})).unwrap();
    let mut h = headless(s);
    h.app.run("dialog.export", json!({})).unwrap();
    let out = dir.0.join("out");
    if let Some(crate::state::Dialog::Export { dir: d, .. }) = &mut h.app.ui.dialog {
        *d = out.to_string_lossy().to_string();
    }
    settle(&mut h);
    assert!(widget_rect(&mut h, "check:exportVideoInclude").is_some(), "the Video section shows");
    crate::tests_library2::click(&mut h, "combo:exportVideoQuality");
    crate::tests_library2::click(&mut h, "button:exportVideoQuality-3");
    shot(&mut h, "video-export-dialog.png");
    crate::tests_library2::click(&mut h, "button:dialogOk");
    let t0 = std::time::Instant::now();
    while h.app.export.is_some() && t0.elapsed() < std::time::Duration::from_secs(90) {
        h.step();
    }
    let mp4 = out.join("clip.mp4");
    assert!(mp4.is_file(), "exported: {:?}", std::fs::read_dir(&dir.0).map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>()));
    let info = lightcraft_engine::video::probe(&mp4.to_string_lossy()).unwrap();
    assert!((info.duration - 1.5).abs() < 0.2, "trimmed: {}", info.duration);
}
