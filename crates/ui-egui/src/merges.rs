//! merges. in the app (Bryan, 2026-11-23): "recognize possible photo merges and put those in a
//! separate area … on the side bar"; 2026-11-24: not the grid — each found group is "a stacked
//! wave that shows all the photos". The finding, the groups and what a merge does are the
//! engine's ([`lightcraft_engine::merge_find`]); here:
//!
//! * the automatic finder ([`tick`]): with Settings ▸ General ▸ find merges: `auto.` (the default)
//!   it looks in the background, off the UI thread, at folders whose photos changed (all of them
//!   the first time, then after each import); with `find.` only when find. is clicked;
//! * `merge.mergeGroups` in the app ([`run_ui`]): the groups go one at a time to the merge worker
//!   ([`crate::merge::start_group`]); the window stays responsive;
//! * the merges. view's bar ([`bar`]: find., all., none., merge., send.back., both., merges.only.,
//!   unmerge., smaller., bigger.) and the view itself ([`view`]): a panel per group — kind (a
//!   menu), confidence, camera and subfolder, frames, reasons, merge. / send.back. — with its
//!   frames as a fan of overlapping cards on a gentle wave that spreads side by side on hover
//!   (kept spread by a click on its empty area); a click on a card takes the frame out of the
//!   group or puts it back (`merge.setFrames`), a double-click opens it; then the merged photos,
//!   then the groups sent back (restore.). Groups are picked by their header (click, ⌘-click,
//!   ⇧-click, all. / none., ⌘A), which picks their frames (`library.select`);
//! * the kind tag on thumbnails ([`tag`]: `hdr.`, `panorama.`, `focus.stack.`, `hdr.panorama.`).
//!
//! The sidebar's `merges.` rows are drawn by `panels::left` (under each shoot folder — its camera
//! subfolders are one pool — and collection that has groups), from [`counts`].

use std::sync::Arc;

use egui::{Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::{MergeType, PhotoId};
use lightcraft_engine::merge_find::{self, Confidence, Counts, GroupState, MergeGroup, Scope};
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

/// The merges. scope matching what the sidebar shows, as Develop follows it: a folder → its
/// merges and those of every folder in it; a collection → its; anything else → all.
pub fn scope_of_source(app: &LightcraftApp) -> Scope {
    use lightcraft_engine::LibrarySource as S;
    let s = &app.session;
    match s.source {
        S::LibraryFolder => s.library_folder.clone().map_or(Scope::All, |path| Scope::Folder { path, deep: true }),
        S::Folder => s.browse.as_ref().map_or(Scope::All, |b| Scope::Folder { path: b.path.clone(), deep: true }),
        S::Album(id) => Scope::Album { id },
        S::Merges => s.merges_scope.clone().unwrap_or_default(),
        _ => Scope::All,
    }
}

/// Show the merges. view of `scope` (`library.source {kind: merges, …}`).
pub fn show_scope(app: &mut LightcraftApp, scope: &Scope) -> Result<Value, String> {
    let p = match scope {
        Scope::Folder { path, deep } => json!({"kind": "merges", "path": path, "deep": deep}),
        Scope::Album { id } => json!({"kind": "merges", "id": id.0}),
        Scope::All => json!({"kind": "merges"}),
    };
    app.run("library.source", p)
}

/// Per frame: land a finder run, start the automatic one, feed the merge worker.
pub fn tick(app: &mut LightcraftApp, ctx: &egui::Context) {
    // the merge. tab follows the sidebar: a folder / collection picked there shows its merges
    if app.merge_tab && app.session.source != lightcraft_engine::LibrarySource::Merges {
        let scope = scope_of_source(app);
        if show_scope(app, &scope).is_err() {
            app.merge_tab = false;
        }
    }
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
/// the setting, `merge.cardSize {size? | step?}` the cards' size (smaller. / bigger.).
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
        // smaller. / bigger.: the cards' size (the grid's thumbnail size, as the bottom bar's slider)
        "merge.cardSize" => {
            let size = match (p.get("size").and_then(Value::as_f64), p.get("step").and_then(Value::as_f64)) {
                (Some(s), _) => s as f32,
                (None, Some(d)) => app.ui.thumb_size + d as f32,
                (None, None) => return Some(Ok(json!({"size": app.ui.thumb_size}))),
            };
            if !size.is_finite() {
                return Some(Err("merge.cardSize: size is a number of points (90–480)".into()));
            }
            app.ui.thumb_size = size.clamp(90.0, 480.0);
            Some(Ok(json!({"size": app.ui.thumb_size})))
        }
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
        if frames.len() < 2 {
            return Err(format!("{}: put two frames or more in the group to merge it", g.kind.tag()));
        }
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

/// The open groups the selection touches (any of their frames picked).
fn picked_groups(app: &LightcraftApp) -> Vec<MergeGroup> {
    let sel = &app.session.selection.ids;
    merge_find::groups(&app.session.catalog)
        .into_iter()
        .filter(|g| g.state == GroupState::Open && g.all_frames().iter().any(|p| sel.contains(p)))
        .collect()
}

/// The merged photos picked.
fn picked_results(app: &LightcraftApp) -> Vec<u64> {
    let merged: Vec<PhotoId> =
        merge_find::groups(&app.session.catalog).into_iter().filter(|g| g.state == GroupState::Merged).filter_map(|g| g.result).collect();
    app.session.selection.ids.iter().filter(|i| merged.contains(i)).map(|i| i.0).collect()
}

/// Height of the merges. bar.
pub const BAR_H: f32 = 40.0;

/// The bar above a merges. view: find. (finding… while it looks), all. / none. (every group and
/// merged photo, or nothing), merge. / send.back. for the picked groups, both. / merges.only. /
/// unmerge. for the picked merged photos, smaller. / bigger. for the cards.
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
        "Look for merges here: panoramas (panorama.), brackets (hdr.), focus stacks (focus.stack.), HDR panoramas (hdr.panorama.)"
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
        let _ = app.run("library.select", json!({"ids": []}));
    }
    gap(&mut row);
    let busy = app.merge.final_task.as_ref().is_some_and(|t| t.group.is_some());
    let waiting = app.merge.group_queue.len();
    let merge_label = if busy { format!("merging… {}", waiting + 1) } else { "merge.".to_string() };
    if crate::widgets::text_button(&mut row, "mergesMerge", &merge_label, busy)
        .on_hover_text(crate::i18n::tr("Merge the picked groups, each as its kind, with the frames that are in it. The originals then leave the folder — not deleted — and the merged photo shows here and where they were"))
        .clicked()
    {
        let ids: Vec<String> = picked_groups(app).into_iter().map(|g| g.id).collect();
        if ids.is_empty() {
            app.toast(&ctx, crate::i18n::tr("Pick a group first: click its header"));
        } else if let Err(e) = app.run("merge.mergeGroups", json!({"ids": ids})) {
            app.toast(&ctx, e);
        }
    }
    if crate::widgets::text_button(&mut row, "mergesSendBack", "send.back.", false)
        .on_hover_text(crate::i18n::tr(
            "Not a merge: the picked groups go back to their folder and are never suggested again (restore. under sent back. brings one back)",
        ))
        .clicked()
    {
        let ids: Vec<String> = picked_groups(app).into_iter().map(|g| g.id).collect();
        if ids.is_empty() {
            app.toast(&ctx, crate::i18n::tr("Pick a group first: click its header"));
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
    gap(&mut row);
    for (id, label, tip, step) in [
        ("mergesSmaller", "smaller.", "Smaller cards (the grid's size)", -40.0f32),
        ("mergesBigger", "bigger.", "Bigger cards (the grid's size)", 40.0),
    ] {
        if crate::widgets::text_button(&mut row, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked()
            && let Err(e) = app.run("merge.cardSize", json!({"step": step}))
        {
            app.toast(&ctx, e);
        }
    }
}

/// Room between the bar's groups of buttons.
fn gap(ui: &mut egui::Ui) {
    ui.add_space(8.0);
}

// ------------------------------------------------------------------------------------- the view

/// A group as the view draws it (worked out once per catalog revision).
#[derive(Clone, Debug)]
struct GroupView {
    g: MergeGroup,
    /// Its frames still in the library: in the group and taken out of it.
    frames: Vec<PhotoId>,
    aspects: Vec<f32>,
    cameras: Vec<String>,
    subfolders: Vec<String>,
    reasons: String,
}

#[derive(Clone, Debug, Default)]
struct ViewData {
    open: Vec<GroupView>,
    merged: Vec<PhotoId>,
    merged_aspects: Vec<f32>,
    sent: Vec<GroupView>,
}

fn group_view(cat: &lightcraft_catalog::Catalog, scope: &merge_find::Scope, g: MergeGroup, frames: Vec<PhotoId>) -> GroupView {
    let mut cameras: Vec<String> = Vec::new();
    let mut subfolders: Vec<String> = Vec::new();
    for p in frames.iter().filter_map(|id| cat.photo(*id)) {
        let c = lightcraft_engine::merge_find::short_camera(&p.meta.camera);
        if !c.is_empty() && !cameras.contains(&c) {
            cameras.push(c);
        }
        let f = lightcraft_catalog::local::folder_of(p).unwrap_or_default();
        let s = merge_find::subfolder(&f, scope.root());
        if !s.is_empty() && !subfolders.contains(&s) {
            subfolders.push(s);
        }
    }
    let reasons = if g.reasons.is_empty() { merge_find::describe(cat, &g) } else { g.reasons.clone() };
    GroupView { aspects: crate::panels::grid::aspects(cat, &frames), g, frames, cameras, subfolders, reasons: reasons.join("  ·  ") }
}

/// The view's groups, once per catalog revision and scope.
fn view_data(app: &LightcraftApp, ctx: &egui::Context) -> Arc<ViewData> {
    let scope = app.session.merges_scope.clone().unwrap_or_default();
    let key = egui::Id::new("merges-view-data");
    let rev = (app.session.catalog.revision, format!("{scope:?}"));
    if let Some((r, d)) = ctx.data(|d| d.get_temp::<((u64, String), Arc<ViewData>)>(key))
        && r == rev
    {
        return d;
    }
    let cat = &app.session.catalog;
    let mut d = ViewData::default();
    for run in merge_find::view(cat, &scope) {
        match run.group.as_deref().and_then(|id| merge_find::group(cat, id)) {
            Some(g) => d.open.push(group_view(cat, &scope, g, run.ids)),
            None => {
                d.merged_aspects = crate::panels::grid::aspects(cat, &run.ids);
                d.merged = run.ids;
            }
        }
    }
    for g in merge_find::sent_back(cat, &scope) {
        let frames = g.live_all(cat);
        d.sent.push(group_view(cat, &scope, g, frames));
    }
    let d = Arc::new(d);
    ctx.data_mut(|m| m.insert_temp(key, (rev, d.clone())));
    d
}

/// Where a group's cards go: stacked (a fan of overlapping cards on a gentle wave) and spread
/// (side by side, wrapping), relative to the wave area's top-left.
struct Wave {
    stack: Vec<Rect>,
    spread: Vec<Rect>,
    stack_h: f32,
    spread_h: f32,
}

/// Room between spread cards (and their rows).
const CARD_GAP: f32 = 12.0;

fn wave_layout(aspects: &[f32], card_h: f32, width: f32) -> Wave {
    let n = aspects.len();
    let width = width.max(40.0);
    let widths: Vec<f32> = aspects.iter().map(|a| (card_h * a.clamp(0.4, 2.5)).min(width)).collect();
    let amp = (card_h * 0.06).round();
    // stacked: each card shows a slice of the one under it
    let mean_w = widths.iter().sum::<f32>() / n.max(1) as f32;
    let last_w = widths.last().copied().unwrap_or(0.0);
    let mut step = (mean_w * 0.3).max(24.0);
    if n > 1 && step * (n - 1) as f32 + last_w > width {
        step = ((width - last_w) / (n - 1) as f32).max(8.0);
    }
    let stack: Vec<Rect> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let x = (i as f32 * step).min((width - w).max(0.0));
            let y = amp + amp * (i as f32 * 0.85).sin();
            Rect::from_min_size(pos2(x, y), vec2(*w, card_h))
        })
        .collect();
    // spread: side by side in one row, smaller when they don't fit; below half their size, in
    // even rows (5 frames: 3 + 2) as big as the rows let them be
    let fit = |k: usize| {
        let total: f32 = mean_w * k as f32;
        let gaps = CARD_GAP * k.saturating_sub(1) as f32;
        if total <= 0.0 { 1.0 } else { ((width - gaps) / total).min(1.0) }
    };
    let mut per_row = n.max(1);
    if fit(per_row) < 0.5 {
        let most = (((width + CARD_GAP) / (mean_w * 0.5 + CARD_GAP)).floor() as usize).max(1);
        let rows = n.div_ceil(most).max(1);
        per_row = n.div_ceil(rows).max(1);
    }
    let s = fit(per_row).max(0.2);
    let h = (card_h * s).round();
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut spread = Vec::with_capacity(n);
    for (i, w) in widths.iter().enumerate() {
        let w = (w * s).min(width);
        if i > 0 && (i % per_row == 0 || x + w > width + 0.5) {
            x = 0.0;
            y += h + CARD_GAP;
        }
        spread.push(Rect::from_min_size(pos2(x, y), vec2(w, h)));
        x += w + CARD_GAP;
    }
    Wave { stack, spread, stack_h: card_h + 2.0 * amp, spread_h: if n == 0 { 0.0 } else { y + h } }
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

/// The cards' height: from the grid's size (the bottom bar's slider, smaller. / bigger.).
fn card_h(app: &LightcraftApp) -> f32 {
    (app.ui.thumb_size * 1.15).clamp(120.0, 440.0)
}

/// The fold of a group (its triangle: the group shrinks to its header) and of its spread
/// (a click on the group's empty area: its cards stay side by side).
fn fold_id(gid: &str) -> String {
    format!("merges.group:{gid}")
}
fn spread_id(gid: &str) -> String {
    format!("merges.spread:{gid}")
}

/// Panel padding, the header's height, the gaps between a group's parts and between groups.
const PAD: f32 = 12.0;
const HEAD_H: f32 = 28.0;
const BLOCK_GAP: f32 = 12.0;
const SECTION_H: f32 = 32.0;

/// A group's pieces: header, reasons and wave, top to bottom (relative to the block's top).
struct Block {
    height: f32,
    reasons_top: f32,
    wave_top: f32,
    wave_h: f32,
    t: f32,
}

fn block(open: bool, reasons_h: f32, wave: &Wave, t: f32) -> Block {
    let reasons_top = PAD + HEAD_H + 6.0;
    if !open {
        return Block { height: PAD + HEAD_H + PAD, reasons_top, wave_top: 0.0, wave_h: 0.0, t };
    }
    let wave_top = reasons_top + reasons_h + BLOCK_GAP;
    // spreading never shrinks it (a pointer near its bottom would leave, and it would fold back)
    let wave_h = (wave.stack_h + (wave.spread_h - wave.stack_h) * t).max(wave.stack_h);
    Block { height: wave_top + wave_h + PAD + 2.0, reasons_top, wave_top, wave_h, t }
}

/// A click on a card waits this long for a second click (a double-click opens the photo).
const DOUBLE_WAIT: f64 = 0.32;

/// The merges. view (Bryan, 2026-11-24: not the grid — "a stacked wave that shows all the
/// photos"): each open group is a panel with its header (kind, confidence, camera, subfolder,
/// frames, merge. / send.back.), its reasons and its frames as a fan of overlapping cards. Hover
/// spreads the fan side by side; a click on the group's empty area keeps it spread. A click on a
/// card takes the frame out of the group or puts it back (`merge.setFrames`); a double-click opens
/// it in the loupe. Then the merged photos, then the groups sent back (restore.).
pub fn view(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let data = view_data(app, &ctx);
    if data.open.is_empty() && data.merged.is_empty() && data.sent.is_empty() {
        let body = if app.ui.settings.merge_find == FindMode::Auto {
            "Panoramas, brackets and focus stacks show up here as LightCraft finds them. find. looks now"
        } else {
            "Click find. to look here for panoramas (panorama.), brackets (hdr.) and focus stacks (focus.stack.)"
        };
        crate::panels::empty_message(ui, ui.max_rect(), "No merges here", body);
        return;
    }
    run_pending(app, &ctx);
    let ppp = ctx.pixels_per_point();
    let ch = card_h(app);
    let outer = ui.available_width();
    let panel_w = (outer - 32.0).max(120.0);
    let inner_w = panel_w - 2.0 * PAD;
    let hovered_before: Option<String> = ctx.data(|d| d.get_temp::<Option<String>>(egui::Id::new("merges-hover"))).flatten();
    // lay out every block first (the scroll area needs the height)
    let mut blocks: Vec<(f32, Block, Wave, Arc<egui::Galley>)> = Vec::new();
    let mut y = 12.0f32;
    for gv in &data.open {
        let open = crate::folds::is_open(&ctx, &fold_id(&gv.g.id));
        let pinned = crate::folds::is_open_or(&ctx, &spread_id(&gv.g.id), false);
        let target = if pinned || hovered_before.as_deref() == Some(gv.g.id.as_str()) { 1.0 } else { 0.0 };
        let tt = ctx.animate_value_with_time(egui::Id::new(("merges-wave", gv.g.id.as_str())), target, 0.2);
        let wave = wave_layout(&gv.aspects, ch, inner_w);
        let reasons = ui.painter().layout(gv.reasons.clone(), t.font(11.5), t.text_dim, inner_w);
        let b = block(open, if open { reasons.size().y } else { 0.0 }, &wave, tt);
        let h = b.height;
        blocks.push((y, b, wave, reasons));
        y += h + BLOCK_GAP;
    }
    let merged_top = y;
    let merged_open = crate::folds::is_open(&ctx, "merges.merged");
    let small_h = (ch * 0.75).round();
    let merged_cells = if data.merged.is_empty() { Vec::new() } else { wave_layout(&data.merged_aspects, small_h, inner_w).spread };
    if !data.merged.is_empty() {
        y += SECTION_H + 8.0;
        if merged_open {
            y += merged_cells.iter().map(|r| r.bottom()).fold(0.0, f32::max) + BLOCK_GAP;
        }
        y += BLOCK_GAP;
    }
    let sent_top = y;
    let sent_open = crate::folds::is_open_or(&ctx, "merges.sentBack", false);
    let sent_h = (ch * 0.45).round();
    let sent_waves: Vec<Wave> = data.sent.iter().map(|g| wave_layout(&g.aspects, sent_h, inner_w)).collect();
    if !data.sent.is_empty() {
        y += SECTION_H + 8.0;
        if sent_open {
            for w in &sent_waves {
                y += PAD + HEAD_H + 8.0 + w.stack_h + PAD + BLOCK_GAP;
            }
        }
    }
    let total_h = y + 16.0;
    let mut hovered_now: Option<String> = None;
    let mut seen = std::collections::HashSet::new();
    let mut animating = false;
    egui::ScrollArea::vertical().id_salt("merges-scroll").auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), total_h), Sense::hover());
        let origin = area.min + vec2(16.0, 0.0);
        let near = viewport.expand2(vec2(0.0, viewport.height() * 0.5));
        let order: Vec<(String, Vec<PhotoId>)> = data.open.iter().map(|g| (g.g.id.clone(), g.frames.clone())).collect();
        for (gv, (top, b, wave, reasons)) in data.open.iter().zip(&blocks) {
            if b.t > 0.001 && b.t < 0.999 {
                animating = true;
            }
            let rect = Rect::from_min_size(pos2(0.0, *top), vec2(panel_w, b.height));
            if !rect.intersects(near) {
                continue;
            }
            let r = rect.translate(origin.to_vec2());
            if group_block(app, ui, gv, r, b, wave, reasons.clone(), &order, ppp, &mut seen) {
                hovered_now = Some(gv.g.id.clone());
            }
        }
        if !data.merged.is_empty() {
            let r = Rect::from_min_size(pos2(0.0, merged_top), vec2(panel_w, SECTION_H)).translate(origin.to_vec2());
            let n = data.merged.len();
            section_header(
                ui,
                "merges.merged",
                "group:merges:merged",
                "merged.",
                &format!("{n} photo{}", if n == 1 { "" } else { "s" }),
                r,
                merged_open,
            );
            if merged_open {
                let base = r.left_bottom() + vec2(PAD, 8.0);
                for (id, cell) in data.merged.iter().zip(&merged_cells) {
                    let cr = cell.translate(base.to_vec2());
                    if cr.intersects(ui.clip_rect().expand(200.0)) {
                        merged_card(app, ui, *id, cr, ppp, &mut seen);
                    }
                }
            }
        }
        if !data.sent.is_empty() {
            let r = Rect::from_min_size(pos2(0.0, sent_top), vec2(panel_w, SECTION_H)).translate(origin.to_vec2());
            let n = data.sent.len();
            section_header(
                ui,
                "merges.sentBack",
                "group:merges:sentBack",
                "sent back.",
                &format!("{n} group{}", if n == 1 { "" } else { "s" }),
                r,
                sent_open,
            );
            if sent_open {
                let mut sy = r.bottom() + 8.0;
                for (gv, w) in data.sent.iter().zip(&sent_waves) {
                    let h = PAD + HEAD_H + 8.0 + w.stack_h + PAD;
                    let br = Rect::from_min_size(pos2(r.left(), sy), vec2(panel_w, h));
                    sent_block(app, ui, gv, br, w, ppp, &mut seen);
                    sy += h + BLOCK_GAP;
                }
            }
        }
        crate::widgets::reveal_in_scroll(ui);
    });
    if hovered_now != hovered_before {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("merges-hover"), hovered_now));
        ctx.request_repaint();
    }
    if animating {
        ctx.request_repaint();
    }
    app.renderer.evict_thumbs(&seen, 600);
}

/// A foldable section header of the view (`merged.`, `sent back.`): triangle, title, count.
fn section_header(ui: &mut egui::Ui, fold: &str, reg: &str, title: &str, count: &str, r: Rect, open: bool) {
    let t = Tokens::get(ui.ctx());
    let resp = ui.interact(r, egui::Id::new(("merges-section", fold)), Sense::click());
    register(ui.ctx(), reg, r);
    register(ui.ctx(), format!("fold:{fold}"), r);
    let open = if resp.clicked() {
        crate::folds::set(ui.ctx(), fold, !open);
        !open
    } else {
        open
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    let p = ui.painter();
    crate::folds::paint_triangle(ui, pos2(r.left() + PAD + 6.0, r.center().y), open, if resp.hovered() { t.text } else { t.text_dim });
    let g = p.layout_no_wrap(crate::i18n::tr(title).to_string(), t.semibold(15.0), t.text);
    let gw = g.size().x;
    let x = r.left() + PAD + 20.0;
    p.galley(pos2(x, r.center().y - g.size().y / 2.0), g, t.text);
    p.text(pos2(x + gw + 10.0, r.center().y), Align2::LEFT_CENTER, format!("· {count}"), t.font(12.5), t.text_dim);
    p.hline(r.x_range(), r.bottom(), Stroke::new(1.0, t.divider));
}

/// A small pill that says something (confidence, camera, subfolder): never a greyed control.
fn chip(ui: &mut egui::Ui, id: &str, label: &str, dot: Option<Color32>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let g = ui.painter().layout_no_wrap(label.to_string(), t.font(11.5), t.text_label);
    let dot_w = if dot.is_some() { 12.0 } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 20.0 + dot_w, 24.0), Sense::hover());
    register(ui.ctx(), format!("chip:{id}"), r);
    let p = ui.painter();
    p.rect(r, 12.0, t.inset, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
    let mut x = r.left() + 10.0;
    if let Some(c) = dot {
        p.circle_filled(pos2(x + 3.0, r.center().y), 3.5, c);
        x += dot_w;
    }
    p.galley(pos2(x, r.center().y - g.size().y / 2.0), g, t.text_label);
    resp
}

fn chip_width(ui: &egui::Ui, label: &str, dot: bool) -> f32 {
    let t = Tokens::get(ui.ctx());
    ui.painter().layout_no_wrap(label.to_string(), t.font(11.5), t.text_label).size().x + 20.0 + if dot { 12.0 } else { 0.0 }
}

fn confidence_color(c: Confidence) -> Color32 {
    match c {
        Confidence::Sure => Color32::from_rgb(98, 186, 110),
        Confidence::Likely => Color32::from_rgb(222, 176, 72),
        Confidence::Maybe => Color32::from_rgb(120, 156, 220),
    }
}

/// One open group: its panel, header, reasons and wave of cards. Returns whether the pointer is
/// over its wave (it spreads).
#[allow(clippy::too_many_arguments)]
fn group_block(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    gv: &GroupView,
    r: Rect,
    b: &Block,
    wave: &Wave,
    reasons: Arc<egui::Galley>,
    order: &[(String, Vec<PhotoId>)],
    ppp: f32,
    seen: &mut std::collections::HashSet<PhotoId>,
) -> bool {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let g = &gv.g;
    let picked = gv.frames.iter().any(|f| app.session.selection.contains(*f));
    let p = ui.painter().clone();
    p.rect(r, 8.0, t.cell, Stroke::new(if picked { 2.0 } else { 1.0 }, if picked { t.accent } else { t.divider }), egui::StrokeKind::Inside);
    // the header: a click on it picks the group (⌘: add / remove, ⇧: the groups from the last one)
    let head = Rect::from_min_size(r.min + vec2(PAD, PAD), vec2(r.width() - 2.0 * PAD, HEAD_H));
    let head_resp = ui.interact(head, egui::Id::new(("merges-head", g.id.as_str())), Sense::click());
    register(&ctx, format!("group:merges:group:{}", g.id), head);
    if head_resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        pick_group(app, &ctx, &g.id, &gv.frames, order, m);
    }
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    // the triangle folds the group to its header
    let open = crate::folds::triangle(&mut row, &fold_id(&g.id), 24.0);
    let kind = crate::widgets::text_button(&mut row, &format!("mergesKind-{}", g.id), g.kind.tag(), false)
        .on_hover_text(crate::i18n::tr("What it merges as: click to merge it as another kind"));
    egui::Popup::menu(&kind).show(|ui| {
        for k in MergeType::ALL {
            let what = match k {
                MergeType::Hdr => "hdr. — brackets → one photo with the whole range",
                MergeType::Pano => "panorama. — overlapping photos → one wide photo",
                MergeType::Stack => "focus.stack. — focused near to far → one photo sharp throughout",
                MergeType::HdrPano => "hdr.panorama. — a bracket at each position → merged, then stitched",
            };
            if ui.selectable_label(k == g.kind, crate::i18n::tr(what)).clicked() {
                let _ = app.run("merge.setKind", json!({"id": g.id, "kind": k.id()}));
            }
        }
    });
    let conf = g.confidence();
    chip(&mut row, &format!("mergesConfidence-{}", g.id), conf.tag(), Some(confidence_color(conf))).on_hover_text(crate::i18n::tr(match conf {
        Confidence::Sure => "sure.: everything says it is this merge",
        Confidence::Likely => "likely.: most things say so",
        Confidence::Maybe => "maybe.: a borderline match — look before merging",
    }));
    // merge. / send.back. at the right; the chips give way to them
    let queued = app.merge.group_queue.iter().any(|q| q.group == g.id);
    let running = app.merge.final_task.as_ref().and_then(|t| t.group.as_ref()).is_some_and(|(q, _)| *q == g.id);
    let merge_label = if running {
        "merging…"
    } else if queued {
        "waiting…"
    } else {
        "merge."
    };
    let tb = |label: &str| chip_width(&row, label, false).max(37.0);
    let right_w = tb(merge_label) + tb("send.back.") + 8.0 + 8.0;
    let in_group = gv.frames.iter().filter(|f| g.photos.contains(f)).count();
    let count = if in_group == gv.frames.len() { format!("{} frames", gv.frames.len()) } else { format!("{in_group} of {} frames", gv.frames.len()) };
    let count_w = row.painter().layout_no_wrap(count.clone(), t.font(12.5), t.text_dim).size().x;
    let limit = head.right() - right_w - count_w - 8.0;
    let mut chips: Vec<(String, String)> = gv.cameras.iter().map(|c| (format!("mergesCamera-{}", g.id), c.clone())).collect();
    chips.extend(gv.subfolders.iter().map(|s| (format!("mergesSubfolder-{}", g.id), s.clone())));
    for (k, (id, label)) in chips.iter().enumerate() {
        let w = chip_width(&row, label, false);
        if row.cursor().left() + w > limit {
            break;
        }
        let tip = if id.starts_with("mergesCamera") { "The camera" } else { "The subfolder of the shoot" };
        chip(&mut row, &format!("{id}-{k}"), label, None).on_hover_text(crate::i18n::tr(tip));
    }
    if row.cursor().left() + count_w <= head.right() - right_w {
        let (cr, _) = row.allocate_exact_size(vec2(count_w, 24.0), Sense::hover());
        row.painter().text(cr.left_center(), Align2::LEFT_CENTER, count, t.font(12.5), t.text_dim);
    }
    let mut right = row.new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.spacing_mut().item_spacing.x = 8.0;
    if crate::widgets::text_button(&mut right, &format!("mergesGroupSendBack-{}", g.id), "send.back.", false)
        .on_hover_text(crate::i18n::tr(
            "Not a merge: its photos stay in their folder and it is never suggested again (restore. under sent back. brings it back)",
        ))
        .clicked()
        && let Err(e) = app.run("merge.dismiss", json!({"ids": [g.id]}))
    {
        app.toast(&ctx, e);
    }
    if crate::widgets::text_button(&mut right, &format!("mergesGroupMerge-{}", g.id), merge_label, running || queued)
        .on_hover_text(crate::i18n::tr("Merge this group, with the frames that are in it"))
        .clicked()
        && !running
        && !queued
        && let Err(e) = app.run("merge.mergeGroups", json!({"ids": [g.id]}))
    {
        app.toast(&ctx, e);
    }
    if !open {
        return false;
    }
    p.galley(pos2(r.left() + PAD, r.top() + b.reasons_top), reasons, t.text_dim);
    // the wave: its empty area keeps it spread (or lets it fold back); hover spreads it
    let wr = Rect::from_min_size(pos2(r.left() + PAD, r.top() + b.wave_top), vec2(r.width() - 2.0 * PAD, b.wave_h));
    let wave_resp = ui.interact(wr, egui::Id::new(("merges-wave-area", g.id.as_str())), Sense::click());
    register(&ctx, format!("mergeWave:{}", g.id), wr);
    if wave_resp.clicked() {
        let id = spread_id(&g.id);
        let on = crate::folds::is_open_or(&ctx, &id, false);
        crate::folds::set(&ctx, &id, !on);
    }
    let hovered = ui.rect_contains_pointer(wr);
    for (i, pid) in gv.frames.iter().enumerate() {
        let (Some(a), Some(s)) = (wave.stack.get(i), wave.spread.get(i)) else { continue };
        let cr = lerp_rect(*a, *s, b.t).translate(wr.min.to_vec2());
        seen.insert(*pid);
        frame_card(app, ui, &g.id, *pid, g.photos.contains(pid), cr, ppp);
    }
    hovered
}

/// A frame of a group as a card: dimmed with a mark when taken out of the group (still a full
/// control: a click puts it back).
fn frame_card(app: &mut LightcraftApp, ui: &mut egui::Ui, gid: &str, pid: PhotoId, in_group: bool, r: Rect, ppp: f32) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let resp = ui.interact(r, egui::Id::new(("merge-card", gid, pid.0)), Sense::click());
    register(&ctx, format!("mergeCard:{gid}:{}", pid.0), r);
    let p = ui.painter();
    p.rect_filled(r.translate(vec2(2.0, 3.0)), 4.0, Color32::from_black_alpha(110));
    crate::panels::grid::request_thumb(app, pid, crate::panels::grid::thumb_px(r.width().max(r.height()), ppp), 10);
    let tint = if in_group { Color32::WHITE } else { Color32::from_gray(80) };
    card_image(app, p, pid, r, tint);
    let edge = if resp.hovered() { Stroke::new(2.0, t.text) } else { Stroke::new(1.0, Color32::from_black_alpha(160)) };
    p.rect_stroke(r, 4.0, edge, egui::StrokeKind::Inside);
    if !in_group {
        // the mark: a circle with a minus, at the card's top-left corner
        let c = r.left_top() + vec2(16.0, 16.0);
        p.circle(c, 9.0, Color32::from_black_alpha(200), Stroke::new(1.5, Color32::WHITE));
        p.line_segment([c - vec2(4.5, 0.0), c + vec2(4.5, 0.0)], Stroke::new(2.0, Color32::WHITE));
    }
    let name = app.session.catalog.photo(pid).map(|p| format!("{}\n{}", p.file_name, merge_find::exposure_text(&p.meta))).unwrap_or_default();
    let what = if in_group { "click: take it out of the group" } else { "taken out — click: put it back" };
    let resp = resp.on_hover_text(format!("{name}\n{}\n{}", crate::i18n::tr(what), crate::i18n::tr("double-click: open it")));
    if resp.double_clicked() {
        ctx.data_mut(|d| d.remove::<(String, u64, f64)>(egui::Id::new("merges-pending")));
        let _ = app.run("library.select", json!({"ids": [pid.0]}));
        let _ = app.run("view.detail", json!({}));
    } else if resp.clicked() {
        let now = ctx.input(|i| i.time);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("merges-pending"), (gid.to_string(), pid.0, now)));
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(DOUBLE_WAIT + 0.02));
    }
}

/// A card's picture (its thumbnail, or a placeholder while it renders), `tint`ed.
fn card_image(app: &LightcraftApp, p: &egui::Painter, id: PhotoId, r: Rect, tint: Color32) {
    match app.renderer.thumb(id) {
        Some(tex) => {
            p.image(tex.tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => {
            p.rect_filled(r, 4.0, Color32::from_gray(46).linear_multiply(f32::from(tint.r()) / 255.0 * 0.5 + 0.5));
        }
    }
}

/// A card clicked once: after the double-click wait, the frame goes out of its group or back in.
fn run_pending(app: &mut LightcraftApp, ctx: &egui::Context) {
    let key = egui::Id::new("merges-pending");
    let Some((gid, pid, at)) = ctx.data(|d| d.get_temp::<(String, u64, f64)>(key)) else { return };
    let now = ctx.input(|i| i.time);
    if now - at < DOUBLE_WAIT {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(DOUBLE_WAIT - (now - at) + 0.02));
        return;
    }
    ctx.data_mut(|d| d.remove::<(String, u64, f64)>(key));
    toggle_frame(app, ctx, &gid, PhotoId(pid));
}

/// Take frame `pid` out of group `gid`, or put it back (`merge.setFrames`).
pub fn toggle_frame(app: &mut LightcraftApp, ctx: &egui::Context, gid: &str, pid: PhotoId) {
    let Some(g) = merge_find::group(&app.session.catalog, gid) else { return };
    let mut photos: Vec<u64> = g.photos.iter().map(|p| p.0).collect();
    if let Some(i) = photos.iter().position(|p| *p == pid.0) {
        photos.remove(i);
    } else {
        photos.push(pid.0);
    }
    if let Err(e) = app.run("merge.setFrames", json!({"id": gid, "photos": photos})) {
        app.toast(ctx, e);
    }
}

/// A click on a group's header: its frames are picked (`library.select`); ⌘: added, or taken
/// away when it was picked; ⇧: every group from the one clicked last.
fn pick_group(app: &mut LightcraftApp, ctx: &egui::Context, gid: &str, frames: &[PhotoId], order: &[(String, Vec<PhotoId>)], m: egui::Modifiers) {
    let anchor_key = egui::Id::new("merges-anchor");
    let ids = |v: &[PhotoId]| v.iter().map(|p| p.0).collect::<Vec<_>>();
    let r = if m.shift {
        let anchor: Option<String> = ctx.data(|d| d.get_temp(anchor_key));
        let at = |id: &str| order.iter().position(|(g, _)| g == id);
        match (anchor.as_deref().and_then(at), at(gid)) {
            (Some(a), Some(b)) => {
                let (lo, hi) = (a.min(b), a.max(b));
                let range: Vec<u64> = order.get(lo..=hi).unwrap_or(&[]).iter().flat_map(|(_, f)| ids(f)).collect();
                app.run("library.select", json!({"ids": range}))
            }
            _ => app.run("library.select", json!({"ids": ids(frames)})),
        }
    } else if m.command {
        let picked: Vec<u64> = frames.iter().filter(|f| app.session.selection.contains(**f)).map(|p| p.0).collect();
        if picked.is_empty() {
            app.run("library.select", json!({"ids": ids(frames), "mode": "add"}))
        } else {
            app.run("library.select", json!({"ids": picked, "mode": "toggle"}))
        }
    } else {
        app.run("library.select", json!({"ids": ids(frames)}))
    };
    if let Err(e) = r {
        app.toast(ctx, e);
    }
    if !m.shift {
        ctx.data_mut(|d| d.insert_temp(anchor_key, gid.to_string()));
    }
}

/// A merged photo: its card with its kind tag; click picks it (⌘ / ⇧ as in the grid), a
/// double-click opens it.
fn merged_card(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId, r: Rect, ppp: f32, seen: &mut std::collections::HashSet<PhotoId>) {
    let t = Tokens::get(ui.ctx());
    seen.insert(id);
    let resp = ui.interact(r, egui::Id::new(("merged-card", id.0)), Sense::click());
    register(ui.ctx(), format!("thumb:{}", id.0), r);
    crate::panels::grid::request_thumb(app, id, crate::panels::grid::thumb_px(r.width().max(r.height()), ppp), 10);
    let p = ui.painter().clone();
    p.rect_filled(r.translate(vec2(2.0, 3.0)), 4.0, Color32::from_black_alpha(110));
    card_image(app, &p, id, r, Color32::WHITE);
    let selected = app.session.selection.contains(id);
    let edge = if selected {
        Stroke::new(2.0, Color32::WHITE)
    } else if resp.hovered() {
        Stroke::new(2.0, t.text_dim)
    } else {
        Stroke::new(1.0, Color32::from_black_alpha(160))
    };
    p.rect_stroke(r, 4.0, edge, egui::StrokeKind::Inside);
    if let Some(kind) = app.session.catalog.photo(id).and_then(|p| p.merge_type()) {
        let br = tag(&p, &t, r.right_top() + vec2(-6.0, 6.0), kind, false, true);
        register(ui.ctx(), format!("badge:merge:{}", id.0), br);
    }
    if resp.double_clicked() {
        let _ = app.run("library.select", json!({"ids": [id.0]}));
        let _ = app.run("view.detail", json!({}));
    } else if resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        let mode = if m.shift {
            "range"
        } else if m.command {
            "toggle"
        } else {
            "replace"
        };
        let _ = app.run("library.select", json!({"ids": [id.0], "mode": mode}));
    }
}

/// A group sent back: its kind, confidence and frames, restore. at the right, and its cards
/// (small, stacked).
fn sent_block(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    gv: &GroupView,
    r: Rect,
    wave: &Wave,
    ppp: f32,
    seen: &mut std::collections::HashSet<PhotoId>,
) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let g = &gv.g;
    ui.painter().rect(r, 8.0, t.cell, Stroke::new(1.0, t.divider), egui::StrokeKind::Inside);
    let head = Rect::from_min_size(r.min + vec2(PAD, PAD), vec2(r.width() - 2.0 * PAD, HEAD_H));
    register(&ctx, format!("group:merges:sent:{}", g.id), head);
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    chip(&mut row, &format!("mergesSentKind-{}", g.id), g.kind.tag(), None);
    let conf = g.confidence();
    chip(&mut row, &format!("mergesSentConfidence-{}", g.id), conf.tag(), Some(confidence_color(conf)));
    let n = gv.frames.len();
    let count = format!("{n} frame{}", if n == 1 { "" } else { "s" });
    let count_w = row.painter().layout_no_wrap(count.clone(), t.font(12.5), t.text_dim).size().x;
    if row.cursor().left() + count_w <= head.right() - 80.0 {
        let (cr, _) = row.allocate_exact_size(vec2(count_w, 24.0), Sense::hover());
        row.painter().text(cr.left_center(), Align2::LEFT_CENTER, count, t.font(12.5), t.text_dim);
    }
    let mut right = row.new_child(egui::UiBuilder::new().max_rect(head).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if crate::widgets::text_button(&mut right, &format!("mergesRestore-{}", g.id), "restore.", false)
        .on_hover_text(crate::i18n::tr("It is a merge after all: back with the groups to review"))
        .clicked()
        && let Err(e) = app.run("merge.restore", json!({"ids": [g.id]}))
    {
        app.toast(&ctx, e);
    }
    let wr = Rect::from_min_size(pos2(r.left() + PAD, head.bottom() + 8.0), vec2(r.width() - 2.0 * PAD, wave.stack_h));
    for (pid, cr) in gv.frames.iter().zip(&wave.stack) {
        let cr = cr.translate(wr.min.to_vec2());
        seen.insert(*pid);
        crate::panels::grid::request_thumb(app, *pid, crate::panels::grid::thumb_px(cr.width().max(cr.height()), ppp), 5);
        let p = ui.painter();
        p.rect_filled(cr.translate(vec2(2.0, 3.0)), 4.0, Color32::from_black_alpha(110));
        card_image(app, p, *pid, cr, Color32::WHITE);
        p.rect_stroke(cr, 4.0, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Inside);
    }
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
    if let Some(a) = t.badge_accent {
        p.rect_stroke(br, h / 2.0, egui::Stroke::new(1.0, a), egui::StrokeKind::Inside);
    }
    p.galley(pos2(br.center().x - g.size().x / 2.0, br.center().y - g.size().y / 2.0), g, Color32::WHITE);
    br
}
