//! "Where I left off": the photo last worked on, overall and per folder / album.
//!
//! Every frame the active photo (and the view, and each edit) is recorded for the source on
//! screen, in the UI state saved with the app settings (`ui.json` → `leftOff`). Opening a folder
//! or album lands on the photo last active there and marks it in the grid and filmstrip
//! ("left off" badge) for the visit; `view.resumeLastLeftOff` (the top bar's left off. button)
//! goes back to the last one of all: its folder or album, the photo and the view.
//!
//! With no record of its own for a source yet, LightCraft asks the engine's
//! `library.resumePoint {folder?|album?}` → `{photoId, at}` when that command exists (it falls
//! back to Lightroom Classic's last-edit times after a catalog migration).

use std::collections::BTreeMap;

use lightcraft_catalog::PhotoId;
use lightcraft_engine::LibrarySource;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{RightPanel, ViewMode};

/// Records kept per library (oldest dropped first).
const MAX_SOURCES: usize = 400;

/// One "left off" spot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Spot {
    /// The library it belongs to (its folder; `memory` for a throwaway session).
    pub library: String,
    /// `all`, `album:<id>`, `folder:<path>`, `recentlyAdded`, `picks`…
    pub source: String,
    pub photo: u64,
    /// `grid`, `detail` or `edit`.
    pub view: String,
    /// Unix seconds.
    pub at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LeftOff {
    /// `"<library>|<source>"` → the spot last active there.
    pub per_source: BTreeMap<String, Spot>,
    /// The last spot of all.
    pub last: Option<Spot>,
}

/// `ui.json` → `leftOff`, read leniently: a damaged section is dropped, never the whole UI state.
pub fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<LeftOff, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

/// Per-session tracking (not saved).
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    /// The source key, active photo, undo depth and view last frame.
    seen: Option<(String, Option<PhotoId>, usize, String)>,
    /// The photo marked "left off" in the source on screen (where the last visit ended).
    pub marker: Option<PhotoId>,
    /// [`entered`] ran since the last frame (it set the marker).
    entered: bool,
    /// Go back to the last spot on the first frame (set at launch: [`LightcraftApp::resume_left_off_at_launch`]).
    pub resume_at_launch: bool,
    /// When (egui time) a spot was last recorded and not yet written to `ui.json`.
    changed_at: Option<f64>,
    /// The active photo is only there by default (the first photo of a folder just opened, the
    /// launch photo, the one Develop picked with none active): looking at it isn't work, so it is
    /// not written down until the user picks a photo or edits something.
    fallback: Option<PhotoId>,
    /// `library.select` ran since the last frame: a photo the user chose (or a script did).
    pub(crate) user_select: bool,
    /// The photo `enter_develop` picked because none was active ([`note_auto_select`]).
    auto: Option<PhotoId>,
}

/// How long after a new spot `ui.json` is written (seconds).
pub const SAVE_DELAY: f64 = 1.0;

impl Tracker {
    /// A spot recorded at least [`SAVE_DELAY`] ago hasn't been written yet: true once (the
    /// caller writes `ui.json` now).
    pub fn save_due(&mut self, now: f64) -> bool {
        match self.changed_at {
            Some(t) if now - t >= SAVE_DELAY || now < t => {
                self.changed_at = None;
                true
            }
            _ => false,
        }
    }

    /// A spot was recorded and not written yet (on quit: write it).
    pub fn unsaved(&self) -> bool {
        self.changed_at.is_some()
    }
}

fn now_secs() -> i64 {
    web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn library_key(app: &LightcraftApp) -> String {
    app.session.library.as_ref().map(|l| l.dir.to_string_lossy().to_string()).unwrap_or_else(|| "memory".into())
}

/// The source on screen as a key (`album:3`, `folder:/Volumes/x/2024`, `all`…).
pub fn source_key(app: &LightcraftApp) -> String {
    match &app.session.source {
        LibrarySource::All => "all".into(),
        LibrarySource::RecentlyAdded => "recentlyAdded".into(),
        LibrarySource::PreviousImport => "previousImport".into(),
        LibrarySource::Album(a) => format!("album:{}", a.0),
        LibrarySource::Published(c) => format!("published:{c}"),
        LibrarySource::RecentlyDeleted => "recentlyDeleted".into(),
        LibrarySource::Picks => "picks".into(),
        LibrarySource::Folder => format!("folder:{}", app.session.browse.as_ref().map(|b| b.path.as_str()).unwrap_or("")),
        LibrarySource::Missing => "missing".into(),
        LibrarySource::LibraryFolder => format!("libraryFolder:{}", app.session.library_folder.as_deref().unwrap_or("")),
        LibrarySource::Merges => format!("merges:{}", serde_json::to_string(&app.session.merges_scope).unwrap_or_default()),
    }
}

/// The view on screen as a spot records it: `grid`, `square`, `detail` (the Library loupe), `edit`
/// (Develop), `compare`, `survey`, `people`, `map`, `galleries`. Older spots only know the first
/// three.
fn view_name(app: &LightcraftApp) -> String {
    match app.ui.view {
        ViewMode::PhotoGrid => "grid",
        ViewMode::SquareGrid => "square",
        ViewMode::Compare => "compare",
        ViewMode::Survey => "survey",
        ViewMode::People => "people",
        ViewMode::Map => "map",
        ViewMode::Galleries => "galleries",
        ViewMode::Detail | ViewMode::Reference if app.ui.module == crate::state::Module::Develop => "edit",
        ViewMode::Reference => "edit",
        ViewMode::Detail => "detail",
    }
    .into()
}

/// `enter_develop` picked `id` because no photo was active: that is a default, not a choice.
pub fn note_auto_select(app: &mut LightcraftApp, id: PhotoId) {
    app.left_off.auto = Some(id);
}

/// The engine's resume point for a source (Lightroom's last edit after a migration), if the
/// command exists and knows one.
fn engine_resume_point(app: &mut LightcraftApp, source: &str) -> Option<PhotoId> {
    lightcraft_engine::find_command("library.resumePoint")?;
    let p = if let Some(a) = source.strip_prefix("album:") {
        json!({"album": a.parse::<u64>().ok()?})
    } else if let Some(f) = source.strip_prefix("folder:").or_else(|| source.strip_prefix("libraryFolder:")) {
        json!({"folder": f})
    } else {
        json!({})
    };
    let r = app.session.execute("library.resumePoint", &p).ok()?;
    r.get("photoId").or_else(|| r.get("id")).and_then(Value::as_u64).map(PhotoId)
}

/// Where the last visit to the source on screen ended. For a folder: the newest spot left in it,
/// in a folder inside it, or in the folder around it — whichever was visited last and still shows
/// here (a shoot worked on through its camera folders lands where the work stopped, Bryan 11-24).
fn remembered(app: &mut LightcraftApp, source: &str) -> Option<PhotoId> {
    let lib = library_key(app);
    let key = format!("{lib}|{source}");
    let folder = |src: &str| src.strip_prefix("libraryFolder:").or_else(|| src.strip_prefix("folder:")).map(str::to_string);
    if let Some(here) = folder(source) {
        let related = |p: &str| same_folder(p, &here) || lightcraft_catalog::query::folder_within(p, &here) || lightcraft_catalog::query::folder_within(&here, p);
        let visible = app.session.visible();
        let best = app
            .ui
            .left_off
            .per_source
            .values()
            .filter(|s| s.library == lib && folder(&s.source).is_some_and(|p| related(&p)) && visible.contains(&PhotoId(s.photo)))
            .max_by_key(|s| s.at)
            .map(|s| PhotoId(s.photo));
        if best.is_some() {
            return best;
        }
    }
    match app.ui.left_off.per_source.get(&key) {
        Some(s) => Some(PhotoId(s.photo)),
        None => engine_resume_point(app, source),
    }
}

fn same_folder(a: &str, b: &str) -> bool {
    lightcraft_catalog::query::folder_key(a).trim_end_matches('/') == lightcraft_catalog::query::folder_key(b).trim_end_matches('/')
}

/// A folder or album was opened (`library.source` / `library.browse` from the side panel, a key,
/// the control channel): land on the photo the last visit there ended on, and mark it.
pub fn entered(app: &mut LightcraftApp) {
    app.left_off.entered = true;
    let source = source_key(app);
    let target = remembered(app, &source).filter(|t| app.session.visible().contains(t));
    app.left_off.marker = target;
    if let Some(t) = target
        && app.session.selection.active != Some(t)
    {
        let _ = app.session.execute("library.select", &json!({"ids": [t.0]}));
    }
}

/// Per frame: record where we are (and, when the source changed some other way — an import
/// showing its photos —, mark the remembered photo without moving the selection). `now`: egui
/// time, for writing `ui.json` about a second after the spot changes.
pub fn tick(app: &mut LightcraftApp, now: f64) {
    if std::mem::take(&mut app.left_off.resume_at_launch) {
        resume_at_launch(app);
    }
    let source = source_key(app);
    let lib = library_key(app);
    let changed = app.left_off.seen.as_ref().is_none_or(|(s, ..)| *s != format!("{lib}|{source}"));
    if changed && !app.left_off.entered {
        app.left_off.marker = remembered(app, &source).filter(|t| app.session.visible().contains(t));
    }
    app.left_off.entered = false;
    let active = app.session.selection.active;
    let undo = app.session.undo.len();
    let view = view_name(app);
    let state = (format!("{lib}|{source}"), active, undo, view.clone());
    // taken every frame, before any early return: a select that changed nothing (the active photo
    // clicked again, none. on an empty selection) must not leave the flag up for a later frame
    let user = std::mem::take(&mut app.left_off.user_select);
    let auto = std::mem::take(&mut app.left_off.auto);
    // the user picked the photo that was only there by default: now it is work
    let adopted = user && active.is_some() && app.left_off.fallback == active;
    if adopted {
        app.left_off.fallback = None;
    }
    if app.left_off.seen.as_ref() == Some(&state) && !adopted {
        return;
    }
    let (first, prev_active, prev_undo) = match &app.left_off.seen {
        Some((_, a, u, _)) => (false, *a, *u),
        None => (true, None, 0),
    };
    app.left_off.seen = Some(state);
    // Which photo is only there by default? The one a folder opens on by itself, the launch photo,
    // the one Develop picks with none active. A photo the user picked, or the one remembered for
    // this place, is not; neither is any photo once something is edited.
    if (changed || first || prev_active != active) && active.is_some() {
        let remembered_here = active.is_some() && app.left_off.marker == active;
        app.left_off.fallback = if user || remembered_here {
            None
        } else if changed || first || (auto.is_some() && auto == active) {
            active
        } else if prev_active != active {
            None
        } else {
            app.left_off.fallback
        };
    }
    let edited = !first && prev_undo != undo;
    if edited {
        app.left_off.fallback = None;
    }
    let Some(id) = active else { return };
    // the first frame only looks (nothing has happened yet: don't move "last" to it)
    if first && app.ui.left_off.last.is_some() {
        return;
    }
    // Opening a folder, a collection, or the merge. tab (which follows the sidebar) is a visit,
    // not work: only a photo picked in it counts. A photo shown by default isn't either.
    if (changed && !user && !edited) || app.left_off.fallback == Some(id) {
        return;
    }
    let spot = Spot { library: lib.clone(), source: source.clone(), photo: id.0, view, at: now_secs() };
    app.left_off.changed_at = Some(now);
    let per = &mut app.ui.left_off.per_source;
    per.insert(format!("{lib}|{source}"), spot.clone());
    if per.len() > MAX_SOURCES
        && let Some(oldest) = per.iter().min_by_key(|(_, s)| s.at).map(|(k, _)| k.clone())
    {
        per.remove(&oldest);
    }
    // throwaway sessions and Recently Deleted aren't places to go back to
    if !matches!(source.as_str(), "recentlyDeleted" | "missing") {
        app.ui.left_off.last = Some(spot);
    }
}

/// At launch: back to the last spot of this library — its folder or album, the photo and (unless
/// Settings ▸ General ▸ start in says otherwise) the view. Nothing happens when there is no spot for
/// this library or it can't be reached (a removed album or photo: the launch view stays).
fn resume_at_launch(app: &mut LightcraftApp) {
    let lib = library_key(app);
    if !app.ui.left_off.last.as_ref().is_some_and(|s| s.library == lib) {
        return;
    }
    let keep_view = app.ui.settings.startup_view != crate::state::StartupView::Last;
    let (view, right, module) = (app.ui.view, app.ui.right, app.ui.module);
    match resume(app) {
        Ok(_) => {
            if keep_view {
                // (a saved Develop module under a grid start-up view: the grid is the Library's)
                // (the merge. page is a grid)
                app.ui.view = if app.merge_tab && !matches!(view, ViewMode::PhotoGrid | ViewMode::SquareGrid) { ViewMode::PhotoGrid } else { view };
                app.ui.right = right;
                app.ui.module = crate::state::Module::of_view(app.ui.view).unwrap_or(module);
                app.ui.sync_module();
            }
        }
        Err(e) => log::info!("left off: not resumed at launch: {e}"),
    }
}

/// `view.resumeLastLeftOff`: back to the last photo worked on — its folder or album, the photo and
/// the view (grid, loupe or editing).
pub fn resume(app: &mut LightcraftApp) -> Result<Value, String> {
    let lib = library_key(app);
    let spot = match app.ui.left_off.last.clone().filter(|s| s.library == lib) {
        Some(s) => s,
        None => {
            // nothing recorded in this library yet: the engine's resume point (Lightroom's last edit)
            let id = engine_resume_point(app, "").ok_or("Nothing to go back to yet: LightCraft remembers the photo you work on from now on")?;
            Spot { library: lib, source: "all".into(), photo: id.0, view: "edit".into(), at: 0 }
        }
    };
    let id = PhotoId(spot.photo);
    if app.session.catalog.photo(id).is_none() && !spot.source.starts_with("folder:") {
        return Err("The photo you left off on is no longer in the library".into());
    }
    if source_key(app) != spot.source {
        let r = if let Some(a) = spot.source.strip_prefix("album:") {
            app.run("library.source", json!({"kind": "album", "id": a.parse::<u64>().unwrap_or(0)}))
        } else if let Some(c) = spot.source.strip_prefix("published:") {
            app.run("library.source", json!({"kind": "published", "id": c.parse::<u64>().unwrap_or(0)}))
        } else if let Some(f) = spot.source.strip_prefix("folder:") {
            app.run("library.browse", json!({"path": f}))
        } else if let Some(f) = spot.source.strip_prefix("libraryFolder:") {
            app.run("library.source", json!({"kind": "libraryFolder", "path": f}))
        } else if let Some(scope) = spot.source.strip_prefix("merges:") {
            // a merges. view: its scope as source_key wrote it ({kind: folder|album|all, …})
            let mut p = serde_json::from_str::<serde_json::Value>(scope).unwrap_or_else(|_| json!({}));
            p["kind"] = json!("merges");
            app.run("library.source", p)
        } else {
            app.run("library.source", json!({"kind": spot.source}))
        };
        // the folder or album is gone: show the photo among all photos
        if r.is_err() {
            let _ = app.run("library.source", json!({"kind": "all"}));
        }
    }
    if !app.session.visible().contains(&id) {
        let _ = app.run("library.clearFilter", json!({}));
    }
    app.run("library.select", json!({"ids": [id.0]}))?;
    // a merges. spot comes back as the merge. page: its tab on, the grid
    let merges = spot.source.starts_with("merges:") && matches!(app.session.source, LibrarySource::Merges);
    app.merge_tab = merges;
    let view = if merges { "grid" } else { spot.view.as_str() };
    let library = |app: &mut LightcraftApp, v: ViewMode| {
        if app.ui.module != crate::state::Module::Library || app.ui.view != v {
            crate::menus::enter_library(app, Some(v));
        }
    };
    match view {
        "grid" => {
            if app.ui.module != crate::state::Module::Library || !matches!(app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid) {
                crate::menus::enter_library(app, Some(ViewMode::PhotoGrid));
            }
        }
        "square" => library(app, ViewMode::SquareGrid),
        "compare" => library(app, ViewMode::Compare),
        "survey" => library(app, ViewMode::Survey),
        "people" => library(app, ViewMode::People),
        "map" => {
            let _ = app.run("view.map", json!({}));
        }
        "galleries" => {
            let _ = app.run("view.galleries", json!({}));
        }
        // Develop: the loupe with the develop panels
        "edit" => {
            crate::menus::enter_develop(app);
            if app.ui.right == RightPanel::None {
                app.ui.right = RightPanel::Edit;
            }
        }
        _ => crate::menus::enter_library(app, Some(ViewMode::Detail)),
    }
    // the source is entered here: the tracker sees it next frame, lands (already there) and marks it
    Ok(json!({"photo": id.0, "source": spot.source, "view": spot.view}))
}

/// The "left off" badge over a thumbnail (`img` = the drawn image) — a ring with a dot and the words,
/// and an accent outline round the image.
pub fn paint_marker(p: &egui::Painter, t: &crate::theme::Tokens, img: egui::Rect, small: bool) {
    use egui::{Align2, Color32, Stroke, StrokeKind, pos2, vec2};
    p.rect_stroke(img, 0.0, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    let font = t.semibold(if small { 8.5 } else { 9.5 });
    let g = p.layout_no_wrap(crate::i18n::tr("left off").to_string(), font, Color32::WHITE);
    let h = if small { 14.0 } else { 16.0 };
    let pill = egui::Rect::from_min_size(pos2(img.left() + 4.0, img.top() + 4.0), vec2(g.size().x + h + 8.0, h));
    if pill.width() > img.width() - 8.0 {
        // too narrow for words: the ring alone
        p.circle_stroke(pos2(img.left() + 4.0 + h / 2.0, img.top() + 4.0 + h / 2.0), h / 2.0 - 2.0, Stroke::new(1.5, t.accent));
        return;
    }
    p.rect_filled(pill, h / 2.0, Color32::from_black_alpha(190));
    let c = pos2(pill.left() + h / 2.0 + 1.0, pill.center().y);
    p.circle_stroke(c, h / 2.0 - 3.5, Stroke::new(1.5, t.accent));
    p.circle_filled(c, 2.0, t.accent);
    p.galley(pos2(c.x + h / 2.0, pill.center().y - g.size().y / 2.0), g, Color32::WHITE);
    let _ = Align2::LEFT_CENTER;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_saved_state_is_ignored() {
        // a damaged or older ui.json section never fails the whole UI state
        let v: LeftOff = serde_json::from_value(json!({"perSource": {"x": {"photo": 3}}, "last": null, "junk": 1})).unwrap();
        assert_eq!(v.per_source.get("x").map(|s| s.photo), Some(3));
        assert!(serde_json::from_value::<LeftOff>(json!({"perSource": 5})).is_err());
    }
}
