//! `library.repairLightroomImport`: straightened crops and grouped radials of earlier migrations.

use std::sync::Arc;

use lightcraft_catalog::{Op, PhotoId};
use lightcraft_develop::MaskShape;
use serde_json::json;

use crate::Session;
use crate::lr_migrate::records_from_rows;

/// A straightened crop and a grouped radial with `Flipped = true` (values written for this test).
const DEVELOP: &str = r#"s = { CropAngle = 2.5,
CropBottom = 0.85,
CropLeft = 0.12,
CropRight = 0.9,
CropTop = 0.1,
Exposure2012 = 0.2,
HasCrop = true,
MaskGroupBasedCorrections = { { What = "Correction", CorrectionActive = true, CorrectionAmount = 1, CorrectionName = "Glow",
  LocalExposure2012 = 0.3,
  CorrectionMasks = { { What = "Mask/CircularGradient", Top = 0.3, Left = 0.2, Bottom = 0.7, Right = 0.8, Angle = 10, Feather = 40,
    Flipped = true, MaskInverted = false, MaskBlendMode = 0 } } } } }"#;

fn png(path: &std::path::Path, v: u8) {
    let img = lightcraft_raster::Rgba8::filled(30, 20, [v, v, v, 255]);
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn radial_invert(s: &Session, id: PhotoId) -> bool {
    match &s.catalog.photo(id).unwrap().develop.masks[0].components[0].shape {
        MaskShape::Radial { invert, .. } => *invert,
        other => panic!("{other:?}"),
    }
}

/// Set photo `id`'s develop settings the way an earlier migration left them (no undo step).
fn set(s: &mut Session, id: PhotoId, f: impl FnOnce(&mut lightcraft_develop::DevelopSettings)) {
    let p = s.catalog.photo(id).unwrap().clone();
    let mut d = (*p.develop).clone();
    f(&mut d);
    s.catalog.apply(Op::SetDevelop { id, settings: Arc::new(d), label: "old".into(), edited: p.edited.clone() }).unwrap();
}

#[test]
fn repair_fixes_what_earlier_migrations_read_wrong_and_skips_edits_here() {
    let dir = std::env::temp_dir().join(format!("lc-lrrepair-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    for (name, v) in [("a.png", 120u8), ("b.png", 60), ("c.png", 90), ("d.png", 30)] {
        png(&photos.join(name), v);
    }
    let root = format!("{}/", dir.display());
    let images: Vec<_> = ['a', 'b', 'c', 'd']
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"id": i + 1, "root": root, "folder": "shoot/", "base": n.to_string(), "ext": "png", "develop": DEVELOP, "edits": 3, "width": 30, "height": 20}))
        .collect();
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    let id = |s: &Session, n: char| s.catalog.photos().find(|p| p.file_name.starts_with(n)).unwrap().id;
    let (a, b, c, d) = (id(&s, 'a'), id(&s, 'b'), id(&s, 'c'), id(&s, 'd'));
    // a fresh migration reads both right: the crop as two corners, the grouped radial not inverted
    let right = s.catalog.photo(a).unwrap().develop.crop.geometry;
    let corners = crate::crs::crop_from_lr([0.12, 0.1, 0.9, 0.85], 2.5, 1.5).unwrap();
    assert_eq!(right, corners);
    assert!(!radial_invert(&s, a));
    // what earlier versions left: the edges as a rectangle (b: the angle also mirrored), the radial inverted
    let old = lightcraft_geom::CropGeometry { rect: lightcraft_geom::Rect { x0: 0.12, y0: 0.1, x1: 0.9, y1: 0.85 }, angle: -2.5 };
    for (p, angle) in [(a, -2.5), (b, 2.5), (c, -2.5), (d, -2.5)] {
        set(&mut s, p, |dv| {
            dv.crop.geometry = lightcraft_geom::CropGeometry { angle, ..old };
            if let MaskShape::Radial { invert, .. } = &mut dv.masks[0].components[0].shape {
                *invert = true;
            }
        });
    }
    // changed here since: c's crop was redrawn, d's radial moved
    s.selection = crate::Selection::single(c);
    s.execute("develop.merge", &json!({"settings": {"crop": {"geometry": {"rect": {"x0": 0.2, "y0": 0.2, "x1": 0.8, "y1": 0.8}, "angle": 1.0}}}}))
        .unwrap();
    set(&mut s, d, |dv| {
        if let MaskShape::Radial { center, .. } = &mut dv.masks[0].components[0].shape {
            center.x += 0.1;
        }
    });
    let before = |s: &Session| [a, b, c, d].map(|p| s.catalog.photo(p).unwrap().develop.clone());
    let was = before(&s);

    let r = s.execute("library.repairLightroomImport", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!(r["crops"], json!({"fixed": 3, "alreadyRight": 0, "skippedChangedHere": 1}), "{r}");
    assert_eq!(r["radials"], json!({"fixed": 3, "photos": 3, "alreadyRight": 0, "skippedChangedHere": 1}), "{r}");
    assert_eq!(r["photosChanged"], json!(4), "{r}");
    let ch = r["changes"].as_array().unwrap();
    let pa = ch.iter().find(|e| e["photo"] == json!(a.0)).unwrap();
    assert_eq!(pa["crop"]["before"]["x0"], json!(0.12));
    assert_eq!(pa["crop"]["after"]["x0"], json!(right.rect.x0));
    assert_eq!(pa["radials"][0]["before"]["invert"], json!(true));
    assert_eq!(pa["radials"][0]["after"]["invert"], json!(false));
    let sk = r["skipped"].as_array().unwrap();
    assert!(sk.iter().any(|e| e["photo"] == json!(c.0) && e["what"] == json!(["crop"])), "{r}");
    assert!(sk.iter().any(|e| e["photo"] == json!(d.0) && e["what"] == json!(["masks"])), "{r}");
    assert_eq!(before(&s), was, "a dry run changes nothing");

    let undo_depth = s.undo.len();
    s.execute("library.repairLightroomImport", &json!({"records": rec_s})).unwrap();
    assert_eq!(s.undo.len(), undo_depth + 1, "one undo step");
    for p in [a, b, d] {
        assert_eq!(s.catalog.photo(p).unwrap().develop.crop.geometry, right, "{p:?}");
    }
    assert_eq!(s.catalog.photo(c).unwrap().develop.crop.geometry.angle, 1.0, "c's own crop is kept");
    for p in [a, b, c] {
        assert!(!radial_invert(&s, p), "{p:?}");
    }
    assert!(radial_invert(&s, d), "d's own radial is kept");
    // the rest of the photo is untouched
    assert_eq!(s.catalog.photo(a).unwrap().develop.light.exposure, was[0].light.exposure);
    assert_eq!(s.catalog.photo(a).unwrap().develop.masks[0].adjust, was[0].masks[0].adjust);
    // nothing twice
    let again = s.execute("library.repairLightroomImport", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!(again["photosChanged"], json!(0), "{again}");
    assert_eq!(again["crops"]["alreadyRight"], json!(3), "{again}");
    // undo puts everything back
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(before(&s), was);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A photo Lightroom shows turned (orientation `DA`, a quarter turn counter-clockwise) has its crop
/// read in the stored frame and turned with it; the repair compares in that frame too.
#[test]
fn turned_photos_are_compared_in_their_own_frame() {
    let dir = std::env::temp_dir().join(format!("lc-lrrepair-turn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shoot")).unwrap();
    png(&dir.join("shoot/a.png"), 100);
    let root = format!("{}/", dir.display());
    let images = vec![json!({"id": 1, "root": root, "folder": "shoot/", "base": "a", "ext": "png", "develop": DEVELOP, "edits": 3,
        "orientation": "DA", "width": 30, "height": 20})];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    let a = s.catalog.photos().next().unwrap().id;
    let right = s.catalog.photo(a).unwrap().develop.crop.geometry;
    // the old reading, turned the same way
    let mut v = json!({"crop": {"geometry": {"rect": {"x0": 0.12, "y0": 0.1, "x1": 0.9, "y1": 0.85}, "angle": -2.5}}});
    crate::lr_migrate::reorient_partial(&mut v, lightcraft_geom::Orientation::Rotate270);
    let old: lightcraft_geom::CropGeometry = serde_json::from_value(v["crop"]["geometry"].clone()).unwrap();
    assert_ne!(old, right);
    set(&mut s, a, |d| d.crop.geometry = old);
    let r = s.execute("library.repairLightroomImport", &json!({"records": rec_s})).unwrap();
    assert_eq!(r["crops"]["fixed"], json!(1), "{r}");
    assert_eq!(s.catalog.photo(a).unwrap().develop.crop.geometry, right);
    let _ = std::fs::remove_dir_all(&dir);
}
