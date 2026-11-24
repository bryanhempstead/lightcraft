//! Photo Merge in the UI: the HDR / Panorama / HDR Panorama dialog (options + live preview) and
//! the background merge jobs.
//!
//! Previews and the final merge run [`lightcraft_engine::merge::MergeJob`]s on worker threads with
//! progress and cancellation; the window stays responsive. Changing an option cancels the running
//! preview and starts a new one. The final merge keeps running after the dialog closes (progress
//! in a toast); when it finishes, the result is written, imported and selected
//! ([`lightcraft_engine::Session::finish_merge`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use lightcraft_catalog::PhotoId;
use lightcraft_engine::merge::{MergeJob, MergeOutput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::LightcraftApp;

/// The dialog's options (also the parameters of the `merge.*` commands).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergeDialog {
    /// "merge.hdr" | "merge.panorama" | "merge.hdrPanorama" | "merge.focus".
    pub command: String,
    pub align: bool,
    /// none | low | medium | high
    pub deghost: String,
    pub show_overlay: bool,
    pub auto_settings: bool,
    pub stack: bool,
    /// auto | spherical | cylindrical | perspective
    pub projection: String,
    pub boundary_warp: f64,
    pub auto_crop: bool,
    pub fill_edges: bool,
    /// HDR panorama: photos per bracket (0 = from EXIF).
    pub bracket: u32,
    /// Best take: the photo (index into the merged photos) everything else comes from.
    pub base: usize,
    /// Best take: the people found, and the photo each one's face comes from.
    pub people: Vec<TakePick>,
}

/// One person in a best take.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TakePick {
    pub name: String,
    /// Their face in each photo (`None` where it wasn't found): normalized `[x, y, w, h]`.
    pub boxes: Vec<Option<[f64; 4]>>,
    /// The photo (index) their face comes from.
    pub from: usize,
}

impl TakePick {
    /// Their face in photo `i`: where it was found there, else where it was found in the nearest
    /// photo (a burst barely moves; the merge's patch search lines it up).
    pub fn box_at(&self, i: usize) -> Option<[f64; 4]> {
        if let Some(b) = self.boxes.get(i).copied().flatten() {
            return Some(b);
        }
        let n = self.boxes.len();
        (1..n).find_map(|d| [i.checked_sub(d), i.checked_add(d)].into_iter().flatten().find_map(|j| self.boxes.get(j).copied().flatten()))
    }
}

impl Default for MergeDialog {
    fn default() -> Self {
        MergeDialog {
            command: "merge.hdr".into(),
            align: true,
            deghost: "none".into(),
            show_overlay: false,
            auto_settings: true,
            stack: false,
            projection: "auto".into(),
            boundary_warp: 0.0,
            auto_crop: true,
            fill_edges: false,
            bracket: 0,
            base: 0,
            people: Vec::new(),
        }
    }
}

impl MergeDialog {
    pub fn for_command(command: &str) -> MergeDialog {
        MergeDialog { command: command.into(), ..Default::default() }
    }
    pub fn title(&self) -> &'static str {
        match self.command.as_str() {
            "merge.panorama" => "Panorama Merge Preview",
            "merge.hdrPanorama" => "HDR Panorama Merge Preview",
            "merge.focus" => "Focus Stack Merge Preview",
            "merge.take" => "Best Take Merge Preview",
            _ => "HDR Merge Preview",
        }
    }
    /// What it makes, for messages ("HDR merge added").
    pub fn what(&self) -> &'static str {
        match self.command.as_str() {
            "merge.panorama" => "Panorama",
            "merge.hdrPanorama" => "HDR panorama",
            "merge.focus" => "Focus stack",
            "merge.take" => "Best take",
            _ => "HDR",
        }
    }
    pub fn is_hdr(&self) -> bool {
        matches!(self.command.as_str(), "merge.hdr" | "merge.hdrPanorama")
    }
    pub fn is_pano(&self) -> bool {
        matches!(self.command.as_str(), "merge.panorama" | "merge.hdrPanorama")
    }
    pub fn params(&self) -> Value {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if self.command == "merge.take" {
            // each person whose face comes from another photo: their face there onto them here
            let swaps: Vec<Value> = self
                .people
                .iter()
                .filter(|p| p.from != self.base)
                .filter_map(|p| {
                    let (donor, base) = (p.box_at(p.from)?, p.box_at(self.base)?);
                    Some(json!({"donor": p.from, "baseBox": base, "donorBox": donor}))
                })
                .collect();
            v["swaps"] = json!(swaps);
        }
        v
    }
}

/// A merge running on a worker thread.
pub struct MergeTask {
    pub preview: bool,
    pub job: MergeJob,
    /// The dialog options it was started with.
    pub options: MergeDialog,
    pub progress: Arc<Mutex<(f32, String)>>,
    pub cancel: Arc<AtomicBool>,
    rx: Receiver<Result<MergeOutput, String>>,
    /// merges.: the group it merges and the frames it was given.
    pub group: Option<(String, Vec<PhotoId>)>,
}

/// merges.: a group waiting to be merged (`merge.mergeGroups` in the app runs them one at a time
/// on the merge worker).
#[derive(Clone, Debug, PartialEq)]
pub struct GroupMerge {
    pub group: String,
    pub kind: lightcraft_catalog::MergeType,
    pub frames: Vec<PhotoId>,
}

/// Merge state of the app.
#[derive(Default)]
pub struct MergeState {
    pub preview_task: Option<MergeTask>,
    pub final_task: Option<MergeTask>,
    /// The latest preview (texture, size, its options, stats) and the latest error.
    pub preview: Option<(egui::TextureHandle, [usize; 2], MergeDialog, Value)>,
    pub preview_pixels: Option<Arc<egui::ColorImage>>,
    pub error: Option<String>,
    /// Options whose preview failed (not retried until they change).
    pub failed_options: Option<MergeDialog>,
    /// The last finished merge (`{id, path, …}`).
    pub last_result: Option<Value>,
    /// Photos the dialog merges (the selection when it opened).
    pub ids: Vec<PhotoId>,
    /// The options of the last merge started, per kind ("merge.hdr" …): Merge with Last Settings.
    pub last: std::collections::HashMap<String, MergeDialog>,
    /// The kind of the last merge started: what Merge… opens on.
    pub last_kind: Option<String>,
    /// merges.: groups waiting for the merge worker, in order.
    pub group_queue: std::collections::VecDeque<GroupMerge>,
    /// How the dialog's preview is framed (dragged and zoomed; whole picture when it opens).
    pub view: crate::pan_zoom::PanZoom,
}

/// The kinds the Merge… dialog switches between: (command, button label, hover text).
pub const KINDS: [(&str, &str, &str); 5] = [
    ("merge.hdr", "hdr.", "HDR — exposure brackets (the same view shot darker and brighter) → one photo with the whole range"),
    ("merge.panorama", "pano.", "Panorama — overlapping photos → one wide photo"),
    ("merge.hdrPanorama", "hdr.pano.", "HDR panorama — a bracket at each position → merged, then stitched"),
    ("merge.focus", "focus.", "Focus stack — the same view focused near to far → one photo sharp throughout"),
    ("merge.take", "take.", "Best take — a burst or group shot: pick, per person, the photo their face comes from"),
];

/// Best take: find the people in the merged photos (tagged faces, else Apple Vision) and start
/// everyone on the base photo. Keeps earlier picks for the same people.
fn load_faces(app: &mut LightcraftApp, opts: &mut MergeDialog) {
    let ids = app.merge.ids.clone();
    if opts.people.first().is_some_and(|p| p.boxes.len() == ids.len()) {
        return;
    }
    if let Some(a) = app.session.active() {
        opts.base = ids.iter().position(|x| *x == a).unwrap_or(0);
    }
    match app.session.take_faces(&ids) {
        Ok(people) => {
            opts.people = people.into_iter().map(|p| TakePick { name: p.name, boxes: p.boxes, from: opts.base }).collect();
            app.merge.error = opts.people.is_empty().then(|| crate::i18n::tr("No faces found in these photos").to_string());
        }
        Err(e) => app.merge.error = Some(e),
    }
}

impl MergeState {
    /// A preview or a merge is running.
    pub fn busy(&self) -> bool {
        self.preview_task.is_some() || self.final_task.is_some()
    }
}

fn spawn(app: &LightcraftApp, options: &MergeDialog, ids: &[PhotoId], preview: bool) -> Result<MergeTask, String> {
    let (kind, finish) = lightcraft_engine::merge::parse(&options.command, &options.params()).map_err(|e| e.to_string())?;
    let job = app.session.plan_merge(kind, finish, ids, preview).map_err(|e| e.to_string())?;
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
        // a panicking merge reports an error instead of leaving "Merging…" up forever
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("the merge failed unexpectedly".into()));
        let _ = tx.send(r);
    };
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name("photo-merge".into()).spawn(work).map_err(|e| e.to_string())?;
    #[cfg(target_arch = "wasm32")]
    work();
    Ok(MergeTask { preview, job, options: options.clone(), progress, cancel, rx, group: None })
}

/// merges.: merge a group on the worker with the options last used for its kind (never stacked:
/// its frames are hidden instead).
pub fn start_group(app: &mut LightcraftApp, g: &GroupMerge) -> Result<(), String> {
    if app.merge.final_task.is_some() {
        return Err("a merge is already running".into());
    }
    let command = g.kind.command();
    let mut opts = app.merge.last.get(command).cloned().unwrap_or_else(|| MergeDialog::for_command(command));
    opts.stack = false;
    let mut task = spawn(app, &opts, &g.frames, false)?;
    task.group = Some((g.group.clone(), g.frames.clone()));
    app.merge.final_task = Some(task);
    Ok(())
}

/// Open the merge dialog for the selection.
pub fn open(app: &mut LightcraftApp, command: &str) -> Result<Value, String> {
    let ids = app.session.targets(&json!({}));
    if ids.len() < 2 {
        return Err("select at least 2 photos to merge".into());
    }
    app.merge.ids = ids;
    app.merge.error = None;
    app.merge.preview = None;
    app.merge.preview_pixels = None;
    app.merge.view = Default::default();
    if let Some(t) = app.merge.preview_task.take() {
        t.cancel.store(true, Ordering::Relaxed);
    }
    // Merge… ("merge"): the kind used last, with the options used last
    let command = if command == "merge" { app.merge.last_kind.clone().unwrap_or_else(|| "merge.hdr".into()) } else { command.to_string() };
    let mut opts = app.merge.last.get(&command).cloned().unwrap_or_else(|| MergeDialog::for_command(&command));
    if command == "merge.take" {
        opts.people.clear(); // other photos than last time
        load_faces(app, &mut opts);
    }
    app.ui.dialog = Some(crate::state::Dialog::Merge { opts });
    Ok(json!({"photos": app.merge.ids.len()}))
}

/// `merge.view {zoom?, center?: [x, y]}`: frame the Merge preview (zoom 1 = the whole preview;
/// `center` the point of it in the middle, 0..1). Without parameters: the current framing.
pub fn set_view(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let mut v = app.merge.view;
    if let Some(z) = p.get("zoom").and_then(Value::as_f64) {
        v.zoom = z as f32;
    }
    if let Some(c) = p.get("center").and_then(Value::as_array) {
        let n = |i: usize| c.get(i).and_then(Value::as_f64).map(|x| x as f32);
        v.center = egui::vec2(n(0).ok_or("`center` is [x, y]")?, n(1).ok_or("`center` is [x, y]")?);
    }
    let pane = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(520.0, 360.0));
    let size = app.merge.preview.as_ref().map_or([520, 360], |p| p.1);
    app.merge.view = v.clamped(pane, size);
    Ok(json!({"zoom": app.merge.view.zoom, "center": [app.merge.view.center.x, app.merge.view.center.y]}))
}

/// Start the full-resolution merge (the dialog's Merge button / `ui.dialog.confirm`).
pub fn start_final(app: &mut LightcraftApp, opts: &MergeDialog) -> Result<Value, String> {
    if app.merge.final_task.is_some() {
        return Err("a merge is already running".into());
    }
    if let Some(t) = app.merge.preview_task.take() {
        t.cancel.store(true, Ordering::Relaxed);
    }
    let ids = app.merge.ids.clone();
    let task = spawn(app, opts, &ids, false)?;
    app.merge.final_task = Some(task);
    app.merge.last.insert(opts.command.clone(), opts.clone());
    app.merge.last_kind = Some(opts.command.clone());
    Ok(json!({"started": true, "photos": ids.len()}))
}

/// Merge the selection without the dialog, with the options last used for `command` (the
/// defaults the first time).
pub fn start_last(app: &mut LightcraftApp, command: &str) -> Result<Value, String> {
    let ids = app.session.targets(&json!({}));
    if ids.len() < 2 {
        return Err("select at least 2 photos to merge".into());
    }
    app.merge.ids = ids;
    let opts = app.merge.last.get(command).cloned().unwrap_or_else(|| MergeDialog::for_command(command));
    start_final(app, &opts)
}

/// Per frame: keep the preview in sync with the dialog's options, collect finished jobs.
pub fn poll(app: &mut LightcraftApp, ctx: &egui::Context) {
    let p3 = app.renderer.display_p3;
    // preview for the open dialog
    let dialog_opts = match &app.ui.dialog {
        Some(crate::state::Dialog::Merge { opts }) => Some(opts.clone()),
        _ => None,
    };
    match &dialog_opts {
        Some(opts) => {
            let current = app.merge.preview_task.as_ref().map(|t| &t.options).or(app.merge.preview.as_ref().map(|p| &p.2));
            let stale = current != Some(opts) && !last_failed_matches(app, opts);
            if stale {
                if let Some(t) = app.merge.preview_task.take() {
                    t.cancel.store(true, Ordering::Relaxed);
                }
                let ids = app.merge.ids.clone();
                match spawn(app, opts, &ids, true) {
                    Ok(t) => {
                        app.merge.error = None;
                        app.merge.preview_task = Some(t);
                    }
                    Err(e) => {
                        app.merge.error = Some(e);
                        app.merge.failed_options = Some(opts.clone());
                    }
                }
            }
        }
        None => {
            if let Some(t) = app.merge.preview_task.take() {
                t.cancel.store(true, Ordering::Relaxed);
            }
        }
    }
    if let Some(t) = &app.merge.preview_task {
        match t.rx.try_recv() {
            Ok(Ok(out)) => {
                let Some(t) = app.merge.preview_task.take() else { return };
                if let Some(img) = out.preview {
                    let color = Arc::new(egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.as_bytes()));
                    // (the screen gets the display's values; the kept pixels stay sRGB)
                    let shown = crate::display_p3::screen_image(p3, [img.width, img.height], &img.as_bytes());
                    let tex = ctx.load_texture("merge-preview", shown, egui::TextureOptions::LINEAR);
                    app.merge.preview_pixels = Some(color);
                    app.merge.preview = Some((tex, [img.width, img.height], t.options, out.info));
                }
            }
            Ok(Err(e)) => {
                let Some(t) = app.merge.preview_task.take() else { return };
                if e != "cancelled" {
                    app.merge.error = Some(e);
                    app.merge.failed_options = Some(t.options);
                }
            }
            Err(_) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
        }
    }
    // the final merge
    if let Some(t) = &app.merge.final_task {
        match t.rx.try_recv() {
            Ok(r) => {
                let Some(t) = app.merge.final_task.take() else { return };
                let what = t.options.what();
                match r.map_err(lightcraft_engine::EngineError::Other).and_then(|out| app.session.finish_merge(&t.job, out)) {
                    Ok(v) => {
                        // merges.: the frames are hidden, the merged photo goes where they were
                        if let Some((g, frames)) = &t.group
                            && let Some(id) = v.get("id").and_then(Value::as_u64)
                            && let Err(e) = app.session.merge_group_done(g, PhotoId(id), frames)
                        {
                            app.ui.status = e.to_string();
                        }
                        app.merge.last_result = Some(v);
                        app.toast(ctx, crate::i18n::tr_format!("{what} merge added", what = what));
                    }
                    Err(e) => {
                        app.ui.status = e.to_string();
                        app.toast(ctx, crate::i18n::tr_format!("{what} merge failed: {e}", e = e, what = what));
                    }
                }
            }
            Err(_) => {
                let (f, stage) = t.progress.lock().map(|g| g.clone()).unwrap_or_default();
                let now = ctx.input(|i| i.time);
                app.ui.toast = Some((crate::i18n::tr_format!("Merging… {stage} {:.0}%", f * 100.0, stage = stage), now + 0.5, None));
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }
}

/// merges.: start the next queued group once the worker is free.
pub fn next_group(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.merge.final_task.is_some() {
        return;
    }
    while let Some(g) = app.merge.group_queue.pop_front() {
        match start_group(app, &g) {
            Ok(()) => {
                ctx.request_repaint();
                return;
            }
            Err(e) => app.toast(ctx, format!("Merge failed: {e}")),
        }
    }
}

fn last_failed_matches(app: &LightcraftApp, opts: &MergeDialog) -> bool {
    app.merge.failed_options.as_ref() == Some(opts)
}

/// The dialog body (options on the left, preview on the right).
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut MergeDialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.horizontal_top(|ui| {
        // preview
        ui.vertical(|ui| {
            let box_size = egui::vec2(520.0, 360.0);
            let (rect, _) = ui.allocate_exact_size(box_size, egui::Sense::hover());
            ui.painter().rect_filled(rect, 4.0, t.canvas);
            crate::widgets::register(ui.ctx(), "merge.preview", rect);
            // drag to look around, scroll or pinch to zoom (crate::pan_zoom)
            let size = app.merge.preview.as_ref().map_or([0, 0], |p| p.1);
            let mut view = app.merge.view;
            crate::pan_zoom::interact(ui, rect, "mergePreview", size, &mut view);
            app.merge.view = view;
            if let Some((tex, size, _, _)) = &app.merge.preview {
                crate::pan_zoom::paint(ui, rect, tex.id(), *size, &view);
            }
            if let Some(task) = &app.merge.preview_task {
                let (f, stage) = task.progress.lock().map(|g| g.clone()).unwrap_or_default();
                ui.painter().text(
                    rect.center_bottom() - egui::vec2(0.0, 14.0),
                    egui::Align2::CENTER_CENTER,
                    crate::i18n::tr_format!("{stage}… {:.0}%", f * 100.0, stage = stage),
                    t.font(12.0),
                    t.text,
                );
            }
            if let Some(e) = &app.merge.error {
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, e, t.font(13.0), t.text);
            }
            let info = app.merge.preview.as_ref().map(|p| p.3.clone());
            if let Some(info) = info {
                let txt = if opts.command == "merge.take" {
                    let n = opts.people.iter().filter(|p| p.from != opts.base).count();
                    format!(
                        "{} · {n} {}",
                        crate::i18n::tr_format!("{} photos", app.merge.ids.len()),
                        crate::i18n::tr(if n == 1 { "face swapped" } else { "faces swapped" })
                    )
                } else if opts.command == "merge.focus" {
                    format!("{} · {}", crate::i18n::tr_format!("{} photos", app.merge.ids.len()), crate::i18n::tr("focus stack"))
                } else if opts.is_pano() {
                    format!(
                        "{} photos · {} · {:.0}° × {:.0}°",
                        info["used"].as_array().map_or(0, Vec::len),
                        info["projection"].as_str().unwrap_or(""),
                        info["fov"][0].as_f64().unwrap_or(0.0),
                        info["fov"][1].as_f64().unwrap_or(0.0)
                    )
                } else {
                    let ev: Vec<String> =
                        info["ev"].as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|v| format!("{v:+.1}")).collect()).unwrap_or_default();
                    crate::i18n::tr_format!("{} photos · exposures {} EV", ev.len(), ev.join(" / "))
                };
                ui.label(egui::RichText::new(txt).color(t.text_dim));
            }
        });
        ui.add_space(12.0);
        // options
        ui.vertical(|ui| {
            ui.set_width(220.0);
            // the kind: switch between the merges without closing the dialog
            ui.label(egui::RichText::new(crate::i18n::tr("Merge")).color(t.text_label));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                for (cmd, label, tip) in KINDS {
                    let r =
                        crate::widgets::text_button(ui, &format!("mergeKind-{cmd}"), label, opts.command == cmd).on_hover_text(crate::i18n::tr(tip));
                    if r.clicked() && opts.command != cmd {
                        // keep the shared options, take this kind's own from its last use
                        let keep = (opts.auto_settings, opts.stack);
                        *opts = app.merge.last.get(cmd).cloned().unwrap_or_else(|| MergeDialog::for_command(cmd));
                        (opts.auto_settings, opts.stack) = keep;
                        if cmd == "merge.take" {
                            opts.people.clear();
                            load_faces(app, opts);
                        }
                    }
                }
            });
            ui.add_space(12.0);
            if opts.command == "merge.take" {
                take_picker(app, ui, opts);
            }
            if opts.command == "merge.focus" {
                ui.checkbox(&mut opts.align, crate::i18n::tr("Auto Align"))
                    .on_hover_text(crate::i18n::tr("Line the photos up first (handheld, or focus breathing changes the scale)"));
                ui.add_space(4.0);
            }
            if opts.is_pano() {
                ui.label(egui::RichText::new(crate::i18n::tr("Projection")).color(t.text_label));
                for (v, l) in [("auto", "Auto Select"), ("spherical", "Spherical"), ("cylindrical", "Cylindrical"), ("perspective", "Perspective")] {
                    if ui.radio(opts.projection == v, l).clicked() {
                        opts.projection = v.to_string();
                    }
                }
                ui.add_space(4.0);
                let mut bw = opts.boundary_warp;
                if num(ui, &BOUNDARY_WARP, &mut bw) {
                    opts.boundary_warp = bw.round();
                }
                ui.checkbox(&mut opts.fill_edges, crate::i18n::tr("Fill Edges"));
                ui.checkbox(&mut opts.auto_crop, crate::i18n::tr("Auto Crop"));
                ui.add_space(4.0);
            }
            if opts.is_hdr() {
                ui.checkbox(&mut opts.align, crate::i18n::tr("Auto Align"));
                ui.label(egui::RichText::new(crate::i18n::tr("Deghost Amount")).color(t.text_label));
                ui.horizontal(|ui| {
                    for (i, (v, l)) in [("none", "None"), ("low", "Low"), ("medium", "Med"), ("high", "High")].iter().enumerate() {
                        if crate::widgets::text_button(ui, &format!("mergeDeghost-{i}"), l, opts.deghost == *v).clicked() {
                            opts.deghost = v.to_string();
                        }
                    }
                });
                if opts.command == "merge.hdr" {
                    ui.add_enabled_ui(opts.deghost != "none", |ui| ui.checkbox(&mut opts.show_overlay, crate::i18n::tr("Show Deghost Overlay")));
                }
                if opts.command == "merge.hdrPanorama" {
                    let mut b = opts.bracket as f64;
                    if num(ui, &BRACKET, &mut b) {
                        opts.bracket = b.round() as u32;
                    }
                }
                ui.add_space(4.0);
            }
            ui.checkbox(&mut opts.auto_settings, crate::i18n::tr("Auto Settings"));
            ui.checkbox(&mut opts.stack, crate::i18n::tr("Create Stack"));
            ui.add_space(8.0);
            ui.label(egui::RichText::new(crate::i18n::tr_format!("{} photos", app.merge.ids.len())).color(t.text_dim));
        });
    });
}

/// Best take: which photo to start from, then per person their face in each photo — click the one
/// to use (the highlighted face is used; faces in the start photo stay as they are).
fn take_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut MergeDialog) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let ids = app.merge.ids.clone();
    ui.label(egui::RichText::new(crate::i18n::tr("Start from photo")).color(t.text_label));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
        for (i, id) in ids.iter().enumerate() {
            let name = app.session.catalog.photo(*id).map(|p| p.file_name.clone()).unwrap_or_default();
            let r = crate::widgets::text_button(ui, &format!("takeBase-{i}"), &format!("{}.", i + 1), opts.base == i).on_hover_text(name);
            if r.clicked() && opts.base != i {
                // people still on the old start photo move with it
                for p in opts.people.iter_mut().filter(|p| p.from == opts.base) {
                    p.from = i;
                }
                opts.base = i;
            }
        }
    });
    ui.add_space(12.0);
    if opts.people.is_empty() {
        ui.label(egui::RichText::new(crate::i18n::tr("No faces found in these photos")).color(t.text_dim));
        if crate::widgets::text_button(ui, "takeFind", "find.", false).on_hover_text(crate::i18n::tr("Look for faces again")).clicked() {
            load_faces(app, opts);
        }
        return;
    }
    let base = opts.base;
    egui::ScrollArea::vertical().id_salt("takePeople").max_height(300.0).show(ui, |ui| {
        for (pi, person) in opts.people.iter_mut().enumerate() {
            ui.label(egui::RichText::new(&person.name).color(t.text));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                for (fi, id) in ids.iter().enumerate() {
                    let Some(b) = person.box_at(fi) else { continue };
                    let found = person.boxes.get(fi).is_some_and(Option::is_some);
                    super_thumb_request(app, *id);
                    let size = egui::vec2(44.0, 44.0);
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    crate::widgets::register(ui.ctx(), format!("takeFace-{pi}-{fi}"), rect);
                    ui.painter().rect_filled(rect, 4.0, t.canvas);
                    if let Some(tex) = app.renderer.thumb(*id) {
                        // the face, a little loose, from the photo's (cropped) thumbnail
                        let crop = app.session.develop_of(*id).map(|d| d.crop.geometry.rect);
                        let to_thumb = |u: f64, v: f64| match crop {
                            Some(c) if c.x1 > c.x0 && c.y1 > c.y0 => ((u - c.x0) / (c.x1 - c.x0), (v - c.y0) / (c.y1 - c.y0)),
                            _ => (u, v),
                        };
                        let pad = 0.25;
                        let (x0, y0) = to_thumb(b[0] - b[2] * pad, b[1] - b[3] * pad);
                        let (x1, y1) = to_thumb(b[0] + b[2] * (1.0 + pad), b[1] + b[3] * (1.0 + pad));
                        let c = |v: f64| v.clamp(0.0, 1.0) as f32;
                        let uv = egui::Rect::from_min_max(egui::pos2(c(x0), c(y0)), egui::pos2(c(x1), c(y1)));
                        if uv.width() > 0.0 && uv.height() > 0.0 {
                            let tint = if found { egui::Color32::WHITE } else { egui::Color32::from_white_alpha(170) };
                            ui.painter().image(tex.tex.id(), rect.shrink(2.0), uv, tint);
                        }
                    }
                    let picked = person.from == fi;
                    if picked {
                        ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
                    }
                    let tip = match (fi == base, found) {
                        (true, _) => crate::i18n::tr("photo {n} (the start photo): keep this face"),
                        (false, true) => crate::i18n::tr("photo {n}: use this face"),
                        (false, false) => {
                            crate::i18n::tr("photo {n}: use this face (not found there: taken from the same spot as in the nearest photo)")
                        }
                    };
                    if resp.on_hover_text(tip.replace("{n}", &(fi + 1).to_string())).clicked() {
                        person.from = fi;
                    }
                }
            });
            ui.add_space(12.0);
        }
    });
}

fn super_thumb_request(app: &mut LightcraftApp, id: PhotoId) {
    crate::panels::grid::request_thumb(app, id, 512, 4);
}

const BOUNDARY_WARP: lightcraft_develop::ControlSpec = lightcraft_develop::ControlSpec {
    id: "merge.boundaryWarp",
    label: "Boundary Warp",
    section: lightcraft_develop::Section::Light,
    min: 0.0,
    max: 100.0,
    default: 0.0,
    step: 1.0,
    decimals: 0,
    track: lightcraft_develop::Track::Plain,
};
const BRACKET: lightcraft_develop::ControlSpec = lightcraft_develop::ControlSpec {
    id: "merge.bracket",
    label: "Photos per bracket (0 = auto)",
    section: lightcraft_develop::Section::Light,
    min: 0.0,
    max: 9.0,
    default: 0.0,
    step: 1.0,
    decimals: 0,
    track: lightcraft_develop::Track::Plain,
};

fn num(ui: &mut egui::Ui, spec: &lightcraft_develop::ControlSpec, v: &mut f64) -> bool {
    match crate::widgets::slider(ui, spec, *v, true, None).value {
        Some(n) => {
            *v = n;
            true
        }
        None => false,
    }
}
