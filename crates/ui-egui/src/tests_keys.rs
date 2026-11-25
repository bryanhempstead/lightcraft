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
    // H held (a tap is fav., Bryan 2026-11-23) while masking: pins; outside it: the Remove tool
    let hold_h = |h: &mut Headless| {
        key(h, "h", json!({"state": "down"}));
        assert!(h.step_until(Duration::from_secs(3), |h| h.app.h_down.is_some_and(|(_, fired)| fired) && !h.app.h_replay));
        h.step();
        key(h, "h", json!({"state": "up"}));
        h.step();
    };
    exec(&mut h, "panel.masking", json!({}));
    h.step();
    assert_eq!(h.app.ui.right, RightPanel::Masking);
    assert!(h.app.ui.mask_pins);
    hold_h(&mut h);
    assert!(!h.app.ui.mask_pins, "H hides the pins");
    assert_eq!(h.app.ui.right, RightPanel::Masking, "and stays in Masking");
    hold_h(&mut h);
    assert!(h.app.ui.mask_pins);
    exec(&mut h, "panel.edit", json!({}));
    h.step();
    hold_h(&mut h);
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

/// The crop dials (press-and-turn: move / scale the photo in the crop) are one drag too: no
/// history step per detent while they turn, one after (Bryan, 2026-11-23: "super laggy").
#[test]
fn crop_dial_turns_are_one_drag() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail"}), T);
    exec(&mut h, "panel.crop", json!({}));
    exec(&mut h, "crop.set", json!({"rect": [0.1, 0.1, 0.9, 0.9]}));
    h.step();
    let id = h.app.session.active().unwrap();
    let steps = |h: &Headless| h.app.session.catalog.photo(id).unwrap().history.len();
    let before = steps(&h);
    for _ in 0..12 {
        exec(&mut h, "keys.lrinput", json!({"line": "CropPositionX = ^0.5,-100,100"}));
    }
    assert!(h.app.session.interaction.is_some(), "turning: one interaction");
    assert_eq!(steps(&h), before, "no history step per detent");
    assert!(h.step_until(Duration::from_secs(3), |h| h.app.session.interaction.is_none()));
    assert_eq!(steps(&h), before + 1);
}

/// A crop dial's drafts are cut out of one uncropped render of the photo: the whole turn asks
/// for one render (every detent used to be a whole-pipeline render: ~5 a second on Bryan's
/// edited M262 DNG), and the cut shows what the sharp render of the same crop shows.
#[test]
fn crop_dial_draws_its_detents_out_of_one_render() {
    use crate::render::Slot;
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    exec(&mut h, "crop.set", json!({"rect": [0.15, 0.2, 0.75, 0.8], "angle": 4.0}));
    exec(&mut h, "photo.flipHorizontal", json!({}));
    let sharp = |h: &Headless| {
        h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.quick.is_none() && Some(t.key) != h.app.reframe_key)
            && !h.app.renderer.is_pending(Slot::Main)
    };
    assert!(h.step_until(T, sharp), "first render: {}", h.app.renderer.diagnose());
    h.settle(T);
    let ref_shot = h.paint();
    let cut = |h: &Headless| h.app.reframe_key.is_some() && h.app.renderer.textures.get(&Slot::Main).map(|t| t.key) == h.app.reframe_key;
    // turn (back and forth, ending where it started) until the uncropped draft is on screen
    let t0 = std::time::Instant::now();
    let mut k = 0;
    while !cut(&h) && t0.elapsed() < T {
        let line = if k % 2 == 0 { "CropPositionX = ^0.5,-100,100" } else { "CropPositionX = v0.5,-100,100" };
        exec(&mut h, "keys.lrinput", json!({"line": line}));
        k += 1;
        for _ in 0..5 {
            h.step();
        }
    }
    assert!(cut(&h), "the uncropped draft: {}", h.app.renderer.diagnose());
    if k % 2 == 1 {
        exec(&mut h, "keys.lrinput", json!({"line": "CropPositionX = v0.5,-100,100"}));
    }
    let asked = h.app.renderer.wanted(Slot::Main);
    for k in 0..8 {
        let line = if k % 2 == 0 { "CropPositionX = ^0.5,-100,100" } else { "CropPositionX = v0.5,-100,100" };
        exec(&mut h, "keys.lrinput", json!({"line": line}));
        h.step();
        assert!(cut(&h), "detent {k}: still the cut");
        assert_eq!(h.app.renderer.wanted(Slot::Main), asked, "detent {k}: no render asked for");
    }
    assert!(h.app.session.interaction.is_some(), "still turning");
    let mid = h.paint();
    // let go: the sharp render of the same crop
    assert!(h.step_until(T, |h| h.app.session.interaction.is_none() && sharp(h)), "sharp after: {}", h.app.renderer.diagnose());
    let after = h.paint();
    let img = h.app.image_rect.unwrap().shrink(6.0);
    let diff = |a: &egui::ColorImage, b: &egui::ColorImage| {
        let (mut sum, mut n) = (0u64, 0u64);
        for y in img.top() as usize..img.bottom() as usize {
            for x in img.left() as usize..img.right() as usize {
                let (p, q) = (a.pixels[y * a.size[0] + x], b.pixels[y * b.size[0] + x]);
                sum += u64::from(p.r().abs_diff(q.r())) + u64::from(p.g().abs_diff(q.g())) + u64::from(p.b().abs_diff(q.b()));
                n += 3;
            }
        }
        sum as f64 / n.max(1) as f64
    };
    let (d, same) = (diff(&mid, &after), diff(&ref_shot, &after));
    eprintln!("cut vs sharp: mean |Δ| {d:.2}; sharp before vs after the turn {same:.2}");
    assert!(same < 1.0, "the turn came back to the same crop ({same:.2})");
    assert!(d < 6.0, "the cut shows the crop the sharp render shows (mean |Δ| {d:.2})");
}

/// The White Balance Selector: W (or the `pick.` chip) arms it, a click on the photo sets a Custom
/// white balance from that spot, Esc puts it away without picking.
#[test]
fn wb_selector_picks_from_a_click() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    h.step();
    let id = h.app.session.active().unwrap();
    h.request("ui.clickWidget", json!({"id": "button:wbPick"}), T);
    assert_eq!(h.app.ui.tool, "wbPicker", "pick. arms it");
    h.request("ui.key", json!({"key": "escape"}), T);
    h.step();
    assert_eq!(h.app.ui.tool, "", "Esc puts it away");
    assert_eq!(h.app.ui.right, RightPanel::Edit, "and stays in Develop");
    h.request("ui.key", json!({"key": "w"}), T);
    h.step();
    assert_eq!(h.app.ui.tool, "wbPicker", "W arms it");
    h.request("ui.key", json!({"key": "w"}), T);
    h.step();
    assert_eq!(h.app.ui.tool, "", "W again puts it away");
    h.request("ui.key", json!({"key": "w"}), T);
    h.step();
    let before = h.app.session.develop_of(id).unwrap().wb;
    let img = h.app.image_rect.unwrap();
    h.request("ui.click", json!({"x": img.left() + img.width() * 0.3, "y": img.top() + img.height() * 0.4}), T);
    h.step();
    let after = h.app.session.develop_of(id).unwrap().wb;
    assert_eq!(after.mode, lightcraft_develop::WbMode::Custom);
    assert!(after != before, "picked: {before:?} → {after:?}");
    assert_eq!(h.app.ui.tool, "", "one pick, then it goes away");
}

/// center.: a soft radial mask on the photo's centre (Masking opens to adjust it); a second
/// press finds the same mask; default. makes its shape and effect what center. makes next.
#[test]
fn center_mask_and_its_default() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    h.step();
    let id = h.app.session.active().unwrap();
    h.request("ui.clickWidget", json!({"id": "icon:quick.center"}), T);
    let d = h.app.session.develop_of(id).unwrap();
    let m = d.masks.iter().find(|m| m.name == crate::menus::CENTER_MASK).expect("a Center mask");
    assert!(m.adjust.exposure < 0.0, "the built-in effect: a darkening vignette");
    assert!(matches!(m.components[0].shape, lightcraft_develop::MaskShape::Radial { invert: true, .. }), "around the centre, not on it");
    assert_eq!(h.app.ui.right, RightPanel::Masking);
    let n = d.masks.len();
    exec(&mut h, "mask.center", json!({}));
    assert_eq!(h.app.session.develop_of(id).unwrap().masks.len(), n, "the same mask again, not a second");
    // change it, save as the default, and a fresh photo's center. gets it
    exec(&mut h, "mask.adjust", json!({"values": {"exposure": -0.5, "clarity": 20.0}}));
    exec(&mut h, "mask.centerDefault", json!({}));
    let other = h.app.session.visible_cloned()[1];
    exec(&mut h, "library.select", json!({"ids": [other.0]}));
    exec(&mut h, "mask.center", json!({}));
    let m2 = h.app.session.develop_of(other).unwrap().masks.iter().find(|m| m.name == crate::menus::CENTER_MASK).cloned().unwrap();
    assert_eq!((m2.adjust.exposure, m2.adjust.clarity), (-0.5, 20.0));
}

/// Set Lens: a manual lens on photos the camera couldn't name it for — name, focal length, and the
/// lens's fastest aperture rules out impossible estimates.
#[test]
fn set_lens_names_the_lens_and_its_focal_length() {
    let mut h = demo();
    exec(&mut h, "app.lenses", json!({"lenses": [{"name": "Summicron-M 1:2/50", "focal": 50}]}));
    let id = h.app.session.active().unwrap();
    exec(&mut h, "photo.setLens", json!({"lens": "Summicron-M 1:2/50", "focal": 50}));
    let m = h.app.session.catalog.photo(id).unwrap().meta.clone();
    assert_eq!((m.lens.as_str(), m.focal_mm), ("Summicron-M 1:2/50", Some(50.0)));
    assert_eq!(h.app.ui.settings.lenses.len(), 1);
}

/// The window's edge tabs hide and show the left panel, the right panel and the filmstrip.
#[test]
fn edge_tabs_hide_and_show_the_panels() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop"}), T);
    h.step();
    let (l, r, f) = (h.app.ui.left_panel, h.app.ui.right_panel, h.app.ui.filmstrip);
    for id in ["button:edgeLeft", "button:edgeRight", "button:edgeBottom"] {
        h.request("ui.clickWidget", json!({"id": id}), T);
        h.step();
    }
    assert_eq!((h.app.ui.left_panel, h.app.ui.right_panel, h.app.ui.filmstrip), (!l, !r, !f));
    h.request("ui.clickWidget", json!({"id": "button:edgeRight"}), T);
    h.step();
    assert_eq!(h.app.ui.right_panel, r, "and back");
}

/// fav.: a quick tap of H toggles the heart on the selected photos; H held does what H is bound
/// to instead (Bryan's keymap: the Hue panel) and leaves the heart alone.
#[test]
fn tapping_h_toggles_fav_and_holding_h_runs_its_binding() {
    let mut h = demo();
    let id = h.app.session.active().unwrap();
    let fav = |h: &Headless| h.app.session.catalog.photo(id).unwrap().favorite;
    h.request("ui.key", json!({"key": "h"}), T);
    h.step();
    assert!(fav(&h), "tap: fav. on");
    assert_eq!(h.app.session.catalog.count(&lightcraft_catalog::Filter { text: "fav:yes".into(), ..Default::default() }, |_| true), 1);
    h.request("ui.key", json!({"key": "h"}), T);
    h.step();
    assert!(!fav(&h), "tap again: off");
    // held: past H_HOLD the binding runs, the release does nothing more
    h.request("ui.key", json!({"key": "h", "state": "down"}), T);
    assert!(h.step_until(Duration::from_secs(3), |h| h.app.h_down.is_some_and(|(_, fired)| fired)), "the hold ran H's binding");
    h.request("ui.key", json!({"key": "h", "state": "up"}), T);
    h.step();
    assert!(h.app.h_down.is_none());
    assert!(!fav(&h), "a hold never toggles fav.");
}

/// A slider picked by its name: ↑ / ↓ and the wheel change it, → picks the next slider, a click on
/// anything without a name lets go (and the arrows go back to stepping photos).
#[test]
fn a_slider_picked_by_its_name_takes_the_arrows_and_the_wheel() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    h.step();
    let id = h.app.session.active().unwrap();
    let dev = |h: &Headless| h.app.session.develop_of(id).unwrap_or_default();
    h.request("ui.clickWidget", json!({"id": "sliderLabel:light.shadows"}), T);
    h.step();
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.shadows"));
    let s0 = dev(&h).light.shadows;
    h.request("ui.key", json!({"key": "up"}), T);
    h.step();
    assert!(dev(&h).light.shadows > s0, "↑ raises the picked slider");
    assert_eq!(h.app.session.active(), Some(id), "the arrows don't step photos while a slider is picked");
    // the wheel with the pointer over the PHOTO (photo-stepping mode on): it turns the picked
    // slider, never steps photos
    exec(&mut h, "view.wheelZoom", json!({"mode": "photos"}));
    let img = h.app.image_rect.unwrap();
    h.request("ui.move", json!({"x": img.center().x, "y": img.center().y}), T);
    let s1 = dev(&h).light.shadows;
    h.request("ui.scroll", json!({"dy": 2.0, "unit": "line"}), T);
    h.step();
    assert!(dev(&h).light.shadows > s1, "the wheel raises it: {} → {}", s1, dev(&h).light.shadows);
    assert_eq!(h.app.session.active(), Some(id), "the wheel didn't step photos while a slider is picked");
    // → : the next slider (Whites follows Shadows)
    h.request("ui.key", json!({"key": "right"}), T);
    h.step();
    h.step();
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.whites"));
    // a click on the photo lets go
    let img = h.app.image_rect.unwrap();
    h.request("ui.click", json!({"x": img.center().x, "y": img.center().y}), T);
    h.step();
    h.step();
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx), None);
}

/// Sync Settings copies from the FIRST photo picked, even after ⌘-clicks moved the active photo;
/// while its box is open the keys are its own (a rating key does nothing behind it) and Return
/// presses OK (Bryan, 2026-11-23).
#[test]
fn sync_copies_from_the_first_pick_and_return_presses_ok() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(3).map(|p| p.0).collect();
    let pid = |i: usize| lightcraft_catalog::PhotoId(ids[i]);
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(&mut h, "develop.set", json!({"control": "light.exposure", "value": 1.25}));
    exec(&mut h, "library.select", json!({"ids": [ids[1]], "mode": "toggle"}));
    exec(&mut h, "library.select", json!({"ids": [ids[2]], "mode": "toggle"}));
    assert_eq!(h.app.session.active(), Some(pid(2)), "the last ⌘-click is shown");
    assert_eq!(h.app.session.sync_source(), Some(pid(0)), "the first pick is the reference");
    exec(&mut h, "dialog.syncSettings", json!({}));
    h.step();
    let rating = h.app.session.catalog.photo(pid(2)).unwrap().rating;
    h.request("ui.key", json!({"key": "4"}), T);
    h.step();
    assert_eq!(h.app.session.catalog.photo(pid(2)).unwrap().rating, rating, "keys don't reach the grid behind the box");
    h.request("ui.key", json!({"key": "enter"}), T);
    h.step();
    assert!(h.app.ui.dialog.is_none(), "Return pressed OK");
    for i in [1, 2] {
        assert_eq!(h.app.session.develop_of(pid(i)).unwrap().light.exposure, 1.25, "photo {i} synced from the first pick");
    }
}

/// View ▸ Scroll Wheel ▸ photos: a mouse-wheel notch over the photo steps a photo (down = next),
/// ⌘-scroll still zooms; Photo ▸ Edit In picks the ⌘E editor (Bryan, 2026-11-23).
#[test]
fn wheel_steps_photos_and_edit_in_picks_the_editor() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail"}), T);
    exec(&mut h, "view.wheelZoom", json!({"mode": "photos"}));
    h.step();
    let first = h.app.session.active().unwrap();
    let img = h.app.image_rect.unwrap();
    h.request("ui.move", json!({"x": img.center().x, "y": img.center().y}), T);
    h.request("ui.scroll", json!({"dy": -1.0, "unit": "line"}), T);
    h.step();
    let second = h.app.session.active().unwrap();
    assert_ne!(second, first, "a notch down = the next photo");
    h.request("ui.scroll", json!({"dy": 1.0, "unit": "line"}), T);
    h.step();
    assert_eq!(h.app.session.active(), Some(first), "a notch up = back");
    assert!(matches!(h.app.ui.zoom, crate::state::Zoom::Fit), "stepping never zoomed");
    // Edit In: the editor list and the choice
    exec(&mut h, "app.externalEditor", json!({"app": "PhotoCraft"}));
    assert_eq!(h.app.ui.settings.external_editor, "PhotoCraft");
    let photo = crate::menubar::menu_bar(&h.app).into_iter().find(|(t, _)| t == "Photo").unwrap().1;
    let text = format!("{photo:?}");
    assert!(text.contains("Edit in PhotoCraft"), "the menu names the editor: {text}");
    exec(&mut h, "app.externalEditor", json!({"app": ""}));
    assert!(h.app.ui.settings.external_editor.is_empty(), "system default");
}

/// A console slider moved through 20 positions is one drag: draft renders while it moves, then one
/// history step once it has been still ([`crate::keymap::CONSOLE_SETTLE`]) — not 20 committed edits
/// (Bryan, 2026-11-23: the Monogram lagged behind Lightroom).
#[test]
fn a_console_slider_move_is_one_drag_and_one_history_step() {
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail"}), T);
    let id = h.app.session.active().unwrap();
    let steps = |h: &Headless| h.app.session.catalog.photo(id).unwrap().history.len();
    let before = steps(&h);
    for v in (0..=255).step_by(13) {
        exec(&mut h, "keys.lrinput", json!({"line": format!("Shadows = {v},-100,100")}));
    }
    assert!(h.app.session.interaction.is_some(), "moving: an interaction, like a mouse drag");
    assert_eq!(steps(&h), before, "no history step per tick");
    let last = h.app.session.develop_of(id).unwrap().light.shadows;
    assert!(last > 90.0, "the value follows the slider while it moves: {last}");
    assert!(h.step_until(Duration::from_secs(3), |h| h.app.session.interaction.is_none()), "let go → committed");
    assert_eq!(steps(&h), before + 1, "one history step for the whole move");
    assert_eq!(h.app.session.develop_of(id).unwrap().light.shadows, last);
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
    assert_eq!(r["label"], "21x9", "{r}");
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = 250,0,1")["unchanged"], true);
    assert!(dev(&h).crop.aspect.is_some());
    // cycle by dial: next wraps to the first (As Shot), previous back to 21 × 9
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = ^1,0,1")["label"], "As Shot");
    assert_eq!(line(&mut h, "lc_cycle_cropAspect = v1,0,1")["label"], "21x9");
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
    assert_eq!(r["label"], "21x9", "top of the slider: {r}");
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
    assert!(!match_exposure && !auto_align && !match_color, "match exposure / auto align / match color start off");
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
    // ⌘V onto a photo with a mask of its own: the paste box asks what goes over, masks merged or replaced
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    key(&mut h, "V", json!({"cmd": true}));
    h.step();
    assert!(matches!(&h.app.ui.dialog, Some(crate::state::Dialog::PasteSettings { groups, .. }) if groups.iter().any(|g| g == "masks")), "{:?}", h.app.ui.dialog);
    for _ in 0..8 {
        h.step();
    }
    click(&mut h, "pasteGroup:replaceMasks");
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

fn crop_of(h: &Headless) -> lightcraft_geom::CropGeometry {
    let id = h.app.session.active().expect("active photo");
    h.app.session.develop_of(id).unwrap_or_default().crop.geometry
}

/// CropPositionX moves the crop sideways only, CropPositionY up/down only, as the photo is shown —
/// also on a straightened photo: in the loupe (the cropped photo) along the crop's axes, where the
/// slide-back-inside used to pull it towards the image centre on both axes; in the Crop tool (the
/// whole photo, the crop a tilted frame) along the screen's (Bryan, 2026-11-23: "the crop
/// adjustment horizontal is being buggy and going both vertical and horizontal").
#[test]
fn crop_dials_move_one_axis_only() {
    // the crop's centre on screen: in the cropped photo's frame, or turned back onto the photo
    fn centre(c: &lightcraft_geom::CropGeometry, tool: bool, w: f64, h: f64) -> (f64, f64) {
        let m = c.rect.center();
        if !tool {
            return (m.x, m.y);
        }
        let a = -c.angle.to_radians();
        let (px, py) = ((m.x - 0.5) * w, (m.y - 0.5) * h);
        ((px * a.cos() - py * a.sin()) / w, (px * a.sin() + py * a.cos()) / h)
    }
    for tool in [false, true] {
        for angle in [0.0, 4.0, -7.5] {
            let mut h = demo();
            h.request("ui.set", json!({"view": "detail"}), T);
            exec(&mut h, "panel.crop", json!({}));
            exec(&mut h, "crop.set", json!({"angle": angle}));
            exec(&mut h, "crop.set", json!({"rect": [0.3, 0.25, 0.6, 0.55]}));
            if !tool {
                h.request("ui.set", json!({"right": "edit"}), T);
            }
            h.step();
            let id = h.app.session.active().unwrap();
            let p = h.app.session.catalog.photo(id).unwrap();
            let (w, hh) = if p.develop.orientation.swaps_axes() { (p.height as f64, p.width as f64) } else { (p.width as f64, p.height as f64) };
            let start = crop_of(&h);
            let (sx, sy) = centre(&start, tool, w, hh);
            let (w0, h0) = (start.rect.width(), start.rect.height());
            // far enough right to hit the (rotated) edge
            for _ in 0..120 {
                exec(&mut h, "keys.lrinput", json!({"line": "CropPositionX = ^0.5,-100,100"}));
                let c = crop_of(&h);
                let (_, y) = centre(&c, tool, w, hh);
                assert!((y - sy).abs() < 1e-9, "x dial moved y at {angle}° (tool {tool}): {c:?}");
                assert!((c.rect.width() - w0).abs() < 1e-9 && (c.rect.height() - h0).abs() < 1e-9, "x dial resized the crop at {angle}°: {c:?}");
                assert!(c.is_within_image(w, hh));
            }
            let right = crop_of(&h);
            assert!(centre(&right, tool, w, hh).0 > sx + 0.05, "it moved right at {angle}° (tool {tool}): {right:?}");
            // (against a slanted edge it may not go up at all: start again from the middle)
            exec(&mut h, "crop.set", json!({"rect": [0.3, 0.25, 0.6, 0.55]}));
            let mid = crop_of(&h);
            let (mx, my) = centre(&mid, tool, w, hh);
            for _ in 0..120 {
                exec(&mut h, "keys.lrinput", json!({"line": "CropPositionY = v0.5,-100,100"}));
                let c = crop_of(&h);
                assert!((centre(&c, tool, w, hh).0 - mx).abs() < 1e-9, "y dial moved x at {angle}° (tool {tool}): {c:?}");
            }
            assert!(centre(&crop_of(&h), tool, w, hh).1 < my - 0.05, "it moved up at {angle}° (tool {tool})");
            assert!((crop_of(&h).angle - angle).abs() < 1e-9, "the angle stays");
        }
    }
}

/// CropScale zooms about the crop's centre and keeps its shape.
#[test]
fn crop_scale_keeps_centre_and_shape() {
    for angle in [0.0, 5.0] {
        let mut h = demo();
        h.request("ui.set", json!({"view": "detail"}), T);
        exec(&mut h, "panel.crop", json!({}));
        exec(&mut h, "crop.set", json!({"angle": angle}));
        exec(&mut h, "crop.set", json!({"rect": [0.35, 0.3, 0.65, 0.6]}));
        let start = crop_of(&h);
        let aspect = start.rect.width() / start.rect.height();
        for _ in 0..6 {
            exec(&mut h, "keys.lrinput", json!({"line": "CropScale = ^0.5,-100,100"}));
        }
        let c = crop_of(&h);
        assert!(c.rect.width() < start.rect.width(), "^ zooms in");
        assert!((c.rect.center().x - start.rect.center().x).abs() < 1e-9 && (c.rect.center().y - start.rect.center().y).abs() < 1e-9, "{c:?}");
        // zooming out a long way: the shape stays and it stays on the photo
        for _ in 0..200 {
            exec(&mut h, "keys.lrinput", json!({"line": "CropScale = v0.5,-100,100"}));
            let c = crop_of(&h);
            assert!((c.rect.width() / c.rect.height() - aspect).abs() < 1e-6, "shape kept at {angle}°: {c:?}");
        }
    }
}

/// Lens Corrections ▸ Profile: with Enable Profile Corrections on, the Make / Model / Profile
/// pickers and auto. / none. are there for a lens the file doesn't name (Bryan's manual Leica M
/// lenses), and the checkbox runs `lens.profileCorrections` (which picks the Set Lens profile).
#[test]
fn lens_corrections_show_the_profile_pickers() {
    let mut h = crate::tests_library2::headless(lightcraft_engine::Session::with_demo());
    let r = h.request("ui.set", json!({"view": "detail", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    // only this panel open, so a screenshot shows it
    h.app.ui.open_sections = vec!["lensCorrections".into()];
    h.step();
    h.step();
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    assert!(!has(&h, "button:lensProfileAuto"), "off: no pickers");
    exec(&mut h, "photo.setLens", json!({"lens": "Leica Summicron-M 1:2/50", "focal": 50}));
    let r = exec(&mut h, "lens.profileCorrections", json!({"on": true}));
    assert_eq!(r["on"], true);
    h.step();
    h.step();
    assert!(has(&h, "button:lensProfileAuto") && has(&h, "button:lensProfileNone"), "the pickers");
    let r = h.request("ui.clickWidget", json!({"id": "button:lensProfileNone"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    let id = h.app.session.active().unwrap();
    assert!(h.app.session.develop_of(id).unwrap().optics.lens_profile_ref.is_none());
    let r = h.request("ui.clickWidget", json!({"id": "button:lensProfileAuto"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.step();
    assert!(h.app.session.develop_of(id).unwrap().optics.lens_profile, "still on");
    crate::tests_library2::shot(&mut h, "lens-profile-pickers.png");
}

/// How far the loupe trails a Monogram dial: 30 Exposure ticks a second for 3 s on a real raw
/// (ignored: `LC_BENCH_RAW=/path/to/24mp.dng cargo test --release -p lightcraft-ui-egui --lib --
/// --ignored console_dial_lag --nocapture`). Prints the draft render time, how many ticks (and
/// 60 Hz frames) the picture is behind while turning, and how long the sharp render takes after.
#[test]
#[ignore]
fn console_dial_lag() {
    use crate::render::Slot;
    use lightcraft_catalog::PhotoId;
    use std::time::Instant;
    let Some(path) = std::env::var_os("LC_BENCH_RAW") else { return eprintln!("skipped: set LC_BENCH_RAW") };
    let mut s = lightcraft_engine::Session::new().with_fs();
    let r = s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().expect("imported");
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let app = LightcraftApp::new(s, Services { png: None, ..Default::default() });
    // a 14" MacBook Pro window (LC_BENCH_SIZE=1800x1100 for a ~2560 px loupe)
    let size = std::env::var("LC_BENCH_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(a, b)| Some([a.parse().ok()?, b.parse().ok()?])))
        .unwrap_or([1512.0, 900.0]);
    let mut h = Headless::new(app, size, 2.0);
    if std::env::var_os("LC_BENCH_LEGACY").is_some() {
        h.app.renderer.legacy_mode();
        eprintln!("(the old way: parallel drafts, no warm-up, drafts from their own source)");
    }
    let r = h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let sharp = |h: &Headless| h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.quick.is_none()) && !h.app.renderer.is_pending(Slot::Main);
    let t0 = Instant::now();
    assert!(h.step_until(Duration::from_secs(180), sharp), "first render");
    h.settle(Duration::from_secs(60));
    let full = h.app.renderer.textures.get(&Slot::Main).map(|t| (t.size, t.ms)).unwrap();
    eprintln!("open: {:.0} ms; loupe render {}×{} in {:.0} ms", t0.elapsed().as_secs_f64() * 1e3, full.0[0], full.0[1], full.1);
    // LC_BENCH_EDIT=1: an edited photo (noise reduction, sharpening, clarity, texture, dehaze,
    // shadows / highlights) rather than the import state; LC_BENCH_CROP=1: cropped to the middle
    if std::env::var_os("LC_BENCH_EDIT").is_some() {
        let v = json!({"light.exposure": 0.3, "light.highlights": -40, "light.shadows": 30, "effects.clarity": 15, "effects.texture": 10,
            "effects.dehaze": 10, "detail.nrLuminance": 30, "detail.nrColor": 25, "detail.sharpenAmount": 40});
        h.app.run("develop.set", json!({"values": v})).unwrap();
    }
    if std::env::var_os("LC_BENCH_CROP").is_some() {
        h.app.run("crop.set", json!({"rect": [0.2, 0.2, 0.8, 0.8]})).unwrap();
    }
    h.settle(Duration::from_secs(60));
    assert!(h.step_until(Duration::from_secs(180), sharp), "edited render");
    // LC_BENCH_GAP=<ms>: wait that long after the photo is sharp before turning (default 0: the
    // turn starts while the draft size is still being warmed)
    let pause = std::env::var("LC_BENCH_GAP").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(pause) {
        h.step();
        std::thread::sleep(Duration::from_millis(8));
    }
    let line = std::env::var("LC_BENCH_LINE").unwrap_or_else(|_| "Exposure = ^0.02,-5,5".into());
    let ticks_n = std::env::var("LC_BENCH_TICKS").ok().and_then(|v| v.parse().ok()).unwrap_or(90);
    let gap = Duration::from_millis(33);
    let start = Instant::now();
    let mut ticks = 0usize;
    // (job key the loupe asked for, the tick it shows, when)
    let mut wanted: Vec<(u64, usize)> = Vec::new();
    let mut shown_key = h.app.renderer.textures.get(&Slot::Main).map(|t| t.key);
    let mut drafts: Vec<(f64, [usize; 2])> = Vec::new();
    let mut lags: Vec<usize> = Vec::new();
    // per tick: when it was sent, and when a render of it (or later) was first on screen
    let mut sent: Vec<Instant> = Vec::new();
    let mut seen: Vec<Option<Instant>> = Vec::new();
    let mut frame_ms: Vec<f64> = Vec::new();
    let mut last = 0usize;
    let mut unchanged: Vec<bool> = Vec::new();
    let trace = std::env::var_os("LC_BENCH_TRACE").is_some();
    loop {
        let now = Instant::now();
        if ticks < ticks_n && now >= start + gap * ticks as u32 {
            let before = h.app.session.develop_of(PhotoId(id));
            crate::keymap::lr_input(&mut h.app, &line).unwrap();
            // a detent past the end of the slider changes nothing: on screen with the one before
            unchanged.push(h.app.session.develop_of(PhotoId(id)) == before);
            ticks += 1;
            sent.push(now);
            seen.push(None);
        }
        let f0 = Instant::now();
        h.step();
        frame_ms.push(f0.elapsed().as_secs_f64() * 1e3);
        if let Some(k) = h.app.renderer.wanted(Slot::Main)
            && ticks > 0
            && wanted.last().map(|w| w.0) != Some(k)
        {
            wanted.push((k, ticks));
        }
        let tex = h.app.renderer.textures.get(&Slot::Main).map(|t| (t.key, t.ms, t.size));
        if let Some((k, ms, size)) = tex
            && Some(k) != shown_key
        {
            shown_key = Some(k);
            if ticks < ticks_n || h.app.session.interaction.is_some() {
                drafts.push((ms, size));
            }
            if let Some(&(_, t)) = wanted.iter().find(|w| w.0 == k) {
                for s in seen.iter_mut().take(t) {
                    s.get_or_insert(Instant::now());
                }
                last = t;
            }
        }
        // a crop dial's uncropped draft on screen: this frame drew every detent so far
        if ticks > 0 && h.app.reframe_key.is_some() && h.app.renderer.textures.get(&Slot::Main).map(|t| t.key) == h.app.reframe_key {
            for s in seen.iter_mut().take(ticks) {
                s.get_or_insert(Instant::now());
            }
            last = ticks;
        }
        if ticks > 0 && ticks <= ticks_n && h.app.session.interaction.is_some() {
            lags.push(ticks - last.min(ticks));
        }
        // LC_BENCH_TRACE=1: what the renderer is doing, frame by frame, for the first second
        if trace && start.elapsed() < Duration::from_millis(400) {
            eprintln!(
                "trace {:>5.0} ms tick {ticks:>2} shows {last:>2} frame {:>5.1} ms pending {:?} queued {}",
                start.elapsed().as_secs_f64() * 1e3,
                frame_ms.last().copied().unwrap_or(0.0),
                h.app.renderer.pending_slots(),
                h.app.renderer.queued()
            );
        }
        let full_size = h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.size[0] >= full.0[0]);
        if ticks == ticks_n && h.app.session.interaction.is_none() && sharp(&h) && full_size {
            break;
        }
        if start.elapsed() > Duration::from_secs(60) {
            eprintln!("gave up waiting");
            break;
        }
        // a 120 Hz display's frame pacing
        std::thread::sleep(Duration::from_millis(8));
    }
    let after = sent.last().map(|t| t.elapsed().as_secs_f64() * 1e3).unwrap_or(0.0);
    let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    for i in 1..seen.len() {
        if unchanged[i] && seen[i].is_none_or(|t| seen[i - 1].is_some_and(|p| p < t)) {
            seen[i] = seen[i - 1].map(|p| p.max(sent[i]));
        }
    }
    if unchanged.iter().any(|u| *u) {
        eprintln!("({} detents past the slider's end: counted as shown with the one before)", unchanged.iter().filter(|u| **u).count());
    }
    let latency: Vec<f64> = sent.iter().zip(&seen).filter_map(|(a, b)| b.map(|b| (b - *a).as_secs_f64() * 1e3)).collect();
    let mut sorted = latency.clone();
    sorted.sort_by(f64::total_cmp);
    let p = |q: f64| sorted.get(((sorted.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(f64::NAN);
    let dm: Vec<f64> = drafts.iter().map(|d| d.0).collect();
    if trace {
        let per: Vec<String> =
            sent.iter().zip(&seen).map(|(a, b)| b.map_or("-".into(), |b| format!("{:.0}", (b - *a).as_secs_f64() * 1e3))).collect();
        eprintln!("tick → on screen, in order (ms): {}", per.join(" "));
    }
    eprintln!(
        "{ticks} ticks at 30/s: {} drafts shown ({:.0} ms each on average, max {:.0}, first ones {:?}, size {:?}); ticks behind while turning: mean {:.1}, max {}; tick → on screen: median {:.0} ms, p90 {:.0} ms (= {:.1} / {:.1} frames at 60 Hz); UI frame: mean {:.1} ms, max {:.1} ms; sharp {:.0} ms after the last tick",
        drafts.len(),
        avg(&dm),
        dm.iter().copied().fold(0.0, f64::max),
        dm.iter().take(3).map(|m| m.round()).collect::<Vec<_>>(),
        drafts.first().map(|d| d.1),
        lags.iter().sum::<usize>() as f64 / lags.len().max(1) as f64,
        lags.iter().max().copied().unwrap_or(0),
        p(0.5),
        p(0.9),
        p(0.5) / 16.7,
        p(0.9) / 16.7,
        avg(&frame_ms),
        frame_ms.iter().copied().fold(0.0, f64::max),
        after,
    );
}

/// What committed edits cost the UI thread on a library the size of Bryan's (ignored:
/// `LC_BENCH_RAW=/path/to.dng [LC_BENCH_PHOTOS=16000] [LC_BENCH_PRESETS=/path/to/presets.json]
/// [LIGHTCRAFT_PROFILE=1] cargo test --release -p lightcraft-ui-egui --lib -- --ignored commit_hitch
/// --nocapture`). An on-disk library with XMP auto-write on; console dial turns (each ends in a
/// commit once the dial is still) and slider-click edits (one committed command each). Prints the
/// frame times around each commit and checks that every edit reaches the photo's sidecar.
#[test]
#[ignore]
fn commit_hitch() {
    use crate::render::Slot;
    use lightcraft_catalog::{Op, Photo, PhotoId, Source};
    use std::time::Instant;
    let Some(raw) = std::env::var_os("LC_BENCH_RAW") else { return eprintln!("skipped: set LC_BENCH_RAW") };
    let dir = std::env::temp_dir().join(format!("lc-commit-hitch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("drive")).unwrap();
    let _t = crate::tests_library2::TempDir(dir.clone());
    let photo = dir.join("drive/L1005121.DNG");
    std::fs::copy(&raw, &photo).unwrap();
    if let Some(p) = std::env::var_os("LC_BENCH_PRESETS") {
        std::fs::copy(p, dir.join("lib/presets.json")).unwrap();
    }
    let mut s = lightcraft_engine::Session::new().with_fs();
    s.open_library(dir.join("lib"), false).unwrap();
    let r = s.execute("library.import", &json!({"paths": [photo.to_string_lossy()]})).unwrap();
    let id = r["imported"][0].as_u64().expect("imported");
    let n: u64 = std::env::var("LC_BENCH_PHOTOS").ok().and_then(|v| v.parse().ok()).unwrap_or(16_000);
    let drive = dir.join("drive").to_string_lossy().to_string();
    let ops: Vec<Op> = (0..n)
        .map(|i| {
            let pid = s.catalog.alloc_photo_id();
            let src = Source::File { path: format!("{drive}/F{:03}/L{i:07}.DNG", i / 500) };
            Op::AddPhoto { photo: Box::new(Photo::new(pid, src, &format!("L{i:07}.DNG"), "DNG", 6000, 4000, "2026-01-01T00:00:00")) }
        })
        .collect();
    s.commit("bench photos", Op::Batch { ops }).unwrap();
    s.persist().unwrap();
    s.execute("library.xmpPreferences", &json!({"autoWrite": true})).unwrap();
    s.execute("library.select", &json!({"ids": [id]})).unwrap();
    let app = LightcraftApp::new(s, Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, [1800.0, 1100.0], 2.0);
    let r = h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    assert_eq!(r["ok"], true, "{r}");
    let sharp = |h: &Headless| h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.quick.is_none()) && !h.app.renderer.is_pending(Slot::Main);
    assert!(h.step_until(Duration::from_secs(180), sharp), "first render");
    h.settle(Duration::from_secs(60));
    let sidecar = photo.with_extension("xmp");
    let step = |h: &mut Headless, frames: &mut Vec<f64>| {
        let f0 = Instant::now();
        h.step();
        let ms = f0.elapsed().as_secs_f64() * 1e3;
        if ms > 15.0 {
            let pf = h.app.perf;
            eprintln!(
                "slow frame {ms:.1} ms: logic {:.1}, layout {:.1}; sidecars queued {}",
                pf.logic_ms,
                pf.frame_ms,
                h.app.session.sidecars_pending()
            );
        }
        frames.push(ms);
        std::thread::sleep(Duration::from_millis(8));
    };
    let pct = |v: &[f64], q: f64| {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        s.get(((s.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(f64::NAN)
    };
    // console dial turns: 6 detents at 30/s, then still until the move is committed
    let mut commit_frames = Vec::new();
    let mut all = Vec::new();
    for turn in 0..8 {
        let line = if turn % 2 == 0 { "Shadows = ^1,-100,100" } else { "Shadows = v1,-100,100" };
        let mut frames = Vec::new();
        for _ in 0..6 {
            crate::keymap::lr_input(&mut h.app, line).unwrap();
            let t = Instant::now();
            while t.elapsed() < Duration::from_millis(33) {
                step(&mut h, &mut frames);
            }
        }
        let t = Instant::now();
        while h.app.session.interaction.is_some() && t.elapsed() < Duration::from_secs(5) {
            step(&mut h, &mut frames);
        }
        // the commit frame and the second after it
        let c0 = frames.len();
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(1000) {
            step(&mut h, &mut frames);
        }
        let after = frames.get(c0.saturating_sub(1)..).unwrap_or_default();
        commit_frames.push(after.iter().copied().fold(0.0, f64::max));
        all.extend(frames);
    }
    eprintln!(
        "dial turns: worst frame in the second after each commit {:?} ms; all frames median {:.1}, p99 {:.1}, max {:.1}",
        commit_frames.iter().map(|m| m.round()).collect::<Vec<_>>(),
        pct(&all, 0.5),
        pct(&all, 0.99),
        all.iter().copied().fold(0.0, f64::max)
    );
    // slider clicks: one committed command each
    let mut cmd_ms = Vec::new();
    let mut frames = Vec::new();
    for k in 0..8 {
        let t = Instant::now();
        h.app.run("develop.set", json!({"control": "light.exposure", "value": 0.1 * f64::from(k + 1)})).unwrap();
        cmd_ms.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(500) {
            step(&mut h, &mut frames);
        }
    }
    eprintln!(
        "slider clicks: command {:?} ms; frames after: median {:.1}, p99 {:.1}, max {:.1}",
        cmd_ms.iter().map(|m| (m * 10.0).round() / 10.0).collect::<Vec<_>>(),
        pct(&frames, 0.5),
        pct(&frames, 0.99),
        frames.iter().copied().fold(0.0, f64::max)
    );
    // every edit reaches the sidecar (after the debounce / once the app quits)
    let t = Instant::now();
    let exposure = |p: &std::path::Path| match std::fs::read_to_string(p)
        .ok()
        .and_then(|x| lightcraft_engine::sidecar::parse_sidecar(&x, true).ok())
        .and_then(|x| x.develop)
    {
        Some(lightcraft_engine::sidecar::DevelopPatch::Full(d)) => Some(d.light.exposure),
        _ => None,
    };
    while t.elapsed() < Duration::from_secs(10) && (exposure(&sidecar).is_none_or(|e| (e - 0.8).abs() > 1e-6) || h.app.session.sidecars_pending() > 0)
    {
        step(&mut h, &mut Vec::new());
    }
    assert_eq!(exposure(&sidecar), Some(0.8), "the last edit reached the sidecar");
    let id = PhotoId(id);
    assert_eq!(h.app.session.metadata_status(id), lightcraft_engine::sidecar::MetadataStatus::UpToDate);
}

/// ⌘C copies everything about the photo; ⌘V on another opens Paste Selected Settings (the
/// sync-style checklist) to pick what goes over; the ticks are remembered (Bryan, 11-24).
#[test]
fn cmd_c_copies_everything_and_cmd_v_asks_what_to_paste() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.catalog.photos().take(2).map(|p| p.id.0).collect();
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 1.0, "detail.sharpenAmount": 90}}));
    h.step();
    key(&mut h, "C", json!({"cmd": true}));
    h.step();
    let copied = h.app.session.clipboard_groups.len();
    assert!(copied > 20, "every setting copied, not the last Copy dialog choice ({copied})");
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    h.step();
    key(&mut h, "V", json!({"cmd": true}));
    for _ in 0..8 {
        h.step();
    }
    let Some(crate::state::Dialog::PasteSettings { groups, .. }) = &h.app.ui.dialog else { panic!("⌘V: {:?}", h.app.ui.dialog) };
    assert_eq!(groups.len(), copied, "all ticked the first time");
    for w in ["pasteGroup:none", "pasteGroup:exposure"] {
        let r = h.request("ui.clickWidget", json!({"id": w}), T);
        assert_eq!(r["ok"], true, "{w}: {r}");
        h.step();
    }
    let r = h.request("ui.dialog.confirm", json!({}), T);
    assert_eq!(r["ok"], true, "{r}");
    let d = h.app.session.develop_of(lightcraft_catalog::PhotoId(ids[1])).unwrap_or_default();
    assert_eq!((d.light.exposure, d.detail.sharpen_amount == 90.0), (1.0, false), "only exposure went over");
    // next ⌘V starts from that pick
    key(&mut h, "V", json!({"cmd": true}));
    for _ in 0..4 {
        h.step();
    }
    let Some(crate::state::Dialog::PasteSettings { groups, .. }) = &h.app.ui.dialog else { panic!("second ⌘V") };
    assert_eq!(groups, &["exposure".to_string()]);
}

/// The menu bar (which gets ⌘-keys before the window on macOS) gives each of Bryan's keys to
/// one command: ⌘⇧E = Export for the picked photos, ⌘E = the external editor, ⌘C / ⌘V = copy
/// settings / the paste box.
#[test]
fn cmd_shift_e_exports_the_picked_photos() {
    let mut h = demo();
    let menus = crate::menus::menu_entries(&h.app);
    let owners = |sc: &str| -> Vec<String> { menus.iter().filter(|e| e.shortcut.as_deref().map(crate::keymap::canonical) == Some(crate::keymap::canonical(sc))).map(|e| e.id.clone()).collect() };
    assert_eq!(owners("Cmd+Shift+E"), ["dialog.export"]);
    assert_eq!(owners("Cmd+E"), ["photo.editInExternal"]);
    assert_eq!(owners("Cmd+V"), ["dialog.pasteSettings"]);
    assert_eq!(owners("Cmd+C"), ["develop.copy"]);
    assert_eq!(owners("Cmd+Shift+M"), ["dialog.merge"]);
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(2).map(|p| p.0).collect();
    exec(&mut h, "library.select", json!({"ids": ids}));
    h.step();
    key(&mut h, "E", json!({"cmd": true, "shift": true}));
    for _ in 0..4 {
        h.step();
    }
    assert!(matches!(h.app.ui.dialog, Some(crate::state::Dialog::Export { .. })), "{:?}", h.app.ui.dialog.as_ref().map(|_| "other dialog"));
    assert_eq!(crate::export_dialog::title(&h.app), "Export 2 Files");
}

/// ⌘⇧M: Merge… for the picked photos; with no kind chosen before, guessed from them — brackets (exposure a stop or
/// more apart) → hdr., one exposure → panorama. (Bryan 11-24)
#[test]
fn cmd_shift_m_merges_the_picked_photos_with_a_guessed_kind() {
    let mut s = lightcraft_engine::Session::with_demo();
    let first = s.visible_cloned()[0];
    let base = lightcraft_catalog::Photo::clone(s.catalog.photo(first).unwrap());
    let mut add = |id: u64, shutter: &str| {
        let mut p = base.clone();
        p.id = lightcraft_catalog::PhotoId(id);
        p.meta.shutter = shutter.into();
        p.meta.exposure_bias = None;
        s.catalog.apply(lightcraft_catalog::Op::AddPhoto { photo: Box::new(p) }).unwrap();
    };
    add(90_001, "1/500");
    add(90_002, "1/30");
    add(90_003, "1/250");
    add(90_004, "1/250");
    let app = crate::LightcraftApp::new(s, crate::Services::default());
    let mut h = Headless::new(app, [1400.0, 900.0], 1.0);
    h.step();
    let kind = |h: &mut Headless, ids: [u64; 2]| {
        exec(h, "library.select", json!({"ids": ids}));
        h.app.ui.dialog = None;
        h.app.merge.last_kind = None; // nothing chosen before: the guess
        h.step();
        key(h, "M", json!({"cmd": true, "shift": true}));
        h.step();
        match &h.app.ui.dialog {
            Some(crate::state::Dialog::Merge { opts }) => opts.command.clone(),
            _ => panic!("⌘⇧M opened no Merge box"),
        }
    };
    assert_eq!(kind(&mut h, [90_001, 90_002]), "merge.hdr", "four stops apart");
    assert_eq!(kind(&mut h, [90_003, 90_004]), "merge.panorama", "one exposure");
}

// ------------------------------------------------------------------ crop-zoom dial: smooth turns

/// What a Monogram crop-zoom turn did to the loupe (see [`zoom_turn`]).
#[derive(Default)]
struct ZoomTurn {
    /// The keys the Main slot asked for or showed during the turn (the pre-turn sharp render's left out).
    keys: std::collections::BTreeSet<u64>,
    /// Frames (after the first draft was up) that did not show a cut of the current crop.
    bad: Vec<String>,
    /// The interaction ended before the last detent (a pause was taken for "let go").
    ended_early: bool,
    /// The crop's centre after each detent.
    centres: Vec<(f64, f64)>,
    /// Frames whose cut was stretched / squashed, or whose picture jumped or resized.
    stretched: Vec<String>,
    /// Frames that drew a cut.
    drawn: usize,
    /// The on-screen rect of the first cut.
    first_rect: Option<egui::Rect>,
}

/// Is Main showing the crop dial's uncropped draft (drawn cut to the current crop)?
fn showing_cut(h: &Headless) -> bool {
    use crate::render::Slot;
    h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.photo == h.app.session.active().unwrap_or(t.photo) && h.app.reframe_shows(t.key))
}

/// A Monogram dial turn on `line` (`CropScale = ^0.5,-100,100` …): `detents` of them, a frame each
/// (16 ms apart), `pause` = (every n detents, ms) a still spell with frames running. The first
/// draft is waited for (a turn's one unavoidable lag), then every frame must show the cut.
fn zoom_turn(h: &mut Headless, line: &str, detents: usize, pause: Option<(usize, u64)>, crop_tool: bool) -> ZoomTurn {
    use crate::render::Slot;
    use std::collections::BTreeSet;
    let id = h.app.session.active().expect("active photo");
    let sharp_key = h.app.renderer.textures.get(&Slot::Main).map(|t| t.key);
    let mut t = ZoomTurn { keys: BTreeSet::new(), ..Default::default() };
    let mut armed = false;
    let look = |h: &mut Headless, t: &mut ZoomTurn, armed: &mut bool, what: &str| {
        h.step();
        if let Some(k) = h.app.renderer.wanted(Slot::Main).filter(|k| Some(*k) != sharp_key) {
            t.keys.insert(k);
        }
        if h.app.session.interaction.is_none() {
            return;
        }
        if let Some((r, uv, tex)) = h.app.reframe_drawn.get() {
            t.drawn += 1;
            let (pw, ph) = h.app.session.active().and_then(|id| h.app.session.catalog.photo(id)).map_or((1, 1), |p| (p.width.max(1) as usize, p.height.max(1) as usize));
            let develop = h.app.session.develop_of(id).unwrap_or_default();
            let (cw, ch) = lightcraft_pipeline::native_output_size(pw, ph, &develop);
            let want = cw / ch;
            let shown = f64::from(r.width() / r.height());
            // the cut's top and left edges in the texture's own pixels
            let px = |a: egui::Pos2, b: egui::Pos2| f64::from(((b.x - a.x) * tex[0] as f32).hypot((b.y - a.y) * tex[1] as f32));
            let sampled = px(uv[0], uv[1]) / px(uv[0], uv[3]);
            if (shown / want - 1.0).abs() > 0.005 {
                t.stretched.push(format!("{what}: drawn {shown:.4} vs the crop's {want:.4}"));
            }
            if (sampled / want - 1.0).abs() > 0.005 {
                t.stretched.push(format!("{what}: samples {sampled:.4} vs the crop's {want:.4} (texture {tex:?})"));
            }
            let first = *t.first_rect.get_or_insert(r);
            if (r.size() - first.size()).abs().max_elem() > 1.0 || (r.center() - first.center()).abs().max_elem() > 1.0 {
                t.stretched.push(format!("{what}: the picture moved / resized: {first:?} → {r:?}"));
            }
        }
        let ok = if crop_tool {
            h.app.renderer.textures.get(&Slot::Main).is_some_and(|x| x.photo == id)
        } else {
            showing_cut(h)
        };
        if ok {
            *armed = true;
        } else if *armed {
            t.bad.push(format!("{what}: Main shows {:?}, reframe {:?}", h.app.renderer.textures.get(&Slot::Main).map(|x| x.key), h.app.reframe_key));
        }
    };
    for k in 0..detents {
        exec(h, "keys.lrinput", json!({"line": line}));
        let c = crop_of(h).rect.center();
        t.centres.push((c.x, c.y));
        look(h, &mut t, &mut armed, &format!("detent {k}"));
        if !armed && k == 0 {
            // the first draft of the turn: its one render
            let t0 = std::time::Instant::now();
            while !armed && t0.elapsed() < T {
                std::thread::sleep(Duration::from_millis(2));
                look(h, &mut t, &mut armed, "first draft");
            }
            t.bad.clear();
        }
        std::thread::sleep(Duration::from_millis(16));
        if let Some((n, ms)) = pause
            && (k + 1) % n == 0
            && k + 1 < detents
        {
            let t0 = std::time::Instant::now();
            while t0.elapsed() < Duration::from_millis(ms) {
                look(h, &mut t, &mut armed, &format!("pause after {k}"));
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        if h.app.session.interaction.is_none() {
            t.ended_early = true;
        }
    }
    t
}

fn zoom_setup(angle: f64, rect: [f64; 4], crop_tool: bool) -> Headless {
    use crate::render::Slot;
    let mut h = demo();
    h.request("ui.set", json!({"view": "detail", "module": "develop", "right": "edit"}), T);
    exec(&mut h, "crop.set", json!({"rect": rect, "angle": angle}));
    if crop_tool {
        exec(&mut h, "panel.crop", json!({}));
    }
    let sharp = |h: &Headless| {
        h.app.renderer.textures.get(&Slot::Main).is_some_and(|t| t.quick.is_none() && Some(t.key) != h.app.reframe_key) && !h.app.renderer.is_pending(Slot::Main)
    };
    assert!(h.step_until(T, sharp), "first render: {}", h.app.renderer.diagnose());
    h.settle(T);
    h
}

/// Zooming in on the crop with the Monogram dial draws every detent out of ONE uncropped render:
/// the size and key of that draft used to follow the crop (uncropped ÷ crop), so each detent asked
/// for a new render and, until it came, Main showed the previous uncropped texture squeezed into the
/// crop's frame (Bryan, 2026-11-24: "zooming into the crop on the Monogram glitches out").
#[test]
fn crop_zoom_turn_is_one_uncropped_render() {
    let mut h = zoom_setup(0.0, [0.1, 0.2, 0.9, 0.65], false);
    let t = zoom_turn(&mut h, "CropScale = ^0.5,-100,100", 30, None, false);
    assert!(t.bad.is_empty(), "{} frames that were not the cut, first: {:?}", t.bad.len(), t.bad.first());
    assert!(t.keys.len() <= 2, "{} renders asked for in one turn: {:?}", t.keys.len(), t.keys);
    assert!(!t.ended_early, "one interaction");
    assert!(t.drawn > 20, "cut frames drawn: {}", t.drawn);
    assert!(t.stretched.is_empty(), "{} stretched frames, first: {:?}", t.stretched.len(), t.stretched.first());
    let w = crop_of(&h).rect.width();
    assert!(w < 0.8 * 0.9, "it zoomed in ({w})");
}

/// … also with short pauses in the turn (a hand that stops for 500 ms to look): the interaction
/// lives on ([`crate::keymap::CROP_SETTLE`]), so no sharp-render swap and no second draft.
#[test]
fn crop_zoom_turn_survives_pauses() {
    let mut h = zoom_setup(0.0, [0.1, 0.2, 0.9, 0.65], false);
    let t = zoom_turn(&mut h, "CropScale = ^0.5,-100,100", 30, Some((6, 500)), false);
    assert!(!t.ended_early, "a 500 ms pause is not letting go");
    assert!(t.bad.is_empty(), "{} frames that were not the cut, first: {:?}", t.bad.len(), t.bad.first());
    assert!(t.keys.len() <= 2, "{} renders asked for in one turn: {:?}", t.keys.len(), t.keys);
    assert!(t.stretched.is_empty(), "{} stretched frames, first: {:?}", t.stretched.len(), t.stretched.first());
    // and when the dial really stops, the sharp render of the final crop replaces the cut
    assert!(h.step_until(T, |h| h.app.session.interaction.is_none() && !h.app.renderer.is_pending(crate::render::Slot::Main) && !showing_cut(h)), "{}", h.app.renderer.diagnose());
}

/// Zooming out of a straightened crop to the image's limit: the crop's centre slides towards the
/// photo's centre, never away, in small steps (no jump), and the draft keeps showing the cut.
#[test]
fn crop_zoom_out_to_the_limit_slides_smoothly() {
    let mut h = zoom_setup(4.0, [0.62, 0.55, 0.92, 0.8], false);
    let t = zoom_turn(&mut h, "CropScale = v0.5,-100,100", 260, None, false);
    // (it got to the photo's limit: one more detent changes nothing)
    let at_limit = crop_of(&h);
    exec(&mut h, "keys.lrinput", json!({"line": "CropScale = v0.5,-100,100"}));
    assert_eq!(crop_of(&h), at_limit, "the zoom-out reached the limit");
    assert!(t.bad.is_empty(), "{} frames that were not the cut, first: {:?}", t.bad.len(), t.bad.first());
    assert!(t.stretched.is_empty(), "{} stretched frames, first: {:?}", t.stretched.len(), t.stretched.first());
    let d = |c: (f64, f64)| ((c.0 - 0.5).powi(2) + (c.1 - 0.5).powi(2)).sqrt();
    let mut worst: f64 = 0.0;
    for (i, w) in t.centres.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        assert!(d(b) <= d(a) + 0.002, "detent {}: the centre moved away from the middle: {a:?} → {b:?}", i + 1);
        worst = worst.max(((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt());
    }
    eprintln!("zoom-out: largest centre step {worst:.4}");
    assert!(worst < 0.01, "the centre jumped {worst:.4} in one detent");
}

/// With the Crop panel open the loupe shows the whole photo: a zoom turn needs one draft at most
/// (the crop is not in the picture): the draft of the turn's first detent, where every detent used to ask for a fresh one.
#[test]
fn crop_zoom_turn_with_the_crop_panel_open() {
    let mut h = zoom_setup(0.0, [0.1, 0.1, 0.9, 0.9], true);
    let t = zoom_turn(&mut h, "CropScale = ^0.5,-100,100", 30, None, true);
    assert!(t.bad.is_empty(), "{} frames that were not the cut, first: {:?}", t.bad.len(), t.bad.first());
    assert!(t.keys.len() <= 1, "{} renders asked for in one turn: {:?}", t.keys.len(), t.keys);
}

// Cycle lists by name: a list called `vertical` is not a dial, `v1` is; names with spaces resolve;
// the step message is just the item's label.
fn keymap_cycle_set(h: &mut Headless, list: &str, labels: &[&str]) {
    let items: Vec<Value> = labels.iter().map(|l| json!({"label": l, "command": "crop.aspect", "params": {"aspect": "4x3"}})).collect();
    exec(h, "keys.set", json!({"cycles": {(list): items}}));
}

fn keymap_cycle_step(h: &mut Headless, line: &str) -> Value {
    crate::keymap::lr_input(&mut h.app, line).unwrap()["label"].clone()
}

#[test]
fn keymap_cycle_list_named_vertical_steps_forward() {
    let mut h = demo();
    keymap_cycle_set(&mut h, "vertical", &["A", "B", "C"]);
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = vertical"), "A");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = vertical"), "B");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = vertical"), "C");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle_vertical = "), "A", "the name form wraps forward too");
}

#[test]
fn keymap_cycle_real_dial_still_steps_backwards() {
    let mut h = demo();
    keymap_cycle_set(&mut h, "vertical", &["A", "B", "C"]);
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = vertical"), "A");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle_vertical = ^1,0,1"), "B");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle_vertical = v3,0,1"), "A", "v + digit is a dial: back one");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle_vertical = v1,0,1"), "C", "back wraps to the end");
}

#[test]
fn keymap_cycle_list_name_with_spaces_resolves() {
    let mut h = demo();
    keymap_cycle_set(&mut h, "4x3 as shot crop", &["X", "Y"]);
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = 4x3 as shot crop"), "X");
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle_4x3 as shot crop = ^1,0,1"), "Y");
}

#[test]
fn keymap_cycle_message_is_just_the_item_label() {
    let mut h = demo();
    keymap_cycle_set(&mut h, "ratio", &["4 × 3", "As Shot"]);
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = ratio"), "4 × 3");
    assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.clone()).as_deref(), Some("4 × 3"));
    assert_eq!(keymap_cycle_step(&mut h, "lc_cycle = ratio"), "As Shot");
    assert_eq!(h.app.ui.toast.as_ref().map(|t| t.0.clone()).as_deref(), Some("As Shot"));
}

/// A macro is one button = one photo: its flag/rating steps don't Auto Advance, and the user's
/// Auto Advance setting comes back on afterwards.
#[test]
fn macro_steps_do_not_auto_advance_and_setting_is_restored() {
    let mut h = demo();
    // Auto Advance moves to the next visible photo, so start on one that has a successor
    let vis = h.app.session.visible().to_vec();
    assert!(vis.len() >= 2, "demo shows at least 2 photos");
    let first = vis[0];
    exec(&mut h, "library.select", json!({"ids": [first.0]}));
    exec(&mut h, "photo.flag", json!({"flag": "pick"}));
    h.app.ui.auto_advance = true;
    exec(&mut h, "keys.macro", json!({"steps": [{"command": "photo.unflag"}]}));
    assert_eq!(h.app.session.catalog.photo(first).unwrap().flag, lightcraft_catalog::Flag::None, "the macro's unflag ran");
    assert_eq!(h.app.session.active(), Some(first), "a macro step doesn't move on");
    assert!(h.app.ui.auto_advance, "Auto Advance is restored after the macro");
}
