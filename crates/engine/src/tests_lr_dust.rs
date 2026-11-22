//! R26: photos Lightroom cleaned with Dust Removal get Find Dust Spots in Lightroom's areas.

use super::*;
use crate::Session;
use serde_json::json;

fn patch(l: u32, t: u32, r: u32, b: u32) -> String {
    format!(
        "{{ Alpha = {{ {{ PixelType = 1, SizeX = {w}, SizeY = {h}, Stage3X = 900, Stage3Y = 600 }} }}, ReferenceImageAreaLeft = {l}, ReferenceImageAreaTop = {t}, ReferenceImageAreaRight = {r}, ReferenceImageAreaBottom = {b} }}",
        w = r.saturating_sub(l),
        h = b.saturating_sub(t)
    )
}

fn develop(patches: &[String]) -> String {
    format!(
        "s = {{ Exposure2012 = 0,\nFilterList = {{ Filters = {{ {{ FilterID = 6,\nImages = {{ {} }},\nName = \"Dust Removal\",\nTitle = \"$$$/CRaw/Filter/DustRemoval/FilterPanelTitle=Dust Removal\" }} }} }} }}",
        patches.join(", ")
    )
}

#[test]
fn dust_areas_are_read_and_hostile_ones_skipped() {
    let a = dust_areas(&develop(&[patch(150, 110, 210, 170), patch(790, 70, 810, 90)]));
    assert_eq!(a.len(), 2);
    assert!((a[0].x0 - 150.0 / 900.0).abs() < 1e-9 && (a[0].y1 - 170.0 / 600.0).abs() < 1e-9);
    // other filters, broken numbers, inverted rects, no Lua at all
    assert!(dust_areas("s = { FilterList = { Filters = { { Name = \"Enhance\", Images = { } } } } }").is_empty());
    assert!(dust_areas(&develop(&[patch(300, 300, 200, 200)])).is_empty());
    assert!(dust_areas(&develop(&["{ Alpha = { { Stage3X = 0, Stage3Y = 0 } }, ReferenceImageAreaLeft = 1 }".into()])).is_empty());
    assert!(dust_areas("Dust Removal s = { {{{{").is_empty());
    assert!(dust_areas("").is_empty());
}

/// Feature (R26): dust found inside Lightroom's patches is healed (a patch with nothing found
/// gets nothing), dust outside them is left; one undo step; nothing twice.
#[test]
fn dust_removal_photos_get_find_dust_spots() {
    let dir = std::env::temp_dir().join(format!("lc-lrdust-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shoot")).unwrap();
    let (w, h) = (900usize, 600usize);
    let dust = [(180.0f32, 140.0f32), (600.0, 300.0)];
    let data: Vec<[u8; 4]> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32, (i / w) as f32);
            let mut v = 0.5 + 0.3 * (y / h as f32);
            for (cx, cy) in dust {
                v -= 0.08 * (-((x - cx).powi(2) + (y - cy).powi(2)) / (2.0 * 9.0)).exp();
            }
            let b = (v * 255.0) as u8;
            [b, b, (b as f32 * 1.15).min(255.0) as u8, 255]
        })
        .collect();
    let png = lightcraft_codecs::encode_png(
        &lightcraft_codecs::EncodeImage::rgba8(&lightcraft_raster::Rgba8 { width: w, height: h, data }),
        &Default::default(),
    )
    .unwrap();
    std::fs::write(dir.join("shoot").join("sky.png"), png).unwrap();
    let root = format!("{}/", dir.display());
    let dev = develop(&[patch(150, 110, 210, 170), patch(790, 70, 810, 90)]);
    let images = vec![json!({"id": 1, "root": root, "folder": "shoot/", "base": "sky", "ext": "png", "develop": dev, "edits": 1})];
    let rec = crate::lr_migrate::records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false, "dust": false})).unwrap();
    let id = s.catalog.photos().next().unwrap().id;
    let r = s.execute("library.lightroomDust", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!((r["photos"].clone(), r["spots"].clone(), r["outsideLightroomAreas"].clone()), (json!(1), json!(1), json!(1)), "{r}");
    assert!(s.develop_of(id).unwrap().spots.is_empty(), "dry run");
    s.execute("library.lightroomDust", &json!({"records": rec_s})).unwrap();
    let spots = s.develop_of(id).unwrap().spots.clone();
    assert_eq!(spots.len(), 1);
    assert!(spots.iter().any(|sp| (sp.points[0].x - 0.2).abs() < 0.02 && (sp.points[0].y - 140.0 / 600.0).abs() < 0.03), "{spots:?}");
    assert!(!spots.iter().any(|sp| (sp.points[0].x - 600.0 / 900.0).abs() < 0.03), "dust outside Lightroom's areas is left");
    // one undo step
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.develop_of(id).unwrap().spots.is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    // nothing twice
    assert_eq!(s.execute("library.lightroomDust", &json!({"records": rec_s})).unwrap()["spots"], json!(0));
    let _ = std::fs::remove_dir_all(&dir);
}
