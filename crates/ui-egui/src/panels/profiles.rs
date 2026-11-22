//! The profile browser (the profile menu's "Browse…", or the grid button next to it): every
//! profile as a live thumbnail of the current photo, grouped (Favorites first, then ours, Adobe
//! Raw, the photo's camera profiles — Camera Matching, Legacy, imported DCPs —, Adaptive, our
//! creative groups and imported ones), with a favourite star, the applied profile's amount, a
//! temporary loupe preview while hovering and apply on click. Lightroom's extras: a color / B&W
//! filter, grid / large / list views, Manage Profiles (show or hide groups) and Import Profiles.

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::DevelopSettings;
use lightcraft_engine::presets::profile_is_bw;
use serde_json::json;

use super::presets::cover_uv;
use crate::icons::{Icon, paint};
use crate::state::RightPanel;
use crate::theme::Tokens;
use crate::widgets::{divider, icon_button, register, segmented, text_button};
use crate::{HoverPreview, LightcraftApp};

/// Long edge of the variant thumbnails (px).
const THUMB_EDGE: usize = 256;
/// Left / right margin of the browser's rows.
const MARGIN_L: f32 = 22.0;
const MARGIN_R: f32 = 22.0;

/// The color / B&W filter (Lightroom's "Filter: All / Color / B&W"), kept per window.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Color,
    Bw,
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    let d = (*app.session.develop_of(id).unwrap_or_default()).clone();
    // header: back + title
    let (hr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
    let mut back = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(hr.min + vec2(12.0, 9.0), vec2(28.0, 28.0))));
    if icon_button(&mut back, "profilesBack", Icon::ChevronLeft, vec2(28.0, 28.0), false, true, "Back to Edit").clicked() {
        app.ui.right = RightPanel::Edit;
    }
    ui.painter().text(pos2(hr.left() + 46.0, hr.center().y + 2.0), Align2::LEFT_CENTER, crate::i18n::tr("Profiles"), t.semibold(15.0), t.text);
    divider(ui);
    // the applied profile and its amount
    let (cr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::hover());
    let name = app.session.profile_info(&d.profile.id).map(|(name, _)| name).unwrap_or("Color");
    ui.painter().text(pos2(cr.left() + 24.0, cr.center().y), Align2::LEFT_CENTER, crate::i18n::tr("Profile"), t.font(13.0), t.text_dim);
    ui.painter().text(
        pos2(cr.left() + 84.0, cr.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::profile_label(&d.profile.id, name),
        t.font(14.0),
        t.text_label,
    );
    if lightcraft_engine::presets::profile_has_amount(&d.profile.id) {
        super::edit::control(app, ui, &d, "profile.amount", true);
        ui.add_space(6.0);
    }
    divider(ui);
    // filter, view, Manage Profiles, Import
    let filter_id = egui::Id::new("profile-filter");
    let manage_id = egui::Id::new("profile-manage");
    let mut filter: Filter = ui.data(|m| m.get_temp(filter_id)).unwrap_or_default();
    let mut manage: bool = ui.data(|m| m.get_temp(manage_id)).unwrap_or(false);
    let view = match app.session.profile_view.as_str() {
        "large" => 1,
        "list" => 2,
        _ => 0,
    };
    ui.add_space(12.0);
    toolbar_row(ui, |ui| {
        let i = match filter {
            Filter::All => 0,
            Filter::Color => 1,
            Filter::Bw => 2,
        };
        if let Some(k) = segmented(ui, "profileFilter", &[("all.", "all"), ("clr.", "color"), ("b&w.", "bw")], Some(i), 3) {
            filter = [Filter::All, Filter::Color, Filter::Bw].get(k).copied().unwrap_or_default();
            ui.data_mut(|m| m.insert_temp(filter_id, filter));
        }
    });
    ui.add_space(12.0);
    toolbar_row(ui, |ui| {
        if let Some(k) = segmented(ui, "profileView", &[("grid.", "grid"), ("lrg.", "large"), ("lst.", "list")], Some(view), 3) {
            let v = ["grid", "large", "list"].get(k).copied().unwrap_or("grid");
            let _ = app.run("profile.view", json!({"view": v}));
        }
    });
    ui.add_space(12.0);
    toolbar_row(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if text_button(ui, "profileManage", "mng.", manage).on_hover_text(crate::i18n::tr("Manage Profiles: show or hide groups")).clicked() {
                manage = !manage;
                ui.data_mut(|m| m.insert_temp(manage_id, manage));
            }
            if text_button(ui, "profileImport", "imp.", false).on_hover_text(crate::i18n::tr("Import Profiles (.xmp, .dcp, .cube, .zip)")).clicked() {
                let _ = app.run("file.importPresets", json!({}));
            }
            if manage
                && !app.session.profile_hidden_groups.is_empty()
                && text_button(ui, "profileReset", "rst.", false).on_hover_text(crate::i18n::tr("Show every group again")).clicked()
            {
                let _ = app.run("profile.groups.reset", json!({}));
            }
        });
    });
    ui.add_space(12.0);
    divider(ui);
    if manage {
        manage_ui(app, ui, id);
        return;
    }
    // (title, cell tag, imported LUT folder, profiles: (id, name, group))
    let mut sections: Vec<(String, &str, bool, Vec<(String, String, String)>)> = Vec::new();
    let favs: Vec<(String, String, String)> = app
        .session
        .profile_favorites
        .iter()
        .filter_map(|f| app.session.profile_info(f).map(|(name, group)| (f.clone(), name.to_string(), group.to_string())))
        .collect();
    if !favs.is_empty() {
        sections.push(("Favorites".into(), "fav", false, favs));
    }
    for g in app.session.profile_groups_for(Some(id), false) {
        let list = g.profiles.into_iter().map(|p| (p.id, p.name, p.group)).collect();
        sections.push((g.name, "", g.imported, list));
    }
    for (title, tag, imported, list) in sections {
        let list: Vec<_> = list
            .into_iter()
            .filter(|(pid, pname, pgroup)| match filter {
                Filter::All => true,
                Filter::Color => !profile_is_bw(pid, pname, pgroup),
                Filter::Bw => profile_is_bw(pid, pname, pgroup),
            })
            .collect();
        if list.is_empty() {
            continue;
        }
        // an imported folder named like a built-in group still opens and closes on its own
        let open_id = egui::Id::new(("profile-group", imported, title.as_str()));
        let open: bool = ui.data(|m| m.get_temp(open_id)).unwrap_or(true);
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
        register(ui.ctx(), if imported { format!("profileGroup:lut:{title}") } else { format!("profileGroup:{title}") }, r);
        paint(
            ui.painter(),
            Rect::from_center_size(pos2(r.left() + MARGIN_L, r.center().y), vec2(12.0, 12.0)),
            if open { Icon::ChevronDown } else { Icon::ChevronRight },
            t.text_label,
        );
        ui.painter().text(
            pos2(r.left() + 36.0, r.center().y),
            Align2::LEFT_CENTER,
            crate::i18n::builtin_label(&title, !imported),
            t.semibold(13.0),
            t.text_label,
        );
        ui.painter().text(pos2(r.right() - MARGIN_R, r.center().y), Align2::RIGHT_CENTER, list.len().to_string(), t.font(12.0), t.text_dim);
        if resp.clicked() {
            ui.data_mut(|m| m.insert_temp(open_id, !open));
        }
        if !open {
            continue;
        }
        if view == 2 {
            for p in &list {
                let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                let row = Rect::from_min_max(pos2(rr.left() + MARGIN_L, rr.top()), pos2(rr.right() - MARGIN_R, rr.bottom()));
                list_row_ui(app, ui, &d, id, tag, p, row);
            }
        } else {
            let columns = if view == 1 { 1 } else { 2 };
            let gap = 8.0;
            let cw = ((ui.available_width() - MARGIN_L - MARGIN_R - gap * (columns as f32 - 1.0)) / columns as f32).max(40.0);
            let th = (cw * if view == 1 { 0.66 } else { 0.75 }).round();
            for row in list.chunks(columns) {
                let (rr, _) = ui.allocate_exact_size(vec2(ui.available_width(), th + 32.0), Sense::hover());
                for (i, p) in row.iter().enumerate() {
                    let cell = Rect::from_min_size(pos2(rr.left() + MARGIN_L + i as f32 * (cw + gap), rr.top()), vec2(cw, th + 24.0));
                    cell_ui(app, ui, &d, id, tag, p, cell, th);
                }
            }
        }
        ui.add_space(6.0);
    }
    ui.add_space(30.0);
    let last_hover_id = egui::Id::new("last-profile-hover");
    let pointer_in_profiles = ui.ctx().input(|i| i.pointer.hover_pos()).is_some_and(|pos| ui.max_rect().contains(pos));
    if pointer_in_profiles {
        if app.hover_preview.is_none()
            && let Some((saved_id, saved_profile_id)) = ui.data(|m| m.get_temp::<(PhotoId, String)>(last_hover_id))
            && saved_id == id
            && let Some((p_name, _)) = app.session.profile_info(&saved_profile_id)
        {
            let s = with_profile_id(&d, &saved_profile_id);
            app.hover_preview = Some(HoverPreview {
                label: crate::i18n::tr_format!("Profile: {}", crate::i18n::profile_label(&saved_profile_id, p_name)),
                settings: s,
            });
        }
    } else {
        ui.data_mut(|m| m.remove::<(PhotoId, String)>(last_hover_id));
    }
}

/// A toolbar row inside the browser's margins.
fn toolbar_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::NONE.inner_margin(egui::Margin { left: MARGIN_L as i8, right: MARGIN_R as i8, top: 0, bottom: 0 }).show(ui, add);
}

/// Manage Profiles: every group (hidden ones too) with a show / hide switch.
fn manage_ui(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    for g in app.session.profile_groups_for(Some(id), true) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        let key = if g.imported { format!("profileShow:lut:{}", g.name) } else { format!("profileShow:{}", g.name) };
        register(ui.ctx(), key, r);
        let bx = Rect::from_center_size(pos2(r.left() + MARGIN_L + 7.0, r.center().y), vec2(14.0, 14.0));
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(r.shrink2(vec2(MARGIN_L - 8.0, 2.0)), 3.0, t.hover);
        }
        p.rect_stroke(bx, 3.0, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
        if !g.hidden {
            paint(p, bx.shrink(2.0), Icon::Check, t.text);
        }
        p.text(
            pos2(bx.right() + 10.0, r.center().y),
            Align2::LEFT_CENTER,
            crate::i18n::builtin_label(&g.name, !g.imported),
            t.font(13.0),
            if g.hidden { t.text_dim } else { t.text_label },
        );
        p.text(pos2(r.right() - MARGIN_R, r.center().y), Align2::RIGHT_CENTER, g.profiles.len().to_string(), t.font(12.0), t.text_dim);
        if resp.clicked() {
            let _ = app.run("profile.group.show", json!({"group": g.name, "show": g.hidden}));
        }
    }
    ui.add_space(30.0);
}

/// `d` with profile `profile_id` (at 100 % unless it is the applied one).
fn with_profile_id(d: &DevelopSettings, profile_id: &str) -> DevelopSettings {
    let mut s = d.clone();
    if s.profile.id != profile_id {
        s.profile.id = profile_id.to_string();
        s.profile.amount = 100.0;
    }
    s
}

/// What a click / hover on a profile does: the star toggles the favourite, elsewhere it applies
/// (and resting on it previews it in the loupe).
#[allow(clippy::too_many_arguments)]
fn interact(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    resp: &egui::Response,
    star: Rect,
    id: PhotoId,
    p: &(String, String, String),
    applied: bool,
    s: &DevelopSettings,
) {
    let (p_id, p_name, p_group) = (p.0.as_str(), p.1.as_str(), p.2.as_str());
    let over_star = resp.hover_pos().is_some_and(|q| star.contains(q));
    if resp.hovered() && !applied && !over_star {
        app.hover_preview =
            Some(HoverPreview { label: crate::i18n::tr_format!("Profile: {}", crate::i18n::profile_label(p_id, p_name)), settings: s.clone() });
        ui.data_mut(|m| m.insert_temp(egui::Id::new("last-profile-hover"), (id, p_id.to_string())));
    }
    let resp = resp.clone().on_hover_text(format!(
        "{} ({})",
        crate::i18n::profile_label(p_id, p_name),
        crate::i18n::builtin_label(p_group, lightcraft_engine::presets::profile(p_id).is_some())
    ));
    if resp.clicked() {
        if resp.interact_pointer_pos().is_some_and(|q| star.contains(q)) {
            let _ = app.run("profile.favorite", json!({"id": p_id}));
        } else if !applied {
            let _ = app.run("develop.profile", json!({"id": p_id, "amount": 100}));
            ui.data_mut(|m| m.remove::<(PhotoId, String)>(egui::Id::new("last-profile-hover")));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cell_ui(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    d: &DevelopSettings,
    id: PhotoId,
    tag: &str,
    p: &(String, String, String),
    cell: Rect,
    th: f32,
) {
    let (p_id, p_name) = (p.0.as_str(), p.1.as_str());
    let t = Tokens::get(ui.ctx());
    let key = if tag.is_empty() { format!("profileCell:{p_id}") } else { format!("profileCell:{tag}:{p_id}") };
    let resp = ui.interact(cell, egui::Id::new(&key), Sense::click());
    register(ui.ctx(), key, cell);
    let thumb = Rect::from_min_size(cell.min, vec2(cell.width(), th));
    let s = with_profile_id(d, p_id);
    let painter = ui.painter();
    painter.rect_filled(thumb, 3.0, t.canvas);
    if let Some(job) = app.session.variant_job(id, &s, THUMB_EDGE)
        && let Some(tex) = app.renderer.variant(job)
    {
        painter.image(tex.tex.id(), thumb, cover_uv(thumb, tex.size), Color32::WHITE);
    }
    let applied = d.profile.id == p_id;
    if applied {
        painter.rect_stroke(thumb.expand(1.0), 4.0, Stroke::new(2.0, t.pick), egui::StrokeKind::Outside);
    } else if resp.hovered() {
        painter.rect_stroke(thumb, 3.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Outside);
    }
    let name_color = if applied { t.text } else { t.text_label };
    // long names (Camera Matching: "Camera Landscape") end in … inside the cell, clear of the next
    let label = crate::i18n::profile_label(p_id, p_name);
    let font = t.font(12.0);
    let fits = |s: &str| painter.layout_no_wrap(s.to_string(), font.clone(), name_color).size().x;
    let max = (cell.width() - 4.0).max(10.0);
    let shown = if fits(label) <= max {
        label.to_string()
    } else {
        let n = label.chars().count();
        (1..n)
            .rev()
            .map(|k| label.chars().take(k).chain(std::iter::once('…')).collect::<String>())
            .find(|c| fits(c) <= max)
            .unwrap_or_else(|| "…".into())
    };
    painter.text(pos2(cell.left() + 2.0, thumb.bottom() + 12.0), Align2::LEFT_CENTER, shown, font, name_color);
    // favourite star (always shown when set, on hover otherwise)
    let fav = app.session.profile_favorites.iter().any(|f| f == p_id);
    let star = Rect::from_center_size(pos2(thumb.right() - 13.0, thumb.top() + 13.0), vec2(20.0, 20.0));
    register(ui.ctx(), format!("profileStar:{}{}", if tag.is_empty() { "" } else { "fav:" }, p_id), star);
    let over_star = resp.hover_pos().is_some_and(|q| star.contains(q));
    if fav || resp.hovered() {
        painter.circle_filled(star.center(), 10.0, Color32::from_black_alpha(110));
        paint(painter, star.shrink(4.0), if fav { Icon::StarFilled } else { Icon::Star }, if fav || over_star { t.star } else { Color32::WHITE });
    }
    interact(app, ui, &resp, star, id, p, applied, &s);
}

/// List view: one row per profile (name, favourite star), no thumbnail.
fn list_row_ui(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &DevelopSettings, id: PhotoId, tag: &str, p: &(String, String, String), row: Rect) {
    let (p_id, p_name) = (p.0.as_str(), p.1.as_str());
    let t = Tokens::get(ui.ctx());
    let key = if tag.is_empty() { format!("profileCell:{p_id}") } else { format!("profileCell:{tag}:{p_id}") };
    let resp = ui.interact(row, egui::Id::new(&key), Sense::click());
    register(ui.ctx(), key, row);
    let applied = d.profile.id == p_id;
    let painter = ui.painter();
    if applied {
        painter.rect_filled(row, 3.0, t.pressed);
    } else if resp.hovered() {
        painter.rect_filled(row, 3.0, t.hover);
    }
    painter.text(
        pos2(row.left() + 10.0, row.center().y),
        Align2::LEFT_CENTER,
        crate::i18n::profile_label(p_id, p_name),
        t.font(12.5),
        if applied { t.text } else { t.text_label },
    );
    let fav = app.session.profile_favorites.iter().any(|f| f == p_id);
    let star = Rect::from_center_size(pos2(row.right() - 14.0, row.center().y), vec2(20.0, 20.0));
    register(ui.ctx(), format!("profileStar:{}{}", if tag.is_empty() { "" } else { "fav:" }, p_id), star);
    let over_star = resp.hover_pos().is_some_and(|q| star.contains(q));
    if fav || resp.hovered() {
        paint(painter, star.shrink(4.0), if fav { Icon::StarFilled } else { Icon::Star }, if fav || over_star { t.star } else { t.text_dim });
    }
    let s = with_profile_id(d, p_id);
    interact(app, ui, &resp, star, id, p, applied, &s);
}
