//! Monogram console inputs that reach every control: `lc_set_<control id>`, the colour-wheel and
//! split inputs, the `local_*` mask sliders, `reset = <tool>` and `lc_cmd` chains.

use std::time::Duration;

use lightcraft_develop::DevelopSettings;
use serde_json::{Value, json};

use crate::headless::Headless;
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

fn dev(h: &Headless) -> DevelopSettings {
    let id = h.app.session.active().expect("active photo");
    (*h.app.session.develop_of(id).unwrap_or_default()).clone()
}

fn line(h: &mut Headless, l: &str) -> Result<Value, String> {
    crate::keymap::lr_input(&mut h.app, l)
}

fn value(h: &Headless, control: &str) -> f64 {
    lightcraft_develop::controls::get(&dev(h), control).unwrap_or(f64::NAN)
}

#[test]
fn lc_set_slider_dial_and_plain_press_on_hdr_max_ev() {
    let mut h = demo();
    assert_eq!(value(&h, "hdr.maxEv"), 4.0);
    // slider: hardware 0–255 across the input's range
    line(&mut h, "lc_set_hdr.maxEv = 255,0,5").unwrap();
    assert_eq!(value(&h, "hdr.maxEv"), 5.0);
    line(&mut h, "lc_set_hdr.maxEv = 51,0,5").unwrap();
    assert!((value(&h, "hdr.maxEv") - 1.0).abs() < 1e-9, "{}", value(&h, "hdr.maxEv"));
    // dial: up and down by Monogram's step
    line(&mut h, "lc_set_hdr.maxEv = ^0.5,0,5").unwrap();
    assert!((value(&h, "hdr.maxEv") - 1.5).abs() < 1e-9);
    line(&mut h, "lc_set_hdr.maxEv = v0.25,0,5").unwrap();
    assert!((value(&h, "hdr.maxEv") - 1.25).abs() < 1e-9);
    // a plain press (no numbers) resets it, and so does `reset`
    line(&mut h, "lc_set_hdr.maxEv =").unwrap();
    assert_eq!(value(&h, "hdr.maxEv"), 4.0);
    line(&mut h, "lc_set_hdr.maxEv = 0,0,5").unwrap();
    assert_eq!(value(&h, "hdr.maxEv"), 0.0);
    line(&mut h, "lc_set_hdr.maxEv = reset").unwrap();
    assert_eq!(value(&h, "hdr.maxEv"), 4.0);
}

#[test]
fn lc_set_reaches_every_control_and_refuses_unknown_ones() {
    let mut h = demo();
    for c in lightcraft_develop::CONTROLS {
        line(&mut h, &format!("lc_set_{} = 255,{},{}", c.id, c.min, c.max)).unwrap_or_else(|e| panic!("{}: {e}", c.id));
        // (wb.temp rides a non-linear track: its top end lands within rounding of the maximum)
        assert!((value(&h, c.id) - c.max).abs() <= 1e-6 * c.max.abs().max(1.0), "{}: {}", c.id, value(&h, c.id));
        line(&mut h, &format!("lc_set_{} =", c.id)).unwrap_or_else(|e| panic!("{}: {e}", c.id));
        assert_eq!(value(&h, c.id), c.default, "{}", c.id);
    }
    for (l, what) in [("lc_set_no.such = 1,0,2", "no.such"), ("lc_set_light.exposur =", "light.exposur"), ("lc_set_ = 1,0,2", "no develop control")] {
        let e = line(&mut h, l).unwrap_err();
        assert!(e.contains(what), "{l}: {e}");
    }
}

#[test]
fn lc_set_indexed_point_color_and_red_eye() {
    let mut h = demo();
    // no sample yet: a list element that isn't there is an error, not a silent no-op
    let e = line(&mut h, "lc_set_pointColor.0.satShift = 0,-100,100").unwrap_err();
    assert!(e.contains("no such list element"), "{e}");
    exec(&mut h, "pointColor.pick", json!({"x": 0.5, "y": 0.08}));
    line(&mut h, "lc_set_pointColor.0.satShift = 0,-100,100").unwrap();
    assert_eq!(dev(&h).point_colors[0].sat_shift, -100.0);
    line(&mut h, "lc_set_pointColor.0.satShift = ^10,-100,100").unwrap();
    assert_eq!(dev(&h).point_colors[0].sat_shift, -90.0);
    line(&mut h, "lc_set_pointColor.0.satShift =").unwrap();
    assert_eq!(dev(&h).point_colors[0].sat_shift, 0.0);
    assert!(line(&mut h, "lc_set_pointColor.3.hueShift = 0,-100,100").is_err(), "only one sample exists");
    assert!(line(&mut h, "lc_set_pointColor.0.nope =").is_err());
    exec(&mut h, "redeye.add", json!({"center": [0.3, 0.4], "rx": 0.03, "ry": 0.02}));
    line(&mut h, "lc_set_redEye.0.darken = 255,0,100").unwrap();
    assert_eq!(dev(&h).red_eye[0].darken, 100.0);
    line(&mut h, "lc_set_redEye.0.darken =").unwrap();
    assert_eq!(dev(&h).red_eye[0].darken, 50.0);
}

#[test]
fn colour_wheels_and_curve_splits() {
    let mut h = demo();
    // X is the wheel's saturation (0–100), Y its hue (0–360)
    line(&mut h, "ShadowsWheelX = 255,0,100").unwrap();
    line(&mut h, "ShadowsWheelY = 255,0,360").unwrap();
    let g = dev(&h).grading;
    assert_eq!((g.shadows.sat, g.shadows.hue), (100.0, 360.0));
    line(&mut h, "MidtonesWheelY = 128,0,360").unwrap();
    assert!((dev(&h).grading.midtones.hue - 180.7).abs() < 0.1);
    line(&mut h, "HighlightsWheelX = ^5,0,100").unwrap();
    assert_eq!(dev(&h).grading.highlights.sat, 5.0);
    line(&mut h, "GlobalWheelY = ^10,0,360").unwrap();
    line(&mut h, "GlobalWheelY = ^10,0,360").unwrap();
    assert_eq!(dev(&h).grading.global.hue, 20.0);
    // a plain press puts the wheel's axis back
    line(&mut h, "ShadowsWheelX = reset").unwrap();
    assert_eq!(dev(&h).grading.shadows.sat, 0.0);
    assert_eq!(dev(&h).grading.shadows.hue, 360.0, "the other axis stays");
    // the tone-curve split points
    line(&mut h, "ParametricShadowSplit = 255,10,70").unwrap();
    line(&mut h, "ParametricMidtoneSplit = 0,20,80").unwrap();
    line(&mut h, "ParametricHighlightSplit = ^5,30,90").unwrap();
    let c = dev(&h).curve;
    assert_eq!((c.split_shadows, c.split_mid, c.split_highlights), (70.0, 20.0, 80.0));
    line(&mut h, "ParametricShadowSplit = reset").unwrap();
    assert_eq!(dev(&h).curve.split_shadows, 25.0);
}

fn mask_adjust(h: &Headless) -> lightcraft_develop::LocalAdjustments {
    let d = dev(h);
    let id = h.app.session.active_mask.expect("a mask is selected");
    d.masks.iter().find(|m| m.id == id).expect("the selected mask").adjust
}

#[test]
fn local_sliders_move_the_selected_mask() {
    let mut h = demo();
    let e = line(&mut h, "local_Exposure = 255,-4,4").unwrap_err();
    assert!(e.contains("no mask selected"), "{e}");
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    line(&mut h, "local_Exposure = 255,-4,4").unwrap();
    assert_eq!(mask_adjust(&h).exposure, 4.0);
    // Monogram draws local_Hue ±180; the slider spans LightCraft's own ±100
    line(&mut h, "local_Hue = 255,-180,180").unwrap();
    assert_eq!(mask_adjust(&h).hue, 100.0);
    line(&mut h, "local_Exposure = v1,-4,4").unwrap();
    assert_eq!(mask_adjust(&h).exposure, 3.0);
    line(&mut h, "local_LuminanceNoise = 0,-100,100").unwrap();
    assert_eq!(mask_adjust(&h).noise, -100.0);
    line(&mut h, "local_Exposure =").unwrap();
    assert_eq!(mask_adjust(&h).exposure, 0.0);
    // the other mask is untouched
    exec(&mut h, "mask.add", json!({"kind": "linear"}));
    line(&mut h, "local_Temperature = 255,-100,100").unwrap();
    assert_eq!(mask_adjust(&h).temp, 100.0);
    let first = dev(&h).masks[0].adjust;
    assert_eq!((first.temp, first.hue), (0.0, 100.0));
    // all 17 names land on a different field
    let mut seen = std::collections::HashSet::new();
    for (name, key) in crate::keymap::LR_LOCAL {
        line(&mut h, &format!("{name} = 255,-100,100")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let v = serde_json::to_value(mask_adjust(&h)).unwrap();
        assert!(v[key].as_f64().unwrap() > 0.0, "{name} → {key}");
        assert!(seen.insert(*key), "{key} twice");
    }
    assert_eq!(seen.len(), 17);
}

#[test]
fn local_slider_in_the_adobe_protocol_line() {
    let mut h = demo();
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    // value 90 of -180…180 is 75 % of the way: 50 on the slider's ±100
    crate::keymap::lr_input_json(&mut h.app, r#"{"input": "local_Hue = ", "argv": [-1, -1, -1, -1, -1, 90, -180, 180, -1]}"#).unwrap();
    assert_eq!(mask_adjust(&h).hue, 50.0);
    crate::keymap::lr_input_json(&mut h.app, r#"{"input": "local_Hue = ", "argv": [-1, -1, -1, -1, -1, -1, -1, -1, -1]}"#).unwrap();
    assert_eq!(mask_adjust(&h).hue, 0.0);
}

#[test]
fn reset_tool_inputs() {
    let mut h = demo();
    exec(&mut h, "mask.add", json!({"kind": "linear"}));
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    exec(&mut h, "mask.add", json!({"kind": "brush"}));
    exec(&mut h, "redeye.add", json!({"center": [0.3, 0.4], "rx": 0.03, "ry": 0.02}));
    exec(&mut h, "spot.add", json!({"points": [[0.2, 0.2]], "source": [0.1, 0.0]}));
    line(&mut h, "reset = gradient").unwrap();
    assert_eq!(dev(&h).masks.len(), 2);
    line(&mut h, "reset = circularGradient").unwrap();
    line(&mut h, "reset = brushing").unwrap();
    assert!(dev(&h).masks.is_empty());
    line(&mut h, "reset = redeye").unwrap();
    line(&mut h, "reset = spotremoval").unwrap();
    assert!(dev(&h).red_eye.is_empty() && dev(&h).spots.is_empty());
    exec(&mut h, "crop.set", json!({"rect": [0.1, 0.1, 0.9, 0.9]}));
    assert_ne!(dev(&h).crop, Default::default());
    line(&mut h, "reset = crop").unwrap();
    assert_eq!(dev(&h).crop, Default::default());
}

#[test]
fn lc_cmd_runs_a_chain_in_order() {
    let mut h = demo();
    let r =
        line(&mut h, r#"lc_cmd = develop.set {"control": "light.exposure", "value": 1} ; develop.set {"control": "light.contrast", "value": 20}"#)
            .unwrap();
    assert_eq!(r["ran"], 2);
    assert_eq!((value(&h, "light.exposure"), value(&h, "light.contrast")), (1.0, 20.0));
    // a step with no params, and a ` ; ` inside a JSON string stays in it
    let r = line(
        &mut h,
        r#"lc_cmd = develop.resetControl {"control": "light.exposure"} ; mask.add {"kind": "radial", "name": "a ; b"} ; mask.select {"id": null}"#,
    )
    .unwrap();
    assert_eq!(r["ran"], 3);
    assert_eq!(value(&h, "light.exposure"), 0.0);
    assert_eq!(dev(&h).masks[0].name, "a ; b");
}

#[test]
fn lc_cmd_chain_stops_at_the_first_error_and_says_which() {
    let mut h = demo();
    let e = line(
        &mut h,
        r#"lc_cmd = develop.set {"control": "light.exposure", "value": 2} ; no.such.command {"x": 1} ; develop.set {"control": "light.contrast", "value": 30}"#,
    )
    .unwrap_err();
    assert!(e.contains("step 2") && e.contains("no.such.command"), "{e}");
    assert_eq!(value(&h, "light.exposure"), 2.0, "the step before ran");
    assert_eq!(value(&h, "light.contrast"), 0.0, "the step after did not");
    // a step that doesn't parse is refused before any runs
    let e = line(&mut h, r#"lc_cmd = develop.set {"control": "light.exposure", "value": 3} ; develop.set {oops"#).unwrap_err();
    assert!(e.contains("lc_cmd"), "{e}");
    assert_eq!(value(&h, "light.exposure"), 2.0, "nothing ran");
    // a single command keeps working as before
    line(&mut h, r#"lc_cmd = develop.set {"control": "light.exposure", "value": 4}"#).unwrap();
    assert_eq!(value(&h, "light.exposure"), 4.0);
}

// ---- cycle lists step on from what the active photo is on now

fn set_cycle(h: &mut Headless, list: &str, items: &[(&str, &str, Value)]) {
    let items: Vec<Value> = items.iter().map(|(l, c, p)| json!({"label": l, "command": c, "params": p})).collect();
    exec(h, "keys.set", json!({"cycles": {(list): items}}));
}

fn aspect(a: &str) -> Value {
    json!({"aspect": a})
}

fn press(h: &mut Headless, list: &str) -> Value {
    line(h, &format!("lc_cycle = {list}")).unwrap()["label"].clone()
}

fn ratio_of_original(h: &Headless) -> f64 {
    let id = h.app.session.active().unwrap();
    let p = h.app.session.catalog.photo(id).unwrap();
    f64::from(p.width.max(p.height)) / f64::from(p.width.min(p.height))
}

#[test]
fn cycle_press_never_reapplies_the_aspect_the_photo_is_already_on() {
    let mut h = demo();
    assert!((ratio_of_original(&h) - 1.25).abs() > 0.01, "the demo photo is not 4x5");
    set_cycle(&mut h, "ratios", &[("As Shot", "crop.aspect", aspect("original")), ("4 × 5", "crop.aspect", aspect("4x5"))]);
    exec(&mut h, "crop.aspect", aspect("4x5"));
    assert_eq!(press(&mut h, "ratios"), "As Shot", "already 4x5: the next one, not 4x5 again");
    assert_eq!(press(&mut h, "ratios"), "4 × 5");
    // the dial too: −1 steps back from the matched item
    assert_eq!(line(&mut h, "lc_cycle_ratios = v1,0,1").unwrap()["label"], "As Shot");
}

#[test]
fn cycle_press_follows_the_photo_not_the_remembered_position() {
    let mut h = demo();
    let ids: Vec<u64> = h.app.session.visible_cloned().iter().take(2).map(|p| p.0).collect();
    set_cycle(
        &mut h,
        "ratios",
        &[("4 × 5", "crop.aspect", aspect("4x5")), ("4 × 3", "crop.aspect", aspect("4x3")), ("3 × 2", "crop.aspect", aspect("3x2"))],
    );
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    exec(&mut h, "crop.aspect", aspect("4x5"));
    assert_eq!(press(&mut h, "ratios"), "4 × 3");
    assert_eq!(press(&mut h, "ratios"), "3 × 2");
    // another photo, on 4x3: the remembered position (3x2) is not where this one is
    exec(&mut h, "library.select", json!({"ids": [ids[1]]}));
    exec(&mut h, "crop.aspect", aspect("4x3"));
    assert_eq!(press(&mut h, "ratios"), "3 × 2");
    // a rotated crop (3x4 on a landscape photo) is still 4x3: ratios compare long side to short
    exec(&mut h, "crop.aspect", aspect("3x4"));
    assert_eq!(line(&mut h, "lc_cycle = ratios").unwrap()["label"], "3 × 2");
    // back on the first photo (3x2 now): the dial goes back from there
    exec(&mut h, "library.select", json!({"ids": [ids[0]]}));
    assert_eq!(line(&mut h, "lc_cycle_ratios = v1,0,1").unwrap()["label"], "4 × 3");
}

#[test]
fn cycle_press_with_no_match_uses_the_remembered_position() {
    let mut h = demo();
    // nothing in the list is on screen (the photo is 4x5, the list has 1x1 and 16x9 only)
    set_cycle(&mut h, "odd", &[("1 × 1", "crop.aspect", aspect("1x1")), ("16 × 9", "crop.aspect", aspect("16x9"))]);
    exec(&mut h, "crop.aspect", aspect("4x5"));
    assert_eq!(press(&mut h, "odd"), "1 × 1", "the first press starts the list");
    // now 1x1 is on screen and matches: the next one
    assert_eq!(press(&mut h, "odd"), "16 × 9");
    // items that can't be read back (a preset, a slider value): step by the remembered position
    let set = |v: f64| json!({"control": "light.exposure", "value": v});
    set_cycle(&mut h, "looks", &[("A", "develop.set", set(1.0)), ("B", "develop.set", set(2.0))]);
    assert_eq!(press(&mut h, "looks"), "A");
    assert_eq!(press(&mut h, "looks"), "B");
    assert_eq!(press(&mut h, "looks"), "A");
}

#[test]
fn cycle_press_reads_the_white_balance_mode() {
    let mut h = demo();
    let wb = |m: &str| json!({"mode": m});
    set_cycle(
        &mut h,
        "wb",
        &[("As Shot", "develop.wb", wb("asShot")), ("Daylight", "develop.wb", wb("daylight")), ("Shade", "develop.wb", wb("shade"))],
    );
    exec(&mut h, "develop.wb", wb("daylight"));
    assert_eq!(press(&mut h, "wb"), "Shade", "on Daylight: the next is Shade");
    exec(&mut h, "develop.wb", wb("asShot"));
    assert_eq!(press(&mut h, "wb"), "Daylight", "back on As Shot by hand: Daylight");
}
