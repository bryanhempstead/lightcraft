//! LR-VID-*: probe parsing, the poster source, trim / poster / Capture Frame / export commands.
//! The tests that need real clips make them with ffmpeg (`testsrc2` + a tone) and skip, saying
//! so, where ffmpeg isn't installed; the AVFoundation helper is checked on macOS.

use serde_json::json;

use crate::Session;
use crate::video::{self, Backend, VideoExport, VideoFormat, VideoQuality};

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-video-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A 3 s 320×240 test clip with sound at `path` (`rotate`: display rotation metadata), or `None`
/// without ffmpeg.
fn make_clip(path: &std::path::Path, rotate: bool) -> Option<()> {
    let ff = video::ffmpeg()?;
    let mut c = std::process::Command::new(ff);
    c.args([
        "-v",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=25:duration=3",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=3",
    ]);
    c.args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", "-metadata", "creation_time=2024-05-01T12:34:56.000000Z"]);
    let tmp = path.with_extension("tmp.mp4");
    c.arg(if rotate { &tmp } else { path });
    assert!(c.status().ok()?.success(), "ffmpeg could not make the test clip");
    if rotate {
        let st = std::process::Command::new(video::ffmpeg()?)
            .args(["-v", "error", "-y", "-display_rotation", "90", "-i"])
            .arg(&tmp)
            .args(["-c", "copy"])
            .arg(path)
            .status()
            .ok()?;
        assert!(st.success());
        let _ = std::fs::remove_file(&tmp);
    }
    Some(())
}

fn skip(why: &str) {
    eprintln!("skipped: {why} (install ffmpeg to run it)");
}

#[test]
fn ffprobe_json_is_read_upright_with_rate_and_date() {
    let j = br#"{"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001",
        "side_data_list":[{"rotation":-90}]},{"codec_type":"audio"}],
        "format":{"duration":"12.5","tags":{"creation_time":"2024-05-01T12:34:56.000000Z"}}}"#;
    let i = video::parse_ffprobe(j).unwrap();
    assert_eq!((i.width, i.height), (1080, 1920), "a portrait phone clip is taller than wide");
    assert!((i.fps - 29.97).abs() < 0.01 && (i.duration - 12.5).abs() < 1e-9 && i.audio && i.codec == "h264");
    assert_eq!(i.created.as_deref(), Some("2024-05-01T12:34:56"));
    // hostile output: errors, never panics
    assert!(video::parse_ffprobe(b"not json").is_err());
    assert!(video::parse_ffprobe(br#"{"streams":[{"codec_type":"audio"}]}"#).is_err());
    assert!(video::parse_ffprobe(br#"{"streams":[{"codec_type":"video","width":-5,"height":1e30}]}"#).is_err());
    let odd =
        video::parse_ffprobe(br#"{"streams":[{"codec_type":"video","width":64,"height":48,"avg_frame_rate":"0/0"}],"format":{"duration":"NaN"}}"#)
            .unwrap();
    assert_eq!((odd.fps, odd.duration), (25.0, 0.0));
}

#[test]
fn poster_time_travels_with_the_path() {
    assert_eq!(video::split_source("/v/a.mov"), ("/v/a.mov", 0.0));
    let s = video::frame_source("/v/a #1.mov", Some(2.5));
    assert_eq!(video::split_source(&s), ("/v/a #1.mov", 2.5));
    assert_eq!(video::frame_source("/v/a.mov", Some(f64::NAN)), "/v/a.mov");
    assert!(video::is_video_path(&s) && video::is_video_path("/x/B.MP4") && !video::is_video_path("/x/b.jpg"));
    assert_eq!(video::split_source("/v/a.mov\u{0}t=garbage").1, 0.0);
    assert_eq!(crate::cmd::video_seconds(&json!("1:02.5")), Some(62.5));
    assert_eq!(crate::cmd::video_seconds(&json!(3)), Some(3.0));
    assert_eq!(crate::cmd::video_seconds(&json!("x")), None);
    assert_eq!(crate::cmd::video_timecode(62.5), "1:02.50");
    assert_eq!(crate::cmd::video_timecode(f64::INFINITY), "0:00.00");
    assert_eq!(video::fit_even(1921, 1081, 1080), (1080, 608));
    let info = video::VideoInfo { width: 3840, height: 2160, ..Default::default() };
    assert_eq!(video::export_size(&info, VideoQuality::High), (1920, 1080));
    assert_eq!(video::export_size(&info, VideoQuality::Max), (3840, 2160));
}

#[test]
fn video_settings_keep_the_edit_panel_but_not_geometry() {
    let mut s = lightcraft_develop::DevelopSettings::default();
    assert!(!video::has_edits(&s));
    // the whole Edit panel applies to video (LR-VID-EDIT)
    s.effects.clarity = 50.0;
    s.light.highlights = -40.0;
    assert!(video::has_edits(&s), "clarity and highlights apply to video");
    s.light.exposure = 1.0;
    s.color.vibrance = 20.0;
    s.mixer.blue.sat = -30.0;
    s.grading.shadows.sat = 25.0;
    s.grading.shadows.hue = 210.0;
    // ...but not what changes a frame's geometry or belongs to one photo
    s.crop.flip_h = true;
    s.geometry.vertical = 20.0;
    s.spots.push(lightcraft_develop::Spot::default());
    s.hdr.enabled = true;
    s.hdr.sdr_brightness = 50.0;
    let v = video::video_settings(&s);
    assert_eq!((v.light.highlights, v.effects.clarity, v.color.vibrance, v.mixer.blue.sat), (-40.0, 50.0, 20.0, -30.0));
    assert_eq!(v.grading, s.grading);
    assert_eq!((v.crop, v.geometry.vertical, v.spots.len(), v.hdr.enabled), (lightcraft_develop::Crop::default(), 0.0, 0, false));
    // an HDR edit's frames are its SDR rendition (Brightness +50 = +0.5 EV)
    assert!((v.light.exposure - 1.5).abs() < 1e-9, "{}", v.light.exposure);
    // an edited frame is brighter, same size
    let img = lightcraft_raster::Rgba8::filled(16, 8, [90, 90, 90, 255]);
    let out = video::develop_frame(&img, &s, lightcraft_pipeline::Quality::Full);
    assert_eq!((out.width, out.height), (16, 8));
    assert!(out.data[0][1] > 110, "{:?}", out.data[0]);
}

/// Clarity on a frame changes it (it was ignored before LR-VID-EDIT), and playback, Capture
/// Frame and export share one render: the same frame developed twice is identical.
#[test]
fn clarity_renders_on_frames_and_the_render_is_shared() {
    let img = lightcraft_raster::Rgba8 {
        width: 64,
        height: 48,
        data: (0..64 * 48)
            .map(|i| {
                let (x, y) = (i % 64, i / 64);
                let v = (60 + (x * 2) + if (x / 8 + y / 8) % 2 == 0 { 40 } else { 0 }) as u8;
                [v, v.saturating_sub(10), v.saturating_add(5), 255]
            })
            .collect(),
    };
    let mut s = lightcraft_develop::DevelopSettings::default();
    let plain = video::develop_frame(&img, &s, lightcraft_pipeline::Quality::Full);
    s.effects.clarity = 80.0;
    s.light.shadows = 60.0;
    let a = video::develop_frame(&img, &s, lightcraft_pipeline::Quality::Full);
    let b = video::develop_frame(&img, &s, lightcraft_pipeline::Quality::Full);
    assert_eq!(a.data, b.data);
    let changed = a.data.iter().zip(&plain.data).filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > 2)).count();
    assert!(changed > a.data.len() / 4, "clarity / shadows changed only {changed} pixels");
}

/// Import a clip: a video with its length and date; its poster renders (frame 0, then the poster
/// set), trim and poster are undoable and checked.
#[test]
fn import_trim_poster_and_capture_frame() {
    let dir = temp_dir("import");
    let clip = dir.join("clip.mp4");
    if make_clip(&clip, false).is_none() {
        return skip("import_trim_poster_and_capture_frame");
    }
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [clip.to_string_lossy()]})).unwrap();
    let id = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().expect("imported"));
    let p = s.catalog.photo(id).unwrap().clone();
    assert_eq!((p.kind, p.width, p.height, p.format.as_str()), (lightcraft_catalog::MediaKind::Video, 320, 240, "MP4"));
    assert!((p.duration.unwrap() - 3.0).abs() < 0.1, "{:?}", p.duration);
    assert_eq!(p.captured.as_deref(), Some("2024-05-01T12:34:56"));
    assert!(p.content_hash.as_deref().is_some_and(|h| h.starts_with("video:")));

    let render = |s: &mut Session| s.render_job(id, 160, 160, false, true).unwrap().run().rendered.unwrap().image;
    let first = render(&mut s);
    assert_eq!((first.width, first.height), (160, 120));
    let key0 = crate::media::content_key(s.catalog.photo(id).unwrap());

    // poster at 2 s: other content, another picture
    s.execute("video.setPoster", &json!({"time": "0:02", "id": id.0})).unwrap();
    assert_eq!(s.catalog.photo(id).unwrap().video.unwrap().poster, Some(2.0));
    assert_ne!(crate::media::content_key(s.catalog.photo(id).unwrap()), key0);
    let later = render(&mut s);
    assert_ne!(first.data, later.data, "the poster frame shows");

    // trim, checked, undoable
    s.execute("video.setTrim", &json!({"id": id.0, "in": 0.5, "out": 2.5})).unwrap();
    let info = s.execute("video.info", &json!({"id": id.0})).unwrap();
    assert_eq!((info["trimIn"].as_f64(), info["trimOut"].as_f64()), (Some(0.5), Some(2.5)), "{info}");
    assert!(info["audio"] == true && info["fps"].as_f64().is_some_and(|f| (f - 25.0).abs() < 0.1), "{info}");
    assert!(s.execute("video.setTrim", &json!({"id": id.0, "in": 2.0, "out": 1.0})).is_err());
    assert!(s.execute("video.setTrim", &json!({"id": id.0, "in": "x"})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.catalog.photo(id).unwrap().video.unwrap().trim_out, None);
    s.execute("edit.redo", &json!({})).unwrap();
    // the end is no out point
    s.execute("video.setTrim", &json!({"id": id.0, "out": 99})).unwrap();
    assert_eq!(s.catalog.photo(id).unwrap().video.unwrap().trim_out, None);

    // Capture Frame: a JPEG next to the clip, imported and stacked with it
    let c = s.execute("video.captureFrame", &json!({"id": id.0, "time": 1.0})).unwrap();
    let path = c["path"].as_str().unwrap();
    assert!(path.ends_with("clip-0-01-00.jpg") && std::path::Path::new(path).is_file(), "{c}");
    let new = lightcraft_catalog::PhotoId(c["id"].as_u64().unwrap());
    assert_eq!(s.catalog.photo(new).unwrap().kind, lightcraft_catalog::MediaKind::Image);
    assert!(s.catalog.stack_of(new).is_some_and(|st| st.photos.contains(&id)), "stacked with the video");

    // not a video: refused
    assert!(s.execute("video.setPoster", &json!({"id": new.0, "time": 1})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Export: trimmed H.264 (its length is the trim), the edits show, Original copies the file, and
/// Include Video Files off leaves videos out of a batch.
#[test]
fn export_trimmed_edited_h264_and_original() {
    let dir = temp_dir("export");
    let clip = dir.join("clip.mov");
    if make_clip(&clip, false).is_none() {
        return skip("export_trimmed_edited_h264_and_original");
    }
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [clip.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    s.execute("video.setTrim", &json!({"id": id, "in": 1.0, "out": 2.0})).unwrap();
    let out = dir.join("out");
    let e = s.execute("video.export", &json!({"ids": [id], "dir": out.to_string_lossy(), "quality": "low"})).unwrap();
    let plain = e["files"][0]["path"].as_str().unwrap_or_else(|| panic!("{e}")).to_string();
    assert!(plain.ends_with("clip.mp4"), "{plain}");
    let i = video::probe(&plain).unwrap();
    assert!((i.duration - 1.0).abs() < 0.15, "trimmed to 1 s: {}", i.duration);
    assert!(i.audio && (i.width, i.height) == (320, 240), "{i:?}");

    // with an edit (+1 EV): every frame goes through the pipeline and comes out brighter
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0, "ids": [id]})).unwrap();
    let e2 = s.execute("video.export", &json!({"ids": [id], "dir": out.to_string_lossy(), "quality": "low"})).unwrap();
    let edited = e2["files"][0]["path"].as_str().unwrap_or_else(|| panic!("{e2}")).to_string();
    assert_ne!(edited, plain, "a free name, the first export kept");
    let mean = |p: &str| {
        let png = video::frame_png(p, 0.5, 64).unwrap();
        let d = lightcraft_codecs::decode(&png, lightcraft_codecs::DecodeOptions::default()).unwrap().to_srgb8();
        d.data.iter().map(|px| px[0] as f64 + px[1] as f64 + px[2] as f64).sum::<f64>() / d.data.len() as f64
    };
    assert!(mean(&edited) > mean(&plain) + 20.0, "edited {} vs plain {}", mean(&edited), mean(&plain));
    assert!((video::probe(&edited).unwrap().duration - 1.0).abs() < 0.15);

    // Original: the file as it is
    let e3 = s.execute("video.export", &json!({"ids": [id], "dir": out.join("orig").to_string_lossy(), "format": "original"})).unwrap();
    let orig = e3["files"][0]["path"].as_str().unwrap();
    assert!(orig.ends_with("clip.mov"));
    assert_eq!(std::fs::read(orig).unwrap(), std::fs::read(&clip).unwrap());

    // Include Video Files off: left out of the batch; bad options refused
    let o = crate::export::ExportOptions { video: VideoExport { include: false, ..Default::default() }, ..Default::default() };
    let items = crate::export::prepare_batch(&mut s, &[lightcraft_catalog::PhotoId(id)], &o).unwrap();
    assert!(items.is_empty());
    assert!(crate::export::ExportOptions::validate("app.export", &json!({"video": {"format": "gif"}})).is_err());
    assert!(crate::export::ExportOptions::validate("app.export", &json!({"video": {"include": true, "quality": "max"}})).is_ok());
    assert!(s.execute("video.export", &json!({"ids": [id], "dir": out.to_string_lossy(), "quality": "huge"})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Playback: frames arrive in order at the requested size, from the in point; dropping the player
/// stops it.
#[test]
fn player_streams_frames_from_the_in_point() {
    let dir = temp_dir("play");
    let clip = dir.join("clip.mp4");
    if make_clip(&clip, false).is_none() {
        return skip("player_streams_frames_from_the_in_point");
    }
    let path = clip.to_string_lossy().to_string();
    let info = video::probe(&path).unwrap();
    let mut backends = vec![Backend::Ffmpeg];
    if video::avfoundation() {
        backends.push(Backend::AvFoundation);
    }
    for b in backends {
        let mut p = video::Player::start_with(b, &path, &info, 1.0, 2.0, (64, 48), None, false).unwrap();
        let t0 = std::time::Instant::now();
        let mut got = Vec::new();
        // a clock that waits for each frame: every frame of the range comes, in order
        let mut clock = 1.0;
        while !p.finished() && t0.elapsed().as_secs() < 30 {
            if let Some(f) = p.frame_at(clock) {
                assert_eq!((f.image.width, f.image.height), (64, 48));
                got.push(f.t);
                clock = f.t + 1.0 / 25.0;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(got.first().is_some_and(|t| (t - 1.0).abs() < 1e-6), "{b:?}: {got:?}");
        assert!(got.windows(2).all(|w| w[1] > w[0]), "{b:?}: in order");
        assert!((24..=27).contains(&got.len()), "{b:?}: one second at 25 fps: {}", got.len());
        // the due frame only: with the clock at 1.2 s nothing later is handed out
        let mut p2 = video::Player::start_with(b, &path, &info, 1.0, 2.0, (32, 24), None, false).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let f = p2.frame_at(1.2).expect("a frame is due");
        assert!(f.t <= 1.2 + 1e-6, "{b:?}: {}", f.t);
        drop(p2);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// macOS: the AVFoundation helper describes a rotated clip upright and gives frames the same size
/// ffmpeg does; avconvert trims.
#[cfg(target_os = "macos")]
#[test]
fn avfoundation_backend_matches_ffmpeg() {
    if !video::avfoundation() {
        return eprintln!("skipped: no Swift compiler");
    }
    let dir = temp_dir("avf");
    let clip = dir.join("rot.mp4");
    if make_clip(&clip, true).is_none() {
        return skip("avfoundation_backend_matches_ffmpeg");
    }
    let path = clip.to_string_lossy().to_string();
    let a = video::probe_with(Backend::AvFoundation, &path).unwrap();
    let f = video::probe_with(Backend::Ffmpeg, &path).unwrap();
    assert_eq!((a.width, a.height), (240, 320), "upright");
    assert_eq!((a.width, a.height), (f.width, f.height));
    assert!((a.duration - f.duration).abs() < 0.1 && a.audio);
    let size = |png: Vec<u8>| {
        let d = lightcraft_codecs::decode(&png, lightcraft_codecs::DecodeOptions::default()).unwrap();
        (d.width, d.height)
    };
    assert_eq!(size(video::frame_png_with(Backend::AvFoundation, &path, 1.0, 160).unwrap()), (120, 160));
    assert_eq!(size(video::frame_png_with(Backend::Ffmpeg, &path, 1.0, 160).unwrap()), (120, 160));
    let out = dir.join("trim.mp4");
    let opts = VideoExport { include: true, format: VideoFormat::H264, quality: VideoQuality::Low };
    let mut edits = lightcraft_develop::DevelopSettings::default();
    edits.light.exposure = 1.0;
    let notice = video::export_with(Backend::AvFoundation, &path, &out, &a, (0.5, Some(1.5)), opts, Some(&edits), &|_| true).unwrap();
    assert!(notice.is_some_and(|n| n.contains("without its edits")), "says the edits are left out");
    assert!((video::probe(&out.to_string_lossy()).unwrap().duration - 1.0).abs() < 0.2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_files_and_tools_are_errors() {
    assert!(video::probe("/nonexistent/clip.mov").is_err());
    assert!(video::probe_with(Backend::None, "/nonexistent/clip.mov").is_err());
    let dir = temp_dir("none");
    let f = dir.join("fake.mov");
    std::fs::write(&f, b"not a movie").unwrap();
    let p = f.to_string_lossy().to_string();
    assert!(video::probe_with(Backend::None, &p).unwrap_err().contains("ffmpeg"));
    assert!(video::frame_png_with(Backend::None, &p, 0.0, 64).is_err());
    if video::ffmpeg().is_some() {
        assert!(video::probe_with(Backend::Ffmpeg, &p).is_err(), "a damaged file is an error");
        assert!(video::frame_png_with(Backend::Ffmpeg, &p, 0.0, 64).is_err());
    }
    let info = video::VideoInfo { width: 64, height: 48, duration: 1.0, fps: 25.0, ..Default::default() };
    assert!(video::Player::start_with(Backend::None, &p, &info, 0.0, 1.0, (64, 48), None, false).is_err());
    let opts = VideoExport::default();
    assert!(video::export_with(Backend::None, &p, &dir.join("o.mp4"), &info, (0.0, None), opts, None, &|_| true).is_err());
    // a damaged clip imports as a failure with the reason, not a crash
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [p]})).unwrap();
    assert_eq!(r["imported"].as_array().map(Vec::len), Some(0), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}
