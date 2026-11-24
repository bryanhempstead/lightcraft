//! Accent colours (Settings ▸ Interface ▸ accents., `app.accents`): up to four active accents,
//! picked from a colour wheel, presets or the owner's saved swatches, split across the areas
//! ("roles") of the UI that are highlighted with an accent. A role can be pinned to a slot; an
//! unpinned role (auto.) gets a slot by [`auto_slot`]: one accent colours everything, two split
//! controls from badges, and so on. With no accent picked every area keeps the classic look.
//!
//! Saved in [`crate::state::AppSettings::accents`] (`ui.json`, `accents`) and applied live through
//! [`crate::theme::Tokens::with_accents`] (the app re-applies the theme the frame after a change).

use std::collections::BTreeMap;

use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::identity::{parse_hex, to_hex};
use crate::theme::{Tokens, text_on};
use crate::widgets::register;

/// How many accents can be active at once.
pub const SLOTS: usize = 4;
/// Most saved swatches kept.
pub const MAX_SAVED: usize = 24;
/// The classic accent (the app's blue).
pub const CLASSIC: &str = "#0165dd";

/// Presets offered next to the wheel: the app's blue, the lime of the "Lr" app icon and a few more.
pub const PRESETS: [(&str, &str); 10] = [
    ("blue", CLASSIC),
    ("lime", "#aacc00"),
    ("teal", "#14b8a6"),
    ("mint", "#3ecf8e"),
    ("sky", "#38a8f0"),
    ("violet", "#8b5cf6"),
    ("pink", "#e85a74"),
    ("red", "#e04a4a"),
    ("orange", "#f08a24"),
    ("gold", "#e3b23c"),
];

/// An area of the UI an accent drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    /// Selection and focus: selected text and items, focus rings, drop targets.
    Selection,
    /// Active tools and toggles: the tool strip, the picked slider, active toolbar and text buttons.
    Tools,
    /// Develop sliders: the thumb and the track's fill.
    Sliders,
    /// Badges and highlights: fav.'s heart, pick flags, counts, merge tags.
    Badges,
    /// The module picker's current module and the open panels' headings.
    Tabs,
}

impl Role {
    pub const ALL: [Role; 5] = [Role::Selection, Role::Tools, Role::Sliders, Role::Badges, Role::Tabs];

    /// The id used in `ui.json` and `app.accents`.
    pub fn id(self) -> &'static str {
        match self {
            Role::Selection => "selection",
            Role::Tools => "tools",
            Role::Sliders => "sliders",
            Role::Badges => "badges",
            Role::Tabs => "tabs",
        }
    }

    pub fn from_id(s: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|r| r.id() == s)
    }

    /// The short label in Settings (Bryan's style: lowercase with a period).
    pub fn label(self) -> &'static str {
        match self {
            Role::Selection => "selection.",
            Role::Tools => "tools.",
            Role::Sliders => "sliders.",
            Role::Badges => "badges.",
            Role::Tabs => "tabs.",
        }
    }

    /// What it colours, for the hint next to it.
    pub fn about(self) -> &'static str {
        match self {
            Role::Selection => "selection, focus rings",
            Role::Tools => "tool strip, active buttons, picked slider",
            Role::Sliders => "slider thumbs and fill",
            Role::Badges => "fav. heart, picks, counts, tags",
            Role::Tabs => "current module, open panel headings",
        }
    }
}

/// The auto. split: which of the `n` active accents (in slot order, 0-based) `role` gets.
/// 1 = everything; 2 = controls (selection, tools, sliders) vs badges and tabs; 3 = selection and
/// tabs / tools and sliders / badges; 4 = selection / tools and sliders / badges / tabs.
pub fn auto_slot(role: Role, n: usize) -> usize {
    let k = match (n, role) {
        (0 | 1, _) => 0,
        (2, Role::Selection | Role::Tools | Role::Sliders) => 0,
        (2, Role::Badges | Role::Tabs) => 1,
        (3, Role::Selection | Role::Tabs) => 0,
        (3, Role::Tools | Role::Sliders) => 1,
        (3, Role::Badges) => 2,
        (_, Role::Selection) => 0,
        (_, Role::Tools | Role::Sliders) => 1,
        (_, Role::Badges) => 2,
        (_, Role::Tabs) => 3,
    };
    k.min(n.saturating_sub(1))
}

/// The accent settings (`ui.json` → `settings.accents`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Accents {
    /// The active accents, `"#rrggbb"` or null for an empty slot (at most [`SLOTS`]).
    pub slots: Vec<Option<String>>,
    /// Roles pinned to a slot: role id → slot 1–4. A role not listed is auto.
    pub roles: BTreeMap<String, u8>,
    /// The owner's saved swatches (`save.`), oldest first.
    pub saved: Vec<String>,
    /// The colour on the wheel.
    pub wheel: String,
}

impl Default for Accents {
    fn default() -> Self {
        Accents { slots: vec![None; SLOTS], roles: BTreeMap::new(), saved: Vec::new(), wheel: CLASSIC.into() }
    }
}

fn norm(s: &str) -> Option<String> {
    parse_hex(s).map(to_hex)
}

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

impl Accents {
    /// Whatever came from a file or a command, made valid: four slots of valid colours, known roles
    /// pinned to 1–4, saved swatches valid, unique and at most [`MAX_SAVED`].
    pub fn sanitize(&mut self) {
        let mut slots: Vec<Option<String>> = self.slots.iter().take(SLOTS).map(|s| s.as_deref().and_then(norm)).collect();
        slots.resize(SLOTS, None);
        self.slots = slots;
        self.roles.retain(|k, v| Role::from_id(k).is_some() && (1..=SLOTS as u8).contains(v));
        let mut saved: Vec<String> = Vec::new();
        for s in self.saved.iter().filter_map(|s| norm(s)) {
            if !saved.contains(&s) {
                saved.push(s);
            }
        }
        let extra = saved.len().saturating_sub(MAX_SAVED);
        saved.drain(..extra);
        self.saved = saved;
        self.wheel = norm(&self.wheel).unwrap_or_else(|| CLASSIC.into());
    }

    /// A slot's colour (0-based), when it holds one.
    pub fn slot(&self, i: usize) -> Option<[u8; 3]> {
        self.slots.get(i).and_then(|s| s.as_deref()).and_then(parse_hex)
    }

    /// The filled slots' indices, in order.
    pub fn filled(&self) -> Vec<usize> {
        (0..SLOTS).filter(|&i| self.slot(i).is_some()).collect()
    }

    /// The slot a role is pinned to (0-based), if any.
    pub fn pin(&self, role: Role) -> Option<usize> {
        self.roles.get(role.id()).map(|&n| usize::from(n).saturating_sub(1))
    }

    /// The slot (0-based) whose colour `role` uses: its pin when that slot holds a colour, else
    /// the auto. split over the filled slots. None when no accent is picked.
    pub fn source(&self, role: Role) -> Option<usize> {
        if let Some(p) = self.pin(role)
            && self.slot(p).is_some()
        {
            return Some(p);
        }
        let filled = self.filled();
        filled.get(auto_slot(role, filled.len())).copied()
    }

    /// Each role's colour (in [`Role::ALL`] order), or None for the classic look.
    pub fn resolve(&self) -> Option<[[u8; 3]; 5]> {
        let mut out = [[0u8; 3]; 5];
        for (o, r) in out.iter_mut().zip(Role::ALL) {
            *o = self.source(r).and_then(|s| self.slot(s))?;
        }
        Some(out)
    }

    /// Put `hex` in slot `i` (0-based); None empties it.
    pub fn set_slot(&mut self, i: usize, hex: Option<&str>) {
        self.sanitize();
        if let Some(s) = self.slots.get_mut(i) {
            *s = hex.and_then(norm);
        }
    }

    /// Add `hex` to the saved swatches (moved to the end when already there).
    pub fn save(&mut self, hex: &str) {
        if let Some(h) = norm(hex) {
            self.saved.retain(|s| s != &h);
            self.saved.push(h);
            self.sanitize();
        }
    }
}

// ------------------------------------------------------------------------------ the command

fn slot_value(v: &Value) -> Result<Option<String>, String> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) if s.trim().is_empty() || s.trim() == "none" => Ok(None),
        Value::String(s) => norm(s).map(Some).ok_or_else(|| format!("app.accents: {s:?} is not a #rrggbb color")),
        _ => Err("app.accents: a slot is \"#rrggbb\" or null".into()),
    }
}

/// `app.accents {slots?, roles?, saved?, wheel?}`: with no parameters, the state. `slots`: up to
/// four `"#rrggbb"` / null (an object `{"2": "#aacc00"}` changes only those slots, 1-based);
/// `roles`: `{role: "auto" | 1–4}` changes only the roles given (selection, tools, sliders, badges,
/// tabs); `saved`: the saved swatches (replaced); `save`: one colour added to them; `wheel`: the
/// wheel's colour. A bad value changes nothing. Returns the state with each role's colour.
pub fn command(app: &mut crate::LightcraftApp, p: &Value) -> Result<Value, String> {
    let mut a = app.ui.settings.accents.clone();
    a.sanitize();
    let Some(obj) = p.as_object().filter(|o| !o.is_empty()) else { return Ok(state(&a)) };
    for (k, v) in obj {
        match k.as_str() {
            "slots" => match v {
                Value::Array(list) => {
                    if list.len() > SLOTS {
                        return Err(format!("app.accents: at most {SLOTS} slots"));
                    }
                    let mut slots = list.iter().map(slot_value).collect::<Result<Vec<_>, _>>()?;
                    slots.resize(SLOTS, None);
                    a.slots = slots;
                }
                Value::Object(m) => {
                    for (n, c) in m {
                        let i = n
                            .parse::<usize>()
                            .ok()
                            .filter(|i| (1..=SLOTS).contains(i))
                            .ok_or_else(|| format!("app.accents: no slot {n:?} (1–{SLOTS})"))?;
                        let c = slot_value(c)?;
                        a.set_slot(i - 1, c.as_deref());
                    }
                }
                _ => return Err("app.accents: `slots` is a list of \"#rrggbb\" / null".into()),
            },
            "roles" => {
                let m = v.as_object().ok_or("app.accents: `roles` is {role: \"auto\" | 1–4}")?;
                for (r, pin) in m {
                    let role =
                        Role::from_id(r).ok_or_else(|| format!("app.accents: unknown role {r:?} (selection, tools, sliders, badges, tabs)"))?;
                    let n = match pin {
                        Value::Null => None,
                        Value::String(s) if s == "auto" || s == "auto." => None,
                        Value::String(s) => {
                            Some(s.trim_end_matches('.').parse::<u8>().map_err(|_| format!("app.accents: {r}: {s:?} is not auto or 1–4"))?)
                        }
                        Value::Number(n) => {
                            Some(n.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(|| format!("app.accents: {r}: {n} is not 1–4"))?)
                        }
                        _ => return Err(format!("app.accents: {r}: auto or 1–4")),
                    };
                    match n {
                        None | Some(0) => {
                            a.roles.remove(role.id());
                        }
                        Some(n) if usize::from(n) <= SLOTS => {
                            a.roles.insert(role.id().into(), n);
                        }
                        Some(n) => return Err(format!("app.accents: {r}: no slot {n} (1–{SLOTS})")),
                    }
                }
            }
            "saved" => {
                let list = v.as_array().ok_or("app.accents: `saved` is a list of \"#rrggbb\"")?;
                a.saved = list
                    .iter()
                    .map(|c| c.as_str().and_then(norm).ok_or_else(|| format!("app.accents: saved {c} is not a #rrggbb color")))
                    .collect::<Result<_, _>>()?;
            }
            "save" => {
                let s = v.as_str().and_then(norm).ok_or_else(|| format!("app.accents: save {v} is not a #rrggbb color"))?;
                a.save(&s);
            }
            "wheel" => {
                a.wheel = v.as_str().and_then(norm).ok_or_else(|| format!("app.accents: wheel {v} is not a #rrggbb color"))?;
            }
            other => return Err(format!("app.accents: unknown parameter `{other}` (slots, roles, saved, save, wheel)")),
        }
    }
    a.sanitize();
    app.ui.settings.accents = a;
    Ok(state(&app.ui.settings.accents))
}

/// The settings plus `resolved` (each role's colour and the slot it comes from; null with no accent
/// picked) and the presets.
pub fn state(a: &Accents) -> Value {
    let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
    let resolved: serde_json::Map<String, Value> = Role::ALL
        .into_iter()
        .map(|r| {
            let s = a.source(r);
            let c = s.and_then(|s| a.slot(s));
            (
                r.id().to_string(),
                c.map_or(Value::Null, |c| json!({"color": to_hex(c), "slot": s.map(|s| s + 1), "pinned": a.pin(r).is_some_and(|p| Some(p) == s)})),
            )
        })
        .collect();
    if let Some(o) = v.as_object_mut() {
        o.insert("resolved".into(), Value::Object(resolved));
        o.insert("presets".into(), json!(PRESETS.iter().map(|(n, h)| json!({"name": n, "color": h})).collect::<Vec<_>>()));
    }
    v
}

// ------------------------------------------------------------------------------ Settings section

const LABEL_W: f32 = 150.0;
const ROW_H: f32 = 24.0;
const GAP: f32 = 8.0;

/// Memory key: the slot a clicked swatch goes into.
fn target_id() -> egui::Id {
    egui::Id::new("accents-target-slot")
}

fn label_cell(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    ui.allocate_ui_with_layout(vec2(LABEL_W, ROW_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(LABEL_W);
        ui.label(RichText::new(text).color(t.text_label));
    });
}

/// A colour square that can be clicked (`swatch:{id}`); `on` outlines it. Returns its response.
fn swatch(ui: &mut egui::Ui, t: &Tokens, id: &str, hex: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ROW_H, ROW_H), Sense::click());
    register(ui.ctx(), format!("swatch:{id}"), r);
    let c = parse_hex(hex).map_or(t.field, rgb);
    let p = ui.painter();
    p.rect_filled(r.shrink(1.0), 4.0, c);
    let ring = if on {
        Stroke::new(2.0, t.text)
    } else if resp.hovered() {
        Stroke::new(1.0, t.text_label)
    } else {
        Stroke::new(1.0, t.field_border)
    };
    p.rect_stroke(r.shrink(1.0), 4.0, ring, StrokeKind::Inside);
    resp
}

/// Settings ▸ Interface ▸ accents.: the wheel with brightness, hex, use. and save.; the presets and
/// saved swatches; the four active slots; each role's slot (auto. or pinned).
pub fn section(app: &mut crate::LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.add_space(4.0);
    // folds like every Settings group (remembered, crate::folds)
    if crate::folds::header(ui, "settings.interface.accents", "accents.") {
        ui.scope(|ui| section_body(app, ui, t));
    }
}

fn section_body(app: &mut crate::LightcraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let before = app.ui.settings.accents.clone();
    let mut a = before.clone();
    a.sanitize();
    let ctx = ui.ctx().clone();
    // the slot a swatch goes into: the one last clicked, else the first empty one
    let target = ctx
        .data(|d| d.get_temp::<usize>(target_id()))
        .filter(|&i| i < SLOTS)
        .unwrap_or_else(|| (0..SLOTS).find(|&i| a.slot(i).is_none()).unwrap_or(0));
    let mut put: Option<String> = None;

    ui.spacing_mut().item_spacing.y = 10.0;

    // the wheel, its brightness, the hex value, use. and save.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        label_cell(ui, t, "wheel");
        wheel(ui, t, &mut a, &mut put);
    });
    // presets
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        label_cell(ui, t, "presets");
        for (name, hex) in PRESETS {
            if swatch(ui, t, &format!("accentPreset-{name}"), hex, a.wheel == hex).on_hover_text(name).clicked() {
                put = Some(hex.to_string());
            }
        }
    });
    // saved swatches (right-click removes one)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        label_cell(ui, t, "saved");
        if a.saved.is_empty() {
            ui.label(RichText::new("save. keeps the wheel's color here").color(t.text_dim));
        }
        let mut remove = None;
        ui.scope(|ui| {
            ui.set_max_width(10.0 * (ROW_H + GAP));
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, 10.0);
                for (i, hex) in a.saved.iter().enumerate() {
                    let r = swatch(ui, t, &format!("accentSaved-{i}"), hex, a.wheel == *hex).on_hover_text(format!("{hex} — right-click removes it"));
                    if r.clicked() {
                        put = Some(hex.clone());
                    }
                    if r.secondary_clicked() {
                        remove = Some(i);
                    }
                }
            });
        });
        if let Some(i) = remove
            && i < a.saved.len()
        {
            a.saved.remove(i);
        }
    });
    // the four active slots
    let mut new_target = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        label_cell(ui, t, "active");
        for i in 0..SLOTS {
            if i > 0 {
                ui.add_space(4.0);
            }
            let (r, resp) = ui.allocate_exact_size(vec2(36.0, ROW_H), Sense::click());
            register(ui.ctx(), format!("swatch:accentSlot-{}", i + 1), r);
            let c = a.slot(i);
            let p = ui.painter();
            let fill = c.map_or(t.field, rgb);
            p.rect_filled(r, 4.0, fill);
            let ring = if i == target { Stroke::new(2.0, t.text) } else { Stroke::new(1.0, t.field_border) };
            p.rect_stroke(r, 4.0, ring, StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, format!("{}", i + 1), t.semibold(11.5), if c.is_some() { text_on(fill) } else { t.text_label });
            let tip = if c.is_some() { "click: the next swatch goes here · right-click: empty it" } else { "click: the next swatch goes here" };
            let resp = resp.on_hover_text(tip);
            if resp.clicked() {
                new_target = Some(i);
            }
            if resp.secondary_clicked() {
                a.set_slot(i, None);
                new_target = Some(i);
            }
            // empty. empties it; an empty slot keeps the space so nothing moves
            if c.is_some() {
                if crate::widgets::text_button(ui, &format!("accentSlotClear-{}", i + 1), "empty.", false).on_hover_text("empty this slot").clicked()
                {
                    a.set_slot(i, None);
                    new_target = Some(i);
                }
            } else {
                ui.allocate_exact_size(vec2(51.0, ROW_H), Sense::hover());
            }
        }
    });
    // each role: auto. or a slot, the colour it gets, what it colours
    for role in Role::ALL {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            label_cell(ui, t, role.label());
            let cur = a.pin(role);
            let text = |p: Option<usize>| p.map_or("auto.".to_string(), |p| format!("{}.", p + 1));
            let mut pick = cur;
            let r = egui::ComboBox::from_id_salt(("accentRole", role.id())).width(64.0).selected_text(text(cur)).show_ui(ui, |ui| {
                ui.selectable_value(&mut pick, None, "auto.");
                for i in 0..SLOTS {
                    ui.selectable_value(&mut pick, Some(i), text(Some(i)));
                }
            });
            register(ui.ctx(), format!("combo:accentRole-{}", role.id()), r.response.rect);
            if pick != cur {
                match pick {
                    Some(i) => {
                        a.roles.insert(role.id().into(), (i + 1) as u8);
                    }
                    None => {
                        a.roles.remove(role.id());
                    }
                }
            }
            // the colour it gets now (the classic look when no accent is picked)
            let got = a.source(role).and_then(|s| a.slot(s));
            let (cr, _) = ui.allocate_exact_size(vec2(ROW_H, ROW_H), Sense::hover());
            register(ui.ctx(), format!("swatch:accentRoleColor-{}", role.id()), cr);
            let p = ui.painter();
            p.rect_filled(cr.shrink(3.0), 3.0, got.map_or(t.accent, rgb));
            p.rect_stroke(cr.shrink(3.0), 3.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
            let mut about = role.about().to_string();
            if a.pin(role).is_some() && a.source(role) != a.pin(role) {
                about.push_str(" (slot empty: auto.)");
            }
            ui.label(RichText::new(about).color(t.text_dim));
        });
    }
    hint(
        ui,
        t,
        "auto.: one accent colors everything; two split the controls (selection., tools., sliders.) from badges. and tabs.; three and four spread further. Pick a slot to pin an area to it. No accent: the classic look.",
    );

    if let Some(hex) = put {
        a.wheel = hex.clone();
        a.set_slot(target, Some(&hex));
        // the next swatch goes into the next empty slot (or stays here when all are full)
        new_target = Some((0..SLOTS).map(|k| (target + 1 + k) % SLOTS).find(|&i| a.slot(i).is_none()).unwrap_or(target));
    }
    if let Some(i) = new_target {
        ctx.data_mut(|d| d.insert_temp(target_id(), i));
    }
    a.sanitize();
    if a != before {
        app.ui.settings.accents = a;
        // applied now, so this frame's later widgets and the next frame match
        crate::theme::sync_accents(&ctx, &app.ui.settings.accents);
        ctx.request_repaint();
    }
}

/// A quiet note that wraps inside what is visible.
fn hint(ui: &mut egui::Ui, t: &Tokens, text: &str) {
    let w = (ui.clip_rect().right() - ui.cursor().left() - 8.0).min(ui.available_width()).max(120.0);
    ui.scope(|ui| {
        ui.set_max_width(w);
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(t.text_dim)).wrap());
    });
}

/// The hue / saturation wheel (the identity plate's, `panels::edit::paint_hue_wheel`), with a
/// brightness slider, the hex field, a chip of the color, use. (into the target slot) and save.
fn wheel(ui: &mut egui::Ui, t: &Tokens, a: &mut Accents, put: &mut Option<String>) {
    let cur = parse_hex(&a.wheel).unwrap_or([1, 101, 221]);
    let [h, s, v] = lightcraft_color::perceptual::rgb_to_hsv([f32::from(cur[0]) / 255.0, f32::from(cur[1]) / 255.0, f32::from(cur[2]) / 255.0]);
    let hex_of = |h: f32, s: f32, v: f32| {
        let c = lightcraft_color::perceptual::hsv_to_rgb(h, s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        to_hex([b(c[0]), b(c[1]), b(c[2])])
    };
    let rad = 52.0;
    let (area, _) = ui.allocate_exact_size(vec2(rad * 2.0 + 4.0, rad * 2.0 + 4.0), Sense::hover());
    let c = area.center();
    let wheel = Rect::from_center_size(c, vec2(rad * 2.0, rad * 2.0));
    let resp = ui.interact(wheel, egui::Id::new("accent-wheel"), Sense::click_and_drag());
    register(ui.ctx(), "wheel:accent", wheel);
    crate::panels::edit::paint_hue_wheel(ui.painter(), c, rad, Color32::from_gray((v * 255.0).round() as u8), 1.0, v.max(0.05));
    let ang = h.to_radians();
    let dot = c + vec2(ang.cos(), -ang.sin()) * rad * s;
    ui.painter().circle_filled(dot, 5.0, Color32::WHITE);
    ui.painter().circle_stroke(dot, 5.0, Stroke::new(1.0, Color32::BLACK));
    if (resp.dragged() || resp.clicked())
        && let Some(q) = resp.interact_pointer_pos()
    {
        let d = q - c;
        let hue = ((-d.y).atan2(d.x).to_degrees() + 360.0) % 360.0;
        a.wheel = hex_of(hue, (d.length() / rad).min(1.0), v.max(0.2));
    }
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 10.0;
        ui.spacing_mut().slider_width = 200.0;
        let mut bright = v * 100.0;
        let r = ui.add(egui::Slider::new(&mut bright, 0.0..=100.0).show_value(false));
        register(ui.ctx(), "slider:accentBrightness", r.rect);
        if r.on_hover_text("brightness").changed() {
            a.wheel = hex_of(h, s, bright / 100.0);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let mut hex = a.wheel.clone();
            let r = ui.add_sized([96.0, ROW_H], egui::TextEdit::singleline(&mut hex).hint_text("#rrggbb"));
            register(ui.ctx(), "field:accentHex", r.rect);
            if r.changed()
                && let Some(n) = norm(&hex)
            {
                a.wheel = n;
            }
            let (chip, _) = ui.allocate_exact_size(vec2(40.0, ROW_H), Sense::hover());
            let fill = rgb(cur);
            ui.painter().rect_filled(chip, 4.0, fill);
            ui.painter().text(chip.center(), Align2::CENTER_CENTER, "Aa", t.semibold(11.5), text_on(fill));
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            if crate::widgets::text_button(ui, "accentUse", "use.", false).on_hover_text("put the wheel's color in the outlined slot").clicked() {
                *put = Some(a.wheel.clone());
            }
            if crate::widgets::text_button(ui, "accentSave", "save.", false)
                .on_hover_text("keep the wheel's color with your saved swatches")
                .clicked()
            {
                let w = a.wheel.clone();
                a.save(&w);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(slots: &[&str]) -> Accents {
        let mut a = Accents { slots: slots.iter().map(|s| Some(s.to_string())).collect(), ..Default::default() };
        a.sanitize();
        a
    }

    #[test]
    fn no_accent_is_the_classic_look() {
        let a = Accents::default();
        assert_eq!(a.resolve(), None);
        let t = Tokens::with_accents(&a);
        let d = Tokens::default();
        assert_eq!((t.accent, t.fav, t.pick), (d.accent, d.fav, d.pick));
        assert_eq!((t.tools_accent, t.slider_accent, t.badge_accent, t.tab_accent), (None, None, None, None));
    }

    #[test]
    fn auto_split_by_count() {
        let lime = [0xaa, 0xcc, 0x00];
        let one = with(&["#aacc00"]);
        assert_eq!(one.resolve(), Some([lime; 5]), "one accent colours everything");
        let two = with(&["#aacc00", "#e85a74"]);
        let src: Vec<_> = Role::ALL.iter().map(|r| two.source(*r)).collect();
        assert_eq!(src, vec![Some(0), Some(0), Some(0), Some(1), Some(1)], "controls vs badges and tabs");
        let three = with(&["#aacc00", "#e85a74", "#14b8a6"]);
        let src: Vec<_> = Role::ALL.iter().map(|r| three.source(*r)).collect();
        assert_eq!(src, vec![Some(0), Some(1), Some(1), Some(2), Some(0)]);
        let four = with(&["#aacc00", "#e85a74", "#14b8a6", "#e3b23c"]);
        let src: Vec<_> = Role::ALL.iter().map(|r| four.source(*r)).collect();
        assert_eq!(src, vec![Some(0), Some(1), Some(1), Some(2), Some(3)]);
        // gaps: slots 2 and 4 filled act as the first and second accent
        let mut gaps = Accents::default();
        gaps.set_slot(1, Some("#aacc00"));
        gaps.set_slot(3, Some("#e85a74"));
        assert_eq!(gaps.source(Role::Selection), Some(1));
        assert_eq!(gaps.source(Role::Badges), Some(3));
    }

    #[test]
    fn pinned_roles_win_and_fall_back_when_empty() {
        let mut a = with(&["#aacc00", "#e85a74"]);
        a.roles.insert("selection".into(), 2);
        a.roles.insert("badges".into(), 1);
        assert_eq!(a.source(Role::Selection), Some(1));
        assert_eq!(a.source(Role::Badges), Some(0));
        assert_eq!(a.source(Role::Tools), Some(0), "the others stay auto.");
        // pinned to an empty slot: auto. until it is filled
        a.roles.insert("tabs".into(), 4);
        assert_eq!(a.source(Role::Tabs), Some(1));
        a.set_slot(3, Some("#14b8a6"));
        assert_eq!(a.source(Role::Tabs), Some(3));
    }

    #[test]
    fn tokens_pick_up_the_roles() {
        let mut a = with(&["#aacc00", "#e85a74"]);
        a.roles.insert("sliders".into(), 2);
        let t = Tokens::with_accents(&a);
        let (lime, pink) = (Color32::from_rgb(0xaa, 0xcc, 0x00), Color32::from_rgb(0xe8, 0x5a, 0x74));
        assert_eq!(t.accent, lime, "selection");
        assert_eq!(t.tools_accent, Some(lime));
        assert_eq!(t.slider_accent, Some(pink), "pinned");
        assert_eq!(t.badge_accent, Some(pink));
        assert_eq!((t.fav, t.pick), (pink, pink), "badges colour the heart and pick flags");
        assert_eq!(t.tab_accent, Some(pink));
        // readable text on the fills
        assert_eq!(text_on(lime), Color32::BLACK);
        assert_eq!(text_on(Color32::from_rgb(1, 101, 221)), Color32::WHITE);
    }

    #[test]
    fn sanitize_keeps_files_valid() {
        let mut a = Accents {
            slots: vec![Some("#ABC".into()), Some("nope".into()), None, Some("#aacc00".into()), Some("#ffffff".into())],
            roles: [("selection".to_string(), 3u8), ("bogus".to_string(), 1), ("tabs".to_string(), 9)].into_iter().collect(),
            saved: vec!["#aacc00".into(), "#AACC00".into(), "x".into()],
            wheel: "??".into(),
        };
        a.sanitize();
        assert_eq!(a.slots, vec![Some("#aabbcc".into()), None, None, Some("#aacc00".into())]);
        assert_eq!(a.roles.len(), 1);
        assert_eq!(a.saved, vec!["#aacc00".to_string()]);
        assert_eq!(a.wheel, CLASSIC);
        let mut many = Accents { saved: (0..40).map(|i| format!("#0000{i:02x}")).collect(), ..Default::default() };
        many.sanitize();
        assert_eq!(many.saved.len(), MAX_SAVED);
    }

    #[test]
    fn settings_round_trip() {
        let mut s = crate::state::AppSettings::default();
        s.accents = with(&["#aacc00", "#e85a74"]);
        s.accents.roles.insert("badges".into(), 1);
        s.accents.saved = vec!["#123456".into()];
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["accents"]["slots"], json!(["#aacc00", "#e85a74", null, null]), "{j}");
        assert_eq!(j["accents"]["roles"], json!({"badges": 1}));
        let back: crate::state::AppSettings = serde_json::from_value(j).unwrap();
        assert_eq!(back.accents, s.accents);
        // an older ui.json without accents: the classic look
        let old: crate::state::AppSettings = serde_json::from_value(json!({"gpu": true})).unwrap();
        assert_eq!(old.accents, Accents::default());
    }

    /// Settings ▸ Interface ▸ accents.: presets fill the slots in turn, the theme follows on the
    /// next frame, empty. empties a slot, `app.accents` pins a role; laid out with nothing clipped or
    /// overlapping and neighbours at least 8 px apart (screenshot with `LIGHTCRAFT_TEST_SHOTS`).
    #[test]
    fn settings_section_live_and_laid_out() {
        use crate::tests_library2::{click, headless, request, settle, shot};
        let mut h = headless(lightcraft_engine::Session::with_demo());
        h.app.run("app.settings", json!({"tab": "interface"})).unwrap();
        settle(&mut h);
        click(&mut h, "swatch:accentPreset-lime");
        click(&mut h, "swatch:accentPreset-pink");
        let a = &h.app.ui.settings.accents;
        assert_eq!(a.slots, vec![Some("#aacc00".into()), Some("#e85a74".into()), None, None], "presets fill the slots in turn");
        settle(&mut h);
        let t = Tokens::get(&h.view.ctx);
        let (lime, pink) = (Color32::from_rgb(0xaa, 0xcc, 0x00), Color32::from_rgb(0xe8, 0x5a, 0x74));
        assert_eq!((t.accent, t.tools_accent, t.badge_accent, t.fav), (lime, Some(lime), Some(pink), pink), "live");
        // the wheel's colour saved and put in slot 3
        let r = request(&mut h, "engine.execute", json!({"command": "app.accents", "params": {"wheel": "#14b8a6"}}));
        assert_eq!(r["result"]["wheel"], "#14b8a6", "{r}");
        click(&mut h, "button:accentSave");
        click(&mut h, "button:accentUse");
        let a = &h.app.ui.settings.accents;
        assert_eq!(a.saved, vec!["#14b8a6".to_string()]);
        assert_eq!(a.slots[2].as_deref(), Some("#14b8a6"));
        // empty. empties slot 2; badges. pinned to 3
        click(&mut h, "button:accentSlotClear-2");
        assert_eq!(h.app.ui.settings.accents.slots[1], None);
        let r = request(&mut h, "engine.execute", json!({"command": "app.accents", "params": {"roles": {"badges": 3}}}));
        assert_eq!(r["result"]["resolved"]["badges"], json!({"color": "#14b8a6", "slot": 3, "pinned": true}), "{r}");
        assert_eq!(r["result"]["resolved"]["tools"]["color"], "#aacc00");
        let bad = h.request("engine.execute", json!({"command": "app.accents", "params": {"slots": ["nope"]}}), crate::tests_library2::T);
        assert_eq!(bad["ok"], false, "{bad}");
        settle(&mut h);
        assert_eq!(Tokens::get(&h.view.ctx).badge_accent, Some(Color32::from_rgb(0x14, 0xb8, 0xa6)));
        // layout: the dialog's texts, and the section's controls 8 px apart inside the dialog
        let dialog = h.app.widgets.iter().find(|(w, _)| w == "dialog:window").map(|(_, r)| *r).unwrap();
        let faults = crate::tests_panels::layout_faults(&h, dialog);
        assert!(faults.is_empty(), "{faults:#?}");
        let ours: Vec<(String, egui::Rect)> = h.app.widgets.iter().filter(|(w, _)| w.contains(":accent")).cloned().collect();
        assert!(ours.len() > 20, "{ours:?}");
        for (i, (a, ra)) in ours.iter().enumerate() {
            assert!(dialog.contains_rect(*ra), "{a} {ra:?} inside the dialog {dialog:?}");
            for (b, rb) in ours.iter().skip(i + 1) {
                let same_row = ra.top().max(rb.top()) < ra.bottom().min(rb.bottom()) - 4.0;
                if same_row {
                    let gap = (rb.left() - ra.right()).max(ra.left() - rb.right());
                    assert!(gap >= 8.0 - 0.5, "{a} {ra:?} / {b} {rb:?}: {gap} px apart");
                }
            }
        }
        shot(&mut h, "accents-settings.png");
    }
}
