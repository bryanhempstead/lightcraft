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
    assert_eq!(p["crop"]["geometry"]["angle"], json!(-1.5));
    // a base look maps to our profile with its amount
    assert_eq!(p["profile"]["id"], json!("adobe:Adobe Monochrome"));
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
    assert_eq!(d.profile.id, "adobe:Adobe Monochrome");
}

#[test]
fn creative_looks_are_reported_and_default_settings_are_not_a_look() {
    let t = r#"s = { Exposure2012 = 0, Look = { Amount = 1, Name = "Summer Fields", Parameters = { RGBTable = "D550" } }, CameraProfile = "Adobe Standard" }"#;
    let m = map_develop(t, true, 1.5).unwrap();
    assert_eq!(m.creative_look, Some(("Summer Fields".to_string(), 1.0)));
    assert!(m.unmapped.contains(&"Look".to_string()));
    // (the base profile underneath the creative look: Adobe's own)
    assert_eq!(m.partial["profile"]["id"], json!("adobe:Adobe Standard"));
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
    assert_eq!(set.parent, None, "the catalog's own tree, no wrapper folder");

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

    // the app's way: prepare off the session, import, then apply without importing
    let mut rec2 = rec.clone();
    prepare(&mut rec2, None);
    assert_eq!(files_to_import(&rec2, None).len(), 2);
    assert!(rec2.images.iter().find(|i| i.id == 1).and_then(|i| i.prepared.as_ref()).is_some_and(|p| p.exists && p.develop.is_some()));
    assert!(rec2.images.iter().find(|i| i.id == 4).and_then(|i| i.prepared.as_ref()).is_some_and(|p| !p.exists));
    let mut s2 = Session::new().with_fs();
    s2.open_library(dir.join("lib2"), false).unwrap();
    s2.execute("library.import", &json!({"paths": files_to_import(&rec2, None)})).unwrap();
    let rec2_path = dir.join("records2.json");
    std::fs::write(&rec2_path, serde_json::to_vec(&rec2).unwrap()).unwrap();
    let r2 = s2.execute("library.migrateLightroom", &json!({"records": rec2_path.to_string_lossy(), "import": false, "presets": false})).unwrap();
    assert_eq!(r2["matched"], json!(3), "{r2}");
    assert_eq!(r2["import"], Value::Null);
    assert!(s2.catalog.photos().any(|p| p.develop.light.exposure == 1.25 && p.rating == 4));

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

/// A catalog shaped like a real one (Bryan's): the Quick Collection (a system-owned plain
/// collection), a "Smart Collections" set, a top-level "From Lightroom" set holding a collection
/// whose members are all unchanged sync-duplicate virtual copies, an empty collection, a
/// top-level smart collection, and print / slideshow drafts. Every real collection comes over
/// with its members and nesting; a dry run says so; a collections-only run on a library made
/// by an earlier migration tidies what that one got wrong and changes no photo.
#[test]
fn collections_come_over_with_members_and_nesting() {
    let dir = std::env::temp_dir().join(format!("lc-lrcols-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    write_png(&photos.join("a.png"));
    let img = lightcraft_raster::Rgba8::filled(20, 16, [9, 9, 9, 255]);
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(photos.join("b.png"), bytes).unwrap();
    let root = format!("{}/", dir.display());
    let im = |id: i64, base: &str, extra: Value| {
        let mut v =
            json!({"id": id, "root": root, "folder": "shoot/", "base": base, "ext": "png", "develop": "s = { Exposure2012 = 0 }", "rating": 4});
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    };
    let sync = "c3b21bc52b6c4c9392c79204e4f0b5bb:sync duplicate";
    let images = vec![
        im(1, "a", json!({})),
        im(2, "b", json!({})),
        im(3, "a", json!({"master": 1, "copyName": "Copy 1", "copyReason": sync})),
        im(4, "b", json!({"master": 2, "copyName": "Copy 1", "copyReason": sync})),
    ];
    let col = |id: i64, name: &str, parent: Value, kind: &str, system: f64| json!({"id": id, "name": name, "parent": parent, "kind": format!("com.adobe.ag.{kind}"), "system": system});
    let cols = vec![
        col(5, "quick collection", Value::Null, "library.collection", 1.0),
        col(12, "Smart Collections", Value::Null, "library.group", 0.0),
        col(18, "Five Stars", json!(12), "library.smart_collection", 0.0),
        col(48589, "From Lightroom", Value::Null, "library.group", 0.0),
        col(100, "Trip Copy 2", json!(48589), "library.collection", 0.0),
        col(101, "Photostoedit", json!(48589), "library.collection", 0.0),
        col(3785731, "Unsaved Slideshow", Value::Null, "slideshow.unsaved", 1.0),
        col(3797593, "Unsaved Print", Value::Null, "print.unsaved", 1.0),
        col(6933125, "Duplicates", Value::Null, "library.smart_collection", 0.0),
    ];
    let ci = vec![json!({"collection": 5, "image": 2}), json!({"collection": 100, "image": 3}), json!({"collection": 100, "image": 4})];
    let smart = vec![
        json!({"collection": 18, "content": "s = { { criteria = \"rating\", operation = \">=\", value = 5 }, combine = \"intersect\" }"}),
        json!({"collection": 6933125, "content": "s = { { criteria = \"keywords\", operation = \"words\", value = \"Duplicate\" }, combine = \"intersect\" }"}),
    ];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &cols, &ci, &smart);
    assert_eq!(rec.collections.iter().find(|c| c.id == 5).map(|c| c.kind.as_str()), Some("quick"));
    assert_eq!(rec.collections.len(), 7, "print / slideshow drafts are left out");
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();

    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let dry = s.execute("library.migrateLightroom", &json!({"records": rec_s, "dryRun": true, "presets": false})).unwrap();
    assert_eq!(dry["albums"], json!({"albums": 2, "smart": 2, "sets": 2, "quick": 1, "skipped": []}), "a dry run counts the collections");

    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(r["albums"]["albums"], json!(2), "{r}");
    let cat = &s.catalog;
    let id_of = |n: &str| cat.photos().find(|p| p.file_name.starts_with(n) && p.copy_of.is_none()).unwrap().id;
    let (a, b) = (id_of("a"), id_of("b"));
    let named = |n: &str| cat.albums().find(|al| al.name == n).cloned().unwrap_or_else(|| panic!("no album {n}"));
    // exactly the catalog's tree: its From Lightroom set at the top, nothing wrapped around it
    let top = named(ALBUM_FOLDER);
    assert!(top.folder && top.parent.is_none());
    assert_eq!(cat.albums().filter(|al| al.name == ALBUM_FOLDER).count(), 1, "one From Lightroom: the catalog's own set");
    let trip = named("Trip Copy 2");
    assert_eq!((trip.parent, trip.photos.clone()), (Some(top.id), vec![a, b]), "sync-duplicate copies stand for their masters");
    assert_eq!(named("Photostoedit").parent, Some(top.id));
    let sets = named("Smart Collections");
    assert!(sets.folder && sets.parent.is_none());
    let five = named("Five Stars");
    assert!(five.is_smart() && five.parent == Some(sets.id));
    assert!(named("Duplicates").is_smart() && named("Duplicates").parent.is_none());
    let quick = cat.quick_collection().and_then(|q| cat.album(q)).unwrap();
    assert_eq!(quick.photos, vec![b], "Lightroom's Quick Collection is ours");
    assert!(!cat.albums().any(|al| al.name.eq_ignore_ascii_case("quick collection") && !al.quick));
    assert!(!cat.albums().any(|al| al.name.contains("Unsaved")));
    // again: nothing twice
    let n = s.catalog.albums().count();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(s.catalog.albums().count(), n);

    // a library an earlier migration got wrong (his): everything wrapped in a From Lightroom
    // folder, so From Lightroom ▸ From Lightroom ▸ Trip, From Lightroom ▸ Smart Collections, and
    // a "quick collection" album; collections only, the photos keep what they have
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib2"), false).unwrap();
    let files: Vec<String> = ["a.png", "b.png"].iter().map(|f| photos.join(f).to_string_lossy().to_string()).collect();
    s.execute("library.import", &json!({"paths": files, "mode": "add"})).unwrap();
    let a = s.catalog.photos().find(|p| p.file_name.starts_with('a')).unwrap().id;
    let mk = |s: &mut Session, name: &str, parent: Option<AlbumId>, folder: bool, photos: Vec<PhotoId>| {
        let id = s.catalog.alloc_album_id();
        s.commit("t", Op::AddAlbum { album: Album { parent, folder, photos, ..Album::new(id, name) } }).unwrap();
        id
    };
    let top = mk(&mut s, ALBUM_FOLDER, None, true, vec![]);
    mk(&mut s, "quick collection", Some(top), false, vec![a]);
    let nested = mk(&mut s, ALBUM_FOLDER, Some(top), true, vec![]);
    mk(&mut s, "Trip Copy 2", Some(nested), false, vec![a]);
    let old_sets = mk(&mut s, "Smart Collections", Some(top), true, vec![]);
    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false, "import": false, "collectionsOnly": true})).unwrap();
    assert_eq!(r["rated"], json!(0), "collections only: {r}");
    let cat = &s.catalog;
    assert!(cat.photos().all(|p| p.rating == 0 && p.copy_of.is_none()), "no photo changed, no copy made");
    let b = cat.photos().find(|p| p.file_name.starts_with('b')).unwrap().id;
    let quick = cat.quick_collection().and_then(|q| cat.album(q)).unwrap();
    assert_eq!(quick.photos, vec![a, b], "the old album's photos joined the Quick Collection");
    assert!(cat.album(top).is_none(), "the wrapper is gone");
    assert_eq!(cat.album(nested).map(|n| n.parent), Some(None), "the catalog's own set is at the top");
    assert_eq!(cat.album(old_sets).map(|n| n.parent), Some(None));
    assert_eq!(cat.albums().filter(|al| al.name == ALBUM_FOLDER).count(), 1);
    assert_eq!(cat.albums().filter(|al| al.name == "Smart Collections").count(), 1);
    let trips: Vec<&Album> = cat.albums().filter(|al| al.name == "Trip Copy 2").collect();
    assert_eq!(trips.len(), 1);
    assert_eq!((trips[0].parent, trips[0].photos.clone()), (Some(nested), vec![a, b]));
    assert!(!cat.albums().any(|al| al.name.eq_ignore_ascii_case("quick collection") && !al.quick));
    assert!(cat.albums().any(|al| al.name == "Five Stars" && al.parent == Some(old_sets)));
    // undo the whole run: the wrapper is back
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.album(top).is_some());

    // the one-shot command, on its own (his library as the first migration left it)
    let r = s.execute("library.flattenLightroomCollections", &json!({})).unwrap();
    assert_eq!(r["flattened"], json!(true), "{r}");
    assert_eq!(r["quick"], json!(1));
    assert!(s.catalog.album(top).is_none() && s.catalog.album(nested).is_some_and(|n| n.parent.is_none()));
    let again = s.execute("library.flattenLightroomCollections", &json!({})).unwrap();
    assert_eq!(again["flattened"], json!(false), "the catalog's own From Lightroom set is left alone");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.catalog.album(top).is_some(), "one undo step");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `onlyNew`: a second run imports what the first couldn't read and applies Lightroom's data to
/// those photos only; photos already in the library keep the changes made since.
#[test]
fn only_new_leaves_migrated_photos_alone() {
    let dir = std::env::temp_dir().join(format!("lc-lrmig-new-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    write_png(&photos.join("a.png"));
    // b can't be read on the first run
    std::fs::write(photos.join("b.png"), "not yet").unwrap();
    let root = format!("{}/", dir.display());
    let im = |id: i64, base: &str| json!({"id": id, "root": root, "folder": "shoot/", "base": base, "ext": "png", "develop": "s = { Exposure2012 = 1.25 }", "edits": 1, "rating": 3});
    let cols = vec![json!({"id": 8, "name": "Trip", "kind": "com.adobe.ag.library.collection"})];
    let ci = vec![json!({"collection": 8, "image": 1}), json!({"collection": 8, "image": 2})];
    let rec = records_from_rows("t.lrcat", &[im(1, "a"), im(2, "b")], &[], &[], &cols, &ci, &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();

    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(s.catalog.len(), 1);
    let a = s.catalog.photos().next().unwrap().id;
    // since the migration: a re-rated and re-edited in LightCraft
    s.commit("t", lightcraft_catalog::Op::SetRating { id: a, rating: 5 }).unwrap();
    s.selection = crate::Selection::single(a);
    s.execute("develop.set", &json!({"control": "light.exposure", "value": -0.5})).unwrap();

    let img = lightcraft_raster::Rgba8::filled(20, 16, [9, 9, 9, 255]);
    std::fs::write(photos.join("b.png"), lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &Default::default()).unwrap())
        .unwrap();
    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false, "onlyNew": true})).unwrap();
    assert_eq!(r["matched"], json!(1), "{r}");
    assert_eq!(s.catalog.len(), 2);
    let pa = s.catalog.photo(a).unwrap();
    assert_eq!(pa.rating, 5, "kept");
    assert_eq!(pa.develop.light.exposure, -0.5, "kept");
    let b = s.catalog.photos().find(|p| p.id != a).unwrap();
    assert_eq!(b.rating, 3);
    assert_eq!(b.develop.light.exposure, 1.25);
    let trip = s.catalog.albums().find(|al| al.name == "Trip").unwrap();
    assert_eq!(trip.photos.len(), 2, "both in the collection, nothing duplicated");
    assert_eq!(s.catalog.albums().filter(|al| al.name == "Trip").count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Creative looks (camera-raw XMP "Look" profiles with a colour table): imported by the
/// migration and matched by name, or matched later by `library.rematchProfiles`.
#[test]
fn creative_profiles_match_on_migration_and_later() {
    use crate::crs_table::tests::{encode_text, table_bytes, xmp};
    let dir = std::env::temp_dir().join(format!("lc-lrlook-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    let profiles = dir.join("profiles");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::create_dir_all(&profiles).unwrap();
    for (name, v) in [("a.png", 120u8), ("b.png", 90)] {
        let img = lightcraft_raster::Rgba8::filled(20, 16, [v, v, v, 255]);
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(photos.join(name), bytes).unwrap();
    }
    // a warm look, and a preset that only names it (no table: nothing to import)
    let text = encode_text(&table_bytes(5, |c| [(c[0] * 1.1 + 0.05).min(1.0), c[1], c[2] * 0.8], (0, 1, 0.0, 1.0)));
    std::fs::write(profiles.join("Fields.xmp"), xmp("Fields Test", "AA11", &text, "0.5")).unwrap();
    std::fs::write(profiles.join("broken.xmp"), xmp("Broken", "BB22", "#####", "1")).unwrap();
    let root = format!("{}/", dir.display());
    let look =
        |name: &str| format!("s = {{ Exposure2012 = 0.5,\nLook = {{ Amount = 0.8,\nName = \"{name}\",\nParameters = {{ RGBTable = \"AA11\" }} }} }}");
    let images = vec![
        json!({"id": 1, "root": root, "folder": "shoot/", "base": "a", "ext": "png", "develop": look("Fields Test"), "edits": 2}),
        json!({"id": 2, "root": root, "folder": "shoot/", "base": "b", "ext": "png", "develop": look("Other Look"), "edits": 2}),
    ];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    let pdir = profiles.to_string_lossy().to_string();

    // migration imports the profile and matches it
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let dry = s.execute("library.migrateLightroom", &json!({"records": rec_s, "dryRun": true, "presets": false, "profileDirs": [pdir]})).unwrap();
    assert_eq!(dry["develop"]["creativeLooksMatched"], json!(1), "{dry}");
    assert!(s.lut_profiles.is_empty(), "a dry run imports nothing");
    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false, "profileDirs": [pdir]})).unwrap();
    assert_eq!(r["develop"]["creativeLooksMatched"], json!(1), "{r}");
    assert_eq!(r["profiles"]["failed"].as_array().map(Vec::len), Some(1), "the broken table is reported: {r}");
    let a = s.catalog.photos().find(|p| p.file_name.starts_with('a')).cloned().unwrap();
    assert!(a.develop.profile.id.starts_with("lut:"), "{:?}", a.develop.profile);
    assert_eq!(a.develop.profile.amount, 80.0);
    assert_eq!(s.profile_info(&a.develop.profile.id), Some(("Fields Test", "Grp")));
    // the look renders: warmer than the same photo at Amount 0
    let warm = s.render_now(a.id, 24, 16).unwrap().image;
    s.selection = crate::Selection::single(a.id);
    s.execute("develop.profile", &json!({"id": a.develop.profile.id, "amount": 0})).unwrap();
    let plain = s.render_now(a.id, 24, 16).unwrap().image;
    let mean = |img: &lightcraft_raster::Rgba8, k: usize| img.data.iter().map(|p| p[k] as f64).sum::<f64>() / img.data.len() as f64;
    assert!(mean(&warm, 0) > mean(&plain, 0) && mean(&warm, 2) < mean(&plain, 2), "warmer");

    // a library migrated without the profile: matched afterwards, from the migration's record
    let mut s2 = Session::new().with_fs();
    s2.open_library(dir.join("lib2"), false).unwrap();
    let r = s2.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    assert_eq!(r["develop"]["creativeLooksMatched"], json!(0), "{r}");
    let r = s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir], "dryRun": true})).unwrap();
    assert_eq!((r["matched"].clone(), r["unmatched"]["Other Look"].clone()), (json!(1), json!(1)), "{r}");
    assert!(s2.catalog.photos().all(|p| !p.develop.profile.id.starts_with("lut:")), "dry run");
    let r = s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir]})).unwrap();
    assert_eq!(r["byLook"]["Fields Test"], json!(1), "{r}");
    let a2 = s2.catalog.photos().find(|p| p.file_name.starts_with('a')).cloned().unwrap();
    assert!(a2.develop.profile.id.starts_with("lut:") && a2.develop.profile.amount == 80.0);
    assert_eq!(a2.develop.light.exposure, 0.5, "the rest of the settings stay");
    s2.undo_step().unwrap();
    assert!(!s2.catalog.photo(a2.id).unwrap().develop.profile.id.starts_with("lut:"), "one undo step");
    // …or from the catalog (records) when the migration didn't record looks
    let resume = dir.join("lib2").join(RESUME_FILE);
    let mut doc: Value = serde_json::from_slice(&std::fs::read(&resume).unwrap()).unwrap();
    doc.as_object_mut().unwrap().remove("looks");
    std::fs::write(&resume, serde_json::to_vec(&doc).unwrap()).unwrap();
    assert_eq!(s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir]})).unwrap()["matched"], json!(0));
    let r = s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir], "records": rec_s})).unwrap();
    assert_eq!(r["matched"], json!(1), "{r}");
    // a profile chosen here since is kept unless forced
    s2.selection = crate::Selection::single(a2.id);
    s2.execute("develop.profile", &json!({"id": "lc.vivid", "amount": 100})).unwrap();
    let r = s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir], "records": rec_s})).unwrap();
    assert_eq!((r["matched"].clone(), r["keptOwnProfile"].clone()), (json!(0), json!(1)), "{r}");
    let r = s2.execute("library.rematchProfiles", &json!({"profileDirs": [pdir], "records": rec_s, "force": true})).unwrap();
    assert_eq!(r["matched"], json!(1), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Libraries migrated while `crs:CropAngle` was read with the wrong sign are repaired in place;
/// a crop changed here since is left alone.
#[test]
fn crop_angles_from_older_migrations_are_fixed() {
    let dir = std::env::temp_dir().join(format!("lc-lrcrop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    for (name, v) in [("a.png", 120u8), ("b.png", 60)] {
        let img = lightcraft_raster::Rgba8::filled(24, 16, [v, v, v, 255]);
        let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
        std::fs::write(photos.join(name), bytes).unwrap();
    }
    let root = format!("{}/", dir.display());
    let dev = "s = { CropAngle = 1.35,\nCropBottom = 0.9,\nCropLeft = 0.1,\nCropRight = 0.9,\nCropTop = 0.1,\nHasCrop = true }";
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
    assert_eq!(s.catalog.photo(a).unwrap().develop.crop.geometry.angle, -1.35);
    // an older migration's mirror image (written as that migration did), and a crop changed here since
    let pa = s.catalog.photo(a).unwrap().clone();
    let mut d = (*pa.develop).clone();
    d.crop.geometry.angle = 1.35;
    s.catalog
        .apply(lightcraft_catalog::Op::SetDevelop { id: a, settings: std::sync::Arc::new(d), label: "old".into(), edited: pa.edited.clone() })
        .unwrap();
    s.selection = crate::Selection::single(b);
    s.execute("develop.merge", &json!({"settings": {"crop": {"geometry": {"rect": {"x0": 0.0, "y0": 0.0, "x1": 1.0, "y1": 1.0}, "angle": 3.0}}}}))
        .unwrap();
    let r = s.execute("library.repairLightroomMigration", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!((r["cropsFixed"].clone(), r["keptChangedHere"].clone()), (json!(1), json!(1)), "{r}");
    assert_eq!(s.catalog.photo(a).unwrap().develop.crop.geometry.angle, 1.35, "dry run");
    s.execute("library.repairLightroomMigration", &json!({"records": rec_s})).unwrap();
    assert_eq!(s.catalog.photo(a).unwrap().develop.crop.geometry.angle, -1.35);
    assert_eq!(s.catalog.photo(b).unwrap().develop.crop.geometry.angle, 3.0);
    assert_eq!(s.execute("library.repairLightroomMigration", &json!({"records": rec_s})).unwrap()["cropsFixed"], json!(0), "nothing twice");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A photo Lightroom shows unedited that picked up settings from an XMP sidecar on import gets
/// Lightroom's settings (the catalog wins).
#[test]
fn sidecar_settings_lose_to_the_catalog() {
    let dir = std::env::temp_dir().join(format!("lc-lrside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let photos = dir.join("shoot");
    std::fs::create_dir_all(&photos).unwrap();
    let img = lightcraft_raster::Rgba8::filled(24, 16, [90, 90, 90, 255]);
    let bytes = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &lightcraft_codecs::EncodeMeta::default()).unwrap();
    std::fs::write(photos.join("a.png"), bytes).unwrap();
    std::fs::write(
        photos.join("a.xmp"),
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" crs:Exposure2012="+1.50" crs:Contrast2012="+40"/></rdf:RDF></x:xmpmeta>"#,
    )
    .unwrap();
    let root = format!("{}/", dir.display());
    let images =
        vec![json!({"id": 1, "root": root, "folder": "shoot/", "base": "a", "ext": "png", "develop": "s = { Exposure2012 = 0 }", "edits": 0})];
    let rec = records_from_rows("t.lrcat", &images, &[], &[], &[], &[], &[]);
    let rec_path = dir.join("records.json");
    std::fs::write(&rec_path, serde_json::to_vec(&rec).unwrap()).unwrap();
    let rec_s = rec_path.to_string_lossy().to_string();
    // a fresh migration: the catalog wins over the sidecar
    let mut s = Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r = s.execute("library.migrateLightroom", &json!({"records": rec_s, "presets": false})).unwrap();
    let p = s.catalog.photos().next().unwrap().clone();
    assert_eq!(p.develop.light.exposure, 0.0, "{r}");
    assert_eq!(r["develop"]["sidecarSettingsReplaced"], json!(1), "{r}");
    // a library imported (with the sidecar) and migrated without that rule: repaired
    let mut s2 = Session::new().with_fs();
    s2.open_library(dir.join("lib2"), false).unwrap();
    s2.execute("library.import", &json!({"paths": [photos.join("a.png").to_string_lossy()]})).unwrap();
    let id = s2.catalog.photos().next().unwrap().id;
    assert_eq!(s2.catalog.photo(id).unwrap().develop.light.exposure, 1.5, "the sidecar was read");
    let r = s2.execute("library.repairLightroomMigration", &json!({"records": rec_s, "dryRun": true})).unwrap();
    assert_eq!(r["uneditedReset"], json!(1), "{r}");
    s2.execute("library.repairLightroomMigration", &json!({"records": rec_s})).unwrap();
    assert_eq!(s2.catalog.photo(id).unwrap().develop.light.exposure, 0.0);
    let _ = std::fs::remove_dir_all(&dir);
}
