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
    /// A small dot before the count: the colour of the disk a shoot is on.
    dot: Option<egui::Color32>,
    /// The icon in the accent colour (a pinned shoot).
    accent: bool,
    /// Room kept free at the right edge for a control the caller puts there (a folder's add.).
    reserve: f32,
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
    if extra.dim {
        register(ui.ctx(), format!("dim:{id}"), inner);
    }
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
        } else if extra.accent {
            t.accent
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
    let edge = edge - extra.reserve;
    let count_left = count_galley.as_ref().map_or(edge - 18.0, |g| edge - 18.0 - g.size().x);
    // the disk dot sits 8 px before the count, the name ends 8 px before the dot
    const DOT_R: f32 = 3.5;
    let dot_center = pos2(count_left - 8.0 - DOT_R, r.center().y);
    let name_end = if extra.dot.is_some() { dot_center.x - DOT_R - 8.0 } else { count_left - 8.0 };
    if let Some(c) = extra.dot {
        let c = if extra.dim { c.gamma_multiply(0.45) } else { c };
        ui.painter().circle_filled(dot_center, DOT_R, c);
        register(ui.ctx(), format!("dot:{id}"), Rect::from_center_size(dot_center, vec2(DOT_R * 2.0, DOT_R * 2.0)));
    }
    // the name gives way to the count: cut with an ellipsis, in full on hover
    let room = name_end - label_left;
    let measure = |s: &str| ui.painter().layout_no_wrap(s.to_string(), font.clone(), color).size().x;
    let full_w = measure(label);
    let shown = if full_w <= room { label.to_string() } else { crate::widgets::elide_head(label, room.max(0.0), measure) };
    let label_rect = ui.painter().text(pos2(label_left, r.center().y), Align2::LEFT_CENTER, &shown, font.clone(), color);
    register(ui.ctx(), format!("label:{id}"), label_rect);
    let resp = if shown != label { resp.on_hover_text(label) } else { resp };
    // a row asks for room for its name up to a share of a panel, so one very long name does not
    // make everything scroll: depth does that
    let mut needed = 42.0 + indent + full_w.min(MAX_NAME_NEED) + 18.0 + extra.reserve + if extra.dot.is_some() { 15.0 } else { 0.0 };
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
    let movable = super::order::title(super::order::LIBRARY_LEFT, id).is_some();
    // (a reorderable section moves by its header: `panels::order`)
    let sense = if movable { Sense::click_and_drag() } else { Sense::click() };
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), sense);
    register(ui.ctx(), format!("sidebarSection:{id}"), r);
    if movable {
        super::order::header(ui, super::order::LIBRARY_LEFT, id, &resp, 8.0);
        resp.context_menu(|ui| super::order::menu_items(app, ui, super::order::LIBRARY_LEFT, id));
    }
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
        selection_actions(app, ui);
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
            // Catalog, shoots., Folders, Collections, Publish Services in the order the user set
            super::order::begin(ui, super::order::LIBRARY_LEFT);
            for id in super::order::ordered(&app.ui.settings, super::order::LIBRARY_LEFT) {
                match id {
                    "catalog" => {
                        let (_, catalog_open) = sidebar_section_header(app, ui, "catalog", "Catalog");
                        if catalog_open {
                            catalog_rows(app, ui, total, picks, previous, deleted, src);
                        }
                    }
                    "shoots" => shoots_section(app, ui, viewport),
                    "folders" => folders_section(app, ui, viewport),
                    "collections" => collections_section(app, ui, viewport),
                    "publish" => publish_section(app, ui, viewport),
                    _ => {}
                }
            }
            super::order::finish(app, ui, super::order::LIBRARY_LEFT);
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

/// With 2+ photos selected: what can be done with them, right under the Navigator — compare,
/// sync, the merges, match brightness, auto align, stack. (`ui-egui` tests: `selection_actions_…`.)
const SELECTION_ACTIONS: &[(&str, &str, &str)] = &[
    ("compare.", "view.compare", "Compare the active photo with the next selected one (C)"),
    ("survey.", "view.survey", "All the selected photos side by side (N)"),
    ("sync.", "dialog.syncSettings", "Copy the active photo's settings to the others — with Match exposure / Auto align (⌘⇧S)"),
    ("match.", "develop.matchBrightness", "Set each photo's Exposure so it comes out as bright as the active one (⌘⌥⇧M)"),
    ("align.", "develop.autoAlign", "Level every selected photo's horizon (⌘⌥⇧U)"),
    ("merge.", "dialog.merge", "Merge — pick HDR, panorama, HDR panorama, focus stack or best take (⌃⌥M)"),
    ("hdr.", "dialog.mergeHdr", "HDR merge — exposure brackets → one photo (⌃H)"),
    ("pano.", "dialog.mergePanorama", "Panorama — overlapping photos → one wide photo (⌃M)"),
    ("focus.", "dialog.mergeFocus", "Focus stack — focused near to far → sharp throughout (⌃⌥F)"),
    ("take.", "dialog.mergeTake", "Best take — pick each person's face from the photo you like (⌃⌥T)"),
    ("stack.", "stack.group", "Stack the selected photos (⌘G)"),
];

fn selection_actions(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let n = app.session.selection.ids.len();
    if n < 2 {
        return;
    }
    let title = format!("{n} {}", crate::i18n::tr("selected"));
    let (_, open) = sidebar_section_header(app, ui, "selection", &title);
    if !open {
        return;
    }
    ui.add_space(8.0);
    let mut run = None;
    // lined up with the section titles; every wrapped row indented the same
    egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 12, top: 0, bottom: 0 }).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
            for (label, cmd, tip) in SELECTION_ACTIONS {
                if crate::widgets::text_button(ui, &format!("sel-{cmd}"), label, false).on_hover_text(crate::i18n::tr(tip)).clicked() {
                    run = Some(*cmd);
                }
            }
        });
    });
    ui.add_space(12.0);
    if let Some(cmd) = run
        && let Err(e) = app.run(cmd, serde_json::json!({}))
    {
        app.toast(ui.ctx(), e);
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
    // L26: the loupe's zoom levels (1:16 … 16:1) from here too
    let (zr, _) = ui.allocate_exact_size(vec2(w, 20.0), Sense::hover());
    super::develop_left::nav_more_zooms(app, ui, Rect::from_min_max(pos2(zr.right() - 90.0, zr.top()), pos2(zr.right() - 12.0, zr.bottom())));
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

fn albums_tree_view(app: &mut LightcraftApp, ui: &mut egui::Ui, all: &[Album], parent: Option<AlbumId>, indent: f32, view: &AlbumView) {
    if indent > 32.0 * 16.0 {
        return;
    }
    let mut kids: Vec<&Album> = all.iter().filter(|a| a.parent == parent).collect();
    // favourites first; then sets, then (by kind) smart collections before collections; then names
    kids.sort_by_key(|a| (!a.favorite, !a.folder, view.by_kind && !a.is_smart(), a.name.to_lowercase()));
    for a in kids {
        let shown = if a.favorite { format!("★ {}", a.name) } else { a.name.clone() };
        let dot = a.label.map(crate::theme::label_color);
        if a.folder {
            let open_id = egui::Id::new(("folder-open", a.id.0));
            let open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(true);
            let extra = RowExtra { dot, ..Default::default() };
            let resp = row_ex(app, ui, &format!("folder:{}", a.id.0), Icon::CollectionSet, &shown, Some(&a.name), None, false, indent, &extra);
            // a set opens and closes from its triangle (as Folders' rows do) or anywhere on it
            let tri = disclosure(ui, &resp, indent, open, egui::Id::new(("album-set-tri", a.id.0)), &format!("albumSetToggle:{}", a.id.0));
            if resp.clicked() || tri.clicked() {
                ui.data_mut(|d| d.insert_temp(open_id, !open));
            }
            folder_menu(app, &tri, a);
            folder_menu(app, &resp, a);
            if open || !view.needle.is_empty() {
                albums_tree_view(app, ui, all, Some(a.id), indent + 16.0, view);
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
            let label = if target == Some(a.id) { format!("{shown} +") } else { shown.clone() };
            let extra = RowExtra { dot, ..Default::default() };
            // narrowed to some of its folders or cameras: those rows are the chosen ones
            let sel = sel && app.session.active_album_part().is_none();
            let mut resp = row_ex(app, ui, &format!("album:{}", a.id.0), icon, &label, Some(&a.name), Some(n), sel, indent, &extra);
            if !a.is_smart() {
                drop_target(app, ui, &resp, a);
            }
            // LRC-LIB-COLL-PARTS: the disclosure triangle opens the folders (or cameras) its photos come from
            let owner = Owner::Album(a);
            let parts_open = app.ui.parts_open.contains(&owner.open_key());
            let mut toggled = false;
            if n > 0 {
                let tri = parts_triangle(ui, &resp, &owner, indent, parts_open);
                toggled = tri.clicked();
                if !a.is_smart() {
                    drop_target(app, ui, &tri, a);
                }
                folder_menu(app, &tri, a);
            }
            if let Some(rules) = &a.smart {
                resp = resp.on_hover_text(crate::i18n::tr_format!("Smart album: {}", rules.describe()));
            }
            if resp.clicked() && !toggled {
                let _ = app.run("library.source", json!({"kind": "album", "id": a.id.0}));
            }
            folder_menu(app, &resp, a);
            if toggled {
                set_parts_open(app, &owner.open_key(), !parts_open);
            }
            // merges.: the merges found among its photos
            let found = crate::merges::counts(app, ui.ctx()).album(a.id);
            if found > 0 {
                merges_row(app, ui, lightcraft_engine::merge_find::Scope::Album { id: a.id }, found, indent + 16.0);
            }
            if parts_open && n > 0 {
                part_rows(app, ui, owner, indent + 16.0);
            }
        }
    }
}

/// A disclosure triangle left of a row's icon (as on Folders rows): returns its response, which
/// takes the clicks on it.
fn disclosure(ui: &mut egui::Ui, row: &egui::Response, indent: f32, open: bool, id: egui::Id, widget: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let c = pos2(row.rect.left() + 10.0 + indent, row.rect.center().y);
    let tri = Rect::from_center_size(c, vec2(14.0, 14.0));
    let tr = ui.interact(tri, id, Sense::click());
    register(ui.ctx(), widget, tri);
    let col = if tr.hovered() { t.text } else { t.text_dim };
    let pts = if open {
        vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
    } else {
        vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(pts, col, egui::Stroke::NONE));
    tr.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, if open { crate::i18n::tr("Collapse") } else { crate::i18n::tr("Expand") })
    });
    tr
}

/// Whose folders / cameras a list of rows is: a collection's or a shoot's.
#[derive(Clone, Copy)]
enum Owner<'a> {
    Album(&'a Album),
    Shoot(&'a lightcraft_catalog::Shoot),
}

impl Owner<'_> {
    fn of(&self) -> lightcraft_engine::album_parts::PartOf {
        match self {
            Owner::Album(a) => lightcraft_engine::album_parts::PartOf::Album { id: a.id },
            Owner::Shoot(s) => lightcraft_engine::album_parts::PartOf::Shoot { path: s.path.clone(), deep: s.deep },
        }
    }
    /// Widget ids start with it (`albumPart:…`, `shootPart:…`).
    fn prefix(&self) -> &'static str {
        match self {
            Owner::Album(_) => "album",
            Owner::Shoot(_) => "shoot",
        }
    }
    /// What names it in widget ids: the album id, the shoot's path.
    fn tag(&self) -> String {
        match self {
            Owner::Album(a) => a.id.0.to_string(),
            Owner::Shoot(s) => s.path.clone(),
        }
    }
    /// Its key in [`crate::state::UiState::parts_open`].
    fn open_key(&self) -> String {
        open_key(self)
    }
    fn name(&self) -> &str {
        match self {
            Owner::Album(a) => &a.name,
            Owner::Shoot(s) => &s.name,
        }
    }
    /// A collection photos can be added to (dropped on its rows, add.).
    fn album(&self) -> Option<&Album> {
        match self {
            Owner::Album(a) if !a.is_smart() => Some(a),
            _ => None,
        }
    }
}

fn open_key(o: &Owner) -> String {
    match o {
        Owner::Album(a) => format!("album:{}", a.id.0),
        Owner::Shoot(s) => format!("shoot:{}", lightcraft_catalog::query::folder_key(&s.path)),
    }
}

fn set_parts_open(app: &mut LightcraftApp, key: &str, open: bool) {
    if open {
        app.ui.parts_open.insert(key.to_string());
    } else {
        app.ui.parts_open.remove(key);
    }
}

/// What the rows under a shoot or collection are split by (the `fldr.` / `cam.` toggle).
fn parts_by(app: &LightcraftApp) -> lightcraft_engine::album_parts::PartBy {
    if app.ui.collection_parts_by_camera { lightcraft_engine::album_parts::PartBy::Camera } else { lightcraft_engine::album_parts::PartBy::Folder }
}

/// `fldr.` / `cam.` in a section header, ending at `right`: what the rows under each triangle are.
fn parts_by_toggle(app: &mut LightcraftApp, ui: &mut egui::Ui, header: Rect, right: f32, id: &str) {
    let by_cam = app.ui.collection_parts_by_camera;
    let mut hdr = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(right - 80.0, header.top()), pos2(right, header.bottom())))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if crate::widgets::text_button(&mut hdr, id, if by_cam { "cam." } else { "fldr." }, false)
        .on_hover_text(crate::i18n::tr(if by_cam {
            "Under each shoot and collection (its triangle): the cameras that took its photos. Click for its folders"
        } else {
            "Under each shoot and collection (its triangle): the folders its photos are in. Click for its cameras"
        }))
        .clicked()
    {
        app.ui.collection_parts_by_camera = !by_cam;
        // a view narrowed the other way shows whole again, not by rows nobody sees
        if let Some(p) = app.session.active_album_part().cloned() {
            let _ = match &p.of {
                lightcraft_engine::album_parts::PartOf::Album { id } => app.run("library.source", json!({"kind": "album", "id": id.0})),
                lightcraft_engine::album_parts::PartOf::Shoot { path, .. } => app.run("shoot.show", json!({"path": path})),
            };
        }
    }
}

/// The rows under a shoot or collection ([`lightcraft_engine::album_parts::parts`]), worked out
/// once per catalog revision: opening, clicking and drawing them reads the cached list.
pub(crate) fn album_parts(
    app: &LightcraftApp,
    ctx: &egui::Context,
    of: &lightcraft_engine::album_parts::PartOf,
    by: lightcraft_engine::album_parts::PartBy,
) -> std::sync::Arc<Vec<lightcraft_engine::album_parts::PartRow>> {
    type Cached = (u64, std::sync::Arc<Vec<lightcraft_engine::album_parts::PartRow>>);
    let key = egui::Id::new(("album-parts", of, by));
    let rev = app.session.catalog.revision;
    if let Some((r, v)) = ctx.data(|d| d.get_temp::<Cached>(key))
        && r == rev
    {
        return v;
    }
    let v = std::sync::Arc::new(lightcraft_engine::album_parts::parts(&app.session.catalog, of, by));
    ctx.data_mut(|d| {
        d.insert_temp(key, (rev, v.clone()));
        let n = d.get_temp_mut_or_insert_with::<u64>(egui::Id::new("album-parts-scans"), || 0);
        *n += 1;
    });
    v
}

/// How many times [`album_parts`] has gone through a collection's photos (tests: a click reads the cache).
#[cfg(test)]
pub(crate) fn album_parts_scans(ctx: &egui::Context) -> u64 {
    ctx.data(|d| d.get_temp::<u64>(egui::Id::new("album-parts-scans"))).unwrap_or(0)
}

/// The triangle on a shoot or collection row (one with photos): returns whether it was clicked.
/// The row's ← / → work while the pointer is on it.
fn parts_triangle(ui: &mut egui::Ui, resp: &egui::Response, owner: &Owner, indent: f32, open: bool) -> egui::Response {
    let tri =
        disclosure(ui, resp, indent, open, egui::Id::new(("parts-tri", owner.open_key())), &format!("{}PartsToggle:{}", owner.prefix(), owner.tag()));
    if resp.hovered() || tri.hovered() {
        hover_parts(ui, &owner.open_key());
    }
    tri
}

/// The folders (or cameras) of an open shoot or collection, one row each with its photo count.
/// Click = just this one, ⌘-click = add / remove it, ⇧-click = the run from the last one clicked;
/// all. / none. under them. Photos dropped on a collection's row go into the collection. Folders
/// beside them on disk with photos not in the library follow ([`beside_rows`]).
fn part_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, owner: Owner, indent: f32) {
    use lightcraft_engine::album_parts::PartBy;
    let by = parts_by(app);
    let of = owner.of();
    let rows = album_parts(app, ui.ctx(), &of, by);
    let picked: Vec<String> = app.session.active_album_part().filter(|p| p.of == of && p.by == by).map(|p| p.keys.clone()).unwrap_or_default();
    let icon = if by == PartBy::Camera { Icon::Camera } else { Icon::Folder };
    let (prefix, tag) = (owner.prefix(), owner.tag());
    for (i, r) in rows.iter().enumerate() {
        let sel = picked.contains(&r.key);
        let id = format!("{prefix}Part:{tag}:{}", r.key);
        // the camera after the folder only when both fit (the name is what tells the rows apart)
        let room = ui.available_width() - 42.0 - indent - 18.0 - 40.0;
        let font = Tokens::get(ui.ctx()).font(13.5);
        let fits = ui.painter().layout_no_wrap(r.label.clone(), font, egui::Color32::WHITE).size().x <= room;
        let label = if fits { &r.label } else { &r.name };
        let resp = row_named(app, ui, &id, icon, label, Some(&r.label), Some(r.count), sel, indent);
        if let Some(a) = owner.album() {
            drop_target(app, ui, &resp, a);
        }
        if resp.hovered() {
            hover_parts(ui, &owner.open_key());
        }
        let mut tip = if by == PartBy::Folder { r.key.clone() } else { r.name.clone() };
        if by == PartBy::Folder && r.cameras.len() > 1 {
            tip.push('\n');
            tip.push_str(&r.cameras.join(", "));
        }
        tip.push_str(&format!("\n{} photo{}", r.count, if r.count == 1 { "" } else { "s" }));
        let resp = resp.on_hover_text(tip);
        if resp.clicked() {
            let mods = ui.input(|i| i.modifiers);
            let keys = pick_parts(ui, &owner.open_key(), &rows, &picked, i, mods);
            show_parts(app, ui.ctx(), &of, by, &keys);
        }
        resp.context_menu(|ui| {
            if ui.button(crate::i18n::tr("Show Only This")).clicked() {
                show_parts(app, ui.ctx(), &of, by, std::slice::from_ref(&r.key));
                ui.close();
            }
            if ui.button(crate::i18n::tr_format!("Show All of “{}”", owner.name())).clicked() {
                show_parts(app, ui.ctx(), &of, by, &[]);
                ui.close();
            }
        });
    }
    if by == PartBy::Folder && !matches!(owner, Owner::Album(a) if a.is_smart()) {
        beside_rows(app, ui, owner, indent);
    }
    if rows.len() > 1 {
        // all. / none.: every row picked, or none (both show all of it)
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(Rect::from_min_max(pos2(r.left() + 42.0 + indent, r.top() + 3.0), pos2(r.right() - 8.0, r.bottom() - 3.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        bar.spacing_mut().item_spacing.x = 8.0;
        let all = picked.len() == rows.len();
        if crate::widgets::text_button(&mut bar, &format!("{prefix}PartsAll-{tag}"), "all.", all)
            .on_hover_text(crate::i18n::tr("Pick every row"))
            .clicked()
        {
            let keys: Vec<String> = rows.iter().map(|r| r.key.clone()).collect();
            show_parts(app, ui.ctx(), &of, by, &keys);
        }
        if crate::widgets::text_button(&mut bar, &format!("{prefix}PartsNone-{tag}"), "none.", false)
            .on_hover_text(crate::i18n::tr("Pick no row: all of it"))
            .clicked()
        {
            show_parts(app, ui.ctx(), &of, by, &[]);
        }
    }
}

/// Folders beside a shoot's or collection's folders on disk (`<shoot>/raw/GR3` next to
/// `raw/M262`) whose photos never came into the library: a full-strength row with a quiet count
/// and add., which imports them in place (into the collection, for a collection's). The disk is
/// read on a worker thread at most once a minute per folder ([`fs_cached`]); the count against the
/// library is worked out once per catalog revision.
fn beside_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, owner: Owner, indent: f32) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    use lightcraft_engine::album_parts as ap;
    let of = owner.of();
    let rows = album_parts(app, ui.ctx(), &of, ap::PartBy::Folder);
    let shoots: Vec<lightcraft_catalog::Shoot> = match owner {
        Owner::Album(_) => app.caches.shoots(&app.session).iter().map(|r| r.shoot.clone()).collect(),
        Owner::Shoot(_) => Vec::new(),
    };
    let roots = ap::roots_of(&of, &rows, &shoots);
    let parents = ap::parents_of(&rows, &roots);
    let scans: Vec<std::sync::Arc<Vec<ap::Scanned>>> =
        parents.iter().take(4).filter_map(|p| fs_cached(ui, "album-beside", p, 60.0, ap::scan_beside)).collect();
    if scans.is_empty() {
        return;
    }
    type Cached = (u64, Vec<usize>, std::sync::Arc<Vec<ap::BesideRow>>);
    let key = egui::Id::new(("album-beside-rows", owner.open_key()));
    let (rev, ptrs) = (app.session.catalog.revision, scans.iter().map(|s| std::sync::Arc::as_ptr(s) as usize).collect::<Vec<_>>());
    let beside = match ui.data(|d| d.get_temp::<Cached>(key)) {
        Some((r, p, v)) if r == rev && p == ptrs => v,
        _ => {
            let all: Vec<ap::Scanned> = scans.iter().flat_map(|s| s.as_ref().clone()).collect();
            let v = std::sync::Arc::new(ap::beside(&app.session.catalog, &of, &all));
            ui.data_mut(|d| d.insert_temp(key, (rev, ptrs, v.clone())));
            v
        }
    };
    let importing: Option<String> = ui.data(|d| d.get_temp(egui::Id::new("album-beside-importing")));
    let importing = importing.filter(|_| app.import.is_some());
    let (prefix, tag) = (owner.prefix(), owner.tag());
    for b in beside.iter() {
        let id = format!("{prefix}Beside:{tag}:{}", b.path);
        let full = format!("{} · {}", b.name, crate::i18n::tr("not in library"));
        // the folder's name stays whole: "· new" when "· not in library" doesn't fit beside it
        let room = ui.available_width() - 42.0 - indent - 18.0 - 46.0 - 30.0;
        let font = Tokens::get(ui.ctx()).font(13.5);
        let fits = ui.painter().layout_no_wrap(full.clone(), font, egui::Color32::WHITE).size().x <= room;
        let label = if fits { full } else { format!("{} · {}", b.name, crate::i18n::tr("new")) };
        let extra = RowExtra { reserve: 46.0, ..Default::default() };
        let resp = row_ex(app, ui, &id, Icon::Folder, &label, Some(&label), Some(b.count), false, indent, &extra);
        let into = match owner.album() {
            Some(a) => format!(" into “{}”", a.name),
            None => String::new(),
        };
        let resp = resp.on_hover_text(format!(
            "{}\n{} photo{} here are not in the library (a raw + JPEG pair counts once, sidecars never): add. imports them in place{into}",
            b.path,
            b.count,
            if b.count == 1 { "" } else { "s" },
        ));
        if resp.hovered() {
            hover_parts(ui, &owner.open_key());
        }
        // add. at the right edge (… while it imports)
        let visible_right: f32 = ui.data(|d| d.get_temp(egui::Id::new("left-visible-right"))).unwrap_or(f32::MAX);
        let right = resp.rect.right().min(visible_right) - 12.0;
        let busy = importing.as_deref() == Some(b.path.as_str());
        let slot = Rect::from_min_max(pos2(right - 60.0, resp.rect.top() + 2.0), pos2(right, resp.rect.bottom() - 2.0));
        let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(slot).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let add = crate::widgets::text_button(&mut bar, &format!("{prefix}BesideAdd-{tag}-{}", b.path), if busy { "…" } else { "add." }, false)
            .on_hover_text(crate::i18n::tr("Import these photos in place: nothing is copied or moved"));
        if add.clicked() && !busy {
            let started = match owner.album() {
                Some(a) => crate::import::start_paths_into(app, vec![b.path.clone()], a.id.0),
                None => crate::import::start_paths(app, vec![b.path.clone()]),
            };
            match started {
                Ok(_) => {
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("album-beside-importing"), b.path.clone()));
                    app.toast(ui.ctx(), format!("Importing {} photos from {}…", b.count, b.name));
                }
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
    }
}

/// The rows picked after a click on row `i`: just it; with ⌘ it added or taken off; with ⇧ the run
/// from the row clicked last (added to the others with ⌘ too).
fn pick_parts(
    ui: &egui::Ui,
    owner: &str,
    rows: &[lightcraft_engine::album_parts::PartRow],
    picked: &[String],
    i: usize,
    mods: egui::Modifiers,
) -> Vec<String> {
    let anchor_id = egui::Id::new(("album-part-anchor", owner));
    let Some(key) = rows.get(i).map(|r| r.key.clone()) else { return picked.to_vec() };
    let anchor: Option<String> = ui.data(|d| d.get_temp(anchor_id));
    let from = anchor.and_then(|k| rows.iter().position(|r| r.key == k));
    let keys = match from {
        Some(f) if mods.shift => {
            let (lo, hi) = if f <= i { (f, i) } else { (i, f) };
            let mut run: Vec<String> = rows.get(lo..=hi).unwrap_or(&[]).iter().map(|r| r.key.clone()).collect();
            if mods.command {
                run.extend(picked.iter().cloned());
            }
            run
        }
        _ if mods.command => {
            let mut v = picked.to_vec();
            if let Some(at) = v.iter().position(|k| *k == key) {
                v.remove(at);
            } else {
                v.push(key.clone());
            }
            v
        }
        _ => vec![key.clone()],
    };
    if !mods.shift {
        ui.data_mut(|d| d.insert_temp(anchor_id, key));
    }
    keys
}

/// Show a shoot or collection narrowed to rows `keys` (none: all of it).
fn show_parts(
    app: &mut LightcraftApp,
    ctx: &egui::Context,
    of: &lightcraft_engine::album_parts::PartOf,
    by: lightcraft_engine::album_parts::PartBy,
    keys: &[String],
) {
    let (id, mut p) = match of {
        lightcraft_engine::album_parts::PartOf::Album { id } => ("library.source", json!({"kind": "album", "id": id.0})),
        lightcraft_engine::album_parts::PartOf::Shoot { path, .. } => ("shoot.show", json!({"path": path})),
    };
    if !keys.is_empty() {
        p[if by == lightcraft_engine::album_parts::PartBy::Camera { "cameras" } else { "folders" }] = json!(keys);
    }
    if let Err(e) = app.run(id, p) {
        app.toast(ctx, e);
    }
}

/// The shoot or collection row (or one of its rows) under the pointer this frame: ← / → close and
/// open it.
fn hover_parts(ui: &egui::Ui, key: &str) {
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| d.insert_temp(egui::Id::new("left-hovered-parts"), (pass, key.to_string())));
}

/// ← closes and → opens the shoot or collection under the pointer (as Folders' triangles do);
/// `true` when the key was taken (it then moves no photo). Called before the shortcuts.
pub(crate) fn collection_keys(app: &mut LightcraftApp, ctx: &egui::Context) -> bool {
    let Some((pass, key)) = ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new("left-hovered-parts"))) else { return false };
    // hovered in the last frame drawn
    if ctx.cumulative_pass_nr().saturating_sub(pass) > 2 || !app.ui.left_panel {
        return false;
    }
    let open = app.ui.parts_open.contains(&key);
    let k = if open { egui::Key::ArrowLeft } else { egui::Key::ArrowRight };
    if !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k)) {
        return false;
    }
    set_parts_open(app, &key, !open);
    true
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
        if !a.quick {
            marks_menu(app, ui, "album", json!({"id": a.id.0}), a.label, a.favorite);
            ui.separator();
        }
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
        if a.quick && ui.button(crate::i18n::tr("Save Quick Collection…")).clicked() {
            crate::panels::dialogs::prompt(app, "Save Quick Collection", "Album name", "", "album.saveQuick", json!({}), "name");
        }
        if !a.folder && !a.quick && ui.button(crate::i18n::tr("Duplicate Album")).clicked() {
            let _ = app.run("album.duplicate", json!({"id": a.id.0}));
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

/// The Collections section in another module's left panel (the Map's), inside its scroll area.
pub(crate) fn map_collections(app: &mut LightcraftApp, ui: &mut egui::Ui, viewport: Rect) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new("left-visible-right"), ui.cursor().left() + viewport.max.x));
    collections_section(app, ui, viewport);
}

/// Develop's Collections panel (under its own header): the Library's rows — sets, collections and
/// smart collections with their counts, each collection's disclosure triangle opening the folders
/// (or cameras, as the Library's fldr. / cam. says) its photos come from (LRC-LIB-COLL-PARTS).
pub(crate) fn develop_collections(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new("left-visible-right"), ui.max_rect().right()));
    let albums: Vec<Album> = app.session.catalog.albums().filter(|a| !a.quick).cloned().collect();
    if albums.is_empty() {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let t = Tokens::get(ui.ctx());
        ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr("No collections yet"), t.font(12.0), t.text_dim);
        return;
    }
    ui.add_space(4.0);
    albums_tree_view(app, ui, &albums, None, 0.0, &AlbumView { needle: String::new(), by_kind: false, fav_only: false });
    ui.add_space(4.0);
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
    // fldr. / cam.: what the rows under each collection's triangle are, 8 px before the +
    parts_by_toggle(app, ui, ar, plus.rect.left() - 8.0, "collectionsPartsBy");
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
        // L24: filter the collections by name, sort by name or kind, favourites only
        let fid = egui::Id::new("collections-filter");
        let (mut needle, mut by_kind, mut fav_only): (String, bool, bool) = ui.data(|d| d.get_temp(fid)).unwrap_or_default();
        egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 12, top: 2, bottom: 6 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 120.0).max(60.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut needle)
                        .hint_text(crate::i18n::tr("Filter Collections"))
                        .desired_width(w)
                        .min_size(egui::vec2(w, 22.0))
                        .vertical_align(egui::Align::Center),
                );
                register(ui.ctx(), "field:collectionsFilter", r.rect);
                if crate::widgets::text_button(ui, "collectionsSort", if by_kind { "kind." } else { "name." }, false)
                    .on_hover_text(crate::i18n::tr("Sort by name or by kind (sets, smart collections, collections)"))
                    .clicked()
                {
                    by_kind = !by_kind;
                }
                if crate::widgets::text_button(ui, "collectionsFavorites", "fav.", fav_only)
                    .on_hover_text(crate::i18n::tr("Favourites only"))
                    .clicked()
                {
                    fav_only = !fav_only;
                }
            });
        });
        ui.data_mut(|d| d.insert_temp(fid, (needle.clone(), by_kind, fav_only)));
        let view = AlbumView { needle: needle.trim().to_lowercase(), by_kind, fav_only };
        let shown: Vec<Album> = albums.iter().filter(|a| album_visible(&albums, a, &view, 0)).cloned().collect();
        if shown.is_empty() && (!view.needle.is_empty() || fav_only) {
            ui.label(egui::RichText::new(crate::i18n::tr("No collections match")).size(11.0).color(Tokens::get(ui.ctx()).text_dim));
        }
        albums_tree_view(app, ui, &shown, None, 0.0, &view);
    }
}

/// Per published collection: (to publish = New + Modified + Deleted, photos), computed once per
/// catalog revision (a state compares each photo's settings and metadata with what was published).
fn publish_counts(app: &LightcraftApp, ui: &egui::Ui) -> std::sync::Arc<std::collections::HashMap<u64, (usize, usize)>> {
    type Counts = (u64, std::sync::Arc<std::collections::HashMap<u64, (usize, usize)>>);
    let key = egui::Id::new("publish-counts");
    let rev = app.session.catalog.revision;
    if let Some((r, m)) = ui.data(|d| d.get_temp::<Counts>(key))
        && r == rev
    {
        return m;
    }
    let cat = &app.session.catalog;
    let m: std::collections::HashMap<u64, (usize, usize)> = lightcraft_engine::publish::collections(cat)
        .iter()
        .map(|c| {
            let n = lightcraft_engine::publish::counts(cat, c);
            let g = |k: &str| n[k].as_u64().unwrap_or(0) as usize;
            (c.id, (g("new") + g("modified") + g("deleted"), g("total")))
        })
        .collect();
    let m = std::sync::Arc::new(m);
    ui.data_mut(|d| d.insert_temp(key, (rev, m.clone())));
    m
}

/// LRC-LIB-PUBLISH: Publish Services — each service with its published collections and sets.
/// A collection's count reads `+3 · 12` when 3 of its 12 photos wait to be published (new,
/// modified or to remove); photos dragged onto it join it; `pub.` publishes the one on screen.
fn publish_section(app: &mut LightcraftApp, ui: &mut egui::Ui, viewport: Rect) {
    use lightcraft_engine::publish::{self, ServiceKind};
    ui.add_space(10.0);
    let (hr, open) = sidebar_section_header(app, ui, "publish", "Publish Services");
    let plus = header_plus(ui, hr, viewport, "publishNew", "Set Up a Publish Service");
    egui::Popup::menu(&plus).show(|ui| {
        let mut r = None;
        if menu_item(ui, "publishNewHardDrive", crate::i18n::tr("Hard Drive…"), "Publish to a folder: each collection becomes a folder in it")
            .clicked()
        {
            r = Some(crate::publish_ui::create_service(app, "hardDrive"));
        }
        if lightcraft_engine::apple_photos::available()
            && menu_item(ui, "publishNewPhotos", crate::i18n::tr("Apple Photos"), "Publish into albums in Photos").clicked()
        {
            r = Some(crate::publish_ui::create_service(app, "photos"));
        }
        if let Some(Err(e)) = r {
            app.toast(ui.ctx(), e);
        }
    });
    if !open {
        return;
    }
    let services = publish::services(&app.session.catalog);
    let t = Tokens::get(ui.ctx());
    if services.is_empty() {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 12, top: 2, bottom: 6 }).show(ui, |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Set one up with +: Hard Drive publishes to a folder")).size(11.0).color(t.text_dim));
        });
        return;
    }
    let cols = publish::collections(&app.session.catalog);
    let counts = publish_counts(app, ui);
    for svc in &services {
        let icon = match svc.kind {
            ServiceKind::HardDrive => Icon::Disk,
            ServiceKind::Photos => Icon::Photos,
        };
        let open_id = egui::Id::new(("publish-service-open", svc.id));
        let is_open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(true);
        let resp = row_ex(app, ui, &format!("publishService:{}", svc.id), icon, &svc.name, None, None, false, 0.0, &RowExtra::default());
        let resp = resp.on_hover_text(match svc.kind {
            ServiceKind::HardDrive => format!("Hard Drive: {}", svc.dir),
            ServiceKind::Photos => crate::i18n::tr("Apple Photos: an album per collection").to_string(),
        });
        if resp.clicked() {
            ui.data_mut(|d| d.insert_temp(open_id, !is_open));
        }
        service_menu(app, &resp, svc);
        if is_open {
            publish_tree(app, ui, &cols, &counts, svc.id, None, 16.0);
        }
    }
    // the collection on screen: publish it, or count it as published as it is
    if let LibrarySource::Published(cid) = app.session.source
        && cols.iter().any(|c| c.id == cid)
    {
        egui::Frame::NONE.inner_margin(egui::Margin { left: 18, right: 12, top: 12, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let busy = app.tasks.is_running(crate::publish_ui::PUBLISHING);
                let label = if busy { "pub. …" } else { "pub." };
                if crate::widgets::text_button(ui, "publishNow", label, false)
                    .on_hover_text(crate::i18n::tr("Publish New and Modified photos, remove Deleted ones"))
                    .clicked()
                    && let Err(e) = app.run("dialog.publish", json!({"id": cid}))
                {
                    app.toast(ui.ctx(), e);
                }
                if crate::widgets::text_button(ui, "publishMarkDone", "mark.", false)
                    .on_hover_text(crate::i18n::tr("Mark as Up-to-Date: count the selected photos (or all) as published as they are"))
                    .clicked()
                {
                    let sel: Vec<u64> = app.session.selection.ids.iter().map(|i| i.0).collect();
                    let p = if sel.len() > 1 { json!({"id": cid, "ids": sel}) } else { json!({"id": cid}) };
                    if let Err(e) = app.run("publish.markUpToDate", p) {
                        app.toast(ui.ctx(), e);
                    }
                }
                if crate::widgets::text_button(ui, "publishRemove", "rem.", false)
                    .on_hover_text(crate::i18n::tr("Take the selected photos out (their copies are removed on the next publish)"))
                    .clicked()
                    && let Err(e) = app.run("publish.removePhotos", json!({"id": cid}))
                {
                    app.toast(ui.ctx(), e);
                }
            });
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_tree(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    cols: &[lightcraft_engine::publish::Collection],
    counts: &std::collections::HashMap<u64, (usize, usize)>,
    service: u64,
    parent: Option<u64>,
    indent: f32,
) {
    if indent > 16.0 * 32.0 {
        return;
    }
    for c in cols.iter().filter(|c| c.service == service && c.parent == parent) {
        let (todo, total) = counts.get(&c.id).copied().unwrap_or((0, 0));
        if c.set {
            let open_id = egui::Id::new(("publish-set-open", c.id));
            let open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(true);
            let resp =
                row_ex(app, ui, &format!("publishSet:{}", c.id), Icon::CollectionSet, &c.name, None, None, false, indent, &RowExtra::default());
            if resp.clicked() {
                ui.data_mut(|d| d.insert_temp(open_id, !open));
            }
            collection_menu(app, &resp, c);
            if open {
                publish_tree(app, ui, cols, counts, service, Some(c.id), indent + 16.0);
            }
            continue;
        }
        let sel = app.session.source == LibrarySource::Published(c.id);
        let extra = RowExtra { badge: (todo > 0 && app.ui.show_counts).then(|| format!("+{todo} · {total}")), ..Default::default() };
        let resp = row_ex(app, ui, &format!("published:{}", c.id), Icon::Share, &c.name, None, Some(total), sel, indent, &extra);
        let resp = if todo > 0 { resp.on_hover_text(format!("{todo} to publish")) } else { resp };
        publish_drop_target(app, ui, &resp, c);
        if resp.clicked() {
            let _ = app.run("library.source", json!({"kind": "published", "id": c.id}));
        }
        collection_menu(app, &resp, c);
    }
}

/// A published collection's row while photos are dragged from the grid: a release adds them.
fn publish_drop_target(app: &mut LightcraftApp, ui: &mut egui::Ui, resp: &egui::Response, c: &lightcraft_engine::publish::Collection) {
    let Some(ids) = app.ui.dragging_photos.clone() else { return };
    let over = ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| resp.rect.contains(p));
    if !over {
        return;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(resp.rect.shrink2(vec2(8.0, 1.0)), 4.0, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    if ui.input(|i| i.pointer.any_released()) {
        let n = ids.len();
        match app.run("publish.addPhotos", json!({"id": c.id, "ids": ids})) {
            Ok(_) => app.toast(ui.ctx(), format!("Added {n} photo{} to “{}” (to publish)", if n == 1 { "" } else { "s" }, c.name)),
            Err(e) => app.toast(ui.ctx(), e),
        }
        app.ui.dragging_photos = None;
    }
}

fn service_menu(app: &mut LightcraftApp, resp: &egui::Response, svc: &lightcraft_engine::publish::Service) {
    use crate::panels::dialogs::prompt;
    resp.context_menu(|ui| {
        let id = svc.id;
        let mut r: Option<Result<serde_json::Value, String>> = None;
        if menu_item(ui, "publishCreateCollection", crate::i18n::tr("Create Published Collection…"), "A collection of photos to publish").clicked()
        {
            prompt(app, "Create Published Collection", "Name", "", "publish.createCollection", json!({"service": id}), "name");
            ui.close();
        }
        if menu_item(ui, "publishCreateSet", crate::i18n::tr("Create Published Collection Set…"), "A set to arrange collections in").clicked() {
            prompt(app, "Create Published Collection Set", "Name", "", "publish.createCollection", json!({"service": id, "set": true}), "name");
            ui.close();
        }
        ui.separator();
        ui.menu_button(crate::i18n::tr("Export Settings"), |ui| {
            for (p, builtin) in app.session.all_export_presets() {
                if ui.button(crate::i18n::builtin_label(&p.name, builtin)).clicked() {
                    r = Some(app.run("publish.editService", json!({"id": id, "preset": p.name})));
                    ui.close();
                }
            }
        });
        if svc.kind == lightcraft_engine::publish::ServiceKind::HardDrive
            && menu_item(ui, "publishChangeFolder", crate::i18n::tr("Change Folder…"), "Where the collections' folders go").clicked()
        {
            if let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f()) {
                r = Some(app.run("publish.editService", json!({"id": id, "dir": dir})));
            }
            ui.close();
        }
        if menu_item(ui, "publishRenameService", crate::i18n::tr("Rename…"), "The service's name").clicked() {
            prompt(app, "Rename Publish Service", "Name", &svc.name, "publish.editService", json!({"id": id}), "name");
            ui.close();
        }
        ui.separator();
        if menu_item(
            ui,
            "publishDeleteService",
            crate::i18n::tr("Delete Service"),
            "The service and its collections (published files stay; ⌘Z brings it back)",
        )
        .clicked()
        {
            r = Some(app.run("publish.deleteService", json!({"id": id})));
            ui.close();
        }
        if let Some(Err(e)) = r {
            app.toast(ui.ctx(), e);
        }
    });
}

fn collection_menu(app: &mut LightcraftApp, resp: &egui::Response, c: &lightcraft_engine::publish::Collection) {
    use crate::panels::dialogs::prompt;
    resp.context_menu(|ui| {
        let id = c.id;
        let mut r: Option<Result<serde_json::Value, String>> = None;
        if c.set {
            if menu_item(ui, "publishCreateInSet", crate::i18n::tr("Create Published Collection…"), "A collection inside this set").clicked() {
                prompt(
                    app,
                    "Create Published Collection",
                    "Name",
                    "",
                    "publish.createCollection",
                    json!({"service": c.service, "parent": id}),
                    "name",
                );
                ui.close();
            }
            if menu_item(ui, "publishCreateSetInSet", crate::i18n::tr("Create Published Collection Set…"), "A set inside this set").clicked() {
                prompt(
                    app,
                    "Create Published Collection Set",
                    "Name",
                    "",
                    "publish.createCollection",
                    json!({"service": c.service, "parent": id, "set": true}),
                    "name",
                );
                ui.close();
            }
        } else {
            if menu_item(ui, "publishCollectionNow", crate::i18n::tr("Publish Now"), "Publish New and Modified photos, remove Deleted ones").clicked()
            {
                r = Some(app.run("dialog.publish", json!({"id": id})));
                ui.close();
            }
            if menu_item(ui, "publishMarkAll", crate::i18n::tr("Mark as Up-to-Date"), "Count every photo as published as it is now").clicked() {
                r = Some(app.run("publish.markUpToDate", json!({"id": id})));
                ui.close();
            }
            if menu_item(ui, "publishAddSelected", crate::i18n::tr("Add Selected Photos"), "They show as New until published").clicked() {
                r = Some(app.run("publish.addPhotos", json!({"id": id})));
                ui.close();
            }
        }
        ui.separator();
        if menu_item(ui, "publishRenameCollection", crate::i18n::tr("Rename…"), "The collection's name").clicked() {
            prompt(app, "Rename Published Collection", "Name", &c.name, "publish.editCollection", json!({"id": id}), "name");
            ui.close();
        }
        if menu_item(ui, "publishDeleteCollection", crate::i18n::tr("Delete"), "Published files stay where they are; ⌘Z brings it back").clicked() {
            r = Some(app.run("publish.deleteCollection", json!({"id": id})));
            if app.session.source == LibrarySource::Published(id) {
                let _ = app.run("library.source", json!({"kind": "all"}));
            }
            ui.close();
        }
        if let Some(Err(e)) = r {
            app.toast(ui.ctx(), e);
        }
    });
}

/// How Collections lists its albums (L24).
#[derive(Default)]
struct AlbumView {
    needle: String,
    by_kind: bool,
    fav_only: bool,
}

/// An album passes the filter itself, or (a set) holds one that does.
fn album_visible(all: &[Album], a: &Album, v: &AlbumView, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    let own = (v.needle.is_empty() || a.name.to_lowercase().contains(&v.needle)) && (!v.fav_only || a.favorite);
    own || (a.folder && all.iter().filter(|c| c.parent == Some(a.id)).any(|c| album_visible(all, c, v, depth + 1)))
}

/// The Label submenu and the favourite toggle of a folder's or a collection's context menu.
fn marks_menu(
    app: &mut LightcraftApp,
    ui: &mut egui::Ui,
    kind: &str,
    target: serde_json::Value,
    label: Option<lightcraft_catalog::ColorLabel>,
    favorite: bool,
) {
    let fav = if favorite { "Remove from Favorites" } else { "Mark as Favorite" };
    if ui.button(crate::i18n::tr(fav)).clicked() {
        let mut p = target.clone();
        p["on"] = json!(!favorite);
        if let Err(e) = app.run(&format!("{kind}.favorite"), p) {
            app.toast(ui.ctx(), e);
        }
        ui.close();
    }
    ui.menu_button(crate::i18n::tr("Label"), |ui| {
        for l in lightcraft_catalog::ColorLabel::ALL {
            let name = app.session.catalog.label_name(l);
            let on = label == Some(l);
            if ui.selectable_label(on, name).clicked() {
                let mut p = target.clone();
                p["label"] = if on { serde_json::Value::Null } else { json!(format!("{l:?}").to_lowercase()) };
                if let Err(e) = app.run(&format!("{kind}.label"), p) {
                    app.toast(ui.ctx(), e);
                }
                ui.close();
            }
        }
        if ui.selectable_label(label.is_none(), crate::i18n::tr("None")).clicked() {
            let mut p = target.clone();
            p["label"] = serde_json::Value::Null;
            let _ = app.run(&format!("{kind}.label"), p);
            ui.close();
        }
    });
}

/// Colours that tell disks apart: a dot on each shoot and on its disk's Folders row.
const DISK_COLORS: [egui::Color32; 6] = [
    egui::Color32::from_rgb(64, 178, 170),
    egui::Color32::from_rgb(230, 146, 64),
    egui::Color32::from_rgb(156, 116, 226),
    egui::Color32::from_rgb(84, 144, 232),
    egui::Color32::from_rgb(120, 184, 84),
    egui::Color32::from_rgb(222, 104, 152),
];

/// The colour of the disk mounted at `disk`, by its place among the library's disks.
fn disk_color(tree: &[FolderNode], disk: &str) -> egui::Color32 {
    let i = tree.iter().position(|v| same_folder(&v.path, disk)).unwrap_or(0);
    DISK_COLORS.get(i % DISK_COLORS.len()).copied().unwrap_or(egui::Color32::GRAY)
}

/// A disk's name as Folders shows it: the startup disk by its own name (`Macintosh HD`).
fn disk_label(ui: &egui::Ui, path: &str, name: &str) -> String {
    if path == "/" {
        fs_cached(ui, "startup-name", "/", 600.0, |_| lightcraft_engine::disks::startup_name())
            .flatten()
            .unwrap_or_else(|| crate::i18n::tr("This Computer").to_string())
    } else {
        name.to_string()
    }
}

/// A short text button at the right edge of a section header (kept at the visible edge when the
/// sidebar scrolls sideways); returns its response (the caller hangs a menu on it).
fn header_text_button(ui: &mut egui::Ui, header: Rect, viewport: Rect, id: &str, label: &str) -> egui::Response {
    let right = (header.left() + viewport.max.x).min(header.right()) - 8.0;
    let mut hdr = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(right - 80.0, header.top()), pos2(right, header.bottom())))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    crate::widgets::text_button(&mut hdr, id, label, false)
}

/// A context-menu item with a hover note, findable by agents and tests as `menu:<id>`.
fn menu_item(ui: &mut egui::Ui, id: &str, label: &str, tip: &str) -> egui::Response {
    let resp = ui.button(label).on_hover_text(crate::i18n::tr(tip));
    register(ui.ctx(), format!("menu:{id}"), resp.rect);
    resp
}

/// Import from here: the Import window, reading `path`, adding in place (Add) by default.
fn import_from(app: &mut LightcraftApp, ctx: &egui::Context, path: &str) {
    if let Err(e) = app.run("file.addPhotos", json!({"source": path, "mode": "add"})) {
        app.toast(ctx, e);
    }
}

/// "shoots.": the library's photo work in one flat list, whatever disk it is on — one row per
/// shoot folder (camera and format folders such as `raw` or `M262` belong to the shoot above
/// them; see `lightcraft_catalog::shoots`), pinned ones first, then the newest import or edit
/// first (or by name: sort.). A row says the shoot's name, its disk's colour and its photo count,
/// quiet while its disk is not connected; a click shows all of its photos. New imports show up
/// here by themselves.
fn shoots_section(app: &mut LightcraftApp, ui: &mut egui::Ui, viewport: Rect) {
    let all = app.caches.shoots(&app.session);
    let tree = app.caches.folder_tree(&app.session);
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    let (hr, open) = sidebar_section_header(app, ui, "shoots", "shoots.");
    let sort = header_text_button(ui, hr, viewport, "shootsSort", "sort.");
    // fldr. / cam.: what the rows under each shoot's triangle are, 8 px before sort.
    parts_by_toggle(app, ui, hr, sort.rect.left() - 8.0, "shootsPartsBy");
    let removed: Vec<&lightcraft_engine::cmd::shoots::ShootRow> = all.iter().filter(|r| r.hidden).collect();
    egui::Popup::menu(&sort).show(|ui| {
        let by = app.session.shoot_prefs.sort;
        use lightcraft_engine::cmd::shoots::ShootSort;
        for (key, label, tip, on) in
            [("date", "date.", "Newest import or edit first", by == ShootSort::Date), ("name", "name.", "By name, A to Z", by == ShootSort::Name)]
        {
            if ui.selectable_label(on, label).on_hover_text(tip).clicked() {
                let _ = app.run("shoot.sort", json!({"by": key}));
                ui.close();
            }
        }
        if !removed.is_empty() {
            ui.separator();
            ui.menu_button(format!("removed. {}", removed.len()), |ui| {
                for r in &removed {
                    if ui.button(&r.shoot.name).on_hover_text(format!("Put back in shoots.\n{}", r.shoot.path)).clicked() {
                        let _ = app.run("shoot.hide", json!({"path": r.shoot.path, "on": false}));
                        ui.close();
                    }
                }
            });
        }
    });
    if !open {
        return;
    }
    let rows: Vec<&lightcraft_engine::cmd::shoots::ShootRow> = all.iter().filter(|r| !r.hidden).collect();
    if rows.is_empty() {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 29.0), Sense::hover());
        let msg = if removed.is_empty() { "Imported folders show up here" } else { "Every shoot is removed (sort. ▸ removed.)" };
        ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(msg), t.font(12.5), t.text_dim);
        register(ui.ctx(), "label:shootsEmpty".to_string(), r);
        return;
    }
    let chosen = app.session.library_folder.clone().filter(|_| app.session.source == LibrarySource::LibraryFolder);
    let mut after_pinned = false;
    for r in rows {
        // a gap between the pinned shoots and the rest
        if after_pinned && !r.pinned {
            ui.add_space(8.0);
        }
        after_pinned = r.pinned;
        let s = &r.shoot;
        // (checked off the UI thread, every few seconds: a disk can be asleep or gone)
        let gone = !cfg!(target_arch = "wasm32") && fs_cached(ui, "is-dir", &s.path, 10.0, is_dir) == Some(false);
        let disk_gone = gone && s.disk != "/" && fs_cached(ui, "is-dir", &s.disk, 10.0, is_dir) == Some(false);
        // narrowed to some of its folders or cameras: those rows are the chosen ones
        let sel = chosen.as_deref().is_some_and(|c| same_folder(c, &s.path))
            && app.session.library_folder_subfolders == Some(s.deep)
            && app.session.active_album_part().is_none();
        let extra = RowExtra { dim: gone, badge: None, dot: Some(disk_color(&tree, &s.disk)), accent: r.pinned, reserve: 0.0 };
        let resp = row_ex(app, ui, &format!("shoot:{}", s.path), Icon::GridPhoto, &s.name, Some(&s.name), Some(s.count), sel, 0.0, &extra);
        let disk = disk_label(ui, &s.disk, &s.disk_name);
        let mut tip = format!("{}\n{disk} · {} photo{}", s.name, s.count, if s.count == 1 { "" } else { "s" });
        if let Some(day) = s.latest.get(..10) {
            tip.push_str(&format!(" · last import or edit {day}"));
        }
        if r.pinned {
            tip.push_str(" · pinned");
        }
        tip.push('\n');
        tip.push_str(&s.path);
        if !s.deep {
            tip.push_str("\nOnly the photos lying in this folder (its subfolders are shoots of their own)");
        }
        if disk_gone {
            tip.push_str(&format!("\nNot connected: plug in {disk} to see these photos"));
        } else if gone {
            tip.push_str("\nNot found: right-click ▸ Find Missing Folder… to point the library at where it went");
        }
        // LRC-LIB-COLL-PARTS: the triangle opens the folders (or cameras) of the shoot
        let owner = Owner::Shoot(s);
        let parts_open = app.ui.parts_open.contains(&owner.open_key());
        let mut toggled = false;
        if s.count > 0 {
            let tri = parts_triangle(ui, &resp, &owner, 0.0, parts_open);
            toggled = tri.clicked();
            shoot_menu(app, &tri, r, gone);
        }
        let resp = resp.on_hover_text(tip);
        if resp.clicked()
            && !toggled
            && let Err(e) = app.run("shoot.show", json!({"path": s.path}))
        {
            app.toast(ui.ctx(), e);
        }
        shoot_menu(app, &resp, r, gone);
        if toggled {
            set_parts_open(app, &owner.open_key(), !parts_open);
        }
        if parts_open && s.count > 0 {
            part_rows(app, ui, owner, 16.0);
        }
    }
}

/// The context menu of a shoot row.
fn shoot_menu(app: &mut LightcraftApp, resp: &egui::Response, r: &lightcraft_engine::cmd::shoots::ShootRow, gone: bool) {
    let s = &r.shoot;
    let path = s.path.as_str();
    resp.context_menu(|ui| {
        if !gone
            && app.services.reveal.is_some()
            && ui.button(crate::i18n::tr("Show in Finder")).clicked()
            && let Some(f) = app.services.reveal.as_mut()
        {
            let _ = f(path);
            ui.close();
        }
        if !gone && menu_item(ui, "import", "import.", "Import from here: the Import window, reading this folder").clicked() {
            import_from(app, ui.ctx(), path);
            ui.close();
        }
        if !gone && ui.button(crate::i18n::tr("Synchronize Folder…")).clicked() {
            synchronize(app, ui.ctx(), path, Some(s.deep));
            ui.close();
        }
        if app.services.pick_folder.is_some()
            && ui
                .button(crate::i18n::tr("Find Missing Folder…"))
                .on_hover_text(crate::i18n::tr("The folder was moved or renamed: pick where it is now, and its photos are relinked"))
                .clicked()
        {
            find_missing(app, ui.ctx(), path);
            ui.close();
        }
        ui.separator();
        if menu_item(ui, "rename", "rename.", "The name shoots. shows; the folder on disk keeps its name").clicked() {
            app.ui.dialog = Some(crate::state::Dialog::TextPrompt {
                title: format!("Rename “{}”", s.name),
                hint: format!("Name in shoots. (empty: {})", r.auto_name),
                value: s.name.clone(),
                command: "shoot.rename".into(),
                params: json!({"path": path}),
                key: "name".into(),
            });
            ui.close();
        }
        let (label, tip) = if r.pinned { ("unpin.", "Back among the other shoots") } else { ("pin.", "Pin to top") };
        if menu_item(ui, "pin", label, tip).clicked() {
            let _ = app.run("shoot.pin", json!({"path": path, "on": !r.pinned}));
            ui.close();
        }
        if menu_item(ui, "remove", "remove.", "Remove from shoots.: the photos stay in the library and in Folders (sort. ▸ removed. puts it back)")
            .clicked()
        {
            let _ = app.run("shoot.hide", json!({"path": path, "on": true}));
            ui.close();
        }
    });
}

/// Find Missing Folder…: pick where `path` went; its photos are relinked.
fn find_missing(app: &mut LightcraftApp, ctx: &egui::Context, path: &str) {
    let to = app.services.pick_folder.as_mut().and_then(|f| f());
    if let Some(to) = to {
        match app.run("folder.locate", json!({"path": path, "to": to})) {
            Ok(r) => app.toast(ctx, format!("Found: {} photo(s) relinked, {} still missing", r["relinked"], r["stillMissing"])),
            Err(e) => app.toast(ctx, e),
        }
    }
}

/// "Folders", as in Lightroom Classic: a row per disk (its name, its colour dot and how full it
/// is; quiet when it is not connected), under it only the folders photos were imported from (the
/// library's root folders, see `lightcraft_catalog::folders::root_folders`) with photo counts —
/// the folders inside included while Library ▸ Show Photos in Subfolders is on. A root folder
/// whose own name says nothing (`M262`) carries its shoot's (`Erika and Connor wedding › M262`),
/// and a folder that only leads to one other is one row (`chicago › -raw`), so a folder opens
/// only where imported folders with photos branch. A click makes that folder the source, like an
/// album. The tree comes from the catalog, never from browsing the disk: anything not imported is
/// reached with Show in Finder / import. (or the Import window).
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
        ui.separator();
        subfolders_toggle(app, ui);
    });
    if open {
        reveal_chosen(app, ui, &tree);
        folder_rows(app, ui, &tree, &tree, 0.0, false);
    }
}

/// Whenever the folder shown from Folders changes (a click, an agent, a rename or its undo), open
/// the rows above it so it is on screen; folding one by hand afterwards sticks until the choice
/// changes. A shoot chosen in shoots. opens nothing here.
fn reveal_chosen(app: &mut LightcraftApp, ui: &egui::Ui, tree: &[FolderNode]) {
    let shown = app
        .session
        .library_folder
        .clone()
        .filter(|_| app.session.source == LibrarySource::LibraryFolder && app.session.library_folder_subfolders.is_none());
    let chosen = shown.filter(|c| !lightcraft_catalog::query::folder_key(c).is_empty());
    let seen = egui::Id::new("libfolder-revealed");
    let now = chosen.as_deref().map(lightcraft_catalog::query::folder_key);
    if ui.data(|d| d.get_temp::<Option<String>>(seen)) == Some(now.clone()) {
        return;
    }
    ui.data_mut(|d| d.insert_temp(seen, now));
    let Some(chosen) = chosen else { return };
    fn open_above(app: &mut LightcraftApp, nodes: &[FolderNode], chosen: &str, depth: usize) {
        for n in nodes {
            if depth < lightcraft_catalog::folders::MAX_DEPTH
                && lightcraft_catalog::query::folder_within(chosen, &n.path)
                && !same_folder(chosen, &n.path)
            {
                app.ui.folders_open.insert(lightcraft_catalog::query::folder_key(&n.path), true);
                open_above(app, &n.children, chosen, depth + 1);
            }
        }
    }
    open_above(app, tree, &chosen, 0);
}

fn is_dir(p: &str) -> bool {
    std::path::Path::new(p).is_dir()
}

/// A folder and the folders it only leads to (no photo of its own, one folder inside), as one
/// row: the names on the way and the last folder (whose subfolders are the row's).
fn chain(n: &FolderNode) -> (Vec<&str>, &FolderNode) {
    let mut names = vec![n.name.as_str()];
    let mut last = n;
    while let [only] = last.children.as_slice()
        && last.own == 0
        && !last.volume
        && names.len() < lightcraft_catalog::folders::MAX_DEPTH
    {
        names.push(only.name.as_str());
        last = only;
    }
    (names, last)
}

/// The rows of `nodes` (disks at the top level; `tree` is the whole tree, for disk colours).
/// `offline`: their disk is not connected, so nothing below is checked on disk.
fn folder_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, tree: &[FolderNode], nodes: &[FolderNode], indent: f32, offline: bool) {
    let t = Tokens::get(ui.ctx());
    let subfolders = app.session.library_subfolders;
    for n in nodes {
        let key = lightcraft_catalog::query::folder_key(&n.path);
        // a disk starts open, a folder closed; a row opened or closed by hand stays so
        let mut open: bool = app.ui.folders_open.get(&key).copied().unwrap_or(n.volume);
        // a row whose path would cover other disks' photos too only opens and closes
        let selectable = n.selectable;
        let sel = selectable
            && app.session.source == LibrarySource::LibraryFolder
            && app.session.library_folder_subfolders.is_none()
            && app.session.library_folder.as_deref().is_some_and(|f| same_folder(f, &n.path));
        // (checked off the UI thread, every few seconds: a disk can be asleep or gone)
        let gone = offline || (!cfg!(target_arch = "wasm32") && fs_cached(ui, "is-dir", &n.path, 10.0, is_dir) == Some(false));
        let (names, last) = if n.volume { (vec![n.name.as_str()], n) } else { chain(n) };
        let (icon, name, extra, count) = if n.volume {
            let name = disk_label(ui, &n.path, &n.name);
            let badge = if gone {
                Some(crate::i18n::tr("not connected").to_string())
            } else {
                fs_cached(ui, "disk-space", &n.path, 30.0, lightcraft_engine::disks::space)
                    .flatten()
                    .map(|(tot, av)| lightcraft_engine::disks::space_label(tot, av))
            };
            (Icon::Disk, name, RowExtra { dim: gone, badge, dot: Some(disk_color(tree, &n.path)), accent: false, reserve: 0.0 }, Some(n.count))
        } else {
            // a root folder (right under its disk) named only by its camera carries its shoot's name
            let first = if indent <= 16.0 { lightcraft_catalog::shoots::context_label(&n.path, &n.name) } else { n.name.clone() };
            let mut label = std::iter::once(first).chain(names.iter().skip(1).map(|s| s.to_string())).collect::<Vec<_>>().join(" › ");
            if gone {
                label.push_str(" ?");
            }
            // L24: the folder's favourite star and colour label
            let rec = app.session.catalog.folder_record(&last.path).cloned().unwrap_or_default();
            if rec.favorite {
                label = format!("★ {label}");
            }
            let dot = rec.label.map(crate::theme::label_color);
            (Icon::Folder, label, RowExtra { dim: gone, dot, ..Default::default() }, Some(if subfolders { n.count } else { n.own }))
        };
        let children = &last.children;
        let resp = row_ex(app, ui, &format!("libfolder:{}", n.path), icon, &name, Some(&name), count, sel, indent, &extra);
        let mut toggled = false;
        if !children.is_empty() {
            // disclosure triangle left of the icon
            let c = pos2(resp.rect.left() + 10.0 + indent, resp.rect.center().y);
            let tri = Rect::from_center_size(c, vec2(14.0, 14.0));
            let tr = ui.interact(tri, egui::Id::new(("libfolder-tri", key.clone())), Sense::click());
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
        let mut tip = format!("{}\n{} photo{}", last.path, photos, if photos == 1 { "" } else { "s" });
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
            app.ui.folders_open.insert(key, open);
        }
        row_menu(app, &resp, n, gone);
        if !n.volume {
            // merges.: the merges found in it (an open folder: its own; a closed one: with the folders inside it)
            let deep = !(open && !children.is_empty());
            let found = crate::merges::counts(app, ui.ctx()).folder(&last.path, deep);
            if found > 0 {
                merges_row(app, ui, lightcraft_engine::merge_find::Scope::Folder { path: last.path.clone(), deep }, found, indent + 16.0);
            }
            // pipeline.editing: a shoot in Second Brain's pipeline (on its topmost row only)
            if let Some(pipe) = crate::sb_pipeline::current(app, ui)
                && let Some(mark) = crate::sb_pipeline::row_mark(&pipe, &n.path, &last.path)
            {
                pipeline_row(app, ui, &last.path, mark, indent + 16.0);
            }
        }
        if open && !children.is_empty() {
            folder_rows(app, ui, tree, children, indent + 16.0, gone);
        }
    }
}

/// merges.: a row under a folder or collection with merges found in it (brackets, panoramas,
/// focus stacks to review, and the merged photos); a click shows them (`library.source {kind:
/// merges}`).
fn merges_row(app: &mut LightcraftApp, ui: &mut egui::Ui, scope: lightcraft_engine::merge_find::Scope, count: usize, indent: f32) {
    use lightcraft_engine::merge_find::Scope;
    let sel = app.session.source == LibrarySource::Merges && app.session.merges_scope.as_ref() == Some(&scope);
    let (id, params) = match &scope {
        Scope::Folder { path, deep } => (format!("merges:folder:{path}"), json!({"kind": "merges", "path": path, "deep": deep})),
        Scope::Album { id } => (format!("merges:album:{}", id.0), json!({"kind": "merges", "id": id.0})),
        Scope::All => ("merges:all".to_string(), json!({"kind": "merges"})),
    };
    let resp = row_ex(app, ui, &id, Icon::Stack, "merges.", Some("merges."), Some(count), sel, indent, &RowExtra::default());
    let resp = resp.on_hover_text(crate::i18n::tr(
        "Photo merges found here — brackets (hdr.), panoramas (pano.), focus stacks (stack.) — to review and merge, and the merged photos",
    ));
    if resp.clicked()
        && let Err(e) = app.run("library.source", params)
    {
        app.toast(ui.ctx(), e);
    }
}

/// pipeline.<stage>: a row under a folder that is a shoot in Second Brain's pipeline (its stage, or
/// `pipeline.pending` when sent from here and not taken yet); a click shows the folder.
fn pipeline_row(app: &mut LightcraftApp, ui: &mut egui::Ui, folder: &str, mark: lightcraft_engine::sb_pipeline::Mark<'_>, indent: f32) {
    let tag = mark.tag();
    let id = format!("pipeline:{folder}");
    let resp = row_ex(app, ui, &id, Icon::Activity, &tag, Some(&tag), None, false, indent, &RowExtra::default());
    let resp = resp.on_hover_text(crate::sb_pipeline::describe(mark));
    if resp.clicked()
        && let Err(e) = app.run("library.source", json!({"kind": "libraryFolder", "path": folder}))
    {
        app.toast(ui.ctx(), e);
    }
}

/// The context menu of a Folders row: a folder's own, a disk's (show it, import from it, remove
/// it), the startup disk's.
fn row_menu(app: &mut LightcraftApp, resp: &egui::Response, n: &FolderNode, gone: bool) {
    if !n.volume {
        folder_menu_for_library(app, resp, n, gone);
    } else if n.path != "/" {
        resp.context_menu(|ui| {
            subfolders_toggle(app, ui);
            ui.separator();
            if !gone
                && app.services.reveal.is_some()
                && ui.button(crate::i18n::tr("Show in Finder")).clicked()
                && let Some(f) = app.services.reveal.as_mut()
            {
                let _ = f(&n.path);
                ui.close();
            }
            if !gone && menu_item(ui, "import", "import.", "Import from here: the Import window, reading this disk").clicked() {
                import_from(app, ui.ctx(), &n.path);
                ui.close();
            }
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
/// `subfolders`: the folders inside it too, or not (`None`: Show Photos in Subfolders).
fn synchronize(app: &mut LightcraftApp, ctx: &egui::Context, path: &str, subfolders: Option<bool>) {
    let mut params = json!({"path": path, "dryRun": true});
    if let Some(b) = subfolders {
        params["subfolders"] = json!(b);
    }
    match app.run("folder.sync", params) {
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
        marks_menu(
            app,
            ui,
            "folder",
            json!({"path": path}),
            app.session.catalog.folder_record(path).cloned().unwrap_or_default().label,
            app.session.catalog.folder_record(path).is_some_and(|r| r.favorite),
        );
        ui.separator();
        if ui.button(crate::i18n::tr("Synchronize Folder…")).clicked() {
            synchronize(app, ui.ctx(), path, None);
            ui.close();
        }
        if ui.button(crate::i18n::tr("Create Album from Folder")).clicked() {
            match app.run("album.fromFolder", json!({"path": path})) {
                Ok(r) => app.toast(ui.ctx(), format!("Album “{name}” with {} photos", r["photos"].as_u64().unwrap_or(0))),
                Err(e) => app.toast(ui.ctx(), e),
            }
            ui.close();
        }
        if app.services.pick_folder.is_some()
            && ui
                .button(crate::i18n::tr("Find Missing Folder…"))
                .on_hover_text(crate::i18n::tr("The folder was moved or renamed: pick where it is now, and its photos are relinked"))
                .clicked()
        {
            find_missing(app, ui.ctx(), path);
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
        if !gone && menu_item(ui, "import", "import.", "Import from here: the Import window, reading this folder").clicked() {
            import_from(app, ui.ctx(), path);
            ui.close();
        }
        // a folder not in Second Brain's shoot pipeline yet can be sent there (it lands in editing)
        let in_pipeline = crate::sb_pipeline::current(app, ui).is_some_and(|p| p.mark(path).is_some());
        if !in_pipeline
            && menu_item(
                ui,
                "sendToPipeline",
                "send.to.pipeline.",
                "Send this folder to Second Brain's shoot pipeline: asks about its backup, then it lands in editing",
            )
            .clicked()
        {
            crate::sb_pipeline::open_dialog(app, path);
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
            if ui.button(crate::i18n::tr("Edit Keyword Tag…")).clicked() {
                let _ = app.run("dialog.editKeyword", json!({"keyword": n.path}));
            }
            if ui.button(crate::i18n::tr("Rename Keyword…")).clicked() {
                app.ui.dialog = Some(crate::state::Dialog::RenameKeyword { from: n.path.clone(), to: n.path.clone() });
            }
            if ui.button(crate::i18n::tr("Merge into…")).clicked() {
                app.ui.dialog = Some(crate::state::Dialog::MergeKeywords { from: vec![n.path.clone()], into: String::new() });
            }
            if ui.button(crate::i18n::tr("Use as Keyword Shortcut")).clicked() {
                let _ = app.run("keyword.setShortcut", json!({"keywords": [n.path]}));
            }
            if ui.button(crate::i18n::tr("Put New Keywords Inside This Keyword")).clicked() {
                let _ = app.run("keyword.setDefaultParent", json!({"keyword": n.path}));
            }
            if ui.button(crate::i18n::tr("Delete Keyword")).clicked() {
                let _ = app.run("keyword.delete", json!({"keyword": n.path}));
            }
            ui.separator();
            if ui.button(crate::i18n::tr("Purge Unused Keywords")).clicked() {
                let _ = app.run("keyword.purgeUnused", json!({}));
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
