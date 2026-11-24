//! LC-SB-PIPELINE in the app (Bryan, 2026-11-23): folders that are shoots in Second Brain's
//! pipeline carry a `pipeline.editing` row in Folders and a line in Info ("from the second brain
//! pipeline · editing"); a folder that isn't there can be sent (`send.to.pipeline.` → a small
//! dialog about its backup → `pipeline.add`). The pipeline is re-read on a worker every couple of
//! seconds (only when its file changed), so a stage changed in Second Brain shows here, whether or
//! not Second Brain is running ([`lightcraft_engine::sb_pipeline`]).

use std::sync::Arc;

use egui::RichText;
use lightcraft_engine::sb_pipeline::{self as sbp, Backup, Mark, Pipeline};
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// Second Brain's pipeline as last read; `None` until the first read lands (or no home folder).
pub(crate) fn current(app: &LightcraftApp, ui: &egui::Ui) -> Option<Arc<Pipeline>> {
    // (tests never read the real Second Brain: only one they set up)
    if cfg!(target_arch = "wasm32") || (cfg!(test) && app.session.second_brain_dir.is_none()) {
        return None;
    }
    let dir = sbp::sb_dir(app.session.second_brain_dir.as_deref())?;
    crate::panels::left::fs_cached(ui, "sb-pipeline", &dir.to_string_lossy(), 2.0, sbp::load_str)
}

/// Is Second Brain running (its local API answers)? Asked on a worker every few seconds.
fn online(app: &LightcraftApp, ui: &egui::Ui) -> Option<bool> {
    if cfg!(target_arch = "wasm32") || (cfg!(test) && app.session.second_brain_dir.is_none()) {
        return Some(false);
    }
    let dir = sbp::sb_dir(app.session.second_brain_dir.as_deref())?;
    crate::panels::left::fs_cached(ui, "sb-online", &dir.to_string_lossy(), 5.0, |d| {
        lightcraft_engine::galleries::connect(Some(std::path::Path::new(d))).is_ok()
    })
}

/// The same shoot (or the same waiting request)?
pub(crate) fn same(a: Mark<'_>, b: Mark<'_>) -> bool {
    match (a, b) {
        (Mark::Shoot(x), Mark::Shoot(y)) => x.id == y.id && x.path == y.path,
        (Mark::Pending(x), Mark::Pending(y)) => x.id == y.id,
        _ => false,
    }
}

/// The mark to show on a Folders row for `folder` (the last folder of the row) whose row starts at
/// `top`: only the topmost row of a shoot carries it, not every folder inside.
pub(crate) fn row_mark<'a>(pipe: &'a Pipeline, top: &str, folder: &str) -> Option<Mark<'a>> {
    let mark = pipe.mark(folder)?;
    let above = std::path::Path::new(top).parent().map(|p| p.to_string_lossy().to_string());
    match above.as_deref().and_then(|p| pipe.mark(p)) {
        Some(up) if same(up, mark) => None,
        _ => Some(mark),
    }
}

/// What a mark says in full (hover text).
pub(crate) fn describe(mark: Mark<'_>) -> String {
    match mark {
        Mark::Shoot(s) => format!(
            "In Second Brain's shoot pipeline: “{}” · {}\n{}\nA stage changed in Second Brain shows here within a few seconds.",
            s.name,
            sbp::stage_word(&s.stage),
            s.path
        ),
        Mark::Pending(r) => format!(
            "Sent to Second Brain's shoot pipeline ({}) — it lands in editing when Second Brain takes it (at once while it's open, else when it next opens).",
            r.path
        ),
    }
}

/// Info's line for a photo in `folder`: where it stands in the pipeline, or `send.to.pipeline.`.
pub(crate) fn info_line(app: &mut LightcraftApp, ui: &mut egui::Ui, folder: &str) {
    let Some(pipe) = current(app, ui) else { return };
    let t = Tokens::get(ui.ctx());
    let mark = pipe.mark(folder);
    let text = match mark {
        Some(Mark::Shoot(s)) => format!("from the second brain pipeline · {}", sbp::stage_word(&s.stage)),
        Some(Mark::Pending(_)) => "sent to the second brain pipeline · waiting for second brain".to_string(),
        None => "not in the second brain pipeline".to_string(),
    };
    // the line on its own (it wraps in a narrow panel), send.to.pipeline. under it
    let color = if mark.is_some() { t.text_label } else { t.text_dim };
    let w = ui.available_width().max(40.0);
    let g = ui.painter().layout(text.clone(), t.font(12.0), color, w);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(w, g.size().y.max(18.0)), egui::Sense::hover());
    ui.painter().galley(egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, color);
    register(ui.ctx(), "info:pipeline", rect);
    if let Some(m) = mark {
        r.on_hover_text(describe(m));
    } else if crate::widgets::text_button(ui, "infoSendToPipeline", "send.to.pipeline.", false)
        .on_hover_text(crate::i18n::tr("Send this photo's folder to Second Brain's shoot pipeline: it lands in editing"))
        .clicked()
    {
        open_dialog(app, folder);
    }
}

/// Open the send dialog for `folder`.
pub(crate) fn open_dialog(app: &mut LightcraftApp, folder: &str) {
    let name = std::path::Path::new(folder).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| folder.to_string());
    app.ui.dialog = Some(crate::state::Dialog::SendToPipeline { path: folder.to_string(), name, backup: Backup::Run });
}

/// The three backup choices (id, label, what it does).
pub(crate) const CHOICES: [(Backup, &str, &str, &str); 3] = [
    (Backup::Run, "pipelineBackup-run", "back.up.", "Second Brain backs it up to its backup place and checks every file"),
    (Backup::Done, "pipelineBackup-done", "already.backed.up.", "it's backed up already: the backup step is marked done"),
    (Backup::Skip, "pipelineBackup-skip", "skip.", "no backup for now"),
];

/// The send dialog's body: the folder, the backup choice, what happens next.
pub(crate) fn dialog_body(app: &mut LightcraftApp, ui: &mut egui::Ui, path: &str, name: &str, backup: &mut Backup) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.label(RichText::new(format!("Send “{name}” to the second brain pipeline? It lands in editing.")).color(t.text));
        ui.add(egui::Label::new(RichText::new(path).size(12.0).color(t.text_dim)).truncate());
    });
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(RichText::new(crate::i18n::tr("The backup first:")).color(t.text_label));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (b, id, label, tip) in CHOICES {
                if crate::widgets::text_button(ui, id, label, *backup == b).on_hover_text(crate::i18n::tr(tip)).clicked() {
                    *backup = b;
                }
            }
        });
        let pipe = current(app, ui);
        let note = match *backup {
            Backup::Run => match pipe.as_deref().map(|p| p.backup_places.as_slice()) {
                Some([]) => "Second Brain asks where to back it up (it has no backup place yet).".to_string(),
                Some(places) => format!(
                    "Second Brain copies it to {} and checks every file.",
                    places.iter().map(|p| p.rsplit('/').next().unwrap_or(p)).collect::<Vec<_>>().join(", ")
                ),
                None => "Second Brain copies it to its backup place and checks every file.".to_string(),
            },
            Backup::Done => "The backup step is marked done; nothing is copied.".to_string(),
            Backup::Skip => "No backup now; you can start one on its card in Second Brain.".to_string(),
        };
        ui.add(egui::Label::new(RichText::new(note).size(12.0).color(t.text_dim)).wrap());
    });
    let when = match online(app, ui) {
        Some(true) => "Second Brain is open: it takes it now.",
        Some(false) => "Second Brain is closed: it takes it when it next opens; until then the folder shows pipeline.pending.",
        None => "",
    };
    if !when.is_empty() {
        let r = ui.add(egui::Label::new(RichText::new(crate::i18n::tr(when)).size(12.0).color(t.text_dim)).wrap());
        register(ui.ctx(), "pipelineWhen", r.rect);
    }
}

/// The dialog's ok.: queue it, say what happens next.
pub(crate) fn confirm(app: &mut LightcraftApp, path: &str, name: &str, backup: Backup) -> Result<serde_json::Value, String> {
    let r = app.run("pipeline.add", json!({"folder": path, "name": name, "backup": backup.id()}))?;
    app.ui.status = format!("“{name}” sent to the second brain pipeline · pipeline.pending until Second Brain takes it");
    Ok(r)
}
