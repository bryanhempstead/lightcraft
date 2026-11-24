//! Headless tests of the side panels' section order (`panels::order`): dragging a header moves the
//! section, the order and the sections open at launch survive a restart (`ui.json`), and the
//! commands do the same as the mouse.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::panels::order::{DEVELOP_LEFT, DEVELOP_RIGHT, LIBRARY_LEFT, LIBRARY_RIGHT, ordered};
use crate::tests_panels::layout_faults;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn demo(ui: serde_json::Value) -> Headless {
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
    let r = h.request("ui.set", ui, T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    h
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn exec(h: &mut Headless, c: &str, p: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": c, "params": p}), T);
    assert_eq!(r["ok"], true, "{c}: {r}");
    h.settle(SETTLE);
    r["result"].clone()
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let p = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": p.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

/// Drag from `a` to `b` (12 steps), then let the panel settle.
fn drag(h: &mut Headless, a: egui::Pos2, b: egui::Pos2) {
    let r = h.request("ui.drag", json!({"x": a.x, "y": a.y, "toX": b.x, "toY": b.y, "steps": 12}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
}

/// The Develop panels move by dragging their headers: Calibration dragged above Basic lands first,
/// Basic dragged under the last panel lands last; the panel's texts stay unclipped.
#[test]
fn develop_panels_reorder_by_dragging_their_headers() {
    let mut h = demo(json!({"module": "develop", "view": "detail", "right": "edit", "openSections": [], "leftPanel": true}));
    let builtin = ordered(&h.app.ui.settings, DEVELOP_RIGHT);
    assert_eq!(builtin.first(), Some(&"basic"));
    // Calibration's header onto Basic's top half
    let (cal, basic) = (widget(&h, "section:calibration"), widget(&h, "section:basic"));
    drag(&mut h, cal.center(), basic.center() - egui::vec2(0.0, 8.0));
    let now = ordered(&h.app.ui.settings, DEVELOP_RIGHT);
    assert_eq!(now.first(), Some(&"calibration"), "{now:?}");
    assert_eq!(now.get(1), Some(&"basic"), "{now:?}");
    assert!(widget(&h, "section:calibration").top() < widget(&h, "section:basic").top(), "drawn in the new order");
    // Basic under the last header (Lens Blur): last
    let (basic, last) = (widget(&h, "section:basic"), widget(&h, "section:lensBlur"));
    drag(&mut h, basic.center(), last.center() + egui::vec2(0.0, 12.0));
    let now = ordered(&h.app.ui.settings, DEVELOP_RIGHT);
    assert_eq!(now.last(), Some(&"basic"), "{now:?}");
    // a click on a header still opens it (a click isn't a drag)
    let detail = widget(&h, "section:detail");
    h.request("ui.click", json!({"x": detail.center().x, "y": detail.center().y}), T);
    h.settle(SETTLE);
    assert!(h.app.ui.section_open("detail"));
    assert_eq!(ordered(&h.app.ui.settings, DEVELOP_RIGHT), now, "and moves nothing");
    let panel = widget(&h, "panel:right_panel");
    let faults = layout_faults(&h, panel);
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    shot(&mut h, "panel-order-develop-right.png");
    // mid-drag: the drop line and the moving title (for the screenshot), then Esc cancels
    let (hsl, tone) = (widget(&h, "section:hsl"), widget(&h, "section:toneCurve"));
    h.app.synthetic.push(egui::Event::PointerMoved(hsl.center()));
    h.app.synthetic.push(egui::Event::PointerButton {
        pos: hsl.center(),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    for i in 1..=8 {
        let t = i as f32 / 8.0;
        h.app.synthetic.push(egui::Event::PointerMoved(hsl.center() + (tone.top() - 4.0 - hsl.center().y) * t * egui::Vec2::Y));
    }
    for _ in 0..12 {
        h.step();
    }
    assert_eq!(crate::panels::order::dragging(&h.view.ctx, DEVELOP_RIGHT).as_deref(), Some("hsl"));
    shot(&mut h, "panel-order-dragging.png");
    h.request("ui.key", json!({"key": "Escape"}), T);
    h.step();
    h.app.synthetic.push(egui::Event::PointerButton {
        pos: egui::pos2(hsl.center().x, tone.top() - 4.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    h.settle(SETTLE);
    assert_eq!(crate::panels::order::dragging(&h.view.ctx, DEVELOP_RIGHT), None);
    assert_eq!(ordered(&h.app.ui.settings, DEVELOP_RIGHT), now, "Esc left the order alone");
}

/// The other sides (Develop's left panel, the Library's left and right panels) reorder too; the
/// order and Open by Default survive a restart; reset. goes back to the built-in order.
#[test]
fn every_side_panel_keeps_its_order_and_default_open_sections_across_restart() {
    let mut h = demo(json!({"module": "develop", "view": "detail", "right": "edit", "leftPanel": true, "developLeftSections": []}));
    // Develop left: History dragged above the Navigator
    let (hist, nav) = (widget(&h, "section:dev.history"), widget(&h, "section:dev.navigator"));
    drag(&mut h, hist.center(), egui::pos2(nav.center().x, nav.top() + 3.0));
    assert_eq!(ordered(&h.app.ui.settings, DEVELOP_LEFT).first(), Some(&"history"));
    assert!(widget(&h, "section:dev.history").top() < widget(&h, "section:dev.navigator").top());
    let left = widget(&h, "panel:left_panel");
    let faults = layout_faults(&h, left);
    assert!(faults.is_empty(), "{}", faults.join("\n"));
    shot(&mut h, "panel-order-develop-left.png");
    // the commands: Library left (Folders before Catalog) and right (Metadata first)
    let r = exec(&mut h, "panel.order", json!({"side": LIBRARY_LEFT, "order": ["folders", "catalog"]}));
    assert_eq!(r["order"][0], "folders", "{r}");
    assert_eq!(r["order"].as_array().map(Vec::len), Some(5), "the rest follow: {r}");
    exec(&mut h, "panel.order", json!({"side": "right", "order": ["calibration"]}));
    assert_eq!(ordered(&h.app.ui.settings, DEVELOP_RIGHT).first(), Some(&"calibration"), "`right` in Develop is Develop's right panel");
    exec(&mut h, "panel.order", json!({"side": LIBRARY_RIGHT, "order": ["metadata"]}));
    let bad = h.request("engine.execute", json!({"command": "panel.order", "params": {"side": LIBRARY_LEFT, "order": ["basic"]}}), T);
    assert_ne!(bad["ok"], true, "a section of another panel is refused: {bad}");
    let bad = h.request("engine.execute", json!({"command": "panel.order", "params": {"side": "sideways"}}), T);
    assert_ne!(bad["ok"], true, "{bad}");
    exec(&mut h, "view.library", json!({"view": "photoGrid"}));
    h.settle(SETTLE);
    assert!(widget(&h, "sidebarSection:folders").top() < widget(&h, "sidebarSection:catalog").top(), "Library left in the new order");
    assert!(widget(&h, "section:lib.metadata").top() < widget(&h, "section:lib.histogram").top(), "Library right in the new order");
    for panel in ["panel:left_panel", "panel:right_panel"] {
        let r = widget(&h, panel);
        let faults = layout_faults(&h, r);
        assert!(faults.is_empty(), "{panel}: {}", faults.join("\n"));
    }
    shot(&mut h, "panel-order-library.png");
    // Open by Default: Detail and Effects on Develop's right; Collections on Library's left
    exec(&mut h, "panel.defaultOpen", json!({"id": "detail", "side": DEVELOP_RIGHT, "open": true}));
    let r = exec(&mut h, "panel.defaultOpen", json!({"id": "effects"}));
    assert_eq!((r["side"].as_str(), r["open"].as_bool()), (Some(DEVELOP_RIGHT), Some(true)), "the only side with Effects; toggled on: {r}");
    exec(&mut h, "panel.defaultOpen", json!({"id": "collections", "side": LIBRARY_LEFT, "open": true}));
    // whatever was open at quit…
    h.app.ui.open_sections = vec!["basic".into(), "hsl".into()];
    h.app.ui.collapsed_sidebar = vec!["collections".into()];
    h.app.ui.library_sections = vec!["keywording".into()];
    // …restart: the saved UI state read back
    let saved = serde_json::to_string(&h.app.ui).unwrap();
    let back = serde_json::from_str::<crate::UiState>(&saved).unwrap().sanitized();
    assert_eq!(back.settings.panel_order, h.app.ui.settings.panel_order);
    assert_eq!(ordered(&back.settings, DEVELOP_LEFT).first(), Some(&"history"));
    let mut open: Vec<&str> = back.open_sections.iter().map(String::as_str).collect();
    open.sort_unstable();
    assert_eq!(open, ["detail", "effects"], "the sections chosen open, the rest closed");
    assert!(!back.sidebar_section_collapsed("collections"), "Collections opens at launch");
    assert!(back.sidebar_section_collapsed("catalog") && back.sidebar_section_collapsed("folders"));
    assert_eq!(back.library_sections, ["keywording"], "a side without defaults keeps what was open");
    // the header's right-click menu offers it, and reset.
    h.app.ui.open_sections.clear();
    exec(&mut h, "view.develop", json!({}));
    let det = widget(&h, "section:detail");
    h.request("ui.click", json!({"x": det.center().x, "y": det.center().y, "button": "right"}), T);
    h.settle(SETTLE);
    assert!(h.app.widgets.iter().any(|(w, _)| w == "menu:defaultOpen-detail"), "Open by Default in the header's menu");
    assert!(h.app.widgets.iter().any(|(w, _)| w == &format!("menu:resetOrder-{DEVELOP_RIGHT}")));
    h.request("ui.key", json!({"key": "Escape"}), T);
    let r = exec(&mut h, "panel.resetOrder", json!({}));
    assert_eq!(r["reset"].as_array().map(Vec::len), Some(4), "{r}");
    for side in crate::panels::order::SIDES {
        let builtin: Vec<&str> = crate::panels::order::defaults(side).iter().map(|d| d.0).collect();
        assert_eq!(ordered(&h.app.ui.settings, side), builtin);
    }
    assert!(h.app.ui.settings.panel_default_open.is_empty());
}
