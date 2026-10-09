//! The user keymap (Settings ▸ shrt. and ctrl.): key, mouse-button and MIDI bindings layered over
//! the built-in shortcuts.
//!
//! Resolution, lowest to highest: the built-in shortcuts (command specs, `UI_COMMANDS`,
//! [`crate::shortcuts::ALIASES`], the rating/label digits) → the selected profile's table
//! (`lightroom` = the built-ins as they are, `classic` = [`CLASSIC`]) → the user's bindings in
//! `keymap.json`. A binding with an empty `command` unbinds its keys. The file lives in the
//! settings folder (`<config>/keymap.json`, or `LIGHTCRAFT_KEYMAP`), is re-read when it changes on
//! disk, and a damaged file keeps the defaults and says why (it is never overwritten then).
//!
//! The same file holds the MIDI mappings (Monogram Creator, any controller): a CC moves a develop
//! slider (absolute, or relative for endless dials) or runs a command; a note runs a command.
//! Importers read Bryan's LrKeys `bindings.json`, LrSuperKeys `Shortcuts.xml` / `SpeedKeys.xml`
//! and a Monogram `state.json` profile (all read-only).

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::{Key, Modifiers, PointerButton};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Key profiles: `(id, label)`.
pub const PROFILES: &[(&str, &str)] = &[("lightroom", "Lightroom (desktop)"), ("classic", "Lightroom Classic")];

/// Lightroom Classic's standard keys where they differ from the built-in (Lightroom desktop)
/// ones: `(keys, command, params)`; `""` command = unbound (Classic uses the key for something
/// LightCraft doesn't have, so it does nothing rather than something unexpected).
pub const CLASSIC: &[(&str, &str, &str)] = &[
    // modules / views
    ("G", "view.photoGrid", "{}"),
    ("E", "view.detail", "{}"),
    ("D", "view.develop", "{}"),
    ("C", "view.compare", "{}"),
    ("Cmd+Alt+1", "view.library", "{}"),
    ("Cmd+Alt+2", "view.develop", "{}"),
    ("I", "view.infoOverlay", "{}"),
    ("Shift+F", "view.enterFullScreen", "{}"),
    // develop tools
    ("R", "panel.crop", "{}"),
    ("Q", "panel.remove", "{}"),
    ("K", "tool.brush", "{}"),
    ("M", "tool.linear", "{}"),
    ("Shift+M", "tool.radial", "{}"),
    ("Shift+W", "panel.masking", "{}"),
    ("Shift+T", "tool.guidedUpright", "{}"),
    ("H", "view.maskPins", "{}"),
    ("Cmd+U", "develop.auto", "{}"),
    ("Cmd+Shift+U", "develop.wb", r#"{"mode": "auto"}"#),
    // library
    ("B", "album.toggleTarget", "{}"),
    ("Cmd+K", "panel.keywords", "{}"),
    ("Cmd+Shift+V", "develop.paste", "{}"),
    ("Cmd+Shift+S", "dialog.syncSettings", "{}"),
    ("Cmd+Alt+S", "develop.sync", "{}"),
    ("Cmd+Shift+N", "dialog.createPreset", "{}"),
    ("Cmd+Shift+E", "dialog.export", "{}"),
    ("Cmd+E", "photo.editInExternal", "{}"),
    ("L", "view.lightsOut", "{}"),
    // MX Master back / forward (LrSuperKeys "mouse back/forward = previous/next photo")
    ("MouseBack", "library.previous", "{}"),
    ("MouseForward", "library.next", "{}"),
];

/// Classic shortcuts with no LightCraft command (shown on the shrt. page and in the report).
pub const CLASSIC_SKIPPED: &[(&str, &str)] = &[
    ("`", "toggle flag"),
    ("Cmd+J", "Library view options"),
    ("Cmd+Shift+M", "email photos"),
    ("Cmd+Alt+3…7", "Map, Book, Slideshow, Print, Web modules"),
    ("Cmd+N (Develop)", "new snapshot — Cmd+N stays New Album (Classic's Library: New Collection)"),
];

/// Ready-made "super key" actions offered on the shrt. page: `(group, label, command, params)`.
pub const SUPER_ACTIONS: &[(&str, &str, &str, &str)] = &[
    ("crop", "crop as shot", "crop.aspect", r#"{"aspect": "original"}"#),
    ("crop", "crop 1x1", "crop.aspect", r#"{"aspect": "1x1"}"#),
    ("crop", "crop 4x5", "crop.aspect", r#"{"aspect": "4x5"}"#),
    ("crop", "crop 8x10", "crop.aspect", r#"{"aspect": "8x10"}"#),
    ("crop", "crop 16x9", "crop.aspect", r#"{"aspect": "16x9"}"#),
    ("crop", "crop 21x9", "crop.aspect", r#"{"aspect": "21x9"}"#),
    ("crop", "crop 2x3", "crop.aspect", r#"{"aspect": "2x3"}"#),
    ("crop", "crop 5x7", "crop.aspect", r#"{"aspect": "5x7"}"#),
    ("crop", "crop reset", "crop.reset", "{}"),
    ("crop", "auto straighten", "crop.autoStraighten", "{}"),
    ("develop", "paste from previous", "develop.pastePrevious", "{}"),
    ("develop", "exposure +1/3", "develop.adjust", r#"{"control": "light.exposure", "delta": 0.33}"#),
    ("develop", "exposure -1/3", "develop.adjust", r#"{"control": "light.exposure", "delta": -0.33}"#),
    ("develop", "contrast +5", "develop.adjust", r#"{"control": "light.contrast", "delta": 5}"#),
    ("develop", "contrast -5", "develop.adjust", r#"{"control": "light.contrast", "delta": -5}"#),
    ("develop", "before / after", "view.beforeAfter", "{}"),
    ("develop", "undo", "edit.undo", "{}"),
    ("library", "pick", "photo.flag", r#"{"flag": "pick"}"#),
    ("library", "reject", "photo.flag", r#"{"flag": "reject"}"#),
    ("library", "unflag", "photo.flag", r#"{"flag": "none"}"#),
    ("library", "label red", "photo.label", r#"{"label": "red"}"#),
    ("library", "label yellow", "photo.label", r#"{"label": "yellow"}"#),
    ("library", "label green", "photo.label", r#"{"label": "green"}"#),
    ("library", "label blue", "photo.label", r#"{"label": "blue"}"#),
    ("library", "label purple", "photo.label", r#"{"label": "purple"}"#),
    ("library", "next photo", "library.next", "{}"),
    ("library", "previous photo", "library.previous", "{}"),
    ("library", "left off", "view.resumeLastLeftOff", "{}"),
];

/// Lightroom Classic develop parameter names (LrKeys `param?name=`, LrSuperKeys `lrParam`,
/// Monogram's Lightroom integration inputs) → LightCraft develop control ids. `crop.x`, `crop.y`
/// and `crop.scale` are nudge-only pseudo controls ([`crate::menus`] `crop.nudge`).
pub const LR_PARAMS: &[(&str, &str)] = &[
    ("Exposure", "light.exposure"),
    ("Exposure2012", "light.exposure"),
    ("Contrast", "light.contrast"),
    ("Contrast2012", "light.contrast"),
    ("Highlights", "light.highlights"),
    ("Highlights2012", "light.highlights"),
    ("Shadows", "light.shadows"),
    ("Shadows2012", "light.shadows"),
    ("Whites", "light.whites"),
    ("Whites2012", "light.whites"),
    ("Blacks", "light.blacks"),
    ("Blacks2012", "light.blacks"),
    ("Temperature", "wb.temp"),
    ("Tint", "wb.tint"),
    ("Texture", "effects.texture"),
    ("Clarity", "effects.clarity"),
    ("Clarity2012", "effects.clarity"),
    ("Dehaze", "effects.dehaze"),
    ("Vibrance", "color.vibrance"),
    ("Saturation", "color.saturation"),
    ("ParametricHighlights", "curve.highlights"),
    ("ParametricLights", "curve.lights"),
    ("ParametricDarks", "curve.darks"),
    ("ParametricShadows", "curve.shadows"),
    ("CurveRefineSaturation", "curve.refineSaturation"),
    ("SplitToningShadowHue", "grading.shadows.hue"),
    ("SplitToningShadowSaturation", "grading.shadows.sat"),
    ("ColorGradeShadowLum", "grading.shadows.lum"),
    ("ColorGradeMidtoneHue", "grading.midtones.hue"),
    ("ColorGradeMidtoneSat", "grading.midtones.sat"),
    ("ColorGradeMidtoneLum", "grading.midtones.lum"),
    ("SplitToningHighlightHue", "grading.highlights.hue"),
    ("SplitToningHighlightSaturation", "grading.highlights.sat"),
    ("ColorGradeHighlightLum", "grading.highlights.lum"),
    ("ColorGradeGlobalHue", "grading.global.hue"),
    ("ColorGradeGlobalSat", "grading.global.sat"),
    ("ColorGradeGlobalLum", "grading.global.lum"),
    ("ColorGradeBlending", "grading.blending"),
    ("SplitToningBalance", "grading.balance"),
    ("HueAdjustmentRed", "mixer.red.hue"),
    ("HueAdjustmentOrange", "mixer.orange.hue"),
    ("HueAdjustmentYellow", "mixer.yellow.hue"),
    ("HueAdjustmentGreen", "mixer.green.hue"),
    ("HueAdjustmentAqua", "mixer.aqua.hue"),
    ("HueAdjustmentBlue", "mixer.blue.hue"),
    ("HueAdjustmentPurple", "mixer.purple.hue"),
    ("HueAdjustmentMagenta", "mixer.magenta.hue"),
    ("SaturationAdjustmentRed", "mixer.red.sat"),
    ("SaturationAdjustmentOrange", "mixer.orange.sat"),
    ("SaturationAdjustmentYellow", "mixer.yellow.sat"),
    ("SaturationAdjustmentGreen", "mixer.green.sat"),
    ("SaturationAdjustmentAqua", "mixer.aqua.sat"),
    ("SaturationAdjustmentBlue", "mixer.blue.sat"),
    ("SaturationAdjustmentPurple", "mixer.purple.sat"),
    ("SaturationAdjustmentMagenta", "mixer.magenta.sat"),
    ("LuminanceAdjustmentRed", "mixer.red.lum"),
    ("LuminanceAdjustmentOrange", "mixer.orange.lum"),
    ("LuminanceAdjustmentYellow", "mixer.yellow.lum"),
    ("LuminanceAdjustmentGreen", "mixer.green.lum"),
    ("LuminanceAdjustmentAqua", "mixer.aqua.lum"),
    ("LuminanceAdjustmentBlue", "mixer.blue.lum"),
    ("LuminanceAdjustmentPurple", "mixer.purple.lum"),
    ("LuminanceAdjustmentMagenta", "mixer.magenta.lum"),
    ("GrayMixerRed", "bw.red"),
    ("GrayMixerOrange", "bw.orange"),
    ("GrayMixerYellow", "bw.yellow"),
    ("GrayMixerGreen", "bw.green"),
    ("GrayMixerAqua", "bw.aqua"),
    ("GrayMixerBlue", "bw.blue"),
    ("GrayMixerPurple", "bw.purple"),
    ("GrayMixerMagenta", "bw.magenta"),
    ("Sharpness", "detail.sharpenAmount"),
    ("SharpenRadius", "detail.sharpenRadius"),
    ("SharpenDetail", "detail.sharpenDetail"),
    ("SharpenEdgeMasking", "detail.sharpenMasking"),
    ("LuminanceSmoothing", "detail.nrLuminance"),
    ("LuminanceNoiseReductionDetail", "detail.nrDetail"),
    ("LuminanceNoiseReductionContrast", "detail.nrContrast"),
    ("ColorNoiseReduction", "detail.nrColor"),
    ("ColorNoiseReductionDetail", "detail.nrColorDetail"),
    ("ColorNoiseReductionSmoothness", "detail.nrColorSmoothness"),
    ("PerspectiveVertical", "geometry.vertical"),
    ("PerspectiveHorizontal", "geometry.horizontal"),
    ("PerspectiveRotate", "geometry.rotate"),
    ("PerspectiveScale", "geometry.scale"),
    ("PerspectiveAspect", "geometry.aspect"),
    ("PerspectiveX", "geometry.offsetX"),
    ("PerspectiveY", "geometry.offsetY"),
    ("LensProfileDistortionScale", "optics.profileDistortion"),
    ("LensProfileVignettingScale", "optics.profileVignetting"),
    ("LensManualDistortionAmount", "optics.distortion"),
    ("DefringePurpleAmount", "optics.defringePurple"),
    ("DefringePurpleHueLo", "optics.defringePurpleHueLo"),
    ("DefringePurpleHueHi", "optics.defringePurpleHueHi"),
    ("DefringeGreenAmount", "optics.defringeGreen"),
    ("DefringeGreenHueLo", "optics.defringeGreenHueLo"),
    ("DefringeGreenHueHi", "optics.defringeGreenHueHi"),
    ("PostCropVignetteAmount", "vignette.amount"),
    ("PostCropVignetteMidpoint", "vignette.midpoint"),
    ("PostCropVignetteRoundness", "vignette.roundness"),
    ("PostCropVignetteFeather", "vignette.feather"),
    ("PostCropVignetteHighlightContrast", "vignette.highlights"),
    ("GrainAmount", "grain.amount"),
    ("GrainSize", "grain.size"),
    ("GrainFrequency", "grain.roughness"),
    ("ShadowTint", "calibration.shadowsTint"),
    ("RedHue", "calibration.redHue"),
    ("RedSaturation", "calibration.redSat"),
    ("GreenHue", "calibration.greenHue"),
    ("GreenSaturation", "calibration.greenSat"),
    ("BlueHue", "calibration.blueHue"),
    ("BlueSaturation", "calibration.blueSat"),
    ("ProfileAmount", "profile.amount"),
    ("straightenAngle", "crop.angle"),
    ("CropAngle", "crop.angle"),
    ("CropPositionX", "crop.x"),
    ("CropPositionY", "crop.y"),
    ("CropScale", "crop.scale"),
];

/// The pseudo controls `crop.nudge` moves (no absolute value).
pub const CROP_PSEUDO: &[&str] = &["crop.x", "crop.y", "crop.scale"];

/// A Lightroom parameter name → LightCraft control id.
pub fn lr_param(name: &str) -> Option<&'static str> {
    let name = name.trim().trim_end_matches('=').trim();
    LR_PARAMS.iter().find(|(lr, _)| lr.eq_ignore_ascii_case(name)).map(|(_, c)| *c)
}

// ------------------------------------------------------------------------------------ the file

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeymapFile {
    /// `lightroom` (built-in keys) or `classic`; empty = `lightroom`.
    pub profile: String,
    /// The user's keys: added to, or replacing, the profile's (an empty `command` unbinds).
    pub bindings: Vec<Binding>,
    /// MIDI in (Settings ▸ ctrl.).
    pub midi_enabled: bool,
    pub midi: Vec<MidiBinding>,
    /// Nudge steps per develop control, `[small, large]` (`keys.nudge`, relative MIDI dials);
    /// missing controls use the slider's step ×1 / ×5.
    pub steps: BTreeMap<String, [f64; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binding {
    /// `Alt+Q`, `Cmd+Shift+S`, `MouseBack`, `MouseForward`, `MouseMiddle`.
    pub keys: String,
    /// Command id; empty = the keys do nothing.
    pub command: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl Binding {
    pub fn new(keys: &str, command: &str, params: Value, label: &str) -> Self {
        Binding { keys: keys.into(), command: command.into(), params, label: label.into() }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiBinding {
    /// `cc` or `note`.
    pub midi: String,
    /// CC or note number.
    pub number: u8,
    /// 1–16; 0 = any channel.
    pub channel: u8,
    /// A develop control (CC): moved by the CC.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub control: String,
    /// `abs` (0–127 = the slider's range) or `rel` (endless dial: ticks × `step`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// Per tick (`rel`); 0 = the control's small step.
    #[serde(skip_serializing_if = "is_zero")]
    pub step: f64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
    /// A command (notes; CCs without a control run it when the value goes above 0).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub params: Value,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Parse `keymap.json`. Unknown fields are ignored; a bad file is an error (the caller keeps
/// the defaults).
pub fn parse_file(text: &str) -> Result<KeymapFile, String> {
    if text.trim().is_empty() {
        return Ok(KeymapFile::default());
    }
    serde_json::from_str::<KeymapFile>(text).map_err(|e| format!("keymap.json: {e}"))
}

// ------------------------------------------------------------------------------- key combos

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(Key),
    Mouse(PointerButton),
}

/// A key or mouse button with modifiers (`Cmd` = the platform command key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub trigger: Trigger,
}

impl Combo {
    pub fn modifiers(&self) -> Modifiers {
        Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: false, command: self.cmd }
    }

    pub fn from_modifiers(m: Modifiers, trigger: Trigger) -> Self {
        // on macOS egui reports ⌘ as `command` (+ `mac_cmd`); elsewhere Ctrl is `command` too
        let ctrl = m.ctrl && !(m.command && !cfg!(target_os = "macos"));
        Combo { cmd: m.command || m.mac_cmd, ctrl, alt: m.alt, shift: m.shift, trigger }
    }
}

/// `Alt+Q` / `MouseBack` → combo; `None` for unknown keys.
pub fn parse_combo(s: &str) -> Option<Combo> {
    let s = s.trim();
    let (mods, last) = match s.rsplit_once('+') {
        // `Cmd++` / `+` alone: the plus key
        Some((m, "")) => (m.strip_suffix('+').unwrap_or(m), "+"),
        Some((m, k)) => (m, k),
        None => ("", s),
    };
    let mouse = match last.to_ascii_lowercase().as_str() {
        "mouseback" | "mouse4" | "back" => Some(PointerButton::Extra1),
        "mouseforward" | "mouse5" | "forward" => Some(PointerButton::Extra2),
        "mousemiddle" | "mouse3" => Some(PointerButton::Middle),
        _ => None,
    };
    let mut c = Combo { cmd: false, ctrl: false, alt: false, shift: false, trigger: Trigger::Key(Key::A) };
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "meta" | "super" => c.cmd = true,
            "ctrl" | "control" => c.ctrl = true,
            "alt" | "opt" | "option" => c.alt = true,
            "shift" => c.shift = true,
            _ => return None,
        }
    }
    if let Some(b) = mouse {
        c.trigger = Trigger::Mouse(b);
        return Some(c);
    }
    let key = crate::shortcuts::parse(&single_key_name(last))?.1;
    c.trigger = Trigger::Key(key);
    Some(c)
}

/// Normalise a key name from other tools (`q`, `\\`, `D4`, `Backslash`, `right`, `+`).
fn single_key_name(k: &str) -> String {
    let k = k.trim();
    let lower = k.to_ascii_lowercase();
    match lower.as_str() {
        "backslash" => "\\".into(),
        "slash" => "/".into(),
        "right" | "left" | "up" | "down" => {
            let mut c = lower.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        }
        "return" | "enter" => "Enter".into(),
        "esc" | "escape" => "Escape".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "delete" | "backspace" => "Delete".into(),
        "+" | "plus" => "Plus".into(),
        _ => {
            // .NET key names: D0–D9 are the digit keys
            if lower.len() == 2 && lower.starts_with('d') && lower.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
                return lower.get(1..).unwrap_or_default().to_string();
            }
            if k.chars().count() == 1 { k.to_ascii_uppercase() } else { k.to_string() }
        }
    }
}

/// The canonical text of a combo (`Cmd+Alt+Q`), the form `keymap.json` and menus use.
pub fn format_combo(c: &Combo) -> String {
    let mut s = String::new();
    for (on, name) in [(c.ctrl, "Ctrl+"), (c.cmd, "Cmd+"), (c.alt, "Alt+"), (c.shift, "Shift+")] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(&match c.trigger {
        Trigger::Mouse(PointerButton::Extra1) => "MouseBack".to_string(),
        Trigger::Mouse(PointerButton::Extra2) => "MouseForward".to_string(),
        Trigger::Mouse(PointerButton::Middle) => "MouseMiddle".to_string(),
        Trigger::Mouse(b) => format!("{b:?}"),
        Trigger::Key(Key::Backspace) => "Delete".to_string(),
        Trigger::Key(Key::Backslash) => "\\".to_string(),
        Trigger::Key(Key::Slash) => "/".to_string(),
        Trigger::Key(Key::OpenBracket) => "[".to_string(),
        Trigger::Key(Key::CloseBracket) => "]".to_string(),
        Trigger::Key(Key::Equals) => "=".to_string(),
        Trigger::Key(Key::Minus) => "-".to_string(),
        Trigger::Key(Key::Quote) => "'".to_string(),
        Trigger::Key(Key::Comma) => ",".to_string(),
        Trigger::Key(k) => k.name().to_string(),
    });
    s
}

/// Normalise a keys string (`opt+q` → `Alt+Q`); unknown keys stay as written.
pub fn canonical(keys: &str) -> String {
    parse_combo(keys).map(|c| format_combo(&c)).unwrap_or_else(|| keys.trim().to_string())
}

/// Human text: `⌘⌥Q` on macOS.
pub fn display(keys: &str) -> String {
    match keys {
        "MouseBack" => "mouse back".into(),
        "MouseForward" => "mouse forward".into(),
        "MouseMiddle" => "mouse middle".into(),
        k => crate::menubar::shortcut_text(k, cfg!(target_os = "macos")),
    }
}

// --------------------------------------------------------------------------------- resolving

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Builtin,
    Profile,
    User,
}

/// One row of the effective keymap (Settings ▸ shrt., `keys.list`).
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub keys: String,
    pub command: String,
    pub params: Value,
    pub label: String,
    pub group: String,
    pub origin: Origin,
    /// A higher layer rebinds these keys (this row doesn't fire).
    pub overridden: bool,
    /// Another binding of the same layer shares the keys.
    pub conflict: bool,
}

/// The built-in shortcuts: `(keys, command, params, label)`.
pub fn builtin() -> Vec<(String, String, Value, String)> {
    let mut v = Vec::new();
    let mut ui_keys = Vec::new();
    for (id, label, sc, _) in crate::menus::ui_commands() {
        if let Some(sc) = sc {
            ui_keys.push(canonical(sc));
            v.push((canonical(sc), id.to_string(), json!({}), label.to_string()));
        }
    }
    for c in lightcraft_engine::command_specs() {
        // a UI command bound to the same key wraps the engine one
        if let Some(sc) = c.shortcut.map(canonical).filter(|k| !ui_keys.contains(k)) {
            v.push((sc, c.id.to_string(), json!({}), c.label.to_string()));
        }
    }
    for (sc, id, params) in crate::shortcuts::ALIASES {
        v.push((canonical(sc), id.to_string(), serde_json::from_str(params).unwrap_or(json!({})), command_label(id)));
    }
    for n in 0..=5u8 {
        v.push((n.to_string(), "photo.rate".into(), json!({"rating": n}), format!("rating {n}")));
        v.push((format!("Shift+{n}"), "photo.rate".into(), json!({"rating": n, "advance": true}), format!("rating {n} + next")));
    }
    for (n, l) in [(6, "red"), (7, "yellow"), (8, "green"), (9, "blue")] {
        v.push((n.to_string(), "photo.label".into(), json!({"label": l}), format!("label {l}")));
    }
    v
}

/// A command's label from the registries (the id when unknown).
pub fn command_label(id: &str) -> String {
    if let Some(c) = crate::menus::ui_commands().find(|c| c.0 == id) {
        return c.1.to_string();
    }
    lightcraft_engine::find_command(id).map(|c| c.label.to_string()).unwrap_or_else(|| id.to_string())
}

/// The profile's bindings.
pub fn profile_bindings(profile: &str) -> Vec<Binding> {
    match profile {
        "classic" => CLASSIC.iter().map(|(k, c, p)| Binding::new(k, c, serde_json::from_str(p).unwrap_or(json!({})), "")).collect(),
        _ => vec![],
    }
}

/// The layers above the built-ins, lowest first: the profile's bindings, then the user's (a user
/// binding replaces the profile's for the same keys).
pub fn overlay(file: &KeymapFile) -> Vec<(Combo, Binding, Origin)> {
    let mut v: Vec<(Combo, Binding, Origin)> = Vec::new();
    for b in profile_bindings(&file.profile) {
        if let Some(c) = parse_combo(&b.keys) {
            v.push((c, b, Origin::Profile));
        }
    }
    for b in &file.bindings {
        let Some(c) = parse_combo(&b.keys) else { continue };
        v.retain(|(c2, _, o)| !(*o == Origin::Profile && *c2 == c));
        v.push((c, b.clone(), Origin::User));
    }
    v
}

/// The binding that fires for `combo` (the last one in the overlay wins), if the overlay has one.
/// `Some(None)` = the keys are unbound.
pub fn lookup<'a>(overlay: &'a [(Combo, Binding, Origin)], combo: &Combo) -> Option<Option<&'a Binding>> {
    overlay.iter().rev().find(|(c, _, _)| c == combo).map(|(_, b, _)| (!b.command.is_empty()).then_some(b))
}

/// What `keys` do with the built-ins and the overlay: `(command, params)`.
pub fn resolve(overlay: &[(Combo, Binding, Origin)], keys: &str) -> Option<(String, Value)> {
    let combo = parse_combo(keys)?;
    if let Some(hit) = lookup(overlay, &combo) {
        return hit.map(|b| (b.command.clone(), b.params.clone()));
    }
    builtin().into_iter().find(|(k, ..)| parse_combo(k) == Some(combo)).map(|(_, c, p, _)| (c, p))
}

fn group_of(command: &str) -> String {
    match command.split('.').next().unwrap_or("") {
        "view" | "panel" | "section" => "view",
        "photo" | "library" | "album" | "stack" | "keyword" | "label" => "library",
        "develop" | "crop" | "tool" | "brush" | "preset" | "version" | "geometry" | "mask" | "spot" => "develop",
        "keys" => "super keys",
        "edit" => "edit",
        "" => "unbound",
        _ => "other",
    }
    .to_string()
}

/// Every binding, grouped by area, with what overrides what and conflicts.
pub fn rows(file: &KeymapFile) -> Vec<Row> {
    let ov = overlay(file);
    let mut out = Vec::new();
    for (k, c, p, label) in builtin() {
        let combo = parse_combo(&k);
        let overridden = combo.is_some_and(|c| ov.iter().any(|(c2, _, _)| *c2 == c));
        out.push(Row { keys: k, group: group_of(&c), command: c, params: p, label, origin: Origin::Builtin, overridden, conflict: false });
    }
    for (i, (c, b, o)) in ov.iter().enumerate() {
        let conflict =
            ov.iter().enumerate().any(|(j, (c2, b2, o2))| j != i && c2 == c && o2 == o && (b2.command != b.command || b2.params != b.params));
        let overridden = ov.iter().skip(i + 1).any(|(c2, _, _)| c2 == c);
        let label = if !b.label.is_empty() {
            b.label.clone()
        } else if b.command.is_empty() {
            "(unbound)".into()
        } else {
            command_label(&b.command)
        };
        out.push(Row {
            keys: format_combo(c),
            command: b.command.clone(),
            params: b.params.clone(),
            label,
            group: group_of(&b.command),
            origin: *o,
            overridden,
            conflict,
        });
    }
    out.sort_by(|a, b| a.group.cmp(&b.group).then(a.keys.cmp(&b.keys)));
    out
}

/// The effective menu shortcut of a command (with `{}` params): the overlay's key for it, else its
/// built-in key unless the overlay took that key for something else.
pub fn menu_shortcut(overlay: &[(Combo, Binding, Origin)], id: &str, builtin: Option<&str>) -> Option<String> {
    let plain = |p: &Value| p.is_null() || p.as_object().is_some_and(|o| o.is_empty());
    if let Some((c, _, _)) =
        overlay.iter().rev().find(|(c, b, _)| b.command == id && plain(&b.params) && lookup(overlay, c).flatten().is_some_and(|w| w.command == id))
    {
        // mouse buttons aren't menu accelerators
        if matches!(c.trigger, Trigger::Key(_)) {
            return Some(format_combo(c));
        }
    }
    let b = builtin?;
    let combo = parse_combo(b)?;
    match lookup(overlay, &combo) {
        Some(Some(w)) if w.command == id => Some(b.to_string()),
        Some(_) => None,
        None => Some(b.to_string()),
    }
}

// ---------------------------------------------------------------------------- live state

/// The loaded keymap, its file and what was wrong with it.
#[derive(Default)]
pub struct Keymap {
    pub file: KeymapFile,
    /// Where it's read from and saved to (`None` = in memory only: tests, the web).
    pub path: Option<PathBuf>,
    /// The file couldn't be read: its error (the defaults are in use, the file is left alone).
    pub error: Option<String>,
    /// Modification stamp of the file as last read (live reload).
    stamp: Option<std::time::SystemTime>,
    checked: f64,
    overlay: Vec<(Combo, Binding, Origin)>,
    /// MIDI learn: waiting for a control to move (`Some(None)`), or the control that moved.
    pub learn: Option<Option<MidiMsg>>,
    /// The last MIDI message received (ctrl. page).
    pub last_midi: Option<MidiMsg>,
    /// shrt. page: recording keys for the draft binding (`Some(None)`) or user binding `i`.
    pub recording: Option<Option<usize>>,
    /// shrt. page: the action the next recorded keys are bound to.
    pub draft: Binding,
}

impl Keymap {
    /// The keymap file in the settings folder (`LIGHTCRAFT_KEYMAP` overrides it).
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("LIGHTCRAFT_KEYMAP")
            .map(PathBuf::from)
            .or_else(|| lightcraft_engine::camera_profiles::config_dir().map(|d| d.join("keymap.json")))
    }

    /// A keymap backed by `path` (read now; a missing file = defaults).
    pub fn at(path: Option<PathBuf>) -> Self {
        let mut k = Keymap { path, ..Default::default() };
        k.reload();
        k
    }

    /// In memory, from a file's contents (tests, `keys.set`).
    pub fn from_file(file: KeymapFile) -> Self {
        let mut k = Keymap { file, ..Default::default() };
        k.overlay = overlay(&k.file);
        k
    }

    pub fn overlay(&self) -> &[(Combo, Binding, Origin)] {
        &self.overlay
    }

    /// Re-read the file. A damaged file keeps the previous bindings and records the error.
    pub fn reload(&mut self) {
        let Some(p) = self.path.clone() else {
            self.overlay = overlay(&self.file);
            return;
        };
        self.stamp = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        match std::fs::read_to_string(&p) {
            Ok(text) => match parse_file(&text) {
                Ok(f) => {
                    self.file = f;
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.error = None;
            }
            Err(e) => self.error = Some(format!("keymap.json: {e}")),
        }
        self.overlay = overlay(&self.file);
    }

    /// Re-read the file when it changed on disk (checked about once a second).
    pub fn tick(&mut self, now: f64) -> bool {
        if now - self.checked < 1.0 {
            return false;
        }
        self.checked = now;
        let Some(p) = &self.path else { return false };
        let stamp = std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if stamp != self.stamp {
            self.reload();
            return true;
        }
        false
    }

    /// Apply a change made in the UI: re-resolve and write the file (unless it was unreadable).
    pub fn set(&mut self, file: KeymapFile) -> Result<(), String> {
        self.file = file;
        self.overlay = overlay(&self.file);
        self.save()
    }

    pub fn save(&mut self) -> Result<(), String> {
        let Some(p) = self.path.clone() else { return Ok(()) };
        if let Some(e) = &self.error {
            return Err(format!("{e} — fix or delete the file first; it was left as it is"));
        }
        let bytes = serde_json::to_vec_pretty(&self.file).map_err(|e| e.to_string())?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("saving the keymap failed: {e}"))?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &p)).map_err(|e| format!("saving the keymap failed: {e}"))?;
        self.stamp = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
        Ok(())
    }

    /// Nudge step for a control: `[small, large]`.
    pub fn steps(&self, control: &str) -> [f64; 2] {
        if let Some(s) = self.file.steps.get(control) {
            return *s;
        }
        default_steps(control)
    }
}

/// A control's default nudge steps (the slider's step ×1 / ×5; Exposure ⅓ / 1 stop like Lightroom's
/// +/- keys; crop pseudo controls in percent).
pub fn default_steps(control: &str) -> [f64; 2] {
    match control {
        "light.exposure" => [0.05, 0.33],
        "wb.temp" => [25.0, 125.0],
        "crop.angle" => [0.15, 0.5],
        "crop.x" | "crop.y" | "crop.scale" => [0.5, 2.5],
        c => lightcraft_develop::controls::find(c).map(|s| [s.step, s.step * 5.0]).unwrap_or([1.0, 5.0]),
    }
}

// --------------------------------------------------------------------------------------- MIDI

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidiKind {
    Cc,
    NoteOn,
    NoteOff,
}

/// One MIDI channel message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiMsg {
    pub kind: MidiKind,
    /// 1–16.
    pub channel: u8,
    pub number: u8,
    pub value: u8,
}

/// Parse a raw MIDI message (CC, note on/off); anything else → `None`.
pub fn parse_midi(bytes: &[u8]) -> Option<MidiMsg> {
    let (&status, rest) = bytes.split_first()?;
    let channel = (status & 0x0F) + 1;
    let number = *rest.first()? & 0x7F;
    let value = *rest.get(1)? & 0x7F;
    let kind = match status & 0xF0 {
        0xB0 => MidiKind::Cc,
        0x90 if value > 0 => MidiKind::NoteOn,
        0x90 | 0x80 => MidiKind::NoteOff,
        _ => return None,
    };
    Some(MidiMsg { kind, channel, number, value })
}

/// Endless-dial ticks from a relative CC value: two's complement (1 = +1, 127 = −1) or 64-offset
/// (65 = +1, 63 = −1), told apart by where the value sits (a single message never turns 16+
/// ticks in practice).
pub fn relative_ticks(v: u8) -> i32 {
    let v = i32::from(v & 0x7F);
    match v {
        48..=80 => v - 64,
        0..=47 => v,
        _ => v - 128,
    }
}

/// What a MIDI message does.
#[derive(Clone, Debug, PartialEq)]
pub enum MidiAction {
    /// `develop.set` the control to an absolute value.
    Set { control: String, value: f64 },
    /// Move the control by a delta (`develop.adjust`, or `crop.nudge` for the crop pseudo controls).
    Nudge { control: String, delta: f64 },
    /// Run a command.
    Run { command: String, params: Value },
}

fn midi_matches(b: &MidiBinding, m: &MidiMsg) -> bool {
    let kind_ok = match m.kind {
        MidiKind::Cc => b.midi == "cc",
        MidiKind::NoteOn => b.midi == "note",
        MidiKind::NoteOff => false,
    };
    kind_ok && b.number == m.number && (b.channel == 0 || b.channel == m.channel)
}

/// The action of the first mapping that matches `m` (with the keymap's steps for relative dials).
pub fn midi_action(km: &Keymap, m: &MidiMsg) -> Option<MidiAction> {
    let b = km.file.midi.iter().find(|b| midi_matches(b, m))?;
    if !b.control.is_empty() && m.kind == MidiKind::Cc {
        let sign = if b.invert { -1.0 } else { 1.0 };
        if b.mode == "rel" || CROP_PSEUDO.contains(&b.control.as_str()) {
            let step = if b.step > 0.0 { b.step } else { km.steps(&b.control)[0] };
            let ticks = if b.mode == "rel" { f64::from(relative_ticks(m.value)) } else { f64::from(m.value) - 64.0 };
            if ticks == 0.0 {
                return None;
            }
            return Some(MidiAction::Nudge { control: b.control.clone(), delta: ticks * step * sign });
        }
        let spec = lightcraft_develop::controls::find(&b.control)?;
        let t = f64::from(m.value.min(127)) / 127.0;
        let t = if b.invert { 1.0 - t } else { t };
        return Some(MidiAction::Set { control: b.control.clone(), value: spec.min + (spec.max - spec.min) * t });
    }
    if b.command.is_empty() || (m.kind == MidiKind::Cc && m.value == 0) {
        return None;
    }
    Some(MidiAction::Run { command: b.command.clone(), params: if b.params.is_null() { json!({}) } else { b.params.clone() } })
}

/// Merge the actions of one frame's messages: absolute moves keep the last value, nudges add up.
pub fn coalesce(actions: Vec<MidiAction>) -> Vec<MidiAction> {
    let mut out: Vec<MidiAction> = Vec::new();
    for a in actions {
        match &a {
            MidiAction::Set { control, value } => {
                if let Some(MidiAction::Set { value: v, .. }) =
                    out.iter_mut().find(|x| matches!(x, MidiAction::Set { control: c, .. } if c == control))
                {
                    *v = *value;
                    continue;
                }
            }
            MidiAction::Nudge { control, delta } => {
                if let Some(MidiAction::Nudge { delta: d, .. }) =
                    out.iter_mut().find(|x| matches!(x, MidiAction::Nudge { control: c, .. } if c == control))
                {
                    *d += *delta;
                    continue;
                }
            }
            MidiAction::Run { .. } => {}
        }
        out.push(a);
    }
    out
}

/// A mapping from a learned message (ctrl. page): a CC moves `control` (`abs` for a slider,
/// `rel` for a dial), a note runs `command`.
pub fn learned_binding(m: &MidiMsg, control: &str, relative: bool, command: &str, params: Value) -> MidiBinding {
    let cc = m.kind == MidiKind::Cc;
    MidiBinding {
        midi: if cc { "cc" } else { "note" }.into(),
        number: m.number,
        channel: m.channel,
        control: if cc { control.to_string() } else { String::new() },
        mode: if cc && !control.is_empty() { if relative { "rel" } else { "abs" }.into() } else { String::new() },
        command: if cc && !control.is_empty() { String::new() } else { command.to_string() },
        params: if cc && !control.is_empty() { Value::Null } else { params },
        ..Default::default()
    }
}

// ----------------------------------------------------------------------------------- imports

/// What an import produced: bindings, MIDI mappings, steps, and a line per thing that mapped or
/// didn't (and why).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Import {
    pub bindings: Vec<Binding>,
    pub midi: Vec<MidiBinding>,
    pub steps: BTreeMap<String, [f64; 2]>,
    pub mapped: Vec<String>,
    pub skipped: Vec<String>,
}

impl Import {
    /// Add the result to a keymap file: same keys / same MIDI number replace what was there.
    pub fn merge_into(&self, f: &mut KeymapFile) {
        for b in &self.bindings {
            let c = parse_combo(&b.keys);
            f.bindings.retain(|x| parse_combo(&x.keys) != c);
            f.bindings.push(b.clone());
        }
        for m in &self.midi {
            f.midi.retain(|x| !(x.midi == m.midi && x.number == m.number && x.channel == m.channel));
            f.midi.push(m.clone());
        }
        for (k, v) in &self.steps {
            f.steps.insert(k.clone(), *v);
        }
    }
}

/// An LrKeys / LrSuperKeys action URL (`crop?w=8&h=10`, `param?name=Exposure&delta=0.33`…) →
/// command + params.
pub fn lrkeys_action(cmd: &str) -> Option<(String, Value)> {
    let (verb, query) = cmd.trim().split_once('?').unwrap_or((cmd.trim(), ""));
    let q: BTreeMap<String, String> = query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.replace("%20", " ").replace('+', " ")))
        .collect();
    let g = |k: &str| q.get(k).map(String::as_str);
    let num = |k: &str| g(k).and_then(|v| v.parse::<f64>().ok());
    Some(match verb {
        "crop" => match (g("a"), num("w"), num("h")) {
            (Some("asshot" | "original"), ..) => ("crop.aspect".into(), json!({"aspect": "original"})),
            (_, Some(w), Some(h)) if w > 0.0 && h > 0.0 => ("crop.aspect".into(), json!({"aspect": format!("{w}x{h}")})),
            _ => return None,
        },
        "cropreset" => ("crop.reset".into(), json!({})),
        "straighten" => ("crop.autoStraighten".into(), json!({})),
        "param" => {
            let control = lr_param(g("name")?)?;
            if let Some(d) = num("delta") {
                ("develop.adjust".into(), json!({"control": control, "delta": d}))
            } else if let Some(v) = num("value") {
                ("develop.set".into(), json!({"control": control, "value": v}))
            } else if g("reset").is_some() {
                ("develop.resetControl".into(), json!({"control": control}))
            } else {
                return None;
            }
        }
        "preset" => ("preset.applyByName".into(), json!({"name": g("name")?})),
        "flag" => {
            let f = match g("v")? {
                "1" => "pick",
                "-1" => "reject",
                _ => "none",
            };
            ("photo.flag".into(), json!({"flag": f}))
        }
        "rating" => ("photo.rate".into(), json!({"rating": num("v")?.clamp(0.0, 5.0) as u8})),
        "label" => ("photo.label".into(), json!({"label": g("v")?})),
        "undo" => ("edit.undo".into(), json!({})),
        "redo" => ("edit.redo".into(), json!({})),
        "next" => ("library.next".into(), json!({})),
        "prev" | "previous" => ("library.previous".into(), json!({})),
        _ => return None,
    })
}

fn mods_keys(mods: &[String], key: &str) -> String {
    let mut s = String::new();
    for m in mods {
        let m = match m.to_ascii_lowercase().as_str() {
            "cmd" | "command" => "Cmd+",
            "alt" | "opt" | "option" => "Alt+",
            "shift" => "Shift+",
            "ctrl" | "control" => "Ctrl+",
            _ => continue,
        };
        if !s.contains(m) {
            s.push_str(m);
        }
    }
    canonical(&format!("{s}{}", single_key_name(key)))
}

/// A keystroke another tool sends → what that key does in LightCraft (Classic profile), or
/// `keys.send` when nothing is bound to it.
fn keystroke_command(keys: &str) -> (String, Value) {
    let ov = overlay(&KeymapFile { profile: "classic".into(), ..Default::default() });
    resolve(&ov, keys).filter(|(c, _)| !c.is_empty()).unwrap_or_else(|| ("keys.send".into(), json!({"keys": keys})))
}

/// Bryan's LrKeys `bindings.json`: `{options, actions: [{id, group, label, kind: keys|url, send|cmd,
/// mods, key}]}`. Actions without a key are reported (they're on the shrt. page's super-key list).
pub fn import_lrkeys(text: &str) -> Result<Import, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bindings.json: {e}"))?;
    let actions = v.get("actions").and_then(Value::as_array).ok_or("bindings.json: no `actions` list")?;
    let mut out = Import::default();
    for a in actions {
        let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("");
        let label = if s("label").is_empty() { s("id") } else { s("label") };
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        let strs = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
        };
        let action = match s("kind") {
            "keys" => {
                let send = a.get("send");
                let key = send.and_then(|s| s.get("key")).and_then(Value::as_str).unwrap_or("");
                if key.is_empty() { None } else { Some(keystroke_command(&mods_keys(&strs(send.and_then(|s| s.get("mods"))), key))) }
            }
            _ => lrkeys_action(s("cmd")),
        };
        let Some((command, params)) = action else {
            out.skipped.push(format!("{label}: `{}` has no LightCraft command", s("cmd")));
            continue;
        };
        match a.get("key").and_then(Value::as_str).filter(|k| !k.is_empty()) {
            Some(key) => {
                let keys = mods_keys(&strs(a.get("mods")), key);
                if parse_combo(&keys).is_none() {
                    out.skipped.push(format!("{label}: key `{key}` isn't a key LightCraft knows"));
                    continue;
                }
                out.mapped.push(format!("{} → {label} ({command})", display(&keys)));
                out.bindings.push(Binding::new(&keys, &command, params, &label));
            }
            None => out.skipped.push(format!("{label}: no key in LrKeys (available as a super-key action: {command})")),
        }
    }
    Ok(out)
}

/// The text of `<tag>…</tag>` inside `block` (XML entities decoded), if present.
fn xml_field(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block.get(start..)?.find(&format!("</{tag}>"))? + start;
    Some(block.get(start..end)?.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'"))
}

fn xml_blocks<'a>(text: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        let Some(after) = rest.get(i + open.len()..) else { break };
        let Some(j) = after.find(&close) else { break };
        v.push(after.get(..j).unwrap_or(""));
        rest = after.get(j + close.len()..).unwrap_or("");
    }
    v
}

/// A LrSuperKeys Mac keystroke (`^%v` = ⌘⌥V: `^` ⌘, `%` ⌥, `+` ⇧, `~` ⌃).
fn superkeys_keystroke(s: &str) -> Option<String> {
    let mut mods = Vec::new();
    let mut key = String::new();
    for ch in s.chars() {
        match ch {
            '^' if key.is_empty() => mods.push("cmd".to_string()),
            '%' if key.is_empty() => mods.push("alt".to_string()),
            '+' if key.is_empty() => mods.push("shift".to_string()),
            '~' if key.is_empty() => mods.push("ctrl".to_string()),
            c => key.push(c),
        }
    }
    let key = key.trim_matches(|c| c == '{' || c == '}');
    (!key.is_empty()).then(|| mods_keys(&mods, key))
}

/// A LrSuperKeys function (`CropRatio = ratio={w=8,h=10}`, `CropRatio = ratio="asshot"`,
/// `AUTOCROP`) → command.
fn superkeys_function(f: &str) -> Option<(String, Value)> {
    let f = f.trim();
    if f.eq_ignore_ascii_case("AUTOCROP") {
        return Some(("crop.autoStraighten".into(), json!({})));
    }
    if f.eq_ignore_ascii_case("CROPRESET") || f.eq_ignore_ascii_case("ResetCrop") {
        return Some(("crop.reset".into(), json!({})));
    }
    let (name, arg) = f.split_once('=')?;
    if name.trim() == "CropRatio" {
        let arg = arg.trim();
        if arg.contains("asshot") {
            return lrkeys_action("crop?a=asshot");
        }
        let num = |k: &str| -> Option<String> {
            let i = arg.find(&format!("{k}="))? + k.len() + 1;
            Some(arg.get(i..)?.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect())
        };
        return lrkeys_action(&format!("crop?w={}&h={}", num("w")?, num("h")?));
    }
    if name.trim() == "set_preset" {
        return Some(("preset.applyByName".into(), json!({"name": arg.trim()})));
    }
    None
}

/// LrSuperKeys `Shortcuts.xml` (+ `SpeedKeys.xml`, optional): shortcuts → bindings (they are
/// ⌥+key: every one Bryan converted to LrKeys is, whatever the file's `ModifierKey` code says);
/// speed-key deltas → nudge steps; the HSL super keys → Color Mixer modes.
pub fn import_superkeys(shortcuts: &str, speedkeys: Option<&str>) -> Import {
    let mut out = Import::default();
    for b in xml_blocks(shortcuts, "ShortCuts") {
        let desc = xml_field(b, "Description").unwrap_or_default();
        let Some(sk) = xml_field(b, "superkey").filter(|s| !s.trim().is_empty()) else {
            out.skipped.push(format!("{desc}: no key"));
            continue;
        };
        let keys = mods_keys(&["alt".into()], &sk);
        let action = if let Some(k) = xml_field(b, "MacCommand").and_then(|c| superkeys_keystroke(&c)) {
            Some(keystroke_command(&k))
        } else {
            xml_field(b, "FunctionCommandString").and_then(|f| superkeys_function(&f))
        };
        match action {
            Some((command, params)) if parse_combo(&keys).is_some() => {
                out.mapped.push(format!("{} → {desc} ({command})", display(&keys)));
                out.bindings.push(Binding::new(&keys, &command, params, &desc));
            }
            _ => out.skipped.push(format!("{desc}: no LightCraft equivalent")),
        }
    }
    if let Some(sp) = speedkeys {
        for b in xml_blocks(sp, "SpeedKeys") {
            let param = xml_field(b, "lrParam").unwrap_or_default();
            let desc = xml_field(b, "Description").unwrap_or_default();
            let small = xml_field(b, "DeltaChange").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
            let large = xml_field(b, "DeltaChangeLarge").and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
            let superkey = xml_field(b, "SuperKey").unwrap_or_default();
            let mode = match param.as_str() {
                "HSLKeyH" => Some("hue"),
                "HSLKeyS" => Some("saturation"),
                "HSLKeyL" => Some("luminance"),
                "HSLKeyBW" => Some("bw"),
                _ => None,
            };
            if let Some(mode) = mode {
                if !superkey.trim().is_empty() {
                    let keys = canonical(&single_key_name(&superkey));
                    out.mapped.push(format!("{} → {desc} panel (view.colorMixer {mode})", display(&keys)));
                    out.bindings.push(Binding::new(&keys, "view.colorMixer", json!({"mode": mode}), &format!("{desc} panel")));
                }
                continue;
            }
            if param.starts_with("--") || small <= 0.0 {
                continue;
            }
            match lr_param(&param) {
                Some(control) => {
                    out.steps.insert(control.to_string(), [small, if large > 0.0 { large } else { small * 5.0 }]);
                    for (tag, dir) in [("IncreaseKey", 1), ("DecreaseKey", -1)] {
                        if let Some(k) = xml_field(b, tag).filter(|k| !k.trim().is_empty()) {
                            let keys = canonical(&single_key_name(&k));
                            out.bindings.push(Binding::new(&keys, "keys.nudge", json!({"control": control, "dir": dir}), &format!("{desc} {dir:+}")));
                        }
                    }
                }
                None => out.skipped.push(format!("speed key {desc} ({param}): no LightCraft slider")),
            }
        }
        out.mapped.push(format!("{} slider steps from SpeedKeys.xml", out.steps.len()));
    }
    out
}

// --------------------------------------------------------------------------------- Monogram

/// MIDI numbers handed out to a Monogram profile's modules (channel 1): sliders CC 20+, dial
/// turns CC 30+, dial press-and-turns CC 50+, notes 60+.
struct Alloc {
    slider_cc: u8,
    turn_cc: u8,
    press_turn_cc: u8,
    note: u8,
}

/// A Monogram `press` / `pressAndHold` / macro step → command.
fn monogram_step(v: &Value) -> Option<(String, Value)> {
    if let Some(m) = v.get("macro").and_then(Value::as_array) {
        let steps: Vec<Value> = m.iter().filter_map(monogram_step).map(|(c, p)| json!({"command": c, "params": p})).collect();
        return match steps.len() {
            0 => None,
            1 => steps.first().and_then(|s| Some((s["command"].as_str()?.to_string(), s["params"].clone()))),
            _ => Some(("keys.macro".into(), json!({"steps": steps}))),
        };
    }
    if let Some(input) = v.get("input").and_then(Value::as_str) {
        let (name, arg) = input.split_once('=').unwrap_or((input, ""));
        return match name.trim() {
            "set_preset" => Some(("preset.applyByName".into(), json!({"name": arg.trim()}))),
            "develop_before_after_horiz" => Some(("view.beforeAfter".into(), json!({}))),
            "develop_before_after_vert" => Some(("view.beforeAfterTopBottom".into(), json!({}))),
            _ => None,
        };
    }
    if let Some(key) = v.get("key").and_then(Value::as_str) {
        let mods: Vec<String> =
            v.get("modifiers").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let keys = mods_keys(&mods, key);
        parse_combo(&keys)?;
        return Some(keystroke_command(&keys));
    }
    None
}

fn monogram_control(v: &Value) -> Option<(&'static str, bool, f64)> {
    let input = v.get("input").and_then(Value::as_str)?;
    let control = lr_param(input.split('=').next().unwrap_or(""))?;
    let invert = v.get("invert").and_then(Value::as_bool).unwrap_or(false);
    let step = v.get("step").and_then(Value::as_f64).unwrap_or(0.0);
    Some((control, invert, step))
}

/// A Monogram profile (from its `state.json` `profiles[]`) → LightCraft MIDI mappings, plus the
/// same profile re-assigned to send those MIDI messages (import it into Monogram Creator; set its
/// app to LightCraft). Lightroom-only inputs become the matching LightCraft commands.
pub fn import_monogram(profile: &Value, app_id: &str) -> (Import, Value) {
    let mut out = Import::default();
    let mut assignments = serde_json::Map::new();
    let mut alloc = Alloc { slider_cc: 20, turn_cc: 30, press_turn_cc: 50, note: 60 };
    let title = profile.get("title").and_then(Value::as_str).unwrap_or("Lightroom");
    let mut keys: Vec<(&String, &Value)> = profile.get("assignments").and_then(Value::as_object).map(|o| o.iter().collect()).unwrap_or_default();
    keys.sort_by(|a, b| a.0.cmp(b.0));
    for (module, a) in keys {
        let Some(st) = a.get("settings").and_then(Value::as_object) else { continue };
        if st.is_empty() {
            // the core / unused modules: as they were
            assignments.insert(module.clone(), a.clone());
            continue;
        }
        let name = module.trim();
        let mut ns = serde_json::Map::new();
        // Lightroom's names would mislabel the console's screen: LightCraft's go in below
        let first_mapping = out.midi.len();
        if let Some(v) = st.get("color") {
            ns.insert("color".into(), v.clone());
        }
        let cc = |kind: &str,
                  control: &str,
                  mode: &str,
                  invert: bool,
                  step: f64,
                  number: u8,
                  slot: &str,
                  ns: &mut serde_json::Map<String, Value>,
                  out: &mut Import| {
            ns.insert(slot.into(), json!({"midi": "cc", "cc": number, "channel": 1}));
            out.midi.push(MidiBinding {
                midi: "cc".into(),
                number,
                channel: 1,
                control: control.into(),
                mode: mode.into(),
                step,
                invert,
                label: format!("{name} {kind}"),
                ..Default::default()
            });
            out.mapped.push(format!("{name} {kind} → cc {number} → {control}{}", if mode == "rel" { " (dial)" } else { "" }));
        };
        // slider position
        if let Some(p) = st.get("pose") {
            match monogram_control(p) {
                Some((c, inv, _)) => {
                    cc("slider", c, "abs", inv, 0.0, alloc.slider_cc, "pose", &mut ns, &mut out);
                    alloc.slider_cc = alloc.slider_cc.saturating_add(1);
                }
                None => out.skipped.push(format!("{name} slider: {p}: no LightCraft slider")),
            }
        }
        for (slot, kind, counter) in [("turn", "turn", 0), ("pressAndTurn", "press+turn", 1)] {
            let Some(p) = st.get(slot) else { continue };
            match monogram_control(p) {
                Some((c, inv, step)) => {
                    // Monogram's step counts in its own units; LightCraft uses the slider's nudge step
                    let _ = step;
                    let n = if counter == 0 { &mut alloc.turn_cc } else { &mut alloc.press_turn_cc };
                    cc(kind, c, "rel", inv, 0.0, *n, slot, &mut ns, &mut out);
                    *n = n.saturating_add(1);
                }
                None => out.skipped.push(format!("{name} {kind}: {p}: no LightCraft slider")),
            }
        }
        for (slot, kind) in [("leftTurn", "left turn"), ("rightTurn", "right turn")] {
            if let Some(p) = st.get(slot) {
                out.skipped.push(format!("{name} {kind}: {p} (Lightroom's adjust-the-hovered-slider; no LightCraft equivalent)"));
            }
        }
        let mut note =
            |kind: &str, slot: &str, action: Option<(String, Value)>, ns: &mut serde_json::Map<String, Value>, out: &mut Import, src: &Value| {
                match action {
                    Some((command, params)) => {
                        let n = alloc.note;
                        alloc.note = alloc.note.saturating_add(1);
                        ns.insert(slot.into(), json!({"midi": "note", "note": n, "channel": 1}));
                        out.mapped.push(format!("{name} {kind} → note {n} → {command} {params}"));
                        out.midi.push(MidiBinding {
                            midi: "note".into(),
                            number: n,
                            channel: 1,
                            command,
                            params,
                            label: format!("{name} {kind}"),
                            ..Default::default()
                        });
                    }
                    None => out.skipped.push(format!("{name} {kind}: {src}: no LightCraft equivalent")),
                }
            };
        for (slot, kind) in [("press", "press"), ("pressAndHold", "hold")] {
            if let Some(p) = st.get(slot) {
                note(kind, slot, monogram_step(p), &mut ns, &mut out, p);
            }
        }
        if let Some(d) = st.get("doubleTap") {
            let action = d.as_array().and_then(|a| {
                let control = lr_param(a.first()?.as_str()?.split('=').next()?)?;
                (a.get(1)?.as_str()? == "reset").then(|| ("develop.resetControl".to_string(), json!({"control": control})))
            });
            note("double-tap", "doubleTap", action, &mut ns, &mut out, d);
        }
        if let Some(m) = out.midi.get(first_mapping) {
            let what = if !m.control.is_empty() {
                lightcraft_develop::controls::find(&m.control).map(|c| c.label.to_string()).unwrap_or_else(|| m.control.replace("crop.", "Crop "))
            } else if let Some(n) = m.params.get("name").and_then(Value::as_str) {
                n.to_string()
            } else {
                command_label(&m.command)
            };
            // the console shows ~24 characters
            let short: String = what.chars().take(24).collect();
            ns.insert("label".into(), json!(short));
            ns.insert("name".into(), json!(what));
            ns.insert("info".into(), json!(format!("LightCraft: {}", out.mapped.get(first_mapping..).unwrap_or_default().join("; "))));
        }
        assignments.insert(module.clone(), json!({"settings": Value::Object(ns)}));
    }
    let monogram = json!({
        "version": "3.0",
        "id": "LightCraftLr1MidiMapx",
        "title": format!("LightCraft ({title})"),
        "app": app_id,
        "assignments": Value::Object(assignments),
        "tags": [],
        "notes": format!("Generated by LightCraft from the Monogram profile \"{title}\": every control sends MIDI on channel 1; LightCraft maps it (Settings ▸ ctrl.)."),
        "link": "",
    });
    (out, monogram)
}

// ---------------------------------------------------------------------------------- app side

/// The macOS bundle id (Logi Options+ / Monogram per-app profiles key off it).
pub const BUNDLE_ID: &str = "ai.storyteller.lightcraft";

/// A short on-canvas message from a command (no egui context at hand).
fn say(app: &mut crate::LightcraftApp, text: impl Into<String>) {
    app.ui.toast = Some((text.into(), app.last_time + 1.4, None));
}

/// Show a control's value after a nudge / MIDI move ("Exposure +0.35").
fn hud(app: &mut crate::LightcraftApp, control: &str) {
    let Some(spec) = lightcraft_develop::controls::find(control) else { return };
    let Some(id) = app.session.active() else { return };
    let v = app.session.develop_of(id).and_then(|d| lightcraft_develop::controls::get(&d, control)).unwrap_or(spec.default);
    let text = format!("{} {}", crate::i18n::tr(spec.label), spec.format(v));
    say(app, text);
}

/// Move a develop control (or the crop: `crop.x` / `crop.y` / `crop.scale`) by `delta`.
pub fn nudge(app: &mut crate::LightcraftApp, control: &str, delta: f64) -> Result<Value, String> {
    let r = match control {
        "crop.x" => app.run("crop.nudge", json!({"x": delta})),
        "crop.y" => app.run("crop.nudge", json!({"y": delta})),
        "crop.scale" => app.run("crop.nudge", json!({"scale": delta})),
        _ => app.run("develop.adjust", json!({"control": control, "delta": delta})),
    };
    if r.is_ok() {
        hud(app, control);
    }
    r
}

/// Drain the host's MIDI input and act on it.
pub fn poll_midi(app: &mut crate::LightcraftApp, ctx: &egui::Context) {
    let Some(rx) = &app.midi_rx else { return };
    let msgs: Vec<MidiMsg> = rx.try_iter().take(4096).filter_map(|b| parse_midi(&b)).collect();
    if !msgs.is_empty() {
        handle_midi(app, ctx, msgs);
        ctx.request_repaint();
    }
}

/// Act on MIDI messages (one frame's worth): MIDI learn takes the first one; mappings run.
/// Returns what ran (`keys.midi`).
pub fn handle_midi(app: &mut crate::LightcraftApp, ctx: &egui::Context, msgs: Vec<MidiMsg>) -> Vec<Value> {
    let mut actions = Vec::new();
    for m in msgs {
        app.keymap.last_midi = Some(m);
        if app.keymap.learn.is_some() {
            if app.keymap.learn == Some(None) && m.kind != MidiKind::NoteOff {
                app.keymap.learn = Some(Some(m));
            }
            continue;
        }
        if let Some(a) = midi_action(&app.keymap, &m) {
            actions.push(a);
        }
    }
    let mut done = Vec::new();
    for a in coalesce(actions) {
        match a {
            MidiAction::Set { control, value } => {
                let r = app.run("develop.set", json!({"control": control, "value": value}));
                if r.is_ok() {
                    hud(app, &control);
                }
                done.push(json!({"set": control, "value": value, "ok": r.is_ok()}));
            }
            MidiAction::Nudge { control, delta } => {
                let r = nudge(app, &control, delta);
                done.push(json!({"nudge": control, "delta": delta, "ok": r.is_ok()}));
            }
            MidiAction::Run { command, params } => {
                crate::shortcuts::dispatch(app, ctx, &command, params.clone());
                done.push(json!({"run": command, "params": params}));
            }
        }
    }
    done
}

fn read_text(path: &std::path::Path) -> Result<String, String> {
    // configs are small; refuse anything that isn't
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > 16 << 20 {
        return Err(format!("{}: too large", path.display()));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// Default locations of the configs `keys.import` reads (read-only).
pub fn import_default_path(source: &str) -> PathBuf {
    match source {
        "lrkeys" => home().join("lrkeys/bindings.json"),
        "superkeys" => home().join("Library/Application Support/LrSuperKeys"),
        _ => home().join("Library/Application Support/Monogram/Service/state.json"),
    }
}

/// `keys.import {source: lrkeys|superkeys|monogram, path?, profile?, write?}`: read another tool's
/// config (never written), merge what maps into the keymap, report what did and didn't. Monogram:
/// `write` = where to save the generated Monogram profile (never inside Monogram's own folders).
pub fn import(app: &mut crate::LightcraftApp, p: &Value) -> Result<Value, String> {
    let source = p.get("source").and_then(Value::as_str).unwrap_or("");
    let path = p.get("path").and_then(Value::as_str).map(PathBuf::from).unwrap_or_else(|| import_default_path(source));
    let mut written = Value::Null;
    let imp = match source {
        "lrkeys" => import_lrkeys(&read_text(&path)?)?,
        "superkeys" => {
            let dir = if path.is_dir() { path.clone() } else { path.parent().map(PathBuf::from).unwrap_or_default() };
            let shortcuts = read_text(&dir.join("Shortcuts.xml"))?;
            let speed = read_text(&dir.join("SpeedKeys.xml")).ok();
            import_superkeys(&shortcuts, speed.as_deref())
        }
        "monogram" => {
            let state: Value = serde_json::from_str(&read_text(&path)?).map_err(|e| format!("{}: {e}", path.display()))?;
            let want = p.get("profile").and_then(Value::as_str).unwrap_or("Lightroom 1");
            let profiles = state.get("profiles").and_then(Value::as_array).ok_or("Monogram state: no profiles")?;
            let profile = profiles
                .iter()
                .find(|x| x.get("title").and_then(Value::as_str) == Some(want))
                .or_else(|| {
                    profiles.iter().find(|x| x.get("title").and_then(Value::as_str).is_some_and(|t| t.to_ascii_lowercase().contains("lightroom")))
                })
                .ok_or_else(|| format!("Monogram state: no profile \"{want}\""))?;
            let (imp, mono) = import_monogram(profile, BUNDLE_ID);
            if let Some(out) = p.get("write").and_then(Value::as_str) {
                if out.contains("/Monogram/") {
                    return Err("won't write inside Monogram's own folders: save the profile elsewhere and import it in Monogram Creator".into());
                }
                let bytes = serde_json::to_vec_pretty(&mono).map_err(|e| e.to_string())?;
                if let Some(d) = std::path::Path::new(out).parent() {
                    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
                }
                std::fs::write(out, bytes).map_err(|e| format!("{out}: {e}"))?;
                written = json!(out);
            }
            imp
        }
        other => return Err(format!("keys.import: unknown source `{other}` (lrkeys|superkeys|monogram)")),
    };
    let mut f = app.keymap.file.clone();
    imp.merge_into(&mut f);
    if source == "monogram" {
        f.midi_enabled = true;
    }
    app.keymap.set(f)?;
    Ok(json!({
        "source": source,
        "path": path.display().to_string(),
        "bindings": imp.bindings.len(),
        "midi": imp.midi.len(),
        "steps": imp.steps.len(),
        "mapped": imp.mapped,
        "skipped": imp.skipped,
        "written": written,
    }))
}

/// Commands of the keymap and its super keys; `None` = not one of them.
pub fn run_command(app: &mut crate::LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    Some(match id {
        "keys.nudge" => (|| {
            let control = s("control").ok_or("keys.nudge: missing `control`")?.to_string();
            let delta = match p.get("delta").and_then(Value::as_f64) {
                Some(d) => d,
                None => {
                    let [small, large] = app.keymap.steps(&control);
                    let step = if s("size") == Some("large") { large } else { small };
                    step * p.get("dir").and_then(Value::as_f64).unwrap_or(1.0).signum()
                }
            };
            nudge(app, &control, delta)
        })(),
        "keys.macro" => (|| {
            let steps = p.get("steps").and_then(Value::as_array).ok_or("keys.macro: missing `steps`")?.clone();
            let ctx = egui::Context::default();
            let mut ran = 0;
            for st in steps.iter().take(64) {
                let Some(cmd) = st.get("command").and_then(Value::as_str).filter(|c| !c.is_empty()) else { continue };
                if cmd == "keys.macro" {
                    return Err("keys.macro: a macro can't run a macro".into());
                }
                crate::shortcuts::dispatch(app, &ctx, cmd, st.get("params").cloned().unwrap_or(json!({})));
                ran += 1;
            }
            Ok(json!({"ran": ran}))
        })(),
        "keys.send" => (|| {
            let keys = s("keys").ok_or("keys.send: missing `keys`")?;
            let (cmd, params) = resolve(app.keymap.overlay(), keys).ok_or_else(|| format!("keys.send: nothing is bound to {keys}"))?;
            if cmd.is_empty() || cmd == "keys.send" || cmd == "keys.macro" {
                return Ok(json!({"keys": keys, "command": null}));
            }
            let ctx = egui::Context::default();
            crate::shortcuts::dispatch(app, &ctx, &cmd, params.clone());
            Ok(json!({"keys": canonical(keys), "command": cmd, "params": params}))
        })(),
        "keys.list" => Ok(json!({
            "path": app.keymap.path.as_ref().map(|p| p.display().to_string()),
            "error": app.keymap.error,
            "file": app.keymap.file,
            "rows": rows(&app.keymap.file),
            "recording": app.keymap.recording.is_some(),
            "learn": app.keymap.learn,
            "lastMidi": app.keymap.last_midi,
            "midiStatus": app.midi_status,
        })),
        "keys.set" => (|| {
            // partial file: {profile?, bindings?, midi?, midiEnabled?, steps?} replace those parts
            let mut v = serde_json::to_value(&app.keymap.file).map_err(|e| e.to_string())?;
            if let (Some(o), Some(src)) = (v.as_object_mut(), p.as_object()) {
                for (k, x) in src {
                    o.insert(k.clone(), x.clone());
                }
            }
            let f: KeymapFile = serde_json::from_value(v).map_err(|e| format!("keys.set: {e}"))?;
            if !f.profile.is_empty() && !PROFILES.iter().any(|(id, _)| *id == f.profile) {
                return Err(format!("keys.set: unknown profile `{}`", f.profile));
            }
            app.keymap.set(f)?;
            Ok(serde_json::to_value(&app.keymap.file).unwrap_or_default())
        })(),
        "keys.add" => (|| {
            let keys = canonical(s("keys").ok_or("keys.add: missing `keys`")?);
            parse_combo(&keys).ok_or_else(|| format!("keys.add: unknown keys `{keys}`"))?;
            let b = Binding::new(&keys, s("command").unwrap_or(""), p.get("params").cloned().unwrap_or(Value::Null), s("label").unwrap_or(""));
            let mut f = app.keymap.file.clone();
            let c = parse_combo(&keys);
            f.bindings.retain(|x| parse_combo(&x.keys) != c);
            f.bindings.push(b);
            app.keymap.set(f)?;
            Ok(json!({"keys": keys}))
        })(),
        "keys.record" => {
            // {index?}: record the next keys for user binding `index`, else for the draft {command, params, label}
            if let Some(c) = s("command") {
                app.keymap.draft = Binding::new("", c, p.get("params").cloned().unwrap_or(Value::Null), s("label").unwrap_or(""));
            }
            app.keymap.recording = Some(p.get("index").and_then(Value::as_u64).map(|i| i as usize));
            Ok(Value::Null)
        }
        "keys.import" => import(app, p),
        "keys.midi" => (|| {
            // {midi: cc|note, number, value?, channel?} or {bytes: [..]}: as if from the MIDI input
            let msg = if let Some(b) = p.get("bytes").and_then(Value::as_array) {
                let bytes: Vec<u8> = b.iter().filter_map(Value::as_u64).map(|x| x.min(255) as u8).collect();
                parse_midi(&bytes).ok_or("keys.midi: not a CC or note message")?
            } else {
                let n = |k: &str, d: u64| p.get(k).and_then(Value::as_u64).unwrap_or(d).min(127) as u8;
                let kind = match s("midi").unwrap_or("cc") {
                    "note" => MidiKind::NoteOn,
                    "noteOff" => MidiKind::NoteOff,
                    _ => MidiKind::Cc,
                };
                MidiMsg { kind, channel: n("channel", 1).clamp(1, 16), number: n("number", 0), value: n("value", 127) }
            };
            let ctx = egui::Context::default();
            Ok(json!({"msg": msg, "done": handle_midi(app, &ctx, vec![msg])}))
        })(),
        "keys.learn" => {
            // {on: bool}: wait for a control to move (ctrl. page)
            app.keymap.learn = p.get("on").and_then(Value::as_bool).unwrap_or(true).then_some(None);
            Ok(Value::Null)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse_and_print() {
        let c = parse_combo("opt+q").unwrap();
        assert!(c.alt && !c.cmd && c.trigger == Trigger::Key(Key::Q));
        assert_eq!(format_combo(&c), "Alt+Q");
        assert_eq!(canonical("Alt+\\"), "Alt+\\");
        assert_eq!(canonical("Cmd+Alt+V"), "Cmd+Alt+V");
        assert_eq!(canonical("shift+cmd+s"), "Cmd+Shift+S");
        assert_eq!(canonical("MouseBack"), "MouseBack");
        assert_eq!(parse_combo("Mouse5").unwrap().trigger, Trigger::Mouse(PointerButton::Extra2));
        assert_eq!(canonical("Delete"), "Delete");
        assert_eq!(canonical("Right"), "Right");
        assert_eq!(mods_keys(&["alt".into()], "D4"), "Alt+4");
        assert_eq!(mods_keys(&["alt".into()], "Backslash"), "Alt+\\");
        // never panic on junk
        for junk in ["", "+", "Cmd+", "Hyper+Q", "Cmd+Shift+NotAKey", "++++", "Alt+ü", "\u{0}"] {
            let _ = parse_combo(junk);
            let _ = canonical(junk);
        }
        assert!(parse_combo("Hyper+Q").is_none());
        // every built-in prints back to a combo it parses from
        for (k, ..) in builtin() {
            let c = parse_combo(&k).unwrap_or_else(|| panic!("{k}"));
            assert_eq!(parse_combo(&format_combo(&c)), Some(c), "{k}");
        }
    }

    #[test]
    fn classic_profile_targets_existing_commands() {
        let ui: Vec<&str> = crate::menus::ui_commands().map(|c| c.0).collect();
        for (k, c, p) in CLASSIC {
            assert!(parse_combo(k).is_some(), "{k}");
            assert!(c.is_empty() || ui.contains(c) || lightcraft_engine::find_command(c).is_some(), "{k} → unknown {c}");
            assert!(serde_json::from_str::<Value>(p).is_ok(), "{k}");
        }
        for (_, l, c, p) in SUPER_ACTIONS {
            assert!(ui.contains(c) || lightcraft_engine::find_command(c).is_some(), "{l} → unknown {c}");
            assert!(serde_json::from_str::<Value>(p).is_ok(), "{l}");
        }
        for (_, c) in LR_PARAMS {
            assert!(CROP_PSEUDO.contains(c) || lightcraft_develop::controls::find(c).is_some(), "{c}");
        }
    }

    #[test]
    fn layers_merge_override_and_unbind() {
        let classic = KeymapFile { profile: "classic".into(), ..Default::default() };
        let ov = overlay(&classic);
        // Classic: R = crop, D = develop, E = loupe; untouched keys keep the built-in
        assert_eq!(resolve(&ov, "R").unwrap().0, "panel.crop");
        assert_eq!(resolve(&ov, "D").unwrap().0, "view.develop");
        assert_eq!(resolve(&ov, "P").unwrap().0, "photo.pick");
        assert_eq!(resolve(&ov, "MouseBack").unwrap().0, "library.previous");
        assert_eq!(resolve(&ov, "L").unwrap().0, "view.lightsOut");
        // the desktop profile = the built-ins
        let ov = overlay(&KeymapFile::default());
        assert_eq!(resolve(&ov, "R").unwrap().0, "tool.radial");
        // user over profile; an empty command unbinds
        let mut f = classic.clone();
        f.bindings.push(Binding::new("R", "tool.radial", json!({}), ""));
        f.bindings.push(Binding::new("alt+q", "crop.aspect", json!({"aspect": "1x1"}), ""));
        f.bindings.push(Binding::new("P", "", Value::Null, ""));
        let ov = overlay(&f);
        assert_eq!(resolve(&ov, "R").unwrap().0, "tool.radial");
        assert_eq!(resolve(&ov, "Alt+Q").unwrap(), ("crop.aspect".to_string(), json!({"aspect": "1x1"})));
        assert_eq!(lookup(&ov, &parse_combo("P").unwrap()), Some(None));
        // menus show the effective key
        assert_eq!(menu_shortcut(&ov, "panel.crop", Some("C")), None, "Classic C = compare");
        assert_eq!(menu_shortcut(&ov, "view.compare", Some("Shift+C")).as_deref(), Some("C"));
        assert_eq!(menu_shortcut(&ov, "photo.pick", Some("P")), None);
    }

    #[test]
    fn conflicts_and_overrides_are_reported() {
        let mut f = KeymapFile { profile: "classic".into(), ..Default::default() };
        f.bindings.push(Binding::new("Alt+Q", "crop.aspect", json!({"aspect": "1x1"}), ""));
        f.bindings.push(Binding::new("Alt+Q", "crop.reset", json!({}), ""));
        let rows = rows(&f);
        let alt_q: Vec<&Row> = rows.iter().filter(|r| r.keys == "Alt+Q").collect();
        assert_eq!(alt_q.len(), 2);
        assert!(alt_q.iter().all(|r| r.conflict));
        assert!(alt_q.first().is_some_and(|r| r.overridden), "the earlier one doesn't fire");
        let r_builtin = rows.iter().find(|r| r.keys == "R" && r.origin == Origin::Builtin).unwrap();
        assert!(r_builtin.overridden && r_builtin.command == "tool.radial");
    }

    #[test]
    fn bad_files_never_crash() {
        assert!(parse_file("{").is_err());
        assert!(parse_file("[1,2]").is_err());
        assert_eq!(parse_file("").unwrap(), KeymapFile::default());
        // wrong types inside are errors, unknown fields are fine
        assert!(parse_file(r#"{"bindings": 3}"#).is_err());
        let f = parse_file(r#"{"profile": "classic", "future": true, "bindings": [{"keys": "Nope+Q", "command": "x"}, {"keys": "Alt+Q"}]}"#).unwrap();
        // a binding with unknown keys is ignored; one without a command unbinds
        let ov = overlay(&f);
        assert_eq!(ov.iter().filter(|(_, _, o)| *o == Origin::User).count(), 1);
        // live reload: a damaged file keeps the previous bindings and reports the error
        let dir = std::env::temp_dir().join(format!("lc-keymap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("keymap.json");
        std::fs::write(&p, r#"{"profile": "classic"}"#).unwrap();
        let mut k = Keymap::at(Some(p.clone()));
        assert!(k.error.is_none() && k.file.profile == "classic");
        std::fs::write(&p, "{ not json").unwrap();
        k.reload();
        assert!(k.error.is_some() && k.file.profile == "classic");
        // and saving refuses to overwrite it
        assert!(k.save().is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ not json");
        std::fs::remove_dir_all(&dir).ok();
        // a missing file = defaults
        let k = Keymap::at(Some(std::env::temp_dir().join("lc-keymap-none/keymap.json")));
        assert!(k.error.is_none() && k.file == KeymapFile::default());
    }

    const LRKEYS: &str = r#"{"options": {"enabled": true}, "actions": [
        {"id": "copyprev", "group": "Settings", "label": "Paste Settings from Previous", "kind": "keys", "send": {"mods": ["cmd", "alt"], "key": "v"}, "mods": ["alt"], "key": "\\"},
        {"id": "crop_asshot", "label": "As Shot ratio", "kind": "url", "cmd": "crop?a=asshot", "mods": ["alt"], "key": "4"},
        {"id": "crop_1x1", "label": "1 x 1", "kind": "url", "cmd": "crop?w=1&h=1", "mods": ["alt"], "key": "q"},
        {"id": "crop_8x10", "label": "4 x 5  /  8 x 10", "kind": "url", "cmd": "crop?w=8&h=10", "mods": ["alt"], "key": "o"},
        {"id": "crop_reset", "label": "Reset crop", "kind": "url", "cmd": "cropreset", "mods": ["alt"], "key": "r"},
        {"id": "straighten", "label": "Auto straighten", "kind": "url", "cmd": "straighten", "mods": ["alt"], "key": "u"},
        {"id": "crop_2x3", "label": "2 x 3", "kind": "url", "cmd": "crop?w=2&h=3", "mods": [], "key": null},
        {"id": "exp_up", "label": "Exposure +1/3", "kind": "url", "cmd": "param?name=Exposure&delta=0.33", "mods": [], "key": null},
        {"id": "weird", "label": "Upright", "kind": "url", "cmd": "upright?v=2", "mods": [], "key": "u"},
        {"id": "junk"}, 7
    ]}"#;

    #[test]
    fn imports_lrkeys() {
        let i = import_lrkeys(LRKEYS).unwrap();
        let find = |k: &str| i.bindings.iter().find(|b| b.keys == k).map(|b| (b.command.as_str(), b.params.clone()));
        assert_eq!(find("Alt+\\"), Some(("develop.pastePrevious", json!({}))));
        assert_eq!(find("Alt+4"), Some(("crop.aspect", json!({"aspect": "original"}))));
        assert_eq!(find("Alt+Q"), Some(("crop.aspect", json!({"aspect": "1x1"}))));
        assert_eq!(find("Alt+O"), Some(("crop.aspect", json!({"aspect": "8x10"}))));
        assert_eq!(find("Alt+R"), Some(("crop.reset", json!({}))));
        assert_eq!(find("Alt+U"), Some(("crop.autoStraighten", json!({}))));
        assert_eq!(i.bindings.len(), 6);
        assert!(i.skipped.iter().any(|s| s.contains("2 x 3") && s.contains("no key")));
        assert!(i.skipped.iter().any(|s| s.contains("upright")));
        assert_eq!(
            lrkeys_action("param?name=Exposure&delta=-0.33"),
            Some(("develop.adjust".into(), json!({"control": "light.exposure", "delta": -0.33})))
        );
        assert_eq!(lrkeys_action("flag?v=-1"), Some(("photo.flag".into(), json!({"flag": "reject"}))));
        assert_eq!(lrkeys_action("preset?name=My%20Preset"), Some(("preset.applyByName".into(), json!({"name": "My Preset"}))));
        for junk in ["", "?", "crop?w=0&h=0", "param?name=Nope&delta=1", "rating?v=x", "crop?w=a&h=b"] {
            assert!(lrkeys_action(junk).is_none(), "{junk}");
        }
        assert!(import_lrkeys("nope").is_err());
        assert!(import_lrkeys("{}").is_err());
    }

    const SHORTCUTS_XML: &str = r#"<?xml version="1.0" standalone="yes"?>
<DocumentElement>
  <ShortCuts><superkey>Backslash</superkey><ModifierKey>4</ModifierKey><type>20</type>
    <KeyboardShortCut><Description>Copy Previous</Description><MacCommand>^%v</MacCommand></KeyboardShortCut></ShortCuts>
  <ShortCuts><superkey>D4</superkey><ModifierKey>2</ModifierKey><type>30</type>
    <LrFunction><Description>As Shot Crop Ratio</Description><FunctionCommandString>CropRatio = ratio="asshot"</FunctionCommandString></LrFunction></ShortCuts>
  <ShortCuts><superkey>O</superkey><ModifierKey>0</ModifierKey><type>30</type>
    <LrFunction><Description>4 x 5  /  8 x 10 Crop</Description><FunctionCommandString>CropRatio = ratio={w=8,h=10}</FunctionCommandString></LrFunction></ShortCuts>
  <ShortCuts><superkey>U</superkey><type>30</type>
    <LrFunction><Description>Auto Straighten</Description><FunctionCommandString>AUTOCROP</FunctionCommandString></LrFunction></ShortCuts>
  <ShortCuts><superkey>Z</superkey><type>30</type>
    <LrFunction><Description>Mystery</Description><FunctionCommandString>FROBNICATE</FunctionCommandString></LrFunction></ShortCuts>
  <ShortCuts><superkey>X</superkey><ModifierKey>4</ModifierKey>
</DocumentElement>"#;

    const SPEED_XML: &str = r#"<DocumentElement>
  <SpeedKeys><lrParam>--SPECIALKEYS</lrParam><Description>Super Keys</Description><SuperKey /><DeltaChange>0</DeltaChange></SpeedKeys>
  <SpeedKeys><lrParam>Exposure</lrParam><Description>Exposure</Description><SuperKey /><DeltaChange>0.05</DeltaChange><DeltaChangeLarge>0.10</DeltaChangeLarge><IncreaseKey /><DecreaseKey /></SpeedKeys>
  <SpeedKeys><lrParam>HSLKeyH</lrParam><Description>Hue</Description><SuperKey>H</SuperKey><DeltaChange>1</DeltaChange><DeltaChangeLarge>5</DeltaChangeLarge></SpeedKeys>
  <SpeedKeys><lrParam>GrayMixerRed</lrParam><Description>B &amp; W - Red</Description><SuperKey /><DeltaChange>1</DeltaChange><DeltaChangeLarge>5</DeltaChangeLarge><IncreaseKey>F13</IncreaseKey></SpeedKeys>
  <SpeedKeys><lrParam>LensBlurAmount</lrParam><Description>Blur Amount</Description><SuperKey /><DeltaChange>1</DeltaChange><DeltaChangeLarge>5</DeltaChangeLarge></SpeedKeys>
  <SpeedKeys><lrParam>Broken"#;

    #[test]
    fn imports_superkeys() {
        let i = import_superkeys(SHORTCUTS_XML, Some(SPEED_XML));
        let find = |k: &str| i.bindings.iter().find(|b| b.keys == k).map(|b| (b.command.clone(), b.params.clone()));
        assert_eq!(find("Alt+\\").unwrap().0, "develop.pastePrevious");
        assert_eq!(find("Alt+4").unwrap().1, json!({"aspect": "original"}));
        assert_eq!(find("Alt+O").unwrap().1, json!({"aspect": "8x10"}));
        assert_eq!(find("Alt+U").unwrap().0, "crop.autoStraighten");
        assert_eq!(find("H").unwrap(), ("view.colorMixer".to_string(), json!({"mode": "hue"})));
        assert_eq!(find("F13").unwrap(), ("keys.nudge".to_string(), json!({"control": "bw.red", "dir": 1})));
        assert!(i.skipped.iter().any(|s| s.contains("Mystery")));
        assert!(i.skipped.iter().any(|s| s.contains("Blur Amount")));
        assert_eq!(i.steps.get("light.exposure"), Some(&[0.05, 0.10]));
        // junk in, nothing out, no panic
        let i = import_superkeys("<<<ShortCuts><superkey>", Some("<SpeedKeys>"));
        assert!(i.bindings.is_empty());
    }

    #[test]
    fn midi_messages_map_to_actions() {
        assert_eq!(parse_midi(&[0xB0, 21, 64]), Some(MidiMsg { kind: MidiKind::Cc, channel: 1, number: 21, value: 64 }));
        assert_eq!(parse_midi(&[0x93, 60, 100]).unwrap().kind, MidiKind::NoteOn);
        assert_eq!(parse_midi(&[0x90, 60, 0]).unwrap().kind, MidiKind::NoteOff);
        assert_eq!(parse_midi(&[0xF8]), None);
        assert_eq!(parse_midi(&[]), None);
        assert_eq!(parse_midi(&[0xB0, 1]), None);
        assert_eq!(relative_ticks(1), 1);
        assert_eq!(relative_ticks(127), -1);
        assert_eq!(relative_ticks(65), 1);
        assert_eq!(relative_ticks(63), -1);
        assert_eq!(relative_ticks(3), 3);
        let km = Keymap::from_file(KeymapFile {
            midi: vec![
                MidiBinding { midi: "cc".into(), number: 20, channel: 1, control: "light.shadows".into(), mode: "abs".into(), ..Default::default() },
                MidiBinding { midi: "cc".into(), number: 30, channel: 0, control: "light.exposure".into(), mode: "rel".into(), ..Default::default() },
                MidiBinding {
                    midi: "cc".into(),
                    number: 31,
                    channel: 1,
                    control: "crop.x".into(),
                    mode: "rel".into(),
                    invert: true,
                    ..Default::default()
                },
                MidiBinding { midi: "note".into(), number: 60, channel: 1, command: "view.beforeAfter".into(), ..Default::default() },
                MidiBinding {
                    midi: "cc".into(),
                    number: 40,
                    channel: 1,
                    command: "photo.flag".into(),
                    params: json!({"flag": "pick"}),
                    ..Default::default()
                },
            ],
            steps: [("light.exposure".to_string(), [0.05, 0.1])].into_iter().collect(),
            ..Default::default()
        });
        let cc = |n: u8, v: u8, ch: u8| MidiMsg { kind: MidiKind::Cc, channel: ch, number: n, value: v };
        assert_eq!(midi_action(&km, &cc(20, 0, 1)), Some(MidiAction::Set { control: "light.shadows".into(), value: -100.0 }));
        assert_eq!(midi_action(&km, &cc(20, 127, 1)), Some(MidiAction::Set { control: "light.shadows".into(), value: 100.0 }));
        assert_eq!(midi_action(&km, &cc(20, 64, 2)), None, "wrong channel");
        let Some(MidiAction::Nudge { delta, .. }) = midi_action(&km, &cc(30, 127, 9)) else { panic!() };
        assert!((delta + 0.05).abs() < 1e-9);
        let Some(MidiAction::Nudge { control, delta }) = midi_action(&km, &cc(31, 2, 1)) else { panic!() };
        assert!(control == "crop.x" && delta < 0.0, "inverted");
        assert_eq!(
            midi_action(&km, &MidiMsg { kind: MidiKind::NoteOn, channel: 1, number: 60, value: 90 }).unwrap(),
            MidiAction::Run { command: "view.beforeAfter".into(), params: json!({}) }
        );
        assert_eq!(midi_action(&km, &MidiMsg { kind: MidiKind::NoteOff, channel: 1, number: 60, value: 0 }), None);
        assert!(matches!(midi_action(&km, &cc(40, 127, 1)), Some(MidiAction::Run { .. })));
        assert_eq!(midi_action(&km, &cc(40, 0, 1)), None);
        assert_eq!(midi_action(&km, &cc(99, 1, 1)), None);
        let merged = coalesce(vec![
            MidiAction::Set { control: "a".into(), value: 1.0 },
            MidiAction::Nudge { control: "b".into(), delta: 1.0 },
            MidiAction::Set { control: "a".into(), value: 2.0 },
            MidiAction::Nudge { control: "b".into(), delta: 2.0 },
        ]);
        assert_eq!(merged, vec![MidiAction::Set { control: "a".into(), value: 2.0 }, MidiAction::Nudge { control: "b".into(), delta: 3.0 }]);
        let l = learned_binding(&cc(5, 10, 3), "light.contrast", true, "", Value::Null);
        assert!(l.mode == "rel" && l.channel == 3 && l.command.is_empty());
    }

    #[test]
    fn imports_monogram_profile() {
        let state: Value = serde_json::from_str(
            r##"{"profiles": [{"title": "Lightroom 1", "app": "com.adobe.lightroom", "assignments": {
            "00005": {"fixedPosition": true, "submodules": {}, "settings": {}},
            "  Ak2": {"app": "com.adobe.lightroom", "settings": {"name": "Shadows = ", "pose": {"input": "Shadows = "}}},
            "  C7J": {"settings": {"press": {"key": "m", "modifiers": ["shift"]}, "pressAndTurn": {"input": "Tint = "}, "turn": {"input": "Exposure = "}}},
            "  H|r": {"settings": {"doubleTap": ["Blacks = ", "reset"], "press": {"key": "w", "modifiers": []}, "pressAndTurn": {"input": "ParametricLights = "}, "turn": {"input": "Temperature = "}}},
            "  AP>": {"settings": {"pressAndTurn": {"input": "CropPositionX = ", "invert": true, "step": 1}, "turn": {"input": "straightenAngle = "}}},
            "  BFo": {"settings": {"press": {"interval": 50, "macro": [{"input": "set_preset = BH - NMD - 7 - TD"}, {}]}, "pressAndHold": {"interval": 50, "macro": [{"input": "set_preset = BH - B&W - HardBlack"}, {"key": "u", "modifiers": []}]}}},
            "  BH6": {"settings": {"press": {"macro": [{"input": "develop_before_after_horiz = 0"}, {}]}, "pressAndHold": {"key": "f", "modifiers": []}}},
            "  C{>": {"settings": {"leftTurn": {"key": "down"}, "pressAndTurn": {"input": "Mystery = "}}}
            }}]}"##,
        )
        .unwrap();
        let (i, mono) = import_monogram(&state["profiles"][0], "ai.storyteller.lightcraft");
        let by = |label: &str| i.midi.iter().find(|m| m.label == label).cloned().unwrap_or_else(|| panic!("{label}: {:?}", i.midi));
        assert_eq!(by("Ak2 slider").control, "light.shadows");
        assert_eq!(by("Ak2 slider").mode, "abs");
        assert_eq!(by("C7J turn").control, "light.exposure");
        assert_eq!(by("C7J press+turn").control, "wb.tint");
        assert_eq!(by("C7J press").command, "tool.radial", "Shift+M = radial filter in Classic");
        assert_eq!(by("H|r turn").control, "wb.temp");
        assert_eq!(by("H|r press+turn").control, "curve.lights");
        assert_eq!(by("H|r press").command, "tool.wbPicker");
        assert_eq!(
            (by("H|r double-tap").command, by("H|r double-tap").params),
            ("develop.resetControl".to_string(), json!({"control": "light.blacks"}))
        );
        assert!(by("AP> press+turn").invert && by("AP> press+turn").control == "crop.x");
        assert_eq!(by("AP> turn").control, "crop.angle");
        assert_eq!((by("BFo press").command, by("BFo press").params), ("preset.applyByName".to_string(), json!({"name": "BH - NMD - 7 - TD"})));
        let hold = by("BFo hold");
        assert_eq!(hold.command, "keys.macro");
        assert_eq!(hold.params["steps"][0], json!({"command": "preset.applyByName", "params": {"name": "BH - B&W - HardBlack"}}));
        assert_eq!(hold.params["steps"][1]["command"], "photo.unflag");
        assert_eq!(by("BH6 press").command, "view.beforeAfter");
        assert_eq!(by("BH6 hold").command, "view.fullScreenPreview");
        assert!(i.skipped.iter().any(|s| s.contains("C{>") && s.contains("Mystery")));
        // MIDI numbers are unique per kind
        let mut seen = std::collections::HashSet::new();
        assert!(i.midi.iter().all(|m| seen.insert((m.midi.clone(), m.number))));
        // the Monogram side sends exactly those messages
        let a = &mono["assignments"];
        assert_eq!(a["  Ak2"]["settings"]["pose"], json!({"midi": "cc", "cc": by("Ak2 slider").number, "channel": 1}));
        assert_eq!(a["  C7J"]["settings"]["turn"]["midi"], "cc");
        assert_eq!(a["  C7J"]["settings"]["press"], json!({"midi": "note", "note": by("C7J press").number, "channel": 1}));
        assert_eq!(a["00005"]["fixedPosition"], true);
        assert_eq!(mono["app"], "ai.storyteller.lightcraft");
        // junk profiles don't panic
        let _ = import_monogram(&json!(null), "x");
        let _ = import_monogram(&json!({"assignments": {"a": 1, "b": {"settings": {"pose": 3, "press": {"macro": "no"}, "doubleTap": [1]}}}}), "x");
    }
}
