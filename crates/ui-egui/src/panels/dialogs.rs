//! Modal dialogs (new album, rename, create preset, choose settings to copy, export, about, shortcuts).

use lightcraft_develop::{ControlSpec, Section, Track};
use serde_json::json;

use crate::LightcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// Rows the Rename dialog previews.
const RENAME_PREVIEW_ROWS: usize = 6;

/// The Rename dialog's preview: its first rows (`None` while being planned) and how many files
/// are renamed. Which names are taken is checked on disk, which can block on a slow drive, so the
/// rows are planned on a worker thread, and only when the template, start number, photos or
/// catalog change — never once per frame.
pub(crate) fn rename_preview(
    app: &mut LightcraftApp,
    ctx: &egui::Context,
    template: &str,
    start: usize,
    text: &str,
) -> (Option<Vec<lightcraft_engine::rename::RenamePlan>>, usize) {
    use std::hash::{Hash, Hasher};
    type Rows = std::sync::Arc<std::sync::Mutex<Option<Vec<lightcraft_engine::rename::RenamePlan>>>>;
    let ids = app.session.targets(&json!({}));
    let photos = lightcraft_engine::rename::rename_photos(&app.session.catalog, &ids);
    let total = photos.len();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (template, start, text, app.session.catalog.revision, &ids).hash(&mut h);
    let key = h.finish();
    let id = egui::Id::new("rename-preview-rows");
    let read = |rows: &Rows| rows.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    if let Some((k, rows)) = ctx.data(|d| d.get_temp::<(u64, Rows)>(id))
        && k == key
    {
        return (read(&rows), total);
    }
    let rows: Rows = Default::default();
    let first: Vec<_> = photos.into_iter().take(RENAME_PREVIEW_ROWS).collect();
    let (out, template, repaint) = (rows.clone(), template.to_string(), ctx.clone());
    let exists = app.session.media.availability.probe();
    let batch = lightcraft_engine::rename::Batch { total, text: lightcraft_engine::rename::Batch::clean(text), ..Default::default() };
    let work = move || {
        let plans = lightcraft_engine::rename::plan_rename_photos_in(&first, &template, start, &batch, &|f| exists(f));
        *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(plans);
        repaint.request_repaint();
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Err(e) = std::thread::Builder::new().name("lc-rename-preview".into()).spawn(work) {
        log::warn!("rename preview: {e}");
    }
    #[cfg(target_arch = "wasm32")]
    work();
    ctx.data_mut(|d| d.insert_temp(id, (key, rows.clone())));
    (read(&rows), total)
}

/// "Template" picker: Lightroom's built-in filename templates and the saved ones fill `template`;
/// save. keeps the current one under a name (asked), del. drops a saved one. Shared by Rename and
/// Export (`id` keeps their widgets apart).
pub(crate) fn naming_template_row(app: &mut LightcraftApp, ui: &mut egui::Ui, id: &str, template: &mut String) {
    let list = app.session.execute("naming.templates", &json!({})).unwrap_or_default();
    let rows = list["templates"].as_array().cloned().unwrap_or_default();
    let current = rows.iter().find(|r| r["template"].as_str() == Some(template.as_str()));
    let shown = current.and_then(|r| r["name"].as_str()).unwrap_or("Custom Settings").to_string();
    let user_current = current.filter(|r| r["builtin"] == false).and_then(|r| r["name"].as_str()).map(str::to_string);
    field(ui, "Presets", |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let r = egui::ComboBox::from_id_salt(format!("{id}-naming-template")).selected_text(shown).width(240.0).show_ui(ui, |ui| {
            for (i, row) in rows.iter().enumerate() {
                let (name, tpl) = (row["name"].as_str().unwrap_or(""), row["template"].as_str().unwrap_or(""));
                let r = ui.selectable_label(template == tpl, format!("{name}   {}", row["example"].as_str().unwrap_or("")));
                crate::widgets::register(ui.ctx(), format!("button:{id}Template-{i}"), r.rect);
                if r.clicked() {
                    *template = tpl.to_string();
                }
            }
        });
        crate::widgets::register(ui.ctx(), format!("button:{id}Templates"), r.response.rect);
        if current.is_none() && !template.trim().is_empty() {
            let r = ui.button("save.");
            crate::widgets::register(ui.ctx(), format!("button:{id}SaveTemplate"), r.rect);
            if r.clicked() {
                let t = template.clone();
                // the rename / export dialog stays open: the name is asked in a toast-free prompt
                // only from the Rename dialog; elsewhere a dated name is used
                let name = format!("Template {}", rows.len().saturating_sub(lightcraft_engine::rename::BUILTIN_TEMPLATES.len()) + 1);
                let _ = app.run("naming.saveTemplate", json!({"name": name, "template": t}));
            }
        }
        if let Some(name) = user_current {
            let r = ui.button("del.");
            crate::widgets::register(ui.ctx(), format!("button:{id}DeleteTemplate"), r.rect);
            if r.clicked() {
                let _ = app.run("naming.deleteTemplate", json!({"name": name}));
            }
        }
    });
}

/// Export ▸ Post-Processing ▸ After Export (E02): nothing. / show. (Finder) / open. in the external
/// editor or another app, or one of the Export Actions (apps / scripts in that folder).
fn after_export_row(app: &mut LightcraftApp, ui: &mut egui::Ui, opts: &mut lightcraft_engine::export::ExportOptions) {
    use lightcraft_engine::export::AfterExport as A;
    let actions = lightcraft_engine::export::export_actions();
    let editor = app.ui.settings.external_editor.trim().to_string();
    field(ui, "After export", |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let shown = match opts.after {
            A::Nothing => "nothing.".to_string(),
            A::ShowInFinder => crate::menus::reveal_label().to_lowercase().replace(' ', ".") + ".",
            A::OpenInApp if opts.after_app.is_empty() => "open.".to_string(),
            A::OpenInApp => {
                format!("open in {}", std::path::Path::new(&opts.after_app).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
            }
        };
        let r = egui::ComboBox::from_id_salt("export-after").selected_text(shown).width(220.0).show_ui(ui, |ui| {
            let mut pick = |ui: &mut egui::Ui, id: &str, label: &str, after: A, app_name: &str| {
                let on = opts.after == after && opts.after_app == app_name;
                let r = ui.selectable_label(on, label);
                crate::widgets::register(ui.ctx(), format!("button:exportAfter-{id}"), r.rect);
                if r.clicked() {
                    opts.after = after;
                    opts.after_app = app_name.to_string();
                }
            };
            pick(ui, "nothing", "Do nothing", A::Nothing, "");
            pick(ui, "show", crate::menus::reveal_label(), A::ShowInFinder, "");
            pick(ui, "open", "Open in the default app", A::OpenInApp, "");
            if !editor.is_empty() {
                pick(ui, "editor", &format!("Open in {editor}"), A::OpenInApp, &editor);
            }
            for (i, (name, path)) in actions.iter().enumerate() {
                pick(ui, &format!("action{i}"), name, A::OpenInApp, path);
            }
        });
        crate::widgets::register(ui.ctx(), "button:exportAfter", r.response.rect);
    });
    if opts.after == A::OpenInApp {
        field(ui, "Application", |ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut opts.after_app).hint_text(crate::i18n::tr("default app")).desired_width(240.0));
            crate::widgets::register(ui.ctx(), "field:exportAfterApp", r.rect);
        });
    }
}

/// About dialog tabs: (widget id suffix, label). The credits come from `crate::credits`.
pub const ABOUT_TABS: &[(&str, &str)] = &[("about", "About"), ("contributors", "Contributors"), ("models", "Models")];

/// Help ▸ What's New (docs/whats-new.md).
pub const WHATS_NEW: &str = include_str!("../../../../docs/whats-new.md");

pub fn show(app: &mut LightcraftApp, ctx: &egui::Context) {
    let Some(mut dlg) = app.ui.dialog.clone() else { return };
    if matches!(&dlg, Dialog::Import { opts } if opts.window) {
        crate::import_window::show(app, ctx);
        crate::import_window::take_pending_source(app, ctx);
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // The backdrop is an area below the dialog window (a bare `Middle` layer painter would be
    // painted after every area — i.e. over the dialog too).
    egui::Area::new(egui::Id::new("dialog-dim")).order(egui::Order::Middle).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        ui.painter().rect_filled(screen, 0.0, egui::Color32::from_black_alpha(140));
    });
    let mut close = false;
    let mut confirm = false;
    let title: String = match &dlg {
        Dialog::TextPrompt { title, .. } => title.as_str(),
        Dialog::NewAlbum { folder: true, .. } => "Create Folder",
        Dialog::NewAlbum { .. } => "Create Album",
        Dialog::RenameAlbum { .. } => "Rename Album",
        Dialog::Rename { .. } => "Rename Photos",
        Dialog::Import { .. } => "Import Photos",
        Dialog::LabelNames { .. } => "Edit Color Label Names",
        Dialog::CaptureTime { .. } => "Edit Capture Time",
        Dialog::RenameKeyword { .. } => "Rename Keyword",
        Dialog::EditKeyword(_) => "Edit Keyword Tag",
        Dialog::ImportCatalog { .. } => "Import from Another Library",
        Dialog::MergeKeywords { .. } => "Merge Keywords",
        Dialog::NewSmartAlbum { .. } => "Create Smart Album",
        Dialog::AllMetadata { .. } => "All Metadata",
        Dialog::SystemInfo { .. } => "System Info",
        Dialog::WhatsNew => "What's New",
        Dialog::Cull { .. } => "Assisted Culling",
        Dialog::SmartRules { id: None, .. } => "New Smart Album",
        Dialog::SmartRules { .. } => "Edit Smart Album",
        Dialog::AutoStack { .. } => "Auto-Stack by Capture Time",
        Dialog::CreatePreset { .. } => "Create Preset",
        Dialog::CopySettings { .. } => "Choose Edit Settings to Copy",
        Dialog::SyncSettings { .. } => "Synchronize Settings",
        Dialog::PasteSettings { .. } => "Paste Selected Settings",
        Dialog::PasteMasks { .. } => "Paste Masks",
        Dialog::Export { .. } => "Export",
        Dialog::Merge { opts } => opts.title(),
        Dialog::Settings { .. } => "Settings",
        Dialog::ConfirmDelete { .. } => "Delete Photos",
        Dialog::DeletePhotos { rejected: true, .. } => "Delete Rejected Photos",
        Dialog::DeletePhotos { count, copies, .. } if count == copies => "Remove Virtual Copies",
        Dialog::DeletePhotos { .. } => "Delete Photos",
        Dialog::RemoveFolder { disk: true, .. } => "Remove Disk from Library",
        Dialog::RemoveFolder { .. } => "Remove Folder from Library",
        Dialog::SamModel { .. } => "Download the SAM 3 Model?",
        Dialog::Backups { .. } => "Catalog Backups",
        Dialog::About => "About LightCraft",
        Dialog::Shortcuts => "Keyboard Shortcuts",
        Dialog::LightroomSync { .. } => "Sync from Lightroom Classic",
    }
    .to_string();
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::symmetric(16, 12));
    // The import review can be resized (its photo grid takes the room); it keeps a window id of its
    // own so the size it is given doesn't carry over to the other dialogs. Its content scrolls
    // rather than growing the window when the options below the grid get taller (e.g. Copy).
    let import = matches!(dlg, Dialog::Import { .. });
    let window_id = egui::Id::new(if import { "lightcraft-import-dialog" } else { "lightcraft-dialog" });
    let shown = egui::Window::new(crate::i18n::tr(&title)).id(window_id)
        .collapsible(false)
        .resizable(import)
        .vscroll(import)
        .resize(|r| if import { r.default_height(crate::import::DIALOG_SIZE[1]) } else { r })
        .frame(frame)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .default_width(match dlg {
            Dialog::Import { .. } => crate::import::DIALOG_SIZE[0],
            Dialog::SmartRules { .. } => 680.0,
            Dialog::AllMetadata { .. } => 620.0,
            Dialog::CreatePreset { .. } | Dialog::CopySettings { .. } | Dialog::SyncSettings { .. } | Dialog::PasteSettings { .. } => 640.0,
            _ => 380.0,
        })
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            match &mut dlg {
                Dialog::AutoStack { gap } => {
                    ui.label(egui::RichText::new(crate::i18n::tr("Stack photos taken within this time of each other:")).color(t.text_label));
                    ui.add(
                        egui::Slider::new(gap, 0.0..=86400.0)
                            .logarithmic(true)
                            .smallest_positive(1.0)
                            .custom_formatter(|v, _| crate::panels::dialogs::fmt_gap(v))
                            .custom_parser(|s| s.trim().trim_end_matches('s').parse().ok()),
                    );
                    let preview = app.session.execute("stack.auto", &json!({"gap": *gap, "preview": true})).unwrap_or_default();
                    let scope = if app.session.selection.ids.len() > 1 { "the selected photos" } else { "the photos in view" };
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!("Creates {} stacks from {} of {scope}", preview["stacks"], preview["photos"], scope = crate::i18n::tr(scope))).color(t.text_dim),
                    );
                }
                Dialog::Cull { reject_below, pick_best } => {
                    let n = app.session.selection.ids.len();
                    let scope = if n > 1 { crate::i18n::tr_format!("the {n} selected photos", n = n) } else { crate::i18n::tr_format!("the {} photos in view", app.session.visible_cloned().len()) };
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Scores {scope} for focus and exposure and finds similar shots taken within seconds of each other (bursts).", scope = scope)).color(t.text_label));
                    ui.add_space(6.0);
                    let r = ui.add(egui::Slider::new(reject_below, 0.0..=80.0).text(crate::i18n::tr("Reject below focus")).step_by(1.0));
                    crate::widgets::register(ui.ctx(), "field:cullReject", r.rect);
                    ui.label(egui::RichText::new(crate::i18n::tr(if *reject_below > 0.0 { "Blurry photos below the score are flagged as rejects." } else { "0: nothing is rejected." })).color(t.text_dim));
                    let r = ui.checkbox(pick_best, crate::i18n::tr("Pick the sharpest photo of each burst"));
                    crate::widgets::register(ui.ctx(), "check:cullPick", r.rect);
                    ui.label(egui::RichText::new(crate::i18n::tr("Scores stay on the photos: filter or make smart albums with Focus and Best of Similar Shots.")).color(t.text_dim));
                }
                Dialog::WhatsNew => {
                    egui::ScrollArea::vertical().max_height(460.0).auto_shrink([false, true]).show(ui, |ui| {
                        for line in WHATS_NEW.lines() {
                            let l = line.trim_end();
                            if let Some(h) = l.strip_prefix("### ") {
                                ui.add_space(6.0);
                                ui.label(egui::RichText::new(crate::i18n::tr(h)).font(t.semibold(12.5)).color(t.text));
                            } else if let Some(h) = l.strip_prefix("## ") {
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new(crate::i18n::tr(h)).font(t.semibold(14.0)).color(t.text));
                            } else if l.starts_with("# ") || l.is_empty() {
                            } else if let Some(b) = l.strip_prefix("- ") {
                                ui.label(egui::RichText::new(format!("•  {}", crate::i18n::tr(b).replace('`', ""))).color(t.text_label));
                            } else {
                                ui.label(egui::RichText::new(crate::i18n::tr(l.trim()).replace('`', "")).color(t.text_label));
                            }
                        }
                    });
                }
                Dialog::SystemInfo { rows } => {
                    egui::Grid::new("sysinfo").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                        for (k, v) in rows.iter() {
                            ui.label(egui::RichText::new(crate::i18n::tr(k)).color(t.text_dim));
                            ui.add(egui::Label::new(egui::RichText::new(v).color(t.text_label)).wrap());
                            ui.end_row();
                        }
                    });
                    if ui.button(crate::i18n::tr("Copy to Clipboard")).clicked() {
                        let text: String = rows.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
                        ui.ctx().copy_text(text);
                    }
                }
                Dialog::AllMetadata { title, rows, search } => {
                    ui.label(egui::RichText::new(title.as_str()).color(t.text_label));
                    let r = ui.add(egui::TextEdit::singleline(search).hint_text(crate::i18n::tr("Filter fields")).desired_width(f32::INFINITY));
                    crate::widgets::register(ui.ctx(), "field:metadataSearch", r.rect);
                    let q = search.trim().to_lowercase();
                    let keep = |n: &str, v: &str| q.is_empty() || n.to_lowercase().contains(&q) || v.to_lowercase().contains(&q);
                    let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();
                    for row in rows["exif"].as_array().into_iter().flatten() {
                        let (g, n, v) = (row["group"].as_str().unwrap_or(""), row["name"].as_str().unwrap_or(""), row["value"].as_str().unwrap_or(""));
                        if !keep(n, v) {
                            continue;
                        }
                        match groups.iter_mut().find(|(x, _)| x == g) {
                            Some((_, list)) => list.push((n.into(), v.into())),
                            None => groups.push((g.into(), vec![(n.into(), v.into())])),
                        }
                    }
                    let xmp: Vec<(String, String)> = rows["xmp"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| (r["name"].as_str().unwrap_or("").to_string(), r["value"].as_str().unwrap_or("").to_string()))
                        .filter(|(n, v)| keep(n, v))
                        .collect();
                    if !xmp.is_empty() {
                        groups.push(("XMP".into(), xmp));
                    }
                    egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                        if groups.is_empty() {
                            ui.label(egui::RichText::new(crate::i18n::tr(rows["note"].as_str().unwrap_or("No metadata found"))).color(t.text_dim));
                        }
                        for (g, list) in &groups {
                            ui.add_space(6.0);
                            ui.label(egui::RichText::new(g).font(t.semibold(12.5)).color(t.text));
                            egui::Grid::new(format!("meta-{g}")).num_columns(2).spacing([16.0, 3.0]).striped(true).show(ui, |ui| {
                                for (n, v) in list {
                                    ui.label(egui::RichText::new(n).color(t.text_dim));
                                    ui.add(egui::Label::new(egui::RichText::new(v).color(t.text_label)).wrap());
                                    ui.end_row();
                                }
                            });
                        }
                    });
                }
                Dialog::SmartRules { id, name, rules } => {
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text(crate::i18n::tr("Name")).desired_width(f32::INFINITY));
                    crate::widgets::register(ui.ctx(), "field:smartName", r.rect);
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show(ui, |ui| {
                        crate::panels::rules_editor::edit(ui, rules, "rules", 0);
                    });
                    let problems = rules.problems();
                    // the folder an album made from a folder view carries is not in the editor, but it counts
                    let folder = id.and_then(|id| app.session.catalog.album(lightcraft_catalog::AlbumId(id))).and_then(|a| a.smart.as_deref().and_then(|f| f.library_folder.clone()));
                    let f = lightcraft_catalog::Filter { rule_set: Some(rules.clone()), library_folder: folder, ..Default::default() };
                    let n = if problems.is_empty() { app.session.catalog.query(&f, &Default::default()).len() } else { 0 };
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(match problems.first() {
                            Some(p) => p.clone(),
                            None => crate::i18n::tr_format!("{n} photo{} match · updates automatically as photos change", if n == 1 { "" } else { "s" }, n = n),
                        })
                        .color(t.text_dim),
                    );
                }
                Dialog::NewSmartAlbum { name } => {
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text(crate::i18n::tr("Name")).desired_width(f32::INFINITY));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                    let rules = app.session.view_rules();
                    let n = app.session.catalog.query(&rules, &Default::default()).len();
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Matches: {}", crate::i18n::filter_label(&rules, &app.session.catalog))).color(t.text_label));
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!("{n} photo{} now · updates automatically as photos change", if n == 1 { "" } else { "s" }, n = n))
                            .color(t.text_dim),
                    );
                }
                Dialog::CaptureTime { mode, time, days, hours, minutes, zone } => {
                    let n = app.session.targets(&json!({})).len();
                    field(ui, "Change", |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, (label, m)) in
                            [("Set date & time", "set"), ("Shift by", "shift"), ("Time zone", "zone"), ("original.", "original"), ("file.date.", "file")].iter().enumerate()
                        {
                            if crate::widgets::text_button(ui, &format!("captureMode-{i}"), label, mode == m).clicked() {
                                *mode = m.to_string();
                            }
                        }
                    });
                    let params = match mode.as_str() {
                        "set" => {
                            field(ui, "New time", |ui| {
                                let r = ui.add(egui::TextEdit::singleline(time).hint_text("2026-09-30 14:05:00").desired_width(f32::INFINITY));
                                crate::widgets::register(ui.ctx(), "field:captureTime", r.rect);
                            });
                            json!({"time": time})
                        }
                        "shift" => {
                            field(ui, "Shift", |ui| {
                                ui.add(egui::DragValue::new(days).suffix(" d"));
                                ui.add(egui::DragValue::new(hours).suffix(" h"));
                                ui.add(egui::DragValue::new(minutes).suffix(" min"));
                            });
                            json!({"shift": *days as i64 * 86_400 + *hours as i64 * 3600 + *minutes as i64 * 60})
                        }
                        "original" | "file" => {
                            ui.label(
                                egui::RichText::new(crate::i18n::tr(if mode == "original" {
                                    "Each photo goes back to the capture time its file records."
                                } else {
                                    "Each photo takes its file's creation date (UTC)."
                                }))
                                .color(t.text_dim),
                            );
                            json!({})
                        }
                        _ => {
                            field(ui, "Hours", |ui| ui.add(egui::DragValue::new(zone).range(-26.0..=26.0).speed(0.25).fixed_decimals(2)));
                            json!({"hours": zone})
                        }
                    };
                    // preview on the active photo (the others move by the same amount)
                    let active = app.session.active().and_then(|id| app.session.catalog.photo(id)).map(|p| (p.file_name.clone(), p.date().to_string()));
                    if let Some((name, cur)) = active.filter(|_| !matches!(mode.as_str(), "original" | "file")) {
                        let delta = match mode.as_str() {
                            "set" => lightcraft_catalog::dates::normalize_iso(time)
                                .and_then(|t| Some(lightcraft_catalog::dates::iso_seconds(&t)? - lightcraft_catalog::dates::iso_seconds(&cur)?)),
                            "shift" => params["shift"].as_i64(),
                            _ => Some((*zone as f64 * 3600.0).round() as i64),
                        };
                        let after = delta.and_then(|d| lightcraft_catalog::dates::shift_iso(&cur, d));
                        ui.label(
                            egui::RichText::new(match after {
                                Some(a) => format!("{name}: {} → {}", cur.replace('T', " "), a.replace('T', " ")),
                                None => "Enter a date as YYYY-MM-DD HH:MM:SS".into(),
                            })
                            .color(t.text_label),
                        );
                    }
                    if n > 1 {
                        ui.label(egui::RichText::new(crate::i18n::tr_format!("All {n} selected photos move by the same amount.", n = n)).color(t.text_dim));
                    }
                }
                Dialog::LabelNames { names, save_as } => {
                    names.resize(5, String::new());
                    // start from a set
                    let sets = lightcraft_engine::cmd::manage::label_sets_json(&app.session);
                    field(ui, "Set", |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for set in sets["sets"].as_array().into_iter().flatten() {
                            let name = set["name"].as_str().unwrap_or_default();
                            if crate::widgets::text_button(ui, &format!("labelSet-{name}"), name, false).clicked() {
                                *names = set["names"].as_array().into_iter().flatten().map(|n| n.as_str().unwrap_or_default().to_string()).collect();
                            }
                        }
                    });
                    for (i, l) in lightcraft_catalog::ColorLabel::ALL.iter().enumerate() {
                        let colour = format!("{l:?}");
                        field(ui, &colour, |ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            ui.painter().circle_filled(r.center(), 6.0, crate::panels::grid::label_color(*l));
                            let te = ui.add(egui::TextEdit::singleline(&mut names[i]).hint_text(colour.as_str()).desired_width(f32::INFINITY));
                            crate::widgets::register(ui.ctx(), format!("field:labelName-{}", colour.to_lowercase()), te.rect);
                        });
                    }
                    field(ui, "Save as set", |ui| {
                        let te = ui.add(egui::TextEdit::singleline(save_as).hint_text(crate::i18n::tr("Optional name")).desired_width(f32::INFINITY));
                        crate::widgets::register(ui.ctx(), "field:labelSetName", te.rect);
                    });
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("Names appear in the label menu, the filter bar and the Info panel, and are written to XMP. Empty = the colour's name."))
                            .color(t.text_dim),
                    );
                }
                Dialog::Rename { template, start, text } => {
                    let n = app.session.targets(&json!({})).len();
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("{n} photo{}", if n == 1 { "" } else { "s" }, n = n)).color(t.text_dim));
                    let template_id = egui::Id::new("rename-template");
                    let tags_open = field(ui, "Template", |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let w = crate::import::tag_field_width(ui);
                        let r = ui.add(egui::TextEdit::singleline(template).id(template_id).hint_text("{name}").desired_width(w));
                        crate::widgets::register(ui.ctx(), "field:renameTemplate", r.rect);
                        crate::import::tag_toggle(ui, "renameTemplate")
                    });
                    if tags_open {
                        crate::import::tag_help(ui, "renameTemplate", template, template_id);
                    }
                    crate::import::unknown_tags_warning(ui, template);
                    naming_template_row(app, ui, "rename", template);
                    field(ui, "Custom Text", |ui| {
                        let r = ui.add(egui::TextEdit::singleline(text).hint_text("{text}").desired_width(200.0));
                        crate::widgets::register(ui.ctx(), "field:renameText", r.rect);
                    });
                    field(ui, "Start at", |ui| ui.add(egui::DragValue::new(start).range(0..=999_999)));
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("Tags: see Tags beside the template. Files are renamed on disk (with their XMP sidecars); existing names get -1, -2…"))
                            .color(t.text_dim),
                    );
                    let (preview, total) = rename_preview(app, ui.ctx(), template, *start as usize, text);
                    match &preview {
                        Some(rows) => {
                            egui::Grid::new("rename-preview").num_columns(3).spacing([8.0, 2.0]).show(ui, |ui| {
                                for pl in rows {
                                    ui.label(egui::RichText::new(&pl.from).color(t.text_dim));
                                    ui.label(egui::RichText::new("→").color(t.text_dim));
                                    ui.label(egui::RichText::new(&pl.to).color(t.text));
                                    ui.end_row();
                                }
                            });
                        }
                        None => {
                            ui.label(egui::RichText::new(crate::i18n::tr("Checking names…")).color(t.text_dim));
                        }
                    }
                    let more = total.saturating_sub(RENAME_PREVIEW_ROWS);
                    if more > 0 {
                        ui.label(egui::RichText::new(crate::i18n::tr_format!("… and {more} more", more = more)).color(t.text_dim));
                    }
                }
                Dialog::ImportCatalog { path, preview, replace } => {
                    ui.label(egui::RichText::new(path.as_str()).color(t.text_label));
                    ui.add_space(4.0);
                    let n = |k: &str| preview[k].as_u64().unwrap_or(0);
                    ui.label(format!(
                        "{} photos in it: {} new, {} this library has already; {} albums",
                        n("photos"),
                        n("added"),
                        n("inBoth"),
                        n("albums")
                    ));
                    ui.add_space(6.0);
                    let mut v: &'static str = match replace.as_str() {
                        "metadata" => "metadata",
                        "all" => "all",
                        _ => "none",
                    };
                    choices(ui, "Photos both have", "importCatalogReplace", &[("none", "keep.mine."), ("metadata", "take.metadata."), ("all", "take.all.")], &mut v);
                    *replace = v.to_string();
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("take.all. also takes the other library's develop settings; the other library is only read, never changed."))
                            .size(11.0)
                            .color(t.text_dim),
                    );
                }
                Dialog::EditKeyword(e) => {
                    field(ui, "Keyword name", |ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(f32::INFINITY));
                        crate::widgets::register(ui.ctx(), "field:editKeywordName", r.rect);
                    });
                    field(ui, "Synonyms", |ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut e.synonyms).hint_text(crate::i18n::tr("commas between them")).desired_width(f32::INFINITY));
                        crate::widgets::register(ui.ctx(), "field:editKeywordSynonyms", r.rect);
                    });
                    ui.add_space(4.0);
                    for (id, label, v) in [
                        ("include", "Include on Export", &mut e.include_on_export),
                        ("containing", "Export Containing Keywords", &mut e.export_containing),
                        ("synonyms", "Export Synonyms", &mut e.export_synonyms),
                        ("person", "Person", &mut e.person),
                    ] {
                        let r = ui.checkbox(v, crate::i18n::tr(label));
                        crate::widgets::register(ui.ctx(), format!("check:editKeyword-{id}"), r.rect);
                    }
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("A keyword left out of export stays in the library; a person keyword is left out when an export removes person info."))
                            .size(11.0)
                            .color(t.text_dim),
                    );
                }
                Dialog::RenameKeyword { from, to } => {
                    let n = app.session.catalog.photos().filter(|p| p.meta.keywords.iter().any(|k| lightcraft_catalog::keywords::is_under(k, from))).count();
                    let r = ui.add(egui::TextEdit::singleline(to).hint_text(crate::i18n::tr("New name")).desired_width(f32::INFINITY));
                    crate::widgets::register(ui.ctx(), "field:keywordName", r.rect);
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!("Renames “{from}” on {n} photo{} (keywords below it too). Use | for levels, e.g. Travel|Italy. An existing name merges the two.",
                            if n == 1 { "" } else { "s" }, from = from, n = n))
                        .color(t.text_dim),
                    );
                }
                Dialog::MergeKeywords { from, into } => {
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Replace {} with:", from.iter().map(|f| format!("“{f}”")).collect::<Vec<_>>().join(", "))).color(t.text_label));
                    let r = ui.add(egui::TextEdit::singleline(into).hint_text(crate::i18n::tr("Keyword")).desired_width(f32::INFINITY));
                    crate::widgets::register(ui.ctx(), "field:keywordInto", r.rect);
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                    let options: Vec<String> = app
                        .session
                        .catalog
                        .keyword_suggestions(from, into, 12)
                        .into_iter()
                        .filter(|k| !from.iter().any(|f| lightcraft_catalog::keywords::is_under(k, f)))
                        .collect();
                    ui.horizontal_wrapped(|ui| {
                        for k in options {
                            if ui.add(egui::Button::new(egui::RichText::new(&k).color(t.text_label)).corner_radius(10.0)).clicked() {
                                *into = k;
                            }
                        }
                    });
                }
                Dialog::TextPrompt { value, hint, .. } => {
                    let r = ui.add(egui::TextEdit::singleline(value).hint_text(crate::i18n::tr(hint.as_str())).desired_width(f32::INFINITY));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                }
                Dialog::NewAlbum { name, .. } | Dialog::RenameAlbum { name, .. } => {
                    let r = ui.add(egui::TextEdit::singleline(name).hint_text(crate::i18n::tr("Name")).desired_width(f32::INFINITY));
                    r.request_focus();
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        confirm = true;
                    }
                }
                Dialog::CreatePreset { name, group, groups, masks } => {
                    ui.add(egui::TextEdit::singleline(name).hint_text(crate::i18n::tr("Preset name")).desired_width(f32::INFINITY));
                    ui.add(egui::TextEdit::singleline(group).hint_text(crate::i18n::tr("Group")).desired_width(f32::INFINITY));
                    ui.label(egui::RichText::new(crate::i18n::tr("Settings to include")).color(t.text_dim));
                    group_checklist(app, ui, "presetInclude", groups, Some(masks));
                }
                Dialog::CopySettings { groups, masks, mode, step } => {
                    group_checklist(app, ui, "copyGroup", groups, Some(masks));
                    paste_mode_row(app, ui, mode, step);
                }
                Dialog::SyncSettings { groups, match_exposure, auto_align, match_color, masks } => {
                    let n = app.session.selection.ids.len().saturating_sub(1);
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Copy to {n} other selected photo{}", if n == 1 { "" } else { "s" }, n = n)).color(t.text_dim));
                    group_checklist(app, ui, "syncGroup", groups, Some(masks));
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(crate::i18n::tr("Then, on each photo")).color(t.text_dim));
                    let r = ui.checkbox(match_color, crate::i18n::tr("Match camera & lens color")).on_hover_text(crate::i18n::tr(
                        "Make each photo's colour match this one when it was shot on another camera, lens or phone: a colour chart through both cameras' own colour profiles sets White Balance, HSL, Calibration and Saturation (ordinary sliders you can still change); lens and phone casts are learned from your own edits; photos shot within 30 minutes of this one also even out their neutrals",
                    ));
                    crate::widgets::register(ui.ctx(), "syncGroup:matchColor", r.rect);
                    let r = ui.checkbox(match_exposure, crate::i18n::tr("Match exposure")).on_hover_text(crate::i18n::tr(
                        "Set each photo's Exposure so it comes out as bright as this one — measured on the finished look, so photos shot 4 stops under or 3 over all land on it",
                    ));
                    crate::widgets::register(ui.ctx(), "syncGroup:matchExposure", r.rect);
                    let r = ui
                        .checkbox(auto_align, crate::i18n::tr("Auto align (horizon)"))
                        .on_hover_text(crate::i18n::tr("Level each photo's horizon (or plumb lines) with its crop angle"));
                    crate::widgets::register(ui.ctx(), "syncGroup:autoAlign", r.rect);
                    // remembered as checked, even when the dialog is cancelled
                    let now = crate::state::SyncPrefs { groups: Some(groups.clone()), match_exposure: *match_exposure, auto_align: *auto_align, match_color: *match_color };
                    if app.ui.sync_prefs != now {
                        app.ui.sync_prefs = now;
                    }
                }
                Dialog::PasteMasks { merge, .. } => {
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("The copied settings have masks, and photos you're pasting onto have masks of their own."))
                            .color(t.text_label),
                    );
                    mask_choice(ui, "pasteMasks", merge);
                }
                Dialog::PasteSettings { groups, merge_masks } => {
                    let n = app.session.targets(&json!({})).len();
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!("Paste into {n} photo{} — only settings that were copied are pasted", if n == 1 { "" } else { "s" }, n = n))
                            .color(t.text_dim),
                    );
                    group_checklist(app, ui, "pasteGroup", groups, None);
                    if groups.iter().any(|g| g == "masks") && crate::menus::paste_meets_masks(app, &json!({})) {
                        ui.add_space(4.0);
                        mask_choice(ui, "pasteGroup", merge_masks);
                    }
                }
                Dialog::Export { opts, full_size, resize, preset_name, limit_kb, dir } => {
                    // taller than a small window: the options scroll inside, the buttons stay in view
                    let room = (screen.height() - 170.0).max(220.0);
                    egui::ScrollArea::vertical().id_salt("exportScroll").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
                    use lightcraft_engine::export::{Anchor as P, ExportFormat as F, MetadataPolicy as M, SharpenAmount as A, SharpenFor as S};
                    let n = app.session.selection.ids.len().max(1);
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("{n} photo{}", if n == 1 { "" } else { "s" }, n = n)).color(t.text_dim));
                    // Preset: load a built-in or saved set of options into the dialog
                    field(ui, "Preset", |ui| {
                        let mut chosen = None;
                        let mut delete = None;
                        let c = egui::ComboBox::from_id_salt("exportPreset").width(220.0).selected_text(crate::i18n::tr("Choose…")).show_ui(ui, |ui| {
                            let mut builtin = true;
                            for (i, (p, b)) in app.session.all_export_presets().into_iter().enumerate() {
                                if builtin && !b {
                                    ui.separator();
                                }
                                builtin = b;
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    if ui.selectable_label(false, crate::i18n::builtin_label(&p.name, b)).clicked() {
                                        chosen = Some(p.name.clone());
                                    }
                                    // a saved preset can be deleted right here (E13)
                                    if !b {
                                        let r = ui.small_button("del.");
                                        crate::widgets::register(ui.ctx(), format!("button:exportPresetDelete-{i}"), r.rect);
                                        if r.clicked() {
                                            delete = Some(p.name.clone());
                                        }
                                    }
                                });
                            }
                        });
                        if let Some(name) = delete {
                            match app.run("export.deletePreset", json!({"name": name})) {
                                Ok(_) => app.toast(ui.ctx(), format!("Deleted export preset “{name}”")),
                                Err(e) => app.toast(ui.ctx(), e),
                            }
                        }
                        crate::widgets::register(ui.ctx(), "combo:exportPreset", c.response.rect);
                        if let Some(name) = chosen
                            && let Ok(params) = app.session.export_params(&json!({"preset": name}))
                        {
                            let o = lightcraft_engine::export::ExportOptions::from_json(&params);
                            *full_size = o.resize.is_none();
                            *resize = o.resize.unwrap_or_default();
                            *limit_kb = o.limit_kb.unwrap_or(0);
                            *opts = o;
                        }
                    });
                    // E05: several presets at once — pick them (click = just this one, ⌘-click adds
                    // or removes one, ⇧-click the run from the last), each exports into a folder
                    // named after it under the chosen folder
                    let bid = egui::Id::new("export-batch-presets");
                    let (mut picked, mut last): (Vec<String>, Option<usize>) = ui.data(|d| d.get_temp(bid)).unwrap_or_default();
                    let names: Vec<String> = app.session.all_export_presets().into_iter().map(|(p, _)| p.name).collect();
                    field(ui, "Several", |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                            let mods = ui.input(|i| i.modifiers);
                            for (i, n) in names.iter().enumerate() {
                                let on = picked.contains(n);
                                if crate::widgets::text_button(ui, &format!("exportBatch-{i}"), n, on).clicked() {
                                    if mods.shift && let Some(a) = last {
                                        let (lo, hi) = (a.min(i), a.max(i));
                                        for m in names.iter().take(hi + 1).skip(lo) {
                                            if !picked.contains(m) {
                                                picked.push(m.clone());
                                            }
                                        }
                                    } else if mods.command {
                                        if on {
                                            picked.retain(|x| x != n);
                                        } else {
                                            picked.push(n.clone());
                                        }
                                    } else {
                                        picked = vec![n.clone()];
                                    }
                                    last = Some(i);
                                }
                            }
                            if crate::widgets::text_button(ui, "exportBatchAll", "all.", false).clicked() {
                                picked = names.clone();
                            }
                            if crate::widgets::text_button(ui, "exportBatchNone", "none.", false).clicked() {
                                picked.clear();
                            }
                        });
                    });
                    if !picked.is_empty() {
                        field(ui, "", |ui| {
                            let label = format!("export.{}.presets.", picked.len());
                            if crate::widgets::text_button(ui, "exportBatchRun", &label, false)
                                .on_hover_text(crate::i18n::tr("Export with each checked preset, each into a folder named after it"))
                                .clicked()
                            {
                                let order: Vec<String> = names.iter().filter(|n| picked.contains(n)).cloned().collect();
                                let mut params = json!({"presets": order});
                                if !dir.trim().is_empty() {
                                    params["dir"] = json!(dir.trim());
                                }
                                match app.run("app.export", params) {
                                    Ok(v) => {
                                        let n = v["presets"].as_array().map_or(0, Vec::len);
                                        app.toast(ui.ctx(), format!("Exported with {n} presets"));
                                        close = true;
                                    }
                                    Err(e) => app.toast(ui.ctx(), e),
                                }
                            }
                        });
                    }
                    ui.data_mut(|d| d.insert_temp(bid, (picked, last)));
                    ui.add_space(4.0);
                    let before = opts.format;
                    choices(
                        ui,
                        "Format",
                        "exportFormat",
                        &[(F::Jpeg, "JPEG"), (F::Png, "PNG"), (F::Tiff, "TIFF"), (F::Webp, "WebP"), (F::Avif, "AVIF"), (F::Dng, "DNG"), (F::Original, "Original files"), (F::Psd, "PSD"), (F::Jxl, "JPEG XL")],
                        &mut opts.format,
                    );
                    if !opts.format.is_rendered() {
                        let note = if opts.format == F::Dng {
                            "Raw photos as DNG, with the edits embedded. Size, color and output options don't apply."
                        } else {
                            "The original files, unchanged, each with an XMP sidecar holding its edits."
                        };
                        ui.label(egui::RichText::new(note).color(t.text_dim));
                    }
                    if opts.format != before {
                        // each format starts at its own default depth (TIFF 16-bit, others 8-bit)
                        opts.bit_depth = None;
                    }
                    let rendered = opts.format.is_rendered();
                    let depths = lightcraft_engine::export::ExportOptions::bit_depths(opts.format);
                    if rendered && depths.len() > 1 {
                        let mut bd = opts.bit_depth.filter(|b| depths.iter().any(|d| d.0 == *b)).unwrap_or(depths[0].0);
                        choices(ui, "Bit depth", "exportBitDepth", depths, &mut bd);
                        opts.bit_depth = Some(bd);
                    }
                    if !rendered {
                    } else if opts.format == F::Avif {
                        ui.label(egui::RichText::new(crate::i18n::tr("Color space: sRGB (AVIF)")).color(t.text_dim));
                    } else {
                        use lightcraft_engine::export::OutputSpace as C;
                        choices(
                            ui,
                            "Color space",
                            "exportColorSpace",
                            &[
                                (C::Srgb, "sRGB"),
                                (C::DisplayP3, "P3"),
                                (C::AdobeRgb, "Adobe RGB"),
                                (C::ProPhoto, "ProPhoto"),
                                (C::Rec2020, "Rec.2020"),
                            ],
                            &mut opts.color_space,
                        );
                    }
                    // E08: convert to another profile (ColorMatch RGB, or any RGB profile on this Mac)
                    if matches!(opts.format, F::Jpeg | F::Png | F::Tiff | F::Psd) {
                        field(ui, "Profile", |ui| {
                            let shown = match opts.icc_profile.as_deref() {
                                None => crate::i18n::tr("the space above").to_string(),
                                Some(x) if x.eq_ignore_ascii_case("colorMatch") => "ColorMatch RGB".to_string(),
                                Some(x) => std::path::Path::new(x).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| x.to_string()),
                            };
                            let key = egui::Id::new("export-installed-profiles");
                            let installed: Vec<String> = match ui.data(|d| d.get_temp::<Vec<String>>(key)) {
                                Some(v) => v,
                                None => {
                                    let v = lightcraft_engine::export::installed_profiles();
                                    ui.data_mut(|d| d.insert_temp(key, v.clone()));
                                    v
                                }
                            };
                            let r = egui::ComboBox::from_id_salt("exportIccProfile").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                                if ui.selectable_label(opts.icc_profile.is_none(), crate::i18n::tr("the space above")).clicked() {
                                    opts.icc_profile = None;
                                }
                                if ui.selectable_label(opts.icc_profile.as_deref() == Some("colorMatch"), "ColorMatch RGB").clicked() {
                                    opts.icc_profile = Some("colorMatch".into());
                                }
                                ui.separator();
                                for path in &installed {
                                    let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    if ui.selectable_label(opts.icc_profile.as_deref() == Some(path.as_str()), name).clicked() {
                                        opts.icc_profile = Some(path.clone());
                                    }
                                }
                            });
                            crate::widgets::register(ui.ctx(), "combo:exportIccProfile", r.response.rect);
                        });
                    }
                    if matches!(opts.format, F::Jpeg | F::Avif | F::Jxl) {
                        let mut q = opts.quality as f64;
                        if num(ui, &QUALITY, &mut q) {
                            opts.quality = q as u8;
                        }
                    }
                    if opts.format == F::Jpeg {
                        let mut k = *limit_kb as f64;
                        if num(ui, &LIMIT_KB, &mut k) {
                            *limit_kb = k as u32;
                        }
                    }
                    if rendered {
                        export_size(ui, full_size, resize, &mut opts.ppi);
                    }
                    if rendered {
                    choices(
                        ui,
                        "Sharpen",
                        "exportSharpen",
                        &[(S::None, "None"), (S::Screen, "Screen"), (S::Matte, "Matte"), (S::Glossy, "Glossy")],
                        &mut opts.sharpen,
                    );
                    if opts.sharpen != S::None {
                        choices(
                            ui,
                            "Amount",
                            "exportSharpenAmount",
                            &[(A::Low, "Low"), (A::Standard, "Standard"), (A::High, "High")],
                            &mut opts.sharpen_amount,
                        );
                    }
                    }
                    if rendered {
                    choices(
                        ui,
                        "Metadata",
                        "exportMetadata",
                        &[(M::All, "All"), (M::AllExceptCamera, "No camera"), (M::Copyright, "Copyright"), (M::None, "None")],
                        &mut opts.metadata,
                    );
                    if !matches!(opts.metadata, M::None | M::Copyright) {
                        ui.checkbox(&mut opts.remove_location, crate::i18n::tr("Remove location info"));
                        let r = ui.checkbox(&mut opts.remove_people, crate::i18n::tr("Remove person info"));
                        crate::widgets::register(ui.ctx(), "check:exportRemovePeople", r.rect);
                        let r = ui.checkbox(&mut opts.keyword_hierarchy, crate::i18n::tr("Write keywords as Lightroom hierarchy"));
                        crate::widgets::register(ui.ctx(), "check:exportKeywordHierarchy", r.rect);
                    }
                    let mut wm_on = opts.watermark.is_some();
                    if ui.checkbox(&mut wm_on, crate::i18n::tr("Watermark")).changed() {
                        opts.watermark = wm_on.then(|| lightcraft_engine::export::Watermark { text: "© ".into(), ..Default::default() });
                    }
                    if opts.watermark.is_some() {
                        // named watermarks (E09): pick one, save the current one
                        field(ui, "Watermark", |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            let saved = app.session.watermark_presets.clone();
                            let r = egui::ComboBox::from_id_salt("export-watermark-preset").selected_text(crate::i18n::tr("Saved…")).width(160.0).show_ui(ui, |ui| {
                                for (i, w) in saved.iter().enumerate() {
                                    let r = ui.selectable_label(opts.watermark.as_ref() == Some(&w.watermark), &w.name);
                                    crate::widgets::register(ui.ctx(), format!("button:watermarkPreset-{i}"), r.rect);
                                    if r.clicked() {
                                        opts.watermark = Some(w.watermark.clone());
                                    }
                                }
                                if saved.is_empty() {
                                    ui.label(crate::i18n::tr("No saved watermarks yet"));
                                }
                            });
                            crate::widgets::register(ui.ctx(), "button:watermarkPresets", r.response.rect);
                            let nid = egui::Id::new("watermark-preset-name");
                            let mut name = ui.data(|d| d.get_temp::<String>(nid)).unwrap_or_default();
                            let f = ui.add(egui::TextEdit::singleline(&mut name).hint_text(crate::i18n::tr("name")).desired_width(110.0));
                            crate::widgets::register(ui.ctx(), "field:watermarkPresetName", f.rect);
                            let s = ui.add_enabled(!name.trim().is_empty(), egui::Button::new("save."));
                            crate::widgets::register(ui.ctx(), "button:watermarkPresetSave", s.rect);
                            if s.clicked() {
                                let w = serde_json::to_value(&opts.watermark).unwrap_or_default();
                                match app.run("watermark.save", json!({"name": name.trim(), "watermark": w})) {
                                    Ok(_) => name.clear(),
                                    Err(e) => app.toast(ui.ctx(), e),
                                }
                            }
                            ui.data_mut(|d| d.insert_temp(nid, name));
                        });
                    }
                    if let Some(wm) = &mut opts.watermark {
                        // text, or a graphic (a logo with transparency)
                        let mut graphic = !wm.image.is_empty() || ui.data(|d| d.get_temp::<bool>(egui::Id::new("wm-graphic"))).unwrap_or(false);
                        let before = graphic;
                        choices(ui, "Style", "exportWmStyle", &[(false, "Text"), (true, "Graphic")], &mut graphic);
                        if graphic != before {
                            ui.data_mut(|d| d.insert_temp(egui::Id::new("wm-graphic"), graphic));
                            if !graphic {
                                wm.image.clear();
                            }
                        }
                        if graphic {
                            field(ui, "Graphic", |ui| {
                                trailing_button_row(ui, |ui| {
                                    if app.services.pick_files.is_some()
                                        && crate::widgets::text_button(ui, "exportWmChoose", crate::i18n::tr("Choose…"), false).clicked()
                                        && let Some(f) = app.services.pick_files.as_mut().and_then(|pick| pick().into_iter().next())
                                    {
                                        wm.image = f;
                                    }
                                    ui.add(egui::TextEdit::singleline(&mut wm.image).hint_text("logo.png").desired_width(ui.available_width()));
                                });
                            });
                            let mut width = wm.image_width as f64 * 100.0;
                            if num(ui, &WM_IMAGE_WIDTH, &mut width) {
                                wm.image_width = (width / 100.0) as f32;
                            }
                        } else {
                            choices(ui, app.ui.language.tr("Text direction"), "exportWmOrientation", &[(false, app.ui.language.tr("Horizontal text")), (true, app.ui.language.tr("Vertical text"))], &mut wm.vertical);
                            field(ui, "Text", |ui| {
                                ui.add(egui::TextEdit::multiline(&mut wm.text).desired_rows(2).hint_text(crate::i18n::tr("© Your Name")).desired_width(f32::INFINITY))
                            });
                        }
                        choices(
                            ui,
                            "Position",
                            "exportWmAnchor",
                            &[(P::TopLeft, "↖"), (P::TopRight, "↗"), (P::Center, "•"), (P::BottomLeft, "↙"), (P::BottomRight, "↘")],
                            &mut wm.anchor,
                        );
                        let mut size = wm.size as f64 * 100.0;
                        if !graphic && num(ui, &WM_SIZE, &mut size) {
                            wm.size = (size / 100.0) as f32;
                        }
                        let mut op = wm.opacity as f64 * 100.0;
                        if num(ui, &WM_OPACITY, &mut op) {
                            wm.opacity = (op / 100.0) as f32;
                        }
                        if !graphic {
                            ui.checkbox(&mut wm.shadow, crate::i18n::tr("Shadow"));
                        }
                    }
                    }
                    ui.add_space(4.0);
                    if opts.format == F::Tiff {
                        use lightcraft_engine::export::TiffCompression as Z;
                        choices(ui, "Compression", "exportTiffCompression", &[(Z::None, "None"), (Z::Lzw, "LZW"), (Z::Deflate, "ZIP")], &mut opts.tiff_compression);
                    }
                    if opts.format == F::Dng {
                        use lightcraft_engine::export::DngCompression as Z;
                        choices(ui, "Compression", "exportDngCompression", &[(Z::Lossless, "Lossless"), (Z::Deflate, "ZIP"), (Z::Uncompressed, "None")], &mut opts.dng_compression);
                    }
                    let naming_id = egui::Id::new("export-naming");
                    let tags_open = field(ui, "File name", |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let w = crate::import::tag_field_width(ui);
                        ui.add(egui::TextEdit::singleline(&mut opts.naming).id(naming_id).hint_text("{name}-{seq}  ·  {date}  ·  {title}  ·  {folder}").desired_width(w));
                        crate::import::tag_toggle(ui, "exportNaming")
                    });
                    if tags_open {
                        crate::import::tag_help(ui, "exportNaming", &mut opts.naming, naming_id);
                    }
                    crate::import::unknown_tags_warning(ui, &opts.naming);
                    naming_template_row(app, ui, "export", &mut opts.naming);
                    if opts.naming.contains("{text") || opts.naming.contains("{custom") {
                        field(ui, "Custom Text", |ui| {
                            let r = ui.add(egui::TextEdit::singleline(&mut opts.custom_text).desired_width(200.0));
                            crate::widgets::register(ui.ctx(), "field:exportText", r.rect);
                        });
                    }
                    {
                        use lightcraft_engine::rename::ExtCase as X;
                        choices(ui, "Extensions", "exportExtCase", &[(X::Keep, "keep."), (X::Lower, "lower."), (X::Upper, "upper.")], &mut opts.ext_case);
                    }
                    if opts.naming.contains("{seq") {
                        let mut v = opts.start_number as f64;
                        if num(ui, &START_NUMBER, &mut v) {
                            opts.start_number = v as u32;
                        }
                    }
                    field(ui, "Export to", |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, (label, same)) in [("chosen.folder.", false), ("same.folder.", true)].iter().enumerate() {
                            if crate::widgets::text_button(ui, &format!("exportTo-{i}"), label, opts.same_folder == *same).clicked() {
                                opts.same_folder = *same;
                            }
                        }
                    });
                    if !opts.same_folder {
                    field(ui, "Folder", |ui| {
                        trailing_button_row(ui, |ui| {
                            if app.services.pick_folder.is_some()
                                && crate::widgets::text_button(ui, "exportChooseFolder", crate::i18n::tr("Choose…"), false).clicked()
                                && let Some(pick) = app.services.pick_folder.as_mut()
                                && let Some(d) = pick()
                            {
                                *dir = d;
                            }
                            ui.add(egui::TextEdit::singleline(dir).desired_width(ui.available_width()));
                        });
                    });
                    }
                    field(ui, "Subfolder", |ui| {
                        ui.add(egui::TextEdit::singleline(&mut opts.subfolder).hint_text(crate::i18n::tr("none")).desired_width(f32::INFINITY))
                    });
                    use lightcraft_engine::export::Conflict as K;
                    choices(
                        ui,
                        "If file exists",
                        "exportConflict",
                        &[(K::Unique, "Add number"), (K::Overwrite, "Overwrite"), (K::Skip, "Skip")],
                        &mut opts.conflict,
                    );
                    after_export_row(app, ui, opts);
                    field(ui, "Catalog", |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let r = ui.checkbox(&mut opts.add_to_catalog, crate::i18n::tr("Add to this catalog"));
                        crate::widgets::register(ui.ctx(), "check:exportAddToCatalog", r.rect);
                        if opts.add_to_catalog {
                            use lightcraft_engine::export::StackPlace as P;
                            let mut st = opts.add_to_stack;
                            for (i, (label, v)) in [("no.stack.", None), ("stack.above.", Some(P::Above)), ("stack.below.", Some(P::Below))].iter().enumerate() {
                                if crate::widgets::text_button(ui, &format!("exportStack-{i}"), label, st == *v).clicked() {
                                    st = *v;
                                }
                            }
                            opts.add_to_stack = st;
                        }
                    });
                    ui.add_space(4.0);
                    field(ui, "Save preset", |ui| {
                        trailing_button_row(ui, |ui| {
                            let named = !preset_name.trim().is_empty();
                            if crate::widgets::text_button(ui, "exportSavePreset", crate::i18n::tr("Save"), false).clicked() && named {
                                let params = export_dialog_params(opts, *full_size, resize, *limit_kb);
                                match app.run("export.savePreset", json!({"name": preset_name.trim(), "params": params})) {
                                    Ok(_) => {
                                        app.toast(ui.ctx(), crate::i18n::tr_format!("Saved export preset “{}”", preset_name.trim()));
                                        preset_name.clear();
                                    }
                                    Err(e) => app.toast(ui.ctx(), e),
                                }
                            }
                            ui.add(egui::TextEdit::singleline(preset_name).hint_text(crate::i18n::tr("Preset name")).desired_width(ui.available_width()));
                        });
                    });
                    });
                }
                Dialog::Merge { opts } => crate::merge::body(app, ui, opts),
                Dialog::Import { opts } => crate::import::body(app, ui, opts),
                Dialog::Settings { tab } => crate::panels::settings::body(app, ui, tab),
                Dialog::SamModel { error, .. } => sam_model_body(app, ui, error.as_deref()),
                Dialog::ConfirmDelete { count } => {
                    let what = if *count == 1 { crate::i18n::tr("this photo").to_string() } else { crate::i18n::tr_format!("these {count} photos", count = count) };
                    ui.label(crate::i18n::tr_format!("Move {what} to Recently Deleted?", what = what));
                    ui.label(egui::RichText::new(crate::i18n::tr("They can be restored from Recently Deleted until it is emptied.")).color(t.text_dim));
                }
                Dialog::DeletePhotos { count, copies, rejected } => {
                    let (count, copies) = (*count, *copies);
                    let masters = count - copies.min(count);
                    let what = match (*rejected, count) {
                        (true, 1) => crate::i18n::tr("the rejected photo").to_string(),
                        (true, n) => format!("the {n} rejected photos", n = n),
                        (false, 1) => crate::i18n::tr("the selected photo").to_string(),
                        (false, n) => crate::i18n::tr_format!("the {n} selected photos", n = n),
                    };
                    if masters == 0 {
                        ui.label(format!("Remove {what} from the library?", what = what));
                        ui.label(
                            egui::RichText::new(crate::i18n::tr("Virtual copies have no file of their own: only the copy goes, the original stays."))
                                .color(t.text_dim),
                        );
                    } else {
                        ui.label(format!("Delete {what} from disk, or just remove {} from the library?", if count == 1 { "it" } else { "them" }, what = what));
                        ui.label(
                            egui::RichText::new(crate::i18n::tr(
                                "delete.from.disk. moves the original files and their XMP sidecars to the Trash. remove. keeps the files. Either way their virtual copies go too.",
                            ))
                            .color(t.text_dim),
                        );
                    }
                }
                Dialog::Backups { pick } => backups_body(app, ui, pick),
                Dialog::RemoveFolder { name, count, path, disk } => {
                    ui.label(crate::i18n::tr_format!(
                        "Remove “{name}” and its {count} photo{} from the library?",
                        if *count == 1 { "" } else { "s" },
                        name = name,
                        count = count
                    ));
                    ui.label(egui::RichText::new(path.as_str()).color(t.text_dim));
                    let scope = if *disk { "All the photos imported from this disk are included." } else { "That includes the photos in the folders inside it." };
                    ui.label(egui::RichText::new(crate::i18n::tr(scope)).color(t.text_dim));
                    ui.label(
                        egui::RichText::new(crate::i18n::tr("They move to Recently Deleted and can be restored; no file on disk is touched."))
                            .color(t.text_dim),
                    );
                }
                Dialog::LightroomSync { catalog, report, conflicts, error, .. } => {
                    crate::lr_sync::body(ui, catalog, report.as_ref(), conflicts, error.as_deref());
                }
                Dialog::About => {
                    ui.set_min_width(680.0);
                    let tab_id = egui::Id::new("about_tab");
                    let mut tab = ui.data_mut(|d| d.get_temp::<u8>(tab_id)).unwrap_or(0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (i, (id, label)) in ABOUT_TABS.iter().enumerate() {
                            if crate::widgets::text_button(ui, &format!("aboutTab-{id}"), label, usize::from(tab) == i).clicked() {
                                tab = u8::try_from(i).unwrap_or(0);
                            }
                        }
                    });
                    ui.data_mut(|d| d.insert_temp(tab_id, tab));
                    ui.separator();
                    match tab {
                        1 => crate::credits::contributors_ui(app, ui),
                        2 => crate::credits::models_ui(ui),
                        _ => {
                            ui.label(egui::RichText::new("LightCraft").font(t.semibold(20.0)).color(t.text));
                            ui.label(crate::i18n::tr_format!("Version {} — a clean-room, pure-Rust photo library and raw developer.", env!("CARGO_PKG_VERSION")));
                            ui.label(crate::i18n::tr_format!("MIT OR Apache-2.0. Fonts: {} (OFL). Icons: original.", crate::theme::font_credits()));
                            ui.add_space(10.0);
                            let discord = egui::Button::new(egui::RichText::new(crate::i18n::tr("Join the ArtCraft Discord")).font(t.semibold(15.0)).color(egui::Color32::WHITE))
                                .fill(t.accent)
                                .min_size(egui::vec2(260.0, 34.0));
                            let r = ui.add(discord).on_hover_text(crate::links::DISCORD);
                            crate::widgets::register(ui.ctx(), "button:aboutDiscord", r.rect);
                            if r.clicked() {
                                let _ = crate::links::open(app, crate::links::DISCORD);
                            }
                            ui.add_space(6.0);
                            for (label, url) in [
                                ("LightCraft website", crate::links::APP_PAGE),
                                ("Source code on GitHub", crate::links::GITHUB),
                                ("ArtCraft — more creative apps", crate::links::WEBSITE),
                            ] {
                                let r = ui.link(crate::i18n::tr(label)).on_hover_text(url);
                                if r.clicked() {
                                    let _ = crate::links::open(app, url);
                                }
                            }
                        }
                    }
                }
                Dialog::Shortcuts => {
                    egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                        egui::Grid::new("shortcuts").striped(true).show(ui, |ui| {
                            for (id, label, sc, _) in crate::menus::ui_commands() {
                                if let Some(sc) = sc {
                                    let grid_pick = *id == "panel.presets" && crate::shortcuts::library_grid(app);
                                    ui.label(crate::i18n::tr(if grid_pick { "Flag as Pick" } else { label }));
                                    ui.label(*sc);
                                    ui.label(egui::RichText::new(if grid_pick { "photo.flag" } else { id }).color(t.text_dim));
                                    ui.end_row();
                                }
                            }
                            for sc in ["0–5", "Shift+0–5"] {
                                ui.label(crate::i18n::tr("Set Rating"));
                                ui.label(sc);
                                ui.label(egui::RichText::new("photo.rate").color(t.text_dim));
                                ui.end_row();
                            }
                            ui.label(crate::i18n::tr("Set Color Label"));
                            ui.label("6–9");
                            ui.label(egui::RichText::new("photo.label").color(t.text_dim));
                            ui.end_row();
                            for c in lightcraft_engine::command_specs() {
                                if let Some(sc) = c.shortcut {
                                    ui.label(crate::i18n::tr(c.label));
                                    ui.label(sc);
                                    ui.label(egui::RichText::new(c.id).color(t.text_dim));
                                    ui.end_row();
                                }
                            }
                            for (sc, id, _) in crate::shortcuts::ALIASES {
                                let label = crate::menus::ui_commands()
                                    .find(|c| c.0 == *id)
                                    .map(|c| c.1)
                                    .or_else(|| lightcraft_engine::find_command(id).map(|c| c.label))
                                    .unwrap_or(id);
                                ui.label(crate::i18n::tr(label));
                                ui.label(*sc);
                                ui.label(egui::RichText::new(*id).color(t.text_dim));
                                ui.end_row();
                            }
                        });
                    });
                }
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let informational = matches!(dlg, Dialog::About | Dialog::Shortcuts | Dialog::Settings { .. });
                let sam = &app.session.segmenter;
                let (sam_installed, sam_running, sam_failed) = (sam.installed(), sam.download_status().running, sam.download_status().error.is_some());
                // no download location in this build: nothing to offer but the manual install
                let sam_nowhere = !sam_installed && !sam_running && sam.mirrors().is_empty();
                let cancel = match &dlg {
                    Dialog::SamModel { .. } if sam_running || sam_installed || sam_nowhere => "Close",
                    Dialog::SamModel { .. } => "Not Now",
                    _ => "Cancel",
                };
                if !informational {
                    let r = ui.button(crate::i18n::tr(cancel));
                    crate::widgets::register(ui.ctx(), "button:dialogCancel", r.rect);
                    if r.clicked() {
                        close = true;
                    }
                }
                // Lightroom's middle choice: out of the library, files kept
                if let Dialog::DeletePhotos { count, copies, rejected } = &dlg
                    && count > copies
                {
                    let r = ui.button("remove.");
                    crate::widgets::register(ui.ctx(), "button:dialogRemove", r.rect);
                    if r.clicked() {
                        let res = if *rejected { app.run("library.deleteRejected", json!({})) } else { app.run("photo.removeFromLibrary", json!({})) };
                        if let Err(e) = res {
                            app.toast(ui.ctx(), e);
                        }
                        close = true;
                    }
                }
                let add_label;
                let ok = match &dlg {
                    Dialog::Import { opts } => {
                        let n = opts.selected_paths().len();
                        let verb = if opts.copy && opts.move_files { "Move" } else { "Import" };
                        add_label = crate::i18n::tr_format!("{verb} {n} Photo{}", if n == 1 { "" } else { "s" }, verb = crate::i18n::tr(verb), n = n);
                        add_label.as_str()
                    }
                    Dialog::Merge { .. } => "Merge",
                    Dialog::ConfirmDelete { .. } => "Delete",
                    Dialog::DeletePhotos { count, copies, .. } if count == copies => "remove.",
                    Dialog::DeletePhotos { .. } => "delete.from.disk.",
                    Dialog::Backups { pick: Some(_) } => "restore.",
                    Dialog::Backups { .. } => "",
                    Dialog::RemoveFolder { .. } => "Remove",
                    Dialog::SamModel { then: Some(_), .. } if sam_installed => "Continue",
                    Dialog::SamModel { .. } if sam_installed || sam_running || sam_nowhere => "",
                    Dialog::SamModel { error, .. } if error.is_some() || sam_failed => "Try Again",
                    Dialog::SamModel { .. } => "Download",
                    // (nothing to apply until the dry run is in)
                    Dialog::LightroomSync { report: None, .. } | Dialog::LightroomSync { error: Some(_), .. } => "",
                    Dialog::LightroomSync { .. } => "sync.",
                    _ if informational => "Close",
                    _ => "OK",
                };
                let r = (!ok.is_empty()).then(|| ui.button(crate::i18n::tr(ok)));
                if let Some(r) = &r {
                    crate::widgets::register(ui.ctx(), "button:dialogOk", r.rect);
                }
                if r.is_some_and(|r| r.clicked()) {
                    if informational {
                        close = true;
                    } else {
                        confirm = true;
                    }
                }
            });
            if import {
                crate::import::note_dialog_bottom(ui);
            }
        });
    if let Some(w) = shown {
        ctx.move_to_top(w.response.layer_id);
        crate::widgets::register(ctx, "dialog:window", w.response.rect);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if confirm {
        match confirm_dialog(app, &dlg) {
            // the import review stays open on an error (e.g. an unusable folder template), and so
            // does Export (e.g. no folder chosen) so the choices aren't lost
            Err(e) if matches!(dlg, Dialog::Import { .. } | Dialog::Export { .. }) => app.toast(ctx, e),
            // the SAM 3 dialog stays open to show the download (or why it can't start)
            Err(e) if matches!(dlg, Dialog::SamModel { .. }) => {
                if let Dialog::SamModel { error, .. } = &mut dlg {
                    *error = Some(e);
                }
            }
            Ok(v) if matches!(dlg, Dialog::LightroomSync { .. }) => {
                app.toast_for(ctx, crate::lr_sync::summary(&v), 8.0);
                close = true;
            }
            Err(e) if matches!(dlg, Dialog::LightroomSync { .. }) => {
                if let Dialog::LightroomSync { error, .. } = &mut dlg {
                    *error = Some(e);
                }
            }
            Ok(_) if keeps_open(app, &dlg) => {
                if let Dialog::SamModel { error, .. } = &mut dlg {
                    *error = None;
                }
            }
            _ => close = true,
        }
    }
    app.ui.dialog = if close { None } else { Some(dlg) };
}

/// Apply a dialog's action (also used by `ui.dialog.confirm`).
/// `90` → `1 min 30 s`.
pub fn fmt_gap(v: f64) -> String {
    let v = v.round() as u64;
    match v {
        0..60 => format!("{v} s"),
        60..3600 if v.is_multiple_of(60) => format!("{} min", v / 60),
        60..3600 => format!("{} min {} s", v / 60, v % 60),
        3600..86400 if v.is_multiple_of(3600) => format!("{} h", v / 3600),
        3600..86400 => format!("{} h {} min", v / 3600, v % 3600 / 60),
        _ => crate::i18n::tr("1 day").into(),
    }
}

/// Whether a dialog stays open after its action succeeded (the SAM 3 dialog while the model
/// downloads).
pub fn keeps_open(app: &LightcraftApp, dlg: &Dialog) -> bool {
    matches!(dlg, Dialog::SamModel { .. }) && !app.session.segmenter.installed()
}

/// The SAM 3 dialog: what the model is, its size and licence, and the download's progress.
fn sam_model_body(app: &mut LightcraftApp, ui: &mut egui::Ui, error: Option<&str>) {
    use lightcraft_engine::segment::{LICENSE_NAME, LICENSE_URL, MODEL_BYTES};
    let t = Tokens::get(ui.ctx());
    let seg = &app.session.segmenter;
    let d = seg.download_status();
    if seg.installed() {
        ui.label(crate::i18n::tr("The SAM 3 model is installed: Object and Describe masks are ready."));
        return;
    }
    let gb = |b: u64| b as f64 / 1e9;
    ui.label(crate::i18n::tr(
        "Object and Describe masks use SAM 3, Meta's segmentation model. It isn't part of LightCraft, and everything else works without it.",
    ));
    let dir = seg.dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default();
    ui.label(format!("{} {:.1} GB, {} {dir}", crate::i18n::tr("A one-time download of about"), gb(MODEL_BYTES), crate::i18n::tr("saved in")));
    ui.label(
        egui::RichText::new(format!(
            "{} {LICENSE_NAME} — {}",
            crate::i18n::tr("Licence:"),
            crate::i18n::tr("Meta's terms, not LightCraft's. Downloading it means accepting them.")
        ))
        .color(t.text_label),
    );
    let r = ui.link(crate::i18n::tr("Read the SAM License")).on_hover_text(LICENSE_URL);
    crate::widgets::register(ui.ctx(), "link:samLicense", r.rect);
    if r.clicked() {
        let _ = crate::links::open(app, LICENSE_URL);
    }
    let seg = &app.session.segmenter;
    if d.running {
        ui.add_space(4.0);
        let frac = if d.total > 0 { d.done as f64 / d.total as f64 } else { 0.0 };
        let text = format!("{:.2} / {:.2} GB · {}", gb(d.done), gb(d.total), d.file);
        let r = ui.add(egui::ProgressBar::new(frac as f32).text(text));
        crate::widgets::register(ui.ctx(), "progress:samDownload", r.rect);
        let r = ui.button(crate::i18n::tr("Cancel Download"));
        crate::widgets::register(ui.ctx(), "button:samCancel", r.rect);
        if r.clicked() {
            app.session.segmenter.cancel_download();
        }
        ui.label(egui::RichText::new(crate::i18n::tr("You can close this: the download continues, and resumes if interrupted.")).color(t.text_dim));
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        return;
    }
    if seg.mirrors().is_empty() {
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "This build has no download location for the model yet: put the files in the folder above yourself (see docs/ai-masks.md).",
            ))
            .color(t.text_dim),
        );
    }
    if let Some(e) = error.map(str::to_string).or(d.error) {
        ui.label(egui::RichText::new(format!("{} {e}", crate::i18n::tr("The download didn't work:"))).color(egui::Color32::from_rgb(230, 90, 80)));
    }
}

pub fn confirm_dialog(app: &mut LightcraftApp, dlg: &Dialog) -> Result<serde_json::Value, String> {
    match dlg {
        Dialog::SamModel { then, .. } => {
            if app.session.segmenter.installed() {
                // installed: start what the user was doing
                if let Some((kind, op)) = then {
                    crate::panels::masking::begin_ai(app, kind, op)?;
                }
                return Ok(serde_json::Value::Null);
            }
            let r = app.run("segment.model.download", json!({"acknowledged": true}));
            if r.is_ok() {
                app.ui.sam_downloading = true;
            }
            r
        }
        Dialog::LightroomSync { records, extras, conflicts, report: Some(_), .. } => crate::lr_sync::confirm(app, records, extras, conflicts),
        Dialog::LightroomSync { .. } => Ok(serde_json::Value::Null),
        Dialog::NewAlbum { name, folder } => app.run("album.create", json!({"name": name, "folder": folder, "addSelected": !folder})),
        Dialog::RenameAlbum { id, name } => app.run("album.rename", json!({"id": id, "name": name})),
        Dialog::TextPrompt { value, command, params, key, .. } => {
            let mut p = params.clone();
            p[key.as_str()] = json!(value.trim());
            app.run(command, p)
        }
        Dialog::CaptureTime { mode, time, days, hours, minutes, zone } => app.run(
            "photo.setCaptureTime",
            match mode.as_str() {
                "set" => json!({"time": time}),
                "shift" => json!({"shift": *days as i64 * 86_400 + *hours as i64 * 3600 + *minutes as i64 * 60}),
                "original" => json!({"revert": true}),
                "file" => json!({"fileDate": true}),
                _ => json!({"hours": zone}),
            },
        ),
        Dialog::LabelNames { names, save_as } => {
            let mut m = serde_json::Map::new();
            for (l, n) in lightcraft_catalog::ColorLabel::ALL.iter().zip(names) {
                m.insert(format!("{l:?}").to_lowercase(), if n.trim().is_empty() { serde_json::Value::Null } else { json!(n.trim()) });
            }
            let r = app.run("label.setNames", json!({"names": m}));
            if r.is_ok() && !save_as.trim().is_empty() {
                return app.run("label.saveSet", json!({"name": save_as.trim()}));
            }
            r
        }
        Dialog::Rename { template, start, text } => app.run("photo.rename", json!({"template": template, "start": start, "text": text})),
        Dialog::EditKeyword(e) => app.run("keyword.edit", e.params()),
        Dialog::ImportCatalog { path, replace, .. } => {
            let r = app.run("library.importCatalog", json!({"path": path, "replace": replace}));
            if let Ok(v) = &r {
                let msg = format!("Imported {} photos and {} albums ({} replaced, {} skipped)", v["added"], v["albums"], v["replaced"], v["skipped"]);
                app.toast_soon(msg, 5.0);
            }
            r
        }
        Dialog::RenameKeyword { from, to } => app.run("keyword.rename", json!({"from": from, "to": to})),
        Dialog::MergeKeywords { from, into } => app.run("keyword.merge", json!({"from": from, "into": into})),
        Dialog::AutoStack { gap } => app.run("stack.auto", json!({"gap": gap})),
        Dialog::AllMetadata { .. } | Dialog::SystemInfo { .. } | Dialog::WhatsNew => Ok(serde_json::Value::Null),
        Dialog::Cull { reject_below, pick_best } => {
            let mut p = json!({"pickBest": pick_best});
            if *reject_below > 0.0 {
                p["rejectBelow"] = json!(reject_below);
            }
            app.run("photo.analyze", p)
        }
        Dialog::SmartRules { id, name, rules } => {
            let name = if name.trim().is_empty() { "Smart Album".to_string() } else { name.trim().to_string() };
            match id {
                Some(id) => {
                    if app.session.catalog.album(lightcraft_catalog::AlbumId(*id)).is_some_and(|a| a.name != name) {
                        app.run("album.rename", json!({"id": id, "name": name}))?;
                    }
                    app.run("album.setRules", json!({"id": id, "replace": true, "rules": {"ruleSet": rules}}))
                }
                None => app.run("album.createSmart", json!({"name": name, "rules": {"ruleSet": rules}})),
            }
        }
        Dialog::NewSmartAlbum { name } => app.run("album.createSmart", json!({"name": if name.trim().is_empty() { "Smart Album" } else { name }})),
        Dialog::CreatePreset { name, group, groups, masks } => app.run(
            "preset.create",
            json!({
                "name": if name.trim().is_empty() { "My Preset" } else { name },
                "group": if group.trim().is_empty() { "User Presets" } else { group },
                "groups": groups,
                "masks": masks,
            }),
        ),
        Dialog::CopySettings { groups, masks, mode, step } => {
            app.run("develop.copy", json!({"groups": groups, "masks": masks, "mode": mode, "step": step}))
        }
        Dialog::SyncSettings { groups, match_exposure, auto_align, match_color, masks } => app.run(
            "develop.sync",
            json!({"groups": groups, "masks": masks, "matchExposure": match_exposure, "autoAlign": auto_align, "matchCameraColor": match_color}),
        ),
        Dialog::PasteSettings { groups, merge_masks } => {
            app.run("develop.paste", json!({"groups": groups, "maskMode": if *merge_masks { "merge" } else { "replace" }}))
        }
        Dialog::PasteMasks { params, merge } => {
            let mut p = params.clone();
            p["maskMode"] = json!(if *merge { "merge" } else { "replace" });
            app.run("develop.paste", p)
        }
        Dialog::Export { opts, full_size, resize, limit_kb, dir, .. } => {
            let mut p = export_dialog_params(opts, *full_size, resize, *limit_kb);
            p["dir"] = json!(dir);
            p["background"] = json!(true);
            app.run("app.export", p)
        }
        Dialog::Merge { opts } => crate::merge::start_final(app, opts),
        Dialog::Import { opts } => crate::import::start(app, opts),
        Dialog::ConfirmDelete { .. } => app.run("photo.delete", json!({})),
        Dialog::DeletePhotos { count, copies, rejected } => {
            let r = match (*rejected, count == copies) {
                (true, _) => app.run("library.deleteRejected", json!({"fromDisk": true})),
                (false, true) => app.run("photo.removeFromLibrary", json!({})),
                (false, false) => app.run("photo.deleteFromDisk", json!({})),
            }?;
            // files that couldn't go to the Trash keep their photos: say so
            match r.get("failed").and_then(serde_json::Value::as_array).map(Vec::len).unwrap_or(0) {
                0 => Ok(r),
                n => Err(format!(
                    "{n} file{} couldn't be moved to the Trash: {e}",
                    if n == 1 { "" } else { "s" },
                    n = n,
                    e = r["failed"][0]["error"].as_str().unwrap_or("")
                )),
            }
        }
        Dialog::RemoveFolder { path, disk, .. } => app.run("library.removeFolder", json!({"path": path, "disk": disk})),
        Dialog::Backups { pick: Some(path) } => {
            let r = app.run("library.restoreBackup", json!({"path": path}))?;
            app.renderer.forget_all();
            app.ui.compare = None;
            Ok(r)
        }
        Dialog::Backups { pick: None } => Ok(serde_json::Value::Null),
        Dialog::About | Dialog::Shortcuts | Dialog::Settings { .. } => Ok(serde_json::Value::Null),
    }
}

/// The backup list: date, photos, size, note; click one to pick it for restore. The picked row
/// says what restoring does (the current state is backed up first).
fn backups_body(app: &mut LightcraftApp, ui: &mut egui::Ui, pick: &mut Option<String>) {
    let t = Tokens::get(ui.ctx());
    let list = app.session.execute("library.backups", &json!({})).unwrap_or_default();
    ui.label(egui::RichText::new(list["dir"].as_str().unwrap_or("")).color(t.text_dim));
    let rows = list["backups"].as_array().cloned().unwrap_or_default();
    if rows.is_empty() {
        ui.label(crate::i18n::tr("No backups yet. Back Up Library… (File menu) or the bkup. settings tab makes one."));
        return;
    }
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        for (i, b) in rows.iter().enumerate() {
            let path = b["path"].as_str().unwrap_or("").to_string();
            let when = b["created"].as_str().map(lightcraft_engine::catalog::dates::display_time).unwrap_or_default();
            let bytes = b["bytes"].as_u64().unwrap_or(0) as f64;
            let size = if bytes < 1_048_576.0 { format!("{:.0} KB", bytes / 1024.0) } else { format!("{:.1} MB", bytes / 1_048_576.0) };
            let mut text = format!("{when} · {n} photos · {size}", when = when, n = b["photos"].as_u64().unwrap_or(0), size = size);
            if let Some(note) = b["note"].as_str() {
                text = format!("{text} · {note}");
            }
            let selected = pick.as_deref() == Some(path.as_str());
            let r = ui.add(egui::Button::selectable(selected, text).min_size(egui::vec2(ui.available_width(), 24.0)));
            crate::widgets::register(ui.ctx(), format!("button:backup-{i}"), r.rect);
            if r.clicked() {
                *pick = if selected { None } else { Some(path) };
            }
        }
    });
    if pick.is_some() {
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "restore. replaces the catalog with this backup. The current catalog is backed up first, so restoring that backup undoes it.",
            ))
            .color(t.text_dim),
        );
    }
}

/// Open a one-field dialog that runs `command` with `params` + `{key: typed value}`.
pub fn prompt(app: &mut LightcraftApp, title: &str, hint: &str, value: &str, command: &str, params: serde_json::Value, key: &str) {
    app.ui.dialog =
        Some(Dialog::TextPrompt { title: title.into(), hint: hint.into(), value: value.into(), command: command.into(), params, key: key.into() });
}

/// The Export dialog's choices as `app.export` params (without the folder).
fn export_dialog_params(
    opts: &lightcraft_engine::export::ExportOptions,
    full_size: bool,
    resize: &lightcraft_engine::export::Resize,
    limit_kb: u32,
) -> serde_json::Value {
    let o = lightcraft_engine::export::ExportOptions {
        resize: (!full_size).then_some(*resize),
        limit_kb: (limit_kb > 0).then_some(limit_kb),
        ..opts.clone()
    };
    o.to_json()
}

// ------------------------------------------------------------------------------------------ dialog widgets

/// Merge or replace the photos' masks with the copied ones.
fn mask_choice(ui: &mut egui::Ui, tag: &str, merge: &mut bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 16.0;
        let r =
            ui.radio_value(merge, true, crate::i18n::tr("Merge masks")).on_hover_text(crate::i18n::tr("Add the copied masks to each photo's own"));
        crate::widgets::register(ui.ctx(), format!("{tag}:mergeMasks"), r.rect);
        let r = ui.radio_value(merge, false, crate::i18n::tr("Replace masks")).on_hover_text(crate::i18n::tr("Remove each photo's masks first"));
        crate::widgets::register(ui.ctx(), format!("{tag}:replaceMasks"), r.rect);
    });
}

/// Copy Settings ▸ how Paste applies them: the exact settings, or (Lightroom's relative paste) the
/// changes made since the first or a chosen history state, moved onto each photo's own values.
fn paste_mode_row(app: &LightcraftApp, ui: &mut egui::Ui, mode: &mut String, step: &mut usize) {
    let t = Tokens::get(ui.ctx());
    let history: Vec<String> = app
        .session
        .active()
        .and_then(|id| app.session.catalog.photo(id))
        .map(|p| p.history.iter().map(|h| h.label.clone()).collect())
        .unwrap_or_default();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(egui::RichText::new(crate::i18n::tr("Paste")).color(t.text_dim));
        let label = |m: &str| match m {
            "sinceFirst" => "Relative changes since the first history state",
            "sinceStep" => "Relative changes since a history state",
            _ => "Exact settings",
        };
        let c = egui::ComboBox::from_id_salt("copyPasteMode").width(300.0).selected_text(crate::i18n::tr(label(mode))).show_ui(ui, |ui| {
            for m in ["exact", "sinceFirst", "sinceStep"] {
                let r = ui.selectable_label(mode == m, crate::i18n::tr(label(m)));
                crate::widgets::register(ui.ctx(), format!("copyPasteMode:{m}"), r.rect);
                if r.clicked() {
                    *mode = m.to_string();
                }
            }
        });
        crate::widgets::register(ui.ctx(), "combo:copyPasteMode", c.response.rect);
    });
    if mode == "sinceStep" {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let last = history.len().max(1);
            let mut n = (*step + 1).min(last);
            let r = ui.add(egui::DragValue::new(&mut n).range(1..=last).prefix(crate::i18n::tr("step ")));
            crate::widgets::register(ui.ctx(), "field:copyPasteStep", r.rect);
            *step = n.saturating_sub(1);
            let name = history.get(*step).cloned().unwrap_or_else(|| crate::i18n::tr("no history yet").into());
            ui.label(egui::RichText::new(name).color(t.text_label));
        });
    }
}

/// The checklist's three columns (heading ids), in Lightroom's order.
const CHECKLIST_COLUMNS: [&[&str]; 3] = [
    &["basic", "light", "presence", "toneCurve"],
    &["colorPanel", "detail", "optics"],
    &["geometry", "effectsPanel", "calibration", "lensBlur", "masks", "remove", "crop"],
];

/// Lightroom's per-setting checklist (Copy Settings, Synchronize, Paste Selected, New Preset):
/// headings with a tri-state checkbox over their settings in three columns, the photo's masks
/// under Masking (`masks`: `None` = all of them), then all. / none. / def. / mod. and the saved
/// subsets (pick one, or type a name to save. / ren. / del.).
fn group_checklist(app: &mut LightcraftApp, ui: &mut egui::Ui, tag: &str, groups: &mut Vec<String>, mut masks: Option<&mut Option<Vec<u32>>>) {
    let t = Tokens::get(ui.ctx());
    let photo_masks: Vec<(u32, String)> = app
        .session
        .active()
        .and_then(|id| app.session.develop_of(id))
        .map(|d| d.masks.iter().map(|m| (m.id, m.name.clone())).collect())
        .unwrap_or_default();
    let set_on = |groups: &mut Vec<String>, id: String, on: bool| {
        groups.retain(|x| *x != id);
        if on {
            groups.push(id);
        }
    };
    ui.set_min_width(600.0);
    // checklist order (clicks, older remembered ids)
    let tidy = crate::state::normalize_groups(groups);
    if tidy != *groups {
        *groups = tidy;
    }
    // the checklist scrolls in a short window rather than pushing OK off the screen
    let max_h = (ui.ctx().content_rect().height() - 380.0).max(240.0);
    egui::ScrollArea::vertical().id_salt(("checklist", tag)).max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        ui.columns(3, |cols| {
            for (col, ids) in cols.iter_mut().zip(CHECKLIST_COLUMNS) {
                col.spacing_mut().item_spacing.y = 4.0;
                for (n, h) in ids.iter().filter_map(|id| lightcraft_develop::HEADINGS.iter().find(|h| h.id == *id)).enumerate() {
                    if n > 0 {
                        col.add_space(10.0);
                    }
                    let on_count = h.items.iter().filter(|g| groups.contains(&g.id())).count();
                    if let [only] = h.items {
                        let key = only.id();
                        let mut on = on_count == 1;
                        let r = col.checkbox(&mut on, egui::RichText::new(crate::i18n::tr(only.label())).font(t.semibold(12.5)));
                        crate::widgets::register(&r.ctx, format!("{tag}:{key}"), r.rect);
                        if r.changed() {
                            set_on(groups, key, on);
                        }
                    } else {
                        let mut all = on_count == h.items.len();
                        let r = col.add(
                            egui::Checkbox::new(&mut all, egui::RichText::new(crate::i18n::tr(h.label)).font(t.semibold(12.5)))
                                .indeterminate(on_count > 0 && on_count < h.items.len()),
                        );
                        crate::widgets::register(&r.ctx, format!("{tag}:{}", h.id), r.rect);
                        if r.changed() {
                            for g in h.items {
                                set_on(groups, g.id(), all);
                            }
                        }
                        col.indent(("checklist", h.id), |col| {
                            for g in h.items {
                                let key = g.id();
                                let mut on = groups.contains(&key);
                                let r = col.checkbox(&mut on, crate::i18n::tr(g.label()));
                                crate::widgets::register(&r.ctx, format!("{tag}:{key}"), r.rect);
                                if r.changed() {
                                    set_on(groups, key, on);
                                }
                            }
                        });
                    }
                    // Masking: which of this photo's masks go along
                    if h.id == "masks"
                        && groups.iter().any(|g| g == "masks")
                        && !photo_masks.is_empty()
                        && let Some(chosen) = masks.as_deref_mut()
                    {
                        col.indent("checklist-masks", |col| {
                            for (id, name) in &photo_masks {
                                let mut on = chosen.as_ref().is_none_or(|c| c.contains(id));
                                let label = if name.trim().is_empty() { format!("Mask {id}") } else { name.clone() };
                                let r = col.checkbox(&mut on, label);
                                crate::widgets::register(&r.ctx, format!("{tag}:mask:{id}"), r.rect);
                                if r.changed() {
                                    let mut list: Vec<u32> = chosen.clone().unwrap_or_else(|| photo_masks.iter().map(|(i, _)| *i).collect());
                                    list.retain(|x| x != id);
                                    if on {
                                        list.push(*id);
                                    }
                                    *chosen = (list.len() < photo_masks.len()).then_some(list);
                                }
                            }
                        });
                    }
                }
            }
        });
    });
    ui.add_space(4.0);
    let ids_of = |v: &serde_json::Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
        for (label, subset) in [("all.", "all"), ("none.", "none"), ("def.", "default"), ("mod.", "modified")] {
            let r = ui.button(label).on_hover_text(crate::i18n::tr(match subset {
                "all" => "Check all",
                "none" => "Check none",
                "default" => "Lightroom's default: everything but crop, masking, remove and red eye",
                _ => "Only what was changed on this photo",
            }));
            crate::widgets::register(ui.ctx(), format!("{tag}:{subset}"), r.rect);
            if r.clicked() {
                *groups = match subset {
                    "all" => lightcraft_develop::SettingsGroup::ALL.iter().map(|g| g.id()).collect(),
                    "none" => Vec::new(),
                    "default" => crate::state::default_preset_groups(),
                    _ => app.session.execute("develop.modifiedGroups", &json!({})).map(|v| ids_of(&v["groups"])).unwrap_or_default(),
                };
            }
        }
        ui.separator();
        // saved subsets
        let saved = app.session.execute("develop.subsets", &json!({})).unwrap_or_default();
        let names: Vec<String> =
            saved["subsets"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        let name_id = egui::Id::new(("checklist-subset-name", tag));
        let mut name: String = ui.data(|d| d.get_temp(name_id)).unwrap_or_default();
        let current = names
            .iter()
            .find(|n| saved["subsets"].as_array().into_iter().flatten().any(|x| x["name"] == n.as_str() && ids_of(&x["groups"]) == *groups));
        let c = egui::ComboBox::from_id_salt(("checklist-subsets", tag))
            .width(120.0)
            .selected_text(current.cloned().unwrap_or_else(|| "subset.".into()))
            .show_ui(ui, |ui| {
                if names.is_empty() {
                    ui.label(egui::RichText::new(crate::i18n::tr("No saved subsets: type a name, then save.")).color(t.text_dim));
                }
                for (i, n) in names.iter().enumerate() {
                    let r = ui.selectable_label(Some(n) == current, n);
                    crate::widgets::register(ui.ctx(), format!("{tag}:subset:{i}"), r.rect);
                    if r.clicked() {
                        if let Some(x) = saved["subsets"].as_array().into_iter().flatten().find(|x| x["name"] == n.as_str()) {
                            *groups = ids_of(&x["groups"]);
                        }
                        name = n.clone();
                    }
                }
            });
        crate::widgets::register(ui.ctx(), format!("combo:{tag}Subset"), c.response.rect);
        let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(crate::i18n::tr("Subset name")).desired_width(110.0));
        crate::widgets::register(ui.ctx(), format!("field:{tag}Subset"), r.rect);
        let ctx = ui.ctx().clone();
        let run = |app: &mut LightcraftApp, cmd: &str, p: serde_json::Value| {
            if let Err(e) = app.run(cmd, p) {
                app.toast_error(&ctx, e);
            }
        };
        let save = ui.button("save.").on_hover_text(crate::i18n::tr("Save the checked settings as a subset with this name"));
        crate::widgets::register(ui.ctx(), format!("{tag}:saveSubset"), save.rect);
        if save.clicked() {
            run(app, "develop.saveSubset", json!({"name": name, "groups": groups}));
        }
        let ren = ui.button("ren.").on_hover_text(crate::i18n::tr("Rename the subset picked above to this name"));
        crate::widgets::register(ui.ctx(), format!("{tag}:renameSubset"), ren.rect);
        if ren.clicked() {
            match current {
                Some(from) => run(app, "develop.renameSubset", json!({"name": from, "to": name})),
                None => app.toast(&ctx, crate::i18n::tr("Pick a saved subset first")),
            }
        }
        let del = ui.button("del.").on_hover_text(crate::i18n::tr("Delete the saved subset with this name"));
        crate::widgets::register(ui.ctx(), format!("{tag}:deleteSubset"), del.rect);
        if del.clicked() {
            run(app, "develop.deleteSubset", json!({"name": name}));
        }
        ui.data_mut(|d| d.insert_temp(name_id, name));
    });
}

/// Width of the label column in dialogs.
const LABEL_W: f32 = 78.0;

const fn spec(id: &'static str, label: &'static str, min: f64, max: f64, default: f64, step: f64) -> ControlSpec {
    ControlSpec { id, label, section: Section::Light, min, max, default, step, decimals: 0, track: Track::Plain }
}
const QUALITY: ControlSpec = spec("export.quality", "Quality", 1.0, 100.0, 90.0, 1.0);
const LIMIT_KB: ControlSpec = spec("export.limitKb", "Limit file size (KB, 0 = off)", 0.0, 20_000.0, 0.0, 10.0);
const SIZE_PX: ControlSpec = spec("export.sizePx", "Pixels", 16.0, 20_000.0, 2048.0, 16.0);
const SIZE_W: ControlSpec = spec("export.sizeW", "Width (px)", 16.0, 20_000.0, 2048.0, 16.0);
const SIZE_H: ControlSpec = spec("export.sizeH", "Height (px)", 16.0, 20_000.0, 2048.0, 16.0);
const SIZE_MP: ControlSpec = ControlSpec { decimals: 1, ..spec("export.sizeMp", "Megapixels", 0.1, 100.0, 12.0, 0.1) };
const SIZE_PCT: ControlSpec = spec("export.sizePercent", "Percent", 1.0, 400.0, 50.0, 1.0);
const START_NUMBER: ControlSpec = spec("export.startNumber", "Start number", 1.0, 9999.0, 1.0, 1.0);
const PPI: ControlSpec = spec("export.ppi", "Resolution (ppi)", 1.0, 1200.0, 240.0, 1.0);

/// Image Sizing: full size, or a resize mode and its value(s), don't enlarge, ppi.
fn export_size(ui: &mut egui::Ui, full: &mut bool, r: &mut lightcraft_engine::export::Resize, ppi: &mut u16) {
    use lightcraft_engine::export::ResizeMode as R;
    const MODES: [(R, &str); 7] = [
        (R::LongEdge, "Long Edge"),
        (R::ShortEdge, "Short Edge"),
        (R::Width, "Width"),
        (R::Height, "Height"),
        (R::Dimensions, "Width × Height"),
        (R::Megapixels, "Megapixels"),
        (R::Percent, "Percentage"),
    ];
    let r_full = ui.checkbox(full, crate::i18n::tr("Full size"));
    crate::widgets::register(ui.ctx(), "check:exportFullSize", r_full.rect);
    if !*full {
        field(ui, "Resize to", |ui| {
            let cur = MODES.iter().find(|m| m.0 == r.mode).map_or("Long Edge", |m| m.1);
            let before = r.mode;
            let c = egui::ComboBox::from_id_salt("exportResizeMode").width(150.0).selected_text(crate::i18n::tr(cur)).show_ui(ui, |ui| {
                for (m, l) in MODES {
                    ui.selectable_value(&mut r.mode, m, crate::i18n::tr(l));
                }
            });
            crate::widgets::register(ui.ctx(), "combo:exportResizeMode", c.response.rect);
            if r.mode != before {
                // a sensible value for the new unit
                r.value = match r.mode {
                    R::Megapixels => 12.0,
                    R::Percent => 50.0,
                    _ if matches!(before, R::Megapixels | R::Percent) => 2048.0,
                    _ => r.value,
                };
                if r.mode == R::Dimensions && r.height == 0 {
                    r.height = r.value as u32;
                }
            }
        });
        let mut v = r.value as f64;
        let spec = match r.mode {
            R::Megapixels => &SIZE_MP,
            R::Percent => &SIZE_PCT,
            R::Dimensions => &SIZE_W,
            _ => &SIZE_PX,
        };
        if num(ui, spec, &mut v) {
            r.value = v as f32;
        }
        if r.mode == R::Dimensions {
            let mut h = r.height as f64;
            if num(ui, &SIZE_H, &mut h) {
                r.height = h as u32;
            }
        }
        let c = ui.checkbox(&mut r.dont_enlarge, crate::i18n::tr("Don't enlarge"));
        crate::widgets::register(ui.ctx(), "check:exportDontEnlarge", c.rect);
    }
    let mut p = *ppi as f64;
    if num(ui, &PPI, &mut p) {
        *ppi = p as u16;
    }
}
const WM_IMAGE_WIDTH: ControlSpec = spec("export.watermarkImageWidth", "Width (% of photo)", 2.0, 100.0, 20.0, 1.0);
const WM_SIZE: ControlSpec = spec("export.watermarkSize", "Size (% of short edge)", 1.0, 15.0, 3.5, 0.5);
const WM_OPACITY: ControlSpec = spec("export.watermarkOpacity", "Opacity (%)", 5.0, 100.0, 70.0, 1.0);

/// A themed slider row editing `v`; true when it changed.
fn num(ui: &mut egui::Ui, spec: &ControlSpec, v: &mut f64) -> bool {
    match crate::widgets::slider(ui, spec, *v, true, None).value {
        Some(n) => {
            *v = n;
            true
        }
        None => false,
    }
}

/// A labelled row (fixed label column).
fn field<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(LABEL_W, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(egui::RichText::new(crate::i18n::tr(label)).color(t.text_label));
        });
        add(ui)
    })
    .inner
}

/// The rest of a [`field`] row, laid out right to left: `add` places its trailing button(s) first,
/// then a text field taking exactly the width that's left. (Subtracting a guessed button width
/// instead made the auto-sized dialog grow every frame when the real button was wider, #8.)
fn trailing_button_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let h = ui.spacing().interact_size.y.max(24.0);
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), h), egui::Layout::right_to_left(egui::Align::Center), add);
}

/// A labelled row of mutually exclusive choice buttons (ids `button:{id}-{index}`).
fn choices<V: PartialEq + Copy>(ui: &mut egui::Ui, label: &str, id: &str, options: &[(V, &str)], value: &mut V) {
    field(ui, label, |ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (v, l)) in options.iter().enumerate() {
            if crate::widgets::text_button(ui, &format!("{id}-{i}"), l, *value == *v).clicked() {
                *value = *v;
            }
        }
    });
}
