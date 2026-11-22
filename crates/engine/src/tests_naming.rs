//! E01: Lightroom's filename templates and tokens — named templates, Custom Text, Total #,
//! Import # / Image #, date parts, metadata tokens, extension case — in Rename, import and export.

use serde_json::json;

use crate::Session;
use crate::rename::{Batch, ExtCase, expand_in, expand_tokens_in, sample_photo};

#[test]
fn lightroom_tokens_expand() {
    let p = sample_photo(); // IMG_0042.CR3, 2026-01-14 05:58:48, Canon EOS R5
    let b = Batch { total: 24, text: "Wedding".into(), shoot: "Smith".into(), import_no: 7, image_start: 100, ..Default::default() };
    let t = |tpl: &str, seq: usize| expand_tokens_in(tpl, &p, seq, 1, &b);
    assert_eq!(t("{text} ({seq} of {total})", 3), "Wedding (3 of 24)");
    assert_eq!(t("{shoot}-{num}", 1), "Smith-0042");
    assert_eq!(t("{import:3}-{image:5}", 3), "007-00102", "Image # counts per photo");
    assert_eq!(t("{total:3}", 1), "024");
    assert_eq!(t("{yyyymmdd}|{yymmdd}|{yyyy}|{yy}|{mm}|{dd}", 1), "20260114|260114|2026|26|01|14");
    assert_eq!(t("{month}|{mon}|{julian}|{hour}{minute}{second}", 1), "January|Jan|014|055848");
    assert_eq!(t("{longdate}", 1), "January 14, 2026");
    assert_eq!(t("{date:%j %B %b}", 1), "014 January Jan");
    assert_eq!(t("{make}|{model}|{EXT}|{ext}", 1), "Canon|EOS R5|CR3|CR3");
    assert_eq!(t("{dimensions}", 1), "6000x4000");
    // every token the help lists expands (none stays as typed)
    for tok in crate::rename::TOKENS {
        assert!(crate::rename::unknown_tokens(tok.tag).is_empty(), "{} is unknown", tok.tag);
    }
    // extension case
    let lower = Batch { ext_case: ExtCase::Lower, ..Default::default() };
    assert_eq!(expand_in("{name}", &p, 1, &lower), "IMG_0042.cr3");
    // a leap year's March
    let mut q = sample_photo();
    q.captured = Some("2024-03-01T00:00:00".into());
    assert_eq!(expand_tokens_in("{julian}", &q, 1, 1, &Batch::default()), "061");
}

#[test]
fn named_templates_list_save_and_rename() {
    let mut s = Session::with_demo();
    let list = s.execute("naming.templates", &json!({})).unwrap();
    let names: Vec<&str> = list["templates"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(names.len(), 10, "Lightroom's ten built-ins");
    assert!(names.contains(&"Custom Name - Sequence") && names.contains(&"Date - Filename"));
    assert!(s.execute("naming.saveTemplate", &json!({"name": "Filename", "template": "{name}"})).is_err(), "built-in names are taken");
    s.execute("naming.saveTemplate", &json!({"name": "Client", "template": "{text}_{date:%Y%m%d}_{seq:3}"})).unwrap();
    let ids: Vec<u64> = s.catalog.photos().take(3).map(|p| p.id.0).collect();
    let plan = s.execute("photo.renamePreview", &json!({"ids": ids, "templateName": "Client", "text": "Smith {x}"})).unwrap();
    let to: Vec<&str> = plan.as_array().unwrap().iter().filter_map(|p| p["to"].as_str()).collect();
    assert!(to[0].starts_with("Smith x_") && to[0].contains("_001"), "{to:?}");
    let plan = s.execute("photo.renamePreview", &json!({"ids": ids, "templateName": "Custom Name (x of y)", "text": "Trip"})).unwrap();
    assert!(plan[1]["to"].as_str().unwrap().starts_with("Trip (2 of 3)"), "{plan}");
    assert!(s.execute("photo.renamePreview", &json!({"ids": ids, "templateName": "Nope"})).is_err());
    s.execute("naming.deleteTemplate", &json!({"name": "client"})).unwrap();
    assert!(s.execute("naming.deleteTemplate", &json!({"name": "Filename"})).is_err());
}

#[test]
fn export_names_use_custom_text_total_and_extension_case() {
    let o = crate::export::ExportOptions::from_json(&json!({"naming": "{text}-{seq:2}-of-{total}", "customText": "Proof", "extCase": "upper"}));
    let p = sample_photo();
    assert_eq!(o.file_name_in(&p, 2, 5), "Proof-02-of-5.JPG");
    let back = crate::export::ExportOptions::from_json(&o.to_json());
    assert_eq!(back, o, "round trip");
    assert!(crate::export::ExportOptions::from_params(&json!({"extCase": "sideways"})).is_err());
}

#[test]
fn import_numbers_count_imports_and_photos() {
    let dir = std::env::temp_dir().join(format!("lc-naming-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for i in 0..2u8 {
        let img = lightcraft_raster::Rgba8 { width: 8, height: 8, data: vec![[i * 90, 20, 30, 255]; 64] };
        let png = lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(&img), &Default::default()).unwrap();
        std::fs::create_dir_all(dir.join("card")).unwrap();
        std::fs::write(dir.join(format!("card/IMG_{i}.png")), png).unwrap();
    }
    let mut s = Session::new();
    s.execute(
        "library.import",
        &json!({"paths": [dir.join("card").to_string_lossy()], "mode": "copy", "destination": dir.join("lib").to_string_lossy(), "organize": "flat",
            "rename": "{shoot}-{import}-{image:3}", "shootName": "Smith", "renameExtCase": "upper"}),
    )
    .unwrap();
    let mut names: Vec<String> = s.catalog.photos().map(|p| p.file_name.clone()).collect();
    names.sort();
    assert_eq!(names, ["Smith-1-001.PNG", "Smith-1-002.PNG"]);
    assert_eq!((s.naming_counters.imports, s.naming_counters.images), (1, 2));
    let _ = std::fs::remove_dir_all(&dir);
}

/// E02: Post-Processing — what runs after an export, for the files it wrote.
#[test]
fn after_export_commands() {
    use crate::export::{AfterExport, ExportOptions, post_process_command};
    let files = vec!["/out/a.jpg".to_string(), "/out/b.jpg".to_string()];
    let o = |after, app: &str| ExportOptions { after, after_app: app.into(), ..Default::default() };
    assert_eq!(post_process_command(&o(AfterExport::Nothing, ""), &files), None);
    assert_eq!(post_process_command(&o(AfterExport::ShowInFinder, ""), &[]), None, "nothing written");
    let (prog, args) = post_process_command(&o(AfterExport::ShowInFinder, ""), &files).unwrap();
    if cfg!(target_os = "macos") {
        assert_eq!((prog.as_str(), args[0].as_str(), args.len()), ("/usr/bin/open", "-R", 3));
        let (prog, args) = post_process_command(&o(AfterExport::OpenInApp, "Adobe Photoshop 2026"), &files).unwrap();
        assert_eq!((prog.as_str(), &args[..2]), ("/usr/bin/open", &["-a".to_string(), "Adobe Photoshop 2026".to_string()][..]));
        let (_, args) = post_process_command(&o(AfterExport::OpenInApp, ""), &files).unwrap();
        assert_eq!(args, files, "the default app");
    }
    // an Export Action script runs with the files
    let dir = std::env::temp_dir().join(format!("lc-after-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("upload.sh");
    std::fs::write(&script, "#!/bin/sh\n").unwrap();
    let (prog, args) = post_process_command(&o(AfterExport::OpenInApp, &script.to_string_lossy()), &files).unwrap();
    assert_eq!((prog, args), (script.to_string_lossy().to_string(), files.clone()));
    let _ = std::fs::remove_dir_all(&dir);
    // round trip + validation
    let e = ExportOptions::from_json(&json!({"after": "showInFinder"}));
    assert_eq!(e.after, AfterExport::ShowInFinder);
    assert_eq!(ExportOptions::from_json(&e.to_json()), e);
    assert!(ExportOptions::from_params(&json!({"after": "explode"})).is_err());
    // `post_process` with nothing to do runs nothing
    assert!(crate::export::post_process(&ExportOptions::default(), &[json!({"path": "/x.jpg"})]).is_none());
}

/// L31: export status — recorded per photo, kept with the library, an Exported filter, Reset.
#[test]
fn export_status_is_recorded_filtered_and_reset() {
    let dir = std::env::temp_dir().join(format!("lc-exported-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.open_library(dir.join("lib"), true).unwrap();
    let id = s.visible_cloned()[0];
    let o = crate::export::ExportOptions { resize: Some(crate::export::Resize::long_edge(64)), ..Default::default() };
    let to = crate::export::Destination { dir: dir.join("out").to_string_lossy().to_string(), exact: None };
    let files = crate::export::export_batch(&mut s, &[id], &o, &to, &mut crate::export::write_file, &|p| std::path::Path::new(p).exists()).unwrap();
    assert_eq!(files[0]["photo"], id.0);
    s.record_exports(&files);
    let st = s.execute("library.exportStatus", &json!({"ids": [id.0]})).unwrap();
    assert!(st[0]["exported"].is_string(), "{st}");
    let n = s.visible_cloned().len();
    assert_eq!(s.execute("library.filter", &json!({"exported": true})).unwrap()["count"], 1);
    assert_eq!(s.execute("library.filter", &json!({"exported": false})).unwrap()["count"], n - 1);
    s.execute("library.filter", &json!({"exported": null})).unwrap();
    // kept with the library
    s.close_library().unwrap();
    drop(s);
    let mut s = Session::new();
    s.open_library(dir.join("lib"), false).unwrap();
    assert!(s.export_log.contains_key(&id.0));
    assert_eq!(s.execute("library.resetExportStatus", &json!({"ids": [id.0]})).unwrap()["reset"], 1);
    assert!(s.export_log.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
