//! Headless tests of dragging the crop frame's edges with the mouse (Bryan, 2026-11-24: on a warped
//! merged pano, dragging the bottom edge moved the whole crop instead of just the bottom).

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);

fn demo() -> Headless {
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    Headless::new(app, [1400.0, 900.0], 1.0)
}

fn exec(h: &mut Headless, command: &str, params: Value) {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    h.settle(T);
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().rev().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn crop(h: &Headless) -> lightcraft_geom::Rect {
    let id = h.app.session.active().unwrap();
    h.app.session.develop_of(id).unwrap().crop.geometry.rect
}

fn drag(h: &mut Headless, a: egui::Pos2, b: egui::Pos2, steps: u64) {
    let r = h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": b.x, "toY": b.y, "steps": steps}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
}

fn crop_tool(warp: bool) -> Headless {
    let mut h = demo();
    h.request("ui.set", json!({"module": "develop", "view": "detail"}), T);
    if warp {
        // a perspective warp: the photo is no longer a rectangle on screen (like a merged pano)
        exec(&mut h, "develop.set", json!({"values": {"geometry.vertical": 40, "geometry.horizontal": -20}}));
    }
    exec(&mut h, "panel.crop", json!({}));
    exec(&mut h, "crop.set", json!({"rect": [0.2, 0.2, 0.8, 0.8]}));
    h.step();
    h
}

fn only_bottom_moved(before: lightcraft_geom::Rect, after: lightcraft_geom::Rect, what: &str) {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    assert!(near(after.x0, before.x0) && near(after.y0, before.y0) && near(after.x1, before.x1), "{what}: other edges moved {before:?} -> {after:?}");
    assert!(after.y1 < before.y1 - 0.02, "{what}: the bottom must move up {before:?} -> {after:?}");
}

/// Grabbing the bottom edge — on its tick, or anywhere along it — and dragging it up moves only the
/// bottom, on a plain photo and a warped one, slow or in one fast flick.
#[test]
fn dragging_the_bottom_edge_moves_only_the_bottom() {
    for warp in [false, true] {
        for (along, steps) in [(0.5, 12), (0.5, 1), (0.25, 12), (0.8, 2)] {
            let mut h = crop_tool(warp);
            let (bl, br) = (widget(&h, "cropHandle:3").center(), widget(&h, "cropHandle:2").center());
            let a = bl + (br - bl) * along;
            let before = crop(&h);
            drag(&mut h, a, a - egui::vec2(0.0, 80.0), steps);
            only_bottom_moved(before, crop(&h), &format!("warp {warp}, {along} along, {steps} steps"));
            if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
                let p = std::path::Path::new(&dir).join(format!("crop-bottom-drag-warp-{warp}-{along}-{steps}.png"));
                let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
                assert_eq!(r["ok"], true, "{r}");
            }
        }
    }
}

/// Inside the frame, away from the edges, a drag still moves the whole crop.
#[test]
fn dragging_inside_the_crop_still_moves_it() {
    let mut h = crop_tool(false);
    let c = (widget(&h, "cropHandle:0").center() + widget(&h, "cropHandle:2").center().to_vec2()) * 0.5;
    let before = crop(&h);
    drag(&mut h, c, c + egui::vec2(40.0, 0.0), 12);
    let after = crop(&h);
    assert!(after.x0 > before.x0 + 0.01 && (after.width() - before.width()).abs() < 1e-6, "{before:?} -> {after:?}");
}
