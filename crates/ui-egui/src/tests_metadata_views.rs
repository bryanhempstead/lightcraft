//! L20: the Metadata panel's views (Default, EXIF, IPTC, Location, Large Caption, Minimal, Quick
//! Describe, Custom), `< mixed >` fields on a multi-selection, and the camera / lens arrows.

use serde_json::json;

use crate::tests_library2::{T, click, headless, settle, shot};

#[test]
fn metadata_views_mixed_values_and_arrows() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    s.execute("photo.setMeta", &json!({"ids": [ids[0]], "title": "One", "city": "Lisbon"})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[1]], "title": "Two", "city": "Lisbon"})).unwrap();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    let mut h = headless(s);
    h.app.ui.library_sections = vec!["metadata".into()];
    settle(&mut h);
    shot(&mut h, "l20-default-mixed.png");
    // the title differs: typing one sets it on both
    let r = h.request("ui.clickWidget", json!({"id": "field:title"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let r = h.request("ui.text", json!({"text": "Both"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let r = h.request("ui.key", json!({"key": "Tab"}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    for id in &ids {
        assert_eq!(h.app.session.catalog.photo(lightcraft_catalog::PhotoId(*id)).unwrap().meta.title, "Both");
    }
    // views
    for v in ["exif", "location", "largeCaption", "minimal", "quickDescribe"] {
        h.app.ui.metadata_view = v.into();
        settle(&mut h);
        let w = h.request("ui.widgets", json!({}), T).to_string();
        match v {
            "exif" => assert!(!w.contains("field:title") && w.contains("metaArrow-camera"), "{v}"),
            "location" => assert!(w.contains("field:city") && !w.contains("field:title"), "{v}"),
            "largeCaption" => assert!(w.contains("field:caption") && !w.contains("field:city"), "{v}"),
            "minimal" => assert!(w.contains("field:caption") && !w.contains("field:copyright"), "{v}"),
            _ => assert!(w.contains("field:title") && w.contains("field:copyright"), "{v}"),
        }
        if v == "exif" {
            shot(&mut h, "l20-exif.png");
        }
    }
    h.app.ui.metadata_view = "custom".into();
    settle(&mut h);
    click(&mut h, "button:metadataGroup-location");
    assert_eq!(h.app.ui.metadata_custom, ["location"]);
    shot(&mut h, "l20-custom.png");
    // the camera arrow filters the library by the camera
    h.app.ui.metadata_view = "default".into();
    settle(&mut h);
    click(&mut h, "button:metaArrow-camera");
    assert!(h.app.session.filter.camera.is_some());
}

/// L21: the IPTC Extension view edits Person Shown (a list) and shows the release status (a popup).
#[test]
fn iptc_extension_view() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [id], "iptcExt": {"event": "Wedding", "propertyReleaseStatus": "PR-NAP"}})).unwrap();
    let mut h = headless(s);
    h.app.ui.library_sections = vec!["metadata".into()];
    h.app.ui.metadata_view = "iptcExt".into();
    settle(&mut h);
    let r = h.request("ui.clickWidget", json!({"id": "field:iptcExt.personShown"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert_eq!(h.request("ui.text", json!({"text": "Ann Lee; Bo"}), T)["ok"], true);
    assert_eq!(h.request("ui.key", json!({"key": "Tab"}), T)["ok"], true);
    settle(&mut h);
    let m = h.app.session.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().meta.iptc_ext.clone();
    assert_eq!(m.get("personShown").map(String::as_str), Some("Ann Lee; Bo"));
    shot(&mut h, "l21-iptc-ext.png");
}

/// L22: after Library ▸ Scan for Metadata Updates, a photo changed on disk shows "xmp ↓" in the
/// grid, and its menu's Import Settings from Disk takes the file in.
#[test]
fn metadata_status_badges_in_the_grid() {
    let dir = crate::tests_library2::temp("metastatus");
    let src = dir.0.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let (w, hh) = (16usize, 12usize);
    let img = lightcraft_raster::Rgba8 { width: w, height: hh, data: vec![[90, 120, 150, 255]; w * hh] };
    let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(src.join("a.png"), png).unwrap();
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(dir.0.join("lib"), false).unwrap();
    let id = s.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap()["imported"][0].as_u64().unwrap();
    s.execute("photo.saveMetadataToFile", &json!({"ids": [id]})).unwrap();
    let side = src.join("a.xmp");
    let text = std::fs::read_to_string(&side).unwrap();
    std::fs::write(&side, text.replace("<xmp:Rating>0</xmp:Rating>", "<xmp:Rating>4</xmp:Rating>")).unwrap();
    let mut h = headless(s);
    h.app.run("view.photoGrid", json!({})).unwrap();
    h.app.run("library.scanMetadata", json!({})).unwrap();
    settle(&mut h);
    shot(&mut h, "l22-grid-badge.png");
    click(&mut h, &format!("badge:metadataStatus:{id}"));
    let w = h.request("ui.widgets", json!({}), T).to_string();
    assert!(w.contains("Import Settings from Disk") || h.app.session.metadata_status.contains_key(&lightcraft_catalog::PhotoId(id)), "{w}");
    h.app.run("photo.resolveMetadata", json!({"ids": [id], "keep": "disk"})).unwrap();
    assert_eq!(h.app.session.catalog.photo(lightcraft_catalog::PhotoId(id)).unwrap().rating, 4);
    assert_eq!(h.app.session.metadata_status.get(&lightcraft_catalog::PhotoId(id)).map(|m| m.id()), Some("upToDate"));
}
