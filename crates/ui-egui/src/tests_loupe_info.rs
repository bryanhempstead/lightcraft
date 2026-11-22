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
