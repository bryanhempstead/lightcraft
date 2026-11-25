//! The merge. page, headless (Bryan's work order 2026-11-24, items 2 and 3, and his layout. /
//! aspect. request): three columns — the sidebar (open merges counted on the shoot), the waves
//! with their filters, the picked group on the right with the Merge dialog's options inline —,
//! the page's keys, the layouts and card shapes, and Settings ▸ merge. (stored in ui.json, used by
//! the merges).
//!
//! `LIGHTCRAFT_TEST_SHOTS=<dir>` writes screenshots there.

use std::time::Duration;

use lightcraft_catalog::MergeType;
use lightcraft_engine::merge_find::{self, GroupState, MergeGroup};
use serde_json::json;

use crate::headless::Headless;
use crate::tests_merges::{SETTLE, T, cards, click, has, rect_of, shot, wave_fixture, widget};

fn key(h: &mut Headless, k: &str, cmd: bool) {
    let r = h.request("ui.key", json!({"key": k, "cmd": cmd}), T);
    assert_eq!(r["ok"], true, "{k}: {r}");
    h.settle(SETTLE);
}

/// The wave fixture on the merge. page, its shoot picked: (pano group, hdr group, shoot path).
fn page(tag: &str) -> (Headless, std::path::PathBuf, MergeGroup, MergeGroup) {
    let (mut h, dir) = wave_fixture(tag);
    assert!(h.step_until(Duration::from_secs(60), |h| merge_find::groups(&h.app.session.catalog).len() == 2), "found both");
    let gs = merge_find::groups(&h.app.session.catalog);
    let pano = gs.iter().find(|g| g.kind == MergeType::Pano).expect("the panorama").clone();
    let hdr = gs.iter().find(|g| g.kind == MergeType::Hdr).expect("the bracket").clone();
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": dir.to_string_lossy()})).unwrap();
    let r = h.request("ui.clickWidget", json!({"id": "module:merge"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(crate::merge_page::on(&h.app), "the merge. page");
    h.app.run("library.select", json!({"ids": []})).unwrap();
    h.settle(SETTLE);
    (h, dir, pano, hdr)
}

/// Everything on the page laid out: nothing clipped, no text on another, in the centre and on
/// the right.
fn audit(h: &Headless, what: &str) {
    let mut faults = crate::tests_panels::layout_faults(h, h.app.canvas_rect.expect("the centre"));
    faults.extend(crate::tests_panels::layout_faults(h, widget(h, "mergePage:right")));
    assert!(faults.is_empty(), "{what}: {faults:#?}");
}

/// The group order the view shows.
fn order(h: &Headless) -> Vec<String> {
    crate::merges::compute_view_data(&h.app).open.iter().map(|g| g.g.id.clone()).collect()
}

#[test]
fn the_merge_page_is_three_columns_with_the_picked_group_on_the_right() {
    let (mut h, dir, pano, hdr) = page("columns");
    let shoot = dir.to_string_lossy().to_string();
    // left: the sidebar, the shoot chosen and counting its open merges
    let shoot_row = format!("shoot:{shoot}");
    assert!(has(&h, &format!("highlight:{shoot_row}")) || has(&h, &format!("source:{shoot_row}")), "the shoot row");
    let count = widget(&h, &format!("count:{shoot_row}"));
    let texts = h.view.visible_texts();
    assert!(texts.iter().any(|(s, r, _)| s == "2" && count.expand(2.0).contains(r.center())), "2 open merges on the shoot: {count:?}");
    // centre: the bar, the filters, the groups; right: the panel
    for w in ["merges:bar", "mergeFilter:kind", "mergeFilter:confidence", "mergeFilter:camera", "mergePage:right", "mergePage:footer", "button:mergePageMerge", "button:mergePageSendBack"] {
        assert!(has(&h, w), "{w}");
    }
    let (canvas, right) = (h.app.canvas_rect.unwrap(), widget(&h, "mergePage:right"));
    assert!(right.left() >= canvas.right() - 1.0, "the panel right of the waves");
    assert!(widget(&h, &format!("group:merges:group:{}", pano.id)).right() <= canvas.right());
    assert!(!has(&h, "section:lib.histogram"), "not the Library right panel");
    assert!(has(&h, "label:mergePageNone"), "nothing picked yet");
    audit(&h, "nothing picked");
    shot(&mut h, "mergepage-1-none.png");

    // → picks the first group: the right panel shows it, its frames large, its options inline
    let first = order(&h);
    assert_eq!(first.len(), 2);
    key(&mut h, "right", false);
    let g0 = merge_find::group(&h.app.session.catalog, &first[0]).unwrap();
    assert_eq!(h.app.session.selection.ids, g0.photos, "→: the first group");
    for f in &g0.photos {
        let c = widget(&h, &format!("mergePageCard:{}:{}", g0.id, f.0));
        assert!(right.contains_rect(c.shrink(0.5)) || c.top() > right.bottom() - 50.0, "in the panel {c:?}");
        assert!(c.width() >= 120.0, "large: {c:?}");
    }
    assert!(has(&h, &format!("button:mergePageKind-{}", g0.kind.id())));
    assert!(has(&h, "check:mergePage.autoSettings") && has(&h, "check:mergePage.stack"), "the options inline");
    if g0.kind == MergeType::Pano {
        assert!(has(&h, "button:mergePageProjection-0"));
    } else {
        assert!(has(&h, "button:mergePageDeghost-0"));
    }
    audit(&h, "a group picked");
    shot(&mut h, "mergepage-2-picked.png");
    // → the next, → stays at the end, ← back
    key(&mut h, "right", false);
    assert_eq!(h.app.session.selection.ids, merge_find::group(&h.app.session.catalog, &first[1]).unwrap().photos);
    key(&mut h, "right", false);
    assert_eq!(h.app.session.selection.ids, merge_find::group(&h.app.session.catalog, &first[1]).unwrap().photos, "stays on the last");
    key(&mut h, "left", false);
    assert_eq!(h.app.session.selection.ids, g0.photos);
    // ⌘A: every group
    key(&mut h, "A", true);
    assert_eq!(h.app.session.selection.ids.len(), 8);
    key(&mut h, "left", false);

    // a frame's ⊖ in the panel asks; out.of.group. takes it out, put.back. puts it back
    let out = g0.photos[1];
    click(&mut h, &format!("mergePageMinus:{}:{}", g0.id, out.0));
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| !merge_find::group(&h.app.session.catalog, &g0.id).unwrap().photos.contains(&out)), "taken out");
    h.advance_time(1.0);
    click(&mut h, &format!("mergePageMinus:{}:{}", g0.id, out.0));
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| merge_find::group(&h.app.session.catalog, &g0.id).unwrap().photos.contains(&out)), "back");
    h.advance_time(1.0);
    h.settle(SETTLE);

    // filters narrow: kind., confidence., camera.
    menu_pick(&mut h, "Kind", "hdr", false);
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![hdr.id.clone()]);
    assert!(rect_of(&h, &format!("group:merges:group:{}", pano.id)).is_none());
    audit(&h, "kind. hdr.");
    shot(&mut h, "mergepage-3-filter.png");
    menu_pick(&mut h, "Kind", "pano", true);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2, "⌘-click adds a kind");
    menu_pick(&mut h, "Kind", "all", false);
    h.settle(SETTLE);
    let best = [pano.confidence(), hdr.confidence()].into_iter().max().unwrap();
    h.app.run("merge.filter", json!({"from": best.id()})).unwrap();
    h.settle(SETTLE);
    let n_best = [&pano, &hdr].iter().filter(|g| g.confidence() >= best).count();
    assert_eq!(order(&h).len(), n_best);
    h.app.run("merge.filter", json!({"from": "maybe", "camera": "RICOH GR III"})).unwrap();
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![pano.id.clone()], "camera. GR III");
    h.app.run("merge.filter", json!({"clear": true})).unwrap();
    assert!(h.app.run("merge.filter", json!({"kinds": ["sideways"]})).is_err());
    h.settle(SETTLE);

    // the narrow window too
    h.size = egui::vec2(1100.0, 800.0);
    h.settle(SETTLE);
    let order_now = order(&h);
    h.app
        .run(
            "library.select",
            json!({"ids": merge_find::group(&h.app.session.catalog, &order_now[0]).unwrap().photos.iter().map(|p| p.0).collect::<Vec<_>>()}),
        )
        .unwrap();
    h.settle(SETTLE);
    audit(&h, "1100 × 800");
    shot(&mut h, "mergepage-4-narrow.png");
    h.size = egui::vec2(1400.0, 900.0);
    h.settle(SETTLE);

    // S sends the picked group back and goes on to the next; M merges with the options shown
    let ids = order(&h);
    h.app
        .run(
            "library.select",
            json!({"ids": merge_find::group(&h.app.session.catalog, &ids[0]).unwrap().photos.iter().map(|p| p.0).collect::<Vec<_>>()}),
        )
        .unwrap();
    h.settle(SETTLE);
    key(&mut h, "S", false);
    assert_eq!(merge_find::group(&h.app.session.catalog, &ids[0]).unwrap().state, GroupState::Dismissed);
    let next = merge_find::group(&h.app.session.catalog, &ids[1]).unwrap();
    assert_eq!(h.app.session.selection.ids, next.photos, "on to the next group");
    h.app.run("merge.restore", json!({"ids": [ids[0]]})).unwrap();
    h.settle(SETTLE);
    // an option changed on the page is the one the merge uses (frames. folded: the options in view)
    click(&mut h, "fold:mergePage.frames");
    h.settle(SETTLE);
    assert!(!crate::folds::is_open(&h.view.ctx, "mergePage.frames"), "the panel's sections fold");
    audit(&h, "frames. folded");
    shot(&mut h, "mergepage-5-options.png");
    click(&mut h, "check:mergePage.autoSettings");
    assert!(crate::merge::page_options(&h.app, next.kind.command()).auto_settings, "ticked on the page (off by default)");
    // M (as merge.) opens the Merge box on the group's kind, with the page's options
    key(&mut h, "M", false);
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => {
            assert_eq!(opts.command, next.kind.command(), "on the group's kind");
            assert!(opts.auto_settings, "the options shown");
        }
        other => panic!("the Merge box: {other:?}"),
    }
    assert_eq!(h.app.merge.dialog_groups, vec![next.id.clone()]);
    h.settle(SETTLE);
    let faults = crate::tests_panels::layout_faults(&h, widget(&h, "dialog:window"));
    assert!(faults.is_empty(), "the Merge box: {faults:#?}");
    shot(&mut h, "mergepage-6-box.png");
    assert_eq!(h.app.merge.ids, next.photos);
    // every other kind one click away in the same box
    for k in ["merge.hdr", "merge.panorama", "merge.hdrPanorama", "merge.focus"] {
        assert!(has(&h, &format!("button:mergeKind-{k}")), "{k}");
    }
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    let task = h.app.merge.final_task.as_ref().expect("merging");
    assert_eq!(task.group.as_ref().map(|g| g.0.clone()), Some(next.id.clone()));
    assert!(task.options.auto_settings, "with the options shown");
    assert!(
        h.step_until(Duration::from_secs(120), |h| merge_find::group(&h.app.session.catalog, &next.id)
            .is_some_and(|g| g.state == GroupState::Merged)),
        "merged: {}",
        h.app.ui.status
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// layout. (waves. / grid. / strip. / focus.) and aspect. (original. / square. / 3:2. / 4:5. /
/// 16:9.): commands, remembered in ui.json, each laid out clean; aspect. shapes only the cards.
#[test]
fn layouts_and_card_shapes() {
    let (mut h, dir, pano, hdr) = page("layouts");
    let v = crate::merges::view_prefs(&h.app);
    assert_eq!((v.layout, v.aspect, v.per_row), (crate::merges::Layout::Waves, crate::merges::CardAspect::Original, 4));
    assert!(has(&h, "button:mergesLayout") && has(&h, "button:mergesAspect"), "on the bar");
    let develop_before: Vec<_> = pano.photos.iter().chain(&hdr.photos).map(|p| h.app.session.develop_of(*p).map(|d| (*d).clone())).collect();
    let crops: Vec<_> = pano.photos.iter().chain(&hdr.photos).map(|p| h.app.session.catalog.photo(*p).map(|x| (x.width, x.height))).collect();
    for layout in ["waves", "grid", "strip", "focus"] {
        let r = h.app.run("merge.layout", json!({"layout": layout})).unwrap();
        assert_eq!(r["layout"], layout);
        h.app.run("library.select", json!({"ids": pano.photos.iter().map(|p| p.0).collect::<Vec<_>>()})).unwrap();
        h.settle(SETTLE);
        let pc = cards(&h, &pano.id);
        // (strip.: the ones scrolled into view)
        if layout == "strip" {
            assert!(!pc.is_empty(), "strip: its frames");
        } else {
            assert_eq!(pc.len(), 5, "{layout}: every frame");
        }
        match layout {
            "grid" | "focus" => {
                let overl = pc.windows(2).any(|w| w[1].1.intersect(w[0].1).area() > 1.0);
                assert!(!overl, "{layout}: side by side {pc:?}");
            }
            "strip" => {
                let ys: Vec<f32> = pc.iter().map(|(_, r)| r.top()).collect();
                assert!(ys.iter().all(|y| (y - ys[0]).abs() < 0.5), "one row: {ys:?}");
            }
            _ => {}
        }
        if layout == "focus" {
            assert!(rect_of(&h, &format!("group:merges:group:{}", hdr.id)).is_none(), "one group at a time");
            assert!(pc.iter().all(|(_, r)| r.height() >= 100.0), "big: {pc:?}");
            assert!(has(&h, "button:mergesNext"));
            key(&mut h, "right", false);
            assert!(
                rect_of(&h, &format!("group:merges:group:{}", hdr.id)).is_some() || rect_of(&h, &format!("group:merges:group:{}", pano.id)).is_some()
            );
        }
        audit(&h, layout);
        shot(&mut h, &format!("mergepage-layout-{layout}.png"));
    }
    h.app.run("merge.layout", json!({"layout": "grid"})).unwrap();
    for (aspect, ratio) in [("square", Some(1.0f32)), ("3:2", Some(1.5)), ("4:5", Some(0.8)), ("16:9", Some(16.0 / 9.0)), ("original", None)] {
        let r = h.app.run("merge.layout", json!({"aspect": aspect})).unwrap();
        assert_eq!(r["aspect"], aspect);
        h.settle(SETTLE);
        if let Some(want) = ratio {
            for (_, c) in cards(&h, &pano.id) {
                assert!((c.width() / c.height() - want).abs() < 0.03, "{aspect}: {c:?}");
            }
        }
        audit(&h, aspect);
        shot(&mut h, &format!("mergepage-aspect-{}.png", aspect.replace(':', "x")));
    }
    // only the cards: the photos' settings and sizes are untouched
    let develop_after: Vec<_> = pano.photos.iter().chain(&hdr.photos).map(|p| h.app.session.develop_of(*p).map(|d| (*d).clone())).collect();
    assert_eq!(develop_before, develop_after, "aspect. never changes a photo");
    let crops_after: Vec<_> = pano.photos.iter().chain(&hdr.photos).map(|p| h.app.session.catalog.photo(*p).map(|x| (x.width, x.height))).collect();
    assert_eq!(crops, crops_after);
    assert!(h.app.run("merge.layout", json!({"layout": "spiral"})).is_err());
    assert!(h.app.run("merge.layout", json!({"aspect": "7:3"})).is_err());
    // remembered: through ui.json
    h.app.run("merge.layout", json!({"layout": "strip", "aspect": "4:5"})).unwrap();
    let saved = serde_json::to_value(&h.app.ui).unwrap();
    let medium = &saved["settings"]["mergeViews"]["medium"];
    assert_eq!((medium["layout"].as_str(), medium["aspect"].as_str()), (Some("strip"), Some("4:5")), "{}", saved["settings"]);
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    let v = back.settings.merge_views.get("medium").cloned().unwrap();
    assert_eq!((v.layout, v.aspect), (crate::merges::Layout::Strip, crate::merges::CardAspect::FourFive));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Settings ▸ merge.: its defaults, ui.json, the tab laid out clean, and the merges using them —
/// the Merge dialog's starting options, a group merge keeping the originals visible with the
/// merged photo in merges. only, the confidence the view shows from.
#[test]
fn merge_settings_are_stored_and_used() {
    let d = crate::state::AppSettings::default();
    assert_eq!(d.merge_find, crate::merges::FindMode::Auto, "weddings only, today's rule");
    assert!(
        d.merge.panorama.stack && !d.merge.hdr.auto_settings && d.merge.panorama.auto_crop && d.merge.hdr.hide_originals && !d.merge.hdr.merges_only
    );
    assert_eq!((d.merge.panorama.projection.as_str(), d.merge.hdr.deghost.as_str()), ("auto", "none"));
    // an older ui.json: no merge. settings, the old find mode names
    let old: crate::state::UiState = serde_json::from_value(json!({"settings": {"mergeFind": "manual"}})).unwrap();
    assert_eq!((old.settings.merge_find, old.settings.merge.clone()), (crate::merges::FindMode::Manual, d.merge.clone()));

    let (mut h, dir, pano, hdr) = page("settings");
    // the tab: after develop.view., every row there, nothing clipped
    let tabs: Vec<&str> = crate::panels::settings::TABS.iter().map(|(id, _)| *id).collect();
    let at = tabs.iter().position(|t| *t == "merge").expect("a merge. tab");
    assert_eq!(tabs.get(at - 1), Some(&"develop"));
    h.app.run("app.settings", json!({"tab": "merge"})).unwrap();
    h.settle(SETTLE);
    for w in [
        "button:settingsMergeFind-1",
        "button:settingsMergeShowFrom-2",
        "button:settingsMerge-panorama-projection-2",
        "button:settingsMerge-hdr-originals-1",
    ] {
        assert!(has(&h, w), "{w}");
    }
    let faults = crate::tests_panels::layout_faults(&h, widget(&h, "dialog:window"));
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "mergepage-settings.png");
    click(&mut h, "button:settingsMergeFind-1");
    assert_eq!(h.app.ui.settings.merge_find, crate::merges::FindMode::Every);
    click(&mut h, "button:settingsMerge-panorama-projection-2");
    assert_eq!(h.app.ui.settings.merge.panorama.projection, "cylindrical");
    click(&mut h, "button:settingsMerge-hdr-originals-1");
    click(&mut h, "button:settingsMerge-hdr-showIn-1");
    assert!(!h.app.ui.settings.merge.hdr.hide_originals && h.app.ui.settings.merge.hdr.merges_only);
    h.app.ui.dialog = None;
    // and General no longer has the row
    h.app.run("app.settings", json!({"tab": "general"})).unwrap();
    h.settle(SETTLE);
    assert!(!has(&h, "button:settingsMergeFind-0"), "moved to merge.");
    h.app.ui.dialog = None;
    // merge.settings / merge.findMode commands
    let v = h.app.run("merge.settings", json!({"panorama": {"stack": false}})).unwrap();
    assert_eq!(v["panorama"]["stack"], false);
    assert!(h.app.run("merge.settings", json!({"panorama": {"stak": false}})).is_err());
    assert_eq!(h.app.run("merge.findMode", json!({"mode": "off."})).unwrap()["mode"], "manual");
    assert_eq!(h.app.run("merge.findMode", json!({"mode": "every"})).unwrap()["mode"], "every");
    // ui.json round trip
    let back: crate::state::UiState = serde_json::from_value(serde_json::to_value(&h.app.ui).unwrap()).unwrap();
    assert_eq!(back.settings.merge, h.app.ui.settings.merge);
    assert_eq!(back.settings.merge_find, crate::merges::FindMode::Every);

    // the Merge dialog starts from them
    h.app.run("library.select", json!({"ids": pano.photos.iter().map(|p| p.0).collect::<Vec<_>>()})).unwrap();
    h.app.run("dialog.merge", json!({})).unwrap();
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => {
            assert_eq!((opts.command.as_str(), opts.projection.as_str(), opts.stack), ("merge.panorama", "cylindrical", false));
        }
        other => panic!("the Merge dialog: {other:?}"),
    }
    h.app.ui.dialog = None;
    h.settle(SETTLE);
    // the page's options start from them too
    assert_eq!(crate::merge::page_options(&h.app, "merge.panorama").projection, "cylindrical");

    // show from: groups less sure are not shown
    let surest = [pano.confidence(), hdr.confidence()].into_iter().max().unwrap();
    h.app.run("merge.settings", json!({"showFrom": surest.id()})).unwrap();
    h.settle(SETTLE);
    let shown = crate::merges::compute_view_data(&h.app).open.len();
    assert_eq!(shown, [&pano, &hdr].iter().filter(|g| g.confidence() >= surest).count());
    h.app.run("merge.settings", json!({"showFrom": "maybe"})).unwrap();
    h.settle(SETTLE);

    // a group merge: the hdr. originals stay visible, the merged photo only in merges.
    h.app.run("library.select", json!({"ids": hdr.photos.iter().map(|p| p.0).collect::<Vec<_>>()})).unwrap();
    h.settle(SETTLE);
    h.app.run("merge.mergePicked", json!({})).unwrap();
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert!(
        h.step_until(Duration::from_secs(120), |h| merge_find::group(&h.app.session.catalog, &hdr.id).is_some_and(|g| g.state == GroupState::Merged)),
        "merged: {}",
        h.app.ui.status
    );
    let merged = merge_find::group(&h.app.session.catalog, &hdr.id).unwrap().result.unwrap();
    let m262 = dir.join("M262").to_string_lossy().to_string();
    h.app.run("view.library", json!({})).unwrap();
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": m262})).unwrap();
    let shown = h.app.session.visible_cloned();
    assert!(hdr.photos.iter().all(|p| shown.contains(p)), "the originals kept visible: {shown:?}");
    assert!(!shown.contains(&merged), "the merged photo in merges. only");
    let _ = std::fs::remove_dir_all(&dir);
}

/// per.row.: how many groups (tiles) go across, whatever the layout; smaller. / bigger. step it; the
/// layout., aspect., per.row. and kind. pick are remembered per screen size (his MacBook and his
/// big display each keep theirs); clean at 1100, 1500 and 2560 points wide.
#[test]
fn per_row_and_per_screen_memory() {
    let (mut h, dir, _pano, _hdr) = page("perrow");
    h.app.run("merge.layout", json!({"layout": "grid"})).unwrap();
    // the first group (in view at every size): its tile narrows as more go across
    let first = merge_find::group(&h.app.session.catalog, &order(&h)[0]).unwrap();
    let mut widths = Vec::new();
    for n in [2u64, 3, 4] {
        let r = h.app.run("merge.layout", json!({"perRow": n})).unwrap();
        assert_eq!(r["perRow"], n);
        h.settle(SETTLE);
        let t = widget(&h, &format!("mergeTile:{}", first.id));
        let canvas = h.app.canvas_rect.unwrap();
        let want = (canvas.width() - 32.0 - 12.0 * (n as f32 - 1.0)) / n as f32;
        assert!((t.width() - want).abs() <= 3.0, "{n} per row: tile {} vs {want}", t.width());
        widths.push(t.width());
    }
    assert!(widths[0] > widths[1] && widths[1] > widths[2], "more per row, narrower tiles: {widths:?}");
    h.app.run("merge.layout", json!({"perRow": 6})).unwrap();
    click(&mut h, "button:mergesBigger");
    assert_eq!(crate::merges::view_prefs(&h.app).per_row, 5);
    assert!(h.app.run("merge.layout", json!({"perRow": 12})).is_err());

    // per screen: this one (1400: medium) keeps grid.; a big display has its own
    h.app.run("merge.filter", json!({"kinds": ["pano"]})).unwrap();
    h.size = egui::vec2(2560.0, 1440.0);
    h.settle(SETTLE);
    assert_eq!(crate::merges::screen(&h.app), "large");
    let big = crate::merges::view_prefs(&h.app);
    assert_eq!((big.layout, big.per_row, big.kinds.len()), (crate::merges::Layout::Waves, 4, 0), "the big display starts from the defaults");
    h.app.run("merge.layout", json!({"layout": "strip", "perRow": 6})).unwrap();
    h.settle(SETTLE);
    audit(&h, "2560");
    shot(&mut h, "mergepage-2560.png");
    h.size = egui::vec2(1500.0, 900.0);
    h.settle(SETTLE);
    let back = crate::merges::view_prefs(&h.app);
    assert_eq!((back.layout, back.per_row, back.kinds.clone()), (crate::merges::Layout::Grid, 5, vec![MergeType::Pano]), "the MacBook's again");
    audit(&h, "1500");
    shot(&mut h, "mergepage-1500.png");
    h.size = egui::vec2(1100.0, 800.0);
    h.settle(SETTLE);
    assert_eq!(crate::merges::screen(&h.app), "small");
    audit(&h, "1100");
    // through ui.json
    let saved = serde_json::to_value(&h.app.ui).unwrap();
    let back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.settings.merge_views, h.app.ui.settings.merge_views);
    assert_eq!(back.settings.merge_views.get("large").map(|v| (v.layout, v.per_row)), Some((crate::merges::Layout::Strip, 6)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open drop-down `which` (`Kind`, `Confidence` or `Camera`) if it isn't open, then click its row
/// `row` (`pano`, `all`, `0`, …; `cmd`: ⌘ held, `shift`: ⇧ held).
fn menu_pick_with(h: &mut Headless, which: &str, row: &str, cmd: bool, shift: bool) {
    let row_id = format!("menu:mergesFilter{which}-{row}");
    if !has(h, &row_id) {
        click(h, &format!("button:mergesFilter{which}"));
        h.settle(SETTLE);
    }
    let r = h.request("ui.clickWidget", json!({"id": row_id, "cmd": cmd, "shift": shift}), T);
    assert_eq!(r["ok"], true, "{row_id}: {r}");
    h.step();
    h.step();
}

fn menu_pick(h: &mut Headless, which: &str, row: &str, cmd: bool) {
    menu_pick_with(h, which, row, cmd, false);
}

/// The quiet count at the right of a menu row.
fn row_count(h: &Headless, id: &str) -> String {
    let r = widget(h, id);
    let texts: Vec<String> =
        h.view.visible_texts().into_iter().filter(|(s, t, _)| r.contains(t.center()) && s.parse::<usize>().is_ok()).map(|(s, _, _)| s).collect();
    texts.last().cloned().unwrap_or_default()
}

/// The text a button shows.
fn button_text(h: &Headless, id: &str) -> String {
    let r = widget(h, id);
    let mut texts: Vec<(f32, String)> =
        h.view.visible_texts().into_iter().filter(|(_, t, _)| r.contains(t.center())).map(|(s, t, _)| (t.left(), s)).collect();
    texts.sort_by(|a, b| a.0.total_cmp(&b.0));
    texts.into_iter().map(|(_, s)| s).collect::<Vec<_>>().join(" ")
}

/// The filters are three drop-downs on the bar's own row (kind., confidence., camera.), each with
/// its rows counting the groups of the scope picked in the sidebar: all. 2, panorama. 1, hdr. 1,
/// focus.stack. 0 (a full row with a quiet 0); click = only that kind, ⌘-click adds, ⇧-click the
/// run, all. every kind; the button says what is picked.
#[test]
fn kind_and_confidence_drop_downs_count_the_groups() {
    let (mut h, dir, pano, hdr) = page("chips");
    // on the bar's row, right of find., no second filter row
    let find = widget(&h, "button:mergesFind");
    for w in ["mergeFilter:kind", "mergeFilter:confidence", "mergeFilter:camera", "label:mergesFilterCount"] {
        assert!((widget(&h, w).center().y - find.center().y).abs() <= 2.0, "{w} on find.'s row");
    }
    assert!(!has(&h, "merges:filters"), "no second filter row");
    assert!(h.app.widgets.iter().all(|(w, _)| !w.starts_with("button:mergesKind") || w == "button:mergesFilterKind" || w.starts_with("button:mergesKind-")), "no kind chips");
    let (kind, conf, cam) = (widget(&h, "mergeFilter:kind"), widget(&h, "mergeFilter:confidence"), widget(&h, "mergeFilter:camera"));
    assert!(find.right() + 8.0 - 0.5 <= kind.left() && kind.right() + 8.0 - 0.5 <= conf.left() && conf.right() + 8.0 - 0.5 <= cam.left(), "8 px apart");
    assert!([kind, conf, cam].iter().all(|r| (r.height() - find.height()).abs() < 0.5), "one control height");
    assert_eq!(button_text(&h, "button:mergesFilterKind"), "kind. all.");
    assert_eq!(button_text(&h, "button:mergesFilterCamera"), "camera.");

    // open: a row per option, with counts; zero is a full-strength row with a quiet 0
    click(&mut h, "button:mergesFilterKind");
    h.settle(SETTLE);
    assert_eq!(row_count(&h, "menu:mergesFilterKind-all"), "2");
    assert_eq!(row_count(&h, "menu:mergesFilterKind-pano"), "1");
    assert_eq!(row_count(&h, "menu:mergesFilterKind-hdr"), "1");
    assert_eq!(row_count(&h, "menu:mergesFilterKind-stack"), "0", "zero is a quiet 0");
    assert_eq!(row_count(&h, "menu:mergesFilterKind-hdrPano"), "0");
    assert!(has(&h, "menu:mergesFilterKind-none"), "none.");
    let stack = widget(&h, "menu:mergesFilterKind-stack");
    assert!((stack.height() - widget(&h, "menu:mergesFilterKind-pano").height()).abs() < 0.5, "a full row");
    shot(&mut h, "mergepage-kind-menu.png");
    let menu_faults = crate::tests_panels::layout_faults(&h, widget(&h, "menu:mergesFilterKind-all").union(widget(&h, "menu:mergesFilterKind-none")));
    assert!(menu_faults.is_empty(), "{menu_faults:#?}");
    // click = only that kind (the menu closes), the button says it; the counts don't follow their own pick
    menu_pick(&mut h, "Kind", "pano", false);
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![pano.id.clone()], "panorama. filters as the old chip did");
    assert_eq!(crate::merges::view_prefs(&h.app).kinds, vec![MergeType::Pano]);
    assert!(!has(&h, "menu:mergesFilterKind-pano"), "a click closes the menu");
    assert_eq!(button_text(&h, "button:mergesFilterKind"), "kind. panorama.");
    click(&mut h, "button:mergesFilterKind");
    h.settle(SETTLE);
    assert_eq!(row_count(&h, "menu:mergesFilterKind-hdr"), "1", "the counts don't follow their own pick");
    // ⌘-click adds a second kind and keeps the menu open
    menu_pick(&mut h, "Kind", "hdr", true);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2);
    assert!(has(&h, "menu:mergesFilterKind-hdr"), "⌘-click keeps the menu open");
    assert_eq!(button_text(&h, "button:mergesFilterKind"), "kind. 2 picked.");
    // ⌘-click again takes it away
    menu_pick(&mut h, "Kind", "hdr", true);
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![pano.id.clone()]);
    // ⇧-click: the run from the last one clicked (hdr. … panorama. in the filter order: pano., hdr.)
    menu_pick(&mut h, "Kind", "pano", false);
    menu_pick_with(&mut h, "Kind", "hdr", false, true);
    h.settle(SETTLE);
    assert_eq!(crate::merges::view_prefs(&h.app).kinds, vec![MergeType::Pano, MergeType::Hdr], "⇧-click: the run");
    // a click on one alone, then all.
    menu_pick(&mut h, "Kind", "hdr", false);
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![hdr.id.clone()]);
    menu_pick(&mut h, "Kind", "all", false);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2);
    assert_eq!(button_text(&h, "button:mergesFilterKind"), "kind. all.");
    menu_pick(&mut h, "Kind", "pano", false);
    menu_pick(&mut h, "Kind", "none", false);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 0, "none.: nothing picked, nothing shows");
    assert_eq!(button_text(&h, "button:mergesFilterKind"), "kind. none.");
    menu_pick(&mut h, "Kind", "all", false);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2, "all. again");

    // confidence.
    let n_conf = |c: lightcraft_engine::merge_find::Confidence| [&pano, &hdr].iter().filter(|g| g.confidence() == c).count();
    click(&mut h, "button:mergesFilterConfidence");
    h.settle(SETTLE);
    for c in crate::merge_page::CONFIDENCES {
        assert_eq!(row_count(&h, &format!("menu:mergesFilterConfidence-{}", c.id())), n_conf(c).to_string(), "{c:?}");
    }
    let c = pano.confidence();
    menu_pick(&mut h, "Confidence", c.id(), false);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), n_conf(c));
    assert_eq!(button_text(&h, "button:mergesFilterConfidence"), format!("confidence. {}", c.tag()));
    menu_pick(&mut h, "Confidence", "all", false);
    h.settle(SETTLE);

    // camera.: one camera at a time, with its count
    click(&mut h, "button:mergesFilterCamera");
    h.settle(SETTLE);
    assert_eq!(row_count(&h, "menu:mergesFilterCamera-all"), "2");
    assert_eq!(row_count(&h, "menu:mergesFilterCamera-0"), "1");
    let cams = crate::merges::compute_view_data(&h.app).cameras;
    menu_pick(&mut h, "Camera", "0", false);
    h.settle(SETTLE);
    assert_eq!(h.app.merge.filter.camera.as_ref(), cams.first());
    assert_eq!(order(&h).len(), 1, "camera.");
    assert_eq!(button_text(&h, "button:mergesFilterCamera"), format!("camera. {}", cams[0]));
    menu_pick(&mut h, "Camera", "all", false);
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2);

    // the counts follow the scope: the M262 folder alone has the bracket only
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": dir.join("M262").to_string_lossy()})).unwrap();
    h.settle(SETTLE);
    click(&mut h, "button:mergesFilterKind");
    h.settle(SETTLE);
    assert_eq!(row_count(&h, "menu:mergesFilterKind-all"), "1");
    assert_eq!(row_count(&h, "menu:mergesFilterKind-pano"), "0");
    audit(&h, "drop-downs");
    shot(&mut h, "mergepage-chips.png");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `merge.mergeGroups {options}` (what Second Brain's merge box sends): its choices reach the
/// merge the worker runs, over the kind's defaults.
#[test]
fn merge_groups_options_reach_the_merge() {
    let (mut h, dir, pano, _) = page("options");
    let r = h
        .app
        .run(
            "merge.mergeGroups",
            json!({"ids": [pano.id], "options": {"projection": "cylindrical", "boundaryWarp": 40, "autoCrop": false, "stack": false}}),
        )
        .unwrap();
    assert_eq!(r["queued"], 1, "{r}");
    let task = h.app.merge.final_task.as_ref().expect("merging");
    assert_eq!(
        (task.options.projection.as_str(), task.options.boundary_warp, task.options.auto_crop, task.options.stack),
        ("cylindrical", 40.0, false, false)
    );
    assert!(!task.options.auto_settings, "a missing key: the kind's default (off)");
    let plan = format!("{:?}", task.job.kind);
    assert!(plan.contains("Cylindrical") && plan.contains("40.0"), "{plan}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------------------- tiles and drop-downs

/// `n` brackets (3 frames each, ten minutes apart, two cameras in turn) in one shoot folder, the
/// merge. page open on it: every group its own tile.
fn tiles_page(tag: &str, n: usize, size: [f32; 2]) -> (Headless, std::path::PathBuf, Vec<String>) {
    use lightcraft_catalog::{Op, PhotoId};
    let dir = std::env::temp_dir().join(format!("lc-ui-mergetiles-wedding-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut paths = Vec::new();
    for g in 0..n {
        for (i, b) in lightcraft_merge::synth::bracket_dngs(120 + 2 * g, 80, &[-2.0, 0.0, 2.0]).unwrap().into_iter().enumerate() {
            let p = dir.join(format!("G{g:02}_{i}.dng"));
            std::fs::write(&p, b).unwrap();
            paths.push(p.to_string_lossy().to_string());
        }
    }
    let mut session = lightcraft_engine::Session::new().with_fs();
    let r = session.execute("library.import", &json!({"paths": paths})).unwrap();
    let ids: Vec<PhotoId> = r["imported"].as_array().unwrap().iter().map(|v| PhotoId(v.as_u64().unwrap())).collect();
    assert_eq!(ids.len(), n * 3);
    for (k, id) in ids.iter().enumerate() {
        let (g, i) = (k / 3, k % 3);
        let mut m = session.catalog.photo(*id).unwrap().meta.clone();
        m.camera = if g % 2 == 0 { "LEICA M (Typ 262)" } else { "RICOH GR III" }.into();
        m.lens = "Summicron 35".into();
        m.focal_mm = Some(35.0);
        m.aperture = Some(8.0);
        m.iso = Some(200);
        m.shutter = ["1/500", "1/125", "1/30"][i].into();
        m.exposure_bias = Some([-2.0, 0.0, 2.0][i]);
        session.catalog.apply(Op::SetMeta { id: *id, meta: Box::new(m) }).unwrap();
        let (hh, mm) = (9 + g / 6, (g % 6) * 10);
        let t = format!("2026-11-20T{hh:02}:{mm:02}:{:02}", if i == 2 { 1 } else { 0 });
        session.catalog.apply(Op::SetCaptured { id: *id, captured: Some(t) }).unwrap();
    }
    let services = crate::Services {
        png: Some(Box::new(|img: &lightcraft_raster::Rgba8| {
            lightcraft_codecs::encode_png(&lightcraft_codecs::EncodeImage::rgba8(img), &lightcraft_codecs::EncodeMeta::default()).unwrap_or_default()
        })),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = crate::LightcraftApp::new(session, services);
    app.fix_notes = crate::fixnotes::FixNotes::default();
    let mut h = Headless::new(app, size, 1.0);
    let r = h.request("ui.set", json!({"view": "photoGrid", "leftPanel": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.app.ui.toggle_sidebar_section("navigator");
    h.settle(SETTLE);
    assert!(h.step_until(Duration::from_secs(120), |h| merge_find::groups(&h.app.session.catalog).len() == n), "found {n} groups");
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": dir.to_string_lossy()})).unwrap();
    let r = h.request("ui.clickWidget", json!({"id": "module:merge"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(crate::merge_page::on(&h.app), "the merge. page");
    h.app.run("library.select", json!({"ids": []})).unwrap();
    h.settle(SETTLE);
    let order = order(&h);
    assert_eq!(order.len(), n);
    (h, dir, order)
}

fn tile(h: &Headless, gid: &str) -> egui::Rect {
    widget(h, &format!("mergeTile:{gid}"))
}

/// Tiles in the order shown: how many share the first one's top, and that they sit 12 px apart.
fn across(h: &Headless, order: &[String]) -> usize {
    let first = tile(h, &order[0]);
    let row: Vec<egui::Rect> = order.iter().filter_map(|g| rect_of(h, &format!("mergeTile:{g}"))).filter(|r| (r.top() - first.top()).abs() < 0.5).collect();
    for w in row.windows(2) {
        assert!(w[1].left() - w[0].right() >= 12.0 - 0.5, "tiles 12 px apart: {w:?}");
        assert!((w[1].height() - w[0].height()).abs() < 0.5, "a row of tiles is one height: {w:?}");
    }
    row.len()
}

#[test]
fn groups_are_tiles_per_row_across() {
    let (mut h, dir, order) = tiles_page("tiles", 6, [1400.0, 900.0]);
    assert_eq!(crate::merges::view_prefs(&h.app).per_row, 4, "4 across by default");
    for layout in ["waves", "grid", "strip"] {
        h.app.run("merge.layout", json!({"layout": layout, "perRow": 4})).unwrap();
        h.settle(SETTLE);
        assert_eq!(across(&h, &order), 4, "{layout}: 4 groups across");
        let (a, e) = (tile(&h, &order[0]), tile(&h, &order[4]));
        assert!(e.top() >= a.bottom() + 12.0 - 0.5, "{layout}: the 5th starts a new row, 12 px under: {a:?} {e:?}");
        assert!((e.left() - a.left()).abs() < 0.5, "{layout}: under the first");
        // every group's header and frames sit inside its tile
        for g in &order {
            let t = tile(&h, g);
            let head = widget(&h, &format!("group:merges:group:{g}"));
            assert!(t.contains_rect(head.shrink(0.5)), "{layout}: the header inside its tile {head:?} {t:?}");
            for w in [format!("button:mergesKind-{g}"), format!("button:mergesGroupMerge-{g}")] {
                assert!(t.contains_rect(widget(&h, &w).shrink(0.5)), "{layout}: {w} inside its tile");
            }
            for (_, c) in cards(&h, g) {
                assert!(t.expand(1.0).contains_rect(c) || layout == "strip", "{layout}: frames inside their tile {c:?} {t:?}");
            }
        }
        audit(&h, &format!("tiles {layout}"));
    }
    h.app.run("merge.layout", json!({"layout": "waves"})).unwrap();
    // per.row. 3 → 3 across; smaller. / bigger. step it
    h.app.run("merge.layout", json!({"perRow": 3})).unwrap();
    h.settle(SETTLE);
    assert_eq!(across(&h, &order), 3);
    click(&mut h, "button:mergesSmaller");
    h.settle(SETTLE);
    assert_eq!(across(&h, &order), 4);
    click(&mut h, "button:mergesBigger");
    click(&mut h, "button:mergesBigger");
    h.settle(SETTLE);
    assert_eq!((crate::merges::view_prefs(&h.app).per_row, across(&h, &order)), (2, 2));
    audit(&h, "2 across");
    // focus. is one big group
    h.app.run("merge.layout", json!({"layout": "focus"})).unwrap();
    h.settle(SETTLE);
    assert_eq!(order.iter().filter(|g| rect_of(&h, &format!("mergeTile:{g}")).is_some()).count(), 1, "focus.: one group");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tiles_are_clean_at_1400_and_2000() {
    for (size, shot_name) in [([1400.0, 900.0], "mergetiles-1400.png"), ([2000.0, 1000.0], "mergetiles-2000.png")] {
        let (mut h, dir, order) = tiles_page(&format!("clean{}", size[0]), 9, size);
        h.app.run("library.select", json!({"ids": merge_find::group(&h.app.session.catalog, &order[1]).unwrap().photos.iter().map(|p| p.0).collect::<Vec<_>>()})).unwrap();
        h.settle(SETTLE);
        assert_eq!(across(&h, &order), 4, "{size:?}");
        audit(&h, &format!("{size:?}"));
        shot(&mut h, shot_name);
        // the drop-downs on find.'s row; the kind. menu open is clean too
        let find = widget(&h, "button:mergesFind");
        for w in ["mergeFilter:kind", "mergeFilter:confidence", "mergeFilter:camera"] {
            assert!((widget(&h, w).center().y - find.center().y).abs() <= 2.0, "{size:?}: {w} on find.'s row");
        }
        click(&mut h, "button:mergesFilterKind");
        h.settle(SETTLE);
        assert!(has(&h, "menu:mergesFilterKind-pano"), "open");
        audit(&h, &format!("{size:?} kind. open"));
        shot(&mut h, &shot_name.replace(".png", "-kind-menu.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ------------------------------------------------------------------------- the frames' question

/// grid. layout: every frame side by side (in a fan the later cards cover the earlier ones).
fn grid(h: &mut Headless) {
    h.app.run("merge.layout", json!({"layout": "grid"})).unwrap();
    h.settle(SETTLE);
}

fn group_photos(h: &Headless, gid: &str) -> Vec<lightcraft_catalog::PhotoId> {
    merge_find::group(&h.app.session.catalog, gid).expect("the group").photos
}

fn card_center(h: &Headless, gid: &str, pid: lightcraft_catalog::PhotoId) -> egui::Pos2 {
    widget(h, &format!("mergeCard:{gid}:{}", pid.0)).center()
}

/// The ⊖ on a frame opens one small popup (out.of.group. / remove. / del. / cancel.), laid out
/// clean; cancel., Esc and a click outside close it without changing anything; a click on a card
/// only picks it.
#[test]
fn the_minus_asks_what_to_do_with_the_frame() {
    let (mut h, dir, pano, _hdr) = page("minus");
    grid(&mut h);
    let f = pano.photos[1];
    // a click picks, nothing else
    click(&mut h, &format!("mergeCard:{}:{}", pano.id, f.0));
    assert_eq!(h.app.session.selection.ids, vec![f]);
    assert!(!has(&h, "mergesFrameMenu"));
    assert_eq!(group_photos(&h, &pano.id), pano.photos);
    // the tooltip is the file name and exposure, and the ⊖ hint (checked in the source of the text)
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    h.settle(SETTLE);
    assert!(has(&h, "mergesFrameMenu"), "the ⊖ opens the question");
    let menu = widget(&h, "mergesFrameMenu");
    let labels = ["mergesFrameOutOfGroup", "mergesFrameRemove", "mergesFrameDelete", "mergesFrameCancel"];
    let bs: Vec<egui::Rect> = labels.iter().map(|l| widget(&h, &format!("button:{l}"))).collect();
    let texts: Vec<String> = h.view.visible_texts().into_iter().filter(|(_, r, _)| menu.contains(r.center())).map(|(s, _, _)| s).collect();
    for want in ["out.of.group.", "remove.", "del.", "cancel."] {
        assert!(texts.contains(&want.to_string()), "{want}: {texts:?}");
    }
    for w in bs.windows(2) {
        assert!(w[1].left() - w[0].right() >= 8.0 - 0.5, "buttons 8 px apart: {w:?}");
        assert!((w[1].height() - w[0].height()).abs() < 0.5 && (w[0].top() - w[1].top()).abs() < 0.5, "one row, one height");
    }
    let title = widget(&h, "label:mergesFrameMenuTitle");
    assert!(bs.iter().all(|b| menu.contains_rect(*b) && b.left() - menu.left() >= 12.0 - 0.5 && menu.bottom() - b.bottom() >= 12.0 - 0.5), "12 px inside: {menu:?} {bs:?}");
    assert!(bs[0].top() - title.bottom() >= 12.0 - 0.5 && menu.contains_rect(title), "title 12 px over the buttons: {title:?}");
    let faults = crate::tests_panels::layout_faults(&h, menu);
    assert!(faults.is_empty(), "{faults:#?}");
    shot(&mut h, "mergepage-frame-menu.png");
    // cancel., Esc, a click outside: closed, nothing changed
    click(&mut h, "button:mergesFrameCancel");
    h.settle(SETTLE);
    assert!(!has(&h, "mergesFrameMenu"));
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(has(&h, "mergesFrameMenu"));
    key(&mut h, "escape", false);
    assert!(!has(&h, "mergesFrameMenu"), "Esc");
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(has(&h, "mergesFrameMenu"));
    let r = h.request("ui.click", json!({"x": 5.0, "y": 300.0}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(!has(&h, "mergesFrameMenu"), "a click outside");
    assert_eq!(group_photos(&h, &pano.id), pano.photos, "nothing changed");
    assert!(h.app.session.catalog.photo(f).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// out.of.group. takes the frame out of its merge group only (the photo is still in the catalog and
/// still shown, dimmed); the ⊖ then offers put.back.; with several frames picked it acts on all.
#[test]
fn out_of_group_keeps_the_photo_in_the_library() {
    let (mut h, dir, pano, _hdr) = page("outgroup");
    grid(&mut h);
    let (a, b) = (pano.photos[1], pano.photos[3]);
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, a.0));
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| !group_photos(h, &pano.id).contains(&a)), "out of the group");
    h.settle(SETTLE);
    assert!(h.app.session.catalog.photo(a).is_some(), "still in the catalog");
    assert!(has(&h, &format!("mergeCard:{}:{}", pano.id, a.0)), "still drawn");
    // offered back
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, a.0));
    let texts: Vec<String> = h.view.visible_texts().into_iter().map(|(s, _, _)| s).collect();
    assert!(texts.contains(&"put.back.".to_string()), "put.back. for a frame that is out");
    click(&mut h, "button:mergesFrameCancel");
    // two picked (⌘-click): the ⊖ of one acts on both
    click(&mut h, &format!("mergeCard:{}:{}", pano.id, b.0));
    let r = h.request("ui.clickWidget", json!({"id": format!("mergeCard:{}:{}", pano.id, pano.photos[2].0), "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, b.0));
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| !group_photos(h, &pano.id).contains(&b) && !group_photos(h, &pano.id).contains(&pano.photos[2])), "both out");
    assert!(group_photos(&h, &pano.id).contains(&pano.photos[0]), "the others stay");
    // an unpicked frame's ⊖ acts on that one alone
    let other = pano.photos[4];
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, other.0));
    assert_eq!(h.app.session.selection.ids, vec![other]);
    click(&mut h, "button:mergesFrameCancel");
    let _ = std::fs::remove_dir_all(&dir);
}

/// remove. takes the frame out of LightCraft (the catalog), the file stays on disk; del. asks once
/// more with Settings ▸ Confirm first on.
#[test]
fn remove_takes_the_frame_out_of_the_catalog() {
    let (mut h, dir, pano, hdr) = page("removeframe");
    grid(&mut h);
    let f = hdr.photos[0];
    let path = match &h.app.session.catalog.photo(f).expect("a photo").source {
        lightcraft_catalog::Source::File { path } => path.clone(),
        other => panic!("{other:?}"),
    };
    assert!(std::path::Path::new(&path).exists());
    click(&mut h, &format!("mergeMinus:{}:{}", hdr.id, f.0));
    click(&mut h, "button:mergesFrameRemove");
    assert!(h.step_until(T, |h| h.app.session.catalog.photo(f).is_none()), "out of the catalog");
    h.settle(SETTLE);
    assert!(std::path::Path::new(&path).exists(), "the file stays on disk");
    assert!(!has(&h, &format!("mergeCard:{}:{}", hdr.id, f.0)));
    assert!(!has(&h, "mergesFrameMenu"));
    // del. with Confirm first on: a second question, cancel. leaves everything
    h.app.ui.settings.confirm_delete = true;
    let g = pano.photos[0];
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, g.0));
    click(&mut h, "button:mergesFrameDelete");
    assert!(has(&h, "button:mergesFrameDeleteYes"), "asks once more");
    assert!(!has(&h, "button:mergesFrameRemove"));
    click(&mut h, "button:mergesFrameCancel");
    assert!(h.app.session.catalog.photo(g).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A right-click on a frame and ⌫ with frames picked open the same question.
#[test]
fn right_click_and_backspace_open_the_same_question() {
    let (mut h, dir, pano, _hdr) = page("rightclick");
    grid(&mut h);
    let f = pano.photos[1];
    let at = card_center(&h, &pano.id, f);
    let r = h.request("ui.click", json!({"x": at.x, "y": at.y, "button": "right"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(has(&h, "mergesFrameMenu"), "right-click");
    assert_eq!(h.app.session.selection.ids, vec![f], "it picks the frame");
    for l in ["mergesFrameOutOfGroup", "mergesFrameRemove", "mergesFrameDelete", "mergesFrameCancel"] {
        assert!(has(&h, &format!("button:{l}")), "{l}");
    }
    click(&mut h, "button:mergesFrameCancel");
    h.settle(SETTLE);
    assert!(!has(&h, "mergesFrameMenu"));
    // ⌫ with the frame picked
    key(&mut h, "backspace", false);
    assert!(has(&h, "mergesFrameMenu"), "⌫");
    let menu = widget(&h, "mergesFrameMenu");
    let faults = crate::tests_panels::layout_faults(&h, menu);
    assert!(faults.is_empty(), "{faults:#?}");
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| !group_photos(h, &pano.id).contains(&f)), "out of the group");
    assert!(h.app.session.catalog.photo(f).is_some());
    // nothing picked: ⌫ does what it did (no question)
    h.app.run("library.select", json!({"ids": []})).unwrap();
    h.settle(SETTLE);
    key(&mut h, "backspace", false);
    assert!(!has(&h, "mergesFrameMenu"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- review fixes (11-24): the ⊖ popup's life, what it acts on, Esc

fn texts(h: &Headless) -> Vec<String> {
    h.view.visible_texts().into_iter().map(|(s, _, _)| s).collect()
}

fn cmd_click(h: &mut Headless, id: &str) {
    let r = h.request("ui.clickWidget", json!({"id": id, "cmd": true}), T);
    assert_eq!(r["ok"], true, "{id}: {r}");
    h.settle(SETTLE);
}

fn menu_open(h: &Headless) -> bool {
    crate::merges::frame_menu_open(&h.view.ctx)
}

/// The popup lives only while the merges. view is on screen for the source it opened in: leaving
/// (another module, another source) closes it for good, it doesn't come back with old ids.
#[test]
fn the_frame_popup_does_not_outlive_the_view() {
    let (mut h, dir, pano, _hdr) = page("popuplife");
    grid(&mut h);
    let f = pano.photos[1];
    // another source: closed, and still closed when the merges come back
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(menu_open(&h));
    h.app.run("library.source", json!({"kind": "all"})).unwrap();
    h.settle(SETTLE);
    assert!(!menu_open(&h), "a source switch closes it");
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": dir.to_string_lossy()})).unwrap();
    h.settle(SETTLE);
    assert!(!menu_open(&h) && !has(&h, "mergesFrameMenu"), "and it stays closed");
    // another module and back
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(menu_open(&h));
    let r = h.request("ui.clickWidget", json!({"id": "module:develop"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    let r = h.request("ui.clickWidget", json!({"id": "module:merge"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert!(!menu_open(&h) && !has(&h, "mergesFrameMenu"), "a module switch closes it");
    // a module that doesn't draw the view (by command: a click would be "a click outside" itself)
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(menu_open(&h));
    h.app.run("view.map", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(!menu_open(&h), "the map. tab doesn't draw it: closed");
    h.app.run("view.merges", json!({})).unwrap();
    h.settle(SETTLE);
    assert!(!menu_open(&h) && !has(&h, "mergesFrameMenu"), "and it doesn't come back");
    // the view with nothing to show (every kind unpicked: "No merges match the filters"): closed too
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(menu_open(&h));
    h.app.run("merge.filter", json!({"kinds": []})).unwrap();
    h.settle(SETTLE);
    assert!(order(&h).is_empty(), "nothing shows");
    assert!(!menu_open(&h) && !has(&h, "mergesFrameMenu"), "an empty view closes it");
    let _ = std::fs::remove_dir_all(&dir);
}

/// none. in the kind. / confidence. menus means nothing picked: no group shows (not all), the
/// button says `none.`; merge.filter keeps `null` = all and `[]` = none.
#[test]
fn none_in_the_filters_shows_no_groups() {
    let (mut h, dir, pano, hdr) = page("filternone");
    assert_eq!(order(&h).len(), 2);
    let all = |h: &Headless| order(h).len();
    let button = |h: &Headless, id: &str| {
        let r = widget(h, id);
        h.view.visible_texts().into_iter().find(|(_, tr, _)| r.contains(tr.center())).map(|(s, _, _)| s).unwrap_or_default()
    };
    // the menus' none. rows
    click(&mut h, "button:mergesFilterKind");
    click(&mut h, "menu:mergesFilterKind-none");
    h.settle(SETTLE);
    assert_eq!(all(&h), 0, "kind. none.: no groups");
    assert!(texts(&h).contains(&"No merges match the filters".to_string()), "the empty view says why");
    assert_eq!(button(&h, "button:mergesFilterKind"), "kind. none.");
    click(&mut h, "button:mergesFilterKind");
    click(&mut h, "menu:mergesFilterKind-all");
    h.settle(SETTLE);
    assert_eq!(all(&h), 2, "all. brings them back");
    assert_eq!(button(&h, "button:mergesFilterKind"), "kind. all.");
    click(&mut h, "button:mergesFilterConfidence");
    click(&mut h, "menu:mergesFilterConfidence-none");
    h.settle(SETTLE);
    assert_eq!(all(&h), 0, "confidence. none.: no groups");
    assert_eq!(button(&h, "button:mergesFilterConfidence"), "confidence. none.");
    // the command: [] none, null all, missing leaves it
    let r = h.app.run("merge.filter", json!({"confidences": null})).unwrap();
    assert_eq!(r["confidences"], serde_json::Value::Null);
    assert_eq!(h.app.run("merge.filter", json!({"kinds": []})).unwrap()["kinds"], json!([]));
    h.settle(SETTLE);
    assert_eq!(all(&h), 0);
    h.app.run("merge.filter", json!({"camera": null})).unwrap();
    assert!(crate::merges::view_prefs(&h.app).kinds_none, "a missing key leaves it");
    h.app.run("merge.filter", json!({"kinds": null})).unwrap();
    h.settle(SETTLE);
    assert_eq!(all(&h), 2, "null: all");
    // an older ui.json (kinds: [] meant all, no none flag) still means all
    let v: crate::merges::MergeView = serde_json::from_value(json!({"kinds": [], "confidences": []})).unwrap();
    assert!(!v.kinds_none && !v.confidences_none);
    let _ = (pano, hdr);
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⊖ / ⌫ act on the picked frames of the groups that are unfolded in the view, and the title says
/// how many frames in how many groups.
#[test]
fn the_popup_acts_on_the_frames_in_view_only() {
    let (mut h, dir, pano, hdr) = page("popupscope");
    grid(&mut h);
    let (a, b) = (pano.photos[1], hdr.photos[0]);
    click(&mut h, &format!("mergeCard:{}:{}", pano.id, a.0));
    cmd_click(&mut h, &format!("mergeCard:{}:{}", hdr.id, b.0));
    assert_eq!(h.app.session.selection.ids.len(), 2);
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, a.0));
    assert!(texts(&h).contains(&"2 frames in 2 groups picked.".to_string()), "{:?}", texts(&h));
    click(&mut h, "button:mergesFrameCancel");
    // fold the bracket's group: its picked frame is out of sight and out of the question
    crate::folds::set(&h.view.ctx, &format!("merges.group:{}", hdr.id), false);
    h.settle(SETTLE);
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, a.0));
    assert!(texts(&h).contains(&"1 frame in 1 group picked.".to_string()), "{:?}", texts(&h));
    click(&mut h, "button:mergesFrameOutOfGroup");
    assert!(h.step_until(T, |h| !group_photos(h, &pano.id).contains(&a)), "the shown frame is out");
    assert!(group_photos(&h, &hdr.id).contains(&b), "the folded group's frame is untouched");
    // ⌫ the same
    key(&mut h, "backspace", false);
    assert!(has(&h, "mergesFrameMenu") && texts(&h).contains(&"1 frame in 1 group picked.".to_string()), "{:?}", texts(&h));
    click(&mut h, "button:mergesFrameCancel");
    let _ = std::fs::remove_dir_all(&dir);
}

/// One view of the groups for the whole question, not one per picked frame.
#[test]
fn the_popup_works_the_view_out_once() {
    let (mut h, dir, pano, _hdr) = page("popuponce");
    grid(&mut h);
    let ids: Vec<u64> = pano.photos.iter().take(4).map(|p| p.0).collect();
    // all out of the group: the question looks at every one of them
    h.app.run("merge.setFrames", json!({"id": pano.id, "photos": [pano.photos[4].0]})).unwrap();
    h.settle(SETTLE);
    let before = crate::merges::VIEW_DATA_CALLS.with(|c| c.get());
    crate::merges::start_frame_menu(&h.app, &h.view.ctx, ids, egui::pos2(300.0, 300.0));
    let calls = crate::merges::VIEW_DATA_CALLS.with(|c| c.get()) - before;
    assert_eq!(calls, 1, "compute_view_data once per question");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Esc closes the popup and nothing else: the key never reaches the keymap (Esc is Back to Grid,
/// which would also let go of the tool in use).
#[test]
fn escape_closes_the_popup_before_any_shortcut() {
    let (mut h, dir, pano, _hdr) = page("popupesc");
    grid(&mut h);
    let f = pano.photos[1];
    h.app.ui.tool = "crop".into();
    click(&mut h, &format!("mergeMinus:{}:{}", pano.id, f.0));
    assert!(menu_open(&h));
    key(&mut h, "escape", false);
    assert!(!menu_open(&h), "closed");
    assert_eq!(h.app.ui.tool, "crop", "Esc went to the popup, not to Back to Grid");
    key(&mut h, "escape", false);
    assert!(h.app.ui.tool.is_empty(), "the next Esc is the keymap's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tile header's wrapped lines are 10 px apart at least (DESIGN.md §15).
#[test]
fn the_tile_header_lines_are_ten_px_apart() {
    let gap = std::hint::black_box(crate::merges::HEAD_LINE_GAP);
    assert!(gap >= 10.0, "{gap}");
}
