//! Headless tests of the keymap (Lightroom Classic profile, imported super keys, the shrt. page,
//! MIDI mappings) and of "where I left off".

use std::time::Duration;

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::state::{RightPanel, ViewMode};
use crate::{LightcraftApp, Services};

const T: Duration = Duration::from_secs(30);

fn demo() -> Headless {
    let services = Services { png: None, ..Default::default() };
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), services);
    Headless::new(app, [1400.0, 900.0], 1.0)
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

fn crop_aspect(h: &Headless) -> Option<(u32, u32)> {
    let id = h.app.session.active().expect("active photo");
    h.app.session.develop_of(id).unwrap_or_default().crop.aspect
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("lc-keys-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn classic_keys_do_the_classic_thing() {
    let mut h = demo();
    exec(&mut h, "keys.set", json!({"profile": "classic"}));
    // G grid, D develop (loupe + Edit panel), R crop, Q spot removal, E loupe, B quick collection
    key(&mut h, "d", json!({}));
    assert_eq!((h.app.ui.view, h.app.ui.right), (ViewMode::Detail, RightPanel::Edit));
    key(&mut h, "r", json!({}));
    assert_eq!(h.app.ui.right, RightPanel::Crop, "R = crop overlay");
    key(&mut h, "q", json!({}));
    assert_eq!(h.app.ui.right, RightPanel::Remove, "Q = spot removal");
    key(&mut h, "g", json!({}));
    assert_eq!(h.app.ui.view, ViewMode::PhotoGrid);
    key(&mut h, "e", json!({}));
    assert_eq!(h.app.ui.view, ViewMode::Detail, "E = loupe");
    // keys Classic doesn't change keep the built-in: P picks
    key(&mut h, "p", json!({}));
    let id = h.app.session.active().unwrap();
    assert_eq!(h.app.session.catalog.photo(id).unwrap().flag, lightcraft_catalog::Flag::Pick);
    // mouse back / forward = previous / next photo
    let first = h.app.session.active();
    let r = h.request("ui.click", json!({"x": 700, "y": 450, "button": "forward"}), T);
    assert_eq!(r["ok"], true);
    assert_ne!(h.app.session.active(), first, "mouse forward = next photo");
    h.request("ui.click", json!({"x": 700, "y": 450, "button": "back"}), T);
    assert_eq!(h.app.session.active(), first, "mouse back = previous photo");
    // the menus show Classic's keys (R is the crop's now, not the radial gradient's)
    let entries = crate::menus::menu_entries(&h.app);
    let sc = |id: &str| entries.iter().find(|e| e.id == id).and_then(|e| e.shortcut.clone());
    assert_eq!(sc("view.compare").as_deref(), Some("C"));
    // the desktop profile is back to the built-ins
    exec(&mut h, "keys.set", json!({"profile": "lightroom"}));
    key(&mut h, "d", json!({}));
    assert_eq!(h.app.ui.view, ViewMode::Detail);
}

#[test]
fn imported_super_keys_crop_and_paste() {
    let mut h = demo();
    let d = scratch("lrkeys");
    let path = d.join("bindings.json");
    std::fs::write(
        &path,
        r#"{"actions": [
          {"id": "crop_1x1", "label": "1 x 1", "kind": "url", "cmd": "crop?w=1&h=1", "mods": ["alt"], "key": "q"},
          {"id": "crop_16x9", "label": "16 x 9", "kind": "url", "cmd": "crop?w=16&h=9", "mods": ["alt"], "key": "x"},
          {"id": "crop_reset", "label": "Reset crop", "kind": "url", "cmd": "cropreset", "mods": ["alt"], "key": "r"},
          {"id": "exp_up", "label": "Exposure +1/3", "kind": "url", "cmd": "param?name=Exposure&delta=0.33", "mods": [], "key": null}
        ]}"#,
    )
    .unwrap();
    let r = exec(&mut h, "keys.import", json!({"source": "lrkeys", "path": path}));
    assert_eq!(r["bindings"], 3, "{r}");
    exec(&mut h, "keys.set", json!({"profile": "classic"}));
    h.request("ui.set", json!({"view": "detail"}), T);
    key(&mut h, "q", json!({"alt": true}));
    assert_eq!(crop_aspect(&h), Some((100, 100)), "⌥Q = 1x1 crop");
    key(&mut h, "x", json!({"alt": true}));
    assert_eq!(crop_aspect(&h), Some((1600, 900)), "⌥X = 16x9");
    key(&mut h, "r", json!({"alt": true}));
    assert_eq!(crop_aspect(&h), None, "⌥R = reset crop");
    // super-key commands
    let before = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap_or_default().light.exposure;
    exec(&mut h, "keys.nudge", json!({"control": "light.exposure", "dir": 1, "size": "large"}));
    let after = h.app.session.develop_of(h.app.session.active().unwrap()).unwrap_or_default().light.exposure;
    assert!((after - before - 0.33).abs() < 1e-4, "{before} → {after}");
    exec(
        &mut h,
        "keys.macro",
        json!({"steps": [{"command": "photo.flag", "params": {"flag": "reject"}}, {"command": "keys.send", "params": {"keys": "U"}}]}),
    );
    let id = h.app.session.active().unwrap();
    assert_eq!(h.app.session.catalog.photo(id).unwrap().flag, lightcraft_catalog::Flag::None, "U unflags after the reject");
    let r = h.request("engine.execute", json!({"command": "preset.applyByName", "params": {"name": "No Such Preset"}}), T);
    assert_eq!(r["ok"], false);
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn shortcuts_page_renders_and_records_keys() {
    let mut h = demo();
    exec(&mut h, "keys.set", json!({"profile": "classic"}));
    exec(&mut h, "app.settings", json!({"tab": "shortcuts"}));
    h.snapshot(T);
    for w in ["button:keysRecord", "combo:keysDraftAction", "field:keysSearch", "button:settingsTab-controllers"] {
        assert!(h.app.widgets.iter().any(|(id, _)| id == w), "{w} on the shrt. page");
    }
    // pick an action, rec., press the keys
    exec(&mut h, "keys.record", json!({"command": "crop.aspect", "params": {"aspect": "5x7"}, "label": "crop 5x7"}));
    key(&mut h, "7", json!({"alt": true}));
    assert!(h.app.keymap.recording.is_none());
    let b = h.app.keymap.file.bindings.iter().find(|b| b.keys == "Alt+7").expect("recorded");
    assert_eq!((b.command.as_str(), b.params.clone()), ("crop.aspect", json!({"aspect": "5x7"})));
    // the rec. button on the page does the same
    let r = h.request("ui.clickWidget", json!({"id": "button:keysRecord"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.keymap.recording, Some(None));
    key(&mut h, "5", json!({"alt": true, "shift": true}));
    assert!(h.app.keymap.file.bindings.iter().any(|b| b.keys == "Alt+Shift+5" && b.command == "crop.aspect"));
    // Escape cancels a recording
    exec(&mut h, "keys.record", json!({}));
    key(&mut h, "escape", json!({}));
    assert!(h.app.keymap.recording.is_none());
    // the controllers page renders too
    let r = h.request("ui.clickWidget", json!({"id": "button:settingsTab-controllers"}), T);
    assert_eq!(r["ok"], true);
    h.snapshot(T);
    assert!(h.app.widgets.iter().any(|(id, _)| id == "button:midiLearn"));
    // and the new binding works once the dialog is closed
    h.request("ui.dialog.cancel", json!({}), T);
    h.request("ui.set", json!({"view": "detail"}), T);
    key(&mut h, "7", json!({"alt": true}));
    assert_eq!(crop_aspect(&h), Some((500, 700)));
}

#[test]
fn midi_moves_sliders_and_runs_commands() {
    let mut h = demo();
    let d = scratch("monogram");
    let state = d.join("state.json");
    std::fs::write(
        &state,
        r#"{"profiles": [{"title": "Lightroom 1", "assignments": {
          "  Ak2": {"settings": {"pose": {"input": "Shadows = "}}},
          "  C7J": {"settings": {"press": {"key": "y", "modifiers": []}, "turn": {"input": "Exposure = "}}},
          "  H|r": {"settings": {"doubleTap": ["Blacks = ", "reset"]}}
        }}]}"#,
    )
    .unwrap();
    let out = d.join("LightCraft.monogram");
    let r = exec(&mut h, "keys.import", json!({"source": "monogram", "path": state, "write": out}));
    assert_eq!(r["midi"], 3, "{r}");
    assert_eq!(r["bindings"], 2, "the Exposure dial's two turn keys: {r}");
    let mono: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    // Monogram's import format: lists per module type (the slider sends a CC)
    let sliders = mono["assignments"]["slider"].as_array().unwrap();
    assert!(sliders.iter().any(|s| s["settings"]["pose"]["midi"] == "cc"), "{mono}");
    h.request("ui.set", json!({"view": "detail"}), T);
    let dev = |h: &Headless| h.app.session.develop_of(h.app.session.active().unwrap()).unwrap_or_default();
    // the Shadows slider at the top
    exec(&mut h, "keys.midi", json!({"midi": "cc", "number": 20, "value": 127}));
    assert_eq!(dev(&h).light.shadows, 100.0);
    // the Exposure dial sends keys (Core V1 firmware can't send relative MIDI): Ctrl+Alt+Shift+A
    // = one detent left, +B = right. Two right, one left = one step up.
    let e0 = dev(&h).light.exposure;
    let k = |h: &mut Headless, key: &str| h.request("ui.key", json!({"key": key, "ctrl": true, "alt": true, "shift": true}), T);
    k(&mut h, "B");
    k(&mut h, "B");
    k(&mut h, "A");
    h.step();
    assert!(dev(&h).light.exposure > e0 + 1e-6, "{} → {}", e0, dev(&h).light.exposure);
    // the dial's press = Y = before / after
    let ba = h.app.ui.before_after;
    let r = exec(&mut h, "keys.midi", json!({"bytes": [0x90, 60, 100]}));
    assert_eq!(r["done"][0]["run"], "view.beforeAfter", "{r}");
    h.step();
    assert_ne!(h.app.ui.before_after, ba);
    // MIDI learn takes the next message instead of running it
    exec(&mut h, "keys.learn", json!({"on": true}));
    exec(&mut h, "keys.midi", json!({"midi": "cc", "number": 20, "value": 0}));
    assert_eq!(dev(&h).light.shadows, 100.0, "not moved while learning");
    assert!(matches!(h.app.keymap.learn, Some(Some(m)) if m.number == 20));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn lands_where_i_left_off() {
    let mut h = demo();
    let all = h.app.session.visible_cloned();
    assert!(all.len() >= 6);
    let a = exec(&mut h, "album.create", json!({"name": "A"}))["id"].as_u64().unwrap();
    exec(&mut h, "album.addPhotos", json!({"id": a, "ids": [all[0].0, all[1].0, all[2].0]}));
    // in All Photos, work on the 5th photo
    exec(&mut h, "library.select", json!({"ids": [all[4].0]}));
    exec(&mut h, "develop.adjust", json!({"control": "light.exposure", "delta": 0.2}));
    h.step();
    // the album: lands on its first photo (no visit yet); move to its 3rd
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    let in_a = h.app.session.visible_cloned();
    assert_eq!(h.app.session.active(), in_a.first().copied());
    exec(&mut h, "library.select", json!({"ids": [in_a[2].0]}));
    h.step();
    // back in All Photos: on the 5th again, marked
    exec(&mut h, "library.source", json!({"kind": "all"}));
    assert_eq!(h.app.session.active(), Some(all[4]), "back on the photo worked on in All Photos");
    assert_eq!(h.app.left_off.marker, Some(all[4]));
    exec(&mut h, "library.select", json!({"ids": [all[1].0]}));
    h.step();
    // and the album again: on its 3rd
    exec(&mut h, "library.source", json!({"kind": "album", "id": a}));
    assert_eq!(h.app.session.active(), Some(in_a[2]));
    assert_eq!(h.app.left_off.marker, Some(in_a[2]));
    h.step();
    let shot = h.snapshot(T);
    assert!(shot.width() > 0);
    // left off. goes back to the last place of all — the album's 3rd photo — from anywhere
    h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    h.step();
    exec(&mut h, "library.source", json!({"kind": "recentlyDeleted"}));
    h.request("ui.set", json!({"view": "photoGrid", "right": "none"}), T);
    h.step();
    let r = exec(&mut h, "view.resumeLastLeftOff", json!({}));
    assert_eq!(r["photo"], in_a[2].0, "{r}");
    assert_eq!(h.app.session.source, lightcraft_engine::LibrarySource::Album(lightcraft_catalog::AlbumId(a)));
    assert_eq!(h.app.session.active(), Some(in_a[2]));
    assert_eq!((h.app.ui.view, h.app.ui.right), (ViewMode::Detail, RightPanel::Edit), "and the view it was in");
    // it survives a save / load of the UI state
    let saved = serde_json::to_value(&h.app.ui).unwrap();
    let back: crate::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(back.left_off, h.app.ui.left_off);
    // the top bar button is there
    assert!(h.app.widgets.iter().any(|(id, _)| id == "button:leftOff"));
}

/// Monogram's Lightroom protocol (what its service sends the Lightroom plugin) drives LightCraft:
/// dial detents with Monogram's step, a slider by position, reset, one-shot inputs, and cycle lists
/// (crop ratios) by slider position or dial detent.
#[test]
fn monogram_lightroom_lines_drive_lightcraft() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail"}), T);
    let dev = |h: &Headless| h.app.session.develop_of(h.app.session.active().unwrap()).unwrap_or_default();
    let line = |h: &mut Headless, l: &str| exec(h, "keys.lrinput", json!({"line": l}));
    let e0 = dev(&h).light.exposure;
    line(&mut h, "Exposure = ^0.05,-10,10");
    line(&mut h, "Exposure = ^0.05,-10,10");
    assert!((dev(&h).light.exposure - e0 - 0.10).abs() < 1e-6, "two detents up: {}", dev(&h).light.exposure);
    line(&mut h, "Exposure = v0.05,-10,10");
    assert!((dev(&h).light.exposure - e0 - 0.05).abs() < 1e-6);
    line(&mut h, "Exposure = reset");
    assert_eq!(dev(&h).light.exposure, 0.0);
    // slider: hardware 0–255 across the input's range
    line(&mut h, "Shadows = 255,-100,100");
    assert_eq!(dev(&h).light.shadows, 100.0);
    line(&mut h, "Shadows = 0,-100,100");
    assert_eq!(dev(&h).light.shadows, -100.0);
    // one-shot: flag = pick → P
    line(&mut h, "flag = pick");
    h.step();
    assert_eq!(h.app.session.catalog.photo(h.app.session.active().unwrap()).unwrap().flag, lightcraft_catalog::Flag::Pick);
    // cycle by slider: the top of the throw = the last crop ratio (21 × 9); same zone again does nothing
    let r = line(&mut h, "lc_cycle_cropAspect = 255,0,1");
    assert_eq!(r["label"], "21 × 9", "{r}");
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = 250,0,1")["unchanged"], true);
    assert!(dev(&h).crop.aspect.is_some());
    // cycle by dial: next wraps to the first (As Shot), previous back to 21 × 9
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = ^1,0,1")["label"], "As Shot");
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = v1,0,1")["label"], "21 × 9");
    // a user list replaces the built-in one
    exec(&mut h, "keys.set", json!({"cycles": {"cropAspect": [{"label": "5 × 7", "command": "crop.aspect", "params": {"aspect": "5x7"}}]}}));
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = 10,0,1")["label"], "5 × 7");
    // the "adobe" (JSON) form Monogram sends LightCraft's own integration
    let j = |h: &mut Headless, v: Value| exec(h, "keys.lrinput", json!({"line": v.to_string()}));
    j(&mut h, json!({"input": "Exposure = ", "argv": [0, 0, 0.05, -1, -1, -99999, -5, 5, 1]}));
    assert!((dev(&h).light.exposure - 0.05).abs() < 1e-6, "dial up: {}", dev(&h).light.exposure);
    j(&mut h, json!({"input": "Contrast = ", "argv": [200, -1, -1, -1, -1, 40, -100, 100, -1]}));
    assert_eq!(dev(&h).light.contrast, 40.0, "slider value");
    j(&mut h, json!({"input": "Contrast = ", "argv": [1, -1, -1, -1, -1, -1, -1, -1, -1]}));
    assert_eq!(dev(&h).light.contrast, 0.0, "pressed = reset");
    exec(&mut h, "keys.set", json!({"cycles": {}}));
    let r = j(&mut h, json!({"input": "lc_cycle_cropAspect = ", "argv": [250, -1, -1, -1, -1, 0.99, 0, 1, -1]}));
    assert_eq!(r["label"], "21 × 9", "top of the slider: {r}");
    // junk doesn't panic
    let _ = h.request("engine.execute", json!({"command": "keys.lrinput", "params": {"line": "{\"input\": 3}"}}), T);
    let _ = h.request("engine.execute", json!({"command": "keys.lrinput", "params": {"line": "nonsense"}}), T);
    let _ = h.request("engine.execute", json!({"command": "keys.lrinput", "params": {"line": "Exposure = ^,,"}}), T);
}

#[test]
fn monogram_lc_cmd_opens_merge_and_it_remembers_the_kind() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    exec(&mut h, "keys.lrinput", json!({"line": "lc_cmd = dialog.merge"}));
    let kind = |h: &Headless| match &h.app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => opts.command.clone(),
        _ => String::new(),
    };
    assert_eq!(kind(&h), "merge.hdr", "Merge… opens on HDR the first time");
    // the panorama shortcut still opens the panorama dialog straight away
    exec(&mut h, "dialog.mergePanorama", json!({}));
    assert_eq!(kind(&h), "merge.panorama");
    exec(&mut h, "dialog.mergeFocus", json!({}));
    assert_eq!(kind(&h), "merge.focus");
    // Merge… opens on the kind merged last
    h.app.merge.last_kind = Some("merge.focus".into());
    exec(&mut h, "dialog.merge", json!({}));
    assert_eq!(kind(&h), "merge.focus");
}

#[test]
fn best_take_dialog_lists_people_and_builds_swaps() {
    let mut h = demo();
    let ids: Vec<lightcraft_catalog::PhotoId> = h.app.session.catalog.photos().take(2).map(|p| p.id).collect();
    for (k, x) in [(0, 0.30), (1, 0.32)] {
        let mut m = h.app.session.catalog.photo(ids[k]).unwrap().meta.clone();
        m.regions = vec![lightcraft_meta::Region {
            rect: lightcraft_geom::Rect { x0: x, y0: 0.2, x1: x + 0.1, y1: 0.35 },
            kind: lightcraft_meta::RegionKind::Face,
            name: Some("Kate".into()),
            description: None,
        }];
        h.app.session.commit("t", lightcraft_catalog::Op::SetMeta { id: ids[k], meta: Box::new(m) }).unwrap();
    }
    exec(&mut h, "library.select", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
    h.app.session.selection.active = Some(ids[0]);
    exec(&mut h, "dialog.mergeTake", json!({}));
    let Some(crate::state::Dialog::Merge { opts }) = h.app.ui.dialog.clone() else { panic!("no merge dialog") };
    assert_eq!(opts.command, "merge.take");
    assert_eq!((opts.base, opts.people.len()), (0, 1));
    assert_eq!(opts.people[0].name, "Kate");
    assert!(opts.params()["swaps"].as_array().unwrap().is_empty(), "everyone on the start photo: nothing to swap");
    let mut o = opts.clone();
    o.people[0].from = 1;
    let sw = o.params()["swaps"].clone();
    assert_eq!(sw[0]["donor"], 1, "{sw}");
    assert!((sw[0]["donorBox"][0].as_f64().unwrap() - 0.32).abs() < 1e-9 && (sw[0]["baseBox"][0].as_f64().unwrap() - 0.30).abs() < 1e-9);
    // the engine takes those params
    assert!(lightcraft_engine::merge::parse("merge.take", &o.params()).is_ok());
    // not found in photo 2: still pickable, from the spot in photo 1
    o.people[0].boxes[1] = None;
    let sw = o.params()["swaps"].clone();
    assert!((sw[0]["donorBox"][0].as_f64().unwrap() - 0.30).abs() < 1e-9, "{sw}");
}

#[test]
fn sync_dialog_remembers_its_checkboxes() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    exec(&mut h, "dialog.syncSettings", json!({}));
    let Some(crate::state::Dialog::SyncSettings { groups, match_exposure, auto_align, match_color }) = h.app.ui.dialog.clone() else {
        panic!("no sync dialog")
    };
    assert!(!match_exposure && !auto_align && !match_color, "match exposure / auto align / match colour start off");
    // tick Match exposure, untick the first group, then cancel
    let first = groups[0].clone();
    if let Some(crate::state::Dialog::SyncSettings { groups, match_exposure, match_color, .. }) = &mut h.app.ui.dialog {
        *match_exposure = true;
        *match_color = true;
        groups.retain(|g| *g != first);
    }
    h.step();
    h.request("ui.dialog.cancel", json!({}), T);
    // after a restart (the UI state saved and read back) it opens as it was left
    let back: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&h.app.ui).unwrap()).unwrap();
    h.app.ui.sync_prefs = back.sync_prefs;
    exec(&mut h, "dialog.syncSettings", json!({}));
    let Some(crate::state::Dialog::SyncSettings { groups, match_exposure, auto_align, match_color }) = h.app.ui.dialog.clone() else {
        panic!("no sync dialog")
    };
    assert!(match_exposure && !auto_align && match_color);
    assert!(!groups.contains(&first), "{first} stays unchecked");
}

#[test]
fn selection_actions_show_for_two_or_more_photos() {
    let mut h = demo();
    h.request("ui.set", json!({"module": "library", "view": "grid"}), T);
    let ids: Vec<u64> = h.app.session.catalog.photos().take(1).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    h.step();
    let has = |h: &Headless, w: &str| h.app.widgets.iter().any(|(id, _)| id == w);
    assert!(!has(&h, "button:sel-dialog.merge"), "one photo: no selection actions");
    let ids: Vec<u64> = h.app.session.catalog.photos().take(3).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    h.step();
    for w in ["button:sel-view.compare", "button:sel-dialog.syncSettings", "button:sel-dialog.merge", "button:sel-dialog.mergeTake"] {
        assert!(has(&h, w), "{w} missing");
    }
    assert_eq!(h.request("ui.clickWidget", json!({"id": "button:sel-dialog.merge"}), T)["ok"], true);
    h.step();
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::Merge { .. })));
    // the module tabs are there and the active one is known
    assert!(has(&h, "module:library") && has(&h, "module:develop"));
}
