//! The Masking panel: create masks, list them, edit components and local adjustments.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use lightcraft_catalog::PhotoId;
use lightcraft_develop::{ControlSpec, LocalAdjustments, MaskShape, Section, Track};
use serde_json::json;

use super::edit::apply_slider_out;
use super::right::header;
use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::theme::Tokens;
use crate::widgets::{divider, icon_button, register, slider, text_button};

const fn spec(id: &'static str, label: &'static str, min: f64, max: f64, step: f64, decimals: u8, track: Track) -> ControlSpec {
    ControlSpec { id, label, section: Section::Light, min, max, default: 0.0, step, decimals, track }
}

/// Local adjustment sliders (key = `LocalAdjustments` field name).
pub const LOCAL: &[ControlSpec] = &[
    spec("temp", "Temp", -100.0, 100.0, 1.0, 0, Track::Temp),
    spec("tint", "Tint", -100.0, 100.0, 1.0, 0, Track::Tint),
    spec("exposure", "Exposure", -4.0, 4.0, 0.01, 2, Track::Centered),
    spec("contrast", "Contrast", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("highlights", "Highlights", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("shadows", "Shadows", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("whites", "Whites", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("blacks", "Blacks", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("texture", "Texture", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("clarity", "Clarity", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("dehaze", "Dehaze", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("hue", "Hue", -100.0, 100.0, 1.0, 0, Track::Rainbow),
    spec("saturation", "Saturation", -100.0, 100.0, 1.0, 0, Track::Gradient { from: "#7a7a7a", to: "#e04a3a" }),
    spec("sharpness", "Sharpness", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("noise", "Noise", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("moire", "Moiré", -100.0, 100.0, 1.0, 0, Track::Centered),
    spec("defringe", "Defringe", -100.0, 100.0, 1.0, 0, Track::Centered),
];

pub fn local_get(a: &LocalAdjustments, key: &str) -> f64 {
    serde_json::to_value(a).ok().and_then(|v| v.get(key).and_then(|x| x.as_f64())).unwrap_or(0.0)
}

fn kind_label(s: &MaskShape) -> (&'static str, Icon) {
    match s {
        MaskShape::Brush { .. } => ("Brush", Icon::Brush),
        MaskShape::Linear { .. } => ("Linear Gradient", Icon::Linear),
        MaskShape::Radial { .. } => ("Radial Gradient", Icon::Radial),
        MaskShape::ColorRange { .. } => ("Color Range", Icon::Picker),
        MaskShape::LuminanceRange { .. } => ("Luminance Range", Icon::Sliders),
        MaskShape::DepthRange { .. } => ("Depth Range", Icon::Sliders),
        MaskShape::Subject { .. } => ("Subject", Icon::Subject),
        MaskShape::Sky { .. } => ("Sky", Icon::Sky),
        MaskShape::Background { .. } => ("Background", Icon::Subject),
        MaskShape::Object { .. } => ("Object", Icon::Subject),
        MaskShape::Prompt { .. } => ("Describe", Icon::Subject),
        MaskShape::People { .. } => ("People", Icon::Subject),
        MaskShape::Landscape { .. } => ("Landscape", Icon::Sky),
    }
}

const TILE: f32 = 52.0;
const TILE_GAP: f32 = 6.0;

/// Columns and tile width of the Create New Mask grid in `width`: four tiles of up to 52 pt (or
/// as wide as the widest label, `label_w`, needs), else three (narrowed if even those don't fit).
fn tile_layout(width: f32, label_w: f32) -> (usize, f32) {
    let need = (label_w + 8.0).max(48.0);
    let tile = |n: f32| ((width - (n - 1.0) * TILE_GAP) / n).floor().min(TILE.max(need));
    if tile(4.0) >= need { (4, tile(4.0)) } else { (3, tile(3.0).max(24.0)) }
}

pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let t = Tokens::get(ui.ctx());
    let d = app.session.develop_of(id).unwrap_or_default();
    header(ui, "Masking");
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 0, bottom: 10 }).show(ui, |ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Create New Mask")).color(t.text_dim));
        ui.add_space(6.0);
        // (Bryan's labels: short, lowercase, with a period)
        let tiles: [(&str, &str, Icon); 13] = [
            ("subject", "subject.", Icon::Subject),
            ("sky", "sky.", Icon::Sky),
            ("background", "background.", Icon::Subject),
            ("object", "object.", Icon::Subject),
            ("people", "people.", Icon::Subject),
            ("landscape", "landscape.", Icon::Sky),
            ("prompt", "describe.", Icon::Subject),
            ("brush", "brush.", Icon::Brush),
            ("linear", "linear.", Icon::Linear),
            ("radial", "radial.", Icon::Radial),
            ("luminanceRange", "luminance.", Icon::Sliders),
            ("colorRange", "color.", Icon::Picker),
            ("depthRange", "depth.", Icon::Sliders),
        ];
        // four 52 pt tiles a row when they fit, else as many as fit (at least three, shrunk)
        let label_w = tiles.iter().map(|(_, l, _)| ui.painter().layout_no_wrap(l.to_string(), t.font(10.5), t.text_dim).size().x).fold(0.0, f32::max);
        let (cols, tile) = tile_layout(ui.available_width(), label_w);
        egui::Grid::new("mask-tiles").spacing(vec2(TILE_GAP, TILE_GAP)).show(ui, |ui| {
            for (i, (kind, label, icon)) in tiles.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(tile, 52.0), Sense::click());
                register(ui.ctx(), format!("maskNew:{kind}"), r);
                ui.painter().rect_filled(r, 4.0, if resp.hovered() { t.hover } else { t.inset });
                paint(ui.painter(), Rect::from_center_size(r.center() - vec2(0.0, 7.0), vec2(20.0, 20.0)), *icon, t.text_label);
                ui.painter().text(pos2(r.center().x, r.bottom() - 9.0), Align2::CENTER_CENTER, *label, t.font(10.5), t.text_dim);
                if resp.clicked() {
                    match *kind {
                        "colorRange" => {
                            // an empty colour range; clicking the photo samples it
                            let _ = app.run("mask.add", json!({"kind": "colorRange"}));
                            app.ui.tool = "colorRange".into();
                            app.toast(ui.ctx(), crate::i18n::tr("Click the photo to pick a color · ⇧-click adds more"));
                        }
                        "object" => start_object(app, ui.ctx(), "new"),
                        "prompt" => start_describe(app, "new"),
                        "people" | "landscape" => open_picker(app, ui.ctx(), id, kind),
                        "sky" | "subject" | "background" => {
                            if let Err(e) = app.run("mask.add", json!({"kind": kind})) {
                                app.toast_error(ui.ctx(), e);
                            }
                        }
                        "brush" | "linear" | "radial" => {
                            app.ui.tool = kind.to_string();
                            if *kind != "brush" {
                                let _ = app.run("mask.add", json!({"kind": kind}));
                            }
                        }
                        k => {
                            let _ = app.run("mask.add", json!({"kind": k}));
                        }
                    }
                }
                if i % cols == cols - 1 {
                    ui.end_row();
                }
            }
        });
        describe_field(app, ui, true);
        picker(app, ui, id);
        models_row(app, ui);
        let seg = &app.session.segmenter;
        let status = if seg.analyzing() {
            Some("Analyzing the photo for AI masks…")
        } else if seg.auto_busy() {
            Some("Finding it in the photo…")
        } else if seg.busy() {
            Some("Selecting…")
        } else if seg.detail_busy() || app.ui.detail_due.is_some() {
            Some("Refining the mask's detail…")
        } else {
            None
        };
        if let Some(status) = status {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(crate::i18n::tr(status)).color(t.text_dim));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        // the model download, while its dialog is closed
        let download = seg.download_status();
        if download.running && app.ui.dialog.is_none() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let pct = if download.total > 0 { download.done as f64 / download.total as f64 } else { 0.0 };
                let r = ui.add(
                    egui::ProgressBar::new(pct as f32).desired_width(ui.available_width() - 70.0).text(format!("SAM 3: {} %", (pct * 100.0).floor())),
                );
                register(ui.ctx(), "maskSamProgress", r.rect);
                if text_button(ui, "maskSamDetails", crate::i18n::tr("Details"), false).clicked() {
                    app.offer_sam_download(None);
                }
            });
        }
    });
    divider(ui);
    // mask list
    egui::Frame::NONE.inner_margin(egui::Margin { left: 16, right: 16, top: 8, bottom: 8 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Masks")).font(t.semibold(13.0)).color(t.text_label));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let on = app.ui.mask_overlay;
                if icon_button(ui, "maskOverlay", Icon::Eye, vec2(24.0, 24.0), on, true, "Show overlay (O)").clicked() {
                    app.ui.mask_overlay = !on;
                }
            });
        });
        if d.masks.is_empty() {
            ui.label(egui::RichText::new(crate::i18n::tr("No masks yet. Choose a mask type above.")).color(t.text_dim));
        }
        let count = d.masks.len();
        let stale = lightcraft_develop::ai::stale_components(&d);
        if !stale.is_empty() {
            // Lightroom's warning, for every AI mask at once (Photo ▸ Update AI Settings)
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(egui::RichText::new(crate::i18n::tr("AI masks need an update")).color(t.text_dim));
                if text_button(ui, "maskUpdateAll", "update.all.", false).clicked() {
                    let _ = app.run("mask.updateAI", json!({}));
                }
            });
            ui.add_space(4.0);
        }
        for (index, m) in d.masks.iter().enumerate() {
            let sel = app.session.active_mask == Some(m.id);
            if let Some((rid, name)) = app.ui.renaming_mask.as_mut().filter(|(rid, _)| *rid == m.id) {
                // inline rename: Enter (or leaving the field) commits, Escape cancels
                let rid = *rid;
                let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).id_salt(("maskRename", rid)));
                register(ui.ctx(), format!("maskRename:{rid}"), r.rect);
                if !r.has_focus() && !r.lost_focus() {
                    r.request_focus();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    app.ui.renaming_mask = None;
                } else if r.lost_focus() {
                    let name = name.trim().to_string();
                    app.ui.renaming_mask = None;
                    if !name.is_empty() && name != m.name {
                        let _ = app.run("mask.rename", json!({"id": rid, "name": name}));
                    }
                }
                continue;
            }
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            register(ui.ctx(), format!("mask:{}", m.id), r);
            ui.painter().rect_filled(
                r,
                4.0,
                if sel {
                    t.tool_active
                } else if resp.hovered() {
                    t.hover.gamma_multiply(0.7)
                } else {
                    t.chrome
                },
            );
            let icon = m.components.first().map(|c| kind_label(&c.shape).1).unwrap_or(Icon::Mask);
            paint(ui.painter(), Rect::from_min_size(r.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), icon, t.text_label);
            ui.painter().text(
                pos2(r.left() + 32.0, r.center().y),
                Align2::LEFT_CENTER,
                &m.name,
                t.font(13.0),
                if m.visible { t.text } else { t.text_disabled },
            );
            // an AI mask made before the photo changed (lens / geometry corrections, spots,
            // Enhance): Lightroom's update warning; the chip recomputes it
            if stale.iter().any(|(sid, _)| *sid == m.id) {
                let cr = Rect::from_min_size(pos2(r.right() - 35.0 - 50.0, r.center().y - 10.0), vec2(50.0, 20.0));
                register(ui.ctx(), format!("maskUpdate:{}", m.id), cr);
                let c = ui.interact(cr, egui::Id::new(("maskUpdate", m.id)), Sense::click());
                ui.painter().rect_filled(cr, 4.0, if c.hovered() { t.hover } else { t.inset });
                ui.painter().text(cr.center(), Align2::CENTER_CENTER, "⚠ update.", t.font(10.5), t.text);
                if c.clicked() {
                    let _ = app.run("mask.updateAI", json!({"id": m.id}));
                }
                c.on_hover_text(crate::i18n::tr("Made before the photo changed: update this AI mask"));
            }
            // show / hide on hover (and always while hidden)
            // (the pointer test, not `hovered`: over the eye, the row itself no longer counts as hovered)
            if ui.rect_contains_pointer(r) || !m.visible {
                let er = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(22.0, 22.0));
                register(ui.ctx(), format!("maskVisible:{}", m.id), er);
                let eye = ui.interact(er, egui::Id::new(("maskVisible", m.id)), Sense::click());
                paint(
                    ui.painter(),
                    er.shrink(3.0),
                    if m.visible { Icon::Eye } else { Icon::EyeOff },
                    if eye.hovered() { t.text } else { t.text_dim },
                );
                if eye.clicked() {
                    let _ = app.run("mask.visible", json!({"id": m.id}));
                }
            }
            if resp.clicked() {
                let _ = app.run("mask.select", json!({"id": m.id}));
            }
            if resp.double_clicked() {
                app.ui.renaming_mask = Some((m.id, m.name.clone()));
            }
            resp.context_menu(|ui| mask_menu(app, ui, m.id, &m.name, m.visible, index, count));
            if sel {
                // right under the selected mask: ＋ adds to it, − takes away from it (any mask
                // type; Describe… opens its field here)
                ui.horizontal(|ui| {
                    ui.add_space(30.0);
                    let plus = text_button(ui, &format!("maskPlus:{}", m.id), "+", false).on_hover_text(crate::i18n::tr("Add to this mask"));
                    egui::Popup::menu(&plus).show(|ui| component_menu(app, ui, "add"));
                    let minus = text_button(ui, &format!("maskMinus:{}", m.id), "−", false).on_hover_text(crate::i18n::tr("Subtract from this mask"));
                    egui::Popup::menu(&minus).show(|ui| component_menu(app, ui, "subtract"));
                });
                describe_field(app, ui, false);
            }
        }
        if !d.masks.is_empty() {
            ui.add_space(6.0);
            overlay_options(app, ui);
        }
    });
    let Some(mid) = app.session.active_mask else { return };
    let Some(m) = d.masks.iter().find(|m| m.id == mid).cloned() else { return };
    divider(ui);
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 8, bottom: 8 }).show(ui, |ui| {
        for (i, c) in m.components.iter().enumerate() {
            let (kind, icon) = kind_label(&c.shape);
            let label = c.name.clone().unwrap_or_else(|| match &c.shape {
                MaskShape::Prompt { text, .. } => format!("“{text}”"),
                MaskShape::Landscape { class, .. } => {
                    class.split(',').map(|c| lightcraft_develop::ai::landscape_label(c.trim()).unwrap_or(c)).collect::<Vec<_>>().join(" + ")
                }
                MaskShape::People { person, parts, .. } => format!(
                    "Person {}: {}",
                    person + 1,
                    parts.iter().map(|x| lightcraft_develop::ai::part_label(x).unwrap_or(x)).collect::<Vec<_>>().join(", ")
                ),
                _ => kind.to_string(),
            });
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                paint(ui.painter(), r, icon, t.text_label);
                let op = match c.op {
                    lightcraft_develop::MaskOp::Add => "",
                    lightcraft_develop::MaskOp::Subtract => "− ",
                    lightcraft_develop::MaskOp::Intersect => "∩ ",
                };
                if let Some((_, _, name)) = app.ui.renaming_component.as_mut().filter(|(mid, k, _)| *mid == m.id && *k == i) {
                    // inline rename: Enter (or leaving the field) commits, Escape cancels
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width() - 30.0).id_salt(("compRename", m.id, i)));
                    register(ui.ctx(), format!("componentRename:{i}"), r.rect);
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if ui.input(|inp| inp.key_pressed(egui::Key::Escape)) {
                        app.ui.renaming_component = None;
                    } else if r.lost_focus() {
                        let name = name.trim().to_string();
                        app.ui.renaming_component = None;
                        if Some(&name) != c.name.as_ref() {
                            let _ = app.run("mask.component", json!({"id": m.id, "component": i, "action": "rename", "name": name}));
                        }
                    }
                    return;
                }
                // a long name is cut short (with …) before the options button, not past the panel
                let text = format!("{op}{label}{}", if c.invert { crate::i18n::tr(" (inverted)") } else { "" });
                let room = (ui.available_width() - 30.0).max(0.0);
                let resp = ui.scope(|ui| {
                    ui.set_max_width(room);
                    ui.add(egui::Label::new(text).truncate().sense(Sense::click()))
                });
                let resp = resp.inner;
                register(ui.ctx(), format!("component:{i}"), resp.rect);
                if resp.double_clicked() {
                    app.ui.renaming_component = Some((m.id, i, label.clone()));
                }
                resp.context_menu(|ui| component_row_menu(app, ui, m.id, i, &label, m.components.len()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let more = ui.small_button("…").on_hover_text(crate::i18n::tr("Component options"));
                    register(ui.ctx(), format!("button:componentMenu{i}"), more.rect);
                    egui::Popup::menu(&more).show(|ui| component_row_menu(app, ui, m.id, i, &label, m.components.len()));
                });
            });
            range_controls(app, ui, i, &c.shape);
        }
        ui.add_space(6.0);
        // wraps: the five actions are wider than a narrow panel
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            let add = text_button(ui, "maskAddComp", crate::i18n::tr("Add"), false);
            egui::Popup::menu(&add).show(|ui| component_menu(app, ui, "add"));
            let sub = text_button(ui, "maskSubComp", crate::i18n::tr("Subtract"), false);
            egui::Popup::menu(&sub).show(|ui| component_menu(app, ui, "subtract"));
            let int = text_button(ui, "maskIntComp", crate::i18n::tr("Intersect"), false);
            egui::Popup::menu(&int).show(|ui| component_menu(app, ui, "intersect"));
            if text_button(ui, "maskInvert", crate::i18n::tr("Invert"), m.invert).clicked() {
                let _ = app.run("mask.invert", json!({}));
            }
            if icon_button(ui, "maskDelete", Icon::Trash, vec2(26.0, 24.0), false, true, "Delete mask").clicked() {
                let _ = app.run("mask.delete", json!({}));
            }
        });
    });
    if app.ui.tool == "brush" {
        divider(ui);
        brush_settings(app, ui);
    }
    divider(ui);
    crate::panels::mask_effects::effect_row(app, ui, &m);
    ui.add_space(4.0);
    for s in LOCAL {
        let v = local_get(&m.adjust, s.id);
        let out = slider(ui, s, v, true, None);
        apply_slider_out(app, s, out, |app, v| app.run("mask.adjust", json!({"values": {s.id: v}})));
    }
    let amt = ControlSpec {
        id: "amount",
        label: "Amount",
        section: Section::Light,
        min: 0.0,
        max: 200.0,
        default: 100.0,
        step: 1.0,
        decimals: 0,
        track: Track::Plain,
    };
    let out = slider(ui, &amt, m.adjust.amount, true, None);
    apply_slider_out(app, &amt, out, |app, v| app.run("mask.adjust", json!({"values": {"amount": v}})));
    let refine = ControlSpec {
        id: "refine",
        label: "Refine Edges",
        section: Section::Light,
        min: 0.0,
        max: 100.0,
        default: 0.0,
        step: 1.0,
        decimals: 0,
        track: Track::Plain,
    };
    let out = slider(ui, &refine, m.refine, true, None);
    apply_slider_out(app, &refine, out, |app, v| app.run("mask.refine", json!({"value": v})));
    ui.add_space(30.0);
    let _ = Stroke::NONE;
}

/// The right-click menu of a mask in the Masks list.
fn mask_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, id: u32, name: &str, visible: bool, index: usize, count: usize) {
    let mut run = |ui: &mut egui::Ui, label: &str, enabled: bool, cmd: &str, p: serde_json::Value| {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            let _ = app.run(cmd, p);
            ui.close();
        }
    };
    run(ui, "Duplicate Mask", true, "mask.duplicate", json!({"id": id}));
    run(ui, "Duplicate and Invert Mask", true, "mask.duplicate", json!({"id": id, "invert": true}));
    run(ui, "Invert Mask", true, "mask.invert", json!({"id": id}));
    run(ui, if visible { "Hide Mask" } else { "Show Mask" }, true, "mask.visible", json!({"id": id}));
    ui.separator();
    run(ui, "Move Up", index > 0, "mask.move", json!({"id": id, "delta": -1}));
    run(ui, "Move Down", index + 1 < count, "mask.move", json!({"id": id, "delta": 1}));
    ui.separator();
    if ui.button(crate::i18n::tr("Rename…")).clicked() {
        app.ui.renaming_mask = Some((id, name.to_string()));
        ui.close();
    }
    if ui.button(crate::i18n::tr("Delete Mask")).clicked() {
        let _ = app.run("mask.delete", json!({"id": id}));
        ui.close();
    }
}

/// Overlay colours offered as swatches (the colour of the selected one is used for the tint).
pub const OVERLAY_COLORS: [[u8; 3]; 5] = [[230, 30, 40], [40, 200, 70], [40, 110, 240], [250, 210, 30], [255, 255, 255]];

/// How the selected mask is shown: overlay mode, colour, opacity, pins.
fn overlay_options(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    use lightcraft_pipeline::MaskView;
    let t = Tokens::get(ui.ctx());
    let view = MaskView::parse(&app.ui.mask_overlay_mode).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::tr("Overlay")).color(t.text_dim));
        let r = crate::widgets::dropdown(ui, "maskOverlayMode", view.label(), t.font(12.5), t.text_label);
        egui::Popup::menu(&r).show(|ui| {
            for v in MaskView::ALL {
                if ui.selectable_label(v == view, v.label()).clicked() {
                    let _ = app.run("view.maskOverlayMode", json!({"mode": v.name()}));
                }
            }
        });
    });
    let colored = matches!(view, MaskView::Color | MaskView::ColorOnBw);
    if colored {
        ui.horizontal(|ui| {
            for c in OVERLAY_COLORS {
                let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                register(ui.ctx(), format!("maskOverlayColor:{:02x}{:02x}{:02x}", c[0], c[1], c[2]), r);
                ui.painter().rect_filled(r.shrink(2.0), 3.0, egui::Color32::from_rgb(c[0], c[1], c[2]));
                if app.ui.mask_overlay_color == c {
                    ui.painter().rect_stroke(r, 3.0, Stroke::new(1.5, t.text), egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    let _ = app.run("view.maskOverlayColor", json!({"color": c}));
                }
            }
        });
    }
    let spec = ControlSpec {
        id: "ui.maskOverlayOpacity",
        label: "Opacity",
        section: Section::Light,
        min: 0.0,
        max: 100.0,
        default: 50.0,
        step: 1.0,
        decimals: 0,
        track: Track::Plain,
    };
    let out = slider(ui, &spec, app.ui.mask_overlay_opacity as f64, colored && app.ui.mask_overlay, None);
    if let Some(v) = out.value {
        app.ui.mask_overlay_opacity = v as f32;
    }
    let mut pins = app.ui.mask_pins;
    if ui.checkbox(&mut pins, crate::i18n::tr("Show Pins")).changed() {
        let _ = app.run("view.maskPins", json!({"show": pins}));
    }
}

/// Controls for a range component: the selected range (two handles over a dark → light bar),
/// Smoothness, and Show Luminance Map; colour ranges get Refine.
fn range_controls(app: &mut LightcraftApp, ui: &mut egui::Ui, comp: usize, shape: &MaskShape) {
    let t = Tokens::get(ui.ctx());
    let update = |app: &mut LightcraftApp, shape: MaskShape| {
        let _ = app.run("mask.update", json!({"component": comp, "shape": shape}));
    };
    match shape {
        MaskShape::LuminanceRange { lo, hi, lo_feather, hi_feather } => {
            ui.label(egui::RichText::new(crate::i18n::tr("Select Luminance Range")).color(t.text_dim).size(11.5));
            let (lo, hi) = (*lo, *hi);
            let w = ui.available_width().min(240.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::click_and_drag());
            register(ui.ctx(), format!("lumRange:{comp}"), rect);
            let p = ui.painter();
            // dark → light ramp, the selected span outlined
            let n = 24;
            for k in 0..n {
                let a = k as f32 / n as f32;
                let r = Rect::from_min_max(
                    pos2(rect.left() + a * w, rect.top() + 4.0),
                    pos2(rect.left() + (a + 1.0 / n as f32) * w + 0.5, rect.bottom() - 4.0),
                );
                let g = (a * 255.0) as u8;
                p.rect_filled(r, 0.0, egui::Color32::from_gray(g));
            }
            let x = |v: f64| rect.left() + v.clamp(0.0, 1.0) as f32 * w;
            p.rect_stroke(
                Rect::from_min_max(pos2(x(lo), rect.top() + 2.0), pos2(x(hi), rect.bottom() - 2.0)),
                2.0,
                Stroke::new(1.5, t.accent),
                egui::StrokeKind::Middle,
            );
            for v in [lo, hi] {
                p.circle_filled(pos2(x(v), rect.center().y), 5.0, egui::Color32::WHITE);
                p.circle_stroke(pos2(x(v), rect.center().y), 5.0, Stroke::new(1.0, egui::Color32::from_gray(40)));
            }
            // drag the nearer handle; one undo step per drag
            if resp.drag_started() {
                let _ = app.run("develop.beginInteraction", json!({"label": "Luminance Range"}));
            }
            if (resp.dragged() || resp.clicked())
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let v = (((pos.x - rect.left()) / w) as f64).clamp(0.0, 1.0);
                let (nlo, nhi) = if (v - lo).abs() <= (v - hi).abs() { (v.min(hi - 0.01), hi) } else { (lo, v.max(lo + 0.01)) };
                update(app, MaskShape::LuminanceRange { lo: nlo, hi: nhi, lo_feather: *lo_feather, hi_feather: *hi_feather });
            }
            if resp.drag_stopped() {
                let _ = app.run("develop.endInteraction", json!({}));
            }
            // Smoothness: both falloffs at once
            let smooth = ControlSpec {
                id: "smoothness",
                label: "Smoothness",
                section: Section::Light,
                min: 0.0,
                max: 100.0,
                default: 20.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let cur = ((lo_feather + hi_feather) / 2.0 / 0.5 * 100.0).clamp(0.0, 100.0);
            let out = slider(ui, &smooth, cur, true, None);
            apply_slider_out(app, &smooth, out, |app, v| {
                let f = v / 100.0 * 0.5;
                app.run("mask.update", json!({"component": comp, "shape": MaskShape::LuminanceRange { lo, hi, lo_feather: f, hi_feather: f }}))
            });
            let mut map = app.ui.mask_overlay && app.ui.mask_overlay_mode == "colorOnBw";
            let r = ui.checkbox(&mut map, crate::i18n::tr("Show Luminance Map"));
            register(ui.ctx(), format!("check:lumMap{comp}"), r.rect);
            if r.changed() {
                // the luminance map: the photo in black & white with the selected range tinted
                if map {
                    app.ui.luminance_map_restore = Some((app.ui.mask_overlay, app.ui.mask_overlay_mode.clone()));
                    app.ui.mask_overlay = true;
                    app.ui.mask_overlay_mode = "colorOnBw".into();
                } else {
                    let (on, mode) = app.ui.luminance_map_restore.take().unwrap_or((false, "color".into()));
                    app.ui.mask_overlay = on;
                    app.ui.mask_overlay_mode = if mode == "colorOnBw" { "color".into() } else { mode };
                }
            }
        }
        MaskShape::Object { edge, .. } | MaskShape::Prompt { edge, .. } => {
            // Edge: how crisp the selection's border is (−100 hard … 0 as computed … 100 soft)
            let spec = ControlSpec {
                id: "edge",
                label: "Edge",
                section: Section::Light,
                min: -100.0,
                max: 100.0,
                default: 0.0,
                step: 1.0,
                decimals: 0,
                track: Track::Centered,
            };
            let out = slider(ui, &spec, *edge, true, None);
            let base = shape.clone();
            apply_slider_out(app, &spec, out, |app, v| {
                let mut s = base.clone();
                if let MaskShape::Object { edge, .. } | MaskShape::Prompt { edge, .. } = &mut s {
                    *edge = v.clamp(-100.0, 100.0);
                }
                app.run("mask.update", json!({"component": comp, "shape": s}))
            });
            ui.label(egui::RichText::new(crate::i18n::tr("− harder border · + softer border")).color(t.text_dim).size(11.0));
        }
        MaskShape::ColorRange { samples, refine } => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(crate::i18n::tr_format!("{} sample{}", samples.len(), if samples.len() == 1 { "" } else { "s" }))
                        .color(t.text_dim)
                        .size(11.5),
                );
                let picking = app.ui.tool == "colorRange";
                if text_button(ui, &format!("colorPick{comp}"), crate::i18n::tr("Pick"), picking)
                    .on_hover_text(crate::i18n::tr("Click the photo to pick a color; ⇧-click adds more (up to 5)"))
                    .clicked()
                {
                    app.ui.tool = if picking { String::new() } else { "colorRange".into() };
                }
            });
            let spec = ControlSpec {
                id: "refine",
                label: "Refine",
                section: Section::Color,
                min: 0.0,
                max: 100.0,
                default: 50.0,
                step: 1.0,
                decimals: 0,
                track: Track::Plain,
            };
            let out = slider(ui, &spec, *refine, true, None);
            let samples = samples.clone();
            apply_slider_out(app, &spec, out, |app, v| {
                app.run("mask.update", json!({"component": comp, "shape": MaskShape::ColorRange { samples: samples.clone(), refine: v }}))
            });
        }
        MaskShape::DepthRange { lo, hi, feather } => {
            // Select Depth Range (0 nearest … 100 farthest) over the photo's depth map
            ui.label(egui::RichText::new(crate::i18n::tr("Select Depth Range")).color(t.text_dim).size(11.5));
            let (lo, hi, feather) = (*lo, *hi, *feather);
            for (spec, v) in [(&DEPTH_NEAR, lo), (&DEPTH_FAR, hi), (&DEPTH_SMOOTH, feather)] {
                let out = crate::widgets::slider(ui, spec, v * 100.0, true, None);
                let id = spec.id;
                crate::panels::edit::apply_slider_out(app, spec, out, |app, v| {
                    let v = v / 100.0;
                    let s = match id {
                        "depthRange.near" => MaskShape::DepthRange { lo: v, hi, feather },
                        "depthRange.far" => MaskShape::DepthRange { lo, hi: v, feather },
                        _ => MaskShape::DepthRange { lo, hi, feather: v },
                    };
                    app.run("mask.update", json!({"component": comp, "shape": s}))
                });
            }
        }
        _ => {}
    }
}

fn depth_spec(id: &'static str, label: &'static str, default: f64) -> lightcraft_develop::ControlSpec {
    lightcraft_develop::ControlSpec {
        id,
        label,
        section: lightcraft_develop::Section::LensBlur,
        min: 0.0,
        max: 100.0,
        default,
        step: 1.0,
        decimals: 0,
        track: lightcraft_develop::Track::Gradient { from: "#f0c040", to: "#5040c0" },
    }
}

static DEPTH_NEAR: std::sync::LazyLock<lightcraft_develop::ControlSpec> = std::sync::LazyLock::new(|| depth_spec("depthRange.near", "Near", 0.0));
static DEPTH_FAR: std::sync::LazyLock<lightcraft_develop::ControlSpec> = std::sync::LazyLock::new(|| depth_spec("depthRange.far", "Far", 30.0));
static DEPTH_SMOOTH: std::sync::LazyLock<lightcraft_develop::ControlSpec> = std::sync::LazyLock::new(|| lightcraft_develop::ControlSpec {
    track: lightcraft_develop::Track::Plain,
    ..depth_spec("depthRange.smoothness", "Smoothness", 5.0)
});

/// The right-click / "…" menu of one component of the selected mask.
fn component_row_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, mask: u32, k: usize, label: &str, count: usize) {
    let mut run = |ui: &mut egui::Ui, text: &str, p: serde_json::Value| {
        if ui.button(text).clicked() {
            let mut p = p;
            p["id"] = json!(mask);
            p["component"] = json!(k);
            let _ = app.run("mask.component", p);
            ui.close();
        }
    };
    run(ui, "Invert", json!({"action": "invert"}));
    run(ui, &crate::i18n::tr_format!("Duplicate \"{label}\"", label = label), json!({"action": "duplicate"}));
    if k > 0 {
        ui.menu_button(crate::i18n::tr("Mode"), |ui| {
            for (op, text) in [("add", "Add"), ("subtract", "Subtract"), ("intersect", "Intersect")] {
                run(ui, text, json!({"action": "op", "op": op}));
            }
        });
    }
    ui.menu_button(crate::i18n::tr("Intersect with"), |ui| component_menu(app, ui, "intersect"));
    ui.menu_button(crate::i18n::tr("Subtract"), |ui| component_menu(app, ui, "subtract"));
    ui.separator();
    if ui.button(crate::i18n::tr("Rename…")).clicked() {
        app.ui.renaming_component = Some((mask, k, label.to_string()));
        ui.close();
    }
    let del = if count == 1 {
        crate::i18n::tr_format!("Delete \"{label}\" (and the mask)", label = label)
    } else {
        crate::i18n::tr_format!("Delete \"{label}\"", label = label)
    };
    if ui.button(del).clicked() {
        let _ = app.run("mask.component", json!({"id": mask, "component": k, "action": "delete"}));
        ui.close();
    }
}

/// Short labels for the People parts and Landscape classes (Bryan's chip style), in
/// `lightcraft_develop::ai` order.
const PART_CHIPS: [&str; 10] = ["person.", "face.", "body.", "eyebrows.", "sclera.", "iris.", "lips.", "teeth.", "hair.", "clothes."];
const CLASS_CHIPS: [&str; 7] = ["sky.", "water.", "vegetation.", "mountains.", "architecture.", "natural.ground.", "artificial.ground."];

/// One click in a pickable list of `n`: click = just this one, ⌘-click adds or removes it,
/// ⇧-click the run from the last click (`anchor`).
pub(crate) fn pick(sel: &mut Vec<usize>, anchor: &mut Option<usize>, i: usize, m: egui::Modifiers) {
    if m.shift
        && let Some(a) = *anchor
    {
        let (lo, hi) = (a.min(i), a.max(i));
        for k in lo..=hi {
            if !sel.contains(&k) {
                sel.push(k);
            }
        }
    } else if m.command {
        if let Some(p) = sel.iter().position(|k| *k == i) {
            sel.remove(p);
        } else {
            sel.push(i);
        }
        *anchor = Some(i);
    } else {
        *sel = vec![i];
        *anchor = Some(i);
    }
    sel.sort_unstable();
}

/// Open the People or Landscape picker and find what the photo has (in the background).
pub(crate) fn open_picker(app: &mut LightcraftApp, ctx: &egui::Context, id: PhotoId, kind: &str) {
    let mut p = crate::state::AiPick { kind: kind.to_string(), separate: false, ..Default::default() };
    if kind == "people" {
        p.parts = vec![0];
        p.people = vec![0];
    }
    app.ui.ai_pick = Some(p);
    let found = if kind == "people" { app.session.people_found(id).is_some() } else { app.session.coverage_found(id).is_some() };
    if !found {
        let cmd = if kind == "people" { "segment.detectPeople" } else { "segment.detectLandscape" };
        if let Err(e) = app.run(cmd, json!({})) {
            // (no model: the panel's get. row says what to download)
            app.ui.ai_pick = None;
            app.toast_error(ctx, e);
        }
    }
}

/// While nothing is being computed but nothing was found yet (the search failed, or the photo
/// changed): retry. runs `cmd` again.
fn retry_row(app: &mut LightcraftApp, ui: &mut egui::Ui, cmd: &str) {
    if app.session.segmenter.auto_busy() || app.session.segmenter.busy() {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        return;
    }
    ui.add_space(8.0);
    if text_button(ui, "aiPickRetry", "retry.", false).clicked()
        && let Err(e) = app.run(cmd, json!({}))
    {
        app.toast_error(ui.ctx(), e);
    }
}

/// A row of chips for a pickable list, with all. / none.; true when the picks changed.
fn chip_row(ui: &mut egui::Ui, key: &str, labels: &[String], sel: &mut Vec<usize>, anchor: &mut Option<usize>) -> bool {
    let before = sel.clone();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
        for (i, l) in labels.iter().enumerate() {
            let r = text_button(ui, &format!("{key}:{i}"), l, sel.contains(&i));
            if r.clicked() {
                let m = ui.input(|inp| inp.modifiers);
                pick(sel, anchor, i, m);
            }
        }
        if text_button(ui, &format!("{key}:all"), "all.", false).clicked() {
            *sel = (0..labels.len()).collect();
        }
        if text_button(ui, &format!("{key}:none"), "none.", false).clicked() {
            sel.clear();
        }
    });
    *sel != before
}

/// Lightroom's People and Landscape panes: what the photo has, what to select, and make.
fn picker(app: &mut LightcraftApp, ui: &mut egui::Ui, id: PhotoId) {
    let Some(mut p) = app.ui.ai_pick.clone() else { return };
    let t = Tokens::get(ui.ctx());
    ui.add_space(12.0);
    let people = p.kind == "people";
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(crate::i18n::tr(if people { "People" } else { "Landscape" })).font(t.semibold(13.0)).color(t.text_label));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if text_button(ui, "aiPickClose", "close.", false).clicked() {
                app.ui.ai_pick = None;
            }
        });
    });
    if app.ui.ai_pick.is_none() {
        return;
    }
    ui.add_space(8.0);
    if people {
        match app.session.people_found(id).map(<[_]>::len) {
            None => {
                ui.label(egui::RichText::new(crate::i18n::tr("Finding the people in this photo…")).color(t.text_dim));
                retry_row(app, ui, "segment.detectPeople");
            }
            Some(0) => {
                ui.label(egui::RichText::new(crate::i18n::tr("No people were found in this photo.")).color(t.text_dim));
            }
            Some(n) => {
                let labels: Vec<String> = (1..=n).map(|k| format!("{k}.")).collect();
                p.people.retain(|k| *k < n);
                let [a0, a1, a2] = &mut p.anchors;
                chip_row(ui, "aiPerson", &labels, &mut p.people, a0);
                ui.add_space(10.0);
                let parts: Vec<String> = PART_CHIPS.iter().map(|s| s.to_string()).collect();
                chip_row(ui, "aiPart", &parts, &mut p.parts, a1);
                let _ = a2;
            }
        }
    } else {
        match app.session.coverage_found(id).map(<[_]>::to_vec) {
            None => {
                ui.label(egui::RichText::new(crate::i18n::tr("Looking at the photo…")).color(t.text_dim));
                retry_row(app, ui, "segment.detectLandscape");
            }
            Some(cov) => {
                // every class, with how much of the photo it covers (a quiet 0 when none)
                let labels: Vec<String> = CLASS_CHIPS
                    .iter()
                    .zip(lightcraft_develop::ai::LANDSCAPE_CLASSES)
                    .map(|(chip, (cid, _))| {
                        let c = cov.iter().find(|x| x.0 == *cid).map_or(0.0, |x| x.1);
                        if c >= 0.005 { format!("{chip} {:.0}%", c * 100.0) } else { format!("{chip} 0") }
                    })
                    .collect();
                let [_, _, a2] = &mut p.anchors;
                chip_row(ui, "aiClass", &labels, &mut p.classes, a2);
            }
        }
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if text_button(ui, "aiPickSeparate", "separate.", p.separate).on_hover_text(crate::i18n::tr("Create separate masks")).clicked() {
            p.separate = !p.separate;
        }
        let ready = if people { !p.people.is_empty() && !p.parts.is_empty() } else { !p.classes.is_empty() };
        if text_button(ui, "aiPickMake", "make.", false).on_hover_text(crate::i18n::tr("Create the mask")).clicked() {
            if !ready {
                app.toast(ui.ctx(), crate::i18n::tr(if people { "Pick a person and a part" } else { "Pick a class" }));
            } else {
                let mut errors = Vec::new();
                if people {
                    let parts: Vec<&str> = p.parts.iter().filter_map(|k| lightcraft_develop::ai::PEOPLE_PARTS.get(*k).map(|x| x.0)).collect();
                    for person in &p.people {
                        let name = format!("Person {}", person + 1);
                        if let Err(e) =
                            app.run("mask.add", json!({"kind": "people", "person": person, "parts": parts, "separate": p.separate, "name": name}))
                        {
                            errors.push(e);
                        }
                    }
                } else {
                    let classes: Vec<&str> =
                        p.classes.iter().filter_map(|k| lightcraft_develop::ai::LANDSCAPE_CLASSES.get(*k).map(|x| x.0)).collect();
                    if let Err(e) = app.run("mask.add", json!({"kind": "landscape", "classes": classes, "separate": p.separate})) {
                        errors.push(e);
                    }
                }
                match errors.into_iter().next() {
                    Some(e) => app.toast_error(ui.ctx(), e),
                    None => app.ui.ai_pick = None,
                }
            }
        }
    });
    if app.ui.ai_pick.is_some() {
        app.ui.ai_pick = Some(p);
    }
}

/// The Sky / Landscape / People models, when they aren't installed: what they are (size,
/// licences) and get. to download them (the row says what the click agrees to).
fn models_row(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let seg = &app.session.segmenter;
    if !lightcraft_engine::segment::Segmenter::AVAILABLE || seg.models.is_none() || seg.parsers_installed() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let d = seg.parsers_download_status();
    ui.add_space(12.0);
    if d.running {
        let pct = if d.total > 0 { d.done as f64 / d.total as f64 } else { 0.0 };
        let r =
            ui.add(egui::ProgressBar::new(pct as f32).desired_width(ui.available_width()).text(format!("AI models: {} %", (pct * 100.0).floor())));
        register(ui.ctx(), "maskModelsProgress", r.rect);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        return;
    }
    let bytes: u64 = seg.parsers().iter().filter(|p| !p.installed).map(|p| p.bytes).sum();
    ui.label(
        egui::RichText::new(format!(
            "{} ({:.0} MB, once). {}",
            crate::i18n::tr("Sky, Landscape and People masks are sharper with the AI models"),
            bytes as f64 / 1e6,
            crate::i18n::tr("Their terms: NVIDIA SegFormer licence and CelebAMask-HQ (non-commercial use), MIT.")
        ))
        .color(t.text_dim),
    );
    if let Some(e) = &d.error {
        ui.label(egui::RichText::new(format!("{} {e}", crate::i18n::tr("The download didn't work:"))).color(egui::Color32::from_rgb(230, 90, 80)));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if text_button(ui, "maskModelsGet", "get.", false).on_hover_text(crate::i18n::tr("Download the models (agreeing to their terms)")).clicked()
            && let Err(e) = app.run("segment.parsers.download", json!({"acknowledged": true}))
        {
            app.toast_error(ui.ctx(), e);
        }
        if text_button(ui, "maskModelsTerms", "terms.", false).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab("https://github.com/NVlabs/SegFormer/blob/master/LICENSE"));
        }
    });
}

/// Whether the SAM 3 model is missing in a build that could use it: then the download is
/// offered (with `then` to start afterwards) instead of starting an AI mask.
fn needs_model(app: &mut LightcraftApp, kind: &str, op: &str) -> bool {
    let seg = &app.session.segmenter;
    let missing = lightcraft_engine::segment::Segmenter::AVAILABLE && seg.dir.is_some() && !seg.installed();
    if missing {
        app.offer_sam_download(Some((kind, op)));
    }
    missing
}

/// Start an Object selection (SAM 3 clicks): a new mask, or a component of the selected one
/// combined by `op`; the photo is analyzed meanwhile (in the background).
pub(crate) fn start_object(app: &mut LightcraftApp, ctx: &egui::Context, op: &str) {
    if needs_model(app, "object", op) {
        return;
    }
    let r =
        if op == "new" { app.run("mask.add", json!({"kind": "object"})) } else { app.run("mask.addComponent", json!({"op": op, "kind": "object"})) };
    match r {
        Ok(_) => {
            app.ui.tool = "object".into();
            app.toast(ctx, crate::i18n::tr("Click the object to select it · ⌥-click leaves a part out"));
        }
        Err(e) => app.ai_error(ctx, e, Some(("object", op))),
    }
}

/// Start an AI mask of `kind` (object|prompt) combined by `op` (new|add|subtract|intersect),
/// after the model was installed.
pub(crate) fn begin_ai(app: &mut LightcraftApp, kind: &str, op: &str) -> Result<serde_json::Value, String> {
    match kind {
        "object" => {
            let r = if op == "new" {
                app.run("mask.add", json!({"kind": "object"}))
            } else {
                app.run("mask.addComponent", json!({"op": op, "kind": "object"}))
            }?;
            app.ui.tool = "object".into();
            Ok(r)
        }
        _ => {
            app.ui.describe = Some((op.to_string(), String::new()));
            Ok(serde_json::Value::Null)
        }
    }
}

/// Open the Describe field (a new mask, or a component combined by `op`).
pub(crate) fn start_describe(app: &mut LightcraftApp, op: &str) {
    if needs_model(app, "prompt", op) {
        return;
    }
    app.ui.describe = Some((op.to_string(), String::new()));
}

/// The Describe field: type what to select ("sky", "the red car") and press Return.
fn describe_field(app: &mut LightcraftApp, ui: &mut egui::Ui, new_mask: bool) {
    let Some((op, mut text)) = app.ui.describe.clone() else { return };
    // a new mask's field sits under the tiles; one that combines, under the selected mask
    if (op == "new") != new_mask {
        return;
    }
    ui.add_space(8.0);
    let prompt = match op.as_str() {
        "subtract" => "Describe what to leave out",
        "intersect" => "Describe what to keep",
        _ => "Describe what to select",
    };
    ui.label(crate::i18n::tr(prompt));
    let mut submit = false;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut text)
                .hint_text(crate::i18n::tr("e.g. sky · the red car · car, road"))
                .desired_width(ui.available_width() - 64.0),
        );
        register(ui.ctx(), "maskDescribe", r.rect);
        if !r.has_focus() && !r.lost_focus() && text.is_empty() {
            r.request_focus();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        if text_button(ui, "maskDescribeGo", crate::i18n::tr("Select"), false).clicked() {
            submit = true;
        }
    });
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.ui.describe = None;
        return;
    }
    app.ui.describe = Some((op.clone(), text.clone()));
    if submit && !text.trim().is_empty() {
        let r = if op == "new" {
            app.run("mask.add", json!({"kind": "prompt", "text": text}))
        } else {
            app.run("mask.addComponent", json!({"op": op, "kind": "prompt", "text": text}))
        };
        match r {
            // the mask appears when the model has found it (a detail pass follows)
            Ok(_) => app.ui.describe = None,
            Err(e) => app.ai_error(ui.ctx(), e, Some(("prompt", op.as_str()))),
        }
    }
}

fn component_menu(app: &mut LightcraftApp, ui: &mut egui::Ui, op: &str) {
    let b = ui.button(crate::i18n::tr("Object"));
    register(ui.ctx(), format!("maskComp:{op}:object"), b.rect);
    if b.clicked() {
        start_object(app, ui.ctx(), op);
    }
    let b = ui.button(crate::i18n::tr("Describe…"));
    register(ui.ctx(), format!("maskComp:{op}:prompt"), b.rect);
    if b.clicked() {
        start_describe(app, op);
    }
    for (kind, label) in [
        ("brush", "Brush"),
        ("linear", "Linear Gradient"),
        ("radial", "Radial Gradient"),
        ("sky", "Sky"),
        ("subject", "Subject"),
        ("background", "Background"),
        ("luminanceRange", "Luminance Range"),
        ("depthRange", "Depth Range"),
    ] {
        // painting only adds or erases
        if kind == "brush" && op == "intersect" {
            continue;
        }
        if ui.button(label).clicked() {
            if kind == "brush" {
                app.ui.tool = "brush".into();
                app.ui.brush_erase = op == "subtract";
            } else {
                let _ = app.run("mask.addComponent", json!({"op": op, "kind": kind}));
            }
        }
    }
}

fn brush_settings(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 6, bottom: 0 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(crate::i18n::tr("Brush")).font(t.semibold(13.0)));
            if text_button(ui, "brushAdd", crate::i18n::tr("Add"), !app.ui.brush_erase).clicked() {
                app.ui.brush_erase = false;
            }
            if text_button(ui, "brushErase", crate::i18n::tr("Erase"), app.ui.brush_erase).clicked() {
                app.ui.brush_erase = true;
            }
        });
        let mut auto = app.ui.brush_auto_mask;
        if ui
            .checkbox(&mut auto, crate::i18n::tr("Auto Mask"))
            .on_hover_text(crate::i18n::tr("Paint only areas like the one under the brush"))
            .changed()
        {
            app.ui.brush_auto_mask = auto;
        }
    });
    for (id, label, min, max, get) in [
        ("ui.brushSize", "Size", 1.0, 100.0, (app.ui.brush_size * 400.0) as f64),
        ("ui.brushFeather", "Feather", 0.0, 100.0, app.ui.brush_feather as f64),
        ("ui.brushFlow", "Flow", 1.0, 100.0, app.ui.brush_flow as f64),
    ] {
        let s = ControlSpec { id, label, section: Section::Light, min, max, default: min, step: 1.0, decimals: 0, track: Track::Plain };
        let out = slider(ui, &s, get, true, None);
        if let Some(v) = out.value {
            match id {
                "ui.brushSize" => app.ui.brush_size = (v / 400.0) as f32,
                "ui.brushFeather" => app.ui.brush_feather = v as f32,
                _ => app.ui.brush_flow = v as f32,
            }
        }
    }
}

#[cfg(test)]
mod pick_tests {
    use super::pick;

    /// People / Landscape chips pick like every list: click = just this one, ⌘-click adds or
    /// removes one, ⇧-click the run from the last click.
    #[test]
    fn chips_pick_with_click_command_and_shift() {
        let (mut sel, mut anchor) = (Vec::new(), None);
        let none = egui::Modifiers::NONE;
        pick(&mut sel, &mut anchor, 2, none);
        assert_eq!(sel, [2]);
        pick(&mut sel, &mut anchor, 5, egui::Modifiers::COMMAND);
        assert_eq!(sel, [2, 5]);
        pick(&mut sel, &mut anchor, 2, egui::Modifiers::COMMAND);
        assert_eq!(sel, [5]);
        pick(&mut sel, &mut anchor, 8, egui::Modifiers::SHIFT);
        assert_eq!(sel, [2, 3, 4, 5, 6, 7, 8], "the run from the last click (2)");
        pick(&mut sel, &mut anchor, 1, none);
        assert_eq!(sel, [1]);
        // ⇧ with nothing clicked yet: just that one
        let (mut sel, mut anchor) = (Vec::new(), None);
        pick(&mut sel, &mut anchor, 3, egui::Modifiers::SHIFT);
        assert_eq!(sel, [3]);
    }
}
