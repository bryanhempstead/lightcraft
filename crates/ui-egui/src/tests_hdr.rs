//! HDR in the UI, headless: the split SDR | HDR histogram of a photo edited in HDR, and the HDR
//! rendition in Compare, Survey and the Second Window on an HDR display (a stand-in
//! [`HdrView`] that records what it was asked to show). `LIGHTCRAFT_TEST_SHOTS=<dir>` writes
//! real-size screenshots.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::hdr_view::{HdrPixels, HdrView};
use crate::render::Slot;
use crate::tests_library2::{headless, settle, shot};

/// An HDR display with `headroom`, recording each slot it was asked to show.
struct FakeView {
    headroom: f32,
    shown: Arc<Mutex<Vec<(String, f32)>>>,
}

impl HdrView for FakeView {
    fn headroom(&self) -> Option<f32> {
        Some(self.headroom)
    }
    fn texture(&mut self, slot: &str, px: &HdrPixels) -> Option<egui::TextureId> {
        self.shown.lock().unwrap().push((slot.to_string(), px.peak));
        None // (draws the 8-bit texture instead)
    }
}

fn sunsets(s: &lightcraft_engine::Session) -> Vec<u64> {
    s.catalog.photos().filter(|p| p.meta.keywords.iter().any(|k| k == "sunset")).map(|p| p.id.0).collect()
}

fn wait(h: &mut crate::headless::Headless, what: &str, mut done: impl FnMut(&crate::headless::Headless) -> bool) {
    let t = Instant::now();
    while !done(h) {
        assert!(t.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn hdr_histogram_splits_sdr_and_hdr() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids = sunsets(&s);
    let Some(&id) = ids.first() else { return };
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    s.execute("develop.hdr", &json!({"enabled": true, "maxEv": 3.0})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
    let mut h = headless(s);
    h.app.run("view.develop", json!({})).unwrap();
    settle(&mut h);
    wait(&mut h, "the HDR histogram", |h| h.app.renderer.textures.get(&Slot::HdrMaster).is_some_and(|t| t.hdr_histogram.is_some()));
    settle(&mut h);
    let hist = h.app.renderer.textures.get(&Slot::HdrMaster).and_then(|t| t.hdr_histogram.clone()).unwrap();
    assert!((hist.limit - 3.0).abs() < 1e-6);
    assert!(hist.above_white() > 0, "the sun rises above SDR white");
    let r = h.request("ui.widgets", json!({"filter": "histogram:hdr"}), Duration::from_secs(10));
    assert!(r.to_string().contains("histogram:hdr"), "the HDR histogram is drawn");
    shot(&mut h, "hdr-histogram-sdr-display.png");
    // HDR off: the ordinary histogram again
    h.app.run("develop.hdr", json!({"enabled": false})).unwrap();
    settle(&mut h);
    assert!(crate::panels::hdr_histogram::current(&mut h.app, lightcraft_catalog::PhotoId(id)).is_none());
}

#[test]
fn compare_survey_and_second_window_show_the_hdr_rendition() {
    let mut s = lightcraft_engine::Session::with_demo();
    let ids = sunsets(&s);
    if ids.len() < 2 {
        return;
    }
    for id in &ids[..2] {
        s.execute("library.select", &json!({"ids": [id]})).unwrap();
        s.execute("develop.hdr", &json!({"enabled": true})).unwrap();
    }
    s.execute("library.select", &json!({"ids": [ids[0], ids[1]]})).unwrap();
    let mut h = headless(s);
    let shown = Arc::new(Mutex::new(Vec::new()));
    h.app.hdr_view = Some(Box::new(FakeView { headroom: 2.0, shown: shown.clone() }));
    let seen = |what: &str| shown.lock().unwrap().iter().any(|(s, peak)| s == what && *peak > 1.0);
    h.app.run("view.compare", json!({})).unwrap();
    wait(&mut h, "Compare's HDR tiles", |_| seen("Compare(0)") && seen("Compare(1)"));
    h.app.run("view.survey", json!({})).unwrap();
    wait(&mut h, "Survey's HDR tiles", |_| seen("Compare(0)"));
    h.app.run("view.secondWindow", json!({"show": true})).unwrap();
    wait(&mut h, "the Second Window's HDR view", |_| shown.lock().unwrap().iter().any(|(s, _)| s.starts_with("second:")));
    // fitted to the display's headroom: nothing brighter than 2× SDR white
    assert!(shown.lock().unwrap().iter().all(|(_, peak)| *peak <= 2.0 + 1e-3), "{:?}", shown.lock().unwrap());
}
