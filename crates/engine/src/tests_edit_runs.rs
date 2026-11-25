//! Bryan, 2026-11-24: "dont split into a bunch of undo steps just every action be one … if i crop and
//! turn it a little then stop then turn it more and then hit enter have those just be one action".
//! A crop session, and a run of edits of one control, are ONE undo step and one History entry.

use serde_json::json;

use crate::Session;

fn active_dev(s: &Session) -> lightcraft_develop::DevelopSettings {
    (*s.develop_of(s.active().unwrap()).unwrap()).clone()
}

fn history(s: &mut Session) -> Vec<String> {
    let h = s.execute("history.list", &json!({})).unwrap();
    h["history"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect()
}

/// The clock ticks (a "pause") between edits.
fn pause(s: &mut Session, secs: u32) {
    s.clock = Box::new(move || format!("2026-09-30T12:00:{secs:02}"));
}

#[test]
fn a_whole_crop_session_is_one_undo_step() {
    let mut s = Session::with_demo();
    // a crop to work with, closed off as its own step
    s.execute("crop.set", &json!({"rect": [0.1, 0.1, 0.7, 0.7]})).unwrap();
    s.execute("develop.endRun", &json!({})).unwrap();
    let original = active_dev(&s).crop.geometry;
    let (n_undo, n_hist) = (s.undo.len(), history(&mut s).len());

    // a drag of the crop (an interaction, as the mouse makes it)
    s.execute("develop.beginInteraction", &json!({"label": "Crop"})).unwrap();
    s.execute("crop.drag", &json!({"handle": "move", "from": [0.4, 0.4], "to": [0.45, 0.43]})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    // a pause, then the angle dial
    pause(&mut s, 2);
    s.execute("crop.straighten", &json!({"delta": 1.5})).unwrap();
    // a pause, then more turning
    pause(&mut s, 5);
    s.execute("crop.straighten", &json!({"delta": 1.0})).unwrap();
    // the wheel (an interaction too)
    s.execute("develop.beginInteraction", &json!({"label": "Crop"})).unwrap();
    s.execute("crop.nudge", &json!({"scale": 8.0})).unwrap();
    s.execute("develop.endInteraction", &json!({})).unwrap();
    s.execute("crop.aspect", &json!({"aspect": "4x5"})).unwrap();
    // Enter
    s.execute("develop.endRun", &json!({})).unwrap();

    assert_ne!(active_dev(&s).crop.geometry, original, "the session changed the crop");
    assert_eq!(s.undo.len(), n_undo + 1, "the whole crop session is one undo entry");
    assert_eq!(s.undo.last().unwrap().label, "Crop");
    let h = history(&mut s);
    assert_eq!(h.len(), n_hist + 1, "one History entry: {h:?}");
    assert_eq!(h.last().unwrap(), "Crop");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(active_dev(&s).crop.geometry, original, "one undo returns the crop from before the session");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!((active_dev(&s).crop.geometry.angle - 2.5).abs() < 1e-9, "redo brings the whole session back");
}

#[test]
fn edits_of_one_control_merge_until_another_control_is_used() {
    let mut s = Session::with_demo();
    let n = s.undo.len();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.25})).unwrap();
    pause(&mut s, 2);
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.25})).unwrap();
    assert_eq!(s.undo.len(), n + 1, "two exposure edits with a pause between are one step");
    assert_eq!(s.undo.last().unwrap().label, "Exposure");
    assert!((active_dev(&s).light.exposure - 0.5).abs() < 1e-9);
    // another control: a step of its own
    s.execute("develop.adjust", &json!({"control": "light.contrast", "delta": 5.0})).unwrap();
    assert_eq!(s.undo.len(), n + 2);
    // and the exposure after it starts a new run
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.0})).unwrap();
    s.execute("develop.set", &json!({"control": "light.exposure", "value": 1.5})).unwrap();
    assert_eq!(s.undo.len(), n + 3);
    // undoing the exposure run goes back to the contrast edit, exposure as it was
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((active_dev(&s).light.exposure - 0.5).abs() < 1e-9 && active_dev(&s).light.contrast == 5.0);
    let h = history(&mut s);
    assert_eq!(h.iter().filter(|l| l.as_str() == "Exposure").count(), 1, "{h:?}");
}

#[test]
fn a_crop_session_then_another_edit_are_two_steps() {
    let mut s = Session::with_demo();
    let n = s.undo.len();
    s.execute("crop.set", &json!({"rect": [0.1, 0.1, 0.7, 0.7]})).unwrap();
    s.execute("crop.straighten", &json!({"delta": 2.0})).unwrap();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.5})).unwrap();
    assert_eq!(s.undo.len(), n + 2, "crop session + exposure");
    assert_eq!(s.undo[n].label, "Crop");
    // a crop move after the exposure is a new crop session
    s.execute("crop.nudge", &json!({"x": 2.0})).unwrap();
    assert_eq!(s.undo.len(), n + 3);
}

#[test]
fn an_undo_or_another_photo_ends_a_run() {
    let mut s = Session::with_demo();
    let n = s.undo.len();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.25})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.5})).unwrap();
    assert_eq!(s.undo.len(), n + 1);
    assert!((active_dev(&s).light.exposure - 0.5).abs() < 1e-9, "not merged into the undone edit");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(active_dev(&s).light.exposure, 0.0);
    // the same control on another photo is its own step
    let a = s.active().unwrap();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.25})).unwrap();
    let other = *s.visible().iter().find(|p| **p != a).unwrap();
    s.execute("library.select", &json!({"ids": [other.0]})).unwrap();
    s.execute("develop.adjust", &json!({"control": "light.exposure", "delta": 0.25})).unwrap();
    assert_eq!(s.undo.len(), n + 2, "one step per photo");
}

#[test]
fn escape_cancels_the_whole_crop_session() {
    let mut s = Session::with_demo();
    let original = active_dev(&s).crop;
    let n = s.undo.len();
    s.execute("crop.aspect", &json!({"aspect": "1x1"})).unwrap();
    s.execute("crop.straighten", &json!({"delta": 3.0})).unwrap();
    s.execute("crop.nudge", &json!({"scale": 10.0})).unwrap();
    assert_ne!(active_dev(&s).crop, original);
    let r = s.execute("crop.cancel", &json!({})).unwrap();
    assert_eq!(r["changed"], true);
    assert_eq!(active_dev(&s).crop, original, "everything since the session began is reverted");
    assert_eq!(s.undo.len(), n, "and no undo step is left");
    // nothing to cancel: no change, no error
    let r = s.execute("crop.cancel", &json!({})).unwrap();
    assert_eq!(r["changed"], false);
    // a finished session is not cancelled by a later Esc
    s.execute("crop.aspect", &json!({"aspect": "1x1"})).unwrap();
    s.execute("develop.endRun", &json!({})).unwrap();
    let r = s.execute("crop.cancel", &json!({})).unwrap();
    assert_eq!(r["changed"], false);
    assert_eq!(s.undo.len(), n + 1);
}
