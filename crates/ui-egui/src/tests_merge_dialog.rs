//! Headless tests of the Panorama Merge Preview dialog: centred on the screen at any size, its own
//! window, the crop frame drawn over the preview and dragged with the mouse (corner, edge, inside),
//! and the aspect. menu that locks it to a shape.

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::merge::MergeDialog;
use crate::tests_panels::layout_faults;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);
/// The preview picture the dialog is given (a wide panorama).
const PIC: [usize; 2] = [1024, 400];

fn demo(size: [f32; 2]) -> Headless {
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    let mut h = Headless::new(app, size, 1.0);
    h.request("ui.set", json!({"view": "photoGrid"}), T);
    h.settle(SETTLE);
    h
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().rev().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let p = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

/// A test panorama: a sky-to-ground ramp with a grid, so the crop frame and the dimming show.
fn picture(ctx: &egui::Context) -> egui::TextureHandle {
    let [w, h] = PIC;
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let line = x % 64 < 2 || y % 64 < 2;
            let (r, g, b) = ((60 + x * 120 / w) as u8, (90 + y * 120 / h) as u8, (200 - y * 120 / h) as u8);
            px.push(if line { egui::Color32::from_gray(230) } else { egui::Color32::from_rgb(r, g, b) });
        }
    }
    ctx.load_texture("t-merge-dialog", egui::ColorImage::new([w, h], px), egui::TextureOptions::LINEAR)
}

/// Open Panorama Merge Preview with a finished preview whose auto-crop frame is `crop` (x, y, w, h).
fn open(h: &mut Headless, crop: [f64; 4]) {
    let ids = h.app.session.visible_cloned();
    h.app.merge.ids = ids.iter().take(3).copied().collect();
    let opts = MergeDialog::for_command("merge.panorama");
    h.app.ui.dialog = Some(crate::state::Dialog::Merge { opts: opts.clone() });
    let info = json!({"used": [1, 2, 3], "projection": "cylindrical", "fov": [96.0, 38.0], "crop": crop});
    h.app.merge.preview = Some((picture(&h.view.ctx), PIC, opts, info));
    h.app.merge.error = None;
    h.app.merge.failed_options = None;
    h.settle(SETTLE);
}

fn opts(h: &Headless) -> MergeDialog {
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => opts.clone(),
        other => panic!("the merge dialog is not open: {}", other.is_some()),
    }
}

fn drag(h: &mut Headless, a: egui::Pos2, b: egui::Pos2, steps: u64) {
    let r = h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": b.x, "toY": b.y, "steps": steps}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
}

fn click(h: &mut Headless, id: &str) {
    let c = widget(h, id).center();
    let r = h.request("ui.click", json!({"x": c.x, "y": c.y}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
}

/// The frame as `[x0, y0, x1, y1]`: what the dialog shows now.
fn frame(h: &Headless) -> [f64; 4] {
    let info = h.app.merge.preview.as_ref().map_or(Value::Null, |p| p.3.clone());
    crate::merge::crop_frame(&opts(h), &info)
}

fn pixel_ratio(c: [f64; 4]) -> f64 {
    ((c[2] - c[0]) * PIC[0] as f64) / ((c[3] - c[1]) * PIC[1] as f64)
}

/// The window is centred on the screen and inside it (the title in the middle of it), whatever the
/// screen's size, and nothing in it is clipped or touching.
#[test]
fn the_merge_window_is_centred_and_fits_the_screen() {
    for size in [[1400.0, 900.0], [2000.0, 1000.0]] {
        let mut h = demo(size);
        open(&mut h, [0.05, 0.1, 0.9, 0.8]);
        let screen = h.view.ctx.content_rect();
        let win = widget(&h, "dialog:window");
        assert!((win.center().x - screen.center().x).abs() <= 2.0 && (win.center().y - screen.center().y).abs() <= 2.0, "{size:?}: {win:?} on {screen:?}");
        assert!(screen.contains_rect(win), "{size:?}: {win:?} inside {screen:?}");
        // the title: drawn once, in the middle of the window
        let title = crate::merge::MergeDialog::for_command("merge.panorama").title();
        let texts = h.view.visible_texts();
        let (_, tr, _) = texts.iter().find(|(s, _, _)| s == title).unwrap_or_else(|| panic!("no title {title:?}"));
        assert!((tr.center().x - win.center().x).abs() <= 2.0, "{size:?}: title {tr:?} in {win:?}");
        // the preview and the options fill the window exactly (no slack on the right)
        let (pre, cancel, ok) = (widget(&h, "merge.preview"), widget(&h, "button:dialogCancel"), widget(&h, "button:dialogOk"));
        assert!(pre.width() >= 360.0 && pre.right() < win.right() - 16.0, "{pre:?} in {win:?}");
        assert!(pre.left() - win.left() >= 16.0 - 1.0, "left margin {pre:?} {win:?}");
        assert!(ok.left() - cancel.right() >= 8.0 - 0.5, "cancel. and merge. 8 px apart: {cancel:?} {ok:?}");
        assert!((ok.height() - cancel.height()).abs() < 0.5 && (ok.center().y - cancel.center().y).abs() < 0.5, "one row, one height");
        assert!(ok.bottom() <= win.bottom() - 8.0, "{ok:?} in {win:?}");
        // the checkboxes and the warp. slider share one left edge
        let (warp, fill, auto) = (widget(&h, "sliderLabel:merge.boundaryWarp"), widget(&h, "check:merge.fillEdges"), widget(&h, "check:merge.autoCrop"));
        assert!((warp.left() - fill.left()).abs() <= 1.0 && (fill.left() - auto.left()).abs() <= 0.5, "left edges: {warp:?} {fill:?} {auto:?}");
        assert!(warp.left() >= pre.right() + 12.0 - 1.0 && warp.right() <= win.right() - 16.0 + 1.0, "inside the options column: {warp:?} {pre:?} {win:?}");
        let faults = layout_faults(&h, win);
        assert!(faults.is_empty(), "{size:?}: {}", faults.join("\n"));
        shot(&mut h, &format!("merge-dialog-{}x{}.png", size[0], size[1]));
    }
}

/// A narrow screen: the preview shrinks (not under 360) and the window stays on the screen.
#[test]
fn on_a_small_screen_the_preview_shrinks_to_fit() {
    let mut h = demo([800.0, 700.0]);
    open(&mut h, [0.05, 0.1, 0.9, 0.8]);
    let (screen, win, pre) = (h.view.ctx.content_rect(), widget(&h, "dialog:window"), widget(&h, "merge.preview"));
    assert!(screen.contains_rect(win), "{win:?} on {screen:?}");
    assert!(pre.width() >= 360.0 && pre.width() < 520.0, "{pre:?}");
    assert!((win.center().x - screen.center().x).abs() <= 2.0, "{win:?}");
}

/// Dragging the preview's bottom crop edge up moves only the bottom of the stored frame; it is
/// the auto-crop frame until then.
#[test]
fn dragging_the_bottom_crop_edge_changes_only_y1() {
    let mut h = demo([2000.0, 1000.0]);
    open(&mut h, [0.05, 0.1, 0.85, 0.7]);
    let before = frame(&h);
    assert!(opts(&h).crop.is_none(), "untouched: the auto-crop frame");
    assert!((before[0] - 0.05).abs() < 1e-9 && (before[3] - 0.8).abs() < 1e-9, "{before:?}");
    // anywhere along the edge, not only on its tick
    for (along, steps) in [(0.5, 12), (0.3, 1)] {
        let (bl, br) = (widget(&h, "mergeCropHandle:3").center(), widget(&h, "mergeCropHandle:2").center());
        let from = frame(&h);
        let a = bl + (br - bl) * along;
        drag(&mut h, a, a - egui::vec2(0.0, 30.0), steps);
        let c = opts(&h).crop.expect("the user's frame is stored");
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(near(c[0], from[0]) && near(c[1], from[1]) && near(c[2], from[2]), "only y1 changes: {from:?} -> {c:?}");
        assert!(c[3] < from[3] - 0.02, "the bottom went up: {from:?} -> {c:?}");
        let (bl2, br2) = (widget(&h, "mergeCropHandle:3").center(), widget(&h, "mergeCropHandle:2").center());
        assert!((bl2.x - bl.x).abs() < 0.5 && (br2.x - br.x).abs() < 0.5 && bl2.y < bl.y - 5.0, "x edges stayed on screen, the bottom went up");
    }
    // the preview did not pan: still the whole picture
    assert_eq!(h.app.merge.view, crate::pan_zoom::PanZoom::default());
    // a new preview of other options keeps it; auto.crop. again goes back to the computed frame
    shot(&mut h, "merge-dialog-crop-dragged.png");
    let kept = opts(&h).crop;
    if let Some(crate::state::Dialog::Merge { opts }) = h.app.ui.dialog.as_mut() {
        opts.fill_edges = true;
    }
    h.settle(SETTLE);
    assert_eq!(opts(&h).crop, kept, "the user's frame survives a changed option");
    click(&mut h, "check:merge.autoCrop");
    click(&mut h, "check:merge.autoCrop");
    assert!(opts(&h).crop.is_none(), "auto.crop. resets the frame");
    assert!(opts(&h).auto_crop);
}

/// Dragging inside the frame moves it (keeping its size); dragging the picture outside it pans.
#[test]
fn dragging_inside_the_frame_moves_it_and_outside_pans_the_zoomed_preview() {
    let mut h = demo([2000.0, 1000.0]);
    open(&mut h, [0.2, 0.2, 0.4, 0.5]);
    let before = frame(&h);
    let pre = widget(&h, "merge.preview");
    let c = egui::pos2(pre.left() + pre.width() * 0.4, pre.top() + pre.height() * 0.45);
    drag(&mut h, c, c + egui::vec2(40.0, 10.0), 8);
    let after = frame(&h);
    assert!(((after[2] - after[0]) - (before[2] - before[0])).abs() < 1e-9 && after[0] > before[0] + 0.02, "{before:?} -> {after:?}");
    assert_eq!(h.app.merge.view, crate::pan_zoom::PanZoom::default(), "no pan");
    // zoomed in, a drag that starts off the frame pans
    h.request("ui.move", json!({"x": pre.center().x, "y": pre.center().y}), T);
    h.request("ui.scroll", json!({"dy": 240.0}), T);
    h.settle(SETTLE);
    let z = h.app.merge.view;
    assert!(z.zoom > 1.5, "{z:?}");
    let f = opts(&h).crop;
    let off = pre.right_bottom() - egui::vec2(20.0, 20.0);
    drag(&mut h, off, off - egui::vec2(60.0, 0.0), 8);
    assert!(h.app.merge.view.center.x > z.center.x + 0.001, "panned: {:?} -> {:?}", z, h.app.merge.view);
    assert_eq!(opts(&h).crop, f, "the frame did not move");
}

/// The aspect. menu: 16:9 fits a 16:9 frame in the current one, then a corner drag keeps it.
#[test]
fn picking_16_9_gives_that_ratio_and_a_corner_drag_keeps_it() {
    let mut h = demo([2000.0, 1000.0]);
    open(&mut h, [0.05, 0.1, 0.85, 0.7]);
    let start = frame(&h);
    click(&mut h, "button:mergeAspect");
    for key in crate::merge::ASPECTS {
        let _ = widget(&h, &format!("button:mergeAspect-{key}"));
    }
    click(&mut h, "button:mergeAspect-16:9");
    let c = opts(&h).crop.expect("a frame was fitted");
    assert!((pixel_ratio(c) - 16.0 / 9.0).abs() < 1e-3, "{c:?} {}", pixel_ratio(c));
    assert!(c[0] >= start[0] - 1e-9 && c[1] >= start[1] - 1e-9 && c[2] <= start[2] + 1e-9 && c[3] <= start[3] + 1e-9, "inside the old frame: {start:?} -> {c:?}");
    assert!(((c[0] + c[2]) / 2.0 - (start[0] + start[2]) / 2.0).abs() < 1e-9, "centred on it");
    assert_eq!(h.app.merge.crop_aspect, Some("16:9"));
    // a corner drag keeps the shape (bottom-right in, then top-left out)
    let br = widget(&h, "mergeCropHandle:2").center();
    drag(&mut h, br, br - egui::vec2(40.0, 5.0), 10);
    let d = opts(&h).crop.expect("frame");
    assert!((pixel_ratio(d) - 16.0 / 9.0).abs() < 1e-3, "{d:?} {}", pixel_ratio(d));
    assert!(d[2] - d[0] < c[2] - c[0] - 0.01, "it shrank: {c:?} -> {d:?}");
    assert!((d[0] - c[0]).abs() < 1e-9 && (d[1] - c[1]).abs() < 1e-9, "the opposite corner stayed");
    // original. is the picture's own shape; free. lets go
    click(&mut h, "button:mergeAspect");
    click(&mut h, "button:mergeAspect-original.");
    let o = opts(&h).crop.expect("frame");
    assert!((pixel_ratio(o) - PIC[0] as f64 / PIC[1] as f64).abs() < 1e-3, "{o:?}");
    click(&mut h, "button:mergeAspect");
    click(&mut h, "button:mergeAspect-free.");
    assert_eq!(h.app.merge.crop_aspect, None);
    // 16:9 once more, for the screenshot
    click(&mut h, "button:mergeAspect");
    click(&mut h, "button:mergeAspect-16:9");
    let faults = layout_faults(&h, widget(&h, "dialog:window"));
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    shot(&mut h, "merge-dialog-16x9.png");
}

/// The crop frame is drawn over the preview in Bryan's crop green, and the preview's own pixels
/// carry no frame any more.
#[test]
fn the_frame_is_drawn_in_crop_green_over_the_preview() {
    let mut h = demo([2000.0, 1000.0]);
    open(&mut h, [0.1, 0.2, 0.7, 0.6]);
    let img = h.snapshot(T);
    let tl = widget(&h, "mergeCropHandle:1").center();
    // the top edge between its middle tick and the corner: one row of the frame colour
    let (x, y) = (((tl.x + widget(&h, "mergeCropHandle:4").center().x) / 2.0).round() as usize, tl.y.round() as usize);
    let hit = (y.saturating_sub(2)..=y + 2).any(|yy| {
        let p = img.pixels[yy * img.width() + x];
        (i32::from(p.r()) - 170).abs() < 40 && (i32::from(p.g()) - 204).abs() < 40 && p.b() < 80
    });
    assert!(hit, "crop green at {x},{y}");
}

/// The checkbox rows (fill.edges., auto.crop., auto.settings., stack.) are one even column: the
/// same gap between every pair, at any screen size.
#[test]
fn the_checkbox_rows_are_evenly_spaced() {
    for size in [[1400.0, 900.0], [2000.0, 1000.0]] {
        let mut h = demo(size);
        open(&mut h, [0.05, 0.1, 0.9, 0.8]);
        let tops: Vec<f32> = ["merge.fillEdges", "merge.autoCrop", "merge.autoSettings", "merge.stack"].iter().map(|id| widget(&h, &format!("check:{id}")).top()).collect();
        let gaps: Vec<f32> = tops.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| *g > 0.0), "{size:?}: in order {tops:?}");
        assert!(gaps.iter().all(|g| (g - gaps[0]).abs() <= 1.0), "{size:?}: row pitch {gaps:?}");
    }
}

/// Merge with Last Settings must not reuse the crop frame dragged for other photos.
#[test]
fn the_last_settings_do_not_keep_the_crop_frame() {
    let mut h = demo([1400.0, 900.0]);
    open(&mut h, [0.05, 0.1, 0.9, 0.8]);
    let mut o = opts(&h);
    o.crop = Some([0.1, 0.2, 0.8, 0.7]);
    crate::merge::remember_last(&mut h.app, &o);
    let last = h.app.merge.last.get("merge.panorama").expect("remembered");
    assert_eq!(last.crop, None, "the crop frame is not a setting");
    assert_eq!(last.auto_crop, o.auto_crop, "the rest is");
}
