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

#[test]
fn list_controls_refuses_an_unknown_section() {
    let mut s = Session::with_demo();
    let e = s.execute("develop.listControls", &json!({"section": "nope"})).unwrap_err().to_string();
    assert!(e.contains("unknown section"), "{e}");
    assert!(s.execute("develop.listControls", &json!({"section": null})).is_ok());
}

#[test]
fn reset_local_tool_goes_by_the_first_component_and_takes_empty_masks() {
    use lightcraft_develop::{Mask, MaskComponent, MaskOp, MaskShape};
    let mut s = Session::with_demo();
    let id = s.active().unwrap();
    // a linear mask with a subtracted brush, a brush mask with an added linear, and a mask with no component
    let comp = |op, shape| MaskComponent { name: None, op, invert: false, shape };
    let linear = || MaskShape::Linear { start: lightcraft_geom::Point::new(0.5, 0.2), end: lightcraft_geom::Point::new(0.5, 0.6) };
    let brush = || MaskShape::Brush { strokes: vec![] };
    let mut d = active_dev(&s);
    let a = d.next_mask_id();
    let (b, c) = (a + 1, a + 2);
    d.masks.push(Mask { id: a, components: vec![comp(MaskOp::Add, linear()), comp(MaskOp::Subtract, brush())], ..Default::default() });
    d.masks.push(Mask { id: b, components: vec![comp(MaskOp::Add, brush()), comp(MaskOp::Add, linear())], ..Default::default() });
    d.masks.push(Mask { id: c, components: vec![], ..Default::default() });
    s.set_develop(id, d, "test").unwrap();
    s.active_mask = Some(a);
    let r = s.execute("develop.resetLocalTool", &json!({"tool": "linear"})).unwrap();
    assert_eq!((r["removed"].clone(), r["kinds"]["linear"].clone(), r["kinds"]["empty"].clone()), (json!(2), json!(1), json!(1)), "{r}");
    let left: Vec<u32> = active_dev(&s).masks.iter().map(|m| m.id).collect();
    assert_eq!(left, vec![b], "the brush-first mask stays");
    assert_eq!(s.active_mask, None, "the selected mask went with it");
    let r = s.execute("develop.resetLocalTool", &json!({"tool": "brush"})).unwrap();
    assert_eq!(r["kinds"]["brush"], 1);
    assert!(active_dev(&s).masks.is_empty());
}
