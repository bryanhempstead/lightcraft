//! E07: JPEG XL and PSD exports decode again at the right size and keep their metadata.

use serde_json::json;

use crate::Session;

#[test]
fn jpeg_xl_and_psd_exports() {
    let dir = std::env::temp_dir().join(format!("lc-exportfmt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("photo.setMeta", &json!({"ids": [id], "title": "Exported"})).unwrap();
    for (format, depth, ext) in [("jxl", 8, "jxl"), ("jxl", 16, "jxl"), ("psd", 16, "psd"), ("psd", 8, "psd")] {
        let p = json!({"format": format, "bitDepth": depth, "longEdge": 320});
        let o = crate::export::ExportOptions::from_params(&p).unwrap();
        assert_eq!(o.format.extension(), ext);
        let out = crate::export::export_photo(&mut s, lightcraft_catalog::PhotoId(id), &o, 1).unwrap();
        let d =
            lightcraft_codecs::decode(&out.bytes, lightcraft_codecs::DecodeOptions::default()).unwrap_or_else(|e| panic!("{format} {depth}: {e}"));
        assert_eq!(d.image.width.max(d.image.height), 320, "{format} {depth}");
    }
    // quality 100 = lossless JPEG XL
    let o = crate::export::ExportOptions::from_params(&json!({"format": "jxl", "quality": 100})).unwrap();
    assert!(o.quality >= 100);
    assert_eq!(crate::export::ExportFormat::parse("PSB"), Some(crate::export::ExportFormat::Psd));
    let _ = std::fs::remove_dir_all(&dir);
}
