//! Lightroom Classic's Import window (File ▸ Import Photos and Video…, ⌘⇧I, and the Library
//! panel's "Import…"): the source on the left ("From:" devices and folders, Include Subfolders),
//! the transfer mode in the middle (Copy as DNG | Copy | Move | Add), the destination on the right
//! ("To:"); a grid of the source's photos with checkboxes (already-imported ones dimmed and
//! unchecked), All Photos / New Photos / Destination Folders, sort and thumbnail size; File
//! Handling, File Renaming, Apply During Import and Destination options; Import / Cancel. The
//! import itself is [`crate::import::start`] (worker thread, progress in the top bar); afterwards
//! the Library shows Previous Import. The choices are remembered (`ui.json` → `importPrefs`).

use egui::{Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::import::{DATE_FORMATS, ImportDialog, ImportPrefs, MODES};
use crate::render::Slot;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets::register;

/// The Import window is available (a desktop build that can read folders).
pub fn has_window() -> bool {
    cfg!(not(target_arch = "wasm32"))
}

/// File ▸ Import Photos and Video…: open the Import window. `{source?, mode?: dng|copy|move|add,
/// destination?, subfolders?}` override the remembered choices; without a source the last one
/// (or a camera card that is plugged in) is read.
pub fn open_window(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    if app.import.as_ref().is_some_and(|t| !t.browse) {
        return Err("an import is running".into());
    }
    let mut prefs = app.ui.import_prefs.clone();
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let mut source = s("source");
    if source.is_none() {
        // a card or camera that is plugged in comes first (Classic)
        if let Some(d) = lightcraft_engine::devices::devices().into_iter().next() {
            source = Some(d.path);
            if prefs.mode == "add" {
                prefs.mode = "copy".into();
            }
        }
    }
    if let Some(src) = source {
        prefs.source = src;
    }
    if let Some(m) = s("mode") {
        if !MODES.iter().any(|x| x.0 == m) {
            return Err(format!("unknown import mode `{m}` (dng|copy|move|add)"));
        }
        prefs.mode = m;
    }
    if let Some(d) = s("destination") {
        prefs.destination = d;
    }
    if let Some(b) = p.get("subfolders").and_then(Value::as_bool) {
        prefs.subfolders = b;
    }
    // a library in browser storage has nowhere to copy to
    if prefs.copies() && !can_copy(app) {
        prefs.mode = "add".into();
    }
    let d = ImportDialog { window: true, prefs, show: "all".into(), sort: "time".into(), thumb: 128.0, ..ImportDialog::new(Vec::new()) };
    let source = d.prefs.source.clone();
    let subfolders = d.prefs.subfolders;
    app.ui.dialog = Some(Dialog::Import { opts: Box::new(d) });
    if !source.is_empty() && std::path::Path::new(&source).is_dir() {
        scan(app, &source, subfolders)?;
    }
    Ok(json!({"open": true, "source": source}))
}

/// The Import window's source: `{path, subfolders?}` (the source column's clicks, or an agent).
pub fn choose_source(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let path = p.get("path").and_then(Value::as_str).ok_or("import.source: missing `path`")?.to_string();
    let Some(Dialog::Import { opts }) = app.ui.dialog.as_mut().filter(|d| matches!(d, Dialog::Import { opts } if opts.window)) else {
        return Err("import.source: the Import window isn't open (File ▸ Import Photos and Video…)".into());
    };
    if let Some(b) = p.get("subfolders").and_then(Value::as_bool) {
        opts.prefs.subfolders = b;
    }
    opts.prefs.source = path.clone();
    let subfolders = opts.prefs.subfolders;
    if !std::path::Path::new(&path).is_dir() {
        return Err(format!("{path}: not a folder"));
    }
    scan(app, &path, subfolders)?;
    Ok(json!({"source": path, "subfolders": subfolders, "scanning": true}))
}

fn can_copy(app: &LightcraftApp) -> bool {
    app.session.library.as_ref().is_some_and(|l| l.on_disk) || app.services.pick_folder.is_some()
}

/// Read `path` (with its subfolders, or only its own files) for the Import window, on a worker.
fn scan(app: &mut LightcraftApp, path: &str, subfolders: bool) -> Result<(), String> {
    if let Some(t) = app.scan.take() {
        t.cancel();
    }
    let root = path.trim_end_matches(['/', '\\']).to_string();
    let root = if root.is_empty() { path.to_string() } else { root };
    let (input, _) = lightcraft_engine::import::ScanInput::new(&mut app.session, std::slice::from_ref(&root));
    let progress = std::sync::Arc::new(lightcraft_engine::import::ScanProgress::default());
    let (tx, rx) = std::sync::mpsc::channel();
    let pr = progress.clone();
    let dir = root.clone();
    let job = move || {
        let files: Vec<String> = if subfolders { vec![dir] } else { own_files(&dir) };
        let _ = tx.send(lightcraft_engine::import::scan_with(input, &files, &pr));
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("lc-import-scan".into()).spawn(job).map_err(|e| format!("could not read the folder: {e}"))?;
    #[cfg(target_arch = "wasm32")]
    job();
    app.scan = Some(crate::import::ScanTask::for_window(progress, rx, vec![root]));
    if let Some(Dialog::Import { opts }) = app.ui.dialog.as_mut() {
        opts.scanning = true;
        opts.candidates.clear();
        opts.checked.clear();
        opts.trashed.clear();
    }
    app.renderer.forget_imports();
    Ok(())
}

/// The supported files directly in `dir` (not its subfolders), sorted.
fn own_files(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|f| {
                    f.is_file() && lightcraft_engine::import::is_supported(f) && !f.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'))
                })
                .map(|f| f.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The subfolders of `path` (name, path), sorted, hidden ones left out.
pub fn list_dirs(path: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| (e.file_name().to_string_lossy().to_string(), e.path().to_string_lossy().to_string()))
                .filter(|(n, _)| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// The Files roots of the source column: home folders, then mounted volumes.
fn roots() -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().to_string()) {
        for (name, sub) in [("Pictures", "Pictures"), ("Desktop", "Desktop"), ("Downloads", "Downloads"), ("Home", "")] {
            let p = if sub.is_empty() { home.clone() } else { format!("{home}/{sub}") };
            if std::path::Path::new(&p).is_dir() {
                v.push((name.to_string(), p));
            }
        }
    }
    for (name, path) in list_dirs("/Volumes") {
        v.push((name, path));
    }
    v
}

/// Run after an import started from the window: Previous Import in the Library grid, the chosen
/// previews, and the second copy.
pub fn after_window_import(app: &mut LightcraftApp, prefs: &ImportPrefs, imported: usize, files: &[String]) {
    if imported == 0 {
        return;
    }
    if app.session.execute("library.source", &json!({"kind": "previousImport"})).is_ok() {
        crate::menus::enter_library(app, Some(crate::state::ViewMode::PhotoGrid));
    }
    let edge = app.ui.settings.preview_edge;
    match prefs.previews.as_str() {
        "standard" => {
            let _ = app.run("library.buildPreviews", json!({"size": "standard", "edge": edge}));
        }
        "full" => {
            let _ = app.run("library.buildPreviews", json!({"size": "full"}));
        }
        _ => {}
    }
    if prefs.smart_previews {
        let _ = app.run("library.smartPreviews", json!({"background": true}));
    }
    if !prefs.second_copy.trim().is_empty() {
        second_copy(prefs.second_copy.trim(), files.to_vec(), &(app.session.clock)());
    }
}

/// Make a Second Copy To: the imported files into `<dir>/Imported on <date>/`, on a worker thread;
/// a file already there is left alone, failures are logged (the import itself has succeeded).
fn second_copy(dir: &str, files: Vec<String>, now: &str) {
    let day = now.get(..10).unwrap_or("today").to_string();
    let target = std::path::Path::new(dir).join(format!("Imported on {day}"));
    let job = move || {
        if let Err(e) = std::fs::create_dir_all(&target) {
            log::warn!("second copy: {}: {e}", target.display());
            return;
        }
        for f in files {
            let src = std::path::Path::new(&f);
            let Some(name) = src.file_name() else { continue };
            let dst = target.join(name);
            if dst.exists() {
                continue;
            }
            if let Err(e) = std::fs::copy(src, &dst) {
                log::warn!("second copy: {f}: {e}");
            }
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(e) = std::thread::Builder::new().name("lc-second-copy".into()).spawn(job) {
        log::warn!("second copy: {e}");
    }
    #[cfg(target_arch = "wasm32")]
    drop(job);
}

/// Candidate indices in the grid's order and filter.
pub fn grid_order(d: &ImportDialog) -> Vec<usize> {
    let mut v: Vec<usize> =
        (0..d.candidates.len()).filter(|i| d.show != "new" || d.candidates.get(*i).is_some_and(|c| c.duplicate.is_none())).collect();
    let key_time = |i: &usize| d.candidates.get(*i).and_then(|c| c.captured.clone()).unwrap_or_default();
    let key_name = |i: &usize| d.candidates.get(*i).map(|c| c.name.to_lowercase()).unwrap_or_default();
    match d.sort.as_str() {
        "name" => v.sort_by_key(key_name),
        "checked" => v.sort_by_key(|i| (!d.checked.get(*i).copied().unwrap_or(false), key_time(i))),
        "type" => v.sort_by_key(|i| (d.candidates.get(*i).map(|c| c.format.clone()).unwrap_or_default(), key_time(i))),
        _ => v.sort_by_key(key_time),
    }
    if d.show == "folders" {
        v.sort_by_key(|i| d.prefs.folder_for(d.candidates.get(*i).and_then(|c| c.captured.as_deref()), ""));
    }
    v
}

/// "N photos / X GB" of the checked files.
pub fn checked_summary(d: &ImportDialog) -> String {
    let (n, bytes) = d
        .candidates
        .iter()
        .enumerate()
        .filter(|(i, _)| d.checked.get(*i).copied().unwrap_or(false) && d.importable(*i))
        .fold((0usize, 0u64), |(n, b), (_, c)| (n + 1, b.saturating_add(c.file_size)));
    let size = match bytes {
        b if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{:.0} MB", b as f64 / 1e6),
        b => format!("{:.0} KB", (b as f64 / 1e3).ceil()),
    };
    format!("{n} {} / {size}", if n == 1 { crate::i18n::tr("photo") } else { crate::i18n::tr("photos") })
}

const TOP_H: f32 = 64.0;
const BOTTOM_H: f32 = 48.0;
const SOURCE_W: f32 = 240.0;
const OPTIONS_W: f32 = 300.0;

/// Draw the Import window (instead of the generic dialog frame). Esc / Cancel close it; Import
/// starts the import and closes it.
pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(Dialog::Import { opts }) = app.ui.dialog.clone() else { return };
    let mut d = *opts;
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::new("import-dim")).order(egui::Order::Middle).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(160));
    });
    let r = screen.shrink2(vec2((screen.width() * 0.03).clamp(12.0, 60.0), (screen.height() * 0.04).clamp(12.0, 48.0)));
    let mut close = false;
    let mut confirm = false;
    let shown = egui::Area::new(egui::Id::new("import-window")).order(egui::Order::Foreground).fixed_pos(r.min).show(ctx, |ui| {
        ui.set_min_size(r.size());
        ui.set_max_size(r.size());
        let p = ui.painter();
        p.rect(r, 6.0, t.chrome, Stroke::new(1.0, t.button_border), StrokeKind::Inside);
        let top = Rect::from_min_size(r.min, vec2(r.width(), TOP_H));
        let bottom = Rect::from_min_max(pos2(r.left(), r.bottom() - BOTTOM_H), r.max);
        let body = Rect::from_min_max(pos2(r.left(), top.bottom()), pos2(r.right(), bottom.top()));
        p.rect_filled(top, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.header);
        p.rect_filled(bottom, egui::CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 }, t.header);
        top_bar(app, ui, top, &mut d);
        let src = Rect::from_min_max(body.min, pos2(body.left() + SOURCE_W, body.bottom()));
        let opt = Rect::from_min_max(pos2(body.right() - OPTIONS_W, body.top()), body.max);
        let grid = Rect::from_min_max(pos2(src.right(), body.top()), pos2(opt.left(), body.bottom()));
        for x in [src.right(), opt.left()] {
            ui.painter().rect_filled(Rect::from_min_max(pos2(x, body.top()), pos2(x + 1.0, body.bottom())), 0.0, t.divider);
        }
        source_column(app, ui, src, &mut d);
        grid_column(app, ui, grid, &mut d);
        options_column(app, ui, opt, &mut d);
        let (c, k) = bottom_bar(ui, bottom, &d);
        close = c;
        confirm = k;
    });
    register(ctx, "dialog:window", shown.response.rect);
    register(ctx, "import:window", r);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    // the window's own edits go back into the dialog (unless something replaced it this frame)
    if let Some(Dialog::Import { opts }) = app.ui.dialog.as_mut()
        && opts.window
    {
        // a scan that finished this frame refilled the candidates: keep those
        let (cands, checked, trashed, sources, scanning) =
            (opts.candidates.clone(), opts.checked.clone(), opts.trashed.clone(), opts.sources.clone(), opts.scanning);
        if cands.len() != d.candidates.len() || sources != d.sources {
            d.candidates = cands;
            d.checked = checked;
            d.trashed = trashed;
            d.sources = sources;
            d.scanning = scanning;
        }
        **opts = d.clone();
    }
    if close {
        if let Some(task) = app.scan.take() {
            task.cancel();
        }
        app.ui.dialog = None;
        return;
    }
    if confirm {
        match crate::import::start(app, &d) {
            Ok(_) => app.ui.dialog = None,
            Err(e) => app.toast(ctx, e),
        }
    }
}

fn top_bar(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, d: &mut ImportDialog) {
    let t = Tokens::get(ui.ctx());
    let p = ui.painter();
    // From:
    let name = |path: &str| std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    let from = if d.prefs.source.is_empty() { crate::i18n::tr("Select a source").to_string() } else { name(&d.prefs.source) };
    p.text(pos2(r.left() + 16.0, r.top() + 20.0), Align2::LEFT_CENTER, crate::i18n::tr("From:"), t.font(11.5), t.text_dim);
    let fr = Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 30.0), vec2(SOURCE_W - 24.0, 24.0));
    p.text(fr.left_center(), Align2::LEFT_CENTER, &from, t.semibold(15.0), t.text);
    register(ui.ctx(), "label:importFrom", fr);
    ui.interact(fr, egui::Id::new("import-from"), Sense::hover()).on_hover_text(d.prefs.source.clone());
    // the mode
    let mode_w = 92.0;
    let total = mode_w * MODES.len() as f32;
    let x0 = r.center().x - total / 2.0;
    for (i, (key, label, _)) in MODES.iter().enumerate() {
        let mr = Rect::from_min_size(pos2(x0 + i as f32 * mode_w, r.top() + 10.0), vec2(mode_w - 4.0, 26.0));
        let enabled = *key == "add" || can_copy(app);
        let resp = ui.interact(mr, egui::Id::new(("import-mode", *key)), if enabled { Sense::click() } else { Sense::hover() });
        register(ui.ctx(), format!("button:importMode-{key}"), mr);
        let on = d.prefs.mode == *key;
        let c = if on {
            t.text
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_dim
        };
        ui.painter().text(mr.center(), Align2::CENTER_CENTER, crate::i18n::tr(label), if on { t.semibold(14.0) } else { t.font(14.0) }, c);
        if on {
            ui.painter().rect_filled(
                Rect::from_min_max(pos2(mr.left() + 14.0, mr.bottom() - 2.0), pos2(mr.right() - 14.0, mr.bottom())),
                1.0,
                t.text,
            );
        }
        if resp.clicked() {
            d.prefs.mode = key.to_string();
        }
    }
    let desc = MODES.iter().find(|m| m.0 == d.prefs.mode).map_or("", |m| m.2);
    let desc_r = Rect::from_min_size(pos2(x0, r.top() + 40.0), vec2(total, 18.0));
    let caution = d.prefs.mode == "move";
    ui.painter().text(desc_r.center(), Align2::CENTER_CENTER, crate::i18n::tr(desc), t.font(11.5), if caution { t.caution } else { t.text_dim });
    register(ui.ctx(), "label:importModeHelp", desc_r);
    // To:
    let to_x = r.right() - OPTIONS_W + 16.0;
    if d.prefs.copies() {
        ui.painter().text(pos2(to_x, r.top() + 20.0), Align2::LEFT_CENTER, crate::i18n::tr("To:"), t.font(11.5), t.text_dim);
        let shown = if d.prefs.destination.trim().is_empty() { crate::i18n::tr("Library Originals").to_string() } else { name(&d.prefs.destination) };
        let tr = Rect::from_min_size(pos2(to_x, r.top() + 30.0), vec2(OPTIONS_W - 100.0, 24.0));
        ui.painter().text(tr.left_center(), Align2::LEFT_CENTER, shown, t.semibold(15.0), t.text);
        register(ui.ctx(), "label:importTo", tr);
        ui.interact(tr, egui::Id::new("import-to"), Sense::hover()).on_hover_text(d.prefs.destination.clone());
        if app.services.pick_folder.is_some() {
            let br = Rect::from_min_size(pos2(r.right() - 84.0, r.top() + 30.0), vec2(70.0, 24.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(egui::Layout::left_to_right(egui::Align::Center)));
            if crate::widgets::text_button(&mut child, "importDest", "Choose…", false).clicked()
                && let Some(f) = app.services.pick_folder.as_mut().and_then(|f| f())
            {
                d.prefs.destination = f;
            }
        }
    } else {
        ui.painter().text(pos2(to_x, r.top() + 20.0), Align2::LEFT_CENTER, crate::i18n::tr("To:"), t.font(11.5), t.text_dim);
        ui.painter().text(pos2(to_x, r.top() + 42.0), Align2::LEFT_CENTER, crate::i18n::tr("My Catalog"), t.semibold(15.0), t.text);
    }
}

/// A Classic section header inside the window; returns whether it is open.
fn section(ui: &mut egui::Ui, id: &str, title: &str, right: bool) -> bool {
    let key = egui::Id::new(("import-section", id));
    let open: bool = ui.data(|m| m.get_temp(key)).unwrap_or(true);
    let (resp, _) = crate::widgets::classic_header(ui, &format!("import.{id}"), title, open, right, None);
    if resp.clicked() {
        ui.data_mut(|m| m.insert_temp(key, !open));
    }
    open
}

fn source_column(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, d: &mut ImportDialog) {
    let mut col = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
    col.set_clip_rect(r);
    egui::ScrollArea::vertical().id_salt("import-source").auto_shrink([false, false]).show(&mut col, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.set_width(r.width());
        egui::Frame::NONE.inner_margin(egui::Margin { left: 12, right: 8, top: 8, bottom: 8 }).show(ui, |ui| {
            let mut sub = d.prefs.subfolders;
            let resp = ui.checkbox(&mut sub, crate::i18n::tr("Include Subfolders"));
            register(ui.ctx(), "check:importSubfolders", resp.rect);
            if resp.changed() {
                d.prefs.subfolders = sub;
                if !d.prefs.source.is_empty() {
                    let src = d.prefs.source.clone();
                    pending_source(ui.ctx(), &src);
                }
            }
        });
        if section(ui, "devices", "Devices", false) {
            let devices = lightcraft_engine::devices::devices();
            if devices.is_empty() {
                note(ui, "No camera or card");
            }
            for dev in devices {
                let on = d.prefs.source == dev.path;
                if source_row(ui, &format!("device:{}", dev.path), &dev.name, 0, on, None).clicked() {
                    if d.prefs.mode == "add" && can_copy(app) {
                        d.prefs.mode = "copy".into();
                    }
                    pending_source(ui.ctx(), &dev.path);
                }
            }
        }
        if section(ui, "files", "Files", false) {
            for (name, path) in roots() {
                folder_rows(ui, d, &name, &path, 0);
            }
        }
        ui.add_space(12.0);
    });
}

/// Ask for a new source (applied after the frame by [`take_pending_source`]).
fn pending_source(ctx: &egui::Context, path: &str) {
    ctx.data_mut(|m| m.insert_temp(egui::Id::new("import-pending-source"), path.to_string()));
}

/// A source chosen in the window this frame: read it.
pub fn take_pending_source(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(path) = ctx.data_mut(|m| m.remove_temp::<String>(egui::Id::new("import-pending-source"))) else { return };
    if let Err(e) = choose_source(app, &json!({"path": path})) {
        app.toast(ctx, e);
    }
}

fn folder_rows(ui: &mut egui::Ui, d: &mut ImportDialog, name: &str, path: &str, depth: usize) {
    if depth > 24 {
        return;
    }
    let key = egui::Id::new(("import-open", path));
    let open: bool = ui.data(|m| m.get_temp(key)).unwrap_or(false);
    let on = d.prefs.source == path;
    let resp = source_row(ui, &format!("folder:{path}"), name, depth, on, Some(open));
    if resp.clicked() {
        let arrow = resp.interact_pointer_pos().is_some_and(|p| p.x < resp.rect.left() + 20.0 + depth as f32 * 14.0);
        if arrow {
            ui.data_mut(|m| m.insert_temp(key, !open));
        } else {
            pending_source(ui.ctx(), path);
            ui.data_mut(|m| m.insert_temp(key, true));
        }
    }
    resp.on_hover_text(path);
    if open {
        // listed off the UI thread (a network share can be slow)
        match crate::panels::left::fs_cached(ui, "import-dirs", path, 30.0, list_dirs) {
            Some(children) => {
                for (n, p) in children {
                    folder_rows(ui, d, &n, &p, depth + 1);
                }
            }
            None => note(ui, "Reading…"),
        }
    }
}

fn source_row(ui: &mut egui::Ui, id: &str, label: &str, depth: usize, on: bool, open: Option<bool>) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    register(ui.ctx(), format!("importSource:{id}"), r);
    if on {
        ui.painter().rect_filled(r, 0.0, t.tool_active);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.7));
    }
    let x = r.left() + 10.0 + depth as f32 * 14.0;
    if let Some(open) = open {
        crate::icons::paint(
            ui.painter(),
            Rect::from_center_size(pos2(x + 5.0, r.center().y), vec2(9.0, 9.0)),
            if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight },
            t.text_dim,
        );
    }
    let icon = if open.is_some() { crate::icons::Icon::Folder } else { crate::icons::Icon::Photos };
    crate::icons::paint(ui.painter(), Rect::from_min_size(pos2(x + 14.0, r.center().y - 7.0), vec2(14.0, 14.0)), icon, t.icon);
    let clip = ui.painter().with_clip_rect(r.shrink2(vec2(4.0, 0.0)));
    clip.text(pos2(x + 34.0, r.center().y), Align2::LEFT_CENTER, label, t.font(12.0), if on { t.text } else { t.text_label });
    resp
}

fn note(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 18.0, r.center().y), Align2::LEFT_CENTER, crate::i18n::tr(text), t.font(11.5), t.text_dim);
}

fn grid_column(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, d: &mut ImportDialog) {
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(r, 0.0, t.canvas);
    // tabs: All Photos | New Photos | Destination Folders
    let tabs = Rect::from_min_size(r.min, vec2(r.width(), 32.0));
    let items = [("all", "All Photos"), ("new", "New Photos"), ("folders", "Destination Folders")];
    let mut x = tabs.center().x - 190.0;
    for (key, label) in items {
        let g = ui.painter().layout_no_wrap(crate::i18n::tr(label).to_string(), t.font(12.5), t.text);
        let tr = Rect::from_min_size(pos2(x, tabs.top() + 4.0), vec2(g.size().x + 20.0, 24.0));
        let resp = ui.interact(tr, egui::Id::new(("import-show", key)), Sense::click());
        register(ui.ctx(), format!("button:importShow-{key}"), tr);
        let on = d.show == key || (d.show.is_empty() && key == "all");
        let c = if on {
            t.text
        } else if resp.hovered() {
            t.text_label
        } else {
            t.text_dim
        };
        if on {
            ui.painter().rect_filled(tr, 3.0, t.tool_active);
        }
        ui.painter().galley(pos2(tr.left() + 10.0, tr.center().y - g.size().y / 2.0), g, c);
        if resp.clicked() {
            d.show = key.to_string();
        }
        x = tr.right() + 8.0;
    }
    // footer: Check All / Uncheck All, Sort, thumbnail size
    let foot = Rect::from_min_max(pos2(r.left(), r.bottom() - 36.0), r.max);
    ui.painter().rect_filled(foot, 0.0, t.chrome);
    let mut row =
        ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(12.0, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 8.0;
    if crate::widgets::text_button(&mut row, "importAll", "Check All", false).clicked() {
        for i in 0..d.candidates.len() {
            if let Some(c) = d.checked.get_mut(i) {
                *c = d.candidates.get(i).is_some_and(|c| {
                    c.error.is_none() && (c.duplicate.is_none() || d.trashed.get(i).copied().unwrap_or(false) && !d.on_deleted.is_empty())
                });
            }
        }
    }
    if crate::widgets::text_button(&mut row, "importNone", "Uncheck All", false).clicked() {
        d.checked.iter_mut().for_each(|c| *c = false);
    }
    row.add_space(8.0);
    row.label(egui::RichText::new(crate::i18n::tr("Sort:")).font(t.font(12.0)).color(t.text_dim));
    let sorts = [("time", "Capture Time"), ("checked", "Checked State"), ("name", "File Name"), ("type", "Media Type")];
    let cur = sorts.iter().find(|s| s.0 == d.sort).map_or(sorts[0].1, |s| s.1);
    let sr = crate::widgets::dropdown(&mut row, "importSort", crate::i18n::tr(cur), t.font(12.0), t.text);
    egui::Popup::menu(&sr).show(|ui| {
        for (k, label) in sorts {
            if ui.selectable_label(d.sort == k, crate::i18n::tr(label)).clicked() {
                d.sort = k.to_string();
            }
        }
    });
    row.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let resp = ui.add_sized([120.0, 20.0], egui::Slider::new(&mut d.thumb, 80.0..=240.0).show_value(false));
        register(ui.ctx(), "slider:importThumb", resp.rect);
        ui.label(egui::RichText::new(crate::i18n::tr("Thumbnails")).font(t.font(12.0)).color(t.text_dim));
    });
    // the photos
    let area = Rect::from_min_max(pos2(r.left(), tabs.bottom()), pos2(r.right(), foot.top()));
    let order = grid_order(d);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area.shrink(8.0)).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(area);
    if order.is_empty() {
        let (title, body) = if d.scanning || app.scan.is_some() {
            ("Looking for photos…", "")
        } else if d.prefs.source.is_empty() {
            ("Select a source", "Choose a card or a folder on the left")
        } else if d.candidates.is_empty() {
            ("No photos found", "Try Include Subfolders, or another folder")
        } else {
            ("No new photos", "Everything here is already in the library")
        };
        crate::panels::empty_message(&child, area, title, body);
        if let Some(task) = &app.scan {
            let s = task.status();
            let (done, total) = (s["done"].as_u64().unwrap_or(0), s["total"].as_u64().unwrap_or(0));
            if total > 0 {
                let label = crate::i18n::tr_format!("Reading photos… {done} of {total}", done = done, total = total);
                ui.painter().text(area.center() + vec2(0.0, 40.0), Align2::CENTER_CENTER, label, t.font(12.0), t.text_dim);
            }
        }
        return;
    }
    let cell = d.thumb.clamp(80.0, 240.0);
    let gap = 8.0;
    let caption = if d.show == "folders" { 34.0 } else { 20.0 };
    let w = child.available_width();
    let cols = ((w + gap) / (cell + gap)).floor().max(1.0) as usize;
    let rows = order.len().div_ceil(cols);
    egui::ScrollArea::vertical().id_salt("import-window-grid").auto_shrink([false, false]).show_viewport(&mut child, |ui, vp| {
        let (grid, _) = ui.allocate_exact_size(vec2(w, rows as f32 * (cell + caption + gap)), Sense::hover());
        for (k, &i) in order.iter().enumerate() {
            let (c, rr) = (k % cols, k / cols);
            let local = Rect::from_min_size(pos2(c as f32 * (cell + gap), rr as f32 * (cell + caption + gap)), vec2(cell, cell + caption));
            if !local.intersects(vp.expand(cell)) {
                continue;
            }
            cell_ui(app, ui, d, i, local.translate(grid.min.to_vec2()), caption);
        }
    });
}

fn cell_ui(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut ImportDialog, i: usize, rect: Rect, caption: f32) {
    let t = Tokens::get(ui.ctx());
    let Some(c) = d.candidates.get(i).cloned() else { return };
    let ok = d.importable(i);
    let on = d.checked.get(i).copied().unwrap_or(false) && ok;
    let img = Rect::from_min_max(rect.min, pos2(rect.right(), rect.bottom() - caption));
    let resp = ui.interact(img, egui::Id::new(("import-window-cell", i)), Sense::click());
    register(ui.ctx(), format!("import:{i}"), img);
    let p = ui.painter();
    p.rect_filled(img, 3.0, if on { Color32::from_gray(0x5a) } else { Color32::from_gray(0x34) });
    let slot = Slot::Import(i as u32);
    if !app.renderer.textures.contains_key(&slot)
        && let Some(job) = app.session.candidate_thumb_job(&c, 256, i as u64)
    {
        app.renderer.request_quick(slot, job, 4);
    }
    if let Some(tex) = app.renderer.textures.get(&slot) {
        let [tw, th] = tex.size;
        let s = ((img.width() - 12.0) / tw.max(1) as f32).min((img.height() - 12.0) / th.max(1) as f32);
        let fit = Rect::from_center_size(img.center(), vec2(tw as f32 * s, th as f32 * s));
        // already imported, or unreadable: dimmed (Classic)
        let tint = if ok { Color32::WHITE } else { Color32::from_gray(90) };
        p.image(tex.tex.id(), fit, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
    // checkbox (top-left, as in Classic)
    let cb = Rect::from_min_size(img.min + vec2(6.0, 6.0), vec2(15.0, 15.0));
    p.rect(cb, 2.0, if on { t.text_label } else { Color32::from_black_alpha(150) }, Stroke::new(1.0, Color32::from_gray(190)), StrokeKind::Inside);
    if on {
        p.line_segment([cb.left_center() + vec2(3.5, 0.5), cb.center_bottom() + vec2(-1.0, -4.0)], Stroke::new(2.0, t.chrome));
        p.line_segment([cb.center_bottom() + vec2(-1.0, -4.0), cb.right_top() + vec2(-3.5, 4.0)], Stroke::new(2.0, t.chrome));
    }
    let badge = match (&c.duplicate, &c.error) {
        (Some(_), _) if d.trashed.get(i).copied().unwrap_or(false) => Some("In Recently Deleted"),
        (Some(_), _) => Some("Already imported"),
        (None, Some(_)) => Some("Unreadable"),
        _ => None,
    };
    if let Some(b) = badge {
        let g = p.layout_no_wrap(crate::i18n::tr(b).to_string(), t.semibold(9.5), Color32::WHITE);
        let br = Rect::from_min_size(pos2(img.right() - g.size().x - 14.0, img.bottom() - g.size().y - 10.0), g.size() + vec2(8.0, 4.0));
        p.rect_filled(br, 3.0, Color32::from_black_alpha(190));
        p.galley(br.min + vec2(4.0, 2.0), g, Color32::from_gray(220));
    }
    let name = if c.name.chars().count() > 22 { format!("{}…", c.name.chars().take(21).collect::<String>()) } else { c.name.clone() };
    let clip = p.with_clip_rect(rect);
    clip.text(pos2(rect.left() + 2.0, img.bottom() + 9.0), Align2::LEFT_CENTER, name, t.font(10.5), if ok { t.text_label } else { t.text_disabled });
    if d.show == "folders" {
        let folder = d.prefs.folder_for(c.captured.as_deref(), &(app.session.clock)());
        let shown = if folder.is_empty() { crate::i18n::tr("(destination)").to_string() } else { folder };
        clip.text(pos2(rect.left() + 2.0, img.bottom() + 24.0), Align2::LEFT_CENTER, shown, t.font(10.0), t.text_dim);
    }
    let tip = format!(
        "{}\n{} × {} · {} · {:.1} MB{}",
        c.path,
        c.width,
        c.height,
        c.format,
        c.file_size as f64 / 1e6,
        c.captured.as_deref().map(|d| format!("\n{}", d.replace('T', " "))).unwrap_or_default()
    );
    let resp = resp.on_hover_text(tip);
    if resp.clicked() && ok {
        let shift = ui.input(|input| input.modifiers.shift);
        d.click(i, shift);
    }
}

fn options_column(app: &mut LightcraftApp, ui: &mut egui::Ui, r: Rect, d: &mut ImportDialog) {
    let t = Tokens::get(ui.ctx());
    let mut col = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
    col.set_clip_rect(r);
    egui::ScrollArea::vertical().id_salt("import-options").auto_shrink([false, false]).show(&mut col, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        ui.set_width(r.width());
        let pad = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            egui::Frame::NONE.inner_margin(egui::Margin { left: 14, right: 14, top: 8, bottom: 10 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                add(ui)
            });
        };
        if section(ui, "fileHandling", "File Handling", true) {
            pad(ui, &mut |ui| {
                ui.label(egui::RichText::new(crate::i18n::tr("Build Previews")).font(t.font(12.0)).color(t.text_label));
                let opts = [("minimal", "Minimal"), ("embedded", "Embedded & Sidecar"), ("standard", "Standard"), ("full", "1:1")];
                let cur = opts.iter().find(|o| o.0 == d.prefs.previews).map_or(opts[0].1, |o| o.1);
                let cb = egui::ComboBox::from_id_salt("import-previews").width(ui.available_width()).selected_text(crate::i18n::tr(cur)).show_ui(
                    ui,
                    |ui| {
                        for (k, label) in opts {
                            if ui.selectable_label(d.prefs.previews == k, crate::i18n::tr(label)).clicked() {
                                d.prefs.previews = k.to_string();
                            }
                        }
                    },
                );
                register(ui.ctx(), "combo:importPreviews", cb.response.rect);
                let smart_ok = app.session.media.smart_dir.is_some();
                let r = ui.checkbox(&mut d.prefs.smart_previews, crate::i18n::tr("Build Smart Previews"));
                register(ui.ctx(), "check:importSmart", r.rect);
                if !smart_ok && d.prefs.smart_previews {
                    ui.label(egui::RichText::new(crate::i18n::tr("This library keeps no smart previews.")).small().color(t.caution));
                }
                ui.label(
                    egui::RichText::new(crate::i18n::tr("Suspected duplicates are never imported twice: they show dimmed and unchecked."))
                        .small()
                        .color(t.text_dim),
                );
                let mut second = !d.prefs.second_copy.is_empty();
                let r = ui.checkbox(&mut second, crate::i18n::tr("Make a Second Copy To:"));
                register(ui.ctx(), "check:importSecondCopy", r.rect);
                if r.changed() {
                    if second {
                        match app.services.pick_folder.as_mut().and_then(|f| f()) {
                            Some(f) => d.prefs.second_copy = f,
                            None => d.prefs.second_copy.clear(),
                        }
                    } else {
                        d.prefs.second_copy.clear();
                    }
                }
                if second {
                    ui.label(egui::RichText::new(&d.prefs.second_copy).small().color(t.text_label));
                }
                album_choice(app, ui, d);
            });
        }
        if d.prefs.copies() && section(ui, "renaming", "File Renaming", true) {
            pad(ui, &mut |ui| {
                let mut on = !d.prefs.rename.is_empty();
                if ui.checkbox(&mut on, crate::i18n::tr("Rename Files")).changed() {
                    d.prefs.rename = if on { "{date}_{seq:4}".into() } else { String::new() };
                }
                if on {
                    let id = egui::Id::new("import-window-rename");
                    let r = ui.add(egui::TextEdit::singleline(&mut d.prefs.rename).id(id).desired_width(f32::INFINITY));
                    register(ui.ctx(), "field:importRename", r.rect);
                    crate::import::unknown_tags_warning(ui, &d.prefs.rename);
                    if crate::import::tag_toggle(ui, "importWindowRename") {
                        crate::import::tag_help(ui, "importWindowRename", &mut d.prefs.rename, id);
                    }
                }
            });
        }
        if section(ui, "apply", "Apply During Import", true) {
            pad(ui, &mut |ui| {
                ui.label(egui::RichText::new(crate::i18n::tr("Develop Settings")).font(t.font(12.0)).color(t.text_label));
                let cur = app
                    .session
                    .presets
                    .iter()
                    .find(|p| p.id == d.prefs.preset)
                    .map(|p| crate::i18n::builtin_label(&p.name, p.builtin).to_string())
                    .unwrap_or_else(|| crate::i18n::tr("None").into());
                egui::ComboBox::from_id_salt("import-window-preset").width(ui.available_width()).height(320.0).selected_text(cur).show_ui(ui, |ui| {
                    if ui.selectable_label(d.prefs.preset.is_empty(), crate::i18n::tr("None")).clicked() {
                        d.prefs.preset.clear();
                    }
                    let mut groups: Vec<String> = app.session.presets.iter().map(|p| p.group.clone()).collect();
                    groups.sort();
                    groups.dedup();
                    for g in groups {
                        ui.menu_button(g.clone(), |ui| {
                            for p in app.session.presets.iter().filter(|p| p.group == g) {
                                if ui.selectable_label(d.prefs.preset == p.id, crate::i18n::builtin_label(&p.name, p.builtin)).clicked() {
                                    d.prefs.preset = p.id.clone();
                                }
                            }
                        });
                    }
                });
                ui.label(egui::RichText::new(crate::i18n::tr("Metadata")).font(t.font(12.0)).color(t.text_label));
                let cur = if d.prefs.metadata_preset.is_empty() { crate::i18n::tr("None").to_string() } else { d.prefs.metadata_preset.clone() };
                egui::ComboBox::from_id_salt("import-window-metadata").width(ui.available_width()).selected_text(cur).show_ui(ui, |ui| {
                    if ui.selectable_label(d.prefs.metadata_preset.is_empty(), crate::i18n::tr("None")).clicked() {
                        d.prefs.metadata_preset.clear();
                    }
                    for m in &app.session.metadata_presets {
                        if ui.selectable_label(d.prefs.metadata_preset == m.name, &m.name).clicked() {
                            d.prefs.metadata_preset = m.name.clone();
                        }
                    }
                });
                ui.label(egui::RichText::new(crate::i18n::tr("Keywords")).font(t.font(12.0)).color(t.text_label));
                let r = ui.add(
                    egui::TextEdit::multiline(&mut d.prefs.keywords)
                        .hint_text(crate::i18n::tr("comma, separated"))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                register(ui.ctx(), "field:importKeywords", r.rect);
            });
        }
        if section(ui, "destination", "Destination", true) {
            pad(ui, &mut |ui| {
                if !d.prefs.copies() {
                    ui.label(egui::RichText::new(crate::i18n::tr("Add leaves the photos where they are.")).color(t.text_dim));
                    return;
                }
                let mut into = d.prefs.into_subfolder;
                let r = ui.checkbox(&mut into, crate::i18n::tr("Into Subfolder"));
                register(ui.ctx(), "check:importIntoSubfolder", r.rect);
                d.prefs.into_subfolder = into;
                if into {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut d.prefs.subfolder).hint_text(crate::i18n::tr("Folder name")).desired_width(f32::INFINITY),
                    );
                    register(ui.ctx(), "field:importSubfolder", r.rect);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.label(egui::RichText::new(crate::i18n::tr("Organize")).font(t.font(12.0)).color(t.text_label));
                    let cur = if d.prefs.organize == "flat" { "Into one folder" } else { "By date" };
                    egui::ComboBox::from_id_salt("import-window-organize").selected_text(crate::i18n::tr(cur)).show_ui(ui, |ui| {
                        for (k, label) in [("date", "By date"), ("flat", "Into one folder")] {
                            if ui.selectable_label(d.prefs.organize == k, crate::i18n::tr(label)).clicked() {
                                d.prefs.organize = k.to_string();
                            }
                        }
                    });
                });
                if d.prefs.organize != "flat" {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(egui::RichText::new(crate::i18n::tr("Date Format")).font(t.font(12.0)).color(t.text_label));
                        let cur = DATE_FORMATS.get(d.prefs.date_format).unwrap_or(&DATE_FORMATS[0]).0;
                        egui::ComboBox::from_id_salt("import-window-date").selected_text(cur).show_ui(ui, |ui| {
                            for (i, (example, _)) in DATE_FORMATS.iter().enumerate() {
                                if ui.selectable_label(d.prefs.date_format == i, *example).clicked() {
                                    d.prefs.date_format = i;
                                }
                            }
                        });
                    });
                }
                let first = d.candidates.iter().find_map(|c| c.captured.clone());
                let folder = d.prefs.folder_for(first.as_deref(), &(app.session.clock)());
                let base = if d.prefs.destination.trim().is_empty() { crate::i18n::tr("Originals").to_string() } else { d.prefs.destination.clone() };
                let example = if folder.is_empty() { base } else { format!("{base}/{folder}") };
                let r = ui.label(egui::RichText::new(example).small().color(t.text_dim));
                register(ui.ctx(), "label:importExample", r.rect);
            });
        }
        ui.add_space(16.0);
    });
}

fn album_choice(app: &mut LightcraftApp, ui: &mut egui::Ui, d: &mut ImportDialog) {
    let mut on = d.album.is_some() || !d.new_album.is_empty();
    let r = ui.checkbox(&mut on, crate::i18n::tr("Add to Collection"));
    register(ui.ctx(), "check:importCollection", r.rect);
    if r.changed() {
        if on {
            d.new_album = crate::i18n::tr("Imported Photos").into();
        } else {
            d.album = None;
            d.new_album.clear();
        }
    }
    if !on {
        return;
    }
    let mut albums: Vec<(u64, String)> =
        app.session.catalog.albums().filter(|a| !a.folder && !a.is_smart()).map(|a| (a.id.0, a.name.clone())).collect();
    albums.sort_by_key(|(_, n)| n.to_lowercase());
    let cur = match d.album {
        Some(a) => albums.iter().find(|x| x.0 == a).map(|x| x.1.clone()).unwrap_or_default(),
        None => crate::i18n::tr("New collection").into(),
    };
    egui::ComboBox::from_id_salt("import-window-album").width(ui.available_width()).selected_text(cur).show_ui(ui, |ui| {
        if ui.selectable_label(d.album.is_none(), crate::i18n::tr("New collection…")).clicked() {
            d.album = None;
            if d.new_album.is_empty() {
                d.new_album = crate::i18n::tr("Imported Photos").into();
            }
        }
        for (id, name) in &albums {
            if ui.selectable_label(d.album == Some(*id), name).clicked() {
                d.album = Some(*id);
                d.new_album.clear();
            }
        }
    });
    if d.album.is_none() {
        let r = ui.add(egui::TextEdit::singleline(&mut d.new_album).desired_width(f32::INFINITY));
        register(ui.ctx(), "field:importAlbumName", r.rect);
    }
}

/// The bottom bar: the count and size of the checked photos, Cancel and Import. Returns
/// (cancel, import) clicked.
fn bottom_bar(ui: &mut egui::Ui, r: Rect, d: &ImportDialog) -> (bool, bool) {
    let t = Tokens::get(ui.ctx());
    ui.painter().text(pos2(r.left() + 16.0, r.center().y), Align2::LEFT_CENTER, checked_summary(d), t.font(12.5), t.text_label);
    let n = d.selected_paths().len();
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(16.0, 0.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    row.spacing_mut().item_spacing.x = 10.0;
    let verb = if d.prefs.mode == "move" { "Move" } else { "Import" };
    let ir = crate::panels::right::wide_button(&mut row, "dialogOk", verb, 110.0);
    let import = ir.clicked() && n > 0;
    if ir.clicked() && n == 0 {
        crate::widgets::register(ui.ctx(), "note:importNothing", r);
    }
    let cancel = crate::panels::right::wide_button(&mut row, "dialogCancel", "Cancel", 110.0).clicked();
    (cancel, import)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightcraft_engine::import::ImportCandidate;

    fn cand(name: &str, captured: &str, dup: bool) -> ImportCandidate {
        ImportCandidate {
            path: format!("/card/{name}"),
            name: name.into(),
            captured: Some(captured.into()),
            duplicate: dup.then(|| "hash".to_string()),
            file_size: 2_000_000,
            ..Default::default()
        }
    }

    #[test]
    fn organize_param_follows_subfolder_and_date_format() {
        let mut p = ImportPrefs::default();
        assert_eq!(p.organize_param(), "{date:%Y}/{date:%Y-%m-%d}");
        p.date_format = 1;
        assert_eq!(p.organize_param(), "{date:%Y}/{date:%m}/{date:%d}");
        p.into_subfolder = true;
        p.subfolder = "Smith / Wedding".into();
        assert_eq!(p.organize_param(), "Smith - Wedding/{date:%Y}/{date:%m}/{date:%d}");
        p.organize = "flat".into();
        assert_eq!(p.organize_param(), "Smith - Wedding/");
        p.into_subfolder = false;
        assert_eq!(p.organize_param(), "flat");
        // a stale index from a damaged ui.json falls back to the first format
        p.organize = "date".into();
        p.date_format = 99;
        assert_eq!(p.organize_param(), DATE_FORMATS[0].1);
    }

    #[test]
    fn destination_folder_of_a_photo() {
        let mut p = ImportPrefs { date_format: 3, ..Default::default() };
        assert_eq!(p.folder_for(Some("2026-10-08T12:00:00"), ""), "2026-10-08");
        assert_eq!(p.folder_for(None, ""), "unknown-unknown-unknown");
        assert_eq!(p.folder_for(None, "2026-10-09T08:00:00"), "2026-10-09", "no capture time: the import's date");
        p.date_format = 0;
        p.into_subfolder = true;
        p.subfolder = "GR3".into();
        assert_eq!(p.folder_for(Some("2026-10-08T12:00:00"), ""), "GR3/2026/2026-10-08");
        // never panics on short or odd dates
        assert_eq!(p.folder_for(Some("20"), ""), "GR3/unknown/unknown-unknown-unknown");
        assert_eq!(p.folder_for(Some("年月日"), ""), "GR3/unknown/unknown-unknown-unknown", "not a char boundary: no panic");
    }

    #[test]
    fn new_photos_hide_duplicates_and_summary_counts_checked() {
        let mut d =
            ImportDialog::new(vec![cand("b.jpg", "2026-10-02", false), cand("a.jpg", "2026-10-01", true), cand("c.jpg", "2026-10-03", false)]);
        d.sort = "name".into();
        assert_eq!(grid_order(&d), vec![1, 0, 2]);
        d.show = "new".into();
        assert_eq!(grid_order(&d), vec![0, 2]);
        d.sort = "time".into();
        assert_eq!(grid_order(&d), vec![0, 2]);
        // the duplicate starts unchecked: 2 photos, 4 MB
        assert_eq!(checked_summary(&d), "2 photos / 4 MB");
    }

    #[test]
    fn modes_are_classic_and_from_prefs_maps_them() {
        assert_eq!(MODES.map(|m| m.1), ["Copy as DNG", "Copy", "Move", "Add"]);
        let mut d = ImportDialog { window: true, ..Default::default() };
        d.prefs.mode = "dng".into();
        let w = crate::import::from_prefs(&d);
        assert!(w.copy && w.dng && !w.move_files);
        d.prefs.mode = "move".into();
        let w = crate::import::from_prefs(&d);
        assert!(w.copy && w.move_files);
        d.prefs.mode = "add".into();
        assert!(!crate::import::from_prefs(&d).copy);
    }
}
