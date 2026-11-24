//! Edit Capture Time / Adjust Time… (LR-LIB-CAPTURETIME): fix a camera clock on one photo and
//! every selected photo moves by the same amount.
//!
//! The dialog opens on the photo right-clicked (the *anchor*; Photo ▸ Edit Capture Time… and the
//! Info panel use the active photo): its thumbnail and current time, a field for the corrected
//! time (`3:34 pm`, `15:34`, `2026-06-14 15:34:12`, `+1h`, `-30m`, `+1d`), a small calendar, step
//! buttons, the resulting offset and a before → after list (first, anchor, last). The other
//! modes — shift by an amount, time zone, the same time for every photo, match camera clocks,
//! back to the file's time, the file's date — are one click away. Enter applies (one undo
//! step), Esc cancels. Everything goes through `photo.setCaptureTime`; the preview is that
//! command's plan ([`lightcraft_engine::capture_time::plan`]).

use egui::{Color32, Rect, Sense, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_catalog::dates::{MONTHS, civil, display_delta, display_long, iso_seconds, parse_time_input};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// Mode chips: (mode, label, what it does).
pub const MODES: &[(&str, &str, &str)] = &[
    ("set", "adjust.", "Type this photo's correct time; the others move by the same amount"),
    ("shift", "shift.", "Move every photo by an amount"),
    ("zone", "zone.", "Time-zone shift (hours)"),
    ("each", "same.time.", "Every photo gets the same time"),
    ("match", "match.camera.", "Match camera clocks: line this camera up with a photo from another camera taken at the same moment"),
    ("original", "original.", "Back to the time each file records"),
    ("file", "file.date.", "Each file's creation date (UTC)"),
];

/// Step buttons under the time field: (label, seconds).
pub const STEPS: &[(&str, i64)] = &[("\u{2212}1d.", -86_400), ("\u{2212}1h.", -3600), ("+1h.", 3600), ("+1d.", 86_400)];

/// The dialog for `dialog.captureTime {anchor?, mode?}`: anchored on `anchor` when it is one of
/// the photos the command will change (else the active photo, else the first).
pub fn open(app: &LightcraftApp, p: &Value) -> Dialog {
    let s = &app.session;
    let targets = s.targets(&json!({}));
    let wanted = p.get("anchor").and_then(Value::as_u64).filter(|a| targets.contains(&PhotoId(*a)));
    let anchor = wanted.map(|a| json!({"anchor": a})).unwrap_or_else(|| json!({}));
    let anchor = lightcraft_engine::capture_time::anchor_of(s, &anchor, &targets).map(|a| a.0).unwrap_or(0);
    let time = s.catalog.photo(PhotoId(anchor)).map(|p| field_text(p.date())).unwrap_or_default();
    let mode = p.get("mode").and_then(Value::as_str).filter(|m| MODES.iter().any(|x| x.0 == *m)).unwrap_or("set").to_string();
    Dialog::CaptureTime { mode, anchor, reference: 0, time, days: 0, hours: 0, minutes: 0, zone: 0.0 }
}

/// `2026-06-13T14:34:12` → `2026-06-13 14:34:12` (what the field shows).
fn field_text(iso: &str) -> String {
    let t = iso.replace('T', " ");
    t.get(..19).unwrap_or(&t).to_string()
}

/// The `photo.setCaptureTime` request the dialog makes (`None` for other dialogs).
pub fn request(dlg: &Dialog) -> Option<Value> {
    let Dialog::CaptureTime { mode, anchor, reference, time, days, hours, minutes, zone } = dlg else { return None };
    let mut r = match mode.as_str() {
        "set" => json!({"time": time}),
        "each" => json!({"time": time, "each": true}),
        "shift" => json!({"shift": *days as i64 * 86_400 + *hours as i64 * 3600 + *minutes as i64 * 60}),
        "match" => json!({"match": reference}),
        "original" => json!({"revert": true}),
        "file" => json!({"fileDate": true}),
        _ => json!({"hours": zone}),
    };
    if *anchor != 0 {
        r["anchor"] = json!(anchor);
    }
    Some(r)
}

/// The plan for the dialog's request, recomputed only when the request or the catalog changes.
fn cached_plan(app: &LightcraftApp, ctx: &egui::Context, req: &Value) -> Result<lightcraft_engine::capture_time::Plan, String> {
    type Cached = (u64, Result<lightcraft_engine::capture_time::Plan, String>);
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (req.to_string(), app.session.catalog.revision, &app.session.selection.ids).hash(&mut h);
    let key = h.finish();
    let id = egui::Id::new("capture-time-plan");
    if let Some((k, plan)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && k == key
    {
        return plan;
    }
    let plan = lightcraft_engine::capture_time::plan(&app.session, req).map_err(|e| e.to_string());
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, plan.clone())));
    plan
}

/// "Jun 13 · 2:34:12 PM" (the list's short form).
fn short(iso: &str) -> String {
    let long = display_long(iso);
    // "Sat, Jun 13 2026 · 2:34:12 PM" → "Jun 13 · 2:34:12 PM"
    match (long.split_once(", "), long.split_once(" · ")) {
        (Some((_, rest)), Some((_, time))) => {
            let date = rest.split(' ').take(2).collect::<Vec<_>>().join(" ");
            format!("{date} · {time}")
        }
        _ => long,
    }
}

/// The dialog's body. Returns true when Enter asks to apply.
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, dlg: &mut Dialog) -> bool {
    let Dialog::CaptureTime { mode, anchor, reference, time, days, hours, minutes, zone } = dlg else { return false };
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut apply = false;
    let n_sel = app.session.targets(&json!({})).len();
    let Some(ap) = app.session.catalog.photo(PhotoId(*anchor)).cloned() else {
        ui.label(egui::RichText::new(crate::i18n::tr("Select the photos to change first.")).color(t.text_dim));
        return false;
    };
    // the anchor: thumbnail, name, camera, its time now (big)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(vec2(112.0, 84.0), Sense::hover());
        crate::widgets::register(ui.ctx(), "captureTime:thumb", r);
        ui.painter().rect_filled(r, 4.0, t.inset);
        crate::panels::grid::request_thumb(app, ap.id, 256, 9);
        if let Some(tex) = app.renderer.thumb(ap.id) {
            let [tw, th] = tex.size;
            let s = (r.width() / tw.max(1) as f32).min(r.height() / th.max(1) as f32);
            let img = Rect::from_center_size(r.center(), vec2(tw as f32 * s, th as f32 * s));
            ui.painter().image(tex.tex.id(), img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let name = match &ap.copy_name {
                Some(c) => format!("{} · {c}", ap.file_name),
                None => ap.file_name.clone(),
            };
            ui.label(egui::RichText::new(name).color(t.text_label));
            let camera = if ap.meta.camera.trim().is_empty() { crate::i18n::tr("Unknown camera").to_string() } else { ap.meta.camera.clone() };
            let scope = if n_sel > 1 { format!("{camera} · 1 of {n} selected", camera = camera, n = n_sel) } else { camera };
            ui.label(egui::RichText::new(scope).color(t.text_dim));
            let r = ui.label(egui::RichText::new(display_long(ap.date())).font(t.semibold(17.0)).color(t.text));
            crate::widgets::register(ui.ctx(), "captureTime:current", r.rect);
            if ap.captured.is_none() {
                ui.label(egui::RichText::new(crate::i18n::tr("No capture time: this is its import time")).color(t.caution));
            }
        });
    });
    ui.add_space(4.0);
    // the modes, one click away
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        for (m, label, tip) in MODES {
            if crate::widgets::text_button(ui, &format!("captureMode-{m}"), label, mode == m).on_hover_text(crate::i18n::tr(tip)).clicked() {
                *mode = m.to_string();
            }
        }
    });
    ui.add_space(2.0);
    match mode.as_str() {
        "set" | "each" => {
            ui.label(
                egui::RichText::new(crate::i18n::tr(if mode == "set" {
                    "This photo's correct time: 3:34 pm, 15:34, 2026-06-14 15:34, or +1h, -30m, +1d"
                } else {
                    "The time every selected photo gets"
                }))
                .color(t.text_label),
            );
            apply |= time_field(ui, &t, time, ap.date());
            calendar(ui, &t, time, ap.date());
        }
        "shift" => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(crate::i18n::tr("Move by")).color(t.text_label));
                ui.add(egui::DragValue::new(days).suffix(" d"));
                ui.add(egui::DragValue::new(hours).suffix(" h"));
                ui.add(egui::DragValue::new(minutes).suffix(" min"));
            });
        }
        "zone" => {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(crate::i18n::tr("Hours")).color(t.text_label));
                ui.add(egui::DragValue::new(zone).range(-26.0..=26.0).speed(0.25).fixed_decimals(2));
            });
        }
        "match" => match_reference(app, ui, &t, &ap, reference),
        _ => {
            ui.label(
                egui::RichText::new(crate::i18n::tr(if mode == "original" {
                    "Each photo goes back to the capture time its file records."
                } else {
                    "Each photo takes its file's creation date (UTC)."
                }))
                .color(t.text_dim),
            );
        }
    }
    if !matches!(mode.as_str(), "original" | "file") {
        ui.add_space(4.0);
        let req = request(&Dialog::CaptureTime {
            mode: mode.clone(),
            anchor: *anchor,
            reference: *reference,
            time: time.clone(),
            days: *days,
            hours: *hours,
            minutes: *minutes,
            zone: *zone,
        });
        match req.filter(|_| !(mode == "match" && *reference == 0)).map(|r| cached_plan(app, &ctx, &r)) {
            Some(Ok(plan)) => summary(app, ui, &t, &plan),
            Some(Err(e)) => {
                ui.label(egui::RichText::new(e.trim_start_matches("photo.setCaptureTime: ")).color(t.caution));
            }
            None => {}
        }
    }
    // Enter applies (not while typing in a drag value)
    apply |= ui.input(|i| i.key_pressed(egui::Key::Enter)) && ctx.memory(|m| m.focused().is_none());
    apply
}

/// The corrected-time field, focused with its text selected when the dialog opens, and the step
/// buttons. Returns true on Enter.
fn time_field(ui: &mut egui::Ui, t: &Tokens, time: &mut String, current: &str) -> bool {
    let mut enter = false;
    let id = egui::Id::new("captureTime-field");
    // just opened: not drawn in the previous pass
    let pass = ui.ctx().cumulative_pass_nr();
    let first = ui.ctx().data(|d| d.get_temp::<u64>(id.with("pass"))).is_none_or(|p| p + 1 < pass);
    ui.ctx().data_mut(|d| d.insert_temp(id.with("pass"), pass));
    let cal = egui::Id::new("captureTime-calendar");
    if first {
        ui.ctx().data_mut(|d| d.insert_temp(cal, false));
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let out = egui::TextEdit::singleline(time)
            .id(id)
            .hint_text("3:34 pm")
            .desired_width(180.0)
            .min_size(vec2(180.0, 24.0))
            .vertical_align(egui::Align::Center)
            .font(t.font(14.0))
            .show(ui);
        crate::widgets::register(ui.ctx(), "field:captureTime", out.response.rect);
        if first {
            out.response.request_focus();
            let mut st = out.state.clone();
            let n = time.chars().count();
            st.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
            st.store(ui.ctx(), id);
        }
        enter = out.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let target = parse_time_input(time, current);
        // date. opens the calendar below, on the target's month
        let open = ui.ctx().data(|d| d.get_temp::<bool>(cal).unwrap_or(false));
        if crate::widgets::text_button(ui, "captureDate", "date.", open).clicked() {
            let day = target.as_deref().and_then(iso_seconds).unwrap_or(0).div_euclid(86_400);
            let (y, m, _) = lightcraft_catalog::dates::civil_from_days(day);
            ui.ctx().data_mut(|d| {
                d.insert_temp(cal, !open);
                d.insert_temp(cal.with("month"), y * 12 + m - 1);
            });
        }
        for (label, secs) in STEPS {
            let r = crate::widgets::text_button(ui, &format!("captureStep{secs}"), label, false);
            if r.clicked()
                && let Some(s) = target.as_deref().and_then(iso_seconds)
            {
                *time = field_text(&civil(s + secs));
            }
        }
    });
    enter
}

/// A small month calendar (opened by date.): a day keeps the typed time of day.
fn calendar(ui: &mut egui::Ui, t: &Tokens, time: &mut String, current: &str) {
    let open_id = egui::Id::new("captureTime-calendar");
    let target = parse_time_input(time, current).unwrap_or_else(|| civil(iso_seconds(current).unwrap_or(0)));
    let day0 = iso_seconds(&target).unwrap_or(0).div_euclid(86_400);
    let (ty, tm, _) = lightcraft_catalog::dates::civil_from_days(day0);
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id).unwrap_or(false));
    // the month shown (months since year 0), starting at the target's
    let month_id = open_id.with("month");
    let mut shown = ui.ctx().data(|d| d.get_temp::<i64>(month_id)).unwrap_or(ty * 12 + tm - 1);
    if open {
        let (y, m) = (shown.div_euclid(12), shown.rem_euclid(12) + 1);
        let first = iso_seconds(&format!("{y:04}-{m:02}-01")).unwrap_or(0).div_euclid(86_400);
        let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
        let len = iso_seconds(&format!("{ny:04}-{nm:02}-01")).unwrap_or(0).div_euclid(86_400) - first;
        // Monday first (1970-01-01 was a Thursday)
        let lead = (first + 3).rem_euclid(7);
        egui::Frame::new().fill(t.inset).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if crate::widgets::text_button(ui, "captureMonthPrev", "\u{2039}", false).clicked() {
                    shown -= 1;
                }
                let name = MONTHS.get((m - 1) as usize).copied().unwrap_or("");
                ui.label(egui::RichText::new(format!("{} {y}", crate::i18n::tr(name))).font(t.semibold(12.5)).color(t.text));
                if crate::widgets::text_button(ui, "captureMonthNext", "\u{203a}", false).clicked() {
                    shown += 1;
                }
            });
            egui::Grid::new("captureTime-days").spacing([4.0, 4.0]).show(ui, |ui| {
                for d in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                    ui.add_sized([32.0, 18.0], egui::Label::new(egui::RichText::new(d).color(t.text_dim)));
                }
                ui.end_row();
                for cell in 0..42 {
                    let day = cell - lead + 1;
                    if (1..=len).contains(&day) {
                        let picked = first + day - 1 == day0;
                        let r = ui.add_sized([32.0, 24.0], egui::Button::new(day.to_string()).selected(picked));
                        crate::widgets::register(ui.ctx(), format!("button:captureDay-{day}"), r.rect);
                        if r.clicked() {
                            let tod = target.get(10..19).unwrap_or("T00:00:00");
                            *time = field_text(&format!("{y:04}-{m:02}-{day:02}{tod}"));
                        }
                    } else {
                        ui.add_sized([32.0, 24.0], egui::Label::new(""));
                    }
                    if cell % 7 == 6 {
                        ui.end_row();
                        if day >= len {
                            break;
                        }
                    }
                }
            });
        });
    }
    ui.ctx().data_mut(|d| d.insert_temp(month_id, shown));
}

/// Match camera clocks: pick the photo from another camera taken at the same moment as the
/// anchor (the selected photos from other cameras, nearest in time first).
fn match_reference(app: &LightcraftApp, ui: &mut egui::Ui, t: &Tokens, ap: &lightcraft_catalog::Photo, reference: &mut u64) {
    let camera = ap.meta.camera.trim().to_string();
    if camera.is_empty() {
        ui.label(egui::RichText::new(crate::i18n::tr("This photo records no camera, so there is no camera clock to match.")).color(t.text_dim));
        return;
    }
    let at = iso_seconds(ap.date()).unwrap_or(0);
    let mut others: Vec<(i64, PhotoId, String)> = app
        .session
        .targets(&json!({}))
        .into_iter()
        .filter_map(|id| app.session.catalog.photo(id))
        .filter(|p| !p.meta.camera.trim().is_empty() && p.meta.camera.trim() != camera)
        .map(|p| ((iso_seconds(p.date()).unwrap_or(0) - at).abs(), p.id, format!("{} · {} · {}", p.file_name, p.meta.camera.trim(), short(p.date()))))
        .collect();
    others.sort();
    ui.label(
        egui::RichText::new(format!(
            "Pick the photo from the other camera taken at the same moment as this one; the selected {camera} photos move by the difference.",
            camera = camera
        ))
        .color(t.text_label),
    );
    if others.is_empty() {
        ui.label(egui::RichText::new(crate::i18n::tr("Select photos from both cameras first.")).color(t.caution));
        return;
    }
    if !others.iter().any(|o| o.1.0 == *reference) {
        *reference = 0;
    }
    let shown = others.iter().find(|o| o.1.0 == *reference).map(|o| o.2.clone()).unwrap_or_else(|| crate::i18n::tr("Choose a photo…").to_string());
    let r = egui::ComboBox::from_id_salt("captureTime-reference").selected_text(shown).width(ui.available_width().min(400.0)).show_ui(ui, |ui| {
        for (i, (_, id, label)) in others.iter().take(200).enumerate() {
            let r = ui.selectable_label(*reference == id.0, label);
            crate::widgets::register(ui.ctx(), format!("button:captureReference-{i}"), r.rect);
            if r.clicked() {
                *reference = id.0;
            }
        }
    });
    crate::widgets::register(ui.ctx(), "button:captureReference", r.response.rect);
}

/// The result: the anchor's new time, the offset, how many photos move, and before → after
/// for the first, the anchor and the last.
fn summary(app: &LightcraftApp, ui: &mut egui::Ui, t: &Tokens, plan: &lightcraft_engine::capture_time::Plan) {
    let Some(anchor) = plan.anchor else { return };
    let n = plan.rows.len();
    let after = plan.rows.iter().find(|r| r.id == anchor).map(|r| r.to.clone());
    if let Some(a) = &after {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(egui::RichText::new("\u{2192}").font(t.semibold(15.0)).color(t.text_dim));
            let r = ui.label(egui::RichText::new(display_long(a)).font(t.semibold(15.0)).color(t.text));
            crate::widgets::register(ui.ctx(), "captureTime:after", r.rect);
            if let Some(d) = plan.delta {
                ui.label(egui::RichText::new(format!("({})", display_delta(d))).font(t.semibold(13.0)).color(t.accent));
            }
        });
    }
    let cam = plan.camera.as_deref().map(|c| format!("{c} ")).unwrap_or_default();
    let what = match plan.delta {
        Some(d) => crate::i18n::tr_format!("{n} {cam}photo{} shift by {d}", if n == 1 { "" } else { "s" }, n = n, cam = cam, d = display_delta(d)),
        None => crate::i18n::tr_format!("{n} photo{} get this time", if n == 1 { "" } else { "s" }, n = n),
    };
    let mut extra = Vec::new();
    if plan.copies > 0 {
        extra.push(if plan.copies == 1 {
            crate::i18n::tr("1 virtual copy").to_string()
        } else {
            crate::i18n::tr_format!("{c} virtual copies", c = plan.copies)
        });
    }
    if plan.stacked > 0 {
        extra.push(crate::i18n::tr_format!("{c} in collapsed stacks", c = plan.stacked));
    }
    let what = if extra.is_empty() { what } else { format!("{what} ({} {})", crate::i18n::tr("including"), extra.join(", ")) };
    let r = ui.label(egui::RichText::new(what).color(t.text_label));
    crate::widgets::register(ui.ctx(), "captureTime:count", r.rect);
    if !plan.skipped.is_empty() {
        let k = plan.skipped.len();
        ui.label(
            egui::RichText::new(format!(
                "{k} photo{} without a capture time stay{} as they are",
                if k == 1 { "" } else { "s" },
                if k == 1 { "s" } else { "" },
                k = k
            ))
            .color(t.text_dim),
        );
    }
    // before → after: first, anchor, last
    let mut picks: Vec<usize> = Vec::new();
    for i in [Some(0), plan.rows.iter().position(|r| r.id == anchor), n.checked_sub(1)].into_iter().flatten() {
        if !picks.contains(&i) {
            picks.push(i);
        }
    }
    picks.sort_unstable();
    if picks.is_empty() {
        return;
    }
    egui::Frame::new().fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_min_width(ui.available_width());
        egui::Grid::new("captureTime-list").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
            for (k, i) in picks.iter().enumerate() {
                let Some(row) = plan.rows.get(*i) else { continue };
                if k > 0 && *i > picks[k - 1] + 1 {
                    ui.label(egui::RichText::new("\u{2026}").color(t.text_dim));
                    ui.end_row();
                }
                let p = app.session.catalog.photo(row.id);
                let mut name = p.map(|p| p.file_name.clone()).unwrap_or_default();
                if name.chars().count() > 22 {
                    name = format!("{}…", name.chars().take(21).collect::<String>());
                }
                let strong = row.id == anchor;
                let color = if strong { t.text } else { t.text_label };
                ui.label(egui::RichText::new(name).color(color));
                let from = p.map(|p| short(p.date())).unwrap_or_default();
                ui.label(egui::RichText::new(from).color(t.text_dim));
                ui.label(egui::RichText::new("\u{2192}").color(t.text_dim));
                ui.label(egui::RichText::new(short(&row.to)).color(color));
                ui.end_row();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_form_and_request() {
        assert_eq!(short("2026-06-13T14:34:12"), "Jun 13 · 2:34:12 PM");
        let d = Dialog::CaptureTime { mode: "set".into(), anchor: 7, reference: 0, time: "3:34 pm".into(), days: 0, hours: 0, minutes: 0, zone: 0.0 };
        assert_eq!(request(&d), Some(json!({"time": "3:34 pm", "anchor": 7})));
        let d = Dialog::CaptureTime { mode: "match".into(), anchor: 7, reference: 9, time: String::new(), days: 0, hours: 0, minutes: 0, zone: 0.0 };
        assert_eq!(request(&d), Some(json!({"match": 9, "anchor": 7})));
        assert_eq!(request(&Dialog::About), None);
    }
}
