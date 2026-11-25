//! merges. in the app (Bryan, 2026-11-23): "recognize possible photo merges and put those in a
//! separate area … on the side bar"; 2026-11-24: not the grid — each found group is "a stacked
//! wave that shows all the photos". The finding, the groups and what a merge does are the
//! engine's ([`lightcraft_engine::merge_find`]); here:
//!
//! * the automatic finder ([`tick`]): with Settings ▸ merge. ▸ find automatically: `weddings.`
//!   (the default) or `every.folder.` it looks in the background, off the UI thread, at folders
//!   whose photos changed (all of them the first time, then after each import); with `off.` only
//!   when find. is clicked;
//! * `merge.mergeGroups` in the app ([`run_ui`]): the groups go one at a time to the merge worker
//!   ([`crate::merge::start_group`]); the window stays responsive;
//! * the merges. view's bar ([`bar`]: find., the kind. / confidence. / camera. drop-downs with the
//!   group count, all., none., merge., send.back., both., merges.only., unmerge., layout., aspect.,
//!   per.row., smaller., bigger.) and the view itself ([`view`]): a tile per group, per.row. of them
//!   across (Bryan, 2026-11-24: 4 by default) — kind (a menu), confidence, frames, merge. /
//!   send.back., camera and subfolder, reasons — with its frames as a fan of overlapping cards on a
//!   gentle wave that spreads side by side on hover (kept spread by a click on its empty area), or
//!   grid. / strip. inside the tile; focus. is one big group. A click on a card takes the frame out
//!   of the group or puts it back (`merge.setFrames`), a double-click opens it; then the merged
//!   photos, then the groups sent back (restore.). Groups are picked by their header (click,
//!   ⌘-click, ⇧-click, all. / none., ⌘A), which picks their frames (`library.select`);
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

/// When merges are found by themselves (Settings ▸ merge. ▸ find automatically:). The stored
/// names (`auto`, `manual`) are those of ui.json files from before `every` was added.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FindMode {
    /// weddings.: in the background, in folders named like a wedding (after imports, and once
    /// over the library); the others only on find. / recheck.merges.
    #[default]
    Auto,
    /// every.folder.: in the background, in every new or changed folder.
    Every,
    /// off.: only when find. is clicked in a merges. view.
    Manual,
}

impl FindMode {
    /// What `merge.findMode` answers.
    pub fn id(self) -> &'static str {
        match self {
            FindMode::Auto => "auto",
            FindMode::Every => "every",
            FindMode::Manual => "manual",
        }
    }
    /// Found by themselves (weddings. or every.folder.).
    pub fn automatic(self) -> bool {
        self != FindMode::Manual
    }
}

/// How the merges. view lays out each group's frames (the bar's `layout.`, remembered in ui.json).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// waves.: a fan of overlapping cards on a gentle wave, spread side by side on hover.
    #[default]
    Waves,
    /// grid.: every frame of a group in a plain grid.
    Grid,
    /// strip.: one row per group, the frames side by side, scrolling sideways.
    Strip,
    /// focus.: one group at a time, big (← / → or prev. / next. for the others).
    Focus,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Waves, Layout::Grid, Layout::Strip, Layout::Focus];
    pub fn id(self) -> &'static str {
        match self {
            Layout::Waves => "waves",
            Layout::Grid => "grid",
            Layout::Strip => "strip",
            Layout::Focus => "focus",
        }
    }
    pub fn tag(self) -> &'static str {
        match self {
            Layout::Waves => "waves.",
            Layout::Grid => "grid.",
            Layout::Strip => "strip.",
            Layout::Focus => "focus.",
        }
    }
    pub fn parse(s: &str) -> Option<Layout> {
        let s = s.trim().trim_end_matches('.');
        Layout::ALL.into_iter().find(|l| l.id() == s)
    }
}

/// The shape of the cards in the merges. view (the bar's `aspect.`, remembered in ui.json). Only
/// the cards: the photo is fitted inside (letterboxed, never stretched or cropped) and the photos
/// themselves — their crop, settings, merges — never change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardAspect {
    /// original.: each card the shape of its photo.
    #[default]
    #[serde(rename = "original")]
    Original,
    #[serde(rename = "square")]
    Square,
    #[serde(rename = "3:2")]
    ThreeTwo,
    #[serde(rename = "4:5")]
    FourFive,
    #[serde(rename = "16:9")]
    SixteenNine,
}

impl CardAspect {
    pub const ALL: [CardAspect; 5] = [CardAspect::Original, CardAspect::Square, CardAspect::ThreeTwo, CardAspect::FourFive, CardAspect::SixteenNine];
    pub fn id(self) -> &'static str {
        match self {
            CardAspect::Original => "original",
            CardAspect::Square => "square",
            CardAspect::ThreeTwo => "3:2",
            CardAspect::FourFive => "4:5",
            CardAspect::SixteenNine => "16:9",
        }
    }
    pub fn tag(self) -> &'static str {
        match self {
            CardAspect::Original => "original.",
            CardAspect::Square => "square.",
            CardAspect::ThreeTwo => "3:2.",
            CardAspect::FourFive => "4:5.",
            CardAspect::SixteenNine => "16:9.",
        }
    }
    pub fn parse(s: &str) -> Option<CardAspect> {
        let s = s.trim().trim_end_matches('.');
        CardAspect::ALL.into_iter().find(|a| a.id() == s)
    }
    /// Width over height of every card (`None`: each its photo's).
    pub fn ratio(self) -> Option<f32> {
        match self {
            CardAspect::Original => None,
            CardAspect::Square => Some(1.0),
            CardAspect::ThreeTwo => Some(1.5),
            CardAspect::FourFive => Some(0.8),
            CardAspect::SixteenNine => Some(16.0 / 9.0),
        }
    }
    /// The cards' shapes for photos of shapes `aspects`.
    pub fn cards(self, aspects: &[f32]) -> Vec<f32> {
        match self.ratio() {
            Some(r) => vec![r; aspects.len()],
            None => aspects.to_vec(),
        }
    }
}

/// How the merges. view shows the groups on one size of screen (Bryan, 2026-11-24: his MacBook
/// and his big display each keep their own): layout., aspect., per.row. and the kind. /
/// confidence. picks. Kept per [`screen_bucket`] in ui.json (`AppSettings::merge_views`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergeView {
    pub layout: Layout,
    pub aspect: CardAspect,
    /// Groups side by side across the view (2–8): each group is a tile, that many to a row.
    pub per_row: u8,
    /// The kinds shown (empty: every kind, unless `kinds_none`).
    pub kinds: Vec<MergeType>,
    /// The confidences shown (empty: every one from Settings ▸ merge. ▸ show from up, unless
    /// `confidences_none`).
    pub confidences: Vec<Confidence>,
    /// none. in the kind. menu: no kind is picked, so no group shows (an empty `kinds` alone
    /// still means all, as older ui.json files wrote it).
    pub kinds_none: bool,
    /// none. in the confidence. menu: no group shows.
    pub confidences_none: bool,
}

impl Default for MergeView {
    fn default() -> Self {
        MergeView { layout: Layout::Waves, aspect: CardAspect::Original, per_row: 4, kinds: Vec::new(), confidences: Vec::new(), kinds_none: false, confidences_none: false }
    }
}

/// Groups per row: the fewest and the most.
pub const PER_ROW: (u8, u8) = (2, 8);

/// The size of screen a window `width` points wide is on: `small` (< 1300, a laptop), `medium`
/// (1300–1800), `large` (1800–2600), `huge` (> 2600).
pub fn screen_bucket(width: f32) -> &'static str {
    if width < 1300.0 {
        "small"
    } else if width < 1800.0 {
        "medium"
    } else if width < 2600.0 {
        "large"
    } else {
        "huge"
    }
}

/// The screen size the window is on now (`medium` before the first frame).
pub fn screen(app: &LightcraftApp) -> &str {
    if app.merge.screen.is_empty() { "medium" } else { app.merge.screen.as_str() }
}

/// The view settings of this screen size.
pub fn view_prefs(app: &LightcraftApp) -> MergeView {
    app.ui.settings.merge_views.get(screen(app)).cloned().unwrap_or_default()
}

/// The view settings of this screen size, to change.
pub fn view_prefs_mut(app: &mut LightcraftApp) -> &mut MergeView {
    let key = screen(app).to_string();
    app.ui.settings.merge_views.entry(key).or_default()
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
    // which screen size the window is on: its layout., aspect. and per.row. are its own
    let bucket = screen_bucket(ctx.content_rect().width());
    if app.merge.screen != bucket {
        app.merge.screen = bucket.to_string();
    }
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
    if app.session.merge_prefs != app.ui.settings.merge {
        app.session.merge_prefs = app.ui.settings.merge.clone();
    }
    if app.ui.settings.merge_find.automatic() && app.session.vision.background && !app.session.merge_find_running() && app.import.is_none() {
        let now = ctx.input(|i| i.time);
        let key = egui::Id::new("merges-auto-at");
        // (time of the last look, the catalog it looked at): nothing changed, nothing to look at
        let last: Option<(f64, u64)> = ctx.data(|d| d.get_temp(key));
        let rev = app.session.catalog.revision;
        if last.is_none_or(|(t, r)| now - t >= AUTO_EVERY && r != rev) {
            ctx.data_mut(|d| d.insert_temp(key, (now, rev)));
            if let Some(r) = app.session.merge_find_auto_in(app.ui.settings.merge_find == FindMode::Every) {
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

/// What `merge.layout` / `merge.cardSize` answer: this screen size's view.
fn layout_json(app: &LightcraftApp) -> Value {
    let v = view_prefs(app);
    json!({"layout": v.layout.id(), "aspect": v.aspect.id(), "perRow": v.per_row, "screen": screen(app)})
}

/// The app's own handling of merges. commands (`None`: the engine's): `merge.find` looks in the
/// background, `merge.mergeGroups` queues the groups for the merge worker, `merge.findMode` is
/// the setting, `merge.cardSize {size? | step?}` the cards' size (smaller. / bigger.).
pub fn run_ui(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "merge.findMode" => Some(match p.get("mode").and_then(Value::as_str).map(|m| m.trim_end_matches('.')) {
            Some(m) => {
                let mode = match m {
                    "auto" | "weddings" => FindMode::Auto,
                    "every" | "every.folder" | "everyFolder" => FindMode::Every,
                    "find" | "manual" | "off" => FindMode::Manual,
                    other => return Some(Err(format!("merge.findMode: mode is weddings (auto), every or off (find), not `{other}`"))),
                };
                app.ui.settings.merge_find = mode;
                Ok(json!({"mode": mode.id()}))
            }
            None => Ok(json!({"mode": app.ui.settings.merge_find.id()})),
        }),
        // layout. / aspect.: how the view shows the groups (remembered in ui.json)
        "merge.layout" => Some((|| {
            let layout = match p.get("layout").and_then(Value::as_str) {
                Some(l) => Some(Layout::parse(l).ok_or_else(|| format!("merge.layout: layout is waves, grid, strip or focus, not `{l}`"))?),
                None => None,
            };
            let aspect = match p.get("aspect").and_then(Value::as_str) {
                Some(a) => Some(CardAspect::parse(a).ok_or_else(|| format!("merge.layout: aspect is original, square, 3:2, 4:5 or 16:9, not `{a}`"))?),
                None => None,
            };
            let per_row = match p.get("perRow") {
                Some(v) => Some(v.as_u64().filter(|n| (2..=8).contains(n)).ok_or("merge.layout: perRow is 2 to 8 groups")? as u8),
                None => None,
            };
            let v = view_prefs_mut(app);
            if let Some(l) = layout {
                v.layout = l;
            }
            if let Some(a) = aspect {
                v.aspect = a;
            }
            if let Some(n) = per_row {
                v.per_row = n;
            }
            Ok(layout_json(app))
        })()),
        // Settings ▸ merge.: the defaults per kind (a partial `MergePrefs` laid over them) → all of them
        "merge.settings" => Some(match p.as_object().filter(|o| !o.is_empty()) {
            None => serde_json::to_value(&app.ui.settings.merge).map_err(|e| e.to_string()),
            Some(_) => app.ui.settings.merge.patched(p).map(|m| {
                app.ui.settings.merge = m;
                app.session.merge_prefs = app.ui.settings.merge.clone();
                serde_json::to_value(&app.ui.settings.merge).unwrap_or_default()
            }),
        }),
        // smaller. / bigger.: one group more / fewer per row (the tiles' size)
        "merge.cardSize" => {
            let v = view_prefs_mut(app);
            let n = match (p.get("perRow").and_then(Value::as_f64), p.get("step").and_then(Value::as_f64)) {
                (Some(n), _) if n.is_finite() => n.round() as i64,
                (None, Some(d)) if d.is_finite() => i64::from(v.per_row) - d.signum() as i64,
                (None, None) => i64::from(v.per_row),
                _ => return Some(Err("merge.cardSize: {perRow: 2–8} or {step: ±1}".into())),
            };
            v.per_row = n.clamp(i64::from(PER_ROW.0), i64::from(PER_ROW.1)) as u8;
            Some(Ok(layout_json(app)))
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
        _ => crate::merge_page::run_ui(app, id, p),
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
    // `options`: the merge options (the Merge dialog's names) over each group's kind's defaults
    let options = p.get("options").filter(|o| o.is_object());
    let autos = p.get("autoSettings").and_then(Value::as_bool);
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
        let mut opts = crate::merge::page_options(app, k.command());
        if let Some(o) = options {
            let mut v = serde_json::to_value(&opts).map_err(|e| e.to_string())?;
            if let (Some(base), Some(o)) = (v.as_object_mut(), o.as_object()) {
                base.extend(o.iter().filter(|(k, _)| k.as_str() != "command").map(|(k, v)| (k.clone(), v.clone())));
            }
            opts = serde_json::from_value(v).map_err(|e| format!("merge.mergeGroups: options: {e}"))?;
        }
        if let Some(a) = autos {
            opts.auto_settings = a;
        }
        opts.command = k.command().to_string();
        app.merge.group_queue.push_back(crate::merge::GroupMerge { group: g.id.clone(), kind: k, frames, opts: Some(opts) });
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
pub(crate) fn picked_groups(app: &LightcraftApp) -> Vec<MergeGroup> {
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

/// The bar above a merges. view: find. (finding… while it looks), kind. / confidence. / camera.
/// drop-downs and the group count, all. / none. (every group and
/// merged photo, or nothing), merge. / send.back. for the picked groups, both. / merges.only. /
/// unmerge. for the picked merged photos, smaller. / bigger. for the cards.
pub fn bar(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    wrapped_band(ui, "merges:bar", |row| bar_buttons(app, row));
}

/// A band across the view (the bar): its controls in a row 8 px from its edges, 8 px
/// apart, wrapping onto lines 10 px apart when the view is narrow; as tall as its lines.
pub(crate) fn wrapped_band(ui: &mut egui::Ui, reg: &str, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    let full = ui.available_rect_before_wrap();
    let top = ui.cursor().top();
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = Rect::from_min_max(pos2(full.left() + 16.0, top + 8.0), pos2(full.right() - 16.0, full.bottom().max(top + BAR_H)));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)));
    row.spacing_mut().item_spacing = vec2(8.0, 10.0);
    add(&mut row);
    let bottom = (row.min_rect().bottom() + 8.0).max(top + BAR_H);
    let r = Rect::from_min_max(pos2(full.left(), top), pos2(full.right(), bottom));
    ui.painter().set(bg, egui::Shape::rect_filled(r, 0.0, t.canvas));
    ui.allocate_rect(r, Sense::hover());
    register(ui.ctx(), reg, r);
}

fn bar_buttons(app: &mut LightcraftApp, row: &mut egui::Ui) {
    let ctx = row.ctx().clone();
    let running = app.session.merge_find_running();
    let find_label = if running { "finding…" } else { "find." };
    let auto = app.ui.settings.merge_find.automatic();
    let tip = if auto {
        "Look for merges here now (they are also found by themselves in the background: Settings ▸ merge. ▸ find automatically:)"
    } else {
        "Look for merges here: panoramas (panorama.), brackets (hdr.), focus stacks (focus.stack.), HDR panoramas (hdr.panorama.)"
    };
    if crate::widgets::text_button(row, "mergesFind", find_label, running).on_hover_text(crate::i18n::tr(tip)).clicked() && !running {
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
    // the filters (kind., confidence., camera.) and the group count, on this same row
    crate::merge_page::filter_menus(app, row);
    gap(row);
    if crate::widgets::text_button(row, "mergesAll", "all.", false)
        .on_hover_text(crate::i18n::tr("Pick every group and merged photo (⌘A)"))
        .clicked()
    {
        let ids: Vec<u64> = app.session.visible_cloned().iter().map(|p| p.0).collect();
        let _ = app.run("library.select", json!({"ids": ids}));
    }
    if crate::widgets::text_button(row, "mergesNone", "none.", false).on_hover_text(crate::i18n::tr("Pick nothing")).clicked() {
        let _ = app.run("library.select", json!({"ids": []}));
    }
    gap(row);
    let busy = app.merge.final_task.as_ref().is_some_and(|t| t.group.is_some());
    let waiting = app.merge.group_queue.len();
    let merge_label = if busy { format!("merging… {}", waiting + 1) } else { "merge.".to_string() };
    if crate::widgets::text_button(row, "mergesMerge", &merge_label, busy)
        .on_hover_text(crate::i18n::tr("Merge the picked groups, each as its kind, with the frames that are in it. The originals then leave the folder — not deleted — and the merged photo shows here and where they were"))
        .clicked()
    {
        let ids: Vec<String> = picked_groups(app).into_iter().map(|g| g.id).collect();
        if ids.is_empty() {
            app.toast(&ctx, crate::i18n::tr("Pick a group first: click its header"));
        } else if let Err(e) = app.run("merge.openGroups", json!({"ids": ids})) {
            app.toast(&ctx, e);
        }
    }
    if crate::widgets::text_button(row, "mergesSendBack", "send.back.", false)
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
    gap(row);
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
        if crate::widgets::text_button(row, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked() {
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
    gap(row);
    let vp = view_prefs(app);
    let layout = vp.layout;
    let menu = crate::widgets::text_button(row, "mergesLayout", &format!("layout. {}", layout.tag()), false)
        .on_hover_text(crate::i18n::tr("How the groups are laid out: waves., grid., strip. or focus. (remembered)"));
    egui::Popup::menu(&menu).show(|ui| {
        for l in Layout::ALL {
            let r = ui.selectable_label(l == layout, l.tag());
            register(ui.ctx(), format!("menu:mergesLayout-{}", l.id()), r.rect);
            if r.clicked() {
                let _ = app.run("merge.layout", json!({"layout": l.id()}));
            }
        }
    });
    let aspect = vp.aspect;
    let menu = crate::widgets::text_button(row, "mergesAspect", &format!("aspect. {}", aspect.tag()), false)
        .on_hover_text(crate::i18n::tr("The cards' shape (the photo is fitted inside; the photos never change)"));
    egui::Popup::menu(&menu).show(|ui| {
        for a in CardAspect::ALL {
            let r = ui.selectable_label(a == aspect, a.tag());
            register(ui.ctx(), format!("menu:mergesAspect-{}", a.id()), r.rect);
            if r.clicked() {
                let _ = app.run("merge.layout", json!({"aspect": a.id()}));
            }
        }
    });
    let menu = crate::widgets::text_button(row, "mergesPerRow", &format!("per.row. {}", vp.per_row), false)
        .on_hover_text(crate::i18n::tr("Groups side by side across the view: the tiles' size (smaller. / bigger. step it)"));
    egui::Popup::menu(&menu).show(|ui| {
        for n in PER_ROW.0..=PER_ROW.1 {
            let r = ui.selectable_label(n == vp.per_row, format!("{n}."));
            register(ui.ctx(), format!("menu:mergesPerRow-{n}"), r.rect);
            if r.clicked() {
                let _ = app.run("merge.layout", json!({"perRow": n}));
            }
        }
    });
    if layout == Layout::Focus {
        for (id, label, tip, by) in [("mergesPrev", "prev.", "The group before (←)", -1), ("mergesNext", "next.", "The group after (→)", 1)] {
            if crate::widgets::text_button(row, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked()
                && let Err(e) = app.run("merge.step", json!({"by": by}))
            {
                app.toast(&ctx, e);
            }
        }
    }
    gap(row);
    for (id, label, tip, step) in [
        ("mergesSmaller", "smaller.", "Smaller tiles: one group more per row", -1.0f32),
        ("mergesBigger", "bigger.", "Bigger tiles: one group fewer per row", 1.0),
    ] {
        if crate::widgets::text_button(row, id, label, false).on_hover_text(crate::i18n::tr(tip)).clicked()
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
pub(crate) struct GroupView {
    pub g: MergeGroup,
    /// Its frames still in the library: in the group and taken out of it.
    pub frames: Vec<PhotoId>,
    pub aspects: Vec<f32>,
    pub cameras: Vec<String>,
    pub subfolders: Vec<String>,
    pub reasons: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ViewData {
    /// The open groups the filters let through (kind., confidence., camera.), in order.
    pub open: Vec<GroupView>,
    /// Open groups in the scope before the filters.
    pub open_total: usize,
    /// The cameras of the open groups in the scope (the camera. filter's choices).
    pub cameras: Vec<String>,
    /// Per kind: the open groups the kind. chip shows (the other filters applied).
    pub kind_counts: std::collections::BTreeMap<MergeType, usize>,
    /// Per confidence: the open groups the confidence. chip shows (the other filters applied).
    pub conf_counts: std::collections::BTreeMap<Confidence, usize>,
    /// What confidence. all. shows: from Settings ▸ merge. ▸ show from up.
    pub conf_all: usize,
    /// Per camera: the open groups the camera. menu's row shows (the other filters applied).
    pub camera_counts: std::collections::BTreeMap<String, usize>,
    /// What camera. all. shows (the other filters applied).
    pub camera_all: usize,
    pub merged: Vec<PhotoId>,
    pub merged_aspects: Vec<f32>,
    pub sent: Vec<GroupView>,
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

/// The view's groups, once per catalog revision, scope and filters.
pub(crate) fn view_data(app: &LightcraftApp, ctx: &egui::Context) -> Arc<ViewData> {
    let scope = app.session.merges_scope.clone().unwrap_or_default();
    let filter = &app.merge.filter;
    let vp = view_prefs(app);
    let from = app.ui.settings.merge.show_from;
    let key = egui::Id::new("merges-view-data");
    let rev = (app.session.catalog.revision, format!("{scope:?} {:?} {:?} {:?} {from:?} {} {}", filter.camera, vp.kinds, vp.confidences, vp.kinds_none, vp.confidences_none));
    if let Some((r, d)) = ctx.data(|d| d.get_temp::<((u64, String), Arc<ViewData>)>(key))
        && r == rev
    {
        return d;
    }
    let d = Arc::new(compute_view_data(app));
    ctx.data_mut(|m| m.insert_temp(key, (rev, d.clone())));
    d
}

#[cfg(test)]
thread_local! {
    /// How many times [`compute_view_data`] ran on this thread (a test counts them).
    pub(crate) static VIEW_DATA_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// [`view_data`] worked out afresh (a command from outside a frame).
pub(crate) fn compute_view_data(app: &LightcraftApp) -> ViewData {
    #[cfg(test)]
    VIEW_DATA_CALLS.with(|c| c.set(c.get() + 1));
    let scope = app.session.merges_scope.clone().unwrap_or_default();
    let filter = &app.merge.filter;
    let vp = view_prefs(app);
    let from = app.ui.settings.merge.show_from;
    let cat = &app.session.catalog;
    let mut d = ViewData::default();
    for run in merge_find::view(cat, &scope) {
        match run.group.as_deref().and_then(|id| merge_find::group(cat, id)) {
            Some(g) => {
                let gv = group_view(cat, &scope, g, run.ids);
                d.open_total += 1;
                for c in &gv.cameras {
                    if !d.cameras.contains(c) {
                        d.cameras.push(c.clone());
                    }
                }
                let camera = filter.camera.as_ref().is_none_or(|c| gv.cameras.contains(c));
                let kind = !vp.kinds_none && (vp.kinds.is_empty() || vp.kinds.contains(&gv.g.kind));
                let conf = !vp.confidences_none && crate::merge_page::confidence_shown(&vp.confidences, gv.g.confidence(), from);
                // each chip counts what it would show with the other filters as they are
                if camera && conf {
                    *d.kind_counts.entry(gv.g.kind).or_default() += 1;
                }
                if kind && conf {
                    d.camera_all += 1;
                    for c in &gv.cameras {
                        *d.camera_counts.entry(c.clone()).or_default() += 1;
                    }
                }
                if camera && kind {
                    *d.conf_counts.entry(gv.g.confidence()).or_default() += 1;
                    if gv.g.confidence() >= from {
                        d.conf_all += 1;
                    }
                }
                if camera && kind && conf {
                    d.open.push(gv);
                }
            }
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

/// grid.: the cards in rows at their height, wrapping (nothing stacked).
fn grid_layout(aspects: &[f32], card_h: f32, width: f32, per_row: usize) -> Wave {
    let width = width.max(40.0);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let mut cells = Vec::with_capacity(aspects.len());
    for (i, a) in aspects.iter().enumerate() {
        let w = (card_h * a.clamp(0.4, 2.5)).min(width);
        // (a card's width is rounded: 2 points of give)
        if x > 0.0 && (x + w > width + 2.0 || i % per_row.max(1) == 0) {
            x = 0.0;
            y += card_h + CARD_GAP;
        }
        cells.push(Rect::from_min_size(pos2(x, y), vec2(w, card_h)));
        x += w + CARD_GAP;
    }
    let h = if aspects.is_empty() { 0.0 } else { y + card_h };
    Wave { stack: cells.clone(), spread: cells, stack_h: h, spread_h: h }
}

/// strip.: the cards side by side in one row (the row scrolls sideways when longer than the view).
fn strip_layout(aspects: &[f32], card_h: f32) -> Wave {
    let mut x = 0.0f32;
    let mut cells = Vec::with_capacity(aspects.len());
    for a in aspects {
        let w = card_h * a.clamp(0.4, 2.5);
        cells.push(Rect::from_min_size(pos2(x, 0.0), vec2(w, card_h)));
        x += w + CARD_GAP;
    }
    // room under the row for its sideways scroll bar
    let h = card_h + STRIP_BAR;
    Wave { stack: cells.clone(), spread: cells, stack_h: h, spread_h: h }
}

/// Room under a strip. row for its scroll bar.
const STRIP_BAR: f32 = 14.0;

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
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
/// A tile's header: two lines (kind., confidence., frames / merge., send.back.) and, when the
/// group has a camera or subfolder, a third with their chips, this far apart.
pub(crate) const HEAD_LINE_GAP: f32 = 10.0;
fn tile_head_h(gv: &GroupView) -> f32 {
    let lines = if gv.cameras.is_empty() && gv.subfolders.is_empty() { 2.0 } else { 3.0 };
    lines * HEAD_H + (lines - 1.0) * HEAD_LINE_GAP
}
/// The narrowest a group's tile gets (per.row. gives way to it on a narrow view): room for
/// merge. and send.back. side by side.
const MIN_TILE_W: f32 = 184.0;
/// Lines of reasons a tile shows (the rest on hover).
const TILE_REASON_LINES: usize = 2;
const SECTION_H: f32 = 32.0;

/// A group's pieces: header, reasons and wave, top to bottom (relative to the block's top).
struct Block {
    height: f32,
    head_h: f32,
    reasons_top: f32,
    wave_top: f32,
    wave_h: f32,
    t: f32,
}

fn block(open: bool, head_h: f32, reasons_h: f32, wave: &Wave, t: f32) -> Block {
    let reasons_top = PAD + head_h + 6.0;
    if !open {
        return Block { height: PAD + head_h + PAD, head_h, reasons_top, wave_top: 0.0, wave_h: 0.0, t };
    }
    let wave_top = reasons_top + reasons_h + BLOCK_GAP;
    // spreading never shrinks it (a pointer near its bottom would leave, and it would fold back)
    let wave_h = (wave.stack_h + (wave.spread_h - wave.stack_h) * t).max(wave.stack_h);
    Block { height: wave_top + wave_h + PAD + 2.0, head_h, reasons_top, wave_top, wave_h, t }
}


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
    if data.open.is_empty() && data.open_total > 0 && data.merged.is_empty() {
        let n = data.open_total;
        let body = crate::i18n::tr_format!("{n} groups here are hidden by kind., confidence. or camera. above: all. shows them", n = n);
        crate::panels::empty_message(ui, ui.max_rect(), "No merges match the filters", &body);
        close_frame_menu(&ctx);
        return;
    }
    if data.open.is_empty() && data.merged.is_empty() && data.sent.is_empty() {
        let body = if app.ui.settings.merge_find.automatic() {
            "Panoramas, brackets and focus stacks show up here as LightCraft finds them. find. looks now"
        } else {
            "Click find. to look here for panoramas (panorama.), brackets (hdr.) and focus stacks (focus.stack.)"
        };
        crate::panels::empty_message(ui, ui.max_rect(), "No merges here", body);
        close_frame_menu(&ctx);
        return;
    }
    let ppp = ctx.pixels_per_point();
    let vp = view_prefs(app);
    let (layout, aspect) = (vp.layout, vp.aspect);
    let outer = ui.available_width();
    let panel_w = (outer - 32.0).max(120.0);
    let inner_w = panel_w - 2.0 * PAD;
    // per.row.: so many groups (tiles) side by side across the view; focus. is one big group
    let per_row = vp.per_row.clamp(PER_ROW.0, PER_ROW.1);
    let tiled = layout != Layout::Focus;
    let cols = if tiled { usize::from(per_row).min((((panel_w + BLOCK_GAP) / (MIN_TILE_W + BLOCK_GAP)).floor() as usize).max(1)) } else { 1 };
    let tile_w = if tiled { ((panel_w - BLOCK_GAP * (cols - 1) as f32) / cols as f32).floor().max(MIN_TILE_W.min(panel_w)) } else { panel_w };
    let tile_inner = tile_w - 2.0 * PAD;
    // the frames' size inside a tile: a fan of cards 70 % of the tile wide, grid. two across,
    // strip. a card and a bit
    let cell_w = match layout {
        Layout::Grid => ((tile_inner - CARD_GAP) / 2.0).max(40.0),
        Layout::Strip => (tile_inner * 0.75).max(40.0),
        _ => (tile_inner * 0.7).max(40.0),
    };
    // (merged. and sent back. keep frames-per-row cards: the old per.row. size)
    let row_cell_w = ((inner_w - CARD_GAP * f32::from(per_row - 1)) / f32::from(per_row)).max(40.0);
    let ch = (row_cell_w / 1.5).round().max(40.0);
    let h_for = |cards: &[f32]| {
        let mean = if cards.is_empty() { 1.5 } else { cards.iter().map(|a| a.clamp(0.4, 2.5)).sum::<f32>() / cards.len() as f32 };
        (cell_w / mean).round().clamp(40.0, 900.0)
    };
    // focus.: one group (the one picked, else the first), as big as the view lets it be
    let open_groups: Vec<GroupView> = if layout == Layout::Focus {
        let i = crate::merge_page::focus(app, &data.open).unwrap_or(0);
        data.open.get(i).cloned().into_iter().collect()
    } else {
        data.open.clone()
    };
    let focus_h = (ui.available_height() - PAD * 2.0 - HEAD_H - 80.0).clamp(160.0, 900.0).round();
    let hovered_before: Option<String> = ctx.data(|d| d.get_temp::<Option<String>>(egui::Id::new("merges-hover"))).flatten();
    let wave_w = if tiled { tile_inner } else { inner_w };
    // lay out every block first (the scroll area needs the height)
    let mut blocks: Vec<(Rect, Block, Wave, Arc<egui::Galley>)> = Vec::new();
    for gv in &open_groups {
        let open = crate::folds::is_open(&ctx, &fold_id(&gv.g.id));
        let pinned = crate::folds::is_open_or(&ctx, &spread_id(&gv.g.id), false);
        let target = if layout != Layout::Waves || pinned || hovered_before.as_deref() == Some(gv.g.id.as_str()) { 1.0 } else { 0.0 };
        let tt = ctx.animate_value_with_time(egui::Id::new(("merges-wave", gv.g.id.as_str())), target, 0.2);
        let cards = aspect.cards(&gv.aspects);
        let gh = h_for(&cards);
        let wave = match layout {
            Layout::Waves => wave_layout(&cards, gh, wave_w),
            Layout::Grid => grid_layout(&cards, gh, wave_w, 2),
            Layout::Strip => strip_layout(&cards, gh),
            Layout::Focus => {
                let mut w = wave_layout(&cards, focus_h, wave_w);
                w.stack = w.spread.clone();
                w.stack_h = w.spread_h;
                w
            }
        };
        let reasons = if tiled {
            let mut job = egui::text::LayoutJob::simple(gv.reasons.clone(), t.font(11.5), t.text_dim, wave_w);
            job.wrap.max_rows = TILE_REASON_LINES;
            job.wrap.overflow_character = Some('…');
            ui.painter().layout_job(job)
        } else {
            ui.painter().layout(gv.reasons.clone(), t.font(11.5), t.text_dim, wave_w)
        };
        let head_h = if tiled { tile_head_h(gv) } else { HEAD_H };
        let b = block(open, head_h, if open { reasons.size().y } else { 0.0 }, &wave, tt);
        blocks.push((Rect::NOTHING, b, wave, reasons));
    }
    // the tiles flow `cols` to a row, the tiles of a row as tall as the tallest
    let mut y = 12.0f32;
    for row in blocks.chunks_mut(cols) {
        let row_h = row.iter().map(|(_, b, _, _)| b.height).fold(0.0, f32::max);
        for (c, (rect, _, _, _)) in row.iter_mut().enumerate() {
            *rect = Rect::from_min_size(pos2(c as f32 * (tile_w + BLOCK_GAP), y), vec2(tile_w, row_h));
        }
        y += row_h + BLOCK_GAP;
    }
    let merged_top = y;
    let merged_open = crate::folds::is_open(&ctx, "merges.merged");
    let small_h = (ch * 0.75).round();
    let merged_cells = if data.merged.is_empty() { Vec::new() } else { wave_layout(&aspect.cards(&data.merged_aspects), small_h, inner_w).spread };
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
    let sent_waves: Vec<Wave> = data.sent.iter().map(|g| wave_layout(&aspect.cards(&g.aspects), sent_h, inner_w)).collect();
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
        let reveal = app.merge.reveal.take();
        for (gv, (rect, b, wave, reasons)) in open_groups.iter().zip(&blocks) {
            if b.t > 0.001 && b.t < 0.999 {
                animating = true;
            }
            let rect = *rect;
            // ← / → on the merge. page: the group picked comes into view
            if reveal.as_deref() == Some(gv.g.id.as_str()) {
                ui.scroll_to_rect(rect.translate(origin.to_vec2()), Some(egui::Align::Center));
            }
            if !rect.intersects(near) {
                continue;
            }
            let r = rect.translate(origin.to_vec2());
            if group_block(app, ui, gv, r, b, wave, reasons.clone(), &order, ppp, &mut seen, layout, tiled) {
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
    frame_menu(app, &ctx);
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
pub(crate) fn chip(ui: &mut egui::Ui, id: &str, label: &str, dot: Option<Color32>) -> egui::Response {
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

pub(crate) fn confidence_color(c: Confidence) -> Color32 {
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
    layout: Layout,
    tiled: bool,
) -> bool {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let g = &gv.g;
    let picked = gv.frames.iter().any(|f| app.session.selection.contains(*f));
    let p = ui.painter().clone();
    p.rect(r, 8.0, t.cell, Stroke::new(if picked { 2.0 } else { 1.0 }, if picked { t.accent } else { t.divider }), egui::StrokeKind::Inside);
    // the header: a click on it picks the group (⌘: add / remove, ⇧: the groups from the last one)
    let head_h = b.head_h;
    let head = Rect::from_min_size(r.min + vec2(PAD, PAD), vec2(r.width() - 2.0 * PAD, head_h));
    register(&ctx, format!("mergeTile:{}", g.id), r);
    let head_resp = ui.interact(head, egui::Id::new(("merges-head", g.id.as_str())), Sense::click());
    register(&ctx, format!("group:merges:group:{}", g.id), head);
    if head_resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        pick_group(app, &ctx, &g.id, &gv.frames, order, m);
    }
    // a tile's header is two lines (kind., confidence. / merge., send.back., camera); the one big
    // group of focus. keeps one
    let line1 = Rect::from_min_size(head.min, vec2(head.width(), HEAD_H));
    let line2 = Rect::from_min_size(head.min + vec2(0.0, HEAD_H + HEAD_LINE_GAP), vec2(head.width(), HEAD_H));
    let line3 = Rect::from_min_size(head.min + vec2(0.0, 2.0 * (HEAD_H + HEAD_LINE_GAP)), vec2(head.width(), HEAD_H));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(line1).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    row.set_clip_rect(line1.intersect(ui.clip_rect()));
    let conf = g.confidence();
    let tb = |row: &egui::Ui, label: &str| chip_width(row, label, false).max(37.0);
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
    let conf_tip = crate::i18n::tr(match conf {
        Confidence::Sure => "sure.: everything says it is this merge",
        Confidence::Likely => "likely.: most things say so",
        Confidence::Maybe => "maybe.: a borderline match — look before merging",
    });
    // merge. / send.back.
    let queued = app.merge.group_queue.iter().any(|q| q.group == g.id);
    let running = app.merge.final_task.as_ref().and_then(|t| t.group.as_ref()).is_some_and(|(q, _)| *q == g.id);
    let merge_label = if running {
        "merging…"
    } else if queued {
        "waiting…"
    } else {
        "merge."
    };
    let in_group = gv.frames.iter().filter(|f| g.photos.contains(f)).count();
    let count = if in_group == gv.frames.len() { format!("{} frames", gv.frames.len()) } else { format!("{in_group} of {} frames", gv.frames.len()) };
    let count_w = row.painter().layout_no_wrap(count.clone(), t.font(12.5), t.text_dim).size().x;
    let mut chips: Vec<(String, String)> = gv.cameras.iter().map(|c| (format!("mergesCamera-{}", g.id), c.clone())).collect();
    chips.extend(gv.subfolders.iter().map(|s| (format!("mergesSubfolder-{}", g.id), s.clone())));
    let (merge_w, back_w) = (tb(&row, merge_label), tb(&row, "send.back."));
    let draw_chips = |row: &mut egui::Ui, limit: f32| {
        for (k, (id, label)) in chips.iter().enumerate() {
            let w = chip_width(row, label, false);
            if row.cursor().left() + w > limit {
                break;
            }
            let tip = if id.starts_with("mergesCamera") { "The camera" } else { "The subfolder of the shoot" };
            chip(row, &format!("{id}-{k}"), label, None).on_hover_text(crate::i18n::tr(tip));
        }
    };
    let merge_btn = |app: &mut LightcraftApp, ui: &mut egui::Ui| {
        if crate::widgets::text_button(ui, &format!("mergesGroupMerge-{}", g.id), merge_label, running || queued)
            .on_hover_text(crate::i18n::tr("Merge this group: the Merge box opens on its kind, with the frames that are in it"))
            .clicked()
            && !running
            && !queued
            && let Err(e) = app.run("merge.openGroups", json!({"ids": [g.id]}))
        {
            app.toast(&ctx, e);
        }
    };
    let back_btn = |app: &mut LightcraftApp, ui: &mut egui::Ui| {
        if crate::widgets::text_button(ui, &format!("mergesGroupSendBack-{}", g.id), "send.back.", false)
            .on_hover_text(crate::i18n::tr(
                "Not a merge: its photos stay in their folder and it is never suggested again (restore. under sent back. brings it back)",
            ))
            .clicked()
            && let Err(e) = app.run("merge.dismiss", json!({"ids": [g.id]}))
        {
            app.toast(&ctx, e);
        }
    };
    if tiled {
        // line 1: confidence. (a dot alone when it doesn't fit), the frame count when there is room
        let room = line1.right() - row.cursor().left();
        let full_w = chip_width(&row, conf.tag(), true);
        let dot_w = chip_width(&row, "", true);
        if room >= full_w + 8.0 {
            chip(&mut row, &format!("mergesConfidence-{}", g.id), conf.tag(), Some(confidence_color(conf))).on_hover_text(conf_tip);
        } else if room >= dot_w + 8.0 {
            chip(&mut row, &format!("mergesConfidence-{}", g.id), "", Some(confidence_color(conf))).on_hover_text(conf_tip);
        }
        if row.cursor().left() + count_w <= line1.right() {
            let (cr, _) = row.allocate_exact_size(vec2(count_w, 24.0), Sense::hover());
            row.painter().text(cr.left_center(), Align2::LEFT_CENTER, count, t.font(12.5), t.text_dim);
        }
        // line 2: merge., send.back.
        let mut row2 = ui.new_child(egui::UiBuilder::new().max_rect(line2).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row2.spacing_mut().item_spacing.x = 8.0;
        row2.set_clip_rect(line2.intersect(ui.clip_rect()));
        merge_btn(app, &mut row2);
        if row2.cursor().left() + back_w <= line2.right() + 0.5 {
            back_btn(app, &mut row2);
        }
        // line 3: the camera and subfolder chips, each shortened to the room left (full on hover)
        if !chips.is_empty() {
            let mut row3 = ui.new_child(egui::UiBuilder::new().max_rect(line3).layout(egui::Layout::left_to_right(egui::Align::Center)));
            row3.spacing_mut().item_spacing.x = 8.0;
            row3.set_clip_rect(line3.intersect(ui.clip_rect()));
            for (k, (id, label)) in chips.iter().enumerate() {
                let room = line3.right() - row3.cursor().left();
                if room < 48.0 {
                    break;
                }
                let font = t.font(11.5);
                let shown = crate::widgets::elide_head(label, room - 20.0, |s| row3.painter().layout_no_wrap(s.to_string(), font.clone(), t.text_label).size().x);
                let tip = if id.starts_with("mergesCamera") { "The camera" } else { "The subfolder of the shoot" };
                chip(&mut row3, &format!("{id}-{k}"), &shown, None).on_hover_text(format!("{label}\n{}", crate::i18n::tr(tip)));
            }
        }
    } else {
        chip(&mut row, &format!("mergesConfidence-{}", g.id), conf.tag(), Some(confidence_color(conf))).on_hover_text(conf_tip);
        // merge. / send.back. at the right; the chips give way to them
        let right_w = merge_w + back_w + 8.0 + 8.0;
        draw_chips(&mut row, head.right() - right_w - count_w - 8.0);
        if row.cursor().left() + count_w <= head.right() - right_w {
            let (cr, _) = row.allocate_exact_size(vec2(count_w, 24.0), Sense::hover());
            row.painter().text(cr.left_center(), Align2::LEFT_CENTER, count, t.font(12.5), t.text_dim);
        }
        let mut right = row.new_child(egui::UiBuilder::new().max_rect(line1).layout(egui::Layout::right_to_left(egui::Align::Center)));
        right.spacing_mut().item_spacing.x = 8.0;
        back_btn(app, &mut right);
        merge_btn(app, &mut right);
    }
    if !open {
        return false;
    }
    let reasons_rect = Rect::from_min_size(pos2(r.left() + PAD, r.top() + b.reasons_top), reasons.size());
    p.galley(reasons_rect.min, reasons, t.text_dim);
    if tiled && !gv.reasons.is_empty() {
        ui.interact(reasons_rect, egui::Id::new(("merges-reasons", g.id.as_str())), Sense::hover()).on_hover_text(gv.reasons.clone());
    }
    // the wave: its empty area keeps it spread (or lets it fold back); hover spreads it
    let wr = Rect::from_min_size(pos2(r.left() + PAD, r.top() + b.wave_top), vec2(r.width() - 2.0 * PAD, b.wave_h));
    register(&ctx, format!("mergeWave:{}", g.id), wr);
    if layout == Layout::Strip {
        // one row, scrolling sideways when it is longer than the view
        let content_w = wave.spread.last().map_or(0.0, |c| c.right());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(wr).layout(egui::Layout::left_to_right(egui::Align::Min)));
        egui::ScrollArea::horizontal().id_salt(("merges-strip", g.id.as_str())).auto_shrink([false, true]).show(&mut child, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(content_w, wr.height() - STRIP_BAR), Sense::hover());
            for (pid, c) in gv.frames.iter().zip(&wave.spread) {
                let cr = c.translate(area.min.to_vec2());
                if !cr.intersects(ui.clip_rect()) {
                    continue;
                }
                seen.insert(*pid);
                frame_card(app, ui, "", &g.id, *pid, g.photos.contains(pid), cr, ppp);
            }
        });
        return false;
    }
    let wave_resp = ui.interact(wr, egui::Id::new(("merges-wave-area", g.id.as_str())), Sense::click());
    if wave_resp.clicked() && layout == Layout::Waves {
        let id = spread_id(&g.id);
        let on = crate::folds::is_open_or(&ctx, &id, false);
        crate::folds::set(&ctx, &id, !on);
    }
    let hovered = ui.rect_contains_pointer(wr);
    for (i, pid) in gv.frames.iter().enumerate() {
        let (Some(a), Some(s)) = (wave.stack.get(i), wave.spread.get(i)) else { continue };
        let cr = lerp_rect(*a, *s, b.t).translate(wr.min.to_vec2());
        seen.insert(*pid);
        frame_card(app, ui, "", &g.id, *pid, g.photos.contains(pid), cr, ppp);
    }
    hovered
}

/// A frame of a group as a card: dimmed when taken out of the group (still a full control). Its ⊖
/// (top-left), a right-click on it, or ⌫ with frames picked asks what to do with the picked frames
/// ([`frame_menu`]: out.of.group., remove., del., cancel.). A click picks it (⌘-click adds / takes
/// away, ⇧-click the run from the last), a double-click opens it. `place`: "" in the view
/// (`mergeCard:…`), "Page" in the merge. page's right panel (`mergePageCard:…`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn frame_card(app: &mut LightcraftApp, ui: &mut egui::Ui, place: &str, gid: &str, pid: PhotoId, in_group: bool, r: Rect, ppp: f32) {
    let t = Tokens::get(ui.ctx());
    let ctx = ui.ctx().clone();
    let resp = ui.interact(r, egui::Id::new(("merge-card", place, gid, pid.0)), Sense::click());
    register(&ctx, format!("merge{place}Card:{gid}:{}", pid.0), r);
    let minus = Rect::from_center_size(r.left_top() + vec2(16.0, 16.0), vec2(MINUS_D, MINUS_D));
    let minus_resp = ui.interact(minus, egui::Id::new(("merge-minus", place, gid, pid.0)), Sense::click());
    register(&ctx, format!("merge{place}Minus:{gid}:{}", pid.0), minus);
    let p = ui.painter();
    p.rect_filled(r.translate(vec2(2.0, 3.0)), 4.0, Color32::from_black_alpha(110));
    crate::panels::grid::request_thumb(app, pid, crate::panels::grid::thumb_px(r.width().max(r.height()), ppp), 10);
    let tint = if in_group { Color32::WHITE } else { Color32::from_gray(80) };
    card_image(app, p, pid, r, tint);
    let picked = app.session.selection.contains(pid);
    let edge = if picked {
        Stroke::new(2.0, t.accent)
    } else if resp.hovered() {
        Stroke::new(2.0, t.text)
    } else {
        Stroke::new(1.0, Color32::from_black_alpha(160))
    };
    p.rect_stroke(r, 4.0, edge, egui::StrokeKind::Inside);
    // the ⊖ (a circle with a minus): opens the question; a frame already out shows it filled white
    let (fill, ink) = if in_group { (Color32::from_black_alpha(if minus_resp.hovered() { 235 } else { 200 }), Color32::WHITE) } else { (Color32::WHITE, Color32::BLACK) };
    let c = minus.center();
    p.circle(c, MINUS_D / 2.0 - 2.0, fill, Stroke::new(1.5, if minus_resp.hovered() { t.accent } else { ink }));
    p.line_segment([c - vec2(4.5, 0.0), c + vec2(4.5, 0.0)], Stroke::new(2.0, ink));
    let name = app.session.catalog.photo(pid).map(|p| format!("{}\n{}", p.file_name, merge_find::exposure_text(&p.meta))).unwrap_or_default();
    resp.clone().on_hover_text(format!("{name}\n{}", crate::i18n::tr("⊖ / right-click / ⌫: remove…")));
    minus_resp.clone().on_hover_text(crate::i18n::tr("out of the group, remove or delete this frame (all the picked ones)…"));
    if minus_resp.clicked() {
        open_frame_menu(app, &ctx, pid, minus.left_bottom() + vec2(0.0, 6.0));
    } else if resp.secondary_clicked() {
        let at = ctx.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
        open_frame_menu(app, &ctx, pid, at);
    } else if resp.double_clicked() {
        let _ = app.run("library.select", json!({"ids": [pid.0]}));
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
        let _ = app.run("library.select", json!({"ids": [pid.0], "mode": mode}));
    }
}

/// Size of a frame card's ⊖ (its hit area).
const MINUS_D: f32 = 24.0;

/// What the question over frames is asking.
#[derive(Clone, Debug)]
struct FrameMenu {
    /// The frames it acts on (the picked ones when the clicked one is picked).
    ids: Vec<u64>,
    at: egui::Pos2,
    /// Every one of them is out of its group: the first choice puts them back.
    put_back: bool,
    /// del.: asked once more (Settings ▸ Confirm first).
    confirm: bool,
    /// The pass that opened it (the click that opened it never closes it).
    opened: u64,
    /// How many merge groups those frames are in (the title says).
    groups: usize,
    /// The source (and merges scope) it was opened in: it closes when that changes.
    source: String,
    /// The last frame [`frame_menu`] drew it: not drawn for a frame (another module, tab or source,
    /// the empty view) it closes ([`frame_menu_tick`]).
    drawn: u64,
}

fn frame_menu_id() -> egui::Id {
    egui::Id::new("merges-frame-menu")
}

/// The picked frames of the groups the view shows unfolded (a folded group, or one the focus.
/// layout leaves out, is not on screen: ⊖ / ⌫ never act on what isn't seen).
fn picked_frames(app: &LightcraftApp, ctx: &egui::Context) -> Vec<u64> {
    let sel = &app.session.selection;
    let data = view_data(app, ctx);
    let shown: Vec<&GroupView> = if view_prefs(app).layout == Layout::Focus {
        let i = crate::merge_page::focus(app, &data.open).unwrap_or(0);
        data.open.get(i).into_iter().collect()
    } else {
        data.open.iter().collect()
    };
    let mut out: Vec<u64> = Vec::new();
    for g in shown.into_iter().filter(|g| crate::folds::is_open(ctx, &fold_id(&g.g.id))) {
        for f in &g.frames {
            if sel.contains(*f) && !out.contains(&f.0) {
                out.push(f.0);
            }
        }
    }
    out
}

/// Open the question for frame `pid` at `at`: it acts on every picked frame in view when `pid` is
/// picked, else on `pid` alone (which it picks).
fn open_frame_menu(app: &mut LightcraftApp, ctx: &egui::Context, pid: PhotoId, at: egui::Pos2) {
    let ids = if app.session.selection.contains(pid) {
        let ids = picked_frames(app, ctx);
        if ids.contains(&pid.0) { ids } else { vec![pid.0] }
    } else {
        let _ = app.run("library.select", json!({"ids": [pid.0]}));
        vec![pid.0]
    };
    start_frame_menu(app, ctx, ids, at);
}

pub(crate) fn start_frame_menu(app: &LightcraftApp, ctx: &egui::Context, ids: Vec<u64>, at: egui::Pos2) {
    // (once: the view's groups are worked out from the catalog each time)
    let data = compute_view_data(app);
    let in_group = |id: u64| data.open.iter().any(|g| g.g.photos.iter().any(|p| p.0 == id));
    let put_back = !ids.iter().any(|i| in_group(*i));
    let groups = data.open.iter().filter(|g| g.frames.iter().any(|f| ids.contains(&f.0))).count();
    let m = FrameMenu {
        ids,
        at,
        put_back,
        confirm: false,
        opened: ctx.cumulative_pass_nr(),
        groups,
        source: crate::leftoff::source_key(app),
        drawn: ctx.cumulative_frame_nr(),
    };
    ctx.data_mut(|d| d.insert_temp(frame_menu_id(), m));
    ctx.request_repaint();
}

/// Close the question (if it is open).
fn close_frame_menu(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<FrameMenu>(frame_menu_id()));
}

/// Is the question open?
#[cfg(test)]
pub(crate) fn frame_menu_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<FrameMenu>(frame_menu_id())).is_some()
}

/// Every frame, before the keys: Esc closes the question before any shortcut sees it, and a
/// question that wasn't drawn last frame (the merges. view isn't on screen: another module, tab
/// or source) or belongs to another source is dropped, so it never comes back later with old ids.
pub fn frame_menu_tick(app: &LightcraftApp, ctx: &egui::Context) {
    let Some(m) = ctx.data(|d| d.get_temp::<FrameMenu>(frame_menu_id())) else { return };
    let esc = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if esc || m.source != crate::leftoff::source_key(app) || ctx.cumulative_frame_nr() > m.drawn + 1 {
        close_frame_menu(ctx);
    }
}

/// ⌫ / Delete in a merges. view with frames picked: the same question as the ⊖ (true when it
/// opened). A merged photo picked, or nothing: Delete does what it always does.
pub fn delete_key(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    if app.session.source != lightcraft_engine::LibrarySource::Merges
        || !matches!(app.ui.view, crate::state::ViewMode::PhotoGrid | crate::state::ViewMode::SquareGrid)
        || ctx.egui_wants_keyboard_input()
    {
        return false;
    }
    let pressed = ctx.input(|i| i.key_pressed(Key::Backspace) || i.key_pressed(Key::Delete)) && ctx.input(|i| i.modifiers == Modifiers::NONE);
    if !pressed {
        return false;
    }
    let ids = picked_frames(app, ctx);
    if ids.is_empty() {
        return false;
    }
    ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Backspace) || i.consume_key(Modifiers::NONE, Key::Delete));
    // under the first picked frame (where it is on screen), else the middle of the view
    let at = app
        .widgets
        .iter()
        .find(|(w, _)| ids.iter().any(|i| w.starts_with("mergeCard:") && w.ends_with(&format!(":{i}"))))
        .map(|(_, r)| r.center_bottom())
        .or_else(|| app.canvas_rect.map(|r| r.center()))
        .unwrap_or(egui::pos2(200.0, 200.0));
    start_frame_menu(app, ctx, ids, at);
    true
}

/// The question over frames (small, at the ⊖ or the pointer): out.of.group. (the frame stays in
/// the library and Develop as normal, only this merge group loses it — put.back. when they are
/// already out), remove. (out of LightCraft, files kept: `photo.removeFromLibrary`), del. (the
/// files to the Trash: `photo.deleteFromDisk`, asked once more with Settings ▸ Confirm first),
/// cancel. (or Esc, or a click outside).
fn frame_menu(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(mut m) = ctx.data(|d| d.get_temp::<FrameMenu>(frame_menu_id())) else { return };
    if m.source != crate::leftoff::source_key(app) {
        close_frame_menu(ctx);
        return;
    }
    m.drawn = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.insert_temp(frame_menu_id(), m.clone()));
    let t = Tokens::get(ctx);
    let n = m.ids.len();
    let what = if n == 1 { "1 frame".to_string() } else { format!("{n} frames") };
    let what = match m.groups {
        0 => what,
        1 => format!("{what} in 1 group"),
        g => format!("{what} in {g} groups"),
    };
    let mut act: Option<&'static str> = None;
    let area = egui::Area::new(egui::Id::new("merges-frame-menu-area"))
        .order(egui::Order::Foreground)
        .fixed_pos(m.at)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.chrome)
                .stroke(Stroke::new(1.0, t.divider))
                .corner_radius(8.0)
                .inner_margin(12.0)
                .shadow(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(90) })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 12.0);
                    let title = if m.confirm { format!("move {what} to the trash?") } else { format!("{what} picked.") };
                    let g = ui.painter().layout_no_wrap(title, t.font(12.0), t.text_dim);
                    let (tr, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                    register(ui.ctx(), "label:mergesFrameMenuTitle", tr);
                    ui.painter().galley(tr.min, g, t.text_dim);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        fn tip(s: &str) -> &str {
                            crate::i18n::tr(s)
                        }
                        if m.confirm {
                            if crate::widgets::text_button(ui, "mergesFrameDeleteYes", "del.", false)
                                .on_hover_text(tip("The files go to the Trash (undo. brings them back)"))
                                .clicked()
                            {
                                act = Some("delete");
                            }
                        } else {
                            let (id, label, tt) = if m.put_back {
                                ("mergesFrameOutOfGroup", "put.back.", "Back into the merge group")
                            } else {
                                ("mergesFrameOutOfGroup", "out.of.group.", "Out of this merge group only: the photo stays in the library and Develop as normal")
                            };
                            if crate::widgets::text_button(ui, id, label, false).on_hover_text(tip(tt)).clicked() {
                                act = Some("group");
                            }
                            if crate::widgets::text_button(ui, "mergesFrameRemove", "remove.", false)
                                .on_hover_text(tip("Remove from LightCraft: the files stay on disk"))
                                .clicked()
                            {
                                act = Some("remove");
                            }
                            if crate::widgets::text_button(ui, "mergesFrameDelete", "del.", false)
                                .on_hover_text(tip("Delete from the computer: the files go to the Trash"))
                                .clicked()
                            {
                                act = Some("del");
                            }
                        }
                        if crate::widgets::text_button(ui, "mergesFrameCancel", "cancel.", false).on_hover_text(tip("Do nothing (Esc)")).clicked() {
                            act = Some("cancel");
                        }
                    });
                });
        });
    register(ctx, "mergesFrameMenu", area.response.rect);
    // Esc or a press outside closes it (not the press that opened it)
    if act.is_none() {
        let outside = ctx.cumulative_pass_nr() > m.opened
            && ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
        if outside || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            act = Some("cancel");
        }
    }
    let Some(act) = act else { return };
    let ids: Vec<u64> = m.ids.clone();
    let end = close_frame_menu;
    match act {
        "group" => {
            end(ctx);
            set_frames_in_group(app, ctx, &ids, m.put_back);
        }
        "remove" => {
            end(ctx);
            match app.run("photo.removeFromLibrary", json!({"ids": ids})) {
                Ok(r) => {
                    let gone = r["removed"].as_u64().unwrap_or(n as u64);
                    let text = format!("{gone} photo{} removed from the library (files kept)", if gone == 1 { "" } else { "s" });
                    crate::menus::delete_toast(app, &r, text);
                }
                Err(e) => app.toast(ctx, e),
            }
        }
        "del" if app.ui.settings.confirm_delete => {
            m.confirm = true;
            ctx.data_mut(|d| d.insert_temp(frame_menu_id(), m));
        }
        "del" | "delete" => {
            end(ctx);
            match app.run("photo.deleteFromDisk", json!({"ids": ids, "keptCopies": false})) {
                Ok(r) => {
                    let text = crate::menus::trash_message(&r);
                    crate::menus::delete_toast(app, &r, text);
                }
                Err(e) => app.toast(ctx, e),
            }
        }
        _ => end(ctx),
    }
}

/// Take frames out of their merge groups (`merge.setFrames`), or, `put_back`, in again. The
/// photos themselves are untouched.
fn set_frames_in_group(app: &mut LightcraftApp, ctx: &egui::Context, ids: &[u64], put_back: bool) {
    for gv in compute_view_data(app).open {
        let mine: Vec<u64> = gv.frames.iter().map(|p| p.0).filter(|p| ids.contains(p)).collect();
        if mine.is_empty() {
            continue;
        }
        let mut photos: Vec<u64> = gv.g.photos.iter().map(|p| p.0).collect();
        if put_back {
            for p in mine {
                if !photos.contains(&p) {
                    photos.push(p);
                }
            }
        } else {
            photos.retain(|p| !mine.contains(p));
        }
        if let Err(e) = app.run("merge.setFrames", json!({"id": gv.g.id, "photos": photos})) {
            app.toast(ctx, e);
        }
    }
}

/// A card's picture (its thumbnail, or a placeholder while it renders), `tint`ed.
pub(crate) fn card_image(app: &LightcraftApp, p: &egui::Painter, id: PhotoId, r: Rect, tint: Color32) {
    match app.renderer.thumb(id) {
        Some(tex) => {
            // fitted inside the card (letterboxed on a card of another shape), never stretched
            let [w, h] = tex.tex.size();
            let fit = if w > 0 && h > 0 {
                let a = w as f32 / h as f32;
                let (fw, fh) = if r.width() / r.height().max(1.0) > a { (r.height() * a, r.height()) } else { (r.width(), r.width() / a) };
                Rect::from_center_size(r.center(), vec2(fw, fh))
            } else {
                r
            };
            if (fit.width() - r.width()).abs() > 1.0 || (fit.height() - r.height()).abs() > 1.0 {
                p.rect_filled(r, 4.0, Color32::from_gray(22));
            }
            p.image(tex.tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
        }
        None => {
            p.rect_filled(r, 4.0, Color32::from_gray(46).linear_multiply(f32::from(tint.r()) / 255.0 * 0.5 + 0.5));
        }
    }
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
pub(crate) fn pick_group(app: &mut LightcraftApp, ctx: &egui::Context, gid: &str, frames: &[PhotoId], order: &[(String, Vec<PhotoId>)], m: egui::Modifiers) {
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
