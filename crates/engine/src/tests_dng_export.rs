//! LR-EXP-DNGOPT: DNG export's JPEG Preview (none / medium / full size) and lossy DNG, written
//! through Adobe's DNG SDK with the photo's own render as the preview; the edits' XMP survives
//! the SDK (built without XMP); without the SDK the DNG is written as before.

use serde_json::json;

use crate::Session;
use crate::export::{Destination, ExportOptions, export_batch};

/// Export the library's one photo as a DNG with `opts` → the file's bytes.
fn export(s: &mut Session, id: lightcraft_catalog::PhotoId, opts: serde_json::Value) -> Vec<u8> {
    let mut p = json!({"format": "dng"});
    if let (Some(o), Some(extra)) = (p.as_object_mut(), opts.as_object()) {
        o.extend(extra.clone());
    }
    let o = ExportOptions::from_json(&p);
    let mut out = Vec::new();
    let mut write = |_: &str, b: &[u8]| {
        out = b.to_vec();
        Ok(())
    };
    export_batch(s, &[id], &o, &Destination { dir: "x".into(), exact: None }, &mut write, &|_| false).unwrap();
    out
}

/// Every IFD of a TIFF (IFD0, its SubIFDs, the chain).
fn ifds(t: &lightcraft_tiff::Tiff) -> Vec<lightcraft_tiff::Ifd> {
    fn walk(i: &lightcraft_tiff::Ifd, out: &mut Vec<lightcraft_tiff::Ifd>) {
        out.push(i.clone());
        for s in &i.sub_ifds {
            walk(s, out);
        }
    }
    let mut v = Vec::new();
    for i in &t.ifds {
        walk(i, &mut v);
    }
    v
}

fn u(i: &lightcraft_tiff::Ifd, tag: u16) -> Option<u64> {
    i.value(tag).and_then(|v| v.get_u64(0))
}

#[test]
fn dng_preview_sizes_and_lossy() {
    let dir = std::env::temp_dir().join(format!("lc-dngopt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("a.dng");
    std::fs::write(&src, crate::tests_xmp::synthetic_dng_with(None, lightcraft_meta::Metadata::default())).unwrap();
    let mut s = Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap();
    let id = lightcraft_catalog::PhotoId(r["imported"][0].as_u64().unwrap());
    s.execute("photo.setMeta", &json!({"ids": [id.0], "title": "Lake at dawn"})).unwrap();
    s.execute("develop.set", &json!({"ids": [id.0], "control": "light.exposure", "value": 0.7})).unwrap();
    // the option round-trips and nonsense is refused
    let o = ExportOptions::from_json(&json!({"format": "dng", "dngPreview": "full", "dngLossy": true}));
    assert_eq!((o.dng_preview, o.dng_lossy), (crate::export::DngPreview::Full, true));
    assert_eq!(ExportOptions::from_json(&o.to_json()).dng_preview, crate::export::DngPreview::Full);
    assert!(ExportOptions::validate("app.export", &json!({"dngPreview": "huge"})).is_err());
    assert!(ExportOptions::validate("app.export", &json!({"dngLossy": "yes"})).is_err());

    let none = export(&mut s, id, json!({"dngPreview": "none"}));
    let medium = export(&mut s, id, json!({}));
    let full = export(&mut s, id, json!({"dngPreview": "full"}));
    let lossy = export(&mut s, id, json!({"dngPreview": "none", "dngLossy": true}));
    for (what, b) in [("none", &none), ("medium", &medium), ("full", &full), ("lossy", &lossy)] {
        // the edits ride along in the XMP whatever wrote the file
        let t = lightcraft_tiff::Tiff::parse(b).unwrap_or_else(|e| panic!("{what}: {e}"));
        let xmp = t.ifds[0].value(700).and_then(|v| v.as_bytes()).map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default();
        assert!(xmp.contains("Lake at dawn") && xmp.contains("0.7"), "{what}: XMP kept: {}", xmp.len());
    }
    // the plain one decodes with LightCraft's own reader (raw data untouched)
    assert!(lightcraft_raw::decode(&none).is_ok());
    // the Metadata panel's DNG view reads the header back
    let info = s.execute("photo.dngInfo", &json!({"id": id.0})).unwrap();
    assert_eq!((info["dng"].as_bool(), info["mosaic"].as_bool()), (Some(true), Some(true)), "{info}");
    assert_eq!(info["compression"], "lossless JPEG", "{info}");
    assert!(info["version"].as_str().is_some_and(|v| v.starts_with("1.")), "{info}");
    let mut demo = Session::with_demo();
    let did = demo.catalog.photos().next().unwrap().id.0;
    assert_eq!(demo.execute("photo.dngInfo", &json!({"id": did})).unwrap()["dng"], false);
    let imported = |s: &mut Session, name: &str, b: &[u8]| -> serde_json::Value {
        let f = dir.join(name);
        std::fs::write(&f, b).unwrap();
        let r = s.execute("library.import", &json!({"paths": [f.to_string_lossy()]})).unwrap();
        let nid = r["imported"][0].as_u64().unwrap_or_else(|| panic!("{name}: {r}"));
        s.execute("photo.dngInfo", &json!({"id": nid})).unwrap()
    };
    if !lightcraft_dng_sdk_sys::available() {
        // no SDK: every one is the plain DNG (a notice is logged), still a valid raw
        for b in [&medium, &full, &lossy] {
            assert!(lightcraft_raw::decode(b).is_ok());
        }
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let previews = |b: &[u8]| -> Vec<(u64, u64, u64)> {
        let t = lightcraft_tiff::Tiff::parse(b).unwrap();
        // NewSubFileType 1 = a reduced-resolution image: (compression, width, height)
        ifds(&t).iter().filter(|i| u(i, 254) == Some(1)).map(|i| (u(i, 259).unwrap_or(0), u(i, 256).unwrap_or(0), u(i, 257).unwrap_or(0))).collect()
    };
    let (pn, pm, pf) = (previews(&none), previews(&medium), previews(&full));
    assert_eq!(pn.len(), 0, "None: no preview beyond LightCraft's own file: {pn:?}");
    assert!(pm.iter().any(|p| p.0 == 7), "Medium: a JPEG preview: {pm:?}");
    assert!(pm.len() >= 2, "and the thumbnail: {pm:?}");
    let raw = lightcraft_raw::decode(&none).unwrap();
    assert!(pf.iter().any(|p| p.0 == 7 && p.1.max(p.2) as usize == raw.width.max(raw.height)), "Full Size: {pf:?} vs {}x{}", raw.width, raw.height);
    // the SDK reads them all back as DNGs; the lossy one stores 8-bit lossy JPEG (34892) linear raw
    for b in [&medium, &full, &lossy] {
        assert!(lightcraft_dng_sdk_sys::render_dng(b, None, 64, false).is_ok());
    }
    let t = lightcraft_tiff::Tiff::parse(&lossy).unwrap();
    let main = ifds(&t).into_iter().find(|i| u(i, 254) == Some(0)).expect("the main image");
    assert_eq!(u(&main, 259), Some(34892), "lossy JPEG");
    assert_eq!(u(&main, 262), Some(34892), "linear raw (demosaiced)");
    let mi = imported(&mut s, "medium.dng", &medium);
    assert!(mi["previews"].as_array().is_some_and(|p| p.iter().any(|x| x["jpeg"] == true)), "{mi}");
    let li = imported(&mut s, "lossy.dng", &lossy);
    assert_eq!((li["compression"].as_str(), li["mosaic"].as_bool()), (Some("lossy JPEG"), Some(false)), "{li}");
    let _ = std::fs::remove_dir_all(&dir);
}
