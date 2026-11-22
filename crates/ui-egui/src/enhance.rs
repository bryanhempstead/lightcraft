//! Photo ▸ Enhance… in the UI (Lightroom Classic's Enhance dialog): Denoise with its Amount, Raw Details, Super
//! Resolution; a before / after preview of the middle of the photo; where the result goes (a new DNG stacked on
//! the original, or in place); the models' downloads with their licences.
//!
//! The preview and the Enhance run [`lightcraft_engine::enhance::EnhanceJob`]s on worker threads with progress and
//! cancellation, so the window never waits. The Enhance keeps running after the dialog closes (progress in a
//! toast) and is finished on the session ([`lightcraft_engine::Session::finish_enhance`]). In-place results that
//! are missing for the photo being edited (a new Denoise amount, cleared caches) are made again in the background.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use lightcraft_catalog::PhotoId;
use lightcraft_engine::enhance::{EnhanceJob, EnhanceOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;

/// The dialog's options (also `photo.enhance`'s parameters).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EnhanceDialog {
    pub denoise: bool,
    pub amount: f64,
    pub raw_details: bool,
    pub super_resolution: bool,
    /// Keep the result in the photo (in place) instead of a new DNG.
    pub in_place: bool,
    /// New DNG: stack it with the original.
    pub stack: bool,
}

impl Default for EnhanceDialog {
    fn default() -> Self {
        EnhanceDialog { denoise: true, amount: 50.0, raw_details: false, super_resolution: false, in_place: false, stack: true }
    }
}

impl EnhanceDialog {
    pub fn options(&self) -> lightcraft_engine::enhance::Options {
        lightcraft_engine::enhance::Options {
            denoise: if self.denoise { self.amount.clamp(1.0, 100.0) as f32 } else { 0.0 },
            raw_details: self.raw_details,
            super_resolution: self.super_resolution,
        }
    }
    pub fn params(&self) -> Value {
        json!({
            "denoise": if self.denoise { self.amount } else { 0.0 }, "rawDetails": self.raw_details,
            "superResolution": self.super_resolution, "inPlace": self.in_place, "stack": self.stack,
        })
    }
}

pub struct EnhanceTask {
    pub job: EnhanceJob,
    pub options: EnhanceDialog,
    pub progress: Arc<Mutex<(f32, String)>>,
    pub cancel: Arc<AtomicBool>,
    pub rx: Receiver<Result<EnhanceOutput, String>>,
    /// An automatic in-place remake (no toast when it ends).
    pub quiet: bool,
}

pub struct EnhanceState {
    pub ids: Vec<PhotoId>,
    pub preview_task: Option<EnhanceTask>,
    pub final_task: Option<EnhanceTask>,
    /// Before / after textures and the options and stats they show.
    pub preview: Option<(egui::TextureHandle, egui::TextureHandle, [usize; 2], EnhanceDialog, Value)>,
    pub error: Option<String>,
    pub failed_options: Option<EnhanceDialog>,
    pub last: Option<EnhanceDialog>,
    pub last_result: Option<Value>,
    /// The in-place result last remade automatically (not retried while it fails).
    pub remade: Option<(PhotoId, String)>,
    /// Lens Blur ▸ Visualize Depth.
    pub visualize_depth: bool,
    /// Lens Blur ▸ Refine brush size (‰ of the long edge) and the stroke being painted.
    pub brush_size: f64,
    pub lens_stroke: Vec<[f64; 2]>,
}

impl Default for EnhanceState {
    fn default() -> Self {
        EnhanceState {
            ids: Vec::new(),
            preview_task: None,
            final_task: None,
            preview: None,
            error: None,
            failed_options: None,
            last: None,
            last_result: None,
            remade: None,
            visualize_depth: false,
            brush_size: 30.0,
            lens_stroke: Vec::new(),
        }
    }
}

impl EnhanceState {
    /// An automatic in-place remake is running.
    pub fn remaking(&self) -> bool {
        self.final_task.as_ref().is_some_and(|t| t.quiet)
    }
}

const PREVIEW_SIDE: usize = 360;

/// What the preview shows: the middle of the photo at 1:1 (`PREVIEW_SIDE` sensor pixels), as Lightroom's Enhance
/// preview does.
fn preview_rect(app: &LightcraftApp, id: Option<PhotoId>) -> [f64; 4] {
    let (w, h) = id.and_then(|i| app.session.catalog.photo(i)).map_or((0, 0), |p| (p.width, p.height));
    let half = |n: u32| if n as usize > PREVIEW_SIDE { PREVIEW_SIDE as f64 / 2.0 / n as f64 } else { 0.5 };
    // (the catalog's size is oriented; the crop is on the sensor: a square window either way)
    let (hx, hy) = (half(w.max(h)), half(w.max(h)) * w.max(h) as f64 / w.min(h).max(1) as f64);
    [0.5 - hx, 0.5 - hy.min(0.5), 0.5 + hx, 0.5 + hy.min(0.5)]
}

fn spawn(app: &LightcraftApp, options: &EnhanceDialog, ids: &[PhotoId], preview: bool, quiet: bool) -> Result<EnhanceTask, String> {
    let job = app
        .session
        .plan_enhance(
            options.options(),
            ids,
            options.in_place && !preview,
            options.stack,
            preview.then(|| (preview_rect(app, ids.first().copied()), PREVIEW_SIDE)),
        )
        .map_err(|e| e.to_string())?;
    let progress = Arc::new(Mutex::new((0.0, "Starting".to_string())));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = channel();
    let (j, p, c) = (job.clone(), progress.clone(), cancel.clone());
    let work = move || {
        let run = || {
            j.run(&|f, stage| {
                if let Ok(mut g) = p.lock() {
                    *g = (f, stage.to_string());
                }
                !c.load(Ordering::Relaxed)
            })
        };
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("Enhance failed unexpectedly".into()));
        let _ = tx.send(r);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("enhance".into()).spawn(work).map_err(|e| e.to_string())?;
    #[cfg(target_arch = "wasm32")]
    work();
    Ok(EnhanceTask { job, options: options.clone(), progress, cancel, rx, quiet })
}

/// Open the Enhance dialog for the selection.
pub fn open(app: &mut LightcraftApp) -> Result<Value, String> {
    if !lightcraft_engine::enhance::AVAILABLE {
        return Err("Enhance is not available in this build".into());
    }
    let ids = app.session.targets(&json!({}));
    if ids.is_empty() {
        return Err("select a raw photo to enhance".into());
    }
    app.enhance.ids = ids;
    app.enhance.error = None;
    app.enhance.preview = None;
    app.enhance.failed_options = None;
    let opts = app.enhance.last.clone().unwrap_or_default();
    app.ui.dialog = Some(crate::state::Dialog::Enhance { opts });
    Ok(json!({"photos": app.enhance.ids.len()}))
}

/// Start the Enhance (the dialog's enh. button / `ui.dialog.confirm`).
pub fn start_final(app: &mut LightcraftApp, opts: &EnhanceDialog) -> Result<Value, String> {
    if app.enhance.final_task.as_ref().is_some_and(|t| !t.quiet) {
        return Err("an Enhance is already running".into());
    }
    if let Some(t) = app.enhance.preview_task.take() {
        t.cancel.store(true, Ordering::Relaxed);
    }
    if let Some(t) = app.enhance.final_task.take() {
        t.cancel.store(true, Ordering::Relaxed);
    }
    let ids = app.enhance.ids.clone();
    let task = spawn(app, opts, &ids, false, false)?;
    app.enhance.final_task = Some(task);
    app.enhance.last = Some(opts.clone());
    Ok(json!({"started": true, "photos": ids.len()}))
}

/// The active photo's in-place result is missing (a new Denoise amount, cleared caches): make it in the
/// background.
fn remake_in_place(app: &mut LightcraftApp) {
    if !lightcraft_engine::enhance::AVAILABLE || app.enhance.final_task.is_some() {
        return;
    }
    let Some(id) = app.session.active() else { return };
    let Some(p) = app.session.catalog.photo(id) else { return };
    let o = lightcraft_engine::enhance::options_of(&p.develop);
    let Some(e) = lightcraft_engine::enhance::enhanced_source(p) else { return };
    if app.enhance.remade.as_ref().is_some_and(|(i, k)| *i == id && *k == e.key) || e.product.is_file() {
        return;
    }
    app.enhance.remade = Some((id, e.key.clone()));
    let opts = EnhanceDialog {
        denoise: o.denoise > 0.0,
        amount: f64::from(o.denoise.max(1.0)),
        raw_details: o.raw_details,
        super_resolution: o.super_resolution,
        in_place: true,
        stack: false,
    };
    match spawn(app, &opts, &[id], false, true) {
        Ok(t) => app.enhance.final_task = Some(t),
        Err(e) => app.ui.status = e,
    }
}

/// Per frame: keep the preview in sync with the dialog's options, collect finished jobs.
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let p3 = app.renderer.display_p3;
    lightcraft_engine::enhance::warm_up_in_background(&app.session.enhancer.models_root);
    let dialog_opts = match &app.ui.dialog {
        Some(crate::state::Dialog::Enhance { opts }) => Some(opts.clone()),
        _ => None,
    };
    match &dialog_opts {
        Some(opts) => {
            // the preview doesn't depend on where the result goes
            let key = |o: &EnhanceDialog| EnhanceDialog { in_place: false, stack: false, ..o.clone() };
            let current = app.enhance.preview_task.as_ref().map(|t| key(&t.options)).or(app.enhance.preview.as_ref().map(|p| key(&p.3)));
            let failed = app.enhance.failed_options.as_ref().map(key) == Some(key(opts));
            if current != Some(key(opts)) && !failed && opts.options().any() {
                if let Some(t) = app.enhance.preview_task.take() {
                    t.cancel.store(true, Ordering::Relaxed);
                }
                let first: Vec<PhotoId> = app.enhance.ids.first().copied().into_iter().collect();
                match spawn(app, opts, &first, true, false) {
                    Ok(t) => {
                        app.enhance.error = None;
                        app.enhance.preview_task = Some(t);
                    }
                    Err(e) => {
                        app.enhance.error = Some(e);
                        app.enhance.failed_options = Some(opts.clone());
                    }
                }
            }
        }
        None => {
            if let Some(t) = app.enhance.preview_task.take() {
                t.cancel.store(true, Ordering::Relaxed);
            }
            remake_in_place(app);
        }
    }
    if let Some(t) = &app.enhance.preview_task {
        match t.rx.try_recv() {
            Ok(Ok(out)) => {
                let Some(t) = app.enhance.preview_task.take() else { return };
                let info = out.photos.first().and_then(|r| r.as_ref().ok()).map(|p| p.info.clone()).unwrap_or(Value::Null);
                if let Some(Err((_, e))) = out.photos.first() {
                    app.enhance.error = Some(e.clone());
                    app.enhance.failed_options = Some(t.options.clone());
                }
                if let Some((before, after)) = out.preview {
                    let tex = |name: &str, img: &lightcraft_raster::Rgba8| {
                        ctx.load_texture(
                            name,
                            crate::display_p3::screen_image(p3, [img.width, img.height], &img.as_bytes()),
                            egui::TextureOptions::NEAREST,
                        )
                    };
                    app.enhance.preview =
                        Some((tex("enhance-before", &before), tex("enhance-after", &after), [after.width, after.height], t.options, info));
                }
            }
            Ok(Err(e)) => {
                let Some(t) = app.enhance.preview_task.take() else { return };
                if e != "cancelled" {
                    app.enhance.error = Some(e);
                    app.enhance.failed_options = Some(t.options);
                }
            }
            Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(80)),
        }
    }
    if let Some(t) = &app.enhance.final_task {
        match t.rx.try_recv() {
            Ok(r) => {
                let Some(t) = app.enhance.final_task.take() else { return };
                match r.map_err(lightcraft_engine::EngineError::Other).and_then(|out| app.session.finish_enhance(&t.job, out)) {
                    Ok(v) => {
                        let failed: Vec<String> = v["photos"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|p| p["ok"] == false)
                            .filter_map(|p| p["error"].as_str().map(str::to_string))
                            .collect();
                        app.enhance.last_result = Some(v);
                        if !t.quiet || !failed.is_empty() {
                            let msg = match failed.first() {
                                Some(e) => format!("Enhance failed: {e}", e = e),
                                None if t.options.in_place => crate::i18n::tr("Enhanced in place").to_string(),
                                None => crate::i18n::tr("Enhanced photo added").to_string(),
                            };
                            app.toast(ctx, msg);
                        }
                    }
                    Err(e) => {
                        if e.to_string() != "cancelled" {
                            app.ui.status = e.to_string();
                            app.toast(ctx, format!("Enhance failed: {e}", e = e));
                        }
                    }
                }
            }
            Err(_) => {
                let (f, stage) = t.progress.lock().map(|g| g.clone()).unwrap_or_default();
                let now = ctx.input(|i| i.time);
                app.ui.toast = Some((format!("Enhancing… {stage} {:.0}%", f * 100.0, stage = stage), now + 0.5, None));
                ctx.request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
    }
}

pub(crate) fn model_row(app: &mut LightcraftApp, ui: &mut egui::Ui, key: &str, models: &Value) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let Some(m) = models["models"].as_array().and_then(|a| a.iter().find(|m| m["model"] == key)) else { return };
    if m["installed"] == true {
        return;
    }
    let dl = &m["download"];
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
        let mb = m["bytes"].as_f64().unwrap_or(0.0) / 1e6;
        if dl["running"] == true {
            let pct = dl["done"].as_f64().unwrap_or(0.0) / dl["total"].as_f64().unwrap_or(1.0).max(1.0) * 100.0;
            ui.label(egui::RichText::new(format!("Downloading {:.0}%", pct)).color(t.text_dim).size(12.0));
            ctx_repaint(ui);
        } else {
            let r = crate::widgets::text_button(ui, &format!("enhanceDownload-{key}"), "dl.", false).on_hover_text(format!(
                "{} — {:.0} MB, {} (not LightCraft's licence; {})",
                m["name"].as_str().unwrap_or(key),
                mb,
                m["licence"].as_str().unwrap_or(""),
                m["licenceUrl"].as_str().unwrap_or("")
            ));
            if r.clicked() {
                // the button's hover text states the licence and size: clicking it is the user's agreement
                if let Err(e) = app.run("enhance.model.download", json!({"model": key, "acknowledged": true})) {
                    app.toast(ui.ctx(), e);
                }
            }
            ui.label(egui::RichText::new(format!("{:.0} MB · {}", mb, m["licence"].as_str().unwrap_or(""))).color(t.text_dim).size(12.0));
        }
        if let Some(e) = dl["error"].as_str() {
            ui.label(egui::RichText::new(e).color(t.text_dim).size(11.0));
        }
    });
}

fn ctx_repaint(ui: &egui::Ui) {
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
}

/// The dialog body (preview on the left, options on the right).
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut EnhanceDialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let models = app.session.enhancer.models_status();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(12.0, 12.0);
        // before | after
        ui.vertical(|ui| {
            let side = PREVIEW_SIDE as f32;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(2.0 * side + 12.0, side), egui::Sense::hover());
            let left = egui::Rect::from_min_size(rect.min, egui::vec2(side, side));
            let right = egui::Rect::from_min_size(rect.min + egui::vec2(side + 12.0, 0.0), egui::vec2(side, side));
            for r in [left, right] {
                ui.painter().rect_filled(r, 4.0, t.canvas);
            }
            crate::widgets::register(ui.ctx(), "enhance.preview", rect);
            if let Some((before, after, size, _, _)) = &app.enhance.preview {
                let s = (side / size[0] as f32).min(side / size[1] as f32);
                let dims = egui::vec2(size[0] as f32 * s, size[1] as f32 * s);
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(before.id(), egui::Rect::from_center_size(left.center(), dims), uv, egui::Color32::WHITE);
                ui.painter().image(after.id(), egui::Rect::from_center_size(right.center(), dims), uv, egui::Color32::WHITE);
            }
            for (r, label) in [(left, "Before"), (right, "After")] {
                ui.painter().text(r.left_top() + egui::vec2(10.0, 10.0), egui::Align2::LEFT_TOP, crate::i18n::tr(label), t.font(12.0), t.text);
            }
            if let Some(task) = &app.enhance.preview_task {
                let (f, stage) = task.progress.lock().map(|g| g.clone()).unwrap_or_default();
                ui.painter().text(
                    right.center_bottom() - egui::vec2(0.0, 16.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{stage}… {:.0}%", f * 100.0, stage = stage),
                    t.font(12.0),
                    t.text,
                );
            }
            if let Some(e) = &app.enhance.error {
                ui.painter().text(right.center(), egui::Align2::CENTER_CENTER, e, t.font(12.0), t.text);
            }
            let info = app.enhance.preview.as_ref().map(|p| p.4.clone());
            let n = app.enhance.ids.len();
            let mut line = format!("{n} photo{}", if n == 1 { "" } else { "s" }, n = n);
            if let Some(info) = info {
                if let Some(d) = info["denoiser"].as_str() {
                    line.push_str(&format!(" · {}", if d == "rawNind" { "RawNIND model" } else { "built-in denoiser" }));
                }
                if let Some(s) = info["seconds"].as_object() {
                    let total: f64 = s.values().filter_map(Value::as_f64).sum();
                    line.push_str(&format!(" · preview {total:.1} s"));
                }
            }
            ui.label(egui::RichText::new(line).color(t.text_dim).size(12.0));
        });
        // options
        ui.vertical(|ui| {
            ui.set_width(250.0);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
            ui.checkbox(&mut opts.raw_details, crate::i18n::tr("Raw Details"))
                .on_hover_text(crate::i18n::tr("A finer demosaic (RCD): more fine detail, fewer maze and zipper artefacts"));
            ui.checkbox(&mut opts.super_resolution, crate::i18n::tr("Super Resolution"))
                .on_hover_text(crate::i18n::tr("Twice the width and height (Real-ESRGAN)"));
            model_row(app, ui, "super-resolution", &models);
            ui.checkbox(&mut opts.denoise, crate::i18n::tr("Denoise")).on_hover_text(crate::i18n::tr(
                "AI Denoise on the raw mosaic (RawNIND once downloaded, else the built-in denoiser); denoises as it demosaics",
            ));
            if opts.denoise {
                let mut a = opts.amount;
                if crate::widgets::slider(ui, &AMOUNT, a, true, None).value.map(|v| a = v).is_some() {
                    opts.amount = a.round();
                }
                model_row(app, ui, "raw-denoise", &models);
            }
            ui.add_space(4.0);
            ui.label(egui::RichText::new(crate::i18n::tr("Result")).color(t.text_label));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                if crate::widgets::text_button(ui, "enhanceTarget-dng", "new.dng.", !opts.in_place)
                    .on_hover_text(crate::i18n::tr("A new DNG next to the raw (Lightroom Classic's way)"))
                    .clicked()
                {
                    opts.in_place = false;
                }
                if crate::widgets::text_button(ui, "enhanceTarget-inPlace", "in.place.", opts.in_place)
                    .on_hover_text(crate::i18n::tr("Keep it in the photo: the photo renders from the enhanced picture; undoable"))
                    .clicked()
                {
                    opts.in_place = true;
                }
            });
            if !opts.in_place {
                ui.checkbox(&mut opts.stack, crate::i18n::tr("Stack with original"));
            }
            if opts.super_resolution && opts.denoise {
                ui.label(egui::RichText::new(crate::i18n::tr("Denoise runs first, then Super Resolution")).color(t.text_dim).size(11.0));
            }
        });
    });
}

/// The dialog's Amount slider.
static AMOUNT: lightcraft_develop::ControlSpec = lightcraft_develop::ControlSpec {
    id: "enhance.amount",
    label: "Amount",
    section: lightcraft_develop::Section::Detail,
    min: 1.0,
    max: 100.0,
    default: 50.0,
    step: 1.0,
    decimals: 0,
    track: lightcraft_develop::Track::Plain,
};
