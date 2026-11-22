//! R14: Lightroom's stored Upright transforms and Constrain Crop.

use super::*;
use crate::Session;
use lightcraft_geom::{Homography, Point};
use serde_json::json;

/// A Level transform as Lightroom stores it (a 3:2 frame turned 0.76°, with its shrink-to-fit).
const LEVEL: &str = "1.019613060,-0.009003966,-0.005304547,0.020177806,1.019613060,-0.019895433,0.000000000,0.000000000,1.000000000";
const IDENTITY: &str = "1.000000000,0.000000000,0.000000000,0.000000000,1.000000000,0.000000000,0.000000000,0.000000000,1.000000000";

fn upright_text(mode: u8, level: &str) -> String {
    format!(
        "s = {{ PerspectiveUpright = {mode},\nCropConstrainToWarp = 1,\nUprightTransformCount = 6,\nUprightTransform_0 = \"{IDENTITY}\",\nUprightTransform_1 = \"{IDENTITY}\",\nUprightTransform_2 = \"{IDENTITY}\",\nUprightTransform_3 = \"{level}\",\nUprightTransform_4 = \"{IDENTITY}\",\nUprightTransform_5 = \"{IDENTITY}\",\nUprightVersion = 151388160 }}"
    )
}

fn homography(v: &serde_json::Value) -> Homography {
    let m: Vec<f64> = v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    Homography(m.try_into().unwrap())
}

/// Feature (R14): Upright modes are Lightroom's (3 = Level), and the transform Lightroom stored is
/// used (inverted into ours: lens-corrected → transformed, centred coordinates), not re-detected.
#[test]
fn upright_takes_lightrooms_stored_transform() {
    let m = map_develop(&upright_text(3, LEVEL), true, 1.5).unwrap();
    let g = &m.partial["geometry"];
    assert_eq!(g["upright"], json!("level"));
    assert_eq!(g["constrain_crop"], json!(true));
    assert!(m.unmapped.is_empty(), "{:?}", m.unmapped);
    let h = homography(&g["upright_transform"]);
    // Lightroom's matrix maps an output point (0..1 per axis) to the source; ours maps it back
    let lr = Homography(LEVEL.split(',').map(|x| x.parse().unwrap()).collect::<Vec<f64>>().try_into().unwrap());
    let centred = |p: Point| Point::new(2.0 * p.x - 1.0, (2.0 * p.y - 1.0) / 1.5);
    for (x, y) in [(0.5, 0.5), (0.1, 0.2), (0.9, 0.7), (0.0, 1.0)] {
        let out = Point::new(x, y);
        let back = h.apply(centred(lr.apply(out)));
        let want = centred(out);
        assert!((back.x - want.x).abs() < 1e-9 && (back.y - want.y).abs() < 1e-9, "{back:?} vs {want:?}");
    }
    // Lightroom's shrink-to-fit is kept: our transform makes the photo a little smaller
    assert!(h.0[0] < 0.99 && h.0[0] > 0.97, "{:?}", h.0);
    // identity stays identity (Lightroom found nothing to level: nothing is re-detected)
    let m = map_develop(&upright_text(3, IDENTITY), true, 1.5).unwrap();
    assert_eq!(homography(&m.partial["geometry"]["upright_transform"]), Homography::IDENTITY);
    // 4 = Vertical, 2 = Full
    assert_eq!(map_develop(&upright_text(4, LEVEL), true, 1.5).unwrap().partial["geometry"]["upright"], json!("vertical"));
    assert_eq!(map_develop(&upright_text(2, LEVEL), true, 1.5).unwrap().partial["geometry"]["upright"], json!("full"));
    // and the settings carry it
    let partial = map_develop(&upright_text(3, LEVEL), true, 1.5).unwrap().partial;
    let d = lightcraft_develop::apply_partial(&lightcraft_develop::DevelopSettings::default(), &partial, 1.0);
    assert_eq!(d.geometry.upright, lightcraft_develop::Upright::Level);
    assert!(d.geometry.upright_transform.is_some() && d.geometry.constrain_crop);
}

/// Hostile input: a corrupt stored transform is ignored (our own Upright analysis runs), never a panic.
#[test]
fn corrupt_upright_transforms_are_ignored() {
    for bad in [
        "",
        "1,2,3",
        "nan,0,0,0,1,0,0,0,1",
        "1e300,0,0,0,1,0,0,0,1",
        "0,0,0,0,0,0,0,0,0",
        "1,0,0,0,1,0,0,0,0",
        "a,b,c,d,e,f,g,h,i",
        "1,0,0,0,1,0,0,0,1,5",
    ] {
        let m = map_develop(&upright_text(3, bad), true, 1.5).unwrap();
        assert!(m.partial["geometry"].get("upright_transform").is_none(), "{bad}: {}", m.partial["geometry"]);
        assert_eq!(m.partial["geometry"]["upright"], json!("level"));
    }
    for aspect in [0.0, f64::NAN, -1.0, f64::INFINITY] {
        assert!(map_develop(&upright_text(3, LEVEL), true, aspect).is_ok());
    }
}

/// A photo Lightroom shows turned: the stored-frame transform is conjugated into the shown frame.
#[test]
fn upright_transform_turns_with_the_photo() {
    let mut partial = map_develop(&upright_text(3, LEVEL), true, 1.5).unwrap().partial;
    let stored = homography(&partial["geometry"]["upright_transform"]);
    let o = lightcraft_geom::Orientation::from_parts(false, 3);
    reorient_partial(&mut partial, o);
    let shown = homography(&partial["geometry"]["upright_transform"]);
    let turn = |p: Point| {
        let (x, y) = o.map(p.x, p.y, 0.0, 0.0);
        Point::new(x, y)
    };
    for (x, y) in [(0.3, -0.2), (-0.8, 0.5), (0.0, 0.0)] {
        let p = Point::new(x, y);
        let (a, b) = (shown.apply(turn(p)), turn(stored.apply(p)));
        assert!((a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
    }
}

/// `library.repairLightroomMigration` gives migrated photos Lightroom's Upright (an earlier version read
/// Level as Vertical and re-detected the transform); a photo changed here since is left alone.
#[test]
fn repair_sets_lightrooms_upright() {
    let dir = std::env::temp_dir().join(format!("lc-lrupright-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    for name in ["a.png", "b.png"] {
        let v = if name == "a.png" { 100 } else { 40 };
        let img = lightcraft_raster::Rgba8::filled(24, 16, [v, v, v, 255]);
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(photos.join(name), bytes).unwrap();
    }
    let root = format!("{}/", dir.display());
    let dev = upright_text(3, IDENTITY);
    let images = vec![
        json!({"id": 1, "root": root, "folder": "shoot/", "base": "a", "ext": "png", "develop": dev, "edits": 2}),
        json!({"id": 2, "root": root, "folder": "shoot/", "base": "b", "ext": "png", "develop": dev, "edits": 2}),
    ];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    let id = |s: &Session, n: char| s.catalog.photos().find(|p| p.file_name.starts_with(n)).unwrap().id;
    let (a, b) = (id(&s, 'a'), id(&s, 'b'));
    let g = s.catalog.photo(a).unwrap().develop.geometry.clone();
    assert_eq!((g.upright, g.upright_transform), (lightcraft_develop::Upright::Level, Some(Homography::IDENTITY)));
    // an older migration's result: Vertical, no stored transform
    for x in [a, b] {
        let p = s.catalog.photo(x).unwrap().clone();
        let mut d = (*p.develop).clone();
        d.geometry.upright = lightcraft_develop::Upright::Vertical;
        d.geometry.upright_transform = None;
        d.geometry.constrain_crop = false;
        s.catalog
            .apply(lightcraft_catalog::Op::SetDevelop { id: x, settings: std::sync::Arc::new(d), label: "old".into(), edited: p.edited.clone() })
            .unwrap();
    }
    // b was edited here afterwards
    s.selection = crate::Selection::single(b);
    s.execute("develop.merge", &json!({"settings": {"light": {"exposure": 0.5}}})).unwrap();
    let r = s.execute("library.repairLightroomMigration", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!((r["uprightFixed"].clone(), r["keptChangedHere"].clone()), (json!(1), json!(1)), "{r}");
    assert_eq!(s.catalog.photo(a).unwrap().develop.geometry.upright, lightcraft_develop::Upright::Vertical, "dry run");
    s.execute("library.repairLightroomMigration", &json!({"records": rec_s})).unwrap();
    let g = s.catalog.photo(a).unwrap().develop.geometry.clone();
    assert_eq!((g.upright, g.upright_transform, g.constrain_crop), (lightcraft_develop::Upright::Level, Some(Homography::IDENTITY), true));
    assert_eq!(s.catalog.photo(b).unwrap().develop.geometry.upright, lightcraft_develop::Upright::Vertical, "changed here: kept");
    assert_eq!(s.execute("library.repairLightroomMigration", &json!({"records": rec_s})).unwrap()["uprightFixed"], json!(0), "nothing twice");
    let _ = std::fs::remove_dir_all(&dir);
}
