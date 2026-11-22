//! L19 (ported from upstream): the loupe's Info 1 / Info 2 lines chosen in Settings ▸ Interface,
//! and Show Briefly When Photo Changes.

use serde_json::json;

use crate::state::{InfoOverlay, LoupeInfoField as F};
use crate::tests_library2::{click, headless, settle, shot};

#[test]
fn loupe_info_lines_are_chosen_and_show_briefly() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids: Vec<u64> = s.catalog.photos().take(2).map(|p| p.id.0).collect();
    s.execute("library.select", &json!({"ids": [ids[0]]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [ids[0]], "title": "Morning lake"})).unwrap();
    let mut h = headless(s);
    h.app.run("view.detail", json!({})).unwrap();
    h.app.ui.settings.loupe_info.info1 = [F::Title, F::CameraAndLens, F::None];
    h.app.ui.info_overlay = InfoOverlay::Basic;
    settle(&mut h);
    let p = h.app.session.catalog.photo(lightcraft_catalog::PhotoId(ids[0])).unwrap().clone();
    let lines = crate::panels::detail::info_overlay_lines(&h.app, &p).unwrap();
    assert_eq!(lines[0], "Morning lake");
    assert_eq!(lines.len(), 2, "None adds no line: {lines:?}");
    shot(&mut h, "l19-info1.png");
    // off, with Show Briefly on Info 2: it shows when the photo changes, then goes
    h.app.ui.info_overlay = InfoOverlay::Off;
    h.app.ui.settings.loupe_info.briefly = InfoOverlay::Exposure;
    h.app.run("library.select", json!({"ids": [ids[1]]})).unwrap();
    settle(&mut h);
    assert!(h.app.ui.brief_info.shown, "shown right after the change");
    shot(&mut h, "l19-briefly.png");
    // the settings rows
    h.app.run("app.settings", json!({"tab": "interface"})).unwrap();
    settle(&mut h);
    click(&mut h, "button:settingsLoupeDefaults");
    assert_eq!(h.app.ui.settings.loupe_info, crate::state::LoupeInfo { briefly: InfoOverlay::Off, ..Default::default() });
    shot(&mut h, "l19-settings.png");
}

/// LRC-DEV-VIEWOPTIONS: Develop's loupe keeps its own overlay and Loupe Info lines (⌘I / I cycle
/// the current module's), ⌘J in Develop opens Develop's View Options, and the video time shows
/// frame numbers when asked.
#[test]
fn develop_view_options_are_their_own() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id;
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    s.execute("photo.setMeta", &json!({"ids": [id.0], "title": "Morning lake"})).unwrap();
    let mut h = headless(s);
    h.app.ui.settings.develop_view.loupe_info.info1 = [F::Title, F::None, F::None];
    h.app.run("view.develop", json!({})).unwrap();
    settle(&mut h);
    assert!(h.app.ui.develop_loupe());
    // ⌘I in Develop: Develop's overlay, not the Library's
    let r = h.app.run("view.infoOverlay", json!({})).unwrap();
    assert_eq!(r["infoOverlay"], "basic");
    assert_eq!((h.app.ui.develop_info_overlay, h.app.ui.info_overlay), (InfoOverlay::Basic, InfoOverlay::Off));
    let p = h.app.session.catalog.photo(id).unwrap().clone();
    assert_eq!(crate::panels::detail::info_overlay_lines(&h.app, &p).unwrap(), vec!["Morning lake".to_string()], "Develop's Info 1");
    settle(&mut h);
    shot(&mut h, "dev-view-info1.png");
    // the Library loupe: its own (off) overlay and lines
    h.app.run("view.detail", json!({})).unwrap();
    settle(&mut h);
    assert!(!h.app.ui.develop_loupe());
    assert!(crate::panels::detail::info_overlay_lines(&h.app, &p).is_none(), "the Library's overlay is off");
    h.app.ui.info_overlay = InfoOverlay::Basic;
    let lib = crate::panels::detail::info_overlay_lines(&h.app, &p).unwrap();
    assert_eq!(lib.first().map(String::as_str), Some(p.file_name.as_str()), "the Library's Info 1 starts with the file name");
    // ⌘J: the grid / Library options in Library, Develop's View Options in Develop
    h.app.run("view.gridOptions", json!({})).unwrap();
    assert!(matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Settings { tab }) if tab == "interface"));
    h.app.ui.dialog = None;
    h.app.run("view.develop", json!({})).unwrap();
    h.app.run("view.gridOptions", json!({})).unwrap();
    assert!(matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Settings { tab }) if tab == "develop"));
    settle(&mut h);
    for w in [
        "button:settingsDevInfo-1",
        "combo:devloupeInfo1-0",
        "check:settings.devLoadingMessage",
        "check:settings.videoFrames",
        "check:settings.videoDraft",
    ] {
        assert!(h.app.widgets.iter().any(|(x, _)| x == w), "{w} on dev.view.");
    }
    shot(&mut h, "dev-view-options.png");
    click(&mut h, "check:settings.videoFrames");
    assert!(h.app.ui.settings.develop_view.video_frames);
    assert_eq!(crate::video_ui::time_label(&h.app, 1.52, 10.0), "0:01:13 / 0:10:00");
    click(&mut h, "check:settings.videoFrames");
    assert_eq!(crate::video_ui::time_label(&h.app, 1.52, 10.0), "0:01.52 / 0:10.00");
    click(&mut h, "button:settingsDevLoupeDefaults");
    assert_eq!(h.app.ui.settings.develop_view.loupe_info, crate::state::LoupeInfo::default());
}
