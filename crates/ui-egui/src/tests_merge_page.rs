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
    for w in ["merges:bar", "merges:filters", "mergePage:right", "mergePage:footer", "button:mergePageMerge", "button:mergePageSendBack"] {
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

    // a frame clicked in the panel goes out of the group and back
    let out = g0.photos[1];
    click(&mut h, &format!("mergePageCard:{}:{}", g0.id, out.0));
    assert!(h.step_until(T, |h| !merge_find::group(&h.app.session.catalog, &g0.id).unwrap().photos.contains(&out)), "taken out");
    h.advance_time(1.0);
    click(&mut h, &format!("mergePageCard:{}:{}", g0.id, out.0));
    assert!(h.step_until(T, |h| merge_find::group(&h.app.session.catalog, &g0.id).unwrap().photos.contains(&out)), "back");
    h.advance_time(1.0);
    h.settle(SETTLE);

    // filters narrow: kind., confidence., camera.
    click(&mut h, "button:mergesKindhdr");
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![hdr.id.clone()]);
    assert!(rect_of(&h, &format!("group:merges:group:{}", pano.id)).is_none());
    audit(&h, "kind. hdr.");
    shot(&mut h, "mergepage-3-filter.png");
    let r = h.request("ui.clickWidget", json!({"id": "button:mergesKindpano", "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2, "⌘-click adds a kind");
    click(&mut h, "button:mergesKindAll");
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
    assert!(!crate::merge::page_options(&h.app, next.kind.command()).auto_settings);
    // M (as merge.) opens the Merge box on the group's kind, with the page's options
    key(&mut h, "M", false);
    match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => {
            assert_eq!(opts.command, next.kind.command(), "on the group's kind");
            assert!(!opts.auto_settings, "the options shown");
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
    assert!(!task.options.auto_settings, "with the options shown");
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
        d.merge.panorama.stack && d.merge.hdr.auto_settings && d.merge.panorama.auto_crop && d.merge.hdr.hide_originals && !d.merge.hdr.merges_only
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

/// per.row.: the cards' size from how many frames go across; smaller. / bigger. step it; the
/// layout., aspect., per.row. and kind. pick are remembered per screen size (his MacBook and his
/// big display each keep theirs); clean at 1100, 1500 and 2560 points wide.
#[test]
fn per_row_and_per_screen_memory() {
    let (mut h, dir, _pano, _hdr) = page("perrow");
    h.app.run("merge.layout", json!({"layout": "grid"})).unwrap();
    // the first group (in view at every size)
    let first = merge_find::group(&h.app.session.catalog, &order(&h)[0]).unwrap();
    let mut widths = Vec::new();
    for n in [2u64, 4, 6] {
        let r = h.app.run("merge.layout", json!({"perRow": n})).unwrap();
        assert_eq!(r["perRow"], n);
        h.settle(SETTLE);
        let pc = cards(&h, &first.id);
        let head = widget(&h, &format!("group:merges:group:{}", first.id));
        let want = (head.width() - 12.0 * (n as f32 - 1.0)) / n as f32;
        let w = pc.iter().map(|(_, r)| r.width()).fold(0.0, f32::max);
        assert!((w - want).abs() <= want * 0.35, "{n} per row: {w} vs {want}");
        let first_row = pc.iter().filter(|(_, r)| (r.top() - pc[0].1.top()).abs() < 0.5).count();
        assert_eq!(first_row, (n as usize).min(first.photos.len()), "{n} across");
        widths.push(w);
    }
    assert!(widths[0] > widths[1] && widths[1] > widths[2], "more per row, smaller cards: {widths:?}");
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

/// kind. and confidence. chips count the groups of the scope picked in the sidebar: all. 2,
/// panorama. 1, hdr. 1, focus.stack. 0 (a full chip with a quiet 0); click = only that kind,
/// ⌘-click adds, all. every kind.
#[test]
fn kind_and_confidence_chips_count_the_groups() {
    let (mut h, dir, pano, hdr) = page("chips");
    let chip_count = |h: &Headless, id: &str| -> String {
        let r = widget(h, &format!("button:{id}"));
        let texts: Vec<String> =
            h.view.visible_texts().into_iter().filter(|(s, t, _)| r.contains(t.center()) && s.parse::<usize>().is_ok()).map(|(s, _, _)| s).collect();
        texts.last().cloned().unwrap_or_default()
    };
    assert_eq!(chip_count(&h, "mergesKindAll"), "2");
    assert_eq!(chip_count(&h, "mergesKindpano"), "1");
    assert_eq!(chip_count(&h, "mergesKindhdr"), "1");
    assert_eq!(chip_count(&h, "mergesKindstack"), "0", "zero is a quiet 0");
    assert_eq!(chip_count(&h, "mergesKindhdrPano"), "0");
    let stack = widget(&h, "button:mergesKindstack");
    assert!((stack.height() - 24.0).abs() < 0.5, "a full chip");
    let n_conf = |c: lightcraft_engine::merge_find::Confidence| [&pano, &hdr].iter().filter(|g| g.confidence() == c).count();
    for c in crate::merge_page::CONFIDENCES {
        assert_eq!(chip_count(&h, &format!("mergesConf{}", c.id())), n_conf(c).to_string(), "{c:?}");
    }
    click(&mut h, "button:mergesKindpano");
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![pano.id.clone()]);
    assert_eq!(chip_count(&h, "mergesKindhdr"), "1", "the counts don't follow their own pick");
    let r = h.request("ui.clickWidget", json!({"id": "button:mergesKindhdr", "cmd": true}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2);
    click(&mut h, "button:mergesKindhdr");
    h.settle(SETTLE);
    assert_eq!(order(&h), vec![hdr.id.clone()]);
    click(&mut h, "button:mergesKindAll");
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), 2);
    let c = pano.confidence();
    click(&mut h, &format!("button:mergesConf{}", c.id()));
    h.settle(SETTLE);
    assert_eq!(order(&h).len(), n_conf(c));
    click(&mut h, "button:mergesConfAll");
    // the counts follow the scope: the M262 folder alone has the bracket only
    h.app.run("library.source", json!({"kind": "libraryFolder", "path": dir.join("M262").to_string_lossy()})).unwrap();
    h.settle(SETTLE);
    assert_eq!(chip_count(&h, "mergesKindAll"), "1");
    assert_eq!(chip_count(&h, "mergesKindpano"), "0");
    audit(&h, "chips");
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
    assert!(task.options.auto_settings, "a missing key: the kind's default");
    let plan = format!("{:?}", task.job.kind);
    assert!(plan.contains("Cylindrical") && plan.contains("40.0"), "{plan}");
    let _ = std::fs::remove_dir_all(&dir);
}
