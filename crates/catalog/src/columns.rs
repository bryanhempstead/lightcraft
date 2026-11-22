//! The Library Filter's metadata column browser (Lightroom's Metadata filter, L07): each column is a
//! metadata field listing the values the photos have, with counts; picking values in several
//! columns narrows the view (any picked value within a column, every column together).

use serde::{Deserialize, Serialize};

use crate::{Catalog, Photo};

/// The columns offered: (id, label).
pub const COLUMNS: &[(&str, &str)] = &[
    ("date", "Date"),
    ("year", "Year"),
    ("camera", "Camera"),
    ("lens", "Lens"),
    ("label", "Label"),
    ("keyword", "Keyword"),
    ("fileType", "File Type"),
    ("flag", "Flag"),
    ("rating", "Rating"),
    ("iso", "ISO Speed"),
    ("focalLength", "Focal Length"),
    ("aperture", "Aperture"),
    ("shutter", "Shutter Speed"),
    ("country", "Country"),
    ("city", "City"),
    ("location", "Location"),
    ("creator", "Creator"),
    ("treatment", "Treatment"),
    ("aspect", "Aspect Ratio"),
    ("copies", "Copy Status"),
    ("folder", "Folder"),
];

/// Lightroom's default columns.
pub const DEFAULT_COLUMNS: &[&str] = &["date", "camera", "lens", "label"];

/// One column's picked values (empty = no constraint).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPick {
    pub column: String,
    pub values: Vec<String>,
}

/// The value(s) a photo has in `column` (`""` = unknown / none). Keywords give one value each.
pub fn values(p: &Photo, cat: &Catalog, column: &str) -> Vec<String> {
    let m = &p.meta;
    let one = |s: &str| vec![s.trim().to_string()];
    match column {
        "date" => one(p.captured.as_deref().and_then(|d| d.get(..7)).unwrap_or("")),
        "year" => one(p.captured.as_deref().and_then(|d| d.get(..4)).unwrap_or("")),
        "camera" => one(&m.camera),
        "lens" => one(&m.lens),
        "label" => one(&p.label.map(|l| cat.label_name(l)).unwrap_or_default()),
        "keyword" => {
            if m.keywords.is_empty() {
                one("")
            } else {
                m.keywords.clone()
            }
        }
        "fileType" => one(&p.format.to_uppercase()),
        "flag" => one(match p.flag {
            crate::Flag::Pick => "Flagged",
            crate::Flag::Reject => "Rejected",
            crate::Flag::None => "Unflagged",
        }),
        "rating" => vec![if p.rating == 0 { "No stars".into() } else { "★".repeat(p.rating.into()) }],
        "iso" => one(&m.iso.map(|v| format!("ISO {v}")).unwrap_or_default()),
        "focalLength" => one(&m.focal_mm.filter(|f| f.is_finite()).map(|f| format!("{} mm", f.round() as i64)).unwrap_or_default()),
        "aperture" => one(&m.aperture.filter(|f| f.is_finite()).map(|f| format!("f/{f}")).unwrap_or_default()),
        "shutter" => one(&m.shutter),
        "country" => one(&m.country),
        "city" => one(&m.city),
        "location" => one(&m.location),
        "creator" => one(&m.creator),
        "treatment" => one(if p.develop.treatment == lightcraft_develop::Treatment::Bw { "Black & White" } else { "Color" }),
        "aspect" => one(match p.width.cmp(&p.height) {
            std::cmp::Ordering::Greater => "Landscape",
            std::cmp::Ordering::Less => "Portrait",
            std::cmp::Ordering::Equal => "Square",
        }),
        "copies" => one(if p.copy_of.is_some() { "Virtual Copy" } else { "Original" }),
        "folder" => one(&match &p.source {
            crate::Source::File { path } => std::path::Path::new(path).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default(),
            crate::Source::Demo { .. } => String::new(),
        }),
        _ => vec![],
    }
}

/// `p` has one of the picked values in every column with picks.
pub fn matches(picks: &[ColumnPick], p: &Photo, cat: &Catalog) -> bool {
    picks.iter().filter(|c| !c.values.is_empty()).all(|c| values(p, cat, &c.column).iter().any(|v| c.values.contains(v)))
}

/// A column's values over `photos` with counts: by count for most columns, newest first for
/// dates, high to low for ratings; the unknown value (`""`) last.
pub fn counts<'a>(photos: impl Iterator<Item = &'a Photo>, cat: &Catalog, column: &str) -> Vec<(String, usize)> {
    let mut map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for p in photos {
        let mut vs = values(p, cat, column);
        vs.sort();
        vs.dedup();
        for v in vs {
            *map.entry(v).or_insert(0) += 1;
        }
    }
    let mut v: Vec<(String, usize)> = map.into_iter().collect();
    match column {
        "date" | "year" | "rating" => v.sort_by(|a, b| b.0.cmp(&a.0)),
        _ => v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))),
    }
    // unknown last
    if let Some(i) = v.iter().position(|(k, _)| k.is_empty()) {
        let u = v.remove(i);
        v.push(u);
    }
    v
}
