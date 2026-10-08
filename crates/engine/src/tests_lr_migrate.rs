//! Lightroom Classic migration: Lua develop text, records, smart rules, presets, resume point.

use super::*;
use crate::Session;
use serde_json::json;

const DEVELOP: &str = r#"s = { AutoLateralCA = 1,
Blacks2012 = -12,
CameraProfile = "Adobe Standard",
ColorNoiseReduction = 25,
Contrast2012 = 14,
CropAngle = 1.5,
CropBottom = 0.9,
CropLeft = 0.1,
CropRight = 0.95,
CropTop = 0.05,
Exposure2012 = 0.65,
Highlights2012 = -40,
HDREditMode = 0,
SDRBlend = 0,
GrainSeed = 518792649,
LensProfileName = "Adobe (Some Lens)",
Look = { Amount = 0.8,
Group = { ["x-default"] = "Profiles" },
Name = "Adobe Monochrome",
Parameters = { ConvertToGrayscale = true } },
ToneCurvePV2012 = { 0, 0, 64, 50, 255, 255 },
WhiteBalance = "Custom",
Temperature = 5200,
Tint = 8,
RetouchAreas = { { Masks = { { Dabs = { "d 0.5 0.4", "r 0.03", "d 0.52 0.41" }, Radius = 0.02, What = "Mask/Paint" } },
  OffsetY = 0.6, Opacity = 0.9, SourceX = 0.3, SpotType = "heal" } },
RetouchInfo = { { centerX = 0.5, centerY = 0.4, radius = 0.02, spotType = "heal" } },
PointColors = { { SrcHue = 3.9, SrcSat = 0.6, SrcLum = 0.2, HueShift = 0.1, SatScale = -1, LumScale = 0, RangeAmount = 0.5,
  HueRange = { LowerNone = 0, LowerFull = 0.33, UpperFull = 0.66, UpperNone = 1 } } },
UprightVersion = 151388160 }"#;

#[test]
fn develop_text_maps_like_a_preset() {
    let m = map_develop(DEVELOP, true, 1.5).unwrap();
    let p = &m.partial;
    assert_eq!(p["light"]["exposure"], json!(0.65));
    assert_eq!(p["light"]["contrast"], json!(14.0));
    assert_eq!(p["wb"]["mode"], json!("custom"));
    assert_eq!(p["wb"]["temp"], json!(5200.0));
    // the catalog has crop edges without HasCrop
    assert_eq!(p["crop"]["geometry"]["rect"]["x0"], json!(0.1));
    assert_eq!(p["crop"]["geometry"]["angle"], json!(1.5));
    // a base look maps to our profile with its amount
    assert_eq!(p["profile"]["id"], json!("lc.mono"));
    assert_eq!(p["profile"]["amount"], json!(80.0));
    // one spot (the legacy RetouchInfo copy is not doubled), its source relative to the path
    let spots = p["spots"].as_array().unwrap();
    assert_eq!(spots.len(), 1);
    assert_eq!(spots[0]["mode"], json!("heal"));
    assert!((spots[0]["source_offset"]["x"].as_f64().unwrap() + 0.2).abs() < 1e-9);
    assert_eq!(spots[0]["points"].as_array().unwrap().len(), 2);
    assert_eq!(p["point_colors"].as_array().unwrap().len(), 1);
    assert_eq!(p["point_colors"][0]["sat_shift"], json!(-100.0));
    // bookkeeping fields are not reported
    assert!(m.unmapped.is_empty(), "{:?}", m.unmapped);
    assert!(m.custom);
    assert!(m.creative_look.is_none());
    // and the whole thing is valid develop settings
    let d = lightcraft_develop::apply_partial(&lightcraft_develop::DevelopSettings::default(), p, 1.0);
    assert_eq!(d.spots.len(), 1);
    assert_eq!(d.point_colors.len(), 1);
    assert_eq!(d.profile.id, "lc.mono");
}

#[test]
fn creative_looks_are_reported_and_default_settings_are_not_a_look() {
    let t = r#"s = { Exposure2012 = 0, Look = { Amount = 1, Name = "Summer Fields", Parameters = { RGBTable = "D550" } }, CameraProfile = "Adobe Standard" }"#;
    let m = map_develop(t, true, 1.5).unwrap();
    assert_eq!(m.creative_look, Some(("Summer Fields".to_string(), 1.0)));
    assert!(m.unmapped.contains(&"Look".to_string()));
    assert_eq!(m.partial["profile"]["id"], json!("lc.color"));
    // Lightroom's defaults only: not an edit
    let plain = r#"s = { Exposure2012 = 0, Contrast2012 = 0, Sharpness = 40, WhiteBalance = "As Shot", ToneCurvePV2012 = { 0, 0, 255, 255 },
        Look = { Name = "Adobe Color", Amount = 1 } }"#;
    assert!(!map_develop(plain, true, 1.5).unwrap().custom);
}

#[test]
fn malformed_develop_text_is_an_error_not_a_crash() {
    for t in [
        "",
        "s = ",
        "s = { Exposure2012 = }",
        "s = { Exposure2012 = 1e999, Contrast2012 = -1e308, Temperature = 99999999999 }",
        "s = { RetouchAreas = { { Masks = { { Dabs = { \"d x y\", \"r -1\", \"d 1e400 nan\" } } } } } }",
        "s = { PointColors = { { SrcHue = \"x\" } }, RetouchInfo = { 7 } }",
        "s = \"text\"",
        "\u{feff}s = { [1] = 2, [\"a\"] = { } }",
    ] {
        let _ = map_develop(t, false, f64::NAN);
    }
    // hostile nesting is bounded
    let deep = format!("s = {}{}", "{".repeat(100_000), "}".repeat(100_000));
    assert!(map_develop(&deep, true, 1.0).is_err());
    let negs = format!("s = {{ A = {}1 }}", "-".repeat(100_000));
    assert!(map_develop(&negs, true, 1.0).is_err());
}

#[test]
fn rotation_moves_crop_and_spots_into_the_shown_frame() {
    let mut p = json!({
        "crop": {"geometry": {"rect": {"x0": 0.1, "y0": 0.2, "x1": 0.5, "y1": 0.9}, "angle": 1.0}},
        "spots": [{"points": [{"x": 0.2, "y": 0.3}], "source_offset": {"x": 0.1, "y": 0.0}}],
        "masks": [{"components": [{"shape": {"kind": "radial", "center": {"x": 0.25, "y": 0.5}, "rx": 0.1, "ry": 0.3}}]}],
    });
    reorient_partial(&mut p, lightcraft_geom::Orientation::Rotate90);
    // a quarter turn clockwise: (x, y) → (1 − y, x)
    let r = &p["crop"]["geometry"]["rect"];
    assert!((r["x0"].as_f64().unwrap() - 0.1).abs() < 1e-9 && (r["x1"].as_f64().unwrap() - 0.8).abs() < 1e-9);
    assert!((r["y0"].as_f64().unwrap() - 0.1).abs() < 1e-9 && (r["y1"].as_f64().unwrap() - 0.5).abs() < 1e-9);
    assert!((p["spots"][0]["points"][0]["x"].as_f64().unwrap() - 0.7).abs() < 1e-9);
    assert!((p["spots"][0]["source_offset"]["y"].as_f64().unwrap() - 0.1).abs() < 1e-9);
    let sh = &p["masks"][0]["components"][0]["shape"];
    assert_eq!(sh["rx"], json!(0.3));
    assert_eq!(sh["ry"], json!(0.1));
}

#[test]
fn smart_collection_rules_map_or_say_why() {
    let five = "s = { { criteria = \"rating\", operation = \">=\", value = 5 }, combine = \"intersect\" }";
    let r = smart_rules(five).unwrap();
    assert_eq!(r["ruleSet"]["match"], json!("all"));
    assert_eq!(r["ruleSet"]["rules"][0], json!({"field": "rating", "op": "gte", "value": 5.0}));
    let month = "s = { { criteria = \"captureTime\", operation = \"inLast\", value = 1, value_units = \"months\" }, combine = \"union\" }";
    assert_eq!(smart_rules(month).unwrap()["ruleSet"]["rules"][0]["value"], json!({"n": 1.0, "unit": "months"}));
    let kw = "s = { { criteria = \"keywords\", operation = \"empty\" }, { { criteria = \"labelColor\", operation = \"==\", value = 1 }, combine = \"exclude\" } }";
    let r = smart_rules(kw).unwrap();
    assert_eq!(r["ruleSet"]["rules"][0]["op"], json!("isEmpty"));
    assert_eq!(r["ruleSet"]["rules"][1]["group"]["match"], json!("none"));
    assert!(smart_rules("s = { { criteria = \"faces\", operation = \"==\", value = 1 } }").is_err());
    assert!(smart_rules("nonsense").is_err());
    // every mapped rule set is accepted by album.createSmart
    let mut s = Session::new();
    for t in [five, month, kw] {
        s.execute("album.createSmart", &json!({"name": "x", "rules": smart_rules(t).unwrap()})).unwrap();
    }
}

#[test]
fn keyword_sets_read_with_localized_titles() {
    let t = "s = { title = \"$$$/AgLibrary/KeywordSetFactoryDefaults/Wedding=Wedding Photography\", type = \"KeywordSet\", value = { shortcut1title = \"love\", shortcut3title = \"bride\" } }";
    assert_eq!(keyword_set(t), Some(("Wedding Photography".into(), vec!["love".into(), "bride".into()])));
    assert_eq!(keyword_set("s = { type = \"Develop\" }"), None);
    assert_eq!(keyword_set("{{{"), None);
}

#[test]
fn records_join_keywords_collections_and_times() {
    let images = vec![
        json!({"id": 1, "root": "/r/", "folder": "a/", "base": "x", "ext": "CR3", "rating": 5.0, "pick": 1.0, "label": "Red", "orientation": "BC",
               "lastEdit": 689608872.8, "touchTime": 697241098.6, "edits": 3, "develop": "s = {}", "caption": "cap"}),
        json!({"id": 2, "master": 1, "copyName": "Copy 1", "copyReason": "abc:sync duplicate", "root": "/r/", "folder": "a/", "base": "x", "ext": "CR3"}),
        json!({"id": "bad"}),
    ];
    let keywords = vec![
        json!({"id": 10, "parent": null, "name": null}),
        json!({"id": 11, "parent": 10, "name": "People"}),
        json!({"id": 12, "parent": 11, "name": "Ann|B"}),
        json!({"id": 13, "parent": 13, "name": "Loop"}),
    ];
    let ik = vec![json!({"image": 1, "tag": 12}), json!({"image": 1, "tag": 13}), json!({"image": 1, "tag": 999})];
    let cols = vec![
        json!({"id": 5, "name": "Best", "parent": null, "kind": "com.adobe.ag.library.collection"}),
        json!({"id": 6, "name": "Draft", "kind": "com.adobe.ag.print.unsaved"}),
    ];
    let ci = vec![json!({"collection": 5, "image": 1})];
    let r = records_from_rows("cat.lrcat", &images, &keywords, &ik, &cols, &ci, &[]);
    assert_eq!(r.images.len(), 2);
    let a = &r.images[0];
    assert_eq!(a.path, "/r/a/x.CR3");
    assert_eq!((a.rating, a.pick), (5, 1));
    assert_eq!(a.keywords, vec!["People|Ann/B".to_string(), "Loop".to_string()]);
    assert_eq!(a.last_edit.as_deref(), Some("2022-11-08T14:01:12"));
    assert!(a.touched.is_some());
    assert!(r.images[1].sync_duplicate);
    assert_eq!(r.collections.len(), 1);
    assert_eq!(r.collections[0].images, vec![1]);
    assert_eq!(cocoa_time(f64::NAN), None);
    assert_eq!(cocoa_time(1e300), None);
}

fn write_png(path: &std::path::Path) {
    let img = lightcraft_raster::Rgba8::new(24, 16);
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A tiny "catalog" over real files: import, metadata, develop, albums, virtual copies, resume point.
#[test]
fn migrate_end_to_end_and_resume_point() {
    let dir = std::env::temp_dir().join(format!("lc-lrmig-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    write_png(&photos.join("a.png"));
    // different bytes, so it isn't a duplicate of a
    let img = lightcraft_raster::Rgba8::filled(20, 16, [9, 9, 9, 255]);
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(photos.join("b.png"), bytes).unwrap();
    let root = format!("{}/", dir.display());
    let im = |id: i64, base: &str, extra: Value| {
        let mut v = json!({"id": id, "root": root, "folder": "shoot/", "base": base, "ext": "png", "develop": "s = { Exposure2012 = 1.25 }", "edits": 2, "lastEdit": 700000000.0});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    };
    let images = vec![
        im(1, "a", json!({"rating": 4, "pick": -1, "touchTime": 710000000.0})),
        im(2, "b", json!({"edits": 0, "develop": "s = { Exposure2012 = 0 }", "touchTime": 720000000.0})),
        im(3, "a", json!({"master": 1, "copyName": "Warm", "develop": "s = { Exposure2012 = -1 }"})),
        im(4, "gone", json!({})),
    ];
    let cols = vec![
        json!({"id": 7, "name": "Set", "kind": "com.adobe.ag.library.group"}),
        json!({"id": 8, "name": "Picks", "parent": 7, "kind": "com.adobe.ag.library.collection"}),
    ];
    let ci = vec![json!({"collection": 8, "image": 1}), json!({"collection": 8, "image": 3})];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &cols, &ci, &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();

    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let dry = s.execute("library.migrateLightroom", &json!({"records": rec_s, "dryRun": true, "presets": false})).unwrap();
    assert_eq!(dry["found"], json!(2));
    assert_eq!(dry["missing"], json!(1));
    assert_eq!(s.catalog.len(), 0, "a dry run changes nothing");

    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(r["matched"], json!(3), "{r}");
    assert_eq!(r["virtualCopies"]["made"], json!(1));
    let by_name =
        |s: &Session, n: &str, copy: bool| s.catalog.photos().find(|p| p.file_name.starts_with(n) && p.copy_of.is_some() == copy).cloned().unwrap();
    let a = by_name(&s, "a", false);
    assert_eq!((a.rating, a.flag), (4, lightcraft_catalog::Flag::Reject));
    assert_eq!(a.develop.light.exposure, 1.25);
    assert_eq!(a.edited.as_deref(), Some("2023-03-08T20:26:40"));
    let warm = by_name(&s, "a", true);
    assert_eq!(warm.copy_name.as_deref(), Some("Warm"));
    assert_eq!(warm.develop.light.exposure, -1.0);
    let b = by_name(&s, "b", false);
    assert!(!b.is_edited(), "never edited in Lightroom: keeps our defaults");
    let picks = s.catalog.albums().find(|al| al.name == "Picks").unwrap().clone();
    assert_eq!(picks.photos.len(), 2);
    let set = s.catalog.album(picks.parent.unwrap()).unwrap();
    assert!(set.folder && set.name == "Set");
    assert_eq!(s.catalog.album(set.parent.unwrap()).unwrap().name, ALBUM_FOLDER);

    // resume: b was touched last in Lightroom
    let folder = photos.to_string_lossy().to_string();
    let rp = s.execute("library.resumePoint", &json!({"folder": folder})).unwrap();
    assert_eq!(rp["photoId"], json!(b.id.0));
    assert_eq!(rp["source"], json!("lightroomTouch"));
    let rp = s.execute("library.resumePoint", &json!({"album": picks.id.0})).unwrap();
    assert_eq!(rp["source"], json!("lightroomTouch"));
    assert_eq!(rp["photoId"], json!(a.id.0));
    // an edit here is newer than anything in Lightroom
    s.selection = crate::Selection::single(a.id);
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.5})).unwrap();
    let rp = s.execute("library.resumePoint", &json!({"folder": folder, "subfolders": false})).unwrap();
    assert_eq!(rp["photoId"], json!(a.id.0));
    assert_eq!(rp["source"], json!("edit"));
    assert_eq!(s.execute("library.resumePoint", &json!({"folder": "/nowhere"})).unwrap(), Value::Null);
    assert!(s.execute("library.resumePoint", &json!({"album": 99999})).is_err());

    // running it again adds nothing new
    let n = s.catalog.len();
    let albums = s.catalog.albums().count();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(s.catalog.len(), n);
    assert_eq!(s.catalog.albums().count(), albums);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_records_and_catalogs_are_errors() {
    let mut s = Session::new();
    assert!(s.execute("library.migrateLightroom", &json!({"records": "/no/such/file.json"})).is_err());
    assert!(s.execute("library.migrateLightroom", &json!({"catalog": "/no/such.lrcat"})).is_err());
    let f = std::env::temp_dir().join(format!("lc-lrmig-bad-{}.json", std::process::id()));
    std::fs::write(&f, b"{\"images\": 7}").unwrap();
    assert!(s.execute("library.migrateLightroom", &json!({"records": f.to_string_lossy()})).is_err());
    let _ = std::fs::remove_file(&f);
}

#[test]
fn presets_import_with_dedupe_and_keyword_sets() {
    let dir = std::env::temp_dir().join(format!("lc-lrmig-presets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (a, b) = (dir.join("Settings").join("My Looks"), dir.join("ImportedSettings"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let xmp = |name: &str, uuid: &str| {
        format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:UUID="{uuid}" crs:Exposure2012="+0.5"><crs:Name><rdf:Alt><rdf:li xml:lang="x-default">{name}</rdf:li></rdf:Alt></crs:Name></rdf:Description></rdf:RDF></x:xmpmeta>"#
        )
    };
    std::fs::write(a.join("one.xmp"), xmp("One", "AAA")).unwrap();
    std::fs::write(b.join("one copy.xmp"), xmp("One", "AAA")).unwrap();
    std::fs::write(b.join("two.xmp"), xmp("Two", "BBB")).unwrap();
    std::fs::write(b.join("broken.xmp"), "<x:xmpmeta").unwrap();
    std::fs::write(a.join("Set.lrtemplate"), "s = { title = \"BH\", type = \"KeywordSet\", value = { shortcut1title = \"love\" } }").unwrap();
    let mut s = Session::new();
    let dirs = vec![dir.join("Settings").to_string_lossy().to_string(), b.to_string_lossy().to_string()];
    let r = import_presets(&mut s, &dirs, false).unwrap();
    assert_eq!(r["imported"], json!(2), "{r}");
    assert_eq!(r["skippedDuplicates"], json!(1));
    assert_eq!(r["failed"].as_array().unwrap().len(), 1);
    assert_eq!(r["keywordSets"], json!(["BH"]));
    assert!(s.presets.iter().any(|p| p.name == "One" && p.group == "My Looks"));
    assert!(s.keyword_sets.iter().any(|k| k.name == "BH" && k.keywords == ["love"]));
    let again = import_presets(&mut s, &dirs, false).unwrap();
    assert_eq!(again["imported"], json!(0));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The folder-scoped ("shoot") commands a pipeline drives: status, import with stars, show,
/// export by folder, export as catalog.
#[test]
fn shoot_commands() {
    let dir = std::env::temp_dir().join(format!("lc-shoot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let shoot = dir.join("Shoot A");
    std::fs::create_dir_all(shoot.join("raw")).unwrap();
    for (i, n) in ["raw/1.png", "raw/2.png", "3.png"].iter().enumerate() {
        let img = lightcraft_raster::Rgba8::filled(16 + i, 12, [i as u8 * 40, 7, 7, 255]);
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(shoot.join(n), bytes).unwrap();
    }
    let folder = shoot.to_string_lossy().to_string();
    let f = |n: &str| shoot.join(n).to_string_lossy().to_string();
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();

    let st = s.execute("library.folderStatus", &json!({"folder": folder})).unwrap();
    assert_eq!((st["inLibrary"].clone(), st["onDisk"].clone(), st["notInLibrary"].clone()), (json!(0), json!(3), json!(3)));

    let r = s
        .execute("library.importRated", &json!({"files": [{"path": f("raw/1.png"), "rating": 5, "flag": "pick", "keywords": ["keeper"]}, {"path": f("raw/2.png"), "rating": 3}, "/no/such.png"]}))
        .unwrap();
    assert_eq!(r["rated"], json!(2), "{r}");
    assert_eq!(r["notFound"], json!(["/no/such.png"]));
    let st = s.execute("library.folderStatus", &json!({"folder": format!("{folder}/"), "ids": true})).unwrap();
    assert_eq!(st["inLibrary"], json!(2));
    assert_eq!(st["ratings"]["5"], json!(1));
    assert_eq!(st["ratings"]["3"], json!(1));
    assert_eq!(st["picks"], json!(1));
    assert_eq!(st["notInLibraryFiles"], json!([f("3.png")]));
    let flat = s.execute("library.folderStatus", &json!({"folder": folder, "subfolders": false})).unwrap();
    assert_eq!(flat["inLibrary"], json!(0));
    assert!(s.execute("library.folderStatus", &json!({"folder": " "})).is_err());

    // open the shoot: its photos shown and selected
    s.execute("library.import", &json!({"paths": [f("3.png")]})).unwrap();
    let other = dir.join("other.png");
    let img = lightcraft_raster::Rgba8::filled(9, 9, [200, 1, 1, 255]);
    std::fs::write(
        &other,
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap(),
    )
    .unwrap();
    s.execute("library.import", &json!({"paths": [other.to_string_lossy()]})).unwrap();
    let shown = s.execute("library.showFolder", &json!({"folder": f("raw")})).unwrap();
    assert_eq!(shown["count"], json!(2), "{shown}");
    assert_eq!(s.selection.ids.len(), 2);

    // export the shoot by folder, with a preset
    let ids = crate::cmd::shoot::photos_under(&s, &folder, true);
    let p = s.export_params(&json!({"folder": folder, "dir": "/tmp/x"})).unwrap();
    assert_eq!(p["ids"].as_array().unwrap().len(), ids.len());
    assert!(p.get("folder").is_none());
    assert!(s.export_params(&json!({"folder": "/nowhere"})).is_err());

    // export as catalog: a library with just the shoot (and its albums), originals untouched
    s.selection = crate::Selection::single(ids[0]);
    s.execute("album.create", &json!({"name": "Best", "addSelected": true})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 0.7})).unwrap();
    let dest = shoot.join("Shoot A.lightcraft");
    let r = s.execute("library.exportCatalog", &json!({"folder": folder, "dest": dest.to_string_lossy()})).unwrap();
    assert_eq!(r["photos"], json!(3), "{r}");
    assert_eq!(r["albums"], json!(1));
    assert!(s.execute("library.exportCatalog", &json!({"folder": folder, "dest": dest.to_string_lossy()})).is_err(), "never into a used folder");
    let mut o = Session::new().with_fs();
    o.open_library(&dest, false).unwrap();
    assert_eq!(o.catalog.len(), 3);
    let p0 = o.catalog.photo(ids[0]).unwrap();
    assert_eq!(p0.develop.light.exposure, 0.7);
    assert!(o.catalog.albums().any(|a| a.name == "Best" && a.photos == vec![ids[0]]));
    assert!(shoot.join("3.png").is_file());
    drop(o);
    let _ = std::fs::remove_dir_all(&dir);
}
