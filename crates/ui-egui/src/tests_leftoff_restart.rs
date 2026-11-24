//! "Always actively remember where we left off" (Bryan, 2026-11-23): the spot is due for
//! `ui.json` about a second after it changes, and a restart goes back to it — the album or folder,
//! the photo and the view — instead of the first photo.

use std::time::Duration;

use serde_json::json;

use crate::headless::Headless;
use crate::state::{Module, StartupView, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(120);

fn app(session: lightcraft_engine::Session) -> Headless {
    let mut app = LightcraftApp::new(session, Services { png: None, ..Default::default() });
    app.fix_notes = crate::fixnotes::FixNotes::default();
    Headless::new(app, [1400.0, 900.0], 1.0)
}

fn exec(h: &mut Headless, c: &str, p: serde_json::Value) -> serde_json::Value {
    let r = h.request("engine.execute", json!({"command": c, "params": p}), T);
    assert_eq!(r["ok"], true, "{c}: {r}");
    h.settle(SETTLE);
    r["result"].clone()
}

fn now(h: &Headless) -> f64 {
    h.view.ctx.input(|i| i.time)
}

/// Quit with the session's catalog kept (as the library on disk would be) and the UI state saved
/// and read back; the next launch starts on the first photo of All Photos in the grid, as the
/// engine restores it, until the left-off spot is resumed.
fn restart(h: &mut Headless, startup: StartupView) -> Headless {
    let mut ui: crate::UiState = serde_json::from_str(&serde_json::to_string(&h.app.ui).unwrap()).unwrap();
    ui.settings.startup_view = startup;
    let ui = ui.sanitized();
    let session = std::mem::replace(&mut h.app.session, lightcraft_engine::Session::with_demo());
    let mut n = app(session);
    n.app.ui = ui;
    n.app.ui.module = Module::Library;
    n.app.ui.view = ViewMode::PhotoGrid;
    // (straight on the session: no frame has run yet at launch)
    n.app.session.execute("library.source", &json!({"kind": "all"})).unwrap();
    let first = n.app.session.visible_cloned()[0];
    n.app.session.execute("library.select", &json!({"ids": [first.0]})).unwrap();
    n
}

#[test]
fn a_restart_goes_back_to_where_i_left_off() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    assert!(all.len() >= 6);
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0, all[4].0]}));
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    let in_a = h.app.session.visible_cloned();
    exec(&mut h, "view.develop", json!({}));
    // (what earlier frames recorded is written; then one change, one frame)
    h.app.left_off.save_due(now(&h) + 10.0);
    h.app.run("library.select", json!({"ids": [in_a[2].0]})).unwrap();
    h.step();
    let t = now(&h);
    let spot = h.app.ui.left_off.last.clone().expect("recorded");
    assert_eq!((spot.source.as_str(), spot.photo, spot.view.as_str()), (format!("album:{a}").as_str(), in_a[2].0, "edit"));
    // written promptly: due about a second after the change, once
    assert!(h.app.left_off.unsaved());
    assert!(!h.app.left_off.save_due(t), "not at once (debounced)");
    h.advance_time(1.2);
    h.step();
    assert!(h.app.left_off.save_due(now(&h)), "due a second later");
    assert!(!h.app.left_off.save_due(now(&h) + 5.0), "and only once");
    // quit (app.quit over the control channel closes the window; the app writes ui.json then)
    let r = h.request("app.quit", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert!(h.quit_requested());
    // launch: back in the album, on its 3rd photo, in Develop
    let mut n = restart(&mut h, StartupView::Last);
    assert_eq!(n.app.session.active(), Some(all[0]), "the engine's launch spot");
    n.app.resume_left_off_at_launch();
    n.settle(SETTLE);
    assert_eq!(n.app.session.source, lightcraft_engine::LibrarySource::Album(lightcraft_catalog::AlbumId(a)));
    assert_eq!(n.app.session.active(), Some(in_a[2]), "the photo worked on last");
    assert_eq!((n.app.ui.module, n.app.ui.view), (Module::Develop, ViewMode::Detail), "and the view it was in");
    assert_eq!(n.app.ui.left_off.last.as_ref().map(|s| s.photo), Some(in_a[2].0), "launching didn't move the spot");
    // the spot survives this second session untouched, and Library ▸ loupe is remembered too
    exec(&mut n, "library.select", json!({"ids": [in_a[3].0]}));
    exec(&mut n, "view.library", json!({"view": "detail"}));
    n.step();
    assert_eq!(n.app.ui.left_off.last.as_ref().map(|s| (s.photo, s.view.clone())), Some((in_a[3].0, "detail".to_string())));
    let mut m = restart(&mut n, StartupView::Last);
    m.app.resume_left_off_at_launch();
    m.settle(SETTLE);
    assert_eq!(m.app.session.active(), Some(in_a[3]));
    assert_eq!((m.app.ui.module, m.app.ui.view), (Module::Library, ViewMode::Detail));
    // Settings ▸ start in grid: the photo and album come back, the grid stays
    let mut g = restart(&mut m, StartupView::Grid);
    g.app.resume_left_off_at_launch();
    g.settle(SETTLE);
    assert_eq!(g.app.session.active(), Some(in_a[3]));
    assert_eq!(g.app.session.source, lightcraft_engine::LibrarySource::Album(lightcraft_catalog::AlbumId(a)));
    assert_eq!(g.app.ui.view, ViewMode::PhotoGrid);
}

/// Nothing to go back to (a first launch, or another library's spot): the launch view stays.
#[test]
fn a_first_launch_has_nothing_to_resume() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.app.ui.view = ViewMode::PhotoGrid;
    h.app.resume_left_off_at_launch();
    h.settle(SETTLE);
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);
    let mut other = app(lightcraft_engine::Session::with_demo());
    other.app.ui.left_off.last =
        Some(crate::leftoff::Spot { library: "/elsewhere".into(), source: "all".into(), photo: 999_999, view: "edit".into(), at: 1 });
    other.app.ui.view = ViewMode::PhotoGrid;
    other.app.resume_left_off_at_launch();
    other.settle(SETTLE);
    assert_eq!(other.app.ui.view, ViewMode::PhotoGrid, "another library's spot is left alone");
}
