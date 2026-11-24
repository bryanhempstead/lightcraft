//! Masking ▸ Effect: Lightroom's local adjustment presets (`mask.effects`, `mask.applyEffect`,
//! `mask.saveEffect`, `mask.deleteEffect`, `mask.importEffects`). Picking one sets the selected
//! mask's sliders; save. keeps the mask's sliders as a new effect; import. reads the presets
//! Lightroom Classic / Camera Raw keep on this Mac (his `.lrtemplate` ones), in place.

use egui::RichText;
use lightcraft_develop::Mask;
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::register;

/// The Effect row above the mask's sliders.
pub fn effect_row(app: &mut LightcraftApp, ui: &mut egui::Ui, mask: &Mask) {
    let t = Tokens::get(ui.ctx());
    let list = app.session.execute("mask.effects", &json!({})).unwrap_or_default();
    let current = list["current"].as_str().map(str::to_string);
    let groups: Vec<(String, Vec<String>)> = list["groups"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|g| {
                    let names = g["effects"].as_array().map(|e| e.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
                    (g["name"].as_str().unwrap_or("").to_string(), names)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut chosen = None;
    egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 22, top: 8, bottom: 4 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(crate::i18n::tr("Effect")).color(t.text_dim));
            let shown = current.clone().unwrap_or_else(|| crate::i18n::tr("Custom").into());
            let width = (ui.available_width() - 8.0).max(80.0);
            let c = egui::ComboBox::from_id_salt(("maskEffect", mask.id)).width(width).selected_text(shown).show_ui(ui, |ui| {
                if groups.is_empty() {
                    ui.label(RichText::new(crate::i18n::tr("No effects yet: save. one, or import. yours from Lightroom")).color(t.text_dim));
                }
                for (gi, (group, names)) in groups.iter().enumerate() {
                    if !group.is_empty() {
                        if gi > 0 {
                            ui.add_space(4.0);
                        }
                        ui.label(RichText::new(group).font(t.semibold(11.5)).color(t.text_dim));
                    }
                    for n in names {
                        let r = ui.selectable_label(current.as_deref() == Some(n.as_str()), n);
                        register(ui.ctx(), format!("maskEffect:{n}"), r.rect);
                        if r.clicked() {
                            chosen = Some(n.clone());
                        }
                    }
                }
            });
            register(ui.ctx(), "combo:maskEffect", c.response.rect);
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);
            let save = ui.button("save.").on_hover_text(crate::i18n::tr("Save this mask's sliders as a new effect"));
            register(ui.ctx(), "button:maskEffectSave", save.rect);
            if save.clicked() {
                crate::panels::dialogs::prompt(app, "Save Effect", "Effect name", "", "mask.saveEffect", json!({"id": mask.id}), "name");
            }
            if let Some(name) = &current {
                let del = ui.button("delete.").on_hover_text(crate::i18n::tr("Delete this effect (the mask keeps its sliders)"));
                register(ui.ctx(), "button:maskEffectDelete", del.rect);
                if del.clicked()
                    && let Err(e) = app.run("mask.deleteEffect", json!({"name": name}))
                {
                    app.toast_error(ui.ctx(), e);
                }
            }
            let import = ui.button("import.").on_hover_text(crate::i18n::tr(
                "Read the local adjustment presets Lightroom Classic and Camera Raw keep on this Mac (Burn, Dodge, Soften Skin…); same-named effects are replaced",
            ));
            register(ui.ctx(), "button:maskEffectImport", import.rect);
            if import.clicked() {
                match app.run("mask.importEffects", json!({"lightroom": true})) {
                    Ok(v) => {
                        let n = v["imported"].as_array().map_or(0, Vec::len);
                        app.toast(ui.ctx(), format!("Imported {n} effects"));
                    }
                    Err(e) => app.toast_error(ui.ctx(), e),
                }
            }
        });
    });
    if let Some(name) = chosen
        && let Err(e) = app.run("mask.applyEffect", json!({"name": name, "id": mask.id}))
    {
        app.toast_error(ui.ctx(), e);
    }
}
