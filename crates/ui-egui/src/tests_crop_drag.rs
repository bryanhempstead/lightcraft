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
    // the set-up is its own step: what a test does next is a fresh crop session
    exec(&mut h, "develop.endRun", json!({}));
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

fn frame_centre(h: &Headless) -> egui::Pos2 {
    (widget(h, "cropHandle:0").center() + widget(h, "cropHandle:2").center().to_vec2()) * 0.5
}

fn geometry(h: &Headless) -> lightcraft_geom::CropGeometry {
    let id = h.app.session.active().unwrap();
    h.app.session.develop_of(id).unwrap().crop.geometry
}

fn drag_with(h: &mut Headless, a: egui::Pos2, b: egui::Pos2, shift: bool) {
    let r = h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": b.x, "toY": b.y, "steps": 12, "shift": shift}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(T);
}

/// Bryan, 2026-11-24: a plain drag inside the frame pans the photo under it, straight: the crop only
/// translates (the photo follows the pointer, the frame goes the other way), never turning or
/// resizing, in any direction and on a straightened crop too.
#[test]
fn dragging_inside_the_crop_pans_the_photo_straight() {
    for angle in [0.0, 4.0] {
        for (dx, dy) in [(40.0, 0.0), (0.0, 30.0), (-35.0, -25.0), (30.0, 22.0)] {
            let mut h = crop_tool(false);
            exec(&mut h, "crop.set", json!({"rect": [0.35, 0.35, 0.65, 0.65], "angle": angle}));
            h.step();
            let before = geometry(&h);
            let c = frame_centre(&h);
            drag_with(&mut h, c, c + egui::vec2(dx, dy), false);
            let after = geometry(&h);
            assert_eq!(after.angle, before.angle, "{angle}: {dx},{dy}: never turns");
            assert!((after.rect.width() - before.rect.width()).abs() < 1e-9 && (after.rect.height() - before.rect.height()).abs() < 1e-9);
            let (mx, my) = (after.rect.center().x - before.rect.center().x, after.rect.center().y - before.rect.center().y);
            // the photo went with the pointer, so the frame went the other way
            assert!(mx * dx as f64 <= 1e-9 && my * dy as f64 <= 1e-9, "{angle}: {dx},{dy}: moved {mx},{my}");
            assert!(mx.hypot(my) > 0.01, "{angle}: {dx},{dy}: it moved");
        }
    }
}

/// Shift in the crop area: the frame itself follows the pointer, from inside it or from an edge (which
/// would otherwise resize), size and angle untouched, with a ratio locked too.
#[test]
fn shift_drag_moves_the_whole_frame_freely() {
    for lock in [false, true] {
        for from_edge in [false, true] {
            let mut h = crop_tool(false);
            if lock {
                exec(&mut h, "crop.aspect", json!({"aspect": "current"}));
                h.step();
            }
            let before = geometry(&h);
            let start = if from_edge {
                let (bl, br) = (widget(&h, "cropHandle:3").center(), widget(&h, "cropHandle:2").center());
                (bl + br.to_vec2()) * 0.5
            } else {
                frame_centre(&h)
            };
            drag_with(&mut h, start, start + egui::vec2(35.0, 20.0), true);
            let after = geometry(&h);
            assert!(
                (after.rect.width() - before.rect.width()).abs() < 1e-9 && (after.rect.height() - before.rect.height()).abs() < 1e-9,
                "lock {lock} edge {from_edge}: resized"
            );
            assert!(after.rect.x0 > before.rect.x0 + 0.01 && after.rect.y0 > before.rect.y0 + 0.01, "lock {lock} edge {from_edge}: {before:?} -> {after:?}");
            assert_eq!(after.angle, before.angle);
        }
    }
}

/// Bryan, 2026-11-24: "if i crop and dont punch in it wont let me move the crop around to the cut
/// off areas". A crop at its largest size (touching the photo on one axis) moves along the other,
/// by the pan, the shift move and the nudge dial, all the way to each edge and no further.
#[test]
fn a_largest_crop_slides_to_each_edge() {
    for how in ["pan", "shift", "nudge"] {
        let mut h = crop_tool(false);
        exec(&mut h, "crop.resetToAsShot", json!({}));
        exec(&mut h, "crop.aspect", json!({"aspect": "4x5"}));
        exec(&mut h, "crop.aspect", json!({"aspect": "1x1"}));
        h.step();
        let start = geometry(&h);
        // the axis with room to spare (the demo photo is portrait: 1x1 has room up and down)
        let vertical = start.rect.width() > 0.99;
        let spare = if vertical { 1.0 - start.rect.height() } else { 1.0 - start.rect.width() };
        assert!(spare > 0.05, "{how}: {start:?}");
        for sign in [-1.0f32, 1.0] {
            match how {
                "nudge" => {
                    for _ in 0..80 {
                        let k = if vertical { "y" } else { "x" };
                        exec(&mut h, "crop.nudge", json!({k: sign * 2.0}));
                    }
                }
                _ => {
                    // from the frame's centre, far past the photo's edge, in two goes
                    for _ in 0..2 {
                        let c = frame_centre(&h);
                        let towards = if how == "pan" { -sign } else { sign };
                        let v = if vertical { egui::vec2(0.0, towards * 600.0) } else { egui::vec2(towards * 600.0, 0.0) };
                        drag_with(&mut h, c, c + v, how == "shift");
                    }
                }
            }
            let g = geometry(&h);
            let (lo, hi) = if vertical { (g.rect.y0, g.rect.y1) } else { (g.rect.x0, g.rect.x1) };
            let edge = if sign < 0.0 { lo } else { 1.0 - hi };
            assert!(edge.abs() < 1e-6, "{how} {sign}: stopped {edge} short of the edge: {g:?}");
            assert!((g.rect.height() - start.rect.height()).abs() < 1e-9 && (g.rect.width() - start.rect.width()).abs() < 1e-9, "{how}: size kept");
        }
    }
}

/// Bryan, 2026-11-24: "my buttons and dials that deal with crop and angle are being finicky and
/// jumping around". Twenty single detents of each crop dial in a row (his driver sends one stock
/// step per detent) are twenty equal steps: X and Y slide the centre by one amount each (one axis
/// only), scale zooms by one factor, and the angle turns by one amount while the crop keeps its
/// centre. In the Crop tool and out of it, on a straight and a straightened photo.
#[test]
fn twenty_detents_of_each_crop_dial_are_twenty_equal_steps() {
    let steps = |h: &mut Headless, input: &str, f: &dyn Fn(&lightcraft_geom::CropGeometry) -> f64| -> Vec<f64> {
        let mut seen = vec![f(&geometry(h))];
        for _ in 0..20 {
            let r = crate::keymap::lr_input(&mut h.app, input);
            assert!(r.is_ok(), "{input}: {r:?}");
            h.step();
            seen.push(f(&geometry(h)));
        }
        seen.windows(2).map(|w| w[1] - w[0]).collect()
    };
    let even = |d: &[f64], what: &str| {
        let first = d[0];
        assert!(first.abs() > 1e-6, "{what}: the first detent moved nothing: {d:?}");
        for (i, s) in d.iter().enumerate() {
            assert!((s - first).abs() <= first.abs() * 0.02 + 1e-9, "{what}: detent {i} moved {s}, the first {first}: {d:?}");
        }
    };
    for tool in [true, false] {
        for angle in [0.0, 3.0] {
            let mut h = crop_tool(false);
            if !tool {
                exec(&mut h, "panel.edit", json!({}));
            }
            // a punched-in crop with room all round
            exec(&mut h, "crop.set", json!({"rect": [0.4, 0.4, 0.6, 0.6], "angle": angle}));
            h.step();
            let tag = format!("tool {tool}, angle {angle}");
            // X: the centre moves along the frame's x only (straightened: the frame's own axes)
            let x = steps(&mut h, "CropPositionX = ^1,0,100", &|g| g.rect.center().x);
            even(&x, &format!("{tag}: X"));
            let y = steps(&mut h, "CropPositionY = ^1,0,100", &|g| g.rect.center().y);
            even(&y, &format!("{tag}: Y"));
            let w0 = geometry(&h).rect.width();
            // scale: the width changes by one factor each time (compare in log)
            let sc = steps(&mut h, "CropScale = ^1,0,100", &|g| g.rect.width().ln());
            even(&sc, &format!("{tag}: scale"));
            assert!(geometry(&h).rect.width() < w0, "{tag}: ^ zooms in");
            // angle: only the angle turns; the centre and the size stay while the crop fits
            let c0 = geometry(&h);
            let an = steps(&mut h, "CropAngle = ^1,0,100", &|g| g.angle);
            even(&an, &format!("{tag}: angle"));
            let c1 = geometry(&h);
            assert!((c1.rect.center().x - c0.rect.center().x).abs() < 1e-9 && (c1.rect.center().y - c0.rect.center().y).abs() < 1e-9, "{tag}: the angle moved the centre");
            assert!((c1.rect.width() - c0.rect.width()).abs() < 1e-9, "{tag}: the angle resized a crop that fits");
        }
    }
    // a full-size crop: the angle shrinks it a little each detent, evenly, about the same centre
    let mut h = crop_tool(false);
    exec(&mut h, "crop.resetToAsShot", json!({}));
    h.step();
    let widths = steps(&mut h, "CropAngle = ^1,0,100", &|g| g.rect.width());
    assert!(widths.iter().all(|d| *d <= 0.0), "{widths:?}");
    let spread = widths.iter().cloned().fold(f64::MIN, f64::max) - widths.iter().cloned().fold(f64::MAX, f64::min);
    assert!(spread < 0.01, "even shrink, no jump: {widths:?}");
    let g = geometry(&h);
    assert!((g.rect.center().x - 0.5).abs() < 1e-6 && (g.rect.center().y - 0.5).abs() < 1e-6);
    // and back: the same angle again gives back the size
    for _ in 0..20 {
        let _ = crate::keymap::lr_input(&mut h.app, "CropAngle = v1,0,100");
        h.step();
    }
    assert!((geometry(&h).rect.width() - 1.0).abs() < 1e-6, "{:?}", geometry(&h));
}

/// Move the wheel burst's last notch back in time, past `CROP_WHEEL_HOLD` (an injected clock: no sleeping).
fn age_wheel(h: &mut Headless) {
    if let Some(t) = h.app.crop_wheel {
        h.app.crop_wheel = Some(t - 10.0);
    }
}

fn history(h: &Headless) -> usize {
    let id = h.app.session.active().unwrap();
    h.app.session.catalog.photo(id).map(|p| p.history.len()).unwrap_or(0)
}

/// A drag that starts while a wheel burst is still open must not have the wheel's timer end the
/// drag's interaction early (each later drag move would be a commit of its own). The wheel burst, the
/// drag and everything else in the Crop tool are ONE undo step (Bryan, 2026-11-24).
#[test]
fn a_wheel_burst_and_the_drag_after_it_stay_in_one_session() {
    let mut h = crop_tool(false);
    let h0 = history(&h);
    let at = frame_centre(&h);
    scroll_at(&mut h, at, 2.0);
    assert!(h.app.crop_wheel.is_some() && h.app.session.interaction.is_some());
    // the hold runs out right as the drag begins
    age_wheel(&mut h);
    let before = geometry(&h);
    let c = frame_centre(&h);
    drag_with(&mut h, c, c + egui::vec2(40.0, 30.0), false);
    assert!(h.app.session.interaction.is_none() && h.app.crop_wheel.is_none());
    assert_ne!(geometry(&h).rect.center(), before.rect.center(), "the drag moved the crop");
    assert_eq!(history(&h), h0 + 1, "the wheel burst and the drag are one step");
    // another burst, then leaving the Crop tool: closed at once, still that one step
    scroll_at(&mut h, at, 2.0);
    assert!(h.app.crop_wheel.is_some());
    exec(&mut h, "panel.edit", json!({}));
    h.step();
    assert!(h.app.crop_wheel.is_none() && h.app.session.interaction.is_none());
    assert_eq!(history(&h), h0 + 1);
}

/// Bryan, 2026-11-24: "if i crop and turn it a little then stop then turn it more and then hit enter
/// have those just be one action". Drag, pause, angle dial, pause, wheel, Return = one undo step that
/// undoes to the crop from before; Esc cancels the whole session instead.
#[test]
fn a_crop_session_in_the_tool_is_one_undo_step_and_escape_cancels_it() {
    let mut h = crop_tool(false);
    let (h0, n0) = (history(&h), h.app.session.undo.len());
    let original = geometry(&h);
    let c = frame_centre(&h);
    drag_with(&mut h, c, c + egui::vec2(30.0, 20.0), false);
    // the angle dial (the console), a pause long enough for it to settle, and again
    for _ in 0..2 {
        crate::keymap::nudge(&mut h.app, "crop.angle", 1.0).unwrap();
        h.settle(T);
        for _ in 0..60 {
            h.step();
        }
    }
    let at = frame_centre(&h);
    scroll_at(&mut h, at, 2.0);
    age_wheel(&mut h);
    h.step();
    assert!(geometry(&h).angle != 0.0 && geometry(&h) != original);
    h.request("ui.key", json!({"key": "Enter"}), T);
    h.settle(T);
    h.step();
    assert_eq!(history(&h), h0 + 1, "one History entry");
    assert_eq!(h.app.session.undo.len(), n0 + 1, "one undo entry");
    assert_eq!(h.app.session.undo.last().unwrap().label, "Crop");
    exec(&mut h, "edit.undo", json!({}));
    assert_eq!(geometry(&h), original, "one undo returns the crop from before the session");
    exec(&mut h, "edit.redo", json!({}));

    // Esc in a new session: everything since entering the tool is reverted, the tool closes
    exec(&mut h, "panel.crop", json!({}));
    h.step();
    let kept = geometry(&h);
    let n1 = h.app.session.undo.len();
    let c = frame_centre(&h);
    drag_with(&mut h, c, c + egui::vec2(-30.0, 10.0), false);
    crate::keymap::nudge(&mut h.app, "crop.angle", -2.0).unwrap();
    h.settle(T);
    assert_ne!(geometry(&h), kept);
    h.request("ui.key", json!({"key": "Escape"}), T);
    h.settle(T);
    h.step();
    assert_eq!(geometry(&h), kept, "Esc reverts the whole session");
    assert_eq!(h.app.session.undo.len(), n1, "and leaves no undo step");
    assert_eq!(h.app.ui.right, crate::state::RightPanel::Edit, "and closes the tool");
}

/// The console's "As Shot" cycle item is the true reset (the whole frame); the default aspect keeps
/// the punch-in. A full-height crop panned to the left edge keeps its place through a ratio cycle.
#[test]
fn the_console_as_shot_resets_and_a_panned_crop_keeps_its_edge() {
    let mut h = crop_tool(false);
    exec(&mut h, "crop.set", json!({"rect": [0.3, 0.3, 0.5, 0.5]}));
    let r = crate::keymap::cycle(&mut h.app, "cropAspect", None, Some(0.0));
    assert!(r.is_ok(), "{r:?}");
    h.settle(T);
    assert!(geometry(&h).is_identity(), "As Shot = the whole frame: {:?}", geometry(&h));
    // a largest-size crop at the edge, through the console's ratio steps
    exec(&mut h, "crop.aspect", json!({"aspect": "1x1"}));
    exec(&mut h, "crop.nudge", json!({"x": -100.0, "y": -100.0}));
    let edge = geometry(&h);
    for _ in 0..4 {
        let r = crate::keymap::cycle(&mut h.app, "cropAspect", Some(1.0), None);
        assert!(r.is_ok(), "{r:?}");
        h.settle(T);
        assert!(geometry(&h).is_within_image(1.0, 1.0));
    }
    exec(&mut h, "crop.aspect", json!({"aspect": "1x1"}));
    let back = geometry(&h);
    assert!((back.rect.x0 - edge.rect.x0).abs() < 1e-9 && (back.rect.y0 - edge.rect.y0).abs() < 1e-9, "{edge:?} -> {back:?}");
}

/// DESIGN.md §15 on the Crop panel's button rows: neighbours ≥ 8 px apart, wrapped lines ≥ 10 px,
/// one control height per row.
#[test]
fn the_crop_panels_button_rows_keep_their_spacing() {
    let h = crop_tool(false);
    let w = |id: &str| widget(&h, &format!("button:{id}"));
    let row = ["cropFlipH", "cropFlipV", "cropSwapAspect", "cropReset", "cropAsShot"].map(w);
    for a in row {
        for b in row {
            if a == b {
                continue;
            }
            let (dx, dy) = ((b.left() - a.right()).max(a.left() - b.right()), (b.top() - a.bottom()).max(a.top() - b.bottom()));
            // on one line the gap is sideways, on two lines it is vertical
            if (a.center().y - b.center().y).abs() < 4.0 {
                assert!(dx >= 7.99, "neighbours {a:?} {b:?}: {dx}");
            } else {
                assert!(dy >= 10.0 - 0.01, "wrapped lines {a:?} {b:?}: {dy}");
            }
        }
    }
    let heights: Vec<f32> = row.iter().map(|r| r.height()).collect();
    assert!(heights.iter().all(|x| (x - heights[0]).abs() < 0.5), "one control height: {heights:?}");
}

fn scroll_at(h: &mut Headless, at: egui::Pos2, dy: f32) {
    h.request("ui.move", json!({"x": at.x, "y": at.y}), T);
    h.request("ui.scroll", json!({"dy": dy, "unit": "line"}), T);
    for _ in 0..30 {
        h.step();
    }
}

/// The wheel in the Crop tool punches the crop in (up) and out (down) about its centre, keeping its
/// shape, and leaves the loupe's own zoom alone; outside the Crop tool the wheel zooms the loupe as it
/// always did.
#[test]
fn the_wheel_punches_the_crop_in_the_crop_tool_only() {
    use crate::state::Zoom;
    let mut h = crop_tool(false);
    let before = geometry(&h);
    let at = h.app.canvas_rect.unwrap().center() + egui::vec2(120.0, 60.0);
    scroll_at(&mut h, at, 3.0);
    let up = geometry(&h);
    assert!(up.rect.width() < before.rect.width() * 0.9, "in: {before:?} -> {up:?}");
    assert!((up.rect.center().x - before.rect.center().x).abs() < 1e-9 && (up.rect.center().y - before.rect.center().y).abs() < 1e-9);
    assert!((up.rect.width() / up.rect.height() - before.rect.width() / before.rect.height()).abs() < 1e-9, "shape kept");
    assert_eq!(h.app.ui.zoom, Zoom::Fit, "the loupe didn't zoom");
    scroll_at(&mut h, at, -6.0);
    let down = geometry(&h);
    assert!(down.rect.width() > up.rect.width() * 1.1, "out: {up:?} -> {down:?}");
    // a burst of notches ends as one undo step
    // (the clock is injected: the burst's last notch is moved back past the hold)
    assert!(h.app.session.interaction.is_some() && h.app.crop_wheel.is_some(), "the burst is open");
    age_wheel(&mut h);
    h.step();
    h.step();
    assert!(h.app.session.interaction.is_none(), "the burst's interaction closed");
    // outside the Crop tool: the wheel is the loupe's, the crop doesn't move
    exec(&mut h, "panel.edit", json!({}));
    h.step();
    let crop_now = geometry(&h);
    let at = h.app.canvas_rect.unwrap().center();
    scroll_at(&mut h, at, 1.0);
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "the wheel zooms the loupe: {:?}", h.app.ui.zoom);
    assert_eq!(geometry(&h), crop_now);
}

/// Inside the frame, away from the edges, a drag moves the crop (the photo under it).
#[test]
fn dragging_inside_the_crop_still_moves_it() {
    let mut h = crop_tool(false);
    let c = frame_centre(&h);
    let before = crop(&h);
    drag(&mut h, c, c + egui::vec2(40.0, 0.0), 12);
    let after = crop(&h);
    assert!(after.x0 < before.x0 - 0.01 && (after.width() - before.width()).abs() < 1e-6, "{before:?} -> {after:?}");
}
