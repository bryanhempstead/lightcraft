//! L23: Library ▸ Validate DNG Files — each DNG's raw data against its digest (Adobe's DNG SDK).

use serde_json::json;

use crate::Session;

#[test]
fn validate_dng_files() {
    let dir = std::env::temp_dir().join(format!("lc-validatedng-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let good = dir.join("files/a.dng");
    std::fs::write(&good, crate::tests_xmp::synthetic_dng_with(None, lightcraft_meta::Metadata::default())).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [good.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    let v = s.execute("library.validateDng", &json!({"ids": [id]})).unwrap();
    assert_eq!(v["checked"], 1, "{v}");
    if lightcraft_dng_sdk_sys::available() {
        // LightCraft's own DNG writer stores no digest
        assert_eq!(v["noDigest"], json!([id]), "{v}");
        // a file cut short is reported, never a crash
        let bytes = std::fs::read(&good).unwrap();
        std::fs::write(&good, &bytes[..bytes.len() / 2]).unwrap();
        let v = s.execute("library.validateDng", &json!({"ids": [id]})).unwrap();
        assert_eq!(v["valid"], 0, "{v}");
        assert!(v["failed"].as_array().unwrap().len() + v["damaged"].as_array().unwrap().len() + v["noDigest"].as_array().unwrap().len() == 1, "{v}");
    } else {
        assert_eq!(v["failed"].as_array().unwrap().len(), 1, "no SDK: said so per file");
    }
    std::fs::remove_file(&good).unwrap();
    let v = s.execute("library.validateDng", &json!({"ids": [id]})).unwrap();
    assert_eq!(v["failed"][0]["error"], "original offline");
    s.close_library().unwrap();
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
