//! The Settings dialog (⌘,): General, Import, Performance, Interface, identity. (the identity plate), …
//!
//! Changes apply immediately (no OK/Cancel). Where they are stored:
//! - **app settings** ([`crate::state::AppSettings`]: startup view, delete confirmation, GPU,
//!   preview size, filmstrip/grid badges, last library) and Auto Advance live in the UI state,
//!   saved by the host in its config folder (`ui.json`);
//! - **library settings** (import defaults, XMP sidecars, thumbnail cache size) go through the
//!   `library.preferences` / `library.xmpPreferences` commands into the library's `prefs.json`,
//!   so they travel with the library.

use egui::RichText;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{GridBadges, PREVIEW_EDGES, StartupView};
use crate::theme::Tokens;
use crate::widgets::register;

/// (id, label) of the tabs, in order.
pub const TABS: &[(&str, &str)] = &[
    ("general", "General"),
    ("import", "Import"),
    ("performance", "Performance"),
    ("backup", "backup."),
    ("interface", "Interface"),
    ("identity", "identity."),
    ("develop", "develop.view."),
    ("shortcuts", "shortcuts."),
    ("controllers", "controllers."),
    ("cameras", "cameras."),
];

/// Thumbnail cache sizes offered (MB).
const CACHE_SIZES: [u32; 5] = [512, 1024, 2048, 4096, 8192];

const LABEL_W: f32 = 150.0;

/// The dialog body for `tab` (the tab bar switches `tab`).
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, tab: &mut String) {
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(560.0);
    ui.set_min_height(330.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (id, label) in TABS {
            if crate::widgets::text_button(ui, &format!("settingsTab-{id}"), label, tab == id).clicked() {
                *tab = id.to_string();
            }
        }
    });
    ui.separator();
    ui.add_space(2.0);
    match tab.as_str() {
        "import" => import_tab(app, ui, &t),
        "performance" => performance_tab(app, ui, &t),
        "backup" => backup_tab(app, ui, &t),
        "interface" => {
            // scrolls inside instead of growing past the window (the dialog sizes to its content)
            let h = (ui.ctx().content_rect().height() - 260.0).clamp(240.0, 640.0);
            egui::ScrollArea::vertical()
                .id_salt("settingsInterface")
                .scroll_source(egui::containers::scroll_area::ScrollSource {
                    scroll_bar: true,
                    mouse_wheel: true,
                    ..egui::containers::scroll_area::ScrollSource::NONE
                })
                .max_height(h)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    interface_tab(app, ui, &t);
                    crate::widgets::reveal_in_scroll(ui);
                });
        }
        "identity" => identity_tab(app, ui, &t),
        "develop" => develop_tab(app, ui, &t),
        "shortcuts" => shortcuts_tab(app, ui, &t),
        "controllers" => controllers_tab(app, ui, &t),
        "cameras" => cameras_tab(app, ui, &t),
        _ => general_tab(app, ui, &t),
    }
}

/// A group heading on a tab, foldable (click it; remembered, `crate::folds`): returns whether its
/// rows are shown.
fn heading(ui: &mut egui::Ui, _t: &Tokens, id: &str, text: &str) -> bool {
    ui.add_space(4.0);
    crate::folds::header(ui, id, text)
}

fn row<R>(ui: &mut egui::Ui, t: &Tokens, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(LABEL_W, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(RichText::new(crate::i18n::tr(label)).color(t.text_label));
        });
        add(ui)
    })
    .inner
}

/// A quiet note, wrapped to what is visible of the dialog (8 px from its edge): a window wider in
/// layout than on screen would otherwise cut the line off.
fn hint(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    let w = (ui.clip_rect().right() - ui.cursor().left() - 8.0).min(ui.available_width()).max(120.0);
    ui.scope(|ui| {
        ui.set_max_width(w);
        ui.add(egui::Label::new(RichText::new(crate::i18n::tr(text)).size(11.0).color(t.text_dim)).wrap());
    });
}

/// A checkbox addressable as `check:{id}`; true when toggled.
fn check(ui: &mut egui::Ui, id: &str, value: &mut bool, label: &str) -> bool {
    let r = ui.checkbox(value, crate::i18n::tr(label));
    register(ui.ctx(), format!("check:{id}"), r.rect);
    r.changed()
}

/// Mutually exclusive buttons (`button:{id}-{index}`).
fn choices<V: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, options: &[(V, &str)], value: &mut V) -> bool {
    let mut changed = false;
    ui.spacing_mut().item_spacing.x = 4.0;
    for (i, (v, l)) in options.iter().enumerate() {
        if crate::widgets::text_button(ui, &format!("{id}-{i}"), l, *value == *v).clicked() && *value != *v {
            *value = *v;
            changed = true;
        }
    }
    changed
}

// ------------------------------------------------------------------------------------- General

fn general_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    // English is the only language installed (Bryan, 11-23: the other languages' files are gone to
    // save space; the setting stays and shows what is there)
    row(ui, t, crate::i18n::tr("Language"), |ui| {
        let mut english = 0u8;
        choices(ui, "settingsLanguage", &[(0u8, "english.")], &mut english);
    });
    if heading(ui, t, "settings.general.library", crate::i18n::tr("Library")) {
        let location = match &app.session.library {
            Some(l) => l.dir.display().to_string(),
            None => crate::i18n::tr("In memory — nothing is saved").to_string(),
        };
        row(ui, t, crate::i18n::tr("Location"), |ui| {
            ui.label(RichText::new(location).color(t.text));
        });
        // merges.: found by themselves in the background, or only on find.
        row(ui, t, "find merges:", |ui| {
            let mut mode = app.ui.settings.merge_find;
            if choices(ui, "settingsMergeFind", &[(crate::merges::FindMode::Auto, "auto."), (crate::merges::FindMode::Manual, "find.")], &mut mode) {
                app.ui.settings.merge_find = mode;
            }
        });
        hint(
            ui,
            t,
            "auto.: brackets, panoramas and focus stacks are found in the background after imports and show under their folder and collection as merges.; find.: only when you click find. there. Nothing is merged until you say so.",
        );
        row(ui, t, "", |ui| {
            let can = app.services.pick_folder.is_some();
            let r = ui.add_enabled(can, egui::Button::new(crate::i18n::tr("Open Library…")));
            register(ui.ctx(), "button:settingsOpenLibrary", r.rect);
            if r.clicked() {
                let _ = app.run("app.openLibrary", json!({}));
            }
            if !can {
                hint(ui, t, crate::i18n::tr("not available here"));
            }
        });
    }
    if heading(ui, t, "settings.general.startup", crate::i18n::tr("Startup")) {
        row(ui, t, crate::i18n::tr("Open in"), |ui| {
            choices(
                ui,
                "settingsStartup",
                &[(StartupView::Last, "Last view"), (StartupView::Grid, "Photo Grid"), (StartupView::Detail, "Detail")],
                &mut app.ui.settings.startup_view,
            );
        });
    }
    if heading(ui, t, "settings.general.culling", crate::i18n::tr("Culling")) {
        check(ui, "settings.autoAdvance", &mut app.ui.auto_advance, "Auto Advance: move to the next photo after rating or flagging");
        row(ui, t, crate::i18n::tr("Delete key"), |ui| {
            use crate::state::DeleteKey;
            choices(
                ui,
                "settingsDeleteKey",
                &[(DeleteKey::Trash, "trash."), (DeleteKey::Recent, "recent."), (DeleteKey::Album, "album."), (DeleteKey::Ask, "ask.")],
                &mut app.ui.settings.delete_key,
            );
        });
        hint(
            ui,
            t,
            match app.ui.settings.delete_key {
                crate::state::DeleteKey::Trash => {
                    "trash.: the file and its sidecar go to the system Trash and the photo leaves the library, no question; ⌘Z puts the files back."
                }
                crate::state::DeleteKey::Recent => "recent.: photos go to Recently Deleted (restore them there); the files stay.",
                crate::state::DeleteKey::Album => "album.: in an album, out of that album only; elsewhere out of the library, files kept.",
                crate::state::DeleteKey::Ask => "ask.: Lightroom's question every time: Delete from Disk (to the Trash) or Remove.",
            },
        );
        if app.ui.settings.delete_key != crate::state::DeleteKey::Ask {
            check(ui, "settings.confirmDelete", &mut app.ui.settings.confirm_delete, "Confirm first: ask before deleting");
        }
        let mut autos = app.session.auto_versions;
        if check(ui, "settings.autoVersions", &mut autos, "Keep an automatic version when leaving an edited photo (Lightroom doesn't)") {
            let _ = app.run("library.preferences", json!({"autoVersions": autos}));
        }
        // Lightroom Classic: Catalog Settings ▸ Metadata ▸ Face Detection, and the People view's first-run choice
    }
    if heading(ui, t, "settings.general.people", crate::i18n::tr("People")) {
        row(ui, t, crate::i18n::tr("Find faces"), |ui| {
            use lightcraft_engine::vision::FaceDetect;
            let mut mode = app.session.face_prefs.detect;
            if choices(ui, "settingsFaces", &[(FaceDetect::Off, "off."), (FaceDetect::AsNeeded, "view."), (FaceDetect::All, "all.")], &mut mode) {
                let _ = app.run("faces.auto", json!({"mode": mode.key()}));
            }
        });
        hint(
            ui,
            t,
            crate::i18n::tr(
                "view.: in the photos the People view shows, while it is open (Lightroom's Only find faces as needed). all.: in every photo of the library, in the background, new imports first (Find faces in entire catalog). Needs the face finder (People ▸ get.).",
            ),
        );
    }
    if heading(ui, t, "settings.general.external.editor", crate::i18n::tr("External Editor")) {
        row(ui, t, crate::i18n::tr("Application"), |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.ui.settings.external_editor).hint_text(crate::i18n::tr("System default")).desired_width(220.0),
            );
            register(ui.ctx(), "field:externalEditor", r.rect);
        });
        // one click to switch (the same list as Photo ▸ Edit In)
        row(ui, t, "", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let current = app.ui.settings.external_editor.clone();
            for e in crate::state::installed_editors() {
                let label = if e == "PhotoCraft" {
                    "photocraft.".to_string()
                } else if e.starts_with("Adobe Photoshop") {
                    "photoshop.".to_string()
                } else {
                    format!("{}.", e.to_lowercase().replace(' ', "."))
                };
                if crate::widgets::text_button(ui, &format!("editor-{e}"), &label, current == e).clicked() {
                    app.ui.settings.external_editor = e.clone();
                }
            }
            if crate::widgets::text_button(ui, "editor-default", "default.", current.trim().is_empty()).clicked() {
                app.ui.settings.external_editor.clear();
            }
        });
        row(ui, t, crate::i18n::tr("Copy"), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let psd = app.ui.settings.external_format != "tiff";
            if crate::widgets::text_button(ui, "editorFormat-psd", "psd.", psd).clicked() {
                app.ui.settings.external_format = "psd".into();
            }
            if crate::widgets::text_button(ui, "editorFormat-tiff", "tiff.", !psd).clicked() {
                app.ui.settings.external_format = "tiff".into();
            }
        });
        row(ui, t, "", |ui| {
            check(
                ui,
                "settings.externalHoldGrain",
                &mut app.ui.settings.external_hold_grain,
                "Leave grain off the copy, put it back on after saving",
            );
        });
        hint(
            ui,
            t,
            crate::i18n::tr(
                "Photo ▸ Edit in External Editor (⇧⌘E) renders a 16-bit TIFF copy, stacks it with the original and opens it here (an app name on macOS, a program path elsewhere).",
            ),
        );
    }
}

// -------------------------------------------------------------------------------------- Import

/// A preset picker: `None` = `none_label`. Returns the new choice when it changed.
fn preset_combo(app: &LightcraftApp, ui: &mut egui::Ui, id: &str, current: Option<&str>, none_label: &str) -> Option<Option<String>> {
    let name = |pid: &str| {
        app.session
            .presets
            .iter()
            .find(|p| p.id == pid)
            .map(|p| crate::i18n::builtin_label(&p.name, p.builtin).to_string())
            .unwrap_or_else(|| crate::i18n::tr_format!("{pid} (missing)", pid = pid))
    };
    let text = current.map(name).unwrap_or_else(|| crate::i18n::tr(none_label).to_string());
    let mut out = None;
    let r = egui::ComboBox::from_id_salt(id).width(240.0).selected_text(text).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), crate::i18n::tr(none_label)).clicked() {
            out = Some(None);
        }
        let mut group = "";
        for p in &app.session.presets {
            if p.group != group {
                group = &p.group;
                ui.label(RichText::new(crate::i18n::builtin_label(group, p.builtin)).size(10.5).weak());
            }
            if ui.selectable_label(current == Some(p.id.as_str()), crate::i18n::builtin_label(&p.name, p.builtin)).clicked() {
                out = Some(Some(p.id.clone()));
            }
        }
    });
    register(ui.ctx(), format!("combo:{id}"), r.response.rect);
    out.filter(|v| v.as_deref() != current)
}

/// Cameras of the raws in the library plus those with a stored default, sorted.
fn cameras(app: &LightcraftApp) -> Vec<String> {
    let mut v: Vec<String> = app
        .session
        .catalog
        .photos()
        .filter(|p| p.kind == lightcraft_catalog::MediaKind::Raw && !p.meta.camera.is_empty())
        .map(|p| p.meta.camera.clone())
        .chain(app.session.import_defaults.cameras.iter().map(|c| c.camera.clone()))
        .collect();
    v.sort_by_key(|c| c.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

fn import_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let d = app.session.import_defaults.clone();
    if heading(ui, t, "settings.import.raw.defaults", crate::i18n::tr("Raw defaults")) {
        hint(ui, t, crate::i18n::tr("Settings new raw photos start from. Changing them doesn't touch photos already in the library."));
        row(ui, t, crate::i18n::tr("Raw photos"), |ui| {
            if let Some(v) = preset_combo(app, ui, "settingsRawPreset", d.raw_preset.as_deref(), "LightCraft Default") {
                let _ = app.run("library.preferences", json!({"import": {"rawPreset": v}}));
            }
        });
        let (mut lens, mut ca) = (d.lens_corrections, d.remove_ca);
        row(ui, t, "", |ui| {
            if check(ui, "settings.lensCorrections", &mut lens, "Enable lens profile corrections") {
                let _ = app.run("library.preferences", json!({"import": {"lensCorrections": lens}}));
            }
        });
        row(ui, t, "", |ui| {
            if check(ui, "settings.removeCa", &mut ca, "Remove chromatic aberration") {
                let _ = app.run("library.preferences", json!({"import": {"removeCa": ca}}));
            }
        });
        let mut per = d.per_camera;
        row(ui, t, "", |ui| {
            if check(ui, "settings.perCamera", &mut per, "Use camera-specific defaults") {
                let _ = app.run("library.preferences", json!({"import": {"perCamera": per}}));
            }
        });
        if per {
            let cams = cameras(app);
            if cams.is_empty() {
                hint(ui, t, crate::i18n::tr("No raw photos yet: cameras appear here once their photos are in the library."));
            }
            egui::ScrollArea::vertical().max_height(150.0).id_salt("settingsCameras").show(ui, |ui| {
                for (i, cam) in cams.iter().enumerate() {
                    let entry = d.cameras.iter().find(|c| c.camera.eq_ignore_ascii_case(cam));
                    row(ui, t, cam, |ui| {
                        // "Raw default" = no entry; otherwise the entry's preset (None = LightCraft Default)
                        const RAW_DEFAULT: &str = "\u{1}raw";
                        let current = match entry {
                            None => Some(RAW_DEFAULT),
                            Some(e) => e.preset.as_deref(),
                        };
                        let mut pick = None;
                        let label = match current {
                            Some(RAW_DEFAULT) => crate::i18n::tr("Same as raw default").to_string(),
                            None => crate::i18n::tr("LightCraft Default").to_string(),
                            Some(pid) => app
                                .session
                                .presets
                                .iter()
                                .find(|p| p.id == pid)
                                .map(|p| crate::i18n::builtin_label(&p.name, p.builtin).to_string())
                                .unwrap_or_else(|| pid.to_string()),
                        };
                        let id = format!("settingsCamera-{i}");
                        let r = egui::ComboBox::from_id_salt(&id).width(240.0).selected_text(label).show_ui(ui, |ui| {
                            if ui.selectable_label(current == Some(RAW_DEFAULT), crate::i18n::tr("Same as raw default")).clicked() {
                                pick = Some(json!({"camera": cam, "remove": true}));
                            }
                            if ui.selectable_label(current.is_none(), crate::i18n::tr("LightCraft Default")).clicked() {
                                pick = Some(json!({"camera": cam, "preset": null}));
                            }
                            for p in &app.session.presets {
                                if ui.selectable_label(current == Some(p.id.as_str()), crate::i18n::builtin_label(&p.name, p.builtin)).clicked() {
                                    pick = Some(json!({"camera": cam, "preset": p.id}));
                                }
                            }
                        });
                        register(ui.ctx(), format!("combo:{id}"), r.response.rect);
                        if let Some(c) = pick {
                            let _ = app.run("library.preferences", json!({"camera": c}));
                        }
                    });
                }
            });
        }
    }
    if heading(ui, t, "settings.import.other.images", crate::i18n::tr("Other images (JPEG, PNG, TIFF, HEIC…)")) {
        row(ui, t, crate::i18n::tr("Non-raw photos"), |ui| {
            if let Some(v) = preset_combo(app, ui, "settingsOtherPreset", d.other_preset.as_deref(), "None") {
                let _ = app.run("library.preferences", json!({"import": {"otherPreset": v}}));
            }
        });
    }
    if heading(ui, t, "settings.import.metadata", crate::i18n::tr("Metadata")) {
        hint(ui, t, crate::i18n::tr("Added to photos you import that don't already have it."));
        for (key, label, hint_text, value) in
            [("copyright", "Copyright", "© 2026 Your Name", d.copyright.clone()), ("creator", "Creator", "Your Name", d.creator.clone())]
        {
            row(ui, t, label, |ui| {
                let id = egui::Id::new(("settingsMeta", key));
                let mut text: String = ui.data(|m| m.get_temp(id)).unwrap_or(value.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(hint_text).desired_width(240.0));
                register(ui.ctx(), format!("field:settings.{key}"), r.rect);
                if r.lost_focus() && text.trim() != value {
                    let _ = app.run("library.preferences", json!({"import": {key: text.trim()}}));
                }
                if r.has_focus() {
                    ui.data_mut(|m| m.insert_temp(id, text));
                } else {
                    ui.data_mut(|m| m.remove::<String>(id));
                }
            });
        }
        if !app.session.metadata_presets.is_empty() {
            row(ui, t, crate::i18n::tr("Metadata preset"), |ui| {
                let cur = d.metadata_preset.clone();
                let mut pick = None;
                let r = egui::ComboBox::from_id_salt("settingsMetaPreset")
                    .width(240.0)
                    .selected_text(cur.clone().unwrap_or_else(|| crate::i18n::tr("None").into()))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(cur.is_none(), crate::i18n::tr("None")).clicked() {
                            pick = Some(String::new());
                        }
                        for m in &app.session.metadata_presets {
                            if ui.selectable_label(cur.as_deref() == Some(m.name.as_str()), &m.name).clicked() {
                                pick = Some(m.name.clone());
                            }
                        }
                    });
                register(ui.ctx(), "combo:settingsMetaPreset", r.response.rect);
                if let Some(n) = pick {
                    let _ = app.run("library.preferences", json!({"import": {"metadataPreset": n}}));
                }
            });
        }
    }
    if heading(ui, t, "settings.import.xmp.sidecars", crate::i18n::tr("XMP sidecars")) {
        let mut xmp = app.session.xmp;
        if check(ui, "settings.autoWriteXmp", &mut xmp.auto_write, "Automatically write changes into XMP sidecars") {
            let _ = app.run("library.xmpPreferences", json!({"autoWrite": xmp.auto_write}));
        }
        row(ui, t, crate::i18n::tr("Sidecar names"), |ui| {
            use lightcraft_engine::sidecar::SidecarNaming as N;
            let mut n = xmp.naming;
            if choices(ui, "settingsXmpNaming", &[(N::Stem, "IMG_1.xmp"), (N::Full, "IMG_1.CR3.xmp")], &mut n) {
                let naming = if n == N::Full { "full" } else { "stem" };
                let _ = app.run("library.xmpPreferences", json!({"naming": naming}));
            }
        });
    }
    if !cfg!(target_arch = "wasm32") && heading(ui, t, "settings.import.auto.import", crate::i18n::tr("Auto Import")) {
        hint(ui, t, crate::i18n::tr("Photos that arrive in this folder (tethering, a scanner, a sync app) are added as soon as they're complete."));
        row(ui, t, crate::i18n::tr("Watched folder"), |ui| {
            ui.label(RichText::new(d.auto_folder.clone().unwrap_or_else(|| crate::i18n::tr("Off").into())).color(t.text));
            let can = app.services.pick_folder.is_some();
            let r = ui.add_enabled(can, egui::Button::new(crate::i18n::tr("Choose…")));
            register(ui.ctx(), "button:settingsAutoFolder", r.rect);
            if r.clicked()
                && let Some(f) = app.services.pick_folder.as_mut().and_then(|f| f())
                && let Err(e) = app.run("library.autoImport", json!({"folder": f}))
            {
                app.toast(ui.ctx(), e);
            }
            if d.auto_folder.is_some() && ui.button(crate::i18n::tr("Turn Off")).clicked() {
                let _ = app.run("library.autoImport", json!({"folder": null}));
            }
        });
        if d.auto_folder.is_some() {
            row(ui, t, "", |ui| {
                let mut copy = d.auto_copy;
                if ui.checkbox(&mut copy, crate::i18n::tr("Copy into the library (else use the files where they are)")).changed() {
                    let _ = app.run("library.autoImport", json!({"copy": copy}));
                }
            });
            row(ui, t, crate::i18n::tr("Album"), |ui| {
                let id = egui::Id::new("auto-album");
                let mut name = ui.data(|m| m.get_temp::<String>(id)).unwrap_or_else(|| d.auto_album.clone().unwrap_or_default());
                let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text(crate::i18n::tr("None")).desired_width(180.0));
                if r.lost_focus() {
                    let _ = app.run("library.autoImport", json!({"album": name.trim()}));
                }
                ui.data_mut(|m| m.insert_temp(id, name));
            });
        }
    }
    super::places::settings_rows(app, ui);
    if app.session.library.is_none() {
        hint(ui, t, crate::i18n::tr("In-memory session: these settings last until LightCraft quits."));
    }
}

// -------------------------------------------------------------------------------------- Backup

/// Lightroom's Catalog Settings ▸ Backup: when LightCraft offers to back up (on quit), where, how
/// many to keep, test + optimize; Back Up Now and the backup list (restore).
fn backup_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let cur = app.session.execute("library.backupSettings", &json!({})).unwrap_or_default();
    if cur["available"] != true {
        if heading(ui, t, "settings.backup.catalog.backup", crate::i18n::tr("Catalog backup")) {
            hint(ui, t, crate::i18n::tr("This library isn't a folder on disk, so there is nothing to back up here."));
        }
        return;
    }
    if heading(ui, t, "settings.backup.catalog.backup", crate::i18n::tr("Catalog backup")) {
        row(ui, t, crate::i18n::tr("Back up"), |ui| {
            use lightcraft_engine::backup::Schedule as S;
            let mut v = app.session.backup.schedule;
            let opts = [
                (S::Never, "never."),
                (S::EveryExit, "each.quit."),
                (S::Daily, "daily."),
                (S::Weekly, "weekly."),
                (S::Monthly, "monthly."),
                (S::NextExit, "next.quit."),
            ];
            if choices(ui, "settingsBackupWhen", &opts, &mut v) {
                let _ = app.run("library.backupSettings", json!({"schedule": v.id()}));
            }
        });
        hint(
            ui,
            t,
            crate::i18n::tr("Asked when LightCraft quits, like Lightroom; the catalog and the library's settings are copied, never the photos."),
        );
        row(ui, t, crate::i18n::tr("Folder"), |ui| {
            ui.add(egui::Label::new(RichText::new(cur["dir"].as_str().unwrap_or("")).color(t.text)).truncate());
        });
        row(ui, t, "", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let can = app.services.pick_folder.is_some();
            let r = ui.add_enabled(can, egui::Button::new("change."));
            register(ui.ctx(), "button:settingsBackupFolder", r.rect);
            if r.clicked()
                && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
            {
                let _ = app.run("library.backupSettings", json!({"dir": dir}));
            }
            if cur["customDir"].is_string() {
                let r = ui.button("default.");
                register(ui.ctx(), "button:settingsBackupDefault", r.rect);
                if r.clicked() {
                    let _ = app.run("library.backupSettings", json!({"dir": null}));
                }
            }
        });
        row(ui, t, crate::i18n::tr("Keep"), |ui| {
            let opts = [(5u32, "5"), (10, "10"), (20, "20"), (50, "50"), (0, "all.")];
            let mut v = app.session.backup.keep;
            if choices(ui, "settingsBackupKeep", &opts, &mut v) {
                let _ = app.run("library.backupSettings", json!({"keep": v}));
            }
        });
        let mut verify = app.session.backup.verify;
        if check(ui, "settings.backupVerify", &mut verify, "Test each backup after writing it (sizes, checksums, the catalog loads)") {
            let _ = app.run("library.backupSettings", json!({"verify": verify}));
        }
        let mut optimize = app.session.backup.optimize;
        if check(ui, "settings.backupOptimize", &mut optimize, "Optimize the catalog after backing up") {
            let _ = app.run("library.backupSettings", json!({"optimize": optimize}));
        }
        let last = cur["last"].as_str().map(lightcraft_engine::catalog::dates::display_time).unwrap_or_else(|| crate::i18n::tr("never").to_string());
        row(ui, t, crate::i18n::tr("Last backup"), |ui| {
            ui.label(RichText::new(last).color(t.text));
        });
        ui.add_space(4.0);
        row(ui, t, "", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = ui.button("back.up.now.");
            register(ui.ctx(), "button:settingsBackupNow", r.rect);
            if r.clicked() {
                match app.run("library.backup", json!({})) {
                    Ok(v) => app.toast(ui.ctx(), format!("Backed up {n} photos", n = v["photos"].as_u64().unwrap_or(0))),
                    Err(e) => app.toast_error(ui.ctx(), e),
                }
            }
            let r = ui.button("backups.");
            register(ui.ctx(), "button:settingsBackups", r.rect);
            if r.clicked() {
                let _ = app.run("file.restoreLibrary", json!({}));
            }
            let r = ui.button("test.catalog.");
            register(ui.ctx(), "button:settingsCheckIntegrity", r.rect);
            if r.clicked() {
                match app.run("library.checkIntegrity", json!({})) {
                    Ok(v) if v["ok"] == true => app.toast(ui.ctx(), crate::i18n::tr("The catalog on disk is complete and matches.")),
                    Ok(v) => app.toast_error(ui.ctx(), v["problems"][0].as_str().unwrap_or("problem").to_string()),
                    Err(e) => app.toast_error(ui.ctx(), e),
                }
            }
        });
    }
}

// --------------------------------------------------------------------------------- Performance

fn performance_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    use lightcraft_engine::gpu;
    if heading(ui, t, "settings.performance.rendering", crate::i18n::tr("Rendering")) {
        check(ui, "settings.gpu", &mut app.ui.settings.gpu, "Use the GPU for rendering");
        let status = if !gpu::available() {
            match gpu::unavailable_reason() {
                Some(why) => crate::i18n::tr_format!("Rendering on the CPU: {why}", why = why),
                None => "No usable GPU found: rendering on the CPU".to_string(),
            }
        } else {
            format!("GPU: {}", gpu::adapter_name().unwrap_or_else(|| "starting…".into()))
        };
        hint(ui, t, &status);
        row(ui, t, crate::i18n::tr("Preview size"), |ui| {
            let opts: Vec<(u32, String)> = PREVIEW_EDGES.iter().map(|e| (*e, format!("{e} px"))).collect();
            let opts: Vec<(u32, &str)> = opts.iter().map(|(e, l)| (*e, l.as_str())).collect();
            choices(ui, "settingsPreview", &opts, &mut app.ui.settings.preview_edge);
        });
        hint(ui, t, crate::i18n::tr("Largest long edge the Detail view renders at; larger is sharper on big displays but slower."));
        row(ui, t, crate::i18n::tr("Memory for caches"), |ui| {
            let auto = crate::i18n::tr_format!("Automatic ({} MB)", lightcraft_engine::memory::default_budget() >> 20);
            let opts = [(0u32, auto.as_str()), (512, "512 MB"), (1024, "1 GB"), (2048, "2 GB"), (4096, "4 GB")];
            choices(ui, "settingsMemory", &opts, &mut app.ui.settings.memory_mb);
        });
    }
    if heading(ui, t, "settings.performance.thumbnail.cache", crate::i18n::tr("Thumbnail cache")) {
        let cur = (app.session.cache_bytes() >> 20) as u32;
        row(ui, t, crate::i18n::tr("Size limit"), |ui| {
            let opts = [(512u32, "512 MB"), (1024, "1 GB"), (2048, "2 GB"), (4096, "4 GB"), (8192, "8 GB")];
            let mut v = if CACHE_SIZES.contains(&cur) { cur } else { 2048 };
            if choices(ui, "settingsCache", &opts, &mut v) {
                let _ = app.run("library.preferences", json!({"cacheMb": v}));
            }
        });
        let used = app.session.media.rendered.disk().map(|d| d.size());
        row(ui, t, crate::i18n::tr("In use"), |ui| {
            ui.label(RichText::new(used.map(|b| format!("{:.1} MB", b as f64 / 1048576.0)).unwrap_or_else(|| "memory only".into())).color(t.text));
            let r = ui.button(crate::i18n::tr("Clear Cache"));
            register(ui.ctx(), "button:settingsClearCache", r.rect);
            if r.clicked() {
                let _ = app.run("library.clearPreviews", json!({}));
            }
        });
    }
    if heading(ui, t, "settings.performance.local.folders", crate::i18n::tr("Local folders")) {
        row(ui, t, crate::i18n::tr("Forget unchanged photos"), |ui| {
            let opts = [(0u32, "Never"), (7, "After 7 days"), (30, "After 30 days"), (90, "After 90 days"), (365, "After a year")];
            let mut v = app.session.forget_local_days;
            if choices(ui, "settingsForgetLocal", &opts, &mut v) {
                let _ = app.run("library.preferences", json!({"forgetLocalDays": v}));
            }
        });
        hint(
            ui,
            t,
            crate::i18n::tr(
                "Photos seen in Local but never added or changed leave the catalog when their folder hasn't been browsed for this long (checked when the library opens). Files stay on disk; browsing the folder shows them again.",
            ),
        );
        #[cfg(not(target_arch = "wasm32"))]
        smart_previews(app, ui, t);
    }
}

/// Where this library keeps its smart previews (the offline-editing proxies, which can be large):
/// the effective folder, what is in it, and choosing another one.
#[cfg(not(target_arch = "wasm32"))]
fn smart_previews(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    // listing a big folder every frame would be slow: refresh every 2 s and after a change
    let cache = egui::Id::new("smart-location");
    let now = ui.input(|i| i.time);
    let loc: Value = match ui.data(|d| d.get_temp::<(f64, Value)>(cache)) {
        Some((at, v)) if now - at < 2.0 => v,
        _ => {
            let v = app.run("library.smartPreviewsLocation", json!({})).unwrap_or(Value::Null);
            ui.data_mut(|d| d.insert_temp(cache, (now, v.clone())));
            v
        }
    };
    if loc.is_null() {
        return;
    }
    if heading(ui, t, "settings.performance.smart.previews", crate::i18n::tr("Smart previews")) {
        let path = loc["path"].as_str().unwrap_or_default().to_string();
        let available = loc["available"].as_bool().unwrap_or(true);
        let custom = loc["custom"].as_bool().unwrap_or(false);
        row(ui, t, crate::i18n::tr("Folder"), |ui| {
            let text = format!("{path}{}", if custom { "" } else { "  (library default)" });
            ui.add(egui::Label::new(RichText::new(text.clone()).color(if available { t.text } else { t.reject })).truncate()).on_hover_text(text);
        });
        let (count, bytes) = (loc["count"].as_u64().unwrap_or(0), loc["bytes"].as_u64().unwrap_or(0));
        if available {
            hint(
                ui,
                t,
                &crate::i18n::tr_format!(
                    "{count} smart preview{} · {:.1} MB",
                    if count == 1 { "" } else { "s" },
                    bytes as f64 / 1048576.0,
                    count = count
                ),
            );
        } else {
            hint(
                ui,
                t,
                crate::i18n::tr(
                    "This folder is not available (drive disconnected?). Smart previews are not built or used until it is back or you choose another folder.",
                ),
            );
        }
        // what happens to the previews already built when the folder changes
        let mode_id = egui::Id::new("smart-existing");
        let mut mode: u8 = ui.data(|d| d.get_temp(mode_id)).unwrap_or(0);
        if count > 0 {
            row(ui, t, crate::i18n::tr("Existing previews"), |ui| {
                if choices(ui, "settingsSmartExisting", &[(0u8, "Move them"), (1, "Leave them"), (2, "Delete them")], &mut mode) {
                    ui.data_mut(|d| d.insert_temp(mode_id, mode));
                }
            });
        }
        let existing = ["move", "leave", "discard"][mode.min(2) as usize];
        let mut result = None;
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W);
            if app.services.pick_folder.is_some() {
                let r = ui.button(crate::i18n::tr("Choose Folder…"));
                register(ui.ctx(), "button:settingsSmartChoose", r.rect);
                if r.clicked()
                    && let Some(dir) = app.services.pick_folder.as_mut().and_then(|f| f())
                {
                    result = Some(app.run("library.smartPreviewsLocation", json!({"path": dir, "existing": existing})));
                }
            }
            let r = ui.add_enabled(custom, egui::Button::new(crate::i18n::tr("Use Library Folder")));
            register(ui.ctx(), "button:settingsSmartReset", r.rect);
            if r.clicked() {
                result = Some(app.run("library.smartPreviewsLocation", json!({"reset": true, "existing": existing})));
            }
        });
        if let Some(r) = result {
            ui.data_mut(|d| d.remove::<(f64, Value)>(cache));
            match r {
                Ok(v) => {
                    let failed = v["failed"].as_array().map_or(0, Vec::len);
                    let msg = match (existing, v["handled"].as_u64().unwrap_or(0)) {
                        (_, 0) => "Smart previews folder changed".to_string(),
                        ("move", n) => format!("Smart previews folder changed; moved {n}"),
                        ("discard", n) => format!("Smart previews folder changed; deleted {n}"),
                        _ => "Smart previews folder changed".to_string(),
                    };
                    app.toast(
                        ui.ctx(),
                        if failed > 0 { crate::i18n::tr_format!("{msg} ({failed} failed)", failed = failed, msg = msg) } else { msg },
                    );
                }
                Err(e) => app.toast(ui.ctx(), e),
            }
        }
        hint(
            ui,
            t,
            crate::i18n::tr("Keep it on a drive with room: smart previews are about 1 MB per photo. The thumbnail cache stays in the library."),
        );
    }
}

// ---------------------------------------------------------------------------------- Interface

fn interface_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    crate::accents::section(app, ui, t);
    if heading(ui, t, "settings.interface.filmstrip", crate::i18n::tr("Filmstrip")) {
        check(ui, "settings.filmNames", &mut app.ui.settings.film_names, "Show file names");
        check(ui, "settings.filmBadges", &mut app.ui.settings.film_badges, "Show ratings, flags and edit badges");
    }
    if heading(ui, t, "settings.interface.grid", crate::i18n::tr("Grid")) {
        row(ui, t, crate::i18n::tr("Ratings & flags"), |ui| {
            choices(
                ui,
                "settingsGridBadges",
                &[(GridBadges::Auto, "When rated or hovered"), (GridBadges::Always, "Always"), (GridBadges::Never, "Never")],
                &mut app.ui.settings.grid_badges,
            );
        });
        check(ui, "settings.showFilenames", &mut app.ui.show_filenames, "Square Grid: show file names and formats");
        check(ui, "settings.gridIndex", &mut app.ui.settings.grid_index, "Square Grid: show each photo's index number");
        row(ui, t, crate::i18n::tr("Square Grid cells"), |ui| {
            choices(ui, "settingsGridStyle", &[(false, "compact."), (true, "expanded.")], &mut app.ui.settings.grid_expanded);
        });
        // LRC-LIB-GRIDOPTIONS: Cell Icons and Extras, one switch each
        check(ui, "settings.gridFlags", &mut app.ui.settings.grid_flags, "Show pick and reject flags");
        check(ui, "settings.gridEditBadge", &mut app.ui.settings.grid_edit_badge, "Show the edited badge");
        check(ui, "settings.gridTint", &mut app.ui.settings.grid_tint, "Tint grid cells with label colors");
        check(ui, "settings.gridRotate", &mut app.ui.settings.grid_rotate, "Expanded cells: include rotation buttons");
        check(ui, "settings.gridTooltips", &mut app.ui.settings.grid_tooltips, "Show image info tooltips");
    }
    if heading(ui, t, "settings.interface.detail", crate::i18n::tr("Detail")) {
        // the tool strip's quick tools (after its |): ticked = shown, in this order
        row(ui, t, crate::i18n::tr("Quick tools"), |ui| {
            ui.label(RichText::new(crate::i18n::tr("ticked = an icon after the tool strip's |")).color(t.text_dim));
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            egui::Grid::new("settingsQuickTools").num_columns(6).spacing(egui::vec2(12.0, 10.0)).show(ui, |ui| {
                for q in super::right::QUICK_TOOLS {
                    let mut on = app.ui.settings.quick_tools.iter().any(|x| x == q.id);
                    if check(ui, &format!("settings.{}", q.id), &mut on, q.label) {
                        let order: Vec<&str> = super::right::QUICK_TOOLS.iter().map(|x| x.id).collect();
                        let set = &mut app.ui.settings.quick_tools;
                        if on {
                            set.push(q.id.to_string());
                            set.sort_by_key(|x| order.iter().position(|o| o == x).unwrap_or(usize::MAX));
                        } else {
                            set.retain(|x| x != q.id);
                        }
                    }
                }
            });
        });
        check(ui, "settings.navigator", &mut app.ui.navigator, "Show the Navigator while zoomed in");
        // which zoom levels the zoom menus and the Navigator offer (ticked = shown); a fixed grid, so it
        // can't widen the dialog (a wrapping row did, and the tabs moved)
        row(ui, t, crate::i18n::tr("Zoom levels"), |ui| {
            ui.label(RichText::new(crate::i18n::tr("ticked = in the zoom menus and the Navigator")).color(t.text_dim));
        });
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            egui::Grid::new("settingsZoomLevels").num_columns(6).spacing(egui::vec2(12.0, 10.0)).show(ui, |ui| {
                let levels = super::develop_left::NAV_MORE_ZOOMS.iter().map(|&z| crate::state::zoom_pct_label(z));
                for (i, label) in levels.chain(["Fill".to_string(), "Fit".to_string()]).enumerate() {
                    let mut on = app.ui.settings.zoom_shown(&label);
                    if check(ui, &format!("settings.zoom-{label}"), &mut on, &label) {
                        let hidden = &mut app.ui.settings.hidden_zooms;
                        hidden.retain(|h| h != &label);
                        if !on {
                            hidden.push(label);
                        }
                    }
                    if i % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        });
        row(ui, t, crate::i18n::tr("Info overlay"), |ui| {
            use crate::state::InfoOverlay as I;
            choices(ui, "settingsInfo", &[(I::Off, "off."), (I::Basic, "info.1."), (I::Exposure, "info.2.")], &mut app.ui.info_overlay);
        });
        row(ui, t, crate::i18n::tr("Photo color"), |ui| {
            use crate::display_p3::ViewSpace as V;
            choices(ui, "settingsViewSpace", &[(V::Auto, "Auto"), (V::DisplayP3, "Display P3"), (V::Srgb, "sRGB")], &mut app.ui.settings.view_space);
        });
        ui.label(
            egui::RichText::new(crate::i18n::tr(
                "Auto: color-managed on a Mac (the window is tagged, and macOS shows photos through the display's own profile, as Preview and Lightroom do). Display P3 / sRGB: the values go to the display as they are, for a display of that space.",
            ))
            .color(t.text_dim),
        );
        let state = match app.window_space.as_ref().and_then(|w| w.tagged()) {
            Some("extendedSrgb") => "Window: color-managed (HDR, extended sRGB)",
            Some(_) => "Window: color-managed (Display P3)",
            None => "Window: not color-managed (values shown as they are)",
        };
        ui.label(egui::RichText::new(crate::i18n::tr(state)).color(t.text_dim));
        // L19: Loupe Info 1 / 2 lines (Library View Options ▸ Loupe View)
        loupe_info_rows(ui, t, &mut app.ui.settings.loupe_info, "", "settingsLoupeDefaults");
        check(ui, "settings.loadingMessage", &mut app.ui.settings.loading_message, "Show message when loading or rendering photos");
        hint(
            ui,
            t,
            "Show briefly: the info overlay appears for a moment when the loupe changes photo while it is off. Develop's loupe has its own (dev.view.).",
        );
    }
}

// ------------------------------------------------------------------------------------- identity.

/// LRC-UI-IDENTITYPLATE: Lightroom Classic's Identity Plate Setup, made better — an image or a
/// text plate, placed in the top bar, a window corner or a photo-area corner, with size, opacity,
/// margin, modules, a tint from a color wheel and a live preview.
fn identity_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    use crate::identity::{Place, PlateKind};
    let before = app.ui.settings.identity_plate.clone();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 20.0;
        ui.vertical(|ui| {
            ui.set_width(400.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            let p = &mut app.ui.settings.identity_plate;
            check(ui, "identityOn", &mut p.on, "Show an identity plate");
            row(ui, t, "Plate", |ui| {
                choices(ui, "identityKind", &[(PlateKind::Image, "image."), (PlateKind::Text, "text.")], &mut p.kind);
            });
            let mut pick = false;
            let mut clear = false;
            if p.kind == PlateKind::Image {
                row(ui, t, "Image", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let name = std::path::Path::new(&p.image).file_name().map(|n| n.to_string_lossy().to_string());
                    let shown = name.unwrap_or_else(|| crate::i18n::tr("none yet").to_string());
                    ui.allocate_ui_with_layout(egui::vec2(130.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(130.0);
                        ui.add(egui::Label::new(RichText::new(shown).color(t.text)).truncate()).on_hover_text(p.image.clone());
                    });
                    pick = crate::widgets::text_button(ui, "identityPick", "pick.", false)
                        .on_hover_text(crate::i18n::tr("Choose an image: PNG, SVG, WebP or JPEG (a transparent PNG or SVG works best)"))
                        .clicked();
                    clear = !p.image.is_empty() && crate::widgets::text_button(ui, "identityImageNone", "none.", false).clicked();
                });
            } else {
                row(ui, t, "Text", |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let r = ui.add_sized([170.0, 24.0], egui::TextEdit::singleline(&mut p.text).hint_text(crate::i18n::tr("your name or studio")));
                    register(ui.ctx(), "field:identityText", r.rect);
                    if crate::widgets::text_button(ui, "identityBold", "bold.", p.bold).clicked() {
                        p.bold = !p.bold;
                    }
                });
            }
            row(ui, t, "Window", |ui| {
                choices(
                    ui,
                    "identityPlace",
                    &[
                        (Place::Bar, "bar."),
                        (Place::Tl, "top.left."),
                        (Place::Tr, "top.right."),
                        (Place::Bl, "bottom.left."),
                        (Place::Br, "bottom.right."),
                    ],
                    &mut p.place,
                );
            });
            row(ui, t, "Photo area", |ui| {
                choices(
                    ui,
                    "identityCanvas",
                    &[
                        (Place::CanvasTl, "top.left."),
                        (Place::CanvasTr, "top.right."),
                        (Place::CanvasBl, "bottom.left."),
                        (Place::CanvasBr, "bottom.right."),
                    ],
                    &mut p.place,
                );
            });
            row(ui, t, "Size", |ui| {
                let r = ui.add(egui::Slider::new(&mut p.size, crate::identity::SIZE_RANGE).suffix(" pt").step_by(1.0));
                register(ui.ctx(), "slider:identitySize", r.rect);
            });
            row(ui, t, "Opacity", |ui| {
                let mut pct = p.opacity * 100.0;
                let r = ui.add(egui::Slider::new(&mut pct, 5.0..=100.0).suffix(" %").step_by(1.0));
                register(ui.ctx(), "slider:identityOpacity", r.rect);
                if r.changed() {
                    p.opacity = pct / 100.0;
                }
            });
            row(ui, t, "Margin", |ui| {
                let r = ui.add(egui::Slider::new(&mut p.margin, crate::identity::MARGIN_RANGE).suffix(" pt").step_by(1.0));
                register(ui.ctx(), "slider:identityMargin", r.rect);
            });
            row(ui, t, "Show in", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (id, label, on) in [
                    ("identityLibrary", "library.", &mut p.library),
                    ("identityDevelop", "develop.", &mut p.develop),
                    ("identityMap", "map.", &mut p.map),
                ] {
                    if crate::widgets::text_button(ui, id, label, *on).clicked() {
                        *on = !*on;
                    }
                }
                ui.add_space(8.0);
                if crate::widgets::text_button(ui, "identityAll", "all.", p.library && p.develop && p.map).clicked() {
                    (p.library, p.develop, p.map) = (true, true, true);
                }
            });
            hint(ui, t, "Decoration only: clicks go through it, it moves off buttons and fades while the pointer is over it.");
            if let Some(e) = app.identity.error.clone() {
                ui.label(RichText::new(format!("⚠ {e}")).color(t.caution));
            }
            if clear {
                let _ = app.run("app.identityPlate", json!({"image": ""}));
            }
            if pick {
                let picker = if app.services.pick_image.is_some() { app.services.pick_image.as_mut() } else { app.services.pick_files.as_mut() };
                if let Some(f) = picker.and_then(|pick| pick().into_iter().next())
                    && let Err(e) = app.run("app.identityPlate", json!({"image": f, "on": true}))
                {
                    app.toast_error(ui.ctx(), e);
                }
            }
        });
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            crate::identity::preview(app, ui, egui::vec2(240.0, 150.0));
            tint_picker(app, ui, t);
        });
    });
    if app.ui.settings.identity_plate != before {
        app.ui.settings.identity_plate.sanitize();
    }
}

/// The plate's tint: a hue / saturation wheel, brightness, a hex field, swatches and none.
fn tint_picker(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let p = &mut app.ui.settings.identity_plate;
    let cur = p.tint_rgb();
    let [h, s, v] = cur.map_or([0.0, 0.0, 1.0], |c| {
        lightcraft_color::perceptual::rgb_to_hsv([f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0])
    });
    let set = |p: &mut crate::identity::IdentityPlate, h: f32, s: f32, v: f32| {
        let c = lightcraft_color::perceptual::hsv_to_rgb(h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        p.tint = Some(crate::identity::to_hex([b(c[0]), b(c[1]), b(c[2])]));
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(RichText::new(crate::i18n::tr("Color")).color(t.text_label));
        if crate::widgets::text_button(ui, "identityTintNone", "none.", cur.is_none())
            .on_hover_text(crate::i18n::tr("The image's own colors"))
            .clicked()
        {
            p.tint = None;
        }
    });
    let rad = 52.0;
    let (area, _) = ui.allocate_exact_size(egui::vec2(240.0, rad * 2.0 + 4.0), egui::Sense::hover());
    let c = egui::pos2(area.left() + rad + 2.0, area.center().y);
    let wheel = egui::Rect::from_center_size(c, egui::vec2(rad * 2.0, rad * 2.0));
    let resp = ui.interact(wheel, egui::Id::new("identity-tint-wheel"), egui::Sense::click_and_drag());
    register(ui.ctx(), "wheel:identityTint", wheel);
    let gray = (v * 255.0).round() as u8;
    super::edit::paint_hue_wheel(ui.painter(), c, rad, egui::Color32::from_gray(gray), 1.0, v.max(0.05));
    if cur.is_some() {
        let a = h.to_radians();
        let dot = c + egui::vec2(a.cos(), -a.sin()) * rad * s;
        ui.painter().circle_filled(dot, 5.0, egui::Color32::WHITE);
        ui.painter().circle_stroke(dot, 5.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
    }
    if (resp.dragged() || resp.clicked())
        && let Some(q) = resp.interact_pointer_pos()
    {
        let d = q - c;
        let hue = ((-d.y).atan2(d.x).to_degrees() + 360.0) % 360.0;
        set(p, hue, (d.length() / rad).min(1.0), if cur.is_some() { v } else { 1.0 });
    }
    // right of the wheel: brightness, the hex value, swatches
    let col = egui::Rect::from_min_max(egui::pos2(wheel.right() + 14.0, area.top() + 4.0), area.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(col).layout(egui::Layout::top_down(egui::Align::Min)));
    child.spacing_mut().item_spacing.y = 10.0;
    child.spacing_mut().slider_width = col.width() - 8.0;
    let mut bright = v * 100.0;
    let r = child.add(egui::Slider::new(&mut bright, 0.0..=100.0).show_value(false));
    register(child.ctx(), "slider:identityBrightness", r.rect);
    let r = r.on_hover_text(crate::i18n::tr("Brightness"));
    if r.changed() {
        set(p, h, if cur.is_some() { s } else { 0.0 }, bright / 100.0);
    }
    let mut hex = p.tint.clone().unwrap_or_default();
    let r = child.add_sized([col.width() - 8.0, 24.0], egui::TextEdit::singleline(&mut hex).hint_text("#rrggbb"));
    register(child.ctx(), "field:identityTint", r.rect);
    if r.changed()
        && let Some(c) = crate::identity::parse_hex(&hex)
    {
        p.tint = Some(crate::identity::to_hex(c));
    }
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (name, hex) in crate::identity::SWATCHES {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            register(ui.ctx(), format!("swatch:identity-{name}"), r);
            let c = crate::identity::parse_hex(hex).map_or(egui::Color32::WHITE, |[r, g, b]| egui::Color32::from_rgb(r, g, b));
            ui.painter().rect_filled(r, 3.0, c);
            let on = p.tint.as_deref() == Some(hex);
            ui.painter().rect_stroke(
                r,
                3.0,
                egui::Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.field_border }),
                egui::StrokeKind::Outside,
            );
            if resp.on_hover_text(name).clicked() {
                p.tint = Some(hex.to_string());
            }
        }
    });
}

/// Loupe Info 1 / 2 (three lines each), Show Briefly and defaults. for one module's loupe
/// (`prefix` "" = Library, "dev" = Develop: widget ids `combo:{prefix}loupeInfo{n}-{i}`).
fn loupe_info_rows(ui: &mut egui::Ui, t: &Tokens, info: &mut crate::state::LoupeInfo, prefix: &str, defaults_id: &str) {
    for (n, which) in [(1usize, "info1"), (2, "info2")] {
        row(ui, t, &format!("Loupe info {n}"), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for i in 0..3 {
                let line = if n == 1 { &mut info.info1[i] } else { &mut info.info2[i] };
                let cur = *line;
                let mut pick = cur;
                let r = egui::ComboBox::from_id_salt(format!("{prefix}loupe-{which}-{i}"))
                    .width(128.0)
                    .selected_text(crate::i18n::tr(cur.label()))
                    .show_ui(ui, |ui| {
                        for f in crate::state::LoupeInfoField::ALL {
                            ui.selectable_value(&mut pick, *f, crate::i18n::tr(f.label()));
                        }
                    });
                register(ui.ctx(), format!("combo:{prefix}loupeInfo{n}-{i}"), r.response.rect);
                *line = pick;
            }
        });
    }
    row(ui, t, crate::i18n::tr("Show briefly"), |ui| {
        use crate::state::InfoOverlay as I;
        choices(ui, &format!("{prefix}settingsBriefInfo"), &[(I::Off, "never."), (I::Basic, "info.1."), (I::Exposure, "info.2.")], &mut info.briefly);
    });
    row(ui, t, "", |ui| {
        if crate::widgets::text_button(ui, defaults_id, "defaults.", false).clicked() {
            *info = crate::state::LoupeInfo::default();
        }
    });
}

// ------------------------------------------------------------------------------ dev.view.

/// LRC-DEV-VIEWOPTIONS: Develop's View Options (⌘J in Develop) — its own info overlay and Loupe
/// Info lines, the loading message, and the Video options.
/// Settings ▸ Develop ▸ Center mask: what the tool strip's centre mask lands on (a starting point;
/// each mask stays adjustable in Masking).
fn center_mask_rows(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    if heading(ui, t, "settings.develop.center.mask", crate::i18n::tr("Center mask")) {
        let mut m = app.ui.settings.center_mask.clone().unwrap_or_else(crate::menus::default_center_mask);
        let before = m.clone();
        let num = |m: &Value, path: &[&str], d: f64| path.iter().try_fold(m, |v, k| v.get(*k)).and_then(Value::as_f64).unwrap_or(d);
        let mut outside = m.pointer("/shape/invert").and_then(Value::as_bool).unwrap_or(true);
        row(ui, t, crate::i18n::tr("Effect on"), |ui| {
            let mut i = usize::from(outside);
            if choices(ui, "settingsCenterSide", &[(0usize, "inside."), (1usize, "outside.")], &mut i) {
                outside = i == 1;
            }
        });
        let mut feather = num(&m, &["shape", "feather"], 85.0);
        let mut size = num(&m, &["shape", "rx"], 0.38) * 100.0;
        let mut ev = num(&m, &["adjust", "exposure"], -0.6);
        row(ui, t, crate::i18n::tr("Feather"), |ui| {
            let r = ui.add(egui::Slider::new(&mut feather, 0.0..=100.0).fixed_decimals(0));
            register(ui.ctx(), "slider:centerFeather", r.rect);
        });
        row(ui, t, crate::i18n::tr("Size"), |ui| {
            let r = ui.add(egui::Slider::new(&mut size, 10.0..=90.0).fixed_decimals(0).suffix("%"));
            register(ui.ctx(), "slider:centerSize", r.rect);
        });
        row(ui, t, crate::i18n::tr("Exposure"), |ui| {
            let r = ui.add(egui::Slider::new(&mut ev, -4.0..=4.0).fixed_decimals(2));
            register(ui.ctx(), "slider:centerExposure", r.rect);
        });
        let rx = size / 100.0;
        let ry = (rx * num(&before, &["shape", "ry"], 0.30) / num(&before, &["shape", "rx"], 0.38).max(0.01)).clamp(0.05, 1.0);
        m["shape"]["invert"] = json!(outside);
        m["shape"]["feather"] = json!(feather);
        m["shape"]["rx"] = json!(rx);
        m["shape"]["ry"] = json!(ry);
        m["adjust"]["exposure"] = json!(ev);
        if m != before {
            app.ui.settings.center_mask = Some(m);
        }
        row(ui, t, "", |ui| {
            if crate::widgets::text_button(ui, "centerMaskReset", "reset.", false).clicked() {
                app.ui.settings.center_mask = None;
            }
        });
        hint(
            ui,
            t,
            "The tool strip's centre mask lands on this; adjust it on each photo in Masking, and right-click the icon to make a photo's mask the new default.",
        );
    }
}

fn develop_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    if heading(ui, t, "settings.develop.develop.loupe", crate::i18n::tr("Develop loupe")) {
        row(ui, t, crate::i18n::tr("Info overlay"), |ui| {
            use crate::state::InfoOverlay as I;
            choices(ui, "settingsDevInfo", &[(I::Off, "off."), (I::Basic, "info.1."), (I::Exposure, "info.2.")], &mut app.ui.develop_info_overlay);
        });
        loupe_info_rows(ui, t, &mut app.ui.settings.develop_view.loupe_info, "dev", "settingsDevLoupeDefaults");
        center_mask_rows(app, ui, t);
    }
    if heading(ui, t, "settings.develop.general", crate::i18n::tr("General")) {
        check(ui, "settings.devLoadingMessage", &mut app.ui.settings.develop_view.loading_message, "Show message when loading or rendering photos");
    }
    if heading(ui, t, "settings.develop.video", crate::i18n::tr("Video")) {
        check(ui, "settings.videoFrames", &mut app.ui.settings.develop_view.video_frames, "Show frame number when displaying video time");
        check(ui, "settings.videoDraft", &mut app.ui.settings.develop_view.video_draft, "Play HD video at draft quality");
        hint(ui, t, "These apply to the loupe in Develop (I / ⌘I cycle its overlay there); the Library loupe keeps its own on Interface.");
    }
}

// ------------------------------------------------------------------------------- Open Library…

/// `app.openLibrary {path?}`: close the current library and open (or create) the one at `path`,
/// or a folder chosen in a dialog. Remembered as the library to open at launch.
pub fn open_library(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let path = match p.get("path").and_then(Value::as_str) {
        Some(x) => x.to_string(),
        None => match app.services.pick_folder.as_mut() {
            Some(pick) => match pick() {
                Some(x) => x,
                None => return Ok(Value::Null),
            },
            None => return Err("no folder dialog on this platform".into()),
        },
    };
    app.session.close_library().map_err(|e| e.to_string())?;
    app.session.open_library(&path, false).map_err(|e| e.to_string())?;
    app.renderer.forget_all();
    app.ui.compare = None;
    app.ui.settings.library_path = path.clone();
    Ok(json!({"path": path, "photos": app.session.catalog.len()}))
}

// ------------------------------------------------------------------------------------ shrt.

/// Keys recorded on the shrt. page: user binding `target` gets them, or (no target) the draft
/// action is bound to them.
pub fn recorded(app: &mut LightcraftApp, target: Option<usize>, keys: &str) -> Result<(), String> {
    let mut f = app.keymap.file.clone();
    let combo = crate::keymap::parse_combo(keys);
    match target {
        Some(i) => {
            let b = f.bindings.get_mut(i).ok_or("that binding is gone")?;
            b.keys = keys.to_string();
        }
        None => {
            let mut b = app.keymap.draft.clone();
            if b.command.is_empty() && b.label.is_empty() {
                return Err("pick an action first".into());
            }
            b.keys = keys.to_string();
            f.bindings.retain(|x| crate::keymap::parse_combo(&x.keys) != combo);
            f.bindings.push(b);
        }
    }
    app.keymap.set(f)
}

/// A fixed-width, left-aligned table cell (long text is cut with …).
fn cell(ui: &mut egui::Ui, w: f32, text: RichText) {
    ui.allocate_ui_with_layout(egui::vec2(w, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add(egui::Label::new(text).truncate());
    });
}

fn save_keymap(app: &mut LightcraftApp, f: crate::keymap::KeymapFile) {
    if let Err(e) = app.keymap.set(f) {
        app.toast_soon(e, 6.0);
    }
}

/// The super-key actions offered for a new binding: `(label, command, params)`.
fn draft_actions(app: &LightcraftApp) -> Vec<(String, String, Value)> {
    let mut v: Vec<(String, String, Value)> = crate::keymap::SUPER_ACTIONS
        .iter()
        .map(|(_, l, c, p)| (l.to_string(), c.to_string(), serde_json::from_str(p).unwrap_or(json!({}))))
        .collect();
    let control = app.keymap.draft.params.get("control").and_then(Value::as_str).unwrap_or("light.exposure").to_string();
    let label = lightcraft_develop::controls::find(&control).map(|c| c.label.to_lowercase()).unwrap_or(control.clone());
    for (dir, size, what) in [(1, "small", "+"), (-1, "small", "-"), (1, "large", "++"), (-1, "large", "--")] {
        v.push((format!("{label} {what}"), "keys.nudge".into(), json!({"control": control, "dir": dir, "size": size})));
    }
    v.push((format!("reset {label}"), "develop.resetControl".into(), json!({"control": control})));
    // every crop ratio on its own key (Bryan, 2026-11-23), …
    for (l, a) in [
        ("As Shot", "original"),
        ("1 × 1", "1x1"),
        ("4 × 5 / 8 × 10", "4x5"),
        ("5 × 7", "5x7"),
        ("8.5 × 11", "85x110"),
        ("2 × 3", "2x3"),
        ("3 × 2", "3x2"),
        ("4 × 3", "4x3"),
        ("16 × 10", "16x10"),
        ("16 × 9", "16x9"),
        ("9 × 16", "9x16"),
        ("2 × 1", "2x1"),
        ("21 × 9", "21x9"),
        ("XPan 65 × 24", "65x24"),
        ("3 × 1", "3x1"),
    ] {
        v.push((format!("crop ratio {l}"), "crop.aspect".into(), json!({"aspect": a})));
    }
    // … each of the user's presets, …
    for p in app.session.presets.iter().filter(|p| !p.builtin) {
        v.push((format!("preset: {}", p.name), "preset.applyByName".into(), json!({"name": p.name})));
    }
    // … and every other command of the app (its menu name)
    let have: std::collections::HashSet<String> =
        v.iter().filter(|(_, _, p)| p.as_object().is_none_or(|o| o.is_empty())).map(|(_, c, _)| c.clone()).collect();
    for (id, l, _, _) in crate::menus::ui_commands() {
        if !have.contains(*id) {
            v.push((crate::i18n::tr(l).to_string(), id.to_string(), json!({})));
        }
    }
    for c in lightcraft_engine::command_specs() {
        if !have.contains(c.id) && !v.iter().any(|(_, id, _)| id == c.id) {
            v.push((c.label.to_string(), c.id.to_string(), json!({})));
        }
    }
    v.push(("nothing (unbind)".into(), String::new(), Value::Null));
    v
}

fn import_result(app: &mut LightcraftApp, r: Result<Value, String>) {
    let text = match r {
        Ok(v) => format!(
            "imported {} keys, {} midi, {} steps · {} not mapped",
            v["bindings"].as_u64().unwrap_or(0),
            v["midi"].as_u64().unwrap_or(0),
            v["steps"].as_u64().unwrap_or(0),
            v["skipped"].as_array().map_or(0, Vec::len)
        ),
        Err(e) => e,
    };
    app.toast_soon(text, 5.0);
}

fn shortcuts_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut f = app.keymap.file.clone();
    row(ui, t, "Profile", |ui| {
        let mut p = if f.profile.is_empty() { "lightroom".to_string() } else { f.profile.clone() };
        let opts: Vec<(&str, &str)> = crate::keymap::PROFILES.iter().map(|(id, l)| (*id, *l)).collect();
        let mut sel = opts.iter().position(|(id, _)| *id == p).unwrap_or(0);
        let labels: Vec<(usize, &str)> = opts.iter().enumerate().map(|(i, (_, l))| (i, *l)).collect();
        if choices(ui, "keysProfile", &labels, &mut sel) {
            p = opts.get(sel).map(|(id, _)| id.to_string()).unwrap_or_default();
            f.profile = p;
            save_keymap(app, f.clone());
        }
    });
    hint(
        ui,
        t,
        crate::i18n::tr(
            "lr.p.picks.: Lightroom's keys with P = pick, Z = zoom. lr.z.picks.: Lightroom desktop exactly (Z = pick, P = photos panel, Space = zoom). lr.classic.: Lightroom Classic.",
        ),
    );
    row(ui, t, "Import", |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (id, label, source) in [("keysImportLrkeys", "lrkeys.", "lrkeys"), ("keysImportSuperkeys", "superkeys.", "superkeys")] {
            if crate::widgets::text_button(ui, id, label, false).clicked() {
                let r = crate::keymap::import(app, &json!({"source": source}));
                import_result(app, r);
            }
        }
        if crate::widgets::text_button(ui, "keysReset", "reset.", false).clicked() {
            let mut g = app.keymap.file.clone();
            g.bindings.clear();
            save_keymap(app, g);
        }
    });
    if let Some(e) = &app.keymap.error {
        ui.label(RichText::new(format!("⚠ {e}")).color(t.caution));
    } else if let Some(p) = &app.keymap.path {
        hint(ui, t, &format!("{} · edits here save at once; edits to the file reload by themselves", p.display()));
    }
    ui.add_space(8.0);
    // search: the bound keys below AND every action that can be bound (Bryan, 2026-11-23)
    let search_id = egui::Id::new("keysSearch");
    let mut q: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    row(ui, t, "Search", |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("keys, action or command (crop, preset, flag…)").desired_width(300.0));
        register(ui.ctx(), "field:keysSearch", r.rect);
    });
    ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
    let ql = q.to_lowercase();
    ui.add_space(8.0);
    // a new binding: pick an action, then rec. and press the keys
    let all_actions = draft_actions(app);
    let actions: Vec<(String, String, Value)> =
        all_actions.iter().filter(|(l, c, _)| ql.is_empty() || l.to_lowercase().contains(&ql) || c.to_lowercase().contains(&ql)).cloned().collect();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(egui::vec2(LABEL_W, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            ui.label(RichText::new("Add").color(t.text_label));
        });
        let d = app.keymap.draft.clone();
        let current = if d.command.is_empty() && d.label.is_empty() {
            "pick an action".to_string()
        } else {
            actions
                .iter()
                .find(|(l, c, p)| *c == d.command && *p == d.params && (d.label.is_empty() || *l == d.label))
                .map(|(l, ..)| l.clone())
                .unwrap_or(d.label.clone())
        };
        let r = egui::ComboBox::from_id_salt("keysDraftAction").width(200.0).selected_text(current).show_ui(ui, |ui| {
            for (l, c, p) in &actions {
                if ui.selectable_label(d.command == *c && d.params == *p, l).clicked() {
                    app.keymap.draft = crate::keymap::Binding::new("", c, p.clone(), l);
                }
            }
        });
        register(ui.ctx(), "combo:keysDraftAction", r.response.rect);
        // the slider the nudge / reset actions move
        let control = app.keymap.draft.params.get("control").and_then(Value::as_str).unwrap_or("light.exposure").to_string();
        let r = egui::ComboBox::from_id_salt("keysDraftControl").width(150.0).selected_text(control.clone()).show_ui(ui, |ui| {
            for c in lightcraft_develop::controls::CONTROLS.iter().map(|c| c.id).chain(crate::keymap::CROP_PSEUDO.iter().copied()) {
                if ui.selectable_label(c == control, c).clicked() {
                    let d = &mut app.keymap.draft;
                    if d.params.get("control").is_some() {
                        d.params["control"] = json!(c);
                        d.label.clear();
                    } else {
                        *d = crate::keymap::Binding::new("", "keys.nudge", json!({"control": c, "dir": 1, "size": "small"}), "");
                    }
                }
            }
        });
        register(ui.ctx(), "combo:keysDraftControl", r.response.rect);
        let rec = app.keymap.recording == Some(None);
        if crate::widgets::text_button(ui, "keysRecord", if rec { "press.keys…" } else { "record." }, rec).clicked() {
            app.keymap.recording = if rec { None } else { Some(None) };
        }
    });
    if !ql.is_empty() {
        ui.label(RichText::new(format!("{} actions match — pick one under Add, then rec.", actions.len())).size(11.0).color(t.text_dim));
    }
    let rows: Vec<crate::keymap::Row> = crate::keymap::rows(&f)
        .into_iter()
        .filter(|r| {
            ql.is_empty() || r.keys.to_lowercase().contains(&ql) || r.label.to_lowercase().contains(&ql) || r.command.to_lowercase().contains(&ql)
        })
        .collect();
    ui.add_space(4.0);
    // one fixed-size list that scrolls inside
    egui::ScrollArea::vertical().id_salt("keysList").max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
        let mut group = String::new();
        let mut group_open = true;
        for (n, r) in rows.iter().enumerate() {
            if r.group != group {
                group = r.group.clone();
                ui.add_space(6.0);
                // each group folds (remembered, crate::folds)
                group_open = crate::folds::header_sized(ui, &format!("settings.shortcuts.group:{group}"), &group, 11.5, 22.0);
            }
            if !group_open {
                continue;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let keys = RichText::new(crate::keymap::display(&r.keys)).font(t.semibold(11.5));
                let keys = if r.overridden { keys.strikethrough().color(t.text_dim) } else { keys.color(t.text) };
                cell(ui, 110.0, keys);
                let mut text = r.label.clone();
                if r.command.is_empty() {
                    text = "(unbound)".into();
                }
                cell(ui, 220.0, RichText::new(text).color(t.text));
                let origin = match r.origin {
                    crate::keymap::Origin::Builtin => "built-in",
                    crate::keymap::Origin::Profile => "classic",
                    crate::keymap::Origin::User => "mine",
                };
                cell(ui, 52.0, RichText::new(origin).size(10.5).color(t.text_dim));
                if r.conflict {
                    ui.label(RichText::new("⚠ conflict").size(10.5).color(t.caution));
                }
                if r.origin == crate::keymap::Origin::User {
                    let idx = app
                        .keymap
                        .file
                        .bindings
                        .iter()
                        .position(|b| crate::keymap::canonical(&b.keys) == r.keys && b.command == r.command && b.params == r.params);
                    if let Some(i) = idx {
                        let rec = app.keymap.recording == Some(Some(i));
                        if crate::widgets::text_button(ui, &format!("keysRec-{n}"), if rec { "press…" } else { "record." }, rec).clicked() {
                            app.keymap.recording = if rec { None } else { Some(Some(i)) };
                        }
                        if crate::widgets::text_button(ui, &format!("keysDel-{n}"), "delete.", false).clicked() {
                            let mut g = app.keymap.file.clone();
                            if i < g.bindings.len() {
                                g.bindings.remove(i);
                            }
                            save_keymap(app, g);
                        }
                    }
                } else if !r.overridden {
                    // rebind a built-in / profile action to other keys, or switch its keys off
                    if crate::widgets::text_button(ui, &format!("keysRec-{n}"), "record.", false).clicked() {
                        app.keymap.draft = crate::keymap::Binding::new("", &r.command, r.params.clone(), &r.label);
                        app.keymap.recording = Some(None);
                    }
                    if crate::widgets::text_button(ui, &format!("keysOff-{n}"), "off.", false).clicked() {
                        let mut g = app.keymap.file.clone();
                        g.bindings.push(crate::keymap::Binding::new(&r.keys, "", Value::Null, ""));
                        save_keymap(app, g);
                    }
                }
            });
        }
        if q.is_empty() && f.profile == "classic" {
            ui.add_space(6.0);
            if crate::folds::header_sized(ui, "settings.shortcuts.unmatched", "Lightroom Classic keys with no LightCraft command", 11.5, 22.0) {
                for (k, what) in crate::keymap::CLASSIC_SKIPPED {
                    hint(ui, t, &format!("{k} — {what}"));
                }
            }
        }
    });
}

// ------------------------------------------------------------------------------------ ctrl.

fn controllers_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let mut f = app.keymap.file.clone();
    let mut on = f.midi_enabled;
    if check(ui, "settings.midiEnabled", &mut on, "MIDI in (Monogram Creator, any MIDI controller)") {
        f.midi_enabled = on;
        save_keymap(app, f.clone());
    }
    if !app.midi_status.is_empty() {
        hint(ui, t, &app.midi_status);
    }
    hint(ui, t, "Mouse back / forward / middle: bind them on the shrt. page (rec., then click).");
    row(ui, t, "Monogram", |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if crate::widgets::text_button(ui, "midiImportMonogram", "monogram.", false).clicked() {
            let out = app.keymap.path.as_ref().and_then(|p| p.parent()).map(|d| d.join("LightCraft.monogram").display().to_string());
            let r = crate::keymap::import(app, &json!({"source": "monogram", "write": out}));
            import_result(app, r);
        }
        hint(ui, t, "maps the \"Lightroom 1\" profile (read only)");
    });
    ui.add_space(8.0);
    // MIDI learn: move a control, then say what it does
    let learn = app.keymap.learn;
    row(ui, t, "Learn", |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        match learn {
            None => {
                if crate::widgets::text_button(ui, "midiLearn", "learn.", false).clicked() {
                    app.keymap.learn = Some(None);
                }
            }
            Some(None) => {
                ui.label(RichText::new("move a slider or dial, or press a button…").color(t.text));
                if crate::widgets::text_button(ui, "midiLearnCancel", "cancel.", false).clicked() {
                    app.keymap.learn = None;
                }
            }
            Some(Some(m)) => {
                let what = match m.kind {
                    crate::keymap::MidiKind::Cc => format!("cc {} · ch {}", m.number, m.channel),
                    _ => format!("note {} · ch {}", m.number, m.channel),
                };
                ui.label(RichText::new(what).font(t.semibold(11.5)).color(t.text));
                let key = egui::Id::new("midiLearnTarget");
                let (mut control, mut dial, mut action): (String, bool, usize) =
                    ui.data(|d| d.get_temp(key)).unwrap_or(("light.exposure".into(), false, 0));
                let cc = m.kind == crate::keymap::MidiKind::Cc;
                if cc {
                    let r = egui::ComboBox::from_id_salt("midiLearnControl").width(150.0).selected_text(control.clone()).show_ui(ui, |ui| {
                        for c in lightcraft_develop::controls::CONTROLS.iter().map(|c| c.id).chain(crate::keymap::CROP_PSEUDO.iter().copied()) {
                            if ui.selectable_label(c == control, c).clicked() {
                                control = c.to_string();
                            }
                        }
                    });
                    register(ui.ctx(), "combo:midiLearnControl", r.response.rect);
                    check(ui, "midiLearnDial", &mut dial, "dial (endless)");
                } else {
                    let acts = crate::keymap::SUPER_ACTIONS;
                    let r = egui::ComboBox::from_id_salt("midiLearnAction")
                        .width(180.0)
                        .selected_text(acts.get(action).map(|a| a.1).unwrap_or(""))
                        .show_ui(ui, |ui| {
                            for (i, a) in acts.iter().enumerate() {
                                if ui.selectable_label(i == action, a.1).clicked() {
                                    action = i;
                                }
                            }
                        });
                    register(ui.ctx(), "combo:midiLearnAction", r.response.rect);
                }
                ui.data_mut(|d| d.insert_temp(key, (control.clone(), dial, action)));
                if crate::widgets::text_button(ui, "midiLearnAdd", "add.", false).clicked() {
                    let (cmd, params) = crate::keymap::SUPER_ACTIONS
                        .get(action)
                        .map(|a| (a.2.to_string(), serde_json::from_str(a.3).unwrap_or(json!({}))))
                        .unwrap_or_default();
                    let b = crate::keymap::learned_binding(&m, if cc { &control } else { "" }, dial, &cmd, params);
                    let mut g = app.keymap.file.clone();
                    g.midi.retain(|x| !(x.midi == b.midi && x.number == b.number && x.channel == b.channel));
                    g.midi.push(b);
                    app.keymap.learn = None;
                    save_keymap(app, g);
                }
                if crate::widgets::text_button(ui, "midiLearnCancel", "cancel.", false).clicked() {
                    app.keymap.learn = None;
                }
            }
        }
    });
    if let Some(m) = app.keymap.last_midi {
        hint(ui, t, &format!("last message: {:?} {} = {} (ch {})", m.kind, m.number, m.value, m.channel));
    }
    ui.add_space(8.0);
    egui::ScrollArea::vertical().id_salt("midiList").max_height(260.0).min_scrolled_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
        let list = app.keymap.file.midi.clone();
        if list.is_empty() {
            hint(ui, t, "No MIDI mappings yet: learn. one, or monogram. to bring in the Lightroom profile");
        }
        for (i, b) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                cell(ui, 100.0, RichText::new(format!("{} {} · ch {}", b.midi, b.number, b.channel)).font(t.semibold(11.5)));
                let what = if !b.control.is_empty() {
                    let name = lightcraft_develop::controls::find(&b.control).map(|c| c.label.to_string()).unwrap_or(b.control.clone());
                    format!("{name} ({})", if b.mode == "rel" { "dial" } else { "slider" })
                } else {
                    let p =
                        b.params.as_object().filter(|o| !o.is_empty()).map(|o| {
                            o.values().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join(", ")
                        });
                    match p {
                        Some(p) => format!("{} · {p}", crate::keymap::command_label(&b.command)),
                        None => crate::keymap::command_label(&b.command),
                    }
                };
                cell(ui, 280.0, RichText::new(what).color(t.text));
                cell(ui, 110.0, RichText::new(&b.label).size(10.5).color(t.text_dim));
                if crate::widgets::text_button(ui, &format!("midiDel-{i}"), "delete.", false).clicked() {
                    let mut g = app.keymap.file.clone();
                    if i < g.midi.len() {
                        g.midi.remove(i);
                    }
                    save_keymap(app, g);
                }
            });
        }
    });
}

// ------------------------------------------------------------------------------------- Cameras

/// The cameras. list, kept until the catalog changes or a command here changes it.
fn cameras_list(app: &mut LightcraftApp, ctx: &egui::Context, force: bool) -> Value {
    let key = egui::Id::new("settingsCamerasList");
    let rev = app.session.catalog.revision;
    if !force
        && let Some((r, v)) = ctx.data(|d| d.get_temp::<(u64, Value)>(key))
        && r == rev
    {
        return v;
    }
    let v = app.session.execute("cameras.list", &json!({})).unwrap_or_default();
    ctx.data_mut(|d| d.insert_temp(key, (rev, v.clone())));
    v
}

/// What colours a camera, in a few words.
fn camera_profile_text(c: &Value) -> String {
    let p = &c["profile"];
    let matching = p["matching"].as_array().map_or(0, Vec::len);
    match p["status"].as_str() {
        Some("adobe") if matching > 0 => format!("Adobe Standard DCP + {matching} camera matching"),
        Some("adobe") => "Adobe Standard DCP".to_string(),
        Some("embedded") => "the DNG's own profile".to_string(),
        Some("own") => "LightCraft's fitted profile".to_string(),
        Some(_) => "fallback (no Adobe profile on this Mac)".to_string(),
        None => "not resolved yet: refresh.".to_string(),
    }
}

fn camera_decoder_text(c: &Value) -> String {
    let d = c["decoder"].as_str().unwrap_or("");
    let fmt = c["format"].as_str().map(|f| format!(" ({f})")).unwrap_or_default();
    match d {
        "native" => format!("LightCraft's own{fmt}"),
        "libraw" => format!("LibRaw{fmt}"),
        "embedded preview" => "the camera's embedded JPEG (raw not decoded yet)".to_string(),
        "rendered" => format!("rendered files only{fmt}"),
        "unknown" => format!("unknown — {}", c["decoderNote"].as_str().unwrap_or("no file to read")),
        _ if c["inLibrary"] == false => "— (no photos yet)".to_string(),
        _ => "—".to_string(),
    }
}

/// A fixed-width, 18-point-high cell (the cameras. lines sit closer than the dialog's 24-point rows).
fn camera_cell(ui: &mut egui::Ui, w: f32, text: RichText) {
    ui.allocate_ui_with_layout(egui::vec2(w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.set_max_width(w);
        ui.add(egui::Label::new(text).truncate());
    });
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn cameras_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let ctx = ui.ctx().clone();
    let mut force = false;
    if heading(ui, t, "settings.cameras.cameras", crate::i18n::tr("Cameras")) {
        hint(ui, t, "The cameras in this library, what decodes and colors each, and the lens profiles found for their lenses.");
        // add a camera: type (suggestions from Camera Raw's cameras on this Mac) or pick one
        let qkey = egui::Id::new("settingsCamerasQuery");
        let mut q: String = ctx.data(|d| d.get_temp(qkey)).unwrap_or_default();
        ui.add_space(4.0);
        row(ui, t, crate::i18n::tr("Add camera"), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("type a model, e.g. Canon EOS R5").desired_width(240.0));
            register(ui.ctx(), "field:settingsCameraAdd", r.rect);
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (crate::widgets::text_button(ui, "settingsCameraAdd", "add.", false).clicked() || enter) && !q.trim().is_empty() {
                match app.run("cameras.add", json!({"name": q.trim()})) {
                    Ok(v) => {
                        app.toast(&ctx, v["notice"].as_str().unwrap_or("Camera added").replace("New camera", "Camera added"));
                        q.clear();
                    }
                    Err(e) => app.toast_error(&ctx, e),
                }
                force = true;
            }
            if crate::widgets::text_button(ui, "settingsCamerasRefresh", "refresh.", false).clicked() {
                match app.run("cameras.refresh", json!({"all": true})) {
                    Ok(v) => {
                        let n = v["refreshed"].as_array().map_or(0, Vec::len);
                        app.toast(&ctx, format!("Refreshed {n} camera{}", plural(n)));
                    }
                    Err(e) => app.toast_error(&ctx, e),
                }
                force = true;
            }
        });
        if q.trim().chars().count() >= 2 {
            let known = app.session.execute("cameras.known", &json!({"query": q.trim(), "limit": 6})).unwrap_or_default();
            let names: Vec<String> = known.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
            if !names.is_empty() {
                row(ui, t, "", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
                        for (i, n) in names.iter().enumerate() {
                            if crate::widgets::text_button(ui, &format!("settingsCameraPick-{i}"), n, false).clicked() {
                                q = n.clone();
                            }
                        }
                    });
                });
            }
        }
        ctx.data_mut(|d| d.insert_temp(qkey, q));
        ui.add_space(8.0);
        // the first time the page opens this session: resolve the cameras not resolved yet (one file
        // read per camera, then kept in cameras.json)
        let auto_key = egui::Id::new("settingsCamerasAutoRefreshed");
        if !ctx.data(|d| d.get_temp::<bool>(auto_key).unwrap_or(false)) {
            ctx.data_mut(|d| d.insert_temp(auto_key, true));
            if let Err(e) = app.run("cameras.refresh", json!({})) {
                log::warn!("cameras: {e}");
            }
            force = true;
        }
        let list = cameras_list(app, &ctx, force);
        let cams: Vec<Value> = list["cameras"].as_array().cloned().unwrap_or_default();
        if list["adobe"] == false {
            hint(ui, t, "Adobe's Camera Raw data or the DNG SDK isn't available here: every camera uses LightCraft's own color.");
        }
        let open_key = egui::Id::new("settingsCamerasOpen");
        let mut open: Vec<String> = ctx.data(|d| d.get_temp(open_key)).unwrap_or_default();
        let mut remove: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("camerasList").max_height(300.0).min_scrolled_height(300.0).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if cams.is_empty() {
                hint(ui, t, "No cameras yet: import photos, or add one above.");
            }
            for (i, c) in cams.iter().enumerate() {
                let name = c["name"].as_str().unwrap_or("").to_string();
                if i > 0 {
                    ui.add_space(14.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let is_open = open.contains(&name);
                    let photos = c["photos"].as_u64().unwrap_or(0) as usize;
                    ui.label(RichText::new(&name).font(t.semibold(12.5)).color(t.text));
                    let count = if photos == 0 { "added by hand".to_string() } else { format!("{photos} photo{}", plural(photos)) };
                    ui.label(RichText::new(count).size(11.0).color(t.text_dim));
                    let lenses = c["lenses"].as_array().map_or(0, Vec::len);
                    if lenses > 0 && crate::widgets::text_button(ui, &format!("settingsCameraLenses-{i}"), "lenses.", is_open).clicked() {
                        if is_open {
                            open.retain(|n| n != &name);
                        } else {
                            open.push(name.clone());
                        }
                    }
                    if c["added"] == true && crate::widgets::text_button(ui, &format!("settingsCameraRemove-{i}"), "remove.", false).clicked() {
                        remove = Some(name.clone());
                    }
                });
                let line = |ui: &mut egui::Ui, label: &str, text: String| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        camera_cell(ui, 90.0, RichText::new(crate::i18n::tr(label)).size(11.0).color(t.text_label));
                        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(t.text)).truncate());
                    });
                };
                line(ui, "decoder", camera_decoder_text(c));
                line(ui, "profile", camera_profile_text(c));
                let lens_list = c["lenses"].as_array().cloned().unwrap_or_default();
                let with = lens_list.iter().filter(|l| l["profile"].is_string()).count();
                let lens_text = if lens_list.is_empty() {
                    match c["makeLensProfiles"].as_u64() {
                        Some(n) if n > 0 => format!("none seen yet · {n} profiles installed for {}", c["make"].as_str().unwrap_or("its make")),
                        _ => "none seen yet".to_string(),
                    }
                } else if c["resolved"] != true {
                    format!("{} lens{} (not resolved yet)", lens_list.len(), if lens_list.len() == 1 { "" } else { "es" })
                } else {
                    format!("profiles for {with} of {} lens{}", lens_list.len(), if lens_list.len() == 1 { "" } else { "es" })
                };
                line(ui, "lenses", lens_text);
                let de = match c["deltaE"]["de"].as_f64() {
                    Some(de) => {
                        let n = c["deltaE"]["photos"].as_u64().map(|n| format!(" · {n} photos")).unwrap_or_default();
                        let when = c["deltaE"]["date"].as_str().or(c["deltaE"]["tag"].as_str()).map(|d| format!(" · {d}")).unwrap_or_default();
                        format!("{de:.2} ΔE{n}{when}")
                    }
                    None => "not measured (tools/lr-compare)".to_string(),
                };
                line(ui, "vs Lightroom", de);
                if open.contains(&name) {
                    for l in &lens_list {
                        let profile = l["profile"].as_str().map(str::to_string).unwrap_or_else(|| "no profile on this Mac".to_string());
                        let n = l["photos"].as_u64().unwrap_or(0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            camera_cell(ui, 90.0, RichText::new(""));
                            camera_cell(ui, 210.0, RichText::new(format!("{} ({n})", l["lens"].as_str().unwrap_or(""))).size(11.0).color(t.text));
                            ui.add(egui::Label::new(RichText::new(profile).size(10.5).color(t.text_dim)).truncate());
                        });
                    }
                }
            }
        });
        ctx.data_mut(|d| d.insert_temp(open_key, open));
        if let Some(name) = remove {
            if let Err(e) = app.run("cameras.remove", json!({"name": name})) {
                app.toast_error(&ctx, e);
            }
            cameras_list(app, &ctx, true);
        }
        if let Some(p) = list["cache"].as_str() {
            ui.add_space(4.0);
            hint(ui, t, &format!("Kept in {p}"));
        }
    }
}
