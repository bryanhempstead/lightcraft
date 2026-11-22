//! L33: tethered capture — a shoot folder watched and imported into an album as shots arrive;
//! gphoto2 started and stopped with it when installed (here a stand-in script).

use serde_json::json;

use crate::Session;

#[test]
fn tethered_capture_imports_shots_as_they_arrive() {
    let dir = std::env::temp_dir().join(format!("lc-tether-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    // a stand-in for gphoto2 that just waits (what --capture-tethered does between shots)
    let fake = dir.join("bin/gphoto2");
    std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    assert!(s.execute("tether.start", &json!({})).is_err());
    assert!(s.execute("tether.start", &json!({"folder": "relative/path"})).is_err());
    assert!(s.execute("tether.start", &json!({"folder": dir.to_string_lossy(), "session": "../x"})).is_err());
    let r = s
        .execute("tether.start", &json!({"folder": dir.join("shots").to_string_lossy(), "session": "Studio", "gphoto2": fake.to_string_lossy()}))
        .unwrap();
    assert_eq!((r["running"].as_bool(), r["gphoto2"].as_bool()), (Some(true), Some(true)), "{r}");
    assert!(s.execute("tether.start", &json!({"folder": dir.to_string_lossy()})).is_err(), "one at a time");
    let folder = std::path::PathBuf::from(r["folder"].as_str().unwrap());
    assert!(folder.ends_with("shots/Studio") && folder.is_dir());
    // a shot arrives: the watched folder imports it once its size holds
    let img = lightcraft_raster::Rgba8 { width: 8, height: 6, data: vec![[40, 80, 120, 255]; 48] };
    let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(folder.join("IMG_0001.png"), png).unwrap();
    for _ in 0..3 {
        s.execute("library.autoImportScan", &json!({})).unwrap();
    }
    assert_eq!(s.catalog.photos().filter(|p| p.in_library()).count(), 1, "imported");
    assert!(s.catalog.albums().any(|a| a.name == "Studio" && a.photos.len() == 1), "into the session's album");
    let st = s.execute("tether.stop", &json!({})).unwrap();
    assert_eq!(st["running"], false);
    assert_eq!(s.execute("tether.status", &json!({})).unwrap()["running"], false);
    s.close_library().unwrap();
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
