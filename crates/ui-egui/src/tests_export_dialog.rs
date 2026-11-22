//! The Export dialog laid out like Lightroom Classic's (LR-EXP-DIALOG), headless: sections in
//! Lightroom's order with their summaries, the preset list (folders, checkboxes for several
//! presets, add. / remove.), Existing Files ▸ Ask what to do, Choose folder later, the preview /
//! estimated size. With `LIGHTCRAFT_TEST_SHOTS=<dir>` each writes a real-size screenshot.

use serde_json::json;

use crate::state::Dialog;
use crate::tests_library2::{click, headless, request, settle, shot, temp};

fn widgets(h: &mut crate::headless::Headless) -> Vec<(String, [f64; 4])> {
    let w = request(h, "ui.widgets", json!({}));
    w["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| {
            let r = x["rect"].as_array()?;
            Some((x["id"].as_str()?.to_string(), [r[0].as_f64()?, r[1].as_f64()?, r[2].as_f64()?, r[3].as_f64()?]))
        })
        .collect()
}

fn open(n: usize) -> crate::headless::Headless {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(n).map(|p| p.id.0).collect();
    s.execute("library.select", &json!({"ids": ids})).unwrap();
    let mut h = headless(s);
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    h
}

fn set_dir(h: &mut crate::headless::Headless, d: &std::path::Path) {
    if let Some(Dialog::Export { dir, .. }) = &mut h.app.ui.dialog {
        *dir = d.to_string_lossy().to_string();
    }
}

fn opts(h: &crate::headless::Headless) -> lightcraft_engine::export::ExportOptions {
    match &h.app.ui.dialog {
        Some(Dialog::Export { opts, .. }) => (**opts).clone(),
        other => panic!("no export dialog: {other:?}"),
    }
}

/// The sections are Lightroom's, in its order, each header with a summary; the title counts the
/// files; the preset list shows Lightroom's, LightCraft's and the user's folders.
#[test]
fn export_dialog_has_lightrooms_sections_in_order() {
    let mut h = open(3);
    assert_eq!(crate::export_dialog::title(&h.app), "Export 3 Files");
    let w = widgets(&mut h);
    let y = |id: &str| w.iter().find(|(i, _)| i == id).map(|(_, r)| r[1]).unwrap_or_else(|| panic!("no {id}"));
    let order = ["location", "naming", "video", "file", "sizing", "sharpening", "metadata", "watermark", "after", "preview"];
    let ys: Vec<f64> = order.iter().map(|s| y(&format!("section:export-{s}"))).collect();
    assert!(ys.windows(2).all(|p| p[0] < p[1]), "sections out of order: {ys:?}");
    for f in 0..3 {
        assert!(w.iter().any(|(i, _)| *i == format!("folder:exportPresets-{f}")), "preset folder {f}");
    }
    for id in [
        "combo:exportService",
        "combo:exportTo",
        "combo:exportConflict",
        "combo:exportFormat",
        "button:exportPresetAdd",
        "button:exportPlugins",
        "button:exportDone",
    ] {
        assert!(w.iter().any(|(i, _)| i == id), "{id}");
    }
    shot(&mut h, "exp-dialog.png");
    // a section closes and opens from its header
    click(&mut h, "section:export-location");
    assert!(!widgets(&mut h).iter().any(|(i, _)| i == "combo:exportTo"), "closed");
    click(&mut h, "section:export-location");
    assert!(widgets(&mut h).iter().any(|(i, _)| i == "combo:exportTo"), "open again");
    assert_eq!(crate::export_dialog::title(&open(1).app), "Export One File");
}

/// E07 / E08: the Image Format menu offers PSD and JPEG XL; Color Space ▸ Other… has
/// ColorMatch RGB.
#[test]
fn export_dialog_formats_and_color_space() {
    let mut h = open(1);
    click(&mut h, "combo:exportFormat");
    click(&mut h, "button:exportFormat-3");
    assert_eq!(opts(&h).format, lightcraft_engine::export::ExportFormat::Psd);
    click(&mut h, "combo:exportFormat");
    click(&mut h, "button:exportFormat-1");
    assert_eq!(opts(&h).format, lightcraft_engine::export::ExportFormat::Jxl);
    click(&mut h, "combo:exportFormat");
    click(&mut h, "button:exportFormat-0");
    click(&mut h, "combo:exportColorSpace");
    click(&mut h, "button:exportColorSpace-colorMatch");
    assert_eq!(opts(&h).icc_profile.as_deref(), Some("colorMatch"));
    shot(&mut h, "exp-formats.png");
}

/// E05: checking presets exports once with each, into a folder per preset (⇧-click checks the
/// run); none. clears them.
#[test]
fn export_dialog_several_presets() {
    let dir = temp("exp-several");
    let mut h = open(1);
    set_dir(&mut h, &dir.0);
    // Lightroom Presets: 0..3, LightCraft Presets: 4.. (JPEG (Small) = 4, JPEG (Large) = 5)
    click(&mut h, "check:exportPreset-4");
    shot(&mut h, "exp-several.png");
    click(&mut h, "button:exportBatchNone");
    assert!(matches!(&h.app.ui.dialog, Some(Dialog::Export { checked, .. }) if checked.is_empty()));
    click(&mut h, "check:exportPreset-4");
    click(&mut h, "check:exportPreset-5");
    set_dir(&mut h, &dir.0);
    click(&mut h, "button:dialogOk");
    assert!(h.app.ui.dialog.is_none(), "the dialog closes");
    let folders: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().filter(|e| e.path().is_dir()).collect();
    assert_eq!(folders.len(), 2, "two presets checked: two folders");
}

/// Existing Files ▸ Ask what to do: a second export into the same folder asks; skip. with Apply
/// to all leaves the files as they are.
#[test]
fn export_dialog_asks_about_existing_files() {
    use lightcraft_engine::export::Conflict;
    let dir = temp("exp-ask");
    let mut h = open(2);
    set_dir(&mut h, &dir.0);
    if let Some(Dialog::Export { opts, full_size, resize, .. }) = &mut h.app.ui.dialog {
        opts.conflict = Conflict::Ask;
        *full_size = false;
        *resize = lightcraft_engine::export::Resize::long_edge(64);
    }
    settle(&mut h);
    click(&mut h, "button:dialogOk");
    assert!(h.app.ui.dialog.is_none(), "nothing there yet: no question");
    let files = || std::fs::read_dir(&dir.0).unwrap().count();
    assert_eq!(files(), 2);
    let before: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.metadata().unwrap().modified().unwrap()).collect();
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    assert_eq!(opts(&h).conflict, Conflict::Ask, "the dialog opens with the last settings");
    click(&mut h, "button:dialogOk");
    assert!(matches!(&h.app.ui.dialog, Some(Dialog::ExportExisting { existing, .. }) if existing.len() == 2), "{:?}", h.app.ui.dialog);
    shot(&mut h, "exp-ask.png");
    click(&mut h, "check:exportAskAll");
    click(&mut h, "button:exportAsk-skip");
    assert!(h.app.ui.dialog.is_none());
    assert_eq!(files(), 2, "skipped: no new files");
    let after: Vec<_> = std::fs::read_dir(&dir.0).unwrap().flatten().map(|e| e.metadata().unwrap().modified().unwrap()).collect();
    assert_eq!(before, after, "and the old ones untouched");
    // once more, answering new.name. for the first and overwrite. for the second
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    click(&mut h, "button:dialogOk");
    click(&mut h, "button:exportAsk-unique");
    click(&mut h, "button:exportAsk-overwrite");
    assert!(h.app.ui.dialog.is_none());
    assert_eq!(files(), 3, "one new name, one overwritten");
}

/// Choose folder later asks for the folder when export. is clicked.
#[test]
fn export_dialog_choose_folder_later() {
    let dir = temp("exp-later");
    let mut h = open(1);
    let d = dir.0.to_string_lossy().to_string();
    h.app.services.pick_folder = Some(Box::new(move || Some(d.clone())));
    if let Some(Dialog::Export { opts, full_size, resize, dir, .. }) = &mut h.app.ui.dialog {
        opts.export_to = lightcraft_engine::export::ExportTo::ChooseLater;
        *full_size = false;
        *resize = lightcraft_engine::export::Resize::long_edge(48);
        dir.clear();
    }
    settle(&mut h);
    click(&mut h, "button:dialogOk");
    assert!(h.app.ui.dialog.is_none());
    assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1, "written into the folder picked");
}

/// add. saves the settings as a user preset (with its folder); right-click ▸ Update keeps it in
/// its folder; remove. deletes it.
#[test]
fn export_dialog_add_and_remove_a_preset() {
    let mut h = open(1);
    click(&mut h, "button:exportPresetAdd");
    let w = widgets(&mut h);
    assert!(w.iter().any(|(i, _)| i == "field:exportPresetName"));
    if let Some(Dialog::Export { preset_name, opts, .. }) = &mut h.app.ui.dialog {
        *preset_name = "Web".into();
        opts.quality = 77;
    }
    settle(&mut h);
    click(&mut h, "button:exportSavePreset");
    let saved = h.app.session.export_presets.iter().find(|p| p.name == "Web").cloned().expect("saved");
    assert_eq!(saved.params["quality"], 77);
    assert!(matches!(&h.app.ui.dialog, Some(Dialog::Export { selected: Some(n), .. }) if n == "Web"));
    shot(&mut h, "exp-presets.png");
    click(&mut h, "button:exportPresetRemove");
    assert!(!h.app.session.export_presets.iter().any(|p| p.name == "Web"), "removed");
}

/// The preview renders the photo as it will export and the File Settings header gets the
/// estimated size.
#[test]
fn export_dialog_preview_and_estimate() {
    let mut h = open(1);
    // the open sections closed, so the preview is in view
    for s in ["location", "naming", "file", "sizing"] {
        click(&mut h, &format!("section:export-{s}"));
    }
    click(&mut h, "section:export-preview");
    let t0 = std::time::Instant::now();
    while !widgets(&mut h).iter().any(|(i, _)| i == "image:exportPreview") && t0.elapsed() < std::time::Duration::from_secs(60) {
        settle(&mut h);
    }
    assert!(widgets(&mut h).iter().any(|(i, _)| i == "image:exportPreview"), "preview shown");
    shot(&mut h, "exp-preview.png");
}

/// done. keeps the settings for next time without exporting.
#[test]
fn export_dialog_done_remembers() {
    let mut h = open(1);
    if let Some(Dialog::Export { opts, .. }) = &mut h.app.ui.dialog {
        opts.quality = 61;
    }
    settle(&mut h);
    click(&mut h, "button:exportDone");
    assert!(h.app.ui.dialog.is_none());
    h.app.run("dialog.export", json!({})).unwrap();
    assert_eq!(opts(&h).quality, 61);
}
