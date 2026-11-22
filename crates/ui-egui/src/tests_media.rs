//! Video, publish and the toast / Export dialog fixes, headless (with `LIGHTCRAFT_TEST_SHOTS=<dir>`
//! each writes a real-size screenshot there).

use serde_json::json;

use crate::headless::Headless;
use crate::tests_library2::{T, headless, settle, shot};

fn widget_rect(h: &mut Headless, id: &str) -> Option<[f64; 4]> {
    let r = h.request("ui.widgets", json!({}), T);
    let w = r["result"].as_array()?.iter().find(|w| w["id"] == id)?.clone();
    let v = |i: usize| w["rect"][i].as_f64().unwrap_or(0.0);
    Some([v(0), v(1), v(2), v(3)])
}

/// The Export dialog is taller than a small window: its options scroll inside it and the window,
/// with its OK button, stays on screen.
#[test]
fn export_dialog_fits_a_small_window() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.catalog.photos().next().unwrap().id.0;
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let mut h = headless(s);
    h.size = egui::vec2(1100.0, 560.0);
    h.app.run("dialog.export", json!({})).unwrap();
    settle(&mut h);
    settle(&mut h);
    let w = widget_rect(&mut h, "dialog:window").expect("dialog on screen");
    assert!(w[1] >= -0.5 && w[1] + w[3] <= 560.5, "the dialog runs off the window: {w:?}");
    let ok = widget_rect(&mut h, "button:dialogOk").expect("OK button");
    assert!(ok[1] + ok[3] <= 560.0, "the OK button is off screen: {ok:?}");
    shot(&mut h, "export-small-window.png");
}

/// A toast set after a modal folder picker (every clock still at the time before the picker)
/// stays for its whole duration from the next frame on: the tethered-capture toast never showed.
#[test]
fn tether_toast_survives_a_modal_picker() {
    let dir = crate::tests_library2::temp("tether-toast");
    let mut h = headless(lightcraft_engine::Session::with_demo());
    let folder = dir.0.to_string_lossy().to_string();
    // the picker "blocks" for a minute: the frame clock jumps past any toast timed before it
    h.app.services.pick_folder = Some(Box::new(move || Some(folder.clone())));
    h.app.run("dialog.tetherStart", json!({})).unwrap();
    h.advance_time(60.0);
    settle(&mut h);
    let toast = h.app.ui.toast.clone().map(|t| t.0).unwrap_or_default();
    assert!(toast.starts_with("Tethered:"), "the toast is gone: {toast:?}");
    let _ = h.app.run("dialog.tetherStop", json!({}));
}
