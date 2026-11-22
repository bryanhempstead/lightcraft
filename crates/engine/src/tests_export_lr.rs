//! LR-EXP-DIALOG: the Lightroom Classic Export dialog's settings in the engine — Export To,
//! Existing Files ▸ Ask what to do, the metadata choices, Width & Height / units / ppcm, the Simple
//! Copyright Watermark, preset folders / rename / files / `.lrtemplate`, and the preview estimate.

use serde_json::json;

use crate::Session;
use crate::export::*;
use crate::tests_xmp::temp_dir;

#[test]
fn export_to_standard_folders_and_legacy_same_folder() {
    let home = std::path::Path::new("/Users/someone");
    assert_eq!(ExportTo::Desktop.standard_dir_in(home), Some(home.join("Desktop")));
    assert_eq!(ExportTo::Pictures.standard_dir_in(home), Some(home.join("Pictures")));
    assert_eq!(ExportTo::SpecificFolder.standard_dir_in(home), None);
    assert_eq!(ExportTo::SpecificFolder.resolve(" /x "), "/x");
    // older presets said sameFolder: true
    assert_eq!(ExportOptions::from_json(&json!({"sameFolder": true})).export_to, ExportTo::SameFolder);
    let o = ExportOptions::from_params(&json!({"exportTo": "chooseLater"})).unwrap();
    assert_eq!(o.export_to, ExportTo::ChooseLater);
    assert_eq!(ExportOptions::from_json(&o.to_json()).export_to, ExportTo::ChooseLater);
    assert!(ExportOptions::from_params(&json!({"exportTo": "attic"})).is_err());
    assert_eq!(ExportTo::ALL.len(), 7);
}

#[test]
fn metadata_choices_match_lightroom() {
    use lightcraft_catalog::{Photo, PhotoId, Source};
    let mut p = Photo::new(PhotoId(1), Source::Demo { scene: 0 }, "a.jpg", "jpeg", 10, 10, "2026-09-30T00:00:00");
    p.meta.camera = "Canon EOS R6".into();
    p.meta.copyright = "© Me".into();
    p.meta.creator = "Me".into();
    p.meta.title = "T".into();
    let m = |policy| export_metadata(&p, &ExportOptions { metadata: policy, ..Default::default() }).unwrap();
    let only = m(MetadataPolicy::Copyright);
    assert_eq!((only.copyright.as_deref(), only.artist.as_deref()), (Some("© Me"), None), "Copyright Only: no contact");
    let contact = m(MetadataPolicy::CopyrightContact);
    assert_eq!(contact.artist.as_deref(), Some("Me"));
    assert!(contact.title.is_none());
    let raw = m(MetadataPolicy::AllExceptCameraRaw);
    assert!(raw.model.is_some() && raw.title.is_some(), "camera info kept");
    let cam = m(MetadataPolicy::AllExceptCamera);
    assert!(cam.model.is_none() && cam.title.is_some());
    for (k, _) in MetadataPolicy::ALL {
        let v = serde_json::to_value(k).unwrap();
        ExportOptions::from_params(&json!({"metadata": v})).unwrap();
    }
}

#[test]
fn width_and_height_keeps_orientation_and_units_convert() {
    let r = Resize { mode: ResizeMode::WidthHeight, value: 400.0, height: 600.0, ..Default::default() };
    assert_eq!(r.apply(4000.0, 6000.0), (400, 600), "portrait");
    assert_eq!(r.apply(6000.0, 4000.0), (400, 267), "landscape");
    let d = Resize { mode: ResizeMode::Dimensions, ..r };
    assert_eq!(d.apply(6000.0, 4000.0), (600, 400), "Dimensions turns with the photo");
    // one side left at 0: the other alone
    let w = Resize { height: 0.0, ..r };
    assert_eq!(w.apply(6000.0, 4000.0), (400, 267));
    // 4 × 6 inches at 300 ppi = 1200 × 1800 pixels
    let inch = Resize { value: 4.0, height: 6.0, unit: SizeUnit::Inches, ..r };
    assert_eq!(inch.in_pixels(300).apply(4000.0, 6000.0), (1200, 1800));
    let cm = Resize { mode: ResizeMode::LongEdge, value: 2.54, unit: SizeUnit::Cm, ..Default::default() };
    assert_eq!(cm.in_pixels(100).value, 100.0);
    // the unit and ppcm survive the params
    let o = ExportOptions { resize: Some(inch), resolution_unit: ResolutionUnit::Cm, ..Default::default() };
    assert_eq!(ExportOptions::from_params(&o.to_json()).unwrap(), o);
    assert!(ExportOptions::from_params(&json!({"resolutionUnit": "furlong"})).is_err());
}

#[test]
fn ask_what_to_do_lists_existing_files_and_honours_answers() {
    let dir = temp_dir("export-ask");
    let mut s = Session::with_demo();
    let ids: Vec<_> = s.catalog.photos().take(2).map(|p| p.id).collect();
    let o = ExportOptions { conflict: Conflict::Ask, resize: Some(Resize::long_edge(32)), ..Default::default() };
    let to = Destination { dir: dir.to_string_lossy().to_string(), exact: None };
    let exists = |p: &str| std::path::Path::new(p).exists();
    let items = prepare_batch(&mut s, &ids, &o).unwrap();
    assert!(existing_targets(&items, &o, &to, &exists).is_empty());
    let r = run_batch(items, &o, &to, &mut |p, b| write_file(p, b), &exists, true, &mut |_, _| true).unwrap();
    assert_eq!(r.len(), 2);
    let items = prepare_batch(&mut s, &ids, &o).unwrap();
    let found = existing_targets(&items, &o, &to, &exists);
    assert_eq!(found.iter().map(|f| f.0).collect::<Vec<_>>(), ids);
    // answered: the first gets a new name, the second is skipped
    let mut items = prepare_batch(&mut s, &ids, &o).unwrap();
    apply_resolutions(&mut items, Some(&json!({ids[0].0.to_string(): "unique", ids[1].0.to_string(): "skip"})));
    let r = run_batch(items, &o, &to, &mut |p, b| write_file(p, b), &exists, true, &mut |_, _| true).unwrap();
    assert!(r[0]["path"].as_str().unwrap().contains("-2."), "{r:?}");
    assert!(r[1].get("skipped").is_some(), "{r:?}");
    // unanswered (CLI / MCP): Ask gives a new name rather than overwriting
    let items = prepare_batch(&mut s, &ids, &o).unwrap();
    let r = run_batch(items, &o, &to, &mut |p, b| write_file(p, b), &exists, true, &mut |_, _| true).unwrap();
    assert!(r.iter().all(|f| f["path"].as_str().is_some_and(|p| p.contains('-'))), "{r:?}");
    // the command, and a bad answer is refused
    let v = s.execute("export.existingFiles", &json!({"ids": [ids[0].0], "dir": dir.to_string_lossy(), "longEdge": 32})).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(1));
    assert!(ExportOptions::validate("app.export", &json!({"resolve": {"1": "burn"}})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn simple_copyright_watermark_skips_photos_without_copyright() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    let o = ExportOptions { watermark: Some(simple_copyright_watermark()), resize: Some(Resize::long_edge(64)), ..Default::default() };
    let p = prepare_export(&mut s, id, &o, 1).unwrap();
    assert_eq!(p.watermark_text(), None, "no copyright: no watermark");
    s.execute("photo.setMeta", &json!({"ids": [id.0], "copyright": "© 2026 B"})).unwrap();
    let p = prepare_export(&mut s, id, &o, 1).unwrap();
    assert_eq!(p.watermark_text().as_deref(), Some("© 2026 B"));
}

#[test]
fn presets_live_in_folders_and_keep_their_destination() {
    let dir = temp_dir("export-presets");
    let mut s = Session::with_demo();
    let tree = s.export_preset_tree();
    assert_eq!(tree.iter().map(|t| t.0.as_str()).collect::<Vec<_>>(), vec![LIGHTROOM_PRESETS, LIGHTCRAFT_PRESETS, USER_PRESETS]);
    assert!(tree[0].2.contains(&"Burn Full-Sized JPEGs".to_string()) && tree[0].2.contains(&"For Email (Hard Drive)".to_string()));
    for p in builtin_presets() {
        ExportOptions::from_params(&p.params).unwrap_or_else(|e| panic!("{}: {e}", p.name));
    }
    s.execute("export.createPresetFolder", &json!({"name": "Clients"})).unwrap();
    assert!(s.execute("export.createPresetFolder", &json!({"name": "clients"})).is_err());
    assert!(s.execute("export.createPresetFolder", &json!({"name": LIGHTROOM_PRESETS})).is_err());
    s.execute("export.savePreset", &json!({"name": "Web", "folder": "Clients", "params": {"format": "jpeg", "dir": "/tmp/out", "quality": 70}}))
        .unwrap();
    let web = s.export_presets.iter().find(|p| p.name == "Web").unwrap();
    assert_eq!((web.folder.as_str(), web.params["dir"].as_str()), ("Clients", Some("/tmp/out")), "the destination is remembered");
    // Update with Current Settings keeps its folder
    s.execute("export.savePreset", &json!({"name": "Web", "params": {"format": "png", "exportTo": "desktop", "dir": "/tmp/out"}})).unwrap();
    let web = s.export_presets.iter().find(|p| p.name == "Web").unwrap();
    assert_eq!(web.folder, "Clients");
    assert!(web.params.get("dir").is_none(), "no folder kept for the desktop");
    s.execute("export.renamePreset", &json!({"name": "Web", "to": "Web Small"})).unwrap();
    assert!(s.execute("export.renamePreset", &json!({"name": "Web Small", "to": "For Email"})).is_err(), "built-in name");
    s.execute("export.movePreset", &json!({"name": "Web Small", "folder": "User Presets"})).unwrap();
    assert_eq!(s.export_presets[0].folder, "");
    s.execute("export.movePreset", &json!({"name": "Web Small", "folder": "Clients"})).unwrap();
    // to a file and back
    let file = dir.join("mine.json");
    s.execute("export.writePresets", &json!({"path": file.to_string_lossy()})).unwrap();
    let v = s.execute("export.importPresets", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(v["imported"], json!(["Web Small 2"]), "a taken name gets a number");
    // removing the folder keeps its presets
    s.execute("export.deletePresetFolder", &json!({"name": "Clients"})).unwrap();
    assert_eq!(s.export_presets.len(), 2);
    assert!(s.export_presets.iter().all(|p| p.folder_name() == USER_PRESETS));
    // a Lightroom .lrtemplate
    let lr = dir.join("Big TIFF.lrtemplate");
    std::fs::write(
        &lr,
        "s = { title = \"Big TIFF\", type = \"Export\", value = { LR_format = \"TIFF\", LR_export_bitDepth = 16, LR_size_doConstrain = false } }",
    )
    .unwrap();
    let v = s.execute("export.importPresets", &json!({"path": lr.to_string_lossy(), "folder": "From Lightroom"})).unwrap();
    assert_eq!(v["imported"], json!(["Big TIFF"]));
    let p = s.export_params(&json!({"preset": "Big TIFF"})).unwrap();
    let o = ExportOptions::from_params(&p).unwrap();
    assert_eq!((o.format, o.bit_depth), (ExportFormat::Tiff, Some(16)));
    assert!(s.export_preset_tree().iter().any(|t| t.0 == "From Lightroom"));
    assert!(s.execute("export.importPresets", &json!({"path": dir.join("missing.json").to_string_lossy()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn preview_estimates_the_file_size() {
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    let o = ExportOptions { quality: 90, ..Default::default() };
    let job = preview_job(&mut s, id, &o, 256).unwrap().unwrap();
    let full = job.size;
    let pv = job.run().unwrap();
    assert!(pv.width.max(pv.height) <= 256 && pv.image.width == pv.width && pv.image.height == pv.height);
    let est = pv.estimated_bytes.unwrap();
    // the real file, for comparison
    let real = export_photo(&mut s, id, &o, 1).unwrap();
    assert!(real.width.abs_diff(full.0) <= 2 && real.height.abs_diff(full.1) <= 2, "{}×{} vs {full:?}", real.width, real.height);
    let ratio = est as f64 / real.bytes.len() as f64;
    assert!((0.3..3.0).contains(&ratio), "estimate {est} vs {}", real.bytes.len());
    // a size limit caps it
    let capped = preview_job(&mut s, id, &ExportOptions { limit_kb: Some(10), ..o }, 256).unwrap().unwrap().run().unwrap();
    assert!(capped.estimated_bytes.unwrap() <= 10 * 1024);
    assert_eq!(human_bytes(1536), "2 KB");
    assert_eq!(human_bytes(3 * 1_048_576), "3.0 MB");
}

#[test]
fn photoshop_is_found_in_its_folder() {
    let dir = temp_dir("export-ps");
    std::fs::create_dir_all(dir.join("Adobe Photoshop 2025/Adobe Photoshop 2025.app")).unwrap();
    std::fs::create_dir_all(dir.join("Adobe Photoshop 2026/Adobe Photoshop 2026.app")).unwrap();
    std::fs::create_dir_all(dir.join("Other.app")).unwrap();
    assert!(photoshop_in(&dir).unwrap().ends_with("Adobe Photoshop 2026.app"));
    assert_eq!(photoshop_in(&dir.join("nothing")), None);
    let _ = std::fs::remove_dir_all(&dir);
}
