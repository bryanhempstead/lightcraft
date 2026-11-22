//! L18: Square Grid compact / expanded cells (Settings ▸ Interface): expanded cells add a line
//! with the dimensions and capture date and rotate buttons in the footer.

use serde_json::json;

use crate::tests_library2::{T, click, headless, settle, shot};

#[test]
fn expanded_grid_cells_rotate() {
    let s = lightcraft_engine::Session::with_demo();
    let mut h = headless(s);
    h.app.run("view.squareGrid", json!({})).unwrap();
    h.app.ui.show_filenames = true;
    settle(&mut h);
    let w = h.request("ui.widgets", json!({}), T).to_string();
    assert!(!w.contains("cellRotateLeft"), "compact cells have no rotate buttons");
    h.app.run("app.settings", json!({"tab": "interface"})).unwrap();
    settle(&mut h);
    click(&mut h, "button:settingsGridStyle-1");
    assert!(h.app.ui.settings.grid_expanded);
    h.app.ui.dialog = None;
    settle(&mut h);
    let id = h.app.session.visible_cloned()[0];
    let before = h.app.session.catalog.photo(id).unwrap().develop.orientation;
    click(&mut h, &format!("button:cellRotateRight-{}", id.0));
    assert_ne!(h.app.session.catalog.photo(id).unwrap().develop.orientation, before, "rotated");
    shot(&mut h, "l18-expanded.png");
}

/// LRC-LIB-GRIDOPTIONS: the per-item switches (flags, edited badge, label tint, rotation buttons,
/// info tooltips) live on Settings ▸ Interface and change the cells.
#[test]
fn grid_view_option_switches() {
    let mut s = lightcraft_engine::Session::with_demo();
    let id = s.visible_cloned()[0];
    s.execute("photo.flag", &json!({"ids": [id.0], "flag": "pick"})).unwrap();
    s.execute("photo.label", &json!({"ids": [id.0], "label": "red"})).unwrap();
    let mut h = headless(s);
    h.app.run("view.squareGrid", json!({})).unwrap();
    h.app.ui.settings.grid_expanded = true;
    h.app.run("app.settings", json!({"tab": "interface"})).unwrap();
    settle(&mut h);
    for c in ["gridFlags", "gridEditBadge", "gridTint", "gridRotate", "gridTooltips"] {
        assert!(h.app.widgets.iter().any(|(w, _)| *w == format!("check:settings.{c}")), "{c} on Interface");
    }
    shot(&mut h, "grid-options-settings.png");
    click(&mut h, "check:settings.gridRotate");
    click(&mut h, "check:settings.gridTint");
    click(&mut h, "check:settings.gridFlags");
    click(&mut h, "check:settings.gridTooltips");
    let st = &h.app.ui.settings;
    assert_eq!((st.grid_rotate, st.grid_tint, st.grid_flags, st.grid_tooltips), (false, false, false, true));
    h.app.ui.dialog = None;
    settle(&mut h);
    let w = h.request("ui.widgets", json!({}), T).to_string();
    assert!(!w.contains("cellRotateLeft"), "no rotation buttons when switched off");
    shot(&mut h, "grid-options-off.png");
    let p = h.app.session.catalog.photo(id).unwrap().clone();
    let tip = crate::panels::grid::cell_tooltip(&p);
    assert!(tip.starts_with(&p.file_name) && tip.contains(&format!("{} × {}", p.width, p.height)), "{tip}");
}
