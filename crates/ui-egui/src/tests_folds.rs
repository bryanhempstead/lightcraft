//! Every list or section folds by its header and the fold is remembered (Bryan, 2026-11-23):
//! `crate::folds`, the `ui.fold` / `ui.folds` commands, the fold map's save / load with ui.json,
//! and folded sections' contents gone from the screen.

use serde_json::json;

use crate::headless::Headless;
use crate::tests_library2::{T, click, headless, request, settle, shot};

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn has_text(h: &Headless, text: &str) -> bool {
    h.view.texts().iter().any(|(s, _, _)| s == text)
}

#[test]
fn a_fold_is_a_command_and_survives_saving_and_loading_the_ui_state() {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    let r = request(&mut h, "engine.execute", json!({"command": "ui.fold", "params": {"id": "settings.general.library", "open": false}}));
    assert_eq!(r["result"], json!({"id": "settings.general.library", "open": false}), "{r}");
    // no `open`: toggles
    let r = request(&mut h, "engine.execute", json!({"command": "ui.fold", "params": {"id": "metadata.exif"}}));
    assert_eq!(r["result"]["open"], false, "{r}");
    let r = request(&mut h, "engine.execute", json!({"command": "ui.fold", "params": {"id": "metadata.exif"}}));
    assert_eq!(r["result"]["open"], true, "{r}");
    request(&mut h, "engine.execute", json!({"command": "ui.fold", "params": {"id": "develop.detail.sharpening", "open": false}}));
    let r = request(&mut h, "engine.execute", json!({"command": "ui.folds"}));
    assert_eq!(r["result"]["folded"], json!(["develop.detail.sharpening", "settings.general.library"]), "{r}");
    // bad params are errors, never a crash
    for bad in [json!({}), json!({"id": ""}), json!({"id": "x", "open": "yes"})] {
        let r = h.request("engine.execute", json!({"command": "ui.fold", "params": bad}), T);
        assert_eq!(r["ok"], false, "{r}");
    }
    // saved with the UI state (ui.json) and read back
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    let back = serde_json::from_str::<crate::UiState>(&saved).unwrap().sanitized();
    assert_eq!(back.folds, h.app.ui.folds);
    assert_eq!(back.folds.get("settings.general.library"), Some(&false));
    assert_eq!(back.folds.get("metadata.exif"), Some(&true));
    // an older ui.json without folds: every section open
    let old: crate::UiState = serde_json::from_value(json!({"thumbSize": 200})).unwrap();
    assert!(old.folds.is_empty());
    // a header click lands in the UI state too (on the next frame)
    h.app.run("app.settings", json!({"tab": "general"})).unwrap();
    settle(&mut h);
    click(&mut h, "fold:settings.general.startup");
    assert_eq!(h.app.ui.folds.get("settings.general.startup"), Some(&false));
}

/// Folded sections hide what is in them, open ones show it; Settings stays laid out cleanly with
/// some groups folded.
#[test]
fn folded_sections_hide_their_contents() {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    // Settings ▸ General ▸ Startup (a header click) and Culling (the command)
    h.app.run("app.settings", json!({"tab": "general"})).unwrap();
    settle(&mut h);
    assert!(has(&h, "button:settingsStartup-0") && has(&h, "fold:settings.general.startup"));
    click(&mut h, "fold:settings.general.startup");
    assert!(!has(&h, "button:settingsStartup-0"), "Startup folded");
    assert!(has(&h, "fold:settings.general.culling"), "the next heading still there");
    let dialog = h.app.widgets.iter().find(|(w, _)| w == "dialog:window").map(|(_, r)| *r).unwrap();
    let faults = crate::tests_panels::layout_faults(&h, dialog);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "folds-settings.png");
    click(&mut h, "fold:settings.general.startup");
    assert!(has(&h, "button:settingsStartup-0"), "unfolded again");
    // Settings ▸ Shortcuts: a group
    h.app.run("app.settings", json!({"tab": "shortcuts"})).unwrap();
    settle(&mut h);
    let group = h.app.widgets.iter().find_map(|(w, _)| w.strip_prefix("fold:settings.shortcuts.group:").map(str::to_string)).unwrap();
    let rows = |h: &Headless| h.app.widgets.iter().filter(|(w, _)| w.starts_with("button:keysRec-")).count();
    let before = rows(&h);
    click(&mut h, &format!("fold:settings.shortcuts.group:{group}"));
    assert!(rows(&h) < before, "{group}'s rows folded away: {before} → {}", rows(&h));
    h.app.ui.dialog = None;
    settle(&mut h);
    // Library ▸ Metadata ▸ EXIF
    h.app.ui.library_sections = vec!["metadata".into()];
    h.app.ui.metadata_view = "exif".into();
    settle(&mut h);
    assert!(has_text(&h, "Date Time Original"), "EXIF open");
    click(&mut h, "fold:metadata.exif");
    assert!(!has_text(&h, "Date Time Original"), "EXIF folded");
    assert!(has(&h, "fold:metadata.exif"), "its title stays");
    // Develop ▸ Detail ▸ Sharpening
    request(&mut h, "engine.execute", json!({"command": "view.develop"}));
    h.app.ui.open_sections = vec!["detail".into()];
    settle(&mut h);
    assert!(has(&h, "slider:detail.sharpenAmount") && has(&h, "slider:detail.nrLuminance"));
    click(&mut h, "fold:develop.detail.sharpening");
    assert!(!has(&h, "slider:detail.sharpenAmount"), "Sharpening folded");
    assert!(has(&h, "slider:detail.nrLuminance"), "Noise Reduction still open");
    // the command folds what's on screen too
    request(&mut h, "engine.execute", json!({"command": "ui.fold", "params": {"id": "develop.detail.noise.reduction", "open": false}}));
    settle(&mut h);
    assert!(!has(&h, "slider:detail.nrLuminance"), "folded by ui.fold");
    let r = request(&mut h, "engine.execute", json!({"command": "ui.folds"}));
    let shown = r["result"]["onScreen"].as_array().unwrap();
    assert!(shown.contains(&json!({"id": "develop.detail.sharpening", "open": false})), "{r}");
    shot(&mut h, "folds-develop.png");
}
