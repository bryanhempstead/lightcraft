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
    assert_eq!(r["midi"], 4, "{r}");
    let mono: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(mono["assignments"]["  Ak2"]["settings"]["pose"]["midi"], "cc");
    h.request("ui.set", json!({"view": "detail"}), T);
    let dev = |h: &Headless| h.app.session.develop_of(h.app.session.active().unwrap()).unwrap_or_default();
    // the Shadows slider at the top
    exec(&mut h, "keys.midi", json!({"midi": "cc", "number": 20, "value": 127}));
    assert_eq!(dev(&h).light.shadows, 100.0);
    // the Exposure dial: two ticks right (two's complement), one left (64-offset)
    let e0 = dev(&h).light.exposure;
    exec(&mut h, "keys.midi", json!({"midi": "cc", "number": 30, "value": 2}));
    exec(&mut h, "keys.midi", json!({"midi": "cc", "number": 30, "value": 63}));
    assert!((dev(&h).light.exposure - e0 - 0.05).abs() < 1e-4, "{}", dev(&h).light.exposure);
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
