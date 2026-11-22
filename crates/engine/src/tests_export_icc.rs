//! E08: exports converted to ColorMatch RGB or an ICC profile file, tagged with it.

use serde_json::json;

use crate::Session;

#[test]
fn colormatch_and_profile_file_exports() {
    let dir = std::env::temp_dir().join(format!("lc-exporticc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_demo();
    let id = lightcraft_catalog::PhotoId(s.catalog.photos().next().unwrap().id.0);
    let cm = lightcraft_codecs::icc::colormatch_profile();
    let info = lightcraft_codecs::icc::parse(&cm).expect("a profile that parses");
    assert!(format!("{info:?}").contains("Rgb"), "{info:?}");
    for (format, depth) in [("png", 8), ("tiff", 16), ("jpeg", 8)] {
        let o = crate::export::ExportOptions::from_params(&json!({"format": format, "bitDepth": depth, "longEdge": 200, "iccProfile": "colorMatch"}))
            .unwrap();
        assert_eq!(o.effective_space(), lightcraft_pipeline::OutputSpace::ProPhoto, "rendered wide first");
        let out = crate::export::export_photo(&mut s, id, &o, 1).unwrap();
        let d = lightcraft_codecs::decode(&out.bytes, lightcraft_codecs::DecodeOptions::default()).unwrap();
        assert_eq!(d.image.width.max(d.image.height), 200, "{format}");
        // (PNG stores its profile compressed)
        assert!(format == "png" || out.bytes.windows(14).any(|w| w == b"ColorMatch RGB"), "{format}: tagged with ColorMatch");
    }
    // a profile file works the same; a missing or non-RGB one is an error, not a crash
    let path = dir.join("cm.icc");
    std::fs::write(&path, &cm).unwrap();
    let o = crate::export::ExportOptions::from_params(&json!({"format": "png", "longEdge": 100, "iccProfile": path.to_string_lossy()})).unwrap();
    assert!(crate::export::export_photo(&mut s, id, &o, 1).is_ok());
    let o = crate::export::ExportOptions::from_params(&json!({"format": "png", "iccProfile": dir.join("nope.icc").to_string_lossy()})).unwrap();
    assert!(crate::export::export_photo(&mut s, id, &o, 1).is_err());
    std::fs::write(dir.join("junk.icc"), b"not a profile").unwrap();
    let o = crate::export::ExportOptions::from_params(&json!({"format": "png", "iccProfile": dir.join("junk.icc").to_string_lossy()})).unwrap();
    assert!(crate::export::export_photo(&mut s, id, &o, 1).is_err());
    assert!(crate::export::ExportOptions::from_params(&json!({"iccProfile": 3})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
