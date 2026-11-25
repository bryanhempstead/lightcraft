//! Hardware-button commands (`crate::device_cmds`): `view.section` opens a Develop panel and picks
//! a slider, never closes anything; `tool.targetedAdjustment` arms the targeted adjustment tool.

use serde_json::{Value, json};

use crate::headless::Headless;
use crate::tests_library2::{T, headless, settle};
use lightcraft_develop::controls;

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    h.request("engine.execute", json!({"command": command, "params": params}), T)
}

fn demo() -> Headless {
    let mut h = headless(lightcraft_engine::Session::with_demo());
    settle(&mut h);
    h
}

#[test]
fn view_section_opens_a_closed_section_and_never_closes_it() {
    let mut h = demo();
    assert!(!h.app.ui.section_open("toneCurve"));
    for _ in 0..2 {
        let r = run(&mut h, "view.section", json!({"id": "toneCurve"}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["open"], true, "{r}");
        settle(&mut h);
        assert!(h.app.ui.section_open("toneCurve"));
    }
    assert_eq!(h.app.ui.module, crate::state::Module::Develop);
    // the older names still work, and the section's header is on screen
    let r = run(&mut h, "view.section", json!({"id": "color"}));
    assert_eq!(r["result"]["section"], "hsl", "{r}");
    settle(&mut h);
    assert!(h.app.ui.section_open("hsl"));
    assert!(h.app.widgets.iter().any(|(w, _)| w == "section:hsl"));
}

#[test]
fn view_section_with_a_control_picks_that_slider() {
    let mut h = demo();
    assert!(crate::widgets::picked_slider(&h.view.ctx).is_none());
    let r = run(&mut h, "view.section", json!({"id": "basic", "control": "light.shadows"}));
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert!(h.app.ui.section_open("basic"));
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.shadows"));
    // a second press keeps it picked and open; another control moves the pick
    run(&mut h, "view.section", json!({"id": "basic", "control": "light.shadows"}));
    settle(&mut h);
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.shadows"));
    run(&mut h, "view.section", json!({"id": "basic", "control": "light.whites"}));
    settle(&mut h);
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.whites"));
    // a control in another panel opens that panel (and, for the mixer, shows its tab)
    let r = run(&mut h, "view.section", json!({"id": "hsl", "control": "mixer.blue.sat"}));
    assert_eq!(r["ok"], true, "{r}");
    settle(&mut h);
    assert!(h.app.ui.section_open("hsl"));
    assert_eq!(h.app.ui.mixer_mode, "saturation");
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("mixer.blue.sat"));
}

#[test]
fn view_section_rejects_unknown_ids_and_controls() {
    let mut h = demo();
    for bad in [
        json!({"id": "nope"}),
        json!({"id": "basic", "control": "light.nope"}),
        json!({"id": "detail", "control": "light.shadows"}),
        json!({}),
        json!({"id": 3}),
        json!({"id": "basic", "control": ""}),
    ] {
        let r = run(&mut h, "view.section", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
        assert!(r["error"].as_str().is_some_and(|e| e.starts_with("view.section")), "{r}");
    }
}

#[test]
fn targeted_adjustment_arms_the_tool_and_stays_on() {
    let mut h = demo();
    let r = run(&mut h, "tool.targetedAdjustment", json!({"target": "curve"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.tool, "tat:curve");
    assert!(h.app.ui.section_open("toneCurve"));
    // pressing again keeps it on
    run(&mut h, "tool.targetedAdjustment", json!({"target": "curve", "on": true}));
    assert_eq!(h.app.ui.tool, "tat:curve");
    settle(&mut h);
    assert_eq!(h.app.ui.tool, "tat:curve");
    // off clears it, and off again is fine
    let r = run(&mut h, "tool.targetedAdjustment", json!({"target": "curve", "on": false}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.tool, "");
    run(&mut h, "tool.targetedAdjustment", json!({"target": "curve", "on": false}));
    assert_eq!(h.app.ui.tool, "");
}

#[test]
fn targeted_adjustment_for_a_colour_attribute_opens_hsl_and_sets_the_mode() {
    let mut h = demo();
    assert!(!h.app.ui.section_open("hsl"));
    for (target, tool, mode) in [("hue", "tat:hue", "hue"), ("saturation", "tat:sat", "saturation"), ("luminance", "tat:lum", "luminance")] {
        let r = run(&mut h, "tool.targetedAdjustment", json!({"target": target}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(h.app.ui.tool, tool);
        assert_eq!(h.app.ui.mixer_mode, mode);
        assert!(h.app.ui.section_open("hsl"));
        settle(&mut h);
        assert_eq!(h.app.ui.tool, tool);
    }
    let r = run(&mut h, "tool.targetedAdjustment", json!({"target": "luminance", "on": false}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.tool, "");
    for bad in [json!({}), json!({"target": "tint"}), json!({"target": "hue", "on": "yes"}), json!({"target": 4})] {
        let r = run(&mut h, "tool.targetedAdjustment", bad.clone());
        assert_eq!(r["ok"], false, "{bad}: {r}");
    }
}

// ---------------------------------------------------------------------------- not shown → error

fn set(h: &mut Headless, settings: Value) {
    let r = run(h, "develop.merge", json!({"settings": settings}));
    assert_eq!(r["ok"], true, "{r}");
    settle(h);
}

/// Nothing is waiting for a frame: no pending pick, no scroll request in the context.
fn nothing_pending(h: &Headless) -> bool {
    let d = &h.app.ui.device;
    d.slider.is_none() && d.reveal.is_none() && h.view.ctx.data(|m| m.get_temp::<String>(egui::Id::new("lc-reveal"))).is_none()
}

/// A refused `control`: an error that names the control and says why, and nothing left behind.
fn refused(h: &mut Headless, control: &str, why: &str) {
    // a finished call first, so a stale request would show
    run(h, "view.section", json!({"id": "basic", "control": "light.exposure"}));
    settle(h);
    assert!(nothing_pending(h));
    let tool = h.app.ui.tool.clone();
    let r = run(h, "view.section", json!({"control": control}));
    assert_eq!(r["ok"], false, "{control}: {r}");
    let e = r["error"].as_str().unwrap_or_default();
    assert!(e.contains(&format!("`{control}` isn't shown")) && e.contains(why), "{control}: {e}");
    settle(h);
    assert!(nothing_pending(h), "{control}: a request was left behind");
    assert_eq!(h.app.ui.tool, tool);
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some("light.exposure"));
}

fn shown(h: &mut Headless, control: &str, drawn_as: &str) -> Value {
    let r = run(h, "view.section", json!({"control": control}));
    assert_eq!(r["ok"], true, "{control}: {r}");
    assert_eq!(r["result"]["picked"], drawn_as, "{control}: {r}");
    settle(h);
    assert!(h.app.widgets.iter().any(|(w, _)| w == &format!("slider:{drawn_as}")), "{control}: no slider:{drawn_as} on screen");
    assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some(drawn_as), "{control}");
    assert!(nothing_pending(h), "{control}");
    r
}

#[test]
fn a_black_and_white_photo_refuses_the_colour_mixer_and_point_colour() {
    let mut h = demo();
    refused(&mut h, "bw.red", "in colour");
    set(&mut h, json!({"treatment": "bw"}));
    refused(&mut h, "mixer.red.hue", "black & white");
    refused(&mut h, "pointColor.0.hueShift", "black & white");
    shown(&mut h, "bw.red", "bw.red");
    // back in colour the mixer is there and the B&W mix is not
    set(&mut h, json!({"treatment": "color"}));
    shown(&mut h, "mixer.red.hue", "mixer.red.hue");
    refused(&mut h, "bw.red", "in colour");
}

#[test]
fn lens_profile_amounts_need_profile_corrections_and_the_manual_ones_show_on_their_tab() {
    let mut h = demo();
    refused(&mut h, "optics.profileDistortion", "Enable Profile Corrections is off");
    // Manual tab controls show whatever the profile switch says
    shown(&mut h, "optics.distortion", "optics.distortion");
    shown(&mut h, "optics.defringePurple", "optics.defringePurple");
    shown(&mut h, "optics.caRed", "optics.caRed");
    shown(&mut h, "optics.vignetting", "optics.vignetting");
    set(&mut h, json!({"optics": {"lens_profile": true}}));
    shown(&mut h, "optics.distortion", "optics.distortion");
    shown(&mut h, "optics.defringeGreen", "optics.defringeGreen");
    // the profile amounts: drawn once the switch is on and the photo has a profile (the demo has
    // no lens, so they are greyed out: the section opens, nothing is picked, and it says why)
    let r = run(&mut h, "view.section", json!({"control": "optics.profileDistortion"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["picked"], Value::Null, "{r}");
    assert!(r["result"]["note"].as_str().is_some_and(|n| n.contains("greyed out")), "{r}");
    settle(&mut h);
    assert!(h.app.ui.section_open("lensCorrections"));
    assert!(crate::widgets::picked_slider(&h.view.ctx).as_deref() != Some("optics.profileDistortion"));
    assert!(nothing_pending(&h));
}

#[test]
fn a_greyed_out_slider_opens_its_section_with_a_note() {
    let mut h = demo();
    // Post-Crop Vignetting's other sliders are greyed out until its Amount is set
    let r = run(&mut h, "view.section", json!({"control": "vignette.feather"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["picked"], Value::Null, "{r}");
    assert!(r["result"]["note"].as_str().is_some_and(|n| n.contains("Post-Crop Vignetting Amount is 0")), "{r}");
    settle(&mut h);
    assert!(h.app.ui.section_open("effects"));
    assert!(h.app.widgets.iter().any(|(w, _)| w == "section:effects"));
    assert!(crate::widgets::picked_slider(&h.view.ctx).is_none());
    assert!(nothing_pending(&h));
    // with an Amount it is drawn and picked
    set(&mut h, json!({"vignette": {"amount": -30.0}}));
    shown(&mut h, "vignette.feather", "vignette.feather");
}

#[test]
fn point_colours_that_do_not_exist_are_refused_and_existing_ones_are_selected() {
    let mut h = demo();
    refused(&mut h, "pointColor.0.hueShift", "0 point color");
    let r = run(&mut h, "view.section", json!({"control": "pointColor.x.hueShift"}));
    assert_eq!(r["ok"], false, "{r}");
    assert!(nothing_pending(&h));
    set(&mut h, json!({"point_colors": [{}, {}]}));
    refused(&mut h, "pointColor.2.hueShift", "2 point color");
    shown(&mut h, "pointColor.1.satShift", "pointColor.satShift");
    assert_eq!(h.app.ui.point_color, 1);
    // its Range fold opens too
    shown(&mut h, "pointColor.0.hueRange", "pointColor.hueRange");
    assert_eq!(h.app.ui.point_color, 0);
}

#[test]
fn other_controls_the_panels_do_not_draw_are_refused_with_a_reason() {
    let mut h = demo();
    refused(&mut h, "hdr.maxEv", "HDR is off");
    refused(&mut h, "lensBlur.amount", "Lens Blur");
    refused(&mut h, "enhance.denoise", "Denoise");
    refused(&mut h, "grading.shadows.hue", "colour wheels");
    refused(&mut h, "curve.splitMid", "no slider");
    refused(&mut h, "crop.angle", "Crop");
    set(&mut h, json!({"hdr": {"enabled": true}}));
    shown(&mut h, "hdr.maxEv", "hdr.maxEv");
    refused(&mut h, "hdr.sdrBlend", "Preview for SDR Display");
    set(&mut h, json!({"lens_blur": {"enabled": true}}));
    shown(&mut h, "lensBlur.amount", "lensBlur.amount");
    shown(&mut h, "lensBlur.focalRange", "lensBlur.focalRange");
}

#[test]
fn a_control_unfolds_and_switches_what_hides_it() {
    let mut h = demo();
    // a closed sub-section, another mixer tab, the curve's parametric view
    run(&mut h, "ui.fold", json!({"id": "develop.detail.noise.reduction", "open": false}));
    shown(&mut h, "detail.nrColor", "detail.nrColor");
    assert_eq!(h.app.ui.folds.get("develop.detail.noise.reduction"), Some(&true));
    run(&mut h, "view.colorMixer", json!({"mode": "hue"}));
    shown(&mut h, "mixer.green.lum", "mixer.green.lum");
    assert_eq!(h.app.ui.mixer_mode, "luminance");
    run(&mut h, "view.section", json!({"id": "toneCurve"}));
    h.app.ui.curve_channel = "master".into();
    shown(&mut h, "curve.darks", "curve.darks");
    assert_eq!(h.app.ui.curve_channel, "parametric");
    // a rendered file's white balance sliders have their own ids
    shown(&mut h, "wb.temp", "wb.tempRel");
}

/// Every control id the commands accept: when `view.section` says ok and picks a slider, that slider
/// is on screen and picked; when it refuses, it says why. (Run on a colour photo, then a B&W one.)
#[test]
fn every_control_is_either_drawn_or_refused() {
    let mut h = demo();
    set(&mut h, json!({"point_colors": [{}], "optics": {"lens_profile": true}, "hdr": {"enabled": true}, "lens_blur": {"enabled": true}}));
    for bw in [false, true] {
        set(&mut h, json!({"treatment": if bw { "bw" } else { "color" }}));
        let mut ids: Vec<String> = controls::CONTROLS.iter().map(|c| c.id.to_string()).collect();
        ids.extend(["pointColor.0.hueShift", "pointColor.0.variance", "pointColor.0.lumRange"].map(String::from));
        for id in ids {
            let r = run(&mut h, "view.section", json!({"control": id}));
            if r["ok"] != true {
                let e = r["error"].as_str().unwrap_or_default();
                assert!(e.starts_with("view.section:"), "{id}: {e}");
                continue;
            }
            let Some(drawn_as) = r["result"]["picked"].as_str().map(String::from) else {
                assert!(r["result"]["note"].is_string(), "{id}: {r}");
                continue;
            };
            settle(&mut h);
            let on_screen = h.app.widgets.iter().any(|(w, _)| w == &format!("slider:{drawn_as}"));
            assert!(on_screen, "{id} (bw: {bw}): view.section said it picked `{drawn_as}` but no such slider is on screen");
            assert_eq!(crate::widgets::picked_slider(&h.view.ctx).as_deref(), Some(drawn_as.as_str()), "{id}");
        }
    }
}

#[test]
fn targeted_adjustment_on_a_black_and_white_photo_is_luminance_only() {
    let mut h = demo();
    set(&mut h, json!({"treatment": "bw"}));
    run(&mut h, "tool.targetedAdjustment", json!({"target": "curve"}));
    run(&mut h, "tool.targetedAdjustment", json!({"target": "curve", "on": false}));
    for target in ["hue", "saturation"] {
        let r = run(&mut h, "tool.targetedAdjustment", json!({"target": target}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some_and(|e| e.contains("only luminance is available on a black & white photo")), "{r}");
        assert_eq!(h.app.ui.tool, "", "{target}");
        assert!(!h.app.ui.section_open("hsl"), "a refused call changes nothing");
    }
    let r = run(&mut h, "tool.targetedAdjustment", json!({"target": "luminance"}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.tool, "tat:lum");
    assert!(h.app.ui.section_open("hsl"));
    // switching it off is always fine
    let r = run(&mut h, "tool.targetedAdjustment", json!({"target": "hue", "on": false}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(h.app.ui.tool, "");
}

// ---------------------------------------------------------------------------- Tab steps the pick

fn key(h: &mut Headless, k: &str, shift: bool) {
    let r = h.request("ui.key", json!({"key": k, "shift": shift}), T);
    assert_eq!(r["ok"], true, "{r}");
    settle(h);
}

fn picked(h: &Headless) -> Option<String> {
    crate::widgets::picked_slider(&h.view.ctx)
}

/// The sliders on screen in Basic, in drawn order.
fn basic_sliders(h: &Headless) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for (w, _) in &h.app.widgets {
        if let Some(id) = w.strip_prefix("slider:")
            && crate::device_cmds::panel_of_slider(id) == "basic"
            && !v.iter().any(|x| x == id)
        {
            v.push(id.to_string());
        }
    }
    v
}

#[test]
fn tab_steps_down_the_sliders_of_a_section_and_round_the_end() {
    let mut h = demo();
    shown(&mut h, "light.exposure", "light.exposure");
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h).as_deref(), Some("light.contrast"));
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h).as_deref(), Some("light.highlights"));
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h).as_deref(), Some("light.shadows"));
    // Shift-Tab goes back up
    key(&mut h, "Tab", true);
    assert_eq!(picked(&h).as_deref(), Some("light.highlights"));
    // the end of Basic wraps to its start, and the start to its end
    let all = basic_sliders(&h);
    assert!(all.len() > 8, "{all:?}");
    let (first, last) = (all[0].clone(), all[all.len() - 1].clone());
    shown(&mut h, &last.replace("Rel", ""), &last);
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h), Some(first.clone()), "Tab from the last slider of Basic wraps to its first");
    key(&mut h, "Tab", true);
    assert_eq!(picked(&h), Some(last), "Shift-Tab from the first wraps to the last");
    assert!(h.app.ui.right_panel, "Tab did not hide the panels");
    // the stepped-to slider takes the arrows like a clicked one
    shown(&mut h, "light.exposure", "light.exposure");
    key(&mut h, "Tab", false);
    let before = h.app.session.active().and_then(|id| h.app.session.develop_of(id)).map(|d| d.light.contrast).unwrap_or_default();
    key(&mut h, "ArrowUp", false);
    let after = h.app.session.active().and_then(|id| h.app.session.develop_of(id)).map(|d| d.light.contrast).unwrap_or_default();
    assert!(after > before, "Up moved Contrast: {before} -> {after}");
}

#[test]
fn tab_skips_greyed_out_sliders() {
    let mut h = demo();
    // Grain: Size and Roughness are greyed out while Amount is 0, so Tab stays on Amount
    shown(&mut h, "grain.amount", "grain.amount");
    run(&mut h, "ui.fold", json!({"id": "develop.effects.post.crop.vignetting", "open": false}));
    settle(&mut h);
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h).as_deref(), Some("grain.amount"));
    set(&mut h, json!({"grain": {"amount": 20.0}}));
    key(&mut h, "Tab", false);
    assert_eq!(picked(&h).as_deref(), Some("grain.size"));
}

#[test]
fn tab_without_a_picked_slider_still_hides_the_panels() {
    let mut h = demo();
    run(&mut h, "view.develop", json!({}));
    settle(&mut h);
    assert!(picked(&h).is_none());
    assert!(h.app.ui.left_panel && h.app.ui.right_panel);
    key(&mut h, "Tab", false);
    assert!(!h.app.ui.left_panel && !h.app.ui.right_panel, "Tab hides the side panels as before");
    key(&mut h, "Tab", false);
    assert!(h.app.ui.left_panel && h.app.ui.right_panel);
    key(&mut h, "Tab", true);
    assert!(!h.app.ui.left_panel && !h.app.ui.right_panel && !h.app.ui.filmstrip, "Shift-Tab hides everything as before");
    key(&mut h, "Tab", true);
    // a picked slider owns Tab; letting go (Esc) gives it back
    shown(&mut h, "light.exposure", "light.exposure");
    key(&mut h, "Escape", false);
    assert!(picked(&h).is_none());
    key(&mut h, "Tab", false);
    assert!(!h.app.ui.right_panel);
}
