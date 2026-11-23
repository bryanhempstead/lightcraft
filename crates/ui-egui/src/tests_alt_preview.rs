//! U03: headless tests of the Alt/Option-drag previews through real input: a slider drag, Alt
//! pressed and released during it, and the loupe's pixels at each step (adapted from upstream
//! storytold/lightcraft's `tests/alt_preview.rs`).

use std::time::Duration;

use egui::{Event, Modifiers, PointerButton};
use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn detail(section: &str) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, [1400.0, 1400.0], 1.0);
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    if !h.app.ui.section_open(section) {
        h.app.ui.toggle_section(section);
    }
    h.app.renderer.keep_pixels = true;
    h.settle(SETTLE);
    // under load the first loupe render can land after the queue looks idle
    h.step_until(SETTLE, |h| h.app.renderer.textures.get(&crate::render::Slot::Main).is_some_and(|t| t.pixels.is_some()));
    h
}

/// Share of the loupe render's pixels for which `f` holds.
fn share(h: &Headless, f: impl Fn([u8; 3]) -> bool) -> f64 {
    let t = h.app.renderer.textures.get(&crate::render::Slot::Main).expect("loupe render");
    let px = &t.pixels.as_ref().expect("pixels kept").pixels;
    px.iter().filter(|c| f([c.r(), c.g(), c.b()])).count() as f64 / px.len().max(1) as f64
}

fn binary(c: [u8; 3]) -> bool {
    c.iter().all(|v| *v == 0 || *v == 255)
}

fn grey(c: [u8; 3]) -> bool {
    c[0] == c[1] && c[1] == c[2]
}

/// Drag slider `control` left with the button held; Alt pressed mid-drag shows `check`'s view,
/// released shows the photo again; the drag still moves the slider.
fn alt_drag(h: &mut Headless, control: &str, check: fn([u8; 3]) -> bool) {
    assert!(share(h, check) < 0.5, "the photo itself is no preview");
    let track = h.app.widgets.iter().find(|(w, _)| w == &format!("slider:{control}")).map(|(_, r)| *r).expect("slider on screen");
    let from = track.center();
    h.app.synthetic.push(Event::PointerMoved(from));
    h.app.synthetic.push(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    for i in 1..=8 {
        h.app.synthetic.push(Event::PointerMoved(from - egui::vec2(4.0 * i as f32, 0.0)));
    }
    h.settle(SETTLE);
    assert_eq!(h.app.ui.dragging_control.as_deref(), Some(control));
    assert!(share(h, check) < 0.5, "no preview without Alt");
    h.app.synthetic.push(Event::ModifiersChanged(Modifiers::ALT));
    // under load the preview's render can land after the queue looked idle: wait for it
    h.settle(SETTLE);
    h.step_until(SETTLE, |h| share(h, check) > 0.99);
    assert!(share(h, check) > 0.99, "{control}: the preview: {} ({})", share(h, check), h.app.renderer.diagnose());
    h.app.synthetic.push(Event::ModifiersChanged(Modifiers::NONE));
    h.settle(SETTLE);
    h.step_until(SETTLE, |h| share(h, check) < 0.5);
    assert!(share(h, check) < 0.5, "the photo is back ({})", h.app.renderer.diagnose());
    h.app.synthetic.push(Event::PointerButton {
        pos: from - egui::vec2(32.0, 0.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.settle(SETTLE);
    assert_eq!(h.app.ui.dragging_control, None);
}

#[test]
fn alt_during_a_blacks_drag_shows_shadow_clipping_until_released() {
    let mut h = detail("light");
    let before = h.app.session.develop_of(h.app.session.active().expect("photo")).expect("develop").light.blacks;
    alt_drag(&mut h, "light.blacks", binary);
    let after = h.app.session.develop_of(h.app.session.active().expect("photo")).expect("develop").light.blacks;
    assert!(after < before, "the drag itself still moved Blacks: {before} → {after}");
}

#[test]
fn alt_during_sharpening_drags_shows_greyscale_and_the_mask() {
    let mut h = detail("detail");
    let r = h.request(
        "engine.execute",
        json!({"command": "develop.set", "params": {"values": {"detail.sharpenAmount": 60, "detail.sharpenMasking": 50}}}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    alt_drag(&mut h, "detail.sharpenAmount", grey);
    // Masking: a grey mask (white where sharpening applies)
    alt_drag(&mut h, "detail.sharpenMasking", grey);
}
