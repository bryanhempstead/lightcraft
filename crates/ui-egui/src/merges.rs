//! merges. in the app (Bryan, 2026-11-23): "recognize possible photo merges and put those in a
//! separate area … on the side bar". The finding, the groups and what a merge does are the
//! engine's ([`lightcraft_engine::merge_find`]); here:
//!
//! * the automatic finder ([`tick`]): with Settings ▸ General ▸ find merges: `auto.` (the default)
//!   it looks in the background, off the UI thread, at folders whose photos changed (all of them
//!   the first time, then after each import); with `find.` only when find. is clicked;
//! * `merge.mergeGroups` in the app ([`run_ui`]): the groups go one at a time to the merge worker
//!   ([`crate::merge::start_group`]); the window stays responsive;
//! * the merges. view's bar ([`bar`]: find., all., none., merge., skip., both., merges.only.,
//!   unmerge.) and its group headers ([`group_header`]: the kind, which can be changed, merge.,
//!   skip.);
//! * the kind tag on thumbnails ([`tag`]: `hdr.`, `pano.`, `stack.`, `hdr.pano.`).
//!
//! The sidebar's `merges.` rows are drawn by `panels::left` (under each folder and collection that
//! has groups), from [`counts`].

use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::{MergeType, PhotoId};
use lightcraft_engine::merge_find::{self, Counts, GroupState, MergeGroup};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// When merges are found (Settings ▸ General ▸ find merges:).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindMode {
    /// auto.: in the background, by themselves (after imports, and once over the library).
    #[default]
    Auto,
    /// find.: only when find. is clicked in a merges. view.
    Manual,
}

/// Seconds between two looks of the automatic finder.
const AUTO_EVERY: f64 = 4.0;

/// The sidebar's counts, worked out once per catalog revision.
pub fn counts(app: &LightcraftApp, ctx: &egui::Context) -> Arc<Counts> {
    let key = egui::Id::new("merges-counts");
    let rev = app.session.catalog.revision;
    if let Some((r, c)) = ctx.data(|d| d.get_temp::<(u64, Arc<Counts>)>(key))
        && r == rev
    {
        return c;
    }
    let c = Arc::new(merge_find::counts(&app.session.catalog));
    ctx.data_mut(|d| d.insert_temp(key, (rev, c.clone())));
    c
}

/// Per frame: land a finder run, start the automatic one, feed the merge worker.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    if let Some(r) = app.session.merge_find_poll() {
        landed(app, ctx, r);
    }
    if app.ui.settings.merge_find == FindMode::Auto && app.session.vision.background && !app.session.merge_find_running() && app.import.is_none() {
        let now = ctx.input(|i| i.time);
        let key = egui::Id::new("merges-auto-at");
        // (time of the last look, the catalog it looked at): nothing changed, nothing to look at
        let last: Option<(f64, u64)> = ctx.data(|d| d.get_temp(key));
        let rev = app.session.catalog.revision;
        if last.is_none_or(|(t, r)| now - t >= AUTO_EVERY && r != rev) {
            ctx.data_mut(|d| d.insert_temp(key, (now, rev)));
            if let Some(r) = app.session.merge_find_auto() {
                landed(app, ctx, r);
            }
        }
    }
    if app.session.merge_find_running() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    crate::merge::next_group(app, ctx);
}

/// What a finder run says when it lands (or starts in the background: nothing yet).
fn landed(app: &mut LightcraftApp, ctx: &egui::Context, r: Result<Value, lightcraft_engine::EngineError>) {
    match r {
        Ok(v) => {
            let n = v.get("found").and_then(Value::as_u64).unwrap_or(0);
            if n > 0 {
                app.toast(ctx, format!("merges.: {n} found"));
            }
        }
        Err(e) => app.ui.status = e.to_string(),
    }
}

/// The app's own handling of merges. commands (`None`: the engine's): `merge.find` looks in the
/// background, `merge.mergeGroups` queues the groups for the merge worker, `merge.findMode` is
/// the setting.
pub fn run_ui(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "merge.findMode" => Some(match p.get("mode").and_then(Value::as_str).map(|m| m.trim_end_matches('.')) {
            Some("auto") => {
                app.ui.settings.merge_find = FindMode::Auto;
                Ok(json!({"mode": "auto"}))
            }
            Some("find" | "manual") => {
                app.ui.settings.merge_find = FindMode::Manual;
                Ok(json!({"mode": "manual"}))
            }
            None => Ok(json!({"mode": if app.ui.settings.merge_find == FindMode::Auto { "auto" } else { "manual" }})),
            Some(other) => Err(format!("merge.findMode: mode is auto or find, not `{other}`")),
        }),
        "merge.find" if p.get("background").is_none() && app.session.vision.background => {
            let mut p = p.clone();
            if let Some(o) = p.as_object_mut() {
                o.insert("background".into(), json!(true));
            } else {
                p = json!({"background": true});
            }
            Some(app.session.execute(id, &p).map_err(|e| e.to_string()))
        }
        "merge.mergeGroups" if p.get("sync").and_then(Value::as_bool) != Some(true) => Some(queue(app, p)),
        _ => None,
    }
}

/// `merge.mergeGroups` in the app: each open group (with its kind, or `kind`; only `frames` of it
/// when two or more) onto the merge worker's queue.
fn queue(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let ids: Vec<String> = p
        .get("ids")
        .and_then(Value::as_array)
        .ok_or("merge.mergeGroups: `ids` are merge group ids")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let kind = match p.get("kind").and_then(Value::as_str).filter(|k| !k.starts_with("auto")) {
        Some(k) => Some(MergeType::parse(k).ok_or_else(|| format!("merge.mergeGroups: kind `{k}` is hdr, pano, stack or hdrPano"))?),
        None => None,
    };
    let only: Vec<PhotoId> =
        p.get("frames").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_u64).map(PhotoId).collect()).unwrap_or_default();
    let mut queued = 0;
    for id in &ids {
        let Some(g) = merge_find::group(&app.session.catalog, id).filter(|g| g.state == GroupState::Open) else { continue };
        if app.merge.group_queue.iter().any(|q| q.group == g.id)
            || app.merge.final_task.as_ref().and_then(|t| t.group.as_ref()).is_some_and(|(q, _)| *q == g.id)
        {
            continue;
        }
        let k = kind.unwrap_or(g.kind);
        if k != g.kind {
            app.session.set_group_kind(&g.id, k).map_err(|e| e.to_string())?;
        }
        let frames = app.session.merge_group_frames(&g, &only);
        app.merge.group_queue.push_back(crate::merge::GroupMerge { group: g.id.clone(), kind: k, frames });
        queued += 1;
    }
    if queued == 0 {
        return Err("no open merge group to merge".into());
    }
    // the first one starts now (the rest as the worker frees up)
    if app.merge.final_task.is_none()
        && let Some(g) = app.merge.group_queue.pop_front()
    {
        crate::merge::start_group(app, &g)?;
    }
    Ok(json!({"queued": queued}))
}

/// The open groups the selection touches (any of their frames picked), and the picked frames.
fn picked_groups(app: &LightcraftApp) -> (Vec<MergeGroup>, Vec<PhotoId>) {
    let sel = &app.session.selection.ids;
    let gs: Vec<MergeGroup> = merge_find::groups(&app.session.catalog)
        .into_iter()
        .filter(|g| g.state == GroupState::Open && g.photos.iter().any(|p| sel.contains(p)))
        .collect();
    (gs, sel.clone())
}

/// The merged photos picked.
fn picked_results(app: &LightcraftApp) -> Vec<u64> {
    let merged: Vec<PhotoId> =
        merge_find::groups(&app.session.catalog).into_iter().filter(|g| g.state == GroupState::Merged).filter_map(|g| g.result).collect();
    app.session.selection.ids.iter().filter(|i| merged.contains(i)).map(|i| i.0).collect()
}

/// Merge the picked groups (only the picked frames of a group when two or more of it are picked).
fn merge_picked(app: &mut LightcraftApp, ctx: &egui::Context) {
    let (gs, frames) = picked_groups(app);
    if gs.is_empty() {
        app.toast(ctx, crate::i18n::tr("Pick a group first: click its header or its photos"));
        return;
    }
    let ids: Vec<&str> = gs.iter().map(|g| g.id.as_str()).collect();
    let frames: Vec<u64> = frames.iter().map(|p| p.0).collect();
    if let Err(e) = app.run("merge.mergeGroups", json!({"ids": ids, "frames": frames})) {
        app.toast(ctx, e);
    }
}

/// Height of the merges. bar.
pub const BAR_H: f32 = 40.0;

/// The bar above a merges. view: find. (finding… while it looks), all. / none., merge. / skip. for
/// the picked groups, both. / merges.only. / unmerge. for the picked merged photos.
pub fn bar(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), BAR_H), Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.canvas);
    register(ui.ctx(), "merges:bar", r);
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(r.left() + 16.0, r.top() + 8.0), pos2(r.right() - 16.0, r.bottom() - 8.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    let ctx = ui.ctx().clone();
    let running = app.session.merge_find_running();
    let find_label = if running { "finding…" } else { "find." };
    let auto = app.ui.settings.merge_find == FindMode::Auto;
    let tip = if auto {
        "Look for merges here now (they are also found by themselves in the background: Settings ▸ General ▸ find merges:)"
    } else {
        "Look for merges here: brackets (hdr.), panoramas (pano.), focus stacks (stack.), HDR panoramas (hdr.pano.)"
    };
    if crate::widgets::text_button(&mut row, "mergesFind", find_label, running).on_hover_text(crate::i18n::tr(tip)).clicked() && !running {
        let scope = app.session.merges_scope.clone().unwrap_or_default();
        match app.run("merge.find", json!({"scope": scope})) {
            Ok(v) if v.get("background").is_none() => {
                let n = v.get("found").and_then(Value::as_u64).unwrap_or(0);
                app.toast(&ctx, format!("merges.: {n} found"));
            }
            Ok(_) => {}
            Err(e) => app.toast(&ctx, e),
        }
    }
    gap(&mut row);
    if crate::widgets::text_button(&mut row, "mergesAll", "all.", false)
        .on_hover_text(crate::i18n::tr("Pick every group and merged photo (⌘A)"))
        .clicked()
    {
        let ids: Vec<u64> = app.session.visible_cloned().iter().map(|p| p.0).collect();
        let _ = app.run("library.select", json!({"ids": ids}));
    }
    if crate::widgets::text_button(&mut row, "mergesNone", "none.", false).on_hover_text(crate::i18n::tr("Pick nothing")).clicked() {
        app.session.selection = Default::default();
    }
    gap(&mut row);
    let busy = app.merge.final_task.as_ref().is_some_and(|t| t.group.is_some());
    let waiting = app.merge.group_queue.len();
    let merge_label = if busy { format!("merging… {}", waiting + 1) } else { "merge.".to_string() };
    if crate::widgets::text_button(&mut row, "mergesMerge", &merge_label, busy)
        .on_hover_text(crate::i18n::tr("Merge the picked groups, each as its kind (only the picked photos of a group when two or more of it are picked). The originals then leave the album — not deleted — and the merged photo shows here and where they were"))
        .clicked()
    {
        merge_picked(app, &ctx);
    }
    if crate::widgets::text_button(&mut row, "mergesSkip", "skip.", false)
        .on_hover_text(crate::i18n::tr("Not a merge: the picked groups are never suggested again (⌘Z brings them back)"))
        .clicked()
    {
        let ids: Vec<String> = picked_groups(app).0.into_iter().map(|g| g.id).collect();
        if ids.is_empty() {
            app.toast(&ctx, crate::i18n::tr("Pick a group first: click its header or its photos"));
        } else if let Err(e) = app.run("merge.dismiss", json!({"ids": ids})) {
            app.toast(&ctx, e);
        }
    }
    gap(&mut row);
    let results = picked_results(app);
    for (id, label, tip, cmd, params) in [
        ("mergesBoth", "both.", "Show the picked merged photos here and in their folder and collections", "merge.showIn", json!({"where": "both"})),
        (
            "mergesOnly",
            "merges.only.",
            "Show the picked merged photos only here, not in their folder or collections",
            "merge.showIn",
            json!({"where": "merges"}),
        ),
        (
            "mergesUnmerge",
            "unmerge.",
            "Bring the picked merged photos' originals back where they were (the merged photo stays)",
            "merge.unmerge",
            json!({}),
        ),
    ] {
        if crate::widgets::text_button(&mut row, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked() {
            if results.is_empty() {
                app.toast(&ctx, crate::i18n::tr("Pick a merged photo first"));
                continue;
            }
            let mut p = params;
            p["ids"] = json!(results);
            if let Err(e) = app.run(cmd, p) {
                app.toast(&ctx, e);
            }
        }
    }
}

/// Room between the bar's groups of buttons.
fn gap(ui: &mut egui::Ui) {
    ui.add_space(8.0);
}

/// A merges. group header: the kind (click: merge as another kind), how many frames, and merge. /
/// skip. at its right. `key` is the run's (`merges:group:<id>`, `merges:merged`).
pub fn group_header(app: &mut LightcraftApp, ui: &mut egui::Ui, key: &str, count: usize, r: Rect, pinned: bool) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter().clone();
    let gid = key.strip_prefix("merges:group:").map(str::to_string);
    let g = gid.as_deref().and_then(|id| merge_find::group(&app.session.catalog, id));
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(r.left() + 8.0, r.top() + 6.0), pos2(r.right() - 16.0, r.bottom() - 6.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    let salt = if pinned { "Pinned" } else { "" };
    match &g {
        Some(g) => {
            // the kind: a menu of the others
            let kind = crate::widgets::text_button(&mut row, &format!("mergesKind{salt}-{}", g.id), g.kind.tag(), false)
                .on_hover_text(crate::i18n::tr("What it merges as: click to merge it as another kind"));
            egui::Popup::menu(&kind).show(|ui| {
                for k in MergeType::ALL {
                    let what = match k {
                        MergeType::Hdr => "hdr. — brackets → one photo with the whole range",
                        MergeType::Pano => "pano. — overlapping photos → one wide photo",
                        MergeType::Stack => "stack. — focused near to far → one photo sharp throughout",
                        MergeType::HdrPano => "hdr.pano. — a bracket at each position → merged, then stitched",
                    };
                    if ui.selectable_label(k == g.kind, crate::i18n::tr(what)).clicked() {
                        let _ = app.run("merge.setKind", json!({"id": g.id, "kind": k.id()}));
                    }
                }
            });
        }
        None => {
            let label = row.painter().layout_no_wrap("merged.".into(), t.semibold(15.0), t.text_label);
            let (lr, _) = row.allocate_exact_size(label.size(), Sense::hover());
            row.painter().galley(lr.min, label, t.text_label);
        }
    }
    let n = count;
    row.label(
        egui::RichText::new(crate::i18n::tr_format!("· {n} photo{}", if n == 1 { "" } else { "s" }, n = n)).font(t.font(12.5)).color(t.text_dim),
    );
    if let Some(g) = &g {
        let mut right = row.new_child(egui::UiBuilder::new().max_rect(row.max_rect()).layout(egui::Layout::right_to_left(egui::Align::Center)));
        right.spacing_mut().item_spacing.x = 8.0;
        let ctx = ui.ctx().clone();
        if crate::widgets::text_button(&mut right, &format!("mergesGroupSkip{salt}-{}", g.id), "skip.", false)
            .on_hover_text(crate::i18n::tr("Not a merge: never suggested again (⌘Z brings it back)"))
            .clicked()
            && let Err(e) = app.run("merge.dismiss", json!({"ids": [g.id]}))
        {
            app.toast(&ctx, e);
        }
        let queued = app.merge.group_queue.iter().any(|q| q.group == g.id);
        let running = app.merge.final_task.as_ref().and_then(|t| t.group.as_ref()).is_some_and(|(q, _)| *q == g.id);
        let label = if running {
            "merging…"
        } else if queued {
            "waiting…"
        } else {
            "merge."
        };
        if crate::widgets::text_button(&mut right, &format!("mergesGroupMerge{salt}-{}", g.id), label, running || queued)
            .on_hover_text(crate::i18n::tr("Merge this group (only its picked photos when two or more are picked)"))
            .clicked()
            && !running
            && !queued
        {
            let frames: Vec<u64> = app.session.selection.ids.iter().filter(|i| g.photos.contains(i)).map(|i| i.0).collect();
            if let Err(e) = app.run("merge.mergeGroups", json!({"ids": [g.id], "frames": frames})) {
                app.toast(&ctx, e);
            }
        }
    }
    let _ = (p, Align2::LEFT_CENTER, Stroke::NONE);
}

/// The kind tag on a merged photo's thumbnail (`pano.`) with its top-left (`from_right`: its
/// top-right) at `at`; `small` for the filmstrip. Returns where it was drawn.
pub fn tag(p: &egui::Painter, t: &Tokens, at: egui::Pos2, kind: MergeType, small: bool, from_right: bool) -> Rect {
    let (size, h) = if small { (9.0, 15.0) } else { (10.5, 20.0) };
    let g = p.layout_no_wrap(kind.tag().into(), t.semibold(size), Color32::WHITE);
    let w = g.size().x + if small { 10.0 } else { 14.0 };
    let at = if from_right { at - vec2(w, 0.0) } else { at };
    let br = Rect::from_min_size(at, vec2(w, h));
    p.rect_filled(br, h / 2.0, Color32::from_black_alpha(170));
    p.galley(pos2(br.center().x - g.size().x / 2.0, br.center().y - g.size().y / 2.0), g, Color32::WHITE);
    br
}
