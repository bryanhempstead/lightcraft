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
