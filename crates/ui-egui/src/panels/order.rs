//! The side panels' section order and the sections that open at launch (Bryan, 2026-11-23:
//! "let me adjust the order of the list on the side and the one that default opens by clicking
//! and dragging").
//!
//! Every reorderable panel lists its sections through [`ordered`]; dragging a section header
//! moves the section (a drop line shows where it lands), and the order is kept per side in the app
//! settings (`ui.json` → `settings.panelOrder`). A header's right-click menu has Open by Default
//! (the sections a side opens at launch: `settings.panelDefaultOpen`) and Reset Order.
//!
//! Commands: `panel.order {side, order?}` (no `order`: the current one), `panel.defaultOpen
//! {side?, id, open?}`, `panel.resetOrder {side?}`.

use egui::{Color32, CursorIcon, Rect, Response, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{AppSettings, UiState};
use crate::theme::Tokens;

pub const DEVELOP_RIGHT: &str = "developRight";
pub const DEVELOP_LEFT: &str = "developLeft";
pub const LIBRARY_LEFT: &str = "libraryLeft";
pub const LIBRARY_RIGHT: &str = "libraryRight";

/// The Library left panel's reorderable sections (below the Navigator): (id, title).
pub const LIBRARY_LEFT_SECTIONS: [(&str, &str); 5] =
    [("catalog", "Catalog"), ("shoots", "shoots."), ("folders", "Folders"), ("collections", "Collections"), ("publish", "Publish Services")];

/// Every side with reorderable sections.
pub const SIDES: [&str; 4] = [DEVELOP_RIGHT, DEVELOP_LEFT, LIBRARY_LEFT, LIBRARY_RIGHT];

/// A side's sections in the built-in order: (id, title).
pub fn defaults(side: &str) -> Vec<(&'static str, &'static str)> {
    match side {
        DEVELOP_RIGHT => super::edit::CLASSIC_PANELS.iter().map(|p| (p.0, p.1)).collect(),
        DEVELOP_LEFT => super::develop_left::SECTIONS.to_vec(),
        LIBRARY_LEFT => LIBRARY_LEFT_SECTIONS.to_vec(),
        LIBRARY_RIGHT => super::library_right::SECTIONS.to_vec(),
        _ => Vec::new(),
    }
}

/// The section's title on `side`.
pub fn title(side: &str, id: &str) -> Option<&'static str> {
    defaults(side).into_iter().find(|s| s.0 == id).map(|s| s.1)
}

/// `saved` (a side's saved order) made whole: its known ids in that order, then any section it
/// doesn't name (a section added since) at its built-in place.
fn whole(saved: &[String], defaults: &[(&'static str, &'static str)]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for id in saved {
        if let Some(d) = defaults.iter().find(|d| d.0 == id)
            && !out.contains(&d.0)
        {
            out.push(d.0);
        }
    }
    for (i, d) in defaults.iter().enumerate() {
        if !out.contains(&d.0) {
            // after the built-in section before it, wherever that is now
            let at = defaults.get(..i).and_then(|before| before.iter().rev().find_map(|b| out.iter().position(|o| *o == b.0))).map_or(0, |p| p + 1);
            out.insert(at.min(out.len()), d.0);
        }
    }
    out
}

/// A side's sections in the order the user set (the built-in order when none is saved).
pub fn ordered(settings: &AppSettings, side: &str) -> Vec<&'static str> {
    let saved = settings.panel_order.get(side).map(Vec::as_slice).unwrap_or_default();
    whole(saved, &defaults(side))
}

/// The side a section id belongs to: `side` when given (`right` / `left` = the current module's),
/// else the only side that has it (the current module's when several do).
fn resolve_side(app: &LightcraftApp, side: Option<&str>, id: Option<&str>) -> Result<&'static str, String> {
    let develop = app.ui.module == crate::state::Module::Develop;
    let named = match side {
        Some("right") => Some(if develop { DEVELOP_RIGHT } else { LIBRARY_RIGHT }),
        Some("left") => Some(if develop { DEVELOP_LEFT } else { LIBRARY_LEFT }),
        Some(s) => Some(SIDES.into_iter().find(|x| *x == s).ok_or_else(|| format!("unknown side `{s}` (one of {})", SIDES.join(", ")))?),
        None => None,
    };
    if let Some(s) = named {
        if let Some(id) = id
            && title(s, id).is_none()
        {
            return Err(format!("`{id}` isn't a section of {s}"));
        }
        return Ok(s);
    }
    let Some(id) = id else { return Err("missing `side`".into()) };
    let have: Vec<&'static str> = SIDES.into_iter().filter(|s| title(s, id).is_some()).collect();
    match have.as_slice() {
        [] => Err(format!("no panel has a section `{id}`")),
        [one] => Ok(one),
        several => {
            let current = if develop { [DEVELOP_RIGHT, DEVELOP_LEFT] } else { [LIBRARY_RIGHT, LIBRARY_LEFT] };
            several.iter().copied().find(|s| current.contains(s)).or(several.first().copied()).ok_or_else(|| "missing `side`".into())
        }
    }
}

/// Open exactly `open` of `side`'s sections.
fn set_open(ui: &mut UiState, side: &str, open: &[String]) {
    let ids = defaults(side);
    let has = |id: &str| open.iter().any(|o| o == id);
    match side {
        DEVELOP_RIGHT => {
            ui.open_sections.retain(|s| !ids.iter().any(|d| d.0 == s.as_str()));
            ui.open_sections.extend(ids.iter().filter(|d| has(d.0)).map(|d| d.0.to_string()));
        }
        DEVELOP_LEFT => ui.develop_left_sections = ids.iter().filter(|d| has(d.0)).map(|d| d.0.to_string()).collect(),
        LIBRARY_RIGHT => ui.library_sections = ids.iter().filter(|d| has(d.0)).map(|d| d.0.to_string()).collect(),
        LIBRARY_LEFT => {
            ui.collapsed_sidebar.retain(|s| !ids.iter().any(|d| d.0 == s.as_str()));
            ui.collapsed_sidebar.extend(ids.iter().filter(|d| !has(d.0)).map(|d| d.0.to_string()));
        }
        _ => {}
    }
}

/// At launch (the UI state read from `ui.json`): each side with sections chosen as Open by
/// Default opens those and closes the rest.
pub fn open_defaults(ui: &mut UiState) {
    let chosen: Vec<(String, Vec<String>)> =
        ui.settings.panel_default_open.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone())).collect();
    for (side, open) in chosen {
        set_open(ui, &side, &open);
    }
}

/// Whether `id` opens at launch on `side`.
pub fn opens_by_default(settings: &AppSettings, side: &str, id: &str) -> bool {
    settings.panel_default_open.get(side).is_some_and(|v| v.iter().any(|s| s == id))
}

/// `panel.order`, `panel.defaultOpen`, `panel.resetOrder`; `None`: not one of them.
pub fn run_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let side = p.get("side").and_then(Value::as_str);
    let r = match id {
        "panel.order" => (|| {
            let side = resolve_side(app, side, None)?;
            if let Some(order) = p.get("order") {
                let ids: Vec<String> = order
                    .as_array()
                    .ok_or("`order` is a list of section ids")?
                    .iter()
                    .map(|v| v.as_str().map(str::to_string).ok_or("`order` is a list of section ids"))
                    .collect::<Result<_, _>>()?;
                if let Some(bad) = ids.iter().find(|i| title(side, i).is_none()) {
                    return Err(format!("`{bad}` isn't a section of {side}"));
                }
                // the sections it doesn't name follow, in the built-in order
                let mut ids = ids;
                for d in defaults(side) {
                    if !ids.iter().any(|i| i == d.0) {
                        ids.push(d.0.to_string());
                    }
                }
                let new: Vec<String> = whole(&ids, &defaults(side)).into_iter().map(str::to_string).collect();
                let builtin: Vec<String> = defaults(side).iter().map(|d| d.0.to_string()).collect();
                if new == builtin {
                    app.ui.settings.panel_order.remove(side);
                } else {
                    app.ui.settings.panel_order.insert(side.to_string(), new);
                }
            }
            Ok(json!({"side": side, "order": ordered(&app.ui.settings, side)}))
        })(),
        "panel.defaultOpen" => (|| {
            let sid = p.get("id").and_then(Value::as_str);
            let side = resolve_side(app, side, sid)?;
            let Some(sid) = sid else {
                return Ok(json!({"side": side, "open": app.ui.settings.panel_default_open.get(side).cloned().unwrap_or_default()}));
            };
            let on = p.get("open").and_then(Value::as_bool).unwrap_or(!opens_by_default(&app.ui.settings, side, sid));
            let list = app.ui.settings.panel_default_open.entry(side.to_string()).or_default();
            list.retain(|s| s != sid);
            if on {
                list.push(sid.to_string());
                // in the panel's order, so ui.json reads like the panel
                let order = ordered(&app.ui.settings, side);
                if let Some(l) = app.ui.settings.panel_default_open.get_mut(side) {
                    l.sort_by_key(|s| order.iter().position(|o| o == s));
                }
            }
            let open = app.ui.settings.panel_default_open.get(side).cloned().unwrap_or_default();
            if open.is_empty() {
                app.ui.settings.panel_default_open.remove(side);
            }
            Ok(json!({"side": side, "id": sid, "open": on, "opensAtLaunch": open}))
        })(),
        "panel.resetOrder" => (|| {
            let sides: Vec<&str> = match side {
                Some(_) => vec![resolve_side(app, side, None)?],
                None => SIDES.to_vec(),
            };
            for s in &sides {
                app.ui.settings.panel_order.remove(*s);
                app.ui.settings.panel_default_open.remove(*s);
            }
            Ok(json!({"reset": sides}))
        })(),
        _ => return None,
    };
    Some(r)
}

/// The UI commands of this module (listed in `menus::UI_COMMANDS`).
pub const COMMANDS: [&str; 3] = ["panel.order", "panel.defaultOpen", "panel.resetOrder"];

// ------------------------------------------------------------------------------ dragging

/// The section being dragged on a side (kept between frames).
#[derive(Clone, Debug)]
struct Drag {
    id: String,
    title: String,
    /// The header widget egui drags.
    widget: egui::Id,
}

fn drag_key(side: &str) -> egui::Id {
    egui::Id::new(("panel-order-drag", side))
}
fn heads_key(side: &str) -> egui::Id {
    egui::Id::new(("panel-order-heads", side))
}

/// A section header is being dragged on `side` (its id).
pub fn dragging(ctx: &egui::Context, side: &str) -> Option<String> {
    ctx.data(|d| d.get_temp::<Drag>(drag_key(side)))
        .filter(|d| ctx.is_being_dragged(d.widget) || ctx.drag_stopped_id() == Some(d.widget))
        .map(|d| d.id)
}

/// Before the side's sections are laid out this frame (a drag left over from when the panel was
/// hidden is dropped, never applied).
pub fn begin(ui: &egui::Ui, side: &str) {
    if dragging(ui.ctx(), side).is_none() {
        ui.data_mut(|d| d.remove::<Drag>(drag_key(side)));
    }
    ui.data_mut(|d| d.insert_temp(heads_key(side), Vec::<(String, Rect)>::new()));
}

/// A section header was laid out (`resp` senses drags): remember where it is, show the grip
/// while hovered, and start a drag. `grip_x`: where the grip goes, from the header's left edge
/// (positive) or from its visible right edge (negative) — wherever the header has room.
pub fn header(ui: &egui::Ui, side: &str, id: &str, resp: &Response, grip_x: f32) {
    let r = resp.rect;
    ui.data_mut(|d| d.get_temp_mut_or_default::<Vec<(String, Rect)>>(heads_key(side)).push((id.to_string(), r)));
    let t = Tokens::get(ui.ctx());
    let dragged = dragging(ui.ctx(), side);
    if dragged.as_deref() == Some(id) {
        // the section being moved: tinted, so it's clear what moves
        ui.painter().rect_filled(r, 0.0, t.accent.gamma_multiply(0.18));
    }
    if resp.hovered() && dragged.is_none() || dragged.as_deref() == Some(id) {
        let visible_right = r.right().min(ui.clip_rect().right());
        let c = if grip_x >= 0.0 { pos2(r.left() + grip_x, r.center().y) } else { pos2(visible_right + grip_x, r.center().y) };
        let grip = Rect::from_center_size(c, vec2(14.0, r.height()));
        for dx in [-2.5f32, 2.5] {
            for dy in [-4.0f32, 0.0, 4.0] {
                ui.painter().circle_filled(c + vec2(dx, dy), 1.2, t.text_dim);
            }
        }
        if ui.rect_contains_pointer(grip) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }
    if resp.drag_started() {
        let title = title(side, id).map(crate::i18n::tr).unwrap_or(id).to_string();
        ui.data_mut(|d| d.insert_temp(drag_key(side), Drag { id: id.to_string(), title, widget: resp.id }));
    }
}

/// Where a drop at `y` lands: the index among `heads` (by header middles).
fn drop_index(heads: &[(String, Rect)], y: f32) -> usize {
    heads.iter().position(|(_, r)| y < r.center().y).unwrap_or(heads.len())
}

/// `order` with `id` moved to land before `heads[at]` (or last).
fn moved(order: &[&str], heads: &[(String, Rect)], id: &str, at: usize) -> Vec<String> {
    // (dropped just above or below itself: the next other header)
    let before = heads.get(at..).and_then(|rest| rest.iter().find(|h| h.0 != id)).map(|h| h.0.clone());
    let mut out: Vec<String> = order.iter().filter(|o| **o != id).map(|o| o.to_string()).collect();
    let pos = before.and_then(|b| out.iter().position(|o| *o == b)).unwrap_or(out.len());
    out.insert(pos, id.to_string());
    out
}

/// After the side's sections: while a header is dragged, the drop line, the moving title and
/// the scroll at the panel's edges; on release, the new order (`panel.order`).
pub fn finish(app: &mut LightcraftApp, ui: &mut egui::Ui, side: &str) {
    let Some(drag) = ui.data(|d| d.get_temp::<Drag>(drag_key(side))) else { return };
    let heads: Vec<(String, Rect)> = ui.data(|d| d.get_temp(heads_key(side))).unwrap_or_default();
    let (pointer, escape) = ui.input(|i| (i.pointer.interact_pos().or(i.pointer.hover_pos()), i.key_pressed(egui::Key::Escape)));
    // egui's own drag of the header: still on, just let go, or gone (the panel was hidden
    // mid-drag, Esc): only a release drops
    let ctx = ui.ctx().clone();
    let released = ctx.drag_stopped_id() == Some(drag.widget);
    let on = ctx.is_being_dragged(drag.widget);
    if escape || heads.is_empty() || !(on || released) {
        ui.data_mut(|d| d.remove::<Drag>(drag_key(side)));
        if on {
            ctx.stop_dragging();
        }
        return;
    }
    let Some(pointer) = pointer else { return };
    let at = drop_index(&heads, pointer.y);
    if released {
        ui.data_mut(|d| d.remove::<Drag>(drag_key(side)));
        let order = ordered(&app.ui.settings, side);
        let new = moved(&order, &heads, &drag.id, at);
        if new.iter().map(String::as_str).ne(order.iter().copied())
            && let Err(e) = app.run("panel.order", json!({"side": side, "order": new}))
        {
            app.toast(ui.ctx(), e);
        }
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    // the drop line: above the header it lands before, or under the last section
    let y = heads.get(at).map_or(ui.cursor().top(), |h| h.1.top());
    let x = heads.first().map_or(ui.max_rect().x_range(), |h| h.1.x_range());
    let right = x.max.min(ui.clip_rect().right());
    ui.painter().line_segment([pos2(x.min + 8.0, y), pos2(right - 8.0, y)], Stroke::new(2.0, t.accent));
    ui.painter().circle_filled(pos2(x.min + 8.0, y), 3.0, t.accent);
    // what moves, by the pointer
    // (on the side of the pointer with room: the left of it over a right-hand panel; one line)
    let right_half = pointer.x > ctx.content_rect().center().x;
    let (pivot, at) =
        if right_half { (egui::Align2::RIGHT_TOP, pointer + vec2(-14.0, 8.0)) } else { (egui::Align2::LEFT_TOP, pointer + vec2(14.0, 8.0)) };
    egui::Area::new(egui::Id::new(("panel-order-label", side))).order(egui::Order::Tooltip).interactable(false).pivot(pivot).fixed_pos(at).show(
        ui.ctx(),
        |ui| {
            egui::Frame::NONE.fill(t.accent).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 3)).show(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&drag.title).font(t.semibold(12.0)).color(Color32::WHITE)).extend());
            });
        },
    );
    // near the panel's top or bottom edge: scroll toward it
    let clip = ui.clip_rect();
    let edge = 28.0;
    let dy = if pointer.y < clip.top() + edge {
        6.0
    } else if pointer.y > clip.bottom() - edge {
        -6.0
    } else {
        0.0
    };
    if dy != 0.0 {
        ui.scroll_with_delta(vec2(0.0, dy));
    }
    ui.ctx().request_repaint();
}

/// The header's right-click menu entries: Open by Default and Reset Order.
pub fn menu_items(app: &mut LightcraftApp, ui: &mut egui::Ui, side: &str, id: &str) {
    ui.separator();
    let mut on = opens_by_default(&app.ui.settings, side, id);
    let r = ui.checkbox(&mut on, crate::i18n::tr("Open by Default")).on_hover_text(crate::i18n::tr("Open this panel when LightCraft starts"));
    crate::widgets::register(ui.ctx(), format!("menu:defaultOpen-{id}"), r.rect);
    if r.changed() {
        let _ = app.run("panel.defaultOpen", json!({"side": side, "id": id, "open": on}));
    }
    let custom = app.ui.settings.panel_order.contains_key(side) || app.ui.settings.panel_default_open.contains_key(side);
    let r = ui.button(crate::i18n::tr("Reset Order")).on_hover_text(crate::i18n::tr("The built-in order, and open what was open at quit"));
    crate::widgets::register(ui.ctx(), format!("menu:resetOrder-{side}"), r.rect);
    if r.clicked() {
        if custom {
            let _ = app.run("panel.resetOrder", json!({"side": side}));
        }
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: [(&str, &str); 4] = [("a", "A"), ("b", "B"), ("c", "C"), ("d", "D")];

    #[test]
    fn saved_orders_are_made_whole() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(whole(&s(&["c", "a", "b", "d"]), &D), ["c", "a", "b", "d"]);
        // unknown and repeated ids dropped; a missing one goes after its built-in neighbour
        assert_eq!(whole(&s(&["d", "zz", "b", "d", "a"]), &D), ["d", "b", "c", "a"]);
        assert_eq!(whole(&s(&["c"]), &D), ["a", "b", "c", "d"]);
        assert_eq!(whole(&[], &D), ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_drop_lands_between_header_middles() {
        let r = |y: f32| Rect::from_min_size(pos2(0.0, y), vec2(100.0, 30.0));
        let heads = vec![("a".to_string(), r(0.0)), ("b".to_string(), r(100.0)), ("c".to_string(), r(400.0))];
        assert_eq!(drop_index(&heads, 5.0), 0);
        assert_eq!(drop_index(&heads, 200.0), 2);
        assert_eq!(drop_index(&heads, 900.0), 3);
        let order = ["a", "b", "c"];
        assert_eq!(moved(&order, &heads, "a", 3), ["b", "c", "a"]);
        assert_eq!(moved(&order, &heads, "c", 0), ["c", "a", "b"]);
        assert_eq!(moved(&order, &heads, "a", 2), ["b", "a", "c"]);
        assert_eq!(moved(&order, &heads, "b", 1), ["a", "b", "c"]);
    }
}
