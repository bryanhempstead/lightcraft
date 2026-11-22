//! LRC-UI-IDENTITYPLATE and the launch frame: the identity plate drawn in every spot without
//! covering a control, its settings surviving a restart, bad files reported; the first frame
//! after launch already shows the UI (no blank grey window).

use std::path::PathBuf;
use std::time::Duration;

use egui::{Color32, Rect, pos2};
use serde_json::{Value, json};

use crate::headless::Headless;
use crate::identity::{Place, controls};
use crate::{LightcraftApp, Services};

const SETTLE: Duration = Duration::from_secs(120);

/// A procedural plate: a white disc with a hole, on transparency.
fn ring_png(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    let (cx, cy, r) = (w as f32 / 2.0, h as f32 / 2.0, w.min(h) as f32 * 0.45);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let on = d < r && d > r * 0.3;
            px.extend_from_slice(&[255, 255, 255, if on { 255 } else { 0 }]);
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&px).unwrap();
    }
    out
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("lc-identity-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn demo() -> Headless {
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    app.ui.view = crate::state::ViewMode::PhotoGrid;
    Headless::new(app, [1400.0, 900.0], 1.0)
}

fn plate(h: &mut Headless, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": "app.identityPlate", "params": params}), SETTLE);
    assert_eq!(r["ok"], true, "{r}");
    r["result"].clone()
}

fn drawn(h: &Headless) -> Rect {
    h.app.identity.rect.expect("the plate is drawn")
}

/// Every spot: the plate's (red-tinted) pixels are on screen where it says, and it covers no
/// control — the top bar's buttons, the toolbar, panel headers, the filmstrip's buttons.
#[test]
fn plate_in_every_spot_shows_and_covers_no_control() {
    let d = tmp("spots");
    let file = d.join("ring.png");
    std::fs::write(&file, ring_png(256, 256)).unwrap();
    let mut h = demo();
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_none(), "off by default");
    plate(&mut h, json!({"image": file.to_string_lossy(), "on": true, "size": 56, "tint": "#ff2000", "opacity": 1.0}));
    assert!(h.settle(SETTLE));
    let r = plate(&mut h, json!({}));
    assert_eq!(r["loaded"], json!([256, 256]), "{r}");
    let out = std::env::var_os("LIGHTCRAFT_SNAPSHOT_DIR").map(PathBuf::from);
    for place in Place::ALL {
        plate(&mut h, json!({"place": place.name()}));
        assert!(h.settle(SETTLE));
        let r = drawn(&h);
        let window = Rect::from_min_size(pos2(0.0, 0.0), h.size);
        assert!(window.contains_rect(r), "{place:?}: {r:?} inside the window");
        for c in controls(&h.app.widgets, window) {
            assert!(!c.intersects(r), "{place:?}: plate {r:?} covers a control at {c:?}");
        }
        if place == Place::Bar {
            assert!(r.max.y <= crate::theme::Tokens::default().top_bar_h, "{place:?}: in the top bar: {r:?}");
        }
        let img = h.paint();
        let red = (r.min.y as usize..r.max.y as usize)
            .flat_map(|y| (r.min.x as usize..r.max.x as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let p = img.pixels[y * img.size[0] + x];
                p.r() > 200 && p.g() < 90 && p.b() < 60
            })
            .count();
        assert!(red as f32 > r.area() * 0.2, "{place:?}: plate pixels on screen ({red} of {})", r.area());
        if let Some(dir) = &out {
            crate::tests_identity::save_png(&img, &dir.join(format!("identity-{}.png", place.name())));
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Save a snapshot for a person to look at (only with `LIGHTCRAFT_SNAPSHOT_DIR`).
fn save_png(img: &egui::ColorImage, path: &std::path::Path) {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.size[0] as u32, img.size[1] as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(&img.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect::<Vec<_>>()).unwrap();
    }
    std::fs::write(path, out).unwrap();
}

/// A text plate draws in the theme's text color (or its tint), only in the modules it's on for.
#[test]
fn text_plate_and_modules() {
    let mut h = demo();
    plate(&mut h, json!({"on": true, "kind": "text", "text": "hempstead.studio", "place": "canvas.br", "size": 40}));
    assert!(h.settle(SETTLE));
    let r = drawn(&h);
    assert!(r.width() > r.height() * 3.0, "{r:?}");
    let img = h.paint();
    let t = crate::theme::Tokens::default().text;
    let lit = (r.min.y as usize..r.max.y as usize)
        .flat_map(|y| (r.min.x as usize..r.max.x as usize).map(move |x| (x, y)))
        .filter(|&(x, y)| img.pixels[y * img.size[0] + x].r() > t.r() - 40)
        .count();
    assert!(lit > 50, "text pixels: {lit}");
    // Library only: not in Develop
    plate(&mut h, json!({"show": ["library"]}));
    let r = h.request("engine.execute", json!({"command": "view.develop"}), SETTLE);
    assert_eq!(r["ok"], true);
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_none(), "not shown in Develop");
    plate(&mut h, json!({"show": "all"}));
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_some(), "shown in Develop again");
}

/// Bad input: a corrupt or missing file, a bad tint or parameter is an error that changes
/// nothing; a file that breaks after it was set becomes a notice, never a panic.
#[test]
fn bad_plates_are_errors_not_crashes() {
    let d = tmp("bad");
    let bad = d.join("bad.png");
    std::fs::write(&bad, b"\x89PNG\r\n\x1a\ngarbage").unwrap();
    let mut h = demo();
    let before = h.app.ui.settings.identity_plate.clone();
    for p in [
        json!({"image": bad.to_string_lossy()}),
        json!({"image": d.join("missing.png").to_string_lossy()}),
        json!({"tint": "#zzzzzz"}),
        json!({"place": "middle"}),
        json!({"sparkle": true}),
    ] {
        let r = h.request("engine.execute", json!({"command": "app.identityPlate", "params": p}), SETTLE);
        assert_eq!(r["ok"], false, "{p}: {r}");
    }
    assert_eq!(h.app.ui.settings.identity_plate, before);
    // set a good file, then corrupt it on disk: a notice, no plate, no panic
    let good = d.join("good.png");
    std::fs::write(&good, ring_png(64, 64)).unwrap();
    plate(&mut h, json!({"image": good.to_string_lossy(), "on": true}));
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_some());
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&good, b"broken now, and a different size").unwrap();
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_none());
    let state = plate(&mut h, json!({}));
    assert!(state["error"].as_str().is_some_and(|e| !e.is_empty()), "{state}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The plate is copied into the identity folder (the original may move); a file already there
/// is used in place; the settings survive a restart (ui.json round trip).
#[test]
fn plate_is_copied_and_persists() {
    let d = tmp("persist");
    let src = d.join("logo.png");
    std::fs::write(&src, ring_png(128, 64)).unwrap();
    let mut h = demo();
    h.app.identity_dir = Some(d.join("identity"));
    let r = plate(
        &mut h,
        json!({"image": src.to_string_lossy(), "on": true, "place": "tr", "tint": "#D8C8A8", "size": 40, "margin": 20, "opacity": 0.6}),
    );
    let copied = d.join("identity").join("logo.png");
    assert_eq!(r["image"], json!(copied.to_string_lossy()));
    assert_eq!(r["tint"], "#d8c8a8");
    std::fs::remove_file(&src).unwrap();
    assert!(h.settle(SETTLE));
    assert!(h.app.identity.rect.is_some(), "drawn from the copy after the original went");
    // pointing at the copy: no second copy
    plate(&mut h, json!({"image": copied.to_string_lossy()}));
    assert_eq!(std::fs::read_dir(d.join("identity")).unwrap().count(), 1);
    // restart: the saved UI state brings the plate back
    let saved = serde_json::to_vec_pretty(&h.app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_slice(&saved).unwrap();
    assert_eq!(back.settings.identity_plate, h.app.ui.settings.identity_plate);
    assert!(back.settings.identity_plate.on && back.settings.identity_plate.place == Place::Tr);
    // an old ui.json without the key: the plate is off
    let mut v: Value = serde_json::from_slice(&saved).unwrap();
    v["settings"].as_object_mut().unwrap().remove("identityPlate");
    let old: crate::state::UiState = serde_json::from_value(v).unwrap();
    assert!(!old.settings.identity_plate.on);
    let _ = std::fs::remove_dir_all(&d);
}

/// The Settings page: identity. shows the controls and a live preview with the plate in it.
#[test]
fn settings_page_previews_the_plate() {
    let d = tmp("page");
    let file = d.join("ring.png");
    std::fs::write(&file, ring_png(128, 128)).unwrap();
    let mut h = demo();
    plate(&mut h, json!({"image": file.to_string_lossy(), "on": true, "place": "canvas.br", "tint": "#ff2000", "size": 120, "opacity": 1.0}));
    let r = h.request("engine.execute", json!({"command": "app.identityPlateSetup"}), SETTLE);
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.settle(SETTLE));
    for id in [
        "check:identityOn",
        "button:identityPick-0",
        "button:identityPlace-0",
        "wheel:identityTint",
        "preview:identityPlate",
        "button:identityTintNone",
    ] {
        let ok = h.app.widgets.iter().any(|(w, _)| w == id || w.starts_with(id.trim_end_matches("-0")));
        assert!(ok, "{id} on the page");
    }
    let preview = h.app.widgets.iter().find(|(w, _)| w == "preview:identityPlate").unwrap().1;
    let img = h.paint();
    let red = (preview.min.y as usize..preview.max.y as usize)
        .flat_map(|y| (preview.min.x as usize..preview.max.x as usize).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = img.pixels[y * img.size[0] + x];
            p.r() > 150 && p.g() < 80
        })
        .count();
    assert!(red > 20, "the plate in the preview ({red} px)");
    // a swatch click sets the tint; none. takes it off
    let r = h.request("ui.clickWidget", json!({"id": "swatch:identity-tan"}), SETTLE);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.settings.identity_plate.tint.as_deref(), Some("#d8c8a8"));
    let r = h.request("ui.clickWidget", json!({"id": "button:identityTintNone"}), SETTLE);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.settings.identity_plate.tint, None);
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_SNAPSHOT_DIR").map(PathBuf::from) {
        let r = h.request("ui.clickWidget", json!({"id": "swatch:identity-tan"}), SETTLE);
        assert_eq!(r["ok"], true);
        assert!(h.settle(SETTLE));
        save_png(&h.paint(), &dir.join("identity-settings.png"));
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// Launch: with the fonts and theme prepared before the first frame (what the desktop app does),
/// frame one already draws the app — the window, which stays hidden until a frame is drawn,
/// never opens on a blank grey box. Without `prepare` it is the app's dark chrome, never empty.
#[test]
fn first_frame_draws_the_app() {
    let mut h = demo();
    let ctx = h.view.ctx.clone();
    h.app.prepare(&ctx);
    h.step();
    assert!(h.app.widgets.iter().any(|(w, _)| w == "module:library"), "frame 1 drew the top bar");
    let img = h.paint();
    let mid = img.pixels[(img.size[1] / 2) * img.size[0] + img.size[0] / 2];
    assert_ne!(mid, Color32::from_rgb(12, 12, 12), "frame 1 is not the empty clear colour");

    let mut cold = demo();
    cold.step();
    let img = cold.paint();
    let chrome = crate::theme::Tokens::default().chrome;
    assert_eq!(img.pixels[(img.size[1] / 2) * img.size[0] + img.size[0] / 2], chrome, "an unprepared first frame is the dark chrome");
}
