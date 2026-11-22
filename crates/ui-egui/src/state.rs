//! UI state (serde: saved as preferences, readable/settable through the control channel).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ViewMode {
    /// Justified rows ("Photo Grid").
    #[default]
    PhotoGrid,
    SquareGrid,
    Detail,
    /// Two photos side by side (select | candidate), synced zoom.
    Compare,
    /// The selected photos tiled.
    Survey,
    /// A reference photo (left, fixed) beside the active photo (right, being edited).
    Reference,
    /// A card per person named on faces (close-up, name, photo count).
    People,
}

/// Lightroom Classic's modules: Library (grid, loupe, compare, survey; organising) and Develop
/// (one photo, the develop panels). The photo selection is shared; each module has its own side
/// panels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Module {
    #[default]
    Library,
    Develop,
}

impl Module {
    /// The views a module shows: Develop is the loupe (or the reference view), the rest is Library.
    pub fn of_view(view: ViewMode) -> Option<Module> {
        match view {
            ViewMode::PhotoGrid | ViewMode::SquareGrid | ViewMode::Compare | ViewMode::Survey | ViewMode::People => Some(Module::Library),
            ViewMode::Detail | ViewMode::Reference => None,
        }
    }
}

/// Lights Out (L cycles): everything but the photo dimmed, then black.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LightsOut {
    #[default]
    On,
    Dim,
    Off,
}

impl LightsOut {
    pub fn next(self) -> LightsOut {
        match self {
            LightsOut::On => LightsOut::Dim,
            LightsOut::Dim => LightsOut::Off,
            LightsOut::Off => LightsOut::On,
        }
    }
    /// How dark the chrome is painted over (0 = not at all).
    pub fn alpha(self) -> u8 {
        match self {
            LightsOut::On => 0,
            LightsOut::Dim => 204,
            LightsOut::Off => 255,
        }
    }
}

/// The right-hand tool/panel shown next to the tool strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RightPanel {
    #[default]
    None,
    Edit,
    Crop,
    Remove,
    Masking,
    RedEye,
    Versions,
    Activity,
    Keywords,
    Info,
    /// The profile browser (part of Edit).
    Profiles,
}

impl RightPanel {
    pub fn is_edit_tool(self) -> bool {
        matches!(self, RightPanel::Edit | RightPanel::Profiles | RightPanel::Crop | RightPanel::Remove | RightPanel::Masking | RightPanel::RedEye)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Zoom {
    #[default]
    Fit,
    Fill,
    /// 100 % = one image pixel per physical screen pixel.
    Percent(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BeforeAfter {
    #[default]
    Off,
    /// Hold `\`: show the original.
    Original,
    SideBySide,
    Split,
    /// Before above after.
    TopBottom,
    /// One image split horizontally: before above the line.
    SplitTopBottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CropOverlay {
    #[default]
    Thirds,
    Grid,
    Golden,
    Diagonal,
    Triangle,
    Spiral,
    None,
}

/// Which view the app opens in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StartupView {
    /// Whatever was showing at quit.
    #[default]
    Last,
    Grid,
    Detail,
}

/// When grid cells show their rating / flag / edited badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GridBadges {
    /// On hover, on the selection, and on rated or flagged photos.
    #[default]
    Auto,
    Always,
    Never,
}

/// The loupe's info overlay (Cmd+I cycles; I in the full-screen preview).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InfoOverlay {
    #[default]
    Off,
    /// File name, capture date and dimensions.
    Basic,
    /// File name, camera, lens and exposure (shutter, aperture, ISO, focal length).
    Exposure,
}

impl InfoOverlay {
    pub fn next(self) -> InfoOverlay {
        match self {
            InfoOverlay::Off => InfoOverlay::Basic,
            InfoOverlay::Basic => InfoOverlay::Exposure,
            InfoOverlay::Exposure => InfoOverlay::Off,
        }
    }
}

/// One line of the loupe's info overlay (Library View Options ▸ Loupe Info 1 / 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoupeInfoField {
    #[default]
    None,
    FileName,
    /// The file name and a virtual copy's name.
    FileAndCopyName,
    /// The capture date and time (the import date when the photo has none).
    CaptureDate,
    /// Width × height in pixels.
    Dimensions,
    /// Shutter speed, aperture, ISO and focal length.
    Exposure,
    Camera,
    Lens,
    /// Camera and lens on one line.
    CameraAndLens,
    Title,
    Caption,
    /// Rating and colour label.
    RatingAndLabel,
    Keywords,
    Folder,
}

impl LoupeInfoField {
    /// Every field, in the order View Options lists them.
    pub const ALL: &'static [LoupeInfoField] = &[
        LoupeInfoField::None,
        LoupeInfoField::FileName,
        LoupeInfoField::FileAndCopyName,
        LoupeInfoField::CaptureDate,
        LoupeInfoField::Dimensions,
        LoupeInfoField::Exposure,
        LoupeInfoField::Camera,
        LoupeInfoField::Lens,
        LoupeInfoField::CameraAndLens,
        LoupeInfoField::Title,
        LoupeInfoField::Caption,
        LoupeInfoField::RatingAndLabel,
        LoupeInfoField::Keywords,
        LoupeInfoField::Folder,
    ];

    /// The name View Options shows.
    pub fn label(self) -> &'static str {
        match self {
            LoupeInfoField::None => "None",
            LoupeInfoField::FileName => "File Name",
            LoupeInfoField::FileAndCopyName => "File Name and Copy Name",
            LoupeInfoField::CaptureDate => "Capture Date/Time",
            LoupeInfoField::Dimensions => "Dimensions",
            LoupeInfoField::Exposure => "Exposure and ISO",
            LoupeInfoField::Camera => "Camera",
            LoupeInfoField::Lens => "Lens",
            LoupeInfoField::CameraAndLens => "Camera + Lens",
            LoupeInfoField::Title => "Title",
            LoupeInfoField::Caption => "Caption",
            LoupeInfoField::RatingAndLabel => "Rating and Label",
            LoupeInfoField::Keywords => "Keywords",
            LoupeInfoField::Folder => "Folder",
        }
    }
}

/// The loupe's Info 1 and Info 2 lines, and Show Briefly When Photo Changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoupeInfo {
    pub info1: [LoupeInfoField; 3],
    pub info2: [LoupeInfoField; 3],
    /// Which Info shows for a moment when the loupe changes photo with the overlay off (Off: none).
    pub briefly: InfoOverlay,
}

impl LoupeInfo {
    /// Info 1's lines before they are changed (and after Use Defaults).
    pub const INFO1: [LoupeInfoField; 3] = [LoupeInfoField::FileName, LoupeInfoField::CaptureDate, LoupeInfoField::Dimensions];
    /// Info 2's lines before they are changed (and after Use Defaults).
    pub const INFO2: [LoupeInfoField; 3] = [LoupeInfoField::FileName, LoupeInfoField::Exposure, LoupeInfoField::CameraAndLens];
}

impl Default for LoupeInfo {
    fn default() -> Self {
        LoupeInfo { info1: LoupeInfo::INFO1, info2: LoupeInfo::INFO2, briefly: InfoOverlay::Off }
    }
}

/// How long Show Briefly When Photo Changes shows the info overlay (seconds).
pub const BRIEF_INFO_SECS: f64 = 1.5;

/// When the loupe last changed photo, for Show Briefly When Photo Changes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BriefInfo {
    pub photo: Option<lightcraft_catalog::PhotoId>,
    pub since: f64,
    pub shown: bool,
}

/// App-level preferences (Settings dialog). They belong to the app, not a library, and are
/// saved with the UI state in the app's config folder (`ui.json`, key `settings`); library
/// preferences (import defaults, XMP, cache size) live in the library's `prefs.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppSettings {
    /// Library opened at launch when no `--library` is given (empty = the default location).
    pub library_path: String,
    pub startup_view: StartupView,
    /// Ask before moving photos to Recently Deleted (keyboard and menu; with `trash_delete` off).
    pub confirm_delete: bool,
    /// Delete (⌫) asks like Lightroom: Delete from Disk (files to the Trash) or Remove (out of the
    /// library). Off: it moves photos to Recently Deleted.
    pub trash_delete: bool,
    /// GPU rendering allowed (`app.gpu`).
    pub gpu: bool,
    /// Largest long edge (pixels) the loupe renders at.
    pub preview_edge: u32,
    /// Edit in External Editor: the application ("" = the system's default for TIFF files).
    pub external_editor: String,
    /// Memory the caches may hold together, in MB (0 = automatic; `app.memoryBudget`).
    pub memory_mb: u32,
    /// Filmstrip: file names above the thumbnails.
    pub film_names: bool,
    /// Filmstrip: rating / flag / edited badges on the thumbnails.
    pub film_badges: bool,
    /// Grid: when to show the rating / flag / edited badges.
    pub grid_badges: GridBadges,
    /// Square Grid: each cell's index number in front of its caption (Lightroom's Index Number).
    pub grid_index: bool,
    /// Loupe Info 1 / Info 2 lines and Show Briefly When Photo Changes (L19, Library View
    /// Options ▸ Loupe View; ported from upstream).
    pub loupe_info: LoupeInfo,
    /// Square Grid cells: compact (name and format) or expanded (L18: a second line with the
    /// dimensions and capture date, rotate buttons in the footer).
    pub grid_expanded: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            library_path: String::new(),
            startup_view: StartupView::Last,
            confirm_delete: false,
            trash_delete: true,
            gpu: true,
            preview_edge: 2560,
            external_editor: String::new(),
            memory_mb: 0,
            film_names: true,
            film_badges: true,
            grid_badges: GridBadges::Auto,
            grid_index: false,
            loupe_info: LoupeInfo::default(),
            grid_expanded: false,
        }
    }
}

/// Preview sizes offered in Settings → Performance.
pub const PREVIEW_EDGES: [u32; 4] = [1600, 2560, 3840, 5120];

/// Click-zoom ratios offered (percent): 1:1, 2:1, 3:1, 4:1, 8:1.
pub const CLICK_ZOOMS: [u32; 5] = [100, 200, 300, 400, 800];

/// Width limits of a side panel the user resizes (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelWidth {
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

impl PanelWidth {
    /// `w` within the limits (the default when it isn't a number).
    pub fn clamp(&self, w: f32) -> f32 {
        if w.is_finite() { w.clamp(self.min, self.max) } else { self.default }
    }
}

/// The left sidebar (sources, albums, folders).
pub const LEFT_WIDTH: PanelWidth = PanelWidth { min: 200.0, default: 268.0, max: 480.0 };
/// The filmstrip's height limits (points; dragging its top edge resizes it).
pub const FILM_HEIGHT: PanelWidth = PanelWidth { min: 74.0, default: 118.0, max: 260.0 };
/// The right panel (Edit, Masking, Info, …).
pub const RIGHT_WIDTH: PanelWidth = PanelWidth { min: 250.0, default: 270.0, max: 520.0 };
/// The photo area the side panels always leave free (as far as their minimum widths allow).
pub const MIN_PHOTO_WIDTH: f32 = 360.0;

/// The Sync Settings dialog's checkboxes as last left. Match exposure, auto align and match camera & lens color start off.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncPrefs {
    /// The settings groups checked (`None`: never opened — Classic's defaults).
    pub groups: Option<Vec<String>>,
    pub match_exposure: bool,
    pub auto_align: bool,
    /// Match camera & lens color (off by default).
    pub match_color: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    #[serde(default = "crate::i18n::default_language")]
    pub language: crate::i18n::Locale,
    /// The Build Previews run last announced (its identity, finished?).
    #[serde(skip)]
    pub preview_build_seen: Option<(usize, bool)>,
    /// Saving the library is failing (announced with a toast; the top bar shows a warning until
    /// a later save succeeds).
    #[serde(skip)]
    pub unsaved_seen: bool,
    pub view: ViewMode,
    /// Library or Develop (the module picker, G / E / D).
    pub module: Module,
    /// The Library view to go back to from Develop (grid, loupe, compare…).
    pub library_view: ViewMode,
    /// The side panels and the toolbar can be hidden (F7 / F8 / Tab / ⇧Tab, T); the top bar (F5).
    pub left_panel: bool,
    pub right_panel: bool,
    pub top_panel: bool,
    pub toolbar: bool,
    /// Filmstrip height (points, within [`FILM_HEIGHT`]).
    pub film_height: f32,
    /// Lights Out (L).
    #[serde(skip)]
    pub lights_out: LightsOut,
    /// Library right panel sections that are open (`histogram`, `quickDevelop`, `keywording`,
    /// `keywordList`, `metadata`, `comments`).
    pub library_sections: Vec<String>,
    /// Develop left panel sections that are open (`navigator`, `presets`, `snapshots`, `history`,
    /// `collections`).
    pub develop_left_sections: Vec<String>,
    /// Solo mode for the side panels (⌥-click a header): opening one closes the others.
    pub solo_left: bool,
    /// Develop ▸ Presets groups that are open (closed by default, as in Classic).
    pub preset_groups_open: Vec<String>,
    /// The Import window's last choices (source, mode, destination, options).
    pub import_prefs: crate::import::ImportPrefs,
    /// Named Import window choices (Import Presets).
    pub import_presets: Vec<crate::import::ImportPreset>,
    /// Sync Settings: what was checked last time (kept between runs and restarts).
    pub sync_prefs: SyncPrefs,
    pub right: RightPanel,
    /// Widths of the left sidebar and of the right panel in points (dragging their inner edge
    /// resizes them; within [`LEFT_WIDTH`] / [`RIGHT_WIDTH`]).
    pub left_width: f32,
    pub right_width: f32,
    /// Presets column open (opens to the left of the Edit panel).
    pub presets: bool,
    /// Presets column: show a live thumbnail of the photo with each preset.
    pub preset_thumbs: bool,
    pub filmstrip: bool,
    pub zoom: Zoom,
    /// Pan offset of the loupe when zoomed (image-normalized centre).
    pub pan: (f32, f32),
    /// Zoom applied by a click on the image (and Z / Space), in percent (100 = 1:1, 200 = 2:1);
    /// one of [`CLICK_ZOOMS`].
    pub click_zoom: u32,
    /// Animate the loupe rect toward its target (set by a click-zoom).
    #[serde(skip)]
    pub zoom_anim: bool,
    pub before_after: BeforeAfter,
    pub thumb_size: f32,
    /// Open Edit sections by id.
    pub open_sections: Vec<String>,
    /// Open flyouts (curve, mixer, grading…).
    pub open_flyouts: Vec<String>,
    /// Single-panel mode: opening one section closes the others.
    pub single_panel: bool,
    pub show_clipping: bool,
    pub histogram: bool,
    /// Soft proofing (S in the loupe): render as `proof` would hold the photo.
    pub soft_proof: bool,
    pub proof: lightcraft_engine::pipeline::Proof,
    /// Masking: show the selected mask as a rendered overlay (O), how (`MaskView` name, ⇧O cycles),
    /// in which colour and opacity (0..100, colour views), and whether pins are drawn.
    pub mask_overlay: bool,
    pub mask_overlay_mode: String,
    pub mask_overlay_color: [u8; 3],
    pub mask_overlay_opacity: f32,
    pub mask_pins: bool,
    /// The overlay (on, mode) to go back to when Show Luminance Map is turned off.
    #[serde(skip)]
    pub luminance_map_restore: Option<(bool, String)>,
    /// Mirroring of the triangle / spiral crop guides (0..4).
    pub crop_overlay_orient: u8,
    pub crop_overlay: CropOverlay,
    pub show_filenames: bool,
    /// What the square grid's caption shows: `filename`, `exposure` (shutter · aperture · ISO ·
    /// focal length) or `date`.
    pub grid_info: String,
    /// Photo counts next to sources and albums in the left panel.
    pub show_counts: bool,
    /// Face / pet boxes (read from XMP) over the photo in the loupe.
    pub face_boxes: bool,
    /// Left-sidebar sections folded shut by their header (`albums`, `local`, `byDate`,
    /// `keywords`); the rest are open.
    pub collapsed_sidebar: Vec<String>,
    /// Folders rows opened or closed by hand (folder key → open); the rest keep their default
    /// (a disk open, a folder closed).
    pub folders_open: std::collections::BTreeMap<String, bool>,
    /// Local sidebar locations hidden with “Remove from Local” (folders on disk are untouched).
    pub hidden_locations: Vec<String>,
    /// Copies opened in an external editor this session (reloaded when the window is focused
    /// again), and whether the window had focus last frame.
    #[serde(skip)]
    pub external_edits: Vec<u64>,
    #[serde(skip)]
    pub was_focused: bool,
    /// When the watched folder was last scanned (egui time).
    #[serde(skip)]
    pub auto_import_at: f64,
    /// The develop control whose slider is being dragged (geometry sliders show a grid).
    #[serde(skip)]
    pub dragging_control: Option<String>,
    pub search: String,
    /// Focus the search field on the next frame (Edit → Find…).
    #[serde(skip)]
    pub focus_search: bool,
    /// A mask being renamed in the Masks list: its id and the edited name.
    #[serde(skip)]
    pub renaming_mask: Option<(u32, String)>,
    /// The Describe field (AI mask from a text prompt) while open: how the selection combines
    /// (`new` mask, or `add`/`subtract`/`intersect` on the selected one) and the text typed.
    #[serde(skip)]
    pub describe: Option<(String, String)>,
    /// A SAM 3 download was started from the app (to report its end once).
    #[serde(skip)]
    pub sam_downloading: bool,
    /// When to start the zoomed-in detail pass of an AI mask (app time) and which mask: set by
    /// each click or description, so the pass runs once the clicking stops.
    #[serde(skip)]
    pub detail_due: Option<(f64, u32)>,
    /// A mask component being renamed inline: (mask id, component index, name).
    pub renaming_component: Option<(u32, usize, String)>,
    /// Close the window on the next frame (File → Quit).
    #[serde(skip)]
    pub quit: bool,
    /// Photos being dragged from the grid (dropped on an album to add them).
    #[serde(skip)]
    pub dragging_photos: Option<Vec<u64>>,
    /// Selected curve channel in the Curve flyout.
    pub curve_channel: String,
    /// Selected mixer mode: "hue" | "saturation" | "luminance" | "all".
    pub mixer_mode: String,
    /// Where the work was left off, overall and per folder / album ([`crate::leftoff`]).
    #[serde(deserialize_with = "crate::leftoff::lenient")]
    pub left_off: crate::leftoff::LeftOff,
    /// Selected colour grading wheel: "3way" | "shadows" | "midtones" | "highlights" | "global".
    pub grading_mode: String,
    /// Active on-canvas tool: "", "brush", "linear", "radial", "wbPicker", "straighten", "remove".
    pub tool: String,
    pub brush_size: f32,
    pub brush_feather: f32,
    pub brush_flow: f32,
    pub brush_erase: bool,
    /// Brush Auto Mask: dabs stick to areas like the one under the brush centre.
    pub brush_auto_mask: bool,
    /// Remove tool brush: size (fraction of the long edge), feather and opacity (0..100).
    pub remove_size: f32,
    pub remove_feather: f32,
    pub remove_opacity: f32,
    /// Selected Point Color sample.
    pub point_color: usize,
    /// Point Color "Visualize range": the selected sample's range in colour, the rest grey.
    pub point_color_visualize: bool,
    /// Red Eye panel: selected correction, and whether new ones are pet eyes.
    pub eye: usize,
    pub eye_pet: bool,
    /// Remove tool: Visualize Spots (high-pass black/white view) and its threshold 0..100.
    pub visualize_spots: bool,
    pub spots_threshold: f32,
    /// The library filter bar above the grid.
    pub filter_bar: bool,
    /// Its metadata column browser (Lightroom's Metadata filter) and the columns shown.
    pub column_browser: bool,
    pub metadata_columns: Vec<String>,
    /// Folders kept in Local (Browse Folder…, Keep in Local / Add to Local; `local.addRoot`):
    /// listed after the built-in locations, across restarts, whatever is browsed.
    pub local_roots: Vec<String>,
    /// The top-level Local row listed for this session only, for a browsed folder outside every
    /// kept location; it stays while browsing below it (see `panels::left::local_places`).
    #[serde(skip)]
    pub local_browse_root: Option<String>,
    /// Culling: after a rating, flag or colour-label key, move to the next photo.
    pub auto_advance: bool,
    /// Full-screen preview (F): the photo alone on black, no chrome.
    pub fullscreen: bool,
    /// Window ▸ Second Window.
    pub second_window: bool,
    /// The keyword painter: clicking a photo in the grid toggles this keyword on it.
    #[serde(skip)]
    pub keyword_painter: Option<String>,
    /// The Painter's other modes (L12): `(mode, value)` — label, flag, rating, preset (develop
    /// preset id), metadata (metadata preset name), rotate (left / right / flipH / flipV),
    /// album (album id). A click on a grid photo applies it (label / album toggle off again).
    #[serde(skip)]
    pub painter: Option<(String, String)>,
    /// A running slideshow (full screen): seconds per photo, when the next one is due (egui
    /// time), paused.
    #[serde(skip)]
    pub slideshow: Option<(f64, f64, bool)>,
    /// Info overlay on the loupe.
    pub info_overlay: InfoOverlay,
    /// The Metadata panel's view (L20): `default`, `all`, `exif`, `iptc`, `location`,
    /// `largeCaption`, `minimal`, `quickDescribe`, `custom` (see `panels::right::METADATA_VIEWS`).
    #[serde(default)]
    pub metadata_view: String,
    /// The groups the `custom` view shows (Customize…).
    #[serde(default)]
    pub metadata_custom: Vec<String>,
    /// Show Briefly When Photo Changes: when the loupe changed photo (not saved).
    #[serde(skip)]
    pub brief_info: BriefInfo,
    /// Navigator mini map in the loupe while zoomed in.
    pub navigator: bool,
    /// App preferences (Settings dialog).
    pub settings: AppSettings,
    /// Window full screen requested (⇧⌘F); the host applies it (`None` = no change pending).
    #[serde(skip)]
    pub window_fullscreen: Option<bool>,
    /// Compare view: (select, candidate) photo ids.
    #[serde(skip)]
    pub compare: Option<(u64, u64)>,
    /// Reference view: the reference photo.
    #[serde(skip)]
    pub reference: Option<u64>,
    /// Transient toast text, expiry (seconds of app time), and optional colour-label styling.
    #[serde(skip)]
    pub toast: Option<(String, f64, Option<lightcraft_catalog::ColorLabel>)>,
    /// The result of the last Find Missing Photos (it searches in the background).
    #[serde(skip)]
    pub last_find_missing: Option<serde_json::Value>,
    /// The library (its `Session::library_generation`) whose preview-only raws were rechecked
    /// (`crate::recheck`).
    #[serde(skip)]
    pub rechecked_library: Option<u64>,
    #[serde(skip)]
    pub status: String,
    #[serde(skip)]
    pub dialog: Option<Dialog>,
}

/// Settings group ids (`SettingsGroup` serde names) a new preset includes by default: everything
/// but crop, masks, spots and red eye.
pub fn default_preset_groups() -> Vec<String> {
    lightcraft_develop::SettingsGroup::default_copy().iter().filter_map(|g| serde_json::to_value(g).ok()?.as_str().map(str::to_string)).collect()
}

impl Dialog {
    /// A fresh Create Preset dialog.
    pub fn create_preset() -> Dialog {
        Dialog::CreatePreset { name: String::new(), group: "User Presets".into(), groups: default_preset_groups() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Dialog {
    NewAlbum {
        name: String,
        folder: bool,
    },
    RenameAlbum {
        id: u64,
        name: String,
    },
    /// One text field; OK runs `command` with `params` plus `{key: value}` (rename a preset or a
    /// version, move a preset to a new group…).
    TextPrompt {
        title: String,
        hint: String,
        value: String,
        command: String,
        params: serde_json::Value,
        key: String,
    },
    /// Edit Capture Time: `mode` "set" (`time`; the others shift along), "shift" (by `days`,
    /// `hours`, `minutes`) or "zone" (time-zone shift by `zone` hours).
    CaptureTime {
        mode: String,
        time: String,
        days: i32,
        hours: i32,
        minutes: i32,
        zone: f32,
    },
    /// The import review (File → Import Photos…).
    Import {
        opts: Box<crate::import::ImportDialog>,
    },
    /// Edit the colour label names (red, yellow, green, blue, purple; empty = the colour's name).
    LabelNames {
        names: Vec<String>,
        /// Also save the names as a label set of this name (empty = don't).
        save_as: String,
    },
    /// Batch rename the selected photos with a file-name template.
    Rename {
        template: String,
        start: u32,
        /// `{text}`: Lightroom's Custom Text.
        text: String,
    },
    /// Edit Keyword Tag (L11): name, synonyms, export options, person.
    EditKeyword(KeywordEdit),
    /// Rename a keyword on every photo (children included).
    RenameKeyword {
        from: String,
        to: String,
    },
    /// Merge keywords into another one on every photo.
    MergeKeywords {
        from: Vec<String>,
        into: String,
    },
    /// Auto-stack by capture time: the largest gap between consecutive shots, in seconds.
    AutoStack {
        gap: f32,
    },
    /// Save the current view (source + filter) as a smart album.
    NewSmartAlbum {
        name: String,
    },
    /// Help ▸ What's New.
    WhatsNew,
    /// Photo ▸ Assisted Culling: reject photos below this focus score (0 = none), pick the best
    /// of each burst.
    Cull {
        reject_below: f32,
        pick_best: bool,
    },
    /// Help ▸ System Info: (label, value) rows.
    SystemInfo {
        rows: Vec<(String, String)>,
    },
    /// Every metadata field of a photo's file (`photo.allMetadata`), filtered by `search`.
    AllMetadata {
        title: String,
        rows: serde_json::Value,
        search: String,
    },
    /// Create (`id` None) or edit a smart album's rules.
    SmartRules {
        id: Option<u64>,
        name: String,
        rules: lightcraft_catalog::RuleSet,
    },
    /// `groups`: the settings groups the preset includes (`SettingsGroup` ids).
    CreatePreset {
        name: String,
        group: String,
        #[serde(default = "default_preset_groups")]
        groups: Vec<String>,
    },
    CopySettings {
        groups: Vec<String>,
    },
    /// Synchronize Settings (Classic's Sync…): which groups of the active photo's settings go to
    /// the other selected photos.
    SyncSettings {
        groups: Vec<String>,
        /// Then set each photo's Exposure so it renders as bright as the active one.
        match_exposure: bool,
        /// Then level each photo's horizon.
        auto_align: bool,
        /// First match each photo's colour to the active one across cameras, lenses and phones.
        match_color: bool,
    },
    /// Paste Selected Settings: which of the copied groups to paste.
    PasteSettings {
        groups: Vec<String>,
    },
    /// `resize` is used unless `full_size`; `limit_kb` 0 = no limit; `dir` empty = default export folder.
    Export {
        opts: lightcraft_engine::export::ExportOptions,
        full_size: bool,
        resize: lightcraft_engine::export::Resize,
        /// Name typed for "Save as Preset".
        #[serde(default)]
        preset_name: String,
        limit_kb: u32,
        dir: String,
    },
    /// Photo Merge (HDR / Panorama / HDR Panorama) options; the preview lives in the app.
    Merge {
        opts: crate::merge::MergeDialog,
    },
    /// Settings (preferences): `tab` = general | import | performance | interface.
    Settings {
        tab: String,
    },
    /// Object and Describe masks need the SAM 3 model, which isn't installed: offer to download
    /// it (size, licence, progress). `then`: the AI mask to start once it is there (`kind`
    /// object|prompt, `op` new|add|subtract|intersect).
    SamModel {
        then: Option<(String, String)>,
        /// Why the download couldn't start (shown in the dialog).
        #[serde(default)]
        error: Option<String>,
    },
    /// Confirm moving photos to Recently Deleted.
    ConfirmDelete {
        count: usize,
    },
    /// Lightroom's delete question: Delete from Disk (`photo.deleteFromDisk`, OK) or Remove
    /// (`photo.removeFromLibrary`). `copies` of the `count` are virtual copies (only removable);
    /// `rejected`: the rejected photos of the view (Delete Rejected Photos, ⌘⌫).
    DeletePhotos {
        count: usize,
        copies: usize,
        rejected: bool,
    },
    /// Confirm taking a folder's photos out of the library (`library.removeFolder`).
    RemoveFolder {
        path: String,
        /// What the question calls it (a folder's last two names, a disk's name).
        name: String,
        count: usize,
        /// A whole disk or share (`library.removeFolder` takes it only on request).
        disk: bool,
    },
    /// File ▸ Restore Library from Backup…: the catalog backups, one picked to restore.
    Backups {
        pick: Option<String>,
    },
    About,
    Shortcuts,
    /// File ▸ Sync from Lightroom Classic…: what a sync would bring (`report`, the dry run; `None`
    /// while the catalog is read into `records` / `extras`) and the rule for photos changed on
    /// both sides (`conflicts`: ask | lightroom | newest).
    LightroomSync {
        catalog: String,
        records: String,
        extras: String,
        report: Option<serde_json::Value>,
        conflicts: String,
        error: Option<String>,
    },
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            language: crate::i18n::default_language(),
            preview_build_seen: None,
            unsaved_seen: false,
            luminance_map_restore: None,
            view: ViewMode::Detail,
            module: Module::Library,
            library_view: ViewMode::PhotoGrid,
            left_panel: true,
            right_panel: true,
            top_panel: true,
            toolbar: true,
            film_height: FILM_HEIGHT.default,
            lights_out: LightsOut::On,
            library_sections: vec!["histogram".into(), "quickDevelop".into(), "keywording".into(), "metadata".into()],
            develop_left_sections: vec!["navigator".into(), "presets".into(), "history".into()],
            solo_left: false,
            import_prefs: Default::default(),
            import_presets: Vec::new(),
            sync_prefs: Default::default(),
            preset_groups_open: Vec::new(),
            left_width: LEFT_WIDTH.default,
            right_width: RIGHT_WIDTH.default,
            right: RightPanel::Edit,
            presets: false,
            preset_thumbs: false,
            filmstrip: true,
            zoom: Zoom::Fit,
            pan: (0.5, 0.5),
            click_zoom: 100,
            zoom_anim: false,
            before_after: BeforeAfter::Off,
            thumb_size: 220.0,
            open_sections: vec!["basic".into()],
            open_flyouts: vec![],
            single_panel: false,
            show_clipping: false,
            histogram: true,
            soft_proof: false,
            proof: lightcraft_engine::pipeline::Proof { dest_warning: false, ..Default::default() },
            mask_overlay: true,
            mask_overlay_mode: "color".into(),
            mask_overlay_color: [230, 30, 40],
            mask_overlay_opacity: 50.0,
            mask_pins: true,
            crop_overlay: CropOverlay::Thirds,
            crop_overlay_orient: 0,
            show_filenames: true,
            grid_info: "filename".into(),
            show_counts: true,
            face_boxes: true,
            collapsed_sidebar: Vec::new(),
            folders_open: Default::default(),
            hidden_locations: Vec::new(),
            dragging_control: None,
            external_edits: Vec::new(),
            was_focused: true,
            auto_import_at: 0.0,
            search: String::new(),
            focus_search: false,
            renaming_mask: None,
            describe: None,
            detail_due: None,
            sam_downloading: false,
            renaming_component: None,
            quit: false,
            dragging_photos: None,
            curve_channel: "parametric".into(),
            mixer_mode: "hue".into(),
            left_off: Default::default(),
            grading_mode: "3way".into(),
            tool: String::new(),
            brush_size: 0.04,
            brush_feather: 50.0,
            brush_flow: 60.0,
            brush_erase: false,
            brush_auto_mask: false,
            remove_size: 0.02,
            remove_feather: 50.0,
            remove_opacity: 100.0,
            point_color: 0,
            point_color_visualize: false,
            eye: 0,
            eye_pet: false,
            visualize_spots: false,
            spots_threshold: 50.0,
            auto_advance: false,
            fullscreen: false,
            slideshow: None,
            second_window: false,
            keyword_painter: None,
            painter: None,
            info_overlay: InfoOverlay::Off,
            brief_info: BriefInfo::default(),
            metadata_view: String::new(),
            metadata_custom: Vec::new(),
            navigator: true,
            settings: AppSettings::default(),
            window_fullscreen: None,
            filter_bar: false,
            column_browser: false,
            metadata_columns: lightcraft_engine::catalog::columns::DEFAULT_COLUMNS.iter().map(|c| c.to_string()).collect(),
            local_roots: Vec::new(),
            local_browse_root: None,
            compare: None,
            reference: None,
            toast: None,
            last_find_missing: None,
            rechecked_library: None,
            status: String::new(),
            dialog: None,
        }
    }
}

impl UiState {
    pub fn section_open(&self, id: &str) -> bool {
        self.open_sections.iter().any(|s| s == id)
    }
    pub fn toggle_section(&mut self, id: &str) {
        if self.section_open(id) {
            self.open_sections.retain(|s| s != id);
        } else {
            if self.single_panel {
                self.open_sections.clear();
            }
            self.open_sections.push(id.to_string());
        }
    }
    /// Open or close a section of a side-panel list (`list` = [`UiState::library_sections`], …);
    /// `solo` closes the others when one opens.
    pub fn toggle_in(list: &mut Vec<String>, id: &str, solo: bool) {
        if list.iter().any(|s| s == id) {
            list.retain(|s| s != id);
        } else {
            if solo {
                list.clear();
            }
            list.push(id.to_string());
        }
    }
    /// The module the current view belongs to (Library views force Library; the loupe keeps the
    /// module it is in).
    pub fn sync_module(&mut self) {
        if let Some(m) = Module::of_view(self.view) {
            self.module = m;
        }
        if self.module == Module::Develop && !matches!(self.view, ViewMode::Detail | ViewMode::Reference) {
            self.view = ViewMode::Detail;
        }
        if self.module == Module::Library {
            self.library_view = self.view;
        }
    }
    pub fn sidebar_section_collapsed(&self, id: &str) -> bool {
        self.collapsed_sidebar.iter().any(|s| s == id)
    }
    pub fn toggle_sidebar_section(&mut self, id: &str) {
        if self.sidebar_section_collapsed(id) {
            self.collapsed_sidebar.retain(|s| s != id);
        } else {
            self.collapsed_sidebar.push(id.to_string());
        }
    }
    pub fn flyout_open(&self, id: &str) -> bool {
        self.open_flyouts.iter().any(|s| s == id)
    }
    pub fn toggle_flyout(&mut self, id: &str) {
        if self.flyout_open(id) {
            self.open_flyouts.retain(|s| s != id);
        } else {
            self.open_flyouts.push(id.to_string());
        }
    }
    /// Clamp values restored from disk.
    pub fn sanitized(mut self) -> Self {
        self.thumb_size = self.thumb_size.clamp(90.0, 480.0);
        self.left_width = LEFT_WIDTH.clamp(self.left_width);
        self.film_height = FILM_HEIGHT.clamp(self.film_height);
        self.right_width = RIGHT_WIDTH.clamp(self.right_width);
        self.brush_size = self.brush_size.clamp(0.002, 0.5);
        {
            use lightcraft_engine::catalog::columns::{COLUMNS, DEFAULT_COLUMNS};
            self.metadata_columns.retain(|c| COLUMNS.iter().any(|(id, _)| id == c));
            self.metadata_columns.truncate(8);
            if self.metadata_columns.is_empty() {
                self.metadata_columns = DEFAULT_COLUMNS.iter().map(|c| c.to_string()).collect();
            }
        }
        self.dialog = None;
        self.fullscreen = false;
        if !crate::state::PREVIEW_EDGES.contains(&self.settings.preview_edge) {
            self.settings.preview_edge = AppSettings::default().preview_edge;
        }
        match self.settings.startup_view {
            StartupView::Last => {}
            StartupView::Grid => self.view = ViewMode::PhotoGrid,
            StartupView::Detail => self.view = ViewMode::Detail,
        }
        // Edit sections saved before the Classic panels (light, color, optics…) open their panel
        let mut open: Vec<String> = Vec::new();
        for id in std::mem::take(&mut self.open_sections) {
            let c = crate::panels::edit::classic_section(&id).to_string();
            if !open.contains(&c) {
                open.push(c);
            }
        }
        self.open_sections = open;
        self.sync_module();
        self
    }
}

/// The Edit Keyword Tag dialog's fields.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeywordEdit {
    /// The keyword edited (its full path).
    pub path: String,
    /// Its own level's name.
    pub name: String,
    /// Synonyms, commas between them.
    pub synonyms: String,
    pub include_on_export: bool,
    pub export_containing: bool,
    pub export_synonyms: bool,
    pub person: bool,
}

impl KeywordEdit {
    /// From `keyword.info`'s answer.
    pub fn from_info(v: &serde_json::Value) -> KeywordEdit {
        let path = v["path"].as_str().unwrap_or("").to_string();
        let flag = |k: &str| v[k].as_bool().unwrap_or(true);
        KeywordEdit {
            name: path.rsplit('|').next().unwrap_or(&path).to_string(),
            synonyms: v["synonyms"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).collect::<Vec<_>>().join(", "),
            include_on_export: flag("includeOnExport"),
            export_containing: flag("exportContaining"),
            export_synonyms: flag("exportSynonyms"),
            person: v["person"].as_bool().unwrap_or(false),
            path,
        }
    }

    /// `keyword.edit`'s parameters.
    pub fn params(&self) -> serde_json::Value {
        let synonyms: Vec<&str> = self.synonyms.split(',').map(str::trim).filter(|x| !x.is_empty()).collect();
        serde_json::json!({
            "keyword": self.path,
            "name": self.name.trim(),
            "synonyms": synonyms,
            "includeOnExport": self.include_on_export,
            "exportContaining": self.export_containing,
            "exportSynonyms": self.export_synonyms,
            "person": self.person,
        })
    }
}
