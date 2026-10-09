//! Headless tests of the Lightroom Classic layout: the Library | Develop modules (keys, picker,
//! the photo carried across), the panel keys (Tab, ⇧Tab, F5–F8, T, L), the filmstrip (source,
//! quick filter, height, hide / show), Develop's panels in Classic's order with their switches and
//! solo mode, Library's right panel, and the Import window (Add, then Previous Import).

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::state::{LightsOut, Module, RightPanel, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(30);
const SETTLE: Duration = Duration::from_secs(120);

fn demo(size: [f32; 2]) -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    let mut h = Headless::new(app, size, 1.0);
    h.settle(SETTLE);
    h
}

fn exec(h: &mut Headless, command: &str, params: Value) -> Value {
    let r = h.request("engine.execute", json!({"command": command, "params": params}), T);
    assert_eq!(r["ok"], true, "{command}: {r}");
    r["result"].clone()
}

fn key(h: &mut Headless, k: &str, mods: Value) {
    let mut p = mods;
    p["key"] = json!(k);
    let r = h.request("ui.key", p, T);
    assert_eq!(r["ok"], true, "{r}");
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.step();
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn rect(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

#[test]
fn modules_switch_by_key_and_picker_and_keep_the_photo() {
    let mut h = demo([1500.0, 950.0]);
    exec(&mut h, "keys.set", json!({"profile": "classic"}));
    exec(&mut h, "view.library", json!({"view": "photoGrid"}));
    let third = h.app.session.visible_cloned()[2];
    exec(&mut h, "library.select", json!({"ids": [third.0]}));
    // D: Develop on the same photo, the develop panels on the right
    key(&mut h, "d", json!({}));
    h.step();
    assert_eq!((h.app.ui.module, h.app.ui.view, h.app.ui.right), (Module::Develop, ViewMode::Detail, RightPanel::Edit));
    assert_eq!(h.app.session.active(), Some(third), "the photo stays selected across modules");
    assert!(has(&h, "section:basic") && has(&h, "section:dev.navigator"), "Develop's panels are shown");
    // G: Library grid; E: Library loupe; the Library right panel
    key(&mut h, "g", json!({}));
    assert_eq!((h.app.ui.module, h.app.ui.view), (Module::Library, ViewMode::PhotoGrid));
    key(&mut h, "e", json!({}));
    h.step();
    assert_eq!((h.app.ui.module, h.app.ui.view), (Module::Library, ViewMode::Detail));
    assert!(has(&h, "section:lib.quickDevelop"), "Library's right panel");
    assert!(!has(&h, "section:basic"));
    // the module picker
    click(&mut h, "module:develop");
    assert_eq!(h.app.ui.module, Module::Develop);
    click(&mut h, "module:library");
    assert_eq!((h.app.ui.module, h.app.ui.view), (Module::Library, ViewMode::Detail), "back to the Library view it came from");
    assert_eq!(h.app.session.active(), Some(third));
    // Esc in Develop goes back to the Library grid
    exec(&mut h, "view.develop", json!({}));
    key(&mut h, "Escape", json!({}));
    assert_eq!((h.app.ui.module, h.app.ui.view), (Module::Library, ViewMode::PhotoGrid));
    // Develop tools (R) and the picker's menu item check marks
    key(&mut h, "r", json!({}));
    assert_eq!((h.app.ui.module, h.app.ui.right), (Module::Develop, RightPanel::Crop));
    key(&mut h, "r", json!({}));
    assert_eq!(h.app.ui.right, RightPanel::Edit, "the tool strip toggles back to the panels");
    assert_eq!(crate::menubar::checked(&h.app, "view.develop"), Some(true));
}

#[test]
fn panel_keys_hide_and_show_like_classic() {
    let mut h = demo([1400.0, 900.0]);
    exec(&mut h, "view.develop", json!({}));
    assert!(h.app.ui.left_panel && h.app.ui.right_panel && h.app.ui.filmstrip && h.app.ui.toolbar);
    key(&mut h, "Tab", json!({}));
    assert!(!h.app.ui.left_panel && !h.app.ui.right_panel, "Tab hides the side panels");
    key(&mut h, "Tab", json!({}));
    assert!(h.app.ui.left_panel && h.app.ui.right_panel);
    key(&mut h, "Tab", json!({"shift": true}));
    assert!(!h.app.ui.left_panel && !h.app.ui.right_panel && !h.app.ui.filmstrip && !h.app.ui.top_panel, "⇧Tab hides everything");
    key(&mut h, "Tab", json!({"shift": true}));
    assert!(h.app.ui.left_panel && h.app.ui.right_panel && h.app.ui.filmstrip && h.app.ui.top_panel);
    key(&mut h, "F7", json!({}));
    assert!(!h.app.ui.left_panel);
    key(&mut h, "F8", json!({}));
    assert!(!h.app.ui.right_panel);
    key(&mut h, "F6", json!({}));
    assert!(!h.app.ui.filmstrip);
    h.step();
    assert!(has(&h, "button:filmstripShow"), "a hidden filmstrip leaves its triangle");
    click(&mut h, "button:filmstripShow");
    assert!(h.app.ui.filmstrip);
    key(&mut h, "T", json!({}));
    assert!(!h.app.ui.toolbar);
    h.step();
    assert!(!has(&h, "icon:detail"), "no toolbar");
    // Lights Out cycles; Esc brings the lights back
    exec(&mut h, "view.lightsOut", json!({}));
    assert_eq!(h.app.ui.lights_out, LightsOut::Dim);
    exec(&mut h, "view.lightsOut", json!({}));
    assert_eq!(h.app.ui.lights_out, LightsOut::Off);
    h.step();
    key(&mut h, "Escape", json!({}));
    assert_eq!(h.app.ui.lights_out, LightsOut::On);
    assert_eq!(h.app.ui.module, Module::Develop, "Esc only ended Lights Out");
    let bad = h.request("engine.execute", json!({"command": "view.lightsOut", "params": {"mode": "disco"}}), T);
    assert_eq!(bad["ok"], false, "an unknown mode is an error, not a crash");
}

#[test]
fn filmstrip_names_its_source_filters_and_resizes() {
    let mut h = demo([1400.0, 900.0]);
    exec(&mut h, "view.library", json!({"view": "photoGrid"}));
    h.step();
    assert!(has(&h, "label:filmSource") && has(&h, "panel:filmstrip"));
    let n = h.app.session.visible_cloned().len();
    assert_eq!(crate::panels::filmstrip::counts_text(n, 1, Some("a.jpg")), format!("{n} photos / 1 selected / a.jpg"));
    // the quick filter: picks only, then off again
    click(&mut h, "button:filmFlag-pick");
    assert_eq!(h.app.session.filter.flag, Some(lightcraft_catalog::Flag::Pick));
    assert!(h.app.session.visible_cloned().len() < n);
    click(&mut h, "button:filmFlag-pick");
    assert_eq!(h.app.session.filter.flag, None);
    click(&mut h, "button:filmRating-3");
    assert_eq!(h.app.session.filter.rating, 3);
    click(&mut h, "button:filmFilterOff");
    assert_eq!(h.app.session.visible_cloned().len(), n);
    // drag the top edge up: taller, within its limits
    let before = h.app.ui.film_height;
    let edge = rect(&h, "edge:filmstrip");
    let r = h.request(
        "ui.drag",
        json!({"x": edge.center().x, "y": edge.center().y, "toX": edge.center().x, "toY": edge.center().y - 60.0, "steps": 8}),
        T,
    );
    assert_eq!(r["ok"], true, "{r}");
    assert!(h.app.ui.film_height > before + 30.0, "{} vs {before}", h.app.ui.film_height);
    exec(&mut h, "view.filmHeight", json!({"height": 10000.0}));
    assert_eq!(h.app.ui.film_height, crate::state::FILM_HEIGHT.max);
    exec(&mut h, "view.filmHeight", json!({"height": f64::NAN}));
    assert_eq!(h.app.ui.film_height, crate::state::FILM_HEIGHT.default, "a bad height is the default, never a panic");
}

#[test]
fn develop_panels_follow_classic_order_switches_and_solo() {
    let mut h = demo([1400.0, 2400.0]);
    exec(&mut h, "view.develop", json!({}));
    let ids: Vec<String> = crate::panels::edit::CLASSIC_PANELS.iter().map(|p| p.0.to_string()).collect();
    h.app.ui.open_sections.clear();
    h.step();
    let ys: Vec<f32> = ids.iter().map(|id| rect(&h, &format!("section:{id}")).top()).collect();
    assert!(
        ys.windows(2).all(|w| w[0] < w[1]),
        "Basic, Tone Curve, HSL / Color, Color Grading, Detail, Lens Corrections, Transform, Effects, Calibration: {ys:?}"
    );
    assert!(rect(&h, "section:histogram").top() < ys[0], "Histogram on top");
    assert!(has(&h, "icon:crop") && has(&h, "icon:remove") && has(&h, "icon:redeye") && has(&h, "icon:masking"), "the tool strip");
    assert!(has(&h, "button:developPrevious") && has(&h, "button:developReset"));
    // a click opens a panel, ⌥-click leaves only it open
    click(&mut h, "section:detail");
    click(&mut h, "section:effects");
    assert!(h.app.ui.section_open("detail") && h.app.ui.section_open("effects"));
    let r = h.request("ui.clickWidget", json!({"id": "section:toneCurve", "alt": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert!(h.app.ui.section_open("toneCurve") && !h.app.ui.section_open("detail") && !h.app.ui.section_open("effects"), "solo");
    // ⌘4 opens Color Grading; the old section names reach their Classic panel
    exec(&mut h, "section.colorGrading", json!({}));
    assert!(h.app.ui.section_open("colorGrading"));
    assert_eq!(exec(&mut h, "section.optics", json!({}))["section"], "lensCorrections");
    // HSL / Color's switch turns the mixer off without losing it
    let id = h.app.session.active().unwrap();
    exec(&mut h, "develop.set", json!({"values": {"mixer.red.hue": 40}}));
    exec(&mut h, "develop.sectionEnabled", json!({"section": "hsl", "enabled": false}));
    let d = h.app.session.develop_of(id).unwrap();
    assert!(!d.section_enabled("hsl"));
    assert_ne!(d.mixer, d.effective().mixer, "switched off: not rendered, but kept");
    // Copy… / Paste at the bottom of the left panel
    assert!(has(&h, "button:developCopy") && has(&h, "button:developPaste"));
    click(&mut h, "button:developCopy");
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::CopySettings { .. })));
}

#[test]
fn library_right_panel_and_sync_metadata() {
    let mut h = demo([1400.0, 1600.0]);
    exec(&mut h, "view.library", json!({"view": "photoGrid"}));
    h.step();
    for id in ["histogram", "quickDevelop", "keywording", "keywordList", "metadata"] {
        assert!(has(&h, &format!("section:lib.{id}")), "{id}");
    }
    assert!(has(&h, "button:syncMetadata") && has(&h, "button:syncSettings"));
    // Quick Develop: +⅓ stop on every selected photo
    let ids = h.app.session.visible_cloned();
    exec(&mut h, "library.select", json!({"ids": [ids[0].0, ids[1].0]}));
    let exp = |h: &Headless, i: usize| h.app.session.develop_of(ids[i]).unwrap().light.exposure;
    let (a, b) = (exp(&h, 0), exp(&h, 1));
    h.step();
    click(&mut h, "button:quick-light.exposure-r");
    assert!((exp(&h, 0) - a - 1.0 / 3.0).abs() < 0.02 && (exp(&h, 1) - b - 1.0 / 3.0).abs() < 0.02);
    // Sync Metadata: the active photo's title and keywords onto the other
    exec(&mut h, "library.select", json!({"ids": [ids[0].0]}));
    exec(&mut h, "photo.setMeta", json!({"title": "Ceremony", "addKeywords": ["BH WEDDING"]}));
    exec(&mut h, "library.select", json!({"ids": [ids[0].0, ids[1].0]}));
    assert_eq!(exec(&mut h, "library.syncMetadata", json!({}))["changed"], 1);
    let other = h.app.session.catalog.photo(ids[1]).unwrap().meta.clone();
    assert_eq!(other.title, "Ceremony");
    assert!(other.keywords.iter().any(|k| k == "BH WEDDING"));
    // with one photo it says what to do instead of failing silently
    exec(&mut h, "library.select", json!({"ids": [ids[0].0]}));
    let r = h.request("engine.execute", json!({"command": "library.syncMetadata"}), T);
    assert_eq!(r["ok"], false);
}

#[test]
fn import_window_adds_a_folder_and_shows_previous_import() {
    let mut h = demo([1500.0, 950.0]);
    let dir = std::env::temp_dir().join(format!("lc-import-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inner")).unwrap();
    let o = lightcraft_engine::export::ExportOptions { format: lightcraft_engine::export::ExportFormat::Jpeg, ..Default::default() };
    for (i, name) in ["a.jpg", "b.jpg", "inner/c.jpg"].iter().enumerate() {
        let img = lightcraft_raster::Rgba8::from_fn(32, 24, |x, y| [(x * 7) as u8, (y * 9) as u8, (i * 80) as u8, 255]);
        std::fs::write(dir.join(name), lightcraft_engine::export::encode_image(&img, &o).unwrap()).unwrap();
    }
    let before = h.app.session.catalog.len();
    // ⌘⇧I opens the window (no file picker)
    key(&mut h, "I", json!({"cmd": true, "shift": true}));
    h.step();
    let open = |h: &Headless| matches!(&h.app.ui.dialog, Some(crate::state::Dialog::Import { opts }) if opts.window);
    assert!(open(&h), "{:?}", h.app.ui.dialog);
    assert!(has(&h, "import:window") && has(&h, "button:importMode-add") && has(&h, "check:importSubfolders"));
    // the source: the folder without its subfolders, then with them
    exec(&mut h, "import.source", json!({"path": dir.to_string_lossy(), "subfolders": false}));
    h.settle(SETTLE);
    let cands = |h: &Headless| match &h.app.ui.dialog {
        Some(crate::state::Dialog::Import { opts }) => opts.candidates.len(),
        _ => 0,
    };
    assert_eq!(cands(&h), 2);
    exec(&mut h, "import.source", json!({"path": dir.to_string_lossy(), "subfolders": true}));
    h.settle(SETTLE);
    assert_eq!(cands(&h), 3);
    click(&mut h, "button:importMode-add");
    click(&mut h, "button:importShow-new");
    h.step();
    assert!(has(&h, "import:0"), "the grid shows the files");
    // Import: the window closes, the import runs, Previous Import shows the new photos
    click(&mut h, "button:dialogOk");
    assert!(h.app.ui.dialog.is_none(), "the window closes on Import");
    h.settle(SETTLE);
    assert_eq!(h.app.session.catalog.len(), before + 3);
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::PreviousImport);
    assert_eq!(h.app.session.visible_cloned().len(), 3);
    assert_eq!((h.app.ui.module, h.app.ui.view), (Module::Library, ViewMode::PhotoGrid));
    // the choices are remembered for the next import
    assert_eq!(h.app.ui.import_prefs.source, dir.to_string_lossy());
    assert_eq!(h.app.ui.import_prefs.mode, "add");
    // again: everything is already imported, shown dimmed and unchecked
    exec(&mut h, "file.addPhotos", json!({}));
    h.settle(SETTLE);
    let Some(crate::state::Dialog::Import { opts }) = &h.app.ui.dialog else { panic!("window") };
    assert_eq!(opts.candidates.len(), 3);
    assert!(opts.selected_paths().is_empty(), "duplicates start unchecked");
    // never-crash: a missing folder is an error the window shows
    let r = h.request("engine.execute", json!({"command": "import.source", "params": {"path": "/no/such/folder"}}), T);
    assert_eq!(r["ok"], false);
    key(&mut h, "Escape", json!({}));
    assert!(h.app.ui.dialog.is_none());
    let r = h.request("engine.execute", json!({"command": "import.source", "params": {"path": dir.to_string_lossy()}}), T);
    assert_eq!(r["ok"], false, "no window, no source");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saved_ui_state_from_before_the_modules_still_opens() {
    // an older ui.json: Edit sections by their earlier names, no module, a damaged film height
    let old: crate::UiState =
        serde_json::from_value(json!({"view": "squareGrid", "openSections": ["light", "color", "curve", "light"], "filmHeight": -5.0}))
            .expect("old ui.json reads");
    let u = old.sanitized();
    assert_eq!(u.open_sections, ["basic", "hsl", "toneCurve"]);
    assert_eq!((u.module, u.view), (Module::Library, ViewMode::SquareGrid));
    assert_eq!(u.film_height, crate::state::FILM_HEIGHT.min);
    // a Develop session saved in a grid view comes back in Library (never a Develop grid)
    let bad: crate::UiState = serde_json::from_value(json!({"module": "develop", "view": "photoGrid"})).expect("reads");
    let u = bad.sanitized();
    assert_eq!(u.module, Module::Library);
    // unknown values are an error for serde, so ui.json loading falls back — never a panic
    assert!(serde_json::from_value::<crate::UiState>(json!({"module": "map"})).is_err());
}
