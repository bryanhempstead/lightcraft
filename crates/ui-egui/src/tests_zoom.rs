//! LR-VIEW-ZOOM: native-resolution detail at 1:1 (`crate::tiles`) and LR-VIEW-COLORMANAGED: the
//! window's colour space (`crate::display_p3`).

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
