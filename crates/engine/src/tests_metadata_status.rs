//! L22: metadata status — up to date / changed in the library / changed on disk / conflict, Scan
//! for Metadata Updates, resolving either way; the sync record survives a reopen (format v6).

use lightcraft_catalog::PhotoId;
use serde_json::json;

use crate::Session;

fn status(s: &Session, id: u64) -> &'static str {
    s.metadata_status(PhotoId(id)).id()
}

#[test]
fn metadata_status_follows_the_library_and_the_file() {
    let dir = std::env::temp_dir().join(format!("lc-metastatus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let file = dir.join("files/a.dng");
    std::fs::write(&file, crate::tests_xmp::synthetic_dng_with(None, lightcraft_meta::Metadata::default())).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let id = s.execute("library.import", &json!({"paths": [file.to_string_lossy()]})).unwrap()["imported"][0].as_u64().unwrap();
    assert_eq!(status(&s, id), "notWritten");
    s.execute("photo.saveMetadataToFile", &json!({"ids": [id]})).unwrap();
    assert_eq!(status(&s, id), "upToDate");
    // changed in the library
    s.execute("photo.setMeta", &json!({"ids": [id], "title": "Dawn"})).unwrap();
    assert_eq!(status(&s, id), "changed");
    let scan = s.execute("library.scanMetadata", &json!({})).unwrap();
    assert_eq!(scan["changed"], json!([id]), "{scan}");
    // and on disk too: a conflict
    let side = dir.join("files/a.xmp");
    let text = std::fs::read_to_string(&side).unwrap();
    std::fs::write(&side, text.replace("<xmp:Rating>0</xmp:Rating>", "<xmp:Rating>5</xmp:Rating>")).unwrap();
    assert_eq!(status(&s, id), "conflict");
    // Import Settings from Disk: the file wins (the title it has is the saved one: none)
    s.execute("photo.resolveMetadata", &json!({"ids": [id], "keep": "disk"})).unwrap();
    assert_eq!(status(&s, id), "upToDate");
    assert_eq!(s.catalog.photo(PhotoId(id)).unwrap().rating, 5);
    // changed only on disk, then Overwrite Settings
    let text = std::fs::read_to_string(&side).unwrap();
    std::fs::write(&side, text.replace("<dc:title>", "<dc:x>").replace("</dc:title>", "</dc:x>") + " ").unwrap();
    assert_eq!(status(&s, id), "changedOnDisk");
    s.execute("library.scanMetadata", &json!({})).unwrap();
    assert_eq!(s.metadata_status.get(&PhotoId(id)).map(|m| m.id()), Some("changedOnDisk"));
    s.execute("photo.resolveMetadata", &json!({"ids": [id], "keep": "library"})).unwrap();
    assert_eq!(status(&s, id), "upToDate");
    assert_eq!(s.metadata_status.get(&PhotoId(id)).map(|m| m.id()), Some("upToDate"), "the grid's status follows");
    assert!(s.execute("photo.resolveMetadata", &json!({"ids": [id], "keep": "both"})).is_err());
    // a later edit marks it in the scan's results
    s.execute("photo.rate", &json!({"ids": [id], "rating": 2})).unwrap();
    assert_eq!(s.metadata_status.get(&PhotoId(id)).map(|m| m.id()), Some("changed"));
    // the sidecar gone: changed on disk
    s.execute("photo.saveMetadataToFile", &json!({"ids": [id]})).unwrap();
    std::fs::remove_file(&side).unwrap();
    assert_eq!(status(&s, id), "changedOnDisk");
    // kept across a reopen
    s.execute("photo.saveMetadataToFile", &json!({"ids": [id]})).unwrap();
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    assert_eq!(status(&s, id), "upToDate");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Auto-write keeps the status up to date by itself.
#[test]
fn auto_write_records_what_it_wrote() {
    let dir = std::env::temp_dir().join(format!("lc-metastatus-auto-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("files")).unwrap();
    let file = dir.join("files/b.dng");
    std::fs::write(&file, crate::tests_xmp::synthetic_dng_with(None, lightcraft_meta::Metadata::default())).unwrap();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let id = s.execute("library.import", &json!({"paths": [file.to_string_lossy()]})).unwrap()["imported"][0].as_u64().unwrap();
    s.execute("library.xmpPreferences", &json!({"autoWrite": true})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [id], "caption": "Written by itself"})).unwrap();
    assert_eq!(status(&s, id), "upToDate");
    let _ = std::fs::remove_dir_all(&dir);
}
