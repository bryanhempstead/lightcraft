//! `develop.listControls` (the catalog of every develop control) and `develop.resetLocalTool`.

use serde_json::json;

use crate::Session;

fn active_dev(s: &Session) -> lightcraft_develop::DevelopSettings {
    (*s.develop_of(s.active().unwrap()).unwrap()).clone()
}

#[test]
fn list_controls_has_every_control_id() {
    let mut s = Session::with_demo();
    let r = s.execute("develop.listControls", &json!({})).unwrap();
    let rows = r["controls"].as_array().unwrap();
    let want = lightcraft_develop::CONTROLS.len() + lightcraft_develop::controls::INDEXED.len();
    assert_eq!(r["count"], want);
    assert_eq!(rows.len(), want);
    for c in lightcraft_develop::CONTROLS {
        let row = rows.iter().find(|x| x["id"] == c.id).unwrap_or_else(|| panic!("{} missing", c.id));
        assert_eq!((row["min"].as_f64(), row["max"].as_f64(), row["default"].as_f64()), (Some(c.min), Some(c.max), Some(c.default)), "{}", c.id);
        assert_eq!(row["label"], c.label);
        assert_eq!(row["sectionLabel"], c.section.label());
        assert_eq!(row["indexed"], false);
    }
    // the list-element templates are there too, marked indexed
    let t = rows.iter().find(|x| x["id"] == "pointColor.hueShift").unwrap();
    assert_eq!((t["indexed"].clone(), t["section"].clone()), (json!(true), json!("pointColor")));
    assert!(rows.iter().any(|x| x["id"] == "redEye.darken" && x["indexed"] == true));
    // a section narrows it; it needs no photo
    let r = s.execute("develop.listControls", &json!({"section": "grading"})).unwrap();
    assert_eq!(r["count"], lightcraft_develop::controls::in_section(lightcraft_develop::Section::Grading).count());
}

#[test]
fn reset_local_tool_removes_one_kind_in_one_undo_step() {
    let mut s = Session::with_demo();
    s.execute("mask.add", &json!({"kind": "linear"})).unwrap();
    s.execute("mask.add", &json!({"kind": "radial"})).unwrap();
    s.execute("mask.add", &json!({"kind": "brush"})).unwrap();
    s.execute("redeye.add", &json!({"center": [0.3, 0.4], "rx": 0.03, "ry": 0.02})).unwrap();
    s.execute("spot.add", &json!({"points": [[0.2, 0.2]], "source": [0.1, 0.0]})).unwrap();
    let kinds = |s: &Session| active_dev(s).masks.len();
    assert_eq!(kinds(&s), 3);
    let r = s.execute("develop.resetLocalTool", &json!({"tool": "linear"})).unwrap();
    assert_eq!((r["removed"].clone(), kinds(&s)), (json!(1), 2));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(kinds(&s), 3, "one undo step");
    s.execute("develop.resetLocalTool", &json!({"tool": "radial"})).unwrap();
    s.execute("develop.resetLocalTool", &json!({"tool": "brush"})).unwrap();
    assert_eq!(kinds(&s), 1, "the linear one is left");
    assert_eq!(s.execute("develop.resetLocalTool", &json!({"tool": "spots"})).unwrap()["removed"], 1);
    assert_eq!(s.execute("develop.resetLocalTool", &json!({"tool": "redEye"})).unwrap()["removed"], 1);
    let d = active_dev(&s);
    assert!(d.spots.is_empty() && d.red_eye.is_empty());
    assert_eq!(s.execute("develop.resetLocalTool", &json!({"tool": "spots"})).unwrap()["removed"], 0);
    assert!(s.execute("develop.resetLocalTool", &json!({"tool": "nope"})).is_err());
    assert!(s.execute("develop.resetLocalTool", &json!({})).is_err());
}
