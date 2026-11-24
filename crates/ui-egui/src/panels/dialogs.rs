//! Modal dialogs (new album, rename, create preset, choose settings to copy, export, about, shortcuts).

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
    // the OK button this frame (empty: none) and whether it only closes: Return presses it
    let mut ok_button: (bool, bool) = (false, false);
    // "Export One File" / "Export 12 Files", as Lightroom titles it
    let export_title = if matches!(dlg, Dialog::Export { .. }) { crate::export_dialog::title(app) } else { String::new() };
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
        Dialog::Export { .. } => export_title.as_str(),
        Dialog::ExportExisting { .. } => "Existing File",
        Dialog::Merge { opts } => opts.title(),
        Dialog::Enhance { .. } => "Enhance",
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
    // (the Export dialogs too: their width must not carry over to the small dialogs)
    let window_id = egui::Id::new(match dlg {
        Dialog::Import { .. } => "lightcraft-import-dialog",
        Dialog::Export { .. } => "lightcraft-export-dialog",
        Dialog::ExportExisting { .. } => "lightcraft-export-ask-dialog",
        // its own size: a small dialog shown before (Delete's question) doesn't narrow it
        Dialog::Settings { .. } => "lightcraft-settings-dialog",
        _ => "lightcraft-dialog",
    });
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
            Dialog::Export { .. } => crate::export_dialog::DIALOG_W,
            Dialog::ExportExisting { .. } => 460.0,
            Dialog::CaptureTime { .. } => 520.0,
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
                Dialog::Cull { reject_below, pick_best, reject_eyes, reject_face_below, reject_expression_below } => {
                    let n = app.session.selection.ids.len();
                    let scope = if n > 1 { crate::i18n::tr_format!("the {n} selected photos", n = n) } else { crate::i18n::tr_format!("the {} photos in view", app.session.visible_cloned().len()) };
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Scores {scope} for focus and exposure and finds similar shots taken within seconds of each other (bursts).", scope = scope)).color(t.text_label));
                    ui.add_space(6.0);
                    let r = ui.add(egui::Slider::new(reject_below, 0.0..=80.0).text(crate::i18n::tr("Reject below focus")).step_by(1.0));
                    crate::widgets::register(ui.ctx(), "field:cullReject", r.rect);
                    ui.label(egui::RichText::new(crate::i18n::tr(if *reject_below > 0.0 { "Blurry photos below the score are flagged as rejects." } else { "0: nothing is rejected." })).color(t.text_dim));
                    let r = ui.checkbox(pick_best, crate::i18n::tr("Pick the sharpest photo of each burst"));
                    crate::widgets::register(ui.ctx(), "check:cullPick", r.rect);
                    ui.add_space(6.0);
                    if app.session.vision.installed(lightcraft_engine::vision::models::Package::Faces) {
                        let r = ui.checkbox(reject_eyes, crate::i18n::tr("Reject photos where someone blinks"));
                        crate::widgets::register(ui.ctx(), "check:cullEyes", r.rect);
                        let r = ui.add(egui::Slider::new(reject_face_below, 0.0..=80.0).text(crate::i18n::tr("Reject below face focus")).step_by(1.0));
                        crate::widgets::register(ui.ctx(), "field:cullFace", r.rect);
                        ui.label(egui::RichText::new(crate::i18n::tr("Faces are found first where needed (in the background); a burst's best is the one where nobody blinks and the face is sharpest.")).color(t.text_dim));
                        ui.add_space(6.0);
                        cull_expressions(app, ui, &t, reject_expression_below);
                    } else {
                        ui.label(egui::RichText::new(crate::i18n::tr("Eyes-closed and face-focus checks need the face finder: People view ▸ get.")).color(t.text_dim));
                    }
                    ui.label(egui::RichText::new(crate::i18n::tr("Scores stay on the photos: filter or make smart albums with Focus, Face Focus, Eyes Closed, Smiles, Expression and Best of Similar Shots.")).color(t.text_dim));
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
                    ui.label(egui::RichText::new(crate::i18n::tr_format!("Matches: {}", rules.describe())).color(t.text_label));
                    ui.label(
                        egui::RichText::new(crate::i18n::tr_format!("{n} photo{} now · updates automatically as photos change", if n == 1 { "" } else { "s" }, n = n))
                            .color(t.text_dim),
                    );
                }
                Dialog::CaptureTime { .. } => {
                    confirm |= crate::capture_time_dialog::body(app, ui, &mut dlg);
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
                    // the reference: the first photo picked (Session::sync_source)
                    let from = app.session.sync_source().and_then(|id| app.session.catalog.photo(id)).map(|p| p.file_name.clone()).unwrap_or_default();
                    let r = ui.label(egui::RichText::new(format!("{} {from}", crate::i18n::tr("from:"))).color(t.text));
                    crate::widgets::register(ui.ctx(), "syncFrom", r.rect);
                    ui.add_space(8.0);
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
                Dialog::Export { opts, full_size, resize, preset_name, limit_kb, dir, selected, checked, album } => {
                    // the lists take the room a window has; the buttons stay in view
                    let room = (screen.height() - 230.0).clamp(240.0, 640.0);
                    let s = crate::export_dialog::State { opts, full_size, resize, preset_name, limit_kb, dir, selected, checked, album };
                    match crate::export_dialog::body(app, ui, s, room) {
                        crate::export_dialog::Action::Close => close = true,
                        crate::export_dialog::Action::Confirm => confirm = true,
                        crate::export_dialog::Action::None => {}
                    }
                }
                Dialog::ExportExisting { existing, at, answers, apply_all, .. } => {
                    let left = existing.len().saturating_sub(*at);
                    let path = existing.get(*at).map(|e| e.1.clone()).unwrap_or_default();
                    match crate::export_dialog::ask_body(ui, &path, left, apply_all) {
                        Some("cancel") => close = true,
                        Some(answer) => {
                            // this file, or (Apply to all) every one left
                            let upto = if *apply_all { existing.len() } else { (*at + 1).min(existing.len()) };
                            for (id, _) in existing.iter().take(upto).skip(*at) {
                                answers.insert(id.to_string(), json!(answer));
                            }
                            *at = upto;
                            if *at >= existing.len() {
                                confirm = true;
                            }
                        }
                        None => {}
                    }
                }
                Dialog::Merge { opts } => crate::merge::body(app, ui, opts),
                Dialog::Enhance { opts } => crate::enhance::body(app, ui, opts),
                Dialog::Import { opts } => crate::import::body(app, ui, opts),
                Dialog::Settings { tab } => crate::panels::settings::body(app, ui, tab),
                Dialog::SamModel { error, .. } => sam_model_body(app, ui, error.as_deref()),
                Dialog::ConfirmDelete { count, rejected } => {
                    let what = match (*rejected, *count) {
                        (true, 1) => crate::i18n::tr("the rejected photo").to_string(),
                        (true, n) => crate::i18n::tr_format!("the {n} rejected photos", n = n),
                        (false, 1) => crate::i18n::tr("this photo").to_string(),
                        (false, n) => crate::i18n::tr_format!("these {count} photos", count = n),
                    };
                    let in_album = matches!(app.session.source, lightcraft_engine::LibrarySource::Album(_));
                    let (ask, note) = match app.ui.settings.delete_key {
                        crate::state::DeleteKey::Recent => ("Move {what} to Recently Deleted?", "They can be restored from Recently Deleted until it is emptied."),
                        crate::state::DeleteKey::Album if in_album => ("Remove {what} from this album?", "The files and the library keep them."),
                        crate::state::DeleteKey::Album => ("Remove {what} from the library?", "The files stay on disk."),
                        _ => ("Move {what} to the Trash?", "The files go to the system Trash; Undo (⌘Z) brings them back."),
                    };
                    ui.label(crate::i18n::tr(ask).replace("{what}", &what));
                    ui.label(egui::RichText::new(crate::i18n::tr(note)).color(t.text_dim));
                    ui.label(egui::RichText::new(crate::i18n::tr("Settings ▸ General ▸ Confirm first turns this question off.")).color(t.text_dim));
                }
                Dialog::DeletePhotos { count, copies, rejected } => {
                    let (count, copies) = (*count, *copies);
                    let masters = count - copies.min(count);
                    let what = match (*rejected, count) {
                        (true, 1) => crate::i18n::tr("the rejected photo").to_string(),
                        (true, n) => crate::i18n::tr_format!("the {n} rejected photos", n = n),
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
            // the Export dialogs draw their own buttons (Lightroom's row)
            let own_buttons = matches!(dlg, Dialog::Export { .. } | Dialog::ExportExisting { .. });
            if !own_buttons {
                ui.add_space(4.0);
            }
            if !own_buttons { ui.horizontal(|ui| {
                // neighbours 8 px apart (DESIGN.md §15)
                ui.spacing_mut().item_spacing.x = 8.0;
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
                    Dialog::Enhance { opts } if !opts.options().any() => "",
                    Dialog::Enhance { .. } => "enh.",
                    Dialog::ConfirmDelete { .. } => "delete.",
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
                    Dialog::CaptureTime { .. } => "apply.",
                    _ if informational => "Close",
                    _ => "OK",
                };
                ok_button = (!ok.is_empty(), informational);
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
            }); }
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
    // Return presses the OK button (Bryan, 2026-11-23: "if I hit enter, then it will hit OK for me")
    // — unless a text box still has the keyboard (a one-line field lets go of it on Return, so its
    // Return confirms too; a multi-line note keeps it)
    if ok_button.0
        && !confirm
        && !close
        && !ctx.egui_wants_keyboard_input()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
    {
        if ok_button.1 {
            close = true;
        } else {
            confirm = true;
        }
    }
    // a dialog the action opens (Export's Existing File question) replaces this one
    let mut follow = None;
    if confirm {
        let before = app.ui.dialog.take();
        let r = confirm_dialog(app, &dlg);
        follow = app.ui.dialog.take();
        app.ui.dialog = before;
        match r {
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
            // a delete that failed says why (its photos stay)
            Err(e) if matches!(dlg, Dialog::ConfirmDelete { .. } | Dialog::DeletePhotos { .. }) => {
                app.toast_error(ctx, e);
                close = true;
            }
            _ => close = true,
        }
    }
    app.ui.dialog = match follow.or_else(|| app.ui.dialog_next.take()) {
        Some(next) => Some(next),
        None if close => None,
        None => Some(dlg),
    };
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

/// The Assisted Culling dialog's smiles row: the expression slider, or the model's get. with its size and licence.
fn cull_expressions(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens, reject_below: &mut f32) {
    use lightcraft_engine::vision::models::{self, Package};
    if app.session.vision.installed(Package::Expressions) {
        let r = ui.add(egui::Slider::new(reject_below, 0.0..=60.0).text(crate::i18n::tr("Reject below expression")).step_by(1.0));
        crate::widgets::register(ui.ctx(), "field:cullExpression", r.rect);
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "Expression: smiling ≈ 90, neutral ≈ 50, frowning or grimacing low. A burst's best also prefers smiles.",
            ))
            .color(t.text_dim),
        );
        return;
    }
    let i = models::info(Package::Expressions);
    let dl = app.session.vision.download_state(Package::Expressions);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if dl.running {
            ui.label(egui::RichText::new(format!("{:.1} / {:.1} MB …", dl.done as f64 / 1e6, dl.total as f64 / 1e6)).color(t.text_dim));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            let tip =
                crate::i18n::tr_format!("Download {m} — {mb} MB, {l}", m = i.models, mb = format!("{:.1}", i.size() as f64 / 1e6), l = i.licence);
            if crate::widgets::text_button(ui, "cullGetExpressions", "get.", false).on_hover_text(tip).clicked()
                && let Err(e) = app.run("vision.model.download", json!({"model": "expressions", "acknowledged": true}))
            {
                app.toast_error(ui.ctx(), e);
            }
        }
        let msg = match &dl.error {
            Some(e) => format!("⚠ {e}"),
            None => crate::i18n::tr_format!(
                "Smiles need the expression model ({mb} MB, {l})",
                mb = format!("{:.1}", i.size() as f64 / 1e6),
                l = i.licence
            ),
        };
        ui.label(egui::RichText::new(msg).color(t.text_dim));
    });
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
        Dialog::CaptureTime { .. } => match crate::capture_time_dialog::request(dlg) {
            Some(r) => app.run("photo.setCaptureTime", r),
            None => Ok(serde_json::Value::Null),
        },
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
        Dialog::Cull { reject_below, pick_best, reject_eyes, reject_face_below, reject_expression_below } => {
            // measured on worker threads: the result lands (and is told) when every photo is in
            let mut p = json!({"pickBest": pick_best, "rejectEyesClosed": reject_eyes, "background": true});
            if *reject_expression_below > 0.0 {
                p["rejectExpressionBelow"] = json!(reject_expression_below);
            }
            if *reject_below > 0.0 {
                p["rejectBelow"] = json!(reject_below);
            }
            if *reject_face_below > 0.0 {
                p["rejectFaceBelow"] = json!(reject_face_below);
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
        Dialog::Export { opts, full_size, resize, limit_kb, dir, checked, album, .. } => {
            let mut p = crate::export_dialog::params_of(opts, *full_size, resize, *limit_kb);
            crate::export_task::start_from_dialog(app, &mut p, dir, checked, album)
        }
        Dialog::ExportExisting { params, answers, .. } => {
            let mut p = params.clone();
            p["resolve"] = serde_json::Value::Object(answers.clone());
            app.run("app.export", p)
        }
        Dialog::Merge { opts } => crate::merge::start_final(app, opts),
        Dialog::Enhance { opts } => crate::enhance::start_final(app, opts),
        Dialog::Import { opts } => crate::import::start(app, opts),
        Dialog::ConfirmDelete { rejected, .. } => crate::menus::delete_now(app, *rejected),
        Dialog::DeletePhotos { count, copies, rejected } => {
            let r = match (*rejected, count == copies) {
                (true, _) => app.run("library.deleteRejected", json!({"fromDisk": true})),
                (false, true) => app.run("photo.removeFromLibrary", json!({})),
                (false, false) => app.run("photo.deleteFromDisk", json!({})),
            }?;
            // what went (· undo.), and files that couldn't go to the Trash keep their photos: say so
            let text = crate::menus::trash_message(&r);
            crate::menus::delete_toast(app, &r, text);
            Ok(r)
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
