//! Commands for hardware buttons (Stream Deck, Monogram, via Hammerspoon and the control port):
//! `view.section {id, control?}` opens a Develop panel (never closes one) and can pick one of its
//! sliders the way a click on the slider's name does; `tool.targetedAdjustment {target, on?}` arms
//! the targeted adjustment tool the way the panel's button does. Both are safe to press twice.
//!
//! A `control` the panel would not draw (a B&W photo's colour mixer, Lens Corrections' profile
//! amounts with profile corrections off, a point colour that doesn't exist…) is an error that says
//! why, and nothing is changed or left waiting for the next frame. A slider that is drawn but
//! greyed out falls back to opening and scrolling to its section, with a `note`.

use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::RightPanel;
use lightcraft_develop::{ControlSpec, DevelopSettings, Section, controls};

/// What a command asked for that needs the egui context (the picked slider, the scroll and the
/// Lens Corrections tab live in it): [`frame`] takes it once per frame, before the panels are drawn.
#[derive(Clone, Debug, Default)]
pub struct Pending {
    /// The slider to pick (as a click on its name does).
    pub slider: Option<String>,
    /// The automation id of the widget to scroll into view (`section:basic`, `slider:light.shadows`).
    pub reveal: Option<String>,
    /// Lens Corrections' tab to show: `true` Manual, `false` Profile.
    pub lens_manual: Option<bool>,
}

/// Run `id` when it is one of ours.
pub fn run(app: &mut LightcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "view.section" => Some(view_section(app, p)),
        "tool.targetedAdjustment" => Some(tool_targeted_adjustment(app, p)),
        _ => None,
    }
}

/// Once a frame: pick the slider, show the tab and ask for the scroll the last command left.
pub fn frame(app: &mut LightcraftApp, ctx: &egui::Context) {
    let d = &app.ui.device;
    if d.slider.is_none() && d.reveal.is_none() && d.lens_manual.is_none() {
        return;
    }
    let pending = std::mem::take(&mut app.ui.device);
    if let Some(c) = pending.slider {
        crate::widgets::pick_slider(ctx, &c);
    }
    if let Some(manual) = pending.lens_manual {
        // the tab's memory in `panels::edit::lens_corrections`
        ctx.data_mut(|m| m.insert_temp(egui::Id::new("lens-corrections-tab"), manual));
    }
    if let Some(w) = pending.reveal {
        crate::widgets::request_reveal(ctx, &w);
    }
    ctx.request_repaint();
}

fn text<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.trim())),
        Some(_) => Err(format!("`{key}` must be a non-empty string")),
    }
}

/// The Develop panel (a `CLASSIC_PANELS` id) a control lives in.
fn panel_of(section: Section) -> Option<&'static str> {
    Some(match section {
        Section::Light | Section::Color | Section::Effects | Section::Profile => "basic",
        Section::Curve => "toneCurve",
        Section::Mixer | Section::BwMix | Section::PointColor => "hsl",
        Section::Grading => "colorGrading",
        Section::Detail => "detail",
        Section::Optics => "lensCorrections",
        Section::Geometry => "transform",
        Section::Vignette | Section::Grain => "effects",
        Section::Calibration => "calibration",
        Section::LensBlur => "lensBlur",
        Section::RedEye | Section::Reflection => return None,
    })
}

/// The Develop panel a slider (as drawn: a control id, or a rendered file's relative white balance)
/// is in; Tab steps round the sliders of one panel.
pub(crate) fn panel_of_slider(id: &str) -> &'static str {
    match controls::find(id) {
        Some(s) => panel_of(s.section).unwrap_or(""),
        None if id.starts_with("wb.") => "basic",
        None if id.starts_with("pointColor.") => "hsl",
        None => "",
    }
}

/// What has to be so for the panel to draw a control's slider, worked out before anything changes.
#[derive(Default)]
struct Plan {
    /// The slider id as drawn (a rendered file's white balance sliders have their own ids).
    pick: Option<String>,
    /// Collapsible sub-sections (`ui.fold` ids) to unfold.
    folds: Vec<&'static str>,
    mixer_mode: Option<String>,
    curve_channel: Option<&'static str>,
    lens_manual: Option<bool>,
    /// The point colour to select (its flyout opens).
    point: Option<usize>,
    /// Drawn but greyed out: why. The slider can't be picked then.
    greyed: Option<String>,
}

fn not_shown(control: &str, why: &str) -> String {
    format!("view.section: `{control}` isn't shown: {why}")
}

/// Work out where `control` (a develop control id, `pointColor.<n>.<field>` included) is drawn, or
/// why it isn't, for the photo being developed.
fn plan(app: &LightcraftApp, control: &str) -> Result<Plan, String> {
    let no = |why: &str| not_shown(control, why);
    let id = app.session.active().ok_or_else(|| no("no photo is selected"))?;
    let d: std::sync::Arc<DevelopSettings> = app.session.develop_of(id).ok_or_else(|| no("the photo has no develop settings"))?;
    let bw = crate::is_bw(&d);
    let mut p = Plan { pick: Some(control.to_string()), ..Plan::default() };
    let grey = |p: &mut Plan, when: bool, why: &str| {
        if when {
            p.greyed = Some(format!("`{control}` is greyed out: {why}"));
        }
    };
    match control {
        "profile.amount" if !lightcraft_engine::presets::profile_has_amount(&d.profile.id) => return Err(no("the photo's profile has no amount")),
        "wb.temp" | "wb.tint" => {
            // a rendered file shows relative sliders with ids of their own (`Basic`)
            let raw = app.session.catalog.photo(id).is_some_and(|ph| ph.develops_raw() && !ph.relative_wb());
            if !raw {
                p.pick = Some(format!("{control}Rel"));
            }
        }
        "crop.angle" => return Err(no("it is the Crop & Straighten panel's slider (Edit has Transform)")),
        c if c.starts_with("curve.split") => return Err(no("the Tone Curve panel has no slider for it")),
        "curve.highlights" | "curve.lights" | "curve.darks" | "curve.shadows" => p.curve_channel = Some("parametric"),
        c if c.starts_with("hdr.") => {
            if !d.hdr.enabled {
                return Err(no("HDR is off (hdr.)"));
            }
            if c.starts_with("hdr.sdr") {
                if !app.ui.hdr_sdr_preview {
                    return Err(no("Preview for SDR Display is off (sdr.preview.)"));
                }
                p.folds.push("develop.hdr.sdr.settings");
            }
        }
        c if c.starts_with("mixer.") => {
            if bw {
                return Err(no("the photo is black & white, so HSL / Color shows the B&W mix (bw.*)"));
            }
            let mut parts = c["mixer.".len()..].split('.');
            if let (Some(band), Some(attr)) = (parts.next(), parts.next()) {
                let mode = app.ui.mixer_mode.as_str();
                if mode == "all" {
                    p.folds.push(match attr {
                        "hue" => "develop.mixer.hue",
                        "sat" => "develop.mixer.sat",
                        _ => "develop.mixer.lum",
                    });
                } else if mode != band {
                    let want = match attr {
                        "hue" => "hue",
                        "sat" => "saturation",
                        _ => "luminance",
                    };
                    if mode != want {
                        p.mixer_mode = Some(want.to_string());
                    }
                }
            }
        }
        c if c.starts_with("bw.") && !bw => return Err(no("the photo is in colour, so HSL / Color shows the colour mixer (mixer.*)")),
        c if c.starts_with("grading.global.") => return Err(no("the Color Grading panel has no Global wheel")),
        c if c.starts_with("grading.") && (c.ends_with(".hue") || c.ends_with(".sat")) => {
            return Err(no("Color Grading sets hue and saturation with its colour wheels, not sliders"));
        }
        "enhance.denoise" => {
            if lightcraft_engine::enhance::options_of(&d).denoise <= 0.0 {
                return Err(no("Denoise isn't applied in place"));
            }
            p.folds.push("develop.detail.noise.reduction");
        }
        c if c.starts_with("detail.sharpen") => {
            p.folds.push("develop.detail.sharpening");
            grey(&mut p, c != "detail.sharpenAmount" && d.detail.sharpen_amount <= 0.0, "Sharpening Amount is 0");
        }
        c if c.starts_with("detail.nr") => p.folds.push("develop.detail.noise.reduction"),
        "optics.profileDistortion" | "optics.profileVignetting" => {
            if !d.optics.lens_profile {
                return Err(no("Enable Profile Corrections is off"));
            }
            p.lens_manual = Some(false);
            p.folds.push("develop.lens.amount");
            let has_lens = app.session.catalog.photo(id).is_some_and(|ph| ph.embedded_lens.is_some());
            grey(&mut p, !has_lens && d.optics.lens_profile_ref.is_none(), "the photo has no lens profile");
        }
        "optics.distortion" => {
            p.lens_manual = Some(true);
            p.folds.push("develop.lens.distortion");
        }
        c if c.starts_with("optics.defringe") => {
            p.lens_manual = Some(true);
            p.folds.push("develop.lens.defringe");
            let amount = if c.contains("Purple") { d.optics.defringe_purple_amount } else { d.optics.defringe_green_amount };
            grey(&mut p, c.contains("Hue") && amount <= 0.0, "its Amount is 0");
        }
        "optics.caRed" | "optics.caBlue" => {
            p.lens_manual = Some(true);
            p.folds.push("develop.lens.lateral.chromatic.aberration");
        }
        c if c.starts_with("optics.vignetting") => {
            p.lens_manual = Some(true);
            p.folds.push("develop.lens.vignetting");
            grey(&mut p, c == "optics.vignettingMidpoint" && d.optics.vignetting == 0.0, "Lens Vignetting Amount is 0");
        }
        c if c.starts_with("vignette.") => {
            p.folds.push("develop.effects.post.crop.vignetting");
            grey(&mut p, c != "vignette.amount" && d.vignette.amount == 0.0, "Post-Crop Vignetting Amount is 0");
        }
        c if c.starts_with("grain.") => {
            p.folds.push("develop.effects.grain");
            grey(&mut p, c != "grain.amount" && d.grain.amount == 0.0, "Grain Amount is 0");
        }
        c if c.starts_with("lensBlur.") => {
            if !d.lens_blur.enabled {
                return Err(no("Lens Blur isn't applied (tick Apply)"));
            }
            if c == "lensBlur.focalDistance" || c == "lensBlur.focalRange" {
                p.folds.push("develop.lens.blur.focal.range");
            }
        }
        "calibration.shadowsTint" => p.folds.push("develop.calibration.shadows"),
        "calibration.redHue" | "calibration.redSat" => p.folds.push("develop.calibration.red.primary"),
        "calibration.greenHue" | "calibration.greenSat" => p.folds.push("develop.calibration.green.primary"),
        "calibration.blueHue" | "calibration.blueSat" => p.folds.push("develop.calibration.blue.primary"),
        c if c.starts_with("pointColor.") => {
            if bw {
                return Err(no("the photo is black & white (no Point Color)"));
            }
            let (n, field) = c["pointColor.".len()..].split_once('.').ok_or_else(|| no("name a point as pointColor.<n>.<field>"))?;
            let n: usize = n.parse().map_err(|_| no("the point must be a number"))?;
            if n >= d.point_colors.len() {
                return Err(no(&format!("the photo has {} point color(s), numbered from 0", d.point_colors.len())));
            }
            p.point = Some(n);
            // the panel draws the selected point's sliders under the template's id
            p.pick = Some(format!("pointColor.{field}"));
            if matches!(field, "range" | "hueRange" | "satRange" | "lumRange") {
                p.folds.push("develop.point.color.range");
            }
        }
        _ => {}
    }
    if p.greyed.is_some() {
        p.pick = None;
    }
    Ok(p)
}

/// `view.section {id, control?}`.
fn view_section(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let id = text(p, "id").map_err(|e| format!("view.section: {e}"))?;
    let control = text(p, "control").map_err(|e| format!("view.section: {e}"))?;
    let spec: Option<&'static ControlSpec> = match control {
        Some(c) => Some(controls::find(c).ok_or_else(|| format!("view.section: unknown control `{c}`"))?),
        None => None,
    };
    let own = match (spec, control) {
        (Some(s), Some(c)) => Some(panel_of(s.section).ok_or_else(|| format!("view.section: control `{c}` is not in a Develop panel"))?),
        _ => None,
    };
    let section = match (id, own) {
        (Some(id), own) => {
            let sec = crate::panels::edit::classic_section(id);
            if !crate::panels::edit::CLASSIC_PANELS.iter().any(|c| c.0 == sec) {
                return Err(format!("view.section: unknown section `{id}`"));
            }
            if let (Some(own), Some(c)) = (own, control)
                && own != sec
            {
                return Err(format!("view.section: control `{c}` is in `{own}`, not `{sec}`"));
            }
            sec
        }
        (None, Some(own)) => own,
        (None, None) => return Err("view.section: missing `id` (basic, toneCurve, hsl, colorGrading, detail, …)".into()),
    };
    // everything that can fail is decided before anything changes
    let plan = match control {
        Some(c) => Some(plan(app, c)?),
        None => None,
    };
    crate::menus::enter_develop(app);
    app.ui.right = RightPanel::Edit;
    if !app.ui.section_open(section) {
        app.ui.toggle_section(section);
    }
    let mut reveal = format!("section:{section}");
    let mut note = None;
    let mut picked = None;
    if let Some(plan) = plan {
        for f in plan.folds {
            crate::folds::set_in(&mut app.ui.folds, f, true);
        }
        if let Some(m) = plan.mixer_mode {
            app.ui.mixer_mode = m;
        }
        if let Some(ch) = plan.curve_channel {
            app.ui.curve_channel = ch.to_string();
        }
        if let Some(n) = plan.point {
            app.ui.point_color = n;
            if !app.ui.flyout_open("pointColor") {
                app.ui.toggle_flyout("pointColor");
            }
        }
        app.ui.device.lens_manual = plan.lens_manual;
        if let Some(pick) = plan.pick {
            picked = Some(pick.clone());
            reveal = format!("slider:{pick}");
            app.ui.device.slider = Some(pick);
        }
        note = plan.greyed;
    }
    app.ui.device.reveal = Some(reveal);
    Ok(json!({"section": section, "open": app.ui.section_open(section), "control": control, "picked": picked, "note": note}))
}

/// `tool.targetedAdjustment {target: curve|hue|saturation|luminance, on?}`.
fn tool_targeted_adjustment(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let target = text(p, "target")
        .map_err(|e| format!("tool.targetedAdjustment: {e}"))?
        .ok_or("tool.targetedAdjustment: missing `target` (curve, hue, saturation, luminance)")?;
    let on = match p.get("on") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or("tool.targetedAdjustment: `on` must be true or false")?,
    };
    let (tool, mode) = match target.to_ascii_lowercase().as_str() {
        "curve" => ("tat:curve", None),
        "hue" => ("tat:hue", Some("hue")),
        "saturation" | "sat" => ("tat:sat", Some("saturation")),
        "luminance" | "lum" => ("tat:lum", Some("luminance")),
        other => return Err(format!("tool.targetedAdjustment: unknown target `{other}` (curve, hue, saturation, luminance)")),
    };
    if !on {
        // only a targeted adjustment tool: a brush or crop the user armed stays
        if app.ui.tool.starts_with("tat:") {
            app.ui.tool.clear();
        }
        return Ok(json!({"tool": app.ui.tool, "on": false}));
    }
    // a black & white photo's HSL / Color panel offers the B&W mix only (`tat:lum`)
    if matches!(mode, Some("hue" | "saturation")) && app.session.active().and_then(|id| app.session.develop_of(id)).is_some_and(|d| crate::is_bw(&d))
    {
        return Err("tool.targetedAdjustment: only luminance is available on a black & white photo".into());
    }
    crate::menus::enter_develop(app);
    app.ui.right = RightPanel::Edit;
    let panel = if mode.is_some() { "hsl" } else { "toneCurve" };
    if !app.ui.section_open(panel) {
        app.ui.toggle_section(panel);
    }
    if let Some(m) = mode {
        app.ui.mixer_mode = m.to_string();
    }
    app.ui.tool = tool.to_string();
    app.ui.device.reveal = Some(format!("section:{panel}"));
    Ok(json!({"tool": tool, "on": true, "mixerMode": app.ui.mixer_mode}))
}
