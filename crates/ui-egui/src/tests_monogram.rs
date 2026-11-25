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
    line(&mut h, "lc_set_hdr.maxEv = v0.2,0,5").unwrap();
    assert!((value(&h, "hdr.maxEv") - 1.3).abs() < 1e-9);
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

// ---- review fixes

fn history_len(h: &mut Headless) -> usize {
    exec(h, "history.list", json!({}))["history"].as_array().map_or(0, Vec::len)
}

fn last_labels(h: &mut Headless, n: usize) -> Vec<String> {
    let v = exec(h, "history.list", json!({}));
    let all: Vec<String> = v["history"].as_array().unwrap().iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
    all[all.len().saturating_sub(n)..].to_vec()
}

/// 1: a one-shot (`reset = <tool>`, a reset, any `lc_cmd` step) right after a slider move is a history step of its own.
#[test]
fn one_shots_end_the_slider_drag_before_they_run() {
    let mut h = demo();
    exec(&mut h, "mask.add", json!({"kind": "linear"}));
    let before = history_len(&mut h);
    line(&mut h, "Exposure = ^0.1,-5,5").unwrap();
    line(&mut h, "reset = gradient").unwrap();
    assert_eq!(history_len(&mut h), before + 2, "{:?}", last_labels(&mut h, 3));
    assert_eq!(last_labels(&mut h, 2)[0], "Exposure");
    assert_eq!(h.app.session.active_mask, None, "cleared once the edit was in");
    // every chain step, and a single `lc_cmd`
    let before = history_len(&mut h);
    line(&mut h, "Contrast = ^5,-100,100").unwrap();
    line(&mut h, r#"lc_cmd = develop.set {"control": "light.exposure", "value": 1} ; develop.set {"control": "light.whites", "value": 5}"#).unwrap();
    assert_eq!(history_len(&mut h), before + 3, "{:?}", last_labels(&mut h, 4));
    line(&mut h, "Tint = ^2,-150,150").unwrap();
    line(&mut h, r#"lc_cmd = develop.set {"control": "light.blacks", "value": 5}"#).unwrap();
    assert_eq!(history_len(&mut h), before + 5);
    // a reset of another control, and the crop's
    line(&mut h, "Texture = ^5,-100,100").unwrap();
    line(&mut h, "lc_set_light.whites =").unwrap();
    assert_eq!(history_len(&mut h), before + 7, "{:?}", last_labels(&mut h, 3));
    line(&mut h, "Dehaze = ^5,-100,100").unwrap();
    line(&mut h, "reset = crop").unwrap();
    assert!(last_labels(&mut h, 3).iter().any(|l| l == "Dehaze"), "{:?}", last_labels(&mut h, 3));
}

/// 3: an empty step stops the whole line before anything runs.
#[test]
fn lc_cmd_chain_with_an_empty_step_runs_nothing() {
    let mut h = demo();
    let e =
        line(&mut h, r#"lc_cmd = develop.set {"control": "light.exposure", "value": 1} ;  ; develop.set {"control": "light.contrast", "value": 9}"#)
            .unwrap_err();
    assert!(e.contains("step 2") && e.contains("empty"), "{e}");
    assert_eq!((value(&h, "light.exposure"), value(&h, "light.contrast")), (0.0, 0.0));
    assert!(line(&mut h, "lc_cmd =").is_err(), "a single empty command too");
}

/// 4: one command and a chain go the same way, errors included.
#[test]
fn lc_cmd_single_and_chain_behave_alike() {
    let mut h = demo();
    let e = line(&mut h, "lc_cmd = no.such.command").unwrap_err();
    assert!(e.contains("no.such.command"), "{e}");
    let e = line(&mut h, "lc_cmd = view.beforeAfter ; no.such.command").unwrap_err();
    assert!(e.contains("step 2") && e.contains("no.such.command"), "{e}");
    // the key path (rating keys) for both
    let rating = |h: &Headless| h.app.session.catalog.photo(h.app.session.active().unwrap()).unwrap().rating;
    line(&mut h, r#"lc_cmd = photo.rate {"rating": 3}"#).unwrap();
    assert_eq!(rating(&h), 3);
    line(&mut h, r#"lc_cmd = photo.rate {"rating": 4} ; view.beforeAfter"#).unwrap();
    assert_eq!(rating(&h), 4);
    // a command that refuses is an error too
    assert!(line(&mut h, r#"lc_cmd = develop.set {"control": "no.such", "value": 1}"#).is_err());
    assert!(line(&mut h, r#"lc_cmd = develop.set {"control": "no.such", "value": 1} ; view.beforeAfter"#).is_err());
}

/// 5: an unedited photo is on As Shot, not on a same-ratio item left as the remembered position.
#[test]
fn cycle_press_on_an_unedited_photo_starts_from_as_shot() {
    let mut h = demo();
    let id = h.app.session.active().unwrap();
    let (w, hh) = h.app.session.catalog.photo(id).map(|p| (p.width, p.height)).unwrap();
    set_cycle(&mut h, "ratios", &[("As Shot", "crop.aspect", aspect("original")), ("Same", "crop.aspect", aspect(&format!("{w}x{hh}")))]);
    assert_eq!(press(&mut h, "ratios"), "Same");
    exec(&mut h, "crop.reset", json!({}));
    assert_eq!(press(&mut h, "ratios"), "Same", "unedited again: the one after As Shot, not As Shot");
}

/// 6: a number that isn't one is an error, never a silent reset.
#[test]
fn non_finite_numbers_are_refused() {
    let mut h = demo();
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    line(&mut h, "lc_set_light.exposure = 1,0,2").unwrap();
    let was = value(&h, "light.exposure");
    for l in [
        "lc_set_light.exposure = nan,-5,5",
        "lc_set_light.exposure = ^nan,-5,5",
        "lc_set_light.exposure = v inf",
        "lc_set_light.exposure = ^inf,-5,5",
        "lc_set_light.exposure = nan",
        "lc_set_light.exposure = 1e999",
        "lc_set_light.exposure = abc",
        "lc_set_light.exposure = 128,nan,5",
        "Exposure = ^inf,-5,5",
        "local_Exposure = nan,-4,4",
        "local_Exposure = ^nan,-4,4",
        "local_Exposure = 1e999,-4,4",
    ] {
        let e = line(&mut h, l).unwrap_err();
        assert!(e.contains("finite"), "{l}: {e}");
    }
    assert_eq!(value(&h, "light.exposure"), was);
    assert_eq!(mask_adjust(&h).exposure, 0.0);
}

/// 7: a bare number is the value; `crop.x` / `y` / `scale` are not `lc_set_` controls.
#[test]
fn lc_set_bare_number_and_crop_pseudo_controls() {
    let mut h = demo();
    line(&mut h, "lc_set_light.exposure = 0.5").unwrap();
    assert_eq!(value(&h, "light.exposure"), 0.5);
    line(&mut h, "lc_set_light.exposure = -2").unwrap();
    assert_eq!(value(&h, "light.exposure"), -2.0);
    line(&mut h, "lc_set_light.exposure = 99").unwrap();
    assert_eq!(value(&h, "light.exposure"), 5.0, "clamped to the range");
    line(&mut h, "lc_set_hdr.maxEv = 2.5").unwrap();
    assert_eq!(value(&h, "hdr.maxEv"), 2.5);
    for c in ["crop.x", "crop.y", "crop.scale"] {
        for l in [format!("lc_set_{c} ="), format!("lc_set_{c} = 0.5"), format!("lc_set_{c} = 128,0,1")] {
            let e = line(&mut h, &l).unwrap_err();
            assert!(e.contains("no develop control"), "{l}: {e}");
        }
    }
}

// ---- the dials and pots do what Monogram's Lightroom plugin does

fn set_temp(h: &mut Headless, k: f64) {
    exec(h, "develop.set", json!({"control": "wb.temp", "value": k}));
}

fn near(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b}");
}

#[test]
fn temperature_dial_steps_grow_with_the_kelvin() {
    let mut h = demo();
    // one tick: 25 K up to 8000, 50 up to 10000, 100 up to 14000, 400 beyond
    for (from, step) in [
        (2000.0, 25.0),
        (5500.0, 25.0),
        (8000.0, 25.0),
        (8025.0, 50.0),
        (9000.0, 50.0),
        (10000.0, 50.0),
        (10050.0, 100.0),
        (14000.0, 100.0),
        (14100.0, 400.0),
        (30000.0, 400.0),
    ] {
        set_temp(&mut h, from);
        line(&mut h, "Temperature = ^1,2000,50000").unwrap();
        assert_eq!(value(&h, "wb.temp"), from + step, "up from {from}");
        set_temp(&mut h, from);
        line(&mut h, "Temperature = v1,2000,50000").unwrap();
        let down = from - temp_scale_below(from);
        assert_eq!(value(&h, "wb.temp"), down.max(2000.0), "down from {from}");
    }
    // Monogram's own step multiplies, and the ends hold
    set_temp(&mut h, 5500.0);
    line(&mut h, "Temperature = ^2,2000,50000").unwrap();
    assert_eq!(value(&h, "wb.temp"), 5550.0);
    set_temp(&mut h, 2010.0);
    line(&mut h, "Temperature = v1,2000,50000").unwrap();
    assert_eq!(value(&h, "wb.temp"), 2000.0);
    set_temp(&mut h, 49900.0);
    line(&mut h, "Temperature = ^1,2000,50000").unwrap();
    assert_eq!(value(&h, "wb.temp"), 50000.0);
    // the line's own range narrows the control's
    set_temp(&mut h, 5990.0);
    line(&mut h, "Temperature = ^1,2000,6000").unwrap();
    assert_eq!(value(&h, "wb.temp"), 6000.0);
}

/// A tick down uses the step of the K it starts from.
fn temp_scale_below(from: f64) -> f64 {
    match from {
        k if k <= 8000.0 => 25.0,
        k if k <= 10000.0 => 50.0,
        k if k <= 14000.0 => 100.0,
        _ => 400.0,
    }
}

#[test]
fn dial_rounds_to_the_controls_precision_and_holds_the_range() {
    let mut h = demo();
    exec(&mut h, "develop.set", json!({"values": {"light.exposure": 0.123, "light.contrast": 3.4}}));
    line(&mut h, "Exposure = ^0.005,-10,10").unwrap();
    assert_eq!(value(&h, "light.exposure"), 0.13, "two decimals");
    line(&mut h, "Contrast = ^1,-100,100").unwrap();
    assert_eq!(value(&h, "light.contrast"), 4.0, "whole numbers");
    // the narrower of the line's range and the control's: ±5 of Exposure, a line of ±1
    line(&mut h, "Exposure = ^20,-10,10").unwrap();
    assert_eq!(value(&h, "light.exposure"), 5.0);
    line(&mut h, "Exposure = v20,-1,1").unwrap();
    assert_eq!(value(&h, "light.exposure"), -1.0);
    // a Monogram line with no range (the adobe protocol's -1, -1) is the control's own
    exec(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0}));
    crate::keymap::lr_input_json(&mut h.app, r#"{"input": "Exposure = ", "argv": [-1, 0, 1, -1, -1, -1, -1, -1, 1]}"#).unwrap();
    assert_eq!(value(&h, "light.exposure"), 1.0);
}

#[test]
fn pots_have_dead_zones_and_the_temperature_track() {
    let mut h = demo();
    // Temperature 2000–50000 K on Monogram's warped track
    line(&mut h, "Temperature = 128,2000,50000").unwrap();
    near(value(&h, "wb.temp"), 6943.0, 2.0, "pot 128");
    line(&mut h, "Temperature = 192,2000,50000").unwrap();
    near(value(&h, "wb.temp"), 12412.0, 5.0, "pot 192");
    // dead zones: the first three detents are the minimum, the last three the maximum
    for (pos, want) in [(0.0, 2000.0), (3.0, 2000.0), (252.0, 50000.0), (255.0, 50000.0)] {
        line(&mut h, &format!("Temperature = {pos},2000,50000")).unwrap();
        assert_eq!(value(&h, "wb.temp"), want, "pot {pos}");
    }
    line(&mut h, "Temperature = 4,2000,50000").unwrap();
    assert!(value(&h, "wb.temp") > 2000.0, "past the dead zone");
    // linear for the others, over the line's range narrowed to the control's (±10 on a ±5 Exposure)
    line(&mut h, "Exposure = 128,-10,10").unwrap();
    near(value(&h, "light.exposure"), -5.0 + 128.0 / 255.0 * 10.0, 1e-9, "exposure pot");
    line(&mut h, "Contrast = 3,-100,100").unwrap();
    assert_eq!(value(&h, "light.contrast"), -100.0);
    line(&mut h, "Contrast = 252,-100,100").unwrap();
    assert_eq!(value(&h, "light.contrast"), 100.0);
    // no pickup: the pot sets the value wherever the control was
    line(&mut h, "Contrast = 128,-50,50").unwrap();
    near(value(&h, "light.contrast"), -50.0 + 128.0 / 255.0 * 100.0, 1e-9, "contrast pot");
    // a bare number is a pot across the control's own range
    line(&mut h, "Blacks = 255").unwrap();
    assert_eq!(value(&h, "light.blacks"), 100.0);
}

#[test]
fn plus_and_minus_lines_step_a_thousandth_of_the_range() {
    let mut h = demo();
    line(&mut h, "Exposure = +").unwrap();
    near(value(&h, "light.exposure"), 0.01, 1e-9, "exposure +");
    line(&mut h, "Exposure = +").unwrap();
    line(&mut h, "Exposure = +").unwrap();
    near(value(&h, "light.exposure"), 0.03, 1e-9, "three +");
    line(&mut h, "Exposure = -").unwrap();
    near(value(&h, "light.exposure"), 0.02, 1e-9, "exposure -");
    // a wide control: 25 at most (Temperature's 48000 K range would be 48)
    set_temp(&mut h, 5500.0);
    line(&mut h, "Temperature = +").unwrap();
    assert_eq!(value(&h, "wb.temp"), 5525.0);
    // the bounds hold
    exec(&mut h, "develop.set", json!({"control": "light.exposure", "value": 5}));
    line(&mut h, "Exposure = +").unwrap();
    assert_eq!(value(&h, "light.exposure"), 5.0);
    // `++` / `--`: one native nudge (an arrow key's), Exposure's is 0.05
    exec(&mut h, "develop.set", json!({"control": "light.exposure", "value": 0}));
    line(&mut h, "Exposure = ++").unwrap();
    near(value(&h, "light.exposure"), 0.05, 1e-9, "exposure ++");
    line(&mut h, "Exposure = --").unwrap();
    near(value(&h, "light.exposure"), 0.0, 1e-9, "exposure --");
    // and a mask slider
    exec(&mut h, "mask.add", json!({"kind": "radial"}));
    line(&mut h, "local_Exposure = +").unwrap();
    near(mask_adjust(&h).exposure, 0.008, 1e-9, "local +");
}

#[test]
fn a_wheel_dial_turns_the_wheel_in_x_and_y() {
    let mut h = demo();
    let set =
        |h: &mut Headless, hue: f64, sat: f64| exec(h, "develop.set", json!({"values": {"grading.shadows.hue": hue, "grading.shadows.sat": sat}}));
    let wheel = |h: &Headless| (dev(h).grading.shadows.hue, dev(h).grading.shadows.sat);
    // straight up the wheel (hue 90, sat 30): an X tick leans it right, a negative one left
    set(&mut h, 90.0, 30.0);
    line(&mut h, "ShadowsWheelX = ^10,0,100").unwrap();
    assert_eq!(wheel(&h), (72.0, 32.0), "x +10: atan2(30, 10), hypot");
    set(&mut h, 90.0, 30.0);
    line(&mut h, "ShadowsWheelX = v10,0,100").unwrap();
    assert_eq!(wheel(&h), (108.0, 32.0));
    // a Y tick along the hue changes only the saturation
    set(&mut h, 90.0, 30.0);
    line(&mut h, "ShadowsWheelY = ^5,0,100").unwrap();
    assert_eq!(wheel(&h), (90.0, 35.0));
    // wrapping below 0°
    set(&mut h, 0.0, 10.0);
    line(&mut h, "ShadowsWheelY = v20,0,100").unwrap();
    assert_eq!(wheel(&h), (297.0, 22.0));
    // from the centre, X goes along hue 0
    set(&mut h, 40.0, 0.0);
    line(&mut h, "ShadowsWheelX = ^1,0,100").unwrap();
    assert_eq!(wheel(&h), (0.0, 1.0));
    // saturation stops at 100
    set(&mut h, 0.0, 100.0);
    line(&mut h, "ShadowsWheelX = ^10,0,100").unwrap();
    assert_eq!(wheel(&h), (0.0, 100.0));
    // the other wheels have their own
    line(&mut h, "GlobalWheelY = ^10,0,360").unwrap();
    assert_eq!((dev(&h).grading.global.hue, dev(&h).grading.global.sat), (90.0, 10.0));
}
