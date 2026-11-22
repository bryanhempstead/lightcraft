//! Adjust Time… headless (LR-LIB-CAPTURETIME), in Bryan's words: "right click on it … it's
//! 2:34 PM, and I'm like oh no, that's supposed to be 3:34 PM for that one, and then everything
//! changes accordingly — not the same time, but corresponding times".
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes the dialog's screenshots there.

use std::time::Duration;

use lightcraft_catalog::dates::iso_seconds;
use lightcraft_catalog::{Op, PhotoId};
use serde_json::json;

use crate::LightcraftApp;
use crate::headless::Headless;
use crate::state::Dialog;

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn demo() -> Headless {
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = crate::Services {
        png: Some(Box::new(png)),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    Headless::new(app, [1400.0, 950.0], 1.0)
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn ok(h: &mut Headless, method: &str, params: serde_json::Value) {
    let r = h.request(method, params, T);
    assert_eq!(r["ok"], true, "{method}: {r}");
    h.step();
    h.step();
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        h.settle(SETTLE);
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

fn time(h: &Headless, id: PhotoId) -> i64 {
    iso_seconds(h.app.session.catalog.photo(id).unwrap().captured.as_deref().unwrap()).unwrap()
}

/// Four photos from two cameras over two days (the newest in the library, so on screen), all
/// selected; the active one is the last.
fn shoot(h: &mut Headless) -> Vec<PhotoId> {
    let ids: Vec<PhotoId> = h.app.session.visible_cloned().iter().take(4).copied().collect();
    let plan = [
        (ids[0], "Canon EOS R6", "2030-06-13T11:02:40"),
        (ids[1], "Canon EOS R6", "2030-06-13T14:34:12"),
        (ids[2], "ILCE-7M4", "2030-06-13T13:37:57"),
        (ids[3], "ILCE-7M4", "2030-06-14T09:15:00"),
    ];
    for (id, camera, t) in plan {
        let mut meta = h.app.session.catalog.photo(id).unwrap().meta.clone();
        meta.camera = camera.into();
        h.app.session.commit("t", Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
        h.app.session.commit("t", Op::SetCaptured { id, captured: Some(t.into()) }).unwrap();
    }
    let all: Vec<u64> = ids.iter().map(|i| i.0).collect();
    ok(h, "engine.execute", json!({"command": "library.select", "params": {"ids": all, "active": ids[3].0}}));
    h.settle(SETTLE);
    ids
}

/// Right-click a photo ▸ Adjust Time… ▸ type 3:34 pm ▸ Enter: that photo is at 3:34 PM and every
/// selected photo moved by the same hour; one undo step puts them back.
#[test]
fn right_click_adjust_time_moves_the_selection_by_one_amount() {
    let mut h = demo();
    let ids = shoot(&mut h);
    let before: Vec<i64> = ids.iter().map(|id| time(&h, *id)).collect();
    let c = widget(&h, &format!("thumb:{}", ids[1].0)).center();
    ok(&mut h, "ui.click", json!({"x": c.x, "y": c.y, "button": "right"}));
    assert!(egui::Popup::is_any_open(&h.view.ctx), "context menu");
    assert_eq!(h.app.session.selection.ids.len(), 4, "a right-click inside the selection keeps it");
    ok(&mut h, "ui.clickWidget", json!({"id": "menu:adjustTime"}));
    h.settle(SETTLE);
    let Some(Dialog::CaptureTime { anchor, mode, time: t, .. }) = h.app.ui.dialog.clone() else { panic!("dialog: {:?}", h.app.ui.dialog) };
    assert_eq!((anchor, mode.as_str(), t.as_str()), (ids[1].0, "set", "2030-06-13 14:34:12"), "anchored on the photo right-clicked");
    assert!(h.app.widgets.iter().any(|(w, _)| w == "captureTime:current"));
    // the field has focus with its text selected: typing replaces it
    ok(&mut h, "ui.text", json!({"text": "3:34 pm"}));
    let Some(Dialog::CaptureTime { time: t, .. }) = h.app.ui.dialog.clone() else { panic!() };
    assert_eq!(t, "3:34 pm");
    h.settle(SETTLE);
    assert!(h.app.widgets.iter().any(|(w, _)| w == "captureTime:after"), "the new time is previewed");
    shot(&mut h, "capture-time-adjust.png");
    // the calendar
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureDate"}));
    h.settle(SETTLE);
    shot(&mut h, "capture-time-calendar.png");
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureDay-14"}));
    let Some(Dialog::CaptureTime { time: t, .. }) = h.app.ui.dialog.clone() else { panic!() };
    assert_eq!(t, "2030-06-14 15:34:12", "a day keeps the typed time");
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureStep-86400"}));
    let Some(Dialog::CaptureTime { time: t, .. }) = h.app.ui.dialog.clone() else { panic!() };
    assert_eq!(t, "2030-06-13 15:34:12", "−1d.");
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureDate"}));
    // Enter applies
    let f = widget(&h, "field:captureTime").center();
    ok(&mut h, "ui.click", json!({"x": f.x, "y": f.y}));
    ok(&mut h, "ui.key", json!({"key": "Enter"}));
    assert!(h.app.ui.dialog.is_none(), "applied and closed: {:?}", h.app.ui.dialog);
    assert_eq!(h.app.session.catalog.photo(ids[1]).unwrap().captured.as_deref(), Some("2030-06-13T15:34:12"));
    for (id, b) in ids.iter().zip(&before) {
        assert_eq!(time(&h, *id) - b, 3600, "{id:?} moved by the same hour");
    }
    // one undo step
    ok(&mut h, "engine.execute", json!({"command": "edit.undo", "params": {}}));
    assert_eq!(ids.iter().map(|id| time(&h, *id)).collect::<Vec<_>>(), before);
}

/// match.cam.: the Sony's clock lines up with the Canon's — only the Sony photos move.
#[test]
fn match_camera_clocks_from_the_dialog() {
    let mut h = demo();
    let ids = shoot(&mut h);
    let before: Vec<i64> = ids.iter().map(|id| time(&h, *id)).collect();
    // the Sony photo ids[2] was taken at the same moment as the Canon's ids[1]
    h.app.run("dialog.captureTime", json!({"anchor": ids[2].0})).unwrap();
    h.settle(SETTLE);
    assert!(matches!(h.app.ui.dialog, Some(Dialog::CaptureTime { anchor, .. }) if anchor == ids[2].0));
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureMode-match"}));
    h.settle(SETTLE);
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureReference"}));
    h.settle(SETTLE);
    // nearest in time first: ids[1] (57 min away) before ids[0]
    ok(&mut h, "ui.clickWidget", json!({"id": "button:captureReference-0"}));
    let Some(Dialog::CaptureTime { reference, .. }) = h.app.ui.dialog.clone() else { panic!() };
    assert_eq!(reference, ids[1].0);
    h.settle(SETTLE);
    shot(&mut h, "capture-time-match.png");
    ok(&mut h, "ui.dialog.confirm", json!({}));
    assert_eq!(time(&h, ids[2]), time(&h, ids[1]));
    assert_eq!(time(&h, ids[3]) - before[3], before[1] - before[2]);
    assert_eq!((time(&h, ids[0]), time(&h, ids[1])), (before[0], before[1]), "the Canon photos stay");
}
