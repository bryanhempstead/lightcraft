//! The Settings dialog (⌘,): General, Import, Performance, Interface.
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
    ("interface", "Interface"),
    ("shortcuts", "shrt."),
    ("controllers", "ctrl."),
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
        "interface" => interface_tab(app, ui, &t),
        "shortcuts" => shortcuts_tab(app, ui, &t),
        "controllers" => controllers_tab(app, ui, &t),
        _ => general_tab(app, ui, &t),
    }
}

fn heading(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(crate::i18n::tr(text)).font(t.semibold(12.5)).color(t.text));
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

fn hint(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(crate::i18n::tr(text)).size(11.0).color(t.text_dim));
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
    row(ui, t, crate::i18n::tr("Language"), |ui| {
        let languages: Vec<_> = crate::i18n::Locale::ALL.iter().map(|language| (*language, language.name())).collect();
        choices(ui, "settingsLanguage", &languages, &mut app.ui.language);
        crate::i18n::set_language(app.ui.language);
    });
    heading(ui, t, crate::i18n::tr("Library"));
    let location = match &app.session.library {
        Some(l) => l.dir.display().to_string(),
        None => crate::i18n::tr("In memory — nothing is saved").to_string(),
    };
    row(ui, t, crate::i18n::tr("Location"), |ui| {
        ui.label(RichText::new(location).color(t.text));
    });
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
    heading(ui, t, crate::i18n::tr("Startup"));
    row(ui, t, crate::i18n::tr("Open in"), |ui| {
        choices(
            ui,
            "settingsStartup",
            &[(StartupView::Last, "Last view"), (StartupView::Grid, "Photo Grid"), (StartupView::Detail, "Detail")],
            &mut app.ui.settings.startup_view,
        );
    });
    heading(ui, t, crate::i18n::tr("Culling"));
    check(ui, "settings.autoAdvance", &mut app.ui.auto_advance, "Auto Advance: move to the next photo after rating or flagging");
    check(ui, "settings.confirmDelete", &mut app.ui.settings.confirm_delete, "Confirm before moving photos to Recently Deleted");
    heading(ui, t, crate::i18n::tr("External Editor"));
    row(ui, t, crate::i18n::tr("Application"), |ui| {
        let r = ui
            .add(egui::TextEdit::singleline(&mut app.ui.settings.external_editor).hint_text(crate::i18n::tr("System default")).desired_width(220.0));
        register(ui.ctx(), "field:externalEditor", r.rect);
    });
    hint(
        ui,
        t,
        crate::i18n::tr(
            "Photo ▸ Edit in External Editor (⇧⌘E) renders a 16-bit TIFF copy, stacks it with the original and opens it here (an app name on macOS, a program path elsewhere).",
        ),
    );
}

// -------------------------------------------------------------------------------------- Import

/// A preset picker: `None` = `none_label`. Returns the new choice when it changed.
fn preset_combo(app: &LightcraftApp, ui: &mut egui::Ui, id: &str, current: Option<&str>, none_label: &str) -> Option<Option<String>> {
    let name = |pid: &str| app.session.presets.iter().find(|p| p.id == pid).map(|p| p.name.clone()).unwrap_or_else(|| format!("{pid} (missing)"));
    let text = current.map(name).unwrap_or_else(|| none_label.to_string());
    let mut out = None;
    let r = egui::ComboBox::from_id_salt(id).width(240.0).selected_text(text).show_ui(ui, |ui| {
        if ui.selectable_label(current.is_none(), none_label).clicked() {
            out = Some(None);
        }
        let mut group = "";
        for p in &app.session.presets {
            if p.group != group {
                group = &p.group;
                ui.label(RichText::new(group).size(10.5).weak());
            }
            if ui.selectable_label(current == Some(p.id.as_str()), &p.name).clicked() {
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
    heading(ui, t, crate::i18n::tr("Raw defaults"));
    hint(ui, t, crate::i18n::tr("Settings new raw photos start from. Changing them doesn't touch photos already in the library."));
    row(ui, t, crate::i18n::tr("Raw photos"), |ui| {
        if let Some(v) = preset_combo(app, ui, "settingsRawPreset", d.raw_preset.as_deref(), "LightCraft Default") {
            let _ = app.run("library.preferences", json!({"import": {"rawPreset": v}}));
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
                        Some(RAW_DEFAULT) => "Same as raw default".to_string(),
                        None => "LightCraft Default".to_string(),
                        Some(pid) => app.session.presets.iter().find(|p| p.id == pid).map(|p| p.name.clone()).unwrap_or_else(|| pid.to_string()),
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
                            if ui.selectable_label(current == Some(p.id.as_str()), &p.name).clicked() {
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
    heading(ui, t, crate::i18n::tr("Other images (JPEG, PNG, TIFF, HEIC…)"));
    row(ui, t, crate::i18n::tr("Non-raw photos"), |ui| {
        if let Some(v) = preset_combo(app, ui, "settingsOtherPreset", d.other_preset.as_deref(), "None") {
            let _ = app.run("library.preferences", json!({"import": {"otherPreset": v}}));
        }
    });
    heading(ui, t, crate::i18n::tr("Metadata"));
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
                .selected_text(cur.clone().unwrap_or_else(|| "None".into()))
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
    heading(ui, t, crate::i18n::tr("XMP sidecars"));
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
    if !cfg!(target_arch = "wasm32") {
        heading(ui, t, crate::i18n::tr("Auto Import"));
        hint(ui, t, crate::i18n::tr("Photos that arrive in this folder (tethering, a scanner, a sync app) are added as soon as they're complete."));
        row(ui, t, crate::i18n::tr("Watched folder"), |ui| {
            ui.label(RichText::new(d.auto_folder.clone().unwrap_or_else(|| "Off".into())).color(t.text));
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
    if app.session.library.is_none() {
        hint(ui, t, crate::i18n::tr("In-memory session: these settings last until LightCraft quits."));
    }
}

// --------------------------------------------------------------------------------- Performance

fn performance_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    use lightcraft_engine::gpu;
    heading(ui, t, crate::i18n::tr("Rendering"));
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
    heading(ui, t, crate::i18n::tr("Thumbnail cache"));
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
    heading(ui, t, crate::i18n::tr("Local folders"));
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
    heading(ui, t, crate::i18n::tr("Smart previews"));
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
                app.toast(ui.ctx(), if failed > 0 { crate::i18n::tr_format!("{msg} ({failed} failed)", failed = failed, msg = msg) } else { msg });
            }
            Err(e) => app.toast(ui.ctx(), e),
        }
    }
    hint(ui, t, crate::i18n::tr("Keep it on a drive with room: smart previews are about 1 MB per photo. The thumbnail cache stays in the library."));
}

// ---------------------------------------------------------------------------------- Interface

fn interface_tab(app: &mut LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    heading(ui, t, crate::i18n::tr("Filmstrip"));
    check(ui, "settings.filmNames", &mut app.ui.settings.film_names, "Show file names");
    check(ui, "settings.filmBadges", &mut app.ui.settings.film_badges, "Show ratings, flags and edit badges");
    heading(ui, t, crate::i18n::tr("Grid"));
    row(ui, t, crate::i18n::tr("Ratings & flags"), |ui| {
        choices(
            ui,
            "settingsGridBadges",
            &[(GridBadges::Auto, "When rated or hovered"), (GridBadges::Always, "Always"), (GridBadges::Never, "Never")],
            &mut app.ui.settings.grid_badges,
        );
    });
    check(ui, "settings.showFilenames", &mut app.ui.show_filenames, "Square Grid: show file names and formats");
    heading(ui, t, crate::i18n::tr("Detail"));
    check(ui, "settings.navigator", &mut app.ui.navigator, "Show the Navigator while zoomed in");
    row(ui, t, crate::i18n::tr("Info overlay"), |ui| {
        use crate::state::InfoOverlay as I;
        choices(ui, "settingsInfo", &[(I::Off, "Off"), (I::Basic, "File & date"), (I::Exposure, "Exposure")], &mut app.ui.info_overlay);
    });
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
        app.ui.toast = Some((e, app.last_time + 6.0));
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
    app.ui.toast = Some((text, app.last_time + 5.0));
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
    // a new binding: pick an action, then rec. and press the keys
    let actions = draft_actions(app);
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
        if crate::widgets::text_button(ui, "keysRecord", if rec { "press keys…" } else { "rec." }, rec).clicked() {
            app.keymap.recording = if rec { None } else { Some(None) };
        }
    });
    ui.add_space(8.0);
    let search_id = egui::Id::new("keysSearch");
    let mut q: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    row(ui, t, "Search", |ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut q).hint_text("keys, action or command").desired_width(260.0));
        register(ui.ctx(), "field:keysSearch", r.rect);
    });
    ui.data_mut(|d| d.insert_temp(search_id, q.clone()));
    let ql = q.to_lowercase();
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
        for (n, r) in rows.iter().enumerate() {
            if r.group != group {
                group = r.group.clone();
                ui.add_space(6.0);
                ui.label(RichText::new(&group).font(t.semibold(11.5)).color(t.text_label));
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
                        if crate::widgets::text_button(ui, &format!("keysRec-{n}"), if rec { "press…" } else { "rec." }, rec).clicked() {
                            app.keymap.recording = if rec { None } else { Some(Some(i)) };
                        }
                        if crate::widgets::text_button(ui, &format!("keysDel-{n}"), "del.", false).clicked() {
                            let mut g = app.keymap.file.clone();
                            if i < g.bindings.len() {
                                g.bindings.remove(i);
                            }
                            save_keymap(app, g);
                        }
                    }
                } else if !r.overridden {
                    // rebind a built-in / profile action to other keys, or switch its keys off
                    if crate::widgets::text_button(ui, &format!("keysRec-{n}"), "rec.", false).clicked() {
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
            ui.label(RichText::new("Lightroom Classic keys with no LightCraft command").font(t.semibold(11.5)).color(t.text_label));
            for (k, what) in crate::keymap::CLASSIC_SKIPPED {
                hint(ui, t, &format!("{k} — {what}"));
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
                if crate::widgets::text_button(ui, &format!("midiDel-{i}"), "del.", false).clicked() {
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
