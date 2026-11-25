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

// ---------------------------------------------------------------------------------------------
// Bryan 11-24: "my left off area is broken again" — what counts as a place worked at, which
// view is remembered, and that the module tabs never move the folder.

fn last(h: &Headless) -> crate::leftoff::Spot {
    h.app.ui.left_off.last.clone().expect("a left-off spot")
}

/// A user pick in All Photos, written down.
fn work_on(h: &mut Headless, id: lightcraft_catalog::PhotoId) {
    h.app.run("library.select", json!({"ids": [id.0]})).unwrap();
    h.step();
    h.step();
}

/// The merge. tab follows the sidebar and its view auto-selects: those frames are bookkeeping,
/// not work, and never replace the spot (A).
#[test]
fn visits_to_the_merge_tab_do_not_overwrite_left_off() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0]}));
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    let in_a = h.app.session.visible_cloned();
    work_on(&mut h, in_a[2]);
    let before = last(&h);
    assert_eq!((before.source.as_str(), before.photo), (format!("album:{a}").as_str(), in_a[2].0));
    // the merge. tab on, its scope following the sidebar; a photo in view is put up for it by the
    // engine in the very frame the scope changes (nobody clicked it)
    h.app.run("view.merges", json!({})).unwrap();
    h.app.session.selection = lightcraft_engine::Selection::single(all[5]);
    h.step();
    h.app.run("library.source", json!({"kind": "all"})).unwrap();
    h.app.session.selection = lightcraft_engine::Selection::single(all[4]);
    h.step();
    h.step();
    h.settle(SETTLE);
    assert!(h.app.merge_tab);
    assert_eq!(last(&h), before, "merge. bookkeeping didn't move the spot");
}

/// A merges. spot (saved before this fix, or picked in the merge. page) comes back with the
/// merge. tab on and its grid, not as a half-restored merges view (A).
#[test]
fn a_restored_merges_spot_opens_the_merge_tab() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    let lib = h.app.session.library.as_ref().map(|l| l.dir.to_string_lossy().to_string()).unwrap_or_else(|| "memory".into());
    h.app.ui.left_off.last = Some(crate::leftoff::Spot {
        library: lib,
        source: "merges:{\"kind\":\"all\"}".into(),
        photo: all[2].0,
        view: "detail".into(),
        at: 5,
    });
    h.app.run("view.resumeLastLeftOff", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(matches!(h.app.session.source, lightcraft_engine::LibrarySource::Merges), "{:?}", h.app.session.source);
    assert!(h.app.merge_tab, "the merge. tab is on");
    assert!(matches!(h.app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid), "{:?}", h.app.ui.view);
    assert!(crate::merge_page::on(&h.app), "the merge. page shows");
}

/// Opening a folder lands on its first photo by itself; that is not work (C). Neither is the
/// photo Develop picks when none is active, nor the launch photo.
#[test]
fn a_fallback_photo_is_not_a_spot() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    assert!(h.app.ui.left_off.last.is_none(), "the launch photo isn't a spot");
    let all = h.app.session.visible_cloned();
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0]}));
    work_on(&mut h, all[4]);
    let before = last(&h);
    assert_eq!(before.photo, all[4].0);
    // a folder opened and looked at: the engine puts its first photo up
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    h.step();
    h.step();
    assert_eq!(last(&h), before, "opening a collection isn't work");
    assert!(!h.app.ui.left_off.per_source.contains_key(&format!("memory|album:{a}")), "nor a spot of its own");
    // Develop with nothing active picks the first photo in view
    h.app.session.selection = lightcraft_engine::Selection::default();
    exec(&mut h, "view.develop", json!({}));
    h.step();
    h.step();
    assert!(h.app.session.active().is_some());
    assert_eq!(last(&h), before, "the photo Develop picked isn't work");
    // a click on a photo is
    let in_a = h.app.session.visible_cloned();
    work_on(&mut h, in_a[1]);
    assert_eq!((last(&h).photo, last(&h).view.as_str()), (in_a[1].0, "edit"));
}

/// The view is recorded as it is: square grid, compare, survey, people, the map and galleries
/// are not all "detail" (D); and they come back.
#[test]
fn views_are_recorded_faithfully_and_come_back() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    work_on(&mut h, all[2]);
    for (cmd, params, name, view) in [
        ("view.library", json!({"view": "squareGrid"}), "square", ViewMode::SquareGrid),
        ("view.library", json!({"view": "compare"}), "compare", ViewMode::Compare),
        ("view.library", json!({"view": "survey"}), "survey", ViewMode::Survey),
        ("view.library", json!({"view": "people"}), "people", ViewMode::People),
        ("view.map", json!({}), "map", ViewMode::Map),
        ("view.galleries", json!({}), "galleries", ViewMode::Galleries),
        ("view.library", json!({"view": "detail"}), "detail", ViewMode::Detail),
        ("view.develop", json!({}), "edit", ViewMode::Detail),
    ] {
        exec(&mut h, cmd, params);
        h.step();
        h.step();
        assert_eq!(last(&h).view, name, "{cmd}");
        // away (somewhere that isn't a place to go back to) and back
        exec(&mut h, "library.source", json!({"kind": "recentlyDeleted"}));
        exec(&mut h, "view.library", json!({"view": "photoGrid"}));
        h.step();
        assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);
        h.app.run("view.resumeLastLeftOff", json!({})).unwrap();
        h.step();
        assert_eq!(h.app.ui.view, view, "{name} comes back");
        assert_eq!(h.app.session.active(), Some(all[2]));
        // (leaving the map / galleries / develop by its own tab records the grid again)
        exec(&mut h, "view.library", json!({"view": "photoGrid"}));
        h.step();
        h.step();
    }
}

/// Resuming into Develop and pressing library. gives the Library back: its grid or loupe, its
/// panels and toolbar (B). And a saved Develop state with Settings ▸ start in grid starts in the
/// Library grid, not in Develop.
#[test]
fn library_after_resuming_into_develop_is_the_library() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    work_on(&mut h, all[2]);
    exec(&mut h, "view.develop", json!({}));
    h.step();
    h.step();
    assert_eq!(last(&h).view, "edit");
    // quit in Develop; launch with start in = grid
    let mut ui: crate::UiState = serde_json::from_str(&serde_json::to_string(&h.app.ui).unwrap()).unwrap();
    ui.settings.startup_view = StartupView::Grid;
    let ui = ui.sanitized();
    let session = std::mem::replace(&mut h.app.session, lightcraft_engine::Session::with_demo());
    let mut n = app(session);
    n.app.ui = ui;
    n.app.resume_left_off_at_launch();
    n.settle(SETTLE);
    assert_eq!((n.app.ui.module, n.app.ui.view), (Module::Library, ViewMode::PhotoGrid), "start in grid wins");
    assert_eq!(n.app.session.active(), Some(all[2]));
    // the spot itself is untouched by it; resuming by hand goes to Develop, library. back to the grid
    n.app.run("view.resumeLastLeftOff", json!({})).unwrap();
    n.step();
    assert_eq!((n.app.ui.module, n.app.ui.view), (Module::Develop, ViewMode::Detail));
    exec(&mut n, "view.library", json!({}));
    n.step();
    n.step();
    assert_eq!((n.app.ui.module, n.app.ui.view), (Module::Library, ViewMode::PhotoGrid));
    for id in ["module:library", "module:develop", "module:merge", "module:map"] {
        assert!(n.app.widgets.iter().any(|(w, _)| w == id), "top bar: {id}");
    }
    assert!(n.app.widgets.iter().any(|(w, _)| w.starts_with("source:")), "Library's side panel, not Develop's");
}

// ---- a subfolder stays the subfolder (Bryan 11-24: "it always brings me back to the main folder")

const GONE: &str = "/Volumes/LC-leftoff-test-unplugged";

fn shoot_app() -> Headless {
    use lightcraft_catalog::{Op, Photo, Source};
    let mut session = lightcraft_engine::Session::new();
    for (i, path) in [
        format!("{GONE}/C&J WEDDING/M262/a.jpg"),
        format!("{GONE}/C&J WEDDING/M262/b.jpg"),
        format!("{GONE}/C&J WEDDING/griii/c.jpg"),
        format!("{GONE}/C&J WEDDING/griii/d.jpg"),
        format!("{GONE}/Other shoot/e.jpg"),
    ]
    .iter()
    .enumerate()
    {
        let id = session.catalog.alloc_photo_id();
        let p = Photo::new(id, Source::File { path: path.clone() }, "x.jpg", "JPEG", 60, 40, &format!("2026-0{}-01T10:00:00", 1 + i));
        session.catalog.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    let mut h = app(session);
    h.settle(SETTLE);
    h
}

/// Working in a subfolder and going through every module tab and key never lands on the shoot
/// around it, with or without the merge. tab.
#[test]
fn module_tabs_keep_the_subfolder() {
    let mut h = shoot_app();
    let sub = format!("{GONE}/C&J WEDDING/M262");
    let shoot = format!("{GONE}/C&J WEDDING");
    exec(&mut h, "library.source", json!({"kind": "libraryFolder", "path": sub}));
    let in_sub = h.app.session.visible_cloned();
    assert_eq!(in_sub.len(), 2, "the subfolder's own photos");
    work_on(&mut h, in_sub[1]);
    let check = |h: &Headless, step: &str| {
        assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::LibraryFolder, "{step}: {:?}", h.app.session.source);
        assert_eq!(h.app.session.library_folder.as_deref(), Some(sub.as_str()), "{step}");
        assert_eq!(h.app.session.active(), Some(in_sub[1]), "{step}");
        assert_ne!(h.app.session.library_folder.as_deref(), Some(shoot.as_str()), "{step}: not the main folder");
    };
    for round in 0..2 {
        for cmd in ["view.develop", "view.library", "view.merges", "view.map", "view.library", "view.galleries", "view.library", "view.develop", "view.merges", "view.develop", "view.library"] {
            let r = h.request("engine.execute", json!({"command": cmd, "params": {}}), T);
            assert_eq!(r["ok"], true, "{cmd}: {r}");
            h.settle(SETTLE);
            if matches!(cmd, "view.library") {
                check(&h, &format!("round {round} after {cmd}"));
                assert!(!h.app.merge_tab);
            }
        }
        // the tab clicks too
        for id in ["module:merge", "module:library", "module:develop", "module:map", "module:library"] {
            let r = h.request("ui.clickWidget", json!({"id": id}), T);
            assert_eq!(r["ok"], true, "{id}: {r}");
            h.settle(SETTLE);
        }
        check(&h, "after the tab clicks");
    }
    // and the spot the merge. tab visits left behind is this subfolder's
    let spot = last(&h);
    assert!(spot.source.ends_with("M262"), "{}", spot.source);
    assert_eq!(spot.photo, in_sub[1].0);
    // resuming from the shoot goes back into the subfolder
    exec(&mut h, "library.source", json!({"kind": "libraryFolder", "path": shoot, "subfolders": true}));
    h.step();
    h.app.run("view.resumeLastLeftOff", json!({})).unwrap();
    h.settle(SETTLE);
    assert_eq!(h.app.session.library_folder.as_deref(), Some(sub.as_str()));
    assert_eq!(h.app.session.active(), Some(in_sub[1]));
}

// ---- review fixes (11-24): stale user_select, adopting the default photo, edit + source change

/// A select that changes nothing (the active photo clicked again) must not leave "the user picked"
/// up for a later source change: the next folder's auto-selected photo is not a spot.
#[test]
fn a_no_op_select_does_not_make_the_next_folders_default_photo_a_spot() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0]}));
    work_on(&mut h, all[4]);
    let before = last(&h);
    // clicking the active photo again: nothing changes, the flag must not outlive the frame
    h.app.run("library.select", json!({"ids": [all[4].0]})).unwrap();
    h.step();
    h.step();
    assert!(!h.app.left_off.user_select, "taken even when nothing changed");
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    h.step();
    h.step();
    assert_eq!(last(&h), before, "the album's first photo is a default, not work");
    assert!(!h.app.ui.left_off.per_source.contains_key(&format!("memory|album:{a}")));
}

/// Clicking the default photo of a freshly opened folder is work, and stays counted when the
/// view changes to the loupe.
#[test]
fn picking_the_default_photo_counts_and_stays_counted() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0]}));
    work_on(&mut h, all[4]);
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    h.step();
    h.step();
    let first = h.app.session.active().expect("the album's default photo");
    assert_eq!(last(&h).photo, all[4].0, "still the old spot");
    // click the photo that is already up
    h.app.run("library.select", json!({"ids": [first.0]})).unwrap();
    h.step();
    h.step();
    assert_eq!((last(&h).photo, last(&h).source.clone()), (first.0, format!("album:{a}")), "now it is a spot");
    // and the loupe keeps it
    exec(&mut h, "view.library", json!({"view": "detail"}));
    h.step();
    h.step();
    assert_eq!((last(&h).photo, last(&h).view.clone()), (first.0, "detail".to_string()));
}

/// An edit in the very frame the source changes is recorded.
#[test]
fn an_edit_in_the_frame_of_a_source_change_is_recorded() {
    let mut h = app(lightcraft_engine::Session::with_demo());
    h.settle(SETTLE);
    let all = h.app.session.visible_cloned();
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[1].0, all[2].0, all[3].0]}));
    work_on(&mut h, all[4]);
    h.app.run("library.source", json!({"kind": "album", "id": a})).unwrap();
    let id = h.app.session.active().expect("a photo up in the album");
    let depth = h.app.session.undo.len();
    // (no frame between the source change and the edit)
    h.app.session.execute("photo.rate", &json!({"ids": [id.0], "rating": 3})).unwrap();
    assert!(h.app.session.undo.len() > depth, "the edit is undoable");
    h.step();
    h.step();
    assert_eq!((last(&h).photo, last(&h).source.clone()), (id.0, format!("album:{a}")), "edited there: a spot");
}
