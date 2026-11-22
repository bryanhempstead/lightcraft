//! L21: IPTC Extension fields — set by command, written to and read back from the XMP sidecar,
//! exported (person fields left out with Remove Person Info), kept across a reopen.

use serde_json::json;

use crate::Session;

#[test]
fn iptc_extension_fields_round_trip_through_xmp_and_export() {
    let dir = std::env::temp_dir().join(format!("lc-iptcext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let file = dir.join("files/a.dng");
    std::fs::write(&file, crate::tests_xmp::synthetic_dng_with(None, lightcraft_meta::Metadata::default())).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [file.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().unwrap();
    let set = json!({"ids": [id], "iptcExt": {"personShown": ["Ann Lee", "Bo"], "event": "Wedding", "modelReleaseStatus": "MR-UMR", "digitalSourceType": "digitalCapture"}});
    s.execute("photo.setMeta", &set).unwrap();
    let p = s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().clone();
    assert_eq!(p.meta.iptc_ext["personShown"], "Ann Lee; Bo");
    assert_eq!(p.meta.iptc_ext["modelReleaseStatus"], "http://ns.useplus.org/ldf/vocab/MR-UMR");
    // bad input is refused and nothing changes
    assert!(s.execute("photo.setMeta", &json!({"ids": [id], "iptcExt": {"nope": "x"}})).is_err());
    assert!(s.execute("photo.setMeta", &json!({"ids": [id], "iptcExt": {"modelReleaseStatus": "maybe"}})).is_err());
    assert!(s.execute("photo.setMeta", &json!({"ids": [id], "iptcExt": "x"})).is_err());
    // the sidecar carries them; clearing the catalog and reading it back restores them
    s.execute("photo.saveMetadataToFile", &json!({"ids": [id]})).unwrap();
    let xmp = std::fs::read_to_string(dir.join("files/a.xmp")).unwrap();
    assert!(xmp.contains("Iptc4xmpExt:PersonInImage") && xmp.contains("<rdf:li>Bo</rdf:li>") && xmp.contains("MR-UMR"), "{xmp}");
    s.execute(
        "photo.setMeta",
        &json!({"ids": [id], "iptcExt": {"personShown": "", "event": null, "modelReleaseStatus": "", "digitalSourceType": ""}}),
    )
    .unwrap();
    assert!(s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().meta.iptc_ext.is_empty());
    s.execute("photo.readMetadataFromFile", &json!({"ids": [id]})).unwrap();
    let back = s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().meta.iptc_ext.clone();
    assert_eq!(back, p.meta.iptc_ext, "read back from the sidecar");
    // export: everything, or without the people
    let o = crate::export::ExportOptions::default();
    let m = crate::export::export_metadata(&p, &o).unwrap();
    assert_eq!(m.iptc_ext.get("event").map(String::as_str), Some("Wedding"));
    let o = crate::export::ExportOptions::from_params(&json!({"removePeople": true})).unwrap();
    let m = crate::export::export_metadata(&p, &o).unwrap();
    assert!(!m.iptc_ext.contains_key("personShown") && m.iptc_ext.contains_key("event"));
    let packet = lightcraft_meta::write_xmp(&m, None);
    let parsed = lightcraft_meta::parse_xmp(&packet).unwrap();
    assert_eq!(parsed.metadata.iptc_ext.get("digitalSourceType"), m.iptc_ext.get("digitalSourceType"));
    // kept across a reopen (catalog format v5)
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new();
    s.open_library(dir.join("lib"), false).unwrap();
    assert_eq!(s.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().meta.iptc_ext, p.meta.iptc_ext);
    let _ = std::fs::remove_dir_all(&dir);
}
