//! Library ▸ shoots. and the tidied Folders section, headless.
//!
//! Scenarios, in Bryan's words ("my photo work centrally located, not just in drives"):
//!
//! * Every shoot is one row under Catalog, whatever disk it is on, named by its folder (camera
//!   folders like `raw/M262` climb to the shoot), and a click shows all of its photos.
//! * A shoot on a disk that is not plugged in is quiet; one that is there is not.
//! * Right-click ▸ import. opens the Import window on that folder, adding in place.
//! * pin. puts a shoot on top, remove. takes it off the list (sort. ▸ removed. puts it back).
//! * Folders lists only imported folders, a camera root folder named by its shoot; a folder
//!   opened by hand stays open.
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes the panel's screenshots there.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

/// A disk that is never plugged in.
const GONE: &str = "/Volumes/LC-shoots-test-unplugged";

/// Two shoots on a disk that is there (a temporary folder) and four camera folders of two shoots
/// on one that is not.
struct Fixture {
    h: Headless,
    here: String,
    _dir: TempDir,
}

struct TempDir(std::path::PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture(name: &str) -> Fixture {
    use lightcraft_catalog::{Op, Photo, Source};
    let dir = std::env::temp_dir().join(format!("lc-shoots-{}-{name}", std::process::id()));
    let here = dir.to_string_lossy().to_string();
    for sub in ["Shoot 2/photos", "Shoot 3"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    let paths = [
        format!("{GONE}/Erika and Connor wedding/raw/M262/a.jpg"),
        format!("{GONE}/Erika and Connor wedding/raw/M262/b.jpg"),
        format!("{GONE}/Erika and Connor wedding/raw/GR3/c.jpg"),
        format!("{GONE}/C&J WEDDING/X-T2 raw/d.jpg"),
        format!("{GONE}/C&J WEDDING/M262/e.jpg"),
        format!("{GONE}/Gabriella Maternity/M262/f.jpg"),
        format!("{here}/Shoot 2/photos/g.jpg"),
        format!("{here}/Shoot 3/h.jpg"),
    ];
    let mut session = lightcraft_engine::Session::new();
    for (i, path) in paths.iter().enumerate() {
        let id = session.catalog.alloc_photo_id();
        let imported = format!("2026-0{}-01T10:00:00", 1 + i % 8);
        let p = Photo::new(id, Source::File { path: path.clone() }, "x.jpg", "JPEG", 60, 40, &imported);
        session.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    let png = |img: &lightcraft_raster::Rgba8| {
        lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
    };
    let services = Services {
        png: Some(Box::new(png)),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        reveal: Some(Box::new(|_: &str| Ok(()))),
        pick_folder: Some(Box::new(|| None)),
        ..Default::default()
    };
    let app = LightcraftApp::new(session, services);
    let mut h = Headless::new(app, [1400.0, 1000.0], 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    // Navigator folded so every shoot and folder fits
    h.app.ui.toggle_sidebar_section("navigator");
    h.settle(SETTLE);
    Fixture { h, here: here.clone(), _dir: TempDir(dir) }
}

fn has(h: &Headless, id: &str) -> bool {
    h.app.widgets.iter().any(|(w, _)| w == id)
}

fn widget(h: &Headless, id: &str) -> egui::Rect {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r).unwrap_or_else(|| panic!("no widget {id}"))
}

fn click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

fn right_click(h: &mut Headless, id: &str) {
    let c = widget(h, id).center();
    let r = h.request("ui.click", json!({"x": c.x, "y": c.y, "button": "right"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    h.step();
}

fn shot(h: &mut Headless, name: &str) {
    if let Some(dir) = std::env::var_os("LIGHTCRAFT_TEST_SHOTS") {
        let path = std::path::Path::new(&dir).join(name);
        let r = h.request("ui.screenshot", json!({"path": path.to_string_lossy(), "headless": true}), T);
        assert_eq!(r["ok"], true, "{r}");
    }
}

/// The shoot rows top to bottom.
fn shoot_order(h: &Headless) -> Vec<String> {
    let mut v: Vec<(f32, String)> =
        h.app.widgets.iter().filter_map(|(w, r)| w.strip_prefix("source:shoot:").map(|p| (r.top(), p.to_string()))).collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, p)| p).collect()
}

#[test]
fn every_shoot_is_one_row_named_by_its_folder_and_shows_all_its_photos() {
    let Fixture { mut h, here, .. } = fixture("rows");
    shot(&mut h, "shoots-1-panel.png");
    let erika = format!("{GONE}/Erika and Connor wedding");
    // newest import first: Shoot 3 (Aug), Shoot 2 (Jul), Gabriella (Jun), C&J (May), Erika (Mar)
    assert_eq!(
        shoot_order(&h),
        vec![
            format!("{here}/Shoot 3"),
            format!("{here}/Shoot 2"),
            format!("{GONE}/Gabriella Maternity"),
            format!("{GONE}/C&J WEDDING"),
            erika.clone()
        ]
    );
    assert!(
        !h.app.widgets.iter().any(|(w, _)| w.starts_with("source:shoot:") && (w.ends_with("/raw") || w.contains("M262"))),
        "camera folders are no shoots"
    );
    // shoots sit between Catalog and Folders, rows one height, 8 px or more from what is next to them
    let (catalog, shoots, folders) =
        (widget(&h, "sidebarSection:catalog"), widget(&h, "sidebarSection:shoots"), widget(&h, "sidebarSection:folders"));
    assert!(catalog.top() < shoots.top() && shoots.top() < folders.top());
    for p in shoot_order(&h) {
        let id = format!("shoot:{p}");
        let row = widget(&h, &format!("source:{id}"));
        assert!((row.height() - 29.0).abs() < 0.5, "{id}");
        let (label, dot, count) = (widget(&h, &format!("label:{id}")), widget(&h, &format!("dot:{id}")), widget(&h, &format!("count:{id}")));
        assert!(dot.left() - label.right() >= 8.0 - 0.5 && count.left() - dot.right() >= 8.0 - 0.5, "{id}: name · dot · count apart");
    }
    // the disk that is not plugged in: its shoots are quiet, the others are not
    assert!(has(&h, &format!("dim:shoot:{erika}")));
    assert!(!has(&h, &format!("dim:shoot:{here}/Shoot 2")));
    // a click shows the whole shoot, even with Show Photos in Subfolders off
    h.request("engine.execute", json!({"id": "library.showSubfolders", "params": {"on": false}}), T);
    click(&mut h, &format!("source:shoot:{erika}"));
    assert_eq!(h.app.session.visible().len(), 3, "both camera folders");
    assert!(has(&h, &format!("highlight:shoot:{erika}")));
    assert!(!h.app.widgets.iter().any(|(w, _)| w.starts_with("highlight:libfolder:")), "the shoot is chosen, not a folder");
    shot(&mut h, "shoots-2-chosen.png");
}

#[test]
fn import_from_a_shoot_opens_the_import_window_on_its_folder() {
    let Fixture { mut h, here, .. } = fixture("import");
    let shoot2 = format!("{here}/Shoot 2");
    right_click(&mut h, &format!("source:shoot:{shoot2}"));
    shot(&mut h, "shoots-3-menu.png");
    click(&mut h, "menu:import");
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Import { opts }) => {
            assert!(opts.window);
            assert_eq!(opts.prefs.source, shoot2);
            assert_eq!(opts.prefs.mode, "add", "adds in place by default");
        }
        other => panic!("no Import window: {other:?}"),
    }
    h.settle(SETTLE);
    shot(&mut h, "shoots-4-import-window.png");
    assert!(has(&h, "button:importBrowse"), "Browse Folder Without Importing lives in the Import window now");
    h.request("ui.key", json!({"key": "escape"}), T);
    h.step();
    // a shoot on a disk that is not there offers no import
    right_click(&mut h, &format!("source:shoot:{GONE}/C&J WEDDING"));
    assert!(!has(&h, "menu:import"));
}

#[test]
fn pin_puts_a_shoot_on_top_and_remove_takes_it_off_the_list() {
    let Fixture { mut h, here, .. } = fixture("pin");
    let erika = format!("{GONE}/Erika and Connor wedding");
    right_click(&mut h, &format!("source:shoot:{erika}"));
    click(&mut h, "menu:pin");
    h.step();
    assert_eq!(shoot_order(&h).first(), Some(&erika));
    assert_eq!(h.app.session.shoot_prefs.pinned, vec![erika.clone()]);
    let gap = widget(&h, &format!("source:shoot:{here}/Shoot 3")).top() - widget(&h, &format!("source:shoot:{erika}")).bottom();
    assert!(gap >= 8.0 - 0.5, "the pinned shoots stand apart ({gap})");
    shot(&mut h, "shoots-5-pinned.png");
    right_click(&mut h, &format!("source:shoot:{here}/Shoot 3"));
    click(&mut h, "menu:remove");
    h.step();
    assert!(!has(&h, &format!("source:shoot:{here}/Shoot 3")));
    assert_eq!(h.app.session.catalog.photos().count(), 8, "no photo left the library");
    let r = h.request("engine.execute", json!({"id": "library.shoots", "params": {"hidden": true}}), T);
    assert!(r["result"]["shoots"].as_array().unwrap().iter().any(|s| s["hidden"] == true), "{r}");
}

#[test]
fn folders_list_only_imported_folders_named_by_their_shoot() {
    let Fixture { mut h, here, .. } = fixture("folders");
    // the unplugged disk's root folders, each a row of its own; nothing else of the disk
    let gone_roots: Vec<String> = h
        .app
        .widgets
        .iter()
        .filter_map(|(w, _)| w.strip_prefix("source:libfolder:").filter(|p| p.starts_with(GONE) && *p != GONE).map(str::to_string))
        .collect();
    assert_eq!(
        gone_roots,
        vec![format!("{GONE}/C&J WEDDING"), format!("{GONE}/Erika and Connor wedding/raw"), format!("{GONE}/Gabriella Maternity/M262")]
    );
    assert_eq!(lightcraft_catalog::shoots::context_label(&format!("{GONE}/Gabriella Maternity/M262"), "M262"), "Gabriella Maternity › M262");
    // a folder opened by hand stays open (kept in the UI state)
    let cj = format!("{GONE}/C&J WEDDING");
    assert!(!has(&h, &format!("source:libfolder:{cj}/M262")), "closed until opened");
    click(&mut h, &format!("libraryFolderToggle:{cj}"));
    assert!(has(&h, &format!("source:libfolder:{cj}/M262")));
    assert_eq!(h.app.ui.folders_open.get(&lightcraft_catalog::query::folder_key(&cj)), Some(&true));
    let back: crate::state::UiState = serde_json::from_value(serde_json::to_value(&h.app.ui).unwrap()).unwrap();
    assert_eq!(back.folders_open.get(&lightcraft_catalog::query::folder_key(&cj)), Some(&true), "survives a restart");
    // the disk rows carry the same colour dot as their shoots
    assert!(has(&h, &format!("dot:libfolder:{GONE}")));
    let _ = here;
    shot(&mut h, "shoots-6-folders.png");
}
