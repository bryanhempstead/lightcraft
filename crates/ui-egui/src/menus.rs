//! UI-level commands (views, panels, zoom, tools, dialogs) and the menu model shared by the native
//! menu bar, the shortcut handler and the control channel.

use serde::Serialize;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{BeforeAfter, DeleteKey, Dialog, Module, RightPanel, ViewMode, Zoom};

/// (id, label, shortcut, menu path)
pub type UiCommand = (&'static str, &'static str, Option<&'static str>, &'static str);

/// What a request to change the UI language gets back.
pub const ENGLISH_ONLY: &str = "LightCraft is English only: the interface language can't be changed";

/// Every UI command. `xtask parity` reads the table from this file, so an id listed in
/// `docs/parity.md` is checked wherever it is declared.
pub fn ui_commands() -> impl Iterator<Item = &'static UiCommand> {
    UI_COMMANDS.iter()
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
    ("view.drawFace", "Draw Face Region", None, "View"),
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
    ("view.zoomFit", "Zoom to Fit", Some("Cmd+Shift+0"), "View"),
    ("view.zoom100", "Zoom 100%", Some("Cmd+Alt+0"), "View"),
    ("view.zoomToggle", "Toggle Zoom", Some("Z"), "View"),
    // the ratio a click (and Z / Space) zooms to
    ("view.clickZoom", "Click Zoom Ratio", None, ""),
    ("view.wheelZoom", "Scroll Wheel Zoom", None, ""),
    // LRC-VIEW-LOUPEOVERLAY: View ▸ Loupe Overlay / Composition Overlay / Aspect Ratio Frames
    ("view.loupeOverlay", "Show Loupe Overlays", Some("Cmd+Alt+O"), "View"),
    ("view.overlayShow", "Show Overlays", None, ""),
    ("view.loupeGrid", "Grid", None, ""),
    ("view.loupeGuides", "Guides", None, ""),
    ("view.layoutImage", "Layout Image", None, ""),
    ("view.chooseLayoutImage", "Choose Layout Image…", None, ""),
    // O / Shift+O in the loupe (outside the crop and masking tools)
    ("view.compositionOverlay", "Cycle Composition Overlay", None, ""),
    ("view.compositionOverlayOrientation", "Cycle Composition Overlay Orientation", None, ""),
    ("view.frameGuides", "Aspect Ratio Frames", None, ""),
    ("view.navigate", "Set Image Zoom and Pan", None, ""),
    ("view.zoomIn", "Zoom In", Some("Cmd+="), "View"),
    ("view.zoomOut", "Zoom Out", Some("Cmd+-"), "View"),
    ("view.clipping", "Show Clipping", Some("J"), "View"),
    ("view.visualizeHdr", "Visualize HDR", None, "View"),
    ("view.sdrPreview", "Preview for SDR Display", None, "View"),
    // in grids S expands/collapses stacks (the engine command it shadows)
    ("view.softProof", "Soft Proofing", Some("S"), "View"),
    ("view.histogram", "Histogram", Some("Cmd+0"), "View"),
    ("view.maskOverlay", "Show Mask Overlay", Some("O"), "View"),
    // Shift+O in the Masking panel (elsewhere it cycles the crop overlay)
    ("view.maskOverlayMode", "Cycle Mask Overlay Mode", None, "View"),
    ("view.maskOverlayColor", "Cycle Mask Overlay Color", None, "View"),
    ("view.maskPins", "Show Mask Pins", None, "View"),
    ("view.visualizeSpots", "Visualize Spots", Some("A"), "View"),
    ("view.cropOverlay", "Cycle Crop Overlay", Some("Shift+O"), "View"),
    ("view.cropOverlayOrientation", "Cycle Crop Overlay Orientation", None, "View"),
    ("view.cropOverlaysToCycle", "Choose Crop Overlays to Cycle", None, ""),
    ("view.beforeAfterCycle", "Cycle Before/After Layouts", None, "View"),
    ("app.viewSpace", "Photo Color Space", None, ""),
    ("app.identityPlate", "Identity Plate", None, ""),
    ("app.accents", "Accent Colors", None, ""),
    ("app.identityPlateSetup", "Identity Plate Setup…", None, "Edit"),
    // LC-UI-FIXNOTES: notes about what isn't working, sent to the Claude session that builds LightCraft
    ("notes.toggle", "Fix Notes", Some("Cmd+Alt+Shift+N"), "Window"),
    ("notes.add", "Add Fix Note", None, ""),
    ("notes.list", "List Fix Notes", None, ""),
    ("notes.send", "Send Fix Notes", None, ""),
    ("notes.markFixed", "Mark Fix Notes Fixed", None, ""),
    ("notes.reopen", "Reopen Fix Notes", None, ""),
    ("notes.edit", "Edit Fix Note", None, ""),
    ("notes.remove", "Remove Fix Notes", None, ""),
    ("notes.select", "Pick Fix Notes", None, ""),
    ("curve.snapToGrid", "Snap to Grid", None, ""),
    ("curve.showAll", "Show All Curves", None, ""),
    ("curve.copyChannel", "Copy Point Curve Channel", None, ""),
    ("curve.pasteChannel", "Paste Point Curve Channel", None, ""),
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
    // the side panels' section order and the sections open at launch (panels::order)
    ("panel.order", "Panel Order", None, ""),
    ("panel.defaultOpen", "Open Panel by Default", None, ""),
    ("panel.resetOrder", "Reset Panel Order", None, "Window>Panels"),
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
    ("dialog.syncLightroom", "Sync from Lightroom Classic…", None, "File"),
    ("photo.editInExternal", "Edit in External Editor", Some("Cmd+Shift+E"), "Photo"),
    ("app.externalEditor", "Choose External Editor", None, ""),
    ("mask.center", "Center Mask", None, ""),
    ("remove.dust", "Find Dust Spots", None, ""),
    ("app.lenses", "My Lenses", None, ""),
    ("mask.centerDefault", "Set Center Mask as Default", None, ""),
    ("dialog.enhance", "Enhance…", Some("Ctrl+Alt+I"), "Photo"),
    // frame the Enhance / Merge previews (what dragging and the wheel do over them)
    ("enhance.view", "Frame Enhance Preview", None, ""),
    ("merge.view", "Frame Merge Preview", None, ""),
    ("dialog.mergeHdr", "HDR…", Some("Ctrl+H"), "Photo>Photo Merge"),
    ("dialog.mergePanorama", "Panorama…", Some("Ctrl+M"), "Photo>Photo Merge"),
    ("dialog.mergeHdrPanorama", "HDR Panorama…", None, "Photo>Photo Merge"),
    ("dialog.mergeFocus", "Focus Stack…", Some("Ctrl+Alt+F"), "Photo>Photo Merge"),
    ("dialog.mergeTake", "Best Take…", Some("Ctrl+Alt+T"), "Photo>Photo Merge"),
    ("dialog.merge", "Merge…", Some("Ctrl+Alt+M"), "Photo>Photo Merge"),
    ("merge.hdrLast", "HDR with Last Settings", Some("Ctrl+Shift+H"), "Photo>Photo Merge"),
    ("merge.panoramaLast", "Panorama with Last Settings", Some("Ctrl+Shift+M"), "Photo>Photo Merge"),
    ("merge.hdrPanoramaLast", "HDR Panorama with Last Settings", None, "Photo>Photo Merge"),
    ("merge.focusLast", "Focus Stack with Last Settings", None, "Photo>Photo Merge"),
    ("file.addPhotos", "Import Photos and Video…", Some("Cmd+Shift+I"), "File"),
    // the Import window's source column: browse to a folder / pick a device (`{path, subfolders?}`)
    ("import.source", "Import Source", None, ""),
    ("file.addFolder", "Import from Folder…", None, "File"),
    ("file.addFromDevice", "Import from Device", None, ""),
    ("file.findMissing", "Find Missing Photos…", None, "File"),
    ("dialog.deleteRejected", "Delete Rejected Photos…", Some("Cmd+Delete"), "Photo"),
    ("tool.painter", "Painter", None, ""),
    ("view.gridOptions", "View Options…", Some("Cmd+J"), "View"),
    // the Import window's Import Preset popup ({name})
    ("import.presets", "Import Presets", None, ""),
    ("import.savePreset", "Save Import Preset", None, ""),
    ("import.applyPreset", "Apply Import Preset", None, ""),
    ("import.deletePreset", "Delete Import Preset", None, ""),
    ("file.backupLibrary", "Back Up Library…", None, "File"),
    ("file.restoreLibrary", "Restore Library from Backup…", None, "File"),
    ("photo.locate", "Locate Missing File…", None, ""),
    ("dialog.saveMetadataPreset", "Save Metadata Preset…", None, ""),
    // Metadata ▸ keyword list (L11)
    ("dialog.importKeywords", "Import Keywords…", None, "Metadata"),
    ("dialog.importCatalog", "Import from Another Library…", None, "File"),
    // L23: Library ▸ Validate DNG Files… (the result as a toast; damaged ones selected)
    ("dialog.validateDng", "Validate DNG Files…", None, "Library"),
    // L33: File ▸ Tethered Capture
    ("dialog.tetherStart", "Start Tethered Capture…", None, "File"),
    ("dialog.tetherStop", "Stop Tethered Capture", None, "File"),
    // LRC-LIB-PUBLISH: publish the published collection on screen; LR-EXP-PHOTOS
    ("dialog.publish", "Publish Now", None, "Library"),
    ("dialog.exportToPhotos", "Export to Photos…", None, "File"),
    // E12: File ▸ Email Photos…
    ("file.emailPhotos", "Email Photos…", Some("Cmd+Shift+M"), "File"),
    // L26: Compare ▸ Link Focus
    ("compare.linkFocus", "Link Focus", None, ""),
    ("dialog.exportKeywords", "Export Keywords…", None, "Metadata"),
    ("dialog.keywordShortcut", "Set Keyword Shortcut…", Some("Cmd+Alt+Shift+K"), "Metadata"),
    ("dialog.editKeyword", "Edit Keyword Tag…", None, ""),
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
    ("view.map", "Map", Some("Cmd+Alt+3"), "Window"),
    // LC-GALLERIES: the galleries. tab (Second Brain's client galleries; engine: gallery.*)
    ("view.galleries", "Galleries", Some("Cmd+Alt+4"), "Window"),
    ("galleries.refresh", "Refresh Galleries", None, ""),
    ("galleries.open", "Open Gallery", None, ""),
    ("galleries.newFrom", "New Gallery from Photos", None, "Library"),
    ("galleries.reexport", "Update Gallery Photos", None, ""),
    ("galleries.publish", "Publish Gallery", None, ""),
    ("galleries.pick", "Pick Gallery Photos", None, ""),
    ("galleries.save", "Save Gallery Settings", None, ""),
    ("galleries.develop", "Open Gallery Photo in Develop", None, ""),
    ("galleries.showPicks", "Show Client Picks", None, ""),
    ("galleries.applyPicks", "Apply Client Picks", None, ""),
    ("galleries.copyLink", "Copy Gallery Link", None, ""),
    ("galleries.openLink", "Open Gallery Link", None, ""),
    ("galleries.hide", "Hide Gallery Photos", None, ""),
    ("galleries.slideshow", "Make Gallery Slideshow", None, ""),
    ("galleries.start", "Start Second Brain", None, ""),
    ("map.fit", "Fit Map to Photos", None, ""),
    // LR-VID-*: the Loupe's video controls (Space plays and pauses a video: shortcuts.rs)
    ("video.playPause", "Play / Pause Video", None, "Photo>Video"),
    ("video.trimMode", "Trim Video", None, "Photo>Video"),
    ("video.posterHere", "Set Poster Frame", None, "Photo>Video"),
    ("video.captureHere", "Capture Frame", None, "Photo>Video"),
    ("video.sound", "Video Sound On / Off", None, "Photo>Video"),
    ("video.seek", "Go to Time in Video", None, ""),
    ("map.zoomIn", "Zoom In on Map", None, ""),
    ("map.zoomOut", "Zoom Out of Map", None, ""),
    ("dialog.mapSaveLocation", "Save Map Location…", None, ""),
    // LR-MAP-PLACES: the Library's Place & Tags section ({q?} starts a search, {selection?: bool})
    ("view.places", "Set Location and Tags…", None, "Library"),
    ("view.colorMixer", "Color Mixer", None, ""),
    ("view.resumeLastLeftOff", "Go to Where I Left Off", None, "View"),
    ("preset.applyByName", "Apply Preset by Name", None, ""),
    ("keys.nudge", "Nudge Slider", None, ""),
    ("keys.macro", "Run Macro", None, ""),
    ("keys.send", "Send Keys", None, ""),
    ("keys.lrinput", "Monogram Lightroom Input", None, ""),
    ("keys.cycle", "Step a Cycle List", None, ""),
    ("keys.cycles", "Cycle Lists", None, ""),
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
    // (Library from the map goes back to the grid: the map is a module of its own)
    app.ui.view =
        if v == ViewMode::Reference || (view.is_none() && matches!(v, ViewMode::Map | ViewMode::Galleries)) { ViewMode::PhotoGrid } else { v };
    if !matches!(app.ui.view, ViewMode::Map | ViewMode::Galleries) {
        app.ui.library_view = app.ui.view;
    }
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
/// Does a paste carry masks onto a photo that has masks of its own?
pub fn paste_meets_masks(app: &LightcraftApp, p: &Value) -> bool {
    let copied = app.session.clipboard.as_ref().and_then(|c| c.get("masks")).and_then(Value::as_array).is_some_and(|m| !m.is_empty());
    copied && app.session.targets(p).iter().any(|id| app.session.develop_of(*id).is_some_and(|d| !d.masks.is_empty()))
}

pub fn run_ui_command(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    // The old Edit ▸ Language commands, from scripts written before LightCraft went English only.
    if id.starts_with("app.language.") {
        return Some(Err(ENGLISH_ONLY.into()));
    }
    if let Some(r) = crate::keymap::run_command(app, id, p) {
        return Some(r);
    }
    if let Some(r) = crate::panels::crop_overlay::run_command(app, id, p) {
        return Some(r);
    }
    if let Some(r) = crate::fixnotes::run_command(app, id, p) {
        return Some(r);
    }
    if let Some(r) = crate::panels::galleries::run_command(app, id, p) {
        return Some(r);
    }
    if let Some(r) = crate::panels::order::run_command(app, id, p) {
        return Some(r);
    }
    let ctx = egui::Context::default();
    let r: Result<Value, String> = match id {
        // Lightroom asks whether copied masks merge with the photos' own or replace them
        "develop.paste" if p.get("maskMode").is_none() && paste_meets_masks(app, p) => {
            app.ui.dialog = Some(Dialog::PasteMasks { params: p.clone(), merge: true });
            Ok(json!({"asked": "maskMode"}))
        }
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
                app.toast_soon(name.clone(), 1.4);
            }
            r
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
        "tool.painter" => {
            // {mode?: keyword|label|flag|rating|preset|metadata|rotate|album, value}: Lightroom's
            // Painter; no mode stops it
            let mode = p.get("mode").and_then(Value::as_str).unwrap_or("").to_string();
            let value = match p.get("value") {
                Some(Value::String(v)) => v.trim().to_string(),
                Some(Value::Number(n)) => n.to_string(),
                _ => String::new(),
            };
            app.ui.keyword_painter = None;
            app.ui.painter = None;
            if mode.is_empty() {
                return Some(Ok(json!({"painter": null})));
            }
            if !["keyword", "label", "flag", "rating", "preset", "metadata", "rotate", "album"].contains(&mode.as_str()) {
                return Some(Err(format!("unknown painter mode `{mode}` (keyword|label|flag|rating|preset|metadata|rotate|album)")));
            }
            if value.is_empty() {
                return Some(Err("the painter needs a `value` (a label, flag, rating, preset, direction or album)".into()));
            }
            if mode == "keyword" {
                app.ui.keyword_painter = Some(value.clone());
            } else {
                app.ui.painter = Some((mode.clone(), value.clone()));
            }
            if !matches!(app.ui.view, ViewMode::PhotoGrid | ViewMode::SquareGrid) {
                app.ui.view = ViewMode::PhotoGrid;
            }
            app.toast(&ctx, format!("Painting {mode} “{value}”: click photos · Esc stops"));
            Ok(json!({"painter": {"mode": mode, "value": value}}))
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
            if !app.ui.face_boxes {
                app.ui.draw_face = false;
            }
            Ok(json!({"show": app.ui.face_boxes}))
        }
        // Lightroom's Draw Face Region tool (the loupe): drag on the photo to draw a face box, then type the name;
        // with the tool on, boxes move by dragging them
        "view.drawFace" => {
            let on = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.draw_face);
            if on {
                if app.session.active().is_none() {
                    return Some(Err("no photo to draw a face on".into()));
                }
                if app.ui.view != ViewMode::Detail {
                    enter_library(app, Some(ViewMode::Detail));
                    app.ui.view = ViewMode::Detail;
                    app.ui.library_view = ViewMode::Detail;
                }
                app.ui.face_boxes = true;
            }
            app.ui.draw_face = on;
            Ok(json!({"on": on}))
        }
        "view.map" => {
            // Lightroom Classic's Map module (⌘⌥3): {lat?, lon?, zoom?} looks there
            enter_library(app, Some(ViewMode::Map));
            if let (Some(la), Some(lo)) = (p.get("lat").and_then(Value::as_f64), p.get("lon").and_then(Value::as_f64))
                && lightcraft_engine::map::valid(la, lo)
            {
                (app.ui.map.lat, app.ui.map.lon, app.ui.map.fit_pending) = (la, lo, false);
            }
            if let Some(z) = p.get("zoom").and_then(Value::as_f64).filter(|z| z.is_finite()) {
                app.ui.map.zoom = z.clamp(1.0, lightcraft_engine::map::MAX_ZOOM);
                app.ui.map.fit_pending = false;
            }
            Ok(json!({"view": "map", "lat": app.ui.map.lat, "lon": app.ui.map.lon, "zoom": app.ui.map.zoom, "tiles": app.map.status()}))
        }
        "video.playPause" | "video.seek" | "video.trimMode" | "video.sound" | "video.posterHere" | "video.captureHere" => {
            crate::video_ui::command(app, id, p)
        }
        "map.fit" => {
            app.ui.map.fit_pending = true;
            Ok(Value::Null)
        }
        "map.zoomIn" | "map.zoomOut" => {
            let d = if id == "map.zoomIn" { 1.0 } else { -1.0 };
            app.ui.map.zoom = (app.ui.map.zoom.round() + d).clamp(1.0, lightcraft_engine::map::MAX_ZOOM);
            Ok(json!({"zoom": app.ui.map.zoom}))
        }
        "view.places" => {
            if app.ui.module != Module::Library || matches!(app.ui.view, ViewMode::Map | ViewMode::Galleries) {
                let _ = app.run("view.library", json!({}));
            }
            crate::panels::places::open(app, p)
        }
        "dialog.mapSaveLocation" => {
            // name the place at the map's centre (radius: a sixth of the view's width)
            let (lat, lon) = (app.ui.map.lat, app.ui.map.lon);
            let radius = crate::panels::map::default_radius(app);
            crate::panels::dialogs::prompt(
                app,
                "Save Location",
                "Name",
                "",
                "map.saveLocation",
                json!({"lat": lat, "lon": lon, "radius": radius}),
                "name",
            );
            Ok(Value::Null)
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
            } else if app.ui.keyword_painter.is_some() || app.ui.painter.is_some() {
                app.ui.keyword_painter = None;
                app.ui.painter = None;
            } else if app.ui.fullscreen {
                app.ui.fullscreen = false;
                app.ui.slideshow = None;
            } else if !app.ui.tool.is_empty() {
                app.ui.tool.clear();
            } else if app.ui.lights_out != crate::state::LightsOut::On {
                app.ui.lights_out = crate::state::LightsOut::On;
            } else if app.ui.module == Module::Develop {
                // Esc in Develop lets go of what's selected — it never leaves Develop (Bryan,
                // 2026-11-23: "don't just bring me to Library, just deselect what's selected"):
                // the selected mask / spot, then a multi-photo selection down to the photo shown
                if app.session.active_mask.is_some() || app.session.active_spot.is_some() {
                    app.session.active_mask = None;
                    app.session.active_spot = None;
                } else if app.session.selection.ids.len() > 1
                    && let Some(a) = app.session.active()
                {
                    let _ = app.run("library.select", json!({"ids": [a.0]}));
                }
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
            // the current module's overlay (Library and Develop each keep their own)
            let mode = match p.get("mode").and_then(Value::as_str) {
                Some(m) => match serde_json::from_value(json!(m)) {
                    Ok(v) => v,
                    Err(_) => return Some(Err(format!("unknown info overlay `{m}` (off|basic|exposure)"))),
                },
                None => app.ui.info_mode().next(),
            };
            *app.ui.info_mode_mut() = mode;
            let label = match mode {
                crate::state::InfoOverlay::Off => "Info Overlay Off",
                crate::state::InfoOverlay::Basic => "Info Overlay: File",
                crate::state::InfoOverlay::Exposure => "Info Overlay: Exposure",
            };
            app.toast(&ctx, label);
            Ok(json!({"infoOverlay": mode, "module": app.ui.module}))
        }
        "view.navigator" => {
            app.ui.navigator = !app.ui.navigator;
            Ok(json!({"navigator": app.ui.navigator}))
        }
        "app.settings" => {
            let tab = p.get("tab").and_then(Value::as_str).unwrap_or("general");
            if !crate::panels::settings::TABS.iter().any(|(id, _)| *id == tab) {
                return Some(Err(format!(
                    "unknown settings tab `{tab}` (general|import|performance|backup|interface|identity|develop|shortcuts|controllers|cameras)"
                )));
            }
            app.ui.dialog = Some(Dialog::Settings { tab: tab.into() });
            Ok(Value::Null)
        }
        "app.openLibrary" => crate::panels::settings::open_library(app, p),
        "dialog.deleteRejected" => delete_rejected(app),
        // Lightroom's View Options (⌘J): the grid's and Library loupe's options live on Settings ▸
        // Interface; in Develop, Develop's own View Options (LRC-DEV-VIEWOPTIONS)
        "view.gridOptions" => {
            let tab = if app.ui.module == Module::Develop { "develop" } else { "interface" };
            app.ui.dialog = Some(Dialog::Settings { tab: tab.into() });
            Ok(Value::Null)
        }
        "import.presets" | "import.savePreset" | "import.applyPreset" | "import.deletePreset" => crate::import::preset_command(app, id, p),
        "file.backupLibrary" | "file.restoreLibrary" => {
            let action = if id == "file.backupLibrary" { app.services.backup_library.as_mut() } else { app.services.restore_library.as_mut() };
            match action {
                Some(f) => f(&mut app.session),
                // a library folder on disk: dated catalog backups (L02)
                None if app.session.backup_root().is_some() => {
                    if id == "file.backupLibrary" {
                        app.session.execute("library.backup", &json!({})).map_err(|e| e.to_string())
                    } else {
                        app.ui.dialog = Some(Dialog::Backups { pick: None });
                        Ok(Value::Null)
                    }
                }
                None => Err("not available here: this library isn't a folder on disk".into()),
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
            // the same level a click on the photo zooms to
            app.ui.zoom = if app.ui.zoom == Zoom::Fit { crate::state::click_zoom_target(app.ui.click_zoom) } else { Zoom::Fit };
            app.ui.zoom_anim = true;
            Ok(Value::Null)
        }
        "view.clickZoom" => {
            // {ratio?: "fill" | 0.25|0.5|1|2|3|4|8|11|16, percent?: 1..1600} → {ratio, label}: the
            // level a click (Z, Space) zooms to, kept until another is chosen (saved with the UI)
            let pct = match (p.get("ratio"), p.get("percent").and_then(Value::as_f64)) {
                (Some(Value::String(s)), _) if s.eq_ignore_ascii_case("fill") => Some(0),
                (Some(v), _) => {
                    let pct = v.as_f64().filter(|r| r.is_finite() && *r > 0.0 && *r <= 16.0).map(|r| (r * 100.0).round() as u32);
                    match pct.filter(|c| crate::state::CLICK_ZOOMS.contains(c)) {
                        Some(c) => Some(c),
                        None => return Some(Err(format!("view.clickZoom: ratio {v} (\"fill\", 0.25, 0.5, 1, 2, 3, 4, 8, 11 or 16)"))),
                    }
                }
                (None, Some(pct)) if pct.is_finite() && (1.0..=1600.0).contains(&pct.round()) => Some(pct.round() as u32),
                (None, Some(pct)) => return Some(Err(format!("view.clickZoom: percent {pct} (1 to 1600)"))),
                (None, None) => None,
            };
            if let Some(pct) = pct {
                app.ui.click_zoom = pct;
            }
            let c = app.ui.click_zoom;
            let ratio = if c == 0 {
                json!("fill")
            } else if c.is_multiple_of(100) {
                json!(c / 100)
            } else {
                json!(c as f64 / 100.0)
            };
            Ok(json!({"ratio": ratio, "label": crate::state::click_zoom_label(c)}))
        }
        "view.wheelZoom" => {
            // {mode?: auto|zoom|pan, step?: 10|25|50|100} → {mode, step}: what the plain scroll
            // wheel / trackpad scroll does over the photo and how far one notch zooms (saved)
            if let Some(m) = p.get("mode") {
                match serde_json::from_value::<crate::state::WheelMode>(m.clone()) {
                    Ok(m) => app.ui.wheel = m,
                    Err(_) => return Some(Err(format!("view.wheelZoom: mode {m} (auto, zoom, pan or photos)"))),
                }
            }
            if let Some(s) = p.get("step") {
                match s.as_u64().map(|v| v as u32).filter(|v| crate::state::WHEEL_STEPS.contains(v)) {
                    Some(v) => app.ui.wheel_step = v,
                    None => return Some(Err(format!("view.wheelZoom: step {s} (10, 25, 50 or 100 percent)"))),
                }
            }
            Ok(json!({"mode": app.ui.wheel, "step": app.ui.wheel_step}))
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
                    Ok(Zoom::Percent(p)) if !p.is_finite() || p <= 0.0 || p > 1600.0 => {
                        return Some(Err("view.navigate: zoom percent must be greater than 0 and at most 1600".into()));
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
            // {remember: true} (the toolbar's zoom menu, the Navigator): a level picked there is
            // what a click zooms to from now on, as in Lightroom
            if p.get("remember").and_then(Value::as_bool) == Some(true) {
                match zoom {
                    Zoom::Fill => app.ui.click_zoom = 0,
                    Zoom::Percent(z) if z.round() >= 1.0 => app.ui.click_zoom = z.round().min(1600.0) as u32,
                    _ => {}
                }
            }
            Ok(json!({"zoom": zoom, "pan": pan, "clickZoom": app.ui.click_zoom}))
        }
        "view.clipping" => {
            app.ui.show_clipping = !app.ui.show_clipping;
            Ok(Value::Null)
        }
        "view.visualizeHdr" => {
            // {on?}; no params toggles. Shown on photos edited in HDR (see `develop.hdr`).
            app.ui.hdr_visualize = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.hdr_visualize);
            Ok(json!({"on": app.ui.hdr_visualize}))
        }
        "view.sdrPreview" => {
            // {on?}; no params toggles. Photos edited in HDR show their SDR rendition.
            app.ui.hdr_sdr_preview = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.hdr_sdr_preview);
            Ok(json!({"on": app.ui.hdr_sdr_preview}))
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
            // {overlay?: thirds|grid|golden|diagonal|triangle|spiral|aspectRatios|center|fifths|none}:
            // that one, else the next of the overlays chosen to cycle
            match p.get("overlay") {
                Some(v) => match serde_json::from_value::<crate::state::CropOverlay>(v.clone()) {
                    Ok(o) => {
                        app.ui.crop_overlay = o;
                        Ok(json!({"overlay": app.ui.crop_overlay}))
                    }
                    Err(e) => Err(format!("view.cropOverlay: `overlay`: {e}")),
                },
                None => {
                    app.ui.crop_overlay = app.ui.crop_overlay.next_in(&app.ui.crop_overlay_cycle);
                    Ok(json!({"overlay": app.ui.crop_overlay}))
                }
            }
        }
        "view.cropOverlaysToCycle" => {
            // {overlays: [id]} — Lightroom's Choose Overlays to Cycle (empty: all)
            match p.get("overlays").map(|v| serde_json::from_value::<Vec<crate::state::CropOverlay>>(v.clone())) {
                Some(Ok(list)) => {
                    app.ui.crop_overlay_cycle = crate::state::CropOverlay::ALL.into_iter().filter(|o| list.contains(o)).collect();
                    Ok(json!({"overlays": app.ui.crop_overlay_cycle}))
                }
                Some(Err(e)) => Err(format!("view.cropOverlaysToCycle: `overlays`: {e}")),
                None => Ok(json!({"overlays": app.ui.crop_overlay_cycle})),
            }
        }
        "curve.snapToGrid" => {
            app.ui.curve_snap = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.curve_snap);
            Ok(json!({"snap": app.ui.curve_snap}))
        }
        "curve.showAll" => {
            app.ui.curve_show_all = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.curve_show_all);
            Ok(json!({"showAll": app.ui.curve_show_all}))
        }
        "curve.copyChannel" | "curve.pasteChannel" => {
            // {channel?: master|red|green|blue (default: the one shown)}
            let ch = p.get("channel").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| app.ui.curve_channel.clone());
            if !["master", "red", "green", "blue"].contains(&ch.as_str()) {
                return Some(Err(format!("{id}: `channel` is master, red, green or blue")));
            }
            if id == "curve.copyChannel" {
                let d = app.session.active().and_then(|x| app.session.develop_of(x)).unwrap_or_default();
                let pts = match ch.as_str() {
                    "red" => &d.curve.red,
                    "green" => &d.curve.green,
                    "blue" => &d.curve.blue,
                    _ => &d.curve.master,
                };
                let pts: Vec<[f64; 2]> = pts.iter().map(|q| [q.x, q.y]).collect();
                app.ui.curve_clipboard = Some(pts.clone());
                Ok(json!({"channel": ch, "points": pts}))
            } else {
                match app.ui.curve_clipboard.clone() {
                    Some(pts) => app.run("develop.curve", json!({"channel": ch, "points": pts})),
                    None => Err("curve.pasteChannel: no point curve copied".into()),
                }
            }
        }
        "app.identityPlate" => crate::identity::command(app, p),
        "app.accents" => crate::accents::command(app, p),
        "app.identityPlateSetup" => {
            // Lightroom Classic's Edit ▸ Identity Plate Setup…: Settings ▸ identity.
            app.ui.dialog = Some(Dialog::Settings { tab: "identity".into() });
            Ok(Value::Null)
        }
        "app.viewSpace" => {
            // {space?: auto|displayP3|srgb} — the colour space photos are shown in
            match p.get("space") {
                Some(v) => match serde_json::from_value::<crate::display_p3::ViewSpace>(v.clone()) {
                    Ok(space) => {
                        app.ui.settings.view_space = space;
                        Ok(view_space_state(app))
                    }
                    Err(e) => Err(format!("app.viewSpace: `space`: {e}")),
                },
                None => Ok(view_space_state(app)),
            }
        }
        "view.beforeAfterCycle" => {
            // the toolbar's Before & After button: the next layout (Left / Right → Top / Bottom →
            // Left / Right Split → Top / Bottom Split → …), as Lightroom's button cycles them
            app.ui.before_after = match app.ui.before_after {
                BeforeAfter::SideBySide => BeforeAfter::TopBottom,
                BeforeAfter::TopBottom => BeforeAfter::Split,
                BeforeAfter::Split => BeforeAfter::SplitTopBottom,
                _ => BeforeAfter::SideBySide,
            };
            Ok(json!({"layout": app.ui.before_after}))
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
                // W again puts it away (Bryan, 2026-11-23)
                "wbPicker" if app.ui.tool == "wbPicker" => app.ui.tool.clear(),
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
            app.ui.dialog = Some(crate::capture_time_dialog::open(app, p));
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
            let text = p.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            app.ui.dialog = Some(Dialog::Rename { template, start: p.get("start").and_then(Value::as_u64).unwrap_or(1) as u32, text });
            Ok(Value::Null)
        }
        "dialog.createPreset" => {
            app.ui.dialog = Some(Dialog::create_preset());
            Ok(Value::Null)
        }
        "dialog.pasteSettings" => {
            let groups = crate::state::group_ids(&app.session.copy_groups);
            app.ui.dialog = Some(Dialog::PasteSettings { groups, merge_masks: false });
            Ok(Value::Null)
        }
        "view.focusSearch" => {
            // the search field lives in the top bar of the library and detail views alike
            app.ui.focus_search = true;
            Ok(Value::Null)
        }
        "dialog.syncSettings" => {
            let groups = crate::state::default_preset_groups();
            // what was checked last time (Match exposure / Auto align / Match camera & lens color start off)
            let prefs = app.ui.sync_prefs.clone();
            let groups = prefs.groups.map(|g| crate::state::normalize_groups(&g)).unwrap_or(groups);
            app.ui.dialog = Some(Dialog::SyncSettings {
                groups,
                match_exposure: prefs.match_exposure,
                auto_align: prefs.auto_align,
                match_color: prefs.match_color,
                masks: None,
            });
            Ok(Value::Null)
        }
        "dialog.copySettings" => {
            let groups = crate::state::group_ids(&app.session.copy_groups);
            app.ui.dialog = Some(Dialog::CopySettings { groups, masks: None, mode: "exact".into(), step: 0 });
            Ok(Value::Null)
        }
        // File ▸ Sync from Lightroom Classic…: the catalog is read in the background (lr_sync.rs)
        "dialog.syncLightroom" if !cfg!(target_arch = "wasm32") => crate::lr_sync::start(app, p),
        "dialog.export" => {
            let prev = app.session.last_export.clone().unwrap_or_default();
            let u = |k: &str, d: u64| prev.get(k).and_then(Value::as_u64).unwrap_or(d);
            let dir = prev.get("dir").and_then(Value::as_str).map(str::to_string).unwrap_or_else(crate::control::default_export_dir);
            let opts = lightcraft_engine::export::ExportOptions::from_json(&prev);
            // no previous export: 2048 px long edge; a previous full-size export: full size
            let full_size = opts.resize.is_none() && lightcraft_engine::export::ExportOptions::has_size_param(&prev);
            let resize = opts.resize.unwrap_or_default();
            app.ui.dialog = Some(Dialog::Export {
                opts: Box::new(opts),
                full_size,
                resize,
                preset_name: String::new(),
                limit_kb: u("limitKb", 0) as u32,
                dir,
                selected: None,
                checked: Vec::new(),
                album: "LightCraft".into(),
            });
            Ok(Value::Null)
        }
        "merge.hdrLast" => crate::merge::start_last(app, "merge.hdr"),
        "merge.panoramaLast" => crate::merge::start_last(app, "merge.panorama"),
        "merge.hdrPanoramaLast" => crate::merge::start_last(app, "merge.hdrPanorama"),
        "enhance.view" => crate::enhance::set_view(app, p),
        "merge.view" => crate::merge::set_view(app, p),
        "dialog.mergeHdr" => crate::merge::open(app, "merge.hdr"),
        "dialog.mergePanorama" => crate::merge::open(app, "merge.panorama"),
        "dialog.mergeHdrPanorama" => crate::merge::open(app, "merge.hdrPanorama"),
        "dialog.mergeFocus" => crate::merge::open(app, "merge.focus"),
        "dialog.mergeTake" => crate::merge::open(app, "merge.take"),
        "dialog.enhance" => crate::enhance::open(app),
        "dialog.merge" => crate::merge::open(app, "merge"),
        "merge.focusLast" => crate::merge::start_last(app, "merge.focus"),
        "app.about" => {
            app.ui.dialog = Some(Dialog::About);
            Ok(Value::Null)
        }
        "remove.dust" => {
            // dust.: the Remove tool, with the sensor dust it finds listed to apply (all. / one by one)
            let _ = app.run("panel.remove", json!({}));
            app.run("remove.detect", json!({"people": false, "wires": false, "dust": true}))
        }
        "app.lenses" => {
            // {lenses?: [{name, focal}]} → the list Set Lens offers
            if let Some(l) = p.get("lenses") {
                match serde_json::from_value::<Vec<crate::state::LensPreset>>(l.clone()) {
                    Ok(v) => app.ui.settings.lenses = v.into_iter().filter(|x| !x.name.trim().is_empty()).collect(),
                    Err(e) => return Some(Err(format!("app.lenses: {e}"))),
                }
            }
            Ok(serde_json::to_value(&app.ui.settings.lenses).unwrap_or_default())
        }
        "mask.center" => (|| {
            // center.: a radial mask at the photo's centre (the saved default's shape and effect),
            // or the photo's Center mask when it has one — then Masking, to move / size / adjust it
            let id = app.session.active().ok_or("no photo selected")?;
            let existing = app.session.develop_of(id).and_then(|d| d.masks.iter().find(|m| m.name == CENTER_MASK).map(|m| m.id));
            let out = if let Some(mid) = existing {
                app.run("mask.select", json!({"id": mid}))?;
                json!({"mask": mid, "existing": true})
            } else {
                let t = app.ui.settings.center_mask.clone().unwrap_or_else(default_center_mask);
                app.run("mask.add", json!({"shape": t["shape"], "name": CENTER_MASK}))?;
                if let Some(adj) = t.get("adjust").filter(|a| a.is_object()) {
                    app.run("mask.adjust", json!({"values": adj}))?;
                }
                json!({"mask": app.session.active_mask, "existing": false})
            };
            let _ = app.run("panel.masking", json!({}));
            Ok(out)
        })(),
        "mask.centerDefault" => (|| {
            // default.: the photo's Center mask (or the selected mask) becomes what center. makes
            let id = app.session.active().ok_or("no photo selected")?;
            let d = app.session.develop_of(id).ok_or("no photo")?;
            let m = d
                .masks
                .iter()
                .find(|m| Some(m.id) == app.session.active_mask && m.name == CENTER_MASK)
                .or_else(|| d.masks.iter().find(|m| m.name == CENTER_MASK))
                .ok_or("this photo has no Center mask (center. makes one)")?;
            let shape = m.components.first().map(|c| serde_json::to_value(&c.shape).unwrap_or_default()).ok_or("the Center mask has no shape")?;
            let t = json!({"shape": shape, "adjust": serde_json::to_value(m.adjust).unwrap_or_default()});
            app.ui.settings.center_mask = Some(t.clone());
            app.toast(&ctx, "center. will make this mask from now on");
            Ok(t)
        })(),
        "app.externalEditor" => {
            // {app: "PhotoCraft" | "Adobe Photoshop 2026" | any app name | "" (the system default)}
            // {app?, format?: psd|tiff, holdGrain?: bool}
            if let Some(a) = p.get("app").and_then(Value::as_str) {
                app.ui.settings.external_editor = a.trim().to_string();
            }
            if let Some(f) = p.get("format").and_then(Value::as_str) {
                if !matches!(f, "psd" | "tiff") {
                    return Some(Err(format!("app.externalEditor: format {f} (psd or tiff)")));
                }
                app.ui.settings.external_format = f.into();
            }
            if let Some(g) = p.get("holdGrain").and_then(Value::as_bool) {
                app.ui.settings.external_hold_grain = g;
            }
            let s = &app.ui.settings;
            Ok(json!({"app": s.external_editor, "format": s.external_format, "holdGrain": s.external_hold_grain}))
        }
        "photo.editInExternal" => {
            // render an edit copy (stacked on the original), then open it in the editor
            let mut params = p.clone();
            if !params.is_object() {
                params = json!({});
            }
            // the copy's format and the grain rule from Settings, unless the call says otherwise
            if params.get("format").is_none() {
                params["format"] = json!(app.ui.settings.external_format);
            }
            if params.get("holdGrain").is_none() {
                params["holdGrain"] = json!(app.ui.settings.external_hold_grain);
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
            app.ui.dialog =
                Some(Dialog::Cull { reject_below: 0.0, pick_best: true, reject_eyes: false, reject_face_below: 0.0, reject_expression_below: 0.0 });
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
        "dialog.tetherStart" => {
            let folder = match p.get("folder").and_then(Value::as_str) {
                Some(f) => Some(f.to_string()),
                None => app.services.pick_folder.as_mut().and_then(|f| f()),
            };
            let Some(folder) = folder else { return Some(Ok(Value::Null)) };
            let session = p
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("Tethered {}", (app.session.clock)().get(..10).unwrap_or("")));
            let r = app.run("tether.start", json!({"folder": folder, "session": session}));
            if let Ok(v) = &r {
                let how = if v["gphoto2"] == true { "gphoto2 is capturing from the camera" } else { "save shots into it from the camera's app" };
                app.toast_soon(format!("Tethered: {} — {how}", v["folder"].as_str().unwrap_or("")), 8.0);
            }
            r
        }
        "dialog.publish" => crate::publish_ui::start_publish(app, p),
        "dialog.exportToPhotos" => crate::publish_ui::start_photos(app, p),
        "dialog.tetherStop" => {
            let r = app.run("tether.stop", json!({}));
            if r.is_ok() {
                app.toast_soon("Tethered capture stopped", 4.0);
            }
            r
        }
        "dialog.validateDng" => {
            let r = match app.run("library.validateDng", p.clone()) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };
            let n = |k: &str| r[k].as_array().map_or(0, Vec::len);
            let msg = format!(
                "Checked {} DNG{}: {} valid, {} damaged, {} without a digest, {} not readable",
                r["checked"],
                if r["checked"] == 1 { "" } else { "s" },
                r["valid"],
                n("damaged"),
                n("noDigest"),
                n("failed")
            );
            if n("damaged") > 0 {
                let _ = app.run("library.select", json!({"ids": r["damaged"]}));
            }
            app.toast_soon(msg, 8.0);
            Ok(r)
        }
        "file.emailPhotos" => {
            // export the selection small (JPEG, 1600 px, quality 80) into a temporary folder, then
            // a new mail message with them attached — nothing is sent
            if app.services.compose_email.is_none() {
                return Some(Err("Email Photos needs the desktop app (Mail)".into()));
            }
            let dir = std::env::temp_dir().join(format!("LightCraft Email {}", (app.session.clock)().replace([':', 'T'], "-")));
            // the Export dialog's Export To ▸ Email passes its own `settings`
            let mut q = match p.get("settings").filter(|s| s.is_object()) {
                Some(s) => s.clone(),
                None => {
                    json!({"format": "jpeg", "longEdge": p.get("longEdge").and_then(Value::as_u64).unwrap_or(1600), "quality": 80, "metadata": "copyright"})
                }
            };
            if let Some(o) = q.as_object_mut() {
                o.remove("service");
                o.insert("exportTo".into(), json!("specificFolder"));
                o.insert("after".into(), json!("nothing"));
                o.insert("addToCatalog".into(), json!(false));
            }
            q["dir"] = json!(dir.to_string_lossy());
            q["background"] = json!(false);
            if let Some(ids) = p.get("ids") {
                q["ids"] = ids.clone();
            }
            let r = match app.run("app.export", q) {
                Ok(r) => r,
                Err(e) => return Some(Err(e)),
            };
            let files: Vec<String> =
                r["files"].as_array().into_iter().flatten().filter_map(|f| f["path"].as_str().or(f.as_str())).map(str::to_string).collect();
            if files.is_empty() {
                return Some(Err("nothing was exported to attach".into()));
            }
            let sent = app.services.compose_email.as_mut().map(|f| f(&files));
            match sent {
                Some(Err(e)) => Err(e),
                _ => Ok(json!({"attached": files})),
            }
        }
        "compare.linkFocus" => {
            app.ui.compare_frozen = match (p.get("on").and_then(Value::as_bool), app.ui.compare_frozen) {
                (Some(true), _) | (None, Some(_)) => None,
                (Some(false), Some(f)) => Some(f),
                (Some(false), None) | (None, None) => Some((app.ui.zoom, app.ui.pan)),
            };
            Ok(json!({"linked": app.ui.compare_frozen.is_none()}))
        }
        "dialog.importCatalog" => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.pick_folder.as_mut().and_then(|f| f()),
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            match app.run("library.importCatalog", json!({"path": path, "dryRun": true})) {
                Ok(preview) => {
                    app.ui.dialog = Some(crate::state::Dialog::ImportCatalog { path, preview, replace: "none".into() });
                    Ok(Value::Null)
                }
                Err(e) => Err(e),
            }
        }
        "dialog.importKeywords" => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.pick_text_file.as_mut().and_then(|f| f().into_iter().next()),
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            let r = app.run("keyword.import", json!({"path": path}));
            if let Ok(v) = &r {
                app.toast_soon(format!("Imported keywords: {} added, {} updated", v["added"], v["updated"]), 4.0);
            }
            r
        }
        "dialog.exportKeywords" => {
            let path = match p.get("path").and_then(Value::as_str) {
                Some(x) => Some(x.to_string()),
                None => app.services.save_text_file.as_mut().and_then(|f| f("Keywords.txt")),
            };
            let Some(path) = path else { return Some(Ok(Value::Null)) };
            let r = app.run("keyword.export", json!({"path": path}));
            if let Ok(v) = &r {
                app.toast_soon(format!("Exported {} keywords", v["keywords"]), 4.0);
            }
            r
        }
        "dialog.keywordShortcut" => {
            let now = app.session.execute("keyword.shortcut", &json!({})).ok().and_then(|v| v["keywords"].as_array().cloned()).unwrap_or_default();
            let text: Vec<&str> = now.iter().filter_map(Value::as_str).collect();
            crate::panels::dialogs::prompt(
                app,
                "Set Keyword Shortcut",
                "Keywords (commas between them)",
                &text.join(", "),
                "keyword.setShortcut",
                json!({}),
                "keywords",
            );
            Ok(Value::Null)
        }
        "dialog.editKeyword" => {
            let Some(k) = p.get("keyword").and_then(Value::as_str) else { return Some(Err("dialog.editKeyword: missing `keyword`".into())) };
            let info = match app.session.execute("keyword.info", &json!({"keyword": k})) {
                Ok(v) => v,
                Err(e) => return Some(Err(e.to_string())),
            };
            app.ui.dialog = Some(crate::state::Dialog::EditKeyword(crate::state::KeywordEdit::from_info(&info)));
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
                    l.ends_with(".cube") || l.ends_with(".dcp") || l.ends_with(".zip") || std::path::Path::new(p.as_str()).is_dir()
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
            let preset_paths: Vec<&String> = paths
                .iter()
                .filter(|p| {
                    let l = p.to_ascii_lowercase();
                    !l.ends_with(".cube") && !l.ends_with(".dcp")
                })
                .collect();
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
        s if s.starts_with("panel.") || s.starts_with("tool.") || s.starts_with("section.") => {
            app.session.active().is_some() || s == "panel.close" || crate::panels::order::COMMANDS.contains(&s)
        }
        "dialog.syncSettings" => app.session.selection.ids.len() > 1,
        "dialog.publish" => matches!(app.session.source, lightcraft_engine::LibrarySource::Published(_)),
        "dialog.exportToPhotos" => app.session.active().is_some() && lightcraft_engine::apple_photos::available(),
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

/// The Delete key and Photo ▸ Delete Photo: what Settings ▸ General ▸ Delete key says. By default
/// straight to the Trash with no dialog (⌘Z, or the toast's undo., brings it back); or Recently
/// Deleted, the album only, or Lightroom's question; with Confirm first on, a confirmation before
/// it. True when it handled the delete (with nothing selected: false, nothing happens).
pub fn delete_key(app: &mut LightcraftApp) -> bool {
    let targets = app.session.targets(&json!({}));
    if targets.is_empty() {
        return false;
    }
    if app.ui.settings.delete_key == DeleteKey::Ask {
        let copies = targets.iter().filter(|id| app.session.catalog.photo(**id).is_some_and(|p| p.copy_of.is_some())).count();
        app.ui.dialog = Some(Dialog::DeletePhotos { count: targets.len(), copies, rejected: false });
        return true;
    }
    if app.ui.settings.confirm_delete {
        app.ui.dialog = Some(Dialog::ConfirmDelete { count: targets.len(), rejected: false });
        return true;
    }
    if let Err(e) = delete_now(app, false) {
        app.toast_soon(e, 6.0);
    }
    true
}

/// Photo ▸ Delete Rejected Photos (⌘⌫): the view's rejects go the way the Delete key setting says
/// (asking first with ask. or Confirm first).
fn delete_rejected(app: &mut LightcraftApp) -> Result<Value, String> {
    let r = app.session.execute("library.rejected", &json!({})).map_err(|e| e.to_string())?;
    let count = r["count"].as_u64().unwrap_or(0) as usize;
    if count == 0 {
        return Err(crate::i18n::tr("There are no rejected photos here.").into());
    }
    if app.ui.settings.delete_key == DeleteKey::Ask {
        let copies = r["virtualCopies"].as_u64().unwrap_or(0) as usize;
        app.ui.dialog = Some(Dialog::DeletePhotos { count, copies, rejected: true });
        return Ok(json!({"count": count}));
    }
    if app.ui.settings.confirm_delete {
        app.ui.dialog = Some(Dialog::ConfirmDelete { count, rejected: true });
        return Ok(json!({"count": count}));
    }
    delete_now(app, true)
}

/// "1 photo" / "3 photos".
fn photos(n: u64) -> String {
    if n == 1 { crate::i18n::tr("1 photo").to_string() } else { crate::i18n::tr_format!("{n} photos", n = n) }
}

/// Delete the selected photos (or, `rejected`, the view's rejects) the way Settings ▸ Delete key
/// says, without asking, and say what happened in a toast with an undo. button. Files that can't go
/// to the Trash keep their photos and the toast says why (an error only when nothing went).
pub fn delete_now(app: &mut LightcraftApp, rejected: bool) -> Result<Value, String> {
    let ids: Vec<u64> = if rejected {
        let r = app.session.execute("library.rejected", &json!({})).map_err(|e| e.to_string())?;
        r["ids"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
    } else {
        app.session.targets(&json!({})).iter().map(|id| id.0).collect()
    };
    if ids.is_empty() {
        return Err(crate::i18n::tr("No photos selected.").into());
    }
    if rejected {
        // the rejects are what leaves (and the commands act on a selection)
        let picked: Vec<lightcraft_catalog::PhotoId> = ids.iter().map(|i| lightcraft_catalog::PhotoId(*i)).collect();
        app.session.selection = lightcraft_engine::Selection { active: picked.first().copied(), ids: picked };
    }
    let n = ids.len() as u64;
    let (r, text) = match app.ui.settings.delete_key {
        DeleteKey::Trash | DeleteKey::Ask => {
            let r = if rejected {
                app.run("library.deleteRejected", json!({"fromDisk": true}))?
            } else {
                app.run("photo.deleteFromDisk", json!({"ids": ids}))?
            };
            let text = trash_message(&r);
            (r, text)
        }
        DeleteKey::Recent => {
            let r = app.run("photo.delete", json!({"ids": ids}))?;
            (r, crate::i18n::tr_format!("{what} moved to Recently Deleted", what = photos(n)))
        }
        DeleteKey::Album => {
            let r = app.run("photo.removeFromAlbum", json!({"ids": ids}))?;
            let gone = r["removed"].as_u64().unwrap_or(n);
            let text = if r["fromLibrary"] == true {
                let mut t = crate::i18n::tr_format!("{what} removed from the library (files kept)", what = photos(gone));
                if !app.ui.settings.album_fallback_told {
                    app.ui.settings.album_fallback_told = true;
                    t.push_str(crate::i18n::tr(" — not in an album, so Delete removes from the library"));
                }
                t
            } else {
                crate::i18n::tr_format!(
                    "{what} removed from {name}",
                    what = photos(gone),
                    name = r["name"].as_str().unwrap_or(crate::i18n::tr("the album"))
                )
            };
            (r, text)
        }
    };
    delete_toast(app, &r, text);
    Ok(r)
}

/// What a Delete from Disk did, for its toast: "3 photos moved to the Trash", with the files that
/// couldn't go (their photos stay).
pub fn trash_message(r: &Value) -> String {
    let removed = r["removed"].as_u64().unwrap_or(0);
    let trashed = r["trashed"].as_array().map(Vec::len).unwrap_or(0);
    let failed = r["failed"].as_array().map(Vec::len).unwrap_or(0);
    let mut text = match (removed, trashed) {
        (0, _) => String::new(),
        (n, 0) => crate::i18n::tr_format!("{what} removed from the library", what = photos(n)),
        (n, _) => crate::i18n::tr_format!("{what} moved to the Trash", what = photos(n)),
    };
    if failed > 0 {
        if !text.is_empty() {
            text.push_str(" · ");
        }
        text.push_str(&format!(
            "{n} file{s} couldn't be moved to the Trash, so {it} stayed: {e}",
            n = failed,
            s = if failed == 1 { "" } else { "s" },
            it = if failed == 1 { "its photo" } else { "their photos" },
            e = r["failed"][0]["error"].as_str().unwrap_or("")
        ));
    }
    text
}

/// The toast after a delete: what happened, and undo. when something changed.
pub fn delete_toast(app: &mut LightcraftApp, r: &Value, text: String) {
    let changed = r["removed"].as_u64().is_none_or(|n| n > 0);
    app.toast_soon(text.clone(), if r["failed"].as_array().is_some_and(|f| !f.is_empty()) { 8.0 } else { 6.0 });
    app.ui.toast_undo = changed.then_some(text);
}

/// The toast after an undo: "Undo", or which files couldn't come back from the Trash.
pub fn undo_message(r: &Value) -> String {
    match r["notBack"].as_array().filter(|a| !a.is_empty()) {
        Some(a) => format!(
            "Undo · {n} file{s} couldn't come back from the Trash: {e}",
            n = a.len(),
            s = if a.len() == 1 { "" } else { "s" },
            e = a[0].as_str().unwrap_or("")
        ),
        None => crate::i18n::tr("Undo").to_string(),
    }
}

/// One Painter click on photo `id` (L12): apply `mode`'s `value`; a label or album the photo
/// already has comes off again (like the keyword painter).
pub fn paint(app: &mut LightcraftApp, id: lightcraft_catalog::PhotoId, mode: &str, value: &str) -> Result<Value, String> {
    let photo = app.session.catalog.photo(id).cloned().ok_or("no such photo")?;
    let ids = json!([id.0]);
    match mode {
        "label" => {
            let has = photo.label.is_some_and(|l| format!("{l:?}").eq_ignore_ascii_case(value));
            app.run("photo.label", json!({"ids": ids, "label": if has { "none" } else { value }}))
        }
        "flag" => app.run("photo.flag", json!({"ids": ids, "flag": value})),
        "rating" => app.run("photo.rate", json!({"ids": ids, "rating": value.parse::<u64>().map_err(|_| "rating is 0..5")?})),
        "preset" => app.run("preset.apply", json!({"ids": ids, "id": value})),
        "metadata" => app.run("metadata.applyPreset", json!({"ids": ids, "name": value})),
        "rotate" => {
            let cmd = match value {
                "left" => "photo.rotateLeft",
                "right" => "photo.rotateRight",
                "flipH" => "photo.flipHorizontal",
                "flipV" => "photo.flipVertical",
                _ => return Err("rotate is left|right|flipH|flipV".into()),
            };
            app.run(cmd, json!({"ids": ids}))
        }
        "album" => {
            let aid = value.parse::<u64>().map_err(|_| "album is an album id")?;
            let has = app.session.catalog.album(lightcraft_catalog::AlbumId(aid)).is_some_and(|a| a.photos.contains(&id));
            app.run(if has { "album.removePhotos" } else { "album.addPhotos" }, json!({"id": aid, "ids": ids}))
        }
        _ => Err(format!("unknown painter mode `{mode}`")),
    }
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

/// `app.viewSpace`'s answer: the setting, whether photo textures carry Display P3 values, and
/// the window surface's tag (`tagged`: the system colour-manages the window onto the display;
/// a new request takes effect on the next frame).
fn view_space_state(app: &LightcraftApp) -> Value {
    let space = app.ui.settings.view_space;
    let tagged = app.window_space.as_ref().and_then(|w| w.tagged());
    json!({
        "space": space.id(),
        "displayP3": space.is_p3(),
        "tagsSurface": space.tags_surface(),
        "tagged": tagged,
        "colorManaged": tagged.is_some(),
        "texturesP3": app.renderer.display_p3,
    })
}

/// The quick centre mask's name (center. finds it again by name).
pub const CENTER_MASK: &str = "Center";

/// center.'s built-in mask: a centred vignette — everything around a soft oval on the middle of
/// the photo a little darker, feathered in (Bryan, 2026-11-23). Settings ▸ Develop ▸ Center mask
/// and right-click ▸ "make this the default" change it.
pub fn default_center_mask() -> Value {
    json!({
        "shape": {"kind": "radial", "center": {"x": 0.5, "y": 0.5}, "rx": 0.38, "ry": 0.30, "angle": 0.0, "feather": 85.0, "invert": true},
        "adjust": {"exposure": -0.6}
    })
}
