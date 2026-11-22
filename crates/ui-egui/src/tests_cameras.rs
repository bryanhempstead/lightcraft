//! Settings ▸ cams.: the library's cameras with their decoder / profile / lens / ΔE lines, adding a
//! camera by hand, and the import toast naming a camera the library hadn't seen.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes screenshots there.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn jpeg(path: &std::path::Path, make: &str, model: &str, lens: &str, shade: u8) {
    let (w, h) = (32usize, 24usize);
    let data: Vec<[u8; 4]> = (0..w * h).map(|k| [(k % w * 5) as u8, (k / w * 7) as u8, shade, 255]).collect();
    let img = lightcraft_raster::Rgba8 { width: w, height: h, data };
    let meta = lightcraft_meta::Metadata { make: Some(make.into()), model: Some(model.into()), lens_model: Some(lens.into()), ..Default::default() };
    let exif = lightcraft_meta::write_exif(&meta);
    let m = lightcraft_codecs::EncodeMeta { exif: Some(&exif), ..Default::default() };
    let bytes = lightcraft_codecs::encode_jpeg(&lightcraft_codecs::EncodeImage::rgba8(&img), 90, Default::default(), &m).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn headless(session: lightcraft_engine::Session) -> Headless {
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = Services { png: Some(Box::new(png)), write: Some(Box::new(lightcraft_engine::export::write_file)), ..Default::default() };
    let mut h = Headless::new(LightcraftApp::new(session, services), [1300.0, 900.0], 1.0);
    h.step();
    h.step();
    h
}

fn click(h: &mut Headless, id: &str) {
    h.step();
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    for _ in 0..4 {
        h.step();
    }
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        for _ in 0..3 {
            h.step();
        }
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

#[test]
fn cameras_settings_list_add_and_import_notice() {
    let dir = std::env::temp_dir().join(format!("lc-ui-cameras-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let _t = TempDir(dir.clone());
    jpeg(&dir.join("src/a.jpg"), "Zeta", "Zeta Z-1", "Zeta 35mm F2", 10);
    jpeg(&dir.join("src/b.jpg"), "Zeta", "Zeta Z-1", "Zeta 50mm F1.8", 90);
    jpeg(&dir.join("src/c.jpg"), "Omega", "Omega O-5", "Omega 24mm", 160);
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.cameras_file = Some(dir.join("config/cameras.json"));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::write(dir.join("config/lr-compare.json"), r#"{"cameras": {"Zeta Z-1": {"de": 2.52, "photos": 40, "date": "2026-11-21"}}}"#).unwrap();
    let r = s.execute("library.import", &json!({"paths": [dir.join("src").to_string_lossy()]})).unwrap();
    let names: Vec<&str> = r["newCameras"].as_array().unwrap().iter().filter_map(|c| c["name"].as_str()).collect();
    assert!(names.contains(&"Zeta Z-1") && names.contains(&"Omega O-5"), "{r}");
    assert_eq!(names.len(), 2, "{r}");
    let mut h = headless(s);
    h.app.run("app.settings", json!({"tab": "cameras"})).unwrap();
    for _ in 0..4 {
        h.step();
    }
    click(&mut h, "button:settingsCameraLenses-0");
    // add one by hand: typed, then the add. button
    let r = h.request("ui.clickWidget", json!({"id": "field:settingsCameraAdd"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let r = h.request("ui.text", json!({"text": "Made Up Cam 9000"}), T);
    assert_eq!(r["ok"], true, "{r}");
    click(&mut h, "button:settingsCameraAdd");
    let list = h.app.session.execute("cameras.list", &json!({})).unwrap();
    let cams = list["cameras"].as_array().unwrap();
    assert_eq!(cams.len(), 3, "{list}");
    assert_eq!(cams[0]["name"], "Zeta Z-1", "most photos first");
    assert_eq!(cams[0]["decoder"], "rendered");
    assert_eq!(cams[0]["deltaE"]["de"], 2.52);
    assert!(cams.iter().any(|c| c["name"] == "Made Up Cam 9000" && c["added"] == true));
    shot(&mut h, "cameras-settings.png");
}
