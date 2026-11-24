//! Headless tests of the Enhance dialog's previews (grab hand, drag to pan, wheel / pinch to zoom,
//! Before and After in step) and its ok. / cancel. under the options; the Merge preview pans and
//! zooms the same way.

use std::time::Duration;

use serde_json::json;

use crate::headless::{Headless, HeadlessView};
use crate::tests_panels::layout_faults;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn demo() -> Headless {
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.request("ui.set", json!({"view": "detail", "right": "edit", "module": "develop"}), T);
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

/// A test picture: a colour ramp with a grid, so panning and zooming show.
fn picture(ctx: &egui::Context, name: &str, w: usize, h: usize, warm: bool) -> egui::TextureHandle {
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let line = x % 64 < 2 || y % 64 < 2;
            let (r, g, b) = ((x * 255 / w) as u8, (y * 255 / h) as u8, if warm { 60 } else { 160 });
            px.push(if line { egui::Color32::from_gray(230) } else { egui::Color32::from_rgb(r, g, b) });
        }
    }
    ctx.load_texture(name, egui::ColorImage::new([w, h], px), egui::TextureOptions::LINEAR)
}

/// The cursor while the pointer rests at `at` (one frame).
fn cursor_at(h: &mut Headless, at: egui::Pos2) -> egui::CursorIcon {
    let mut cursor = egui::CursorIcon::Default;
    for _ in 0..2 {
        let raw = HeadlessView::raw_input(h.size, h.pixels_per_point, h.view.ctx.input(|i| i.time) + 0.02, vec![egui::Event::PointerMoved(at)]);
        let app = &mut h.app;
        h.view.run(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
            cursor = ui.ctx().output(|o| o.cursor_icon);
        });
    }
    // the clock never runs backwards for the frames after these
    h.advance_time(0.1);
    cursor
}

/// Open the Enhance dialog on the active photo with a finished preview (a demo photo can't be
/// enhanced, so the preview pictures are made here).
fn open_with_preview(h: &mut Headless) {
    let id = h.app.session.active().or_else(|| h.app.session.visible_cloned().first().copied()).expect("a photo");
    h.app.enhance.ids = vec![id];
    h.app.enhance.view = crate::enhance::initial_view();
    let opts = crate::enhance::EnhanceDialog::default();
    h.app.ui.dialog = Some(crate::state::Dialog::Enhance { opts: opts.clone() });
    let (before, after) = (picture(&h.view.ctx, "t-before", 640, 640, false), picture(&h.view.ctx, "t-after", 640, 640, true));
    h.app.enhance.preview = Some((before, after, [640, 640], opts, json!({"denoiser": "builtin", "seconds": {"denoise": 1.2}})));
    h.app.enhance.error = None;
    h.app.enhance.failed_options = None;
    h.settle(SETTLE);
}

#[test]
fn enhance_previews_pan_and_zoom_together_and_ok_sits_under_the_options() {
    let mut h = demo();
    open_with_preview(&mut h);
    let (before, after) = (widget(&h, "preview:enhanceBefore"), widget(&h, "preview:enhanceAfter"));
    let (ok, cancel, dialog) = (widget(&h, "button:dialogOk"), widget(&h, "button:dialogCancel"), widget(&h, "dialog:window"));
    // ok. and cancel. in the options column, under the options, 8 px apart, inside the window
    let options_bottom = widget(&h, "button:enhanceTarget-dng").bottom();
    assert!(ok.left() > after.right() && cancel.left() > after.right(), "in the right-hand column: {ok:?} {cancel:?} {after:?}");
    assert!(ok.top() > options_bottom && cancel.top() > options_bottom, "under the options");
    assert!(ok.left() - cancel.right() >= 8.0 - 0.5, "cancel. and ok. 8 px apart: {cancel:?} {ok:?}");
    assert!((ok.center().y - cancel.center().y).abs() < 0.5 && (ok.height() - cancel.height()).abs() < 0.5, "one row, one height");
    assert!(ok.right() <= dialog.right() - 8.0 && ok.bottom() <= dialog.bottom() - 8.0, "{ok:?} in {dialog:?}");
    let faults = layout_faults(&h, dialog);
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    // ok. is filled with the accent colour
    let img = h.snapshot(T);
    let at = |p: egui::Pos2| img.pixels[(p.y.round() as usize) * img.width() + p.x.round() as usize];
    let fill = at(ok.left_center() + egui::vec2(3.0, 0.0));
    assert!(fill.b() > 150 && fill.b() > fill.r() + 100, "accent fill, got {fill:?}");
    shot(&mut h, "enhance-dialog.png");
    // a grab hand over either preview
    assert_eq!(cursor_at(&mut h, after.center()), egui::CursorIcon::Grab);
    assert_eq!(cursor_at(&mut h, before.center()), egui::CursorIcon::Grab);
    // opens at 1:1 on the middle
    let v0 = h.app.enhance.view;
    assert!((v0.zoom - 640.0 / 360.0).abs() < 1e-3 && v0.center == egui::vec2(0.5, 0.5), "{v0:?}");
    // drag the After preview 60 pt right: both show more of the left (one shared view)
    let a = after.center();
    h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": a.x + 60.0, "toY": a.y + 30.0, "steps": 10}), T);
    h.settle(SETTLE);
    let v1 = h.app.enhance.view;
    assert!((v1.center.x - (0.5 - 60.0 / 640.0)).abs() < 0.01, "{v1:?}");
    assert!((v1.center.y - (0.5 - 30.0 / 640.0)).abs() < 0.01, "{v1:?}");
    assert_eq!(v1.zoom, v0.zoom, "a drag doesn't zoom");
    // the wheel over Before zooms in around the pointer; a pinch zooms out
    let p = before.center() + egui::vec2(40.0, -30.0);
    h.request("ui.move", json!({"x": p.x, "y": p.y}), T);
    h.settle(SETTLE);
    h.request("ui.scroll", json!({"dy": 120.0}), T);
    h.settle(SETTLE);
    let v2 = h.app.enhance.view;
    assert!(v2.zoom > v1.zoom * 1.2, "{v1:?} → {v2:?}");
    h.request("ui.zoom", json!({"factor": 0.5}), T);
    h.settle(SETTLE);
    assert!(h.app.enhance.view.zoom < v2.zoom * 0.75, "pinch out: {:?}", h.app.enhance.view);
    shot(&mut h, "enhance-dialog-zoomed.png");
    // enhance.view frames it from the control channel; zoom 1 is the whole preview
    let r = h.request("engine.execute", json!({"command": "enhance.view", "params": {"zoom": 1.0, "center": [0.1, 0.9]}}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.enhance.view, crate::pan_zoom::PanZoom::default(), "a whole picture is always centred");
    // Return presses ok. (a demo photo can't be enhanced: the dialog stays open and says why)
    h.request("ui.key", json!({"key": "Enter"}), T);
    h.settle(SETTLE);
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::Enhance { .. })), "stays open on an error");
    assert!(h.app.ui.toast.is_some(), "the reason is shown");
    // cancel. closes; Esc closes
    let c = widget(&h, "button:dialogCancel").center();
    h.request("ui.click", json!({"x": c.x, "y": c.y}), T);
    h.settle(SETTLE);
    assert!(h.app.ui.dialog.is_none(), "cancel. closed it");
    open_with_preview(&mut h);
    h.request("ui.key", json!({"key": "Escape"}), T);
    h.settle(SETTLE);
    assert!(h.app.ui.dialog.is_none(), "Esc closed it");
}

/// The Merge dialog's preview takes the same hand, drag and zoom.
#[test]
fn merge_preview_pans_and_zooms() {
    let mut h = demo();
    let ids = h.app.session.visible_cloned();
    h.app.merge.ids = ids.iter().take(2).copied().collect();
    let opts = crate::merge::MergeDialog::for_command("merge.hdr");
    h.app.ui.dialog = Some(crate::state::Dialog::Merge { opts: opts.clone() });
    h.app.merge.preview = Some((picture(&h.view.ctx, "t-merge", 1040, 720, true), [1040, 720], opts, json!({"ev": [0.0, 2.0]})));
    h.settle(SETTLE);
    let pane = widget(&h, "preview:mergePreview");
    assert_eq!(cursor_at(&mut h, pane.center()), egui::CursorIcon::Grab);
    let c = pane.center();
    h.request("ui.move", json!({"x": c.x, "y": c.y}), T);
    h.request("ui.scroll", json!({"dy": 200.0}), T);
    h.settle(SETTLE);
    let z = h.app.merge.view;
    assert!(z.zoom > 1.5, "{z:?}");
    h.request("ui.drag", json!({"x": c.x, "y": c.y, "toX": c.x - 50.0, "toY": c.y, "steps": 8}), T);
    h.settle(SETTLE);
    assert!(h.app.merge.view.center.x > z.center.x, "dragged left shows more of the right: {z:?} → {:?}", h.app.merge.view);
}

/// denoise.: a raw shot at a high ISO shows ISO · denoise. · hide. in the chosen corner of the
/// preview; denoise. opens Enhance with Denoise ticked; hide. puts it away for that photo; the
/// Settings switch / ISO turn it off.
#[test]
fn high_iso_raw_offers_denoise_in_a_corner() {
    let mut s = lightcraft_engine::Session::with_demo();
    let first = s.visible_cloned()[0];
    let mut p = lightcraft_catalog::Photo::clone(s.catalog.photo(first).unwrap());
    let id = lightcraft_catalog::PhotoId(90_001);
    p.id = id;
    p.kind = lightcraft_catalog::MediaKind::Raw;
    p.meta.iso = Some(6400);
    s.catalog.apply(lightcraft_catalog::Op::AddPhoto { photo: Box::new(p) }).unwrap();
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    let services = Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = LightcraftApp::new(s, services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.request("ui.set", json!({"view": "detail"}), T);
    h.settle(SETTLE);
    if !lightcraft_engine::enhance::AVAILABLE {
        return;
    }
    let canvas = h.app.canvas_rect.unwrap();
    let r = widget(&h, "notice:denoiseHint");
    assert!(r.right() <= canvas.right() - 15.0 && r.bottom() <= canvas.bottom() - 15.0 && r.left() > canvas.center().x, "bottom right: {r:?} in {canvas:?}");
    let faults = layout_faults(&h, r.expand(2.0));
    assert!(faults.is_empty(), "{faults:?}");
    shot(&mut h, "denoise-hint.png");
    // the corner follows the setting
    h.app.ui.settings.denoise_hint_corner = crate::state::HintCorner::TopLeft;
    h.settle(SETTLE);
    let r = widget(&h, "notice:denoiseHint");
    assert!(r.left() >= canvas.left() + 15.0 && r.top() >= canvas.top() + 15.0 && r.right() < canvas.center().x, "top left: {r:?}");
    // denoise. → Enhance, Denoise ticked
    if let Some(e) = h.app.enhance.last.as_mut() {
        e.denoise = false;
    }
    let r = h.request("ui.clickWidget", json!({"id": "button:denoiseHint"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Enhance { opts }) if opts.denoise), "{:?}", h.app.ui.dialog.is_some());
    h.app.ui.dialog = None;
    h.settle(SETTLE);
    // under the ISO limit: none
    h.app.ui.settings.denoise_hint_iso = 12800;
    h.settle(SETTLE);
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "notice:denoiseHint"));
    h.app.ui.settings.denoise_hint_iso = 3200;
    h.settle(SETTLE);
    // hide.: gone for this photo
    let r = h.request("ui.clickWidget", json!({"id": "button:denoiseHintHide"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(!h.app.widgets.iter().any(|(w, _)| w == "notice:denoiseHint"));
}
