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

/// KEY-HIDEPINS, KEY-HISTOGRAM, KEY-PICK, KEY-PHOTOSPANEL: H hides the mask pins while masking
/// (Remove elsewhere), ⌘0 shows / hides the histogram, and the `desktop` profile picks with Z and
/// toggles the photos panel with P, as Lightroom desktop does.
#[test]
fn lightroom_desktop_keys() {
    let mut h = demo();
    exec(&mut h, "view.detail", json!({}));
    h.step();
    let hist = h.app.ui.histogram;
    key(&mut h, "0", json!({"cmd": true}));
    assert_eq!(h.app.ui.histogram, !hist, "⌘0 = histogram");
    // H while masking: pins; outside it: the Remove tool
    exec(&mut h, "panel.masking", json!({}));
    h.step();
    assert_eq!(h.app.ui.right, RightPanel::Masking);
    assert!(h.app.ui.mask_pins);
    key(&mut h, "h", json!({}));
    assert!(!h.app.ui.mask_pins, "H hides the pins");
    assert_eq!(h.app.ui.right, RightPanel::Masking, "and stays in Masking");
    key(&mut h, "h", json!({}));
    assert!(h.app.ui.mask_pins);
    exec(&mut h, "panel.edit", json!({}));
    h.step();
    key(&mut h, "h", json!({}));
    assert_eq!(h.app.ui.right, RightPanel::Remove, "H = Remove outside Masking");
    // the desktop profile: Z picks, P shows / hides the photos (left) panel
    exec(&mut h, "keys.set", json!({"profile": "desktop"}));
    let id = h.app.session.active().expect("active photo");
    exec(&mut h, "photo.flag", json!({"flag": "none"}));
    key(&mut h, "z", json!({}));
    assert_eq!(h.app.session.catalog.photo(id).unwrap().flag, lightcraft_engine::catalog::Flag::Pick, "Z = pick");
    let left = h.app.ui.left_panel;
    key(&mut h, "p", json!({}));
    assert_eq!(h.app.ui.left_panel, !left, "P = photos panel");
    assert_eq!(h.app.session.catalog.photo(id).unwrap().flag, lightcraft_engine::catalog::Flag::Pick, "P didn't flag");
    // the built-ins keep P = pick
    exec(&mut h, "keys.set", json!({"profile": "lightroom"}));
    exec(&mut h, "photo.flag", json!({"flag": "none"}));
    key(&mut h, "p", json!({}));
    assert_eq!(h.app.session.catalog.photo(id).unwrap().flag, lightcraft_engine::catalog::Flag::Pick);
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
            rejected: Vec::new(),
            drawn: false,
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
    let Some(crate::state::Dialog::SyncSettings { groups, match_exposure, auto_align, match_color, .. }) = h.app.ui.dialog.clone() else {
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
    let Some(crate::state::Dialog::SyncSettings { groups, match_exposure, auto_align, match_color, .. }) = h.app.ui.dialog.clone() else {
        panic!("no sync dialog")
    };
    assert!(match_exposure && !auto_align && match_color);
    assert!(!groups.contains(&first), "{first} stays unchecked");
}

/// U01: the Sync dialog's per-setting checklist (Lightroom's): none., tick one slider, the Basic
/// Tone heading ticks its six, mod. picks what was changed, a typed name saves a subset; only the
/// ticked sliders go to the other photos.
#[test]
fn sync_dialog_per_setting_checklist_and_subsets() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(
        &mut h,
        "develop.set",
        json!({"values": {"light.exposure": 1.0, "light.contrast": 30, "detail.sharpenAmount": 90, "detail.nrLuminance": 40}}),
    );
    exec(&mut h, "library.select", json!({"ids": ids, "active": ids[0]}));
    exec(&mut h, "dialog.syncSettings", json!({}));
    // the (centred) window takes a few frames to find its size
    for _ in 0..8 {
        h.step();
    }
    let click = |h: &mut Headless, w: &str| {
        let r = h.request("ui.clickWidget", json!({"id": w}), T);
        assert_eq!(r["ok"], true, "{w}: {r}");
        h.step();
    };
    let groups = |h: &Headless| match &h.app.ui.dialog {
        Some(crate::state::Dialog::SyncSettings { groups, .. }) => groups.clone(),
        d => panic!("dialog: {d:?}"),
    };
    click(&mut h, "syncGroup:none");
    assert!(groups(&h).is_empty());
    click(&mut h, "syncGroup:light");
    assert_eq!(groups(&h).len(), 6, "the heading ticks its six sliders");
    click(&mut h, "syncGroup:contrast");
    assert_eq!(groups(&h).len(), 5);
    click(&mut h, "syncGroup:modified");
    let modified = groups(&h);
    for g in ["exposure", "contrast", "sharpening", "luminanceNoise"] {
        assert!(modified.iter().any(|x| x == g), "{g}: {modified:?}");
    }
    click(&mut h, "syncGroup:none");
    click(&mut h, "syncGroup:sharpening");
    click(&mut h, "syncGroup:exposure");
    // save the two as a subset
    click(&mut h, "field:syncGroupSubset");
    h.request("ui.text", json!({"text": "Sharp + exp"}), T);
    h.step();
    click(&mut h, "syncGroup:saveSubset");
    assert_eq!(h.app.session.copy_subsets.len(), 1, "saved");
    assert_eq!(h.app.session.copy_subsets[0].groups, ["exposure", "sharpening"]);
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    let d = h.app.session.develop_of(lightcraft_catalog::PhotoId(ids[1])).unwrap();
    assert_eq!((d.light.exposure, d.light.contrast, d.detail.sharpen_amount, d.detail.nr_luminance), (1.0, 0.0, 90.0, 0.0));
    // remembered per setting
    assert_eq!(h.app.ui.sync_prefs.groups.as_deref(), Some(&["exposure".to_string(), "sharpening".to_string()][..]));
}

/// U02: Copy Settings' paste mode (relative changes since the first history state) and ⌘V asking
/// to merge or replace masks when the photo has its own.
#[test]
fn copy_relative_and_paste_asks_merge_or_replace_masks() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 1.0}}));
    exec(&mut h, "mask.add", json!({"kind": "radial", "name": "Face"}));
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 0.5}}));
    exec(&mut h, "mask.add", json!({"kind": "linear", "name": "Sky"}));
    exec(&mut h, "dialog.copySettings", json!({}));
    for _ in 0..8 {
        h.step();
    }
    let click = |h: &mut Headless, w: &str| {
        let r = h.request("ui.clickWidget", json!({"id": w}), T);
        assert_eq!(r["ok"], true, "{w}: {r}");
        h.step();
    };
    click(&mut h, "copyGroup:none");
    click(&mut h, "copyGroup:exposure");
    click(&mut h, "copyGroup:masks");
    click(&mut h, "combo:copyPasteMode");
    click(&mut h, "copyPasteMode:sinceFirst");
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    assert!(h.app.session.clipboard_base.is_some(), "relative");
    // ⌘V onto a photo with a mask of its own: asked, merge is the default
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    key(&mut h, "V", json!({"cmd": true}));
    h.step();
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::PasteMasks { merge: true, .. })), "{:?}", h.app.ui.dialog);
    for _ in 0..8 {
        h.step();
    }
    click(&mut h, "pasteMasks:replaceMasks");
    assert_eq!(h.request("ui.dialog.confirm", json!({}), T)["ok"], true);
    let d = h.app.session.develop_of(lightcraft_catalog::PhotoId(ids[1])).unwrap();
    assert!((d.light.exposure - 1.5).abs() < 1e-9, "+0.5 EV on its own 1.0: {}", d.light.exposure);
    assert_eq!(d.masks.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Sky"], "replaced");
}

/// Sync prefs saved by older versions (coarse groups) open as their per-setting checkboxes.
#[test]
fn old_coarse_sync_prefs_open_as_per_setting_checkboxes() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    h.app.ui.sync_prefs.groups = Some(vec!["light".into(), "detail".into(), "gone".into()]);
    exec(&mut h, "dialog.syncSettings", json!({}));
    let Some(crate::state::Dialog::SyncSettings { groups, .. }) = h.app.ui.dialog.clone() else { panic!("no sync dialog") };
    assert_eq!(groups.len(), 10, "{groups:?}");
    assert!(groups.contains(&"blacks".to_string()) && groups.contains(&"colorNoise".to_string()));
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

/// U05: crop extras — the overlays to cycle (Choose Overlays to Cycle), the Aspect Ratios / Center
/// / Fifths overlays, As Shot aspect, and the Before & After button cycling the layouts.
#[test]
fn crop_overlays_cycle_the_chosen_ones_and_before_after_cycles_layouts() {
    use crate::state::{BeforeAfter, CropOverlay};
    let mut h = demo();
    let first = h.app.session.catalog.photos().next().unwrap().id.0;
    exec(&mut h, "library.select", json!({"ids": [first]}));
    let r = h.request("ui.set", json!({"view": "detail", "right": "crop"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(60));
    exec(&mut h, "view.cropOverlaysToCycle", json!({"overlays": ["thirds", "aspectRatios", "fifths"]}));
    h.app.ui.crop_overlay = CropOverlay::Thirds;
    for want in [CropOverlay::AspectRatios, CropOverlay::Fifths, CropOverlay::Thirds] {
        exec(&mut h, "view.cropOverlay", json!({}));
        assert_eq!(h.app.ui.crop_overlay, want);
    }
    exec(&mut h, "view.cropOverlay", json!({"overlay": "center"}));
    assert_eq!(h.app.ui.crop_overlay, CropOverlay::Center);
    let r = h.request("engine.execute", json!({"command": "view.cropOverlay", "params": {"overlay": "nope"}}), T);
    assert_eq!(r["ok"], false, "{r}");
    // the panel's cycle. menu ticks one off
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": "button:cropOverlayCycle"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let r = h.request("ui.clickWidget", json!({"id": "cropOverlayCycle:fifths"}), T);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.crop_overlay_cycle, [CropOverlay::Thirds, CropOverlay::AspectRatios]);
    // As Shot locks the photo's own shape
    exec(&mut h, "crop.aspect", json!({"aspect": "asShot"}));
    let id = h.app.session.active().unwrap();
    assert!(h.app.session.develop_of(id).unwrap().crop.aspect.is_some());
    // Before & After: the button steps through the layouts
    for want in [BeforeAfter::SideBySide, BeforeAfter::TopBottom, BeforeAfter::Split, BeforeAfter::SplitTopBottom, BeforeAfter::SideBySide] {
        exec(&mut h, "view.beforeAfterCycle", json!({}));
        assert_eq!(h.app.ui.before_after, want);
    }
}

/// R29: photos are shown in Display P3 values on a Mac window (Auto), sRGB when asked; the
/// headless host (screenshots) stays sRGB; switching re-renders.
#[test]
fn view_space_follows_the_setting_and_skips_the_headless_host() {
    let mut h = demo();
    let r = exec(&mut h, "app.viewSpace", json!({}));
    assert_eq!(r["space"], "auto");
    assert_eq!(r["displayP3"], cfg!(target_os = "macos"));
    h.step();
    assert!(!h.app.renderer.display_p3, "the headless host keeps sRGB");
    // as the desktop app (not headless)
    h.app.headless_host = false;
    exec(&mut h, "app.viewSpace", json!({"space": "displayP3"}));
    h.step();
    assert!(h.app.renderer.display_p3);
    exec(&mut h, "app.viewSpace", json!({"space": "srgb"}));
    h.step();
    assert!(!h.app.renderer.display_p3);
    let r = h.request("engine.execute", json!({"command": "app.viewSpace", "params": {"space": "adobeRgb"}}), T);
    assert_eq!(r["ok"], false, "{r}");
    // remembered with the UI state
    let back: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&h.app.ui).unwrap()).unwrap();
    assert_eq!(back.settings.view_space, crate::display_p3::ViewSpace::Srgb);
}

/// U10: point curve extras — Snap to Grid (points land on the eighths), Copy / Paste a channel,
/// Show All Curves (a view option, remembered).
#[test]
fn point_curve_snap_copy_paste_and_show_all() {
    let mut h = demo();
    let first = h.app.session.catalog.photos().next().unwrap().id.0;
    exec(&mut h, "library.select", json!({"ids": [first]}));
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit", "openSections": ["toneCurve"]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.app.ui.curve_channel = "red".into();
    exec(&mut h, "curve.snapToGrid", json!({"on": true}));
    h.settle(Duration::from_secs(60));
    // a press a little off the 3/8 × 5/8 grid crossing adds a point on it (a tiny drag keeps it there)
    let r = h.request("ui.dragWidget", json!({"id": "curve", "fx": 0.39, "fy": 0.39, "dx": 1.0, "dy": 0.0, "steps": 2}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let id = h.app.session.active().unwrap();
    let red = h.app.session.develop_of(id).unwrap().curve.red.clone();
    assert!(red.iter().any(|p| (p.x - 0.375).abs() < 1e-9 && (p.y - 0.625).abs() < 1e-9), "snapped: {red:?}");
    // copy red, paste onto blue
    exec(&mut h, "curve.copyChannel", json!({}));
    exec(&mut h, "curve.pasteChannel", json!({"channel": "blue"}));
    assert_eq!(h.app.session.develop_of(id).unwrap().curve.blue, red);
    let r = h.request("engine.execute", json!({"command": "curve.pasteChannel", "params": {"channel": "purple"}}), T);
    assert_eq!(r["ok"], false, "{r}");
    // show all curves: a remembered view option
    exec(&mut h, "curve.showAll", json!({}));
    assert!(h.app.ui.curve_show_all);
    h.step();
    let back: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&h.app.ui).unwrap()).unwrap();
    assert!(back.curve_show_all && back.curve_snap);
}

/// U11: ⌥-drag a marquee in the Remove tool deletes the spots inside it (one undo step); a Fill
/// brush adds a content-aware Fill spot (R17).
#[test]
fn remove_tool_marquee_deletes_spots_and_fill_mode_adds_fill_spots() {
    let mut h = demo();
    let first = h.app.session.catalog.photos().next().unwrap().id.0;
    exec(&mut h, "library.select", json!({"ids": [first]}));
    let r = h.request("ui.set", json!({"view": "detail", "right": "remove"}), T);
    assert_eq!(r["ok"], true, "{r}");
    for (x, y) in [(0.2, 0.2), (0.3, 0.25), (0.8, 0.8)] {
        exec(&mut h, "spot.add", json!({"mode": "heal", "points": [[x, y]], "size": 0.02}));
    }
    let id = h.app.session.active().unwrap();
    // engine: rejects junk, deletes what's inside (corners in any order)
    let r = h.request("engine.execute", json!({"command": "spot.deleteInRect", "params": {"rect": [0, 0, 1]}}), T);
    assert_eq!(r["ok"], false, "{r}");
    let undo = h.app.session.undo.len();
    let r = exec(&mut h, "spot.deleteInRect", json!({"rect": [0.35, 0.3, 0.1, 0.1]}));
    assert_eq!(r["deleted"], 2);
    assert_eq!(h.app.session.develop_of(id).unwrap().spots.len(), 1);
    assert_eq!(h.app.session.undo.len(), undo + 1, "one undo step");
    // ⌥-drag on the photo across the last spot
    h.settle(Duration::from_secs(60));
    let r = h.request("ui.pointer", json!({"alt": true, "events": [{"kind": "down", "x": 0.7, "y": 0.7}, {"x": 0.75, "y": 0.75}, {"x": 0.9, "y": 0.9}, {"kind": "up", "x": 0.9, "y": 0.9}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(60));
    assert!(h.app.session.develop_of(id).unwrap().spots.is_empty(), "the marquee deleted it");
    // Fill: a click with the Fill tool adds a Fill spot
    h.app.ui.tool = "fill".into();
    let r = h.request("ui.pointer", json!({"events": [{"kind": "down", "x": 0.5, "y": 0.5}, {"kind": "up", "x": 0.5, "y": 0.5}]}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(Duration::from_secs(60));
    let spots = h.app.session.develop_of(id).unwrap().spots.clone();
    assert_eq!(spots.len(), 1, "{spots:?}");
    assert_eq!(spots[0].mode, lightcraft_develop::SpotMode::Fill);
}
