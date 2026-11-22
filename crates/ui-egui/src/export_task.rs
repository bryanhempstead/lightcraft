//! Background export: the Export dialog, File → Export with Preset and Export with Previous hand
//! their batch to a worker thread so the window stays responsive. Photos are prepared on the UI
//! thread ([`lightcraft_engine::export::prepare_export`]: cheap, needs the session) and rendered,
//! encoded and written on the worker. Progress shows in the top bar's activity strip (where
//! Lightroom Classic shows it), with pause. / resume. and a cancel ×. An export started while
//! one runs waits in a queue (LightCraft) and starts when the one before it is done.
//!
//! Needs [`crate::Services::write_shared`] (a thread-safe writer); without it (web) the batch runs
//! synchronously as before.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use lightcraft_engine::export::{Destination, ExportOptions, ExportService, ExportTo, PreparedExport, run_batch};
use serde_json::{Value, json};

use crate::LightcraftApp;

pub struct ExportTask {
    pub total: usize,
    /// Photos started so far and the file in progress.
    pub progress: Arc<Mutex<(usize, String)>>,
    pub cancel: Arc<AtomicBool>,
    /// Paused between photos (the one being written finishes first).
    pub paused: Arc<AtomicBool>,
    rx: Receiver<Result<Vec<Value>, String>>,
    /// The batch's options (Add to This Catalog runs here once the files are written).
    opts: ExportOptions,
}

/// An export waiting for the running one to finish.
pub struct Queued {
    items: Vec<PreparedExport>,
    opts: ExportOptions,
    to: Destination,
}

impl Queued {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl ExportTask {
    /// `{total, done, current, paused}` for `ui.inspect`.
    pub fn status(&self) -> Value {
        let (done, current) = self.progress.lock().map(|g| g.clone()).unwrap_or_default();
        json!({"total": self.total, "done": done, "current": current, "paused": self.paused.load(Ordering::Relaxed)})
    }

    /// `(done, total, current file)`.
    pub fn counts(&self) -> (usize, usize, String) {
        let (done, current) = self.progress.lock().map(|g| g.clone()).unwrap_or_default();
        (done, self.total, current)
    }
}

/// The Export dialog's export. button: several checked presets, Email, Apple Photos, or the
/// files to a folder (Choose folder later asks for it now; Existing Files ▸ Ask what to do asks
/// about the files already there first, in the Existing File dialog).
pub fn start_from_dialog(app: &mut LightcraftApp, p: &mut Value, dir: &str, checked: &[String], album: &str) -> Result<Value, String> {
    let o = ExportOptions::from_json(p);
    if !checked.is_empty() {
        let mut q = json!({"presets": checked});
        if !dir.trim().is_empty() {
            q["dir"] = json!(dir.trim());
        }
        let r = app.run("app.export", q)?;
        let n = r["presets"].as_array().map_or(0, Vec::len);
        app.toast_soon(format!("Exported with {n} presets"), 4.0);
        return Ok(r);
    }
    match o.service {
        ExportService::Email => return app.run("file.emailPhotos", json!({"settings": p})),
        ExportService::Photos => {
            let mut q = p.clone();
            if let Some(m) = q.as_object_mut() {
                m.remove("service");
            }
            q["album"] = json!(album.trim());
            return app.run("dialog.exportToPhotos", q);
        }
        ExportService::HardDrive | ExportService::Disc => {}
    }
    let folder = match o.export_to {
        ExportTo::ChooseLater => match app.services.pick_folder.as_mut().and_then(|f| f()) {
            Some(d) => d,
            None if !dir.trim().is_empty() && app.services.pick_folder.is_none() => dir.trim().to_string(),
            None => return Err("Choose a folder to export to".into()),
        },
        ExportTo::SpecificFolder | ExportTo::SameFolder => dir.trim().to_string(),
        t => t.resolve(dir),
    };
    p["dir"] = json!(folder);
    p["background"] = json!(true);
    if o.conflict == lightcraft_engine::export::Conflict::Ask {
        let found = app.run("export.existingFiles", p.clone())?;
        let existing: Vec<(u64, String)> =
            found.as_array().into_iter().flatten().filter_map(|e| Some((e["photo"].as_u64()?, e["path"].as_str()?.to_string()))).collect();
        if !existing.is_empty() {
            let n = existing.len();
            app.ui.dialog =
                Some(crate::state::Dialog::ExportExisting { params: p.clone(), existing, at: 0, answers: serde_json::Map::new(), apply_all: false });
            return Ok(json!({"asking": n}));
        }
    }
    app.run("app.export", p.clone())
}

/// Start exporting `items` in the background (or queue them behind the running export). Errors
/// per photo are collected, not fatal.
pub fn start(app: &mut LightcraftApp, items: Vec<PreparedExport>, opts: ExportOptions, to: Destination) -> Result<Value, String> {
    if app.services.write_shared.is_none() {
        return Err("no background writer".into());
    }
    if app.export.is_some() {
        let n = items.len();
        app.export_queue.push(Queued { items, opts, to });
        return Ok(json!({"background": true, "queued": app.export_queue.len(), "total": n}));
    }
    launch(app, Queued { items, opts, to })
}

fn launch(app: &mut LightcraftApp, q: Queued) -> Result<Value, String> {
    let write = app.services.write_shared.clone().ok_or("no background writer")?;
    let Queued { items, opts, to } = q;
    let total = items.len();
    let progress = Arc::new(Mutex::new((0, String::new())));
    let cancel = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let (p, c, z) = (progress.clone(), cancel.clone(), paused.clone());
    let task_opts = opts.clone();
    let work = move || {
        let mut w = |path: &str, bytes: &[u8]| write(path, bytes);
        let mut r = run_batch(items, &opts, &to, &mut w, &|path| std::path::Path::new(path).exists(), false, &mut |done, name| {
            if let Ok(mut g) = p.lock() {
                *g = (done, name.to_string());
            }
            // pause. holds the next photo (cancel still stops it)
            #[cfg(not(target_arch = "wasm32"))]
            while z.load(Ordering::Relaxed) && !c.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            !c.load(Ordering::Relaxed)
        });
        // Post-Processing (Show in Finder, open in an app, an Export Action), not after a cancel
        if let Ok(files) = &mut r
            && !c.load(Ordering::Relaxed)
            && let Some(after) = lightcraft_engine::export::post_process(&opts, files)
        {
            files.push(json!({"postProcessing": after}));
        }
        let _ = tx.send(r);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("export".into()).spawn(work).map_err(|e| e.to_string())?;
    #[cfg(target_arch = "wasm32")]
    work();
    app.export = Some(ExportTask { total, progress, cancel, paused, rx, opts: task_opts });
    Ok(json!({"background": true, "total": total}))
}

/// pause. / resume. the running export.
pub fn toggle_pause(app: &mut LightcraftApp) -> bool {
    match &app.export {
        Some(t) => {
            let now = !t.paused.load(Ordering::Relaxed);
            t.paused.store(now, Ordering::Relaxed);
            now
        }
        None => false,
    }
}

/// Cancel the running export (and drop the queued ones).
pub fn cancel_all(app: &mut LightcraftApp) {
    app.export_queue.clear();
    if let Some(t) = &app.export {
        t.cancel.store(true, Ordering::Relaxed);
    }
}

/// Per frame: when the batch finishes, report it and start the next queued one.
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(task) = &app.export else { return };
    match task.rx.try_recv() {
        Ok(r) => {
            let cancelled = task.cancel.load(Ordering::Relaxed);
            let total = task.total;
            let task_opts = task.opts.clone();
            app.export = None;
            let msg = match r {
                Ok(files) => {
                    let ok = files.iter().filter(|f| f.get("path").is_some()).count();
                    let failed: Vec<&Value> = files.iter().filter(|f| f.get("error").is_some()).collect();
                    let skipped = files.iter().filter(|f| f.get("skipped").is_some()).count();
                    let mut m =
                        crate::i18n::tr_format!("Exported {ok} of {total} photo{}", if total == 1 { "" } else { "s" }, ok = ok, total = total);
                    if skipped > 0 {
                        m += &crate::i18n::tr_format!(" · {skipped} skipped (file exists)", skipped = skipped);
                    }
                    if let Some(first) = failed.first() {
                        m += &crate::i18n::tr_format!(" · {} failed: {}", failed.len(), first["error"].as_str().unwrap_or("error"));
                    }
                    if cancelled {
                        m += " · cancelled";
                    }
                    app.session.record_exports(&files);
                    if let Some(added) = lightcraft_engine::export::add_to_catalog(&mut app.session, &task_opts, &files) {
                        log::info!("export: added to the catalog: {added}");
                    }
                    app.last_export_result = Some(json!({"files": files, "cancelled": cancelled}));
                    m
                }
                Err(e) => e,
            };
            app.toast(ctx, msg);
            if !app.export_queue.is_empty() {
                let next = app.export_queue.remove(0);
                if let Err(e) = launch(app, next) {
                    app.toast(ctx, e);
                }
            }
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.export = None;
            app.toast(ctx, crate::i18n::tr("Export stopped unexpectedly"));
        }
    }
}
