//! The left "My Photos" panel: library sources, albums tree, and date groups.

use egui::{Align2, Rect, Sense, pos2, vec2};
use lightcraft_catalog::{Album, AlbumId, FolderNode, KeywordNode};
use lightcraft_engine::LibrarySource;
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{icon_button, register};

/// How wide the sidebar's content needs to be, from the widest row of the last frame: rows are
/// drawn at this width (or the panel's, if wider), and the sidebar scrolls sideways when it
/// exceeds the panel. One frame behind, which needs no second layout pass.
pub(crate) fn content_width(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp::<f32>(egui::Id::new("left-content-width"))).unwrap_or(0.0)
}

/// The most a row asks for its name: longer ones are cut when there is no more room.
const MAX_NAME_NEED: f32 = 140.0;

/// A row says how wide it needs to be.
fn note_width(ui: &egui::Ui, w: f32) {
    ui.data_mut(|d| {
        let m = d.get_temp_mut_or_insert_with::<f32>(egui::Id::new("left-content-width-next"), || 0.0);
        *m = m.max(w);
    });
}

fn row(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    id: &str,
    icon: Icon,
    label: &str,
    count: Option<usize>,
    selected: bool,
    indent: f32,
) -> egui::Response {
    row_named(app, ui, id, icon, label, None, count, selected, indent)
}

/// [`row`] whose spoken name is `spoken` when the painted `label` is a shortened form of it.
#[allow(clippy::too_many_arguments)]
fn row_named(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    id: &str,
    icon: Icon,
    label: &str,
    spoken: Option<&str>,
    count: Option<usize>,
    selected: bool,
    indent: f32,
) -> egui::Response {
    row_ex(app, ui, id, icon, label, spoken, count, selected, indent, &RowExtra::default())
}

/// How a row differs from a plain one.
#[derive(Default)]
struct RowExtra {
    /// Drawn quieter: a folder whose disk is not connected.
    dim: bool,
    /// Text at the right edge instead of the count (a disk's `1.2 / 2 TB`).
    badge: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn row_ex(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    id: &str,
    icon: Icon,
    label: &str,
    spoken: Option<&str>,
    count: Option<usize>,
    selected: bool,
    indent: f32,
    extra: &RowExtra,
) -> egui::Response {
    let label = if matches!(id, "all" | "recentlyAdded" | "previousImport" | "quick" | "picks" | "missing" | "recentlyDeleted") {
        crate::i18n::tr(label)
    } else {
        label
    };
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 29.0), Sense::click());
    register(ui.ctx(), format!("source:{id}"), r);
    let said = spoken.unwrap_or(label);
    let name = match count {
        Some(n) => crate::i18n::tr_format!("{label}, {n} photos", label = said, n = n),
        None => said.to_string(),
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &name));
    // what is visible of the row: when the sidebar is scrolled sideways its bars and counts end at
    // the panel's edge, not at the end of the (wider) content
    let visible_right: f32 = ui.data(|d| d.get_temp(egui::Id::new("left-visible-right"))).unwrap_or(f32::MAX);
    let edge = r.right().min(visible_right);
    let inner = Rect::from_min_max(r.min + vec2(8.0, 0.0), pos2((r.right() - 8.0).min(edge - 8.0).max(r.left() + 8.0), r.bottom()));
    if selected {
        ui.painter().rect_filled(inner, 4.0, t.canvas);
        register(ui.ctx(), format!("highlight:{id}"), inner);
    } else if resp.hovered() {
        ui.painter().rect_filled(inner, 4.0, t.hover.gamma_multiply(0.6));
    }
    paint(
        ui.painter(),
        Rect::from_min_size(pos2(r.left() + 18.0 + indent, r.center().y - 8.0), vec2(16.0, 16.0)),
        icon,
        if selected {
            t.text
        } else if extra.dim {
            t.text_dim
        } else {
            t.icon
        },
    );
    let font = t.font(13.5);
    let color = if selected {
        t.text
    } else if extra.dim {
        t.text_dim
    } else {
        t.text_label
    };
    let count_galley = match &extra.badge {
        Some(b) => Some(ui.painter().layout_no_wrap(b.clone(), t.font(12.0), t.text_dim)),
        None => count.filter(|_| app.ui.show_counts).map(|n| ui.painter().layout_no_wrap(n.to_string(), t.font(12.5), t.text_dim)),
    };
    let label_left = r.left() + 42.0 + indent;
    let count_left = count_galley.as_ref().map_or(edge - 18.0, |g| edge - 18.0 - g.size().x);
    // the name gives way to the count: cut with an ellipsis, in full on hover
    let room = count_left - 8.0 - label_left;
    let measure = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), color).size().x;
    let full_w = measure(label);
    let shown = if full_w <= room { label.to_string() } else { crate::widgets::elide_head(label, room.max(0.0), measure) };
    let label_rect = ui.painter().text(pos2(label_left, r.center().y), Align2::LEFT_CENTER, &shown, font.clone(), color);
    register(ui.ctx(), format!("label:{id}"), label_rect);
    let resp = if shown != label { resp.on_hover_text(label) } else { resp };
    // a row asks for room for its name up to a share of a panel, so one very long name does not
    // make everything scroll: depth does that
    let mut needed = 42.0 + indent + full_w.min(MAX_NAME_NEED) + 18.0;
    if let Some(galley) = count_galley {
        let rect = Rect::from_min_size(pos2(edge - 18.0 - galley.size().x, r.center().y - galley.size().y / 2.0), galley.size());
        needed += rect.width() + 16.0;
        register(ui.ctx(), format!("count:{id}"), rect);
        ui.painter().galley(rect.min, galley, t.text_dim);
    }
    note_width(ui, needed);
    let _ = app;
    resp
}

/// A collapsible section header (Albums, Local, By Date, Keywords): the bold title with a
/// disclosure chevron after it; a click folds or unfolds the section (kept in the UI state, so it
/// survives restarts). Returns the header's rect and whether the section is now open.
fn sidebar_section_header(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, title: &str) -> (Rect, bool) {
    let t = Tokens::get(ui.ctx());
    let title = crate::i18n::tr(title);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    register(ui.ctx(), format!("sidebarSection:{id}"), r);
    if resp.clicked() {
        app.ui.toggle_sidebar_section(id);
    }
    let open = !app.ui.sidebar_section_collapsed(id);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    let text = ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, title, t.semibold(13.5), t.text_label);
    let c = pos2(text.right() + 10.0, r.center().y);
    let col = if resp.hovered() { t.text } else { t.text_dim };
    let pts = if open {
        vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
    } else {
        vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    (r, open)
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let frame = egui::Frame::NONE.fill(t.chrome).stroke(egui::Stroke::new(1.0, t.divider));
    let width = app.ui.left_width;
    let resized = super::resizable_side(ui, true, "left_panel", frame, width, crate::state::LEFT_WIDTH, 0.0, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        // Lightroom Classic's Library left panel: Navigator, then Catalog, Folders and
        // Collections (scrolling), then Import… / Export… at the bottom
        navigator(app, ui);
        let counts = app.caches.counts(&app.session.catalog);
        let (total, picks, deleted, previous) = (counts.total, counts.picks, counts.deleted, counts.previous_import);
        let list_h = (ui.available_height() - BOTTOM_BAR).max(0.0);
        egui::ScrollArea::both().id_salt("left-scroll").auto_shrink([false, false]).max_height(list_h).show_viewport(ui, |ui, viewport| {
            // rows are as wide as the widest one needs (last frame), at least the panel
            let wide = viewport.width().max(content_width(ui.ctx()));
            ui.set_min_width(wide);
            ui.set_max_width(wide);
            ui.data_mut(|d| {
                d.insert_temp(egui::Id::new("left-content-width-next"), 0.0f32);
                // where the visible part of the content ends (screen x), for what stays at the edge
                d.insert_temp(egui::Id::new("left-visible-right"), ui.cursor().left() + viewport.max.x);
            });
            let src = app.session.source;
            let (_, catalog_open) = sidebar_section_header(app, ui, "catalog", "Catalog");
            if catalog_open {
                catalog_rows(app, ui, total, picks, previous, deleted, src);
            }
            folders_section(app, ui, viewport);
            collections_section(app, ui, viewport);
            // Local (a folder browsed without importing) only while one is shown
            if src == LibrarySource::Folder {
                ui.add_space(10.0);
                local_section(app, ui);
            }
            ui.add_space(12.0);
            // what the rows asked for becomes next frame's width
            let next = ui.data(|d| d.get_temp::<f32>(egui::Id::new("left-content-width-next"))).unwrap_or(0.0);
            if (next - content_width(ui.ctx())).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(egui::Id::new("left-content-width"), next));
                ui.ctx().request_repaint();
            }
        });
        bottom_buttons(app, ui);
    });
    if let Some(w) = resized {
        app.ui.left_width = w;
    }
}

/// Height of the Import… / Export… bar at the bottom of the panel.
const BOTTOM_BAR: f32 = 48.0;

/// Import… and Export… under the panel, as in Lightroom Classic's Library module.
fn bottom_buttons(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), BOTTOM_BAR), Sense::hover());
    ui.painter().hline(bar.x_range(), bar.top(), egui::Stroke::new(1.0, t.divider));
    // two equal buttons, 12 px from the edges and 8 px apart
    let w = ((bar.width() - 24.0 - 8.0) / 2.0).max(40.0);
    let h = 28.0;
    let y = bar.center().y - h / 2.0;
    for (i, (id, label, command)) in [("import", "Import…", "file.addPhotos"), ("export", "Export…", "dialog.export")].into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(bar.left() + 12.0 + i as f32 * (w + 8.0), y), vec2(w, h));
        let resp = ui.put(r, egui::Button::new(egui::RichText::new(crate::i18n::tr(label)).font(t.font(13.0))).min_size(vec2(w, h)));
        register(ui.ctx(), format!("button:left-{id}"), r);
        if resp.clicked()
            && let Err(e) = app.run(command, json!({}))
        {
            app.toast(ui.ctx(), e);
        }
    }
}

/// Navigator: the active photo, small, at the top of the panel (a click shows it in Loupe).
fn navigator(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (_, open) = sidebar_section_header(app, ui, "navigator", "Navigator");
    if !open {
        return;
    }
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, (w * 0.62).clamp(90.0, 200.0)), Sense::click());
    register(ui.ctx(), "navigator".to_string(), r);
    let frame = Rect::from_min_max(r.min + vec2(12.0, 0.0), r.max - vec2(12.0, 10.0));
    ui.painter().rect_filled(frame, 2.0, t.canvas);
    let Some(id) = app.session.active() else {
        ui.painter().text(frame.center(), Align2::CENTER_CENTER, crate::i18n::tr("No photo selected"), t.font(12.5), t.text_dim);
        return;
    };
    let ppp = ui.ctx().pixels_per_point();
    super::grid::request_thumb(app, id, super::grid::thumb_px(frame.width().max(frame.height()), ppp), 8);
    if let Some(tex) = app.renderer.thumb(id) {
        let fit = super::detail::fit_texture_rect(frame.shrink(4.0), tex.size);
        ui.painter().image(tex.tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
    if resp.clicked() {
        app.ui.view = crate::state::ViewMode::Detail;
    }
}

/// How many library photos have no file (Local browse records are not checked; see
/// `cmd::missing::checked_path`). Checking stats every file, which on a network share takes
/// seconds, so it runs on a worker thread: the count shown is the last finished one, refreshed
/// at most every 5 s, and at once (after the running check) when the catalog changed.
fn missing_count(app: &mut LightcraftApp, ui: &mut egui::Ui) -> usize {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
    #[derive(Clone, Default)]
    struct Job {
        n: std::sync::Arc<AtomicUsize>,
        running: std::sync::Arc<AtomicBool>,
        at: f64,
        rev: u64,
    }
    let id = egui::Id::new("missing-count");
    let now = ui.input(|i| i.time);
    let rev = app.session.catalog.revision;
    let mut job: Job = ui.data(|d| d.get_temp(id)).unwrap_or(Job { rev: u64::MAX, at: f64::MIN, ..Default::default() });
    let fresh = now - job.at < 5.0 && job.rev == rev;
    if !fresh && !job.running.load(Relaxed) {
        // the same scope as the Missing Photos view: library photos only, never Local browse records
        let paths = lightcraft_engine::cmd::missing::candidates(&app.session.catalog);
        job.running.store(true, Relaxed);
        job.at = now;
        job.rev = rev;
        let (n, running, ctx) = (job.n.clone(), job.running.clone(), ui.ctx().clone());
        let work = move || {
            let missing = if cfg!(target_arch = "wasm32") { 0 } else { paths.iter().filter(|p| !std::path::Path::new(p).exists()).count() };
            n.store(missing, Relaxed);
            running.store(false, Relaxed);
            ctx.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(work);
        #[cfg(target_arch = "wasm32")]
        work();
        ui.data_mut(|d| d.insert_temp(id, job.clone()));
    }
    job.n.load(Relaxed)
}

/// Folders on this computer to browse without adding (Lightroom's Local): Pictures, Desktop,
/// Downloads, the home folder, the folder being browsed, and Browse Folder….
fn local_section(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let (_, open) = sidebar_section_header(app, ui, "local", "Local");
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).unwrap_or_default();
    let mut builtin: Vec<(String, String)> = Vec::new();
    if !home.is_empty() {
        for (name, sub) in [("Pictures", "Pictures"), ("Desktop", "Desktop"), ("Downloads", "Downloads"), ("Home", "")] {
            // joined with the platform's separator, like the paths browsing gives back
            let p = if sub.is_empty() { home.clone() } else { std::path::Path::new(&home).join(sub).to_string_lossy().to_string() };
            // (checked off the UI thread: a home folder can be on a network share)
            if fs_cached(ui, "is-dir", &p, 5.0, |p| std::path::Path::new(p).is_dir()) == Some(true) {
                builtin.push((crate::i18n::tr(name).to_string(), p));
            }
        }
    }
    let browsing = app.session.browse.clone().filter(|_| app.session.source == LibrarySource::Folder);
    let current = browsing.as_ref().map(|b| b.path.clone());
    let local = local_places(builtin, &app.ui.local_roots, current.as_deref(), app.ui.local_browse_root.as_deref(), &app.ui.hidden_locations);
    if current.is_some() {
        app.ui.local_browse_root = local.browse_root.clone();
    }
    if !open {
        ui.add_space(10.0);
        return;
    }
    for (i, (name, path)) in local.places.iter().enumerate() {
        let transient = local.browse_root.as_deref() == Some(path.as_str());
        let reveal = local.owner == Some(i);
        folder_tree(app, ui, name, path, 0.0, current.as_deref(), reveal, transient);
    }
    if app.services.pick_folder.is_some() && row(app, ui, "local:browse", Icon::Plus, crate::i18n::tr("Browse Folder…"), None, false, 0.0).clicked()
    {
        let picked = app.services.pick_folder.as_mut().and_then(|f| f());
        if let Some(path) = picked {
            match app.run("library.browse", json!({"path": path})) {
                // the picked folder stays in Local (and comes back if it was hidden)
                Ok(r) => {
                    let dir = r["path"].as_str().unwrap_or(&path).to_string();
                    let _ = app.run("local.addRoot", json!({"path": dir}));
                }
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
    }
    let hidden = app.ui.hidden_locations.len();
    if hidden > 0 {
        let label = crate::i18n::tr_format!("Show {hidden} hidden location{}", if hidden == 1 { "" } else { "s" }, hidden = hidden);
        if row(app, ui, "local:restoreHidden", Icon::Folder, &label, None, false, 0.0)
            .on_hover_text(crate::i18n::tr("Put the locations you removed from Local back (no files change)"))
            .clicked()
        {
            let _ = app.run("local.restoreHidden", json!({}));
        }
    }
    ui.add_space(10.0);
}

/// Whether two paths name the same folder, however they are spelled (separators, trailing
/// slash, `.`/`..`, drive-letter case; see `lightcraft_catalog::query::folder_key`).
pub(crate) fn same_folder(a: &str, b: &str) -> bool {
    a == b || lightcraft_catalog::query::folder_key(a) == lightcraft_catalog::query::folder_key(b)
}

/// Local's top-level folders and how the folder being browsed sits among them.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LocalPlaces {
    /// (label, path) of each top-level folder, in order.
    pub places: Vec<(String, String)>,
    /// The top-level folder the browsed folder lies in (the innermost one): its tree opens on
    /// the way down to it. None when that way passes through a hidden folder.
    pub owner: Option<usize>,
    /// A folder listed only for this session because the browsed folder is in no saved
    /// location (browsed from a breadcrumb, the CLI…); it stays while browsing below it.
    pub browse_root: Option<String>,
}

fn folder_label(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Local's top-level folders, in order: the built-in places, the folders kept with Browse
/// Folder… / Keep in Local (`saved`), then — only when the browsed folder lies in none of those —
/// a session root for it: the previous one (`browse_root`) while browsing stays below it, else
/// the browsed folder itself. A folder inside a listed one is shown inside that one's tree (the
/// root stays; its siblings stay reachable), never as a root of its own. Each folder is listed
/// once however its path is spelled (the first spelling wins), and hidden ones are left out —
/// they stay reachable through breadcrumbs and Browse Folder….
pub(crate) fn local_places(
    builtin: Vec<(String, String)>,
    saved: &[String],
    browsing: Option<&str>,
    browse_root: Option<&str>,
    hidden: &[String],
) -> LocalPlaces {
    use lightcraft_catalog::query::{folder_key, folder_within};
    let is_hidden = |p: &str| hidden.iter().any(|h| same_folder(h, p));
    let mut places = builtin;
    for path in saved {
        if !places.iter().any(|(_, p)| same_folder(p, path)) {
            places.push((folder_label(path), path.clone()));
        }
    }
    places.retain(|(_, p)| !is_hidden(p));
    let mut out = LocalPlaces::default();
    if let Some(c) = browsing
        && !places.iter().any(|(_, p)| folder_within(c, p))
    {
        let root = browse_root.filter(|r| folder_within(c, r)).unwrap_or(c);
        if !is_hidden(root) {
            places.push((folder_label(root), root.to_string()));
            out.browse_root = Some(root.to_string());
        }
    }
    if let Some(c) = browsing {
        // A tree never opens on the way down through a hidden folder: hiding a kept folder
        // beneath Home would otherwise reveal it again inside Home's (possibly huge) tree.
        let through_hidden = |p: &str| hidden.iter().any(|h| folder_within(c, h) && folder_within(h, p));
        out.owner = places
            .iter()
            .enumerate()
            .filter(|(_, (_, p))| folder_within(c, p) && !through_hidden(p))
            .max_by_key(|(_, (_, p))| folder_key(p).len())
            .map(|(i, _)| i);
    }
    out.places = places;
    out
}

/// The subfolders of `path` (not hidden ones), sorted; listed on a worker thread at most every
/// 2 s per folder (empty until the first listing).
fn subfolders(ui: &egui::Ui, path: &str) -> Vec<(String, String)> {
    fs_cached(ui, "subfolders", path, 2.0, list_subfolders).unwrap_or_default()
}

fn list_subfolders(path: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    (!name.starts_with('.')).then(|| (name, e.path().to_string_lossy().to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// How many [`fs_cached`] answers for `ctx` are being worked out right now. Rows appear (and
/// the sidebar below them moves) when they land, so the headless driver counts them as pending
/// work and waits for them before acting on widget positions.
pub(crate) fn fs_cached_running(ctx: &egui::Context) -> usize {
    fs_running_counter(ctx).load(std::sync::atomic::Ordering::Acquire)
}

fn fs_running_counter(ctx: &egui::Context) -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<std::sync::Arc<std::sync::atomic::AtomicUsize>>(egui::Id::new("fs-cached-running")).clone())
}

/// A file-system answer for `path` (`f(path)`), kept per `kind` and path and refreshed on a worker
/// thread at most every `every` seconds: a folder on a sleeping NAS, a dropped share or a
/// spinning-up drive never blocks a frame. `None` until the first answer arrives.
pub(crate) fn fs_cached<T: Clone + Send + 'static>(ui: &egui::Ui, kind: &'static str, path: &str, every: f64, f: fn(&str) -> T) -> Option<T> {
    struct Entry<T> {
        value: Option<T>,
        at: Option<f64>,
        running: bool,
    }
    type Cell<T> = std::sync::Arc<std::sync::Mutex<Entry<T>>>;
    let id = egui::Id::new(("fs-cached", kind, path.to_string()));
    let now = ui.input(|i| i.time);
    let cell: Cell<T> = match ui.data(|d| d.get_temp::<Cell<T>>(id)) {
        Some(c) => c,
        None => {
            let c: Cell<T> = std::sync::Arc::new(std::sync::Mutex::new(Entry { value: None, at: None, running: false }));
            ui.data_mut(|d| d.insert_temp(id, c.clone()));
            c
        }
    };
    let lock = |c: &Cell<T>| c.lock().unwrap_or_else(std::sync::PoisonError::into_inner).value.clone();
    let start = {
        let mut e = cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let due = !e.running && e.at.is_none_or(|at| now - at >= every || now < at);
        if due {
            e.running = true;
            e.at = Some(now);
        }
        due
    };
    if start {
        use std::sync::atomic::Ordering;
        let running = fs_running_counter(ui.ctx());
        running.fetch_add(1, Ordering::AcqRel);
        let (out, path, repaint, done) = (cell.clone(), path.to_string(), ui.ctx().clone(), running.clone());
        let work = move || {
            let v = f(&path);
            let mut e = out.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            e.value = Some(v);
            e.running = false;
            drop(e);
            // after the answer is stored: a frame that sees the count drop also sees the answer
            done.fetch_sub(1, Ordering::AcqRel);
            repaint.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if std::thread::Builder::new().name("lc-fs-list".into()).spawn(work).is_err() {
            cell.lock().unwrap_or_else(std::sync::PoisonError::into_inner).running = false;
            running.fetch_sub(1, Ordering::AcqRel);
        }
        #[cfg(target_arch = "wasm32")]
        work();
    }
    lock(&cell)
}

/// A folder on disk with a disclosure triangle: click browses it, the triangle lists its
/// subfolders. In the tree that holds the folder being browsed (`reveal`), the folders on the
/// way down to it open whenever the browsed folder changes, so it shows highlighted in place.
/// `transient`: a top-level row listed for this session only (it offers Keep in Local).
#[allow(clippy::too_many_arguments)]
fn folder_tree(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    name: &str,
    path: &str,
    indent: f32,
    current: Option<&str>,
    reveal: bool,
    transient: bool,
) {
    let t = Tokens::get(ui.ctx());
    let open_id = egui::Id::new(("folder-open", path.to_string()));
    let sel = current.is_some_and(|c| same_folder(c, path));
    let on_the_way = !sel && current.is_some_and(|c| lightcraft_catalog::query::folder_within(c, path));
    let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
    if reveal && on_the_way {
        // opened once per browsed folder: collapsing it again afterwards sticks
        let revealed_id = egui::Id::new(("folder-revealed", path.to_string()));
        let target = current.map(str::to_string);
        if ui.data(|d| d.get_temp::<Option<String>>(revealed_id)) != Some(target.clone()) {
            open = true;
            ui.data_mut(|d| {
                d.insert_temp(open_id, true);
                d.insert_temp(revealed_id, target);
            });
        }
    }
    let resp = row(app, ui, &format!("local:{path}"), Icon::Folder, name, None, sel, indent + 12.0).on_hover_text(path);
    let c = pos2(resp.rect.left() + 10.0 + indent, resp.rect.center().y);
    let tri = Rect::from_center_size(c, vec2(14.0, 14.0));
    let tr = ui.interact(tri, egui::Id::new(("folder-tri", path.to_string())), Sense::click());
    register(ui.ctx(), format!("folderToggle:{path}"), tri);
    let col = if tr.hovered() { t.text } else { t.text_dim };
    let pts = if open {
        vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
    } else {
        vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    if tr.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(open_id, open));
    } else if resp.clicked()
        && let Err(e) = app.run("library.browse", json!({"path": path}))
    {
        app.toast(ui.ctx(), e);
    }
    resp.context_menu(|ui| {
        if (transient || indent > 0.0)
            && ui
                .button(if transient { "Keep in Local" } else { "Add to Local" })
                .on_hover_text(crate::i18n::tr("List this folder in Local from now on"))
                .clicked()
        {
            let _ = app.run("local.addRoot", json!({"path": path}));
            ui.close();
        }
        if indent == 0.0
            && ui
                .button(crate::i18n::tr("Remove from Local"))
                .on_hover_text(crate::i18n::tr("Hides this shortcut only; the folder and its photos stay as they are"))
                .clicked()
        {
            let _ = app.run("local.hide", json!({"path": path}));
            ui.close();
        }
        if ui.button(crate::i18n::tr("Rename Folder…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: crate::i18n::tr_format!("Rename “{name}”", name = name),
                hint: "Folder name (renamed on disk; its photos follow)".into(),
                value: name.to_string(),
                command: "folder.rename".into(),
                params: json!({"path": path}),
                key: "name".into(),
            });
        }
        if app.services.pick_folder.is_some() && ui.button(crate::i18n::tr("Move Folder To…")).clicked() {
            let into = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(into) = into {
                match app.run("folder.move", json!({"path": path, "into": into})) {
                    Ok(r) => app.toast(ui.ctx(), crate::i18n::tr_format!("Moved; {} photo(s) relinked", r["relinked"])),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
        }
        if app.services.reveal.is_some()
            && ui.button(crate::i18n::tr(crate::menus::reveal_label())).clicked()
            && let Some(f) = app.services.reveal.as_mut()
        {
            let _ = f(path);
        }
    });
    if open && indent < 12.0 * 8.0 {
        for (n, p) in subfolders(ui, path) {
            folder_tree(app, ui, &n, &p, indent + 12.0, current, reveal, false);
        }
    }
}

/// By Date and Keywords count every photo in the library, so choosing a row shows those photos
/// from All Photos, not from whatever album or folder happened to be open, where they could be
/// missing (issue #341). Only when choosing (`on`), not when clearing the row again.
fn browse_all_photos(app: &mut LightcraftApp, on: bool) {
    if on && app.session.source != LibrarySource::All {
        let _ = app.run("library.source", json!({"kind": "all"}));
    }
}

fn albums_tree(app: &mut LightcraftApp, ui: &mut egui::Ui, all: &[Album], parent: Option<AlbumId>, indent: f32) {
    let mut kids: Vec<&Album> = all.iter().filter(|a| a.parent == parent).collect();
    kids.sort_by_key(|a| (!a.folder, a.name.to_lowercase()));
    for a in kids {
        if a.folder {
            let open_id = egui::Id::new(("folder-open", a.id.0));
            let open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(true);
            let resp = row(app, ui, &format!("folder:{}", a.id.0), Icon::CollectionSet, &a.name, None, false, indent);
            if resp.clicked() {
                ui.data_mut(|d| d.insert_temp(open_id, !open));
            }
            folder_menu(app, &resp, a);
            if open {
                albums_tree(app, ui, all, Some(a.id), indent + 16.0);
            }
        } else {
            let sel = app.session.source == LibrarySource::Album(a.id);
            let icon = if a.is_smart() { Icon::SmartAlbum } else { Icon::Album };
            // cached: a smart album's count scans the catalog
            let now = (app.session.clock)();
            let n = app.caches.album_counts(&app.session.catalog, &now).get(&a.id).copied().unwrap_or(0);
            // the album B adds to is marked "+"
            let target =
                app.session.target_album.filter(|t| app.session.catalog.album(*t).is_some()).or_else(|| app.session.catalog.quick_collection());
            let label = if target == Some(a.id) { format!("{} +", a.name) } else { a.name.clone() };
            let mut resp = row(app, ui, &format!("album:{}", a.id.0), icon, &label, Some(n), sel, indent);
            if !a.is_smart() {
                drop_target(app, ui, &resp, a);
            }
            if let Some(rules) = &a.smart {
                resp = resp.on_hover_text(crate::i18n::tr_format!("Smart album: {}", crate::i18n::filter_label(rules, &app.session.catalog)));
            }
            if resp.clicked() {
                let _ = app.run("library.source", json!({"kind": "album", "id": a.id.0}));
            }
            folder_menu(app, &resp, a);
        }
    }
}

/// An album row while photos are dragged from the grid: highlighted under the pointer; a
/// release there adds them.
fn drop_target(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, a: &Album) {
    let Some(ids) = app.ui.dragging_photos.clone() else { return };
    let over = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| resp.rect.contains(p));
    if !over {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(resp.rect.shrink2(vec2(8.0, 1.0)), 4.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    if ui.input(|i| i.pointer.any_released()) {
        let n = ids.len();
        match app.run("album.addPhotos", json!({"id": a.id.0, "ids": ids})) {
            Ok(_) => app.toast(ui.ctx(), crate::i18n::tr_format!("Added {n} photo{} to “{}”", if n == 1 { "" } else { "s" }, a.name, n = n)),
            Err(e) => app.toast(ui.ctx(), e),
        }
        app.ui.dragging_photos = None;
    }
}

fn folder_menu(app: &mut LightcraftApp, resp: &egui::Response, a: &Album) {
    resp.context_menu(|ui| {
        if !a.folder && !a.is_smart() && ui.button(crate::i18n::tr("Add Selected Photos")).clicked() {
            let _ = app.run("album.addPhotos", json!({"id": a.id.0}));
        }
        if !a.folder && !a.is_smart() {
            let is_target = app.session.target_album == Some(a.id) || (app.session.target_album.is_none() && a.quick);
            if !is_target && ui.button(crate::i18n::tr("Set as Target Album (B adds to it)")).clicked() {
                let _ = app.run("album.setTarget", json!({"id": if a.quick { serde_json::Value::Null } else { json!(a.id.0) }}));
            }
            if is_target && !a.quick && ui.button(crate::i18n::tr("Stop Using as Target Album")).clicked() {
                let _ = app.run("album.setTarget", json!({"id": null}));
            }
        }
        if a.quick && ui.button(crate::i18n::tr("Clear Quick Collection")).clicked() {
            let _ = app.run("album.clearQuick", json!({}));
        }
        if a.is_smart() && ui.button(crate::i18n::tr("Edit Smart Album…")).clicked() {
            // older smart albums keep their filter fields; the editor works on the rule set
            let rules = a.smart.as_ref().and_then(|f| f.rule_set.clone()).unwrap_or_default();
            app.ui.dialog = Some(crate::state::Dialog::SmartRules { id: Some(a.id.0), name: a.name.clone(), rules });
        }
        if a.is_smart() && ui.button(crate::i18n::tr("Update Rules from Current Filter")).clicked() {
            let _ = app.run("album.setRules", json!({"id": a.id.0, "fromView": true}));
        }
        if !a.folder {
            // export: show the album, select its photos, then the dialog / a preset
            let show_all = |app: &mut LightcraftApp| {
                let _ = app.run("library.source", json!({"kind": "album", "id": a.id.0}));
                let _ = app.run("library.selectAll", json!({}));
            };
            let has_photos = app.session.catalog.album_count(a.id) > 0;
            if ui.add_enabled(has_photos, egui::Button::new(crate::i18n::tr("Export Album…"))).clicked() {
                show_all(app);
                let _ = app.run("dialog.export", json!({}));
            }
            ui.add_enabled_ui(has_photos, |ui| {
                ui.menu_button(crate::i18n::tr("Export Album with Preset"), |ui| {
                    for (p, _) in app.session.all_export_presets() {
                        if ui.button(&p.name).clicked() {
                            show_all(app);
                            if let Err(e) = app.run("app.export", json!({"preset": p.name, "background": true})) {
                                app.toast(ui.ctx(), e);
                            }
                        }
                    }
                });
            });
            ui.separator();
        }
        // move into another folder (not into itself or one of its own subfolders)
        let mut folders: Vec<(u64, String)> =
            app.session.catalog.albums().filter(|f| f.folder && !is_within(app, f.id, a.id)).map(|f| (f.id.0, f.name.clone())).collect();
        folders.sort_by_key(|(_, n)| n.to_lowercase());
        ui.menu_button(crate::i18n::tr("Move to"), |ui| {
            if ui.add_enabled(a.parent.is_some(), egui::Button::new(crate::i18n::tr("Top Level"))).clicked() {
                let _ = app.run("album.move", json!({"id": a.id.0, "parent": null}));
            }
            for (fid, name) in &folders {
                if ui.add_enabled(a.parent.map(|p| p.0) != Some(*fid), egui::Button::new(name)).clicked() {
                    let _ = app.run("album.move", json!({"id": a.id.0, "parent": fid}));
                }
            }
        });
        if ui.button(crate::i18n::tr("Rename…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::RenameAlbum { id: a.id.0, name: a.name.clone() });
        }
        if ui.button(crate::i18n::tr("Delete")).clicked() {
            let _ = app.run("album.delete", json!({"id": a.id.0}));
        }
    });
}

/// Whether `id` is `ancestor` or lies inside it.
fn is_within(app: &LightcraftApp, id: lightcraft_catalog::AlbumId, ancestor: lightcraft_catalog::AlbumId) -> bool {
    let mut cur = Some(id);
    let mut guard = 0;
    while let Some(c) = cur {
        if c == ancestor {
            return true;
        }
        cur = app.session.catalog.album(c).and_then(|x| x.parent);
        guard += 1;
        if guard > 64 {
            break;
        }
    }
    false
}

/// Catalog (Lightroom Classic's first section): All Photographs, Quick Collection, Previous
/// Import, Picks, Missing Photographs (when files are gone) and Recently Deleted.
#[allow(clippy::too_many_arguments)]
fn catalog_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, total: usize, picks: usize, previous: usize, deleted: usize, src: LibrarySource) {
    if row(app, ui, "all", Icon::Photos, "All Photographs", Some(total), src == LibrarySource::All, 0.0).clicked() {
        let _ = app.run("library.source", json!({"kind": "all"}));
    }
    // the Quick Collection: B adds to it ("+" while it is the target, as in Classic)
    let quick = app.session.catalog.quick_collection().and_then(|q| app.session.catalog.album(q)).cloned();
    let n = quick.as_ref().map_or(0, |q| app.session.catalog.album_count(q.id));
    let target = app.session.target_album.filter(|t| app.session.catalog.album(*t).is_some());
    let label = if target.is_none() { "Quick Collection +" } else { "Quick Collection" };
    let sel = quick.as_ref().is_some_and(|q| src == LibrarySource::Album(q.id));
    let resp = row(app, ui, "quick", Icon::Album, label, Some(n), sel, 0.0);
    if let Some(q) = &quick {
        drop_target(app, ui, &resp, q);
    }
    if resp.clicked() {
        match &quick {
            Some(q) => {
                let _ = app.run("library.source", json!({"kind": "album", "id": q.id.0}));
            }
            None => app.toast(ui.ctx(), crate::i18n::tr("Press B to add the selected photos to the Quick Collection")),
        }
    }
    resp.context_menu(|ui| {
        if target.is_some() && ui.button(crate::i18n::tr("Set as Target Collection")).clicked() {
            let _ = app.run("album.setTarget", json!({"id": null}));
            ui.close();
        }
        if ui.add_enabled(n > 0, egui::Button::new(crate::i18n::tr("Clear Quick Collection"))).clicked() {
            let _ = app.run("album.clearQuick", json!({}));
            ui.close();
        }
    });
    if row(app, ui, "previousImport", Icon::Clock, "Previous Import", Some(previous), src == LibrarySource::PreviousImport, 0.0).clicked() {
        let _ = app.run("library.source", json!({"kind": "previousImport"}));
    }
    if row(app, ui, "picks", Icon::FlagPick, "Picks", Some(picks), src == LibrarySource::Picks, 0.0).clicked() {
        let _ = app.run("library.source", json!({"kind": "picks"}));
    }
    // photos whose files can't be found (checked every few seconds, not every frame)
    let missing = missing_count(app, ui);
    if (missing > 0 || src == LibrarySource::Missing)
        && row(app, ui, "missing", Icon::Folder, "Missing Photographs", Some(missing), src == LibrarySource::Missing, 0.0).clicked()
    {
        let _ = app.run("library.source", json!({"kind": "missing"}));
    }
    if row(app, ui, "recentlyDeleted", Icon::Trash, "Recently Deleted", Some(deleted), src == LibrarySource::RecentlyDeleted, 0.0).clicked() {
        let _ = app.run("library.source", json!({"kind": "recentlyDeleted"}));
    }
}

/// A section header's "+" button, kept at the visible right edge when the sidebar scrolls
/// sideways; returns its response (the caller hangs a menu on it).
fn header_plus(ui: &mut egui::Ui, header: Rect, viewport: Rect, id: &str, tip: &str) -> egui::Response {
    let plus_right = (header.left() + viewport.max.x).min(header.right());
    let mut hdr = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(plus_right - 50.0, header.top()), pos2(plus_right, header.bottom())))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    icon_button(&mut hdr, id, Icon::Plus, vec2(26.0, 26.0), false, true, tip)
}

/// Collections: the library's albums as Lightroom Classic's collections — sets (album folders),
/// collections and smart collections, with counts. The Quick Collection is in Catalog.
fn collections_section(app: &mut LightcraftApp, ui: &mut egui::Ui, viewport: Rect) {
    ui.add_space(10.0);
    let (ar, open) = sidebar_section_header(app, ui, "collections", "Collections");
    let plus = header_plus(ui, ar, viewport, "albumNew", "Create Collection");
    egui::Popup::menu(&plus).show(|ui| {
        if ui.button(crate::i18n::tr("Create Collection…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::NewAlbum { name: String::new(), folder: false });
        }
        if ui.button(crate::i18n::tr("Create Smart Collection…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::SmartRules {
                id: None,
                name: String::new(),
                rules: lightcraft_catalog::RuleSet { rules: vec![crate::panels::rules_editor::new_rule()], ..Default::default() },
            });
        }
        if ui.button(crate::i18n::tr("Create Smart Collection from Filter…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::NewSmartAlbum { name: String::new() });
        }
        if ui.button(crate::i18n::tr("Create Collection Set…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::NewAlbum { name: String::new(), folder: true });
        }
    });
    if open {
        let albums: Vec<Album> = app.session.catalog.albums().filter(|a| !a.quick).cloned().collect();
        albums_tree(app, ui, &albums, None, 0.0);
    }
}

/// "Folders", as in Lightroom Classic: a row per disk (its name and how full it is; quiet when it
/// is not connected), under it the library's root folders (see
/// `lightcraft_catalog::folders::root_folders`) with photo counts — the folders inside included
/// while Library ▸ Show Photos in Subfolders is on. A click makes that folder the source, like
/// an album; the triangle opens a level. The tree comes from the catalog, never from browsing the
/// disk (that is Local).
fn folders_section(app: &mut LightcraftApp, ui: &mut egui::Ui, viewport: Rect) {
    let tree = app.caches.folder_tree(&app.session);
    ui.add_space(10.0);
    let (fr, open) = sidebar_section_header(app, ui, "folders", "Folders");
    let plus = header_plus(ui, fr, viewport, "folderNew", "Add Folder");
    egui::Popup::menu(&plus).show(|ui| {
        if app.services.pick_folder.is_some() && ui.button(crate::i18n::tr("Add Folder…")).clicked() {
            let picked = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(dir) = picked
                && let Err(e) = crate::import::open(app, vec![dir])
            {
                app.toast(ui.ctx(), e);
            }
            ui.close();
        }
        if app.services.pick_folder.is_some()
            && ui
                .button(crate::i18n::tr("Browse Folder Without Importing…"))
                .on_hover_text(crate::i18n::tr("Look at a folder's photos in place (Local); nothing is added to the library"))
                .clicked()
        {
            let picked = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(path) = picked {
                match app.run("library.browse", json!({"path": path})) {
                    Ok(r) => {
                        let dir = r["path"].as_str().unwrap_or(&path).to_string();
                        let _ = app.run("local.addRoot", json!({"path": dir}));
                    }
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
            ui.close();
        }
        ui.separator();
        subfolders_toggle(app, ui);
    });
    if open {
        reveal_chosen(app, ui, &tree);
        folder_rows(app, ui, &tree, 0.0, false);
    }
}

/// Whenever the shown folder changes (a click, an agent, a rename or its undo), open the rows
/// above it so it is on screen; folding one by hand afterwards sticks until the choice changes.
fn reveal_chosen(app: &LightcraftApp, ui: &egui::Ui, tree: &[FolderNode]) {
    let shown = app.session.library_folder.clone().filter(|_| app.session.source == LibrarySource::LibraryFolder);
    let chosen = shown.filter(|c| !lightcraft_catalog::query::folder_key(c).is_empty());
    let seen = egui::Id::new("libfolder-revealed");
    let now = chosen.as_deref().map(lightcraft_catalog::query::folder_key);
    if ui.data(|d| d.get_temp::<Option<String>>(seen)) == Some(now.clone()) {
        return;
    }
    ui.data_mut(|d| d.insert_temp(seen, now));
    let Some(chosen) = chosen else { return };
    fn open_above(ui: &egui::Ui, nodes: &[FolderNode], chosen: &str) {
        for n in nodes {
            if lightcraft_catalog::query::folder_within(chosen, &n.path) && !same_folder(chosen, &n.path) {
                let key = lightcraft_catalog::query::folder_key(&n.path);
                ui.data_mut(|d| d.insert_temp(egui::Id::new(("libfolder-open", key)), true));
                open_above(ui, &n.children, chosen);
            }
        }
    }
    open_above(ui, tree, &chosen);
}

fn is_dir(p: &str) -> bool {
    std::path::Path::new(p).is_dir()
}

/// The rows of `nodes` (disks at the top level). `offline`: their disk is not connected, so
/// nothing below is checked on disk.
fn folder_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, nodes: &[FolderNode], indent: f32, offline: bool) {
    let t = Tokens::get(ui.ctx());
    let subfolders = app.session.library_subfolders;
    for n in nodes {
        let key = lightcraft_catalog::query::folder_key(&n.path);
        let open_id = egui::Id::new(("libfolder-open", key.clone()));
        // a disk starts open
        let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(n.volume);
        // a row whose path would cover other disks' photos too only opens and closes
        let selectable = n.selectable;
        let sel = selectable
            && app.session.source == LibrarySource::LibraryFolder
            && app.session.library_folder.as_deref().is_some_and(|f| same_folder(f, &n.path));
        // (checked off the UI thread, every few seconds: a disk can be asleep or gone)
        let gone = offline || (!cfg!(target_arch = "wasm32") && fs_cached(ui, "is-dir", &n.path, 10.0, is_dir) == Some(false));
        let (icon, name, extra, count) = if n.volume {
            let name = if n.path == "/" {
                fs_cached(ui, "startup-name", "/", 600.0, |_| lightcraft_engine::disks::startup_name())
                    .flatten()
                    .unwrap_or_else(|| crate::i18n::tr("This Computer").to_string())
            } else {
                n.name.clone()
            };
            let badge = if gone {
                Some(crate::i18n::tr("not connected").to_string())
            } else {
                fs_cached(ui, "disk-space", &n.path, 30.0, lightcraft_engine::disks::space)
                    .flatten()
                    .map(|(tot, av)| lightcraft_engine::disks::space_label(tot, av))
            };
            (Icon::Disk, name, RowExtra { dim: gone, badge }, Some(n.count))
        } else {
            let name = if gone { format!("{} ?", n.name) } else { n.name.clone() };
            (Icon::Folder, name, RowExtra { dim: gone, badge: None }, Some(if subfolders { n.count } else { n.own }))
        };
        let resp = row_ex(app, ui, &format!("libfolder:{}", n.path), icon, &name, Some(&name), count, sel, indent, &extra);
        let mut toggled = false;
        if !n.children.is_empty() {
            // disclosure triangle left of the icon
            let c = pos2(resp.rect.left() + 10.0 + indent, resp.rect.center().y);
            let tri = Rect::from_center_size(c, vec2(14.0, 14.0));
            let tr = ui.interact(tri, egui::Id::new(("libfolder-tri", key)), Sense::click());
            register(ui.ctx(), format!("libraryFolderToggle:{}", n.path), tri);
            let col = if tr.hovered() { t.text } else { t.text_dim };
            let pts = if open {
                vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
            toggled = tr.clicked();
            tr.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, if open { crate::i18n::tr("Collapse") } else { crate::i18n::tr("Expand") })
            });
            // the triangle sits on the row and takes its clicks: the menu opens from it too
            row_menu(app, &tr, n, gone);
        }
        let photos = if subfolders { n.count } else { n.own };
        let mut tip = format!("{}\n{} photo{}", n.path, photos, if photos == 1 { "" } else { "s" });
        if gone {
            tip.push_str(if n.volume {
                "\nNot connected: connect the disk, or use Find Missing Folder… on a folder that moved"
            } else {
                "\nNot found: right-click ▸ Find Missing Folder… to point the library at where it went"
            });
        }
        let resp = resp.on_hover_text(tip);
        if resp.clicked() && !toggled {
            if selectable {
                // a source like an album or a Local folder: it replaces what the grid showed
                if let Err(e) = app.run("library.source", json!({"kind": "libraryFolder", "path": n.path})) {
                    app.toast(ui.ctx(), e);
                }
            } else {
                toggled = true;
            }
        }
        if toggled {
            open = !open;
            ui.data_mut(|d| d.insert_temp(open_id, open));
        }
        row_menu(app, &resp, n, gone);
        if open && !n.children.is_empty() {
            folder_rows(app, ui, &n.children, indent + 16.0, gone);
        }
    }
}

/// The context menu of a Folders row: a folder's own, a disk's (remove it), none for the
/// startup disk.
fn row_menu(app: &mut LightcraftApp, resp: &egui::Response, n: &FolderNode, gone: bool) {
    if !n.volume {
        folder_menu_for_library(app, resp, n, gone);
    } else if n.path != "/" {
        resp.context_menu(|ui| {
            subfolders_toggle(app, ui);
            ui.separator();
            if ui
                .button(crate::i18n::tr("Remove Disk from Library…"))
                .on_hover_text(crate::i18n::tr("Moves every photo imported from this disk to Recently Deleted; no file is touched"))
                .clicked()
            {
                app.ui.dialog = Some(crate::state::Dialog::RemoveFolder { path: n.path.clone(), name: n.name.clone(), count: n.count, disk: true });
                ui.close();
            }
        });
    } else {
        resp.context_menu(|ui| subfolders_toggle(app, ui));
    }
}

/// Library ▸ Show Photos in Subfolders, as a menu item that shows its state.
fn subfolders_toggle(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let mut on = app.session.library_subfolders;
    if ui.checkbox(&mut on, crate::i18n::tr("Show Photos in Subfolders")).clicked() {
        let _ = app.run("library.showSubfolders", json!({"on": on}));
        ui.close();
    }
}

/// Synchronize Folder: what is on disk and not in the library is imported in the background (as
/// a drop on the window is); photos whose file is gone are reported.
fn synchronize(app: &mut LightcraftApp, ctx: &egui::Context, path: &str) {
    match app.run("folder.sync", json!({"path": path, "dryRun": true})) {
        Ok(r) => {
            let files: Vec<String> =
                r["newFiles"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let missing = r["missing"].as_u64().unwrap_or(0);
            let gone = if missing > 0 { format!("; {missing} photo(s) have no file (see Missing Photographs)") } else { String::new() };
            if files.is_empty() {
                app.toast(ctx, format!("Synchronized: nothing new{gone}"));
            } else {
                let n = files.len();
                match crate::import::start_paths(app, files) {
                    Ok(_) => app.toast(ctx, format!("Synchronizing: importing {n} new photo(s){gone}")),
                    Err(e) => app.toast(ctx, e),
                }
            }
        }
        Err(e) => app.toast(ctx, e),
    }
}

/// The context menu of a folder row (Lightroom Classic's): subfolders, synchronize, find a moved
/// folder, create one inside, rename / move on disk (photos follow), add the parent folder, show
/// in Finder, and taking its photos out of the library.
fn folder_menu_for_library(app: &mut LightcraftApp, resp: &egui::Response, n: &FolderNode, gone: bool) {
    let path = n.path.as_str();
    let name = std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    // what dialogs call it: the last two names, so same-named folders are told apart
    let label = lightcraft_catalog::folders::folder_label(path);
    let added_parent = app.session.folder_parents.iter().any(|f| same_folder(f, path));
    resp.context_menu(|ui| {
        subfolders_toggle(app, ui);
        ui.separator();
        if ui.button(crate::i18n::tr("Synchronize Folder…")).clicked() {
            synchronize(app, ui.ctx(), path);
            ui.close();
        }
        if app.services.pick_folder.is_some()
            && ui
                .button(crate::i18n::tr("Find Missing Folder…"))
                .on_hover_text(crate::i18n::tr("The folder was moved or renamed: pick where it is now, and its photos are relinked"))
                .clicked()
        {
            let to = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(to) = to {
                match app.run("folder.locate", json!({"path": path, "to": to})) {
                    Ok(r) => app.toast(ui.ctx(), format!("Found: {} photo(s) relinked, {} still missing", r["relinked"], r["stillMissing"])),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
            ui.close();
        }
        if !gone && ui.button(crate::i18n::tr("Create Folder Inside…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: format!("New folder inside “{label}”"),
                hint: "Folder name (made on disk)".into(),
                value: String::new(),
                command: "folder.create".into(),
                params: json!({"path": path}),
                key: "name".into(),
            });
            ui.close();
        }
        if !gone && ui.button(crate::i18n::tr("Rename…")).clicked() {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: format!("Rename “{label}”"),
                hint: "Folder name (renamed on disk; its photos follow)".into(),
                value: name.clone(),
                command: "folder.rename".into(),
                params: json!({"path": path}),
                key: "name".into(),
            });
            ui.close();
        }
        if !gone && app.services.pick_folder.is_some() && ui.button(crate::i18n::tr("Move Folder To…")).clicked() {
            let into = app.services.pick_folder.as_mut().and_then(|f| f());
            if let Some(into) = into {
                match app.run("folder.move", json!({"path": path, "into": into})) {
                    Ok(r) => app.toast(ui.ctx(), format!("Moved; {} photo(s) relinked", r["relinked"])),
                    Err(e) => app.toast(ui.ctx(), e),
                }
            }
            ui.close();
        }
        let has_parent = lightcraft_catalog::folders::parent_folder(path).is_some_and(|p| !lightcraft_catalog::folders::is_disk_root(&p));
        if has_parent && ui.button(crate::i18n::tr("Add Parent Folder")).clicked() {
            if let Err(e) = app.run("folder.addParent", json!({"path": path})) {
                app.toast(ui.ctx(), e);
            }
            ui.close();
        }
        if added_parent && ui.button(crate::i18n::tr("Promote Subfolders")).clicked() {
            let _ = app.run("folder.promoteSubfolders", json!({"path": path}));
            ui.close();
        }
        if !gone
            && app.services.reveal.is_some()
            && ui.button(crate::i18n::tr("Show in Finder")).clicked()
            && let Some(f) = app.services.reveal.as_mut()
        {
            let _ = f(path);
            ui.close();
        }
        ui.separator();
        if ui
            .button(crate::i18n::tr("Remove from Library…"))
            .on_hover_text(crate::i18n::tr("Moves the photos imported from this folder to Recently Deleted; no file is touched"))
            .clicked()
        {
            app.ui.dialog = Some(crate::state::Dialog::RemoveFolder { path: path.to_string(), name: label.clone(), count: n.count, disk: false });
            ui.close();
        }
    });
}

/// "Keywords": the library's keyword tree with photo counts (`a|b|c` keywords nest). A click
/// filters the grid by the keyword (children included), the triangle opens a level, and the
/// context menu renames, merges or deletes the keyword across the library.
pub fn keywords_section(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let tree = app.caches.keyword_tree(&app.session.catalog);
    if tree.is_empty() {
        return;
    }
    ui.add_space(10.0);
    if sidebar_section_header(app, ui, "keywords", "Keywords").1 {
        keyword_rows(app, ui, &tree, 0.0);
    }
}

fn keyword_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, nodes: &[KeywordNode], indent: f32) {
    let t = Tokens::get(ui.ctx());
    for n in nodes {
        let open_id = egui::Id::new(("kw-open", n.path.to_lowercase()));
        let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
        let sel = app.session.filter.keyword.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(&n.path));
        let resp = row(app, ui, &format!("keyword:{}", n.path), Icon::Tag, &n.name, Some(n.count), sel, indent);
        if !n.children.is_empty() {
            // disclosure triangle left of the icon
            let c = pos2(resp.rect.left() + 10.0 + indent, resp.rect.center().y);
            let tri = Rect::from_center_size(c, vec2(14.0, 14.0));
            let tr = ui.interact(tri, egui::Id::new(("kw-tri", n.path.to_lowercase())), Sense::click());
            register(ui.ctx(), format!("keywordToggle:{}", n.path), tri);
            let col = if tr.hovered() { t.text } else { t.text_dim };
            let pts = if open {
                vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
            if tr.clicked() {
                open = !open;
                ui.data_mut(|d| d.insert_temp(open_id, open));
            }
        }
        let resp = resp.on_hover_text(if n.children.is_empty() { n.path.clone() } else { format!("{} (includes the keywords below it)", n.path) });
        if resp.clicked() {
            let v = if sel { serde_json::Value::Null } else { json!(n.path) };
            browse_all_photos(app, !sel);
            let _ = app.run("library.filter", json!({"keyword": v}));
        }
        resp.context_menu(|ui| {
            let has_sel = app.session.active().is_some();
            if ui.add_enabled(has_sel, egui::Button::new(crate::i18n::tr("Add to Selected Photos"))).clicked() {
                let _ = app.run("photo.setMeta", json!({"addKeywords": [n.path]}));
            }
            if ui.add_enabled(has_sel, egui::Button::new(crate::i18n::tr("Remove from Selected Photos"))).clicked() {
                let _ = app.run("photo.setMeta", json!({"removeKeywords": [n.path]}));
            }
            ui.separator();
            if ui.button(crate::i18n::tr("Rename Keyword…")).clicked() {
                app.ui.dialog = Some(crate::state::Dialog::RenameKeyword { from: n.path.clone(), to: n.path.clone() });
            }
            if ui.button(crate::i18n::tr("Merge into…")).clicked() {
                app.ui.dialog = Some(crate::state::Dialog::MergeKeywords { from: vec![n.path.clone()], into: String::new() });
            }
            if ui.button(crate::i18n::tr("Delete Keyword")).clicked() {
                let _ = app.run("keyword.delete", json!({"keyword": n.path}));
            }
        });
        if open && !n.children.is_empty() {
            keyword_rows(app, ui, &n.children, indent + 16.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::local_places;
    use crate::widgets::elide_head;

    /// Width = characters, so a limit of 12 is "12 characters".
    fn fit(text: &str, max: usize) -> String {
        elide_head(text, max as f32, |s| s.chars().count() as f32)
    }

    #[test]
    fn a_label_that_fits_is_left_alone() {
        assert_eq!(fit("/Users/me/Pictures", 18), "/Users/me/Pictures");
    }

    #[test]
    fn a_long_path_loses_its_leading_folders_not_its_end() {
        assert_eq!(fit("/Users/me/Pictures/Lightroom", 20), "…/Pictures/Lightroom");
        assert_eq!(fit("/Users/me/Pictures/Lightroom", 12), "…/Lightroom");
    }

    #[test]
    fn a_single_name_too_long_is_cut_at_the_end() {
        assert_eq!(fit("2024-summer-holiday", 8), "2024-su…");
        assert_eq!(fit("2024-summer-holiday", 1), "…");
        assert_eq!(fit("2024-summer-holiday", 0), "…");
    }

    #[test]
    fn multibyte_names_are_cut_on_character_boundaries() {
        assert_eq!(fit("/写真/夏休み旅行の記録", 6), "夏休み旅行…");
    }

    fn names(v: &[(String, String)]) -> Vec<&str> {
        v.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// A built-in location browsed (or picked) under another spelling of its path is the same
    /// row, not a second root — for every built-in, not just named ones.
    #[test]
    fn equivalent_paths_are_one_local_location() {
        for (home, sep) in [("D:\\Example", '\\'), ("D:/Example", '/'), ("/home/example", '/')] {
            let builtin: Vec<(String, String)> =
                ["Pictures", "Desktop", "Downloads"].iter().map(|s| (s.to_string(), format!("{home}{sep}{s}"))).collect();
            for (_, p) in builtin.clone() {
                // the browsed spelling: other separators, a trailing one, a different drive-letter case
                let flipped = p.replace(['/', '\\'], if sep == '/' { "\\" } else { "/" });
                for browsing in [flipped.clone(), format!("{p}{sep}"), p.replacen("D:", "d:", 1)] {
                    let l = local_places(builtin.clone(), std::slice::from_ref(&browsing), Some(&browsing), None, &[]);
                    assert_eq!(names(&l.places), ["Pictures", "Desktop", "Downloads"], "{p} browsed as {browsing}");
                    assert_eq!(l.places[l.owner.unwrap()].1, p, "the built-in row is the one highlighted");
                }
                // hiding under one spelling hides the other
                let l = local_places(builtin.clone(), &[], None, None, std::slice::from_ref(&flipped));
                assert_eq!(l.places.len(), 2, "hidden {flipped}");
            }
        }
        // a different folder is still added
        let l = local_places(vec![("Pictures".into(), "/home/example/Pictures".into())], &[], Some("/home/example/Pictures2"), None, &[]);
        assert_eq!(names(&l.places), ["Pictures", "Pictures2"]);
    }

    /// A folder inside a kept root is shown inside that root's tree: the root stays (its other
    /// subfolders stay reachable) and no second root appears; other kept roots stay too.
    #[test]
    fn browsing_below_a_kept_root_keeps_the_root() {
        let photos = "/data/Photos".to_string();
        let other = "/data/Scans".to_string();
        let saved = [photos.clone(), other.clone()];
        let builtin = || vec![("Home".to_string(), "/home/example".to_string())];
        for browsing in ["/data/Photos/2026/20260101", "/data/Photos/2026", "/data/Photos", "D:\\x"] {
            let l = local_places(builtin(), &saved, Some(browsing), None, &[]);
            let roots: Vec<&str> = l.places.iter().map(|(_, p)| p.as_str()).collect();
            if browsing.starts_with("/data") {
                assert_eq!(roots, ["/home/example", "/data/Photos", "/data/Scans"], "{browsing}");
                assert_eq!(l.owner, Some(1), "the Photos tree opens down to {browsing}");
                assert_eq!(l.browse_root, None);
            } else {
                assert_eq!(roots, ["/home/example", "/data/Photos", "/data/Scans", "D:\\x"], "a folder outside them gets a row");
            }
        }
        // the innermost containing root is the one that opens
        let l = local_places(builtin(), &["/home/example/Pictures".into()], Some("/home/example/Pictures/Trip"), None, &[]);
        assert_eq!(l.owner, Some(1));
    }

    /// A browsed folder outside every kept root gets a session row, which stays while browsing
    /// below it and gives way when browsing moves elsewhere.
    #[test]
    fn session_root_stays_while_browsing_below_it() {
        let l = local_places(Vec::new(), &[], Some("/t/base"), None, &[]);
        assert_eq!(l.browse_root.as_deref(), Some("/t/base"));
        let l = local_places(Vec::new(), &[], Some("/t/base/Trip/Day 1"), l.browse_root.as_deref(), &[]);
        assert_eq!((l.places.len(), l.browse_root.as_deref(), l.owner), (1, Some("/t/base"), Some(0)), "{l:?}");
        let l = local_places(Vec::new(), &[], Some("/elsewhere"), l.browse_root.as_deref(), &[]);
        assert_eq!(l.browse_root.as_deref(), Some("/elsewhere"));
        // hidden: no row at all
        let l = local_places(Vec::new(), &[], Some("/t/base"), None, &["/t/base/".into()]);
        assert!(l.places.is_empty() && l.browse_root.is_none());
    }

    /// A hidden folder inside a listed one (a kept folder beneath Home) is not revealed in that
    /// one's tree while it is browsed, nor are the folders below it; hiding Home itself still
    /// lets Pictures open down to a folder browsed inside it.
    #[test]
    fn hidden_folder_is_not_revealed_in_an_outer_tree() {
        let builtin = || vec![("Pictures".to_string(), "/home/example/Pictures".to_string()), ("Home".to_string(), "/home/example".to_string())];
        let kept = ["/home/example/AppData/Temp/lc".to_string()];
        let l = local_places(builtin(), &kept, Some("/home/example/AppData/Temp/lc"), None, &[]);
        assert_eq!(l.owner, Some(2), "shown as its own kept row");
        for browsing in ["/home/example/AppData/Temp/lc", "/home/example/AppData/Temp/lc/Day 1"] {
            let l = local_places(builtin(), &kept, Some(browsing), None, &["/home/example/AppData/Temp/lc/".into()]);
            assert_eq!(names(&l.places), ["Pictures", "Home"], "{browsing}");
            assert_eq!((l.owner, l.browse_root.as_deref()), (None, None), "Home does not open down to {browsing}");
        }
        let l = local_places(builtin(), &[], Some("/home/example/Pictures/Trip"), None, &["/home/example".into()]);
        assert_eq!((names(&l.places), l.owner), (vec!["Pictures"], Some(0)));
    }
}
