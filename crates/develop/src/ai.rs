//! Lightroom's AI mask vocabulary (Landscape classes, People parts) and the basis that tells
//! when an AI mask is out of date.

use crate::{DevelopSettings, MaskShape};

/// Lightroom's Landscape classes: id, label (its order).
pub const LANDSCAPE_CLASSES: &[(&str, &str)] = &[
    ("sky", "Sky"),
    ("water", "Water"),
    ("vegetation", "Vegetation"),
    ("mountains", "Mountains"),
    ("architecture", "Architecture"),
    ("naturalGround", "Natural Ground"),
    ("artificialGround", "Artificial Ground"),
];

/// Lightroom's People parts: id, label (its order).
pub const PEOPLE_PARTS: &[(&str, &str)] = &[
    ("person", "Entire Person"),
    ("faceSkin", "Face Skin"),
    ("bodySkin", "Body Skin"),
    ("eyebrows", "Eyebrows"),
    ("eyeSclera", "Eye Sclera"),
    ("iris", "Iris and Pupil"),
    ("lips", "Lips"),
    ("teeth", "Teeth"),
    ("hair", "Hair"),
    ("clothes", "Clothes"),
];

pub fn landscape_label(id: &str) -> Option<&'static str> {
    LANDSCAPE_CLASSES.iter().find(|c| c.0 == id).map(|c| c.1)
}

pub fn part_label(id: &str) -> Option<&'static str> {
    PEOPLE_PARTS.iter().find(|p| p.0 == id).map(|p| p.1)
}

/// What an AI mask on `d` was computed from: a hash of the settings that move or change the
/// photo's pixels under the mask (orientation and flips, lens and geometry corrections, spots,
/// red eye, Enhance) — not tone or color, which the masks don't depend on enough to matter, and
/// not the crop (masks cover the uncropped photo).
pub fn basis(d: &DevelopSettings) -> String {
    let parts = serde_json::json!([d.orientation, d.crop.flip_h, d.crop.flip_v, d.optics, d.geometry, d.spots, d.red_eye, d.enhance]);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in parts.to_string().bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Whether `shape` (an AI mask component) was computed on a photo that looked different from
/// `d` now, so Update AI Settings would change it. A shape never computed (no segmentation, or
/// one from before bases were recorded) isn't out of date.
pub fn stale(shape: &MaskShape, d: &DevelopSettings) -> bool {
    shape.is_ai() && shape.seg().and_then(|s| s.basis.as_deref()).is_some_and(|b| b != basis(d))
}

/// The (mask id, component index) of every out-of-date AI component of `d`.
pub fn stale_components(d: &DevelopSettings) -> Vec<(u32, usize)> {
    let now = basis(d);
    let mut out = Vec::new();
    for m in &d.masks {
        for (k, c) in m.components.iter().enumerate() {
            if c.shape.is_ai() && c.shape.seg().and_then(|s| s.basis.as_deref()).is_some_and(|b| b != now) {
                out.push((m.id, k));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Mask, MaskComponent, MaskOp, SegMask};

    #[test]
    fn a_mask_goes_stale_when_the_pixels_move_not_when_the_tone_changes() {
        let mut d = DevelopSettings::default();
        let mut seg = SegMask::from_logits(2, &[1.0; 4]);
        seg.basis = Some(basis(&d));
        d.masks.push(Mask {
            id: 3,
            components: vec![MaskComponent { name: None, op: MaskOp::Add, invert: false, shape: MaskShape::Sky { seg: Some(seg) } }],
            ..Default::default()
        });
        assert!(stale_components(&d).is_empty());
        d.light.exposure = 1.5;
        d.crop.geometry.angle = 3.0;
        assert!(stale_components(&d).is_empty(), "tone and crop don't matter");
        d.spots.push(crate::Spot::default());
        assert_eq!(stale_components(&d), [(3, 0)]);
        assert!(stale(&d.masks[0].components[0].shape, &d));
        // never computed, or from before bases: not stale
        assert!(!stale(&MaskShape::Sky { seg: None }, &d));
        assert!(!stale(&MaskShape::Sky { seg: Some(SegMask::from_logits(2, &[1.0; 4])) }, &d));
        assert!(!stale(&MaskShape::Linear { start: lightcraft_geom::Point::new(0.0, 0.0), end: lightcraft_geom::Point::new(1.0, 1.0) }, &d));
    }

    #[test]
    fn old_documents_still_load() {
        // AI shapes saved before they carried segmentations
        for j in [
            r#"{"kind":"subject"}"#,
            r#"{"kind":"sky"}"#,
            r#"{"kind":"background"}"#,
            r#"{"kind":"people","person":0,"parts":["hair"]}"#,
            r#"{"kind":"landscape","class":"water"}"#,
        ] {
            let s: MaskShape = serde_json::from_str(j).unwrap();
            assert!(s.is_ai() && s.seg().is_none(), "{j}");
            assert_eq!(serde_json::to_string(&s).unwrap(), j, "round-trips unchanged");
        }
        assert_eq!(landscape_label("naturalGround"), Some("Natural Ground"));
        assert_eq!(part_label("iris"), Some("Iris and Pupil"));
    }
}
