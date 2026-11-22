//! Presets and copy/paste subsets.
//!
//! A preset is a *partial* settings object (JSON) — only the groups it includes. Applying merges it
//! into the photo's settings; `amount` (0..200 %) interpolates numeric values between the current
//! settings and the preset's. Copy/paste/sync use the same group selection ([`SettingsGroup`]).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::DevelopSettings;

/// One checkbox in "Choose settings to copy" / "Synchronize Settings" / "New Preset": Lightroom
/// Classic's per-setting checklist (Exposure, Contrast, … Sharpening, Luminance / Color Noise
/// Reduction, Lens Profile Corrections, Upright Mode / Transforms, Straighten Angle, Aspect Ratio …).
/// The checkboxes sit under headings ([`HEADINGS`]); a heading's id (`light`, `toneCurve`,
/// `detail`, `optics`, `geometry`, `crop` …) and the coarse ids older versions used (`color`,
/// `colorMixer`, `effects`) are accepted wherever a list of groups is ([`SettingsGroup::parse_ids`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SettingsGroup {
    Treatment,
    Profile,
    WhiteBalance,
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Texture,
    Clarity,
    Dehaze,
    Vibrance,
    Saturation,
    ParametricCurve,
    PointCurve,
    HslHue,
    HslSaturation,
    HslLuminance,
    BwMix,
    PointColor,
    ColorGrading,
    Sharpening,
    LuminanceNoise,
    ColorNoise,
    Enhance,
    LensProfile,
    ChromaticAberration,
    LensDistortion,
    LensVignetting,
    UprightMode,
    UprightTransforms,
    ManualTransform,
    Vignette,
    Grain,
    Calibration,
    Masks,
    Spots,
    RedEye,
    CropRect,
    Straighten,
    AspectRatio,
    LensBlur,
}

/// A heading in the checklist: its id, label and the checkboxes under it.
#[derive(Clone, Copy, Debug)]
pub struct GroupHeading {
    pub id: &'static str,
    pub label: &'static str,
    pub items: &'static [SettingsGroup],
}

use SettingsGroup as G;

/// The checklist's headings in Lightroom's order (a heading with one item is shown as that item).
pub const HEADINGS: &[GroupHeading] = &[
    GroupHeading { id: "basic", label: "Basic", items: &[G::Treatment, G::Profile, G::WhiteBalance] },
    GroupHeading { id: "light", label: "Basic Tone", items: &[G::Exposure, G::Contrast, G::Highlights, G::Shadows, G::Whites, G::Blacks] },
    GroupHeading { id: "presence", label: "Presence", items: &[G::Texture, G::Clarity, G::Dehaze, G::Vibrance, G::Saturation] },
    GroupHeading { id: "toneCurve", label: "Tone Curve", items: &[G::ParametricCurve, G::PointCurve] },
    GroupHeading {
        id: "colorPanel",
        label: "Color",
        items: &[G::HslHue, G::HslSaturation, G::HslLuminance, G::BwMix, G::PointColor, G::ColorGrading],
    },
    GroupHeading { id: "detail", label: "Detail", items: &[G::Sharpening, G::LuminanceNoise, G::ColorNoise, G::Enhance] },
    GroupHeading { id: "optics", label: "Lens Corrections", items: &[G::LensProfile, G::ChromaticAberration, G::LensDistortion, G::LensVignetting] },
    GroupHeading { id: "geometry", label: "Transform", items: &[G::UprightMode, G::UprightTransforms, G::ManualTransform] },
    GroupHeading { id: "effectsPanel", label: "Effects", items: &[G::Vignette, G::Grain] },
    GroupHeading { id: "calibration", label: "Calibration", items: &[G::Calibration] },
    GroupHeading { id: "lensBlur", label: "Lens Blur", items: &[G::LensBlur] },
    GroupHeading { id: "masks", label: "Masking", items: &[G::Masks] },
    GroupHeading { id: "remove", label: "Remove", items: &[G::Spots, G::RedEye] },
    GroupHeading { id: "crop", label: "Crop", items: &[G::CropRect, G::Straighten, G::AspectRatio] },
];

/// Coarse ids from before the per-setting checklist that are not heading ids.
const LEGACY: &[(&str, &[SettingsGroup])] = &[
    ("color", &[G::Vibrance, G::Saturation]),
    ("colorMixer", &[G::HslHue, G::HslSaturation, G::HslLuminance, G::BwMix, G::PointColor]),
    ("effects", &[G::Texture, G::Clarity, G::Dehaze]),
];

impl SettingsGroup {
    pub const ALL: [SettingsGroup; 43] = [
        G::Treatment,
        G::Profile,
        G::WhiteBalance,
        G::Exposure,
        G::Contrast,
        G::Highlights,
        G::Shadows,
        G::Whites,
        G::Blacks,
        G::Texture,
        G::Clarity,
        G::Dehaze,
        G::Vibrance,
        G::Saturation,
        G::ParametricCurve,
        G::PointCurve,
        G::HslHue,
        G::HslSaturation,
        G::HslLuminance,
        G::BwMix,
        G::PointColor,
        G::ColorGrading,
        G::Sharpening,
        G::LuminanceNoise,
        G::ColorNoise,
        G::Enhance,
        G::LensProfile,
        G::ChromaticAberration,
        G::LensDistortion,
        G::LensVignetting,
        G::UprightMode,
        G::UprightTransforms,
        G::ManualTransform,
        G::Vignette,
        G::Grain,
        G::Calibration,
        G::Masks,
        G::Spots,
        G::RedEye,
        G::CropRect,
        G::Straighten,
        G::AspectRatio,
        G::LensBlur,
    ];

    /// Groups included by default (Lightroom's "Default" subset: everything but crop, masking,
    /// remove and red eye).
    pub fn default_copy() -> Vec<SettingsGroup> {
        SettingsGroup::ALL.into_iter().filter(|g| !g.local()).collect()
    }

    /// Edits tied to where things are in one photo (left out of the default subset).
    pub fn local(self) -> bool {
        matches!(self, G::CropRect | G::Straighten | G::AspectRatio | G::Masks | G::Spots | G::RedEye)
    }

    /// The id used in commands (`exposure`, `lensProfile`, …).
    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
    }

    /// One id (a checkbox, a heading or an older coarse id) → its checkboxes.
    pub fn expand(id: &str) -> Option<Vec<SettingsGroup>> {
        if let Ok(g) = serde_json::from_value::<SettingsGroup>(Value::String(id.to_string())) {
            return Some(vec![g]);
        }
        HEADINGS
            .iter()
            .find(|h| h.id == id)
            .map(|h| h.items)
            .or_else(|| LEGACY.iter().find(|(k, _)| *k == id).map(|(_, v)| *v))
            .map(<[SettingsGroup]>::to_vec)
    }

    /// Parse a list of ids (see [`SettingsGroup::expand`]) into checklist order without repeats;
    /// `Err` names the first unknown id.
    pub fn parse_ids<S: AsRef<str>>(ids: &[S]) -> Result<Vec<SettingsGroup>, String> {
        let mut set = Vec::new();
        for id in ids {
            let id = id.as_ref();
            set.extend(SettingsGroup::expand(id).ok_or_else(|| format!("unknown settings group `{id}`"))?);
        }
        Ok(SettingsGroup::ALL.into_iter().filter(|g| set.contains(g)).collect())
    }

    /// As [`SettingsGroup::parse_ids`], skipping unknown ids (remembered UI state).
    pub fn parse_ids_lenient<S: AsRef<str>>(ids: &[S]) -> Vec<SettingsGroup> {
        let set: Vec<SettingsGroup> = ids.iter().filter_map(|id| SettingsGroup::expand(id.as_ref())).flatten().collect();
        SettingsGroup::ALL.into_iter().filter(|g| set.contains(g)).collect()
    }

    pub fn label(self) -> &'static str {
        match self {
            G::Treatment => "Treatment",
            G::Profile => "Profile",
            G::WhiteBalance => "White Balance",
            G::Exposure => "Exposure",
            G::Contrast => "Contrast",
            G::Highlights => "Highlights",
            G::Shadows => "Shadows",
            G::Whites => "White Clipping",
            G::Blacks => "Black Clipping",
            G::Texture => "Texture",
            G::Clarity => "Clarity",
            G::Dehaze => "Dehaze",
            G::Vibrance => "Vibrance",
            G::Saturation => "Saturation",
            G::ParametricCurve => "Parametric Curve",
            G::PointCurve => "Point Curve",
            G::HslHue => "HSL Hue",
            G::HslSaturation => "HSL Saturation",
            G::HslLuminance => "HSL Luminance",
            G::BwMix => "B&W Mix",
            G::PointColor => "Point Color",
            G::ColorGrading => "Color Grading",
            G::Sharpening => "Sharpening",
            G::LuminanceNoise => "Luminance Noise",
            G::ColorNoise => "Color Noise",
            G::Enhance => "Enhance (Denoise)",
            G::LensProfile => "Lens Profile Corrections",
            G::ChromaticAberration => "Chromatic Aberration",
            G::LensDistortion => "Manual Distortion",
            G::LensVignetting => "Lens Vignetting",
            G::UprightMode => "Upright Mode",
            G::UprightTransforms => "Upright Transforms",
            G::ManualTransform => "Transform Adjustments",
            G::Vignette => "Post-Crop Vignetting",
            G::Grain => "Grain",
            G::Calibration => "Calibration",
            G::Masks => "Masking",
            G::Spots => "Remove",
            G::RedEye => "Red Eye",
            G::CropRect => "Crop",
            G::Straighten => "Straighten Angle",
            G::AspectRatio => "Aspect Ratio",
            G::LensBlur => "Lens Blur",
        }
    }

    /// JSON paths (dot-separated, `*` = every key) of the `DevelopSettings` values the group covers.
    pub fn paths(self) -> &'static [&'static str] {
        match self {
            G::Treatment => &["treatment"],
            G::Profile => &["profile"],
            G::WhiteBalance => &["wb"],
            G::Exposure => &["light.exposure"],
            G::Contrast => &["light.contrast"],
            G::Highlights => &["light.highlights"],
            G::Shadows => &["light.shadows"],
            G::Whites => &["light.whites"],
            G::Blacks => &["light.blacks"],
            G::Texture => &["effects.texture"],
            G::Clarity => &["effects.clarity"],
            G::Dehaze => &["effects.dehaze"],
            G::Vibrance => &["color.vibrance"],
            G::Saturation => &["color.saturation"],
            G::ParametricCurve => &[
                "curve.highlights",
                "curve.lights",
                "curve.darks",
                "curve.shadows",
                "curve.split_shadows",
                "curve.split_mid",
                "curve.split_highlights",
            ],
            G::PointCurve => &["curve.master", "curve.red", "curve.green", "curve.blue", "curve.refine_saturation"],
            G::HslHue => &["mixer.*.hue"],
            G::HslSaturation => &["mixer.*.sat"],
            G::HslLuminance => &["mixer.*.lum"],
            G::BwMix => &["bw_mix"],
            G::PointColor => &["point_colors"],
            G::ColorGrading => &["grading"],
            G::Sharpening => &["detail.sharpen_amount", "detail.sharpen_radius", "detail.sharpen_detail", "detail.sharpen_masking"],
            G::LuminanceNoise => &["detail.nr_luminance", "detail.nr_detail", "detail.nr_contrast"],
            G::ColorNoise => &["detail.nr_color", "detail.nr_color_detail", "detail.nr_color_smoothness"],
            G::Enhance => &["enhance"],
            G::LensProfile => &["optics.lens_profile", "optics.lens_profile_ref", "optics.profile_distortion", "optics.profile_vignetting"],
            G::ChromaticAberration => &[
                "optics.remove_ca",
                "optics.ca_red",
                "optics.ca_blue",
                "optics.defringe_purple_amount",
                "optics.defringe_purple_hue_lo",
                "optics.defringe_purple_hue_hi",
                "optics.defringe_green_amount",
                "optics.defringe_green_hue_lo",
                "optics.defringe_green_hue_hi",
            ],
            G::LensDistortion => &["optics.distortion"],
            G::LensVignetting => &["optics.vignetting", "optics.vignetting_midpoint"],
            G::UprightMode => &["geometry.upright", "geometry.guides"],
            G::UprightTransforms => &["geometry.upright_transform"],
            G::ManualTransform => &[
                "geometry.vertical",
                "geometry.horizontal",
                "geometry.rotate",
                "geometry.aspect",
                "geometry.scale",
                "geometry.offset_x",
                "geometry.offset_y",
                "geometry.constrain_crop",
            ],
            G::Vignette => &["vignette"],
            G::Grain => &["grain"],
            G::Calibration => &["calibration"],
            G::Masks => &["masks"],
            G::Spots => &["spots"],
            G::RedEye => &["red_eye"],
            G::CropRect => &["crop.geometry.rect", "crop.flip_h", "crop.flip_v", "orientation"],
            G::Straighten => &["crop.geometry.angle"],
            G::AspectRatio => &["crop.aspect"],
            G::LensBlur => &["lens_blur"],
        }
    }

    /// The groups in which `s` differs from `base` (Lightroom's "Modified" subset).
    pub fn modified_in(s: &DevelopSettings, base: &DevelopSettings) -> Vec<SettingsGroup> {
        let (a, b) = (s.to_json(), base.to_json());
        SettingsGroup::ALL.into_iter().filter(|g| extract_json(&a, &[*g]) != extract_json(&b, &[*g])).collect()
    }
}

/// Copy the value at `path` from `src` into `out`, creating objects on the way. A value missing
/// in `src` (an optional field left out) is copied as `null`, so pasting it clears the target's.
fn copy_path(src: &Value, out: &mut Map<String, Value>, path: &[&str]) {
    let Some((first, rest)) = path.split_first() else { return };
    if *first == "*" {
        if let Value::Object(o) = src {
            for k in o.keys() {
                let mut key_path = vec![k.as_str()];
                key_path.extend_from_slice(rest);
                copy_path(src, out, &key_path);
            }
        }
        return;
    }
    let v = src.get(*first).unwrap_or(&Value::Null);
    if rest.is_empty() {
        out.insert((*first).to_string(), v.clone());
        return;
    }
    if !v.is_object() {
        return;
    }
    let slot = out.entry((*first).to_string()).or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(child) = slot {
        copy_path(v, child, rest);
    }
}

fn extract_json(full: &Value, groups: &[SettingsGroup]) -> Value {
    let mut out = Map::new();
    for g in groups {
        for p in g.paths() {
            let path: Vec<&str> = p.split('.').collect();
            copy_path(full, &mut out, &path);
        }
    }
    Value::Object(out)
}

/// The subset of `s` covering `groups`, as a partial JSON object.
pub fn extract_groups(s: &DevelopSettings, groups: &[SettingsGroup]) -> Value {
    extract_json(&s.to_json(), groups)
}

/// Keep only the masks with these ids in a partial settings object (Lightroom's "choose masks").
pub fn retain_masks(partial: &mut Value, ids: &[u32]) {
    if let Some(Value::Array(masks)) = partial.get_mut("masks") {
        masks.retain(|m| m.get("id").and_then(Value::as_u64).is_some_and(|id| ids.iter().any(|x| u64::from(*x) == id)));
    }
}

/// Recursively merge `patch` into `base` (objects merge; everything else replaces).
pub fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Interpolate numbers in `patch` towards/away from `base` by `t` (1 = patch as-is); non-numeric
/// values are taken from the patch when `t >= 0.5`.
fn scale_patch(base: &Value, patch: &Value, t: f64) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out = Map::new();
            for (k, pv) in p {
                let bv = b.get(k).unwrap_or(&Value::Null);
                out.insert(k.clone(), scale_patch(bv, pv, t));
            }
            Value::Object(out)
        }
        (Value::Number(b), Value::Number(p)) => {
            let (b, p) = (b.as_f64().unwrap_or(0.0), p.as_f64().unwrap_or(0.0));
            serde_json::Number::from_f64(b + (p - b) * t).map(Value::Number).unwrap_or(Value::Null)
        }
        (b, p) => {
            if t >= 0.5 {
                p.clone()
            } else {
                b.clone()
            }
        }
    }
}

/// Apply a partial settings object with an amount (1.0 = 100 %). Out-of-range values are clamped
/// through the control specs.
pub fn apply_partial(s: &DevelopSettings, partial: &Value, amount: f64) -> DevelopSettings {
    let base = s.to_json();
    let patch = if (amount - 1.0).abs() < 1e-9 { partial.clone() } else { scale_patch(&base, partial, amount) };
    let mut out = s.merged(&patch).unwrap_or_else(|_| s.clone());
    for c in crate::controls::CONTROLS {
        if let Some(v) = crate::controls::get(&out, c.id) {
            crate::controls::set(&mut out, c.id, v);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub group: String,
    /// Partial settings.
    pub settings: Value,
    #[serde(default)]
    pub favorite: bool,
    /// Built-in presets ship with the app and can't be deleted.
    #[serde(default)]
    pub builtin: bool,
}

impl Preset {
    pub fn from_settings(id: &str, name: &str, group: &str, s: &DevelopSettings, groups: &[SettingsGroup]) -> Preset {
        Preset { id: id.into(), name: name.into(), group: group.into(), settings: extract_groups(s, groups), favorite: false, builtin: false }
    }
    /// Apply at `amount` (1.0 = 100 %). Masks in the preset are added to the photo's own (ones it
    /// already has are skipped), their effect scaled by the amount.
    pub fn apply(&self, s: &DevelopSettings, amount: f64) -> DevelopSettings {
        let Some(masks) = self.settings.get("masks").and_then(Value::as_array).filter(|m| !m.is_empty()) else {
            return apply_partial(s, &self.settings, amount);
        };
        let mut rest = self.settings.clone();
        if let Some(o) = rest.as_object_mut() {
            o.remove("masks");
        }
        let mut out = apply_partial(s, &rest, amount);
        let mut next = out.masks.iter().map(|m| m.id).max().unwrap_or(0);
        for m in masks {
            let Ok(mut m) = serde_json::from_value::<crate::Mask>(m.clone()) else { continue };
            if out.masks.iter().any(|q| q.name == m.name && q.components == m.components) {
                continue;
            }
            next += 1;
            m.id = next;
            m.adjust.amount = (m.adjust.amount * amount).clamp(0.0, 200.0);
            out.masks.push(m);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_and_apply_roundtrip() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.effects.clarity = 30.0;
        a.crop.geometry.angle = 5.0;
        let p = Preset::from_settings("p", "P", "User", &a, &SettingsGroup::default_copy());
        assert!(p.settings.get("crop").is_none());
        let b = p.apply(&DevelopSettings::default(), 1.0);
        assert_eq!(b.light.exposure, 1.0);
        assert_eq!(b.effects.clarity, 30.0);
        assert_eq!(b.crop.geometry.angle, 0.0);
    }

    #[test]
    fn amount_scales_numbers() {
        let base = DevelopSettings::default();
        let partial = json!({"light": {"exposure": 1.0, "contrast": 40}});
        let half = apply_partial(&base, &partial, 0.5);
        assert!((half.light.exposure - 0.5).abs() < 1e-9);
        assert!((half.light.contrast - 20.0).abs() < 1e-9);
        let more = apply_partial(&base, &partial, 2.0);
        assert!((more.light.contrast - 80.0).abs() < 1e-9);
        // clamped through specs
        let lots = apply_partial(&base, &json!({"light": {"exposure": 4.0}}), 2.0);
        assert_eq!(lots.light.exposure, 5.0);
    }

    #[test]
    fn per_slider_groups_copy_only_their_slider() {
        let mut a = DevelopSettings::default();
        a.light.exposure = 1.0;
        a.light.contrast = 20.0;
        a.mixer.red.hue = 10.0;
        a.mixer.red.sat = 30.0;
        a.detail.sharpen_amount = 60.0;
        a.detail.nr_luminance = 20.0;
        a.crop.geometry.angle = 3.0;
        let p = extract_groups(&a, &[SettingsGroup::Exposure, SettingsGroup::HslHue, SettingsGroup::Sharpening, SettingsGroup::Straighten]);
        assert_eq!(p["light"], json!({"exposure": 1.0}));
        assert_eq!(p["mixer"]["red"], json!({"hue": 10.0}));
        assert!(p["detail"].get("nr_luminance").is_none() && p["detail"]["sharpen_amount"] == 60.0);
        assert_eq!(p["crop"], json!({"geometry": {"angle": 3.0}}));
        let mut b = DevelopSettings::default();
        b.light.contrast = -10.0;
        b.mixer.red.sat = -5.0;
        let c = apply_partial(&b, &p, 1.0);
        assert_eq!((c.light.exposure, c.light.contrast), (1.0, -10.0));
        assert_eq!((c.mixer.red.hue, c.mixer.red.sat), (10.0, -5.0));
        assert_eq!((c.detail.sharpen_amount, c.detail.nr_luminance, c.crop.geometry.angle), (60.0, 0.0, 3.0));
    }

    #[test]
    fn every_group_covers_existing_settings_and_groups_partition_them() {
        let full = DevelopSettings::default().to_json();
        let all = extract_groups(&DevelopSettings::default(), &SettingsGroup::ALL);
        for (k, v) in full.as_object().unwrap() {
            if k == "version" || k == "disabled_sections" {
                continue;
            }
            fn covers(a: &Value, b: &Value, at: &str) {
                if let (Value::Object(a), Value::Object(b)) = (a, b) {
                    for (k, v) in b {
                        covers(a.get(k).unwrap_or_else(|| panic!("`{at}.{k}` is in no group")), v, &format!("{at}.{k}"));
                    }
                } else {
                    assert_eq!(a, b, "{at}");
                }
            }
            covers(all.get(k).unwrap_or_else(|| panic!("`{k}` is in no group")), v, k);
        }
        for g in SettingsGroup::ALL {
            assert!(HEADINGS.iter().filter(|h| h.items.contains(&g)).count() == 1, "{g:?} under one heading");
            assert_eq!(SettingsGroup::expand(&g.id()), Some(vec![g]));
        }
    }

    #[test]
    fn heading_and_legacy_ids_expand() {
        let g = SettingsGroup::parse_ids(&["light", "color", "detail", "effects"]).unwrap();
        assert!(g.contains(&SettingsGroup::Exposure) && g.contains(&SettingsGroup::Blacks) && g.contains(&SettingsGroup::Vibrance));
        assert!(g.contains(&SettingsGroup::LuminanceNoise) && g.contains(&SettingsGroup::Clarity) && !g.contains(&SettingsGroup::Grain));
        assert!(SettingsGroup::parse_ids(&["nope"]).unwrap_err().contains("nope"));
        assert_eq!(SettingsGroup::parse_ids_lenient(&["nope", "grain"]), vec![SettingsGroup::Grain]);
        // an optional value missing on the source clears the target's
        let mut t = DevelopSettings::default();
        t.optics.lens_profile_ref = Some(crate::LensProfileRef { name: "x".into(), ..Default::default() });
        let p = extract_groups(&DevelopSettings::default(), &[SettingsGroup::LensProfile]);
        assert!(apply_partial(&t, &p, 1.0).optics.lens_profile_ref.is_none());
    }

    #[test]
    fn modified_lists_changed_groups() {
        let base = DevelopSettings::default();
        let mut a = base.clone();
        a.light.highlights = -40.0;
        a.grain.amount = 20.0;
        assert_eq!(SettingsGroup::modified_in(&a, &base), vec![SettingsGroup::Highlights, SettingsGroup::Grain]);
    }

    #[test]
    fn deep_merge_objects() {
        let mut a = json!({"x": {"a": 1, "b": 2}, "y": [1]});
        deep_merge(&mut a, &json!({"x": {"b": 3}, "y": [2, 3]}));
        assert_eq!(a, json!({"x": {"a": 1, "b": 3}, "y": [2, 3]}));
    }
}
