//! The merge. page (Bryan, 2026-11-24 work order item 2): what the merge. tab shows, in three
//! columns like Develop's —
//!
//! * left: the sidebar as on library. (folders, shoots and collections pick the scope; on this tab
//!   each shoot and folder row counts its open merges, `panels::left`);
//! * centre: the merges. view ([`crate::merges::view`]) under its bar, whose row also holds the
//!   filters ([`filter_menus`]: drop-downs kind. panorama. / hdr. / focus.stack. / hdr.panorama.,
//!   confidence. sure. / likely. / maybe., camera.);
//! * right ([`right`], in place of the Library right panel): the picked group — its kind,
//!   confidence and reasons, its frames large in a strip, the Merge dialog's options for its kind
//!   inline ([`crate::merge::options`]) — with `merge.` and `send.back.` at the bottom.
//!
//! Keys ([`keys`]): ← / → the previous / next group, ↩ or M open the Merge box for the picked
//! groups (on the first one's kind; the others of that kind follow it), S sends them back, ⌘A
//! picks every group. Each is a command ([`run_ui`]): `merge.step {by}`, `merge.mergePicked`,
//! `merge.openGroups {ids}`, `merge.mergeNow` (no box: the panel's options), `merge.sendBackPicked`,
//! `merge.pickAll`, `merge.filter {kinds?, confidences?, from?, camera?, clear?}`.

use egui::{Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::MergeType;
use lightcraft_engine::merge_find::Confidence;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::merges::GroupView;
use crate::theme::Tokens;
use crate::widgets::register;

/// The kinds in the filter row's order (what Bryan shoots most first).
pub const KIND_ORDER: [MergeType; 4] = [MergeType::Pano, MergeType::Hdr, MergeType::Stack, MergeType::HdrPano];
/// The confidences, surest first.
pub const CONFIDENCES: [Confidence; 3] = [Confidence::Sure, Confidence::Likely, Confidence::Maybe];

/// The merges. view's filter that isn't remembered (kind. and confidence. are, per screen size:
/// [`crate::merges::MergeView`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    /// Only the groups shot with this camera (its short name).
    pub camera: Option<String>,
    /// The kind / confidence chip clicked last (⇧-click picks the ones from it).
    pub anchor_kind: Option<MergeType>,
    pub anchor_conf: Option<Confidence>,
}

/// Whether a group of confidence `c` shows with the confidences picked (`picked`; none: every one
/// from `floor` up, Settings ▸ merge. ▸ show from).
pub fn confidence_shown(picked: &[Confidence], c: Confidence, floor: Confidence) -> bool {
    if picked.is_empty() { c >= floor } else { picked.contains(&c) }
}

/// The merge. page is showing: the merge. tab on, a merges. view in the grid.
pub fn on(app: &LightcraftApp) -> bool {
    app.merge_tab
        && app.session.source == lightcraft_engine::LibrarySource::Merges
        && matches!(app.ui.view, crate::state::ViewMode::PhotoGrid | crate::state::ViewMode::SquareGrid)
}

// ------------------------------------------------------------------------------------ commands

/// The page's commands (`None`: not one of them).
pub fn run_ui(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "merge.filter" => set_filter(app, p),
        "merge.step" => {
            let by = p.get("by").and_then(Value::as_i64).unwrap_or(1);
            step(app, by)
        }
        "merge.mergePicked" => open_picked(app),
        "merge.mergeNow" => merge_picked(app),
        "merge.openGroups" => {
            let ids: Vec<String> =
                p.get("ids").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            crate::merge::open_groups(app, &ids)
        }
        "merge.sendBackPicked" => send_back_picked(app),
        "merge.pickAll" => {
            let ids: Vec<u64> = shown(app).iter().flat_map(|g| g.frames.iter().map(|p| p.0)).collect();
            let n = shown(app).len();
            app.run("library.select", json!({"ids": ids})).map(|_| json!({"groups": n}))
        }
        _ => return None,
    })
}

/// The open groups the view shows (the filters applied), in order. Outside a frame (a command
/// from the control channel) worked out afresh.
fn shown(app: &LightcraftApp) -> Vec<GroupView> {
    crate::merges::compute_view_data(app).open
}

/// The group the right panel shows: the one holding the active photo, else the first picked one.
pub(crate) fn focus(app: &LightcraftApp, groups: &[GroupView]) -> Option<usize> {
    let sel = &app.session.selection;
    sel.active
        .and_then(|a| groups.iter().position(|g| g.frames.contains(&a)))
        .or_else(|| groups.iter().position(|g| g.frames.iter().any(|f| sel.contains(*f))))
}

/// The picked groups among those shown.
fn picked(app: &LightcraftApp, groups: &[GroupView]) -> Vec<GroupView> {
    let sel = &app.session.selection;
    groups.iter().filter(|g| g.frames.iter().any(|f| sel.contains(*f))).cloned().collect()
}

/// Pick group `i` of `groups` (its frames) and bring it into view.
fn pick(app: &mut LightcraftApp, groups: &[GroupView], i: usize) -> Result<Value, String> {
    let Some(g) = groups.get(i) else { return Ok(json!({"group": null})) };
    let ids: Vec<u64> = g.frames.iter().map(|p| p.0).collect();
    app.run("library.select", json!({"ids": ids}))?;
    app.merge.reveal = Some(g.g.id.clone());
    Ok(json!({"group": g.g.id, "index": i, "of": groups.len()}))
}

/// `merge.step {by}`: the group `by` places on from the one shown (nothing picked: the first, or
/// the last going back); stops at the ends.
fn step(app: &mut LightcraftApp, by: i64) -> Result<Value, String> {
    let groups = shown(app);
    if groups.is_empty() {
        return Err("no merge groups here".into());
    }
    let last = groups.len() as i64 - 1;
    let to = match focus(app, &groups) {
        Some(i) => (i as i64 + by).clamp(0, last),
        None if by < 0 => last,
        None => 0,
    };
    pick(app, &groups, to as usize)
}

/// After the picked groups were merged or sent back: the group after them is picked (the review
/// goes on), or nothing at the end.
fn advance(app: &mut LightcraftApp, groups: &[GroupView], done: &[GroupView]) {
    let after = done.iter().filter_map(|d| groups.iter().position(|g| g.g.id == d.g.id)).max();
    let next = after.and_then(|i| groups.iter().skip(i + 1).position(|g| !done.iter().any(|d| d.g.id == g.g.id)).map(|k| i + 1 + k));
    match next {
        Some(i) => {
            let _ = pick(app, groups, i);
        }
        None => {
            let _ = app.run("library.select", json!({"ids": []}));
        }
    }
}

/// `merge.mergePicked` (merge., ↩, M): the Merge box for the picked groups — the first one's,
/// and the others of its kind with it.
fn open_picked(app: &mut LightcraftApp) -> Result<Value, String> {
    let groups = shown(app);
    let done = picked(app, &groups);
    if done.is_empty() {
        return Err("pick a group first: click its header, or press →".into());
    }
    // the one the panel shows first
    let first = focus(app, &groups).and_then(|i| groups.get(i)).map(|g| g.g.id.clone());
    let mut ids: Vec<String> = first.into_iter().collect();
    for g in &done {
        if !ids.contains(&g.g.id) {
            ids.push(g.g.id.clone());
        }
    }
    crate::merge::open_groups(app, &ids)
}

/// After the Merge box sent `ids` to the merge worker: on to the next group.
pub fn after_merge(app: &mut LightcraftApp, ids: &[String]) {
    let groups = shown(app);
    let done: Vec<GroupView> = groups.iter().filter(|g| ids.contains(&g.g.id)).cloned().collect();
    if !done.is_empty() {
        advance(app, &groups, &done);
    }
}

/// `merge.mergeNow` (the panel's merge.now.): the picked groups onto the merge worker without the
/// box, each kind with the options the page shows for it.
fn merge_picked(app: &mut LightcraftApp) -> Result<Value, String> {
    let groups = shown(app);
    let done = picked(app, &groups);
    if done.is_empty() {
        return Err("pick a group first: click its header, or press →".into());
    }
    let mut queued = 0;
    for k in MergeType::ALL {
        let ids: Vec<&str> = done.iter().filter(|g| g.g.kind == k).map(|g| g.g.id.as_str()).collect();
        if ids.is_empty() {
            continue;
        }
        let opts = crate::merge::page_options(app, k.command());
        let options = serde_json::to_value(&opts).map_err(|e| e.to_string())?;
        let r = app.run("merge.mergeGroups", json!({"ids": ids, "options": options}))?;
        queued += r.get("queued").and_then(Value::as_u64).unwrap_or(0);
    }
    advance(app, &groups, &done);
    Ok(json!({"queued": queued}))
}

/// `merge.sendBackPicked`: the picked groups are not merges.
fn send_back_picked(app: &mut LightcraftApp) -> Result<Value, String> {
    let groups = shown(app);
    let done = picked(app, &groups);
    if done.is_empty() {
        return Err("pick a group first: click its header, or press →".into());
    }
    let ids: Vec<&str> = done.iter().map(|g| g.g.id.as_str()).collect();
    let r = app.run("merge.dismiss", json!({"ids": ids}))?;
    advance(app, &groups, &done);
    Ok(r)
}

/// `merge.filter {kinds?: [kind] | null, confidences?: [sure|likely|maybe] | null, from?:
/// sure|likely|maybe (that one and the surer ones), camera?: name|null, clear?: true}` → the
/// filters now. kind. and confidence. are remembered per screen size.
fn set_filter(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let parse_conf = |s: &str| CONFIDENCES.into_iter().find(|c| c.id() == s.trim_end_matches('.'));
    // null: every one; [] : none of them (nothing picked, no group shows)
    let kinds = match p.get("kinds") {
        None => None,
        Some(Value::Null) => Some((Vec::new(), false)),
        Some(Value::Array(a)) => {
            let mut out = Vec::new();
            for v in a {
                let s = v.as_str().unwrap_or_default();
                let k = MergeType::parse(s).ok_or_else(|| format!("merge.filter: kind `{s}` is panorama, hdr, focus.stack or hdr.panorama"))?;
                if !out.contains(&k) {
                    out.push(k);
                }
            }
            let none = out.is_empty();
            Some((out, none))
        }
        Some(_) => return Err("merge.filter: `kinds` is a list of kinds (null for every kind, [] for none)".into()),
    };
    let mut confs = match p.get("confidences") {
        None => None,
        Some(Value::Null) => Some((Vec::new(), false)),
        Some(Value::Array(a)) => {
            let mut out = Vec::new();
            for v in a {
                let s = v.as_str().unwrap_or_default();
                let c = parse_conf(s).ok_or_else(|| format!("merge.filter: confidence `{s}` is sure, likely or maybe"))?;
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            let none = out.is_empty();
            Some((out, none))
        }
        Some(_) => return Err("merge.filter: `confidences` is a list (null for every one, [] for none)".into()),
    };
    if let Some(f) = p.get("from").and_then(Value::as_str) {
        let c = parse_conf(f).ok_or_else(|| format!("merge.filter: from is sure, likely or maybe, not `{f}`"))?;
        confs = Some((CONFIDENCES.into_iter().filter(|x| *x >= c).collect(), false));
    }
    if p.get("clear").and_then(Value::as_bool) == Some(true) {
        app.merge.filter = Filter::default();
        let v = crate::merges::view_prefs_mut(app);
        v.kinds.clear();
        v.confidences.clear();
        v.kinds_none = false;
        v.confidences_none = false;
    }
    if let Some(c) = p.get("camera") {
        app.merge.filter.camera = c.as_str().filter(|s| !s.is_empty() && !s.starts_with("all")).map(str::to_string);
    }
    let v = crate::merges::view_prefs_mut(app);
    if let Some((k, none)) = kinds {
        v.kinds = k;
        v.kinds_none = none;
    }
    if let Some((c, none)) = confs {
        v.confidences = c;
        v.confidences_none = none;
    }
    Ok(filter_json(app))
}

fn filter_json(app: &LightcraftApp) -> Value {
    let v = crate::merges::view_prefs(app);
    json!({
        // (null: every one; []: none)
        "kinds": if v.kinds.is_empty() && !v.kinds_none { Value::Null } else { json!(v.kinds.iter().map(|k| k.id()).collect::<Vec<_>>()) },
        "confidences": if v.confidences.is_empty() && !v.confidences_none { Value::Null } else { json!(v.confidences.iter().map(|c| c.id()).collect::<Vec<_>>()) },
        "camera": app.merge.filter.camera,
    })
}

// --------------------------------------------------------------------------------------- keys

/// The page's keys; true when one was used.
pub fn keys(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    if !on(app) {
        return false;
    }
    let ran = |app: &mut LightcraftApp, id: &str, p: Value| {
        if let Err(e) = app.run(id, p) {
            app.toast(ctx, e);
        }
        true
    };
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight)) {
        return ran(app, "merge.step", json!({"by": 1}));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft)) {
        return ran(app, "merge.step", json!({"by": -1}));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::M)) {
        return ran(app, "merge.mergePicked", json!({}));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::S)) {
        return ran(app, "merge.sendBackPicked", json!({}));
    }
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
        return ran(app, "merge.pickAll", json!({}));
    }
    false
}

// ------------------------------------------------------------------------------------ filters

/// A drop-down button of the bar: its label and a ▾; `button:{id}` and `mergeFilter:{kind}`.
fn drop_button(ui: &mut egui::Ui, id: &str, which: &str, label: &str, active: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(crate::i18n::tr(label).to_string(), t.semibold(11.5), t.text);
    let (r, resp) = ui.allocate_exact_size(vec2((galley.size().x + 18.0 + 18.0).max(37.0), 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, label));
    register(ui.ctx(), format!("button:{id}"), r);
    register(ui.ctx(), format!("mergeFilter:{which}"), r);
    let fill = if active {
        t.tools_accent.unwrap_or(t.pressed)
    } else if resp.hovered() {
        t.hover
    } else {
        t.button
    };
    let text_c = if active && t.tools_accent.is_some() { crate::theme::text_on(fill) } else { t.text };
    ui.painter().rect(r, 4.0, fill, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
    ui.painter().galley(pos2(r.left() + 9.0, r.center().y - galley.size().y / 2.0), galley, text_c);
    crate::folds::paint_triangle(ui, pos2(r.right() - 13.0, r.center().y), true, text_c);
    resp
}

/// Width of a filter menu's rows.
const MENU_W: f32 = 220.0;

/// A row of a filter menu: its label, then how many groups it shows (quiet; a 0 is a full-strength
/// row with a quiet 0, never greyed out); the picked ones are filled. `menu:{id}`.
fn menu_row(ui: &mut egui::Ui, id: &str, label: &str, count: Option<usize>, picked: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(MENU_W, 26.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, picked, label));
    register(ui.ctx(), format!("menu:{id}"), r);
    let r = r.shrink2(vec2(2.0, 1.0));
    if picked {
        ui.painter().rect_filled(r, 4.0, t.tools_accent.unwrap_or(t.pressed));
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 4.0, t.hover);
    }
    let on_accent = picked && t.tools_accent.is_some();
    let ink = if on_accent { crate::theme::text_on(t.tools_accent.unwrap_or(t.pressed)) } else { t.text };
    let quiet = if on_accent { ink } else { t.text_dim };
    let font = if picked { t.semibold(12.0) } else { t.font(12.0) };
    ui.painter().text(pos2(r.left() + 10.0, r.center().y), egui::Align2::LEFT_CENTER, crate::i18n::tr(label), font, ink);
    if let Some(n) = count {
        ui.painter().text(pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, n.to_string(), t.font(12.0), quiet);
    }
    resp
}

/// What a filter button says of its picks: `all.`, the one picked, or `3 picked.`.
fn picks_label<K>(picked: &[K], none: bool, tag: impl Fn(&K) -> &'static str) -> String {
    match picked {
        [] if none => "none.".to_string(),
        [] => "all.".to_string(),
        [k] => tag(k).to_string(),
        more => format!("{} picked.", more.len()),
    }
}

/// The filters, drop-downs on the merges. bar's own row (right after find.): kind. and
/// confidence. (click: only that one; ⌘-click adds / takes away; ⇧-click: the ones from the last
/// clicked; all. / none.: every one), camera. (one camera), each row with how many groups it
/// shows here, then how many groups show.
pub fn filter_menus(app: &mut LightcraftApp, row: &mut egui::Ui) {
    let ctx = row.ctx().clone();
    let data = crate::merges::view_data(app, &ctx);
    let vp = crate::merges::view_prefs(app);
    let t = Tokens::get(&ctx);
    // kind.
    let kinds_label = format!("kind. {}", picks_label(&vp.kinds, vp.kinds_none, |k| k.tag()));
    let btn = drop_button(row, "mergesFilterKind", "kind", &kinds_label, !vp.kinds.is_empty() || vp.kinds_none)
        .on_hover_text(crate::i18n::tr("Which kinds of merge show (click: only that one, ⌘-click: add or take away, ⇧-click: the run from the last one)"));
    let close = egui::Popup::default_response_id(&btn);
    egui::Popup::menu(&btn).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let all: usize = data.kind_counts.values().sum();
        let mut pick: Option<Vec<MergeType>> = None;
        let mut none = false;
        let mut keep_open = false;
        if menu_row(ui, "mergesFilterKind-all", "all.", Some(all), vp.kinds.is_empty() && !vp.kinds_none).clicked() {
            pick = Some(Vec::new());
        }
        for k in KIND_ORDER {
            let n = data.kind_counts.get(&k).copied().unwrap_or(0);
            if menu_row(ui, &format!("mergesFilterKind-{}", k.id()), k.tag(), Some(n), vp.kinds.contains(&k)).clicked() {
                let m = ui.input(|i| i.modifiers);
                keep_open = m.command || m.shift;
                pick = Some(chip_click(&KIND_ORDER, &vp.kinds, app.merge.filter.anchor_kind, k, m));
                app.merge.filter.anchor_kind = Some(k);
            }
        }
        if menu_row(ui, "mergesFilterKind-none", "none.", None, vp.kinds_none).clicked() {
            pick = Some(Vec::new());
            none = true;
        }
        if let Some(next) = pick {
            let ids: Vec<&str> = next.iter().map(|k| k.id()).collect();
            let _ = app.run("merge.filter", json!({"kinds": if ids.is_empty() && !none { Value::Null } else { json!(ids) }}));
            if !keep_open {
                egui::Popup::close_id(ui.ctx(), close);
            }
        }
    });
    // confidence.
    let from = app.ui.settings.merge.show_from;
    let conf_label = format!("confidence. {}", picks_label(&vp.confidences, vp.confidences_none, |c| c.tag()));
    let btn = drop_button(row, "mergesFilterConfidence", "confidence", &conf_label, !vp.confidences.is_empty() || vp.confidences_none)
        .on_hover_text(crate::i18n::tr("Which confidences show (click: only that one, ⌘-click: add or take away, ⇧-click: the run from the last one)"));
    let close = egui::Popup::default_response_id(&btn);
    egui::Popup::menu(&btn).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let mut pick: Option<Vec<Confidence>> = None;
        let mut none = false;
        let mut keep_open = false;
        if menu_row(ui, "mergesFilterConfidence-all", "all.", Some(data.conf_all), vp.confidences.is_empty() && !vp.confidences_none)
            .on_hover_text(crate::i18n::tr(&format!("Every group from {} up (Settings ▸ merge. ▸ show from)", from.tag())))
            .clicked()
        {
            pick = Some(Vec::new());
        }
        for c in CONFIDENCES {
            let n = data.conf_counts.get(&c).copied().unwrap_or(0);
            if menu_row(ui, &format!("mergesFilterConfidence-{}", c.id()), c.tag(), Some(n), vp.confidences.contains(&c)).clicked() {
                let m = ui.input(|i| i.modifiers);
                keep_open = m.command || m.shift;
                pick = Some(chip_click(&CONFIDENCES, &vp.confidences, app.merge.filter.anchor_conf, c, m));
                app.merge.filter.anchor_conf = Some(c);
            }
        }
        if menu_row(ui, "mergesFilterConfidence-none", "none.", None, vp.confidences_none).clicked() {
            pick = Some(Vec::new());
            none = true;
        }
        if let Some(next) = pick {
            let ids: Vec<&str> = next.iter().map(|c| c.id()).collect();
            let _ = app.run("merge.filter", json!({"confidences": if ids.is_empty() && !none { Value::Null } else { json!(ids) }}));
            if !keep_open {
                egui::Popup::close_id(ui.ctx(), close);
            }
        }
    });
    // camera.
    let cam = app.merge.filter.camera.clone();
    let cam_label = cam.as_ref().map_or_else(|| "camera.".to_string(), |c| format!("camera. {c}"));
    let btn = drop_button(row, "mergesFilterCamera", "camera", &cam_label, cam.is_some())
        .on_hover_text(crate::i18n::tr("Only the groups shot with one camera"));
    let close = egui::Popup::default_response_id(&btn);
    egui::Popup::menu(&btn).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        let mut pick: Option<Option<String>> = None;
        if menu_row(ui, "mergesFilterCamera-all", "all.", Some(data.camera_all), cam.is_none()).clicked() {
            pick = Some(None);
        }
        for (i, c) in data.cameras.iter().enumerate() {
            let n = data.camera_counts.get(c).copied().unwrap_or(0);
            if menu_row(ui, &format!("mergesFilterCamera-{i}"), c, Some(n), cam.as_ref() == Some(c)).clicked() {
                pick = Some(if cam.as_ref() == Some(c) { None } else { Some(c.clone()) });
            }
        }
        if menu_row(ui, "mergesFilterCamera-none", "none.", None, false).clicked() {
            pick = Some(None);
        }
        if let Some(next) = pick {
            let _ = app.run("merge.filter", json!({"camera": next}));
            egui::Popup::close_id(ui.ctx(), close);
        }
    });
    // how many groups show
    let n = data.open.len();
    let count = if n == data.open_total { format!("{n} group{}", if n == 1 { "" } else { "s" }) } else { format!("{n} of {} groups", data.open_total) };
    let g = row.painter().layout_no_wrap(crate::i18n::tr(&count).to_string(), t.font(12.0), t.text_dim);
    let (r, _) = row.allocate_exact_size(vec2(g.size().x, 24.0), Sense::hover());
    register(&ctx, "label:mergesFilterCount", r);
    row.painter().galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, t.text_dim);
}

/// Which of `order` a click on chip `k` leaves picked (empty: every one): click = only it (again:
/// every one), ⌘-click = add / take away, ⇧-click = the ones from `anchor` to it.
fn chip_click<K: Copy + PartialEq>(order: &[K], picked: &[K], anchor: Option<K>, k: K, m: egui::Modifiers) -> Vec<K> {
    let at = |x: K| order.iter().position(|y| *y == x);
    if m.shift
        && let (Some(a), Some(b)) = (anchor.and_then(at), at(k))
    {
        let (lo, hi) = (a.min(b), a.max(b));
        return order.get(lo..=hi).unwrap_or(&[]).to_vec();
    }
    if m.command {
        let mut v = picked.to_vec();
        match v.iter().position(|x| *x == k) {
            Some(i) => {
                v.remove(i);
            }
            None => v.push(k),
        }
        return v;
    }
    if picked == [k] { Vec::new() } else { vec![k] }
}

// -------------------------------------------------------------------------------- right panel

/// Room inside the right panel's edges.
const SIDE: f32 = 12.0;
const FOOTER_H: f32 = 44.0;

/// The right panel of the merge. page: the picked group, its frames, its options; merge. and
/// send.back. at the bottom.
pub fn right(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(Stroke::new(1.0, t.divider));
    let reserve = if app.ui.left_panel { crate::state::LEFT_WIDTH.min } else { 0.0 };
    let width = app.ui.right_width;
    let resized = crate::panels::resizable_side(ui, false, "right_panel", frame, width, crate::state::RIGHT_WIDTH, reserve, |ui| {
        let full = ui.max_rect();
        register(ui.ctx(), "mergePage:right", full);
        let body = Rect::from_min_max(full.min, pos2(full.right(), full.bottom() - FOOTER_H));
        let mut top = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
        egui::ScrollArea::vertical().id_salt("merge-page-right").auto_shrink([false, false]).show(&mut top, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            panel_body(app, ui);
            ui.add_space(12.0);
            crate::widgets::reveal_in_scroll(ui);
        });
        footer(app, ui, Rect::from_min_max(pos2(full.left(), body.bottom()), full.max));
        ui.expand_to_include_rect(full);
    });
    if let Some(w) = resized {
        app.ui.right_width = w;
    }
}

/// A section header of the panel (foldable, remembered: `crate::folds`); whether it is open.
fn section(ui: &mut egui::Ui, id: &str, title: &str) -> bool {
    let fold = format!("mergePage.{id}");
    let open = crate::folds::is_open(ui.ctx(), &fold);
    let (resp, _) = crate::widgets::classic_header(ui, &fold, title, open, true, None);
    register(ui.ctx(), format!("fold:{fold}"), resp.rect);
    if resp.clicked() {
        crate::folds::set(ui.ctx(), &fold, !open);
        return !open;
    }
    open
}

/// Content of the panel, `SIDE` in from its edges, blocks 12 px apart.
fn inset<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE.inner_margin(egui::Margin { left: SIDE as i8, right: SIDE as i8, top: 12, bottom: 12 }).show(ui, add).inner
}

fn quiet(ui: &mut egui::Ui, id: &str, text: &str) {
    let t = Tokens::get(ui.ctx());
    let r = ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(text)).size(12.0).color(t.text_dim)).wrap());
    register(ui.ctx(), format!("label:{id}"), r.rect);
}

fn panel_body(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let data = crate::merges::view_data(app, &ctx);
    let groups = &data.open;
    let Some(i) = focus(app, groups) else {
        if section(ui, "group", "group.") {
            inset(ui, |ui| {
                let msg =
                    if groups.is_empty() { "No group to review here" } else { "Pick a group: click its header, or press → for the first one" };
                quiet(ui, "mergePageNone", msg);
            });
        }
        return;
    };
    let Some(gv) = groups.get(i).cloned() else { return };
    let n_picked = picked(app, groups).len();
    if section(ui, "group", "group.") {
        inset(ui, |ui| group_info(app, ui, &gv, i, groups.len(), n_picked));
    }
    if section(ui, "frames", "frames.") {
        inset(ui, |ui| frames(app, ui, &gv));
    }
    if section(ui, "options", "options.") {
        inset(ui, |ui| options(app, ui, &gv));
    }
}

/// prev. / next. and where the group is; its kind (buttons: merge it as another); confidence,
/// cameras, subfolders, frames; its reasons.
fn group_info(app: &mut LightcraftApp, ui: &mut egui::Ui, gv: &GroupView, i: usize, of: usize, n_picked: usize) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let g = &gv.g;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (id, label, tip, by) in [("mergePagePrev", "prev.", "The group before (←)", -1), ("mergePageNext", "next.", "The group after (→)", 1)]
        {
            if crate::widgets::text_button(ui, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked()
                && let Err(e) = app.run("merge.step", json!({"by": by}))
            {
                app.toast(&ctx, e);
            }
        }
        let mut pos = format!("{} of {of}", i + 1);
        if n_picked > 1 {
            pos.push_str(&format!(" · {n_picked} picked"));
        }
        let gl = ui.painter().layout_no_wrap(pos, t.font(12.0), t.text_dim);
        let (r, _) = ui.allocate_exact_size(vec2(gl.size().x, 24.0), Sense::hover());
        register(ui.ctx(), "label:mergePagePosition", r);
        ui.painter().galley(pos2(r.left(), r.center().y - gl.size().y / 2.0), gl, t.text_dim);
    });
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        for k in KIND_ORDER {
            let r = crate::widgets::text_button(ui, &format!("mergePageKind-{}", k.id()), k.tag(), g.kind == k)
                .on_hover_text(crate::i18n::tr("What it merges as: click to merge it as this kind"));
            if r.clicked()
                && g.kind != k
                && let Err(e) = app.run("merge.setKind", json!({"id": g.id, "kind": k.id()}))
            {
                app.toast(&ctx, e);
            }
        }
    });
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        let conf = g.confidence();
        crate::merges::chip(ui, "mergePageConfidence", conf.tag(), Some(crate::merges::confidence_color(conf)));
        for (k, c) in gv.cameras.iter().enumerate() {
            crate::merges::chip(ui, &format!("mergePageCamera-{k}"), c, None).on_hover_text(crate::i18n::tr("The camera"));
        }
        for (k, s) in gv.subfolders.iter().enumerate() {
            crate::merges::chip(ui, &format!("mergePageSubfolder-{k}"), s, None).on_hover_text(crate::i18n::tr("The subfolder of the shoot"));
        }
        let in_group = gv.frames.iter().filter(|f| g.photos.contains(f)).count();
        let count = if in_group == gv.frames.len() { format!("{in_group} frames") } else { format!("{in_group} of {} frames", gv.frames.len()) };
        crate::merges::chip(ui, "mergePageFrames", &count, None);
    });
    if !gv.reasons.is_empty() {
        ui.add_space(12.0);
        quiet(ui, "mergePageReasons", &gv.reasons);
    }
}

/// The group's frames large, one under the other (a strip); a click takes a frame out of the group
/// or puts it back, a double-click opens it.
fn frames(app: &mut LightcraftApp, ui: &mut egui::Ui, gv: &GroupView) {
    let ppp = ui.ctx().pixels_per_point();
    let w = ui.available_width().max(60.0);
    for (k, pid) in gv.frames.iter().enumerate() {
        let a = gv.aspects.get(k).copied().unwrap_or(1.5).clamp(0.4, 2.5);
        let h = (w / a).min(240.0).round();
        let cw = (h * a).min(w);
        let (row, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let r = Rect::from_center_size(row.center(), vec2(cw, h));
        crate::merges::frame_card(app, ui, "Page", &gv.g.id, *pid, gv.g.photos.contains(pid), r, ppp);
        if k + 1 < gv.frames.len() {
            ui.add_space(12.0);
        }
    }
}

/// The Merge dialog's options for the group's kind, inline; defaults. puts back those of
/// Settings ▸ merge.
fn options(app: &mut LightcraftApp, ui: &mut egui::Ui, gv: &GroupView) {
    let command = gv.g.kind.command();
    let mut opts = crate::merge::page_options(app, command);
    ui.spacing_mut().item_spacing.y = 6.0;
    if crate::merge::options(ui, &mut opts, "mergePage") {
        app.merge.page.insert(command.to_string(), opts);
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let changed = app.merge.page.contains_key(command);
        if crate::widgets::text_button(ui, "mergePageDefaults", "defaults.", false)
            .on_hover_text(crate::i18n::tr("Back to this kind's defaults (Settings ▸ merge.)"))
            .clicked()
        {
            app.merge.page.remove(command);
        }
        if crate::widgets::text_button(ui, "mergePageMergeNow", "merge.now.", false)
            .on_hover_text(crate::i18n::tr("Merge the picked groups with these options, without the Merge box"))
            .clicked()
            && let Err(e) = app.run("merge.mergeNow", json!({}))
        {
            app.toast(ui.ctx(), e);
        }
        let note = if changed { "changed here" } else { "the defaults" };
        let t = Tokens::get(ui.ctx());
        ui.add_space(0.0);
        ui.label(egui::RichText::new(crate::i18n::tr(note)).size(12.0).color(t.text_dim));
    });
}

/// merge. and send.back. for the picked groups.
fn footer(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    ui.painter().rect_filled(r, 0.0, t.header);
    register(&ctx, "mergePage:footer", r);
    let half = (r.width() - SIDE * 2.0 - 8.0) / 2.0;
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(SIDE, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    let busy = app.merge.final_task.as_ref().is_some_and(|t| t.group.is_some());
    let label = if busy { format!("merging… {}", app.merge.group_queue.len() + 1) } else { "merge.".to_string() };
    if crate::panels::right::wide_button(&mut row, "mergePageMerge", &label, half)
        .on_hover_text(crate::i18n::tr("Merge the picked groups: the Merge box opens on the first one's kind (↩ or M)"))
        .clicked()
        && let Err(e) = app.run("merge.mergePicked", json!({}))
    {
        app.toast(&ctx, e);
    }
    if crate::panels::right::wide_button(&mut row, "mergePageSendBack", "send.back.", half)
        .on_hover_text(crate::i18n::tr("Not a merge: the picked groups stay in their folder and are never suggested again (S)"))
        .clicked()
        && let Err(e) = app.run("merge.sendBackPicked", json!({}))
    {
        app.toast(&ctx, e);
    }
}
