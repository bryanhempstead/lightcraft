//! LR-VIEW-ZOOM: native-resolution detail at 1:1 (`crate::tiles`), the remembered click level and
//! wheel zoom; LRC-VIEW-LOUPEOVERLAY: overlays on the loupe; LRC-LIB-MISSING: the loupe's missing
//! original banner; LR-VIEW-COLORMANAGED: the window's colour space (`crate::display_p3`).

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const RENDER: Duration = Duration::from_secs(240);

/// The 1:1 tests decode and render 24 MP photos: one at a time, so they don't starve the rest of
/// the suite running beside them.
static HEAVY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn heavy() -> std::sync::MutexGuard<'static, ()> {
    HEAVY.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn demo() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    Headless::new(app, [1000.0, 700.0], 1.0)
}

fn exec(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

/// The loupe's detail layer: (active, pieces of the current look).
fn detail(h: &Headless) -> (bool, usize) {
    h.app.renderer.tiles.get(&0).map_or((false, 0), |t| (t.look.is_some(), t.look.map_or(0, |l| t.pieces.iter().filter(|p| p.look == l).count())))
}

/// Is the visible part of the photo covered by pieces of the current look?
fn covered(h: &Headless) -> bool {
    let (Some(t), Some(img), Some(canvas)) = (h.app.renderer.tiles.get(&0), h.app.image_rect, h.app.canvas_rect) else { return false };
    let Some(look) = t.look else { return false };
    let vis = img.intersect(canvas.shrink(24.0));
    let n = |p: egui::Pos2| (((p.x - img.left()) / img.width()) as f64, ((p.y - img.top()) / img.height()) as f64);
    let (a, b) = (n(vis.min), n(vis.max));
    t.covered(look, &lightcraft_pipeline::Region { x0: a.0, y0: a.1, x1: b.0, y1: b.1 })
}

#[test]
fn one_to_one_shows_the_photos_own_pixels_equal_to_the_full_size_render() {
    let _one = heavy();
    let mut h = demo();
    h.app.renderer.keep_pixels = true;
    h.request("ui.set", json!({"view": "detail", "right": "none", "leftPanel": false, "filmstrip": false}), T);
    h.settle(RENDER);
    let (active, _) = detail(&h);
    assert!(!active, "Fit: the preview is enough");
    exec(&mut h, "view.zoom100", json!({}));
    assert!(h.step_until(RENDER, covered), "1:1: the visible part is rendered at native resolution");
    let t = &h.app.renderer.tiles[&0];
    assert_eq!(t.level, 1.0);
    let look = t.look.unwrap();
    let piece = t.pieces.iter().find(|p| p.look == look && p.size == [crate::tiles::PIECE; 2]).expect("a whole piece");
    let (rect, px) = (piece.rect, piece.pixels.clone().expect("kept pixels"));
    // the same pixels of the full-size (export-size) render
    let id = h.app.session.active().unwrap();
    let (fw, fh) = h.app.session.detail_size(id, 1.0, true).unwrap();
    let full = h.app.session.render_now(id, fw, fh).unwrap().image;
    assert_eq!((full.width, full.height), (fw, fh), "the export-size render");
    assert!(fw.max(fh) >= 4000, "a large photo: {fw}×{fh}");
    let (x0, y0) = ((rect.x0 * fw as f64).round() as usize, (rect.y0 * fh as f64).round() as usize);
    let (mut max, mut sum) = (0u8, 0u64);
    for y in 0..piece.size[1] {
        for x in 0..piece.size[0] {
            let a = full.data[(y0 + y) * fw + x0 + x];
            let b = px.pixels[y * piece.size[0] + x];
            for (c, v) in [b.r(), b.g(), b.b()].into_iter().enumerate() {
                let d = a[c].abs_diff(v);
                max = max.max(d);
                sum += d as u64;
            }
        }
    }
    let mean = sum as f64 / (piece.size[0] * piece.size[1] * 3) as f64;
    assert!(max <= 3 && mean < 0.5, "tile vs full-size render: max {max}, mean {mean:.3}");
    // ui.inspect reports it
    let r = h.request("ui.inspect", json!({}), T);
    assert_eq!(r["result"]["perf"]["detail"]["0"]["active"], true, "{}", r["result"]["perf"]["detail"]);
}

#[test]
fn panning_at_one_to_one_renders_the_new_part_and_fit_hides_the_detail() {
    let _one = heavy();
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "right": "none", "leftPanel": false, "filmstrip": false}), T);
    exec(&mut h, "view.zoom100", json!({}));
    assert!(h.step_until(RENDER, covered));
    exec(&mut h, "view.navigate", json!({"pan": [0.15, 0.2]}));
    h.step();
    assert!(h.step_until(RENDER, covered), "the panned-to part is rendered too");
    // pieces of earlier regions stay (panning back costs nothing)
    let (_, n) = detail(&h);
    assert!(n >= 4, "{n} pieces");
    // panning over what is rendered draws pieces only: frame times stay those of the UI
    h.settle(RENDER);
    let mut times = Vec::new();
    for i in 0..40 {
        let x = 0.15 + 0.002 * (i % 20) as f64;
        exec(&mut h, "view.navigate", json!({"pan": [x, 0.2]}));
        let t0 = std::time::Instant::now();
        h.step();
        times.push(t0.elapsed());
    }
    times.sort();
    let (median, worst) = (times[times.len() / 2], times[times.len() - 1]);
    eprintln!("panning at 1:1 over rendered tiles: median {:.1} ms, slowest {:.1} ms", median.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
    // (a sanity bound: the machine running the tests may be loaded)
    assert!(median < Duration::from_millis(250), "median pan frame {median:?}");
    exec(&mut h, "view.zoomFit", json!({}));
    h.settle(T);
    assert!(!detail(&h).0, "Fit: no detail layer");
}

#[test]
fn an_edit_at_one_to_one_re_renders_the_detail_in_the_new_look() {
    let _one = heavy();
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "right": "none", "leftPanel": false, "filmstrip": false}), T);
    exec(&mut h, "view.zoom100", json!({}));
    assert!(h.step_until(RENDER, covered));
    let before = h.app.renderer.tiles[&0].look;
    exec(&mut h, "develop.set", json!({"control": "light.exposure", "value": 1.0}));
    h.step();
    assert_ne!(h.app.renderer.tiles[&0].look, before, "a new look");
    assert!(h.step_until(RENDER, covered), "covered in the new look");
    let t = &h.app.renderer.tiles[&0];
    assert!(t.pieces.iter().all(|p| Some(p.look) == t.look), "the old look's pieces are gone");
}

#[test]
fn auto_tags_the_window_display_p3_on_a_mac_and_the_manual_choices_do_not() {
    use crate::display_p3::{ViewSpace, WindowSpace};
    #[derive(Default)]
    struct Probe(std::rc::Rc<std::cell::Cell<Option<bool>>>);
    impl WindowSpace for Probe {
        fn request_p3_tag(&mut self, on: bool) {
            self.0.set(Some(on));
        }
        fn tagged(&self) -> Option<&'static str> {
            self.0.get().filter(|on| *on).map(|_| "displayP3")
        }
    }
    let mut h = demo();
    let asked = std::rc::Rc::new(std::cell::Cell::new(None));
    h.app.window_space = Some(Box::new(Probe(asked.clone())));
    h.app.headless_host = false;
    h.step();
    assert_eq!(asked.get(), Some(cfg!(target_os = "macos")), "Auto: tagged on a Mac");
    let r = exec(&mut h, "app.viewSpace", json!({}));
    assert_eq!(r["colorManaged"], cfg!(target_os = "macos"), "{r}");
    assert_eq!(r["texturesP3"], cfg!(target_os = "macos"), "{r}");
    for space in ["displayP3", "srgb"] {
        exec(&mut h, "app.viewSpace", json!({"space": space}));
        h.step();
        assert_eq!(asked.get(), Some(false), "{space}: untagged, values as they are");
        assert_eq!(h.app.renderer.display_p3, space == "displayP3");
    }
    assert!(ViewSpace::Auto.tags_surface() == cfg!(target_os = "macos") && !ViewSpace::Srgb.tags_surface());
    // video frames, previews and map tiles get the same conversion as the photos
    let red = crate::display_p3::screen_image(true, [1, 1], &[255, 0, 0, 255]).pixels[0];
    assert!(red.r() < 250 && red.g() > 10, "sRGB red as P3 values: {red:?}");
    let red = crate::display_p3::screen_image(false, [1, 1], &[255, 0, 0, 255]).pixels[0];
    assert_eq!((red.r(), red.g()), (255, 0));
}

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

fn loupe(h: &mut Headless) {
    h.request("ui.set", json!({"view": "detail", "right": "none", "leftPanel": false, "filmstrip": false}), T);
    h.settle(T);
}

/// The zoom a click goes to is the level last picked (click. menu, the toolbar's zoom menu or the
/// Navigator), and it, the wheel mode and step survive a restart (`ui.json`).
#[test]
fn click_zoom_level_and_wheel_settings_are_remembered_across_restart() {
    use crate::state::{WheelMode, Zoom};
    let mut h = demo();
    loupe(&mut h);
    // the toolbar's zoom menu / Navigator: picking a level makes it the click level
    let r = exec(&mut h, "view.navigate", json!({"zoom": {"percent": 200}, "remember": true}));
    assert_eq!(r["clickZoom"], 200, "{r}");
    exec(&mut h, "view.navigate", json!({"zoom": "fit"}));
    assert_eq!(h.app.ui.click_zoom, 200, "Fit leaves the click level alone");
    assert_eq!(exec(&mut h, "view.clickZoom", json!({"ratio": "fill"}))["ratio"], "fill");
    assert_eq!(h.app.ui.click_zoom, 0);
    assert_eq!(exec(&mut h, "view.clickZoom", json!({"ratio": 0.5}))["label"], "1:2");
    let r = h.request("engine.execute", json!({"command": "view.clickZoom", "params": {"ratio": 7}}), T);
    assert_ne!(r["ok"], true, "7:1 isn't offered: {r}");
    exec(&mut h, "view.clickZoom", json!({"ratio": 4}));
    exec(&mut h, "view.wheelZoom", json!({"mode": "zoom", "step": 50}));
    let r = h.request("engine.execute", json!({"command": "view.wheelZoom", "params": {"step": 33}}), T);
    assert_ne!(r["ok"], true, "{r}");
    // quit and start again: the saved UI state is read back
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    let back: crate::UiState = serde_json::from_str::<crate::UiState>(&saved).unwrap().sanitized();
    assert_eq!((back.click_zoom, back.wheel, back.wheel_step), (400, WheelMode::Zoom, 50));
    let mut h = demo();
    h.app.ui = back;
    loupe(&mut h);
    assert_eq!(h.app.ui.zoom, Zoom::Fit);
    h.request("ui.clickWidget", json!({"id": "canvas:image", "fx": 0.3, "fy": 0.4}), T);
    assert_eq!(h.app.ui.zoom, Zoom::Percent(400.0), "a click zooms to the remembered 4:1");
    assert!((h.app.ui.pan.0 - 0.3).abs() < 0.01 && (h.app.ui.pan.1 - 0.4).abs() < 0.01, "centred on the clicked point: {:?}", h.app.ui.pan);
    h.settle(T);
    let c = h.app.canvas_rect.unwrap().center();
    h.request("ui.click", json!({"x": c.x, "y": c.y}), T);
    assert_eq!(h.app.ui.zoom, Zoom::Fit, "the next click goes back to Fit");
    // Z goes to the same level; a Fill click level fills
    h.settle(T);
    h.request("ui.key", json!({"key": "z"}), T);
    assert_eq!(h.app.ui.zoom, Zoom::Percent(400.0));
    exec(&mut h, "view.zoomFit", json!({}));
    exec(&mut h, "view.clickZoom", json!({"ratio": "fill"}));
    h.settle(T);
    h.request("ui.clickWidget", json!({"id": "canvas:image"}), T);
    assert_eq!(h.app.ui.zoom, Zoom::Fill);
    // a damaged value in ui.json is brought back into range
    let bad: crate::UiState = serde_json::from_value(json!({"clickZoom": 99999, "wheelStep": 7})).unwrap();
    let bad = bad.sanitized();
    assert_eq!((bad.click_zoom, bad.wheel_step), (100, 25));
}

/// The plain wheel zooms in steps anchored under the pointer (a notched mouse wheel in the default
/// mode, any scroll in "zoom"); a trackpad's two-finger scroll still pans; "pan" pans with both.
#[test]
fn wheel_zoom_steps_anchored_under_the_pointer() {
    use crate::state::Zoom;
    let mut h = demo();
    loupe(&mut h);
    let fit = h.app.image_rect.unwrap();
    let anchor = h.app.canvas_rect.unwrap().center() + egui::vec2(60.0, -40.0);
    h.request("ui.move", json!({"x": anchor.x, "y": anchor.y}), T);
    // one notch up from Fit: 25 % larger, the photo no longer fits
    h.request("ui.scroll", json!({"dy": 1.0, "unit": "line"}), T);
    for _ in 0..40 {
        h.step();
    }
    let one = h.app.image_rect.unwrap();
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "{:?}", h.app.ui.zoom);
    assert!((one.width() / fit.width() - 1.25).abs() < 0.01, "one notch = 25 %: {} → {}", fit.width(), one.width());
    // from 1:1, a notch keeps the photo's point under the pointer where it is
    exec(&mut h, "view.navigate", json!({"zoom": {"percent": 100}, "pan": [0.4, 0.45]}));
    h.settle(T);
    let before = h.app.image_rect.unwrap();
    let point = (anchor - before.min) / before.size();
    h.request("ui.scroll", json!({"dy": 1.0, "unit": "line"}), T);
    let mut worst = std::time::Duration::ZERO;
    for _ in 0..40 {
        let t0 = std::time::Instant::now();
        h.step();
        worst = worst.max(t0.elapsed());
    }
    let after = h.app.image_rect.unwrap();
    assert!((after.width() / before.width() - 1.25).abs() < 0.01, "{} → {}", before.width(), after.width());
    assert!(((anchor - after.min) / after.size() - point).length() < 0.001, "the point under the pointer stays put");
    eprintln!("wheel zoom: slowest frame {:.1} ms", worst.as_secs_f64() * 1e3);
    // notches down: back to Fit and no further
    h.request("ui.scroll", json!({"dy": -40.0, "unit": "line"}), T);
    for _ in 0..60 {
        h.step();
    }
    assert_eq!(h.app.ui.zoom, Zoom::Fit);
    // a trackpad's scroll (points) pans in the default mode
    exec(&mut h, "view.zoom100", json!({}));
    h.settle(T);
    let (z, pan) = (h.app.ui.zoom, h.app.ui.pan);
    h.request("ui.scroll", json!({"dx": 30.0, "dy": -20.0}), T);
    for _ in 0..20 {
        h.step();
    }
    assert_eq!(h.app.ui.zoom, z, "trackpad scroll doesn't zoom");
    assert_ne!(h.app.ui.pan, pan, "it pans");
    // "zoom": the trackpad zooms too; "pan": the mouse wheel pans
    exec(&mut h, "view.wheelZoom", json!({"mode": "zoom"}));
    h.request("ui.scroll", json!({"dy": 40.0}), T);
    for _ in 0..20 {
        h.step();
    }
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(p) if (p - 125.0).abs() < 0.5), "{:?}", h.app.ui.zoom);
    exec(&mut h, "view.wheelZoom", json!({"mode": "pan", "step": 10}));
    let z = h.app.ui.zoom;
    h.request("ui.scroll", json!({"dy": 1.0, "unit": "line"}), T);
    for _ in 0..40 {
        h.step();
    }
    assert_eq!(h.app.ui.zoom, z, "pan mode: the wheel pans");
    // a scroll over the toolbar never moves the photo
    exec(&mut h, "view.wheelZoom", json!({"mode": "auto"}));
    h.request("ui.move", json!({"x": 10, "y": 10}), T);
    h.request("ui.scroll", json!({"dy": 1.0, "unit": "line"}), T);
    for _ in 0..40 {
        h.step();
    }
    assert_eq!(h.app.ui.zoom, z);
}

/// The composition guides, frames and grid draw on the loupe outside the crop tool (Library loupe
/// and Develop), never take a click, and O / ⇧O cycle and turn them there.
#[test]
fn loupe_overlays_draw_outside_the_crop_tool_and_never_block_clicks() {
    use crate::state::{CropOverlay, Zoom};
    let mut h = demo();
    h.request("ui.set", json!({"module": "develop", "view": "detail", "right": "edit", "leftPanel": false, "filmstrip": false}), T);
    h.settle(T);
    let plain = h.snapshot(T);
    exec(&mut h, "view.compositionOverlay", json!({"overlay": "thirds"}));
    exec(&mut h, "view.frameGuides", json!({"frames": ["4:5"], "dim": false}));
    let r = exec(&mut h, "view.loupeGrid", json!({"on": true, "size": 40, "opacity": 0.5}));
    assert_eq!(r["grid"], true, "{r}");
    let shot = h.snapshot(T);
    let img = widget(&h, "canvas:image").expect("the photo");
    assert!(widget(&h, "overlay:composition:thirds").is_some() && widget(&h, "overlay:grid").is_some());
    let frame = widget(&h, "overlay:frame:4:5").expect("the 4:5 frame");
    assert!((frame.width() / frame.height() - 0.8).abs() < 0.01, "{frame:?}");
    assert!(img.expand(0.5).contains_rect(frame), "inside the photo");
    // the thirds line is drawn on the photo's pixels
    let (x, y) = ((img.left() + img.width() / 3.0).round() as usize, (img.top() + img.height() * 0.55).round() as usize);
    let px = |c: &egui::ColorImage, x: usize, y: usize| c.pixels[y * c.size[0] + x];
    let lum = |c: egui::Color32| c.r() as i32 + c.g() as i32 + c.b() as i32;
    let near = (x.saturating_sub(1)..=x + 1).map(|x| lum(px(&shot, x, y)) - lum(px(&plain, x, y))).max().unwrap();
    assert!(near > 30, "the thirds line brightens the photo at x = {x}: {near}");
    // a click still zooms
    h.request("ui.clickWidget", json!({"id": "canvas:image"}), T);
    assert!(matches!(h.app.ui.zoom, Zoom::Percent(_)), "{:?}", h.app.ui.zoom);
    exec(&mut h, "view.zoomFit", json!({}));
    // O cycles the composition overlay, ⇧O turns it; ⌘⌥O hides everything
    h.request("ui.key", json!({"key": "o"}), T);
    assert_eq!(h.app.ui.loupe_overlay.composition, CropOverlay::Grid);
    exec(&mut h, "view.compositionOverlay", json!({"overlay": "spiral"}));
    h.request("ui.key", json!({"key": "o", "shift": true}), T);
    assert_eq!(h.app.ui.loupe_overlay.orient, 1);
    h.request("ui.key", json!({"key": "o", "cmd": true, "alt": true}), T);
    h.settle(T);
    assert!(widget(&h, "overlay:grid").is_none(), "Show off: nothing drawn");
    exec(&mut h, "view.loupeOverlay", json!({"show": true}));
    // the Library loupe too
    h.request("ui.set", json!({"module": "library", "view": "detail"}), T);
    h.settle(T);
    assert!(widget(&h, "overlay:composition:spiral").is_some(), "Library loupe");
    // the crop tool draws its own guides on the uncropped frame instead
    h.request("ui.set", json!({"module": "develop", "view": "detail", "right": "crop"}), T);
    h.settle(T);
    assert!(widget(&h, "overlay:grid").is_none() && widget(&h, "overlay:frame:4:5").is_none());
    assert_eq!(h.app.ui.crop_overlay, CropOverlay::Thirds, "the crop tool keeps its own overlay");
    // guides: ⌘-drag moves them
    h.request("ui.set", json!({"right": "edit"}), T);
    exec(&mut h, "view.loupeGuides", json!({"on": true}));
    h.settle(T);
    let g = widget(&h, "overlay:guides").expect("guides").center();
    let img = widget(&h, "canvas:image").unwrap();
    let to = g + egui::vec2(img.width() * 0.2, 0.0);
    h.request("ui.drag", json!({"x": g.x, "y": g.y, "toX": to.x, "toY": to.y, "cmd": true}), T);
    h.settle(T);
    let at = h.app.ui.loupe_overlay.guides_at;
    assert!((at.0 - 0.7).abs() < 0.02 && (at.1 - 0.5).abs() < 0.02, "{at:?}");
    assert_eq!(h.app.ui.zoom, Zoom::Fit, "dragging a guide doesn't pan or zoom");
    // bad input is refused
    for (c, p) in [("view.frameGuides", json!({"toggle": "4:0"})), ("view.loupeGrid", json!({"size": 1})), ("view.overlayShow", json!({"mode": "x"}))]
    {
        let r = h.request("engine.execute", json!({"command": c, "params": p}), T);
        assert_ne!(r["ok"], true, "{c}: {r}");
    }
}

/// A photo whose original is gone shows a banner over the loupe with loc. (Locate Missing File…)
/// and fldr. (Find Missing Folder…); pointing at where it went clears it.
#[test]
fn missing_original_banner_locates_the_file_or_its_folder() {
    let dir = std::env::temp_dir().join(format!("lightcraft-zoomux-missing-{}", std::process::id()));
    let (gone, moved) = (dir.join("gone"), dir.join("moved"));
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::write(moved.join("IMG_0001.jpg"), b"not decoded here").unwrap();
    let mut h = demo();
    let id = h.app.session.active().unwrap();
    let path = gone.join("IMG_0001.jpg").to_string_lossy().to_string();
    let relink =
        lightcraft_catalog::Op::Relink { id, file_name: "IMG_0001.jpg".into(), source: lightcraft_catalog::Source::File { path }, format: None };
    h.app.session.catalog.apply(relink).unwrap();
    loupe(&mut h);
    assert!(h.step_until(T, |h| widget(h, "notice:missing:loupe").is_some()), "the banner shows");
    let banner = widget(&h, "notice:missing:loupe").unwrap();
    let (loc, fldr) = (widget(&h, "button:missingLocate").unwrap(), widget(&h, "button:missingFolder").unwrap());
    assert!(banner.contains_rect(loc) && banner.contains_rect(fldr), "{banner:?} {loc:?} {fldr:?}");
    assert!(fldr.left() - loc.right() >= 8.0, "chips 8 apart");
    // fldr.: the folder it was in has moved
    let to = moved.to_string_lossy().to_string();
    h.app.services.pick_folder = Some(Box::new(move || Some(to.clone())));
    h.request("ui.clickWidget", json!({"id": "button:missingFolder"}), T);
    let now = match &h.app.session.catalog.photo(id).unwrap().source {
        lightcraft_catalog::Source::File { path } => path.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(std::path::Path::new(&now), moved.join("IMG_0001.jpg"));
    assert!(h.step_until(T, |h| widget(h, "notice:missing:loupe").is_none()), "found: no banner");
    // loc.: gone again, then pointed at the file itself
    let elsewhere = dir.join("elsewhere.jpg");
    std::fs::rename(moved.join("IMG_0001.jpg"), &elsewhere).unwrap();
    let probe = h.app.session.media.availability.probe();
    h.app.session.media.availability.set_probe(probe);
    assert!(h.step_until(T, |h| widget(h, "notice:missing:loupe").is_some()), "missing again");
    let e = elsewhere.to_string_lossy().to_string();
    h.app.services.pick_files = Some(Box::new(move || vec![e.clone()]));
    h.request("ui.clickWidget", json!({"id": "button:missingLocate"}), T);
    assert!(h.step_until(T, |h| widget(h, "notice:missing:loupe").is_none()), "located: no banner");
    let _ = std::fs::remove_dir_all(&dir);
}
