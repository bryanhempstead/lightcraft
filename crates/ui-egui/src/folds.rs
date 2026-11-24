//! Foldable lists and sections (Bryan, 2026-11-23: "Every list or section of items in his apps
//! must be collapsible (tap the header to fold / unfold, remembered)").
//!
//! One map, [`crate::state::UiState::folds`] (section id → open, saved in ui.json), holds every
//! fold. Drawing code only has a `Ui`, so the map is mirrored into the egui context each frame
//! ([`sync`], at the top of `LightcraftApp::ui`): a header click is queued there and applied to
//! the UI state on the next frame, and commands (`ui.fold`, `ui.folds`) change the UI state
//! directly. Sections are open until folded, so nothing looks different until Bryan folds one.
//!
//! Section ids are stable, dotted and lowercase: `settings.general.library`,
//! `metadata.exif`, `grid.group:2024-05-01`, …. Every header registers `fold:{id}` for the
//! control channel (`ui.clickWidget`).

use std::collections::BTreeMap;

use egui::{Align2, Rect, Response, Sense, Ui, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::register;

#[derive(Clone, Default)]
struct Live {
    map: BTreeMap<String, bool>,
    /// Header clicks this frame, applied to the UI state by the next [`sync`].
    queued: Vec<(String, bool)>,
}

fn data_id() -> egui::Id {
    egui::Id::new("lightcraft.folds")
}

/// Applies last frame's header clicks to `folds` (the UI state's map) and mirrors it into the
/// context for this frame's drawing. Returns whether anything changed.
pub fn sync(ctx: &egui::Context, folds: &mut BTreeMap<String, bool>) -> bool {
    let live: Live = ctx.data(|d| d.get_temp(data_id())).unwrap_or_default();
    let changed = !live.queued.is_empty();
    for (id, open) in live.queued {
        set_in(folds, &id, open);
    }
    ctx.data_mut(|d| d.insert_temp(data_id(), Live { map: folds.clone(), queued: Vec::new() }));
    changed
}

/// Records `open` for `id`.
pub fn set_in(folds: &mut BTreeMap<String, bool>, id: &str, open: bool) {
    folds.insert(id.to_string(), open);
}

/// Whether section `id` is open this frame (never folded: open).
pub fn is_open(ctx: &egui::Context, id: &str) -> bool {
    is_open_or(ctx, id, true)
}

/// Whether section `id` is open this frame; `default` until it is first folded or unfolded (a
/// few export sections start closed, as in Lightroom Classic).
pub fn is_open_or(ctx: &egui::Context, id: &str, default: bool) -> bool {
    ctx.data(|d| d.get_temp::<Live>(data_id()).and_then(|l| l.map.get(id).copied())).unwrap_or(default)
}

/// Opens or closes `id` now (this frame) and on the UI state from the next frame on.
pub fn set(ctx: &egui::Context, id: &str, open: bool) {
    ctx.data_mut(|d| {
        let live = d.get_temp_mut_or_default::<Live>(data_id());
        set_in(&mut live.map, id, open);
        live.queued.push((id.to_string(), open));
    });
    ctx.request_repaint();
}

/// Folds `id` when `resp` (a header the caller drew) was clicked; returns whether it is open now.
pub fn toggle_on_click(ui: &Ui, resp: &Response, id: &str) -> bool {
    toggle_on_click_or(ui, resp, id, true)
}

/// [`toggle_on_click`] for a section that starts `default` (open or closed).
pub fn toggle_on_click_or(ui: &Ui, resp: &Response, id: &str, default: bool) -> bool {
    let open = is_open_or(ui.ctx(), id, default);
    if resp.clicked() {
        set(ui.ctx(), id, !open);
        return !open;
    }
    open
}

/// The ▾ / ▸ disclosure triangle, centered at `c`.
pub fn paint_triangle(ui: &Ui, c: egui::Pos2, open: bool, color: egui::Color32) {
    paint(ui.painter(), Rect::from_center_size(c, vec2(12.0, 12.0)), if open { Icon::ChevronDown } else { Icon::ChevronRight }, color);
}

/// A foldable header: a ▾ / ▸ triangle and `title` (semibold, `size` pt) over the full width,
/// `h` points high. Click anywhere on it to fold or unfold. Returns whether the section is open.
pub fn header_sized(ui: &mut Ui, id: &str, title: &str, size: f32, h: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let title = crate::i18n::tr(title);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(40.0), h), Sense::click());
    let open = toggle_on_click(ui, &resp, id);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    register(ui.ctx(), format!("fold:{id}"), r);
    let hover = ui.rect_contains_pointer(r);
    if hover {
        ui.painter().rect_filled(r, 3.0, t.hover);
    }
    paint_triangle(ui, pos2(r.left() + 8.0, r.center().y), open, if hover { t.text } else { t.text_dim });
    let clip = Rect::from_min_max(pos2(r.left() + 20.0, r.top()), r.right_bottom());
    ui.painter().with_clip_rect(clip).text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, title, t.semibold(size), t.text);
    open
}

/// [`header_sized`] at the size of a dialog's group heading (12.5 pt, 24 points high).
pub fn header(ui: &mut Ui, id: &str, title: &str) -> bool {
    header_sized(ui, id, title, 12.5, 24.0)
}

/// `ui.fold {id, open?}`: opens or closes a section (no `open`: toggles it). `ui.folds`: every
/// folded section and the foldable sections on screen.
pub fn run_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "ui.fold" => {
            let Some(section) = p.get("id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) else {
                return Some(Err("ui.fold needs `id`, the section to fold (see ui.folds)".into()));
            };
            let open = match p.get("open") {
                None | Some(Value::Null) => !app.ui.folds.get(section).copied().unwrap_or(true),
                Some(v) => match v.as_bool() {
                    Some(b) => b,
                    None => return Some(Err("`open` must be true or false".into())),
                },
            };
            set_in(&mut app.ui.folds, section, open);
            Some(Ok(json!({"id": section, "open": open})))
        }
        "ui.folds" => {
            let mut on_screen: Vec<&str> = app.widgets.iter().filter_map(|(w, _)| w.strip_prefix("fold:")).collect();
            on_screen.sort_unstable();
            on_screen.dedup();
            let shown: Vec<Value> = on_screen.iter().map(|s| json!({"id": s, "open": app.ui.folds.get(*s).copied().unwrap_or(true)})).collect();
            let folded: Vec<&String> = app.ui.folds.iter().filter(|(_, open)| !**open).map(|(k, _)| k).collect();
            Some(Ok(json!({"folded": folded, "saved": app.ui.folds, "onScreen": shown})))
        }
        _ => None,
    }
}
