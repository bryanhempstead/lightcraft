//! UI-level commands (views, panels, zoom, tools, dialogs) and the menu model shared by the native
//! menu bar, the shortcut handler and the control channel.

use serde::Serialize;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{BeforeAfter, Dialog, Module, RightPanel, ViewMode, Zoom};

/// (id, label, shortcut, menu path)
pub type UiCommand = (&'static str, &'static str, Option<&'static str>, &'static str);

/// The "Edit → Language" entries, one per language in [`crate::i18n::Locale::ALL`], so a language
/// added to the table shows up in the menu (and in the control channel's command list) by itself.
pub const LANGUAGE_COMMANDS: &[UiCommand] = &[
    ("app.language.english", crate::i18n::Locale::En.name(), None, "Edit>Language"),
    ("app.language.simplifiedChinese", crate::i18n::Locale::ZhHans.name(), None, "Edit>Language"),
    ("app.language.traditionalChinese", crate::i18n::Locale::ZhHant.name(), None, "Edit>Language"),
    ("app.language.japanese", crate::i18n::Locale::Ja.name(), None, "Edit>Language"),
    ("app.language.portuguese", crate::i18n::Locale::PtBr.name(), None, "Edit>Language"),
    ("app.language.german", crate::i18n::Locale::De.name(), None, "Edit>Language"),
    ("app.language.russian", crate::i18n::Locale::Ru.name(), None, "Edit>Language"),
];

/// Every UI command: the languages, then everything else. `xtask parity` reads both tables from
/// this file, so an id listed in `docs/parity.md` is checked wherever it is declared.
pub fn ui_commands() -> impl Iterator<Item = &'static UiCommand> {
    LANGUAGE_COMMANDS.iter().chain(UI_COMMANDS)
}

/// The language a Language-menu command selects, if the id is one. The engine and the UI both go
/// through here, so the menu, the settings row and the control channel agree on the mapping.
pub fn language_from_command(id: &str) -> Option<crate::i18n::Locale> {
    match id {
        "app.language.english" => Some(crate::i18n::Locale::En),
        "app.language.simplifiedChinese" => Some(crate::i18n::Locale::ZhHans),
        "app.language.traditionalChinese" => Some(crate::i18n::Locale::ZhHant),
        "app.language.japanese" => Some(crate::i18n::Locale::Ja),
        "app.language.portuguese" => Some(crate::i18n::Locale::PtBr),
        "app.language.german" => Some(crate::i18n::Locale::De),
        "app.language.russian" => Some(crate::i18n::Locale::Ru),
        _ => None,
    }
}

pub const UI_COMMANDS: &[UiCommand] = &[
    ("view.photoGrid", "Photo Grid", None, "View"),
    ("view.squareGrid", "Square Grid", None, "View"),
    // G: Photo Grid ↔ Square Grid (from other views: the photo grid)
    ("view.gridToggle", "Grid", Some("G"), ""),
    ("tool.guidedUpright", "Guided Upright", Some("Shift+G"), "Window>Tools"),
    ("view.detail", "Detail", Some("D"), "View"),
    ("view.compare", "Compare", Some("Shift+C"), "View"),
    ("view.survey", "Survey", Some("N"), "View"),
    ("view.people", "People", None, "View"),
    ("view.faceBoxes", "Face Boxes", None, "View"),
    ("view.reference", "Reference View", Some("Shift+R"), "View"),
    ("photo.setReference", "Set as Reference Photo", None, ""),
    ("compare.swap", "Swap Compare Photos", None, "View"),
    ("compare.makeSelect", "Make Candidate the Select", None, "View"),
    ("view.autoAdvance", "Auto Advance", None, "Photo"),
    ("view.filmstrip", "Filmstrip", Some("/"), "Window>Panels"),
    ("view.leftPanel", "Left Panel", Some("Cmd+Shift+L"), "Window>Panels"),
    ("view.rightPanel", "Right Panel", Some("F8"), "Window>Panels"),
    ("view.topPanel", "Module Picker", Some("F5"), "Window>Panels"),
    ("view.sidePanels", "Side Panels", Some("Tab"), "Window>Panels"),
    ("view.allPanels", "All Panels", Some("Shift+Tab"), "Window>Panels"),
    ("view.toolbar", "Toolbar", Some("T"), "View"),
    ("view.lightsOut", "Lights Out", None, "Window"),
    ("view.filmHeight", "Filmstrip Height", None, ""),
    ("view.solo", "Solo Mode", None, ""),
    ("view.beforeAfter", "Compare Before and After", Some("Y"), "View"),
    ("view.beforeAfterSplit", "Before/After Split", Some("Shift+Y"), "View"),
    ("view.beforeAfterTopBottom", "Before/After Top/Bottom", Some("Alt+Y"), "View"),
    ("view.beforeAfterSplitTopBottom", "Before/After Split Top/Bottom", Some("Alt+Shift+Y"), "View"),
    ("view.showOriginal", "Show Original", Some("\\"), "View"),
    ("view.zoomFit", "Zoom to Fit", Some("Cmd+0"), "View"),
    ("view.zoom100", "Zoom 100%", Some("Cmd+Alt+0"), "View"),
    ("view.zoomToggle", "Toggle Zoom", Some("Z"), "View"),
    // the ratio a click (and Z / Space) zooms to
    ("view.clickZoom", "Click Zoom Ratio", None, ""),
    ("view.navigate", "Set Image Zoom and Pan", None, ""),
    ("view.zoomIn", "Zoom In", Some("Cmd+="), "View"),
    ("view.zoomOut", "Zoom Out", Some("Cmd+-"), "View"),
    ("view.clipping", "Show Clipping", Some("J"), "View"),
    // in grids S expands/collapses stacks (the engine command it shadows)
    ("view.softProof", "Soft Proofing", Some("S"), "View"),
    ("view.histogram", "Histogram", Some("Cmd+Shift+H"), "View"),
    ("view.maskOverlay", "Show Mask Overlay", Some("O"), "View"),
    // Shift+O in the Masking panel (elsewhere it cycles the crop overlay)
    ("view.maskOverlayMode", "Cycle Mask Overlay Mode", None, "View"),
    ("view.maskOverlayColor", "Cycle Mask Overlay Color", None, "View"),
    ("view.maskPins", "Show Mask Pins", None, "View"),
    ("view.visualizeSpots", "Visualize Spots", Some("A"), "View"),
    ("view.cropOverlay", "Cycle Crop Overlay", Some("Shift+O"), "View"),
    ("view.cropOverlayOrientation", "Cycle Crop Overlay Orientation", None, "View"),
    ("view.back", "Back to Grid", Some("Escape"), ""),
    ("tool.done", "Done", Some("Enter"), ""),
    ("view.filterBar", "Filter Bar", Some("Shift+F"), "View"),
    ("local.addRoot", "Add Folder to Local", None, ""),
    ("local.hide", "Remove from Local", None, ""),
    ("local.restoreHidden", "Show Hidden Local Locations", None, ""),
    ("view.fullScreenPreview", "Full Screen Preview", Some("F"), "View"),
    ("view.enterFullScreen", "Enter Full Screen", Some("Cmd+Shift+F"), "View"),
    ("view.infoOverlay", "Cycle Info Overlay", Some("Cmd+I"), "View"),
    ("view.navigator", "Navigator", None, "View"),
    ("panel.edit", "Edit", Some("E"), "Window"),
    ("panel.profiles", "Profile Browser", None, "Window"),
    ("panel.crop", "Crop & Rotate", Some("C"), "Window"),
    ("panel.remove", "Remove", Some("H"), "Window"),
    ("panel.masking", "Masking", Some("M"), "Window"),
    ("panel.redeye", "Red Eye", None, "Window"),
    ("panel.presets", "Presets", Some("Shift+P"), "Window"),
    ("panel.info", "Info", Some("I"), "Window"),
    ("panel.keywords", "Keywords", Some("K"), "Window"),
    ("panel.versions", "Versions", Some("Shift+V"), "Window"),
    ("panel.activity", "History", None, "Window"),
    ("panel.close", "Close Panel", None, ""),
    // Develop's panels, as Lightroom Classic numbers them (⌘1…⌘9 open one, ⌘-click solo)
    ("section.basic", "Basic", Some("Cmd+1"), "Window>Develop Panels"),
    ("section.toneCurve", "Tone Curve", Some("Cmd+2"), "Window>Develop Panels"),
    ("section.hsl", "HSL / Color", Some("Cmd+3"), "Window>Develop Panels"),
    ("section.colorGrading", "Color Grading", Some("Cmd+4"), "Window>Develop Panels"),
    ("section.detail", "Detail", Some("Cmd+5"), "Window>Develop Panels"),
    ("section.lensCorrections", "Lens Corrections", Some("Cmd+6"), "Window>Develop Panels"),
    ("section.transform", "Transform", Some("Cmd+7"), "Window>Develop Panels"),
    ("section.effects", "Effects", Some("Cmd+8"), "Window>Develop Panels"),
    ("section.calibration", "Calibration", Some("Cmd+9"), "Window>Develop Panels"),
    // the earlier (Lightroom desktop) section names open the Classic panel holding their sliders
    ("section.light", "Light", None, ""),
    ("section.color", "Color", None, ""),
    ("section.optics", "Optics", None, ""),
    ("tool.brush", "Brush", Some("B"), "Window>Tools"),
    ("tool.linear", "Linear Gradient", Some("L"), "Window>Tools"),
    ("tool.radial", "Radial Gradient", Some("R"), "Window>Tools"),
    ("tool.wbPicker", "White Balance Selector", Some("W"), "Window>Tools"),
    ("tool.none", "No Tool", None, ""),
    // brush size / feather of the active brush (Masking brush, Remove tool and its selected spot)
    ("brush.smaller", "Decrease Brush Size", Some("["), "Window>Tools"),
    ("brush.larger", "Increase Brush Size", Some("]"), "Window>Tools"),
    ("brush.featherLess", "Decrease Brush Feather", Some("Shift+["), "Window>Tools"),
    ("brush.featherMore", "Increase Brush Feather", Some("Shift+]"), "Window>Tools"),
    ("dialog.newAlbum", "New Album…", Some("Cmd+N"), "File"),
    ("dialog.newFolder", "New Folder…", Some("Cmd+Shift+N"), "File"),
    ("dialog.smartAlbum", "New Smart Album…", None, "File"),
    ("view.photoCounts", "Show Photo Counts", None, "View"),
    ("view.slideshow", "Slideshow", Some("Cmd+Alt+Enter"), "View"),
    ("view.secondWindow", "Second Window", Some("Cmd+F11"), "Window"),
    ("tool.keywordPainter", "Keyword Painter", None, ""),
    ("view.gridInfo", "Grid Info", None, ""),
    ("dialog.allMetadata", "All Metadata…", None, "Photo"),
    ("dialog.newSmartAlbum", "New Smart Album from Filter…", Some("Cmd+Alt+N"), "File"),
    ("dialog.createPreset", "Create Preset…", Some("Cmd+Shift+P"), "Photo"),
    ("dialog.autoStack", "Auto-Stack by Capture Time…", None, "Photo>Stack"),
    ("dialog.copySettings", "Choose Edit Settings to Copy…", Some("Cmd+Shift+C"), "Edit"),
    ("dialog.pasteSettings", "Paste Selected Settings…", Some("Cmd+Shift+V"), "Edit"),
    ("dialog.syncSettings", "Sync Settings…", None, "Photo>Develop Settings"),
    ("library.syncMetadata", "Sync Metadata", None, "Library"),
    ("view.previousImport", "Previous Import", None, "Library"),
    ("view.focusSearch", "Find…", Some("Cmd+F"), "Edit"),
    ("dialog.export", "Export…", None, "File"),
    ("photo.editInExternal", "Edit in External Editor", Some("Cmd+Shift+E"), "Photo"),
    ("dialog.mergeHdr", "HDR…", Some("Ctrl+H"), "Photo>Photo Merge"),
    ("dialog.mergePanorama", "Panorama…", Some("Ctrl+M"), "Photo>Photo Merge"),
    ("dialog.mergeHdrPanorama", "HDR Panorama…", None, "Photo>Photo Merge"),
    ("merge.hdrLast", "HDR with Last Settings", Some("Ctrl+Shift+H"), "Photo>Photo Merge"),
    ("merge.panoramaLast", "Panorama with Last Settings", Some("Ctrl+Shift+M"), "Photo>Photo Merge"),
    ("merge.hdrPanoramaLast", "HDR Panorama with Last Settings", None, "Photo>Photo Merge"),
    ("file.addPhotos", "Import Photos and Video…", Some("Cmd+Shift+I"), "File"),
    // the Import window's source column: browse to a folder / pick a device (`{path, subfolders?}`)
    ("import.source", "Import Source", None, ""),
    ("file.addFolder", "Import from Folder…", None, "File"),
    ("file.addFromDevice", "Import from Device", None, ""),
    ("file.findMissing", "Find Missing Photos…", None, "File"),
    ("file.backupLibrary", "Back Up Library…", None, "File"),
    ("file.restoreLibrary", "Restore Library from Backup…", None, "File"),
    ("photo.locate", "Locate Missing File…", None, ""),
    ("dialog.saveMetadataPreset", "Save Metadata Preset…", None, ""),
    ("app.quit", "Quit LightCraft", Some("Cmd+Q"), "File"),
    ("file.importPresets", "Import Profiles & Presets…", None, "File"),
    ("file.exportPresets", "Export Presets…", None, "File"),
    // Edit panel ▸ Curve ▸ Point Curve dropdown
    ("file.importCurvePresets", "Import Point Curve Presets…", None, ""),
    ("file.exportCurvePresets", "Export Point Curve Presets…", None, ""),
    ("app.settings", "Settings…", Some("Cmd+,"), "Edit"),
    ("app.openLibrary", "Open Library…", None, "File"),
    ("app.about", "About LightCraft", None, "Help"),
    ("app.systemInfo", "System Info…", None, "Help"),
    ("app.whatsNew", "What's New", None, "Help"),
    ("dialog.cull", "Assisted Culling…", None, "Photo"),
    ("app.help", "LightCraft Help", Some("F1"), "Help"),
    ("app.discord", "Join the ArtCraft Discord…", None, "Help"),
    ("app.feedback", "Send Feedback…", None, "Help"),
    ("app.website", "LightCraft Website", None, "Help"),
    ("app.github", "LightCraft on GitHub", None, "Help"),
    ("app.artcraft", "ArtCraft Website", None, "Help"),
    ("app.shortcuts", "Keyboard Shortcuts", Some("Cmd+/"), "Help"),
    ("app.export", "Export Now", None, ""),
    ("app.showInFinder", "Show in Finder", Some("Cmd+R"), "Photo"),
    ("dialog.rename", "Rename Photos…", Some("F2"), "Photo"),
    ("dialog.labelNames", "Edit Color Label Names…", None, ""),
    ("dialog.captureTime", "Edit Capture Time…", None, "Photo"),
    ("photo.tagFromTracklog", "Auto-Tag from Tracklog…", None, "Photo"),
    ("app.exportPrevious", "Export with Previous", Some("Cmd+Alt+Shift+E"), "File"),
    // Lightroom Classic modules and "super keys" (Settings ▸ shrt.; see keymap.rs)
    ("view.library", "Library", Some("Cmd+Alt+1"), "Window"),
    ("view.develop", "Develop", Some("Cmd+Alt+2"), "Window"),
    ("view.colorMixer", "Color Mixer", None, ""),
    ("view.resumeLastLeftOff", "Go to Where I Left Off", None, "View"),
    ("preset.applyByName", "Apply Preset by Name", None, ""),
    ("crop.nudge", "Nudge Crop", None, ""),
    ("keys.nudge", "Nudge Slider", None, ""),
    ("keys.macro", "Run Macro", None, ""),
    ("keys.send", "Send Keys", None, ""),
    ("keys.list", "List Key Bindings", None, ""),
    ("keys.set", "Set Key Bindings", None, ""),
    ("keys.add", "Add Key Binding", None, ""),
    ("keys.record", "Record Keys", None, ""),
    ("keys.import", "Import Shortcuts", None, ""),
    ("keys.midi", "MIDI Message", None, ""),
    ("keys.learn", "MIDI Learn", None, ""),
];

/// Lightroom Classic's Develop module: the active photo in the loupe with the develop panels
/// (the reference view stays). The Library view it came from is remembered for G / E.
pub fn enter_develop(app: &mut LightcraftApp) {
    if app.ui.module == Module::Library {
        app.ui.library_view = app.ui.view;
    }
    app.ui.module = Module::Develop;
    if app.ui.view != ViewMode::Reference {
        app.ui.view = ViewMode::Detail;
    }
    if !app.ui.right.is_edit_tool() {
        app.ui.right = RightPanel::Edit;
    }
    app.ui.right_panel = true;
    // the selection carries over: with none, the first photo in view is the one developed
    if app.session.active().is_none()
        && let Some(first) = app.session.visible_cloned().first().copied()
    {
        let _ = app.session.execute("library.select", &json!({"ids": [first.0]}));
    }
}

/// Back to the Library module, in `view` (or the Library view last used there).
pub fn enter_library(app: &mut LightcraftApp, view: Option<ViewMode>) {
    let v = view.unwrap_or(app.ui.library_view);
    app.ui.module = Module::Library;
    app.ui.view = if v == ViewMode::Reference { ViewMode::PhotoGrid } else { v };
    app.ui.library_view = app.ui.view;
    // Develop's on-canvas tools end with it
    if app.ui.tool != "wbPicker" {
        app.ui.tool.clear();
    }
    let _ = app.session.end_interaction();
}

/// Open a side-panel section (`list`: which side), revealing that side.
fn open_section(app: &mut LightcraftApp, develop_left: bool, id: &str) {
    let list = if develop_left { &mut app.ui.develop_left_sections } else { &mut app.ui.library_sections };
    if !list.iter().any(|s| s == id) {
        list.push(id.to_string());
    }
    if develop_left {
        app.ui.left_panel = true;
    } else {
        app.ui.right_panel = true;
    }
}

fn panel(app: &mut LightcraftApp, ctx: &egui::Context, p: RightPanel, name: &str) {
    if app.ui.right == p && app.ui.module == Module::Develop && p.is_edit_tool() {
        // a Develop tool closes back to the panels (Classic's tool strip toggles)
        app.ui.right = RightPanel::Edit;
        app.toast(ctx, crate::i18n::tr_format!("{name} Off", name = crate::i18n::tr(name)));
    } else if app.ui.right == p && !p.is_edit_tool() {
        app.ui.right = if app.ui.module == Module::Develop { RightPanel::Edit } else { RightPanel::None };
        app.toast(ctx, crate::i18n::tr_format!("{name} Off", name = crate::i18n::tr(name)));
    } else {
        app.ui.right = p;
        app.toast(ctx, crate::i18n::tr_format!("{name} On", name = crate::i18n::tr(name)));
        match p {
            _ if p.is_edit_tool() => {
                let tool = app.ui.right;
                enter_develop(app);
                app.ui.right = tool;
            }
            // Library's right panel: Keywording, Metadata
            RightPanel::Info | RightPanel::Keywords => {
                if app.ui.module == Module::Develop {
                    enter_library(app, Some(ViewMode::Detail));
                }
                open_section(app, false, if p == RightPanel::Info { "metadata" } else { "keywording" });
            }
            // Develop's left panel: Snapshots, History
            RightPanel::Versions | RightPanel::Activity => {
                enter_develop(app);
                app.ui.right = p;
                open_section(app, true, if p == RightPanel::Versions { "snapshots" } else { "history" });
            }
            _ => {}
        }
    }
    if p != RightPanel::Masking && app.ui.tool != "wbPicker" {
        app.ui.tool.clear();
    }
    let _ = app.session.end_interaction();
}

/// `[` / `]` (size ×`k`) and ⇧`[` / ⇧`]` (feather +`df`) for the brush in use: the Remove tool's
/// (and its selected spot's) or the Masking brush's.
fn adjust_brush(app: &mut LightcraftApp, k: f32, df: f32) -> Value {
    if app.ui.right == RightPanel::Remove {
        app.ui.remove_size = (app.ui.remove_size * k).clamp(0.001, 0.25);
        app.ui.remove_feather = (app.ui.remove_feather + df).clamp(0.0, 100.0);
        if app.session.active_spot.is_some() {
            let mut p = json!({});
            if k != 1.0 {
                p["size"] = json!(app.ui.remove_size);
            }
            if df != 0.0 {
                p["feather"] = json!(app.ui.remove_feather);
            }
            let _ = app.run("spot.update", p);
        }
        json!({"size": app.ui.remove_size, "feather": app.ui.remove_feather})
    } else {
        app.ui.brush_size = (app.ui.brush_size * k).clamp(0.002, 0.5);
        app.ui.brush_feather = (app.ui.brush_feather + df).clamp(0.0, 100.0);
        json!({"size": app.ui.brush_size, "feather": app.ui.brush_feather})
    }
}

/// An sRGB colour from `"#rrggbb"` or `[r, g, b]` (0..255).
pub fn parse_rgb(v: &Value) -> Option<[u8; 3]> {
    if let Some(s) = v.as_str() {
        let h = s.strip_prefix('#').unwrap_or(s);
        if h.len() != 6 {
            return None;
        }
        let c = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
        return Some([c(0)?, c(2)?, c(4)?]);
    }
    let a = v.as_array()?;
    let c = |i: usize| a.get(i)?.as_f64().map(|x| x.clamp(0.0, 255.0).round() as u8);
    Some([c(0)?, c(1)?, c(2)?])
}

/// Handle UI commands; `None` means "not a UI command — send it to the engine".
pub fn run_ui_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    if let Some(language) = language_from_command(id) {
        app.ui.language = language;
        // Immediately, not on the next frame: the reply and anything else run this frame
        // (menus rebuilt from it, toasts) are already in the new language.
        crate::i18n::set_language(app.ui.language);
        return Some(Ok(json!(app.ui.language)));
    }
    if let Some(r) = crate::keymap::run_command(app, id, p) {
        return Some(r);
    }
    let ctx = egui::Context::default();
    let r: Result<Value, String> = match id {
        "view.develop" => {
            // Lightroom Classic's Develop module (D): the loupe with the develop panels, never a toggle
            enter_develop(app);
            Ok(json!({"module": app.ui.module}))
        }
        "view.library" => {
            // {view?: photoGrid|squareGrid|detail|compare|survey|people}: the Library module
            let view = match p.get("view") {
                Some(v) => match serde_json::from_value::<ViewMode>(v.clone()) {
                    Ok(v) => Some(v),
                    Err(_) => return Some(Err(format!("view.library: unknown view {v}"))),
                },
                None => None,
            };
            enter_library(app, view);
            Ok(json!({"module": app.ui.module, "view": app.ui.view}))
        }
        "view.rightPanel" => {
            app.ui.right_panel = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.right_panel);
            Ok(json!({"show": app.ui.right_panel}))
        }
        "view.topPanel" => {
            app.ui.top_panel = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.top_panel);
            Ok(json!({"show": app.ui.top_panel}))
        }
        "view.toolbar" => {
            app.ui.toolbar = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.toolbar);
            Ok(json!({"show": app.ui.toolbar}))
        }
        "view.sidePanels" => {
            // Tab: both side panels away, or back
            let show = p.get("show").and_then(Value::as_bool).unwrap_or(!(app.ui.left_panel || app.ui.right_panel));
            app.ui.left_panel = show;
            app.ui.right_panel = show;
            Ok(json!({"show": show}))
        }
        "view.allPanels" => {
            // ⇧Tab: side panels, top bar and filmstrip away, or all back
            let any = app.ui.left_panel || app.ui.right_panel || app.ui.filmstrip || app.ui.top_panel;
            let show = p.get("show").and_then(Value::as_bool).unwrap_or(!any);
            app.ui.left_panel = show;
            app.ui.right_panel = show;
            app.ui.filmstrip = show;
            app.ui.top_panel = show;
            Ok(json!({"show": show}))
        }
        "view.lightsOut" => {
            // {mode?: on|dim|off}; cycles when omitted
            app.ui.lights_out = match p.get("mode") {
                Some(m) => match serde_json::from_value(m.clone()) {
                    Ok(v) => v,
                    Err(_) => return Some(Err(format!("view.lightsOut: unknown mode {m} (on|dim|off)"))),
                },
                None => app.ui.lights_out.next(),
            };
            Ok(json!({"mode": app.ui.lights_out}))
        }
        "view.filmHeight" => {
            let h = p.get("height").and_then(Value::as_f64).unwrap_or(crate::state::FILM_HEIGHT.default as f64) as f32;
            app.ui.film_height = crate::state::FILM_HEIGHT.clamp(h);
            app.ui.filmstrip = true;
            Ok(json!({"height": app.ui.film_height}))
        }
        "view.solo" => {
            // {side?: left|right, on?}: Solo Mode — opening a panel closes the others on that side
            let left = p.get("side").and_then(Value::as_str) == Some("left");
            let cur = if left { app.ui.solo_left } else { app.ui.single_panel };
            let on = p.get("on").and_then(Value::as_bool).unwrap_or(!cur);
            if left {
                app.ui.solo_left = on;
            } else {
                app.ui.single_panel = on;
            }
            Ok(json!({"on": on}))
        }
        "import.source" => crate::import::choose_source(app, p),
        "library.syncMetadata" => crate::panels::library_right::sync_metadata(app),
        "view.previousImport" => {
            // Library ▸ Previous Import: the photos of the last import, in the grid
            let r = app.session.execute("library.source", &json!({"kind": "previousImport"})).map_err(|e| e.to_string());
            if r.is_ok() {
                enter_library(app, Some(ViewMode::PhotoGrid));
            }
            r
        }
        "view.colorMixer" => {
            // {mode: hue|saturation|luminance|all|bw}: Develop's HSL / Color panel (LrSuperKeys H/J/K/B)
            enter_develop(app);
            app.ui.right = RightPanel::Edit;
            if !app.ui.section_open("hsl") {
                app.ui.toggle_section("hsl");
            }
            match p.get("mode").and_then(Value::as_str) {
                Some(m @ ("hue" | "saturation" | "luminance" | "all")) => app.ui.mixer_mode = m.to_string(),
                Some("bw") | None => {}
                Some(other) => return Some(Err(format!("view.colorMixer: unknown mode `{other}`"))),
            }
            Ok(json!({"mode": app.ui.mixer_mode}))
        }
        "view.resumeLastLeftOff" => crate::leftoff::resume(app),
        "preset.applyByName" => (|| {
            // {name, amount?}: the preset called `name` (case-insensitive), as the Monogram / LrKeys
            // presets name them
            let name = p.get("name").and_then(Value::as_str).ok_or("preset.applyByName: missing `name`")?.trim().to_string();
            let found = app.session.presets.iter().find(|x| x.name.trim().eq_ignore_ascii_case(&name)).map(|x| x.id.clone());
            let id = found.ok_or_else(|| format!("No preset named \"{name}\""))?;
            let mut q = json!({"id": id});
            if let Some(a) = p.get("amount") {
                q["amount"] = a.clone();
            }
            let r = app.run("preset.apply", q);
            if r.is_ok() {
                app.ui.toast = Some((name.clone(), app.last_time + 1.4, None));
            }
            r
        })(),
        "crop.nudge" => (|| {
            // {x?, y?, scale?} in percent of the image: move the crop / zoom it in (+) or out (-)
            let id = app.session.active().ok_or("no photo selected")?;
            let d = app.session.develop_of(id).unwrap_or_default();
            let r = d.crop.geometry.rect;
            let f = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0) / 100.0;
            let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
            let k = (1.0 - f("scale")).clamp(0.05, 20.0);
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (hw, hh) = (((x1 - x0) * k / 2.0).min(0.5), ((y1 - y0) * k / 2.0).min(0.5));
            (x0, x1, y0, y1) = (cx - hw + f("x"), cx + hw + f("x"), cy - hh + f("y"), cy + hh + f("y"));
            // keep it on the photo: slide back inside
            let (sx, sy) = ((-x0).max(0.0) - (x1 - 1.0).max(0.0), (-y0).max(0.0) - (y1 - 1.0).max(0.0));
            app.run("crop.set", json!({"rect": [x0 + sx, y0 + sy, x1 + sx, y1 + sy]}))
        })(),
        "view.photoGrid" => {
            app.ui.view = ViewMode::PhotoGrid;
            Ok(Value::Null)
        }
        "view.squareGrid" => {
            app.ui.view = ViewMode::SquareGrid;
            Ok(Value::Null)
        }
        "view.gridInfo" => {
            // {info?: filename | exposure | date} (cycles when omitted)
            let next = match p.get("info").and_then(Value::as_str) {
                Some(i @ ("filename" | "exposure" | "date")) => i.to_string(),
                Some(other) => return Some(Err(format!("view.gridInfo: unknown `{other}` (filename|exposure|date)"))),
                None => match app.ui.grid_info.as_str() {
                    "filename" => "exposure".into(),
                    "exposure" => "date".into(),
                    _ => "filename".into(),
                },
            };
            app.ui.grid_info = next;
            app.ui.show_filenames = true;
            Ok(json!({"info": app.ui.grid_info}))
        }
        "tool.keywordPainter" => {
            // {keyword?}: paint that keyword onto photos in the grid by clicking them; no keyword stops
            app.ui.keyword_painter = p.get("keyword").and_then(Value::as_str).map(str::trim).filter(|k| !k.is_empty()).map(str::to_string);
            if app.ui.keyword_painter.is_some() && !matches!(app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid) {
                app.ui.view = ViewMode::PhotoGrid;
            }
            if let Some(k) = app.ui.keyword_painter.clone() {
                app.toast(&ctx, crate::i18n::tr_format!("Painting “{k}”: click photos to add or remove it · Esc stops", k = k));
            }
            Ok(json!({"keyword": app.ui.keyword_painter}))
        }
        "view.secondWindow" => {
            app.ui.second_window = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.second_window);
            Ok(json!({"show": app.ui.second_window}))
        }
        "view.photoCounts" => {
            app.ui.show_counts = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.show_counts);
            Ok(json!({"show": app.ui.show_counts}))
        }
        "view.gridToggle" => {
            app.ui.view = if app.ui.view == ViewMode::PhotoGrid { ViewMode::SquareGrid } else { ViewMode::PhotoGrid };
            Ok(Value::Null)
        }
        "view.detail" => {
            // the Library loupe (Lightroom Classic's E); from Develop it goes back to Library
            if app.ui.module == Module::Develop {
                enter_library(app, Some(ViewMode::Detail));
            }
            app.ui.view = ViewMode::Detail;
            app.ui.library_view = ViewMode::Detail;
            Ok(Value::Null)
        }
        "view.compare" => crate::panels::compare::enter_compare(app),
        "view.faceBoxes" => {
            app.ui.face_boxes = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.face_boxes);
            Ok(json!({"show": app.ui.face_boxes}))
        }
        "view.people" => {
            app.ui.view = ViewMode::People;
            Ok(json!({"people": app.session.catalog.people().len()}))
        }
        "view.survey" => {
            app.ui.view = ViewMode::Survey;
            Ok(json!({"photos": crate::panels::compare::survey_photos(app).len()}))
        }
        "view.reference" => {
            // the reference: the one set before, else the active photo (the next one becomes active)
            let vis = app.session.visible_cloned();
            let reference = app
                .ui
                .reference
                .filter(|r| app.session.catalog.photo(lightcraft_catalog::PhotoId(*r)).is_some())
                .or_else(|| app.session.active().map(|a| a.0));
            let Some(r) = reference else { return Some(Err("select a photo to use as the reference".into())) };
            app.ui.reference = Some(r);
            if app.session.active().is_none_or(|a| a.0 == r)
                && let Some(i) = vis.iter().position(|x| x.0 == r)
                && let Some(next) = vis.get(i + 1).or(i.checked_sub(1).and_then(|j| vis.get(j)))
            {
                let _ = app.session.execute("library.select", &json!({"ids": [next.0]}));
            }
            app.ui.view = ViewMode::Reference;
            Ok(json!({"reference": r, "active": app.session.active().map(|a| a.0)}))
        }
        "photo.setReference" => {
            let id = p.get("id").and_then(Value::as_u64).or_else(|| app.session.active().map(|a| a.0));
            app.ui.reference = id;
            Ok(json!({"reference": id}))
        }
        "compare.swap" => crate::panels::compare::swap(app),
        "compare.makeSelect" => crate::panels::compare::make_select(app),
        "view.autoAdvance" => {
            app.ui.auto_advance = !app.ui.auto_advance;
            app.toast(&ctx, if app.ui.auto_advance { "Auto Advance On" } else { "Auto Advance Off" });
            Ok(json!({"autoAdvance": app.ui.auto_advance}))
        }
        "view.back" => {
            if app.ui.dialog.is_some() {
                app.ui.dialog = None;
            } else if app.ui.keyword_painter.is_some() {
                app.ui.keyword_painter = None;
            } else if app.ui.fullscreen {
                app.ui.fullscreen = false;
                app.ui.slideshow = None;
            } else if !app.ui.tool.is_empty() {
                app.ui.tool.clear();
            } else if app.ui.lights_out != crate::state::LightsOut::On {
                app.ui.lights_out = crate::state::LightsOut::On;
            } else if app.ui.module == Module::Develop {
                enter_library(app, Some(ViewMode::PhotoGrid));
            } else if matches!(app.ui.view, ViewMode::Compare | ViewMode::Survey) {
                app.ui.view = ViewMode::Detail;
            } else if app.ui.view == ViewMode::Detail {
                app.ui.view = ViewMode::PhotoGrid;
            }
            Ok(Value::Null)
        }
        "view.slideshow" => {
            // {interval?: seconds (4)}: the photos in view, full screen, one after another
            let interval = p.get("interval").and_then(Value::as_f64).unwrap_or(4.0).clamp(0.5, 120.0);
            if app.session.active().is_none() {
                let first = app.session.visible_cloned().first().copied();
                match first {
                    Some(f) => {
                        let _ = app.run("library.select", json!({"ids": [f.0]}));
                    }
                    None => return Some(Err("no photos to show".into())),
                }
            }
            let now = ctx.input(|i| i.time);
            app.ui.slideshow = Some((interval, now + interval, false));
            app.ui.fullscreen = true;
            app.ui.zoom = Zoom::Fit;
            app.ui.tool.clear();
            let _ = app.session.end_interaction();
            app.toast(&ctx, crate::i18n::tr("Slideshow · Space pauses · Esc ends"));
            Ok(json!({"interval": interval}))
        }
        "view.fullScreenPreview" => {
            app.ui.fullscreen = !app.ui.fullscreen;
            if !app.ui.fullscreen {
                app.ui.slideshow = None;
            }
            if app.ui.fullscreen {
                app.ui.zoom = Zoom::Fit;
                app.ui.tool.clear();
                let _ = app.session.end_interaction();
            }
            Ok(json!({"fullscreen": app.ui.fullscreen}))
        }
        "view.enterFullScreen" => {
            // applied by the host's frame logic (it knows the window's current state)
            let on = p.get("on").and_then(Value::as_bool);
            app.ui.window_fullscreen = Some(on.unwrap_or(!app.window_is_fullscreen));
            Ok(json!({"windowFullscreen": app.ui.window_fullscreen}))
        }
        "view.infoOverlay" => {
            app.ui.info_overlay = match p.get("mode").and_then(Value::as_str) {
                Some(m) => match serde_json::from_value(json!(m)) {
                    Ok(v) => v,
                    Err(_) => return Some(Err(format!("unknown info overlay `{m}` (off|basic|exposure)"))),
                },
                None => app.ui.info_overlay.next(),
            };
            let label = match app.ui.info_overlay {
                crate::state::InfoOverlay::Off => "Info Overlay Off",
                crate::state::InfoOverlay::Basic => "Info Overlay: File",
                crate::state::InfoOverlay::Exposure => "Info Overlay: Exposure",
            };
            app.toast(&ctx, label);
            Ok(json!({"infoOverlay": app.ui.info_overlay}))
        }
        "view.navigator" => {
            app.ui.navigator = !app.ui.navigator;
            Ok(json!({"navigator": app.ui.navigator}))
        }
        "app.settings" => {
            let tab = p.get("tab").and_then(Value::as_str).unwrap_or("general");
            if !crate::panels::settings::TABS.iter().any(|(id, _)| *id == tab) {
                return Some(Err(format!("unknown settings tab `{tab}` (general|import|performance|interface|shortcuts|controllers)")));
            }
            app.ui.dialog = Some(Dialog::Settings { tab: tab.into() });
            Ok(Value::Null)
        }
        "app.openLibrary" => crate::panels::settings::open_library(app, p),
        "file.backupLibrary" | "file.restoreLibrary" => {
            let action = if id == "file.backupLibrary" { app.services.backup_library.as_mut() } else { app.services.restore_library.as_mut() };
            match action {
                Some(f) => f(&mut app.session),
                None => Err("not available here: on the desktop the library is a folder; back it up with your other files".into()),
            }
        }
        "view.filmstrip" => {
            // {show?} (toggles when omitted)
            app.ui.filmstrip = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.filmstrip);
            Ok(json!({"show": app.ui.filmstrip}))
        }
        "view.leftPanel" => {
            app.ui.left_panel = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.left_panel);
            Ok(json!({"show": app.ui.left_panel}))
        }
        "view.beforeAfter" => {
            app.ui.before_after = if app.ui.before_after == BeforeAfter::SideBySide { BeforeAfter::Off } else { BeforeAfter::SideBySide };
            Ok(Value::Null)
        }
        "view.beforeAfterSplit" => {
            app.ui.before_after = if app.ui.before_after == BeforeAfter::Split { BeforeAfter::Off } else { BeforeAfter::Split };
            Ok(Value::Null)
        }
        "view.beforeAfterTopBottom" => {
            app.ui.before_after = if app.ui.before_after == BeforeAfter::TopBottom { BeforeAfter::Off } else { BeforeAfter::TopBottom };
            Ok(Value::Null)
        }
        "view.beforeAfterSplitTopBottom" => {
            app.ui.before_after = if app.ui.before_after == BeforeAfter::SplitTopBottom { BeforeAfter::Off } else { BeforeAfter::SplitTopBottom };
            Ok(Value::Null)
        }
        "view.showOriginal" => {
            app.ui.before_after = if app.ui.before_after == BeforeAfter::Original { BeforeAfter::Off } else { BeforeAfter::Original };
            Ok(Value::Null)
        }
        "view.zoomFit" => {
            app.ui.zoom = Zoom::Fit;
            Ok(Value::Null)
        }
        "view.zoom100" => {
            app.ui.zoom = Zoom::Percent(100.0);
            Ok(Value::Null)
        }
        "view.zoomToggle" => {
            // the same ratio a click on the photo zooms to
            app.ui.zoom = if app.ui.zoom == Zoom::Fit { Zoom::Percent(app.ui.click_zoom as f32) } else { Zoom::Fit };
            app.ui.zoom_anim = true;
            Ok(Value::Null)
        }
        "view.clickZoom" => {
            // {ratio?: 1|2|3|4|8} → {ratio}
            if let Some(r) = p.get("ratio").and_then(Value::as_f64) {
                let pct = (r * 100.0).round() as u32;
                if !crate::state::CLICK_ZOOMS.contains(&pct) {
                    return Some(Err(format!("view.clickZoom: ratio {r} (1, 2, 3, 4 or 8)")));
                }
                app.ui.click_zoom = pct;
            }
            Ok(json!({"ratio": app.ui.click_zoom / 100}))
        }
        "view.zoomIn" | "view.zoomOut" => {
            let steps = [25.0, 50.0, 100.0, 200.0, 400.0, 800.0];
            let cur = match app.ui.zoom {
                Zoom::Percent(p) => p,
                _ => 25.0,
            };
            let next = if id == "view.zoomIn" {
                steps.iter().find(|s| **s > cur).copied().unwrap_or(800.0)
            } else {
                steps.iter().rev().find(|s| **s < cur).copied().unwrap_or(0.0)
            };
            app.ui.zoom = if next == 0.0 { Zoom::Fit } else { Zoom::Percent(next) };
            Ok(Value::Null)
        }
        "view.navigate" => {
            // {zoom?: "fit"|"fill"|{percent: number}, pan?: [x, y]} (normalized image centre).
            // Validate the complete request before changing either part of the viewport.
            let zoom = match p.get("zoom") {
                Some(v) => match serde_json::from_value::<Zoom>(v.clone()) {
                    Ok(Zoom::Percent(p)) if !p.is_finite() || p <= 0.0 || p > 800.0 => {
                        return Some(Err("view.navigate: zoom percent must be greater than 0 and at most 800".into()));
                    }
                    Ok(z) => z,
                    Err(e) => return Some(Err(format!("view.navigate: {e}"))),
                },
                None => app.ui.zoom,
            };
            let pan = match p.get("pan") {
                Some(v) => match serde_json::from_value::<(f32, f32)>(v.clone()) {
                    Ok((x, y)) if x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) => (x, y),
                    _ => return Some(Err("view.navigate: pan must be [x, y] with finite coordinates from 0 to 1".into())),
                },
                None => app.ui.pan,
            };
            app.ui.zoom = zoom;
            app.ui.pan = pan;
            app.ui.zoom_anim = false;
            Ok(json!({"zoom": zoom, "pan": pan}))
        }
        "view.clipping" => {
            app.ui.show_clipping = !app.ui.show_clipping;
            Ok(Value::Null)
        }
        "view.softProof" => {
            // {on?, space?, destWarning?, displayWarning?}; no params toggles
            let has = |k: &str| p.get(k).is_some();
            if let Some(s) = p.get("space").and_then(Value::as_str) {
                match lightcraft_engine::pipeline::OutputSpace::parse(s) {
                    Some(sp) => app.ui.proof.space = sp,
                    None => return Some(Err(format!("view.softProof: unknown space {s:?} (srgb|displayP3|adobeRgb|proPhoto|rec2020)"))),
                }
            }
            if let Some(b) = p.get("destWarning").and_then(Value::as_bool) {
                app.ui.proof.dest_warning = b;
            }
            if let Some(b) = p.get("displayWarning").and_then(Value::as_bool) {
                app.ui.proof.display_warning = b;
            }
            app.ui.soft_proof = match p.get("on").and_then(Value::as_bool) {
                Some(b) => b,
                None if has("space") || has("destWarning") || has("displayWarning") => app.ui.soft_proof,
                None => !app.ui.soft_proof,
            };
            if app.ui.soft_proof && !matches!(app.ui.view, ViewMode::Detail | ViewMode::Reference) {
                app.ui.view = ViewMode::Detail;
            }
            let pr = app.ui.proof;
            Ok(json!({"on": app.ui.soft_proof, "space": pr.space, "destWarning": pr.dest_warning, "displayWarning": pr.display_warning}))
        }
        "view.histogram" => {
            app.ui.histogram = !app.ui.histogram;
            Ok(Value::Null)
        }
        "view.maskOverlay" => {
            app.ui.mask_overlay = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.mask_overlay);
            Ok(json!({"maskOverlay": app.ui.mask_overlay}))
        }
        "view.maskOverlayMode" => {
            use lightcraft_engine::pipeline::MaskView;
            let cur = MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default();
            let next = match p.get("mode").and_then(Value::as_str) {
                Some(m) => match MaskView::parse(m) {
                    Some(v) => v,
                    None => {
                        let names: Vec<&str> = MaskView::ALL.iter().map(|v| v.name()).collect();
                        return Some(Err(format!("view.maskOverlayMode: unknown mode `{m}` ({})", names.join("|"))));
                    }
                },
                None => cur.next(),
            };
            app.ui.mask_overlay_mode = next.name().into();
            app.ui.mask_overlay = true;
            app.toast(&ctx, next.label());
            Ok(json!({"mode": next.name()}))
        }
        "view.maskOverlayColor" => {
            // no params: the next of the panel's swatch colours
            if p.get("color").is_none() && p.get("opacity").is_none() {
                let all = crate::panels::masking::OVERLAY_COLORS;
                let i = all.iter().position(|c| *c == app.ui.mask_overlay_color).map_or(0, |i| (i + 1) % all.len());
                app.ui.mask_overlay_color = all[i];
            }
            if let Some(c) = p.get("color") {
                match parse_rgb(c) {
                    Some(rgb) => app.ui.mask_overlay_color = rgb,
                    None => return Some(Err("view.maskOverlayColor: `color` is \"#rrggbb\" or [r, g, b]".into())),
                }
            }
            if let Some(o) = p.get("opacity").and_then(Value::as_f64) {
                app.ui.mask_overlay_opacity = o.clamp(0.0, 100.0) as f32;
            }
            let [r, g, b] = app.ui.mask_overlay_color;
            Ok(json!({"color": format!("#{r:02x}{g:02x}{b:02x}"), "opacity": app.ui.mask_overlay_opacity}))
        }
        "brush.smaller" | "brush.larger" | "brush.featherLess" | "brush.featherMore" => {
            let (k, df) = match id {
                "brush.smaller" => (1.0 / 1.2, 0.0),
                "brush.larger" => (1.2, 0.0),
                "brush.featherLess" => (1.0, -10.0),
                _ => (1.0, 10.0),
            };
            Ok(adjust_brush(app, k, df))
        }
        "view.maskPins" => {
            app.ui.mask_pins = p.get("show").and_then(Value::as_bool).unwrap_or(!app.ui.mask_pins);
            Ok(json!({"maskPins": app.ui.mask_pins}))
        }
        "view.visualizeSpots" => {
            // like Lightroom's A: opens the Remove tool with the view on, or toggles it there
            if app.ui.right == RightPanel::Remove {
                app.ui.visualize_spots = !app.ui.visualize_spots;
            } else {
                app.ui.right = RightPanel::Remove;
                app.ui.visualize_spots = true;
            }
            Ok(Value::Null)
        }
        "view.cropOverlay" => {
            use crate::state::CropOverlay::*;
            app.ui.crop_overlay = match app.ui.crop_overlay {
                Thirds => Grid,
                Grid => Golden,
                Golden => Diagonal,
                Diagonal => Triangle,
                Triangle => Spiral,
                Spiral => None,
                None => Thirds,
            };
            Ok(json!({"overlay": app.ui.crop_overlay}))
        }
        "view.cropOverlayOrientation" => {
            app.ui.crop_overlay_orient = (app.ui.crop_overlay_orient + 1) % 4;
            Ok(json!({"orientation": app.ui.crop_overlay_orient}))
        }
        "local.addRoot" => {
            // a folder kept in Local's sidebar (saved with the UI state); nothing on disk changes
            let Some(path) = p.get("path").and_then(Value::as_str) else { return Some(Err("local.addRoot needs a path".into())) };
            let abs = std::path::absolute(path).map(|a| a.to_string_lossy().trim_end_matches(['/', '\\']).to_string()).unwrap_or(path.into());
            let abs = if abs.is_empty() { path.to_string() } else { abs };
            if !std::path::Path::new(&abs).is_dir() {
                return Some(Err(format!("{path}: not a folder")));
            }
            use crate::panels::left::same_folder;
            if !app.ui.local_roots.iter().any(|r| same_folder(r, &abs)) {
                app.ui.local_roots.push(abs.clone());
            }
            app.ui.hidden_locations.retain(|h| !same_folder(h, &abs));
            Ok(json!({"roots": app.ui.local_roots}))
        }
        "local.hide" => match p.get("path").and_then(Value::as_str) {
            Some(path) => {
                if !app.ui.hidden_locations.iter().any(|h| crate::panels::left::same_folder(h, path)) {
                    app.ui.hidden_locations.push(path.to_string());
                }
                Ok(json!({"hidden": app.ui.hidden_locations}))
            }
            None => Err("local.hide needs a path".into()),
        },
        "local.restoreHidden" => {
            // one path, or (no path) every hidden location
            match p.get("path").and_then(Value::as_str) {
                Some(path) => app.ui.hidden_locations.retain(|h| !crate::panels::left::same_folder(h, path)),
                None => app.ui.hidden_locations.clear(),
            }
            Ok(json!({"hidden": app.ui.hidden_locations}))
        }
        "view.filterBar" => {
            app.ui.filter_bar = !app.ui.filter_bar;
            if app.ui.filter_bar && !matches!(app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid) {
                app.ui.view = ViewMode::PhotoGrid;
            }
            Ok(json!({"filterBar": app.ui.filter_bar}))
        }
        "panel.edit" => {
            // Lightroom desktop's E: the edit panels (Develop), or back to the Library loupe
            if app.ui.module == Module::Develop && app.ui.right == RightPanel::Edit {
                enter_library(app, Some(ViewMode::Detail));
                app.toast(&ctx, crate::i18n::tr("Library"));
            } else {
                enter_develop(app);
                app.ui.right = RightPanel::Edit;
                app.toast(&ctx, crate::i18n::tr("Develop"));
            }
            Ok(json!({"module": app.ui.module}))
        }
        "panel.profiles" => {
            // toggles between the profile browser and the Basic panel it belongs to
            let open = app.ui.right != RightPanel::Profiles;
            enter_develop(app);
            app.ui.right = if open { RightPanel::Profiles } else { RightPanel::Edit };
            Ok(json!({"open": app.ui.right == RightPanel::Profiles}))
        }
        "panel.crop" => {
            panel(app, &ctx, RightPanel::Crop, "Crop, Rotate, Geometry");
            Ok(Value::Null)
        }
        "panel.remove" => {
            panel(app, &ctx, RightPanel::Remove, "Remove");
            if app.ui.right == RightPanel::Remove && app.ui.tool.is_empty() {
                app.ui.tool = "remove".into();
            }
            Ok(Value::Null)
        }
        "panel.masking" => {
            panel(app, &ctx, RightPanel::Masking, "Masking");
            Ok(Value::Null)
        }
        "panel.redeye" => {
            panel(app, &ctx, RightPanel::RedEye, "Red Eye");
            Ok(Value::Null)
        }
        "panel.info" => {
            panel(app, &ctx, RightPanel::Info, "Info");
            Ok(Value::Null)
        }
        "panel.keywords" => {
            panel(app, &ctx, RightPanel::Keywords, "Keywords");
            Ok(Value::Null)
        }
        "panel.versions" => {
            panel(app, &ctx, RightPanel::Versions, "Versions");
            Ok(Value::Null)
        }
        "panel.activity" => {
            panel(app, &ctx, RightPanel::Activity, "History");
            Ok(Value::Null)
        }
        "panel.presets" => {
            // Develop's left panel ▸ Presets
            let open = !(app.ui.module == Module::Develop && app.ui.left_panel && app.ui.develop_left_sections.iter().any(|s| s == "presets"));
            enter_develop(app);
            if open {
                open_section(app, true, "presets");
            } else {
                app.ui.develop_left_sections.retain(|s| s != "presets");
            }
            app.ui.presets = open;
            Ok(json!({"open": open}))
        }
        "tool.done" => {
            // Return commits a tool panel (crop, remove, red eye, masking): back to Edit
            use RightPanel::*;
            if app.ui.dialog.is_none() && matches!(app.ui.right, Crop | Remove | RedEye | Masking) {
                let _ = app.session.end_interaction();
                app.ui.tool.clear();
                app.ui.right = Edit;
            }
            Ok(Value::Null)
        }
        "panel.close" => {
            app.ui.right = if app.ui.module == Module::Develop { RightPanel::Edit } else { RightPanel::None };
            app.ui.presets = false;
            Ok(Value::Null)
        }
        s if s.starts_with("section.") => {
            let sec = crate::panels::edit::classic_section(&s["section.".len()..]);
            enter_develop(app);
            app.ui.right = RightPanel::Edit;
            app.ui.toggle_section(sec);
            Ok(json!({"section": sec, "open": app.ui.section_open(sec)}))
        }
        s if s.starts_with("tool.") => {
            let tool = &s["tool.".len()..];
            match tool {
                "none" => app.ui.tool.clear(),
                "guidedUpright" => {
                    // Transform with Guided Upright on, ready to draw guides
                    enter_develop(app);
                    app.ui.right = RightPanel::Crop;
                    let guided = app
                        .session
                        .active()
                        .and_then(|id| app.session.develop_of(id))
                        .is_some_and(|d| d.geometry.upright == lightcraft_develop::Upright::Guided);
                    if !guided && let Err(e) = app.run("geometry.upright", json!({"mode": "guided"})) {
                        return Some(Err(e));
                    }
                    app.ui.tool = "guidedUpright".into();
                }
                "brush" => {
                    enter_develop(app);
                    app.ui.right = RightPanel::Masking;
                    app.ui.tool = "brush".into();
                }
                "linear" | "radial" => {
                    enter_develop(app);
                    app.ui.right = RightPanel::Masking;
                    app.ui.tool = tool.into();
                    return Some(app.session.execute("mask.add", &json!({"kind": tool})).map_err(|e| e.to_string()));
                }
                "wbPicker" => {
                    enter_develop(app);
                    app.ui.right = RightPanel::Edit;
                    app.ui.tool = "wbPicker".into();
                }
                other => return Some(Err(format!("unknown tool `{other}`"))),
            }
            Ok(Value::Null)
        }
        "dialog.newFolder" => {
            app.ui.dialog = Some(Dialog::NewAlbum { name: p.get("name").and_then(Value::as_str).unwrap_or("").into(), folder: true });
            Ok(Value::Null)
        }
        "dialog.newAlbum" => {
            app.ui.dialog = Some(Dialog::NewAlbum { name: p.get("name").and_then(Value::as_str).unwrap_or("").into(), folder: false });
            Ok(Value::Null)
        }
        "dialog.autoStack" => {
            app.ui.dialog = Some(Dialog::AutoStack { gap: p.get("gap").and_then(Value::as_f64).unwrap_or(60.0) as f32 });
            Ok(Value::Null)
        }
        "dialog.allMetadata" => {
            let r = match app.session.execute("photo.allMetadata", p) {
                Ok(r) => r,
                Err(e) => return Some(Err(e.to_string())),
            };
            let title = app.session.active().and_then(|id| app.session.catalog.photo(id)).map(|p| p.file_name.clone()).unwrap_or_default();
            app.ui.dialog = Some(Dialog::AllMetadata { title, rows: r, search: String::new() });
            Ok(Value::Null)
        }
        "dialog.smartAlbum" => {
            // {id?}: edit that smart album's rules; without: a new one starting at Rating ≥ 3
            let album = p
                .get("id")
                .and_then(Value::as_u64)
                .and_then(|id| app.session.catalog.album(lightcraft_catalog::AlbumId(id)).filter(|a| a.is_smart()).cloned());
            app.ui.dialog = Some(match album {
                Some(a) => Dialog::SmartRules {
                    id: Some(a.id.0),
                    name: a.name.clone(),
                    rules: a.smart.as_ref().and_then(|f| f.rule_set.clone()).unwrap_or_default(),
                },
                None => Dialog::SmartRules {
                    id: None,
                    name: p.get("name").and_then(Value::as_str).unwrap_or("").into(),
                    rules: lightcraft_catalog::RuleSet { rules: vec![crate::panels::rules_editor::new_rule()], ..Default::default() },
                },
            });
            Ok(Value::Null)
        }
        "dialog.newSmartAlbum" => {
            app.ui.dialog = Some(Dialog::NewSmartAlbum { name: p.get("name").and_then(Value::as_str).unwrap_or("").into() });
            Ok(Value::Null)
        }
        "dialog.captureTime" => {
            let time = app.session.active().and_then(|id| app.session.catalog.photo(id)).map(|p| p.date().replace('T', " ")).unwrap_or_default();
            let mode = p.get("mode").and_then(Value::as_str).unwrap_or("set").to_string();
            app.ui.dialog =
                Some(Dialog::CaptureTime { mode, time: time.get(..19).unwrap_or(&time).to_string(), days: 0, hours: 0, minutes: 0, zone: 0.0 });
            Ok(Value::Null)
        }
        "dialog.labelNames" => {
            let names =
                lightcraft_catalog::ColorLabel::ALL.iter().map(|l| app.session.catalog.custom_label_name(*l).unwrap_or("").to_string()).collect();
            app.ui.dialog = Some(Dialog::LabelNames { names, save_as: String::new() });
            Ok(Value::Null)
        }
        "dialog.rename" => {
            let template = p.get("template").and_then(Value::as_str).unwrap_or("{name}").to_string();
            app.ui.dialog = Some(Dialog::Rename { template, start: p.get("start").and_then(Value::as_u64).unwrap_or(1) as u32 });
            Ok(Value::Null)
        }
        "dialog.createPreset" => {
            app.ui.dialog = Some(Dialog::create_preset());
            Ok(Value::Null)
        }
        "dialog.pasteSettings" => {
            let groups =
                app.session.copy_groups.iter().filter_map(|g| serde_json::to_value(g).ok().and_then(|v| v.as_str().map(str::to_string))).collect();
            app.ui.dialog = Some(Dialog::PasteSettings { groups });
            Ok(Value::Null)
        }
        "view.focusSearch" => {
            // the search field lives in the top bar of the library and detail views alike
            app.ui.focus_search = true;
            Ok(Value::Null)
        }
        "dialog.syncSettings" => {
            let groups = crate::state::default_preset_groups();
            app.ui.dialog = Some(Dialog::SyncSettings { groups });
            Ok(Value::Null)
        }
        "dialog.copySettings" => {
            let groups =
                app.session.copy_groups.iter().filter_map(|g| serde_json::to_value(g).ok().and_then(|v| v.as_str().map(str::to_string))).collect();
            app.ui.dialog = Some(Dialog::CopySettings { groups });
            Ok(Value::Null)
        }
        "dialog.export" => {
            let prev = app.session.last_export.clone().unwrap_or_default();
            let u = |k: &str, d: u64| prev.get(k).and_then(Value::as_u64).unwrap_or(d);
            let dir = prev.get("dir").and_then(Value::as_str).map(str::to_string).unwrap_or_else(crate::control::default_export_dir);
            let opts = lightcraft_engine::export::ExportOptions::from_json(&prev);
            // no previous export: 2048 px long edge; a previous full-size export: full size
            let full_size = opts.resize.is_none() && lightcraft_engine::export::ExportOptions::has_size_param(&prev);
            let resize = opts.resize.unwrap_or_default();
            app.ui.dialog = Some(Dialog::Export { opts, full_size, resize, preset_name: String::new(), limit_kb: u("limitKb", 0) as u32, dir });
            Ok(Value::Null)
        }
        "merge.hdrLast" => crate::merge::start_last(app, "merge.hdr"),
        "merge.panoramaLast" => crate::merge::start_last(app, "merge.panorama"),
        "merge.hdrPanoramaLast" => crate::merge::start_last(app, "merge.hdrPanorama"),
        "dialog.mergeHdr" => crate::merge::open(app, "merge.hdr"),
        "dialog.mergePanorama" => crate::merge::open(app, "merge.panorama"),
        "dialog.mergeHdrPanorama" => crate::merge::open(app, "merge.hdrPanorama"),
        "app.about" => {
            app.ui.dialog = Some(Dialog::About);
            Ok(Value::Null)
        }
        "photo.editInExternal" => {
            // render an edit copy (stacked on the original), then open it in the editor
            let mut params = p.clone();
            if !params.is_object() {
                params = json!({});
            }
            let r = match app.session.execute("photo.editExternal", &params) {
                Ok(r) => r,
                Err(e) => return Some(Err(e.to_string())),
            };
            let path = r["path"].as_str().unwrap_or_default().to_string();
            if let Some(id) = r["id"].as_u64() {
                app.ui.external_edits.push(id);
            }
            let editor = p.get("app").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| app.ui.settings.external_editor.clone());
            if let Some(f) = app.services.open_with.as_mut()
                && let Err(e) = f(&path, &editor)
            {
                app.toast(&ctx, crate::i18n::tr_format!("Couldn't open the editor: {e}", e = e));
            }
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            app.toast(&ctx, crate::i18n::tr_format!("{name} opened for editing; it is stacked with the original", name = name));
            Ok(r)
        }
        "dialog.cull" => {
            app.ui.dialog = Some(Dialog::Cull { reject_below: 0.0, pick_best: true });
            Ok(Value::Null)
        }
        "app.whatsNew" => {
            app.ui.dialog = Some(Dialog::WhatsNew);
            Ok(json!({"text": crate::panels::dialogs::WHATS_NEW}))
        }
        "app.systemInfo" => {
            let info = app.session.execute("library.info", &json!({})).unwrap_or_default();
            let gpu = (lightcraft_engine::gpu::ready() && lightcraft_engine::gpu::available()).then(lightcraft_engine::gpu::adapter_name).flatten();
            let mb = |b: u64| format!("{:.0} MB", b as f64 / (1u64 << 20) as f64);
            let mut rows = vec![
                ("Version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
                ("System".to_string(), format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)),
                ("CPU threads".to_string(), std::thread::available_parallelism().map(|n| n.get().to_string()).unwrap_or_else(|_| "?".into())),
                (
                    "GPU".to_string(),
                    gpu.unwrap_or_else(|| match lightcraft_engine::gpu::unavailable_reason() {
                        Some(why) => crate::i18n::tr_format!("none (CPU rendering): {why}", why = why),
                        None => "none (CPU rendering)".into(),
                    }),
                ),
                ("GPU rendering".to_string(), if app.ui.settings.gpu { "on".into() } else { "off".into() }),
                ("Memory budget".to_string(), mb(lightcraft_engine::memory::default_budget() as u64)),
                ("Preview size".to_string(), format!("{} px", app.ui.settings.preview_edge)),
                ("Photos".to_string(), info["photos"].to_string()),
                ("Albums".to_string(), info["albums"].to_string()),
            ];
            if let Some(dir) = info["dir"].as_str().or(info["path"].as_str()) {
                rows.push(("Library".into(), dir.to_string()));
            }
            rows.push(("Frame time".into(), format!("{:.1} ms ({:.0} fps)", app.perf.frame_ms, app.perf.fps)));
            rows.push((
                "Frame update".into(),
                format!("{:.1} ms (logic {:.1} ms; slowest {:.0} ms)", app.perf.update_ms, app.perf.logic_ms, app.perf.max_update_ms),
            ));
            rows.push(("Last loupe render".into(), format!("{:.0} ms", app.renderer.last_main_ms)));
            if let Some(f) = lightcraft_engine::gpu::last_fallback() {
                rows.push(("Last GPU fallback".into(), f));
            }
            let r = json!(rows.iter().map(|(k, v)| json!({"label": k, "value": v})).collect::<Vec<_>>());
            if p.get("open").and_then(Value::as_bool).unwrap_or(true) {
                app.ui.dialog = Some(Dialog::SystemInfo { rows });
            }
            Ok(r)
        }
        "app.shortcuts" => {
            app.ui.dialog = Some(Dialog::Shortcuts);
            Ok(Value::Null)
        }
        // File ▸ Migrate from Lightroom Classic…: read + import in the background (lr_migrate.rs)
        "library.migrateLightroom" if !cfg!(target_arch = "wasm32") && crate::lr_migrate::runs_in_background(p) => crate::lr_migrate::start(app, p),
        "library.browse" if !cfg!(target_arch = "wasm32") => {
            // listed and read in the background (see `import::browse`)
            let path = p.get("path").and_then(Value::as_str)?;
            crate::import::browse(app, path, p.get("subfolders").and_then(Value::as_bool))
        }
        "file.addPhotos" => {
            let paths: Vec<String> = match p.get("paths").and_then(Value::as_array) {
                Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                // the Import window (Lightroom Classic's): source, files, options in one place
                None if crate::import::has_window() => return Some(crate::import::open_window(app, p)),
                None => app.services.pick_files.as_mut().map(|f| f()).unwrap_or_default(),
            };
            if paths.is_empty() {
                return Some(Ok(Value::Null));
            }
            // review first: the import dialog lists what was found
            crate::import::open(app, paths)
        }
        "app.quit" => {
            app.ui.quit = true;
            Ok(Value::Null)
        }
        "dialog.saveMetadataPreset" => {
            // from the active photo's copyright, creator and place
            crate::panels::dialogs::prompt(app, "Save Metadata Preset", "Preset name", "", "metadata.savePreset", json!({}), "name");
            Ok(Value::Null)
        }
        "file.findMissing" => {
            let folder = match p.get("folder").and_then(Value::as_str) {
                Some(f) => Some(f.to_string()),
                None => app.services.pick_folder.as_mut().and_then(|f| f()),
            };
            let Some(folder) = folder else { return Some(Ok(Value::Null)) };
            // the search (checking every photo's file, walking the folder) runs on a worker thread;
            // the relinking happens back here, as one undo step
            const LABEL: &str = "Find Missing Photos";
            if app.tasks.is_running(LABEL) {
                return Some(Err("Find Missing Photos is already searching".into()));
            }
            let candidates = lightcraft_engine::cmd::missing::find_candidates(&app.session.catalog);
            let work = move || lightcraft_engine::cmd::missing::plan_find_missing(&candidates, &folder);
            let done = |app: &mut LightcraftApp, ctx: &egui::Context, plan: Result<lightcraft_engine::cmd::missing::FindPlan, String>| {
                // relinked under the session as it is now: photos relinked meanwhile and files
                // now in use are skipped
                let r = plan.and_then(|plan| app.run("library.findMissing", plan.to_json()));
                match r {
                    Ok(v) => {
                        let n = v["found"].as_array().map_or(0, Vec::len);
                        let left = v["missing"].as_u64().unwrap_or(0);
                        let unsure = v["ambiguous"].as_array().map_or(0, Vec::len);
                        let unsure = if unsure > 0 {
                            crate::i18n::tr_format!(" ({unsure} with several look-alike files: use Locate)", unsure = unsure)
                        } else {
                            String::new()
                        };
                        app.toast(
                            ctx,
                            crate::i18n::tr_format!(
                                "Found {n} missing photo{}; {left} still missing{unsure}",
                                if n == 1 { "" } else { "s" },
                                left = left,
                                n = n,
                                unsure = unsure
                            ),
                        );
                        app.ui.last_find_missing = Some(v);
                    }
                    Err(e) => app.toast(ctx, e),
                }
            };
            app.ui.last_find_missing = None;
            if let Err(e) = crate::tasks::spawn(app, LABEL, work, done) {
                return Some(Err(e));
            }
            if p.get("wait").and_then(Value::as_bool).unwrap_or(false) {
                let ctx = app.tasks.repaint.clone().unwrap_or_default();
                crate::tasks::wait(app, &ctx, std::time::Duration::from_secs(600));
                return Some(Ok(app.ui.last_find_missing.clone().unwrap_or(Value::Null)));
            }
            Ok(json!({"background": true}))
        }
        "photo.tagFromTracklog" => {
            // a GPX file → GPS for the selected photos by capture time (one undo step)
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.pick_tracklog.as_mut().and_then(|f| f().into_iter().next()),
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            let mut params = p.as_object().cloned().unwrap_or_default();
            params.insert("path".into(), json!(path));
            let ask_zone = !params.contains_key("offset");
            let params = Value::Object(params);
            if ask_zone {
                // GPX times are UTC, camera clocks are local: ask for the camera's zone, then come back here
                crate::panels::dialogs::prompt(
                    app,
                    "Auto-Tag from Tracklog",
                    "Camera time zone, e.g. -07:00 (empty: UTC)",
                    "",
                    "photo.tagFromTracklog",
                    params,
                    "offset",
                );
                return Some(Ok(Value::Null));
            }
            let r = app.run("photo.autoTagTracklog", params);
            if let Ok(v) = &r {
                let n = v["tagged"].as_u64().unwrap_or(0);
                let sk = &v["skipped"];
                let mut msg = crate::i18n::tr_format!("Tagged {n} photo{} from the tracklog", if n == 1 { "" } else { "s" }, n = n);
                let outside = sk["outside"].as_u64().unwrap_or(0);
                if outside > 0 {
                    msg += &crate::i18n::tr_format!("; {outside} outside its time range", outside = outside);
                }
                let kept = sk["hasGps"].as_u64().unwrap_or(0);
                if kept > 0 {
                    msg += &crate::i18n::tr_format!("; {kept} already had a location", kept = kept);
                }
                app.toast(&egui::Context::default(), msg);
            }
            r
        }
        "photo.locate" => {
            let Some(id) = app.session.active() else { return Some(Err("no photo selected".into())) };
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.pick_files.as_mut().and_then(|f| f().into_iter().next()),
            };
            match path {
                Some(path) => app.run("photo.relink", json!({"id": id.0, "path": path})),
                None => Ok(Value::Null),
            }
        }
        "file.addFromDevice" => {
            // a camera / card: review its DCIM folder, copying into the library by default
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => x.to_string(),
                None => match lightcraft_engine::devices::devices_now().into_iter().next() {
                    Some(d) => d.path,
                    None => return Some(Err("no camera or memory card found".into())),
                },
            };
            let r = crate::import::open(app, vec![path]);
            // only the scan just started (an error means another scan is running)
            if r.is_ok()
                && let Some(t) = &mut app.scan
            {
                t.copy = true;
            }
            r
        }
        "file.addFolder" => {
            // a folder (searched recursively) into the import review
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.pick_folder.as_mut().and_then(|f| f()),
            };
            match path {
                Some(path) => crate::import::open(app, vec![path]),
                None => Ok(Value::Null),
            }
        }
        "file.importPresets" => {
            let paths = match p.get("paths").and_then(Value::as_array) {
                Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                None => match app.services.pick_preset_files.as_mut() {
                    Some(f) => f(),
                    None => return Some(Err("no file dialog on this platform".into())),
                },
            };
            if paths.is_empty() {
                return Some(Ok(Value::Null));
            }
            // .cube LUTs (and those inside zips / folders) become profiles
            let lut_paths: Vec<&String> = paths
                .iter()
                .filter(|p| {
                    let l = p.to_ascii_lowercase();
                    l.ends_with(".cube") || l.ends_with(".zip") || std::path::Path::new(p.as_str()).is_dir()
                })
                .collect();
            let profiles = if lut_paths.is_empty() {
                0
            } else {
                app.session
                    .execute("profile.import", &json!({"paths": lut_paths}))
                    .ok()
                    .and_then(|v| v["imported"].as_array().map(Vec::len))
                    .unwrap_or(0)
            };
            let preset_paths: Vec<&String> = paths.iter().filter(|p| !p.to_ascii_lowercase().ends_with(".cube")).collect();
            let r = if preset_paths.is_empty() {
                Ok(json!({"imported": [], "failed": [], "skipped": 0}))
            } else {
                app.session.execute("preset.import", &json!({"paths": preset_paths})).map_err(|e| e.to_string())
            };
            if profiles > 0 {
                app.toast(
                    &ctx,
                    crate::i18n::tr_format!(
                        "Imported {profiles} profile{} (Profile browser ▸ their groups)",
                        if profiles == 1 { "" } else { "s" },
                        profiles = profiles
                    ),
                );
            }
            if let Ok(v) = &r {
                let n = v["imported"].as_array().map_or(0, Vec::len);
                let failed = v["failed"].as_array().map_or(0, Vec::len);
                let mut msg = match (n, failed) {
                    (0, 0) => "No new presets".to_string(),
                    (n, 0) => crate::i18n::tr_format!("Imported {n} preset{}", if n == 1 { "" } else { "s" }, n = n),
                    (n, f) => crate::i18n::tr_format!(
                        "Imported {n} preset{}, {f} file{} not readable",
                        if n == 1 { "" } else { "s" },
                        if f == 1 { "" } else { "s" },
                        f = f,
                        n = n
                    ),
                };
                // settings with no counterpart here (the other editor's profiles, masks…)
                let mut skipped: Vec<&str> = v["imported"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|i| i["unmapped"].as_array().into_iter().flatten())
                    .filter_map(Value::as_str)
                    .collect();
                skipped.sort_unstable();
                skipped.dedup();
                if !skipped.is_empty() {
                    let names: Vec<&str> = skipped.iter().take(3).copied().collect();
                    msg += &crate::i18n::tr_format!(" — not carried over: {}{}", names.join(", "), if skipped.len() > 3 { "…" } else { "" });
                }
                app.toast(&ctx, msg);
                if n > 0 {
                    app.ui.presets = true;
                }
            }
            return Some(r);
        }
        "file.exportPresets" => {
            let group = p.get("group").and_then(Value::as_str).map(str::to_string);
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => {
                    let name = format!("{}.lcpreset", group.as_deref().unwrap_or("LightCraft Presets"));
                    match app.services.save_preset_file.as_mut() {
                        Some(f) => f(&name),
                        None => return Some(Err("no file dialog on this platform".into())),
                    }
                }
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            let mut params = json!({"path": path});
            if let Some(g) = group {
                params["group"] = json!(g);
            }
            if let Some(ids) = p.get("ids") {
                params["ids"] = ids.clone();
            }
            let r = app.session.execute("preset.export", &params).map_err(|e| e.to_string());
            if let Ok(v) = &r {
                app.toast(&ctx, crate::i18n::tr_format!("Exported {} preset{}", v["count"], if v["count"] == 1 { "" } else { "s" }));
            }
            return Some(r);
        }
        "file.importCurvePresets" => {
            let paths: Vec<String> = match p.get("paths").and_then(Value::as_array) {
                Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                None => match app.services.pick_curve_preset_files.as_mut() {
                    Some(f) => f(),
                    None => return Some(Err("no file dialog on this platform".into())),
                },
            };
            if paths.is_empty() {
                return Some(Ok(Value::Null));
            }
            let r = app.session.execute("curve.importPresets", &json!({"paths": paths})).map_err(|e| e.to_string());
            if let Ok(v) = &r {
                let n = v["imported"].as_array().map_or(0, Vec::len);
                let failed = v["failed"].as_array().map_or(0, Vec::len);
                let mut msg = crate::i18n::tr_format!("Imported {n} point curve preset{}", if n == 1 { "" } else { "s" }, n = n);
                if failed > 0 {
                    msg += &crate::i18n::tr_format!(", {failed} file{} not readable", if failed == 1 { "" } else { "s" }, failed = failed);
                }
                app.toast(&ctx, msg);
            }
            return Some(r);
        }
        "file.exportCurvePresets" => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => match app.services.save_curve_preset_file.as_mut() {
                    Some(f) => f("Point Curves.lccurve"),
                    None => return Some(Err("no file dialog on this platform".into())),
                },
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            let mut params = json!({"path": path});
            if let Some(n) = p.get("names") {
                params["names"] = n.clone();
            }
            let r = app.session.execute("curve.exportPresets", &params).map_err(|e| e.to_string());
            if let Ok(v) = &r {
                app.toast(&ctx, crate::i18n::tr_format!("Exported {} point curve preset{}", v["count"], if v["count"] == 1 { "" } else { "s" }));
            }
            return Some(r);
        }
        "app.export" => crate::control::export_active(app, p),
        "app.showInFinder" => show_in_finder(app),
        "app.discord" | "app.website" | "app.github" | "app.artcraft" | "app.help" | "app.feedback" => {
            let url = crate::links::url_of(id).unwrap_or(crate::links::WEBSITE);
            crate::links::open(app, url)
        }
        "app.exportPrevious" => match app.session.last_export.clone() {
            Some(prev) => crate::control::export_active(app, &lightcraft_engine::export::ExportOptions::known_keys_only(&prev)),
            None => Err("nothing exported yet — use Export…".into()),
        },
        _ => return None,
    };
    Some(r)
}

pub fn ui_enabled(app: &LightcraftApp, id: &str) -> bool {
    match id {
        s if s.starts_with("panel.") || s.starts_with("tool.") || s.starts_with("section.") => app.session.active().is_some() || s == "panel.close",
        "dialog.syncSettings" => app.session.selection.ids.len() > 1,
        "app.export" | "dialog.export" | "dialog.createPreset" | "dialog.rename" | "dialog.captureTime" | "dialog.copySettings" => {
            app.session.active().is_some()
        }
        "photo.tagFromTracklog" => app.session.active().is_some() && app.services.pick_tracklog.is_some(),
        "app.exportPrevious" => app.session.active().is_some() && app.session.last_export.is_some(),
        "dialog.pasteSettings" => app.session.active().is_some() && app.session.clipboard.is_some(),
        "app.showInFinder" => {
            app.services.reveal.is_some()
                && app
                    .session
                    .active()
                    .and_then(|id| app.session.catalog.photo(id))
                    .is_some_and(|p| matches!(p.source, lightcraft_engine::catalog::Source::File { .. }))
        }
        "file.exportPresets" => app.session.presets.iter().any(|p| !p.builtin),
        "file.exportCurvePresets" => !app.session.curve_presets.is_empty(),
        "view.compare" => app.session.catalog.len() > 1,
        "view.fullScreenPreview" | "view.infoOverlay" | "view.navigator" => app.session.active().is_some() || app.ui.fullscreen,
        "app.openLibrary" | "file.addFolder" => app.services.pick_folder.is_some(),
        "file.backupLibrary" => app.services.backup_library.is_some(),
        "file.restoreLibrary" => app.services.restore_library.is_some(),
        "compare.swap" | "compare.makeSelect" => app.ui.view == ViewMode::Compare,
        s if s.starts_with("dialog.merge") || (s.starts_with("merge.") && s.ends_with("Last")) => {
            app.session.targets(&serde_json::json!({})).len() >= 2 && app.merge.final_task.is_none()
        }
        _ => true,
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MenuEntry {
    pub id: String,
    pub label: String,
    pub menu: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
}

/// The flattened menu model (UI commands + engine commands with menu paths).
/// Commands that live in Lightroom Classic's Library menu, whatever menu their declaration names
/// (the engine's own commands keep their paths for the CLI and MCP).
pub const LIBRARY_MENU: &[&str] = &[
    "dialog.newAlbum",
    "dialog.newFolder",
    "dialog.smartAlbum",
    "dialog.newSmartAlbum",
    "view.filterBar",
    "library.clearFilter",
    "view.previousImport",
    "library.showSubfolders",
    "file.findMissing",
    "library.syncMetadata",
];

pub fn menu_entries(app: &LightcraftApp) -> Vec<MenuEntry> {
    let mut v: Vec<MenuEntry> = ui_commands()
        .filter(|c| !c.3.is_empty())
        .map(|(id, label, sc, m)| MenuEntry {
            id: id.to_string(),
            label: label.to_string(),
            menu: m.split('>').map(str::to_string).collect(),
            shortcut: crate::keymap::menu_shortcut(app.keymap.overlay(), id, *sc),
            enabled: ui_enabled(app, id),
        })
        .collect();
    for c in app.session.commands() {
        if !c.menu.is_empty() {
            v.push(MenuEntry {
                id: c.id.into(),
                label: c.label.into(),
                menu: c.menu.iter().map(|s| s.to_string()).collect(),
                shortcut: crate::keymap::menu_shortcut(app.keymap.overlay(), c.id, c.shortcut),
                enabled: c.enabled,
            });
        }
    }
    for e in v.iter_mut().filter(|e| LIBRARY_MENU.contains(&e.id.as_str())) {
        e.menu = vec!["Library".into()];
    }
    v
}

/// With Settings → General → "Confirm before deleting" on, open the confirmation dialog instead
/// of deleting; true when it did (the dialog's OK runs `photo.delete`).
pub fn confirm_delete(app: &mut LightcraftApp) -> bool {
    if !app.ui.settings.confirm_delete {
        return false;
    }
    let count = app.session.targets(&json!({})).len();
    if count == 0 {
        return false;
    }
    app.ui.dialog = Some(Dialog::ConfirmDelete { count });
    true
}

/// Platform-appropriate label for revealing a file in the system file manager.
pub fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in Explorer"
    } else {
        "Show in File Manager"
    }
}

/// Reveal the active photo's original in the system file manager.
fn show_in_finder(app: &mut LightcraftApp) -> Result<Value, String> {
    let id = app.session.active().ok_or("no photo selected")?;
    let path = match app.session.catalog.photo(id).map(|p| p.source.clone()) {
        Some(lightcraft_engine::catalog::Source::File { path }) => path,
        _ => return Err("this photo has no file (demo scene)".into()),
    };
    let reveal = app.services.reveal.as_mut().ok_or("not available here")?;
    reveal(&path)?;
    Ok(json!({"path": path}))
}
