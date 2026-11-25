//! merges. defaults per kind (Settings ▸ merge., Bryan 2026-11-24: "pick and choose by default for
//! the different types of merges"): where the merged photo shows, whether it is stacked with its
//! frames, Auto Settings, Auto Crop, the panorama projection, the HDR deghost amount, whether the
//! originals are hidden after merging; and from which confidence groups are shown.
//!
//! The app keeps them in its settings (`ui.json`) and hands them to the session
//! ([`crate::Session::merge_prefs`]); `merge.mergeGroups`, [`crate::Session::merge_group_done`] and
//! the Merge dialog start from them. Without the app (CLI, MCP) the defaults apply.

use lightcraft_catalog::MergeType;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::merge_find::Confidence;

/// What one kind of merge does by default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KindPrefs {
    /// The merged photo shows only in merges. (`merges.only.`), not also in its folder and
    /// collections (`both.`, the default).
    pub merges_only: bool,
    /// Stack the merged photo with its frames.
    pub stack: bool,
    /// Auto Settings on the merged photo.
    pub auto_settings: bool,
    /// Auto Crop (panoramas: the blank edges cropped away).
    pub auto_crop: bool,
    /// Panoramas: auto | spherical | cylindrical | perspective.
    pub projection: String,
    /// HDR: none | low | medium | high.
    pub deghost: String,
    /// Hide the frames after merging (they stay in the library, files untouched); off: they stay
    /// visible next to the merged photo.
    pub hide_originals: bool,
}

impl Default for KindPrefs {
    fn default() -> Self {
        KindPrefs {
            merges_only: false,
            stack: true,
            auto_settings: false, // Bryan 11-24: Auto blew merges out; the merged photo takes its frames' look
            auto_crop: true,
            projection: "auto".into(),
            deghost: "none".into(),
            hide_originals: true,
        }
    }
}

/// The merges. defaults (Settings ▸ merge.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MergePrefs {
    pub panorama: KindPrefs,
    pub hdr: KindPrefs,
    pub hdr_panorama: KindPrefs,
    pub focus_stack: KindPrefs,
    /// Groups below this confidence are not shown (`maybe.`: every group, the default).
    pub show_from: Confidence,
}

impl Default for MergePrefs {
    fn default() -> Self {
        MergePrefs {
            panorama: KindPrefs::default(),
            hdr: KindPrefs::default(),
            hdr_panorama: KindPrefs::default(),
            focus_stack: KindPrefs::default(),
            show_from: Confidence::Maybe,
        }
    }
}

/// The projections a panorama can take, as the settings and the Merge dialog name them.
pub const PROJECTIONS: [&str; 4] = ["auto", "spherical", "cylindrical", "perspective"];
/// The deghost amounts.
pub const DEGHOSTS: [&str; 4] = ["none", "low", "medium", "high"];

impl MergePrefs {
    pub fn kind(&self, k: MergeType) -> &KindPrefs {
        match k {
            MergeType::Pano => &self.panorama,
            MergeType::Hdr => &self.hdr,
            MergeType::HdrPano => &self.hdr_panorama,
            MergeType::Stack => &self.focus_stack,
        }
    }
    pub fn kind_mut(&mut self, k: MergeType) -> &mut KindPrefs {
        match k {
            MergeType::Pano => &mut self.panorama,
            MergeType::Hdr => &mut self.hdr,
            MergeType::HdrPano => &mut self.hdr_panorama,
            MergeType::Stack => &mut self.focus_stack,
        }
    }
    /// A kind's defaults as `merge.*` parameters (`autoSettings`, `stack`, `autoCrop`,
    /// `projection`, `deghost`); a value the engine doesn't know falls back to its default.
    pub fn params(&self, k: MergeType) -> Value {
        let p = self.kind(k);
        let projection = if PROJECTIONS.contains(&p.projection.as_str()) { p.projection.as_str() } else { "auto" };
        let deghost = if DEGHOSTS.contains(&p.deghost.as_str()) { p.deghost.as_str() } else { "none" };
        json!({
            "autoSettings": p.auto_settings,
            "stack": p.stack,
            "autoCrop": p.auto_crop,
            "projection": projection,
            "deghost": deghost,
        })
    }
    /// These prefs with `patch` (a partial [`MergePrefs`] in JSON: `{panorama: {stack: false}}`)
    /// laid over them. Unknown keys are refused, so a typo doesn't pass silently.
    pub fn patched(&self, patch: &Value) -> Result<MergePrefs, String> {
        let mut v = serde_json::to_value(self).map_err(|e| e.to_string())?;
        overlay(&mut v, patch, "")?;
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}

/// Lay `patch` over `base` key by key (objects deep, anything else replaced).
fn overlay(base: &mut Value, patch: &Value, at: &str) -> Result<(), String> {
    let Some(p) = patch.as_object() else {
        *base = patch.clone();
        return Ok(());
    };
    let Some(b) = base.as_object_mut() else {
        return Err(format!("`{at}` is not a group of settings"));
    };
    for (k, v) in p {
        let path = if at.is_empty() { k.clone() } else { format!("{at}.{k}") };
        let Some(slot) = b.get_mut(k) else {
            return Err(format!("no merge setting `{path}`"));
        };
        if slot.is_object() {
            overlay(slot, v, &path)?;
        } else {
            *slot = v.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_patch() {
        let d = MergePrefs::default();
        assert!(d.panorama.stack && !d.panorama.auto_settings && d.panorama.auto_crop && d.panorama.hide_originals && !d.panorama.merges_only);
        assert_eq!((d.panorama.projection.as_str(), d.hdr.deghost.as_str()), ("auto", "none"));
        assert_eq!(d.show_from, Confidence::Maybe);
        let v = serde_json::to_value(&d).unwrap();
        assert!(v.get("hdrPanorama").is_some() && v["focusStack"].get("hideOriginals").is_some(), "{v}");
        assert_eq!(serde_json::from_value::<MergePrefs>(v).unwrap(), d);
        // an old file without the section (or with part of it) reads as the defaults
        assert_eq!(serde_json::from_value::<MergePrefs>(json!({"hdr": {"deghost": "high"}})).unwrap().hdr.deghost, "high");
        let p = d.patched(&json!({"panorama": {"projection": "cylindrical", "stack": false}, "showFrom": "likely"})).unwrap();
        assert_eq!((p.panorama.projection.as_str(), p.panorama.stack, p.show_from), ("cylindrical", false, Confidence::Likely));
        assert!(p.hdr.stack, "the other kinds keep theirs");
        assert!(d.patched(&json!({"panorama": {"stak": false}})).is_err());
        assert!(d.patched(&json!({"panorama": {"stack": "yes"}})).is_err());
        let params = p.params(MergeType::Pano);
        assert_eq!((params["projection"].as_str(), params["stack"].as_bool()), (Some("cylindrical"), Some(false)));
        let mut odd = d.clone();
        odd.hdr.deghost = "lots".into();
        assert_eq!(odd.params(MergeType::Hdr)["deghost"], "none", "an unknown amount falls back");
    }
}
