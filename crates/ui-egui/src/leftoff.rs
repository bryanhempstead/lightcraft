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
}

fn now() -> i64 {
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
        LibrarySource::RecentlyDeleted => "recentlyDeleted".into(),
        LibrarySource::Picks => "picks".into(),
        LibrarySource::Folder => format!("folder:{}", app.session.browse.as_ref().map(|b| b.path.as_str()).unwrap_or("")),
        LibrarySource::Missing => "missing".into(),
        LibrarySource::LibraryFolder => format!("libraryFolder:{}", app.session.library_folder.as_deref().unwrap_or("")),
    }
}

fn view_name(app: &LightcraftApp) -> String {
    match app.ui.view {
        ViewMode::PhotoGrid | ViewMode::SquareGrid => "grid",
        _ if app.ui.right != RightPanel::None => "edit",
        _ => "detail",
    }
    .into()
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

/// Where the last visit to the source on screen ended.
fn remembered(app: &mut LightcraftApp, source: &str) -> Option<PhotoId> {
    let key = format!("{}|{source}", library_key(app));
    match app.ui.left_off.per_source.get(&key) {
        Some(s) => Some(PhotoId(s.photo)),
        None => engine_resume_point(app, source),
    }
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
/// showing its photos —, mark the remembered photo without moving the selection).
pub fn tick(app: &mut LightcraftApp) {
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
    if app.left_off.seen.as_ref() == Some(&state) {
        return;
    }
    let first = app.left_off.seen.is_none();
    app.left_off.seen = Some(state);
    let Some(id) = active else { return };
    // the first frame only looks (nothing has happened yet: don't move "last" to it)
    if first && app.ui.left_off.last.is_some() {
        return;
    }
    let spot = Spot { library: lib.clone(), source: source.clone(), photo: id.0, view, at: now() };
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
        } else if let Some(f) = spot.source.strip_prefix("folder:") {
            app.run("library.browse", json!({"path": f}))
        } else if let Some(f) = spot.source.strip_prefix("libraryFolder:") {
            app.run("library.source", json!({"kind": "libraryFolder", "path": f}))
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
    match spot.view.as_str() {
        "grid" => {
            if !matches!(app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid) {
                app.ui.view = ViewMode::PhotoGrid;
            }
        }
        "edit" => {
            app.ui.view = ViewMode::Detail;
            if app.ui.right == RightPanel::None {
                app.ui.right = RightPanel::Edit;
            }
        }
        _ => app.ui.view = ViewMode::Detail,
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
