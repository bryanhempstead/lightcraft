//! The Export dialog, laid out like Lightroom Classic's: the title says how many files ("Export
//! One File", "Export 12 Files"), Export To at the top (Hard Drive, Email, CD/DVD, Apple Photos),
//! the preset list on the left (Lightroom Presets, LightCraft Presets, User Presets and the user's
//! folders; a checkbox per preset for exporting with several at once; add. / remove. under it, a
//! right-click menu per preset), and on the right the collapsible sections in Lightroom's order —
//! Export Location, File Naming, Video, File Settings, Image Sizing, Output Sharpening, Metadata,
//! Watermarking, Post-Processing — each header with its one-line summary. LightCraft's extras sit
//! in a Preview section of their own (the photo as it will export, with the estimated file size)
//! and behind more. menus, so the Lightroom part reads the same. Buttons: plug.ins., done.
//! (keeps the settings for next time), cancel., export.

use std::sync::{Arc, Mutex};

use egui::{Align, Layout, Rect, Sense, pos2, vec2};
use lightcraft_engine::export::{
    AfterExport, Conflict, ExportFormat as F, ExportOptions, ExportService, ExportTo, MetadataPolicy, Resize, ResizeMode, ResolutionUnit,
    SharpenAmount, SharpenFor, SizeUnit, StackPlace, Watermark,
};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;

/// Width of the whole dialog body.
pub const DIALOG_W: f32 = 960.0;
/// Width of the preset list.
const PRESETS_W: f32 = 230.0;
/// Width of the right-aligned field labels in the sections.
const LABEL_W: f32 = 132.0;
/// One control height for every row.
const ROW_H: f32 = 24.0;
/// Long edge of the before-export preview.
const PREVIEW_EDGE: u32 = 480;

/// What the dialog's buttons asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    Confirm,
}

/// The Export dialog's state (the fields of [`crate::state::Dialog::Export`]).
pub struct State<'a> {
    pub opts: &'a mut ExportOptions,
    pub full_size: &'a mut bool,
    pub resize: &'a mut Resize,
    pub preset_name: &'a mut String,
    pub limit_kb: &'a mut u32,
    pub dir: &'a mut String,
    pub selected: &'a mut Option<String>,
    pub checked: &'a mut Vec<String>,
    pub album: &'a mut String,
}

/// "Export One File" / "Export 12 Files" (Lightroom's title), for the photos the export takes.
pub fn title(app: &LightcraftApp) -> String {
    match app.session.targets(&json!({})).len() {
        0 | 1 => "Export One File".into(),
        n => format!("Export {n} Files"),
    }
}

// ------------------------------------------------------------------------------- section summaries

/// Export Location's summary: where the files go.
pub fn location_summary(o: &ExportOptions, dir: &str) -> String {
    let base = match o.export_to {
        ExportTo::SpecificFolder if dir.trim().is_empty() => "No folder chosen".to_string(),
        ExportTo::SpecificFolder => short_path(dir),
        ExportTo::ChooseLater => "Folder chosen at export".into(),
        ExportTo::SameFolder => "Same folder as original photo".into(),
        t => t.standard_dir().map(|d| short_path(&d)).unwrap_or_else(|| t.label().to_string()),
    };
    if o.subfolder.trim().is_empty() { base } else { format!("{base}/{}", o.subfolder.trim()) }
}

/// `~/Pictures/Out` for a path under the home folder.
fn short_path(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match p.strip_prefix(home.as_str()).filter(|_| !home.is_empty()) {
        Some(rest) => format!("~{rest}"),
        None => p.to_string(),
    }
}

/// File Settings' summary: `JPEG (90%) / sRGB / 8 bit`.
pub fn file_settings_summary(o: &ExportOptions) -> String {
    let fmt = match o.format {
        F::Jpeg => format!("JPEG ({}%)", o.quality),
        F::Jxl if o.quality >= 100 => "JPEG XL (lossless)".into(),
        F::Jxl => format!("JPEG XL ({}%)", o.quality),
        F::Avif => format!("AVIF ({}%)", o.quality),
        F::Png => "PNG".into(),
        F::Tiff => "TIFF".into(),
        F::Webp => "WebP (lossless)".into(),
        F::Psd => "PSD".into(),
        F::Dng => return "DNG".into(),
        F::Original => return "Original".into(),
    };
    let space = match o.icc_profile.as_deref() {
        Some(x) if x.eq_ignore_ascii_case("colorMatch") => "ColorMatch RGB".to_string(),
        Some(x) => std::path::Path::new(x).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| x.to_string()),
        None => space_label(o.effective_space()).to_string(),
    };
    let depth = match o.effective_depth() {
        lightcraft_engine::export::OutputDepth::U8 if o.format == F::Avif && o.bit_depth == Some(10) => "10 bit",
        lightcraft_engine::export::OutputDepth::U8 => "8 bit",
        lightcraft_engine::export::OutputDepth::U16 if o.format == F::Avif => "10 bit",
        lightcraft_engine::export::OutputDepth::U16 => "16 bit",
        _ => "32 bit",
    };
    let hdr = if o.hdr && o.hdr_output() { " / HDR" } else { "" };
    format!("{fmt} / {space} / {depth}{hdr}")
}

fn space_label(s: lightcraft_engine::export::OutputSpace) -> &'static str {
    use lightcraft_engine::export::OutputSpace as C;
    match s {
        C::Srgb => "sRGB",
        C::DisplayP3 => "Display P3",
        C::AdobeRgb => "AdobeRGB (1998)",
        C::ProPhoto => "ProPhoto RGB",
        C::Rec2020 => "Rec. 2020",
        #[allow(unreachable_patterns)]
        _ => "RGB",
    }
}

/// Image Sizing's summary: `Resize to Long Edge 2048 pixels at 240 ppi`, or just the resolution.
pub fn sizing_summary(o: &ExportOptions, full_size: bool, r: &Resize) -> String {
    if !o.format.is_rendered() {
        return "Original size".into();
    }
    let res = match o.resolution_unit {
        ResolutionUnit::Inch => format!("{} ppi", o.ppi),
        ResolutionUnit::Cm => format!("{:.0} ppcm", f64::from(o.ppi) / 2.54),
    };
    if full_size {
        return res;
    }
    let n = |v: f32| if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    let what = match r.mode {
        ResizeMode::Megapixels => format!("{} megapixels", n(r.value)),
        ResizeMode::Percent => format!("{}%", n(r.value)),
        m if m.is_pair() => format!("{} {} × {} {}", m.label(), n(r.value), n(r.height), r.unit.label()),
        m => format!("{} {} {}", m.label(), n(r.value), r.unit.label()),
    };
    format!("Resize to {what} at {res}")
}

/// Output Sharpening's summary.
pub fn sharpening_summary(o: &ExportOptions) -> String {
    let media = match o.sharpen {
        SharpenFor::None => return "Sharpening Off".into(),
        SharpenFor::Screen => "Screen",
        SharpenFor::Matte => "Matte Paper",
        SharpenFor::Glossy => "Glossy Paper",
    };
    let amount = match o.sharpen_amount {
        SharpenAmount::Low => "Low",
        SharpenAmount::Standard => "Standard",
        SharpenAmount::High => "High",
    };
    format!("{amount} sharpening for {media}")
}

/// Metadata's summary: the Include choice and the options that are on.
pub fn metadata_summary(o: &ExportOptions) -> String {
    let mut s = o.metadata.label().to_string();
    if o.metadata.is_full() {
        for (on, l) in
            [(o.remove_people, "Remove Person Info"), (o.remove_location, "Remove Location Info"), (o.keyword_hierarchy, "Keyword Hierarchy")]
        {
            if on {
                s += ", ";
                s += l;
            }
        }
    }
    s
}

/// Watermarking's summary.
pub fn watermark_summary(app: &LightcraftApp, o: &ExportOptions) -> String {
    match &o.watermark {
        None => "No watermark".into(),
        Some(w) if *w == lightcraft_engine::export::simple_copyright_watermark() => "Simple Copyright Watermark".into(),
        Some(w) => match app.session.watermark_presets.iter().find(|p| p.watermark == *w) {
            Some(p) => p.name.clone(),
            None if !w.image.is_empty() => "Graphic watermark".into(),
            None => format!("“{}”", w.text.lines().next().unwrap_or("").chars().take(30).collect::<String>()),
        },
    }
}

/// Post-Processing's summary.
pub fn after_summary(o: &ExportOptions) -> String {
    match o.after {
        AfterExport::Nothing => "Do nothing".into(),
        AfterExport::ShowInFinder => crate::menus::reveal_label().into(),
        AfterExport::OpenInApp if o.after_app.trim().is_empty() => "Open in the default app".into(),
        AfterExport::OpenInApp => format!("Open in {}", app_name(&o.after_app)),
    }
}

fn app_name(path: &str) -> String {
    std::path::Path::new(path.trim()).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

/// Video's summary.
pub fn video_summary(o: &ExportOptions, videos: usize) -> String {
    use lightcraft_engine::video::{VideoFormat as VF, VideoQuality as VQ};
    if videos == 0 {
        return "No videos in the selection".into();
    }
    if !o.video.include {
        return "Videos are not exported".into();
    }
    match o.video.format {
        VF::Original => "Original video files".into(),
        VF::H264 => format!(
            "H.264, {}",
            match o.video.quality {
                VQ::Max => "Max",
                VQ::High => "High",
                VQ::Medium => "Medium",
                VQ::Low => "Low",
            }
        ),
    }
}

/// The file name the export would give the active photo (File Naming's summary and Example).
fn example_name(app: &LightcraftApp, o: &ExportOptions) -> String {
    let id = app.session.targets(&json!({})).first().copied().or_else(|| app.session.active());
    match id.and_then(|i| app.session.catalog.photo(i)) {
        Some(p) => o.file_name_in(p, 1, app.session.targets(&json!({})).len().max(1)),
        None => format!("Untitled.{}", o.format.extension()),
    }
}

// ------------------------------------------------------------------------------- widgets

/// A right-aligned field label then `add` (one row, one control height).
fn row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(vec2(LABEL_W, ROW_H), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_width(LABEL_W);
            if !label.is_empty() {
                ui.label(egui::RichText::new(crate::i18n::tr(label)).color(t.text_label));
            }
        });
        add(ui)
    })
    .inner
}

/// A popup menu of `options` (`combo:{id}`, items `button:{id}-{i}`); true when it changed.
fn combo<V: PartialEq + Copy>(ui: &mut egui::Ui, id: &str, width: f32, options: &[(V, &str)], value: &mut V) -> bool {
    let shown = options.iter().find(|(v, _)| v == value).map_or("", |(_, l)| l);
    let before = *value;
    let r = egui::ComboBox::from_id_salt(format!("export-{id}")).width(width).selected_text(crate::i18n::tr(shown)).show_ui(ui, |ui| {
        for (i, (v, l)) in options.iter().enumerate() {
            let r = ui.selectable_label(*value == *v, crate::i18n::tr(l));
            crate::widgets::register(ui.ctx(), format!("button:{id}-{i}"), r.rect);
            if r.clicked() {
                *value = *v;
            }
        }
    });
    crate::widgets::register(ui.ctx(), format!("combo:{id}"), r.response.rect);
    *value != before
}

/// A registered checkbox (`check:{id}`).
fn check(ui: &mut egui::Ui, id: &str, on: &mut bool, label: &str) -> bool {
    let r = ui.checkbox(on, crate::i18n::tr(label));
    crate::widgets::register(ui.ctx(), format!("check:{id}"), r.rect);
    r.changed()
}

/// A small number field (`field:{id}`).
fn number<N: egui::emath::Numeric>(ui: &mut egui::Ui, id: &str, v: &mut N, range: std::ops::RangeInclusive<N>, speed: f64, suffix: &str) -> bool {
    let r = ui.add_sized([72.0, ROW_H], egui::DragValue::new(v).range(range).speed(speed).suffix(suffix));
    crate::widgets::register(ui.ctx(), format!("field:{id}"), r.rect);
    r.changed()
}

/// The UI-only toggles that remember a value while off (Rename To, Put in Subfolder, Sharpen For).
fn memo_bool(ui: &egui::Ui, key: &str, init: bool) -> bool {
    ui.data(|d| d.get_temp::<bool>(egui::Id::new(("export-toggle", key)))).unwrap_or(init)
}
fn set_memo_bool(ui: &egui::Ui, key: &str, v: bool) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("export-toggle", key)), v));
}
fn memo_text(ui: &egui::Ui, key: &str) -> Option<String> {
    ui.data(|d| d.get_temp::<String>(egui::Id::new(("export-memo", key))))
}
fn set_memo_text(ui: &egui::Ui, key: &str, v: String) {
    ui.data_mut(|d| d.insert_temp(egui::Id::new(("export-memo", key)), v));
}

/// A section: its header (triangle, title, the summary on the right; click to open or close,
/// `section:export-{id}`) and, when open, `add` below it. Open states are remembered.
fn section(ui: &mut egui::Ui, id: &str, title: &str, summary: &str, default_open: bool, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get(ui.ctx());
    let sid = egui::Id::new(("export-section", id));
    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(sid, default_open));
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    crate::widgets::register(ui.ctx(), format!("section:export-{id}"), r);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    if resp.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_persisted(sid, open));
    }
    let p = ui.painter();
    p.rect_filled(r, 3.0, if resp.hovered() { t.hover } else { t.header });
    let icon = Rect::from_center_size(pos2(r.left() + 14.0, r.center().y), vec2(10.0, 10.0));
    crate::icons::paint(p, icon, if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight }, t.text_label);
    let title_g = p.layout_no_wrap(crate::i18n::tr(title).to_string(), t.semibold(12.5), t.text);
    let title_w = title_g.size().x;
    p.galley(pos2(r.left() + 28.0, r.center().y - title_g.size().y / 2.0), title_g, t.text);
    // the summary, cut to the room left of it
    let room = (r.width() - 28.0 - title_w - 24.0 - 10.0).max(0.0);
    let font = t.font(11.5);
    let full = summary.to_string();
    let fits = |s: &str| p.layout_no_wrap(s.to_string(), font.clone(), t.text_dim).size().x <= room;
    let mut shown = full.clone();
    if !fits(&shown) {
        let chars: Vec<char> = full.chars().collect();
        let mut n = chars.len();
        while n > 0 && !fits(&format!("{}…", chars.iter().take(n).collect::<String>())) {
            n -= 1;
        }
        shown = format!("{}…", chars.iter().take(n).collect::<String>());
    }
    if room > 20.0 {
        p.text(pos2(r.right() - 10.0, r.center().y), egui::Align2::RIGHT_CENTER, shown, font, t.text_dim);
    }
    if resp.hovered() && full.chars().count() > 0 {
        resp.on_hover_text(full);
    }
    if open {
        ui.add_space(4.0);
        egui::Frame::NONE.inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 8 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            add(ui);
        });
    }
    ui.add_space(6.0);
}

// ------------------------------------------------------------------------------- the dialog

/// Draw the dialog's body (`room` = the height its lists may take). Returns what a button asked.
pub fn body(app: &mut LightcraftApp, ui: &mut egui::Ui, s: State<'_>, room: f32) -> Action {
    let mut s = s;
    let t = Tokens::get(ui.ctx());
    ui.set_width(DIALOG_W);
    ui.spacing_mut().interact_size.y = ROW_H;
    let mut action = Action::None;
    // Export To (Hard Drive / Email / CD/DVD / Apple Photos)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(egui::RichText::new(crate::i18n::tr("Export To:")).color(t.text_label));
        let photos = lightcraft_engine::apple_photos::available();
        let services: Vec<(ExportService, &str)> =
            ExportService::ALL.iter().copied().filter(|(x, _)| photos || *x != ExportService::Photos).collect();
        combo(ui, "exportService", 160.0, &services, &mut s.opts.service);
        let note = match s.opts.service {
            ExportService::HardDrive => "",
            ExportService::Email => "Written to a temporary folder and attached to a new Mail message — nothing is sent.",
            ExportService::Disc => "LightCraft burns no discs: the files go to the folder below, ready to burn.",
            ExportService::Photos => "Written, then imported into Apple Photos.",
        };
        if !note.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::tr(note)).color(t.text_dim));
        }
    });
    ui.add_space(8.0);
    let list_h = room.max(240.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        ui.allocate_ui_with_layout(vec2(PRESETS_W, list_h + 40.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(PRESETS_W);
            presets_pane(app, ui, &mut s, list_h);
        });
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("exportScroll")
                .max_height(list_h + 40.0)
                .min_scrolled_height(list_h + 40.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // room for the scroll bar: rows never run under it
                    egui::Frame::NONE.inner_margin(egui::Margin { right: 14, ..Default::default() }).show(ui, |ui| {
                        if s.checked.is_empty() {
                            settings(app, ui, &mut s);
                        } else {
                            several(app, ui, &mut s);
                        }
                    });
                });
        });
    });
    ui.add_space(12.0);
    // bottom row: plug.ins. on the left; done. cancel. export. on the right
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let r = crate::widgets::text_button(ui, "exportPlugins", "plug.ins.", false).on_hover_text(crate::i18n::tr("Plug-in Manager…"));
        let pid = egui::Id::new("export-plugins-open");
        if r.clicked() {
            let open = ui.data(|d| d.get_temp::<bool>(pid)).unwrap_or(false);
            ui.data_mut(|d| d.insert_temp(pid, !open));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let n = if s.checked.is_empty() { String::new() } else { format!(".{}.presets", s.checked.len()) };
            let r = crate::widgets::text_button(ui, "dialogOk", &format!("export{n}."), true);
            if r.clicked() {
                action = Action::Confirm;
            }
            if crate::widgets::text_button(ui, "dialogCancel", "cancel.", false).clicked() {
                action = Action::Close;
            }
            let r = crate::widgets::text_button(ui, "exportDone", "done.", false)
                .on_hover_text(crate::i18n::tr("Keep these settings for next time and close"));
            if r.clicked() {
                remember(app, &s);
                action = Action::Close;
            }
        });
    });
    if ui.data(|d| d.get_temp::<bool>(egui::Id::new("export-plugins-open"))).unwrap_or(false) {
        plugin_manager(ui);
    }
    action
}

/// done.: the dialog's settings become the ones it opens with next time (and Export with
/// Previous's), without exporting.
fn remember(app: &mut LightcraftApp, s: &State<'_>) {
    let mut p = params(s);
    p["dir"] = json!(s.dir.trim());
    app.session.last_export = Some(p);
    let _ = app.session.save_prefs();
}

/// The dialog's choices as `app.export` params (without the folder).
pub fn params(s: &State<'_>) -> Value {
    params_of(s.opts, *s.full_size, s.resize, *s.limit_kb)
}

/// [`params`] from the fields.
pub fn params_of(opts: &ExportOptions, full_size: bool, resize: &Resize, limit_kb: u32) -> Value {
    let o = ExportOptions { resize: (!full_size).then_some(*resize), limit_kb: (limit_kb > 0).then_some(limit_kb), ..opts.clone() };
    o.to_json()
}

/// Plug-in Manager: LightCraft runs no Lightroom plug-ins; what it has instead.
fn plugin_manager(ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(8.0);
    egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new(crate::i18n::tr("Plug-in Manager")).font(t.semibold(12.5)).color(t.text));
        ui.label(egui::RichText::new(crate::i18n::tr(
            "LightCraft has no plug-ins to manage: Lightroom plug-ins (.lrplugin) are Lua programs written for Lightroom's own SDK and can't run here. Built in instead: Export To Hard Drive, Email, CD/DVD (a folder) and Apple Photos; publish services (Library ▸ Publish Services); Export Actions (Post-Processing).",
        ))
        .color(t.text_label));
        let installed = lightroom_plugins();
        if !installed.is_empty() {
            ui.label(egui::RichText::new(format!("Lightroom plug-ins on this Mac (not loaded): {}", installed.join(", "))).color(t.text_dim));
        }
    });
}

/// The Lightroom Classic plug-ins installed on this Mac (names only; never loaded).
fn lightroom_plugins() -> Vec<String> {
    let Some(home) = std::env::var_os("HOME") else { return vec![] };
    let base = std::path::PathBuf::from(home).join("Library/Application Support/Adobe/Lightroom");
    let mut v: Vec<String> = ["Modules", "Export Presets/User Presets"]
        .iter()
        .filter_map(|d| std::fs::read_dir(base.join(d)).ok())
        .flat_map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lrplugin") || x.eq_ignore_ascii_case("lrdevplugin")))
        .filter_map(|p| p.file_stem().map(|n| n.to_string_lossy().to_string()))
        .collect();
    v.sort();
    v.dedup();
    v
}

// ------------------------------------------------------------------------------- the preset list

fn presets_pane(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>, h: f32) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(crate::i18n::tr("Preset:")).color(t.text_label));
    let tree = app.session.export_preset_tree();
    let names: Vec<String> = tree.iter().flat_map(|(_, _, p)| p.clone()).collect();
    let all = app.session.all_export_presets();
    let last_id = egui::Id::new("export-preset-last-check");
    let mut chosen: Option<String> = None;
    let mut menu: Option<(String, String)> = None;
    egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
        ui.set_width(PRESETS_W - 12.0);
        egui::ScrollArea::vertical().id_salt("exportPresetList").max_height(h - 40.0).min_scrolled_height(h - 40.0).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (fi, (folder, builtin, presets)) in tree.iter().enumerate() {
                    let fid = egui::Id::new(("export-preset-folder", folder.as_str()));
                    let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(fid, true));
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
                    crate::widgets::register(ui.ctx(), format!("folder:exportPresets-{fi}"), r);
                    if resp.clicked() {
                        open = !open;
                        ui.data_mut(|d| d.insert_persisted(fid, open));
                    }
                    let p = ui.painter();
                    crate::icons::paint(
                        p,
                        Rect::from_center_size(pos2(r.left() + 7.0, r.center().y), vec2(9.0, 9.0)),
                        if open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight },
                        t.text_label,
                    );
                    crate::icons::paint(
                        p,
                        Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(12.0, 12.0)),
                        crate::icons::Icon::Folder,
                        t.text_label,
                    );
                    p.text(pos2(r.left() + 34.0, r.center().y), egui::Align2::LEFT_CENTER, crate::i18n::tr(folder), t.semibold(12.0), t.text);
                    if !*builtin {
                        let folder = folder.clone();
                        resp.context_menu(|ui| {
                            if ui.button(crate::i18n::tr("New Folder…")).clicked() {
                                menu = Some(("newFolder".into(), String::new()));
                                ui.close();
                            }
                            if ui.button(crate::i18n::tr("Import…")).clicked() {
                                menu = Some(("import".into(), folder.clone()));
                                ui.close();
                            }
                            if folder != lightcraft_engine::export::USER_PRESETS && ui.button(crate::i18n::tr("Remove Folder")).clicked() {
                                menu = Some(("removeFolder".into(), folder.clone()));
                                ui.close();
                            }
                        });
                    }
                    if !open {
                        continue;
                    }
                    for name in presets {
                        let i = names.iter().position(|n| n == name).unwrap_or(0);
                        let user = !all.iter().any(|(p, b)| *b && p.name == *name);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.add_space(14.0);
                            // the checkbox: export with several presets at once (⇧-click: the run)
                            let mut on = s.checked.contains(name);
                            let c = ui.checkbox(&mut on, "");
                            crate::widgets::register(ui.ctx(), format!("check:exportPreset-{i}"), c.rect);
                            if c.changed() {
                                let shift = ui.input(|x| x.modifiers.shift);
                                let last = ui.data(|d| d.get_temp::<usize>(last_id));
                                match (shift, last) {
                                    (true, Some(a)) => {
                                        let (lo, hi) = (a.min(i), a.max(i));
                                        for m in names.iter().take(hi + 1).skip(lo) {
                                            if !s.checked.contains(m) {
                                                s.checked.push(m.clone());
                                            }
                                        }
                                    }
                                    _ if on => s.checked.push(name.clone()),
                                    _ => s.checked.retain(|x| x != name),
                                }
                                ui.data_mut(|d| d.insert_temp(last_id, i));
                            }
                            let sel = s.selected.as_deref() == Some(name.as_str());
                            let r = ui.selectable_label(sel, crate::i18n::builtin_label(name, !user));
                            crate::widgets::register(ui.ctx(), format!("button:exportPreset-{i}"), r.rect);
                            if r.clicked() {
                                chosen = Some(name.clone());
                            }
                            let name = name.clone();
                            r.context_menu(|ui| {
                                if user {
                                    if ui.button(crate::i18n::tr("Update with Current Settings")).clicked() {
                                        menu = Some(("update".into(), name.clone()));
                                        ui.close();
                                    }
                                    if ui.button(crate::i18n::tr("Rename…")).clicked() {
                                        menu = Some(("rename".into(), name.clone()));
                                        ui.close();
                                    }
                                    ui.menu_button(crate::i18n::tr("Move to Folder"), |ui| {
                                        for (f, b, _) in &tree {
                                            if !*b && ui.button(crate::i18n::tr(f)).clicked() {
                                                menu = Some((format!("move:{f}"), name.clone()));
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                                if ui.button(crate::i18n::tr("Export…")).clicked() {
                                    menu = Some(("write".into(), name.clone()));
                                    ui.close();
                                }
                                if user && ui.button(crate::i18n::tr("Delete")).clicked() {
                                    menu = Some(("delete".into(), name.clone()));
                                    ui.close();
                                }
                            });
                        });
                    }
                }
            },
        );
    });
    if let Some(name) = chosen {
        load_preset(app, s, &name);
    }
    if let Some((what, name)) = menu {
        preset_menu(app, ui, s, &what, &name);
    }
    // add. / remove. (+ more.: new folder, import, export all)
    ui.add_space(8.0);
    let adding_id = egui::Id::new("export-preset-adding");
    let mut adding = ui.data(|d| d.get_temp::<bool>(adding_id)).unwrap_or(false);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if crate::widgets::text_button(ui, "exportPresetAdd", "add.", adding)
            .on_hover_text(crate::i18n::tr("Save these settings as a new preset"))
            .clicked()
        {
            adding = !adding;
        }
        if crate::widgets::text_button(ui, "exportPresetRemove", "remove.", false)
            .on_hover_text(crate::i18n::tr("Delete the selected user preset"))
            .clicked()
        {
            let user = s.selected.clone().filter(|n| app.session.export_presets.iter().any(|p| p.name == *n));
            match user {
                Some(name) => preset_menu(app, ui, s, "delete", &name),
                None => app.toast(ui.ctx(), crate::i18n::tr("Pick one of your presets first: built-in presets can't be removed")),
            }
        }
        let r = crate::widgets::text_button(ui, "exportPresetMore", "more.", false);
        egui::Popup::menu(&r).show(|ui| {
            if ui.button(crate::i18n::tr("New Folder…")).clicked() {
                preset_menu(app, ui, s, "newFolder", "");
            }
            if ui.button(crate::i18n::tr("Import Presets…")).clicked() {
                preset_menu(app, ui, s, "import", "");
            }
            if ui.button(crate::i18n::tr("Export My Presets…")).clicked() {
                preset_menu(app, ui, s, "write", "");
            }
        });
    });
    if adding {
        ui.add_space(8.0);
        let folders: Vec<String> = tree.iter().filter(|(_, b, _)| !*b).map(|(f, _, _)| f.clone()).collect();
        let fid = egui::Id::new("export-preset-new-folder");
        let mut folder = ui.data(|d| d.get_temp::<String>(fid)).unwrap_or_else(|| lightcraft_engine::export::USER_PRESETS.to_string());
        let r = ui.add(egui::TextEdit::singleline(s.preset_name).hint_text(crate::i18n::tr("Preset name")).desired_width(PRESETS_W - 4.0));
        crate::widgets::register(ui.ctx(), "field:exportPresetName", r.rect);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let c = egui::ComboBox::from_id_salt("export-new-preset-folder").width(140.0).selected_text(crate::i18n::tr(&folder)).show_ui(ui, |ui| {
                for f in &folders {
                    ui.selectable_value(&mut folder, f.clone(), crate::i18n::tr(f));
                }
            });
            crate::widgets::register(ui.ctx(), "combo:exportPresetFolder", c.response.rect);
            if crate::widgets::text_button(ui, "exportSavePreset", "create.", false).clicked() {
                let name = s.preset_name.trim().to_string();
                if name.is_empty() {
                    app.toast(ui.ctx(), crate::i18n::tr("Type a name for the preset"));
                } else {
                    let mut p = params(s);
                    if s.opts.export_to == ExportTo::SpecificFolder && !s.dir.trim().is_empty() {
                        p["dir"] = json!(s.dir.trim());
                    }
                    match app.run("export.savePreset", json!({"name": name, "params": p, "folder": folder})) {
                        Ok(_) => {
                            app.toast(ui.ctx(), crate::i18n::tr_format!("Saved export preset “{}”", name));
                            *s.selected = Some(name);
                            s.preset_name.clear();
                            adding = false;
                        }
                        Err(e) => app.toast(ui.ctx(), e),
                    }
                }
            }
        });
        ui.data_mut(|d| d.insert_temp(fid, folder));
    }
    ui.data_mut(|d| d.insert_temp(adding_id, adding));
}

/// Load preset `name` into the dialog (it keeps its own folder when it has one).
pub fn load_preset(app: &mut LightcraftApp, s: &mut State<'_>, name: &str) {
    let Ok(params) = app.session.export_params(&json!({"preset": name})) else { return };
    let o = ExportOptions::from_json(&params);
    *s.full_size = o.resize.is_none();
    *s.resize = o.resize.unwrap_or_default();
    *s.limit_kb = o.limit_kb.unwrap_or(0);
    *s.opts = o;
    if let Some(d) = params.get("dir").and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
        *s.dir = d.to_string();
    }
    *s.selected = Some(name.to_string());
}

/// A preset-list menu item: update, rename, move:<folder>, write, delete, newFolder,
/// removeFolder, import.
fn preset_menu(app: &mut LightcraftApp, ui: &egui::Ui, s: &mut State<'_>, what: &str, name: &str) {
    let ctx = ui.ctx().clone();
    let r = match what {
        "update" => {
            let mut p = params(s);
            if s.opts.export_to == ExportTo::SpecificFolder && !s.dir.trim().is_empty() {
                p["dir"] = json!(s.dir.trim());
            }
            app.run("export.savePreset", json!({"name": name, "params": p})).map(|_| format!("Updated “{name}” with the current settings"))
        }
        "rename" => {
            crate::panels::dialogs::prompt(app, "Rename Preset", "New name", name, "export.renamePreset", json!({"name": name}), "to");
            app.ui.dialog_next = app.ui.dialog.clone();
            // the prompt replaces this dialog; the export settings come back from the prefs
            remember(app, s);
            return;
        }
        "delete" => app.run("export.deletePreset", json!({"name": name})).map(|_| {
            if s.selected.as_deref() == Some(name) {
                *s.selected = None;
            }
            s.checked.retain(|x| x != name);
            format!("Deleted export preset “{name}”")
        }),
        "newFolder" => {
            remember(app, s);
            crate::panels::dialogs::prompt(app, "New Folder", "Folder name", "", "export.createPresetFolder", json!({}), "name");
            app.ui.dialog_next = app.ui.dialog.clone();
            return;
        }
        "removeFolder" => {
            app.run("export.deletePresetFolder", json!({"name": name})).map(|_| format!("Removed “{name}”: its presets are in User Presets"))
        }
        "import" => {
            let Some(path) = app.services.pick_preset_files.as_mut().and_then(|f| f().into_iter().next()) else { return };
            let mut q = json!({"path": path});
            if !name.is_empty() {
                q["folder"] = json!(name);
            }
            app.run("export.importPresets", q).map(|v| {
                let n = v["imported"].as_array().map_or(0, Vec::len);
                let notes = v["notes"].as_array().map_or(0, Vec::len);
                if notes > 0 {
                    format!("Imported {n} preset(s); {notes} setting(s) couldn't be carried over")
                } else {
                    format!("Imported {n} preset(s)")
                }
            })
        }
        "write" => {
            let suggested = if name.is_empty() { "Export Presets.json".to_string() } else { format!("{name}.json") };
            let Some(path) = app.services.save_preset_file.as_mut().and_then(|f| f(&suggested)) else { return };
            let mut q = json!({"path": path});
            if !name.is_empty() {
                q["names"] = json!([name]);
            }
            app.run("export.writePresets", q).map(|v| format!("Wrote {}", v["path"].as_str().unwrap_or("")))
        }
        w if w.starts_with("move:") => {
            let folder = w.trim_start_matches("move:");
            app.run("export.movePreset", json!({"name": name, "folder": folder})).map(|_| format!("Moved “{name}” to {folder}"))
        }
        _ => return,
    };
    match r {
        Ok(m) => app.toast(&ctx, m),
        Err(e) => app.toast(&ctx, e),
    }
}

// ------------------------------------------------------------------------------- several presets

/// With presets checked (Lightroom's multi-batch export): each exports with its own settings into
/// a subfolder named after it under one parent folder.
fn several(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(format!(
            "{} presets checked: the photos export once with each, into a folder named after the preset under the parent folder. Clear the checkboxes to see or change a preset's settings.",
            s.checked.len()
        ))
        .color(t.text_label),
    );
    ui.add_space(8.0);
    row(ui, "Parent folder", |ui| folder_field(app, ui, s.dir));
    ui.add_space(8.0);
    let order: Vec<String> = app.session.export_preset_tree().into_iter().flat_map(|(_, _, p)| p).filter(|n| s.checked.contains(n)).collect();
    for name in &order {
        let Ok(p) = app.session.export_params(&json!({"preset": name})) else { continue };
        let o = ExportOptions::from_json(&p);
        let size = o.resize.as_ref().map_or_else(|| "full size".to_string(), |r| sizing_summary(&o, false, r));
        ui.label(egui::RichText::new(format!("{name}  ·  {}  ·  {size}", file_settings_summary(&o))).color(t.text_dim));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if crate::widgets::text_button(ui, "exportBatchNone", "none.", false).clicked() {
            s.checked.clear();
        }
        if crate::widgets::text_button(ui, "exportBatchAll", "all.", false).clicked() {
            *s.checked = app.session.all_export_presets().into_iter().map(|(p, _)| p.name).collect();
        }
    });
}

/// The folder field with its choose. button.
fn folder_field(app: &mut LightcraftApp, ui: &mut egui::Ui, dir: &mut String) {
    ui.spacing_mut().item_spacing.x = 8.0;
    let w = (ui.available_width() - 80.0).max(120.0);
    let r = ui.add_sized([w, ROW_H], egui::TextEdit::singleline(dir).hint_text(crate::i18n::tr("Folder")));
    crate::widgets::register(ui.ctx(), "field:exportFolder", r.rect);
    if app.services.pick_folder.is_some()
        && crate::widgets::text_button(ui, "exportChooseFolder", "choose.", false).clicked()
        && let Some(d) = app.services.pick_folder.as_mut().and_then(|pick| pick())
    {
        *dir = d;
    }
}

// ------------------------------------------------------------------------------- the sections

fn settings(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    let rendered = s.opts.format.is_rendered();
    let videos = app
        .session
        .targets(&json!({}))
        .iter()
        .filter(|id| app.session.catalog.photo(**id).is_some_and(|p| p.kind == lightcraft_catalog::MediaKind::Video))
        .count();
    let loc = location_summary(s.opts, s.dir);
    section(ui, "location", "Export Location", &loc, true, |ui| location(app, ui, s));
    let example = example_name(app, s.opts);
    section(ui, "naming", "File Naming", &example, true, |ui| naming(app, ui, s, &example));
    section(ui, "video", "Video", &video_summary(s.opts, videos), videos > 0, |ui| video(ui, s.opts, videos));
    let mut fs = file_settings_summary(s.opts);
    if let Some(b) = estimate(app, ui.ctx(), s).and_then(|p| p.estimated_bytes) {
        fs = format!("{fs} · ≈ {}", lightcraft_engine::export::human_bytes(b));
    }
    section(ui, "file", "File Settings", &fs, true, |ui| file_settings(app, ui, s));
    // (a lossy DNG takes the Image Sizing size)
    if rendered || (s.opts.format == F::Dng && s.opts.dng_lossy) {
        let summary = if rendered {
            sizing_summary(s.opts, *s.full_size, s.resize)
        } else {
            sizing_summary(&ExportOptions { format: F::Jpeg, ..s.opts.clone() }, *s.full_size, s.resize)
        };
        section(ui, "sizing", "Image Sizing", &summary, true, |ui| sizing(app, ui, s));
    }
    if rendered {
        section(ui, "sharpening", "Output Sharpening", &sharpening_summary(s.opts), false, |ui| sharpening(ui, s.opts));
    }
    if rendered || s.opts.format == F::Original {
        section(ui, "metadata", "Metadata", &metadata_summary(s.opts), false, |ui| metadata(ui, s.opts));
    }
    if rendered {
        let wm = watermark_summary(app, s.opts);
        section(ui, "watermark", "Watermarking", &wm, false, |ui| watermarking(app, ui, s.opts));
    }
    section(ui, "after", "Post-Processing", &after_summary(s.opts), false, |ui| post_processing(app, ui, s.opts));
    let pv = match estimate(app, ui.ctx(), s) {
        Some(p) => format!(
            "{} × {} px{}",
            p.size.0,
            p.size.1,
            p.estimated_bytes.map(|b| format!(" · ≈ {} each", lightcraft_engine::export::human_bytes(b))).unwrap_or_default()
        ),
        None => "As it will export".into(),
    };
    section(ui, "preview", "Preview (LightCraft)", &pv, false, |ui| preview(app, ui, s));
}

fn location(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    let t = Tokens::get(ui.ctx());
    let choices: Vec<(ExportTo, &str)> = ExportTo::ALL.to_vec();
    row(ui, "Export To:", |ui| combo(ui, "exportTo", 300.0, &choices, &mut s.opts.export_to));
    match s.opts.export_to {
        ExportTo::SpecificFolder => {
            row(ui, "Folder:", |ui| folder_field(app, ui, s.dir));
        }
        ExportTo::ChooseLater => {
            row(ui, "", |ui| ui.label(egui::RichText::new(crate::i18n::tr("The folder is asked for when you click export.")).color(t.text_dim)));
        }
        ExportTo::SameFolder => {
            row(ui, "", |ui| {
                ui.label(egui::RichText::new(crate::i18n::tr("Each file next to its original (originals are never written over).")).color(t.text_dim))
            });
        }
        to => {
            let d = to.standard_dir().unwrap_or_default();
            row(ui, "Folder:", |ui| ui.label(egui::RichText::new(short_path(&d)).color(t.text_label)));
        }
    }
    // Put in Subfolder: the name is kept while it's off
    let mut sub_on = memo_bool(ui, "subfolder", !s.opts.subfolder.trim().is_empty());
    row(ui, "", |ui| {
        if check(ui, "exportPutInSubfolder", &mut sub_on, "Put in Subfolder:") {
            if sub_on {
                s.opts.subfolder = memo_text(ui, "subfolder").filter(|x| !x.trim().is_empty()).unwrap_or_else(|| "Exports".into());
            } else {
                set_memo_text(ui, "subfolder", s.opts.subfolder.clone());
                s.opts.subfolder.clear();
            }
        }
        if sub_on {
            let r = ui.add_sized([(ui.available_width()).max(120.0), ROW_H], egui::TextEdit::singleline(&mut s.opts.subfolder));
            crate::widgets::register(ui.ctx(), "field:exportSubfolder", r.rect);
        }
    });
    set_memo_bool(ui, "subfolder", sub_on);
    row(ui, "", |ui| {
        check(ui, "exportAddToCatalog", &mut s.opts.add_to_catalog, "Add to This Catalog");
        if s.opts.add_to_catalog {
            let mut stack = s.opts.add_to_stack.is_some();
            if check(ui, "exportAddToStack", &mut stack, "Add to Stack:") {
                s.opts.add_to_stack = stack.then_some(StackPlace::Below);
            }
            if let Some(place) = s.opts.add_to_stack.as_mut() {
                combo(ui, "exportStackPlace", 140.0, &[(StackPlace::Below, "Below Original"), (StackPlace::Above, "Above Original")], place);
            }
        }
    });
    row(ui, "Existing Files:", |ui| combo(ui, "exportConflict", 300.0, &Conflict::ALL, &mut s.opts.conflict));
}

fn naming(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>, example: &str) {
    let t = Tokens::get(ui.ctx());
    let mut on = memo_bool(ui, "rename", s.opts.naming.trim() != "{name}");
    row(ui, "", |ui| {
        if check(ui, "exportRenameTo", &mut on, "Rename To:") {
            if on {
                s.opts.naming =
                    memo_text(ui, "naming").filter(|x| x.trim() != "{name}" && !x.trim().is_empty()).unwrap_or_else(|| "{text}-{seq}".into());
            } else {
                set_memo_text(ui, "naming", s.opts.naming.clone());
                s.opts.naming = "{name}".into();
            }
        }
    });
    set_memo_bool(ui, "rename", on);
    if on {
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W - 70.0);
            crate::panels::dialogs::naming_template_row(app, ui, "export", &mut s.opts.naming);
        });
        let naming_id = egui::Id::new("export-naming");
        let tags_open = row(ui, "Template:", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let w = (ui.available_width() - 90.0).max(160.0);
            ui.add_sized([w, ROW_H], egui::TextEdit::singleline(&mut s.opts.naming).id(naming_id).hint_text("{name}-{seq}  ·  {date}  ·  {title}"));
            crate::import::tag_toggle(ui, "exportNaming")
        });
        if tags_open {
            crate::import::tag_help(ui, "exportNaming", &mut s.opts.naming, naming_id);
        }
        crate::import::unknown_tags_warning(ui, &s.opts.naming);
        row(ui, "Custom Text:", |ui| {
            let r = ui.add_sized([200.0, ROW_H], egui::TextEdit::singleline(&mut s.opts.custom_text));
            crate::widgets::register(ui.ctx(), "field:exportText", r.rect);
            ui.add_space(8.0);
            ui.label(egui::RichText::new(crate::i18n::tr("Start Number:")).color(t.text_label));
            number(ui, "exportStartNumber", &mut s.opts.start_number, 0..=999_999, 1.0, "");
        });
    }
    use lightcraft_engine::rename::ExtCase as X;
    row(ui, "Example:", |ui| {
        ui.label(egui::RichText::new(example).color(t.text));
        ui.add_space(16.0);
        ui.label(egui::RichText::new(crate::i18n::tr("Extensions:")).color(t.text_label));
        combo(ui, "exportExtCase", 120.0, &[(X::Lower, "Lowercase"), (X::Upper, "Uppercase"), (X::Keep, "As is")], &mut s.opts.ext_case);
    });
}

fn video(ui: &mut egui::Ui, o: &mut ExportOptions, videos: usize) {
    use lightcraft_engine::video::{VideoFormat as VF, VideoQuality as VQ};
    let t = Tokens::get(ui.ctx());
    row(ui, "", |ui| check(ui, "exportVideoInclude", &mut o.video.include, "Include Video Files"));
    if o.video.include {
        row(ui, "Video Format:", |ui| combo(ui, "exportVideoFormat", 180.0, &[(VF::H264, "H.264"), (VF::Original, "Original")], &mut o.video.format));
        if o.video.format == VF::H264 {
            row(ui, "Quality:", |ui| {
                combo(
                    ui,
                    "exportVideoQuality",
                    180.0,
                    &[(VQ::Max, "Max"), (VQ::High, "High"), (VQ::Medium, "Medium"), (VQ::Low, "Low")],
                    &mut o.video.quality,
                )
            });
        }
        let note = match o.video.format {
            VF::H264 => {
                "H.264 .mp4: trimmed, with the edits Classic applies to video (white balance, tone, curve, vibrance, saturation, B&W). No DPX."
            }
            VF::Original => "The video files as they are: no trim, no edits.",
        };
        row(ui, "", |ui| ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(note)).color(t.text_dim)).wrap()));
    }
    if videos == 0 {
        row(ui, "", |ui| ui.label(egui::RichText::new(crate::i18n::tr("The selection has no videos.")).color(t.text_dim)));
    }
}

fn file_settings(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    use lightcraft_engine::export::OutputSpace as C;
    let t = Tokens::get(ui.ctx());
    let before = s.opts.format;
    let formats = [
        (F::Jpeg, "JPEG"),
        (F::Jxl, "JPEG XL"),
        (F::Avif, "AVIF"),
        (F::Psd, "PSD"),
        (F::Tiff, "TIFF"),
        (F::Png, "PNG"),
        (F::Dng, "DNG"),
        (F::Original, "Original"),
        (F::Webp, "WebP (LightCraft)"),
    ];
    row(ui, "Image Format:", |ui| combo(ui, "exportFormat", 180.0, &formats, &mut s.opts.format));
    if s.opts.format != before {
        // each format starts at its own default depth (TIFF / PSD 16-bit, others 8-bit)
        s.opts.bit_depth = None;
    }
    let o = &mut *s.opts;
    match o.format {
        F::Dng => {
            use lightcraft_engine::export::DngCompression as Z;
            row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(crate::i18n::tr("Raw photos as DNG, with the edits embedded. Size, color and output options don't apply."))
                        .color(t.text_dim),
                )
            });
            row(ui, "Compression:", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, (v, l)) in [(Z::Lossless, "Lossless"), (Z::Deflate, "ZIP"), (Z::Uncompressed, "None")].iter().enumerate() {
                    if crate::widgets::text_button(ui, &format!("exportDngCompression-{i}"), l, o.dng_compression == *v).clicked() {
                        o.dng_compression = *v;
                    }
                }
            });
            // LR-EXP-DNGOPT: Lightroom's JPEG Preview and Use Lossy Compression
            use lightcraft_engine::export::DngPreview as Pv;
            row(ui, "JPEG Preview:", |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, (v, l)) in [(Pv::None, "none."), (Pv::Medium, "med."), (Pv::Full, "full.")].iter().enumerate() {
                    if crate::widgets::text_button(ui, &format!("exportDngPreview-{i}"), l, o.dng_preview == *v).clicked() {
                        o.dng_preview = *v;
                    }
                }
            });
            row(ui, "", |ui| check(ui, "exportDngLossy", &mut o.dng_lossy, "Use Lossy Compression"));
            let note = if (o.dng_preview != Pv::None || o.dng_lossy) && !lightcraft_engine::dng_sdk_available() {
                "The preview and lossy DNG need Adobe's DNG SDK, which this build doesn't have: written without."
            } else if o.dng_preview != Pv::None && o.dng_compression == Z::Deflate && !o.dng_lossy {
                "With a JPEG preview the raw data is stored as lossless JPEG (the DNG SDK has no ZIP for it)."
            } else {
                ""
            };
            if !note.is_empty() {
                row(ui, "", |ui| ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tr(note)).color(t.text_dim)).wrap()));
            }
            return;
        }
        F::Original => {
            row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(crate::i18n::tr("The original files, unchanged, each with an XMP sidecar holding its edits."))
                        .color(t.text_dim),
                )
            });
            return;
        }
        _ => {}
    }
    if matches!(o.format, F::Jpeg | F::Avif | F::Jxl) {
        let mut lossless = o.format == F::Jxl && o.quality >= 100;
        if o.format == F::Jxl {
            row(ui, "", |ui| {
                if check(ui, "exportJxlLossless", &mut lossless, "Lossless Quality") {
                    o.quality = if lossless { 100 } else { 90 };
                }
            });
        }
        if !lossless {
            row(ui, "Quality:", |ui| {
                ui.spacing_mut().slider_width = 260.0;
                let mut q = o.quality;
                let r = ui.add(egui::Slider::new(&mut q, 1..=100));
                crate::widgets::register(ui.ctx(), "field:exportQuality", r.rect);
                o.quality = q.clamp(1, 100);
            });
        }
    }
    // Color Space: Lightroom's list, then Other… (ColorMatch RGB, the profiles on this Mac)
    let hdr_rec2020 = o.hdr && o.hdr_output() && o.effective_space() == C::Rec2020 && o.format != F::Tiff;
    if hdr_rec2020 {
        let curve = if o.hdr_transfer == lightcraft_engine::export::HdrCurve::Hlg { "HLG" } else { "PQ" };
        row(ui, "Color Space:", |ui| ui.label(egui::RichText::new(format!("Rec. 2020 {curve} (HDR)")).color(t.text_label)));
    } else if o.format == F::Avif && !o.hdr {
        row(ui, "Color Space:", |ui| ui.label(egui::RichText::new(crate::i18n::tr("sRGB (AVIF)")).color(t.text_label)));
    } else {
        row(ui, "Color Space:", |ui| color_space(ui, o));
    }
    let depths = ExportOptions::bit_depths(o.format);
    if depths.len() > 1 {
        let mut bd = o.bit_depth.filter(|b| depths.iter().any(|d| d.0 == *b)).unwrap_or(depths[0].0);
        let labelled: Vec<(u8, String)> =
            depths.iter().map(|(b, l)| (*b, format!("{} bits/component", l.trim_end_matches("-bit").trim_end_matches("-bit float")))).collect();
        let opts: Vec<(u8, &str)> = labelled.iter().map(|(b, l)| (*b, l.as_str())).collect();
        row(ui, "Bit Depth:", |ui| combo(ui, "exportBitDepth", 200.0, &opts, &mut bd));
        o.bit_depth = Some(bd);
    }
    if o.format == F::Tiff {
        use lightcraft_engine::export::TiffCompression as Z;
        row(ui, "Compression:", |ui| {
            combo(ui, "exportTiffCompression", 160.0, &[(Z::None, "None"), (Z::Lzw, "LZW"), (Z::Deflate, "ZIP")], &mut o.tiff_compression)
        });
    }
    if matches!(o.format, F::Jpeg | F::Tiff | F::Avif | F::Png | F::Jxl | F::Psd) {
        let tip = "Photos edited in HDR (Lightroom's HDR Output): JPEG with an HDR gain map; AVIF, PNG and JPEG XL as Rec. 2020 PQ or HLG, AVIF (macOS) and JPEG XL with Maximize Compatibility as SDR + gain map; TIFF needs 32-bit float. Photos without an HDR edit export as usual";
        row(ui, "", |ui| {
            let r = ui.checkbox(&mut o.hdr, crate::i18n::tr("HDR Output")).on_hover_text(crate::i18n::tr(tip));
            crate::widgets::register(ui.ctx(), "check:exportHdr", r.rect);
            if o.hdr && (o.format == F::Jxl || (o.format == F::Avif && lightcraft_engine::imageio::available())) {
                check(ui, "exportHdrCompat", &mut o.hdr_compat, "Maximize Compatibility");
            }
        });
        if o.hdr && !o.hdr_output() {
            row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(crate::i18n::tr(if o.format == F::Psd {
                        "HDR PSD isn't written yet: these export SDR."
                    } else {
                        "HDR TIFF needs 32-bit float."
                    }))
                    .color(t.text_dim),
                )
            });
        }
        if o.hdr && matches!(o.format, F::Avif | F::Png | F::Jxl) && !o.gain_map_output() {
            use lightcraft_engine::export::HdrCurve as H;
            row(ui, "HDR Curve:", |ui| combo(ui, "exportHdrCurve", 120.0, &[(H::Pq, "PQ"), (H::Hlg, "HLG")], &mut o.hdr_transfer));
        }
    }
    if o.format == F::Jpeg {
        let mut limit = *s.limit_kb > 0;
        row(ui, "", |ui| {
            if check(ui, "exportLimitSize", &mut limit, "Limit File Size To:") {
                *s.limit_kb = if limit {
                    memo_text(ui, "limitKb").and_then(|x| x.parse().ok()).unwrap_or(100)
                } else {
                    set_memo_text(ui, "limitKb", s.limit_kb.to_string());
                    0
                };
            }
            if limit {
                number(ui, "exportLimitKb", s.limit_kb, 1..=10_000_000, 10.0, " K");
            }
        });
    }
    // the estimate (LightCraft)
    if let Some(p) = estimate(app, ui.ctx(), s).filter(|p| p.estimated_bytes.is_some()) {
        let b = p.estimated_bytes.unwrap_or(0);
        row(ui, "Estimated size:", |ui| {
            ui.label(
                egui::RichText::new(format!("≈ {} per photo ({} × {} px)", lightcraft_engine::export::human_bytes(b), p.size.0, p.size.1))
                    .color(t.text_label),
            )
        });
    }
}

/// The Color Space popup: sRGB, Display P3, AdobeRGB (1998), ProPhoto RGB, Rec. 2020, then
/// Other: ColorMatch RGB and the RGB profiles installed on this Mac (E08).
fn color_space(ui: &mut egui::Ui, o: &mut ExportOptions) {
    use lightcraft_engine::export::OutputSpace as C;
    let shown = match o.icc_profile.as_deref() {
        None => space_label(o.color_space).to_string(),
        Some(x) if x.eq_ignore_ascii_case("colorMatch") => "ColorMatch RGB".to_string(),
        Some(x) => std::path::Path::new(x).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| x.to_string()),
    };
    let other_ok = matches!(o.format, F::Jpeg | F::Png | F::Tiff | F::Psd);
    let key = egui::Id::new("export-installed-profiles");
    let r = egui::ComboBox::from_id_salt("export-color-space").width(240.0).selected_text(shown).show_ui(ui, |ui| {
        for (i, (c, l)) in [
            (C::Srgb, "sRGB"),
            (C::DisplayP3, "Display P3"),
            (C::AdobeRgb, "AdobeRGB (1998)"),
            (C::ProPhoto, "ProPhoto RGB"),
            (C::Rec2020, "Rec. 2020"),
        ]
        .iter()
        .enumerate()
        {
            let r = ui.selectable_label(o.icc_profile.is_none() && o.color_space == *c, *l);
            crate::widgets::register(ui.ctx(), format!("button:exportColorSpace-{i}"), r.rect);
            if r.clicked() {
                o.color_space = *c;
                o.icc_profile = None;
            }
        }
        if other_ok {
            ui.separator();
            ui.label(crate::i18n::tr("Other…"));
            let r = ui.selectable_label(o.icc_profile.as_deref() == Some("colorMatch"), "ColorMatch RGB");
            crate::widgets::register(ui.ctx(), "button:exportColorSpace-colorMatch", r.rect);
            if r.clicked() {
                o.icc_profile = Some("colorMatch".into());
            }
            let installed: Vec<String> = match ui.data(|d| d.get_temp::<Vec<String>>(key)) {
                Some(v) => v,
                None => {
                    let v = lightcraft_engine::export::installed_profiles();
                    ui.data_mut(|d| d.insert_temp(key, v.clone()));
                    v
                }
            };
            for path in &installed {
                let name = std::path::Path::new(path).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if ui.selectable_label(o.icc_profile.as_deref() == Some(path.as_str()), name).clicked() {
                    o.icc_profile = Some(path.clone());
                }
            }
        }
    });
    crate::widgets::register(ui.ctx(), "combo:exportColorSpace", r.response.rect);
    crate::widgets::register(ui.ctx(), "combo:exportIccProfile", r.response.rect);
}

fn sizing(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    let t = Tokens::get(ui.ctx());
    let r = &mut *s.resize;
    let mut fit = !*s.full_size;
    row(ui, "", |ui| {
        if check(ui, "exportResizeToFit", &mut fit, "Resize to Fit:") {
            *s.full_size = !fit;
        }
        let before = r.mode;
        combo(ui, "exportResizeMode", 170.0, &ResizeMode::ALL, &mut r.mode);
        if r.mode != before {
            // a sensible value for the new unit
            match r.mode {
                ResizeMode::Megapixels => r.value = 12.0,
                ResizeMode::Percent => r.value = 50.0,
                _ if !before.is_length() => {
                    r.value = 2048.0;
                    r.unit = SizeUnit::Pixels;
                }
                _ => {}
            }
            if r.mode.is_pair() && r.height <= 0.0 {
                r.height = r.value;
            }
        }
        check(ui, "exportDontEnlarge", &mut r.dont_enlarge, "Don't Enlarge");
    });
    let (max, speed) = match r.unit {
        SizeUnit::Pixels => (65_000.0, 4.0),
        _ => (500.0, 0.1),
    };
    match r.mode {
        ResizeMode::WidthHeight | ResizeMode::Dimensions => {
            row(ui, if r.mode == ResizeMode::WidthHeight { "W:" } else { "Dimensions:" }, |ui| {
                number(ui, "exportWidth", &mut r.value, 0.0..=max, speed, "");
                ui.label(if r.mode == ResizeMode::WidthHeight { "H:" } else { "×" });
                number(ui, "exportHeight", &mut r.height, 0.0..=max, speed, "");
                combo(ui, "exportSizeUnit", 90.0, &SizeUnit::ALL, &mut r.unit);
            });
        }
        ResizeMode::Megapixels => {
            row(ui, "Megapixels:", |ui| number(ui, "exportMegapixels", &mut r.value, 0.01..=500.0, 0.1, " MP"));
        }
        ResizeMode::Percent => {
            row(ui, "Percentage:", |ui| number(ui, "exportPercent", &mut r.value, 1.0..=400.0, 1.0, " %"));
        }
        m => {
            row(ui, m.label(), |ui| {
                number(ui, "exportEdge", &mut r.value, 1.0..=max, speed, "");
                combo(ui, "exportSizeUnit", 90.0, &SizeUnit::ALL, &mut r.unit);
            });
        }
    }
    // Resolution: shown in pixels per inch or per cm; stored per inch
    row(ui, "Resolution:", |ui| {
        let mut shown = match s.opts.resolution_unit {
            ResolutionUnit::Inch => f64::from(s.opts.ppi),
            ResolutionUnit::Cm => (f64::from(s.opts.ppi) / 2.54 * 10.0).round() / 10.0,
        };
        if number(ui, "exportResolution", &mut shown, 1.0..=10_000.0, 1.0, "") {
            let ppi = match s.opts.resolution_unit {
                ResolutionUnit::Inch => shown,
                ResolutionUnit::Cm => shown * 2.54,
            };
            s.opts.ppi = ppi.round().clamp(1.0, f64::from(u16::MAX)) as u16;
        }
        combo(
            ui,
            "exportResolutionUnit",
            170.0,
            &[(ResolutionUnit::Inch, "pixels per inch"), (ResolutionUnit::Cm, "pixels per cm")],
            &mut s.opts.resolution_unit,
        );
    });
    // what the active photo comes out at (LightCraft)
    let id = app.session.targets(&json!({})).first().copied();
    if let Some(p) = id.and_then(|i| app.session.catalog.photo(i)) {
        let o = ExportOptions { resize: (!*s.full_size).then_some(*s.resize), ..s.opts.clone() };
        let (w, h) = lightcraft_engine::export::output_size(p, &o);
        let inch = |px: usize| px as f64 / f64::from(s.opts.ppi.max(1));
        row(ui, "", |ui| {
            ui.label(
                egui::RichText::new(format!("{}: {w} × {h} px ({:.1} × {:.1} in at {} ppi)", p.file_name, inch(w), inch(h), s.opts.ppi))
                    .color(t.text_dim),
            )
        });
    }
}

fn sharpening(ui: &mut egui::Ui, o: &mut ExportOptions) {
    let mut on = o.sharpen != SharpenFor::None;
    row(ui, "", |ui| {
        if check(ui, "exportSharpenFor", &mut on, "Sharpen For:") {
            o.sharpen = if on {
                match memo_text(ui, "sharpen").as_deref() {
                    Some("matte") => SharpenFor::Matte,
                    Some("glossy") => SharpenFor::Glossy,
                    _ => SharpenFor::Screen,
                }
            } else {
                set_memo_text(
                    ui,
                    "sharpen",
                    match o.sharpen {
                        SharpenFor::Matte => "matte",
                        SharpenFor::Glossy => "glossy",
                        _ => "screen",
                    }
                    .into(),
                );
                SharpenFor::None
            };
        }
        if on {
            combo(
                ui,
                "exportSharpen",
                150.0,
                &[(SharpenFor::Screen, "Screen"), (SharpenFor::Matte, "Matte Paper"), (SharpenFor::Glossy, "Glossy Paper")],
                &mut o.sharpen,
            );
            ui.add_space(8.0);
            ui.label(crate::i18n::tr("Amount:"));
            combo(
                ui,
                "exportSharpenAmount",
                120.0,
                &[(SharpenAmount::Low, "Low"), (SharpenAmount::Standard, "Standard"), (SharpenAmount::High, "High")],
                &mut o.sharpen_amount,
            );
        }
    });
}

fn metadata(ui: &mut egui::Ui, o: &mut ExportOptions) {
    let t = Tokens::get(ui.ctx());
    if o.format == F::Original {
        row(ui, "", |ui| {
            ui.label(
                egui::RichText::new(crate::i18n::tr("Original files keep their own metadata; the edits and your metadata go in the XMP sidecar."))
                    .color(t.text_dim),
            )
        });
        return;
    }
    row(ui, "Include:", |ui| combo(ui, "exportMetadata", 280.0, &MetadataPolicy::ALL, &mut o.metadata));
    if o.metadata.is_full() {
        row(ui, "", |ui| check(ui, "exportRemovePeople", &mut o.remove_people, "Remove Person Info"));
        row(ui, "", |ui| check(ui, "exportRemoveLocation", &mut o.remove_location, "Remove Location Info"));
        row(ui, "", |ui| check(ui, "exportKeywordHierarchy", &mut o.keyword_hierarchy, "Write Keywords as Lightroom Hierarchy"));
    }
    if o.metadata == MetadataPolicy::AllExceptCameraRaw {
        row(ui, "", |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::i18n::tr(
                        "LightCraft never writes its develop settings into exported files, so this is the same as All Metadata.",
                    ))
                    .color(t.text_dim),
                )
                .wrap(),
            )
        });
    }
}

fn watermarking(app: &mut LightcraftApp, ui: &mut egui::Ui, o: &mut ExportOptions) {
    use lightcraft_engine::export::Anchor as P;
    let t = Tokens::get(ui.ctx());
    let simple = lightcraft_engine::export::simple_copyright_watermark();
    let mut on = o.watermark.is_some();
    let edit_id = egui::Id::new("export-watermark-edit");
    let mut editing = ui.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    row(ui, "", |ui| {
        if check(ui, "exportWatermark", &mut on, "Watermark:") {
            o.watermark = on.then(|| simple.clone());
        }
        if on {
            let saved = app.session.watermark_presets.clone();
            let shown = watermark_summary(app, o);
            let r = egui::ComboBox::from_id_salt("export-watermark-preset").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                let r = ui.selectable_label(o.watermark.as_ref() == Some(&simple), crate::i18n::tr("Simple Copyright Watermark"));
                crate::widgets::register(ui.ctx(), "button:watermarkPreset-simple", r.rect);
                if r.clicked() {
                    o.watermark = Some(simple.clone());
                }
                for (i, w) in saved.iter().enumerate() {
                    let r = ui.selectable_label(o.watermark.as_ref() == Some(&w.watermark), &w.name);
                    crate::widgets::register(ui.ctx(), format!("button:watermarkPreset-{i}"), r.rect);
                    if r.clicked() {
                        o.watermark = Some(w.watermark.clone());
                    }
                }
                ui.separator();
                if ui.selectable_label(false, crate::i18n::tr("Edit Watermarks…")).clicked() {
                    editing = true;
                }
            });
            crate::widgets::register(ui.ctx(), "button:watermarkPresets", r.response.rect);
            if crate::widgets::text_button(ui, "exportWatermarkEdit", "edit.", editing).clicked() {
                editing = !editing;
            }
        }
    });
    if o.watermark.as_ref() == Some(&simple) && !editing {
        row(ui, "", |ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Each photo's Copyright field; photos without one get no watermark.")).color(t.text_dim))
        });
    }
    ui.data_mut(|d| d.insert_temp(edit_id, editing));
    let Some(wm) = o.watermark.as_mut().filter(|_| editing) else { return };
    // the Watermark Editor, inline: text or graphic, then the effects
    egui::Frame::NONE.fill(t.inset).corner_radius(4.0).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let mut graphic = !wm.image.is_empty() || ui.data(|d| d.get_temp::<bool>(egui::Id::new("wm-graphic"))).unwrap_or(false);
        let before = graphic;
        row(ui, "Watermark Style:", |ui| combo(ui, "exportWmStyle", 120.0, &[(false, "Text"), (true, "Graphic")], &mut graphic));
        if graphic != before {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("wm-graphic"), graphic));
            if !graphic {
                wm.image.clear();
            }
        }
        if graphic {
            row(ui, "Image:", |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let w = (ui.available_width() - 80.0).max(120.0);
                ui.add_sized([w, ROW_H], egui::TextEdit::singleline(&mut wm.image).hint_text("logo.png"));
                if app.services.pick_files.is_some()
                    && crate::widgets::text_button(ui, "exportWmChoose", "choose.", false).clicked()
                    && let Some(f) = app.services.pick_files.as_mut().and_then(|pick| pick().into_iter().next())
                {
                    wm.image = f;
                }
            });
            row(ui, "Width:", |ui| {
                let mut pct = wm.image_width * 100.0;
                if number(ui, "exportWmImageWidth", &mut pct, 1.0..=100.0, 0.5, " % of photo") {
                    wm.image_width = (pct / 100.0).clamp(0.01, 1.0);
                }
            });
        } else {
            row(ui, "Text:", |ui| {
                let r = ui.add(
                    egui::TextEdit::multiline(&mut wm.text)
                        .desired_rows(2)
                        .hint_text(crate::i18n::tr("© Your Name  ·  {copyright}  ·  {title}"))
                        .desired_width(ui.available_width()),
                );
                crate::widgets::register(ui.ctx(), "field:exportWmText", r.rect);
            });
            row(ui, "Direction:", |ui| combo(ui, "exportWmOrientation", 160.0, &[(false, "Horizontal"), (true, "Vertical")], &mut wm.vertical));
            row(ui, "Size:", |ui| {
                let mut pct = wm.size * 100.0;
                if number(ui, "exportWmSize", &mut pct, 0.5..=50.0, 0.1, " % of short edge") {
                    wm.size = (pct / 100.0).clamp(0.005, 0.5);
                }
                ui.add_space(8.0);
                let mut c = egui::Color32::from_rgb(wm.color[0], wm.color[1], wm.color[2]);
                let r = egui::widgets::color_picker::color_edit_button_srgba(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque);
                crate::widgets::register(ui.ctx(), "button:exportWmColor", r.rect);
                wm.color = [c.r(), c.g(), c.b()];
                check(ui, "exportWmShadow", &mut wm.shadow, "Shadow");
            });
        }
        row(ui, "Opacity:", |ui| {
            let mut pct = wm.opacity * 100.0;
            if number(ui, "exportWmOpacity", &mut pct, 0.0..=100.0, 1.0, " %") {
                wm.opacity = (pct / 100.0).clamp(0.0, 1.0);
            }
            ui.add_space(8.0);
            ui.label(crate::i18n::tr("Inset:"));
            let mut inset = wm.inset * 100.0;
            if number(ui, "exportWmInset", &mut inset, 0.0..=40.0, 0.1, " %") {
                wm.inset = (inset / 100.0).clamp(0.0, 0.4);
            }
        });
        // Anchor: Lightroom's nine points
        row(ui, "Anchor:", |ui| {
            egui::Grid::new("export-wm-anchor").spacing([4.0, 4.0]).show(ui, |ui| {
                let grid = [[P::TopLeft, P::Top, P::TopRight], [P::Left, P::Center, P::Right], [P::BottomLeft, P::Bottom, P::BottomRight]];
                for (y, line) in grid.iter().enumerate() {
                    for (x, a) in line.iter().enumerate() {
                        let on = wm.anchor == *a;
                        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                        crate::widgets::register(ui.ctx(), format!("button:exportWmAnchor-{}", y * 3 + x), r);
                        ui.painter().circle_stroke(r.center(), 6.0, egui::Stroke::new(1.0, t.text_label));
                        if on {
                            ui.painter().circle_filled(r.center(), 3.5, t.accent);
                        }
                        if resp.clicked() {
                            wm.anchor = *a;
                        }
                    }
                    ui.end_row();
                }
            });
        });
        // named watermarks (E09): save the current one
        row(ui, "Save as:", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let nid = egui::Id::new("watermark-preset-name");
            let mut name = ui.data(|d| d.get_temp::<String>(nid)).unwrap_or_default();
            let f = ui.add_sized([180.0, ROW_H], egui::TextEdit::singleline(&mut name).hint_text(crate::i18n::tr("watermark name")));
            crate::widgets::register(ui.ctx(), "field:watermarkPresetName", f.rect);
            if crate::widgets::text_button(ui, "watermarkPresetSave", "save.", false).clicked() {
                if name.trim().is_empty() {
                    app.toast(ui.ctx(), crate::i18n::tr("Type a name for the watermark"));
                } else {
                    let w = serde_json::to_value(&*wm).unwrap_or_default();
                    match app.run("watermark.save", json!({"name": name.trim(), "watermark": w})) {
                        Ok(_) => name.clear(),
                        Err(e) => app.toast(ui.ctx(), e),
                    }
                }
            }
            ui.data_mut(|d| d.insert_temp(nid, name));
        });
    });
    let _: &Watermark = &simple;
}

/// Post-Processing ▸ After Export: Do nothing, Show in Finder, Open in Adobe Photoshop (when
/// installed), Open in the external editor (Settings), Open in Other Application…, the Export
/// Actions, and Go to Export Actions Folder Now.
fn post_processing(app: &mut LightcraftApp, ui: &mut egui::Ui, o: &mut ExportOptions) {
    let t = Tokens::get(ui.ctx());
    let actions = lightcraft_engine::export::export_actions();
    let editor = app.ui.settings.external_editor.trim().to_string();
    let photoshop = ui.data(|d| d.get_temp::<Option<String>>(egui::Id::new("export-photoshop"))).unwrap_or_else(|| {
        let p = lightcraft_engine::export::photoshop_app();
        ui.data_mut(|d| d.insert_temp(egui::Id::new("export-photoshop"), p.clone()));
        p
    });
    let other_id = egui::Id::new("export-after-other");
    let mut other = ui.data(|d| d.get_temp::<bool>(other_id)).unwrap_or(false);
    let mut go_folder = false;
    row(ui, "After Export:", |ui| {
        let r = egui::ComboBox::from_id_salt("export-after").selected_text(after_summary(o)).width(280.0).show_ui(ui, |ui| {
            fn pick(ui: &mut egui::Ui, o: &mut ExportOptions, id: &str, label: &str, after: AfterExport, app_name: &str) -> bool {
                let on = o.after == after && o.after_app == app_name;
                let r = ui.selectable_label(on, crate::i18n::tr(label));
                crate::widgets::register(ui.ctx(), format!("button:exportAfter-{id}"), r.rect);
                if r.clicked() {
                    o.after = after;
                    o.after_app = app_name.to_string();
                    return true;
                }
                false
            }
            pick(ui, o, "nothing", "Do nothing", AfterExport::Nothing, "");
            pick(ui, o, "show", crate::menus::reveal_label(), AfterExport::ShowInFinder, "");
            if let Some(ps) = &photoshop {
                pick(ui, o, "photoshop", "Open in Adobe Photoshop", AfterExport::OpenInApp, ps);
            }
            if !editor.is_empty() {
                pick(ui, o, "editor", &format!("Open in {}", app_name(&editor)), AfterExport::OpenInApp, &editor);
            }
            pick(ui, o, "open", "Open in the default app", AfterExport::OpenInApp, "");
            let r = ui.selectable_label(other, crate::i18n::tr("Open in Other Application…"));
            crate::widgets::register(ui.ctx(), "button:exportAfter-other", r.rect);
            if r.clicked() {
                o.after = AfterExport::OpenInApp;
                other = true;
            }
            if !actions.is_empty() {
                ui.separator();
                for (i, (name, path)) in actions.iter().enumerate() {
                    pick(ui, o, &format!("action{i}"), name, AfterExport::OpenInApp, path);
                }
            }
            ui.separator();
            let r = ui.selectable_label(false, crate::i18n::tr("Go to Export Actions Folder Now"));
            crate::widgets::register(ui.ctx(), "button:exportAfter-actionsFolder", r.rect);
            if r.clicked() {
                go_folder = true;
            }
        });
        crate::widgets::register(ui.ctx(), "button:exportAfter", r.response.rect);
    });
    let known =
        o.after_app.is_empty() || Some(&o.after_app) == photoshop.as_ref() || o.after_app == editor || actions.iter().any(|(_, p)| *p == o.after_app);
    if o.after == AfterExport::OpenInApp && (other || !known) {
        row(ui, "Application:", |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let w = (ui.available_width() - 80.0).max(120.0);
            let r = ui.add_sized([w, ROW_H], egui::TextEdit::singleline(&mut o.after_app).hint_text(crate::i18n::tr("an application, e.g. Preview")));
            crate::widgets::register(ui.ctx(), "field:exportAfterApp", r.rect);
            if app.services.pick_files.is_some()
                && crate::widgets::text_button(ui, "exportAfterChoose", "choose.", false).clicked()
                && let Some(f) = app.services.pick_files.as_mut().and_then(|pick| pick().into_iter().next())
            {
                o.after_app = f;
            }
        });
    } else if o.after != AfterExport::OpenInApp {
        other = false;
    }
    ui.data_mut(|d| d.insert_temp(other_id, other));
    if go_folder {
        match app.run("export.actionsFolder", json!({})) {
            Ok(v) => {
                let path = v["path"].as_str().unwrap_or("").to_string();
                if let Some(reveal) = app.services.reveal.as_mut() {
                    let _ = reveal(&path);
                }
                app.toast(ui.ctx(), format!("Export Actions folder: {}", path));
            }
            Err(e) => app.toast(ui.ctx(), e),
        }
    }
    let _ = t;
}

// ------------------------------------------------------------------------------- the preview (LightCraft)

/// A finished preview: the image, the full output size and the estimated file size.
#[derive(Clone)]
pub struct PreviewOut {
    pub size: (usize, usize),
    pub estimated_bytes: Option<u64>,
    pub texture: Option<egui::TextureHandle>,
    pub error: Option<String>,
}

type Slot = Arc<Mutex<Option<Result<(lightcraft_engine::export::Preview, (usize, usize)), String>>>>;

/// The preview / estimate for the dialog's current settings: rendered on a worker thread when
/// the settings, the photo or the catalog change (never per frame); `None` until it is in.
pub fn estimate(app: &mut LightcraftApp, ctx: &egui::Context, s: &State<'_>) -> Option<PreviewOut> {
    use std::hash::{Hash, Hasher};
    let id = app.session.targets(&json!({})).first().copied().or_else(|| app.session.active())?;
    let p = params(s);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (p.to_string(), id.0, app.session.catalog.revision).hash(&mut h);
    let key = h.finish();
    let done_id = egui::Id::new("export-preview-done");
    let slot_id = egui::Id::new("export-preview-slot");
    if let Some((k, out)) = ctx.data(|d| d.get_temp::<(u64, PreviewOut)>(done_id))
        && k == key
    {
        return Some(out);
    }
    match ctx.data(|d| d.get_temp::<(u64, Slot)>(slot_id)) {
        Some((k, slot)) if k == key => {
            let ready = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            let r = ready?;
            let out = match r {
                Ok((pv, size)) => {
                    let img = &pv.image;
                    let texture = Some(ctx.load_texture(
                        "export-preview",
                        egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.as_bytes()),
                        egui::TextureOptions::LINEAR,
                    ));
                    PreviewOut { size, estimated_bytes: pv.estimated_bytes, texture, error: None }
                }
                Err(e) => PreviewOut { size: (0, 0), estimated_bytes: None, texture: None, error: Some(e) },
            };
            ctx.data_mut(|d| d.insert_temp(done_id, (key, out.clone())));
            Some(out)
        }
        _ => {
            // settings changed: start a new preview (the old one's result is dropped)
            let o = ExportOptions::from_json(&p);
            let job = match lightcraft_engine::export::preview_job(&mut app.session, id, &o, PREVIEW_EDGE) {
                Ok(Some(j)) => j,
                Ok(None) => return None,
                Err(e) => {
                    let out = PreviewOut { size: (0, 0), estimated_bytes: None, texture: None, error: Some(e) };
                    ctx.data_mut(|d| d.insert_temp(done_id, (key, out.clone())));
                    return Some(out);
                }
            };
            let slot: Slot = Arc::default();
            let (out, repaint) = (slot.clone(), ctx.clone());
            let size = job.size;
            let work = move || {
                let r = job.run().map(|pv| (pv, size));
                *out.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(r);
                repaint.request_repaint();
            };
            #[cfg(not(target_arch = "wasm32"))]
            if let Err(e) = std::thread::Builder::new().name("lc-export-preview".into()).spawn(work) {
                log::warn!("export preview: {e}");
            }
            #[cfg(target_arch = "wasm32")]
            work();
            ctx.data_mut(|d| d.insert_temp(slot_id, (key, slot)));
            None
        }
    }
}

fn preview(app: &mut LightcraftApp, ui: &mut egui::Ui, s: &mut State<'_>) {
    let t = Tokens::get(ui.ctx());
    match estimate(app, ui.ctx(), s) {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(egui::RichText::new(crate::i18n::tr("Rendering the preview…")).color(t.text_dim));
            });
        }
        Some(PreviewOut { error: Some(e), .. }) => {
            ui.label(egui::RichText::new(format!("⚠ {e}")).color(t.caution));
        }
        Some(out) => {
            if let Some(tex) = &out.texture {
                let [w, h] = tex.size();
                let k = (ui.available_width().min(520.0) / w.max(1) as f32).min(360.0 / h.max(1) as f32).min(1.5);
                let r = ui.add(egui::Image::new(tex).fit_to_exact_size(vec2(w as f32 * k, h as f32 * k)));
                crate::widgets::register(ui.ctx(), "image:exportPreview", r.rect);
            }
            let size = out.estimated_bytes.map(|b| format!(" · ≈ {} per photo", lightcraft_engine::export::human_bytes(b))).unwrap_or_default();
            ui.label(
                egui::RichText::new(format!(
                    "{} × {} px{size}. Sized, sharpened and watermarked as it will export (shown small, in sRGB).",
                    out.size.0, out.size.1
                ))
                .color(t.text_dim),
            );
        }
    }
}

// ------------------------------------------------------------------------------- Existing Files ▸ Ask

/// The answer to one file that exists (Ask what to do).
pub const ASK_ANSWERS: [(&str, &str); 3] = [("unique", "new.name."), ("overwrite", "overwrite."), ("skip", "skip.")];

/// The Existing File dialog's body: which file, then the answers; `apply_all` answers the rest.
/// Returns the answer clicked (`unique` / `overwrite` / `skip`), or `cancel`.
pub fn ask_body(ui: &mut egui::Ui, path: &str, left: usize, apply_all: &mut bool) -> Option<&'static str> {
    let t = Tokens::get(ui.ctx());
    ui.set_width(460.0);
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    ui.label(egui::RichText::new(format!("A file named “{}” already exists.", name)).font(t.semibold(13.0)).color(t.text));
    ui.label(egui::RichText::new(short_path(path)).color(t.text_dim));
    ui.label(
        egui::RichText::new(crate::i18n::tr(
            "new.name. adds -2, -3… to the new file; overwrite. replaces the file (never an original in the library); skip. leaves it and doesn't export this photo.",
        ))
        .color(t.text_label),
    );
    if left > 1 {
        let r = ui.checkbox(apply_all, format!("Apply to all {} files that exist", left));
        crate::widgets::register(ui.ctx(), "check:exportAskAll", r.rect);
    }
    ui.add_space(8.0);
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if crate::widgets::text_button(ui, "dialogCancel", "cancel.", false).clicked() {
            out = Some("cancel");
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for (k, label) in ASK_ANSWERS.iter().rev() {
                if crate::widgets::text_button(ui, &format!("exportAsk-{k}"), label, *k == "unique").clicked() {
                    out = Some(*k);
                }
            }
        });
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_read_like_lightroom() {
        let mut o = ExportOptions::default();
        assert_eq!(file_settings_summary(&o), "JPEG (90%) / sRGB / 8 bit");
        o.format = F::Tiff;
        o.color_space = lightcraft_engine::export::OutputSpace::AdobeRgb;
        assert_eq!(file_settings_summary(&o), "TIFF / AdobeRGB (1998) / 16 bit");
        o.format = F::Jxl;
        o.quality = 100;
        assert!(file_settings_summary(&o).starts_with("JPEG XL (lossless)"));
        let o = ExportOptions::default();
        assert_eq!(sharpening_summary(&o), "Sharpening Off");
        let s = ExportOptions { sharpen: SharpenFor::Matte, sharpen_amount: SharpenAmount::High, ..Default::default() };
        assert_eq!(sharpening_summary(&s), "High sharpening for Matte Paper");
        assert_eq!(sizing_summary(&o, true, &Resize::default()), "240 ppi");
        assert_eq!(sizing_summary(&o, false, &Resize::long_edge(2048)), "Resize to Long Edge 2048 pixels at 240 ppi");
        let cm = ExportOptions { resolution_unit: ResolutionUnit::Cm, ppi: 254, ..Default::default() };
        assert_eq!(sizing_summary(&cm, true, &Resize::default()), "100 ppcm");
        let m = ExportOptions { remove_location: true, ..Default::default() };
        assert_eq!(metadata_summary(&m), "All Metadata, Remove Location Info");
        let c = ExportOptions { metadata: MetadataPolicy::Copyright, remove_location: true, ..Default::default() };
        assert_eq!(metadata_summary(&c), "Copyright Only", "the options don't apply to copyright only");
        assert_eq!(after_summary(&o), "Do nothing");
        assert_eq!(
            after_summary(&ExportOptions { after: AfterExport::OpenInApp, after_app: "/Applications/Preview.app".into(), ..Default::default() }),
            "Open in Preview"
        );
        assert_eq!(
            location_summary(&ExportOptions { export_to: ExportTo::SameFolder, subfolder: "Out".into(), ..Default::default() }, ""),
            "Same folder as original photo/Out"
        );
        assert_eq!(location_summary(&o, ""), "No folder chosen");
        assert_eq!(video_summary(&o, 0), "No videos in the selection");
    }
}
