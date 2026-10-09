//! Lightroom Classic's filmstrip: a strip of the current source's photos along the bottom of the
//! window in both modules. Its header shows where the photos come from ("Folder: GR3 · 208 photos /
//! 1 selected") with the grid and back / forward buttons on the left and a quick flag / rating
//! filter on the right. Hidden with F6 (or `/`), resized by dragging its top edge; the active photo
//! is kept in view, and the "left off." marker shows on its thumbnail.

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use lightcraft_catalog::Flag;
use lightcraft_engine::LibrarySource;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::state::FILM_HEIGHT;
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// Cell colours (Classic: the active photo lightest, the rest of the selection lighter than the
/// others).
pub const ACTIVE_CELL: Color32 = Color32::from_gray(0x6a);
pub const SELECTED_CELL: Color32 = Color32::from_gray(0x50);
pub const HOVER_CELL: Color32 = Color32::from_gray(0x3c);
pub const CELL: Color32 = Color32::from_gray(0x30);

/// Height of the header row above the thumbnails.
pub const HEADER_H: f32 = 26.0;
/// Height of the bar left when the strip is hidden (its triangle brings it back).
pub const HIDDEN_H: f32 = 12.0;

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if !app.ui.filmstrip {
        // Classic's panel triangle: a thin bar at the bottom edge
        egui::Panel::bottom("filmstrip_hidden").exact_size(HIDDEN_H).frame(egui::Frame::NONE.fill(t.header)).show(ui, |ui| {
            let r = ui.max_rect();
            let resp = ui.interact(r, egui::Id::new("film-show"), Sense::click()).on_hover_text(crate::i18n::tr("Show Filmstrip (F6)"));
            register(ui.ctx(), "button:filmstripShow", r);
            let tri = Rect::from_center_size(r.center(), vec2(10.0, 10.0));
            ui.painter().text(tri.center(), Align2::CENTER_CENTER, "▲", t.font(8.0), if resp.hovered() { t.text } else { t.text_dim });
            if resp.clicked() {
                let _ = app.run("view.filmstrip", json!({}));
            }
        });
        return;
    }
    let h = FILM_HEIGHT.clamp(app.ui.film_height);
    let resp = egui::Panel::bottom("filmstrip").exact_size(h + HEADER_H).resizable(false).frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
        let full = ui.max_rect();
        let head = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        header(app, ui, head);
        cells(app, ui, Rect::from_min_max(pos2(full.left(), head.bottom() + 4.0), full.max));
    });
    let r = resp.response.rect;
    register(ui.ctx(), "panel:filmstrip", r);
    // the top edge resizes the strip (generous grab zone; Lightroom Classic drags it too)
    let edge = Rect::from_min_max(pos2(r.left(), r.top() - 4.0), pos2(r.right(), r.top() + 4.0));
    let er = ui.interact(edge, egui::Id::new("film-resize"), Sense::drag());
    register(ui.ctx(), "edge:filmstrip", edge);
    if er.hovered() || er.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    if er.dragged()
        && let Some(y) = ui.ctx().pointer_interact_pos().map(|p| p.y)
    {
        app.ui.film_height = FILM_HEIGHT.clamp(r.bottom() - y - HEADER_H);
    }
    if er.double_clicked() {
        app.ui.film_height = FILM_HEIGHT.default;
    }
}

/// "Folder: GR3", "Collection: Portfolio", "All Photographs"… — the source as Classic names it.
pub fn source_breadcrumb(app: &LightcraftApp) -> String {
    let title = crate::i18n::source_title(&app.session);
    match app.session.source {
        LibrarySource::LibraryFolder | LibrarySource::Folder => crate::i18n::tr_format!("Folder: {title}", title = title),
        LibrarySource::Album(_) => crate::i18n::tr_format!("Collection: {title}", title = title),
        _ => title,
    }
}

/// The text of the breadcrumb after the source: "208 photos / 1 selected / IMG_0001.DNG".
pub fn counts_text(total: usize, selected: usize, active_name: Option<&str>) -> String {
    let mut s = crate::i18n::tr_format!("{total} photo{}", if total == 1 { "" } else { "s" }, total = total);
    if selected > 0 {
        s.push_str(&crate::i18n::tr_format!(" / {selected} selected", selected = selected));
    }
    if let Some(n) = active_name {
        s.push_str(" / ");
        s.push_str(n);
    }
    s
}

fn header(app: &mut LightcraftApp, ui: &mut egui::Ui, head: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(head, 0.0, t.header);
    let mut row =
        ui.new_child(egui::UiBuilder::new().max_rect(head.shrink2(vec2(8.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    let sz = vec2(22.0, 22.0);
    if icon_button(&mut row, "filmGrid", Icon::GridSquare, sz, false, true, "Grid (G)").clicked() {
        let _ = app.run("view.library", json!({"view": "photoGrid"}));
    }
    if icon_button(&mut row, "filmPrev", Icon::ChevronLeft, sz, false, true, "Previous Photo (←)").clicked() {
        let _ = app.run("library.previous", json!({}));
    }
    if icon_button(&mut row, "filmNext", Icon::ChevronRight, sz, false, true, "Next Photo (→)").clicked() {
        let _ = app.run("library.next", json!({}));
    }
    // right: the quick filter (Classic's "Filter:" flags and rating)
    let filter_w = 236.0;
    let crumb_right = head.right() - filter_w - 16.0;
    let total = app.session.visible().len();
    let selected = app.session.selection.ids.len();
    let active_name = app.session.active().and_then(|id| app.session.catalog.photo(id)).map(|p| p.file_name.clone());
    let source = source_breadcrumb(app);
    let counts = counts_text(total, selected, active_name.as_deref());
    let x0 = row.cursor().left() + 4.0;
    let room = (crumb_right - x0).max(0.0);
    let g1 = row.painter().layout_no_wrap(source.clone(), t.semibold(11.5), t.text_label);
    let src_w = g1.size().x.min(room);
    let clip = Rect::from_min_max(pos2(x0, head.top()), pos2(crumb_right, head.bottom()));
    let p = row.painter().with_clip_rect(clip);
    p.galley(pos2(x0, head.center().y - g1.size().y / 2.0), g1, t.text_label);
    let g2 = p.layout_no_wrap(counts.clone(), t.font(11.5), t.text_dim);
    p.galley(pos2(x0 + src_w + 10.0, head.center().y - g2.size().y / 2.0), g2, t.text_dim);
    let crumb = Rect::from_min_max(pos2(x0, head.top()), pos2((x0 + src_w + 10.0 + 200.0).min(crumb_right), head.bottom()));
    register(ui.ctx(), "label:filmSource", crumb);
    ui.interact(crumb, egui::Id::new("film-crumb"), Sense::hover()).on_hover_text(format!("{source} · {counts}"));
    filter(app, ui, Rect::from_min_max(pos2(head.right() - filter_w - 8.0, head.top()), pos2(head.right() - 8.0, head.bottom())));
}

/// The filmstrip's quick filter: flag buttons (pick / unflagged / reject) and a minimum rating.
fn filter(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::right_to_left(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 2.0;
    let f = &app.session.filter;
    let (flag, rating) = (f.flag, f.rating);
    let any = flag.is_some() || rating > 0;
    // a click on the active choice clears it
    let off = crate::widgets::text_button(&mut row, "filmFilterOff", if any { "filters off." } else { "no filter." }, false);
    if off.on_hover_text(crate::i18n::tr("Show every photo of the source")).clicked() && any {
        let _ = app.run("library.filter", json!({"flag": Value::Null, "rating": 0}));
    }
    row.add_space(6.0);
    for n in (1..=5u8).rev() {
        let (sr, resp) = row.allocate_exact_size(vec2(14.0, 20.0), Sense::click());
        register(row.ctx(), format!("button:filmRating-{n}"), sr);
        let on = rating >= n;
        let c = if on {
            t.star
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_disabled
        };
        paint(row.painter(), Rect::from_center_size(sr.center(), vec2(11.0, 11.0)), if on { Icon::StarFilled } else { Icon::Star }, c);
        let tip = crate::i18n::tr_format!("Rated {n} star{} or more", if n == 1 { "" } else { "s" }, n = n);
        if resp.on_hover_text(tip).clicked() {
            let v = if rating == n { 0 } else { n };
            let _ = app.run("library.filter", json!({"rating": v, "ratingOp": "atLeast"}));
        }
    }
    row.add_space(8.0);
    for (key, icon, fl, tip) in [
        ("reject", Icon::FlagReject, Flag::Reject, "Rejected photos"),
        ("none", Icon::Circle, Flag::None, "Unflagged photos"),
        ("pick", Icon::FlagPick, Flag::Pick, "Picked photos"),
    ] {
        let (fr, resp) = row.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
        register(row.ctx(), format!("button:filmFlag-{key}"), fr);
        let on = flag == Some(fl);
        if on {
            row.painter().rect_filled(fr, 3.0, t.tool_active);
        }
        let c = match (on, fl) {
            (true, Flag::Reject) => t.reject,
            (true, _) => t.pick,
            _ if resp.hovered() => t.text,
            _ => t.icon,
        };
        paint(row.painter(), fr.shrink(4.0), icon, c);
        if resp.on_hover_text(crate::i18n::tr(tip)).clicked() {
            let v = if on { Value::Null } else { json!(key) };
            let _ = app.run("library.filter", json!({"flag": v}));
        }
    }
    row.add_space(4.0);
    row.label(egui::RichText::new(crate::i18n::tr("Filter:")).font(t.font(11.5)).color(t.text_dim));
}

/// A filmstrip cell's file-name label: names longer than 14 characters are cut to 13 and an
/// ellipsis. Counts characters, not bytes, so a CJK name is never cut inside a character (#266).
fn film_label(name: &str) -> String {
    if name.chars().nth(14).is_some() { format!("{}…", name.chars().take(13).collect::<String>()) } else { name.to_string() }
}

fn cells(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, t.canvas);
    let ids = app.session.visible_cloned();
    let active = app.session.selection.active;
    // square cells as tall as the strip (Lightroom Classic's), a little wider for landscape thumbs
    let cell_w = (r.height() * 1.18).clamp(64.0, 300.0);
    let ppp = ui.ctx().pixels_per_point();
    if ids.is_empty() {
        let why = if app.session.filter != Default::default() { "No photos match the filters (View → Clear Filters)" } else { "No photos" };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, why, t.font(12.5), t.text_dim);
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    // a vertical mouse wheel scrolls the strip sideways (horizontal trackpad scrolls still work)
    child.style_mut().always_scroll_the_only_direction = true;
    // only a new active photo (or the strip appearing) scrolls it; see `grid::follow_active`
    let follow = super::grid::follow_active(child.ctx(), egui::Id::new("film-follow-active"), active);
    egui::ScrollArea::horizontal().id_salt("filmstrip").auto_shrink([false, false]).show_viewport(&mut child, |ui, vp| {
        let (area, _) = ui.allocate_exact_size(vec2(ids.len() as f32 * cell_w, r.height() - 12.0), Sense::hover());
        app.film_scroll = Some(vp.left());
        for (i, id) in ids.iter().enumerate() {
            let cr = Rect::from_min_size(pos2(area.left() + i as f32 * cell_w, area.top()), vec2(cell_w, area.height())).shrink2(vec2(2.0, 0.0));
            let local = Rect::from_min_size(pos2(i as f32 * cell_w, 0.0), cr.size());
            // centred when it is (partly) off screen; a click on a visible thumbnail leaves the strip alone
            if follow && Some(*id) == active && !(local.left() >= vp.left() && local.right() <= vp.right()) {
                ui.scroll_to_rect(cr, Some(egui::Align::Center));
            }
            if !local.intersects(vp.expand2(vec2(cell_w * 4.0, 0.0))) {
                continue;
            }
            let resp = ui.interact(cr, egui::Id::new(("film", id.0)), Sense::click());
            register(ui.ctx(), format!("film:{}", id.0), cr);
            let state = app.session.selection.state_of(*id);
            let sel = state == lightcraft_engine::SelectionState::Active;
            let p = ui.painter();
            let label = app.session.catalog.photo(*id).and_then(|ph| ph.label);
            let selected = state != lightcraft_engine::SelectionState::NotSelected;
            // Classic's cells: dark, the selection lighter, the active photo lightest
            let base = if sel {
                ACTIVE_CELL
            } else if selected {
                SELECTED_CELL
            } else if resp.hovered() {
                HOVER_CELL
            } else {
                CELL
            };
            p.rect_filled(cr, 2.0, crate::theme::label_background(base, label, selected));
            // names only when the strip is tall enough to keep a usable thumbnail under them
            let names = app.ui.settings.film_names && cr.height() >= 96.0;
            if let Some(ph) = app.session.catalog.photo(*id).filter(|_| names) {
                let name = ph.file_name.rsplit_once('.').map(|(n, _)| n).unwrap_or(&ph.file_name);
                let c = if selected { t.text } else { t.text_dim };
                p.text(pos2(cr.left() + 6.0, cr.top() + 9.0), Align2::LEFT_CENTER, film_label(name), t.font(10.0), c);
                p.text(pos2(cr.right() - 6.0, cr.top() + 9.0), Align2::RIGHT_CENTER, &ph.format, t.semibold(8.5), c);
            }
            let top = if names { 18.0 } else { 6.0 };
            let img_area = Rect::from_min_max(cr.min + vec2(6.0, top), cr.max - vec2(6.0, 6.0));
            super::grid::request_thumb(app, *id, (256.0 * ppp.min(2.0) / 2.0) as usize * 2, 8);
            if let Some(tex) = app.renderer.thumb(*id) {
                let [tw, th] = tex.size;
                let s = (img_area.width() / tw as f32).min(img_area.height() / th as f32);
                let fr = Rect::from_center_size(img_area.center(), vec2(tw as f32 * s, th as f32 * s));
                p.image(tex.tex.id(), fr, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if sel {
                    p.rect_stroke(fr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
                } else if state == lightcraft_engine::SelectionState::Selected {
                    p.rect_stroke(fr, 0.0, Stroke::new(1.5, Color32::from_gray(170)), StrokeKind::Outside);
                }
                if app.left_off.marker == Some(*id) {
                    crate::leftoff::paint_marker(p, &t, fr, true);
                }
                if app.ui.settings.film_badges
                    && let Some(ph) = app.session.catalog.photo(*id)
                {
                    film_badges(p, &t, fr, ph);
                }
                if let Some(st) = app.session.catalog.stack_of(*id) {
                    let text =
                        if st.collapsed { st.photos.len().to_string() } else { format!("{}/{}", st.position(*id).unwrap_or(0) + 1, st.photos.len()) };
                    let g = p.layout_no_wrap(text, t.semibold(9.5), Color32::WHITE);
                    let br = Rect::from_min_size(fr.min + vec2(3.0, 3.0), vec2(g.size().x + 22.0, 15.0));
                    p.rect_filled(br, 7.5, Color32::from_black_alpha(170));
                    crate::icons::paint(p, Rect::from_min_size(br.min + vec2(4.0, 2.0), vec2(11.0, 11.0)), crate::icons::Icon::Stack, Color32::WHITE);
                    p.galley(pos2(br.min.x + 17.0, br.center().y - g.size().y / 2.0), g, Color32::WHITE);
                }
            }
            if resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                let mode = if m.command {
                    "toggle"
                } else if m.shift {
                    "range"
                } else {
                    "replace"
                };
                let _ = app.run("library.select", json!({"ids": [id.0], "mode": mode}));
            }
            // the same photo actions as the grid and loupe (Restore / Delete Permanently in Recently Deleted)
            resp.context_menu(|ui| super::grid::context_menu(app, ui, *id));
        }
    });
}

/// Rating, flag and edited badges along a filmstrip thumbnail's bottom edge (Settings → Interface).
fn film_badges(p: &egui::Painter, t: &Tokens, fr: Rect, ph: &lightcraft_catalog::Photo) {
    use crate::icons::{Icon, paint};
    use lightcraft_catalog::Flag;
    let edited = ph.is_edited();
    if ph.rating == 0 && ph.flag == Flag::None && !edited {
        return;
    }
    let bar = Rect::from_min_max(pos2(fr.left(), fr.bottom() - 15.0), fr.right_bottom());
    p.rect_filled(bar, 0.0, Color32::from_black_alpha(130));
    let y = bar.center().y;
    let mut x = bar.left() + 3.0;
    for i in 0..ph.rating {
        paint(p, Rect::from_min_size(pos2(x + i as f32 * 9.0, y - 4.0), vec2(8.0, 8.0)), Icon::StarFilled, t.star);
    }
    x += ph.rating as f32 * 9.0 + 2.0;
    match ph.flag {
        Flag::Pick => paint(p, Rect::from_min_size(pos2(x, y - 5.0), vec2(10.0, 10.0)), Icon::FlagPick, t.pick),
        Flag::Reject => paint(p, Rect::from_min_size(pos2(x, y - 5.0), vec2(10.0, 10.0)), Icon::FlagReject, t.reject),
        Flag::None => {}
    }
    if edited {
        paint(p, Rect::from_min_size(pos2(bar.right() - 13.0, y - 5.0), vec2(10.0, 10.0)), Icon::Sliders, t.text_label);
    }
}

#[cfg(test)]
mod tests {
    use super::film_label;

    #[test]
    fn film_labels_cut_on_characters_not_bytes() {
        // the name from #266: byte 13 falls inside '限'
        assert_eq!(film_label("202407層三限定訂閱圖(4)"), "202407層三限定訂閱圖…");
        assert_eq!(film_label("写真"), "写真");
        // exactly 14 characters (42 bytes) is shown whole
        assert_eq!(film_label("一二三四五六七八九十一二三四"), "一二三四五六七八九十一二三四");
        assert_eq!(film_label("IMG_20240712_153012"), "IMG_20240712_…");
        assert_eq!(film_label("DSC_0001"), "DSC_0001");
        assert_eq!(film_label(""), "");
    }
}
